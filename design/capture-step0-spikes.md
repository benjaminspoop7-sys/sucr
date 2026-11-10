# Browser capture — step 0 spike record

* **Started:** 2026-11-07
* **Status:** in progress. Design/spike only — **0 product bytes spent**, no product file edited. § 1 bundling closed; § 4 browser legs **parked by the user** with the Chrome half measured; §§ 2, 3, 5 are the live work.
* **Authority:** the user ruled asks **1** and **4** of [browser-capture-design_draft.md](../app/docs/designs/browser-capture-design_draft.md) on 2026-11-07: **publish the add-on**, and **bundling on is acceptable as a prerequisite**. Asks 2 and 3 remain the user's; only the user writes `DECIDED`.
* **Why this file exists:** the draft's § 8 lists nine spikes that must run before capture becomes an implementation sequence. Each verdict lands here with its evidence, and a spike that cannot run on this box is recorded as `blocked` or `unmeasured` — never as a pass.

Admission rule applied throughout: a claim about what Sucr or the toolchain does carries a `writer:` (who produced the evidence), a `predicate:` (what would falsify it) and an `evidence-noun:` (the file, report or byte offset it rests on). Confidence words are not evidence.

## 1 Bundling — RUN, verdict: available at 0 B of exe cost

The draft's § 6 blocker was "no installer toolchain on this box". **Retracted.** It was true of the programs installed on PATH and was read as a capability limit; the Tauri CLI brings its own.

### 1.1 What ran

Two commands, neither of which edited a product file:

1. A full build with bundling merged in over the CLI's `-c` flag, into a **separate** `CARGO_TARGET_DIR` so the shipped exe could not be overwritten:

   ```
   PATH="/h/tools/cargo/bin:$PATH" CARGO_HOME=H:/tools/cargo RUSTUP_HOME=H:/tools/rustup \
   CARGO_TARGET_DIR=D:/RE/Sucr/app/src-tauri/target-bundle-probe \
   node node_modules/@tauri-apps/cli/tauri.js build \
     --runner 'H:/tools/cargo/bin/cargo.exe' \
     -c '{"bundle":{"active":true,"icon":["icons/icon.ico"]}}' --bundles nsis
   ```

2. `tauri bundle` (no rebuild) over a **byte-copy of the shipped exe** in `D:\RE\_cap-harness\bundle-patch-ctl\release\`, same `-c` merge. This is the control that isolates the patch: same file before and after, so any diff is the bundler's.

Two box facts made command 1 work, both measured before it ran:

- `--runner <BIN>` replaces cargo **for the build only**. The CLI still shells out to `cargo metadata` from PATH, and PATH's `cargo` here is a `#!/bin/sh` shim, so the first attempt died with `failed to run 'cargo metadata' … program not found`. Prepending the real `H:/tools/cargo/bin` (whose `cargo.exe` starts with `MZ`, 12 724 224 B) fixed it; `--runner` was kept.
- `-c/--config` **merges** JSON, so `bundle.active:true` was probed without touching [tauri.conf.json](../app/src-tauri/tauri.conf.json). That file is still 30 lines and still `bundle.active:false`.

### 1.2 Measurements

| Thing | Bytes | How |
|---|---|---|
| Shipped exe (untouched throughout) | **4 593 152** | `stat` |
| Probe exe, built with bundling on | **4 593 152** | `stat` |
| Copy of shipped exe, before `tauri bundle` | **4 593 152** | `stat` |
| Same copy, after `tauri bundle` | **4 593 152** | `stat` |
| `sucr.exe` extracted from the installer | **4 593 152** | `stat` |
| Installer from the probe build | **1 866 169** | `stat` |
| Installer from the shipped-exe copy | **1 866 696** | `stat` |

`writer:` this harness, `stat -c %s` on each path. `predicate:` any of the five exe numbers differing from 4 593 152 would falsify "0 B". `evidence-noun:` the table above; the two installers differ by 527 B, which is NSIS metadata, not app code — the embedded exe is the same size in both.

**The installer is a separate artifact and does not count against the 5 MB exe ceiling.** Headroom stays **649 728 B**.

### 1.3 What the bundle-type patch actually does

The CLI logs `Patching … with bundle type information: nsis`. A log line is not a witness, so the patch was isolated three ways.

- **Source.** [tauri-bundler/src/bundle.rs:40-99](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle.rs) at the installed CLI's version (2.12.4): `BUNDLE_VAR_TOKEN = b"__TAURI_BUNDLE_TYPE_VAR_UNK"`, found by `kmp::index_of`, then `copy_from_slice(b"__TAURI_BUNDLE_TYPE_VAR_NSS")` over the same span. Both are 26 bytes, so **the write cannot change the file size**. Local copy: `D:\RE\_cap-harness\bundler-bundle.rs`.
- **Bytes.** The installer's embedded exe differs from the shipped exe in **exactly 3 bytes**, at offsets **3998149, 3998150, 3998151**: `…_VAR_UNK` → `…_VAR_NSS`. Marker counts in that copy: `UNK` **0**, `NSS` **2** (one match arm, one patched static). In the loose shipped exe: `UNK` **1**, `NSS` **1**.
- **Runtime.** `plugin:app|bundle_type` over CDP, three launched exes, each in its own `--data-dir`/WebView2 dir/out dir/CDP port:

  | exe | `bundle_type` | `identifier` | bridge | title | body children |
  |---|---|---|---|---|---|
  | shipped (loose) | `null` | `app.sucr` | ready | `Sucr 0.1.0` | 13 |
  | probe (loose, bundling-on config) | `null` | `app.sucr` | ready | `Sucr 0.1.0` | 13 |
  | installer's extracted copy | **`"nsis"`** | `app.sucr` | ready | `Sucr 0.1.0` | 13 |

