# Boot reconcile — transfers the machine stopped under

Record of the slice that made "transfers survive a reboot" true. Written 2026-11-05 under the user's
delegation ("Do as you see fit"), continuing the held-defect queue. Everything here is either cited to
a file:line, counted, or read out of a launched-binary report.

## 1. Why this slice

`project-sucr-open-defects.md` had held this at the front of the queue: the autostart and schedule
concepts both depend on a transfer surviving a reboot, and one of the queue's premises was that it did
not. The defect was structural, not cosmetic — see §2.

## 2. What was true before this change (`writer:` grep with locators, plus the launched counterfactual below)

- `downloading` is written by exactly one statement in the whole crate:
  [engine/job.rs:526](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L526), inside `job::run`, in the
  same statement that clears `error_code`/`error_hint`. `grep -rn "SET status = 'downloading'"` returns
  that line plus two test files. **A `downloading` row is therefore proof that someone pressed Start.**
- `AppState` was built with an empty engine registry (`job::Registry`), and nothing at boot looked at
  the store. A reboot kills the threads; the row keeps saying `downloading` forever.
- The consequence at the surface: the row offers **Pause** (the action for a live job) when the correct
  action is **Resume** (the action that would bring the job back), and `streamsLive` is null — a status
  claiming an activity that has stopped.
- Nothing else was broken by it. Resume from the row still worked, because the row carried its own
  `blocks.bytes_done` and its staging file. So this was a truth-and-affordance defect, not a data-loss
  defect. That distinction is why it cost 2 KB and not 20 KB.

## 3. The mechanism

[ipc.rs:806 `reclaim_interrupted`](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L806)
(written at `:798`; re-read 2026-11-06):

1. `SELECT id FROM downloads WHERE status = 'downloading'` — only those rows, ordered by id.
2. `UPDATE downloads SET status = 'queued' WHERE status = 'downloading'`
   ([ipc.rs:816](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L816)).
3. `drop(db)` **before** claiming any slot. Required: the queue re-reads the row it is about to launch
   and skips anything that is not `queued`, so an uncommitted write would make the reclaim a no-op.
4. For each id: [`engine.push_pending`](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3804).
5. The caller then runs [ipc.rs:837 `pump_queue`](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L837)
   (`:829` when this was written),
   which was widened from private to `pub(crate)` and whose body is unchanged.

Wired at [lib.rs:472-485](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L472)
(`:437-450` when this was written), inside `.setup()`, below
the `--smoke` early return at [lib.rs:464](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L464) (`:429`) and above
`watch_network_changes`. The guard is in a nested block so the `State` borrow drops before the tray.
The result is `unwrap_or_default()`ed: a reconcile that cannot read the store must not stop the app
from booting; the rows are then simply left as they were, and Resume still works.

`pump_queue` already respects `max_active`, already re-reads each row's status, and already bails while
`bond_routes` is empty — so a reclaimed row with no route up stays `queued` and pending, which is
exactly what the manual path does today. No transient condition becomes a permanent `needs-attention`.

### Reused, not rewritten

No new IPC channel, no new setting key, no schema version, no renderer surface, no new engine type.
Which is why no ratchet site moved: `CHANNELS` still 35, setting keys still 20, tables still 16, the
M0 gate's `check(` sites still 50.

## 4. Rulings — CHOSEN, not `DECIDED`

These are delegated answers under the same rule as Feature 1's four rulings and the proxy slice's Q1-Q6:
this agent chose them, and this agent may reverse them. Only the user writes `DECIDED`.

- **R1 — scope the scan to `downloading`.** Chosen because §2 makes that status a deliberate Start.
  `queued`, `paused` and `needs-attention` rows stay their user's to launch; auto-launching them would
  turn a crash into traffic the user never asked for.
- **R2 — reclaim into the queue, do not launch directly.** The queue is the only thing that knows
  `max_active`, the route set and the per-row status. Reimplementing that at boot would drift.
- **R3 — commit before claiming.** See step 3 above; without the `drop`, the queue skips every row.
- **R4 — swallow a store failure rather than failing the boot.** A reconcile is a convenience; a window
  that never opens is not. The cost is that a broken store makes the reclaim silent.
- **R5 — run it before the first paint**, so no window ever shows a row claiming a stopped activity.

## 5. Gates, and the gap the gates cannot cover

- `cargo test --release` in `src-tauri`: **201 passed, 6 ignored** (was 196 + 5 new). The new ones:
  `a_transfer_the_machine_stopped_under_is_queued_for_a_slot` (ipc.rs:3018),
  `the_reclaim_collects_only_the_rows_a_start_ever_touched` (:3049),
  `an_empty_store_gives_the_queue_nothing_to_launch` (:3080),
  `a_second_reclaim_finds_nothing_the_first_moved` (:3097),
  `a_store_that_cannot_be_read_is_reported_and_launches_nothing` (:3123) — the last on a schema-less
  in-memory connection, asserting `is_err()`, `pop_pending()` → `None`, `running_count()` → 0.
  Helpers `set_status` (:2996) and `status_of` (:3005).
  *These were 2863/2894/2925/2942/2968 and 2841/2850 on the build this record shipped against; the unattributed
  ledger, the settings-coverage tests and the download window — all written after it — pushed them down by
  exactly 155 lines (every one of the seven moved by the same amount, which is how the shift was checked). Re-read with `grep -n "fn <name>"` before
  citing them again — the names are the stable locator, the numbers are not.*
