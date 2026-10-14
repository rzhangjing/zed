// i18n.mjs — 注释英译中安全工具（词法级 Rust 注释解析）
//
// 子命令：
//   strip  <file>            打印该文件"去注释后的代码指纹"（诊断用）
//   scan   <dir...>          统计各文件英文注释行 vs 中文注释行（未跟踪，仅本地）
//   list   <dir...>          只列出仍有英文注释的文件（按英文行数降序）
//   extract <file>           列出文件中每条注释块（行号 + 原文），供翻译参照
//   verify <file...>         对比 git HEAD 版本与工作区版本的代码指纹，判定"仅注释改变"
//
// 设计：单遍状态机，正确处理 行注释/嵌套块注释/字符串/原始字符串/字节串/字符字面量，
//   使指纹仅反映"代码 token + 字符串原文"，注释的增删改与空白变化不影响指纹。

import { readFileSync, writeFileSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join, relative } from 'node:path';

const NL = new Set([' ', '\t', '\n', '\r', '\f', '\v']);
const CJK = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\u3000-\u303f\uff00-\uffef]/;

/** 词法扫描：返回 { fp, comments: [{text, line}] } */
export function lex(src) {
  // 规范化行尾：工作区磁盘常为 CRLF，而 git HEAD blob 为 LF；
  // 仅多行原始字符串（r#"..."#）内的换行会被逐字保留，否则会造成
  // 纯行尾差异的假阳性 CODE-DRIFT。代码空白会被折叠，故归一无副作用。
  src = src.replace(/\r\n/g, '\n');
  let fp = '';
  const comments = [];
  let line = 1;
  let i = 0;
  const n = src.length;

  const addCode = (s) => {
    for (let k = 0; k < s.length; k++) {
      const c = s[k];
      if (NL.has(c)) {
        if (fp.length && fp[fp.length - 1] !== ' ') fp += ' ';
      } else fp += c;
    }
  };
  const addRaw = (s) => { fp += s; };

  // 尝试在 i 处解析字符字面量；成功返回完整字符串（含两端单引号），否则 null
  function tryChar(s, idx) {
    // s[idx] === "'"
    let j = idx + 1;
    if (j >= s.length) return null;
    if (s[j] === '\\') {
      j++;
      if (j >= s.length) return null;
      const e = s[j];
      if (e === 'x') { j += 3; }            // \xHH
      else if (e === 'u') {                 // \u{...}
        if (s[j + 1] !== '{') return null;
        const close = s.indexOf('}', j);
        if (close === -1) return null;
        j = close;
      } else { j += 1; }                     // \n \' \\ \0 ...
      if (s[j] === "'") return s.slice(idx, j + 1);
      return null;
    }
    // 单字符 'a'
    if (j + 1 < s.length && s[j + 1] === "'") return s.slice(idx, j + 2);
    return null; // 生命周期标签等，非字符
  }

  while (i < n) {
    const c = src[i];
    const d = src[i + 1];

    if (c === '\n') { line++; addCode('\n'); i++; continue; }

    // 行注释
    if (c === '/' && d === '/') {
      let j = i + 2;
      while (j < n && src[j] !== '\n') j++;
      comments.push({ text: src.slice(i, j), line });
      addCode(' '); i = j; continue;
    }
    // 块注释（可嵌套）
    if (c === '/' && d === '*') {
      let depth = 1, j = i + 2, start = line;
      while (j < n && depth > 0) {
        if (src[j] === '\n') line++;
        if (src[j] === '/' && src[j + 1] === '*') { depth++; j += 2; continue; }
        if (src[j] === '*' && src[j + 1] === '/') { depth--; j += 2; continue; }
        j++;
      }
      comments.push({ text: src.slice(i, j), line: start });
      addCode(' '); i = j; continue;
    }
    // 原始字符串 r"..." / r#"..."# 及字节原始串 br"..." / b"..."
    {
      let p = i, isB = false, isR = false;
      if (src[p] === 'b') { isB = true; p++; }
      if (src[p] === 'r') { isR = true; p++; }
      if (isR) {
        let hashes = 0;
        while (src[p] === '#') { hashes++; p++; }
        if (src[p] === '"') {
          const open = '#'.repeat(hashes) + '"';
          const close = '"' + '#'.repeat(hashes);
          const bodyStart = p + 1;
          const end = src.indexOf(close, bodyStart);
          const stop = end === -1 ? n : end + close.length;
          addRaw(src.slice(i, stop));
          for (let k = i; k < stop; k++) if (src[k] === '\n') line++;
          i = stop; continue;
        }
      } else if (isB && src[p] === '"') {
        // 字节串 b"..."（含转义）
        let j = p + 1;
        while (j < n) { if (src[j] === '\\') { j += 2; continue; } if (src[j] === '"') { j++; break; } if (src[j] === '\n') line++; j++; }
        addRaw(src.slice(i, j)); i = j; continue;
      }
    }
    // 普通字符串 "..."
    if (c === '"') {
      let j = i + 1;
      while (j < n) {
        if (src[j] === '\\') { j += 2; continue; }
        if (src[j] === '"') { j++; break; }
        if (src[j] === '\n') line++;
        j++;
      }
      addRaw(src.slice(i, j)); i = j; continue;
    }
    // 字符字面量（先于生命周期判断）
    if (c === "'") {
      const ch = tryChar(src, i);
      if (ch) { addRaw(ch); i += ch.length; continue; }
      addCode("'"); i++; continue;
    }
    addCode(c); i++;
  }

  return { fp: fp.trim(), comments };
}

