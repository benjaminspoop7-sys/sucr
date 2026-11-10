# Completion chime — a held defect that was not a defect

Record of the measurement that closed the front item of the held-defect queue without spending a product
byte. Written 2026-11-05 under the same delegation that landed the boot reconcile ("Do as you see fit"),
and governed by the standing rule that a claim is retracted when the measurement refutes it, not patched.

Everything below is either cited to a file:line, counted, or read out of one launched-binary report:
`D:\RE\_cap-harness\chime-report-2026-11-05T03-05-33-759ZZ.json` — 7 legs / 37 checks, `allLegsPass: true`,
taken against `sucr.exe` at **4,569,600 B**, mtime `2026-11-05T01:18:25.678Z`. That size is asserted by the
harness itself before it launches anything, so the report cannot describe a build that differs from the
shipped one.

**This record is the template the admission rule cites.** The two claims it refuted had been carried for two
days with `writer:` and `predicate:` unanswered — "auto-verify crosses both terminal edges" never enumerated
who can *write* a terminal status, and "hidden webview means throttled timers" never measured the predicate
Chromium actually keys on. The rule now lives at `app/docs/designs/SITEMAP.md` § *Admission rule for what is
asserted as fact*, and with the defect queue at `project-sucr-open-defects.md` § *Admission rule for records*.
The promotion this file demonstrates — one run, both halves refuted, **0 B spent** — is what the rule asks for
before an `unmeasured` claim is allowed to lead a queue.

## 1. Why this slice

`project-sucr-open-defects.md` held two claims about the chime, and put them at the front of the queue
because they looked cheap:

- *"the renderer's status edge chimes on `finished` **or** `verified`, so an auto-verify job sounds twice"*;
- *"it is also renderer WebAudio, so it is silent when the window is hidden in the tray — which is the
  audience autostart is for"*.

The second was load-bearing: the autostart and notification concepts were priced on the assumption that a
tray-resident finish cannot announce itself. Both halves are **false**, and both were refuted by
measurement rather than by argument. Because the cheap fix that looked owed was a retraction, the rule
applied: measure first, spend nothing until the measurement says something is owed.

## 2. What is true in the source (`writer:` grep, locators inline)

**One completion is one terminal status, so one edge.** The only statement in the crate that can put a row
in a terminal state is
[job.rs:3468](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3468):

```rust
let status = if checksum { "verified" } else { "finished" };
```