`writer:` [bundle-type-proof.mjs](../../_cap-harness/bundle-type-proof.mjs). `predicate:` the installer copy answering `null`, or the shipped copy answering `"nsis"`, would falsify the patch landing. `evidence-noun:` `D:\RE\_cap-harness\bundle-type-report-2026-11-06T20-12-16-056Z.json` — **7 checks, all pass**, control group (`null`) differs from test group (`"nsis"`).

### 1.4 The trap this spike fell into, and the correction

The first run of the proof compared the two **loose** exes, both answered `null`, and the harness called the patch failed. That verdict was wrong and is retracted here.

`bundle_project` keeps a `tempfile::tempfile()` copy of the original binary and, after each package type, truncates the target and copies the original back — the source comment reads **"Restore unsigned and unpatched binary"** ([bundle.rs:212-218](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle.rs)). Confirmed by measurement: the control copy was **0 differing bytes** after `tauri bundle` despite the log line naming it.

So the loose exe on disk is *designed* to be unpatched; only the copy inside the installer carries `nsis`. The general lesson, added to the harness-witness catalogue: **a build log naming a path is not evidence about the file at that path afterwards** — the tool may have restored it. Ask the artifact that ships, not the one left in the target dir.

Two side observations recorded so they are not mistaken for findings later:

- `.rsrc` is **byte-identical** (0 of 47 104 raw bytes differing) between the shipped and probe builds, and `icons/icon.ico`'s first image is present in **both** at offset 4528616. `tauri-winres` embeds the icon regardless of `bundle.icon`, so the `-c` icon list bought nothing and cost nothing.
- `.text` virtual size differs by 256 B between the two builds while every section's raw size is identical. The exe is **not byte-reproducible** (3 499 020 of 4 593 152 bytes differ between two builds of the same source), so that 256 B is build noise and is **not** attributed to the config merge. Size is judged by `stat`, never by a hash or a section-level diff across builds.

### 1.5 What this unblocks, and what it does not

- **Unblocks:** an NSIS installer can be produced on this box, repeatably, at 0 B of exe cost. The draft's § 6 packaging blocker is gone. `--no-binary-patching` exists (CLI 2.12.4) for the day a signature matters: patching rewrites 3 bytes and would invalidate one.
- **Does not unblock by itself:** the notification-identity gap. That needs the installer to **run**, which writes HKCU keys and registers an AUMID. Nothing was installed here — the installer was only extracted with 7-Zip (`C:\Program Files\7-Zip\7z.exe`) into a scratch dir. Running it is a user-visible, registry-writing step and is owed to the user, not taken as a side effect.
- **Still absent:** WiX (no MSI). Not needed for a single-bundle-type ship.
- **Config not edited:** `bundle.active` is still `false` in the product config. The ruling makes turning it on *acceptable*; it gets applied at the step that needs it (step 2, registration and packaging identity), not pre-emptively in a shared tree another agent also builds in.

### 1.6 Artifacts

| Path | What |
|---|---|
| `D:\RE\_cap-harness\bundle-probe.log` | full build + NSIS fetch + `tauri-exit=0` |
| `D:\RE\_cap-harness\bundle-patch-ctl.log` | the control `tauri bundle` run |
| `D:\RE\_cap-harness\bundle-type-proof.mjs` | the three-exe launched proof |
| `D:\RE\_cap-harness\bundle-type-report-2026-11-06T20-12-16-056Z.json` | 7 checks, all pass |
| `D:\RE\_cap-harness\bundler-bundle.rs` | the patch + restore source, fetched at the installed tag |
| `D:\RE\_cap-harness\nsis-extract\` | the installer's contents, including the patched `sucr.exe` |
| `D:\RE\Sucr\app\src-tauri\target-bundle-probe\` | the probe build tree (separate from `target\`) |

The first report, `bundle-type-report-2026-11-06T20-08-39-272Z.json`, contains the **retracted** two-exe verdict (2 of 5 checks failing). Kept, not deleted, so the correction has a witness.

## 2 Spikes 1 + 6 — listener and token pricing: RUN

**Verdict: the candidate stack is `std::net` + `httparse` at +7 680 B, and a 128-bit token costs 0 B on top of it because the entropy source is already linked.** The two alternatives are 5.3× and 11.4× dearer. A new IPC *door* is cheap (+1 024 B string arm, +2 048 B real command); the dearer numbers in the project's cost map priced a whole feature, not the door.

### 2.1 How it was measured

A **full copy of the crate** at `D:\RE\_cap-harness\listener-spike\app`, made with `tar --exclude='src-tauri/target*'`. No product file is edited by any variant. One shared `CARGO_TARGET_DIR` so dependencies compile once and each variant pays only its own relink. Between variants the driver restores `Cargo.toml`, `src/ipc.rs` and `src/lib.rs` from `pristine/`, writes one `src/capture_spike.rs`, and re-wires the same four anchors.

`writer:` [listener-spike.mjs](../../_cap-harness/listener-spike.mjs), `stat -c %s` on `target/release/sucr.exe` after each `cargo build --release`. `predicate:` any variant's size equalling v0's would falsify "the listener costs something"; the import-set diff below would falsify "the code landed" if a variant's imports were identical to its control's. `evidence-noun:` `listener-spike/results.json`, the per-variant `v*.log`, and the kept `v*.exe` copies scanned by [pe-imports.mjs](../../_cap-harness/pe-imports.mjs).

**The control is v0, the untouched copy: 4 593 152 B — exactly the shipped size.** Every delta is `Vn − V0`, same directory, same toolchain, same profile. Build time was 64–71 s per variant.

| Variant | Bytes | Δ vs v0 | What it adds |
|---|---|---|---|
| **v0-baseline** | **4 593 152** | — | the untouched copy |
| v1-channel-only | 4 594 176 | **+1 024** | a `capture:state` string arm + `CHANNELS` 35→36 + one module, **no listener** |
| v8-tauri-command | 4 595 200 | **+2 048** | a real `#[tauri::command]` in `generate_handler!`, no listener |
| **v2-std-httparse** | **4 600 832** | **+7 680** | v1 + the candidate listener: `std::net` + `httparse` + `socket2`, placeholder token |
| v3-bcrypt | 4 600 832 | +7 680 | v2 + `BCryptGenRandom` (new `Win32_Security_Cryptography` feature) |
| v4-processprng | 4 600 832 | +7 680 | v2 + `ProcessPrng` by raw `extern`, `kind = "raw-dylib"`, no new feature |
| v5-getrandom | 4 600 832 | +7 680 | v2 + `getrandom::fill` (`getrandom = "0.4"`) |
| v7-tokio-net | 4 634 112 | **+40 960** | v2 + `tokio` `net`/`rt`/`io-util` on the tokio already linked without `net` |
| v6-tiny-http | 4 680 704 | **+87 552** | a new server crate instead of `std::net` + `httparse` |

