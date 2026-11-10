# Browser Capture — Design Draft (Chrome/Edge and Firefox)

* **Date:** 2026-11-06
* **Status:** draft, for review. Builds on [browser-bridge-concepts_brief.md](browser-bridge-concepts_brief.md); that brief's analysis stands except where § 0 retracts it.
* **Scope:** design only. No product byte was spent writing this, and nothing here is implemented.
* **Naming, fixed here before anything else can be read:** **handoff listener** = the app-side door. **Browser add-on** = the extension. **Bridge** is retired in this document — in this repo the word already means the Tauri IPC boundary, the link-check refusal flow, and an unrelated Python tool at `D:\RE\Sucr\bridge\` that must never be run.

## 0 What this draft changes about the brief

- **Blocker 5's number is stale, and its conclusion survives for a different reason.** The brief said "two added IPC channels exceed the entire 20,684 B headroom at the gate's own recorded ~11 KB per command". The ceiling was RULED to 5 MB on 2026-11-04; headroom is now **649 728 B** at exe **4 593 152 B** (`stat`, corrected 2026-11-07 — this draft first quoted 663 040 B at 4 579 840 B, which the byte-sample slice's +13 824 B superseded), and the two real precedents for a new channel are +12 800 B (`proxy:set-auth`) and +27 648 B (`origin:set-auth`) — so the margin *can* pay. Zero-new-channels stays, but as a **security** constraint, not a budget one: the window-only `sucr_invoke` dispatch exposes `app:quit` and `settings:set`, and must never be reachable from a socket.
- **Three capabilities the brief could not assume now exist**, and this design leans on all three:
  1. A refused resume **names its cause** and ranks the validators `etag > last-modified` ([engine/job.rs:768-777](../../src-tauri/src/engine/job.rs), `"validatorKind"` at `:808`, shipped 2026-11-06 for +512 B). That is exactly the check an *imported browser partial* has to survive — see § 4 C.
  2. Bytes that no adapter carried are booked to a separate `unattributed_usage` ledger (schema **v4**, 17 tables, +3 584 B). A local socket is an unattributable client, and § 7 can therefore be honest about it.
  3. Origin credentials per host exist in two kinds (Basic and bearer/header-secret), so "this link needs a session" has an app-side answer that is not a cookie read.
- **The probe's latency budget is now measured, not estimated.** `probe_link` deliberately uses a **5 s** connect bound ([ipc.rs:941](../../src-tauri/src/ipc.rs)) because it runs while the user is still typing; the engine's per-read bound is a literal 4 s (`engine/mod.rs:203`, proven in the settings-coverage slice). The brief's "a decision can take on the order of a minute" is the *worst case across 10 redirects*, and it is why § 4 puts the token at `/add`, not at `/ping`.

## 1 The asymmetry that decides the shape

Every claim in this table was read from an official document on 2026-11-06; the URLs are at the bottom. Chrome and Edge are one platform for our purposes except in the registry path.

| Axis | Chrome / Edge (MV3) | Firefox (MV3) | Why it decides anything |
|---|---|---|---|
| Background runtime | Service worker; an event resets a 30 s idle timer, 5 min cap per event | `background.scripts`; Firefox **ignores** `background.service_worker` (bug 1573659 open) | One manifest must carry both keys, and the Chrome side cannot hold state across the transfer |
| Refuse a request **before** it starts | No — blocking `webRequest` is removed in MV3 (retained only for policy-installed extensions); `declarativeNetRequest` replaces it and *can* redirect a download URL, but it fires *instead of* a fetch from a rules table, not *after a user clicked*, and its rule-count limits are a separate question this draft does not need | Yes — `webRequestBlocking` + `"blocking"` in `extraInfoSpec` still works, and the listener returns a `BlockingResponse` | **On Chrome the browser always wins the race to begin. On Firefox the add-on can refuse before byte zero.** |
| Take over an in-flight download | `downloads.onCreated` + `downloads.cancel()`; `onDeterminingFilename` exists ("extensions will be given the opportunity to override the target `DownloadItem.filename`") | `browser.downloads` + `cancel()`; **`onDeterminingFilename` is not implemented** (bug 1245652, open since 2016) | The destination path is choosable on Chrome and *not* on Firefox |
| Native host registration (Windows) | `HKCU\Software\Google\Chrome\NativeMessagingHosts\<name>` (HKLM also read); Edge: `…\Microsoft\Edge\NativeMessagingHosts\<name>` | `HKCU\Software\Mozilla\NativeMessagingHosts\<name>` (HKLM also) | Three keys, and nothing in this app owns any of them — § 6 |
| Manifest allow-list | `allowed_origins`, values `chrome-extension://<id>/` | `allowed_extensions`, values `<id>@<author>` from `browser_specific_settings.gecko.id` | Two different trust vocabularies for one host manifest file — **two files**, or one generated per browser |
| Framing / limits | JSON, UTF-8, **32-bit message length in native byte order**, `"type": "stdio"`; host→browser **1 MB**, browser→host **64 MiB** | Identical framing; host→browser **1 MB**, browser→host **4 GB** | Every message we need is tiny; the limits are not a design driver, but the asymmetry is |
| Host process lifetime | `connectNative` = one host process per open port, kept running; `sendNativeMessage` = a new process per message | same API pair | The wake path must use `connectNative` or it forks a process per click |
| Self-distribution | A Web Store listing is effectively required for ordinary users; self-hosting is only via enterprise policy (`ExtensionInstallForcelist` / `ExtensionSettings`), which needs domain join or MDM; unpacked loading is developer-mode only | **Self-hosting works**: an *unlisted* AMO-signed XPI, free, with automated + human review, needs an addons.mozilla.org account; unsigned only in Dev Edition / Nightly / ESR | The add-on is a **published artifact with its own release cadence**, and Chrome's is slower than ours — hence additive-only contracts |

