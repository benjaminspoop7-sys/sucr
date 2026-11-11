// M0 gate for the Tauri shell: builds the web bundle, runs Rust tests, launches
// the real app with --smoke (which drives window.sucr from inside WebView2 and
// hands the report back through the Rust side), then checks the database it
// produced. Exits non-zero on any mismatch.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { DatabaseSync } from 'node:sqlite';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const exe = path.join(root, 'src-tauri', 'target', 'release', 'sucr.exe');
const dataDir = mkdtempSync(path.join(tmpdir(), 'sucr-m0-'));

const failures = [];
const check = (name, ok, detail = '') => {
  if (!ok) failures.push(`${name}${detail ? ` — ${detail}` : ''}`);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  (${detail})` : ''}`);
};

console.log(`data dir: ${dataDir}`);
check('release exe built', existsSync(exe), path.relative(root, exe));
if (!existsSync(exe)) process.exit(1);

const size = readFileSync(exe).length;
// 4 MB held until close-to-tray needed tauri's tray-icon feature (+70 KB).
// 4.2 MB held until the add bar's link check needed its own async command; the
// IPC glue for one more command entry costs 11 KB, and the check has to be off
// the main thread or a dead host would freeze the window while the user types.
// 4.3 MB held until the compact rail needed a 150 ms hold on its departure —
// one new declaration inside an existing :root block cost 512 B, and the
// headroom fell to 204 B with nothing in the queue to spend it on.
// RULED 2026-11-04, on the user's instruction to "rule the budget": the ceiling
// is 5 MB. The ratchet had become the constraint instead of the measurement --
// every slice was being shaped by 11 KB lines rather than by what users feel, and
// the gap between a 4.3 MB and a 5 MB binary is invisible next to the 458 MB of
// the Electron app this replaces. The ceiling still gates: it catches an
// accidental dependency, not a planned feature. Still judged per slice: cost is
// measured and named before it is accepted, never discovered afterwards.
check('exe under 5 MB', size < 5 * 1024 * 1024, `${(size / 1024 / 1024).toFixed(2)} MB`);

const run = () =>
  spawnSync(exe, ['--smoke', `--data-dir=${dataDir}`], { timeout: 120_000, encoding: 'utf8' });

const first = run();
const reportPath = path.join(dataDir, 'm0-smoke.json');
check('app exited 0', first.status === 0, `status ${first.status}`);
check('smoke report written', existsSync(reportPath));

