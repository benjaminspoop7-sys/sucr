# Manifest sniffing in the add-on — what shipped and the six bugs it fixed

Date: 2026-11-11. Scope: `extension/background.js`, `extension/content.js`
only — 0 exe bytes. Shipped in 0.1.1 (commit `944244c`). Plan origin: the
universal-capture direction (manifest-aware capture for HLS/DASH/direct).

## What ships

- The background service worker sniffs every response whose **pathname** ends
  `.m3u8` or `.mpd` (query strings allowed) in tabs it can attribute, gated by
  `d.tabId >= 0 && settings.enabled && !siteOff(d.url)`, and keeps a per-tab
  ledger `tabManifests: Map(tabId -> Set(href))`, dropped on `tabs.onRemoved`.
- Dispatch is narrowed to `['*://*/*.m3u8*', '*://*/*.mpd*']` instead of an
  all-URLs listener, so the worker is not woken for every response.
- The content script pushes detections (`manifest-detected` messages) and
  pulls the ledger at startup (`manifest-list`), so a page that *is* the
  manifest — served before the content script existed — still toasts.
- Detection shows a toast naming the file; the video grabber lists each
  manifest as a row (`HLS Manifest · m3u8` / `DASH Manifest · mpd`).
- **Known limit, disclosed in the row comment:** Take hands the manifest URL
  to the engine as-is, so it saves the playlist file, not a playable video.
  The engine-side manifest parser + segment muxing is the next task.

## The six bugs the check found (all fixed before ship)

1. Dead candidate path: `MEDIA_EXT` excludes m3u8/mpd, so the network-media
   list could never hold a manifest — the ledger replaced that route.
2. `endsWith('.m3u8')` missed query-string manifests — pathname regex now.
3. `tabs.sendMessage` rejection / `tabId === -1` unhandled — `.catch` + gate.
4. Sniffer ignored `settings.enabled` and the per-site `siteOff()` gate —
   both now consulted before recording or toasting.
5. `<all_urls>` webRequest listener = service-worker wake storm under
   Chrome's event rate cap — narrowed URL filter.
6. Injection race: a direct `.m3u8` navigation fires webRequest before the
   content script exists, losing the toast — the startup `manifest-list` pull
   closes it.

## Evidence

- `node --check` clean on both scripts; all four dists (chrome/firefox/dev×2)
  rebuilt and grep-verified to carry `manifest-list`.
- Bug-check harness legs in a real Chromium (Helium, throwaway dev-session
  profile, dev add-on only — the installed app and its add-on untouched),
  screenshots in `scratch/realpath/bc-*.png` (harness itself lives in
  gitignored `scratch/`):
  - A plain `.m3u8` navigation → toast over the native HLS player. (writer:
    webRequest listener; predicate: toast text visible)
  - B query-string manifest → toast. (proves fix 2)
  - C direct `.mpd` navigation → Helium intercepts it as a download, no page
    hosts a toast; same code path as A, so treated as environmental, not a
    product failure.
  - D capture disabled → the HLS page plays with no toast. (proves fix 4)
  - E grabber list over a playing video → three `HLS Manifest · m3u8` rows.
    (proves the ledger reaches `videoCandidates`)
- Engine suite unchanged by this slice: 246 passed / 0 failed / 9 ignored.

## Not yet done

Segment fetching, manifest parsing in Rust, and muxing to a playable file —
the capture plan's engine half. Until then the manifest rows are honestly
labelled and save the manifest text.
