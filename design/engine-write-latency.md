# Engine write latency — the WAL fsync defect and fix

Date: 2026-11-10. Scope: the Dev engine (`app/src-tauri/src/db.rs`); shipped in
the build after 0.1.0. The comparison harness that exposed this defect is not
part of this record; only Sucr's own numbers are.

## The defect

`db::open()` set WAL journal mode but left `synchronous` at SQLite's default
FULL, so every autocommit fsynced the WAL. A 256 MiB / 32-block job makes
roughly 150 ledger autocommits (attempt INSERT/UPDATE, usage booking, 150 ms
progress flushes, history events). On an SSD each flush is cheap and invisible;
on a spinning disk each costs 30-45 ms. The coordinator loop serialized behind
the platter while the workers, which are free-running, finished the fetches in
about 0.4 s and left the results queued.

Phase timers on a SATA HDD put the coordinator loop at 5.30 s inside a 6.58 s
fetch phase: claim 991 ms + attempt bookkeeping 2 047 ms + progress flush
2 262 ms. A control probe with the same job on an NVMe temp dir showed only
about 334 ms of DB time, which is why the defect had stayed invisible: the
cost is real but drive-conditional.

## The fix

One pragma, in `open()` right after the WAL check:

```rust
conn.pragma_update(None, "synchronous", "NORMAL")
```

WAL's standard pairing. Durability trade, stated plainly: commits survive a
process crash; a power loss can drop the newest commits but never corrupts the
database. A resumed job re-verifies its bytes anyway (the byte-sample check),
so a dropped tail commit costs a re-fetch of at most the unflushed window, not
a wrong file.

Regression assertion in `creates_schema_v4_in_wal` (`db.rs`): `PRAGMA
synchronous` must read 1 (NORMAL), with a message naming this defect.

## Measured effect

Harness: local loopback HTTP fixture, deterministic payload, staging under a
temp dir on the SATA HDD, rotated AB pairs, 6 measured runs per scenario,
medians, every download validated byte-for-byte. Elapsed time covers fetch,
write and the publish-time SHA-256 verify.

| Scenario | Before | After |
|---|---|---|
| 256 MiB, 1 streamed range | 5.668 s | 1.115 s |
| 256 MiB, 8 ranges | 7.647 s | 0.866 s |
| 32 MiB, 4 MiB/s per request | 8.800 s | 8.383 s |
| 32 MiB, 8 paced streams | 2.512 s | 1.374 s |
| 32 MiB, no Range support | 4.779 s | 4.278 s |

The paced legs are server-throttled, so their floor is the fixture's pacing,
not the engine; they confirm the fix did not slow the throttle-bound paths.
Post-fix phase probes show coordinator DB time at about 7 ms (was 5.30 s).

Cost: +2 048 B in the release exe (measured with `stat`, never a hash).
Full suite after the fix: 246 passed, 0 failed, 9 ignored.
