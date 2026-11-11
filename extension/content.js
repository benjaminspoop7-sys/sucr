// The in-page affordances: a chip over any downloadable link or media, a
// grabber over playing video, and the file info panel that answers a capture.
// Alt is the prevent key: held Alt means the browser keeps the file. Nothing is read from the page; each affordance carries one URL and
// one referer, both of which the page already knows.
(() => {
  if (window.__sucrChip) return;
  window.__sucrChip = true;

  const KIND_LINK = 'link';
  const KIND_MEDIA = 'media';
  const DWELL_MEDIA = 260;
  const DWELL_LINK = 0;

  const api = typeof browser !== 'undefined' ? browser : chrome;

  // Manifests the background sniffed for this tab (HLS/DASH), href -> true.
  // The grabber lists these; the toast names the file so a page without an
  // open grabber still says what was found. One toast per distinct manifest:
  // a live playlist re-polls the same URL every few seconds, and re-saying
  // it would keep the toast alive forever. The background pushes what it
  // sees after injection; a page whose own document is the manifest is
  // served before this script exists, so it also pulls the tab's ledger.
  const manifests = new Map();
  function rememberManifest(raw) {
    let u;
    try {
      u = new URL(raw);
    } catch {
      return null;
    }
    if (!/\.(m3u8|mpd)$/i.test(u.pathname) || manifests.has(u.href)) return null;
    manifests.set(u.href, true);
    return u;
  }
  const toastManifest = (u) =>
    say(`Sucr detected manifest: ${u.pathname.split('/').filter(Boolean).pop() || u.host}`, null, false);
  api.runtime.onMessage.addListener((msg) => {
    if (!msg || msg.op !== 'manifest-detected') return;
    const u = rememberManifest(msg.url);
    if (u) toastManifest(u);
  });
  api.runtime.sendMessage({ op: 'manifest-list' })
    .then((r) => {
      if (!r || !Array.isArray(r.urls)) return;
      let last = null;
      for (const raw of r.urls) {
        const u = rememberManifest(raw);
        if (u) last = u;
      }
      if (last) toastManifest(last);
    })
    .catch(() => {});

  const host = document.createElement('div');
  host.style.cssText = 'position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none';
  const shadow = host.attachShadow({ mode: 'closed' });

  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    .chip, .toast {
      position: fixed;
      display: none;
      align-items: center;
      gap: 6px;
      height: 26px;
      padding: 0 9px;
      border: 1px solid rgba(0, 0, 0, 0.18);
      border-radius: 6px;
      font: 600 12px/1 "Segoe UI", system-ui, -apple-system, sans-serif;
      white-space: nowrap;
      box-shadow: 0 2px 6px rgba(0, 0, 0, 0.28);
      opacity: 0;
      transition: opacity 120ms cubic-bezier(0.2, 0, 0, 1), background 80ms linear;
    }
    .chip {
      background: #1A73E8;
      color: #FFFFFF;
      cursor: pointer;
      user-select: none;
      transform: translateY(-2px);
      transition: opacity 120ms cubic-bezier(0.2, 0, 0, 1),
                  transform 120ms cubic-bezier(0.2, 0, 0, 1),
                  background 80ms linear;
      pointer-events: auto;
    }
    .toast {
      background: #202124;
      color: #FFFFFF;
      font-weight: 500;
      max-width: 280px;
      overflow: hidden;
      text-overflow: ellipsis;
      pointer-events: none;
    }
    [data-shown] { display: inline-flex; }
    [data-in] { opacity: 1; }
    .chip[data-in] { transform: translateY(0); }
    .toast[data-bad] { background: #A31515; }
    .chip:hover { background: #1765CC; }
    .chip:active { background: #1357B4; }
    .chip svg { width: 12px; height: 12px; flex: none; }
    .vbtn {
      position: fixed;
      display: none;
      align-items: center;
      gap: 6px;
      height: 28px;
      padding: 0 10px;
      border: 1px solid rgba(0, 0, 0, 0.18);
      border-radius: 6px;
      background: #1A73E8;
      color: #FFFFFF;
      font: 600 12px/1 "Segoe UI", system-ui, -apple-system, sans-serif;
      white-space: nowrap;
      cursor: pointer;
      user-select: none;
      box-shadow: 0 2px 6px rgba(0, 0, 0, 0.28);
      pointer-events: auto;
    }
    .vbtn[data-shown] { display: inline-flex; }
    .vbtn:hover { background: #1765CC; }
    .vbtn svg { width: 12px; height: 12px; flex: none; }
    .vlist {
      position: fixed;
      display: none;
      flex-direction: column;
      width: 340px;
      max-height: min(72vh, 560px);
      border: 1px solid rgba(0, 0, 0, 0.18);
      border-radius: 8px;
      background: #FFFFFF;
      color: #202124;
      font: 400 12px/1.4 "Segoe UI", system-ui, -apple-system, sans-serif;
      box-shadow: 0 4px 14px rgba(0, 0, 0, 0.28);
      pointer-events: auto;
      overflow: hidden;
    }
    .vlist[data-shown] { display: flex; }
    .vlist[data-dark] { background: #202124; color: #E8EAED; border-color: rgba(255, 255, 255, 0.18); }
    .v-head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      padding: 8px 10px;
      border-bottom: 1px solid rgba(0, 0, 0, 0.1);
      font-weight: 600;
    }
    .vlist[data-dark] .v-head { border-bottom-color: rgba(255, 255, 255, 0.12); }
    .v-x {
      width: 20px;
      height: 20px;
      padding: 0;
      border: none;
      border-radius: 4px;
      background: transparent;
      color: inherit;
      font: 600 13px/1 "Segoe UI", system-ui, sans-serif;
      cursor: pointer;
    }
    .v-x:hover { background: rgba(0, 0, 0, 0.08); }
    .vlist[data-dark] .v-x:hover { background: rgba(255, 255, 255, 0.12); }
    .v-rows { overflow-y: auto; }
    .v-row {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 10px;
    }
    .v-row + .v-row { border-top: 1px solid rgba(0, 0, 0, 0.06); }
    .vlist[data-dark] .v-row + .v-row { border-top-color: rgba(255, 255, 255, 0.08); }
    .v-text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
    .v-name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .v-meta { color: #5F6368; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .vlist[data-dark] .v-meta { color: #9AA0A6; }
    .v-take {
      flex: none;
      height: 24px;
      padding: 0 10px;
      border: none;
      border-radius: 6px;
      background: #1A73E8;
      color: #FFFFFF;
      font: 600 12px/1 "Segoe UI", system-ui, sans-serif;
      cursor: pointer;
    }
    .v-take:hover { background: #1765CC; }
    .v-take:disabled { opacity: 0.6; cursor: default; }
    .v-watch { background: #2f7d4f; }
    .v-watch:hover { background: #266a44; }
    .v-watch:disabled { opacity: 0.8; }
    .v-x:focus-visible, .v-take:focus-visible { outline: 2px solid #1A73E8; outline-offset: 1px; }
    .panel {
      position: fixed;
      right: 16px;
      bottom: 16px;
      display: none;
      flex-direction: column;
      gap: 8px;
      width: 300px;
      padding: 12px;
      border: 1px solid rgba(0, 0, 0, 0.18);
      border-radius: 8px;
      background: #FFFFFF;
      color: #202124;
      font: 400 12px/1.4 "Segoe UI", system-ui, -apple-system, sans-serif;
      box-shadow: 0 4px 14px rgba(0, 0, 0, 0.28);
      pointer-events: auto;
    }
    .panel[data-shown] { display: flex; }
    .panel[data-dark] { background: #202124; color: #E8EAED; border-color: rgba(255, 255, 255, 0.18); }
    .p-name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .p-meta { color: #5F6368; }
    .panel[data-dark] .p-meta { color: #9AA0A6; }
    .p-track { height: 4px; border-radius: 2px; background: rgba(0, 0, 0, 0.12); overflow: hidden; }
    .panel[data-dark] .p-track { background: rgba(255, 255, 255, 0.16); }
    .p-bar { height: 100%; width: 0; background: #1A73E8; transition: width 240ms linear; }
    .p-bar[data-ind] { width: 30%; animation: p-slide 1.1s ease-in-out infinite alternate; }
    @keyframes p-slide { from { margin-left: 0; } to { margin-left: 70%; } }
    .p-actions { display: flex; gap: 8px; justify-content: flex-end; }
    .p-btn {
      height: 26px;
      padding: 0 10px;
      border: 1px solid rgba(0, 0, 0, 0.18);
      border-radius: 6px;
      background: transparent;
      color: inherit;
      font: 600 12px/1 "Segoe UI", system-ui, -apple-system, sans-serif;
      cursor: pointer;
    }
    .p-btn:hover { background: rgba(0, 0, 0, 0.06); }
    .panel[data-dark] .p-btn:hover { background: rgba(255, 255, 255, 0.1); }
    .p-btn[data-primary] { background: #1A73E8; border-color: #1A73E8; color: #FFFFFF; }
    .p-btn[data-primary]:hover { background: #1765CC; }
    .panel[data-bad] .p-meta { color: #A31515; }
    .panel[data-dark][data-bad] .p-meta { color: #F28B82; }
    .panel[data-bad] .p-bar { background: #A31515; }
    @media (prefers-reduced-motion: reduce) {
      .chip, .toast { transition: none; }
      .chip { transform: none; }
      .p-bar { transition: none; }
      .p-bar[data-ind] { animation: none; width: 100%; }
    }
  `);
  shadow.adoptedStyleSheets = [sheet];

  const DOWN = '<svg viewBox="0 0 12 12" fill="none" aria-hidden="true"><path d="M6 1v7M3 5.5 6 8.5l3-3M2 10.5h8" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';

  const chip = document.createElement('div');
  chip.className = 'chip';
  chip.setAttribute('role', 'button');
  chip.innerHTML = DOWN + '<span></span>';
  const chipText = chip.lastElementChild;

  const toast = document.createElement('div');
  toast.className = 'toast';
  toast.setAttribute('role', 'status');

  // The grabber over playing media: a video panel drawn in-page.
  const vbtn = document.createElement('div');
  vbtn.className = 'vbtn';
  vbtn.setAttribute('role', 'button');
  vbtn.innerHTML = DOWN + '<span>Download this video</span>';

  // The stream list the grabber opens: video/audio candidates only, anchored
  // to the player instead of living in the toolbar popup.
  const vlist = document.createElement('div');
  vlist.className = 'vlist';
  if (matchMedia('(prefers-color-scheme: dark)').matches) vlist.setAttribute('data-dark', '');
  const vHead = document.createElement('div');
  vHead.className = 'v-head';
  const vTitle = document.createElement('span');
  vTitle.textContent = 'Download this video';
  const vX = document.createElement('button');
  vX.className = 'v-x';
  vX.type = 'button';
  vX.setAttribute('aria-label', 'Close');
  vX.textContent = '\u00d7';
  vHead.append(vTitle, vX);
  const vRows = document.createElement('div');
  vRows.className = 'v-rows';
  vlist.append(vHead, vRows);

  // The file info panel: the verdict and live progress of one capture.
  const panel = document.createElement('div');
  panel.className = 'panel';
  panel.setAttribute('role', 'status');
  if (matchMedia('(prefers-color-scheme: dark)').matches) panel.setAttribute('data-dark', '');
  const pName = document.createElement('div');
  pName.className = 'p-name';
  const pMeta = document.createElement('div');
  pMeta.className = 'p-meta';
  const pTrack = document.createElement('div');
  pTrack.className = 'p-track';
  const pBar = document.createElement('div');
  pBar.className = 'p-bar';
  pTrack.appendChild(pBar);
  const pActions = document.createElement('div');
  pActions.className = 'p-actions';
  const pOpen = document.createElement('button');
  pOpen.className = 'p-btn';
  pOpen.type = 'button';
  pOpen.setAttribute('data-primary', '');
  pOpen.textContent = 'Open Sucr';
  const pClose = document.createElement('button');
  pClose.className = 'p-btn';
  pClose.type = 'button';
  pClose.textContent = 'Dismiss';
  pActions.append(pOpen, pClose);
  panel.append(pName, pMeta, pTrack, pActions);

  shadow.append(chip, toast, vbtn, vlist, panel);

  let target = null;
  let kind = null;
  let url = null;
  let dwell = 0;
  let hideTimer = 0;
  let sayTimer = 0;
  let sayAnchor = null;
  let enabled = true;
  let lastX = 0;
  let lastY = 0;
  let vEl = null;
  let panelTimer = 0;

  function applyStored(v) {
    const sites = v && v.sites && typeof v.sites === 'object' ? v.sites : {};
    enabled = (!v || v.enabled !== false) && sites[location.host] !== false;
    if (!enabled) { reset(); hideVideo(); hidePanel(false); }
  }
  api.storage.local.get(['enabled', 'sites']).then(applyStored, () => {});
  api.storage.onChanged.addListener((changes, area) => {
    if (area !== 'local') return;
    if (changes.enabled || changes.sites) {
      api.storage.local.get(['enabled', 'sites']).then(applyStored, () => {});
    }
  });

  function absolute(raw) {
    try {
      const u = new URL(raw, location.href);
      return /^https?:$/.test(u.protocol) ? u.href : null;
    } catch {
      return null;
    }
  }

  function mediaUrl(el) {
    const src = el.currentSrc || el.src;
    return src ? absolute(src) : null;
  }

  // The hovered thing, or null. A link wins over the picture inside it: the
  // page made that picture the link's affordance.
  function find(at) {
    if (!at || at.nodeType !== 1 || at === host) return null;
    const link = at.closest && at.closest('a[href]');
    if (link) {
      const href = absolute(link.getAttribute('href'));
      return href ? { el: link, kind: KIND_LINK, url: href } : null;
    }
    const media = at.closest && at.closest('img, video, audio');
    if (!media) return null;
    const r = media.getBoundingClientRect();
    // Tracking pixels and icons are not files anyone wants; 64x40 is the
    // smallest thing that reads as a picture on screen.
    if (r.width < 64 || r.height < 40) return null;
    const src = mediaUrl(media);
    return src ? { el: media, kind: KIND_MEDIA, url: src } : null;
  }

  function place(rect, node) {
    if (!rect.width && !rect.height) return false;
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    if (rect.bottom < 0 || rect.top > vh) return false;
    node.setAttribute('data-shown', '');
    const w = node.offsetWidth;
    const h = node.offsetHeight;
    node.style.left = Math.min(Math.max(8, rect.left), Math.max(8, vw - w - 8)) + 'px';
    const below = rect.bottom + 6;
    node.style.top = (below + h > vh - 8 ? Math.max(8, rect.top - h - 6) : below) + 'px';
    return true;
  }

  function showChip() {
    if (!enabled || !target || !url) return;
    // On a page with a real video player, the grabber is the only affordance
    // the user asked for and it appears only once a video plays. The generic
    // chip must never compete with it or pop before the video loads.
    if (hasPlayer()) return;
    chipText.textContent = kind === KIND_MEDIA ? 'Download media with Sucr' : 'Download with Sucr';
    if (!place(target.getBoundingClientRect(), chip)) return;
    requestAnimationFrame(() => chip.setAttribute('data-in', ''));
  }

  function hideChip() {
    chip.removeAttribute('data-in');
    chip.removeAttribute('data-shown');
  }

  function reset() {
    clearTimeout(dwell);
    clearTimeout(hideTimer);
    target = null;
    kind = null;
    url = null;
    hideChip();
  }

  function enter(hit) {
    clearTimeout(dwell);
    clearTimeout(hideTimer);
    if (!hit) {
      // Leaving the item is not an immediate dismissal: the pointer travels
      // from the link to the chip that sits just under it.
      hideTimer = setTimeout(() => { target = null; hideChip(); }, 140);
      return;
    }
    const same = target === hit.el && url === hit.url;
    target = hit.el;
    kind = hit.kind;
    url = hit.url;
    if (same && chip.hasAttribute('data-shown')) return;
    hideChip();
    dwell = setTimeout(showChip, kind === KIND_LINK ? DWELL_LINK : DWELL_MEDIA);
  }

  function hoveringChip(node) {
    // Events inside a shadow root are retargeted to its host, so the host is
    // the identity the page-level listeners actually see.
    return !!node && (node === host || node === chip || chip.contains(node));
  }

  document.addEventListener('pointerover', (e) => {
    if (!enabled || hoveringChip(e.target)) return;
    enter(find(e.target));
  }, true);

  document.addEventListener('pointerout', (e) => {
    if (!enabled || hoveringChip(e.target) || hoveringChip(e.relatedTarget)) {
      clearTimeout(hideTimer);
      return;
    }
    enter(find(e.relatedTarget));
  }, true);

  addEventListener('pointermove', (e) => { lastX = e.clientX; lastY = e.clientY; }, { capture: true, passive: true });
  addEventListener('scroll', () => { reset(); placeVideo(); }, { capture: true, passive: true });
  addEventListener('blur', () => {
    api.runtime.sendMessage({ op: 'keys', alt: false }).catch(() => {});
    reset();
    // Losing focus is not the video ending: hide the button but keep tracking
    // the element, so refocusing the tab brings the grabber back.
    vbtn.removeAttribute('data-shown');
    hideVideoList();
    vRect = null;
  });
  addEventListener('focus', () => { placeVideo(); });

  // Alt is the prevent key: the background keeps the live key state
  // and a held Alt leaves the download to the browser.
  addEventListener('keydown', (e) => {
    if (e.key === 'Alt' && !e.repeat) api.runtime.sendMessage({ op: 'keys', alt: true }).catch(() => {});
  }, true);
  addEventListener('keyup', (e) => {
    if (e.key === 'Alt') api.runtime.sendMessage({ op: 'keys', alt: false }).catch(() => {});
  }, true);

  // Attribute the click that caused a download, so the verdict and the file
  // info panel land in the tab that asked. The click itself is never touched:
  // a normal click downloads normally, and the background decides who takes it.
  addEventListener('click', (e) => {
    if (!enabled) return;
    const link = e.target && e.target.closest && e.target.closest('a[href]');
    if (!link) return;
    const href = absolute(link.getAttribute('href'));
    if (href) api.runtime.sendMessage({ op: 'page-click', url: href, alt: !!e.altKey }).catch(() => {});
  }, true);

  chip.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (url) take(url, target);
  });
  chip.addEventListener('pointerover', () => clearTimeout(hideTimer));
  chip.addEventListener('pointerout', () => {
    hideTimer = setTimeout(reset, 140);
  });

  async function take(u, el, fname) {
    if (!enabled || !u) return;
    const anchor = el || target;
    sayAnchor = anchor;
    hideChip();
    let out = null;
    try {
      out = await api.runtime.sendMessage({ op: 'take', url: u, filename: fname || nameOf(u), referer: location.href, ua: navigator.userAgent });
    } catch {
      out = null;
    }
    const ok = !!(out && out.ok);
    const code = out && out.body && out.body.code;
    // A yes opens the file info panel in this tab; only a refusal needs a toast.
    if (!ok) {
      say(
        code === 'not-running' ? 'Sucr is not running'
          : code === 'site-off' ? 'Sucr is standing down on this site'
          : 'Sucr did not take it',
        anchor,
        true,
      );
    }
  }

  // A high-res YouTube stream is DASH: the picture and the sound arrive as two
  // separate adaptive rows. Sucr's muxers join a picture with a sound into one
  // playable file, so a video-only row is paired with the best audio row of its
  // own container — MP4 (H.264/AV1) with an m4a sound, WebM (VP9) with a WebM
  // sound — because a container copy cannot cross families. `cands` is already
  // bitrate-sorted, so the first match is the best one. A row that already
  // carries its audio is taken alone.
  function pairAudio(c, cands) {
    if (!c.isVideo || c.muxed) return null;
    const family = c.ext === 'mp4' ? ['mp4', 'm4a'] : c.ext === 'webm' ? ['webm'] : null;
    if (!family) return null;
    return cands.find((x) => !x.isVideo && family.includes(x.ext)) || null;
  }

  // One Take is one download: the sound's link rides along with the picture's
  // and Sucr fetches both under the row it opens, so the library never shows
  // the two streams as separate entries. A ladder row is named
  // `<title>-<tier>-<codec>-<itag>.<ext>`; the joined file keeps the title.
  function mergedName(video) {
    return video.filename.replace(/-[^-]+-[a-z0-9]+-\d+(?=\.[^.]+$)/i, '');
  }

  async function takeMerged(video, audio, anchor) {
    if (!enabled) return;
    sayAnchor = anchor;
    let out = null;
    try {
      out = await api.runtime.sendMessage({
        op: 'take',
        url: video.url,
        filename: mergedName(video),
        audioUrl: audio.url,
        referer: location.href,
        ua: navigator.userAgent,
      });
    } catch {
      out = null;
    }
    // A yes opens the file info panel on the row that is doing both streams;
    // only a refusal that never reached the panel needs a toast.
    if (!(out && out.ok)) {
      const code = out && out.body && out.body.code;
      say(
        code === 'not-running' ? 'Sucr is not running'
          : code === 'site-off' ? 'Sucr is standing down on this site'
          : 'Sucr did not take it',
        anchor,
        true,
      );
    }
  }

  function nameOf(u) {
    try {
      const path = new URL(u).pathname;
      const last = path.split('/').filter(Boolean).pop() || '';
      return decodeURIComponent(last);
    } catch {
      return '';
    }
  }

  function say(text, el, bad) {
    clearTimeout(sayTimer);
    toast.textContent = text;
    toast.toggleAttribute('data-bad', !!bad);
    const live = el && el.isConnected ? el : sayAnchor && sayAnchor.isConnected ? sayAnchor : null;
    const rect = live ? live.getBoundingClientRect() : null;
    if (!rect || !place(rect, toast)) {
      // A verdict that arrives after the pointer left still gets a place:
      // the foot of the viewport, where nothing else of ours lives.
      const vh = document.documentElement.clientHeight;
      place({ width: 1, height: 1, left: document.documentElement.clientWidth / 2, top: vh - 72, bottom: vh - 72 }, toast);
    }
    requestAnimationFrame(() => toast.setAttribute('data-in', ''));
    sayTimer = setTimeout(() => {
      toast.removeAttribute('data-in');
      setTimeout(() => toast.removeAttribute('data-shown'), 140);
    }, 1800);
  }

  // --- the grabber over playing media ---------------------------------------
  // The download affordance floats over the playing video, not in the
  // toolbar. The button anchors to the player; the stream list it opens
  // carries video/audio candidates only.

  // Media-only evidence: a file extension that reads as video/audio on the
  // URL's path (never in a query value), or a videoplayback stream carrying
  // its mime in the query. Fetches, scripts and tracking pixels never qualify.
  const MEDIA_EXT = /\.(mp4|m4v|webm|mkv|mov|avi|mpg|mpeg|flv|wmv|m4a|mp3|opus|ogg|oga|flac|wav|aac)$/i;

  function streamMime(u) {
    try {
      const url = new URL(u);
      if (!/\/videoplayback$/.test(url.pathname)) return null;
      const mime = url.searchParams.get('mime') || '';
      return /^(video|audio)\//.test(mime) ? mime : null;
    } catch {
      return null;
    }
  }

  function mediaNetUrl(u) {
    if (!u || !/^https?:/i.test(u)) return false;
    if (streamMime(u)) return true;
    try {
      return MEDIA_EXT.test(new URL(u).pathname);
    } catch {
      return false;
    }
  }

  // Blob players (YouTube's MSE) expose no http src on the element; the
  // network log is the evidence. Watch it from injection, and read whatever
  // the resource buffer already holds when the grabber is asked.
  const netMedia = new Map();
  function noteMedia(raw) {
    if (!mediaNetUrl(raw) || netMedia.size >= 60) return;
    let u;
    let key;
    try {
      u = new URL(raw);
      if (streamMime(raw)) {
        // Segments of one stream differ only by range/cpn; keep one URL per
        // itag+mime and let the engine make its own range requests.
        key = 'vp:' + (u.searchParams.get('itag') || '') + ':' + u.searchParams.get('mime');
        u.searchParams.delete('range');
        u.searchParams.delete('cpn');
      } else {
        key = u.href.split('#')[0];
      }
    } catch {
      return;
    }
    if (!netMedia.has(key)) netMedia.set(key, u.href);
  }
  try {
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) noteMedia(e.name);
    }).observe({ type: 'resource', buffered: true });
  } catch { /* resource timing unavailable: element srcs still work */ }

  function extOfMime(mime) {
    if (mime === 'video/webm' || mime === 'audio/webm') return 'webm';
    if (mime === 'audio/mp4') return 'm4a';
    if (mime === 'audio/mpeg') return 'mp3';
    return 'mp4';
  }

  function cleanName(s) {
    return String(s).replace(/[\\/:*?"<>|]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 120);
  }

  function videoCandidates(el) {
    for (const e of performance.getEntriesByType('resource')) noteMedia(e.name);
    const out = [];
    const seen = new Set();
    const push = (c) => {
      if (!seen.has(c.url)) { seen.add(c.url); out.push(c); }
    };
    const addSrc = (raw, type) => {
      const u = absolute(raw);
      if (!u || !mediaNetUrl(u)) return;
      push({ url: u, filename: nameOf(u), label: type || 'Media file', meta: new URL(u).host });
    };
    // Manifests sniffed for this tab. Taking one hands the manifest URL to
    // Sucr as-is: until the engine parses manifests (Task 292) that saves
    // the playlist file, not a playable video — the label says manifest.
    const mTitle = cleanName(document.title) || 'video';
    for (const u of manifests.keys()) {
      const isHls = /\.m3u8$/i.test(new URL(u).pathname);
      push({
        url: u,
        filename: `${mTitle}.${isHls ? 'm3u8' : 'mpd'}`,
        label: isHls ? 'HLS Manifest · m3u8' : 'DASH Manifest · mpd',
        meta: new URL(u).host,
        isManifest: true,
      });
    }
    if (el) {
      addSrc(el.currentSrc || el.src, el.type || '');
      for (const s of el.querySelectorAll('source')) addSrc(s.src, s.type);
    }
    const title = cleanName(document.title) || 'video';
    for (const u of netMedia.values()) {
      const mime = streamMime(u);
      if (mime) {
        const url = new URL(u);
        const ext = extOfMime(mime);
        const itag = url.searchParams.get('itag');
        const clen = Number(url.searchParams.get('clen'));
        push({
          url: u,
          filename: `${title}.${ext}`,
          label: `${mime.startsWith('video') ? 'Video' : 'Audio'} · ${ext.toUpperCase()}${itag ? ` · ${itag}` : ''}`,
          meta: clen > 0 ? bytes(clen) : url.host,
        });
      } else {
        push({ url: u, filename: nameOf(u), label: 'Media file', meta: new URL(u).host });
      }
    }
    return out.slice(0, 12);
  }

  // YouTube's web player streams over SABR: the videoplayback URLs on the
  // wire carry no per-format query, and the watch page's own player response
  // holds no format URLs. So the ladder is read from the page's own desktop
  // innertube context (the same request the player makes, same origin, same
  // cookies) — it answers status OK with every format the desktop player
  // offers, but only the progressive muxed row carries a `url`; adaptive rows
  // are server-side streaming ids. A second call in the visionOS client fills
  // in the direct URL for each adaptive itag, and it must carry the page's
  // visitorData or Google answers LOGIN_REQUIRED. Both run in the background
  // worker's page-world injection: this script lives in an isolated world and
  // cannot read the page's ytcfg for itself.
  async function playerLadders(id) {
    try {
      return await api.runtime.sendMessage({ op: 'yt-ladders', id });
    } catch {
      return null;
    }
  }

  async function youtubeCandidates() {
    const id = new URL(location.href).searchParams.get('v');
    if (!id) return null;
    const ladders = await playerLadders(id);
    if (!ladders) return null;
    const web = ladders.web || ladders.vision;
    if (!web) return null;
    const title = cleanName(web.title || document.title) || 'video';
    const progressive = new Set((web.formats || []).map((f) => f.itag));
    const direct = new Map();
    const vs = ladders.vision || {};
    for (const f of [...(vs.formats || []), ...(vs.adaptive || [])]) if (f.url) direct.set(f.itag, f.url);
    const all = [...(web.formats || []), ...(web.adaptive || [])]
      .map((f) => ({ ...f, url: f.url || direct.get(f.itag) }))
      .filter((f) => f.url && f.mimeType);
    const seenItag = new Set();
    const uniq = all.filter((f) => (seenItag.has(f.itag) ? false : (seenItag.add(f.itag), true)));
    const isVideo = (f) => /^video\//.test(f.mimeType);
    // Every format the player will hand over, one row per itag: the ladder is
    // the gift, not a shortlist. Collapsing by quality label hid whole codecs —
    // at a given height the player offers H.264, VP9 and AV1 at different
    // sizes, and a muxed row must not swallow the same tier's video-only rows.
    const codecOf = (f) => {
      const c = ((f.mimeType.match(/codecs="([^"]+)"/) || [])[1] || '').split(',')[0].trim();
      if (/^avc1/.test(c)) return 'H.264';
      if (/^vp0?9/.test(c)) return 'VP9';
      if (/^av01/.test(c)) return 'AV1';
      if (/^mp4a\.40\.5/.test(c)) return 'HE-AAC';
      if (/^mp4a/.test(c)) return 'AAC';
      if (/^opus/.test(c)) return 'Opus';
      if (/^ec-3/.test(c)) return 'E-AC-3';
      if (/^ac-3/.test(c)) return 'AC-3';
      return c || '?';
    };
    const slug = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, '');
    const videos = uniq.filter(isVideo).sort((a, b) => (b.height || 0) - (a.height || 0) || (b.fps || 0) - (a.fps || 0) || (b.bitrate || 0) - (a.bitrate || 0));
    const audios = uniq.filter((f) => !isVideo(f)).sort((a, b) => (b.bitrate || 0) - (a.bitrate || 0));
    const rows = [...videos, ...audios].map((f) => {
      const mime = f.mimeType.split(';')[0].trim();
      const ext = extOfMime(mime);
      const clen = Number(f.contentLength) || 0;
      const vid = isVideo(f);
      const muxed = progressive.has(f.itag);
      const codec = codecOf(f);
      // qualityLabel already carries the frame rate ("2160p60"); only a row
      // without one needs the fps appended by hand.
      const fps = Number(f.fps) || 0;
      const tier = vid
        ? f.qualityLabel || (f.height ? f.height + 'p' + (fps > 30 ? fps : '') : f.quality || '')
        : '';
      const kbps = f.bitrate ? Math.round(f.bitrate / 1000) + ' kbps' : (f.quality || '');
      const label = vid
        ? `Video · ${ext.toUpperCase()} · ${tier} · ${codec}${muxed ? ' · muxed' : ''}`
        : `Audio · ${ext.toUpperCase()} · ${codec} · ${kbps}`;
      // One distinct name per itag, or two codecs at the same height collide on
      // disk and the second job appears to overwrite the first.
      const stem = vid ? `${title}-${tier}-${slug(codec)}-${f.itag}` : `${title}-audio-${slug(codec)}-${f.itag}`;
      // The label already says Video/Audio and marks a muxed row, so the meta
      // carries only what the label cannot: size, itag, and (below) protection.
      // A 340px panel leaves 206px for this line; anything longer clips the one
      // warning that matters.
      const meta = clen > 0 ? `${bytes(clen)} · itag ${f.itag}` : `itag ${f.itag}`;
      return { url: f.url, filename: `${stem}.${ext}`, label, meta, clen, muxed, ext, itag: f.itag, height: f.height || 0, isVideo: vid };
    });
    // Show the whole ladder, but mark the formats Google protects. Google
    // guards the adaptive tiers per video with a Proof-of-Origin token: on a
    // gated id the URL answers 206 only for the first ~12-16 MiB "free window"
    // and 403s every range past it, while the progressive muxed row serves the
    // whole file. Sucr's engine fetches in 8 MiB blocks with no cookies and no
    // BotGuard attestation, so a protected row reads the free chunk, the next
    // block 403s, and the row lands at ~2% as `block-failed`. The user asked to
    // see every format up to the video's top quality, so the list is NOT pruned
    // — instead each row is probed (offset 0 and, past the free window, near
    // the end) and a row that returns a definite non-206 is labelled protected,
    // so its cost is visible before Take rather than a silent failure. A 0
    // (googlevideo unreachable from the worker) is inconclusive and is not
    // treated as protection.
    const wall = 12582912; // ~12 MiB: past it, a protected adaptive URL 403s.
    const probes = [];
    for (const row of rows) {
      row.probeIdx = probes.length;
      probes.push({ url: row.url, offset: 0 });
      if (row.clen > wall) probes.push({ url: row.url, offset: row.clen - 65536 });
    }
    let codes = null;
    try {
      const out = await api.runtime.sendMessage({ op: 'probe', probes, ua: navigator.userAgent });
      if (out && out.ok && Array.isArray(out.codes)) codes = out.codes;
    } catch {}
    for (const row of rows) row.protected = false;
    if (codes) {
      for (const row of rows) {
        const n = row.clen > wall ? 2 : 1;
        let prot = false;
        for (let i = 0; i < n; i++) {
          const code = codes[row.probeIdx + i];
          if (code && code !== 206) prot = true;
        }
        row.protected = prot;
        if (prot) row.meta = 'protected · ' + row.meta;
      }
    }
    // `muxed`, `ext` and `isVideo` stay on the row so a video-only MP4 Take can
    // pair with the best MP4 audio and hand both to the muxer.
    return rows.map(({ clen, probeIdx, ...c }) => c);
  }

  let vRect = null;

  // Protected formats the background is re-probing to catch an open window.
  // Keyed `${videoId}:${itag}`, seeded from the worker on each list open so a
  // reopened page still shows which rows are already being watched.
  const watching = new Set();

  function hideVideoList() {
    vlist.removeAttribute('data-shown');
    vRows.textContent = '';
  }

  function hideVideo() {
    vEl = null;
    vRect = null;
    vbtn.removeAttribute('data-shown');
    hideVideoList();
  }

  // A video player is any visible <video> big enough to be a player, not a
  // background sliver. Its presence is what retires the generic chip on a
  // video page, so the grabber stays the single affordance.
  function hasPlayer() {
    for (const v of document.querySelectorAll('video')) {
      const r = v.getBoundingClientRect();
      if (r.width >= 64 && r.height >= 40) return true;
    }
    return false;
  }

  function showVideo(el) {
    if (!enabled || !el || !(el instanceof HTMLMediaElement)) return;
    vEl = el;
    // Do not reveal the grabber until the video has actually started rendering.
    // A `playing` event can fire during the first buffering stall with the
    // clock still at zero, which read as the button appearing before the video
    // loaded. A later timeupdate reveals it once frames are on screen.
    if (!vbtn.hasAttribute('data-shown') &&
        !(el.readyState >= 2 && !el.paused && el.currentTime > 0)) {
      vRect = null;
      return;
    }
    placeVideo();
  }

  // Position the grabber over the tracked video, or hide it if the video has
  // scrolled out of view. This never clears vEl: scrolling back, or refocusing
  // the tab, brings the grabber again. A detached or replaced player reads as a
  // zero rect and the button goes away on its own; only disabling the extension
  // (hideVideo) lets go of the element.
  function placeVideo() {
    if (!vEl || !enabled) return;
    const r = vEl.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    // Tiny (audio-only) players, detached elements, and a video scrolled past
    // the viewport have nowhere to anchor the button.
    if (r.width < 64 || r.height < 40 || r.bottom <= 0 || r.top >= vh) {
      vbtn.removeAttribute('data-shown');
      hideVideoList();
      vRect = null;
      return;
    }
    vRect = r;
    vbtn.setAttribute('data-shown', '');
    const w = vbtn.offsetWidth;
    vbtn.style.left = Math.min(Math.max(8, r.right - w - 8), Math.max(8, vw - w - 8)) + 'px';
    vbtn.style.top = Math.max(8, r.top + 8) + 'px';
  }

  function openVideoList(cands) {
    vRows.textContent = '';
    const videoId = location.pathname === '/watch' ? new URLSearchParams(location.search).get('v') : '';
    for (const c of cands) {
      const row = document.createElement('div');
      row.className = 'v-row';
      const text = document.createElement('div');
      text.className = 'v-text';
      const label = document.createElement('span');
      label.className = 'v-name';
      label.textContent = c.label;
      const meta = document.createElement('span');
      meta.className = 'v-meta';
      meta.textContent = c.meta;
      text.append(label, meta);
      const btn = document.createElement('button');
      btn.className = 'v-take';
      btn.type = 'button';
      btn.textContent = 'Take';
      btn.addEventListener('click', () => {
        btn.disabled = true;
        hideVideoList();
        const audio = pairAudio(c, cands);
        if (audio) takeMerged(c, audio, vEl);
        else take(c.url, vEl, c.filename);
      });
      row.append(text, btn);
      // A protected row can also be watched: the worker re-probes on a timer
      // and pulls the whole file the first tick the gate opens. Offered only
      // for a real video id with a known format; a watch already running for
      // that format just reads as Watching, never enrolled twice.
      if (c.protected && videoId && c.itag) {
        const key = `${videoId}:${c.itag}`;
        const wbtn = document.createElement('button');
        wbtn.className = 'v-take v-watch';
        wbtn.type = 'button';
        if (watching.has(key)) {
          wbtn.textContent = 'Watching';
          wbtn.disabled = true;
        } else {
          wbtn.textContent = 'Watch';
          wbtn.addEventListener('click', async () => {
            // A watched picture is enrolled with its sound, so the wait ends in
            // one playable file rather than a silent one.
            const sound = pairAudio(c, cands);
            let out = null;
            try {
              out = await api.runtime.sendMessage({
                op: 'retry',
                videoId,
                itag: c.itag,
                filename: c.filename,
                pairItag: sound ? sound.itag : undefined,
                pairFilename: sound ? sound.filename : undefined,
                referer: location.href,
              });
            } catch {}
            if (out && out.ok) {
              watching.add(key);
              wbtn.textContent = 'Watching';
              wbtn.disabled = true;
              meta.textContent = c.meta + ' · watching; Sucr pulls it whole when the gate opens';
            }
          });
        }
        row.append(wbtn);
      }
      vRows.appendChild(row);
    }
    const r = vRect || { left: 0, right: 320, top: 0, bottom: 0, width: 320, height: 0 };
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    vlist.setAttribute('data-shown', '');
    const w = vlist.offsetWidth;
    const h = vlist.offsetHeight;
    vlist.style.left = Math.min(Math.max(8, r.right - w - 8), Math.max(8, vw - w - 8)) + 'px';
    const below = r.top + 42;
    vlist.style.top = (below + h > vh - 8 ? Math.max(8, r.top - h - 8) : below) + 'px';
  }

  document.addEventListener('playing', (e) => {
    if (e.target instanceof HTMLMediaElement) showVideo(e.target);
  }, true);
  // Reveal the grabber the moment a tracked video truly starts: timeupdate
  // fires as the clock advances, so a playing event that fired during the
  // first stall (currentTime 0) still brings the button once frames render.
  // It also adopts a video that was already running before this script
  // attached (reload, SPA navigation, restore): no `playing` event will ever
  // arrive for that element, and requiring one left the grabber missing.
  document.addEventListener('timeupdate', (e) => {
    const v = e.target;
    if (!(v instanceof HTMLMediaElement) || vbtn.hasAttribute('data-shown')) return;
    if (v.readyState >= 2 && !v.paused && v.currentTime > 0) showVideo(v);
  }, true);
  // The grabber sticks once the video has genuinely started and stays while the
  // player is on screen. A real YouTube first-load fires transient `pause` /
  // `emptied` events during ad breaks and buffering stalls; tearing the grabber
  // down on those is what made it vanish until the user paused and played again
  // (and left `vEl` null, so a click did nothing). We no longer tear down on
  // playback events at all: `placeVideo()` hides the button when the player
  // scrolls out of view, and a detached or replaced element reads as a zero
  // rect, so the button goes away on its own. A fresh `playing` on a new element
  // re-tracks it via `showVideo`.
  document.addEventListener('ended', () => placeVideo(), true);
  vbtn.addEventListener('click', async (e) => {
    e.preventDefault();
    e.stopPropagation();
    const el = vEl;
    if (!el) return;
    let cands = [];
    if (/(^|\.)youtube\.com$/.test(location.hostname) && location.pathname === '/watch') {
      try { cands = (await youtubeCandidates()) || []; } catch { cands = []; }
    }
    if (!cands.length) cands = videoCandidates(el);
    if (!cands.length) {
      say('Sucr found no video stream here', el, true);
      return;
    }
    // Reflect any watch already running for this video so a protected row the
    // worker is re-probing reads as Watching, not as a fresh Watch button.
    const vid = location.pathname === '/watch' ? new URLSearchParams(location.search).get('v') : '';
    if (vid) {
      watching.clear();
      try {
        const st = await api.runtime.sendMessage({ op: 'retry-status', videoId: vid });
        if (st && st.ok && Array.isArray(st.itags)) for (const it of st.itags) watching.add(`${vid}:${it}`);
      } catch {}
    }
    // Always open the format list, even for a single stream: a silent one-click
    // grab hid the other qualities and read as "only 1 format, no dropdown."
    openVideoList(cands);
  });
  vX.addEventListener('click', hideVideoList);
  // A click anywhere outside our shadow closes the stream list. Shadow
  // events retarget to the host, so anything else is the page.
  document.addEventListener('pointerdown', (e) => {
    if (vlist.hasAttribute('data-shown') && e.target !== host) hideVideoList();
  }, true);

  // --- the file info panel ---------------------------------------------------

  function bytes(n) {
    if (n == null || !(n >= 0)) return '';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
    return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  }

  function hidePanel(notify) {
    clearTimeout(panelTimer);
    panel.removeAttribute('data-shown');
    panel.removeAttribute('data-bad');
    if (notify) api.runtime.sendMessage({ op: 'panel-close' }).catch(() => {});
  }

  function showPanel(msg) {
    if (!enabled) return;
    clearTimeout(panelTimer);
    panel.removeAttribute('data-bad');
    pName.textContent = msg.filename || 'This file';
    const size = bytes(msg.totalBytes);
    pMeta.textContent = msg.id == null
      ? 'The browser is fetching this file for Sucr' + (size ? ` · ${size}` : '')
      : (msg.phase === 'adopt' ? 'Sucr is taking the finished file' : 'Sucr is taking this file') + (size ? ` · ${size}` : '') +
        // A positive ride note only. If it shows and the row still stalls, the
        // block is Google's gate, not a missing session. If it never shows, the
        // browser session was not found for this link (play it first, sign in).
        (msg.session === 'id' || msg.session === 'last' ? ' · riding your browser session' : '');
    pBar.removeAttribute('data-ind');
    pBar.style.width = '0%';
    if (msg.id == null) pBar.setAttribute('data-ind', '');
    panel.setAttribute('data-shown', '');
  }

  function panelRow(row) {
    if (!panel.hasAttribute('data-shown') || !row) return;
    if (row.filename) pName.textContent = row.filename;
    const total = row.totalBytes || row.wantedBytes;
    const done = row.verifiedBytes;
    const size = bytes(total);
    const running = row.status === 'queued' || row.status === 'downloading' || row.status === 'paused';
    if (row.status === 'needs-attention' || row.status === 'failed' || row.status === 'cancelled') {
      panel.setAttribute('data-bad', '');
      pBar.removeAttribute('data-ind');
      pMeta.textContent = row.errorHint || row.errorCode || 'Sucr could not take this file';
      return;
    }
    panel.removeAttribute('data-bad');
    if (row.status === 'finished' || row.status === 'verified') {
      pBar.removeAttribute('data-ind');
      pBar.style.width = '100%';
      pMeta.textContent = 'Finished' + (size ? ` · ${size}` : '') + (row.folder ? ` · ${row.folder}` : '');
      panelTimer = setTimeout(() => hidePanel(true), 6000);
      return;
    }
    clearTimeout(panelTimer);
    if (running && total > 0 && done != null) {
      pBar.removeAttribute('data-ind');
      pBar.style.width = Math.min(100, Math.round((done / total) * 100)) + '%';
      pMeta.textContent = `${row.status === 'paused' ? 'Paused' : row.status === 'queued' ? 'Queued' : 'Downloading'} · ${bytes(done)} of ${size}`;
    } else if (running) {
      pBar.setAttribute('data-ind', '');
      pMeta.textContent = (row.status === 'paused' ? 'Paused' : row.status === 'queued' ? 'Queued' : 'Downloading') + (size ? ` · ${size}` : '');
    }
  }

  pOpen.addEventListener('click', () => {
    api.runtime.sendMessage({ op: 'wake' }).catch(() => {});
  });
  pClose.addEventListener('click', () => hidePanel(true));

  // The background's verdicts: a toast when no panel is open, the panel itself
  // for a capture this tab asked for.
  api.runtime.onMessage.addListener((msg) => {
    if (!msg) return;
    if (msg.op === 'sucr-say') say(String(msg.text || ''), null, !!msg.bad);
    else if (msg.op === 'sucr-panel') showPanel(msg);
    else if (msg.op === 'sucr-panel-row') panelRow(msg.row);
  });

  document.documentElement.appendChild(host);
})();
