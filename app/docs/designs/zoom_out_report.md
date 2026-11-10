# Stage 5 — Initial Zoom Out review (reconstructed)
> **Provenance, stated plainly.** egm-review requires one report file per stage. During the review these
> findings were only pasted into chat and were never written to disk, which the skill forbids. This file is
> a reconstruction written after the review concluded: each reviewer's own report text is copied verbatim
> out of the session transcript
> (`b3e35f46-b23d-4ce7-a3ea-456fdf83e848.jsonl` in the Qoder project directory). Nothing below is paraphrased
> or added by the orchestrating agent, except this note and the disposition lines at the end.

---

<!-- reviewer: Agent "Stage 5 Zoom Out review" finished | transcript source: note_46312_1.txt -->

# Stage 5 (Initial Zoom Out) Review: Per-Network Speed Limits & Data Quotas

**Document under review:** `D:\RE\Sucr\app\docs\designs\per-network-limits.md`  
**Reviewer Persona:** `egm-zoom-out` (Skeptical Architect / High-Altitude Simplicity & Codebase Reuse)

---

### High-Level Summary & Assessment
The proposed design attempts to complete per-network speed limits and data quotas by adding a runtime `BucketRegistry`, DB polling in block fetch loops, IPC renaming, and metric updates. However, from a high-altitude architectural view, the document suffers from **scoped dishonesty in claims vs codebase reality**, **altitude drop into low-level detail while missing foundational engine primitives**, **YAGNI bloat / premature optimization in IPC renaming**, and **misaligned framing regarding existing milestone assertions**.

---

### Key Findings

#### 🔴 Blocker: False Claims Regarding Codebase State & Unverified IPC Scope
- **Evidence (seen)**: 
  - `per-network-limits.md:15`: Claims columns exist at `schema.sql:53` and are published at `ipc.rs:1360`.
  - `schema.sql:46-57`: `networks` table actually starts at line 46 (`id`, `mac_hint`, `display_name`, `kind`, `color_id`, `enabled`, `speed_limit`, `data_limit`, `data_limit_period`, `custom_name`). Line 53 is `speed_limit INTEGER`.
  - `ipc.rs:1360`: `ipc.rs` is only 1378 lines long. Lines 1340-1372 contain `network_payload`, where `speedLimit`, `dataLimit`, and `dataLimitPeriod` are mapped into JSON at lines 1360-1362.
  - `ipc.rs:31-43` & `scripts/verify-m0.mjs:85-87`: `network:set-enabled` is defined, but **there are NO IPC channels to update `speed_limit`, `data_limit`, or `data_limit_period` for networks!**
- **Simplicity / Scope Rationale**: The document claims "no code writes them, no control edits them". But the design completely omits proposing new IPC channels to *write* or *update* these network limits! It proposes mutating `IPC Contract` by renaming fields (`speed_limit` -> `rate_cap`) in `ipc.rs` (detailed plan §4), yet fails to specify the actual mutation IPC channel (e.g., `network:set-limits`). A feature that user-facing controls cannot update via IPC is non-functional.
- **Recommendation**: Correct line references (`schema.sql:46-57`, `ipc.rs:1340-1372`). Add an explicit goal and technical contract for network limit mutation IPC channels instead of arbitrary property renames.

#### 🔴 Blocker: Flawed Engine Architecture for Quota Gates (Db Polling inside `fetch_block` / Job Thread)
- **Evidence (seen)**:
  - `per-network-limits.md:63, 103-107, 122-124`: Proposes polling `net_usage` at block boundaries inside the read loop / `job.rs:fetch_block` via `db.rs::quota_gate(network_id)`.
  - `lib.rs:285-290` & `job.rs`: `AppState.db` uses a standard `std::sync::Mutex<rusqlite::Connection>`. Blocking disk queries on every block fetch across 32 concurrent HTTP streams will severely lock the main database Mutex and stall throughput.
- **Simplicity / Scope Rationale**: Synchronously querying SQLite inside high-frequency asynchronous engine block loops violates the app's concurrency model (where runtime engine state is held in-memory via `Arc`/atomics in `job::Registry` or `Bucket`).
- **Recommendation**: Avoid database calls in the engine loop. Network byte usage should be tracked or cached in-memory within engine state (or loaded when a job starts / updated on completion), checking limits against in-memory atomic counters.