function fingerprint(src) { return lex(src).fp; }

const arg = process.argv[2];
const rest = process.argv.slice(3);

function walk(dir, out = []) {
  // 使用 git ls-files 更可靠；对目录用 shell find
  const files = execSync(`git -c core.quotepath=false ls-files "${dir.replace(/\\/g, '/')}"`, { encoding: 'utf8', cwd: REPO_ROOT })
    .split('\n').filter((s) => s.endsWith('.rs'));
  for (const f of files) out.push(f);
  return out;
}
const REPO_ROOT = execSync('git rev-parse --show-toplevel', { encoding: 'utf8' }).trim();
const HERE = dirname(fileURLToPath(import.meta.url));
const SNAP = join(HERE, '.i18n_snap.json');
function loadSnap() { try { return JSON.parse(readFileSync(SNAP, 'utf8')); } catch { return {}; } }
function saveSnap(o) { writeFileSync(SNAP, JSON.stringify(o)); }

function readWork(rel) { return readFileSync(join(REPO_ROOT, rel), 'utf8'); }
function readHead(rel) {
  try { return execSync(`git show "HEAD:${rel}"`, { encoding: 'utf8', cwd: REPO_ROOT, maxBuffer: 64 * 1024 * 1024 }); }
  catch { return null; }
}

function classifyComments(src) {
  const { comments } = lex(src);
  let en = 0, zh = 0;
  for (const cm of comments) {
    // 按物理行拆分统计：注释块可能多行
    const lines = cm.text.split('\n');
    for (const ln of lines) {
      const body = ln.replace(/^\s*\/\/+\s?|^\/\*+|^\*+\/?|\*+\/$|^\*+\s?/g, '');
      if (body.trim() === '') continue;
      if (CJK.test(body)) zh++; else if (/[A-Za-z]{3,}/.test(body)) en++;
    }
  }
  return { en, zh };
}

if (arg === 'strip') {
  for (const f of rest) { console.log(fingerprint(readFileSync(f, 'utf8'))); }
} else if (arg === 'snap') {
  const s = loadSnap();
  for (const f of rest) {
    const rel = relative(REPO_ROOT, join(process.cwd(), f)).replace(/\\/g, '/');
    s[rel] = fingerprint(readFileSync(join(REPO_ROOT, rel), 'utf8'));
  }
  saveSnap(s);
  console.log(`SNAP ok: ${rest.length} files (baseline stored in .i18n_snap.json)`);
} else if (arg === 'check') {
  const s = loadSnap();
  let bad = 0;
  for (const f of rest) {
    const rel = relative(REPO_ROOT, join(process.cwd(), f)).replace(/\\/g, '/');
    const base = s[rel] !== undefined ? s[rel] : (() => { const h = readHead(rel); return h === null ? null : fingerprint(h); })();
    if (base === null) { console.log(`NEW-FILE  ${rel} (无基线，跳过)`); continue; }
    const cur = fingerprint(readWork(rel));
    if (cur === base) console.log(`OK  ${rel}`); else { bad++; console.log(`!! CODE-DRIFT !!  ${rel}`); }
  }
  console.log(`\nDRIFT=${bad}  (0 = 仅注释改变，代码/字符串逐 token 一致)`);
  process.exit(bad ? 1 : 0);
} else if (arg === 'verify') {
  let bad = 0;
  for (const f of rest) {
    const rel = relative(REPO_ROOT, join(process.cwd(), f)).replace(/\\/g, '/');
    const head = readHead(rel);
    if (head === null) { console.log(`NEW-FILE  ${rel} (无 HEAD 基线，跳过)`); continue; }
    const a = fingerprint(head), b = fingerprint(readWork(rel));
    if (a === b) console.log(`OK-CODE-UNCHANGED  ${rel}`);
    else { bad++; console.log(`!! CODE-DRIFT !!  ${rel}`); }
  }
  console.log(`\nDRIFT=${bad}  (0 = 该批次仅注释改变，代码/字符串逐 token 一致)`);
  process.exit(bad ? 1 : 0);
} else if (arg === 'scan' || arg === 'list') {
  const all = [];
  for (const d of rest) for (const f of walk(d)) all.push(f);
  const uniq = [...new Set(all)];
  let totEn = 0, totZh = 0;
  const rows = [];
  for (const rel of uniq) {
    const { en, zh } = classifyComments(readWork(rel));
    totEn += en; totZh += zh;
    rows.push({ rel, en, zh });
  }
  rows.sort((a, b) => b.en - a.en);
  if (arg === 'list') {
    for (const r of rows) if (r.en > 0) console.log(`${String(r.en).padStart(5)} ${r.rel}`);
  } else {
    console.log(`files=${uniq.length} english_comment_lines=${totEn} chinese_comment_lines=${totZh}`);
    console.log('--- top 25 by english lines ---');
    for (const r of rows.slice(0, 25)) console.log(`  en=${String(r.en).padStart(4)} zh=${String(r.zh).padStart(4)}  ${r.rel}`);
  }
} else if (arg === 'extract') {
  for (const f of rest) {
    const src = readFileSync(f, 'utf8');
    const { comments } = lex(src);
    console.log(`### ${f}  (${comments.length} comment blocks)`);
    for (const cm of comments) console.log(`  L${cm.line}: ${cm.text.replace(/\n/g, '\\n').slice(0, 120)}`);
  }
} else {
  console.log('usage: node i18n.mjs strip|verify|scan|list|extract <args>');
}
