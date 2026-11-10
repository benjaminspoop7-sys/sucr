# The byte sample — a moved validator now asks the disk before it costs a partial

Record of the slice that made "the host changed its stamp but not its bytes, and Sucr keeps what it
already downloaded" true in the shipped exe. Written 2026-11-06 under the standing delegation ("Do as
you see fit"), opened by "Yea, do it." and the gap list "So what's missing in our app that Plexo has
now?". Every claim carries a `writer:` / `predicate:` / `evidence-noun:` or is marked `unmeasured`;
confidence words are not evidence. A locator is not evidence until its lines were re-read today.

## 1. Why this slice

The previous slice ([resume-validator-and-keep-awake.md](resume-validator-and-keep-awake.md)) taught
the resume path to *notice* a rotated `Last-Modified`. Noticing is half the job: a static server that
rebuilds an artifact publishes a new stamp over the same bytes, and the honest answer then is to keep
the 400 MB already on disk rather than re-pull it. Plexo does this by reading spread samples of the
file and comparing them. Before this slice Sucr had the opposite behaviour twice over — on a Resume
and on a relink — and no way to say which decision the bytes were behind.

## 2. Mechanism — one hearing, in front of both byte-losing decisions

The evidence noun is the **staging file's own bytes**, read at spread offsets and compared against the
same offsets fetched from the host. The writer is the engine, at two sites:

| site | file:line | what the verdict can change |
| --- | --- | --- |
| A — resume pre-flight | [job.rs:792-824](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L792) | `reset_progress = false` only on `matched` ([:805-807](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L805)) |
| B — relink (`refresh_url`) | [job.rs:3945-3976](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3945) | a `refresh-changed` refusal becomes a take, with `validator: "sample"` ([:3975](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3975)) |

The guard rails are in the predicate, not in a fallback: the hearing only opens when
`reset_progress && !plan_changed && probe.accept_ranges` and a staging file exists
([:793-794](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L793)). A moved **length** is the louder
claim and is never argued with; a host that will not serve ranges, and a partial nobody is holding,
get no hearing. Site B fires only inside the `Err(e) if e.code == "refresh-changed"` arm, so a link to
another file at a different length still fails exactly as before.

**Direction of the safety.** The sample can only ever *rescue* bytes: it runs solely where the old code
had already decided to discard them, and `differed`, `inconclusive` and `no-evidence` all leave the old
decision standing. Nothing this slice adds can lose a byte that the previous build would have kept.

### 2.1 What counts as evidence — the finding that rewrote the design

The first shape of this slice sampled a **contiguous run of finished blocks** and reported the field as
`prefix`. Launched-exe report `resume-report-2026-11-06T05-32-37-582Z.json` falsified it: a bonded job
parked by Pause leaves every block half-full and **none** `state = 'done'`, so the run was empty and the
hearing answered `prefix: 0` → `no-evidence` at every real pause. The feature was dead in the common
case while 216 engine tests stayed green, because each test seeds whole finished blocks — a control
group identical to its test group.

The shipped shape reads the **per-block received front** instead
(`covered_ranges`, [job.rs:3196](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3196)): for each
block, `min(bytes_done, range_len)` clamped to what the staging file actually reaches, skipping any
front shorter than one window. `spread_windows`
([:3249](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3249)) then places 8 × 16 KiB windows
([:3175-3176](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3175)) evenly across that covered
space and never outside a range. The payload noun is `covered`
([:818](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L818)); `prefix` is retracted everywhere
(§6).

The clamp is the other half of the same finding: `bytes_done` is a count, not a promise about the file.
A row that says 512 KiB landed where the file does not reach is answered by reading nothing — engine
test `a_partial_whose_file_fell_behind_its_ledger_leaves_the_sample_nothing_to_read`. And a tail-split
hole reads preallocated zeros against the host, disagrees, and leaves the old discard in place —
`a_hole_costs_its_own_block_and_not_the_blocks_behind_it`. The sample never widens what may be trusted.

