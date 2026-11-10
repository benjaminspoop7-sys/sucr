# Per-Network Speed Caps and Data Quotas

* **Author:** Benjamin (machine account; `git config user.name` is unset on this box)
* **Date:** 2026-11-03

## Overview

Give each adapter its own transfer cap and its own byte allowance, and let the user set them.

Today the app declares three per-network columns (`speed_limit`, `data_limit`, `data_limit_period`
at [schema.sql:53](../../../app/src-tauri/schema.sql)) and publishes them to the interface
([ipc.rs:1360](../../../app/src-tauri/src/ipc.rs)), but no code writes them, no control edits them, and the
engine never acts on them. The rate limiter enforces only two scopes: one download and one global cap
([limit.rs:104](../../../app/src-tauri/src/engine/limit.rs)).

The premise this brief started with — "the per-network byte counters are missing too" — was wrong, and
five of six reviewers found it independently. Per-network accounting **already ships**: the
`net_usage` table ([schema.sql:144](../../../app/src-tauri/schema.sql)) is written once per finished attempt
([job.rs:831](../../../app/src-tauri/src/engine/job.rs) calling `add_net_usage` at
[db.rs:720](../../../app/src-tauri/src/db.rs)), rolled up by `network_usage`
([db.rs:741](../../../app/src-tauri/src/db.rs)), published as `usedToday` / `usedWeek` / `usedMonth` /
`usedLifetime` ([ipc.rs:1368](../../../app/src-tauri/src/ipc.rs)), and rendered on the adapter card
([main.ts:4704](../../../app/src/renderer/main.ts)). So this is not a new subsystem. It is the missing
enforcement half of a feature the project already reports as finished.

## Analysis

**The milestone record is wrong, and that matters more than the missing feature.**
`D:\RE\Sucr\design\app-design-plan-v2-engine.md:53` lists M2 as **Shipped 2026-11-01** including
"per-adapter caps incl. metered/cellular wires". No per-adapter cap exists in the binary. The shipped
part is the metered guard, which only holds `Kind::Cellular` out of the bonding pool
([ipc.rs:1348](../../../app/src-tauri/src/ipc.rs)) — exclusion, not capping. Any honesty claim this feature
is meant to support has to survive the fact that the project's own ledger already overstates it.

**The mechanism is mostly wiring.** The limiter is arity-independent by design: it charges a slice of
buckets and sleeps the longest single wait so caps intersect instead of adding up
([limit.rs:92](../../../app/src-tauri/src/engine/limit.rs), proven by the test
`scopes_intersect_instead_of_adding_up` at [limit.rs:176](../../../app/src-tauri/src/engine/limit.rs)).
Adding a third bucket is therefore not new machinery. What is new: a registry that owns one bucket per
network id (nothing holds them today — `global_limit` is a single `Arc<Bucket>` on `AppState` at
[lib.rs:304](../../../app/src-tauri/src/lib.rs)), and threading that bucket to the charge site. The charge
site does not currently know which adapter it is on: `fetch_block`
([job.rs:2493](../../../app/src-tauri/src/engine/job.rs)) receives the job and global buckets plus a
`ureq::Agent`, and only its caller knows the route, so two call sites ([job.rs:2149](../../../app/src-tauri/src/engine/job.rs),
[:2352](../../../app/src-tauri/src/engine/job.rs)) must pass it down.

**Six blockers and majors, in the order that will hurt:**

1. *A zero cap means unlimited.* `Bucket` filters `c > 0` into `None`
   ([limit.rs:36](../../../app/src-tauri/src/engine/limit.rs)), so the intuitive "quota exhausted, set the
   cap to 0" **opens the cable instead of closing it**. A quota cannot be expressed as a bucket value;
   it needs its own gate, polled by the read loop.
2. *On one cable the feature would do nothing, silently.* `Route.network_id` is an `Option` and the
   default route is `None` ([bond.rs:28](../../../app/src-tauri/src/engine/bond.rs)); a single-adapter
   machine never bonds and only gains an id if `attribute_egress` resolves it
   ([job.rs:617](../../../app/src-tauri/src/engine/job.rs)). The same condition already skips the usage
   write, so an unattributed byte is neither counted nor capped.
3. *Any cap freezes the adaptive ramp.* `RampSample.capped` is job-wide
   ([job.rs:1499](../../../app/src-tauri/src/engine/job.rs)) and a capped sample makes `next_width` hold
   ([job.rs:1830](../../../app/src-tauri/src/engine/job.rs)). Shipping a per-network cap as written would
   stop the Auto 8-to-32 band from ever growing. Either the ramp becomes per-route or the two features
   conflict in the UI.
