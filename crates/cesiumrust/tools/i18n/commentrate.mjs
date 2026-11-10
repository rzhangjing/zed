// commentrate.mjs — 度量中文注释率
// 定义：CJK 注释行 / 代码行(非空、非注释)  >= 目标
// 用法: node tools/i18n/commentrate.mjs <dir...>   (dir 相对 git 根，如 cesiumrust/domain)
import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { lex } from './i18n.mjs';

const CJK = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\u3000-\u303f\uff00-\uffef]/;
const REPO_ROOT = execSync('git rev-parse --show-toplevel', { encoding: 'utf8' }).trim();
const dirs = process.argv.slice(2);

function walk(dir) {
  return execSync(`git -c core.quotepath=false ls-files "${dir}"`, { encoding: 'utf8', cwd: REPO_ROOT })
    .split('\n').filter((s) => s.endsWith('.rs'));
}

// 从源码计算：代码行(去注释去空行的物理行)、中文注释行、英文注释行
function measure(src) {
  src = src.replace(/\r\n/g, '\n');
  const lines = src.split('\n');
  const { comments } = lex(src);
  // 标记被注释占据的物理行
  const commentLine = new Set();
  for (const cm of comments) {
    const start = cm.line; // 1-based
    const nl = cm.text.split('\n').length;
    for (let k = 0; k < nl; k++) commentLine.add(start + k);
  }
  let code = 0, zh = 0, en = 0;
  for (const cm of comments) {
    for (const ln of cm.text.split('\n')) {
      const body = ln.replace(/^\s*\/\/+\s?|^\/\*+|^\*+\/?|\*+\/$|^\*+\s?/g, '');
      if (body.trim() === '') continue;
      if (CJK.test(body)) zh++; else if (/[A-Za-z]{3,}/.test(body)) en++;
    }
  }
  for (let i = 0; i < lines.length; i++) {
    const ln = lines[i];
    if (commentLine.has(i + 1)) continue;
    if (ln.trim() === '') continue;
    code++;
  }
  return { code, zh, en };
}

let totCode = 0, totZh = 0, totEn = 0;
const byDir = {};
const files = [...new Set(dirs.flatMap(walk))];
for (const rel of files) {
  const m = measure(readFileSync(`${REPO_ROOT}/${rel}`, 'utf8'));
  totCode += m.code; totZh += m.zh; totEn += m.en;
  const top = rel.split('/').slice(0, 2).join('/');
  byDir[top] ??= { code: 0, zh: 0, en: 0, files: 0 };
  byDir[top].code += m.code; byDir[top].zh += m.zh; byDir[top].en += m.en; byDir[top].files++;
}
console.log(`files=${files.length} codeLines=${totCode} zhComment=${totZh} enComment=${totEn}`);
console.log(`中文注释率 = zh/code = ${(totZh / totCode * 100).toFixed(2)}%`);
console.log(`注释总占比 = (zh+en)/(code+zh+en) = ${((totZh + totEn) / (totCode + totZh + totEn) * 100).toFixed(2)}%`);
console.log('--- by top dir (zh/code %) ---');
for (const [k, v] of Object.entries(byDir)) {
  console.log(`  ${(v.zh / v.code * 100).toFixed(1)}%  code=${String(v.code).padStart(6)} zh=${String(v.zh).padStart(5)} en=${String(v.en).padStart(5)} files=${String(v.files).padStart(4)}  ${k}`);
}
