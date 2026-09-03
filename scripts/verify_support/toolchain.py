# 外部工具获取与 imgkit 二进制定位、子进程调用封装。

import os
import platform
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from . import config, downloader
from .paths import VerifyError, ensure_dir, repo_root

TOOLS_DIR = "build/tmp/downloads/tools"


@dataclass
class Toolchain:
    # pdg_bin: payload-dumper-go; erofs_extract_bin: extract.erofs; imgkit_bin: 本工具
    pdg_bin: Path
    erofs_extract_bin: Path
    imgkit_bin: Path


def current_platform() -> tuple[str, str]:
    machine = platform.machine().lower()
    arch = "arm64" if machine in ("arm64", "aarch64") else "amd64"
    os_name = {"win32": "windows", "darwin": "darwin", "linux": "linux"}.get(sys.platform)
    if os_name is None:
        raise VerifyError(f"不支持的系统: {sys.platform}")
    return os_name, arch


def _exe(name: str) -> str:
    return f"{name}.exe" if sys.platform == "win32" else name


def _locate_binary(tools_root: Path, name: str) -> Path | None:
    # 在解压目录中按文件名递归查找可执行文件
    for candidate in tools_root.rglob(_exe(name)):
        if candidate.is_file():
            return candidate
    return None


def _ensure_tool(opener, tools_root: Path, url_prefix: str, asset_name: str,
                 extract_dir_name: str, binary_name: str) -> Path:
    archive_dir = ensure_dir(tools_root / "archives")
    extract_dir = ensure_dir(tools_root / extract_dir_name)
    existing = _locate_binary(extract_dir, binary_name)
    if existing is not None:
        return existing

    archive = downloader.download(opener, url_prefix + asset_name,
                                  archive_dir / asset_name, None, asset_name)
    downloader.extract_archive(archive, extract_dir)
    binary = _locate_binary(extract_dir, binary_name)
    if binary is None:
        raise VerifyError(f"解压后未找到 {binary_name}: {extract_dir}")
    # zip 归档可能丢失可执行位, 类 Unix 下强制补齐
    binary.chmod(0o755)
    return binary


def resolve_imgkit(opener, explicit: str | None, no_build: bool) -> Path:
    # 优先级: 显式参数 > 环境变量 > cargo 构建产物 > 自动构建
    if explicit:
        path = Path(explicit)
        if not path.is_file():
            raise VerifyError(f"imgkit 二进制不存在: {explicit}")
        return path

    env_bin = os.environ.get("IMGKIT_BIN")
    if env_bin:
        path = Path(env_bin)
        if not path.is_file():
            raise VerifyError(f"IMGKIT_BIN 指向的文件不存在: {env_bin}")
        return path

    local = repo_root() / "target" / "release" / _exe("imgkit_scuti")
    if local.is_file():
        return local
    if no_build:
        raise VerifyError("未找到 imgkit 二进制, 且指定了 --no-build")

    print("[toolchain] 未找到 imgkit 二进制, 执行 cargo build --release")
    run_command(["cargo", "build", "--release"])
    if not local.is_file():
        raise VerifyError("cargo build 完成但仍未找到二进制")
    return local


def ensure_tools(opener) -> tuple[Path, Path]:
    # 返回 (payload-dumper-go, extract.erofs) 可执行路径
    os_name, arch = current_platform()
    tools_root = repo_root() / TOOLS_DIR

    pdg_asset = config.PDG_ASSET.get((os_name, arch))
    if pdg_asset is None:
        raise VerifyError(f"payload-dumper-go 不支持该平台: {os_name}/{arch}")
    pdg_bin = _ensure_tool(opener, tools_root, config.PDG_URL_PREFIX,
                           pdg_asset, "payload-dumper-go", "payload-dumper-go")

    erofs_asset = config.EROFS_ASSET.get((os_name, arch))
    if erofs_asset is None:
        raise VerifyError(f"erofs-utils 不支持该平台: {os_name}/{arch}")
    erofs_bin = _ensure_tool(opener, tools_root, config.EROFS_URL_PREFIX,
                             erofs_asset, "erofs-utils", "extract.erofs")
    return pdg_bin, erofs_bin


def run_command(cmd: list[str]) -> None:
    # 以仓库根为工作目录执行命令, 相对路径传参避免绝对路径问题
    print(f"[exec] {' '.join(cmd)}")
    result = subprocess.run(cmd, cwd=str(repo_root()))
    if result.returncode != 0:
        raise VerifyError(f"命令失败 (exit {result.returncode}): {' '.join(cmd)}")


def imgkit_unpack(imgkit_bin: Path, image: str, out_dir: str,
                  partitions: list[str] | None = None) -> None:
    cmd = [str(imgkit_bin), "unpack", "-i", image, "-o", out_dir, "-l", "1"]
    for name in partitions or []:
        cmd += ["-p", name]
    run_command(cmd)


def imgkit_pack(imgkit_bin: Path, fs_type: str, source: str, output: str,
                size: int | None = None, compress: str | None = None,
                compress_level: int | None = None) -> None:
    cmd = [str(imgkit_bin), "pack", "--type", fs_type,
           "-s", source, "-o", output, "-l", "1"]
    if size is not None:
        cmd += ["-z", str(size)]
    if compress is not None:
        cmd += ["--compress", compress]
    if compress_level is not None:
        cmd += ["--compress-level", str(compress_level)]
    run_command(cmd)
