# 验证阶段实现: prepare / payload / baseline / fs。
# 子进程一律以仓库根为工作目录并传相对路径, 阶段输出目录独立命名。

import time
import zipfile
from dataclasses import dataclass, field
from pathlib import Path

from . import compare, config, downloader, toolchain
from .paths import (VerifyError, build_tmp, ensure_dir, rel_to_repo,
                    safe_rmfile, safe_rmtree)
from .report import CaseResult, log


@dataclass
class Context:
    # 由入口脚本构造: 工具链与过滤选项
    pdg_bin: Path
    erofs_extract_bin: Path
    imgkit_bin: Path
    fs_types: list[str] = field(default_factory=list)
    erofs_algos: list[str] = field(default_factory=list)
    erofs_tiers: list[str] = field(default_factory=list)
    keep_going: bool = False
    prune: list[str] = field(default_factory=list)
    rom_mirror: str = "auto"
    baseline_partition: str | None = None


def _rom_zip_path() -> Path:
    return build_tmp() / "downloads" / "rom" / config.ROM_ZIP_NAME


def _strip_slot(name: str) -> str:
    # payload 输出保留 slot 后缀 (system_a), 归一化后比对
    for suffix in ("_a", "_b"):
        if name.endswith(suffix):
            return name[: -len(suffix)]
    return name


def _require(path: Path, hint: str) -> None:
    if not path.exists():
        raise VerifyError(f"缺少前置产物 {rel_to_repo(path) if path.is_absolute() else path}: {hint}")


def stage_prepare(ctx: Context, opener) -> list[CaseResult]:
    # 下载 ROM 压缩包: auto 先测速选最快, 失败自动切换镜像续传
    start = time.monotonic()
    log("阶段 prepare: 下载 ROM")
    rom_dir = ensure_dir(build_tmp() / "downloads" / "rom")
    rom_dest = rom_dir / config.ROM_ZIP_NAME

    if ctx.rom_mirror == "auto":
        log("镜像模式 auto: 测速选最快")
        ranked = downloader.speed_test(opener, config.ROM_MIRRORS,
                                       config.MIRROR_TEST_BYTES,
                                       config.MIRROR_TEST_TIMEOUT)
        if not ranked:
            raise VerifyError("全部镜像测速失败")
        mirrors = {name: config.ROM_MIRRORS[name] for name, _ in ranked}
    else:
        mirrors = {ctx.rom_mirror: config.ROM_MIRRORS[ctx.rom_mirror]}

    downloader.download_mirrors(opener, mirrors, rom_dest, config.ROM_SIZE, "ROM")
    return [CaseResult("prepare:rom", "PASS", time.monotonic() - start,
                       f"{config.ROM_SIZE / 1024**3:.2f} GiB")]


def _extract_payload_bin() -> Path:
    # 从 ROM zip 提取 payload.bin (STORED 纯拷贝), 并校验 CrAU magic
    rom_zip = _rom_zip_path()
    _require(rom_zip, "先运行 --stage prepare")
    payload_bin = build_tmp() / "verify" / "rom" / "payload.bin"
    ensure_dir(payload_bin.parent)

    with zipfile.ZipFile(rom_zip) as zf:
        names = zf.namelist()
        if "payload.bin" not in names:
            raise VerifyError(f"ROM zip 中无 payload.bin: {names[:10]}")
        info = zf.getinfo("payload.bin")
        if payload_bin.exists() and payload_bin.stat().st_size == info.file_size:
            log(f"payload.bin 已存在, 跳过提取 ({info.file_size} 字节)")
        else:
            log(f"提取 payload.bin ({info.file_size / 1024**3:.2f} GiB)")
            with zf.open(info) as src, open(payload_bin, "wb") as dst:
                while True:
                    block = src.read(4 * 1024 * 1024)
                    if not block:
                        break
                    dst.write(block)

    with open(payload_bin, "rb") as f:
        if f.read(4) != b"CrAU":
            raise VerifyError("payload.bin magic 校验失败 (期望 CrAU)")
    return payload_bin


