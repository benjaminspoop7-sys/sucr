const params = new URLSearchParams(location.search);
const forced = params.get('state'); // preview.html pins a state; the real popup passes none
const theme = params.get('theme') || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');

document.documentElement.classList.toggle('sucr-theme-dark', theme === 'dark');
document.documentElement.classList.toggle('sucr-theme-light', theme !== 'dark');
document.body.style.width = '360px';

const $ = (id) => document.getElementById(id);
const chip = $('chip');
const footNote = $('footNote');

function setState(state) {
  for (const el of document.querySelectorAll('[data-when]')) {
    el.hidden = !el.dataset.when.split(' ').includes(state);
  }
  const labels = { connected: 'Connected', waking: 'Starting Sucr', unregistered: 'Not registered', empty: 'Connected' };
  chip.textContent = labels[state] || labels.connected;
  chip.dataset.tone = state === 'connected' || state === 'empty' ? 'ok' : 'warn';
}

/* ---- boot ---------------------------------------------------------------- */
// The popup is settings only. Capture lives over the playing video:
// the in-page grabber and its stream list, not a toolbar file list.

(async function boot() {
  if (forced) {
    setState(forced);
    footNote.textContent = { connected: 'Capture lives on the page', empty: 'Play a video to capture it', waking: 'Sucr is opening', unregistered: 'Not registered here yet' }[forced] || '';
    return;
  }

  const state = await chrome.runtime.sendMessage({ op: 'state' }).catch(() => null);
  const live = state && state.door && !state.door.error;

  if (!live) {
    setState('waking');
    footNote.textContent = 'Starting Sucr';
    const woke = await chrome.runtime.sendMessage({ op: 'wake' }).catch(() => null);
    if (!woke || !woke.ok) {
      if (woke && woke.code === 'no-host') {
        setState('unregistered');
        $('registerBtn').hidden = false;
      } else {
        setState('waking');
        footNote.textContent = 'Sucr is not running';
      }
      return;
    }
  }

  // The page row is the site the capture settings apply to: the active tab,
  // or the most recent page when the popup itself holds the focus.
  const tabs = await chrome.tabs.query({ currentWindow: true });
  const pages = tabs.filter((t) => t.url && !/^(chrome-extension|moz-extension|about|chrome):/i.test(t.url));
  const tab = pages.find((t) => t.active) || pages.sort((a, b) => (b.lastAccessed || 0) - (a.lastAccessed || 0))[0] || null;
  $('host').textContent = tab && tab.url ? new URL(tab.url).host : 'this page';
  $('title').textContent = (tab && tab.title) || '';
  const siteHost = tab && tab.url ? new URL(tab.url).host : '';
  $('pageBlock').hidden = !tab;
  const siteOn = !(state && state.sites && state.sites[siteHost] === false);
  $('siteToggle').checked = siteOn;
  $('siteToggle').addEventListener('change', async () => {
    const on = $('siteToggle').checked;
    const out = await chrome.runtime.sendMessage({ op: 'site', url: tab.url, on }).catch(() => null);
    if (!out || !out.ok) {
      $('siteToggle').checked = !on;
      footNote.textContent = 'Could not save that choice';
      return;
    }
    footNote.textContent = on ? `Capturing on ${siteHost}` : `Standing down on ${siteHost}`;
  });
  $('openBtn').hidden = true; // the app answered, so there is nothing to open
  setState('connected');
  footNote.textContent = siteOn ? 'Play a video — the download button appears on it' : 'Sucr is standing down on this site';
})();

$('registerBtn').addEventListener('click', async () => {
  const out = await chrome.runtime.sendMessage({ op: 'register' }).catch(() => null);
  footNote.textContent = out && out.ok ? 'Registered. Close and reopen the browser.' : 'Registration failed';
});

$('openBtn').addEventListener('click', async () => {
  footNote.textContent = 'Opening Sucr';
  const out = await chrome.runtime.sendMessage({ op: 'open' }).catch(() => null);
  footNote.textContent = out && out.ok ? 'Sucr is up' : 'Sucr did not open';
});

$('gearBtn').addEventListener('click', async () => {
  const row = $('modeRow');
  row.hidden = !row.hidden;
  if (row.hidden) return;
  const state = await chrome.runtime.sendMessage({ op: 'state' }).catch(() => null);
  const mode = (state && state.settings && state.settings.mode) || 'take';
  for (const b of row.querySelectorAll('[data-mode]')) b.setAttribute('aria-pressed', String(b.dataset.mode === mode));
});

for (const b of document.querySelectorAll('[data-mode]')) {
  b.addEventListener('click', async () => {
    const out = await chrome.runtime.sendMessage({ op: 'settings', mode: b.dataset.mode }).catch(() => null);
    for (const other of document.querySelectorAll('[data-mode]')) other.setAttribute('aria-pressed', String(other === b));
    if (out && out.ok) footNote.textContent = `New downloads: ${b.textContent.toLowerCase()}`;
  });
}