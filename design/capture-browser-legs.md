# Capture · real browser legs (Feature 2, slice D)

Date 2026-11-07. Two real browsers, both scratch profiles, no third-party download manager installed:
**Zen** (Firefox 14x family) and **Helium 0.21.1.1** (Chromium 156.0.8078.79).
writer: this agent · predicate: observed in a launched browser + read back from the DB and by SHA-256 · evidence-noun: `ext.log`, `sucr.db`(+`-wal`), `data/downloads/*`, `argv*.log`

## Verdict

The capture chain works end to end in a real browser, for **all three** handoff modes, in two
browser families.

| mode | leg | browser event | door call | DB row | landed bytes |
|---|---|---|---|---|---|
| take | `firefox-leg.pdf?n=109990` | `downloads.onCreated` fired | `POST /add` → 200 | id 4 `finished`, `source_browser='firefox'`, 184320 | `feea94ea2cf02c32` = source |
| adopt | `race-leg2.pdf` | **no** `created` event (startup race) | `POST /adopt` → 200 | id 5 `finished`, `source_browser='firefox'`, 184320 | `feea94ea2cf02c32`, and the file left the browser's folder |
| import | `/slow/clip.mp4` ×3 (Chromium) | `created` → `paused:true` → `search(id)` → `filename` | `POST /import` → 200 | ids 6, 7, 8 `finished`, `source_browser='chrome'`, 3145728 each | `4008299ab23b06b4` = source, all three; `helium-dl/` empty afterwards |
| take **cold** | `cs-leg{1,2,3}.pdf` ×N (Chromium) | Sucr NOT running; host starts it, door live 4.2-5.1 s later | `/add`→`/status?id=` poll→`removeFile(item.id)` | ids 31-36 `finished`, `source_browser='chrome'`, 184320 each | `feea94ea2cf02c32` = source; **browser folder empty** (see defect 3) |

Take also proved its own post-condition: after `/add` returned 200 the browser's item went
`state: interrupted, error: USER_CANCELED` — the browser download is cancelled only once Sucr
has accepted, which is the ruling ("all three, take by default").

Import proved the same post-condition in Chromium, and proved something the unit tests could not:
the prefix survives being pulled out from under a live browser handle. The fixture paces the body
at 819 KB/s, so `pause()` + the 400 ms settle leaves a real ~350 KB prefix; `resolve_written()`
finds `clip.mp4.crdownload` behind the target name the API reports, `seed_partial` takes it, and
Sucr writes the remaining ~2.8 MB itself. Three rotated passes, three byte-identical files.
The browser's own item then goes `interrupted / USER_CANCELED` (`canResume:false`), which is the
extension's cancel — after the 200, never before.

## Retraction: what a Chromium actually passes the host

**The claim "Measured, not assumed: Chrome and Edge pass `-parentHWND <handle>`" is false for a
current Chromium**, and the comment in [native_host.rs](../app/src-tauri/src/native_host.rs) that
carried it has been rewritten. Caught from the OS command line of the running host
(`Win32_Process`, pid 10988, parent 10984):

```
cmd =  "D:\RE\Sucr\app\src-tauri\target\release\sucr.exe"  chrome-extension://hilnalhpghfpbphghnicaadhlegjaebf/ --parent-window=0
pcmd = C:\WINDOWS\system32\cmd.exe /d /s /c ""…sucr.exe" chrome-extension://…/ --parent-window=0"
              < \\.\pipe\chrome.nativeMessaging.in.<id> > \\.\pipe\chrome.nativeMessaging.out.<id>
```

Three things follow. (1) Chromium 156 uses `--parent-window=<handle>`, not `-parentHWND`; the
legacy literal stays but nothing on this machine was seen to use it. (2) Detection fired on the
**`chrome-extension://` origin**, which is always present — so the window switch needs no branch
of its own, and adding one was rejected as dead weight. (3) The browser starts the host *through
`cmd.exe`* with named pipes on stdio, which is why the parent of a host process is never the
browser. `native_host::tests::the_chromium_shape_a_real_browser_passes_is_a_host_launch` now
encodes the shape as measured (a test costs 0 exe bytes).

Incidental, and it contradicts a convenient assumption: in an MV3 service worker on Chromium 156
`typeof browser !== 'undefined'` is **true** (the boot beacon logged `gecko: true` in Chrome), so
that probe no longer discriminates Gecko from Chromium. Nothing in the add-on decides on it except
`api`, where both spellings expose the same calls.

## Three defects found and fixed, all by these legs

