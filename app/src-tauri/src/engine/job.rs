// The coordinator for one download: probe, plan, worker threads, progress
// persistence, single-stream fallback and publish. Workers own their HTTP
// responses; only the coordinator touches SQLite, so writes never contend.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde_json::json;

use super::{probe, plan_blocks, CHUNK, DEFAULT_BLOCK, DEFAULT_STREAMS, MAX_STREAMS};
use super::{adaptive_block, retry_after_secs};
use super::{rename_with_retry, unique_path, write_zone_identifier, JobError, Probe};
use super::bond::Route;
use super::limit::{self, Bucket};

pub mod live;
use live::{Live, LiveAttempt, LiveBlock, UNATTRIBUTED};

pub const RUN: u8 = 0;
pub const PAUSE: u8 = 1;
pub const STOP: u8 = 2;
pub const FAIL: u8 = 3;
pub const RESTART: u8 = 4;

const MAX_ATTEMPTS: u32 = 5;
const PROGRESS_MS: u128 = 150;

/// How a job spreads its streams over the routes it was handed.
///
/// `settings` stores this as the string the wizard writes, and the engine is the
/// only reader, so an unrecognised value resolves to `dynamic`: what the engine
/// did before the setting meant anything. The three are different promises, not
/// three spellings of one behaviour — see `route_for` and `may_hedge`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BondPolicy {
    /// Every route carries blocks at once; a straggler's tail is handed to
    /// whichever worker is free, on any route.
    Dynamic,
    /// The primary carries the bulk; a block whose stream has gone quiet is
    /// raced on a secondary route, and the first half to arrive wins.
    Hedging,
    /// One route until it errors. A slow block is left alone, because moving it
    /// would use a link the user said is only for backup.
    Failover,
}

impl BondPolicy {
    pub fn from_setting(value: &str) -> BondPolicy {
        match value {
            "hedging" => BondPolicy::Hedging,
            "failover" => BondPolicy::Failover,
            _ => BondPolicy::Dynamic,
        }
    }

    /// The route index a stream should use for its `attempt`-th try at a block.
    ///
    /// `dynamic` rotates by both the worker and the attempt, so a job's blocks
    /// land on every device and a retry never reuses the route that just failed.
    /// `hedging` and `failover` keep the first attempt on route 0 — the primary
    /// — and only escalate when the attempt itself failed.
    pub fn route_for(self, worker: usize, attempt: u32, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        match self {
            BondPolicy::Dynamic => (worker + attempt as usize) % count,
            BondPolicy::Hedging | BondPolicy::Failover => (attempt as usize).min(count - 1),
        }
    }

    /// The route a hedging stream should take: never the primary, because the
    /// point of the race is that the primary is the one that went quiet.
    pub fn secondary_route_for(self, worker: usize, attempt: u32, count: usize) -> usize {
        if count < 2 {
            return 0;
        }
        1 + (worker + attempt as usize) % (count - 1)
    }

    /// Whether the straggler-stealing loop may take a tail at all, and on which
    /// trigger. `false` for failover: its copy promises the backup link stays
    /// idle until the primary drops, and an error already moves the attempt.
    pub fn steals(self) -> bool {
        !matches!(self, BondPolicy::Failover)
    }

    /// The word the activity log and the settings row both show, so a plan line
    /// names the strategy that was in force for those bytes.
    pub fn as_str(self) -> &'static str {
        match self {
            BondPolicy::Dynamic => "dynamic",
            BondPolicy::Hedging => "hedging",
            BondPolicy::Failover => "failover",
        }
    }

    /// True when a steal is a *hedge* — triggered by the stream going quiet
    /// rather than by it being the slowest of the lot.
    pub fn hedges(self) -> bool {
        matches!(self, BondPolicy::Hedging)
    }
}

