# Debug log (shipped 2026-11-10)

A file in `logs/` beside the database, written by the app itself: what started, what
an IPC call refused, a job that failed before it started, a capture door that could
not bind, and a panic (a hook is installed at boot; `panic = "abort"` still runs it).
No crate, no logging framework, no network path: one sink, one line per event.

Shipped **on by default**: the switch reads `system.debug_log`, and anything that is
not the literal `false` — including an absent key — counts as on. Only an explicit
off in Settings stops the writes.

## Shape

- `src-tauri/src/debug_log.rs` — sink, 1 MiB roll to `sucr.previous.log`, UTC stamps
  from a 12-line civil-date conversion (no date crate), `tail`, `bytes`, `truncate`.
- Boot (`lib.rs`): read the setting, `switch()`, `install_hook()`, write the
  `started (pid …) on <data dir>; debug log on|off` line.
- `sucr_invoke`: every refused channel writes `ERROR <channel> refused: <message>`.
  Args are deliberately not logged — they can carry a pasted session cookie.
- `engine/job.rs` — a job that fails before start; `capture.rs` — a door that cannot bind.
- `ipc.rs` — `debug:report` (`state` | `open` | `clear`) and the `system.debug_log`
  key (validated boolean: only `true`/`false`; `on`, `off`, `yes`, `FALSE` refused).
- Settings → System → Debug log card: toggle, file path, Open log folder, Clear log,
  a 40-line tail, and a note with the byte count. `Open log folder` reuses the
  already-proven `pick::open` path.

## Cost

Exe `stat` **4 755 968 B** (+11 776 B over 4 744 192), headroom 486 912 B under the
5 242 880 ceiling. 38 channels, 26 setting keys. Suite **245 passed / 0 failed /
8 ignored**; `verify-m0.mjs` PASS at 38 channels.

## Legs (launched `target/release/sucr.exe`, `--data-dir=` scratch, CDP 9333)

1. Fresh data dir, no setting: `logs/sucr.log` exists and holds the `started` line.
2. Card reads back: toggle checked, real path, note "Writing. 116 B so far…", tail.
3. Refused `settings:set` with the switch on → `ERROR settings:set refused: …`, 116 → 196 B.
4. Toggle off in the UI → stored value `false`, `enabled:false`, note "Writing is off.";
   another refusal wrote nothing (196 → 196 B).
5. Toggle on → `debug log switched on in Settings`, then a refusal at 338 B.
6. Clear → file truncated to the single `log cleared from Settings` line (53 B), tail
   and note update.
7. Restart with the key `true` → `started … debug log on` appended.
8. Restart with the key `false` → file byte-identical before and after (168 → 168 B).

`design/shots-debug-log/settings-system.png` is the card as launched.

## 0.1.1: Report a problem

The same `debug:report` channel gained a fourth action, `issue` — no new channel, no token,
no telemetry. It builds a GitHub new-issue URL (title `Sucr <version> bug report`; body with
two blank prompts, the build's version/OS/arch, and the last 40 log lines capped at 2400
chars and marked with an ellipsis when cut), percent-encodes every byte outside the
unreserved set, and hands it to the browser through the same `updates::open` /
`ShellExecuteW` path as the update button. Nothing is posted until Submit is pressed on
GitHub. Target repo is the private `benjaminspoop7-sys/sucr` (issues visible to the owner).

- **Cost:** exe `stat` **4 758 528 B** (+2 560 B over 4 755 968), headroom 484 352 B.
  Installer `Sucr_0.1.1_x64-setup.exe` **1 959 787 B**. Suite 246 passed / 0 failed /
  8 ignored; M0 gate PASS.
- **Test:** `an_issue_link_carries_the_log_inside_a_web_address` — host, title encoding,
  a log with `\`, `"` and a newline, unreserved characters left alone, and a 5000-char
  log cut rather than refused.
- **Launched legs (0.1.1 over CDP):** the card renders the button; a click resolved
  `{ok:true, url:"https://github.com/benjaminspoop7-sys/sucr/issues/new?title=Sucr%200.1.1…"}`
  and the note switched to the Submit explanation; a new Zen process started at the click
  (`StartTime 10:44:12`), which is the browser opening the page. Live update check from
  0.1.1 against the real feed: `current 0.1.1, latest v0.1.1, available false`.
- **Not witnessed:** the GitHub page itself (sign-in and rendering are the browser's),
  and the newer-version leg — still needs a build older than the newest tag; no installed
  copy of 0.1.0 exists on this machine to run it.
