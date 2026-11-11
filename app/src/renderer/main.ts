import { listen } from '@tauri-apps/api/event';
import { CHANNELS } from '../shared/ipc';
import type {
  AddonReport,
  AppInfo,
  BlockMap,
  BrowserSession,
  Category,
  DebugReport,
  Download,
  LinkProbe,
  MapAttempt,
  MapBlock,
  Network,
  NetworkList,
  OriginAuth,
  UpdateReport,
} from '../shared/models';
import { invoke, sucr } from './sucr';

// The release channel this build came from. Shown beside the version so nobody
// mistakes an early build for a finished one.
const BUILD_STAGE = 'Alpha';

const $ = <T extends Element>(sel: string): T => {
  const el = document.querySelector(sel);
  if (!el) throw new Error(`missing element: ${sel}`);
  return el as unknown as T;
};

const $$ = <T extends Element>(sel: string): T[] =>
  Array.from(document.querySelectorAll<T>(sel));

/* --- theme: one preference, three values, applied as a class ------------- */
type ThemePref = 'system' | 'light' | 'dark';

const THEME_KEY = 'sucr.theme';
const darkQuery = window.matchMedia('(prefers-color-scheme: dark)');
const themeButtons = $$<HTMLButtonElement>('.theme-seg [data-theme-pref]');

let pref: ThemePref = 'system';
const stored = localStorage.getItem(THEME_KEY);
if (stored === 'light' || stored === 'dark' || stored === 'system') pref = stored;

function applyTheme(): void {
  const dark = pref === 'dark' || (pref === 'system' && darkQuery.matches);
  document.documentElement.classList.toggle('sucr-theme-dark', dark);
  document.documentElement.classList.toggle('sucr-theme-light', !dark);
  for (const button of themeButtons) {
    button.setAttribute('aria-pressed', String(button.dataset['themePref'] === pref));
  }
  localStorage.setItem(THEME_KEY, pref);
  alignIndicator();
}

for (const button of themeButtons) {
  button.addEventListener('click', () => {
    button.classList.remove('animating');
    void button.offsetWidth;
    button.classList.add('animating');
    window.setTimeout(() => button.classList.remove('animating'), 360);
    const next = button.dataset['themePref'];
    if (next === 'system' || next === 'light' || next === 'dark') {
      pref = next;
      applyTheme();
      renderSettings();
      void saveSetting('ui.theme', next).catch(reportSettingError);
    }
  });
}
darkQuery.addEventListener('change', () => {
  if (pref === 'system') applyTheme();
});

/* --- motion: Sucr's own switch outranks the OS preference ----------------- */
type MotionPref = 'system' | 'full' | 'reduced';
const MOTION_KEY = 'sucr.motion';
const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');

let motionPref: MotionPref = 'full';
const storedMotion = localStorage.getItem(MOTION_KEY);
if (storedMotion === 'system' || storedMotion === 'full' || storedMotion === 'reduced') motionPref = storedMotion;

/** Every reduced-motion decision in this file goes through here, and the
 *  stylesheet reads the attribute `applyMotion` writes. Keyed on an attribute
 *  rather than on the media query because a `1ms !important` duration cannot
 *  be undone by a later override — see app.css. */
function motionReduced(): boolean {
  return motionPref === 'reduced' || (motionPref === 'system' && motionQuery.matches);
}

/* The crest speed is cached here rather than beside the wave: `applyMotion()` runs at
   module evaluation, before a `let` declared further down exists, and it has to
   invalidate that cache. See `waveTravel()`. */
let waveTravelDirty = true;
let waveTravelS = 1;

function applyMotion(): void {
  document.documentElement.dataset['motion'] = motionReduced() ? 'reduced' : 'full';
  localStorage.setItem(MOTION_KEY, motionPref);
  waveTravelDirty = true;
}

// Runs at module evaluation, before the first frame, so a stale localStorage
// mirror only ever paints for a moment: loadSettings hands the decision to
// SQLite (Ruling 5) as soon as the window's data is in.
applyMotion();
motionQuery.addEventListener('change', () => {
  if (motionPref === 'system') {
    applyMotion();
    // So the Settings row cannot go on describing the old preference.
    renderSettings();
  }
});

/* --- progress: segmented telemetry bar (plan §5.5) or aggregate wave ----- */
type ProgressMode = 'segmented' | 'wave';

const MODE_KEY = 'sucr.progress';
const progressBtn = $<HTMLButtonElement>('#progress-mode');
const progressLabel = $<HTMLElement>('#progress-label');

let progressMode: ProgressMode = localStorage.getItem(MODE_KEY) === 'wave' ? 'wave' : 'segmented';

function applyProgressMode(): void {
  const wave = progressMode === 'wave';
  progressBtn.setAttribute('aria-pressed', String(wave));
  progressBtn.title = wave ? 'Switch to the segmented telemetry bar' : 'Switch to the aggregate wave';
  progressLabel.textContent = wave ? 'Wave' : 'Segmented';
  progressBtn.querySelector('use')?.setAttribute('href', wave ? '#i-wave' : '#i-bar');
  localStorage.setItem(MODE_KEY, progressMode);
  renderRows();
  syncWaveLoop();
}

progressBtn.addEventListener('click', () => {
  progressBtn.classList.remove('animating');
  void progressBtn.offsetWidth;
  progressBtn.classList.add('animating');
  window.setTimeout(() => progressBtn.classList.remove('animating'), 360);
  progressMode = progressMode === 'wave' ? 'segmented' : 'wave';
  applyProgressMode();
  renderSettings();
  void saveSetting('ui.meter_style', progressMode).catch(reportSettingError);
});

/* --- settings: one SQLite table, five pillars (spec c-07) ---------------- */
const SETTING_DEFAULTS: Record<string, string> = {
  'engine.max_active_downloads': '3',
  'engine.default_streams': 'auto',
  'engine.auto_start': 'true',
  // Off by default, and a mask of 0 is every day: a profile that has never been
  // asked must download the moment it is told to. `settingOn` reads an absent key
  // as true, so this row is what keeps the switch honest before the first write.
  'engine.window_enabled': 'false',
  'engine.window_start': '',
  'engine.window_stop': '',
  'engine.window_days': '0',
  'storage.default_folder': '',
  'storage.preallocation': 'sparse',
  'storage.motw': 'true',
  'storage.auto_verify': 'true',
  'network.bonding_policy': 'dynamic',
  'network.metered_guard': 'true',
  'network.timeout_seconds': '30',
  'network.proxy_mode': 'direct',
  'network.proxy_url': '',
  'network.proxy_bypass': '',
  'ui.theme': 'system',
  'ui.meter_style': 'segmented',
  // Full on a fresh install: Sucr's motion is part of the product, and the
  // common Windows profile here reports `prefers-reduced-motion`, so following
  // the OS would ship a still app to most people. Reduced is one click away.
  'ui.motion': 'full',
  'ui.sound_on_complete': 'true',
  'ui.close_to_tray': 'true',
  'system.debug_log': 'true',
};

/** Keys that actually live in the database. The UI shows defaults for the
 *  rest, but only a stored key overrides localStorage or the topbar state. */
let storedSettings: Record<string, string> = {};
let settings: Record<string, string> = { ...SETTING_DEFAULTS };
let appInfo: AppInfo | null = null;

function setting(key: string): string {
  return settings[key] ?? '';
}

function settingOn(key: string): boolean {
  return setting(key) !== 'false';
}

async function loadSettings(): Promise<void> {
  storedSettings = await sucr.settings.list();
  settings = { ...SETTING_DEFAULTS, ...storedSettings };
  const theme = storedSettings['ui.theme'];
  if (theme === 'system' || theme === 'light' || theme === 'dark') pref = theme;
  const meter = storedSettings['ui.meter_style'];
  if (meter === 'segmented' || meter === 'wave') progressMode = meter;
  // Merged `settings`, not `storedSettings`: an absent row means the default
  // (Full), so a cleared key returns motion to that instead of leaving the
  // localStorage mirror in charge.
  const motion = settings['ui.motion'];
  if (motion === 'system' || motion === 'full' || motion === 'reduced') {
    motionPref = motion;
    applyMotion();
  }
}

async function saveSetting(key: string, value: string | null): Promise<void> {
  await sucr.settings.set(key, value);
  if (value === null) {
    delete storedSettings[key];
    settings[key] = SETTING_DEFAULTS[key] ?? '';
  } else {
    storedSettings[key] = value;
    settings[key] = value;
  }
  if (key === 'ui.theme' && pref !== value && (value === 'system' || value === 'light' || value === 'dark')) {
    pref = value;
    applyTheme();
  } else if (key === 'ui.meter_style' && progressMode !== value && (value === 'segmented' || value === 'wave')) {
    progressMode = value;
    applyProgressMode();
  } else if (key === 'ui.motion') {
    // A cleared key is the default (Full), not "whatever the last write left".
    const next: MotionPref = value === 'system' || value === 'reduced' ? value : 'full';
    if (motionPref !== next) {
      motionPref = next;
      applyMotion();
    }
  } else if (key.startsWith('network.proxy_')) {
    void syncProxyVia();
  } else if (key === 'system.debug_log') {
    // The engine switched the sink on this same write, so the reading that
    // follows is the new truth, not the one the toggle just painted.
    void refreshDebugLog();
  }
  renderSettings();
}

/** The egress line is the engine's answer, not these three keys: `system` is a
 *  question for the environment, and a `manual` mode holding an address the
 *  engine cannot read downloads direct. Neither is something the page can work
 *  out, so the answer is read back after any write that could have moved it. */
async function syncProxyVia(): Promise<void> {
  try {
    appInfo = await sucr.app.getInfo();
  } catch {
    // The settings write itself landed; the line keeps its last reading.
  }
  renderSettings();
}

function reportSettingError(err: unknown): void {
  const note = $<HTMLElement>('#set-system-note');
  note.dataset['tone'] = 'error';
  note.textContent = `Setting not saved: ${err instanceof Error ? err.message : String(err)}`;
}

/* Ruling 5: first run reads SQLite, never localStorage. WebView2 shares one
 * localStorage across every --data-dir profile on Windows, so the old flag
 * could not prove a clean start — which is also why the wizard had no gate
 * coverage until now. */
function setupPending(): boolean {
  return storedSettings['system.setup_completed'] !== 'true';
}

/** A profile set up before the flag moved has other stored settings but no
 *  flag; hand it one so it is not walked through the wizard a second time.
 *  A clean data dir has no rows at all, so it stays a first run. */
async function adoptLegacySetupFlag(): Promise<void> {
  if (storedSettings['system.setup_completed'] === undefined
    && Object.keys(storedSettings).length > 0) {
    await saveSetting('system.setup_completed', 'true');
  }
}

/* --- inspector tabs: roving tabindex, arrows, Home/End, Ctrl+1..6 -------- */
const tabs = $$<HTMLButtonElement>('[role="tab"]');
const panes = $$<HTMLElement>('[role="tabpanel"]');
const tabStrip = $<HTMLElement>('.tabs');
const indicator = $<HTMLElement>('#tab-indicator');
let activeTab = 0;

function alignIndicator(): void {
  const tab = tabs[activeTab];
  if (!tab) return;
  const width = tab.offsetWidth;
  indicator.style.width = `${width}px`;
  indicator.style.transform = `translateX(${tab.offsetLeft}px)`;
  indicator.style.visibility = width > 0 ? 'visible' : 'hidden';
}

function selectPane(index: number, focus: boolean): void {
  const tab = tabs[index];
  if (!tab) return;
  const direction = index >= activeTab ? 'slide-in-right' : 'slide-in-left';
  activeTab = index;
  tabs.forEach((t, i) => {
    const on = i === index;
    t.setAttribute('aria-selected', String(on));
    t.tabIndex = on ? 0 : -1;
    t.classList.remove('animating');
  });
  const pane = tab.dataset['pane'];
  for (const p of panes) {
    const on = p.dataset['pane'] === pane;
    if (on) {
      p.hidden = false;
      p.classList.remove('slide-in-right', 'slide-in-left');
      void p.offsetWidth;
      if (selectedId !== null) p.classList.add(direction);
    } else {
      p.hidden = true;
    }
  }
  tab.classList.remove('animating');
  void tab.offsetWidth;
  tab.classList.add('animating');
  window.setTimeout(() => tab.classList.remove('animating'), 360);
  alignIndicator();
  syncWaveLoop();
  void syncMap();
  if (focus) {
    tab.focus();
    tab.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }
}

tabs.forEach((tab, i) => tab.addEventListener('click', () => selectPane(i, false)));

tabStrip.addEventListener('keydown', (event) => {
  const e = event as KeyboardEvent;
  const current = tabs.findIndex((t) => t.getAttribute('aria-selected') === 'true');
  let next = -1;
  if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (current + 1) % tabs.length;
  else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (current - 1 + tabs.length) % tabs.length;
  else if (e.key === 'Home') next = 0;
  else if (e.key === 'End') next = tabs.length - 1;
  if (next < 0) return;
  e.preventDefault();
  selectPane(next, true);
});

document.addEventListener('keydown', (event) => {
  const e = event as KeyboardEvent;
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
  const match = /^([1-9])$/.exec(e.key);
  if (!match) return;
  const target = e.target as Element | null;
  if (target && typeof target.closest === 'function' && target.closest('input, textarea, select')) return;
  const index = Number(match[1]) - 1;
  if (index >= tabs.length) return;
  e.preventDefault();
  selectPane(index, true);
});

/* --- library state ------------------------------------------------------- */
const STATUS_LABEL: Record<string, string> = {
  queued: 'Queued',
  downloading: 'Downloading',
  paused: 'Paused',
  finished: 'Finished',
  verified: 'Verified',
  'needs-attention': 'Needs attention',
};

const CATEGORY_OF: Record<string, string> = {
  iso: 'archive', img: 'archive', zip: 'archive', rar: 'archive', '7z': 'archive', tar: 'archive',
  gz: 'archive', xz: 'archive', zst: 'archive', bz2: 'archive', bz: 'archive',
  tgz: 'archive', tbz2: 'archive', txz: 'archive', cab: 'archive', pup: 'archive',
  msi: 'program', exe: 'program', appx: 'program', dmg: 'program', deb: 'program', rpm: 'program', pkg: 'program',
  mp4: 'video', mkv: 'video', webm: 'video', avi: 'video', mov: 'video', m4v: 'video',
  mp3: 'audio', m4a: 'audio', flac: 'audio', wav: 'audio', ogg: 'audio', opus: 'audio',
  pdf: 'document', epub: 'document', doc: 'document', docx: 'document', txt: 'document',
  png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image', svg: 'image',
  torrent: 'torrent',
};

/** The engine's colour for bytes it cannot say which device carried: a block
 *  resumed from an older run, or one written before attribution existed. */
const UNATTRIBUTED = 'n0';

let library: Download[] = [];
let networks: Network[] = [];
/** The ledger's other half: bytes the engine booked to no adapter — a proxied
 *  tunnel, a localhost target, a route whose device has no card. Read on the
 *  same call as the adapter rows, so the two figures on this pane are one
 *  instant. */
let unattributed: NetworkList['unattributed'] = { today: 0, week: 0, month: 0, lifetime: 0 };
let categories: Category[] = [];
let selectedId: number | null = null;
let currentView = 'all';
let query = '';
let totals = { active: 0, bps: 0 };
/** The app-wide cap; per-download caps arrive on each row. */
let globalCap: number | null = null;

const rowsEl = $<HTMLDivElement>('#rows');
const emptyEl = $<HTMLElement>('#empty');
const emptyTitleEl = $<HTMLElement>('#empty-title');
const emptyBodyEl = $<HTMLElement>('#empty-body');
const viewTitleEl = $<HTMLElement>('#view-title');
const viewSubEl = $<HTMLElement>('#view-sub');
const summaryEl = $<HTMLElement>('#summary');
const detailEl = $<HTMLElement>('#detail');
const sourceEl = $<HTMLElement>('#source');
const scheduleEl = $<HTMLElement>('#schedule');
const filesEl = $<HTMLElement>('#files');
const activityEl = $<HTMLElement>('#activity');
const ihSub = $<HTMLElement>('#ih-sub');
const settingsViewEl = $<HTMLElement>('#settings-view');
const settingsBodyEl = $<HTMLElement>('#settings-body');
const networksViewEl = $<HTMLElement>('#networks-view');

