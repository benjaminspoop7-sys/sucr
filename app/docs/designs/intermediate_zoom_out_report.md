# Stage 8 — Intermediate Zoom Out review (reconstructed)
> **Provenance, stated plainly.** egm-review requires one report file per stage. During the review these
> findings were only pasted into chat and were never written to disk, which the skill forbids. This file is
> a reconstruction written after the review concluded: each reviewer's own report text is copied verbatim
> out of the session transcript
> (`b3e35f46-b23d-4ce7-a3ea-456fdf83e848.jsonl` in the Qoder project directory). Nothing below is paraphrased
> or added by the orchestrating agent, except this note and the disposition lines at the end.

---

<!-- reviewer: Agent "Stage 8 Intermediate Zoom Out review" finished | transcript source: note_46471_5.txt -->

Here is the Stage 8 (Intermediate Zoom Out) review for the design document `D:\RE\Sucr\app\docs\designs\per-network-limits.md`.

### Review Findings

1. **Contradictory Cache Refresh Strategy** 
   - **Severity**: 🟡 Major
   - **Affected Scope**: Technical Plan §3 (Quota Gate) vs. Risks & Mitigations (`per-network-limits.md:63` vs. `113`).
   - **Simplicity Rationale**: The document contains an internal contradiction regarding the lifetime of the `net_usage` cache. Section 3 states the cache is refreshed "once per job (or when the period changes)" (`per-network-limits.md:63`), whereas the Risks & Mitigations section states it is refreshed "after every `add_net_usage` call" (`per-network-limits.md:113`). Refreshing only once per job would result in stale data for long-running downloads that span significant data consumption or calendar periods, allowing quotas to be severely overshot.
   - **Recommendation**: Harmonize the design to explicitly state that the in-memory cache is updated upon every `add_net_usage` write (as correctly noted in the mitigation) rather than once per job.

2. **Unspecified Startup Initialization of Network Buckets**
   - **Severity**: 🟡 Major
   - **Affected Scope**: Technical Plan §1 (`Registry of Network-Scope Buckets`, `per-network-limits.md:56`) & `lib.rs:213`.
   - **Simplicity Rationale**: The design details adding `network_buckets: Arc<Mutex<BTreeMap<i64, Arc<Bucket>>>>` to `AppState` (`lib.rs:213` [seen]) and rebuilding them on `network:changed`, but omits how pre-existing speed limits and allowances stored in the `networks` SQLite table (`schema.sql:53-55` [seen]) are loaded into these buckets during application boot. While global limits are explicitly loaded at startup (`lib.rs:213` [seen]), network-specific limits lack a bootloader step.
   - **Recommendation**: Document how `lib::run()` queries the `networks` table at startup to populate initial `network_buckets` alongside `global_limit`.

3. **IPC Channel Payload and Type Consistency**
   - **Severity**: 🟢 Nit
   - **Affected Scope**: Technical Plan §4 (`ipc.rs`) & `schema.sql:53-55`.
   - **Simplicity Rationale**: The proposal introduces `network:set-limits` accepting `[id, speed_limit, data_limit, data_limit_period]`, but does not specify validation rules for `data_limit_period` (`day`, `week`, `month` check constraint in `schema.sql:55` [seen]).
   - **Recommendation**: Add validation expectations in the IPC handler contract to reject invalid period strings before database execution.

---

### Counter-Arguments & Framing Analysis

* **Counter-Argument to Document Framing**: The design frames per-network limits as a straightforward additive feature leveraging existing token buckets (`limit.rs`) and usage tables (`net_usage`). However, introducing dynamic route ejection based on real-time data quotas transforms the transfer engine from a stateless block dispatcher into a stateful routing controller, introducing potential race conditions between route selection (`bond.rs`) and quota exhaustion.
* **Counter-Argument to Reviewer Recommendations**: Mandating frequent cache invalidation or synchronous database checks to prevent quota overshoots risks reintroducing database contention on SQLite, conflicting with the stated performance goal of avoiding per-block disk reads.

### Verdict
The design is appropriately scoped (monthly-only allowances, clear non-goals) and correctly identifies prior milestone discrepancies (`per-network-limits.md:17` [seen]). However, due to the contradictory cache refresh description and omitted startup bucket hydration, it requires minor structural clarification before proceeding to Stage 9 (Implementation Readiness).

---

## Disposition (orchestrator, added after the review)
