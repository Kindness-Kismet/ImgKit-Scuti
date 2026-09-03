# 工作区路径解析与受控删除。
# 删除仅接受 build/tmp 内的相对路径, 逐级校验防止误删仓库外内容。

import os
import shutil
from pathlib import Path


class VerifyError(Exception):
    # 验证流程统一业务异常
    pass


def repo_root() -> Path:
    # 以脚本位置向上找 Cargo.toml 定位仓库根
    current = Path(__file__).resolve().parent.parent
    for candidate in [current, *current.parents]:
        if (candidate / "Cargo.toml").is_file():
            return candidate
    raise VerifyError("未找到仓库根目录 (缺少 Cargo.toml)")


def build_tmp() -> Path:
    return repo_root() / "build" / "tmp"


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


def rel_to_repo(path: Path) -> str:
    # 转为仓库根相对路径字符串, 供子进程参数使用
    return path.resolve().relative_to(repo_root()).as_posix()


def _validate_delete_target(rel: str) -> Path:
    # 词法校验: 必须是相对路径且归一化后落在 build/tmp 内, 不解析符号链接
    if os.path.isabs(rel) or rel.startswith("~"):
        raise VerifyError(f"拒绝删除绝对路径: {rel}")
    root = build_tmp()
    merged = os.path.normpath(os.path.join(str(root), rel))
    root_str = str(root)
    if merged == root_str or not merged.startswith(root_str + os.sep):
        raise VerifyError(f"拒绝删除越界路径: {rel}")
    return Path(merged)


def safe_rmtree(rel: str) -> None:
    target = _validate_delete_target(rel)
    if target.is_dir() and not target.is_symlink():
        shutil.rmtree(target)


def safe_rmfile(rel: str) -> None:
    target = _validate_delete_target(rel)
    if target.is_file() and not target.is_symlink():
        target.unlink()
