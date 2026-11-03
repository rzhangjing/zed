#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""规范机检入口（stdlib only）。

与《70-模板与机检》中的命令一一对应：

    python tools/xtask.py spec-check             # 规范自检（编号/判定/引用/表格/链接/schema 同步）
    python tools/xtask.py sync-schemas           # 从 70 章抽取 schema 到 tools/schemas/
    python tools/xtask.py req-lint FILE...       # 需求卡校验（REQ-3.10 ~ REQ-3.22）
    python tools/xtask.py testcase-lint FILE...  # 用例校验（TST-4.4 ~ TST-4.7）
    python tools/xtask.py release-lint FILE...   # 发布单校验（REL-3.1 ~ REL-3.3）
    python tools/xtask.py trace --req-dir D --test-dir T   # 覆盖与孤儿（TST-4.5、TST-7.1）
    python tools/xtask.py commit-lint --message "..."       # 提交信息（COD-5.2）

退出码：0 通过，1 失败。失败信息以 [规则编号] 开头，指向具体条文。
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

import spec_paths

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

ROOT = Path(__file__).resolve().parent.parent
SCHEMA_DIR = spec_paths.SCHEMA_DIR
FENCE = re.compile(r"^```")
RULE_RE = re.compile(r"^\*\*([A-Z]{3}-\d+\.\d+)（(必须|应)）\*\*")
REF_RE = re.compile(r"\b([A-Z]{3}-\d+\.\d+)(?![.\d])")
SEC_RE = re.compile(r"(?<![\w])(\d{2})\s*§\s*(\d+)")
CHAPTER_RE = re.compile(r"^## (\d+) ")
FM_KEYS = ["id", "title", "version", "status", "layer", "audience", "enforcement", "owner", "depends_on"]
VERBS = ["支持", "新增", "增加", "导出", "导入", "生成", "修改", "删除", "移除", "修复", "调整",
         "拆分", "合并", "展示", "同步", "校验", "限制", "重命名", "替换", "接入", "迁移", "记录"]
BAD_WORDS = ["友好", "快速", "合理", "正常", "流畅", "稳定", "尽量", "适当", "良好"]
UNIT_RE = re.compile(r"\d+\s*(毫秒|秒|分钟|小时|%|％)")
CALIBER_KEYS = ["P50", "P95", "P99", "分位", "样本", "环境", "万行", "并发", "用户"]


class Result:
    def __init__(self, name):
        self.name = name
        self.errors = []
        self.warns = []

    def err(self, rule, msg):
        self.errors.append(f"[{rule}] {msg}")

    def warn(self, rule, msg):
        self.warns.append(f"[{rule}] {msg}")

    def report(self):
        for w in self.warns:
            print(f"  warn {w}")
        for e in self.errors:
            print(f"  FAIL {e}")
        status = "PASS" if not self.errors else "FAIL"
        print(f"{status}  {self.name}  (errors={len(self.errors)}, warns={len(self.warns)})")
        return 0 if not self.errors else 1


# --------------------------------------------------------------------- 迷你 schema 校验
TYPE_MAP = {
    "object": dict, "array": list, "string": str, "boolean": bool, "integer": int, "number": (int, float),
}


def _type_ok(value, expected):
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if expected == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if expected == "boolean":
        return isinstance(value, bool)
    return isinstance(value, TYPE_MAP[expected])


