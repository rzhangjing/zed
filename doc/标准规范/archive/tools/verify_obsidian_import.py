#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""独立复核 Obsidian 导入结果。

与 import_to_obsidian.py 不共享任何代码：这里做的是反向归一化——
把导入件还原成"纯文本"，与源文件的"纯文本"逐字比较。

检查四件事：
1. 内容一致性：去掉新增的 front matter / 横幅 / 关联段、把链接还原成文字后，两边必须完全相同
2. 链接完整性：所有 [[...]] 目标都必须在库中存在（不留悬空）
3. 关联正确性：每篇「关联」段里的上游/下游，必须与 front matter 的 depends_on 反推一致
4. 索引正确性：总览里的规则数与依赖关系，必须能从导入件重新算出同样的值

用法：python tools/verify_obsidian_import.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

SRC = Path(r"E:\Rust\zed\doc\标准规范")
VAULT = Path(r"E:\Rust\zed\doc\规范仓库")
DST = VAULT / "规范"
EXTRA_DIR = VAULT / "附录"

MAIN = sorted(p for p in SRC.glob("*.md") if p.name[:2].isdigit())
EXTRAS = ["规范检查报告.md", "研发规范体系清单.md", "研发工序执行规范清单.md"]

WIKI = re.compile(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]")
MDLINK = re.compile(r"\[([^\]\n]+)\]\(([^()\s]+?\.md)\)")
RULE = re.compile(r"^\*\*([A-Z]{3}-\d+\.\d+)（(?:必须|应)）\*\*", re.M)


def split_fm(text: str):
    if not text.startswith("---\n"):
        return "", text
    end = text.find("\n---\n", 4)
    if end == -1:
        return "", text
    return text[4:end], text[end + 5:]


def links_to_text(text: str) -> str:
    text = WIKI.sub(lambda m: m.group(2) or m.group(1), text)
    text = MDLINK.sub(lambda m: m.group(1), text)
    return text


def normalize_source(text: str) -> str:
    _, body = split_fm(text)
    return links_to_text(body)


def strip_related_section(body: str) -> str:
    """只删「关联」这一节，保留它后面的变更记录。"""
    lines = body.splitlines()
    start = next((i for i, ln in enumerate(lines) if ln.strip() == "## 关联"), None)
    if start is None:
        return body
    end = next((j for j in range(start + 1, len(lines)) if lines[j].startswith("## ")), len(lines))
    return "\n".join(lines[:start] + lines[end:])


def normalize_imported(text: str, is_extra: bool) -> str:
    _, body = split_fm(text)
    lines = body.splitlines()
    # 去掉导入横幅
    lines = [ln for ln in lines if not ln.startswith("> 导入自 ")]
    lines = [ln for ln in lines if ln.strip() != "> 规划视图，已被 000–700 正式规范取代，仅作背景。"]
    body = "\n".join(lines)
    body = strip_related_section(body)
    return links_to_text(body)


def canon(text: str) -> str:
    """统一空白：去掉行尾空格与首尾空行，其余保持原样。"""
    lines = [ln.rstrip() for ln in text.splitlines()]
    while lines and not lines[0].strip():
        lines.pop(0)
    while lines and not lines[-1].strip():
        lines.pop()
    return "\n".join(lines)


def first_diff(a: str, b: str, n=60):
    i = 0
    while i < min(len(a), len(b)) and a[i] == b[i]:
        i += 1
    return a[max(0, i - 30): i + n].replace("\n", "/"), b[max(0, i - 30): i + n].replace("\n", "/")


def parse_fm(text: str) -> dict:
    fm, _ = split_fm(text)
    out = {}
    for line in fm.splitlines():
        if ":" in line and not line.startswith(" "):
            k, v = line.split(":", 1)
            out[k.strip()] = v.strip()
    return out


def parse_deps(meta: dict):
    raw = meta.get("depends_on", "")
    if not raw.startswith("["):
        return []
    return [x.strip() for x in raw[1:-1].split(",") if x.strip()]


