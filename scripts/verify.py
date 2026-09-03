# imgkit 本地验证入口。
# 流程: 下载 ROM -> 双工具 payload 提取比对 -> erofs-utils 基准提取 -> ext/erofs/f2fs 打包提取往返比对。
# 所有阶段可独立选择, 文件操作基于仓库相对路径, 删除仅限 build/tmp。

import argparse
import os
import sys
import time
from pathlib import Path

# Windows 控制台编码统一, 避免中文输出乱码
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_support import config, downloader, report, stages, toolchain
from verify_support.paths import VerifyError
from verify_support.report import log
from verify_support.stages import Context


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="imgkit 本地验证: payload 与文件系统打包提取往返比对")
    parser.add_argument("--stage", default="all",
                        help=f"阶段选择, 逗号分隔: {','.join(config.ALL_STAGES)} 或 all")
    parser.add_argument("--fs", default="ext4,erofs,f2fs",
                        help="fs 阶段文件系统过滤, 逗号分隔")
    parser.add_argument("--erofs-algo", default="lz4,lz4hc,lzma,deflate,zstd",
                        help="erofs 压缩算法过滤, 逗号分隔")
    parser.add_argument("--erofs-tier", default="default",
                        help="erofs 级别档位: default / high / all")
    parser.add_argument("--imgkit-bin", default=None,
                        help="显式指定 imgkit_scuti 二进制路径")
    parser.add_argument("--no-build", action="store_true",
                        help="缺少 imgkit 二进制时不自动 cargo build")
    parser.add_argument("--rom-mirror", default="auto",
                        help="ROM 镜像选择: auto (测速选最快) 或 "
                             + "/".join(config.ROM_MIRRORS))
    parser.add_argument("--baseline-partition", default=None,
                        help="基准镜像分区名, 如 system_ext (默认取最大)")
    parser.add_argument("--keep-going", action="store_true",
                        help="验证项失败后继续执行其余项")
    parser.add_argument("--prune", default="",
                        help="分级清理, 逗号分隔: payload (删 ref 与 ROM zip), "
                             "baseline (删提取产物), cases (单 case 验完即清)")
    return parser.parse_args()


def _parse_list(raw: str, allowed: list[str], option: str) -> list[str]:
    values = [v.strip() for v in raw.split(",") if v.strip()]
    unknown = [v for v in values if v not in allowed]
    if unknown or not values:
        raise SystemExit(f"--{option} 无效值: {unknown or raw}, 可选: {allowed}")
    return values


def _parse_tier(raw: str) -> list[str]:
    if raw == "all":
        return ["default", "high"]
    if raw in ("default", "high"):
        return [raw]
    raise SystemExit(f"--erofs-tier 无效值: {raw}, 可选: default / high / all")


def _parse_prune(raw: str) -> list[str]:
    # 空值表示不做任何清理
    if not raw.strip():
        return []
    return _parse_list(raw, ["payload", "baseline", "cases"], "prune")


def _parse_stages(raw: str) -> list[str]:
    if raw == "all":
        return list(config.ALL_STAGES)
    values = _parse_list(raw, config.ALL_STAGES, "stage")
    # 后续阶段依赖前置产物, 按固定顺序执行
    return [s for s in config.ALL_STAGES if s in values]


def main() -> int:
    args = parse_args()
    run_start = time.monotonic()

    opener = downloader.build_opener()
    try:
        pdg_bin, erofs_bin = toolchain.ensure_tools(opener)
        imgkit_bin = toolchain.resolve_imgkit(opener, args.imgkit_bin, args.no_build)
    except VerifyError as err:
        log(f"工具准备失败: {err}")
        return 1
    log(f"工具就绪: imgkit={imgkit_bin.name}, pdg={pdg_bin.name}, erofs={erofs_bin.name}")

    if args.rom_mirror != "auto" and args.rom_mirror not in config.ROM_MIRRORS:
        raise SystemExit(f"--rom-mirror 无效值: {args.rom_mirror}, "
                         f"可选: auto 或 {'/'.join(config.ROM_MIRRORS)}")

    ctx = Context(
        pdg_bin=pdg_bin,
        erofs_extract_bin=erofs_bin,
        imgkit_bin=imgkit_bin,
        fs_types=_parse_list(args.fs, config.ALL_FS_TYPES, "fs"),
        erofs_algos=_parse_list(args.erofs_algo, config.ALL_EROFS_ALGOS, "erofs-algo"),
        erofs_tiers=_parse_tier(args.erofs_tier),
        keep_going=args.keep_going,
        rom_mirror=args.rom_mirror,
        baseline_partition=args.baseline_partition,
        prune=_parse_prune(args.prune),
    )

    results = []
    stage_funcs = {
        "prepare": lambda: stages.stage_prepare(ctx, opener),
        "payload": lambda: stages.stage_payload(ctx),
        "baseline": lambda: stages.stage_baseline(ctx),
        "fs": lambda: stages.stage_fs(ctx),
    }
    for stage_name in _parse_stages(args.stage):
        try:
            results += stage_funcs[stage_name]()
        except VerifyError as err:
            log(f"阶段 {stage_name} 失败: {err}")
            if stage_name == "prepare" or not args.keep_going:
                results.append(report.CaseResult(f"stage:{stage_name}", "FAIL",
                                                 0.0, str(err)))
                break
            results.append(report.CaseResult(f"stage:{stage_name}", "FAIL",
                                             0.0, str(err)))

    failed = report.print_report(results)
    log(f"总耗时 {(time.monotonic() - run_start) / 60:.1f} 分钟")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