def validate(value, schema, path="$"):
    """返回错误列表。支持本项目实际用到的 schema 子集。"""
    errs = []
    if not isinstance(schema, dict):
        return errs

    if "type" in schema and not _type_ok(value, schema["type"]):
        return [f"{path}: 期望 {schema['type']}，实际 {type(value).__name__}"]
    if "enum" in schema and value not in schema["enum"]:
        errs.append(f"{path}: 取值 {value!r} 不在 {schema['enum']}")
    if "const" in schema and value != schema["const"]:
        errs.append(f"{path}: 期望常量 {schema['const']!r}")
    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]:
            errs.append(f"{path}: 长度 {len(value)} < {schema['minLength']}")
        if "maxLength" in schema and len(value) > schema["maxLength"]:
            errs.append(f"{path}: 长度 {len(value)} > {schema['maxLength']}")
        if "pattern" in schema and not re.search(schema["pattern"], value):
            errs.append(f"{path}: 不匹配 {schema['pattern']}")
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]:
            errs.append(f"{path}: {value} < {schema['minimum']}")
        if "maximum" in schema and value > schema["maximum"]:
            errs.append(f"{path}: {value} > {schema['maximum']}")
    if isinstance(value, list):
        if "minItems" in schema and len(value) < schema["minItems"]:
            errs.append(f"{path}: 元素 {len(value)} < {schema['minItems']}")
        if "items" in schema:
            for i, item in enumerate(value):
                errs.extend(validate(item, schema["items"], f"{path}[{i}]"))
    if isinstance(value, dict):
        for key in schema.get("required", []):
            if key not in value:
                errs.append(f"{path}: 缺少必填字段 {key}")
        props = schema.get("properties", {})
        for key, sub in props.items():
            if key in value:
                errs.extend(validate(value[key], sub, f"{path}.{key}"))
        if schema.get("additionalProperties") is False:
            for key in value:
                if key not in props:
                    errs.append(f"{path}: 未定义字段 {key}")
    if "allOf" in schema:
        for i, sub in enumerate(schema["allOf"]):
            if "if" in sub:
                if not validate(value, sub["if"], path):
                    errs.extend(validate(value, sub.get("then", {}), path))
            else:
                errs.extend(validate(value, sub, path))
    return errs


# --------------------------------------------------------------------- schema 抽取
def extract_schemas(text):
    out = {}
    for m in re.finditer(r"### 3\.\d+ [^\n]*`schemas/([a-z]+)\.schema\.json`", text):
        name = m.group(1)
        tail = text[m.end():]
        block = re.search(r"```json\n(.*?)```", tail, re.S)
        if not block:
            raise SystemExit(f"70 章 {name} schema 代码块缺失")
        out[name] = json.loads(block.group(1))
    return out


def load_schemas():
    docs = sorted(spec_paths.SPEC_DIR.glob("*模板与机检.md"))
    if not docs:
        raise SystemExit(f"找不到「模板与机检」规范文件：{spec_paths.SPEC_DIR}")
    text = docs[0].read_text(encoding="utf-8")
    return extract_schemas(text)


# --------------------------------------------------------------------- spec-check
def split_front_matter(text):
    if not text.startswith("---\n"):
        return None, text
    end = text.find("\n---\n", 4)
    if end == -1:
        return None, text
    raw = text[4:end]
    meta = {}
    for line in raw.splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            meta[k.strip()] = v.strip()
    return meta, text[end + 5:]


def scan_md(path):
    text = path.read_text(encoding="utf-8")
    meta, body = split_front_matter(text)
    lines = body.splitlines()
    refs, rules, chapters, tables = [], [], [], []
    in_fence = False
    prev_is_table = False
    expect_sep = False
    for i, line in enumerate(lines, 1):
        if FENCE.match(line):
            in_fence = not in_fence
            prev_is_table = False
            continue
        if in_fence:
            continue
        m = RULE_RE.match(line)
        if m:
            rules.append((i, m.group(1), m.group(2)))
        if line.startswith("## "):
            cm = CHAPTER_RE.match(line)
            if cm:
                chapters.append(int(cm.group(1)))
        if line.startswith("|"):
            if not prev_is_table:
                expect_sep = True
            elif expect_sep:
                if not re.match(r"^\|[\s:\-|]+\|$", line.strip()):
                    tables.append(i)
                expect_sep = False
            prev_is_table = True
        else:
            prev_is_table = False
            expect_sep = False
        for r in REF_RE.findall(line):
            refs.append((i, r))
    return meta, lines, rules, refs, chapters, tables


