mod capture;
mod db;
mod debug_log;
pub mod engine;
mod ipc;
mod keep_awake;
mod native_host;
mod paths;
mod pick;
mod updates;

/// Runs the native-messaging relay a browser spawned. Called from `main` before
/// any window exists, so this process never touches the database or the tray.
pub fn native_host_main() -> i32 {
    native_host::run()
}

/// Whether this process was started by a browser as its native host.
pub fn launched_by_browser(args: &[String]) -> bool {
    native_host::launched_by_browser(args)
}

/// The installer's and uninstaller's one command. Runs before any window,
/// database or tray exists: it copies the bundled add-on beside the database and
/// writes (or removes) the native-messaging registrations. The answer is JSON on
/// stdout so a hand run can be read; NSIS only waits for the exit code.
pub fn addon_cli(install: bool) -> i32 {
    let data_dir = paths::resolve_data_dir();
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("sucr.exe"));
    let report = if install {
        capture::install_addon(&data_dir, &exe)
    } else {
        capture::remove_addon(&data_dir)
    };
    println!("{report}");
    0
}

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::Manager;

/// Whether the notification-area icon was created. Closing the window only
/// hides Sucr when the tray exists to bring it back: without it, "Minimize"
/// would strand the app invisible with no way to reach or quit it.
static TRAY_READY: AtomicBool = AtomicBool::new(false);

pub struct AppState {
    pub paths: paths::AppPaths,
    pub db: Mutex<rusqlite::Connection>,
    pub started_at: i64,
    pub smoke: bool,
    pub engine: Arc<engine::job::Registry>,
    pub global_limit: Arc<engine::limit::Bucket>,
    /// One scope per `networks.id` — the adapter's rate bucket and its data
    /// allowance — keyed so a running job's route can find both. Starts empty:
    /// the `networks` rows do not exist yet when `run()` builds the state,
    /// because the adapter watcher creates them.
    ///
    /// `RwLock` rather than `Mutex` because the intended reader is every worker
    /// thread at once while one thread rebuilds the map.
    pub network_scopes: engine::job::NetworkScopes,
}

impl AppState {
    /// The live rate bucket for one `networks.id`, or `None` if the adapter has
    /// no cap.
    ///
    /// An adapter with neither limit has no entry: what a worker thread charges is
    /// a slice of buckets, and the whole point of the `Option` is that it leaves
    /// an unlimited scope out of the slice instead of locking a bucket that would
    /// answer "no wait" on every block read. An adapter with an allowance and no
    /// cap does get an entry, and its bucket is the unlimited one — the same shape
    /// as the shared global bucket, which is charged on every read already.
    pub fn network_bucket(&self, id: i64) -> Option<Arc<engine::limit::Bucket>> {
        self.network_scopes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id)
            .map(|scope| Arc::clone(&scope.bucket))
    }

    /// Puts an adapter's saved cap on the live scope.
    ///
    /// Updates the bucket that is already in the map rather than replacing it, so
    /// a job that resolved this id earlier holds the same `Arc` and reads the new
    /// ceiling on its next charge.
    ///
    /// Takes `i64` because that is what the column and the IPC argument are, and
    /// folds zero and negatives to "no cap" here rather than at each caller: a
    /// negative cast straight to `u64` would wrap into a colossal ceiling.
    pub fn set_network_cap(&self, id: i64, cap: Option<i64>) {
        let cap = cap.filter(|c| *c > 0).map(|c| c as u64);
        let mut map = self
            .network_scopes
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match map.get(&id) {
            Some(scope) => scope.set_cap(cap),
            None if cap.is_some() => {
                map.insert(id, Arc::new(engine::job::NetScope::new(cap)));
            }
            None => return,
        }
        prune_scope(&mut map, id);
    }

    /// Puts an adapter's saved data allowance, and the window it counts, on the
    /// live scope.
    ///
    /// No allowance is the state in which no route is ever ejected, so clearing
    /// one leaves the byte count alone rather than zeroing it: the count is the
    /// ledger's, and a user who raises a ceiling back to nothing should not have
    /// to wait for the next pass to be told what the adapter has really carried.
    pub fn set_network_quota(&self, id: i64, allowance: Option<i64>, window: u8) {
        let allowance = allowance.filter(|a| *a > 0).map(|a| a as u64);
        let mut map = self
            .network_scopes
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match map.get(&id) {
            Some(scope) => scope.set_allowance(allowance.unwrap_or(0), window),
            None if allowance.is_some() => {
                map.insert(
                    id,
                    Arc::new(engine::job::NetScope::with_allowance(
                        None,
                        allowance.unwrap_or(0),
                        window,
                    )),
                );
            }
            None => return,
        }
        prune_scope(&mut map, id);
    }

    /// Replaces one scope's byte count with the ledger's own sums, keeping the
    /// window its allowance is measured against.
    ///
    /// Written from the pass that already asked SQLite, so the quota check at a
    /// block boundary never takes the database lock. The ledger stays the record:
    /// this is a copy, and it is the copy that lets a window which rolled over
    /// while nobody was looking heal by itself.
    pub fn set_network_usage(&self, id: i64, today: i64, week: i64, month: i64) {
        if let Some(scope) = self
            .network_scopes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id)
        {
            scope.note_ledger(today, week, month);
        }
    }
}

