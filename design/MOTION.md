# Sucr — MOTION.md

**Status: drafted 2026-11-03, not ratified.** Nothing in this file is `DECIDED` on its own authority —
per the standing rule for this project, only the user can promote an entry to `DECIDED`, and this file
records what the code already does and what it therefore asks them to settle. **One entry now carries
their word: `OPEN-3` is `DECIDED` (§3, §6, §11), and §10's wave-travel reversal is flagged for the same
kind of ruling and does not have one yet.**
**Executed the same day, in the shipped binary** (obligation → taste → mechanical, §9): O-003 ships
with a card control, T-004 ships, `OPEN-1` and `OPEN-2` ship with the §4 exclusion honoured.
`OPEN-3` is untouched at that point and still needs a ruling (it gets one the same day — §11). Evidence
for each is inline in §3, §4 and §6.
**Second pass executed the same day** (§10): the four residual defects §9 owed are closed in the
shipped binary — `--wave-travel` now actually stops the crest, the geometry is re-read when the
motion mode flips mid-run, the last `transition: all` is gone, and the last keyword curves are
tokens. **One of those is a behaviour reversal of a previously documented ruling** and is flagged in
§10 for the user to keep or veto.
**Third pass executed the same day, under the user's `OPEN-3` ruling** (§11): **17** computed motion
lines — 13 surfaces, 12 of them in the capture — snap to the six rungs; the toolbar/theme one-shot
flicks and the whole `.tabs` surface are recorded as the single exception class; and §4's reason for the
carve-out is corrected from a stylistic preference to a measured load-bearing mechanism. **Two of my own
errors in that execution are named in §11, both caught by measuring the shipped binary rather than the
source, and both corrected there. A third, smaller one is too: this paragraph first said 16, and §11
now says why the count is 17.**
**Rung: 3 — AS-IS + OBSERVED, with one entry at 4.** (Ladder: 1 undocumented → 2 craft defaults → 3 this
file → 4 ratified `DECIDED` entries → 5 machine-enforced. The ladder is per entry, not per file:
`OPEN-3` is at 4 because the user wrote it; everything else here is at 3.)

## 0. Platform — settled by measurement, not by assumption

This is **not** an Android app. `find D:/RE -maxdepth 4` for `build.gradle*`, `*.kt`,
`AndroidManifest.xml`, `settings.gradle*` returns **zero files**. The UI is a WebView2 renderer
(`app/src/renderer/{app.css,main.ts,index.html}`) embedded in a 4,478,464 B Tauri binary (post-`OPEN-3`;
the pre-batch build it was written against measured 4,477,952 B).
So the `animate-compose` / `improve-android-motion` / `review-android-motion` pack cannot be run
verbatim: its **floor** rules F-001..F-010 name Compose APIs (`Animatable`, `Modifier.offset { }`,
`animateItem`, `sharedElement`) with no equivalent here and are reported **not applicable**.
What is applied is the Material-3-agnostic core — the `obligation` rules O-001/O-002/O-003 and the
`taste` rules T-002/T-004/T-009/T-013/T-016/T-017/T-019/T-020/T-023/T-024/T-026/T-027/T-028 —
bound to CSS custom properties, which is the adaptation the pack itself invites with its
"Outside Material" section. Every future finding in this frame is a **webview adaptation**, never
an Android ruling.

## 1. Source of truth

`design/tokens.css:191-205` defines the scale. `scripts/build.mjs:42-46` copies it **verbatim** into
`dist/renderer/tokens.css` and stamps its sha256; `app.css` has no `@import` and no local
`--dur-*` definition, and `index.html:8` links `./tokens.css`. One source, no mirror to drift.
`MOTION.md` lives beside the source, in `design/`.

## 2. The scale — Intent and Binding

| Token | Value | Intent (as written in the file) | `var()` uses in app.css |
|---|---|---|---|
| `--dur-1` | 80 ms | state tint | **4** |
| `--dur-2` | 120 ms | hover, tab switch | **54** |
| `--dur-3` | 180 ms | chip, map cell | **3** |
| `--dur-4` | 220 ms | entrance, overlay | 9 |
| `--dur-5` | 260 ms | bar width, sheet | 12 |
| `--dur-6` | 400 ms | large-surface transition | **0 — and 0 literals. Still dead.** |
| `--e-std` | `cubic-bezier(.2,0,0,1)` | standard, symmetric | **88** |
| `--e-dec` | `cubic-bezier(.05,.7,.1,1)` | arriving motion | **9** |
| `--e-acc` | `cubic-bezier(.3,0,.8,.15)` | leaving motion | **0 — dead.** |
| `--e-back` | `cubic-bezier(.34,1.56,.64,1)` | passes its target, then settles | 7 |
| `--e-out-quint` | `cubic-bezier(.16,1,.3,1)` | leaves instantly, arrives slowly | 3 |

Re-measured after §11's third pass, every column at once by `grep -o` on `app.css`. **Treat this
column as a snapshot, not as a diff.** The earlier version of this table was updated only for the
tokens each pass touched, so its per-token before/after is not trustworthy — `--dur-3` reads 3 now and
the third pass added two `--dur-3` sites, which means the previous "3" was already stale (real value 1).
The pre-batch source is not preserved either (`D:/RE/Sucr` is not a git repo; `src/renderer/app.css`
and `dist/renderer/app.css` were both overwritten by the build, and the rename-aside exe holds its copy
brotli-compressed), so **the authoritative before/after for this pass is the shipped-window computed
fingerprint in §11**, not these counts.

**OBSERVED.** The `--dur-3` *comment* (`tokens.css:193`, "chips, map cell") is now the stale part, and
the three sites that read it are why: `.add-bar { animation: addBarSlide var(--dur-3) }`
(`app.css:754`) and `.toast-notice { animation: toastFade var(--dur-3) }` (`:2933`) are an entrance and
a transient, not a chip or a map cell; the only remaining hover-linked read is `transform var(--dur-3)
var(--e-back)` on an icon button (`:604`). The chip itself left `--dur-3` in the second pass (§9 item 3:
`.add-preset-chip` now names four properties on `--dur-2`). Editing the comment is **not free here**:
`tokens.css` is copied verbatim (`scripts/build.mjs:42-46`), so a comment edit is billed to the binary
in full, unlike the same edit in the minified `app.css`. Recorded, not done.
**OBSERVED.** `--dur-6` and `--e-acc` are still defined and unreachable after the swap, so either
they describe motion the app no longer makes or that motion is one of the off-scale literals in §3.
The third pass narrows that: every off-scale literal left in the file is a **one-shot** value
(`modeFlipSpring 320`, `sunSpinFlare 320`, `moonRock 340`, `autoPulse 300`, `navPillPop 280`, plus the
`.tabs` flicks), and `--dur-6` is a *large-surface transition* — a different job, so this scale rung is
not where that motion belongs.

**OBSERVED.** The scale is correct in form and honoured on the surface that matters most:
`.tbar-fill { transition: width var(--dur-5) var(--e-std) }` (`app.css:1204`) — no overshoot on the
element whose width *is* the progress value (T-004 honoured there, and now on the wizard bar too,
§6). 102 declarations read tokens and `grep -c cubic-bezier app.css` is **0**: no curve is typed
into a rule any more.
**OBSERVED.** `--dur-6` and `--e-acc` are still defined and unreachable after the swap, so either
they describe motion the app no longer makes or that motion is one of the off-scale literals in §3.
**OBSERVED, second pass.** Three more unreachable tokens are in the same file and were not counted
above because they are read by script, not by a CSS rule: `--wave-indeterminate` (`tokens.css:249`)
and `--wave-ring-amp` (`:250`) match **0** times in `app.css` *and* `main.ts` — defined, never read
by anything. `--wave-travel` (`:248`) is the opposite case: **no CSS rule reads it** (0 matches in
`app.css`), it is consumed only by `main.ts:827` through `cssPx()`, which is why it can be honoured
or defeated by a JS guard and why §10 is about a JS change, not a CSS one.

## 3. The divergence, measured

Baseline, as drafted before the swap: `app.css` then carried **61** millisecond literals inside
`transition:`/`animation:` declarations, against 45 token reads. The line numbers in the bullets
below are the draft's and have since moved.

- **26 of the 61** (in **22** declarations) already equal a rung exactly — `80×4, 120×6, 180×9,
  220×2, 260×5`. Substituting `var(--dur-*)` there is **zero visual change**.
- **31** are off-scale: `100×1, 140×5, 150×1, 160×4, 200×4, 240×6, 280×4, 300×2, 320×3, 340×1`.
  Snapping any of these to the nearest rung **changes what the user sees**. Rung-adjacent and cheap
  (`140→120|180`, `240→220|260`, `280→260`, `320→260|400`); `200` and `340` have no near neighbour.