### 2.2 Outcomes

| outcome | when | effect on the partial | written by |
| --- | --- | --- | --- |
| `matched` | every window fetched 206 and equalled the disk | kept; site A skips the reset, site B takes the link | both |
| `differed` | the first window that disagrees ends the hearing (`windows` = how many agreed first) | discarded — the old decision | both |
| `inconclusive` | no 206, short body, unopenable file, cancelled mid-hearing | discarded — the old decision | both |
| `no-evidence` | nothing wide enough on disk, or no ranges / no file | discarded — the old decision | A only |

Site A always writes the row when the hearing opened
([:811-824](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L811)); its absence means the sample
never ran, which is a leg failure, not a pass. Site B folds its verdict into the `url-refreshed` row it
writes only when it says yes
([:3999-4008](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3999): `sampleOutcome`,
`sampleWindows`, `sampleCovered`), so a refusal carries the byte reason nowhere but the thrown message.

The renderer reads the same nouns: `case 'bytes-sampled'`
([main.ts:4108](file:///D:/RE/Sucr/app/src/renderer/main.ts#L4108)) labels the four outcomes and says
"N samples inside <covered>"; `validator === 'sample'` renders as "bytes matched"
([main.ts:4160](file:///D:/RE/Sucr/app/src/renderer/main.ts#L4160)).

## 3. Tests

`cargo test --release -- --test-threads=1` at this exact tree: **223 passed / 0 failed / 6 ignored**.
The sample's own names, in [job/tests.rs](file:///D:/RE/Sucr/app/src-tauri/src/engine/job/tests.rs):
`a_rotated_stamp_with_no_etag_is_answered_by_the_bytes_and_keeps_them`,
`a_moved_validator_over_differing_bytes_still_drops_the_partial`,
`a_host_that_ignores_range_gets_no_hearing`,
`a_partial_whose_file_fell_behind_its_ledger_leaves_the_sample_nothing_to_read`,
`the_sample_windows_are_spread_over_the_bytes_the_disk_holds`,
`a_hole_costs_its_own_block_and_not_the_blocks_behind_it`,
`a_link_that_only_changed_its_claim_is_taken_because_the_bytes_agree`,
`a_relink_whose_bytes_disagree_is_still_turned_down`,
`a_link_to_another_file_is_turned_down_and_the_row_is_left_alone`.

## 4. Byte cost (measured with `stat`, never a hash)

| state | bytes |
| --- | --- |
| `sucr-held.exe`, the build before the validator slice | 4 579 328 |
| the first shape of *this* slice (contiguous-run, `prefix`) | 4 592 640 |
| shipped shape (`covered_ranges` + spread + clamp) | **4 593 152** |
| slice total vs the validator build | **+13 824 B** |
| redesign alone | +512 B |

Granularity is 512 B, so the split above is the honest limit of what two `stat` reads support; no
per-function claim is made. No new IPC channel (still 35), no new setting key (still 24), schema still
v4 / 17 tables, one new history kind — `scripts/build.mjs` counts 32 kinds written by the engine, all
formatted by the UI. Ceiling 5 MB (RULED 2026-11-04); headroom now **649 728 B**. M0 gate PASS,
typecheck clean.

Content, not size, was proved in the binary: a byte scan of the two exes counts `bytes-sampled` 0 → 2,
`sampleCovered` 0 → 1, `no-evidence` 0 → 3, while `refresh-changed` stays 1 → 1 — the refusal path is
unchanged, the hearing is new.

## 5. Launched-exe proof — `D:\RE\_cap-harness\resume-run.mjs`

Eight legs, each its own `--data-dir`, WebView2 dir, out dir and CDP port, against the fixture on 8151
(8 MiB, `Range` honoured, ETag only when the leg asks). Both exe sizes are pinned before anything
launches. Three consecutive full passes, 48 checks each, `allLegsPass: true`, `blocked: []`:
`resume-report-2026-11-06T06-08-59-883Z.json`, `…T06-10-57-452Z.json`, `…T06-12-47-923Z.json`.

| leg | establishes | the numbers it rests on |
| --- | --- | --- |
| `a_rotated_stamp_over_unchanged_bytes_is_answered_by_the_bytes` | the meaning of the earlier slice's claim changed: a stamp move now *asks* the disk | park leaves `finished: 0` of 8 blocks, `held: 4 194 304`; hearing `{validator: "last-modified", covered: 4 194 304, windows: 8, outcome: "matched", kept: true}`; zero `resume-reset` rows read after the run finished; wire 4 325 377 = the 4 194 304 remainder **+ 131 073**, i.e. the 8 × 16 KiB windows and one straggler |
| `an_unmoved_stamp_keeps_the_bytes` | control: nothing moved, nothing thrown away | unchanged from the validator slice — no hearing, no reset, remainder only |
| `previous_build_kept_the_stale_bytes` | the old build keeps the same bytes **and says nothing about why** | no `resume-reset`, `resumed` + second `probe` prove the press reached the host, 4 194 304 stale bytes carried into a `finished` row |
| `a_rotated_etag_over_unchanged_bytes_keeps_the_partial` | site A with an ETag, the stronger validator | `{validator: "etag", covered: 4 194 304, windows: 8, matched, kept}`; no reset anywhere after the claim moved; remainder + samples only |
| `a_rotated_etag_over_changed_bytes_still_drops_the_partial` | the sample is not a rubber stamp | one byte flipped at absolute 4096 → `{windows: 0, outcome: "differed", kept: false}`; the `resume-reset` row carries `{validatorChanged: true, sampleOutcome: "differed"}`; host asked for 8 404 994 — the whole file again |
| `a_relink_whose_bytes_agree_is_taken` | site B takes a link that only changed its claim | `url-refreshed` `{validator: "sample", sampleOutcome: "matched", sampleWindows: 8, sampleCovered: 4 194 304, keptBytes: 4 194 304}`; the next Resume then writes **no** reset and **no** second hearing — the validator stored with the new link settles it — and asks for 4 194 305 |
| `a_relink_whose_bytes_differ_is_turned_down` | site B still refuses a different file | thrown `the new link serves a different file ("v1" vs "v9")`; no `url-refreshed` row; url and half intact. The byte reason is carried by the differential with the leg above: same claim shape, nothing but the flip changed |
| `previous_build_dropped_unchanged_bytes` | the control for the whole slice, on `sucr-held.exe` | the same rotated ETag produced `resume-reset {validatorChanged: true, planChanged: false, stagingMissing: false}`, the file was re-pulled, and **no `bytes-sampled` row exists anywhere in its trail** — it cannot write one |

## 6. Retractions this pass forces

- **"A rotated stamp costs the partial."** Superseded by this slice: it now costs the partial only when
  the bytes disagree. The validator-slice record's own row
  ([resume-validator-and-keep-awake.md §5](resume-validator-and-keep-awake.md)) describes the behaviour
  as of its build; it is not this build's behaviour.
- **`prefix` as the evidence noun.** The contiguous-run field is gone; a report that names `prefix: 0`
  describes the falsified first shape, not the shipped one. Nothing else in either record changed.
- **"Plexo's remaining advantage here is the 8 spread byte samples — Sucr discards the partial
  instead."** Closed; see the Plexo gap study in memory, and §8 for what is still missing.

## 7. Harness traps this pass produced (each cost a wrong answer first)

1. **A derived claim about a stored number.** "`verified_bytes` is a contiguous prefix" read as a fact
   because the column is a sum of per-block counts. It is not a span, and a live pause proves it. Only
   the launched ledger shape (`finished: 0`, `held: 4 MiB`) caught it — the engine suite could not,
   because it seeds whole blocks.
2. **A trail snapshot cannot speak about a row written later.** The reset follows the hearing row, so
   reading `resume-reset` presence *or absence* out of the hearing's own event list is a race. It failed
   one leg once and would have made every "no reset" check unsound; both now read the whole trail after
   the run reaches a terminal state.
3. **The harness's own clip can fail a check.** `must()` truncated the thrown message at 180 chars, so
   the relink-refusal check failed on the harness, not on the app. Raised to 400 with the reason in place.
4. **A row's status is not the job's liveness.** The relink guard reads the `status` column
   (`refresh-running`, [job.rs:3908-3913](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3908)), and
   a row that has just been paused can still answer `downloading` a moment later. One pass lost the
   relink leg that way — the same leaving-the-registry gap as the open dropped-Resume defect, now
   witnessed from a second call site. The legs wait for two consecutive `paused` reads and keep the
   observed flip in `notes.relinkable` rather than hiding it.

## 8. Still absent after this slice

- Nothing is fetched concurrently with the transfer: the 8 windows are paid for serially at the front of
  the second run, which is the +131 KiB the wire shows. A hung window holds the Resume until the job is
  cancelled — no per-window timeout was spent. `writer:` the loop in
  [job.rs:3285](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3285); `predicate:` the `cancel`
  check per window.
- No surface asks the user. The hearing is automatic and its verdict is only readable afterwards, in the
  history trail; there is no "keep these bytes?" prompt.
- A moved **length** is still decisive without argument, by design (§2). A host that keeps one length and
  genuinely changes the file outside all 8 windows is not caught here — spread sampling is a bet on
  rebuilding, not a checksum. Verified checksums remain the answer for that, and are absent.
- `differed` at site B leaves no trail row of its own (§2.2); the reason lives in the thrown message only.
- The OS-side keep-awake leg is still `blocked` pending one elevated `awake-watcher.ps1` run.
- ~~**The dropped-Resume fix is still not taken.**~~ — **CLOSED 2026-11-07**, one quantum (+512 B,
  4 579 840 → 4 593 664 B), both call sites guarded. See `resume-validator-and-keep-awake.md` § 4 and § 6.

## 9. Step-0 spike cost — CLOSED AS A RECORDED DEVIATION, 2026-11-07

`predicate:` the record this section closes was written at the time of the byte-sample slice and stated
that **no separate spike build was run to isolate the slice's own cost from the validator slice's**; the
only cost evidence for the byte sample is `stat` on the two builds actually shipped, and the +13 824 B
figure in § 4 is the difference between them. `writer:` the slice's own § 4, which already said "Granularity
is 512 B, so the split above is the honest limit of what two `stat` reads support; no per-function claim is
made."

**What was owed, and why it is not owed now:** a spike would have relinked a build with the byte sample and
nothing else, to quote the slice's own price. The slice shipped at 4 593 152 B and the next change — the
Resume-swallow fix — landed at 4 593 664 B, **+512 B**, the same single quantum the validator slice cost. Two
consecutive changes each landing in one quantum is exactly the situation a spike exists to separate, and the
second one is now measured rather than predicted: `cargo build --release`, `stat`, and a byte scan of the new
exe counting `pause this download before replacing its link` 0 → 1 while `wait_idle` and
`Duration::from_secs` both stay 0 (the symbol is mangled away in the stripped binary — the size move is the
evidence, the literal is the locator). So the honest closure is: **the byte sample's cost was never isolated,
and the next measured quantum is not separable from it either.** No spike build was run, and this section
exists so the next session does not re-open #187 looking for a measurement that was never taken.
`evidence-noun:` `stat` on `sucr.exe` 4 593 664 B, `resume-report-2026-11-06T22-09-59-211Z.json` (51 checks,
`allLegsPass: true`), `cargo test --release --lib -- --test-threads=1` → 223 passed / 6 ignored.