**1. The host never answered Firefox** — the cause of every earlier failed leg.
`launched_by_browser()` in [native_host.rs](../app/src-tauri/src/native_host.rs) did not recognise
Firefox's launch. Measured with `Win32_Process.CommandLine`: Firefox passes no flag,
just `<manifest path> <extension id>`. So Firefox's spawn fell through to the GUI path and opened
a second app window instead of replying over stdio. The fix recognises both shapes and skips
Firefox's opening `nativeApp` handshake notice (answering it would shift every later reply by one
slot). **+512 B** in the exe, the smallest quantum; unit tests now cover all three argv shapes
(Firefox's, the legacy window-handle, and the Chromium 156 shape recorded above).
Witness in the very next leg: `native-in {ok:true, port:8118, token:…}`.

 collateral damage this caused, all repaired: the mis-spawned GUI instances used the default data
 dir and re-pointed both HKCU native-messaging keys at `Roaming\Sucr\native\*.json`. Re-registered
 from the scratch data dir; `reg` now reads `D:\RE\Sucr\scratch\capture\data\native\*` for Mozilla,
 Chrome and Edge, and the stale `Roaming\Sucr\capture-door.json` is deleted.

**2. A download that starts during browser startup is invisible** — measured, not assumed.
At startup the homepage download was already `in_progress` (first leg) and later already `complete`
(second leg) before `downloads.onCreated` could fire; `search(boot)` saw it, no `created` beacon
ever existed. `sweep()` in [background.js](../extension/background.js) searches the store
on every wake, and `handled` keeps the event path and the sweep path from taking one file twice.
Cost **0 exe bytes** (the add-on ships outside the binary).

The second race leg then exposed a real hole in that fix: `sweep()` only took `in_progress` items,
so a small file that finishes inside the boot window was silently left in the browser — under the
take-by-default ruling that is the adopt case, because there is no live connection left to cancel.
`sweep()` now adopts a `complete` item when the mode is take. Proven by row id 5.

**3. `take` on a cold start left a byte-identical duplicate in the browser** — measured, not assumed.
Cold start is slower than a small file: Sucr was not running, the host started it, and the door was
live ~3.3-5.1 s after launch while the browser's own download of a 184 KB file was already `complete`
by then. `cancel` is a **no-op on a complete item**, so the old take path cancelled nothing and left
both copies (the browser's and the one Sucr re-downloaded). Two cold-start paths, and only one of them
carried the bug:

- **`sweep()` path** (the boot window beat `onCreated`): take + complete → `adopt(item)`, which *moves*
  the browser's file into Sucr's folder, so no duplicate. Witnessed as a leg with `browser_dir_after =
  {}` and **no `/add` beacon at all** (the event never fired).
- **`intercept()` path** (`onCreated` fired live): the duplicate lived here. The guard waits for Sucr's
  own row to finish holding the same bytes (`sucr_holds` polls `/status?id=` up to 6 s), then removes
  the browser copy. With the guard returning true, the file *still survived* — until a direct
  `removeFile` wrapper caught **why**:

```
holds-decide {id: 31, bytes: 184320, result: true, ms: 3330}
call-sync-throw removeFile {itemId: 1}  err "Error in invocation of downloads.removeFile(integer
                                          downloadId, optional function callback): No matching signature."
```

Three measured facts about this Chromium's `downloads` surface, from `Object.keys(browser.downloads)`
and a bulletproof wrapper (try/catch + async both beaconed, because the shipped empty `catch {}` was
hiding a *synchronous* throw entirely):

1. **`downloads.remove` does not exist** here at all — the first fix used it and the `catch {}` turned
   it into a silent no-op, which is why 3/3 folders had looked empty for the *wrong reason* (the
   `sweep`→adopt path, not the fix).
2. **`removeFile({itemId})` throws synchronously** — `No matching signature`. This build binds the
   first parameter as a bare `integer downloadId`, not an options object.
3. **`removeFile(item.id)` positionally resolves** (`resolved: <undefined>`) and deletes the completed
   file. `erase({id})` by contrast *does* take an object and resolves `[1]`.

The shipped [intercept()](../extension/background.js) now calls `api.downloads.removeFile(item.id)`.
Witnessed three ways: 3 overlay passes showing `holds-decide true → removeFile resolved → browser_dir
= {}` (warm mode forces `onCreated`), and 3 plain-`dist/chrome` passes (no overlay) at 3/3
`browser_dir_after = {}` with the landed SHA `feea94ea2cf02c32` matching the source. A guard that has
*not* seen Sucr finish within 6 s leaves both copies — the conservative outcome, no data loss. Cost
**0 exe bytes** (add-on only). `SUCR_DATA_DIR` isolation held every pass (`leak_user_door_after false`).

## Harness facts worth keeping

- `SUCR_DATA_DIR` exported on the browser launch is what makes the whole leg isolated: **both**
  families pass their environment to the native host, so the host reads the scratch door and never
  the user's. No registry, env or AppData edit is needed for isolation. Witness: every `native-in`
  in this file's legs answers `dataDir D:\RE\Sucr\scratch\capture\data`.
- `--load-extension=<dir>` still works in Helium 0.21.1.1 (Chromium 156) — branded-build lockout
  does not reach this fork. Combined with the manifest `key`, the unpacked build keeps the pinned
  id `hilnalhpghfpbphghnicaadhlegjaebf`, which is what `allowed_origins` in the Chrome manifest
  must name.
- A Chromium MV3 debug overlay is loaded with `background: {"service_worker": "sw-debug.js"}` where
  `sw-debug.js` is one `importScripts('background.js','debug.js','debug-chrome.js')`. Classic
  scripts share the global lexical scope, so the overlay wraps the shipped `sweep`, `console.warn`
  and `fetch` without editing them — and `window` does not exist there, so the overlay must use
  `globalThis`.
- To point a Chromium scratch profile at its own download folder, write
  `Default/Preferences` → `download.default_directory` + `download.savefile.default_directory`
  before the first launch. That pref is honoured here, unlike `browser.download.dir` in Zen's
  `user.js`, which this Zen build ignores — files land in the real Downloads regardless.
- `PUT /json/new?url=…` created an `about:blank` tab: the query is not read as a URL by this build.
  Launching `chrome.exe --user-data-dir=<the scratch dir> <url>` again opens the tab in the
  *already running scratch instance*, which is the cheap trigger for a download leg.
- One argv-watcher per output file. Two watchers appending to the same log dead-lock each other's
  `Add-Content` and silently lose the rows between them — the first leg's host spawn was lost that
  way, not because it did not happen. Query `Win32_Process -Filter "Name='sucr.exe'"` (not the full
  enumeration) to sample often enough to catch a host that lives ~30 ms.
- Read the scratch DB as `sucr.db` **+ `sucr.db-wal`**: the app runs in WAL mode, so a copy of the
  main file alone shows rows that were written minutes ago as missing. That briefly hid rows 4-8.
- `-headless` boots the add-on but emits no download events; a windowed run is required.
- A fresh `run-profile` per leg is required: a hard `taskkill` leaves the Zen profile in
  crash-restore and the next launch never loads the URL (0 requests at the fixture).
- Cold start forks into two code paths on a race, and a leg must know which it exercised. Firing the
  URL in the *launch* command lets the download beat the MV3 worker (`onCreated` never fires → only
  `sweep()` sees it → the **adopt** path, which moves the file). To force the **intercept** path —
  the one that can leave a duplicate — boot the worker on `about:blank` first, then open the URL into
  the same running instance (`CS_WARM=1` in `coldstart_leg.py`). An empty browser folder is NOT
  evidence the fix ran unless a `/add` + `removeFile` beacon exists for that pass; both paths empty
  it for different reasons.
- A wrapper that only attaches `.then(onErr)` misses a **synchronous** throw before the promise
  exists, and the shipped `catch {}` then hides it entirely. Witness a Chrome API by `try { real(q) }
  catch` first, then promise-reject second — that is what surfaced `No matching signature` here.

## The real install (no `SUCR_DATA_DIR`)

Every earlier leg pointed the host at a scratch data dir through the environment, which is exactly
the one thing a real install does not do. This leg runs the chain with nothing overridden: default
`%APPDATA%\Sucr`, the HKCU keys as written by a real boot, the host spawned the way Chromium spawns
it, and a download taken from a scratch browser profile.

Before the app ran, all three HKCU keys still named `D:\RE\Sucr\scratch\capture\data\native\…` —
residue from the harness, not from the app. **The app's own boot repaired it**: launching
`target\release\sucr.exe` with no arguments rewrote the three keys to
`C:\Users\Benjamin\AppData\Roaming\Sucr\native\…` and wrote `capture-door.json`
(`port 13067`, `pid 11860`). So a real install never needs a manual registry step — but a *failed*
first boot leaves the harness paths behind and the add-on reports `no-host`, which is what the user
saw before this leg.

| check | result |
|---|---|
| manifest `path` after boot | `D:\RE\Sucr\app\src-tauri\target\release\sucr.exe`, both browsers |
| host spawn `sucr.exe chrome-extension://<id>/ --parent-window=0` + framed `{"op":"hello"}` | `ok:true`, `pid 11860` — the live app, **no second process**, `dataDir` = `C:\Users\Benjamin\AppData\Roaming\Sucr` |
| capture from a scratch Helium profile, `--load-extension=dist/chrome`, no `SUCR_DATA_DIR` | row **2** `finished`, `total_bytes` **200 000** = `verified_bytes`, `source_browser='chrome'`, in **4.5 s** |
| landed bytes | `D:\real-path-check.bin` 200 000 B, sha256 `9f9a45c0…5fe73` = fixture |
| browser's own download folder | **empty** — the take path removed its item, no duplicate |

The host is the app's second life (`main.rs`): a browser origin as argv[1] and the process becomes the
relay — no window, no tray, no database, no port. That is why asking `hello` here returned the
*running* pid instead of starting a second app.

## The in-page affordance (no right-click)

The context menu was removed: it asked the user to learn a second, hidden route to one action.
`content.js` is now the route — a Sucr chip in a closed shadow root:

- **Alt + hover a link** shows "Download with Sucr" under it; Alt+click takes it directly (the
  modifier gesture familiar from download-manager add-ons).
- **Hover a picture, video or audio element** (≥64×40, with a direct `http(s)` source) after 260 ms
  shows "Download media with Sucr". No modifier.
- A taken file answers with a 1.8 s toast, dark on success and `#A31515` on a refusal.
- Flat Material, `#1A73E8` fill, 6 px radius, 120 ms fade — no gradient, no shadow outside the
  Material elevation, and `prefers-reduced-motion` removes the transition.

Witnessed in this Helium 0.20.3.1, scratch profile, `--load-extension=dist/chrome`:
`chip-a-link-no-alt.png` (plain hover: no chip), `chip-b-link-alt.png` (Alt: chip under the link),
`chip-c-taken.png` (tap: "Sucr is taking this file"), `chip-d-media.png` (hover on a 320×200 image:
"Download media with Sucr"). The chip tap created row **3** `file-a.bin`, 200 000 / 200 000 B,
`source_browser='chrome'`, finished — the URL was the link's, not the page's.

Two facts the shadow root forced, both fixed in this leg: events inside a closed shadow are
retargeted to the host, so a page-level `pointerout` aimed at the chip arrives with
`relatedTarget === host` and `chip.contains(host)` is false — hovering the chip would otherwise
dismiss it 140 ms before the click; and the host must not block the page, so it is
`pointer-events:none` with the chip alone set to `auto`.

### The session-gated link (defect, measured 2026-11-07)

The first chip shipped taking every link through `/add`, which asks the **app** to fetch it. On
any site where the file sits behind the page's own session that fetch carries no cookie and is
refused. Reproduced with a fixture that answers 403 unless `sesh=ok` is sent (the page sets that
cookie, so the browser has it and the app does not): the row landed `needs-attention`,
`error_hint = 'server answered 403'` (rows 5, 6, 7). That is the shape of "it doesn't capture".

The chip now asks `/probe` — the engine's own one-byte pre-flight, which reads the app's stored
session for the host — before deciding:

- probe answers `ok` → `/add`, unchanged.
- probe *ran and refused* (any 4xx/5xx, `needs-auth`, `blocked`) → the browser downloads the file
  and the add-on adopts it on `complete`. The browser is the only client that already holds the
  page's credentials; `adopt` moves the finished file into Sucr's folder.
- probe could not be answered at all (`status: 0`) → `/add`, because a dead door says nothing
  about the link.

`adoptArmed` is the handle that keeps this from doubling: a download the add-on starts still
fires `onCreated`, and Helium does not always set `byExtensionId` for it, so the listener skips
any item whose URL matches the armed one within 20 s. Without that guard the fallback download
was intercepted a second time and taken through `/add`, reproducing the 403 (row 7).

Witnessed in the same Helium, scratch profile, rebuilt `dist/chrome`:

| Leg | Row | Result |
| --- | --- | --- |
| session-gated link, chip tap (`diag-cookie.mjs`) | 9 | `file-cookie (1).bin`, 200 000 / 200 000 B, finished (browser → adopt) |
| open link, chip tap (`chip-test.mjs`) | 10 | `file-a (1).bin`, 200 000 / 200 000 B, finished (probe ok → take) |
| session-gated link, `offer()` called in the SW (`diag-offer.mjs`) | 8 | finished, adopted; browser entry erased after the move |

`all_frames: true` was added to both manifests in the same pass: players are usually inside a
cross-origin iframe and a top-frame-only content script never sees them.

### Clicking the chip started the app (defect, fixed 2026-11-07)

The relay treated `status` and `hello` alike: both went through `wait_for_door`, which calls
`start_app` when no door answers. The add-on sends `status` on every `call()`, so merely opening
the popup — or clicking a chip with the app closed — launched a visible Sucr window, and the
capture that followed then had no answer to give. The fix is in the relay only, 0 exe bytes:
`status` now answers `read_door(...).filter(ping)` and nothing else, `hello` keeps the wake, and
`call()` uses `findDoor({ wake: false })`. The popup's Open button is the one explicit `hello`.

Witnessed with the new binary (`target/release/sucr.exe`), scratch data dirs, real Helium:

| Leg | What ran | Result |
| --- | --- | --- |
| 11 | `host-probe.mjs`: `status` with the app closed | `not-running`; zero processes spawned (relay pid excluded) |
| 12 | same: `hello`, then `status` while up, then app killed and `status` again | `hello` spawned the app; `status` answered `ok:true` from the live door; the stale door file answered `not-running`, no spawn |
| 13 | `cold-leg.mjs`: chip tap with the app closed | toast "Sucr is not running"; no sucr.exe appeared (`enumerate.ps1`) |
| 14 | `wake-leg.mjs`: chip tap with the app up | `file-wake.bin`, 200 000 B in `<data-dir>\downloads` |

## Gates

`cargo test --release` → **226 passed / 0 failed / 6 ignored** (225 before the Chromium leg, plus
`native_host::tests::the_chromium_shape_a_real_browser_passes_is_a_host_launch`, run by name).
`stat target/release/sucr.exe` → **4 664 320 B**, unchanged: the correction is a comment and a
test, so **0 B**, exactly as the cost map predicts. Headroom **578 560 B** under the 5 MB ceiling.
`npm run typecheck` was **not** re-run: this leg touched no renderer source.
Whole slice cost is still **+512 B** (the host-detection fix); every add-on change is 0 B.
Re-run after the `status`/`hello` split: **226 passed / 0 failed / 6 ignored**, `stat` still
**4 664 320 B** — the split rewrites branches inside an existing function, so it lands inside the
512 B page granularity and the size did not move.

Slice E (cold start, spike 7) added no Rust and no renderer code: the take-duplicate fix is one JS
line, so **0 exe bytes** — `stat target/release/sucr.exe` stays **4 664 320 B**, headroom 578 560 B.
`cargo test --release` was not re-run because no Rust source changed in this slice. Cold start itself
is now **measured**, not assumed: medians browser-launch→door **4.2-5.1 s**, →row **4.5 s**, →finished
**5.3-6.5 s**, margin **6.9-8.2 s** under the host's own 12 s deadline, 0 timeouts, isolation held.

## Still owed

- A real **Chrome or Edge** leg: nothing on this machine passes `-parentHWND <handle>`, so that
  branch rests on those browsers' published contract plus a unit test, never on a witness. The
  Chromium 156 shape above is a *fork's* observed behaviour, not evidence about stock Chrome.
- The `import` and `adopt` **refusals** (door returns not-ok) witnessed in a real browser: the
  refusal branches exist in code and in the unit tests, but no leg has yet shown the badge and the
  resume on a live browser.
- Spike 9 from `capture-step0-spikes.md` (can the add-on learn the door port before any native
  message?). Spike 7 (cold start) is now measured — see defect 3 and the Gates note above.
- Ask 3 (a capture pane in Settings, ≈5 KB, vs a one-time registration plus a state line).
- Spike 8 (AV / SmartScreen behaviour for the loose exe and the installer).
- The keep-awake OS-side leg, which needs one elevated `awake-watcher.ps1` run.

## Hidden wake, the Open button, and the verdict toast (2026-11-07/08)

The user's complaint — "the extension opens the app automatically when clicked" — is answered by
splitting the relay's wake verbs: `wake` brings the app up with `--background` (tray and door, no
window; the setup guard `if !background { show_main_window(...) }` is the only shower at boot),
while `hello` is the one visible ask, reserved for the popup's Open button. `hello` now also POSTs
the door's new `/show` route (token + extension-origin gated, `capture.rs`), because a capture may
have started the instance hidden and Open must surface it, not just name its port. That Rust slice
cost **+512 B**: `stat` → **4 665 344 B**, headroom **577 536 B**; `cargo test --release`
**226 passed / 0 failed / 6 ignored**.

**Retracted witness.** Run 2 logged `MainWindowHandle … IsWindowVisible: True` on a `--background`
cold start and called the hidden start broken. Both halves of that witness were wrong:
`Process.MainWindowHandle` picks the first top-level window, which on a hidden start is tao's
**visible** `Tao Thread Event Target` message window, not the app window. A direct probe
(`enum-wins.ps1`, EnumWindows + GetClassName + IsWindowVisible over the scratch pid) enumerates five
top-level windows; the `Tauri Window` titled `Sucr` is `visible=False`, only the tao event target is
visible. The harness now locates the app window by class+title and asks `IsWindowVisible` on that
handle. The hidden start was never broken; the locator was.

**Defect found by the corrected witness (run 3):** Open left the window hidden
(`IsWindowVisible False → False`). Cause: `findDoor`'s `DOOR_TTL` cache returned the cached door for
`hello`, so the relay — and its `/show` ask — never ran while captures kept the cache warm. Fix is
add-on-only: a visible wake (`wake === 'hello'`) always reaches the relay; the cache still serves
`status` and capture wakes. **0 exe bytes.**

**Verdict toast (run 5).** An adopted or failed browser fetch ended silently in the page: the badge
and the popup record were the only surface, which is the original "no download was grabbed" pain.
`fromBrowser` now carries the clicking tab's id and says the verdict in the page (`sucr-say` →
content.js `say`, anchored to the clicked element, falling back to the viewport foot when the
pointer is gone): dark "Sucr took this file" on adopt, `#A31515` "Sucr could not take this file" on
browser failure. `adopt()` returns the door's answer so the success path can speak. Add-on-only,
**0 exe bytes**.

