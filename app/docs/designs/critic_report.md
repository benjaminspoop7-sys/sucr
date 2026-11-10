# Stage 7 — Design Critique (reconstructed)
> **Provenance, stated plainly.** egm-review requires one report file per stage. During the review these
> findings were only pasted into chat and were never written to disk, which the skill forbids. This file is
> a reconstruction written after the review concluded: each reviewer's own report text is copied verbatim
> out of the session transcript
> (`b3e35f46-b23d-4ce7-a3ea-456fdf83e848.jsonl` in the Qoder project directory). Nothing below is paraphrased
> or added by the orchestrating agent, except this note and the disposition lines at the end.

---

<!-- reviewer: Agent "Stage 7 design critic review" finished | transcript source: note_46377_4.txt -->

- 🔴 Blocker — `per-network-limits.md:53-59`: The design proposes UI controls on adapter cards but specifies no IPC write path. `ipc.rs` exposes only `network:list` and `network:set-enabled` (seen `ipc.rs:190-201`); `network_payload` publishes `speedLimit`/`dataLimit`/`dataLimitPeriod` read-only (seen `ipc.rs:1360-1362`). Without a new channel (and likely DB setters, which are not described), "let the user set them" cannot be delivered. Add a `network:set-limits` channel and its DB write to the architecture.
- 🔴 Blocker — `per-network-limits.md:61-65` + `:105`: The Quota Gate depends on `net_usage`, but the design never defines period boundaries or reset semantics. `net_usage` stores `period_key`, `day`, `week`, `month` (seen `schema.sql:144-152`), and `data_limit_period` allows `day|week|month` (seen `schema.sql:55`), yet the Non-Goals explicitly exclude "A calendar reset day or timezone database." This is a direct contradiction: monthly/weekly allowances cannot be enforced or reset without period boundaries. Either scope quotas to a single period or add reset semantics.
- 🟡 Major — `per-network-limits.md:46-49`: CUJ 2 claims the engine "shifts remaining blocks to other active adapters" on quota exhaustion. But routes are resolved once at job start and frozen in `RunContext` (seen `ipc.rs:592-617`, `spawn_job`), and `bond_routes` reads the DB at start only. The design does not specify how a route is removed from the bonding pool mid-transfer or how in-flight blocks on that route are handled. This is an unaddressed integration risk.
- 🟡 Major — `per-network-limits.md:74-99` (Alternatives): Rejection rationales repeatedly say "Rejected by author" (items 2, 3, 4, 5). The standard forbids attributing rejection to a maintainer without linking an issue/discussion/PR. Rationales for items 2-5 also lack concrete technical trade-offs beyond one-line cons. Rewrite with evidence-backed reasons or link the relevant discussion.
- 🟡 Major — `per-network-limits.md:103-107` (NFRs): The NFR section covers only performance and rollout. The standard requires explicit Risks & Mitigations (failure modes, security, scalability). The design introduces a new in-memory registry of per-network buckets and a quota gate but does not address memory growth with many adapters, stale buckets for removed adapters, cache invalidation for the `net_usage` cache, or failure behavior when the gate is unstable. Add a Risks & Mitigations section.
- 🟡 Major — `per-network-limits.md:33-38` (Open Questions): "Size Margin" references "~20 KB headroom" and Alternatives reference "4.3 MB ceiling" (`:79`). Neither number is defined or measured anywhere in the reviewed sources (unknown). These are unverified assertions; replace with measured values or remove.
- 🟢 Nit — `per-network-limits.md:15`: The claim that "the app's internal milestone record claims M2 already supports this" is not verifiable from the provided sources (unknown). If the milestone ledger is in the repo, cite its file:line; otherwise mark as unknown rather than asserted.

---

<!-- reviewer: Agent "Stage 7 structural critic review" finished | transcript source: note_46368_3.txt -->

- 🔴 Blocker — `per-network-limits.md:15` claims three per-network columns at `schema.sql:53`; **seen** `schema.sql:53-55` confirms `speed_limit`, `data_limit`, `data_limit_period` exist. However the doc later proposes renaming `speed_limit`→`rate_cap` and `data_limit`→`data_allowance` in IPC (`per-network-limits.md:128`), while leaving the DB schema columns unchanged — an unexplained split in naming between storage and IPC layers. No migration or aliasing strategy is stated. Recommendation: either rename both or document why only IPC renames.

- 🔴 Blocker — `per-network-limits.md:54` states `AppState` at `lib.rs:213` holds a single `Arc<Bucket>` for the global cap. **seen** `lib.rs:213` is `global_limit`, yes. But the doc's `BucketRegistry` is to live in `limit.rs` (impl plan #1, line 116), while `AppState` construction in `lib.rs` is where the global bucket is instantiated — no integration point is specified for wiring registry into `AppState`. Recommendation: specify `AppState` field addition and construction site.

