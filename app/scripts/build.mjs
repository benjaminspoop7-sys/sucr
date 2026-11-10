// Builds the renderer into dist/renderer and stages the static files.
// tokens.css is copied verbatim from ../design — that file is the design
// source of truth and is never edited here. The hash printed below is the
// contract that the copy is byte-identical.
import { build, transformSync } from 'esbuild';
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readFileSync, writeFileSync, readdirSync, rmSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const dist = path.join(root, 'dist', 'renderer');

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });

checkHistoryKindsHaveUiCases();

execFileSync(
  process.execPath,
  [path.join(root, 'node_modules', 'typescript', 'bin', 'tsc'), '-p', 'tsconfig.renderer.json'],
  { cwd: root, stdio: 'inherit' },
);

await build({
  entryPoints: [path.join(root, 'src', 'renderer', 'main.ts')],
  bundle: true,
  format: 'esm',
  target: 'chrome120',
  minify: true,
  sourcemap: process.env['SUCR_SOURCEMAP'] === '1', // the map is embedded in the exe otherwise
  outfile: path.join(dist, 'main.js'),
  absWorkingDir: root,
});

copyFileSync(path.join(root, 'src', 'renderer', 'index.html'), path.join(dist, 'index.html'));
const cssRaw = readFileSync(path.join(root, 'src', 'renderer', 'app.css'), 'utf8');
const cssMin = transformSync(cssRaw, { loader: 'css', minify: true }).code;
writeFileSync(path.join(dist, 'app.css'), cssMin);

const tokensSrc = path.join(root, '..', 'design', 'tokens.css');
const tokensBytes = readFileSync(tokensSrc);
copyFileSync(tokensSrc, path.join(dist, 'tokens.css'));
const tokensHash = createHash('sha256').update(tokensBytes).digest('hex').slice(0, 12);
console.log(`tokens.css: design/tokens.css -> dist/renderer/tokens.css (sha256 ${tokensHash})`);
console.log('build:web ok');

// --- history-kind contract -------------------------------------------------
// Every kind the engine writes to history_events has to have a `case` in the
// renderer's formatEvent(). Without one it falls through to the default
// branch, which shows the raw payload JSON in the Activity trail. The two
// halves live in different languages and nothing else cross-checks them, so
// the check runs before the compiler does.
function checkHistoryKindsHaveUiCases() {
  const kinds = new Set();
  const unmappable = [];
  for (const file of rustSources(path.join(root, 'src-tauri', 'src'))) {
    const text = readFileSync(file, 'utf8');
    for (const open of historyCallSites(text)) {
      const args = callArgs(text, open);
      const kind = args ? bareLiteral(args[2]) : null;
      if (kind) kinds.add(kind);
      else unmappable.push(`  ${rel(file)}: history() kind is not a plain "lowercase" literal`);
    }
    for (const m of text.matchAll(/INSERT INTO history_events[^;]{0,220}?VALUES[^;]{0,220}?'([a-z0-9-]+)'/g)) {
      kinds.add(m[1]);
    }
  }
  if (kinds.size === 0) fail(['no engine history kind found — did the writer move?']);

  const body = formatEventBody(readFileSync(path.join(root, 'src', 'renderer', 'main.ts'), 'utf8'));
  if (body === null) {
    fail(['formatEvent() not found in src/renderer/main.ts — the guard would be vacuous']);
  }
  if (!body.includes('default:')) {
    fail(['formatEvent() has no default branch — the guard cannot tell a mapped kind from an unmapped one']);
  }
  const cases = new Set();
  for (const m of body.matchAll(/case '([a-z0-9-]+)':/g)) cases.add(m[1]);

  const missing = [...kinds].filter((k) => !cases.has(k)).sort();
  if (missing.length > 0) {
    fail([
      ...missing.map((k) => `  kind "${k}" has no case in formatEvent(); the trail would print its raw JSON`),
      '',
      'add the case in src/renderer/main.ts, or stop writing the kind.',
    ]);
  }
  const orphan = [...cases].filter((c) => !kinds.has(c)).sort();
  const report = [`history kinds: ${kinds.size} written by the engine, all formatted by the UI`];
  if (unmappable.length > 0) report.push(...unmappable, '(unmappable: they are not counted, and the trail would dump them)');
  if (orphan.length > 0) report.push(`note: UI cases with no writer: ${orphan.join(', ')}`);
  console.log(report.join('\n'));
}

function fail(lines) {
  console.error(`\nHISTORY KIND GATE FAILED\n${lines.join('\n')}\n`);
  process.exit(1);
}

function rel(file) {
  return path.relative(root, file).replace(/\\/g, '/');
}

function rustSources(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) rustSources(file, out);
    else if (entry.name.endsWith('.rs') && entry.name !== 'tests.rs') out.push(file);
  }
  return out;
}

// offsets of every '(' that opens a history(...) call, skipping the definition
// itself and anything reached through a dot.
function historyCallSites(text) {
  const hits = [];
  for (let i = text.indexOf('history('); i !== -1; i = text.indexOf('history(', i + 8)) {
    // Only the text before the call on its own line can make it somebody's
    // method: a comment or a string on the line above ends in '.' all the time.
    const before = text.slice(text.lastIndexOf('\n', i - 1) + 1, i);
    if (/\bfn\s+$/.test(before) || /[\w.)]\s*$/.test(before)) continue;
    hits.push(i + 'history'.length);
  }
  return hits;
}

// the argument list of a call whose '(' sits at `open`
function callArgs(text, open) {
  const args = [];
  let depth = 0;
  let current = '';
  let quote = null;
  for (let i = open + 1; i < text.length; i++) {
    const c = text[i];
    if (quote !== null) {
      current += c;
      if (c === quote && text[i - 1] !== '\\') quote = null;
      continue;
    }
    if (c === '"' || c === "'") quote = c;
    else if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') {
      if (depth === 0) {
        args.push(current);
        return args;
      }
      depth--;
    } else if (c === ',' && depth === 0) {
      args.push(current);
      current = '';
      continue;
    }
    current += c;
  }
  return null;
}

function bareLiteral(arg) {
  const m = /^\s*"([a-z0-9-]+)"\s*$/.exec(arg ?? '');
  return m ? m[1] : null;
}

// formatEvent's own body, so `case` labels from the status switches cannot be
// mistaken for history kinds. Top-level functions in this file close at column
// zero, so the next `\n}` is the end.
function formatEventBody(text) {
  const start = text.indexOf('function formatEvent(');
  if (start === -1) return null;
  const end = text.indexOf('\n}', start);
  return end === -1 ? null : text.slice(start, end);
}
