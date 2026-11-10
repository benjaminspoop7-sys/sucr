# Stage 9 — Implementation Readiness (reconstructed)
> **Provenance, stated plainly.** egm-review requires one report file per stage. During the review these
> findings were only pasted into chat and were never written to disk, which the skill forbids. This file is
> a reconstruction written after the review concluded: each reviewer's own report text is copied verbatim
> out of the session transcript
> (`b3e35f46-b23d-4ce7-a3ea-456fdf83e848.jsonl` in the Qoder project directory). Nothing below is paraphrased
> or added by the orchestrating agent, except this note and the disposition lines at the end.

---

<!-- reviewer: Agent "Stage 9 impl-readiness reviewer" finished | transcript source: note_46512_6.txt -->

### Review Findings

1. **Blocker | Affected Scope: Detailed Implementation Plan Item 1 (`src-tauri/src/engine/limit.rs`)**
   - **Finding & Guidance**: The plan proposes adding `network_buckets: Arc<Mutex<BTreeMap<i64, Arc<Bucket>>>>` and helper methods to `AppState` within `src-tauri/src/engine/limit.rs` (seen in design doc lines 123–125). However, `AppState` is defined and instantiated in `src-tauri/src/lib.rs` (seen at `src-tauri/src/lib.rs:219`), not `limit.rs`. Placing state management fields in `limit.rs` violates architectural separation. The plan must explicitly mandate modifying `src-tauri/src/lib.rs` for `AppState` updates and define `limit.rs` strictly for rate limiting algorithms.

2. **Blocker | Affected Scope: Detailed Implementation Plan Item 3 (`src-tauri/src/db.rs`) & Architecture Section 3**
   - **Finding & Guidance**: The plan instructs implementing `quota_available(network_id)` against an in-memory `net_usage` cache (seen in design doc lines 130–132). However, it omits the data structure design, thread-safety primitives (e.g. `Mutex` / `RwLock`), lifecycle management, and invalidation rules for this cache. Because `net_usage` is persisted in SQLite (seen at `schema.sql:144`), leaving cache synchronization undefined forces the implementer to make unguided concurrency and storage synchronization choices.

3. **Major | Affected Scope: Detailed Implementation Plan Item 4 (`src-tauri/src/ipc.rs`) & `scripts/verify-m0.mjs`**
   - **Finding & Guidance**: The plan proposes adding `network:set-limits` (seen in design doc lines 133–137). However, existing IPC methods use camelCase (e.g., `network:setEnabled`, `download:setSpeedLimit`, seen at `verify-m0.mjs:64, 79`), and `verify-m0.mjs` strictly validates exposed method names (seen at `verify-m0.mjs:53–88`). Introducing a hyphenated command name without updating the M0 reflection assertion ledger will break test suites.

4. **Major | Affected Scope: Detailed Implementation Plan Item 5 (`src/renderer/main.ts`)**
   - **Finding & Guidance**: The plan instructs updating adapter card input mappings in `src/renderer/main.ts` (seen in design doc lines 138–140). However, `src/renderer/main.ts` is an entrypoint bootstrap file (seen at `src/renderer/main.ts:1–20`), whereas UI rendering logic for adapter cards is delegated to component templates/modules not referenced in the plan. The affected UI component files must be explicitly identified.

5. **Nit | Affected Scope: Verification Plan (design doc lines 145–151)**
   - **Finding & Guidance**: The plan states that `verify-m0.mjs` will be extended to smoke-test rate and quota enforcement (seen in design doc line 150), but does not specify test assertions, mock interfaces, or expected payloads.

---

### Counter-Argument Analysis

* **Against the Document's Framing**: The author might argue that abstracting cache implementation details and file placements allows implementing engineers the freedom to optimize structures during coding. However, without specifying where state lives (`lib.rs` vs `limit.rs`) and how caches synchronize with SQLite, engineers will produce incompatible implementations, directly violating the Zero-Decision Rule.
* **Against the Recommendations**: Over-specifying internal cache layouts can lead to brittleness if the database layer (`db.rs`) evolves independently. Nonetheless, stating the exact synchronization interface and data ownership model is necessary to prevent architectural drift across multi-threaded Tauri state boundaries.

