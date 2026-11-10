# OS Notifications, Quiet Hours and Login Autostart

* **Author:** Benjamin (machine account; `git config user.name` is unset on this box)
* **Date:** 2026-11-03

## Overview

Tell the user through Windows when a download finishes, hold those messages back at night, and
start Sucr at login so a scheduled transfer can run when nobody is at the desk.

As proposed: "OS notifications + quiet hours + autostart — cheap, and the scheduler is meaningless
without them."

The review found the opposite. Of the four concepts in this batch this one is the least cheap: it is
three features, two of which depend on machinery that does not exist yet, and the third of which
would switch on a thread that currently burns a core. The "cheap" in the proposal is real but
belongs to a different thing than the one named — the honest cue is a tray tooltip that is already
there and reads a static string today.

## Analysis

Six lenses reviewed this. Every one of them landed on the same structural objection from a
different direction: **there is no scheduler to defend.** The rest follows from that.

> **Correction 2026-11-06 — half of that objection is now false, and the half above is the part the rest of
> this brief relied on.** A **global download window** ships: four `settings` keys, an engine gate on Start
> and on the queue pump, and an edge-triggered release in the watcher's pass. It is proven in the launched
> exe — 10 legs / 97 checks `allLegsPass: true` on the 4 579 328 B build — and the record is
> [../../../design/download-window.md](../../../design/download-window.md). That record's ruling 9 also touches
> this brief's other half: the window's own sentence now reads `ui.close_to_tray` and says, when it is Quit,
> that the app ends when the last window closes so the advertised hours never open — **disclosure only**.
> Autostart is still what would make that sentence false, and still unowned.
> What remains absent, and what
> blocker 5 was actually about, is the *per-download* scheduler: the `schedules` table still has zero
> references in Rust, and there is still no wake timer. Quiet hours for notifications and autostart still
> have no scheduler of their own to defend, but "nothing anywhere decides when a transfer may start" is no
> longer a true sentence about this build.

### Blockers

**1. Autostart would switch on a spin loop. — blocker — CLEARED 2026-11-03**
`watch_network_changes` compares the adapter set to what it saw last pass and, when nothing
changed, executes `continue` ([lib.rs:183](../../../app/src-tauri/src/lib.rs)). The `continue` jumps to
the top of the `loop` and skips the five-second sleep written at the bottom of the body
([lib.rs:196](../../../app/src-tauri/src/lib.rs)). So on a machine whose adapters are not moving — which
is what a sleeping-at-night machine looks like — the thread re-enumerates through
`GetAdaptersAddresses` as fast as it can, and rewrites the `networks` rows on every pass, which is a
write to a WAL database each time. *[seen]*

