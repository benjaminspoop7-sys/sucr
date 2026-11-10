// Sucr Capture — the add-on's only job is to carry a link to Sucr.
//
// It never reads the cookie store, the history or the profile wholesale. Three
// calls leave this file, all over loopback and all carrying the token the native
// messaging host handed it: /add (Sucr re-pulls the link), /adopt (the file
// already finished here) and /import (the browser is paused mid-transfer and
// its contiguous prefix becomes Sucr's staging file).
//
// One link-scoped exception: an attested stream (a Google Proof-of-Origin URL
// on googlevideo) only serves its bytes to the client that minted the token, so
// when the user hands such a row to Sucr the add-on forwards the identity the
// browser itself used on that request — its User-Agent and any Cookie — so the
// engine replays them instead of starting cold and stalling. Those values come
// from observing the request already flying to googlevideo for the file the user
// chose, are held only briefly, and are never read from the cookie store or sent
// anywhere but Sucr's loopback /add.

const HOST = 'app.sucr.capture';

/** @type {{port:number, token:string}|null} */
let door = null;
let doorAt = 0;
const DOOR_TTL = 30_000;

let settings = { enabled: true, mode: 'take' };
let sites = {}; // host -> false stands down there, the add-on's exclusion list
let altDown = false; // live prevent-key state: held Alt means browser only
let lastClick = null; // {url, tabId, alt, at}: which tab asked for this download
const panels = new Map(); // tabId -> {id, timer, deadline}: the in-page file info panel
const recent = []; // newest first, this browser session only

const CHROME_ID = 'hilnalhpghfpbphghnicaadhlegjaebf';

const extOrigin = () =>
  typeof browser !== 'undefined' ? browser.runtime.getURL('').replace(/\/$/, '') : `chrome-extension://${chrome.runtime.id}`;

const api = typeof browser !== 'undefined' ? browser : chrome;

// --- the ride (Leg A) ------------------------------------------------------
//
// A gated stream URL is only replayable by the client that minted its token, so
// we remember the identity the browser itself used on the request to googlevideo
// and hand it to the engine beside the link. Keyed by the video `id` the URL
// carries (stable across every format and across the live request), with a short
// TTL: the token is fresh only while the page plays.
const rideHeaders = new Map(); // id -> {ua, cookie, referer, at}
const RIDE_TTL = 120_000;
// The freshest identity seen on ANY googlevideo leg, regardless of the `id` the
// URL carried. Modern web players stream over SABR and may never make a classic
// `/videoplayback?id=` request, so the per-id map can stay empty for the very
// row a Take asks for. One browser session signs one googlevideo domain, so the
// newest cookie/UA the page itself used is a faithful stand-in for a same-page
// format whose `id` never appeared. Only the fresh values ride.
let lastGoogle = null; // {ua, cookie, referer, at}

const rideId = (url) => {
  try {
    return (String(url).match(/[?&]id=([^&]+)/) || [])[1] || '';
  } catch {
    return '';
  }
};

// The session a ride carries belongs to googlevideo only. Handing it to any
// other host would leak the browser's YouTube cookie to an unrelated link and
// show a session note on a plain file, so a ride is offered only for a link on
// that host.
const onGoogle = (url) => {
  try {
    return /(^|\.)googlevideo\.com$/i.test(new URL(String(url)).hostname);
  } catch {
    return false;
  }
};

if (api.webRequest && api.webRequest.onBeforeSendHeaders) {
  api.webRequest.onBeforeSendHeaders.addListener(
    (d) => {
      const pick = (name) =>
        (d.requestHeaders || []).find((h) => h.name.toLowerCase() === name)?.value;
      const head = {
        ua: pick('user-agent'),
        cookie: pick('cookie'),
        referer: pick('referer'),
        at: Date.now(),
      };
      lastGoogle = head;
      const id = rideId(d.url);
      if (id) rideHeaders.set(id, head);
    },
    { urls: ['https://*.googlevideo.com/*'] },
    // `extraHeaders` is what lets MV3 read Cookie/User-Agent off the live request.
    ['requestHeaders', 'extraHeaders'],
  );
}

// The saved identity for a link, if one is still fresh. A fresh per-`id` capture
// wins; failing that a fresh whole-session capture rides (same browser, same
// googlevideo domain), tagged `'last'` so the page can say the session rode. An
// absent or stale ride returns null so the engine falls back to the host session.
function rideFor(url) {
  if (!onGoogle(url)) return null;
  const fresh = (r) => r && Date.now() - r.at <= RIDE_TTL;
  const id = rideId(url);
  if (id) {
    const r = rideHeaders.get(id);
    if (r && fresh(r)) return { ...r, src: 'id' };
    if (r) rideHeaders.delete(id);
  }
  if (fresh(lastGoogle)) return { ...lastGoogle, src: 'last' };
  return null;
}

// --- settings --------------------------------------------------------------

async function loadSettings() {
  try {
    const stored = await api.storage.local.get(['enabled', 'mode', 'sites']);
    settings = {
      enabled: stored.enabled !== false,
      mode: ['take', 'adopt', 'import'].includes(stored.mode) ? stored.mode : 'take',
    };
    sites = stored.sites && typeof stored.sites === 'object' ? stored.sites : {};
  } catch {
    /* defaults are the safe answer */
  }
  return settings;
}

api.storage.onChanged.addListener((changes, area) => {
  if (area !== 'local') return;
  if (changes.enabled) settings.enabled = changes.enabled.newValue !== false;
  if (changes.mode && ['take', 'adopt', 'import'].includes(changes.mode.newValue)) {
    settings.mode = changes.mode.newValue;
  }
  if (changes.sites) sites = changes.sites.newValue && typeof changes.sites.newValue === 'object' ? changes.sites.newValue : {};
});

