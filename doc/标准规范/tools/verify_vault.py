#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""库内自检（库即源模式）。

    python tools/verify_vault.py

不依赖任何外部源文件，只检查库自身是否自洽：
1. 布局、front matter 必填字段、id 格式
2. 规则编号全局唯一、每条都有「判定」
3. 章节连续、表格有分隔行、代码围栏闭合
4. 所有 [[wikilink]] 目标存在（无悬空）
5. 每篇「关联」段与 depends_on 反推一致
6. 总览的清单、ID、规则数与正文一致
7. 无遗留导入横幅、无本机绝对路径
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import spec_paths as sp

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

FENCE = re.compile(r"^```")
RULE = re.compile(r"^\*\*([A-Z]{3}-\d+\.\d+)（(必须|应)）\*\*", re.M)
WIKI = re.compile(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]")
CHAPTER = re.compile(r"^## (\d+) ")
REQUIRED = ["id", "title", "version", "status", "layer", "audience", "enforcement", "owner", "depends_on"]
ABS_PATH = re.compile(r"(?<![A-Za-z])[A-Za-z]:\\\\")


def split_fm(text: str):
    if not text.startswith("---\n"):
        return None, text
    end = text.find("\n---\n", 4)
    if end == -1:
        return None, text
    return text[4:end], text[end + 5:]


def parse_meta(raw):
    meta = {}
    for line in (raw or "").splitlines():
        if ":" in line and not line.startswith(" "):
            k, v = line.split(":", 1)
            meta[k.strip()] = v.strip()
    return meta


def parse_deps(meta):
    raw = meta.get("depends_on", "")
    if not raw.startswith("["):
        return []
    return [x.strip() for x in raw[1:-1].split(",") if x.strip()]


def section_lines(body: str, title: str):
    lines = body.splitlines()
    start = next((i for i, ln in enumerate(lines) if ln.strip() == title), None)
    if start is None:
        return None
    end = next((j for j in range(start + 1, len(lines)) if lines[j].startswith("## ")), len(lines))
    return lines[start:end]


