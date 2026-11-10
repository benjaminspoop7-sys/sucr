# Browser-Bridge Handoff

* **Author:** Benjamin (machine account; `git config user.name` is unset on this box)
* **Date:** 2026-11-03

## Overview

Let a click in Chrome or Edge hand a download to Sucr instead of the browser, with Sucr deciding
whether it should have the file at all.

As proposed: a loopback HTTP listener on 127.0.0.1 in a fixed port range answering ping, add and
confirm; Sucr probes the link and answers 422 to leave it in the browser or 200 to take it; a
short-lived token authorises the take; the browser extension cancels its own download only after that
token returns; native messaging is the wake path that starts Sucr when it is not running. The decide-
in-the-app inversion comes from a competitor audit, not from this repo.

**Nothing of this exists today, and the shape of "nothing" matters.** The only ingest door is
`download:add` ([ipc.rs:494](../../../app/src-tauri/src/ipc.rs)), which hard-rejects anything that is not
`http(s)://`. There is no listener in production code — the only `TcpListener` in the tree is a unit
test ([engine/bond.rs:370](../../../app/src-tauri/src/engine/bond.rs)) and a Node fixture
(test/chaos-server.mjs). There is no protocol handler, because bundling is off: `bundle.active` is
`false` ([tauri.conf.json:27](../../../app/src-tauri/tauri.conf.json)). There is no single-instance guard
of any kind.

## Analysis

**Two of the author's premises were wrong, and both made the feature look cheaper than it is.**
First, "hyper and tokio are already linked, so a listener is nearly free." The reviewers checked the
compiled units: `hyper` and `reqwest` are transitive lock entries that are **not linked on Windows** —
they are target-gated to android/apple by tauri — and `tokio` *is* linked, but **without the `net`
feature**, so there is no `mio`. Second, "per-network accounting has to be built" is not this feature's
problem, but the probe's cost is understated: `probe_link` sets a 5 s connect timeout
([ipc.rs:750](../../../app/src-tauri/src/ipc.rs)) *and* 4 s per read with 10 redirects
([engine/mod.rs:147](../../../app/src-tauri/src/engine/mod.rs)), so a decision can take on the order of a
minute against a 15 s token. The honest floor is `std::net::TcpListener` plus `httparse`, which
*is* already linked through `ureq → ureq-proto`.

**Blockers, in dependency order:**

1. *No artifact can register anything.* With bundling off there is no installer and no signing story,
   so nothing owns the native-messaging manifest or the per-browser HKCU key, and moving the exe
   silently bricks the bridge. A registration step plus a boot-time self-heal is a prerequisite, not a
   detail.
2. *The wake path starts a second Sucr.* No single-instance guard exists, so a browser-launched process
   opens its own WAL connection ([db.rs:14](../../../app/src-tauri/src/db.rs)), its own tray icon, and a
   contended port, and — because data-dir resolution keys off `--data-dir` / `SUCR_DATA_DIR`
   ([paths.rs:14](../../../app/src-tauri/src/paths.rs)) — it can land in a *different* database. Cheapest
   real fix: a `SO_EXCLUSIVEADDRUSE` bind-as-lock, since `socket2` is already a direct dependency.
3. *The listener would bypass the app's own gates.* Intake policy is split today: folder, category and
   checksum defaults resolve inside `download_add` ([ipc.rs:510](../../../app/src-tauri/src/ipc.rs)), while
   the link check and the missing-folder refusal live in the renderer's `LinkCheck` state machine
   ([main.ts:3940](../../../app/src/renderer/main.ts)). A bridge that calls `download_add` inherits the
   defaults and skips both gates. This must be one server-side intake normaliser that the paste flow
   also uses, or the two paths will diverge on the first bug report.
4. *"A loopback port plus a token" is not an authorization model.* A simple `text/plain` POST or
   `sendBeacon` skips CORS preflight, DNS rebinding can point a page at 127.0.0.1, and a fixed port
   range can be squatted by a process that forges "the app is running". Required shape: bind asserted
   to 127.0.0.1 only, an exact-match `chrome-extension://<id>` Origin allowlist with no wildcard,
   crypto-random one-shot tokens bound to the connection that probed, default-to-422 on every ambiguous
   state, and no reuse of the window-only `sucr_invoke` dispatch — which would otherwise expose
   `app:quit` and `settings:set` to a local process.
