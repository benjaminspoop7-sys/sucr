#!/usr/bin/env node
/* Builds the Sucr app binary without the Tauri CLI: `tauri build` cannot spawn
   the `cargo` on PATH here (it is a POSIX shim), so this locates the real
   cargo.exe and passes the same environment the shim would have set. */
import { spawnSync } from 'node:child_process';
import { existsSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';

const isWin = process.platform === 'win32';
const exe = isWin ? '.exe' : '';

function firstDir(candidates) {
  for (const dir of candidates) if (dir && existsSync(dir)) return dir;
  return null;
}

const cargoHome = process.env['CARGO_HOME'] ?? firstDir([
  'H:\\tools\\cargo',
  join(homedir(), '.cargo'),
]);
if (!cargoHome) {
  console.error('build-app: no CARGO_HOME (looked for H:\\tools\\cargo and ~/.cargo)');
  process.exit(1);
}

const rustupHome = process.env['RUSTUP_HOME'] ?? firstDir([
  'H:\\tools\\rustup',
  join(homedir(), '.rustup'),
]);

const cargo = join(cargoHome, 'bin', `cargo${exe}`);
if (!existsSync(cargo)) {
  console.error(`build-app: missing ${cargo}`);
  process.exit(1);
}

const debug = process.argv.includes('--debug');
const args = [
  'build',
  ...(debug ? [] : ['--release']),
  '--manifest-path',
  'src-tauri/Cargo.toml',
  ...process.argv.slice(2).filter((a) => a !== '--debug'),
];

const env = { ...process.env, CARGO_HOME: cargoHome };
if (rustupHome) env['RUSTUP_HOME'] = rustupHome;

console.log(`build-app: ${cargo} ${args.join(' ')}`);
const result = spawnSync(cargo, args, { stdio: 'inherit', env, cwd: process.cwd() });
if (result.error) {
  console.error(`build-app: ${result.error.message}`);
  process.exit(1);
}
if (result.status !== 0) process.exit(result.status ?? 1);

const out = join('src-tauri', 'target', debug ? 'debug' : 'release', `sucr${exe}`);
if (existsSync(out)) {
  const kb = statSync(out).size / 1024;
  console.log(`build-app: ${out} · ${kb.toFixed(0)} KB (${(kb / 1024).toFixed(2)} MB)`);
}
