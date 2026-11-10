# Per-Network Speed Limits & Data Quotas

* **Author:** Benjamin (machine account)
* **Status:** Draft
* **Date:** 2026-11-03

--------------------------------------------------------------------------------

## Overview

Give each network adapter its own transfer cap and its own byte allowance, and let the user set them.

## The Problem

Today the app declares three per-network columns (`speed_limit`, `data_limit`, `data_limit_period` at [schema.sql:53](../../../app/src-tauri/schema.sql)) and publishes them to the interface ([ipc.rs:1360](../../../app/src-tauri/src/ipc.rs)), but no code writes them, no control edits them, and the engine never acts on them. The rate limiter enforces only two scopes: one download and one global cap ([limit.rs:104](../../../app/src-tauri/src/engine/limit.rs)). Furthermore, the app's internal milestone record claims M2 already supports this, which is an untruth that this design must rectify.

## Goals

* Enforce a per-network rate cap that the engine honours at charge time.
* Enforce a per-network data allowance using `net_usage` rather than a second counter.
* Expose these controls on the adapter card.
* Show live used/remaining usage in units the engine enforces.
* Reconcile the M2 milestone ledger to reflect that this is new work, not a fix.

## Non-Goals

* Per-site, per-category, or per-queue limits.
* A calendar reset day or timezone database.
* Billing accuracy competing with Windows' own metered accounting.
* Replacing the existing metered-exclusion guard.
* A general scope registry refactor.

## Open Questions

* **Unattributed Bytes**: How should bytes booked against the "default" route (no network ID) be handled? Options: ignore, charge a primary card, or allow only while bonding is off.
* **Throughput Variance**: Does adding a third bucket to the charge slice significantly increase jitter or CPU load at 32 streams?
* **Size Margin**: Does the added IPC write channel and enforcement logic fit within the current ~20 KB headroom?

## Critical User Journeys (CUJs)

### CUJ 1: Throttled Secondary Link
* **Given**: A multi-bonded transfer using Fiber (unlimited) and a USB LTE Dongle (capped at 500 KB/s).
* **When**: A block is requested on the dongle route.
* **Then**: The engine charges the dongle bucket, the job bucket, and the global bucket; the longest wait (dongle) is applied, preserving Fiber throughput.

### CUJ 2: Quota Exhaustion & Ejection
* **Given**: A cellular network with a 1 GB data allowance, currently at 990 MB used.
* **When**: The next block fetch loop begins.
* **Then**: The engine detects the quota is exceeded, marks the route as unavailable, and shifts remaining blocks to other active adapters.

## Technical Plan (Architecture)

### 1. Registry of Network-Scope Buckets
The `AppState` ([lib.rs:213](../../../app/src-tauri/src/lib.rs)) currently holds a single `Arc<Bucket>` for the global cap. We will introduce a `BucketRegistry` that maintains a map of network IDs to `Bucket` instances, ensuring each physical adapter has its own token-bucket state.

### 2. Multi-Bucket Charging (Engine Layer)
The rate limiter in `limit.rs` already supports intersecting multiple buckets via `charge(buckets: &[&Bucket], ...)` ([limit.rs:104](../../../app/src-tauri/src/engine/limit.rs)).
* Update `fetch_block` ([job.rs:2493](../../../app/src-tauri/src/engine/job.rs)) to accept the network bucket.
* Update `download_to_file` ([job.rs:2149](../../../app/src-tauri/src/engine/job.rs)) and `verify_remote_hash` ([job.rs:2352](../../../app/src-tauri/src/engine/job.rs)) to pass the route's bucket down.

### 3. Quota Gate
Since a 0 cap means "unlimited" in `Bucket` ([limit.rs:36](../../../app/src-tauri/src/engine/limit.rs)), allowances need a separate gate.
* The read loop will poll `net_usage` at block boundaries.
* Routes exceeding their `data_allowance` are ejected from the job's bonding pool.

### 4. Ramp Isolation
Update `RampSample.capped` ([job.rs:1499](../../../app/src-tauri/src/engine/job.rs)) so that a throughput ceiling on a single network adapter does not trigger a job-wide freeze of the Auto 8-to-32 stream ramp.

