use crate::AppState;
use crate::engine::{self, job};
use rusqlite::Connection;
use serde_json::{json, Value};
use tauri::AppHandle;

/// The schema's channel list. shared/ipc.ts is the TypeScript half; the two are
/// compared at boot (app:get-info returns this list) so drift is caught by the
/// smoke test rather than by a missing command at runtime.
pub const CHANNELS: [&str; 38] = [
    "app:get-info",
    "app:get-folder-presets",
    "app:folder-state",
    "app:create-folder",
    "app:get-paths",
    "app:quit",
    "app:pick-folder",
    "download:list",
    "download:get",
    "download:add",
    "download:start",
    "download:pause",
    "download:remove",
    "download:open",
    "download:reveal",
    "download:map",
    "download:watch",
    "download:unwatch",
    "download:refresh-url",
    "download:probe",
    "network:list",
    "network:set-enabled",
    "network:set-limits",
    "proxy:set-auth",
    "origin:set-auth",
    "category:list",
    "history:tail",
    "download:set-category",
    "download:set-speed-limit",
    "app:set-speed-limit",
    "settings:list",
    "settings:set",
    "session:list",
    "session:set",
    "session:forget",
    "addon:setup",
    "update:check",
    "debug:report",
];

pub fn dispatch(
    channel: &str,
    args: &[Value],
    state: &AppState,
    app: &AppHandle,
) -> Result<Value, String> {
    match channel {
        "app:get-info" => app_info(state),
        // The add bar's quick folders. Answers with the ones that exist on disk,
        // so a chip never points at a folder the machine does not have.
        "app:get-folder-presets" => {
            let base = args.first().and_then(|v| v.as_str()).unwrap_or("");
            serde_json::to_value(crate::paths::folder_presets(base))
                .map_err(|e| format!("preset serialisation failed: {e}"))
        }
        // Two halves of one affordance: is this folder there, and if not, make it.
        // Read-only and cheap, so --smoke runs it like any other channel.
        "app:folder-state" => {
            let raw = args.first().and_then(|v| v.as_str()).unwrap_or("");
            match crate::paths::folder_state(raw) {
                Some(state) => serde_json::to_value(state)
                    .map_err(|e| format!("folder state serialisation failed: {e}")),
                None => Ok(Value::Null),
            }
        }
        "app:create-folder" => {
            let raw = args.first().and_then(|v| v.as_str()).unwrap_or("");
            serde_json::to_value(crate::paths::create_folder(raw))
                .map_err(|e| format!("create folder serialisation failed: {e}"))
        }
        "app:get-paths" => Ok(json!({
            "dataDir": state.paths.data_dir,
            "dbPath": state.paths.db_path,
            "logsDir": state.paths.logs_dir,
            "downloadsDir": state.paths.downloads_dir,
        })),
        "app:quit" => {
            app.exit(0);
            Ok(Value::Null)
        }
        // Blocking by nature: the shell dialog runs its own modal loop and
        // answers once the user closes it. Under --smoke it is refused rather
        // than opened, because a modal nobody dismisses hangs the gate.
        "app:pick-folder" => {
            if state.smoke {
                return Ok(Value::Null);
            }
            Ok(match crate::pick::folder() {
                Some(path) => Value::String(path),
                None => Value::Null,
            })
        }
        "download:list" => {
            let filter = args
                .first()
                .and_then(|v| v.get("status"))
                .and_then(|v| v.as_str());
            download_list(state, filter)
        }
        "download:get" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:get expects [id]".to_string())?;
            download_get(state, id)
        }
        "download:add" => {
            let spec = args.first().cloned().unwrap_or(Value::Null);
            download_add(state, &spec)
        }
        "download:start" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:start expects [id]".to_string())?;
            download_start(state, app, id)
        }
        "download:pause" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:pause expects [id]".to_string())?;
            download_pause(state, id)
        }
        "download:remove" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:remove expects [id, deleteFile?]".to_string())?;
            let delete_file = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            download_remove(state, id, delete_file)
        }
        "download:open" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:open expects [id]".to_string())?;
            download_open(state, id)
        }
        "download:reveal" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:reveal expects [id]".to_string())?;
            download_reveal(state, id)
        }
        "download:map" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:map expects [id]".to_string())?;
            download_map(state, id)
        }
        "download:watch" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:watch expects [id]".to_string())?;
            download_watch(state, id)
        }
        "download:unwatch" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:unwatch expects [id]".to_string())?;
            download_unwatch(state, id)
        }
        "download:refresh-url" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:refresh-url expects [id, newUrl]".to_string())?;
            let new_url = args
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| "download:refresh-url expects [id, newUrl]".to_string())?;
            download_refresh_url(state, app, id, new_url)
        }
        "download:probe" => {
            let url = args
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| "download:probe expects [url]".to_string())?;
            probe_link(std::path::Path::new(&state.paths.db_path), url)
        }
        "network:list" => network_list(state),
        "network:set-enabled" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "network:set-enabled expects [id, enabled]".to_string())?;
            let enabled = args
                .get(1)
                .and_then(Value::as_bool)
                .ok_or_else(|| "network:set-enabled expects [id, enabled]".to_string())?;
            network_set_enabled(state, id, enabled)
        }
        "network:set-limits" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    "network:set-limits expects [id, speedLimit?, allowance?, period?]".to_string()
                })?;
            let speed_limit = byte_count_arg(args.get(1), "speed limit", "bytes per second")?;
            let data_limit = byte_count_arg(args.get(2), "data allowance", "bytes")?;
            let period = data_limit_period_arg(args.get(3), data_limit.is_some())?;
            network_set_limits(state, id, speed_limit, data_limit, period)
        }
        "category:list" => category_list(state),
        "download:set-category" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:set-category expects [id, categoryId|null]".to_string())?;
            let category_id = match args.get(1) {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_i64()
                        .ok_or_else(|| {
                            "download:set-category expects a category id or null".to_string()
                        })?,
                ),
            };
            download_set_category(state, id, category_id)
        }
        "history:tail" => {
            let download_id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "history:tail expects [downloadId, limit?]".to_string())?;
            let limit = args
                .get(1)
                .and_then(Value::as_i64)
                .unwrap_or(100)
                .clamp(1, 10_000);
            history_tail(state, download_id, limit)
        }
        "download:set-speed-limit" => {
            let id = args
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| "download:set-speed-limit expects [id, bytesPerSec?]".to_string())?;
            let cap = byte_count_arg(args.get(1), "speed limit", "bytes per second")?;
            download_set_speed_limit(state, id, cap)
        }
        "app:set-speed-limit" => {
            let cap = byte_count_arg(args.first(), "speed limit", "bytes per second")?;
            app_set_speed_limit(state, cap)
        }
        "settings:list" => settings_list(state),
        "proxy:set-auth" => {
            let username = args
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| "proxy:set-auth expects [username, password]".to_string())?;
            let password = match args.get(1) {
                None | Some(Value::Null) => "",
                Some(Value::String(s)) => s.as_str(),
                Some(_) => return Err("proxy:set-auth expects a password string or null".into()),
            };
            proxy_set_auth(state, username, password)
        }
        "origin:set-auth" => {
            let link = args
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    "origin:set-auth expects [link, username, password, kind]".to_string()
                })?;
            let username = match args.get(1) {
                None | Some(Value::Null) => "",
                Some(Value::String(s)) => s.as_str(),
                Some(_) => return Err("origin:set-auth expects a username string or null".into()),
            };
            let password = match args.get(2) {
                None | Some(Value::Null) => "",
                Some(Value::String(s)) => s.as_str(),
                Some(_) => return Err("origin:set-auth expects a password string or null".into()),
            };
            // The fourth argument is the kind, and it is optional in the oldest sense: a
            // call that leaves it off is the step 4 shape — a pair — so the bridge the
            // page used before tokens existed still means the same thing. The two words it
            // takes are the scheme names the credentials table already allows, so the same
            // spelling travels the write path, the read path and the store.
            let kind = match args.get(3) {
                None | Some(Value::Null) => engine::auth::Kind::Basic,
                Some(Value::String(s)) if s.is_empty() => engine::auth::Kind::Basic,
                Some(Value::String(s)) => match s.as_str() {
                    "basic" => engine::auth::Kind::Basic,
                    "header-secret" => engine::auth::Kind::Bearer,
                    other => {
                        return Err(format!(
                            "origin:set-auth kind expects basic or header-secret, not {other}"
                        ))
                    }
                },
                Some(_) => return Err("origin:set-auth expects a kind string or null".into()),
            };
            origin_set_auth(state, link, kind, username, password)
        }
        "settings:set" => {
            let key = args
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| "settings:set expects [key, value]".to_string())?;
            let value = match args.get(1) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.as_str()),
                Some(_) => return Err("settings:set expects a string value or null".into()),
            };
            settings_set(state, app, key, value)
        }
        "session:list" => session_list(state),
        "session:set" => {
            let url_or_host = args
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| "session:set expects [link, cookie, userAgent?]".to_string())?;
            let cookie = args
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| "session:set expects [link, cookie, userAgent?]".to_string())?;
            let user_agent = match args.get(2) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.as_str()),
                Some(_) => return Err("session:set expects a user agent string or null".into()),
            };
            session_set(state, url_or_host, cookie, user_agent)
        }
        "session:forget" => {
            let url_or_host = args
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| "session:forget expects [link]")?;
            session_forget(state, url_or_host)
        }
        // The browser add-on: state, install, open one browser's add-on page,
        // reveal the staged folder. `target` only selects a browser the machine
        // registered, so nothing here starts a path a caller chose.
        "addon:setup" => {
            let action = args.first().and_then(Value::as_str).unwrap_or("state");
            let target = args.get(1).and_then(Value::as_str);
            let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("sucr.exe"));
            Ok(crate::capture::addon_command(
                std::path::Path::new(&state.paths.data_dir),
                &exe,
                action,
                target,
            ))
        }
        // The update check reads the feed saved in Settings and answers with the
        // version it found; `open` hands a link the answer carried to Windows.
        // The link is passed back in, so nothing outside a released feed is ever
        // opened, and the fetch itself never leaves this process.
        "update:check" => match args.first().and_then(Value::as_str) {
            Some("open") => Ok(crate::updates::open(
                args.get(1).and_then(Value::as_str).unwrap_or(""),
            )),
            _ => {
                // The lock is released before the request: a slow feed must
                // never hold the database every other channel needs.
                let feed = {
                    let db = lock(state)?;
                    crate::db::setting(&db, "update.feed_url")?
                };
                Ok(crate::updates::check(feed.as_deref()))
            }
        },
        "debug:report" => debug_report(state, args.first().and_then(Value::as_str)),
        other => Err(format!("unknown channel: {other}")),
    }
}

/// The debug card's one channel: read the log, empty it, or hand its folder to
/// Explorer. The tail is bounded and read fresh on every call — a stale copy on
/// the page would be the one thing a user checks after a break.
fn debug_report(state: &AppState, action: Option<&str>) -> Result<Value, String> {
    let dir = state.paths.logs_dir.clone();
    match action {
        Some("open") => {
            // A profile that has never written a line has no folder yet, and
            // Explorer opening nothing is indistinguishable from the button
            // doing nothing at all.
            std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {dir}: {e}"))?;
            crate::pick::open(std::path::Path::new(&dir))
                .map_err(|code| format!("Explorer refused the log folder (code {code})"))?;
            Ok(json!({ "path": dir }))
        }
        Some("clear") => {
            crate::debug_log::truncate(&dir);
            if crate::debug_log::enabled() {
                crate::debug_log::write("INFO", "log cleared from Settings");
            }
            Ok(json!({ "path": dir, "bytes": crate::debug_log::bytes(&dir) }))
        }
        Some("issue") => {
            // The browser posts it, not the app: no token ships in the exe, and
            // the user reads the text on GitHub before pressing submit.
            let url = issue_url(&crate::debug_log::tail(&dir, 40), env!("CARGO_PKG_VERSION"));
            let answer = crate::updates::open(&url);
            Ok(json!({ "ok": answer["ok"], "url": url, "error": answer["error"] }))
        }
        _ => Ok(json!({
            "enabled": crate::debug_log::enabled(),
            "path": crate::debug_log::path(&dir).display().to_string(),
            "bytes": crate::debug_log::bytes(&dir),
            "tail": crate::debug_log::tail(&dir, 40),
        })),
    }
}

/// A GitHub new-issue page, filled in but not posted. The repo is the private
/// source repo: the issue is visible to the owner, and nothing is sent until
/// submit is pressed in the browser.
fn issue_url(log: &str, version: &str) -> String {
    let mut excerpt = log.trim().to_string();
    if excerpt.len() > 2400 {
        let cut = (excerpt.len() - 2400..excerpt.len())
            .find(|i| excerpt.is_char_boundary(*i))
            .unwrap_or(excerpt.len());
        excerpt = format!("…{}", &excerpt[cut..]);
    }
    let body = format!(
        "What happened:\n\n\nWhat I was doing:\n\n\n---\nSucr {version} · {} · {}\n\nLast log lines:\n```\n{}\n```\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        excerpt
    );
    format!(
        "https://github.com/benjaminspoop7-sys/sucr/issues/new?title={}&body={}",
        quoted(&format!("Sucr {version} bug report")),
        quoted(&body)
    )
}

/// Percent-encodes everything outside the unreserved set; a query string has no
/// room for a literal newline or a backslash, and the log is full of both.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, String> {    state
        .db
        .lock()
        .map_err(|_| "database lock poisoned".to_string())
}

fn app_info(state: &AppState) -> Result<Value, String> {
    let db = lock(state)?;
    let schema_version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let journal_mode: String = db
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let tables: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    // Read beside the proxy mirror's own answer, so the two are the same instant.
    let proxy_auth = proxy_auth_summary(&db)?;

    Ok(json!({
        "name": "Sucr",
        "version": env!("CARGO_PKG_VERSION"),
        "runtime": format!("Tauri {}", tauri::VERSION),
        "webview": tauri::webview_version().unwrap_or_else(|_| "unknown".into()),
        "sqlite": rusqlite::version(),
        "schemaVersion": schema_version,
        "journalMode": journal_mode,
        "tables": tables,
        // `as_slice` is load-bearing: serde implements `Serialize` for arrays
        // only up to 32 elements, so a bare `CHANNELS` stops compiling once the
        // list grows past that.
        "channels": CHANNELS.as_slice(),
        "channelCount": CHANNELS.len(),
        "globalSpeedLimit": state.global_limit.cap().map(|v| v as i64),
        // The engine's own answer, not the stored strings: `via` resolves the
        // mirror the way the next agent will, so a manual mode holding nothing
        // readable reports itself instead of showing the address that failed.
        // Read here rather than recomposed in the page, because `system` asks
        // the environment, which the renderer cannot see.
        "proxyVia": engine::proxy::via(),
        // Whether a credential is stored, whose name it carries, which proxy it was
        // typed for, and whether that proxy is the one in front of the app now. The
        // secret is not in this answer and cannot be: the panel has to say "a
        // credential is stored for 127.0.0.1:8127" without putting the password in
        // the webview's memory to print a dot.
        "proxyAuth": proxy_auth,
        // Every stored origin credential, host and username and the length of the
        // secret — never the secret. A list rather than one object because there is one
        // per host, and it rides on this payload rather than a `origin:list` channel for
        // the reason `proxyAuth` does: the page needs it to say "the stored credential was
        // refused" beside a link, and a read that returns no secrets cannot leak one
        // by being called too often.
        "originAuths": crate::db::list_origin_auths(&db)?
            .into_iter()
            .map(|a| json!({
                "host": a.host,
                "kind": a.kind.scheme(),
                "username": a.username,
                "secretBytes": a.secret_bytes,
                "createdAt": a.created_at,
            }))
            .collect::<Vec<_>>(),
        // Read back from the registry, not inferred from a write that returned
        // success: a browser named here is one whose key still points at this
        // executable, and the add-on's own popup repeats this answer.
        "capture": crate::capture::host_state(
            std::path::Path::new(&state.paths.data_dir),
            &std::env::current_exe().unwrap_or_default(),
        ),
        "startedAt": state.started_at,
        "smoke": state.smoke,
        "paths": {
            "dataDir": state.paths.data_dir,
            "dbPath": state.paths.db_path,
            "logsDir": state.paths.logs_dir,
            "downloadsDir": state.paths.downloads_dir,
        },
    }))
}