- **4** are the `1ms !important` of the reduced-motion block itself.
- **11 inline curves, two shapes, and they are the only inline curves in the file:**
  `cubic-bezier(0.34,1.56,0.64,1)` at `app.css:383, 434, 545, 604, 1943, 1976, 3100, 3149` (8) and
  `cubic-bezier(0.16,1,0.3,1)` at `:524, 754, 3079` (3).

`OPEN-1` — **done.** `--e-back: cubic-bezier(.34,1.56,.64,1)` and `--e-out-quint:
cubic-bezier(.16,1,.3,1)` are in `tokens.css` and all 11 inline sites are bound to them. No value
changed; `app.css` now contains zero `cubic-bezier(` literals.

`OPEN-2` — **done, with the §4 exclusion.** Every exact-rung literal outside the tab strip is a
`var(--dur-*)` read. The residue, re-counted in the shipped file: **44** millisecond values remain
inside `transition:`/`animation:` declarations — `1×4` (the reduced-motion block itself), `180×6`
and `260×4` (all `.tabs` descendants, deliberately kept), and **30** off-scale values
(`100, 140×5, 150, 160×4, 200×4, 240×5, 280×4, 300×2, 320×3, 340`). Flagged rather than smoothed:
this file's draft said 31 off-scale and 240×6; the current single-line scan reproduces 30 and 240×5,
so one draft count came from a looser pattern than the one used now. The load-bearing claim is not
the arithmetic, it is the fingerprint below.

**Proven value-neutral in the binary, not in a harness.** `Temp/sucr-motion-fp2.mjs` dumps every
computed duration, curve and animation name in the live window, for both motion modes. Run against
the preserved pre-change exe and the final one, the de-duplicated motion inventories are **72 lines
in `full` and 1366 in `reduced`, and exactly one line differs in each**:

```
#wizard-progress-fill  old   0.24s | cubic-bezier(0.34, 1.56, 0.64, 1)
                     final   0.26s | cubic-bezier(0.2, 0, 0, 1)
```

which is T-004, the one intended change. The other four diffs across the run are the O-003 control's
semantics on `.hero-progress` (`role`, `tabindex`, `aria-pressed`, `aria-label`), not motion values.
Raw element-instance counts moved (3271 → 3223 in `reduced`) because the capture is taken mid-
transfer and the map had fewer claimed blocks at that instant; the *unique* motion tuples did not
move, which is what the swap is accountable for.