- **The M0 UI gate cannot exercise this path at all**: `setup()` returns early under `--smoke`
  ([lib.rs:464](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L464), `:429` when this was written), so the gate never reaches the
  reconcile. This is the same structural gap already noted for the tray/notification fixes. Coverage
  here is the Rust tests (pure DB + registry, no threads) plus the launched-exe proof in §7.
- typecheck clean, M0 GATE PASS on the build under test.
- The change was proved to have reached the *link*, not just the source: both new SQL strings appear in
  the shipped exe and **zero times** in the previous build kept aside as a control
  (`rg -a`, per [[feedback-verify-in-shipped-binary]]).

## 6. Cost

| | bytes |
|---|---|
| before | 4,567,552 |
| after | **4,569,600** |
| delta | **+2,048** |
| headroom under the ruled 5 MB | **673,280** |

Cheaper than the cost map's "≈5 KB for a new rusqlite read path" estimate, because this is a two-statement
scan plus four lines of wiring on top of machinery that already shipped, and because it added no channel —
the map's own data says a channel is the expensive kind (the origin-auth channel cost +27,648 B alone).
Measured with `stat`, never a hash: the exe is not byte-reproducible.

## 7. Proof in the launched binaries

`D:\RE\_cap-harness\reboot-run.mjs`, report `reboot-report-2026-11-05T01-40-38-656Z.json`:
**3 legs, 19 checks, `allLegsPass: true`.** A 32 MiB Range fixture, each instance on its own
`--data-dir` and its own WebView2 folder; the user's own running app untouched; no leftover process or
directory after the run.

- **Leg 1 — counterfactual, on the previous build** (`sucr-old-4567552.exe`). Killed mid-transfer at
  ledger 5,109,603 B. After relaunch: `status: downloading`, `streamsLive: null`, and the *same* ledger
  at 5 s, 15 s and 25 s. That is the defect, reproduced in the shipped binary rather than asserted.
- **Leg 2 — the shipped build opened on the identical wrecked data dir.** Same row: `downloading` with
  `streamsLive: 8` (a live job owns it), ledger 6,551,395 B ≥ leg 1's floor (it continues from its own
  bytes, not from zero), advanced to 9,301,697 B with **no Start pressed**, then `finished` with the
  full 33,554,432 B file and the staging file gone.
- **Leg 3 — hands-off negatives, with free slots** (so the negatives are not an artifact of `max_active`).
  Three rows: started-and-killed, added-but-never-started, paused. After reboot the first resumed
  (`streamsLive: 8`) and finished the whole 33,554,432 B file on its own; the never-started row stayed
  `queued` at 0 bytes with `startedAt: null` before, after and at the end; the paused row stayed
  `paused`, its staging file byte-identical, `streamsLive: null`.

Reboot is proxied by `taskkill /T /F` on the instance. That is a hard stop, which is the harsher case:
it does not let the app write a farewell status.

## 8. One discovery that changed the harness's own witness

The staging file's **length is not a progress witness**. Three streams write at each block's offset, so
the length is the furthest offset reached while the holes below it can still be empty: the killed 32 MiB
job's file measured **30,408,427 B while its ledger had verified 5,109,603 B** — a 5.9× gap. Leg 3 first
failed because it compared file lengths, and a row that *finished* during the wait has no staging file
left to measure at all.

`reboot-run.mjs` now reads the row's own `verifiedBytes` for progress, and keeps the file length only as
a named contrast (`stagingFileLengthAtBoot`). This is a measured mechanism for the shape of the still-held
"unpaced-host `verified_bytes` lag": the two numbers are different quantities, so a gap between them is
expected — that does **not** yet explain the lag that defect reported, which was a lag against a
self-report, and stays open as a hypothesis.

## 9. What is now true

"Transfers survive a reboot" is true in the shipped build, and proven in the shipped build: the row a
reboot killed comes back owned by a job and finishes without a click, and the rows a reboot did not kill
are exactly where their user left them. The autostart and schedule concepts no longer depend on a broken
premise.

## 10. Not in this slice

Nothing about *why* the machine stopped is recorded or reported — a reclaimed row says nothing about a
power cut versus a taskkill. `blocks.bytes_done` is trusted as the ledger with no checksum of the holes
it implies. There is no dedup between "reclaimed at boot" and "started at boot" beyond the status scan
itself, which the second-reclaim test covers. And the held list keeps: the registry-backed system-proxy
read, SOCKS5, an `https://` origin, the notification packaging identity, plaintext
session cookies, the Networks-pane proxied-byte sums, the inert card label, the `verified_bytes` lag. (The cap
+7.8 %/−3.9 % question that also sat on that list is **closed and retracted, 2026-11-05** — it was an instrument
artifact, not a limiter error; see `design/cap-precision.md`.) (The double chime that used to head that list was
measured and found not to exist — see `design/completion-chime.md`.)