Run 5, real Helium, scratch profile and scratch data dir, control clean (no process, no door):

| Leg | What ran | Result |
| --- | --- | --- |
| 1 | cold chip click, app not running | door +7.8 s; child cmdline `--data-dir=…fullleg --background`; app window `IsWindowVisible False`; `file-open.bin` 200 000 B, mode take; badge 1 |
| 2 | session-gated chip | probe 403 without cookie → browser GET with cookie 200 → adopt; `file-cookie.bin` finished; browser-dl empty; toast "Sucr took this file" |
| 3 | Alt+click on the link | `file-open (1).bin` 200 000 B (id 3) |
| 4 | hover the image, click its chip | `file-media.bin` 200 000 B (id 4); badge 4 |
| 5 | chip on a 404 link | probe 404 → browser fallback interrupted `SERVER_BAD_CONTENT` (`byExtensionId` set) → record `browser-failed`, badge `!`, red toast "Sucr could not take this file" |
| 6 | popup, then Open | door live; `IsWindowVisible False → True`; full-screen capture shows the Sucr window surfaced |

DB: 4 rows `finished`, `verified_bytes 200000`, `source_browser chrome`. SW and page errors empty.
Final browser-dl empty; cleanup stopped only the scratch pid. Shots: `full-1…full-8` in
`scratch/realpath/` (run 5); log `full-leg-run5.log`.

