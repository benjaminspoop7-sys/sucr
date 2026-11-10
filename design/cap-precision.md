# Cap enforcement precision — measured, with the predicate written down

Date: 2026-11-05. Build under test: `sucr.exe` **4,569,600 B**, mtime `2026-11-05T01:18:25.678Z`
(`evidence-noun: stat`, recorded inside every report as `report.exe`).
Cost of this work: **0 product bytes** — see §8.

Read this file with the admission rule from `project-sucr-open-defects.md`: every claim about what
Sucr does carries a `writer:` (what code produces the number), a `predicate:` (exactly what is
counted and over what span) and an `evidence-noun:` (the run or locator that shows it). A claim
without the three tags below is marked `unmeasured` and must not be quoted as a result.

---

## 1. What this file closes

The defect record held: *"the same nominal 2,097,152 B/s (2 MiB) cap measured **2,260,210 B/s
median (+7.8 %)** in the step-6 harness and **2,015,033 B/s (−3.9 %)** for 2,000,000 B/s in step 3's.
The likely cause is the denominator … but that is a **hypothesis**; no run here separated
server-written from app-read. Do not cite either figure as the enforcement precision until one does."*

Four things were wrong with that sentence, and all four are now measured:

1. **The `−3.9 %` was an arithmetic error by me.** Step 3's 2,015,033 B/s was divided by step 6's
   2,097,152 instead of by its own saved 2,000,000. Against its own cap it is **+0.75 %**, which is
   what [the sequence document already said](per-network-limits-implementation-sequence.md) at line
   376. Both historical figures were *above* their caps; the record's sign was wrong.
   `evidence-noun: recompute over the two stored reports`.
2. **The denominator cannot be the cause of the gap, because both headline figures were already the
   same denominator.** Step 6's `measuredBps` is the fixture's `served` counter
   (`writer: server.mjs note(bytes){ state.served += bytes }`), and step 3's headline `rate` is the
   same counter (`writer: cap-run.mjs rate(samples).wire`). The claimed "socket buffering is not the
   app's rate" distinction was real as a *possibility* and false as an *explanation of that pair*.
   `evidence-noun: grep of both harness sources`.
3. **A span sweep dissolves the rest.** Step 6's own two stored capped legs read **+2.07 %** and
   **+7.38 %** on the same counter at the same late span start — the two legs of one harness, one
   build, one setting, disagree by 5 pp. A figure with a 5 pp leg-to-leg spread was never a precision
   measurement, so there was no discrepancy to explain. `evidence-noun: recompute over
   report-step6-2026-11-03T23-12-13-829Z.json`.
4. **The build was not the cause either.** Re-running step 6's own harness (card path, typed `2` +
   `MiB`, row read back `2097152`) on today's exe reproduces the family: **+8.6 %** and **+3.4 %**,
   median **+6.0 %**. Same numbers from a different binary → they come from the instrument, not the
   limiter. `evidence-noun: launched-exe run report-step6-2026-11-05T04-52-52-849Z.json`.

The claim is **retracted**, not patched, and the corrected statement is §6.

---

## 2. The two counters, and what each one actually is

| counter | `writer:` | `predicate:` | resolution |
|---|---|---|---|
| **fixture-written** (`served` / `wire`) | `server.mjs` `note(bytes){ state.served += bytes }` — bytes the Range fixture pushed into its sockets | every byte the fixture wrote, whether or not the app has read it | grows smoothly, ~262,144 B steps between polls |
| **app-read** (`downloads.verified_bytes` / `db`) | `engine/job.rs:472 set_progress()` — the only `UPDATE downloads SET verified_bytes`; on the range path fed by `let sum: u64 = done.iter().sum()` at `job.rs:1904`/`:1967` behind `if last_flush.elapsed().as_millis() >= PROGRESS_MS` (`const PROGRESS_MS: u128 = 150;` `job.rs:33`) | **whole finished blocks only**, so it moves in 8,388,608-byte steps and reads 0 until the first block closes | 8,388,608 B quantised; first booking at 4.7–7.3 s into a capped 8-stream job |

Neither counter is *bytes charged at the limiter*. The limiter (`engine/limit.rs reserve()`) charges
on read and the engine keeps no exposed counter for that act; `block_attempts.bytes` is booked at
attempt end, so it carries the same step function. **`unmeasured`: bytes-read, as a continuous
quantity.** Every precision statement below is therefore bounded by that gap, not by the bucket.

---

## 3. The identity that ties the two counters together (the reason "denominator" was the wrong noun)

Both series are sampled from the same tick list, so for any span
`[a, b]`:

    (app_rate − wire_rate) × span  ≡  excess(a) − excess(b)     where excess(t) = wire(t) − db(t)

Measured, not assumed — `evidence-noun: report-2026-11-05T04-38-55-397Z.json` and
`report-2026-11-05T04-44-22-568Z.json`, 6/6 legs, mismatch 0.00 MiB (**0.000 blocks**) in every leg:

| leg | span (t≥5 s) | (app−wire)×span | excess drop | blocks |
|---|---|---|---|---|
| capped#1 30 s | 25.0 s | 9.20 MiB | 9.20 MiB | 0.000 off |
| capped#2 30 s | 25.0 s | −0.08 MiB | −0.08 MiB | 0.000 off |
| capped#3 30 s | 25.0 s | 0.26 MiB | 0.26 MiB | 0.000 off |
| capped#1 24 s | 18.9 s | 8.57 MiB | 8.57 MiB | 0.000 off |
| capped#2 24 s | 18.9 s | −0.60 MiB | −0.60 MiB | 0.000 off |
| capped#3 24 s | 19.1 s | 7.69 MiB | 7.69 MiB | 0.000 off |

Consequence, and this is the part that mattered: **the two denominators cannot disagree about the
limiter at all.** Their disagreement is, by construction, a change in the run-ahead. So a
+22…+24 % app reading is one 8,388,608-byte block released inside an ~19 s span
(8,388,608 / 18.9 s = 443,842 B/s = **+22.2 %** on a 2,000,000 B/s cap — the measured +24.21 %), and
a −4.5 % wire reading is that same block's worth of buffer *draining* during the span. The counters
**bracket** the true rate; they do not compete with it.

---

## 4. Span table, all five runs on the shipping exe

Every row is signed deviation against **the cap that leg actually saved**, on the
`fixture-written` counter (F) or the `app-read` counter (A). `evidence-noun:` the four
`report-2026-11-05T04-*` runs plus `report-step6-2026-11-05T04-52-52-849Z.json`, all on exe 4,569,600.

| run | path | saved cap | window | whole F | whole A | t≥1 s F | t≥1 s A | t≥5 s F | t≥5 s A | excess@5 s → end |
|---|---|---|---|---|---|---|---|---|---|---|
| A | channel | 2,000,000 | 30 s | +7.90/+8.03/+8.04 % | −1.33/−0.88/−1.04 % | +0.42/+0.55/+0.54 % | +2.13/+2.60/+2.41 % | **−0.51/+0.16/−0.40 %** | +18.75/−0.02/**+0.14 %** | 16.50/7.03/7.47 → 7.30/7.11/7.21 MiB |
| B | channel | 2,000,000 | 24 s | +10.81/+10.82/+12.72 % | −2.14/−2.40/−2.05 % | +2.07/+2.03/+3.53 % | +2.23/+1.91/+2.26 % | +0.46/+1.12/+2.91 % | +24.21/−0.53/+24.02 % | 16.50/7.46/16.50 → 7.93/8.05/8.81 MiB |
| C | channel | 2,097,152 | 30 s | +6.25/+6.36/+6.34 % | −8.11/−9.89/−4.75 % | +6.54/−0.24/−3.69 % | −4.89/−6.74/−1.42 % | −4.10/**−4.50**/−7.03 % | +10.59/+8.45/+14.61 % | 16.00/16.25/17.50 → 8.65/9.78/6.68 MiB |
| D | channel | 4,000,000 | 15 s | — | — | +0.85 % (median) | +4.77 % (median) | **−0.72 %** (median) | +44.6 % (median) | 26.25/8.00/26.25 → ~9 MiB |
| E | **card** | 2,097,152 | 24 s | +18.52/+13.87 % | +0.35/−5.79 % | — | — | +7.23/+1.97 % (t≥5 s) | +26.78/+19.20 % | 16.25/16.00 → 20.75/34.25 MiB mid-leg |

Three readings that only this table makes possible:

- **Window length alone is worth pp.** Same build, same cap, same path: A's whole-window F is
  +8.03 % and B's is +10.82 %, because the run-ahead is a roughly fixed pile of bytes (~1 block,
  7.1–8.8 MiB at window end) amortised over less time. Any whole-window figure is therefore a
  statement about the window, not about the cap.
- **The card path runs further behind than the channel path.** E's excess grows to **34.25 MiB**
  (four blocks, ~17 s of transfer) before the ledger dumps ~25 MiB in the last 10 s of the leg —
  which is why E's t≥14 s app reading is **+127.95 %**. That is the held `verified_bytes`-lag defect
  showing up on the card path; the cause split (flush cost vs `database is locked` against the 5 s
  deck refresh vs the CDP read path) is still **`unmeasured`**, and this run does not separate them.
