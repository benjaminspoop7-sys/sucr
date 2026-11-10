# Per-Network Speed Limits & Data Quotas

* **Author:** Benjamin (machine account)
* **Status:** Reviewed — five-stage Goldfish review complete, Stage 7 and Stage 9 findings resolved. **Steps 0-4
  of the sequence are implemented and measured, including 3b** (both spikes passed, the write channel shipped,
  the scope registry is live, **a cap saved on an adapter now limits the transfer that leaves through it** on the
  parallel path and on the non-range path, **an adapter that has met its allowance is no longer claimed** at
  the next block boundary, and **the Auto ramp no longer reads limiter-paced bytes as host evidence** — step 4).
  What is now built too is **step 6: the two controls are on the adapter card** (2026-11-04), so a user can set
  either figure, and both were measured in the launched exe driven only through that card. **The premise of the
  sentence that used to sit here — "no user can set either figure yet, and step 4 is invisible in a launched run on
  this one-adapter machine for the same reason ruling 2 makes a saved cap inert there" — was wrong twice over:**
  ruling 2's inertness does not apply to this box (`attribute_egress` attributes its default route), and step 4 was
  visible in a launched run exactly because of that. The four rulings this document used to call open are
  [chosen in the sequence document](per-network-limits-implementation-sequence.md#the-four-rulings-chosen-2026-11-04)
  under the user's 2026-11-04 delegation ("Chose what you see fit"); they are recorded as chosen, not `DECIDED`,
  because that marker stays the user's. No line marked DECIDED in this document.
* **Date:** 2026-11-03, enforcement landed 2026-11-04

--------------------------------------------------------------------------------

## Overview

Give each network adapter its own transfer cap and its own byte allowance, and let the user set them.

A "bucket" is a token-bucket rate limiter (defined in [limit.rs](../../../app/src-tauri/src/engine/limit.rs)): it holds a balance of bytes, refills at the cap rate, and blocks a read until enough tokens are available. A "scope" is one bucket applied to one transfer — the app already applies two (per-job and global); this feature adds a third (per-network).

## The Problem

Today the app declares three per-network columns (`speed_limit`, `data_limit`, `data_limit_period` at [schema.sql:53](../../../app/src-tauri/schema.sql)) and publishes them read-only through `network_rows` at [ipc.rs:1370](../../../app/src-tauri/src/ipc.rs) (the three fields are emitted at [ipc.rs:1417-1419](../../../app/src-tauri/src/ipc.rs)), but no code writes them, no control edits them, and the engine never acts on them. The only network mutation channel is `network:set-enabled` ([ipc.rs:32](../../../app/src-tauri/src/ipc.rs), dispatched at [ipc.rs:192](../../../app/src-tauri/src/ipc.rs)); there is no channel to update limits. The rate limiter enforces only two scopes: one download and one global cap ([limit.rs:104](../../../app/src-tauri/src/engine/limit.rs)).

*(This paragraph describes the pre-feature state and is now mostly obsolete. Since step 1 `network:set-limits`
exists (dispatch [ipc.rs:203](../../../app/src-tauri/src/ipc.rs)) and `db::update_network_limits` writes the three
columns, so "no code writes them" is no longer true — but "no control edits them" still is, and that is why the
channel has no caller outside tests. Per-adapter buckets are built since step 2, and since step 3 a transfer
charges a third scope: the limiter functions always took a slice of any length
([limit.rs:92](../../../app/src-tauri/src/engine/limit.rs), [:104](../../../app/src-tauri/src/engine/limit.rs)),
and the production caller now supplies the adapter's bucket where it used to supply `None`. What remains untrue is
only the quota half: nothing ejects or refuses a transfer for being over its allowance, and no control writes any
of it.)*

Two naming collisions are already live and this feature lands on top of them. First, `speed_limit` is a column on both `networks` ([schema.sql:53](../../../app/src-tauri/schema.sql)) and `downloads` ([schema.sql:82](../../../app/src-tauri/schema.sql)), and a `setSpeedLimit` handler already writes the per-download one (dispatch [ipc.rs:245](../../../app/src-tauri/src/ipc.rs), `download_set_speed_limit` at [ipc.rs:809](../../../app/src-tauri/src/ipc.rs)). Second, the word "limit" in the engine means a rate, an allowance, the bucket type, and the job scope (`ctx.limit`, `set_cap`, `app_set_speed_limit`). This document therefore says **rate cap** for bytes-per-second and **data allowance** for bytes-per-period in prose and UI copy, even where the column is named `speed_limit`.

**The milestone ledger already claims this is finished.** [app-design-plan-v2-engine.md:53](../../../design/app-design-plan-v2-engine.md) lists M2 as **Shipped 2026-11-01** and names "per-adapter caps incl. metered/cellular wires" as part of that scope. No per-adapter cap exists in the binary. What did ship is the metered guard, which holds `Kind::Cellular` out of the bonding pool — exclusion, not capping; the same ledger row records that limitation itself. The correction is part of this work, not a prerequisite someone else owes.

## Goals

* Enforce a per-network rate cap that the engine honours at charge time, on **both** transfer paths: the parallel block loop and the single-stream fallback used when a server will not honour `Range`.
* Enforce a per-network data allowance using `net_usage` as the record, with an in-memory copy read at block boundaries so the engine never queries SQLite in the hot loop.
* Apply a cap the user edits while a job is running, without restarting that job.
* Expose both controls on the adapter card and persist them through one new IPC write channel.
* Show live used-of-allowance in bytes, from the same `net_usage` window the engine enforces against.
* Leave the adaptive 8-to-32 stream ramp able to grow when only one route in a bonded job is capped. **Met by step 4** (`a_capped_adapter_does_not_silence_the_route_beside_it`): the capped route's bytes are subtracted out of the window the ramp judges, so the free route's evidence still carries the 8→9 step, while `a_capped_bond_offers_the_ramp_no_host_evidence` shows a wholly capped bond offers none.
* Correct the M2 milestone ledger so this is recorded as new work, not as a fix to something shipped.

## Non-Goals

* Per-site, per-category, or per-queue limits.
* A calendar reset day or timezone database. The first version supports monthly allowances only; weekly and daily periods are deferred until the period-reset question is resolved.
* Billing accuracy competing with Windows' own metered accounting.
* Replacing the existing metered-exclusion guard.
* A general scope registry refactor.

## Open Questions

* **Unattributed Bytes — ruled 2026-11-04 (choice 2: a limit gates only attributed routes, and the inertness is shown, not hidden)**: Bytes booked against a route with no network ID are charged to the global bucket only. Per-network caps apply only when `Route.network_id` is `Some` ([bond.rs:28](../../../app/src-tauri/src/engine/bond.rs)). This is consistent with the existing behaviour where unattributed bytes skip the `net_usage` write — the `if let Some(network_id)` guard around `add_net_usage` ([job.rs:856-857](../../../app/src-tauri/src/engine/job.rs)). **Step 3 turned the consequence from a warning into a confirmed fact**: on a machine whose pool has fewer than two routes, `bond_routes` returns exactly one `Route::default_route()`, whose `network_id` is `None` ([ipc.rs:1497-1499](../../../app/src-tauri/src/ipc.rs), [bond.rs:51-58](../../../app/src-tauri/src/engine/bond.rs)), and the only code that can fill the id in afterwards is `attribute_egress` ([job.rs:642](../../../app/src-tauri/src/engine/job.rs), called at [:469](../../../app/src-tauri/src/engine/job.rs)), which needs the routing table to name the interface. A rate cap saved on a one-adapter machine is therefore **inert** — not unproven, inert — and that was ruling 2.
**Retracted 2026-11-04 by two launched-exe runs on this box.** `attribute_egress` does name the interface here: it
credits the unbonded default route with the adapter the routing table leaves on, so `Route.network_id` is `Some(1)`
(Ethernet) with one adapter and no bond, a card-saved cap paced the transfer at a **0.2479** ratio of the same host
uncapped, and a card-saved allowance froze that transfer for ~10 s and released it **988 ms** after the popover
cleared it. The rule stands — an unattributed route is never gated — its reach on this machine did not.
**Chosen as stated above**: no fallback to the sole adapter, because it would make the engine guess which device a
socket left on, which is the guess `attribute_egress` exists to avoid, and on a machine with one *named* adapter
beside a bond it would silently double-charge a link the user did not limit. Step 6 therefore has to label both
controls inert **with the reason on screen** rather than accept a number that does nothing;
`bytes_no_adapter_can_be_credited_for_charge_no_allowance` is the engine proof the card is describing.
**That labelling did not ship, and is an open gap, not a closed one.** With this box's routes attributed, no control
here is inert, so there was nothing to label and the popover spent its bytes on the two working fields instead; the
chip titles and the note state the scope ("Each limit spends only the traffic this adapter carries.") without
warning about its absence. On a machine that genuinely returns an unattributed route, a number typed into that card
would still do nothing silently — the failure mode this ruling exists to end.
* **Mid-job ejection contradicts a documented invariant — ruled 2026-11-04 (choice 1: eject at the block-claim boundary, and show the drift)**: `RunContext.routes` says routes are "resolved when the job was started … and never re-read mid-transfer, so a plug or an unplug cannot move a block's bytes to another device behind the label already shown" ([job.rs:167-169](../../../app/src-tauri/src/engine/job.rs)). Ejecting an over-quota route mid-job does exactly what that comment forbids: it moves future blocks to a different device than the one the route deck labels. The rate cap has no such conflict — the cap value is applied through `Bucket::set_cap` ([limit.rs:52](../../../app/src-tauri/src/engine/limit.rs)), which is interior-mutable, so a shared `Arc<Bucket>` picks up an edit without re-reading routes, and **step 3 proved it end to end**: `a_cap_saved_after_the_job_began_reaches_the_next_attempt` covers the seam and a test that reads the registry per attempt closes the first-cap hole a frozen bucket list would leave. The ejection needed its own ruling, and it has one: **eject at the block-claim boundary**. A block already
claimed finishes on the device it started on, so what is re-derived mid-job is *eligibility within the frozen
route set*, never the set itself — which is the narrower reading the comment's own words support (a plug or an
unplug cannot move bytes; a ceiling the user set can). The seam is the one step 3 already crossed for a cap that
did not exist when the job began, and it is proven the same way:
`an_adapter_out_of_bytes_carries_no_block_while_its_bond_finishes` and
`a_ceiling_raised_while_the_job_runs_gives_the_adapter_its_blocks_back`. The label drift is accepted because the
alternative is worse — a byte past a quota the user named — and because step 6 names the shut route on the
deck rather than letting it look gone: the route stays listed and its chip reads "Over allowance" with the spent
figure over the limit.
* **Non-range servers**: **Closed by step 3.** `single_attempt` takes the optional bucket ([:2837](../../../app/src-tauri/src/engine/job.rs)), builds its slice at [:2889-2898](../../../app/src-tauri/src/engine/job.rs), reads at [:2905](../../../app/src-tauri/src/engine/job.rs) and charges at [:2911](../../../app/src-tauri/src/engine/job.rs), and its only caller now supplies the bucket of the route it chose (`single_download` at [:2733](../../../app/src-tauri/src/engine/job.rs), passed at [:2737](../../../app/src-tauri/src/engine/job.rs)). `a_network_cap_paces_the_single_stream_too` runs it against the fixture's `?norange=1` and asserts the transfer respects the cap, so the bypass this question described is gone rather than pending.
* **Throughput Variance**: **Measured 2026-11-03 — no measurable cost.** Adding a third bucket to the `charge` slice is one more balance update and one more deadline computation per read slice, on every one of up to 32 streams. Bracketed at 14–15 ns per `read_slice`+`charge` round in debug codegen, and a capped stream charges 20 times a second by the slice rule, so one more bucket is about 1.1 ms of CPU per stream-hour. **Re-measured in release codegen, which is what ships: the marginal is ~0.5 ns per round — 0.036 ms per stream-hour, and in that profile a fourth bucket lands inside the three-bucket spread, so the cost is indistinguishable from zero.** The debug figure inflated it by ~30×. Over real sockets — 32 streams, 4 MiB, the same 2 MiB/s cap applied through two buckets then through three — every capped arm measured 2.04–2.05 MiB/s in both profiles, including the arm where the cap sits on the *network* bucket, so the ceiling holds identically whichever scope carries it; the uncapped arms swing 180–470 MiB/s on loopback and cannot price anything. The acceptable-regression ruling the plan said it needed turns out to be moot; it is still the user's to make. Numbers, method and limits: [the sequence's Measurements section](per-network-limits-implementation-sequence.md).
* **Size Margin**: Measured, and still the binding constraint. The gate is `size < 4.3 * 1024 * 1024` ([verify-m0.mjs:31](../../../app/scripts/verify-m0.mjs)) = 4,508,876 B; the exe at planning time was 4,488,192 B, so headroom was **20,684 B**. After steps 0-2 the exe was **4,497,920 B with 10,956 B left** (step 0 3,072 B, step 1 0 B, step 2 8,704 B), and after **step 3 it is 4,499,456 B with 9,420 B left — step 3 cost 1,536 B**, the smallest delta of the feature so far. **Step 3b then went the other way: it landed at 4,498,944 B and
returned 512 B**, because the allowance did not arrive as a third map — it merged with the cap into one
`NetScope` per adapter, which removed a `RwLock<BTreeMap<…>>`, its poison handling and one lookup per claim, and
added no `rusqlite` read path. **Step 4 then cost 1,536 B again: the exe is 4,500,480 B with 8,396 B left.** Its
first cut cost 5,120 B, and 3,584 B of that was one `ids.sort_unstable()` over a `Vec<(i64, bool, bool)>` — a sort
is a generic instantiation, not a function call — so deleting it and taking the routes in their existing order is
both *more* correct for a per-route signature and 2.3× cheaper. The cost model now has a second clause: **enforcement
wired into an existing abstraction is free; a new generic instantiation is not.** Linker output here is deterministic, so each delta is code, not noise. The prediction this question carried is now falsified in both halves: the real cost was never the `Bucket` per adapter or the two `RwLock<BTreeMap<…>>` fields (step 0 priced the fields plus the whole write path at 3,072 B, step 1's inert third bucket at 0 B), **and** the hot-path enforcement code step 3 added was cheap — the limiter already took a slice, so enforcement was an argument and a lookup, not a subsystem. That leaves the renderer controls (step 6) as the only term the sequence has not exercised, and **8,396 B after four steps does not yet force the parked size-ceiling ruling back**; it does if step 6's markup and handlers cost more than about half of what is left. The known cost model still says a fresh `rusqlite` read path is ~5 KB each, which is why step 3 read through the registry instead of adding a query. *(Scope of that band, measured 2026-11-05: what costs ~5 KB is a **new generic instantiation** — `query_map` over a new row type. A read that reuses an existing `query_row`-over-scalars shape is cheap: the unattributed ledger's read + write + 17th table + v4 migration measured **+3 584 B** whole.)* **Since then: step 6's card controls cost 5,632 B (4,506,112 B / 2,764 B left) and the ramp plateau fix cost 1,024 B in two relinks, so the exe is 4,507,136 B with 1,740 B left.** Read against the sentence above, step 6 did spend more than half of what was then left, which is the condition this question said would force the parked size-ceiling ruling back — it did not have to be raised, because the user had already ruled "It's alright if you have to cross than 5mb"; the gate stayed at 4,508,876 B and the feature stayed inside it. If the margin is exceeded, fall back to Alternative 1 — delete the three dead columns and publish nothing — rather than raising the ceiling.

## Critical User Journeys (CUJs)

### CUJ 1: Throttled Secondary Link
* **Given**: A multi-bonded transfer using Fiber (unlimited) and a USB LTE Dongle (capped at 500 KB/s).
* **When**: A block is requested on the dongle route.
* **Then**: The engine charges the dongle bucket, the job bucket, and the global bucket; the longest wait (dongle) is applied, preserving Fiber throughput.
* **Shipped (step 3), and measured selectively.** `a_network_cap_slows_its_own_adapter_and_not_the_other` bonds two loopback aliases, caps one, and compares the mean finished-attempt duration of each adapter's blocks across two rotated release passes: **927 ms and 925 ms on the capped adapter against 22 ms on the one beside it**. The three-scope intersection itself is proven in `limit.rs` by `three_scopes_intersect_instead_of_adding_up`.
* **And step 4 added the ramp row.** The goal this journey serves is that the *uncapped* route keeps climbing while the capped one is paced, and that now holds by measurement rather than by accident: `a_capped_adapter_does_not_silence_the_route_beside_it` caps one adapter and leaves the other free, and the Auto band still takes its 8→9 step, with every recorded sample satisfying `0 < host_bytes ≤ bytes_delta`; `a_capped_bond_offers_the_ramp_no_host_evidence` caps both and shows **no `ramp-grow` row at all** — the width stays 8 and the whole file still lands.

### CUJ 2: Quota Exhaustion & Ejection
* **Given**: A cellular network with a 1 GB monthly allowance, currently at 990 MB used, bonded alongside an uncapped fibre route.
* **When**: An attempt on the cellular route finishes and `add_net_usage` pushes the cache past 1 GB.
* **Then**: The cellular route is marked over-quota in memory. The next block assignment skips it, so blocks go to fibre instead; a block already in flight on cellular finishes normally. The route stays visible on the adapter card, showing the allowance as spent rather than silently vanishing. The monthly counter is what is checked — a `data_limit` of 1 GB with `data_limit_period` = `month` means 1 GB per calendar month, not 1 GB total.
* **Shipped by step 3b, with the wording corrected against what the code actually does.** `open_route`
  ([job.rs:293-312](../../../app/src-tauri/src/engine/job.rs)) re-derives the route index at each claim and skips
  adapters whose scope reports `used >= allowance`; a block already claimed finishes on the device it started on,
  because only the *next* claim is re-decided — the route set itself is never re-read, which is the promise
  `RunContext.routes` makes. Proof: `an_adapter_out_of_bytes_carries_no_block_while_its_bond_finishes` (a shut
  adapter carries zero bytes of a 2 MiB bond while the open one carries all of it) and
  `a_ceiling_raised_while_the_job_runs_gives_the_adapter_its_blocks_back` (the wait and the re-admission, over
  real sockets). Two things the paragraph above predicted are **not** how it works: nothing is ever "marked
  over-quota in memory" — eligibility is derived from the two counters on every claim, so there is no flag to
  clear and no way for a raised ceiling to be missed; and the route "staying visible on the card showing the
  allowance as spent" is **built** — step 6 shipped it on 2026-11-04, and the launched run read exactly that: the
  card's allowance chip stayed on the route, went `is-shut`, and printed "136 MB / 89.0 MB", then "208 MB / 89.0 MB",
  while the transfer's own byte count did not move for ~10 s. The monthly window is checked, not the total, and a window that
  rolls re-opens the adapter by itself because `net_usage` is keyed by local date
  (`every_route_being_out_of_bytes_waits_and_a_raised_allowance_re_admits_it`).

### CUJ 3: Cap Edited While a Job Runs
* **Given**: A download is running at full speed on a route whose adapter has no cap.
* **When**: The user sets that adapter's rate cap to 500 KB/s and saves.
* **Then**: `network:set-limits` writes the row and calls `Bucket::set_cap` on the bucket that job already holds, so the running transfer slows on its own without a restart. **Shipped (steps 1-3).** The write and the live-bucket application came in with steps 1-2; step 3 is what made the second half of that sentence true, and it did so by reading the registry **per block attempt** rather than once per job — because `set_cap` only reaches an adapter that already has a bucket, while a first-ever cap *creates* the entry, and a slice frozen at job start would never look at it. `a_cap_saved_after_the_job_began_reaches_the_next_attempt` covers that seam. The card shows the new value because the renderer refetches `network:list` after its own write ([main.ts:5282-5287](../../../app/src/renderer/main.ts)) — that same call refreshes the registry; there is no "other open window" to notify in a one-window app. What still stops a user from living this journey is the missing control (step 6), not the engine.

## Technical Plan (Architecture)

### 1. Registry of Network-Scope Buckets
`AppState` ([lib.rs:17-34](../../../app/src-tauri/src/lib.rs)) holds exactly one bucket today: `global_limit: Arc<engine::limit::Bucket>` at [lib.rs:23](../../../app/src-tauri/src/lib.rs), built in `run()` at [lib.rs:304](../../../app/src-tauri/src/lib.rs). Two fields are added:

* `network_buckets: Arc<RwLock<BTreeMap<i64, Arc<Bucket>>>>` — one rate-cap bucket per network id.
* `usage_cache: Arc<RwLock<BTreeMap<i64, u64>>>` — bytes used this month per network id, mirroring `net_usage`.

  *Superseded by step 3b as built*: the two fields became **one** `network_scopes: NetworkScopes` registry of
  `NetScope { bucket, allowance, used, window }` per adapter id
  ([job.rs:144-249](../../../app/src-tauri/src/engine/job.rs)). Every later mention of `network_buckets`,
  `usage_cache`, or "a per-route over-quota flag" in this document is the plan's spelling; the shipped shape has
  one entry per adapter, holds both figures, and holds **no flag** — eligibility is derived at each claim
  (ruling 4). The merge is also why step 3b returned 512 B instead of costing any.

Both start empty. `RwLock`, not `Mutex`: many worker threads read a bucket list while one thread rebuilds it. The rows do not exist at `run()` time — `networks` is populated by the background adapter watcher `watch_network_changes` ([lib.rs:238](../../../app/src-tauri/src/lib.rs)), which starts after `run()` returns, so a query in `run()` would read an empty table. Populate and rebuild both maps inside that watcher, on every pass where the adapter set or a row's properties change.

**Corrected while step 2 shipped.** The paragraph this section replaced said the watcher "emits
`network:changed` only when the adapter **count** changes" and that a property-change emit was needed "or an edit
made in one window never reaches another." Both halves were wrong. The emit fires whenever the watcher's cached
fingerprint differs from the previous pass, so a property change did emit; what was missing is that the
fingerprint *contained* no properties — only the identity and the local IP — so a renamed adapter never reached the branch at all. The
fix was to widen the fingerprint, not to add an emit. And there are no "other windows": `tauri.conf.json` declares
one window (`main`) and no `WebviewWindowBuilder` exists in the Rust, so a write handler emitting to a second
renderer that cannot exist buys nothing — the acting renderer already refetches after its own write
([main.ts:5282-5287](../../../app/src/renderer/main.ts) — the `.setEnabled` call then `refreshNetworks()`; the listener
at [:5302](../../../app/src/renderer/main.ts) is the *other* path, and it only fires on the watcher's changed pass). The `network:changed` emits from the IPC write handlers
planned here are therefore **not built**, recorded as a latent need if a second window ever ships rather than as a
fix.

A cap edit does not need a rebuild to take effect. `Bucket::set_cap(&self, …)` ([limit.rs:52](../../../app/src-tauri/src/engine/limit.rs)) mutates through a shared reference, so `network:set-limits` can call it on the `Arc<Bucket>` every running job already holds. The registry is a source of identity, not a per-block lookup.

### 2. Multi-Bucket Charging (Engine Layer)
The limiter was already arity-independent: `read_slice(buckets: &[&Bucket])` ([limit.rs:92](../../../app/src-tauri/src/engine/limit.rs)) and `charge(buckets: &[&Bucket], n, cancel)` ([limit.rs:104](../../../app/src-tauri/src/engine/limit.rs)) take a slice and sleep the longest single wait, so caps intersect instead of adding up — proven for two scopes by `scopes_intersect_instead_of_adding_up` ([limit.rs:176](../../../app/src-tauri/src/engine/limit.rs)) and, since step 3, for three by `three_scopes_intersect_instead_of_adding_up` ([:193](../../../app/src-tauri/src/engine/limit.rs)). There are **two** charge sites, and **step 3 gave both the network bucket** — shipped and measured 2026-11-04, on both transfer paths.

* `fetch_block` ([job.rs:2519](../../../app/src-tauri/src/engine/job.rs), parameter at [:2532](../../../app/src-tauri/src/engine/job.rs)), `caps` built at [:2602-2609](../../../app/src-tauri/src/engine/job.rs), charging via `read_slice(caps)` at [:2621](../../../app/src-tauri/src/engine/job.rs) and `charge(caps, to_write, cancel)` at [:2637](../../../app/src-tauri/src/engine/job.rs). Both worker sites now bind a bucket before the call — [job.rs:2182](../../../app/src-tauri/src/engine/job.rs) (primary pass) and [:2387](../../../app/src-tauri/src/engine/job.rs) (tail-split pass) — and hand it over as `net.as_deref()` at [:2196](../../../app/src-tauri/src/engine/job.rs) and [:2401](../../../app/src-tauri/src/engine/job.rs). The literal `None`s step 1 left at those arguments are gone.
* `single_attempt` ([job.rs:2834](../../../app/src-tauri/src/engine/job.rs), parameter at [:2837](../../../app/src-tauri/src/engine/job.rs)), `caps` at [:2889-2898](../../../app/src-tauri/src/engine/job.rs), charging at [:2905](../../../app/src-tauri/src/engine/job.rs) and [:2911](../../../app/src-tauri/src/engine/job.rs) — the one-stream fallback for servers that will not honour `Range`, used by `single_download` ([job.rs:2690](../../../app/src-tauri/src/engine/job.rs)), which binds `live_cap(&ctx.networks, route.network_id)` at [:2733](../../../app/src-tauri/src/engine/job.rs) inside its retry loop and passes it at [:2737](../../../app/src-tauri/src/engine/job.rs), on the route it chose at [:2703-2704](../../../app/src-tauri/src/engine/job.rs).

The route identity this section said was missing is now `route_nets`. `parallel_download` ([job.rs:1076](../../../app/src-tauri/src/engine/job.rs)) builds a `Vec<Option<i64>>` from `routes` at [:1105](../../../app/src-tauri/src/engine/job.rs) — one network id per route, frozen with the routes — and carries it in `WorkerEnv` beside the registry (struct at [job.rs:2060](../../../app/src-tauri/src/engine/job.rs), `networks` at [:2074](../../../app/src-tauri/src/engine/job.rs), `route_nets` at [:2077](../../../app/src-tauri/src/engine/job.rs), assigned at [:1283-1284](../../../app/src-tauri/src/engine/job.rs)). The worker destructures both at [:2121-2124](../../../app/src-tauri/src/engine/job.rs) and, per block attempt, resolves the live bucket through `live_cap` ([job.rs:145](../../../app/src-tauri/src/engine/job.rs)) over the `NetworkBuckets` map ([job.rs:135](../../../app/src-tauri/src/engine/job.rs)), which is `RunContext.networks` ([job.rs:166](../../../app/src-tauri/src/engine/job.rs)) — the same map the application state holds.

**One deliberate deviation from the plan in this section.** It proposed building `net: Vec<Option<Arc<Bucket>>>` once at job start and never touching the registry again. What shipped freezes the **ids** and looks up the **bucket** per attempt. A frozen slice of buckets would honour a cap that *changes*, because `set_network_cap` mutates the `Arc` in place, but it would not honour an adapter receiving its **first** cap: that adds a map entry a frozen `Vec` never consults, and CUJ 3 promises that saving a cap slows the transfer already running. The price is one shared-lock read per block attempt, taken only where the route carries a network id — `live_cap` returns before locking on `None` — over a map written only by a save or the watcher's changed pass. Step 3's whole byte cost measured at 1,536 B ([the sequence's step-3 measurements](per-network-limits-implementation-sequence.md)); the lock itself was not separately priced, so its cost rests on that total and on the paced-throughput tests rather than on a microbenchmark. `single_download` needed no `Vec` at all: it has one route and reads `route.network_id` directly.

### 3. Quota Gate
A rate cap cannot express an allowance. `Bucket::new` filters `cap.filter(|c| *c > 0)` ([limit.rs:36](../../../app/src-tauri/src/engine/limit.rs)), so a cap of 0 becomes `None` — unlimited. The intuitive "quota exhausted, set the cap to 0" opens the cable instead of closing it, so the allowance is a separate gate, not a bucket value.

* ~~`usage_cache` holds bytes used this calendar month per network id~~ — **superseded by step 3b as built**: the
  cap and the allowance now live in one `NetScope` per adapter id
  ([job.rs:144-249](../../../app/src-tauri/src/engine/job.rs)) in one `NetworkScopes` registry, and the scope
  carries the day/week/month mirror as well. The two maps this section described would have disagreed in exactly
  the case that matters — an adapter whose limit arrived while a job was transferring — so they were merged.
  `used` advances where the bytes are booked (`finish_attempt`, [job.rs:1025](../../../app/src-tauri/src/engine/job.rs))
  and is replaced by the ledger's own sum when a pass reads SQLite
  (`note_ledger`, called from `set_network_usage`); `used == 0` for a scope with no allowance, because
  `note_carried` only counts when an allowance exists to compare against. An adapter with neither figure has no
  entry at all.
  **The invariant, as built:** the counter is *never* trusted as a bill — it may lag or double-count between the two
  writers, and the double-count direction is the accepted one because it ejects a route slightly early, which is safe
  on a metered link and healed by the next ledger pass. No `max()` merge rule was added, and no lock is held across
  the database: the ids are collected under the map's read guard, the guard is dropped, then the query runs.
* The read loop consults the cache before each block claim. It must not query SQLite there: the engine talks to
  the database through one `Mutex<Connection>`, so a per-block read would put every stream behind one lock. This
  is what the mirror is for, and it is why the merged type is nearly free — enforcement sits on an abstraction the
  rate cap already had.
* Overshoot is bounded by construction, not eliminated. `add_net_usage` is called once per finished attempt
  ([job.rs:1025](../../../app/src-tauri/src/engine/job.rs)), blocks are 1–8 MiB, and up to 32 streams can be in
  flight, so the worst case is roughly the current width times the block size past the allowance before anyone
  notices. The gate reads the mirror before assigning a block, so the exposure is the blocks already claimed, not
  the rest of the file. **One path is worse than that paragraph said and the code says so**: a non-range job is
  one attempt for the whole file, so it has no block boundary to be ejected at — an allowance saved while a
  single-stream download is in flight is honoured between attempts, not during one.
* Ejection is where this design and the existing invariant disagree; see Open Questions. **Ruled and built
  2026-11-04** (choice 1 in the sequence document): eject at the block-claim boundary. A block already in flight
  finishes on its device, only a *new* claim re-derives eligibility, the route set stays frozen, and the label
  drift is admitted on screen by step 6 — which shipped it: the shut route keeps its card and its allowance chip
  reads "Over allowance / 136 MB / 89.0 MB" while the transfer's own bytes stand still.

### 4. Ramp Isolation — **built 2026-11-04 (step 4)**
`RampSample.capped` used to be set once, job-wide, as `ctx.limit.cap().is_some() || ctx.global.cap().is_some()`, and a capped sample made the Auto ramp hold its width instead of growing. ORing a third scope into that expression would have meant any per-network cap froze the 8-to-32 band for the whole job — including on the uncapped adapter. Steps 0-3 therefore shipped *without* consulting `ctx.networks` at all, which left the ramp reading a per-adapter-paced window as **uncapped** host evidence: the band could climb toward 32 on the limiter's arithmetic while the link was already at its ceiling.

What shipped is not an enum of scopes. Naming a scope answers nothing — an adapter cap that argues for `Hold` freezes the band the design forbids freezing, and one that argues for "fair" repeats the original bug. The question the ramp can act on is **how much of the window the host set**, so the boolean became a quantity:

* `RampSample.host_bytes` ([job.rs:2041](../../../app/src-tauri/src/engine/job.rs), struct at [:2028](../../../app/src-tauri/src/engine/job.rs)) is `bytes_delta` minus the bytes that left on a route whose adapter had a cap in force during the window; a job or app-wide cap still zeroes it, exactly as the boolean did, so no old behaviour changed. `next_width` ([job.rs:2154](../../../app/src-tauri/src/engine/job.rs)) both **prices** the growth bar from it ([:2224](../../../app/src-tauri/src/engine/job.rs)) and **judges** the window against it ([:2189-2197](../../../app/src-tauri/src/engine/job.rs)) — the two halves move together or the rule merely gets stricter.
* **The bar is a rate, not a total** (same fix, 2026-11-04). `RampSample.span_ms` ([:2034](../../../app/src-tauri/src/engine/job.rs)) says how long the window actually measured and `RampState.gain_bar_span_ms` ([:2075](../../../app/src-tauri/src/engine/job.rs)) says how long the window that *priced* the bar measured, and the judgement cross-multiplies the two (paid `host_bytes × bar_span` against asked `bar × span`). On the equal spans a healthy job produces this is the arithmetic the totals were; on an unfair pair it is the only comparison that means anything. The step that found this is step 4's launched run: the first tick of a loaded transfer arrived **2,907-6,432 ms after it was sent** (`Msg::Tick` now carries its send time, [:1537](../../../app/src-tauri/src/engine/job.rs) and [:1726](../../../app/src-tauri/src/engine/job.rs)) and swallowed seconds of the transfer, while the window that answered the bill was the next one — and under the old ordering the bar branch ran before the window-age gate, so a 22-90 ms sliver could latch `plateau` for the rest of the job. `next_width` now gates on **two clocks** (cadence since the last action, and this window's own age, [:2176](../../../app/src-tauri/src/engine/job.rs)) before it judges anything, and the coordinator keeps a window accumulating across a "too young" hold instead of closing it — the only hold that does not close a window is the one that says nothing about the window.
* The subtraction is fed by `BlockCoord::limited_done` ([job.rs:1116](../../../app/src-tauri/src/engine/job.rs)), added at the same two `report` closures that book `total_done` ([:2518-2520](../../../app/src-tauri/src/engine/job.rs), [:2737-2739](../../../app/src-tauri/src/engine/job.rs)) and summed at the window boundary ([:1740-1741](../../../app/src-tauri/src/engine/job.rs)). **A cap in force credits that route's bytes in full**, not only where the bucket actually made a stream wait — the same conservative trade the job-wide cap already made, since a cap is the user saying "do not go faster here" and widening against it buys refusals, not bytes. Measuring real stall time would need the limiter to report slept time; not built, and `a_capped_bond_offers_the_ramp_no_host_evidence` uses deliberately generous caps to prove the trade is not an artefact of a slow link.
* `RampSample.shifted` ([:2044](../../../app/src-tauri/src/engine/job.rs)) carries what no subtraction can: an adapter that met its allowance carries *nothing*, so it contributes no limiter bytes, yet the window it produced is smaller than the one the bar was priced from. Answering that bar with the survivors' bytes plateaus the job against a bond that no longer exists. `bond_state` ([:327](../../../app/src-tauri/src/engine/job.rs)) snapshots each adapter the job can leave bytes on with `(capped, spent)` at both ends of the window, and a difference voids the bar and holds — voided, never a plateau. It returns an empty list when no route carries an adapter id, so on this machine's single-adapter pool every window reads as one steady bond: the same inertness ruling 2 declares for a saved cap.
* **A refusal is re-derivable from the history table.** `ramp-grow` rows carry `span` and `lag`; `ramp-plateau` rows carry `span`, `lag`, `host`, `bar` and `bar_span`. The first pilot of the fix stopped at nine streams and nothing in the record said whether that judgement was right, which is why the fields exist — the depth of the climb varies pass to pass on this link, and a plateau is only worth believing if it can be checked. The run that followed showed `host-limited#2` asking 8.52 MB/s and receiving 8.01 MB/s, i.e. a control host that hands out 8 MB/s to the whole connection set and therefore deserves the plateau it got.
* Sampled at window boundaries, so a limit applied **and** lifted inside one window is invisible — the blind spot the 250 ms claim cadence already had, said in the code comment rather than glossed.

**The invariant, as built:** *only bytes that left on a route with no limiter in force may ever raise the stream
width.* A window can therefore never be evidence of host headroom for a route the user has capped, and a bond that
changed mid-window is never evidence about the bond it replaced. Both are pinned — the first by the two socket tests
and the falsified counterfactual, the second by `a_bond_that_shifted_voids_the_bar_instead_of_plateauing`, which also
asserts the identical window *without* the shift still plateaus, so the forgiveness cannot silently become a soft rule.

**A third, added with the plateau fix:** *a window is evidence only about the span it measured.* Nothing is judged
until both clocks have reached `window_ms`, and a bar priced over one span is answered against that span, not
against the answering window's byte count. Pinned by `a_window_22_ms_old_is_not_an_answer_to_the_growth_bill` (the
bar stands, and the band climbs on the next full window) and `a_full_window_at_the_old_rate_is_still_evidence_against_the_growth`
(the gate does not forgive a genuine no: a full window at the old width's rate still plateaus). Both failed against
the shipped rule with `left: Plateau, right: Hold` before the rule moved.

```text
[Execution Loop]
Job Thread -> Select Route (skip adapters at their allowance) -> [Fetch Block]
          -> [Charge: Global + Job + Net] -> Write Bytes
          -> [once per attempt: add_net_usage -> NetScope counter]
          -> [once per ramp window: host bytes + bond signature -> next_width]
```

## Alternatives Considered

### 1. Delete dead columns and publish nothing
* **Pros**: Costs zero bytes, closes no gap but creates none, and removes three fields the UI already publishes but nobody can set.
* **Cons**: The requested feature does not exist; the M2 ledger row stays wrong in the other direction.
* **Status**: Not rejected — this is the **named fallback** if the size spike cannot fit the headroom. Both spikes
  ran on 2026-11-03 and passed: the headroom was 19,660 B rather than the 20,684 B assumed here, the write path
  took 3,072 B of it, and threading the third bucket took 0 B. It stays on the record in case steps 4, 6 and 7
  outgrow the **9,932 B** left after step 3b (the step returned 512 B), and the Verification Plan still treats it as the pass condition for a failed spike.
  See [the sequence's Measurements section](per-network-limits-implementation-sequence.md).

### 2. Widen the metered guard
* **Pros**: Uses existing machinery; exclusion is strictly stronger than a cap.
* **Cons**: Catches only the classes Windows names as metered; does not allow user-configurable limits.
* **Reason for Rejection**: The metered guard is a binary exclusion, not a configurable ceiling. It cannot enforce a 1 GB monthly allowance or a 500 KB/s rate cap.

### 3. Steer blocks away (route weighting)
* **Pros**: Avoids hard ceilings; keeps the bonding pool active.
* **Cons**: Does not stop over-quota usage; a weighted route still charges the bucket.
* **Reason for Rejection**: The problem is a ceiling, not a preference. Route weighting is a separate mechanism that does not prevent quota overshoot.

### 4. Parallel quota counter in memory
* **Pros**: Simple to implement.
* **Cons**: Forks a subsystem that already works, creating two sources of truth for the same number.
* **Reason for Rejection**: Rejected as an *independent* counter — one that accumulates on its own and would have to be reconciled with `net_usage`. What this design does add is a `usage_cache`, which is different in kind: it is seeded from `net_usage` through the existing `network_usage` query, and it is written by the same code that books the bytes ([job.rs:857](../../../app/src-tauri/src/engine/job.rs)), so the table remains the record and the map is only a copy read at block boundaries. The distinction is the whole reason a cache is acceptable here and a counter is not, so it is worth stating rather than leaving the two sections looking like they disagree.

### 5. Populate day/week/month columns in net_usage
* **Pros**: Uses existing schema columns.
* **Cons**: Fills the database with duplicate facts; complicates update logic.
* **Reason for Rejection**: `net_usage` already holds the raw facts in `bytes` with a `period_key`. The `day`/`week`/`month` columns are deliberately written as placeholders ([db.rs:748-750](../../../app/src-tauri/src/db.rs)) and the windows are derived at read time by summing `bytes` over `period_key` bounds computed in SQL ([db.rs:776-778](../../../app/src-tauri/src/db.rs)).

--------------------------------------------------------------------------------

## Non-Functional Requirements

* **Performance & Latency**: The quota gate reads the in-memory counter, not SQLite, because the engine reaches the database through one `Mutex<Connection>` and a per-block read would serialise every stream behind it. The counter is written by the same code path that books the bytes ([job.rs:1056-1062](../../../app/src-tauri/src/engine/job.rs)), once per finished attempt, and overwritten by the ledger on the pass that already asked SQLite. The rate cap adds one element to an existing slice per read; **since step 3 it also adds one shared-lock read of the registry per block attempt**, which is a read lock on a map that only a save or the watcher's changed pass ever writes, and it is skipped entirely when the route has no network id. **Step 4 adds one more read of that same map, once per ramp window rather than per block** — `bond_state` snapshots the bond at each boundary, and returns early without touching the lock when no route carries an adapter id, which is every job this app has run to date. None of those locks was microbenchmarked; what is measured is step 3's and step 4's 1,536 B each and the paced-throughput gap between a capped and an uncapped adapter inside one job ([the sequence document](per-network-limits-implementation-sequence.md)).
* **Rollout**: No feature flag. The feature is dormant by its own data: **as shipped, a `NULL` rate cap means no
  bucket exists at all for that adapter** (stronger than the un-capped bucket this plan first described — see
  Target Files item 1), and a `NULL` allowance is an un-gated route; `speed_limit` / `data_limit` are nullable
  today ([schema.sql:53-54](../../../app/src-tauri/schema.sql)). A flag would add a `settings` key — `SETTING_KEYS`
  is a fixed `[&str; 17]` at [ipc.rs:850](../../../app/src-tauri/src/ipc.rs), so a new key means editing its type
  too — and would gate behaviour the null values already gate. **The gate the plan described — "enforcement stays
  off until the chaos-server benchmark is run" — has been passed, not deferred**: both spikes ran on the chaos
  server, step 3's four tests pace real transfers through it, and enforcement is on. What kept the feature dormant
  in a shipped build was the missing control — no card wrote a cap, so a `NULL` was also what every adapter had —
  and **step 6 removed that on 2026-11-04**: both figures are set, saved, repainted and read back from the adapter
  card, and the launched-exe runs drove nothing else. The feature is therefore dormant only by choice now: an
  adapter with empty fields is unlimited, exactly as before, and there is no migration that fills them in.

## Risks & Mitigations

* **Quota gate overshoot**: A route can exceed its allowance by the blocks already claimed before ejection lands — roughly the stream width times the block size, up to about 256 MB at 32 streams of 8 MiB. **Mitigation**: consult the cache before each assignment and update it where the bytes are booked; show the overshoot on the card instead of implying the number is exact. This feature is not a billing instrument.
* **One path, one enforcement gap**: `fetch_block` and `single_attempt` charge separately ([job.rs:2637](../../../app/src-tauri/src/engine/job.rs), [:2911](../../../app/src-tauri/src/engine/job.rs)). Covering only the first means a server that refuses `Range` is uncapped and uncounted. **Closed by step 3**: both call sites pass the route's bucket, and `a_network_cap_paces_the_single_stream_too` runs the `?norange=1` fixture against a capped registry so the fallback path is proven rather than assumed. The quota gate (Architecture §3) **closed the same way in step 3b** — `open_route` at both block-claim sites and `route_spent` in `single_download`'s attempt loop — with the overshoot this bullet names still true on that path, because a single-stream job has no block boundary to be ejected at.
* **Route ejection mid-block**: A block already in flight on an ejected route completes normally; only new assignments skip it. **Mitigation**: no abort of in-flight work, and the route stays labelled on the deck. **Built by step 3b** exactly as mitigated — `an_adapter_out_of_bytes_carries_no_block_while_its_bond_finishes` shows the skip over real sockets — and the "stays labelled on the deck" half shipped with step 6: the deck keeps naming the route and its allowance chip flips to "Over allowance", which the launched run watched appear in the same second the ledger crossed with no click in between.
* **Wait instead of fail**: Once a route can be ejected, a bond can be entirely out of bytes, and that is a state no download used to be able to reach. **Chosen (ruling 3)**: the claim sleeps the existing `TICK_POLL_MS` 250 ms and re-checks — no new job state, no new activity word, no renderer vocabulary for the rarest case in the feature. The cost is honest and named: while a job waits the ramp sees an empty window and shrinks rather than grows. **Step 4 now handles that window honestly** — `host_bytes` is zero when nothing landed, so the bar is not answered with limiter bytes, and when the ejection itself shifts the bond, `shifted` voids the bar and holds instead of plateauing the job against a bond that no longer exists. Saying so *on screen* shipped in step 6 as the shut allowance chip and its title, "This month is spent: no new blocks leave on this adapter until the window rolls.", which the launched run watched hold across ~10 s of frozen transfer with `status` still `downloading`. The ramp's own shrink is still not named on screen. The plateau defect step 4's launched run found — one growth step, then a latch for the rest of the job — was **fixed on 2026-11-04**: the same harness now measures `streams_final` 10 / 13 / 13 with 2 / 5 / 5 `ramp-grow` rows on an uncapped per-connection-paced host, while the one-clock control still stops at 9 and a capped bond still records no growth row at all. `a_ceiling_raised_while_the_job_runs_gives_the_adapter_its_blocks_back` asserts the wait happened (the run cannot finish before the ceiling moves) as well as the resume.
* **A shut adapter must be able to come back**: Ruling 4, and the answer is to store nothing. There is no over-quota flag anywhere in the engine; eligibility is `used >= allowance` read on each claim, so a raised ceiling or a rolled window (a ledger read of zero) is the whole of re-admission. A latch would have to be cleared by every path that can free an adapter, and the first one forgotten leaves a route shut for the session with a card claiming it has bytes.
* **A latched plateau must *not* come back — the user's ruling, 2026-11-04**: the ramp's own permanence is the mirror image of that rule and was put to the user deliberately, because step 97's launched run ended every pass in a plateau and the climb depth varied (10, 13, 13). The answer was to leave it recorded as **"considered, rejected, reopen on that evidence"** — the user's ruling, written here as theirs and not as `DECIDED`, which stays their marker. `plateau` is set in one place ([job.rs:2190](../../../app/src-tauri/src/engine/job.rs)) and cleared nowhere for the life of the job. The reasoning, kept because it is the reason the code is shaped this way: a plateau is a refutation of the last growth by a window that was old enough and full enough to answer it, which is a fact about the host, not a transient state; every *circumstantial* refusal already has its own door — a bond that changes voids the bar through `shifted` ([job.rs:2157](../../../app/src-tauri/src/engine/job.rs)) instead of plateauing, and a quiet or zero-byte window takes the Hold branch ([job.rs:2166](../../../app/src-tauri/src/engine/job.rs)) without being charged. An un-latch on a timer would make the ramp re-probe after every dip, and probes are paid for in the user's transfer bytes — the same sawtooth the evidence rule exists to prevent — against a cost that is bounded anyway: at most `(32 − plateau width)` streams, for one job, since `RampState` is rebuilt per job ([job.rs:2242](../../../app/src-tauri/src/engine/job.rs)). **Reopen only on this evidence**: a launched run whose `ramp-plateau` rows (`span`, `lag`, `host`, `bar`, `bar_span`) show a long job sitting several streams below what the same host demonstrably kept paying afterwards — and price the rule in bytes first, since headroom is 1,740 B.
* **Memory growth with many adapters**: ~~`network_buckets` and `usage_cache` hold one entry per network row~~ — as built since step 3b it is **one** `network_scopes` entry per network row, which is strictly fewer objects than the plan's two maps, and ghost rows from unplugged adapters are still never purged. **Measured narrower after step 2**: the registry is keyed off the live enumeration inside `network_payload`, so an adapter that is not present gets no entry — ghost rows cost database bytes, not registry entries, and the registry's size is bounded by the adapters Windows reports. Step 3b tightened the same rule: an entry with neither a cap nor an allowance is removed on the spot (`load_bearing`), so an adapter whose limits were taken away leaves nothing behind. The remaining truth is the mitigation: a row purged later must not delete its `net_usage` history — it drops the scope, not the bytes. Ghost rows are a pre-existing defect.
* **The gate proves less than it looks like it does**: under `--smoke`, `setup` returns early ([lib.rs:325-327](../../../app/src-tauri/src/lib.rs)), so the adapter watcher that populates the registry never starts. A green `npm run verify` after this feature proves the channel and the payload, not the enforcement. **Mitigation**: engine tests that construct `RunContext` directly, plus one manual capped run measured in the launched exe. The first half of that mitigation is now carrying the feature: step 3's four tests run against the chaos server through `RunContext`, and no `verify` check touches enforcement. The second half is **done**: the manual capped run was measured in the launched exe on 2026-11-04 over CDP (`D:\RE\_cap-harness`, 11 passes, shipped 4,500,480 B binary), where a saved 2,000,000 B/s cap paced the transfer at a median **2,015,033 B/s** against **9,063,729 B/s** for the same job uncapped on the same host, and every one of the run's 297 block attempts carried `network_id 1`. That also retires this item's other premise: the run showed a saved cap is **live on a single-adapter machine**, because `attribute_egress` credits the unbonded default route to the adapter the routing table leaves on. **Two more clauses of this bullet were then falsified by step 6 and are retracted** (2026-11-04, same harness, two fresh reports at `D:\RE\_cap-harness`): the allowance is *not* inert here either — an 89 MB allowance typed into the card popover froze a running transfer on this single attributed adapter at the block-claim boundary (`served` frozen 10.1 s, `status` still `downloading`) and restarted it 988 ms after the popover cleared it — and a cap is no longer "unreachable by a click", because the card writes both. The inertness ruling 2 describes survives only as a rule for a machine whose routes really do carry no adapter id; on such a machine this card would still accept a number that does nothing, and that warning remains unshipped. Enforcement is now **proven in the shipped binary and reachable by a click, on both halves**. **Its precision is
now measured too (2026-11-05), and the number to quote is ±1 %, not the 2,015,033 B/s median by itself**: on the
fixture-written byte counter over a span starting at t ≥ 5 s, a saved 2,000,000 B/s cap paced at
**−0.40 %** and a saved 4,000,000 B/s cap at **−0.72 %** (3 rotated legs each, exe 4,569,600) — while the floor of
any counter readable over CDP is ~5 %, because `verified_bytes` books whole 8,388,608-byte blocks and the fixture
runs ~1 block ahead of it. [cap-precision.md](../../../design/cap-precision.md) has the predicates and the four
items still `unmeasured`.


--------------------------------------------------------------------------------

## Detailed Implementation Plan

### Target Files to Create/Modify

#### 1. `src-tauri/src/lib.rs` (Modify)
* **Type / Scope**: Application State & Lifecycle
* **Proposed Changes**: Add two fields to `AppState` (struct at [lib.rs:17](../../../app/src-tauri/src/lib.rs), existing bucket at [:23](../../../app/src-tauri/src/lib.rs)): `network_buckets: Arc<RwLock<BTreeMap<i64, Arc<Bucket>>>>` and `usage_cache: Arc<RwLock<BTreeMap<i64, u64>>>` (bytes this calendar month, per network id). Initialise both empty where `global_limit` is built ([lib.rs:304](../../../app/src-tauri/src/lib.rs)). **Done in step 2, with three changes from this plan:**
  * Population is folded into `ipc::network_payload(conn, adapters, state)` ([ipc.rs:1388](../../../app/src-tauri/src/ipc.rs)), which the watcher's changed-branch and `network_rows` ([ipc.rs:1370](../../../app/src-tauri/src/ipc.rs)) both call — so `network:list` refreshes the registry too, instead of a saved cap waiting for the watcher's next changed pass. Reading the caps and usage here was free: the pass already held the row and had already called `network_usage`.
  * The resolve returns `Option<Arc<Bucket>>` ([lib.rs:45](../../../app/src-tauri/src/lib.rs)) rather than "creating an un-capped one if absent", as planned. An un-capped bucket is a `Mutex` taken on every block read of every stream for an adapter that has no cap; entry exists **only** for an adapter with a saved cap, and `set_network_cap` removes the entry when the cap is cleared or non-positive ([lib.rs:63](../../../app/src-tauri/src/lib.rs)). Step 3 maps onto this directly, and reads the map per block attempt rather than caching it, for the reason in Architecture §2.
  * `set_network_cap` takes `Option<i64>`, not `Option<u64>`: the column and the IPC argument are signed, and a negative cast straight to `u64` wraps into a colossal ceiling instead of meaning "no cap". The fold-to-`None` lives in that method, with the test covering zero and `-1`.
  * The `network:changed` emit is not added here — see the correction in Architecture §1. Lock order is database mutex, then the map guards; nothing may take the database lock while holding a map guard, or this pass and a worker booking its bytes deadlock.
* **Deliberately not touched**: `src-tauri/src/engine/limit.rs` — its **behaviour**. It imports only `std` and two engine siblings ([limit.rs:10-15](../../../app/src-tauri/src/engine/limit.rs)) and no Tauri types; `charge` already takes a slice, so a third bucket is a caller change. Putting an `AppHandle` lookup in the limiter would push a Tauri dependency into the engine layer for no benefit. Step 3 did add one thing here, and only a test: `three_scopes_intersect_instead_of_adding_up` ([limit.rs:193](../../../app/src-tauri/src/engine/limit.rs)), the proof that three caps still intersect rather than add. No limiter code changed.

#### 2. `src-tauri/src/engine/job.rs` (Modify)
* **Type / Scope**: Job Supervisor & Worker Loops
* **Proposed Changes**: The rate-cap half of this is **shipped by step 3**; the quota, skip and attribution
  bullets below are not. What step 3 wrote:

  ```rust
  // parallel_download — ids travel with the routes, buckets are looked up live
  let route_nets: Vec<Option<i64>> = routes.iter().map(|r| r.network_id).collect();

  // each worker, per block attempt, at both call sites
  let net = live_cap(&networks, route_nets.get(route).copied().flatten());
  ```

  with `NetworkBuckets` = `Arc<RwLock<BTreeMap<i64, Arc<Bucket>>>>` at [job.rs:135](../../../app/src-tauri/src/engine/job.rs),
  `live_cap` at [:145](../../../app/src-tauri/src/engine/job.rs), `RunContext.networks` at [:166](../../../app/src-tauri/src/engine/job.rs),
  the `route_nets` build at [:1105](../../../app/src-tauri/src/engine/job.rs), the `WorkerEnv` fields at
  [:2074](../../../app/src-tauri/src/engine/job.rs) and [:2077](../../../app/src-tauri/src/engine/job.rs), the two worker
  binds at [:2182](../../../app/src-tauri/src/engine/job.rs) and [:2387](../../../app/src-tauri/src/engine/job.rs), and
  `single_download`'s equivalent at [:2733](../../../app/src-tauri/src/engine/job.rs).
  The slice each charge site builds is unchanged from step 1:

  ```rust
  let mut caps = [limit, global, limit];
  let caps: &[&Bucket] = match net {
      Some(net) => { caps[2] = net; &caps }
      None => &caps[..2],
  };
  ```

  ([job.rs:2602-2609](../../../app/src-tauri/src/engine/job.rs), mirrored at
  [:2889-2898](../../../app/src-tauri/src/engine/job.rs)) — `read_slice(caps)` at [:2621](../../../app/src-tauri/src/engine/job.rs)
  and `charge(caps, to_write, cancel)` at [:2637](../../../app/src-tauri/src/engine/job.rs); in `single_attempt`
  [:2905](../../../app/src-tauri/src/engine/job.rs) and [:2911](../../../app/src-tauri/src/engine/job.rs). The third
  slot is pre-filled with `limit` so the slice is only ever two long when `net` is `None`. `fetch_block` keeps the
  `#[allow(clippy::too_many_arguments)]` it already needed at [:2518](../../../app/src-tauri/src/engine/job.rs); `single_attempt`
  still takes no new parameter, so it runs with its 9 arguments and no attribute. Step 1 had measured this shape at
  2.05 MiB/s with the network bucket capped — the same throttle as a job-bucket cap at 2.04 — and step 3 confirmed it
  on real bonded jobs, so the intersection held and **what step 3 delivered was selectivity, not mechanism.**

  What is left, unchanged from the plan:
  * **Not the shape this section proposed.** It asked for `net: Vec<Option<Arc<Bucket>>>` resolved at job start, so
    the hot path would only index a `Vec`. Shipped instead: the frozen `Vec` carries `Option<i64>` and `live_cap`
    reads the registry per attempt. Reason in Architecture §2 — a frozen bucket list cannot see an adapter's
    *first* cap, and CUJ 3 promises it can.
  * Over-quota skip: **built in step 3b, and not the way this section said.** It proposed a per-route
    `AtomicBool` set by the writer that updates `usage_cache`; the registry already owned that fact, so
    `NetScope::spent()` ([job.rs:237](../../../app/src-tauri/src/engine/job.rs)) behind `route_spent`
    ([:275](../../../app/src-tauri/src/engine/job.rs)) and `open_route` ([:293](../../../app/src-tauri/src/engine/job.rs))
    replaced the flag. The filter sits at the **block-claim** boundaries — `open_route` called from the worker loop
    at [job.rs:2406](../../../app/src-tauri/src/engine/job.rs) and from the straggler/hedge loop at
    [:2634](../../../app/src-tauri/src/engine/job.rs), and `route_spent` inside `single_download`'s attempt loop at
    [:3020](../../../app/src-tauri/src/engine/job.rs) — not in `BondPolicy::route_for` ([:69](../../../app/src-tauri/src/engine/job.rs))
    or `secondary_route_for` ([:81](../../../app/src-tauri/src/engine/job.rs)), which stay
    `(worker, attempt, count) -> usize`, cannot see a network id, and were never the seam.
  * Ramp attribution: **built in step 4, and not as "which scope".** `capped` is gone; the sample carries
    `host_bytes` and `shifted` and `next_width` prices the bar from the host's share — see Architecture §4, which
    now describes the shipped fields rather than the planned boolean.
  * Cache write: **built in step 3b, with two writers instead of the one this section imagined.** `note_carried`
    ([job.rs:229](../../../app/src-tauri/src/engine/job.rs)) runs beside the existing `add_net_usage` call
    ([:1056-1062](../../../app/src-tauri/src/engine/job.rs)) on every finished attempt — exact and mid-job — and
    `note_ledger` ([:219](../../../app/src-tauri/src/engine/job.rs)) **overwrites** the same counter with the sum
    for the window the allowance names, taken by the watcher pass and `network_payload` — the authoritative figure,
    and the one that heals any double count between the two. Both live in the one `NetScope` per adapter, not in a
    second map. Keep `db.rs` free of app state — still true, and still how the ~5 KB read-path cost was avoided.

#### 3. `src-tauri/src/db.rs` (Modify)
* **Type / Scope**: Database Layer
* **Proposed Changes**: Add `update_network_limits(conn, id, speed_limit, data_limit, data_limit_period)` writing the three columns at [schema.sql:53-55](../../../app/src-tauri/schema.sql) — **shipped in step 1**; it returns whether a row actually changed so the IPC layer can distinguish "no such network" from a write. No new read query is needed: `network_usage` ([db.rs:764](../../../app/src-tauri/src/db.rs)) already returns the calendar-month sum as the third tuple element, and step 2 proved the alternative expensive — a first shape for the registry added two fresh `rusqlite` read paths (`prepare` + `query_map` + `collect`) and cost **10,752 B** of release binary for them, which is ~5 KB per read path. Reusing the pass that already held both answers brought step 2 from 17,408 B to 8,704 B. **How to apply to steps 3-7: read through a query that already runs, or pay 5 KB.** *(Read "pay 5 KB" as "pay for a new generic instantiation": the 10,752 B was two `query_map`-over-a-new-row-type paths. `db.rs`'s later `unattributed_usage` is a second `query_row` over four `i64` scalars with the same closure shape, and the whole ledger — read, write, table, migration — cost +3 584 B.)* `add_net_usage` ([db.rs:743](../../../app/src-tauri/src/db.rs)) is unchanged — the in-memory cache belongs to the caller, not the storage layer.

#### 4. `src-tauri/src/ipc.rs` (Modify)
* **Type / Scope**: IPC Contract & Dispatch
* **Proposed Changes**: Add the channel **`network:set-limits`** — kebab-case, matching `network:list` and `network:set-enabled` in the Rust `CHANNELS` list ([ipc.rs:31-32](../../../app/src-tauri/src/ipc.rs)) — and a dispatch arm beside [ipc.rs:191](../../../app/src-tauri/src/ipc.rs). Args `[id, speed_limit, data_limit, data_limit_period]`. **Shipped in step 1** (dispatch at [ipc.rs:203](../../../app/src-tauri/src/ipc.rs), handler at [:1567](../../../app/src-tauri/src/ipc.rs)). Reuse `byte_count_arg` ([ipc.rs:1041](../../../app/src-tauri/src/ipc.rs)) for the rate cap — this plan named it `speed_limit_arg`, which does not exist; `byte_count_arg` is the shared normaliser (`(n > 0).then_some(n)`) the per-download and global cap channels already use, and it is what gives 0 and negatives as `None` = unlimited. Validate the period against the string literal `"month"` or `null` and reject `"day"` / `"week"` with an error: the DB `CHECK (data_limit_period IN ('day','week','month'))` at [schema.sql:55](../../../app/src-tauri/schema.sql) accepts all three, so the monthly-only boundary in Non-Goals can only be enforced here — `data_limit_period_arg` does that, and requires a period only when an allowance is set. After a successful write, apply the cap to the live bucket through `state.set_network_cap` ([lib.rs:63](../../../app/src-tauri/src/lib.rs)); this is required, not a nicety, because the registry otherwise fills only on the watcher's changed pass or a `network:list`. Do **not** emit `network:changed` from the write handlers, as this plan said: the app declares one window and the acting renderer already refetches. `network_rows` already publishes `speedLimit`, `dataLimit` and `dataLimitPeriod` ([ipc.rs:1417-1419](../../../app/src-tauri/src/ipc.rs)) — no payload change.

#### 5. `src/shared/ipc.ts` (Modify)
* **Type / Scope**: TypeScript IPC Types
* **Proposed Changes**: Add `'network:set-limits': { args: [id: number, speedLimit: number | null, dataLimit: number | null, period: 'month' | null]; result: { ok: true } }` to `IpcContract` beside `network:set-enabled` ([ipc.ts:75](../../../app/src/shared/ipc.ts)) and the matching string to the `CHANNELS` array ([ipc.ts:155-157](../../../app/src/shared/ipc.ts)). Both lists must change together or the `channelsMatch` check ([verify-m0.mjs:44](../../../app/scripts/verify-m0.mjs)) fails. **Shipped in step 0/1 as planned** — the contract entry is at [ipc.ts:81](../../../app/src/shared/ipc.ts) and the channel string at [:157](../../../app/src/shared/ipc.ts).

#### 6. `src/shared/models.ts` — no change
* **Type / Scope**: Shared UI Data Models
* **Proposed Changes**: None. `Network` already declares `speedLimit`, `dataLimit` and `dataLimitPeriod` ([models.ts:73-75](../../../app/src/shared/models.ts)); the columns were published and typed long before anything could write them. Recording this so the plan does not "add" fields that exist.

#### 7. `src/renderer/main.ts` (Modify)
* **Type / Scope**: Renderer UI
* **Proposed Changes**: `netCard` ([main.ts:4793](../../../app/src/renderer/main.ts)) already receives the `Network` object with the three values. Add two controls to the card: a rate-cap number field and a data-allowance field, labelled "rate cap" and "data allowance", not "speed limit" / "data limit". Persist on an explicit Save or blur, not on every keystroke, calling `network:set-limits`. Show used-of-allowance from the existing `usedMonth` field so the number shown and the number enforced are the same quantity. The card must say plainly that an exhausted allowance stops the route from taking new blocks rather than promising a hard cut.
* **As shipped, step 6 (2026-11-04)**: the two controls are **chips**, one per limit, that open a single popover
  (`#net-limits-popover`) anchored to the chip that was clicked, with a value field and a unit select each —
  MB/s · KB/s for the cap, GB · MB for the allowance — one Save button, and a note line. **Explicit Save only, no
  blur-save**, which is a deviation from "Save or blur" with a reason: the deck repaints on a timer and would
  detach an inline editor mid-edit, and `db::update_network_limits` writes all three columns in one UPDATE, so a
  blur of one field would erase the other unless the popover resends it — it does, and both launched-exe reports
  record the untouched field (`keptOther`) to prove that rather than assume it. Labels follow the ask exactly
  ("Cap", "Allowance", never "speed limit"/"data limit"). The used-of-allowance is the enforced quantity: the chip
  prints `network:list`'s own `usedMonth` over `dataLimit`, so an overshoot is *shown* rather than hidden — the
  launched run read "136 MB / 89.0 MB" and then "208 MB / 89.0 MB" while the transfer was frozen, which is the
  Risk item above's mitigation arriving in the UI. The shut state says what it does and not more: "This month is
  spent: no new blocks leave on this adapter until the window rolls.", set states say "Blocks leaving on this
  adapter are paced to this rate." **A zero or negative number is not a stop**: measured, both `0` and `-1` typed
  into the cap field write NULL and repaint "Cap / none", consistent with `byte_count_arg`; a user who wants to
  shut an adapter empties the box and clears the allowance instead. Freshness is carried by a 5 s read while a
  transfer is active (`maybeRefreshNetworks`, gated on the deck or the modal being visible; the Save path refreshes
  immediately), which is what let the shut chip appear in the same second the ledger crossed with no click in
  between — and the bound that sets: on a met allowance the screen can lag the gate by up to that window.

#### 8. `scripts/verify-m0.mjs` (Modify)
* **Type / Scope**: Build/Verification Gate
* **Proposed Changes**: The channel count is produced in Rust (`channelCount: CHANNELS.len()` at [ipc.rs:334](../../../app/src-tauri/src/ipc.rs)) and asserted as 32 at [verify-m0.mjs:45](../../../app/scripts/verify-m0.mjs); bump the expectation to 33 — **done in step 0**, along with `channelsMatch` and the renderer bridge method (`network: ['list','setEnabled','setLimits']` at [verify-m0.mjs:85](../../../app/scripts/verify-m0.mjs)). Step 0 also had to change `json!({ "channels": CHANNELS })` to `CHANNELS.as_slice()`: serde only implements `Serialize` for arrays up to 32 elements, so the list was sitting exactly on that wall. **Not done, and no longer planned:** a separate "size check against the headroom". The gate stays the single fixed `exe under 4.3 MB` check; per-step headroom is recorded in [the sequence document](per-network-limits-implementation-sequence.md) instead, because a second number in the harness would only restate the same ceiling. The set-then-read-back round trip is covered by Rust tests (`set_limits_writes_the_three_columns_and_clears_them`, plus step 2's registry tests) rather than a new smoke assertion. Note the structural blind spot: under `--smoke`, `setup` returns early ([lib.rs:325-327](../../../app/src-tauri/src/lib.rs)) so the adapter watcher never runs — but since step 2 the registry is **also** filled by `network:list`, so the harness now populates it; what a green `npm run verify` still does not prove is enforcement, which step 3 did put in a worker — but only where a cap exists, and the harness never creates one, so enforcement remains provable only by the engine tests and by a cap set through the bridge.
**Step 6 changed the last clause and not the blind spot.** The harness does create a cap now — check 48,
`a card cap chip saves through network:set-limits and clears again`, clicks the real chip, types 2 MB into the real
popover, presses the real Save, re-reads the row *after* the repaint and asserts `speedLimit = 2_097_152`, then
clears it — so the write path, the delegation and the chip's own repaint are guarded by a gate. What it still does
not do is transfer a byte against that cap, so enforcement remains provable only by the engine tests and by the
launched-exe runs described in the E2E item below. The count went 47 → 48 checks, still with `exe under 4.3 MB` as
the single size gate.

### Verification Plan
* **Compilation & Build**: `cargo build` then `npm run build`, then measure the release exe. The headroom planned
  here was **20,684 B** (4,508,876 B ceiling minus 4,488,192 B measured); steps 0 and 1 re-measured it as
  **19,660 B** — the baseline rebuild came out 2,048 B smaller, step 0 cost 3,072 B and step 1 cost nothing — and
  step 2 cost **8,704 B**, putting the exe at **4,497,920 B with 10,956 B left**, and step 3's enforcement cost
  **1,536 B** — the cheapest step of the feature and the one this plan expected to be the most expensive. Step 3b
  went further the other way and **returned 512 B**, so the exe is **4,498,944 B with 9,932 B left**: the
  allowance merged into the cap's existing registry rather than arriving as a third map, and it added no new
  read query. Step 2's first shape cost
  17,408 B and was cut in half by removing two new read queries; see Target Files item 3. `check('exe under 4.3 MB', …)`
  at [verify-m0.mjs:31](../../../app/scripts/verify-m0.mjs) is the gate. If it fails, fall back to Alternative 1.
  `npm run build` needs `CARGO_HOME=H:/tools/cargo` and `RUSTUP_HOME=H:/tools/rustup` on this machine; the
  `cargo` shim on `PATH` exports neither in the form `build-app.mjs` accepts.
* **Unit Tests**: `cargo test` — as the existing gate runs it, with `-- --test-threads=1` — **144 `#[test]` after step
  3, of which 143 compile on this Windows target (one is `#[cfg(unix)]` in `paths.rs`): 137 pass, 6 ignored**. Step 3
  also ran them under `--release`, because a timing assertion in debug codegen asserts a different thing — same 137/6
  there. `scopes_intersect_instead_of_adding_up` ([limit.rs:176](../../../app/src-tauri/src/engine/limit.rs)) stayed green untouched — it is the proof that adding a slice element intersects rather than adds. Of the cases this plan called for, **shipped by step 3**: the three-bucket intersection (`three_scopes_intersect_instead_of_adding_up`, [limit.rs:193](../../../app/src-tauri/src/engine/limit.rs)) and the non-range `single_attempt` cap (`a_network_cap_paces_the_single_stream_too`); **already covered elsewhere**: a `cap` of 0 means unlimited, not stopped, in `cap_swap_applies_to_the_next_read` ([limit.rs:221](../../../app/src-tauri/src/engine/limit.rs)) and in `set_network_cap`'s test, so no duplicate was added. **Still owed**: ~~a test that a route marked over-quota receives no new block but finishes its in-flight one~~ — **that is CUJ 2 and it shipped with step 3b**: `an_adapter_out_of_bytes_carries_no_block_while_its_bond_finishes` (no new block on the shut adapter, every byte of the bond on the open one, the shut adapter's own figure unmoved) and `a_ceiling_raised_while_the_job_runs_gives_the_adapter_its_blocks_back` (the wait is observable, because nothing is claimable until the ceiling moves, and only the re-admitted adapter carries bytes after). Neither needed a flag, because there isn't one. Seven pure gate tests came with them, listed in [the sequence's step 3b measurements](per-network-limits-implementation-sequence.md#step-3b--over-quota-ejection-done-2026-11-04), and the engine total went from 137 passing to **146** in both profiles. **Step 4 added six more** — four pure (`a_partially_paced_window_is_judged_on_what_the_host_paid`, `a_paced_window_cannot_climb_the_ceiling_either`, `a_bond_that_shifted_voids_the_bar_instead_of_plateauing`, `the_bond_signature_lists_only_the_adapters_this_job_can_leave_on`) and two over sockets (`a_capped_bond_offers_the_ramp_no_host_evidence`, `a_capped_adapter_does_not_silence_the_route_beside_it`) — taking the total to **152 passed, 6 ignored**. **The plateau fix then added two more pure cases** —
`a_window_22_ms_old_is_not_an_answer_to_the_growth_bill` and
`a_full_window_at_the_old_rate_is_still_evidence_against_the_growth` — taking it to **154 passed, 6 ignored** over
161 `#[test]` attributes of which 160 compile on this target, re-run and confirmed in release on 2026-11-04 in
54.4 s. Both new cases were run against the *shipped* rule first and failed with `left: Plateau, right: Hold`. Step 2's registry tests (`Arc::ptr_eq` across a cap raise in [lib.rs](../../../app/src-tauri/src/lib.rs), watcher-pass adoption of saved caps and this month's bytes in [ipc.rs](../../../app/src-tauri/src/ipc.rs)) remain, and step 3 added the enforcement four: a capped route paces its own download, a cap slows one adapter and not the one beside it, the non-range path is paced, and a cap saved after a job began reaches its next attempt.
* **Static Analysis**: `npm run typecheck` for the new channel across `IpcContract` and `CHANNELS`.
* **E2E/Smoke Integration**: `npm run verify` for the 33-channel assertion and the set/read round trip — with the watcher caveat in item 8 in the report. **The manual run against the chaos server is done, 2026-11-04**: `D:\RE\_cap-harness\report-2026-11-03T22-16-44-419Z.json` holds eleven CDP-driven passes in the shipped exe — capped median **2,015,033 B/s** against a 2,000,000 B/s cap and **9,063,729 B/s** uncapped on the identical host, a cap saved mid-run stepping **8,952,856 → 2,434,607 B/s**, a release stepping **2,455,640 → 9,180,140 B/s**, zero unattributed attempts, and Σ`verified_bytes` = Σ`block_attempts.bytes` = `net_usage.bytes` = **2,144,518,002**. `npm run verify` still exercises none of it: `--smoke` returns early from `setup` ([lib.rs:325-327](../../../app/src-tauri/src/lib.rs)) so the watcher that populates the registry never starts, which is why the enforcement evidence is a launched-exe run and not a gate. **Step 6 added two more launched runs,
both driven only through the card** (`network.setLimits` is never called by either harness):
`report-step6-2026-11-03T23-12-13-829Z.json` — a cap typed into the popover with the browser's own
`Input.insertText`, four rotated 24 s passes alternating capped and free for **median 2,260,210 B/s** capped against
**9,116,362 B/s** uncapped on the same host (ratio **0.2479**), then read back after an `app.quit` and a relaunch on
the same data dir with the field prefilled `2` / `1048576`; and `report-step6b-2026-11-03T23-17-35-407Z.json` — a
fresh data dir, an 89 MB allowance saved while the job ran at ~9 MB/s with 9 connections, the ledger crossing
`93,323,264` at t = 6,836 ms with the shut chip in the same sample, **69,206,016 B** of already-claimed blocks
finishing, then `served` frozen at **218,103,809** for 11 samples (~10.1 s) with `status` never leaving
`downloading`, and `open` back to 9 **988 ms** after both fields were cleared through the popover. Full numbers in
[the sequence's step 6 measurements](per-network-limits-implementation-sequence.md#step-6--card-controls-done-2026-11-04).
**The `2,260,210 B/s` (+7.8 %) in that clause is not an enforcement figure and its recorded discrepancy is
retracted (2026-11-05).** Re-measured with both denominators separated, spans swept, and a scaling leg, on the
current exe: the cap holds to **±1 %** on the fixture-written counter over a span starting at t ≥ 5 s
(**−0.40 %** at a saved 2,000,000 B/s, **−0.72 %** at 4,000,000 B/s, 3 rotated legs each), the `+7.8 %` belongs to
a 24 s window that starts inside the run-ahead, step 3's counterpart was **+0.75 %** (not −3.9 %), and no counter
exposed over CDP resolves the 4.86 % MiB-vs-10⁶ difference the card's own unit makes — so this harness's floor is
about **5 %**. Predicates, the six-leg identity tying the two counters together, and the four things still
`unmeasured`: [cap-precision.md](../../../design/cap-precision.md).

--------------------------------------------------------------------------------

*Drafted with EGM*