function formatBytes(n: number | null | undefined): string {
  if (n === null || n === undefined) return '—';
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = n / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

function formatRate(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

/** 24-hour by ruling: `7:54:50 pm` wrapped onto two lines inside the trail's
 *  58 px time column, `19:54:50` cannot. */
function formatClock(ts: number): string {
  const d = new Date(ts);
  const two = (n: number) => String(n).padStart(2, '0');
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
}

/** The engine sleeps on max(global, job), so the enforced rate is the lower
 *  of the two caps; the chip must show exactly that number. */
function effectiveCap(d: Download): number | null {
  const caps = [d.speedLimit, globalCap].filter(
    (c): c is number => typeof c === 'number' && c > 0,
  );
  return caps.length > 0 ? Math.min(...caps) : null;
}

function capChip(cap: number): HTMLElement {
  const chip = document.createElement('span');
  chip.className = 'badge-capped';
  chip.title = `Speed capped at ${formatRate(cap)}`;
  const dot = document.createElement('span');
  dot.className = 'cap-dot';
  chip.append(dot, document.createTextNode(`Capped: ${formatRate(cap)}`));
  return chip;
}

function clockText(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = String(m).padStart(2, '0');
  const ss = String(sec).padStart(2, '0');
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function isSystemView(view: string): boolean {
  return view === 'networks' || view === 'settings';
}

function visibleDownloads(): Download[] {
  let base: Download[];
  if (isSystemView(currentView)) base = [];
  else if (currentView.startsWith('category:')) {
    const raw = currentView.slice('category:'.length);
    const id = Number(raw);
    if (Number.isFinite(id) && id > 0) {
      base = library.filter((d) => d.categoryId === id);
    } else {
      base = library.filter((d) => {
        const ext = d.filename.split('.').pop()?.toLowerCase() || '';
        return (CATEGORY_OF[ext] ?? 'misc') === raw;
      });
    }
  } else {
    switch (currentView) {
      case 'downloading':
        base = library.filter((d) => d.status === 'downloading');
        break;
      case 'queued':
        base = library.filter((d) => d.status === 'queued' || d.status === 'paused');
        break;
      case 'finished':
        base = library.filter((d) => d.status === 'finished' || d.status === 'verified');
        break;
      case 'attention':
        base = library.filter((d) => d.status === 'needs-attention');
        break;
      default:
        base = library;
    }
  }
  if (!query) return base;
  return base.filter((d) =>
    `${d.filename} ${d.url} ${d.finalUrl ?? ''} ${d.errorCode ?? ''}`.toLowerCase().includes(query),
  );
}

/* --- rate sampling: one sample per refresh, shared by cards and status --- */
const rates = new Map<number, { bytes: number; ts: number; bps: number }>();

function sampleBps(d: Download): number {
  if (d.status !== 'downloading') {
    rates.delete(d.id);
    return 0;
  }
  const now = performance.now();
  const prev = rates.get(d.id);
  if (!prev) {
    rates.set(d.id, { bytes: d.verifiedBytes, ts: now, bps: 0 });
    return 0;
  }
  const dt = (now - prev.ts) / 1000;
  if (dt < 0.05) return prev.bps;
  const instant = Math.max(0, (d.verifiedBytes - prev.bytes) / dt);
  const bps = prev.bps === 0 ? instant : prev.bps * 0.7 + instant * 0.3;
  rates.set(d.id, { bytes: d.verifiedBytes, ts: now, bps });
  return bps;
}

function sampleAll(): { active: number; bps: number } {
  let active = 0;
  let bps = 0;
  for (const d of library) {
    const rate = sampleBps(d);
    if (d.status === 'downloading') {
      active += 1;
      bps += rate;
    }
  }
  return { active, bps };
}

function rateOf(d: Download): number {
  return rates.get(d.id)?.bps ?? 0;
}

/* --- throughput history: one point per second over the last minute -------- *
 *  The EMA above only ever holds the newest value, so a curve needs its own
 *  ledger. Points carry a wall clock, not an index: refresh is event-driven,
 *  so an index axis would silently compress a stall into a shorter line
 *  instead of showing the gap the transfer actually left. */
const TP_WINDOW_MS = 60_000;
const TP_STEP_MS = 1_000;
const TP_H = 56;
type TpPoint = { t: number; v: number };
const tpTotal: TpPoint[] = [];
const tpJobs = new Map<number, TpPoint[]>();

function tpPush(series: TpPoint[], now: number, v: number): void {
  series.push({ t: now, v });
  while (series.length > 0 && now - series[0]!.t > TP_WINDOW_MS) series.shift();
}

function tpTick(): void {
  const now = Date.now();
  tpPush(tpTotal, now, totals.bps);
  const live = new Set<number>();
  for (const d of library) {
    live.add(d.id);
    let series = tpJobs.get(d.id);
    if (!series) tpJobs.set(d.id, (series = []));
    tpPush(series, now, d.status === 'downloading' ? rateOf(d) : 0);
  }
  for (const id of [...tpJobs.keys()]) if (!live.has(id)) tpJobs.delete(id);
  paintThroughput();
}

/** Rounds a peak up to the nearest readable step x 10^n for the top gridline. */
const TP_STEPS = [1, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];
function tpScale(peak: number): number {
  if (peak <= 0) return 1;
  const exp = 10 ** Math.floor(Math.log10(peak));
  const unit = peak / exp;
  return (TP_STEPS.find((s) => unit <= s) ?? 10) * exp;
}

function tpChart(
  host: HTMLElement,
  series: TpPoint[],
  label: string,
  filled = true,
): SVGSVGElement {
  // clientWidth includes the host's own padding, and in the Networks view the
  // host is the card — measuring it whole draws past the content box and clips
  // the right-hand label.
  const pad = getComputedStyle(host);
  const w = Math.max(
    120,
    Math.round(host.clientWidth - Number.parseFloat(pad.paddingLeft) - Number.parseFloat(pad.paddingRight)),
  );
  const now = Date.now();
  const t0 = now - TP_WINDOW_MS;
  const seen = series.filter((p) => p.t >= t0);
  // The newest point is also the "Now" label, so the plot stops short of the
  // right border instead of running into it.
  const span = w - 6;
  const x = (t: number): number => ((t - t0) / TP_WINDOW_MS) * span;
  const scale = tpScale(seen.reduce((m, p) => Math.max(m, p.v), 0));
  const y = (v: number): number => TP_H - 3 - (Math.min(v, scale) / scale) * (TP_H - 12);

  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('class', 'tp');
  svg.setAttribute('viewBox', `0 0 ${w} ${TP_H}`);
  svg.setAttribute('width', String(w));
  svg.setAttribute('height', String(TP_H));
  svg.setAttribute('role', 'img');
  const peak = seen.reduce((m, p) => Math.max(m, p.v), 0);
  svg.setAttribute('aria-label', `${label}, last minute, peak ${formatRate(peak)}`);

  for (const gy of [y(scale), TP_H - 3]) {
    const line = document.createElementNS(NS, 'line');
    line.setAttribute('class', 'tp-grid');
    line.setAttribute('x1', '0');
    line.setAttribute('x2', String(w));
    line.setAttribute('y1', String(gy));
    line.setAttribute('y2', String(gy));
    svg.append(line);
  }

  if (seen.length > 1) {
    const first = seen[0]!;
    const last = seen[seen.length - 1]!;
    const d = seen
      .map((p, i) => `${i ? 'L' : 'M'}${x(p.t).toFixed(1)} ${y(p.v).toFixed(1)}`)
      .join(' ');
    // The app-wide curve carries an area fill, a single download stays a line:
    // two different quantities should not wear the same shape.
    const parts: SVGElement[] = [];
    if (filled) {
      const area = document.createElementNS(NS, 'path');
      area.setAttribute('class', 'tp-area');
      area.setAttribute(
        'd',
        `${d} L${x(last.t).toFixed(1)} ${TP_H - 3} L${x(first.t).toFixed(1)} ${TP_H - 3} Z`,
      );
      parts.push(area);
    }
    const line = document.createElementNS(NS, 'path');
    line.setAttribute('class', 'tp-line');
    line.setAttribute('d', d);
    parts.push(line);
    const head = document.createElementNS(NS, 'circle');
    head.setAttribute('class', 'tp-head');
    head.setAttribute('cx', x(last.t).toFixed(1));
    head.setAttribute('cy', y(last.v).toFixed(1));
    head.setAttribute('r', '2.5');
    parts.push(head);
    svg.append(...parts);
  }

  const top = document.createElementNS(NS, 'text');
  top.setAttribute('class', 'tp-y');
  top.setAttribute('x', '3');
  top.setAttribute('y', String(y(scale) - 2));
  top.textContent = formatRate(scale);
  const ago = document.createElementNS(NS, 'text');
  ago.setAttribute('class', 'tp-x');
  ago.setAttribute('x', '3');
  ago.setAttribute('y', String(TP_H - 1));
  ago.textContent = '1m ago';
  const present = document.createElementNS(NS, 'text');
  present.setAttribute('class', 'tp-x');
  present.setAttribute('x', String(w - 3));
  present.setAttribute('y', String(TP_H - 1));
  present.setAttribute('text-anchor', 'end');
  present.textContent = 'Now';
  svg.append(top, ago, present);
  return svg;
}

function paintThroughput(): void {
  for (const host of document.querySelectorAll<HTMLElement>('[data-tp]')) {
    const whole = host.dataset.tp === 'total';
    const series = whole ? tpTotal : selectedId === null ? [] : tpJobs.get(selectedId) ?? [];
    host.replaceChildren(
      tpChart(host, series, whole ? 'Total throughput' : 'Download throughput', whole),
    );
  }
}

function pctOf(d: Download): number {
  if (d.totalBytes && d.totalBytes > 0) {
    return Math.max(0, Math.min(100, Math.floor((d.verifiedBytes / d.totalBytes) * 100)));
  }
  return d.status === 'finished' || d.status === 'verified' ? 100 : 0;
}

/** Same ratio without the floor: the wave needs the real fraction, only the
 *  labels want a whole percent. */
function pctExact(d: Download): number {
  if (d.totalBytes && d.totalBytes > 0) {
    return Math.max(0, Math.min(100, (d.verifiedBytes / d.totalBytes) * 100));
  }
  return d.status === 'finished' || d.status === 'verified' ? 100 : 0;
}

function etaOf(d: Download): string {
  if (d.status !== 'downloading' || !d.totalBytes) return '';
  const bps = rateOf(d);
  if (bps <= 0) return '';
  const remaining = Math.max(0, d.totalBytes - d.verifiedBytes);
  return `ETA ${clockText(remaining / bps)}`;
}

function statusRate(d: Download): string {
  switch (d.status) {
    case 'downloading': {
      const bps = rateOf(d);
      return bps > 0 ? formatRate(bps) : '—';
    }
    case 'queued':
      return 'Pending';
    case 'paused':
      return 'Paused';
    case 'finished':
      return 'Done';
    case 'verified':
      return 'Verified';
    case 'needs-attention':
      return 'Stopped';
    default:
      return d.status;
  }
}

function toneOf(status: string): string {
  if (status === 'finished' || status === 'verified') return 'ok';
  if (status === 'needs-attention') return 'bad';
  if (status === 'paused') return 'warn';
  if (status === 'downloading') return 'info';
  return 'muted';
}

/* --- file badge ---------------------------------------------------------- */
function fileBadge(filename: string): { label: string; cat: string } {
  const base = filename.split(/[?#]/)[0] ?? '';
  const lower = base.toLowerCase();
  const compound = ['tar.gz', 'tar.zst', 'tar.xz', 'tar.bz2'].find((s) => lower.endsWith(`.${s}`));
  const ext = compound
    ? compound.split('.')[0] ?? ''
    : lower.includes('.')
      ? lower.slice(lower.lastIndexOf('.') + 1)
      : '';
  const label = (compound ? 'TAR' : ext.slice(0, 4).toUpperCase()) || 'FILE';
  const cat = CATEGORY_OF[compound ? 'tar' : ext] ?? 'misc';
  return { label, cat };
}

const NS = 'http://www.w3.org/2000/svg';

function svgUse(id: string, className = ''): SVGSVGElement {
  const svg = document.createElementNS(NS, 'svg');
  if (className) svg.setAttribute('class', className);
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(NS, 'use');
  use.setAttribute('href', id);
  svg.append(use);
  return svg;
}

/* --- per-network attribution from the watched map ------------------------ */
/** Bytes per stripe from whichever block rows describe the job. Split out of the
 *  watch so a deck can be filled from the durable rows of a job the subscription
 *  has moved away from — its attribution does not stop existing. */
function shareWithin(blocks: Iterable<MapBlock>): Map<string, number> | null {
  const shares = new Map<string, number>();
  for (const block of blocks) {
    if (block.bytesDone > 0) {
      shares.set(block.networkId, (shares.get(block.networkId) ?? 0) + block.bytesDone);
    }
  }
  return shares.size > 0 ? shares : null;
}

function sharesOf(id: number): Map<string, number> | null {
  if (mapWatchId !== id || mapBlocks.size === 0) return null;
  return shareWithin(mapBlocks.values());
}

function netName(id: string, labels?: Map<string, string>): string {
  // n0 first: an attempt row whose adapter was never recorded carries the
  // engine's own fallback label, and "Default route" is not a cable.
  if (id === UNATTRIBUTED || !id) return 'Not attributed';
  // Rows built from a stored map bring the names their own attempts recorded:
  // the deck must say "Ethernet 2" for the same cable whether or not the watch
  // currently sits on this job, and whether or not that adapter is plugged in
  // today. It is the name of the bytes, not of the slot.
  const recorded = labels?.get(id);
  if (recorded) return recorded;
  for (const attempt of mapAttempts.values()) {
    if (attempt.networkId === id) return attempt.networkLabel;
  }
  // The row's own colour is the truth: the list order can shift when a device
  // is unplugged, and a stripe must not change owner mid-download. So there is
  // no positional guess here — a colour the live list does not carry belongs to
  // an adapter that is gone, and naming it by index would hand its bytes to
  // whichever cable happens to sit at that slot today.
  const byColor = networks.find((n) => n.colorId === id);
  if (byColor) return byColor.displayName;
  return `Offline (${id.toUpperCase()})`;
}

/* --- route rates, measured from the watcher's ticks ---------------------- */

/** A route is a cable the engine was allowed to use, not only a cable that has
 *  already finished a block, so the deck needs a rate per stripe. */
interface RouteRow {
  net: string;
  label: string;
  bytes: number;
  rate: number | null;
}

const RATE_WINDOW_MS = 2000;

/** A span shorter than this is two ticks and reads as noise, so the deck shows
 *  a dash until the window is old enough to divide by. */
const RATE_MIN_SPAN_MS = 750;

/** Cumulative bytes per stripe, kept for the last window. The engine never
 *  publishes a per-route rate, but every 250 ms snapshot carries cumulative
 *  bytes per stripe, and the difference over a known span is a measured number. */
const rateSamples: { at: number; bytes: Map<string, number> }[] = [];
let rateSamplesFor: number | null = null;

function stripeBytesNow(): Map<string, number> {
  const bytes = new Map<string, number>();
  for (const block of mapBlocks.values()) {
    if (block.bytesDone > 0) {
      bytes.set(block.networkId, (bytes.get(block.networkId) ?? 0) + block.bytesDone);
    }
  }
  return bytes;
}

function sampleRoutes(at: number): void {
  if (rateSamplesFor !== mapWatchId) {
    rateSamplesFor = mapWatchId;
    rateSamples.length = 0;
  }
  rateSamples.push({ at, bytes: stripeBytesNow() });
  while (rateSamples.length > 2 && at - (rateSamples[0]?.at ?? at) > RATE_WINDOW_MS) {
    rateSamples.shift();
  }
}

function routeRate(net: string): number | null {
  const first = rateSamples[0];
  const last = rateSamples[rateSamples.length - 1];
  if (!first || !last) return null;
  const span = last.at - first.at;
  if (span < RATE_MIN_SPAN_MS) return null;
  const delta = (last.bytes.get(net) ?? 0) - (first.bytes.get(net) ?? 0);
  return delta > 0 ? (delta * 1000) / span : 0;
}

/** The names a map recorded for its stripes. Attempts are the canonical record
 *  of who carried what, and stored blocks carry the same name for the same join,
 *  so a cable whose attempts fell out of the recent list still gets named. */
function labelsFrom(
  entries: Iterable<{ networkId: string; networkLabel?: string | null }>,
): Map<string, string> {
  const labels = new Map<string, string>();
  for (const entry of entries) {
    if (entry.networkLabel && !labels.has(entry.networkId)) {
      labels.set(entry.networkId, entry.networkLabel);
    }
  }
  return labels;
}

/** Every stripe this job has carried bytes on, plus every enabled adapter it
 *  was handed but has not used yet. A bond of two cables must not read as one
 *  cable because the second happens to be between blocks.
 *
 *  `live` is decided by the caller, not read from the status: only a deck that
 *  owns the subscription has samples that describe its own job, and a rate
 *  borrowed from another download would be the worst kind of wrong number. */
function routeRowsFrom(
  shares: Map<string, number>,
  live: boolean,
  labels?: Map<string, string>,
): RouteRow[] {
  const rows: RouteRow[] = [...shares].map(([net, bytes]) => ({
    net,
    label: netName(net, labels),
    bytes,
    rate: live ? routeRate(net) : null,
  }));
  for (const net of networks) {
    const color = net.colorId;
    if (!color || !net.enabled || color === UNATTRIBUTED || shares.has(color)) continue;
    rows.push({ net: color, label: netName(color), bytes: 0, rate: live ? 0 : null });
  }
  rows.sort((a, b) => b.bytes - a.bytes || a.label.localeCompare(b.label));
  return rows;
}

function routeRows(id: number): RouteRow[] | null {
  const shares = sharesOf(id);
  if (!shares) return null;
  return routeRowsFrom(
    shares,
    mapStatus === 'downloading',
    labelsFrom([...mapAttempts.values(), ...mapBlocks.values()]),
  );
}

function rateText(rate: number | null): string {
  if (rate === null) return '—';
  return rate > 0 ? formatRate(rate) : 'idle';
}

/** The inspector's routes deck. The engine emits a map tick while a job is
 *  watched, and a rate older than the last selection would be a frozen number
 *  presented as live, so the tick redraws these rows in place. When the watch
 *  moves to another job the deck stops guessing: it reads that job's stored
 *  rows, keeps the attribution, and drops the rates to dashes and says so. What
 *  it never does is blank a deck the user is reading. */
let routesDeck: { id: number; total: HTMLElement; body: HTMLElement; last: RouteRow[] | null } |
  null = null;

function paintRoutes(live: boolean, rows: RouteRow[] | null): void {
  const deck = routesDeck;
  if (!deck) return;
  const { total, body } = deck;
  body.replaceChildren();
  if (!rows) {
    total.textContent = '';
    const empty = document.createElement('p');
    empty.className = 'pane-empty';
    empty.style.marginTop = '6px';
    empty.textContent =
      networks.length > 0
        ? 'No chunk attributed yet — the deck fills once bytes land.'
        : 'No chunk attributed, and no adapter is enumerated.';
    body.append(empty);
    return;
  }
  const carried = rows.reduce((sum, row) => sum + row.bytes, 0);
  // The denominator has always been every byte the job has, so a header that
  // called all of it "attributed" was untrue for any job with an unattributed
  // stripe in it — which a proxied download now is from the first byte. Say how
  // much is credited instead of claiming the whole pile is.
  const credited = rows.reduce(
    (sum, row) => sum + (row.net === UNATTRIBUTED ? 0 : row.bytes),
    0,
  );
  const head =
    credited === carried
      ? `of ${formatBytes(carried)} attributed`
      : `${formatBytes(credited)} of ${formatBytes(carried)} attributed`;
  total.textContent = live ? head : `${head} · not measured`;
  const list = document.createElement('div');
  list.className = 'attrib';
  list.style.marginTop = '8px';
  for (const row of rows) {
    const el = document.createElement('div');
    el.className = 'attrib-row';
    el.dataset['net'] = row.net;
    const dot = document.createElement('i');
    dot.className = 'dot';
    const nm = document.createElement('span');
    nm.className = 'nm';
    nm.textContent = row.label;
    nm.title = row.label;
    const val = document.createElement('span');
    val.className = 'val';
    val.textContent = live ? rateText(row.rate) : '—';
    const sub = document.createElement('span');
    sub.className = 'sub';
    sub.textContent =
      row.bytes > 0
        ? `${formatBytes(row.bytes)} · ${Math.round((row.bytes / carried) * 100)}%`
        : 'enabled · no bytes on this job yet';
    el.append(dot, nm, val, sub);
    list.append(el);
  }
  body.append(list);
}

async function fillRoutesDeck(id: number): Promise<void> {
  const deck = routesDeck;
  if (!deck || deck.id !== id) return;
  const watched = mapWatchId === id;
  const live = watched && mapStatus === 'downloading';
  let rows = watched ? routeRows(id) : null;
  if (!rows && !watched) {
    // The subscription moved to a job the user started while this deck was open.
    // Ask for the stored rows instead of announcing that nothing is attributed:
    // the bytes are on disk and their cables are named, the watch is just busy.
    const map = await sucr.download.map(id).catch(() => null);
    if (routesDeck !== deck || routesDeck.id !== id) return;
    if (map) {
      const shares = shareWithin(map.blocks);
      if (shares) rows = routeRowsFrom(shares, false, labelsFrom([...map.attempts, ...map.blocks]));
    }
  }
  if (rows) deck.last = rows;
  // Rows taken from the cache are not being sampled any more, so they never get
  // to claim a measurement in the denominator.
  paintRoutes(rows !== null && live, rows ?? deck.last);
}

function refreshRoutesDeck(): void {
  if (routesDeck) void fillRoutesDeck(routesDeck.id);
}

/* --- hero progress: segmented bar --------------------------------------- */
function telemetryBar(d: Download): HTMLElement {
  const pct = pctOf(d);
  const wrap = document.createElement('div');
  wrap.className = 'tbar';
  wrap.dataset['state'] = d.status;
  if (d.status === 'downloading' && pct > 0 && pct < 100) wrap.dataset['live'] = 'true';

  const track = document.createElement('div');
  track.className = 'tbar-track';
  const fill = document.createElement('div');
  fill.className = 'tbar-fill';
  fill.style.width = `${pct}%`;

  const shares = d.status === 'downloading' ? sharesOf(d.id) : null;
  if (shares) {
    const total = [...shares.values()].reduce((a, b) => a + b, 0);
    for (const [net, bytes] of shares) {
      const band = document.createElement('i');
      band.className = 'tbar-band';
      band.dataset['net'] = net;
      band.style.width = `${total > 0 ? (bytes / total) * 100 : 100}%`;
      fill.append(band);
    }
  } else if (d.status === 'downloading') {
    // No attribution yet (map not watched): one unlabelled band. Non-running
    // states deliberately keep the fill bare so their state colour shows.
    const band = document.createElement('i');
    band.className = 'tbar-band';
    band.dataset['net'] = 'unknown';
    band.style.width = '100%';
    fill.append(band);
  }

  track.append(fill);
  const frontier = document.createElement('div');
  frontier.className = 'tbar-frontier';
  frontier.style.left = `${pct}%`;
  wrap.append(track, frontier);
  return wrap;
}

/* --- hero progress: aggregate wave -------------------------------------- */
function cssPx(name: string, fallback: number): number {
  const v = Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name));
  return Number.isFinite(v) ? v : fallback;
}

/** Geometry comes from the design tokens so the token file drives the render. Travel is
 *  deliberately not in here: it is the one geometry the motion switch rewrites, so it is
 *  read through `waveTravel()` instead of once, at module load. */
const WAVE = {
  amp: cssPx('--wave-amp', 3),
  len: cssPx('--wave-len', 20),
  h: cssPx('--wave-h', 18),
  // Frontier marker: a level capsule, not a dot riding the crest.
  pillW: 22,
  pillH: 10,
};

/** Seconds per wavelength, or `0` for "the crest does not travel". Both reduced blocks
 *  pin `--wave-travel` to `0s` (`tokens.css:268`, `:272`), and that `0` is the answer, so
 *  it survives: `drawWave` skips the phase rather than the token substituting a fallback
 *  speed nothing could override. Cached because a `getComputedStyle` read every frame
 *  would flush styles inside the animation loop, and `applyMotion` is the only writer. */
function waveTravel(): number {
  if (waveTravelDirty) {
    waveTravelDirty = false;
    waveTravelS = Math.max(0, cssPx('--wave-travel', 1));
  }
  return waveTravelS;
}

/** One wave node per download: rebuilding it every refresh would restart the
 *  eased frontier and make the crest stutter. */
const waveNodes = new Map<number, SVGSVGElement>();

const waveResizeObserver = new ResizeObserver((entries) => {
  for (const entry of entries) {
    const target = entry.target;
    if (target instanceof SVGSVGElement && target.isConnected) {
      drawWave(target, performance.now());
    }
  }
});

function waveFor(d: Download): SVGSVGElement {
  const baseline = WAVE.h / 2;
  let svg = waveNodes.get(d.id);
  if (!svg) {
    svg = document.createElementNS(NS, 'svg');
    svg.classList.add('hero-wave');
    svg.setAttribute('aria-hidden', 'true');

    const track = document.createElementNS(NS, 'line');
    track.setAttribute('class', 'wave-track');
    track.setAttribute('y1', String(baseline));
    track.setAttribute('y2', String(baseline));
    track.setAttribute('x1', '0');
    track.setAttribute('x2', '100%');

    const path = document.createElementNS(NS, 'path');
    path.setAttribute('class', 'wave-path');

    const pill = document.createElementNS(NS, 'rect');
    pill.setAttribute('class', 'wave-thumb');
    pill.setAttribute('width', String(WAVE.pillW));
    pill.setAttribute('height', String(WAVE.pillH));
    pill.setAttribute('rx', String(WAVE.pillH / 2));
    pill.setAttribute('y', String(baseline - WAVE.pillH / 2));
    pill.setAttribute('x', '0');

    svg.append(track, path, pill);
    waveNodes.set(d.id, svg);
  }
  waveResizeObserver.observe(svg);
  svg.dataset['state'] = d.status;
  const pct = pctExact(d);
  svg.dataset['pct'] = pct.toFixed(2);
  sampleWave(svg, pct);
  return svg;
}

type WaveSample = { t: number; pct: number };

/** Progress is pushed, not streamed: refreshes land every ~150 ms and the value
 *  jumps in steps. The wave renders a point slightly in the past and
 *  interpolates between the two samples around it, so the marker moves at the
 *  rate the transfer actually has instead of lurching sample to sample. */
const WAVE_LAG_MS = 220;
const waveSamples = new WeakMap<SVGSVGElement, WaveSample[]>();

function sampleWave(svg: SVGSVGElement, pct: number): void {
  const now = performance.now();
  const buf = waveSamples.get(svg);
  if (!buf || (buf.length > 0 && pct < (buf[buf.length - 1]?.pct ?? 0))) {
    waveSamples.set(svg, [{ t: now, pct }]);
    return;
  }
  const last = buf[buf.length - 1];
  if (last && now - last.t < 40) {
    last.t = now;
    last.pct = pct;
  } else {
    buf.push({ t: now, pct });
    if (buf.length > 16) buf.splice(0, buf.length - 16);
  }
}

function easedPct(svg: SVGSVGElement, now: number): number {
  const buf = waveSamples.get(svg);
  if (!buf || buf.length === 0) return Number(svg.dataset['pct'] ?? 0) / 100;
  const at = now - WAVE_LAG_MS;
  let shown = buf[0]?.pct ?? 0;
  for (let i = 1; i < buf.length; i += 1) {
    const a = buf[i - 1];
    const b = buf[i];
    if (!a || !b) continue;
    if (b.t >= at) {
      const span = b.t - a.t;
      const k = span > 0 ? Math.max(0, Math.min(1, (at - a.t) / span)) : 1;
      shown = a.pct + (b.pct - a.pct) * k;
      break;
    }
    shown = b.pct;
  }
  return shown / 100;
}

function drawWave(svg: SVGSVGElement, now: number): number | null {
  const path = svg.querySelector<SVGPathElement>('.wave-path');
  const track = svg.querySelector<SVGLineElement>('.wave-track');
  const pill = svg.querySelector<SVGRectElement>('.wave-thumb');
  if (!path || !track || !pill) return null;

  const width = svg.clientWidth;
  if (width <= 0) return null;

  const state = svg.dataset['state'] ?? '';
  const t = ambientClock(now);
  let shown: number;
  if (state === 'finished' || state === 'verified') {
    shown = 1.0;
  } else if (state === 'queued' && Number(svg.dataset['pct'] ?? 0) === 0) {
    shown = 0;
  } else if (state !== 'downloading') {
    shown = Math.max(0, Math.min(1, Number(svg.dataset['pct'] ?? 0) / 100));
  } else {
    shown = Math.max(0, Math.min(1, easedPct(svg, t)));
  }

  // Any live transfer waves, even at 1%: completion is the only flat state.
  const live = state === 'downloading' && shown < 1;
  const amp = live ? WAVE.amp : 0;
  const active = shown * width;
  const baseline = WAVE.h / 2;
  const travel = waveTravel();
  const phase = live && travel > 0 ? (t / 1000) * (Math.PI * 2 / travel) : 0;
  // The wave rises out of the origin instead of starting at full amplitude.
  const taper = WAVE.len * 0.6;
  const ampAt = (x: number): number => amp * Math.min(1, Math.max(0, x / taper));
  const waveY = (x: number): number =>
    baseline + Math.sin((x / WAVE.len) * Math.PI * 2 - phase) * ampAt(x);

  const step = Math.max(1, WAVE.len / 12);
  // The pill marks the frontier and the wave runs up to its left edge, so the
  // crest never pokes past the marker.
  const pillX = Math.max(0, Math.min(active - WAVE.pillW, width - WAVE.pillW));
  let d = `M 0 ${baseline}`;
  for (let x = step; x < pillX; x += step) {
    d += ` L ${x.toFixed(1)} ${waveY(x).toFixed(2)}`;
  }
  d += ` L ${pillX.toFixed(1)} ${waveY(pillX).toFixed(2)}`;
  path.setAttribute('d', d);
  track.setAttribute('x1', String(active));
  track.setAttribute('x2', String(width));
  pill.setAttribute('x', String(pillX));
  return shown;
}

let waveRaf: number | null = null;

/* One control on the card stops every ambient motion at once: the crest's frame loop,
   the bar's specular shimmer, and the active attempt dot. WCAG 2.2 SC 2.2.2 accepts a
   bounded run or a pause/stop/hide mechanism *on that screen*; the OS reduced-motion
   switch and `ui.meter_style` in Settings are both a system-wide toggle somewhere else,
   so neither one satisfies it. Session-only on purpose — a stored key would join the
   seventeen settings the release gate counts, and a pause that survives a restart would
   strand a fresh download silently. */
let ambientHeld = false;
let heldSince = 0;

/** The wave's own clock. While the card control holds it, wall time is not used at all, so
 *  every draw in the held frame — the ~150 ms refresh pass, a resize, the ResizeObserver —
 *  reproduces the same crest instead of advancing the phase. On release it returns to wall
 *  time: the transfer kept downloading while its animation was paused, so the frontier
 *  resumes at the true position rather than rewinding to catch up. */
function ambientClock(now: number): number {
  return ambientHeld ? heldSince : now;
}

/* The meter control lives on a node that is thrown away and rebuilt ~150 ms later, so keyboard
   focus has to be handed to the replacement or a second press would land on the body. */
let ambientFocusWanted = false;
document.addEventListener('focusin', (event) => {
  ambientFocusWanted = !!(event.target as HTMLElement | null)?.closest('.hero-progress');
});
// A press elsewhere means the user left the control, and no focus event fires when the focused
// node is simply discarded. Capture phase, because the card stops `pointerdown` propagation.
document.addEventListener('pointerdown', (event) => {
  if (ambientFocusWanted && !(event.target as Element | null)?.closest('.hero-progress')) {
    ambientFocusWanted = false;
  }
}, true);

function applyAmbient(): void {
  const root = document.documentElement;
  if (ambientHeld) {
    heldSince = performance.now();
    root.setAttribute('data-ambient', 'held');
  } else {
    heldSince = 0;
    root.removeAttribute('data-ambient');
  }
  syncWaveLoop();
}

/** Cards are rebuilt on every progress refresh, so this runs on a fresh node each time:
   the pressed state is re-read from the flag instead of needing a second pass to update
   an aria attribute that would otherwise go stale on the next repaint. */
function ambientControl(node: HTMLElement): void {
  node.setAttribute('role', 'button');
  node.setAttribute('tabindex', '0');
  node.setAttribute('aria-pressed', String(ambientHeld));
  node.setAttribute('aria-label', ambientHeld ? 'Resume progress animation' : 'Pause progress animation');
  // Without this the control is a keyboard trap of a different kind: the node a user tabbed
  // to is discarded ~150 ms later, so a second Enter would go to the body. Deferred because
  // this runs inside `cardFor`, while the node is still detached — `focus()` on a detached
  // element does nothing — and the microtask lands after `replaceChildren` has attached it.
  if (ambientFocusWanted) {
    queueMicrotask(() => {
      if (ambientFocusWanted && node.isConnected) node.focus({ preventScroll: true });
    });
  }
  const toggle = (): void => {
    ambientHeld = !ambientHeld;
    applyAmbient();
  };
  // `onTap`, not a click listener: the card cancels `pointerdown`, so a physical press never
  // produces a `click` here at all, and the row is replaced between down and up anyway.
  onTap(node, toggle);
  node.addEventListener('keydown', (event) => {
    const e = event as KeyboardEvent;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      toggle();
    }
  });
}

function waveTick(now: number): void {
  const svg = document.querySelector<SVGSVGElement>('.card--hero .hero-wave');
  // The loop stops itself while held rather than relying on the cancel in `syncWaveLoop`:
  // a frame already in flight when the toggle lands would otherwise re-arm it and the crest
  // would keep travelling through a paused control.
  if (!svg || progressMode !== 'wave' || ambientHeld) {
    waveRaf = null;
    return;
  }
  const shown = drawWave(svg, now);
  const target = Number(svg.dataset['pct'] ?? 0) / 100;
  // The render trails the pushed samples by WAVE_LAG_MS, so a transfer that
  // just stopped still has buffered distance to cover. Keep ticking until the
  // frontier reaches it; otherwise the next refresh restarts the loop.
  if (shown === null || (svg.dataset['state'] !== 'downloading' && Math.abs(target - shown) < 0.0005)) {
    waveRaf = null;
    return;
  }
  waveRaf = requestAnimationFrame(waveTick);
}

function syncWaveLoop(): void {
  // Cards are rebuilt on every progress refresh, so stale wave nodes have to
  // go: they are the ones no longer attached to the list. This runs before the
  // early return on purpose. A `ResizeObserver` keeps a strong reference to
  // everything it observes, so a node left observed after it detaches — the
  // switch to bars, or the last download leaving the list — can never be
  // collected, and it is exactly then that nobody is left to prune it.
  for (const [id, node] of waveNodes) {
    if (!node.isConnected) {
      waveResizeObserver.unobserve(node);
      waveNodes.delete(id);
    }
  }
  const hero = document.querySelector('.card--hero .hero-wave');
  if (progressMode !== 'wave' || !hero) {
    if (waveRaf !== null) cancelAnimationFrame(waveRaf);
    waveRaf = null;
    return;
  }
  // Ensure the hero wave is immediately drawn at its current client width
  if (hero instanceof SVGSVGElement) {
    drawWave(hero, performance.now());
  }
  // Held means held: the last draw leaves a flat-frontier wave sitting at the position
  // the pause happened, so the resting state still reads as a transfer in flight.
  if (ambientHeld) {
    if (waveRaf !== null) cancelAnimationFrame(waveRaf);
    waveRaf = null;
    return;
  }
  // The loop may still run with the crest frozen: either reduced trigger pins travel to
  // 0, and the frontier has to reach the distance the buffered samples hold, because that
  // movement is the transfer's progress rather than decoration. The card's own control is
  // the stop for the full-motion case.
  if (waveRaf === null) waveRaf = requestAnimationFrame(waveTick);
}

window.addEventListener('resize', () => {
  const hero = document.querySelector<SVGSVGElement>('.card--hero .hero-wave');
  if (hero && hero.isConnected) {
    drawWave(hero, performance.now());
  }
});

/* --- download cards ------------------------------------------------------ */
function heroMeta(d: Download): string {
  const bits = [formatBytes(d.totalBytes ?? d.verifiedBytes)];
  if (d.blockCount && d.blockSize) bits.push(`${d.blockCount} chunks × ${formatBytes(d.blockSize)}`);
  // A row that says `single` ran one connection whatever the tunable asked, so
  // the count the hero shows has to follow the recorded mode, not the setting.
  const width = d.streamsMode === 'single' ? 1 : d.streamsRequested;
  const top = d.streamsMode === 'single' ? width : d.streamsCeiling;
  // Two numbers describe how many streams actually ran: the live one while the
  // job is up, and the one the run left on the row once it is not. Either beats
  // restating a promise the job did not keep.
  const ran = d.streamsLive ?? d.streamsFinal;
  if (width) {
    if (width === 1) {
      bits.push('single stream');
    } else if (typeof top === 'number' && top > width) {
      // A band only says what the job may do; the engine also says what it is
      // doing, and that is the number worth the front of the card.
      bits.push(
        ran != null ? `${ran} of ${top} parallel streams` : `${width}–${top} parallel streams`,
      );
    } else {
      // A pin is a ceiling, not a floor: a 429 halves the gate below it and only
      // widens back up to the pin. So the number the engine runs can be smaller
      // than the number the dial promised, and then it is the honest one.
      bits.push(
        ran != null && ran < width
          ? `${ran} of ${width} parallel streams`
          : `${width} parallel streams`,
      );
    }
  }
  if (d.status !== 'downloading') bits.push(STATUS_LABEL[d.status] ?? d.status);
  if (d.errorCode) bits.push(d.errorCode);
  return bits.join(' · ');
}

function compactMeta(d: Download): string {
  const bits = [formatBytes(d.totalBytes ?? d.verifiedBytes)];
  if (d.totalBytes) bits.push(`${pctOf(d)}%`);
  bits.push(STATUS_LABEL[d.status] ?? d.status);
  return bits.join(' · ');
}

function miniBar(d: Download): HTMLElement {
  const wrap = document.createElement('div');
  wrap.className = 'card-mini';
  wrap.dataset['state'] = d.status;
  const track = document.createElement('div');
  track.className = 'mbar';
  const fill = document.createElement('i');
  fill.style.width = `${pctOf(d)}%`;
  track.append(fill);
  const pct = document.createElement('span');
  pct.className = 'pct';
  pct.textContent = d.totalBytes ? `${pctOf(d)}%` : formatBytes(d.verifiedBytes);
  wrap.append(track, pct);
  return wrap;
}

function heroFoot(d: Download): HTMLElement {
  const foot = document.createElement('div');
  foot.className = 'hero-foot';

  const stats = document.createElement('div');
  stats.className = 'hero-stats';
  const pct = document.createElement('b');
  pct.className = 'pct';
  pct.textContent = `${pctOf(d)}%`;
  const sep1 = document.createElement('span');
  sep1.className = 'sep';
  sep1.textContent = '·';
  const done = document.createElement('b');
  done.textContent = formatBytes(d.verifiedBytes);
  const of = document.createElement('span');
  of.className = 'opt';
  of.textContent = `of ${formatBytes(d.totalBytes)}`;
  stats.append(pct, sep1, done, of);
  const eta = etaOf(d);
  if (eta) {
    const sep2 = document.createElement('span');
    sep2.className = 'sep';
    sep2.textContent = '·';
    const etaEl = document.createElement('span');
    etaEl.className = 'eta';
    etaEl.textContent = eta;
    stats.append(sep2, etaEl);
  }

  const chips = document.createElement('div');
  chips.className = 'chips';
  const shares = d.status === 'downloading' ? sharesOf(d.id) : null;
  if (shares) {
    const total = [...shares.values()].reduce((a, b) => a + b, 0);
    for (const [net, bytes] of shares) {
      const chip = document.createElement('span');
      chip.className = 'chip';
      chip.dataset['net'] = net;
      const dot = document.createElement('i');
      dot.className = 'dot';
      chip.append(dot, document.createTextNode(`${netName(net)} ${Math.round((bytes / total) * 100)}%`));
      chips.append(chip);
    }
  }
  const stateChip = document.createElement('span');
  stateChip.className = 'chip';
  stateChip.dataset['tone'] = toneOf(d.status);
  stateChip.textContent =
    d.status === 'downloading' && d.totalBytes && d.verifiedBytes >= d.totalBytes
      ? 'Finishing'
      : STATUS_LABEL[d.status] ?? d.status;
  chips.append(stateChip);

  foot.append(stats, chips);
  return foot;
}

async function redownload(d: Download): Promise<void> {
  const oldSvg = waveNodes.get(d.id);
  if (oldSvg) {
    waveResizeObserver.unobserve(oldSvg);
    waveSamples.delete(oldSvg);
    waveNodes.delete(d.id);
  }

  // Re-fetch the original spec, not the redirect target: some CDNs only serve
  // the asset when the request looks like the page that linked it.
  const url = d.url;
  const filename = d.filename;
  const isSorting = localStorage.getItem('sucr.category_sorting') !== 'false';
  const folder = (isSorting ? resolveCategoryPath(url) : undefined) || d.folder || undefined;
  const checksumAlgo = (d.checksumAlgo as 'md5' | 'sha1' | 'sha256' | 'sha512' | null) ?? undefined;
  const checksumHex = d.checksumHex ?? undefined;
  const referer = d.referer ?? undefined;
  const streams = d.streams ?? undefined;

  // remove() stops the live job and waits for it to exit before clearing the
  // row, so the next add cannot reuse the id of a job that is still writing.
  // If it does not exit in time the old row stays, and adding now would put a
  // second job on the same file with the first one still running.
  const retired = await sucr.download.remove(d.id, true);
  if (!retired.removed) {
    showToast(stillStopping(d.filename));
    return;
  }
  const fresh = await sucr.download.add({
    url,
    filename,
    folder,
    checksumAlgo,
    checksumHex,
    referer,
    streams,
  });
  await sucr.download.start(fresh.id).catch(() => undefined);
  selectedId = fresh.id;
  await refresh();
}

/** Cards and the inspector are rebuilt on every progress refresh (~150 ms),
 *  so a physical click can land mouseup on a replacement node and never emit
 *  `click`. Act on pointerdown; keep click for keyboard activation only. */
function onTap(el: HTMLElement, run: () => void): void {
  let viaPointer = false;
  el.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    viaPointer = true;
    run();
  });
  el.addEventListener('click', (e) => {
    e.stopPropagation();
    if (viaPointer) {
      viaPointer = false;
      return;
    }
    run();
  });
}

function redownloadButton(d: Download, mini = false): HTMLButtonElement {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = mini ? 'btn-redownload btn-redownload-sm' : 'btn-redownload';
  btn.title = 'Redownload from beginning';
  const verdict = linkVerdicts.get(d.url);
  if (verdict && !verdict.ok) {
    btn.disabled = true;
    btn.title = `Source link is dead — ${verdictLine(verdict)}`;
  }
  btn.setAttribute('aria-label', `Redownload ${d.filename}`);
  btn.append(svgUse('#i-redo', 'icon-redo'));
  onTap(btn, () => void redownload(d));
  return btn;
}

function cardFor(d: Download, hero: boolean): HTMLElement {
  const card = document.createElement('div');
  card.className = hero ? 'card card--hero' : 'card';
  card.setAttribute('role', 'listitem');
  card.tabIndex = 0;
  card.dataset['id'] = String(d.id);
  card.setAttribute('aria-selected', String(d.id === selectedId));

  const main = document.createElement('div');
  main.className = 'card-main';

  const badge = fileBadge(d.filename);
  const tile = document.createElement('div');
  tile.className = 'tile';
  tile.dataset['cat'] = badge.cat;
  tile.textContent = badge.label;

  const idBox = document.createElement('div');
  idBox.className = 'card-id';
  const name = document.createElement('span');
  name.className = 'card-name';
  name.textContent = d.filename;
  name.title = d.filename;
  const meta = document.createElement('span');
  meta.className = 'card-meta';
  meta.textContent = hero ? heroMeta(d) : compactMeta(d);
  meta.title = d.errorCode ?? d.finalUrl ?? d.url;
  idBox.append(name, meta);

  const rate = document.createElement('div');
  rate.className = 'card-rate';
  const cap = effectiveCap(d);
  if (cap !== null && d.status !== 'finished' && d.status !== 'verified') {
    rate.append(capChip(cap));
  }
  const rateMain = document.createElement('span');
  rateMain.className = 'rate';
  rateMain.dataset['tone'] = toneOf(d.status);
  rateMain.textContent = statusRate(d);
  rate.append(rateMain);
  const eta = etaOf(d);
  if (eta && hero) {
    const etaEl = document.createElement('span');
    etaEl.className = 'eta';
    etaEl.textContent = eta;
    rate.append(etaEl);
  }

  if (hero) {
    rate.id = 'speedCapPill';
    speedTrigger(rate, { mode: 'download', id: d.id });
    main.append(tile, idBox, rate);
    const progressRow = document.createElement('div');
    progressRow.className = 'hero-progress-row';
    const progress = document.createElement('div');
    progress.className = 'hero-progress';
    progress.append(progressMode === 'wave' ? waveFor(d) : telemetryBar(d));
    ambientControl(progress);
    // The trigger lives on the hero card only: a paused or broken job becomes
    // the hero as soon as it is selected, and the compact rows have no room for
    // a second icon button.
    progressRow.append(progress, redownloadButton(d, false));
    if (isRefreshable(d)) progressRow.append(updateLinkButton(d, false));
    card.append(main, progressRow, heroFoot(d));
  } else {
    const side = document.createElement('div');
    side.className = 'card-side';
    side.append(miniBar(d), redownloadButton(d, true), rate);
    main.append(tile, idBox, side);
    card.append(main);
  }

  const select = (): void => selectDownload(d.id);
  onTap(card, select);
  card.addEventListener('keydown', (event) => {
    const e = event as KeyboardEvent;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      select();
    }
  });
  card.addEventListener('contextmenu', (event) => {
    const e = event as MouseEvent;
    e.preventDefault();
    e.stopPropagation();
    select();
    openContextMenu(d, e.clientX, e.clientY);
  });
  return card;
}

function renderRows(): void {
  const items = visibleDownloads();
  const isSettings = currentView === 'settings';
  const isNetworks = currentView === 'networks';
  settingsViewEl.hidden = !isSettings;
  networksViewEl.hidden = !isNetworks;
  // A hidden host measures 0 wide, so the curve is redrawn the moment the view
  // it lives in becomes visible rather than waiting for the next second.
  if (isNetworks) paintThroughput();
  rowsEl.hidden = isSettings || isNetworks;
  emptyEl.hidden = isSettings || isNetworks || items.length > 0;

  if (isSettings) {
    rowsEl.replaceChildren();
    syncWaveLoop();
    return;
  }

  if (isNetworks) {
    rowsEl.replaceChildren();
    syncWaveLoop();
    renderNetworksView();
    return;
  }

  if (query && items.length === 0) {
    emptyTitleEl.textContent = 'No matches';
    emptyBodyEl.textContent = `Nothing in this view matches “${query}”.`;
  } else {
    emptyTitleEl.textContent = 'No downloads yet';
    emptyBodyEl.textContent =
      'Paste a URL into New link. Ranges, resume, retries and checksums are handled by the Rust engine.';
  }

  const heroId = items.some((d) => d.id === selectedId) ? selectedId : items[0]?.id ?? null;
  rowsEl.replaceChildren(...items.map((d) => cardFor(d, d.id === heroId)));
  syncWaveLoop();
}

function updateCounts(): void {
  const counts: Record<string, number> = {
    all: library.length,
    downloading: 0,
    queued: 0,
    finished: 0,
    attention: 0,
  };
  for (const d of library) {
    if (d.status === 'downloading') counts['downloading'] = (counts['downloading'] ?? 0) + 1;
    else if (d.status === 'queued' || d.status === 'paused') counts['queued'] = (counts['queued'] ?? 0) + 1;
    else if (d.status === 'finished' || d.status === 'verified') counts['finished'] = (counts['finished'] ?? 0) + 1;
    else if (d.status === 'needs-attention') counts['attention'] = (counts['attention'] ?? 0) + 1;
  }
  for (const el of $$<HTMLElement>('[data-count]')) {
    const key = el.dataset['count'];
    if (key) el.textContent = String(counts[key] ?? 0);
  }
  const netCount = networks.length > 0 ? networks.length : (library.some((d) => d.status === 'downloading') || appInfo ? 1 : 0);
  $<HTMLElement>('#net-count').textContent = String(netCount);
  summaryEl.textContent = `${totals.active} active · ${formatRate(totals.bps)}`;
  renderNicDeck();
  renderCategoriesList();
  if (currentView === 'networks') renderNetworksView();
  // The usage figures the limit chips quote only move while something is moving.
  if (totals.active > 0) maybeRefreshNetworks();
}

/* --- Speed limit popover -------------------------------------------------- */
type SpeedTarget = { mode: 'download'; id: number } | { mode: 'global' };
let speedTarget: SpeedTarget | null = null;

const speedPopoverEl = $<HTMLElement>('#speed-popover');
const speedPopoverTitle = $<HTMLElement>('#speed-popover-title');
const speedCustomVal = $<HTMLInputElement>('#speed-custom-val');
const speedCustomUnit = $<HTMLSelectElement>('#speed-custom-unit');

function currentCapFor(target: SpeedTarget): number | null {
  if (target.mode === 'global') return globalCap;
  return library.find((d) => d.id === target.id)?.speedLimit ?? null;
}

function placePopover(popover: HTMLElement, anchor: HTMLElement): void {
  const r = anchor.getBoundingClientRect();
  const box = popover.getBoundingClientRect();
  const left = Math.min(Math.max(8, r.left), window.innerWidth - box.width - 8);
  const below = r.bottom + 6;
  const top = below + box.height > window.innerHeight - 8
    ? Math.max(8, r.top - box.height - 6)
    : below;
  popover.style.left = `${Math.round(left)}px`;
  popover.style.top = `${Math.round(top)}px`;
}

function openSpeedPopover(target: SpeedTarget, anchor: HTMLElement): void {
  speedTarget = target;
  speedPopoverTitle.textContent =
    target.mode === 'global' ? 'Global speed limit' : 'Download speed limit';
  const cap = currentCapFor(target);
  for (const btn of $$<HTMLButtonElement>('#speed-popover .preset-btn')) {
    btn.classList.toggle('is-current', Number(btn.dataset['bps']) === (cap ?? 0));
  }
  speedCustomVal.value = '';
  speedPopoverEl.hidden = false;
  placePopover(speedPopoverEl, anchor);
  $<HTMLButtonElement>('#speed-popover .preset-btn').focus();
}

function closeSpeedPopover(): void {
  speedPopoverEl.hidden = true;
  speedTarget = null;
}

async function applySpeed(bps: number | null): Promise<void> {
  const target = speedTarget;
  if (target === null) return;
  closeSpeedPopover();
  if (target.mode === 'global') {
    const info = await sucr.app.setSpeedLimit(bps);
    globalCap = info.globalSpeedLimit;
    renderSettings();
  } else {
    const updated = await sucr.download.setSpeedLimit(target.id, bps);
    const at = library.findIndex((d) => d.id === updated.id);
    if (at >= 0) library[at] = updated;
  }
  renderRows();
  updateCounts();
}