Listener-only cost, isolated by subtracting v1: **std::net+httparse +6 656 B · tokio/net +39 936 B · tiny_http +86 528 B.**

### 2.2 The content proof, because three variants share one byte count

v2, v3 and v5 are all **4 600 832 B**. At this project's 512 B granularity that is a 0 B delta — but equal sizes are only a finding if the exes genuinely differ. `stat` cannot answer that, so each built exe was kept and its **PE import directory** walked.

| Comparison | Only in the second | Source |
|---|---|---|
| shipped → v2 | **`ws2_32.dll!listen`, `ws2_32.dll!accept`** (2 imports, 0 removed) | `imports-v0-v2.json` |
| v2 → v3 | **`bcrypt.dll!BCryptGenRandom`** (1 import, **a whole new DLL**, 0 removed) | `imports-v2-v3.json` |
| v2 → v4 | **nothing — the import sets are identical** | `imports-v2-v4.json` |

So the listener's entire new OS surface is two Winsock calls, and **v3 really does add a new DLL and a new function while measuring exactly the same size as v2**. The 0 B is real, not dead-code elimination.

**v4's identical import set is the predicted result, not a void measurement.** `ProcessPrng` is already imported (§ 2.3), so a raw extern against it adds nothing to the import table. The witness that v4's call is *live* is its own first build: with a plain `#[link(name = "bcryptprimitives")]` the linker was asked for `bcryptprimitives.lib` and died with `LNK1181`. **A declaration the compiler had eliminated would never have reached the linker.** Switching the same declaration to `kind = "raw-dylib"` turned that failure into a successful link at the same byte count. The failure mode is the control.

Two harness bugs were fixed to get a trustworthy diff, both recorded so the numbers are not re-read naively:

- **DLL-name case.** The same build emits both `KERNEL32.dll` and `kernel32.dll` descriptors. A case-sensitive comparison reported **32 fake "only in first" and 32 fake "only in second"** entries and buried the two real ones. DLL names are case-insensitive to the loader; the comparison now lowers them. After the fix the shipped→v2 diff is exactly the two Winsock calls.
- **The exe was being overwritten.** The driver originally kept only the last variant's exe, so no cross-variant content claim was possible. It now copies each successful build to `<id>.exe`.

**The general lesson for the cost model: 0 B means "below the 512 B measurement granularity", not "free".** A new DLL import fits in that slack.

### 2.3 RETRACTED — "a 128-bit token needs another entropy source"

The pending text in this section said: *"`getrandom` has **0 hits** despite two `Cargo.lock` entries (lock membership ≠ linkage), so a 128-bit token needs another entropy source and its cost is unmeasured."* **The inference is falsified and withdrawn.** The shipped exe already imports the CSPRNG:

| Needle | Hits in the shipped exe | First offset | What it actually is |
|---|---|---|---|
| `ProcessPrng` | 1 | **4 392 674** | the import name, `bcryptprimitives.dll!ProcessPrng` |
| `bcryptprimitives` | 1 | 4 392 686 | the import DLL name |
| `getrandom` | **1** | **3 393 832** | the literal error string `"getrandom: this target is not supported"` |
| `BCryptGenRandom` | 0 | — | genuinely absent; v3 adds it |
| `getrandom-0.x.y\src` | 0 | — | no crate-path string |

`writer:` [exe-strings.mjs](../../_cap-harness/exe-strings.mjs) and [pe-imports.mjs](../../_cap-harness/pe-imports.mjs) over the shipped exe. `predicate:` a zero count for `ProcessPrng` **in the import table** would have supported the old claim; the import table is the authority on linkage and it says 1. `evidence-noun:` the offsets above, `imports-v0-v2.json`.

**Why the original scan missed it.** The old measurement looked for crate-path strings (`getrandom-0.4.3\src`). getrandom **returns `Error`s instead of panicking**, so it leaves no `#[track_caller]` location string — while `httparse-1.10.1\src` and `socket2-0.6.5\src` do survive because those crates panic. The instrument was wrong for the question. 63 distinct crate-path strings are present in the shipped exe and none of them is getrandom's, yet its error string and its backend import are both there.