// The absolute file a row writes to: its folder, or the library's downloads
// directory when no per-download folder was named, joined with the filename. The
// capture door reads this to hand finished files to the muxer without guessing a
// path from a separator.
fn final_path(state: &AppState, folder: Option<&str>, filename: &str) -> String {
    let dir = folder.unwrap_or(state.paths.downloads_dir.as_str());
    std::path::Path::new(dir).join(filename).to_string_lossy().into_owned()
}

fn download_list(state: &AppState, filter: Option<&str>) -> Result<Value, String> {
    let db = lock(state)?;
    const COLS: &str = "id, url, final_url, method, status, category_id, folder, filename, \
         total_bytes, wanted_bytes, verified_bytes, block_size, block_count, streams_mode, \
         checksum_algo, checksum_hex, resumable, created_at, started_at, finished_at, error_code, \
         error_hint, speed_limit, referer, streams_fixed, streams_final";
    // The band a job with no dial of its own runs in, so a list can name the
    // ceiling the ramp may reach as well as the width it opened at.
    let band = job::stream_band(&db);
    // A running job's width lives with the job, not the row: the ramp moves it
    // once a second and the row would have to be rewritten to keep up.
    let live_width = |id: i64, status: &str| -> Option<i64> {
        if status != "downloading" {
            return None;
        }
        state.engine.live(id).map(|l| l.streams() as i64)
    };
    // Once the job is gone the registry is empty, so the row carries the width the
    // run stopped at. A queued or running row must not borrow a previous run's.
    let ended_width = |status: &str, stored: Option<i64>| -> Option<i64> {
        match status {
            "finished" | "verified" | "needs-attention" | "paused" => stored,
            _ => None,
        }
    };
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Value> {
        let id: i64 = r.get(0)?;
        let status: String = r.get(4)?;
        let folder: Option<String> = r.get(6)?;
        let filename: String = r.get(7)?;
        let live = live_width(id, &status);
        let ended = ended_width(&status, r.get::<_, Option<i64>>(25)?);
        Ok(json!({
            "id": id,
            "url": r.get::<_, String>(1)?,
            "finalUrl": r.get::<_, Option<String>>(2)?,
            "method": r.get::<_, String>(3)?,
            "status": status,
            "categoryId": r.get::<_, Option<i64>>(5)?,
            "folder": folder,
            "filename": filename.clone(),
            "path": final_path(state, folder.as_deref(), &filename),
            "totalBytes": r.get::<_, Option<i64>>(8)?,
            "wantedBytes": r.get::<_, Option<i64>>(9)?,
            "verifiedBytes": r.get::<_, i64>(10)?,
            "blockSize": r.get::<_, Option<i64>>(11)?,
            "blockCount": r.get::<_, Option<i64>>(12)?,
            "streamsMode": r.get::<_, Option<String>>(13)?,
            "checksumAlgo": r.get::<_, Option<String>>(14)?,
            "checksumHex": r.get::<_, Option<String>>(15)?,
            "resumable": r.get::<_, Option<i64>>(16)?.map(|v| v == 1),
            "createdAt": r.get::<_, i64>(17)?,
            "startedAt": r.get::<_, Option<i64>>(18)?,
            "finishedAt": r.get::<_, Option<i64>>(19)?,
            "errorCode": r.get::<_, Option<String>>(20)?,
            "errorHint": r.get::<_, Option<String>>(21)?,
            "speedLimit": r.get::<_, Option<i64>>(22)?,
            "referer": r.get::<_, Option<String>>(23)?,
            "streams": r.get::<_, Option<i64>>(24)?,
            "streamsRequested": r.get::<_, Option<i64>>(24)?
                .filter(|s| *s > 0).unwrap_or(band.start as i64),
            "streamsCeiling": r.get::<_, Option<i64>>(24)?
                .filter(|s| *s > 0).unwrap_or(band.ceiling as i64),
            "streamsLive": live,
            "streamsFinal": ended,
        }))
    };

    let mut out = Vec::new();
    match filter {
        Some(status) => {
            let mut stmt = db
                .prepare(&format!(
                    "SELECT {COLS} FROM downloads WHERE status = ?1 ORDER BY created_at DESC"
                ))
                .map_err(|e| e.to_string())?;
            let rows = stmt.query_map([status], map).map_err(|e| e.to_string())?;
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
        }
        None => {
            let mut stmt = db
                .prepare(&format!("SELECT {COLS} FROM downloads ORDER BY created_at DESC"))
                .map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], map).map_err(|e| e.to_string())?;
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
        }
    }
    Ok(Value::Array(out))
}

/// The one read the capture door is allowed: a row by id, same shape the
/// window gets. Kept as its own name so the socket's reach is visible in one
/// place instead of hiding inside the IPC dispatch.
pub(crate) fn download_get_public(state: &AppState, id: i64) -> Result<Value, String> {
    download_get(state, id)
}

fn download_get(state: &AppState, id: i64) -> Result<Value, String> {
    let db = lock(state)?;
    // The band a job with no dial of its own runs in, so a list can name the
    // ceiling the ramp may reach as well as the width it opened at.
    let band = job::stream_band(&db);
    let live_width = |id: i64, status: &str| -> Option<i64> {
        if status != "downloading" {
            return None;
        }
        state.engine.live(id).map(|l| l.streams() as i64)
    };
    let ended_width = |status: &str, stored: Option<i64>| -> Option<i64> {
        match status {
            "finished" | "verified" | "needs-attention" | "paused" => stored,
            _ => None,
        }
    };
    let row = db
        .query_row(
            "SELECT id, url, final_url, method, status, category_id, folder, filename, \
             total_bytes, wanted_bytes, verified_bytes, block_size, block_count, streams_mode, \
             checksum_algo, checksum_hex, resumable, created_at, started_at, finished_at, error_code, \
             error_hint, speed_limit, referer, streams_fixed, streams_final \
             FROM downloads WHERE id = ?1",
            [id],
            |r| {
                let status: String = r.get(4)?;
                let folder: Option<String> = r.get(6)?;
                let filename: String = r.get(7)?;
                let live = live_width(id, &status);
                let ended = ended_width(&status, r.get::<_, Option<i64>>(25)?);
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "url": r.get::<_, String>(1)?,
                    "finalUrl": r.get::<_, Option<String>>(2)?,
                    "method": r.get::<_, String>(3)?,
                    "status": status,
                    "categoryId": r.get::<_, Option<i64>>(5)?,
                    "folder": folder,
                    "filename": filename.clone(),
                    "path": final_path(state, folder.as_deref(), &filename),
                    "totalBytes": r.get::<_, Option<i64>>(8)?,
                    "wantedBytes": r.get::<_, Option<i64>>(9)?,
                    "verifiedBytes": r.get::<_, i64>(10)?,
                    "blockSize": r.get::<_, Option<i64>>(11)?,
                    "blockCount": r.get::<_, Option<i64>>(12)?,
                    "streamsMode": r.get::<_, Option<String>>(13)?,
                    "checksumAlgo": r.get::<_, Option<String>>(14)?,
                    "checksumHex": r.get::<_, Option<String>>(15)?,
                    "resumable": r.get::<_, Option<i64>>(16)?.map(|v| v == 1),
                    "createdAt": r.get::<_, i64>(17)?,
                    "startedAt": r.get::<_, Option<i64>>(18)?,
                    "finishedAt": r.get::<_, Option<i64>>(19)?,
                    "errorCode": r.get::<_, Option<String>>(20)?,
                    "errorHint": r.get::<_, Option<String>>(21)?,
                    "speedLimit": r.get::<_, Option<i64>>(22)?,
                    "referer": r.get::<_, Option<String>>(23)?,
                    "streams": r.get::<_, Option<i64>>(24)?,
                    "streamsRequested": r.get::<_, Option<i64>>(24)?
                        .filter(|s| *s > 0).unwrap_or(band.start as i64),
                    "streamsCeiling": r.get::<_, Option<i64>>(24)?
                        .filter(|s| *s > 0).unwrap_or(band.ceiling as i64),
                    "streamsLive": live,
                    "streamsFinal": ended,
                }))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })?;
    Ok(row.unwrap_or(Value::Null))
}

pub(crate) fn download_add(state: &AppState, spec: &Value) -> Result<Value, String> {
    let url = spec
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "download:add expects { url }".to_string())?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http and https URLs can be added".into());
    }
    let filename = spec
        .get("filename")
        .and_then(Value::as_str)
        .map(engine::sanitize_filename)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| engine::filename_from_url(url));
    let stored_folder = {
        let db = lock(state)?;
        crate::db::setting(&db, "storage.default_folder")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
    };
    let folder = spec
        .get("folder")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(stored_folder)
        .unwrap_or_else(|| state.paths.downloads_dir.clone());
    let checksum_algo = spec
        .get("checksumAlgo")
        .and_then(Value::as_str)
        .map(str::to_string);
    let checksum_hex = spec
        .get("checksumHex")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    if let Some(algo) = &checksum_algo {
        if !engine::is_algo(algo) {
            return Err(format!("unknown checksum {algo}"));
        }
        if checksum_hex.is_none() {
            return Err("checksumHex is required with checksumAlgo".into());
        }
    }
    let referer = spec
        .get("referer")
        .and_then(Value::as_str)
        .map(str::to_string);
    let streams = spec
        .get("streams")
        .and_then(Value::as_i64)
        .map(|s| s.clamp(1, engine::MAX_STREAMS as i64));
    // Which browser handed this link over, when one did. Empty is a paste and
    // stays empty, so the library can tell the two apart without a guess.
    let source = spec
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // The browser session a captured link rode, when the add-on handed one over.
    // Stored per-download so the engine replays this exact identity on every
    // range request instead of the host-wide one — the ride an attested adaptive
    // stream needs. Empty is a paste and stays absent.
    let ua = spec
        .get("ua")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let cookie = spec
        .get("cookie")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // A DASH pair arrives as one job: this is the sound stream's link, fetched
    // by the same job after the picture and merged into the one file the row
    // names. Empty is an ordinary single-link download.
    let audio_url = spec
        .get("audioUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(audio) = &audio_url {
        if !(audio.starts_with("http://") || audio.starts_with("https://")) {
            return Err("audioUrl must be an http or https link".into());
        }
    }

    let db = lock(state)?;
    // The extension decides the starting category; the card's Category menu is
    // the override. Folder placement is the renderer's call, so a file still
    // lands where the add bar said it would.
    let category_id = crate::db::category_for_file(&db, &filename)?;
    db.execute(
        "INSERT INTO downloads (url, filename, folder, status, category_id, checksum_algo, \
         checksum_hex, referer, created_at, streams_fixed, source_browser, ua, cookie, audio_url) \
         VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        rusqlite::params![
            url,
            filename,
            folder,
            category_id,
            checksum_algo,
            checksum_hex,
            referer,
            crate::now_ms(),
            streams,
            source,
            ua,
            cookie,
            audio_url
        ],
    )
    .map_err(|e| e.to_string())?;
    let id = db.last_insert_rowid();
    drop(db);
    download_get(state, id)
}

pub(crate) fn download_start(state: &AppState, app: &AppHandle, id: i64) -> Result<Value, String> {
    let status: Option<String> = {
        let db = lock(state)?;
        db.query_row("SELECT status FROM downloads WHERE id = ?1", [id], |r| r.get(0))
            .ok()
    };
    let status = status.ok_or_else(|| format!("download {id} not found"))?;
    if status == "finished" || status == "verified" {
        return download_get(state, id);
    }
    // A live job keeps `downloading`; `queued` is a Start still in flight (the
    // thread registered but has not written its first status). Either way the row
    // is already being carried, so a second Start is a no-op.
    if (status == "downloading" || status == "queued") && state.engine.is_running(id) {
        return download_get(state, id);
    }
    // The row says paused or needs-attention, but the job thread may still be
    // draining: it writes that status inside run() and only unregisters afterwards
    // (ipc.rs spawn_job). A Resume pressed in that gap used to be swallowed by
    // the is_running short-circuit. Wait for the thread to retire before deciding,
    // as the delete path does; if it cannot (a worker blocked past the bound),
    // leave the row alone rather than spawn a second job onto the same id.
    if state.engine.is_running(id) && !state.engine.wait_idle(id, std::time::Duration::from_secs(6)) {
        return download_get(state, id);
    }

    // One Start at a time across the UI thread and the completion pump, so two
    // slots can never be claimed for the same free place.
    let _gate = START_LOCK.lock().map_err(|_| "start lock poisoned".to_string())?;
    let (max, routes, policy, open) = {
        let db = lock(state)?;
        (
            max_active(&db),
            bond_routes(&db),
            bonding_policy(&db),
            crate::db::window_open_now(&db),
        )
    };
    if routes.is_empty() {
        return Err("all network connections are disabled".into());
    }
    // Outside the window the row keeps its place in the queue and waits for the
    // pass that opens it — the same hand-off a full cap uses, so a closed window
    // needs no second queue of its own.
    if !open {
        state.engine.push_pending(id);
        return download_get(state, id);
    }
    if state.engine.running_count() >= max {
        state.engine.push_pending(id);
        return download_get(state, id);
    }
    state.engine.remove_pending(id);
    spawn_job(
        &state.paths.db_path,
        &state.paths.downloads_dir,
        app,
        &state.engine,
        &state.global_limit,
        &state.network_scopes,
        id,
        routes,
        policy,
    );
    download_get(state, id)
}

pub(crate) static START_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn max_active(conn: &Connection) -> usize {
    crate::db::setting(conn, "engine.max_active_downloads")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3)
        .clamp(1, 10)
}