---

<!-- reviewer: Agent "Stage 9 codebase-details reviewer" finished | transcript source: note_46530_8.txt -->

Finishing the remaining nit from the last point and delivering the full report.

---

### 🟢 Nit 18 (continued) — Missing files from the Target Files list
The list of six files omits, at minimum: `src/shared/ipc.ts` (required by its own contract assertion at `ipc.ts:160-162`), `src-tauri/src/engine/bond.rs` (defines `Route` at `bond.rs:26`, the only place `network_id` lives), and `src-tauri/src/engine/job.rs`'s `live.rs` (defines `UNATTRIBUTED` at `job.rs:24` via `use live::UNATTRIBUTED`). The plan item 2 does list `job.rs`, so that one is covered — but `bond.rs` and `live.rs` are integration points the plan never names.

### 🟢 Nit 19 — `RampSample` is not a per-route concept
**Seen** `job.rs:1718-1732` — `RampSample` has one `capped` bool, one `width`, one `bytes_delta`. It describes the whole job window, not a single route. The plan (line 130) says "Update `RampSample` to remove global `capped` state freeze, allowing per-route caps." There is no per-route sample field to add. Either a per-route sample struct must be introduced (and its aggregation into the existing sample specified), or the claim is describing a mechanism that does not exist.

### 🟢 Nit 20 — "Remaining usage" has no defined source
Goal (line 24) says "Show live used/remaining usage in bytes, sourced from `net_usage`, refreshed when the adapter card is visible." **Seen** `network_payload` at `ipc.rs:1334-1376` already returns `usedToday`/`usedWeek`/`usedMonth`/`usedLifetime`. "Remaining" requires subtracting from `data_limit`, which is per-network and per-period. The plan never defines which period window to use for "remaining" (the Non-Goal at line 30 defers daily/weekly), and the card at `main.ts:4859-4865` renders only Today and Lifetime.

---

## Counter-argument to the document's framing

The document frames this as closing a small gap: three dead columns, one missing IPC channel, one missing bucket scope. The counter-argument is that the scope is larger than the plan admits, because the existing engine already has two working rate-limit scopes and a working `net_usage` attribution system. The plan's own "Alternatives Considered" section (lines 76-101) shows the author understands the machinery well — yet the Detailed Implementation Plan then re-derives the quota gate from scratch as an "in-memory cache" (line 65) rather than reusing the `network_usage` aggregation that `db.rs:741-762` already computes and that the alternatives section explicitly cites as the right pattern. That is a design choice the plan makes without saying so, and it contradicts its own stated Non-Goal of avoiding a "parallel quota counter" (line 93-96).

## Counter-argument to my own recommendations

On Blocker 3 (period aggregation): the plan may intend that the quota gate only ever needs the monthly window in the first version, and that the daily rows are simply summed at job start into one cache entry keyed by month. That would be consistent with the Non-Goal at line 30. But if that is the intent, the plan should say "sum all `net_usage` rows for the current calendar month" — it does not, and it instead says "checks an in-memory cache of `net_usage`" (line 65), which reads as a per-row cache. So the recommendation stands: the mechanism is unspecified, not necessarily wrong.

On Blocker 5 (no route selector): the plan could mean that ejection is implemented by removing the adapter from the `routes` slice in `parallel_download` (`job.rs:1051`) before the worker loop, rather than by modifying `BondPolicy`. That is a valid and smaller change. But the plan says "Integrate into the route selector" (line 133) and "skipped by the route selector" (line 51), naming a component that does not exist. The recommendation is to name the actual integration point — either `parallel_download`'s route slice or a new field on `BondPolicy`.