So: **the entropy source is already paid for.** Which caller owns the import — std's Windows PAL (it sits beside `RtlGetVersion`, `GetProcAddress`, `GetModuleHandleA` in the same descriptor group) or getrandom's backend — is **not separable from the import table alone**, and `rust-src` is not installed on this box so std's source cannot be read to settle it. It does not change the price: the DLL and the function are already imported either way. Recorded as `unattributed`, not guessed.

### 2.4 What this changes in the design

- **Transport chosen on cost:** `std::net::TcpListener` + `httparse`, both already in the exe. +6 656 B for the listener over the door. `tokio/net` is 6× that and `tiny_http` 13× that, for no capability the capture hand-off needs.
- **Token chosen on cost:** all three routes measure 0 B over v2. `ProcessPrng` via `getrandom::fill` adds **no new DLL** (v5's import set is v2's); `BCryptGenRandom` adds `bcrypt.dll`. The already-imported route is the one to take, and it needs no new `windows-sys` feature.
- **The door is cheap.** +1 024 B for a string arm, +2 048 B for a real command. The cost map's "+12 800 / +27 648 B per new channel" is **not** the price of the door. Those two numbers are now read at their source rather than inferred: `design/proxy-and-http-auth.md:397` lists step 3's 12 800 B as three `db::` credential functions, the engine half, a typed error, the card's two boxes with three answer states and `--smoke` assertions, **and** the `proxy:set-auth` channel; `:398` lists step 4's 27 648 B as the middleware, store, card, link-check distinction, **and** the `origin:set-auth` channel with 8 new tests. Both rows say in their own text that the step was one link and the engine/renderer/gate split is **not measured**. So the honest statement is: a whole feature that happened to include a channel cost 12–28 KB, and the channel's own share was never isolated until this spike isolated it at 1 024 B.
- **Headroom:** 5 242 880 − 4 600 832 = **642 048 B** under the ruled 5 MiB ceiling (re-derived, not remembered). The listener and token together fit in **7 680 B**, i.e. **1.2 %** of that headroom. v9, the fixed-port variant spike 2 needs, measures **4 601 856 B** (+8 704 over v0, +1 024 over v2 for the fixed-port string, the `AtomicU16`/`AtomicU32` bind-stage counters and the served `pid`), leaving **641 024 B**.

### 2.5 A toolchain fact worth keeping, and what this section does not price

**`bcryptprimitives.lib` does not exist on this box.** v4's first build died with `LNK1181: cannot open input file 'bcryptprimitives.lib'`; there is no such file under the MSVC `VC/Tools/MSVC/*/lib/x64` or the Windows SDK `10/Lib/*/um/x64` directories. getrandom's own Windows backend documents the same fact in a source comment — *"As bcryptprimitives.dll lacks an import library, we use raw-dylib"* — and uses `#[link(name = "bcryptprimitives", kind = "raw-dylib")]`. With that attribute v4 builds at **4 600 832 B**, the same as v2/v3/v5. Any future raw Windows extern against a DLL that ships no import lib needs `raw-dylib`, not a plain `#[link(name)]`.

What this section does **not** price:

- The spike listener serves a **placeholder** token in v1/v2/v6/v7/v8, so those rows price the transport without pricing authentication. v3/v4/v5 price a real 128-bit token, and all three measure 0 B over v2.
- No variant prices the **hand-off payload** (URL, filename, cookies, referrer), the JSON shape, or the renderer surface. Those are implementation steps, not step 0.
- No variant is a **complete capture listener**: each serves one `GET` returning its own state, to isolate cost. Request routing, POST bodies, CORS headers and the add-on's polling loop are unpriced.
- The token routes are priced for **generation**, not for the comparison, storage or per-request validation a real hand-off needs.

### 2.6 Artifacts

| Path | What |
|---|---|
| `D:\RE\_cap-harness\listener-spike.mjs` | the variant driver (scratch copy, pristine restores, exe copies) |
| `D:\RE\_cap-harness\listener-spike\results.json` | every row: bytes, delta, seconds, exit, deps, features |
| `D:\RE\_cap-harness\listener-spike\driver.log`, `driver2.log` | the two runs |
| `D:\RE\_cap-harness\listener-spike\v*.exe` | kept builds for the import-table comparison |
| `D:\RE\_cap-harness\pe-imports.mjs` | PE import-directory walker + case-folded set diff |
| `D:\RE\_cap-harness\exe-strings.mjs` | needle counts and crate-path inventory |
| `D:\RE\_cap-harness\imports-v0-v2.json`, `imports-v2-v3.json`, `imports-v2-v4.json` | the content proofs: the two new Winsock calls, the new `bcrypt.dll`, and v4's identical import set |
| `D:\RE\_cap-harness\listener-spike\v9-fixed-port.exe` | the fixed-port build spike 2 launches (4 601 856 B) |

Product tree verified untouched after the runs: no `src/capture_spike.rs`, `CHANNELS` still `[&str; 35]`, `generate_handler![sucr_invoke, sucr_probe, smoke_report]`, `bundle.active` still `false`, shipped exe still **4 593 152 B** with mtime **2026-11-06 11:22**, and no `.rs` under `src/` modified today.

## 3 Spike 2 — single-instance ownership: RUN, verdict: the bind is a complete detector, and the reaction is still a decision

**0 product bytes spent.** Everything below ran on a scratch copy (`v9-fixed-port.exe`) and on the untouched shipped exe, each with its own `--data-dir`, WebView2 folder and CDP port. The product tree was not edited.

### 3.1 What Sucr does today — leg A, the shipped exe

Two instances of the **shipped** 4 593 152 B exe, **one** data dir, no listener anywhere.