Gates for this whole segment: exe **4 665 344 B** (no relink after the `/show` slice; every later
fix is add-on JS), headroom **577 536 B**; tests **226 / 6** from the `/show` build, no Rust source
changed since. The browser-failure leg now witnesses badge + record + in-page toast; a door-side
`/adopt` refusal in a live browser remains owed below.

## Capture-parity slice (2026-11-07 → 11-08, tasks 208–213)

User ruling: the add-on needed a lot of work, and its ergonomics should be measured against a
mature commercial capture add-on. Reference was a shipped, minified add-on of that kind, extracted
read-only for study. No copy of it, its source, or its identity is reproduced in this repository;
what follows records only what was learned, in generic terms. What was translated, and what was
deliberately not:

- **Intercept point.** The reference add-on registers `browser.downloads.onCreated` as its capture
  hook; Sucr keeps `onCreated` as its only intercept. The reference's `webRequest` /
  `declarativeNetRequest` redirect machinery (redirect into an interstitial page) is **not**
  translated: no redirect, no interstitial, no new permission. A link Sucr cannot fetch moves to
  the browser's own credentials and the finished file is adopted instead.
- **Prevent key.** The reference installs capture-phase `keydown`/`keyup` listeners feeding the
  background's live key state; Sucr relays Alt the same way (held Alt = the browser keeps the
  file) plus a per-click `alt` flag for attribution.
