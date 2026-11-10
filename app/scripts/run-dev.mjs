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
const child = spawn(bin, extra, { stdio: 'inherit' });
child.on('exit', (code, signal) => {
  if (signal) console.error(`run-dev: exited on ${signal}`);
  process.exit(code ?? 0);
});
