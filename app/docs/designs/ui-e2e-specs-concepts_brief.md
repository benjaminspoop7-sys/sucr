# UI End-to-End Specs for the Sucr Renderer

* **Author:** Benjamin (machine account; `git config user.name` is unset on this box)
* **Date:** 2026-11-03

> Written late. This brief was reported as saved in an earlier turn and it had not been written; the file did not
> exist on disk until now. Everything below is reconstructed from the six-lens review that was actually run, and
> every claim here was re-checked against the tree before it was typed.

## Overview

Give the renderer a way to be tested from the outside, and give the tests that already exist a way to be chosen,
run one at a time, and trusted.

The premise this feature started from — "the app has zero renderer coverage, so write e2e specs" — is false, and
discovering that is most of the finding. There **is** end-to-end coverage, and it is unusually real: `npm run
verify` launches the **release** exe with `--smoke`, the renderer runs a scripted pass over its own live UI, and
the script makes **47** `check()` assertions in [scripts/verify-m0.mjs](../../../app/scripts/verify-m0.mjs) —
clicking actual controls, dispatching real `KeyboardEvent`s, reading `.hidden`, `.disabled`, `aria-selected` and
`getComputedStyle().transitionDuration`, then independently `statSync`ing folders on disk and opening `sucr.db`
itself. What is missing is not coverage. It is the ability to address UI elements reliably, and to run a
*subset* of what exists.

## Analysis