---

## Summary

**Blockers (5):** non-existent `download_to_file`/`verify_remote_hash` functions; registry field assigned to the wrong file; quota period aggregation entirely undefined; no phasing/checkpoints (fails the standard); quota gate contradicts its own architecture and targets a non-existent route selector.

**Majors (7):** internal period contradiction; IPC channel missing from three-part contract edit and breaks the M0 channel-count gate; renderer uses camelCase while plan proposes snake_case; `network:changed` payload insufficient for the stated rebuild; no DB write function for limits exists; quota cache concurrency unspecified; `BucketRegistry` does not exist and no test cases are given.

**Nits (14):** anchor drift on 5 cited lines; invented type name; `lib::run()` naming; `data_allowance` vs `data_limit` inconsistency; benchmark figure sourced from a code comment; missing target files; `RampSample` is not per-route; "remaining usage" undefined.

**Key files:** `D:\RE\Sucr\app\docs\designs\per-network-limits.md` (target), `D:\RE\Sucr\app/src-tauri/src/lib.rs`, `D:\RE\Sucr\app/src-tauri/src/engine/limit.rs`, `D:\RE\Sucr\app/src-tauri/src/engine/job.rs`, `D:\RE\Sucr\app/src-tauri/src/engine/bond.rs`, `D:\RE\Sucr\app/src-tauri/src/ipc.rs`, `D:\RE\Sucr\app/src-tauri/src/db.rs`, `D:\RE\Sucr\app/src/shared/ipc.ts`, `D:\RE\Sucr\app/src/shared/models.ts`, `D:\RE\Sucr\app/src/renderer/main.ts`, `D:\RE\Sucr\app/scripts/verify-m0.mjs`.

---

<!-- reviewer: Agent "Stage 9 style critic review" finished | transcript source: note_46521_7.txt -->

I read the persona file, the standard, the design doc (lines 110–end plus the earlier sections I needed for claim verification), and the linked sources. Findings below.

---

## Findings

### 🔴 Blocker 1 — References two functions that do not exist
**Seen.** The plan says "Update `fetch_block` ([job.rs:2493](…/job.rs)), `download_to_file` ([job.rs:2149](…/job.rs)) and `verify_remote_hash` ([job.rs:2352](…/job.rs))". I read `job.rs`: `fetch_block` starts at line 2482 (its `limit:` param is 2493 — that part is right), but lines 2149 and 2352 are both `match fetch_block(` call sites inside worker closures. There is no function named `download_to_file` or `verify_remote_hash` anywhere in the file (grep returns no matches). A fresh implementer cannot find them. The line numbers also silently redirect attention away from the real call sites, which are inside `single_attempt`-style closures that already close over `ctx.limit`/`ctx.global`.
**Recommendation:** Replace the two bogus references with the actual enclosing functions. I read them as the two `fetch_block` call sites near 2149 and 2352; name them explicitly and give their line numbers.

### 🔴 Blocker 2 — Zero-decision violations in every plan item
**Seen.** The standard requires zero design choices left to the implementer. Each item instead names a goal and stops:
- Item 1 (`limit.rs`): says add the field to `AppState` — but `AppState` is in `lib.rs:16-23`, not `limit.rs`. The helper `get_network_bucket` is unspecified: where it lives, whether it locks the mutex, and what it returns for a missing id (panic? `None`? insert-on-demand?). No signature, no error contract.
- Item 3 (`db.rs`): "Implement `quota_available(network_id)` that checks the in-memory `net_usage` cache" — **there is no in-memory `net_usage` cache in the codebase**. `db.rs:741` (`network_usage`) reads SQLite directly. The cache is a *proposed* artifact, yet the plan item treats it as existing. Worse, "Integrate into the route selector before block assignment" — the only route selection in the engine is `BondPolicy::route_for` (`job.rs:69`), a pure index function. The ejection mechanism (an in-memory "available routes" set consulted at block-assignment time) is entirely undesigned: no data structure, no mutation points, no consistency rule with the bucket registry.
- Item 2 (`job.rs`): "Update `fetch_block` signature to accept a `network_id`" — but `fetch_block` already takes `limit: &Bucket` and `global: &Bucket` (`job.rs:2493-2494`). The plan does not say whether to add a third bucket param, extend the `&[&Bucket]` slice, or look the bucket up at the call site. The call sites (`job.rs:2149`, `2352`) close over `ctx`; no wiring path is described.
- Item 5 (renderer): "Update adapter card input mappings" — I read `netCard` (`main.ts:4793-4877`). The card has **no inputs** except the bond checkbox (`main.ts:4838-4854`). There are no speed/data fields to map. The plan asks the implementer to design the entire control UI from scratch.
**Recommendation:** For each item, add: exact function signatures, the new data structures with their invariants, the lookup/error semantics, and the call-site wiring. Specify the ejection set, who mutates it, and when.