speedPopoverEl.addEventListener('click', (event) => {
  const el = event.target as Element;
  const preset = el.closest<HTMLButtonElement>('.preset-btn');
  if (preset) {
    const bps = Number(preset.dataset['bps']);
    void applySpeed(bps > 0 ? bps : null);
  } else if (el.closest('#speed-popover-close')) {
    closeSpeedPopover();
  } else if (el.closest('#speed-custom-apply')) {
    const value = Number(speedCustomVal.value);
    if (Number.isFinite(value) && value > 0) {
      void applySpeed(Math.round(value * Number(speedCustomUnit.value)));
    }
  }
});

document.addEventListener('pointerdown', (event) => {
  if (speedPopoverEl.hidden) return;
  const el = event.target;
  if (!(el instanceof Element)) return;
  if (speedPopoverEl.contains(el) || el.closest('.speed-trigger')) return;
  closeSpeedPopover();
});

/* --- remove-from-list prompt --------------------------------------------- */
const removePopoverEl = $<HTMLElement>('#remove-popover');
const removePopoverBody = $<HTMLElement>('#remove-popover-body');
const removePopoverFile = $<HTMLElement>('#remove-popover-file');
const removePopoverActions = $<HTMLElement>('#remove-popover-actions');
const REMOVE_PROMPT = removePopoverBody.textContent ?? '';
let removeTarget: Download | null = null;

function openRemovePopover(d: Download, anchor: HTMLElement): void {
  removeTarget = d;
  removePopoverBody.textContent = REMOVE_PROMPT;
  removePopoverActions.hidden = false;

  const name = document.createElement('b');
  name.textContent = d.filename;
  const size = document.createElement('span');
  size.textContent = formatBytes(d.totalBytes ?? d.verifiedBytes);
  removePopoverFile.replaceChildren(name, document.createTextNode(' · '), size);
  if (d.folder) {
    const where = document.createElement('span');
    where.textContent = d.folder;
    removePopoverFile.append(document.createElement('br'), where);
  }

  removePopoverEl.hidden = false;
  placePopover(removePopoverEl, anchor);
  $<HTMLButtonElement>('#remove-keep').focus();
}

function closeRemovePopover(): void {
  removePopoverEl.hidden = true;
  removeTarget = null;
}

/** A row is only given up once its job has retired. A worker still blocked in a
 *  read keeps the row, its partial file and every byte already transferred, so
 *  this says what did *not* happen instead of claiming the row is gone. */
const stillStopping = (name: string): string =>
  `${name} is still finishing its stop, so nothing was removed — try again in a moment.`;

/** Dropping the row always happens; unlinking the file may not, so the popover
 *  stays put to report a file that refused to go rather than claim success. */
async function confirmRemove(deleteFile: boolean): Promise<void> {
  const target = removeTarget;
  if (!target) {
    closeRemovePopover();
    return;
  }
  const outcome = await sucr.download.remove(target.id, deleteFile);
  if (selectedId === target.id) selectedId = null;
  await refresh();
  removeTarget = null;
  if (!outcome.removed) {
    removePopoverBody.textContent = stillStopping(target.filename);
    removePopoverActions.hidden = true;
    return;
  }
  if (deleteFile && outcome.filePresent && !outcome.fileDeleted) {
    removePopoverBody.textContent = `${target.filename} is still on disk — it is open elsewhere or protected.`;
    removePopoverActions.hidden = true;
    return;
  }
  closeRemovePopover();
}

removePopoverEl.addEventListener('click', (event) => {
  const el = event.target as Element;
  if (el.closest('#remove-popover-close')) closeRemovePopover();
  else if (el.closest('#remove-keep')) void confirmRemove(false);
  else if (el.closest('#remove-delete')) void confirmRemove(true);
});

document.addEventListener('pointerdown', (event) => {
  if (removePopoverEl.hidden) return;
  const el = event.target;
  if (!(el instanceof Element)) return;
  if (removePopoverEl.contains(el)) return;
  closeRemovePopover();
});

/* --- update-link prompt ---------------------------------------------------- */
const refreshPopoverEl = $<HTMLElement>('#refresh-url-popover');
const refreshUrlInput = $<HTMLInputElement>('#refresh-url-input');
const refreshUrlError = $<HTMLElement>('#refresh-url-error');
const refreshUrlSubmit = $<HTMLButtonElement>('#refresh-url-submit');
let refreshTarget: number | null = null;

/** Only a job that is stopped can be re-pointed: a running one has workers
 *  holding the old link, and swapping underneath them would split one file
 *  across two answers. The engine refuses the same thing; the button simply
 *  does not appear where the answer is always no. */
function isRefreshable(d: Download): boolean {
  return d.status === 'paused' || d.status === 'needs-attention';
}

function showRefreshError(message: string): void {
  refreshUrlError.classList.remove('is-hint');
  refreshUrlError.textContent = message;
  refreshUrlError.hidden = false;
}

function openRefreshPopover(d: Download, anchor: HTMLElement): void {
  refreshTarget = d.id;
  refreshUrlInput.value = '';
  clearVerdict(refreshCheck);
  refreshPopoverEl.hidden = false;
  placePopover(refreshPopoverEl, anchor);
  refreshUrlInput.focus();
}

function closeRefreshPopover(): void {
  refreshPopoverEl.hidden = true;
  refreshTarget = null;
}

/** The engine probes the new link before it writes anything, so this can take
 *  the full connect timeout. The button is held down for exactly that long —
 *  a second tap would start a second probe of the same link. */
async function applyRefresh(): Promise<void> {
  const id = refreshTarget;
  if (id === null) return;
  // Same rule as the add bar: Enter is not a way past a link the engine cannot
  // pull. The button state carries the verdict; this makes it binding.
  if (refreshUrlSubmit.disabled) return;
  const url = refreshUrlInput.value.trim();
  if (url === '') {
    showRefreshError('Paste a link first.');
    refreshUrlInput.focus();
    return;
  }
  refreshUrlSubmit.disabled = true;
  refreshUrlError.hidden = true;
  try {
    await sucr.download.refreshUrl(id, url);
    closeRefreshPopover();
    await refresh();
  } catch (err) {
    showRefreshError(err instanceof Error ? err.message : String(err));
    refreshUrlSubmit.disabled = false;
    refreshUrlInput.focus();
  }
}

function updateLinkButton(d: Download, mini = false): HTMLButtonElement {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = `btn-redownload btn-link-update${mini ? ' btn-redownload-sm' : ''}`;
  btn.title = 'Paste a fresh link for this file';
  btn.setAttribute('aria-label', `Update the link for ${d.filename}`);
  btn.append(svgUse('#i-link', 'icon-link'));
  onTap(btn, () => openRefreshPopover(d, btn));
  return btn;
}

refreshPopoverEl.addEventListener('click', (event) => {
  const el = event.target as Element;
  if (el.closest('#refresh-url-submit')) void applyRefresh();
  else if (el.closest('#refresh-url-close') || el.closest('#refresh-url-cancel')) {
    closeRefreshPopover();
  }
});

refreshUrlInput.addEventListener('keydown', (event) => {
  if ((event as KeyboardEvent).key === 'Enter') {
    event.preventDefault();
    void applyRefresh();
  }
});

document.addEventListener('pointerdown', (event) => {
  if (refreshPopoverEl.hidden) return;
  const el = event.target;
  if (!(el instanceof Element)) return;
  if (refreshPopoverEl.contains(el) || el.closest('.btn-link-update')) return;
  closeRefreshPopover();
});

/* --- clipboard ------------------------------------------------------------ */

/** Put text on the clipboard, answering yes-or-no instead of throwing.
 *
 *  The menu items had `.then(toast)` with no rejection path, so a denied write
 *  looked identical to a click that did nothing. Denied is reachable: the async
 *  clipboard needs focus and a user activation, and either can be missing.
 *  Measured here (`--smoke` reports it): the webview is a secure context and
 *  `navigator.clipboard.writeText` exists, so the throw below is a rejection,
 *  not a missing API — and a missing API lands in the same catch.
 */
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/* --- card context menu ---------------------------------------------------- */
const ctxMenu = $<HTMLElement>('#card-context-menu');
const ctxOpenBtn = $<HTMLButtonElement>('#ctx-open');
const ctxOpenLabel = $<HTMLElement>('#ctx-open-label');
const ctxRevealBtn = $<HTMLButtonElement>('#ctx-reveal');
const ctxCopyNameBtn = $<HTMLButtonElement>('#ctx-copy-name');
const ctxCopyNameLabel = $<HTMLElement>('#ctx-copy-name-label');
const ctxCopyLinkBtn = $<HTMLButtonElement>('#ctx-copy-link');
const ctxCategoryBtn = $<HTMLButtonElement>('#ctx-category');
const ctxCategoryValue = $<HTMLElement>('#ctx-category-value');
const ctxCategoryMenu = $<HTMLElement>('#ctx-category-menu');
const ctxHideBtn = $<HTMLButtonElement>('#ctx-hide');
const ctxDeleteBtn = $<HTMLButtonElement>('#ctx-delete');
const toastEl = $<HTMLElement>('#toast-notice');
let toastTimer: number | null = null;
let ctxTarget: Download | null = null;

function showToast(msg: string): void {
  toastEl.textContent = msg;
  toastEl.hidden = false;
  if (toastTimer !== null) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toastEl.hidden = true;
    toastTimer = null;
  }, 2200);
}

function closeCategoryFlyout(): void {
  ctxCategoryMenu.hidden = true;
  ctxCategoryBtn.setAttribute('aria-expanded', 'false');
}

function closeContextMenu(): void {
  closeCategoryFlyout();
  ctxMenu.hidden = true;
  ctxTarget = null;
}

function categoryName(d: Download): string | null {
  return categories.find((c) => c.id === d.categoryId)?.name ?? null;
}

/** The five seeded categories, plus the way back out of one.
 *
 *  Rebuilt on every open rather than cached: the rows are the table's, a
 *  re-categorised card changes which item is checked, and five buttons is not
 *  worth an invalidation rule. */
function buildCategoryFlyout(d: Download): void {
  const items: Array<{ id: number | null; label: string; current: boolean }> = categories.map(
    (c) => ({ id: c.id, label: c.name, current: c.id === d.categoryId }),
  );
  if (d.categoryId !== null) {
    items.push({ id: null, label: 'Uncategorized', current: false });
  }
  ctxCategoryMenu.replaceChildren(
    ...items.map((item) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'context-menu-item';
      button.setAttribute('role', 'menuitemradio');
      button.setAttribute('aria-checked', String(item.current));
      button.dataset['categoryId'] = item.id === null ? '' : String(item.id);
      const label = document.createElement('span');
      label.className = 'ctx-label';
      label.textContent = item.label;
      button.append(label);
      return button;
    }),
  );
}

function openCategoryFlyout(): void {
  const target = ctxTarget;
  if (!target) return;
  buildCategoryFlyout(target);
  ctxCategoryMenu.hidden = false;
  ctxCategoryBtn.setAttribute('aria-expanded', 'true');

  const width = ctxCategoryMenu.offsetWidth;
  const height = ctxCategoryMenu.offsetHeight;
  const menu = ctxMenu.getBoundingClientRect();
  // The card menu is already clamped inside the window, so it is the side with
  // no room that has to give: open left when the right edge is out of reach.
  ctxCategoryMenu.classList.toggle(
    'context-submenu--flip',
    width > window.innerWidth - 8 - (menu.right + 4),
  );

  // The flyout is taller than the menu it hangs off, so it rides up when the
  // bottom edge is short, and never past 8 px from the top.
  const top = Math.min(-4, Math.max(8 - menu.top, window.innerHeight - 8 - height - menu.top));
  ctxCategoryMenu.style.top = `${Math.round(top)}px`;
  ctxCategoryMenu.querySelector<HTMLButtonElement>('.context-menu-item')?.focus();
}

function openContextMenu(d: Download, clientX: number, clientY: number): void {
  closeRemovePopover();
  closeSpeedPopover();
  closeCategoryFlyout();
  ctxTarget = d;

  ctxOpenLabel.textContent = `Open ${d.filename}`;
  ctxCopyNameLabel.textContent = `Copy ${d.filename}`;
  ctxOpenBtn.title = `Open ${d.filename}`;
  ctxCopyNameBtn.title = `Copy ${d.filename}`;

  const isCompleted = d.status === 'finished' || d.status === 'verified';
  $<HTMLElement>('#ctx-group-actions').hidden = !isCompleted;
  $<HTMLElement>('#ctx-div-copy').hidden = !isCompleted;
  // The same rule the inspector already keeps: only a published job has a file
  // worth asking about. Before publish the bytes are staging, and unlinking them
  // is what "Hide from list" says it does — offering "Delete File" for a file
  // that does not exist yet promises a deletion the row cannot perform.
  $<HTMLElement>('#ctx-delete').hidden = !isCompleted;
  const hasCategories = categories.length > 0;
  $<HTMLElement>('#ctx-group-category').hidden = !hasCategories;
  $<HTMLElement>('#ctx-div-category').hidden = !hasCategories;
  ctxCategoryValue.textContent = hasCategories ? (categoryName(d) ?? 'Uncategorized') : '';

  ctxMenu.hidden = false;

  // Measured, not assumed: the menu is now tall enough for a submenu trigger and
  // its width follows the filename, so either constant could be wrong on its own
  // open. `offset` reads the layout box, which the opening scale animation skips.
  const menuWidth = ctxMenu.offsetWidth;
  const menuHeight = ctxMenu.offsetHeight;
  const padding = 8;
  let left = clientX;
  let top = clientY;

  if (left + menuWidth > window.innerWidth - padding) {
    left = window.innerWidth - menuWidth - padding;
  }
  if (top + menuHeight > window.innerHeight - padding) {
    top = window.innerHeight - menuHeight - padding;
  }
  left = Math.max(padding, left);
  top = Math.max(padding, top);

  ctxMenu.style.left = `${left}px`;
  ctxMenu.style.top = `${top}px`;
}

/* One direction only. A tap fires pointerenter before click, so a toggle here
   would open the list on the way in and close it again on the way out — the
   flyout hides with the menu, and Escape or a click outside is the way back. */
ctxCategoryBtn.addEventListener('click', () => {
  if (ctxCategoryMenu.hidden) openCategoryFlyout();
});

ctxCategoryBtn.addEventListener('pointerenter', () => {
  if (ctxCategoryMenu.hidden) openCategoryFlyout();
});

ctxCategoryMenu.addEventListener('click', (event) => {
  const button = (event.target as Element).closest<HTMLButtonElement>('.context-menu-item');
  if (!button) return;
  const target = ctxTarget;
  const categoryId = button.dataset['categoryId'];
  closeContextMenu();
  if (!target) return;
  void applyCategory(target, categoryId === '' ? null : Number(categoryId));
});

async function applyCategory(d: Download, categoryId: number | null): Promise<void> {
  let updated: Download;
  try {
    updated = await sucr.download.setCategory(d.id, categoryId);
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err));
    return;
  }
  const at = library.findIndex((row) => row.id === updated.id);
  if (at >= 0) library[at] = updated;
  renderRows();
  updateCounts();
  const name = categoryId === null ? null : categoryName(updated);
  showToast(name === null ? 'Uncategorized' : `Moved to ${name}`);
}

ctxOpenBtn.addEventListener('click', () => {
  const target = ctxTarget;
  closeContextMenu();
  if (!target) return;
  void (async () => {
    try {
      const res = await sucr.download.open(target.id);
      if (!res.ok) {
        showToast('File no longer found on disk');
      }
    } catch {
      showToast('Could not open file');
    }
  })();
});

ctxRevealBtn.addEventListener('click', () => {
  const target = ctxTarget;
  closeContextMenu();
  if (!target) return;
  void (async () => {
    try {
      const res = await sucr.download.reveal(target.id);
      if (!res.ok) {
        showToast('Could not open folder');
      }
    } catch {
      showToast('Could not open folder');
    }
  })();
});

ctxCopyNameBtn.addEventListener('click', () => {
  const target = ctxTarget;
  closeContextMenu();
  if (!target) return;
  void (async () => {
    const copied = await copyText(target.filename);
    showToast(copied ? `Copied: ${target.filename}` : 'Could not reach the clipboard.');
  })();
});

ctxCopyLinkBtn.addEventListener('click', () => {
  const target = ctxTarget;
  closeContextMenu();
  if (!target) return;
  const link = target.finalUrl || target.url;
  void (async () => {
    const copied = await copyText(link);
    showToast(copied ? 'Copied download link' : 'Could not reach the clipboard.');
  })();
});

ctxHideBtn.addEventListener('click', () => {
  const target = ctxTarget;
  closeContextMenu();
  if (!target) return;
  void (async () => {
    const outcome = await sucr.download.remove(target.id, false);
    if (selectedId === target.id) selectedId = null;
    await refresh();
    showToast(
      outcome.removed ? `Hidden from list: ${target.filename}` : stillStopping(target.filename),
    );
  })();
});

ctxDeleteBtn.addEventListener('click', () => {
  const target = ctxTarget;
  closeContextMenu();
  if (!target) return;
  const card = document.querySelector<HTMLElement>(`.card[data-id="${target.id}"]`) ?? document.body;
  openRemovePopover(target, card);
  $<HTMLButtonElement>('#remove-delete')?.focus();
});

document.addEventListener('pointerdown', (event) => {
  if (ctxMenu.hidden) return;
  const el = event.target;
  if (!(el instanceof Element)) return;
  if (ctxMenu.contains(el)) return;
  closeContextMenu();
});

document.addEventListener('keydown', (event) => {
  if (!ctxMenu.hidden && event.key === 'Escape') {
    closeContextMenu();
  }
});

window.addEventListener('resize', () => closeContextMenu());
window.addEventListener('scroll', () => closeContextMenu(), true);

/* --- settings stage: rendering, handlers, maintenance -------------------- */
/** The last reading of the log, kept so a repaint does not need a round trip. */
let debugReport: DebugReport | null = null;
let debugSeq = 0;

/** Read the log fresh. The card is only worth showing if its reading is newer
 *  than the last repaint: a cached tail after a crash is exactly the thing a
 *  user is looking for. */
async function refreshDebugLog(): Promise<void> {
  const seq = ++debugSeq;
  try {
    const report = (await sucr.debug.report('state')) as DebugReport;
    if (seq !== debugSeq) return;
    debugReport = report;
    renderDebugLog();
  } catch {
    // The card keeps its last reading; a log that cannot be read is itself
    // something the next refresh will show as an empty tail.
  }
}

function renderDebugLog(): void {
  const path = $<HTMLElement>('#set-debug-path');
  const tail = $<HTMLElement>('#set-debug-tail');
  const note = $<HTMLElement>('#set-debug-note');
  if (!debugReport) {
    path.textContent = appInfo?.paths.logsDir || '—';
    return;
  }
  path.textContent = debugReport.path;
  tail.textContent = debugReport.tail.trim() || 'Nothing logged yet.';
  tail.scrollTop = tail.scrollHeight;
  note.dataset['tone'] = '';
  note.textContent = debugReport.enabled
    ? `Writing. ${formatBytes(debugReport.bytes)} so far; the file rolls over at 1 MB and keeps one previous copy.`
    : 'Writing is off. The file keeps whatever it already has.';
}

function renderSettings(): void {
  for (const group of $$<HTMLElement>('.settings-body .seg-control[data-set]')) {
    const key = group.dataset['set'];
    if (!key) continue;
    const value = setting(key);
    for (const button of group.querySelectorAll<HTMLButtonElement>('button[data-val]')) {
      button.setAttribute('aria-pressed', String(button.dataset['val'] === value));
    }
  }
  for (const input of $$<HTMLInputElement>('.settings-body input[data-toggle]')) {
    const key = input.dataset['toggle'];
    if (key) input.checked = settingOn(key);
  }
  const folder = $<HTMLInputElement>('#set-folder');
  if (document.activeElement !== folder) folder.value = setting('storage.default_folder');
  folder.placeholder = appInfo?.paths.downloadsDir || 'Empty = app Downloads folder';
  $<HTMLElement>('#set-folder-note').textContent = setting('storage.default_folder')
    ? 'Saved in the database; new links start here.'
    : "Empty uses the app's Downloads folder.";
  const proxyUrl = $<HTMLInputElement>('#set-proxy-url');
  if (document.activeElement !== proxyUrl) proxyUrl.value = setting('network.proxy_url');
  const proxyBypass = $<HTMLInputElement>('#set-proxy-bypass');
  if (document.activeElement !== proxyBypass) proxyBypass.value = setting('network.proxy_bypass');
  const updateFeed = $<HTMLInputElement>('#set-update-feed');
  if (document.activeElement !== updateFeed) updateFeed.value = setting('update.feed_url');
  renderProxyVia();
  renderProxyAuth();
  renderDownloadWindow();
  $<HTMLElement>('#set-cap-readout').textContent =
    globalCap === null ? 'Unlimited' : formatRate(globalCap);
  $<HTMLElement>('#set-close-note').textContent = settingOn('ui.close_to_tray')
    ? 'Closing hides Sucr in the notification area; transfers keep running.'
    : 'Stored now; closing the window quits the app.';
  // The switch is only worth having if its row says what it currently means —
  // System resolves against Windows, which on this machine asks for stillness.
  $<HTMLElement>('#set-motion-note').textContent = motionPref === 'full'
    ? 'Sucr animates even where Windows asks for stillness.'
    : motionPref === 'reduced'
      ? 'Sucr keeps still even where Windows allows motion.'
      : `Windows ${motionQuery.matches ? 'asks for stillness' : 'allows motion'}; Sucr follows it.`;
  renderSessions();
  renderOrigins();
  renderDebugLog();
}

/** One line, three readings, each in the engine's own words.
 *
 *  A mode that resolved to nothing arrives as `manual direct` or `system
 *  direct`, so the line names the misconfiguration instead of the label hiding
 *  it behind the word "direct". The bypass clause is only true because the proxy
 *  connector consults the bypass before it opens a proxy socket, which is what
 *  the step 0 harness measured. */
function renderProxyVia(): void {
  const note = $<HTMLElement>('#set-proxy-via');
  const via = appInfo?.proxyVia ?? '';
  note.dataset['tone'] = '';
  if (!via) {
    note.textContent = 'Egress: not read yet.';
  } else if (via === 'direct') {
    note.textContent = 'Egress: direct. Every link leaves on the adapter the bond picks.';
  } else if (via.endsWith(' direct')) {
    note.textContent = `Egress: ${via}. That mode found nothing to use, so links leave on the route Windows picks.`;
  } else {
    note.textContent = `Egress: ${via}. A proxied link runs on the route Windows picks: bonding is off for it and no adapter is credited. A host on the bypass list keeps its bond.`;
  }
}

/** One row's worth of truth about the proxy credential, in three readings.
 *
 *  `applies` comes from the engine rather than from comparing these two boxes here:
 *  the answer to "is this pair the one about to be sent" belongs to the layer that
 *  sends it, so an address edited after the credential lands shows as stranded
 *  instead of working. The password is never in the page, so the line says a
 *  credential exists and whose it is, and the box below is a replacement field. */
function renderProxyAuth(): void {
  const note = $<HTMLElement>('#set-proxy-auth');
  const user = $<HTMLInputElement>('#set-proxy-user');
  const pass = $<HTMLInputElement>('#set-proxy-pass');
  const auth = appInfo?.proxyAuth;
  note.dataset['tone'] = '';
  if (!auth) {
    note.textContent = 'Credential: not read yet.';
    return;
  }
  if (document.activeElement !== user) user.value = auth.username ?? '';
  pass.placeholder = auth.stored ? 'stored; type to replace' : 'password';
  if (!auth.stored) {
    note.textContent = 'No credential stored. A proxy that asks for one refuses the link.';
    return;
  }
  const named = `${auth.username} for ${auth.host}`;
  note.textContent = auth.applies
    ? `Credential: ${named} is in use.`
    : setting('network.proxy_mode') === 'manual'
      ? `Credential: ${named} is stored, not in use — the address no longer names that proxy.`
      : `Credential: ${named} is stored, not in use — only a manual proxy takes a typed credential.`;
}

function setPillar(name: string, scroll: boolean): void {
  for (const button of $$<HTMLElement>('.settings-nav button[data-pillar]')) {
    button.setAttribute('aria-current', String(button.dataset['pillar'] === name));
  }
  if (scroll) {
    $(`#pillar-${name}`).scrollIntoView({
      block: 'start',
      behavior: motionReduced() ? 'auto' : 'smooth',
    });
  }
}

$('.settings-nav').addEventListener('click', (event) => {
  const button = (event.target as Element).closest<HTMLButtonElement>('button[data-pillar]');
  const pillar = button?.dataset['pillar'];
  if (button && pillar) {
    button.classList.remove('animating');
    void button.offsetWidth;
    button.classList.add('animating');
    window.setTimeout(() => button.classList.remove('animating'), 360);
    setPillar(pillar, true);
  }
});

function commitFolder(): void {
  const value = $<HTMLInputElement>('#set-folder').value.trim();
  if (value === setting('storage.default_folder')) return;
  void saveSetting('storage.default_folder', value === '' ? null : value).catch(reportSettingError);
}

/** Both proxy boxes, because both are strings the engine has to be able to read.
 *  A refusal lands on this card's own line, not on the System pillar's note the
 *  generic handler would use: the user is looking at the box they just typed in. */
function commitProxyField(id: string, key: string): void {
  const input = $<HTMLInputElement>(id);
  const value = input.value.trim();
  if (value === setting(key)) return;
  void saveSetting(key, value === '' ? null : value).catch((err) => {
    const note = $<HTMLElement>('#set-proxy-via');
    note.dataset['tone'] = 'error';
    note.textContent = `Not saved: ${err instanceof Error ? err.message : String(err)}`;
  });
}

/** A window time, committed exactly as the proxy boxes are: one string the engine's
 *  own parser has to be able to read, refused back onto this row's note instead of
 *  stored as something that resolves to midnight. `<input type="time">` gives back
 *  `HH:MM` or nothing, which is the shape `engine.window_*` stores.
 */
function commitWindowTime(id: string, key: string): void {
  const input = $<HTMLInputElement>(id);
  const value = input.value.trim();
  if (value === setting(key)) return;
  void saveSetting(key, value === '' ? null : value).catch((err) => {
    $<HTMLElement>('#set-window-note').textContent =
      `Not saved: ${err instanceof Error ? err.message : String(err)}`;
  });
}

const DAY_NAMES = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

// Masked to seven bits on the way in, so a hand-edited row cannot light a day that
// does not exist.
function windowDayMask(): number {
  const stored = Number.parseInt(setting('engine.window_days'), 10);
  return Number.isFinite(stored) ? stored & 0x7f : 0;
}

/** `0` is not "no days", it is every day — so clearing the last chip clears the key
 *  rather than storing a zero that would open the window on no day at all.
 */
function toggleWindowDay(bit: number): void {
  if (!(bit >= 0 && bit < 7)) return;
  const mask = windowDayMask();
  const next = mask & (1 << bit) ? mask & ~(1 << bit) : mask | (1 << bit);
  void saveSetting('engine.window_days', next === 0 ? null : String(next))
    .catch(reportSettingError);
}

/** The window row and the Schedule pane both say what the four stored keys mean,
 *  so they say it from here and cannot drift.
 *
 *  Deliberately not a "closed right now" verdict: that would be the engine's
 *  minute-and-weekday judgment written a second time in here, and the app already
 *  refuses a second parser for the proxy URL for exactly that reason. The sentence
 *  is the rule; the queue is the evidence.
 *
 *  It does read `ui.close_to_tray`, because that setting can cancel the rule: with
 *  the close action on Quit, closing the last window ends the process, and a stored
 *  hour cannot open a scheduler that is not running. Saying the hours and staying
 *  silent about that is a sentence that is true only half the time. */
function windowRuleText(): string {
  if (!settingOn('engine.window_enabled')) return 'Off: links start as soon as they are added.';
  const mask = windowDayMask();
  const opens = setting('engine.window_start') || '00:00';
  const closes = setting('engine.window_stop') || '00:00';
  const named = DAY_NAMES.filter((_, index) => (mask & (1 << index)) !== 0);
  const days = named.length === 0 ? 'every day' : named.join(', ');
  const running = settingOn('ui.close_to_tray')
    ? 'Nothing wakes the machine: it has to be running.'
    : 'Nothing wakes the machine: it has to be running, and Window close action is set to Quit '
      + '— the app ends when you close its last window, so these hours never open.';
  return `Queued links start between ${opens} and ${closes}, ${days}. `
    + 'Outside them a link you add stays Queued and starts on its own. '
    + running;
}

function renderDownloadWindow(): void {
  const start = $<HTMLInputElement>('#set-window-start');
  if (document.activeElement !== start) start.value = setting('engine.window_start');
  const stop = $<HTMLInputElement>('#set-window-stop');
  if (document.activeElement !== stop) stop.value = setting('engine.window_stop');
  const mask = windowDayMask();
  for (const button of $$<HTMLButtonElement>('#set-window-days button[data-day]')) {
    const bit = Number(button.dataset['day']);
    button.setAttribute('aria-pressed', String((mask & (1 << bit)) !== 0));
  }
  $<HTMLElement>('#set-window-note').textContent = windowRuleText();
}

/** The credential commits as one pair, because it is sent as one field: ureq writes
 *  it into the tunnel request before the proxy gets a chance to ask, so there is no
 *  round left to answer and half a pair cannot work.
 *
 *  That is also why an empty password box is not "keep the stored one". The page
 *  never holds the secret, so the only thing a commit can mean is the text in front
 *  of it. Both boxes empty is how the credential is forgotten. A refusal keeps what
 *  was typed — including the password — so the fix is one keystroke, and the engine
 *  never saw the pair. */
function commitProxyAuth(): void {
  const user = $<HTMLInputElement>('#set-proxy-user');
  const pass = $<HTMLInputElement>('#set-proxy-pass');
  const username = user.value.trim();
  const password = pass.value;
  if (username === (appInfo?.proxyAuth.username ?? '') && password === '') return;
  void sucr.proxy
    .setAuth(username, password)
    .then(() => {
      pass.value = '';
      return syncProxyVia();
    })
    .catch((err) => {
      const note = $<HTMLElement>('#set-proxy-auth');
      note.dataset['tone'] = 'error';
      note.textContent = `Not saved: ${err instanceof Error ? err.message : String(err)}`;
    });
}

async function copyDbPath(): Promise<void> {
  const path = appInfo?.paths.dbPath ?? '';
  if (!path) return;
  try {
    await navigator.clipboard.writeText(path);
  } catch {
    const area = document.createElement('textarea');
    area.value = path;
    document.body.append(area);
    area.select();
    document.execCommand('copy');
    area.remove();
  }
  const button = $<HTMLButtonElement>('#set-copy-path');
  const label = button.textContent;
  button.textContent = 'Copied';
  window.setTimeout(() => {
    button.textContent = label ?? 'Copy path';
  }, 1200);
}

async function clearFinished(): Promise<void> {
  const note = $<HTMLElement>('#set-system-note');
  const done = library.filter((d) => d.status === 'finished' || d.status === 'verified');
  for (const d of done) await sucr.download.remove(d.id, false);
  if (selectedId !== null && done.some((d) => d.id === selectedId)) selectedId = null;
  await refresh();
  note.dataset['tone'] = '';
  note.textContent = done.length === 0
    ? 'Nothing finished to clear; files on disk are kept.'
    : `Cleared ${done.length} finished row${done.length === 1 ? '' : 's'}; files on disk are kept.`;
}

async function resetAllSettings(): Promise<void> {
  // The credential stores are not setting keys, and that is deliberate — a password
  // inside `settings` would cross the bridge on every Settings open — so nulling the
  // keys alone left three kinds of secret behind. A clean slate means them too. Both
  // already have a forget: an empty pair drops the proxy's, and one empty pair per host
  // drops the origin ones, which is why the hosts are read before anything is written.
  const hosts = (appInfo?.originAuths ?? []).map((o) => o.host);
  for (const key of Object.keys(storedSettings)) await sucr.settings.set(key, null);
  await sucr.proxy.setAuth('', '');
  for (const host of hosts) await sucr.origin.setAuth(host, '', '');
  storedSettings = {};
  settings = { ...SETTING_DEFAULTS };
  appInfo = await sucr.app.getInfo();
  globalCap = (await sucr.app.setSpeedLimit(null)).globalSpeedLimit;
  pref = 'system';
  progressMode = 'segmented';
  motionPref = 'full';
  applyTheme();
  applyProgressMode();
  applyMotion();
  updateCounts();
  renderSettings();
  $<HTMLElement>('#set-system-note').dataset['tone'] = '';
  $<HTMLElement>('#set-system-note').textContent =
    'All preferences and stored credentials reset to defaults.';
}

/* --- browser sessions: the rows, the paste form, the refusal bridge --------- */
/* Only the summaries cross the bridge: host, user agent, how many bytes of
   cookie, when. The value itself is written once and never read back, so a
   secret does not have to sit in the page's memory to be managed. */
let sessions: BrowserSession[] = [];

async function loadSessions(): Promise<void> {
  try {
    sessions = await sucr.session.list();
  } catch (err) {
    console.error(err);
  }
}

/** The identity a session is stored under, mirroring `engine::host_key`: no
 *  scheme, no port, lowercased. A download URL always parses; a bare host typed
 *  into the box is sent to the engine as it stands, which reduces it too. */
function hostKey(link: string): string {
  try {
    return new URL(link).hostname.toLowerCase();
  } catch {
    return '';
  }
}

function sessionFor(link: string): BrowserSession | undefined {
  const host = hostKey(link);
  return host === '' ? undefined : sessions.find((s) => s.host === host);
}

function sessionNote(): HTMLElement {
  return $<HTMLElement>('#session-note');
}