- 🟡 Major — `per-network-limits.md:57` cites `charge(buckets: &[&Bucket], ...)` at `limit.rs:104`. **seen** `limit.rs:104` is `pub fn charge(buckets: &[&Bucket], n: usize, cancel: &AtomicU8)`. Accurate. But the doc says `fetch_block` at `job.rs:2493` will "accept the network bucket"; **unknown** whether that line is `fetch_block` — I did not read job.rs. Recommendation: verify line anchors before review approval.

- 🟡 Major — `per-network-limits.md:62` says "0 cap means unlimited in Bucket (`limit.rs:36`)". **seen** `limit.rs:95-100` shows `.filter_map(|b| b.cap())` and `unwrap_or(CHUNK)`, consistent with 0→unlimited-ish semantics, but the exact claim at line 36 was not read. Recommendation: verify `limit.rs:36`.

- 🟡 Major — Open Questions (lines 35-37) leave three load-bearing unknowns unresolved (unattributed bytes, throughput jitter, 20 KB headroom) yet the design proceeds to commit to a third bucket and IPC changes. Per feature-spec standard, risks must be identified and mitigated; these are stated as questions, not mitigations. Recommendation: resolve or gate behind measurements before implementation.

- 🟡 Major — `per-network-limits.md:105` NFR says Quota Gate must cache `net_usage` rather than poll SQLite, but impl plan #3 (`per-network-limits.md:122-124`) says `quota_gate(network_id)` polls `net_usage`. Contradiction between NFR and plan. Recommendation: reconcile.

- 🟢 Nit — `per-network-limits.md:78-84` alternatives cite "Rejected by author" without issue/PR links, violating Alternatives Rule's attribution requirement (rejection rationales must not attribute to a maintainer without linking). Recommendation: add links or remove attribution.

- 🟢 Nit — `per-network-limits.md:106` requires a feature flag but no flag name, storage location, or default is specified. Recommendation: specify.

---

<!-- reviewer: Agent "Stage 7 style critic review" finished | transcript source: note_46359_2.txt -->

- 🔴 Blocker — Alternatives Considered (lines 84, 89, 94, 99): four rejections are attributed to "author" ("Rejected by author"), with no linked issue/discussion/PR. Violates the Alternatives Rule in `feature-spec-standard.md` (line 20). Recommendation: replace author attribution with concrete technical/scope trade-offs, or cite the discussion/PR.
- 🔴 Blocker — Risks & Mitigations: the standard (line 26, 38-39) requires explicit failure modes, security implications, scalability limits, and mitigations. The doc's `Non-Functional Requirements` (lines 103-107) lists only a cache suggestion and a feature flag; no risks, failure modes, or mitigations are enumerated. Recommendation: add a Risks & Mitigations section naming concrete failure modes (quota gate stall, route ejection mid-block, cache staleness) and mitigations.
- 🟡 Major — Open Questions (lines 35-38): "Unattributed Bytes" offers three options with no decision criterion or owner; "Throughput Variance" and "Size Margin" pose questions with no measurement method or threshold. Recommendation: convert each to a decision with a measurable gate or move to Risks.
- 🟡 Major — Problem (line 15): claim "the app's internal milestone record claims M2 already supports this, which is an untruth" is unverifiable from the supplied files (no milestone ledger read); label as inferred/unknown. Recommendation: cite the ledger file:line or soften to "appears to claim."
- 🟡 Major — Technical Plan (line 62-65): "poll `net_usage`" contradicts NFR line 105 ("must cache `net_usage` rather than polling"). Internal contradiction. Recommendation: reconcile wording (cache vs. poll) and state cache invalidation.
- 🟡 Major — Goals (line 22): "Show live used/remaining usage in units the engine enforces" is vague; no unit definition or refresh cadence. Recommendation: specify units (bytes), source (`net_usage`), and update frequency.
- 🟡 Major — Alternatives (line 79): "size spike fails to meet the 4.3 MB ceiling" introduces an unexplained 4.3 MB ceiling and "size spike" jargon; no source. Recommendation: define the ceiling and its origin or remove.
- 🟢 Nit — Overview (line 11): "Give each network adapter its own transfer cap and its own byte allowance, and let the user set them." Redundant with Goals; could be one sentence. (DROP-level filler.)
- 🟢 Nit — Technical Plan (line 53): "Registry of Network-Scope Buckets" and `BucketRegistry` introduce jargon without defining "bucket" or "scope" for a fresh reader; `limit.rs` header explains token-bucket but doc does not. Recommendation: add a one-line definition.
- 🟢 Nit — CUJ 2 (line 49): "marks the route as unavailable, and shifts remaining blocks to other active adapters" leaves the ejection mechanism and ownership unspecified. Recommendation: name the component that performs ejection.

Counter-argument to the document's framing: treating the dead columns as a feature gap assumes the columns were intended for this feature; they may be vestigial. Counter-argument to my own recommendations: demanding full risk sections and decision criteria may over-scope a proposed design doc; a lighter "open risks" list could suffice, but the standard explicitly requires risks.

---

## Disposition (orchestrator, added after the review)
