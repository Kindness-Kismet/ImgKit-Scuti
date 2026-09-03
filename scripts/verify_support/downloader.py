# 系统代理探测、镜像测速、断点续传下载与压缩包解压。

import socket
import tarfile
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

from .paths import VerifyError

CHUNK_SIZE = 1024 * 1024
PROGRESS_STEP = 256 * 1024 * 1024


def build_opener() -> urllib.request.OpenerDirector:
    # 跨平台系统代理: Windows 注册表 / macOS 系统配置 / 环境变量, 有则优先使用
    proxies = urllib.request.getproxies()
    if proxies:
        print(f"[net] 使用系统代理: {proxies}")
    return urllib.request.build_opener(urllib.request.ProxyHandler(proxies))


def _download_from(opener, url: str, dest: Path, expected_size: int | None,
                   label: str, log_prefix: str) -> None:
    # 单镜像下载核心: 从已有字节数续传, 写满期望大小或断流返回
    downloaded = dest.stat().st_size if dest.exists() else 0
    if expected_size is not None and downloaded == expected_size:
        return
    start = time.monotonic()
    last_report = downloaded

    request = urllib.request.Request(url)
    if downloaded > 0:
        request.add_header("Range", f"bytes={downloaded}-")
    with opener.open(request, timeout=60) as response:
        if downloaded > 0 and response.status != 206:
            # 服务器不支持续传, 放弃已有部分从头写
            downloaded = 0
        mode = "ab" if downloaded > 0 else "wb"
        with open(dest, mode) as out:
            while True:
                block = response.read(CHUNK_SIZE)
                if not block:
                    break
                out.write(block)
                downloaded += len(block)
                if downloaded - last_report >= PROGRESS_STEP:
                    speed = (downloaded - last_report) / max(time.monotonic() - start, 0.001) / 1024 / 1024
                    print(f"{log_prefix} {downloaded / 1024 / 1024:.0f} MiB (近段 {speed:.1f} MiB/s)")
                    last_report = downloaded
    if expected_size is not None and dest.stat().st_size != expected_size:
        raise VerifyError(f"{label} 断流: {dest.stat().st_size} < {expected_size}")


def download(opener, url: str, dest: Path,
             expected_size: int | None, label: str) -> Path:
    # 单链接下载入口, 断流时自动重试一次续传
    if dest.exists() and expected_size is not None and dest.stat().st_size == expected_size:
        print(f"[net] 已存在, 跳过 {label}: {dest.name}")
        return dest
    if dest.exists() and expected_size is not None and dest.stat().st_size > expected_size:
        dest.unlink()
    dest.parent.mkdir(parents=True, exist_ok=True)

    for attempt in range(2):
        try:
            _download_from(opener, url, dest, expected_size, label, f"[net] {label}:")
            break
        except VerifyError:
            if attempt == 1:
                raise
            print(f"[net] {label}: 断流, 尝试续传")
    print(f"[net] {label} 完成: {dest.stat().st_size / 1024 / 1024:.0f} MiB")
    return dest


def speed_test(opener, mirrors: dict[str, str], test_bytes: int,
               per_mirror_timeout: float) -> list[tuple[str, float]]:
    # 对每个镜像拉取头部样本测速, 返回按速度降序的 (名称, MiB/s) 列表
    results: list[tuple[str, float]] = []
    for name, url in mirrors.items():
        try:
            start = time.monotonic()
            request = urllib.request.Request(url)
            request.add_header("Range", f"bytes=0-{test_bytes - 1}")
            received = 0
            with opener.open(request, timeout=per_mirror_timeout) as response:
                if response.status not in (200, 206):
                    print(f"[net] 镜像测速 {name}: HTTP {response.status}, 不可用")
                    continue
                while received < test_bytes:
                    block = response.read(CHUNK_SIZE)
                    if not block or time.monotonic() - start > per_mirror_timeout:
                        break
                    received += len(block)
            elapsed = time.monotonic() - start
            speed = received / 1024 / 1024 / max(elapsed, 0.001)
            results.append((name, speed))
            print(f"[net] 镜像测速 {name}: {speed:.2f} MiB/s "
                  f"({received / 1024 / 1024:.0f} MiB / {elapsed:.1f}s)")
        except (urllib.error.URLError, socket.timeout, OSError) as err:
            print(f"[net] 镜像测速 {name}: 失败 {err}")
    return sorted(results, key=lambda item: item[1], reverse=True)


def download_mirrors(opener, mirrors: dict[str, str], dest: Path,
                     expected_size: int, label: str) -> Path:
    # 多镜像下载: 各镜像字节一致, 断流或失败自动切换下一镜像续传
    if dest.exists() and dest.stat().st_size == expected_size:
        print(f"[net] 已存在, 跳过 {label}: {dest.name}")
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)

    ordered = list(mirrors.items())
    tried: list[str] = []
    while ordered:
        name, url = ordered.pop(0)
        tried.append(name)
        try:
            _download_from(opener, url, dest, expected_size, label,
                           f"[net] {label}@{name}:")
            if dest.stat().st_size == expected_size:
                print(f"[net] {label} 完成 (@{name}): "
                      f"{expected_size / 1024 / 1024:.0f} MiB")
                return dest
            print(f"[net] {label}: @{name} 断流, 切换下一镜像")
        except (urllib.error.URLError, socket.timeout, OSError, VerifyError) as err:
            print(f"[net] {label}: @{name} 失败 ({err}), 切换下一镜像")
    raise VerifyError(f"{label} 全部镜像尝试失败: {tried}")


def extract_archive(archive: Path, dest: Path) -> None:
    # 解压 zip/tar.gz 到目标目录, 类 Unix 下补齐可执行位
    dest.mkdir(parents=True, exist_ok=True)
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as zf:
            for info in zf.infolist():
                zf.extract(info, dest)
                mode = (info.external_attr >> 16) & 0o777
                if mode:
                    (dest / info.filename).chmod(mode)
    else:
        with tarfile.open(archive, "r:gz") as tf:
            tf.extractall(dest, filter="data")