`OPEN-3` — **`DECIDED` by the user on 2026-11-03, and executed (§11).** The ruling was the split
option, not "snap everything" and not "extend the scale", in three parts: **(1)** snap the
hover-linked, entrance/overlay and transient families to the existing rungs; **(2)** record the
toolbar/theme one-shot click flicks **and** the `.tabs` literals as the single exception class that
keeps its millisecond values; **(3)** for the hover family specifically, the `--dur-2` 120 ms rung.
A recount, because §3's headline numbers were measured with the loose one-line pattern this file already
flagged in `OPEN-2`, and a loose pattern is exactly what a surface-by-surface ruling cannot be executed
against. Two scans, both re-run after the batch: `grep -nE '[0-9]+ms' app.css` returns **27** lines —
22 declaration lines, 4 of the reduced block's own `1ms !important`, 1 prose comment — and a targeted
search for a millisecond on a declaration *continuation* line returns **nothing**, so the 22 are the
complete enumeration; `app.css` therefore holds **24** millisecond values in `transition:`/`animation:`
declarations (**28** with the blanket's four), against the **44** §3 recorded before the pass. Of those
24, **15** are `.tabs` descendants, **7** are the flick family and **2** are the `[role="tabpanel"]`
slides that belong to neither (below). What the ruling authorised to move did move, and nothing else
did: the shipped-binary diff in §11 counts **17** changed computed lines and no others. The residue,
line-attributed:
`.tab-pill 180×2`, `.tab-icon 140×2`, `tabTap 180`, `.icon-speedo .needle-group 180`,
`needleGlide 320 forwards`, `suckerBlockRipple 240×2`, `suckerDotPulse 260`, `linkSlideL/R 260×2
forwards`, `clockQuarterSweep 300 forwards`, `folderDocLift 260`, `ekgCleanTrace 280 forwards`,
`paneInRight/Left 180×2`, `modeFlipSpring 320`, `sunSpinFlare 320`, `moonRock 340`, `autoPulse 300`,
`navPillPop 280`, and the two spring-settle transitions `.tb-mode svg` / `.theme-seg button svg` at
`180ms` (`:122`, `:172`).

## 4. The trap that governs §3

The reduced-motion contract zeroes the **tokens** (`tokens.css:267` for the pre-renderer gap,
`:271` for the app's own switch) **and** blanket-zeroes every declaration
(`app.css:3036-3043` for the media trigger, `:3046-3060` for the attribute trigger). The blanket
carves out exactly one surface and does so deliberately:

> `.tabs [role="tab"].animating …` (`app.css:2120-2161`, ten rules) — 180/240/260/280/300/320 ms
> one-shot icon nudges; the comment at `app.css:3053-3056` says the flick "is the only thing that
> says the pane actually changed, so it stays".

That exemption is expressible **only because those sites hold literals**. Bind them to
`var(--dur-*)` and `tokens.css:266` sets the token to `0ms` on the root, so the tab flick dies under
reduced motion — the blanket rule's `:not(.tabs)` would be honouring a carve-out that no longer
exists. Consequences:

1. ~~`app.css:2107` (`180ms`) and `:2128, :2132, :2133, :2144` (`260ms`) are inside the 26 "safe"
   swaps in §3 and **must not be swapped** as-is.~~ **Honoured.** The ten `180`/`260` values still in
   the file are all `.tabs` descendants — `.tab-pill` (`app.css:2080`, two values), `tabTap`
   (`:2120`), `.icon-speedo .needle-group` (`:2132`), and the four `.animating` icon keyframes
   (`:2141, :2145, :2146, :2157`). `index.html:659-664` is the containment evidence: the pill and
   the tab icons are children of `div.tabs`.
2. ~~If tokenising wins, the exemption has to be re-expressed on the token side~~ — **not needed**,
   because nothing inside `.tabs` was tokenised. The literal route and the token re-declaration route
   both work; the literal route was taken, so the mechanism in consequence 3 remains unused and
   unexercised.
3. The same logic will apply to any future "keep this one motion when reduced" case. A per-surface
   token re-declaration is the mechanism; a blanket-rule exclusion is not, because the blanket
   cannot reach a value the token already collapsed.

**Corrected by the `OPEN-3` ruling, 2026-11-03 — the literals here are load-bearing, not stylistic.**
The user ruled that the `.tabs` values stay exactly as they are, and this file's reason for that needed
to be measured rather than asserted, because the sentence above ("that exemption is expressible **only**
because those sites hold literals") was right in conclusion and thin in mechanism. The mechanism is a
difference between two kinds of zero:

- **Literals (current state).** Under a reduced trigger the blanket's `:not(.tabs)` guard never reaches
  these rules, so they keep 140/180/240/260/280/300/320 ms and the flick visibly plays. That *is* the
  carve-out.
- **Tokenised (the road not taken).** `tokens.css:266-268` / `:271-272` set `--dur-*: 0ms` on the root
  element, and the blanket's `1ms !important` explicitly excludes `.tabs` — so a tokenised tab would
  resolve to **0 ms**, not 1 ms. `0ms` and `1ms` are not the same end state for the five `forwards`-fill
  animations (`needleGlide`, `linkSlideL`, `linkSlideR`, `clockQuarterSweep`, `ekgCleanTrace`): a
  non-zero duration runs and its fill leaves the element on the final keyframe, while a zero-duration
  one applies the fill with no traversal at all.

The 0-vs-1 distinction is now measured, not reasoned: `Temp/sucr-netcard-read.mjs` read a tokenised,
non-`.tabs` surface under this box's default reduced state and got `--dur-5: 0ms` at the root against
`transition-duration: 0.001s` on the element — two different numbers for the same intent, which is
exactly the wedge that would have closed the tab flick if the literals had been swapped. Nothing in the
`.tabs` surface moved in the third pass, and consequence 3 still stands as the mechanism for any future
"keep this one motion when reduced" case.

**Carve-out re-measured after the swap** (`Temp/sucr-tabs-carveout.mjs`, real mouse press on
`#tab-source` with `data-motion="reduced"`): the flick still runs — `tabTap 0.18s` on the icon and
`0.18s, 0.18s` on the pill — while two tokenised, non-`.tabs` controls in the same run read
`0.001s` (`.settings-nav button` on `--dur-2`, `.nic-modal-backdrop` on `--dur-4 var(--e-out-quint)`).
The second half matters: it shows the blanket was firing, so the flick surviving is the exemption
working and not reduced motion being off. Two sites were left as literals for a different reason and
it should be recorded precisely: `[role="tabpanel"].slide-in-right/left` (`app.css:2174-2175`, 180 ms)
are **not** `.tabs` descendants, so the blanket does reach them; they are also named-suppressed at
`app.css:3071-3072` (`animation: none`). They were left alone to keep the whole tab surface inside
one future ruling (OPEN-3), not because they carry an exemption.

**That deferral is now closed, and it closed with a finding, not an answer.** `OPEN-3` governed the
**off-scale** literals; `paneInRight` / `paneInLeft` are `180ms`, which is exactly `--dur-3`, so they
were never in OPEN-3's scope — they belong to `OPEN-2`, whose rule is already "every exact-rung literal
outside the tab strip is a `var(--dur-*)` read". Whether `[role="tabpanel"]` counts as "the tab strip"
was never written down, which is how two sites ended up deferred by a sentence that did not apply to
them. The swap would be value-neutral in **both** motion modes: under reduce they are killed by the
named `animation: none` whatever the duration says, and under full motion `--dur-3` is 180 ms. So this
is a 2-site, zero-visual-change cleanup that the ruling does **not** authorise, because the user's
split named families, not surfaces, and an entrance slide arguably falls in the snap list. **Left
untouched and flagged for one word: tokenise them, or write the exemption down.**

## 5. Reduced motion (O-001) — mechanism sound; the one surface that refused it now obeys

Two triggers, one body, and the renderer collapses `(prefers-reduced-motion, ui.motion)` into
`html[data-motion]` as its first statement, so the app's own setting outranks the OS — which a media
query alone cannot do, because `1ms !important` can be neither `unset` nor `reverted`ed. Named
suppressions beyond the blanket (`app.css:3045-3053` media, `:3055-3069` attribute — the line numbers
moved when the chip's `transition: all` became four named properties): `:3071-3072` tab panels stop
sliding, `:3074` the live bar shimmer is removed (`content: none`), `:3075` the attempt dot, `:3078`
the rail reveal, `:3081` the transfer stage. This is stronger than the rule asks. **O-001: met.**

The one exception used to be in the script layer: `main.ts` read `--wave-travel` once and clamped it
away, so the CSS said `0s` and the crest travelled anyway. That is closed (§10), and with it the
sentence "one surface refuses it" — with a caveat the user should rule on rather than inherit, because
the old code carried a written justification for refusing.

## 6. Open rulings

| ID | Rule | Where | Status and finding |
|---|---|---|---|
| **O-003** | obligation | `main.ts:961-1031` | **Shipped as option (b)** — a control on the hero card, not a reduced-motion branch. `.hero-progress` is now `role="button"`, `tabindex="0"`, `aria-pressed`, and an `aria-label` that flips between "Pause" and "Resume progress animation"; one press stops the wave's frame loop, the live bar's shimmer and the active attempt dot at the same time, via `html[data-ambient="held"]`. Session-only on purpose: a stored key would become the 18th setting the release gate counts, and a pause that survived a restart would strand a fresh download silently. Measured in the shipped window at `ui.motion=full` (`Temp/o003-final.json`): shimmer and dot `animation-play-state: paused`, wave frozen (`d` and thumb `x` identical across 900 ms), release resumes, row selection unchanged through six presses (`"1,1,1,1,1,1"`), focus ring `solid 2px`, reachable in three Tab hops. Three defects found only by running the binary — each fixed there, not in a harness: **(1)** a plain `click` listener never fires, because the card's `onTap` calls `preventDefault()` on `pointerdown` and the row is replaced between down and up, so the control uses `onTap` too; **(2)** "paused" still drew — `syncWaveLoop()` paints once with `performance.now()` on every ~150 ms refresh, so the clock is frozen through `ambientClock()` and `waveTick()` bails on `ambientHeld` itself rather than trusting the cancel; **(3)** keyboard focus died with the discarded node, so a second Enter landed on `<body>` — restored on the replacement. The third fix took two attempts: `focus()` called from `ambientControl` is a no-op because `cardFor` builds the node detached and `replaceChildren` attaches it afterwards, so the restore is a `queueMicrotask`. |
| **T-004** | taste | `app.css:wizard-progress-fill` | **Shipped.** `.wizard-progress-fill` was `width 240ms cubic-bezier(0.34,1.56,0.64,1)` — overshoot on an element whose width encodes a value, so the bar bounced past the true step and settled back. Now `var(--dur-5) var(--e-std)`, the treatment already correct at `:1204`. The fingerprint (§3) shows it as the only computed-value change in the whole swap, in both motion modes. |
| **T-019** | taste | n/a | **Not applicable, with evidence — not a pass.** `skeleton` matches 0 in `app.css`, `main.ts` and `index.html`; `spinner\|is-loading` matches 0. The only `placeholder` in the stylesheet is `.tb-search input::placeholder` (`app.css:88`), a text colour, and `#empty` has no rule in `app.css` at all — the empty state is `emptyEl.hidden` (`main.ts:1401`), which appears with no fade. There is no loading placeholder to anti-flicker, so the wait ~150 ms → fade in → min hold → cross-fade sequence has nothing to govern. It becomes live the day a skeleton is added. |
| **T-013** | taste | absent | **Confirmed absence, accepted.** `animation-delay\|transition-delay` matches 0 declarations. No stagger anywhere. If adopted later: total window ≤300 ms, first paint only, never on scroll-in, and only on rare surfaces (empty state, wizard, freshly-added row). The download list is a 100+/day surface and stays static. |
| **OPEN-3** | mechanical | `app.css` motion declarations | **`DECIDED` by the user, 2026-11-03, and executed the same day (§11).** The split: snap hover-linked / entrance-overlay / transient durations to the six rungs, keep the one-shot click flicks and the whole `.tabs` surface as literals — one exception class, two families in it — and take the hover family to `--dur-2` 120 ms. Executed against the shipped binary, not the source: **17** computed lines across 13 surfaces moved, 0 changed under reduced motion, 24 millisecond values left, all of them line-attributed in §3. Three of my own errors were caught by measurement and corrected in the record (a `180 → 160 ms` drift in the protected class, a site wrongly classed as protected when it has no animation at all, and a first count of the moved lines that read 16 — see §11); all are recorded rather than smoothed. Cost: **+512 B**, gate **47 PASS / 0 FAIL**. |

## 7. Frequency ceiling (T-009 / T-026) — what may move, where

| Surface | Frequency | Budget |
|---|---|---|
| Download list rows, `.tbar`, attempt dots | every refresh, many rows | state layer + width only, `var(--dur-2/5) var(--e-std)`, no entrance, no celebration |
| Hero wave + bonding diagram | continuous while a transfer runs | **the product story** — this is where motion is earned, and the only place it may loop. See O-003. |
| Inspector `.tabs` icon flick | one-shot per click | stays under reduced motion (§4) |
| Dialog / sheet / wizard | once per action | `--dur-4/5`; T-004 applies, no overshoot on a value-bearing bar |
| First-run wizard, empty state, freshly-added row | once per install / session | the only surfaces where a flourish or a short stagger is affordable |

Asymmetry to protect: **restraint on the list, one real flourish on finish.** A 100+/day list must
not celebrate 100 times a day, so completion motion (if any) belongs on the row leaving the active
set or on verification passing — rare, terminal, and readable as a conclusion rather than a loop.

## 8. What is *not* wrong

- **No input gating on animations** found; the wave loop self-terminates. `waveTick` bails on
  `ambientHeld` and nulls `waveRaf` (`main.ts:1033-1052`) as well as when the frontier settles, and
  `syncWaveLoop` prunes detached wave nodes before its early return (`main.ts:1054-1088`) so a
  `ResizeObserver` cannot strand a strong reference. Verified in source, then observed: the O-003 run
  in §6 shows the crest byte-identical across 900 ms while held, which is only possible if no frame
  is being requested.
- Every suppression block has a written *why*, including the two hard CSS facts
  (`!important` not revertable; durations collapse rather than disappear so `forwards` fill states
  still land). That is the rarest thing in a motion layer and it should be preserved verbatim when
  the surface list grows.

## 9. Sequencing — as executed, with measured costs

The user's order was obligation, then taste, then mechanical. That is what ran, each step gated and
then measured in the real window.

1. **O-003** (obligation) — shipped, §6.
2. **T-004** (taste) — shipped, §6. One declaration; the only computed-value change in the bundle.
3. **T-019 / T-013** (taste) — assessed and reported, §6. No code.
4. **`OPEN-1` + `OPEN-2`** with the `.tabs` exclusion — shipped, §3.
   **Cost measured, not estimated**, as the size of each link in the chain: 4,476,416 B before any of
   it, 4,477,440 B after steps 1-4 together (**+1,024 B**; the two were in one link, so the split
   between them is not isolated), and 4,477,952 B after the focus restore (**+512 B** more). The M0
   gate on the final binary is **PASS (exe 4.27 MB)** — **30,924 B** left under the 4,508,876 B
   ceiling — with zero FAIL across its 47 checks.
   This contradicts §9's draft prediction that tokenising would *shrink* the file — it did not,
   because `app.css` is esbuild-minified at `scripts/build.mjs:38-40`, where `180ms` and
   `var(--dur-3)` compress to nearly the same length, while `tokens.css` is copied **verbatim**
   (`:42-46`) and the two new curve tokens plus their comment are paid for in full. Only the
   comment-free part of the design file is free.
5. **`OPEN-3`** — not started; it is a `DECIDED`-class ruling and needs the user.

**Was owed, now executed — see §10.** Until the second pass this paragraph read:
`--wave-travel: 0s` under reduced motion was defeated at `main.ts:807` (`cssPx('--wave-travel', 1) || 1`);
`const WAVE` was read once at module load so a runtime change to the token would not be picked up;
`.add-preset-chip` (`app.css:898`) still animated `transition: all`; and `.nic-modal-dialog` /
`.wizard-modal-dialog` plus the other keyword curves kept `ease` / `ease-out`. All four are closed in
the shipped binary, each with the measurement in §10. The fifth item in that list — that converting a
keyword to a token is a value change and therefore OPEN-3's territory — is **the reason §10 changes
curves but not a single duration**, and `OPEN-3` is still exactly as open as it was.

Bytes remain the constraint, not a footnote: `app.css` and `tokens.css` are embedded in the exe, so a
motion edit is a binary edit. Every step above was gated (`npm run verify`, 47 checks) and then
measured over CDP in the launched window, because a harness number is not a shipped number.

## 10. Second pass — the four residuals, each closed with a measurement

Same day, same binary, zero bytes: the exe is **4,477,952 B before and after** (§9's headroom of
**30,924 B** is untouched), `npm run verify` **M0 GATE: PASS**, `npm run typecheck` clean. That is
not a coincidence to celebrate — `app.css` is minified, so `ease-out` → `var(--e-dec)` and a named
four-property list cost what they saved, and the `main.ts` edits are two small functions.

**What changed, and how each one is proven.**

1. **`--wave-travel: 0s` now stops the crest.** The `|| 1` at the old `main.ts:807` was a
   divide-by-zero guard for the phase divisor, and the effect was that the token could never win. Now
   `waveTravel()` (`main.ts:824-830`) returns `Math.max(0, cssPx('--wave-travel', 1))` and `drawWave`
   skips the phase instead of substituting a fallback speed (`main.ts:955-956`:
   `live && travel > 0 ? (t/1000) * (2π/travel) : 0`). A `0` is a real answer, not a missing value.
2. **The mode flip is live.** `WAVE` stayed a module-load read for geometry (`main.ts:806-817`); travel
   moved out of it precisely because it is the one value the motion switch rewrites. `applyMotion()`
   sets `waveTravelDirty` (`main.ts:83-92`) and is the only writer, so the cache cannot go stale, and
   it is not read per frame because a `getComputedStyle` in the rAF loop would flush styles.
3. **`.add-preset-chip` no longer uses `transition: all`** (`app.css:887`). It is now the four
   properties its `:hover` and `[aria-pressed]` rules actually change (`:905`, `:911`). The visible
   consequence is the one the shorthand was hiding: the focus ring used to fade in over `--dur-2`,
   which breaks the "appears instantly" rule the reduced-motion and focus layers both depend on.
   Measured in-window: computed `transition-property` is `background-color, color, border-color,
   box-shadow` and the ring is absent from it.
4. **No keyword curves left in the stylesheet.** Ten sites tokenised (`.nic-modal-dialog`,
   `.wizard-modal-dialog`, `.add-url-row input`, `.add-folder-input-wrap input`,
   `.add-folder-browse-btn`, `.context-menu-item`, `ctxAppear`, `toastFade`, `fadeIn` and the chip).
   Two are **deliberately kept**: `linear` on `wireShimmer` (a shimmer is linear or it pulses) and
   `ease-in-out` on `activeDotPulse` (an infinite breathing loop; a token curve would make it lurch).
   Built `dist/renderer/app.css` (68,698 B): `transition: all` **0**, bare `ease`/`ease-out` **0**,
   `cubic-bezier` literals **0**.

**Proof, run twice because a claim about motion is a claim about frames.** `Temp/sucr-wave-travel.mjs`
drives a real transfer against `test/chaos-server.mjs`, then samples the wave path's `d` attribute
twice 300 ms apart and compares **x-aligned** points only — the frontier marker is appended after the
step loop at a different `x` per sample, and comparing it index-for-index is what produced a first run
of bogus `maxDy` 0.07–0.93 that looked like a defect and was the probe's own bug.

| build | motion | `maxDy` across 300 ms | shared points | frontier advance |
|---|---|---|---|---|
| **pre-fix baseline** (4,476,416 B, `Temp/sucr-motion-old/sucr.exe`, 04:19 — before §9 steps 1-5, so the guard and the load-time read are both intact) | reduced | **4.280 / 4.400 / 4.180** | — | — |
| final | reduced | **0.000** | 59 / 244 / 270 | 36–43 px |
| final | full | **4.360** | — | — |

The baseline row is the whole finding: with `token=0s` **and** `attr=reduced` both set, its crest was
indistinguishable from full motion. The final row has the crest frozen while the frontier still moves
36–43 px in the same 300 ms, which is the distinction that matters — the progress position is the
transfer's state, the sine travel is decoration, and only the decoration is reduced.

**The A/B nobody asked for, and it is the better half of the proof:** in the same page, with **no
reload**, `Emulation.setEmulatedMedia` flips `prefers-reduced-motion` reduce → no-preference → reduce
and the crest goes `0.000 → 4.360 → 0.000`. That fires the app's own `matchMedia` listener
(`main.ts:101`), so it exercises `applyMotion` → `waveTravelDirty` → the next frame. It could not have
worked before item 2.

**Ruling reversed — needs your word, not mine.** The pre-fix `main.ts` carried a written
justification for the old behaviour — "Wave is an explicit in-app choice, so the crest keeps
travelling even when the OS asks for reduced motion" (that sentence was at `main.ts:1090` before this
pass; the replacement comment now sits at `:1109-1112`). Under the OS request the crest kept
travelling and only the app's own switch stopped it. I did not keep that. `html[data-motion]` is
documented as *the* answer to the collapse of `(OS, pref)` (§5), the token pins `--wave-travel` to
`0s` under **both** triggers (`tokens.css:267-268`, `:271-272`), and the old code made one of those two
triggers a lie. The counter-argument is real and I am not disposing of it: a user who picked "wave"
did ask for this surface, and the OS flag was never about the app's own meter — and note that this was
**a code comment, not one of your `DECIDED` entries**, so I am overriding my own predecessor's
reasoning, not one of your rulings. If the ruling is to restore it, the change is one condition in
`waveTravel()` (`main.ts:824-830`) and this paragraph becomes the evidence that it was decided once,
deliberately, in the other direction. Nothing in §4's `.tabs` carve-out is involved either way.

**What the pass does not touch.** No duration moved. `OPEN-3`'s residue is still exactly **30**
off-scale literals (`100×1, 140×5, 150×1, 160×4, 200×4, 240×5, 280×4, 300×2, 320×3, 340×1`) plus the
`.tabs`-owned `180×6`/`260×4`; `--dur-6` and `--e-acc` are still dead. Converting a keyword curve to a
token changes a value, and §9 said value changes belong to `OPEN-3` — that sentence still governs, so
this pass spent its curve budget on curves that were already token-shaped and left every millisecond
number for the ruling only the user can make.

*That sentence is the last true statement of §10 and is now superseded: the user ruled on `OPEN-3` the
same day and §11 executed it. The "exactly 30" was one of the loose counts §3 re-measures; the residue
that pass can vouch for is the line-attributed enumeration in §3 and the binary diff in §11, not a
grep total.*

## 11. `OPEN-3` — the split ruling, as executed

**Ruling (`DECIDED`, the user, 2026-11-03), three parts.** Snap the hover-linked, entrance/overlay and
transient families to the existing rungs; record the toolbar/theme one-shot click flicks **and** the
`.tabs` literals as the single exception class that keeps its milliseconds; take the hover family to
`--dur-2` 120 ms.

**What moved — 17 computed lines across 13 surfaces, read out of the shipped binary itself.**
`Temp/sucr-motion-fp2.mjs` captures the whole computed-motion inventory; `Temp/sucr-fp-diff.cjs` diffs
it. Pre-batch (`fp-open3-before.json`, the 4,477,952 B binary preserved as `sucr.exe.pre-open3`) against
shipped (`fp-open3-final.json`, 4,478,464 B), de-duplicated computed lines, `full` mode: **72 unique
lines in both, 17 changed, 55 identical** — and every one of the 17 is an intended surface. Thirteen
rows below, because a row is a surface and a surface can be several lines: the two modal dialogs are 2
of the 17, the four wizard steps are 4, the two context menus are 2, and `.net-card-bar-fill` is none
(separate probe, below). **This count said 16 until the binary was relaunched and recounted — the third
error in this section, and the only one that was in the record rather than in the build.**

| surface | before | after | rung |
|---|---|---|---|
| `.add-url-row input` ×2 props | `0.14s, 0.14s` | `0.12s, 0.12s` | `--dur-2` |
| `.add-folder-input-wrap input` | `0.14s` | `0.12s` | `--dur-2` |
| `.settings-nav button svg` (5 instances, anonymous key) | `0.16s` | `0.12s` | `--dur-2` |
| `.btn-redownload` transform (4-prop line) | `0.12s, 0.12s, **0.16s**, 0.12s` | `0.12s ×4` | `--dur-2` |
| `.nic-state i` | `0.12s, 0.2s` | `0.12s, 0.22s` | `--dur-4` |
| `.nic-row .dot` | `0.24s, 0.24s, 0.2s` | `0.26s, 0.26s, 0.12s` | `--dur-5 ×2`, `--dur-2` |
| `.nic-modal-dialog` / `.wizard-modal-dialog` | `0.28s, 0.2s` | `0.26s, 0.22s` | `--dur-5`, `--dur-4` |
| `.wizard-step` ×4 | `0.2s` | `0.22s` | `--dur-4` |
| `.net-card-bar-fill` | `240ms` (source, not captured) | `0.26s` (separate probe) | `--dur-5` |
| `#add-bar` `addBarSlide` | `0.16s` | `0.18s` | `--dur-3` |
| `#toast-notice` `toastFade` | `0.16s` | `0.18s` | `--dur-3` |
| `#context-menu` + sub-menu `ctxAppear` | `0.1s` | `0.12s` | `--dur-2` |
| toggle thumb `SPAN::after` | `0.24s, 0.12s, 0.15s` | `0.26s, 0.12s, 0.12s` | `--dur-5`, `--dur-2` |

**And the reduced capture changed by nothing.** Same diff in `reduced`: **1366 unique lines before,
1366 after, 0 removed, 0 added.** That is the proof the batch is safe rather than merely small — every
snapped surface still resolves to the blanket's `1ms !important`, so no reduced-motion surface can
regress when a token is swapped. (A first run of this diff appeared to show 3 changed lines; they were
the same element captured with and without the `is-open` class, values identical — a capture-state
artifact, not a motion change. The final run has no such pair.)

**Re-captured from a fresh launch of the shipped binary, later the same day.** `Temp/sucr-motion-fp2.mjs`
run again against `target/release/sucr.exe` on a throwaway data dir (`Temp/fp-open3-relaunch.json`,
`Temp/sucr-verify-relaunch.cjs`): `full` mode **0 removed / 0 added** against the recorded capture, and
the inventory hash reproduces byte-for-byte (`26357adaa39ab025`, 193 nodes) — the binary is stable
across launches, so the record describes what a relaunch actually shows, not what one lucky run showed.
`reduced` **0 removed / 0 added** against *both* the recorded capture and the pre-`OPEN-3` binary
(1366 unique lines all three ways; raw node counts moved 3253 → 3241 purely because the download had
progressed differently, which is why the comparison is de-duplicated lines and not a hash). Two surfaces
in that reduced capture are worth naming, because together they are §4's mechanism observed rather than
argued: `#nic-modal-dialog`, `#add-bar` and `#toast-notice` compute `0.001s` — their new `--dur-4`/
`--dur-5`/`--dur-3` values collapse exactly as the old literals did — while `#tab-pill` still computes
`0.18s, 0.18s`, untouched by the blanket, in the same window at the same moment. That recount is where
the 16-vs-17 came from.

**The one surface the fingerprint could not reach, closed separately.** `.net-card-bar-fill` was in
neither capture because both ran with `state.cards = 0`. `Temp/sucr-netcard-read.mjs` closes it three
ways in the shipped window: the embedded rule text is
`transition: width var(--dur-5) var(--e-std)` (token, not literal); under this box's default reduced
state the element computes `0.001s` while the root's `--dur-5` reads `0ms` (the §4 mechanism, measured);
and after flipping `ui.motion` to `full` **in the live window, no reload**, a real rendered card reads
`transition-duration: 0.26s` on `width` with `cubic-bezier(0.2, 0, 0, 1)`.

**Three errors of mine, all found by measurement — the first two inside the execution pass, the third
only when the shipped binary was relaunched and recounted. Recorded because the alternative is a file
that describes a build that does not exist.**

1. **I mis-reverted the two flick settles.** When the snap swept `.tb-mode svg` and `.theme-seg button
   svg` to `--dur-2`, the revert retyped them as `160ms` — but their pre-batch value was **180 ms**, and
   the before-capture said so (`tbModeSvg` / `themeSegSvg` `0.18s` before, `0.16s` in the interim
   binary). "Leave the exception class exactly as it is" was the ruling, so a −20 ms drift in it is a
   violation even when it is a revert. Restored to `180ms`, re-linked, re-captured: both read `0.18s`
   again and are absent from the before→final diff.
2. **`.btn-redownload` was wrongly put in the exception class at all.** The reason I gave for keeping
   the two settles is that their base `transform` transition is the settle half of a spring pair, and
   the discriminator is in the keyframes: `modeFlipSpring` ends at `rotate(180deg)` and `sunSpinFlare`
   at `rotate(90deg)` — a resting transform *different* from the base, so the base duration is part of
   the composite gesture. `.btn-redownload` has **no animation of any kind**; its `transform 160ms`
   governs only `:hover { transform: rotate(45deg) }` (`app.css:1168-1172`), which makes it a plain
   hover-linked transform, family (1), and the ruling says `--dur-2`. It is now
   `transform var(--dur-2) var(--e-std)`, the fourth property in that declaration matching the other
   three, and measured: `0.12s ×4`.
   The same discriminator says the *other* three flicks do **not** constrain their neighbours:
   `moonRock`, `autoPulse` and `navPillPop` all end at identity, so a base transition on the same
   element is independent of them — which is why `.settings-nav button svg` was free to snap to
   `--dur-2` while `navPillPop 280ms` stayed a literal.
3. **The moved-line count in this section was 16; it is 17.** Every surface in the ruling's three
   families did move, so the build was never wrong — the record under-counted by one line, and the
   sentence that carried it was in §4, §6 and the header as well, which is how a single slip becomes
   three. Recounting required launching the shipped binary again: the de-duplicated diff is a set
   difference over computed lines, and a line is not a surface (2 dialogs + 4 wizard steps + 2 context
   menus = 8 lines across 3 rows, plus the `.settings-nav button svg` anonymous-key line).
   **Lesson kept: count from the diff output, never from the table rows that summarise it.**

**What the ruling left as literals — the exception class, measured at 22 values.** `.tabs` owns 15:
`.tab-pill 180×2`, `.tab-icon 140×2`, `tabTap 180`, `.icon-speedo .needle-group 180` (transition) and
its `needleGlide 320 forwards`, `suckerBlockRipple 240×2`, `suckerDotPulse 260`, `linkSlideL/R 260×2
forwards`, `clockQuarterSweep 300 forwards`, `folderDocLift 260`, `ekgCleanTrace 280 forwards`. The
flicks own 7: `modeFlipSpring 320`, `sunSpinFlare 320`, `moonRock 340`, `autoPulse 300`,
`navPillPop 280`, and the two settle halves `180 ×2`. Plus the two `[role="tabpanel"] 180ms` slides
that belong to neither class and are still open (§4). `app.css` now holds **24** millisecond values in
`transition:`/`animation:` declarations, **28** with the four `1ms` of the reduced block, against the
40 (44) §3 recorded before the batch with the looser pattern.

**Cost, as always measured on the real file.** `sucr.exe` 4,477,952 → **4,478,464 B (+512 B)**,
headroom to the 4,508,876 B ceiling **30,412 B**; `dist/renderer/app.css` 68,698 → 68,848 B (+150 B),
`tokens.css` untouched at 13,734 B — the batch spent no design-file bytes, which is why the exe moved
in one 512-byte section step and not more. `npm run verify`: **M0 GATE PASS (exe 4.27 MB), 47 PASS / 0
FAIL**; `npm run typecheck` clean. The two corrections (revert-restore, redownload snap) together cost
**0 B**: the final binary is the same 4,478,464 B as the interim one, because minified `180ms`/`160ms`
and `var(--dur-2)` all land on the same byte length in `app.css`.

**Not done, named so it is not mistaken for done.** `.toggle-switch span::after` now animates `width`
through a token — the transition itself is the Hallmark-floor violation (never animate `width`), and
this batch changed its duration, not that sin; `--dur-6` and `--e-acc` are still dead, and folding the
five spring/flick values up to a rung (`--dur-6` 400 ms or a new rung) remains available as its own
ruling; the `tokens.css:193` "chips, map cell" comment is stale (§2) and editing it costs binary bytes
because `tokens.css` is copied verbatim; and the `[role="tabpanel"]` pair awaits one word (§4).

## 12. The compact rail reveal is now Zen's slide, on Zen's sizes (2026-11-04)

The user re-pasted `zen_tQLCdc9PeH.mp4` and ruled: *"fix the hover animation, and make it animate the same way
Zen browser does, and use the same side bar size of Zen browser."* That is the first motion call in this file
taken from a named external product's **source**, not from a look. Sparse clone at
`D:\RE\_coldstart-harness\zen-src` (`zen-browser/desktop` @ `dev`), and every number below is a line in it:

- **Departure** — `src/zen/compact-mode/sidebar.inc.css:84` `translate 0.15s ease` (+ `visibility 0.15s ease`).
- **Arrival** — `:275` `--zen-compact-mode-time: 0.25s`, `:276` `transition: translate var(--time) var(--func)`,
  where `--zen-compact-mode-func` (`:172`) is a 101-stop `linear()` table that peaks at `1.010904 73%` (`:246`)
  and settles at `1.003423 100%` (`:273`). It overshoots its own edge by ~1 %, then comes back.
- **Content does not move or fade** — `:279-282` sets `#titlebar { transition: none; visibility: visible }`
  inside the same hover block, and `HOVER_HACK_DELAY` (`ZenCompactMode.mjs:75`) defaults to 0 ms.
- **Sizes** — `ZenCustomizableUI.sys.mjs:60` `kDefaultSidebarWidth = 186px` (non-macOS); compact keeps
  `--zen-toolbox-max-width: 62px` (`sidebar.inc.css:50`) and `--zen-element-separation: 8px`, so ~9 px of it
  stays inside the window. `186px` and `9px` are what this build now uses.

**What was wrong.** Sucr's reveal animated `clip-path: inset(0 Npx 0 0) → inset(0)`. A clip is an unmask: the
nav sat still while its own right edge travelled across it, and the panel's border and shadow never arrived
because they are outside the clip. Zen moves the panel. Replaced with the independent `translate` property
(`app.css:326-354`): parked at `calc(var(--rail-strip) - var(--rail-open))` = −177 px, released to `0`, with
`--dur-rail-out 150ms ease` and `--dur-rail-in 250ms var(--e-rail-in)`. `--e-rail-in` is Zen's 101-stop table
sampled every 10 % — same shape, 11 stops, 40 fewer tokens.

**Measured in the shipped exe**, `D:\RE\_cap-harness\rail-hover.mjs`, two passes, real `Input.dispatchMouseEvent`
hover with `.rail:hover` confirmed `true` at the peak of the move, motion forced to `full` because this box's OS
reports `prefers-reduced-motion`:

| | pass 1 | pass 2 |
|---|---|---|
| IN −177 → 0, peak past the edge | +1.80 px @ 185 ms | +1.85 px @ 160 ms |
| IN last frame still moving | 238.4 ms | 226.4 ms |
| IN at rest at 0 by | 251.7 ms | 239.8 ms |
| OUT 0 → −177, last frame moving | 120.2 ms | 120.1 ms |
| OUT parked at −177 by | 146.7 ms | 146.7 ms |

1.83 px is 1.04 % of the 177 px travel and 0.98 % of the 186 px panel — Zen's own `1.010904`. The arrival runs
240-252 ms against a 250 ms token and the departure 146.7 ms against 150 ms. Hit test on a row inside the open
panel returns `nav-text`, `clickable: true`; the middle grid column reads `x = 9` parked **and** open, so the
overlay reflows nothing, which is the whole point of Zen's `#titlebar { transition: none }`.
`D:\RE\_cap-harness\rail-verify.mjs` covers the rest: at rest `--rail-w: 9px`, shell `9px 879px 392px`,
`clip-path: none`, and under `data-motion="reduced"` the transition property is `none` while `translate` still
reaches `0px` two frames after entry — **the reveal keeps working, it stops sliding.**

**Cost.** `tokens.css` 13,734 → 14,823 B (+1,089 B, and it is copied verbatim into the binary), `app.css`
94,177 B source → 69,127 B minified. `sucr.exe` 4,507,136 → **4,508,160 B (+1,024 B)**, headroom to the
4,508,876 B ceiling **716 B**. `npm run typecheck` clean; `node scripts/verify-m0.mjs --smoke` **M0 GATE: PASS
(exe 4.30 MB)**, 48 checks.

**Two facts named, not fixed.** `tauri.conf.json:16` sets `minWidth: 1024`, so the `@media (max-width: 900px)`
icon-rail branch cannot be reached by resizing and the new `--rail-icons: 60px` token (Zen's collapsed rail:
`--tab-min-width` 48 + `--zen-toolbox-padding` 6 × 2) is inert until that floor moves — it is in the token file
so the number is recorded where a ruling can pick it up. And the 9 px strip is the hover target: at 0 px there
is nothing to enter, which is why Zen keeps `--zen-element-separation` of it inside the window too.

## 13. The delay is on the departure, not the entrance (2026-11-04, corrected the same hour)

**The asks, in order.** "The animation still needs to be bit delayed. Check the video again." → "Need to delay the
slide even more." → **"This should've been the opposite of IN dwell."** The third is the correction, and the first
two explain why it was missed: the delay was read off the wrong edge of the gesture, and then tuned harder in the
same wrong direction.

**What was wrong, stated plainly.** `D:\RE\_coldstart-harness\zen2-delay.mjs` streams rgb24 from
`zen_tQLCdc9PeH.mp4` (1920x1032, exactly 30 fps, 403 frames) cropped to the left 300 px, reporting per frame the
panel edge (sidebar fill luma 28-29 against a page at 15, so the rightmost column above 22 is the edge) and the
cursor (near-white pixels, luma > 235, at x < 60). In all four open gestures the pointer is already at x~5 for one
to two of the recording's 33 ms frames before the edge creeps, which looked like a 33-67 ms entrance dwell. It
measures nothing: **at 30 fps a delay of 0 ms and a delay of 60 ms are the same pixel series**, because the pointer
arrives part-way through a frame. The video could only ever bound a delay from above at ~67 ms and could not say
*which* edge it sat on. Zen's source said all along, and the note was already in §12's research:
`HOVER_HACK_DELAY` (pref `zen.view.compact.hover-hack-delay`) is **0** for the entrance.

**The number that was actually there.** `prefs/zen/compact-mode.yaml` in zen-browser/desktop @ dev:

```
zen.view.compact.sidebar-keep-hover.duration:      150   # the sidebar holds, then collapses
zen.view.compact.hover-hack-delay:                   0   # the entrance waits for nothing
zen.view.compact.toolbar-hide-after-hover.duration: 1000 # the toolbar, not the sidebar
```

and the code matches it: the `mouseleave` path (`ZenCompactMode.mjs:799-852`) collapses through
`flashElement(target, keepHoverDuration, ...)` when the element has one, and `flashElement` (`:643-655`) clears the
pending timeout if the pointer comes back before it fires. So the behaviour to copy is: **reveal on contact, hold
150 ms after the pointer leaves, then slide out.**

**Shipped.** `--dur-rail-hold: 150ms` (was `--dur-rail-armed`) in `design/tokens.css`, declared on the **base** rule
of `.rail-scroll` (`app.css:345-350`) and on nothing in the `:hover`/`:focus-within` branch. That placement is the
mechanism rather than a style choice: a transition takes the properties of the state it is travelling *to*, so a
delay on the base rule holds the departure and a delay on the hover rule holds the arrival. Reduced motion is
unaffected either way — `app.css:3136` `transition: none` resets the longhand.

**Measured in the shipped exe**, `D:\RE\_cap-harness\rail-delay.mjs`, three rotated passes, both timestamps from
the page's own `performance.now()` (`Input.dispatchMouseEvent` raises a real `mousemove` in the renderer, so
pointer-arrives and `transitionstart` share one clock and the CDP round-trip sits outside the measurement):

| | pass 1 | pass 2 | pass 3 |
|---|---|---|---|
| IN dwell, mousemove → transitionstart | 10.4 ms | 14.7 ms | 2.3 ms |
| OUT dwell | 154.4 ms | 155.1 ms | 155.7 ms |

IN is one frame at this box's 75 Hz: the panel answers on contact. OUT is the 150 ms token plus a frame.
`transition-delay` now reads `0.15s` **parked** (the base rule owns it) and `0s` while arriving. The slide is
unchanged from §12: −177 → 0, peak +1.85 px, last frame moving 213-227 ms, and the departure still covers its own
150 ms of travel within 120 ms of starting.

**Cost: 0 B across both corrections.** The entrance-delay build and this departure-hold build are both
**4,508,672 B**, headroom to the 4,508,876 B ceiling **204 B**. The 512 B was spent once, on the new
`transition-delay` *declaration* in the minified `app.css`; moving that declaration between rules and renaming the
token it references is free. `npm run typecheck` clean; `node scripts/verify-m0.mjs --smoke` **M0 GATE: PASS
(exe 4.30 MB)**.

**The rule kept from this:** a 30 fps screen recording cannot measure a delay shorter than its own frame period and
cannot attribute a dwell to an edge of the gesture. When the reference is a video but the reference *app's* source is
readable, the pref default is the evidence and the video only proves the behaviour exists at all.

## 14. The rail holds: choosing a section docks it (2026-11-10)

**The ask.** `Qoder_nXwfl309wO.mp4` plus "I need the Sucr sidebar animation to be held like this. Based on Zen
browser." The reference is not a curve to copy, it is a *state*: the sidebar stays out.

**What the recording actually contains.** Re-measured 2026-11-10 with `scratch/vid.mjs`, which did not exist when this
section was written. The clip is **Qoder Desktop** (1920x1032, 30 fps, 14.47 s), not Zen. Its sidebar ends at a 1 px
divider at x=252; a dark gutter runs to a second divider at x=319 where the content pane begins. Both sit at the same
pixel in every 1 s sample from 0 s to 13 s. A transition would move that divider; a held panel keeps it. So the video
proves "held", and nothing about timing.

**Retracted (2026-11-10).** This section first claimed the first bright column "sits at x=42 ... and x=59 ... in every
frame, no moving edge anywhere in the clip", measured over the left 700 px. That was wrong twice over: the region
includes the embedded browser pane, which moves constantly (per-frame difference peaks at 5.2 around t=2.85 s and 8.9
at t=9.77 s, both inside that pane), and the probe never reached the sidebar's real edge at x=252. The evidence for
"held" is the constant divider, not frame-identical pixels. §15 carries the clip's real hover measurement.

**Zen's model, from its source** (`D:\RE\_coldstart-harness\zen-src`). Two independent reasons for the sidebar to be
open: `zen-has-hover` (transient -- the pointer is on the strip) and `zen-user-show` (persistent -- the user asked for
it, `toggleSidebar()` flips it). `isSidebarPotentiallyOpen()` is their union, and the CSS expands on both. Hover is
the peek; `zen-user-show` is the hold. That is the distinction Sucr was missing: it had the peek only.

**The decision, and the trigger.** A nav row is the point where the user has committed to the nav, so choosing a
section calls `holdRail()` -> `setRailCompact(false)` (main.ts, both the `pointerdown` handler and the keyboard/`click`
fallback). Hover stays exactly as §12/§13 left it: reveal on contact, Zen's 150 ms keep-hover, retreat. The Sidebar
button and Alt+1 still toggle, and are the way back to compact. Rationale: a hover reveal that retracts the moment
the pointer leaves is wrong for the second click -- the panel retreats mid-reach and the row just chosen is gone.

**The mechanism, one number.** `--rail-w` drives both the grid column (`grid-template-columns`) and the panel's
`translate: calc(var(--rail-w) - var(--rail-open))`, so the panel edge and the list's edge can never disagree.
Docking is *not* a second animation: the panel is already at `translate: 0`, the column grows 9 -> 186 with the
arrival curve, and the list slides out from behind the panel's right edge on the same curve -- one motion, not a cut
followed by a slide. The hold delay for the departure stays on `body.rail-compact .rail-scroll` and
`body.rail-compact .shell` only, because a transition takes the properties of the state it is travelling *to*; the
dock and the reveal therefore answer on the frame they are asked for.

**Measured** in the running Dev binary (`target/debug/sucr.exe`, CDP 9336, 40 ms state sampler, real OS cursor
driven with `SetCursorPos`/`mouse_event`):

| leg | observation |
|---|---|
| hover the 9 px strip | panel 0, peak overshoot +1.64 px, column unchanged |
| click "Queued" (80,155) | `compact=false`; column 46.6 -> 187.6 -> 186 while the panel stays `translate: 0` |
| mid-dock, 201 ms | panel edge -70.5 + 186 = 115.5 == `col1` 115.5 -- panel and list in lockstep |
| pointer leaves to (1020,521) | **held** 1.4 s later: `compact=false`, `translate: 0`, `col1: 186px`, `view=Queued`, `aria-pressed=false` |
| Sidebar button | `compact=true`, ~160 ms hold, then panel 0 -> -177 while the column 186 -> 9 collapses on the same frames |

Screenshot of the docked state: `scratch/rail-open.png` (LIBRARY / CATEGORIES / SYSTEM visible, "Queued" current,
`columns "186px 702px 392px"`, hit test at (60,120) is `nav-text`).

**A harness trap, recorded.** CDP `Input.dispatchMouseEvent` does not hold `:hover` in this WebView2 window -- the
synthetic pointer sits inside the panel while the page reports `:hover` false and the panel retracts. Any hover leg
here must drive the real cursor; samples were taken with `scratch/drive-real.mjs` + `scratch/drive-rail.ps1`.

**Cost: unmeasured by ruling.** The user's standing instruction is to keep all changes in the Dev build and cut the
Alpha by hand later, so neither the release exe nor the installer was rebuilt for this slice. `target/release/sucr.exe`
still measures 4,758,528 B and the installed 0.1.0 exe 4,757,504 B -- both untouched. The change itself is CSS
declarations plus one small function and two call sites in `main.ts`; the byte price will be taken with `stat` at
the Alpha build, not inferred here.

**Hover, re-measured after the dock landed (2026-11-10, same build).** Three full peek-retreat cycles were driven
with the real cursor: away -> strip (324,421) -> into the panel body (400,301) -> away (1020,521), twice, then a final
leave to (900,760).

| leg | observation |
|---|---|
| hover the strip | reveal starts on the next sample, overshoots +1.73 px and +1.92 px in the two passes, settles at `translate: 0` |
| pointer inside the panel body | stays fully open (`x=0`, column still 9 px); the row under the pointer takes its own hover fill |
| pointer leaves | `:hover` false at once, panel holds at 0 for ~160 ms, then retreats -177 over ~160 ms |
| hover frames overall | 176 samples with `:hover` true, `compact` **true in all of them** |

The last row is the point of the leg: hovering never docks the rail. The hold was reserved for a nav choice, so a peek
could not leave the window permanently wider.

**Superseded 2026-11-10 — see §16.** The nav-click dock is gone: the user's clips ask for reveal-on-contact and
collapse-on-leave with no session pin, and a nav row that stayed focused was pinning the panel a second way.

Captures: `scratch/h-1-strip.png` (panel out, "All downloads" current), `scratch/h-2-inside.png` (pointer on the
Finished row, panel still out), `scratch/h-3-leave.png` (back to the 9 px strip, list reflowed to x=9).

## 15. The reference's actual hover: a hard fill and one colour fade (2026-11-10)

The only hover in `Qoder_nXwfl309wO.mp4` is a task-row hover ("Install openclaude to h drive", x 8-252, y 634-661).
It is not a rail expand (§14) and it is not timed. Measured frame by frame at 30 fps with `scratch/vid.mjs` plus raw
gray-frame probes over a text-free band of the row (y 636-640) and the label's 99th-percentile luma:

| property | hover in | hover out |
|---|---|---|
| row fill, luma 16 -> 62 | one frame: 0% at 4.833 s, 100% at 4.867 s | one frame: 62 at 6.980 s, 16 at 7.013 s |
| `...` action button, mean 16 -> 68 | same frame as the fill | same frame as the fill |
| label p99, 141 -> 242 | 7% 4.880 · 70% 4.913 · 92% 4.947 · 100% 4.980 | 75% 7.047 · 4% 7.080 · 0% 7.113 |

Nothing about the fill is animated: it appears and disappears inside one 33 ms frame, with no intermediate value in
either direction, and the action button rides with it. The label's colour is the only animated property -- about
**150 ms cubic-out** in (cubic-out predicts 73% at 53 ms and 93% at 87 ms; measured 70% and 92%) and about
**100 ms cubic-out** out (predicts 71% at 34 ms and 96% at 67 ms; measured 75% and 96%). No transform, no shadow, no
width change: the pill's left and right edges are x=8 and x=252 in every frame of the transition.

Sucr's `.nav-item` animates `background` over `--dur-2` = 120 ms, `--e-std` = `cubic-bezier(.2, 0, 0, 1)` (app.css:264,
tokens.css:197/203), so the reference is the *less* animated of the two: it hard-cuts the fill and fades only the text.
Matching it would mean removing the rail rows' fill transition, not adding one.

Artifacts: `scratch/vid-row-zoom2.png` (hovered row, 4x), `scratch/vid-rowtx-tile.png` (six consecutive frames across
hover-in), `scratch/vid-right-4.767.png` (the resize cursor on the divider, one frame before the fill).

## 16. The dock is withdrawn: the rail collapses on leave again (2026-11-10)

**The ask.** Three clips and: "The hover still stays on the left even after cursor is not on it ... I need it to
collapse like how it did in 2nd video attachment. I need you to preserve the animation that's in 3rd video attachment."
§14 is what the user is rejecting: it made a nav choice dock the rail open for the rest of the session.

**What each clip is.** ~~`explorer_CYn9Sap5iK.mp4` (1920x1080, 23.7 s) is Windows File Explorer with the details pane
opening and closing; it shows no rail and nothing to copy.~~ **Retracted in §17**: it is a Zen window on GitHub and it
does carry the tab-strip sidebar, mostly collapsed. `zen_dW5MrW3s1W.mp4` (1920x1032, 11.9 s) is a Zen window on
GitHub: while the pointer is inside the sidebar it is 190 px wide (t=4.95 s, arrow at ~(100,377); t=9.5 s, arrow at
~(75,422)), and once the pointer is over the page it is gone and the content fills the window from x=0 (t=10.8 s,
11.5 s, 11.8 s). That is reveal-on-contact and collapse-on-leave, with the content reflowing into the freed width.
`sucr_zfngs1JmXV.mp4` (1920x1032, 6.2 s) is the defect: the rail is compact for ~0.75 s, then stays 186 px wide for
the remaining 5.4 s while the user clicks through All downloads, Downloading, Queued, Finished, Error.

**The cause was two pins, not one.** `holdRail()` called `setRailCompact(false)` from both the `pointerdown` and the
`click` fallback, which removed `body.rail-compact` — and the whole reveal/collapse block lives under that class, so
the rail could never return. Fixing only that left a second pin: a nav row is a `<button>`, so the click leaves it
focused, and `body.rail-compact .rail:focus-within > .rail-scroll` kept the panel at `translate: 0` with the pointer
long gone. The rule is now `:has(:focus-visible)`, which is false for a pointer-activated button and true for Tab.

**What changed.** `src/renderer/main.ts`: `holdRail()` and its two call sites deleted. `src/renderer/app.css:369-378`:
the focus half of the reveal selector is `:has(:focus-visible)`. Nothing in the animation moved — `--dur-rail-in`
250 ms `--e-rail-in` in, `--dur-rail-hold` 150 ms then `--dur-rail-out` 150 ms out — which is the "preserve the
animation" half of the ask.

**Measured in the running Dev binary** (`target/debug/sucr.exe`, CDP 9336, 40 ms sampler, real OS cursor via
`scratch/hover-stage.ps1`; harness `scratch/rail-collapse.mjs`, 1139 frames):

| leg | observation |
|---|---|
| at rest | `compact=true`, `translate: -177px`, column `9px` |
| hover the strip | reveal over ~240 ms, overshoot +1.9 px at 3800 ms, settle `0` |
| click "Queued" with the pointer still on the rail | `compact=true` throughout, `view=Queued`, focus on the row |
| pointer leaves to (1020,521) | holds at `0` for ~150 ms, then -8.8 -> -177 over ~120 ms; focus still on Queued |
| re-hover, then leave | same reveal and same retreat |
| keyboard | first Tab lands in the rail with `:focus-visible` true and the panel still reveals |

Keyboard focus keeps the rail out while the user is in it; that is the one intended difference from a pure hover
state. Note the harness trap in §14 still applies: synthetic `Input.dispatchMouseEvent` does not hold `:hover` in this
WebView2 window, so every hover leg above used the real cursor.

**Cost.** Renderer-only: debug exe unchanged in size. The release exe and the installer were **not** rebuilt, per the
standing instruction to keep all changes in the Dev build until the Alpha is cut by hand; the byte price is taken at
that build with `stat`.

## 17. The last pin was focus, not hover: a keyboard-only reveal latch (2026-11-10)

**Retraction first.** §16 says the first clip "shows no rail and nothing to copy". That is wrong, and the user said so
twice ("Check the first video properly"). `explorer_CYn9Sap5iK.mp4` is a Zen window on
`github.com/benjaminspoop7-sys/sucr` whose left tab strip is mostly collapsed. Sampling the strip's width off raw
frames (flat-column variance, band y 200-900, 10 fps) gives ~7-16 px for most of the 23.7 s, with brief exposures at
2.6 s (~178 px), 9.4 s (76), 15.8 s (103), 17.8 s (141) and 21.3 s (76), and the collapses land at 2.7 / 10.7 / 15.9 /
18.5 / 18.7 s. The capture draws no cursor, so whether those exposures are hover or toggling cannot be decided from
pixels — but the clip is a rail that stays narrow and returns, which is exactly the shape the user asked for. The
§16 reading came from a downscaled contact sheet; a full-frame sheet at 640 px per tile is what showed the strip.

**The surviving pin.** After §16 the mouse paths all collapsed, yet the panel could still open with the pointer far
away: a nav row left with `:focus-visible` keeps `body.rail-compact .rail:has(:focus-visible) > .rail-scroll` at
`translate: 0`. Reproduced by Tabbing into the rail (`scratch/rail-hover2.mjs` leg E ended at `t=0px`, `fv=true`,
`hover=false`) and found live: `{"stored":"true","compact":true,"active":"BUTTON#All downloads","fv":true,
"translate":"0px","hover":false}`. A keyboard user pays for that rule; a mouse user must never be caught by it, because
a click on a `<button>` can leave `:focus-visible` behind through script focus, a browser find, or a Tab that happened
minutes ago. The old note that a pointer-activated button never matches `:focus-visible` is true of the click itself
and false of the state it leaves behind.

**The latch.** `body.rail-kbd` gates the focus half of the reveal. `main.ts` adds the class on `keydown` of Tab and
removes it on the first `pointerdown` or `pointermove` (both captured on `document`), so the reveal is armed only while
the keyboard is the last thing the user touched. The same listener now also carries Alt+1, which was previously a
separate handler. `app.css` compact block:

```css
body.rail-compact .rail:hover > .rail-scroll,
body.rail-compact.rail-kbd .rail:has(:focus-visible) > .rail-scroll { translate: 0; transition: translate var(--dur-rail-in) var(--e-rail-in); }
```

Nothing in the timeline moved: 250 ms `--e-rail-in` in (overshoot measured at +1.7 px), `--dur-rail-hold` 150 ms then
150 ms `ease` out, `.shell` column animated on the same frames.

**Measured in the running Dev binary** (`target/debug/sucr.exe`, CDP 9336, real OS cursor and real clicks; harnesses
`scratch/rail-hover2.mjs`, `scratch/rail-kbd.mjs`, `scratch/rail-pin.mjs`, `scratch/rail-leave-window.mjs`):

| leg | end state |
|---|---|
| click a nav row, then leave | `t:-177px`, `compact:true`, `hover:false`, `fv:false` |
| click empty panel space, then leave | `t:-177px`, `compact:true`, `fv:false` |
| flick out of the rail | `t:-177px` |
| leave via the titlebar | `t:-177px` |
| Tab into the rail, pointer parked at (1330,521) | `kbd:true`, `t:0px` — reveal |
| move the pointer anywhere afterwards | `kbd:false`, `t:-177px` while the row keeps `fv:true` |
| toggle the rail (Sidebar button) | `compact:false`, stays `t:0px` with the pointer away; toggling back returns `-177px` |
| Alt+1 | same two-way toggle, same stored key |
| reveal, then leave the whole window to the desktop | `t:-177px`, `hover:false`, `body:hover` false |

Screenshots (`scratch/rh-at-rest.png` … `rh-gone.png`): at rest and after leaving the tiles are byte-identical; the
+80 ms tile still shows the panel open (the 150 ms hold), and the +230 ms tile is back to the strip.

**Cost.** `app.css` + `main.ts` only; no Rust, no IPC, no new asset. The release exe and installer were **not** rebuilt
per the standing rule; the byte price is taken with `stat` when the Alpha is cut.
