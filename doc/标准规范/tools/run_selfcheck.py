#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""机检自身的回归测试：正例必须过，反例必须拦。

    python tools/run_selfcheck.py

每个用例声明"期望退出码"，实际不符即失败。这保证门禁不会退化成永远放行。
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

ROOT = Path(__file__).resolve().parent.parent
XTASK = ROOT / "tools" / "xtask.py"

CASES = [
    ("规范自检（编号/判定/引用/表格/链接）", ["spec-check"], 0),
    ("需求卡-合法样例", ["req-lint", "docs/req/*.json"], 0),
    ("需求卡-反例必须拦", ["req-lint", "tests/fixtures/req-bad.json"], 1),
    ("用例-合法样例", ["testcase-lint", "docs/test/*.json"], 0),
    ("发布单-合法样例", ["release-lint", "docs/release/*.json"], 0),
    ("评审意见 schema", ["artifact-lint", "--schema", "review", "docs/review/*.json"], 0),
    ("交接包-合法（通过必须附证据）", ["artifact-lint", "--schema", "handoff", "docs/handoff/*.json"], 0),
    ("交接包-假绿必须拦", ["artifact-lint", "--schema", "handoff", "tests/fixtures/handoff-bad.json"], 1),
    ("自进化提案 schema", ["artifact-lint", "--schema", "proposal", "docs/proposal/*.json"], 0),
    ("覆盖追溯", ["trace"], 0),
    ("提交信息-合法", ["commit-lint", "--message", "feat(export): 支持按状态筛选后导出 CSV\n\n改为异步队列。\n\nRefs: REQ-0137"], 0),
    ("提交信息-反例必须拦", ["commit-lint", "--message", "update code"], 1),
    ("错误码表-合法", ["errcodes"], 0),
    ("错误码表-反例必须拦", ["errcodes", "--table", "tests/fixtures/errcodes-bad.json"], 1),
    ("依赖方向-合法", ["dep-graph", "--src", "tests/fixtures/src-good"], 0),
    ("依赖方向-反例必须拦（反向+环）", ["dep-graph", "--src", "tests/fixtures/src-bad"], 1),
    ("契约示例-合法", ["contract-test"], 0),
    ("契约示例-反例必须拦", ["contract-test", "--examples", "tests/fixtures/contract-bad.json"], 1),
    ("评测门槛-合法", ["eval-gate", "docs/eval/*.json"], 0),
    ("评测门槛-反例必须拦", ["eval-gate", "tests/fixtures/eval-bad.json"], 1),
    ("智能体注册表-合法", ["agent-registry"], 0),
    ("智能体注册表-反例必须拦", ["agent-registry", "--registry", "tests/fixtures/registry-bad.json"], 1),
    ("改动行覆盖-80% 达标", ["coverage"], 0),
    ("改动行覆盖-90% 不达标必须拦", ["coverage", "--fail-under", "90"], 1),
    ("库内自检（库即源）", ["@script", "tools/verify_vault.py"], 0),
]


def run(args):
    if args and args[0] == "@script":
        cmd = [sys.executable, str(ROOT / args[1]), *args[2:]]
    else:
        cmd = [sys.executable, str(XTASK), *args]
    return subprocess.run(
        cmd,
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )


def main():
    failed = 0
    for name, args, expect in CASES:
        proc = run(args)
        ok = proc.returncode == expect
        mark = "PASS" if ok else "FAIL"
        print(f"{mark}  {name}  (exit={proc.returncode}, 期望 {expect})")
        if not ok:
            failed += 1
            print("      stdout:", (proc.stdout or "").strip().replace("\n", "\n      "))
            print("      stderr:", (proc.stderr or "").strip()[:500])
    print(f"\n{'PASS' if not failed else 'FAIL'}  run_selfcheck  ({len(CASES) - failed}/{len(CASES)} 通过)")
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