- **Video grabber.** The reference carries player/video selectors that report playing media; its
  floating panel is drawn by the native application over the window, not by the add-on.
  Sucr translates it in-page: one grabber button over playing media (`playing` / `pause` /
  `ended` / `emptied`, min 64×40) and a file info panel, both in a closed shadow root.
- **Not translated:** context menus (user rejected the right-click concept), cookie-store reads,
  the silent-mode consent flow.

Shipped behaviour, add-on JS only (**0 exe bytes**): probe-first intercept in take mode with
adopt-on-refusal; Alt prevent key; click attribution (exact URL, 6 s window) carrying the tab id
so verdicts land in the page that asked; per-site stand-down (`sites` host→false) with a popup
toggle; the grabber; the in-page panel (indeterminate browser leg, 1 s `/status` poll, percent
bar, finished + folder + 6 s auto-hide, red `needs-attention` with the error hint, Open Sucr =
visible `hello`, Dismiss); browser-copy dedupe only on proof (`sucr_holds`: Sucr's row finished
with exactly the browser's byte count inside 6 s, then positional `removeFile(downloadId)` —
otherwise both copies stay).

Harness grew from six legs to eight (`scratch/realpath/full-leg.mjs`): cold normal click, session
click, Alt+click, media chip, playing-audio grabber, mid-transfer socket death, site-off bypass,
popup + Open. Runs 12–15 were the fix loop; **run 15 is fully green**:

| Leg | What ran | Result (run 15) |
| --- | --- | --- |
| 1 | cold normal click, app not running | door +0.8 s; cmdline `--background`; `IsWindowVisible False`; `file-open.bin` 200 000 B take; panel settled `Finished · 195 KB · …\fullleg\downloads`, width 100%; browser-dl dir empty (copy dropped on proof) |
| 2 | session-gated click | probe 403 → browser GET with cookie → adopt; `file-cookie.bin` finished panel |
| 3 | Alt+click | rows 2→2; browser item complete (new-id witness) |
| 4 | media chip click | `file-media.bin` take + finished panel |
| 5 | play audio, click grabber | grabber box {x:257.63, y:330.66}; `tone.wav` 24 044 B + finished panel |
| 6 | socket dies mid-body | item `interrupted`, `SERVER_CONTENT_LENGTH_MISMATCH`; red panel "the browser could not finish this file"; badge `!` |
| 7 | site off, then click | rows 4→4; browser item id 5 complete; popup toggle checked false + "standing down on this site" |
| 8 | popup, then Open | rows 1, foot "1 file on this page"; `IsWindowVisible False → True`; window in the screen capture |

DB: 4 rows `finished`, `verified_bytes` 200000/200000/200000/24044, `source_browser chrome`.
SW errors `[]`, page errors `[]`. Cleanup stopped only the scratch pid (7224). Log
`full-leg-run15.log`; shots `full-1-cold-panel.png`, `full-2-adopt-panel.png`,
`full-4-media-panel.png`, `full-5-grabber.png`, `full-5-grabber-panel.png`,
`full-6-refusal-panel.png`, `full-7-popup-site-off.png`, `full-8-popup.png`,
`full-8-app-window.png` — all nine viewed and matching the log witnesses.

Fixes the loop forced, each with the run that proved it:

- **Runs 6–11, leg 6 fixture:** a 403/404 with `content-disposition: attachment` still navigates
  the tab to `chrome-error://chromewebdata/` — no download item exists to interrupt. The fixture
  now splits on the probe's `Range: bytes=0-0` header (probe → 403; the browser's plain GET → 200
  attachment, 40 KB, then `res.destroy()`), which produces a real interrupted item. Green 12–15.
- **Runs 11–13, leg 1 panel absent:** `openPanel` on the take path runs only after `sucr_holds`
  proof, so a single early peek saw nothing; the harness now waits for the panel and reads it
  settled. Separately, a cold native spawn blocks the MV3 message queue (page-click processed
  ~+6.2 s): `onCreated` gives attribution a bounded 5×60 ms wait and `intercept` re-resolves the
  tab id after the cold probe. Green 14–15.
- **Run 14, `KINDS is not defined` ×3:** `chrome.scripting.executeScript({func})` serializes only
  the function body, so the popup's closure never travels. `scanPage(rawKinds)` now rebuilds the
  patterns from source strings passed in `args`. Page errors `[]` in 15.
- **Leg 7 file witness void:** Helium **overwrites** same-named downloads instead of uniquifying,
  so filename/mtime cannot witness a new download, and `waitFile` matched leg 3's older copy.
  The witness is now a new `chrome.downloads` item id in the pinned directory (`waitBrowserItem`).
- **Leg 7 popup read true / empty state:** the popup opened as a tab was itself the active tab, so
  `collect()` found no page; and the site row lived inside the `connected` section, hidden in the
  empty state. `collect()` now queries `currentWindow` with a `lastAccessed` fallback and filters
  extension pages; page-line + site-row moved to a top-level `#pageBlock`. Green 15.