| Observation | Measured |
|---|---|
| Both processes boot and both bridges come ready | pids **18600** and **17612**, both `title: "Sucr"` |
| Instance 2 reads instance 1's write | `ui.meter_style` set to `wave` by #1, `settings:list` on #2 returned `wave` |
| The shared DB survives two idle instances | `sucr.db` 4 096 B, `journal_mode = wal`, `integrity_check = ok`, `user_version = 4`, **17 tables** |

So there is **no single-instance guard**, and the cross-instance write hazard is **real, not theoretical**: two windows on one profile share one row set and each reads the other's writes. `ui.meter_style` was chosen because it is inert — flipping it cannot start, stop or re-route a job.

`writer:` [single-instance-spike.mjs](../../_cap-harness/single-instance-spike.mjs) leg A over CDP + `node:sqlite` in `readOnly` mode. `predicate:` if a guard existed, the second launch would have exited, focused the first, or refused the shared dir; it did none of those, and the second instance's `settings:list` would not have carried the first's write. `evidence-noun:` `single-instance-report-2026-11-06T20-59-26-089Z.json`, leg `A-today-shipped`, `crossWrite.seenBySecondInstance = "wave"`.

**The qualifier that matters:** both instances were **idle**. `integrity_check = ok` says nothing about two engines pumping one queue — see § 3.6.

### 3.2 The OS half — five raw-socket legs, no app involved

`SO_EXCLUSIVEADDRUSE` is the Windows option that makes a loopback bind behave like a lock. socket2 0.6.5 has **no helper for it**, so the spike sets it through a raw `setsockopt` before `bind`, using `windows-sys`'s existing `Win32_Networking_WinSock` feature. Whether the option is load-bearing was measured on .NET sockets, where `ReuseAddress` and `ExclusiveAddressUse` can be set independently — a Node `net.Server` would have been the **wrong** squatter, because libuv sets `SO_EXCLUSIVEADDRUSE` itself on Windows.

| Leg | Owner | Second | Port | Second bound? | WSA code |
|---|---|---|---|---|---|
| 1 — hijack control | reuse | reuse | 8171 | **yes** | 0 |
| 2 — the candidate lock | **exclusive** | reuse | 8172 | no | **10013** `WSAEACCES` |
| 3 — two Sucr instances | **exclusive** | **exclusive** | 8173 | no | **10048** `WSAEADDRINUSE` |
| 4 — zero-option control | plain | plain | 8174 | no | 10048 |
| 5 — squatter **first** | reuse | **exclusive** | 8175 | no | **10048** |

Leg 1 is the counterfactual that makes the rest mean something: **on Windows a second `SO_REUSEADDR` bind takes a live port.** The POSIX-shaped assumption is false here. Leg 4 is the second control: a plain bind already refuses, so the option's value is not "refusing a duplicate" — it is **leg 2**, refusing the *hijack* that leg 1 proves is otherwise possible. Leg 5 is the one that decides whether the bind can be a lock at all: a foreign reuse listener that got there **first** still refuses an exclusive second bind, so Sucr cannot end up sharing the port with a squatter.

`writer:` [exclusive-addr-probe.ps1](../../_cap-harness/exclusive-addr-probe.ps1), five legs on five ports, every socket closed in `finally`. `predicate:` if the option were decorative, leg 2 would have bound like leg 1; if the lock were incomplete, leg 5's exclusive second bind would have succeeded. `evidence-noun:` `exclusive-addr-report-2026-11-06T20-44-31-9063833Z.json`, `secondResult.native` per leg, and its `verdict` string.

### 3.3 The launched half — legs B and C, 16/16

`v9-fixed-port.exe` (**4 601 856 B**) is the § 2 listener on a **fixed** port (`SUCR_SPIKE_PORT`, default 8161) with `SO_EXCLUSIVEADDRUSE`, plus an `AtomicU16` bind stage and an `AtomicU32` WSA code served over `capture:state`. Port 0 — the cost variants' ephemeral port — locks nothing, because every process gets its own.

The stage counters exist because a silent `return` on a failed bind leaves `port` at 0, which is indistinguishable from "the thread has not run yet". Stage **3** = listening, **92** = bind refused, with `bind_err` carrying the code.

**Leg B — two v9 instances, one data dir, one fixed port:**

| Check | Measured |
|---|---|
| First owns the port | `bind_stage 3`, `port 8161`, `pid 13568` |
| Second **observes its own refusal** | `bind_stage 92`, `bind_err 10048`, `port 0`, `pid 18788` |
| The OS attributes the socket | `netstat -ano` owners = **[13568]** only — not the loser |
| One listener answers, and refuses a tokenless request | HTTP **401**, body `{"error":"token"}` |
| The loser still boots | `bridgeReady true`, `bodyChildren 13` |
| Teardown hands the port back | `released true`, `waitedMs 10` |

**Leg C — a foreign `SO_REUSEADDR` squatter holds 8161 first, then v9 starts:**

| Check | Measured |
|---|---|
| Precondition: no process owns the port | owners **[]** |
| The squatter owns it, attributed by pid | owners **[2356]** = the squatter's pid, and no other |
| v9 reports the refusal | `bind_stage 92`, `bind_err 10048`, `pid 8988` |
| v9 survives and boots its window | `bridgeReady true`, `bodyChildren 13` |
| v9 did **not** hijack the port | owners after = **[2356]**, still the squatter |

**The two halves agree on the error codes.** The OS probe predicted 10048 for exclusive-vs-exclusive (leg 3) and for reuse-squatter-first (leg 5); the launched app produced exactly 10048 in both B and C. That is the probe's prediction reproduced inside the real binary, not a restatement of it.