**The one sentence that follows from all of it:** the protocol is one thing, the take mechanics are two, and the invariant that keeps them a single product is **nothing is cancelled until Sucr has said it took the file.**

## 2 Product shape: what "capture" is allowed to mean

Sucr's promise is *accounted-for bytes on a bonded link*. A capture feature that only re-writes where a file lands would spend the app's whole machine on someone else's single stream. So the design keeps three modes and refuses a fourth:

- **Take** — Sucr carries the transfer (multi-stream, ramped, capped, resumable, attributed).
- **Adopt** — the browser carries it and Sucr only imports the finished file into the library, so the row exists, the category/folder rules apply, and the digest check still runs.
- **Decline** — say 422 and let the browser keep it, silently and correctly.
- **Not in scope:** sniffing media segments (HLS/DASH), which is a different product and was already a non-goal.

## 3 The door

Two artifacts, one contract:

1. **The add-on** (per browser) — observes, decides nothing on its own, and speaks to (a) the native host to *wake and discover*, (b) the loopback listener to *hand off*.
2. **The handoff listener inside `sucr.exe`** — the only thing that can say "I took it".

Why not native messaging alone: the host the browser spawns is a **second process**. There is no single-instance guard of any kind today (`grep` for `single-instance`/`deep-link`/`global-shortcut` over `src/` and `Cargo.toml` → 0 matches), and data-dir resolution keys off `--data-dir=` → `SUCR_DATA_DIR` → platform default ([paths.rs:14-27](../../src-tauri/src/paths.rs)), so a browser-spawned host can open a **different database**, a second tray icon, and a contended port. That is blocker 2 of the brief, still true, and it is **the first thing to fix**.

Why not the listener alone: nothing starts the app, and the add-on cannot learn the port.

**Chosen shape (CHOSEN, reversible, not `DECIDED`):**
- The app binds a **random ephemeral port on 127.0.0.1 only**, and publishes it — together with a token seed and the protocol range — over **native messaging**, which is the channel the *browser itself* authenticates. A fixed port range is squat-able and would let any local process forge "the app is running".
- The native host is a thin relay, not an intake path: it either finds the live listener or starts the app and reports what came up. `socket2` is already a direct dependency ([Cargo.toml:31](../../src-tauri/Cargo.toml) `socket2 = "0.6"`), so the single-instance lock is a `SO_EXCLUSIVEADDRUSE` bind, at no new-crate cost.
- Implementation is `std::net::TcpListener` + `httparse` (1.10.1 in `Cargo.lock`, reached today through `ureq → ureq-proto`) — **not** `hyper` (not linked on Windows; target-gated to android/apple by tauri) and **not** enabling `tokio/net` (the linked tokio lacks `net`), because that second option changes the linked feature set of the runtime under the whole engine. Spike § 8.1 prices all three.
- `TcpListener` appears in the tree only inside `#[cfg(test)]` modules ([engine/auth.rs:215](../../src-tauri/src/engine/auth.rs), [engine/bond.rs:378](../../src-tauri/src/engine/bond.rs)) — `writer:` grep. There is no production listener, today.