### 🔴 Blocker 3 — The IPC plan breaks the M0 gate and misstates its own cost
**Seen.** `verify-m0.mjs:45` asserts `report.info?.channelCount === 32`, and `ipc.rs:10-43` lists exactly 32 channels. Adding `network:set-limits` makes 33. The plan's Verification Plan does not mention updating that assertion, so the gate fails on first build. Separately, the plan's fallback logic rests on "One new IPC channel costs roughly 11 KB" (`per-network-limits.md:39`, citing `verify-m0.mjs:28-30`). I read `verify-m0.mjs:28-30`: the 11 KB is the cost of a *separate Tauri command* (`sucr_probe`), explicitly because it runs off-thread. Adding an entry to the `CHANNELS` array and the `IpcContract` interface costs ~0 bytes. The cost model is misattributed, so the "drop the IPC rename" fallback is calibrated against the wrong number.
**Recommendation:** State the real incremental cost (a channel array entry + TS contract edit ≈ 0 KB; only a new `#[tauri::command]` costs bytes). Add "update channelCount assertion to 33" to the Verification Plan.

### 🟡 Major 4 — Self-contradictory cache refresh policy
**Seen / inferred.** NFR (`per-network-limits.md:107`): "The cache is refreshed once per job, not per block." Risks (`per-network-limits.md:112`): "the cache is updated synchronously after every `add_net_usage` call." `add_net_usage` is `db.rs:720`, called per finished block at `job.rs:832`. Once-per-job and per-`add_net_usage` are different frequencies; the document endorses both. The risk section's rationale (avoid overshoot) is the stronger argument, but it directly contradicts the NFR, and the implementer cannot resolve it from the text.
**Recommendation:** Pick one. If per-block, drop the "once per job" sentence. If once-per-job, restate the overshoot bound explicitly and reconcile with the 1–8 MB overshoot already accepted.

### 🟡 Major 5 — Incomplete file boundary list
**Seen.** The standard requires an explicit list of exact paths to create/modify/delete. The plan lists 6 files. Missing, with evidence they must change:
- `src-tauri/src/lib.rs` — `AppState` (`lib.rs:16-23`) gains the field; `lib::run()` (`lib.rs:201`) must populate it from `networks`; the registry rebuild on `network:changed` must be wired (the event is emitted at `lib.rs:191`; no handler exists).
- `src-tauri/src/engine/bond.rs` / route construction — `Route` (`bond.rs:26`) carries `network_id`; the quota gate must influence which routes exist.
- `src/shared/ipc.ts` — the contract edit is mandatory for a new channel (the file itself states at `ipc.ts:17-19` that a channel is a three-part edit).
- `src/shared/models.ts` — `Network` (`models.ts:66-87`) already exposes read-only `speedLimit`/`dataLimit`; a setter needs args and a result shape.
- Test files — the plan names a `BucketRegistry` (`per-network-limits.md:149`) that does not exist in the codebase (grep: only `Bucket`, `limit.rs:24`).
**Recommendation:** Extend the Target Files list to every path that must change, and delete the invented `BucketRegistry` name or define it.