4. *Route choice is round-robin with no capacity weight and no mid-job ejection*
   ([job.rs:1368](../../../app/src-tauri/src/engine/job.rs); the route list is frozen at spawn). A
   quota-exhausted cable keeps receiving an equal share of blocks.
5. *The quota overshoots by design.* Bytes are booked once per attempt
   ([job.rs:829](../../../app/src-tauri/src/engine/job.rs)) in 1-8 MiB blocks, so 32 streams can blow a
   quota by the width before anyone notices. A loss on a hedged race still charges the bucket, so
   bucket debt can exceed `net_usage`.
6. *Size.* Headroom is 20,684 B and the gate's own comment records roughly 11 KB per added IPC
   command ([verify-m0.mjs:28](../../../app/scripts/verify-m0.mjs)). One new write channel consumes over
   half the margin, and every channel added must be mirrored in
   [ipc.ts:147](../../../app/src/shared/ipc.ts) or the "channel lists agree" smoke check fails.

**Naming is a live defect, not a polish item.** The word "limit" currently means a rate, an allowance,
the bucket type, the cap value, and the job scope (`ctx.limit`, `Registry.limits`, `set_limit`).
`speed_limit` beside `data_limit` in the same table reads as two rates. Reviewers recommend renaming in
the payload and the copy first — rate cap versus data allowance — because enforcement lands on the most
overloaded word in the codebase.

**Adjacent shipped bug found during review — FIXED 2026-11-03.** The adapter watch thread
did `continue` when the adapter set was unchanged, which skipped the `sleep(5 s)` at the end of the loop. On a
stable machine it re-enumerated adapters (`GetAdaptersAddresses`) as fast as it can, forever. Step 2 of
[the implementation sequence](per-network-limits-implementation-sequence.md) moved the sleep to the loop itself
([lib.rs:238](../../../app/src-tauri/src/lib.rs#L238)). Correction to this paragraph's original claim: it did
**not** rewrite the `networks` rows per pass — the row write sits in the changed branch the spin could not
re-trigger — so the cost was CPU only, and it was read in source, never measured as a CPU figure.

## Goals vs. Non-Goals

* **In-Scope Goals:** a per-network rate cap that the engine honours at charge time; a per-network data
  allowance built on `net_usage` rather than a second counter; inputs on the adapter card; live
  used/remaining shown in the same units the engine enforces; an explicit answer for unattributed
  bytes; the ramp taught to read caps per route; a ledger correction for the M2 claim.
* **Out-of-Scope Non-Goals:** per-site or per-category or per-queue limits; a calendar reset day or
  timezone database; billing accuracy that competes with Windows' own metered accounting; replacing the
  metered-exclusion guard; a general scope registry as a refactor.

## Alternatives Weighed

* **Delete the three dead columns and publish nothing.** Rejected by the swarm as the honest cheapest
  fix, but kept on the table because it costs zero bytes and removes a lie. Carried forward as the
  fallback if the size spike fails.
* **Widen the metered guard instead of adding caps.** Raised by the zoom-out lens: for "don't eat my
  metered connection", exclusion is strictly stronger than a cap, and the guard's own comment admits it
  catches only the class the OS names ([ipc.rs:1449](../../../app/src-tauri/src/ipc.rs)). Not rejected —
  it is a separate, smaller fix this brief does not cover.
* **Steer blocks away from a weak cable rather than cap it.** The dongle-collapse job is a weighting
  problem, not a ceiling. Held until route weighting exists.
* **A parallel quota counter in memory.** Rejected: it forks a subsystem that already works and creates
  two sources of truth for the same number.
* **Populating the `day`/`week`/`month` columns in `net_usage`.** Rejected: they are deliberately
  written 0 and derived from `period_key` ([db.rs:716](../../../app/src-tauri/src/db.rs)); filling them
  gives the same fact two homes.

## Key Decisions & Staging for Next Steps

1. Fix the M2 ledger entry before this is funded as new work. Draft must open with the defect, not the
   feature.
2. Decide the unattributed-bytes rule first — it is a product choice (no-op, charge the sole card, or
   cap only while bonded) and every later decision depends on it.
3. Spike before drafting: measure the exe delta of one new channel plus its dispatch and write path,
   and measure throughput variance of a three-bucket charge at 32 streams against the chaos server.
   Nothing here is sizeable without those two numbers.
4. Keep the window counter derived, seeded once at job start, written once per attempt as today.
5. Put the cap on `Route` where `attribute_egress` already reads, so the read loop never touches the
   database.
6. Rename before enforcing: rate cap and data allowance, in payload and copy.
7. Make the ramp per-route, or ship caps in a mode that cannot be combined with Auto. Do not ship the
   conflict.
