<p align="center">
  <img src="branding/sucr-icon-256.png" width="112" height="112" alt="Sucr logo">
</p>

<h1 align="center">Sucr</h1>

<p align="center"><strong>Sucr 0.1.1 (Alpha)</strong> — a multi-tether download manager for Windows: turn on more than one network and one file leaves across all of them, every chunk tagged by the adapter that carried it.</p>

Sucr is a single native executable (Tauri 2 + WebView2, no Electron) of about 4.7 MB. It does not ship a browser engine, a background service, or an installer that runs at boot.

> **Alpha.** This is 0.1.1 of the Alpha stage. The feature set can change between releases.

![Sucr downloading a file](docs/screenshots/downloads.png)

## Multi-tether

Windows hands each new socket to one interface, so most download managers' "multi-connection" downloads still ride a single adapter. Sucr binds its connections to the networks you have enabled instead, so Ethernet, Wi-Fi and a phone tethered over USB carry the same file at once — and the Chunks inspector colours every block by the route that delivered it.

- **Dynamic** — every route carries blocks at once, and a straggler's tail is handed to whichever worker is free, on any route.
- **Hedging** — the primary carries the bulk; a block whose stream goes quiet for three seconds is raced on a secondary route, and the first half to arrive wins.
- **Failover** — one route until it errors, with the backup link left idle until then.

A metered guard keeps a metered connection out of the bond, and each adapter keeps its own speed cap and data allowance.

*Credit: the multi-tether idea — bonding several network connections onto one download — comes from Plexo; Sucr's implementation is its own.*

![Networks screen](docs/screenshots/networks.png)

## What it does

- **Multi-tether downloads.** One job spreads its blocks over every adapter you enable — Ethernet, Wi-Fi, a tethered phone — instead of queueing behind the one route Windows would have picked.
- **Eight connections that earn more.** A job starts with 8 concurrent HTTP connections and widens toward 32 only when an extra connection earns its share of the bandwidth. If the host refuses one, the job narrows again instead of stalling.
- **Resuming that checks.** A paused transfer continues only when the remote's ETag, `Last-Modified` and sampled bytes still match what was downloaded.
- **An inspector per download.** Overview, chunks with the route that carried each block, source, schedule, files and activity.
- **Proxies and credentials.** System, direct or manual proxy with Basic auth and bypasses, plus per-host site credentials for links that need a login.
- **Browser capture.** A Manifest V3 add-on that hands downloads to Sucr — Chromium (Chrome, Edge, Helium) and Firefox (Firefox, Zen).
- **Download window.** Queued links start only inside the hours and days you pick.
- **Updates, debug log and a report button** that opens a pre-filled GitHub issue. Nothing is sent unless you submit it.

## Benchmarks

Engine speed, measured with Sucr's own harness: a local HTTP fixture on the loopback interface serving a deterministic payload, staging on a SATA hard disk, six runs per scenario reported as medians, and every download validated byte-for-byte before its timer stops. The elapsed time covers the whole job — fetch, write, and the SHA-256 verification at publish — not just socket reads. These are machine-local numbers: they measure the engine, not any particular internet link.