if (existsSync(reportPath)) {
  const report = JSON.parse(readFileSync(reportPath, 'utf8'));
  check('window.sucr present', report.ok === true, report.error ?? '');
  check('channel lists agree', report.channelsMatch === true);
  check('38 channels', report.info?.channelCount === 38, String(report.info?.channelCount));
  check(
    'domains derived from schema',
    JSON.stringify(report.domains) ===
      JSON.stringify(['addon', 'app', 'category', 'debug', 'download', 'history', 'network', 'origin', 'proxy', 'session', 'settings', 'update']),
    JSON.stringify(report.domains),
  );
  check(
    'methods derived from schema',
    JSON.stringify(report.methods) ===
      JSON.stringify({
        addon: ['setup'],
        app: [
          'createFolder',
          'folderState',
          'getFolderPresets',
          'getInfo',
          'getPaths',
          'pickFolder',
          'quit',
          'setSpeedLimit',
        ],
        category: ['list'],
        debug: ['report'],
        download: [
          'add',
          'get',
          'list',
          'map',
          'open',
          'pause',
          'probe',
          'refreshUrl',
          'remove',
          'reveal',
          'setCategory',
          'setSpeedLimit',
          'start',
          'unwatch',
          'watch',
        ],
        history: ['tail'],
        network: ['list', 'setEnabled', 'setLimits'],
        origin: ['setAuth'],
        proxy: ['setAuth'],
        session: ['forget', 'list', 'set'],
        settings: ['list', 'set'],
        update: ['check'],
      }),
    JSON.stringify(report.methods),
  );
  check('missing download has no map', report.mapNull === true, String(report.mapNull));
  check(
    'folder dialog wired but refused under --smoke',
    report.folderPick === 'refused under smoke',
    String(report.folderPick),
  );
  check(
    'Browse reaches the picker and takes the refusal quietly',
    report.browse?.errorShown === false && report.browse?.folderKept === true,
    JSON.stringify(report.browse),
  );
  // A quick folder is only worth a chip if the folder exists, and this process is
  // the one party here that can prove it: the renderer asked the main process to
  // scan the drive and the profile, and the paths it got back must be directories
  // right now — not paths a string happened to assemble.
  check(
    'folder presets are real directories named for their preset',
    Array.isArray(report.presets?.resolved) &&
      report.presets.resolved.length > 0 &&
      report.presets.chips === report.presets.resolved.length &&
      report.presets.resolved.every((p) => {
        try {
          if (!statSync(p.path).isDirectory()) return false;
        } catch {
          return false;
        }
        // A chip must name the folder, not the drive or any folder merely holding
        // it: the reported bug was Downloads resolving to `D:\`.
        const leaf = p.path.trim().replace(/[\\/]+$/, '').split(/[\\/]/).pop();
        return leaf?.toLowerCase() === p.preset;
      }),
    JSON.stringify(report.presets),
  );
  // The create affordance, proved against the disk rather than the DOM: the
  // renderer says the folder was missing and blocked Download while it was, the
  // Create button answered, and the folder the button promised is a directory
  // this process can stat. It lives in this run's own data dir, so the cleanup at
  // the end of the gate takes it with everything else.
  check(
    'a missing folder is offered, created on request, and really on disk',
    report.folderCreate?.noticeShown === true &&
      report.folderCreate?.missingWhileAbsent === true &&
      report.folderCreate?.blockedWhileAbsent === true &&
      report.folderCreate?.noticeCleared === true &&
      report.folderCreate?.fieldKept === true &&
      report.folderCreate?.enabledAgain === true &&
      // The race: the field was retyped after the probe and the button clicked
      // straight away. Node proves which of the two folders the disk now has.
      report.folderCreate?.lateCreated === true &&
      report.folderCreate?.probedLeftAlone === true &&
      report.folderCreate?.fieldFollowsClick === true &&
      (() => {
        try {
          return statSync(report.folderCreate?.target ?? '').isDirectory();
        } catch {
          return false;
        }
      })() &&
      (() => {
        try {
          return statSync(report.folderCreate?.target2 ?? '').isDirectory();
        } catch {
          return false;
        }
      })() &&
      !existsSync(report.folderCreate?.target1 ?? ''),
    JSON.stringify(report.folderCreate),
  );
  // The wizard's step-1 gate is the same rule on the other surface, and the
  // escape hatch has to stay reachable: a notice must never become a trap. Its
  // chips are the bar's chips, so Node stats every path they claim to offer.
  check(
    'the wizard blocks Next on a missing folder and lifts it after Create',
    report.wizardGate?.noticeShown === true &&
      report.wizardGate?.nextBlocked === true &&
      report.wizardGate?.nextLabel === 'Next' &&
      report.wizardGate?.skipLive === true &&
      report.wizardGate?.noticeCleared === true &&
      report.wizardGate?.nextLive === true &&
      report.wizardGate?.onePressedAfterClick === true &&
      report.wizardGate?.fieldIsChip === true &&
      report.wizardGate?.chipUnblocks === true &&
      Array.isArray(report.wizardGate?.chips) &&
      report.wizardGate.chips.length > 0 &&
      report.wizardGate.chips.every((c) => {
        if (!['Downloads', 'Desktop', 'Documents'].includes(c.label)) return false;
        try {
          return statSync(c.path).isDirectory();
        } catch {
          return false;
        }
      }) &&
      (() => {
        try {
          return statSync(report.wizardGate?.target ?? '').isDirectory();
        } catch {
          return false;
        }
      })(),
    JSON.stringify(report.wizardGate),
  );
  // The refresh channel's refusal contract, proved without a server: a link it
  // cannot check is turned down, and the row keeps both its link and its status.
  check(
    'refused refresh leaves the row alone',
    report.refresh?.rejected === true &&
      report.refresh?.urlKept === true &&
      report.refresh?.statusKept === true,
    JSON.stringify(report.refresh),
  );
  // Which half of the clipboard this webview even offers. Reported, never
  // written: the gate must not replace what the user had copied, and a synthetic
  // click has no user activation, so a write here would fail for the wrong reason.
  check(
    'clipboard reachability reported',
    typeof report.clipboard?.secure === 'boolean' && typeof report.clipboard?.api === 'boolean',
    JSON.stringify(report.clipboard),
  );
  // Both spawn a window, so --smoke answers before the database is read. That
  // makes this a round-trip proof only: the channel is registered, the args
  // parse, and the answer is the shape the menu switches on. ipc.rs's
  // open_answers_with_the_reason_not_a_window covers the arms hidden by it.
  check(
    'open and reveal round-trip',
    typeof report.openReveal?.open?.ok === 'boolean' &&
      typeof report.openReveal?.reveal?.ok === 'boolean',
    JSON.stringify(report.openReveal),
  );
  check(
    'networks dashboard renders one card per adapter',
    report.networkView?.shown === true &&
      report.networkView?.cards === Math.max(report.counts?.networks ?? 0, 1) &&
      report.networkView?.switches === (report.counts?.networks ?? 0),
    JSON.stringify(report.networkView),
  );
  // The limit chips are the only way a person reaches `network:set-limits`, so
  // the gate opens one, saves a cap, and reads the row: a channel that exists
  // but has no control on the surface is the failure this feature set out to end.
  check(
    'a card cap chip saves through network:set-limits and clears again',
    report.networkLimits?.chip === true &&
      report.networkLimits?.opened === true &&
      report.networkLimits?.closed === true &&
      report.networkLimits?.stored === 2_097_152 &&
      report.networkLimits?.cleared === null,
    JSON.stringify(report.networkLimits),
  );
  // The proxy row, same shape of proof: the panel's own click writes the key, the
  // engine's answer comes back on the card line, the boundary's refusal lands on
  // the card in an error tone with the typed text still editable, and every key
  // is cleared again so no later check inherits a stored proxy. The byte path
  // through a live proxy is not this gate's claim — `proxy-run.mjs` in the
  // capability harness proves that against the shipped exe.
  //
  // The Networks pane is in the conjunction because a mode that resolved to a
  // proxy has to say so in its own words: the cards show no adapter carrying the
  // link, and the pane's totals now add the separate unattributed ledger, so the
  // sentence is what tells a reader why no card moves.
  check(
    'the proxy panel writes, reads the engine back, refuses credentials, and leaves nothing stored',
    report.proxy?.manualPressed === true &&
      report.proxy?.via === true &&
      (report.proxy?.viaText ?? '').includes('manual 127.0.0.1:8199') &&
      report.proxy?.bypassStored === 'localhost' &&
      report.proxy?.netNotes === true &&
      (report.proxy?.netNoteText ?? '').includes(
        'A proxy carries new links (manual 127.0.0.1:8199)',
      ) &&
      report.proxy?.refusedTone === 'error' &&
      (report.proxy?.refusedText ?? '').includes('carries no credentials') &&
      report.proxy?.refusedKeepsBox === true &&
      report.proxy?.refusedKeepsStore === true &&
      // The credential row: stored through the boxes, witnessed by the engine's own
      // `applies`, refused when a password arrives with no name in front of it, and
      // forgotten again. The secret is checked for absence twice — on the card line
      // that names the pair, and in this report, which is a file on disk.
      report.proxy?.authLine === true &&
      (report.proxy?.authLineText ?? '').includes('gate-user for 127.0.0.1:8199') &&
      !(report.proxy?.authLineText ?? '').includes('gate-secret') &&
      report.proxy?.authApplied === true &&
      report.proxy?.authNoSecret === true &&
      report.proxy?.passCleared === true &&
      report.proxy?.refusedNameless === true &&
      report.proxy?.refusedNamelessTone === 'error' &&
      report.proxy?.refusedNamelessKeepsStore === true &&
      report.proxy?.authForgotten === true &&
      report.proxy?.stored?.mode === 'direct' &&
      report.proxy?.stored?.url === null &&
      report.proxy?.stored?.bypass === null,
    JSON.stringify(report.proxy),
  );
  check(
    'adapter rows carry an IPv4 source and a kind',
    Array.isArray(report.adapters) &&
      report.adapters.every(
        (a) => /^\d+\.\d+\.\d+\.\d+$/.test(a.ip) && typeof a.kind === 'string',
      ),
    JSON.stringify(report.adapters),
  );
  const bytes = (v) => typeof v === 'string' && /^[\d.]+ (B|KB|MB|GB|TB)$/.test(v);
  check(
    'summary paints an adapter count and two byte amounts',
    report.networkView?.stats?.length === 3 &&
      /^\d+$/.test(report.networkView.stats[0] ?? '') &&
      bytes(report.networkView.stats[1]) &&
      bytes(report.networkView.stats[2]),
    JSON.stringify(report.networkView?.stats),
  );
  // Which window each figure is must survive a rewording, so the windows are
  // matched on meaning, not on the exact label Antigravity owns.
  const windows = (list) =>
    /today/i.test(list?.[1] ?? '') && /lifetime/i.test(list?.[2] ?? '');
  check(
    'both summary windows are named, today and lifetime',
    windows(report.networkView?.statLabels),
    JSON.stringify(report.networkView?.statLabels),
  );
  check(
    'adapter cards state both windows and the row carries a lifetime figure',
    /today/i.test(report.networkView?.cardMeta ?? '') &&
      /lifetime/i.test(report.networkView?.cardMeta ?? '') &&
      (report.networkView?.lifetimeType === 'number' ||
        report.networkView?.lifetimeType === 'no adapters'),
    JSON.stringify([report.networkView?.cardMeta, report.networkView?.lifetimeType]),
  );
  check(
    'write path round-trips',
    report.write?.added > 0 && report.write?.afterAdd === 2 && report.write?.afterRemove === 0,
    JSON.stringify(report.write),
  );
  // The library is filed, not merely listed: five seeded categories, an extension
  // that matches a rule, an override that beats it, a clear, and a refusal for an
  // id that is not in the table.
  check(
    'five categories seed once',
    JSON.stringify(report.category?.seeded) ===
      JSON.stringify(['Archives', 'Videos', 'Audio', 'Documents', 'Programs']),
    JSON.stringify(report.category?.seeded),
  );
  check(
    'the extension files the row and an override moves it',
    report.category?.filed === true &&
      report.category?.moved === true &&
      report.category?.cleared === true,
    JSON.stringify(report.category),
  );
  check(
    'a name with no rule stays unfiled, and an unknown category is refused',
    report.category?.unfiled === true && report.category?.refused === true,
    JSON.stringify(report.category?.reason ?? ''),
  );
  check(
    'app:get-info round-trips',
    report.info?.journalMode === 'wal' && report.info?.schemaVersion === 4,
    `${report.info?.journalMode} v${report.info?.schemaVersion} sqlite ${report.info?.sqlite}`,
  );
  check(
    'speed limits round-trip',
    report.speedLimit?.perJob === 131072 &&
      report.speedLimit?.globalSet === 262144 &&
      report.speedLimit?.globalCleared === null,
    JSON.stringify(report.speedLimit),
  );
  check(
    'settings round-trip through SQLite',
    report.settings?.after?.['engine.max_active_downloads'] === '5' &&
      report.settings?.cleared?.['engine.max_active_downloads'] === undefined,
    JSON.stringify(report.settings),
  );
  // This box reports prefers-reduced-motion: reduce, so the OS flag alone cannot
  // tell a live animation from a collapsed one. ui.motion is Sucr's own switch,
  // and it has to outrank Windows in both directions; clearing the key returns
  // the row to the install default, which is Full.
  check(
    "Sucr's motion switch outranks the OS preference",
    report.motion?.fullMoves === true &&
      report.motion?.reducedStills === true &&
      report.motion?.clearedIsFull === true,
    JSON.stringify(report.motion),
  );
  // A pasted session is written once and never read back: the link reduces to the
  // host a job looks up, the list reports the size not the value, a cookie that
  // cannot be a header is refused without leaving a row, and forget clears it.
  check(
    'a browser session stores, lists without echoing, and forgets',
    report.session?.host === '127.0.0.1' &&
      report.session?.bytesKept === true &&
      report.session?.agentKept === true &&
      report.session?.listed === true &&
      report.session?.noEcho === true &&
      report.session?.refused === true &&
      report.session?.refusedLeftNothing === true &&
      report.session?.dropped === true &&
      report.session?.cleared === true,
    JSON.stringify(report.session),
  );
  // The site credential is stored by a click on the row's own Save button, so this
  // is the only check that the paste form, the row list and the Forget button are
  // wired to each other and not only to the bridge. What it also has to prove is
  // the keying: the pair is stored under the host the link reduces to, one host's
  // forget leaves the other standing, and no reading of the info payload carries
  // the password back. The last six terms are the bearer half: the kind switch
  // takes the username box away, the token replaces a pair saved for the same host
  // rather than joining it, the row names the kind it holds, and the same Forget
  // button drops a credential the form never stored as a pair.
  check(
    'a site credential stores by click, keys by host, never echoes, and forgets one host at a time',
    report.origin?.line === true &&
      report.origin?.keyed === true &&
      report.origin?.listed === true &&
      report.origin?.bytesKept === true &&
      report.origin?.noEcho === true &&
      report.origin?.boxesEmpty === true &&
      report.origin?.refused === true &&
      report.origin?.refusedLeftNothing === true &&
      report.origin?.dropped === true &&
      report.origin?.isolation === true &&
      report.origin?.cleared === true &&
      report.origin?.tokenUserBoxHidden === true &&
      report.origin?.tokenKindListed === true &&
      report.origin?.tokenReplacedPair === true &&
      report.origin?.tokenNamedInRow === true &&
      report.origin?.tokenNoEcho === true &&
      report.origin?.tokenDropped === true,
    JSON.stringify(report.origin),
  );
  check(
    'a fresh data dir reads as a first run',
    report.setupFirstRun === true && report.wizardInDom === true,
    JSON.stringify({ firstRun: report.setupFirstRun, wizard: report.wizardInDom }),
  );
  check('empty library', report.counts?.downloads === 0, JSON.stringify(report.counts));
  check('six inspector tabs', report.tabs === 6, String(report.tabs));
  check(
    'tabs respond to ArrowRight, Ctrl+3 and Ctrl+5',
    report.keyboard?.first === 'tab-overview' &&
      report.keyboard?.afterArrow === 'tab-connections' &&
      report.keyboard?.afterCtrl3 === 'tab-source' &&
      report.keyboard?.afterCtrl5 === 'tab-files',
    JSON.stringify(report.keyboard),
  );
  check('webview reported', typeof report.info?.webview === 'string' && report.info.webview.length > 0,
    report.info?.webview);
}