// --- the door --------------------------------------------------------------

function nativeCall(op, payload = {}) {
  return new Promise((resolve) => {
    let port;
    try {
      port = api.runtime.connectNative(HOST);
    } catch (e) {
      resolve({ ok: false, code: 'no-host', message: String(e && e.message ? e.message : e) });
      return;
    }
    const timer = setTimeout(() => {
      try { port.disconnect(); } catch {}
      resolve({ ok: false, code: 'timeout', message: 'Sucr did not answer in time' });
    }, 25_000);
    port.onMessage.addListener((msg) => {
      clearTimeout(timer);
      try { port.disconnect(); } catch {}
      resolve(msg || { ok: false, code: 'empty' });
    });
    port.onDisconnect.addListener(() => {
      clearTimeout(timer);
      const err = api.runtime.lastError;
      if (err) resolve({ ok: false, code: 'no-host', message: err.message });
    });
    try {
      port.postMessage({ op, ...payload });
    } catch (e) {
      clearTimeout(timer);
      resolve({ ok: false, code: 'no-host', message: String(e) });
    }
  });
}

async function findDoor({ wake = 'hello' } = {}) {
  // A visible wake must always reach the relay: the cached door says the app
  // is up, not that its window is. Only hello carries the /show ask.
  if (wake !== 'hello' && door && Date.now() - doorAt < DOOR_TTL) return door;
  const msg = await nativeCall(wake === false ? 'status' : wake);
  if (!msg || !msg.ok || !msg.port || !msg.token) {
    const e = new Error(msg && msg.code ? msg.code : 'not-running');
    e.code = msg && msg.code ? msg.code : 'not-running';
    throw e;
  }
  door = { port: msg.port, token: msg.token, startedAt: msg.startedAt };
  doorAt = Date.now();
  return door;
}

