# Unattributed bytes — the Networks pane's missing half

Record of the slice that closed the proxy adapter gap. Written 2026-11-05 under the
user's instruction "Close the proxy adapter gap first". Everything here is cited to a
file:line, counted, or read out of a launched-binary report; anything that is not is
marked `unmeasured`.

## 1. What was held, and why it was a defect (`writer:` project-sucr-open-defects.md, quoted before this change)

The Networks pane's Downloaded Today / Week / Month / Lifetime figures are sums over
`net_usage`, one row per adapter per local calendar day. A proxied link is deliberately
never attributed — [job.rs:1090](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L1090)
books `add_net_usage` only `if let Some(network_id) = route.network_id`, and under a
resolved proxy the route carries `None` (attribution is skipped upstream at
[job.rs:692](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L692)). So the engine
refused to credit an interface that did not carry the bytes — correct — and those bytes
were then booked **nowhere at all**: dropped. A 32 MiB file crossing a tunnel moved the
pane by zero.

The record's own instruction ruled out the cheap fix, and this change honours it: *do not
fold `n0` into the adapter sums (that charges a byte to an interface that did not carry
it). The real fix is unattributed bytes in the usage payload.*

## 2. The mechanism

| Part | Site | What it does |
| --- | --- | --- |
| Ledger | [schema.sql:154-162](file:///D:/RE/Sucr/app/src-tauri/schema.sql#L154) | `unattributed_usage(period_key PRIMARY KEY, bytes)` — one row per local calendar day, the same day-keyed shape as `net_usage`, no `network_id` column, because a row here belongs to no interface. |
| Version | [db.rs:4](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L4) | `SCHEMA_VERSION = 4`; the `v3 -> v4` arm at [db.rs:61-67](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L61) creates only this table. |
| Write | [db.rs:1049](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L1049) `add_unattributed_usage` | upsert on `date('now','localtime')`; returns without writing when `bytes == 0`. |
| Read | [db.rs:1088](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L1088) `unattributed_usage` | the same four windows (today / week / month / lifetime) that `network_usage` answers, factored into a shared `usage_windows` helper at [db.rs:1105](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L1105) so the two ledgers cannot drift in how a week or a month is bounded. |
| Booking | [job.rs:1099-1106](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L1099) | the `else` of the attribution branch: `add_unattributed_usage(conn, bytes)`. Nothing reaches a live scope here — a cap or an allowance paces a device, and no device carried this. |
| Payload | [ipc.rs:1791-1804](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L1791) | `network:list` answers `{ adapters: [...], unattributed: { today, week, month, lifetime } }`. |
| Surface | [models.ts:97](file:///D:/RE/Sucr/app/src/shared/models.ts#L97) `NetworkList`, [ipc.ts:75](file:///D:/RE/Sucr/app/src/shared/ipc.ts#L75) | the typed shape of that answer. |

### Why a table and not a derivation

`block_attempts.network_id` is already nullable, so the unattributed total could be read
off the attempts. It was rejected: `block_attempts.download_id` is `ON DELETE CASCADE`
and the app opens foreign keys, so deleting a finished download erases its attempts —
while `net_usage` survives. A figure that vanishes when a row is deleted cannot sit beside
one that does not and be called a lifetime total.

### Why two figures in one answer, not a row in the list

Every consumer of `adapters` asks *which device is this* — the cards, the bond count, the
per-network shares, the limiter lookups the M0 gate reads back. An extra element in that
array would be a device that is nobody's. So `network:list` grew a second field rather than
a sentinel `networks` row, a pseudo-row, or a 36th channel (a new channel is the expensive
kind: step 3's `proxy:set-auth` made its step +12,800 B, step 4's `origin:set-auth`
+27,648 B).

## 3. The pane, and the one path that must not add

`renderNetworksView` adds the second ledger **only where adapters were enumerated**
([main.ts:5508-5515](file:///D:/RE/Sucr/app/src/renderer/main.ts#L5508)). When nothing is
enumerated the totals come from the library (`sumLibrary`), and the library already holds
proxied bytes — adding the ledger there would count them twice.

The disclosure sentence moved with the arithmetic
([main.ts:5527-5538](file:///D:/RE/Sucr/app/src/renderer/main.ts#L5527)). Step 1b's words
claimed "their bytes are booked to no adapter, so **the totals above** and the caps on each
card do not count or pace them"; after this change the first half of that is false, so the
proxy sentence now names only what the cards cannot (caps and allowances), and a second
sentence states the amount in the reader's own units:

> ` 33.0 MB lifetime (33.0 MB today) belongs to no adapter; it is inside the totals above and on no card.`

It appears only when `unattributed.lifetime > 0` and only on the adapter-present branches.
The `'A proxy carries new links (…)'` prefix that the M0 gate asserts is unchanged, so the
gate still bites on the same needle.

## 4. Rulings — CHOSEN, not `DECIDED`

Delegated answers under the same rule as Feature 1's four rulings and the proxy slice's
Q1-Q6: this agent chose them and may reverse them. Only the user writes `DECIDED`.

1. **A separate ledger, not a column in `net_usage`.** Rejected alternatives: a sentinel
   `networks` row, a pseudo-row in `adapters`, folding `n0` into the adapter sums, a new
   channel.
2. **Lifetime + today disclosed, week and month carried in the payload but not spoken.**
   The pane's headline is two numbers; a fourth sentence would be read as noise.
3. **The library path never adds the ledger** (see §3), because it already contains those
   bytes.
4. **Zero-byte attempts are not written.** `add_unattributed_usage` returns early on
   `bytes == 0`, so a failed claim cannot leave a row that reads as traffic.

## 5. Cost, and what moved (`writer:` stat + grep predicates, run 2026-11-05)

| Measure | Before | After | Predicate |
| --- | --- | --- | --- |
| `sucr.exe` bytes | 4 569 600 | **4 573 184** | `stat -c '%s' target/release/sucr.exe` — **+3 584 B** (7 × the 512 B granularity), headroom **669 696 B** under the ruled 5 MB ceiling. |
| Schema version | 3 | 4 | `PRAGMA user_version` in the gate's own report (`wal v4 sqlite 3.53.2`). |
| Tables | 16 | 17 | `SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'` → 17, incl. `unattributed_usage`. |
| Rust tests | 201 pass / 6 ignored | **203 pass / 6 ignored** | `cargo test --release -- --test-threads=1`. |
| IPC channels | 35 | 35 | unchanged by design (§2). |
| Setting keys | 20 | 20 | unchanged. |
| M0 `check(` sites | 50 | 50 | unchanged; four of them re-pointed (v3→v4 twice, 16→17 twice). |

This is the "new rusqlite read path" band the proxy record priced at ≈5 KB, measured
3 584 B — the third data point that the band is a *new engine/DB path* band, not a
per-feature band.

New tests, both in [db.rs](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L1282):
`migrates_v3_to_v4_without_touching_adapter_ledgers` (seeds 4 096 B of `net_usage`, drops
the new table, demotes `user_version` to 3, reopens, and requires the adapter ledger to
still read 4 096 and the new ledger to read `(0,0,0,0)`), and
`unattributed_usage_adds_up_its_own_days` (a zero-byte write books nothing; two writes
collapse into one day row; a hand-inserted yesterday row appears in lifetime only;
`net_usage` stays empty). The existing engine test
`a_bond_that_cannot_reach_the_host_finishes_on_the_default_route`
([tests.rs:674](file:///D:/RE/Sucr/app/src-tauri/src/engine/job/tests.rs#L674)) now asserts
both halves at once: the escape books the whole size in `unattributed_usage` **and** zero
rows in `net_usage`.

The migration fixture initially failed against the real arm — it had demoted
`user_version` without dropping the table, which is not a state a v3 file can be in. Fixed
the fixture, not the arm: a file that already holds the ledger is a v4 file whatever its
`user_version` says, so no `IF NOT EXISTS` guard was added for an unreachable state.

## 6. Proof in the launched exe

Two runs of the same CDP harness (`D:\RE\_cap-harness\proxy-run.mjs`), which drives the
shipped binary in its own `--data-dir`, never the user's. Both runs moved 32 MiB
(33 554 432 B) per transfer leg through a local CONNECT proxy.

| Run | Binary | Result |
| --- | --- | --- |
| new | `sucr.exe` 4 573 184 B, mtime 06:30:48Z | **13 legs, `allPass: true`** — `proxy-report-2026-11-05T06-35-41-651Z.json` |
| counterfactual | `sucr-old-4567552.exe` (the build before this change) | **5 of 13 legs failed** — `proxy-report-2026-11-05T06-38-20-689Z.json` |

### What the new build does (`predicate:` each leg's own check, read out of the report)

| Leg | Route the bytes took | `unattributed_usage` lifetime delta |
| --- | --- | --- |
| `direct-default` | adapter `n1` | **0** |
| `system-resolves-env` | proxy, `n0` | **+33 554 432** |
| `manual-typed` | proxy, `n0` | **+33 554 432** |
| `manual-with-bypass` | bypassed → adapter `n1` | **0** |
| `auth-pair-typed-in-card` | proxy with Basic, `n0` | **+33 554 432** |

So the ledger moves exactly when no adapter is credited, and not when one is: the
bypassed leg proves there is no double count, which is the failure mode §3's rule was
written to avoid.

### The pane, in the shipped binary (`evidence-noun:` `#net-stat-lifetime`, `#net-desc` read over CDP)

Standing in the Networks view on the `manual-typed` leg, after two prior proxied legs had
already moved 32 MiB each:

- before the leg: lifetime **`0 B`** — this run's own data dir, nothing attributed yet;
- after the leg: lifetime **`96.0 MB`** = 32 MiB attributed on the direct leg + 64 MiB
  booked to no adapter, and `#net-desc` ends with
  **`64.0 MB lifetime (64.0 MB today) belongs to no adapter; it is inside the totals above and on no card.`**

The proxy sentence keeps its asserted prefix, reworded to what is still true: *"their bytes
belong to no adapter, so the caps and allowances on each card do not count or pace them."*
The false half — *"the totals above … do not count"* — is gone, on the screen and in
`main.ts`.

### What the counterfactual shows, and what it does not

* Genuine: on the old binary the proxied legs report `before` and `after` of
  `{ today: null, week: null, month: null, lifetime: null }` — the payload carried no such
  field at all — and the pane's `#net-desc` still ends with the old sentence while its
  lifetime figure rose only to **`32.0 MB`**, i.e. exactly the one attributed leg. Two 32 MiB
  proxied transfers, 67 108 864 B, were invisible on that screen. That is the gap, measured.
* Not evidence: `direct-default` and `manual-with-bypass` also failed on the old binary, but
  with `got: []` — the harness's re-pointed `netUsage()` asks for `.adapters`, which the bare
  array of the old payload does not have. Those two failures say something about the harness's
  shape, not about the product, and are excluded from the claim.

### Regression on the same build

`auth-run.mjs` (the credential suite) and `verify-m0.mjs` (50 `check(` sites, `M0 GATE: PASS`,
`exe 4.36 MB`) both re-run against 4 573 184 B; results recorded in §8 below.

## 7. Retraction

The claim that travelled — and this record, `proxy-and-http-auth.md` §4.3/§6, the Settings
card, the Networks card and the M0 gate's comment all carried some form of it — was:

> under a proxy the Networks pane's totals omit the proxied bytes; the real fix is unattributed
> bytes in the usage payload, ≈ 5 KB.

The first half is now false of the shipped build, so it is retracted rather than patched:
the totals include those bytes and say what part of them belongs to no adapter. The second
half was right about the shape of the fix and slightly wrong about its price: measured
**+3 584 B**, not ≈5 KB.

What was *not* wrong, and is kept: the engine's refusal to credit an interface that did not
carry the bytes (`job.rs:1090`), and the ruling that folding `n0` into the adapter sums is
the false instrument. This change did not move a single byte into an adapter row — the
`manual-with-bypass` leg and the extended engine test both prove `net_usage` stayed where it
was.

## 8. Gate and suite figures on this build (`writer:` the runs above, 2026-11-05)

- `cargo test --release -- --test-threads=1` → **203 passed, 0 failed, 6 ignored**.
- `npm run typecheck` → clean (and the now-unused `Network` type import was dropped from
  `shared/ipc.ts`).
- `node scripts/verify-m0.mjs` → **M0 GATE: PASS (exe 4.36 MB)**, including
  `schema v4 (4)`, `17 tables (17: …, unattributed_usage)`, `still schema v4 after a second
  open (4)` and `and 17 tables (17)`.
- `proxy-run.mjs` → **13 legs `allPass: true`** on 4 573 184 B
  (`proxy-report-2026-11-05T06-35-41-651Z.json`).
- `auth-run.mjs` (the credential suite, unchanged by this slice) → **15 legs
  `allLegsPass: true`** on the same build, 106 `check(` sites counted with
  `grep -c "check('" auth-run.mjs`, report
  `auth-report-2026-11-05T06-39-01-945Z.json` with `bytes.exe = 4573184`. Every credential
  claim that was green before the ledger change is green after it, including
  `reset-all clears credentials`.
- The counterfactual run on the retained previous binary is
  `proxy-report-2026-11-05T06-38-20-689Z.json` (5 of 13 legs failed; see §6 for which of
  those failures are product evidence and which are harness shape).
- **The wording is proved in the linked binary, not in source** (`writer:` `rg -a -o … | wc -l`
  over `target/release/sucr.exe`, 4 573 184 B, mtime 12:00:48 — the same file the two harnesses
  launched): `belongs to no adapter` 1, `caps and allowances on each card` 1,
  `inside the totals above and on no card` 1, `unattributed_usage` 4, and the retracted half
  — `totals above and the caps` — **0**. A `grep` over `src/` for the retracted sentence returns
  nothing either, so source and binary agree about what is gone.

## 9. What this change does not claim (`writer:` the same runs)

- **The pane re-reads the ledger only while something is moving.** `writer:`
  [main.ts:1720](file:///D:/RE/Sucr/app/src/renderer/main.ts#L1720) calls
  `maybeRefreshNetworks` only when `totals.active > 0`, and
  [main.ts:4343](file:///D:/RE/Sucr/app/src/renderer/main.ts#L4343) floors any read at 5 000 ms
  since `netReadAt`. So a completed proxied transfer appears in the totals at the next read
  taken while another job is active, or on a view change that re-reads — not the instant the
  row turns `finished`. The harness therefore *polls* for the figure to move (§6) rather than
  asserting a timestamp. This is recorded as a coverage fact about what was proved, not as a
  defect claim: no leg measures how long a stale figure can be seen.
- **Week and month windows are unmeasured at the surface.** They are read and carried in
  the payload; no pane prints them, and no gate asserts them beyond "is an integer ≥ 0".
- **A localhost or cardless-route transfer** reaching the ledger is proven by the unit
  test and by the proxied launched legs only. No leg exists for a `127.0.0.1` target on
  purpose.
- **The pane's exact figure** is asserted as "the displayed number moved and the sentence
  names an amount", not as a byte-for-byte match with the ledger: the displayed string is
  `formatBytes`-rendered and the harness would have to reimplement the formatter to compare
  them, which would prove nothing about the app.
- `unattributed_usage` is never pruned. Nothing writes a row older than today, so the table
  grows one row per day the app books unattributed bytes — the same shape `net_usage`
  already has, and the same non-problem.