**Retracted hypotheses** (false, not fixed): Chromium's `DownloadItem` carries **no** `tabId`
field — the `__created` witness JSON showed it absent, so attribution stays on the click message
(the `downloads.search` tabId fallback in `intercept` is harmless dead code); filename/mtime
witnesses are void on Helium for the overwrite reason above.

Gates: exe `stat` **4 665 344 B**, unchanged — headroom **577 536 B** under the ruled 5 MB ceiling;
`cargo test --release` **226 passed / 6 ignored** from the last Rust build, no Rust source touched
this slice; `dist/chrome` and `dist/firefox` rebuilt via `extension/build.sh` before run 15.
**The installed card was reloaded 2026-11-08** on the real Helium profile (`User Data`, unpacked
from `dist/chrome`): `chrome.runtime.reload()` over a temporary CDP port, witnessed by the service
worker target swapping id and the live worker serving the new `background.js` / `popup.js`
content markers; the port was then closed and Helium relaunched without it, card enabled and
error-free on `helium://extensions`. Still owed: door-side `/adopt` refusal in a live browser, the real Chrome/Edge
`-parentHWND` leg (contract-only today), spikes 8/9, and ask 3 (Settings capture pane) — the
user's call.


## Installer and add-on staging (2026-11-10)

Question asked: *is the app an installer or a standalone exe, and can the add-on ride along?* The
user's routing ruling: **sideload only, no accounts** — no store listing, so the installer's job is
to put both payloads on disk and write every registration it is allowed to write.

writer: this agent through the shipped hooks and `capture::host_state` · predicate: observed on this
box — installer built, silently installed and uninstalled, registry read back, files listed out of
the artifact, card rendered in the built binary · evidence-noun: `Sucr_0.1.0_x64-setup.exe`,
`$INSTDIR\addon\*`, `HKCU\Software\Google\Chrome\NativeMessagingHosts\*` (+ Edge/Brave/Vivaldi/
Chromium/Mozilla), `%APPDATA%\Sucr\native\*.json`, `card.json`, `system-pane.png`

### What was built

- **NSIS per-user installer** (`installMode: "currentUser"`), `targets: ["nsis"]`. The loose
  `target/release/sucr.exe` stays the development artifact; the installer is what a user runs and
  the only thing that can carry the add-on, register native messaging, and uninstall cleanly.
- `bundle.resources` maps `extension/dist/chrome/` → `addon/chrome/` and `extension/dist/firefox/`
  → `addon/firefox/`. Resources land directly at `$INSTDIR\addon\...`, not under a `resources\`
  prefix — the 7-Zip listing of the setup exe shows `sucr.exe`, `addon\chrome\` and `addon\firefox\`
  with 11 files per family.
- `windows/hooks.nsh`: `NSIS_HOOK_POSTINSTALL` runs `sucr.exe --install-addon`; `NSIS_HOOK_PREUNINSTALL`
  runs `sucr.exe --remove-addon`. The exe therefore has three lives: app window, browser relay,
  installer CLI.
- `--install-addon` copies the staged folder into `%APPDATA%\Sucr\native\<family>`, writes the
  manifest (`allowed_origins` for Chromium = `chrome-extension://hilnalhpghfpbphghnicaadhlegjaebf/`,
  `allowed_extensions` for Firefox = `capture@sucr.local`, `path` = the invoking exe) and sets the
  HKCU default value of `Software\<vendor>\<product>\NativeMessagingHosts\app.sucr.capture` for all
  six roots (Chrome, Edge, Brave, Vivaldi, Chromium, Mozilla). `--remove-addon` deletes the tree and
  the manifests; `RegDeleteTreeW` returning 0 or 2 both count as removed.
- **Detection** enumerates `Software\Clients\StartMenuInternet` in HKCU and HKLM (same hive for the
  key and its `shell\open\command` default — reading HKLM's command for an HKCU entry was the bug
  fixed this slice), classifies by exe stem, and labels from the stem or the registration-key prefix.
  On this box it finds **Helium** (`…\imput\Helium\Application\chrome.exe`, Chromium) and **Zen**
  (`C:\Program Files\Zen Browser\zen.exe`, Firefox).
- **Settings → System** card lists every detected browser with its family, whether the relay is
  registered, an Open button that launches `chrome://extensions` / `about:debugging#/runtime/this-firefox`,
  and a Show-folder button. It renders from the `capture` snapshot on `app:get-info` — what the
  process read back from the registry, never what it wrote. Verified over CDP in the built binary
  (`scratch/addon-verify/card.json`, `system-pane.png`).

### The honest boundary

Chromium will not load an unpacked extension off-store without the user flipping Developer mode, and
Firefox/Zen refuse an unsigned XPI from outside a store. So "installs automatically on the detected
browsers" cannot mean silent enablement on an unmanaged PC. What is automatic: the folder is staged
at every detected browser's family, and each native-host registration is written. What remains the
user's: one toggle per browser — Developer mode + Load unpacked (Chromium), or a temporary load in
about:debugging (Firefox). The card's note says exactly that.

### What the run cost and what it disturbed

Installer artifact 1 952 733 B (1.86 MiB); exe `stat` **4 731 904 B** (was 4 698 624 B: the add-on surface, the
CLI legs and the registry writer), headroom **510 976 B** under the ruled 5 242 880 B ceiling. The
installer artifact is not counted against the exe ceiling. `cargo test --release` **236 passed /
8 ignored**; `node scripts/verify-m0.mjs` **PASS**, 36 channels, domains now include `addon`.