def stage_payload(ctx: Context) -> list[CaseResult]:
    # 双工具提取 payload 并逐镜像比对哈希
    log("阶段 payload: 双工具提取比对")
    start = time.monotonic()
    payload_bin = _extract_payload_bin()

    ref_dir = ensure_dir(build_tmp() / "verify" / "payload" / "ref")
    tool_dir = ensure_dir(build_tmp() / "verify" / "payload" / "tool")
    safe_rmtree("verify/payload/ref")
    safe_rmtree("verify/payload/tool")
    ref_dir = ensure_dir(ref_dir)
    tool_dir = ensure_dir(tool_dir)

    payload_rel = rel_to_repo(payload_bin)
    toolchain.run_command([str(ctx.pdg_bin), "-o", rel_to_repo(ref_dir), payload_rel])
    toolchain.imgkit_unpack(ctx.imgkit_bin, payload_rel, rel_to_repo(tool_dir))

    ref_files = {_strip_slot(p.stem): p for p in ref_dir.glob("*.img")}
    tool_files = {_strip_slot(p.stem): p for p in tool_dir.glob("*.img")}

    results = []
    missing = sorted(set(ref_files) - set(tool_files))
    extra = sorted(set(tool_files) - set(ref_files))
    if missing or extra:
        results.append(CaseResult("payload:文件集合", "FAIL",
                                  time.monotonic() - start,
                                  f"缺失 {missing} / 多出 {extra}"))
        return results

    for name in sorted(ref_files):
        case_start = time.monotonic()
        ref_hash = compare.sha256_file(ref_files[name])
        tool_hash = compare.sha256_file(tool_files[name])
        size = ref_files[name].stat().st_size
        if ref_hash == tool_hash:
            results.append(CaseResult(f"payload:{name}", "PASS",
                                      time.monotonic() - case_start,
                                      f"{size / 1024**2:.0f} MiB"))
        else:
            results.append(CaseResult(f"payload:{name}", "FAIL",
                                      time.monotonic() - case_start,
                                      f"{size / 1024**2:.0f} MiB 哈希不一致"))

    # 比对完成, 磁盘紧张时清理 ref 与 ROM zip (payload.bin 已独立, 重跑 prepare 可恢复)
    if "payload" in ctx.prune:
        safe_rmtree("verify/payload/ref")
        safe_rmfile(f"downloads/rom/{config.ROM_ZIP_NAME}")
        log("prune: 已清理 ref/ 与 ROM zip")
    return results


def _select_baseline_partition(ctx: Context) -> Path:
    # 基准镜像选择: 显式指定分区名优先, 否则有 super 则拆分后取最大, 否则 payload 直出最大
    tool_dir = build_tmp() / "verify" / "payload" / "tool"
    _require(tool_dir, "先运行 --stage payload")

    if ctx.baseline_partition is not None:
        for candidate_root in (tool_dir, build_tmp() / "verify" / "super"):
            target = candidate_root / f"{ctx.baseline_partition}.img"
            if target.is_file():
                log(f"指定基准镜像: {target.name} "
                    f"({target.stat().st_size / 1024**2:.1f} MiB)")
                return target
        raise VerifyError(f"未找到指定分区镜像: {ctx.baseline_partition}.img")

    super_images = list(tool_dir.glob("super*.img"))
    if super_images:
        super_img = max(super_images, key=lambda p: p.stat().st_size)
        log(f"发现 {super_img.name} ({super_img.stat().st_size / 1024**3:.2f} GiB), 拆分 super")
        super_dir = build_tmp() / "verify" / "super"
        safe_rmtree("verify/super")
        super_dir = ensure_dir(super_dir)
        toolchain.imgkit_unpack(ctx.imgkit_bin, rel_to_repo(super_img),
                                rel_to_repo(super_dir))
        candidates = list(super_dir.glob("*.img"))
        source = "super"
    else:
        log("payload 无 super 镜像 (动态分区直出), 取 payload 输出中最大镜像")
        candidates = list(tool_dir.glob("*.img"))
        source = "payload"

    if not candidates:
        raise VerifyError(f"候选镜像为空: {source}")
    largest = max(candidates, key=lambda p: p.stat().st_size)
    log(f"最大镜像: {largest.name} ({largest.stat().st_size / 1024**3:.2f} GiB, 来源 {source})")
    return largest


def stage_baseline(ctx: Context) -> list[CaseResult]:
    # 以 erofs-utils 提取最大子镜像, 生成基准目录
    log("阶段 baseline: erofs-utils 基准提取")
    start = time.monotonic()
    largest = _select_baseline_partition(ctx)

    baseline_root = build_tmp() / "verify" / "baseline"
    safe_rmtree("verify/baseline")
    baseline_root = ensure_dir(baseline_root)
    toolchain.run_command([str(ctx.erofs_extract_bin), "-i", rel_to_repo(largest),
                           "-x", "-o", rel_to_repo(baseline_root)])

    baseline_tree = baseline_root / largest.stem
    if not baseline_tree.is_dir():
        raise VerifyError(f"基准目录未生成: {rel_to_repo(baseline_tree)}")

    snapshot = compare.snapshot_tree(baseline_tree, config.COMPARE_IGNORE_NAMES)
    log(f"基准目录 {largest.stem}/: {compare.tree_stats(snapshot)}")

    # 基准树已建立, 磁盘紧张时清理提取链产物 (fs 阶段仅依赖 baseline/)
    if "baseline" in ctx.prune:
        safe_rmtree("verify/payload/tool")
        safe_rmtree("verify/super")
        safe_rmfile("verify/rom/payload.bin")
        log("prune: 已清理 payload 提取产物")
    return [CaseResult("baseline:erofs-tools", "PASS", time.monotonic() - start,
                       compare.tree_stats(snapshot))]


