# Sucr Capture — extensive test and performance record

Date: 2026-11-08. Scope: the browser add-on (`extension/dist/chrome`, built 00:45:51,
newer than every source edit) plus the shipped `sucr.exe` it wakes.
Browser: Helium (Chromium 156) on scratch profiles only. Fixture: loopback HTTP
server with Range support and optional per-connection throttle. Nothing here
touches the user's profile, data dir, window or database; every scratch app is
stopped by its own door pid at the end (verified: no survivors).

Method rules honoured: rotated passes and medians, never a single shot; every
comparison has a control; a measurement whose witness can be satisfied by stale
state is void and was re-run (see §5). Timestamps are poll-based: door at 50 ms,
file/recent at 100–150 ms, so each number carries up to one poll interval of
quantisation.

Harnesses: `scratch/realpath/extensive.mjs` (11 functional legs),
`scratch/realpath/perf.mjs` (latency, throughput, throttled throughput,
page-load overhead, SW heap). Logs: `extensive-run3.log`, `perf-run2.log`,
`perf-run3.log`, `perf-results.json` (run 3).

## 1. Functional matrix (extensive run 3, all legs green)

| Leg | Behaviour proven | Witness |
|---|---|---|
| 1 | Four normal clicks 150 ms apart all taken concurrently | 4 db rows `finished`, 4 files at exact fixture size, browser-dl dir empty after bounded settle, badge `↓` then count |
| 2 | Popup "Take all" over 8 links | popup lists 8 rows, 8 files land, badge `12` |
| 3 | 64 MiB at 1 MiB/s per connection: live panel percent | samples 0% → 6% → … → 100% → `Finished · 64.0 MB · <folder>`; wall 9.6 s; `verified_bytes` 67 108 864 |
| 4 | 302 redirect + server-chosen filename | file and panel named `renamed-by-server.zip` |
| 5 | Query URL, no disposition, no extension | saved as `download`, panel `Finished` |
| 6 | Adopt mode | recent `ok:true mode:adopt`, file in library, browser-dl empty, panel "taking the finished file" |
| 7 | Import mode on a 4 MiB throttled file | browser item paused then erased, library file complete, panel `Finished` |
| 8 | Per-site stand-down | chip hidden on hover (`shown:false`), normal click stays in browser (item complete in browser-dl), db rows unchanged |
| 9 | Same URL clicked twice | two rows, `dup.zip` + `dup (1).zip` (uniquify, not silent overwrite) |
| 10 | Alt+click prevent key | browser item complete, db rows unchanged |
| 11 | Transfer dies mid-body after probe refusal | probe 403 → browser leg `interrupted / SERVER_CONTENT_LENGTH_MISMATCH` → red panel "the browser could not finish this file", badge `!`, recent `ok:false browser-failed` |

End state: 19 db rows, every one `finished` with `verified_bytes` equal to the
fixture size and `source_browser: chrome`; zero service-worker errors, zero page
errors. Renders viewed and matching the log: `ext-3-big-panel.png` (finished
panel with folder and full bar), `ext-8-site-off.png` (no chip, no panel),
`ext-11-refusal-panel.png` (red verdict).

## 2. Capture latency (1 MiB unthrottled, 3 cold + 3 warm passes, rotated C/W)

Click-referenced medians, run 3 (run 2 replication in parentheses):

| Stage | Cold (app not running) | Warm (app running) |
|---|---|---|
| Door appears (app bound) | 294 ms (345) | — |
| Take accepted by app | 450 ms (446) | 178 ms (177) |
| File complete on disk | 1 063 ms (1 109) | 782 ms (782) |
| Panel shown in page | 1 371 ms (1 363) | 180 ms (179) |

Per-pass (run 3): cold door 319/292/294, accepted 627/447/450, file
1 240/1 058/1 063, panel 1 397/1 365/1 371; warm accepted 177/213/178, file
780/1 119/782, panel 180/216/180. Cold start therefore costs roughly +270 ms to
accept and +0.6 s to the in-page verdict over warm; a warm click is answered in
under a fifth of a second.

## 3. Throughput, 16 MiB, take vs browser-native (Alt), 4 rotated passes each

| Link shape | Take median | Browser median | Verdict |
|---|---|---|---|
| Unthrottled loopback | 2 676 ms ≈ 6.0 MB/s (run 2: 2 531) | 816 ms ≈ 19.6 MB/s (run 2: 784) | browser single stream wins |
| 1 MiB/s per connection | 3 384 ms ≈ 4.7 MB/s | 16 520 ms ≈ 0.97 MB/s | **take 4.9× faster** |

On a fast local link the engine's spread streams, preallocation and byte
verification cost more than they save; on a per-connection-limited link — the
case a download manager exists for — the spread aggregates and wins by ~5×.
The 64 MiB throttled functional leg agrees: 9.6 s wall (≈6.7 MiB/s aggregate)
in both extensive runs 2 and 3, where a single browser stream would need ~64 s.
This is a documented trade-off, not a defect; no code change made.

## 4. Page-load overhead and service-worker footprint

Overhead control: six fresh-profile launches (3 rounds × 2 arms, order
rotated), 10 cache-busted navigations each, median of the settled last 8.
Both arms strip Helium's bundled add-ons (`--disable-extensions-except` in the
ext arm, `--disable-extensions` in the plain arm) so the only difference is
Sucr. Run 3 medians-of-medians: DCL 9.5 ms ext vs 9 ms plain; load 11.5 ms vs
10 ms (run 2: 9/9 and 11/10). **No measurable page-load overhead** — the
difference is below timer resolution on this trivial page.

Two symmetric artefacts, excluded by the median and present in both arms:
the first navigation of a fresh profile costs 2.7–4.9 s (profile warmup), and
one navigation per launch (usually the sixth) stalls ~4.4 s in both arms — a
browser-side periodic task, not the add-on.

SW JS heap (`Runtime.getHeapUsage`): idle 598 400 B; after the latency passes
1 094 940 B; after all throughput passes 1 327 652 B. Extensive run 3: idle
568 888 B, 810 060 B under four concurrent clicks, 1 496 516 B at end of 19
downloads. The worker stays in the 0.6–1.5 MB band.

## 5. Void measurements retracted, and harness fixes (no product code changed)

- Perf run 1 latency: passes reused one URL, so `recent` and an existing file
  satisfied the cold-pass witnesses before the app started (accepted "22 ms"
  with the door at 290 ms — impossible). Void. Fixed with a unique filename per
  pass; runs 2–3 are the record.
- Perf run 1 overhead: `--disable-extensions-except` strips Helium's bundled
  uBlock in the ext arm only, so "plain" measured uBlock, not absence of Sucr
  (it looked like the add-on made pages faster). Void. Fixed with a symmetric
  `--disable-extensions` control; runs 2–3 are the record.
- `Performance.enable` does not exist on a service-worker CDP session; heap
  sampling uses `Runtime.getHeapUsage` instead.
- Adopt/import legs: the browser item is erased by design once the file is
  handed over, so a "complete item" search can never witness it. Witnesses are
  the recent entry, the library file and the empty browser-dl dir.
- Site-off: the chip is hidden by design (hover is gated on `enabled`), so the
  leg asserts hidden chip + browser keeps the click, not a refusal toast.
- Browser-copy dedupe waits on the app's byte proof (`sucr_holds`, bounded 6 s
  retry), so emptiness of the browser dir is checked after a bounded settle.

## 6. Gates and limits

Exe gate: `stat` 4 665 344 B, headroom 577 536 B under the ruled 5 242 880 B
ceiling. This session changed extension code and harnesses only: 0 exe bytes.
Rust suite untouched this session (last green: 226 passed / 6 ignored).

Limits: loopback fixture, not the internet — no TLS leg, no real loss/latency,
throttle is server-side per connection. One machine (5600X), one browser family
for these runs; Firefox-family take/adopt legs live in
`capture-browser-legs.md`. Overhead measured on a trivial HTML page, not a
heavy site. Poll quantisation ±50–150 ms per timestamp. Medians over 3–4
passes per cell, not distributions.

Evidence index: `scratch/realpath/extensive-run3.log`, `perf-run2.log`,
`perf-run3.log`, `perf-results.json`, `ext-3-big-panel.png`,
`ext-8-site-off.png`, `ext-11-refusal-panel.png`; harnesses `extensive.mjs`,
`perf.mjs`. Earlier reference-app parity legs and the installed-card reload receipt:
`capture-browser-legs.md`.

## 7. Video-only capture + real YouTube (video-only runs 4–5, 2026-11-08)

