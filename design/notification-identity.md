# Notification packaging identity — step 0: the preconditions are not met on this box

Record of the measurement taken **before** any product byte was spent on Windows toast notifications.
Written 2026-11-05 under the standing delegation, on the user's instruction to continue with the item
that the completion-chime slice left at the front of the held queue
(`project-sucr-open-defects.md` § *How to apply*; `design/completion-chime.md` § 7).

The instruction was to continue with the item, not to ship it. This step decided whether shipping it is
**measurable here**. It is not, for two independent reasons, both now measured.

**Cost of this step: 0 B.** No product file was modified. `sucr.exe` is still **4,569,600 B**
(`stat`, mtime `2026-11-05 06:48:25 +0530`), and every source file predates it — checked with
`stat` on `src/renderer/*`, `src/shared/*`, `src-tauri/src/**`, `scripts/verify-m0.mjs`.
All probes were read-only: they read Start Menu `.lnk` properties and `HKCU` values, wrote nothing, and
created no shortcut and no registry key.

---

## 1. What the repository actually declares — `writer:` grep, with locators

| Fact | Locator |
| --- | --- |
| `"bundle": { "active": false, "icon": [] }` — **no bundler is configured at all** | [app/src-tauri/tauri.conf.json:26](file:///D:/RE/Sucr/app/src-tauri/tauri.conf.json#L26) |
| The identity string a bundler would use **does** exist: `"identifier": "app.sucr"`, `"productName": "Sucr"` | [app/src-tauri/tauri.conf.json:3-5](file:///D:/RE/Sucr/app/src-tauri/tauri.conf.json#L3) |
| `windows 0.62` is already a direct dependency and already opts `Win32_System_Com`, `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`, `Win32_Foundation` | [app/src-tauri/Cargo.toml:43-48](file:///D:/RE/Sucr/app/src-tauri/Cargo.toml#L43) |
| No notification dependency of any kind — `tauri` is `default-features = false, features = ["wry","tray-icon"]`, and there is no `tauri-plugin-notification` | [app/src-tauri/Cargo.toml:29](file:///D:/RE/Sucr/app/src-tauri/Cargo.toml#L29) |
| The tray tooltip is static: `.tooltip("Sucr")` | [app/src-tauri/src/lib.rs:274](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L274) |

**Correction to the premise this item inherited — plus a correction to this record's own first count of it,
caught by re-reading the cited feature list.** The defect record said the toast path needs
`Win32_UI_Notifications` + `Data_Xml_XDom` + `Win32_System_Com`. **One** of those three is *already opted
in* (`Win32_System_Com`), not two: the four features at
[Cargo.toml:43-48](file:///D:/RE/Sucr/app/src-tauri/Cargo.toml#L43) are `Win32_Foundation`,
`Win32_System_Com`, `Win32_UI_Shell` and `Win32_UI_WindowsAndMessaging`, and `Win32_UI_Shell` is the one I
was miscounting against the toast list. What is actually new is
`Win32_UI_Notifications` and `Win32_Data_Xml_Dom` — and the bigger new thing, which the record never
named, is that **`Win32_UI_Shell` being opted in already does not register an identity; registration is done
by a shell extension property on a Start Menu shortcut, which is an install artefact, not a call.**

## 2. Method, and the harness trap that had to be fixed first

Probes ran through `powershell.exe` against the inbox WinRT projection
(`C:\WINDOWS\system32\WinMetadata\Windows.UI.winmd`), which is what any script-driven identity test on
this box must use. Scripts live in `D:\RE\_cap-harness\toast-probe.ps1`, `toast-probe2.ps1`, `aumid-cf.ps1`.

**Trap, recorded because it produced a false reading.** The first version of the probe wrapped the
notifier call in a PowerShell function that also wrote log lines. A PowerShell function returns **all**
pipeline output, so the log string became the first element of the return value and `$notifier.Show()`
was called on a `System.String` — the run reported
`Method invocation failed because [System.String] does not contain a method named 'Show'`, which reads
like an OS rejection and is nothing of the kind. The second version accumulates into a script-scoped
list and emits it only at the end. **A witness that reports the harness's own type confusion as a
result is the `evidence-noun:` failure mode, in my harness rather than in the app.**

Also measured, and a constraint on any future PS-based proof: **this build's inbox projection does not
expose `CreateToastNotifierWithId` or `NotificationUsageState` at all** (`has_CreateToastNotifierWithId=False`,
`has_NotificationUsageState=False`). The only surface available to a script is
`CreateToastNotifier()` and `CreateToastNotifier(String)`.

## 3. What was measured

**3a. A process with no registered identity cannot even get a notifier.**
`ToastNotificationManager.CreateToastNotifier()` (no argument — the current process's identity) throws
**`Element not found. (Exception from HRESULT: 0x80070490)`**. That is the packaging gap in the OS's own
error code: `sucr.exe` run loose from `target/release` has no identity the notification subsystem can
resolve. `— launched probe, D:\RE\_cap-harness\toast-probe2.ps1`

**3b. …but passing *any* AUMID string is not the test.** `CreateToastNotifier('app.sucr.probe.unregistered')`
returned a `ToastNotifier` object **successfully** — an id that exists nowhere on this machine. So object
creation cannot distinguish registered from unregistered, and any implementation that treats "the call
returned" as "we have an identity" is wrong on day one. `— launched probe`

**3c. `ToastNotifier.Setting` measures nothing on this box.** Every AUMID probed — the unregistered
synthetic, `app.sucr` itself, **and six real registered ones read out of the machine's own Start Menu
shortcuts** (`Microsoft.Windows.AdministrativeTools`, `Microsoft.Windows.Explorer`,
`electron.app.Antigravity`, `com.squirrel.Discord.Discord`, `com.deepseek.dsh`, `com.freebuff.desktop`) —
returned **`DisabledForUser`**. A measurement that is identical for the control group and the test group
is not a measurement. `— launched probe + counterfactual on six registered ids`

**3d. Why: toasts are switched off machine-wide, by the user, at the OS level.**
`HKCU\Software\Microsoft\Windows\CurrentVersion\PushNotifications\ToastEnabled = 0` (Int32) and
`HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\NOC_GLOBAL_SETTING_TOASTS_ENABLED = 0`
(Int32). Only **2** subkeys exist under `Notifications\Settings`, so this machine's notification
subsystem has recorded almost no per-app state at all. `— read-only registry probe`

**3e. `Show()` is not an oracle — it succeeded while the OS was refusing everything.**
With `ToastEnabled = 0`, `notifier.Show(toast)` for the unregistered id **returned without exception**.
Nothing was verified to have appeared on screen, and nothing could be: the Action Centre store is a
user-profile read this harness does not make. So the honest statement is the one the chime slice already
taught — **raised is not heard** — now with a mechanism: the WinRT toast API accepts and drops. Any Sucr
code that reports "notification sent" from the success of the call would be lying on every machine where
the user has toasts off, which is this one. `— launched probe`

**3f. The packaging toolchain is not on this box either.** `tauri-cli 2.12.4` is installed
(`npx tauri --version`), but there is **no NSIS and no WiX**: `C:\Program Files\NSIS`,
`C:\Program Files (x86)\NSIS`, `C:\Program Files (x86)\WiX Toolset*` are absent and
`makensis.exe` / `candle.exe` / `light.exe` resolve to nothing on `PATH`. So `bundle.active: true` would
not produce an installer here without installing a toolchain first — an act outside this project.
`— filesystem + PATH probe`

> **CORRECTION 2026-11-07 — the observation stands, the conclusion is retracted.** Those paths are still
> empty and `makensis.exe` is still not on PATH. But the Tauri CLI **fetches its own NSIS**: measured,
> `tauri build --bundles nsis` downloaded `nsis-3.13.zip` and `nsis_tauri_utils.dll` from the tauri-apps
> GitHub releases, validated both hashes, cached them under `%LOCALAPPDATA%\tauri\NSIS\` and produced
> `Sucr_0.1.0_x64-setup.exe` at **1 866 169 B**, with the embedded exe at **4 593 152 B** — the same size
> as the shipped one, so bundling costs **0 B** of exe. "Would not produce an installer here without
> installing a toolchain first" is false and is withdrawn wherever it travelled (§ 4 reason 2, § 5
> ruling 2, the concepts brief, and two memory entries). WiX is still absent, so MSI is still unavailable
> and still unneeded. `writer:` the capture step-0 probe · `predicate:` a failed NSIS fetch or a missing
> setup exe would have falsified it · `evidence-noun:`
> [capture-step0-spikes.md](capture-step0-spikes.md) § 1, `D:\RE\_cap-harness\bundle-probe.log`.
> **What is still blocked:** reason 1 below (`ToastEnabled = 0`) is untouched, and producing an installer
> is not the same as *running* one — the AUMID only exists after an install, which writes the user's
> registry and remains theirs to authorize.

## 4. What is therefore blocked, and what deliberately was not measured

**Blocked (two independent reasons, either sufficient):**
1. **No proof is possible on this box while `ToastEnabled = 0`.** Flipping it is a change to the user's
   OS configuration, not to the app, and it is theirs to make or refuse. Until then, a launched-exe leg
   for a toast can only ever assert that `Show()` did not throw — which 3e just proved is not evidence.
2. **No installed identity is reachable without an installer toolchain that is not installed** (3f), and
   the alternative dev-time route — writing a Start Menu `.lnk` with `System.AppUserModel.ID` set, or an
   `HKCU\Software\Classes\AppUserModelId` key — writes into the user's profile and registry. This harness
   does not do that unasked.
   **CORRECTED 2026-11-07:** the toolchain half of this reason is withdrawn (see 3f) — an NSIS installer
   **can** be produced here, at 0 B of exe cost. What survives is the second half: an identity exists only
   after something *writes it*, whether that is a real install or a Start-Menu/registry write, and both are
   the user's to authorize. So this reason is now **one** reason, not two, and reason 1 (`ToastEnabled = 0`)
   is still sufficient on its own.

**Deliberately not measured: the byte cost of the two new `windows` features.** Pricing it properly means
writing Rust that *uses* `Win32_UI_Notifications` and `Win32_Data_Xml_Dom` in the app, and per
[[project-sucr-gate-mechanics]] an inert parameter measures a misleading **0 B** — a spike here would
report "free" for a path that cannot be proven. So the number stays **`unmeasured`**, and the admission
rule says what that costs it: it cannot lead a queue and cannot become a premise. It is recorded as a
question to be priced **after** the identity route is ruled, not before.

## 5. The rulings this item needs, in the order they unblock each other

None of these are mine. **Only the user writes `DECIDED`.**

1. **Is the OS switch to be turned on for testing?** (`ToastEnabled` / global toasts on this machine.) If
   yes, the identity measurement in §3 can be re-run and a real launched-exe leg becomes possible. If no,
   toasts are untestable here and the item should be re-ranked behind things this box can prove.
2. **Is an installed identity in scope at all**, given `bundle.active: false` and no NSIS/WiX? Three
   coherent answers: (a) adopt a real bundler and accept installing a toolchain; (b) register an AUMID at
   first run by writing a shortcut into the user's Start Menu — an explicit, visible act the app should
   ask permission for; (c) **do not do toasts**, and ship the cue that needs no identity.
   **CORRECTED 2026-11-07:** option (a) no longer requires installing a toolchain — the CLI fetches NSIS
   itself and the installer was produced (3f). The user has since ruled **bundling on acceptable as a
   prerequisite** (2026-11-07, browser-capture ask 4), so (a) is now the cheapest route and (b)'s
   profile write is avoidable. This ruling is still the user's to make for *notifications* specifically:
   accepting bundling for capture is not an instruction to install Sucr on this machine.
3. **If (c): the tray tooltip** ([lib.rs:274](file:///D:/RE/Sucr/app/src-tauri/src/lib.rs#L274) is a static
   `"Sucr"`) is the only completion cue on this machine that is **provable today** — it does not consult
   `ToastEnabled`, and the tray is built by `setup()` on the real path, which the `--smoke` gate never
   reaches (`design/completion-chime.md` § 2, and `project-sucr-open-defects.md` on the gate's blind spot).
   It is also the one that cannot mislead: a tooltip says nothing about a delivery the OS refused.

## 6. Limits of this record

- **Nothing was observed on screen.** Not a toast, and not its absence. §3e is a statement about the API
  returning, not about the UI rendering — the same distinction the chime slice had to make.
- **PowerShell was the instrument, not the app.** The `windows` 0.62 Rust projection carries its own
  metadata and *does* expose `CreateToastNotifierWithId`; a Rust probe could reach APIs this box's inbox
  winmd cannot (§ 2). Any conclusion about an API **only** available through that projection is
  `unmeasured` here. The four findings above do not depend on it.
- **The global switch is the user's setting, not a Windows defect.** A machine with toasts on may behave
  differently for §3c and §3e; both were measured under `ToastEnabled = 0` and are re-runnable in minutes
  if ruling 1 changes it.
- **Read-only was maintained.** Six `.lnk` properties and three `HKCU` values were read; nothing was
  written to the profile, the Start Menu or the registry, and no window other than a harness instance's
  own was touched — there is no launched Sucr in this record at all.

Related: [[project-sucr-open-defects]], [[project-sucr-egm-design-cycle]],
[[project-sucr-gate-mechanics]], [[windows-machine-crash-telemetry-off]], `design/completion-chime.md`.