/// Registers and spawns one job; when it ends, the next queued Start may claim
/// the free slot.
///
/// `routes` and `policy` are resolved by the caller while it still holds the
/// database, so every job starts from the device list and the strategy the user
/// is looking at and never re-reads them mid-transfer. `networks` is the live
/// registry instead: a job reads it per block attempt, so a cap saved while a
/// transfer runs applies to it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_job(
    db_path: &str,
    downloads_dir: &str,
    app: &AppHandle,
    engine: &std::sync::Arc<job::Registry>,
    global: &std::sync::Arc<engine::limit::Bucket>,
    networks: &job::NetworkScopes,
    id: i64,
    routes: Vec<engine::bond::Route>,
    policy: job::BondPolicy,
) {
    let (cancel, live, limit) = engine.register(id);
    // Claimed a slot, so the machine is now carrying a transfer: ask for it.
    engine.refresh_keep_awake();
    let ctx = job::RunContext {
        db_path: std::path::PathBuf::from(db_path),
        downloads_dir: std::path::PathBuf::from(downloads_dir),
        app: Some(app.clone()),
        live: Some(live),
        limit,
        global: std::sync::Arc::clone(global),
        networks: std::sync::Arc::clone(networks),
        routes,
        policy,
        ramp: job::RampTuning::default(),
    };
    let registry = std::sync::Arc::clone(engine);
    let pump_app = app.clone();
    let pump_db = db_path.to_string();
    let pump_dir = downloads_dir.to_string();
    let pump_engine = std::sync::Arc::clone(engine);
    let pump_global = std::sync::Arc::clone(global);
    let pump_networks = std::sync::Arc::clone(networks);
    std::thread::spawn(move || {
        if let Err(e) = job::run(ctx, id, cancel) {
            job::note_early_failure(std::path::Path::new(&pump_db), &pump_app, id, &e);
        }
        registry.unregister(id);
        // And this is the thread that knows a slot just emptied, whatever the
        // reason the job returned for.
        registry.refresh_keep_awake();
        pump_queue(
            &pump_db,
            &pump_dir,
            &pump_app,
            &pump_engine,
            &pump_global,
            &pump_networks,
        );
    });
}

/// The transfers the machine stopped under. `downloading` is the one status only a
/// deliberate Start ever writes (`job::run` alone writes it), so the rows holding it
/// are ones the user asked for and this is the only place a scan of them belongs: a
/// row merely `queued` was added with auto-start off and stays its user's to launch,
/// and `paused` or a failed row is his to retry.
///
/// They are moved to `queued` before anything claims them, because they own nothing
/// yet: a row left at `downloading` with no job behind it claims an activity that has
/// stopped, and offers its user a Pause instead of the Resume that would bring it
/// back. The queue is then handed the same slots a Start would have.
pub(crate) fn reclaim_interrupted(state: &AppState) -> Result<Vec<i64>, String> {
    let db = lock(state)?;
    let mut ids: Vec<i64> = Vec::new();
    {
        let mut stmt = db
            .prepare("SELECT id FROM downloads WHERE status = 'downloading' ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            ids.push(row.map_err(|e| e.to_string())?);
        }
    }
    if ids.is_empty() {
        return Ok(ids);
    }
    db.execute(
        "UPDATE downloads SET status = 'queued' WHERE status = 'downloading'",
        [],
    )
    .map_err(|e| e.to_string())?;
    // The writes have to be visible before a slot is claimed, because the queue
    // re-reads the row it is about to launch and skips anything that is not `queued`.
    drop(db);
    for id in &ids {
        state.engine.push_pending(*id);
    }
    Ok(ids)
}

pub(crate) fn pump_queue(
    db_path: &str,
    downloads_dir: &str,
    app: &AppHandle,
    engine: &std::sync::Arc<job::Registry>,
    global: &std::sync::Arc<engine::limit::Bucket>,
    networks: &job::NetworkScopes,
) {
    let _gate = match START_LOCK.lock() {
        Ok(gate) => gate,
        Err(_) => return,
    };
    let Ok(conn) = crate::db::open(std::path::Path::new(db_path)) else {
        return;
    };
    // Ahead of the loop, not inside it: popping first and then refusing would
    // take the id out of the queue with nowhere to put it back. Shut, the pump
    // simply does not look, and every waiting Start keeps its place until the
    // watcher's pass finds the window open.
    if !crate::db::window_open_now(&conn) {
        return;
    }
    loop {
        if engine.running_count() >= max_active(&conn) {
            return;
        }
        let Some(id) = engine.pop_pending() else {
            return;
        };
        let queued = conn
            .query_row("SELECT status FROM downloads WHERE id = ?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .map(|s| s == "queued")
            .unwrap_or(false);
        if !queued {
            continue;
        }
        let routes = bond_routes(&conn);
        if routes.is_empty() {
            return;
        }
        let policy = bonding_policy(&conn);
        spawn_job(
            db_path,
            downloads_dir,
            app,
            engine,
            global,
            networks,
            id,
            routes,
            policy,
        );
    }
}

fn download_pause(state: &AppState, id: i64) -> Result<Value, String> {
    state.engine.remove_pending(id);
    if !state.engine.signal(id, job::PAUSE) {
        let db = lock(state)?;
        db.execute(
            "UPDATE downloads SET status = 'paused' \
             WHERE id = ?1 AND status IN ('queued', 'downloading')",
            [id],
        )
        .map_err(|e| e.to_string())?;
    }
    download_get(state, id)
}

/// Ask a link whether it can be fetched, before any job exists.
///
/// This is the same one-byte range request the engine's own pre-flight makes
/// (`engine::probe`), so the answer the add bar shows is the answer the transfer
/// would have got — a link greyed out here is a link the engine cannot pull.
/// The host's stored browser session is applied the same way a job applies it,
/// because a link the user has already unlocked must not stay greyed out.
/// Nothing is written: no row, no history event, no file.
pub fn probe_link(db_path: &std::path::Path, url: &str) -> Result<Value, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Ok(json!({
            "ok": false, "code": "scheme", "status": Value::Null,
            "message": "only http and https links can be downloaded"
        }));
    }
    let host = engine::host_key(url);
    let session = crate::db::session_at(db_path, &host);
    // Whether this host has an origin credential in the engine's mirror, asked of the
    // mirror rather than the database: the middleware below reads that same store, so
    // this is the answer to "did the request that just failed carry a credential?", which
    // is the only thing the page can turn into "the stored credential was refused".
    let auth = engine::auth::has(&host);
    let ua = session.as_ref().and_then(|s| s.user_agent.as_deref());
    let cookie = session.as_ref().map(|s| s.cookie.as_str());
    // Deliberately shorter than the engine's connect timeout: a link check runs
    // while the user is still typing, and five seconds of silence from a host is
    // already a dead link for this purpose.
    let agent = engine::config_ua(std::time::Duration::from_secs(5), ua).new_agent();
    match engine::probe(&agent, url, None, cookie) {
        Ok(p) => Ok(json!({
            "ok": true,
            "code": "ok",
            "status": p.status,
            "totalBytes": p.total_bytes,
            "acceptRanges": p.accept_ranges,
            "filename": p.filename,
            "session": cookie.is_some(),
            "auth": auth,
        })),
        Err(e) => Ok(json!({
            "ok": false,
            "code": e.code,
            "status": e.status,
            "message": e.message,
            "session": cookie.is_some(),
            "auth": auth,
        })),
    }
}

fn download_refresh_url(
    state: &AppState,
    app: &AppHandle,
    id: i64,
    new_url: &str,
) -> Result<Value, String> {
    // The row says `downloading` while a worker owns it, but the registry is the
    // authority: a job that died without writing its status must not be re-pointed
    // out from under its own threads. Wait for the thread to retire if it was
    // just paused.
    if state.engine.is_running(id) && !state.engine.wait_idle(id, std::time::Duration::from_secs(6)) {
        return Err("pause this download before replacing its link".to_string());
    }
    // Deliberately not through `lock(state)`: this probes the new link first, and
    // every other channel would queue behind the shared connection for as long as
    // that takes.
    job::refresh_url(std::path::Path::new(&state.paths.db_path), id, new_url)
        .map_err(|e| e.message)?;
    // Resume through the same door Start uses, so the slot limit and the pending
    // queue treat a refreshed job like any other one.
    download_start(state, app, id)
}

fn download_set_speed_limit(state: &AppState, id: i64, cap: Option<i64>) -> Result<Value, String> {
    {
        let db = lock(state)?;
        let exists: Option<i64> = db
            .query_row("SELECT id FROM downloads WHERE id = ?1", [id], |r| r.get(0))
            .ok();
        if exists.is_none() {
            return Err(format!("download {id} not found"));
        }
        db.execute(
            "UPDATE downloads SET speed_limit = ?1 WHERE id = ?2",
            rusqlite::params![cap, id],
        )
        .map_err(|e| e.to_string())?;
        let _ = db.execute(
            "INSERT INTO history_events (download_id, ts, kind, payload_json) \
             VALUES (?1, ?2, 'speed-limit', ?3)",
            rusqlite::params![
                id,
                crate::now_ms(),
                json!({ "bytesPerSec": cap }).to_string()
            ],
        );
    }
    state.engine.set_limit(id, cap.map(|v| v as u64));
    download_get(state, id)
}

fn app_set_speed_limit(state: &AppState, cap: Option<i64>) -> Result<Value, String> {
    {
        let db = lock(state)?;
        crate::db::set_setting(&db, "global_speed_limit", cap.map(|v| v.to_string()).as_deref())?;
    }
    state.global_limit.set_cap(cap.map(|v| v as u64));
    app_info(state)
}

/// The keys the settings KV accepts: the 19 tunables the Settings stage exposes
/// plus the first-run flag, which lives here rather than in localStorage because
/// WebView2 shares that store across `--data-dir` profiles on Windows. Unknown
/// keys are rejected so the table cannot become a scratchpad.
///
/// The proxy trio and its Settings row shipped together, in the same slice: the
/// engine reads these keys the moment they are legal, and the row is the only
/// place a user can see what the engine then answered.
const SETTING_KEYS: [&str; 26] = [
    "engine.max_active_downloads",
    "engine.default_streams",
    "global_speed_limit",
    "engine.auto_start",
    "engine.window_enabled",
    "engine.window_start",
    "engine.window_stop",
    "engine.window_days",
    "storage.default_folder",
    "storage.preallocation",
    "storage.motw",
    "storage.auto_verify",
    "network.bonding_policy",
    "network.metered_guard",
    "network.timeout_seconds",
    "network.proxy_mode",
    "network.proxy_url",
    "network.proxy_bypass",
    "ui.theme",
    "ui.meter_style",
    "ui.motion",
    "ui.sound_on_complete",
    "ui.close_to_tray",
    "system.setup_completed",
    "system.debug_log",
    "update.feed_url",
];

fn settings_list(state: &AppState) -> Result<Value, String> {
    let db = lock(state)?;
    let mut stmt = db
        .prepare("SELECT key, value FROM settings ORDER BY key")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut map = serde_json::Map::new();
    for row in rows {
        let (key, value) = row.map_err(|e| e.to_string())?;
        map.insert(key, Value::String(value));
    }
    Ok(Value::Object(map))
}

fn settings_set(
    state: &AppState,
    app: &AppHandle,
    key: &str,
    value: Option<&str>,
) -> Result<Value, String> {
    if !SETTING_KEYS.contains(&key) {
        return Err(format!("unknown setting key: {key}"));
    }
    let value = value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(v) = &value {
        validate_setting(key, v)?;
    }
    {
        let db = lock(state)?;
        crate::db::set_setting(&db, key, value.as_deref())?;
    }
    match key {
        "global_speed_limit" => {
            let cap = value
                .as_deref()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0);
            state.global_limit.set_cap(cap);
        }
        "network.timeout_seconds" => {
            if let Some(secs) = value.as_deref().and_then(|v| v.parse::<u64>().ok()) {
                engine::set_connect_timeout(secs);
            }
        }
        // Takes effect on this write, not on the next launch: a user who turns the
        // log off because it is noisy should not have to restart to be believed.
        "system.debug_log" => {
            let on = value.as_deref() != Some("false");
            crate::debug_log::switch(&state.paths.logs_dir, on);
            if on {
                crate::debug_log::write("INFO", "debug log switched on in Settings");
            }
        }
        // One decision in three keys, so whichever one moved, the mirror reads
        // all three back: a mode must never run against a URL from before this
        // write, and clearing the URL is a legal write that means direct.
        "network.proxy_mode" | "network.proxy_url" | "network.proxy_bypass" => {
            let db = lock(state)?;
            let (mode, url, bypass) = crate::db::proxy_settings(&db)?;
            engine::proxy::configure(&mode, &url, &bypass);
        }
        // Raising the ceiling may free slots for Starts already waiting, and
        // opening the window releases the same queue — so either write pumps, and
        // a write that shuts it makes the pump return before it looks at a row.
        "engine.max_active_downloads"
        | "engine.window_enabled"
        | "engine.window_start"
        | "engine.window_stop"
        | "engine.window_days" => pump_queue(
            &state.paths.db_path,
            &state.paths.downloads_dir,
            app,
            &state.engine,
            &state.global_limit,
            &state.network_scopes,
        ),
        _ => {}
    }
    Ok(Value::Null)
}

/// Every pasted browser session, oldest first.
///
/// The cookie value is deliberately absent: the list is for painting rows and a
/// secret does not need to cross the bridge to show how big it is.
fn session_list(state: &AppState) -> Result<Value, String> {
    let db = lock(state)?;
    let rows = crate::db::list_sessions(&db)?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|s| {
                json!({
                    "host": s.host,
                    "userAgent": s.user_agent,
                    "cookieBytes": s.cookie_bytes,
                    "createdAt": s.created_at,
                })
            })
            .collect(),
    ))
}

/// Stores one host's session, replacing whatever was there.
///
/// The first argument may be a host or a whole link: `host_key` reduces both to the
/// identity a job looks up, so the card that was refused can hand over its own URL
/// and the session still lands where the next Start will find it.
fn session_set(
    state: &AppState,
    url_or_host: &str,
    cookie: &str,
    user_agent: Option<&str>,
) -> Result<Value, String> {
    let host = engine::host_key(url_or_host);
    if host.is_empty() {
        return Err("session:set expects a host or an http(s) link".to_string());
    }
    let cookie = crate::db::validate_cookie(cookie)?
        .ok_or_else(|| "a session needs a cookie; use session:forget to drop one".to_string())?;
    let user_agent = crate::db::validate_user_agent(user_agent)?;
    let session = {
        let db = lock(state)?;
        crate::db::put_session(&db, &host, &cookie, user_agent.as_deref(), crate::now_ms())?
    };
    Ok(json!({
        "host": host,
        "userAgent": session.user_agent,
        "cookieBytes": session.cookie.len(),
    }))
}

fn session_forget(state: &AppState, url_or_host: &str) -> Result<Value, String> {
    let host = engine::host_key(url_or_host);
    if host.is_empty() {
        return Err("session:forget expects a host or an http(s) link".to_string());
    }
    let forgotten = {
        let db = lock(state)?;
        crate::db::forget_session(&db, &host)?
    };
    Ok(json!({ "host": host, "forgotten": forgotten }))
}

/// The credential as the page is allowed to see it: that there is one, whose name it
/// carries, which proxy it was typed for, and whether that proxy is the one in front of
/// the app now. The last is the mirror's answer and not a string comparison the
/// renderer could make, because the mirror is what decides.
fn proxy_auth_summary(db: &Connection) -> Result<Value, String> {
    let Some(auth) = crate::db::proxy_auth(db)? else {
        return Ok(json!({
            "stored": false,
            "username": Value::Null,
            "host": Value::Null,
            "applies": false,
        }));
    };
    Ok(json!({
        "stored": true,
        "username": auth.username,
        "host": auth.host,
        "applies": engine::proxy::auth_host().as_deref() == Some(auth.host.as_str()),
    }))
}