| Scenario | Payload | Median |
| --- | --- | --- |
| One streamed range | 256 MiB, 1 connection | 1.115 s (≈ 230 MiB/s) |
| Split across 8 connections | 256 MiB, 8 connections | 0.866 s (≈ 296 MiB/s) |
| Server-paced single stream | 32 MiB, 4 MiB/s per request | 8.383 s (the server's own pace sets the floor) |
| Server-paced, 8 connections | 32 MiB, 8 connections | 1.374 s (≈ 23 MiB/s) |
| Server without `Range` support | 32 MiB | 4.278 s (≈ 7.5 MiB/s) |

The first two rows ran on the current development build. Before a write-latency fix they took 5.668 s and 7.647 s: with the staging drive on spinning disk, every ledger commit had been forcing its own flush.

The fixture and harness used for these runs live under `scratch/`, which is not part of the published tree; the method described above is what makes the numbers reproducible.

### Multi-tether, over the real internet

The table above is loopback, so it measures the engine rather than a link. This one is a real download across two Ethernet adapters on one machine, to show what bonding actually buys. The target is `SysinternalsSuite.zip` (374 682 911 bytes) served with HTTP range support. The only variable is how many adapters Sucr may bind to — **two bonded** versus **one**, the single path Windows hands a socket to on its own. Three runs each, interleaved, reported as medians.

| Adapters | Median throughput | Bytes carried by the second adapter |
| --- | --- | --- |
| One (single path) | ≈ 18.0 MiB/s | — |
| Two (bonded) | ≈ 20.6 MiB/s | 22–27% of the file |

Bonding delivered the same file **about 14.6% faster**, and the runs did not overlap: the slowest bonded download still beat the fastest single-path one. All six downloads hashed byte-for-byte identical, so bonding assembled a correct multi-source file, not just a fast one.

This is one box, one target, on one day. The gain scales with the spare capacity on your second link and can fall toward nothing if that link is slower than idle noise, so read it as a real-world confirmation of the feature rather than a portable headline number. This test's harness lives under `scratch/` as well.

## Install

1. Open [Releases](https://github.com/benjaminspoop7-sys/sucr/releases/latest) and download `Sucr_0.1.1_x64-setup.exe`.
2. Run it. It installs per-user (no administrator prompt) into `%LOCALAPPDATA%\Sucr`.
3. The build is unsigned, so Windows SmartScreen will warn you: **More info → Run anyway**.

Requirements: Windows 10 or 11, x64. The app uses the WebView2 runtime, which Windows 11 and current Windows 10 builds already include.

Progress is checked against the release feed compiled into the build. Settings → System → **Check for updates** compares your version with the newest published release and opens the installer link in your browser; it never installs anything by itself.

## Browser add-on

The installer ships the add-on under `%LOCALAPPDATA%\Sucr\addon`. Both browsers ask for a one-time confirmation to load it:

1. Launch Sucr once, so it registers the native-messaging host.
2. Settings → System → **Browser add-on** lists the browsers Sucr found. **Set up add-on** copies the bundled add-on to `%APPDATA%\Sucr\addon` and registers the relay; **Open add-on folder** opens that copy. Each browser also has an **Open** button that takes you to its own add-on page.
3. Chromium: Developer mode → **Load unpacked** → `addon\chrome`.
   Firefox: **Load Temporary Add-on** → `addon\firefox\manifest.json`.

With capture on, a browser download is handed to Sucr before it starts. Hold <kbd>Alt</kbd> over a link or a video to grab it directly; pasted links also work. On YouTube every format in the ladder takes as one merged file — video and audio are fetched separately and muxed by Sucr's own MP4/WebM muxer.

## Settings

![Engine settings](docs/screenshots/settings-engine.png)

![Network settings](docs/screenshots/settings-networks.png)

![System settings](docs/screenshots/settings-system.png)

Everything is stored locally in one SQLite file, `%APPDATA%\Sucr\sucr.db`. There is no account and no sync.

## Build from source

Prerequisites: a current Node.js LTS, a Rust toolchain, and the WebView2 runtime that ships with Windows.

```sh
cd app
npm install
npm run build        # renders src/renderer and builds src-tauri into a release exe
npm run typecheck
```

For the installer:

```sh
cd app
PATH="/path/to/cargo/bin:$PATH" ./node_modules/.bin/tauri build --bundles nsis
```

`npm run build` locates the real `cargo` through `CARGO_HOME` (falling back to `~/.cargo`) because the Tauri CLI cannot spawn a POSIX `cargo` shim.

A development run is a separate app from the installed one. `npm run dev` builds the debug binary under the identifier `app.sucr.dev` with the window titled `Sucr Alpha (dev)`, so its WebView2 profile lives in `%LOCALAPPDATA%\app.sucr.dev`. A debug build also keeps its own data root, `%APPDATA%\Sucr Dev`: the database, logs, downloads, capture door and the native-messaging manifests it registers all live there, and its relay answers to `app.sucr.capture.dev` rather than the installed `app.sucr.capture`. The installed app keeps its data, its registration and its add-on untouched, and the two can run at the same time without sharing a row, a setting or a browser hand-off.

The add-on has its own build:

```sh
cd extension
./build.sh           # writes dist/chrome and dist/firefox, plus dist/dev for a debug build
```

A capture test from a local build runs in a browser session of its own:

```sh
cd app
npm run browser:dev                          # Helium, or the first Chromium it finds
npm run browser:dev -- --browser=zen         # any browser registered under StartMenuInternet
npm run browser:dev -- -- --remote-debugging-port=9334
npm run browser:dev -- --remove              # unregister the dev relay when done
```

The launcher packages the dev add-on (it asks for `app.sucr.capture.dev`) and copies it to `%APPDATA%\Sucr Dev\addon\<family>`, then opens the browser on a throwaway profile under `%APPDATA%\Sucr Dev\browser\<browser>`. The copy carries no pinned `key` and the Firefox copy names `capture-dev@sucr.local`, so the dev add-on has an extension ID of its own and cannot stand in for the installed one even if both are loaded in one profile.

Rather than register anything system-wide, the launcher starts the debug app if its door is not already answering and writes the door's `{port, token}` to `relay.json` beside the add-on; a small watcher keeps that file following app restarts while the browser is open. The add-on reads that file only when its baked relay name ends in `.dev`, and the release copy refuses it, so a dropped `relay.json` beside the shipped add-on is ignored. The launcher opens a **Sucr dev session** tab that marks the window as a test session. Chromium browsers load the add-on directly; for a Firefox-family browser it prints the manifest path to load in `about:debugging`, because Gecko has no command-line way to install an unpacked add-on.

## Repository layout

| Path | What it is |
| --- | --- |
| `app/` | The application: `src/renderer` (UI), `src-tauri/src` (engine, SQLite, IPC), `src/shared` (models and the IPC contract) |
| `extension/` | The browser add-on, Chromium and Firefox manifests |
| `design/` | Design notes and measured records for each shipped slice |
| `branding/` | App icon sources |
| `docs/screenshots/` | The images in this README |

## License

Sucr is released under the MIT License. See [LICENSE](LICENSE) for the full text.