User ruling this slice: *capture only videos* (no fetch/script junk) and *no
toolbar-popup capture* — the surface follows the reference add-on: a "Download this video"
grabber over the playing media, an in-page stream list when there is a choice,
and the in-page status panel. The popup is settings-only (leg 5 witnesses
`list:false, takeAll:false, siteToggle:true`).

**Junk filter.** `MEDIA_EXT` is anchored to the URL **pathname** (`$`-anchored),
never the query string: run 3 had four `www.youtube.com` "Media file" rows that
matched extensions inside query *values*. Run 4–5 leg 2 puts `generate_204`,
`module.js` and preload junk on the page; the list carries only the two real
media rows (`junk in list: none`).

**YouTube source.** The web player streams over SABR: wire `/videoplayback`
URLs carry `sabr=1` and no mime/itag, and the page's own
`ytInitialPlayerResponse.streamingData` has 40 adaptiveFormats with **zero
`url`/`signatureCipher`** — only `serverAbrStreamingUrl`. Resource timing
therefore cannot supply per-format URLs. The content script instead makes one
same-origin `POST /youtubei/v1/player` with the ANDROID client
(`20.10.38`, sdk 30) per grabber click, which answers with direct URLs.

**The adaptive-format wall (measured, not engine).** Run 4 offered the full
adaptive ladder (9 video + 3 audio rows); the take created the row, the probe
answered 206, block 0 landed (8 388 608 B) and blocks 1–7 died on `403` through
5 retries → `needs-attention`. Isolation: the engine-only door leg
(`yt-engine.mjs`, scratch data dir) reproduced it, then **plain python
reproduced it too** — so it was never ureq/Schannel. The measured shape of the
wall on adaptive URLs (itag 302/160/249, fresh mints included):

- first bounded range `bytes=0-8388607` → 206; every further range → 403,
  sequential or 8-parallel, immediately or at +2/+10/+30/+60/+90 s;
- first range of 16/32/64 MiB → 403; open-ended `bytes=0-`, no Range header,
  or a first chunk starting past 0 → 403;
- `Origin: https://www.youtube.com`, Android-app UA, mint/fetch UA mismatch,
  `c=WEB` swap: no change. `alr=yes` hands a fresh GGC host URL — same wall;
- 403 body is empty, `Server: gvs 1.0`, `Vary: Origin`;
- WEB/MWEB/TVHTML5/IOS/ANDROID_VR/WEB_EMBEDDED clients are bot-gated
  (`LOGIN_REQUIRED`/`UNPLAYABLE`) from this IP **even from the watch page with
  its cookies** (`yt-pageclients.mjs`); only ANDROID answers.

**The open path: progressive formats.** itag 18 (muxed 360p MP4, video+audio,
`ratebypass=yes` in its URL) serves **unlimited** sequential and 8-parallel
1 MiB ranges (16/16 × 206). `youtubeCandidates` now offers only
`streamingData.formats` (progressive), deduped by itag, quality-descending,
labelled `Video · MP4 · 360p` / meta `video + audio`. Adaptive formats are not
offered because they cannot complete.

**Proof.** Engine door leg, fresh itag 18: `finished`,
`verified_bytes = total_bytes = 28 523 658`, **0 retries**, ~5 s, file on disk
exactly that size, `ftyp mp42`. Run 5 (`video-only-run5.log`): all six legs
green — legs 1–4 exact fixture bytes (53 390 / 46 223), leg 5 settings-only
popup, leg 6 real YouTube in Helium: grabber over the playing player → one
progressive candidate → direct take → row `finished`,
`verified_bytes 28 523 658` in ~4 s, title-based filename, zero SW/page errors,
browser-dl dir empty. `ffprobe` on the captured file: **h264 640×360 + aac
audio** — a complete, playable video. Shots `vid-7-youtube.png`,
`vid-8-youtube-capture.png`, `vid-9-youtube-result.png`.

**Retracted:** the run-4 claim that the ANDROID call yields "direct,
range-capable URLs" was true only for the *first ~8 MiB chunk* of an adaptive
URL; the adaptive ladder could not complete a download. The comment in
`content.js` now states the wall instead.

**Limits.** YouTube capture tops out at the progressive quality (360p on the
test video — the only progressive format YouTube offers). Higher qualities are
adaptive-only and need PO-token/SABR work that is out of this slice's scope;
parked, not promised. One video, one IP, one day: the wall's exact quota model
is inferred from the measurements above, not from any Google specification.
Gates: extension-only change, exe `stat` **4 665 344 B** unchanged (0 B spent);
`dist/chrome` + `dist/firefox` rebuilt via `extension/build.sh` before run 5.
Harnesses: `video-only.mjs`, `yt-engine.mjs`, `yt-sameurl.py`, `yt-rate.py`,
`yt-clients.py`, `yt-pagefetch.mjs`, `yt-pageclients.mjs` (all
`scratch/realpath/`).
## 8. The format ladder (ladder runs 1–2, 2026-11-09)

User ruling: the grabber must offer the different formats a page's video can
play, YouTube as the example. § 7 had narrowed YouTube to the progressive
(muxed) format because the adaptive URLs 403 past their first chunk. That
narrowing was too broad, and § 7's "adaptive formats are walled" claim is now
**narrowed by measurement**: the guard is **per video and per moment**, not
global.

Measured with the ANDROID innertube client, ranges of 16 B / 8 MiB, this box:

| video | best adaptive tier | ranges past the first chunk |
|---|---|---|
| `dQw4w9WgXcQ` | 2160p (342 MB) | 206, 206, 206 — open |
| `9bZkp7q19f0` | 1080p | 206 on the top tiers, 403 below 720p |
| `aqz-KE-bpKQ` | 2160p60 | 206 at 9 MiB in one pass, 403 in an earlier pass |
| `M7lc1UVf-VE` | 720p | 403 from the first probed block |

So a listed format cannot be trusted from its metadata alone, and it cannot be
judged from offset 0 either: the 8.9 MB row that survived a single-offset probe
in ladder run 1 then failed in the engine with `block-failed`, 0 bytes landed.
The shipped rule is therefore **probe before offering**: `youtubeCandidates()`
asks the background worker for three 16-byte ranges per row (offset 0, 9 MiB
when the file is larger, and 64 KiB before the end) and lists a row only when
every probe answers 206. Rows are deduped to one per quality label, a muxed
(video + audio) format winning its own tier over a same-height video-only
stream. Audio-only rows trail the videos.

Witnesses:

- Ladder run 2 (`yt-ladder-leg.mjs`, real Helium, scratch profile + scratch
  data dir): 11 rows offered — 2160p WEBM 342 MB, 1440p, 1080p MP4, 720p,
  480p, 360p (video + audio), 240p, 144p, then 137/131/50 kbps audio. The
  720p row was taken: row `finished`, `verified_bytes = total_bytes =
  26 455 880`, ~4 s, one file in `ytleg/downloads`, ffprobe
  `h264 1280x720, duration 213.04 s`, size exact. Zero SW errors, zero page
  errors, browser-dl dir empty. Screenshot `ladder-1-list.png`.
- Guarded case, `video-only.mjs` run 7: every walled adaptive row was dropped,
  the single surviving muxed candidate was taken directly, row `finished` at
  28 523 658 B. All six legs green. The list never offers a row the engine is
  known to fail.
- Probe plumbing: `manifest.json` gains `https://*.googlevideo.com/*`
  host permission (the content script's own range fetch is CORS-blocked; the
  worker's is not — measured), `background.js` answers `{op:'probe'}` with up
  to 48 parallel 8 s-bounded range requests.

§ 7's narrowing stands where it was measured (progressive URLs are never
walled); its "adaptive always walled" framing is superseded by this table.
PO-token minting was probed and rejected: the watch page's `GenerateIT` token
(88 chars, 12 h validity) does **not** unlock a replayed player call from
another process or from an in-page WEB call (measured: still 403 / UNPLAYABLE),
so no product bytes were spent on it.

Limits: probes cost up to ~30 small requests per grabber click on YouTube, and
they can go stale if Google flips the guard mid-list — the engine's own retry
and `needs-attention` path is the backstop. Adaptive rows are video-only: the
file has no sound by design (labelled "video only"), and Sucr does not remux
audio in. Quality offered is whatever the ANDROID client returns; SABR-only
videos still yield just the progressive tier.

Gates: extension-only change, exe `stat` **4 665 344 B** unchanged (0 B spent),
headroom 577 536 B under the ruled 5 242 880 B ceiling. Rust suite untouched.
Harnesses added: `yt-ladder.py`, `yt-ladder-leg.mjs`, `yt-clients2.py`,
`yt-pot.mjs`, `yt-pot2.py`, `yt-pot3.mjs`, `yt-wire.mjs`, `yt-botguard.mjs`,
`yt-realvids.py` (all `scratch/realpath/`).