/// Stores, or forgets, the Basic pair for the proxy the user typed.
///
/// Two boxes and one command, because the pair is one thing: ureq writes it into the
/// `CONNECT` request it sends *before* the proxy can challenge (`connect.rs:116-124`),
/// so there is no round to answer and both halves have to be in place first.
///
/// The row is keyed to [`engine::proxy::auth_host`], which is the authority the mirror
/// is about to build: a pair stored for `127.0.0.1:8127` must not travel to
/// `proxy.corp:3128` the moment the URL box is edited, and the only way to know which
/// proxy a credential belongs to is to ask the layer that uses it. With no manual proxy
/// in front of the app there is nothing to key to, so the answer is the instruction the
/// panel already gives.
///
/// Not a `settings:set` key, and that is the point: `settings:list` hands every value
/// it holds to the page, so a password in `settings` would cross the bridge on every
/// Settings open. The reply names the username and how many bytes the secret was, never
/// the secret.
fn proxy_set_auth(state: &AppState, username: &str, password: &str) -> Result<Value, String> {
    let username = username.trim();
    if username.is_empty() && password.trim().is_empty() {
        let forgotten = {
            let db = lock(state)?;
            crate::db::forget_proxy_auth(&db)?
        };
        engine::proxy::set_auth("", "", "");
        return Ok(json!({ "stored": false, "forgotten": forgotten }));
    }
    if username.is_empty() {
        // ureq builds the userinfo only inside `if let Some(username)`, so this pair
        // would be stored, shown on the screen as a credential, and never sent.
        return Err(
            "a proxy password needs a username: the two go out as one field, and a \
             password without a name before it is never sent"
                .to_string(),
        );
    }
    if username.len() > crate::db::MAX_PROXY_USERNAME_BYTES {
        return Err(format!(
            "a proxy username longer than {} bytes is not one",
            crate::db::MAX_PROXY_USERNAME_BYTES
        ));
    }
    if password.len() > crate::db::MAX_PROXY_SECRET_BYTES {
        return Err(format!(
            "a proxy password longer than {} bytes is not one",
            crate::db::MAX_PROXY_SECRET_BYTES
        ));
    }
    let host = engine::proxy::auth_host()
        .ok_or_else(|| "set a manual proxy address first; a credential belongs to the proxy it is typed for".to_string())?;
    let auth = {
        let db = lock(state)?;
        crate::db::put_proxy_auth(&db, &host, username, password, crate::now_ms())?
    };
    engine::proxy::set_auth(&auth.host, &auth.username, &auth.secret);
    Ok(json!({
        "stored": true,
        "host": auth.host,
        "username": auth.username,
        "passwordBytes": auth.secret.len(),
    }))
}

/// Stores, or forgets, the Basic pair for the host a link points at.
///
/// Three arguments where the proxy's command takes two, because an origin credential
/// needs to be told which host it belongs to and the page is the only place holding the
/// link the user is looking at. The host is taken with [`engine::host_key`], the same
/// function the middleware and the cookie rows use, so a password typed beside a link
/// lands under the exact key the request will be looked up by — and a hand-written host
/// in the box cannot create a second identity for the same server.
///
/// The pair commits as one write for the same reason the proxy's does: Basic puts
/// `username:password` in a single header, so half a pair is not a partial success, it
/// is a credential that can never work. Both halves empty is the forget path.
///
/// A token has no second half, so for `Kind::Bearer` the name is not read at all — the
/// page hides that box — and an empty token alone is the forget. One credential per host
/// either way: writing the other kind for a host replaces what it held, because the
/// middleware can put exactly one `Authorization` on a request.
fn origin_set_auth(
    state: &AppState,
    link: &str,
    kind: engine::auth::Kind,
    username: &str,
    password: &str,
) -> Result<Value, String> {
    let host = engine::host_key(link);
    if host.is_empty() {
        return Err("origin:set-auth expects a link or a host".to_string());
    }
    let token = kind == engine::auth::Kind::Bearer;
    let noun = if token { "token" } else { "password" };
    let username = if token { "" } else { username.trim() };
    if username.is_empty() && password.trim().is_empty() {
        let forgotten = {
            let db = lock(state)?;
            crate::db::forget_origin_auth(&db, &host)?
        };
        engine::auth::set(&host, kind, "", "");
        return Ok(json!({ "stored": false, "host": host, "forgotten": forgotten }));
    }
    if username.is_empty() && !token {
        // The header this app writes is `user:password`; a password with no name before
        // it is a pair that cannot be used, and storing one would show the host as
        // credentialled while every request went out anonymous.
        return Err(
            "a password needs a username: the two go out as one header, and a password \
             without a name before it is never sent"
                .to_string(),
        );
    }
    if username.len() > crate::db::MAX_ORIGIN_USERNAME_BYTES {
        return Err(format!(
            "a username longer than {} bytes is not one",
            crate::db::MAX_ORIGIN_USERNAME_BYTES
        ));
    }
    if password.len() > crate::db::MAX_ORIGIN_SECRET_BYTES {
        return Err(format!(
            "a {noun} longer than {} bytes is not one",
            crate::db::MAX_ORIGIN_SECRET_BYTES
        ));
    }
    let auth = {
        let db = lock(state)?;
        crate::db::put_origin_auth(&db, &host, kind, username, password, crate::now_ms())?
    };
    engine::auth::set(&auth.host, auth.kind, &auth.username, &auth.secret);
    Ok(json!({
        "stored": true,
        "host": auth.host,
        "kind": auth.kind.scheme(),
        "username": auth.username,
        "secretBytes": auth.secret.len(),
    }))
}

fn validate_setting(key: &str, value: &str) -> Result<(), String> {
    let one_of = |allowed: &[&str]| -> Result<(), String> {
        if allowed.contains(&value) {
            Ok(())
        } else {
            Err(format!("{key} expects one of {}", allowed.join(", ")))
        }
    };
    let boolean = || one_of(&["true", "false"]);
    match key {
        "engine.max_active_downloads" => match value.parse::<u8>() {
            Ok(n) if (1..=10).contains(&n) => Ok(()),
            _ => Err(format!("{key} expects 1-10")),
        },
        // `auto` hands the width to the ramp; a number is a promise to the host
        // and the job starts and stops there.
        "engine.default_streams" => one_of(&["auto", "1", "4", "8", "16", "32"]),
        "global_speed_limit" => match value.parse::<u64>() {
            Ok(n) if n > 0 => Ok(()),
            _ => Err(format!("{key} expects whole bytes per second")),
        },
        "engine.auto_start"
        | "storage.motw"
        | "storage.auto_verify"
        | "network.metered_guard"
        | "ui.sound_on_complete"
        | "ui.close_to_tray"
        | "engine.window_enabled"
        // "false" is a legitimate write: it puts a profile back into first-run
        // without anyone having to delete the database.
        | "system.setup_completed"
        // Writing the log is the default; only an explicit `false` stops it.
        | "system.debug_log" => boolean(),
        // One decision in four keys: the times are the `HH:MM` an `<input
        // type="time">` gives back, judged by the same parser the engine reads
        // with, so a value the Settings page accepts is a value that cannot be
        // silently read as midnight.
        "engine.window_start" | "engine.window_stop" => {
            if crate::db::parse_minute(value).is_some() {
                Ok(())
            } else {
                Err(format!("{key} expects a 24-hour time like 22:00"))
            }
        }
        "engine.window_days" => match value.trim().parse::<u8>() {
            Ok(mask) if mask <= 0x7f => Ok(()),
            _ => Err(format!(
                "{key} expects 0-127, a Monday-first day mask; 0 is every day"
            )),
        },
        "storage.default_folder" => Ok(()),
        "storage.preallocation" => one_of(&["sparse", "full", "none"]),
        "network.bonding_policy" => one_of(&["dynamic", "hedging", "failover"]),
        "network.timeout_seconds" => one_of(&["15", "30", "60"]),
        "network.proxy_mode" => one_of(&["direct", "system", "manual"]),
        // The engine owns what it can read, so it is asked instead of a second
        // parser here that would have to be kept in step with the first.
        "network.proxy_url" => {
            if engine::proxy::accepts(value) {
                Ok(())
            } else {
                Err("network.proxy_url expects http://host:port or https://host:port \
                     and carries no credentials"
                    .to_string())
            }
        }
        "network.proxy_bypass" => {
            if engine::proxy::accepts_bypass(value) {
                Ok(())
            } else {
                Err("network.proxy_bypass expects comma-separated hosts, with * only at \
                     either end of an entry; a CIDR range such as 10.0.0.0/8 matches \
                     nothing"
                    .to_string())
            }
        }
        "ui.theme" => one_of(&["system", "light", "dark"]),        "ui.meter_style" => one_of(&["segmented", "wave"]),
        // `full` is deliberate, not a typo for a missing value: the renderer
        // lets this outrank prefers-reduced-motion, so the OS flag stops being
        // the only way to make the app still.
        "ui.motion" => one_of(&["system", "full", "reduced"]),
        // The check reads this over the network, so a value that is not a web
        // address is refused here instead of failing quietly at check time.
        "update.feed_url" => {
            if (value.starts_with("http://") || value.starts_with("https://"))
                && !value.contains(char::is_whitespace)
            {
                Ok(())
            } else {
                Err("update.feed_url expects an http:// or https:// address".to_string())
            }
        }
        _ => Err(format!("unknown setting key: {key}")),
    }
}

/// `null` and any non-positive value mean unlimited. `what` and `unit` only
/// shape the message, so a cap in bytes and a rate in bytes per second can share
/// the one rule instead of growing a second parser that must not drift.
fn byte_count_arg(
    value: Option<&Value>,
    what: &str,
    unit: &str,
) -> Result<Option<i64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(|n| (n > 0).then_some(n))
            .ok_or_else(|| format!("{what} expects whole {unit}, or null")),
    }
}

/// The window an allowance is counted over. Only `month` is honoured: the schema
/// `CHECK` at schema.sql accepts `day` and `week` too, so nothing below the IPC
/// boundary stops them, and a weekly allowance silently enforced as a monthly
/// one would be a lie on the card.
///
/// An allowance with no period is a number nothing can enforce, so a saved
/// allowance without one is stored as `month`; clearing the allowance clears the
/// period with it.
fn data_limit_period_arg(
    value: Option<&Value>,
    has_allowance: bool,
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(if has_allowance {
            Some("month".to_string())
        } else {
            None
        }),
        Some(Value::String(s)) if s == "month" => Ok(Some(s.clone())),
        Some(Value::String(s)) if s == "day" || s == "week" => Err(format!(
            "data allowance period '{s}' is not supported; only 'month' is"
        )),
        Some(_) => Err("data allowance period expects 'month' or null".to_string()),
    }
}