function renderSessions(): void {
  const rows = $<HTMLElement>('#session-rows');
  if (sessions.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'session-row';
    empty.textContent = 'No sessions stored.';
    rows.replaceChildren(empty);
    return;
  }
  rows.replaceChildren(
    ...sessions.map((s) => {
      const row = document.createElement('div');
      row.className = 'session-row';
      const host = document.createElement('b');
      host.textContent = s.host;
      host.title = `Saved ${when(s.createdAt)}`;
      const meta = document.createElement('span');
      meta.textContent = `${formatBytes(s.cookieBytes)} cookie · ${
        s.userAgent ? 'its own user agent' : 'the app user agent'
      }`;
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'ghost';
      button.dataset['forget'] = s.host;
      button.textContent = 'Forget';
      row.append(host, meta, button);
      return row;
    }),
  );
}

function clearSessionForm(): void {
  $<HTMLTextAreaElement>('#session-cookie').value = '';
  $<HTMLInputElement>('#session-agent').value = '';
  const note = sessionNote();
  note.dataset['tone'] = '';
  note.textContent = '';
}

async function saveSession(): Promise<void> {
  const note = sessionNote();
  const link = $<HTMLInputElement>('#session-host').value.trim();
  const cookie = $<HTMLTextAreaElement>('#session-cookie').value;
  const agent = $<HTMLInputElement>('#session-agent').value.trim();
  if (!link) {
    note.dataset['tone'] = 'error';
    note.textContent = 'Give the host, or paste the link that came back refused.';
    return;
  }
  if (cookie.trim() === '') {
    note.dataset['tone'] = 'error';
    note.textContent = 'The Cookie header copied from that browser is what unlocks the host.';
    return;
  }
  let saved: { host: string } | null = null;
  try {
    saved = await sucr.session.set(link, cookie, agent === '' ? null : agent);
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
    return;
  }
  clearSessionForm();
  // After the clear, not before: `clearSessionForm` blanks this very line, and
  // the toast alone would leave the form with no record of what just happened.
  note.dataset['tone'] = '';
  note.textContent = `Saved for ${saved.host}; every request to that host carries it now.`;
  await loadSessions();
  renderSessions();
  showToast(`Session saved for ${saved.host}`);
}

async function forgetSession(host: string): Promise<void> {
  const note = sessionNote();
  try {
    const gone = await sucr.session.forget(host);
    note.dataset['tone'] = '';
    note.textContent = gone.forgotten
      ? `Dropped the session for ${host}.`
      : `There was no session left for ${host}.`;
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
    return;
  }
  await loadSessions();
  renderSessions();
}

/** From a refused card: carry the link to the form that can unlock it. */
function openSessionForm(link: string): void {
  selectView('settings', 'Settings');
  setPillar('network', true);
  const host = $<HTMLInputElement>('#session-host');
  host.value = link;
  $<HTMLTextAreaElement>('#session-cookie').focus();
}

/* --- site credentials: the rows, the form, the 401 bridge -------------------- */
/* The list arrives inside `app:get-info`, which carries the length of the secret
   and never the secret, so nothing here can put a stored secret back into a box.
   The boxes are a replacement field, same as the proxy pair. */

/** Which header the form is filling in. The switch's pressed button is the whole
 *  state: no second copy of it to fall out of step, and the word it carries is the
 *  same word the credentials table and the bridge use. */
type OriginKind = 'basic' | 'header-secret';

function originKindBox(): HTMLElement {
  return $<HTMLElement>('#origin-kind');
}

function originKind(): OriginKind {
  const pressed = originKindBox().querySelector<HTMLButtonElement>('button[aria-pressed="true"]');
  return pressed?.dataset['val'] === 'header-secret' ? 'header-secret' : 'basic';
}

/** Moves the switch and the boxes together, because the boxes mean different things
 *  per kind: a token has no name in front of it, so its username box is not merely
 *  ignored, it is not part of the form. The label and placeholder are CSSOM/DOM
 *  attribute writes, not new markup, which is what keeps this switch free of bytes
 *  in the stylesheet. */
function setOriginKind(kind: OriginKind): void {
  for (const button of originKindBox().querySelectorAll<HTMLButtonElement>('button[data-val]')) {
    button.setAttribute('aria-pressed', String(button.dataset['val'] === kind));
  }
  const user = $<HTMLInputElement>('#origin-user');
  const secret = $<HTMLInputElement>('#origin-pass');
  if (kind === 'header-secret') {
    user.value = '';
    user.hidden = true;
    secret.placeholder = 'bearer token, exactly as pasted';
    secret.setAttribute('aria-label', 'Token for that host');
  } else {
    user.hidden = false;
    secret.placeholder = 'password';
    secret.setAttribute('aria-label', 'Password for that host');
  }
}

function renderOrigins(): void {
  const rows = $<HTMLElement>('#origin-rows');
  const list = appInfo?.originAuths ?? [];
  if (list.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'session-row';
    empty.textContent = 'No site credentials stored.';
    rows.replaceChildren(empty);
    return;
  }
  rows.replaceChildren(
    ...list.map((o) => {
      const row = document.createElement('div');
      row.className = 'session-row';
      const host = document.createElement('b');
      host.textContent = o.host;
      host.title = `Saved ${when(o.createdAt)}`;
      const meta = document.createElement('span');
      meta.textContent =
        o.kind === 'header-secret'
          ? `${formatBytes(o.secretBytes)} bearer token`
          : `${o.username} · ${formatBytes(o.secretBytes)} password`;
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'ghost';
      button.dataset['forgetOrigin'] = o.host;
      button.textContent = 'Forget';
      row.append(host, meta, button);
      return row;
    }),
  );
}

function originNote(): HTMLElement {
  return $<HTMLElement>('#origin-note');
}

function originFor(link: string): OriginAuth | undefined {
  const host = hostKey(link);
  return host === '' ? undefined : (appInfo?.originAuths ?? []).find((o) => o.host === host);
}

function clearOriginForm(): void {
  $<HTMLInputElement>('#origin-host').value = '';
  $<HTMLInputElement>('#origin-user').value = '';
  $<HTMLInputElement>('#origin-pass').value = '';
  // The kind switch is deliberately left where the user put it: it is how the form is
  // shaped, not part of what was typed, and a host that takes a token usually takes
  // more than one.
  const note = originNote();
  note.dataset['tone'] = '';
  note.textContent = '';
}

/** The rows are the engine's answer, not something this page may edit in place,
 *  so a write is followed by a fresh read of the whole info line. */
async function refreshOrigins(): Promise<void> {
  try {
    appInfo = await sucr.app.getInfo();
  } catch {
    // The write itself landed; the list keeps its last reading.
  }
  renderSettings();
}

type OriginWrite = Awaited<ReturnType<typeof sucr.origin.setAuth>>;

/** The three readings one write can give back, in the order the engine can
 *  produce them: a pair stored, a pair dropped, or nothing there to drop. A
 *  row's Forget button and the form's Save button run through the same channel,
 *  so they get their answer in the same words. */
function originWriteLine(saved: OriginWrite, host: string): string {
  return saved.stored
    ? `Saved for ${host}; every request to that host carries it now.`
    : saved.forgotten
      ? `Dropped the credential for ${host}.`
      : `There was no credential stored for ${host}.`;
}

/** One write for the whole credential, because a host holds one: the engine puts
 *  exactly one `Authorization` on a request, so the kind chosen here replaces
 *  whatever the host had rather than adding to it. An empty secret is the forget for
 *  either kind, which is why the box named "Save" answers for the removal too — and
 *  why a token needs no username to be sent alongside it. */
async function saveOriginAuth(): Promise<void> {
  const note = originNote();
  const link = $<HTMLInputElement>('#origin-host').value.trim();
  const kind = originKind();
  const username = kind === 'header-secret' ? '' : $<HTMLInputElement>('#origin-user').value.trim();
  const password = $<HTMLInputElement>('#origin-pass').value;
  if (link === '') {
    note.dataset['tone'] = 'error';
    note.textContent = 'Give the host, or paste the link that answered 401.';
    return;
  }
  let saved: OriginWrite;
  try {
    saved = await sucr.origin.setAuth(link, username, password, kind);
  } catch (err) {
    // The refusal keeps what was typed, password included: the fix is one
    // keystroke, and the engine never saw the pair.
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
    return;
  }
  clearOriginForm();
  note.dataset['tone'] = '';
  note.textContent = originWriteLine(saved, saved.host);
  await refreshOrigins();
  showToast(`Credential ${saved.stored ? 'saved' : 'dropped'} for ${saved.host}`);
}

async function forgetOrigin(host: string): Promise<void> {
  const note = originNote();
  let gone: OriginWrite;
  try {
    // No kind: the host is the whole key, and a row's Forget drops whichever
    // credential it holds — which is why the list can act without remembering.
    gone = await sucr.origin.setAuth(host, '', '');
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
    return;
  }
  note.dataset['tone'] = '';
  note.textContent = originWriteLine(gone, host);
  await refreshOrigins();
}

/** The add-on card. Rows are elements, not markup: a browser's name comes from
 *  the registry and a registry value is not a template. */
function renderAddonCard(addon: AddonReport | undefined): void {
  const list = $<HTMLElement>('#set-addon-list');
  list.replaceChildren();
  if (!addon || addon.browsers.length === 0) {
    const empty = document.createElement('li');
    empty.className = 'addon-empty';
    empty.textContent = 'No browser found. Install one, then reopen this card.';
    list.append(empty);
  } else {
    for (const browser of addon.browsers) {
      const row = document.createElement('li');
      row.className = 'addon-row';
      const name = document.createElement('span');
      name.className = 'addon-name';
      name.textContent = browser.name;
      name.title = browser.exe;
      const family = document.createElement('span');
      family.className = 'addon-family';
      family.textContent = browser.familyLabel;
      const state = document.createElement('span');
      state.className = 'addon-state';
      state.dataset['tone'] = browser.registered ? 'ok' : 'plain';
      state.textContent = browser.registered ? 'relay registered' : 'relay not registered';
      const open = document.createElement('button');
      open.type = 'button';
      open.className = 'ghost addon-open';
      open.dataset['exe'] = browser.exe;
      open.textContent = 'Open';
      row.append(name, family, state, open);
      list.append(row);
    }
  }
  const note = $<HTMLElement>('#set-addon-note');
  const staged = addon?.staged;
  note.dataset['tone'] = staged && (staged.chromePresent || staged.firefoxPresent) ? '' : 'error';
  note.textContent = staged && !staged.chromePresent && !staged.firefoxPresent
    ? 'The bundled add-on folder is missing from this install. Reinstall Sucr to restore it.'
    : 'A sideloaded add-on cannot be pushed into a browser: Sucr stages it and registers the relay, and each browser takes one toggle — Developer mode → Load unpacked, or about:debugging → Load Temporary Add-on.';
}

async function setupAddon(action: 'install' | 'open' | 'folder', target?: string): Promise<void> {
  const note = $<HTMLElement>('#set-addon-note');
  let answer: { ok: boolean; addon?: AddonReport; error?: string; browser?: string };
  try {
    answer = (await sucr.addon.setup(action, target ?? null)) as typeof answer;
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
    return;
  }
  if (!answer.ok) {
    note.dataset['tone'] = 'error';
    note.textContent = answer.error ?? 'That did not work.';
    return;
  }
  if (answer.addon) {
    renderAddonCard(answer.addon);
    if (action === 'install') {
      note.dataset['tone'] = '';
      note.textContent = 'The add-on folder is staged and the relay is registered for every browser Sucr found. Open a browser and load the folder — Sucr cannot click that toggle for you.';
    }
  } else if (action === 'open') {
    note.dataset['tone'] = '';
    note.textContent = `${answer.browser ?? 'The browser'} is open on its add-on page. Turn on Developer mode, then load the staged folder or file.`;
  }
}

/** The update card. The feed is a setting and the link is the browser's: this
 *  app reads a version number and asks Windows to open one address, nothing
 *  more. `updateUrl` is remembered from the last check so the button can hand
 *  over exactly the link that was reported available. */
let updateUrl = '';

function renderUpdateReport(report: UpdateReport): void {
  const note = $<HTMLElement>('#set-update-note');
  const button = $<HTMLButtonElement>('#set-update-download');
  if (report.error) {
    updateUrl = '';
    button.hidden = true;
    note.dataset['tone'] = 'error';
    note.textContent = report.error;
    return;
  }
  note.dataset['tone'] = '';
  if (report.available && report.url) {
    updateUrl = report.url;
    button.hidden = false;
    button.disabled = false;
    note.textContent = `Version ${report.latest} is published; this build is ${report.current} ${BUILD_STAGE}. The button opens the installer link in your browser.`;
    return;
  }
  updateUrl = '';
  button.hidden = true;
  note.textContent = report.latest
    ? `This build is ${report.current} ${BUILD_STAGE}; the release file says ${report.latest}, so there is nothing newer.`
    : `This build is ${report.current} ${BUILD_STAGE}; the release file carries no version Sucr can compare.`;
}

async function saveUpdateFeed(): Promise<void> {
  const field = $<HTMLInputElement>('#set-update-feed');
  const value = field.value.trim();
  const note = $<HTMLElement>('#set-update-note');
  try {
    await saveSetting('update.feed_url', value || null);
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
    return;
  }
  $<HTMLElement>('#set-update-feed-row').hidden = true;
  note.dataset['tone'] = '';
  note.textContent = value
    ? 'Custom feed saved. Check for updates reads it instead of the built-in one.'
    : 'Built-in feed restored. Check for updates reads it again.';
}

function toggleUpdateFeed(): void {
  const row = $<HTMLElement>('#set-update-feed-row');
  row.hidden = !row.hidden;
  if (!row.hidden) $<HTMLInputElement>('#set-update-feed').focus();
}

async function checkForUpdate(): Promise<void> {
  const button = $<HTMLButtonElement>('#set-update-check');
  const note = $<HTMLElement>('#set-update-note');
  note.dataset['tone'] = '';
  note.textContent = 'Reading the release feed…';
  button.disabled = true;
  try {
    renderUpdateReport(await sucr.update.check('check'));
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
  } finally {
    button.disabled = false;
  }
}

async function openUpdate(): Promise<void> {
  const note = $<HTMLElement>('#set-update-note');
  if (!updateUrl) {
    note.dataset['tone'] = 'error';
    note.textContent = 'Check for updates first: there is no link to open yet.';
    return;
  }
  const answer = await sucr.update.check('open', updateUrl);
  note.dataset['tone'] = answer.ok ? '' : 'error';
  note.textContent = answer.ok
    ? 'The installer link is open in your browser. Sucr leaves the download and the install to it.'
    : answer.error ?? 'Windows did not open that link.';
}

async function openLogFolder(): Promise<void> {
  const note = $<HTMLElement>('#set-debug-note');
  try {
    const answer = (await sucr.debug.report('open')) as { path: string };
    note.dataset['tone'] = '';
    note.textContent = `Explorer is open on ${answer.path}.`;
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
  }
}

async function reportProblem(): Promise<void> {
  const note = $<HTMLElement>('#set-debug-note');
  try {
    const answer = (await sucr.debug.report('issue')) as { ok: boolean; error?: string };
    note.dataset['tone'] = answer.ok ? '' : 'error';
    note.textContent = answer.ok
      ? 'GitHub is open in your browser with this build’s version and the last log lines. Nothing is posted until you press Submit there.'
      : answer.error ?? 'Windows did not open that link.';
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
  }
}

async function clearDebugLog(): Promise<void> {
  const note = $<HTMLElement>('#set-debug-note');
  try {
    await sucr.debug.report('clear');
  } catch (err) {
    note.dataset['tone'] = 'error';
    note.textContent = err instanceof Error ? err.message : String(err);
    return;
  }
  debugReport = null;
  await refreshDebugLog();
}

/** From a link that answered 401: carry it to the form that can answer it. */
function openOriginForm(link: string): void {
  selectView('settings', 'Settings');
  setPillar('network', true);
  $<HTMLInputElement>('#origin-host').value = link;
  // The box the kind actually uses — a form left on token has no visible name box,
  // and focusing a hidden field would leave the user typing into nothing.
  const user = $<HTMLInputElement>('#origin-user');
  ($<HTMLInputElement>(user.hidden ? '#origin-pass' : '#origin-user')).focus();
}

settingsBodyEl.addEventListener('click', (event) => {
  const el = event.target as Element;
  const forget = el.closest<HTMLButtonElement>('button[data-forget]');
  if (forget) {
    void forgetSession(forget.dataset['forget'] ?? '');
    return;
  }
  if (el.closest('#session-save')) {
    void saveSession().catch(reportSettingError);
    return;
  }
  if (el.closest('#session-clear-form')) {
    clearSessionForm();
    return;
  }
  const forgetCredential = el.closest<HTMLButtonElement>('button[data-forget-origin]');
  if (forgetCredential) {
    void forgetOrigin(forgetCredential.dataset['forgetOrigin'] ?? '').catch(reportSettingError);
    return;
  }
  if (el.closest('#origin-save')) {
    void saveOriginAuth().catch(reportSettingError);
    return;
  }
  if (el.closest('#origin-clear-form')) {
    clearOriginForm();
    return;
  }
  // The kind switch is a `.seg-control` for its look and nothing else: no `data-set`,
  // because a credential kind is not a setting. It is part of the one write, and the
  // engine only learns it when the form is saved.
  const originKindButton = el.closest<HTMLButtonElement>('#origin-kind button[data-val]');
  if (originKindButton) {
    const value = originKindButton.dataset['val'];
    if (value === 'basic' || value === 'header-secret') setOriginKind(value);
    return;
  }
  const addonOpen = el.closest<HTMLButtonElement>('button.addon-open');
  if (addonOpen) {
    void setupAddon('open', addonOpen.dataset['exe']);
    return;
  }
  const seg = el.closest<HTMLButtonElement>('.seg-control[data-set] button[data-val]');
  if (seg) {
    const key = seg.closest<HTMLElement>('.seg-control')?.dataset['set'];
    const value = seg.dataset['val'];
    if (key && value !== undefined) void saveSetting(key, value).catch(reportSettingError);
    return;
  }
  // The window's day chips are one mask, not one value per button, so the generic
  // handler above cannot own them: a click recomputes the whole seven bits.
  const dayButton = el.closest<HTMLButtonElement>('#set-window-days button[data-day]');
  if (dayButton) {
    toggleWindowDay(Number(dayButton.dataset['day']));
    return;
  }
  if (el.closest('#set-folder-browse')) {
    void (async () => {
      try {
        const picked = await sucr.app.pickFolder();
        if (picked) {
          await saveSetting('storage.default_folder', picked).catch(reportSettingError);
          const folderInput = $<HTMLInputElement>('#set-folder');
          if (folderInput) folderInput.value = picked;
          renderSettings();
        }
      } catch (err) {
        console.error('pickFolder error:', err);
      }
    })();
  } else if (el.closest('#set-folder-reset')) {
    void saveSetting('storage.default_folder', null).catch(reportSettingError);
  } else if (el.closest('#set-cap-btn')) {
    openSpeedPopover({ mode: 'global' }, $<HTMLElement>('#set-cap-btn'));
  } else if (el.closest('#set-copy-path')) {
    void copyDbPath();
  } else if (el.closest('#set-clear-finished')) {
    void clearFinished().catch(reportSettingError);
  } else if (el.closest('#set-reset-all')) {
    void resetAllSettings().catch(reportSettingError);
  } else if (el.closest('#set-addon-setup')) {
    void setupAddon('install');
  } else if (el.closest('#set-addon-folder')) {
    void setupAddon('folder');
  } else if (el.closest('#set-update-save')) {
    void saveUpdateFeed();
  } else if (el.closest('#set-update-toggle')) {
    toggleUpdateFeed();
  } else if (el.closest('#set-update-check')) {
    void checkForUpdate();
  } else if (el.closest('#set-update-download')) {
    void openUpdate();
  } else if (el.closest('#set-debug-open')) {
    void openLogFolder();
  } else if (el.closest('#set-debug-clear')) {
    void clearDebugLog();
  } else if (el.closest('#set-debug-issue')) {
    void reportProblem();
  }
});

settingsBodyEl.addEventListener('change', (event) => {
  const target = event.target;
  if (!(target instanceof HTMLInputElement)) return;
  const key = target.dataset['toggle'];
  if (key) {
    void saveSetting(key, target.checked ? 'true' : 'false').catch(reportSettingError);
  } else if (target.dataset['setTime']) {
    commitWindowTime(`#${target.id}`, target.dataset['setTime']);
  } else if (target.id === 'set-folder') {
    commitFolder();
  } else if (target.id === 'set-proxy-url') {
    commitProxyField('#set-proxy-url', 'network.proxy_url');
  } else if (target.id === 'set-proxy-bypass') {
    commitProxyField('#set-proxy-bypass', 'network.proxy_bypass');
  } else if (target.id === 'set-proxy-user' || target.id === 'set-proxy-pass') {
    commitProxyAuth();
  }
});

$<HTMLInputElement>('#set-folder').addEventListener('keydown', (event) => {
  if ((event as KeyboardEvent).key === 'Enter') {
    event.preventDefault();
    commitFolder();
  }
});

/* Enter commits in any of these boxes, so a pasted address — or a pasted pair —
   does not have to be tabbed out of before the engine sees it. */
for (const [id, commit] of [
  ['#set-proxy-url', () => commitProxyField('#set-proxy-url', 'network.proxy_url')],
  ['#set-proxy-bypass', () => commitProxyField('#set-proxy-bypass', 'network.proxy_bypass')],
  ['#set-proxy-user', commitProxyAuth],
  ['#set-proxy-pass', commitProxyAuth],
] as const) {
  $<HTMLInputElement>(id).addEventListener('keydown', (event) => {
    if ((event as KeyboardEvent).key === 'Enter') {
      event.preventDefault();
      commit();
    }
  });
}

/* --- completion chime ---------------------------------------------------- */
let audioCtx: AudioContext | null = null;
const lastStatus = new Map<number, string>();

/** Two sine notes, under 200 ms. The AudioContext is created lazily because
 *  WebView2 blocks audio until the user has interacted with the page. */
function chime(): void {
  if (!settingOn('ui.sound_on_complete')) return;
  try {
    audioCtx ??= new AudioContext();
    const ctx = audioCtx;
    const now = ctx.currentTime;
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.0001, now);
    gain.gain.exponentialRampToValueAtTime(0.14, now + 0.012);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.18);
    gain.connect(ctx.destination);
    for (const [offset, freq] of [[0, 880], [0.07, 1320]] as const) {
      const osc = ctx.createOscillator();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(freq, now + offset);
      osc.connect(gain);
      osc.start(now + offset);
      osc.stop(now + offset + 0.16);
    }
  } catch (err) {
    console.error(err);
  }
}

function detectCompletions(): void {
  const seen = new Set<number>();
  for (const d of library) {
    seen.add(d.id);
    const before = lastStatus.get(d.id);
    if (before !== undefined && before !== d.status) {
      if (d.status === 'finished' || d.status === 'verified') chime();
    }
    lastStatus.set(d.id, d.status);
  }
  for (const id of [...lastStatus.keys()]) {
    if (!seen.has(id)) lastStatus.delete(id);
  }
}

document.addEventListener('keydown', (event) => {
  if ((event as KeyboardEvent).key !== 'Escape') return;
  if (!speedPopoverEl.hidden) closeSpeedPopover();
  if (!removePopoverEl.hidden) closeRemovePopover();
  if (!refreshPopoverEl.hidden) closeRefreshPopover();
});

function speedTrigger(el: HTMLElement, target: SpeedTarget): void {
  el.classList.add('speed-trigger');
  el.tabIndex = 0;
  el.setAttribute('role', 'button');
  el.title = target.mode === 'global' ? 'Set global speed limit' : 'Set download speed limit';
  el.addEventListener('click', (event) => {
    event.stopPropagation();
    openSpeedPopover(target, el);
  });
  el.addEventListener('keydown', (event) => {
    const k = event as KeyboardEvent;
    if (k.key !== 'Enter' && k.key !== ' ') return;
    k.preventDefault();
    k.stopPropagation();
    openSpeedPopover(target, el);
  });
}

/* --- Chunks: live blocks + attempts -------------------------------------- */
const attemptsEl = $<HTMLElement>('#attempts');

let mapWatchId: number | null = null;
let mapStatus: string | null = null;
const mapBlocks = new Map<number, MapBlock>();
const mapAttempts = new Map<number, MapAttempt>();

/** The inspector is a static column on wide layouts and a drawer below 1180px;
 *  a shut drawer means no watchers, so the engine stops emitting. */
function inspectorVisible(): boolean {
  return !drawerQuery.matches || document.body.classList.contains('drawer-open');
}

/** One subscription at a time, on the selected download while the inspector is
 *  on screen. The engine only emits while watched, so a closed inspector (or no
 *  selection) costs nothing. */
async function syncMap(): Promise<void> {
  const want = selectedId !== null && inspectorVisible() ? selectedId : null;

  if (mapWatchId !== null && mapWatchId !== want) {
    const old = mapWatchId;
    mapWatchId = null;
    mapStatus = null;
    mapBlocks.clear();
    mapAttempts.clear();
    renderAttempts();
    refreshRoutesDeck();
    await sucr.download.unwatch(old).catch(() => undefined);
  }
  if (want === null || mapWatchId === want) return;

  mapWatchId = want;
  mapStatus = library.find((d) => d.id === want)?.status ?? null;
  const snapshot = await sucr.download.watch(want).catch(() => null);
  if (mapWatchId !== want) return;
  if (snapshot) applyMap(snapshot);
}

/** Replaces the subscription for one download; used when its status changes
 *  and `watch` should start reading the live map or the durable rows. */
async function rewatchMap(id: number): Promise<void> {
  mapWatchId = null;
  await sucr.download.unwatch(id).catch(() => undefined);
  mapWatchId = id;
  const snapshot = await sucr.download.watch(id).catch(() => null);
  if (mapWatchId !== id) return;
  if (snapshot) applyMap(snapshot);
}

function applyMap(payload: BlockMap): void {
  if (payload.full) {
    mapBlocks.clear();
    mapAttempts.clear();
  }
  for (const block of payload.blocks) mapBlocks.set(block.idx, block);
  for (const attempt of payload.attempts) mapAttempts.set(attempt.id, attempt);
  sampleRoutes(Date.now());
  renderAttempts();
  if (routesDeck && routesDeck.id === payload.id) refreshRoutesDeck();
}

/** A chunk as the pane shows it: one row per byte-range slice, with every
 *  attempt that touched it folded in. Wire bytes and transfer time are summed
 *  across attempts: a retry really was paid for, so it counts. */
interface ChunkRow {
  blockIdx: number;
  rangeStart: number;
  rangeEnd: number;
  bytes: number;
  durationMs: number;
  /** Start of the attempt still running, or null once the chunk has landed. */
  liveStartedAt: number | null;
  state: string;
  live: boolean;
  networkLabel: string;
  networkId: string;
  retries: number;
  reason: string | null;
  detail: string | null;
}

function chunkRows(): ChunkRow[] {
  const byBlock = new Map<number, MapAttempt[]>();
  for (const attempt of mapAttempts.values()) {
    const group = byBlock.get(attempt.blockIdx);
    if (group) group.push(attempt);
    else byBlock.set(attempt.blockIdx, [attempt]);
  }
  const rows: ChunkRow[] = [];
  for (const [blockIdx, attempts] of byBlock) {
    attempts.sort((a, b) => a.id - b.id);
    const first = attempts[0] as MapAttempt;
    const last = attempts[attempts.length - 1] as MapAttempt;
    const live = attempts.filter((a) => a.state === 'active');
    let reason: string | null = null;
    let detail: string | null = null;
    for (const attempt of attempts) {
      if (attempt.reason) reason = attempt.reason;
      if (attempt.detail) detail = attempt.detail;
    }
    rows.push({
      blockIdx,
      rangeStart: first.rangeStart,
      rangeEnd: last.rangeEnd,
      bytes: attempts.reduce((sum, a) => sum + a.bytes, 0),
      durationMs: attempts.reduce((sum, a) => sum + a.durationMs, 0),
      liveStartedAt: live.length > 0 ? (live[live.length - 1] as MapAttempt).startedAt : null,
      state: live.length > 0 ? 'active' : last.state,
      live: live.length > 0,
      networkLabel: (live[0] ?? last).networkLabel,
      networkId: (live[0] ?? last).networkId,
      retries: attempts.length - 1,
      reason,
      detail,
    });
  }
  return rows;
}

function chunkRate(row: ChunkRow): string {
  // The closed attempts' durations plus the clock on the one still running: a
  // retried chunk has new bytes against a nearly-empty recorded duration, and
  // dividing by the recorded part alone reads 100× the truth.
  const open = row.liveStartedAt === null ? 0 : Date.now() - row.liveStartedAt;
  const elapsed = row.durationMs + open;
  return elapsed > 0 ? formatRate((row.bytes / elapsed) * 1000) : '—';
}

function renderAttempts(): void {
  // Chunks still transferring are pinned above finished ones: they are what
  // the pane is opened to diagnose. The rest read in chunk order, because a
  // row is now a slice of the file, not a connection.
  const rows = chunkRows()
    .sort((a, b) => {
      const live = Number(!a.live) - Number(!b.live);
      return live !== 0 ? live : a.blockIdx - b.blockIdx;
    })
    .slice(0, 24);
  if (rows.length === 0) {
    attemptsEl.replaceChildren(
      Object.assign(document.createElement('p'), {
        className: 'pane-empty',
        textContent: 'No chunks recorded yet.',
      }),
    );
    return;
  }
  const fragment = document.createDocumentFragment();
  for (const row of rows) {
    const el = document.createElement('div');
    el.className = 'attempt';
    el.dataset['state'] = row.state;
    el.dataset['net'] = row.networkId;
    el.title = row.detail ?? row.reason ?? '';
    const dot = document.createElement('span');
    dot.className = 'dot';
    dot.setAttribute('aria-hidden', 'true');
    const main = document.createElement('span');
    main.className = 'attempt-main';
    const title = document.createElement('span');
    title.className = 'attempt-title';
    const retry = row.retries > 0 ? ` · retry ${row.retries}` : '';
    title.textContent = `Chunk #${row.blockIdx} · ${formatBytes(row.rangeStart)}–${formatBytes(row.rangeEnd)}${retry}`;
    const sub = document.createElement('span');
    sub.className = 'attempt-sub';
    sub.textContent = `${row.networkLabel} · ${formatBytes(row.bytes)} · ${chunkRate(row)} · ${row.state}`;
    main.append(title, sub);
    const why = document.createElement('span');
    why.className = 'attempt-why';
    why.textContent = row.reason ?? '';
    el.append(dot, main, why);
    fragment.append(el);
  }
  attemptsEl.replaceChildren(fragment);
}

/* --- inspector drawer below 1180px --------------------------------------- */
const drawerQuery = window.matchMedia('(max-width: 1180px)');
const scrimEl = $<HTMLElement>('#scrim');
const drawerCloseEl = $<HTMLButtonElement>('#drawer-close');
const drawerToggleEl = $<HTMLButtonElement>('#drawer-toggle');

function setDrawer(open: boolean): void {
  document.body.classList.toggle('drawer-open', open);
  void syncMap();
}

scrimEl.addEventListener('click', () => setDrawer(false));
drawerCloseEl.addEventListener('click', () => setDrawer(false));
drawerToggleEl.addEventListener('click', () => {
  if (drawerQuery.matches) setDrawer(!document.body.classList.contains('drawer-open'));
  else tabs[activeTab]?.focus();
});
document.addEventListener('keydown', (event) => {
  const e = event as KeyboardEvent;
  if (e.key === 'Escape' && document.body.classList.contains('drawer-open')) {
    e.preventDefault();
    setDrawer(false);
    return;
  }
  if (e.altKey && e.key === '3') {
    e.preventDefault();
    if (drawerQuery.matches) setDrawer(!document.body.classList.contains('drawer-open'));
    else tabs[activeTab]?.focus();
  }
});
drawerQuery.addEventListener('change', () => {
  if (!drawerQuery.matches) setDrawer(false);
  alignIndicator();
  void syncMap();
});

/* --- compact rail ---------------------------------------------------------
   One class on <body> drives the whole thing: the grid column drops to a 6 px
   edge and CSS :hover/:focus-within wipe the full nav open over the list. No
   pointer bookkeeping here, so the reveal cannot desync from the pointer.
   On by default and remembered, like the other view prefs: a fresh launch used
   to come back expanded and the hover behaviour looked missing. Only an
   explicit "false" turns it off, so first run needs no key. */
const railToggleEl = $<HTMLButtonElement>('#rail-toggle');
const RAIL_KEY = 'sucr.rail_compact';

function setRailCompact(compact: boolean): void {
  document.body.classList.toggle('rail-compact', compact);
  railToggleEl.setAttribute('aria-pressed', String(compact));
  localStorage.setItem(RAIL_KEY, String(compact));
}

setRailCompact(localStorage.getItem(RAIL_KEY) !== 'false');

railToggleEl.addEventListener('click', () => {
  setRailCompact(!document.body.classList.contains('rail-compact'));
});

/* The rail's focus reveal is armed only by Tab and disarmed by the first
   pointer event anywhere. Without that, a nav row left focused keeps the panel
   open after the pointer walks away -- the reveal is a peek, not a pin. */
const armRailKeyboard = () => document.body.classList.add('rail-kbd');
const disarmRailKeyboard = () => document.body.classList.remove('rail-kbd');
document.addEventListener('keydown', (event) => {
  const e = event as KeyboardEvent;
  if (e.key === 'Tab') armRailKeyboard();
  if (e.altKey && e.key === '1') {
    e.preventDefault();
    setRailCompact(!document.body.classList.contains('rail-compact'));
  }
});
document.addEventListener('pointerdown', disarmRailKeyboard, { capture: true });
document.addEventListener('pointermove', disarmRailKeyboard, { capture: true });