### 🟡 Major 6 — Non-Goals contradicts the IPC contract it just specified
**Seen.** Non-Goals (`per-network-limits.md:30`): "The first version supports monthly allowances only; weekly and daily periods are deferred." Item 4 of the plan (`per-network-limits.md:137`) says the handler "validates `data_limit_period` against `day`, `week`, `month` (or null)". `schema.sql:55` also permits all three. The plan defers day/week while simultaneously accepting and validating them.
**Recommendation:** Align: either restrict the validator to `month`/null, or move day/week out of Non-Goals.

### 🟡 Major 7 — Renderer claim describes a UI that does not exist; "displayed units" unspecified
**Seen.** Item 5 says "Update adapter card input mappings and displayed units to `speed_limit` / `data_limit`." I read `netCard` (`main.ts:4793`): it renders name, kind badge, state badge, bond checkbox, Today/Lifetime meta, and a share bar. No speed/data control, no unit formatting for limits. `models.ts:73-75` already projects `speedLimit`/`dataLimit` from the row. The plan gives the implementer no control type, no placement, no unit convention (B/KB/MB, per-second vs total), and no error display.
**Recommendation:** Specify the control (input type, debounce, unit suffix, validation range, error copy) and the exact IPC call it issues.

### 🟡 Major 8 — Headroom number does not match the build artifact
**Seen.** The plan states "The headroom is 20,684 B as of the last build" (`per-network-limits.md:39`). I read the current binary: `sucr.exe` = 4,488,192 bytes; the gate limit is `4.3 * 1024 * 1024` = 4,508,672 bytes (`verify-m0.mjs:31`). Headroom is 20,480 B, not 20,684. The discrepancy is 204 B. The doc presents it as measured, but it is not derivable from the artifact on disk.
**Recommendation:** Recompute from the actual binary and quote the real figure; a go/no-go size gate resting on a wrong number is unsafe.

### 🟢 Nit 9 — AppState cited at the wrong line
**Seen.** Plan section 1: "The `AppState` ([lib.rs:213](…/lib.rs)) currently held a single `Arc<Bucket>`." Line 213 is the `global_limit` construction; the struct is at `lib.rs:16-23`. The claim itself is true (field at `lib.rs:22`), the citation is off by ~190 lines.
**Recommendation:** Cite `lib.rs:22` (or `lib.rs:16-23`).

### 🟢 Nit 10 — "chaos-server benchmark" is not defined as a gate
**Seen / inferred.** The Open Questions (`per-network-limits.md:38`) and Risks (`per-network-limits.md:115`) reference "the chaos-server benchmark" and `test/chaos-server.mjs` exists (I read it: it is a misbehaving HTTP server, no benchmark harness, no 32-stream driver, no 10% jitter criterion). The pass/fail criteria are stated in prose but no script or threshold file is named.
**Recommendation:** Point the gate at a concrete script with the 32-stream/500 KB/s configuration and the 10% jitter rule, or mark the benchmark as not-yet-written.

### 🟢 Nit 11 — Verification Plan references a nonexistent test target
**Seen.** "`cargo test` ... to ensure `BucketRegistry` correctly intersects buckets" (`per-network-limits.md:149`). No `BucketRegistry` exists; the existing intersection test is `scopes_intersect_instead_of_adding_up` at `limit.rs:176`.
**Recommendation:** Name the real test or define the new one.

### 🟢 Nit 12 — "Reconcile the M2 milestone ledger" names no file
**Seen.** Goal (`per-network-limits.md:25`) says reconcile the ledger; no path is given, and I found no ledger file referenced elsewhere in the doc.
**Recommendation:** Give the path, or drop the goal from an implementation plan (it is editorial, not engineering).

---

## Counter-argument to the document's framing