**Status now:** fixed by step 2 of the per-network-limits sequence — the sleep belongs to the loop
([lib.rs:284](../../../app/src-tauri/src/lib.rs#L284)), so an unchanged pass pauses. **Correction to the
paragraph above, which was my own reading at the time:** the spin never rewrote the `networks` rows, because the
row write sits in the changed branch it could not re-trigger; the cost was CPU only. Line numbers quoted above
are pre-step-2 and have shifted. Autostart no longer has this prerequisite, but it still has the boot reconcile
and the lifetime ruling.

Today that costs something only while the app is open, which the user controls. Under login
autostart it costs from boot, forever, on a 6-core machine, and the user never sees it. The fix is
two characters (fall through to the sleep instead of `continue`), but it is a prerequisite, not an
option, and it is a shipped bug that exists right now independent of this concept.

**2. The window-exit rule and autostart contradict each other. — blocker**
The rule shipped yesterday: closing the main window with "close to tray" off means quit intent, so
`Destroyed` calls `app.exit(0)` when no `main` window is left and the tray is off
([lib.rs:250-254](../../../app/src-tauri/src/lib.rs)). *[seen]* That is the right behaviour for a person
finishing a session. It is fatal for an overnight schedule: the user closes the window at 23:00 and
the 02:00 run never happens, with no error and no log line saying so. Any autostart promise has to
answer which of these two rules wins, and today nothing answers it.

**3. Notifications from this build are a packaging project first. — blocker — MEASURED 2026-11-05, and blocked
on two OS/toolchain preconditions, not on code**
Windows toasts are addressed to an App User Model ID that has to be attached both to the shortcut
that launches the process and to the notification itself. This app is not packaged:
`bundle.active` is `false` ([tauri.conf.json:27](../../../app/src-tauri/tauri.conf.json)) and there is no
Start-Menu `.lnk` for it anywhere in the repo — it runs as `target\release\sucr.exe`. *[seen]*
Tauri's notification plugin is not a dependency at all; `[dependencies]` is seven crates and the
capability list is `core:default` alone ([capabilities/default.json](../../../app/src-tauri/capabilities/default.json)).
*[seen]* The `windows` crate is linked with four modules — Foundation, System_Com, UI_Shell,
UI_WindowsAndMessaging ([Cargo.toml:43-48](../../../app/src-tauri/Cargo.toml)) — which does not include
`Win32_UI_Notifications` or `Data_Xml_XDom`. *[seen]* The registry key autostart needs
(`HKCU\...\Run`) is likewise not among the enabled modules.

**Status now — step 0 measured before any byte was spent**
([../../../design/notification-identity.md](../../../design/notification-identity.md), 0 B; every claim below
is an `evidence-noun: launched probe` or `read-only registry probe` from that run):
* Toasts are switched off **on this machine at the OS level** — `PushNotifications\ToastEnabled = 0` and
  `Notifications\Settings\NOC_GLOBAL_SETTING_TOASTS_ENABLED = 0`. Consequence: `ToastNotifier.Setting` returns
  `DisabledForUser` for an unregistered synthetic id **and for six real registered ids read out of this box's
  own Start Menu**, so a control group identical to the test group means **no identity proof is producible here
  until the user turns that switch on** — a ruling, not a step.
* `CreateToastNotifier()` with no argument fails `0x80070490 Element not found` for a loose exe (the packaging
  gap in the OS's own error code), while `CreateToastNotifier('<any string>')` **succeeds for an id that exists
  nowhere** — so object creation is not an identity test, and an implementation that reads "the call returned"
  as "we have an identity" is wrong on day one.
* `Show()` returns without exception while the OS refuses everything → **raised is not heard**, again. No Sucr
  code may report "notification sent" from the success of the call.
* There is **no installer toolchain on this box**: `tauri-cli 2.12.4` is present, NSIS and WiX are not, and
  `bundle.active` is `false`, so an installed identity needs either a toolchain install or a Start-Menu /
  registry write into the user's profile — neither of which this harness does unasked.
  **CORRECTED 2026-11-07:** the observation about PATH stands, the conclusion does not. The CLI fetches its
  own NSIS; `tauri build --bundles nsis` produced a **1 866 169 B** setup exe whose embedded `sucr.exe` is
  **4 593 152 B**, identical in size to the shipped one — bundling costs **0 B** of exe. No toolchain install
  is needed. What an identity still needs is something that *writes* it (a real install, or a Start-Menu /
  registry write), and that remains the user's to authorize. WiX is still absent; MSI is still unavailable.
  Evidence: [capture-step0-spikes.md](../../../design/capture-step0-spikes.md) § 1.
* `predicate:` correction to the paragraph above — the new opt-ins are `Win32_UI_Notifications` and
  `Win32_Data_Xml_Dom`; `Win32_System_Com` **is** already enabled. The byte cost of the two new features is
  **`unmeasured`** on purpose: pricing it would mean app code on an unprovable path, and an inert parameter
  measures a misleading 0 B. Per the admission rule it cannot become a premise, and it is priced after the
  identity route is ruled, not before.

What is provable on this box today, with no identity and no OS switch, is the **live tray tooltip**
([lib.rs:274](../../../app/src-tauri/src/lib.rs#L274) is a static `.tooltip("Sucr")`) — the piece this brief
already chose as candidate (a).

So "add notifications" means: pick a packaging path, produce an installed identity, opt in
Windows API modules, and add a plugin crate. Each of those is a decision of its own; none of them is
in the proposal.

**4. ~~Transfers do not survive a reboot, so the promise behind autostart is unearned.~~ — CLEARED 2026-11-05:
the boot reconcile shipped and is launched-proven**
On boot, `AppState` used to be built with `engine: Arc::new(engine::job::Registry::default())`
([lib.rs:310-319](../../../app/src-tauri/src/lib.rs#L310); that same field is `lib.rs:455` today). *[seen, pre-reconcile]* That was an empty registry —
nothing read the database for rows still marked `downloading`, nothing resumed them, nothing marked them failed,
so after a reboot those rows claimed to be running forever. **That is no longer the build.** `reclaim_interrupted`
([ipc.rs:806](../../../app/src-tauri/src/ipc.rs#L806), `:798` when this was first written; wired in `setup()` at
[lib.rs:472](../../../app/src-tauri/src/lib.rs#L472), `:437` then) reclaims a `downloading` row left behind by a machine that
stopped to `queued`, and the existing queue resumes it — **+2,048 B**, proven in the launched exe by 3 legs /
19 checks `allLegsPass: true` including a counterfactual on the previous binary, and shown *not* to launch
never-started or `paused` rows
([../../../design/boot-reconcile.md](../../../design/boot-reconcile.md)).
Autostart therefore no longer waits on a boot reconcile. What it still waits on is this brief's blocker 2 (the
lifetime ruling) and a scheduler that does not exist yet (blocker 5).

**5. The scheduler being defended does not exist. — blocker — PARTLY SUPERSEDED 2026-11-06**
The `schedules` table is defined with its kind, minutes, days, end date and retries
([schema.sql:23-33](../../../app/src-tauri/schema.sql)) and has **zero references in Rust** — grepping
`schedules` across `src-tauri/src` returns nothing. *[seen]* The renderer already says so in the
interface: "No global schedule window or wake timer is configured in this build."
([main.ts:3507](../../../app/src/renderer/main.ts)). *[seen]*

The proposal's own reason for existing — "the scheduler is meaningless without them" — therefore has
no scheduler in it. Quiet hours with nothing to suppress, and autostart with nothing to run, are two
features waiting on a third that is not in this batch.

**What is still true of the two `*[seen]*` marks above, and what is not** (re-checked 2026-11-06, all of it
against the source and the shipped binary, `writer:` grep):

- **True:** the table is still dead. `grep -rn "schedules" src-tauri/src` reaches nothing but a design
  comment, and `grep -rn "schedule_id\|queue_id" src-tauri/src src/renderer` returns **0 matches** — no row
  type is scheduled, and the per-download start that table was drawn for is not implemented. So the half of
  this blocker that autostart and quiet hours actually lean on still holds.
- **False now:** the quoted interface sentence. It was asserting the absence of a global schedule window,
  and a global download window shipped. The sentence was found still sitting in the per-download Schedule
  pane while the Settings panel described the feature two clicks away — no gate had ever read it — and was
  replaced by the one shared rule sentence both surfaces call
  ([main.ts:4005](../../../app/src/renderer/main.ts#L4005) → `windowRuleText()`,
  [main.ts:2473](../../../app/src/renderer/main.ts#L2473)). In the linked exe the old string is gone and the
  new one is present, proved by `grep -a -o` counts (0 / 1), and a launched leg asserts the pane says it:
  `window-report-2026-11-05T23-16-18-936Z.json`, `schedule_pane` leg, 11 checks
  ([../../../design/download-window.md](../../../design/download-window.md) § 8).
- **The locator above drifted twice:** `main.ts:3507` was `main.ts:3996`, and after ruling 9 lengthened the
  shared sentence it is **`main.ts:4005`**; the function itself is `windowRuleText()` at
  [main.ts:2473](../../../app/src/renderer/main.ts#L2473). That is the usual
  price of citing a line instead of a function name, and this file now carries the name plus the live `#L` —
  re-read with `grep -n` on the current source, not recalled.

**6. ~~The chime is already silent when the app is in the tray, and it double-fires.~~ — RETRACTED 2026-11-05, both halves false; this finding is now the opposite of a blocker**
The two claims in this heading's original text were read off source with `*[seen]*` as the only warrant, and
a launched-exe run (`chime-run.mjs`, **7 legs / 37 checks `allLegsPass: true`** on the unmodified 4,569,600 B
exe; record [../../../design/completion-chime.md](../../../design/completion-chime.md)) refuted both:
* **"Chimes twice on auto-verify" — false.** `chime()` does fire on either the `finished` **or** `verified`
  edge ([main.ts:2594-2596](../../../app/src/renderer/main.ts)), but a job never crosses both: one `UPDATE`
  statement writes the terminal status ([engine/job.rs:3468](../../../app/src-tauri/src/engine/job.rs#L3468))
  and is the crate's only terminal-state writer — `writer:` grep. The listener compares against the previous
  row, so the edge exists once per job at most. Measured, not inferred: **exactly 2 notes per completion on
  all 7 legs** (two oscillators in one `AudioContext`, so "2 notes" = "one chime"), never 4.
* **"Silent when tray-resident" — false.** The mechanism borrowed here is Chromium's hidden-page timer
  throttling, and its predicate is `document.visibilityState === 'hidden'`. Measured on this build under
  `SW_MINIMIZE`, `SW_HIDE`, and the app's own close-to-tray (`WM_CLOSE` → `CloseRequested` → `window.hide()`),
  that value is **`'visible'` in every leg** — WebView2 never flips it for a top-level window hide, so the
  throttling has no predicate to engage. The chime **does** reach a person who is not looking at the window:
  29-260 ms, **median 210 ms** after `finished_at`; after a 318 s dwell the page woke at 1,004 ms and still
  chimed.
What survives is narrower and is not a defect: **a two-tone sine chirp is not an OS notification** — no
Start-Menu record, no Action Centre history, nothing an unattended machine can hand to someone who walked
away. That gap is real, and its blocker is the packaging identity (installed app id + shortcut + the
`Win32_UI_Notifications` path nothing currently builds), **not** a broken cue. *The premise this finding fed
— "the tray-resident finish cannot announce itself, so notifications are needed" — was load-bearing for the
"notifications + autostart + chime are one interacting bundle" framing of this concept, and it is gone:
autostart, notifications and the chime are three independent things.*

### Majors

**Quiet hours is already decided elsewhere, and Windows has the feature.** The design plan puts
"Login autostart, notifications (with OS Focus Assist detection)" under **General**
([app-design-plan.md:127](../../../design/app-design-plan.md)) and puts night-schedule rules and
wake timers under Limits & Schedule at :131. *[seen]* **Correction to my own earlier note:** I
reported that the plan assigns quiet hours to "Limits & Schedule". It does not — the plan assigns
notifications and Focus-Assist detection to General, and only schedules to Limits & Schedule. Focus
Assist already suppresses toasts on a Windows schedule the user can set in the OS; a second,
app-private night rule that disagrees with the OS one is user-visible confusion, not a feature.

**Settings are cheap and the count is known.** One new setting key is three edits: `SETTING_KEYS`
is a fixed `[&str; 17]` ([ipc.rs:834](../../../app/src-tauri/src/ipc.rs)), `validate_setting` is the
matching branch, and `SETTING_DEFAULTS` is the renderer's copy
([main.ts:139](../../../app/src/renderer/main.ts)). *[seen]* That is not where the cost is; the cost is
the API surface, the packaging identity and the registry. For scale: the release binary is
4,488,192 B against a 4,508,876 B ceiling, i.e. 20,684 B of headroom as measured today. *[measured:
`ls -l src-tauri/target/release/sucr.exe`]* A new IPC command has cost about 11 KB each
([verify-m0.mjs:28-30](../../../app/scripts/verify-m0.mjs)). *[seen]*

**The cheap cue is already in the building.** The tray tooltip is a literal
`.tooltip("Sucr")` ([lib.rs:127](../../../app/src-tauri/src/lib.rs)) — static, while the app knows its
own live throughput. *[seen]* The renderer has an `aria-live="polite"` status region, `#toast-notice`
([index.html:1159](../../../app/src/renderer/index.html)), already wired to `toastEl`
([main.ts:1980](../../../app/src/renderer/main.ts)). *[seen]* A tray tooltip that reads "3 running ·
6.2 MB/s · last finish 21:14" costs no Windows API, no plugin, no identity and no packaging, and it
works when the window is hidden.

### Minors

* Naming: "notifications" covers an in-window toast, a tray tooltip, a sound and a Windows toast.
  The proposal uses one word for four different surfaces with four different costs.
* The data hazard that makes notification content sensitive — cookies stored in plaintext at
  [db.rs:286](../../../app/src-tauri/src/db.rs) — was found while reviewing the browser bridge and
  applies here too: a lock-screen toast shows whatever the message says. *[seen: the column write;
  inference: lock-screen exposure depends on the user's Windows settings]*

## Goals vs. Non-Goals

**In-scope goals**
* Make completion legible without the window in front: a live tray tooltip, and the existing
  in-page live region for in-window cases.
* Fix the `continue`/`sleep` defect in `watch_network_changes` — **done 2026-11-03** (per-network-limits
  step 2) — on its own merit, before either feature leans on it. ~~and the double chime~~ — **retracted
  2026-11-05: there was nothing to fix. `launched-exe run` (`chime-run.mjs` 7 legs / 37 checks green,
  `design/completion-chime.md`) — one terminal-status writer at `engine/job.rs:3468`, 2 notes per completion
  on every leg.** Leaving this goal in the list was how the false premise kept being re-borrowed.
* Decide, on paper, whether autostart or close-to-exit owns the process lifetime, before writing
  either.

**Out-of-scope non-goals**
* Windows toast notifications, until the app has an installed identity and a packaging decision.
* An app-private quiet-hours rule, while Focus Assist is the OS's answer to the same question and
  the design plan already names it.
* Login autostart, until there is a scheduler with something to run. **The other half of this condition
  is already satisfied — `launched-exe run` 2026-11-05 (`design/boot-reconcile.md`, 3 legs / 19 checks
  `allLegsPass: true`, +2,048 B): transfers do survive a stop-and-restart, because a row left claiming
  `downloading` is reclaimed to `queued` at boot (`ipc.rs:806`, `:798` when written). The concept's founding premise — "the
  scheduler is meaningless without autostart, and autostart is meaningless without a boot reconcile" —
  now has its boot reconcile, and what is still missing is only the scheduler.**
* Building the scheduler. It is a separate concept and is not in this batch.

## Alternatives Weighed

* **Ship all three as proposed.** Rejected by review (6 of 6 lenses): the spin loop, the exit
  contradiction and the missing scheduler make it the one concept in the batch that could ship a
  regression rather than a feature.
* **Package first, then notifications, then autostart.** Correct order, but it puts a build and
  installer project in front of a download manager whose other three concepts are product
  behaviour. Deferred, not rejected.
* **Dynamic tray tooltip instead of toasts** (chosen candidate): zero new Windows API, zero plugin,
  visible when hidden, and it tells the truth about a transfer the app already measures. Cost: a
  tooltip is not an interruption, so a finished-at-3-AM job still goes unnoticed unless the user
  looks. That is the honest limit of this option.
* **Widen the existing in-page `#toast-notice` live region to a notification queue.** Already
  accessible, already there, but it cannot reach a user whose window is hidden.
* **`windows`-crate direct toast API rather than the Tauri plugin.** Rejected as a first move: it
  needs `Win32_UI_Notifications` plus `Data_Xml_XDom` opted in and still needs an AppUserModelID, so
  it buys the packaging problem without buying the plugin's capability handling.
* **Do nothing and let the chime stand.** **Rejected — but not for the reason first written here.** The
  original grounds ("it is silent in exactly the tray case this feature is for, and it double-fires on
  auto-verify") were both refuted on 2026-11-05 by a `launched-exe run` — 7 legs / 37 checks green,
  `design/completion-chime.md`: the chime arrives 29-260 ms after `finished_at` with the window minimized,
  hidden, and hidden-by-close-to-tray, and it fires once because one `UPDATE` writes the terminal status
  (`writer:` grep at `engine/job.rs:3468`). What the option is actually up against is narrower and still
  true: a two-tone sine chirp leaves **no OS record** — nothing in Action Centre, nothing reviewable after
  the fact — and it needs the process to be running with a live renderer, so it cannot announce a job that
  finished while the app was closed. That is the honest case for a notification, and it does not depend on
  the chime being broken. See blocker 6.

## Key Decisions & Staging for Next Steps

1. **Only the user writes DECIDED.** Nothing in this brief is a ruling.
2. ~~Treat the `continue`/`sleep` defect as a shipped bug report now, separate from this concept, and get the
   user's go-ahead before touching code~~ — **done**: the user's "Proceed to step 2" gave that go-ahead and the
   loop is fixed (see blocker 1 above).
3. Split the concept into three independently shippable pieces: (a) live tray tooltip, (b) quiet
   hours — probably "delegate to Focus Assist", (c) autostart.
   ~~blocked behind a boot reconcile plus a packaging and identity decision~~ — **the boot reconcile half is
   done** (2026-11-05, `design/boot-reconcile.md`, +2,048 B, launched-proven), so (c) is now behind blocker 2
   (the lifetime ruling), blocker 5 (a scheduler that does not exist), and its own `HKCU\...\Run` write; (a) is
   behind nothing at all and is the only piece provable on this box today; (b) needs no OS identity either.
4. Decide the lifetime rule before autostart: does an overnight schedule need the process to survive
   a window close, and if so does close-to-tray-off stop meaning quit?
5. Ask whether the scheduler belongs in this batch at all, since features (a)-(c) and concepts 1 and
   2 are all downstream of whether a schedule exists.
6. ~~Carry the double-chime and the plaintext-cookie findings into the drafting step rather than
   leaving them as review trivia.~~ — **halved 2026-11-05.** The double-chime never existed (blocker 6,
   `design/completion-chime.md`), so carrying it into drafting would have shipped a fix for a false defect; only
   the plaintext-cookie finding travels. The toast coverage gap travels as a **question**, not a premise:
   `design/notification-identity.md` § 4 keeps its byte cost `unmeasured` until the identity route is ruled.