/* --- inspector panes ----------------------------------------------------- */
function infoCard(title?: string): { card: HTMLElement; body: HTMLElement } {
  const card = document.createElement('div');
  card.className = 'info-card';
  const body = document.createElement('div');
  if (title) {
    const h = document.createElement('h3');
    h.textContent = title;
    card.append(h);
  }
  card.append(body);
  return { card, body };
}

function monoLine(parent: HTMLElement, text: string): HTMLElement {
  const line = document.createElement('div');
  line.className = 'mono-line';
  line.textContent = text;
  parent.append(line);
  return line;
}

function fact(list: HTMLElement, key: string, value: string, copy?: string): void {
  const dt = document.createElement('dt');
  dt.textContent = key;
  const dd = document.createElement('dd');
  dd.textContent = value;
  if (copy !== undefined) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'fact-copy';
    btn.title = 'Copy this link';
    btn.setAttribute('aria-label', `Copy the ${key.toLowerCase()} link`);
    btn.append(svgUse('#i-copy'));
    onTap(btn, () => {
      void (async () => {
        const copied = await copyText(copy);
        showToast(copied ? 'Copied download link' : 'Could not reach the clipboard.');
      })();
    });
    dd.append(btn);
  }
  list.append(dt, dd);
}

function when(ms: number | null): string {
  return ms === null ? '—' : new Date(ms).toLocaleString();
}

function safeHost(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return '—';
  }
}

function updateOverview(d: Download): void {
  const wrap = document.createElement('div');
  wrap.className = 'detail';

  const speed = document.createElement('div');
  speed.className = 'ov-card';
  const label = document.createElement('div');
  label.className = 'ov-label';
  label.textContent = 'Bandwidth';
  const value = document.createElement('div');
  value.className = 'ov-speed';
  const bps = rateOf(d);
  value.textContent = d.status === 'downloading' ? (bps > 0 ? formatRate(bps) : '—') : '—';
  if (d.status === 'downloading' && bps <= 0) {
    value.append(document.createTextNode(' '));
    const small = document.createElement('small');
    small.textContent = 'sampling';
    value.append(small);
  }
  speed.append(label, value);
  speedTrigger(value, { mode: 'download', id: d.id });

  const note = document.createElement('div');
  note.className = 'ov-note';
  const inflight = [...mapAttempts.values()].filter((a) => a.state === 'active').length;
  let noteText = STATUS_LABEL[d.status] ?? d.status;
  let tone = 'muted';
  if (d.status === 'downloading') {
    if (mapWatchId === d.id && inflight > 0) {
      // The width the ramp is running and the attempts actually carrying bytes
      // are different numbers; said together they explain a straggler instead
      // of looking like a contradiction. A refusal halves the gate at once but
      // cannot stop the requests already streaming, so for a moment more are
      // running than the gate would now admit — and then a ratio is a lie.
      noteText =
        typeof d.streamsLive === 'number' && d.streamsLive > 0
          ? inflight <= d.streamsLive
            ? `${inflight} of ${d.streamsLive} carrying bytes`
            : `${inflight} running, ${d.streamsLive} allowed`
          : `${inflight} stream${inflight === 1 ? '' : 's'} in flight`;
      tone = 'ok';
    } else if (mapWatchId === d.id) {
      noteText = 'between streams — retrying';
      tone = 'warn';
    } else {
      noteText = 'download in progress';
      tone = 'ok';
    }
  } else if (d.status === 'finished' || d.status === 'verified') {
    noteText = d.checksumAlgo ? `Verified · ${d.checksumAlgo}` : 'Finished';
    tone = 'ok';
  } else if (d.status === 'needs-attention') {
    noteText = d.errorCode ?? 'Needs attention';
    tone = 'error';
  } else if (d.status === 'paused') {
    noteText = `Paused · ${formatBytes(d.verifiedBytes)} kept`;
    tone = 'warn';
  }
  note.dataset['tone'] = tone;
  if (tone === 'ok') note.append(svgUse('#i-check', 'ov-check'));
  note.append(document.createTextNode(noteText));
  speed.append(note);
  const tpHost = document.createElement('div');
  tpHost.className = 'tp-host';
  tpHost.dataset.tp = 'job';
  speed.append(tpHost);
  wrap.append(speed);

  const deck = document.createElement('div');
  deck.className = 'ov-card';
  const deckHead = document.createElement('div');
  deckHead.className = 'ov-label';
  deckHead.textContent = 'Chunk routes';
  const deckTotal = document.createElement('p');
  deckTotal.className = 'attrib-note';
  const deckBody = document.createElement('div');
  deck.append(deckHead, deckTotal, deckBody);
  wrap.append(deck);
  routesDeck = { id: d.id, total: deckTotal, body: deckBody, last: null };
  void fillRoutesDeck(d.id);

  const facts = document.createElement('dl');
  facts.className = 'facts';
  const sourceLink = d.finalUrl ?? d.url;
  fact(facts, 'Source', sourceLink, sourceLink);
  fact(facts, 'Folder', d.folder ?? '—');
  fact(facts, 'Chunks', d.blockCount ? `${d.blockCount} × ${formatBytes(d.blockSize)}` : 'single stream');
  const width = d.streamsMode === 'single' ? 1 : d.streamsRequested;
  const top = d.streamsMode === 'single' ? width : d.streamsCeiling;
  const streamCap = typeof top === 'number' ? Math.max(top, width ?? 0) : width;
  // The live width while the job runs, the recorded one after it: a closed row
  // has no job to ask, so the number the run stopped at has to be on the row.
  const ran = d.streamsLive ?? d.streamsFinal;
  // An auto job climbs its band, so the live count is measured against the top
  // of it. A pinned job has no band, but a rate limit can still drop it below
  // the pin, and then the pair is the truth rather than the promise. The engine
  // publishes the width, so this no longer waits for the Chunks tab to be open.
  fact(
    facts,
    'Streams',
    width === null || width === undefined
      ? '—'
      : width === 1
        ? '1 (single)'
        : typeof top === 'number' && top > width
          ? ran != null
            ? `${ran} / ${streamCap} parallel`
            : `${width}–${top} parallel`
          : ran != null && ran < width
            ? `${ran} / ${width} parallel`
            : `${width} parallel`,
  );
  fact(facts, 'Checksum', d.checksumAlgo ? `${d.checksumAlgo} · ${d.checksumHex ?? ''}` : 'none');
  if (d.errorCode) fact(facts, 'Error', d.errorHint ? `${d.errorCode} · ${d.errorHint}` : d.errorCode);
  wrap.append(facts);

  const actions = document.createElement('div');
  actions.className = 'actions';
  const act = (
    text: string,
    primary: boolean,
    run: (anchor: HTMLButtonElement) => Promise<unknown> | void,
  ): void => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = primary ? 'primary' : 'ghost';
    button.textContent = text;
    onTap(button, () => {
      const result = run(button);
      if (result) {
        void result
          .then(refresh)
          .catch((err: unknown) => {
            showToast(err instanceof Error ? err.message : String(err));
          });
      }
    });
    actions.append(button);
  };
  if (d.status === 'downloading') act('Pause', false, () => sucr.download.pause(d.id));
  if (d.status === 'queued' || d.status === 'paused' || d.status === 'needs-attention') {
    act('Resume', true, () => sucr.download.start(d.id));
  }
  act('Redownload', false, () => redownload(d));
  act('Remove from list', false, (anchor) => {
    // Only a published job has a file worth asking about; everything else is
    // still staging, and staging is always dropped with the row.
    if (d.status === 'finished' || d.status === 'verified') {
      openRemovePopover(d, anchor);
      return;
    }
    return sucr.download.remove(d.id, false).then((outcome) => {
      if (!outcome.removed) showToast(stillStopping(d.filename));
    });
  });
  wrap.append(actions);

  detailEl.replaceChildren(wrap);
  // Only after the mount: the chart reads the host's width, and a detached
  // element measures 0, which would draw a 120 px stub for a second.
  paintThroughput();
}

function updateSource(d: Download): void {
  const request = infoCard('HTTP request');
  monoLine(request.body, `${d.method} ${d.finalUrl ?? d.url}`);
  if (d.finalUrl && d.finalUrl !== d.url) {
    monoLine(request.body, `redirected from ${d.url}`);
  }
  const resume = document.createElement('div');
  resume.className = 'mono-line';
  resume.style.marginTop = '6px';
  resume.textContent =
    d.resumable === true
      ? 'Ranges accepted — resume is enabled.'
      : d.resumable === false
        ? 'Server refused ranges — this job runs as one stream.'
        : d.errorCode
          ? `Probe refused: ${d.errorHint ?? d.errorCode}`
          : 'Not probed yet.';
  request.body.append(resume);

  // The one refusal a pasted session can answer. Deliberately not a claim about
  // the attempt that just failed: a session saved after it cannot have been
  // carried by it, so the row says what is stored and what to do next.
  if (d.errorCode === 'blocked') {
    const held = sessionFor(d.finalUrl ?? d.url) ?? sessionFor(d.url);
    const line = document.createElement('div');
    line.className = 'mono-line';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'ghost';
    button.textContent = held ? 'Replace the session for this host' : 'Paste a browser session';
    onTap(button, () => openSessionForm(d.finalUrl ?? d.url));
    line.append(button);
    request.body.append(line);
    if (held) {
      monoLine(
        request.body,
        `A session is saved for ${held.host}. Resume this job to send it again with that identity.`,
      );
    }
  }

  // A 401 has exactly one answer this app can hold: the credential the host asked
  // for. Which of the two it needs — a first one, or a better one — is the engine's
  // knowledge, not something to guess from the row. Once a row exists its kind is
  // known locally, so the button can name the thing the box actually takes.
  if (d.errorCode === 'needs-auth') {
    const target = d.finalUrl ?? d.url;
    const held = originFor(target) ?? originFor(d.url);
    const line = document.createElement('div');
    line.className = 'mono-line';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'ghost';
    button.textContent = !held
      ? 'Add a credential for this host'
      : held.kind === 'header-secret'
        ? 'Replace the token for this host'
        : 'Replace the password for this host';
    onTap(button, () => openOriginForm(target));
    line.append(button);
    request.body.append(line);
    if (held) {
      monoLine(
        request.body,
        `A credential is saved for ${held.host}. Resume this job to send it again.`,
      );
    }
  }

  const zone = infoCard('Mark-of-the-Web');
  const note = document.createElement('div');
  note.className = 'mono-line';
  note.textContent =
    d.status === 'finished' || d.status === 'verified'
      ? 'Written to the published file:'
      : 'Will be written when the file is published:';
  const pre = document.createElement('code');
  pre.className = 'zone';
  const referrer = d.finalUrl ?? d.url;
  pre.textContent = `[ZoneTransfer]\nZoneId=3\nReferrerUrl=${referrer}\nHostUrl=${safeHost(referrer)}`;
  zone.body.append(note, pre);

  sourceEl.replaceChildren(request.card, zone.card);
}

function updateSchedule(d: Download): void {
  const timeline = infoCard('Timeline');
  const dl = document.createElement('dl');
  dl.className = 'timeline';
  const row = (key: string, value: string): void => {
    const l = document.createElement('div');
    l.className = 'tl-row';
    const dt = document.createElement('dt');
    dt.textContent = key;
    const dd = document.createElement('dd');
    dd.textContent = value;
    l.append(dt, dd);
    dl.append(l);
  };
  row('Created', when(d.createdAt));
  row('Started', when(d.startedAt));
  row('Finished', when(d.finishedAt));
  if (d.startedAt !== null && d.finishedAt !== null) {
    row('Elapsed', clockText((d.finishedAt - d.startedAt) / 1000));
  }
  timeline.body.append(dl);

  const queue = infoCard('Queue');
  const queued = library
    .filter((x) => x.status === 'queued')
    .sort((a, b) => a.createdAt - b.createdAt);
  const position = queued.findIndex((x) => x.id === d.id);
  const qline = document.createElement('div');
  qline.className = 'mono-line';
  qline.textContent =
    d.status === 'queued' && position >= 0
      ? `Position ${position + 1} of ${queued.length} queued.`
      : `Status: ${STATUS_LABEL[d.status] ?? d.status}.`;
  const policy = document.createElement('div');
  policy.className = 'mono-line';
  policy.style.marginTop = '6px';
  policy.textContent = windowRuleText();
  queue.body.append(qline, policy);

  scheduleEl.replaceChildren(timeline.card, queue.card);
}

function updateFiles(d: Download): void {
  const target = infoCard('Target file');
  const folder = d.folder;
  monoLine(target.body, d.filename);
  monoLine(target.body, folder ? `${folder}` : 'Folder is assigned when the file is published.');
  if (folder) monoLine(target.body, `${folder}\\${d.filename}`);

  const staging = infoCard('Staging');
  monoLine(staging.body, `${d.filename}.${d.id}.sucrpart`);
  const note = document.createElement('div');
  note.className = 'mono-line';
  note.style.marginTop = '6px';
  note.textContent = 'Bytes land in the staging file and are renamed on publish; a resume reuses it.';
  staging.body.append(note);

  const shape = infoCard('Structure');
  monoLine(shape.body, 'Single file.');
  const shapeNote = document.createElement('div');
  shapeNote.className = 'mono-line';
  shapeNote.style.marginTop = '6px';
  shapeNote.textContent = `${formatBytes(d.verifiedBytes)} of ${formatBytes(d.totalBytes)} · ${d.blockCount ?? 0} blocks · ${formatBytes(d.blockSize)} each.`;
  shape.body.append(shapeNote);

  filesEl.replaceChildren(target.card, staging.card, shape.card);
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function asText(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null;
}

/** Strings from a JSON array, for the routes the engine says it tried. */
function asList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : [];
}

/** Turns a history row into the same sentence the engine would say out loud. */
function formatEvent(kind: string, raw: unknown): { label: string; detail: string; tone: string; tooltip?: string } {
  const p = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  switch (kind) {
    case 'probe':
      if (asText(p['error'])) {
        return {
          label: 'Probe refused',
          detail: `${asNumber(p['status']) !== null ? `HTTP ${asNumber(p['status'])}` : asText(p['detail']) ?? 'no answer'} · ${asText(p['route']) ?? 'default route'}`,
          tone: 'error',
        };
      }
      return {
        label: 'Probe',
        detail: `${asNumber(p['status']) !== null ? `HTTP ${asNumber(p['status'])}` : '—'} · ${formatBytes(asNumber(p['total']))} · ${p['ranges'] ? 'ranges' : 'no ranges'}${asText(p['etag']) ? ' · ETag' : ''}`,
        tone: 'info',
      };
    case 'bond-fallback':
      return {
        label: 'Bonding off',
        detail: `none of ${asList(p['tried']).length} bonded devices reached this host — the default route carried it`,
        tone: 'warn',
      };
    case 'proxy': {
      const via = asText(p['via']) ?? 'the proxy';
      // The engine asks the proxy layer per URL, so this row is that answer and not
      // the setting: the two differ the moment a bypass list is non-empty. What a
      // proxied job gives up is named here because it is real — a cap typed on a
      // card paces that card's bytes, and this job books none to any card.
      return p['state'] === 'bypassed'
        ? {
            label: 'Proxy bypassed',
            detail: `${via} is configured, but this host is exempt — the bond carries it as usual`,
            tone: 'info',
          }
        : {
            label: 'Via proxy',
            detail: `${via} carries this download — no adapter is credited, so a per-network limit does not pace it`,
            tone: 'warn',
          };
    }
    case 'plan': {
      const streams = asNumber(p['streams']);
      const max = asNumber(p['streamsMax']);
      return {
        label: 'Plan',
        detail: `${asNumber(p['blocks']) ?? '?'} chunks × ${formatBytes(asNumber(p['blockSize']))} · ${
          streams !== null && max !== null && max > streams ? `${streams}–${max} streams` : `${streams ?? '?'} streams`
        }`,
        tone: 'info',
      };
    }
    case 'resumed':
      return {
        label: 'Resumed',
        detail: `${asNumber(p['done']) ?? 0} of ${asNumber(p['blocks']) ?? '?'} blocks already on disk`,
        tone: 'info',
      };
    case 'bytes-sampled': {
      const windows = asNumber(p['windows']) ?? 0;
      const covered = asNumber(p['covered']);
      const outcome = asText(p['outcome']) ?? 'no-evidence';
      const label = {
        matched: 'Bytes matched — partial kept',
        differed: 'Bytes differ — partial dropped',
        inconclusive: 'Host would not confirm the bytes',
        'no-evidence': 'Nothing on disk to compare',
      }[outcome];
      return {
        label: label ?? 'Bytes sampled',
        detail: `${windows} sample${windows === 1 ? '' : 's'} inside ${formatBytes(covered)}`,
        tooltip: `${asText(p['validator']) ?? 'validator'} moved · ${label ?? outcome}`,
        tone: outcome === 'matched' ? 'info' : 'warn',
      };
    }
    case 'single-stream':
      return { label: 'Single stream', detail: asText(p['reason']) ?? '', tone: 'warn' };
    case 'fallback-single':
      return { label: 'Fallback', detail: 'server ignored Range — restarting as one stream', tone: 'warn' };
    case 'resume-reset':
      return {
        label: 'Resume reset',
        // Causes, read from the flags the engine actually sends. `validatorKind`
        // says which of the two validators moved; a row with the flag set and no
        // kind is an event written before the kind existed, and before the
        // timestamp was ever asked — so its validator can only have been an ETag.
        // A row with none of the flags set had its partial file deleted.
        detail: p['planChanged']
          ? 'announced length changed'
          : p['validatorChanged']
            ? p['validatorKind'] === 'last-modified'
              ? 'last-modified changed'
              : 'validator changed'
            : 'the partial file was gone',
        tone: 'warn',
      };
    case 'paused':
      return { label: 'Paused', detail: `${formatBytes(asNumber(p['verified']))} kept`, tone: 'muted' };
    case 'url-refreshed': {
      const oldHost = asText(p['oldHost']) ?? '?';
      const newHost = asText(p['newHost']) ?? '?';
      const kept = asNumber(p['keptBytes']);
      const validator = asText(p['validator']);
      const hostLabel = oldHost === newHost ? 'same host' : `${oldHost} → ${newHost}`;
      const keptLabel = kept && kept > 0 ? `${formatBytes(kept)} kept` : 'nothing kept';
      const validatorLabel =
        validator === 'etag'
          ? 'ETag matched'
          : validator === 'last-modified'
            ? 'date matched'
            : validator === 'sample'
              ? 'bytes matched'
              : 'length matched only';
      return {
        label: 'Link updated',
        detail: kept && kept > 0 ? `${formatBytes(kept)} kept` : 'resumed',
        tooltip: `${hostLabel} · ${keptLabel} · ${validatorLabel}`,
        tone: 'info',
      };
    }
    case 'tail-split': {
      const block = asNumber(p['block']) ?? '?';
      const rem = asNumber(p['bytesLeft']);
      return {
        label: 'Tail split',
        detail: `#${block} · ${rem !== null ? `${formatBytes(rem)} shared` : 'split with idle worker'}`,
        tone: 'info',
      };
    }
    case 'speed-limit': {
      const cap = asNumber(p['bytesPerSec']);
      return {
        label: 'Speed limit',
        detail: cap === null ? 'limit cleared' : `capped at ${formatBytes(cap)}/s`,
        tone: cap === null ? 'muted' : 'info',
      };
    }
    case 'throttled': {
      const streams = asNumber(p['streams']);
      const secs = asNumber(p['waitSecs']);
      const previous = asNumber(p['previous']);
      const ceiling = asNumber(p['ceiling']);
      return {
        label: 'Throttled',
        detail:
          streams === null || secs === null
            ? `rate-limited (HTTP ${asNumber(p['status']) ?? '?'})`
            : `rate-limit: holding ${streams} ${streams === 1 ? 'stream' : 'streams'} for ${secs}s`,
        tooltip:
          previous !== null && ceiling !== null
            ? `${previous} streams were in flight · ceiling now ${ceiling}, one step back per quiet minute`
            : undefined,
        tone: 'warn',
      };
    }
    case 'ramp-grow': {
      const from = asNumber(p['from']);
      const to = asNumber(p['to']);
      const bytes = asNumber(p['bytes']);
      return {
        label: 'Streams up',
        detail: `${from} → ${to} · ${bytes !== null ? formatBytes(bytes) : 'more'} in the last window`,
        tone: 'info',
      };
    }
    case 'ramp-raise':
      return {
        label: 'Ceiling back',
        detail: `a quiet minute · up to ${asNumber(p['ceiling']) ?? '?'} streams again`,
        tone: 'info',
      };
    case 'ramp-plateau':
      return {
        label: 'Streams steady',
        detail: `stopped at ${asNumber(p['streams']) ?? '?'} — another bought no more bytes`,
        tone: 'muted',
      };
    case 'ramp-final':
      return {
        label: 'Streams',
        detail: `${asNumber(p['width']) ?? '?'} at the finish · ceiling ${asNumber(p['ceiling']) ?? '?'} of ${asNumber(p['hard']) ?? '?'}`,
        tooltip:
          p['plateau'] === true
            ? 'growth stopped early: an extra stream would not have moved more bytes'
            : `${asNumber(p['workers']) ?? '?'} connections opened for this file`,
        tone: 'muted',
      };
    case 'block-retry':
      return {
        label: 'Block retry',
        detail: `#${asNumber(p['block']) ?? '?'} · ${asText(p['why']) ?? ''}`,
        tone: 'warn',
      };
    case 'block-failed':
      return {
        label: 'Block failed',
        detail: `#${asNumber(p['block']) ?? '?'} · ${asText(p['code']) ?? ''} · ${asText(p['why']) ?? ''}`,
        tone: 'error',
      };
    case 'stream-retry':
      return {
        label: 'Stream retry',
        detail: `attempt ${asNumber(p['attempt']) ?? '?'} · ${asText(p['why']) ?? ''}`,
        tone: 'warn',
      };
    case 'coverage-ok':
      return { label: 'Coverage ok', detail: `${formatBytes(asNumber(p['bytes']))} accounted for`, tone: 'ok' };
    case 'checksum-ok':
      return {
        label: 'Checksum ok',
        detail: `${asText(p['algo']) ?? ''} · ${(asText(p['digest']) ?? '').slice(0, 16)}…`,
        tone: 'ok',
      };
    case 'checksum-mismatch':
      return {
        label: 'Checksum mismatch',
        detail: `expected ${(asText(p['expected']) ?? '').slice(0, 12)}… got ${(asText(p['actual']) ?? '').slice(0, 12)}…`,
        tone: 'error',
      };
    case 'auto-verify':
      return {
        label: 'Auto-verify',
        detail: `${asText(p['algo']) ?? ''} · ${(asText(p['digest']) ?? '').slice(0, 16)}…`,
        tone: 'info',
      };
    case 'auto-verify-skipped':
      return { label: 'Auto-verify skipped', detail: asText(p['why']) ?? '', tone: 'muted' };
    case 'publish-preparing':
      return { label: 'Publish', detail: 'checking staged length', tone: 'info' };
    case 'publish-verifying':
      return { label: 'Verify', detail: `hashing ${asText(p['algo']) ?? ''}`, tone: 'info' };
    case 'publish-finalizing':
      return { label: 'Finalize', detail: asText(p['path']) ?? '', tone: 'info' };
    case 'published':
      return {
        label: 'Published',
        detail: `${asText(p['path']) ?? ''}${p['verified'] ? ' · verified' : ''}`,
        tone: 'ok',
      };
    case 'pair-picture':
      return {
        label: 'Picture kept',
        detail: `${formatBytes(asNumber(p['bytes']))} for the sound to join`,
        tooltip: asText(p['path']) ?? '',
        tone: 'info',
      };
    case 'merge-start':
      return {
        label: 'Joining',
        detail: `sound finished · ${formatBytes(asNumber(p['base']))} of picture + sound on disk`,
        tooltip: asText(p['out']) ?? '',
        tone: 'info',
      };
    case 'merged':
      return {
        label: 'Joined',
        detail: `${formatBytes(asNumber(p['bytes']))} in one file`,
        tooltip: asText(p['path']) ?? '',
        tone: 'ok',
      };
    case 'capture-adopted':
      return {
        label: 'Taken from the browser',
        detail: `${formatBytes(asNumber(p['bytes']))} · ${asText(p['source']) ?? 'capture'}`,
        tooltip: asText(p['path']) ?? '',
        tone: 'info',
      };
    case 'capture-imported':
      return {
        label: 'Handed over',
        detail: `${formatBytes(asNumber(p['bytes']))} of ${formatBytes(asNumber(p['total']))} already here · ${asNumber(p['blocks']) ?? '?'} chunks planned`,
        tooltip: asText(p['source']) ?? '',
        tone: 'info',
      };
    case 'zone-skipped':
      return { label: 'Mark-of-the-Web skipped', detail: asText(p['why']) ?? '', tone: 'warn' };
    case 'failed':
      return {
        label: 'Failed',
        detail: `${asText(p['code']) ?? ''} · ${asText(p['message']) ?? ''}`,
        tone: 'error',
      };
    default:
      return {
        label: kind,
        detail: raw ? JSON.stringify(raw).slice(0, 120) : '',
        tone: 'muted',
      };
  }
}

async function renderActivity(id: number): Promise<void> {
  const events = await sucr.history.tail(id, 20).catch(() => []);
  if (events.length === 0) {
    activityEl.replaceChildren(
      Object.assign(document.createElement('p'), {
        className: 'pane-empty',
        textContent: 'Nothing recorded yet for this download.',
      }),
    );
    return;
  }
  const list = document.createElement('ol');
  list.className = 'trail';
  for (const event of events) {
    const view = formatEvent(event.kind, event.payload);
    const li = document.createElement('li');
    const time = document.createElement('time');
    time.dateTime = new Date(event.ts).toISOString();
    time.textContent = formatClock(event.ts);
    const pill = document.createElement('span');
    pill.className = 'pill';
    pill.dataset['tone'] = view.tone;
    pill.textContent = view.label;
    const what = document.createElement('span');
    what.className = 'trail-detail';
    what.textContent = view.detail;
    what.title = view.tooltip ?? view.detail;
    li.append(time, pill, what);
    list.append(li);
  }
  activityEl.replaceChildren(list);
}

let inspectorSeq = 0;

async function updateInspector(): Promise<void> {
  const seq = ++inspectorSeq;
  const id = selectedId;
  if (id === null) {
    ihSub.textContent = 'nothing selected';
    detailEl.replaceChildren(
      Object.assign(document.createElement('p'), {
        className: 'pane-empty',
        textContent: 'Progress, per-network contribution and file identity appear here once a download is selected.',
      }),
    );
    for (const el of [sourceEl, scheduleEl, filesEl, activityEl]) {
      el.replaceChildren(
        Object.assign(document.createElement('p'), {
          className: 'pane-empty',
          textContent: 'Select a download to fill this pane.',
        }),
      );
    }
    return;
  }
  const d = await sucr.download.get(id).catch(() => null);
  if (seq !== inspectorSeq || selectedId !== id) return;
  if (!d) {
    selectedId = null;
    ihSub.textContent = 'nothing selected';
    return;
  }
  ihSub.textContent = `#${d.id} · ${d.filename}`;
  ihSub.title = d.filename;
  updateOverview(d);
  updateSource(d);
  updateSchedule(d);
  updateFiles(d);
  await renderActivity(d.id);
}

function selectDownload(id: number | null): void {
  selectedId = id;
  // Row buttons swallow pointerdown, so the outside-click handler never sees
  // them; without this the prompt would float over an unrelated download.
  closeRemovePopover();
  if (id !== null && drawerQuery.matches) setDrawer(true);
  renderRows();
  // Refresh is event-driven, and a finished row produces no events: without
  // this the dead-link probe would only ever run while something else was
  // transferring, which is exactly when nobody looks at an old download.
  void checkSelectedLink(id !== null ? library.find((d) => d.id === id) : undefined);
  void syncMap();
  void updateInspector();
}

/* --- refresh loop -------------------------------------------------------- */
let refreshing = false;
let refreshTimer: number | undefined;

/** A started, paused or resumed job swaps what `watch` subscribes to: the live
 *  map on download, the database rows once nothing is running. Re-subscribe
 *  exactly on status transitions so the pane never freezes on a dead map. */
async function syncMapStatus(d: Download): Promise<void> {
  if (mapWatchId !== d.id || mapStatus === d.status) return;
  mapStatus = d.status;
  await rewatchMap(d.id);
}

async function refresh(): Promise<void> {
  if (refreshing) return;
  refreshing = true;
  try {
    library = await sucr.download.list();
    if (selectedId !== null && !library.some((d) => d.id === selectedId)) selectedId = null;
    if (selectedId === null && library.length > 0 && !isSystemView(currentView)) {
      selectedId = library[0]?.id ?? null;
    }
    totals = sampleAll();
    detectCompletions();
    renderRows();
    updateCounts();
    const current = selectedId !== null ? library.find((d) => d.id === selectedId) : undefined;
    if (current) await syncMapStatus(current);
    void checkSelectedLink(current);
    void syncMap();
    await updateInspector();
  } catch (err) {
    console.error(err);
  } finally {
    refreshing = false;
  }
}

function scheduleRefresh(): void {
  if (refreshTimer !== undefined) return;
  refreshTimer = window.setTimeout(() => {
    refreshTimer = undefined;
    void refresh();
  }, 150);
}

/** The adapter list changes when hardware or a switch changes, and not with
 *  download progress, so it is re-read on those two events rather than in the
 *  4 Hz loop: `network:list` asks Windows for the adapter table and writes a
 *  row per device. */
let netReadAt = 0;

/* A card that says "Over allowance" is claiming something about the engine's
   live bucket, and the usage figures behind that claim only move during a
   transfer. So while a limit-bearing surface is actually on screen and bytes are
   moving, the deck re-reads on a five-second floor — the same cadence the Rust
   watcher keeps its own mirror on. */
function maybeRefreshNetworks(): void {
  const deckOpen =
    networksViewEl.hidden === false || $<HTMLElement>('#nic-modal-backdrop')?.hidden === false;
  if (!deckOpen) return;
  if (Date.now() - netReadAt < 5000) return;
  void refreshNetworks();
}

async function refreshNetworks(): Promise<void> {
  // Stamped before the await: the repaint this triggers reaches `updateCounts`,
  // and that is where the live-deck refresh is asked for. Without the floor set
  // on entry the two calls would chase each other.
  netReadAt = Date.now();
  try {
    const list = await sucr.network.list();
    networks = list.adapters;
    unattributed = list.unattributed;
  } catch (err) {
    console.error(err);
    return;
  }
  updateCounts();
  await updateInspector();
  const modal = $<HTMLElement>('#nic-modal-backdrop');
  if (modal && !modal.hidden) {
    renderNicModal();
  }
}

/* --- add bar ------------------------------------------------------------- */
const addBtn = $<HTMLButtonElement>('#add');
const addBar = $<HTMLFormElement>('#add-bar');
const addUrl = $<HTMLInputElement>('#add-url');
const addError = $<HTMLElement>('#add-error');
const addFolder = $<HTMLInputElement>('#add-folder');
const addFolderBrowse = $<HTMLButtonElement>('#add-folder-browse');
const addFolderReset = $<HTMLButtonElement>('#add-folder-reset');
const addSubmit = $<HTMLButtonElement>('#add-submit');
const addFolderPresets = $<HTMLElement>('#add-folder-presets');

/* --- link check ----------------------------------------------------------- */
/* The verdict comes from the engine's own pre-flight probe, so "greyed out" here
   means the same thing it means during a transfer: this engine cannot pull this
   link. No client-side guess about file names or extensions. */

/** One input + its submit button + the line the verdict is written to. The add
 *  bar and the update-link popover share the whole state machine, so a link is
 *  judged identically wherever it is pasted. `linkOk` is remembered rather than
 *  written straight to `disabled`, because the add bar has a second reason to
 *  stay dark — a save-to folder that does not exist yet — and the two must not
 *  overwrite each other. */
type LinkCheck = {
  input: HTMLInputElement;
  submit: HTMLButtonElement;
  line: HTMLElement;
  timer: number;
  checking: string;
  linkOk: boolean;
  extraBlock?: () => boolean;
};

function syncSubmit(c: LinkCheck): void {
  c.submit.disabled = !c.linkOk || (c.extraBlock?.() ?? false);
}

const addCheck: LinkCheck = {
  input: addUrl, submit: addSubmit, line: addError, timer: 0, checking: '', linkOk: true,
};
const refreshCheck: LinkCheck = {
  input: refreshUrlInput, submit: refreshUrlSubmit, line: refreshUrlError, timer: 0, checking: '',
  linkOk: true,
};

/** The engine's codes are precise but written for a log, not for a person who is
 *  mid-typing. The sentence goes in the line, the raw message in `title`, so the
 *  detail is one hover away instead of being thrown away. */
function verdictLine(v: LinkProbe): string {
  if (v.ok) return '';
  switch (v.code) {
    case 'scheme':
      return 'Only http and https links can be downloaded.';
    case 'blocked':
      return `Edge protection is blocking this link${v.status ? ` (${v.status})` : ''}.`;
    case 'too-many-redirects':
      return 'This link redirects in a loop.';
    case 'needs-auth':
      // The two readings are the engine's, not a guess made here: `auth` says
      // whether a credential was on the request that came back 401. The noun covers
      // both kinds the store holds, because a probe carries no kind to branch on.
      return v.auth
        ? 'The credential stored for this host was refused (401).'
        : 'This link needs a credential (401).';
    case 'http-status':
      switch (v.status) {
        case 403: return 'The server refuses this download (403).';
        case 404: return 'There is nothing at this link (404).';
        default:
          if ((v.status ?? 0) >= 500) return `The server is failing (${v.status}).`;
          return v.status ? `The server answered ${v.status}.` : 'The server refused this link.';
      }
    case 'io':
      return 'No connection to that server.';
    default:
      return v.message || 'This link cannot be downloaded.';
  }
}

function setChecking(c: LinkCheck): void {
  c.linkOk = false;
  syncSubmit(c);
  c.line.textContent = 'Checking link…';
  c.line.classList.add('is-hint');
  c.line.title = '';
  c.line.hidden = false;
}