The doc frames this as a small, low-risk addition: "add a third scope" to an existing limiter, reuse `net_usage`, keep the column names. That framing is only defensible if the quota gate, the bucket registry, and the route-ejection mechanism are treated as *existing* machinery. They are not. `charge()` (`limit.rs:104`) intersects buckets cleanly — that part is genuinely reusable and the doc is right to lean on it. But the ejection half has no analogue anywhere in the codebase: there is no "available routes" set, no per-block quota consult, and no handler that removes a route mid-job. Calling that "a third scope" hides the fact that the hard part is a new concurrency domain (a registry read by workers, mutated by a watcher thread, consulted by the route selector) with its own poisoning, rebuild, and consistency rules. The honest framing is: the rate half is a ~10-line change to call sites; the quota half is a new subsystem. The plan's phrasing ("Integrate into the route selector", "the cache is populated ... at job start") makes the subsystem sound already designed.

## Counter-argument to my own recommendations

On Blocker 3 (the 11 KB claim): the doc's cost figure is wrong, but the *conclusion* — that the feature fits in headroom — may still hold. A channel array entry plus a TS contract entry plus a handler match arm is genuinely near-zero bytes; the real costs are the new `Bucket` instances at startup and the `BTreeMap`/`Mutex` in `AppState`. So I should not imply the size risk is large. The finding stands as "the stated rationale is wrong", not "the feature will blow the budget."

On Major 8 (headroom arithmetic): the 204 B gap could be explained by a build that ran after the doc was drafted (the exe is timestamped 11:39, the doc 2026-11-03, same day). I cannot rule that out without a second build. I flag it as a nit-level verification issue rather than a factual error, but the doc should still quote the number it was computed from.

---

<!-- reviewer: Agent "Stage 9 re-audit impl-readiness" finished | transcript source: note_46631_10.txt -->

The implementation plan for the `per-network-limits` feature, as documented in `D:\RE\Sucr\app\docs\designs\per-network-limits.md` (lines 110-180), has been reviewed against the Implementation Readiness Standard and the provided context.

### Findings

🔴 **Blocker: Thread-Safety and Consistency of Quota Cache**
*   **Affected Scope:** `src-tauri/src/db.rs` / `src-tauri/src/engine/bond.rs`
*   **Finding & Guidance:** The plan (line 107) proposes an in-memory cache for `net_usage` updated synchronously after every `add_net_usage` call. It fails to specify the required synchronization primitives (e.g., `Arc<RwLock<...>>` or `AtomicU64`) to ensure thread-safe access from the engine's worker threads while maintaining performance. Relying on "synchronous updates" without detailing the concurrency model leaves the implementer to make critical thread-safety design choices, creating a high risk of data races or deadlocks in the bonding pool's route selector.
*   **Recommendation:** Explicitly specify the concurrency primitive (e.g., `Arc<RwLock<BTreeMap<i64, NetworkUsage>>>`) in the `AppState` section and define the locking strategy for read/write access.

🟡 **Major: IPC Validation Ambiguity**
*   **Affected Scope:** `src-tauri/src/ipc.rs` (line 145)
*   **Finding & Guidance:** The plan mentions validating `data_limit_period` against "month / null" but does not define the expected input type for this IPC command (e.g., string vs. enum). As `ipc.rs` (lines 880-1020) demonstrates existing validation patterns, leaving this to the implementer invites inconsistency.
*   **Recommendation:** Define the exact validation schema for `data_limit_period` in the plan and provide the expected string literals to be used for validation (e.g., strictly `"month"` or `null`).

🟢 **Nit: Registry Rebuild Mechanism**
*   **Affected Scope:** `src-tauri/src/lib.rs` (line 125)
*   **Finding & Guidance:** The instruction to "rebuild the registry when `network:changed` fires" is high-level. While a competent developer can resolve this, the standard requires deterministic instructions.
*   **Recommendation:** Specify the exact mechanism for registry invalidation—e.g., direct invocation from the `watch_network_changes` loop or a dedicated event listener—to eliminate implementation guesswork.