const dbPath = path.join(dataDir, 'sucr.db');
check('sucr.db exists', existsSync(dbPath));

if (existsSync(dbPath)) {
  const db = new DatabaseSync(dbPath);
  const { user_version: version } = db.prepare('PRAGMA user_version').get();
  const { journal_mode: journal } = db.prepare('PRAGMA journal_mode').get();
  const tables = db
    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
    .all()
    .map((t) => t.name);
  const categories = db.prepare('SELECT name FROM categories ORDER BY sort_order, id').all();
  const rules = db
    .prepare("SELECT patterns_json FROM categories WHERE name = 'Archives'")
    .get();
  db.close();
  check('schema v4', version === 4, String(version));
  check('journal mode wal', journal === 'wal', journal);
  check('17 tables', tables.length === 17, `${tables.length}: ${tables.join(', ')}`);
  check(
    'the seed survived the smoke run',
    categories.length === 5,
    categories.map((c) => c.name).join(', '),
  );
  // The rule lives in the row, so the table — not a copy in the renderer — is
  // what the writer reads.
  check(
    'the archive rule is stored as patterns',
    JSON.parse(rules?.patterns_json ?? '[]').includes('zip') === true,
    rules?.patterns_json,
  );
}

const second = run();
check('second launch migrates nothing', second.status === 0, `status ${second.status}`);

if (existsSync(dbPath)) {
  const db = new DatabaseSync(dbPath);
  const { user_version: version } = db.prepare('PRAGMA user_version').get();
  const tables = db
    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
    .all();
  const after = db.prepare('SELECT count(*) AS n FROM categories').get();
  db.close();
  check('still schema v4 after a second open', version === 4, String(version));
  check('and 17 tables', tables.length === 17, String(tables.length));
  // The seed keys off an empty table, so relaunching cannot double the rail.
  check('a second launch does not re-seed', after?.n === 5, String(after?.n));
}

if (failures.length === 0) {
  rmSync(dataDir, { recursive: true, force: true });
  console.log(`\nM0 GATE: PASS  (exe ${(size / 1024 / 1024).toFixed(2)} MB)`);
} else {
  console.log(`\nM0 GATE: FAIL (${failures.length})\n - ${failures.join('\n - ')}\nreport kept at ${dataDir}`);
  process.exitCode = 1;
}
