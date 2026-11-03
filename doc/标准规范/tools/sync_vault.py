#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""库即源：重建「关联」段与总览。

    python tools/sync_vault.py

只做三件事，不碰正文：
1. 删掉历史导入横幅（「导入自 …」，那段话在库即源之后已错误）
2. 按 front matter 的 depends_on 重新生成每篇的「关联」段
3. 重新生成总览（清单表、依赖图、交叉引用、阅读路径）

链接目标统一用 [[系统研发规范总览|总览]]，因为总览文件已改名。
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

WIKI = re.compile(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]")
RULE = re.compile(r"^\*\*([A-Z]{3})-\d+\.\d+（", re.M)
OVERVIEW_STEM = sp.OVERVIEW.stem  # 系统研发规范总览

DOMAIN_TAG = {
    "GOV": "治理", "REQ": "需求", "ARC": "设计", "DES": "设计", "UI": "UI",
    "COD": "编码", "TST": "测试", "REL": "发布", "AGT": "智能体", "TPL": "模板",
}


def split_fm(text: str):
    if not text.startswith("---\n"):
        return "", text
    end = text.find("\n---\n", 4)
    if end == -1:
        return "", text
    return text[4:end], text[end + 5:]


def parse_meta(raw: str) -> dict:
    meta = {}
    for line in raw.splitlines():
        if ":" in line and not line.startswith(" "):
            k, v = line.split(":", 1)
            meta[k.strip()] = v.strip()
    return meta


def parse_deps(meta: dict) -> list[str]:
    raw = meta.get("depends_on", "")
    if not raw.startswith("["):
        return []
    return [x.strip() for x in raw[1:-1].split(",") if x.strip()]


def display(stem: str) -> str:
    return stem.split("-", 1)[1] if "-" in stem else stem


def strip_banner(body: str) -> str:
    return "\n".join(ln for ln in body.splitlines() if not ln.startswith("> 导入自 "))


def strip_related(body: str) -> str:
    lines = body.splitlines()
    start = next((i for i, ln in enumerate(lines) if ln.strip() == "## 关联"), None)
    if start is None:
        return body
    end = next((j for j in range(start + 1, len(lines)) if lines[j].startswith("## ")), len(lines))
    return "\n".join(lines[:start] + lines[end:])


def related_block(deps: list[str], down: list[str], by_id: dict) -> list[str]:
    def link(doc_id):
        stem = by_id[doc_id]
        return f"[[{stem}|{display(stem)}]]"

    lines = ["## 关联", ""]
    if deps:
        lines.append("- 上游：" + "、".join(link(d) for d in deps if d in by_id))
    else:
        lines.append("- 上游：无（本册是根）")
    if down:
        lines.append("- 下游：" + "、".join(link(d) for d in down))
    else:
        lines.append("- 下游：无")
    lines.append(f"- 索引：[[{OVERVIEW_STEM}|总览]]")
    return lines


def insert_block(body: str, block: list[str]) -> str:
    lines = body.strip("\n").splitlines()
    if "## 变更记录" in lines:
        i = lines.index("## 变更记录")
        if i > 0 and lines[i - 1].strip():
            lines.insert(i, "")
            i += 1
        lines[i:i] = block + [""]
    else:
        lines += [""] + block
    return "\n".join(lines) + "\n"