***

### Counter-Arguments

*   **Counter-argument to Document Framing:** The design justifies a synchronous in-memory cache for the quota gate as a performance optimization to avoid SQLite overhead at block loops (line 107). However, if `add_net_usage` (which writes to SQLite) becomes blocked or slow, the quota gate becomes unreliable or, worse, causes engine thread starvation. A purely asynchronous update might be safer, accepting eventual consistency at the cost of transient over-quota allowance, which the plan rejects without sufficient performance benchmarking data.
*   **Counter-argument to Findings:** The requirement for "Zero-Decision Rule" is being applied strictly. Requiring the design document to specify the exact concurrency primitive or IPC validation logic for a mature Rust codebase may be excessive, as a senior engineer would inherently choose an appropriate `RwLock` or `Atomic` type. Imposing these granular decisions in the design document risks creating "analysis paralysis" and reducing the developer's agency to optimize for the actual runtime characteristics observed during development.

---

<!-- reviewer: Agent "Stage 9 re-audit codebase-details" finished | transcript source: note_46640_11.txt -->

The following audit covers the Detailed Implementation Plan (lines 110-end) of `D:\RE\Sucr\app\docs\designs\per-network-limits.md` against the existing codebase and the `Implementation Readiness Standard`.

### Findings

*   🔴 **Blocker**: Plan §5.1 claims to "Rebuild the registry when `network:changed` fires". The current codebase in `D:\RE\Sucr\app/src-tauri/src/lib.rs:191` only emits `network:changed` when the adapter *count* changes. If a network row's properties change (e.g., limits), the registry will not rebuild as claimed, potentially leading to stale bucket configurations.
*   🟡 **Major**: Plan §5.4 defines the ejection logic in `bond.rs`. However, `D:\RE\Sucr\app/src-tauri/src/lib.rs:608` shows that `spawn_job` and the bonding policy resolution are orchestrated in `lib.rs`, and the `BondPolicy` definition exists in `job.rs`. Ejection logic that affects route selection must be coordinated where the bonding policy is applied, which is not exclusively in `bond.rs`.
*   🟡 **Major**: Plan §5.1 states `lib::run()` will populate `network_buckets` at boot time by querying the `networks` table. The `networks` table is populated dynamically by `watch_network_changes` (`lib.rs:168`). There is a race condition where `lib::run()` may attempt to query the `networks` table before the background watcher has populated it, leading to an empty bucket registry at boot.
*   🟢 **Nit**: Plan §5.6 mentions `network:setLimits` (camelCase) for the IPC contract. While the plan cites `ipc.rs` line 32 (`network:set-enabled`) as a reference, the IPC contract in `src/shared/ipc.ts` relies on specific naming conventions. Ensure the TypeScript compiler `IpcContract` and the `CHANNELS` array are updated consistently to avoid build-time assertion failures.

### Counter-Arguments

*   **Against the Document's Framing**: The design assumes that an in-memory cache of `net_usage` is sufficient (Plan §5.3) for quota gates. While this avoids blocking SQLite queries, it relies on the assumption that `add_net_usage` is *always* called immediately following a block, which, if delayed or interrupted, could lead to significant quota overages beyond the documented 8 MB estimate.
*   **Against My Recommendations**: The requirement to fix the `network:changed` emission in `lib.rs` might be overkill if the author intended for the `Registry` to be reconstructed dynamically upon request, rather than purely reactive to the event. The race condition at boot might be mitigated by existing `lib::run` sequentiality, making my "Major" finding a premature concern.

### Verified Resolutions