#### 🟡 Major: Scope Creep & YAGNI Violations (Unnecessary IPC Field Renaming)
- **Evidence (seen)**:
  - `per-network-limits.md:126-136`: Proposes renaming IPC payload keys `speed_limit` to `rate_cap` and `data_limit` to `data_allowance` across Rust (`ipc.rs`), TypeScript (`main.ts`), and smoke tests (`verify-m0.mjs`).
  - `schema.sql:53-54`: Database columns are named `speed_limit` and `data_limit`.
  - `downloads` table (`schema.sql:82`) and `AppState` (`lib.rs:319`, `ipc.rs:232`) both use `speed_limit` / `speedLimit`.
- **Simplicity / Scope Rationale**: Renaming existing, functioning IPC schema fields across TS, Rust, and test suites adds churn and breaks consistency with download/global speed limits (`speed_limit`). It offers zero functional value for per-network enforcement and risks breaking `verify-m0.mjs`.
- **Recommendation**: Reject field renames. Retain existing `speed_limit` and `data_limit` field names throughout DB, IPC, and UI.

#### 🟡 Major: Altitude Drop into Weeds while Missing High-Level Interface Abstractions
- **Evidence (seen)**:
  - `per-network-limits.md:56-60, 114-124`: Spends extensive text specifying exact function signatures (`fetch_block`, `download_to_file`, `verify_remote_hash`) and low-level struct modifications (`BucketRegistry`, `RampSample.capped`).
  - `per-network-limits.md:1-40`: Leaves fundamental architectural questions unstated (e.g., How are period windows—'day', 'week', 'month'—reset or evaluated? How does network dynamic route selection handle mid-transfer ejection without deadlocks?).
- **Simplicity / Scope Rationale**: The document drops into concrete pseudocode and helper signatures while leaving core lifecycle semantics (e.g. how period keys in `net_usage` are matched against `data_limit_period`) completely unstated for a fresh reader.
- **Recommendation**: Remove premature helper method specs. Define high-level data contracts for period tracking and route lifecycle state.

#### 🟢 Nit: Misleading Problem Framing regarding Milestone Ledger
- **Evidence (seen)**:
  - `per-network-limits.md:15, 24`: States "the app's internal milestone record claims M2 already supports this, which is an untruth that this design must rectify."
- **Simplicity / Scope Rationale**: Retrospective documentation cleanup is a non-functional process task, not a software design goal.
- **Recommendation**: Move milestone ledger updates out of architectural goals into an admin footnote.

---

### Counter-Argument Analysis

**Strongest Counter-Argument to Document's Framing:**  
*Why build per-network speed limits & quotas inside the download engine at all?*  
Sucr already has a global speed limit, per-download speed limits, dynamic bonding policy, and a `metered_guard` (`ipc.rs:1437`) that holds cellular connections out of the bonding pool entirely. OS-level network meters (e.g. Windows Metered Connection settings) already notify the app. Adding custom token buckets and quota gates per network adapter adds significant concurrency overhead to high-throughput stream dispatching (32 streams), while 95% of user needs are satisfied simply by enabling `metered_guard` or pausing heavy adapters manually.

**Strongest Counter-Argument to My Own Recommendations:**  
*Why demand in-memory usage tracking instead of simple DB checks?*  
If network usage is cached in memory, multiple app instances or worker threads mutating usage might introduce drift unless synced back to SQLite. Querying SQLite on block boundaries is conceptually simpler and keeps SQLite as the single source of truth, avoiding dual-state synchronization bugs.

---

### Claim Tagging & Evidence Summary

- **Seen (`schema.sql:46-57`)**: `networks` table schema (contains `speed_limit`, `data_limit`, `data_limit_period`).
- **Seen (`ipc.rs:1340-1372`)**: `network_payload` maps DB fields to IPC output.
- **Seen (`ipc.rs:31-43`)**: `CHANNELS` array shows `network:list` and `network:set-enabled` are the only network channels.
- **Seen (`limit.rs:104`)**: `charge(buckets: &[&Bucket], ...)` exists and handles multiple buckets.
- **Seen (`lib.rs:285-290`)**: `AppState` uses single `Mutex<rusqlite::Connection>`.
- **Inferred**: Mid-transfer quota ejection during 32-stream parallel downloads could cause thread contention if gating accesses the main DB Mutex directly.

---

## Disposition (orchestrator, added after the review)