def cmd_spec_check(args):
    res = Result("spec-check")
    spec_paths.check_layout()
    schemas_md = load_schemas()
    spec_paths_list = spec_paths.spec_files()
    parsed = {p.name: scan_md(p) for p in spec_paths_list}

    # 第一遍：收集全部规则编号，供跨文件引用检查
    defined = set()
    for _, _, rules, _, _, _ in parsed.values():
        defined.update(rid for _, rid, _ in rules)

    # 文件号（000/100/…/700）与文档 ID 两种章节引用都要能解析
    chapters_by_prefix = {n.split("-")[0]: set(parsed[n][4]) for n in parsed}
    chapters_by_id = {
        parsed[n][0]["id"]: set(parsed[n][4])
        for n in parsed
        if parsed[n][0] and "id" in parsed[n][0]
    }

    for name, (meta, lines, rules, refs, chapters, tables) in parsed.items():
        path = spec_paths.SPEC_DIR / name
        if meta is None:
            res.err("GOV §6", f"{name} 缺少 YAML 头")
        else:
            for key in FM_KEYS:
                if key not in meta:
                    res.err("GOV §6", f"{name} 头部缺字段 {key}")

        seen = {}
        for lineno, rid, level in rules:
            if rid in seen:
                res.err("GOV §11", f"{name}:{lineno} 规则编号重复 {rid}（首次在第 {seen[rid]} 行）")
            seen[rid] = lineno
            if level not in ("必须", "应"):
                res.err("GOV §6", f"{name}:{lineno} {rid} 级别非法")
            has_verdict = False
            for nxt in lines[lineno:]:
                if RULE_RE.match(nxt) or nxt.startswith("## "):
                    break
                if nxt.startswith("判定："):
                    has_verdict = True
                    break
            if not has_verdict:
                res.err("GOV §6", f"{name}:{lineno} {rid} 缺少判定")

        for lineno, rid in refs:
            if rid not in defined:
                res.err("GOV §11", f"{name}:{lineno} 引用不存在的规则 {rid}")

        # 章节引用："700 §5"（三位文件号）与 "STD-REQ-10 §4"（文档 ID）
        in_fence = False
        for lineno, line in enumerate(lines, 1):
            if FENCE.match(line):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for pre, num in re.findall(r"(?<![\w])(\d{3})\s*§\s*(\d+)", line):
                if pre in chapters_by_prefix and int(num) not in chapters_by_prefix[pre]:
                    res.err("GOV §11", f"{name}:{lineno} 引用 {pre} §{num}，该章不存在")
            for did, num in re.findall(r"(STD-[A-Z]{3}-\d{2})\s*§\s*(\d+)", line):
                if did in chapters_by_id and int(num) not in chapters_by_id[did]:
                    res.err("GOV §11", f"{name}:{lineno} 引用 {did} §{num}，该章不存在")

        if chapters:
            start = chapters[0]
            if start not in (0, 1):
                res.err("GOV §6", f"{name} 章节起始为 {start}")
            for a, b in zip(chapters, chapters[1:]):
                if b != a + 1:
                    res.err("GOV §6", f"{name} 章节不连续：{a} → {b}")
        for i in tables:
            res.err("GOV §6", f"{name}:{i} 表格缺少分隔行，整表无法渲染")

        if body_fences_odd(path):
            res.err("GOV §6", f"{name} 代码围栏未闭合")

        for target in re.findall(r"\]\(([^)]+\.md)\)", path.read_text(encoding="utf-8")):
            if not (path.parent / target).exists():
                res.err("GOV §11", f"{name} 链接指向不存在的文件 {target}")

    for name, schema in schemas_md.items():
        f = SCHEMA_DIR / f"{name}.schema.json"
        if not f.exists():
            res.err("TPL-1.3", f"schema 文件缺失：{f.name}")
        elif json.loads(f.read_text(encoding="utf-8")) != schema:
            res.err("TPL-1.3", f"{f.name} 与 700 章不一致（先跑 sync-schemas）")

    # 总览与附录：只查表格、围栏、相对链接（无规则）
    loose = [spec_paths.OVERVIEW] + spec_paths.appendix_files()
    for path in loose:
        if not path.exists():
            res.err("GOV §11", f"缺少 {path}")
            continue
        _, _, _, _, _, tables = scan_md(path)
        for i in tables:
            res.err("GOV §6", f"{path.name}:{i} 表格缺少分隔行")
        if body_fences_odd(path):
            res.err("GOV §6", f"{path.name} 代码围栏未闭合")
        for target in re.findall(r"\]\(([^)]+\.md)\)", path.read_text(encoding="utf-8")):
            if not (path.parent / target).exists():
                res.err("GOV §11", f"{path.name} 链接指向不存在的文件 {target}")

    print(f"  库：{spec_paths.VAULT}")
    print(f"  规则数：{len(defined)}；正文：{len(parsed)}；索引/附录：{len(loose)}")
    return res.report()


