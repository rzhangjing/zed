// audit.mjs — 生产代码注释审计：溯源命中 + 缺注释条目 + 到目标率缺口
// 用法: node tools/i18n/audit.mjs [--target 25] [--crate <name>] <dir...>
// dir 相对 git 根，如 cesiumrust/domain。--crate 只输出该 crate 汇总。
import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { lex } from './i18n.mjs';

const CJK = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\u3000-\u303f\uff00-\uffef]/;
const REPO_ROOT = execSync('git rev-parse --show-toplevel', { encoding: 'utf8' }).trim();

let argv = process.argv.slice(2);
let TARGET = 25;
const ti = argv.indexOf('--target');
if (ti !== -1) { TARGET = parseFloat(argv[ti + 1]); argv.splice(ti, 2); }
const dirs = argv.filter((a) => !a.startsWith('--'));
const SRC_ONLY = argv.includes('--src');

// 溯源记录模式（CesiumJS / JS 源码 / cesium-rs 移植指针 / JS 测试名镜像）
const PROV = [
  /移植自/i, /忠实移植/i, /ported\s+from/i, /derived\s+from/i, /based\s+on\s+cesium/i,
  /packages\/engine/i, /\.js\b/, /\.glsl\b/, /对应\s*CesiumJS/i, /上游\s*CesiumJS/i,
  /cesium-rs\/crates/i, /来源[:：]/i,
];
function provHits(line) {
  const body = line.replace(/^\s*\/\/+\s?|^\/\*+|^\*+\/?|\*+\/$|^\*+\s?/g, '');
  if (body.trim() === '') return false;
  return PROV.some((re) => re.test(body));
}

function walk(dir) {
  return execSync(`git -c core.quotepath=false ls-files "${dir}"`, { encoding: 'utf8', cwd: REPO_ROOT })
    .split('\n').filter((s) => s.endsWith('.rs'));
}

// 判断某物理行是否有"上方紧邻的 /// 或 // 文档注释"（跳过 #[...] 属性与空行）
function hasDocAbove(lines, idx) {
  for (let j = idx - 1; j >= 0; j--) {
    const t = lines[j].trim();
    if (t === '') continue;
    if (t.startsWith('#[') || t.startsWith(']')) continue; // 属性/续行
    if (t.startsWith('/*') || t.startsWith('//!') || t.startsWith('//')) return true;
    return false;
  }
  return false;
}

// 向上跳过空行，检查紧邻的属性行是否包含指定属性名（如 test/bench）
function hasAttrNamed(lines, idx, names) {
  for (let j = idx - 1; j >= 0; j--) {
    const t = lines[j].trim();
    if (t === '') continue;
    if (!t.startsWith('#')) return false;
    if (names.some((n) => new RegExp('#\\[.*\\b' + n + '\\b').test(t))) return true;
    return false;
  }
  return false;
}

