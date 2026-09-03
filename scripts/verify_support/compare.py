# 目录树快照与递归比对, 含符号链接与文件哈希。

import hashlib
import os
from dataclasses import dataclass, field
from fnmatch import fnmatch
from pathlib import Path

HASH_BLOCK = 4 * 1024 * 1024


@dataclass
class Entry:
    kind: str          # file / dir / link
    size: int = 0
    link_target: str = ""
    digest: str = ""


@dataclass
class TreeDiff:
    missing: list = field(default_factory=list)         # 基准有, 提取无
    extra: list = field(default_factory=list)           # 提取有, 基准无
    kind_mismatch: list = field(default_factory=list)   # 类型不一致
    size_mismatch: list = field(default_factory=list)
    content_mismatch: list = field(default_factory=list)
    link_mismatch: list = field(default_factory=list)

    @property
    def is_clean(self) -> bool:
        return not (self.missing or self.extra or self.kind_mismatch
                    or self.size_mismatch or self.content_mismatch
                    or self.link_mismatch)

    def summary(self, sample: int = 3) -> str:
        # 差异计数 + 每类前几条示例
        parts = []
        for field_name in ("missing", "extra", "kind_mismatch",
                           "size_mismatch", "content_mismatch", "link_mismatch"):
            items = getattr(self, field_name)
            if items:
                shown = ", ".join(str(i) for i in items[:sample])
                parts.append(f"{field_name}={len(items)} [{shown}]")
        return "; ".join(parts) if parts else "完全一致"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            block = f.read(HASH_BLOCK)
            if not block:
                break
            digest.update(block)
    return digest.hexdigest()


def _is_ignored(name: str, ignore_names: set[str]) -> bool:
    return any(fnmatch(name, pattern) for pattern in ignore_names)


def is_ignored(name: str, ignore_names: set[str]) -> bool:
    # 供阶段逻辑复用的公开判断
    return _is_ignored(name, ignore_names)


def snapshot_tree(root: Path, ignore_names: set[str]) -> dict[str, Entry]:
    # 相对路径 -> Entry; 符号链接不跟随
    result: dict[str, Entry] = {}

    def walk(rel: str) -> None:
        base = root / rel if rel else root
        for name in sorted(os.listdir(base)):
            if _is_ignored(name, ignore_names):
                continue
            child_rel = f"{rel}/{name}" if rel else name
            full = base / name
            if os.path.islink(full):
                result[child_rel] = Entry("link", link_target=os.readlink(full))
            elif os.path.isdir(full):
                result[child_rel] = Entry("dir")
                walk(child_rel)
            elif os.path.isfile(full):
                result[child_rel] = Entry("file", size=full.stat().st_size,
                                          digest=sha256_file(full))
            else:
                result[child_rel] = Entry("special")

    walk("")
    return result


def compare_trees(baseline: dict[str, Entry], extracted: dict[str, Entry]) -> TreeDiff:
    diff = TreeDiff()
    for rel in sorted(baseline):
        if rel not in extracted:
            diff.missing.append(rel)
            continue
        base_entry, out_entry = baseline[rel], extracted[rel]
        if base_entry.kind != out_entry.kind:
            diff.kind_mismatch.append(f"{rel} ({base_entry.kind}->{out_entry.kind})")
        elif base_entry.kind == "link" and base_entry.link_target != out_entry.link_target:
            diff.link_mismatch.append(f"{rel} -> {out_entry.link_target}")
        elif base_entry.kind == "file":
            if base_entry.size != out_entry.size:
                diff.size_mismatch.append(f"{rel} ({base_entry.size}->{out_entry.size})")
            elif base_entry.digest != out_entry.digest:
                diff.content_mismatch.append(rel)
    for rel in sorted(extracted):
        if rel not in baseline:
            diff.extra.append(rel)
    return diff


def tree_stats(snapshot: dict[str, Entry]) -> str:
    # 规模统计: 文件/目录/链接数与总字节数
    files = [e for e in snapshot.values() if e.kind == "file"]
    dirs = sum(1 for e in snapshot.values() if e.kind == "dir")
    links = sum(1 for e in snapshot.values() if e.kind == "link")
    total = sum(e.size for e in files)
    return (f"{len(files)} 文件 / {dirs} 目录 / {links} 链接, "
            f"共 {total / 1024 / 1024:.0f} MiB")