/// How long a claimed block may go without a single reported chunk before
/// hedging races it on a secondary route.
///
/// Three seconds is roughly twenty 144 KiB chunks on a 1 GbE link, so a stream
/// that quiet is stalled rather than slow. `SUCR_HEDGE_STALL_MS` exists for the
/// same reason `SUCR_MIN_SPLIT_BYTES` does: the tests cannot wait for a real
/// stall, and a hedge that only fires in a test is not a hedge.
fn hedge_stall_ms() -> u64 {
    std::env::var("SUCR_HEDGE_STALL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_000)
}

/// How often the ramp's ticker wakes to look for the job ending, between the
/// windows it actually measures. One second is the window a stream is worth
/// arguing about; a quarter-second is how long a finished job may wait for that
/// check to notice.
const TICK_POLL_MS: u64 = 250;

/// The two limits one adapter can carry, as a running job sees them.
///
/// The rate bucket and the allowance live in one entry because the same pass
/// that reads the saved cap reads the saved allowance: a job that kept them
/// apart would have to put two maps back in step by itself, and the two would
/// disagree in exactly the case that matters — an adapter whose limit arrived
/// while it was already transferring.
///
/// The figures are atomics rather than fields behind the map's write lock
/// because they move while other workers are reading them: a worker adds what
/// it carried at the end of every attempt, and the watcher's pass replaces the
/// total with the ledger's own sum.
pub struct NetScope {
    pub bucket: Arc<Bucket>,
    /// Bytes the user allowed inside `window`. Zero means no allowance, because
    /// a worker asks this at every block claim and "no ceiling" and "no answer"
    /// are the same reply.
    allowance: AtomicU64,
    /// Bytes booked inside the window, including whatever the last ledger read
    /// reported. The engine's own bookings are added on top of that sum, so the
    /// figure can briefly count one attempt twice — which only ever ejects a
    /// route a little early, and is corrected by the next pass. For a metered
    /// link, erring early is the safe direction.
    used: AtomicU64,
    /// Which ledger window the allowance is measured against: 0 today, 1 this
    /// week, 2 this month. A number rather than an enum because the row stores
    /// the word and the read returns all three figures in a tuple.
    window: AtomicU8,
}

/// The live per-adapter limits, keyed by `networks.id`. An adapter with neither
/// a cap nor an allowance has no entry, which is the point: an entry in a charge
/// slice is a mutex taken on every block read, and an unlimited scope has
/// nothing to say.
pub type NetworkScopes = Arc<RwLock<BTreeMap<i64, Arc<NetScope>>>>;

impl NetScope {
    pub fn new(cap: Option<u64>) -> Self {
        Self::with_allowance(cap, 0, Self::MONTH)
    }

    /// A scope created by an allowance alone: no rate bucket to charge, so the
    /// entry exists only to answer "is this device out of bytes".
    pub fn with_allowance(cap: Option<u64>, allowance: u64, window: u8) -> Self {
        Self {
            bucket: Arc::new(Bucket::new(cap)),
            allowance: AtomicU64::new(allowance),
            used: AtomicU64::new(0),
            window: AtomicU8::new(window),
        }
    }

    pub const DAY: u8 = 0;
    pub const WEEK: u8 = 1;
    pub const MONTH: u8 = 2;

    /// The window an allowance is measured over, from the word the row keeps.
    /// Anything unrecognised is a month, which is what a quota has always
    /// meant here: `set_network_usage` only ever carried the month figure.
    pub fn window_for(period: Option<&str>) -> u8 {
        match period {
            Some("day") => Self::DAY,
            Some("week") => Self::WEEK,
            _ => Self::MONTH,
        }
    }

    pub fn allowance(&self) -> u64 {
        self.allowance.load(Ordering::Relaxed)
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    pub fn set_allowance(&self, allowance: u64, window: u8) {
        self.allowance.store(allowance, Ordering::Relaxed);
        self.window.store(window, Ordering::Relaxed);
    }

    pub fn set_cap(&self, cap: Option<u64>) {
        self.bucket.set_cap(cap);
    }

    /// The ledger's own sum for the three windows, keeping the one this
    /// allowance is measured against. Written from the pass that already asked
    /// SQLite, so a block claim never takes the database lock.
    pub fn note_ledger(&self, today: i64, week: i64, month: i64) {
        let bytes = match self.window.load(Ordering::Relaxed) {
            Self::DAY => today,
            Self::WEEK => week,
            _ => month,
        };
        self.used.store(bytes.max(0) as u64, Ordering::Relaxed);
    }

    /// Books what this job just carried against the allowance.
    pub fn note_carried(&self, bytes: u64) {
        if self.allowance.load(Ordering::Relaxed) > 0 {
            self.used.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    /// Whether the device has met its allowance, so nothing new should leave on
    /// it until the window rolls or the user raises the number.
    pub fn spent(&self) -> bool {
        let allowance = self.allowance.load(Ordering::Relaxed);
        allowance > 0 && self.used.load(Ordering::Relaxed) >= allowance
    }

    /// Whether anything still needs this entry to exist. An adapter whose cap
    /// was taken away keeps its scope only while an allowance is still being
    /// counted; otherwise the map would hold a bucket that answers "no wait"
    /// to every read for the rest of the session.
    pub fn load_bearing(&self) -> bool {
        self.bucket.cap().is_some() || self.allowance() > 0
    }
}

/// The cap saved for one adapter right now, if it has one.
///
/// Read per block attempt rather than frozen when the job starts. A job that
/// resolved its slice once would keep charging the buckets that existed at
/// start: `set_network_cap` edits a live bucket in place, so a *changed* cap
/// would reach the transfer, but an adapter getting its *first* cap adds a new
/// entry that the frozen slice never looks at — and the promise the card makes
/// is that saving a cap slows the download that is already running.
fn live_cap(networks: &NetworkScopes, id: Option<i64>) -> Option<Arc<Bucket>> {
    let id = id?;
    let map = networks
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.get(&id).map(|scope| Arc::clone(&scope.bucket))
}

/// Whether the adapter behind one route index has met its allowance.
///
/// A route with no adapter is never spent: bytes that cannot be attributed to a
/// device are charged to no allowance, so such a route cannot be ejected by a
/// limit the user set on another row. That is the exception, not the ordinary
/// one-adapter case — `attribute_egress` credits the unbonded default route with
/// the interface the routing table leaves on, and the 2026-11-04 launched run
/// froze a transfer here and restarted it 988 ms later through exactly this
/// gate, on a machine with one adapter and no bond. A route that really does
/// carry no id stays ungated, and the card has no warning for that yet.
fn route_spent(networks: &NetworkScopes, id: Option<i64>) -> bool {
    let Some(id) = id else { return false };
    let map = networks
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.get(&id).is_some_and(|scope| scope.spent())
}

/// The route this claim should leave on, or `None` when every route the policy
/// may use is out of bytes.
///
/// Scans from the index the policy asked for, so a job with no allowances set
/// gets exactly the route it got before this function existed: the wrap is only
/// ever reached from a device that has met its allowance, and it hands the block
/// to the next one rather than failing the download.
///
/// `hedge` narrows the band to the secondary routes, because the half being
/// raced must not fall back onto the stream that went quiet.
fn open_route(
    preferred: usize,
    count: usize,
    hedge: bool,
    route_nets: &[Option<i64>],
    networks: &NetworkScopes,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let (base, span) = if hedge && count > 1 { (1, count - 1) } else { (0, count) };
    let first = preferred.saturating_sub(base) % span;
    for step in 0..span {
        let candidate = base + (first + step) % span;
        if !route_spent(networks, route_nets.get(candidate).copied().flatten()) {
            return Some(candidate);
        }
    }
    None
}

/// What the adapters behind this job's routes are doing, as one comparable value.
///
/// The ramp takes it at both ends of a window: bytes carried while one device was
/// capped and another was not are one measurement, but a cap saved or an allowance
/// met *during* the window makes the two halves incomparable, and the growth bar
/// priced from the earlier half has to be voided rather than answered with a
/// plateau. Only the devices a route can actually leave on are listed, so an
/// adapter the job never uses cannot make the signature move.
///
/// Sampled at the window boundary, so a limit applied and lifted inside one window
/// is invisible — the same blind spot the 250 ms claim cadence already has.
fn bond_state(networks: &NetworkScopes, route_nets: &[Option<i64>]) -> Vec<(i64, bool, bool)> {
    if route_nets.iter().all(|id| id.is_none()) {
        return Vec::new();
    }
    let map = networks
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // One entry per route, in route order, so the list is already the same shape
    // window to window: an adapter two routes share appears twice, and both halves
    // of that pair move together.
    let mut seen = Vec::with_capacity(route_nets.len());
    for id in route_nets.iter().flatten() {
        match map.get(id) {
            Some(scope) => seen.push((*id, scope.bucket.cap().is_some(), scope.spent())),
            None => seen.push((*id, false, false)),
        }
    }
    seen
}

#[derive(Clone)]
pub struct RunContext {
    pub db_path: PathBuf,
    pub downloads_dir: PathBuf,
    pub app: Option<tauri::AppHandle>,
    pub live: Option<Arc<Live>>,
    /// This job's cap; the job thread sets it from the row before transferring.
    pub limit: Arc<Bucket>,
    /// The application-wide cap shared by every job.
    pub global: Arc<Bucket>,
    /// The live per-adapter limits, the same map the application state holds. The
    /// engine names the type through `NetworkScopes` so it never has to reach
    /// for a Tauri type.
    pub networks: NetworkScopes,
    /// Routes this download may use, resolved when the job was started: at
    /// least one, and never re-read mid-transfer, so a plug or an unplug cannot
    /// move a block's bytes to another device behind the label already shown.
    pub routes: Vec<Route>,
    /// Read from `settings` with the routes, by the same rule: a policy changed
    /// halfway through a download would attribute already-stored bytes to a
    /// strategy that was never in force for them.
    pub policy: BondPolicy,
    /// The ramp's clocks. Production always uses the default; the engine tests
    /// hand in a fast one, because a rule with a minute-long recovery in it
    /// otherwise needs a minute-long test to cover.
    pub ramp: RampTuning,
}

impl RunContext {
    fn emit(&self, id: i64) {
        if let Some(app) = &self.app {
            use tauri::Emitter;
            let _ = app.emit("download:changed", json!({ "id": id }));
        }
    }

    /// Pushes one batched block delta to whoever is watching the map. The
    /// cadence and the "is anyone watching" check live in `Live`.
    fn emit_blocks(&self, id: i64, live: Option<&Arc<Live>>, force: bool) {
        let (Some(live), Some(app)) = (live, &self.app) else {
            return;
        };
        let Some(delta) = live.take_delta(force) else {
            return;
        };
        use tauri::Emitter;
        let mut payload = serde_json::to_value(delta).unwrap_or(serde_json::Value::Null);
        if let serde_json::Value::Object(map) = &mut payload {
            map.insert("id".into(), json!(id));
            map.insert("rev".into(), json!(live.rev()));
            map.insert("full".into(), json!(false));
        }
        let _ = app.emit("download:blocks", payload);
    }
}

#[derive(Debug, Clone)]
struct Row {
    url: String,
    folder: Option<String>,
    filename: String,
    etag: Option<String>,
    last_modified: Option<String>,
    total_bytes: Option<i64>,
    block_size: Option<i64>,
    checksum_algo: Option<String>,
    checksum_hex: Option<String>,
    referer: Option<String>,
    streams_fixed: Option<i64>,
    staging_path: Option<String>,
    speed_limit: Option<i64>,
    ua: Option<String>,
    cookie: Option<String>,
    /// The sound stream of a DASH pair taken as one job, if the add-on paired
    /// one. Present makes `run` go through `combined`: picture, then sound,
    /// then one merge — never two rows.
    audio_url: Option<String>,
    /// The finished picture of that pair, kept while the sound is still coming.
    part_path: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn open_db(ctx: &RunContext) -> Result<Connection, JobError> {
    crate::db::open(&ctx.db_path).map_err(|e| JobError::new("db", e))
}

fn load_row(conn: &Connection, id: i64) -> Result<Row, JobError> {
    conn.query_row(
        "SELECT url, folder, filename, etag, last_modified, total_bytes, block_size, \
         checksum_algo, \
         checksum_hex, referer, streams_fixed, staging_path, speed_limit, ua, cookie, \
         audio_url, part_path \
         FROM downloads WHERE id = ?1",
        [id],
        |r| {
            Ok(Row {
                url: r.get(0)?,
                folder: r.get(1)?,
                filename: r.get(2)?,
                etag: r.get(3)?,
                last_modified: r.get(4)?,
                total_bytes: r.get(5)?,
                block_size: r.get(6)?,
                checksum_algo: r.get(7)?,
                checksum_hex: r.get(8)?,
                referer: r.get(9)?,
                streams_fixed: r.get(10)?,
                staging_path: r.get(11)?,
                speed_limit: r.get(12)?,
                ua: r.get(13)?,
                cookie: r.get(14)?,
                audio_url: r.get(15)?,
                part_path: r.get(16)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => JobError::new("missing", format!("download {id} not found")),
        other => JobError::new("db", other.to_string()),
    })
}

/// Which identity one transfer presents: the browser's live value the row rode
/// in on, if the capture saved a usable one, else the host-wide pasted session.
///
/// A per-download value wins because an attested stream (e.g. a Google
/// Proof-of-Origin URL) is only replayable by the exact client that fetched it,
/// and the host session may name a different browser. An empty or whitespace row
/// value is not a usable ride, so it falls through instead of blanking the header.
fn ride<'a>(row_value: Option<&'a str>, session_value: Option<&'a str>) -> Option<&'a str> {
    row_value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(session_value)
}

fn set_status(conn: &Connection, id: i64, status: &str) -> Result<(), JobError> {
    conn.execute("UPDATE downloads SET status = ?1 WHERE id = ?2", params![status, id])
        .map_err(|e| JobError::new("db", e.to_string()))?;
    Ok(())
}

fn history(conn: &Connection, id: i64, kind: &str, payload: serde_json::Value) {
    let _ = conn.execute(
        "INSERT INTO history_events (download_id, ts, kind, payload_json) VALUES (?1, ?2, ?3, ?4)",
        params![id, now_ms(), kind, payload.to_string()],
    );
}

fn set_progress(conn: &Connection, id: i64, verified: u64, total: Option<u64>) -> Result<(), JobError> {
    conn.execute(
        "UPDATE downloads SET verified_bytes = ?1, total_bytes = COALESCE(?2, total_bytes) WHERE id = ?3",
        params![verified as i64, total.map(|t| t as i64), id],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    Ok(())
}

fn mark_attention(conn: &Connection, id: i64, code: &str, message: &str) {
    let _ = conn.execute(
        "UPDATE downloads SET status = 'needs-attention', error_code = ?1, error_hint = ?2 WHERE id = ?3",
        params![code, message, id],
    );
    history(conn, id, "failed", json!({ "code": code, "message": message }));
}

/// A job that fails before its first status write (open/load) would otherwise
/// sit at `queued` forever, because the thread's result was discarded.
pub fn note_early_failure(
    db_path: &std::path::Path,
    app: &tauri::AppHandle,
    id: i64,
    err: &JobError,
) {
    eprintln!("sucr job {id} failed before start: {} ({})", err.message, err.code);
    crate::debug_log::write(
        "ERROR",
        &format!("job {id} failed before start: {} ({})", err.message, err.code),
    );
    let Ok(conn) = crate::db::open(db_path) else {
        return;
    };
    let changed = conn
        .execute(
            "UPDATE downloads SET status = 'needs-attention', error_code = ?1, error_hint = ?2 \
             WHERE id = ?3 AND status = 'queued'",
            params![err.code, err.message, id],
        )
        .unwrap_or(0);
    if changed > 0 {
        history(&conn, id, "failed", json!({ "code": err.code, "message": err.message }));
        use tauri::Emitter;
        let _ = app.emit("download:changed", json!({ "id": id }));
    }
}

/// Entry point for one job thread. Runs to completion; the caller owns the
/// cancel flag and thread lifetime.
pub fn run(ctx: RunContext, id: i64, cancel: Arc<AtomicU8>) -> Result<(), JobError> {
    let mut conn = open_db(&ctx)?;
    let row = load_row(&conn, id)?;
    // The row is the source of truth: a cap set while the job was not running
    // reaches the bucket here.
    ctx.limit
        .set_cap(row.speed_limit.filter(|v| *v > 0).map(|v| v as u64));

    conn.execute(
        "UPDATE downloads SET status = 'downloading', error_code = NULL, error_hint = NULL, \
         started_at = COALESCE(started_at, ?1) WHERE id = ?2",
        params![now_ms(), id],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    ctx.emit(id);

    let outcome = match row.audio_url.clone().filter(|a| !a.trim().is_empty()) {
        // One row, one file: the picture and its sound are two transfers under
        // one job, and the row only publishes the joined file.
        Some(audio) => combined(&ctx, &mut conn, id, &row, &audio, &cancel),
        None => match download(&ctx, &mut conn, id, &row, &cancel, 0) {
            Ok(leg) => publish(&conn, &ctx, id, &row, &leg.final_url, &leg.staging, leg.total, &cancel),
            Err(e) => Err(e),
        },
    };
    match outcome {
        Ok(()) => Ok(()),
        Err(e) => {
            match cancel.load(Ordering::SeqCst) {
                // A stop is a pause with no promise of a restart: the bytes stay,
                // so the row must stop saying it is transferring. Before the
                // removal could refuse, the row was always deleted behind this and
                // the stale status never reached a screen; now it survives, and a
                // row that reads Downloading while no job runs is its own lie.
                // When the removal does go through, this row is deleted anyway.
                STOP | PAUSE => {
                    let _ = set_status(&conn, id, "paused");
                    history(&conn, id, "paused", json!({ "verified": load_verified(&conn, id) }));
                    ctx.emit(id);
                }
                _ => mark_attention(&conn, id, e.code, &e.message),
            }
            ctx.emit(id);
            Err(e)
        }
    }
}

fn load_verified(conn: &Connection, id: i64) -> u64 {
    conn.query_row("SELECT verified_bytes FROM downloads WHERE id = ?1", [id], |r| {
        r.get::<_, i64>(0)
    })
    .map(|v| v.max(0) as u64)
    .unwrap_or(0)
}

/// One stream, start to finish: probe, plan, transfer, length check. It stops
/// short of publishing, because a combined job has a second stream to fetch
/// before the row's one file exists.
struct Leg {
    staging: PathBuf,
    total: u64,
    final_url: String,
}

fn download(
    ctx: &RunContext,
    conn: &mut Connection,
    id: i64,
    row: &Row,
    cancel: &Arc<AtomicU8>,
    // Bytes already carried by an earlier leg of the same row, so one progress
    // bar covers the whole job instead of restarting at the sound.
    base: u64,
) -> Result<Leg, JobError> {
    // The probe goes out on the same route the first stream will use, so the
    // answer it records describes the path the bytes actually take.
    //
    // A bond is probed route by route with the OS default route kept back as the
    // last resort: an enabled device that cannot reach this host (a tunnel, a
    // cable on a subnet the server is not on) may cost bandwidth, but it must
    // never cost the download. A server that answered with an error was
    // reachable, so the remaining routes are not asked again.
    if ctx.routes.is_empty() {
        return Err(JobError::new("no-routes", "All network connections are disabled"));
    }
    let mut routes: Vec<Route> = ctx.routes.clone();
    // Where this URL's bytes leave, asked of the proxy layer itself. A proxied
    // download is not carried by any adapter in the pool: every socket it opens
    // goes to the proxy's address, so the route the OS would have taken to the
    // origin — the one `attribute_egress` below reads — names a device that
    // carries nothing. Until attribution can be re-pointed at the proxy, the
    // honest shape is one unbound route booked to no adapter, which also keeps a
    // loopback proxy reachable from a machine that bonds: a socket bound to
    // 192.168.1.2 cannot dial 127.0.0.1 at all (`EADDRNOTAVAIL`, the same reason
    // `bond_routes` starts bonding at two).
    //
    // A host on the bypass list is exempt and keeps the bond, because the
    // transport asks the same question before it opens a proxy socket and then
    // falls through to `BoundConnector`.
    let egress = super::proxy::egress(&row.url);
    if egress.is_proxied() {
        routes = vec![Route::default_route()];
    }
    if let Some(via) = egress.via() {
        history(
            conn,
            id,
            "proxy",
            json!({
                "via": via,
                "state": if egress.is_proxied() { "proxied" } else { "bypassed" },
            }),
        );
    }
    let mut candidates = routes.clone();
    let escapable = candidates.len() > 1;
    if escapable {
        candidates.push(Route::default_route());
    }

    let mut answered: Option<JobError> = None;
    let mut unreachable: Option<JobError> = None;
    let mut reached: Option<(usize, ureq::Agent, Probe)> = None;
    // The host's pasted browser session, if the user gave one. Read once, here:
    // it decides the identity of every agent this job builds and the Cookie on
    // every request it makes, so a pre-flight that gets through and workers that
    // do not cannot happen. A redirect to another host is that host's business;
    // the cookie stays with the host the link names.
    let host = super::host_key(&row.url);
    let session = if host.is_empty() {
        None
    } else {
        crate::db::session_for(conn, &host).map_err(|e| JobError::new("db", e))?
    };
    let ua = ride(row.ua.as_deref(), session.as_ref().and_then(|s| s.user_agent.as_deref()));
    let cookie = ride(row.cookie.as_deref(), session.as_ref().map(|s| s.cookie.as_str()));
    for (i, cand) in candidates.iter().enumerate() {
        let agent = cand.agent_with(ua);
        match probe(&agent, &row.url, row.referer.as_deref(), cookie) {
            Ok(p) => {
                reached = Some((i, agent, p));
                break;
            }
            Err(e) => {
                // Record what each route said, so the inspector can show the
                // refusal instead of the pre-probe placeholder.
                history(
                    conn,
                    id,
                    "probe",
                    json!({
                        "status": e.status,
                        "error": e.code,
                        "detail": e.message,
                        "route": cand.label,
                    }),
                );
                ctx.emit(id);
                let said_so = e.status.is_some();
                if said_so {
                    answered = Some(e);
                    break;
                }
                if unreachable.is_none() {
                    unreachable = Some(e);
                }
            }
        }
    }
    let (used, agent, mut probe) = match reached {
        Some(v) => v,
        None => return Err(answered.or(unreachable).expect("a probe that failed reports why")),
    };
    if escapable && used + 1 == candidates.len() {
        // Only the unbonded route got through. Bonding is off for this job, and
        // the map stays uncoloured instead of crediting a device that carried
        // nothing.
        routes = vec![Route::default_route()];
        history(
            conn,
            id,
            "bond-fallback",
            json!({
                "tried": candidates.iter().map(|r| r.label.clone()).collect::<Vec<_>>(),
            }),
        );
    } else if used > 0 {
        // Keep the route that answered first: the workers pair `agents[i]` with
        // `routes[i]`, and the pooled handshake the probe just made is index 0.
        routes.rotate_left(used);
    }
    // The probe reached the server, so a route exists to it. Ask which interface
    // that route leaves on and name it, instead of booking nothing — except under
    // a proxy, where the answer would name the device that reaches the *origin*
    // while the bytes travel to the proxy. Those stay booked to no adapter, and
    // the `proxy` history row above is what says why.
    if !egress.is_proxied() {
        attribute_egress(conn, &row.url, &mut routes);
    }
    let referer = row
        .referer
        .clone()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| probe.final_url.clone());
    probe.referer = referer;
    probe.cookie = cookie.map(str::to_string);
    if cancel.load(Ordering::SeqCst) != RUN {
        return Err(JobError::new("cancelled", "cancelled during probe"));
    }

    let folder = PathBuf::from(
        row.folder
            .clone()
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| ctx.downloads_dir.to_string_lossy().into_owned()),
    );
    std::fs::create_dir_all(&folder)?;

    let filename = probe
        .filename
        .as_deref()
        .map(super::sanitize_filename)
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| row.filename.clone());

    let etag = probe.etag.clone().or_else(|| row.etag.clone());
    let total = probe.total_bytes;

    conn.execute(
        "UPDATE downloads SET final_url = ?1, etag = ?2, last_modified = ?3, content_type = ?4, \
         total_bytes = COALESCE(?5, total_bytes), filename = ?6, folder = ?7 WHERE id = ?8",
        params![
            probe.final_url,
            etag,
            probe.last_modified,
            probe.content_type,
            total.map(|t| t as i64),
            filename,
            folder.to_string_lossy(),
            id
        ],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    history(
        conn,
        id,
        "probe",
        json!({
            "status": probe.status,
            "total": total,
            "ranges": probe.accept_ranges,
            "etag": probe.etag,
            "finalUrl": probe.final_url,
        }),
    );
    ctx.emit(id);

    // A changed validator or length invalidates every byte already written.
    let plan_changed = row.total_bytes.is_some()
        && total.is_some()
        && row.total_bytes != total.map(|t| t as i64);
    // The two validators are alternatives, not a checklist: a host that answers
    // with an ETag has said more about the file than its timestamp can, so the
    // timestamp is only asked when neither side carries an ETag — the same
    // priority the relink assert in `check_refresh` uses. Comparing both
    // unconditionally would throw away resumable bytes over a clock skew.
    let validator = match (&row.etag, &probe.etag) {
        (Some(have), Some(now)) => (have != now).then_some("etag"),
        (Some(_), None) | (None, Some(_)) => None,
        (None, None) => {
            let moved = row.last_modified.is_some()
                && probe.last_modified.is_some()
                && row.last_modified != probe.last_modified;
            moved.then_some("last-modified")
        }
    };
    let validator_changed = validator.is_some();

    let parallel = probe.accept_ranges && total.map(|t| t > 0).unwrap_or(false);
    let mut reset_progress = plan_changed || validator_changed;
    let saved_staging = row.staging_path.as_ref().map(PathBuf::from);
    // A partial file that is simply gone is a third cause, on its own: neither
    // the announced length nor the validator moved, so the trail must not blame
    // the server for bytes nobody was holding.
    let staging_missing = matches!(&saved_staging, Some(path) if !path.exists());
    // A moved validator says the host has another file; a moved length says it
    // loudly enough that nothing can argue. In between, only the bytes can
    // settle it, and settling it is the difference between re-pulling a
    // gigabyte and resuming it. A host that will not serve ranges, or a partial
    // nobody is holding, gets no hearing: both answer "no evidence".
    let mut sampled: Option<Sampled> = None;
    if reset_progress && !plan_changed && probe.accept_ranges {
        if let Some(path) = saved_staging.as_ref().filter(|path| path.exists()) {
            let ranges = covered_ranges(conn, id, Some(path));
            let verdict = sample_windows(
                &agent,
                &row.url,
                &probe.referer,
                cookie,
                &ranges,
                path,
                &cancel,
            );
            if verdict.outcome == "matched" {
                reset_progress = false;
            }
            sampled = Some(verdict);
        }
    }
    if let Some(sampled) = &sampled {
        history(
            conn,
            id,
            "bytes-sampled",
            json!({
                "validator": validator,
                "covered": sampled.covered,
                "windows": sampled.windows,
                "outcome": sampled.outcome,
                "kept": sampled.outcome == "matched",
            }),
        );
    }
    let staging = match saved_staging {
        Some(path) if path.exists() && !reset_progress => path,
        _ => {
            if row.staging_path.is_some() {
                reset_progress = true;
            }
            reset_staging(conn, id, &folder, &filename, total)?
        }
    };
    if reset_progress {
        conn.execute("DELETE FROM blocks WHERE download_id = ?1", [id])
            .map_err(|e| JobError::new("db", e.to_string()))?;
        conn.execute("UPDATE downloads SET verified_bytes = 0 WHERE id = ?1", [id])
            .map_err(|e| JobError::new("db", e.to_string()))?;
        history(
            conn,
            id,
            "resume-reset",
            json!({
                "planChanged": plan_changed,
                "validatorChanged": validator_changed,
                "validatorKind": validator,
                "stagingMissing": staging_missing,
                // Why the samples did not save these bytes, when a comparison
                // was made at all: `differed` is a real change of file,
                // `no-evidence` is a partial too short to compare, `inconclusive`
                // is a host that would not answer the way a sample needs.
                "sampleOutcome": sampled.as_ref().map(|s| s.outcome),
            }),
        );
    }

    let result = if parallel {
        let total = total.unwrap();
        // A number stored on the row is the user's own dial: that job starts and
        // stops there. Otherwise the band in the setting decides, and `auto` is
        // the band the ramp is allowed to climb inside.
        let band = row
            .streams_fixed
            .filter(|s| *s > 0)
            .map(|s| StreamBand { start: s as usize, ceiling: s as usize })
            .unwrap_or_else(|| stream_band(conn));
        // A block size stored on the row is a plan already in progress: reuse
        // it, or the resume would re-plan and throw away the bytes already
        // written. Only a fresh job sizes its own slices.
        let block_size = row
            .block_size
            .filter(|b| *b > 0)
            .map(|b| b as u64)
            .unwrap_or_else(|| adaptive_block(total, band.start, DEFAULT_BLOCK))
            .min(total.max(1));
        parallel_download(conn, ctx, &routes, id, row, &probe, &agent, ua, &staging, total, block_size, &band, cancel, base)
    } else {
        conn.execute(
            "UPDATE downloads SET streams_mode = 'single', block_size = NULL, block_count = NULL WHERE id = ?1",
            [id],
        )
        .map_err(|e| JobError::new("db", e.to_string()))?;
        history(conn, id, "single-stream", json!({ "reason": if probe.accept_ranges { "unknown-length" } else { "no-range-support" } }));
        single_download(conn, ctx, &routes, id, &probe, &agent, &staging, cancel, base)
    };

    let done = match result {
        Ok(done) => Ok(done),
        Err(JobError { code: "fallback", .. }) => {
            // A server that ignores Range: forget the plan and stream it whole.
            // RESTART was a control signal, not a cancellation, so clear it —
            // but never overwrite a pause or remove that arrived meanwhile.
            if cancel.load(Ordering::SeqCst) == RESTART {
                cancel.store(RUN, Ordering::SeqCst);
            }
            conn.execute("DELETE FROM blocks WHERE download_id = ?1", [id])
                .map_err(|e| JobError::new("db", e.to_string()))?;
            conn.execute(
                "UPDATE downloads SET streams_mode = 'single', block_size = NULL, block_count = NULL, \
                 verified_bytes = 0 WHERE id = ?1",
                [id],
            )
            .map_err(|e| JobError::new("db", e.to_string()))?;
            history(conn, id, "fallback-single", json!({}));
            single_download(conn, ctx, &routes, id, &probe, &agent, &staging, cancel, base)
        }
        Err(e) => Err(e),
    }?;

    // The bytes on disk are the length the server announced. Checked per leg:
    // a truncated picture would otherwise be merged into a file that plays
    // only halfway.
    let on_disk = std::fs::metadata(&staging)?.len();
    if on_disk != done {
        return Err(JobError::new(
            "length-mismatch",
            format!("staging file is {on_disk} bytes, expected {done}"),
        ));
    }
    Ok(Leg {
        staging,
        total: done,
        final_url: probe.final_url.clone(),
    })
}

/// One row carrying a DASH pair: the picture arrives first, the sound second,
/// and the row publishes the joined file — never the two streams, so the
/// library shows one download with one progress bar.
///
/// Resume works per leg. The picture is kept at `part_path` once its length is
/// proven; a restart during the sound re-fetches only the sound, and a restart
/// during the picture resumes it from its own staging file.
fn combined(
    ctx: &RunContext,
    conn: &mut Connection,
    id: i64,
    row: &Row,
    audio_url: &str,
    cancel: &Arc<AtomicU8>,
) -> Result<(), JobError> {
    let folder = PathBuf::from(
        row.folder
            .clone()
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| ctx.downloads_dir.to_string_lossy().into_owned()),
    );
    std::fs::create_dir_all(&folder)?;
    let name = row.filename.clone();
    let part = folder.join(format!("{name}.{id}.sucrpart.v"));
    let keep_part = row.part_path.as_deref() == Some(part.to_string_lossy().as_ref())
        && part.is_file();

    let (base, feed_url) = if keep_part {
        (std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0), row.url.clone())
    } else {
        let _ = std::fs::remove_file(&part);
        reset_leg(conn, id)?;
        let picture = download(ctx, conn, id, row, cancel, 0)?;
        super::rename_with_retry(&picture.staging, &part)
            .map_err(|e| JobError::new("merge", format!("keep the picture: {e}")))?;
        reset_leg(conn, id)?;
        conn.execute(
            "UPDATE downloads SET part_path = ?1, filename = ?2 WHERE id = ?3",
            params![part.to_string_lossy(), name, id],
        )
        .map_err(|e| JobError::new("db", e.to_string()))?;
        history(conn, id, "pair-picture", json!({ "bytes": picture.total, "path": part.to_string_lossy() }));
        ctx.emit(id);
        (picture.total, picture.final_url)
    };

    // The sound leg keeps whatever the row loaded — on a restart during the
    // sound that is its own staging path, validators and block plan, which is
    // what makes the second leg resumable at all.
    let sound = Row {
        url: audio_url.to_string(),
        total_bytes: if keep_part { row.total_bytes } else { None },
        etag: if keep_part { row.etag.clone() } else { None },
        last_modified: if keep_part { row.last_modified.clone() } else { None },
        block_size: if keep_part { row.block_size } else { None },
        staging_path: if keep_part { row.staging_path.clone() } else { None },
        folder: row.folder.clone(),
        filename: name.clone(),
        checksum_algo: None,
        checksum_hex: None,
        referer: row.referer.clone(),
        streams_fixed: row.streams_fixed,
        speed_limit: row.speed_limit,
        ua: row.ua.clone(),
        cookie: row.cookie.clone(),
        audio_url: None,
        part_path: None,
    };
    let leg = download(ctx, conn, id, &sound, cancel, base)?;
    // The sound's probe may have named itself off a header; the row keeps the
    // one file's name.
    conn.execute(
        "UPDATE downloads SET filename = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;

    let dest = super::unique_path(&folder, &name);
    history(
        conn,
        id,
        "merge-start",
        json!({
            "picture": part.to_string_lossy(),
            "sound": leg.staging.to_string_lossy(),
            "out": dest.to_string_lossy(),
            "base": base,
        }),
    );
    ctx.emit(id);
    join_streams(&part, &leg.staging, &dest).map_err(|e| JobError::new("merge", e))?;
    let merged = std::fs::metadata(&dest)?.len();
    let _ = std::fs::remove_file(&part);
    let _ = std::fs::remove_file(&leg.staging);
    finish_merged(conn, ctx, id, row, &dest, &feed_url, merged, cancel)
}

/// Forget a leg's transfer state so the next one plans from nothing. The pair
/// itself (`part_path`) is untouched: it is the row, not a leg.
fn reset_leg(conn: &Connection, id: i64) -> Result<(), JobError> {
    conn.execute("DELETE FROM blocks WHERE download_id = ?1", [id])
        .map_err(|e| JobError::new("db", e.to_string()))?;
    conn.execute(
        "UPDATE downloads SET staging_path = NULL, verified_bytes = 0, total_bytes = NULL, \
         etag = NULL, last_modified = NULL, block_size = NULL, block_count = NULL, \
         streams_mode = NULL, streams_final = NULL WHERE id = ?1",
        [id],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    Ok(())
}

/// Container-copy join, picked by the extension the row's one file carries:
/// a VP9/Opus pair stays WebM, everything else becomes a progressive MP4.
fn join_streams(picture: &Path, sound: &Path, out: &Path) -> Result<(), String> {
    let webm = out
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("webm"));
    if webm {
        super::webm::mux(picture, sound, out).map_err(|e| e.0)
    } else {
        super::mux::mux(picture, Some(sound), out).map_err(|e| e.0)
    }
}

/// Close a combined job: the merged file is the row's file.
fn finish_merged(
    conn: &Connection,
    ctx: &RunContext,
    id: i64,
    row: &Row,
    dest: &Path,
    feed_url: &str,
    total: u64,
    cancel: &Arc<AtomicU8>,
) -> Result<(), JobError> {
    if cancel.load(Ordering::SeqCst) != RUN {
        return Err(JobError::new("cancelled", "cancelled before publish"));
    }
    let motw = crate::db::setting(conn, "storage.motw")
        .ok()
        .flatten()
        .map(|v| !v.eq_ignore_ascii_case("false") && v != "0")
        .unwrap_or(true);
    if cfg!(windows) && motw {
        let referrer = row.referer.as_deref().unwrap_or(feed_url);
        if let Err(e) = write_zone_identifier(dest, referrer, &host_of(feed_url)) {
            history(conn, id, "zone-skipped", json!({ "why": e.to_string() }));
        }
    }
    let folder = dest.parent().unwrap_or(Path::new("."));
    let name = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| row.filename.clone());
    conn.execute(
        "UPDATE downloads SET status = 'finished', folder = ?1, filename = ?2, \
         staging_path = NULL, part_path = NULL, verified_bytes = ?3, total_bytes = ?3, \
         finished_at = ?4, error_code = NULL, error_hint = NULL WHERE id = ?5",
        params![
            folder.to_string_lossy(),
            name,
            total as i64,
            now_ms(),
            id
        ],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    history(conn, id, "merged", json!({ "path": dest.to_string_lossy(), "bytes": total }));
    ctx.emit(id);
    Ok(())
}

/// Credits an unbonded download to the adapter the routing table says carried it.
///
/// A bond names its own device per chunk, but with fewer than two enabled
/// adapters the pool is the OS default route and `network_id` was left `None` —
/// so `finish_attempt` booked nothing and every single-NIC machine reported
/// `Data downloaded: 0 B` forever, on the one line the card exists to show.
///
/// This is a lookup, not a bind: the transfer keeps whatever reachability
/// Windows gives an unbound socket. A localhost target answers `127.0.0.1`, and
/// `network::is_routable_ipv4` keeps that address out of the adapter list, so
/// those jobs stay unattributed exactly as they did before.
fn attribute_egress(conn: &Connection, url: &str, routes: &mut [Route]) {
    let Some(route) = routes
        .iter_mut()
        .find(|route| route.bind.is_none() && route.network_id.is_none())
    else {
        return;
    };
    let Some(source) = super::bond::egress_source(url) else {
        return;
    };
    let wanted = source.to_string();
    let Ok(adapters) = super::network::enumerate() else {
        return;
    };
    let Some(adapter) = adapters.iter().find(|a| a.local_ip == wanted) else {
        return;
    };
    let hint = super::network::identity(adapter);
    let Ok(Some(member)) = crate::db::network_for_hint(conn, &hint) else {
        // No card for this adapter yet: it has never been enumerated into the
        // database, so there is no row to charge and no colour to claim.
        return;
    };
    route.network_id = Some(member.id);
    route.color = member
        .color_id
        .unwrap_or_else(|| UNATTRIBUTED.to_string());
}

/// `storage.preallocation=none` keeps the staging file growing with the data;
/// every other mode reserves the full length up front.
fn reserves_length(conn: &Connection) -> bool {
    !matches!(
        crate::db::setting(conn, "storage.preallocation").ok().flatten().as_deref(),
        Some("none")
    )
}

fn reset_staging(
    conn: &Connection,
    id: i64,
    folder: &Path,
    filename: &str,
    total: Option<u64>,
) -> Result<PathBuf, JobError> {
    let path = folder.join(format!("{filename}.{id}.sucrpart"));
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    let mode = crate::db::setting(conn, "storage.preallocation")
        .ok()
        .flatten()
        .unwrap_or_else(|| "sparse".to_string());
    if let Some(total) = total {
        match mode.as_str() {
            // Leave the file at zero length: positional writes grow it.
            "none" => {}
            // Reserve the full length and write real zeros, so a slow link
            // never sees the file grow while Explorer reports a tiny size.
            "full" => {
                file.set_len(total)?;
                let zeros = vec![0u8; 1024 * 1024];
                let mut out = &file;
                let mut left = total;
                while left > 0 {
                    let n = left.min(zeros.len() as u64) as usize;
                    out.write_all(&zeros[..n])?;
                    left -= n as u64;
                }
            }
            // Default: mark the staging file sparse, then reserve its length.
            // Marking is best-effort (FAT and network shares refuse it).
            _ => {
                #[cfg(windows)]
                let _ = set_sparse(&file);
                file.set_len(total)?;
            }
        }
    }
    drop(file);
    conn.execute(
        "UPDATE downloads SET staging_path = ?1 WHERE id = ?2",
        params![path.to_string_lossy(), id],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    Ok(path)
}

/// FSCTL_SET_SPARSE via DeviceIoControl. Declared here rather than pulling a
/// Win32 crate in: the release binary budget is tight and this is four lines.
#[cfg(windows)]
fn set_sparse(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    const FSCTL_SET_SPARSE: u32 = 0x0009_00C4;

    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(
            handle: *mut core::ffi::c_void,
            code: u32,
            in_buf: *mut core::ffi::c_void,
            in_size: u32,
            out_buf: *mut core::ffi::c_void,
            out_size: u32,
            returned: *mut u32,
            overlapped: *mut core::ffi::c_void,
        ) -> i32;
    }

    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as *mut _,
            FSCTL_SET_SPARSE,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct BlockSpec {
    idx: usize,
    start: u64,
    end: u64,
}

impl BlockSpec {
    fn len(&self) -> u64 {
        self.end - self.start + 1
    }
}

/// One HTTP attempt for one block. The row is written when the block is claimed
/// and updated when the attempt ends, so the Connections table can show a live
/// row from the first byte instead of waiting for a completion.
#[allow(clippy::too_many_arguments)]
fn begin_attempt(
    conn: &Connection,
    live: Option<&Arc<Live>>,
    id: i64,
    block_idx: usize,
    range_start: u64,
    range_end: u64,
    started_at: i64,
    route: &Route,
) -> Option<i64> {
    let inserted = conn.execute(
        "INSERT INTO block_attempts (download_id, block_idx, network_id, started_at, bytes, state, reason) \
         VALUES (?1, ?2, ?3, ?4, 0, 'active', NULL)",
        params![id, block_idx as i64, route.network_id, started_at],
    );
    if inserted.is_err() {
        return None;
    }
    let attempt_id = conn.last_insert_rowid();
    if let Some(live) = live {
        live.push_attempt(LiveAttempt {
            id: attempt_id,
            network_id: route.color.clone(),
            network_label: route.label.clone(),
            block_idx,
            range_start,
            range_end,
            bytes: 0,
            state: "active".to_string(),
            started_at,
            duration_ms: 0,
            reason: None,
            detail: None,
        });
    }
    Some(attempt_id)
}

#[allow(clippy::too_many_arguments)]
fn finish_attempt(
    conn: &Connection,
    live: Option<&Arc<Live>>,
    attempt_id: i64,
    block_idx: usize,
    range_start: u64,
    range_end: u64,
    started_at: i64,
    bytes: u64,
    state: &str,
    reason: Option<String>,
    detail: Option<String>,
    route: &Route,
    networks: &NetworkScopes,
) {
    let ended_at = now_ms();
    let updated = conn.execute(
        "UPDATE block_attempts SET ended_at = ?1, bytes = ?2, state = ?3, reason = ?4, \
         detail = ?5 WHERE id = ?6",
        params![ended_at, bytes as i64, state, reason.clone(), detail.clone(), attempt_id],
    );
    if updated.is_err() {
        return;
    }
    // Bytes are booked to the device that actually carried them, once, when the
    // attempt ends. Counting them here rather than per chunk keeps the write
    // rate at one row update per attempt.
    //
    // The live scope is told in the same breath, from the same figure: the table
    // is the record, but a worker asks the scope at every block claim and must not
    // take the database lock to do it. The watcher's pass replaces this running
    // total with the ledger's own sum, so a count that drifted — through two jobs
    // booking at once, or a window that rolled mid-attempt — corrects itself.
    if let Some(network_id) = route.network_id {
        let _ = crate::db::add_net_usage(conn, network_id, bytes);
        if let Some(scope) = networks
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&network_id)
        {
            scope.note_carried(bytes);
        }
    } else {
        // No adapter was credited with these: a proxied tunnel, a localhost
        // target, a route whose device has no card. They go to a ledger of their
        // own rather than being dropped, because the Networks pane's totals are
        // sums over the adapter rows and a class of traffic missing from a sum
        // reads as bytes that never moved. Nothing here reaches a live scope: a
        // cap or allowance paces a device, and no device carried this.
        let _ = crate::db::add_unattributed_usage(conn, bytes);
    }
    if let Some(live) = live {
        live.push_attempt(LiveAttempt {
            id: attempt_id,
            network_id: route.color.clone(),
            network_label: route.label.clone(),
            block_idx,
            range_start,
            range_end,
            bytes,
            state: state.to_string(),
            started_at,
            duration_ms: ended_at - started_at,
            reason,
            detail,
        });
    }
}

/// Attempts left `active` by a killed process would otherwise look live forever.
fn close_dangling_attempts(conn: &Connection, id: i64) {
    let _ = conn.execute(
        "UPDATE block_attempts SET state = 'interrupted', ended_at = ?1 \
         WHERE download_id = ?2 AND state = 'active' AND ended_at IS NULL",
        params![now_ms(), id],
    );
}

fn sync_live(live: Option<&Arc<Live>>, done: &[u64]) {
    let Some(live) = live else {
        return;
    };
    live.update_blocks(|b| {
        let d = done.get(b.idx).copied().unwrap_or(0);
        b.bytes_done = d;
        if d == b.len {
            b.state = "done".to_string();
        } else if d > 0 && b.state == "pending" {
            b.state = "active".to_string();
        }
    });
}

struct BlockCoord {
    pos: AtomicU64,
    cutoff: AtomicU64,
    total_done: AtomicU64,
    /// Part of `total_done`: the bytes this block carried while an adapter rate
    /// cap was in force on the route that claimed them. The ramp subtracts it so
    /// a paced window is not read as host evidence.
    limited_done: AtomicU64,
    /// When this block last produced a chunk. Only `hedging` reads it: the
    /// dynamic trigger compares remaining bytes, which needs no clock.
    last_ms: AtomicI64,
    active: AtomicBool,
    done_flag: AtomicBool,
}

const MIN_SPLIT_BYTES: u64 = 1024 * 1024;

fn min_split_bytes() -> u64 {
    if let Ok(val) = std::env::var("SUCR_MIN_SPLIT_BYTES") {
        if let Ok(n) = val.parse::<u64>() {
            return n;
        }
    }
    MIN_SPLIT_BYTES
}

fn try_steal_tail(
    coords: &[BlockCoord],
    plan: &[BlockSpec],
    policy: BondPolicy,
) -> Option<(usize, u64, u64, u64)> {
    if !policy.steals() {
        return None;
    }
    let min_bytes = min_split_bytes();
    let stall = hedge_stall_ms();
    let now = now_ms();
    let mut best: Option<(usize, u64, u64, u64)> = None;
    let mut max_rem = 0u64;

    for (k, _) in plan.iter().enumerate() {
        if coords[k].done_flag.load(Ordering::SeqCst) {
            continue;
        }
        if !coords[k].active.load(Ordering::SeqCst) {
            continue;
        }
        let pos = coords[k].pos.load(Ordering::SeqCst);
        let cut = coords[k].cutoff.load(Ordering::SeqCst);
        if cut <= pos {
            continue;
        }
        let rem = cut - pos;
        if rem < 2 * min_bytes {
            continue;
        }
        if policy.hedges() {
            // A hedge answers a stalled stream, not a merely large one: the
            // candidate has to have gone quiet for the stall window. Handed
            // over as a half, same as the dynamic steal, because the cutoff
            // compare-exchange is what keeps the two streams' bytes from being
            // counted twice — a hedge that re-asked the whole remainder would
            // have to invent a second accounting rule to stay correct.
            let quiet = (now - coords[k].last_ms.load(Ordering::Relaxed)).max(0) as u64;
            if quiet < stall {
                continue;
            }
        } else if rem <= max_rem {
            continue;
        }
        max_rem = rem;
        best = Some((k, pos, cut, rem));
    }

    if let Some((k, pos, cut, rem)) = best {
        let half = rem / 2;
        let mid = pos + half;
        if coords[k]
            .cutoff
            .compare_exchange(cut, mid, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Some((k, mid, cut, rem));
        }
    }
    None
}

enum Msg {
    Chunk { idx: usize, done: u64 },
    Done { idx: usize },
    /// `route` indexes the job's route list, so the coordinator can attribute
    /// the attempt to the device that carried it.
    Claim {
        worker: usize,
        idx: usize,
        range_start: u64,
        range_end: u64,
        started_at: i64,
        route: usize,
    },
    /// `reason` is the chip the Chunks pane shows, so it has to survive a
    /// 100-pixel column; `detail` is the full sentence for the hover text and
    /// the activity log, where length costs nothing.
    Attempt {
        worker: usize,
        idx: usize,
        range_start: u64,
        range_end: u64,
        started_at: i64,
        bytes: u64,
        state: &'static str,
        reason: Option<String>,
        detail: Option<String>,
        route: usize,
    },
    TailSplit {
        idx: usize,
        split_offset: u64,
        bytes_left: u64,
    },
    Retry { idx: usize, why: String },
    /// A host answered a rate limit and the job's stream width narrowed.
    /// `streams` is the new ceiling, so the activity log can say what the
    /// server forced rather than only what the client chose, and `previous` is
    /// the width it was called from, which is how the ramp knows how far above
    /// what this host tolerates it had climbed.
    Throttle {
        idx: usize,
        status: u16,
        streams: usize,
        previous: usize,
        wait_secs: u64,
    },
    Fail { idx: usize, code: &'static str, why: String },
    Restart,
    /// The ramp's clock. A second thread cannot touch SQLite, so the ticker only
    /// wakes the coordinator, which owns the ramp's state and does all of its
    /// arithmetic here. Silence is the reason it exists: a stalled host sends no
    /// messages at all, and the recovery rules are the ones that have to run then.
    /// The payload is the wall time at send, so a window can report how long this tick
    /// waited behind the coordinator's other work — the only way to tell a channel
    /// backlog from a ticker that is itself late.
    Tick(i64),
    /// A worker or the ticker has finished. The supervisor counts these instead
    /// of waiting for the channel to close, because it keeps a sender of its own
    /// to start the streams the ramp asks for.
    Left,
}

/// The width a job opens when its row pinned none: the tunable, or the
/// built-in when the setting is missing or nonsense. The IPC reads use this too,
/// so the hero's stream count and the engine's worker count never disagree.
pub fn default_streams(conn: &Connection) -> usize {
    stream_band(conn).start
}

/// The width band a fresh job runs in.
///
/// A number in `engine.default_streams` is pinned: it starts and stops there,
/// because a dial set to 4 is a promise to the host, not a suggestion. `auto`
/// (and an unset setting) is the band the engine owns — it opens at
/// `DEFAULT_STREAMS` and the ramp may take it to `MAX_STREAMS` if the bytes
/// prove the extra streams are worth a socket.
pub struct StreamBand {
    pub start: usize,
    pub ceiling: usize,
}

pub fn stream_band(conn: &Connection) -> StreamBand {
    let pinned = crate::db::setting(conn, "engine.default_streams")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|s| *s > 0)
        .map(|s| s.clamp(1, MAX_STREAMS));
    match pinned {
        Some(n) => StreamBand { start: n, ceiling: n },
        None => StreamBand { start: DEFAULT_STREAMS, ceiling: MAX_STREAMS },
    }
}
#[allow(clippy::too_many_arguments)]
fn parallel_download(
    conn: &Connection,
    ctx: &RunContext,
    routes: &[Route],
    id: i64,
    row: &Row,
    probe: &Probe,
    agent: &ureq::Agent,
    // The pre-flight's session identity, so a second route handshakes as the same
    // client the server already said yes to.
    user_agent: Option<&str>,
    staging: &Path,
    total: u64,
    block_size: u64,
    band: &StreamBand,
    cancel: &Arc<AtomicU8>,
    base: u64,
) -> Result<u64, JobError> {
    let live = ctx.live.as_ref();

    // The route the probe already used is `routes[0]` and keeps its pooled
    // connection, so the first worker reuses it instead of opening a second
    // handshake.
    let mut agents: Vec<ureq::Agent> = Vec::with_capacity(routes.len());
    agents.push(agent.clone());
    for route in routes.iter().skip(1) {
        agents.push(route.agent_with(user_agent));
    }
    let agents = Arc::new(agents);
    // The adapter behind each route index, kept beside the agents it belongs to.
    let route_nets: Vec<Option<i64>> = routes.iter().map(|r| r.network_id).collect();

    let ranges = plan_blocks(total, block_size);
    let plan: Vec<BlockSpec> = ranges
        .iter()
        .enumerate()
        .map(|(idx, (s, e))| BlockSpec { idx, start: *s, end: *e })
        .collect();

    let existing: Vec<(i64, i64, i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT idx, bytes_done, range_end, state FROM blocks WHERE download_id = ?1")
            .map_err(|e| JobError::new("db", e.to_string()))?;
        let rows = stmt
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(|e| JobError::new("db", e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| JobError::new("db", e.to_string()))?
    };
    let plan_matches = existing.len() == plan.len()
        && existing.iter().all(|(idx, done, end, _)| {
            let b = &plan[*idx as usize];
            *done <= b.len() as i64 && *end == b.end as i64
        });

    if !plan_matches {
        conn.execute("DELETE FROM blocks WHERE download_id = ?1", [id])
            .map_err(|e| JobError::new("db", e.to_string()))?;
        let tx = conn.unchecked_transaction().map_err(|e| JobError::new("db", e.to_string()))?;
        for b in &plan {
            tx.execute(
                "INSERT INTO blocks (download_id, idx, range_start, range_end, state, bytes_done) \
                 VALUES (?1, ?2, ?3, ?4, 'pending', 0)",
                params![id, b.idx as i64, b.start as i64, b.end as i64],
            )
            .map_err(|e| JobError::new("db", e.to_string()))?;
        }
        tx.commit().map_err(|e| JobError::new("db", e.to_string()))?;
        conn.execute(
            "UPDATE downloads SET block_size = ?1, block_count = ?2, streams_mode = 'adaptive', \
             resumable = 1, verified_bytes = 0 WHERE id = ?3",
            params![block_size as i64, plan.len() as i64, id],
        )
        .map_err(|e| JobError::new("db", e.to_string()))?;
        history(
            conn,
            id,
            "plan",
            json!({
                "blocks": plan.len(),
                "blockSize": block_size,
                "streams": band.start,
                // How far the ramp may take this job. Recorded so the trail of a
                // job that ran at 24 streams says that 24 was allowed, not won.
                "streamsMax": band.ceiling,
                // The strategy these bytes were planned under, so a later reader
                // of the activity log can tell a bond from a failover.
                "policy": ctx.policy.as_str(),
                // Which devices the plan will actually spread over, so the
                // activity log can say what "bonded" meant for this job.
                "routes": routes.iter().map(|r| r.label.clone()).collect::<Vec<String>>(),
            }),
        );
    } else {
        conn.execute(
            "UPDATE downloads SET streams_mode = 'adaptive', resumable = 1 WHERE id = ?1",
            [id],
        )
        .map_err(|e| JobError::new("db", e.to_string()))?;
        history(
            conn,
            id,
            "resumed",
            // A chunk is kept when its own byte count reaches its length, which
            // is how the map decides `done` too. Comparing the count with the
            // block's end offset instead — as this line used to — can only ever
            // answer zero, so the trail claimed nothing survived a resume while
            // the map showed the chunks sitting there finished.
            json!({
                "done": existing
                    .iter()
                    .filter(|(idx, bytes, _, _)| *bytes == plan[*idx as usize].len() as i64)
                    .count(),
                "blocks": plan.len()
            }),
        );
    }

    let mut done: Vec<u64> = vec![0; plan.len()];
    if plan_matches {
        for (idx, bytes, _, _) in &existing {
            done[*idx as usize] = (*bytes).max(0) as u64;
        }
    }
    // Where the database said each block stood when this run began. A block that
    // is split mid-run can only safely be continued from here; see `resumable`.
    let resume = done.clone();
    // Blocks this run handed a tail to, so their progress is a sum of two
    // streams rather than a run of bytes from the front.
    let mut split: Vec<bool> = vec![false; plan.len()];

    if let Some(live) = live {
        live.set_plan(
            block_size,
            total,
            plan.iter()
                .map(|b| LiveBlock {
                    idx: b.idx,
                    offset: b.start,
                    len: b.len(),
                    bytes_done: done[b.idx],
                    state: if done[b.idx] == b.len() { "done" } else { "pending" }.to_string(),
                    // "n0" is the unattributed colour: a block resumed from an
                    // earlier run has no record of which device pulled it, and
                    // claiming one now overwrites this with that device's id.
                    network_id: UNATTRIBUTED.to_string(),
                })
                .collect(),
        );
    }

    let file = Arc::new(
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(staging)?,
    );
    if reserves_length(conn) {
        file.set_len(total)?;
    }

    let max_viable_workers = (total / MIN_SPLIT_BYTES).max(1) as usize;
    let threads = band.start.clamp(1, MAX_STREAMS).min(max_viable_workers);
    let next = Arc::new(AtomicUsize::new(0));
    // All of them may run at first; a rate limit narrows the gate and the
    // threads simply wait their turn instead of dying.
    let gate = Arc::new(Gate::new(threads));
    // The UI reads the width off the live job, so it has to be there from the
    // first tick and not only after the ramp moves it.
    if let Some(live) = live {
        live.set_streams(gate.width());
    }
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut handles = Vec::with_capacity(MAX_STREAMS);

    // Shared by every worker and the supervisor, so a stream added at minute
    // nine costs no more than the first one did.
    let plan = Arc::new(plan);
    let coords: Arc<Vec<BlockCoord>> = Arc::new(
        plan.iter()
            .enumerate()
            .map(|(i, b)| BlockCoord {
                pos: AtomicU64::new(done[i]),
                cutoff: AtomicU64::new(b.len()),
                total_done: AtomicU64::new(done[i]),
                limited_done: AtomicU64::new(0),
                last_ms: AtomicI64::new(now_ms()),
                active: AtomicBool::new(false),
                done_flag: AtomicBool::new(done[i] >= b.len()),
            })
            .collect(),
    );

    let alive = Arc::new(AtomicUsize::new(0));
    let env = WorkerEnv {
        agents,
        url: row.url.clone(),
        etag: probe.etag.clone().or_else(|| row.etag.clone()),
        referer: probe.referer.clone(),
        cookie: probe.cookie.clone(),
        plan: Arc::clone(&plan),
        coords: Arc::clone(&coords),
        file: Arc::clone(&file),
        next,
        gate: Arc::clone(&gate),
        cancel: Arc::clone(cancel),
        limit: Arc::clone(&ctx.limit),
        global: Arc::clone(&ctx.global),
        networks: Arc::clone(&ctx.networks),
        route_nets,
        policy: ctx.policy,
    };
    for w in 0..threads {
        handles.push(env.spawn(tx.clone(), w, &alive));
    }
    // The ordinal the next stream the ramp starts will carry, and the number of
    // `Msg::Left` that ends this job: every worker plus the ticker.
    let mut ordinals = threads;
    let mut expect_left = threads + 1;

    // The ramp's clock. It only wakes the coordinator, which owns the stream
    // state and is the only writer of SQLite in a job. The tick exists because a
    // host that has stopped answering also stops sending messages, and recovery
    // is the part of the rule that has to run exactly then.
    let spawner_tx = tx.clone();
    let ticker_tx = tx.clone();
    let ticker_alive = Arc::clone(&alive);
    let ticker_cancel = Arc::clone(cancel);
    let window = ctx.ramp.window_ms.max(TICK_POLL_MS as i64);
    let ticker = std::thread::spawn(move || {
        let _left = Left {
            tx: ticker_tx.clone(),
            alive: None,
        };
        let mut elapsed = 0i64;
        loop {
            // No streams left means the job is over; saying so is this thread's
            // last act, so the supervisor is never waiting on a tick that
            // cannot come.
            if ticker_alive.load(Ordering::SeqCst) == 0 || ticker_cancel.load(Ordering::SeqCst) != RUN {
                return;
            }
            std::thread::sleep(Duration::from_millis(TICK_POLL_MS));
            elapsed += TICK_POLL_MS as i64;
            if elapsed >= window {
                elapsed = 0;
                if ticker_tx.send(Msg::Tick(now_ms())).is_err() {
                    return;
                }
            }
        }
    });
    drop(tx);

    close_dangling_attempts(conn, id);

    let mut last_flush = Instant::now();
    let mut last_written = vec![u64::MAX; plan.len()];
    // (status, streams) of the last rate-limit decision written to history.
    let mut last_throttle: Option<(u16, usize)> = None;
    // Chunks finished since the last refusal; see `WIDEN_AFTER_CLEAN`.
    let mut clean = 0usize;
    // The ramp. `hard` is the band's top folded with what this file can use, so
    // a ceiling the plan could not fill never turns into an idle worker.
    let hard = band
        .ceiling
        .clamp(1, MAX_STREAMS)
        .min(max_viable_workers)
        .max(threads);
    let mut ramp = RampState::new(threads, hard, now_ms());
    let mut ramp_landed: u64 = coords.iter().map(|c| c.total_done.load(Ordering::SeqCst)).sum();
    let mut ramp_limited: u64 = coords
        .iter()
        .map(|c| c.limited_done.load(Ordering::SeqCst))
        .sum();
    // The adapter limits in force when the first window opens, so the ramp can tell
    // a window measured under one bond from a window the bond changed inside.
    let mut ramp_bond = bond_state(&ctx.networks, &env.route_nets);
    // The window the ramp is accumulating: where it started, and whether a limit moved
    // inside it. A tick the rule holds for being too young does not close the window,
    // so its bytes belong to the window that finally answers the growth bill; a shift
    // has to survive the hold for the same reason, or the bar is answered by a window
    // whose two halves were measured against different bonds.
    let mut ramp_window_ms = ramp.last_change_ms;
    let mut ramp_shifted = false;
    // (worker, block idx) -> (attempt row id, bytes already on disk when it started,
    // start ms, route index, range_start, range_end)
    let mut active: std::collections::HashMap<(usize, usize), (i64, u64, i64, usize, u64, u64)> =
        std::collections::HashMap::new();
    let mut gone = 0usize;
    // The supervisor keeps a sender so it can start a stream mid-job, which is
    // why the channel closing cannot be what ends this loop: it ends when every
    // thread that was started has reported in.
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Chunk { idx, done: bytes } => {
                if idx < done.len() {
                    done[idx] = bytes.min(plan[idx].len());
                }
            }
            Msg::Done { idx } => {
                if idx < done.len() {
                    done[idx] = plan[idx].len();
                }
                // A stretch of refused-free chunks is the evidence that the
                // limit was a burst rule, so the width creeps back by one — up
                // to the ceiling, and only as far as threads that exist.
                clean += 1;
                if clean >= WIDEN_AFTER_CLEAN {
                    gate.widen(ramp.ceiling.min(handles.len()));
                    clean = 0;
                }
            }
            Msg::TailSplit { idx, split_offset, bytes_left } => {
                if let Some(seen) = split.get_mut(idx) {
                    *seen = true;
                }
                history(
                    conn,
                    id,
                    "tail-split",
                    json!({
                        "block": idx,
                        "splitOffset": split_offset,
                        "bytesLeft": bytes_left
                    }),
                );
            }
            Msg::Claim {
                worker,
                idx,
                range_start,
                range_end,
                started_at,
                route,
            } => {
                let r = &routes[route % routes.len()];
                if let Some(live) = live {
                    live.claim_block(idx, &r.color);
                }
                let base = done.get(idx).copied().unwrap_or(0);
                if let Some(attempt_id) =
                    begin_attempt(conn, live, id, idx, range_start, range_end, started_at, r)
                {
                    active.insert((worker, idx), (attempt_id, base, started_at, route, range_start, range_end));
                }
            }
            Msg::Attempt {
                worker,
                idx,
                range_start,
                range_end,
                started_at,
                bytes,
                state,
                reason,
                detail,
                route,
            } => {
                let r = &routes[route % routes.len()];
                let attempt_id = match active.remove(&(worker, idx)) {
                    Some((attempt_id, _, _, _, _, _)) => attempt_id,
                    None => {
                        begin_attempt(conn, live, id, idx, range_start, range_end, started_at, r)
                            .unwrap_or(-1)
                    }
                };
                if attempt_id >= 0 {
                    finish_attempt(
                        conn,
                        live,
                        attempt_id,
                        idx,
                        range_start,
                        range_end,
                        started_at,
                        bytes,
                        state,
                        reason,
                        detail,
                        r,
                        &ctx.networks,
                    );
                }
            }
            Msg::Retry { idx, why } => {
                history(conn, id, "block-retry", json!({ "block": idx, "why": why }));
            }
            Msg::Throttle { idx, status, streams, previous, wait_secs } => {
                clean = 0;
                ramp.note_refusal(previous, now_ms());
                // One line per distinct decision, not one per refused block: a
                // host that rate-limits every chunk would otherwise bury the
                // activity log in identical rows.
                if last_throttle != Some((status, streams)) {
                    last_throttle = Some((status, streams));
                    history(
                        conn,
                        id,
                        "throttled",
                        json!({
                            "status": status,
                            "streams": streams,
                            "previous": previous,
                            "ceiling": ramp.ceiling,
                            "waitSecs": wait_secs,
                            "block": idx,
                        }),
                    );
                }
            }
            Msg::Fail { idx, code, why } => {
                let _ = conn.execute(
                    "UPDATE blocks SET state = 'failed' WHERE download_id = ?1 AND idx = ?2",
                    params![id, idx as i64],
                );
                if let Some(live) = live {
                    live.set_state(idx, "failed");
                }
                history(conn, id, "block-failed", json!({ "block": idx, "code": code, "why": why }));
                if cancel.load(Ordering::SeqCst) == RUN {
                    cancel.store(FAIL, Ordering::SeqCst);
                }
            }
            Msg::Restart => {
                if cancel.load(Ordering::SeqCst) == RUN {
                    cancel.store(RESTART, Ordering::SeqCst);
                }
            }
            Msg::Left => {
                gone += 1;
                if gone >= expect_left {
                    break;
                }
            }
            Msg::Tick(sent) => {
                let now = now_ms();
                // How long this tick sat in the channel. A large lag with a large span
                // is the coordinator being busy; a large span with no lag is the ticker
                // itself being late, and the two need different fixes.
                let lag = now.saturating_sub(sent);
                let stall = hedge_stall_ms();
                let split_floor = 2 * min_split_bytes();
                let mut landed = 0u64;
                let mut paced = 0u64;
                let mut pending = 0usize;
                let mut quiet = false;
                let mut stealable = false;
                for c in coords.iter() {
                    landed += c.total_done.load(Ordering::SeqCst);
                    paced += c.limited_done.load(Ordering::SeqCst);
                    if c.done_flag.load(Ordering::SeqCst) {
                        continue;
                    }
                    pending += 1;
                    if !c.active.load(Ordering::SeqCst) {
                        continue;
                    }
                    if (now - c.last_ms.load(Ordering::Relaxed)).max(0) as u64 >= stall {
                        quiet = true;
                    }
                    let pos = c.pos.load(Ordering::SeqCst);
                    let cut = c.cutoff.load(Ordering::SeqCst);
                    if ctx.policy.steals() && cut.saturating_sub(pos) >= split_floor {
                        stealable = true;
                    }
                }
                let delta = landed.saturating_sub(ramp_landed);
                let paced_delta = paced.saturating_sub(ramp_limited);
                // Which adapters carry a limit decides what the window is evidence about,
                // and a limit saved or met mid-window makes the two halves incomparable.
                let bond = bond_state(&ctx.networks, &env.route_nets);
                let shifted = ramp_shifted || ramp_bond != bond;
                ramp_bond = bond;
                let span = now.saturating_sub(ramp_window_ms).max(1);
                let sample = RampSample {
                    now_ms: now,
                    span_ms: span,
                    bytes_delta: delta,
                    // A job- or app-wide cap paces every byte, so the host set none of
                    // them; otherwise the ones that left under an adapter's cap are
                    // taken off and what the server itself handed over is left.
                    host_bytes: if ctx.limit.cap().is_some() || ctx.global.cap().is_some() {
                        0
                    } else {
                        delta.saturating_sub(paced_delta)
                    },
                    shifted,
                    width: gate.width(),
                    pending,
                    stealable,
                    quiet,
                };
                // Snapshot the price before the rule spends it, so a plateau row says
                // what was asked and what came back, not only that they disagreed.
                let open_bar = ramp.gain_bar;
                let action = next_width(&sample, &mut ramp, &ctx.ramp);
                // A window closes on any decision, and on a hold that was itself a
                // verdict about the window — silent, paced end to end, or measured
                // across a bond change. Only "too young to be a measurement" keeps
                // accumulating, because that is the hold the loaded transfer hits when
                // the coordinator drains a backlog of ticks 22-90 ms apart.
                if !matches!(action, RampAction::Hold) || quiet || shifted || sample.host_bytes == 0
                {
                    ramp_landed = landed;
                    ramp_limited = paced;
                    ramp_window_ms = now;
                    ramp_shifted = false;
                } else {
                    ramp_shifted = shifted;
                }
                match action {
                    RampAction::Hold => {}
                    RampAction::Plateau => {
                        history(
                            conn,
                            id,
                            "ramp-plateau",
                            json!({
                                "streams": sample.width,
                                "ceiling": ramp.ceiling,
                                "span": sample.span_ms,
                                "lag": lag,
                                "host": sample.host_bytes,
                                "bar": open_bar.unwrap_or(0),
                                "bar_span": ramp.gain_bar_span_ms,
                            }),
                        );
                    }
                    RampAction::Raise => {
                        gate.widen(ramp.ceiling.min(handles.len()));
                        history(
                            conn,
                            id,
                            "ramp-raise",
                            json!({ "ceiling": ramp.ceiling, "streams": gate.width() }),
                        );
                    }
                    RampAction::Grow => {
                        // A stream the gate allows has to exist first. The
                        // thread is only new when the budget has already used up
                        // the ones the job started with; after a halving the
                        // idle workers are still there, and widening alone is the
                        // whole step.
                        if sample.width + 1 > handles.len() && handles.len() < hard {
                            handles.push(env.spawn(spawner_tx.clone(), ordinals, &alive));
                            ordinals += 1;
                            expect_left += 1;
                        }
                        let to = gate.widen(ramp.ceiling.min(handles.len()));
                        history(
                            conn,
                            id,
                            "ramp-grow",
                            json!({
                                "from": sample.width,
                                "to": to,
                                "ceiling": ramp.ceiling,
                                "bytes": sample.bytes_delta,
                                "host": sample.host_bytes,
                                "span": sample.span_ms,
                                "lag": lag,
                            }),
                        );
                    }
                }
            }
        }
        if last_flush.elapsed().as_millis() >= PROGRESS_MS {
            let safe = resumable(&done, &resume, &split, &plan);
            flush_blocks(conn, id, &plan, &safe, &mut last_written);
            sync_live(live, &done);
            if let Some(live) = live {
                live.set_streams(gate.width());
                for ((_, idx), (attempt_id, base, _, _, _, _)) in &active {
                    let bytes = done.get(*idx).copied().unwrap_or(0).saturating_sub(*base);
                    live.update_active_attempt(*attempt_id, bytes);
                }
            }
            let sum: u64 = done.iter().sum();
            set_progress(conn, id, base + sum, Some(base + total))?;
            ctx.emit_blocks(id, live, false);
            ctx.emit(id);
            last_flush = Instant::now();
        }
    }

    // One line for the whole run: what the width reached, how many threads the
    // ramp started, and where the ceiling stands now. The activity log shows a
    // job's streams as a number the dial asked for; this is what actually ran.
    history(
        conn,
        id,
        "ramp-final",
        json!({
            "start": threads,
            "width": gate.width(),
            "workers": handles.len(),
            "ceiling": ramp.ceiling,
            "hard": hard,
            "plateau": ramp.plateau,
        }),
    );
    drop(spawner_tx);
    let _ = ticker.join();
    for h in handles {
        let _ = h.join();
    }
    for ((_w, idx), (attempt_id, base, started_at, route, r_start, r_end)) in active.drain() {
        let bytes = done.get(idx).copied().unwrap_or(0).saturating_sub(base);
        finish_attempt(
            conn,
            live,
            attempt_id,
            idx,
            r_start,
            r_end,
            started_at,
            bytes,
            "cancelled",
            None,
            None,
            &routes[route % routes.len()],
            &ctx.networks,
        );
    }
    let safe = resumable(&done, &resume, &split, &plan);
    flush_blocks(conn, id, &plan, &safe, &mut last_written);
    sync_live(live, &done);
    if let Some(live) = live {
        // One last reading, so the width a reader sees is the width `ramp-final`
        // recorded and not the one standing one step behind it.
        live.set_streams(gate.width());
    }
    // The row outlives the job and the registry does not. Without this a closed
    // card can only quote what the dial asked for, which is a promise the run may
    // not have kept — a rate-limited job that ended narrowed would read as though
    // it finished at full width. Same number as `ramp-final`, on disk.
    let _ = conn.execute(
        "UPDATE downloads SET streams_final = ?1 WHERE id = ?2",
        params![gate.width() as i64, id],
    );
    let sum: u64 = done.iter().sum();
    set_progress(conn, id, base + sum, Some(base + total))?;
    ctx.emit_blocks(id, live, true);
    ctx.emit(id);

    // Nothing is in flight once the workers are joined: unfinished blocks go
    // back to pending instead of claiming an activity that has stopped.
    if cancel.load(Ordering::SeqCst) != RUN {
        let _ = conn.execute(
            "UPDATE blocks SET state = 'pending' \
             WHERE download_id = ?1 AND state = 'active'",
            [id],
        );
    }

    match cancel.load(Ordering::SeqCst) {
        PAUSE => return Err(JobError::new("paused", "paused")),
        STOP => return Err(JobError::new("stopped", "stopped")),
        FAIL => {
            return Err(JobError::new(
                "block-failed",
                format!("{sum} of {total} bytes landed; the rest kept failing"),
            ))
        }
        RESTART => return Err(JobError::new("fallback", "server ignored the range request")),
        _ => {}
    }

    // A3: every block accounted for, and the sum is exactly the announced size.
    let mut covered = 0;
    for (i, b) in plan.iter().enumerate() {
        if done[i] != b.len() {
            return Err(JobError::new(
                "coverage-incomplete",
                format!("block {i} has {} of {} bytes", done[i], b.len()),
            ));
        }
        covered += done[i];
    }
    if covered != total {
        return Err(JobError::new(
            "coverage-mismatch",
            format!("blocks cover {covered} bytes, expected {total}"),
        ));
    }
    history(conn, id, "coverage-ok", json!({ "bytes": covered }));
    Ok(total)
}

/// The progress a block may be resumed from.
///
/// Two streams on one block add up, and a sum says nothing about where those
/// bytes sit: parent at its front, child at the back, and a hole in the middle
/// is still the right number of bytes. Paused or killed in that moment, the sum
/// would hand the next run a restart point past the hole, and the finished file
/// would be exactly as long as it is wrong. So a block that was split but has
/// not actually finished keeps the point this run started from, and pays for it
/// with one block re-fetched. A split that ran to the end needs nothing special
/// — both halves are on disk, and `done` equals the block length.
fn resumable(done: &[u64], resume: &[u64], split: &[bool], plan: &[BlockSpec]) -> Vec<u64> {
    plan.iter()
        .enumerate()
        .map(|(i, b)| {
            if split.get(i).copied().unwrap_or(false) && done[i] < b.len() {
                resume[i]
            } else {
                done[i]
            }
        })
        .collect()
}

fn flush_blocks(conn: &Connection, id: i64, plan: &[BlockSpec], done: &[u64], last: &mut [u64]) {
    for (i, b) in plan.iter().enumerate() {
        if last[i] == done[i] {
            continue;
        }
        last[i] = done[i];
        let state = if done[i] == b.len() { "done" } else { "active" };
        let _ = conn.execute(
            "UPDATE blocks SET bytes_done = ?1, state = ?2 WHERE download_id = ?3 AND idx = ?4",
            params![done[i] as i64, state, id, b.idx as i64],
        );
    }
}

/// How many refused-free chunks must land before the gate dares add a stream.
/// Four is a probe, not a restoration: a host that limits bursts gets re-tested
/// every four chunks instead of being re-flooded at once.
const WIDEN_AFTER_CLEAN: usize = 4;

/// What one ramp window measured. Built by the ramp thread, read by
/// [`next_width`], which holds no clock and no socket so the whole rule is
/// testable with literals.
#[derive(Clone, Copy, Debug)]
pub struct RampSample {
    pub now_ms: i64,
    /// How long this window actually measured, from the end of the last window the ramp
    /// looked at. The ticker is supposed to fire once per `window_ms`, but a loaded
    /// transfer lets ticks queue, so a window can be a full second or 22 ms — and byte
    /// totals from windows of different length are not two of the same measurement.
    pub span_ms: i64,
    /// Bytes that landed inside this window, however they were paced.
    pub bytes_delta: u64,
    /// The part of `bytes_delta` that no limiter set: what the server handed over on
    /// its own, and the only throughput figure the ramp may read as host evidence.
    /// Zero means the window was paced end to end — by a job or app cap, or by a cap on
    /// every adapter a route left on — which is the one boolean `capped` used to cover.
    pub host_bytes: u64,
    /// An adapter gained a cap, met its allowance or lost a limit inside this window, so
    /// its two halves were not measured against the same bond.
    pub shifted: bool,
    /// Streams running now.
    pub width: usize,
    /// Blocks not finished yet.
    pub pending: usize,
    /// Another block could still be split off a straggler's tail.
    pub stealable: bool,
    /// A live stream has gone silent, so this window says nothing about the host.
    pub quiet: bool,
}

/// What the ramp remembers between windows.
#[derive(Clone, Copy, Debug)]
pub struct RampState {
    /// The width the job opened at. Equal to `hard` for a pinned band, and then
    /// the ramp has nothing it is allowed to add.
    pub start: usize,
    /// The top the ramp may widen to. A refusal pulls it down; a long clean
    /// stretch pushes it back up one at a time.
    pub ceiling: usize,
    /// The band's outer edge, which recovery may climb back to and never past.
    pub hard: usize,
    /// The byte total the window after a growth has to reach, priced from the
    /// width that was in force when the step was taken — see [`step_bill`].
    /// Miss it and the ramp stops growing for the rest of the job.
    pub gain_bar: Option<u64>,
    /// The length of the window that priced `gain_bar`. The ask is a rate over this
    /// span, not a total: the window that answers it is only promised to be a full
    /// `window_ms` old, and a first window that swallowed a tick backlog can measure
    /// seconds. Comparing the two totals would plateau a job whose only crime was a
    /// busy coordinator.
    pub gain_bar_span_ms: i64,
    pub last_change_ms: i64,
    /// Both clocks restart on a refusal, so a host that keeps saying no is not
    /// re-tested at the same rate as one that has been quiet for a minute.
    pub last_raise_ms: i64,
    pub plateau: bool,
}

/// The ramp's clocks and its appetite for risk. Production runs the default;
/// the engine tests hand in a fast one rather than overriding a shipped
/// constant through the environment.
#[derive(Clone, Copy, Debug)]
pub struct RampTuning {
    pub window_ms: i64,
    /// How much of a *fair share* the new stream has to earn, in percent. At the
    /// default 50 a ninth stream must bring at least half of what an eighth of
    /// the throughput it already has would predict, which is a smaller absolute
    /// ask the wider the job runs — see [`step_bill`].
    pub share_pct: u64,
    pub refuse_cool_ms: i64,
}

impl Default for RampTuning {
    fn default() -> Self {
        RampTuning {
            window_ms: 1_000,
            share_pct: 50,
            refuse_cool_ms: 60_000,
        }
    }
}

/// Price the window that follows a growth.
///
/// A fair host scales `w -> w+1` as `(w+1) / w`, so the gain one extra stream
/// can show *shrinks* as the ladder climbs: +12.5% at width 8, +6.25% at 16,
/// +3.2% at 31. A flat percentage therefore cannot gate additive growth, and the
/// first cut of this rule proved it in the shipped app: with the bar pinned at
/// 15%, a 96 MiB job grew exactly once, to 9, and plateaued there while every
/// stream was still paying in full — the dial's 32 was decoration.
///
/// Scaling the ask by the step keeps the invariant that made this better than
/// Plexo's rule (a stream has to *move bytes*, not merely answer cleanly) and
/// makes it reachable all the way to the cap: half a share asks +6.25% at width
/// 8 and +1.6% at width 31. A host that caps the client instead of the connection
/// brings the total up by nothing, so it plateaus on the first step — the case
/// Plexo cannot see at all.
///
/// The trailing `+ 1` is what keeps `share_pct: 0` honest: a window that added
/// not one byte is no evidence, whatever the tuning says.
fn step_bill(bytes: u64, width: usize, share_pct: u64) -> u64 {
    let width = width.max(1) as u64;
    let extra = bytes.saturating_mul(share_pct) / (100 * width);
    bytes.saturating_add(extra).saturating_add(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RampAction {
    Hold,
    /// One more stream, and a worker to run it.
    Grow,
    /// The ceiling climbs back by one after a minute without a refusal.
    Raise,
    /// The last growth bought nothing: stop growing for the rest of the job.
    Plateau,
}

/// The ramp's whole rule, with no threads and no sockets in it.
///
/// Growth is additive — one stream per window — and every step has to pay for
/// itself. Plexo doubles 8 to 32 in two 500 ms ticks on one condition: that
/// every stream answered. It never asks whether the extra streams moved a
/// single extra byte, which is exactly the question a per-client cap makes
/// expensive, because a host that throttles the client rather than the
/// connection pays for 24 useless sockets and hands back a rate limit.
///
/// So the evidence required here is stricter than Plexo's and the cadence is
/// slower on purpose: a job measured in minutes loses nothing to a 24-second
/// climb, and a job that would have flooded a capped host loses the flood.
pub fn next_width(sample: &RampSample, state: &mut RampState, tuning: &RampTuning) -> RampAction {
    // The one unfair window that also buys back a bar it never disproved. An adapter
    // ejected or a cap saved mid-window shrinks the bond, so the survivors' bytes alone
    // would plateau the job against a price the old, wider bond paid. The bar is voided
    // and the next window is judged under the bond that actually exists.
    if sample.shifted {
        state.gain_bar = None;
        return RampAction::Hold;
    }
    // An unfair window — silent, paced end to end, or one where nothing landed at all,
    // which is the same reading — is no evidence about the host, and it does not get to
    // spend the bar either.
    if sample.quiet || sample.host_bytes == 0 || state.plateau {
        return RampAction::Hold;
    }
    // Two clocks, both of which have to be answered before anything is judged: the
    // cadence since the last action, and how long this window actually measured. These
    // sit above the bill because the 2026-11-04 launched run found the coordinator
    // draining a backlog of ticks 22-90 ms apart after a first window that swallowed
    // seconds of the transfer; the bar branch used to run before any of this, so that
    // sliver answered the growth bill with 200 KB, latched `plateau`, and Auto stopped at
    // nine streams in every uncapped pass.
    if sample.now_ms - state.last_change_ms < tuning.window_ms || sample.span_ms < tuning.window_ms
    {
        return RampAction::Hold;
    }
    // The bill for the last growth, judged as a rate rather than as a total. The window
    // answering it is only promised to be a full `window_ms` long, while the window that
    // priced it may have measured seconds behind a busy coordinator; comparing totals
    // would ask a 1 s window to produce a 3.4 s window's bytes. Equal spans — every
    // window a healthy job produces — make this the same arithmetic the totals were.
    if let Some(bar) = state.gain_bar {
        state.gain_bar = None;
        let paid = sample.host_bytes.saturating_mul(state.gain_bar_span_ms.max(1) as u64);
        let asked = bar.saturating_mul(sample.span_ms.max(1) as u64);
        if paid < asked {
            state.plateau = true;
            return RampAction::Plateau;
        }
    }
    if state.ceiling < state.hard {
        // A dropped ceiling only climbs back a minute after the last refusal —
        // the same +1-per-minute Plexo recovers with, but spent on the ceiling
        // rather than the width, so a host that said no is not re-flooded.
        if sample.now_ms - state.last_raise_ms < tuning.refuse_cool_ms {
            return RampAction::Hold;
        }
        state.last_raise_ms = sample.now_ms;
        state.last_change_ms = sample.now_ms;
        state.ceiling += 1;
        return RampAction::Raise;
    }
    if sample.width >= state.ceiling || sample.width >= state.hard {
        return RampAction::Hold;
    }
    // Work for the new stream: a block nobody has claimed, or a straggler whose
    // tail the stealing loop could hand over. Two streams per unfinished block
    // is the most a block can use, and Plexo plans for the same ratio.
    if sample.pending == 0 && !sample.stealable {
        return RampAction::Hold;
    }
    if sample.width >= sample.pending.saturating_mul(2) + usize::from(sample.stealable) {
        return RampAction::Hold;
    }
    state.last_change_ms = sample.now_ms;
    // The bill for the next window, priced from the width that is about to
    // become last month's. A refusal clears it (see `note_refusal`), so the first
    // step back up is the one step a job gets without proving the bytes — because
    // the window a halving produced says nothing about either.
    state.gain_bar_span_ms = sample.span_ms.max(1);
    state.gain_bar = Some(step_bill(sample.host_bytes, sample.width, tuning.share_pct));
    RampAction::Grow
}

impl RampState {
    /// A job opens at `start` streams and may be taken up to `hard` by the
    /// ramp. `hard` already folds in what the file can actually use, so a
    /// ceiling the plan cannot fill never shows up as an idle worker.
    fn new(start: usize, hard: usize, now: i64) -> Self {
        let hard = hard.max(start).max(1);
        RampState {
            ceiling: hard,
            hard,
            start,
            gain_bar: None,
            gain_bar_span_ms: 0,
            last_change_ms: now,
            last_raise_ms: now,
            plateau: false,
        }
    }

    /// A refusal says the width that was in force was too much for this host.
    /// The ceiling drops just under it and both recovery clocks restart; the
    /// width itself is the gate's business, and it has already halved.
    ///
    /// A band the user pinned keeps its ceiling: a dial set to 4 is a promise,
    /// and the only thing a refusal may take away there is how many of the four
    /// may run at once, which is what the gate already did.
    fn note_refusal(&mut self, previous: usize, now: i64) {
        if self.start < self.hard {
            self.ceiling = self.ceiling.min(previous.saturating_sub(1).max(1));
        }
        self.last_raise_ms = now;
        self.last_change_ms = now;
        self.gain_bar = None;
    }
}

/// How many workers may hold an open request at once.
///
/// Threads are spawned for the whole job, so narrowing the concurrency means
/// gating who may start a transfer, not killing anyone. A host that answers 429
/// gets a smaller number rather than merely a slower one: re-opening the same
/// width of connections is exactly what the rate limit exists to stop, and it is
/// why the 2026-11-01 OVH job died as "the rest kept failing".
struct Gate {
    active: AtomicUsize,
    budget: AtomicUsize,
}

impl Gate {
    fn new(budget: usize) -> Self {
        Gate {
            active: AtomicUsize::new(0),
            budget: AtomicUsize::new(budget.max(1)),
        }
    }

    fn try_acquire(&self) -> bool {
        loop {
            let active = self.active.load(Ordering::SeqCst);
            if active >= self.budget.load(Ordering::SeqCst) {
                return false;
            }
            if self
                .active
                .compare_exchange(active, active + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Halves the width, floored at one so a single rate limit cannot stall the
    /// job. Returns the width it was called from and the new one: the ramp needs
    /// the first to know how far above what the host tolerated it had climbed.
    fn halve(&self) -> (usize, usize) {
        loop {
            let budget = self.budget.load(Ordering::SeqCst);
            let next = (budget / 2).max(1);
            if next == budget {
                return (budget, next);
            }
            if self
                .budget
                .compare_exchange(budget, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return (budget, next);
            }
        }
    }

    /// One stream back, never past what the job asked for. A limit is usually a
    /// burst rule rather than a permanent sentence, and a job that halved once
    /// at the top of a two-hour file would otherwise run the whole way on one
    /// connection — which is slower than the un-limited engine it replaced.
    fn widen(&self, cap: usize) -> usize {
        loop {
            let budget = self.budget.load(Ordering::SeqCst);
            let next = (budget + 1).min(cap.max(1));
            if next == budget {
                return budget;
            }
            if self
                .budget
                .compare_exchange(budget, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return next;
            }
        }
    }

    fn width(&self) -> usize {
        self.budget.load(Ordering::SeqCst)
    }
}

/// A held slot, released on every exit path — including the early `return` a
/// pause or a cancelled attempt takes.
struct Slot<'g>(&'g Gate);

impl<'g> Slot<'g> {
    /// Waits for a slot, giving up if the job is paused or cancelled meanwhile.
    fn wait(gate: &'g Gate, cancel: &AtomicU8) -> Option<Self> {
        loop {
            if cancel.load(Ordering::SeqCst) != RUN {
                return None;
            }
            if gate.try_acquire() {
                return Some(Slot(gate));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Everything a worker needs, held together so the ramp can start one in the
/// middle of a job. Cloning it costs handles only: the plan is shared, and the
/// only real copies are a URL, a referer and maybe an ETag.
///
/// The alternative was to spawn the whole ceiling on day one and gate it, which
/// is what the gate is for anyway. It would also park up to 24 threads per
/// download polling for a slot the host has not earned, and a paused job would
/// keep them parked; a thread that exists is a stack and a wake-up, so the ramp
/// pays for a thread only when the bytes have asked for one.
#[derive(Clone)]
struct WorkerEnv {
    agents: Arc<Vec<ureq::Agent>>,
    url: String,
    etag: Option<String>,
    referer: String,
    cookie: Option<String>,
    plan: Arc<Vec<BlockSpec>>,
    coords: Arc<Vec<BlockCoord>>,
    file: Arc<File>,
    next: Arc<AtomicUsize>,
    gate: Arc<Gate>,
    cancel: Arc<AtomicU8>,
    limit: Arc<Bucket>,
    global: Arc<Bucket>,
    networks: NetworkScopes,
    /// Which adapter each route index leaves on, frozen with the routes. Only the
    /// cap is re-read per attempt; the device a block belongs to never moves.
    route_nets: Vec<Option<i64>>,
    policy: BondPolicy,
}

impl WorkerEnv {
    /// Starts one stream. `alive` counts the workers still running so the ramp's
    /// ticker can leave before it is the only thing keeping the job waiting.
    fn spawn(
        &self,
        tx: mpsc::Sender<Msg>,
        ordinal: usize,
        alive: &Arc<AtomicUsize>,
    ) -> std::thread::JoinHandle<()> {
        let env = self.clone();
        let alive = Arc::clone(alive);
        alive.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let _left = Left {
                tx: tx.clone(),
                alive: Some(alive),
            };
            worker(ordinal, tx, env);
        })
    }
}

/// Tells the supervisor this thread has finished, and stops counting as a
/// stream. In a `Drop` because a worker that panics on the way out still has to
/// be accounted for — the same unwind that skips the rest of its body runs this.
struct Left {
    tx: mpsc::Sender<Msg>,
    alive: Option<Arc<AtomicUsize>>,
}

impl Drop for Left {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Left);
        if let Some(alive) = &self.alive {
            alive.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn worker(ordinal: usize, tx: mpsc::Sender<Msg>, env: WorkerEnv) {
    let WorkerEnv {
        agents, url, etag, referer, cookie, plan, coords, file, next, gate, cancel, limit,
        global, networks, route_nets, policy,
    } = env;
    loop {
        if cancel.load(Ordering::SeqCst) != RUN {
            return;
        }
        // Only as many workers as the gate allows may hold a request; the rest
        // wait here rather than opening a connection the host just refused.
        let Some(_slot) = Slot::wait(&gate, &cancel) else {
            return;
        };
        let i = next.fetch_add(1, Ordering::SeqCst);
        if i >= plan.len() {
            break;
        }
        let block = plan[i];
        if coords[i].done_flag.load(Ordering::SeqCst) {
            let _ = tx.send(Msg::Done { idx: i });
            continue;
        }

        coords[i].active.store(true, Ordering::SeqCst);
        // The hedge clock starts at the claim, not at job planning: a block
        // claimed late must not look stalled because its neighbours moved
        // before it was ever asked for.
        coords[i].last_ms.store(now_ms(), Ordering::Relaxed);
        let mut attempt = 0u32;
        loop {
            if cancel.load(Ordering::SeqCst) != RUN {
                coords[i].active.store(false, Ordering::SeqCst);
                return;
            }
            let Some(route) = open_route(
                policy.route_for(ordinal, attempt, agents.len()),
                agents.len(),
                false,
                &route_nets,
                &networks,
            ) else {
                // Every device this job may use has met its allowance. Wait for the
                // mirror to move rather than spending an attempt, failing the
                // download, or spending bytes the user said were spent. The cancel
                // check at the top of this loop is what makes the wait interruptible.
                std::thread::sleep(Duration::from_millis(TICK_POLL_MS));
                continue;
            };
            let agent = &agents[route];
            let started_at = now_ms();
            let mut done = coords[i].pos.load(Ordering::SeqCst);
            let attempt_from = done;
            let req_start = block.start + done;
            let req_end = block.end;
            let _ = tx.send(Msg::Claim {
                worker: ordinal,
                idx: i,
                range_start: req_start,
                range_end: req_end,
                started_at,
                route,
            });
            let report_tx = tx.clone();
            let cancel_check = Arc::clone(&cancel);
            let coords_ref = Arc::clone(&coords);
            let net = live_cap(&networks, route_nets.get(route).copied().flatten());
            // Bytes that left under a rate cap are the limiter's answer, not the
            // host's, so they are booked twice: once as throughput, once as evidence
            // the ramp has to discount.
            let paced = net.as_ref().is_some_and(|bucket| bucket.cap().is_some());
            let mut report = |delta: u64| {
                if cancel_check.load(Ordering::SeqCst) != RUN {
                    return;
                }
                coords_ref[i].pos.fetch_add(delta, Ordering::SeqCst);
                coords_ref[i].last_ms.store(now_ms(), Ordering::Relaxed);
                if paced {
                    coords_ref[i].limited_done.fetch_add(delta, Ordering::SeqCst);
                }
                let total = coords_ref[i].total_done.fetch_add(delta, Ordering::SeqCst) + delta;
                let _ = report_tx.send(Msg::Chunk { idx: i, done: total });
            };
            match fetch_block(
                agent,
                &url,
                etag.as_deref(),
                &referer,
                cookie.as_deref(),
                block,
                &mut done,
                Some(&coords[i].cutoff),
                &file,
                &cancel,
                &limit,
                &global,
                net.as_deref(),
                &mut report,
            ) {
                Ok(()) => {
                    let bytes = done.saturating_sub(attempt_from);
                    let _ = tx.send(Msg::Attempt {
                        worker: ordinal,
                        idx: i,
                        range_start: req_start,
                        range_end: req_end,
                        started_at,
                        bytes,
                        state: "done",
                        reason: None,
                        detail: None,
                        route,
                    });
                    coords[i].active.store(false, Ordering::SeqCst);
                    if coords[i].total_done.load(Ordering::SeqCst) >= block.len() {
                        coords[i].done_flag.store(true, Ordering::SeqCst);
                        let _ = tx.send(Msg::Done { idx: i });
                    }
                    break;
                }
                Err(FetchError::Cancelled) => {
                    let bytes = done.saturating_sub(attempt_from);
                    let _ = tx.send(Msg::Attempt {
                        worker: ordinal,
                        idx: i,
                        range_start: req_start,
                        range_end: req_end,
                        started_at,
                        bytes,
                        state: "cancelled",
                        reason: None,
                        detail: None,
                        route,
                    });
                    coords[i].active.store(false, Ordering::SeqCst);
                    return;
                }
                Err(FetchError::Restart) => {
                    let bytes = done.saturating_sub(attempt_from);
                    let _ = tx.send(Msg::Attempt {
                        worker: ordinal,
                        idx: i,
                        range_start: req_start,
                        range_end: req_end,
                        started_at,
                        bytes,
                        state: "restart",
                        reason: Some("server ignored the range request".into()),
                        detail: None,
                        route,
                    });
                    coords[i].active.store(false, Ordering::SeqCst);
                    let _ = tx.send(Msg::Restart);
                    return;
                }
                Err(FetchError::Retry { why, status, wait }) => {
                    let last = attempt + 1 >= MAX_ATTEMPTS;
                    let backoff = wait.unwrap_or(Duration::from_millis(250u64 << attempt.min(4)));
                    let try_no = attempt + 1;
                    let (chip, why_str) = match status {
                        Some(code @ (429 | 503)) => {
                            let (previous, streams) = gate.halve();
                            let _ = tx.send(Msg::Throttle {
                                idx: i,
                                status: code,
                                streams,
                                previous,
                                wait_secs: backoff.as_secs(),
                            });
                            let noun = if streams == 1 { "stream" } else { "streams" };
                            (
                                format!("{code} · {streams} {noun} · {}s", backoff.as_secs()),
                                format!(
                                    "server answered {code}, holding {streams} {noun} for {}s",
                                    backoff.as_secs()
                                ),
                            )
                        }
                        Some(code) => (
                            format!("{code} · retry {try_no}/{MAX_ATTEMPTS}"),
                            format!(
                                "server answered {code}, retry {try_no}/{MAX_ATTEMPTS} in {}ms: {why}",
                                backoff.as_millis()
                            ),
                        ),
                        None => (
                            format!("retry {try_no}/{MAX_ATTEMPTS}"),
                            format!(
                                "retry {try_no}/{MAX_ATTEMPTS} in {}ms: {why}",
                                backoff.as_millis()
                            ),
                        ),
                    };
                    let bytes = done.saturating_sub(attempt_from);
                    let _ = tx.send(Msg::Attempt {
                        worker: ordinal,
                        idx: i,
                        range_start: req_start,
                        range_end: req_end,
                        started_at,
                        bytes,
                        state: if last { "failed" } else { "retry" },
                        reason: Some(chip),
                        detail: Some(why_str),
                        route,
                    });
                    let retry_note = match status {
                        Some(code @ (429 | 503)) => format!("rate-limited (HTTP {code})"),
                        _ => why.clone(),
                    };
                    if last {
                        coords[i].active.store(false, Ordering::SeqCst);
                        let _ = tx.send(Msg::Fail {
                            idx: i,
                            code: "block-retry-exhausted",
                            why,
                        });
                        return;
                    }
                    let _ = tx.send(Msg::Retry { idx: i, why: retry_note });
                    std::thread::sleep(backoff);
                    attempt += 1;
                }
            }
        }
    }

    // Work-stealing phase: all initial blocks are claimed; steal straggler tails!
    loop {
        if cancel.load(Ordering::SeqCst) != RUN {
            return;
        }
        if coords.iter().all(|c| c.done_flag.load(Ordering::SeqCst)) {
            return;
        }
        let Some(_slot) = Slot::wait(&gate, &cancel) else {
            return;
        };
        if let Some((k, mid, end, rem)) = try_steal_tail(&coords, &plan, policy) {
            let block = plan[k];
            let _ = tx.send(Msg::TailSplit {
                idx: k,
                split_offset: block.start + mid,
                bytes_left: rem,
            });
            let mut attempt = 0u32;
            let mut done_in_slice = 0u64;
            let slice_block = BlockSpec {
                idx: k,
                start: block.start + mid,
                end: block.start + end - 1,
            };
            loop {
                if cancel.load(Ordering::SeqCst) != RUN {
                    return;
                }
                let preferred = if policy.hedges() {
                    // The half being raced must leave on a different device than
                    // the stream that went quiet, or the hedge re-opens the stall.
                    policy.secondary_route_for(ordinal, attempt, agents.len())
                } else {
                    policy.route_for(ordinal, attempt, agents.len())
                };
                let Some(route) = open_route(
                    preferred,
                    agents.len(),
                    policy.hedges(),
                    &route_nets,
                    &networks,
                ) else {
                    std::thread::sleep(Duration::from_millis(TICK_POLL_MS));
                    continue;
                };
                let agent = &agents[route];
                let started_at = now_ms();
                let req_start = slice_block.start + done_in_slice;
                let req_end = slice_block.end;
                let attempt_from = done_in_slice;
                let _ = tx.send(Msg::Claim {
                    worker: ordinal,
                    idx: k,
                    range_start: req_start,
                    range_end: req_end,
                    started_at,
                    route,
                });
                let report_tx = tx.clone();
                let cancel_check = Arc::clone(&cancel);
                let coords_ref = Arc::clone(&coords);
                let net = live_cap(&networks, route_nets.get(route).copied().flatten());
                let paced = net.as_ref().is_some_and(|bucket| bucket.cap().is_some());
                let mut report = |delta: u64| {
                    if cancel_check.load(Ordering::SeqCst) != RUN {
                        return;
                    }
                    coords_ref[k].last_ms.store(now_ms(), Ordering::Relaxed);
                    if paced {
                        coords_ref[k].limited_done.fetch_add(delta, Ordering::SeqCst);
                    }
                    let total = coords_ref[k].total_done.fetch_add(delta, Ordering::SeqCst) + delta;
                    let _ = report_tx.send(Msg::Chunk { idx: k, done: total });
                };
                match fetch_block(
                    agent,
                    &url,
                    etag.as_deref(),
                    &referer,
                    cookie.as_deref(),
                    slice_block,
                    &mut done_in_slice,
                    None,
                    &file,
                    &cancel,
                    &limit,
                    &global,
                    net.as_deref(),
                    &mut report,
                ) {
                    Ok(()) => {
                        let bytes = done_in_slice.saturating_sub(attempt_from);
                        let _ = tx.send(Msg::Attempt {
                            worker: ordinal,
                            idx: k,
                            range_start: req_start,
                            range_end: req_end,
                            started_at,
                            bytes,
                            state: "done",
                            reason: None,
                            detail: None,
                            route,
                        });
                        if coords[k].total_done.load(Ordering::SeqCst) >= block.len() {
                            coords[k].done_flag.store(true, Ordering::SeqCst);
                            let _ = tx.send(Msg::Done { idx: k });
                        }
                        break;
                    }
                    Err(FetchError::Cancelled) => {
                        let bytes = done_in_slice.saturating_sub(attempt_from);
                        let _ = tx.send(Msg::Attempt {
                            worker: ordinal,
                            idx: k,
                            range_start: req_start,
                            range_end: req_end,
                            started_at,
                            bytes,
                            state: "cancelled",
                            reason: None,
                            detail: None,
                            route,
                        });
                        return;
                    }
                    Err(FetchError::Restart) => {
                        let _ = tx.send(Msg::Restart);
                        return;
                    }
                    Err(FetchError::Retry { why, status, wait }) => {
                        let last = attempt + 1 >= MAX_ATTEMPTS;
                        let backoff = wait.unwrap_or(Duration::from_millis(250u64 << attempt.min(4)));
                        let try_no = attempt + 1;
                        let chip = match status {
                            Some(code) => format!("{code} · retry {try_no}/{MAX_ATTEMPTS}"),
                            None => format!("retry {try_no}/{MAX_ATTEMPTS}"),
                        };
                        let bytes = done_in_slice.saturating_sub(attempt_from);
                        let _ = tx.send(Msg::Attempt {
                            worker: ordinal,
                            idx: k,
                            range_start: req_start,
                            range_end: req_end,
                            started_at,
                            bytes,
                            state: if last { "failed" } else { "retry" },
                            reason: Some(chip),
                            detail: Some(why.clone()),
                            route,
                        });
                        let retry_note = match status {
                            Some(code @ (429 | 503)) => format!("rate-limited (HTTP {code})"),
                            _ => why.clone(),
                        };
                        if last {
                            let _ = tx.send(Msg::Fail {
                                idx: k,
                                code: "block-retry-exhausted",
                                why,
                            });
                            return;
                        }
                        let _ = tx.send(Msg::Retry { idx: k, why: retry_note });
                        std::thread::sleep(backoff);
                        attempt += 1;
                    }
                }
            }
            continue;
        }
        if coords.iter().all(|c| c.done_flag.load(Ordering::SeqCst)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// After a failed read, the bytes that reached the file are the block's
/// progress: `fetch_block` advances the offset as it writes, so the retry
/// resumes there instead of starting the block over.
enum FetchError {
    /// `status` is what the host answered, when it answered at all. `wait` is
    /// its own `Retry-After`; only a 429 or 503 sets it, and only those narrow
    /// the gate.
    Retry {
        why: String,
        status: Option<u16>,
        wait: Option<Duration>,
    },
    Restart,
    Cancelled,
}

impl FetchError {
    fn retry(e: impl std::fmt::Display) -> Self {
        FetchError::Retry {
            why: e.to_string(),
            status: None,
            wait: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fetch_block(
    agent: &ureq::Agent,
    url: &str,
    etag: Option<&str>,
    referer: &str,
    cookie: Option<&str>,
    block: BlockSpec,
    offset: &mut u64,
    cutoff: Option<&AtomicU64>,
    file: &File,
    cancel: &Arc<AtomicU8>,
    limit: &Bucket,
    global: &Bucket,
    net: Option<&Bucket>,
    report: &mut dyn FnMut(u64),
) -> Result<(), FetchError> {
    let end = block.end;
    let mut req = agent
        .get(url)
        .header("Accept-Encoding", "identity")
        .header("Referer", referer)
        .header("Range", format!("bytes={}-{}", block.start + *offset, end));
    if let Some(tag) = etag {
        req = req.header("If-Range", tag);
    }
    if let Some(cookie) = cookie {
        req = req.header("Cookie", cookie);
    }
    let resp = req.call().map_err(FetchError::retry)?;
    let status = resp.status().as_u16();
    match status {
        206 => {}
        200 => {
            // A whole body where a range was asked for: this server will not
            // do ranged transfers, so the job restarts as a single stream.
            return Err(FetchError::Restart);
        }
        416 => {
            return Err(FetchError::Retry {
                why: "server answered 416".into(),
                status: Some(416),
                wait: None,
            });
        }
        429 | 503 => {
            // The host is asking for a slower client, and usually says how much
            // slower. Both are honoured: retrying a rate limit at the same width
            // of connections is what turns one refused block into a dead job.
            let wait = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| retry_after_secs(v, now_ms() / 1000))
                .map(|secs| Duration::from_secs(secs.max(1) as u64));
            return Err(FetchError::Retry {
                why: format!("server answered {status}"),
                status: Some(status),
                wait,
            });
        }
        400..=599 => {
            return Err(FetchError::Retry {
                why: format!("server answered {status}"),
                status: Some(status),
                wait: None,
            });
        }
        _ => {
            return Err(FetchError::Retry {
                why: format!("unexpected status {status}"),
                status: Some(status),
                wait: None,
            })
        }
    }

    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut buf = vec![0u8; CHUNK];
    // The caps this block reads under. `read_slice` and `charge` take a slice of
    // buckets, so an optional adapter cap has to join it rather than sit beside
    // it. The third slot is only in range while `net` is Some, so the `limit`
    // written there otherwise never reaches a charge.
    let mut caps = [limit, global, limit];
    let caps: &[&Bucket] = match net {
        Some(net) => {
            caps[2] = net;
            &caps
        }
        None => &caps[..2],
    };
    let target_len = block.len();
    loop {
        if cancel.load(Ordering::SeqCst) != RUN {
            return Err(FetchError::Cancelled);
        }
        if let Some(cut) = cutoff {
            let max_offset = cut.load(Ordering::SeqCst);
            if *offset >= max_offset {
                break;
            }
        }
        let slice = limit::read_slice(caps);
        let n = match reader.read(&mut buf[..slice]) {
            Ok(n) => n,
            Err(e) => return Err(FetchError::retry(e)),
        };
        if n == 0 {
            break;
        }
        let mut to_write = n;
        if let Some(cut) = cutoff {
            let max_offset = cut.load(Ordering::SeqCst);
            if *offset + to_write as u64 > max_offset {
                to_write = max_offset.saturating_sub(*offset) as usize;
            }
        }
        if to_write > 0 {
            if limit::charge(caps, to_write, cancel).is_err() {
                return Err(FetchError::Cancelled);
            }
            write_at(file, &buf[..to_write], block.start + *offset).map_err(FetchError::retry)?;
            *offset += to_write as u64;
            report(to_write as u64);
        }
        if let Some(cut) = cutoff {
            let max_offset = cut.load(Ordering::SeqCst);
            if *offset >= max_offset {
                break;
            }
        }
    }
    let expected_len = if let Some(cut) = cutoff {
        cut.load(Ordering::SeqCst).min(target_len)
    } else {
        target_len
    };
    if *offset != expected_len {
        return Err(FetchError::retry(format!(
            "short body: {} of {} bytes",
            offset,
            expected_len
        )));
    }
    Ok(())
}

fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    let mut written = 0usize;
    while written < buf.len() {
        let n = write_some_at(file, &buf[written..], offset + written as u64)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "positional write wrote 0"));
        }
        written += n;
    }
    Ok(())
}

#[cfg(windows)]
fn write_some_at(file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_write(buf, offset)
}

#[cfg(not(windows))]
fn write_some_at(file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.write_at(buf, offset)
}

#[cfg(windows)]
fn read_some_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}

#[cfg(not(windows))]
fn read_some_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    let mut read = 0usize;
    while read < buf.len() {
        let n = read_some_at(file, &mut buf[read..], offset + read as u64)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "positional read hit the end of the partial",
            ));
        }
        read += n;
    }
    Ok(())
}

/// Bytes read per sample window, and how many windows a verdict is made of.
const SAMPLE_WINDOW: u64 = 16 * 1024;
const SAMPLE_COUNT: usize = 8;

/// The byte ranges this row's own ledger says it holds, as `(start, len)`.
///
/// A stream writes its block from the block's front, so `bytes_done` bytes are
/// on disk there — finished block or not. Requiring an unbroken run of *finished*
/// blocks from the first byte was the first shape of this helper, and the
/// launched exe measured what that means: a job widened to eight streams on eight
/// blocks leaves all eight half-full rather than four done, so at a pause 3.5 MB
/// into 8 MB the run answered zero and every real hearing ended `no-evidence`.
///
/// Two things narrow the ledger before it is trusted. A range the file does not
/// actually reach is cut to the file, because counted bytes with no bytes behind
/// them are the trap `verified_bytes` has always been. And a range shorter than a
/// window is dropped, since it cannot be read whole.
///
/// A block that was handed a tail can carry its count with a hole in the middle;
/// the bytes in that hole are the zeros the file was preallocated with, so a read
/// there disagrees with any host's bytes and the verdict lands on `differed` —
/// the direction that keeps the decision the validator already made.
fn covered_ranges(conn: &Connection, id: i64, staging: Option<&Path>) -> Vec<(u64, u64)> {
    let limit = staging
        .filter(|path| path.exists())
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    if limit == 0 {
        return Vec::new();
    }
    let Ok(mut stmt) = conn.prepare_cached(
        "SELECT range_start, range_end, bytes_done FROM blocks \
         WHERE download_id = ?1 ORDER BY idx",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    });
    let Ok(rows) = rows else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in rows {
        let Ok((start, end, done)) = row else { return out };
        if done < SAMPLE_WINDOW as i64 || start < 0 || end < start {
            continue;
        }
        let len = (done.min(end - start + 1) as u64).min(limit.saturating_sub(start as u64));
        if len >= SAMPLE_WINDOW {
            out.push((start as u64, len));
        }
    }
    out
}

/// One byte-sample verdict, in the shape the Activity trail prints.
pub struct Sampled {
    pub windows: usize,
    pub covered: u64,
    pub outcome: &'static str,
}

/// The offsets of `count` sample windows spread over the ranges the disk holds.
///
/// Windows are placed by position in the *covered* bytes, so a partial spread
/// over four half-full blocks reads the front of each rather than eight reads of
/// the first one. A window that would straddle two ranges is pulled back inside
/// its own; two placements that land on the same offset are one read, and the
/// caller is told how many windows the answer really rests on.
pub fn spread_windows(ranges: &[(u64, u64)], count: usize, window: u64) -> Vec<u64> {
    let total: u64 = ranges.iter().map(|(_, len)| *len).sum();
    if window == 0 || ranges.is_empty() || total < window || count == 0 {
        return Vec::new();
    }
    let n = count.min((total / window) as usize);
    if n == 1 {
        return vec![ranges[0].0];
    }
    let span = total - window;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut target = span * i as u64 / (n - 1) as u64;
        for (start, len) in ranges {
            if target < *len {
                let off = start + target.min(len - window);
                if out.last() != Some(&off) {
                    out.push(off);
                }
                break;
            }
            target -= *len;
        }
    }
    out
}

/// Compare spread windows of the link about to be trusted against the bytes
/// this job already holds.
///
/// Called when the announced length is unchanged and only a validator moved. No
/// `If-Range` is sent: that header's whole job is to let a host answer a
/// mismatched tag with the entire body, and here the tag is the thing under
/// suspicion. A window that cannot be fetched, or that comes back short, is not
/// evidence that the file is the same, so anything but a full 206 ends the
/// hearing inconclusive and the caller drops the partial as it always did.
fn sample_windows(
    agent: &ureq::Agent,
    url: &str,
    referer: &str,
    cookie: Option<&str>,
    ranges: &[(u64, u64)],
    staging: &Path,
    cancel: &AtomicU8,
) -> Sampled {
    let covered = ranges.iter().map(|(_, len)| *len).sum();
    let offsets = spread_windows(ranges, SAMPLE_COUNT, SAMPLE_WINDOW);
    if offsets.is_empty() {
        return Sampled {
            windows: 0,
            covered,
            outcome: "no-evidence",
        };
    }
    let Ok(file) = File::open(staging) else {
        return Sampled {
            windows: 0,
            covered,
            outcome: "inconclusive",
        };
    };
    let mut done = 0usize;
    for &start in &offsets {
        if cancel.load(Ordering::SeqCst) != RUN {
            return Sampled {
                windows: done,
                covered,
                outcome: "inconclusive",
            };
        }
        let mut req = agent
            .get(url)
            .header("Accept-Encoding", "identity")
            .header("Referer", referer)
            .header(
                "Range",
                format!("bytes={}-{}", start, start + SAMPLE_WINDOW - 1),
            );
        if let Some(cookie) = cookie {
            req = req.header("Cookie", cookie);
        }
        let verdict = |outcome: &'static str| Sampled {
            windows: done,
            covered,
            outcome,
        };
        let Ok(resp) = req.call() else {
            return verdict("inconclusive");
        };
        if resp.status().as_u16() != 206 {
            return verdict("inconclusive");
        }
        let mut remote = vec![0u8; SAMPLE_WINDOW as usize];
        let mut body = resp.into_body();
        if body.as_reader().read_exact(&mut remote).is_err() {
            return verdict("inconclusive");
        }
        let mut local = vec![0u8; SAMPLE_WINDOW as usize];
        if read_exact_at(&file, &mut local, start).is_err() {
            return verdict("inconclusive");
        }
        if remote != local {
            return verdict("differed");
        }
        done += 1;
    }
    Sampled {
        windows: done,
        covered,
        outcome: "matched",
    }
}

fn single_download(
    conn: &Connection,
    ctx: &RunContext,
    routes: &[Route],
    id: i64,
    probe: &Probe,
    agent: &ureq::Agent,
    staging: &Path,
    cancel: &Arc<AtomicU8>,
    base: u64,
) -> Result<u64, JobError> {
    let live = ctx.live.as_ref();
    // One stream cannot bond: it goes out on the route that answered the probe,
    // which the caller put first.
    let route = routes
        .first()
        .cloned()
        .unwrap_or_else(Route::default_route);
    let announced = probe.total_bytes.unwrap_or(0);
    if let Some(live) = live {
        live.set_plan(
            0,
            announced,
            vec![LiveBlock {
                idx: 0,
                offset: 0,
                len: announced,
                bytes_done: 0,
                state: "pending".to_string(),
                network_id: route.color.clone(),
            }],
        );
    }

    let range_end = announced.saturating_sub(1);
    close_dangling_attempts(conn, id);
    let mut attempt = 0u32;
    loop {
        if cancel.load(Ordering::SeqCst) != RUN {
            return Err(JobError::new("cancelled", "cancelled"));
        }
        // A single stream has no block boundary to eject at, because the whole
        // file is one attempt, so this is where the allowance can be honoured:
        // before an attempt starts, and again on every retry. Bytes already on
        // the wire finish, which means an in-flight single-stream download can
        // overshoot the number the user set.
        if route_spent(&ctx.networks, route.network_id) {
            std::thread::sleep(Duration::from_millis(TICK_POLL_MS));
            continue;
        }
        let started_at = now_ms();
        let attempt_id =
            begin_attempt(conn, live, id, 0, 0, range_end, started_at, &route).unwrap_or(-1);
        let net = live_cap(&ctx.networks, route.network_id);
        match single_attempt(
            conn,
            ctx,
            net.as_deref(),
            id,
            probe,
            agent,
            staging,
            cancel,
            attempt_id,
            base,
        ) {
            Ok(total) => {
                if attempt_id >= 0 {
                    finish_attempt(
                        conn,
                        live,
                        attempt_id,
                        0,
                        0,
                        range_end,
                        started_at,
                        total,
                        "done",
                        None,
                        None,
                        &route,
                        &ctx.networks,
                    );
                }
                return Ok(total);
            }
            Err(JobError { code: "cancelled", .. }) => {
                if attempt_id >= 0 {
                    finish_attempt(
                        conn,
                        live,
                        attempt_id,
                        0,
                        0,
                        range_end,
                        started_at,
                        staging_len(staging),
                        "cancelled",
                        None,
                        None,
                        &route,
                        &ctx.networks,
                    );
                }
                return Err(JobError::new("cancelled", "cancelled"));
            }
            Err(e) => {
                if attempt_id >= 0 {
                    finish_attempt(
                        conn,
                        live,
                        attempt_id,
                        0,
                        0,
                        range_end,
                        started_at,
                        staging_len(staging),
                        "retry",
                        Some(match e.status {
                            Some(code) => format!("{code} · retry {}/{}", attempt + 1, MAX_ATTEMPTS),
                            None => format!("retry {}/{}", attempt + 1, MAX_ATTEMPTS),
                        }),
                        Some(e.message.clone()),
                        &route,
                        &ctx.networks,
                    );
                }
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    return Err(JobError::new("stream-retry-exhausted", e.message));
                }
                // The host's wait beats our doubling; one stream has no workers
                // to shed, so waiting is the whole of obeying it.
                let backoff = e
                    .wait_secs
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| Duration::from_millis(250u64 << (attempt - 1).min(4)));
                match e.wait_secs {
                    Some(secs) => history(
                        conn,
                        id,
                        "throttled",
                        json!({ "status": e.status, "streams": 1, "waitSecs": secs, "block": 0 }),
                    ),
                    None => {
                        history(conn, id, "stream-retry", json!({ "attempt": attempt, "why": e.message }));
                    }
                }
                std::thread::sleep(backoff);
            }
        }
    }
}

fn staging_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn single_attempt(
    conn: &Connection,
    ctx: &RunContext,
    net: Option<&Bucket>,
    id: i64,
    probe: &Probe,
    agent: &ureq::Agent,
    staging: &Path,
    cancel: &Arc<AtomicU8>,
    attempt_id: i64,
    base: u64,
) -> Result<u64, JobError> {
    let mut req = agent
        .get(&probe.final_url)
        .header("Accept-Encoding", "identity")
        .header("Referer", &probe.referer);
    if let Some(cookie) = probe.cookie.as_deref() {
        req = req.header("Cookie", cookie);
    }
    let resp = req.call()?;
    let status = resp.status().as_u16();
    if status >= 400 {
        // Only a rate limit carries a waiting instruction worth honouring; any
        // other 4xx/5xx just earns the ordinary backoff.
        let wait = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| retry_after_secs(v, now_ms() / 1000))
            .filter(|_| matches!(status, 429 | 503))
            .map(|secs| secs.max(1) as u64);
        return Err(JobError::new(
            if status == 401 { "needs-auth" } else { "http-status" },
            if status == 401 {
                // The middleware put the stored credential on this very request when the
                // host has one, so which of the two cases this is — none, or a refused one —
                // is knowable here from the store, rather than guessed at by the page
                // from a bare 401. `credential` because the host may be answered by a
                // pair or by a token and only the store knows which.
                if super::auth::has(&super::host_key(&probe.final_url)) {
                    "the stored credential was refused".to_string()
                } else {
                    "server needs a credential".to_string()
                }
            } else {
                format!("server answered {status}")
            },
        )
        .with_status(status)
        .with_wait(wait));
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .or(probe.total_bytes);

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(staging)?;
    if reserves_length(conn) {
        if let Some(t) = total {
            file.set_len(t)?;
        }
    }

    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut buf = vec![0u8; CHUNK];
    let mut caps: [&Bucket; 3] = [&ctx.limit, &ctx.global, &ctx.limit];
    // The same shape as in `fetch_block`: the third slot is only in range while
    // `net` is Some, so the repeated job bucket there is never charged.
    let caps: &[&Bucket] = match net {
        Some(net) => {
            caps[2] = net;
            &caps
        }
        None => &caps[..2],
    };
    let mut offset = 0u64;
    let mut last_flush = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) != RUN {
            return Err(JobError::new("cancelled", "cancelled"));
        }
        let slice = limit::read_slice(caps);
        let n = match reader.read(&mut buf[..slice]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(JobError::new("stream-read", e.to_string())),
        };
        if limit::charge(caps, n, cancel).is_err() {
            return Err(JobError::new("cancelled", "cancelled"));
        }
        file.write_all(&buf[..n])?;
        offset += n as u64;
        if last_flush.elapsed().as_millis() >= PROGRESS_MS {
            set_progress(conn, id, base + offset, total.map(|t| base + t))?;
            if let Some(live) = ctx.live.as_ref() {
                live.update_blocks(|b| {
                    b.bytes_done = offset;
                    if b.state == "pending" {
                        b.state = "active".to_string();
                    }
                });
                if attempt_id >= 0 {
                    live.update_active_attempt(attempt_id, offset);
                }
            }
            ctx.emit_blocks(id, ctx.live.as_ref(), false);
            ctx.emit(id);
            last_flush = Instant::now();
        }
    }

    if let Some(t) = total {
        if offset != t {
            return Err(JobError::new(
                "stream-short",
                format!("stream ended at {offset} of {t} bytes"),
            ));
        }
    }
    file.sync_all()?;
    set_progress(conn, id, base + offset, total.map(|t| base + t).or(Some(base + offset)))?;
    if let Some(live) = ctx.live.as_ref() {
        live.update_blocks(|b| {
            b.bytes_done = offset;
            b.state = "done".to_string();
        });
    }
    ctx.emit_blocks(id, ctx.live.as_ref(), true);
    ctx.emit(id);
    Ok(total.unwrap_or(offset))
}

fn publish(
    conn: &Connection,
    ctx: &RunContext,
    id: i64,
    row: &Row,
    final_url: &str,
    staging: &Path,
    total: u64,
    cancel: &Arc<AtomicU8>,
) -> Result<(), JobError> {
    if cancel.load(Ordering::SeqCst) != RUN {
        return Err(JobError::new("cancelled", "cancelled before publish"));
    }
    history(conn, id, "publish-preparing", json!({ "total": total }));

    // A4: the bytes on disk are the length the server announced.
    let opened = File::open(staging)?;
    let on_disk = opened.metadata()?.len();
    drop(opened);
    if on_disk != total {
        return Err(JobError::new(
            "length-mismatch",
            format!("staging file is {on_disk} bytes, expected {total}"),
        ));
    }

    let bool_setting = |key: &str, fallback: bool| {
        crate::db::setting(conn, key)
            .ok()
            .flatten()
            .map(|v| !v.eq_ignore_ascii_case("false") && v != "0")
            .unwrap_or(fallback)
    };
    let auto_verify = bool_setting("storage.auto_verify", true);

    let checksum = match (&row.checksum_algo, &row.checksum_hex) {
        (Some(algo), Some(expected)) if super::is_algo(algo) => {
            history(conn, id, "publish-verifying", json!({ "algo": algo }));
            let actual = super::file_digest(staging, algo)?;
            if !actual.eq_ignore_ascii_case(expected.trim()) {
                let _ = conn.execute(
                    "UPDATE downloads SET status = 'needs-attention', error_code = 'checksum-mismatch', \
                     error_hint = ?1 WHERE id = ?2",
                    params![format!("expected {expected}, got {actual}"), id],
                );
                history(
                    conn,
                    id,
                    "checksum-mismatch",
                    json!({ "algo": algo, "expected": expected, "actual": actual }),
                );
                ctx.emit(id);
                return Err(JobError::new("checksum-mismatch", "checksum did not match"));
            }
            history(conn, id, "checksum-ok", json!({ "algo": algo, "digest": actual }));
            true
        }
        _ => {
            // No expected digest supplied: hash the staged bytes anyway so the
            // Activity trail records the identity of what was published.
            if auto_verify {
                history(conn, id, "publish-verifying", json!({ "algo": "sha256" }));
                match super::file_digest(staging, "sha256") {
                    Ok(digest) => history(
                        conn,
                        id,
                        "auto-verify",
                        json!({ "algo": "sha256", "digest": digest }),
                    ),
                    Err(e) => history(conn, id, "auto-verify-skipped", json!({ "why": e.to_string() })),
                }
            }
            false
        }
    };

    let folder = staging.parent().unwrap_or(Path::new("."));
    let name: String = conn
        .query_row("SELECT filename FROM downloads WHERE id = ?1", [id], |r| r.get(0))
        .unwrap_or_else(|_| "download".to_string());
    let dest = unique_path(folder, &name);
    history(conn, id, "publish-finalizing", json!({ "path": dest.to_string_lossy() }));
    rename_with_retry(staging, &dest).map_err(|e| {
        JobError::new("publish-failed", format!("rename to {}: {e}", dest.display()))
    })?;

    let referrer = row.referer.clone().unwrap_or_else(|| final_url.to_string());
    if cfg!(windows) && bool_setting("storage.motw", true) {
        if let Err(e) = write_zone_identifier(&dest, &referrer, &host_of(final_url)) {
            history(conn, id, "zone-skipped", json!({ "why": e.to_string() }));
        }
    }

    let final_name = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(name);
    let status = if checksum { "verified" } else { "finished" };
    conn.execute(
        "UPDATE downloads SET status = ?1, folder = ?2, filename = ?3, staging_path = NULL, \
         verified_bytes = ?4, total_bytes = ?4, finished_at = ?5, error_code = NULL, error_hint = NULL \
         WHERE id = ?6",
        params![
            status,
            folder.to_string_lossy(),
            final_name,
            total as i64,
            now_ms(),
            id
        ],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    history(
        conn,
        id,
        "published",
        json!({ "path": dest.to_string_lossy(), "verified": checksum }),
    );
    ctx.emit(id);
    Ok(())
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_string()
}

/// Bytes in a shape a refusal can be read aloud. Whole units only: one decimal
/// place would pull std's float formatter into a binary that is a few kilobytes
/// from its size ceiling.
fn human_bytes(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = KB * 1024;
    const GB: i64 = MB * 1024;
    let b = bytes.abs();
    if b < KB {
        format!("{b} B")
    } else if b < MB {
        format!("{} KB", b / KB)
    } else if b < GB {
        format!("{} MB", b / MB)
    } else {
        format!("{} GB", b / GB)
    }
}

/// Whether a new link can carry the bytes this job already holds.
///
/// Pure, so every refusal is reachable from a unit test without a server: the
/// interesting part of a refresh is the comparison, not the probe. On success it
/// returns which validator agreed, for the history row.
pub fn check_refresh(
    job_total: Option<i64>,
    job_etag: Option<&str>,
    job_modified: Option<&str>,
    fresh: &Probe,
) -> Result<&'static str, JobError> {
    if !fresh.accept_ranges {
        return Err(JobError::new(
            "refresh-no-range",
            "the new link will not serve byte ranges, so the part already downloaded cannot be resumed",
        ));
    }
    let fresh_total = fresh.total_bytes.map(|t| t as i64);
    match (job_total, fresh_total) {
        (Some(have), Some(now)) if have != now => {
            return Err(JobError::new(
                "refresh-size",
                format!(
                    "Size mismatch: this download is {}, the new link is {}",
                    human_bytes(have),
                    human_bytes(now)
                ),
            ));
        }
        // Ranges were honoured, yet no length came back: nothing to compare the
        // finished slices against, so the plan cannot be trusted.
        (Some(_), None) => {
            return Err(JobError::new(
                "refresh-unknown-size",
                "the new link gives no length, so it cannot be checked against this download",
            ));
        }
        _ => {}
    }
    // ETag first: it is the stronger of the two. Last-Modified only when no
    // ETag exists on either side, and no claim at all when the host offers
    // neither — a matching length is then all the promise there is.
    if let (Some(have), Some(now)) = (job_etag, fresh.etag.as_deref()) {
        if have != now {
            return Err(JobError::new(
                "refresh-changed",
                format!("the new link serves a different file ({have} vs {now})"),
            ));
        }
        return Ok("etag");
    }
    if let (Some(have), Some(now)) = (job_modified, fresh.last_modified.as_deref()) {
        if have != now {
            return Err(JobError::new(
                "refresh-changed",
                format!("the new link was last modified at another time ({have} vs {now})"),
            ));
        }
        return Ok("last-modified");
    }
    Ok("none")
}

/// Point a paused or broken download at a fresh link for the same file.
///
/// The probe runs with no connection held — a signed link can take the whole
/// connect timeout to answer, and the UI thread owns the shared one — and
/// nothing is written until the new link passes every assert, so a refused
/// refresh leaves the job exactly as it was. The finished slices stay: the row
/// keeps its length, block plan and staging file, and the next Start resumes
/// from where the old link stopped working.
pub fn refresh_url(db_path: &Path, id: i64, new_url: &str) -> Result<(), JobError> {
    let new_url = new_url.trim();
    if !(new_url.starts_with("http://") || new_url.starts_with("https://")) {
        return Err(JobError::new("refresh-scheme", "the new link is not an http(s) URL"));
    }
    let conn = crate::db::open(db_path).map_err(|e| JobError::new("db", e))?;
    let (url, status, total, etag, modified, referer, staging, kept, row_ua, row_cookie) = conn
        .query_row(
            "SELECT url, status, total_bytes, etag, last_modified, referer, staging_path, \
             verified_bytes, ua, cookie FROM downloads WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                JobError::new("missing", format!("download {id} not found"))
            }
            other => JobError::new("db", other.to_string()),
        })?;
    if status == "downloading" {
        return Err(JobError::new(
            "refresh-running",
            "pause this download before replacing its link",
        ));
    }
    if status == "finished" || status == "verified" {
        return Err(JobError::new(
            "refresh-complete",
            "this download is already finished; its link does not need refreshing",
        ));
    }
    let referer = referer.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
    // Bytes counted in the database that no longer have a file behind them are
    // not kept bytes, whatever the trail says.
    let kept = match staging.as_ref() {
        Some(p) if Path::new(p).exists() => kept,
        _ => 0,
    };
    // The new link is the one being asked for permission, so its host's stored
    // session decides this probe's identity too. Read while the connection is
    // still open: the probe itself must not hold it.
    let new_host = super::host_key(new_url);
    let session = if new_host.is_empty() {
        None
    } else {
        crate::db::session_for(&conn, &new_host).map_err(|e| JobError::new("db", e))?
    };
    // Read before the connection goes: a sample may need to know which byte ranges
    // this row's ledger says it holds.
    let ranges = covered_ranges(&conn, id, staging.as_ref().map(Path::new));
    drop(conn);

    let ua = ride(row_ua.as_deref(), session.as_ref().and_then(|s| s.user_agent.as_deref()));
    let cookie = ride(row_cookie.as_deref(), session.as_ref().map(|s| s.cookie.as_str()));
    let agent = super::agent_session(ua);
    let fresh = probe(&agent, new_url, referer.as_deref(), cookie)?;
    let mut sampled: Option<Sampled> = None;
    let validator = match check_refresh(total, etag.as_deref(), modified.as_deref(), &fresh) {
        Ok(validator) => validator,
        // The new link swears it is a different file, and the length already
        // agreed, and ranges are honoured — which is the exact shape a rebuild
        // of the same file leaves behind. The bytes are the only witness left,
        // so ask them before turning the link down.
        Err(e) if e.code == "refresh-changed" => {
            let verdict = match &staging {
                Some(path) => sample_windows(
                    &agent,
                    new_url,
                    referer.as_deref().unwrap_or(new_url),
                    cookie,
                    &ranges,
                    Path::new(path),
                    &AtomicU8::new(RUN),
                ),
                // Nothing on disk to compare against, so nothing to argue with.
                None => Sampled {
                    windows: 0,
                    covered: 0,
                    outcome: "no-evidence",
                },
            };
            let outcome = verdict.outcome;
            sampled = Some(verdict);
            if outcome != "matched" {
                return Err(e);
            }
            "sample"
        }
        Err(e) => return Err(e),
    };

    let conn = crate::db::open(db_path).map_err(|e| JobError::new("db", e))?;
    conn.execute(
        "UPDATE downloads SET url = ?1, final_url = ?2, etag = ?3, last_modified = ?4, \
         total_bytes = COALESCE(?5, total_bytes), error_code = NULL, error_hint = NULL \
         WHERE id = ?6",
        params![
            new_url,
            fresh.final_url,
            fresh.etag,
            fresh.last_modified,
            fresh.total_bytes.map(|t| t as i64),
            id
        ],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    history(
        &conn,
        id,
        "url-refreshed",
        json!({
            "oldHost": host_of(&url),
            "newHost": host_of(new_url),
            "keptBytes": kept,
            "total": total,
            "validator": validator,
            "sampleOutcome": sampled.as_ref().map(|s| s.outcome),
            "sampleWindows": sampled.as_ref().map(|s| s.windows),
            "sampleCovered": sampled.as_ref().map(|s| s.covered),
        }),
    );
    Ok(())
}