function auditFile(rel) {
  const src = readFileSync(`${REPO_ROOT}/${rel}`, 'utf8').replace(/\r\n/g, '\n');
  const lines = src.split('\n');
  const { comments } = lex(src);
  const commentLine = new Set();
  for (const cm of comments) {
    const nl = cm.text.split('\n').length;
    for (let k = 0; k < nl; k++) commentLine.add(cm.line + k);
  }
  let code = 0, zh = 0, en = 0, prov = 0;
  const provLines = [];
  for (const cm of comments) {
    const arr = cm.text.split('\n');
    for (let k = 0; k < arr.length; k++) {
      const ln = arr[k];
      const body = ln.replace(/^\s*\/+\s?|^\/+|^\*+\/?|\*+\/$|^\*+\s?/g, '');
      if (body.trim() === '') continue;
      if (CJK.test(body)) zh++; else if (/[A-Za-z]{3,}/.test(body)) en++;
      if (provHits(ln)) { prov++; provLines.push({ line: cm.line + k, text: body.trim().slice(0, 80) }); }
    }
  }
  for (let i = 0; i < lines.length; i++) {
    if (commentLine.has(i + 1)) continue;
    if (lines[i].trim() === '') continue;
    code++;
  }
  // 缺注释条目：fn 定义 与 struct/enum 字段
  const uncommented = [];
  let braceDepth = 0; const structStack = [];
  // 跳过整个 #[cfg(test)] 模块（测试与辅助函数不需 /// 文档）
  let inTest = false, testBottom = -1, pendingTest = false;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i]; const t = raw.trim();
    // 跳过注释行
    if (t.startsWith('//') || t.startsWith('/*') || t.startsWith('*')) { braceDepth += (raw.match(/\{/g) || []).length - (raw.match(/\}/g) || []).length; continue; }
    if (inTest) {
      braceDepth += (raw.match(/\{/g) || []).length - (raw.match(/\}/g) || []).length;
      if (braceDepth < testBottom) inTest = false;
      continue;
    }
    const isFn = /^(pub(\([^)]*\))?\s+)?(async\s+)?(unsafe\s+)?fn\s/.test(t);
    // 测试/基准函数不需 /// 文档：函数名后跟 #[test]/#[bench] 属性，或符合 test_* 命名约定
    const fnName = (t.match(/\bfn\s+([A-Za-z_]\w*)/) || [])[1] || '';
    const isTestFn = /^(test_.+|.+_test)$/.test(fnName) || hasAttrNamed(lines, i, ['test', 'bench', 'ignore']);
    const opensStruct = /(struct|union)\s+\w+[^({;]*\{/.test(t) || /(struct|union)\s+\w+.*\{$/.test(t);
    const opensEnum = /enum\s+\w+[^{]*\{/.test(t) || /enum\s+\w+.*\{$/.test(t);
    const isFieldLike = /^(pub(\([^)]*\)\s*)?)?[a-z_]\w*\s*:\s*.+[,;]?\s*(\/\/.*)?$/.test(t) && !t.startsWith('let ') && !t.startsWith('return');
    if (isFn && !isTestFn && !hasDocAbove(lines, i)) uncommented.push({ line: i + 1, kind: 'fn', text: t.slice(0, 60) });
    if ((structStack.length > 0) && isFieldLike && !hasDocAbove(lines, i)) uncommented.push({ line: i + 1, kind: 'field', text: t.slice(0, 60) });
    braceDepth += (raw.match(/\{/g) || []).length - (raw.match(/\}/g) || []).length;
    if (opensStruct || opensEnum) structStack.push(braceDepth);
    while (structStack.length && braceDepth < structStack[structStack.length - 1]) structStack.pop();
    // 测试模块边界检测
    const isCfgTest = /^#\[cfg\(\s*test\s*\)\]/.test(t);
    if (isCfgTest) {
      pendingTest = true;
      if (/\bmod\s+\w+/.test(t)) { inTest = true; testBottom = braceDepth; pendingTest = false; }
    } else if (pendingTest && /^(pub\s+)?mod\s+\w+/.test(t)) {
      inTest = true; testBottom = braceDepth; pendingTest = false;
    } else if (t !== '' && !t.startsWith('#')) {
      pendingTest = false;
    }
  }
  const needZh = Math.max(0, Math.ceil((TARGET / 100) * code - zh));
  return { rel, code, zh, en, prov, provLines, rate: (zh / code * 100), uncommented, needZh };
}

let files = [...new Set(dirs.flatMap(walk))];
if (SRC_ONLY) files = files.filter((f) => /\/src\//.test('/' + f));
let agg = { code: 0, zh: 0, prov: 0, uncommented: 0, needZh: 0 };
const rows = [];
for (const rel of files) {
  const a = auditFile(rel);
  agg.code += a.code; agg.zh += a.zh; agg.prov += a.prov; agg.uncommented += a.uncommented.length;
  rows.push(a);
}
// 只有当该文件缺条目或溯源>0 才列出（Top 需补率的文件）
rows.sort((x, y) => y.needZh - x.needZh);
if (process.argv.includes('--detail')) {
  for (const r of rows) for (const u of r.uncommented) console.log(`  UNCM ${r.rel}:${u.line} [${u.kind}] ${u.text}`);
  for (const r of rows) for (const p of r.provLines) console.log(`  PROV ${r.rel}:${p.line} ${p.text}`);
}
console.log(`TARGET=${TARGET}%  files=${files.length} code=${agg.code} zh=${agg.zh} rate=${(agg.zh / agg.code * 100).toFixed(1)}%  provenanceHits=${agg.prov} uncommentedItems=${agg.uncommented}`);
console.log(`总需补中文注释行(整仓拉到${TARGET}%) ≈ ${rows.reduce((s, r) => s + r.needZh, 0)}`);
console.log('--- 需补率最多的文件 (top 30) ---');
for (const r of rows.slice(0, 30)) {
  console.log(`  needZh=${String(r.needZh).padStart(4)} rate=${r.rate.toFixed(0).padStart(3)}% prov=${String(r.prov).padStart(3)} uncmt=${String(r.uncommented.length).padStart(3)} code=${String(r.code).padStart(5)}  ${r.rel}`);
}
