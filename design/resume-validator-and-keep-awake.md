# Resume validators + keeping the machine awake — the two cheap Plexo gaps closed

Record of the slice that made "a resumed download notices a host that changed under it"
true, and "a running transfer is not cut off by the machine going to sleep" true. Written
2026-11-06 under the user's standing delegation ("Do as you see fit"), approved by "Yea, do it."
Everything here is cited to a file:line re-read today, counted, or read out of a launched-binary
report; where a claim has no such warrant it is marked `unmeasured`. Confidence words are not evidence.

> **Superseded in part, same day.** § 2's mechanism and § 5's row 1 describe a build that could only read the
> stamp. The byte-sample slice that followed it ([`byte-sample-keeps-the-partial.md`](file:///D:/RE/Sucr/design/byte-sample-keeps-the-partial.md))
> put a hearing in front of the discard, so **"a rotated stamp costs the partial" is no longer what the shipped
> exe does** — a stamp-only host whose bytes still answer is kept, and the row below is the measurement of the
> build before that. The `etag > last-modified` precedence, the `validatorKind` naming in the reset payload, the
> keep-awake mechanism and the § 6 defect all stand.

## 1. Why this slice

The 2026-11-04 Plexo gap analysis listed advantages the Electron app had and Sucr did not. Two of
them were small, real and independently fixable:

- **Resume integrity.** Sucr's resume path compared only the announced length and the ETag. A host
  that rotates `Last-Modified` without an ETag — the common case for a rebuilt artifact served by a
  plain static server — kept its stale prefix, and the finished file was a mix of two generations.
- **Keep-awake.** Nothing asked Windows to stay awake, so a sleep policy set by the machine could
  stop a long transfer with no message.

Everything else on that list (per-NIC bonding, proxy, HTTP auth, per-download schedule) was already
either shipped or deliberately not this slice.

## 2. Mechanism A — validator priority on resume

The judgment is one `match` in the resume pre-flight
([job.rs:768-777](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L768)):

| arm | when it fires | effect |
| --- | --- | --- |
| both sides carry an ETag | `have != now` | cause `"etag"` |
| exactly one side carries an ETag | — | `None`: no claim either way |
| neither side carries an ETag | both stamps present and different | cause `"last-modified"` |

`validator_changed` ([job.rs:778](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L778)) folds into
`reset_progress` with the length change and the missing-partial case
([:781](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L781)), and a reset deletes the block map,
zeroes `verified_bytes`, stages a new file and writes the trail row
([:834-855](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L834); this body sat at `:796-812` when this
record was written and the byte-sample hearing was inserted above it the same day — the drift is why a
locator is quoted with its function name).

**Why the timestamp is only consulted when neither side has an ETag:** the two validators are
alternatives, not a checklist. A host that answers with an ETag has said more about the bytes than
its clock can, and comparing both unconditionally would discard resumable bytes over a clock skew.
This mirrors the priority the relink assert already uses
(`check_refresh`, [job.rs:3849-3866](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3849); cited as
`:3600-3607` when this record was written, before the sampler moved it down).

The cause is now named in the payload, so a `resume-reset` row says which of the four reasons fired:
`planChanged`, `validatorChanged`, `validatorKind`, `stagingMissing`. Before this slice a reset for a
timestamp move was indistinguishable from a reset for a vanished partial.

## 3. Mechanism B — `SetThreadExecutionState` held per registry

New module [keep_awake.rs](file:///D:/RE/Sucr/app/src-tauri/src/keep_awake.rs), 41 lines. `set(on)`
passes `ES_CONTINUOUS | ES_SYSTEM_REQUIRED` when a transfer is live and bare `ES_CONTINUOUS` when not
([keep_awake.rs:26-35](file:///D:/RE/Sucr/app/src-tauri/src/keep_awake.rs#L26)). Display state is
deliberately untouched: a transfer needs the machine, not a lit panel.

The call is made only from threads that outlive the transfers they speak for —
`Registry::refresh_keep_awake` ([job.rs:4132-4134](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L4132);
`:3839-3841` when written, drifted by the byte-sample insert above it)
from three sites:

| site | locator | why this thread |
| --- | --- | --- |
| job start | [ipc.rs:762](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L762) | the IPC thread that claimed the slot |
| job finish | [ipc.rs:789](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L789) | the same worker thread that ran the job and is still alive |
| adapter watcher | [lib.rs:406](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L406) | the always-running pass; a guard, not a state machine |

`ES_SYSTEM_REQUIRED` is per-thread and dies with the thread that set it, which is why the watcher
re-asserts every pass and why no job worker ever calls it. There is no setting for this: an overnight
download must survive a dial nobody looked at. What the slice does **not** claim is the right to wake
a machine *from* sleep — starting a transfer on a timer is a different, unwritten capability.

## 4. Byte cost (measured with `stat`, never a hash)

| state | bytes |
| --- | --- |
| build before the validator slice, retained on disk as `sucr-held.exe` | 4 579 328 |
| validator + keep-awake slice, `sucr.exe` | 4 579 840 |
| **Resume-swallow fix, `sucr.exe` (2026-11-07)** | **4 593 664** |
| delta from the validator slice | **+512 B** |

The fix is one quantum at the 512 B granularity — the same cost as the validator slice — so a
per-component split cannot be quoted from these two numbers. No new channel, no new setting key, no
schema version: `channels` stays 35, `SETTING_KEYS` stays 24, schema stays v4 with 17 tables. Rust
suite at this build: **223 passed / 6 ignored** (was 216 / 6). Ceiling 5 MB, RULED 2026-11-04;
headroom **649 216 B**.

**A `stat` match proves size only, and this is the case that proves it.** The fix is two
`wait_idle` calls — a generic function name that is mangled away in a stripped `opt-level="z"` binary,
so a byte scan of the new exe counts `wait_idle` **0** and `Duration::from_secs` **0**, while the
literal the new code carries, `pause this download before replacing its link`, goes 0 → 1. The size
move is the evidence; the literal is the locator. A symbol count would have been the wrong instrument
twice over: `wait_idle` already existed (the delete path), so a 0 → 1 count would have been
misleading, and a 0 → 0 would have been indistinguishable from "not linked". `stat` + a literal probe
is the pair.

## 5. Launched-exe proof — `D:\RE\_cap-harness\resume-run.mjs`

Five legs, each with its own `--data-dir`, WebView2 folder, out dir and CDP port (9461+), against a
local fixture on 8151 that serves 8 MiB at 256 KiB/s per stream, honours `Range`, and **never** sends
an ETag. Every leg presses the shipped controls: a card is a listitem with no buttons, so Start and
Pause go through the inspector (`#detail .actions button`, labelled by `updateOverview`). The size of
both exes is pinned before launch.

Full green run `resume-report-2026-11-06T02-57-53-661Z.json`: **18 checks, `allLegsPass: true`,
blocked `[awake_while_a_transfer_lives]`**; the two witnesses added afterwards
(`resumed` trail, directional byte count) re-passed in `…02-59-57-067Z.json` (12 checks, legs `rotate`
and `keep`, all passing).

| leg | what it establishes | the numbers it rests on |
| --- | --- | --- |
| `resume_resets_on_a_rotated_stamp` | a stamp-only host costs the partial, and the trail names the cause — **true of this build only**: the byte-sample slice that followed makes the same press a `matched` keep, see `byte-sample-keeps-the-partial.md` § 2 | row 4 194 304 parked; `resume-reset` payload `{validatorChanged: true, validatorKind: "last-modified", planChanged: false, stagingMissing: false}`; both `probe` rows with `etag: null`; after the second Start the host was asked for **8 388 609** B, not the 4 194 304 remainder; `finished` at 4 780 ms |
| `an_unmoved_stamp_keeps_the_bytes` | the control: nothing moved, so nothing is thrown away | zero `resume-reset` rows; host asked for 4 194 305 B = the remainder; row `finished` at 8 388 608 B; a `resumed` trail row within 250 ms of the press |
| `previous_build_kept_the_stale_bytes` | the counterfactual, on the retained `sucr-held.exe` | no `resume-reset`; a `resumed` + second `probe` prove the press reached the host; 4 194 306 B served = the remainder only, `finished`, with **4 194 304 stale bytes kept** |
| `awake_witness_is_thread_local` | falsifies the cheap OS witness before any claim rests on it | a holder sees its own claim; a fresh process reads `0x80000000` both idle and while a sibling holds it; the holder's own release returns `0x80000001` |
| `awake_while_a_transfer_lives` | the OS-side claim | **BLOCKED, not passed.** Needs `powercfg -requests`, which Win11 26200 refuses to an unelevated shell (`-a` confirms Standby (S3) exists on this box) |

To unblock the last leg, run this once in an elevated window while the leg runs:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File D:\RE\_cap-harness\awake-watcher.ps1 -Seconds 200
```

It appends `HH:MM:SS|holders|(none)` per second to `awake-requests.log`, which the leg then correlates
against its own idle / live / parked / after windows. Until that file exists the awake claim is:
**in the binary (proved by string count), not measured at the OS.**

## 6. Resume-swallow defect — FIXED 2026-11-07

`download_start` used to answer with the row itself whenever the registry still carried its id
([ipc.rs:684](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L684)), and the registry releases the id only
after the job returns ([ipc.rs:786](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L786)) — which is after
the job has already written `paused`
([job.rs:547-551](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L547)). A Pause then a quick Resume
could land inside that window and be silently ignored.

**Fixed by making the IPC layer wait for the thread to retire.** `download_start` now uses
`Registry::wait_idle` with a 6 s bound ([ipc.rs:696-701](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L696))
when the row says `paused` or `needs-attention` but the registry still says running. A start-in-progress
(`queued` + running) or a genuine transfer (`downloading` + running) remains idempotent and skips the wait.
If the thread cannot be joined within the bound (a worker blocked past the timeout), the start is refused
rather than spawn a second job onto the same id.

**Second witness closed:** `download_refresh_url` now uses the same `wait_idle` guard
([ipc.rs:973-975](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L973)), closing the race where a relink was
refused because the old thread was still in its post-pause cleanup. The harness's `waitRelinkable` workaround
is now redundant but preserved for safety.

## 7. Harness traps this pass produced (all of them cost a wrong answer first)

- **A download card has no buttons.** `cardFor` builds a listitem that only selects; the controls are
  the inspector's, written by `updateOverview`. A leg that clicks `.card button` clicks nothing.
- **`history:tail` is `ORDER BY ts DESC`.** `rows[0]` is the newest; the first pass read the oldest
  `probe` and reported the host's validator backwards.
- **`waitStatus(downloading)` is not a witness for a short run.** At ~2 MiB/s an 8 MiB second pass
  finishes inside the budget, so a poll for `downloading` can time out on a job that already completed.
- **`verifiedBytes` is block-quantised.** This plan chose 1 MiB blocks, so the row advances once per
  ~4 s of a 256 KiB/s stream. A check on "the row counted more than before" read false on a run that
  was verifiably transferring 1 MiB on the wire.
- **An arithmetic bound is not a proof.** The in-flight tolerance (8 streams × 1 MiB = 9 MiB) exceeded
  the 8 MiB file, so the byte cross-check was vacuous; it now asserts only the real invariant — the
  host sent at least what the row verified — and reports the bound as a note.
- **`stat` proves size, never content.** The old build's silence about a reset was only evidence once
  a binary scan showed it lacked the timestamp arm (`last-modified` 2 vs 4) and its own run had
  reached the host.
- **A shared fixture needs its counters zeroed after any incidental request.** The Add bar's own link
  check touches the path before the row exists.
- **A refusal text can be on stderr.** `powercfg` writes its access denial there; reading only
  `stdout` made an honest "not permitted" look like an empty result.

## 8. Still absent after this slice

Per-download start times and any wake-from-sleep (both still the user's calls), SOCKS5, the
registry-backed system-proxy read, any `https://` origin credential, and cross-process `queued`
disclosure. The keep-awake half is shipped but its OS-side proof is blocked on one elevated run.