pub(crate) fn download_remove(state: &AppState, id: i64, delete_file: bool) -> Result<Value, String> {
    let row: Option<(Option<String>, Option<String>, String)> = {
        let db = lock(state)?;
        db.query_row(
            "SELECT staging_path, folder, filename FROM downloads WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok()
    };
    state.engine.remove_pending(id);
    state.engine.signal(id, job::STOP);
    // The job must be gone before the row is deleted: SQLite hands the freed
    // rowid to the next insert, and a still-live job would then write its
    // final status onto the replacement row and hold the staging file open.
    // Waiting is not the same as succeeding — a worker blocked in a read can
    // outlast the timeout — so the wait's answer decides whether anything is
    // thrown away. On a no the row, its staging file and its partial bytes all
    // stay put, and the latched stop lets the job retire onto its own row.
    if !state.engine.wait_idle(id, std::time::Duration::from_secs(6)) {
        return Ok(json!({ "fileDeleted": false, "filePresent": false, "removed": false }));
    }
    {
        let db = lock(state)?;
        db.execute("DELETE FROM downloads WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
    }
    let mut file_deleted = false;
    let mut file_present = false;
    if let Some((staging, folder, filename)) = row {
        if let Some(path) = staging {
            let _ = std::fs::remove_file(path);
        }
        if delete_file {
            if let Some(folder) = folder {
                let path = std::path::Path::new(&folder).join(filename);
                file_present = path.is_file();
                file_deleted = file_present && std::fs::remove_file(&path).is_ok();
            }
        }
    }
    // The renderer asks whether the file actually went so it can say so honestly
    // when it was already missing, locked, or outside our write rights.
    Ok(json!({ "fileDeleted": file_deleted, "filePresent": file_present, "removed": true }))
}

fn download_open(state: &AppState, id: i64) -> Result<Value, String> {
    if state.smoke {
        return Ok(json!({ "ok": true }));
    }
    let row: Option<(Option<String>, String)> = {
        let db = lock(state)?;
        db.query_row(
            "SELECT folder, filename FROM downloads WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    };
    if let Some((folder, filename)) = row {
        let dir = folder.unwrap_or_else(|| state.paths.downloads_dir.clone());
        let path = std::path::Path::new(&dir).join(filename);
        if !path.exists() {
            return Ok(json!({ "ok": false, "error": "file_not_found" }));
        }
        // No shell in this path — see pick::open for why that matters. The code
        // is the shell's own answer, so the UI can say what went wrong instead
        // of only that a spawn worked.
        return Ok(match crate::pick::open(&path) {
            Ok(()) => json!({ "ok": true }),
            Err(code) => json!({ "ok": false, "error": format!("shell_rejected_code_{code}") }),
        });
    }
    Ok(json!({ "ok": false, "error": "download_not_found" }))
}

fn download_reveal(state: &AppState, id: i64) -> Result<Value, String> {
    if state.smoke {
        return Ok(json!({ "ok": true }));
    }
    let row: Option<(Option<String>, String)> = {
        let db = lock(state)?;
        db.query_row(
            "SELECT folder, filename FROM downloads WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    };
    if let Some((folder, filename)) = row {
        let dir = folder.unwrap_or_else(|| state.paths.downloads_dir.clone());
        let path = std::path::Path::new(&dir).join(filename);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let mut cmd = std::process::Command::new("explorer");
            if path.exists() {
                cmd.arg(format!("/select,{}", path.to_string_lossy()));
            } else {
                cmd.arg(&dir);
            }
            let res = cmd.creation_flags(CREATE_NO_WINDOW).spawn();
            return Ok(json!({ "ok": res.is_ok() }));
        }
        #[cfg(not(windows))]
        {
            let _ = &path;
            let _ = &dir;
            return Ok(json!({ "ok": false }));
        }
    }
    Ok(json!({ "ok": false, "error": "download_not_found" }))
}

/// Full snapshot of the Sucker Map: the coordinator's live state while the job
/// runs, otherwise the `blocks` / `block_attempts` rows it left behind.
fn download_map(state: &AppState, id: i64) -> Result<Value, String> {
    // The row stops saying "downloading" before the job thread unregisters, so
    // a map request in that window must still read the rows, not a dead map.
    let running = {
        let db = lock(state)?;
        db.query_row("SELECT status FROM downloads WHERE id = ?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .map(|s| s == "downloading")
        .unwrap_or(false)
    };
    if running {
        if let Some(live) = state.engine.live(id) {
            if let Some(snapshot) = live.snapshot() {
                return Ok(map_payload(id, &snapshot, live.rev(), true));
            }
        }
    }
    block_map_from_db(state, id)
}

fn download_watch(state: &AppState, id: i64) -> Result<Value, String> {
    if let Some(live) = state.engine.live(id) {
        live.watch();
    }
    download_map(state, id)
}

fn download_unwatch(state: &AppState, id: i64) -> Result<Value, String> {
    if let Some(live) = state.engine.live(id) {
        live.unwatch();
    }
    Ok(Value::Null)
}

fn map_payload(id: i64, snapshot: &job::live::LiveState, rev: u64, full: bool) -> Value {
    let mut value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert("id".into(), json!(id));
        map.insert("rev".into(), json!(rev));
        map.insert("full".into(), json!(full));
    }
    value
}

fn block_map_from_db(state: &AppState, id: i64) -> Result<Value, String> {
    let db = lock(state)?;
    let meta: Option<(Option<i64>, Option<i64>)> = db
        .query_row(
            "SELECT total_bytes, block_size FROM downloads WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let Some((total, block_size)) = meta else {
        return Ok(Value::Null);
    };

    // Attempts first, because a finished block's stripe has to say which device
    // actually carried it, and that only lives on the attempt rows.
    let mut attempts = Vec::new();
    {
        let mut stmt = db
            .prepare(
                "SELECT a.id, a.block_idx, a.started_at, a.bytes, a.state, a.reason, \
                 b.range_start, b.range_end, a.ended_at, d.total_bytes, \
                 COALESCE(n.color_id, 'n0'), \
                 COALESCE(NULLIF(n.custom_name, ''), n.display_name, 'Default route'), \
                 a.detail \
                 FROM block_attempts a \
                 LEFT JOIN blocks b ON b.download_id = a.download_id AND b.idx = a.block_idx \
                 LEFT JOIN downloads d ON d.id = a.download_id \
                 LEFT JOIN networks n ON n.id = a.network_id \
                 WHERE a.download_id = ?1 ORDER BY a.id DESC LIMIT 64",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([id], |r| {
                let started_at = r.get::<_, i64>(2)?;
                let ended_at = r.get::<_, Option<i64>>(8)?;
                // A single-stream job has no block rows; its one attempt spans
                // the whole file instead of a zero-width range.
                let whole = r.get::<_, Option<i64>>(9)?.unwrap_or(0);
                let block_idx = r.get::<_, i64>(1)?;
                let color = r.get::<_, String>(10)?;
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "networkId": color,
                    "networkLabel": r.get::<_, String>(11)?,
                    "blockIdx": block_idx,
                    "rangeStart": r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    "rangeEnd": r.get::<_, Option<i64>>(7)?.unwrap_or(whole.saturating_sub(1)),
                    "bytes": r.get::<_, i64>(3)?,
                    "state": r.get::<_, String>(4)?,
                    "startedAt": started_at,
                    "durationMs": ended_at.map(|e| (e - started_at).max(0)).unwrap_or(0),
                    "reason": r.get::<_, Option<String>>(5)?,
                    "detail": r.get::<_, Option<String>>(12)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            attempts.push(row.map_err(|e| e.to_string())?);
        }
        attempts.reverse();
    }

    let mut blocks = Vec::new();
    {
        let mut stmt = db
            .prepare(
                "SELECT b.idx, b.range_start, b.range_end, b.bytes_done, b.state, \
                 COALESCE(n.color_id, 'n0'), \
                 COALESCE(NULLIF(n.custom_name, ''), n.display_name) \
                 FROM blocks b \
                 LEFT JOIN (SELECT download_id, block_idx, MAX(id) AS newest \
                            FROM block_attempts GROUP BY download_id, block_idx) t \
                   ON t.download_id = b.download_id AND t.block_idx = b.idx \
                 LEFT JOIN block_attempts a ON a.id = t.newest \
                 LEFT JOIN networks n ON n.id = a.network_id \
                 WHERE b.download_id = ?1 ORDER BY b.idx",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([id], |r| {
                let start: i64 = r.get(1)?;
                let end: i64 = r.get(2)?;
                let bytes_done = r.get::<_, i64>(3)?;
                // A block nobody has pulled yet has no device to name, and
                // guessing from the current adapter list would be a lie. A block
                // that does have bytes wears the stripe its own newest attempt
                // recorded, however far back that attempt is: the attempt list
                // below is a window, and attribution must not end where the
                // window does. An attempt with no route at all still joins to no
                // networks row, so it comes back as n0 and says so.
                let color = if bytes_done > 0 {
                    r.get::<_, String>(5)?
                } else {
                    job::live::UNATTRIBUTED.to_string()
                };
                Ok(json!({
                    "idx": r.get::<_, i64>(0)?,
                    "offset": start,
                    "len": end - start + 1,
                    "bytesDone": bytes_done,
                    "state": r.get::<_, String>(4)?,
                    "networkId": color,
                    "networkLabel": if bytes_done > 0 { r.get::<_, Option<String>>(6)? } else { None },
                }))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            blocks.push(row.map_err(|e| e.to_string())?);
        }
    }

    Ok(json!({
        "id": id,
        "rev": 0,
        "full": true,
        "blockSize": block_size.unwrap_or(0),
        "total": total.unwrap_or(0),
        "blocks": blocks,
        "attempts": attempts,
    }))
}

/// The adapters this machine can actually put bytes on, right now.
///
/// Windows decides which rows exist; SQLite decides whether we may use them.
/// Enumerating on every call is what keeps the sidebar count honest — a stored
/// list would keep showing a cable that was pulled five minutes ago.
fn network_rows(state: &AppState) -> Result<Value, String> {
    let adapters = engine::network::enumerate().unwrap_or_default();
    let db = lock(state)?;
    network_payload(&db, &adapters, state)
}

/// Records what Windows reports and hands back the rows the deck renders.
///
/// The same pass puts the live per-adapter caps, allowances and byte counts on
/// `AppState`, because it is the only place that reads them: `note_network` has
/// the saved cap and allowance in hand and `network_usage` has the three windows,
/// so carrying each one step further costs no query. Measured against a read of
/// its own for the same two answers, a separate query was 10,752 bytes of release
/// binary for a pair of `rusqlite` row-mapping instantiations.
///
/// Locks in this order — database, then the maps — and nothing may take the
/// database lock while holding one of the map guards, or this pass and a worker
/// booking its bytes deadlock against each other.
pub(crate) fn network_payload(
    conn: &Connection,
    adapters: &[engine::network::Adapter],
    state: &AppState,
) -> Result<Value, String> {
    let mut out = Vec::with_capacity(adapters.len());

    for adapter in adapters {
        let name = engine::network::display_name(adapter);
        let prefs = crate::db::note_network(
            conn,
            &engine::network::identity(adapter),
            &name,
            adapter.kind.as_str(),
            // A tunnel is not extra bandwidth: routing downloads through a VPN
            // endpoint is a choice, never a default.
            !adapter.virtual_,
        )?;
        let usage = crate::db::network_usage(conn, prefs.id)?;
        state.set_network_cap(prefs.id, prefs.speed_limit);
        state.set_network_quota(
            prefs.id,
            prefs.data_limit,
            engine::job::NetScope::window_for(prefs.data_limit_period.as_deref()),
        );
        state.set_network_usage(prefs.id, usage.0, usage.1, usage.2);

        out.push(json!({
            "id": prefs.id,
            "ifIndex": adapter.if_index,
            "displayName": prefs.custom_name.clone().unwrap_or_else(|| name.clone()),
            "kind": adapter.kind.as_str(),
            "colorId": prefs.color_id,
            "enabled": prefs.enabled,
            "speedLimit": prefs.speed_limit,
            "dataLimit": prefs.data_limit,
            "dataLimitPeriod": prefs.data_limit_period,
            "localIp": adapter.local_ip,
            "gateway": adapter.gateway,
            "linkSpeedBps": adapter.link_speed_bps,
            "ipv4Metric": adapter.ipv4_metric,
            "virtual": adapter.virtual_,
            "usedToday": usage.0,
            "usedWeek": usage.1,
            "usedMonth": usage.2,
            "usedLifetime": usage.3,
        }));
    }

    let unattributed = crate::db::unattributed_usage(conn)?;
    Ok(json!({
        // The adapters, and beside them the bytes that no adapter carried. Two
        // figures in one answer rather than a row appended to the list: every
        // consumer of `adapters` — the cards, the bond count, the shares, the
        // limiter lookups the gate reads back — asks which device this is, and
        // this one is nobody's.
        "adapters": Value::Array(out),
        "unattributed": {
            "today": unattributed.0,
            "week": unattributed.1,
            "month": unattributed.2,
            "lifetime": unattributed.3,
        },
    }))
}

/// The routes one new job may spread its chunks over.
///
/// Two rules decide the shape:
/// - nothing bonded, or no adapter we can bind, leaves the socket to Windows.
///   A download must not fail because a dongle was pulled out.
/// - one usable adapter does not bind either. With a single route the source
///   address buys no bandwidth and no attribution the routing table has not
///   already given us, and it costs real reachability: a socket bound to
///   192.168.1.2 cannot answer 127.0.0.1 at all (`EADDRNOTAVAIL`), so every
///   local server would fail for a machine that has one cable and no bond.
///   Bonding starts at two.
fn bond_routes(conn: &Connection) -> Vec<engine::bond::Route> {
    let members = crate::db::bond_members(conn).unwrap_or_default();
    if members.is_empty() {
        return Vec::new();
    }
    let adapters = engine::network::enumerate().unwrap_or_default();
    let guard = metered_guard(conn);
    let all = resolve_routes(&members, &adapters, false);

    // An enabled row whose adapter is gone is not a route. Rows are never deleted
    // (net_usage and block_attempts point at them), so a pulled or renamed
    // interface would otherwise keep the default route alive after the user
    // switched off every adapter the Networks pane actually shows.
    if all.is_empty() {
        return all;
    }
    let routes = if guard {
        resolve_routes(&members, &adapters, true)
    } else {
        all
    };
    // One route cannot bond, and a guard that held back the only *other* card
    // leaves one: either way the job runs unbonded on the OS default route. The
    // guard keeps a cellular link out of the pool, it does not forbid using a
    // machine that has nothing else.
    if routes.len() < 2 {
        return vec![engine::bond::Route::default_route()];
    }
    routes
}

/// The bonding strategy this job will run under.
///
/// Read with the routes and frozen for the download, so changing the setting
/// mid-transfer cannot re-attribute bytes already booked to the old strategy.
fn bonding_policy(conn: &Connection) -> job::BondPolicy {
    crate::db::setting(conn, "network.bonding_policy")
        .ok()
        .flatten()
        .map(|v| job::BondPolicy::from_setting(&v))
        .unwrap_or(job::BondPolicy::Dynamic)
}

/// Whether cellular adapters stay out of the bonding pool.
///
/// On unless the user explicitly stored `false`, because the wizard ships the
/// toggle checked and a missing row means "never asked yet".
fn metered_guard(conn: &Connection) -> bool {
    crate::db::setting(conn, "network.metered_guard")
        .ok()
        .flatten()
        .map(|v| v != "false")
        .unwrap_or(true)
}

/// Pairs stored members with the adapters present right now. A member with no
/// live adapter is dropped, and so is an adapter with no enabled row.
///
/// With `hold_back_cellular` a mobile-broadband adapter is skipped: Windows
/// reports a tethered phone as a USB or mobile adapter depending on the phone,
/// so this catches the class the OS names, not every metered link.
fn resolve_routes(
    members: &[crate::db::BondMember],
    adapters: &[engine::network::Adapter],
    hold_back_cellular: bool,
) -> Vec<engine::bond::Route> {
    let mut routes = Vec::with_capacity(members.len());
    for adapter in adapters {
        if hold_back_cellular && adapter.kind == engine::network::Kind::Cellular {
            continue;
        }
        let hint = engine::network::identity(adapter);
        let Some(member) = members.iter().find(|m| m.mac_hint == hint) else {
            continue;
        };
        let Ok(bind) = adapter.local_ip.parse() else {
            continue;
        };
        routes.push(engine::bond::Route {
            network_id: Some(member.id),
            // The colour the card was last drawn with, so the stripe on the map
            // and the Networks view agree even if the adapter list shifts later.
            color: member
                .color_id
                .clone()
                .unwrap_or_else(|| job::live::UNATTRIBUTED.to_string()),
            label: member.label.clone(),
            bind: Some(bind),
        });
    }
    routes
}

fn network_list(state: &AppState) -> Result<Value, String> {
    network_rows(state)
}

fn network_set_enabled(state: &AppState, id: i64, enabled: bool) -> Result<Value, String> {
    let db = lock(state)?;
    if !crate::db::set_network_enabled(&db, id, enabled)? {
        return Err(format!("no such network: {id}"));
    }
    // The same question a start asks: is there a route at all. Counting enabled
    // rows is not it — a row can outlive its adapter, and then the pane reads
    // "all disabled" while transfers keep running.
    if !enabled && bond_routes(&db).is_empty() {
        state.engine.pause_all();
    }
    Ok(json!({ "ok": true }))
}

/// Saves an adapter's rate cap and monthly allowance.
///
/// The rate cap goes to the live bucket as well as the row, because the watcher
/// only re-reads the ledger when an adapter's identity or name changes — a saved
/// cap that waited for that would not arrive until a cable was moved.
///
/// Still writes the row for both figures, and both now reach the transfer that is
/// already running: a saved cap paces the next block read on that adapter, and a
/// saved allowance stops new blocks leaving on it once the ledger says its window
/// is full. A cap or allowance saved here is therefore worth the click even
/// mid-download, which is what the card promises.
fn network_set_limits(
    state: &AppState,
    id: i64,
    speed_limit: Option<i64>,
    data_limit: Option<i64>,
    data_limit_period: Option<String>,
) -> Result<Value, String> {
    let changed = {
        let db = lock(state)?;
        crate::db::update_network_limits(
            &db,
            id,
            speed_limit,
            data_limit,
            data_limit_period.as_deref(),
        )?
    };
    if !changed {
        return Err(format!("no such network: {id}"));
    }
    state.set_network_cap(id, speed_limit);
    state.set_network_quota(
        id,
        data_limit,
        engine::job::NetScope::window_for(data_limit_period.as_deref()),
    );
    if data_limit.is_some() {
        // A scope this call just created has no byte count, and zero would let a
        // device that is already spent take one more block before the watcher's
        // next pass. One read on a click is cheap; the row was just written anyway.
        let db = lock(state)?;
        if let Ok((today, week, month, _lifetime)) = crate::db::network_usage(&db, id) {
            state.set_network_usage(id, today, week, month);
        }
    }
    Ok(json!({ "ok": true }))
}

fn category_list(state: &AppState) -> Result<Value, String> {
    let db = lock(state)?;
    let mut stmt = db
        .prepare(
            "SELECT id, name, folder, rule_tier, match_all, sort_order FROM categories \
             ORDER BY sort_order, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "folder": r.get::<_, Option<String>>(2)?,
                "ruleTier": r.get::<_, String>(3)?,
                "matchAll": r.get::<_, i64>(4)? == 1,
                "sortOrder": r.get::<_, i64>(5)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(Value::Array(out))
}

fn download_set_category(
    state: &AppState,
    id: i64,
    category_id: Option<i64>,
) -> Result<Value, String> {
    {
        let db = lock(state)?;
        crate::db::assign_category(&db, id, category_id)?;
    }
    download_get(state, id)
}

fn history_tail(state: &AppState, download_id: i64, limit: i64) -> Result<Value, String> {
    let db = lock(state)?;
    let mut stmt = db
        .prepare(
            "SELECT id, download_id, ts, kind, payload_json FROM history_events \
             WHERE download_id = ?1 ORDER BY ts DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([download_id, limit], |r| {
            let payload: String = r.get(4)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "downloadId": r.get::<_, i64>(1)?,
                "ts": r.get::<_, i64>(2)?,
                "kind": r.get::<_, String>(3)?,
                "payload": serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null),
            }))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_whose_adapter_is_gone_is_not_a_route() {
        let live = engine::network::Adapter {
            if_index: 12,
            name: "Ethernet".to_string(),
            description: "Realtek Gaming 2.5GbE".to_string(),
            kind: engine::network::Kind::Ethernet,
            mac: Some("A8:A1:59:12:B8:05".to_string()),
            local_ip: "192.168.1.2".to_string(),
            gateway: Some("192.168.1.1".to_string()),
            ipv4_metric: 35,
            link_speed_bps: 1_000_000_000,
            virtual_: false,
        };
        let member = |id: i64, mac: &str| crate::db::BondMember {
            id,
            mac_hint: mac.to_string(),
            color_id: None,
            label: format!("adapter {id}"),
        };

        // The user's real state: every visible adapter off, one stale row still
        // enabled. Pairing has to come back empty so the pane and the engine
        // agree that nothing may carry traffic.
        assert!(
            resolve_routes(&[member(4, "82:46:E2:A0:B3:57")], &[live.clone()], false).is_empty(),
            "a phantom member must not keep the default route alive"
        );

        // The same row, now actually present, is a route again.
        let routes = resolve_routes(
            &[member(4, "82:46:E2:A0:B3:57"), member(1, "A8:A1:59:12:B8:05")],
            &[live],
            false,
        );
        assert_eq!(routes.len(), 1, "only the member with a live adapter pairs");
        assert_eq!(routes[0].network_id, Some(1));
    }

    #[test]
    fn the_metered_guard_holds_cellular_out_of_the_pool_only() {
        let wired = engine::network::Adapter {
            if_index: 12,
            name: "Ethernet".to_string(),
            description: "Realtek Gaming 2.5GbE".to_string(),
            kind: engine::network::Kind::Ethernet,
            mac: Some("A8:A1:59:12:B8:05".to_string()),
            local_ip: "192.168.1.2".to_string(),
            gateway: Some("192.168.1.1".to_string()),
            ipv4_metric: 35,
            link_speed_bps: 1_000_000_000,
            virtual_: false,
        };
        let phone = engine::network::Adapter {
            if_index: 40,
            name: "Cellular".to_string(),
            description: "LTE Module".to_string(),
            kind: engine::network::Kind::Cellular,
            mac: Some("0E:00:21:4A:00:0C".to_string()),
            local_ip: "10.30.20.5".to_string(),
            gateway: Some("10.30.20.1".to_string()),
            ipv4_metric: 90,
            link_speed_bps: 0,
            virtual_: false,
        };
        let members = || {
            vec![
                crate::db::BondMember {
                    id: 1,
                    mac_hint: "A8:A1:59:12:B8:05".to_string(),
                    color_id: None,
                    label: "cable".to_string(),
                },
                crate::db::BondMember {
                    id: 2,
                    mac_hint: "0E:00:21:4A:00:0C".to_string(),
                    color_id: None,
                    label: "phone".to_string(),
                },
            ]
        };

        let both = resolve_routes(&members(), &[wired.clone(), phone.clone()], false);
        assert_eq!(both.len(), 2, "with the guard off the phone is a route");

        let guarded = resolve_routes(&members(), &[wired.clone(), phone.clone()], true);
        assert_eq!(guarded.len(), 1, "the guard holds the phone back");
        assert_eq!(guarded[0].network_id, Some(1));

        // The case that must not turn into a download ban: a machine whose only
        // card is cellular. `bond_routes` reads this emptiness as "nothing to
        // bond" and falls through to the default route, so the job still runs.
        let alone = resolve_routes(&members(), &[phone], true);
        assert!(alone.is_empty(), "the guard is not a way to refuse a job");
    }

    #[test]
    fn a_bogus_policy_string_is_dynamic_not_a_panic() {
        assert_eq!(
            job::BondPolicy::from_setting("hedging"),
            job::BondPolicy::Hedging
        );
        assert_eq!(
            job::BondPolicy::from_setting("failover"),
            job::BondPolicy::Failover
        );
        assert_eq!(
            job::BondPolicy::from_setting(""),
            job::BondPolicy::Dynamic,
            "an unset row must keep the behaviour the engine had before the setting existed"
        );
        assert_eq!(
            job::BondPolicy::from_setting("everything"),
            job::BondPolicy::Dynamic,
            "a value settings_set would not have accepted still has to resolve"
        );
    }

    /// The two tests above hand the decision its input: `from_setting` gets a
    /// `&str` and `resolve_routes` gets a `bool`. Neither touches the settings
    /// row, so a reader that ignored `network.*` and returned `Dynamic` / `true`
    /// forever would have stayed green. These two are that missing edge — the
    /// value goes in through `db::set_setting`, the same write `settings_set`
    /// performs, and comes back through the readers a new job actually calls.
    #[test]
    fn the_stored_bonding_policy_is_the_policy_a_new_job_acts_on() {
        let (dir, state) = scratch_state("bonding-key");
        let db = state.db.lock().expect("scratch lock");

        // Unset is what an install that never opened Settings looks like.
        assert_eq!(
            bonding_policy(&db),
            job::BondPolicy::Dynamic,
            "no row keeps the pre-setting behaviour"
        );

        for (stored, want) in [
            ("dynamic", job::BondPolicy::Dynamic),
            ("hedging", job::BondPolicy::Hedging),
            ("failover", job::BondPolicy::Failover),
        ] {
            crate::db::set_setting(&db, "network.bonding_policy", Some(stored)).expect("store");
            assert_eq!(bonding_policy(&db), want, "{stored} in the settings row");
        }

        // A cleared row is the unset case again, not an error the job eats.
        crate::db::set_setting(&db, "network.bonding_policy", None).expect("clear");
        assert_eq!(bonding_policy(&db), job::BondPolicy::Dynamic, "cleared row");

        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The guard half of the same gap. The guard's effect on the pool is already
    /// covered — `the_metered_guard_holds_cellular_out_of_the_pool_only` pairs a
    /// fake adapter set with the `bool` — so key -> guard -> pool is only proven
    /// end to end if this edge is.
    #[test]
    fn the_stored_metered_guard_decides_the_pool_the_job_spreads_over() {
        let (dir, state) = scratch_state("metered-key");
        let db = state.db.lock().expect("scratch lock");

        // "never asked yet" guards, because the wizard ships the toggle checked.
        assert!(metered_guard(&db), "no row guards the cellular link");

        crate::db::set_setting(&db, "network.metered_guard", Some("true")).expect("store");
        assert!(metered_guard(&db), "true guards");

        // The only off value the panel can write.
        crate::db::set_setting(&db, "network.metered_guard", Some("false")).expect("store");
        assert!(
            !metered_guard(&db),
            "false lets the phone into the pool"
        );

        // Recorded here because it is *not* reachable from Settings: the
        // validator admits only "true" and "false", yet this reader asks only for
        // != "false". A "0" row written by anything but the app therefore guards,
        // while job.rs's own bool reader treats "0" as off. The two readers of
        // the same boolean class disagree off the panel's path, not on it.
        crate::db::set_setting(&db, "network.metered_guard", Some("0")).expect("direct write");
        assert!(metered_guard(&db), "a row the panel cannot write still guards");

        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failover_keeps_the_primary_and_only_escalates_after_a_failure() {
        // Three routes, four workers: dynamic puts every worker on a different
        // card, and a retry moves again.
        let d = job::BondPolicy::Dynamic;
        assert_eq!((d.route_for(0, 0, 3), d.route_for(1, 0, 3), d.route_for(2, 0, 3)), (0, 1, 2));
        assert_eq!(d.route_for(0, 1, 3), 1, "a retried block leaves the route that failed");

        // Failover and hedging both leave the first attempt on the primary.
        for p in [job::BondPolicy::Failover, job::BondPolicy::Hedging] {
            assert_eq!(
                (p.route_for(0, 0, 3), p.route_for(1, 0, 3), p.route_for(2, 0, 3)),
                (0, 0, 0),
                "{p:?}: the backup link stays idle until the primary drops"
            );
            assert_eq!(p.route_for(0, 1, 3), 1, "{p:?}: the first failure moves up one");
            assert_eq!(p.route_for(0, 9, 3), 2, "{p:?}: escalation stops at the last route");
        }

        // Only failover refuses to move bytes for being slow.
        assert!(job::BondPolicy::Dynamic.steals());
        assert!(job::BondPolicy::Hedging.steals());
        assert!(!job::BondPolicy::Failover.steals(), "its copy says backup only if primary drops");
        assert!(job::BondPolicy::Hedging.hedges());
        assert!(!job::BondPolicy::Dynamic.hedges(), "dynamic steals the biggest tail, not the quietest");
    }

    #[test]
    fn a_hedge_leaves_on_a_different_card_than_the_stream_it_races() {
        let h = job::BondPolicy::Hedging;
        for worker in 0..4 {
            let r = h.secondary_route_for(worker, 0, 3);
            assert_ne!(r, 0, "worker {worker}: racing a stall on the same card re-opens it");
            assert!(r < 3);
        }
        assert_eq!(
            h.secondary_route_for(0, 0, 1),
            0,
            "with one route there is nowhere else to go, and index 0 beats a panic"
        );
    }

    #[test]
    fn every_channel_is_unique_and_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for channel in CHANNELS {
            assert!(seen.insert(channel), "duplicate channel {channel}");
            let (domain, verb) = channel.split_once(':').expect("domain:verb");
            assert!(!domain.is_empty() && !verb.is_empty(), "malformed {channel}");
        }
        assert_eq!(CHANNELS.len(), 38);
    }

    /// The write path for `network:set-limits`: the row is the only thing this
    /// step touches, so the test reads the three columns back rather than asking
    /// whether a transfer slowed.
    #[test]
    fn set_limits_writes_the_three_columns_and_clears_them() {
        let (dir, state) = scratch_state("set-limits");
        let id = {
            let db = state.db.lock().expect("scratch lock");
            db.execute(
                "INSERT INTO networks (mac_hint, display_name, kind, color_id, enabled) \
                 VALUES ('aa', 'First cable', 'ethernet', 'n1', 1)",
                [],
            )
            .expect("network row");
            db.last_insert_rowid()
        };

        network_set_limits(&state, id, Some(500_000), Some(1 << 30), Some("month".into()))
            .expect("save limits");
        let row = {
            let db = state.db.lock().expect("scratch lock");
            db.query_row(
                "SELECT speed_limit, data_limit, data_limit_period FROM networks WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .expect("read back")
        };
        assert_eq!(row, (Some(500_000), Some(1 << 30), Some("month".to_string())));

        // Clearing both writes NULL, which is what an unlimited adapter looks
        // like in every other read; a sentinel would make "no cap" a fourth value.
        network_set_limits(&state, id, None, None, None).expect("clear limits");
        let row = {
            let db = state.db.lock().expect("scratch lock");
            db.query_row(
                "SELECT speed_limit, data_limit, data_limit_period FROM networks WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .expect("read back")
        };
        assert_eq!(row, (None, None, None));

        let missing = network_set_limits(&state, id + 999, Some(1), None, None)
            .expect_err("no such row must be an error");
        assert!(missing.contains("no such network"), "{missing}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The period rule is enforced here rather than in the schema, because the
    /// schema's `CHECK` accepts `day` and `week` and nothing would stop a card
    /// from saving a weekly allowance that the engine then counts monthly.
    #[test]
    fn period_arg_rejects_the_windows_that_are_not_enforced() {
        let has_allowance = true;
        let month = Value::String("month".into());
        let hour = Value::String("hour".into());
        let none = Value::Null;
        assert_eq!(
            data_limit_period_arg(Some(&month), has_allowance).unwrap(),
            Some("month".to_string())
        );
        for rejected in ["day", "week"] {
            let value = Value::String(rejected.into());
            let err = data_limit_period_arg(Some(&value), has_allowance)
                .expect_err(&format!("{rejected} is not enforceable yet"));
            assert!(err.contains("only 'month' is"), "{err}");
        }
        assert!(data_limit_period_arg(Some(&hour), has_allowance).is_err());
        assert!(data_limit_period_arg(None, false).unwrap().is_none());
        // An allowance with no window is a number nothing can enforce.
        assert_eq!(
            data_limit_period_arg(Some(&none), true).unwrap(),
            Some("month".to_string())
        );
    }

    /// A library on disk with one row and no app around it.
    ///
    /// Ruling 5 moved the first-run flag out of localStorage into this KV, so the
    /// allowlist plus its validator is all that stands between the wizard and a
    /// typo. `false` stays legal: it puts a profile back into first run without
    /// anyone having to delete a database.
    #[test]
    fn the_first_run_flag_is_a_setting_and_still_validated() {
        assert!(SETTING_KEYS.contains(&"system.setup_completed"));
        assert!(validate_setting("system.setup_completed", "true").is_ok());
        assert!(validate_setting("system.setup_completed", "false").is_ok());
        assert!(validate_setting("system.setup_completed", "yes").is_err());
        assert!(validate_setting("system.setup_compltd", "true").is_err());
    }

    /// The debug log ships on, so the value that matters is the stored `false`
    /// that turns it off; a word that merely looks like a switch is refused.
    #[test]
    fn the_debug_switch_is_a_validated_boolean() {
        assert!(SETTING_KEYS.contains(&"system.debug_log"));
        assert!(validate_setting("system.debug_log", "true").is_ok());
        assert!(validate_setting("system.debug_log", "false").is_ok());
        for refused in ["on", "off", "yes", "FALSE"] {
            assert!(validate_setting("system.debug_log", refused).is_err(), "{refused}");
        }
    }

    /// The report button opens a web page, so the whole payload has to survive
    /// the query string: newlines and backslashes from the log, an em dash from
    /// the excerpt marker, and a title. Nothing may stay raw enough to break it.
    #[test]
    fn an_issue_link_carries_the_log_inside_a_web_address() {
        let url = issue_url("2026-11-10T05:07:05Z ERROR a\\b said \"no\"\nnext", "0.1.1");
        assert!(url.starts_with("https://github.com/benjaminspoop7-sys/sucr/issues/new?"));
        assert!(!url.contains('\n') && !url.contains(' '));
        assert!(url.contains("Sucr%200.1.1%20bug%20report"));
        assert!(url.contains("ERROR%20a%5Cb%20said%20%22no%22%0Anext"));
        assert_eq!(quoted("a b~c.d_e-f"), "a%20b~c.d_e-f");
        // A very long log is cut, not refused: the link stays a web address.
        let long = "x".repeat(5000);
        let url = issue_url(&long, "0.1.1");
        assert!(url.len() < 9000, "{}", url.len());
        assert!(url.contains("%E2%80%A6"), "the cut is marked");
    }

    /// Motion exists to contradict the OS, so its allowlist is the whole
    /// contract: `full` has to be writable, and a word that only looks like
    /// `false` must not be quietly accepted as `reduced`.
    #[test]
    fn the_motion_switch_takes_three_values_and_no_other() {
        assert!(SETTING_KEYS.contains(&"ui.motion"));
        for allowed in ["system", "full", "reduced"] {
            assert!(validate_setting("ui.motion", allowed).is_ok(), "{allowed}");
        }
        for refused in ["off", "true", "FULL", ""] {
            assert!(validate_setting("ui.motion", refused).is_err(), "{refused}");
        }
    }

    /// The window is one decision in four keys, and the two times are the shape
    /// the engine's own parser reads: a value the Settings page accepts must never
    /// resolve to midnight at download time.
    #[test]
    fn the_window_quartet_is_validated_at_the_boundary() {
        for key in [
            "engine.window_enabled",
            "engine.window_start",
            "engine.window_stop",
            "engine.window_days",
        ] {
            assert!(SETTING_KEYS.contains(&key), "{key}");
        }
        assert!(validate_setting("engine.window_enabled", "true").is_ok());
        assert!(validate_setting("engine.window_enabled", "yes").is_err());
        for allowed in ["00:00", "22:00", "06:30", "23:59"] {
            assert!(validate_setting("engine.window_start", allowed).is_ok(), "{allowed}");
        }
        for refused in ["24:00", "12:60", "2200", "half past nine", ""] {
            assert!(validate_setting("engine.window_stop", refused).is_err(), "{refused}");
        }
        for allowed in ["0", "1", "127"] {
            assert!(validate_setting("engine.window_days", allowed).is_ok(), "{allowed}");
        }
        for refused in ["128", "-1", "Mon,Tue", "0b1111001"] {
            assert!(validate_setting("engine.window_days", refused).is_err(), "{refused}");
        }
    }

    /// The feed is one address, judged before it can reach the network: a
    /// `file:` or relative value would otherwise become a path the check fails
    /// on later, where nobody sees why.
    #[test]
    fn the_update_feed_is_a_web_address_or_nothing() {
        assert!(SETTING_KEYS.contains(&"update.feed_url"));
        assert!(validate_setting("update.feed_url", "https://example.com/sucr.json").is_ok());
        assert!(validate_setting("update.feed_url", "http://127.0.0.1:8080/feed.json").is_ok());
        for refused in ["file:///C:/feed.json", "C:/feed.json", "example.com/feed", "https://a b"] {
            assert!(validate_setting("update.feed_url", refused).is_err(), "{refused}");
        }
    }

    /// The proxy is one decision in three keys, and a stored value has to be one
    /// the engine can read: an unusable URL is refused at the boundary instead of
    /// resolving to direct at download time, where nobody would see it.
    #[test]
    fn the_proxy_trio_is_validated_at_the_boundary() {
        for key in ["network.proxy_mode", "network.proxy_url", "network.proxy_bypass"] {
            assert!(SETTING_KEYS.contains(&key), "{key}");
        }
        assert!(validate_setting("network.proxy_mode", "manual").is_ok());
        assert!(validate_setting("network.proxy_mode", "socks").is_err());
        assert!(validate_setting("network.proxy_url", "http://proxy.corp:3128").is_ok());
        // Credentials in a URL are refused, not parsed: they belong to their own
        // table, and a password inside a link ends up on screen.
        assert!(validate_setting("network.proxy_url", "http://u:p@proxy.corp").is_err());
        // The shape a corporate bypass box invites, which matches nothing.
        assert!(validate_setting("network.proxy_bypass", "10.0.0.0/8").is_err());
        assert!(validate_setting("network.proxy_bypass", "internal.corp, .lab").is_ok());
    }

    /// Store, read back as one triple, mirror, and hand the answer to
    /// `app:get-info`: the write path in full, because a proxy that never
    /// reaches the engine is a setting that does nothing, and a Settings line
    /// that quoted the three keys instead of the engine would hide a mode that
    /// resolved to nothing. The cleared URL is the case an add-only test would
    /// miss — the mode stays, the route goes, and the label has to say so.
    ///
    /// One test owns the mirror for its whole body: `via` reads a process-wide
    /// state, so a second test writing it in parallel would show up as a race.
    #[test]
    fn a_stored_proxy_reaches_the_engine_mirror_and_the_page() {
        let (dir, state) = scratch_state("proxy-mirror");

        {
            let db = state.db.lock().expect("scratch lock");
            crate::db::set_setting(&db, "network.proxy_mode", Some("manual")).expect("store");
            crate::db::set_setting(&db, "network.proxy_url", Some("http://proxy.corp:3128"))
                .expect("store");
            crate::db::set_setting(&db, "network.proxy_bypass", Some("internal.corp"))
                .expect("store");
            let (mode, url, bypass) = crate::db::proxy_settings(&db).expect("read");
            engine::proxy::configure(&mode, &url, &bypass);
        }
        assert_eq!(app_info(&state).expect("info")["proxyVia"], "manual proxy.corp:3128");

        {
            let db = state.db.lock().expect("scratch lock");
            crate::db::set_setting(&db, "network.proxy_url", None).expect("clear");
            let (mode, url, bypass) = crate::db::proxy_settings(&db).expect("read");
            engine::proxy::configure(&mode, &url, &bypass);
        }
        assert_eq!(app_info(&state).expect("info")["proxyVia"], "manual direct");

        // Leave the mirror as an untouched install would find it: the bonding and
        // job tests in this binary transfer for real, and would otherwise spend
        // their time trying to resolve a proxy that does not exist.
        engine::proxy::configure("direct", "", "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The credential's whole path through the boundary: stored keyed to the proxy in
    /// front of the app, read back by the engine that sends it, named without its
    /// secret on the page, stranded when the URL moves, and gone when forgotten.
    #[test]
    fn a_proxy_credential_is_keyed_to_its_proxy_and_never_echoed() {
        let (dir, state) = scratch_state("proxy-credential");

        // Nothing to key to before a proxy exists, and the answer is the instruction.
        engine::proxy::configure("direct", "", "");
        let err = proxy_set_auth(&state, "agent", "s3cret").expect_err("no proxy yet");
        assert!(err.contains("manual proxy"), "{err}");

        engine::proxy::configure("manual", "http://proxy.corp:3128", "");
        let stored = proxy_set_auth(&state, "agent", "s3cret").expect("store");
        assert_eq!(stored["stored"], true);
        assert_eq!(stored["host"], "proxy.corp:3128");
        assert_eq!(stored["username"], "agent");
        assert_eq!(stored["passwordBytes"], 6);
        assert!(
            !stored.to_string().contains("s3cret"),
            "the reply is the page's, and the page does not get the secret"
        );

        // The engine is the other reader of the same row: what was stored is what the
        // next `Proxy` carries.
        let proxy = engine::proxy::resolve().expect("manual proxy");
        assert_eq!(proxy.username(), Some("agent"));
        assert_eq!(proxy.password(), Some("s3cret"));

        let info = app_info(&state).expect("info");
        assert_eq!(info["proxyAuth"]["stored"], true);
        assert_eq!(info["proxyAuth"]["applies"], true);
        assert_eq!(info["proxyAuth"]["host"], "proxy.corp:3128");
        assert!(!info.to_string().contains("s3cret"), "nor does the info card");

        // A moved URL strands it: the row is still there, still named, and no longer
        // sent anywhere.
        engine::proxy::configure("manual", "http://other.corp:3128", "");
        assert_eq!(app_info(&state).expect("info")["proxyAuth"]["applies"], false);
        assert_eq!(app_info(&state).expect("info")["proxyAuth"]["stored"], true);
        assert_eq!(engine::proxy::resolve().unwrap().username(), None);

        // Both boxes empty is how forgetting works.
        engine::proxy::configure("manual", "http://proxy.corp:3128", "");
        let forgotten = proxy_set_auth(&state, "", "").expect("forget");
        assert_eq!(forgotten["stored"], false);
        assert_eq!(forgotten["forgotten"], true);
        assert_eq!(crate::db::proxy_auth(&state.db.lock().expect("lock")).unwrap(), None);
        assert_eq!(app_info(&state).expect("info")["proxyAuth"]["stored"], false);
        assert_eq!(engine::proxy::resolve().unwrap().username(), None);

        // A password with no username is refused rather than stored: ureq would drop
        // it, and a credential that looks saved and never goes out is the worst
        // possible outcome of typing one in.
        let err = proxy_set_auth(&state, "", "s3cret").expect_err("nameless password");
        assert!(err.contains("username"), "{err}");
        assert_eq!(crate::db::proxy_auth(&state.db.lock().expect("lock")).unwrap(), None);

        // The pair is one field on the wire, so the two halves have their own bounds.
        let long = "x".repeat(crate::db::MAX_PROXY_SECRET_BYTES + 1);
        assert!(proxy_set_auth(&state, "agent", &long).is_err(), "a paste error is not a password");
        assert!(proxy_set_auth(&state, &"u".repeat(300), "p").is_err());
        // A username with no password is a real Basic pair, and the only shape a
        // token-as-username proxy takes.
        assert_eq!(proxy_set_auth(&state, "tok3n", "").expect("name only")["stored"], true);
        assert_eq!(engine::proxy::resolve().unwrap().password(), Some(""));

        engine::proxy::set_auth("", "", "");
        engine::proxy::configure("direct", "", "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The origin credential's whole path through the boundary: keyed to the host the
    /// link names, mirrored for the middleware that sends it, listed on the page without
    /// its secret, and separate per host — the one behaviour a proxy credential does not
    /// have, because there is one proxy at a time and there are many hosts.
    ///
    /// The pair and the token are both here, because the two kinds share every step of
    /// that path and differ in exactly one thing: which header the write turns into.
    #[test]
    fn an_origin_credential_is_keyed_to_its_host_and_never_echoed() {
        use engine::auth::Kind;
        let (dir, state) = scratch_state("origin-credential");

        let stored = origin_set_auth(
            &state,
            "http://Files.Example:8443/a.zip",
            Kind::Basic,
            "agent",
            "s3cret",
        )
        .expect("store");
        assert_eq!(stored["stored"], true);
        // The key is the engine's own, so a link and a bare host cannot make two rows for
        // one server, and a port never splits a host in two.
        assert_eq!(stored["host"], "files.example");
        assert_eq!(stored["kind"], "basic");
        assert_eq!(stored["username"], "agent");
        assert_eq!(stored["secretBytes"], 6);
        assert!(!stored.to_string().contains("s3cret"), "the reply has no secret in it");
        assert!(engine::auth::has("files.example"));

        // A second host is its own row.
        origin_set_auth(&state, "http://127.0.0.2/b.zip", Kind::Basic, "other", "pw2")
            .expect("second host");
        let info = app_info(&state).expect("info");
        let list = info["originAuths"].as_array().expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["host"], "files.example");
        assert_eq!(list[0]["secretBytes"], 6);
        assert!(
            !info.to_string().contains("s3cret") && !info.to_string().contains("pw2"),
            "the payload the page can ask for wholesale is the last place a password belongs"
        );

        // Forgetting is per host, and both boxes empty is how it happens.
        let forgotten = origin_set_auth(&state, "files.example", Kind::Basic, "", "").expect("forget");
        assert_eq!(forgotten["forgotten"], true);
        assert!(!engine::auth::has("files.example"));
        assert!(engine::auth::has("127.0.0.2"), "one host's forget is not another's");

        // The refusals, each with the reason the write path cannot work around: a name
        // is half a pair, a paste error is not a password, and nothing is half-stored.
        assert!(origin_set_auth(&state, "http://x.example", Kind::Basic, "", "s3cret").is_err());
        assert!(crate::db::origin_auths(&state.db.lock().expect("lock"))
            .expect("read")
            .iter()
            .all(|a| a.host != "x.example"));
        let long = "x".repeat(crate::db::MAX_ORIGIN_SECRET_BYTES + 1);
        assert!(origin_set_auth(&state, "http://x.example", Kind::Basic, "agent", &long).is_err());
        assert!(origin_set_auth(&state, "   ", Kind::Basic, "agent", "p").is_err(), "no host to key to");

        // The boot read is the other half of surviving a restart: everything the table
        // has, into the mirror, without the page asking for it.
        engine::auth::configure(Vec::new());
        assert!(!engine::auth::has("127.0.0.2"), "an empty mirror is empty");
        let rows = crate::db::origin_auths(&state.db.lock().expect("lock")).expect("read");
        engine::auth::configure(
            rows.into_iter()
                .map(|a| (a.host, a.kind, a.username, a.secret))
                .collect(),
        );
        assert!(engine::auth::has("127.0.0.2"));
        assert!(!engine::auth::has("files.example"), "the forgotten one stays gone");
        assert_eq!(
            engine::auth::authorization_for("127.0.0.2", false)
                .expect("header")
                .to_str()
                .expect("ascii"),
            "Basic b3RoZXI6cHcy",
            "the pair the boot read carried is the pair that goes out"
        );

        // A token on its own host: no username is stored or asked for, and the row the
        // page lists says `header-secret` — the word the table's own CHECK uses — so the
        // Settings list can name what it holds without holding it.
        let token = origin_set_auth(
            &state,
            "http://api.example/v1/data.bin",
            Kind::Bearer,
            "",
            "ey.abc-123",
        )
        .expect("store a token");
        assert_eq!(token["kind"], "header-secret");
        assert_eq!(token["username"], "", "a token has no name in front of it");
        assert_eq!(token["secretBytes"], 10);
        assert!(!token.to_string().contains("ey.abc-123"), "the reply has no token in it");
        assert_eq!(
            engine::auth::authorization_for("api.example", false)
                .expect("bearer")
                .to_str()
                .expect("ascii"),
            "Bearer ey.abc-123"
        );

        // One credential per host, and the write path enforces it: a token typed where a
        // pair lived leaves exactly one row, and it is the token's.
        origin_set_auth(&state, "http://api.example/other.bin", Kind::Basic, "agent", "pw")
            .expect("a pair first");
        assert_eq!(
            crate::db::origin_auths(&state.db.lock().expect("lock"))
                .expect("read")
                .iter()
                .filter(|a| a.host == "api.example")
                .count(),
            1,
            "the store holds one credential per host, not one per scheme"
        );
        origin_set_auth(&state, "http://api.example/other.bin", Kind::Bearer, "agent", "tok")
            .expect("then a token");
        let rows = crate::db::origin_auths(&state.db.lock().expect("lock")).expect("read");
        let api = rows.iter().find(|a| a.host == "api.example").expect("row");
        assert_eq!(api.kind, Kind::Bearer, "the name typed with a token is dropped");
        assert_eq!(api.username, "");
        assert_eq!(
            engine::auth::authorization_for("api.example", false)
                .expect("header")
                .to_str()
                .expect("ascii"),
            "Bearer tok",
            "and the mirror sends the token, not the pair it replaced"
        );

        // An empty token is the forget for either kind, asked by host — which is what the
        // row's own button can do without remembering what it stored.
        let forgotten = origin_set_auth(&state, "api.example", Kind::Basic, "", "").expect("forget");
        assert_eq!(forgotten["forgotten"], true, "forgetting a token row works");
        assert!(!engine::auth::has("api.example"));

        // A pasted cookie session shares this table, and forgetting a host's credential
        // must not touch it: the delete is by scheme name, not by host alone.
        crate::db::put_session(
            &state.db.lock().expect("lock"),
            "api.example",
            "cf_clearance=abc",
            Some("Mozilla/5.0"),
            crate::now_ms(),
        )
        .expect("store a session");
        origin_set_auth(&state, "api.example", Kind::Bearer, "", "tok").expect("token beside it");
        origin_set_auth(&state, "api.example", Kind::Basic, "", "").expect("forget the token");
        assert!(
            crate::db::session_for(&state.db.lock().expect("lock"), "api.example")
                .expect("read")
                .is_some(),
            "an origin forget left the cookie row alone"
        );

        engine::auth::configure(Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pass over the adapter table is what puts a saved cap on a live bucket:
    /// it already holds the row and the month figure, so the mirror costs no query
    /// of its own. See `network_payload`.
    #[test]
    fn reading_the_adapter_table_puts_saved_caps_on_live_buckets() {
        let (dir, state) = scratch_state("network-registry");
        let adapter = |mac: &str, name: &str| engine::network::Adapter {
            if_index: 12,
            name: name.to_string(),
            description: "Realtek Gaming 2.5GbE".to_string(),
            kind: engine::network::Kind::Ethernet,
            mac: Some(mac.to_string()),
            local_ip: "192.168.1.2".to_string(),
            gateway: Some("192.168.1.1".to_string()),
            ipv4_metric: 35,
            link_speed_bps: 1_000_000_000,
            virtual_: false,
        };
        let wired = adapter("A8:A1:59:12:B8:05", "Ethernet");
        let spare = adapter("A8:A1:59:12:B8:06", "Tether");
        let present = [wired.clone(), spare.clone()];

        let (capped, free) = {
            let db = state.db.lock().expect("scratch lock");
            let one = crate::db::note_network(
                &db,
                "A8:A1:59:12:B8:05",
                "Ethernet",
                "ethernet",
                true,
            )
            .expect("row");
            crate::db::update_network_limits(&db, one.id, Some(1024 * 1024), None, None)
                .expect("limits");
            crate::db::add_net_usage(&db, one.id, 64 * 1024).expect("usage");
            let two =
                crate::db::note_network(&db, "A8:A1:59:12:B8:06", "Tether", "usb", true)
                    .expect("row");
            crate::db::add_net_usage(&db, two.id, 8 * 1024).expect("usage");
            let _ = network_payload(&db, &present, &state).expect("adapter rows");
            (one.id, two.id)
        };

        assert_eq!(
            state.network_bucket(capped).map(|b| b.cap()),
            Some(Some(1024 * 1024)),
            "the cap written on the row is the cap on the bucket"
        );
        assert!(
            state.network_bucket(free).is_none(),
            "an adapter with no saved cap gets no bucket"
        );
        {
            let scopes = state
                .network_scopes
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert_eq!(
                scopes.get(&capped).map(|scope| scope.used()),
                Some(64 * 1024),
                "the ledger's month figure reaches the scope that has a limit"
            );
            assert!(
                scopes.get(&free).is_none(),
                "an adapter with neither cap nor allowance has no scope to count into"
            );
        }

        // Clearing the cap is the case an answer that only added would miss: the
        // row stays, the number goes, and a bucket left behind would go on pacing
        // bytes the user stopped asking to pace.
        {
            let db = state.db.lock().expect("scratch lock");
            crate::db::update_network_limits(&db, capped, None, None, None).expect("clear");
            let _ = network_payload(&db, &present, &state).expect("second pass");
        }
        assert!(
            state.network_bucket(capped).is_none(),
            "a cleared cap leaves no bucket behind"
        );
        drop(state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `open`, `reveal` and `pick-folder` all answer `{ ok: true }` before they
    /// touch anything under `--smoke`, because a window or a modal in the gate is
    /// a hung gate. So the gate proves their wiring and nothing else — this is
    /// where the arms that read the row get exercised instead.
    fn scratch_state(name: &str) -> (std::path::PathBuf, AppState) {
        let dir = std::env::temp_dir().join(format!(
            "sucr-ipc-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db_path = dir.join("sucr.db");
        let conn = crate::db::open(&db_path).expect("scratch db");
        let state = AppState {
            paths: crate::paths::AppPaths {
                data_dir: dir.display().to_string(),
                db_path: db_path.display().to_string(),
                logs_dir: dir.display().to_string(),
                downloads_dir: dir.display().to_string(),
            },
            db: std::sync::Mutex::new(conn),
            started_at: 0,
            smoke: false,
            engine: std::sync::Arc::new(job::Registry::default()),
            global_limit: std::sync::Arc::new(engine::limit::Bucket::new(None)),
            network_scopes: std::sync::Arc::new(std::sync::RwLock::new(
                std::collections::BTreeMap::new(),
            )),
        };
        (dir, state)
    }

    fn insert_row(state: &AppState, folder: &str, filename: &str) -> i64 {
        let db = state.db.lock().expect("scratch lock");
        db.execute(
            "INSERT INTO downloads (url, filename, folder, created_at) VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![format!("https://x.test/{filename}"), filename, folder],
        )
        .expect("insert");
        db.last_insert_rowid()
    }

    /// A chunk wears the colour of the attempt that finished it, and the attempt
    /// list a map carries is a window of the newest 64. A job that retried can
    /// finish its early blocks long before that window still reaches them, and
    /// the map must not then announce those bytes as nobody's: 440 MB that really
    /// rode one cable read as "Not attributed" when the join was made from the
    /// truncated list instead of from each block's own newest attempt.
    #[test]
    fn a_stripe_survives_its_attempt_falling_out_of_the_recent_window() {
        let (dir, state) = scratch_state("stripe-window");
        let id = insert_row(&state, &dir.display().to_string(), "wide.bin");
        {
            let db = state.db.lock().expect("scratch lock");
            db.execute(
                "UPDATE downloads SET total_bytes = 300, block_size = 100 WHERE id = ?1",
                rusqlite::params![id],
            )
            .expect("shape");
            db.execute(
                "INSERT INTO networks (mac_hint, display_name, kind, color_id, enabled) \
                 VALUES ('aa', 'First cable', 'ethernet', 'n1', 1), \
                        ('bb', 'Second cable', 'ethernet', 'n2', 1)",
                [],
            )
            .expect("networks");
            for idx in 0..3i64 {
                db.execute(
                    "INSERT INTO blocks (download_id, idx, range_start, range_end, state, bytes_done) \
                     VALUES (?1, ?2, ?3, ?4, 'done', 100)",
                    rusqlite::params![id, idx, idx * 100, idx * 100 + 99],
                )
                .expect("block");
            }
            // The oldest attempt in the table, and the only one for block 0.
            db.execute(
                "INSERT INTO block_attempts (download_id, block_idx, network_id, started_at, bytes, state) \
                 VALUES (?1, 0, 1, 1, 100, 'done')",
                rusqlite::params![id],
            )
            .expect("first attempt");
            // Then enough retries on the second cable to push the first out of
            // the window the attempt list reads.
            for i in 0..70i64 {
                db.execute(
                    "INSERT INTO block_attempts (download_id, block_idx, network_id, started_at, bytes, state) \
                     VALUES (?1, 1, 2, ?2, 50, 'done')",
                    rusqlite::params![id, 2 + i],
                )
                .expect("retry");
            }
            // A block whose attempts name no route at all: the honest n0.
            db.execute(
                "INSERT INTO block_attempts (download_id, block_idx, network_id, started_at, bytes, state) \
                 VALUES (?1, 2, NULL, 90, 100, 'done')",
                rusqlite::params![id],
            )
            .expect("unrouted attempt");
        }

        let map = block_map_from_db(&state, id).expect("durable map");
        let blocks = map["blocks"].as_array().expect("blocks array");
        assert_eq!(blocks.len(), 3, "{map}");
        assert_eq!(blocks[0]["networkId"], "n1", "an early block keeps its cable");
        assert_eq!(blocks[0]["networkLabel"], "First cable");
        assert_eq!(blocks[1]["networkId"], "n2");
        assert_eq!(
            blocks[2]["networkId"],
            job::live::UNATTRIBUTED,
            "bytes the engine never bound do belong to no cable"
        );
        assert_eq!(
            map["attempts"].as_array().expect("attempts").len(),
            64,
            "the attempt list stays a window; attribution must not be its side effect"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_answers_with_the_reason_not_a_window() {
        let (dir, state) = scratch_state("open");
        let id = insert_row(&state, &dir.display().to_string(), "ghost.bin");

        // The row is real and the file is not. Returning ok here would let the UI
        // claim it opened something.
        let missing = download_open(&state, id).expect("open missing");
        assert_eq!(missing["error"], "file_not_found", "{missing}");
        assert_eq!(missing["ok"], Value::Bool(false));

        let gone = download_open(&state, 999_999).expect("open unknown");
        assert_eq!(gone["error"], "download_not_found", "{gone}");

        // Reveal on an unknown id is the only arm tested: the row-present arms
        // start Explorer, and `pick::tests` covers the shell calls by hand.
        let gone = download_reveal(&state, 999_999).expect("reveal unknown");
        assert_eq!(gone["error"], "download_not_found", "{gone}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn set_status(state: &AppState, id: i64, status: &str) {
        let db = state.db.lock().expect("scratch lock");
        db.execute(
            "UPDATE downloads SET status = ?1 WHERE id = ?2",
            rusqlite::params![status, id],
        )
        .expect("set status");
    }

    fn status_of(state: &AppState, id: i64) -> String {
        let db = state.db.lock().expect("scratch lock");
        db.query_row("SELECT status FROM downloads WHERE id = ?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .expect("read status")
    }

    /// `downloading` is the one status only a deliberate Start writes, so a row left
    /// holding it by a machine that stopped is a transfer the user asked for. The
    /// reclaim turns it back into `queued` — what it really is until a slot claims it
    /// — and hands it to the queue, oldest first, exactly as a Start would have.
    #[test]
    fn a_transfer_the_machine_stopped_under_is_queued_for_a_slot() {
        let (dir, state) = scratch_state("reclaim-resumes");
        let first = insert_row(&state, &dir.display().to_string(), "first.bin");
        let second = insert_row(&state, &dir.display().to_string(), "second.bin");
        set_status(&state, first, "downloading");
        set_status(&state, second, "downloading");

        let ids = reclaim_interrupted(&state).expect("reclaim");
        assert_eq!(ids, vec![first, second], "both asked-for transfers found");

        // A row claiming an activity that has stopped is the lie this removes.
        assert_eq!(status_of(&state, first), "queued");
        assert_eq!(status_of(&state, second), "queued");

        // Claimed in the order a Start would have claimed them.
        assert_eq!(
            state.engine.pop_pending(),
            Some(first),
            "the older transfer waits for a slot first"
        );
        assert_eq!(state.engine.pop_pending(), Some(second));
        assert_eq!(state.engine.pop_pending(), None, "and nothing else");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The queue is in memory so that only a deliberate Start launches a queued row,
    /// and the reclaim must not break that from the other side: a row `queued` was
    /// added with auto-start off, a `paused` row is waiting for its user, and a failed
    /// row is his to retry. None of them may be collected here, or launched.
    #[test]
    fn the_reclaim_collects_only_the_rows_a_start_ever_touched() {
        let (dir, state) = scratch_state("reclaim-hands-off");
        let added = insert_row(&state, &dir.display().to_string(), "added.bin");
        let paused = insert_row(&state, &dir.display().to_string(), "paused.bin");
        let failed = insert_row(&state, &dir.display().to_string(), "failed.bin");
        let done = insert_row(&state, &dir.display().to_string(), "done.bin");
        set_status(&state, paused, "paused");
        set_status(&state, failed, "needs-attention");
        set_status(&state, done, "finished");

        assert_eq!(
            reclaim_interrupted(&state).expect("reclaim"),
            Vec::<i64>::new(),
            "no row was ever started"
        );
        assert_eq!(status_of(&state, added), "queued", "still its user's");
        assert_eq!(status_of(&state, paused), "paused");
        assert_eq!(status_of(&state, failed), "needs-attention");
        assert_eq!(status_of(&state, done), "finished");
        assert_eq!(
            state.engine.pop_pending(),
            None,
            "and the queue launched nothing"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A store that never had a transfer running must not reach the queue at all,
    /// because the caller only hands it over when the reclaim found something.
    #[test]
    fn an_empty_store_gives_the_queue_nothing_to_launch() {
        let (dir, state) = scratch_state("reclaim-empty");

        assert_eq!(
            reclaim_interrupted(&state).expect("reclaim"),
            Vec::<i64>::new(),
            "no rows at all"
        );
        assert_eq!(state.engine.pop_pending(), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reclaim must be safe to run twice — a second pass finds rows it already
    /// moved, and moving them again would hand the queue a row that is already
    /// waiting for a slot.
    #[test]
    fn a_second_reclaim_finds_nothing_the_first_moved() {
        let (dir, state) = scratch_state("reclaim-twice");
        let id = insert_row(&state, &dir.display().to_string(), "twice.bin");
        set_status(&state, id, "downloading");

        assert_eq!(reclaim_interrupted(&state).expect("first"), vec![id]);
        assert_eq!(
            reclaim_interrupted(&state).expect("second"),
            Vec::<i64>::new(),
            "the row is already queued"
        );
        assert_eq!(status_of(&state, id), "queued");
        assert_eq!(
            state.engine.pop_pending(),
            Some(id),
            "waiting for a slot once, not twice"
        );
        assert_eq!(state.engine.pop_pending(), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A store that cannot be read is not a reason to launch anything: the caller only
    /// hands the queue over when the reclaim found rows, so a reported break has to
    /// leave the queue untouched rather than launching whatever it failed to read.
    #[test]
    fn a_store_that_cannot_be_read_is_reported_and_launches_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "sucr-ipc-test-{}-reclaim-broken",
            std::process::id()
        ));
        let state = AppState {
            paths: crate::paths::AppPaths {
                data_dir: dir.display().to_string(),
                db_path: dir.display().to_string(),
                logs_dir: dir.display().to_string(),
                downloads_dir: dir.display().to_string(),
            },
            // A connection with no schema behind it: every read of `downloads` fails.
            db: std::sync::Mutex::new(
                rusqlite::Connection::open_in_memory().expect("bare connection"),
            ),
            started_at: 0,
            smoke: false,
            engine: std::sync::Arc::new(job::Registry::default()),
            global_limit: std::sync::Arc::new(engine::limit::Bucket::new(None)),
            network_scopes: std::sync::Arc::new(std::sync::RwLock::new(
                std::collections::BTreeMap::new(),
            )),
        };

        assert!(
            reclaim_interrupted(&state).is_err(),
            "the break is reported"
        );
        assert_eq!(
            state.engine.pop_pending(),
            None,
            "and nothing was handed to the queue"
        );
        assert_eq!(state.engine.running_count(), 0, "and nothing was launched");
    }
}
