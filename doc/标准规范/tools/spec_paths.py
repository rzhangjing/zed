#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""规范库的路径约定（唯一来源）。

布局（相对，不含任何绝对路径）：

    <vault>/
        系统研发规范/       000–700 正文（唯一事实源）
        系统研发附录/       检查报告、规划清单
        系统研发规范总览.md  索引
    <repo>/                 tools/ docs/ tests/（机检代码与工件）

库根可用环境变量 SPEC_VAULT 覆盖，默认取仓库目录的兄弟目录「规范仓库」。
所有脚本都应从这里取路径，不要再各写各的绝对路径。
"""

from __future__ import annotations

import os
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parent

VAULT = Path(os.environ.get("SPEC_VAULT", REPO.parent / "规范仓库")).resolve()
SPEC_DIR = VAULT / "系统研发规范"
APPENDIX_DIR = VAULT / "系统研发附录"
OVERVIEW = VAULT / "系统研发规范总览.md"

SCHEMA_DIR = TOOLS / "schemas"


def spec_files() -> list[Path]:
    """正文：文件名以数字开头的 md。"""
    return sorted(p for p in SPEC_DIR.glob("*.md") if p.name[:1].isdigit())


def appendix_files() -> list[Path]:
    return sorted(APPENDIX_DIR.glob("*.md"))


def check_layout() -> None:
    missing = [p for p in (SPEC_DIR, APPENDIX_DIR, OVERVIEW) if not p.exists()]
    if missing:
        raise SystemExit(
            "规范库布局不完整：\n  " + "\n  ".join(str(p) for p in missing)
            + "\n可用环境变量 SPEC_VAULT 指定库根。"
        )