`writer:` [single-instance-spike.mjs](../../_cap-harness/single-instance-spike.mjs) legs B and C, over CDP through `window.__TAURI_INTERNALS__.invoke('sucr_invoke', {channel:'capture:state', args:[]})`. `predicate:` a lock that works must leave exactly one owner attributed by the OS, must let the loser *report* its own refusal, and must not let an exclusive bind displace a squatter that arrived first. `evidence-noun:` `single-instance-report-2026-11-06T20-59-26-089Z.json`, **16/16**, `portOwners` / `ownersAfter` / `capture.bind_stage` / `capture.bind_err`.

The spike channel is reached through the raw door because `window.sucr` is generated from the TS channel list, which the spike does **not** patch — one of the four snapshot sites stays at 35 in the product and 36 only in the scratch tree.

### 3.4 The trap run 1 fell into, and the correction

Run 1 scored **12/13** and its **leg C is VOID**. Instances were killed in `finally` alone, so leg C's "the port is free" precondition ran while **leg B's listener was still alive** — and the predicate was `portTaken`, which cannot say *who* owns the socket. The squatter's "I hold the port" pass was therefore satisfiable by B1's listener, and v9's 10048 could not be attributed to the squatter rather than to B1.

Two fixes, both in the harness: each leg now **releases its own pids and waits for the port to come back** before the next leg asserts on it, and every port assertion names the **owning PID** from `netstat -ano` instead of mere presence. Run 2 is 16/16 with `owners: []` at both preconditions.

New entry for the trap catalogue in [[project-sucr-gate-mechanics]]: *a presence predicate cannot witness attribution — when a leg's claim is "X holds the resource", the witness must name X, and the previous leg must have handed the resource back first.*

### 3.5 What this changes in the design

- **The bind can be the lock.** No mutex, no named pipe, no lock file, no extra crate: `SO_EXCLUSIVEADDRUSE` on a fixed loopback port is sufficient to detect a duplicate, and leg C proves it cannot be displaced by a squatter. The capture listener and the single-instance guard are **one socket**, not two mechanisms.
- **The loser's state is reachable without a listener.** `bind_stage`/`bind_err` are served over the existing IPC door even when the bind failed, so a second instance can know it lost and act. That is the seam a "focus the first instance" reaction would use.
- **Detection is measured; prevention is not implemented.** v9 records the refusal and **keeps booting**. Exiting, focusing the first window, or handing the URL over are reactions, and each is a decision — see § 3.6.
- **Cost:** v9 is **+1 024 B over v2** (4 600 832 → 4 601 856), i.e. **+8 704 B over the shipped exe**, leaving **641 024 B** under the ruled ceiling. That 1 024 B buys the fixed-port string *and* the diagnostic counters; the two are **not separable** in this build, so it is an upper bound on the lock alone, not its price.
- **A refused bind must not be fatal.** Leg C is the common real case — a foreign process on the port — and v9 still booted a working window. Whatever reaction is chosen, "cannot bind" must not mean "cannot download".

### 3.6 What is NOT measured, and the decision it leaves

- **The double-pump hazard.** Boot runs `reclaim_interrupted` + `pump_queue` per instance, but an empty scratch DB makes both a no-op. Measuring it needs a queued job and a fixture. **Not run.** Leg A's `integrity_check = ok` is two *idle* instances and must not be quoted as "two instances are safe".
- **The tray count.** `build_tray` runs per instance — a read-only code fact — so two live processes imply two trays. That is **inferred**, not enumerated.
- **Prevention.** No reaction was implemented, so none was measured.
- **Per-machine vs per-profile.** The socket binds a fixed loopback port with **no data dir in the address**, so the lock is machine-wide: two instances with *different* `--data-dir` values would also collide. This is **inferred from the address shape**, not measured — every leg shared one data dir on purpose. Whether Sucr should allow two profiles at once is a **decision owed**, and it changes the port scheme if the answer is yes.
- **Graceful quit.** The port came back after a **forced** kill (`waitedMs 10`). A graceful quit releasing it follows from process death but was not separately measured.
- **Port choice.** 8161 is a spike default, not a ruling. A real implementation needs a port that is unlikely to be squatted, and leg C says what happens when it is.

The `title` field differs between legs (`"Sucr"` in B, `"Sucr 0.1.0"` in C). That is when the evaluate ran relative to the renderer's own title update, not a fact about the app; `bodyChildren 13` in both is the boot witness.

### 3.7 Artifacts

| Path | What |
|---|---|
| `D:\RE\_cap-harness\exclusive-addr-probe.ps1` | the five OS legs, raw .NET sockets, no app |
| `D:\RE\_cap-harness\exclusive-addr-report-2026-11-06T20-44-31-9063833Z.json` | all five legs + verdict (the 4-leg `exclusive-addr-report.json` is run 1) |
| `D:\RE\_cap-harness\single-instance-spike.mjs` | legs A/B/C, per-leg release, PID attribution |
| `D:\RE\_cap-harness\single-instance-report-2026-11-06T20-59-26-089Z.json` | **run 2, 16/16** — the report of record |
| `D:\RE\_cap-harness\single-instance-report-2026-11-06T20-54-23-493Z.json` | run 1, 12/13 — **leg C void**, kept as the bug's witness |
| `D:\RE\_cap-harness\single-instance-run.log`, `single-instance-run2.log` | the two console runs |
| `D:\RE\_cap-harness\listener-spike\v9-fixed-port.exe` | the launched build, 4 601 856 B |

Cleanup verified after run 2: no `sucr.exe` in `tasklist`, no `LISTENING` on 8161 (only a pid-0 `TIME_WAIT` from the probe fetch), and every `single-A/B/C-*` scratch dir removed. Only pids the harness spawned were killed.

