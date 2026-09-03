# 验证结果收集与汇总输出。

import time
from dataclasses import dataclass, field


@dataclass
class CaseResult:
    name: str
    status: str          # PASS / FAIL / SKIP
    elapsed: float
    detail: str = ""


def log(message: str) -> None:
    stamp = time.strftime("%H:%M:%S")
    print(f"[{stamp}] {message}")


def print_report(results: list[CaseResult]) -> int:
    # 逐条输出并汇总, 返回失败数量
    print("\n========== 验证报告 ==========")
    for result in results:
        marker = {"PASS": "[PASS]", "FAIL": "[FAIL]", "SKIP": "[SKIP]"}[result.status]
        line = f"{marker} {result.name} ({result.elapsed:.1f}s)"
        if result.detail:
            line += f"  {result.detail}"
        print(line)

    passed = sum(1 for r in results if r.status == "PASS")
    failed = sum(1 for r in results if r.status == "FAIL")
    skipped = sum(1 for r in results if r.status == "SKIP")
    print(f"\n合计 {len(results)} 项: {passed} 通过, {failed} 失败, {skipped} 跳过")
    return failed
