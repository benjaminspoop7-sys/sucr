// End-to-end engine tests over real sockets. Each test starts the Node chaos
// server, inserts a downloads row, runs the job, and inspects both the file on
// disk and the database it left behind.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::AtomicU8;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::params;

use super::limit::Bucket;
use super::{check_refresh, refresh_url, resumable, run, Probe, Registry, RunContext, BlockSpec,
            next_width, step_bill, RampAction, RampSample, RampState, RampTuning, RUN};

const SEED: u64 = 1;

fn app_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

struct Chaos {
    child: Child,
    stdin: Option<ChildStdin>,
    url: String,
    /// `"sucr-<size>-1"`: derived from the bytes, so two servers with the same
    /// size agree on it and a refresh can be told a genuine re-link from a
    /// different file.
    etag: String,
    /// The fixed stamp every response carries: `Wed, 22 Nov 2023 00:53:20 GMT`.
    last_modified: String,
}

impl Chaos {
    fn start(size: usize) -> Chaos {
        let mut child = Command::new("node")
            .arg("test/chaos-run.mjs")
            .current_dir(app_dir())
            .env("SUCR_CHAOS_SIZE", size.to_string())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .expect("spawn node chaos server (is node on PATH?)");
        let stdout = child.stdout.take().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(stdout).read_line(&mut line).unwrap();
        let info: serde_json::Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("chaos server said {line:?}: {e}"));
        Chaos {
            stdin: child.stdin.take(),
            url: info["url"].as_str().unwrap().to_string(),
            etag: info["etag"].as_str().unwrap().to_string(),
            last_modified: info["lastModified"].as_str().unwrap().to_string(),
            child,
        }
    }

    fn url(&self, query: &str) -> String {
        format!("{}{}", self.url, query)
    }
}

impl Drop for Chaos {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

fn body(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| ((i as u64 * 31 + SEED * 17) & 0xff) as u8)
        .collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sucr-engine-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("dl")).unwrap();
    dir
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

struct Fixture {
    dir: PathBuf,
    conn: rusqlite::Connection,
    id: i64,
}

impl Fixture {
    fn new(name: &str, url: &str, block: Option<u64>, streams: Option<u64>) -> Fixture {
        let dir = temp_dir(name);
        let conn = crate::db::open(&dir.join("sucr.db")).unwrap();
        conn.execute(
            "INSERT INTO downloads (url, filename, folder, status, created_at, block_size, streams_fixed) \
             VALUES (?1, 'file.bin', ?2, 'queued', ?3, ?4, ?5)",
            params![
                url,
                dir.join("dl").to_string_lossy(),
                now_ms(),
                block.map(|b| b as i64),
                streams.map(|s| s as i64)
            ],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        Fixture { dir, conn, id }
    }

    fn ctx(&self) -> RunContext {
        RunContext {
            db_path: self.dir.join("sucr.db"),
            downloads_dir: self.dir.join("dl"),
            app: None,
            live: None,
            limit: Arc::new(Bucket::new(None)),
            global: Arc::new(Bucket::new(None)),
            networks: Arc::default(),
            // Every harness job runs on the plain route: the chaos server is on
            // loopback, and a bound socket has to match the family it dials.
            routes: vec![super::Route::default_route()],
            policy: super::BondPolicy::Dynamic,
            ramp: super::RampTuning::default(),
        }
    }

    fn run(&self) -> Result<(), super::JobError> {
        run(self.ctx(), self.id, Arc::new(AtomicU8::new(RUN)))
    }

    fn status(&self) -> String {
        self.conn
            .query_row("SELECT status FROM downloads WHERE id = ?1", [self.id], |r| r.get(0))
            .unwrap()
    }

    fn column(&self, name: &str) -> Option<String> {
        self.conn
            .query_row(
                &format!("SELECT {name} FROM downloads WHERE id = ?1"),
                [self.id],
                |r| r.get(0),
            )
            .ok()
            .flatten()
    }

    fn num_column(&self, name: &str) -> Option<i64> {
        self.conn
            .query_row(
                &format!("SELECT {name} FROM downloads WHERE id = ?1"),
                [self.id],
                |r| r.get(0),
            )
            .ok()
            .flatten()
    }

    fn history(&self) -> Vec<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind FROM history_events WHERE download_id = ?1 ORDER BY id")
            .unwrap();
        stmt.query_map([self.id], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// The payload of every event of one kind, oldest first — how a test reads
    /// what the engine decided, not only that it decided something.
    fn history_json(&self, kind: &str) -> Vec<serde_json::Value> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT payload_json FROM history_events \
                 WHERE download_id = ?1 AND kind = ?2 ORDER BY id",
            )
            .unwrap();
        stmt.query_map(params![self.id, kind], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|row| {
                let text = row.unwrap();
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{kind} payload {text:?}: {e}"))
            })
            .collect()
    }

