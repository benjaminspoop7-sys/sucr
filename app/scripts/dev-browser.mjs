#!/usr/bin/env node
/* A browser session of its own for the dev build.

   The installed Sucr owns its browser profile, its add-on and the relay
   registration the installer wrote. Testing capture from a local build must not
   borrow any of that, so this session is built from parts that belong to the
   dev build alone: a throwaway profile folder, a dev-flavoured add-on (its own
   extension ID, its own relay name) and the dev build's own data folder.

   Nothing here touches the registry. Instead of a native-messaging
   registration, the dev add-on learns the dev relay's door from `relay.json`,
   which this writes beside the add-on and keeps current while the session runs.
   That keeps the whole dev capture path inside %APPDATA%\Sucr Dev: the normal
   browser has no host to find, the installed add-on keeps asking for
   `app.sucr.capture`, and the two add-ons cannot replace each other.

   Usage: npm run browser:dev [-- --browser=<name|path>] [-- <browser flags>]
          npm run browser:dev -- --remove    (clear a dev relay left by an older launcher) */
import { spawn, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const isWin = process.platform === 'win32';
const exe = isWin ? '.exe' : '';
const argv = process.argv.slice(2);
const split = argv.indexOf('--');
const own = split >= 0 ? argv.slice(0, split) : argv;
const passthrough = split >= 0 ? argv.slice(split + 1) : [];

const appDir = process.cwd();
const repoRoot = resolve(appDir, '..');
const self = fileURLToPath(import.meta.url);

function fail(message) {
  console.error(`dev-browser: ${message}`);
  process.exit(1);
}

/// Where the dev build keeps everything, mirroring `paths::default_dir`: a
/// debug build owns `Sucr Dev` under the platform data root, and the installed
/// app's `Sucr` folder is never named here.
function devDataDir() {
  if (isWin) return join(process.env['APPDATA'] || join(homedir(), 'AppData', 'Roaming'), 'Sucr Dev');
  if (process.platform === 'darwin') return join(homedir(), 'Library', 'Application Support', 'Sucr Dev');
  return join(process.env['XDG_DATA_HOME'] || join(homedir(), '.local', 'share'), 'Sucr Dev');
}

const bin = join(appDir, 'src-tauri', 'target', 'debug', `sucr${exe}`);
if (!existsSync(bin)) {
  console.log('dev-browser: no debug binary; building it');
  const build = spawnSync(process.execPath, ['scripts/build-app.mjs', '--debug'], {
    stdio: 'inherit',
    env: process.env,
    cwd: appDir,
  });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

// `--remove` exists for a machine that already has a dev registration from the
// first version of this launcher. The debug binary removes only `app.sucr.capture.dev`
// keys and manifests, so the installed registration is not in reach.
if (own.includes('--remove')) {
  const out = spawnSync(bin, ['--remove-addon'], { encoding: 'utf8' });
  process.stdout.write(out.stdout ?? '');
  process.stderr.write(out.stderr ?? '');
  console.log('\ndev-browser: dev relay unregistered; the installed registration was not touched');
  process.exit(out.status ?? 0);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// --- the dev relay ---------------------------------------------------------

const doorFile = (data) => join(data, 'capture-door.json');

function readDoor(data) {
  try {
    return JSON.parse(readFileSync(doorFile(data), 'utf8'));
  } catch {
    return null;
  }
}

/// True only for a Sucr door: a port can be reused by something else between
/// an app run and this check.
async function live(d) {
  if (!d || !d.port || !d.token) return false;
  try {
    const res = await fetch(`http://127.0.0.1:${d.port}/ping`, {
      headers: { 'x-sucr-token': d.token },
      signal: AbortSignal.timeout(1500),
    });
    const body = await res.json();
    return res.ok && body && body.app === 'sucr';
  } catch {
    return false;
  }
}

/// Bring up the dev app if it is not already answering, and hand back its door.
async function ensureApp(data) {
  const current = readDoor(data);
  if (await live(current)) return current;
  spawn(bin, [], { detached: true, stdio: 'ignore' }).unref();
  for (let i = 0; i < 150; i++) {
    await sleep(200);
    const d = readDoor(data);
    if (await live(d)) return d;
  }
  fail('the dev app did not open its door');
}

/// The add-on reads this file from its own package folder, so the session needs
/// no registration anywhere else on the machine.
function writeRelay(data, d) {
  const info = JSON.stringify({ port: d.port, token: d.token, at: d.startedAt || Date.now() }, null, 0);
  for (const family of ['chrome', 'firefox']) {
    const dir = join(data, 'addon', family);
    if (!existsSync(dir)) continue;
    writeFileSync(join(dir, 'relay.json'), info);
  }
}

/// A restart of the dev app moves the door; the session follows it instead of
/// going quiet until the launcher is run again. Exits with the browser.
async function watch(pid, data) {
  let written = '';
  for (;;) {
    await sleep(2000);
    try {
      process.kill(pid, 0);
    } catch {
      return;
    }
    const d = readDoor(data);
    if (!(await live(d))) continue;
    const stamp = `${d.port}/${d.token}`;
    if (stamp === written) continue;
    writeRelay(data, d);
    written = stamp;
  }
}

if (own[0] === '--watch') {
  await watch(Number(own[1]), own[2]);
  process.exit(0);
}

// --- the add-on ------------------------------------------------------------

const addonBuild = spawnSync('bash', [join(repoRoot, 'extension', 'build.sh')], {
  stdio: 'inherit',
  cwd: repoRoot,
});
if (addonBuild.error || addonBuild.status !== 0) {
  fail(
    `could not package the add-on (${addonBuild.error ? addonBuild.error.message : `exit ${addonBuild.status}`}); run "bash extension/build.sh" by hand to see why`,
  );
}

// --- which browser ---------------------------------------------------------

function commandExe(command) {
  const text = command.trim();
  const path = text.startsWith('"') ? text.slice(1).split('"')[0] : text.split(/\s+/)[0];
  const end = path.toLowerCase().indexOf('.exe');
  return end < 0 ? null : path.slice(0, end + 4);
}

const CHROMIUM = new Set(['chrome', 'msedge', 'brave', 'vivaldi', 'chromium', 'thorium', 'slimjet', 'helium']);
const GECKO = new Set(['firefox', 'firefox-esr', 'zen', 'librewolf', 'waterfox', 'floorp', 'palemoon']);

function familyOf(path) {
  const stem = basename(path).replace(/\.exe$/i, '').toLowerCase();
  return CHROMIUM.has(stem) ? 'chromium' : GECKO.has(stem) ? 'gecko' : null;
}

/// Every browser the shell lists as an internet client, read the same way the
/// app reads it: only the registered command counts, never a guessed path.
function startMenuBrowsers() {
  const found = [];
  if (!isWin) return found;
  for (const hive of ['HKCU', 'HKLM']) {
    const r = spawnSync('reg', ['query', `${hive}\\Software\\Clients\\StartMenuInternet`, '/s'], {
      encoding: 'utf8',
    });
    if (r.status !== 0 || !r.stdout) continue;
    let key = '';
    for (const line of r.stdout.split(/\r?\n/)) {
      const text = line.trim();
      if (text.startsWith('HKEY_')) {
        key = text;
        continue;
      }
      if (!/\\shell\\open\\command$/i.test(key)) continue;
      const match = text.match(/REG_SZ\s+(.+)$/);
      if (!match) continue;
      const path = commandExe(match[1]);
      if (!path || !existsSync(path) || !familyOf(path)) continue;
      if (found.some((b) => b.exe.toLowerCase() === path.toLowerCase())) continue;
      // The key ends in `\shell\open\command`; the segment before that is the
      // browser's registered name (e.g. `Helium`), which titles its profile.
      const label = key.replace(/\\shell\\open\\command$/i, '').split('\\').pop().split(/[-.]/)[0];
      found.push({ exe: path, name: label, family: familyOf(path) });
    }
  }
  return found;
}

const browsers = startMenuBrowsers();
if (!browsers.length) fail('no supported browser is registered on this machine');

const wantedArg = own.find((a) => a.startsWith('--browser='));
const wanted = wantedArg ? wantedArg.slice('--browser='.length) : null;
const browser = wanted
  ? browsers.find((b) => [b.exe, b.name, basename(b.exe, '.exe')]
      .some((v) => v.toLowerCase() === wanted.toLowerCase()))
  : browsers.find((b) => b.family === 'chromium') ?? browsers[0];
if (!browser) fail(`"${wanted}" is not a registered browser (saw ${browsers.map((b) => b.name).join(', ')})`);

// --- stage, wire, launch ---------------------------------------------------

const data = devDataDir();
const profile = join(data, 'browser', browser.name.toLowerCase().replace(/[^a-z0-9]+/g, '-'));
const addon = join(data, 'addon', browser.family === 'chromium' ? 'chrome' : 'firefox');

// A stale copy would keep an extra file the current build no longer ships.
rmSync(addon, { recursive: true, force: true });
mkdirSync(addon, { recursive: true });
cpSync(join(repoRoot, 'extension', 'dist', 'dev', basename(addon)), addon, { recursive: true });
mkdirSync(profile, { recursive: true });

const door = await ensureApp(data);
writeRelay(data, door);

// A window that looks like the user's own browser is the one thing that made
// this session confusing; the first tab says what it is.
const marker = join(data, 'browser', 'session.html');
writeFileSync(
  marker,
  `<!doctype html><meta charset="utf-8"><title>Sucr dev session</title>
<style>
  :root { color-scheme: light dark; }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
         font: 16px/1.55 system-ui, "Segoe UI", sans-serif; background: #fafafa; color: #1a1a1a; }
  main { max-width: 34rem; padding: 2.5rem; }
  h1 { font-size: 1.5rem; margin: 0 0 1rem; }
  p { margin: 0 0 0.85rem; }
  code { background: #eee; padding: 0.1rem 0.3rem; border-radius: 3px; }
  @media (prefers-color-scheme: dark) { body { background: #161616; color: #ececec; } code { background: #2a2a2a; } }
</style>
<main>
  <h1>Sucr dev session</h1>
  <p>This window is a test session for the local Sucr build. It runs on its own
     profile, its own add-on (<strong>Sucr Capture (dev)</strong>) and its own data
     folder, <code>${data.replaceAll('&', '&amp;').replaceAll('<', '&lt;')}</code>.</p>
  <p>Your normal browser is untouched: same profile, same add-on, same Sucr. Nothing
     in this window can reach the installed app, and nothing from the installed app
     can reach this session.</p>
  <p>Sucr itself reaches this session through <code>relay.json</code> in the
     add-on folder, so no native-messaging registration was written anywhere.</p>
</main>
`,
  'utf8',
);

console.log(`dev-browser: relay 127.0.0.1:${door.port} (dev build, ${data})`);
console.log(`dev-browser: add-on ${addon}`);
console.log(`dev-browser: browser ${browser.name} (${browser.exe})`);
console.log(`dev-browser: profile ${profile}`);
console.log(`dev-browser: no registry entries — the installed relay and add-on are untouched`);

const url = pathToFileURL(marker).href;

if (browser.family === 'chromium') {
  const child = spawn(
    browser.exe,
    [
      `--user-data-dir=${profile}`,
      `--load-extension=${addon}`,
      `--disable-extensions-except=${addon}`,
      '--no-first-run',
      '--no-default-browser-check',
      ...passthrough,
      url,
    ],
    { detached: true, stdio: 'ignore' },
  );
  child.unref();
  spawn(process.execPath, [self, '--watch', String(child.pid), data], {
    detached: true,
    stdio: 'ignore',
  }).unref();
  console.log('dev-browser: launched; the dev add-on is loaded in this profile only');
} else {
  const child = spawn(
    browser.exe,
    ['-no-remote', '-profile', profile, ...passthrough, url, 'about:debugging#/runtime/this-firefox'],
    { detached: true, stdio: 'ignore' },
  );
  child.unref();
  spawn(process.execPath, [self, '--watch', String(child.pid), data], {
    detached: true,
    stdio: 'ignore',
  }).unref();
  console.log(
    `dev-browser: launched; load the temporary add-on from ${join(addon, 'manifest.json')} in the debugging tab`,
  );
}