def body_fences_odd(path):
    n = sum(1 for line in path.read_text(encoding="utf-8").splitlines() if FENCE.match(line))
    return n % 2 == 1


def cmd_sync_schemas(args):
    schemas = load_schemas()
    SCHEMA_DIR.mkdir(parents=True, exist_ok=True)
    for name, schema in schemas.items():
        f = SCHEMA_DIR / f"{name}.schema.json"
        f.write_text(json.dumps(schema, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"  wrote {f.relative_to(ROOT)}")
    print(f"PASS  sync-schemas  ({len(schemas)} 个)")
    return 0


def expand(pattern):
    paths = sorted(ROOT.glob(pattern))
    if not paths:
        p = Path(pattern)
        paths = [p] if p.exists() else []
    return paths


# --------------------------------------------------------------------- req-lint
def cmd_req_lint(args):
    res = Result("req-lint")
    schemas = load_schemas()
    schema = schemas["requirement"]
    for pattern in args.files:
        paths = expand(pattern)
        if not paths:
            res.err("GOV §3", f"无匹配文件：{pattern}")
            continue
        for path in paths:
            data = json.loads(path.read_text(encoding="utf-8"))
            tag = path.name
            for e in validate(data, schema):
                res.err("REQ-3.10", f"{tag} {e}")
            res.errors.extend(check_req_rules(data, tag))
    return res.report()


def check_req_rules(d, tag):
    errs = []
    title = d.get("title", "")
    if len(title) > 30:
        errs.append(f"[REQ-3.11] {tag} 标题 {len(title)} 字，超过 30")
    if not any(v in title for v in VERBS):
        errs.append(f"[REQ-3.11] {tag} 标题未检出动词：{title}")
    for w in BAD_WORDS:
        if w in title:
            errs.append(f"[REQ-3.17] {tag} 标题含禁用词：{w}")

    acs = d.get("acceptance", [])
    if not any(a.get("kind") == "exception" for a in acs):
        errs.append(f"[REQ-3.19] {tag} 无异常类验收标准")
    for a in acs:
        text = " ".join(str(a.get(k, "")) for k in ("given", "when", "then"))
        for w in BAD_WORDS:
            if w in text:
                errs.append(f"[REQ-3.17] {tag} {a.get('id')} 含禁用词 {w}")
        if UNIT_RE.search(text) and not any(k in text for k in CALIBER_KEYS):
            errs.append(f"[REQ-3.18] {tag} {a.get('id')} 含时间/比例数值但缺口径（分位数/样本量/环境）")

    if not d.get("out_of_scope"):
        errs.append(f"[REQ-3.12] {tag} 范围外为空")
    elif any(s.strip() in ("无", "暂无", "") for s in d["out_of_scope"]):
        errs.append(f"[REQ-3.12] {tag} 范围外写了“无”，等于没写边界")

    pr = d.get("priority", {})
    if len(pr.get("rationale", "")) < 10:
        errs.append(f"[REQ-3.22] {tag} 优先级依据不足 10 字")
    est = d.get("estimate", {})
    if est.get("pessimistic", 0) > 5:
        errs.append(f"[REQ-3.7] {tag} 估时 {est.get('pessimistic')} 人日 > 5，必须拆分")
    return errs


# --------------------------------------------------------------------- testcase / release
def cmd_testcase_lint(args):
    res = Result("testcase-lint")
    schema = load_schemas()["testcase"]
    for pattern in args.files:
        paths = expand(pattern)
        if not paths:
            res.err("GOV §3", f"无匹配文件：{pattern}")
            continue
        for path in paths:
            data = json.loads(path.read_text(encoding="utf-8"))
            for e in validate(data, schema):
                res.err("TST-4.4", f"{path.name} {e}")
            if data.get("priority") == "P0" and data.get("automation") != "auto":
                res.err("TST-4.7", f"{path.name} P0 用例未自动化")
            if not any(str(x).startswith(("AC-", "REQ-")) or "." in str(x) for x in data.get("links", [])):
                res.err("TST-4.5", f"{path.name} 未关联需求或验收")
    return res.report()


def cmd_release_lint(args):
    res = Result("release-lint")
    schema = load_schemas()["release"]
    for pattern in args.files:
        paths = expand(pattern)
        if not paths:
            res.err("GOV §3", f"无匹配文件：{pattern}")
            continue
        for path in paths:
            data = json.loads(path.read_text(encoding="utf-8"))
            for e in validate(data, schema):
                res.err("REL-3.1", f"{path.name} {e}")
            if data.get("kind") == "migration" and not data.get("rollback", {}).get("script"):
                res.err("REL-6.3", f"{path.name} 数据迁移缺少回滚脚本")
    return res.report()


# --------------------------------------------------------------------- artifact-lint
def cmd_artifact_lint(args):
    res = Result("artifact-lint")
    schemas = load_schemas()
    if args.schema not in schemas:
        res.err("TPL-1.3", f"未知 schema：{args.schema}，可选 {sorted(schemas)}")
        return res.report()
    schema = schemas[args.schema]
    for pattern in args.files:
        paths = expand(pattern)
        if not paths:
            res.err("GOV §3", f"无匹配文件：{pattern}")
            continue
        for path in paths:
            data = json.loads(path.read_text(encoding="utf-8"))
            for e in validate(data, schema):
                res.err("TPL-1.3", f"{path.name} {e}")
    return res.report()


# --------------------------------------------------------------------- trace
def cmd_trace(args):
    res = Result("trace")
    reqs = {}
    for f in sorted((ROOT / args.req_dir).glob("*.json")):
        d = json.loads(f.read_text(encoding="utf-8"))
        reqs[d["id"]] = {a["id"]: a for a in d.get("acceptance", [])}
    tests = [json.loads(f.read_text(encoding="utf-8")) for f in sorted((ROOT / args.test_dir).glob("*.json"))]

    covered = {rid: set() for rid in reqs}
    for t in tests:
        for link in t.get("links", []):
            if link in reqs:
                covered[link].update(reqs[link].keys())
            elif re.match(r"^AC-\d+$", str(link)):
                for rid, acs in reqs.items():
                    if link in acs:
                        covered[rid].add(link)
            else:
                res.err("TST-4.5", f"{t['id']} 指向不存在的对象 {link}")
    gaps = [f"{rid}/{ac}" for rid, acs in reqs.items() for ac in acs if ac not in covered.get(rid, set())]
    for g in gaps:
        res.err("TST-4.5", f"验收标准无用例覆盖：{g}")
    print(f"  需求 {len(reqs)} 条，验收 {sum(len(v) for v in reqs.values())} 条，用例 {len(tests)} 条，缺口 {len(gaps)}")
    return res.report()


# --------------------------------------------------------------------- commit-lint
COMMIT_RE = re.compile(r"^(feat|fix|refactor|test|docs|chore|perf|build|ci)(\([a-z0-9_/-]+\))?: .{1,72}$")


def cmd_commit_lint(args):
    res = Result("commit-lint")
    text = args.message or (Path(args.file).read_text(encoding="utf-8") if args.file else "")
    if not text.strip():
        res.err("COD-5.2", "提交信息为空")
        return res.report()
    first = text.splitlines()[0]
    if not COMMIT_RE.match(first):
        res.err("COD-5.2", f"首行不合格式（type(scope): subject，≤72 字）：{first}")
    if not re.search(r"Refs:\s*(REQ|BUG|TSK)-\d+", text):
        res.err("COD-5.2", "缺少 Refs: REQ-xxxx 关联")
    return res.report()


# --------------------------------------------------------------------- errcodes
CODE_RE = re.compile(r"\b([A-Z]{3})-0([1-5])-(\d{3})\b")


def cmd_errcodes(args):
    res = Result("errcodes")
    data = json.loads(Path(args.table).read_text(encoding="utf-8"))
    seen_code, seen_name = {}, {}
    for c in data.get("codes", []):
        code, name = c.get("code", ""), c.get("name", "")
        if not re.fullmatch(r"[A-Z]{3}-0[1-5]-\d{3}", code):
            res.err("DES-4.10", f"错误码格式非法：{code}（应为 ABC-01-001 ~ ABC-05-999）")
        if code in seen_code:
            res.err("DES-4.10", f"错误码重复：{code}")
        seen_code[code] = True
        if name in seen_name:
            res.err("DES-4.10", f"同一错误名对应两个码：{name}（{seen_name[name]} / {code}）")
        seen_name[name] = code
        if c.get("class") and code[4:6] != c["class"]:
            res.err("DES-4.10", f"{code} 的类与 class 字段不一致：{c['class']}")
    if args.scan:
        known = set(seen_code)
        for f in Path(args.scan).rglob("*.rs"):
            for m in CODE_RE.finditer(f.read_text(encoding="utf-8", errors="replace")):
                code = m.group(0)
                if code not in known:
                    res.err("DES-4.10", f"{f.name}: {code} 未在错误码表登记")
    print(f"  错误码 {len(seen_code)} 条")
    return res.report()


# --------------------------------------------------------------------- dep-graph
def find_cycle(edges):
    graph = {}
    for a, b in edges:
        graph.setdefault(a, set()).add(b)
    WHITE, GRAY, BLACK = 0, 1, 2
    color = {n: WHITE for n in graph}
    stack = []

    def dfs(node):
        color[node] = GRAY
        stack.append(node)
        for nxt in graph.get(node, ()):
            if color.get(nxt, WHITE) == GRAY:
                return stack[stack.index(nxt):] + [nxt]
            if color.get(nxt, WHITE) == WHITE:
                found = dfs(nxt)
                if found:
                    return found
        stack.pop()
        color[node] = BLACK
        return None

    for n in list(graph):
        if color[n] == WHITE:
            found = dfs(n)
            if found:
                return found
    return None


def cmd_dep_graph(args):
    res = Result("dep-graph")
    layers = json.loads(Path(args.layers).read_text(encoding="utf-8"))["layers"]
    edges, modules = set(), set()
    for f in Path(args.src).rglob("*.rs"):
        rel = f.relative_to(args.src)
        mod = rel.parts[0] if len(rel.parts) > 1 else rel.stem
        modules.add(mod)
        text = f.read_text(encoding="utf-8", errors="replace")
        for target in re.findall(r"crate::([a-z_][a-z0-9_]*)", text):
            if target != mod:
                edges.add((mod, target))
    for m in sorted(modules | {t for _, t in edges}):
        if m not in layers:
            res.err("ARC-3.9", f"模块 {m} 未在分层配置中声明")
    for a, b in sorted(edges):
        if a in layers and b in layers and layers[b] < layers[a]:
            res.err("COD-3.1", f"依赖方向错误：{a}(第 {layers[a]} 层) → {b}(第 {layers[b]} 层)")
    cyc = find_cycle(edges)
    if cyc:
        res.err("COD-3.1", f"依赖环：{' → '.join(cyc)}")
    print(f"  模块 {len(modules)} 个，依赖 {len(edges)} 条")
    return res.report()


# --------------------------------------------------------------------- contract
def cmd_contract_test(args):
    res = Result("contract-test")
    spec = json.loads(Path(args.spec).read_text(encoding="utf-8"))
    for pattern in args.examples:
        for path in expand(pattern):
            ex = json.loads(path.read_text(encoding="utf-8"))
            p, m, s = ex.get("path"), ex.get("method", "").lower(), str(ex.get("status", 200))
            node = spec.get("paths", {}).get(p, {}).get(m, {}).get("responses", {}).get(s)
            if node is None:
                res.err("ARC-3.8", f"{path.name}: 契约中无 {m.upper()} {p} {s}")
                continue
            schema = node.get("content", {}).get("application/json", {}).get("schema")
            if schema is None:
                continue
            for e in validate(ex.get("body"), schema):
                res.err("DES-4.2", f"{path.name} {e}")
    return res.report()


# --------------------------------------------------------------------- eval
def cmd_eval_gate(args):
    res = Result("eval-gate")
    for pattern in args.files:
        paths = expand(pattern)
        if not paths:
            res.err("GOV §3", f"无匹配文件：{pattern}")
            continue
        for path in paths:
            d = json.loads(path.read_text(encoding="utf-8"))
            rate = d.get("pass_rate", 0)
            if d.get("samples", 0) < 60:
                res.err("AGT-7.1", f"{path.name}: 任务集 {d.get('samples')} 条 < 60")
            if rate < 0.85:
                res.err("AGT-7.3", f"{path.name}: 通过率 {rate:.0%} < 85%")
            if rate < d.get("baseline_pass_rate", 0):
                res.err("AGT-7.3", f"{path.name}: 低于上一版基线 {d.get('baseline_pass_rate'):.0%}")
            if d.get("severe_errors", 1) != 0:
                res.err("AGT-7.3", f"{path.name}: 严重错误 {d.get('severe_errors')} 条，必须为 0")
    return res.report()


# --------------------------------------------------------------------- agent-registry
AGENT_ID_RE = re.compile(r"^AGT-[A-Z]{2,4}$")
LEVELS = {"none", "read", "local", "global", "irreversible"}
AGENT_FIELDS = ["id", "name", "owner", "version", "inputs", "tools", "outputs",
                "forbidden", "refusal", "escalation", "budget"]


def cmd_agent_registry(args):
    res = Result("agent-registry")
    data = json.loads(Path(args.registry).read_text(encoding="utf-8"))
    tools = data.get("tools", {})
    seen = set()
    for a in data.get("agents", []):
        aid = a.get("id", "")
        if not AGENT_ID_RE.match(aid):
            res.err("AGT-2.1", f"agent id 非法：{aid}")
        if aid in seen:
            res.err("AGT-2.1", f"agent id 重复：{aid}")
        seen.add(aid)
        for k in AGENT_FIELDS:
            if k not in a or a[k] in ("", [], {}):
                res.err("AGT-2.1", f"{aid} 缺字段 {k}")
        if a.get("owner") in (None, ""):
            res.err("AGT-2.2", f"{aid} 未绑定责任人类 owner")
        for t in a.get("tools", []):
            if t not in tools:
                res.err("AGT-3.1", f"{aid} 使用了未登记工具 {t}")
                continue
            lv = tools[t].get("level")
            if lv not in LEVELS:
                res.err("AGT-3.1", f"工具 {t} 的副作用级别非法：{lv}")
        b = a.get("budget", {})
        for k in ("minutes", "tool_calls", "tokens"):
            if not isinstance(b.get(k), int) or b.get(k) <= 0:
                res.err("AGT-10.1", f"{aid} 预算字段 {k} 缺失或非正数")
    print(f"  智能体 {len(seen)} 个，工具 {len(tools)} 个")
    return res.report()


# --------------------------------------------------------------------- coverage
def parse_lcov(path):
    data, current = {}, None
    for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("SF:"):
            current = line[3:].strip().replace("\\", "/")
            data.setdefault(current, {})
        elif line.startswith("DA:") and current is not None:
            line_no, _, count = line[3:].partition(",")
            try:
                data[current][int(line_no)] = int(count)
            except ValueError:
                pass
    return data


def parse_diff(path):
    """返回 {文件: 新增行号集合}，支持 git diff 的统一格式。"""
    added, cur = {}, None
    for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("+++ "):
            target = line[4:].strip()
            target = target[2:] if target.startswith("b/") else target
            cur = target
            added.setdefault(cur, set())
        elif line.startswith("@@") and cur is not None:
            m = re.search(r"\+(\d+)(?:,(\d+))?", line)
            if m:
                start, count = int(m.group(1)), int(m.group(2) or 1)
                added[cur].update(range(start, start + count))
    return added


def cmd_coverage(args):
    res = Result("coverage")
    lcov = parse_lcov(args.lcov)
    diff = parse_diff(args.diff)
    total = covered = 0
    for f, lines in diff.items():
        norm = f.replace("\\", "/")
        if norm not in lcov:
            continue
        for ln in lines:
            if ln in lcov[norm]:
                total += 1
                if lcov[norm][ln] > 0:
                    covered += 1
    if total == 0:
        res.warn("TST-7.2", "本次改动没有可统计的执行行（可能只改了注释或文档）")
    else:
        pct = covered * 100 / total
        print(f"  改动行覆盖：{covered}/{total} = {pct:.1f}%（阈值 {args.fail_under}%）")
        if pct < args.fail_under:
            res.err("TST-7.2", f"改动行覆盖 {pct:.1f}% < {args.fail_under}%")
    return res.report()


# --------------------------------------------------------------------- main
def main(argv=None):
    p = argparse.ArgumentParser(prog="xtask")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("spec-check")
    sub.add_parser("sync-schemas")
    for name in ("req-lint", "testcase-lint", "release-lint"):
        sp = sub.add_parser(name)
        sp.add_argument("files", nargs="+")
    sp = sub.add_parser("artifact-lint")
    sp.add_argument("--schema", required=True, choices=["requirement", "review", "handoff", "proposal", "testcase", "release"])
    sp.add_argument("files", nargs="+")
    sp = sub.add_parser("trace")
    sp.add_argument("--req-dir", default="docs/req")
    sp.add_argument("--test-dir", default="docs/test")
    sp = sub.add_parser("commit-lint")
    sp.add_argument("--message")
    sp.add_argument("--file")
    sp = sub.add_parser("errcodes")
    sp.add_argument("--table", default="docs/errors/errcodes.json")
    sp.add_argument("--scan")
    sp = sub.add_parser("dep-graph")
    sp.add_argument("--src", default="tests/fixtures/src-good")
    sp.add_argument("--layers", default="docs/arch/layers.json")
    sp = sub.add_parser("contract-test")
    sp.add_argument("--spec", default="docs/contract/openapi.json")
    sp.add_argument("--examples", nargs="+", default=["docs/contract/examples/*.json"])
    sp = sub.add_parser("eval-gate")
    sp.add_argument("files", nargs="+")
    sp = sub.add_parser("agent-registry")
    sp.add_argument("--registry", default="docs/agents/registry.json")
    sp = sub.add_parser("coverage")
    sp.add_argument("--lcov", default="tests/fixtures/lcov.info")
    sp.add_argument("--diff", default="tests/fixtures/change.patch")
    sp.add_argument("--fail-under", type=float, default=80.0)
    args = p.parse_args(argv)

    table = {
        "spec-check": cmd_spec_check,
        "sync-schemas": cmd_sync_schemas,
        "req-lint": cmd_req_lint,
        "testcase-lint": cmd_testcase_lint,
        "release-lint": cmd_release_lint,
        "artifact-lint": cmd_artifact_lint,
        "trace": cmd_trace,
        "commit-lint": cmd_commit_lint,
        "errcodes": cmd_errcodes,
        "dep-graph": cmd_dep_graph,
        "contract-test": cmd_contract_test,
        "eval-gate": cmd_eval_gate,
        "agent-registry": cmd_agent_registry,
        "coverage": cmd_coverage,
    }
    return table[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
