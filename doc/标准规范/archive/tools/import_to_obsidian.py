#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""把 标准规范/ 下编号 00~70 的规范导入 Obsidian 库，并生成关联。

用法：
    python tools/import_to_obsidian.py            # 导入 + 生成总览
    python tools/import_to_obsidian.py --dry-run   # 只看会写哪些文件

做的事：
1. 读 00~70 八份规范，保留 front matter，补 type/doc_id/tags
2. 正文里的 [文字](xx.md) 转成 [[xx|文字]]
3. 每篇末尾追加「关联」段：上游、下游、总览
4. 生成库根目录的 总览.md（MOC + 依赖图 + 交叉引用统计 + 机检速查）
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

SRC = Path(r"E:\Rust\zed\doc\标准规范")
VAULT = Path(r"E:\Rust\zed\doc\规范仓库")
DST = VAULT / "规范"

DOMAIN_TAG = {
    "GOV": "规范/治理",
    "REQ": "规范/需求",
    "ARC": "规范/设计",
    "DES": "规范/设计",
    "UI": "规范/UI",
    "COD": "规范/编码",
    "TST": "规范/测试",
    "REL": "规范/发布",
    "AGT": "规范/智能体",
    "TPL": "规范/模板",
}

DOC_ID_RE = re.compile(r"^id:\s*(STD-[A-Z]{3}-\d{2})", re.M)
LINK_RE = re.compile(r"\[([^\]\n]+)\]\(([^()\s]+?\.md)\)")
RULE_RE = re.compile(r"^\*\*([A-Z]{3}-\d+\.\d+)（(?:必须|应)）\*\*", re.M)


def split_front_matter(text: str):
    if not text.startswith("---\n"):
        return {}, text
    end = text.find("\n---\n", 4)
    if end == -1:
        return {}, text
    block = text[4:end]
    body = text[end + 5:]
    meta = {}
    for line in block.splitlines():
        if ":" in line and not line.startswith(" "):
            k, v = line.split(":", 1)
            meta[k.strip()] = v.strip()
    return meta, body


def parse_deps(raw: str) -> list[str]:
    raw = (raw or "").strip()
    if not raw.startswith("["):
        return []
    return [x.strip() for x in raw[1:-1].split(",") if x.strip()]


def to_wikilinks(text: str) -> str:
    def repl(m):
        label, target = m.group(1), m.group(2)
        stem = Path(target).stem
        return f"[[{stem}]]" if label == stem else f"[[{stem}|{label}]]"

    return LINK_RE.sub(repl, text)