def main():
    sp.check_layout()
    docs = []
    for p in sp.spec_files():
        text = p.read_text(encoding="utf-8")
        raw, body = split_fm(text)
        meta = parse_meta(raw)
        docs.append({
            "path": p, "stem": p.stem, "raw": raw, "body": body,
            "meta": meta, "id": meta.get("id", ""), "deps": parse_deps(meta),
        })

    if not docs:
        print("规范目录为空：", sp.SPEC_DIR)
        return 1

    by_id = {d["id"]: d["stem"] for d in docs if d["id"]}
    downstream = {d["id"]: [] for d in docs}
    for d in docs:
        for dep in d["deps"]:
            if dep in downstream:
                downstream[dep].append(d["id"])

    for d in docs:
        body = strip_banner(d["body"])
        body = strip_related(body)
        block = related_block(d["deps"], downstream.get(d["id"], []), by_id)
        new = "---\n" + d["raw"].strip("\n") + "\n---\n\n" + insert_block(body, block)
        d["path"].write_text(new, encoding="utf-8")
        print(f"  sync {d['path'].name}")

    # ---------------- 附录：只统一「索引」链接（不重建上下游）
    for p in sp.appendix_files():
        text = p.read_text(encoding="utf-8")
        _, body = split_fm(text)
        if "## 关联" in body:
            new_body = re.sub(r"- 索引：\[\[[^\]]+\]\]",
                              f"- 索引：[[{OVERVIEW_STEM}|总览]]", body)
        else:
            new_body = body.rstrip("\n") + f"\n\n## 关联\n\n- 索引：[[{OVERVIEW_STEM}|总览]]\n"
        if new_body != body:
            p.write_text("---\n" + text[4:text.find("\n---\n", 4)] + "\n---\n" + new_body,
                         encoding="utf-8")
            print(f"  sync {p.name}（索引链接）")

    # ---------------- 总览
    total_rules = 0
    rows = []
    for d in docs:
        n = len(RULE.findall(d["path"].read_text(encoding="utf-8")))
        total_rules += n
        ups = "、".join(f"[[{by_id[x]}]]" for x in d["deps"] if x in by_id) or "—"
        rows.append((d, n, ups))

    tags = sorted({DOMAIN_TAG.get(d["id"].split("-")[1], "其他") for d in docs if d["id"]})
    lines = [
        "---", "type: 索引", "tags: [规范, 总览]", "---", "",
        "# 研发规范体系 · 总览", "",
        f"{len(docs)} 份规范（8 主册 + 3 专题），{total_rules} 条规则，覆盖需求 → 方案与设计 → 编码 → 测试 → 发布与运营。",
        "人和智能体共用同一份：**「必须」由机器或评审强制，「应」允许带豁免单偏离**。",
        "",
        "## 规范清单", "",
        "| 文档 | ID | 规则数 | 上游 |", "| --- | --- | --- | --- |",
    ]
    for d, n, ups in rows:
        lines.append(f"| [[{d['stem']}]] | `{d['id']}` | {n or '—'} | {ups} |")

    lines += ["", "## 依赖关系", "", "```mermaid", "graph TD"]
    short = {d["id"]: d["id"].split("-")[1] + re.sub(r"[^0-9]", "", d["id"]) for d in docs}
    for d in docs:
        for dep in d["deps"]:
            if dep in by_id:
                lines.append(f"  {short[dep]}[{by_id[dep]}] --> {short[d['id']]}[{d['stem']}]")
    lines += ["```", ""]

    # 交叉引用
    mentions = {}
    for a in docs:
        for b in docs:
            if a["id"] != b["id"]:
                n = a["body"].count(b["id"])
                if n:
                    mentions[(a["id"], b["id"])] = n
    if mentions:
        lines += ["## 交叉引用（正文提到其他规范 ID 的次数）", "", "| 来自 | 提到 | 次数 |", "| --- | --- | --- |"]
        for (a, b), n in sorted(mentions.items(), key=lambda kv: -kv[1])[:12]:
            lines.append(f"| [[{by_id[a]}]] | [[{by_id[b]}]] | {n} |")
        lines.append("")

    lines += [
        "## 阅读路径", "",
        f"- **新人**：读 [[{by_id.get('STD-GOV-00', '000-总则与治理规范')}]] 的第 2–8 章，再读所在环节那一册。",
        f"- **智能体**：按任务装载最小集，清单见 [[{by_id.get('STD-AGT-60', '600-智能体作业规范')}]] §4.1。",
        f"- **只想落地**：先接机检，见 [[{by_id.get('STD-TPL-70', '700-模板与机检')}]] §6。",
        "",
        "## 维护（库即源）", "",
        "本库是唯一事实源。改完笔记后跑：", "",
        "```bash",
        "python tools/sync_vault.py     # 重建各篇「关联」与本总览",
        "python tools/verify_vault.py   # 自检：属性、规则、链接、依赖一致性",
        "python tools/xtask.py spec-check",
        "```", "",
        "> 正文里的章节引用保留文档编号（如「700 §5」），与规则编号（`REQ-3.10`）一致；文件名前的数字只是排序用。",
        "",
    ]
    if sp.APPENDIX_DIR.exists() and list(sp.APPENDIX_DIR.glob("*.md")):
        lines += ["## 附录与背景", ""]
        for p in sorted(sp.APPENDIX_DIR.glob("*.md")):
            lines.append(f"- [[{p.stem}]]")
        lines.append("")

    sp.OVERVIEW.write_text("\n".join(lines), encoding="utf-8")
    print(f"  sync {sp.OVERVIEW.name}（{len(docs)} 篇，{total_rules} 条规则）")
    print(f"PASS  sync-vault  tags={','.join(tags)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