inside the single `UPDATE downloads SET status = ?1, … finished_at = ?5` at
[job.rs:3470-3473](file:///D:/RE/Sucr/app/src-tauri/src/engine/job.rs#L3470). `grep` for `"verified"` over
`src-tauri/src` finds that write and no other: there is no channel that later flips `finished` to
`verified`, and no verify-after-the-fact path that could hand the renderer two different terminal values for
one job. The renderer's edge at
[main.ts:2966](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2966)
(`(d.status === 'finished' || d.status === 'verified') && before !== undefined`) can therefore fire once.
The claim was a plausible reading of a disjunction without checking who could satisfy each side of it.

**The chime is a renderer behaviour, not an engine one.** The chain is: engine emits `download:changed` →
`listen` → `scheduleRefresh()` → `window.setTimeout(…, 150)` → `refresh()` → `detectCompletions()` →
`chime()` ([main.ts:4314-4320](file:///D:/RE/Sucr/app/src/renderer/main.ts#L4314),
[main.ts:2925-2955](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2925)). Two sine oscillators, 880 Hz then
1320 Hz, ~180 ms envelope, on one lazily constructed `AudioContext`. So the question "does it sound when the
window is away" is really "does a WebView2 page whose top-level window is put away still run its timers, and
does WebAudio still work there" — which is a measurement question, not a code-reading question.

**The gate covers none of it.** `grep -c "sound_on_complete" scripts/verify-m0.mjs` returns **0**. The key
appears in `index.html` (the Settings checkbox at
[index.html:661](file:///D:/RE/Sucr/app/src/renderer/index.html#L661), labelled *Completion chime*), in the
renderer (default `true` at [main.ts:157](file:///D:/RE/Sucr/app/src/renderer/main.ts#L157), the guard at
[main.ts:2932](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2932), the wizard copy at `:5822`/`:5871`), and in
Rust only as an allowed key and a validator arm
([ipc.rs:1029](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L1029),
[ipc.rs:1351](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L1351)). Nothing anywhere asserted that the box the
user clicks has any effect — until this run.

## 3. How it was measured

`D:\RE\_cap-harness\chime-run.mjs` launches the shipped exe with its own `--data-dir`, its own
`WEBVIEW2_USER_DATA_FOLDER` and `--remote-debugging-port=9361`, and drives it over CDP. The fixture is a
local Range-capable 8 MiB file served at 2 MiB/s (so a completion lands ~4 s after Start), and the tray legs
use 256 KiB/s so the window can be put away and stay away for most of a transfer.

The witness is patched into the page, not into the bundle:

```js
BaseAudioContext.prototype.createOscillator = function () {
  window.__chime.notes += 1;
  window.__chime.states.push(this.state);
  window.__chime.visibility.push(document.visibilityState);
  window.__chime.times.push(Date.now());
  …
```

Counting on the prototype means the product's own `new AudioContext()` is observed whoever constructs it,
and each note records the state it was raised in. What a pass reports is therefore: **how many notes a
completion scheduled**, **what the page believed its visibility was at that instant**, and **the gap from the
engine's own `finished_at` to the first note** — the page and this driver read the same machine clock, so the
two are directly comparable. That gap is the whole cost of any timer throttling, in the only unit a user
waiting for a sound feels.

The window is put away three different ways, because they are three different questions, all through
`chime-window.ps1`: `SW_MINIMIZE`, `SW_HIDE`, and `WM_CLOSE` — the last one so that the product's own
`CloseRequested` → `close_hides_to_tray` → `window.hide()` path
([lib.rs:453-460](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L453)) does the hiding, which is the tray the
user actually reaches. The state is always read back from the window manager (`IsIconic` /
`IsWindowVisible`), never trusted from the script that asked for it.

**Two harness bugs had to die first, and both are traps worth recording:**

- The first draft of the PowerShell helper joined statements with `\n`, which split `Add-Type
  -MemberDefinition` from its value, so the P/Invoke type never compiled. `stderr` printed the error
  *after* stdout printed `OK`, and an `out.includes('OK')` gate reported a minimize that never happened. The
  helper now compiles the whole definition inside one `@'…'@` here-string, and the runner requires exit 0
  **and** a parseable `RESULT` line.
- The next draft took `Get-Process.MainWindowHandle` on trust. For this app that handle is not the product
  window: the real top-level is class `Tauri Window`, and the process also owns a `Tao Thread Event Target`
  tool window. The readback said `iconic=false visible=true` after a minimize, then `iconic=true
  visible=true` after a hide — both believed a page that had been visible the whole time. The helper now
  enumerates with `EnumWindows` + `GetWindowThreadProcessId` + `GetClassName` and selects the window whose
  class is `Tauri Window`, among the windows owned by *this* harness child's pid.

Nothing about the user's own app, profile, window or library is touched: this instance has its own data
dir, and no other pid's handle is ever passed to `ShowWindow`.

## 4. Results — 7 legs, 37 checks, all pass

| leg | state at completion | notes raised | `finished_at` → first note | page visibility |
| --- | --- | --- | --- | --- |
| 1 visible, never gestured | control | 2 | 239 ms | `visible` |
| 2 minimized mid-transfer | `iconic: true` | 2 | 29 ms | `visible` |
| 3 restored | `iconic: false` | 2 | 260 ms | `visible` |
| 4 `SW_HIDE` from outside | `visible: false` | 2 | 220 ms | `visible` |
| 5 Settings toggle **off** | stored `false` | **0** | — | `visible` |
| 5 Settings toggle **on** | stored `true` | 2 | 210 ms | `visible` |
| 6 `WM_CLOSE` → app's own close-to-tray | `visible: false` | 2 | 202 ms | `visible` |
| 7 same tray, 318 s dwell + ~34 s before it | `visible: false` | 2 | 202 ms | `visible` |

- **Not doubled**: every completion scheduled exactly **2** notes, which is the two-oscillator chime. A
  doubled chime would read 4. `AudioContext` constructions across the whole session: **1**.
- **Not suspended**: each note recorded `state: 'running'`, and the shared context's `currentTime` advanced
  monotonically across the session (2.24 → 43.14 → 78.56 s in the earlier pass) — a suspended context does
  not advance. No page gesture was ever made, so WebView2 allowed an audio context built with zero
  interaction to run.
- **The tray case is not a defect, and the mechanism is now named**: under minimize, `SW_HIDE` *and* the
  product's own close-to-tray, `document.visibilityState` stayed **`visible`** and `document.hasFocus()`
  stayed **`true`**. Chromium's timer throttling — and its one-wake-per-minute intensive variant — is
  conditioned on the page reporting hidden, so there was never a condition for it to engage on. Leg 7
  crossed the five-minute threshold anyway (318,877 ms of dwell on top of the ~34 s leg 6 had already spent
  hidden) and measured the page responding to a fresh `downloading` edge **1,004 ms** after Start — one
  timer tick, not one minute — then chiming 202 ms after `finished_at`.
- **Spread**: 29–260 ms, median 210 ms, across seven completions. The low end is not a different mechanism;
  the refresh is event-driven with a 150 ms debounce, so when the terminal event arrives inside an
  already-open window the note can be raised almost immediately.
- **The toggle works, and now something says so**: clicking the real checkbox made the page write
  `ui.sound_on_complete = 'false'` through its own `change` handler
  ([main.ts:2885-2890](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2885)); the next completion **raised no
  notes at all** in a 4-second grace (generously longer than the ~210 ms the same page takes when on); and
  clicking it back on restored `'true'` and the following completion chimed 2 again, at 210 ms. Before the
  first click the stored value read `null` — the key has no row in a fresh data dir, so the chime is on by
  the renderer's default map, not by a persisted setting.
- **Silence was not a crash**: the gated transfer still reached `finished` with its `finished_at` written,
  so the zero came from the setting, not from a page that had stopped working.

## 5. Cost

**0 B.** No source file was edited for this slice; the exe is the same 4,569,600 B the boot-reconcile slice
left, with 673,280 B of headroom under the 5 MB ceiling the user ruled on 2026-11-04. The correction is a
retraction in the record, not a patch. `CHANNELS` 35, the gate's `check(` sites 50 and the Rust test count
are all unmoved.

## 6. What this does not prove

- **Raised is not heard.** The witness counts `createOscillator` calls and reads the context's state and
  clock. No audio was captured, no waveform was recorded, and nothing measured the default output device.
  The strongest honest sentence is that the audio graph was constructed, running, and advancing its clock at
  completion — not that a speaker emitted sound in a room.
- **CDP attachment could in principle change throttling behaviour.** The mitigation is that the page never
  reported hidden in any leg, so there was nothing for the throttling to be applied to; but the run cannot
  exclude an attachment effect on some other timer path.
- **`WM_CLOSE` was exercised on a fresh instance whose data dir had never set `ui.close_to_tray`** — the
  default is hide (any non-`'false'` value, `fn close_hides_to_tray` at
  [lib.rs:248-262](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L248), the test itself at `:260`),
  so this is the shipped behaviour, but the *Quit* branch of that setting was not measured here and belongs
  to the lifetime-contradiction item, which is still the user's ruling to make.
  > **Correction to this bullet and to leg 6's own check (2026-11-06).** Two things are now untrue as
  > written. (1) **The Quit branch has been measured** — not by this file's suite but by `window-run.mjs`
  > legs `close_quit_ends` (7 checks) and `quit_hours_never_open` (16), and the record is
  > `design/download-window.md` § 5 and § 7. (2) **Leg 6's label over-claimed what its predicate proved.**
  > The check read "the process lives", and all it actually read was `winState()` — `visible: false` — which
  > a destroyed window also answers, since the helper simply fails to find a handle. So on the *hide*
  > branch a real quit would have passed the same check. The predicate now reads four things: the window
  > gone, the window state re-read after the close, and `tasklist` naming the launched pid with image
  > `sucr.exe` (`function pidAlive` at
  > [chime-run.mjs:103](file:///D:/RE/_cap-harness/chime-run.mjs#L103), the check at `:710-713`).
  > `evidence-noun:` `chime-report-2026-11-06T00-33-21-530ZZ.json` — **6 legs / 31 checks,
  > `allLegsPass: true`** — whose leg-6 first check detail carries `{win: {action: 'close', visible: false,
  > windows: '1'}, still: {action: 'state', visible: false, windows: '1'}, alive: 'sucr.exe …'}`. The
  > `windows: '1'` in both fields is the witness that a *destroyed* window would not return, so this leg
  > measures the hide branch and not a quit. **0 product bytes**: harness and records only, and the same
  > build the window suite measures.
  > **One pin moved while I was in there:** the run refused to start against the boot-reconcile build
  > (`st.size !== 4569600`) and is re-pinned to the current `4 579 328` at
  > [chime-run.mjs:568](file:///D:/RE/_cap-harness/chime-run.mjs#L568), so § 5's "4,569,600 B" below is
  > that slice's cost at the time it ran, not the exe this addendum re-passed on.
- **Nothing here prices the usefulness of a chime** as an announcement for an unattended machine. Two sine
  tones at 880/1320 Hz are not a notification; the OS-notification packaging gap below is untouched and is
  the one that actually limits autostart.

## 7. What follows from it

The held queue loses its front item, and the load-bearing premise it carried ("a tray-resident job finishing
cannot announce itself") is gone: the chime is raised on time from the tray. What remains in that area is
the packaging identity that would let the app raise a *toast* — `bundle.active: false`, `core:default`
capabilities only, no tauri plugin dependency, and a `windows` crate that opts neither
`Win32_UI_Notifications` nor `Win32_Data_Xml_Dom` — which needs an installed identity and a
Start-Menu shortcut, so it is a release-engineering decision rather than a slice. That is the user's to
rule, and it is now the front of the queue.

> **Correction to this paragraph (2026-11-05, same day).** As first written it listed
> `Win32_System_Com` among the features the crate lacks. The `writer:` grep at `src-tauri/Cargo.toml:43-48`
> refutes that: `Win32_System_Com` is one of the four already opted in. The step-0 measurement of this item
> is `design/notification-identity.md`.