def main():
    errors = []
    try:
        sp.check_layout()
    except SystemExit as e:
        print(e)
        return 1

    all_notes = {p.stem: p for p in sp.VAULT.rglob("*.md")}
    docs = sp.spec_files()
    if not docs:
        print("规范目录为空：", sp.SPEC_DIR)
        return 1

    rule_owner = {}
    deps_by_id = {}
    id_by_stem = {}
    seen_ids = {}
    overview_stem = sp.OVERVIEW.stem

    for p in docs:
        text = p.read_text(encoding="utf-8")
        raw, body = split_fm(text)
        meta = parse_meta(raw)

        if raw is None:
            errors.append(f"[GOV §6] {p.name} 缺少 YAML 头")
            continue
        for k in REQUIRED:
            if k not in meta:
                errors.append(f"[GOV §6] {p.name} 头部缺字段 {k}")
        if not re.fullmatch(r"STD-[A-Z]{2,4}-\d{2,3}", meta.get("id", "")):
            errors.append(f"[GOV §3] {p.name} id 格式非法：{meta.get('id')}")

        id_by_stem[p.stem] = meta.get("id", "")
        deps_by_id[meta.get("id", "")] = parse_deps(meta)
        if meta.get("id") in seen_ids:
            errors.append(f"[GOV §3] 文档 ID 重复 {meta.get('id')}：{seen_ids[meta.get('id')]} 与 {p.name}")
        seen_ids[meta.get("id", "")] = p.name

        if "导入自 " in body:
            errors.append(f"[库即源] {p.name} 仍留有导入横幅")
        if ABS_PATH.search(body):
            errors.append(f"[可移植] {p.name} 正文含本机绝对路径")

        # 规则与判定
        lines = body.splitlines()
        for i, ln in enumerate(lines):
            m = RULE.match(ln)
            if not m:
                continue
            rid = m.group(1)
            if rid in rule_owner:
                errors.append(f"[GOV §11] 规则编号重复 {rid}：{rule_owner[rid]} 与 {p.name}")
            rule_owner[rid] = p.name
            has = False
            for nxt in lines[i + 1:]:
                if RULE.match(nxt) or nxt.startswith("## "):
                    break
                if nxt.startswith("判定："):
                    has = True
                    break
            if not has:
                errors.append(f"[GOV §6] {p.name}:{i + 1} {rid} 缺少判定")

        # 章节、表格、围栏（代码块里的 "## " 不算章节）
        chs, in_fence_ch = [], False
        for ln in lines:
            if FENCE.match(ln):
                in_fence_ch = not in_fence_ch
                continue
            if in_fence_ch:
                continue
            m_ch = CHAPTER.match(ln)
            if m_ch:
                chs.append(int(m_ch.group(1)))
        if chs:
            for a, b in zip(chs, chs[1:]):
                if b != a + 1:
                    errors.append(f"[GOV §6] {p.name} 章节不连续：{a} → {b}")
        in_fence, prev_table, expect_sep = False, False, False
        for i, ln in enumerate(lines, 1):
            if FENCE.match(ln):
                in_fence = not in_fence
                prev_table = False
                continue
            if in_fence:
                continue
            if ln.startswith("|"):
                if not prev_table:
                    expect_sep = True
                elif expect_sep:
                    if not re.match(r"^\|[\s:\-|]+\|$", ln.strip()):
                        errors.append(f"[GOV §6] {p.name}:{i} 表格缺分隔行")
                    expect_sep = False
                prev_table = True
            else:
                prev_table = False
                expect_sep = False
        if in_fence:
            errors.append(f"[GOV §6] {p.name} 代码围栏未闭合")

        # 链接
        for target in WIKI.findall(text):
            if target not in all_notes:
                errors.append(f"[链接] {p.name} → [[{target}]] 不存在")

    # 附录：属性、链接、结构、索引链接（不参与依赖图）
    for p in sp.appendix_files():
        text = p.read_text(encoding="utf-8")
        raw, body = split_fm(text)
        meta = parse_meta(raw)
        if raw is None:
            errors.append(f"[GOV §6] {p.name} 缺少 YAML 头")
            continue
        for k in REQUIRED:
            if k not in meta:
                errors.append(f"[GOV §6] {p.name} 头部缺字段 {k}")
        doc_id = meta.get("id", "")
        if not re.fullmatch(r"STD-[A-Z]{2,4}-\d{2,3}", doc_id):
            errors.append(f"[GOV §3] {p.name} id 格式非法：{doc_id}")
        if doc_id in seen_ids:
            errors.append(f"[GOV §3] 文档 ID 重复 {doc_id}：{seen_ids[doc_id]} 与 {p.name}")
        seen_ids[doc_id] = p.name

        if "导入自 " in body:
            errors.append(f"[库即源] {p.name} 仍留有导入横幅")
        if ABS_PATH.search(body):
            errors.append(f"[可移植] {p.name} 正文含本机绝对路径")
        for target in WIKI.findall(text):
            if target not in all_notes:
                errors.append(f"[链接] {p.name} → [[{target}]] 不存在")
        if not re.search(rf"\[\[{re.escape(overview_stem)}(\||\]\])", text):
            errors.append(f"[关联] {p.name} 缺少指向「{overview_stem}」的索引链接")

        in_fence, prev_table, expect_sep = False, False, False
        for i, ln in enumerate(body.splitlines(), 1):
            if FENCE.match(ln):
                in_fence = not in_fence
                prev_table = False
                continue
            if in_fence:
                continue
            if ln.startswith("|"):
                if not prev_table:
                    expect_sep = True
                elif expect_sep:
                    if not re.match(r"^\|[\s:\-|]+\|$", ln.strip()):
                        errors.append(f"[GOV §6] {p.name}:{i} 表格缺分隔行")
                    expect_sep = False
                prev_table = True
            else:
                prev_table = False
                expect_sep = False
        if in_fence:
            errors.append(f"[GOV §6] {p.name} 代码围栏未闭合")

    # 关联段一致性
    downstream = {}
    for doc_id, deps in deps_by_id.items():
        for dep in deps:
            if dep in deps_by_id:
                downstream.setdefault(dep, set()).add(doc_id)
            else:
                errors.append(f"[依赖] {doc_id} 依赖 {dep}，库中无此文档")

    stem_by_id = {v: k for k, v in id_by_stem.items()}
    for p in docs:
        text = p.read_text(encoding="utf-8")
        _, body = split_fm(text)
        sec = section_lines(body, "## 关联")
        if sec is None:
            errors.append(f"[关联] {p.name} 缺少「关联」段")
            continue
        up = next((ln for ln in sec if "上游：" in ln), "")
        down = next((ln for ln in sec if "下游：" in ln), "")
        got_up = set(WIKI.findall(up))
        got_down = set(WIKI.findall(down))
        doc_id = id_by_stem.get(p.stem, "")
        want_up = {stem_by_id[d] for d in deps_by_id.get(doc_id, []) if d in stem_by_id}
        want_down = {stem_by_id[d] for d in downstream.get(doc_id, set()) if d in stem_by_id}
        if got_up != want_up:
            errors.append(f"[关联] {p.name} 上游不符：{sorted(got_up)} ≠ {sorted(want_up)}")
        if got_down != want_down:
            errors.append(f"[关联] {p.name} 下游不符：{sorted(got_down)} ≠ {sorted(want_down)}")

    # 总览
    if not sp.OVERVIEW.exists():
        errors.append(f"[总览] 缺少 {sp.OVERVIEW.name}")
    else:
        text = sp.OVERVIEW.read_text(encoding="utf-8")
        rows = re.findall(r"^\| \[\[([^\]|]+)\]\] \| `([^`]+)` \| ([^|]+) \|", text, re.M)
        if len(rows) != len(docs):
            errors.append(f"[总览] 清单 {len(rows)} 行 ≠ 正文 {len(docs)} 篇")
        total = 0
        for stem, doc_id, n in rows:
            if stem not in all_notes:
                errors.append(f"[总览] 引用不存在的笔记 {stem}")
                continue
            real = len(RULE.findall(all_notes[stem].read_text(encoding="utf-8")))
            total += real
            claimed = n.strip()
            if claimed not in ("—", str(real)):
                errors.append(f"[总览] {stem} 规则数 {claimed} ≠ 实际 {real}")
            if id_by_stem.get(stem) != doc_id:
                errors.append(f"[总览] {stem} 的 ID {doc_id} ≠ 正文 {id_by_stem.get(stem)}")
        for target in WIKI.findall(text):
            if target not in all_notes:
                errors.append(f"[总览] 链接 [[{target}]] 不存在")

    print(f"检查库：{sp.VAULT}")
    print(f"  正文 {len(docs)} 篇，规则 {len(rule_owner)} 条，问题 {len(errors)} 个")
    for e in errors:
        print("  FAIL " + e)
    print("PASS  vault-check" if not errors else "FAIL  vault-check")
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