def display_name(stem: str) -> str:
    return stem.split("-", 1)[1] if "-" in stem else stem


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)

    files = sorted(p for p in SRC.glob("*.md") if p.name[:2].isdigit())
    if not files:
        print("没有找到编号规范文件")
        return 1

    docs = []
    for p in files:
        text = p.read_text(encoding="utf-8")
        meta, body = split_front_matter(text)
        doc_id = meta.get("id", "")
        deps = parse_deps(meta.get("depends_on", ""))
        domain = doc_id.split("-")[1] if doc_id.count("-") >= 2 else "GOV"
        docs.append({
            "path": p,
            "stem": p.stem,
            "meta": meta,
            "body": body,
            "id": doc_id,
            "deps": deps,
            "domain": domain,
            "rules": len(RULE_RE.findall(body)),
            "chars": len(text),
        })

    by_id = {d["id"]: d for d in docs}
    downstream = {d["id"]: [] for d in docs}
    for d in docs:
        for dep in d["deps"]:
            if dep in downstream:
                downstream[dep].append(d)

    # 交叉引用计数：A 正文提到 B 的 id
    mentions = {}
    for a in docs:
        for b in docs:
            if a["id"] == b["id"]:
                continue
            n = a["body"].count(b["id"])
            if n:
                mentions[(a["id"], b["id"])] = n

    DST.mkdir(parents=True, exist_ok=True)
    written = []

    for d in docs:
        body = to_wikilinks(d["body"].rstrip())

        block = ["## 关联", ""]
        if d["deps"]:
            block.append("- 上游：" + "、".join(
                f"[[{by_id[dep]['stem']}|{display_name(by_id[dep]['stem'])}]]" if dep in by_id else dep
                for dep in d["deps"]))
        else:
            block.append("- 上游：无（本册是根）")
        if downstream[d["id"]]:
            block.append("- 下游：" + "、".join(
                f"[[{x['stem']}|{display_name(x['stem'])}]]" for x in downstream[d["id"]]))
        else:
            block.append("- 下游：无")
        block.append("- 索引：[[总览]]")
        block.append("")

        lines = body.splitlines()
        if "## 变更记录" in lines:
            i = lines.index("## 变更记录")
            lines[i:i] = block
        else:
            lines += [""] + block
        body = "\n".join(lines) + "\n"

        fm_extra = [
            "type: 规范",
            f"doc_id: {d['id']}",
            f"tags: [规范, {DOMAIN_TAG.get(d['domain'], '规范')}]",
        ]
        src_fm = text_front_matter_block(d["path"])
        new_fm = "---\n" + src_fm.rstrip("\n") + "\n" + "\n".join(fm_extra) + "\n---\n"

        note = (
            new_fm
            + f"\n> 导入自 `{d['path']}`，改内容请改源文件后重跑 `tools/import_to_obsidian.py`。\n"
            + body
        )
        out = DST / f"{d['stem']}.md"
        written.append(out)
        if not args.dry_run:
            out.write_text(note, encoding="utf-8")

    # ---------------- 附录（检查报告与规划清单）
    extras = [
        ("规范检查报告.md", ["检查报告"], None),
        ("研发规范体系清单.md", ["规划", "已取代"], "> 规划视图，已被 000–700 正式规范取代，仅作背景。\n"),
        ("研发工序执行规范清单.md", ["规划", "已取代"], "> 规划视图，已被 000–700 正式规范取代，仅作背景。\n"),
    ]
    for src_name, tags, banner in extras:
        p = SRC / src_name
        if not p.exists():
            continue
        meta, body = split_front_matter(p.read_text(encoding="utf-8"))
        fm = text_front_matter_block(p).rstrip("\n") if meta else "type: 附录\nstatus: archived"
        note = (
            "---\n" + fm + f"\ntags: [规范, {', '.join(tags)}]\n---\n\n"
            + (banner + "\n" if banner else "")
            + to_wikilinks(body.rstrip())
            + "\n\n## 关联\n\n- 索引：[[总览]]\n"
        )
        out = VAULT / "附录" / src_name
        out.parent.mkdir(parents=True, exist_ok=True)
        written.append(out)
        if not args.dry_run:
            out.write_text(note, encoding="utf-8")

    # ---------------- 总览
    lines = [
        "---",
        "type: 索引",
        "tags: [规范, 总览]",
        "---",
        "",
        "# 研发规范体系 · 总览",
        "",
        f"{len(docs)} 份规范（8 主册 + 3 专题），覆盖需求 → 方案与设计 → 编码 → 测试 → 发布与运营，外加智能体作业与机检。",
        "人和智能体共用同一份：**「必须」由机器或评审强制，「应」允许带豁免单偏离**。",
        "",
        "## 规范清单",
        "",
        "| 文档 | ID | 规则数 | 上游 |",
        "| --- | --- | --- | --- |",
    ]
    for d in docs:
        ups = "、".join(f"[[{by_id[x]['stem']}]]" for x in d["deps"] if x in by_id) or "—"
        lines.append(f"| [[{d['stem']}]] | `{d['id']}` | {d['rules'] or '—'} | {ups} |")

    lines += [
        "",
        "## 依赖关系",
        "",
        "```mermaid",
        "graph TD",
    ]
    short = {d["id"]: d["id"].split("-")[1] + re.sub(r"[^0-9]", "", d["id"]) for d in docs}
    for d in docs:
        for dep in d["deps"]:
            if dep in by_id:
                lines.append(f"  {short[dep]}[{by_id[dep]['stem']}] --> {short[d['id']]}[{d['stem']}]")
    lines += ["```", ""]

    top = sorted(mentions.items(), key=lambda kv: -kv[1])[:10]
    if top:
        lines += ["## 交叉引用（正文提到其他规范 ID 的次数）", "", "| 来自 | 提到 | 次数 |", "| --- | --- | --- |"]
        for (a, b), n in top:
            lines.append(f"| [[{by_id[a]['stem']}]] | [[{by_id[b]['stem']}]] | {n} |")
        lines.append("")

    lines += [
        "## 阅读路径",
        "",
        "- **新人**：读 [[000-总则与治理规范]] 的第 2–8 章（术语、ID、角色、规则、评审），再读你所在环节那一册。",
        "- **智能体**：按任务装载最小集，清单见 [[600-智能体作业规范]] §4.1，不要一次读完。",
        "- **只想落地**：先接机检，见 [[700-模板与机检]] §6。",
        "",
        "## 机检速查",
        "",
        "```bash",
        "python tools/xtask.py spec-check          # 规范自检：编号/判定/引用/表格/schema",
        'python tools/xtask.py req-lint "docs/req/*.json"',
        "python tools/xtask.py trace               # 验收 ↔ 用例覆盖",
        "python tools/run_selfcheck.py             # 门禁回归 24 例",
        "```",
        "",
        "## 维护",
        "",
        "本库是**导入件**，不是源。改规范请改 `E:\\Rust\\zed\\doc\\标准规范\\` 下的源文件，再重跑：",
        "",
        "```bash",
        "python tools/import_to_obsidian.py",
        "```",
        "",
        "## 附录与背景",
        "",
        "- [[规范检查报告]] —— 7 轮检查的发现、修正与已知边界，回答「为什么这么定」",
        "- [[研发规范体系清单]] —— 规范体系的分层清单（规划视图，已被取代）",
        "- [[研发工序执行规范清单]] —— 工序级执行清单（规划视图，已被取代）",
        "",
        "> 正文里的章节引用保留原始文档编号（如「70 §5」「40 §7」），与规则编号（`REQ-3.10`）一致；文件名前的 000–700 只是排序用。",
        "",
    ]
    overview = VAULT / "总览.md"
    if not args.dry_run:
        overview.write_text("\n".join(lines), encoding="utf-8")

    print(f"导入 {len(written)} 篇 → {DST}")
    for w in written:
        print(f"  {w.name}")
    print(f"总览 → {overview}")
    if args.dry_run:
        print("（dry-run，未写盘）")
    return 0


def text_front_matter_block(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        return ""
    end = text.find("\n---\n", 4)
    return text[4:end]


if __name__ == "__main__":
    sys.exit(main())
