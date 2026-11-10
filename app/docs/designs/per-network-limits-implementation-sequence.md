# Per-Network Limits — Implementation Sequence

* **Author:** Benjamin (machine account)
* **Date:** 2026-11-03
* **Source design:** [per-network-limits.md](per-network-limits.md)
* **Status:** Steps 0, 1, 2, 3, **3b**, **4** and **6** are done and measured. 3's rate-cap half: a cap saved on an
  adapter limits the transfer that leaves through it, on both the parallel and the non-range path. 3b's over-quota
  half: an adapter that has met its allowance stops being claimed at the next block boundary and is re-admitted
  the moment the ceiling moves or the window rolls. 4: the ramp reads only the bytes no limiter set, so a paced
  window neither grows the Auto band nor plateaus it. **6: the two controls are on the card, and the launched exe
  was driven entirely through them** — the sentence this section used to carry, "Step 6 (the controls) is what makes
  any of it reachable by a user", is now closed; a cap and an allowance are reachable by a click, and both were
  measured live by a click. Headroom after step 6: **2,764 B** (the step cost 5,632 B; step 4 left 8,396 B). The
  four rulings are
  [chosen below](#the-four-rulings-chosen-2026-11-04) under the user's 2026-11-04 delegation — not `DECIDED`,
  which stays the user's own marker. Step 4 was mandatory rather than merely sequenced: enforcement was live on
  both halves and
  [step 3's measurements](#step-3--done-2026-11-04) record why. It is now shipped. **Step 7, the ledger correction
  in [app-design-plan-v2-engine.md](../../../design/app-design-plan-v2-engine.md), is done too — all seven steps of
  Feature 1 are closed.** What the feature left open is recorded where it was found: the ramp plateau defect under
  step 4's measurements, ruling 2's unshipped inert label, and the two-directional overshoot-vs-cap discrepancy
  under step 6's.
* **Correction carried into step 3:** the third bucket's cost was first quoted as 14–15 ns per charge round in
  debug codegen. `cargo test --release` runs after all (`panic = "abort"` does not block it), and in the profile
  that ships the marginal cost is **~0.5 ns** — so the throughput premise below is stronger than recorded, not
  weaker. Details in [step 1's measurements](#step-1--done-2026-11-03).

## What this document is not

The design doc's Detailed Implementation Plan lists **what** changes in each file. This is the **order** to do
them in, what each step proves before the next one is allowed, and where a step can be abandoned without
wrecking the ones before it. It is still not code.

The two spike measurements the design doc demands — the byte cost and the three-bucket throughput cost — come
first, because the whole feature is conditional on them. If either fails, Alternative 1 (delete the columns) is
the shipped outcome and no later step happens.

## The four rulings, chosen 2026-11-04

The user's instruction was "Chose what you see fit", so these four are **chosen by EGM under delegation, not
`DECIDED`** — the marker stays the user's own, and any of the four can be overturned. Rulings 1 and 2 were the
two this section used to call "owed"; 3 and 4 are the pair that only appeared once ejection was on the table
(what to do when every route is shut, and how a shut route gets let back).

1. **Mid-job ejection.** `RunContext.routes` promises routes are never re-read mid-transfer
   ([job.rs:142-144](../../../app/src-tauri/src/engine/job.rs#L142)). Ejecting an over-quota route breaks that
   promise for a quota, which is a deliberate change rather than an oversight. Either accept that blocks move to
   another adapter mid-job while the deck still labels them, or let an exhausted route finish the job and only
   exclude it from the next. The design doc's narrower second version was the pre-delegation default; the
   choice below takes the first.
   * **Chosen: eject at the block-claim boundary.** A block already claimed finishes on the device it started
     on; only a *new* claim looks at the allowance. The route **set** stays frozen exactly as the comment
     promises — the job never gains or loses a device mid-transfer — and what is re-derived is
     *eligibility* within that set, which is the same seam step 3 already crossed for a cap that did not exist
     when the job began (`a_cap_saved_after_the_job_began_reaches_the_next_attempt`). A quota is a ceiling the
     user set for the month, not a property of the file, so a byte that would blow past it is the worse
     betrayal than a block landing on the other adapter. The label drift is real and step 6 has to show it:
     the deck names the route that is out of bytes, so the screen admits what the transfer is doing.
     **Step 6 shipped that admission, and only its single-adapter half**: the shut route keeps its card, its chip
     label flips to "Over allowance" and its amount prints the spent figure over the ceiling ("136 MB / 89.0 MB"
     in the fresh-data-dir run, in the same one-second sample that first showed the ledger past the ceiling and with
     no click in between; 8 ms after the save in the already-met run, where the Save path's own refresh carries it). What
     remains unobserved is the drift *between* devices — a route ejected while a named route beside it carries on —
     which needs two physical links and is still carried only by
     `a_capped_adapter_does_not_silence_the_route_beside_it`.
2. **What the cap means on a single-adapter machine.** With one adapter and no bonding, `attribute_egress` can
   leave `Route.network_id` as `None` ([job.rs:617](../../../app/src-tauri/src/engine/job.rs#L617)), so the card
   shows two controls that do nothing. Either the cap falls back to the sole adapter, or the controls are
   disabled with a reason on screen. Shipping a silently inert control is the failure mode this feature was
   supposed to end.
   *Sharpened by step 3, which measured the case instead of assuming it:* on a machine whose bond resolves to
   fewer than two routes, `bond_routes` returns `vec![Route::default_route()]`
   ([ipc.rs:1497-1499](../../../app/src-tauri/src/ipc.rs#L1497)) and `default_route()` carries
   `network_id: None` ([bond.rs:51-58](../../../app/src-tauri/src/engine/bond.rs#L51)), so the registry lookup
   finds nothing and **the cap is inert — not unproven, inert, on this box**. Enforcement does hold for every
   job that actually bonds, and for the non-range path when a route carries an id
   ([ipc.rs:1550-1560](../../../app/src-tauri/src/ipc.rs#L1550)).
   ***Retracted 2026-11-04: that sharpening was wrong about this box, and the launched exe said so.***
   `attribute_egress` ([job.rs:834](../../../app/src-tauri/src/engine/job.rs#L834), reached from the route build at
   [:661](../../../app/src-tauri/src/engine/job.rs#L661)) credits the unbonded default route with the adapter the
   routing table actually leaves on, so `Route.network_id` is `Some(1)` (Ethernet) here even with one adapter and
   no bond. Step 6 then measured both controls on this machine, driven entirely from the card: a saved cap paced
   the transfer to a **0.2479** ratio against the same host uncapped, and a met allowance froze that same transfer
   and restarted it 988 ms after the popover cleared it. The static reading got the **rule** right and its *reach*
   wrong; nothing below depends on this box being unattributed.
   * **Chosen: a limit gates only attributed routes, and the inertness is shown rather than hidden.** An
     unattributed route is never spent and never ejected, and bytes no adapter can be credited for charge no
     allowance — the same rule `add_net_usage` already follows, so the ledger and the gate cannot disagree.
     Falling back to the sole adapter was rejected: it would need the engine to guess which device a socket
     left on, which is the guess `attribute_egress` exists to avoid, and on a machine with one *named* adapter
     plus a bond it would silently double-charge the link the user did not limit. The consequence is the honest
     one, and it belongs on the card: on a machine whose routes carry no adapter, both controls are labelled
     inert with the reason, instead of accepting a number that does nothing.
   * **As shipped (step 6): the rule is in force; the inert label is not.** The engine gates only attributed
     routes, and `bytes_no_adapter_can_be_credited_for_charge_no_allowance` is the proof of that half. The card
     carries no inert-with-the-reason state — on this box every route is attributed, so no control here is inert
     and there was nothing to label; the 5,632 B of UI that step 6 spent went on the controls themselves. What
     shipped instead names the scope on every surface: the cap chip's title is "Blocks leaving on this adapter are
     paced to this rate.", the allowance chip's is "This month is spent: no new blocks leave on this adapter until
     the window rolls.", and the popover note reads "Each limit spends only the traffic this adapter carries."
     **The gap this leaves open, stated plainly:** on a machine whose bond really does hand back an unattributed
     route, a limit typed into that card would be silently inert again — exactly the failure mode this ruling was
     written to end. It is unshipped, not closed.
3. **Everything shut.** Once a route can be ejected, a bond can reach a state where every route in it is out of
   bytes. Neither of the two obvious answers is good: failing the download punishes a job for a ceiling the
   user set, and a new "waiting on quota" state means a new job state, a new activity word and new renderer
   vocabulary for the rarest case in the feature.
   * **Chosen: wait on the tick that already exists.** A claim that finds no open route sleeps
     `TICK_POLL_MS` (250 ms) and re-checks, which is the cadence the worker loop already uses to watch for
     cancel ([job.rs:130](../../../app/src-tauri/src/engine/job.rs#L130)). No new state, no new word, and the
     job resumes on its own from either direction — the user raises the allowance or the window rolls. Proven
     by `a_ceiling_raised_while_the_job_runs_gives_the_adapter_its_blocks_back`, which cannot finish early
     because nothing is claimable until the ceiling moves.
   * The known cost of this choice: the ramp sees a stalled window while a job waits, so it shrinks rather than
     grows. That is the safe direction, and step 4's attribution is what will name it on screen.
4. **Re-admission.** A shut route needs to come back, and a latch (a flag set when it is ejected, cleared when
   the user edits the card) is the shape that breaks.
   * **Chosen: derive it, store nothing.** Eligibility is read from the two numbers on every claim —
     `used >= allowance` and `allowance > 0` — so raising the ceiling, or a ledger read of a window that rolled,
     is the whole of re-admission. A latch would have to be cleared by every path that can free an adapter, and
     the first one somebody forgets is a route stuck shut for the rest of the session with a card that says it
     has bytes left.

## Measurements

### Step 0 — done 2026-11-03

* **Baseline.** A rebuild of the unchanged tree gave **4,486,144 B**, not the
  4,488,192 B the 11:39 exe measured. The difference is not build noise: after
  step 0 the exe was rebuilt once more with two layout-only edits (a wrapped
  signature, a wrapped `ok_or_else`) and came out at **4,489,216 B** again,
  byte-identical. The 2,048 B shift came from the earlier build's dependency
  state — that first rebuild recompiled `tokio`, `tauri`, `rusqlite` and
  `webview2-com`, which had not been touched at 11:39. So the exe is
  reproducible for a given source and fingerprint state, and a headroom figure
  belongs to the build state that produced it.
* **Cost.** Step 0 is **+3,072 B** over its own baseline: two `AppState` fields,
  `db::update_network_limits`, the channel entry, the dispatch arm, the two arg
  helpers and the `as_slice` fix.
* **Headroom after step 0: 19,660 B** (4,508,876 − 4,489,216). Steps 1-6 are
  measured against this, not against the design doc's 20,684 B.
* **Gate.** `exe under 4.3 MB` passes. The channel count moved 32 → 33 in all
  three places: the array type and its test in `ipc.rs`, and
  `verify-m0.mjs`.
* **A constraint nobody wrote down.** `json!({ "channels": CHANNELS })` stopped
  compiling at 33, because serde implements `Serialize` for arrays only up to 32
  elements. It is now `CHANNELS.as_slice()` with the reason in a comment. The
  list was sitting exactly on the limit, so the next channel after this one
  would have hit the same wall.
* **What the harness can and cannot prove.** `network:list` enumerates the live
  adapters and creates the row through `note_network`, so a set-then-read-back
  round trip **is** testable under `--smoke`. The blind spot is narrower than the
  design doc's item 8 implied: what the harness cannot reach is the bucket
  registry, because `setup` returns before the watcher starts
  ([lib.rs:325-327](../../../app/src-tauri/src/lib.rs#L325)) — enforcement, not
  the write path.
* **Tests added.** `set_limits_writes_the_three_columns_and_clears_them` and
  `period_arg_rejects_the_windows_that_are_not_enforced`, both in
  [ipc.rs](../../../app/src-tauri/src/ipc.rs) beside the channel test.
* **Contract settled in code, not in the doc.** An allowance saved with no period
  is stored as `month`; clearing the allowance clears the period; `day` and `week`
  are refused with an error even though the schema `CHECK` admits them, because a
  weekly allowance silently counted monthly would misreport the number on the
  card.

### Step 1 — done 2026-11-03

* **What shipped in the tree:** `net: Option<&Bucket>` on `fetch_block` and
  `single_attempt`, joined to the two `read_slice`/`charge` calls; all three call
  sites pass `None`. `limit.rs` is untouched, as the design requires — the extra
  cap arrives through the existing slice rather than by a new arity, because
  `Bucket::reserve` is private and a job cannot book a bucket itself.
* **Byte cost: zero.** The relinked release exe is **4,489,216 B**, byte-identical
  to step 0's. An `Option` that every caller passes as `None` is inlined and the
  third slot is dead-code eliminated, so the parameter and the widened array cost
  nothing until step 3 hands over a real bucket. Headroom is still 19,660 B.
* **The measurement is a test:** `the_third_bucket_is_priced_before_anything_uses_it`
  in [tests.rs](../../../app/src-tauri/src/engine/job/tests.rs), `#[ignore]`d because it
  measures rather than asserts — a timing assertion fails on a busier machine for
  the wrong reason. Run it with
  `cargo test --lib -- --ignored --nocapture --test-threads=1`.
* (a) **Bookkeeping alone.** 200,000 `read_slice` + 1-byte `charge` rounds per
  shape, five passes rotated so no shape wins by running second, medians:
  460.8 ns for two buckets, 477.7 for three with the extra capped, 476.6 for three
  with the extra unlimited, 491.6 for four. Bracketed that way, **one more bucket
  is 14–15 ns per round** in debug codegen. A cap charges once per slice and a
  slice is a twentieth of a second's bytes, so 20 rounds/s per stream is the
  ceiling: **about 1.1 ms of CPU per stream-hour**, 0.00003 % of one core.
  The first (cold) pass of the two-bucket shape read 273 ns and the single-shot
  first run put the delta at +163 ns; the rotated medians say the real delta is an
  order of magnitude smaller. Single-shot timings on this box are not evidence.
* (b) **32 real streams over a socket.** Chaos server, 4 MiB, 32 blocks of 128
  KiB, 32 threads calling `fetch_block`, three runs per arm, whole-file SHA-256
  checked after every arm so no arm can look fast by losing bytes:
  cap on the job bucket (two buckets) 2.04 MiB/s median, the same cap on the
  network bucket (three) 2.04, cap on the job plus an open network bucket 2.04.
  The shapes are indistinguishable at the resolution the pipe allows — the run
  spread inside one arm is smaller than 0.01 MiB/s, while the two uncapped arms
  swing 180 to 400 MiB/s on loopback. So: **the difference is the cap, not the
  overhead**, and the uncapped arms are too noisy to price anything.
* **What this does not prove.** ~~Release-profile timing is unreachable~~ — **that claim was wrong and step 2
  disproved it.** `panic = "abort"` in `[profile.release]` does not stop `cargo test --release` from building and
  running the harness; I had recorded it as a blocker and quoted debug numbers as the only option. Run in both
  profiles on 2026-11-03, five rotated passes of 200,000 `read_slice` + 1-byte `charge` rounds each:

  | | two buckets | three, extra capped | three, extra unlimited | four | marginal, one more bucket |
  | --- | --- | --- | --- | --- | --- |
  | debug codegen | 455.1 ns | 476.9 | 476.6 | 497.4 | **+21 ns** |
  | **release codegen (what ships)** | 476.5 ns | 477.0 | 477.1 | 476.8 | **+0.5 ns** |

  Two independent release runs gave +0.6 and +0.5 ns, and in release the four-bucket shape landed *inside* the
  three-bucket spread (1.2 ns), so the shipped marginal is not distinguishable from zero. At the slice rule's 20
  rounds per second per stream, 0.5 ns/round is 10 ns per second — **0.036 ms of CPU per stream-hour**, against
  the **1.5 ms** the same test prints when run in debug (and the 1.1 ms that the 14–15 ns figure quoted above
  implied). **That earlier figure is a debug-codegen artifact; the shipped number is ~0.5 ns.** The per-round
  *total* barely moves between profiles (455 vs 476 ns) because it is
  dominated by the bucket mutex and the clock, not by the arithmetic.
  The socket arm re-ran too and repeats the same shape in both profiles: every capped arm 2.04–2.05 MiB/s —
  including the one where the cap sits on the *network* bucket rather than the job bucket — while the uncapped
  arms swing (358/393 debug, 422/473 release) and their minimums show the first arm is cold. The spike still never
  measures a bucket whose cap changes mid-transfer (`set_cap` under a running reader); that is step 2's test.
* **A gate that step 0 broke and did not notice.** `npm run verify` failed on
  *methods derived from schema*: `verify-m0.mjs` keeps a literal snapshot of the
  renderer's derived surface and `network: ['list', 'setEnabled', 'setLimits']`
  was not in it. The channel-count sites moved, this fourth one did not. Fixed
  during step 1; the gate now reports `M0 GATE: PASS (exe 4.28 MB)`.

### Step 2 — done 2026-11-03

Measurements.

* **Byte cost: 8,704 B, and the first attempt at it cost twice that.** The step
  landed at 4,497,920 B against the 4,508,876 B ceiling: **10,956 B of headroom
  left for steps 3-7**, down from 19,660 B.
* **The number worth knowing: a `rusqlite` read of its own is expensive.** The
  first shape of this step added two aggregate queries (`SELECT id, speed_limit
  FROM networks …` and a `GROUP BY` over `net_usage`) plus the code to fold their
  answers into the two maps. That measured **+17,408 B** and left 2,252 B — the
  feature would have stopped there. Disabling only the two calls, in place,
  measured the pair at **10,752 B**: two more instantiations of `prepare` +
  `query_map` + row-mapping + `collect::<Vec<…>>` in a binary that has no other
  caller for them. Rewriting the step to read what the existing adapter pass
  already holds — `note_network` returns the saved cap, `network_usage` returns
  the month figure — brought the step to 8,704 B for the same behaviour. The
  design doc's "no new read query is needed" was right, and this is the number
  that says why: **a new read path in `db.rs` should be assumed to cost ~5 KB
  until measured.** *(Measured later, and narrower than this sentence: the
  unattributed ledger added a read **and** a write plus a 17th table and a v4
  migration for **+3 584 B** in total, because `unattributed_usage` is a
  `query_row` over four `i64` scalars using the same closure shape
  `network_usage` already instantiates. What costs the 5 KB is a *fresh generic
  instantiation* — `query_map` over a new row type — not a query.)*
* **Link variance is not the explanation.** Relinking with a whitespace-only
  change to an unrelated module produced the identical 4,506,624 B, so the 17 KB
  was code, not the linker reshuffling.
* **What the step now does.** `AppState::network_bucket` (resolve, `None` =
  uncapped), `AppState::set_network_cap` (in-place `set_cap`, or remove),
  `AppState::set_network_usage` (the month mirror), all fed from
  `ipc::network_payload`, which every `network:list` and every watcher pass runs.
  `network:set-limits` applies its own cap immediately, because the watcher only
  re-reads on a fingerprint change and a saved cap would otherwise wait for a
  cable to move.
* **Two premises in the design doc did not survive contact with the code.**
  *It said* the emit fires from a count branch only; the branch is `if announced`,
  which already fires on any fingerprint change — the real gap was that the
  fingerprint carried only identity + local IP, so a Windows-side rename rewrote
  neither the row nor anyone's view. Fixing it cost five fields in one `format!`.
  *It said* the resolve helper should create "an un-capped" bucket when a row is
  absent; the map now holds **only capped** adapters, because step 3's slice is
  built from `Option` and an unlimited bucket in that slice would take a mutex on
  every block read for no ceiling.
* **Deliberately not built: the emits from the two write handlers.** The design
  asked for `network:changed` after `set-enabled` / `set-limits` "so other windows
  refresh". This app has exactly one window — `tauri.conf.json` declares `main`
  and no Rust code calls `WebviewWindowBuilder` — and the acting renderer already
  refetches (main.ts:5285). So the emit would only duplicate a fetch in the window
  that caused it. Recorded as a latent need, not a fix.
* **The spin loop is gone.** `watch_network_changes` used to `continue` past its
  own five-second sleep whenever nothing changed, so the loop hammered
  `GetAdaptersAddresses` on one core for the life of the process. The sleep now
  belongs to the loop. Correction to the defect note: it never rewrote the
  `networks` rows per pass — the row write sits in the changed branch, which the
  spin could not re-trigger — so the cost was CPU, not database churn.
  **Seen in the source; not measured as a CPU figure**, because `--smoke` returns
  early from `setup` and no gate observes this thread.
* **Tests.** `ipc::tests::reading_the_adapter_table_puts_saved_caps_on_live_buckets`
  (a cap on the row reaches the bucket, an uncapped adapter gets nothing, a month
  figure lands in the mirror, and a *cleared* cap removes its bucket) and
  `tests::a_cap_saved_after_a_job_resolved_it_reaches_the_same_bucket` (the
  `Arc::ptr_eq` identity across a `set_cap`, zero and negative caps both mean no
  cap). The design's "cap 0 is unlimited" case was **not** duplicated:
  `limit.rs`'s `cap_swap_applies_to_the_next_read` already covers it.
  138 tests, 132 pass + 6 ignored, `M0 GATE: PASS (exe 4.29 MB)`. The source holds **139** `#[test]` attributes;
  one is `#[cfg(unix)]` in `paths.rs` and does not compile on this Windows target, so 138 is the right number to
  quote for a gate run here.
* **Determinism, a third time.** After the documentation sweep, the only code-tree change was a doc comment on
  `watch_network_changes`; the release relink came out at **4,497,920 B** again, byte-identical, and
  `npm run typecheck` + `npm run build` + `npm run verify` all pass on it. Every byte figure in this file is
  therefore attributable to code, not to the linker.
* **What was still unproven at the close of step 2.** Nothing charged a network bucket yet — the registry
  was populated and read by tests only, so a saved cap still did not slow a transfer. Step 3 charged it;
  see [its measurements](#step-3--done-2026-11-04).

### Step 3 — done 2026-11-04

Rate-cap half only. Measurements.

* **Byte cost: 1,536 B.** The step landed at **4,499,456 B** against the 4,508,876 B ceiling: **9,420 B of
  headroom left for steps 4-7**, down from 10,956 B. The delta is again a multiple of 512 B, so the quantum
  note from step 1's budget still stands — this cannot be read as an exact instruction count.
* **Cheaper than the budget feared.** The step was planned as the one most likely to break the ceiling
  (registry reads, a `Vec` build and a per-route index in the hot path) and it cost 1.5 KB, one seventh of
  the 10.7 KB a second `rusqlite` read path cost in step 2. Enforcement in the charge slice is not what
  costs bytes; new DB reads are.
* **Throughput: the cap binds on the adapter it was saved on, and only there.** Engine test
  `a_network_cap_slows_its_own_adapter_and_not_the_other` — 4 MiB, 512 KiB blocks, four streams, two loopback
  routes, 1 MiB/s on the first one only, release codegen, two rotated passes. Mean duration of a *finished*
  attempt per adapter, read back from `block_attempts`:
  | adapter | pass 1 | pass 2 |
  | --- | --- | --- |
  | capped (1 MiB/s) | **925 ms** | **927 ms** |
  | uncapped, same job | **22 ms** | **22 ms** |
  A 512 KiB block at 1 MiB/s is 500 ms of debt; two streams share the one bucket, so ~925 ms per attempt is the
  cap arriving at the socket rather than a 42× margin being luck. The thresholds in the test are 200 ms and 3×,
  both far inside the measured figures on purpose: the box has 6 cores and other agents run builds on it.
* **Both paths are enforced.** `a_network_cap_paces_the_download_that_leaves_on_that_adapter` (2 MiB at
  1 MiB/s, parallel) and `a_network_cap_paces_the_single_stream_too` (1 MiB at 512 KiB/s against a server that
  refuses `Range`) reuse the margins the shipped job-scope tests already trust — ≥1.2 s / ≥1.0 s and <8 s — and
  both pass, so step 5's non-range work did not stay a separate step.
* **Three scopes intersect.** `limit::tests::three_scopes_intersect_instead_of_adding_up` adds the per-adapter
  bucket beside the job and global ones at the same 400 KiB/s: the second charge still lands in the 30–90 ms
  window, so a network cap does not halve a job that also carries its own.
* **One deviation from the plan, made because the plan contradicted a shipped promise.** The step was written
  to build `Vec<Option<Arc<Bucket>>>` from `routes` once in `parallel_download` and carry it in `WorkerEnv`.
  That cannot satisfy the card's own wording: `set_network_cap` edits an existing bucket in place, but an
  adapter receiving its *first* cap inserts a new map entry
  ([lib.rs:46-65](../../../app/src-tauri/src/lib.rs#L46)), so a slice frozen at job start would never see a cap
  saved while a transfer runs. What shipped instead: `WorkerEnv` carries `route_nets: Vec<Option<i64>>` — the
  device each route leaves on, frozen with the routes, because that part never moves — and `live_cap` reads the
  registry per block *attempt* ([job.rs:132-152](../../../app/src-tauri/src/engine/job.rs#L132)).
  `a_cap_saved_after_the_job_began_reaches_the_next_attempt` guards the seam, including the `set_cap` case where
  no new entry is needed. The read is one `RwLock` acquire per attempt, never per chunk; a job of 1 MiB blocks
  does ~4 of them per MiB.
* **No new `AppState` reach in the engine.** `RunContext.networks` is a plain
  `Arc<RwLock<BTreeMap<i64, Arc<Bucket>>>>` (`job::NetworkBuckets`) handed in by `ipc::spawn_job` from
  `state.network_buckets`, so `job.rs` still has zero `AppState` references and the engine tests inject their
  own registry. The lock order rule was not touched: the registry read happens in a worker thread while the
  database mutex is held by the supervisor, and nothing takes the db lock under a map guard.
* **What step 3 does NOT do.** ~~No over-quota ejection (ruling 1)~~ — **superseded the same day by step 3b
  below**, which built it once ruling 1 was chosen. What still stands: no quota writes of any kind from this
  path, and no UI — the only way to set a cap or an allowance today is the `network:set-limits` channel, which
  no control calls until step 6. So the feature is enforced but unreachable; a user sees no change from either
  step.
* **The consequence that makes step 4 non-optional.** `RampSample.capped` is
  `ctx.limit.cap().is_some() || ctx.global.cap().is_some()`
  ([job.rs:1530](../../../app/src-tauri/src/engine/job.rs#L1530)) — it does not look at the adapter registry.
  With enforcement live, a window paced only by an adapter cap now looks *fair* to the ramp, so the ramp bills
  those bytes against the growth bar and can plateau the Auto band on a limiter rather than on the host. Harmless
  today (nothing can set a cap) and a silent break of a shipped feature the moment step 6 ships. That is what the
  "must not be deferred past step 3" sequencing note was for.
* **Gates.** `cargo build --release` ok; `cargo test -- --test-threads=1` **137 passed, 6 ignored**;
  `cargo test --release -- --test-threads=1` the same 137/6; `npm run typecheck` clean; `npm run build` ok;
  `npm run verify` → `PASS exe under 4.3 MB (4.29 MB)`, `PASS 33 channels`, `M0 GATE: PASS`. Test attributes
  moved from 139 to **144** (five added: four in `engine/job/tests.rs`, one in `engine/limit.rs`), 143 compiled
  on this Windows target.

* **Launched-exe proof, 2026-11-04 — the cap binds in the shipped binary, not only in tests.** The gap this
  leaves from step 2 and step 4's own honesty note is now closed. Driver and fixture live in `D:\RE\_cap-harness`
  (outside `app/` on purpose — anything under `app/` is bundled into the embedded renderer). The exe under test is
  the shipped **4,500,480 B** build (mtime 2026-11-03T21:11:41Z), launched detached with its own `--data-dir` and
  a WebView2 remote-debugging port, driven over CDP through `window.sucr.*` and read back from the isolated
  `sucr.db` with `node:sqlite`. Eleven passes, three rotated rounds (round 1 kinds, round 2 reversed, round 3
  capped-first) plus the two mid-run switch passes; report
  `D:\RE\_cap-harness\report-2026-11-03T22-16-44-419Z.json`.
* **The host is a two-mode measuring fixture, because "a slow host" is two experiments.** Per-connection pacing
  (one clock per socket, 1 MiB/s each) means more streams really do move more bytes, so an uncapped job has a
  reason to climb and a cap that bites below what the link offers. Global pacing (one clock for the whole host,
  8 MiB/s) is the plateau control. A *free* host is deliberately out of the matrix: this link then does ~117 MB/s,
  the gigabyte is gone in nine seconds and the app's own counters read late — measured, not assumed.
* **Numbers, all inside the window and started one second after the first byte to exclude the settle.** Capped
  (2,000,000 B/s saved on Ethernet, host offering ~9 MiB/s): **2,011,843 / 2,015,033 / 2,021,769 B/s**, median
  **2,015,033** — **+0.75 %** off the cap. Uncapped on the same host: median **9,063,729 B/s**, so the cap held the
  job to **22.2 %** of what the very same link gave it. The host offered **4.53×** the cap, which is the premise
  every claim below rests on: the capped job was not idle, it was paced.
* **A cap saved mid-download is worth the click.** `cap-applied` measured **8,952,856 B/s** before the save at
  15,044 ms and **2,434,607 B/s** over the rest of the window; the per-second trace shows why the average is above
  the cap — the blocks already claimed at 8 MiB each keep draining for three seconds before the rate settles at
  ~2 MiB/s. `cap-released` is the mirror: **2,455,640 B/s** capped, **9,180,140 B/s** after the release.
* **Attribution, on the machine the app actually runs on.** Every one of the run's **297** block attempts carries
  `network_id 1`; **zero** rows are unattributed, and three independent counters agree to the byte — Σ
  `downloads.verified_bytes` = Σ `block_attempts.bytes` = `net_usage.bytes` = **2,144,518,002**. `network.list`
  returned `speedLimit: 2000000` while a capped pass ran and `null` after the release, so the saved cap is live in
  the app's own adapter surface, not only in the engine's map.
* **What this run did NOT exercise.** No allowance/quota ejection (the passes set `speed_limit` only, and an
  exhausted `data_limit` needs a second adapter to be visible), no second physical NIC, no `download.set-speed-limit`
  job-scope cap. Ruling 2's "inert on this one-adapter box" still holds for the *allowance*; it no longer holds for
  the *cap*.

### Step 3b — over-quota ejection, done 2026-11-04

The half step 3 held, written once rulings 1-4 were chosen. Measurements.

* **Byte cost: −512 B.** The exe went from **4,499,456 B to 4,498,944 B** — the step *returned* bytes, so
  headroom is now **9,932 B** against the 4,508,876 B ceiling. The reason is the design change, not the
  feature: the allowance and the cap now live in **one** `NetScope` entry per adapter
  ([job.rs](../../../app/src-tauri/src/engine/job.rs)), which replaced two `RwLock<BTreeMap<…>>` fields on
  `AppState` (the bucket registry and the separate usage mirror) with one. One lock, one poison handler, one
  lookup per claim instead of two, and `RunContext`/`WorkerEnv` gained **no** field — the merged type took the
  place of `networks`, so no test literal had to be edited for the plumbing. The cost rule from steps 0-3
  holds a third time: enforcement wired into an abstraction that already existed is free or better; a new
  `rusqlite` read path is what costs ~5 KB, and this step added none (it reads through `network_usage`, the
  query step 2 already had). *(The band's scope was fixed later: a read path that reuses an existing
  `query_row`-over-scalars shape costs far less — the unattributed ledger's read + write + new table + v4
  migration measured **+3 584 B** whole. The 10,752 B here was two *new generic instantiations*.)*
* **The two writers, settled.** A worker `note_carried` on every finished attempt (exact, mid-job) and the
  watcher pass / `network_payload` `note_ledger` **overwrites** with the ledger's own sums (authoritative).
  Between the two, a block can be counted once twice — which ejects a route slightly early, the safe direction
  on a metered link, and is corrected by the next pass. No `max()` merge rule was needed, and no lock is held
  across the database: the ids are collected under the map's read guard, the guard is dropped, then the query
  runs.
* **The window roll is the ledger's, not the engine's.** `net_usage` rows are keyed by local date
  (`period_key`), and the day/week/month figures are `SUM`s over a date range computed with
  `date('now','localtime')`, so the database reopens a window by construction. The engine only needs the
  refresh to *happen*: `watch_network_changes` now calls `refresh_network_quotas` on the tick where nothing
  changed, so a month rolling over with no cap edits and no card open still heals. A machine with no
  allowances set takes no database lock at all — the guard is the early return when no scope has
  `allowance() > 0`.
* **Where ejection bites, and where it does not.** Both claim sites (the worker loop and the straggler/hedge
  loop) and `single_download`'s attempt loop consult `open_route`/`route_spent`, so the gate is at a **block
  boundary**. A single-stream job has no block boundary — the whole file is one attempt — so a single-stream
  download that is already in flight can overshoot an allowance set while it runs; it is honoured *between*
  attempts, which is what `a_network_cap_paces_the_single_stream_too` shows for a cap and what
  `bytes_no_adapter_can_be_credited_for_charge_no_allowance` bounds here. Recorded in the code comment rather
  than glossed.
* **Hedge band.** `open_route` takes `hedge` and narrows the scan to the secondaries, so a raced half can
  never fall back onto the primary that went quiet — `a_hedge_never_races_onto_the_primary_that_went_quiet`
  pins that, including the one-route case where the band is the whole band.
* **Nine tests** ([engine/job/tests.rs](../../../app/src-tauri/src/engine/job/tests.rs)): seven pure
  (`a_job_with_no_allowances_gets_the_route_its_policy_asked_for` — the gate is silent for all three policies
  until somebody saves a quota; `an_adapter_out_of_bytes_yields_its_next_block_to_the_route_beside_it`, wrap
  included; the hedge band; ruling 2's unattributed case; `every_route_being_out_of_bytes_waits_and_a_raised_allowance_re_admits_it`,
  which is ruling 4's derived predicate with the window-roll half attached; `an_allowance_is_counted_against_the_window_the_row_names`,
  including a negative ledger row; `the_ledger_wins_where_the_engine_stopped`, the two-writer rule and
  `load_bearing`), and two over real sockets: `an_adapter_out_of_bytes_carries_no_block_while_its_bond_finishes`
  (2 MiB, four streams, two routes, one shut — every byte lands on the open route and the shut adapter's
  figure does not move) and `a_ceiling_raised_while_the_job_runs_gives_the_adapter_its_blocks_back` (both
  adapters shut, a thread raises one after 300 ms; the run must take **≥250 ms** — which is what makes the
  wait observable — and only the re-admitted adapter carries bytes afterwards).
* **Gates.** `cargo build --release` ok at **4,498,944 B**; `cargo test -- --test-threads=1` and
  `cargo test --release -- --test-threads=1` both **146 passed, 6 ignored** (137 → 146 = the nine above);
  `npm run typecheck` clean; `npm run build` ok; `npm run verify` → **M0 GATE: PASS (exe 4.29 MB)**.
* **Not measured yet.** The `open_route` scan takes the map's read lock once per candidate, so an all-spent
  bond with 32 streams and 32 routes is 32 shared-lock reads per blocked claim on a 250 ms tick — trivial at
  that cadence, and the common case (no allowances set) is one read. Not microbenchmarked; the earlier
  charge-slice numbers say the per-round cost of a lock read is single-digit ns.

### Step 4 — ramp attribution, done 2026-11-04

Measurements.

* **Byte cost: +1,536 B.** The exe went from **4,498,944 B to 4,500,480 B**, so headroom is now **8,396 B**.
  The first cut cost **5,120 B**, and 3,584 B of that was one line: `ids.sort_unstable()` over a
  `Vec<(i64, bool, bool)>`. A sort is a generic instantiation, not a function call — deleting it and taking the
  routes in their existing order (an adapter two routes share simply appears twice, and both halves of the pair
  move together) is *more* correct for a per-route signature and 2.3× cheaper. The cost rule from steps 0-3 now
  has a second clause: **wiring into an existing abstraction is free; a new generic instantiation is not.**
* **What `capped: bool` became, and why not an enum.** The plan said "record *which* scope capped the sample".
  Naming a scope answers nothing: an adapter cap that argues for `Hold` freezes the Auto band, which the design
  forbids, and one that argues for "fair" repeats today's bug. The question the ramp can actually act on is how
  much of the window the host set, so the field is a *quantity*: `host_bytes`, the part of `bytes_delta` that
  left on a route with no adapter cap in force. `next_width` both **prices** the growth bar from it and **judges**
  the window against it — the two halves move together or the rule merely gets stricter. A job or app-wide cap
  still zeroes it, exactly as the boolean did, so no old behaviour changed.
* **A cap in force is credited in full, not only where it bit.** Bytes are booked as limiter bytes whenever the
  route's adapter has *any* cap, not only when the bucket actually made a stream wait. A slack cap therefore
  blinds the ramp to that route. That is the same conservative trade the job-wide cap already made — a cap is the
  user saying "do not go faster here", and widening streams against it buys refusals, not bytes — and
  `a_capped_bond_offers_the_ramp_no_host_evidence` uses deliberately generous caps to prove the trade is real
  rather than an artefact of a slow link. Measuring actual bucket stalls instead would need the limiter to report
  slept time; not built, and recorded here as the reason.
* **`shifted`, the half `host_bytes` cannot carry.** An adapter that meets its allowance carries *nothing*, so it
  contributes no limiter bytes to subtract — yet the window it produced is smaller than the one the bar was priced
  from. Answering that bar with the survivors' bytes plateaus the job against a bond that no longer exists, which
  is exactly how the Auto band stalls high. So the sample also carries `shifted`: the adapters this job can leave
  bytes on, each with whether it is capped and whether it is spent, taken at both ends of the window. On a shift
  the bar is **voided**, not disproved, and the next window is judged under the bond that exists. `bond_state`
  returns an empty list when no route carries an adapter id — on this machine's single-adapter pool the ramp reads
  every window as one steady bond, the same inertness ruling 2 already declared.
* **Sampled at window boundaries.** A limit applied *and* lifted inside one window is invisible, because the
  registry is only read at the boundary. That is the blind spot the 250 ms claim cadence already has, not a new
  one, and the doc comment says so rather than the test failing to cover it.
* **Six tests** ([engine/job/tests.rs](../../../app/src-tauri/src/engine/job/tests.rs)). Pure:
  `a_partially_paced_window_is_judged_on_what_the_host_paid` (the bar is priced off 600 host bytes, not the
  window's 1,000, and the follow-up window is judged against the same smaller ask — plus the reverse case, one
  host byte short is a plateau), `a_paced_window_cannot_climb_the_ceiling_either` (recovery is evidence about the
  host too, so a paced window leaves the ceiling where the refusal put it),
  `a_bond_that_shifted_voids_the_bar_instead_of_plateauing` (and the identical window *without* the shift is still
  the plateau it always was, so the forgiveness is the ejection and not a soft rule),
  `the_bond_signature_lists_only_the_adapters_this_job_can_leave_on`. Over real sockets, a 12 MiB auto-band job on
  two loopback routes: `a_capped_bond_offers_the_ramp_no_host_evidence` (both adapters capped ⇒ **no `ramp-grow`
  row at all**, width stays 8, the whole file still lands) and `a_capped_adapter_does_not_silence_the_route_beside_it`
  (one capped, one free ⇒ the 8→9 step is still taken, and every recorded row satisfies `0 < host ≤ bytes`).
* **The regression test was falsified, not just passed.** With the subtraction replaced by `delta` — i.e. the
  shipped bytes again read as host evidence — `a_capped_bond_offers_the_ramp_no_host_evidence` fails and prints
  the widening it objects to: `ramp-grow` 8→9→10→11 with `host == bytes` in all three rows. The counterfactual was
  run, the edit reverted, and the release rebuild returned to **4,500,480 B** byte-for-byte.
* **Gates.** `cargo build --release` ok at **4,500,480 B**; `cargo test -- --test-threads=1` and
  `cargo test --release -- --test-threads=1` both **152 passed, 6 ignored** (146 → 152 = the six above; the
  signature test is one of them, not a step-3b carry-over); `npm run typecheck` clean; `npm run build` ok; `npm run verify` → **M0 GATE: PASS
  (exe 4.29 MB)**. Re-run on 2026-11-04 after the doc pass: a fresh `cargo build --release` came out
  **byte-identical at 4,500,480 B**, both profiles report **152 passed, 6 ignored** (debug 55.25 s, release
  50.72 s), `npm run typecheck` silent, and `npm run verify` PASSes all **47** checks.
* **What this step could not show when it closed, and now does.** The paragraph written here on the day claimed
  that on this machine `bond_routes` hands back one *unattributed* route, so `host_bytes == bytes_delta` in every
  window and the attribution would be invisible in a launched run. **That premise was wrong** and the launched-exe
  run below falsifies it: `attribute_egress` credits the unbonded default route to the adapter the routing table
  leaves on (Ethernet, `network_id 1`), so a single-adapter box does get attributed routes, a saved cap does reach
  the transfer path, and the limiter subtraction is visible in the shipped binary's own history rows. The claim
  that survives is the narrower one: the two *loopback-alias* socket tests still prove attribution for two routes,
  which no physical second NIC is here to repeat. **The clause this line carried with it — "an exhausted allowance
  still needs a bond to be observable (ruling 2)" — is retracted**: step 6 measured an allowance met by the
  transfer itself freezing that transfer on this single attributed adapter, through the card, and unfreezing it
  988 ms later. A bond is still needed to observe one adapter being *left out while the other carries on*; it was
  never needed to observe the allowance closing a route at all.
* **Launched-exe proof, 2026-11-04 — a capped window is not read as the host's answer.** Same run, same
  report (`D:\RE\_cap-harness\report-2026-11-03T22-16-44-419Z.json`), same 4,500,480 B exe. The evidence is three
  integers in `history`: **11** `ramp-final` rows (one per pass), **8** `ramp-grow`, **8** `ramp-plateau` — and the
  eight of each are exactly the eight passes that ran without a cap. The three capped passes wrote **no ramp row
  of any kind** while **57.3-57.5 MiB** landed in each of them, and their recorded `streams_final` is **8** against
  **9** for every uncapped pass on the identical host. This is the whole point of step 4 stated as an absence: the
  pre-step-4 reading of those windows would have been a host plateau — a permanently stalling Auto band — and the
  shipped binary does not record one.
* **The two directions, in the ramp's own rows.** `cap-applied` grew 8→9 at rel 3,381 ms with
  `bytes == host == 29,360,128` (every byte was the host's, because nothing paced it), then transferred for a
  further 21 s at the cap with silence. `cap-released` is the sharper row: it grew 8→9 at rel 19,979 ms carrying
  `bytes 17,170,432 / host 12,582,912` — **4,587,520 bytes of the accumulated window were billed to the limiter and
  the step was taken on the 12.58 MB the host actually paid**. That is step 4's arithmetic running inside the shipped
  exe, on a real socket, with the subtraction visible in the payload it recorded.
* **Why the subtraction outlives the release by a few seconds.** `paced` is decided per block *attempt*
  ([job.rs:2380-2470](../../../app/src-tauri/src/engine/job.rs#L2380)), and an 8 MiB block claimed at 1 MiB/s per
  connection is in flight for ~8 s. The workers that started under the cap keep charging its bucket until those
  attempts end, so limiter bytes are still subtracted for a few seconds after the cap is lifted. The direction is
  conservative — the ramp is given *less* evidence than it could claim — and it is the same claim-boundary the
  ejection ruling already accepted.
* **What this run still cannot show.** No pass on this box can show a capped adapter being left *out* of a bond
  while a second one carries on — that needs two physical links — and `shifted` has no history row of its own, so
  the 4.7 s gap between the release and the growth in `cap-released` is consistent with a voided bar but does not
  prove it; `a_bond_that_shifted_voids_the_bar_instead_of_plateauing` remains the only evidence for that rule. What
  the run *did* show, and what caps the climb short of 32 in every uncapped condition, is a defect in the shipped
  ramp rather than in the harness — next bullet.
* **The uncapped climb is one step, and the reason is not the host.** Every uncapped pass grows 8→9 and records a
  `ramp-plateau` **22-90 ms later** (free 6,770→6,792 · host-limited 5,085→5,115 · cap-applied 3,381→3,406 ·
  cap-released 19,979→20,068). The `bytes` in each grow row is the giveaway: free#1 reports 54,525,952 bytes at
  rel 6,770 ms, and 8.6 MB/s × 6.77 s = 58 MB — the window the ramp judged was not 1,000 ms, it was the whole
  transfer up to that point, because **no `Msg::Tick` was handled before the first growth**. The ticker sends one
  tick per second ([job.rs:1516-1535](../../../app/src-tauri/src/engine/job.rs#L1516)) but the coordinator that owns
  the ramp and the SQLite writes does not reach them for 3.4-6.8 s, then processes the backlog back to back: the
  first tick sees a huge accumulated window and grows, the next arrives 22 ms later with ~200 KB in it. That tiny
  window answers the growth bar — the `gain_bar` branch at
  [job.rs:2112-2122](../../../app/src-tauri/src/engine/job.rs#L2112) is checked *before* the
  `now_ms - last_change_ms < window_ms` gate at [job.rs:2124](../../../app/src-tauri/src/engine/job.rs#L2124), so
  short windows are not exempted the way silent and paced ones are — `host_bytes < bar`, `state.plateau = true`, and
  the latch holds for the rest of the job. Hence `streams_final = 9` in all eight uncapped passes. Two separate
  faults, one feature cost: the ramp's clock does not advance during a loaded transfer, and a window much shorter
  than `window_ms` is allowed to disprove a growth. The Auto band therefore cannot climb past +1 on this machine in
  a real download. **Step 6 did not put a label on it** — what shipped there is the two limit controls and their
  chips, and the ramp's own plateau still has no word on screen, so this stays open as a rule change rather than a
  UI change. Not fixed here: it is a rule change, not a measurement, and the engine tests pass because their
  windows are handed in one per simulated second.

### Step 6 — card controls, done 2026-11-04

Two launched-exe runs, both driving nothing but the real window: every limit below was typed into the card's
popover and saved with the popover's own Save button. `network:setLimits` is never called by either harness — the
only IPC the harnesses use is the app's own `window.sucr.*` surface (`network:list`, `download.*`, `app.quit`).
Reports: `D:\RE\_cap-harness\report-step6-2026-11-03T23-12-13-829Z.json` (cap, pacing, restart, allowance, input
semantics) and `report-step6b-2026-11-03T23-17-35-407Z.json` (a fresh data dir, the crossing itself).

* **Gates, with the UI in.** `npm run typecheck` clean; `cargo test --release -- --test-threads=1` **152 passed,
  6 ignored** (unchanged — step 6 is renderer-only); `npm run verify` **M0 GATE: PASS, 48 checks**, the 48th being
  `a card cap chip saves through network:set-limits and clears again`, which clicks the real chip, types 2 MB,
  re-reads the row *after* the repaint and then clears. Release exe **4,506,112 B** against the 4,508,876 B
  ceiling → **2,764 B of headroom left**. Step 6's cost is **+5,632 B** over step 4's 4,500,480 B (the popover,
  the two chips and the click delegation); the live-deck freshness hook added afterwards rebuilt to the *same*
  4,506,112 B — 0 B — which is what a minified, embedded bundle does in 512-B steps. Feature 1 therefore finishes
  inside the 4.3 MB ceiling without needing the parked 5 MB ruling.
* **The shape shipped is a popover with an explicit Save, not the inline pair the plan described**
  ([per-network-limits.md:375](per-network-limits.md)). Three reasons, all measured before choosing: the deck
  repaints on a timer and would detach an inline editor mid-edit; `db::update_network_limits` writes all three
  columns in one UPDATE, so a blur-save of one field would erase the other unless the untouched one is resent —
  the popover does resend it (`keptOther`/`keptUnit` are recorded in both reports precisely to watch that); and
  two fields with two unit selects do not fit a card without a second layer of chrome. What the plan asked for is
  honoured in behaviour: one click on the adapter's own chip, focus lands in the value field (`focused` was
  `net-cap-val` in every open of both runs), and the popover is positioned by the chip that opened it (top 385,
  left 43 for the cap chip and left 114 for the allowance chip, width 248, `insideWindow` true in all four opens).
* **A cap typed into the card paces the transfer.** Row after Save: `speedLimit 2097152`, `dataLimit null`,
  `period null`; the chip repaints `net-limit-btn is-set` reading "Cap / 2.0 MB/s". Four rotated passes
  (`card-cap#a`, `free#a`, `card-cap#b`, `free#b`), 24 s each, rate = bytes served by the fixture / elapsed:
  **2,242,382 and 2,278,038 B/s** capped (median **2,260,210**), **9,157,763 and 9,074,961 B/s** uncapped on the
  same host (median **9,116,362**) → ratio **0.2479**. The cap alternated with free, so no single shot carries the
  claim. ~~**An unexplained discrepancy, recorded rather than smoothed:** this is **+7.8 %** above the 2,097,152 B/s
  ceiling, where step 3's launched run measured **2,015,033 B/s** — 3.9 % *below* the same nominal cap. The
  candidate cause is the denominator, not the bucket: this harness counts bytes the *server* wrote, and a limiter
  charges on read, so the socket buffer between them is not the app's rate. Unconfirmed — no run here separated
  the two counters — and it does not touch the ruling, only the precision of the number.~~
  **That discrepancy is CLOSED and RETRACTED, 2026-11-05 — it was an instrument artifact, and one of its two
  numbers was my arithmetic error.** Full measurement, predicates and the corrected sentence:
  [cap-precision.md](../../../design/cap-precision.md). In brief: (1) **the `−3.9 %` was wrong** — step 3's
  2,015,033 B/s divided by its own saved 2,000,000 B/s is **+0.75 %**, which is what this document's step-3
  measurement section already said; both figures were above their caps. (2) **The denominator was not the cause of
  the gap**, because step 6's `measuredBps` and step 3's headline `rate` are *the same* fixture-written counter —
  and the two counters are tied by an identity, `(app_rate − wire_rate) × span ≡ excess(start) − excess(end)`,
  verified to 0.00 MiB on 6/6 fresh legs, so they can only disagree by a change in the run-ahead and never about
  the bucket. (3) **Step 6's own two legs disagree by 5 pp** (+2.07 % and +7.38 % at a late span start on the same
  build), so `+7.8 %` was never a precision figure, and re-running this harness on the current exe gives
  **+8.6 % / +3.4 %, median +6.0 %** — the family reproduces on a different binary, which rules out the build too.
  (4) **The enforcement scales and is tight**: on exe 4,569,600, fixture-written endpoint slope over a span starting
  at t ≥ 5 s, three rotated legs — **−0.40 %** at a saved 2,000,000 B/s and **−0.72 %** at 4,000,000 B/s.
  (5) **The instrument's floor is ~5 %, not ~1 %**: a saved 2,097,152 and a saved 2,000,000 both delivered
  ≈2.00–2.01 MB/s, and every 30 s capped leg served byte-identical **67,108,865 B**, so nothing measurable over
  CDP resolves the MiB-vs-10⁶ difference the card's own unit makes. Read this bullet's `+7.8 %` as the harness's
  24 s window talking, not as the limiter's error, and quote §6 of `cap-precision.md` instead.
* **A card-saved cap survives a restart, exactly.** Instance 1 was closed with the app's own `app.quit`, instance 2
  launched on the same `--data-dir`: row still `speedLimit 2097152`, and the popover **prefilled** `capValue "2"`
  with `capUnit "1048576"` — the value round-trips through the unit select, not just the bytes. The chip opened
  `is-set`.
* **An allowance met by the transfer itself stops that transfer on one attributed adapter — the case ruling 2
  said this box could not produce.** Fresh data dir, `usedMonth 0`, job running at ~9 MB/s with 9 connections
  open. Baseline read at the moment of the write: ledger `67,108,864` while the wire had already delivered
  `89,915,393` — booking happens at attempt end, so the numerator moves in ~8 MiB steps and lags the transfer;
  this is why the allowance was set to 89 MB (`93,323,264`) rather than a round number of the wire count. Then,
  sampling once a second with no further clicks:
  * **crossing at t = 6,836 ms** — ledger `142,606,336` ≥ `93,323,264` — and **the same sample** shows the chip as
    `net-limit-btn is-shut` "Over allowance / 136 MB / 89.0 MB". Nothing was clicked between the Save and this
    sample, so this is the 5 s live-deck refresh carrying the state, not the save-time repaint. (The save path does
    refresh — `saveNetLimits` awaits `refreshNetworks` — which is why the other run's already-met allowance showed
    the shut chip **8 ms** after Save.)
  * **in-flight bytes finish, then nothing new starts**: `69,206,016 B` landed after the crossing (`open` 9 → 8 at
    12,442 ms → **0** at 13,884 ms), then `served` froze at **218,103,809** across **11 consecutive samples** from
    13,884 ms to 23,944 ms — ~10.1 s — while `status` stayed **`downloading`** the whole time. Not failed, not
    paused, no new state: exactly ruling 3's 250 ms re-check presenting as a stalled transfer. `verifiedBytes`
    converged to 218,103,808 during the stall, so the file on disk and the ledger agreed at the freeze.
  * **re-admission from the card alone**: both fields cleared through the same popover (`dataLimit null`,
    `period null`), and the very next sample at t = 25,238 ms — **988 ms** after the clear — shows
    `served 225,181,697`, `open` back to **9**, the chip `net-limit-btn` "Allowance / none". The job was never
    touched: no pause, no resume, no restart.
  * The first run corroborates the same behaviour at a different granularity: allowance already met at Save, shut chip
    8 ms later, `served` frozen at 75,497,473 across **11 consecutive samples** (12,994 ms → 23,046 ms, ~10.05 s) with
    `open 0` from the first of them, in-flight overshoot 21,233,664 B, `stallBytes 0`, resume **1,643 ms** after the
    clear. *Correction, same day:* this row first read that freeze as "9 samples" because it quoted the harness field
    `stallWindowSamples`, which counts only the samples taken more than 8 s after the spend was first seen — the trace
    in the same report has 11 identical `served` values. The stall length is the trace, not the field.
* **Input semantics measured, not assumed.** Typing `0` and `-1` into the cap field both write **NULL** and repaint
  the chip "Cap / none" — consistent with `byte_count_arg`'s "zero or negative means unlimited", so a zero does not
  mean "shut". The note stayed the hint ("Each limit spends only the traffic this adapter carries.",
  `noteIsError` false) in every one of those writes; the popover's `unreadable` branch never fired and cannot fire
  through a `type=number` field, so it is defensive code with no test path, and the doc should not claim it is one.
* **What this step still cannot show.** One adapter: no run on this box can watch a route be *left out* of a bond
  while a second one carries on — that needs two physical links, and `a_capped_adapter_does_not_silence_the_route_beside_it`
  remains the only evidence for that half. And ruling 1 promised label drift; on one adapter the shut chip *is* the
  whole of that admission, so the deck-vs-transfer mismatch under a real bond is unobserved. The inert-with-the-reason
  labelling ruling 2 asked for is **not** shipped, for the reason given in the rulings section.


## Sequence

### Step 0 — Spike: byte cost (no behaviour change) — done

* **Do:** add the two `AppState` fields and the `network:set-limits` channel with a handler that writes the row
  and returns `{ok: true}`, nothing else. Build release.
* **Measure:** `stat` the exe against the 4,508,876 B ceiling; the number this step planned against was 20,684 B
  of headroom, and [its own measurement](#measurements) corrected that to 19,660 B before anything else moved.
* **Pass:** the build fits with margin left for steps 1-5. If it does not, stop and take Alternative 1.
* **Reversible:** yes — delete the fields and the channel entry.

### Step 1 — Spike: throughput cost (no behaviour change visible to the user) — done
* **Do:** thread an `Option<&Bucket>` through `fetch_block` and `single_attempt` and add it to the two
  `charge`/`read_slice` calls, with every value `None` so behaviour is unchanged.
* **Measure:** chaos server, one fixture, 32 streams, run A = no cap, run B = 500 KB/s on the route the streams
  actually use. Compare throughput and spread across at least three runs each, on this box's 6 cores / 12
  threads.
* **Pass:** the difference is explainable as the cap itself, not as overhead. Report both numbers; the design doc
  deliberately leaves the acceptable regression unnumbered — it is the user's ruling.
* **Note:** this is the cheapest place to discover that a third bucket costs real CPU, and it is before any
  registry exists.

### Step 2 — State and registry, watcher-populated — done
* **Do:** `network_buckets` and `usage_cache` on `AppState` as `Arc<RwLock<BTreeMap<…>>>`, initialised empty at
  [lib.rs:304](../../../app/src-tauri/src/lib.rs#L304) and populated/rebuilt inside `watch_network_changes`
  ([lib.rs:238](../../../app/src-tauri/src/lib.rs#L238)); the resolve-and-`set_cap` helper; and a property-change
  emit at [lib.rs:276](../../../app/src-tauri/src/lib.rs#L276) — *that last item is written as if a new emit were
  added there. What shipped is the fingerprint widened, so the emit that already existed now covers property
  changes; see **Independent value**.*
* **Proves itself:** the boot race cannot recur, because nothing reads `networks` before the watcher writes it.
* **Test:** a Rust test that a `Bucket` with cap 0 is unlimited — the semantics that make a quota a separate
  mechanism — plus one that `set_cap` on a shared `Arc` is seen by a holder from before the call.
* **Independent value:** the spin loop. Before step 2 the watcher's unchanged pass `continue`d over its own sleep,
  so the one branch that runs every session re-enumerated adapters as fast as the processor allowed. That is what
  this step actually fixed on its own. The emit this step was planned around was wrong in two ways, corrected in
  [the measurements](#step-2--done-2026-11-03) and in the design doc's Architecture §1.
  * *It said* a property-change emit fixed a live defect: "an adapter renamed or toggled in one window currently
    never reaches another." There is no other window — `tauri.conf.json` declares `main` and no
    `WebviewWindowBuilder` exists — and a toggle already refetches in the acting renderer
    ([main.ts:5282-5287](../../../app/src/renderer/main.ts#L5282)).
  * *It said* the emit at [lib.rs:276](../../../app/src-tauri/src/lib.rs#L276) was the new part. The emit already
    existed inside `if announced`; the gap was that the **fingerprint carried only identity plus the local IP**, so
    a Windows-side rename rewrote neither the row nor anyone's view and never reached that branch at all. Step 2
    widened it with five fields in one `format!` ([:252-263](../../../app/src-tauri/src/lib.rs#L252))
    and left the emit alone.
* **As shipped:** the population moved from a pair of new queries to the adapter pass that already held both
  answers ([ipc.rs `network_payload`](../../../app/src-tauri/src/ipc.rs)), so `network:list` refreshes the
  registry as well as the watcher; the resolve returns `Option` rather than an un-capped bucket; the cap-0 test
  was already in `limit.rs`; and the write handlers do not emit, because the app has one window. See
  [the measurements](#step-2--done-2026-11-03).

### Step 3 — Enforcement on the parallel path — both halves done
* **The over-quota half was held on ruling 1, and shipped the same day as step 3b** once the four rulings were
  chosen ([the measurements](#step-3b--over-quota-ejection-done-2026-11-04)); the rate-cap half is
  [step 3's own measurements](#step-3--done-2026-11-04).
* **Do:** build `Vec<Option<Arc<Bucket>>>` from `routes` beside `agents` in `parallel_download`
  ([job.rs:1073](../../../app/src-tauri/src/engine/job.rs#L1073)), carry it in `WorkerEnv`
  ([job.rs:2031](../../../app/src-tauri/src/engine/job.rs#L2031)), and replace the placeholder `None` that step 1
  left in each `fetch_block` call ([job.rs:2162](../../../app/src-tauri/src/engine/job.rs#L2162),
  [:2366](../../../app/src-tauri/src/engine/job.rs#L2366)) with that route's bucket. `fetch_block` already takes
  `net: Option<&Bucket>` ([:2497](../../../app/src-tauri/src/engine/job.rs#L2497)) and already folds it into the
  charged slice ([:2567-2574](../../../app/src-tauri/src/engine/job.rs#L2567)), so no signature work is left here.
  *As shipped this differs in one way:* the frozen `Vec` of buckets became a frozen `Vec<Option<i64>>` plus a
  per-attempt registry read, for the reason recorded in the measurements. The `None` replacements and the
  `WorkerEnv` carrying happened exactly as written.
* **Already pre-proven, so this step is wiring rather than a gamble:** step 1's socket arm, re-run in release
  codegen on 2026-11-03, throttles 32 real streams to 2.05 MiB/s with the cap on the *network* bucket exactly as
  it does with the cap on the job bucket (2.04), and an extra open bucket leaves the rate at 2.05 — the slice
  intersects rather than adds, over real sockets, before any production caller exists. What step 3 still has to
  prove is *selectivity*: a capped route respects its cap while an uncapped route in the same job does not slow.
* **Test:** engine test constructing `RunContext` directly, asserting three buckets intersect rather than add,
  and that a capped route reaches roughly its cap while an uncapped route in the same job does not slow. Run the
  timing part with `cargo test --release` — debug inflates a per-bucket marginal cost ~30× and is not a
  conservative bound, it is the wrong number.
* **Budget:** this is the step most likely to break the ceiling. 10,956 B left, and the `Vec` build, the registry
  reads and the per-route index are new work in the hot path. Step 1's measured 0 B should not be read as
  "enforcement is free": the *inert* shape cost nothing observable, and whether that is because three slice
  elements really are that cheap or because the size quantum hides a few hundred bytes is **not known** — the exe
  has only ever moved in multiples of 512 B (every measured delta so far: 2,048, 3,072, 8,704, 10,752, 17,408).
  Step 3 additionally makes the arms live at run time, which
  cannot be cheaper than 0 B. If it does not fit, the size-ceiling ruling comes back before any renderer work —
  not after.

### Step 4 — Ramp attribution — done 2026-11-04
* **Do:** `RampSample` records which scope capped the sample instead of a job-wide boolean
  ([job.rs:1530](../../../app/src-tauri/src/engine/job.rs#L1530), field at
  [:1760](../../../app/src-tauri/src/engine/job.rs#L1760), struct at
  [:1747](../../../app/src-tauri/src/engine/job.rs#L1747)), so one capped route does not freeze the Auto band.
* **Test:** the existing ramp tests must stay green with no cap set; add a case with one route capped and assert
  the job still grows width.
* **Sequencing reason:** before this step, turning on any cap silently breaks a shipped feature. This step must
  not be deferred past step 3.
  **Step 3 was shipped, which made this the step that had to land before step 6 — and it has now landed, so step 6
  is cleared to make a cap switchable.** The condition the
  note predicted is real and its exact shape is now known — and it is the *opposite* failure from the one the plan
  feared. `RampSample.capped` reads
  `ctx.limit.cap().is_some() || ctx.global.cap().is_some()`
  ([job.rs:1530](../../../app/src-tauri/src/engine/job.rs#L1530)) and never looks at the adapter registry, so a cap
  on an adapter is invisible to it: the band is never frozen by an adapter cap (the goal holds, by accident), and a
  window paced only by an adapter bucket is recorded as fair, uncapped evidence and can keep the Auto band climbing
  toward 32 on a link already at its ceiling. Nothing overshoots the cap — the buckets intersect — but the signal is
  wrong in both directions at once. Nothing can set such a cap from the UI until step 6, which is the only reason the
  break is latent rather than live; a cap set through the bridge or the database is already enough to produce it.
* **Widened by step 3b.** The same field is now blind to a second limiter: an adapter that has met its
  allowance stops being claimed, and the window the remaining adapters produce is still recorded as ordinary
  fair evidence. So the sample has to name *which* scope capped it — rate or allowance, and which adapter —
  because "one route was shut and the others carried the file" is a bond that worked, not a link at its
  ceiling, and billing those bytes to the growth bar is how the Auto band stalls high.
* **As built.** The sample names no scope. It carries `host_bytes` — the window's total minus what left on a
  route whose adapter had a cap in force, and zero when a job or app-wide cap is set — and `shifted`, which says
  the per-route cap-and-allowance signature differed across that window. `capped: bool` is gone; `next_width`
  prices the growth bar *and* judges the window on `host_bytes`, and a shift **voids** the bar rather than
  answering it. Naming a scope would have settled nothing, because the two answers available — `Hold` (freeze the
  Auto band, which the design forbids) and "fair" (repeat the bug) — are both wrong for the same field. What the
  ramp can act on is how much of the window the host set. See the measurements above.
* **Now closed.** The paragraphs under *Sequencing reason* and *Widened by step 3b* describe the defect as it stood
  before this step — adapter caps invisible to the sample, and an ejected adapter billing its survivors' bytes to the
  growth bar. Both are what the shipped `host_bytes` / `shifted` pair answers, so read those two paragraphs as the
  bug report this step fixed, not as an open condition. The test the *Test* line asked for — one route capped, the
  job still grows — is `a_capped_adapter_does_not_silence_the_route_beside_it`.

### Step 5 — The non-range path — done 2026-11-04, inside step 3
* **Do:** one argument. `single_attempt` already takes `net: Option<&Bucket>`
  ([job.rs:2837](../../../app/src-tauri/src/engine/job.rs#L2837)) and builds its charged slice at
  [job.rs:2889-2898](../../../app/src-tauri/src/engine/job.rs#L2889), read at
  [:2905](../../../app/src-tauri/src/engine/job.rs#L2905) and charged at
  [:2911](../../../app/src-tauri/src/engine/job.rs#L2911) — step 1 shipped all of that. What was missing is that
  its only caller handed it `None`; it now takes the bucket of the route `single_download` chose
  ([job.rs:2703-2704](../../../app/src-tauri/src/engine/job.rs#L2703), `routes.first()` falling back to
  `Route::default_route()`), bound at [:2733](../../../app/src-tauri/src/engine/job.rs#L2733) and passed at
  [:2737](../../../app/src-tauri/src/engine/job.rs#L2737), inside the retry loop rather than outside it.
* **Test:** `a_network_cap_paces_the_single_stream_too` — a fixture server that refuses `Range`
  (`?norange=1`), one capped route, the transfer respects the cap.
* **Sequencing reason:** this is the step most likely to be skipped and the one that decides whether the feature
  is true or decorative. **It shipped with step 3**, as required.

### Step 6 — Write path and UI — done 2026-11-04
* **Do:** the two controls in `netCard`
  ([main.ts:4793](../../../app/src/renderer/main.ts#L4793)) labelled rate cap and data allowance, persisting on an
  explicit Save or blur through `network:set-limits`. **Everything else this step was written to do is already
  shipped**: `db::update_network_limits` and the dispatch arm came in step 1 ([ipc.rs:203](../../../app/src-tauri/src/ipc.rs#L203)),
  both `CHANNELS` lists ([ipc.rs:31-33](../../../app/src-tauri/src/ipc.rs#L31) and
  [ipc.ts:155-157](../../../app/src/shared/ipc.ts#L155)), the `IpcContract` entry
  ([ipc.ts:81](../../../app/src/shared/ipc.ts#L81)) and the 33-channel expectation
  ([verify-m0.mjs:45](../../../app/scripts/verify-m0.mjs#L45)) in step 0.
* **Ruling 2 said** this step carries its consequence: on a machine whose routes carry no adapter id, both controls
  labelled inert with the reason. **As shipped, that half is not there and the premise under it is gone** — the
  launched run attributes this box's default route, so both controls are live here and were measured live
  ([the measurements](#step-6--card-controls-done-2026-11-04)). The engine still gates only attributed routes
  (`bytes_no_adapter_can_be_credited_for_charge_no_allowance`), and on a genuinely unattributed machine a card
  limit would still be silently inert. That gap is open, not closed; the chip titles and the popover note name the
  scope but do not warn about its absence.
* **Verify in the shipped binary:** relink, launch, set a cap in the real window, read it back after a restart, and
  watch an adapter unplug-and-replug re-read the deck. **Corrected after step 2 shipped:** the original instruction
  was "confirm a second window refreshes", which cannot be done — `tauri.conf.json` declares one window (`main`) and
  no `WebviewWindowBuilder` exists in the Rust, so there is no second renderer to observe. The only emit is the
  watcher's, on its changed branch ([lib.rs:276](../../../app/src-tauri/src/lib.rs#L276)). The `--smoke` harness
  cannot cover any of it either, because `setup` returns before the watcher starts
  ([lib.rs:325-327](../../../app/src-tauri/src/lib.rs#L325)) — since step 2 the harness does populate the registry,
  through `network:list`, but it never proves enforcement.
  * **Done that way.** Cap typed into the popover through the browser's own `Input.insertText`, saved, paced
    (0.2479 of uncapped on the same host, 4 rotated passes), and read back after an `app.quit` + relaunch on the
    same data dir with the field prefilled. Allowance saved through the same popover on a fresh data dir: shut
    while the ledger was met, `served` frozen 10.1 s with `status downloading`, running again 988 ms after the
    popover cleared it. The one instruction's item that no run here can reach is the unplug-and-replug, which needs
    a second adapter, and the deck-refresh that step 2's watcher owns was instead proven on the tick path — the
    chip crossed to `is-shut` without a click.
* **Note:** `src/shared/models.ts` needs no change — `speedLimit`, `dataLimit` and `dataLimitPeriod` are already
  declared ([models.ts:73-75](../../../app/src/shared/models.ts#L73)).

### Step 7 — Ledger correction — done 2026-11-04
* **Do:** correct the M2 row at [app-design-plan-v2-engine.md:53](../../../design/app-design-plan-v2-engine.md)
  so per-adapter caps read as this feature, not as shipped 2026-11-01.
* **Sequencing reason:** it belongs last, with the binary that proves the claim, not first as a promise.
* **As done:** the M2 row now says the cap clause was *not* part of what shipped on 2026-11-01 — the columns and
  the socket binding were, nothing read or wrote them — and points at this feature with the two launched-exe numbers
  beside it. The M0 row's present-tense ledger in the same table was re-measured in the same pass (48 checks, 33
  channels, 4,506,112 B, 152/6), because leaving it stale would have replaced one overstatement with another. Docs
  only: 0 B.

### The ramp plateau defect (found by step 4's launched run, fixed 2026-11-04) — done
* **The defect:** in that run's eight uncapped passes the Auto band never opened past nine streams — one
  `ramp-grow` row and then silence, every pass, while the host kept offering one MiB/s per connection. The rule was
  not wrong about the host; it was wrong about the window it was reading.
* **Two mechanisms, now measured rather than inferred.** `Msg::Tick` carries the time it was *sent* and the
  handler records `lag = now − sent`, so the two clocks are separable. The task-97 run's first window of a loaded
  transfer arrives **2,907-6,432 ms late** (`ramp-grow` rows carry `lag` 6432 / 3713 / 2907 / 3748 / 3381), and its
  `span` is therefore 3.9-7.4 s of the whole transfer; after it drains, ticks land with `lag` 0-73 ms. Under the
  shipped ordering the growth bill was answered *before* the window-age gate, so a 22-90 ms sliver of the drained
  backlog — and, worse, a full-length window asked to produce a 6.6 s window's byte **total** — could latch
  `plateau` for the rest of the job.
* **The rule as shipped now** ([job.rs `next_width`](../../../app/src-tauri/src/engine/job.rs)): `shifted` voids
  the bar and holds; an unfair window (quiet / nothing landed / whole bond paced) holds; then **two clocks gate every
  judgement** — the cadence since the last action *and* how long this window actually measured both have to be at
  least `window_ms`; then the bar is judged as a **rate**, cross-multiplied (`host_bytes × bar_span` against
  `bar × span`), which is the same arithmetic the totals were on the equal spans a healthy job produces. The
  coordinator keeps `ramp_window_ms`, `ramp_landed` and `ramp_limited` and closes a window only on a decision or on a
  hold that was a verdict *about* that window (quiet / paced / shifted); "too young to be a measurement" keeps
  accumulating instead of throwing the bytes away.
* **Evidence in the row:** `ramp-grow` gained `span` + `lag`; `ramp-plateau` gained `span`, `lag`, `host`, `bar` and
  `bar_span`, so a refusal can be re-derived by hand from the history table alone. The first pilot of this fix
  stopped at nine streams and there was no way to say whether that judgement was right — the fields exist because
  that question could not be answered.
* **Tests:** two new pure cases in [job/tests.rs](../../../app/src-tauri/src/engine/job/tests.rs) — a 22 ms window
  is not an answer to the bill (the bar stands, the band climbs again on the next full window), and a full window at
  the *old* rate is still evidence against the growth. Both written against the shipped rule first: they failed
  with `left: Plateau, right: Hold` (tests.rs:2302, :2328). The counterfactual was then run in both directions — the
  old branch order and the old byte-total comparison restored, both tests fail; the new rule restored, both pass.
  `cargo test --release -- --test-threads=1`: **154 passed / 6 ignored**. `npm run typecheck` clean (no renderer
  file changed), `npm run verify` **M0 GATE: PASS**.
* **Launched exe, same harness as step 4** ([cap-run.mjs](../../../../_cap-harness/cap-run.mjs) →
  `report-2026-11-04T00-08-50-448Z.json`, exe 4,507,136 B, 1 GiB file, 8 MiB blocks, per-connection pacing at
  1 MiB/s, three rotated rounds + the two mid-run switch passes, each pass sampled at the fixture):

  | pass | `ramp-grow` rows | `streams_final` | wire rate |
  | --- | --- | --- | --- |
  | `free#1/#2/#3` | 2 / 5 / 5 | **10 / 13 / 13** | 9.72 / 12.23 / 12.19 MB/s |
  | `capped#1/#2/#3` | 0 / 0 / 0 | 8 / 8 / 8 | 2,010,943 / 2,013,090 / 2,020,795 B/s |
  | `host-limited#1/#2/#3` | 1 / 1 / 1 | 9 / 9 / 9 | 8.01 / 8.01 / 8.00 MB/s |
  | `cap-applied` | 1 before the switch, 0 after | 9 | 5.39 MB/s |
  | `cap-released` | 0 under the cap, 4 after | 12 | 7.53 MB/s |

  The pass criterion was `streams_final > 9` with more than one growth row: met, 20 `ramp-grow` rows across the run.
  Every plateau in the run is arithmetically justified on its own numbers — `host-limited#2`: asked
  37,322,753 B over 4,382 ms (8.52 MB/s), got 13,631,488 B over 1,702 ms (8.01 MB/s), which is a host that hands out
  8 MB/s to the whole connection set, so the ninth stream genuinely bought nothing; `free#2`: asked 14.06 MB/s, got
  12.74 MB/s, which is this link's ceiling. The `host-limited` control is the point: a free host climbs and a
  one-clock host still stops, so "stopped" is not the limiter's doing and "climbed" is not luck.
* **Byte cost: 1,024 B** in two relinks (rule +512 B to 4,506,624 B, telemetry +512 B to **4,507,136 B**); headroom
  is now **1,740 B** against the 4,508,876 B gate. The two fields that were tempting — a `settled` grace window after
  each growth — were **not** added, because the pilot measured the opposite: the ninth stream paid for itself inside
  its first window (16,515,072 B over 1,796 ms = 9.2 MB/s against a 8.55 MB/s ask), so a grace rule would have been
  a fix for a cost that this link does not have.
* **What it still does not show.** The depth of the climb varies pass to pass (10, 13, 13) and every pass ends in a
  latched `plateau`, which is the designed semantics — the rule stops growing for the rest of the job after one
  refusal — so a transient dip on a link that tops out mid-file permanently caps the width; whether that is the
  right thing for a 100 GB job is **not measured here**. And the `verified_bytes` lag seen in every pass (e.g.
  `free#1` wire 291 MiB against a smaller ledger figure at the same instant) remains the held defect step 4 opened:
  cause still not isolated.
* **The latch is closed as considered-and-rejected — the user's ruling, 2026-11-04.** Asked whether a latched
  `plateau` should ever clear, the user's answer was to leave it recorded as **"considered, rejected, reopen on that
  evidence"**. Recorded here as the user's own ruling; it is *not* written `DECIDED`, because that marker stays
  theirs (see the note at the top of this document). The reasoning that went with it, for whoever reads this after
  the fact: a plateau means a fair window came back and did not pay the last growth, which is a fact about the host
  at that width rather than a mood, and every *circumstantial* refusal already leaves by another door — a bond that
  changes voids the bar via `shifted` instead of plateauing, and a quiet or zero-byte window takes the Hold branch
  without being charged. A time-based re-test would turn the ramp back into a probe loop paid for in the user's
  download bytes, which is what the whole evidence rule exists to prevent; the cost of keeping it is bounded, at
  most `(32 − plateau width)` streams for one job, since `RampState` is rebuilt per job.
  **The evidence that would reopen it** is a measurement, not a feeling: a launched run whose `ramp-plateau` rows —
  they now carry `span`, `lag`, `host`, `bar` and `bar_span` — show a long job sitting several streams below what the
  same host demonstrably kept paying afterwards. Price the rule in bytes before writing it; headroom is 1,740 B.

## Where this can stop
Steps 0 and 1 are spikes and cost nothing if they fail. Steps 2, 4 and 5 each fix or protect something real on
their own. Step 3 without step 5 ships a cap that a plain HTTP/1.0 server walks through — which is the same kind
of overstatement the M2 row is, so 3 and 5 are one unit.

## Gate list per step

Every step ends with: `cargo build`, `cargo test`, `npm run typecheck`, `npm run build`, `npm run verify`, and
`stat` of the release exe. Steps 3 through 6 add the launched-app check, because a passing gate does not prove
the watcher-populated registry works — the harness never starts the watcher.
