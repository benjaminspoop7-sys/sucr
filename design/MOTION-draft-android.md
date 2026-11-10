# Motion language — Sucr

DRAFT, 2026-11-03, authored by the motion/UI-UX agent. Not ratified. Move to the app
repository root as `MOTION.md` only after a human has resolved every `OPEN` entry and
committed it. No entry below is `DECIDED`, and an agent must never write that tag.

## Design system
Custom design system, not Material 3 theming: a six-step duration ladder and three easing
curves in `design/tokens.css`. Rules come from the `android-ux-skills` pack and are applied
through each rule's "Outside Material" binding, because this app has no Material theme.

## Fallback
Where both this file and `tokens.css` are silent, use a non-overshooting curve for anything
that encodes a value and a springy one only for containers. Never invent a new duration —
pick a tier. Ask before picking neither.

## Specs

### State change on a control (hover, focus, press, selection, colour)
- **Intent** — instant and mechanical. No travel, no overshoot; the state simply is.
- **Binding** — `--e-std` at `--dur-2`
- **Tag** — `OBSERVED`

### A surface moving to a new position
- **Intent** — settles without bounce. Confident, not playful.
- **Binding** — `OPEN`
- **Tag** — `OPEN` — spatial declarations split between the token ladder and raw literals.

### Geometry that encodes a measured value (progress fill, throughput bar, step dots)
- **Intent** — the width *is* the number. It must never read as having passed the truth.
- **Binding** — `OPEN`
- **Tag** — `OPEN` — the download bar and the wizard fill currently disagree. Ratify one.

### Enter of an overlay or a screen
- **Intent** — arrives, decelerating into place.
- **Binding** — `OPEN`
- **Tag** — `OPEN` — `--e-dec` exists and is not used.

### Exit of an overlay or a screen
- **Intent** — leaves with fewer properties and on a quicker tier than it arrived.
- **Binding** — `OPEN`
- **Tag** — `OPEN` — `--e-acc` exists and is not used.

### Repeated per-item motion in a list
- **Intent** — the group settles inside one short window, not one item per tick.
- **Binding** — `OPEN`
- **Tag** — `OPEN`

### Continuous indicator of live work
- **Intent** — reads as "still going" at a glance, and can be stopped on its own surface.
- **Binding** — `--wave-*`
- **Tag** — `OPEN` — whether a loop may run at all when the OS asks for no animation is unsettled.

## Exceptions
- `--wave-*` — allowed to keep running under reduced motion because the wave is an explicit
  in-app choice. Expires the moment it stops being user-selected.

## Frequency map
- **Never animate** — rows that only re-report bytes, menu chrome, tooltips
- **Standard** — download rows, popover open/close, wizard steps, inspector sections
- **Delight allowed** — the completion moment, once per job

## Gestures and haptics
A value the finger controls keeps the finger's velocity on release and hands it to the
settle; nothing dragged snaps to a stop. Dismiss thresholds are a fraction of the surface,
not a fixed pixel count. Haptics are out of scope for the desktop webview and required for
the Android port.

## Reduced motion
Android's "Remove animations" snaps transitions to their end state rather than softening
them, so every state must be readable at rest and every motion must carry its meaning
statically. An in-app `ui.motion` toggle exists; a stop control on the animating surface
itself is still owed.

## Not in this codebase
- Celebration on a retraction (un-star, remove, undo) — a retraction is the inverse of its
  forward motion, not an achievement.
- Overshoot on any width, height or position that encodes a measured value.
- Endless shimmer running beside readable content.