async function call(path, body, { wake = 'wake' } = {}) {
  let reached = false;
  const tryOnce = async () => {
    // A capture may start the app, but only hidden: `wake` brings up the tray
    // and the door with no window. `hello` (a visible window) belongs to the
    // popup's Open button alone.
    const d = await findDoor({ wake });
    reached = true;
    const init = {
      method: body === undefined ? 'GET' : 'POST',
      headers: { 'x-sucr-token': d.token, 'x-sucr-origin': extOrigin() },
      cache: 'no-store',
    };
    if (body !== undefined) {
      init.headers['content-type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    const res = await fetch(`http://127.0.0.1:${d.port}${path}`, init);
    let value = null;
    try { value = await res.json(); } catch {}
    return { res, value };
  };
  try {
    const { res, value } = await tryOnce();
    // A door that restarted (app quit and came back) answers 401 with the old
    // token in hand; re-ask the host once rather than surfacing a stale port.
    if (res.status === 401) {
      door = null;
      const retry = await tryOnce();
      return { ok: retry.res.ok, status: retry.res.status, body: retry.value };
    }
    return { ok: res.ok, status: res.status, body: value };
  } catch (e) {
    door = null;
    // The door was found but the port refused: the app died between discovery
    // and the request. One fresh discovery, then the refusal stands.
    if (reached) {
      try {
        const retry = await tryOnce();
        return { ok: retry.res.ok, status: retry.res.status, body: retry.value };
      } catch (e2) {
        return { ok: false, status: 0, body: { ok: false, code: e2.code || 'unreachable', message: String(e2.message || e2) } };
      }
    }
    return { ok: false, status: 0, body: { ok: false, code: e.code || 'unreachable', message: String(e.message || e) } };
  }
}

// --- badge -----------------------------------------------------------------

function badge(text, tone) {
  api.action.setBadgeText({ text });
  if (tone) api.action.setBadgeBackgroundColor({ color: tone });
}
function record(entry) {
  recent.unshift({ ...entry, at: Date.now() });
  if (recent.length > 12) recent.pop();
  const taken = recent.filter((r) => r.ok).length;
  if (taken > 0) badge(String(taken), '#2f7d4f');
  else badge('');
}

// --- attribution and exclusions --------------------------------------------

const hostOf = (u) => {
  try { return new URL(u).host; } catch { return ''; }
};
const siteOff = (u) => sites[hostOf(u)] === false;

// A download is attributed to the click that caused it; the tab id is what
// lets the verdict and the file info panel land in the page that asked. Only
// an exact URL match: the page's own URL would claim every later download.
function clickFor(url) {
  if (lastClick && Date.now() - lastClick.at < 6_000 && lastClick.url === url) return lastClick;
  return null;
}

// --- the in-page file info panel --------------------------------------------

const rowFacts = (row) => ({
  status: row.status,
  filename: row.filename,
  folder: row.folder,
  totalBytes: row.totalBytes,
  wantedBytes: row.wantedBytes,
  verifiedBytes: row.verifiedBytes,
  errorCode: row.errorCode,
  errorHint: row.errorHint,
});

function closePanel(tabId) {
  const st = panels.get(tabId);
  if (st) clearTimeout(st.timer);
  panels.delete(tabId);
}

function openPanel(tabId, id, filename, totalBytes, phase, session) {
  if (tabId == null) return;
  closePanel(tabId);
  const st = { id: id == null ? null : id, timer: 0, deadline: Date.now() + 180_000 };
  panels.set(tabId, st);
  api.tabs.sendMessage(tabId, { op: 'sucr-panel', id: st.id, filename: filename || '', totalBytes: totalBytes == null ? null : totalBytes, phase, session: session || null }).catch(() => {});
  if (st.id != null) pollPanel(tabId);
}

function pollPanel(tabId) {
  const st = panels.get(tabId);
  if (!st || st.id == null) return;
  st.timer = setTimeout(async () => {
    if (!panels.has(tabId)) return;
    const out = await call(`/status?id=${st.id}`, undefined, { wake: false });
    const row = out.ok && out.body && out.body.row;
    if (row) {
      api.tabs.sendMessage(tabId, { op: 'sucr-panel-row', row: rowFacts(row) }).catch(() => {});
      const running = row.status === 'queued' || row.status === 'downloading' || row.status === 'paused';
      if (running && Date.now() < st.deadline) pollPanel(tabId);
    } else if (Date.now() < st.deadline) {
      pollPanel(tabId);
    }
  }, 1_000);
}

function panelError(tabId, code, message) {
  if (tabId != null && panels.has(tabId)) {
    api.tabs.sendMessage(tabId, { op: 'sucr-panel-row', row: { status: 'needs-attention', errorCode: code, errorHint: message } }).catch(() => {});
    return true;
  }
  return false;
}

// --- modes -----------------------------------------------------------------

function baseName(p) {
  if (!p) return '';
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return i >= 0 ? p.slice(i + 1) : p;
}

// A single safe file name for the browser's Downloads folder: drop any directory
// component, replace the characters Windows forbids plus control codes with a
// space, strip leading dots, and bound the length. Empty input yields no name,
// so Chrome auto-names as before.
function safeDownloadName(name) {
  const bad = new Set(['<', '>', ':', '"', '/', '\\', '|', '?', '*']);
  for (let c = 0; c < 32; c++) bad.add(String.fromCharCode(c));
  let out = '';
  for (const ch of baseName(name || '')) out += bad.has(ch) ? ' ' : ch;
  return out.replace(/^\.+/, '').trim().slice(0, 200);
}

function searchOne(id) {
  return api.downloads.search({ id }).then(([live]) => live || null, () => null);
}

/// True once Sucr's own row finished holding exactly the bytes the browser wrote.
/// A browser download that completed before Sucr answered leaves two copies of
/// one file, and only Sucr can say it is safe to drop the browser's. Bounded: a
/// row that has not finished within the window leaves the user's copy in place.
async function sucr_holds(id, bytes) {
  const deadline = Date.now() + 6_000;
  while (Date.now() < deadline) {
    const out = await call(`/status?id=${id}`, undefined, { wake: false });
    const row = out.ok && out.body && out.body.row;
    if (row && (row.status === 'finished' || row.status === 'verified')) {
      return row.verifiedBytes === bytes;
    }
    if (row && row.status !== 'downloading' && row.status !== 'queued') return false;
    await new Promise((r) => setTimeout(r, 300));
  }
  return false;
}

async function take({ url, filename, referer, tabUrl, ua: pageUa, audioUrl }) {
  const ride = rideFor(url);
  // A token minted by the page's own same-origin fetch is coherent with the
  // desktop UA that fetched it, so replay THAT, not the engine's default. Prefer
  // the UA the browser actually put on the googlevideo leg; fall back to the
  // page's navigator.userAgent the content script passed up.
  const ua = (ride && ride.ua) || pageUa || undefined;
  const out = await call('/add', {
    url,
    filename: filename || undefined,
    referer: referer || (ride && ride.referer) || tabUrl || undefined,
    // The identity the browser used on this very stream, if we caught it fresh:
    // the engine replays it so an attested row finishes instead of stalling.
    ua,
    cookie: (ride && ride.cookie) || undefined,
    // The sound half of a DASH pair: Sucr fetches it under the same row and
    // publishes one joined file, so no second download ever appears.
    audioUrl: audioUrl || undefined,
  });
  if (out.ok) {
    record({ ok: true, mode: 'take', url, filename, id: out.body && out.body.id, ride: (ride && ride.src) || 'none' });
  } else {
    record({ ok: false, mode: 'take', url, filename, code: out.body && out.body.code, message: out.body && out.body.message });
  }
  // Tell the caller which identity rode so the panel can say, in one line,
  // whether the browser session was found — the one signal that distinguishes a
  // still-gated row (session rode, Google refused) from a signed-out row (no
  // session to ride) without the user checking each format by hand.
  out.ride = (ride && ride.src) || 'none';
  return out;
}

// The chip's route. A link Sucr can fetch itself is taken. A link it cannot —
// usually a session cookie the browser holds and the app does not — is fetched
// by the browser instead and the finished file is adopted, because the browser
// is the one client that already has the page's credentials.
async function offer({ url, filename, referer, tabUrl, tabId, ua }) {
  if (!/^https?:/i.test(url || '')) {
    return { ok: false, body: { code: 'scheme', message: 'only http and https links can be captured' } };
  }
  if (siteOff(url)) {
    return { ok: false, body: { code: 'site-off', message: 'Sucr is standing down on this site' } };
  }
  if (settings.mode === 'take') {
    const probe = await call('/probe', { url });
    // A probe the app could not answer at all says nothing about the link; only
    // a probe that ran and refused it moves the fetch to the browser.
    const refused = probe.status === 200 && probe.body && probe.body.ok === false;
    if (!refused) {
      const out = await take({ url, filename, referer, tabUrl, ua });
      if (out.ok) openPanel(tabId, out.body && out.body.id, baseName(filename), probe.body && probe.body.totalBytes, 'take', out.ride);
      return out;
    }
  }
  return fromBrowser({ url, referer, tabId, filename });
}

// The chip's own toast answers the click; the verdict on a browser-fetched
// file arrives seconds later, when the pointer is long gone. Say it in the
// page anyway: a silent failure is the complaint this add-on exists to end.
function sayTab(tabId, text, bad) {
  // An open panel already carries the verdict; a toast on top would say it twice.
  if (tabId == null || panels.has(tabId)) return;
  api.tabs.sendMessage(tabId, { op: 'sucr-say', text, bad }).catch(() => {});
}

// A failed browser leg is a notice, never a silent no-op. The panel carries
// the verdict when one is open; otherwise the page gets a toast.
function browserFailed(tabId, code, message) {
  api.action.setBadgeText({ text: '!' });
  api.action.setBadgeBackgroundColor({ color: '#a4441f' });
  if (!panelError(tabId, code, message)) sayTab(tabId, 'Sucr could not take this file', true);
}

// The browser wrote the file; hand it to Sucr and answer in the page.
async function adoptDone(item, tabId) {
  const out = await adopt(item).catch((e) => ({ ok: false, body: { code: 'adopt', message: String((e && e.message) || e) } }));
  if (out && out.ok) {
    openPanel(tabId, out.body && out.body.id, baseName(item.filename), item.fileSize, 'adopt');
  } else {
    browserFailed(tabId, (out.body && out.body.code) || 'adopt', (out.body && out.body.message) || 'Sucr refused this file');
  }
  api.downloads.erase({ id: item.id }).catch(() => {});
}

// Watch a browser download to completion, then adopt it. Shared by the adopt
// mode, the chip's session-link fallback and intercept's probe refusal.
function watchAdopt(item, tabId) {
  const onChanged = (delta) => {
    if (delta.id !== item.id || !delta.state) return;
    if (delta.state.current === 'interrupted') {
      api.downloads.onChanged.removeListener(onChanged);
      record({ ok: false, mode: 'adopt', url: item.url, code: 'browser-failed', message: 'the browser could not finish this file' });
      browserFailed(tabId, 'browser-failed', 'the browser could not finish this file');
      return;
    }
    if (delta.state.current !== 'complete') return;
    api.downloads.onChanged.removeListener(onChanged);
    searchOne(item.id).then((live) => { if (live) adoptDone(live, tabId); });
  };
  api.downloads.onChanged.addListener(onChanged);
  // Catch a download that finished between creation and this listener.
  searchOne(item.id).then((live) => {
    if (live && live.state === 'complete') {
      api.downloads.onChanged.removeListener(onChanged);
      adoptDone(live, tabId);
    }
  });
}

// Let the browser download it, then adopt the file it wrote. The download is
// ours, so `onCreated` skips it; completion is watched here instead.
async function fromBrowser({ url, referer, tabId, filename }) {
  let id;
  // A download this add-on starts is still a browser download, and onCreated
  // fires for it before `download` resolves the id. The URL is the only handle
  // available at that moment, so the listener below skips anything matching it
  // rather than taking the same link a second time.
  adoptArmed = { url, at: Date.now() };
  openPanel(tabId, null, baseName(filename), null, 'browser');
  const name = safeDownloadName(filename);
  try {
    id = await api.downloads.download(name ? { url, filename: name, conflictAction: 'uniquify' } : { url, conflictAction: 'uniquify' });
  } catch (e) {
    adoptArmed = null;
    const message = String((e && e.message) || e);
    record({ ok: false, mode: 'adopt', url, code: 'browser-refused', message });
    browserFailed(tabId, 'browser-refused', message);
    return { ok: false, body: { code: 'browser-refused', message } };
  }
  watchAdopt({ id, url }, tabId);
  return { ok: true, pending: true, body: { mode: 'adopt' } };
}

async function intercept(item, tabId) {
  if (!settings.enabled) return;
  const url = item.finalUrl || item.url;
  if (!/^https?:/i.test(url)) return;

  if (settings.mode === 'take') {
    // Ask the app before acting: a link Sucr cannot fetch itself
    // (a session cookie the browser holds) moves to the browser leg instead
    // of leaving a failed row and an orphan browser copy.
    const probe = await call('/probe', { url });
    // A cold probe takes seconds; attribution may have arrived since onCreated.
    if (tabId == null) {
      const c = clickFor(url);
      if (c) tabId = c.tabId;
      else { const [live] = await api.downloads.search({ id: item.id }); if (live && live.tabId != null) tabId = live.tabId; }
    }
    const refused = probe.status === 200 && probe.body && probe.body.ok === false;
    if (refused) {
      openPanel(tabId, null, baseName(item.filename) || baseName(url.split('?')[0]), probe.body && probe.body.totalBytes, 'browser');
      watchAdopt(item, tabId);
      return;
    }
    const out = await take({ url: item.url, filename: baseName(item.filename) || (probe.body && probe.body.filename) || '', referer: item.referrer });
    // Nothing was cancelled until Sucr answered yes. A refusal leaves the
    // browser's own download exactly where it was.
    if (out.ok) {
      const id = out.body && out.body.id;
      let live = await searchOne(item.id);
      if (!live || live.state !== 'complete') {
        try { await api.downloads.cancel(item.id); } catch {}
        // A small file can finish between the search and the cancel, and
        // cancel is a no-op on a complete item: re-check before leaving two
        // copies of one file behind.
        live = await searchOne(item.id);
      }
      if (live && live.state === 'complete') {
        // No connection left to cancel. Drop the browser's copy only on proof,
        // otherwise both copies stay -- the worse outcome only when Sucr cannot
        // show it holds the same bytes. Measured on a real Chromium leg: this
        // build's `removeFile(downloadId, callback)` wants the bare integer, and
        // an options object throws "No matching signature" (there is no
        // downloads.remove at all). Pass the id positionally.
        if (id != null && (await sucr_holds(id, live.fileSize))) {
          try { await api.downloads.removeFile(item.id); } catch {}
        }
      }
      try { await api.downloads.erase({ id: item.id }); } catch {}
      badge('↓', '#2f7d4f');
      setTimeout(() => badge(recent.filter((r) => r.ok).length ? String(recent.filter((r) => r.ok).length) : '', '#2f7d4f'), 1600);
      openPanel(tabId, id, baseName(item.filename) || (probe.body && probe.body.filename) || '', probe.body && probe.body.totalBytes, 'take');
    } else {
      sayTab(tabId, (out.body && out.body.message) || 'Sucr refused this one', true);
    }
    return;
  }

  if (settings.mode === 'adopt') {
    watchAdopt(item, tabId);
    return;
  }

  // import: pause first, then hand over the bytes the browser already wrote.
  try { await api.downloads.pause(item.id); } catch {}
  // Let Chromium flush and close the handle before the prefix is read.
  await new Promise((r) => setTimeout(r, 400));
  const [live] = await api.downloads.search({ id: item.id });
  const path = (live && live.filename) || item.filename;
  const out = await call('/import', {
    url: item.url,
    path,
    filename: baseName(path),
    referer: item.referrer || undefined,
  });
  if (out.ok) {
    try { await api.downloads.cancel(item.id); } catch {}
    try { await api.downloads.erase({ id: item.id }); } catch {}
    record({ ok: true, mode: 'import', url: item.url, filename: baseName(path), id: out.body && out.body.id });
    openPanel(tabId, out.body && out.body.id, baseName(path), live && live.totalBytes, 'import');
  } else {
    try { await api.downloads.resume(item.id); } catch {}
    record({ ok: false, mode: 'import', url: item.url, filename: baseName(path), code: out.body && out.body.code, message: out.body && out.body.message });
    // A refusal is a notice, never a silent no-op.
    browserFailed(tabId, (out.body && out.body.code) || 'import', (out.body && out.body.message) || 'Sucr refused this file');
  }
}

async function adopt(item) {
  const path = item.filename;
  const out = await call('/adopt', {
    url: item.finalUrl || item.url,
    path,
    filename: baseName(path),
    referer: item.referrer || undefined,
  });
  record({
    ok: out.ok,
    mode: 'adopt',
    url: item.finalUrl || item.url,
    filename: baseName(path),
    id: out.body && out.body.id,
    code: out.body && out.body.code,
    message: out.body && out.body.message,
  });
  return out;
}

// --- YouTube per-moment retry ---------------------------------------------
//
// Google guards the adaptive (high-res) tiers per video with a Proof-of-Origin
// token: on a gated id the URL answers 206 only for a ~12-16 MiB free window
// and 403s every later Range, while the same format serves the whole file once
// Google lifts the gate for that video and moment. The gate is not permanent,
// so a protected row can be pulled complete by re-probing later and taking the
// first tick where the whole file answers 206. This holds no cookies and mints
// no attestation: it only retries the same page-context pull until the window
// opens. `chrome.alarms` is the timer an MV3 worker survives; the job list
// lives in storage, so a worker restart keeps watching.

const RETRY_ALARM = 'sucr-yt-retry';
const RETRY_PERIOD_MIN = 5; // re-check cadence; Chrome clamps very short periods.
const RETRY_MAX_AGE_MS = 24 * 3600 * 1000; // give up after a day of watching.
const YT_WALL = 12582912; // ~12 MiB: past it, a protected adaptive URL 403s.

async function getJobs() {
  try {
    const s = await api.storage.local.get('retryJobs');
    return Array.isArray(s.retryJobs) ? s.retryJobs : [];
  } catch {
    return [];
  }
}
async function setJobs(jobs) {
  try {
    await api.storage.local.set({ retryJobs: jobs });
  } catch { /* storage is best-effort here; a lost job just stops watching */ }
}

// The content script runs in an isolated world, so it cannot read the page's
// ytcfg or anything built on it. This helper runs in the page's own world:
// it reads the desktop innertube context (with its visitorData) and returns
// the desktop ladder plus the visionOS ladder that carries direct URLs,
// trimmed to the fields the list renders. Run on every list open.
async function ytLadders(tabId, videoId) {
  if (tabId == null) return null;
  let inj;
  try {
    inj = await api.scripting.executeScript({
      target: { tabId },
      world: 'MAIN',
      func: async (vid) => {
        const cfg = window.ytcfg && window.ytcfg.data_ && window.ytcfg.data_.INNERTUBE_CONTEXT;
        if (!cfg) return null;
        const vis = (cfg.client && cfg.client.visitorData) || '';
        const post = async (body) => {
          try {
            const r = await fetch('/youtubei/v1/player?prettyPrint=false', {
              method: 'POST',
              headers: { 'content-type': 'application/json', 'X-Goog-Visitor-Id': vis },
              body: JSON.stringify(body),
            });
            if (!r.ok) return null;
            const j = await r.json();
            if (!j.playabilityStatus || j.playabilityStatus.status !== 'OK' || !j.streamingData) return null;
            const pick = (f) => ({
              itag: f.itag,
              url: f.url || '',
              mimeType: f.mimeType || '',
              qualityLabel: f.qualityLabel || '',
              quality: f.quality || '',
              height: f.height || 0,
              fps: f.fps || 0,
              bitrate: f.bitrate || 0,
              contentLength: f.contentLength || '',
            });
            return {
              title: (j.videoDetails && j.videoDetails.title) || '',
              formats: (j.streamingData.formats || []).map(pick),
              adaptive: (j.streamingData.adaptiveFormats || []).map(pick),
            };
          } catch {
            return null;
          }
        };
        const web = await post({ context: cfg, videoId: vid });
        const c = { ...cfg.client };
        delete c.originalUrl;
        delete c.timeZone;
        delete c.utcOffsetMinutes;
        Object.assign(c, {
          clientName: 'VISIONOS',
          clientVersion: '1.02',
          deviceMake: 'Apple',
          deviceModel: 'RealityDevice17,1',
          osName: 'visionOS',
          osVersion: '26.5.23O471',
          hl: 'en',
          gl: 'US',
          visitorData: vis,
        });
        const vision = await post({ context: { client: c }, videoId: vid });
        return { web, vision };
      },
      args: [videoId],
    });
  } catch {
    return null;
  }
  return (inj && inj[0] && inj[0].result) || null;
}

// Re-derive the format row from the PAGE, not the worker. Measured on this
// build: the same player call made cross-origin from the service worker 403s,
// because it carries none of the page's origin, referrer or session; issued
// from inside the open watch tab via chrome.scripting.executeScript it returns
// 200 with the whole adaptive ladder. The call mirrors the desktop page: its
// own innertube context (client fields swapped to visionOS, which serves
// direct URLs where the desktop web client only returns SABR ids) plus the
// page's visitorData, without which Google answers LOGIN_REQUIRED. So the
// worker owns the timer, the googlevideo Range probe and the take, and borrows
// the page only to run a fetch the page already makes for its own player. No
// cookie is read, nothing is forged, and the URL handed back is the same
// self-signed one a Take already uses. Returns the row's direct URL and length
// for that itag, or a state: 'tab-gone' (watch tab closed or navigated ->
// retry next tick), 'retry' (the page call failed transiently), 'missing'
// (Google no longer offers that format -> stop watching it).
async function ytRowFromPage(tabId, videoId, itag) {
  if (tabId == null) return { state: 'tab-gone' };
  let inj;
  try {
    inj = await api.scripting.executeScript({
      target: { tabId },
      world: 'MAIN',
      func: async (vid, want) => {
        try {
          const cfg = window.ytcfg && window.ytcfg.data_ && window.ytcfg.data_.INNERTUBE_CONTEXT;
          if (!cfg) return { state: 'retry' };
          const vis = (cfg.client && cfg.client.visitorData) || '';
          const c = { ...cfg.client };
          delete c.originalUrl;
          delete c.timeZone;
          delete c.utcOffsetMinutes;
          Object.assign(c, {
            clientName: 'VISIONOS',
            clientVersion: '1.02',
            deviceMake: 'Apple',
            deviceModel: 'RealityDevice17,1',
            osName: 'visionOS',
            osVersion: '26.5.23O471',
            hl: 'en',
            gl: 'US',
            visitorData: vis,
          });
          const r = await fetch('/youtubei/v1/player?prettyPrint=false', {
            method: 'POST',
            headers: { 'content-type': 'application/json', 'X-Goog-Visitor-Id': vis },
            body: JSON.stringify({ context: { client: c }, videoId: vid }),
          });
          if (!r.ok) return { state: 'retry' };
          const j = await r.json();
          if (!j.streamingData || (j.playabilityStatus && j.playabilityStatus.status !== 'OK')) return { state: 'retry' };
          const list = [...(j.streamingData.formats || []), ...(j.streamingData.adaptiveFormats || [])];
          const f = list.find((x) => x.itag === want && x.url);
          if (!f) return { state: 'missing' };
          return { state: 'row', url: f.url, clen: Number(f.contentLength) || 0 };
        } catch {
          return { state: 'retry' };
        }
      },
      args: [videoId, itag],
    });
  } catch {
    return { state: 'tab-gone' }; // no host permission or the tab is gone.
  }
  return (inj && inj[0] && inj[0].result) || { state: 'retry' };
}

async function ytProbe(url, offset) {
  try {
    const c = new AbortController();
    const t = setTimeout(() => c.abort(), 8000);
    const r = await fetch(url, { headers: { Range: `bytes=${offset}-${offset + 15}`, 'Accept-Encoding': 'identity' }, signal: c.signal });
    clearTimeout(t);
    return r.status;
  } catch {
    return 0;
  }
}

// A row is open only when the first byte and a byte near the very end both
// answer 206 — the same two-range test content.js uses to label protection. A
// 0 (googlevideo unreachable) is inconclusive, so the caller keeps waiting
// rather than guessing and stalling a real download at ~2%.
async function ytRowOpen(job) {
  const row = await ytRowFromPage(job.tabId, job.videoId, job.itag);
  if (row.state !== 'row') return row; // 'tab-gone' | 'retry' | 'missing'
  const first = await ytProbe(row.url, 0);
  if (!first) return { state: 'retry' };
  if (first !== 206) return { state: 'protected' };
  if (row.clen > YT_WALL) {
    const tail = await ytProbe(row.url, row.clen - 65536);
    if (!tail) return { state: 'retry' };
    if (tail !== 206) return { state: 'protected' };
  }
  return { state: 'open', url: row.url, clen: row.clen };
}

// A ladder row is named `<title>-<tier>-<codec>-<itag>.<ext>`; the joined file
// keeps the title. Same rule content.js applies on a Take.
function mergedName(name) {
  return (name || '').replace(/-[^-]+-[a-z0-9]+-\d+(?=\.[^.]+$)/i, '');
}

async function runRetry() {
  const jobs = await getJobs();
  if (!jobs.length) {
    try { await api.alarms.clear(RETRY_ALARM); } catch {}
    return;
  }
  const keep = [];
  for (const job of jobs) {
    if (Date.now() - (job.addedAt || 0) > RETRY_MAX_AGE_MS) {
      sayTab(job.tabId, 'Sucr stopped watching that quality — the gate never opened', true);
      continue;
    }
    const res = await ytRowOpen(job);
    if (res.state === 'open') {
      // The sound is resolved fresh at the moment the gate opens, because its
      // URL is as short-lived as the picture's. A watched row is never taken
      // alone: a missing sound leaves it watching rather than producing a
      // silent file, and a sound that appeared is handed over with the picture
      // so the engine fetches both under one row.
      let sound = null;
      if (job.pairItag) {
        const srow = await ytRowFromPage(job.tabId, job.videoId, job.pairItag);
        if (srow.state === 'missing') continue; // Google dropped the sound row too
        if (srow.state !== 'row' || !srow.url) { keep.push(job); continue; }
        sound = srow.url;
      }
      const out = await take({
        url: res.url,
        filename: sound ? mergedName(job.filename) : job.filename,
        audioUrl: sound || undefined,
        referer: job.referer,
      });
      if (out.ok) {
        openPanel(job.tabId, out.body && out.body.id, baseName(out.body && out.body.filename), null, 'take', out.ride);
        sayTab(job.tabId, 'Sucr caught the window — taking ' + (job.filename || 'that quality'), false);
        continue;
      }
      keep.push(job); // the Take refused (app not running, site off): keep watching.
    } else if (res.state !== 'missing') {
      keep.push(job); // tab closed, transient page failure, protected, or an
                      // inconclusive probe: all wait for the next tick.
    }
    // 'missing' (Google no longer offers that format) drops the watch.
  }
  await setJobs(keep);
  if (!keep.length) {
    try { await api.alarms.clear(RETRY_ALARM); } catch {}
  }
}

async function armRetry() {
  const jobs = await getJobs();
  if (!jobs.length) return;
  try {
    await api.alarms.create(RETRY_ALARM, { periodInMinutes: RETRY_PERIOD_MIN });
  } catch { /* a clamp or a missing permission only delays the next tick */ }
}

api.alarms.onAlarm.addListener((a) => {
  if (a && a.name === RETRY_ALARM) runRetry().catch((e) => console.warn('sucr retry:', e));
});

api.downloads.onCreated.addListener(async (item) => {
  if (item.byExtensionId === api.runtime.id) return;
  const armed = adoptArmed;
  if (armed && Date.now() - armed.at < 20_000 && (item.url === armed.url || item.finalUrl === armed.url)) {
    // Our own fallback download: the watcher it started adopts the file.
    adoptArmed = null;
    handled.add(item.id);
    return;
  }
  handled.add(item.id);
  // The page's click message can trail the download event when both wake an
  // idle worker, and a cold native spawn blocks the queue for seconds; give
  // attribution a brief moment so the prevent key and the tab id are not lost.
  let click = clickFor(item.url);
  for (let i = 0; i < 5 && !click && !altDown; i++) {
    await new Promise((r) => setTimeout(r, 60));
    click = clickFor(item.url);
  }
  // The prevent key: a held Alt means the browser keeps this file. The
  // click that caused it carries the same flag for the window after keyup.
  if (altDown || (click && click.alt)) return;
  if (siteOff(item.finalUrl || item.url)) return;
  intercept(item, click && click.tabId).catch((e) => console.warn('sucr capture:', e));
});

// A download that begins while the browser is still starting can beat this
// script to its own event: onCreated never fires for it, and the file silently
// finishes in the browser. Measured on a real startup, not assumed. Searching
// the store on every wake is the only way to see that download, and `handled`
// keeps the two paths from taking the same file twice.
const handled = new Set();

async function sweep() {
  if (!settings.enabled || !api.downloads || !api.downloads.search) return;
  const since = new Date(Date.now() - 120_000).toISOString();
  let items;
  try {
    items = await api.downloads.search({ limit: 10, startedAfter: since });
  } catch {
    return;
  }
  for (const item of items || []) {
    if (item.byExtensionId === api.runtime.id) continue;
    if (handled.has(item.id)) continue;
    // An interrupted transfer is neither cancellable nor complete, so it is
    // the one state with nothing to hand over.
    if (item.state !== 'in_progress' && item.state !== 'complete') continue;
    if (!/^https?:/i.test(item.finalUrl || item.url || '')) continue;
    if (siteOff(item.finalUrl || item.url)) continue;
    handled.add(item.id);
    try {
      // Already finished means the boot window beat this script and there is no
      // connection left to cancel, so taking is impossible: the file the browser
      // wrote is adopted instead. Measured on a real startup, where a small file
      // completes before the first event can fire.
      if (item.state === 'complete' && settings.mode === 'take') await adopt(item);
      else await intercept(item);
    } catch (e) {
      console.warn('sucr sweep:', e);
    }
  }
}

api.runtime.onStartup.addListener(() => loadSettings().then(sweep).then(armRetry));

// The fallback download this add-on started, if one is in flight.
let adoptArmed = null;

// The page-side chip, video panel and file info panel live in content.js; this
// side answers their messages. Alt is the prevent key (the browser keeps the
// file). No context menu: the chip is the affordance.

// --- popup -----------------------------------------------------------------

api.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  (async () => {
    switch (msg && msg.op) {
      case 'state': {
        await loadSettings();
        let live = null;
        try {
          const d = await findDoor({ wake: false });
          live = { port: d.port, startedAt: d.startedAt };
        } catch (e) {
          live = { error: e.code || 'not-running' };
        }
        sendResponse({ ok: true, settings, sites, door: live, recent, id: api.runtime.id });
        break;
      }
      case 'page-click': {
        lastClick = { url: msg.url, tabId: sender.tab && sender.tab.id, alt: !!msg.alt, at: Date.now() };
        sendResponse({ ok: true });
        break;
      }
      case 'keys': {
        altDown = !!msg.alt;
        sendResponse({ ok: true });
        break;
      }
      case 'panel-close': {
        closePanel(sender.tab && sender.tab.id);
        sendResponse({ ok: true });
        break;
      }
      case 'site': {
        const host = hostOf(msg.url || '');
        if (!host) {
          sendResponse({ ok: false, code: 'host', message: 'no site to stand down on' });
          break;
        }
        sites[host] = msg.on !== false;
        await api.storage.local.set({ sites });
        sendResponse({ ok: true, sites });
        break;
      }
      case 'wake': {
        try {
          const d = await findDoor({ wake: 'hello' });
          sendResponse({ ok: true, port: d.port });
        } catch (e) {
          sendResponse({ ok: false, code: e.code || 'not-running', message: String(e.message || e) });
        }
        break;
      }
      case 'take': {
        const tabId = sender.tab && sender.tab.id;
        // A DASH pair is one hand-off: the picture's link and its sound go to
        // Sucr together, and the engine fetches and joins both under a single
        // row. Probing or falling back to the browser here would split it back
        // into two downloads, so this path never does either.
        if (msg.audioUrl) {
          if (!/^https?:/i.test(msg.url || '')) {
            sendResponse({ ok: false, body: { code: 'scheme', message: 'only http and https links can be captured' } });
            break;
          }
          if (siteOff(msg.url)) {
            sendResponse({ ok: false, body: { code: 'site-off', message: 'Sucr is standing down on this site' } });
            break;
          }
          const out = await take({ url: msg.url, filename: msg.filename, audioUrl: msg.audioUrl, referer: msg.referer, ua: msg.ua, tabUrl: sender.tab && sender.tab.url });
          if (out.ok) openPanel(tabId, out.body && out.body.id, baseName(msg.filename), null, 'take', out.ride);
          sendResponse(out);
          break;
        }
        const out = await offer({ url: msg.url, filename: msg.filename, referer: msg.referer, ua: msg.ua, tabUrl: sender.tab && sender.tab.url, tabId });
        sendResponse(out);
        break;
      }
      case 'yt-ladders': {
        sendResponse(await ytLadders(sender.tab && sender.tab.id, msg.id));
        break;
      }
      case 'probe': {
        // Range-check a list of {url, offset} pairs so the video list only ever
        // offers formats the engine can actually pull past their first chunk.
        // Probe under the SAME identity the engine will ride (the browser's own
        // UA + any fresh googlevideo cookie), so a row that a session-riding
        // pull can finish is not mislabelled `protected` by a cookieless probe.
        const probeHeaders = (p) => {
          const h = { Range: `bytes=${p.offset}-${p.offset + 15}`, 'Accept-Encoding': 'identity' };
          const ride = rideFor(p.url);
          const ua = msg.ua || (ride && ride.ua);
          if (ua) h['User-Agent'] = ua;
          if (ride && ride.cookie) h['Cookie'] = ride.cookie;
          return h;
        };
        const codes = await Promise.all((msg.probes || []).slice(0, 48).map(async (p) => {
          try {
            const c = new AbortController();
            const t = setTimeout(() => c.abort(), 8000);
            const r = await fetch(p.url, { headers: probeHeaders(p), signal: c.signal });
            clearTimeout(t);
            return r.status;
          } catch {
            return 0;
          }
        }));
        sendResponse({ ok: true, codes });
        break;
      }
      case 'retry': {
        // Enroll a protected row to re-probe on a timer and pull it whole
        // when Google lifts the gate. Replacing any job for the same video +
        // itag keeps one watch per format instead of a pile of duplicates.
        const jobs = await getJobs();
        const clean = jobs.filter((x) => !(x.videoId === msg.videoId && x.itag === msg.itag));
        clean.push({
          videoId: msg.videoId,
          itag: msg.itag,
          filename: msg.filename,
          // A watched picture is still one file: the sound it will be joined
          // with is enrolled beside it.
          pairItag: msg.pairItag,
          pairFilename: msg.pairFilename,
          referer: msg.referer,
          tabId: sender.tab && sender.tab.id,
          addedAt: Date.now(),
        });
        await setJobs(clean);
        try {
          await api.alarms.create(RETRY_ALARM, { periodInMinutes: RETRY_PERIOD_MIN });
        } catch {}
        runRetry().catch(() => {}); // one immediate check: the window may be open now.
        sendResponse({ ok: true, watching: clean.length });
        break;
      }
      case 'retry-cancel': {
        const jobs = await getJobs();
        const keep = jobs.filter((x) => !(x.videoId === msg.videoId && x.itag === msg.itag));
        await setJobs(keep);
        if (!keep.length) {
          try { await api.alarms.clear(RETRY_ALARM); } catch {}
        }
        sendResponse({ ok: true, watching: keep.length });
        break;
      }
      case 'retry-status': {
        const jobs = await getJobs();
        sendResponse({ ok: true, itags: jobs.filter((x) => x.videoId === msg.videoId).map((x) => x.itag) });
        break;
      }
      case 'settings': {
        const next = {
          enabled: msg.enabled !== false,
          mode: ['take', 'adopt', 'import'].includes(msg.mode) ? msg.mode : settings.mode,
        };
        await api.storage.local.set(next);
        settings = next;
        sendResponse({ ok: true, settings });
        break;
      }
      case 'open': {
        const r = await nativeCall('hello');
        sendResponse(r);
        break;
      }
      case 'register': {
        const r = await nativeCall('register');
        sendResponse(r);
        break;
      }
      default:
        sendResponse({ ok: false, code: 'unknown' });
    }
  })();
  return true;
});

api.runtime.onInstalled.addListener(() => {
  loadSettings().then(sweep).then(armRetry);
});
loadSettings().then(sweep).then(armRetry);