/// Drops a scope nothing asked for any more.
///
/// An adapter whose cap was taken away keeps its entry only while an allowance is
/// still being counted: an entry is a mutex in every charge slice, and a bucket
/// that answers "no wait" for the rest of the session is not a limit.
fn prune_scope(map: &mut BTreeMap<i64, Arc<engine::job::NetScope>>, id: i64) {
    if map
        .get(&id)
        .is_some_and(|scope| !scope.load_bearing())
    {
        map.remove(&id);
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Re-reads the allowance figures a running job compares its routes against, from
/// the ledger the engine itself writes into.
///
/// Every pass, not only a changed one: a quota window closes on the clock, not
/// when a cable moves, and a route ejected for a window that has already rolled
/// over would stay ejected until somebody opened the Networks view. Scopes exist
/// only for adapters with a limit, so on a machine with no allowance set this
/// never takes the database lock at all.
///
/// Reads the map, releases it, then locks the database: never both at once, which
/// is the order every other pass in this file keeps.
#[cfg(desktop)]
pub(crate) fn refresh_network_quotas(state: &AppState) {
    let ids: Vec<i64> = state
        .network_scopes
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|(_, scope)| scope.allowance() > 0)
        .map(|(id, _)| *id)
        .collect();
    if ids.is_empty() {
        return;
    }
    let Ok(db) = state.db.lock() else { return };
    for id in ids {
        if let Ok((today, week, month, _lifetime)) = crate::db::network_usage(&db, id) {
            state.set_network_usage(id, today, week, month);
        }
    }
}

/// Whether the download window is open right now, read off the same database the
/// queue starts from.
///
/// A lock nobody can hand back answers "open": the window is a preference, and
/// the failure to read a preference is not a reason to stop transferring.
#[cfg(desktop)]
fn window_is_open(state: &AppState) -> bool {
    match state.db.lock() {
        Ok(db) => db::window_open_now(&db),
        Err(_) => true,
    }
}

/// The single IPC surface. Renderer calls arrive here as (channel, args) and are
/// dispatched by the same names shared/ipc.ts declares.
#[tauri::command]
fn sucr_invoke(
    channel: String,
    args: Vec<serde_json::Value>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let answer = ipc::dispatch(&channel, &args, &state, &app);
    // The refusals are the interesting half of a support report: the page already
    // shows the message, but a user who closed the window and came back later has
    // no way to see what the engine said. Only the message is kept — `args` can
    // carry a pasted session cookie.
    if let Err(message) = &answer {
        debug_log::write("ERROR", &format!("{channel} refused: {message}"));
    }
    answer
}

/// The link check has its own command because `sucr_invoke` is a sync command and
/// runs on the main thread: a probe through it would freeze the window for as
/// long as the host takes to answer. This one runs off-thread and opens the
/// library only to read one host's pasted session, so a link the user has already
/// unlocked is not greyed out.
#[tauri::command]
async fn sucr_probe(
    url: String,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db_path = state.paths.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ipc::probe_link(std::path::Path::new(&db_path), &url)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Only active under --smoke: writes the renderer's report and exits so the
/// M0/M1 gate can run headless in CI.
#[tauri::command]
fn smoke_report(
    report: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if !state.smoke {
        return Err("smoke mode is not enabled".into());
    }
    let ok = serde_json::from_str::<serde_json::Value>(&report)
        .ok()
        .and_then(|v| v.get("ok").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    std::fs::write(
        std::path::Path::new(&state.paths.data_dir).join("m0-smoke.json"),
        &report,
    )
    .map_err(|e| format!("write smoke report: {e}"))?;
    app.exit(if ok { 0 } else { 1 });
    Ok(())
}

/// Reads the "Window close action" preference. An unset row keeps the
/// renderer's default, which is Minimize.
fn close_hides_to_tray(app: &tauri::AppHandle) -> bool {
    if !TRAY_READY.load(Ordering::Relaxed) {
        return false;
    }
    let state = app.state::<AppState>();
    let conn = state
        .db
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    db::setting(&conn, "ui.close_to_tray")
        .ok()
        .flatten()
        .map(|value| value.trim() != "false")
        .unwrap_or(true)
}

#[cfg(desktop)]
pub fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Notification-area icon: left click restores the window, the menu is the only
/// way to quit a hidden Sucr.
#[cfg(desktop)]
fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "tray-open", "Open Sucr", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit Sucr", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;

    let _tray = TrayIconBuilder::with_id("sucr-tray")
        .icon(tauri::include_image!("icons/tray-32.png"))
        .tooltip("Sucr")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-quit" => app.exit(0),
            "tray-open" => show_main_window(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    TRAY_READY.store(true, Ordering::Relaxed);
    Ok(())
}

#[cfg(not(desktop))]
fn build_tray(_app: &mut tauri::App) -> tauri::Result<()> {
    Ok(())
}

/// Watches for a cable or tether appearing and going away.
///
/// Windows has `NotifyIpInterfaceChange`, but it wants a callback handle and a
/// thread parked on it. Polling the adapter list is a few microseconds of work
/// and five seconds is short enough that plugging in a phone reads as immediate.
/// Every pass that finds a change rewrites the `networks` rows and refreshes the
/// live caps and usage, not just the first: a bonded job reads them, so a dongle
/// that appeared while the Networks view was closed has to be usable by the next
/// Start. A pass that finds nothing does no database work at all.
///
/// The first pass runs before the first sleep, because the rows are what job
/// attribution writes against: a download that starts inside the boot interval
/// would otherwise have no card to charge and its bytes would book nowhere.
#[cfg(desktop)]
fn watch_network_changes(app: &mut tauri::App) {
    use tauri::Emitter;
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let mut seen: Option<String> = None;
        // Starts open: a machine that boots with the window shut has not missed
        // an opening yet, and the first pass that finds it open is the one to act.
        let mut window_was_open = true;
        loop {
            let adapters = engine::network::enumerate().unwrap_or_default();
            // Identity, not the interface index: a replugged adapter keeps its
            // row and its user's switches, and only a device that really joined
            // or left should make the deck say so. Windows' own name and class
            // travel with it, because a cable renamed in the OS has to reach its
            // card before someone opens the deck again. Deliberately absent: the
            // link speed and metric, which a renegotiating radio rewrites several
            // times a minute, and the deck already reads them live.
            let current: String = adapters
                .iter()
                .map(|a| {
                    format!(
                        "{}|{}|{}|{}|{};",
                        engine::network::identity(a),
                        a.local_ip,
                        engine::network::display_name(a),
                        a.kind.as_str(),
                        a.virtual_
                    )
                })
                .collect();
            let state = handle.state::<AppState>();
            if seen.as_deref() != Some(current.as_str()) {
                // Anything after the first pass is a change the user can see, so
                // the deck is told to re-read. The first pass is not announced: the
                // renderer has not asked for rows yet, and would only pay for a
                // fetch it was about to make anyway.
                let announced = seen.replace(current).is_some();
                if let Ok(db) = state.db.lock() {
                    let _ = ipc::network_payload(&db, &adapters, &state);
                }
                if announced {
                    let _ = handle.emit("network:changed", serde_json::json!({ "count": adapters.len() }));
                }
            } else {
                // Nothing moved, so no row to rewrite and nothing to announce —
                // but a quota window can still have closed underneath a running
                // job, and only the ledger knows that.
                refresh_network_quotas(&state);
            }
            // A download window that opens needs the pass that finds it: while it
            // was shut, every Start handed its row back to the queue, and nothing
            // else wakes that queue. Judged once per pass, outside the branch,
            // because an adapter that moved is not a reason to miss an opening.
            if window_is_open(&state) {
                if !window_was_open {
                    ipc::pump_queue(
                        &state.paths.db_path,
                        &state.paths.downloads_dir,
                        &handle,
                        &state.engine,
                        &state.global_limit,
                        &state.network_scopes,
                    );
                }
                window_was_open = true;
            } else {
                window_was_open = false;
            }
            // The pause belongs to the loop, not to the branch that found
            // something: an earlier `continue` on the unchanged path jumped over
            // it, so the one pass that runs every second of every session
            // re-enumerated adapters and rebuilt this string as fast as the
            // processor allowed, while holding nothing back but its own core.
            // The same pass also re-asserts the awake request, so a job that
            // started or ended on a thread this one never sees cannot leave the
            // machine's sleep state out of step with its transfers.
            state.engine.refresh_keep_awake();
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

#[cfg(not(desktop))]
fn watch_network_changes(_app: &mut tauri::App) {}

pub fn run() {
    let paths = paths::resolve();
    let conn = db::open(std::path::Path::new(&paths.db_path)).unwrap_or_else(|e| {
        eprintln!("sucr.db: {e}");
        std::process::exit(1);
    });
    // Before anything that can panic: the log has to exist by the time the first
    // thing that could go wrong does. Absent means on — the switch is in Settings
    // and it ships switched on, so only a stored `false` turns it off.
    let debug_on = db::setting(&conn, "system.debug_log")
        .ok()
        .flatten()
        .map(|v| v.trim() != "false")
        .unwrap_or(true);
    debug_log::switch(&paths.logs_dir, debug_on);
    debug_log::install_hook();
    debug_log::write(
        "INFO",
        &format!(
            "Sucr {} started (pid {}) on {}; debug log {}",
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            paths.data_dir,
            if debug_on { "on" } else { "off" }
        ),
    );
    let smoke = std::env::args().any(|a| a == "--smoke");
    // A capture-driven start: tray and door, no window. The window is created
    // hidden by the config and shown here for every other launch.
    let background = std::env::args().any(|a| a == "--background");
    // A harness run owns its own data directory and must not touch the machine's
    // registry: registration is for the install the user actually runs.
    let harness_profile = std::env::args().any(|a| a.starts_with("--data-dir="))
        || std::env::var("SUCR_DATA_DIR").map(|v| !v.is_empty()).unwrap_or(false);
    if let Ok(Some(secs)) = db::setting(&conn, "network.timeout_seconds") {
        if let Ok(secs) = secs.trim().parse::<u64>() {
            engine::set_connect_timeout(secs);
        }
    }
    // Before the first agent exists, so the opening probe of a session already
    // leaves through the proxy that was chosen. A row that cannot be read is
    // direct: the safe answer, and the one the status line can name.
    let (mode, url, bypass) = db::proxy_settings(&conn).unwrap_or_default();
    engine::proxy::configure(&mode, &url, &bypass);
    // The credential after the URL, and read from its own table: it is keyed to the
    // proxy named by these keys, and whether it is used is decided by that pair
    // matching, not by the order the two were saved in.
    if let Ok(Some(auth)) = db::proxy_auth(&conn) {
        engine::proxy::set_auth(&auth.host, &auth.username, &auth.secret);
    }
    // Every stored origin credential at once, for the same reason: the first agent is
    // built before the first probe, and the middleware reads this mirror, not the
    // database. Unreadable is empty, which is the same safe answer as direct — a link
    // that needs a credential says so instead of silently going out anonymous twice.
    engine::auth::configure(
        db::origin_auths(&conn)
            .unwrap_or_default()
            .into_iter()
            .map(|a| (a.host, a.kind, a.username, a.secret))
            .collect(),
    );
    let global_limit = Arc::new(engine::limit::Bucket::new(
        db::setting(&conn, "global_speed_limit")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<u64>().ok()),
    ));
    let state = AppState {
        paths,
        db: Mutex::new(conn),
        started_at: now_ms(),
        smoke,
        engine: Arc::new(engine::job::Registry::default()),
        global_limit,
        network_scopes: Arc::new(RwLock::new(BTreeMap::new())),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![sucr_invoke, sucr_probe, smoke_report])
        .setup(move |app| {
            #[cfg(desktop)]
            if !background {
                show_main_window(app.handle());
            }
            if app.state::<AppState>().smoke {
                return Ok(());
            }
            // The capture door: one loopback port, a token, and a discovery file
            // the native host reads. Started before the tray so a browser that
            // woke the app can hand off while the window is still painting.
            let data_dir = std::path::PathBuf::from(&app.state::<AppState>().paths.data_dir);
            capture::start(app.handle().clone(), &data_dir);
            if !harness_profile {
                let exe = std::env::current_exe().unwrap_or_default();
                let _ = capture::register_host(&data_dir, &exe);
            }
            // A machine that stopped under a running transfer would otherwise leave
            // that row claiming an activity that has stopped, with a Pause button and
            // nothing behind it. Hand those rows back to the queue before the first
            // window paints. Under `--smoke` this does not run, so the gate never
            // launches a transfer; the pure reclaim behind it is exercised in tests.
            {
                let state = app.state::<AppState>();
                let resumed = ipc::reclaim_interrupted(&state).unwrap_or_default();
                if !resumed.is_empty() {
                    ipc::pump_queue(
                        &state.paths.db_path,
                        &state.paths.downloads_dir,
                        &app.handle(),
                        &state.engine,
                        &state.global_limit,
                        &state.network_scopes,
                    );
                }
            }
            watch_network_changes(app);
            Ok(build_tray(app)?)
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" && close_hides_to_tray(window.app_handle()) {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                // Closing the window with "close to tray" off is quit intent, and the
                // runtime's own last-window exit does not always reach us: a window
                // that was hidden first is destroyed without it, which left Sucr
                // running windowless with only the tray menu as a way out.
                tauri::WindowEvent::Destroyed => {
                    let app = window.app_handle();
                    if app.get_webview_window("main").is_none() && !close_hides_to_tray(app) {
                        app.exit(0);
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Sucr");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state over a scratch database, so the registry can be tested against the
    /// same rows the watcher writes. Test-only: `#[cfg(test)]` never reaches the
    /// release binary, so this costs the exe nothing.
    fn scratch_state(name: &str) -> (std::path::PathBuf, AppState) {
        let dir = std::env::temp_dir().join(format!("sucr-lib-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let db_path = dir.join("sucr.db");
        let state = AppState {
            paths: paths::AppPaths {
                data_dir: dir.display().to_string(),
                db_path: db_path.display().to_string(),
                logs_dir: dir.display().to_string(),
                downloads_dir: dir.display().to_string(),
            },
            db: Mutex::new(db::open(&db_path).expect("scratch db")),
            started_at: 0,
            smoke: false,
            engine: Arc::new(engine::job::Registry::default()),
            global_limit: Arc::new(engine::limit::Bucket::new(None)),
            network_scopes: Arc::new(RwLock::new(BTreeMap::new())),
        };
        (dir, state)
    }

    #[test]
    fn a_cap_saved_after_a_job_resolved_it_reaches_the_same_bucket() {
        let (dir, state) = scratch_state("shared-cap");
        let id = 7;
        assert!(state.network_bucket(id).is_none(), "no cap, no bucket");

        state.set_network_cap(id, Some(2 * 1024 * 1024));
        let first = state
            .network_bucket(id)
            .expect("a saved cap gets a bucket");
        assert_eq!(first.cap(), Some(2 * 1024 * 1024));

        // Raising the ceiling must not swap the object out: a worker thread holds
        // the `Arc` it resolved when its job was built and would otherwise keep
        // pacing at the old number for the rest of the transfer.
        state.set_network_cap(id, Some(4 * 1024 * 1024));
        let again = state.network_bucket(id).expect("the bucket is still there");
        assert!(
            Arc::ptr_eq(&first, &again),
            "the same adapter keeps one bucket"
        );
        assert_eq!(first.cap(), Some(4 * 1024 * 1024), "the holder sees it too");

        state.set_network_cap(id, Some(0));
        assert!(
            state.network_bucket(id).is_none(),
            "a zero cap is no cap, and leaves nothing in the map"
        );

        // A row can only be written through `network:set-limits`, which refuses a
        // number that is not positive, so a negative here comes from an edited or
        // ageing database. Cast straight to the bucket's `u64` it would wrap into a
        // colossal ceiling, which is the opposite of a limit.
        state.set_network_cap(id, Some(3 * 1024 * 1024));
        state.set_network_cap(id, Some(-1));
        assert!(
            state.network_bucket(id).is_none(),
            "a negative cap is no cap either"
        );
        drop(state);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