def main():
    errors = []
    checked = 0

    vault_notes = {p.stem: p for p in VAULT.rglob("*.md")}
    id_to_stem = {}
    deps_by_stem = {}

    # ---------- 1. 内容一致性
    for src in MAIN:
        dst = DST / src.name
        if not dst.exists():
            errors.append(f"[缺失] {dst}")
            continue
        a = canon(normalize_source(src.read_text(encoding="utf-8")))
        b = canon(normalize_imported(dst.read_text(encoding="utf-8"), False))
        checked += 1
        if a != b:
            sa, sb = first_diff(a, b)
            errors.append(f"[内容不一致] {src.name}\n    源: {sa!r}\n    库: {sb!r}")
        meta = parse_fm(dst.read_text(encoding="utf-8"))
        id_to_stem[meta.get("id", "")] = dst.stem
        deps_by_stem[dst.stem] = parse_deps(meta)

    for name in EXTRAS:
        src = SRC / name
        dst = EXTRA_DIR / name
        if not src.exists():
            continue
        if not dst.exists():
            errors.append(f"[缺失] {dst}")
            continue
        a = canon(normalize_source(src.read_text(encoding="utf-8")))
        b = canon(normalize_imported(dst.read_text(encoding="utf-8"), True))
        checked += 1
        if a != b:
            sa, sb = first_diff(a, b)
            errors.append(f"[内容不一致] {name}\n    源: {sa!r}\n    库: {sb!r}")

    # ---------- 2. 链接完整性
    all_links = 0
    dangling = []
    for p in list(DST.glob("*.md")) + list(EXTRA_DIR.glob("*.md")) + [VAULT / "总览.md"]:
        for m in WIKI.finditer(p.read_text(encoding="utf-8")):
            target = m.group(1).strip()
            all_links += 1
            if target not in vault_notes:
                dangling.append(f"{p.name} → [[{target}]]")
    if dangling:
        errors.append(f"[悬空链接] {len(dangling)} 处\n    " + "\n    ".join(dangling[:20]))

    # ---------- 3. 关联段正确性
    downstream = {}
    for stem, deps in deps_by_stem.items():
        for dep in deps:
            tgt = id_to_stem.get(dep)
            if tgt:
                downstream.setdefault(tgt, set()).add(stem)
            else:
                errors.append(f"[未知依赖] {stem} 依赖 {dep}，库里没有对应文档")

    for stem in deps_by_stem:
        p = DST / f"{stem}.md"
        text = p.read_text(encoding="utf-8")
        idx = text.find("## 关联")
        if idx == -1:
            errors.append(f"[缺少关联段] {stem}")
            continue
        block = text[idx:]
        up_line = next((ln for ln in block.splitlines() if "上游：" in ln), "")
        down_line = next((ln for ln in block.splitlines() if "下游：" in ln), "")
        got_up = {m[0] for m in WIKI.findall(up_line)} if up_line else set()
        got_down = {m[0] for m in WIKI.findall(down_line)} if down_line else set()
        want_up = {id_to_stem[d] for d in deps_by_stem.get(stem, []) if d in id_to_stem}
        want_down = downstream.get(stem, set())
        if got_up != want_up:
            errors.append(f"[上游不符] {stem}: 库 {sorted(got_up)} ≠ 期望 {sorted(want_up)}")
        if got_down != want_down:
            errors.append(f"[下游不符] {stem}: 库 {sorted(got_down)} ≠ 期望 {sorted(want_down)}")

    # ---------- 4. 总览正确性
    overview = (VAULT / "总览.md").read_text(encoding="utf-8")
    rows = re.findall(r"^\| \[\[([^\]|]+)\]\] \| `([^`]+)` \| ([^|]+) \|", overview, re.M)
    if len(rows) != len(MAIN):
        errors.append(f"[总览行数] {len(rows)} ≠ {len(MAIN)}")
    for stem, doc_id, rules in rows:
        p = DST / f"{stem}.md"
        if not p.exists():
            errors.append(f"[总览引用不存在] {stem}")
            continue
        real = len(RULE.findall(p.read_text(encoding="utf-8")))
        claimed = rules.strip()
        if claimed not in ("—", str(real)):
            errors.append(f"[规则数不符] {stem}: 总览 {claimed}，实际 {real}")
        if parse_fm(p.read_text(encoding="utf-8")).get("id") != doc_id:
            errors.append(f"[ID 不符] {stem}: 总览 {doc_id}")

    # ---------- 输出
    print(f"检查 {checked} 篇，链接 {all_links} 条，问题 {len(errors)} 个")
    for e in errors:
        print("  FAIL " + e)
    print("PASS  导入复核" if not errors else "FAIL  导入复核")
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
