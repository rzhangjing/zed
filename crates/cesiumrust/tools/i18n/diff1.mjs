// diff one file: baseline snapshot fp vs current fp, show first divergence
import { readFileSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const SNAP = join(HERE, '.i18n_snap.json');
const REPO_ROOT = execSync('git rev-parse --show-toplevel', { encoding: 'utf8' }).trim();

const { lex } = await import('./i18n.mjs');
const rel = process.argv[2];
const snap = JSON.parse(readFileSync(SNAP, 'utf8'));
const key = Object.keys(snap).find((k) => k.endsWith(rel.replace(/\\/g, '/')));
const base = key ? snap[key] : (() => { const h = execSync(`git show "HEAD:${rel}"`, { encoding: 'utf8', cwd: REPO_ROOT, maxBuffer: 64*1024*1024 }); return lex(h).fp; })();
const cur = lex(readFileSync(join(REPO_ROOT, rel), 'utf8')).fp;

console.log('base len', base.length, 'cur len', cur.length);
let i = 0;
while (i < Math.min(base.length, cur.length) && base[i] === cur[i]) i++;
console.log('first divergence at', i);
console.log('BASE <<<', JSON.stringify(base.slice(Math.max(0, i - 60), i + 80)), '>>>');
console.log('CUR  <<<', JSON.stringify(cur.slice(Math.max(0, i - 60), i + 80)), '>>>');
