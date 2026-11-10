# Settings coverage sweep — the 20 `SETTING_KEYS` against what the gates actually drive

Run 2026-11-05, on the user's "run the settings-coverage sweep". **0 product bytes.** The sweep itself
was an audit of *assertions*, not of behaviour — nothing here said a setting was broken, only which
settings nobody proves. §5 was then executed the same day, at 0 bytes, and §7 is what it measured; the
matrix in §2 is the **post-leg** table and §3 marks which findings the legs closed.

The exe is **4 573 184 B** and its size did not move across this whole slice, but it was **not** untouched:
`stat` at the end reads mtime `2026-11-05T07:26:25Z`, one relink after the four `#[cfg(test)]` tests item 6
added (`ipc.rs:2174`, `ipc.rs:2207` in a file saved `07:22:46Z`; `engine/job/tests.rs:2249`, `:2279` in a file
saved `07:23:14Z`) — the suite went 203 → 207 and the binary stayed 4 573 184 B.
Same size, so item 6's predicted zero
is *measured*, not inferred — and because the link is not byte-reproducible (`~20 B` of PDB GUID / PE
timestamp move at constant size), size and `stat` are the witnesses here, never a hash.
(`predicate:` `stat -c '%s'` before and after every leg, plus `TZ=UTC date -r sucr.exe`; `writer:` this
record.) The size the boot reconcile's +2 048 B and the unattributed ledger's +3 584 B left is the same
4 573 184 B — `design/unattributed-bytes.md:92`, which recorded that build at mtime `06:30:48Z`. An earlier
revision of this file said 4,569,600 B and mtime `04:56:25Z`; those were the pre-boot-reconcile build's size
and a timestamp that did not match any build since, and both are corrected against `stat` here.

## 1. Method, and what each column's predicate is

Five sources, all finite and all read or grepped in full:

| column | predicate (what makes a cell true) | evidence-noun |
| --- | --- | --- |
| **control** | the markup element whose handler calls `saveSetting(key, …)` | `src/renderer/index.html` (`data-set=` / `data-toggle=`), `main.ts:193` (`saveSetting`) and `main.ts:2280` (`renderSettings`) |
| **reader** | a statement that resolves the key's value and branches on it | `db::setting(…)` / `bool_setting(…)` in `src-tauri/src`, or `storedSettings[…]` / `setting('…')` / `settingOn('…')` in `main.ts` |
| **unit** | a `#[test]` that writes the key *and* asserts the reader's result | `grep -rn set_setting\(&…,"KEY"` over `src-tauri/src` |
| **launched** | a gate that runs the built exe and asserts something about this key | `scripts/verify-m0.mjs` (503 lines, 50 `check(` sites, read in full) + the CDP harnesses in `D:\RE\_cap-harness\*.mjs` |

The **launched** column is graded on a ladder, because "the gate mentions it" is not one fact:

- **L0** — the key exists and validates. Nothing above.
- **L1** — a Rust unit test asserts a rule *in isolation* (the function is called with a literal
  argument, not through the stored key).
- **L2** — a launched gate proves the **round trip** (the value lands in SQLite / comes back), usually
  by writing through the bridge rather than clicking.
- **L2c** *(added by the legs)* — a launched harness **clicks the real control** and the stored row and
  the control's own press state agree afterwards. The click→commit edge is proved; the consequence of the
  value is not. This is the half of L3 that finding (3) said every key was missing, and the control sweep
  now gives it to all 14 generic-handler keys plus `ui.theme`.
- **L3** — a launched gate **clicks the real control** and asserts the **consequence**.