## 4 The three handoffs, and the one that is new here

**A. Re-pull (take the URL).** The add-on POSTs the URL; the app probes; the app says 200+token; the add-on cancels its own download and removes its partial; Sucr runs a normal job. This is the only mode that *earns* the app's promise. Cost stated plainly: the bytes the browser already carried are paid twice — **`unmeasured` as a figure**, spike § 8.3.

**B. Adopt (take the file).** The browser finishes; the add-on hands path + size + (if available) digest; Sucr moves the file to the rule-chosen folder and writes the row. Zero duplicate bytes, zero speedup. On Chrome this is the mode `onDeterminingFilename` can even place correctly; on Firefox it cannot, so B ships differently per browser.

**C. Import the partial (the middle path this draft adds).** The browser hands over its **in-flight** file plus its byte count; Sucr seeds the block manifest with the contiguous prefix and resumes with `Range: bytes=N-` on the same URL.

Why this is the interesting one: it is the only mode that is both *fast* and *not wasteful*, and the machinery it needs already shipped —
- contiguity: a browser downloads single-stream, so `0..N` is real. **`unmeasured`** whether Chrome's `.crdownload` / Firefox's `.part` is flushed-and-readable while the browser still holds the handle, and whether the rename that hands ownership over can happen before the browser releases it (on Windows, renaming a file another process holds open fails). Spike § 8.3.
- safety: importing someone else's bytes is *exactly* the resume-integrity question — same URL, possibly different remote. The validator rule shipped 2026-11-06 (`etag > last-modified`, cause named in the payload) is the guard, and a mismatch lands as `resume-reset` with the cause, not as a corrupt file.

**Proposed selection rule (CHOSEN):** take (A or C) when `acceptRanges` is true and `totalBytes` is above a threshold worth a multi-stream job; import (C) when a usable partial exists; adopt (B) when the browser already finished, or ranges are refused, or the file is small; **decline (422) for everything ambiguous** — `blob:`, `data:`, `chrome://`, POST-body forms, `Content-Disposition` flows we cannot reproduce, a host that refuses the app but not the browser, an expired signed URL. Ambiguity resolving to "keep it in the browser" is the whole reason the add-on can be trusted.

## 5 Authorization, made concrete (blocker 4)

- The socket is not the trust anchor; **the native port is** (the browser authenticated it). The socket is the data path and inherits a token minted by the app over that port.
- Token: 128-bit crypto-random, **one-shot**, bound to the connection and the URL that probed, issued at `/add` so its TTL only has to outlast one probe, not a whole user conversation. `getrandom` is in `Cargo.lock` (two versions) but **linked-ness is `unmeasured`** — per this repo's own rule, lock membership is not linkage (spike § 8.6).
- `Origin` is a second gate, never the first: Chrome sends `chrome-extension://<id>`; Firefox's pages are `moz-extension://<uuid>` and **whether a background-script fetch carries `Origin` at all is `unmeasured`** (spike § 8.5). Exact-match allowlist, no wildcard, per browser.
- A simple `text/plain` POST or `sendBeacon` skips a CORS preflight, and DNS rebinding can point a page at 127.0.0.1 — hence: loopback bind only (never `0.0.0.0`, never `::`), token-first, and **default-to-422 on every ambiguous state**.
- The socket may reach **exactly three** functions: the intake normaliser, the probe, and a status read. Nothing else. No `sucr_invoke`.
- **No cookie-store read, ever** (blocker 6). Sessions stay where the user pasted them, and the app's deliberate refusal to read browser state must not be reversed through a local port. (Plaintext-at-rest in `sessions` — `db.rs:796 session_at` / `:810 put_session` — remains a held defect, unchanged by this design.)
- Chrome's **Local Network Access** permission prompt: the doc I read describes it behind a flag in Chrome 138 and planned for 142, and says **nothing about whether extensions with `host_permissions` are exempt**. `unmeasured`, and it is the one OS-side behaviour that could make the whole loopback design require a user consent click per site — spike § 8.4 before drafting any sequence.