```text
[Execution Loop]
Job Thread -> Select Route -> [Quota Check] -> [Fetch Block] -> [Charge Buckets: Global + Job + Net] -> Write Bytes
```

## Alternatives Considered

### 1. Delete dead columns and publish nothing
* **Pros**: Cost zero bytes; removes the unused columns.
* **Cons**: Fails to deliver the requested feature; leaves the UI/engine gap unclosed.
* **Reason for Rejection**: Rejected as the fallback if the size spike fails to meet the 4.3 MB ceiling.

### 2. Widen the metered guard
* **Pros**: Uses existing machinery; exclusion is strictly stronger than a cap.
* **Cons**: Catches only the classes Windows names as metered; does not allow user-configurable limits.
* **Reason for Rejection**: Rejected by author: separate requirement, does not solve the quota cap ask.

### 3. Steer blocks away (route weighting)
* **Pros**: Avoids hard ceilings; keeps the bonding pool active.
* **Cons**: Does not stop over-quota usage.
* **Reason for Rejection**: Rejected by author: the problem is a ceiling, not a preference; route weighting is a separate mechanism.

### 4. Parallel quota counter in memory
* **Pros**: Simple to implement.
* **Cons**: Forks a subsystem that already works, creating two sources of truth.
* **Reason for Rejection**: Rejected by author: poor architecture; duplicates existing `net_usage` storage.

### 5. Populate day/week/month columns in net_usage
* **Pros**: Uses existing schema columns.
* **Cons**: Fills the database with duplicate facts; complicates update logic.
* **Reason for Rejection**: Rejected by author: `net_usage` already holds the raw facts; the period-summary fields are redundant.

--------------------------------------------------------------------------------

## Non-Functional Requirements

* **Performance & Latency**: The Quota Gate must cache `net_usage` rather than polling the SQLite database at every block loop to avoid stalling the engine thread.
* **Rollout**: A feature flag is required to safely skip enforcement if the data gate is unstable in the field.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

### Target Files to Create/Modify

#### 1. `src-tauri/src/engine/limit.rs` (Modify)
* **Type / Scope**: Engine Rate Limiter
* **Proposed Changes**: Implement `BucketRegistry` structure mapping `network_id` (String) to `Arc<Bucket>`. Add helper `get_bucket(id: &str) -> Arc<Bucket>` for use by the `fetch_block` logic.

#### 2. `src-tauri/src/engine/job.rs` (Modify)
* **Type / Scope**: Job Supervisor
* **Proposed Changes**: Update `fetch_block` signature to accept a `network_id` and pass it to the charging loop. Update `RampSample` to remove global `capped` state freeze, allowing per-route caps.

#### 3. `src-tauri/src/db.rs` (Modify)
* **Type / Scope**: Database Layer
* **Proposed Changes**: Implement `quota_gate(network_id)` that polls `net_usage` for the current period and returns boolean availability. Integrate into the job thread before the byte fetch.

#### 4. `src-tauri/src/ipc.rs` (Modify)
* **Type / Scope**: IPC Contract
* **Proposed Changes**: Rename `speed_limit` to `rate_cap` and `data_limit` to `data_allowance` in all payload models.

#### 5. `src/renderer/main.ts` (Modify)
* **Type / Scope**: Renderer / UI
* **Proposed Changes**: Update adapter card input mappings and displayed units to `rate_cap` / `data_allowance`.

#### 6. `scripts/verify-m0.mjs` (Modify)
* **Type / Scope**: Build/Verification Gate
* **Proposed Changes**: Update any smoke assertions that rely on the exact IPC payload keys for speed/data limits.

### Verification Plan
* **Compilation**: `cargo build` and `npm run build`.
* **Unit Tests**: `cargo test` in `src-tauri/src` to ensure `BucketRegistry` correctly intersects buckets.
* **Static Analysis**: `npm run typecheck` to verify IPC renaming across TS code.
* **E2E/Integration**: Extend `verify-m0.mjs` to smoke-test the new rate/quota enforcement path.