    /// What refused attempts say, oldest first: the chip the pane shows in a
    /// width-limited column, and the full sentence the hover text carries.
    fn attempt_reasons(&self) -> Vec<(String, String)> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT reason, COALESCE(detail, '') FROM block_attempts \
                 WHERE download_id = ?1 AND reason IS NOT NULL ORDER BY id",
            )
            .unwrap();
        stmt.query_map([self.id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn published(&self) -> Vec<u8> {
        std::fs::read(self.dir.join("dl").join("file.bin")).unwrap()
    }

    fn staging_exists(&self) -> bool {
        std::fs::read_dir(self.dir.join("dl"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().ends_with(".sucrpart"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn clean_download_lands_every_block() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("clean", &chaos.url(""), Some(1024 * 1024), Some(4));

    fx.run().expect("clean download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert!(!fx.staging_exists(), "staging file should be renamed away");
    let blocks: Vec<(i64, String, i64)> = {
        let mut stmt = fx
            .conn
            .prepare("SELECT idx, state, bytes_done FROM blocks WHERE download_id = ?1 ORDER BY idx")
            .unwrap();
        stmt.query_map([fx.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(blocks.len(), 4);
    assert!(blocks.iter().all(|(_, state, bytes)| state == "done" && *bytes == 1024 * 1024));
    let events = fx.history();
    assert!(events.contains(&"coverage-ok".to_string()), "{events:?}");
    assert!(events.contains(&"published".to_string()), "{events:?}");
    assert_eq!(fx.num_column("total_bytes"), Some(size as i64));
}

#[test]
fn dropped_sockets_resume_inside_the_block() {
    // 16 KiB of every 64 KiB slice, then the connection dies. Each retry keeps
    // the bytes already written instead of restarting the block.
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    let url = chaos.url(&format!("?drop={}", 64 * 1024));
    let fx = Fixture::new("drop", &url, Some(128 * 1024), Some(1));

    fx.run().expect("download survives drops");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert!(fx.history().iter().any(|k| k == "block-retry"));
}

#[test]
fn cut_streambody_is_retried() {
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    let url = chaos.url(&format!("?cut={}", 64 * 1024));
    let fx = Fixture::new("cut", &url, Some(128 * 1024), Some(1));

    fx.run().expect("download survives cuts");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
}

#[test]
fn server_that_lies_about_ranges_falls_back_to_one_stream() {
    let size = 2 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let url = chaos.url("?probe206=1");
    let fx = Fixture::new("fallback", &url, Some(256 * 1024), Some(4));

    fx.run().expect("fallback download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    let events = fx.history();
    assert!(events.contains(&"fallback-single".to_string()), "{events:?}");
    assert_eq!(fx.column("streams_mode").as_deref(), Some("single"));
    let blocks: i64 = fx
        .conn
        .query_row("SELECT count(*) FROM blocks WHERE download_id = ?1", [fx.id], |r| r.get(0))
        .unwrap();
    assert_eq!(blocks, 0, "the abandoned block plan must be cleared");
}

#[test]
fn no_range_support_streams_whole() {
    let size = 1024 * 1024;
    let chaos = Chaos::start(size);
    let url = chaos.url("?norange=1");
    let fx = Fixture::new("norange", &url, Some(256 * 1024), Some(8));

    fx.run().expect("single stream download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert_eq!(fx.column("streams_mode").as_deref(), Some("single"));
    let attempts: Vec<(String, Option<i64>, i64)> = {
        let mut stmt = fx
            .conn
            .prepare("SELECT state, ended_at, bytes FROM block_attempts WHERE download_id = ?1")
            .unwrap();
        stmt.query_map([fx.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(attempts.len(), 1, "the single stream writes one attempt row");
    assert_eq!(attempts[0].0, "done");
    assert!(attempts[0].1.is_some(), "the row is closed when the stream ends");
    assert_eq!(attempts[0].2, size as i64);
}

#[test]
fn matching_checksum_marks_verified() {
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("verified", &chaos.url(""), Some(256 * 1024), Some(2));
    let expected = sha256_hex(&body(size));
    fx.conn
        .execute(
            "UPDATE downloads SET checksum_algo = 'sha256', checksum_hex = ?1 WHERE id = ?2",
            params![expected, fx.id],
        )
        .unwrap();

    fx.run().expect("verified download");

    assert_eq!(fx.status(), "verified");
    assert_eq!(fx.published(), body(size));
    assert!(fx.history().iter().any(|k| k == "checksum-ok"));
}

#[test]
fn mismatched_checksum_holds_the_file_back() {
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("mismatch", &chaos.url(""), None, None);
    fx.conn
        .execute(
            "UPDATE downloads SET checksum_algo = 'sha256', checksum_hex = ?1 WHERE id = ?2",
            params!["0".repeat(64), fx.id],
        )
        .unwrap();

    let err = fx.run().unwrap_err();

    assert_eq!(err.code, "checksum-mismatch");
    assert_eq!(fx.status(), "needs-attention");
    assert_eq!(fx.column("error_code").as_deref(), Some("checksum-mismatch"));
    assert!(!fx.dir.join("dl").join("file.bin").exists(), "nothing may be published");
    assert!(fx.staging_exists(), "the bytes are kept for inspection");
}

#[test]
fn pause_before_the_probe_then_resume_finishes() {
    let size = 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("pause", &chaos.url(""), Some(256 * 1024), Some(2));

    let paused = Arc::new(AtomicU8::new(super::PAUSE));
    let err = run(fx.ctx(), fx.id, Arc::clone(&paused)).unwrap_err();
    assert_eq!(err.code, "cancelled");
    assert_eq!(fx.status(), "paused");
    assert_eq!(
        fx.num_column("streams_final"),
        None,
        "a run that never transferred has no width to report"
    );

    fx.run().expect("resume after pause");
    assert_eq!(fx.status(), "finished");
    let width = fx.history_json("ramp-final")[0]["width"].as_i64().unwrap();
    assert_eq!(
        fx.num_column("streams_final"),
        Some(width),
        "the run that did transfer leaves its width on the row"
    );
    assert!(width <= 2, "a pinned 2 on a one-block file may not claim more: {width}");
    assert_eq!(fx.published(), body(size));
}

#[test]
fn a_stopped_job_leaves_its_row_saying_paused() {
    // `download_remove` now refuses while this job is still registered, so the
    // row survives and its status reaches a screen. A stop used to write nothing
    // — safe only because the row was always deleted behind it — which left a
    // job with no thread claiming it was still Downloading.
    let size = 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("stop", &chaos.url(""), Some(256 * 1024), Some(2));

    let err = run(fx.ctx(), fx.id, Arc::new(AtomicU8::new(super::STOP))).unwrap_err();

    assert_eq!(err.code, "cancelled");
    assert_eq!(fx.status(), "paused", "a stopped row may not keep saying downloading");
    assert!(
        fx.history().iter().any(|k| k == "paused"),
        "the log line is what tells a stopped run from a failed one: {:?}",
        fx.history()
    );
    assert_eq!(
        fx.column("error_code"),
        None,
        "nothing went wrong, so the row must not wear an error"
    );
}

#[test]
fn a_row_is_given_up_only_once_its_job_leaves_the_registry() {
    // `download_remove` waits on this answer before it deletes anything, because
    // SQLite hands the freed rowid to the next insert: a row whose job is still
    // registered has to keep its bytes rather than be cleared on a timeout.
    let reg = Registry::default();
    let (_cancel, _live, _bucket) = reg.register(7);
    assert!(reg.is_running(7));
    assert!(
        !reg.wait_idle(7, Duration::from_millis(40)),
        "a registered job must not read as idle"
    );

    // Pressing Stop latches the flag; it does not retire the job. Only the
    // worker's own unregister ends the wait, so a job blocked in a read keeps
    // the row alive for as long as it holds the staging file open.
    assert!(reg.signal(7, super::STOP));
    assert!(!reg.wait_idle(7, Duration::from_millis(20)), "a latched flag is not an exit");

    reg.unregister(7);
    assert!(reg.wait_idle(7, Duration::from_millis(20)));
    assert_eq!(reg.running_count(), 0);
}

#[test]
fn corrupt_byte_without_checksum_still_publishes() {
    // Without a checksum the engine cannot see the flipped byte; this documents
    // that limitation rather than pretending the file is verified.
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    let url = chaos.url("?corrupt=100000");
    let fx = Fixture::new("corrupt", &url, None, None);

    fx.run().expect("download completes");

    assert_eq!(fx.status(), "finished");
    let published = fx.published();
    assert_eq!(published.len(), size);
    assert_ne!(published, body(size), "the corrupted byte is expected to differ");
}

#[test]
fn map_and_attempts_track_the_clean_run() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("live", &chaos.url(""), Some(1024 * 1024), Some(4));
    let live = Arc::new(super::live::Live::new());
    live.watch();
    let ctx = RunContext {
        live: Some(Arc::clone(&live)),
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("clean download");

    let snapshot = live.snapshot().expect("snapshot");
    assert_eq!(snapshot.blocks.len(), 4);
    assert!(snapshot.blocks.iter().all(|b| b.state == "done" && b.bytes_done == b.len));
    assert_eq!(snapshot.attempts.len(), 4);
    assert!(snapshot.attempts.iter().all(|a| a.state == "done" && a.bytes == 1024 * 1024));

    let rows: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM block_attempts WHERE download_id = ?1",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 4, "every attempt is persisted");
    let finished: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM block_attempts WHERE download_id = ?1 AND ended_at IS NOT NULL",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(finished, 4, "every attempt gets an end time");
    let still_active: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM block_attempts WHERE download_id = ?1 AND state = 'active'",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(still_active, 0, "no attempt may look live after the run");

    let delta = live.take_delta(true).expect("final delta");
    assert_eq!(delta.total, size as u64);
    assert_eq!(delta.block_size, 1024 * 1024);
}

#[test]
fn two_bonded_routes_each_carry_part_of_the_file() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("bond", &chaos.url(""), Some(1024 * 1024), Some(4));

    // Two loopback sources stand in for two devices. This box has one cable, and
    // what the test is about is whether a chunk really leaves from the address
    // it was told to, not which silicon is on the other end.
    let wired =
        crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:01", "Ethernet", "ethernet", true)
            .unwrap();
    let wifi =
        crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:02", "Wi-Fi", "wifi", true).unwrap();
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };

    let live = Arc::new(super::live::Live::new());
    live.watch();
    let ctx = RunContext {
        live: Some(Arc::clone(&live)),
        routes: vec![
            route(wired.id, "n1", "Ethernet", "127.0.0.1"),
            route(wifi.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("bonded download");

    let devices: Vec<i64> = {
        let mut stmt = fx
            .conn
            .prepare(
                "SELECT DISTINCT network_id FROM block_attempts \
                 WHERE download_id = ?1 ORDER BY network_id",
            )
            .unwrap();
        stmt.query_map([fx.id], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(
        devices,
        vec![wired.id, wifi.id],
        "both enabled devices carry blocks, so the toggle means something"
    );

    let carried: Vec<(i64, i64)> = {
        let mut stmt = fx
            .conn
            .prepare(
                "SELECT network_id, sum(bytes) FROM block_attempts \
                 WHERE download_id = ?1 GROUP BY network_id ORDER BY network_id",
            )
            .unwrap();
        stmt.query_map([fx.id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(carried.len(), 2);
    assert!(
        carried.iter().all(|(_, bytes)| *bytes > 0),
        "a device in the bond is never shown as carrying nothing: {carried:?}"
    );
    assert_eq!(
        carried.iter().map(|(_, bytes)| *bytes).sum::<i64>(),
        size as i64,
        "every byte is booked to exactly one device"
    );

    let snapshot = live.snapshot().expect("snapshot");
    assert!(
        snapshot.blocks.iter().all(|b| b.network_id == "n1" || b.network_id == "n2"),
        "the map's stripe is the device that pulled the block"
    );

    let charged: i64 = fx
        .conn
        .query_row("SELECT count(*) FROM net_usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(charged, 2, "each device's day row is written once");
}

/// An enabled device that cannot reach the server must cost bandwidth, not the
/// download. TEST-NET-1 is not assigned to this machine, so binding to it fails
/// the same way a real adapter fails when the host lives behind another route.
#[test]
fn a_bond_that_cannot_reach_the_host_finishes_on_the_default_route() {
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("escape", &chaos.url(""), Some(256 * 1024), Some(2));

    let first = crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:FE", "Dead LAN", "ethernet", true).unwrap();
    let second = crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:FF", "Dead Wi-Fi", "wifi", true).unwrap();
    let dead = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };

    let live = Arc::new(super::live::Live::new());
    live.watch();
    let ctx = RunContext {
        live: Some(Arc::clone(&live)),
        routes: vec![
            dead(first.id, "n1", "Dead LAN", "192.0.2.1"),
            dead(second.id, "n2", "Dead Wi-Fi", "192.0.2.2"),
        ],
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN)))
        .expect("the default route still delivers");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert!(
        fx.history().iter().any(|k| k == "bond-fallback"),
        "the log has to say bonding was given up: {:?}",
        fx.history()
    );

    let snapshot = live.snapshot().expect("snapshot");
    assert!(
        snapshot
            .blocks
            .iter()
            .all(|b| b.network_id == super::live::UNATTRIBUTED),
        "no device is credited with bytes its bond did not carry"
    );

    let charged: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM net_usage WHERE network_id IN (?1, ?2)",
            params![first.id, second.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(charged, 0, "a device that carried nothing books no usage");

    // ... and the bytes are not lost either. The pane's totals were adapter sums,
    // so an escape like this — like every proxied link — moved a file and reported
    // nothing. The default route's blocks go to a ledger of their own, which is
    // what lets the deck add the two and name the part no adapter carried.
    let (un_today, _, _, un_lifetime) = crate::db::unattributed_usage(&fx.conn).unwrap();
    assert_eq!(
        un_lifetime,
        size as i64,
        "every byte still has a record, just not an interface: {un_lifetime} of {size}"
    );
    assert_eq!(un_today, size as i64, "booked into the same day window");
    let anywhere: i64 = fx
        .conn
        .query_row("SELECT count(*) FROM net_usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(anywhere, 0, "no adapter is credited with the escape's bytes");
}

#[test]
fn attempts_left_active_by_a_killed_process_are_closed() {
    let size = 256 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("dangling", &chaos.url(""), Some(64 * 1024), Some(1));
    fx.conn
        .execute(
            "INSERT INTO block_attempts (download_id, block_idx, network_id, started_at, bytes, state, reason) \
             VALUES (?1, 0, NULL, ?2, 0, 'active', NULL)",
            params![fx.id, now_ms() - 60_000],
        )
        .unwrap();
    let attempt_id = fx.conn.last_insert_rowid();

    fx.run().expect("clean download");

    let (state, ended_at): (String, Option<i64>) = fx
        .conn
        .query_row(
            "SELECT state, ended_at FROM block_attempts WHERE id = ?1",
            [attempt_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "interrupted");
    assert!(ended_at.is_some(), "the orphan must be given an end time");
}

#[test]
fn speed_limit_caps_a_parallel_transfer() {
    // 2 MiB at 1 MiB/s: one burst slice, then paced. The server is unthrottled,
    // so the elapsed time is the engine's cap alone.
    let size = 2 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("capped", &chaos.url(""), Some(512 * 1024), Some(4));
    fx.conn
        .execute(
            "UPDATE downloads SET speed_limit = ?1 WHERE id = ?2",
            params![1024 * 1024i64, fx.id],
        )
        .unwrap();

    let t = Instant::now();
    fx.run().expect("capped download");
    let elapsed = t.elapsed();

    assert_eq!(fx.published(), body(size));
    assert!(elapsed >= Duration::from_millis(1_200), "not capped: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(8), "cap stalled the job: {elapsed:?}");
}

#[test]
fn speed_limit_caps_the_single_stream() {
    let size = 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("capped-single", &chaos.url("?norange=1"), None, None);
    fx.conn
        .execute(
            "UPDATE downloads SET speed_limit = ?1 WHERE id = ?2",
            params![512 * 1024i64, fx.id],
        )
        .unwrap();

    let t = Instant::now();
    fx.run().expect("capped single stream");
    let elapsed = t.elapsed();

    assert_eq!(fx.published(), body(size));
    assert!(elapsed >= Duration::from_millis(1_000), "not capped: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(8), "cap stalled the stream: {elapsed:?}");
}

/// The registry step 3 reads per block attempt. An adapter with no saved cap has
/// no entry, which is the whole shape of it: a scope in the charge slice is a
/// mutex taken on every read, and an unlimited adapter has nothing to say.
fn registry(entries: &[(i64, Option<u64>)]) -> super::NetworkScopes {
    let mut map = std::collections::BTreeMap::new();
    for (id, cap) in entries {
        if let Some(cap) = cap {
            map.insert(*id, Arc::new(super::NetScope::new(Some(*cap))));
        }
    }
    Arc::new(std::sync::RwLock::new(map))
}

/// Mean milliseconds a finished attempt on one adapter took. Only `done` rows,
/// because a cancelled read is short by construction and would water down the
/// average of a device whose blocks got cut short.
fn mean_attempt_ms(conn: &rusqlite::Connection, network_id: i64) -> i64 {
    conn.query_row(
        "SELECT avg(ended_at - started_at) FROM block_attempts \
         WHERE network_id = ?1 AND state = 'done'",
        [network_id],
        |r| r.get::<_, Option<f64>>(0),
    )
    .unwrap()
    .unwrap_or_else(|| panic!("no finished attempt ever left adapter {network_id}")) as i64
}

/// Same fixture and margins as `speed_limit_caps_a_parallel_transfer`, with the
/// cap moved from the job row to the adapter's bucket: the download on that
/// adapter slows to it.
#[test]
fn a_network_cap_paces_the_download_that_leaves_on_that_adapter() {
    let size = 2 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("net-cap", &chaos.url(""), Some(512 * 1024), Some(4));
    let wired =
        crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:11", "Ethernet", "ethernet", true)
            .unwrap();
    let ctx = RunContext {
        routes: vec![super::Route {
            network_id: Some(wired.id),
            color: "n1".to_string(),
            label: "Ethernet".to_string(),
            bind: Some("127.0.0.1".parse().unwrap()),
        }],
        networks: registry(&[(wired.id, Some(1024 * 1024))]),
        ..fx.ctx()
    };

    let t = Instant::now();
    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("paced download");
    let elapsed = t.elapsed();

    assert_eq!(fx.published(), body(size));
    assert!(
        elapsed >= Duration::from_millis(1_200),
        "the adapter's cap was not applied: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(8), "cap stalled the job: {elapsed:?}");
}

/// What the step 1 spike could not show, because it never had two routes: the
/// cap belongs to one adapter, and the adapter beside it in the same bond keeps
/// its own speed. A 512 KiB block cannot cross a 1 MiB/s ceiling in under 500 ms
/// of debt, minus the one burst slice the bucket starts full with.
#[test]
fn a_network_cap_slows_its_own_adapter_and_not_the_other() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("net-select", &chaos.url(""), Some(512 * 1024), Some(4));
    let wired =
        crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:12", "Ethernet", "ethernet", true)
            .unwrap();
    let wifi =
        crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:13", "Wi-Fi", "wifi", true).unwrap();
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };

    let ctx = RunContext {
        routes: vec![
            route(wired.id, "n1", "Ethernet", "127.0.0.1"),
            route(wifi.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        networks: registry(&[(wired.id, Some(1024 * 1024))]),
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("paced bond");
    assert_eq!(fx.published(), body(size));

    let paced = mean_attempt_ms(&fx.conn, wired.id);
    let free = mean_attempt_ms(&fx.conn, wifi.id);
    // Measured on this box, release codegen, two passes: 927/925 ms on the
    // capped adapter against 22 ms on the one beside it.
    assert!(paced >= 200, "the capped adapter was not paced: {paced} ms");
    assert!(
        paced >= free * 3,
        "both adapters slowed, so the cap is not per-adapter: {paced} ms vs {free} ms"
    );
}

/// The non-range path is one stream on one route, and the cap has to reach it
/// there too: a feature that only holds on the parallel path is decorative for
/// every server that refuses a range request.
#[test]
fn a_network_cap_paces_the_single_stream_too() {
    let size = 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("net-cap-single", &chaos.url("?norange=1"), None, None);
    let wired =
        crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:14", "Ethernet", "ethernet", true)
            .unwrap();
    let ctx = RunContext {
        routes: vec![super::Route {
            network_id: Some(wired.id),
            color: "n1".to_string(),
            label: "Ethernet".to_string(),
            bind: Some("127.0.0.1".parse().unwrap()),
        }],
        networks: registry(&[(wired.id, Some(512 * 1024))]),
        ..fx.ctx()
    };

    let t = Instant::now();
    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("paced single stream");
    let elapsed = t.elapsed();

    assert_eq!(fx.published(), body(size));
    assert!(
        elapsed >= Duration::from_millis(1_000),
        "the adapter's cap was not applied to one stream: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(8),
        "cap stalled the stream: {elapsed:?}"
    );
}

/// Why the registry is read per attempt instead of frozen when the job is
/// planned: an adapter getting its *first* cap adds an entry the frozen slice
/// would never look at, and the card promises that saving a cap slows the
/// transfer that is already running.
#[test]
fn a_cap_saved_after_the_job_began_reaches_the_next_attempt() {
    let networks: super::NetworkScopes = Arc::default();
    assert!(
        super::live_cap(&networks, Some(1)).is_none(),
        "no saved cap is no scope"
    );
    assert!(super::live_cap(&networks, None).is_none(), "no adapter is no scope");

    let scope = Arc::new(super::NetScope::new(Some(64 * 1024)));
    networks.write().unwrap().insert(1, Arc::clone(&scope));
    let resolved = super::live_cap(&networks, Some(1)).expect("a cap saved mid-run is seen");
    assert_eq!(resolved.cap(), Some(64 * 1024));
    assert!(
        super::live_cap(&networks, Some(2)).is_none(),
        "a cap belongs to the one adapter id it was saved on"
    );

    scope.set_cap(Some(128 * 1024));
    assert_eq!(
        super::live_cap(&networks, Some(1)).expect("the same entry").cap(),
        Some(128 * 1024),
        "raising a cap that already exists needs no new entry"
    );
}

// --- the per-adapter allowance (step 3b) ------------------------------------

/// An adapter already past its allowance, with no rate cap beside it.
fn spent_scope(allowance: u64) -> Arc<super::NetScope> {
    let scope = Arc::new(super::NetScope::with_allowance(
        None,
        allowance,
        super::NetScope::MONTH,
    ));
    scope.note_carried(allowance);
    scope
}

/// The live registry the engine reads at every block claim, one adapter at a
/// time. An adapter missing from it is an adapter with neither figure saved.
fn scope_map(entries: &[(i64, Arc<super::NetScope>)]) -> super::NetworkScopes {
    let mut map = std::collections::BTreeMap::new();
    for (id, scope) in entries {
        map.insert(*id, Arc::clone(scope));
    }
    Arc::new(std::sync::RwLock::new(map))
}

#[test]
fn a_job_with_no_allowances_gets_the_route_its_policy_asked_for() {
    // Every gate below has to be silent until somebody saves a quota: the claim
    // site now runs through `open_route`, and a bond that answered anything
    // other than the policy's own index would move every download on the
    // machine to prove a feature nobody had switched on.
    let nets = vec![Some(1), Some(2), Some(3)];
    let networks = scope_map(&[]);
    for policy in [
        super::BondPolicy::Dynamic,
        super::BondPolicy::Hedging,
        super::BondPolicy::Failover,
    ] {
        for worker in 0..4 {
            for attempt in 0..3u32 {
                let wanted = policy.route_for(worker, attempt, 3);
                assert_eq!(
                    super::open_route(wanted, 3, false, &nets, &networks),
                    Some(wanted),
                    "{policy:?} worker {worker} attempt {attempt}"
                );
                if policy.hedges() {
                    let second = policy.secondary_route_for(worker, attempt, 3);
                    assert_eq!(
                        super::open_route(second, 3, true, &nets, &networks),
                        Some(second),
                        "{policy:?} worker {worker} attempt {attempt} (hedge)"
                    );
                }
            }
        }
    }
}

#[test]
fn an_adapter_out_of_bytes_yields_its_next_block_to_the_route_beside_it() {
    let nets = vec![Some(1), Some(2), Some(3)];
    let networks = scope_map(&[
        (1, spent_scope(1024 * 1024)),
        (3, spent_scope(2 * 1024 * 1024)),
    ]);

    assert_eq!(
        super::open_route(0, 3, false, &nets, &networks),
        Some(1),
        "a spent route hands the block to the next one the policy may use"
    );
    assert_eq!(
        super::open_route(2, 3, false, &nets, &networks),
        Some(1),
        "the scan wraps, so a spent tail still reaches the open head"
    );
    assert_eq!(
        super::open_route(1, 3, false, &nets, &networks),
        Some(1),
        "an open route keeps the block it was already given"
    );
}

#[test]
fn a_hedge_never_races_onto_the_primary_that_went_quiet() {
    // The band narrows to the secondary routes: a half that fell back onto the
    // primary would re-read the very stream the hedge exists to work around,
    // however many bytes that route has left.
    let nets = vec![Some(1), Some(2), Some(3)];
    let networks = scope_map(&[(2, spent_scope(1024)), (3, spent_scope(1024))]);
    assert_eq!(
        super::open_route(1, 3, true, &nets, &networks),
        None,
        "route 0 being free is no reason to hedge onto it"
    );

    let networks = scope_map(&[(1, spent_scope(1024)), (2, spent_scope(1024))]);
    assert_eq!(
        super::open_route(1, 3, true, &nets, &networks),
        Some(2),
        "the wrap stays inside the band"
    );

    // With one route there is no primary to avoid, so the band is the whole band.
    let networks = scope_map(&[]);
    assert_eq!(
        super::open_route(0, 1, true, &[Some(1)], &networks),
        Some(0),
        "a lone route hedges on itself rather than answering nothing"
    );
}

#[test]
fn bytes_no_adapter_can_be_credited_for_charge_no_allowance() {
    // Ruling 2 as chosen on 2026-11-04: an allowance gates only attributed
    // routes. Bytes that cannot be pinned to a device are charged to no
    // allowance, so a route carrying no id is never spent — and the unbound
    // route is answered before the lock, not by a lookup that misses. This is
    // the exception, not the one-adapter case: `attribute_egress` gives this
    // machine's unbonded default route its adapter id, and step 6 measured the
    // allowance freezing and releasing a real transfer on it.
    let networks = scope_map(&[(1, spent_scope(1024 * 1024))]);
    assert!(
        !super::route_spent(&networks, None),
        "an unattributed route is never spent"
    );
    assert_eq!(
        super::open_route(0, 1, false, &[None], &networks),
        Some(0),
        "the one unbound route on the machine keeps its blocks"
    );

    let networks = scope_map(&[(1, spent_scope(1024)), (2, spent_scope(1024))]);
    assert_eq!(
        super::open_route(1, 3, false, &[None, Some(1), Some(2)], &networks),
        Some(0),
        "the unbound route inherits the bytes the spent adapters refused"
    );
}

#[test]
fn every_route_being_out_of_bytes_waits_and_a_raised_allowance_re_admits_it() {
    // Rulings 3 and 4 as chosen on 2026-11-04: nothing open is a wait on the
    // existing tick, not a failure or a new job state, and eligibility is
    // derived from the two numbers each time it is asked, so nothing saved on a
    // route can leave it stuck out after the ceiling moves.
    let first = spent_scope(1024 * 1024);
    let second = spent_scope(2 * 1024 * 1024);
    let nets = vec![Some(1), Some(2)];
    let networks = scope_map(&[(1, Arc::clone(&first)), (2, Arc::clone(&second))]);

    assert_eq!(
        super::open_route(0, 2, false, &nets, &networks),
        None,
        "no route is open, so the claim waits"
    );

    first.set_allowance(first.used() + 1024 * 1024, super::NetScope::MONTH);
    assert_eq!(
        super::open_route(0, 2, false, &nets, &networks),
        Some(0),
        "raising the ceiling is the whole of re-admission"
    );
    assert_eq!(
        super::open_route(1, 2, false, &nets, &networks),
        Some(0),
        "the adapter still out of bytes stays out"
    );

    // A window that rolls is a ledger read of zero, not something the user does.
    second.note_ledger(0, 0, 0);
    assert_eq!(
        super::open_route(1, 2, false, &nets, &networks),
        Some(1),
        "the new window re-admits the other adapter on its own"
    );
}

#[test]
fn an_allowance_is_counted_against_the_window_the_row_names() {
    assert_eq!(super::NetScope::window_for(Some("day")), super::NetScope::DAY);
    assert_eq!(super::NetScope::window_for(Some("week")), super::NetScope::WEEK);
    assert_eq!(
        super::NetScope::window_for(Some("month")),
        super::NetScope::MONTH
    );
    assert_eq!(
        super::NetScope::window_for(None),
        super::NetScope::MONTH,
        "a quota with no period named is a month, which is what one has always meant here"
    );
    assert_eq!(
        super::NetScope::window_for(Some("fortnight")),
        super::NetScope::MONTH,
        "an unrecognised word gets the default window rather than one nothing counts"
    );

    let scope = super::NetScope::with_allowance(None, 200, super::NetScope::WEEK);
    scope.note_ledger(1_000, 100, 9_000);
    assert_eq!(
        scope.used(),
        100,
        "the week figure is the one a week allowance reads"
    );
    assert!(!scope.spent());

    scope.set_allowance(200, super::NetScope::MONTH);
    scope.note_ledger(1_000, 100, 9_000);
    assert_eq!(
        scope.used(),
        9_000,
        "the same read follows the window the user changed"
    );
    assert!(scope.spent());

    scope.note_ledger(-5, -5, -5);
    assert_eq!(
        scope.used(),
        0,
        "a negative row cannot wrap into a colossal figure"
    );
}

#[test]
fn the_ledger_wins_where_the_engine_stopped() {
    // The two writers and the rule between them: a worker adds what it just
    // carried, and the pass that asked SQLite replaces the total with the
    // ledger's own sum. Between the two a byte can be counted once twice, which
    // ejects a route a little early — the safe direction on a metered link.
    let scope = super::NetScope::with_allowance(None, 10 * 1024 * 1024, super::NetScope::MONTH);
    assert_eq!(scope.used(), 0, "no allowance is met before a byte is booked");

    scope.note_carried(3 * 1024 * 1024);
    assert_eq!(scope.used(), 3 * 1024 * 1024);

    scope.note_ledger(1024, 1024, 7 * 1024 * 1024);
    assert_eq!(
        scope.used(),
        7 * 1024 * 1024,
        "the ledger's sum replaces the running figure instead of adding to it"
    );

    scope.note_carried(1024);
    assert_eq!(
        scope.used(),
        7 * 1024 * 1024 + 1024,
        "a booking after the read still counts, so a pass landing mid-attempt costs no bytes"
    );

    // An adapter with a cap alone counts nothing: the entry exists to pace, and
    // a figure nobody compares against is one fetch-add per block for nothing.
    let capped = super::NetScope::new(Some(1024 * 1024));
    capped.note_carried(8 * 1024 * 1024);
    assert_eq!(capped.used(), 0, "no allowance is no counting");
    assert!(!capped.spent());
    assert!(capped.load_bearing(), "a cap keeps the entry on its own");

    let empty = super::NetScope::new(None);
    assert!(!empty.load_bearing(), "neither figure means the entry should go");
}

/// The one thing the ramp is allowed to know about the registry: which devices this
/// job can leave bytes on, and whether each is under a cap or out of allowance. It
/// decides both what a window's bytes are worth and whether the window was measured
/// against one bond or two.
#[test]
fn the_bond_signature_lists_only_the_adapters_this_job_can_leave_on() {
    let capped = Arc::new(super::NetScope::new(Some(1024 * 1024)));
    let spent = Arc::new(spent_scope(4 * 1024));
    // In the registry with a cap of its own, but no route of this job uses it.
    let other = Arc::new(super::NetScope::new(Some(64 * 1024)));
    let networks = scope_map(&[
        (7, Arc::clone(&capped)),
        (3, Arc::clone(&spent)),
        (9, Arc::clone(&other)),
    ]);

    // Two routes share adapter 7, one route is unbound, one reaches adapter 3.
    let nets: [Option<i64>; 4] = [Some(7), Some(7), None, Some(3)];
    assert_eq!(
        super::bond_state(&networks, &nets),
        vec![(7, true, false), (7, true, false), (3, false, true)],
        "one entry per route, in route order, with the cap and the allowance it is under"
    );

    // A machine whose routes carry no adapter id — which is every machine this build
    // has run on so far — has nothing to attribute and nothing that can shift.
    assert_eq!(super::bond_state(&networks, &[None, None]), Vec::new());
}

/// The bond test above with the limit moved from rate to bytes: the adapter that
/// has met its allowance is never claimed again, and the one beside it carries
/// the whole file instead of the job failing.
#[test]
fn an_adapter_out_of_bytes_carries_no_block_while_its_bond_finishes() {
    let size = 2 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("net-quota", &chaos.url(""), Some(512 * 1024), Some(4));
    let (wired, wifi) = note_pair(&fx);
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };
    let out = spent_scope(64 * 1024);

    let ctx = RunContext {
        routes: vec![
            route(wired.id, "n1", "Ethernet", "127.0.0.1"),
            route(wifi.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        networks: scope_map(&[(wired.id, Arc::clone(&out))]),
        ..fx.ctx()
    };

    let t = Instant::now();
    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("quota bond");
    let elapsed = t.elapsed();

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert!(
        elapsed < Duration::from_secs(8),
        "the job waited on the spent adapter instead of using the open one: {elapsed:?}"
    );

    let carried = device_bytes(&fx);
    assert_eq!(
        carried,
        vec![(wifi.id, size as i64)],
        "an adapter out of bytes still had a block claimed on it"
    );
    assert_eq!(
        out.used(),
        64 * 1024,
        "a shut adapter must not be booked what it never carried"
    );
}

/// The other half of ruling 4 with a transfer behind it. Both adapters are shut,
/// so the claim loop waits and no byte moves — which is what makes the delay
/// below un-racy: the job cannot finish before the allowance is raised.
#[test]
fn a_ceiling_raised_while_the_job_runs_gives_the_adapter_its_blocks_back() {
    let size = 2 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("quota-resume", &chaos.url(""), Some(512 * 1024), Some(4));
    let (wired, wifi) = note_pair(&fx);
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };
    let raised = spent_scope(64 * 1024);
    let shut = spent_scope(64 * 1024);
    let networks = scope_map(&[
        (wired.id, Arc::clone(&raised)),
        (wifi.id, Arc::clone(&shut)),
    ]);

    let late = Arc::clone(&raised);
    let raiser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        late.set_allowance(size as u64 * 4, super::NetScope::MONTH);
    });

    let ctx = RunContext {
        routes: vec![
            route(wired.id, "n1", "Ethernet", "127.0.0.1"),
            route(wifi.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        networks,
        ..fx.ctx()
    };

    let t = Instant::now();
    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("quota reopened");
    let elapsed = t.elapsed();
    raiser.join().unwrap();

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert!(
        elapsed >= Duration::from_millis(250),
        "the job moved bytes while every adapter was shut, so nothing waited: {elapsed:?}"
    );
    let carried = device_bytes(&fx);
    assert!(
        carried.iter().any(|(id, _)| *id == wired.id),
        "the adapter whose allowance was raised never got a block back: {carried:?}"
    );
    assert!(
        !carried.iter().any(|(id, _)| *id == wifi.id),
        "the adapter still out of bytes was let through anyway: {carried:?}"
    );
}

/// The 2026-11-01 public-host failure: eight streams against a rate-limited
/// host, one refused block after another, the job dead at "the rest kept
/// failing". A `Retry-After` is a waiting instruction from the server and the
/// stream count is the thing the server is objecting to, so both are honoured.
#[test]
fn rate_limit_is_awaited_and_narrows_the_streams() {
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    // Two of the four blocks come back 429 with the server's own one second.
    let url = chaos.url("?throttle=2&after=1");
    let fx = Fixture::new("throttle", &url, Some(128 * 1024), Some(4));

    let t = Instant::now();
    fx.run().expect("the job waits instead of dying");
    let elapsed = t.elapsed();

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size), "a refused chunk is retried, not lost");

    // Loopback moves 512 KiB in well under a tenth of this, so the extra second
    // can only be the wait the server asked for — our own backoff is 250 ms.
    assert!(elapsed >= Duration::from_millis(900), "Retry-After ignored: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(8), "the limit stalled the job: {elapsed:?}");

    let events = fx.history();
    assert!(events.contains(&"throttled".to_string()), "{events:?}");

    let reasons = fx.attempt_reasons();
    let chips: Vec<&str> = reasons.iter().map(|(chip, _)| chip.as_str()).collect();
    let sentences: Vec<&str> = reasons.iter().map(|(_, why)| why.as_str()).collect();
    let held: Vec<u64> = sentences
        .iter()
        .filter_map(|r| r.split("holding ").nth(1)?.split(' ').next()?.parse().ok())
        .collect();
    assert!(
        sentences.iter().any(|r| r.contains("429") && r.contains("holding")),
        "the refused attempt has to say it narrowed the bond: {reasons:?}"
    );
    assert!(
        held.iter().min() < Some(&4),
        "a rate limit must cost streams, not only time: {held:?}"
    );
    assert!(
        chips.iter().any(|c| c.starts_with("429 · ")),
        "the chip itself has to name the refusal: {chips:?}"
    );
    assert!(
        chips.iter().all(|c| c.chars().count() <= 24),
        "a chip wider than the column reads as an ellipsis: {chips:?}"
    );
}

/// The same refusal on a host that ignores ranges: one stream, and the wait is
/// still the server's rather than our own doubling.
#[test]
fn the_single_stream_waits_out_the_limit_too() {
    let size = 512 * 1024;
    let chaos = Chaos::start(size);
    // The whole file arrives as one body, so the refusal has to be on the
    // single request: `norange` makes the server answer 200, and `throttle`
    // still bites because the request is not a range probe.
    let url = chaos.url("?norange=1&throttle=1&after=1");
    let fx = Fixture::new("throttle-single", &url, None, None);

    let t = Instant::now();
    fx.run().expect("the single stream waits");
    let elapsed = t.elapsed();

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert!(elapsed >= Duration::from_millis(900), "Retry-After ignored: {elapsed:?}");
    assert!(fx.history().iter().any(|k| k == "throttled"), "got {:?}", fx.history());
}

#[test]
fn the_gate_halves_to_one_and_creeps_back() {
    let gate = super::Gate::new(8);
    // Each halving reports the width it was called from, because that is the
    // width the host just refused and the ramp has to remember.
    assert_eq!(gate.halve(), (8, 4));
    assert_eq!(gate.halve(), (4, 2));
    assert_eq!(gate.halve(), (2, 1));
    assert_eq!(gate.halve(), (1, 1), "one stream is the floor: a limit must not stall the job");
    assert_eq!(gate.width(), 1);

    // Recovery is additive and capped: a job narrowed once at the top of a long
    // file must not spend the rest of it on one connection, but it also never
    // exceeds the width the dial asked for.
    assert_eq!(gate.widen(8), 2);
    assert_eq!(gate.widen(8), 3);
    for _ in 0..10 {
        gate.widen(8);
    }
    assert_eq!(gate.width(), 8, "widening stops at the requested width");
    assert_eq!(gate.widen(8), 8, "already as wide as the job allows");

    assert_eq!(super::Gate::new(1).halve(), (1, 1), "a single worker is already at the floor");
    assert!(super::Gate::new(0).try_acquire(), "a zero budget still lets one through");

    let narrowed = super::Gate::new(2);
    narrowed.halve();
    assert!(narrowed.try_acquire(), "the floor still serves one connection");
    assert!(!narrowed.try_acquire(), "one budget slot means one connection at a time");
}

/// The Settings dial promised 32 streams; with a fixed 8 MiB slice a 100 MB
/// file only affords 13 of them, so the rest of the control was a lie. Sizing
/// the slice from the stream count is what makes the number mean something.
#[test]
fn streams_ask_for_and_get_their_own_chunks() {
    let size = 8 * 1024 * 1024;
    let chaos = Chaos::start(size);
    // No stored block size: the job sizes itself, and eight streams against an
    // eight megabyte file cannot each keep an 8 MiB slice.
    let fx = Fixture::new("adaptive", &chaos.url(""), None, Some(8));

    fx.run().expect("adaptive plan download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert_eq!(fx.num_column("block_size"), Some(1024 * 1024));
    assert_eq!(fx.num_column("block_count"), Some(8), "eight streams, eight chunks");

    let attempts: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM block_attempts WHERE download_id = ?1 AND state = 'done'",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 8, "every stream pulls a chunk of its own");
}

/// A hostile host: sixteen chunks, four workers, and ten of the requests come
/// back 429. The halving must be able to reach the floor and the job must still
/// land every byte — a rate limit is allowed to cost time, never the file.
#[test]
fn a_host_that_limits_almost_every_chunk_still_loses_nothing() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let url = chaos.url("?throttle=10&after=1");
    // 256 KiB slices over a 4 MiB file is sixteen chunks against four workers.
    let fx = Fixture::new("throttle-storm", &url, Some(256 * 1024), Some(4));

    fx.run().expect("the job waits its way through");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    let events = fx.history();
    assert!(events.contains(&"throttled".to_string()), "{events:?}");
    assert!(events.contains(&"coverage-ok".to_string()), "{events:?}");
    assert!(events.contains(&"published".to_string()), "{events:?}");

    let done_blocks: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM blocks WHERE download_id = ?1 AND state = 'done'",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(done_blocks, 16, "no chunk may be given up while the host still answers");
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// A target on this machine names no card.
///
/// The route lookup for a loopback destination answers `127.0.0.1`, and that is
/// not an address any adapter row carries, so the bytes stay unattributed — the
/// same answer the harness gave before attribution existed.
#[test]
fn loopback_traffic_is_not_credited_to_a_cable() {
    let fx = Fixture::new("egress-loopback", "http://127.0.0.1:9/", None, None);
    crate::db::note_network(&fx.conn, "AA:BB:CC:11:22:33", "Only Cable", "ethernet", true)
        .unwrap();

    let mut routes = vec![super::Route::default_route()];
    super::attribute_egress(&fx.conn, "http://127.0.0.1:9/", &mut routes);

    assert_eq!(routes[0].network_id, None, "loopback leaves no adapter");
    assert_eq!(routes[0].color, super::live::UNATTRIBUTED);
    assert_eq!(routes[0].bind, None, "attributing must not start binding");
}

/// The unbonded route is credited to the interface the routing table uses for it.
///
/// The destination is one of this machine's own routable addresses, so the
/// lookup answers with the adapter that owns it. Nothing is sent: a connected
/// UDP socket picks a route and stops there. This is the case the single-NIC
/// machine lives in, where the card's only body line used to stay `0 B`.
#[test]
fn unbonded_bytes_are_credited_to_the_interface_that_carried_them() {
    let adapters = crate::engine::network::enumerate().unwrap_or_default();
    let fx = Fixture::new("egress-self", "http://127.0.0.1:9/", None, None);

    let Some(adapter) = adapters.first() else {
        // No usable adapter on this box: there is nothing to credit, which is
        // what the code answers and what the card already shows.
        let mut routes = vec![super::Route::default_route()];
        super::attribute_egress(&fx.conn, "https://example.com/", &mut routes);
        assert_eq!(routes[0].network_id, None);
        return;
    };

    let hint = crate::engine::network::identity(adapter);
    let row = crate::db::note_network(
        &fx.conn,
        &hint,
        &adapter.name,
        adapter.kind.as_str(),
        true,
    )
    .unwrap();
    let url = format!("http://{}:80/", adapter.local_ip);

    // The claim being tested: the routing table names this adapter for its own
    // address. If it answered anything else, the mapping below proves nothing.
    let source = super::super::bond::egress_source(&url).expect("a route to our own address");
    assert_eq!(
        source.to_string(),
        adapter.local_ip,
        "the lookup must return the address we dial back at ourselves"
    );

    let mut routes = vec![super::Route::default_route()];
    super::attribute_egress(&fx.conn, &url, &mut routes);

    assert_eq!(routes[0].network_id, Some(row.id), "the card gets the bytes");
    assert_eq!(routes[0].color, "n1", "and the stripe matches the card");
    assert_eq!(routes[0].bind, None, "the transfer is still unbound");
}

/// Hand-driven proof, like the folder picker: this dials the public internet and
/// then reads what the engine booked. Run it, then open Networks — the card for
/// the adapter that carries this machine's traffic has to show the bytes rather
/// than `Data downloaded: 0 B`. Out of the gate because it needs a route out.
#[test]
#[ignore = "dials the public internet; run by hand"]
fn unbonded_traffic_from_the_internet_books_its_adapter() {
    let fx = Fixture::new("egress-live", "https://example.com/", None, None);

    // Seed the card rows exactly as `ipc.rs`'s network refresh does: enumerate,
    // then upsert by the same identity key. Without them attribution has no card
    // to charge, which is itself the finding.
    for adapter in crate::engine::network::enumerate().unwrap_or_default().iter() {
        crate::db::note_network(
            &fx.conn,
            &crate::engine::network::identity(adapter),
            &adapter.name,
            adapter.kind.as_str(),
            true,
        )
        .unwrap();
    }
    let seeded: i64 = fx
        .conn
        .query_row("SELECT count(*) FROM networks", [], |r| r.get(0))
        .unwrap();

    let live = Arc::new(super::live::Live::new());
    live.watch();
    let ctx = RunContext {
        live: Some(Arc::clone(&live)),
        ..fx.ctx()
    };

    super::run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("download");

    let booked: Vec<(i64, i64)> = {
        let mut stmt = fx
            .conn
            .prepare("SELECT network_id, bytes FROM net_usage ORDER BY network_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let stripe: Vec<i64> = {
        let mut stmt = fx
            .conn
            .prepare("SELECT DISTINCT network_id FROM block_attempts WHERE download_id = ?1")
            .unwrap();
        stmt.query_map([fx.id], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let verified: i64 = fx
        .conn
        .query_row(
            "SELECT verified_bytes FROM downloads WHERE id = ?1",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "status {} / verified {verified} / networks {seeded} / net_usage {booked:?} / attempt network ids {stripe:?}",
        fx.status()
    );
    assert!(
        !booked.is_empty() && booked.iter().all(|(_, bytes)| *bytes > 0),
        "the adapter that carried this stayed at 0 B: {booked:?}"
    );
}

/* --- link refresh ---------------------------------------------------------- */

/// A job frozen halfway through its plan, which is what an expired signature
/// leaves behind. Seeded rather than flown so the byte and block figures are
/// certain instead of a race with the worker scheduler: a refresh reads only
/// the row, the `blocks` table and the staging file.
fn seed_half_done(fx: &Fixture, url: &str, etag: &str, size: usize, block: usize) {
    let half = size / 2;
    let blocks = size / block;
    let staging = fx.dir.join("dl").join("file.bin.sucrpart");
    std::fs::write(&staging, &body(size)[..half]).unwrap();
    fx.conn
        .execute(
            "UPDATE downloads SET status = 'paused', url = ?1, final_url = ?1, etag = ?2, \
             total_bytes = ?3, block_size = ?4, block_count = ?5, verified_bytes = ?6, \
             staging_path = ?7, streams_mode = 'adaptive' WHERE id = ?8",
            params![
                url,
                etag,
                size as i64,
                block as i64,
                blocks as i64,
                half as i64,
                staging.to_string_lossy(),
                fx.id
            ],
        )
        .unwrap();
    for idx in 0..blocks {
        let start = (idx * block) as i64;
        let (state, done) = if idx < blocks / 2 {
            ("done", block as i64)
        } else {
            ("pending", 0)
        };
        fx.conn
            .execute(
                "INSERT INTO blocks (download_id, idx, range_start, range_end, state, bytes_done) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![fx.id, idx as i64, start, start + block as i64 - 1, state, done],
            )
            .unwrap();
    }
}

fn done_blocks(fx: &Fixture) -> i64 {
    fx.conn
        .query_row(
            "SELECT count(*) FROM blocks WHERE download_id = ?1 AND state = 'done'",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap()
}

fn refresh_payload(fx: &Fixture) -> serde_json::Value {
    let text: String = fx
        .conn
        .query_row(
            "SELECT payload_json FROM history_events WHERE download_id = ?1 \
             AND kind = 'url-refreshed'",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The newest event of a kind, decoded — what the activity trail renders.
fn latest_payload(fx: &Fixture, kind: &str) -> serde_json::Value {
    let text: String = fx
        .conn
        .query_row(
            "SELECT payload_json FROM history_events WHERE download_id = ?1 AND kind = ?2 \
             ORDER BY id DESC LIMIT 1",
            params![fx.id, kind],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_new_link_is_measured_against_the_plan_it_has_to_inherit() {
    let fresh = |ranges: bool,
                 total: Option<u64>,
                 etag: Option<&str>,
                 modified: Option<&str>| {
        Probe {
            accept_ranges: ranges,
            total_bytes: total,
            etag: etag.map(str::to_string),
            last_modified: modified.map(str::to_string),
            ..Default::default()
        }
    };
    // No ranges means no resume, however well the rest matches.
    assert_eq!(
        check_refresh(Some(1024), None, None, &fresh(false, Some(1024), None, None)).unwrap_err().code,
        "refresh-no-range"
    );
    // Length and validator both agreeing is the promise a refresh needs.
    assert_eq!(
        check_refresh(Some(1024), Some("\"a\""), None, &fresh(true, Some(1024), Some("\"a\""), None))
            .unwrap(),
        "etag"
    );
    // A length that moved is refused, and the sentence carries both figures,
    // because this is the one line the user actually reads.
    let err = check_refresh(
        Some(2 * 1024 * 1024),
        Some("\"a\""),
        None,
        &fresh(true, Some(4 * 1024 * 1024), Some("\"a\""), None),
    )
    .unwrap_err();
    assert_eq!(err.code, "refresh-size");
    assert_eq!(err.message, "Size mismatch: this download is 2 MB, the new link is 4 MB");
    // The same length can still be a different file; only the validator knows.
    assert_eq!(
        check_refresh(Some(1024), Some("\"a\""), None, &fresh(true, Some(1024), Some("\"b\""), None))
            .unwrap_err()
            .code,
        "refresh-changed"
    );
    // No ETag on either side falls through to Last-Modified, and when neither
    // exists the length is all the promise there is — said plainly, not invented.
    assert_eq!(
        check_refresh(
            Some(1024),
            None,
            Some("Sun, 15 Dec 2024 22:13:20 GMT"),
            &fresh(true, Some(1024), None, Some("Sun, 15 Dec 2024 22:13:20 GMT")),
        )
        .unwrap(),
        "last-modified"
    );
    assert_eq!(
        check_refresh(Some(1024), None, Some("old"), &fresh(true, Some(1024), None, Some("new")))
            .unwrap_err()
            .code,
        "refresh-changed"
    );
    assert_eq!(
        check_refresh(Some(1024), None, None, &fresh(true, Some(1024), None, None)).unwrap(),
        "none"
    );
    // Ranges honoured yet no length in the answer: nothing to compare the
    // finished slices against, so the plan cannot be trusted.
    assert_eq!(
        check_refresh(Some(1024), None, None, &fresh(true, None, None, None)).unwrap_err().code,
        "refresh-unknown-size"
    );
    // A job that never probed has no plan to break.
    assert_eq!(
        check_refresh(None, None, None, &fresh(true, Some(99), None, None)).unwrap(),
        "none"
    );
}

#[test]
fn a_link_that_is_not_a_link_never_reaches_the_row() {
    let fx = Fixture::new("refuse", "http://127.0.0.1:9/gone.bin", None, None);
    let db = fx.dir.join("sucr.db");
    assert_eq!(refresh_url(&db, fx.id, "   ").unwrap_err().code, "refresh-scheme");
    assert_eq!(
        refresh_url(&db, fx.id, "file:///C:/Users/you/thing.zip").unwrap_err().code,
        "refresh-scheme"
    );
    assert_eq!(refresh_url(&db, 4242, "https://x.test/a.bin").unwrap_err().code, "missing");
    // A row whose workers still own it cannot be re-pointed out from under them,
    // and a finished one has nothing left to resume. Both answers come from the
    // row, before the new link is asked for anything at all.
    fx.conn
        .execute("UPDATE downloads SET status = 'downloading' WHERE id = ?1", [fx.id])
        .unwrap();
    assert_eq!(
        refresh_url(&db, fx.id, "https://x.test/a.bin").unwrap_err().code,
        "refresh-running"
    );
    for done in ["finished", "verified"] {
        fx.conn
            .execute("UPDATE downloads SET status = ?1 WHERE id = ?2", params![done, fx.id])
            .unwrap();
        assert_eq!(
            refresh_url(&db, fx.id, "https://x.test/a.bin").unwrap_err().code,
            "refresh-complete"
        );
    }
    assert_eq!(fx.column("url").unwrap(), "http://127.0.0.1:9/gone.bin");
}

#[test]
fn a_fresh_link_keeps_the_slices_the_dead_one_finished() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let dead = Chaos::start(size);
    let fresh = Chaos::start(size);
    let fx = Fixture::new("keep", &dead.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &dead.url(""), &dead.etag, size, block);
    assert_eq!(done_blocks(&fx), 2);

    refresh_url(&fx.dir.join("sucr.db"), fx.id, &fresh.url("")).expect("same file, new link");

    let want = fresh.url("");
    assert_eq!(fx.column("url").as_deref(), Some(want.as_str()));
    assert_eq!(fx.num_column("verified_bytes"), Some((size / 2) as i64));
    assert_eq!(done_blocks(&fx), 2, "the finished slices survived the swap");
    let payload = refresh_payload(&fx);
    assert_eq!(payload["keptBytes"], serde_json::json!((size / 2) as i64));
    assert_eq!(payload["validator"], serde_json::json!("etag"));
    assert_eq!(payload["newHost"].as_str().unwrap(), super::host_of(&want));
    assert_eq!(payload["oldHost"].as_str().unwrap(), super::host_of(&dead.url("")));

    fx.run().expect("the rest arrives on the new link");
    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    // The resume has to announce the chunks it actually inherited: two of the
    // four were finished on the dead link.
    let resumed = latest_payload(&fx, "resumed");
    assert_eq!(resumed["done"], serde_json::json!(2));
    assert_eq!(resumed["blocks"], serde_json::json!(4));
    // A refresh that kept the plan must not throw it away afterwards:
    // `resume-reset` is what a run writes when it decides the bytes already on
    // disk are worthless, and here half of them were not.
    let kinds = fx.history();
    let at = kinds.iter().position(|k| k == "url-refreshed").unwrap();
    assert!(
        !kinds[at..].iter().any(|k| k == "resume-reset"),
        "the kept half was discarded anyway: {kinds:?}"
    );
}

/// A partial file deleted off the disk is its own reason to start over. The run
/// used to write the same two flags it writes for a server that changed its
/// mind, and the Activity trail told the user the validator had moved.
#[test]
fn a_missing_partial_is_named_as_the_reason_the_bytes_were_dropped() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let server = Chaos::start(size);
    let fx = Fixture::new("staging-gone", &server.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &server.url(""), &server.etag, size, block);

    std::fs::remove_file(fx.dir.join("dl").join("file.bin.sucrpart")).unwrap();

    fx.run().expect("the whole file is pulled again");
    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));

    let payload = latest_payload(&fx, "resume-reset");
    assert_eq!(
        payload["stagingMissing"],
        serde_json::json!(true),
        "the trail must name the missing partial"
    );
    assert_eq!(payload["planChanged"], serde_json::json!(false));
    assert_eq!(payload["validatorChanged"], serde_json::json!(false));
}

/// A moved stamp is a claim about the file, not a measurement of it. This test
/// used to prove the claim was enough to throw the bytes away; the byte sample
/// now hears it first, and on a host whose bytes did not change the claim loses.
/// The drop it documented still happens — it just needs the bytes to agree that
/// they changed (`a_moved_validator_over_differing_bytes_still_drops_the_partial`).
#[test]
fn a_rotated_stamp_with_no_etag_is_answered_by_the_bytes_and_keeps_them() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let server = Chaos::start(size);
    let url = server.url("?noetag=1");
    let fx = Fixture::new("modified-sampled", &url, Some(block as u64), Some(2));
    seed_half_done(&fx, &url, "", size, block);
    let stale = shift_stamp(&server.last_modified, -3_600);
    // Both sides really carry no ETag: the row cleared, the host silent. What is
    // left to compare is the stamp, and the stamp moved.
    fx.conn
        .execute(
            "UPDATE downloads SET etag = NULL, last_modified = ?1 WHERE id = ?2",
            params![stale, fx.id],
        )
        .unwrap();
    assert_eq!(fx.column("etag"), None, "the fixture must serve no ETag");

    fx.run().expect("the second half arrives on top of the first");
    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));

    let sampled = latest_payload(&fx, "bytes-sampled");
    assert_eq!(sampled["outcome"], serde_json::json!("matched"));
    assert_eq!(
        sampled["validator"],
        serde_json::json!("last-modified"),
        "the trail still names which validator moved, even though it lost"
    );
    assert_eq!(sampled["kept"], serde_json::json!(true));
    // Half of 2 MiB is a 1 MiB run of finished blocks, which holds all eight
    // 16 KiB windows without one byte of overlap.
    assert_eq!(sampled["covered"], serde_json::json!(size / 2));
    assert_eq!(sampled["windows"], serde_json::json!(8));
    assert!(
        !fx.history().iter().any(|k| k == "resume-reset"),
        "a kept partial must not also report a reset: {:?}",
        fx.history()
    );
    // `resumed` with two blocks already done is the proof the plan was inherited,
    // not rebuilt: a reset writes a `plan` row and re-pulls from zero.
    assert_eq!(
        latest_payload(&fx, "resumed")["done"],
        serde_json::json!(2),
        "{:?}",
        fx.history()
    );
}

/// The sample can only ever rescue bytes, never lose them: when the host's
/// bytes are not the bytes on disk, the old decision stands and the trail says
/// who made it.
#[test]
fn a_moved_validator_over_differing_bytes_still_drops_the_partial() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let server = Chaos::start(size);
    // corrupt=0 flips the first byte of every slice this host serves, so the
    // very first window the sampler asks for comes back wrong.
    let url = server.url("?corrupt=0&etag=rotated");
    let fx = Fixture::new("sampled-differed", &url, Some(block as u64), Some(2));
    seed_half_done(&fx, &url, &server.etag, size, block);
    fx.conn
        .execute(
            "UPDATE downloads SET etag = ?1 WHERE id = ?2",
            params!["\"before-the-rotation\"", fx.id],
        )
        .unwrap();

    fx.run().expect("the file is pulled again from the front");
    assert_eq!(fx.status(), "finished");

    let sampled = latest_payload(&fx, "bytes-sampled");
    assert_eq!(sampled["outcome"], serde_json::json!("differed"));
    assert_eq!(sampled["kept"], serde_json::json!(false));
    assert_eq!(
        sampled["windows"],
        serde_json::json!(0),
        "the hearing ends at the first window that disagreed"
    );
    let reset = latest_payload(&fx, "resume-reset");
    assert_eq!(reset["validatorChanged"], serde_json::json!(true));
    assert_eq!(
        reset["sampleOutcome"],
        serde_json::json!("differed"),
        "the reset row says the bytes were asked and refused to agree"
    );
    assert_eq!(done_blocks(&fx), size as i64 / block as i64);
}

/// A host that will not serve a range gives the sampler nothing to ask with, so
/// the sample is not attempted at all and the decision is exactly what it was
/// before this feature existed.
#[test]
fn a_host_that_ignores_range_gets_no_hearing() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let server = Chaos::start(size);
    let url = server.url("?norange=1&etag=rotated");
    let fx = Fixture::new("sampled-no-range", &url, Some(block as u64), Some(2));
    seed_half_done(&fx, &url, &server.etag, size, block);

    fx.run().expect("the whole file arrives over one stream");
    assert_eq!(fx.status(), "finished");
    assert!(
        !fx.history().iter().any(|k| k == "bytes-sampled"),
        "no ranges means no sample row at all: {:?}",
        fx.history()
    );
    assert_eq!(latest_payload(&fx, "resume-reset")["validatorChanged"], serde_json::json!(true));
}

/// The ledger can claim a run of finished blocks whose file ends in the middle
/// of it — a crash between a write and the row that records it. The ranges
/// offered to the sampler are cut to what the file actually reaches, so what is
/// left here is a quarter of one window: nothing to read. The bytes are dropped
/// exactly as they were before the sampler existed, and the hearing says why.
#[test]
fn a_partial_whose_file_fell_behind_its_ledger_leaves_the_sample_nothing_to_read() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let server = Chaos::start(size);
    let url = server.url("?etag=rotated");
    let fx = Fixture::new("sampled-short-file", &url, Some(block as u64), Some(2));
    seed_half_done(&fx, &url, &server.etag, size, block);
    let staging = fx.dir.join("dl").join("file.bin.sucrpart");
    // The ledger still says 1 MiB landed; the file holds a quarter of one window.
    std::fs::write(&staging, &body(size)[..super::SAMPLE_WINDOW as usize / 4]).unwrap();
    assert!(
        super::covered_ranges(&fx.conn, fx.id, Some(&staging)).is_empty(),
        "cut to the file, not one range is left wide enough to hold a window"
    );

    fx.run().expect("the file is pulled again from the front");
    assert_eq!(fx.status(), "finished");

    let sampled = latest_payload(&fx, "bytes-sampled");
    assert_eq!(sampled["outcome"], serde_json::json!("no-evidence"));
    assert_eq!(sampled["windows"], serde_json::json!(0));
    assert_eq!(sampled["kept"], serde_json::json!(false));
    assert_eq!(
        latest_payload(&fx, "resume-reset")["sampleOutcome"],
        serde_json::json!("no-evidence"),
        "the reset row says the bytes were asked and there were none left to ask about"
    );
}

/* --- sample arithmetic ----------------------------------------------------- */

/// Where the windows land is the whole guarantee the feature makes, so it is
/// tested without a socket: inside what the disk holds, never past it, never
/// across a gap, and fewer rather than duplicated when there is little to read.
#[test]
fn the_sample_windows_are_spread_over_the_bytes_the_disk_holds() {
    let w = super::SAMPLE_WINDOW;
    let meg = 1024 * 1024;

    // One job that ran on one stream: the whole front is one covered range.
    let run = super::spread_windows(&[(0, 2 * meg)], super::SAMPLE_COUNT, w);
    assert_eq!(run.len(), 8);
    assert_eq!(run[0], 0);
    assert_eq!(run[7] + w, 2 * meg, "the last window ends exactly at the covered bytes");
    for pair in run.windows(2) {
        assert!(pair[1] - pair[0] >= w, "windows overlap: {run:?}");
    }

    // The shape a paused bonded job really leaves: eight blocks, each half-full,
    // each with a gap after it. Every window has to sit inside one block's own
    // bytes — a read that straddled the gap would land on the file's zeros and
    // report a difference the host never made.
    let block = 512 * 1024;
    let fronts: Vec<(u64, u64)> = (0..8u64).map(|i| (i * block, block / 2)).collect();
    let wide = super::spread_windows(&fronts, super::SAMPLE_COUNT, w);
    assert_eq!(wide.len(), 8, "{wide:?}");
    for off in &wide {
        assert!(
            fronts
                .iter()
                .any(|(start, len)| *off >= *start && *off + w <= *start + *len),
            "window at {off} straddles a gap: {wide:?}"
        );
    }

    // Bytes that cannot fit eight windows stop the count instead of re-reading.
    let tight = super::spread_windows(&[(0, 5 * w)], super::SAMPLE_COUNT, w);
    assert_eq!(tight.len(), 5, "five windows fit in five windows' worth of bytes");
    assert_eq!(tight[0], 0);
    assert_eq!(tight[4] + w, 5 * w);

    // One window's worth of bytes is one window, at the front.
    assert_eq!(super::spread_windows(&[(0, w)], super::SAMPLE_COUNT, w), vec![0]);

    // Nothing at all below a window, from an empty disk, or for a degenerate ask.
    assert!(super::spread_windows(&[(0, w - 1)], super::SAMPLE_COUNT, w).is_empty());
    assert!(super::spread_windows(&[], super::SAMPLE_COUNT, w).is_empty());
    assert!(super::spread_windows(&[(0, meg)], 0, w).is_empty());
    assert!(super::spread_windows(&[(0, meg)], 8, 0).is_empty());
}

/// Blocks finish out of order, so a run of *finished* blocks from the first byte
/// is not what the disk holds: a job widened to eight streams leaves every block
/// half-full and none of them finished, which is what the launched exe measured
/// the first version of this rule answering as zero. What each block's own count
/// says landed is on disk, from that block's front — so a hole costs the bytes
/// inside the hole and nothing behind it.
#[test]
fn a_hole_costs_its_own_block_and_not_the_blocks_behind_it() {
    let size = 4 * 1024 * 1024;
    let block = 512 * 1024;
    let server = Chaos::start(size);
    let fx = Fixture::new("prefix-hole", &server.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &server.url(""), &server.etag, size, block);
    let file = fx.dir.join("dl").join("file.bin.sucrpart");
    let covered = || -> u64 {
        super::covered_ranges(&fx.conn, fx.id, Some(&file))
            .iter()
            .map(|(_, len)| len)
            .sum()
    };
    assert_eq!(covered(), size as u64 / 2, "four finished blocks, each whole");

    // Pull one of the four back to nothing: its bytes leave the offer, the three
    // finished blocks behind it stay in it.
    fx.conn
        .execute(
            "UPDATE blocks SET state = 'pending', bytes_done = 0 WHERE download_id = ?1 AND idx = 1",
            [fx.id],
        )
        .unwrap();
    assert_eq!(covered(), (size as u64 / 2) - block as u64);

    // A count a byte short of its length is a range a byte short. It does not
    // vanish: the sampler reads what the block says it has.
    fx.conn
        .execute(
            "UPDATE blocks SET bytes_done = bytes_done - 1 WHERE download_id = ?1 AND idx = 0",
            [fx.id],
        )
        .unwrap();
    assert_eq!(covered(), (size as u64 / 2) - block as u64 - 1);
}

/// An ETag outranks a timestamp: a host that keeps saying the same ETag has not
/// changed the file, whatever its clock now claims. Without this arm, adding
/// the timestamp compare would discard resumable bytes over a clock skew.
#[test]
fn a_rotated_stamp_alongside_an_agreeing_etag_keeps_the_bytes() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let server = Chaos::start(size);
    let fx = Fixture::new("etag-outranks-stamp", &server.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &server.url(""), &server.etag, size, block);
    let skewed = shift_stamp(&server.last_modified, -3_600);
    fx.conn
        .execute(
            "UPDATE downloads SET last_modified = ?1 WHERE id = ?2",
            params![skewed, fx.id],
        )
        .unwrap();

    fx.run().expect("the second half arrives");
    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    assert_eq!(done_blocks(&fx), size as i64 / block as i64);
    assert!(
        !fx.history().iter().any(|k| k == "resume-reset"),
        "an agreeing ETag was overridden by a moving clock: {:?}",
        fx.history()
    );
    let resumed = latest_payload(&fx, "resumed");
    assert_eq!(resumed["done"], serde_json::json!(2));
}

/// Shift an RFC 1123 stamp by a whole number of seconds, so a test can say
/// "the host now claims another time" without a clock of its own and without a
/// new dependency: the chaos stamp is a fixed date, so the arithmetic here is
/// civil-from-days and a `strftime` on the result.
fn shift_stamp(stamp: &str, delta: i64) -> String {
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = stamp.split_whitespace();
    parts.next(); // the weekday name, which this recomputes from the epoch day
    let day: u32 = parts.next().unwrap().parse().unwrap();
    let month = parts.next().unwrap();
    let year: i64 = parts.next().unwrap().parse().unwrap();
    let clock = parts.next().unwrap();
    assert_eq!(parts.next(), Some("GMT"), "{stamp}");

    fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 };
        let doy = (153 * mp + 2) / 5 + d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
    }

    let m = MONTHS
        .iter()
        .position(|x| *x == month)
        .unwrap_or_else(|| panic!("unknown month in {stamp}")) as u32
        + 1;
    let hh = clock[0..2].parse::<i64>().unwrap();
    let mi = clock[3..5].parse::<i64>().unwrap();
    let ss = clock[6..8].parse::<i64>().unwrap();
    let secs = days_from_civil(year, m, day) * 86_400 + hh * 3_600 + mi * 60 + ss + delta;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{}, {d:02} {} {y:04} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[days.rem_euclid(7) as usize],
        MONTHS[(mo - 1) as usize],
        sod / 3_600,
        (sod % 3_600) / 60,
        sod % 60
    )
}

#[test]
fn a_link_to_another_file_is_turned_down_and_the_row_is_left_alone() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let dead = Chaos::start(size);
    let rotated = Chaos::start(size);
    let bigger = Chaos::start(4 * 1024 * 1024);
    let fx = Fixture::new("refuse-live", &dead.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &dead.url(""), &dead.etag, size, block);

    // The host swears this is another file, and this time its bytes agree with
    // the claim: `corrupt=0` flips the first byte of every slice, so the first
    // window the sampler reads comes back different and the link is turned down.
    let err = refresh_url(
        &fx.dir.join("sucr.db"),
        fx.id,
        &rotated.url("?etag=rotated&corrupt=0"),
    )
    .unwrap_err();
    assert_eq!(err.code, "refresh-changed");

    // A raw quote in the query is not a legal URL: the agent answers `invalid
    // uri character` before the socket is opened, so the verdict is `http` and
    // not one of the refresh reasons.
    let quoted =
        refresh_url(&fx.dir.join("sucr.db"), fx.id, &rotated.url("?etag=\"rotated\"")).unwrap_err();
    assert_eq!(quoted.code, "http", "{}", quoted.message);

    // Right file, wrong length, and the refusal quotes both. A length that moved
    // is the one case no sample can argue: there is no plan left to inherit.
    let err = refresh_url(&fx.dir.join("sucr.db"), fx.id, &bigger.url("")).unwrap_err();
    assert_eq!(err.code, "refresh-size");
    assert!(err.message.contains("2 MB") && err.message.contains("4 MB"), "{err}");

    // A host that answers the 1-byte probe with the whole body can resume
    // nothing, so it is not accepted as a replacement link.
    let err = refresh_url(&fx.dir.join("sucr.db"), fx.id, &rotated.url("?norange=1")).unwrap_err();
    assert_eq!(err.code, "refresh-no-range");

    // Four refusals, and the job is exactly where it was: still pointing at
    // the link it had, still holding its half, and nothing new in the trail.
    assert_eq!(fx.column("url").unwrap(), dead.url(""));
    assert_eq!(fx.num_column("verified_bytes"), Some((size / 2) as i64));
    assert_eq!(done_blocks(&fx), 2);
    assert!(!fx.history().contains(&"url-refreshed".to_string()));
    // The error written before the refresh stays put too, so the card still
    // explains why the job stopped.
    fx.conn
        .execute(
            "UPDATE downloads SET error_code = 'http-status', error_hint = '403' WHERE id = ?1",
            [fx.id],
        )
        .unwrap();
    assert!(
        refresh_url(
            &fx.dir.join("sucr.db"),
            fx.id,
            &rotated.url("?etag=rotated&corrupt=0")
        )
        .is_err()
    );
    assert_eq!(fx.column("error_hint").as_deref(), Some("403"));
}

/// The same claim over the same bytes is not a different file: it is what a
/// rebuild of one looks like. The relink used to turn it down and force a full
/// re-pull; now the sample answers and the link is taken with the half intact.
#[test]
fn a_link_that_only_changed_its_claim_is_taken_because_the_bytes_agree() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let dead = Chaos::start(size);
    let rotated = Chaos::start(size);
    let fx = Fixture::new("accept-sampled", &dead.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &dead.url(""), &dead.etag, size, block);
    fx.conn
        .execute(
            "UPDATE downloads SET error_code = 'expired', error_hint = '403' WHERE id = ?1",
            [fx.id],
        )
        .unwrap();

    refresh_url(&fx.dir.join("sucr.db"), fx.id, &rotated.url("?etag=rotated"))
        .expect("the bytes settle the claim");

    let payload = refresh_payload(&fx);
    assert_eq!(
        payload["validator"],
        serde_json::json!("sample"),
        "the reason this link was accepted is the bytes, not a validator"
    );
    assert_eq!(payload["sampleOutcome"], serde_json::json!("matched"));
    assert_eq!(payload["sampleWindows"], serde_json::json!(8));
    assert_eq!(payload["sampleCovered"], serde_json::json!(size / 2));
    assert_eq!(payload["keptBytes"], serde_json::json!(size / 2));

    // Taken, and nothing was thrown away to take it.
    assert_eq!(fx.column("url").unwrap(), rotated.url("?etag=rotated"));
    assert_eq!(fx.column("etag").as_deref(), Some("rotated"));
    assert_eq!(fx.num_column("verified_bytes"), Some((size / 2) as i64));
    assert_eq!(done_blocks(&fx), 2);
    // The stale error goes with the old link, the same as any accepted refresh.
    assert_eq!(fx.column("error_hint"), None);
}

/// A relink to a host that serves different bytes at the same length is still a
/// different file, however calm the length looks.
#[test]
fn a_relink_whose_bytes_disagree_is_still_turned_down() {
    let size = 2 * 1024 * 1024;
    let block = 512 * 1024;
    let dead = Chaos::start(size);
    let moved = Chaos::start(size);
    let fx = Fixture::new("refuse-sampled", &dead.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &dead.url(""), &dead.etag, size, block);
    fx.conn
        .execute(
            "UPDATE downloads SET etag = ?1 WHERE id = ?2",
            params!["\"rebuilt\"", fx.id],
        )
        .unwrap();

    let err = refresh_url(&fx.dir.join("sucr.db"), fx.id, &moved.url("?corrupt=4096")).unwrap_err();
    assert_eq!(err.code, "refresh-changed");
    assert_eq!(fx.column("etag").as_deref(), Some("\"rebuilt\""));
    assert_eq!(fx.num_column("verified_bytes"), Some((size / 2) as i64));
    assert!(!fx.history().contains(&"url-refreshed".to_string()));
}

#[test]
fn bytes_with_no_file_behind_them_are_not_reported_as_kept() {
    let size = 1024 * 1024;
    let block = 512 * 1024;
    let dead = Chaos::start(size);
    let fresh = Chaos::start(size);
    let fx = Fixture::new("no-staging", &dead.url(""), Some(block as u64), Some(2));
    seed_half_done(&fx, &dead.url(""), &dead.etag, size, block);
    // The row says half of it is on disk; someone deleted that half meanwhile.
    std::fs::remove_file(fx.dir.join("dl").join("file.bin.sucrpart")).unwrap();

    refresh_url(&fx.dir.join("sucr.db"), fx.id, &fresh.url("")).unwrap();

    assert_eq!(refresh_payload(&fx)["keptBytes"], serde_json::json!(0));
    // The plan itself still survives the check: the next run finds no staging
    // file, resets, and says so in its own history row rather than here.
    assert_eq!(fx.num_column("verified_bytes"), Some((size / 2) as i64));
}

#[test]
fn idle_worker_steals_the_straggler_tail() {
    let size = 8 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("tail-split-test", &chaos.url(""), Some(4 * 1024 * 1024), Some(4));
    fx.run().expect("tail split download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));

    let split_events: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM history_events WHERE download_id = ?1 AND kind = 'tail-split'",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert!(split_events >= 1, "at least one tail-split event was recorded in history");

    let done_attempts: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM block_attempts WHERE download_id = ?1 AND state = 'done'",
            [fx.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(done_attempts >= 3, "split sub-attempts recorded in block_attempts: got {done_attempts}");
}

#[test]
fn a_split_block_resumes_from_the_edge_of_the_hole() {
    let plan = [
        BlockSpec { idx: 0, start: 0, end: 99 },
        BlockSpec { idx: 1, start: 100, end: 199 },
    ];
    // The database said block 0 stood at 30 bytes when this run began, block 1
    // at none.
    let resume = [30u64, 0];

    // Block 0 was handed a tail: the parent wrote its front, the child its back,
    // and the middle is still missing even though the sum is 70.
    let split = [true, false];
    let sum = [70u64, 50];
    assert_eq!(
        resumable(&sum, &resume, &split, &plan),
        vec![30, 50],
        "a split block that has not finished must not publish a resume point past its hole"
    );

    // Once both halves land, the sum is the whole block and there is nothing to undo.
    let finished = [100u64, 50];
    assert_eq!(resumable(&finished, &resume, &split, &plan), vec![100, 50]);

    // An unsplit block reads straight from the front, so its sum is a real edge
    // and a partial sum is a genuine restart point.
    let unsplit = [false, false];
    assert_eq!(resumable(&sum, &resume, &unsplit, &plan), vec![70, 50]);
}

#[test]
fn the_steal_trigger_is_the_policy_not_the_byte_count() {
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64};
    let mi = 1024u64 * 1024;
    let plan = vec![
        super::BlockSpec { idx: 0, start: 0, end: 8 * mi - 1 },
        super::BlockSpec { idx: 1, start: 8 * mi, end: 12 * mi - 1 },
    ];
    // A block with `pos` of `len` done that last reported `quiet_ms` ago. Both
    // are big enough to split, so only the policy can tell them apart.
    let coord = |pos: u64, len: u64, quiet_ms: i64| super::BlockCoord {
        pos: AtomicU64::new(pos),
        cutoff: AtomicU64::new(len),
        total_done: AtomicU64::new(pos),
        limited_done: AtomicU64::new(0),
        last_ms: AtomicI64::new(super::now_ms() - quiet_ms),
        active: AtomicBool::new(true),
        done_flag: AtomicBool::new(false),
    };
    let pair = |quiet_second: i64| {
        vec![coord(mi, 8 * mi, 0), coord(mi, 4 * mi, quiet_second)]
    };

    // Block 0 has 7 MiB left and is moving; block 1 has 3 MiB left and has been
    // silent for a minute.
    let d = super::try_steal_tail(&pair(60_000), &plan, super::BondPolicy::Dynamic)
        .expect("dynamic takes the biggest tail");
    assert_eq!(d.0, 0, "dynamic follows remaining bytes");

    let h = super::try_steal_tail(&pair(60_000), &plan, super::BondPolicy::Hedging)
        .expect("hedging races the quiet block");
    assert_eq!(h.0, 1, "hedging follows the stall, even against a larger straggler");

    let f = super::try_steal_tail(&pair(60_000), &plan, super::BondPolicy::Failover);
    assert!(f.is_none(), "failover leaves a stalled block alone: its backup is for a drop");

    let quiet = super::try_steal_tail(&pair(0), &plan, super::BondPolicy::Hedging);
    assert!(quiet.is_none(), "a stream that is still reporting chunks is not a hedge candidate");
}

#[test]
fn failover_books_every_byte_to_the_primary() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("failover", &chaos.url(""), Some(1024 * 1024), Some(4));
    let (first, second) = note_pair(&fx);
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };

    let ctx = RunContext {
        routes: vec![
            route(first.id, "n1", "Ethernet", "127.0.0.1"),
            route(second.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        policy: super::BondPolicy::Failover,
        ..fx.ctx()
    };
    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("failover download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));

    let carried: Vec<(i64, i64)> = device_bytes(&fx);
    assert_eq!(
        carried.len(),
        1,
        "the standby device must not carry a byte while the primary works: {carried:?}"
    );
    assert_eq!(carried[0].0, first.id);
    assert_eq!(carried[0].1, size as i64);

    assert_eq!(
        latest_payload(&fx, "plan")["policy"],
        "failover",
        "the plan line names the strategy those bytes were planned under"
    );
}

#[test]
fn hedging_delivers_the_same_file_on_the_same_pair() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("hedge", &chaos.url(""), Some(1024 * 1024), Some(4));
    let (first, second) = note_pair(&fx);
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };

    let ctx = RunContext {
        routes: vec![
            route(first.id, "n1", "Ethernet", "127.0.0.1"),
            route(second.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        policy: super::BondPolicy::Hedging,
        ..fx.ctx()
    };
    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("hedged download");

    // The honest claim on loopback: nothing here stalls, so this proves the
    // hedge path cannot corrupt the accounting. Which is the thing a hedge
    // risks: two streams over one range, one table.
    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));
    let carried: Vec<(i64, i64)> = device_bytes(&fx);
    let total: i64 = carried.iter().map(|(_, b)| *b).sum();
    assert_eq!(
        total,
        size as i64,
        "every byte booked exactly once, whatever the split: {carried:?}"
    );
    assert!(
        carried.iter().all(|(_, b)| *b > 0),
        "a device that booked an attempt booked bytes: {carried:?}"
    );
}

fn note_pair(fx: &Fixture) -> (crate::db::NetworkPrefs, crate::db::NetworkPrefs) {
    // Two loopback sources stand in for two devices: this box has one cable, and
    // what is under test is whether a stream leaves from the address it was
    // told to, not which silicon is on the other end.
    let first = crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:01", "Ethernet", "ethernet", true).unwrap();
    let second = crate::db::note_network(&fx.conn, "AA:BB:CC:00:00:02", "Wi-Fi", "wifi", true).unwrap();
    (first, second)
}

fn device_bytes(fx: &Fixture) -> Vec<(i64, i64)> {
    let mut stmt = fx
        .conn
        .prepare(
            "SELECT network_id, sum(bytes) FROM block_attempts \
             WHERE download_id = ?1 AND network_id IS NOT NULL \
             GROUP BY network_id ORDER BY network_id",
        )
        .unwrap();
    stmt.query_map([fx.id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

// --- the stream ramp ---------------------------------------------------------
//
// Plexo doubles its stream count when everything answers cleanly. This ramp
// grows one stream per measured window and only when the window proves the
// bytes, so the rule is a pure function and gets tested without a socket.

fn healthy(now_ms: i64, bytes: u64, width: usize, pending: usize) -> RampSample {
    RampSample {
        now_ms,
        // Every test below steps `now_ms` by one shipped window, so a healthy window is
        // exactly `window_ms` long. The uneven-span tests overwrite this.
        span_ms: 1_000,
        bytes_delta: bytes,
        // Nothing paced this window: every byte came from the host.
        host_bytes: bytes,
        shifted: false,
        width,
        pending,
        stealable: false,
        quiet: false,
    }
}

/// The shipped clocks, spelled out so a reader sees what a test assumes.
fn clocks() -> RampTuning {
    RampTuning { window_ms: 1_000, share_pct: 50, refuse_cool_ms: 60_000 }
}

/// Windows short enough that a real job reaches several of them before the
/// last block lands. The bill asks only for one extra byte — this covers the
/// shape of the loop, and the priced bar has its own tests.
fn fast_ramp() -> RampTuning {
    RampTuning { window_ms: 100, share_pct: 0, ..Default::default() }
}

#[test]
fn the_band_is_auto_until_a_number_is_named() {
    let dir = temp_dir("band");
    let conn = crate::db::open(&dir.join("sucr.db")).unwrap();
    let band = || super::stream_band(&conn);

    // Nothing chosen, or "auto" chosen: open at the old default, grow to the cap.
    assert_eq!((band().start, band().ceiling), (8, 32), "unset");
    crate::db::set_setting(&conn, "engine.default_streams", Some("auto")).unwrap();
    assert_eq!((band().start, band().ceiling), (8, 32), "auto");

    // A number is a promise: start and top are the same number.
    crate::db::set_setting(&conn, "engine.default_streams", Some("4")).unwrap();
    assert_eq!((band().start, band().ceiling), (4, 4));
    crate::db::set_setting(&conn, "engine.default_streams", Some("64")).unwrap();
    assert_eq!(
        (band().start, band().ceiling),
        (32, 32),
        "a pinned band is still clamped to what the engine can run"
    );

    // Junk falls back to auto rather than to a stream count nobody asked for.
    for junk in ["0", "eight", ""] {
        crate::db::set_setting(&conn, "engine.default_streams", Some(junk)).unwrap();
        assert_eq!((band().start, band().ceiling), (8, 32), "{junk:?}");
    }
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}

/// `storage.preallocation` is the setting this file's own staging witnesses
/// depend on — a `.sucrpart` length only means anything once the mode that wrote
/// it is known — and until now nothing read the row on the product's side. So
/// both halves: the decision the key drives, then the bytes the filesystem holds
/// after the call that acts on it.
#[test]
fn the_stored_preallocation_key_decides_whether_the_length_is_reserved() {
    let dir = temp_dir("prealloc-key");
    let conn = crate::db::open(&dir.join("sucr.db")).unwrap();

    // Unset is the shipped default, `sparse`, and sparse reserves.
    assert!(super::reserves_length(&conn), "no row reserves");
    for on in ["sparse", "full"] {
        crate::db::set_setting(&conn, "storage.preallocation", Some(on)).unwrap();
        assert!(super::reserves_length(&conn), "{on} reserves");
    }

    // The one value that means "do not": the file grows with the data instead.
    crate::db::set_setting(&conn, "storage.preallocation", Some("none")).unwrap();
    assert!(!super::reserves_length(&conn), "none leaves the length to the writes");

    // Compared case-insensitively nowhere, and `settings_set` admits only the
    // three names — so anything else reserves, which is the safe direction: a
    // junk row cannot silently turn a reserved file into a growing one.
    for junk in ["", "NONE", "off"] {
        crate::db::set_setting(&conn, "storage.preallocation", Some(junk)).unwrap();
        assert!(super::reserves_length(&conn), "{junk:?} still reserves");
    }
    drop(conn);
    let _ = std::fs::remove_dir_all(dir);
}

/// The decision above is worth nothing if the call that acts on it does not make
/// the file look like the mode. This is the on-disk half, with no server and no
/// app: `reset_staging` runs and the length is read back from the directory.
#[test]
fn each_preallocation_mode_leaves_the_staging_file_the_mode_promises() {
    let total = 4 * 1024 * 1024;

    for (mode, want_len) in [("none", 0u64), ("sparse", total), ("full", total)] {
        let dir = temp_dir(&format!("prealloc-file-{mode}"));
        let conn = crate::db::open(&dir.join("sucr.db")).unwrap();
        crate::db::set_setting(&conn, "storage.preallocation", Some(mode)).unwrap();

        let path = super::reset_staging(&conn, 7, &dir.join("dl"), "piece.bin", Some(total))
            .expect("staging created");
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            want_len,
            "{mode} staging length"
        );
        assert!(path.to_string_lossy().ends_with(".7.sucrpart"), "{mode} name");

        drop(conn);
        let _ = std::fs::remove_dir_all(dir);
    }
    // `sparse` and `full` are the same length, which is exactly why this test
    // cannot tell them apart: only the sparse attribute and the zeros actually
    // written differ, and the attribute is set best-effort (FAT and shares
    // refuse it), so it is not a promise the product makes. Not claimed here.
}

#[test]
fn growth_needs_a_window_that_bought_bytes() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);

    assert_eq!(next_width(&healthy(1_000, 1_000, 8, 12), &mut s, &t), RampAction::Grow);
    // Half of a fair share at width 8 is +6.25%, so 1000 bytes bill 1063.
    assert_eq!(s.gain_bar, Some(1_063), "the growth bills the next window");

    // A ninth stream that brings its full proportional share (+12.5%) pays it.
    assert_eq!(next_width(&healthy(2_000, 1_125, 9, 12), &mut s, &t), RampAction::Grow);
    assert_eq!(s.gain_bar, Some(1_188), "and the next step is priced from width 9");

    // 1150 against that bill is a stream that carried 25 bytes while it owed 63:
    // the extra connection moved nothing worth having, so the ramp stops trying
    // for the rest of the job.
    assert_eq!(next_width(&healthy(3_000, 1_150, 10, 12), &mut s, &t), RampAction::Plateau);
    assert!(s.plateau);
    assert_eq!(
        next_width(&healthy(10_000, 100_000, 10, 12), &mut s, &t),
        RampAction::Hold,
        "a plateau is not renegotiated by a window that arrived later"
    );
}

#[test]
fn an_unfair_window_is_not_evidence() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);
    assert_eq!(next_width(&healthy(1_000, 1_000, 8, 12), &mut s, &t), RampAction::Grow);

    // Three ways a window says nothing about the host. None of them may spend
    // the job's one chance at a plateau.
    let mut quiet = healthy(2_000, 900, 9, 12);
    quiet.quiet = true;
    assert_eq!(next_width(&quiet, &mut s, &t), RampAction::Hold, "a silent stream");
    let mut paced = healthy(3_000, 900, 9, 12);
    paced.host_bytes = 0;
    assert_eq!(next_width(&paced, &mut s, &t), RampAction::Hold, "the limiter paced it");
    assert_eq!(
        next_width(&healthy(4_000, 0, 9, 12), &mut s, &t),
        RampAction::Hold,
        "nothing landed"
    );
    assert_eq!(s.gain_bar, Some(1_063), "the bar waits for a window it can trust");
    assert!(!s.plateau);

    // The first trustworthy window is judged on its merits.
    assert_eq!(next_width(&healthy(5_000, 500, 9, 12), &mut s, &t), RampAction::Plateau);
}

/// The shape the 2026-11-04 launched run measured in the shipped binary: a loaded
/// transfer leaves the coordinator ~3.4 s without a tick, so the first window swallows
/// the whole run and the tick 22 ms later carries a sliver. Under the old rule that
/// sliver sat in front of the window-age gate, answered the growth bill with 200 KB and
/// latched `plateau` for the rest of the job — which is why Auto stopped at 9 streams in
/// every one of the eight uncapped passes.
#[test]
fn a_window_22_ms_old_is_not_an_answer_to_the_growth_bill() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);

    // A first window of 3.4 s on a ~8.8 MB/s link: 30 MB, all of it the host's.
    let mut backlog = healthy(3_400, 30_000_000, 8, 12);
    backlog.span_ms = 3_400;
    assert_eq!(next_width(&backlog, &mut s, &t), RampAction::Grow);
    // step_bill(30 000 000, 8, 50) = 30 000 000 + 1 875 000 + 1.
    assert_eq!(s.gain_bar, Some(31_875_001), "the ask is priced from that window");
    assert_eq!(
        s.gain_bar_span_ms, 3_400,
        "and remembers how long a window it was earned over"
    );

    // The tick that drains 22 ms later is the backlog, not a measurement.
    let mut sliver = healthy(3_422, 200_000, 9, 12);
    sliver.span_ms = 22;
    assert_eq!(next_width(&sliver, &mut s, &t), RampAction::Hold, "22 ms is not a window");
    assert_eq!(s.gain_bar, Some(31_875_001), "the ask stands for one that is");
    assert!(!s.plateau, "and a sliver does not decide the job");

    // The window that does answer it is a full one at the rate the pricing asked for:
    // half a fair share at width 9 is +5.56 % over 8.8 MB/s, so 9.5 MB in 1 s clears it.
    // Its total is a third of the ask — only a rate comparison can see that it paid.
    let mut full = healthy(4_422, 9_500_000, 9, 12);
    full.span_ms = 1_000;
    assert_eq!(next_width(&full, &mut s, &t), RampAction::Grow, "the band climbs past 9");
    assert!(!s.plateau);
}

/// The guard must not become a licence. A full-length window carrying the rate the host
/// held before the growth — the ninth stream bought nothing — is still the plateau the
/// rule was written for, and a 22 ms window cannot be credited with disproving anything.
#[test]
fn a_full_window_at_the_old_rate_is_still_evidence_against_the_growth() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);
    let mut backlog = healthy(3_400, 30_000_000, 8, 12);
    backlog.span_ms = 3_400;
    assert_eq!(next_width(&backlog, &mut s, &t), RampAction::Grow);

    let mut sliver = healthy(3_422, 200_000, 9, 12);
    sliver.span_ms = 22;
    assert_eq!(next_width(&sliver, &mut s, &t), RampAction::Hold);

    let mut flat = healthy(4_422, 8_000_000, 9, 12);
    flat.span_ms = 1_000;
    assert_eq!(
        next_width(&flat, &mut s, &t),
        RampAction::Plateau,
        "8 MB/s is the width-8 rate: the extra stream carried nothing"
    );
    assert!(s.plateau);
}

/// A bond with one capped adapter and one uncapped: the window holds real evidence,
/// just not as much as its byte count claims. Both halves of the rule — the price and
/// the reading — have to move to host bytes together, or the ramp is merely stricter.
#[test]
fn a_partially_paced_window_is_judged_on_what_the_host_paid() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);

    // 1 000 bytes landed, 400 of them bought with a cap on the wire.
    let mut mixed = healthy(1_000, 1_000, 8, 12);
    mixed.host_bytes = 600;
    assert_eq!(next_width(&mixed, &mut s, &t), RampAction::Grow);
    // Priced from the host's 600, not the window's 1 000: step_bill(600, 8, 50).
    assert_eq!(s.gain_bar, Some(638), "the ask was priced off the paced total");

    // The next window carries the same total but clears the smaller ask. Judged on
    // bytes_delta it would have had to beat the old bar of 1 063, so the growth step
    // after an adapter cap would be the one a host-only window could never afford.
    let mut paid = healthy(2_000, 1_000, 9, 12);
    paid.host_bytes = 638;
    assert_eq!(next_width(&paid, &mut s, &t), RampAction::Grow, "638 host bytes is the whole ask");

    // And the other half of the same rule: a window that looked busy only because a
    // limiter was spending bytes gets plateaued, not credited for the pace.
    let mut u = RampState::new(8, 32, 0);
    let mut first = healthy(1_000, 1_000, 8, 12);
    first.host_bytes = 600;
    assert_eq!(next_width(&first, &mut u, &t), RampAction::Grow);
    let mut thin = healthy(2_000, 1_000, 9, 12);
    thin.host_bytes = 637;
    assert_eq!(next_width(&thin, &mut u, &t), RampAction::Plateau, "one byte short of the ask");
}

#[test]
fn a_paced_window_cannot_climb_the_ceiling_either() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);
    // Narrowed by a refusal at width 8; both recovery clocks restart with it.
    s.note_refusal(8, 0);
    assert_eq!((s.ceiling, s.hard), (7, 32));

    // Sixty seconds on the recovery clock, but every byte in the window left under an
    // adapter cap. Recovery is evidence about the host too, so a paced window may not
    // spend it — the top of the band stays where the refusal put it.
    let mut paced = healthy(61_000, 100_000, 7, 12);
    paced.host_bytes = 0;
    assert_eq!(next_width(&paced, &mut s, &t), RampAction::Hold);
    assert_eq!(s.ceiling, 7, "a paced window proved nothing about the host");

    assert_eq!(
        next_width(&healthy(62_000, 1_000, 7, 12), &mut s, &t),
        RampAction::Raise,
        "the clock had already run out; only the bytes were missing"
    );
}

/// Step 3b takes an adapter out of the bond while the job runs. The bytes that land
/// afterwards are a smaller number measured against a narrower bond, so answering the
/// old ask with them would plateau a job that has done nothing wrong.
#[test]
fn a_bond_that_shifted_voids_the_bar_instead_of_plateauing() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);
    assert_eq!(next_width(&healthy(1_000, 1_000, 8, 12), &mut s, &t), RampAction::Grow);
    assert_eq!(s.gain_bar, Some(1_063));

    let mut shifted = healthy(2_000, 200, 9, 12);
    shifted.shifted = true;
    assert_eq!(next_width(&shifted, &mut s, &t), RampAction::Hold, "two bonds, one window");
    assert_eq!(s.gain_bar, None, "an ask priced from a bond that no longer exists is voided");
    assert!(!s.plateau, "and voiding it is not the same as disproving it");

    // The first window measured against the new, narrower bond is judged fresh — and
    // grows, because 200 bytes at width 9 is what a nine-stream job actually pays.
    assert_eq!(next_width(&healthy(3_000, 200, 9, 12), &mut s, &t), RampAction::Grow);

    // Without the shift the identical window is the plateau it always was, so the
    // above is the ejection being forgiven and not the rule having gone soft.
    let mut u = RampState::new(8, 32, 0);
    assert_eq!(next_width(&healthy(1_000, 1_000, 8, 12), &mut u, &t), RampAction::Grow);
    assert_eq!(next_width(&healthy(2_000, 200, 9, 12), &mut u, &t), RampAction::Plateau);
}

#[test]
fn growth_stops_where_the_work_or_the_ceiling_does() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);

    // Every block claimed and no tail to steal: a ninth stream would idle.
    assert_eq!(next_width(&healthy(1_000, 1_000, 8, 0), &mut s, &t), RampAction::Hold);
    // Four blocks left is eight streams of work, and eight already run.
    assert_eq!(next_width(&healthy(1_000, 1_000, 8, 4), &mut s, &t), RampAction::Hold);
    // A straggler whose tail can be split off is work, so the same window grows.
    let mut with_tail = healthy(1_000, 1_000, 8, 4);
    with_tail.stealable = true;
    assert_eq!(next_width(&with_tail, &mut s, &t), RampAction::Grow);

    // A file that only supports nine workers never asks for a tenth.
    let mut small = RampState::new(8, 9, 0);
    assert_eq!(next_width(&healthy(1_000, 1_000, 8, 12), &mut small, &t), RampAction::Grow);
    assert_eq!(next_width(&healthy(2_000, 2_000, 9, 12), &mut small, &t), RampAction::Hold);
    assert_eq!((small.ceiling, small.hard), (9, 9));

    // Cadence: a second window inside the first one's span is not a measurement.
    let mut early = RampState::new(8, 32, 0);
    assert_eq!(next_width(&healthy(500, 4_000, 8, 12), &mut early, &t), RampAction::Hold);
    assert_eq!(early.gain_bar, None, "a window nobody measured cannot owe a bill");
}

#[test]
fn a_refusal_drops_the_ceiling_and_pins_the_recovery_clock() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);
    s.note_refusal(16, 1_000);
    assert_eq!(s.ceiling, 15, "just under the width the host objected to");
    assert_eq!(s.gain_bar, None, "the window a halving produced proves nothing");

    // A minute of quiet buys one step back, not a flood.
    assert_eq!(next_width(&healthy(2_000, 5_000, 8, 12), &mut s, &t), RampAction::Hold);
    assert_eq!(next_width(&healthy(61_000, 5_000, 8, 12), &mut s, &t), RampAction::Raise);
    assert_eq!(s.ceiling, 16);
    assert_eq!(next_width(&healthy(62_000, 6_000, 8, 12), &mut s, &t), RampAction::Hold);

    // Left alone, it climbs one at a time and stops at the band's edge.
    let mut now = 62_000i64;
    let mut bytes = 6_000u64;
    for _ in 0..40 {
        now += 61_000;
        bytes += 2_000;
        next_width(&healthy(now, bytes, 8, 12), &mut s, &t);
    }
    assert_eq!(s.ceiling, 32, "recovery never passes the hard top");

    // A pinned band is a promise: the ceiling is not the ramp's to spend.
    let mut pinned = RampState::new(4, 4, 0);
    pinned.note_refusal(4, 1_000);
    assert_eq!(pinned.ceiling, 4, "a dial set to 4 stays 4");
}

#[test]
fn the_bill_shrinks_with_the_step_it_prices() {
    // Growth is additive, so the gain one extra stream can show shrinks with the
    // width: `w -> w+1` is worth (w+1)/w. A bill that does not shrink with it is
    // unreachable, and that is the bug these numbers pin: with the ask flat at
    // +15% at every width, a real 96 MiB job grew exactly once — to 9 — and
    // plateaued there while every stream was still paying in full.
    for (width, fair_pct) in [(8usize, 12.5f64), (16, 6.25), (31, 3.23)] {
        let bytes = 1_000_000 * width as u64;
        let asked = (step_bill(bytes, width, 50) - bytes - 1) as f64 * 100.0 / bytes as f64;
        assert!(
            asked < fair_pct,
            "width {width}: the bill asks {asked}% but a fair host only brings {fair_pct}%"
        );
    }
    // Half a share, spelled out at the width that failed in the shipped app: 16
    // MiB in a window bills 17 MiB, and nine streams at the same pace bring 18.
    assert_eq!(step_bill(16 * 1024 * 1024, 8, 50), 17 * 1024 * 1024 + 1);
    // A stream that landed not one extra byte owes a bill even with the ask
    // turned off, so `share_pct: 0` cannot be read as "growth is free".
    assert_eq!(step_bill(1_000, 8, 0), 1_001);
}

#[test]
fn a_fair_host_climbs_the_whole_ladder() {
    let t = clocks();
    // Every stream earns the same rate, so each window shows (w+1)/w of the last:
    // the best case a host can offer, and the one the flat bar used to refuse.
    let mut s = RampState::new(8, 32, 0);
    let mut now = 0i64;
    let mut width = 8usize;
    let mut steps = 0;
    while width < 32 {
        now += 1_000;
        let sample = healthy(now, 2_000_000 * width as u64, width, 40);
        match next_width(&sample, &mut s, &t) {
            RampAction::Grow => {
                width += 1;
                steps += 1;
            }
            other => panic!("width {width} got {other:?} from a host paying per stream"),
        }
    }
    assert_eq!(steps, 24, "8 to 32, one stream per window");
    assert_eq!(now, 24_000, "and no window wasted");
    assert!(!s.plateau, "a fair host never leaves a bill unpaid");
}

#[test]
fn a_host_that_adds_no_bytes_plateaus_at_the_first_step() {
    let t = clocks();
    let mut s = RampState::new(8, 32, 0);
    // The per-client cap: the total is the same however many streams run, which
    // is the host Plexo's "everything answered cleanly" test cannot tell apart
    // from a fast one. Here it costs one extra socket, not twenty-four.
    assert_eq!(next_width(&healthy(1_000, 5_000_000, 8, 40), &mut s, &t), RampAction::Grow);
    assert_eq!(
        next_width(&healthy(2_000, 5_000_000, 9, 40), &mut s, &t),
        RampAction::Plateau,
        "the ninth stream carried nothing"
    );
    assert!(s.plateau);
}

#[test]
fn an_auto_job_opens_more_streams_when_the_bytes_ask_for_them() {
    // 12 blocks of 1 MiB against a server that paces each connection at 1 MiB/s:
    // more streams is more bytes per second, so a growing window is real
    // evidence rather than a race. The file has to be this big for the plan to
    // support more than the opening eight workers at all.
    let size = 12 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("ramp-auto", &chaos.url("?bps=1048576"), Some(1024 * 1024), None);
    let ctx = RunContext { ramp: fast_ramp(), ..fx.ctx() };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("auto-band download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size), "a wider bond must not lose a byte");

    let final_row = &fx.history_json("ramp-final")[0];
    assert_eq!(
        final_row["hard"].as_i64(),
        Some(12),
        "the file, not the setting, is the ceiling"
    );
    assert_eq!(final_row["start"].as_i64(), Some(8));
    let workers = final_row["workers"].as_i64().unwrap();
    assert!(workers > 8 && workers <= 12, "the ramp never opened a stream: {final_row}");

    let grows = fx.history_json("ramp-grow");
    assert!(
        grows.iter().any(|g| g["to"].as_i64() == Some(9)),
        "the first step is 8 to 9, whatever the run did after: {grows:?}"
    );

    let mut stmt = fx
        .conn
        .prepare("SELECT state FROM blocks WHERE download_id = ?1 ORDER BY idx")
        .unwrap();
    let states: Vec<String> = stmt
        .query_map([fx.id], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(states.len(), 12);
    assert!(states.iter().all(|s| s == "done"), "{states:?}");
}

#[test]
fn the_live_width_is_the_width_that_ran() {
    // The card and the inspector read the width off the live job, so the number
    // a reader sees has to agree with the one the supervisor recorded.
    let size = 12 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("live-width", &chaos.url("?bps=1048576"), Some(1024 * 1024), None);
    let live = Arc::new(super::live::Live::new());
    live.watch();
    let ctx = RunContext {
        live: Some(Arc::clone(&live)),
        ramp: fast_ramp(),
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("auto-band download");

    let final_row = &fx.history_json("ramp-final")[0];
    let width = final_row["width"].as_i64().unwrap();
    assert!(width > 8, "the ramp never opened a stream: {final_row}");
    assert_eq!(
        live.streams() as i64,
        width,
        "the reader's number and the recorded one have to be the same reading"
    );
}

#[test]
fn a_pinned_job_publishes_the_number_it_was_given() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("live-width-pinned", &chaos.url(""), Some(1024 * 1024), Some(3));
    let live = Arc::new(super::live::Live::new());
    live.watch();
    let ctx = RunContext {
        live: Some(Arc::clone(&live)),
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("pinned download");

    assert_eq!(live.streams(), 3, "a pinned band publishes its pin");
    assert!(
        fx.history_json("ramp-grow").is_empty(),
        "and never moves it"
    );
}

/// The bug step 4 exists for, on a real transfer: an adapter cap paces the bytes, and
/// until now the window's total was read as though the server had handed them over. A
/// job whose every route is capped therefore has to stop growing — the same way one
/// under a job-wide cap already did — while still moving the whole file.
#[test]
fn a_capped_bond_offers_the_ramp_no_host_evidence() {
    let size = 12 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("ramp-capped-bond", &chaos.url("?bps=1048576"), Some(1024 * 1024), None);
    let (wired, wifi) = note_pair(&fx);
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };
    // Generous caps: the point is not that the link is slow, it is that no byte in the
    // window is the server's answer, so none of them is evidence about the host.
    let a = Arc::new(super::NetScope::new(Some(8 * 1024 * 1024)));
    let b = Arc::new(super::NetScope::new(Some(8 * 1024 * 1024)));

    let ctx = RunContext {
        routes: vec![
            route(wired.id, "n1", "Ethernet", "127.0.0.1"),
            route(wifi.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        networks: scope_map(&[(wired.id, Arc::clone(&a)), (wifi.id, Arc::clone(&b))]),
        ramp: fast_ramp(),
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("capped bond download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size), "a ramp that will not grow still downloads");
    let grows = fx.history_json("ramp-grow");
    assert!(
        grows.is_empty(),
        "paced bytes were booked as host evidence and the ramp widened anyway: {grows:?}"
    );
    let final_row = &fx.history_json("ramp-final")[0];
    assert_eq!(
        final_row["start"].as_i64(),
        Some(8),
        "and the band it opened on is the band it finished on: {final_row}"
    );
    assert_eq!(final_row["width"].as_i64(), Some(8));
}

/// The other half: one adapter capped must not blind the ramp to the adapter beside
/// it. This is the guard against attributing too much — a rule that froze the band
/// whenever any cap existed anywhere would pass the test above and fail this one.
#[test]
fn a_capped_adapter_does_not_silence_the_route_beside_it() {
    let size = 12 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("ramp-half-capped", &chaos.url("?bps=1048576"), Some(1024 * 1024), None);
    let (wired, wifi) = note_pair(&fx);
    let route = |id: i64, color: &str, label: &str, ip: &str| super::Route {
        network_id: Some(id),
        color: color.to_string(),
        label: label.to_string(),
        bind: Some(ip.parse().unwrap()),
    };
    let capped = Arc::new(super::NetScope::new(Some(8 * 1024 * 1024)));
    let free = Arc::new(super::NetScope::new(None));

    let ctx = RunContext {
        routes: vec![
            route(wired.id, "n1", "Ethernet", "127.0.0.1"),
            route(wifi.id, "n2", "Wi-Fi", "127.0.0.2"),
        ],
        networks: scope_map(&[(wired.id, Arc::clone(&capped)), (wifi.id, Arc::clone(&free))]),
        ramp: fast_ramp(),
        ..fx.ctx()
    };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("half-capped bond download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));

    let grows = fx.history_json("ramp-grow");
    assert!(
        grows.iter().any(|g| g["to"].as_i64() == Some(9)),
        "the uncapped adapter carried bytes all along, so the first step had to be taken: {grows:?}"
    );
    for g in &grows {
        let bytes = g["bytes"].as_i64().unwrap();
        let host = g["host"].as_i64().unwrap();
        assert!(host > 0, "a window with no host bytes cannot grow: {g}");
        assert!(host <= bytes, "more host bytes credited than landed in the window: {g}");
    }
}

#[test]
fn a_closed_row_keeps_the_width_the_run_ended_at() {
    // When the job ends the registry goes with it, so a card that wants to say
    // more than the dial promised has to read a number the run left on the row.
    let size = 12 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("final-width", &chaos.url("?bps=1048576"), Some(1024 * 1024), None);
    let ctx = RunContext { ramp: fast_ramp(), ..fx.ctx() };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("auto-band download");

    let recorded = fx.history_json("ramp-final")[0]["width"].as_i64().unwrap();
    assert!(recorded > 8, "the ramp never opened a stream: {recorded}");
    assert_eq!(
        fx.num_column("streams_final"),
        Some(recorded),
        "the row and the activity trail have to be one reading, not two"
    );
}

#[test]
fn a_run_that_met_a_rate_limit_leaves_its_width_on_the_row() {
    let size = 12 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let url = chaos.url("?throttle=2&after=1&bps=1048576");
    let fx = Fixture::new("final-width-narrowed", &url, Some(1024 * 1024), Some(8));

    run(fx.ctx(), fx.id, Arc::new(AtomicU8::new(RUN))).expect("pinned download after refusals");

    let refusals = fx.history_json("throttled");
    assert!(!refusals.is_empty(), "the fixture refused nothing: {refusals:?}");
    assert!(
        refusals.iter().any(|t| t["streams"].as_i64() == Some(4)),
        "a refusal halves the gate below the pin: {refusals:?}"
    );
    let width = fx.history_json("ramp-final")[0]["width"].as_i64().unwrap();
    assert_eq!(
        fx.num_column("streams_final"),
        Some(width),
        "the row keeps whatever the last window was running"
    );
    assert!(width <= 8, "a pinned run may not end above the number it was given: {width}");
}

#[test]
fn a_refusal_takes_the_ceiling_down_with_it() {
    let size = 12 * 1024 * 1024;
    let chaos = Chaos::start(size);
    // Two refusals, each with a second to wait, on a paced server so the job
    // outlives both and the ceiling they leave behind is observable.
    let url = chaos.url("?throttle=2&after=1&bps=1048576");
    let fx = Fixture::new("ramp-refusal", &url, Some(1024 * 1024), None);
    let ctx = RunContext { ramp: fast_ramp(), ..fx.ctx() };

    run(ctx, fx.id, Arc::new(AtomicU8::new(RUN))).expect("download after a refusal");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));

    let refusals = fx.history_json("throttled");
    assert_eq!(refusals.len(), 2, "{refusals:?}");
    assert_eq!(refusals[0]["previous"].as_i64(), Some(8));
    assert_eq!(
        refusals[0]["ceiling"].as_i64(),
        Some(7),
        "one under the width that was objected to"
    );
    assert_eq!(refusals[1]["previous"].as_i64(), Some(4));
    assert_eq!(refusals[1]["ceiling"].as_i64(), Some(3), "the second refusal is not forgiven");

    let final_row = &fx.history_json("ramp-final")[0];
    assert_eq!(final_row["ceiling"].as_i64(), Some(3));
    assert_eq!(
        final_row["workers"].as_i64(),
        Some(8),
        "a lowered ceiling may not open a new stream"
    );
    // The 60s recovery is left at its real length on purpose: a test that spent
    // a minute waiting proves nothing about the rule.
    let width = final_row["width"].as_i64().unwrap();
    assert!(width <= 3, "the width crept past what the host agreed to: {final_row}");
}

#[test]
fn a_pinned_job_never_opens_more_streams_than_the_number() {
    let size = 4 * 1024 * 1024;
    let chaos = Chaos::start(size);
    let fx = Fixture::new("ramp-pinned", &chaos.url("?bps=1048576"), Some(512 * 1024), Some(4));

    fx.run().expect("pinned download");

    assert_eq!(fx.status(), "finished");
    assert_eq!(fx.published(), body(size));

    let kinds = fx.history();
    for kind in ["ramp-grow", "ramp-raise", "ramp-plateau"] {
        assert!(!kinds.iter().any(|k| k == kind), "{kind} on a pinned band: {kinds:?}");
    }
    let final_row = &fx.history_json("ramp-final")[0];
    assert_eq!(final_row["workers"], 4, "the dial is a promise");
    assert_eq!(final_row["hard"], 4);
    assert_eq!(final_row["ceiling"], 4);
}

/// How the 32-stream arm of the spike presents a per-adapter bucket: absent,
/// present but unlimited, or present with the cap that the job arm puts on its
/// own bucket.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NetArm {
    Absent,
    Open,
    Capped(u64),
}

/// Step 1 of the per-network limits: what does a third bucket in the slice cost?
/// Ignored because it measures instead of asserting — a timing assertion would
/// fail on a busier machine for the wrong reason. Run it with
/// `cargo test --lib -- --ignored --nocapture --test-threads=1`, and add
/// `--release` to price the codegen that actually ships — measured 2026-11-03:
/// `panic = "abort"` in `[profile.release]` does not stop the test harness from
/// building under `--release`. The two profiles differ by an order of magnitude
/// on (a), so the printed label says which one produced the numbers.
///
/// Both halves answer the same question at different distances: (a) prices the
/// bookkeeping alone, (b) prices it inside 32 real streams over a real socket.
#[test]
#[ignore]
fn the_third_bucket_is_priced_before_anything_uses_it() {
    use super::fetch_block;
    use super::limit::{self, charge, read_slice};
    use std::fs::OpenOptions;

    const STREAMS: usize = 32;
    const SIZE: usize = 4 * 1024 * 1024;
    const CAP: u64 = 2 * 1024 * 1024;
    const CHARGES: usize = 200_000;
    const REPS: usize = 5;
    const RUNS_ARMS: usize = 3;

    let cancel = Arc::new(AtomicU8::new(RUN));

    // (a) One read_slice and one charge per round, with the charge at one byte:
    // a 1-byte reserve cannot make a bucket pace, so the sleep stays out of the
    // clock and only the locking and arithmetic are measured.
    let book = |buckets: &[&Bucket]| -> f64 {
        let t = Instant::now();
        for _ in 0..CHARGES {
            let _ = read_slice(buckets);
            charge(buckets, 1, &cancel).unwrap();
        }
        t.elapsed().as_nanos() as f64 / CHARGES as f64
    };

    let job = Bucket::new(Some(CAP));
    let global = Bucket::new(None);
    let net_capped = Bucket::new(Some(CAP));
    let net_open = Bucket::new(None);

    // One pass each would rank the shapes by which ran first, so every shape is
    // measured REPS times in the same rotation. The spread printed beside each
    // number is this box's noise floor: a delta inside it is not a finding.
    // Four buckets brackets the two three-bucket shapes, so a difference the
    // medians cannot resolve still comes out with a size.
    let mut samples = [[0.0f64; REPS]; 4];
    for rep in 0..REPS {
        samples[0][rep] = book(&[&job, &global]);
        samples[1][rep] = book(&[&job, &global, &net_capped]);
        samples[2][rep] = book(&[&job, &global, &net_open]);
        samples[3][rep] = book(&[&job, &global, &net_capped, &net_open]);
    }
    let order = |row: &[f64]| {
        let mut row = row.to_vec();
        row.sort_by(|a, b| a.partial_cmp(b).unwrap());
        row
    };
    let median = |row: &[f64]| order(row)[REPS / 2];
    let spread = |row: &[f64]| {
        let row = order(row);
        row[REPS - 1] - row[0]
    };

    let two = median(&samples[0]);
    // A cap charges once per slice, and a slice is a twentieth of a second's
    // bytes, so 20 rounds a second per stream is the ceiling on this bookkeeping.
    let per_second = CAP / limit::slice_of(CAP) as u64;
    println!(
        "(a) per read_slice + charge, {} codegen, {REPS} passes of {CHARGES}:",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    for (label, row) in [
        ("two buckets, one capped   ", &samples[0]),
        ("three, the extra capped   ", &samples[1]),
        ("three, the extra unlimited", &samples[2]),
        ("four, one of each         ", &samples[3]),
    ] {
        println!(
            "    {label} {:7.1} ns  (+{:6.1})  spread {:6.1}",
            median(row),
            median(row) - two,
            spread(row)
        );
    }
    println!(
        "    the marginal cap        {:7.1} ns  (four minus the open three)",
        median(&samples[3]) - median(&samples[2])
    );
    println!(
        "    the marginal open bucket{:7.1} ns  (four minus the capped three)",
        median(&samples[3]) - median(&samples[1])
    );
    let marginal = (median(&samples[3]) - two) / 2.0;
    println!(
        "    one more bucket is {marginal:.0} ns per round, so {per_second} rounds/s cost \
         {:.1} ms of CPU per stream-hour",
        marginal * per_second as f64 * 3600.0 / 1e6
    );

    // (b) The same question with a socket in the way. The two capped arms apply
    // one identical cap through different slice shapes, so a difference between
    // them is the third bucket rather than the limit.
    let chaos = Chaos::start(SIZE);
    let url = chaos.url("");
    let agent = super::Route::default_route().agent_with(None);
    let dir = temp_dir("third-bucket");
    let path = dir.join("body.bin");
    let expected = sha256_hex(&body(SIZE));
    let block_len = (SIZE / STREAMS) as u64;
    let blocks: Vec<BlockSpec> = (0..STREAMS)
        .map(|i| BlockSpec {
            idx: i,
            start: i as u64 * block_len,
            end: (i as u64 + 1) * block_len - 1,
        })
        .collect();

    let run_arm = |cap_job: Option<u64>, net: NetArm| -> f64 {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.set_len(SIZE as u64).unwrap();
        let job = Bucket::new(cap_job);
        let global = Bucket::new(None);
        let bucket = match net {
            NetArm::Absent => None,
            NetArm::Open => Some(Bucket::new(None)),
            NetArm::Capped(cap) => Some(Bucket::new(Some(cap))),
        };
        let t = Instant::now();
        std::thread::scope(|s| {
            for block in &blocks {
                s.spawn(|| {
                    let mut offset = 0u64;
                    let mut report = |_delta: u64| {};
                    fetch_block(
                        &agent,
                        &url,
                        Some(chaos.etag.as_str()),
                        "",
                        None,
                        *block,
                        &mut offset,
                        None,
                        &file,
                        &cancel,
                        &job,
                        &global,
                        bucket.as_ref(),
                        &mut report,
                    )
                    .unwrap_or_else(|_| panic!("a stream failed its block"));
                });
            }
        });
        let elapsed = t.elapsed().as_secs_f64();
        assert_eq!(
            sha256_hex(&std::fs::read(&path).unwrap()),
            expected,
            "job={cap_job:?} net={net:?} lost or gained bytes"
        );
        SIZE as f64 / elapsed / 1024.0 / 1024.0
    };

    let arms = [
        ("no cap, two buckets", None, NetArm::Absent),
        ("no cap, three buckets", None, NetArm::Open),
        ("cap on the job bucket, two buckets", Some(CAP), NetArm::Absent),
        ("the same cap on the net bucket, three", None, NetArm::Capped(CAP)),
        ("cap on the job, open net bucket", Some(CAP), NetArm::Open),
    ];
    println!("(b) {STREAMS} streams over {SIZE} bytes, MiB/s over {RUNS_ARMS} runs:");
    for (label, cap_job, net) in arms {
        let mut rates = [0.0f64; RUNS_ARMS];
        for rate in rates.iter_mut() {
            *rate = run_arm(cap_job, net);
        }
        rates.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "    {label:38} min {:.2}  median {:.2}  max {:.2}",
            rates[0],
            rates[RUNS_ARMS / 2],
            rates[RUNS_ARMS - 1]
        );
    }
}

#[test]
fn ride_prefers_the_row_and_falls_through_an_empty_value() {
    // A captured row carries the browser's live identity; it must win over the
    // host-wide session, because an attested stream is only replayable by the
    // exact client that fetched it.
    assert_eq!(super::ride(Some("browser UA"), Some("host UA")), Some("browser UA"));
    assert_eq!(super::ride(Some("a=1"), Some("b=2")), Some("a=1"));
    // No saved ride value leaves the host session untouched.
    assert_eq!(super::ride(None, Some("host UA")), Some("host UA"));
    assert_eq!(super::ride(None, None), None);
    // An empty or whitespace-only column is not a ride: it must fall through
    // rather than blank the header an existing session would have sent.
    assert_eq!(super::ride(Some(""), Some("host UA")), Some("host UA"));
    assert_eq!(super::ride(Some("   "), Some("b=2")), Some("b=2"));
    // A padded row value is trimmed before it is used.
    assert_eq!(super::ride(Some("  browser UA  "), Some("host UA")), Some("browser UA"));
}

// --- one Take, one job ------------------------------------------------------

/// A fixed-bytes media server: the chaos server generates synthetic bodies, but
/// a joined file has to start from real fragmented media, so these tests serve
/// the muxer fixtures over their own loopback port.
struct MediaServer {
    base: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MediaServer {
    fn start(files: &[(&str, Vec<u8>)]) -> MediaServer {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let files: Arc<Vec<(String, Vec<u8>)>> =
            Arc::new(files.iter().map(|(p, b)| (p.to_string(), b.clone())).collect());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut sock, _)) => {
                        let files = Arc::clone(&files);
                        std::thread::spawn(move || {
                            sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
                            let mut head = Vec::new();
                            let mut buf = [0u8; 4096];
                            while head.len() <= 16 * 1024 {
                                match sock.read(&mut buf) {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => {
                                        head.extend_from_slice(&buf[..n]);
                                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                                            break;
                                        }
                                    }
                                }
                            }
                            let text = String::from_utf8_lossy(&head).into_owned();
                            let mut lines = text.lines();
                            let target = lines
                                .next()
                                .and_then(|l| l.split_whitespace().nth(1))
                                .unwrap_or("/")
                                .to_string();
                            let path = target.split('?').next().unwrap_or("/").to_string();
                            let mut range: Option<(u64, Option<u64>)> = None;
                            for line in lines {
                                let Some((k, v)) = line.split_once(':') else { continue };
                                if !k.eq_ignore_ascii_case("range") {
                                    continue;
                                }
                                let Some(spec) = v.trim().strip_prefix("bytes=") else {
                                    continue;
                                };
                                let (a, b) = spec.split_once('-').unwrap_or((spec, ""));
                                range =
                                    Some((a.trim().parse().unwrap_or(0), b.trim().parse().ok()));
                            }
                            let Some((_, body)) = files.iter().find(|(p, _)| *p == path) else {
                                let _ = sock.write_all(
                                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                );
                                return;
                            };
                            let total = body.len() as u64;
                            let (status, first, last) = match range {
                                Some((a, _)) if a >= total => (416u16, 0u64, 0u64),
                                Some((a, e)) => {
                                    let end = e.unwrap_or(total - 1).min(total - 1);
                                    (206, a, end)
                                }
                                None => (200, 0, total - 1),
                            };
                            let n = if status == 416 { 0 } else { last - first + 1 };
                            let mut out = format!(
                                "HTTP/1.1 {status} {}\r\nContent-Length: {n}\r\nAccept-Ranges: bytes\r\n\
                                 ETag: \"media{path}\"\r\nLast-Modified: Wed, 22 Nov 2023 00:53:20 GMT\r\n\
                                 Content-Type: application/octet-stream\r\nConnection: close\r\n",
                                if status == 206 { "Partial Content" } else { "OK" },
                            );
                            if status == 416 {
                                out.push_str(&format!("Content-Range: bytes */{total}\r\n"));
                            } else if status == 206 {
                                out.push_str(&format!(
                                    "Content-Range: bytes {first}-{last}/{total}\r\n"
                                ));
                            }
                            out.push_str("\r\n");
                            let _ = sock.write_all(out.as_bytes());
                            if status != 416 {
                                let _ = sock.write_all(&body[first as usize..=last as usize]);
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        MediaServer { base, stop, thread: Some(thread) }
    }

    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

impl Drop for MediaServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[test]
fn one_row_takes_a_dash_pair_and_publishes_the_joined_file() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data");
    let picture = std::fs::read(data.join("v_multi.mp4")).unwrap();
    let sound = std::fs::read(data.join("a_multi.mp4")).unwrap();
    let media = MediaServer::start(&[("/v", picture.clone()), ("/a", sound.clone())]);

    let fx = Fixture::new("combined", &media.at("/v"), Some(64 * 1024), Some(2));
    fx.conn
        .execute(
            "UPDATE downloads SET filename = 'movie.mp4', audio_url = ?1 WHERE id = ?2",
            params![media.at("/a"), fx.id],
        )
        .unwrap();

    fx.run().expect("the pair is fetched and joined");

    assert_eq!(fx.status(), "finished");
    let merged = std::fs::read(fx.dir.join("dl").join("movie.mp4")).expect("one finished file");
    assert_eq!(&merged[4..8], b"ftyp", "the joined file opens as MP4");
    assert!(merged.windows(4).any(|w| w == b"vide"), "picture track landed");
    assert!(merged.windows(4).any(|w| w == b"soun"), "sound track landed");

    // One file and nothing else: both legs' parts are gone, and the sound never
    // became a row of its own.
    let mut names: Vec<String> = std::fs::read_dir(fx.dir.join("dl"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["movie.mp4".to_string()], "one file, no parts");
    let rows: i64 = fx
        .conn
        .query_row("SELECT COUNT(*) FROM downloads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "one Take is one row in the library");

    // The row describes the file that exists, not either stream.
    assert_eq!(fx.num_column("total_bytes"), Some(merged.len() as i64));
    assert_eq!(fx.num_column("verified_bytes"), Some(merged.len() as i64));
    assert_eq!(fx.column("staging_path"), None);
    assert_eq!(fx.column("part_path"), None);
    assert!(fx.history().iter().any(|k| k == "pair-picture"));
    assert!(fx.history().iter().any(|k| k == "merged"));
}

#[test]
fn a_failed_sound_leg_keeps_the_picture_for_the_next_run() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data");
    let picture = std::fs::read(data.join("v_multi.mp4")).unwrap();
    let sound = std::fs::read(data.join("a_multi.mp4")).unwrap();
    let media = MediaServer::start(&[("/v", picture), ("/a", sound)]);

    let fx = Fixture::new("combined-resume", &media.at("/v"), Some(64 * 1024), Some(2));
    fx.conn
        .execute(
            "UPDATE downloads SET filename = 'movie.mp4', audio_url = 'http://127.0.0.1:1/a' \
             WHERE id = ?1",
            [fx.id],
        )
        .unwrap();

    fx.run().expect_err("a sound leg with no server cannot finish the row");
    let part = fx.column("part_path").expect("the picture is kept while it is unjoined");
    assert!(Path::new(&part).is_file(), "the kept picture is on disk: {part}");

    // The next run re-points the sound and must resume, not re-fetch the picture.
    fx.conn
        .execute(
            "UPDATE downloads SET audio_url = ?1 WHERE id = ?2",
            params![media.at("/a"), fx.id],
        )
        .unwrap();
    fx.run().expect("the sound leg finishes on the second run");

    assert_eq!(fx.status(), "finished");
    let merged = std::fs::read(fx.dir.join("dl").join("movie.mp4")).expect("one finished file");
    assert!(merged.windows(4).any(|w| w == b"soun"), "sound track landed");
    assert!(!Path::new(&part).exists(), "the kept picture is consumed by the join");
    assert_eq!(
        fx.history_json("pair-picture").len(),
        1,
        "the picture is fetched once, not again on the resume"
    );
}

#[test]
#[ignore = "requires an external local benchmark server; run by hand"]
fn external_speed_benchmark() {
    let url = std::env::var("SUCR_BENCH_URL").expect("SUCR_BENCH_URL is required");
    assert!(
        url.starts_with("http://127.0.0.1:"),
        "SUCR_BENCH_URL must start with http://127.0.0.1:"
    );
    let size: usize = std::env::var("SUCR_BENCH_SIZE")
        .expect("SUCR_BENCH_SIZE is required")
        .parse()
        .expect("SUCR_BENCH_SIZE must be a byte count");
    let streams: u64 = std::env::var("SUCR_BENCH_STREAMS")
        .expect("SUCR_BENCH_STREAMS is required")
        .parse()
        .expect("SUCR_BENCH_STREAMS must be an integer");
    let block: Option<u64> = match std::env::var("SUCR_BENCH_BLOCK") {
        Ok(value) => Some(value.parse().expect("SUCR_BENCH_BLOCK must be a byte count")),
        Err(std::env::VarError::NotPresent) => None,
        Err(err) => panic!("invalid SUCR_BENCH_BLOCK: {err}"),
    };
    assert!(size > 0, "SUCR_BENCH_SIZE must be positive");
    assert!(streams > 0 && streams <= i64::MAX as u64, "invalid SUCR_BENCH_STREAMS");
    if let Some(block) = block {
        assert!(block > 0 && block <= i64::MAX as u64, "invalid SUCR_BENCH_BLOCK");
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let fx = Fixture::new(&format!("external-speed-{stamp}"), &url, block, Some(streams));

    let started = Instant::now();
    let result = fx.run();
    let elapsed = started.elapsed().as_secs_f64();

    result.expect("external benchmark download");
    assert_eq!(fx.status(), "finished");
    assert!(fx.published() == body(size), "benchmark payload must match every byte");
    println!(
        "SUCR_BENCH_JSON {}",
        serde_json::json!({
            "elapsed_seconds": elapsed,
            "size": size,
            "streams": streams,
            "block": block,
            "validation": true,
        })
    );
}