**The harness is one straight line, so it cannot be aimed.** `runSmoke()` spans
[main.ts:5483-5935](../../../app/src/renderer/main.ts#L5483) — 453 lines, 20,601 bytes of TypeScript — a single
sequential function with no test names, no isolation, no selection, and no runner. There is no way to ask for
"the wizard" or "the network card" or "the cookie import". It is all of it or none of it. `check()` is a
three-argument arrow at [verify-m0.mjs:17](../../../app/scripts/verify-m0.mjs#L17) that appends to a list; a
failure does not stop the run and nothing is addressable after the fact. Any new "e2e spec" written into this
shape becomes line 460 of the same function.

**Elements are found by hand-built DOM, which is why specs cannot be written today.** The renderer has **42**
`querySelector` calls and **zero** `data-testid` attributes, in
[main.ts](../../../app/src/renderer/main.ts) (5,937 lines) and
[index.html](../../../app/src/renderer/index.html) (1,163 lines, 215 `id=` attributes). `index.html` carries
static ids; everything the smoke script touches inside a card, a row or a chip is located by structure or by
text, because the DOM is constructed in code. A spec that says "the Download button on the second card" has no
hook to hang on, so each new assertion re-derives a traversal. This is the actual gap the feature name points
at — not the absence of tests, the absence of **addresses**.

**The harness has a structural blind spot, and it is documented nowhere.** Under `--smoke`, `setup` returns
before wiring the tray or the adapter watcher ([lib.rs:325-327](../../../app/src-tauri/src/lib.rs#L325)), so
`watch_network_changes` never runs in a verified build. Anything that only exists in those surfaces — the tray
menu, adapter enumeration, the shipped spin-loop defect, and per-network feature work that depends on the
registry the watcher populates — is invisible to `npm run verify` and cannot be made visible by adding checks to
it. A green gate after such work would be a false reassurance. This bounds what any e2e spec effort can promise.

**"Dev-only tooling costs nothing" is false, and measurably so.** The smoke driver is not a separate entry
point: `runSmoke()` lives in the renderer source, so it is compiled into the shipped bundle. Its call survives
minification in `dist/renderer/main.js` (`e.smoke ? await Ni(e, s, c) : …`), and the cookie-import probe's
fixture string — `"cf_clearance=smoke-token; other=1"` — is present in that same 105,802-byte file. The gate is
`size < 4.3 * 1024 * 1024` ([verify-m0.mjs:31](../../../app/scripts/verify-m0.mjs#L31)) = 4,508,876 B against a
measured 4,488,192 B exe, so the whole budget is 20,684 B — and the harness already occupies part of it in the
binary, not just in the repo.

**A build-time gate is coupled to the current file layout, and that constrains any refactor.**
`checkHistoryKindsHaveUiCases()` ([build.mjs:55](../../../app/scripts/build.mjs#L55)) parses the Rust `history(…)`
kinds and the renderer's `formatEvent()` cases and fails the build when a kind has no UI case. It locates
`formatEvent` **textually** ([build.mjs:72-77](../../../app/scripts/build.mjs#L72)). Splitting `main.ts` into
modules — the obvious move if the file grows again — makes that lookup find nothing and the gate passes
vacuously. Whatever happens to the test surface, this gate must keep seeing the real function.

**The Rust side is healthy and is not the problem.** 134 `#[test]` sites across `src-tauri/src`, and the engine
already has a chaos server for failure injection. Cross-language contract tests exist (above). The weakness is
concentrated in the renderer and in the all-or-nothing shape of the gate.

## Goals vs. Non-Goals

* **In-Scope Goals:** a stable addressing scheme for renderer elements that survives markup edits; the ability
  to name a check and run a chosen subset; a report that says which check failed, not just a count; coverage of
  at least the surfaces the current harness cannot see, or an explicit statement that it cannot; a decision on
  where the smoke driver compiles to.
* **Out-of-Scope Non-Goals:** adopting Playwright/Vitest/Cypress or any Node-side browser driver — this is a
  Tauri app whose webview is the thing under test, and a second browser is a second behaviour; visual snapshot
  or pixel-diff testing; testing Rust engine correctness, which 134 tests already own; CI, which does not exist
  here; rewriting `runSmoke()` into a framework.

## Open Questions

* **Addressing scheme, three ways, no ranking yet.** `data-testid` attributes in the renderer, which is the
  convention but adds bytes to every card template; reuse of the existing 215 `id` attributes in `index.html`,
  which costs nothing but only covers static markup, not generated cards; or semantic labels already rendered on
  screen, which costs nothing in the binary and breaks when copy changes. The choice depends on whether the
  renderer is expected to grow generated surfaces (it is) and on the byte budget. Not decided.
* **Does the harness belong in the shipped binary?** Moving it behind a build flag saves bytes and risks
  shipping a renderer whose test path was never compiled — an unchecked path is not a safe path. Measured bytes
  first, then decide.
* **How far does one-at-a-time selection have to go?** A `--only=<name>` filter is a few lines inside the current
  function; a real registry of named checks is a rewrite of the same function. The gap between those is the
  whole scope decision, and it has not been made.
* **What is the answer for the watcher blind spot?** Options are a second smoke mode that does start the watcher,
  a fixture adapter injected in the harness, or accepting the hole and saying so in the gate's own output. A
  product decision, not an implementation detail.

## Alternatives Weighed

* **Write the e2e specs the feature name asks for, on top of what exists.** Rejected as stated: adding checks to
  an unaddressable 453-line sequence increases the debt and cannot be aimed.
* **Adopt Playwright against the WebView2 window.** Not weighed on evidence beyond the constraint that the app
  is Tauri and its webview is the product; a Chromium driver tests Chromium, not this renderer's host, and it
  adds a runtime dependency to a box that has no platform QUIC or CI to hang it on. Marked **unknown**, not
  disproved — it was not measured.
* **Leave it alone.** The gate passes today and 47 real checks run against the release binary on every `npm run
  verify`. Rejected only for the parts that bite: no way to run one check, and a blind spot in exactly the
  surfaces Feature 1 and Feature 3 touch.
* **Split `main.ts` to make room for a test structure.** Rejected for now, because of the textual `formatEvent`
  coupling above; the refactor would need the build gate fixed first, which is its own piece of work.

## Key Decisions & Staging for Next Steps

1. Drop the "zero coverage" framing wherever it appears. The finding is *unaddressable coverage*, which is a
   different and cheaper problem.
2. Settle the addressing scheme before writing any spec — it is the dependency every later check inherits, and
   it is the one choice that touches every card template.
3. Measure the shipped bytes the harness costs now, before arguing about moving it out of the bundle.
4. Name the blind spot in the gate's own output. A check that cannot see the tray or the watcher should say so
   rather than reporting a clean run.
5. Keep this behind Feature 1 and Feature 3 in priority, not ahead of them: those features are what made the
   gap visible, and neither is blocked by it.

## Related

* `per-network-limits-concepts_brief.md` — the feature whose registry lives in the watcher this harness cannot start.
* `notifications-autostart-concepts_brief.md` — tray and adapter-watch surfaces, likewise invisible to `--smoke`.
* `browser-bridge-concepts_brief.md`