@dataclass
class FsCase:
    name: str
    fs_type: str
    algo: str | None = None
    level: int | None = None


def _build_fs_cases(ctx: Context) -> list[FsCase]:
    cases = []
    if "ext4" in ctx.fs_types:
        cases.append(FsCase("ext4", "ext4"))
    if "f2fs" in ctx.fs_types:
        cases.append(FsCase("f2fs", "f2fs"))
    if "erofs" in ctx.fs_types:
        matrix = []
        if "default" in ctx.erofs_tiers:
            matrix += config.EROFES_TIER_DEFAULT
        if "high" in ctx.erofs_tiers:
            matrix += config.EROFES_TIER_HIGH
        for algo, level in matrix:
            if algo not in ctx.erofs_algos:
                continue
            name = f"erofs_{algo}" if level is None else f"erofs_{algo}_{level}"
            cases.append(FsCase(name, "erofs", algo, level))
    return cases


def _estimate_fs_size(snapshot: dict) -> int:
    # ext4/f2fs 镜像容量估算: 数据量系数 + 余量后对齐
    total = sum(e.size for e in snapshot.values() if e.kind == "file")
    size = int(total * config.FS_SIZE_FACTOR) + config.FS_SIZE_MARGIN
    return (size + config.FS_SIZE_ALIGN - 1) // config.FS_SIZE_ALIGN * config.FS_SIZE_ALIGN


def stage_fs(ctx: Context) -> list[CaseResult]:
    # 打包提取往返验证: 基准目录 -> 本工具打包 -> 本工具提取 -> 与基准比对
    baseline_tree = build_tmp() / "verify" / "baseline"
    if not baseline_tree.is_dir():
        raise VerifyError("先运行 --stage baseline")
    entries = [p for p in baseline_tree.iterdir()
               if p.is_dir() and not compare.is_ignored(p.name, config.COMPARE_IGNORE_NAMES)]
    if len(entries) != 1:
        raise VerifyError(f"基准目录应只含一个树根, 实际: {[p.name for p in entries]}")
    baseline_tree = entries[0]

    snapshot = compare.snapshot_tree(baseline_tree, config.COMPARE_IGNORE_NAMES)
    log(f"基准树: {compare.tree_stats(snapshot)}")
    # 指纹绑定基准树, 基准变更后旧 case 产物自动作废
    baseline_fp = (f"{baseline_tree.name}:"
                   f"{len(snapshot)}:"
                   f"{sum(e.size for e in snapshot.values() if e.kind == 'file')}")

    results = []
    for case in _build_fs_cases(ctx):
        case_start = time.monotonic()
        try:
            # safe_rmtree 接收 build/tmp 相对路径, 子进程参数用仓库根相对路径
            case_dir = build_tmp() / "verify" / "fs" / case.name
            done_marker = case_dir / ".done"
            image_path = case_dir / f"{case.name}.img"
            extracted_tree = case_dir / case.name
            done_valid = (done_marker.is_file() and image_path.is_file()
                          and extracted_tree.is_dir()
                          and done_marker.read_text(encoding="utf-8").strip() == baseline_fp)
            if done_valid:
                log(f"fs:{case.name} 产物已存在, 直接比对")
            else:
                safe_rmtree(f"verify/fs/{case.name}")
                case_dir = ensure_dir(case_dir)
                image_rel = rel_to_repo(image_path)
                source_rel = rel_to_repo(baseline_tree)

                size = _estimate_fs_size(snapshot) if case.fs_type in ("ext4", "f2fs") else None
                toolchain.imgkit_pack(ctx.imgkit_bin, case.fs_type, source_rel,
                                      image_rel, size, case.algo, case.level)
                toolchain.imgkit_unpack(ctx.imgkit_bin, image_rel, rel_to_repo(case_dir))
                done_marker.write_text(baseline_fp, encoding="utf-8")

            extracted = compare.snapshot_tree(extracted_tree, config.COMPARE_IGNORE_NAMES)
            diff = compare.compare_trees(snapshot, extracted)
            status = "PASS" if diff.is_clean else "FAIL"
            results.append(CaseResult(f"fs:{case.name}", status,
                                      time.monotonic() - case_start, diff.summary()))
            log(f"fs:{case.name} {status} ({time.monotonic() - case_start:.0f}s)")
        except VerifyError as err:
            if not ctx.keep_going:
                raise
            results.append(CaseResult(f"fs:{case.name}", "FAIL",
                                      time.monotonic() - case_start, str(err)))
        # 磁盘紧张时按需清理单 case 产物, 只保留报告结果
        if "cases" in ctx.prune:
            safe_rmtree(f"verify/fs/{case.name}")
            log(f"fs:{case.name} 已清理产物")
    return results