- **A cap of 2,097,152 and a cap of 2,000,000 are indistinguishable in absolute bytes.** A's steady
  F median is 2,001,000–2,005,000 B/s and C's is 2,002,860–2,011,255 B/s, and every 30 s capped leg
  in A and C served **exactly 67,108,865 B** (`evidence-noun: report passes[].wireBytes`, 6 legs).
  So this harness has **less than 4.86 % resolving power against the saved value**, and the identical
  byte total is a counter artifact whose mechanism is **`unmeasured`** (candidates: block-sized
  booking in the fixture's own accounting, or the 8-stream × 1-block claim pattern).

---

## 5. What the limiter itself does (scaling, proven)

Run D put a **4,000,000 B/s** cap on the same adapter, same build, same host, 3 rotated legs:
headline **4,034,134 B/s (+0.85 %)** and steady **−0.72 %** on the fixture-written counter, with the
transfer pinned at width 8 and `ramp` silent — the same shape as the 2 MB/s legs. Doubling the saved
cap doubled the delivered rate.

`writer:` `engine/limit.rs Bucket::reserve()` (`slice_of(cap) = (cap/20).clamp(8 KiB, 256 KiB)`,
tokens capped at one slice, debt preserved, sleep in ≤50 ms ticks).
`evidence-noun:` launched-exe run `report-2026-11-05T04-58-14-117Z.json`.

**The cap scales with the setting.** The earlier worry — that a cap could read +7.8 % because the
bucket was loose — is not supported: the loose readings belong to the instrument's span and counter,
and the two legs of run C that read −4.5 % are the *same absolute rate* the +0.1 % legs of run A
delivered.

---

## 6. The sentence to quote from now on

> A saved per-adapter rate cap paces a real 8-stream 1 GiB transfer to **within about ±1 % of the
> saved value**, measured as the fixture-written byte counter's endpoint slope over a span that starts
> past the first 5 s, three rotated legs per point, on exe 4,569,600: **−0.40 %** at a saved
> 2,000,000 B/s and **−0.72 %** at 4,000,000 B/s. `evidence-noun:` launched-exe runs
> `report-2026-11-05T04-38-55-397Z.json`, `report-2026-11-05T04-58-14-117Z.json`.

And the three limits that belong glued to it:

- **Nothing on this box resolves 4.86 % in a saved cap.** 2,000,000 and 2,097,152 both delivered
  ≈2.00–2.01 MB/s, so a card-typed `2 MiB` cap cannot be verified to better than its own MiB-vs-10⁶
  difference by any counter exposed over CDP. Do not quote a cap error smaller than ~5 %.
- **The two counters bracket, they do not compete.** Over a 24–30 s leg the fixture-written reading is
  the delivery rate and the `verified_bytes` reading is the booking rate; they differ by exactly the
  run-ahead change (§3), which is worth up to ±24 % on a 2 MB/s cap and up to +44 % on a 4 MB/s cap.
- **No whole-window figure is an enforcement figure.** +8 % (30 s) / +10.8 % (24 s) / +18.5 % (card,
  24 s) on the same build and cap are the window talking.

Retracted permanently: **"cap measured +7.8 % vs −3.9 %; the harness denominator is the likely
cause"** and every restatement of it, including "the +7.8 % over the 2,097,152 B/s nominal, where
step 3 measured −3.9 %". Also retracted as wrong-at-the-time: the `−3.9 %` sign.

---

## 7. What is still `unmeasured` (do not upgrade these without a run)

1. **Bytes charged at the limiter.** No exposed counter; `block_attempts.bytes` is booked at attempt
   end. A precision claim tighter than ~±5 % needs one (product work, byte cost owed — *not* agreed).
2. **Why the card path falls four blocks (34 MiB) behind the fixture while the channel path stays at
   one block.** Candidate causes are the three already held against the `verified_bytes`-lag defect;
   this work does not separate them.
3. **Why six 30 s capped legs served byte-identical 67,108,865 B across two different caps.** A
   fixture/counter artifact, mechanism not measured.
4. **How long a span must be to converge.** 30 s and 24 s legs both settle to ~±1 % on the
   fixture-written steady span, so the answer is "≥ 3 block-times at this cap"; a formal convergence
   curve was not measured.

## 8. Cost and hygiene

- **0 product bytes.** `stat` after all five runs: `sucr.exe` **4,569,600 B**, mtime
  `2026-11-05T01:18:25.678Z` — unchanged from before the first run today. Every product source is
  older than that build (`limit.rs` 2026-11-04 00:16, `job.rs`/`mod.rs`/`index.html` 04:45,
  `main.ts` 05:22, `lib.rs` 06:38, `ipc.rs` 06:44 local vs the exe's 06:48 build), so nothing this
  task touched was relinked. All edits are under `D:\RE\_cap-harness` (`cap-run.mjs` flags, four
  read-only recompute scripts).
- **Harness-only changes:** `--capped`, `--window <ms>`, `--cap <bps>` flags, the second denominator
  and the excess/excess-growth fields per leg, and the capped-track sampling. `pacedToCap`'s existing
  15 % threshold is unchanged and now visibly far too loose to be a precision gate (§6 says why).
- **Void data:** run E's *uncapped* legs (7.71–8.09 MiB/s against 9.07–9.16 MiB/s measured clean on
  2026-11-03) ran concurrently with run C on the same link, so they are not comparable and are not
  quoted anywhere above. Run E's capped legs are quoted only for the excess trajectory and the span
  sweep, which are cap-bound at 2 MB/s each against a ~117 MB/s link.
- Each harness run used its own `--data-dir`, its own WebView2 folder and its own CDP port
  (9345/9347) under `D:\RE\_cap-harness\run-<stamp>`; the user's own app instance, profile, window and
  database were not touched, and no `sucr.exe` was left running after each report.