## 4 Spikes 3, 4, 5, 7, 9 — browser mechanics: Chrome half RUN, then PARKED by the user

**Ruling (user, 2026-11-07): park the browser legs and do the Rust spikes.** Every download leg on this box can pop a third-party download manager the user runs, because that product's browser integration is a system-wide network filter (§ 4.5). The Chrome results below stand as measured; the two open items in § 4.4 stay open and are **not** to be closed by inference. No browser leg runs again without a fresh user ruling.

Only Chromium is installed on this box (**Helium 0.20.3.1**, Chromium base **156.0.8078.79**, `C:\Users\Benjamin\AppData\Local\imput\Helium\Application\chrome.exe`). No Firefox, so the Firefox half of spikes 3 and 5 is `blocked` unless a portable extract is made to a scratch dir; it will not be recorded as measured from Chromium behaviour.

Harness: [browser-partial.mjs](../../_cap-harness/browser-partial.mjs) + the scratch MV3 add-on in `D:\RE\_cap-harness\partial-ext\` (`permissions: ["downloads"]`, `host_permissions: ["http://127.0.0.1:8161/*"]`). One headless browser, one scratch `--user-data-dir`, one scratch download dir, own CDP port 9471, own fixture port 8161. The user's Helium profile, Sucr instance and database were never touched. Both legs are driven **through the add-on's command channel**, because that is the product path: the extension owns the download id and only it can `pause()`. CDP navigates and tallies which browser events actually arrive.

Report: `D:\RE\_cap-harness\browser-partial-report-2026-11-06T20-25-23-335Z.json` — **15 of 17 checks pass**, one leg `blocked`. The earlier `…T20-19-14-865Z.json` (1 of 3) is kept: its harness had four bugs, listed in § 4.4.

### 4.1 Spike 3 — the `.crdownload`, Chrome half: mode C is viable, but only via `pause()`

The pause leg crossed 4 MiB and the harness sent `pause` down the add-on's channel. 40 disk samples were taken while the file grew, each read at **8 spread 16 KiB windows** and compared against the deterministic bytes the fixture served.

| Question the draft asks | Answer | Evidence |
|---|---|---|
| contiguous from byte 0, no hole | **yes** | 40 of 40 growing samples `allMatch=true`, sizes 131 072 → 4 194 304 B |
| flushed, size tracks the transfer | **yes** | 40 distinct sizes observed while in flight |
| readable while Chrome holds it open | **yes** | `openSync(file,'r')` succeeded on every in-flight sample; `readableWhileOpen=true` |
| renameable after `cancel()` | **no file to rename** | after `cancel()`: `files=[]`, item `state=interrupted`, `error=USER_CANCELED`, `canResume=false` |
| renameable while **paused** | **yes** | `renameSync` to `…crdownload.renamed-by-sucr` succeeded, then renamed back |
| stops growing when paused | **yes** | 4 194 304 → 4 194 304 B over 2.5 s |

`writer:` this harness. `predicate:` any in-flight sample failing `allMatch`, any `openError`, or a surviving file after `cancel()` would falsify the row it sits on. `evidence-noun:` the report's `legs[1].samples`, `.afterAction`, `.afterCancel`, `.rename`, `.stability`.

**Two consequences for the design, both load-bearing:**

1. **Mode C must `pause()`, never `cancel()`.** `cancel()` deletes the partial. The draft's spike-3 phrasing ("renameable after `cancel()`") presumed a survivor; there is none. If the user cancels in the browser, there is nothing to import and the handoff must fall back to a re-pull.
2. **The file size is ahead of the API's counter.** At the pause, `item.bytesReceived = 3 932 160` while the file on disk was **4 194 304 B** — a 262 144 B gap. An importer that trusts `bytesReceived` as the valid prefix would discard a quarter-megabyte of good bytes. The disk size, verified by sampling, is the authority. This is the same lesson the byte-sample slice taught on Sucr's own side: the stamp is not the bytes.

### 4.2 Spikes 4 and 5 — loopback fetch and the Origin the server sees

- **Spike 4, the operative half: measured.** An MV3 service worker with `host_permissions` for `127.0.0.1` fetched the loopback fixture with **no consent prompt and no failure** — 9 POSTs and 6 long-poll GETs all reached the server. The "is Local Network Access shipped in this build" half is `unmeasured`; it would need `chrome://flags` inspection, and the fetch success does not settle it.
- **Spike 5, Chrome half: measured.** Every report arrived with `Origin: chrome-extension://jomhjepinbiofladdfcflmacmimgfpbj`, `sec-fetch-site: none`, `sec-fetch-mode: cors`, UA `HeadlessChrome/156.0.0.0`. So a loopback listener **can** tell an add-on caller from a page caller by header alone.
- **The byte-for-byte half is `inferred`, not measured.** The measured Origin has **no trailing slash**. Chrome's native-messaging `allowed_origins` format is `chrome-extension://<id>/` **with** one (doc-sourced; no native host was registered on this box, so it was not observed here). If that doc fact holds, the two strings do **not** match byte-for-byte and the listener must normalise before comparing. Spike 2's registration step must measure this against a real manifest rather than inherit the inference.
- **Origin is a hint, not an authentication.** Any local process can set it. Spike 6's token is still owed; nothing here reduces it.

### 4.3 Spike 7 — worker lifetime: measured for one case only

One boot id, `mw5fbxvogcpp`, from the first `worker-boot` report to the closing `pong` (`aliveSince` equal, `boots=1`) across ~96 s and 6 long-polls, each held open 20 s by the fixture. **A continuously pending fetch kept the worker alive for the run's duration.**

