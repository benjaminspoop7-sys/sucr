#!/usr/bin/env node
/* Local cycle: build the debug binary (fast, with symbols) and launch it.
   `--skip-build` launches whatever release exe already exists. */
import { spawn, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';

const isWin = process.platform === 'win32';
const exe = isWin ? '.exe' : '';
const skipBuild = process.argv.includes('--skip-build');

if (!skipBuild) {
  const build = spawnSync(process.execPath, ['scripts/build-app.mjs', '--debug'], {
    stdio: 'inherit',
    env: process.env,
  });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

const candidates = skipBuild
  ? [join('src-tauri', 'target', 'release', `sucr${exe}`)]
  : [join('src-tauri', 'target', 'debug', `sucr${exe}`)];

const bin = candidates.find((p) => existsSync(p));
if (!bin) {
  console.error(`run-dev: no binary (looked for ${candidates.join(', ')})`);
  process.exit(1);
}

const extra = process.argv.slice(2).filter((a) => a !== '--skip-build');

// The debug binary is its own app already — bundle identifier `app.sucr.dev`,
// WebView2 profile under %LOCALAPPDATA%\app.sucr.dev, and its own data root
// (`%APPDATA%\Sucr Dev`), which the browser-spawned relay resolves by itself.
// Passing a --data-dir here would split the two. The release binary has none of
// that, so `npm run start` still names a folder of its own.
if (skipBuild) {
  const hasDataDir =
    extra.some((a) => a.startsWith('--data-dir=')) ||
    (process.env['SUCR_DATA_DIR'] ?? '') !== '';
  if (!hasDataDir) {
    const dataDir = join(process.cwd(), '.devdata');
    extra.push(`--data-dir=${dataDir}`);
    console.log(`run-dev: data ${dataDir}`);
  }
  console.log('run-dev: release binary — data is isolated, WebView2 profile is the shipped one; use `npm run dev` for a fully separate app');
} else {
  console.log('run-dev: debug build — data %APPDATA%\\Sucr Dev; browser session: npm run browser:dev');
}

const child = spawn(bin, extra, { stdio: 'inherit' });
child.on('exit', (code, signal) => {
  if (signal) console.error(`run-dev: exited on ${signal}`);
  process.exit(code ?? 0);
});