/// Queue of ids handed to the thread pool by the IPC layer. Kept here so the
/// engine owns its own scheduling state rather than the UI.
#[derive(Default)]
pub struct Registry {
    inner: std::sync::Mutex<VecDeque<(i64, Arc<AtomicU8>)>>,
    live: std::sync::Mutex<std::collections::HashMap<i64, Arc<Live>>>,
    limits: std::sync::Mutex<std::collections::HashMap<i64, Arc<Bucket>>>,
    /// Ids the user pressed Start on while every slot was busy. In memory on
    /// purpose: only a deliberate Start may launch a queued row, never a scan
    /// of rows that were added with auto-start off.
    pending: std::sync::Mutex<VecDeque<i64>>,
}

impl Registry {
    pub fn register(&self, id: i64) -> (Arc<AtomicU8>, Arc<Live>, Arc<Bucket>) {
        let cancel = Arc::new(AtomicU8::new(RUN));
        let live = Arc::new(Live::new());
        let limit = Arc::new(Bucket::new(None));
        if let Ok(mut q) = self.inner.lock() {
            q.retain(|(other, _)| *other != id);
            q.push_back((id, Arc::clone(&cancel)));
        }
        if let Ok(mut map) = self.live.lock() {
            map.insert(id, Arc::clone(&live));
        }
        if let Ok(mut map) = self.limits.lock() {
            map.insert(id, Arc::clone(&limit));
        }
        (cancel, live, limit)
    }