The legs also use three witness classes worth naming, because the predicate differs: a **DOM** witness
(`aria-pressed` / `checked` read back after the click), an **OS- or fixture-held** witness (an NTFS
alternate data stream, a staging file's length, how many readers the *server* was serving at once), and an
**app-row** witness (the row's own status, `errorHint`, or a history `kind`). The middle class is the one
that cannot be talked out of by a misleading row: for `motw`, `preallocation` and the concurrency cap the
claim is settled outside the app.

## 2. The matrix

Post-leg. `settings-run` is `D:\RE\_cap-harness\settings-run.mjs`, and a citation there is a `check(` label
whose detail payload § 7 reproduces. All ten of its legs run green in one pass on the shipped exe:
`settings-report-2026-11-05T09-16-42-641Z.json`, **10 legs / 48 checks, `allLegsPass: true`**, exe
4,573,184 B before and after. The bundled gate was run on the same build and passes unchanged
(`npm run verify` → `M0 GATE: PASS`, 50 `check(` sites).

| key | control | reader | unit | launched | level |
| --- | --- | --- | --- | --- | --- |
| `engine.max_active_downloads` | `index.html:377` seg | `ipc.rs:724` | — | settings-run `max_active` — cap pressed to 5 on the control, six rows added, **five `downloading` + one `queued` at 2 007 ms and the fixture itself never served a sixth** (`fixture.maxActive === 5`), then cap→10 and the queued row claims the slot with no Start | **L3** |
| `engine.default_streams` | `index.html:390` seg | `job.rs:1319 stream_band` | `job/tests.rs:2214-2241` `the_band_is_auto_until_a_number_is_named` (auto/4/64/junk) | cap-run `:669` reads it back for the report only; settings-run `max_active` **presses `1`** and the far side then serves one transfer per row (40 concurrent readers at `auto` → 5 at `1`, six rows) | **L3** (width), cap-run report leg unchanged |
| `global_speed_limit` | popover, `#set-cap-btn` → `app.setSpeedLimit` | `lib.rs:410`, written `ipc.rs:997` | `db.rs:1274-1277` round trip | verify-m0 `:361` via the channel; cap-run measures pacing | **L3 by consequence, L2 by control** |
| `engine.auto_start` | `index.html:415` checkbox | `main.ts:4913` gates `download.start` | — | settings-run `auto_start on` / `off` — the box clicked, then the Add bar alone: checked ⇒ `downloading` at **253 ms with no Start pressed**, unchecked ⇒ `queued`, 0 bytes, `startedAt: null` | **L3** |
| `storage.default_folder` | `#set-folder` + Browse + reset | `ipc.rs:611` | — | verify-m0 `:118`/`:141`/`:174` — presets, create-on-request, wizard gate, all proved **on disk** | **L3** (behaviour), key storage unasserted |
| `storage.preallocation` | `index.html:442` seg | `job.rs:902`/`:920` via `reserves_length` | `job/tests.rs:2249` writes the row then calls `reserves_length`; `job/tests.rs:2279` runs `reset_staging` and reads the **file length** back for all three modes | settings-run `prealloc-sparse` / `prealloc-none` — mode pressed on the control, staging `.sucrpart` length sampled over CDP: sparse holds **16 777 216 B while 0 B have verified**; none grows 2 MiB → 14 MiB behind the bytes | **L3** |
| `storage.motw` | `index.html:454` checkbox | `job.rs:3466` (`cfg!(windows)`) | — | settings-run `motw` — box clicked on, published file read with `dir /r`: streams `['Zone.Identifier']` with `[ZoneTransfer] / ZoneId=3 / ReferrerUrl=… / HostUrl=127.0.0.1:8131`; box clicked off, **streams `[]`** | **L3** |
| `storage.auto_verify` | `index.html:464` checkbox | `job.rs:3412` | — | settings-run `auto_verify` — clicked on ⇒ history kinds `auto-verify`, `publish-verifying`; clicked off ⇒ **no kinds**; counterfactual off + supplied digest ⇒ still `verified` (`checksum-ok`), wrong digest ⇒ `needs-attention` / `checksum-mismatch` | **L3** |
| `network.bonding_policy` | `index.html:490` seg | `ipc.rs:1854` → `BondPolicy::from_setting` | `ipc.rs:2174` `the_stored_bonding_policy_is_the_policy_a_new_job_acts_on` — writes the row with `db::set_setting`, then asserts what `bonding_policy(conn)` returns; clearing the row restores the default | sweep clicks the control and the stored row agrees | **L2c** + stored-key unit; the *route-pool consequence* is still unmeasured by a leg |
| `network.metered_guard` | `index.html:502` checkbox | `ipc.rs:1866`, used in route resolution | `ipc.rs:2207` `the_stored_metered_guard_decides_the_pool_the_job_spreads_over` — writes the row, asserts the pool the stored key selects, incl. a direct `'0'` write | sweep clicks the control and the stored row agrees | **L2c** + stored-key unit; consequence unmeasured by a leg |
| `network.timeout_seconds` | `index.html:479` seg | `lib.rs:381-385` → `engine::set_connect_timeout` (`mod.rs:128`) — connect only; reads use the literal `Duration::from_secs(4)` at `mod.rs:203` | — | settings-run `timeout-15` — pressed to 15 on the control; a never-answering host is refused at **15.087 s** with `errorHint "timeout: connect"`; a stalled body is dropped by the engine at **median 4 000 ms, widest 4 001 ms** across 41 abandoned reads, all inside the stored 15 s | **L3** (connect), and the leg *disproves* the read bound — finding (7) |
| `network.proxy_mode` | `index.html:519` seg | `db.rs:277`, `engine/proxy.rs` | `ipc.rs:1360` validation | verify-m0 `:261` **clicks** `button[data-val="manual"]` (`main.ts:6387`), reads the engine's answer, restores; proxy-run, auth-run | **L3** |
| `network.proxy_url` | `#set-proxy-url` | `db.rs:278`, `proxy.rs:50` | `ipc.rs:1363` | verify-m0 `:265` typed, engine echoes `manual 127.0.0.1:8199`; `:290` cleared | **L3** |
| `network.proxy_bypass` | `#set-proxy-bypass` | `db.rs:279` | `ipc.rs:1372` | verify-m0 `:266` `bypassStored === 'localhost'` | **L3** |
| `ui.theme` | theme buttons `main.ts:32`, handler `:60`; wizard `:5837` | `main.ts:179`, `:5836` (renderer-only) | — | sweep clicks a `data-theme-pref` value, the stored row and the pressed button agree | **L2c** — the repaint it drives is visible to a person, asserted by nothing |
| `ui.meter_style` | `index.html:639` seg | `main.ts:181`, `:5841` (renderer-only) | — | sweep clicks it, stored row agrees | **L2c** — consequence unasserted |
| `ui.motion` | `index.html:649` seg | renderer + `data-motion` attribute | — | verify-m0 `:375` asserts a real computed transition in 3 states, **bridge-written** (`main.ts:6532-6536`); sweep clicks the control itself | **L3 consequence, L2c control** |
| `ui.sound_on_complete` | `index.html:661` checkbox | `main.ts:2938`, `:5847` (renderer-only) | — | chime-run `:351` helper + `:637` **clicks the box**, leg 5 measures 0 notes off / 2 notes on | **L3** |
| `ui.close_to_tray` | `index.html:670` seg | `lib.rs:248` (key read `:257`) | — | sweep clicks all three values and the rows agree, `close_quits` clicks the real seg-buttons for both branches, and `close_minimize` / `close_quit_ends` / `quit_hours` read the **process** outcome from outside the app on the shipped exe — `tasklist` presence, the window manager, the instance's own CDP socket, and the fixture's request and byte counters | **L3**, finding (6) closed 2026-11-06 |
| `system.setup_completed` | wizard finish `main.ts:252-254`/`:5898` | `main.ts:245` first-run test | `ipc.rs:2387-2389` | verify-m0 `:428` first run + wizard in DOM; reset-all leg | **L2/L3** |

**Locator drift in this matrix, measured 2026-11-06 rather than recalled.** The download window's panel was
inserted at `index.html:425-447`, so **every `index.html` locator below it in this table is 32 lines lower
today** and each one above it is unchanged. Re-read individually and confirmed at the new numbers:
`storage.preallocation` **474**, `storage.motw` **486**, `storage.auto_verify` **496**,
`network.timeout_seconds` **511**, `network.bonding_policy` **522**, `network.metered_guard` **534**,
`network.proxy_mode` **551**, `ui.meter_style` **671**, `ui.motion` **681**, `ui.sound_on_complete` **693**,
`ui.close_to_tray` **702** — and `engine.auto_start` **415**, above the insert, still exact. The cells keep the
numbers the sweep actually saw, because that is what was measured; this line is the correction.
Treat a bare `main.ts:`/`ipc.rs:` number anywhere in this file the same way: the function name is the stable
locator, the line is not, and `grep -n "fn <name>"` / `grep -n "<id>"` settles it in one call.


## 3. Findings

Each finding below keeps the sentence the sweep wrote, then says what the legs did to it. The evidence
for every closure is the same report: `settings-report-2026-11-05T09-16-42-641Z.json`, 10 legs / 48 checks
/ `allLegsPass: true`, exe 4 573 184 B before and after.

**(1) Eight keys were at L0: stored, validated, read by live code, and asserted by nobody.** *CLOSED — all
eight are now L3 or L2c; the eighth, `ui.close_to_tray`, reached **L3** on 2026-11-06 through finding (6).*
`engine.auto_start`, `storage.preallocation`, `storage.motw`, `storage.auto_verify`,
`network.timeout_seconds`, `ui.theme`, `ui.meter_style`, `ui.close_to_tray`.
`writer:` this audit. `predicate:` the key literal appears in no `#[test]` write, no `check(` site in
`verify-m0.mjs` and no CDP harness file, *and* no harness clicks its `data-set`/`data-toggle` control.
Each still has a reader (column 3), so these are not dead keys — they are **unproven** keys. The two
with a real-world effect outside the app are the ones worth pricing first: `storage.motw` (whether a
downloaded file carries a Zone.Identifier is a fact the *file system* holds) and `ui.close_to_tray`
(whether closing the last window ends the process is a fact `tasklist` holds).
**Now proven:** `motw` on the *file* — `streams: ['Zone.Identifier']` with
`[ZoneTransfer] / ZoneId=3 / ReferrerUrl=http://127.0.0.1:8131/page.html / HostUrl=127.0.0.1:8131`, and
`streams: []` with the box clicked off; `preallocation` on the *directory* — sparse holds
16 777 216 B while 0 B have verified, none grows behind the bytes; `auto_verify` on the history kinds;
`auto_start` on the status transition; `timeout_seconds` on the connect refusal *and* on the read path it
does **not** bound (see (7)); `ui.theme` and `ui.meter_style` on the click→row edge (L2c); and
`ui.close_to_tray` on the **process** itself — `tasklist`, the window manager, the instance's own CDP socket
and the fixture's counters (L3, finding (6)).
`evidence-noun:` the check details in § 7.

**(2) Two keys were L1 where the L1 looks like L3.** *CLOSED — both tests now write the row.*
`network.bonding_policy` and `network.metered_guard` each had a named test calling the *pure* thing
(`from_setting("hedging")`, `resolve_routes(…, true)`) with a literal argument. Neither test wrote the
settings row, so the stored-key→rule edge (`ipc.rs:1854`, `ipc.rs:1866`) had no assertion at all, and a
regression that made `bonding_policy(conn)` ignore the row would have kept both green.
`ipc.rs:2174 the_stored_bonding_policy_is_the_policy_a_new_job_acts_on` and `ipc.rs:2207
the_stored_metered_guard_decides_the_pool_the_job_spreads_over` now call `db::set_setting` first and assert
what the *conn*-taking reader returns — including clearing the row back to the default and one direct `'0'`
write. `predicate:` each test writes the key literal and then asserts on `bonding_policy(&conn)` /
`metered_guard(&conn)`, not on the pure function. `evidence-noun:` `cargo test --release` on this build →
**207 passed / 6 ignored** (`_cap-harness/cargo-test-settings.log`). § 5 item 6's predicted zero is
measured, not inferred: the exe was relinked after the tests landed and `stat` still reads
**4 573 184 B** (see the header). **What remains:** the *route-pool
consequence* of the stored key is still not measured by a launched leg; both keys are L2c + a stored-key
unit test, not L3.

**(3) Three keys were asserted through the bridge, never through their control.** *CLOSED — the click→commit
edge is now proved for every generic-handler key, which is what this finding actually asked for.*
`engine.max_active_downloads` (`main.ts:6515` calls `settings.set`), `ui.motion`
(`main.ts:6532-6536`), `storage.preallocation` (reboot-run `:391` over CDP). The round trip was proved;
**the click→commit handler was not**, for every seg/checkbox except `network.proxy_mode`. This is the
same shape as the step-8 reset-all defect: a control whose handler is wrong still round-trips through
the bridge, because the bridge is what the gate uses.
`predicate:` of the **14** keys that write through the generic handlers
(`grep -oE 'data-(set|toggle)="key"' index.html | sort -u` → 14; § 4 lists the other 6, each with its own
selector), exactly one — `network.proxy_mode` — was clicked by any gate. `evidence-noun:`
`grep -rn 'seg-control|data-toggle'` over the harness dir returned 4 hits, 3 proxy and the chime's
parameterised helper.
**Now:** the sweep leg presses **57** controls (every value of every `data-set` seg, every `data-toggle`
box, every `[data-theme-pref]` button) in the shipped exe and asserts two things per press — the stored row
equals the pressed value and the control's own `aria-pressed`/`checked` agrees — with
`failed: []` and `disagree: []`. Fifteen of the twenty keys had *no row at all* before the sweep
(`rowsCreatedFromNothing`), so each press is a real click writing a real row, not a restore. That is the
L2c level in § 1, and it closes the bridge-vs-control asymmetry finding (3) named.

**(4) `engine.max_active_downloads` never had its *consequence* observed.** *CLOSED — L3.* The gate asserted
the string `'5'` landed in SQLite (a bridge write at `main.ts:6515`); no gate added six rows and counted
five running, so enforcement of the queue depth was `unmeasured` — the word the admission rule reserves for
a claim that cannot lead the queue.
**Now measured, with the control group actually different from the test group:** cap pressed to `5` on the
seg, six rows added, sampled every 500 ms — `{queued:6}` → `2/4` → `3/3` → `4/2` → `{downloading:5,
queued:1}` at **2 007 ms**, peak `5`, and the *fixture's* own high-water mark of concurrent paced responses
was `5` (`fixture.maxActive === 5`), so the sixth row was refused by the app, not by the server. Cap→`10`
and the queued row claimed the freed slot with no Start pressed (`ms: 755`). `evidence-noun:` the
`max_active_downloads` leg's five samples plus its `fixture` block in § 7.
**One honest remainder, recorded not asserted:** cap→`1` with six rows already running left
`{downloading: 6}` — lowering the ceiling gates *claiming*, not running. That is the engine's existing
shape (the cap is read at claim time in `download_start`, `ipc.rs:724`); it is noted here so nobody later
reports it as a bug, and it is not an assertion about design intent.

**(5) The proxy trio was the template, and it was the only trio that was.** Written by a real click, read
back from the engine's own answer, refused in a named tone, cleared again so nothing leaks to the next
leg. Every L0/L1 row above was a request to be treated like these three — and `motw`, `auto_verify`,
`auto_start`, `preallocation` and `max_active_downloads` now are: each is clicked, each is witnessed
outside the app or in its own row, each restores before the next leg.

**(6) The ladder was not reachable for `ui.close_to_tray` by design — CLOSED 2026-11-06, 0 product bytes.**
`--smoke` returns before the tray is built: `setup()` tests `app.state::<AppState>().smoke` at `lib.rs:464`
and returns `Ok(())` at `:465`, ahead of the boot reconcile (`reclaim_interrupted`, wired at `:474`),
`watch_network_changes` (`:486`) and `build_tray` (`:487`), so the bundled gate structurally cannot assert
this key. The reader is `close_hides_to_tray` (`lib.rs:248`): the `TRAY_READY` guard at `:249`, the key read
at `:257`, the `!= "false"` test at `:260`, and `TRAY_READY` stored only inside `build_tray` at `:306`. The
writers are `on_window_event` (`lib.rs:489`) — `CloseRequested` → `prevent_close()` + `window.hide()` at
`:491`, `Destroyed` → `app.exit(0)` at `:501`-`:504`.
*Locator drift, re-read again 2026-11-06:* these lines were 429/430, 451/452, 457/468 and 244 when this sweep
was written and are 464/465, 474, 486/487 and 248 today; the download window's wiring moved them, which is
why each carries the function name beside the number.
**Ruling 9's witness is a different witness** ([download-window.md](download-window.md) § 5, `close_quits`):
a launched leg clicks the real Appearance seg-buttons and `aria-pressed`, the stored boolean and the window's
own sentence move together — the *renderer* half, **L2c**.

**What closes it** — three new legs in `window-run.mjs`, report
`window-report-2026-11-06T00-27-38-759Z.json`, **13 legs / 134 checks `allLegsPass: true`** on the shipped
4 579 328 B exe:
- **`close_minimize`, 14 checks** — the default branch, which is also where a fresh profile sits because
  migrate seeds no row (`stored: null`). A row capped to 256 KiB/s and Started, then `WM_CLOSE` posted to
  *this instance's* own window through `chime-window.ps1`: the window reads `visible=False iconic=False`
  (hidden, not minimized), `tasklist` still lists `sucr.exe` — pid 26 176, 32 200 K — the *same* CDP target
  answers `1 + 1 === 2`, a fresh attach finds `visibilityState === 'visible'`, `settings.list()` still
  returns the sentinel this leg wrote minutes earlier, and the transfer keeps arriving: 2 097 153 B served
  before the close, 6 815 745 B after, then `finished` at 25 901 ms with all 8 388 608 B on disk. `SW_SHOW`
  on the same hwnd returns `visible=True`, the internal control that says the leg moved the app's own handle.
- **`close_quit_ends`, 7 checks** — the real seg-button pressed to **Quit** (`aria-pressed=true`, `'false'`
  stored). From the `WM_CLOSE` post to Node's own `exit` event: **1 935 ms**, `code 0`, `signal null`, pid
  gone 179 ms later; the helper then reports `NO_TAURI_WINDOW_FOR_PID` (destroyed, not hidden), the
  instance's CDP port is free and its socket errors. Nothing killed it — the harness subscribes
  `child.once('exit')` *before* it posts the close.
- **`quit_hours_never_open`, 16 checks** — the consequence the Settings sentence prints, followed across a
  process boundary. What it found is a product limit, not a passing assertion: see
  [download-window.md](download-window.md) § 7 and that record's § 5.

**Why the two parts this finding called hard turned out to be ordinary.** A leg survives its own window
closing by not waiting on the page: the process is watched through the child handle, and the window manager,
`tasklist` and the port are read afterwards. The cleanup blocker — `close()` EPERMs on a resident
`sucr.exe`'s WebView2 dir — is sidestepped by `close({dirs:'keep'})` plus `boot({reuseDirs:true})` for the
hand-off, and in the Quit branch there is nothing resident left to kill. The 24 orphans that afternoon were
why a run must be watched, not why the assertion was impossible.

**Status:** the sweep clicks all three values and the rows agree; the process-outcome half is now **measured
from outside the app**, on the launched exe, for both branches, and `ui.close_to_tray` is **L3** in § 2.

**(7) A new finding, produced by measurement rather than by the audit: `network.timeout_seconds` does not
bound the read path.** *This is the one place a leg disproved a claim the record would otherwise have made.*
`writer:` the `timeout-15` leg. `predicate:` the fixture times each request it abandons, so the witness is
server-side and cannot be explained by a row's retry budget.
With `network.timeout_seconds` pressed to `15` and a body that stalls for 25 s at 64 KiB/s, the engine
abandoned **41** reads, first at 1 ms, **median 4 000 ms, widest 4 001 ms** — a fixed ~4 s read bound that
is the engine's own, not the stored one; the row's total elapsed was 25 130 ms because that is a *retry
budget*, not one timer. The same key does bound **connect**: a never-answering host is refused at
**15.087 s** with `errorHint "timeout: connect"`. `evidence-noun:` the leg's detail block, quoted in § 7.
**Consequence for the project's language:** do not write that this setting controls download timeouts,
period. It controls connect timeouts and is documented by the leg that says so. **The shipped copy already
passes that test** — the row reads `Socket timeout` / "Connection handshake limit; a stalled attempt retries."
(`index.html:475-476`, seg at `:479`), which is precisely the measured shape, including the retry part that
made the row's elapsed a budget rather than a timer. So the finding is a *coverage* gain, not a wording fix:
the sentence nobody could prove is now one a leg proves, and no renderer byte was owed.
**The code agrees, which makes this two independent witnesses:** `lib.rs:381-385` reads the key and hands it
to `engine::set_connect_timeout` and to nothing else, while `engine/mod.rs:203` builds every agent with
`.timeout_per_read(Some(Duration::from_secs(4)))` — a literal. So the ~4 s the fixture measured is the
engine's own per-read bound, and `network.timeout_seconds` cannot reach the read path as written. Whether
that literal *should* follow the key is a behaviour change and therefore the user's ruling; recorded here
as a held fact, not fixed.

## 4. What the sweep did *not* find

- **No dead key.** All 20 have at least one reader that branches on the value
  (`predicate:` column 3 of the matrix, every cell non-empty). The suspicion that a settings row could
  be write-only is answered as false for all twenty.
- **No missing control.** Every key has a surface a person can reach. Fourteen write through the
  generic handlers (`data-set` seg-controls and `data-toggle` checkboxes in `index.html`); the other
  six have their own selector: `ui.theme` (`data-theme-pref` buttons, `index.html:146-152`, plus the
  wizard's `#wizard-theme-seg`), `global_speed_limit` (`#set-cap-btn` popover), `storage.default_folder`
  (`#set-folder` + Browse + reset), `network.proxy_url` and `network.proxy_bypass` (their own boxes),
  and `system.setup_completed` (written by the wizard's finish, never by a switch).
- **Nothing broken, with one exception that is not a defect but is not "fine" either.** This audit read no
  runtime behaviour. An L0 row meant *no assertion*, not a broken setting — and the legs confirmed that for
  every key they ran: no leg found a control that failed to store, a value the reader ignored, or a setting
  whose effect was wrong. The single thing measurement *did* change is the claim about
  `network.timeout_seconds`: the key works exactly as wired (connect), and the surprise is that the read
  path was never its to govern (finding (7)). That is a documentation and expectation gap, not a malfunction
  — and whether the ~4 s literal at `engine/mod.rs:203` should follow the key is a behaviour change, so it
  belongs to the user, not to this sweep.

## 5. Cheapest legs, with their real byte cost named

The distinction that matters: **`verify-m0.mjs`'s driver lives in the bundled renderer** — the smoke
block in `main.ts` ships inside the exe, so a new smoke leg costs bytes (measured band for a small
renderer-behaviour slice: ~2 KB). **A CDP harness in `D:\RE\_cap-harness` costs 0 bytes** and runs the
shipped exe, which is the standard this project already holds ("verify in the shipped binary").

So, in the order that buys the most assertion per byte. Each item now carries its outcome; the legs all ran
on the shipped exe and none of them added a byte to it.

1. **A control-sweep leg in a CDP harness — 0 B. SHIPPED.** Open Settings in the launched exe, click each
   `.seg-control[data-set]` value and each `input[data-toggle]` once, and assert `settings.list()`
   changed to exactly that value and the DOM reflects it, then restore. One leg lifts the 14
   generic-handler keys from "the bridge round-trips" to "the control commits" — finding (3)'s edge —
   and adding `[data-theme-pref]` plus the `#set-cap-btn` popover extends it to all 20.
   *Done:* the `sweep` leg presses **57** controls, `failed: []`, `disagree: []`; `ui.theme` is included,
   the `#set-cap-btn` popover is not swept (it needs a typed value, and `global_speed_limit` was already
   L3 by consequence through cap-run, so the extension was not worth the flake).
2. **`storage.motw` — 0 B. SHIPPED.** Download from the fixture with the box on, then off; assert on the
   **file**, not the app, whether `Zone.Identifier` is present. The witness is OS-held, the same class
   reboot-run already trusts. *Done:* `streams ['Zone.Identifier']` with a `[ZoneTransfer]` body naming the
   referrer, vs `streams []` off.
3. **`ui.close_to_tray` — 0 B. SHIPPED 2026-11-06, and it closes finding (6).** Launch normally with each
   value, close the last window, assert `tasklist` presence. *How it actually went:* three legs —
   `close_minimize`, `close_quit_ends`, `quit_hours_never_open` — 37 checks between them, all green in the
   full 13-leg / 134-check run. The predicted `tasklist` assertion is one of them; the others read the window
   manager, the instance's own CDP socket and the fixture's served-byte counter, because a process that is
   alive proves nothing unless the page behind its hidden window is also alive. The cleanup the deferral
   named as the blocker is the reason the hand-off leg exists at all: `close({dirs:'keep'})` then
   `boot({reuseDirs:true})`, and the Quit branch needs no cleanup because the app did it.
4. **`engine.auto_start` + `engine.max_active_downloads` — 0 B. SHIPPED.** Add rows and count status
   transitions: a row that goes `queued → downloading` with no click proves (1)'s first key; six rows with
   the cap at 5 and five running proves finding (4). *Done:* `downloading` at 253 ms with the box checked;
   `queued`, 0 B, `startedAt: null` with it unchecked; and the six-row/5-slot leg with the fixture's own
   `maxActive === 5` as the outside witness.
5. **`storage.preallocation`, `storage.auto_verify`, `network.timeout_seconds` — 0 B. SHIPPED** each, with
   the fixture-side witnesses the item predicted (file length, a checksum row's transition, a hung server
   measured at 15 s). The timeout leg is the one that turned into a *disproof* — see finding (7).
6. **The two L1→L3 gaps (`bonding_policy`, `metered_guard`) — SHIPPED as a Rust test** that writes the
   settings row and calls `bonding_policy(&conn)` / `metered_guard(&conn)` rather than the pure function,
   plus the same treatment for `storage.preallocation` (`engine/job/tests.rs:2249` and `:2279`, which reads
   the staging **file length** back). Test-only code: `#[cfg(test)]` does not link into the bin, and the
   predicted **0 B** was then measured by `stat` after a relink rather than left as an inference.

Items 1-5 spent zero product bytes and item 6 spent zero and was verified to. None of them changed a
user-visible behaviour, so none needed a ruling — but they expand what the project may claim, and per the
admission rule every claim below cites the harness that earned it.

## 6. Hygiene

Each harness leg runs on its own `--data-dir`, its own WebView2 profile dir, its own out dir and its own CDP
port, against a local fixture on `127.0.0.1:8131`; the user's own instance, profile, window and library were
never opened or read. No product source outside `#[cfg(test)]` was touched, and the bundled gate was not
modified — `verify-m0.mjs` is still 503 lines / **50 `check(` sites** and re-passed on this build
(`M0 GATE: PASS`, `_cap-harness/m0-settings.log`), and the Rust suite is **207 passed / 6 ignored**.

Two housekeeping facts worth recording, because they are the shape of the cost of running ten launched legs:

- **24 orphaned harness `sucr.exe` processes** were left by the earlier partial runs and stopped with
  `kill-harness-sucr.ps1`, which filters on `--data-dir` containing `_cap-harness` — so it cannot name the
  user's instance, and no tray-resident process of theirs was touched. The orphans come from each leg's
  `close()` hitting `EPERM` on the WebView2 dir while the process is still resident; the dirs are kept for
  the same reason.
- **The `settings-run-*` dirs** (data + out, 16 MiB fixture copies) **were deleted after the final pass** —
  37 of them, 37 removed, 0 left, the harness dir back from 2.2 GB to 61 MB. The report JSON and the logs are
  kept, because § 7 quotes them.

## 7. What the legs measured

`writer:` `D:\RE\_cap-harness\settings-run.mjs`, run 2026-11-05, report
`settings-report-2026-11-05T09-16-42-641Z.json` — **10 legs / 48 checks, `allLegsPass: true`**,
`bytes.exe = 4573184` at both ends of the run, `startedAt 09:16:42.664Z` → `finishedAt 09:22:10.372Z`
(5 m 28 s for ten launched boots). `fixture:` `{port: 8131, size: 16 777 216, rate: 4 194 304 B/s}`.
`predicate:` each bullet is a `check(` label's `detail` object, reproduced verbatim below — not a summary of
one. `evidence-noun:` the report file.

**The sweep (finding (3)'s closure).** 57 presses, `failed: []`, `disagree: []`. `rowsCreatedFromNothing`
lists **15** of the 20 keys — `engine.auto_start`, `engine.default_streams`, `engine.max_active_downloads`,
`network.bonding_policy`, `network.metered_guard`, `network.proxy_mode`, `network.timeout_seconds`,
`storage.auto_verify`, `storage.motw`, `storage.preallocation`, `ui.close_to_tray`, `ui.meter_style`,
`ui.motion`, `ui.sound_on_complete`, `ui.theme` — i.e. on a fresh `--data-dir` a first click on each control
creates the row; only `system.setup_completed` was already there (the wizard). Every press is therefore
click → handler → `settings.set` → row → re-read, with the DOM's `aria-pressed`/`checked` agreeing in the
same breath.

**`engine.auto_start` (finding (1)).** Box checked, one row from the Add bar, no Start:
`{status: 'downloading', ms: 253, streamsLive: 0}`. Box unchecked, same add:
`{status: 'queued', verifiedBytes: 0, startedAt: null}`. The reader is `main.ts:4913`.

**`engine.max_active_downloads` (finding (4)).** `engine.auto_start` pressed off first so a Start is the
only thing that launches a row, cap pressed to `5` on the seg, `engine.default_streams` pressed to `1`
(see the trap below), six rows added, sampled every 500 ms:

| t (ms) | statuses |
| --- | --- |
| 1 | `queued: 6` |
| 502 | `downloading: 2`, `queued: 4` |
| 1004 | `downloading: 3`, `queued: 3` |
| 1505 | `downloading: 4`, `queued: 2` |
| 2007 | `downloading: 5`, `queued: 1` |

`peakDownloading: 5`, `filledAtMs: 2007`, and the *fixture's* `maxActive: 5` across `requests: 9` — the
sixth row never reached the server, so the ceiling is the app's. Cap→`10` and the queued row claimed the
slot with no Start at `ms: 755`. Cap→`1` left `{downloading: 6}`: the cap gates **claiming**, not running.
**The trap this leg paid for:** at the shipped `auto` width the stream ramp opens 8+ streams per job, so six
rows put ~40 concurrent paced responses on one fixture and the reads starved past the engine's ~4 s read
bound, producing `{downloading: 5, needs-attention: 1}`. That looked like an app defect and was a harness
defect; pinning `engine.default_streams` to `1` made the six rows six transfers and the witness clean. A
leg that cannot tell those apart cannot accuse the app.

**`storage.motw` (finding (1), OS-held).** Box clicked on → published file read with `dir /r`:
`streams: ['Zone.Identifier']`, body `[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=http://127.0.0.1:8131/page.html\r\nHostUrl=127.0.0.1:8131`.
Box clicked off → `streams: []`, `ads: null`. The witness is NTFS's, not the app's; reader `job.rs:3466`.

**`storage.auto_verify` (finding (1), app-row + counterfactuals).** Checked: a plain row records
`kinds: ['auto-verify', 'publish-verifying']`. Unchecked: `kinds: []` — the auto digest is not taken.
Counterfactuals proving the box is not load-bearing for *integrity*: unchecked **with** a supplied digest
still ends `verified` (`['checksum-ok', 'publish-verifying']`), and a wrong digest still ends
`needs-attention` / `checksum-mismatch`. Reader `job.rs:3412`.

**`storage.preallocation` (finding (1), OS-held).** Mode pressed on the seg, `engine.default_streams`
pinned to `1` so the growing mode really grows, staging `.sucrpart` length sampled over CDP.
`sparse`: `{len: 16 777 216, verified: 0}` at the first sample — the whole length is reserved before a byte
arrives — then `verified` climbs 1 179 648 → 12 713 984 while `len` never moves.
`none`: `{len: 2 097 152, verified: 0}` at the same moment, i.e. the file grows behind the writes.
Both publish at the full 16 777 216 B, which is the point: the difference is only in the staging file.
Readers `job.rs:900`/`:902`/`:920`; the same three modes are now also covered by `engine/job/tests.rs:2249`
and `:2279`.

**`network.timeout_seconds` (finding (7) — the disproof).** Key pressed to `15` on the seg.
*Connect:* a host that never answers is refused at `{elapsed: 15.087 s, status: 'needs-attention',
errorCode: 'http', hint: 'timeout: connect'}` — the stored value, not a fixed one, and the row names which
timer ran out.
*Read:* a body that stalls 25 s at 64 KiB/s. The fixture times each request it **abandons**, so this is
server-side: `requestsSeen: 41`, `abandonedReads: 41`, `firstAbandonMs: 1`, `medianAbandonMs: 4000`,
`widestAbandonMs: 4001`. Every read was dropped at the engine's own ~4 s bound, all of them inside the
stored 15 s. The row's total `{ms: 25 130, errorCode: 'block-failed', hint: '655360 of 16777216 bytes landed;
the rest kept failing'}` is a **retry budget**, not one timer.
**What this changed in the project's language:** the earlier draft of this record's timeout row claimed the
key bounds reads. It does not. `writer:` the leg; `predicate:` the abandons are timed by the fixture, not by
the app; `evidence-noun:` the block above.

**What the first version of this leg got wrong, and why it is here:** the original predicate was
`Date.now() - t0 < secs * 1000` against the *row's* elapsed. An idle row at 25 157 ms failed it, which read
like a product defect and was a void test — a row's elapsed spans retries, so it cannot witness a per-read
bound. The rewritten predicate measures the far side of the socket. Same lesson as the `max_active` trap
above and the two harness-witness traps already in memory: **a measurement whose control group equals its
test group is void, and a row is not an outside witness.**

## 8. Addendum 2026-11-06 — the surface is 24 keys now, not 20

The download window grew `SETTING_KEYS` from 20 to 24 ([ipc.rs:1026](../app/src-tauri/src/ipc.rs), the four
window keys at :1031-1034), so every count in this record — the title, §2's matrix, §5's "all 20 have a
reader" — describes the **pre-window** surface and is not wrong about what it measured. The four new keys are
covered by their own record and their own launched suite, at the same or a higher rung than this sweep reached:
each of the four is exercised at its own control — the switch is clicked, the two time boxes are written and
dispatch `change`, and the chips are clicked one bit at a time (with one recorded exception: `setTime` falls
back to a bridge write when the stored value already equals what the leg wants, because the page's own commit
returns early on an unchanged box). The consequence is then asserted outside the app — fixture hit counts,
bytes on disk, a hold spanning two watcher passes, a release no settings write caused — which is **L3** in §1's
ladder. See [download-window.md](download-window.md) §5: the suite is now **13 legs / 134 checks
`allLegsPass: true`** on the 4 579 328 B exe
(`window-report-2026-11-06T00-27-38-759Z.json`; it was 10 legs / 97 checks in
`window-report-2026-11-05T23-16-18-936Z.json` when this addendum was first written). `close_quits` clicks the
real Window close action seg-buttons and reads the consequence back out of both surfaces — a **L2c** leg — and
the three legs added on 2026-11-06 (`close_minimize`, `close_quit_ends`, `quit_hours_never_open`) carry the
same control into the process itself, which is what finding (6) had deferred. `writer:` this addendum, `predicate:` the `check(` labels in the window report and the `check(` labels
in this sweep's own report, `evidence-noun:` the leg names in the download-window table.