`unmeasured`, and it is the case that matters most: **Sucr not running.** Then the poll fails immediately instead of hanging, the worker loops on a 1 s retry, and there is no pending fetch to hold it. Whether it survives that, and whether an alarm must re-arm it, was not exercised. Spike 7 cannot be called closed.

### 4.4 What is open, and the harness bugs already fixed

**Open item 1 — the first leg vanishes. `unseparated`, two candidate causes.** In *both* runs the first navigation produced **no Chrome download item at all**: the second leg received `id: 1` each time. In the second run the fixture served **2** payload GETs (`requests=17` = 6 polls + 9 posts + 2 payloads) while CDP tallied `Browser.downloadWillBegin: 1` and `downloadProgress: 19` — so the bytes left the server but Chrome's download manager never began a download for leg 1. Candidates: (a) the third-party filter driver consumed it (§ 4.5); (b) a first-navigation race in headless Chrome, e.g. `Browser.setDownloadBehavior` not yet propagated, or a user-activation rule on the first automatic download. The position pattern (first leg always the one that vanishes, in two independent runs) fits (b) at least as well as (a). **No verdict.** The separator, when a browser leg is next authorised, is a warm-up non-download navigation before leg 1 plus per-leg file-mtime witnesses on the third-party client's own history files.

**Open item 2 — spike 9 is `unmeasured`.** The harness hardcoded port 8161 in `bg.js`. No discovery path was exercised, so "can the add-on learn the port before any native message" has no evidence either way.

**Open item 3 — Firefox half of 3 and 5: `blocked`.** No Firefox on this box.

Four harness bugs found and fixed between the two runs, recorded so the 1-of-3 report is not mistaken for a browser finding:

1. Extension presence sampled at a fixed 3 s — the worker booted later, so run 1 called it absent. Now waits for the `worker-boot` report, up to 25 s.
2. `reports.length = 0` between legs discarded the boot evidence. Reports are never cleared now; each leg slices from its own offset.
3. The CDP-driven leg waited 20 s for `Browser.downloadWillBegin`, which never came for leg 1, while the add-on's own `downloads.onCreated` fired normally for leg 2. Both legs now run through the add-on.
4. `downloads.onChanged` delivered no `bytesReceived` deltas in this setup — the only deltas observed were `filename`, then `paused`/`canResume`, then `state`/`error`. The worker's threshold-based auto-pause therefore never fired and run 1's download completed. Arming is now opt-in via `?arm=1` and the harness triggers `pause` from **disk size**, which is also the authority per § 4.1.

### 4.5 Why the legs were parked: a third-party filter driver sits below the browser

The user reported that their own commercial download manager was triggered to download during the first run. Probed read-only; no product name, path, identifier or version is reproduced here:

| Fact | Value | How |
|---|---|---|
| Filter driver | **Running**, start **Auto**, a kernel callout driver under `system32\drivers` | `Win32_SystemDriver` + `sc qc` |
| That product's external Chrome extension | one id registered in **both** HKCU and HKLM `Software\Google\Chrome\Extensions`, pointing into its install folder | `reg query` |
| Extensions in the scratch profile | 4 ids: ours, the PDF viewer, two component ones — **not** that product's | `Secure Preferences` → `extensions.settings` |
| Files it wrote inside run 1's window | two of its own history/scheduler files, mtimes `20:19:51.773Z` and `20:20:18.113Z` | `stat` |
| Its tray process | running, **no visible window** at probe time | `tasklist` + `EnumWindows` |

`writer:` this harness, read-only probes. `predicate:` that product's extension id appearing in the scratch profile would falsify "the extension path was not active"; its filter driver being stopped would falsify the network-filter path. `evidence-noun:` the table above.

So the **extension path was not active** in the scratch browser, yet the product reacted during run 1. The running WFP callout driver is the mechanism that reaches a browser with no extension installed in it — it filters below the profile. That makes every download leg on this box a user-visible event in the user's own client, which is why the legs are parked rather than re-run.

**Not done, and not to be done as a side effect:** stopping or disabling the driver, editing that product's exclusion lists, exiting its tray process, or touching the HKCU/HKLM extension registrations. All are the user's app and the user's system.

**Consequence for the capture design, recorded as a premise to be tested rather than a finding:** a competing integrator can sit *below* the add-on. If it takes the transfer, `downloads.onCreated` never fires and Sucr's add-on sees nothing — no handoff, no partial, no error. The draft assumes the add-on sees every download; on a box with such a driver that assumption is falsified in at least one observed instance (cause unseparated, § 4.4). Step 3's listener and step 4's re-pull need a stated behaviour for "the browser never told us", and the Settings state line (ask 3) is where a user would learn that another integrator is winning.

### 4.6 Artifacts

| Path | What |
|---|---|
| `D:\RE\_cap-harness\browser-partial.mjs` | the two-leg harness, disk sampling + add-on command channel |
| `D:\RE\_cap-harness\partial-ext\` | the scratch MV3 add-on (`manifest.json`, `bg.js`) — not product code |
| `D:\RE\_cap-harness\browser-partial-report-2026-11-06T20-25-23-335Z.json` | 15/17, the standing result |
| `D:\RE\_cap-harness\browser-partial-report-2026-11-06T20-19-14-865Z.json` | 1/3, kept as the witness for the four harness bugs |
| `D:\RE\_cap-harness\browser-partial-2026-11-06T20-25-23-335Z\` | scratch profile + download dir, kept via `--keep` |

## 5 Spike 8 — antivirus / SmartScreen

Pending.