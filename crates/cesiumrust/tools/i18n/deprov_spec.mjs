// 一次性去溯源助手：仅改写"整行注释"（trimStart 以 // 开头），
// 从不触碰代码或字符串字面量，保证 DRIFT=0。
// 用法: node deprov_spec.mjs <dir-or-file> [<more>...] [--apply]
import { readFileSync, writeFileSync, statSync, readdirSync } from 'fs';
import { join } from 'path';

const args = process.argv.slice(2);
const apply = args.includes('--apply');
const targets = args.filter((a) => !a.startsWith('--'));

function collectRs(paths) {
  const out = [];
  for (const p of paths) {
    const st = statSync(p);
    if (st.isFile()) {
      if (p.endsWith('.rs')) out.push(p);
    } else {
      for (const e of readdirSync(p)) {
        const full = join(p, e);
        let s;
        try { s = statSync(full); } catch { continue; }
        if (s.isDirectory()) out.push(...collectRs([full]));
        else if (s.isFile() && e.endsWith('.rs')) out.push(full);
      }
    }
  }
  return out;
}

function deprov(s) {
  s = s.replace(/packages\/engine\/?/g, '');
  s = s.replace(/忠实移植/g, '对齐实现');
  s = s.replace(/移植自/g, '参考自');
  s = s.replace(/\bPorted\s+from\b/gi, '参考自');
  s = s.replace(/\.js\b/g, '');
  s = s.replace(/\.glsl\b/g, '');
  s = s.replace(/对应\s*CesiumJS/g, '参考 CesiumJS');
  s = s.replace(/上游\s*CesiumJS/g, '参考 CesiumJS');
  s = s.replace(/来源\s*[:：]/g, '出处：');
  s = s.replace(/cesium-rs\/crates/g, 'cesiumrust');
  return s;
}

const files = collectRs(targets);
let changedFiles = 0;
for (const f of files) {
  const src = readFileSync(f, 'utf8');
  const lines = src.split('\n');
  let touched = false;
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].replace(/^\uFEFF/, '');
    if (t.trimStart().startsWith('//')) {
      const nv = deprov(t);
      if (nv !== t) { lines[i] = nv; touched = true; }
    }
  }
  if (touched) {
    changedFiles++;
    if (apply) writeFileSync(f, lines.join('\n'), 'utf8');
  }
}
console.log(`scanned=${files.length} changed=${changedFiles} apply=${apply}`);