1.  **Non-existent function references**: Verified `fetch_block` is present at `job.rs:2482`.
2.  **AppState field location**: `AppState` exists in `lib.rs:16`.
3.  **Quota aggregation**: Aggregation is documented via `net_usage` in plan §5.3.
4.  **IPC channel-count gate**: `network:setLimits` is correctly specified in the plan.
5.  **Headroom arithmetic**: 20,480 B is documented in plan line 39.
6.  **Incomplete target-file list**: The plan now includes 10 files (121+).
7.  **Adapter-card UI**: Input controls are specified in plan §5.9.

---

<!-- reviewer: Agent "Stage 9 re-audit style critic" finished | transcript source: note_46622_9.txt -->

This audit evaluates the "Detailed Implementation Plan" in `D:\RE\Sucr\app\docs\designs\per-network-limits.md` (lines 119–168) for implementation readiness and verifies the status of the seven required resolutions.

### Findings

*   **🟢 Nit: Registry Atomicity Uncertainty**
    *   **Scope**: Section 1. Registry of Network-Scope Buckets (`per-network-limits.md:56`)
    *   **Rationale**: The plan specifies rebuilding the registry on `network:changed` events, but it does not define the synchronization mechanism (e.g., locking strategy) required to prevent race conditions when jobs might be actively accessing buckets during a registry rebuild.
    *   **Recommendation**: Specify the locking protocol for the `BTreeMap` replacement to ensure atomicity.

*   **🟢 Nit: Benchmark Environment Ambiguity**
    *   **Scope**: Section 4. Risks & Mitigations (`per-network-limits.md:115`)
    *   **Rationale**: The plan requires passing a "chaos-server benchmark" but lacks a reference to where this benchmark is defined or how the "chaos" conditions are configured. The plan is not fully self-contained as a fresh implementer cannot know how to execute this prerequisite.
    *   **Recommendation**: Cite the specific repository path for the benchmark runner or provide the necessary configuration parameters.

*   **🟢 Nit: In-Flight Block Assignment Logic**
    *   **Scope**: Section 3. Quota Gate (`per-network-limits.md:66`)
    *   **Rationale**: The document states routes exceeding allowance are skipped, but "in-flight blocks on that route complete normally". It is unclear if the `net_usage` cache update includes these in-flight bytes, potentially leading to immediate re-ejection upon completion.
    *   **Recommendation**: Clarify whether the byte-counter update includes completed in-flight blocks immediately.

---

### Verification of Previous Audit Resolutions

| Item | Resolution Requirement | Status | Evidence (File:Line) |
| :--- | :--- | :--- | :--- |
| 1 | `fetch_block` call sites | Verified | `per-network-limits.md:60-61` |
| 2 | AppState field in `lib.rs` | Verified | `per-network-limits.md:125` (`lib.rs:22`) |
| 3 | Monthly `net_usage` aggregation | Verified | `per-network-limits.md:30` |
| 4 | IPC `network:setLimits`, assertion | Verified | `per-network-limits.md:145`, `:161` |
| 5 | Headroom arithmetic (20,480 B) | Verified | `per-network-limits.md:39` |
| 6 | Expanded target-file list (10 files) | Verified | `per-network-limits.md:121-161` |
| 7 | Adapter-card UI inputs | Verified | `per-network-limits.md:157` |

---

### Counter-Arguments

**Counter-argument to the document's framing:**
The plan is highly granular, bordering on excessive for an experienced team. By prescribing the exact line numbers for `fetch_block` updates (`job.rs:2482`, `2149`, `2352`), the document may be masking larger architectural problems. If a feature implementation requires such surgical, file-specific directives to function, it indicates that the underlying engine lacks proper encapsulation, and the focus should be on refactoring the `job.rs` worker closure, not merely implementing this feature.

**Counter-argument to my own recommendations:**
Requiring explicit locking protocols or benchmark details within the design document adds significant maintenance overhead. If the underlying engine's locking mechanism changes, this design document becomes instantly stale and contradicts the implementation. A better approach might be to trust the implementer to follow existing patterns for `AppState` synchronization rather than prescribing them anew in this design.

---

## Disposition (orchestrator, added after the review)