## 6 Registration, the exe move, and the publication reality (blocker 1)

`bundle.active` is `false` ([tauri.conf.json:27](../src-tauri/tauri.conf.json)). So today **nothing owns** the three registry keys, the two host manifests (`allowed_origins` vs `allowed_extensions` are different files), or the add-on's update channel — and moving `sucr.exe` silently bricks the door.

**Retracted 2026-11-07: "there is no installer toolchain on this box."** That was true of the *installed* programs when the notification-identity slice looked (no NSIS, no WiX on PATH), and it was read as "cannot bundle". The Tauri CLI fetches its own NSIS: measured, `tauri build --bundles nsis` downloaded `nsis-3.13.zip` and `nsis_tauri_utils.dll` from the tauri-apps GitHub releases, validated both hashes, cached them under `%LOCALAPPDATA%\tauri\NSIS\`, and produced `Sucr_0.1.0_x64-setup.exe` at **1 866 169 B**. WiX is still absent and still unneeded. The full verdict, including what the bundle-type patch does and does not change, is in [capture-step0-spikes.md](../../../design/capture-step0-spikes.md) § 1.

Chosen shape (CHOSEN): a first-run, **HKCU-only, no-elevation** self-register, plus a boot-time **self-heal** that re-reads each key, compares the stored host path to the running exe, rewrites it when it moved, and **shows the state per browser** in the app rather than assuming it. The same discipline the keep-awake and notification work used: the state must be displayed, not inferred from a call that returned successfully.

And the part no code can soften: **the Chrome add-on cannot be distributed without a Web Store listing** for an ordinary user, while the Firefox one can be self-hosted once AMO signs it. So the contract is **versioned and additive-only** (`proto_min`/`proto_max` in the handshake, app tolerates an add-on newer than itself, add-on tolerates an app older than itself), because the store is the slow mover and our exe is not.

## 7 Attribution and the surface

A handoff job runs on the normal engine, so its bytes land on the same per-adapter routes and the same caps — the Feature 1 machinery does not need to know the URL arrived from a browser. What it needs is a **source** so the row can say *captured from Chrome* and the totals can separate paste from capture. That is not free: three `json!` telemetry keys measured **512 B** in the ramp work, so a `source` field is priced, not assumed.

Two surfaces this draft owes and does not design yet: the add-on's own popup (one state line: *app found / app waking / not registered / declined because…*), and the app's Settings pane for it, which per the cost map is a **new renderer surface ≈ 5 KB** (precedent: the proxy panel at 3 584 B). Both are cheap; neither is free, and the pane is the user's call (§ 9).

## 8 Spikes that must run before this becomes an implementation sequence

1. Byte delta of `std::net` + `httparse` vs `tiny_http` vs enabling `tokio/net`; and confirm the two new-channel precedents (+12 800 / +27 648 B) on this toolchain.
2. Two instances against one data dir: tray, WAL, and the `SO_EXCLUSIVEADDRUSE` bind as the lock.
3. Chrome and Firefox **partial-file mechanics**: is the in-flight file contiguous, flushed, readable while open, and renameable after `cancel()`? This decides whether mode C exists at all.
4. Chrome Local Network Access: shipped or not, and does an extension with `host_permissions` for `127.0.0.1` bypass the consent?
5. Does Firefox send `Origin` on a background fetch to loopback, and does Chrome's value match the `allowed_origins` string byte-for-byte?
6. Is `getrandom` actually linked (lock membership ≠ linkage), and what does a 128-bit token cost?
7. Does an open native port keep a Chrome MV3 service worker alive indefinitely, or must the add-on re-arm an alarm?
8. Antivirus / SmartScreen: a loose exe that opens a loopback port and writes HKCU keys — flagged or not, on this box and in principle.
9. Whether the add-on can learn the port before any native message (if not, `hello` must be the first thing the port ever carries).

## 9 Asks — only the user writes `DECIDED`

1. **DECIDED (user, 2026-11-07): yes — publish the add-on.** Store accounts, review and a release cadence slower than the exe's are accepted. § 6 stands and the additive-only `proto_min`/`proto_max` contract is now load-bearing rather than precautionary. The `sucr://` fallback is no longer the only live door.
2. Default handoff mode: **take** (fastest, may double-carry), **adopt** (never wastes a byte, never speeds anything up), or **import-the-partial** (both, most new machinery, gated on spike 3). **Still the user's.** Spike 3's **Chrome half has now run** ([capture-step0-spikes.md](../../../design/capture-step0-spikes.md) § 4.1): the in-flight `.crdownload` is contiguous, flushed, readable while open and renameable while paused — but `cancel()` **deletes** it, and the file size runs 262 144 B ahead of `item.bytesReceived`. So mode C exists on Chromium via `pause()` only. The Firefox half is `blocked` and the browser legs are parked by the user (2026-11-07, § 4.5: a competing download manager's filter driver intercepts every browser download leg on that box). This ask is ripe to be put, on those facts, once the Rust spikes land.
3. Should capture be a first-class Settings pane (≈ 5 KB renderer), or a one-time registration with a state line only? **Still the user's.**
4. **DECIDED (user, 2026-11-07): yes — bundling on is acceptable as a prerequisite.** Measured available at **0 B** of exe cost (§ 6 retraction; [capture-step0-spikes.md](../../../design/capture-step0-spikes.md) § 1). Two things this ruling does *not* do by itself: it does not edit `tauri.conf.json` (the probe used a `-c` merge, so the product config is still `bundle.active:false`), and it does not install anything. The notification-identity unblock needs the installer to actually **run**, which writes HKCU keys and registers an AUMID — a user-visible step to take when that slice resumes, not a side effect of this one.

## 10 Staging, if the answers are yes

Step 0 spikes above → **1** single-instance ownership + `intake::normalise` shared by paste and handoff (no listener yet; provable by Rust tests alone) → **2** native host + HKCU registration + boot self-heal (wake path only) → **3** listener + token + **adopt** → **4** **re-pull** + the full 422 rule set → **5** **import-the-partial** → **6** Firefox parity (early refuse via `webRequestBlocking`, no `onDeterminingFilename`) → **7** `source` attribution + surfaces.

Every step carries the discipline the last four slices used: typecheck, `cargo test --release --lib -- --test-threads=1`, `npm run verify`, `stat` before and after, a launched-exe proof over CDP in its **own** `--data-dir`/WebView2 dir/out dir/CDP port, and a record in `D:\RE\Sucr\design\`.

## Sources

- [Chrome · Service worker lifecycle](https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle)
- [Chrome · Migrate blocking web requests (MV3)](https://developer.chrome.com/docs/extensions/develop/migrate/blocking-web-requests)
- [Chrome · declarativeNetRequest](https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest)
- [Chrome · downloads API](https://developer.chrome.com/docs/extensions/reference/api/downloads)
- [Chrome · Native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)
- [Chrome · Cross-origin network requests](https://developer.chrome.com/docs/extensions/develop/concepts/network-requests)
- [Chrome · Local Network Access](https://developer.chrome.com/blog/local-network-access)
- [Chrome · Distributing an extension](https://developer.chrome.com/docs/extensions/how-to/distribute)
- [Edge · Native messaging](https://learn.microsoft.com/en-us/microsoft-edge/extensions/developer-guide/native-messaging)
- [MDN · webRequest](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/webRequest)
- [MDN · downloads](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/downloads)
- [MDN · Native messaging (Firefox)](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging)
- [MDN · manifest.json/background](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background)
- [Bugzilla 1573659 · Firefox MV3 service worker](https://bugzilla.mozilla.org/show_bug.cgi?id=1573659)
- [Bugzilla 1245652 · downloads.onDeterminingFilename not implemented](https://bugzilla.mozilla.org/show_bug.cgi?id=1245652)
- [Extension Workshop · Signing and distribution overview](https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/)
- Competent prior art actually readable: [XDM's integration module](https://github.com/subhra74/xdm-integration-module) (uses the `downloads` API plus native messaging) and [JDownloader's MV3 add-on](https://github.com/magnetgrouplabs/myjdownloader-extension-mv3) (uses a cloud relay instead — neither uses a loopback port).