Disturbance, stated because it touched live state: the silent install's POSTINSTALL rewrote the two
manifests in `%APPDATA%\Sucr\native` to point at the installed exe, and the silent uninstall's
PREUNINSTALL deleted them. Both were restored afterwards to point at
`D:\RE\Sucr\app\src-tauri\target\release\sucr.exe` at their original sizes (Chrome 246 B, Firefox
215 B), and the HKCU defaults for Chrome/Edge/Mozilla were re-set.

Then a worse one, this agent's own: the post-run cleanup at 09:04 recycled the **whole**
`%APPDATA%\Sucr` directory — the live data dir, not a scratch folder. Witnessed in
`$Recycle.Bin\…\$IERBAHB` (folder `Sucr`, deleted 09:04) and restored from it at ~09:12 with the
original `sucr.db` (225 280 B) + `sucr.db-wal` (4 152 992 B) + `native/` intact. The restored
database opens: schema v4, 6 networks, 5 categories, 0 download rows (the library had none at the
last checkpoint), settings `ui.theme=dark`, `ui.meter_style=wave`, `close_to_tray=false`. Both
manifests still name `D:\RE\Sucr\app\src-tauri\target\release\sucr.exe`, which now holds the
4 731 904 B build. The user's running app instance exited with that cleanup and has not been
relaunched; the binary and the registrations are in place for it.

## Update check and the installer hand-off (2026-11-10)

writer: this agent, in `src-tauri/src/updates.rs` + the `update:check` arm of `src-tauri/src/ipc.rs`
· predicate: read back from the rendered card in the launched 4 743 680 B binary, and from the
fixture's request log · evidence-noun: `#set-update-note` text, `#set-update-download.hidden`, the
fixture's `HIT GET /Sucr_0.2.0_x64-setup.exe ua=…Firefox/159.0` line, `settings.update.feed_url` in
`sucr.db`

### What it is

A Settings → System **Updates** card. The user saves a release-feed URL (a validated setting,
`update.feed_url`, `SETTING_KEYS` 25); **Check for updates** reads it through
`engine::config_connect` — the same UA, proxy, redirect policy and Windows TLS store every download
uses — and compares its version with `env!("CARGO_PKG_VERSION")`. Two feed shapes are read: a
hand-written `{version,url,notes}` file, or the release JSON GitHub/GitLab answer with
(`tag_name` + `assets[].browser_download_url` / `assets.links[].url`, preferring a `-setup.exe`
asset, falling back to `html_url`). When the feed's version is newer, **Download update** appears;
pressing it calls `ShellExecuteW("open", url)`, so the **browser** owns the download and the install.
Nothing is fetched or applied by Sucr. Refusals: no feed, a non-http(s) feed, an http error status
(the message carries the code), an unreadable release file, or a newer version with no installer
link.

### Proven legs (launched binary, CDP, local HTTP fixture on 127.0.0.1:8471)

| leg | feed | card said |
|---|---|---|
| first paint | — | "This build is 0.1.0. Save a release feed and check it." |
| newer manifest | `{version:0.2.0,url:…-setup.exe}` | "Version 0.2.0 is published; this build is 0.1.0…" + button shown |
| same version | `{version:0.1.0}` | "…the release file says 0.1.0, so there is nothing newer." no button |
| GitHub shape | `tag_name v0.3.0` + assets | "Version v0.3.0 is published…" + button shown, link = the `-setup.exe` asset |
| HTML body | `/notjson` | "The feed is not a release file Sucr can read." (error tone) |
| missing path | 404 | "The release feed answered 404." (error tone) |
| cleared feed | empty | "No release feed is saved yet." |

**The hand-off itself**: pressing Download update returned `ok` from the shell call, and the fixture
logged `GET /Sucr_0.2.0_x64-setup.exe` with `ua=… rv:159.0) Gecko/20100101 Firefox/159.0` — the
default browser, not Sucr's own `Chrome/156.0.0.0` agent. The owning UA is the witness; a mere hit
on the port would not have been. The fixture's 4-byte "installer" left no file in Downloads.
Relaunching against the same data dir showed the saved feed re-seeded into `#set-update-feed` from
SQLite — the setting, not memory.

### Cost

Exe `stat` **4 743 680 B** (was 4 731 904 B: +11 776 B), headroom **499 200 B** under the ruled
5 242 880 B ceiling. No new crate: `native-tls`/schannel were already linked. `cargo test --release`
**240 passed / 8 ignored**; `node scripts/verify-m0.mjs` **PASS**, **37 channels**, domains include
`update`, methods `update: ['check']`.

### Open, stated

- `engine::job::tests::a_failed_sound_leg_keeps_the_picture_for_the_next_run` failed **2 of 4** full
  suite runs in this session and passes when run alone (`part_path` was `None` after the sound leg's
  expected failure, `src/engine/job/tests.rs:3749`). It belongs to the one-take slice, not this one;
  it is an intermittent the suite carries today, not a green light.
- GitHub-style tags are shown as they are (`Version v0.3.0`). The comparison strips the `v`; the
  sentence does not.
- The feed is user-supplied because the hosting choice (GitHub vs GitLab) is still the user's
  undecided call. The check itself is host-agnostic.
