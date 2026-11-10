# Global download window — when Sucr is allowed to *start* a queued link

Record of the slice that made "queued links start inside the hours you chose" true. Written 2026-11-06
under the user's standing delegation ("Do as you see fit"), as steps 1–4 (+4b) of the window plan, with a
second pass the same day (ruling 9, §11) after the parity review showed the rule could be inert.
Everything here is cited to a file:line re-read today, counted, or read out of a launched-binary report;
where a claim has no such warrant it is marked `unmeasured`. Confidence words are not evidence.

## 1. Why this slice

The Plexo gap analysis held one scheduling absence: Sucr could start a link the moment it was added, and
nothing let a user say "only overnight". `docs/designs/notifications-autostart-concepts_brief.md` had made
the same absence a blocker for two *other* concepts ("there is no scheduler to defend", line 23). This
slice builds exactly one thing — a global, app-wide start gate — and deliberately does not build the rest
of what that word suggests. §7 lists what is still absent, and §8 retracts the sentence this slice made
false without updating it.

## 2. The mechanism (`writer:` grep over `src-tauri/src` and `src/renderer`, locators re-read 2026-11-06)

Four `settings` keys, added to the 24-key surface
([ipc.rs:1026 `SETTING_KEYS: [&str; 24]`](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L1026), the window
keys at [:1031-1034](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L1031)):
`engine.window_enabled` (`"true"`/`"false"`), `engine.window_start`, `engine.window_stop` (`HH:MM`),
`engine.window_days` (a Monday-first bit mask, `0`–`127`). They are four keys and not a `schedules` row,
for the reason written in the source itself ([db.rs:285-288](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L285)).

The judgment is one struct:

| piece | locator | what it decides |
| --- | --- | --- |
| `DownloadWindow` | [db.rs:289](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L289) | the four values, `days` masked to `& 0x7f` on read ([db.rs:348](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L348)) |
| `open_at(weekday, minute)` | [db.rs:308](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L308) | returns `true` at once when `!enabled` ([db.rs:309](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L309)); `start == stop` is every minute; `stop < start` wraps overnight and the small hours belong to the evening that opened it ([db.rs:324-328](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L324)) |
| `parse_minute` | [db.rs:355](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L355) | `HH:MM` → minute of day, rejecting `hours >= 24` / `minutes >= 60` |
| `local_clock` | [db.rs:373](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L373) | Monday-first weekday and minute of day **from SQLite**: `CAST(strftime('%H','now','localtime') …)`, `(strftime('%w','now','localtime') + 6) % 7` — the same clock authority the quota week boundary uses, so the window and the ledgers cannot disagree about when a day ends, and no timezone crate links in |
| `window_open_now` | [db.rs:387](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L387) | the whole app's "may a transfer start now". Short-circuits before asking the clock when the window is off ([db.rs:391](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L391)), and answers **open** for an unreadable row or an unreadable clock ([db.rs:394-396](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L394)) |

Three consumers, and no fourth:

1. **A Start** reads it inside the same lock as the concurrency cap
   ([ipc.rs:691-698](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L691) tuple
   `(max, routes, policy, open)`), and a closed window hands the row back after queueing it:
   `state.engine.push_pending(id); return download_get(state, id)`
   ([ipc.rs:706-709](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L706)). That is the *same* pending queue a
   full cap uses — a closed window needed no second queue.
2. **The queue pump** refuses before it pops ([ipc.rs:852-858](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L852)):
   popping first and then refusing would take an id out of the queue with nowhere to put it back.
3. **The watcher's pass** finds the opening. The 5 s adapter loop keeps `window_was_open`
   (initialised `true`, [lib.rs:336](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L336)) and pumps the queue
   only on the closed→open **edge**, judged outside the changed/unchanged branch
   ([lib.rs:378-396](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L378)).

A write to any of the four keys pumps once more, so a change takes effect without waiting for the next
edge ([ipc.rs:1110-1124](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L1110)) — and because `pump_queue`
itself re-reads the gate, a write that *shuts* the window makes the pump return before it looks at a row.