function setVerdict(c: LinkCheck, v: LinkProbe): void {
  c.linkOk = v.ok;
  syncSubmit(c);
  c.line.classList.remove('is-hint');
  c.line.textContent = v.ok ? '' : verdictLine(v);
  c.line.title = v.ok ? '' : v.message ?? '';
  c.line.hidden = v.ok;
}

function clearVerdict(c: LinkCheck): void {
  window.clearTimeout(c.timer);
  c.checking = '';
  c.linkOk = true;
  syncSubmit(c);
  c.line.classList.remove('is-hint');
  c.line.hidden = true;
}

async function runCheck(c: LinkCheck, url: string): Promise<void> {
  c.checking = url;
  setChecking(c);
  let verdict: LinkProbe;
  try {
    verdict = await sucr.download.probe(url);
  } catch {
    // The check itself failed. That is this app's problem, not evidence about
    // the user's link, so it must never leave the button stranded.
    if (c.checking === url) clearVerdict(c);
    return;
  }
  if (c.checking !== url) return;
  setVerdict(c, verdict);
}

function wireLinkCheck(c: LinkCheck): void {
  c.input.addEventListener('input', () => {
    window.clearTimeout(c.timer);
    const link = c.input.value.trim();
    if (!link) clearVerdict(c);
    else c.timer = window.setTimeout(() => void runCheck(c, link), 550);
  });
}

wireLinkCheck(addCheck);
wireLinkCheck(refreshCheck);

/* --- is the source of a finished download still alive? --------------------- */
/* The redownload button is the one control that promises a file can be pulled
   again. It is judged by the same probe, but lazily: only the selected download
   is checked, once per selection, so a long list costs no network chatter and a
   link that came back alive is re-checked the next time the row is picked. */
const linkVerdicts = new Map<string, LinkProbe>();
let linkCheckedId: number | null = null;

function isDone(d: Download): boolean {
  return d.status === 'finished' || d.status === 'verified';
}

async function checkSelectedLink(current: Download | undefined): Promise<void> {
  if (!current || !isDone(current)) {
    linkCheckedId = null;
    return;
  }
  if (linkCheckedId === current.id) return;
  linkCheckedId = current.id;
  const verdict = await sucr.download.probe(current.url).catch(() => null);
  if (!verdict || selectedId !== current.id) return;
  const before = linkVerdicts.get(current.url);
  linkVerdicts.set(current.url, verdict);
  const dead = !verdict.ok;
  const wasDead = before !== undefined ? !before.ok : false;
  if (dead !== wasDead) renderRows();
}

function getCategorySubfolder(cat: string): string | null {
  switch (cat) {
    case 'archive': return 'Archives';
    case 'video': return 'Videos';
    case 'audio': return 'Audio';
    case 'document': return 'Documents';
    case 'program': return 'Programs';
    default: return null;
  }
}