## 9. Real-install diagnosis + probe-resilience fix (2026-11-09)

`writer:` this agent · `predicate:` the shipped `youtubeCandidates()` keeps a row unless a probe answers a definite non-206 HTTP status · `evidence-noun:` the `codes` array from `background.js` `case 'probe'`.

The user reported "the extension still doesn't work" on the real install (Helium + the loaded Sucr Capture card + the tray app). Ground truth measured, not assumed:

- Door alive: `C:\Users\Benjamin\AppData\Roaming\Sucr\capture-door.json` → pid 1720, port 52437, mtime 08:44:19; `sucr.exe --background` from `D:\RE\Sucr\app\src-tauri\target\release` is the running process (the user's own — never touched by this diagnosis).
- Native host registered: `HKCU\Software\Google\Chrome\NativeMessagingHosts\app.sucr.capture` → `…\Roaming\Sucr\native\app.sucr.capture.chrome.json` (written 08:44), `path` = that same exe, `allowed_origins` = `chrome-extension://hilnalhpghfpbphghnicaadhlegjaebf/`.
- Card loaded from `D:\RE\Sucr\extension\dist\chrome` (Secure Preferences), id matches; build (06:38) predates Helium+app start (08:44), so their live card carried the §8 ladder code.
- Door auth proven from an external client: a `POST /state` with the real token + `chrome-extension://…` origin reached route dispatch (answer `not-found` for the unknown route), so token+origin pass.
- Their stored settings (extension `Local Extension Settings` WAL, read from a copy — the live LOCK was held, not touched): `enabled=true`, `mode` last write `take`, `sites` last write `{"www.youtube.com":true}`. Config fully on.

Read path reproduced in an isolated `--user-data-dir` Helium loading only `dist/chrome` (their real profile/db never opened; no `SUCR_DATA_DIR`, and no Take performed): the grabber button appears over a playing `dQw4w9WgXcQ` and the probe-gated list renders the full 11-row ladder, zero page errors, zero SW errors. So detection + offering + auth are all healthy in their exact condition; the only leg that could not be exercised is the actual Take download against their real library.

**Defect found by code inspection, not reproduced live:** `background.js` `case 'probe'` returns `0` when the googlevideo fetch *throws* (blocked by another extension, offline, aborted). The §8 keep-rule was `if (code !== 206) good[row]=false`, so an all-`0` `codes` array (a valid array, not the missing-`codes` fallback) dropped **every** row → empty list → reads as "doesn't work." A `0` means googlevideo was unreachable, which is not evidence a format is walled; only a definite non-206 HTTP status is.

**Fix (extension-only, 0 exe bytes):** the keep-rule is now `if (code && code !== 206)` — a `0` is inconclusive and keeps the row; only an HTTP status that isn't 206 prunes it. Deterministic proof of the filter (one probe per row, 4 rows):

| probe outcome | old kept | new kept |
|---|---|---|
| all 206 (healthy) | 4 | 4 (working path unchanged) |
| all 0 (uBlock / offline / aborted) | **0 → empty** | **4 → offer everything** |
| 403,403,403,206 (walled adaptive, muxed serves) | 1 | 1 (correct pruning preserved) |
| 403,403,403,0 (walled video, muxed probe blocked) | 0 | 1 (keeps the muxed row) |

Real-browser re-run of the read path on the fixed build: still 11 rows, zero errors — no regression.

**Action owed by the user:** their live Helium still runs the pre-fix code cached at the 08:44 load. The card must be reloaded (chrome://extensions ⟳ on Sucr Capture, or a Helium restart) to pick up the rebuilt `dist/chrome`. If a capture still fails after reload, the only untested leg is the Take download on their real library and the concrete symptom (button missing vs click-nothing vs row fails) is the deciding datum.

**Gate:** exe unchanged at 4 665 344 B (headroom 577 536 B under the ruled 5 242 880 B ceiling). `writer:` this agent.

## 10. Grabber UX on the real install (2026-11-09)

The user's screenshot of a YouTube watch page showed two affordances at once — a generic "Download with Sucr" link chip top-left over the player, and the "Download this video" grabber top-right — and reported that clicking the grabber downloaded only one format with no dropdown. Both are extension-only, 0 exe bytes.

- **Silent single take removed.** `vbtn` click used to `take(cands[0])` when `cands.length === 1`. On a real install that path fired because §9's probe-blocked bug emptied `youtubeCandidates()` and the handler fell through to `videoCandidates(el)` (the one playing-media source) → one row → silent grab. Now the handler always `openVideoList(cands)`; the dropdown shows even for a single stream. With §9 fixed, a normal video returns the full ladder. `writer:` this agent · `predicate:` the click handler has no `cands.length === 1` branch.
- **Generic chip suppressed over the player.** `showChip()` returns early when the hovered link's rect overlaps the active `vRect`. Proven on four rects: an overlay link and a link wrapping the player are suppressed; a sidebar link (right) and a title link (below) still get the chip. So over the player only the grabber (right) remains; unrelated links elsewhere keep the standard link chip.

Real-browser re-run (`dQw4w9WgXcQ`, isolated profile): grabber on the right, click opens the 11-row dropdown (2160p→144p + 3 audio), zero page/SW errors; screenshot `realdoor-list.png` shows no left chip over the player. **Gate:** exe unchanged at 4 665 344 B. **Owed:** the user must reload the Sucr Capture card so their live Helium picks up the rebuilt `dist/chrome`.

## 11. Full ladder always listed; one button per video page (2026-11-09)

After the §10 reload the user's screenshot still showed **one** format row ("Video · MP4 · 360p · video + audio") and, on a second page, **two** Sucr buttons at once (the "Download with Sucr" chip top-left and the "Download this video" grabber top-right). Their ruling: *"I do not want both of them to show up. The extension should only show up once a video is playing with all audio, and video formats."* Both fixes are extension-only, 0 exe bytes.

**Why one row survived on their machine (measured, not assumed).** `scratch/realpath/client-ladder.mjs` POSTs the innertube player response for five clients and range-probes every returned googlevideo URL exactly as the extension does. On `dQw4w9WgXcQ` the **ANDROID** client returns the full 11-row ladder (2160p→144p + 3 audio) and **every row answers 206** — IOS is HTTP 400 (bad device params), WEB/MWEB/TVHTML5 are UNPLAYABLE without the page's cookies. So the ladder is healthy; the collapse to one row is the **extension's own probe**, which runs through the service worker. There, an ad-blocker or another extension can return a definite **403** on the adaptive range fetches (not a throw/`0`, so §9's "0 is inconclusive" rule does not save them), pruning all adaptive rows and leaving only the muxed row whose sole probe was the free first chunk. The probe measured through a polluted path while Sucr's engine — a separate process with no blockers — downloads the same URL fine.

> **RETRACTED 2026-11-09 by §14.** The ad-blocker cause and "the engine downloads the same URL fine" are both false. The past-window 403 is Google's server-side **PO-token gate** on adaptive formats (reproduces from bare Node, no browser or blocker) and the engine hits the same wall — it lands the gated row at ~2 % as `block-failed`. **Fix 1 below was the regression** that broke extension downloads on gated ids; the probe is restored in §14. The rest of §11 (Fix 2, the one-affordance chip guard) stands.

**Fix 1 — stop pruning the ladder.** `youtubeCandidates()` no longer sends the `probe` message; it returns the full ladder unconditionally and the engine decides at download time. `predicate:` `grep -c "op: 'probe'" dist/chrome/content.js` = 0; `return rows.map(({ clen, ...c }) => c);` present. — **SUPERSEDED by §14, which restores the `probe` send (grep = 1).**

**Fix 2 — one affordance on a video page.** `showChip()` now returns early whenever the grabber is active (`if (vEl && vRect) return;`), not merely where the chip overlaps the player. The earlier geometric guard missed the top-left chip beside a top-right grabber. `predicate:` `grep -c "if (vEl && vRect) return;"` = 1 and the overlap test `r.left < vRect.right` is gone. On a page with no playing video `vEl` is null, so the standard link chip still serves generic links.

Real-browser re-run (`dQw4w9WgXcQ`, isolated profile, fixed build): grabber reads "Download this video", click opens the **11-row** dropdown (2160p→144p + 3 audio), zero page/SW errors. **Gate:** exe unchanged at 4 665 344 B (headroom 577 536 B). **Owed:** the user must reload the Sucr Capture card (chrome://extensions ⟳ or Helium restart) to pick up the rebuilt `dist/chrome`. `writer:` this agent.

## 12. Grabber persistence across scroll (2026-11-09)

The user reported the extension "is not persistent, and would disappear soon." Cause, read from the code: `addEventListener('scroll', () => { reset(); hideVideo(); }, { capture: true, passive: true })`. A window-level capture listener fires for **every** scroll on the page — the comments feed, the sidebar, the player snapping into place on load — and `hideVideo()` tore the grabber down and nulled `vEl`. Nothing re-shows it, because the only other call site is the `playing` event, which does not fire again while the video keeps playing. So one wheel tick made the button vanish for the rest of the video.

**Fix (extension-only, 0 exe bytes):** positioning is split out of `showVideo` into a non-destructive `placeVideo()` that recomputes the video rect, repositions the grabber to follow it, hides the button only when the video has scrolled out of the viewport (`r.bottom <= 0 || r.top >= vh`) or is a tiny/detached element, and **never clears `vEl`**. `showVideo(el)` now just sets `vEl = el` then calls `placeVideo()`. The scroll handler calls `placeVideo()` instead of `hideVideo()`. Blur no longer drops `vEl` (it hides the button and a new `focus` listener calls `placeVideo()` to restore it). Only a real pause/end still tears down via `hideVideo()`. `predicate:` `grep -c "reset(); placeVideo(); }"` = 1 and `grep -c "reset(); hideVideo(); }, { capture: true, passive: true });"` = 0.

Real-browser proof (`scratch/realpath/yt-persist.mjs`, `dQw4w9WgXcQ`, isolated profile, no Take): A grabber shown after play (top 76px over a video at top 68px) · B after 5s steady playback still shown (no self-vanish) · C after `scrollBy 120` with the video still partly in view it stays shown and follows (top clamps to 8px) · D after `scrollTo 2000` pushing the video fully off the top it hides · E after `scrollTo 0` it returns (top 76px). Zero page errors. **Gate:** exe unchanged at 4 665 344 B (headroom 577 536 B). **Owed:** reload the Sucr Capture card to pick up the rebuilt `dist/chrome`. `writer:` this agent.

## 13. Neither affordance before the video actually plays (2026-11-09)

The user reported the old bug again — *"the previous bug 'Download with Sucr' and 'Download this Video' before the video even loads"* — and told this agent to stop one-by-one checking and drive the extension to completion with the verified-task-loop discipline. Two things were still wrong on a cold watch page.

**Cause, read from the code.** (1) The grabber revealed on the `playing` event via `showVideo`, but YouTube's SABR/MSE player can fire `playing` during the first buffering stall while the media clock is still `0` and no frame is on screen, so the "Download this video" button popped over a black player before the video loaded. (2) The §11 chip guard `if (vEl && vRect) return;` only suppressed the generic chip *after* the grabber had locked a video rect; before playback `vRect` is null, so hovering any link on the page still raised "Download with Sucr" over a not-yet-loaded player — two affordances again.

**Fix (extension-only, 0 exe bytes).**
- **Playback gate on reveal.** `showVideo(el)` now refuses to place the button unless the element has genuinely started: `!(el.readyState >= 2 && !el.paused && el.currentTime > 0)`. A fresh `timeupdate` listener (capture) calls `placeVideo()` the moment the clock advances, so a stall-`playing` still brings the grabber once frames render. `predicate:` `grep -c "el.readyState >= 2 && !el.paused && el.currentTime > 0"` = 1 (showVideo) and a second site in the `timeupdate` listener; `grep -c "document.addEventListener('timeupdate'"` = 1.
- **Chip retired on any video page.** `showChip()` drops the vEl/vRect test for `if (hasPlayer()) return;`, where `hasPlayer()` is true whenever any `<video>` is at least 64×40 — a player exists on the page whether or not it is playing yet, so the generic chip never competes with the grabber and never pops pre-load. On a page with no player `hasPlayer()` is false, so the standard link chip still serves generic links and media. `predicate:` `grep -c "if (hasPlayer()) return;"` = 1, `grep -c "function hasPlayer()"` = 1, and §11's `grep -c "if (vEl && vRect) return;"` = 0 (replaced).

**Real-browser proofs, no Take, isolated profile.**
- `scratch/realpath/yt-preload.mjs` (autoplay off, timeline sample): `btn=n` at t=0.07 while the player is still stalled → `btn=Y` at t=0.54 once frames render; `chip=n` throughout. The grabber no longer appears before the video loads.
- `scratch/realpath/yt-sweep.mjs` (deterministic http fixture serving a real ffmpeg `clip.mp4` with Range/206 support, a `frame.jpg`, and three pages): non-YouTube `<video>` — before play `vbtn.shown=false`, hovering a download link on that page `chip.shown=false` (hasPlayer suppresses it), after play (currentTime 0.37) `vbtn` REVEALED, click lists its own row (`Media file · 127.0.0.1`, via `videoCandidates` over the direct src); image page — hover `<img>` gives `chip` "Download media with Sucr"; plain link page — hover `<a>` gives `chip` "Download with Sucr". Zero page errors on all three surfaces.
- Regression held: `yt-realdoor-diag.mjs` still returns the full 11-row ladder (§11) and `yt-persist.mjs` still shows PERSIST / STILL-SHOWN / off-top-hidden / RETURNED (§12), both with zero page/SW errors.

**Gate:** exe unchanged at 4 665 344 B (headroom 577 536 B under the ruled 5 242 880 ceiling); source matches both `dist/chrome` and `dist/firefox` byte-for-byte. **Owed:** the user must reload the Sucr Capture card (chrome://extensions ⟳ or restart Helium) so their live browser picks up the rebuilt `dist/chrome`. `writer:` this agent.

## 14. The PO-token wall, and why the §11 un-prune was the download-failure regression (2026-11-09)

The user returned with a screenshot: two GTA6 "Download from YouTube" rows stuck at 2 % (8.0 MB of 396 MB) → **Stopped / Needs attention / `block-failed`**, Source an `googlevideo.com/videoplayback` **itag=137** URL, "9 of 32 parallel streams". *"The downloads from extension would not work. I said to check everything until it works."* This section supersedes §11's Fix 1 and retracts the premise it rested on.

**§11's premise was FALSE, not merely incomplete.** §11 reasoned that the service-worker probe was polluted by an ad-blocker that 403s adaptive range fetches *even though Sucr's own engine — a separate process, no blockers — downloads the same URL fine*, so it stopped pruning and listed the whole ladder "and let the engine decide at download time." Both halves of the italicised claim are wrong, measured:

- The 403 past the first chunk is **Google's server-side Proof-of-Origin (PO-token) gate on adaptive formats**, not a local blocker. It reproduces from a **bare Node `fetch`** (`scratch/realpath/yt-blockdiag.mjs`, `yt-shipped-rule.mjs`) with no browser, no extension, no ad-blocker in the path. The ANDROID innertube player call returns the adaptive URL self-signed (no cookies), yet only its first ~12–16 MiB serves.
- Sucr's engine does **not** download the gated adaptive URL fine. It reads in **8 MiB blocks** (`engine/job.rs:2994`), so it pulls the free first block, the next block hits the 403, and the row lands at ~2 % as `block-failed` (`job.rs:1813/2056`). That is exactly the screenshot. Listing the gated row so "the engine decides" was the defect, not the fix.

**Measured per-video wall (`yt-shipped-rule.mjs`, ANDROID client, offsets 0 and clen−64 KiB, this box):**

| id | itag 137 (1080p) | itag 18 (360p muxed) |
|---|---|---|
| `hNJe9d__yQM` (gated, GTA6-class) | `[206, 403]` — walled past the free window | `[206, 206]` — full file serves |
| `dQw4w9WgXcQ` (ungated) | `[206, 206]` | `[206]` |

Adaptive (137/136/135/133/160 + large audio) all `[206, 403]` on the gated id; the **progressive muxed** itag=18 is never gated. §8's own table already showed this variability ("per video and per moment"); §11 threw the probe out entirely on one machine's symptom and broke every gated video. No innertube client bypasses the gate (WEB/MWEB/TVHTML5 are UNPLAYABLE without page cookies; IOS is HTTP 400), and page-captured WEB URLs 403 even at block 0 cookieless (T227). PO-token minting was already probed and rejected in §8. So a gated adaptive tier is simply **not downloadable**, and offering it cannot be made to work.

**Fix — restore probe-before-offering (extension-only, 0 exe bytes).** `youtubeCandidates()` again sends `{op:'probe'}` (transport still present at `background.js:614`, `case 'probe'`, up to 48 parallel 8 s-bounded range fetches). For each row it probes offset 0 and, when `clen > 12 MiB` (the conservative free-window floor), the range at `clen − 64 KiB`. Keep-rule is §9's: prune a row **only on a definite non-206 HTTP status** (`if (code && code !== 206)`); a `0` (worker throw/unreachable) is inconclusive and keeps the row, so an ad-blocker or offline probe no longer collapses the list to empty. If, on a pathological id, every probe 403s, the code keeps the muxed rows rather than offering nothing — an empty list reads as "the grabber doesn't work," the §9 lesson. The comment over the block now states the PO-token cause, replacing the false ad-blocker rationale. `predicate:` `grep -c "op: 'probe', probes" dist/chrome/content.js` = 1 (was §11's 0).

**Proofs, no Take, isolated profile, user library never opened.**
- Deterministic mirror of the shipped rule (`yt-shipped-rule.mjs`): gated `hNJe9d__yQM` → **OFFER=2** (itag=18 360p muxed `[206,206]`, one sub-window audio row); every gated adaptive row dropped `[206,403]`. Ungated `dQw4w9WgXcQ` → **OFFER=11**, full ladder. The 396 MB itag=137 the screenshot choked on is now never offered on a gated id.
- Real-browser read-path (`yt-prune-leg.mjs`, only `dist/chrome` loaded, fresh profile, no `SUCR_DATA_DIR`): `hNJe9d__yQM` renders exactly **2 rows** — "Video · MP4 · 360p | video + audio · 107 MB" and "Audio · WEBM · 63 kbps | audio only · 10.0 MB"; `dQw4w9WgXcQ` renders the full **11-row** ladder. Zero SW errors, zero page errors on both.
- What an offered row does in the engine is already witnessed: §8's take leg landed a 206-verified 720p row `finished` with `verified_bytes = total_bytes`; §7 established progressive URLs are never walled. The near-end 206 probe is the same evidence the §8 rule used, so the rows now offered are exactly the ones the engine completes. This trades "all formats" for "downloads that work" on gated videos — the only honest outcome without BotGuard attestation.

**§12/§13 regressions intact** — the edit touches only the `youtubeCandidates()` return path, not persistence or the playback-gated reveal.

**Gate:** exe unchanged at 4 665 344 B (headroom 577 536 B under the ruled 5 242 880 ceiling); extension-only. `writer:` this agent. **Owed:** the user must reload the Sucr Capture card (chrome://extensions ⟳ or restart Helium) to pick up the rebuilt `dist/chrome`; the live card still runs §11's un-pruned code and will keep offering gated rows until reloaded.

## 15. Show the whole ladder, label the protected rows (user ruling, 2026-11-09)

The user answered §14's trade-off by overruling it: a screenshot of a video whose top quality is 1080p, with the grabber offering only 360p/144p + audio, and the words *"The extension do not show all the video formats. The video is upto 1080P."* The §14 prune hides exactly the rows the user wants to see. Two rulings now sit against each other — §14 "downloads that work" (hide gated) and this one "show all formats" (don't hide) — and only one can ship, because a gated 1080p row is not fully downloadable by the engine. The user's latest, explicit instruction wins: **list every format, do not prune.**

**Fix (extension-only, 0 exe bytes).** `youtubeCandidates()` keeps the probe but no longer drops rows. For each row it still probes offset 0 and, when `clen > 12 MiB`, the range at `clen − 64 KiB`; a row whose probe returns a definite non-206 is not removed but **labelled** — its `meta` gains `· protected, may stop early`. A `0` (worker unreachable) is not treated as protection. So the full ladder is offered, the working rows read plainly, and the cost of a protected row is visible before Take instead of arriving as a silent `block-failed`. `predicate:` `grep -c "protected, may stop early" dist/chrome/content.js` = 1; the §14 prune block (`const keep = good.length > 0`) is gone.

**Real-browser proof, no Take, isolated profile, user library never opened** (`yt-prune-leg.mjs`, `hNJe9d__yQM`, fixed build): the grabber renders **9 rows** — "Video · MP4 · 1080p | video only · 396 MB · protected, may stop early", 720p/480p/240p/144p likewise labelled, "Video · MP4 · 360p | video + audio · 107 MB" (the muxed row, unlabelled — it serves fully), and three audio rows (the two large ones labelled, the 10 MB one clean). Zero SW errors, zero page errors. The gate is also **per-moment**, re-confirmed: an earlier §14 run on this id returned a different surviving set, and this run's ANDROID response carried itag=134 where §14's carried itag=18 — the ladder Google hands back shifts between calls, which is why the list is labelled live rather than judged from metadata.

**What this does not change:** a protected row, if taken, still stops at ~2% with `block-failed` — that is Google's server-side wall, not a Sucr defect, and the label now says so on the row. The only way to make those tiers actually download is BotGuard attestation the engine does not have (§8, §14). The user chose visibility over hiding; the failure is now disclosed, not hidden.

**§12/§13 regressions intact** — only the `youtubeCandidates()` return path changed.

**Gate:** exe unchanged at 4 665 344 B (headroom 577 536 B under the ruled 5 242 880 ceiling); extension-only. `writer:` this agent. **Owed:** reload the Sucr Capture card to pick up the rebuilt `dist/chrome`.

## 16. Grabber sticks on first play; click reliably grabs (2026-11-09)

The user reported two symptoms with a screenshot of the "Download this video" button: *"when clicked links do not grab to the downloader"* and *"the extension would not stick unless the video is paused, and played again. I need the extension to stick and stay the moment when the video was first played."*

**Cause, read from the code.** §12/§13 had wired a teardown on `pause` / `emptied`: after 1500 ms, if the tracked element was still `paused`/`ended`, `hideVideo()` ran, which **nulled `vEl`** and removed the button. A real YouTube first-load fires transient `pause`/`emptied` events during ad breaks and buffering stalls, so a stall longer than 1.5 s tore the grabber down right after it appeared. The button only came back on the next `playing` event — which, in the user's flow, was the manual pause→play. And because `hideVideo` nulled `vEl`, the grabber-button click handler (`const el = vEl; if (!el) return;`) returned early during that dead window: the click grabbed nothing. Both symptoms are one root: the playback-event teardown.

**Fix (extension-only, 0 exe bytes).** The `pause`/`emptied`/`ended` teardown is removed. Once the video genuinely starts (the §13 reveal gate `readyState >= 2 && !paused && currentTime > 0`, unchanged), the grabber **stays**: `placeVideo()` hides the button when the player scrolls out of view, and a detached or replaced element reads as a zero rect, so the button goes away on its own without nulling `vEl`; a fresh `playing` on a new element re-tracks via `showVideo`. `ended` now calls `placeVideo()` (keep, reposition) instead of `hideVideo()`. `predicate:` `grep -c "stopTimer" dist/chrome/content.js` = 0 (the timer is gone); `grep -c "if (e.target === vEl) clearTimeout(stopTimer)"` = 0.

**Real-browser proofs, no Take, isolated profile, user library never opened.**
- `scratch/realpath/yt-stick.mjs` (`dQw4w9WgXcQ`, fixed build): first play (never paused) → button `n` at t=1–3 s while `readyState=0, currentTime=0` (correct pre-load suppression), then **`Y` from t=4 s and held through t=20 s** — it sticks the moment the video starts. Click → list opens with **11 rows**. Then the discriminator: a 2.5 s pause → button **SHOWN** (was `NOT SHOWN` on the pre-fix build — the teardown hid it); replay → SHOWN. Zero SW/page errors.
- `scratch/realpath/yt-persist.mjs` re-run on the same build: scroll persistence still PERSIST / STILL-SHOWN / off-top-hidden / RETURNED, zero errors — removing the teardown did not regress §12.

**Bug 1 scope note:** the click→list→`take()`→`offer()`→`/add` handoff path is unchanged and was proven landing a `finished` row in §8's real take; the new failure mode it removes is the click landing in the `vEl`-null window. A Take that still does not reach the app after the card reload is the door/app-reachability leg, which the page toast names ("Sucr is not running") — not the grabber state.

**§13 regression intact** — the reveal gate is untouched, so neither affordance still appears before the video actually plays.

**Gate:** exe unchanged at 4 665 344 B (headroom 577 536 B under the ruled 5 242 880 ceiling); extension-only. `writer:` this agent. **Owed:** reload the Sucr Capture card to pick up the rebuilt `dist/chrome`.

## 17. "Download still fails after certain download" — diagnosed, not a defect (2026-11-09)

The user reported *"The download still fails after certain download."* This is the free-window wall (§8, §14) arriving as a partial file: Sucr reads the first ~8–13 MiB of a protected adaptive row, then Google 403s every later range, the engine's `Msg::Fail` sets the job cancel flag to `FAIL` ([engine/job.rs:1805-1816](../../app/src-tauri/src/engine/job.rs#L1805)), and the download stops partway. It is Google's server-side wall on a gated video, not a Sucr bug, and the §15 label "· protected, may stop early" already predicts it. **No product bytes were spent** — the finding is evidence, not a fix; §15/§16 already ship the disclosure and the stickiness.

**Per-row wall measured, HTTP Range probes from bare Node (cookieless, self-signed ANDROID URLs — the same path the engine uses) (`scratch/realpath/yt-wall.mjs`, `writer:` this agent, `predicate:` the `codes=[…]` table is the evidence):**
- `dQw4w9WgXcQ`, this run: **no wall on any row** — 2160p/1440p/1080p/720p/480p audio, every row reads `206` from offset 0 to `clen − 64 KiB`. Gate currently OFF.
- `hNJe9d__yQM`, this run: **gated** — 1080p `206` to ~13 MiB then `403` (396 MB file), 720p/480p/240p/144p `206` only at offset 0 then `403` at 8 MiB, both large audio rows walled; **the muxed 360p "video + audio" row (107 MB) reads `206` end to end** — the one row on a gated video that completes.
- The gate is **per-video AND per-moment**: `dQw4w9WgXcQ` was gated in the §14 run and is clean now. So "fails after certain download" is a property of the video at that moment, which is why the extension labels live instead of judging from metadata (§15).

**End-to-end proof the path works when the gate is off** (`yt-ladder-leg.mjs`, `dQw4w9WgXcQ`, shipped `dist/chrome`): grabber shows "Download this video" (§16), the full **11-row ladder** renders (§15), a **720p video-only adaptive row** (25.2 MB) is taken and the scratch-app DB row lands `status=finished`, `verified_bytes 26 455 880 == total_bytes 26 455 880`, `error_code=null`; the row was correctly **unlabelled** (no "protected, may stop early"), zero SW errors, zero page errors. Unlabelled ⇒ completes; the wall table shows precisely which rows would be labelled on a gated video and stop.

**Actionable state for the user:** a download that stops partway is a row carrying the "· protected, may stop early" label on a gated video — take an **unlabelled** row instead (the muxed 360p "video + audio" always completes; small rows fully under the free window do too), or the same tier completes whole once Google lets that video through. The only way to make a gated high-quality tier whole is BotGuard attestation the engine does not have (§8, §14) — out of scope. **Gate:** exe unchanged at 4 665 344 B; this section spent 0 bytes. `writer:` this agent. **Owed:** reload the Sucr Capture card so the live pick-up is §15/§16, not §11.

## 18. "Ride the browser session" tested — no reusable URL survives the gate (2026-11-09)

The user asked to bypass the wall the way a commercial download manager appears to. Before answering, I tested the one genuinely non-circumventing idea — that Sucr could download whole high-res by **riding the user's own attested browser session** (so the request looks like the browser YouTube already serves for playback) instead of the cookieless engine. It fails, and this records why, so it is not retried.

**Prior-art check.** String forensics on a widely used commercial download manager's shipped binary (read-only, not run) found only legacy YouTube markers — `get_video_info`, `redirector.googlevideo.com/videoplayback`, `%s&signature=%s` — and **no** `potoken`/`proof of origin`/`botguard`/`attestation`/`innertube`/`youtubei` string anywhere. So that model is the pre-2022 signature-decode era plus browser-session piggybacking; nothing in it mints a PO token. *Confirmed* from the strings; *inferred*: whatever such a client still lands on a gated video today is at most the free window or ungated content.

**Attested-browser test, read-path only, isolated profile, no library touched.** Two legs, the gated video `hNJe9d__yQM`, `play=OK`:
- `scratch/realpath/yt-attested.mjs`: pull `window.ytInitialPlayerResponse`, take the top adaptive (itag 137, 1080p, `contentLength 415 701 491`) and attempt an in-session fetch — but the fetch branch is skipped because the format carries **no `url`**.
- `scratch/realpath/yt-fields.mjs` confirms it directly: for the top adaptive `{urlPresent:false, sigCipherPresent:false, cipherPresent:false, keys:[]}`, `hasUrlInAnyAdaptive:false`, `hasUrlInAnyMuxed:false`. **The attested WEB response exposes no re-downloadable URL or cipher at all** — Google strips them unless the request itself carried a valid PO token, and the page's own playback runs on internal per-request MSE segment tokens that are not surfaced to page JS.

**Consequence, stated honestly.** There is no browser-side URL to adopt. Every route that yields a *whole* gated high-res file reduces to forging the PO-token/BotGuard attestation in Sucr's client — that is circumvention of a technological protection measure, and it is the part that enables bulk scraping. **This agent will not build it, independent of how the request is framed.** The §17 legitimate outcomes stand unchanged: ungated videos/moments already download 1080p/2160p whole (proven, §17), the muxed "video + audio" row always completes, and a per-moment retry can catch the window when Google lifts the gate — all without attestation. **Gate:** exe unchanged at 4 665 344 B; this section spent 0 bytes. `writer:` this agent.

## 19. A commercial add-on reviewed — no attestation defeat, one legit parity gap (2026-11-09)

The user corrected §18: the binary read there was the desktop app, but the piece that "still does it" in their mind is the **browser extension**. I reviewed the extension too. No copy of it, its source, or its identity is reproduced in this repository; this section records only the conclusions, in generic terms.

**What such an extension actually does for YouTube — every leg is evidence, no attestation code:**
- **Progressive-only itag table.** Its quality picker is built on the legacy **progressive/muxed** set (itag 16, 17, 18, 45, 46); the top of that ladder is 360p muxed. It never names an adaptive 137/248/264 tier. So the picker cannot offer a gated high-res stream — it offers what is ungated by construction.
- **Legacy signature decode, not PO token.** It scrapes the player-JS URL and related page variables and reproduces the pre-2022 `s`/`signature` scramble (find the player JS, run its decode function). A grep of the whole bundle for `attestation|botguard|pot|proof|interactive` returns **zero hits**. Nothing there mints the token the current gate demands.
- **It rides the real browser session.** It reads the cookie store, builds a `Cookie:` header, and attaches the browser's `User-Agent` before handing both to its native download engine, so a cookie-gated link works. This is the same session-piggyback §18 tested — it defeats *auth* walls, not the *PO-token* wall.
- **Manifest/segment sniffing.** A mime map routes `MPD`, `M3U8`, `M4S`, `F4M` and `application/vnd.yt-ump → UMP`. It captures DASH/HLS/UMP by sniffing the segment stream the player already pulls — which yields the free-window/MSE chunks, not a whole gated file from a single Range.

**Verdict, answering "they still do it, so why can't you".** That extension contains no PO-token/BotGuard defeat; on a gated adaptive tier it 403s past the free window exactly like Sucr does. Where it still lands a whole file today it is (a) progressive itag ≤ 360p, which is ungated, or (b) content whose only wall is a session cookie — and that second case is the **one legitimate parity gap** this review exposes: Sucr's engine is cookieless with a fixed desktop Chrome UA (`engine/mod.rs:136`), while a mature add-on forwards the browser's cookies + UA. Forwarding them would rescue cookie-gated captures for Sucr too. It would **not** beat the PO-token gate, so it is not the "make high-res work" answer — but it is a real, non-circumventing improvement. **Gate:** exe unchanged at 4 665 344 B; this section spent 0 bytes. `writer:` this agent.

## 20. Per-moment retry to open the gated window — built, proven, extension-only (2026-11-09)

The user approved the one legitimate route left standing after §17–§19: *"build the per-moment retry to open window."* The premise is §17's own finding — the gate is per-video **and** per-moment — so a high-res row that is walled now can open later on its own, no forged token involved. The add-on now watches an enrolled protected row on a timer and pulls it whole the instant Google lets that itag through.

**What ships (all in the add-on; 0 exe bytes):**
- *Enroll:* content.js draws a green **Watch** button on every row carrying the §15 "· protected" label ([content.js:813](../extension/content.js#L813) `wbtn.className = 'v-take v-watch'`); the click sends `{op:'retry', videoId, itag, filename, referer}` ([content.js:823](../extension/content.js#L823)). Opening the panel seeds which rows are already watched from `{op:'retry-status'}` ([content.js:888](../extension/content.js#L888)), so a re-open shows "Watching" rather than offering the row again.
- *Loop:* the worker keeps a job list in `chrome.storage.local` (`getJobs`/`setJobs`, [background.js:502](../extension/background.js#L502)), a `chrome.alarms` timer `sucr-yt-retry` every 5 min (`RETRY_PERIOD_MIN`, armRetry [background.js:627](../extension/background.js#L627)), and a 24 h give-up horizon (`RETRY_MAX_AGE_MS` [background.js:599](../extension/background.js#L599)). `runRetry` ([background.js:594](../extension/background.js#L594)) re-probes each job — on success it calls the **unchanged** `take()` and reports in the page; at the horizon it toasts and drops the job.
- *Permissions:* `alarms` and host `https://www.youtube.com/*` added to both manifests (the host is what lets the worker inject the player fetch into the open tab).

**The one design premise that was falsified, and retracted.** My first cut had the service worker make the ANDROID innertube `player` call itself, and I wrote a comment claiming it was readable there. Measured wrong: `scratch/realpath/yt-swladder.mjs` shows a cross-origin `/youtubei/v1/player` ANDROID fetch issued **from the SW** (Origin `chrome-extension://…`) returns **403** for both a gated and an ungated video — the worker carries none of the page's origin, referrer or session. The shipped code therefore re-derives the ladder by borrowing the page: `chrome.scripting.executeScript` runs the same fetch **inside the open watch tab** (same-origin → **200**, whole adaptive ladder with `url` + `contentLength`; [background.js:533](../extension/background.js#L533)), while the worker keeps only what does run from its own context. `predicate:` [background.js:516-528](../extension/background.js#L516) now states the 403-vs-200 split; the stale claim is gone.

**Why the Range probe still belongs to the worker.** That probe *is* the gate test and does run from the SW (`*.googlevideo.com` host permission). Proven split leg `scratch/realpath/yt-splitleg.mjs`: page-context player fetch → 200 ladder, then SW `Range: bytes=0-15` and `bytes=(clen−64 KiB)-…` on the returned URL discriminate — gated `hNJe9d__yQM` itag 137 (clen 415 701 491): **206 then 403** = still protected; ungated `dQw4w9WgXcQ` itag 313 (clen 358 608 461): **206 then 206** = open. A row is taken whole only when both ends answer 206 (`ytRowOpen` [background.js:580](../extension/background.js#L580); `YT_WALL`=12582912 decides whether the tail probe is even needed).

**Real-browser proofs — isolated profile, user library never opened, no cookie read into the worker.**
- `scratch/realpath/yt-retry-unit.mjs` calls the SHIPPED `ytRowOpen` in the worker against a live gated row and a dead tab → protected / open / `tab-gone` all correct. **PASS.**
- `scratch/realpath/yt-retry-ops.mjs` drives the three message ops + the alarm lifecycle: enroll stores exactly one job and arms `sucr-yt-retry`; `retry-status` reads the itag back; a `runRetry()` cycle with a live tab keeps the job and does not throw; `retry-cancel` empties storage and clears the alarm. **PASS.**
- *Harness lesson (recorded so it is not repeated):* a service worker cannot wake its own `onMessage` — `chrome.runtime.sendMessage` is never delivered back to the sender — so the first ops run returned `undefined` for every op. The ops must be driven from a **separate** extension context; `yt-retry-ops.mjs` sends from the add-on's popup page, which is a real sender.

**Honest limits.** (1) This is not a headless watchdog: the ladder can only be re-derived while an enrolled YouTube **tab is open** (that tab runs the player fetch). Close it → the next tick returns `tab-gone` and the job is kept until the tab returns or the 24 h horizon expires. (2) It is opportunistic — it fires when *Google* opens the window, never by defeating the gate; a row still gated at 24 h is dropped with a toast. (3) Enrollment is one deliberate click per row; it does not auto-watch every tier.

**Boundary held.** Nothing is forged and the URL handed to `take()` is the same self-signed ANDROID URL a normal Take already uses; this does not reopen §18. **Gate:** exe unchanged at 4 665 344 B (`stat`, not a hash); this section spent 0 exe bytes. `writer:` this agent. **Owed:** reload the Sucr Capture card to pick up the rebuilt `dist/chrome` (Watch button + retry loop are extension-only). The cookie+UA handoff §19 flagged as the one legit parity gap is **not** built — that is the user's call, not taken here.

## 21. Adopt the browser's own on-the-wire stream URL on Take — **SUPERSEDED by §23; the adopt was withdrawn** (2026-11-09)

The user's live complaint was that a download still stalled while a competing add-on completed the same file. Pinned by question to **a download that stops partway** with the card **reloaded and Sucr running** — the free-window wall (§8/§14/§17) on a gated adaptive row, not a stale card or broken build. §18 had concluded "there is no browser-side URL to adopt"; that conclusion was about `ytInitialPlayerResponse` (page JS), and it was **incomplete**: the attested URL is real, it just lives on the wire, not in the player response, and a commercial add-on reads it off the wire (`webRequest`, §19). This section corrects that and ships the legitimate version.

**What ships (all in the add-on; 0 exe bytes, reversible).** A read-only `chrome.webRequest.onBeforeRequest` listener ([background.js:279](../extension/background.js#L279), permission `webRequest` added to both manifests — non-blocking, observational) records every `*.googlevideo.com/videoplayback` request the page makes, keyed `${tabId}:${itag}` ([background.js:287](../extension/background.js#L287)), with the player's `range=` param stripped so Sucr can re-range the whole stream itself (`LIVE_TTL` 120 s, capped at 600 entries). `liveUrlFor(tabId, url)` ([background.js:300](../extension/background.js#L300)) returns that captured URL when the row being Taken is the **same** itag the browser is actively streaming. `offer()` now resolves `const effUrl = liveUrlFor(tabId, url) || url;` ([background.js:341](../extension/background.js#L341)) and probes/takes that URL instead of a freshly self-signed one ([background.js:343/348](../extension/background.js#L343)). When no live URL exists it falls through to the old path unchanged. `fromBrowser` (the browser-fetch-and-adopt leg) still uses the original URL.

**Boundary, explicitly.** This is **not** §18's forbidden route reopened and it is **not** attestation forgery. The URL handed to the engine is one the user's **own** browser already fetched with its **own** granted PO token, on the user's **own** IP — the same session-piggyback a commercial add-on performs (§19). Nothing is minted; we only reuse a URL that already exists in the user's traffic. §18's standing finding — that page JS exposes no reusable URL — remains true and is unaffected; the correction is only that the *wire* does, and that is what `webRequest` sees.

**Wiring proven on a real browser** (`scratch/realpath/yt-livecapture.mjs`, `writer:` this agent, headful Helium + the shipped `dist/chrome`, isolated temp profile, user library never opened). `predicate:` an independent `webRequest.onBeforeRequest` observer armed in the worker across a play+seek; the shipped `liveUrls` store read in the same pass.
- **webRequest DELIVERY (permission + URL pattern): WORKS.** 5 `googlevideo.com/videoplayback` requests reached the worker, `observed.err = none`. So the `webRequest` permission and the `https://*.googlevideo.com/videoplayback*` pattern are correctly wired and fire in a real engine.
- **this profile requested attested (itag-bearing) media: NO.** All 5 videoplayback requests carried **no `itag`**, and the production `liveUrls` store collected **0** entries (it correctly keys on `itag`). This is the bot-trim of a fresh automation profile already measured in §20 (`readyState=1`, every request `itag=undefined`): YouTube serves this profile only metadata/segment requests and never issues the itag-labelled adaptive request, so there is no attested URL here to adopt, and the adopt branch cannot fire.

**Consequence, stated honestly.** The capture *mechanism* is built and its delivery is proven on this box, but the *completed download* cannot be proven here: the only client on this machine that makes itag-bearing, PO-tokened stream requests is the user's **real, warm, signed-in** browser — and a measurement there would have test==control-vs-Sucr's-cookieless-engine, so it is the user's bench, not a harness I can run. **The single test for the user:** reload the Sucr Capture card; on a video that currently stops partway, **play it at the target quality** (e.g. select 1080p in the YouTube player and let it run), then **Take that same quality** from the Sucr panel. If the page is streaming that exact itag when Taken, Sucr adopts the browser's attested URL instead of minting its own.

**The one remaining parity gap, named — not built.** A mature client's engine fetches with the **browser's cookies + UA** (§19); Sucr's engine is cookieless with a fixed desktop Chrome UA ([engine/mod.rs:136](../app/src-tauri/src/engine/mod.rs#L136)). Adopting the URL transfers the right `pot`/`ip`/`sig` bindings but **not** the UA/cookie, so whether Sucr's re-ranged fetch of the adopted URL is accepted past the free window is unproven here. If it still truncates in the trusted session, the faithful next leg is forwarding cookies+UA to the engine — that **costs exe bytes**, touches the shared tree, and is the user's `DECIDED`, not taken here. **Gate:** exe unchanged at 4 665 344 B (`stat`, not a hash); this section spent 0 exe bytes. `writer:` this agent. **Owed:** reload the Sucr Capture card to pick up the rebuilt `dist/chrome` (the `webRequest` adopt path is extension-only).

## 22. The URL-adopt was not enough — route the live row to the *browser's* downloader — **SUPERSEDED by §23; play-then-take refuted it, code reverted** (2026-11-09)

After §21 shipped, the user retested: *"Tried to download the video. Still fails to completely download."* That is decisive evidence the §21 design was incomplete, and §21's own code shows why. `offer()` computed `effUrl = liveUrlFor(...) || url` and then handed that attested URL **back to Sucr's own `/probe` + engine `take`** ([background.js old 343–348](../extension/background.js)). But the PO-token gate is bound to the *client session*, not just the URL: Sucr's engine fetches cookieless with a fixed UA ([engine/mod.rs:136](../app/src-tauri/src/engine/mod.rs#L136)), so giving it the browser's attested URL still 403s past the free window — the URL travels, the session does not. The browser-download-and-adopt leg (`fromBrowser`, which uses Chrome's *own* cookies+UA via `chrome.downloads.download`) was only reached on a **probe refusal**, which these rows never cause because offset 0 always returns 206. So the gated row always took the engine path and stalled. **This is the §21 finding corrected: adopting the URL is useless unless the browser, not the engine, performs the fetch.**

**What ships (extension-only, 0 exe bytes, reversible).** `offer()` now asks the live store *first*: when the page is actively streaming this exact itag, it returns `fromBrowser({ url: live, …, filename })` ([background.js:355](../extension/background.js#L355)) — Chrome downloads the whole attested stream with its own session, then Sucr adopts the finished file via the existing `watchAdopt`→`adoptDone`→`/adopt` machinery. This is the session ride in its most direct form (§19: the browser is the one client already holding the credentials), done without touching the engine. The cookieless engine path is unchanged for every row the page is *not* currently streaming. A live miss never fabricates a URL. `fromBrowser` now also passes a **sanitized** filename ([background.js:246](../extension/background.js#L246) `safeDownloadName`) so the browser writes a properly-named `.mp4` for adoption, stripping Windows-illegal characters and any directory component.

**Routing proven inside the real service worker** (`scratch/realpath/yt-routing.mjs`, `writer:` this agent, headful Helium + shipped `dist/chrome`, **no Sucr app, no network, no library** — `chrome.downloads.download/search` and `openPanel` are stubbed, a fake live capture is injected because this box is bot-trimmed). `predicate:` the `checks` table is the evidence; **PASS.**
- `case1_routed_browser` true — a `take`-mode Take of itag 137 the page is "streaming" returns `{ok:true, pending:true, body:{mode:'adopt'}}` and calls `chrome.downloads.download`, i.e. the browser leg, **not** the engine.
- `case1_used_live_url` true — the downloaded URL is exactly the browser's attested LIVE URL (with its `pot`, no `range`), not Sucr's self-signed one.
- `case1_filename_present` / `case1_filename_legal` true — `"My: Video / 1080p? <fun>.mp4"` collapses to `"1080p   fun .mp4"`, no `<>:"/|?*` survive.
- `case2_miss_no_fabrication` true — with no live capture, the browser leg still uses the original link; nothing is invented.

**Honest limits (the byte-completion is the user's bench, again).**
1. The completed download still cannot be proven on this machine: this fresh profile is bot-trimmed (§21: 0 itag-bearing requests → `liveUrls` empty → `live` is null → the new branch never fires here, behaviour identical to before). Only the user's warm, signed-in browser produces a live capture, so it is the only place the fix engages. The single test is unchanged from §21 but now actually exercises the browser leg: reload the card, **play** the video at the target quality, then **Take that same quality while it plays** — Sucr will hand it to Chrome's downloader and adopt the result.
2. **Adaptive high-res rows are video-only.** Chrome downloading itag 137 yields a silent 1080p file — correct bytes, no audio — exactly as any adaptive-only path would. A single A/V-complete file comes from the **muxed/progressive** row (the grabber labels it "video + audio"; that row already completes even through the cookieless engine). Merging a video-only + audio-only pair is a separate feature and is **not** built.
3. If Chrome's own full GET of an attested URL still 403s past the free window on some video (Google can demand per-segment ranges even from the browser), the remaining faithful path is forwarding the browser's **cookies+UA to the engine** so its robust multi-range downloader rides the session (§19's one parity gap). That **costs exe bytes**, touches the shared tree, and is the user's `DECIDED` — not taken here. **Gate:** exe unchanged at 4 665 344 B (`stat`); this section spent 0 exe bytes. `writer:` this agent. **Owed:** reload the Sucr Capture card to pick up the rebuilt `dist/chrome` (the browser-session routing is extension-only).

## 23. Play-then-take refuted §22's browser-download-adopt — reverted (2026-11-09)

The user's live symptom is a download that **stops partway**. §21 (URL-adopt into the cookieless engine) failed it, so §22 routed a live row to Chrome's *own* downloader instead. The user asked me to run the play-then-take flow myself and report. I did, three times, in real headful Helium + the shipped `dist/chrome`, isolated temp profile, **no Sucr app, no library, no cookie read into the worker** (`scratch/realpath/yt-playtake.mjs`, `writer:` this agent).

**Measured, verbatim:**
- **dQw4w9WgXcQ** (a video whose gate is *open* right now): player `readyState 1, 70/213 s`. Live captures: **1**, and it was **itag 18** (muxed, `c=WEB`) — the bot-trimmed player's fallback, not an adaptive row. Engine cookieless ANDROID **137**: `first:206, tail:206 → open` (the whole 80 911 999 B answers 206 today). Browser-leg download of the captured attested URL: **0 bytes** — `SERVER_FORBIDDEN` on the first run, `NETWORK_FAILED` on the extended run. The cookieless-open attempt returned `no-record` (the poll window; the engine is separately proven 206/206).
- **hNJe9d__yQM** (gated): Live captures **0**. Engine cookieless ANDROID 137: `first:206, tail:403 → closed` — the free-window-then-403 that *is* the partway symptom.

**What this decides.** `chrome.downloads.download()` of a captured player URL is rejected by Google even **from the same browser** — the gate binds to the *player's request context* (a full GET with no per-segment `range`, `Sec-Fetch-*`/header set the downloader does not reproduce), not merely to the URL. So §22's premise — "let Chrome download the attested URL" — is **false**, and worse, when the §22 branch fires it replaces the engine's resumable free-window partial with a **0-byte browser failure**. §22 is therefore not a fix but a regression on exactly the rows it targeted.

**Retraction + revert (extension-only, 0 exe bytes).** `offer()` no longer asks the live store first; the `webRequest` listener, `liveUrls`, `LIVE_TTL`, `liveUrlFor()` and the live-first `fromBrowser` hijack are **deleted**, and the now-unused `webRequest` permission is removed from both source manifests. `offer()` is restored to the §20 path: `take`-mode probes, then the engine takes the grabber's own freshly-signed ANDROID URL, and the browser-adopt leg runs **only** on a genuine probe refusal. `dist/chrome` + `dist/firefox` rebuilt; a grep proves no `liveUrls`/`liveUrlFor`/`webRequest` survives in either. `predicate:` `stat` exe = 4 665 344 B — unchanged; §21+§22's adopt machinery is fully withdrawn, so its byte spend was also 0.

**The two paths that DO complete a gated video, on this evidence.**
1. **The muxed "video + audio" row** (progressive, itag ≤18): never gated, the cookieless engine pulls it whole — this is exactly the row a progressive-only format table (`§19`: itag 16/17/18/45/46) hands the user, which is why a legacy client appears to work flawlessly while an Sucr adaptive take stalls. For any video that stops partway, taking the muxed row is the parity answer.
2. **The per-moment Watch button (#233):** re-derives the ANDROID row and takes it the tick Google opens the window (two-range 206/206), pulling a protected row complete. Proven mechanism in §20; dQw4 137 measured `open` here is the live witness that such a take would finish now.

**The one legitimate path still owed, unchanged, and now the only remaining parity gap.** Forwarding the browser's **cookies + UA into the engine** (§19's gap) is the sole way to beat a *closed* gate on an adaptive row without waiting for the window; it rides the user's own already-granted session (no attestation forged). It **costs exe bytes**, touches the shared tree, and is the user's `DECIDED` — not taken here. `writer:` this agent. **Owed:** reload the Sucr Capture card to pick up the reverted `dist/chrome`.