The renderer has one sentence and one place that writes it:
`windowRuleText()` ([main.ts:2473](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2473)) is used by the
Settings note ([main.ts:2499](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2499)) **and** by the per-download
Schedule pane's policy line ([main.ts:4005](file:///D:/RE/Sucr/app/src/renderer/main.ts#L4005)). The controls
are a `data-toggle` switch, two `<input type="time">` boxes and seven `data-day` chips
([index.html:425](file:///D:/RE/Sucr/app/src/renderer/index.html#L425),
[:435/:438](file:///D:/RE/Sucr/app/src/renderer/index.html#L435),
[:440-447](file:///D:/RE/Sucr/app/src/renderer/index.html#L440)), with the note's initial static content at
[index.html:432](file:///D:/RE/Sucr/app/src/renderer/index.html#L432). Clearing the last chip **clears the
key** instead of storing a `0` ([main.ts:2453-2458](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2453)).

## 3. Rulings — **CHOSEN** under the delegation, not `DECIDED` by the user

Only the user writes `DECIDED`. These nine are this agent's calls, each reversible, each with the
sentence that would overturn it.

| # | ruling | why | what would reopen it |
| --- | --- | --- | --- |
| 1 | Four `settings` keys, not a `schedules` row | one global decision, edited in place, visible in `settings:list` like every other engine-wide switch ([db.rs:285-288](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L285)) | a second, independent window (per-download start) — then the table earns its rows |
| 2 | Mask `0` means **every day**, not **no days** | a user who filtered nothing is not a user who asked for nothing; and the chips all light for it ([db.rs:293-296](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L293)) | evidence that someone wants an empty window as a hold-all |
| 3 | The last chip clears the key rather than writing `"0"` | keeps one meaning in the store; the two readings cannot drift apart ([main.ts:2457](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2457)) | a need to distinguish "cleared" from "never set" |
| 4 | A closed window reuses the pending queue | zero new machinery, and the cap and the window then release rows the same way | a queue that must show *why* it is waiting |
| 5 | The opening is found by the existing 5 s pass, edge-triggered | no new thread, no new timer, and 5 s is far below the granularity the feature claims | a user who needs the first minute to start to the second |
| 6 | `start == stop` means every minute | it is what an enabled switch and two empty boxes say ([db.rs:306-307](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L306)) | a ruling that an empty pair should refuse |
| 7 | Unreadable setting or clock means **open** | the safe answer is the choice the user made by never making one — the same polarity as an unreadable proxy row meaning direct ([db.rs:384-386](file:///D:/RE/Sucr/app/src-tauri/src/db.rs#L384)) | any evidence that a silent refusal is safer than a silent start |
| 8 | Nothing wakes the machine | a wake timer is OS registration machinery this box cannot prove, and the sentence says so out loud ([main.ts:2481](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2481)) | the packaging-identity work that `design/notification-identity.md` is already blocked on |
| 9 | The window's own sentence also reads `ui.close_to_tray`, and says plainly when the app would not be there to open the hours | ruling 8 told the user the machine must be running; it did not tell them the *app* might not be, which `lib.rs:503` makes true whenever Window close action is Quit — and a rule that is stated only in the state where it holds is the same class of defect §8 retired. **Disclosure, not override**: the Quit setting still quits, because that is what the user asked for ([main.ts:2480-2483](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2480)) | start-on-logon, or a quit guard that refuses while a window is registered — either one makes the warning false, and then the sentence must go, not stay and rot |

## 4. What it cost

`predicate:` `stat -c '%s %y'` before and after every relink — never a hash, because the link is not
byte-reproducible (~20 B of PDB GUID / PE timestamp moves at constant size).

- The slice as a whole: **4 573 184 B → 4 579 328 B = +6 144 B**, twelve 512 B quanta.
- Headroom under the 5 MB ceiling the user ruled on 2026-11-04: **663 552 B**.
- The step-4b sentence fix (§8) relinked at an unchanged 4 579 328 B. That size reading is the relink
  measurement from the working transcript; what is independently proved here is the *contents*: in the
  shipped exe `grep -a -o` gives "No global schedule window" **0**, "Queued links start between" **1**,
  "stays Queued and starts on its own" **1**. Its fourth probe, "Nothing wakes the machine", read **1** on
  that build and **2** on the current one — ruling 9 added a second branch that begins with the same words —
  so the count below is the one to quote (`writer:` this record, run against
  `src-tauri/target/release/sucr.exe`, 4 579 328 B, mtime 2026-11-06 04:46:08 +0530).
- Ruling 9's sentence: **0 B**. `stat` reads 4 579 328 B before and after the relink, and the renderer text
  is embedded, so the only proof that matters is the contents, measured on the final exe:
  "the app ends when you close its last window" **1**, "and Window close action is set to Quit" **1**,
  "so these hours never open" **1**, "it has to be running" **2** (one per branch), "Nothing wakes the
  machine" **2**; and the retracted mid-draft wording, "closing this window ends Sucr", **0** — the sentence
  that shipped is not the sentence that was first written (§11).
- Gates on that same binary, all run after the launched suite and with the exe untouched (`stat` unchanged
  in both size and mtime): `npm run typecheck` clean; `cargo test --release -- --test-threads=1` →
  **214 passed, 0 failed, 6 ignored** (7 above the 207 recorded in `design/settings-coverage.md` — the
  window's validation, mask, overnight-wrap and clock tests); `node scripts/verify-m0.mjs` →
  **M0 GATE: PASS**, schema v4, 17 tables.

## 5. Proof in the launched exe

`writer:` `D:\RE\_cap-harness\window-run.mjs` (13 legs), report
`window-report-2026-11-06T00-27-38-759Z.json` — **13 legs / 134 checks, `allLegsPass: true`**, exe
`bytes.exe: 4579328` recorded by the report itself, fixture 8 388 608 B at 262 144 B/s on 127.0.0.1:8141.
Clock at run time, from the app's own `local_clock`: `{weekdayMondayFirst: 4, minuteOfDay: 357,
zone: "Asia/Calcutta"}` — i.e. the run happened at about 05:57 local. Each leg boots its **own**
instance with its own `--data-dir`, WebView2 dir, out dir and CDP port (allocated from 9421; this run
reached 9433, because `quit_hours` boots a second instance on its own port and the counterfactual leg boots
two); the user's running app was never attached to, and no leg wrote outside its own out dir.
The suite ran 4 m 41 s for thirteen launched boots.
*The 10-leg / 97-check predecessor is `window-report-2026-11-05T23-16-18-936Z.json` on the same 4 579 328 B
exe; the three close-lifecycle legs below were added to it afterwards and spent no product bytes.*

| leg | checks | the witness that matters |
| --- | --- | --- |
| `controls` | 12 | one switch, two time boxes, seven chips and a note in the shipped DOM; a fresh profile reads **off**, not as an empty verdict; presses store `false`/`true`; the note repeats the two times it just stored; two chips clicked store mask `17`; clearing every chip **clears the key** |
| `schedule_pane` | 11 | real clicks — Add bar → card → Schedule tab — open the pane; off-state sentence; **the false sentence is gone from the shipped surface**; the pane carries the stored `02:30`/`06:00` pair; "Nothing wakes the machine"; pane text contains the Settings note, so it is one sentence and not two that can drift |
| `close_quits` | 15 | ruling 9's both-branch witness, on real controls: a **fresh profile** (key absent ⇒ Minimize) does not claim a quit; the Appearance pillar's Quit button is clicked and `aria-pressed` and the stored `false` both move; with Quit the note names the consequence ("…Window close action is set to Quit — the app ends when you close its last window, so these hours never open") **and still states the hours and the wake rule**, so the warning did not eat the sentence; `#set-close-note` on the other pillar reads "closing the window quits the app" and contradicts nothing; the row's Schedule pane carries the consequence **word for word** (`has: true`); then the real Minimize button is clicked and the warning is gone from the note *and* from the pane, so neither surface keeps a stale one (`writer:` the two `note`/`pane` fields quoted in the report) |
| `shut_then_open` | 11 | the differential: whole day `00:00`–`23:59` owned by one weekday two days out (`stored: 64`), a Start returns `queued` and puts **0 requests** on the host, holds across two watcher passes with no write in between (`bytesOnDisk: -1`), then the identical row starts with no Start pressed after every chip is cleared through the real control (`ms: 1, hits: 1`) |
| `add_bar_promise` | 9 | the panel's own promise, tested: added outside the hours it is `queued` with `hits: 0`, the Add bar shows this run's folder rather than the user's Downloads, and once the chips are cleared it starts on its own with the Add bar never touched again |
| `disabled_inert` | 4 | the switch off over the top of a shut configuration lets the identical row run and asks the host — the hours are not in force without the switch |
| `clock_edge_wake` | 6 | a window opened two minutes ahead is stored as shut now; the Start before the boundary holds it; it starts **by itself** once the clock passes the opening minute (`wokeAt: 02:56:4`, `hits: 9`), the wake followed **no settings write at all**, the release came **3 189 ms** after the boundary epoch and the host was asked only after that |
| `overnight_tail` | 9 | `03:26`→`03:04` stored as a wrapping pair with ≥5 min of runway left, judged on the **opening evening** mask (yesterday only) it gets on the wire; flipped to today only, the same minute is outside the window and the row is *held*, proved by `hits: 0` on a second fixture rather than by a status; a window that has just closed does not halt the job already running (`status: finished`) |
| `boundary` | 15 | the refusals at the real write path: `"128"`, `"255"`, `"-1"`, `"abc"` refused for days; `"0"` and `"127"` accepted; `"25:00"`, `"12:70"`, `"noon"` refused for times; `"07:05"`, `"00:00"` accepted; `"yes"` refused, `"true"`/`"false"` accepted; a null write clears the key |
| `counterfactual_previous_build` | 5 | on `sucr-old-4567552.exe` (4 567 552 B) all four keys are refused with `unknown setting key: engine.window_enabled`, its Settings page has no window box to type into, and the same whole-day-plus-one-weekday intent **cannot** hold a row there — it runs, and the old build really asked the host (`hits > 0`) |
| `close_minimize` | 14 | **the process-outcome half of `ui.close_to_tray`, on its default branch.** A fresh profile stores nothing (`stored: null`), so the engine itself reads Minimize; a row capped to 256 KiB/s and Started is caught mid-`downloading`, then `WM_CLOSE` is posted to *this instance's own* window: the window manager then reports `visible=False iconic=False` — hidden, not minimized — `tasklist` still lists `sucr.exe` (pid 26 176, 32 200 K), the same CDP target answers `1 + 1 === 2`, a **fresh** attach reports `visibilityState === 'visible'`, `settings.list()` returns the sentinel written minutes earlier, and the transfer keeps arriving: the fixture had served **1 B** when `WM_CLOSE` was posted, **2 097 153 B** by the time the hidden state was read and **6 815 745 B** after it, then `finished` at 25 901 ms with all 8 388 608 B on disk. `SW_SHOW` on the same hwnd returns `visible=True` — the internal control that the leg moved the app's handle and not its own |
| `close_quit_ends` | 7 | **the other branch of the same message.** The real Appearance seg-button is pressed to Quit (`aria-pressed=true`, `'false'` stored); `child.once('exit')` is armed **before** the `WM_CLOSE` post, so the exit is the app's own: `{code: 0, signal: null}` at **1 935 ms**, pid gone 179 ms later, the window helper then fails `NO_TAURI_WINDOW_FOR_PID` (destroyed, not hidden), the instance's CDP port is released and its socket errors |
| `quit_hours_never_open` | 16 | **ruling 9's consequence, followed across a process boundary — and it found a limit, not a pass.** Held by day mask, a Start returns `queued` with `hits: 0` across two watcher passes and nothing on disk (`bytesOnDisk: -1`); the shipped sentence is the one under test; the app is closed on Quit (`code 0` at 1 923 ms, pid gone 179 ms later) and the still-alive fixture is asked for **nothing** for another 11 s. The same data dir is then relaunched: the row is still `queued` and `'false'` is still stored. **Control** — a row Started *in the new process* under the same shut window reaches `downloading` at `ms: 0` with the host asked as soon as every chip is cleared. **Survivor** — the row the closed process was holding is still `queued`, still `hits: 0`, after that same opening and two more watcher passes. **Revived** — Started again here, the identical row runs immediately (`then: 'downloading'`, `hits: 9`). See § 7 |

**One honesty note about "the real control"**, because the ladder in `design/settings-coverage.md` §1 is
defined by it. `toggleKey` clicks the checkbox and `setDaysMask` clicks the chips, always. `setTime` clicks
nothing: it writes the box's `value` and dispatches `change`, which *is* the page's own commit path, but when
the stored key already equals what the leg wants the page's commit returns early, so the helper writes through
the bridge instead and reports `via: 'bridge (already equal)'`. A leg reading that field is not an L2c witness.
No claim in §5 depends on one: every time-box check below runs on a fresh profile whose key is unset, so the
value always differed.

## 6. Two asymmetries this slice measured, because both can be misread


- **An accepted Start and a refused Start answer differently.** A refusal comes back `queued` *synchronously*,
  because the row is untouched and only handed to the pending queue
  ([ipc.rs:706-708](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L706)). An accept comes back `queued` too and
  only becomes `downloading` when the job thread writes it — measured here at **251 ms** after the call
  (`overnight_tail`, `{start_status: "queued", status: "downloading", ms: 251}`), the same lag the interim
  runs measured at 252 ms. **Therefore a synchronous `queued` proves refusal only in combination with a
  host-request witness.** `predicate:` the fixture's own hit counter. The overnight leg's hold check reads
  `hits: 0` on a *second* URL for exactly this reason; the status alone would be worth nothing.
- **A write-pump and a clock edge are different releases.** Four of the keys pump the queue on write
  ([ipc.rs:1113-1124](file:///D:/RE/Sucr/app/src-tauri/src/ipc.rs#L1113)); the opening that no key caused is
  found only by the watcher's edge pass ([lib.rs:382-392](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L382)).
  `clock_edge_wake` isolates the second one: it asserts no settings write happened between the Start and the
  release, and reports `afterBoundaryMs: 3189`. Its sibling field `releaseMs: 78971` is the harness's own
  elapsed budget from leg setup, **not** a wake latency — reading it as one would be a mistake this record
  is explicitly making room for.

## 7. What this window does not do (absences, each with its predicate)

- **No per-download scheduled start.** `predicate:` `grep -rn "schedule_id\|queue_id" src-tauri/src src/renderer`
  → **0 matches**, re-run today. The `schedules` table (kind, `at_minute`, `days`, `end_date`, `one_time`,
  `stop_minute`, `retries`, `on_done`) still has no writer and no reader, and `queues` no scheduling use.
  Columns `end_date`, `retries`, `on_done` and `one_time` have **no equivalent in the shipped keys** — an
  expiring window, a retry policy and a run-once start are not implemented, not merely unproven.
- **No wake timer, no start-on-logon** (ruling 8). The app must already be running; the sentence says so.
- **The window does not keep the app alive.** With Window close action on **Quit**, `lib.rs:503` ends the
  process when the last window closes, so the hours the panel is showing never open — the rule and the
  setting cancelled each other and neither surface said so. **Ruling 9 closes that as disclosure, not
  prevention**: the shared sentence now names the consequence while it is true, but the app still quits,
  still exactly as the user set it. A *guard* (refusing to quit while a window is registered) or start-on-
  logon would make the warning false, and both are the user's calls, not this agent's (§10).
  `predicate:` `lib.rs:489-505` read today; `writer:` the `close_quits` leg drives both branches of the
  *renderer*, and `close_minimize` / `close_quit_ends` now drive both branches of the **process** — same
  message, same handle, same build, only the stored key differs.
- **A Start refused by a shut window survives the close, but not the process.** *This is the one place a leg
  disproved what the record was about to claim,* and it is why `quit_hours` is a differential rather than the
  single assertion it was written as. A Start that finds the window shut does not lose the row — it hands it
  to the queue (`ipc.rs:705-707`, the same hand-off a full cap uses). But that queue is
  `Registry.pending`, a `Mutex<VecDeque<i64>>` in this process's memory (`job.rs:3711`), and `pump_queue`
  takes its next row from that deque alone (`ipc.rs:863`). The only code that fills it at boot is
  `reclaim_interrupted`, whose `SELECT` is `status = 'downloading'` (`ipc.rs:811`), so a row the previous
  process left at `queued` arrives in the new one with **no place in the queue**: opening the window releases
  the rows Started there and silently skips the one Started earlier. The row is not lost, and neither is its
  URL — Started again in this process it runs immediately.
  `writer:` `ipc.rs:705-707`, `ipc.rs:863`, `ipc.rs:811`, `job.rs:3711`, all re-read 2026-11-06.
  `predicate:` the fixture's own per-path request counter (`hits`), which is server-side and cannot be
  explained by a status the app reports about itself. `evidence-noun:` the three fields in
  `quit_hours_never_open`'s `notes.boundary` — `survivor {status: 'queued', hits: 0}`, control
  `{status: 'downloading', ms: 0}`, revived `{then: 'downloading', hits: 9}`.
  **What is *not* said about it:** the shipped sentence at
  [main.ts:2484-2486](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2484) promises, in its middle clause,
  "Outside them a link you add stays Queued and **starts on its own**" — true inside one process, false across
  one, in the same paragraph whose last clause is the one that tells the user the process *ends*. So either the
  clause gains that case (renderer prose, ≤ the cost of a sentence inside existing padding), or the queue
  place is persisted (`queued` rows pushed at boot, which contradicts the deliberate reasoning recorded in
  `reclaim_interrupted`'s own doc comment — "a row merely `queued` was added with auto-start off and stays its
  user's to launch", `ipc.rs:799`). **Both are the user's calls, not this agent's**; nothing was changed
  here, and 0 product bytes were spent.
- **Nothing about notifications.** "Quiet hours" for toasts is still a different feature and is still
  blocked on a packaging identity — `design/notification-identity.md`. This window gates *starts*, not alerts.
- **A closing window never stops a running transfer.** The gate is consulted at Start and at pump, and not
  on a tick: `overnight_tail` proves a job that was running when its window closed finished. Whether that is
  right is a ruling nobody has made yet; the shipped behaviour is simply stated here.
- **The pause/resume buttons stay immediate.** A paused row resumed inside a shut window is not this slice's
  subject; `resume` was not measured in any leg (`unmeasured`, not absent).
- **A third copy of the off-sentence exists**: the static `#set-window-note` content in
  [index.html:432](file:///D:/RE/Sucr/app/src/renderer/index.html#L432) and the JS string in
  [main.ts:2474](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2474) — two bytes' worth of drift risk that the
  `controls` leg covers for a fresh profile ("a fresh profile reads the window as off") but that no leg
  compares against future edits. Held, 0 B owed.
- **`#set-window-note` has a second writer** — the refuse path of `commitWindowTime`
  ([main.ts:2436](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2436)), which replaces the note with
  `Not saved: …` after a rejected time string. That is a refusal message, not a competing copy of the rule,
  and §2's "one place that writes the sentence" means the sentence. It is listed so the next reader does not
  find it by surprise.

## 8. The sentence this slice made false — retracted everywhere it travelled

The per-download Schedule pane used to assert, in the interface: **"No global schedule window or wake timer
is configured in this build."** That stopped being true when §2 shipped, and nothing read it, so it sat there
claiming the feature was absent. Step 4b replaced the hardcoded line with the shared
`windowRuleText()` ([main.ts:4005](file:///D:/RE/Sucr/app/src/renderer/main.ts#L4005) — the pane's call site; the
function itself is `main.ts:2473`) — the same function
the Settings note calls, at **0 B** (§4) — and the `schedule_pane` leg now asserts both halves of the
replacement in the shipped exe.

Where the claim had travelled, and what is now corrected:

- `app/docs/designs/notifications-autostart-concepts_brief.md:23` — "**there is no scheduler to defend**".
  Half false: a global start window now exists and is proven. The other half of that sentence, the
  `schedules` table's deadness, still holds (§7).
- The same brief's **blocker 5** (":115-124") quoted the interface sentence as evidence, citing
  `main.ts:3507`. That locator has drifted **twice** — to 3996, then to **`main.ts:4005`**, a **+9-line** shift
  since the 3996 reading. The earlier draft of this bullet said "five lines", which was never measured, and the
  split between §11's reword and ruling 9's clause cannot be recovered from the current source — so the record
  keeps only the two numbers it can still prove. `predicate:` `grep -n "windowRuleText"` → 2473 (definition),
  2499 (Settings note), 4005 (Schedule pane). The quoted string itself is gone from the
  source and from the binary. Blocker 5's remaining, surviving claim — the table is defined and has zero
  references in Rust — is still true and still re-checked by grep.
- `design/app-design-plan-v2-engine.md`'s "per-download schedule" entry is a **reconstruction** of the old
  plan, not a live claim about the build; §7 is the current truth.
- **And one claim this slice made false in the other records' locators, found on the same sweep.** Eight cites
  across five files (four memory files plus the auth slice's own `design/proxy-and-http-auth.md`, at `:947` and
  `:1300`) cited `main.ts:2473` for **`resetAllSettings`** — correct when step 8 wrote it on 2026-11-05. `windowRuleText()`
  was inserted at that line, so today `:2473` is a *different live function* and `resetAllSettings` is
  **`main.ts:2563`** (+90; its click handler `:2880-2881` → `:2977-2978`). Every one of those cites is corrected
  to the name plus the live number. `predicate:` `grep -n "function resetAllSettings\|function windowRuleText"
  src/renderer/main.ts`. **The lesson generalises the drift rule from §5 of `design/settings-coverage.md`: a stale
  line number is not merely off, it can be occupied — and it only reads as valid because the sentence beside it
  names a function, so always cite the name.**

## 9. Harness defects found on the way (both fixed)

- `killTree` passed `'//PID'`. Git Bash escaping does not apply to `spawnSync`, so taskkill saw an invalid
  option, failed silently, and **every leg of every earlier run left a whole app instance resident**. Fixed
  in `window-run.mjs` and `settings-run.mjs` (single slash, non-zero status logged). `predicate:` the run
  dirs delete, and `tasklist //FI "IMAGENAME eq sucr.exe"` → "INFO: No tasks are running which match the
  specified criteria." after the 10-leg suite. Some `kill … status 128/255` log lines remain, and they are the
  child of a process that already exited — the fix is judged by the leftover count, not by the absence of
  error text.
- **`download:add` over the bridge leaves the renderer's library empty.** It writes the row and emits no
  `download:changed`, and the renderer's `refresh()` is event-driven — the listener at
  [main.ts:6019](file:///D:/RE/Sucr/app/src/renderer/main.ts#L6019) calls `scheduleRefresh()`
  ([main.ts:4413](file:///D:/RE/Sucr/app/src/renderer/main.ts#L4413)), which is what debounces into a
  refresh — so a leg that bridges a row in and then
  clicks a card sees `cards: 0`. Fixed by a new helper, `addRowViaBar`, which drives the real Add bar. This
  is the same rung the settings sweep named **L2c** — a harness that clicks the real control — and the
  `schedule_pane` leg now needs it for a reason nobody predicted. (This locator was stale in this record at
  `6010` before anyone here touched the file; re-read today, it is `6019`. A locator is not evidence until
  its lines are re-read.)

## 10. What is next

Nothing is owed by this slice: the window ships, is proven in the launched exe, its false neighbour is
retracted (§8), and the rule that was inert under a Quit setting is at least true on screen (§7, ruling 9).
The scheduling work that remains is a **separate decision**, in rough order of what it costs:

- **Whether the app should be kept alive for the hours it advertises.** This slice disclosed the
  contradiction; it did not resolve it. A start-on-logon, or a quit guard that refuses to exit while a
  window is registered, are both behaviour changes with the user's name on them, and either one makes
  ruling 9's warning false — in which case the sentence has to be removed, not left to rot like the one in
  §8. Not this agent's call.
- **Per-download start** (needs a `schedules` writer, a card surface, and a queue that can hold rows for
  reasons other than "cap"), the three unread `schedules` columns above, and a **wake timer**, which is
  blocked behind the same packaging identity as notifications.
- Whether the closing window should halt a running job (§7) is a
  ruling the user has not been asked for and this agent has not made.

## 11. The sentence this slice wrote twice

The first version of ruling 9 ended its enabled sentence with **"Nothing wakes the machine: it has to be
running, and closing this window ends Sucr."** It was built and relaunched before anyone read it in the app's
own voice, and in the Settings panel **"this window" already names something else** — the Download window
row above it, or the Settings page itself. Two sentences earlier the same paragraph is talking about the
hours, so the clause read as if the *window with the hours in it* were the thing that ended Sucr. The
consequence was right and the referent was not.

Before it shipped, it became **"the app ends when you close its last window"** — subject named, no
antecedent to borrow ([main.ts:2483](file:///D:/RE/Sucr/app/src/renderer/main.ts#L2483)). That is a
reword, not a behaviour change, and the proof it is only a reword is §4: the relink landed at an unchanged
4 579 328 B, the old wording probes to **0 hits in the binary**, the new wording to 1, and `close_quits`
re-ran green against the final exe. Nothing in §2–§7 describes the discarded sentence.

**Why this is in the record at all:** the defect class is the one §8 retired — a sentence in the interface
that says something false. It caught this slice's own first draft, one relink before the user saw it, and
the only thing that caught it was reading the shipped text in the shipped surface instead of in the diff.
`predicate:` the two `grep -a -o` counts above, on the same 4 579 328 B exe the suite ran against.