    /// Applies a new cap to a running job's bucket. Returns false when the job
    /// is not running, in which case the row alone carries the setting.
    pub fn set_limit(&self, id: i64, cap: Option<u64>) -> bool {
        let bucket = self
            .limits
            .lock()
            .ok()
            .and_then(|map| map.get(&id).cloned());
        match bucket {
            Some(bucket) => {
                bucket.set_cap(cap);
                true
            }
            None => false,
        }
    }

    /// The live map for a running job. Gone once the job unregisters; callers
    /// fall back to the `blocks` and `block_attempts` rows.
    pub fn live(&self, id: i64) -> Option<Arc<Live>> {
        self.live.lock().ok().and_then(|map| map.get(&id).cloned())
    }

    pub fn pause_all(&self) {
        if let Ok(q) = self.inner.lock() {
            for (_, flag) in q.iter() {
                flag.store(PAUSE, Ordering::SeqCst);
            }
        }
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }

    pub fn signal(&self, id: i64, code: u8) -> bool {
        if let Ok(q) = self.inner.lock() {
            if let Some((_, flag)) = q.iter().find(|(other, _)| *other == id) {
                flag.store(code, Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    pub fn unregister(&self, id: i64) {
        if let Ok(mut q) = self.inner.lock() {
            q.retain(|(other, _)| *other != id);
        }
        if let Ok(mut map) = self.live.lock() {
            map.remove(&id);
        }
        if let Ok(mut map) = self.limits.lock() {
            map.remove(&id);
        }
    }

    pub fn is_running(&self, id: i64) -> bool {
        self.inner
            .lock()
            .map(|q| q.iter().any(|(other, _)| *other == id))
            .unwrap_or(false)
    }

    /// Block until the job thread has unregistered, so a removed id cannot be
    /// reused by a fresh insert while the old job is still writing to its row.
    pub fn wait_idle(&self, id: i64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.is_running(id) {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        true
    }

    pub fn running_count(&self) -> usize {
        self.inner.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// Re-asserts the system-awake request for exactly as long as this registry
    /// carries a job. Called from the thread that starts a job, the thread that
    /// finishes one, and the adapter watcher — never from a job's own worker,
    /// because `SetThreadExecutionState` belongs to the thread that made it and
    /// a request from a short-lived thread would end with that thread.
    /// Idempotent, so the watcher's five-second pass is a guard rather than a
    /// state machine: nothing can leave the request held by a machine with no
    /// transfer on it.
    pub fn refresh_keep_awake(&self) {
        crate::keep_awake::set(self.running_count() > 0);
    }

    pub fn push_pending(&self, id: i64) {
        if let Ok(mut q) = self.pending.lock() {
            q.retain(|other| *other != id);
            q.push_back(id);
        }
    }

    pub fn remove_pending(&self, id: i64) {
        if let Ok(mut q) = self.pending.lock() {
            q.retain(|other| *other != id);
        }
    }

    pub fn pop_pending(&self) -> Option<i64> {
        self.pending.lock().ok().and_then(|mut q| q.pop_front())
    }
}

// ---------------------------------------------------------------------------
// Browser capture: rows that start from bytes the browser already carried.
// Both paths write the same tables the engine writes, so the library, the
// category rules and the digest check treat a captured file like any other.
// ---------------------------------------------------------------------------

/// A file the browser finished by itself. Zero bytes are moved over the wire;
/// the only work is placing it and writing the row that makes it visible.
pub struct AdoptSpec {
    /// The link the file came from, when the add-on knew it. A `blob:` capture
    /// has none, and the row is still worth having.
    pub url: Option<String>,
    pub path: std::path::PathBuf,
    pub filename: String,
    pub referer: Option<String>,
    /// `chrome` or `firefox`, from the origin the hand-off carried.
    pub source: String,
}

pub fn adopt(
    db_path: &Path,
    downloads_dir: &str,
    spec: AdoptSpec,
) -> Result<i64, JobError> {
    let size = std::fs::metadata(&spec.path)
        .map_err(|e| JobError::new("io", format!("{}: {e}", spec.path.display())))?
        .len();
    let conn = crate::db::open(db_path).map_err(|e| JobError::new("db", e))?;
    let folder = crate::db::setting(&conn, "storage.default_folder")
        .ok()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(downloads_dir));
    std::fs::create_dir_all(&folder)?;
    let dest = super::unique_path(&folder, &super::sanitize_filename(&spec.filename));
    if dest != spec.path {
        if super::rename_with_retry(&spec.path, &dest).is_err() {
            // Another volume: a rename is not available, so copy and then let the
            // browser's own copy go. A failed copy removes the half-written dest.
            std::fs::copy(&spec.path, &dest).map_err(|e| {
                let _ = std::fs::remove_file(&dest);
                JobError::new("io", format!("copy to {}: {e}", dest.display()))
            })?;
            let _ = std::fs::remove_file(&spec.path);
        }
    }
    let filename = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(spec.filename);
    let category = crate::db::category_for_file(&conn, &filename).unwrap_or(None);
    let url = spec.url.unwrap_or_default();
    conn.execute(
        "INSERT INTO downloads (url, final_url, filename, folder, status, category_id, \
         referer, source_browser, total_bytes, verified_bytes, created_at, finished_at) \
         VALUES (?1, ?1, ?2, ?3, 'finished', ?4, ?5, ?6, ?7, ?7, ?8, ?8)",
        params![
            url,
            filename,
            folder.to_string_lossy(),
            category,
            spec.referer,
            spec.source,
            size as i64,
            now_ms()
        ],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    let id = conn.last_insert_rowid();
    history(
        &conn,
        id,
        "capture-adopted",
        json!({ "bytes": size, "source": spec.source, "path": dest.to_string_lossy() }),
    );
    Ok(id)
}

/// An in-flight browser download: the contiguous prefix on disk becomes the
/// staging file, and the row is planned exactly as the engine would plan it so
/// the first Start resumes at the last byte the browser wrote.
pub struct PartialSpec {
    pub url: String,
    pub path: std::path::PathBuf,
    pub filename: Option<String>,
    pub referer: Option<String>,
    pub source: String,
}

pub fn seed_partial(
    db_path: &Path,
    downloads_dir: &str,
    spec: PartialSpec,
) -> Result<i64, JobError> {
    let prefix = std::fs::metadata(&spec.path)
        .map_err(|e| JobError::new("io", format!("{}: {e}", spec.path.display())))?
        .len();
    if prefix == 0 {
        return Err(JobError::new("empty-partial", "the browser has written no bytes yet"));
    }
    let conn = crate::db::open(db_path).map_err(|e| JobError::new("db", e))?;
    let host = super::host_key(&spec.url);
    let session = crate::db::session_for(&conn, &host).ok().flatten();
    let ua = session.as_ref().and_then(|s| s.user_agent.as_deref());
    let cookie = session.as_ref().map(|s| s.cookie.as_str());
    let agent = super::config_ua(std::time::Duration::from_secs(10), ua).new_agent();
    let probe = super::probe(&agent, &spec.url, spec.referer.as_deref(), cookie)?;
    let Some(total) = probe.total_bytes else {
        return Err(JobError::new(
            "unknown-length",
            "the host did not say how long the file is, so a prefix cannot be placed",
        ));
    };
    if !probe.accept_ranges {
        return Err(JobError::new(
            "no-ranges",
            "the host will not serve the rest of the file from an offset",
        ));
    }
    let prefix = prefix.min(total);
    if prefix == 0 || prefix >= total {
        return Err(JobError::new(
            "not-partial",
            "the browser's file is empty or already whole",
        ));
    }
    let filename = spec
        .filename
        .as_deref()
        .or(probe.filename.as_deref())
        .map(super::sanitize_filename)
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| super::filename_from_url(&spec.url));
    let folder = crate::db::setting(&conn, "storage.default_folder")
        .ok()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(downloads_dir));
    std::fs::create_dir_all(&folder)?;
    let band = row_band(&conn, None);
    let block_size = super::adaptive_block(total, band.start, DEFAULT_BLOCK).min(total.max(1));
    let category = crate::db::category_for_file(&conn, &filename).unwrap_or(None);
    let referer = spec
        .referer
        .clone()
        .or_else(|| Some(probe.final_url.clone()));
    conn.execute(
        "INSERT INTO downloads (url, final_url, filename, folder, status, category_id, referer, \
         source_browser, etag, last_modified, content_type, total_bytes, block_size, \
         streams_mode, resumable, created_at) \
         VALUES (?1, ?2, ?3, ?4, 'paused', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'adaptive', 1, ?13)",
        params![
            spec.url,
            probe.final_url,
            filename,
            folder.to_string_lossy(),
            category,
            referer,
            spec.source,
            probe.etag,
            probe.last_modified,
            probe.content_type,
            total as i64,
            block_size as i64,
            crate::now_ms(),
        ],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    // block_count is filled after the plan exists, in one statement with the
    // staging path, so a row never claims a plan it does not have.
    let id = conn.last_insert_rowid();
    let plan = super::plan_blocks(total, block_size);
    let staging = folder.join(format!("{filename}.{id}.sucrpart"));
    {
        let mut source = std::fs::File::open(&spec.path)?;
        let mut out = std::fs::File::create(&staging)?;
        let copied = std::io::copy(&mut (&mut source).take(prefix), &mut out)
            .map_err(|e| {
                let _ = std::fs::remove_file(&staging);
                JobError::new("io", format!("copy browser partial: {e}"))
            })?;
        if copied != prefix {
            let _ = std::fs::remove_file(&staging);
            return Err(JobError::new(
                "short-partial",
                format!("the browser's file shrank while it was read: {copied} of {prefix}"),
            ));
        }
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| JobError::new("db", e.to_string()))?;
    for (idx, (start, end)) in plan.iter().enumerate() {
        let len = end - start + 1;
        let done = prefix.saturating_sub(*start).min(len);
        let state = if done == len { "done" } else { "pending" };
        tx.execute(
            "INSERT INTO blocks (download_id, idx, range_start, range_end, state, bytes_done) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, idx as i64, *start as i64, *end as i64, state, done as i64],
        )
        .map_err(|e| JobError::new("db", e.to_string()))?;
    }
    tx.execute(
        "UPDATE downloads SET staging_path = ?1, block_count = ?2, verified_bytes = ?3 WHERE id = ?4",
        params![staging.to_string_lossy(), plan.len() as i64, prefix as i64, id],
    )
    .map_err(|e| JobError::new("db", e.to_string()))?;
    tx.commit().map_err(|e| JobError::new("db", e.to_string()))?;
    history(
        &conn,
        id,
        "capture-imported",
        json!({
            "bytes": prefix,
            "total": total,
            "blocks": plan.len(),
            "source": spec.source,
            "etag": probe.etag,
        }),
    );
    Ok(id)
}

/// The starting width a job would choose for a row with this stored dial.
fn row_band(conn: &Connection, fixed: Option<i64>) -> StreamBand {
    fixed
        .filter(|s| *s > 0)
        .map(|s| StreamBand { start: s as usize, ceiling: s as usize })
        .unwrap_or_else(|| stream_band(conn))
}

#[cfg(test)]
mod tests;