5. *Size.* Two added IPC channels exceed the entire 20,684 B headroom at the gate's own recorded ~11 KB
   per command ([verify-m0.mjs:28](../../../app/scripts/verify-m0.mjs)). The viable route is zero new
   channels: the bridge thread dispatches internally against `AppState`, reusing the same
   `Mutex<Connection>` as the engine writers, and says so in writing.
6. *Cookies would arrive over an unencrypted path.* Session values are stored in plaintext in
   `credentials` ([db.rs:286](../../../app/src-tauri/src/db.rs)) after a deliberate manual paste. The app
   refuses to read the browser's cookie store on purpose; a bridge must not become that read by the
   back door.

**Terminology is already broken here.** "Bridge" in this repo means the Tauri IPC boundary
([db.rs:296](../../../app/src-tauri/src/db.rs)), the refusal flow ([main.ts:2355](../../../app/src/renderer/main.ts)),
and an unrelated Python tool at `D:\RE\Sucr\bridge\`. "Extension" means the filename suffix
([ipc.rs:552](../../../app/src-tauri/src/ipc.rs) literally says "the extension decides the starting
category"). The brief has to rename before it can be read: handoff listener, browser add-on.

**The decision half already ships; the delivery half is a second product.** `sucr_probe`
([lib.rs:50](../../../app/src-tauri/src/lib.rs)) already runs the verdict off-thread and returns
`ok/code/status/totalBytes/acceptRanges/filename/session`, though it discards `content_type`,
`final_url` and `etag`. Whatever carries the URL in, that door does not have to change.

## Goals vs. Non-Goals

* **In-Scope Goals:** a handoff listener that cannot be driven by a page or another local process; one
  intake normaliser shared by paste and handoff; versioned app/add-on handshake that fails to 422;
  single-instance ownership; a registration path that survives an exe move; the add-on kept additive so
  a pasted browser session never loses a download.
* **Out-of-Scope Non-Goals:** media or HLS/DASH capture; magnet and torrent intake; reading the
  browser's cookie store; replacing the paste flow; a Firefox add-on in the first round; automatic
  store publishing.

## Alternatives Weighed

* **A `sucr://` URL-scheme handler or a browser context-menu item.** Rejected only for now: no scheme is
  registered because bundling is off, and it still needs the add-on. It remains the smallest surface
  that reuses the existing probe at zero listener cost, so it is the fallback if the add-on budget is
  refused.
* **A clipboard watcher.** Rejected outright by two lenses: it reads the user's whole clipboard and
  contradicts the deliberate refusal to take browser state without asking
  ([db.rs:288](../../../app/src-tauri/src/db.rs)).
* **An external helper process, outside sucr.exe.** Kept as a live option: it costs zero gated bytes,
  the repo already has precedent for an external bridge script, and it isolates the trust boundary from
  the product binary. Cost: another artifact to install and version.
* **Making paste pleasant.** The add bar *is* a paste box by design
  ([main.ts:1650](../../../app/src/renderer/main.ts)). For the residual jobs — a host that refuses the
  browser, a link that needs a session — a pasted browser session already answers it. Rejected as the
  whole strategy, not as the first measurable step.
* **hyper or a framework listener.** Rejected on evidence: `hyper` is not linked on this target, so it
  is new bytes against a margin that cannot pay for it.

## Key Decisions & Staging for Next Steps

1. Resolve the two prerequisites first — single-instance ownership and a registration story — because
   without them the design is unbuildable regardless of what the listener looks like.
2. Decide the intake normaliser before the protocol; the 422 rule is only trustworthy if paste and
   handoff cannot diverge.
3. Adopt zero-new-channels as a constraint, not an optimisation.
4. Spikes that must run before drafting: exe delta of `std::net` + `httparse` versus `tiny_http`
   versus `tokio/net` + hyper; confirm the ~11 KB per-channel figure on this toolchain; whether
   `getrandom` is linked and what it costs; two instances against one data dir (tray, WAL, port);
   whether the add-on can learn the port before any native message; whether the app is already
   mis-flagged by antivirus or firewall heuristics.
5. Record the versioned contract next to `TRANSPORT.md` in `D:\RE\Sucr\design\`, using that document's
   measured-not-ratified ladder, and make it additive-only because store review makes the add-on the
   slow mover.
6. Sequence it after the cheap correctness items. The product's promise is accounted-for bytes; a local
   port is an unattributable client inside that promise, and it should not be the first thing to spend
   the remaining margin.