function resolveCategoryPath(url: string, baseFolder?: string): string {
  const base = (baseFolder ?? getDefaultDownloadFolder()).trim();
  if (!base) return '';
  const isSorting = localStorage.getItem('sucr.category_sorting') !== 'false';
  if (!isSorting) return base;

  const urlClean = url.split(/[#?]/)[0] || '';
  const parts = urlClean.split('/').filter(Boolean);
  const rawFilename = parts.pop() || '';
  const fileParts = rawFilename.split('.').filter(Boolean);
  const ext = fileParts.length > 1 ? fileParts.pop()!.toLowerCase() : '';
  const compound =
    fileParts.length > 1 &&
    ['tar.gz', 'tar.xz', 'tar.bz2', 'tar.zst'].includes(
      `${fileParts[fileParts.length - 1]}.${ext}`,
    );
  const cat = CATEGORY_OF[compound ? 'tar' : ext];
  const sub = cat ? getCategorySubfolder(cat) : null;
  if (!sub) return base;

  const sep = base.includes('/') ? '/' : '\\';
  return base.endsWith(sep) ? `${base}${sub}` : `${base}${sep}${sub}`;
}

let folderUserOverridden = false;

function getDefaultDownloadFolder(): string {
  return setting('storage.default_folder') || appInfo?.paths.downloadsDir || '';
}

/** The three quick folders the add bar offers, and the label each carries. */
type PresetId = 'downloads' | 'desktop' | 'documents';

const PRESET_LABELS: Record<PresetId, string> = {
  downloads: 'Downloads',
  desktop: 'Desktop',
  documents: 'Documents',
};

/** The folders that exist, in chip order. The main process scans the drive the
 *  default folder sits on and then the profile, so a `D:\` default with no
 *  `D:\Desktop` offers the Desktop that is really there — and a name with no
 *  directory anywhere gets no chip at all. */
let folderPresets: Array<{ preset: PresetId; path: string }> = [];
let presetsResolvedFor: string | null = null;
let presetsToken = 0;

/** Windows paths ignore case and a trailing slash, and the field may hold either
 *  separator, so presets match on a normalised form rather than byte equality. */
function sameFolder(a: string, b: string): boolean {
  const norm = (s: string) => s.trim().replace(/[\\/]+$/, '').replace(/\//g, '\\').toLowerCase();
  const left = norm(a);
  return left !== '' && left === norm(b);
}

function presetChips(): HTMLButtonElement[] {
  return Array.from(
    addFolderPresets.querySelectorAll<HTMLButtonElement>('.add-preset-chip[data-preset]'),
  );
}

/** Chips are built from what `loadFolderPresets` resolved, before the Reset chip
 *  that stays in the markup. The tooltip is the real path: a chip labelled
 *  Desktop can live on another drive than the one being downloaded to. */
function renderFolderPresets(): void {
  for (const chip of presetChips()) chip.remove();
  for (const entry of folderPresets) {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'add-preset-chip';
    chip.dataset['preset'] = entry.preset;
    chip.textContent = PRESET_LABELS[entry.preset];
    chip.title = entry.path;
    chip.setAttribute('aria-pressed', 'false');
    addFolderPresets.insertBefore(chip, addFolderReset);
  }
}

async function loadFolderPresets(base: string): Promise<void> {
  if (presetsResolvedFor === base) return;
  const token = ++presetsToken;
  let list: Array<{ preset: PresetId; path: string }>;
  try {
    list = await sucr.app.getFolderPresets(base);
  } catch (err) {
    addError.textContent = `Could not read the quick folders: ${err instanceof Error ? err.message : String(err)}`;
    addError.hidden = false;
    return;
  }
  if (token !== presetsToken) return; // a newer default folder answered first
  folderPresets = list;
  presetsResolvedFor = base;
  renderFolderPresets();
  updatePresetStates(addFolder.value);
}

/** One chip pressed, never two: the field is matched against the resolved paths
 *  and only the chip that owns that folder answers. An empty field means the
 *  default folder, which is whatever the Downloads chip resolved to. */
function updatePresetStates(currentFolder: string): void {
  const target = currentFolder.trim() || getDefaultDownloadFolder();
  const active = folderPresets.find((entry) => sameFolder(target, entry.path))?.preset ?? '';

  for (const chip of presetChips()) {
    chip.setAttribute('aria-pressed', String(chip.dataset['preset'] === active));
  }

  const isCustom = currentFolder.trim() !== '' && !sameFolder(currentFolder, getDefaultDownloadFolder());
  addFolderReset.hidden = !folderUserOverridden && !isCustom;
}

/* --- a folder that is not there yet --------------------------------------- */
/* One line and one button, shared by the add bar and the setup wizard. The shell
   decides whether the folder exists; the button makes it. A path the user typed
   is never created silently, because a typo'd `D:\Downlods` would otherwise
   become a real folder and every download would land inside it. */
type FolderEnsure = {
  input: HTMLInputElement;
  notice: HTMLElement;
  text: HTMLElement;
  button: HTMLButtonElement;
  missing: string | null;
  timer: number;
  token: number;
  onBlockedChange?: (missing: string | null) => void;
  onCreated?: (path: string) => void;
};

function setMissingFolder(f: FolderEnsure, path: string | null): void {
  f.missing = path;
  f.notice.hidden = path === null;
  f.text.textContent = path ? `${path} does not exist yet.` : '';
  f.onBlockedChange?.(path);
}

async function probeFolder(f: FolderEnsure, raw: string): Promise<void> {
  const path = raw.trim();
  // An empty field means the default download folder, which the shell owns and
  // already creates, so there is nothing to offer.
  if (!path) {
    setMissingFolder(f, null);
    return;
  }
  const token = ++f.token;
  let state: { path: string; exists: boolean; isDir: boolean } | null;
  try {
    state = await sucr.app.folderState(path);
  } catch {
    // A failed probe is this app's problem, not evidence about the user's folder:
    // it must never leave the button dark with nothing to click.
    setMissingFolder(f, null);
    return;
  }
  if (token !== f.token) return; // a newer keystroke answered first
  setMissingFolder(f, state && !state.isDir ? state.path : null);
}

async function createMissingFolder(f: FolderEnsure): Promise<void> {
  const typed = f.input.value.trim();
  // An empty field names the default folder, which the shell owns: nothing to make.
  if (!typed) return;
  window.clearTimeout(f.timer);
  // A click can beat the debounce, and creating the last probed path then would
  // make a folder nobody aimed at and write it over the one in the field. So the
  // button answers for the text the user is looking at, and asks the disk again.
  if (typed !== f.missing) {
    await probeFolder(f, typed);
    if (f.missing === null) return; // it appeared, or the probe could not tell
  }
  const target = f.missing;
  f.button.disabled = true;
  let res: { path: string; created: boolean; alreadyPresent: boolean; error: string | null };
  try {
    res = await sucr.app.createFolder(target);
  } catch (err) {
    f.button.disabled = false;
    f.text.textContent = `Could not create it: ${err instanceof Error ? err.message : String(err)}`;
    return;
  }
  f.button.disabled = false;
  if (res.error) {
    // Still missing, still blocked, and the disk's own reason is on screen.
    f.text.textContent = res.error;
    return;
  }
  f.input.value = res.path;
  setMissingFolder(f, null);
  f.onCreated?.(res.path);
}

function wireFolderEnsure(f: FolderEnsure): void {
  f.input.addEventListener('input', () => {
    window.clearTimeout(f.timer);
    f.timer = window.setTimeout(() => void probeFolder(f, f.input.value), 400);
  });
  f.button.addEventListener('click', () => void createMissingFolder(f));
}

const addFolderNotice = $<HTMLElement>('#add-folder-notice');
const addFolderNoticeText = $<HTMLElement>('#add-folder-notice-text');
const addFolderCreate = $<HTMLButtonElement>('#add-folder-create');

const addFolderEnsure: FolderEnsure = {
  input: addFolder,
  notice: addFolderNotice,
  text: addFolderNoticeText,
  button: addFolderCreate,
  missing: null,
  timer: 0,
  token: 0,
  onBlockedChange: () => syncSubmit(addCheck),
  onCreated: (path) => {
    // A folder that did not exist a second ago can change which preset resolves
    // where — `D:\Documents` created from the add bar is now a real D: chip.
    folderUserOverridden = true;
    updatePresetStates(path);
    presetsResolvedFor = null;
    void loadFolderPresets(getDefaultDownloadFolder());
  },
};
addCheck.extraBlock = () => addFolderEnsure.missing !== null;
wireFolderEnsure(addFolderEnsure);

function openAdd(open: boolean): void {
  addBar.hidden = !open;
  addBtn.setAttribute('aria-expanded', String(open));
  if (open) {
    addError.hidden = true;
    folderUserOverridden = false;
    const defaultFolder = getDefaultDownloadFolder();
    const url = addUrl.value.trim();
    const targetFolder = (url ? resolveCategoryPath(url, defaultFolder) : '') || defaultFolder;
    addFolder.value = targetFolder;
    addFolder.placeholder = targetFolder || defaultFolder || 'Default download folder';
    updatePresetStates(targetFolder);
    void loadFolderPresets(defaultFolder);
    void probeFolder(addFolderEnsure, targetFolder);
    addUrl.focus();
    if (url) void runCheck(addCheck, url);
    else clearVerdict(addCheck);
  }
}

addBtn.addEventListener('click', () => openAdd(addBar.hidden));
$('#add-cancel').addEventListener('click', () => {
  openAdd(false);
  addBtn.focus();
});

addUrl.addEventListener('keydown', (event) => {
  if ((event as KeyboardEvent).key === 'Escape') {
    openAdd(false);
    addBtn.focus();
  }
});

addFolder.addEventListener('keydown', (event) => {
  if ((event as KeyboardEvent).key === 'Escape') {
    openAdd(false);
    addBtn.focus();
  }
});

// One listener for chips that are built at runtime: the click is matched on the
// container. Reset keeps its own listener — it has no `data-preset`, so the
// `closest` lookup answers null and this handler stays out of its way.
addFolderPresets.addEventListener('click', (event) => {
  const chip = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>(
    '.add-preset-chip[data-preset]',
  );
  if (!chip) return;

  const entry = folderPresets.find((preset) => preset.preset === chip.dataset['preset']);
  if (!entry) return;
  folderUserOverridden = true;
  addFolder.value = entry.path;
  updatePresetStates(entry.path);
  void probeFolder(addFolderEnsure, entry.path);
});

addFolderReset.addEventListener('click', () => {
  folderUserOverridden = false;
  localStorage.removeItem('sucr.last_download_folder');
  const defaultBase = getDefaultDownloadFolder();
  const url = addUrl.value.trim();
  const target = (url ? resolveCategoryPath(url, defaultBase) : '') || defaultBase;
  addFolder.value = target;
  addFolder.placeholder = target || 'Default download folder';
  updatePresetStates(target);
  void probeFolder(addFolderEnsure, target);
});

addFolder.addEventListener('input', () => {
  folderUserOverridden = true;
  updatePresetStates(addFolder.value);
});

addFolder.addEventListener('change', () => {
  const trimmed = addFolder.value.trim();
  if (trimmed) {
    localStorage.setItem('sucr.last_download_folder', trimmed);
  }
});

addFolderBrowse.addEventListener('click', async () => {
  addFolderBrowse.classList.remove('animating');
  void addFolderBrowse.offsetWidth;
  addFolderBrowse.classList.add('animating');
  window.setTimeout(() => addFolderBrowse.classList.remove('animating'), 200);

  // Ask the shell, not the webview. showDirectoryPicker() hands back a handle
  // that deliberately withholds the absolute path, and <input webkitdirectory>
  // only yields a relative name — either way the field would end up with a
  // folder the user never chose. The main process owns the real path.
  addError.hidden = true;
  let picked: string | null;
  try {
    picked = await sucr.app.pickFolder();
  } catch {
    addError.textContent = 'Could not open the folder picker.';
    addError.hidden = false;
    return;
  }
  if (!picked) return; // cancelled: keep whatever was there
  folderUserOverridden = true;
  addFolder.value = picked;
  updatePresetStates(picked);
  void probeFolder(addFolderEnsure, picked);
  localStorage.setItem('sucr.last_download_folder', picked);
});

addUrl.addEventListener('input', () => {
  if (folderUserOverridden) return;
  const url = addUrl.value.trim();
  const defaultBase = getDefaultDownloadFolder();
  if (!defaultBase) return;
  const isSorting = localStorage.getItem('sucr.category_sorting') !== 'false';
  if (isSorting && url) {
    const catPath = resolveCategoryPath(url, defaultBase);
    addFolder.value = catPath || defaultBase;
    addFolder.placeholder = catPath || defaultBase;
    updatePresetStates(addFolder.value);
  } else {
    addFolder.value = defaultBase;
    addFolder.placeholder = defaultBase;
    updatePresetStates(defaultBase);
  }
  // A category path is derived, so it can name a subfolder that was never made.
  void probeFolder(addFolderEnsure, addFolder.value);
});

addBar.addEventListener('submit', (event) => {
  event.preventDefault();
  // The button may also be disabled by implicit submission, so the guard lives
  // here as well as in the button state: Enter must not be a way past a dead link.
  if (addSubmit.disabled) return;
  const url = addUrl.value.trim();
  addError.hidden = true;
  if (!url) return;

  let folder: string | undefined = addFolder.value.trim() || undefined;
  if (folderUserOverridden && folder) {
    localStorage.setItem('sucr.last_download_folder', folder);
  } else {
    folder = resolveCategoryPath(url) || folder || getDefaultDownloadFolder() || undefined;
  }

  void (async () => {
    try {
      const row = await sucr.download.add({ url, folder: folder || undefined });
      addUrl.value = '';
      openAdd(false);
      selectedId = row.id;
      if (settingOn('engine.auto_start')) await sucr.download.start(row.id);
      await refresh();
    } catch (err) {
      addError.textContent = err instanceof Error ? err.message : String(err);
      addError.hidden = false;
    }
  })();
});

/* --- search -------------------------------------------------------------- */
$<HTMLInputElement>('#search').addEventListener('input', (event) => {
  query = (event.target as HTMLInputElement).value.trim().toLowerCase();
  renderRows();
});

/* --- navigation: filters the rows already in memory ---------------------- */
const navItems = $$<HTMLButtonElement>('.nav-item');

const VIEW_SUB: Record<string, string> = {
  all: 'Every connection, every block, why it chose it.',
  downloading: 'Live transfers, block by block.',
  queued: 'Waiting for a slot or a resume.',
  finished: 'Completed transfers and their checksums.',
  attention: 'Jobs that need a decision before they continue.',
  networks: 'Connected network adapters, bonding telemetry & multi-device routing.',
  settings: 'Engine tuning, storage allocation, network bonding & appearance.',
};

function renderCategoriesList(): void {
  const list = $<HTMLUListElement>('#categories');
  if (!list) return;

  const isSorting = localStorage.getItem('sucr.category_sorting') !== 'false';
  if (!isSorting && categories.length === 0) {
    if (!list.querySelector('.nav-empty')) {
      const empty = document.createElement('li');
      empty.className = 'nav-empty';
      empty.textContent = 'None yet';
      list.replaceChildren(empty);
    }
    return;
  }

  const items: Array<{ view: string; title: string; count: number }> =
    categories.length > 0
      ? categories.map((c) => ({
          view: `category:${c.id}`,
          title: c.name,
          count: library.filter((d) => d.categoryId === c.id).length,
        }))
      : [
          { id: 'archive', name: 'Archives' },
          { id: 'video', name: 'Videos' },
          { id: 'audio', name: 'Audio' },
          { id: 'document', name: 'Documents' },
          { id: 'program', name: 'Programs' },
        ].map((cat) => ({
          view: `category:${cat.id}`,
          title: cat.name,
          count: library.filter((d) => {
            const ext = d.filename.split('.').pop()?.toLowerCase() || '';
            return (CATEGORY_OF[ext] ?? 'misc') === cat.id;
          }).length,
        }));

  const existingButtons = Array.from(list.querySelectorAll<HTMLButtonElement>('.nav-item'));
  const structureMatches =
    existingButtons.length === items.length &&
    existingButtons.every(
      (btn, i) =>
        btn.dataset['view'] === items[i]!.view &&
        btn.dataset['title'] === items[i]!.title,
    );

  if (structureMatches) {
    // In-place reconciliation: keep DOM buttons stable so ongoing clicks/hover states never drop
    for (let i = 0; i < items.length; i++) {
      const btn = existingButtons[i]!;
      const item = items[i]!;
      const isCurrent = currentView === item.view;
      if (btn.getAttribute('aria-current') !== (isCurrent ? 'true' : null)) {
        if (isCurrent) btn.setAttribute('aria-current', 'true');
        else btn.removeAttribute('aria-current');
      }
      const countEl = btn.querySelector<HTMLElement>('.nav-count');
      const countStr = String(item.count);
      if (countEl && countEl.textContent !== countStr) {
        countEl.textContent = countStr;
      }
    }
    return;
  }

  list.replaceChildren(
    ...items.map((item) => {
      const li = document.createElement('li');
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'nav-item';
      button.dataset['view'] = item.view;
      button.dataset['title'] = item.title;
      if (currentView === item.view) button.setAttribute('aria-current', 'true');

      const label = document.createElement('span');
      label.className = 'nav-text';
      label.textContent = item.title;

      const countBadge = document.createElement('span');
      countBadge.className = 'nav-count';
      countBadge.textContent = String(item.count);

      button.append(label, countBadge);
      li.append(button);
      return li;
    }),
  );
}

function selectView(view: string, title: string): void {
  /* The panel is hover- and focus-driven only. A nav click used to pin it open
     for the session so the selected row's label stayed readable; the ruling on
     2026-11-03 is that it must slide back once the pointer leaves, so the pin
     and the class that carried it are gone. */
  currentView = view;
  for (const item of navItems) {
    item.setAttribute('aria-current', String(item.dataset['view'] === view));
  }
  viewTitleEl.textContent = title;
  viewSubEl.textContent =
    VIEW_SUB[view] ?? (view.startsWith('category:') ? 'Files filed under this category.' : '');
  document.querySelectorAll<HTMLButtonElement>('#categories .nav-item').forEach((item) => {
    item.setAttribute('aria-current', String(item.dataset['view'] === view));
  });
  if (isSystemView(view)) openAdd(false);
  renderRows();
  updateCounts();
  if (view === 'settings') {
    setPillar('engine', false);
    renderSettings();
    void refreshDebugLog();
  } else if (view === 'networks') {
    renderNetworksView();
  }
}

// Delegated so category buttons created after boot behave like the rest.
let railPointerHandled = false;

$('.rail').addEventListener('pointerdown', (event) => {
  const e = event as PointerEvent;
  if (e.button !== 0) return;
  const target = e.target as Element | null;
  if (target && target.closest('.toggle-switch')) return;
  const item = target && typeof target.closest === 'function' ? target.closest('.nav-item') : null;
  if (!(item instanceof HTMLElement)) return;
  railPointerHandled = true;
  selectView(item.dataset['view'] ?? 'all', item.dataset['title'] ?? 'All downloads');
});

$('.rail').addEventListener('click', (event) => {
  if (railPointerHandled) {
    railPointerHandled = false;
    return;
  }
  const target = event.target as Element | null;
  if (target && target.closest('.toggle-switch')) return;
  const item = target && typeof target.closest === 'function' ? target.closest('.nav-item') : null;
  if (!(item instanceof HTMLElement)) return;
  selectView(item.dataset['view'] ?? 'all', item.dataset['title'] ?? 'All downloads');
});

/* --- network bonding deck ------------------------------------------------- */
function renderNicDeck(): void {
  const state = $<HTMLElement>('#nic-state');
  const light = state.querySelector('span');
  const live = library.some((d) => d.status === 'downloading');
  document.body.dataset['engine'] = live ? 'live' : 'idle';

  const rows = $<HTMLElement>('#nic-rows');
  const fragment = document.createDocumentFragment();

  if (networks.length === 0) {
    if (light) light.textContent = live ? '1 active' : '1 ready';

    const row = document.createElement('div');
    row.className = 'nic-row';
    row.dataset['net'] = 'n1';
    row.dataset['enabled'] = 'true';
    const dot = document.createElement('i');
    dot.className = 'dot';
    const name = document.createElement('span');
    name.className = 'nic-name';
    name.textContent = 'Primary Network';
    const right = document.createElement('div');
    right.className = 'nic-row-right';
    const kind = document.createElement('span');
    kind.className = 'nic-kind-badge';
    kind.textContent = 'DEFAULT';
    right.append(kind);
    row.append(dot, name, right);
    fragment.append(row);

    const hint = document.createElement('p');
    hint.className = 'nic-hint';
    hint.textContent = 'Plug in USB tethering or Ethernet to bond';
    fragment.append(hint);
    rows.replaceChildren(fragment);
    return;
  }

  // The deck's number is the pool the engine binds, not the count of plugs.
  const enabled = networks.filter((n) => n.enabled).length;
  const bonded = bondable(networks);
  if (light) {
    light.textContent =
      bonded > 1
        ? `${bonded} bonded`
        : bonded === 1
          ? live
            ? '1 active'
            : '1 ready'
          : enabled > 0
            ? 'all held back'
            : 'all disabled';
  }

  networks.slice(0, 4).forEach((net, i) => {
    const row = document.createElement('div');
    row.className = 'nic-row';
    row.dataset['net'] = net.colorId ?? `n${i + 1}`;
    row.dataset['enabled'] = String(net.enabled);
    row.dataset['held'] = String(net.enabled && guardHeld(net));
    const dot = document.createElement('i');
    dot.className = 'dot';
    dot.style.background = `var(--${net.colorId ?? 'n1'})`;
    const name = document.createElement('span');
    name.className = 'nic-name';
    name.textContent = net.displayName;
    name.title = net.displayName;

    const right = document.createElement('div');
    right.className = 'nic-row-right';
    if (net.usedToday > 0) {
      const bytes = document.createElement('span');
      bytes.className = 'nic-row-bytes';
      bytes.textContent = formatBytes(net.usedToday);
      right.append(bytes);
    }
    const kind = document.createElement('span');
    kind.className = 'nic-kind-badge';
    kind.textContent = !net.enabled
      ? `${net.kind.toUpperCase()} OFF`
      : guardHeld(net)
        ? 'HELD'
        : net.kind.toUpperCase();
    if (net.enabled && guardHeld(net)) {
      kind.title = 'Kept out of the bond by the metered guard';
    }
    right.append(kind);

    row.append(dot, name, right);
    fragment.append(row);
  });

  const hint = document.createElement('p');
  hint.className = 'nic-hint' + (bonded > 1 ? ' nic-hint--bonded' : '');
  hint.textContent =
    bonded > 1
      ? live
        ? `Aggregating ${bonded} devices · ${formatBytes(totals.bps)}/s`
        : `${bonded} devices bonded & ready`
      : bonded === 1
      ? 'Plug in USB tethering or Ethernet to bond'
      : enabled > 0
        ? 'The metered guard holds every connection out of the bond'
        : 'All network connections are disabled';
  fragment.append(hint);

  rows.replaceChildren(fragment);
}

/** Bytes per device, as the engine measured them.
 *
 *  While a job is watched its attempt rows are the only real split we have, so
 *  they win. With nothing running the card falls back to the day's total the
 *  engine booked, and with neither the bar stays at zero: an equal split was
 *  never measured and would be a made-up number. */
function perNetworkBytes(): Map<string, number> {
  const live = new Map<string, number>();
  for (const attempt of mapAttempts.values()) {
    if (attempt.bytes > 0) {
      live.set(attempt.networkId, (live.get(attempt.networkId) ?? 0) + attempt.bytes);
    }
  }
  if (live.size > 0) return live;
  const day = new Map<string, number>();
  for (const net of networks) {
    if (net.colorId && net.usedToday > 0) day.set(net.colorId, net.usedToday);
  }
  return day;
}

function netMetaItem(label: string, value: string): HTMLElement {
  const item = document.createElement('span');
  item.className = 'net-meta-item';
  item.append(`${label}: `);
  const bold = document.createElement('b');
  bold.textContent = value;
  item.append(bold);
  return item;
}

function netBadge(kind: string): string {
  if (kind === 'usb') return 'net-badge-usb';
  if (kind === 'ethernet') return 'net-badge-eth';
  return 'net-badge-wifi';
}

/* Whether the metered guard holds this card out of the bond.

   The engine applies the same filter when it builds a job's routes; the deck has
   to agree with it, because a card drawn as "Bonded" that never carries a byte is
   the same kind of lie as a bandwidth label that names the wrong number. */
function guardHeld(net: Network): boolean {
  return net.kind === 'cellular' && settingOn('network.metered_guard');
}

function bondable(list: Network[]): number {
  return list.filter((n) => n.enabled && !guardHeld(n)).length;
}

/* --- per-adapter limits (step 6) ------------------------------------------ */

const netLimitsPopoverEl = $<HTMLElement>('#net-limits-popover');
const netLimitsTitleEl = $<HTMLElement>('#net-limits-title');
const netLimitsNoteEl = $<HTMLElement>('#net-limits-note');
const netCapValEl = $<HTMLInputElement>('#net-cap-val');
const netCapUnitEl = $<HTMLSelectElement>('#net-cap-unit');
const netAllowValEl = $<HTMLInputElement>('#net-allow-val');
const netAllowUnitEl = $<HTMLSelectElement>('#net-allow-unit');
let netLimitsId: number | null = null;

/** A field the user left empty means unlimited; a field that holds no number at
 *  all must not be read as one, because this write replaces every column. */
type LimitRead = number | null | 'unreadable';

function readLimit(val: HTMLInputElement, unit: HTMLSelectElement): LimitRead {
  const raw = val.value.trim();
  if (raw === '') return null;
  const bytes = Number(raw) * Number(unit.value);
  if (!Number.isFinite(bytes)) return 'unreadable';
  return bytes > 0 ? Math.round(bytes) : null;
}

/* Both unit lists are powers of two, so dividing a stored figure back out is
   exact: a cap typed as 2 MB/s reads back as 2, not 1.9. The largest unit is
   offered unless the stored figure is smaller than it. */
function showLimit(val: HTMLInputElement, unit: HTMLSelectElement, bytes: number | null): void {
  const units = Array.from(unit.options, (o) => Number(o.value));
  const big = units.reduce((a, b) => Math.max(a, b), 0);
  const small = units.reduce((a, b) => Math.min(a, b), Number.POSITIVE_INFINITY);
  if (!bytes || bytes <= 0) {
    val.value = '';
    unit.value = String(big);
    return;
  }
  unit.value = String(bytes >= big ? big : small);
  val.value = String(bytes / Number(unit.value));
}

function setNetLimitsNote(text: string, isHint: boolean): void {
  netLimitsNoteEl.textContent = text;
  netLimitsNoteEl.classList.toggle('is-hint', isHint);
}

function closeNetLimits(): void {
  netLimitsPopoverEl.hidden = true;
  netLimitsId = null;
}

function openNetLimits(id: number, anchor: HTMLElement): void {
  const net = networks.find((n) => n.id === id);
  if (!net) return;
  netLimitsId = id;
  netLimitsTitleEl.textContent = `${net.displayName} limits`;
  showLimit(netCapValEl, netCapUnitEl, net.speedLimit);
  showLimit(netAllowValEl, netAllowUnitEl, net.dataLimit);
  setNetLimitsNote('Each limit spends only the traffic this adapter carries.', true);
  netLimitsPopoverEl.hidden = false;
  placePopover(netLimitsPopoverEl, anchor);
  netCapValEl.focus();
}

/* One write, both figures: `network:set-limits` sets all three columns, so a
   save that sent only the cap would silently clear an allowance nobody touched.
   The allowance window is `month` because that is the only one the IPC accepts. */
async function saveNetLimits(): Promise<void> {
  const id = netLimitsId;
  if (id === null) return;
  const cap = readLimit(netCapValEl, netCapUnitEl);
  const allowance = readLimit(netAllowValEl, netAllowUnitEl);
  if (cap === 'unreadable' || allowance === 'unreadable') {
    setNetLimitsNote('Use a number above zero, or leave the box empty for unlimited.', false);
    return;
  }
  try {
    await sucr.network.setLimits(id, cap, allowance, allowance === null ? null : 'month');
  } catch (err) {
    setNetLimitsNote(err instanceof Error ? err.message : String(err), false);
    return;
  }
  closeNetLimits();
  await refreshNetworks();
}

function netLimitChip(net: Network, which: 'cap' | 'allow'): HTMLButtonElement {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'net-limit-btn';
  btn.dataset['networkId'] = String(net.id);
  const name = document.createElement('span');
  const amount = document.createElement('b');
  const capSet = (net.speedLimit ?? 0) > 0;
  const allowanceSet = (net.dataLimit ?? 0) > 0;
  if (which === 'cap') {
    name.textContent = 'Cap';
    amount.textContent = capSet ? formatRate(net.speedLimit ?? 0) : 'none';
    if (capSet) btn.classList.add('is-set');
    btn.title = capSet
      ? 'Blocks leaving on this adapter are paced to this rate.'
      : 'No rate cap on this adapter.';
  } else {
    // The claim gate hands no new block to a route whose window is full, so the
    // chip says the consequence rather than repeating the number twice.
    const spent = allowanceSet && net.usedMonth >= (net.dataLimit ?? 0);
    name.textContent = spent ? 'Over allowance' : 'Allowance';
    amount.textContent = allowanceSet
      ? `${formatBytes(net.usedMonth)} / ${formatBytes(net.dataLimit ?? 0)}`
      : 'none';
    if (spent) btn.classList.add('is-shut');
    else if (allowanceSet) btn.classList.add('is-set');
    btn.title = spent
      ? 'This month is spent: no new blocks leave on this adapter until the window rolls.'
      : allowanceSet
        ? 'Blocks stop leaving on this adapter once this month\'s allowance is spent.'
        : 'No monthly allowance on this adapter.';
  }
  btn.setAttribute(
    'aria-label',
    `${which === 'cap' ? 'Rate cap' : 'Monthly allowance'} for ${net.displayName}`,
  );
  btn.append(name, amount);
  return btn;
}

document.addEventListener('click', (event) => {
  const btn = (event.target as Element).closest<HTMLElement>('.net-limit-btn');
  if (!btn) return;
  const id = Number(btn.dataset['networkId']);
  if (!Number.isFinite(id)) return;
  openNetLimits(id, btn);
});

netLimitsPopoverEl.addEventListener('click', (event) => {
  const el = event.target as Element;
  if (el.closest('#net-limits-close')) closeNetLimits();
  else if (el.closest('#net-limits-save')) void saveNetLimits();
});

document.addEventListener('pointerdown', (event) => {
  if (netLimitsPopoverEl.hidden) return;
  const el = event.target;
  if (!(el instanceof Element)) return;
  if (netLimitsPopoverEl.contains(el) || el.closest('.net-limit-btn')) return;
  closeNetLimits();
});

function netCard(net: Network, sharePct: number, live: boolean, bonded: number): HTMLElement {
  const color = `var(--${net.colorId ?? 'n1'})`;
  const card = document.createElement('div');
  card.className = 'net-card';

  const top = document.createElement('div');
  top.className = 'net-card-top';
  const group = document.createElement('div');
  group.className = 'net-card-title-group';

  const dot = document.createElement('i');
  dot.className = 'dot';
  dot.style.background = color;
  const name = document.createElement('span');
  name.className = 'net-card-name';
  name.textContent = net.displayName;
  const kindBadge = document.createElement('span');
  kindBadge.className = `net-badge ${netBadge(net.kind)}`;
  kindBadge.textContent = net.kind.toUpperCase();
  const stateBadge = document.createElement('span');
  stateBadge.className = `net-badge ${
    net.enabled && guardHeld(net)
      ? 'net-badge-held'
      : net.enabled
        ? 'net-badge-active'
        : 'net-badge-standby'
  }`;
  // A lone enabled device is not bonded — the engine only binds when two or
  // more routes are in the pool, so the badge says "Ready" until then.
  stateBadge.textContent = !net.enabled
    ? 'Standby'
    : guardHeld(net)
      ? 'Held'
      : live
        ? 'Active'
        : bonded >= 2
          ? 'Bonded'
          : 'Ready';
  if (net.enabled && guardHeld(net)) {
    stateBadge.title = 'Kept out of the bond by the metered guard';
  }
  group.append(dot, name, kindBadge, stateBadge);

  // Every value here is built with textContent: an adapter name is Windows'
  // string and a rename is the user's, and neither should be parsed as markup.
  const label = document.createElement('label');
  label.className = 'toggle-switch';
  label.title = 'Bond this adapter';
  const box = document.createElement('input');
  box.type = 'checkbox';
  box.checked = net.enabled;
  box.dataset['networkId'] = String(net.id);
  box.setAttribute('aria-label', `Bond ${net.displayName}`);
  const knob = document.createElement('span');
  knob.setAttribute('aria-hidden', 'true');
  label.append(box, knob);
  label.addEventListener('click', () => {
    label.classList.remove('animating');
    void label.offsetWidth;
    label.classList.add('animating');
    window.setTimeout(() => label.classList.remove('animating'), 320);
  });
  top.append(group, label);

  const meta = document.createElement('div');
  meta.className = 'net-card-meta';
  meta.append(
    // Two windows, not one unlabeled number: a card that says only "downloaded"
    // cannot be read for whether it stopped yesterday or has been idle for a
    // month, and neither window is derivable from the other.
    netMetaItem('Today', formatBytes(net.usedToday)),
    netMetaItem('Lifetime', formatBytes(net.usedLifetime)),
  );

  const bar = document.createElement('div');
  bar.className = 'net-card-bar';
  const fill = document.createElement('div');
  fill.className = 'net-card-bar-fill';
  fill.style.width = `${sharePct}%`;
  fill.style.background = color;
  bar.append(fill);

  const limits = document.createElement('div');
  limits.className = 'net-card-limits';
  limits.append(netLimitChip(net, 'cap'), netLimitChip(net, 'allow'));

  card.append(top, meta, bar, limits);
  return card;
}

function renderNetworksView(): void {
  const statusText = $<HTMLElement>('#net-bond-status-text');
  const headline = $<HTMLElement>('#net-headline');
  const desc = $<HTMLElement>('#net-desc');
  const statCount = $<HTMLElement>('#net-stat-count');
  const statToday = $<HTMLElement>('#net-stat-today');
  const statLifetime = $<HTMLElement>('#net-stat-lifetime');
  const cardsContainer = $<HTMLElement>('#net-adapter-cards');
  const live = library.some((d) => d.status === 'downloading');
  // What the deck counts is the pool the engine will actually use, not the
  // adapters that happen to be plugged in: a standby device is not bonded.
  const bonded = bondable(networks);
  const shares = perNetworkBytes();
  const shareTotal = [...shares.values()].reduce((a, b) => a + b, 0);
  const shareOf = (net: Network): number => {
    if (!net.enabled || shareTotal <= 0 || !net.colorId) return 0;
    return ((shares.get(net.colorId) ?? 0) / shareTotal) * 100;
  };

  // The summary is the sum of the cards plus the bytes no card was entitled to.
  // With nothing enumerated there is no per-adapter telemetry to add up, and the
  // library is the only record of bytes that actually moved — and it already
  // holds the proxied ones, so the second ledger is not added to that path.
  const dayStart = new Date().setHours(0, 0, 0, 0);
  const sumLibrary = (sinceMidnight: boolean) =>
    library.reduce(
      (acc, d) => acc + (!sinceMidnight || (d.finishedAt ?? 0) >= dayStart ? d.verifiedBytes : 0),
      0,
    );
  const todayBytes =
    networks.length === 0
      ? sumLibrary(true)
      : networks.reduce((a, n) => a + n.usedToday, 0) + unattributed.today;
  const lifetimeBytes =
    networks.length === 0
      ? sumLibrary(false)
      : networks.reduce((a, n) => a + n.usedLifetime, 0) + unattributed.lifetime;

  statToday.textContent = formatBytes(todayBytes);
  statLifetime.textContent = formatBytes(lifetimeBytes);
  statCount.textContent = String(networks.length === 0 ? 1 : bonded);

  const fragment = document.createDocumentFragment();

  // A mode that resolved to a proxy carries the links started from now on, and
  // those bytes belong to no adapter — so no card can pace them. The pane's
  // totals do hold them now, so say only what the cards cannot.
  const via = appInfo?.proxyVia ?? '';
  const proxyCarries =
    via && via !== 'direct' && !via.endsWith(' direct')
      ? ` A proxy carries new links (${via}): their bytes belong to no adapter, so the caps and allowances on each card do not count or pace them. A host on the bypass list keeps its own route.`
      : '';
  // The same words serve the other ways a byte ends up unattributed — a localhost
  // target, a route whose device has no card. Only the adapter-present path needs
  // them: with no card enumerated the totals come from the library, which already
  // holds these bytes, so there is nothing beside the total that could disagree.
  const carriedByNone =
    unattributed.lifetime > 0
      ? ` ${formatBytes(unattributed.lifetime)} lifetime (${formatBytes(unattributed.today)} today) belongs to no adapter; it is inside the totals above and on no card.`
      : '';

  if (networks.length === 0) {
    // Nothing enumerated: the system default route, which we cannot name.
    statusText.textContent = live ? '1 Active Route · Downloading' : 'Single Network Connected';
    headline.textContent = 'Primary System Route Active';
    desc.textContent =
      'Download traffic runs across your default system adapter. Plug in a secondary device (e.g. Phone USB Tethering or an Ethernet cable alongside Wi-Fi) to bond multiple connections and multiply your bandwidth.';

    const card = document.createElement('div');
    card.className = 'net-card';
    const top = document.createElement('div');
    top.className = 'net-card-top';
    const group = document.createElement('div');
    group.className = 'net-card-title-group';
    const dot = document.createElement('i');
    dot.className = 'dot';
    dot.style.background = 'var(--n1)';
    const name = document.createElement('span');
    name.className = 'net-card-name';
    name.textContent = 'Primary Network Adapter';
    const badge = document.createElement('span');
    badge.className = 'net-badge net-badge-active';
    badge.textContent = live ? 'Carrying Traffic' : 'Connected';
    group.append(dot, name, badge);
    top.append(group);

    const meta = document.createElement('div');
    meta.className = 'net-card-meta';
    meta.append(
      netMetaItem('Today', formatBytes(todayBytes)),
      netMetaItem('Lifetime', formatBytes(lifetimeBytes)),
    );

    const bar = document.createElement('div');
    bar.className = 'net-card-bar';
    const fill = document.createElement('div');
    fill.className = 'net-card-bar-fill';
    fill.style.width = live ? '100%' : '0%';
    fill.style.background = 'var(--n1)';
    bar.append(fill);

    card.append(top, meta, bar);
    fragment.append(card);
  } else if (bonded === 0) {
    statusText.textContent = 'All Networks Disabled · Transfers Paused';
    headline.textContent = 'Network Transfers Blocked';
    desc.textContent =
      'Every adapter is disabled. Active downloads are paused and no traffic will be sent or received until at least one network is enabled.' +
      carriedByNone;
    networks.forEach((net) => fragment.append(netCard(net, shareOf(net), live, bonded)));
  } else if (bonded === 1) {
    statusText.textContent = live
      ? '1 Active Route · Downloading'
      : '1 Route Ready · Bonding Off';
    headline.textContent = 'Single Route, Bonding Off';
    desc.textContent =
      'One device is enabled, so chunks leave on the route Windows picks. Nothing is bound to its address yet — turn on a second device to aggregate bandwidth.' +
      proxyCarries +
      carriedByNone;
    networks.forEach((net) => fragment.append(netCard(net, shareOf(net), live, bonded)));
  } else {
    statusText.textContent = live
      ? `${bonded} Networks Bonded · Live Aggregation`
      : `${bonded} Networks Bonded & Ready`;
    headline.textContent = 'Multi-Device Bandwidth Aggregation Active';
    desc.textContent = `Sucr spreads chunks across the ${bonded} enabled routes simultaneously. Each device keeps the colour of the blocks it carried.` + proxyCarries + carriedByNone;
    networks.forEach((net) => fragment.append(netCard(net, shareOf(net), live, bonded)));
  }

  cardsContainer.replaceChildren(fragment);
}

/* --- network bonding modal ----------------------------------------------- */
function renderNicModal(): void {
  const badgeText = $<HTMLElement>('#nic-modal-bond-text');
  const countBadge = $<HTMLElement>('#nic-modal-count');
  const cardsContainer = $<HTMLElement>('#nic-modal-cards');
  if (!cardsContainer) return;

  const live = library.some((d) => d.status === 'downloading');
  const bonded = bondable(networks);
  const shares = perNetworkBytes();
  const shareTotal = [...shares.values()].reduce((a, b) => a + b, 0);
  const shareOf = (net: Network): number => {
    if (!net.enabled || shareTotal <= 0 || !net.colorId) return 0;
    return ((shares.get(net.colorId) ?? 0) / shareTotal) * 100;
  };

  if (badgeText) {
    if (bonded >= 2) {
      badgeText.textContent = live ? `${bonded} Networks Bonded · Live` : `${bonded} Networks Bonded · Ready`;
    } else if (bonded === 1) {
      badgeText.textContent = live ? '1 Active Route · Downloading' : 'Single Network Connected';
    } else {
      badgeText.textContent = 'All Networks Disabled';
    }
  }

  if (countBadge) {
    const total = networks.length;
    countBadge.textContent = total === 1 ? '1 adapter' : `${total} adapters`;
  }

  const fragment = document.createDocumentFragment();

  if (networks.length === 0) {
    const card = document.createElement('div');
    card.className = 'net-card';
    const top = document.createElement('div');
    top.className = 'net-card-top';
    const group = document.createElement('div');
    group.className = 'net-card-title-group';
    const dot = document.createElement('i');
    dot.className = 'dot';
    dot.style.background = 'var(--n1)';
    const name = document.createElement('span');
    name.className = 'net-card-name';
    name.textContent = 'Primary System Adapter';
    const kindBadge = document.createElement('span');
    kindBadge.className = 'net-badge net-badge-eth';
    kindBadge.textContent = 'DEFAULT';
    const stateBadge = document.createElement('span');
    stateBadge.className = 'net-badge net-badge-active';
    stateBadge.textContent = live ? 'Active' : 'Ready';
    group.append(dot, name, kindBadge, stateBadge);
    top.append(group);
    card.append(top);
    fragment.append(card);
  } else {
    for (const net of networks) {
      fragment.append(netCard(net, shareOf(net), live, bonded));
    }
  }

  cardsContainer.replaceChildren(fragment);
}

function openNicModal(): void {
  const modal = $<HTMLElement>('#nic-modal-backdrop');
  if (!modal) return;
  renderNicModal();
  modal.hidden = false;
  requestAnimationFrame(() => {
    modal.classList.add('is-open');
  });
}

function closeNicModal(): void {
  const modal = $<HTMLElement>('#nic-modal-backdrop');
  if (!modal) return;
  modal.classList.remove('is-open');
  window.setTimeout(() => {
    if (!modal.classList.contains('is-open')) {
      modal.hidden = true;
    }
  }, 220);
}

/* --- setup wizard / tutorial --------------------------------------------- */
let wizardStep = 1;

function updateWizardCategoryPaths(baseFolder: string): void {
  // An empty field falls back to the folder the app already uses, never to a
  // drive letter this machine may not have.
  const trimmed = baseFolder.trim() || getDefaultDownloadFolder();
  const sep = trimmed.includes('/') ? '/' : '\\';
  const displayBase = trimmed.endsWith(sep) ? trimmed : `${trimmed}${sep}`;

  const pathArch = $<HTMLElement>('#wizard-cat-path-archive');
  const pathVid = $<HTMLElement>('#wizard-cat-path-video');
  const pathAud = $<HTMLElement>('#wizard-cat-path-audio');
  const pathDoc = $<HTMLElement>('#wizard-cat-path-doc');
  const pathProg = $<HTMLElement>('#wizard-cat-path-prog');

  if (pathArch) pathArch.textContent = `${displayBase}Archives`;
  if (pathVid) pathVid.textContent = `${displayBase}Videos`;
  if (pathAud) pathAud.textContent = `${displayBase}Audio`;
  if (pathDoc) pathDoc.textContent = `${displayBase}Documents`;
  if (pathProg) pathProg.textContent = `${displayBase}Programs`;

  const summaryFolder = $<HTMLElement>('#wizard-summary-folder');
  if (summaryFolder) summaryFolder.textContent = trimmed;

  // One chip pressed, never two — the bar's rule with the bar's normalisation, so
  // a Browse pick or a typed path lights the chip that owns that folder instead of
  // leaving the row all-unpressed. An empty field presses none.
  const current = baseFolder.trim();
  document
    .querySelectorAll<HTMLButtonElement>('#wizard-presets-chips .wizard-preset-chip')
    .forEach((chip) => {
      const active = sameFolder(chip.dataset['folder'] ?? '', current);
      chip.classList.toggle('is-active', active);
      chip.setAttribute('aria-pressed', String(active));
    });
}

function renderWizardStep(step: number): void {
  wizardStep = Math.max(1, Math.min(4, step));

  for (let i = 1; i <= 4; i++) {
    const el = $<HTMLElement>(`#wizard-step-${i}`);
    if (el) el.hidden = i !== wizardStep;
  }

  const indicator = $<HTMLElement>('#wizard-step-indicator');
  if (indicator) indicator.textContent = `Step ${wizardStep} of 4`;

  const progressFill = $<HTMLElement>('#wizard-progress-fill');
  if (progressFill) progressFill.style.width = `${wizardStep * 25}%`;

  const backBtn = $<HTMLButtonElement>('#wizard-btn-back');
  if (backBtn) backBtn.disabled = wizardStep === 1;

  const nextBtn = $<HTMLButtonElement>('#wizard-btn-next');
  if (nextBtn) {
    nextBtn.textContent = wizardStep === 4 ? 'Finish & Start' : 'Next';
  }
  syncWizardNext();
}

/* --- the wizard's folder field -------------------------------------------- */
/* The same affordance as the add bar, and now the same gate: the step that owns
   the field will not advance while that field names a folder that is not there.
   Skip stays live, so nobody is trapped — the escape hatch is the one the wizard
   already had, not a folder created behind the user's back. */
const wizardFolderEnsure: FolderEnsure = {
  input: $<HTMLInputElement>('#wizard-folder-input'),
  notice: $<HTMLElement>('#wizard-folder-notice'),
  text: $<HTMLElement>('#wizard-folder-notice-text'),
  button: $<HTMLButtonElement>('#wizard-folder-create'),
  missing: null,
  timer: 0,
  token: 0,
  onBlockedChange: () => syncWizardNext(),
  onCreated: (path) => updateWizardCategoryPaths(path),
};
wireFolderEnsure(wizardFolderEnsure);

function syncWizardNext(): void {
  const nextBtn = $<HTMLButtonElement>('#wizard-btn-next');
  if (nextBtn) nextBtn.disabled = wizardStep === 1 && wizardFolderEnsure.missing !== null;
}

/** The wizard's suggested locations are the folders this machine actually has —
 *  the same answer the add bar gets. Hardcoding them meant offering paths like
 *  `D:\SucrDownloads` that no scan had ever confirmed. */
async function loadWizardPresets(base: string): Promise<void> {
  const host = $<HTMLElement>('#wizard-presets-chips');
  let list: Array<{ preset: PresetId; path: string }>;
  try {
    list = await sucr.app.getFolderPresets(base);
  } catch {
    host.replaceChildren();
    return;
  }
  host.replaceChildren(
    ...list.map((entry) => {
      const chip = document.createElement('button');
      chip.type = 'button';
      chip.className = 'wizard-preset-chip';
      chip.dataset['folder'] = entry.path;
      // The same grammar as the bar's chips: the folder's name on the face, the
      // resolved path in the tooltip, and the pressed state in `aria-pressed`.
      chip.textContent = PRESET_LABELS[entry.preset];
      chip.title = entry.path;
      chip.setAttribute('aria-pressed', 'false');
      return chip;
    }),
  );
}

function openSetupWizard(): void {
  const modal = $<HTMLElement>('#setup-wizard-backdrop');
  if (!modal) return;

  const currentFolder = setting('storage.default_folder');
  const folderInput = $<HTMLInputElement>('#wizard-folder-input');
  if (folderInput) {
    // The app's own folder, never an invented one: a suggestion this machine does
    // not have is exactly the defect the add bar just stopped having.
    folderInput.value = currentFolder || getDefaultDownloadFolder();
    updateWizardCategoryPaths(folderInput.value);
    void probeFolder(wizardFolderEnsure, folderInput.value);
    void loadWizardPresets(folderInput.value);
  }

  const currentPolicy = setting('network.bonding_policy') || 'dynamic';
  document.querySelectorAll<HTMLElement>('.wizard-option-card').forEach((card) => {
    const input = card.querySelector<HTMLInputElement>('input[name="wizard-policy"]');
    const isMatching = card.dataset['policy'] === currentPolicy;
    if (input) input.checked = isMatching;
    card.classList.toggle('is-selected', isMatching);
  });

  const meteredGuard = $<HTMLInputElement>('#wizard-metered-guard');
  if (meteredGuard) meteredGuard.checked = settingOn('network.metered_guard');

  const currentTheme = setting('ui.theme') || 'dark';
  document.querySelectorAll<HTMLButtonElement>('#wizard-theme-seg button').forEach((btn) => {
    btn.setAttribute('aria-pressed', String(btn.dataset['val'] === currentTheme));
  });

  const currentMeter = setting('ui.meter_style') || 'wave';
  document.querySelectorAll<HTMLButtonElement>('#wizard-meter-seg button').forEach((btn) => {
    btn.setAttribute('aria-pressed', String(btn.dataset['val'] === currentMeter));
  });

  const soundCheck = $<HTMLInputElement>('#wizard-sound-complete');
  if (soundCheck) soundCheck.checked = settingOn('ui.sound_on_complete');

  renderWizardStep(1);
  modal.hidden = false;
  requestAnimationFrame(() => {
    modal.classList.add('is-open');
  });
}

function closeSetupWizard(): void {
  const modal = $<HTMLElement>('#setup-wizard-backdrop');
  if (!modal) return;
  modal.classList.remove('is-open');
  window.setTimeout(() => {
    if (!modal.classList.contains('is-open')) {
      modal.hidden = true;
    }
  }, 220);
}

async function saveWizardSettings(): Promise<void> {
  const folderInput = $<HTMLInputElement>('#wizard-folder-input');
  // An empty field keeps the folder the app already uses; it never invents a
  // drive-letter path that may not exist on this machine.
  const folder = folderInput?.value.trim() || getDefaultDownloadFolder();
  await saveSetting('storage.default_folder', folder).catch(reportSettingError);

  const setFolder = $<HTMLInputElement>('#set-folder');
  if (setFolder) setFolder.value = folder;

  const catSorting = $<HTMLInputElement>('#wizard-cat-sorting')?.checked ?? true;
  localStorage.setItem('sucr.category_sorting', String(catSorting));
  renderCategoriesList();

  const selectedPolicy = document.querySelector<HTMLInputElement>('input[name="wizard-policy"]:checked')?.value || 'dynamic';
  await saveSetting('network.bonding_policy', selectedPolicy).catch(reportSettingError);

  const meteredGuard = $<HTMLInputElement>('#wizard-metered-guard')?.checked ?? true;
  await saveSetting('network.metered_guard', String(meteredGuard)).catch(reportSettingError);

  const selectedTheme = document.querySelector<HTMLButtonElement>('#wizard-theme-seg button[aria-pressed="true"]')?.dataset['val'] || 'dark';
  await saveSetting('ui.theme', selectedTheme).catch(reportSettingError);
  applyTheme();

  const selectedMeter = document.querySelector<HTMLButtonElement>('#wizard-meter-seg button[aria-pressed="true"]')?.dataset['val'] || 'wave';
  await saveSetting('ui.meter_style', selectedMeter).catch(reportSettingError);
  applyProgressMode();

  const soundOn = $<HTMLInputElement>('#wizard-sound-complete')?.checked ?? true;
  await saveSetting('ui.sound_on_complete', String(soundOn)).catch(reportSettingError);

  await saveSetting('system.setup_completed', 'true').catch(reportSettingError);
  showToast(`Setup complete! Downloads configured to ${folder}`);
}

/* Turning a card's switch on or off changes which sockets the engine binds for
 * the *next* job; a running one keeps the route list it started with. */
function bindNetworkToggles(container: HTMLElement): void {
  container.addEventListener('change', (event) => {
    const box = event.target as HTMLInputElement;
    const id = Number(box.dataset?.['networkId']);
    if (!Number.isFinite(id)) return;
    void sucr.network
      .setEnabled(id, box.checked)
      .then(async () => {
        await refreshNetworks();
        await refresh();
      })
      .catch((err: unknown) => {
        box.checked = !box.checked;
        reportSettingError(err);
      });
  });
}

/* --- boot: everything shown comes from the Rust side --------------------- */
async function boot(): Promise<void> {
  try {
    const info = await sucr.app.getInfo();
    await listen('download:changed', () => scheduleRefresh());
    // The main thread polls the adapter list, so a cable plugged in while this
    // view was open re-reads the rows instead of waiting for a manual refresh.
    await listen('network:changed', () => void refreshNetworks());
    // The curve needs a clock of its own: refresh only fires when the engine has
    // something to say, and an idle minute still has to be drawn as idle.
    window.setInterval(tpTick, TP_STEP_MS);
    bindNetworkToggles($<HTMLElement>('#net-adapter-cards'));
    bindNetworkToggles($<HTMLElement>('#nic-modal-cards'));
    $<HTMLElement>('#nic-deck')?.addEventListener('click', () => {
      openNicModal();
    });
    $<HTMLElement>('#nic-modal-close')?.addEventListener('click', () => {
      closeNicModal();
    });
    $<HTMLElement>('#nic-modal-done')?.addEventListener('click', () => {
      closeNicModal();
    });
    $<HTMLElement>('#nic-modal-backdrop')?.addEventListener('click', (e) => {
      if (e.target === e.currentTarget) {
        closeNicModal();
      }
    });
    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        const modal = $<HTMLElement>('#nic-modal-backdrop');
        if (modal && !modal.hidden) {
          closeNicModal();
        }
        const wizardModal = $<HTMLElement>('#setup-wizard-backdrop');
        if (wizardModal && !wizardModal.hidden) {
          closeSetupWizard();
        }
      }
    });

    // Setup tutorial listeners
    $<HTMLButtonElement>('#set-setup-tutorial')?.addEventListener('click', () => {
      openSetupWizard();
    });
    $<HTMLButtonElement>('#wizard-folder-browse')?.addEventListener('click', async () => {
      try {
        const picked = await sucr.app.pickFolder();
        if (picked) {
          const input = $<HTMLInputElement>('#wizard-folder-input');
          if (input) {
            input.value = picked;
            updateWizardCategoryPaths(picked);
          }
        }
      } catch (err) {
        console.error('pickFolder error:', err);
      }
    });
    $<HTMLInputElement>('#wizard-folder-input')?.addEventListener('input', (e) => {
      updateWizardCategoryPaths((e.target as HTMLInputElement).value);
    });
    // Chips are built at runtime from the disk scan, so the click is matched on
    // the container rather than bound per chip.
    $<HTMLElement>('#wizard-presets-chips').addEventListener('click', (event) => {
      const chip = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>(
        '.wizard-preset-chip',
      );
      const folder = chip?.dataset['folder'];
      const input = $<HTMLInputElement>('#wizard-folder-input');
      if (!folder || !input) return;
      input.value = folder;
      updateWizardCategoryPaths(folder);
      void probeFolder(wizardFolderEnsure, folder);
    });
    $$<HTMLElement>('.wizard-option-card').forEach((card) => {
      card.addEventListener('click', () => {
        const radio = card.querySelector<HTMLInputElement>('input[type="radio"]');
        if (radio) radio.checked = true;
        $$<HTMLElement>('.wizard-option-card').forEach((c) => c.classList.remove('is-selected'));
        card.classList.add('is-selected');
      });
    });
    $$<HTMLButtonElement>('#wizard-theme-seg button').forEach((btn) => {
      btn.addEventListener('click', () => {
        $$<HTMLButtonElement>('#wizard-theme-seg button').forEach((b) => b.setAttribute('aria-pressed', 'false'));
        btn.setAttribute('aria-pressed', 'true');
      });
    });
    $$<HTMLButtonElement>('#wizard-meter-seg button').forEach((btn) => {
      btn.addEventListener('click', () => {
        $$<HTMLButtonElement>('#wizard-meter-seg button').forEach((b) => b.setAttribute('aria-pressed', 'false'));
        btn.setAttribute('aria-pressed', 'true');
      });
    });
    $<HTMLButtonElement>('#wizard-btn-next')?.addEventListener('click', () => {
      if (wizardStep < 4) {
        renderWizardStep(wizardStep + 1);
      } else {
        void saveWizardSettings();
        closeSetupWizard();
      }
    });
    $<HTMLButtonElement>('#wizard-btn-back')?.addEventListener('click', () => {
      if (wizardStep > 1) {
        renderWizardStep(wizardStep - 1);
      }
    });
    $<HTMLButtonElement>('#wizard-btn-skip')?.addEventListener('click', () => {
      void saveSetting('system.setup_completed', 'true').catch(reportSettingError);
      closeSetupWizard();
    });
    $<HTMLElement>('#setup-wizard-backdrop')?.addEventListener('click', (e) => {
      if (e.target === e.currentTarget) {
        closeSetupWizard();
      }
    });
    await listen<BlockMap>('download:blocks', (event) => {
      const payload = event.payload;
      if (!payload || payload.id !== mapWatchId) return;
      applyMap(payload);
    });

    const [downloads, netList, categoryRows] = await Promise.all([
      sucr.download.list(),
      sucr.network.list(),
      sucr.category.list(),
      loadSessions(),
    ]);
    appInfo = info;
    library = downloads;
    networks = netList.adapters;
    unattributed = netList.unattributed;
    categories = categoryRows;
    globalCap = info.globalSpeedLimit;
    await loadSettings();
    void loadFolderPresets(getDefaultDownloadFolder());

    const channelsMatch = JSON.stringify(info.channels) === JSON.stringify([...CHANNELS]);

    $<HTMLElement>('#version').textContent = `v${info.version} ${BUILD_STAGE}`;
    document.title = `Sucr ${info.version} ${BUILD_STAGE}`;

    $<HTMLElement>('#set-runtime').textContent =
      `${info.runtime} · WebView2 ${info.webview} · SQLite ${info.sqlite} · ` +
      `schema v${info.schemaVersion} (${info.journalMode.toUpperCase()}) · ${info.tables} tables`;
    renderAddonCard(info.capture?.addon);
    $<HTMLElement>('#set-update-note').textContent = `This build is ${info.version} ${BUILD_STAGE}. Check for updates whenever you like.`;
    const dbPath = $<HTMLElement>('#set-db-path');
    dbPath.textContent = info.paths.dbPath;
    dbPath.title = info.paths.dbPath;

    if (selectedId === null && library.length > 0) selectedId = library[0]?.id ?? null;

    totals = sampleAll();
    detectCompletions();
    renderRows();
    updateCounts();
    renderSettings();
    $<HTMLElement>('#net-count').textContent = String(networks.length);

    renderCategoriesList();

    applyTheme();
    applyProgressMode();
    selectPane(0, false);
    await syncMap();
    await updateInspector();

    await adoptLegacySetupFlag().catch(reportSettingError);
    const firstRun = setupPending();
    if (info.smoke) {
      await runSmoke(info, channelsMatch, firstRun);
    } else if (firstRun) {
      openSetupWizard();
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    showToast(`window.sucr failed: ${message}`);
    console.error(err);
    if (location.search.includes('smoke')) {
      await invoke('smoke_report', {
        report: JSON.stringify({ ok: false, error: message }),
      }).catch(() => undefined);
    }
  }
}

window.addEventListener('resize', () => alignIndicator());
if (document.fonts) void document.fonts.ready.then(() => alignIndicator());

/* --- smoke: drives the real surface, then hands the report to Rust -------- */
async function runSmoke(info: AppInfo, channelsMatch: boolean, firstRun: boolean): Promise<void> {
  const report: Record<string, unknown> = { ok: false };
  try {
    const [downloads, one, trail, netList, categoryRows] = await Promise.all([
      sucr.download.list(),
      sucr.download.get(1),
      sucr.history.tail(1, 5),
      sucr.network.list(),
      sucr.category.list(),
    ]);
    const networkRows = netList.adapters;
    const mapOfMissing = await sucr.download.map(999_999);

    // The shell dialog is refused under --smoke instead of opened, so this
    // proves app:pick-folder is wired end to end without parking a modal
    // nobody is there to dismiss.
    const pickedInSmoke = await sucr.app.pickFolder();

    // And through the button itself: Browse must reach the channel and take the
    // refusal quietly — same field, no error line. This is what catches a
    // mistyped method name, which a channel-level call would not.
    document.querySelector<HTMLElement>('#add-cancel')?.click();
    openAdd(true);
    const folderBefore = addFolder.value;
    addFolderBrowse.click();
    await new Promise((r) => window.setTimeout(r, 150));
    const browseClick = {
      errorShown: addError.hidden === false,
      folderKept: addFolder.value === folderBefore,
    };

    // The quick folders are read from the disk rather than derived from a string,
    // so the gate asks the same question the chip asks and hands the paths to
    // Node, which is the only party here that can prove a directory exists. The
    // chip count catches a renderer that resolved a list it never rendered.
    await loadFolderPresets(getDefaultDownloadFolder());
    const presets = {
      resolved: folderPresets.map((entry) => ({ preset: entry.preset, path: entry.path })),
      chips: presetChips().length,
    };

    // A folder that is not there yet has to say so and offer to make it. The path
    // lives inside this run's own data dir, so Node can prove the create touched
    // the disk, and the whole dir is deleted when the gate finishes. The name is
    // fresh per launch because the gate opens the same data dir twice: a second
    // run that found its own leftover would report an absent folder as present.
    clearVerdict(addCheck); // the link verdict is not what this step is about
    const scratchFolder = `${appInfo?.paths.dataDir ?? ''}\\m0-missing\\deeper-${Date.now().toString(36)}`;
    addFolder.value = scratchFolder;
    await probeFolder(addFolderEnsure, scratchFolder);
    const folderCreate = {
      target: scratchFolder,
      noticeShown: addFolderNotice.hidden === false,
      missingWhileAbsent: addFolderEnsure.missing !== null,
      blockedWhileAbsent: addSubmit.disabled === true,
      noticeCleared: false,
      existsAfter: false,
      fieldKept: false,
      enabledAgain: false,
      target1: '',
      target2: '',
      lateCreated: false,
      probedLeftAlone: false,
      fieldFollowsClick: false,
    };
    addFolderCreate.click();
    for (let i = 0; i < 20 && !addFolderNotice.hidden; i++) {
      await new Promise((r) => window.setTimeout(r, 50));
    }
    const afterCreate = await sucr.app.folderState(scratchFolder);
    folderCreate.noticeCleared = addFolderNotice.hidden === true;
    folderCreate.existsAfter = afterCreate?.isDir === true;
    folderCreate.fieldKept = addFolder.value === (afterCreate?.path ?? '');
    folderCreate.enabledAgain = addSubmit.disabled === false;

    // A click can beat the debounce. The button answers for the text in the field
    // when it is clicked, so the folder made is the one the user is looking at and
    // the one the probe merely saw stays absent.
    const stamp = Date.now().toString(36);
    const probedFirst = `${appInfo?.paths.dataDir ?? ''}\\m0-probed-${stamp}`;
    const typedLast = `${appInfo?.paths.dataDir ?? ''}\\m0-typed-${stamp}`;
    addFolder.value = probedFirst;
    await probeFolder(addFolderEnsure, probedFirst);
    addFolder.value = typedLast; // no debounce awaited: this is the race
    addFolderCreate.click();
    for (let i = 0; i < 20 && !addFolderNotice.hidden; i++) {
      await new Promise((r) => window.setTimeout(r, 50));
    }
    const lateState = await sucr.app.folderState(typedLast);
    const probedState = await sucr.app.folderState(probedFirst);
    folderCreate.target1 = probedFirst;
    folderCreate.target2 = typedLast;
    folderCreate.lateCreated = lateState?.isDir === true;
    folderCreate.probedLeftAlone = probedState?.isDir !== true;
    folderCreate.fieldFollowsClick = addFolder.value === (lateState?.path ?? '');

    // The wizard gates its own step the same way the bar gates Download: Next
    // stays dark while the field names a folder that is not there, and Skip stays
    // live so the notice is never a trap.
    const wizardGate = {
      target: '',
      noticeShown: false,
      nextBlocked: false,
      nextLabel: '',
      skipLive: false,
      noticeCleared: false,
      nextLive: false,
      chips: [] as Array<{ label: string; path: string; pressed: boolean }>,
      onePressedAfterClick: false,
      fieldIsChip: false,
      chipUnblocks: false,
    };
    const wizardFolder = `${appInfo?.paths.dataDir ?? ''}\\m0-wizard-${stamp}`;
    const wizardInput = $<HTMLInputElement>('#wizard-folder-input');
    const wizardNotice = $<HTMLElement>('#wizard-folder-notice');
    const wizardNext = $<HTMLButtonElement>('#wizard-btn-next');
    if (wizardInput && wizardNotice && wizardNext) {
      wizardInput.value = wizardFolder;
      await probeFolder(wizardFolderEnsure, wizardFolder);
      wizardGate.target = wizardFolder;
      wizardGate.noticeShown = wizardNotice.hidden === false;
      wizardGate.nextBlocked = wizardNext.disabled === true;
      wizardGate.nextLabel = wizardNext.textContent ?? '';
      wizardGate.skipLive = $<HTMLButtonElement>('#wizard-btn-skip')?.disabled === false;
      wizardFolderEnsure.button.click();
      for (let i = 0; i < 20 && !wizardNotice.hidden; i++) {
        await new Promise((r) => window.setTimeout(r, 50));
      }
      wizardGate.noticeCleared = wizardNotice.hidden === true;
      wizardGate.nextLive = wizardNext.disabled === false;

      // The chips are the bar's chips on this surface too: a name on the face, the
      // resolved path behind it, one pressed at a time, and choosing one is enough
      // to clear the gate. `--smoke` never opens the wizard, so this asks the
      // wizard's own loader rather than reading an empty container.
      await loadWizardPresets(wizardInput.value);
      const chips = [
        ...document.querySelectorAll<HTMLButtonElement>('#wizard-presets-chips .wizard-preset-chip'),
      ];
      wizardGate.chips = chips.map((c) => ({
        label: c.textContent ?? '',
        path: c.title,
        pressed: c.getAttribute('aria-pressed') === 'true',
      }));
      chips[0]?.click();
      for (let i = 0; i < 20 && !wizardNotice.hidden; i++) {
        await new Promise((r) => window.setTimeout(r, 50));
      }
      wizardGate.onePressedAfterClick =
        chips.filter((c) => c.getAttribute('aria-pressed') === 'true').length === 1;
      wizardGate.fieldIsChip = wizardInput.value === (chips[0]?.title ?? '');
      wizardGate.chipUnblocks = wizardNotice.hidden === true && wizardNext.disabled === false;
    }
    openAdd(false);

    // The two Copy items depend on the clipboard, and whether the webview has
    // `navigator.clipboard` at all depends on its origin counting as a secure
    // context. Reported, never written: the gate must not replace what is on the
    // user's clipboard, and a synthetic click has no user activation to borrow.
    const clipboard = {
      secure: window.isSecureContext,
      api: typeof navigator.clipboard?.writeText === 'function',
    };

    // The Networks dashboard is the surface the bonding spec added, so the gate
    // clicks into it: one card per enumerated adapter, one bond switch each, and
    // a summary painted from the same rows the cards add up. The two amount boxes
    // start life as em dashes in the markup, so a byte figure in them proves the
    // renderer ran rather than a default that happened to look like the answer.
    document.querySelector<HTMLElement>('.nav-item[data-view="networks"]')?.click();
    const netView = {
      shown: networksViewEl.hidden === false,
      cards: document.querySelectorAll('#net-adapter-cards .net-card').length,
      switches: document.querySelectorAll('#net-adapter-cards input[type="checkbox"]').length,
      stats: [...document.querySelectorAll('.net-summary-stats .net-stat-num')].map((n) => n.textContent),
      statLabels: [...document.querySelectorAll('.net-summary-stats .net-stat-label')].map((n) => n.textContent),
      cardMeta: document.querySelector('#net-adapter-cards .net-card-meta')?.textContent ?? '',
      lifetimeType:
        networkRows.length === 0 ? 'no adapters' : typeof networkRows[0]?.usedLifetime,
    };
    document.querySelector<HTMLElement>('.nav-item[data-view="all"]')?.click();

    // Step 6's write path is a click, so the gate clicks it: a chip opens the
    // popover, a saved 2 MB/s cap reads back off the row, and clearing it puts
    // the adapter back to unlimited before any download check could run under a
    // limiter. The Save button's handler returns nothing to await, so the gate
    // calls the same function that handler dispatches to.
    const limits: {
      chip: boolean;
      opened: boolean;
      closed: boolean;
      stored: number | null;
      cleared: number | null;
    } = { chip: false, opened: false, closed: false, stored: null, cleared: null };
    const capChip = document.querySelector<HTMLElement>('#net-adapter-cards .net-limit-btn');
    const capTargetId = Number(capChip?.dataset['networkId']);
    limits.chip = capChip !== null && Number.isFinite(capTargetId);
    if (capChip) {
      capChip.click();
      limits.opened = netLimitsPopoverEl.hidden === false;
      netCapValEl.value = '2';
      await saveNetLimits();
      limits.closed = netLimitsPopoverEl.hidden === true;
      limits.stored =
        (await sucr.network.list()).adapters.find((n) => n.id === capTargetId)?.speedLimit ?? null;
      document.querySelector<HTMLElement>('#net-adapter-cards .net-limit-btn')?.click();
      netCapValEl.value = '';
      await saveNetLimits();
      limits.cleared =
        (await sucr.network.list()).adapters.find((n) => n.id === capTargetId)?.speedLimit ?? null;
    }

    // The proxy row is a real click path too, and before this slice the gate had
    // no UI check for it at all: a missing node would have thrown, but a row that
    // renders and never moves the engine would have passed. So the smoke clicks
    // the seg-control, commits a typed address through the change handler, reads
    // the engine's own answer back off the card line, asks for the two refusals
    // that the boundary owns, and leaves all three keys cleared — a stored proxy
    // must not outlive the check that wrote it, because later download checks
    // run inside the same process and read the mirror this one moves.
    // Card lines are settled by polling: the change handlers dispatch their writes
    // without returning a promise the gate could await, so the gate waits on the
    // reading the write produces instead of on the write itself.
    const noteHas = async (id: string, needle: string): Promise<boolean> => {
      for (let i = 0; i < 40; i++) {
        if ((document.querySelector(id)?.textContent ?? '').includes(needle)) return true;
        await new Promise((r) => setTimeout(r, 50));
      }
      return false;
    };
    const proxyNoteHas = (needle: string) => noteHas('#set-proxy-via', needle);
    const authNote = (): string => document.querySelector('#set-proxy-auth')?.textContent ?? '';
    // The change handlers dispatch `saveSetting` without returning a promise the
    // gate could await, so the gate settles each write by reading the table back
    // instead — an overlapping write that lands after the next one would leave a
    // cleared key in the table, and a stored proxy left behind moves the engine
    // mirror for every check that runs after this one.
    const commitProxy = (id: string) => {
      document.querySelector(id)?.dispatchEvent(new Event('change', { bubbles: true }));
    };
    const proxyKeys = async (): Promise<Record<string, string | undefined>> =>
      await sucr.settings.list();
    const waitKey = async (key: string, want: string | null): Promise<boolean> => {
      for (let i = 0; i < 40; i++) {
        // `settings.list` omits a cleared key rather than returning null, so an
        // absent key has to read as the null the caller is waiting for.
        if (((await proxyKeys())[key] ?? null) === want) return true;
        await new Promise((r) => setTimeout(r, 50));
      }
      return false;
    };
    const proxy: {
      manualPressed?: boolean;
      via?: boolean;
      viaText?: string;
      bypassStored?: string | null;
      netNotes?: boolean;
      netNoteText?: string;
      refusedTone?: string;
      refusedText?: string;
      refusedKeepsBox?: boolean;
      refusedKeepsStore?: boolean;
      authLine?: boolean;
      authLineText?: string;
      authApplied?: boolean;
      authNoSecret?: boolean;
      passCleared?: boolean;
      refusedNameless?: boolean;
      refusedNamelessTone?: string;
      refusedNamelessKeepsStore?: boolean;
      authForgotten?: boolean;
      stored?: { mode: string | null; url: string | null; bypass: string | null };
      ok?: boolean;
    } = {};
    document.querySelector<HTMLElement>('.nav-item[data-view="settings"]')?.click();
    (document.querySelector('#set-proxy-url') as HTMLInputElement | null)?.focus();
    document.querySelector<HTMLElement>('.seg-control[data-set="network.proxy_mode"] button[data-val="manual"]')?.click();
    // 'Egress: manual' cannot be satisfied by the boot line, which says 'Egress:
    // direct.' until this click lands — so the wait proves the click moved the
    // engine, not that a previous reading happened to be on screen.
    await proxyNoteHas('Egress: manual');
    await waitKey('network.proxy_mode', 'manual');
    proxy.manualPressed =
      document.querySelector('.seg-control[data-set="network.proxy_mode"] button[data-val="manual"]')
        ?.getAttribute('aria-pressed') === 'true';
    (document.querySelector('#set-proxy-url') as HTMLInputElement).value = 'http://127.0.0.1:8199';
    commitProxy('#set-proxy-url');
    proxy.via = await proxyNoteHas('manual 127.0.0.1:8199');
    proxy.viaText = document.querySelector('#set-proxy-via')?.textContent ?? '';
    (document.querySelector('#set-proxy-bypass') as HTMLInputElement).value = 'localhost';
    commitProxy('#set-proxy-bypass');
    await waitKey('network.proxy_bypass', 'localhost');
    proxy.bypassStored = (await proxyKeys())['network.proxy_bypass'] ?? null;
    // The Networks pane states the same rule in its own words, and it has to:
    // that view's totals are the sum of what each adapter recorded, and a proxied
    // link records nothing on any adapter. Read it by leaving Settings and coming
    // back, which is the click path a user takes.
    const netNoteHas = (needle: string) => noteHas('#net-desc', needle);
    document.querySelector<HTMLElement>('.nav-item[data-view="networks"]')?.click();
    proxy.netNotes = await netNoteHas('A proxy carries new links (manual 127.0.0.1:8199)');
    proxy.netNoteText = document.querySelector('#net-desc')?.textContent ?? '';
    document.querySelector<HTMLElement>('.nav-item[data-view="settings"]')?.click();
    // The credential row, on the same click path. Three things the address row cannot
    // prove: that the pair actually reaches the engine (`applies` is the engine's own
    // answer to "is this the proxy in front of the app", not this page comparing two
    // boxes); that the password never comes back across the bridge, in the reading or
    // in the whole `app:get-info` payload; and that the boxes stop holding it once the
    // write lands. Then the one refusal this row owns — a password with no name in
    // front of it is never sent — and the forget, because a credential stored for this
    // fixture would move the mirror for every check that runs after.
    (document.querySelector('#set-proxy-user') as HTMLInputElement).value = 'gate-user';
    (document.querySelector('#set-proxy-pass') as HTMLInputElement).value = 'gate-secret';
    commitProxy('#set-proxy-user');
    proxy.authLine = await noteHas('#set-proxy-auth', 'gate-user for 127.0.0.1:8199 is in use');
    proxy.authLineText = authNote();
    const authInfo = await sucr.app.getInfo();
    proxy.authApplied = authInfo.proxyAuth.stored === true && authInfo.proxyAuth.applies === true;
    proxy.authNoSecret =
      !authNote().includes('gate-secret') && !JSON.stringify(authInfo).includes('gate-secret');
    proxy.passCleared = (document.querySelector('#set-proxy-pass') as HTMLInputElement).value === '';
    // The boxes are cleared and committed in the same synchronous step, with no await
    // between: a poll that lands in between would repaint the name from the engine and
    // turn a refusal test into a write.
    (document.querySelector('#set-proxy-user') as HTMLInputElement).value = '';
    (document.querySelector('#set-proxy-pass') as HTMLInputElement).value = 'gate-orphan';
    commitProxy('#set-proxy-pass');
    proxy.refusedNameless = await noteHas('#set-proxy-auth', 'Not saved:');
    proxy.refusedNamelessTone =
      document.querySelector('#set-proxy-auth')?.getAttribute('data-tone') ?? '';
    // The refused pair was never written, so the one before it is still what the engine
    // holds — and still in use, because the address it was keyed to has not moved.
    proxy.refusedNamelessKeepsStore =
      (await sucr.app.getInfo()).proxyAuth.username === 'gate-user';
    (document.querySelector('#set-proxy-user') as HTMLInputElement).value = '';
    (document.querySelector('#set-proxy-pass') as HTMLInputElement).value = '';
    commitProxy('#set-proxy-user');
    await noteHas('#set-proxy-auth', 'No credential stored');
    proxy.authForgotten = (await sucr.app.getInfo()).proxyAuth.stored === false;
    const refusedUrl = 'http://user:pass@127.0.0.1:8199';
    (document.querySelector('#set-proxy-url') as HTMLInputElement).value = refusedUrl;
    commitProxy('#set-proxy-url');
    await proxyNoteHas('Not saved:');
    proxy.refusedTone = document.querySelector('#set-proxy-via')?.getAttribute('data-tone') ?? '';
    proxy.refusedText = document.querySelector('#set-proxy-via')?.textContent ?? '';
    proxy.refusedKeepsBox =
      (document.querySelector('#set-proxy-url') as HTMLInputElement).value.includes('user:pass@');
    // The refusal must not have written anything: the address stays what the
    // previous commit stored. Polled, because that commit is the handler's async
    // write and this read must not overtake it.
    proxy.refusedKeepsStore = await waitKey('network.proxy_url', 'http://127.0.0.1:8199');
    document.querySelector<HTMLElement>('.seg-control[data-set="network.proxy_mode"] button[data-val="direct"]')?.click();
    await waitKey('network.proxy_mode', 'direct');
    (document.querySelector('#set-proxy-url') as HTMLInputElement).value = '';
    commitProxy('#set-proxy-url');
    (document.querySelector('#set-proxy-bypass') as HTMLInputElement).value = '';
    commitProxy('#set-proxy-bypass');
    await proxyNoteHas('Egress: direct.');
    document.querySelector<HTMLElement>('.nav-item[data-view="all"]')?.click();
    const urlCleared = await waitKey('network.proxy_url', null);
    const bypassCleared = await waitKey('network.proxy_bypass', null);
    const proxyNow = await proxyKeys();
    proxy.stored = {
      mode: proxyNow['network.proxy_mode'] ?? null,
      url: proxyNow['network.proxy_url'] ?? null,
      bypass: proxyNow['network.proxy_bypass'] ?? null,
    };
    proxy.ok =
      urlCleared &&
      bypassCleared &&
      proxy.manualPressed === true &&
      proxy.via === true &&
      proxy.bypassStored === 'localhost' &&
      proxy.netNotes === true &&
      proxy.refusedTone === 'error' &&
      proxy.refusedText.includes('carries no credentials') &&
      proxy.refusedKeepsBox === true &&
      proxy.refusedKeepsStore === true &&
      proxy.authLine === true &&
      proxy.authApplied === true &&
      proxy.authNoSecret === true &&
      proxy.passCleared === true &&
      proxy.refusedNameless === true &&
      proxy.refusedNamelessTone === 'error' &&
      proxy.refusedNamelessKeepsStore === true &&
      proxy.authForgotten === true &&
      proxy.stored.mode === 'direct' &&
      proxy.stored.url === null &&
      proxy.stored.bypass === null;

    const selected = (): string | null =>
      document.querySelector('[role="tab"][aria-selected="true"]')?.id ?? null;
    const first = selected();
    tabs[0]?.focus();
    tabs[0]?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
    const afterArrow = selected();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '3', ctrlKey: true, bubbles: true }));
    const afterCtrl3 = selected();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '5', ctrlKey: true, bubbles: true }));
    const afterCtrl5 = selected();

    // Write path without any network: add a row, see it, remove it. The two
    // speed-limit channels are exercised on the same row so the gate covers
    // their round trip without touching a socket.
    const globalCapped = await sucr.app.setSpeedLimit(262_144);
    await sucr.settings.set('engine.max_active_downloads', '5');
    const settingsAfter = await sucr.settings.list();
    await sucr.settings.set('engine.max_active_downloads', null);
    const settingsCleared = await sucr.settings.list();

    // Reduced motion used to be Windows' call alone, so on a box that asks for
    // stillness the gate could not tell a working animation from a collapsed
    // one — it could only report the OS. `ui.motion` is Sucr's override, and the
    // proof is a computed transition on a real element: Full must bring the
    // durations back even while the OS says reduce, Reduced must collapse them,
    // and clearing hands the decision back to the install default.
    const railProbe = document.querySelector<HTMLElement>('.nav-item');
    const transitionOf = () => (railProbe ? getComputedStyle(railProbe).transitionDuration : 'no probe');
    const stillness = (spec: string) =>
      spec.split(',').every((part) => Number.parseFloat(part) <= 0.002);
    const osReduce = motionQuery.matches;
    await saveSetting('ui.motion', 'full');
    const fullSpec = transitionOf();
    await saveSetting('ui.motion', 'reduced');
    const reducedSpec = transitionOf();
    await saveSetting('ui.motion', null);
    const motion = {
      osReduce,
      full: fullSpec,
      reduced: reducedSpec,
      cleared: transitionOf(),
      // The attribute is what the stylesheet keys on; both readings are reported
      // so a failure says whether the switch or the CSS let it down.
      attribute: document.documentElement.dataset['motion'] ?? '',
      fullMoves: !stillness(fullSpec),
      reducedStills: stillness(reducedSpec),
      // A cleared key returns to the install default, which is Full.
      clearedIsFull: transitionOf() === fullSpec && !stillness(transitionOf()),
    };
    const added = await sucr.download.add({ url: 'http://127.0.0.1:9/not-a-real-file' });
    const capped = await sucr.download.setSpeedLimit(added.id, 131_072);
    const globalCleared = await sucr.app.setSpeedLimit(null);
    // A link that is not http(s) is turned down before a socket opens, so the
    // refresh channel's round trip needs no server — and the row must be left
    // exactly as it was, which is the promise the popover's error line rests on.
    const refreshReason = await sucr.download
      .refreshUrl(added.id, 'not a link')
      .then(() => '')
      .catch((e: unknown) => String(e));
    const rowAfterRefusal = await sucr.download.get(added.id);
    const refresh = {
      rejected: refreshReason !== '',
      urlKept: rowAfterRefusal?.url === added.url,
      statusKept: rowAfterRefusal?.status === 'queued',
    };
    const openedInSmoke = await sucr.download.open(added.id);
    const revealedInSmoke = await sucr.download.reveal(added.id);
    // The rail's five categories are rows now, so the gate can prove the writer
    // end to end: an extension files the row, an override moves it, a clear
    // un-files it, and a category that is not in the table is refused out loud.
    const archives = categoryRows.find((c) => c.name === 'Archives');
    const audio = categoryRows.find((c) => c.name === 'Audio');
    const zipped = await sucr.download.add({ url: 'http://127.0.0.1:9/package.zip' });
    const moved = await sucr.download.setCategory(zipped.id, audio?.id ?? -1);
    const cleared = await sucr.download.setCategory(zipped.id, null);
    const refuseReason = await sucr.download
      .setCategory(zipped.id, 999_999)
      .then(() => '')
      .catch((e: unknown) => String(e));
    const category = {
      seeded: categoryRows.map((c) => c.name),
      // A link with no extension has no rule to match, so it stays unfiled.
      unfiled: added.categoryId === null,
      filed: zipped.categoryId === archives?.id,
      archives: archives?.id ?? -1,
      moved: moved.categoryId === audio?.id,
      cleared: cleared.categoryId === null,
      refused: refuseReason !== '',
      reason: refuseReason,
    };
    const afterAdd = await sucr.download.list();
    await sucr.download.remove(added.id, false);
    await sucr.download.remove(zipped.id, false);
    const afterRemove = await sucr.download.list();

    // A pasted browser session is the one write that must never come back: the
    // list proves the shape of what is stored without the value. The host here is
    // a loopback URL, so the round trip also proves the link is reduced to the
    // identity a job looks up.
    const sessionBefore = await sucr.session.list();
    const savedSession = await sucr.session.set(
      'http://127.0.0.1:9/package.zip',
      'cf_clearance=smoke-token; other=1',
      'Mozilla/5.0 (smoke)',
    );
    const sessionAfter = await sucr.session.list();
    const sessionRefuseReason = await sucr.session
      .set('127.0.0.2', 'a cookie with no equals sign')
      .then(() => '')
      .catch((e: unknown) => String(e));
    const dropped = await sucr.session.forget('127.0.0.1');
    const sessionCleared = await sucr.session.list();
    const session = {
      host: savedSession.host,
      // The byte count is what the row shows, so it has to be the cookie's length.
      bytesKept: savedSession.cookieBytes === 'cf_clearance=smoke-token; other=1'.length,
      agentKept: savedSession.userAgent === 'Mozilla/5.0 (smoke)',
      listed: sessionAfter.length === sessionBefore.length + 1,
      noEcho: sessionAfter.every((s) => !('cookie' in s)),
      refused: sessionRefuseReason !== '',
      // A refused write must not leave a row behind.
      refusedLeftNothing: !sessionCleared.some((s) => s.host === '127.0.0.2'),
      dropped: dropped.forgotten,
      cleared: sessionCleared.length === sessionBefore.length,
    };

    // The site credential is the session's shape of proof with one addition: the
    // write is a click on the Save button, so the gate also watches the row list
    // repaint. Two hosts, because the store keeps one pair per host and dropping
    // one has to leave the other standing — and because the key is the host alone,
    // 127.0.0.1 and 127.0.0.2 are that isolation test while two ports of one host
    // would not be. Everything is forgotten at the end: a stored pair moves the
    // engine mirror for every check that runs after this one.
    const originBox = (id: string) => document.querySelector(id) as HTMLInputElement;
    const originList = async () => (await sucr.app.getInfo()).originAuths;
    const forgetRow = async (host: string): Promise<HTMLButtonElement | null> => {
      for (let i = 0; i < 40; i++) {
        const found = [...document.querySelectorAll<HTMLButtonElement>('#origin-rows button')].find(
          (b) => b.dataset['forgetOrigin'] === host,
        );
        if (found) return found;
        await new Promise((r) => setTimeout(r, 50));
      }
      return null;
    };
    // The rows are re-rendered *after* the note is written, so a note this check
    // settles on can name a state the list has not drawn yet — and when the same
    // host is written twice the note never changes at all. A row is therefore
    // settled on the row, the way the note is settled on the note.
    const rowShows = async (host: string, needle: string): Promise<boolean> => {
      for (let i = 0; i < 40; i++) {
        const text = [...document.querySelectorAll<HTMLElement>('#origin-rows .session-row')]
          .filter((row) =>
            [...row.querySelectorAll<HTMLButtonElement>('button')].some(
              (b) => b.dataset['forgetOrigin'] === host,
            ),
          )
          .map((row) => row.textContent ?? '')
          .join(' ');
        if (text.includes(needle)) return true;
        await new Promise((r) => setTimeout(r, 50));
      }
      return false;
    };
    const originBefore = await originList();
    document.querySelector<HTMLElement>('.nav-item[data-view="settings"]')?.click();
    originBox('#origin-host').value = 'http://127.0.0.1:9/package.zip';
    originBox('#origin-user').value = 'gate-user';
    originBox('#origin-pass').value = 'gate-secret';
    document.querySelector<HTMLElement>('#origin-save')?.click();
    const originLine = await noteHas('#origin-note', 'Saved for 127.0.0.1');
    // Read here, before the next write: the clear is part of the same commit, and
    // a later reading would only prove the last save cleared the boxes.
    const originBoxesEmpty =
      originBox('#origin-host').value === '' &&
      originBox('#origin-user').value === '' &&
      originBox('#origin-pass').value === '';
    const originShown = await originList();
    const originInfo = await sucr.app.getInfo();
    const originRow = originShown.find((o) => o.host === '127.0.0.1');
    originBox('#origin-host').value = '127.0.0.2';
    originBox('#origin-user').value = '';
    originBox('#origin-pass').value = 'gate-orphan';
    document.querySelector<HTMLElement>('#origin-save')?.click();
    const originRefused = await noteHas('#origin-note', 'a password needs a username');
    const originRefusedTone = document.querySelector('#origin-note')?.getAttribute('data-tone') ?? '';
    const originRefusedEmpty = !(await originList()).some((o) => o.host === '127.0.0.2');
    originBox('#origin-user').value = 'gate-user-2';
    originBox('#origin-pass').value = 'gate-secret-2';
    document.querySelector<HTMLElement>('#origin-save')?.click();
    await noteHas('#origin-note', 'Saved for 127.0.0.2');
    // A pair, then a token, for one third host: the store keeps a credential per host,
    // so the second write replaces the first instead of joining it — and the row that
    // comes back has to say which kind it is, because its username box is now empty.
    originBox('#origin-host').value = 'http://127.0.0.3:9/x.zip';
    originBox('#origin-user').value = 'gate-user-3';
    originBox('#origin-pass').value = 'gate-secret-3';
    document.querySelector<HTMLElement>('#origin-save')?.click();
    await noteHas('#origin-note', 'Saved for 127.0.0.3');
    document.querySelector<HTMLElement>('#origin-kind button[data-val="header-secret"]')?.click();
    const originUserHidden = originBox('#origin-user').hidden;
    originBox('#origin-host').value = 'http://127.0.0.3:9/x.zip';
    originBox('#origin-pass').value = 'gate-token-3';
    document.querySelector<HTMLElement>('#origin-save')?.click();
    await noteHas('#origin-note', 'Saved for 127.0.0.3');
    const originTokenRows = (await originList()).filter((o) => o.host === '127.0.0.3');
    const originTokenRow = originTokenRows[0];
    const originTokenNamed = await rowShows('127.0.0.3', 'bearer token');
    const originTokenInfo = await sucr.app.getInfo();
    void forgetRow('127.0.0.3').then((b) => b?.click());
    const originTokenDropped = await noteHas('#origin-note', 'Dropped the credential for 127.0.0.3');
    void forgetRow('127.0.0.1').then((b) => b?.click());
    const originDropped = await noteHas('#origin-note', 'Dropped the credential for 127.0.0.1');
    const originLeft = await originList();
    void forgetRow('127.0.0.2').then((b) => b?.click());
    await noteHas('#origin-note', 'Dropped the credential for 127.0.0.2');
    const originAfter = await originList();
    const origin = {
      line: originLine,
      // The link pasted into the box is not the key: what stores is the host.
      keyed: originRow?.host === '127.0.0.1',
      listed: originRow?.username === 'gate-user',
      bytesKept: originRow?.secretBytes === 'gate-secret'.length,
      noEcho:
        !JSON.stringify(originInfo).includes('gate-secret') &&
        !JSON.stringify(originShown).includes('gate-secret'),
      // A stored secret never comes back to a box, so the form is a replacement
      // field and a landed write leaves nothing typed behind it.
      boxesEmpty: originBoxesEmpty,
      refused: originRefused && originRefusedTone === 'error',
      refusedLeftNothing: originRefusedEmpty,
      dropped: originDropped,
      isolation: originLeft.length === 1 && originLeft[0]?.host === '127.0.0.2',
      cleared: originAfter.length === originBefore.length,
      // The token legs: the shape the form takes, the kind the row reports, one
      // credential per host, the word the row shows instead of a username, and the
      // same forget channel working on a row it never stored as a pair.
      tokenUserBoxHidden: originUserHidden,
      tokenKindListed: originTokenRow?.kind === 'header-secret',
      tokenReplacedPair: originTokenRows.length === 1 && originTokenRow?.username === '',
      tokenNamedInRow: originTokenNamed,
      tokenNoEcho: !JSON.stringify(originTokenInfo).includes('gate-token-3'),
      tokenDropped: originTokenDropped,
    };

    const domains = Object.keys(sucr).sort();
    const methods = Object.fromEntries(
      domains.map((d) => [
        d,
        Object.keys((sucr as unknown as Record<string, object>)[d] ?? {}).sort(),
      ]),
    );

    report.ok =
      channelsMatch &&
      info.journalMode === 'wal' &&
      info.schemaVersion === 4 &&
      downloads.length === 0 &&
      one === null &&
      // `network:list` answers with the adapter table and, beside it, the ledger
      // of bytes no adapter was credited with — a fresh library still reports
      // this machine's real routes, and reports zero unattributed. What the gate
      // can require is that every row carries the fields the deck renders, and
      // that the second figure is four numbers rather than an absent one: the
      // pane adds it into its totals, so an `undefined` there is a NaN on screen.
      networkRows.every(
        (n) =>
          Number.isInteger(n.id) &&
          n.displayName.length > 0 &&
          /^n\d+$/.test(n.colorId ?? '') &&
          /^\d+\.\d+\.\d+\.\d+$/.test(n.localIp) &&
          typeof n.enabled === 'boolean' &&
          typeof n.virtual === 'boolean',
      ) &&
      [
        netList.unattributed.today,
        netList.unattributed.week,
        netList.unattributed.month,
        netList.unattributed.lifetime,
      ].every((n) => Number.isInteger(n) && n >= 0) &&
      categoryRows.length === 5 &&
      category.unfiled &&
      category.filed &&
      category.moved &&
      category.cleared &&
      category.refused &&
      trail.length === 0 &&
      mapOfMissing === null &&
      pickedInSmoke === null &&
      !browseClick.errorShown &&
      browseClick.folderKept &&
      presets.chips === presets.resolved.length &&
      presets.resolved.length > 0 &&
      folderCreate.noticeShown &&
      folderCreate.missingWhileAbsent &&
      folderCreate.blockedWhileAbsent &&
      folderCreate.noticeCleared &&
      folderCreate.existsAfter &&
      folderCreate.fieldKept &&
      folderCreate.enabledAgain &&
      folderCreate.lateCreated &&
      folderCreate.probedLeftAlone &&
      folderCreate.fieldFollowsClick &&
      folderCreate.target1 !== '' &&
      folderCreate.target2 !== '' &&
      wizardGate.noticeShown &&
      wizardGate.nextBlocked &&
      wizardGate.nextLabel === 'Next' &&
      wizardGate.skipLive &&
      wizardGate.noticeCleared &&
      wizardGate.nextLive &&
      wizardGate.onePressedAfterClick &&
      wizardGate.fieldIsChip &&
      wizardGate.chipUnblocks &&
      netView.shown &&
      netView.cards === Math.max(networkRows.length, 1) &&
      netView.switches === networkRows.length &&
      limits.chip &&
      limits.opened &&
      limits.closed &&
      limits.stored === 2_097_152 &&
      limits.cleared === null &&
      tabs.length === 6 &&
      first === 'tab-overview' &&
      afterArrow === 'tab-connections' &&
      afterCtrl3 === 'tab-source' &&
      afterCtrl5 === 'tab-files' &&
      added.id > 0 &&
      refresh.rejected &&
      refresh.urlKept &&
      refresh.statusKept &&
      capped.speedLimit === 131_072 &&
      globalCapped.globalSpeedLimit === 262_144 &&
      settingsAfter['engine.max_active_downloads'] === '5' &&
      settingsCleared['engine.max_active_downloads'] === undefined &&
      motion.fullMoves &&
      motion.reducedStills &&
      motion.clearedIsFull &&
      globalCleared.globalSpeedLimit === null &&
      afterAdd.length === 2 &&
      session.host === '127.0.0.1' &&
      session.bytesKept &&
      session.agentKept &&
      session.listed &&
      session.noEcho &&
      session.refused &&
      session.refusedLeftNothing &&
      session.dropped &&
      session.cleared &&
      origin.line &&
      origin.keyed &&
      origin.listed &&
      origin.bytesKept &&
      origin.noEcho &&
      origin.boxesEmpty &&
      origin.refused &&
      origin.refusedLeftNothing &&
      origin.dropped &&
      origin.isolation &&
      origin.cleared &&
      origin.tokenUserBoxHidden &&
      origin.tokenKindListed &&
      origin.tokenReplacedPair &&
      origin.tokenNamedInRow &&
      origin.tokenNoEcho &&
      origin.tokenDropped &&
      proxy.ok &&
      afterRemove.length === 0;

    Object.assign(report, {
      channelsMatch,
      domains,
      methods,
      info,
      counts: { downloads: downloads.length, networks: networkRows.length, categories: categoryRows.length },
      adapters: networkRows.map((n) => ({
        name: n.displayName,
        kind: n.kind,
        ip: n.localIp,
        virtual: n.virtual,
        enabled: n.enabled,
        metric: n.ipv4Metric,
      })),
      mapNull: mapOfMissing === null,
      refresh,
      // Ruling 5: a clean --data-dir must read as a first run. Taken before any
      // of the writes above, and the backdrop id is what the real path opens.
      setupFirstRun: firstRun,
      wizardInDom: document.querySelector('#setup-wizard-backdrop') !== null,
      folderPick: pickedInSmoke === null ? 'refused under smoke' : String(pickedInSmoke),
      browse: browseClick,
      presets,
      folderCreate,
      wizardGate,
      clipboard,
      openReveal: { open: openedInSmoke, reveal: revealedInSmoke },
      networkView: netView,
      networkLimits: limits,
      proxy,
      write: { added: added.id, afterAdd: afterAdd.length, afterRemove: afterRemove.length },
      category,
      session,
      origin,
      speedLimit: {
        perJob: capped.speedLimit,
        globalSet: globalCapped.globalSpeedLimit,
        globalCleared: globalCleared.globalSpeedLimit,
      },
      settings: {
        after: settingsAfter,
        cleared: settingsCleared,
      },
      motion,
      tabs: tabs.length,
      pane: document.querySelector('[role="tabpanel"]:not([hidden])')?.id ?? null,
      keyboard: { first, afterArrow, afterCtrl3, afterCtrl5 },
      progressMode,
      title: document.title,
    });
  } catch (err) {
    report.error = err instanceof Error ? err.message : String(err);
  }

  await invoke('smoke_report', { report: JSON.stringify(report) }).catch(() => undefined);
}

void boot();
