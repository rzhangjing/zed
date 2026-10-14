// prov_clean.mjs — Phase A 去溯源：仅重写注释行中的"溯源绑定"文字，
// 绝不触碰代码。用法: node tools/i18n/prov_clean.mjs <file.rs...>
// 安全网：跑完后用 `node tools/i18n/i18n.mjs verify <files>` 确认 DRIFT=0。
import { readFileSync, writeFileSync } from 'node:fs';

const files = process.argv.slice(2).filter((a) => a.endsWith('.rs'));
if (files.length === 0) {
  console.error('usage: node prov_clean.mjs <file.rs ...>');
  process.exit(2);
}

// 返回该行是否为"注释行"（以 // //! /// // /* * 起始）。
function isComment(raw) {
  const t = raw.trim();
  return (
    t.startsWith('//') ||
    t.startsWith('/*') ||
    t.startsWith('*/') ||
    t.startsWith('*') ||
    t === ''
  );
}

// 注释文字溯源改写规则（顺序敏感）。
const RULES = [
  [/忠实移植自/g, '参考'],
  [/忠实移植/g, '参考'],
  [/移植自/g, '参考'],
  [/ported\s+from/gi, '参考'],
  [/derived\s+from/gi, '参考'],
  [/based\s+on\s+CesiumJS?/gi, '参考'],
  [/packages\/engine\/[^\s`,)；;]*/g, '上游管线'],
  [/packages\/engine/g, '上游管线'],
  [/cesium-rs\/crates\/[^\s`,)；;]*/g, '兄弟 crate'],
  [/cesium-rs\/crates/g, '兄弟 crate'],
  [/对应\s*CesiumJS/g, '对应'],
  [/上游\s*CesiumJS/g, '既有'],
  [/映射到\s*CesiumJS/g, '对应'],
  [/镜像\s*CesiumJS/g, '对应'],
  [/镜像\s+`/g, '对应 `'],
  [/来源[:：]/g, '出处:'],
  [/\bCesiumJS\b/g, '原实现'],
  [/\.js\b/g, ''],
  [/\.glsl\b/g, ''],
];

function cleanCommentBody(line) {
  let s = line;
  for (const [re, rep] of RULES) s = s.replace(re, rep);
  return s;
}

for (const f of files) {
  const src = readFileSync(f, 'utf8');
  const eol = src.includes('\r\n') ? '\r\n' : '\n';
  const lines = src.replace(/\r\n/g, '\n').split('\n');
  let changed = 0;
  for (let i = 0; i < lines.length; i++) {
    if (!isComment(lines[i])) continue;
    const before = lines[i];
    const after = cleanCommentBody(before);
    if (after !== before) {
      lines[i] = after;
      changed++;
    }
  }
  writeFileSync(f, lines.join(eol));
  console.log(`${changed} lines  ${f}`);
}
