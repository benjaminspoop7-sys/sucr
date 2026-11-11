use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 4;
const SCHEMA: &str = include_str!("../schema.sql");

/// Opens <dataDir>/sucr.db in WAL mode and brings it up to the current schema.
pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create data dir: {e}"))?;
    }
    let conn =
        Connection::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .map_err(|e| format!("pragma: {e}"))?;

    let mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .map_err(|e| format!("journal_mode: {e}"))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(format!("sucr.db: expected WAL journal mode, got {mode}"));
    }
    // WAL's standard pairing: commits stop fsyncing the WAL on every write.
    // With FULL, each of a job's ~150 ledger autocommits paid a disk flush,
    // which on a spinning data drive serialized the supervisor behind the
    // platter (5.3 s of a 6.6 s transfer). NORMAL keeps process-crash
    // durability; a power loss can drop the newest commits but never corrupts
    // the database, and a resumed job re-verifies its bytes anyway.
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("synchronous: {e}"))?;

    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| format!("user_version: {e}"))?;

    if current > SCHEMA_VERSION {
        return Err(format!(
            "sucr.db is schema v{current}; this build knows v{SCHEMA_VERSION}. Refusing to open a newer file."
        ));
    }
    if current == SCHEMA_VERSION {
        repair_columns(conn)?;
        restripe(conn)?;
        return ensure_categories(conn);
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin: {e}"))?;
    if current == 0 {
        tx.execute_batch(SCHEMA).map_err(|e| format!("apply schema: {e}"))?;
    } else {
        if current <= 1 {
            tx.execute_batch("ALTER TABLE block_attempts ADD COLUMN ended_at INTEGER")
                .map_err(|e| format!("v1 -> v2: {e}"))?;
        }
        if current <= 2 {
            tx.execute_batch(
                "ALTER TABLE downloads ADD COLUMN speed_limit INTEGER;\
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .map_err(|e| format!("v2 -> v3: {e}"))?;
        }
        if current <= 3 {
            tx.execute_batch(
                "CREATE TABLE unattributed_usage (\
                   period_key TEXT NOT NULL PRIMARY KEY, bytes INTEGER NOT NULL DEFAULT 0);",
            )
            .map_err(|e| format!("v3 -> v4: {e}"))?;
        }
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| format!("set user_version: {e}"))?;
    tx.commit().map_err(|e| format!("commit: {e}"))?;
    repair_columns(conn)?;
    restripe(conn)?;
    ensure_categories(conn)
}

/// The five categories the rail lists, named for the subfolders the add bar
/// files a download into. `patterns_json` is the rule itself: a row's extension
/// is matched against these arrays, so the table — not a copy in Rust or in the
/// renderer — decides what `Archives` holds.
const DEFAULT_CATEGORIES: [(&str, &str, &[&str]); 5] = [
    (
        "Archives",
        "Archives",
        &[
            "iso", "img", "zip", "rar", "7z", "tar", "gz", "xz", "zst", "bz2", "bz", "tgz",
            "tbz2", "txz", "cab", "pup",
        ],
    ),
    ("Videos", "Videos", &["mp4", "mkv", "webm", "avi", "mov", "m4v"]),
    ("Audio", "Audio", &["mp3", "m4a", "flac", "wav", "ogg", "opus"]),
    ("Documents", "Documents", &["pdf", "epub", "doc", "docx", "txt"]),
    ("Programs", "Programs", &["msi", "exe", "appx", "dmg", "deb", "rpm", "pkg"]),
];

/// Seeds the category table and files the rows that predate it.
///
/// An empty table is the idempotency gate, so this also catches a database
/// created before the writer existed. Skipped when the table itself is missing:
/// a partial file still has to open.
fn ensure_categories(conn: &Connection) -> Result<(), String> {
    if !table_exists(conn, "categories")? {
        return Ok(());
    }
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
        .map_err(|e| format!("count categories: {e}"))?;
    if rows == 0 {
        for (index, (name, folder, patterns)) in DEFAULT_CATEGORIES.iter().enumerate() {
            conn.execute(
                "INSERT INTO categories (name, folder, rule_tier, patterns_json, sort_order) \
                 VALUES (?1, ?2, 'EXTENSION', ?3, ?4)",
                rusqlite::params![
                    name,
                    folder,
                    serde_json::to_string(patterns).map_err(|e| e.to_string())?,
                    index as i64
                ],
            )
            .map_err(|e| format!("seed category {name}: {e}"))?;
        }
    }
    backfill_categories(conn)
}

/// Which category a filename's extension falls under, by the rules in the table.
///
/// The last dot wins, so `movie.tar.gz` is read as `gz` — which the archive
/// rules already list, so no compound-extension table is needed. A name with no
/// dot is not forced into a category.
pub fn category_for_file(conn: &Connection, filename: &str) -> Result<Option<i64>, String> {
    let ext = match filename.rsplit_once('.') {
        Some((_, "")) | None => return Ok(None),
        Some((_, ext)) => ext.to_ascii_lowercase(),
    };
    let mut stmt = conn
        .prepare(
            "SELECT id, patterns_json FROM categories \
             WHERE rule_tier = 'EXTENSION' ORDER BY sort_order, id",
        )
        .map_err(|e| format!("read category rules: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| format!("read category rules: {e}"))?;
    for row in rows {
        let (id, patterns) = row.map_err(|e| e.to_string())?;
        let list: Vec<String> = serde_json::from_str(&patterns).unwrap_or_default();
        if list.iter().any(|p| p == &ext) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Points one download at one category; `None` clears it.
///
/// Both rows have to exist. A silent no-op on a stale id would let the card keep
/// showing the old category while the picker said the file had moved.
pub fn assign_category(
    conn: &Connection,
    download_id: i64,
    category_id: Option<i64>,
) -> Result<(), String> {
    let known: Option<i64> = conn
        .query_row("SELECT 1 FROM downloads WHERE id = ?1", [download_id], |r| r.get(0))
        .optional()
        .map_err(|e| format!("read download: {e}"))?;
    if known.is_none() {
        return Err(format!("download {download_id} not found"));
    }
    if let Some(category_id) = category_id {
        let name: Option<String> = conn
            .query_row(
                "SELECT name FROM categories WHERE id = ?1",
                [category_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("read category: {e}"))?;
        if name.is_none() {
            return Err(format!("no such category: {category_id}"));
        }
    }
    conn.execute(
        "UPDATE downloads SET category_id = ?2 WHERE id = ?1",
        rusqlite::params![download_id, category_id],
    )
    .map_err(|e| format!("set category: {e}"))?;
    Ok(())
}

/// Files every row still uncategorised, once, on open. A library that predates
/// the writer would otherwise show five empty categories next to a full list.
fn backfill_categories(conn: &Connection) -> Result<(), String> {
    let rows: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, filename FROM downloads WHERE category_id IS NULL")
            .map_err(|e| format!("read uncategorised: {e}"))?;
        let found = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("read uncategorised: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        found
    };
    for (id, filename) in rows {
        if let Some(category) = category_for_file(conn, &filename)? {
            assign_category(conn, id, Some(category))?;
        }
    }
    Ok(())
}

/// Whether the file holds this table at all — a v1 stub database has only the
/// tables an upgrade touched.
fn table_exists(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| format!("sqlite_master: {e}"))
}

/// A build can stamp `user_version` while an older table shape is still on
/// disk (an early v3 did exactly that with the probe columns), so a version
/// check alone cannot be trusted to make a file current. Add whatever a
/// current build reads but the file lacks; the check makes this a no-op on
/// databases that already match.
fn repair_columns(conn: &Connection) -> Result<(), String> {
    for (table, name, decl) in [
        ("downloads", "content_type", "TEXT"),
        ("downloads", "staging_path", "TEXT"),
        ("downloads", "etag", "TEXT"),
        ("downloads", "last_modified", "TEXT"),
        ("downloads", "streams_final", "INTEGER"),
        // The browser session a captured link rode: the User-Agent and the Cookie
        // are both nullable columns `job::run`/`refresh_url` prefer over the
        // host-wide session when present — the per-download ride an attested
        // stream needs. `ua` already ships in schema.sql; it is ensured here so a
        // file older than that column still opens and reads.
        ("downloads", "ua", "TEXT"),
        ("downloads", "cookie", "TEXT"),
        // The combined A/V job: the sound's link on the same row, and the
        // picture kept on disk while the sound is still arriving.
        ("downloads", "audio_url", "TEXT"),
        ("downloads", "part_path", "TEXT"),
        ("block_attempts", "detail", "TEXT"),
    ] {
        if !table_exists(conn, table)? {
            continue;
        }
        let exists: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                [name],
                |r| r.get(0),
            )
            .map_err(|e| format!("table_info: {e}"))?;
        if exists == 0 {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {name} {decl}"))
                .map_err(|e| format!("repair {table}.{name}: {e}"))?;
        }
    }
    Ok(())
}

/// One value from the settings key/value table; a missing key is not an error.
pub fn setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })
}

/// The three keys that make one proxy decision, an unset key read as empty.
///
/// Together because they are one decision: the mirror must never apply a mode
/// against a URL or bypass list from before a write, and the two places that
/// read them — the boot pass and `settings_set` — would otherwise have to agree
/// on that by hand.
pub fn proxy_settings(conn: &Connection) -> Result<(String, String, String), String> {
    let one = |key: &str| setting(conn, key).map(|value| value.unwrap_or_default());
    Ok((
        one("network.proxy_mode")?,
        one("network.proxy_url")?,
        one("network.proxy_bypass")?,
    ))
}

/// When Sucr is allowed to *start* a queued link, chosen once for the whole app.
///
/// Four `settings` keys rather than a `schedules` row: this is one global
/// decision the Settings page edits in place, and every other engine-wide switch
/// already lives where `settings:list` can show it. The table keeps its place for
/// per-download starts, which nothing here implements.
pub struct DownloadWindow {
    pub enabled: bool,
    pub start_minute: u32,
    pub stop_minute: u32,
    /// Monday-first bit mask, bit 0 = Monday. `0` means every day, not no days:
    /// an unset mask is a user who has not filtered any day, and the day chips
    /// all light up for it.
    pub days: u32,
}

impl DownloadWindow {
    /// Whether a transfer may start at `minute` of a `weekday` counted Monday
    /// first — the same convention the quota week boundary uses in
    /// [`network_usage`], so the two never disagree about when a day ends.
    ///
    /// A stop at or before the start is an overnight window, which is the whole
    /// reason to have one: 22:00-06:00 is one window, and the small hours belong
    /// to the evening that opened it. A start equal to the stop is every minute,
    /// because that is what an enabled switch and two empty time boxes mean.
    pub fn open_at(&self, weekday: u32, minute: u32) -> bool {
        if !self.enabled {
            return true;
        }
        let (start, stop) = (self.start_minute, self.stop_minute);
        let wraps = stop < start;
        let inside = if start == stop {
            true
        } else if wraps {
            minute >= start || minute < stop
        } else {
            minute >= start && minute < stop
        };
        if !inside {
            return false;
        }
        let start_day = if wraps && minute < stop {
            (weekday + 6) % 7
        } else {
            weekday
        };
        self.days == 0 || self.days & (1 << start_day) != 0
    }

    /// The window a user asked for, an unset key read as "no opinion".
    pub fn read(conn: &Connection) -> Result<Self, String> {
        let minute_of = |key: &str| -> Result<u32, String> {
            Ok(setting(conn, key)?
                .as_deref()
                .and_then(parse_minute)
                .unwrap_or(0))
        };
        Ok(Self {
            enabled: setting(conn, "engine.window_enabled")?.as_deref() == Some("true"),
            start_minute: minute_of("engine.window_start")?,
            stop_minute: minute_of("engine.window_stop")?,
            days: setting(conn, "engine.window_days")?
                .as_deref()
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(0)
                & 0x7f,
        })
    }
}

/// `HH:MM` in 24-hour form — the shape an `<input type="time">` gives back, and
/// the one worth storing because a user can read it out of `settings:list`.
pub fn parse_minute(value: &str) -> Option<u32> {
    let (hours, minutes) = value.trim().split_once(':')?;
    let hours: u32 = hours.trim().parse().ok()?;
    let minutes: u32 = minutes.trim().parse().ok()?;
    if hours < 24 && minutes < 60 {
        Some(hours * 60 + minutes)
    } else {
        None
    }
}

/// Where this machine's clock says it is: Monday-first weekday, then minute of
/// day.
///
/// SQLite, not a Rust timezone crate: `date('now','localtime')` is already the
/// authority that decides quota rollover in [`network_usage`], so the window and
/// the ledgers cannot disagree about when a day ends, and nothing new links in to
/// answer a question the bundled database already answers.
fn local_clock(conn: &Connection) -> Result<(u32, u32), String> {
    conn.query_row(
        "SELECT (strftime('%w', 'now', 'localtime') + 6) % 7, \
                CAST(strftime('%H', 'now', 'localtime') AS INTEGER) * 60 + \
                CAST(strftime('%M', 'now', 'localtime') AS INTEGER)",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32)),
    )
    .map_err(|e| format!("local_clock: {e}"))
}

/// Whether a transfer may start right now. A window nobody can read is an open
/// one: the safe answer is the choice a user made by never making one, the same
/// way an unreadable proxy row means direct.
pub fn window_open_now(conn: &Connection) -> bool {
    match DownloadWindow::read(conn) {
        // The common case is no window at all, and it should not have to ask the
        // clock: this is the answer the watcher reaches for every five seconds.
        Ok(window) if !window.enabled => true,
        Ok(window) => match local_clock(conn) {
            Ok((weekday, minute)) => window.open_at(weekday, minute),
            Err(_) => true,
        },
        Err(_) => true,
    }
}

/// The Basic pair for the proxy the user typed, stored with the proxy it belongs to.
///
/// `credentials` rather than a `settings` key, for one reason: `settings:list` hands
/// every value in that table to the page, so a password in it would cross the bridge
/// on every Settings open and sit in the webview's memory for no benefit. Here the
/// read that paints the row is this one, by one caller, and the list read never
/// selects the blob — the same rule [`list_sessions`] follows for cookies. The scheme
/// name `proxy` is already one the table's CHECK allows, so nothing about the schema
/// moves for it.
///
/// The blob is not encrypted, exactly as the session cookie beside it: same file,
/// same profile only this user can read, and code that can read it can already read
/// the browser's own store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyAuth {
    pub host: String,
    pub username: String,
    pub secret: String,
}

/// Both halves of a credential are short one-line values, and both go into a header
/// on every tunnel this proxy opens. A megabyte pasted into the password box is a
/// paste error, and the `Proxy-Authorization` it becomes is one the proxy will refuse
/// after the request line is already written.
pub const MAX_PROXY_USERNAME_BYTES: usize = 256;
pub const MAX_PROXY_SECRET_BYTES: usize = 1024;

/// The stored proxy credential, if there is one.
pub fn proxy_auth(conn: &Connection) -> Result<Option<ProxyAuth>, String> {
    let row = conn
        .query_row(
            "SELECT host, username, secret_blob FROM credentials \
             WHERE scheme = 'proxy' ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("read proxy credential: {e}"))?;
    // A row with no username is not a credential — the write path cannot leave one
    // (see [`put_proxy_auth`]), so this is a hand edit, and reading it as a usable
    // pair would put a nameless password on the wire.
    Ok(row.and_then(|(host, username, blob)| {
        username
            .filter(|name| !name.is_empty())
            .map(|username| ProxyAuth {
                host,
                username,
                secret: String::from_utf8_lossy(&blob).into_owned(),
            })
    }))
}

/// Replaces the proxy credential with exactly what the user just wrote, keyed to the
/// proxy that is in front of it right now.
///
/// Delete then insert, because `credentials` has no unique index on (scheme, host)
/// and one row for the whole app is this feature's rule, not the table's. The delete
/// clears every `proxy` row rather than only this host's: there is one manual proxy at
/// a time, and a row left behind for the URL before it would show on the screen as a
/// credential that is not in use.
pub fn put_proxy_auth(
    conn: &Connection,
    host: &str,
    username: &str,
    secret: &str,
    now: i64,
) -> Result<ProxyAuth, String> {
    conn.execute("DELETE FROM credentials WHERE scheme = 'proxy'", [])
        .map_err(|e| format!("forget proxy credential: {e}"))?;
    let auth = ProxyAuth {
        host: host.to_string(),
        username: username.to_string(),
        secret: secret.to_string(),
    };
    conn.execute(
        "INSERT INTO credentials (scheme, host, username, secret_blob, created_at) \
         VALUES ('proxy', ?1, ?2, ?3, ?4)",
        rusqlite::params![auth.host, auth.username, auth.secret.as_bytes(), now],
    )
    .map_err(|e| format!("store proxy credential: {e}"))?;
    Ok(auth)
}

/// Drops the proxy credential. Reports whether there was one to drop.
pub fn forget_proxy_auth(conn: &Connection) -> Result<bool, String> {
    conn.execute("DELETE FROM credentials WHERE scheme = 'proxy'", [])
        .map(|n| n > 0)
        .map_err(|e| format!("forget proxy credential: {e}"))
}

/// One host's Basic pair for the origin itself.
///
/// The same reasoning as [`ProxyAuth`] for the table it lives in — `settings:list`
/// hands every value it holds to the page, so a password there would cross the bridge
/// on every Settings open — and one more, which is why there are several of these rows
/// where the proxy has exactly one: origin credentials are keyed per host, because the
/// host is what the user is looking at when a link refuses them.
///
/// `scheme` is which of the two kinds a row is, and both of them are already in the
/// table's CHECK list — a token is stored as `scheme='header-secret'`, the word the schema
/// has allowed since before this app could use it, so no schema version is owed. It is
/// *not* a `scheme = 'cookie'` row, though: for cookies `username` holds the
/// User-Agent the session was copied with (`session_for` below), and an origin row
/// must not inherit that overload — here `username` is the name that goes into the
/// header, which is what Basic's own definition says it is, and a token row leaves it
/// empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginAuth {
    pub host: String,
    pub kind: crate::engine::auth::Kind,
    pub username: String,
    pub secret: String,
}

/// Both halves go into a header on every request to that host, so they are short
/// one-line values. The same limit the proxy pair carries, for the same reason: a
/// megabyte pasted into the password box is a paste error, and the header it becomes is
/// one the server will refuse after the request line is already written.
pub const MAX_ORIGIN_USERNAME_BYTES: usize = 256;
pub const MAX_ORIGIN_SECRET_BYTES: usize = 1024;

/// Every stored origin credential, with the secrets — the boot read that hands the
/// whole set to [`engine::auth::configure`].
///
/// All of them at once rather than one lookup per host, so the engine's mirror is a
/// complete copy of the table and not a cache with an eviction rule to get wrong. The
/// list is small by construction: one row per host the user has given a credential for.
///
/// Both origin kinds come back whether or not their halves are filled in. Whether a row
/// can become a header is one rule and it lives in the mirror
/// ([`crate::engine::auth::Kind`] and its `usable` check), not twice — a `WHERE username <>
/// ''` here would be the second copy of it, and would also be wrong for a token, which has
/// no username by design.
pub fn origin_auths(conn: &Connection) -> Result<Vec<OriginAuth>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT scheme, host, username, secret_blob FROM credentials \
             WHERE scheme IN ('basic', 'header-secret') \
             ORDER BY host",
        )
        .map_err(|e| format!("read origin credentials: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(OriginAuth {
                kind: crate::engine::auth::Kind::from_scheme(&r.get::<_, String>(0)?),
                host: r.get(1)?,
                username: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                secret: String::from_utf8_lossy(&r.get::<_, Vec<u8>>(3)?).into_owned(),
            })
        })
        .map_err(|e| format!("read origin credentials: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Every stored origin credential, without the secrets: what the Settings list shows.
pub fn list_origin_auths(conn: &Connection) -> Result<Vec<OriginSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT scheme, host, username, length(secret_blob), created_at FROM credentials \
             WHERE scheme IN ('basic', 'header-secret') ORDER BY created_at, id",
        )
        .map_err(|e| format!("list origin credentials: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(OriginSummary {
                kind: crate::engine::auth::Kind::from_scheme(&r.get::<_, String>(0)?),
                host: r.get(1)?,
                username: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                secret_bytes: r.get::<_, i64>(3)? as usize,
                created_at: r.get(4)?,
            })
        })
        .map_err(|e| format!("list origin credentials: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// What the page may know about one stored origin credential. `host` is the key, so the
/// Settings list can say which host a password belongs to, `kind` is which header it
/// becomes — so a row can say `Bearer token` instead of showing an empty name where a
/// username would be — and `secret_bytes` is the only thing the answer can say about the
/// secret: how big it is, not what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginSummary {
    pub host: String,
    pub kind: crate::engine::auth::Kind,
    pub username: String,
    pub secret_bytes: usize,
    pub created_at: i64,
}

/// Replaces one host's origin credential with exactly what the user just wrote.
///
/// Delete then insert, the same reason [`put_session`] gives: `credentials` has no
/// unique index on (scheme, host), so one row per host is this feature's rule. Only
/// this host's row goes, unlike the proxy's — here several hosts are the point — and
/// *both* origin scheme names go with it, because the mirror holds one credential per host
/// and a token typed where a password lived replaces it rather than joining it. A host
/// can never hold both: the middleware writes one `Authorization`.
///
/// The delete is by scheme name, never by the `'cookie'` rows that share this table —
/// a pasted session is a different feature with a different key meaning.
pub fn put_origin_auth(
    conn: &Connection,
    host: &str,
    kind: crate::engine::auth::Kind,
    username: &str,
    secret: &str,
    now: i64,
) -> Result<OriginAuth, String> {
    conn.execute(
        "DELETE FROM credentials WHERE scheme IN ('basic', 'header-secret') AND host = ?1",
        [host],
    )
    .map_err(|e| format!("forget origin credential: {e}"))?;
    let auth = OriginAuth {
        host: host.to_string(),
        kind,
        username: username.to_string(),
        secret: secret.to_string(),
    };
    conn.execute(
        "INSERT INTO credentials (scheme, host, username, secret_blob, created_at) \
         VALUES (?5, ?1, ?2, ?3, ?4)",
        rusqlite::params![
            auth.host,
            auth.username,
            auth.secret.as_bytes(),
            now,
            kind.scheme()
        ],
    )
    .map_err(|e| format!("store origin credential: {e}"))?;
    Ok(auth)
}

/// Drops one host's origin credential, of either kind. Reports whether there was one to
/// drop — which is why the filter is the pair of scheme names and not the kind: the page
/// forgets by host, because a host with a refused credential is the thing the user is
/// looking at, and asking it to remember which kind it typed would be a second copy of a
/// fact this table already holds.
pub fn forget_origin_auth(conn: &Connection, host: &str) -> Result<bool, String> {
    conn.execute(
        "DELETE FROM credentials WHERE scheme IN ('basic', 'header-secret') AND host = ?1",
        [host],
    )
    .map(|n| n > 0)
    .map_err(|e| format!("forget origin credential: {e}"))
}

/// Upserts a setting; `None` removes the key.
pub fn set_setting(conn: &Connection, key: &str, value: Option<&str>) -> Result<(), String> {
    match value {
        Some(value) => conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        ),
        None => conn.execute("DELETE FROM settings WHERE key = ?1", [key]),
    }
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// The two things a browser sends that an edge-protection site checks, kept for
/// one host.
///
/// Stored in `credentials` rather than in a new table: that table is the schema's
/// place for "a secret tied to a host", `scheme = 'cookie'` is the name it already
/// gives this kind, and the columns are the ones this needs — `host` the key,
/// `username` the User-Agent the cookie was copied with (Cloudflare binds
/// `cf_clearance` to the UA that earned it), `secret_blob` the Cookie value,
/// `created_at` the stamp the settings list shows. A new table would cost a schema
/// version for nothing.
///
/// The blob is not encrypted. It sits in the same SQLite file as the rest of the
/// library, inside a profile only this user can read; code that can read this file
/// can already read the browser's own cookie store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserSession {
    pub cookie: String,
    pub user_agent: Option<String>,
}

/// What the settings list shows for one host. The cookie itself is not in it: the
/// renderer never needs the value back, and echoing a secret across the bridge to
/// paint a row would put it in the webview's memory for nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub host: String,
    pub user_agent: Option<String>,
    pub cookie_bytes: usize,
    pub created_at: i64,
}

/// The longest Cookie value accepted. A whole browser profile pasted by mistake
/// is not a session for one download, and an unbounded blob in a row that every
/// probe reads back is not a thing to leave lying around.
pub const MAX_COOKIE_BYTES: usize = 8192;
/// A User-Agent is a single short line; anything longer is a paste error.
pub const MAX_USER_AGENT_BYTES: usize = 512;

/// Checks a pasted Cookie header and returns it in the form that goes on the wire.
///
/// Rejects what would break the request line rather than what looks unlikely: a
/// carriage return or line feed is how a pasted header smuggles a second one, and
/// NUL truncates the value inside SQLite's own C boundary. A leading `Cookie:` is
/// dropped because devtools copies the whole line.
pub fn validate_cookie(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value = trimmed
        .strip_prefix("Cookie:")
        .or_else(|| trimmed.strip_prefix("cookie:"))
        .map(str::trim_start)
        .unwrap_or(trimmed);
    if value.is_empty() {
        return Err("a cookie needs a name and a value".to_string());
    }
    if value.len() > MAX_COOKIE_BYTES {
        return Err(format!(
            "a cookie longer than {MAX_COOKIE_BYTES} bytes is not one session"
        ));
    }
    if value
        .bytes()
        .any(|b| b < 0x20 || b == 0x7f || (0x80..=0x9f).contains(&b))
    {
        return Err("a cookie cannot contain a control character".to_string());
    }
    if !value.contains('=') {
        return Err("a cookie looks like name=value".to_string());
    }
    Ok(Some(value.to_string()))
}

/// Checks a pasted User-Agent. Empty clears it, so the app's own identity is used.
pub fn validate_user_agent(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_USER_AGENT_BYTES {
        return Err(format!(
            "a user agent longer than {MAX_USER_AGENT_BYTES} bytes is not one"
        ));
    }
    if trimmed
        .bytes()
        .any(|b| b < 0x20 || b == 0x7f || (0x80..=0x9f).contains(&b))
    {
        return Err("a user agent cannot contain a control character".to_string());
    }
    Ok(Some(trimmed.to_string()))
}

/// The stored session for one host, if the user pasted one.
pub fn session_for(conn: &Connection, host: &str) -> Result<Option<BrowserSession>, String> {
    let row = conn
        .query_row(
            "SELECT secret_blob, username FROM credentials \
             WHERE scheme = 'cookie' AND host = ?1 \
             ORDER BY id DESC LIMIT 1",
            [host],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|e| format!("read session: {e}"))?;
    Ok(row.map(|(blob, ua)| BrowserSession {
        cookie: String::from_utf8_lossy(&blob).into_owned(),
        user_agent: ua.filter(|s| !s.is_empty()),
    }))
}

/// `session_for` for a caller that does not own the IPC connection.
///
/// The link check runs off the main thread and must not wait behind the lock every
/// other command holds, so it opens the library for this one read. No session is a
/// possible answer here; a database that will not open is the same answer, because
/// a failed lookup must not be reported to the user as a refused link.
pub fn session_at(db_path: &Path, host: &str) -> Option<BrowserSession> {
    if host.is_empty() {
        return None;
    }
    let conn = open(db_path).ok()?;
    session_for(&conn, host).ok().flatten()
}

/// Replaces one host's session with exactly what the user just wrote.
///
/// A write is a delete then an insert because `credentials` has no unique index on
/// (scheme, host): one row per host is this feature's rule, not the table's. The
/// cookie is required — `forget_session` is how a session goes away — so a half
/// empty form cannot leave a session that only carries a user agent.
pub fn put_session(
    conn: &Connection,
    host: &str,
    cookie: &str,
    user_agent: Option<&str>,
    now: i64,
) -> Result<BrowserSession, String> {
    conn.execute(
        "DELETE FROM credentials WHERE scheme = 'cookie' AND host = ?1",
        [host],
    )
    .map_err(|e| format!("forget session: {e}"))?;
    let session = BrowserSession {
        cookie: cookie.to_string(),
        user_agent: user_agent.map(str::to_string),
    };
    conn.execute(
        "INSERT INTO credentials (scheme, host, username, secret_blob, created_at) \
         VALUES ('cookie', ?1, ?2, ?3, ?4)",
        rusqlite::params![host, session.user_agent, session.cookie.as_bytes(), now],
    )
    .map_err(|e| format!("store session: {e}"))?;
    Ok(session)
}

/// Drops one host's session. Reports whether there was one to drop.
pub fn forget_session(conn: &Connection, host: &str) -> Result<bool, String> {
    conn.execute(
        "DELETE FROM credentials WHERE scheme = 'cookie' AND host = ?1",
        [host],
    )
    .map(|n| n > 0)
    .map_err(|e| format!("forget session: {e}"))
}

/// Every stored session, oldest first, without the secrets.
pub fn list_sessions(conn: &Connection) -> Result<Vec<SessionSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT host, username, length(secret_blob), created_at FROM credentials \
             WHERE scheme = 'cookie' ORDER BY created_at, id",
        )
        .map_err(|e| format!("list sessions: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SessionSummary {
                host: r.get(0)?,
                user_agent: r.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty()),
                cookie_bytes: r.get::<_, i64>(2)? as usize,
                created_at: r.get(3)?,
            })
        })
        .map_err(|e| format!("list sessions: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// What the user has chosen for one adapter. `mac_hint` is the stable identity;
/// the live facts (address, metric) come from Windows on every refresh.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkPrefs {
    pub id: i64,
    pub enabled: bool,
    pub speed_limit: Option<i64>,
    pub data_limit: Option<i64>,
    pub data_limit_period: Option<String>,
    pub custom_name: Option<String>,
    /// The stripe this adapter owns for good. See [`note_network`].
    pub color_id: String,
}

/// How many NIC stripes the palette has: `--n1` to `--n6` in `tokens.css`.
/// A stripe beyond the last one has no colour to paint with, so allocation
/// stops here rather than inventing an index the renderer cannot resolve.
const NET_COLOR_SLOTS: u32 = 6;

/// The first stripe no other row holds.
///
/// Rows are never deleted, so a pulled adapter keeps its colour and the
/// newcomer must not take it. Only when all four are spoken for does a row with
/// no bytes behind it give one up: nothing can be misread, because there is no
/// history pointing at that colour.
fn next_color(conn: &Connection) -> Result<String, String> {
    let mut stmt = conn
        .prepare("SELECT color_id FROM networks WHERE color_id IS NOT NULL")
        .map_err(|e| format!("read colours: {e}"))?;
    let taken: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| format!("read colours: {e}"))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    for i in 1..=NET_COLOR_SLOTS {
        let candidate = format!("n{i}");
        if !taken.iter().any(|held| held == &candidate) {
            return Ok(candidate);
        }
    }
    conn.query_row(
        "SELECT n.color_id FROM networks n \
         WHERE n.color_id IS NOT NULL \
           AND NOT EXISTS (SELECT 1 FROM block_attempts a WHERE a.network_id = n.id) \
           AND NOT EXISTS (SELECT 1 FROM net_usage u WHERE u.network_id = n.id) \
         ORDER BY n.id LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .map_err(|e| format!("reclaim colour: {e}"))
}

/// Gives every row its own stripe, once, at boot.
///
/// The colour used to follow the enumeration order, so two adapters could end up
/// wearing the same stripe and the map could not tell their bytes apart. Keeping
/// the colour stable stops that happening to a new row; it cannot undo it for the
/// rows the old code already collided, and a database from before the fix has
/// those collisions on disk. The lowest id keeps the first stripe and a row only
/// moves off a colour an earlier row already holds, so a clean database is left
/// alone and this is safe to run on every boot.
///
/// Past the last stripe the palette cannot help: the seventh adapter shares a
/// colour with something, and the routes block names both rows instead of the
/// map inventing a stripe the renderer has no value for.
fn restripe(conn: &Connection) -> Result<(), String> {
    // A database old enough to predate the networks table has nothing to
    // separate; the schema pass that follows will give it one.
    if !table_exists(conn, "networks")? {
        return Ok(());
    }
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT id FROM networks ORDER BY id")
            .map_err(|e| format!("read network ids: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| format!("read network ids: {e}"))?;
        rows.filter_map(Result::ok).collect()
    };

    let mut held: Vec<String> = Vec::new();
    for id in ids {
        let current: Option<String> = conn
            .query_row("SELECT color_id FROM networks WHERE id = ?1", [id], |r| r.get(0))
            .ok()
            .flatten();
        let want = current
            .clone()
            .filter(|c| !held.contains(c))
            .or_else(|| {
                (1..=NET_COLOR_SLOTS)
                    .map(|i| format!("n{i}"))
                    .find(|c| !held.contains(c))
            })
            .or(current.clone());
        let Some(want) = want else { continue };

        let changed = current.as_deref() != Some(want.as_str());
        held.push(want.clone());
        if changed {
            conn.execute(
                "UPDATE networks SET color_id = ?1 WHERE id = ?2",
                rusqlite::params![want, id],
            )
            .map_err(|e| format!("restripe row {id}: {e}"))?;
        }
    }
    Ok(())
}

/// Records the row for one adapter, or refreshes its live description, and
/// returns the choices the user made for it.
///
/// Rows are never deleted: `net_usage` and `block_attempts` point at them, so
/// the history of an unplugged adapter has to survive. `mac_hint` has no unique
/// index, so this is a lookup then a write rather than an UPSERT; every caller
/// holds the same connection lock, so it cannot race.
///
/// The colour is chosen once, at the first sight of an identity, and never
/// rewritten. It is the key the Sucker Map joins an old attempt row to a card
/// with, so a value that followed the enumeration order would repaint every
/// finished download whenever a cable was pulled.
pub fn note_network(
    conn: &Connection,
    hint: &str,
    display_name: &str,
    kind: &str,
    default_enabled: bool,
) -> Result<NetworkPrefs, String> {
    let existing: Option<i64> = conn
        .query_row("SELECT id FROM networks WHERE mac_hint = ?1", [hint], |r| r.get(0))
        .ok();

    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE networks SET display_name = ?1, kind = ?2 WHERE id = ?3",
                rusqlite::params![display_name, kind, id],
            )
            .map_err(|e| format!("refresh network: {e}"))?;
        }
        None => {
            let color_id = next_color(conn)?;
            conn.execute(
                "INSERT INTO networks (mac_hint, display_name, kind, color_id, enabled) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![hint, display_name, kind, color_id, default_enabled as i64],
            )
            .map_err(|e| format!("record network: {e}"))?;
        }
    }

    conn.query_row(
        "SELECT id, enabled, speed_limit, data_limit, data_limit_period, custom_name, color_id \
         FROM networks WHERE mac_hint = ?1",
        [hint],
        |r| {
            Ok(NetworkPrefs {
                id: r.get(0)?,
                enabled: r.get::<_, i64>(1)? == 1,
                speed_limit: r.get::<_, Option<i64>>(2)?,
                data_limit: r.get::<_, Option<i64>>(3)?,
                data_limit_period: r.get::<_, Option<String>>(4)?,
                custom_name: r.get::<_, Option<String>>(5)?,
                color_id: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        },
    )
    .map_err(|e| format!("read network prefs: {e}"))
}

/// Turns an adapter's user choice on or off. Returns false if no such row.
pub fn set_network_enabled(conn: &Connection, id: i64, enabled: bool) -> Result<bool, String> {
    let changed = conn
        .execute(
            "UPDATE networks SET enabled = ?1 WHERE id = ?2",
            rusqlite::params![enabled as i64, id],
        )
        .map_err(|e| format!("set network enabled: {e}"))?;
    Ok(changed > 0)
}

/// Saves an adapter's rate cap and data allowance. `None` writes NULL, which is
/// the same as saying "no limit here" — the columns are nullable already, so a
/// cleared control needs no sentinel. Returns false if no such row.
///
/// The period travels with the allowance and is cleared with it, because an
/// allowance with no window is a number nothing can enforce.
pub fn update_network_limits(
    conn: &Connection,
    id: i64,
    speed_limit: Option<i64>,
    data_limit: Option<i64>,
    data_limit_period: Option<&str>,
) -> Result<bool, String> {
    let changed = conn
        .execute(
            "UPDATE networks SET speed_limit = ?1, data_limit = ?2, data_limit_period = ?3 \
             WHERE id = ?4",
            rusqlite::params![speed_limit, data_limit, data_limit_period, id],
        )
        .map_err(|e| format!("set network limits: {e}"))?;
    Ok(changed > 0)
}

/// One adapter the user has put in the bonding pool.
#[derive(Clone, Debug)]
pub struct BondMember {
    pub id: i64,
    /// The key `engine::network::identity` produces for this adapter.
    pub mac_hint: String,
    /// The stripe colour the Sucker Map was told to use for this adapter.
    pub color_id: Option<String>,
    /// What the card calls it: the user's rename when there is one.
    pub label: String,
}

/// The adapters a new job may spread its chunks over, oldest row first.
///
/// The colour comes from the row, not from the position in the current
/// enumeration: unplugging a dongle shifts every later index, and a stripe
/// that silently changed colour mid-download would misreport which device
/// carried the bytes.
pub fn bond_members(conn: &Connection) -> Result<Vec<BondMember>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, mac_hint, color_id, COALESCE(custom_name, display_name) \
             FROM networks WHERE enabled = 1 AND mac_hint IS NOT NULL ORDER BY id",
        )
        .map_err(|e| format!("read bonded networks: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(BondMember {
                id: r.get(0)?,
                mac_hint: r.get(1)?,
                color_id: r.get(2)?,
                label: r.get(3)?,
            })
        })
        .map_err(|e| format!("read bonded networks: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The row for one adapter, whether or not the user put it in the bonding pool.
///
/// Attribution has to be able to name a device that is on standby: switching it
/// off takes it out of *our* binding list, but its interface can still be the
/// one the routing table used, and those bytes are a fact about that card.
pub fn network_for_hint(conn: &Connection, hint: &str) -> Result<Option<BondMember>, String> {
    conn.query_row(
        "SELECT id, mac_hint, color_id, COALESCE(custom_name, display_name) \
         FROM networks WHERE mac_hint = ?1 ORDER BY id LIMIT 1",
        [hint],
        |r| {
            Ok(BondMember {
                id: r.get(0)?,
                mac_hint: r.get(1)?,
                color_id: r.get(2)?,
                label: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("read network row: {e}"))
}

/// Charges a finished attempt's bytes to the device that carried them.
///
/// One row per adapter per calendar day, keyed by the local date SQLite
/// derives from the OS clock — no timezone database in the binary. The
/// `day`/`week`/`month` columns are left at 0; those windows are sums over
/// `period_key` (see `network_usage`).
pub fn add_net_usage(conn: &Connection, network_id: i64, bytes: u64) -> Result<(), String> {
    if bytes == 0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO net_usage (network_id, period_key, bytes, day, week, month) \
         VALUES (?1, date('now', 'localtime'), ?2, 0, 0, 0) \
         ON CONFLICT(network_id, period_key) DO UPDATE SET bytes = bytes + excluded.bytes",
        rusqlite::params![network_id, bytes.min(i64::MAX as u64) as i64],
    )
    .map(|_| ())
    .map_err(|e| format!("record network usage: {e}"))
}

/// Books bytes that no adapter was credited with, to the same calendar-day key
/// `net_usage` uses, so the two ledgers add up to what Sucr actually moved.
///
/// Not a `net_usage` row for a made-up adapter: charging a byte to an interface
/// that did not carry it is the untruth the proxy slice was built to remove, and
/// a synthetic `networks` row would answer "which devices exist" with one that
/// does not, reaching the limiter and the deck as well as the ledger.
pub fn add_unattributed_usage(conn: &Connection, bytes: u64) -> Result<(), String> {
    if bytes == 0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO unattributed_usage (period_key, bytes) \
         VALUES (date('now', 'localtime'), ?1) \
         ON CONFLICT(period_key) DO UPDATE SET bytes = bytes + excluded.bytes",
        rusqlite::params![bytes.min(i64::MAX as u64) as i64],
    )
    .map(|_| ())
    .map_err(|e| format!("record unattributed usage: {e}"))
}

/// Bytes this adapter has carried today, this week (Monday-start), this
/// calendar month, and ever.
///
/// The three window starts come from SQLite because it already knows the local
/// timezone; the sums are then plain lexicographic compares over `period_key`.
/// The lifetime figure is the whole table for this adapter, because a day that
/// has fallen out of the month still happened.
pub fn network_usage(conn: &Connection, network_id: i64) -> Result<(i64, i64, i64, i64), String> {
    let (today, week_start, month_start) = usage_windows(conn)?;
    conn.query_row(
        "SELECT \
           COALESCE(SUM(CASE WHEN period_key = ?2 THEN bytes END), 0), \
           COALESCE(SUM(CASE WHEN period_key BETWEEN ?3 AND ?2 THEN bytes END), 0), \
           COALESCE(SUM(CASE WHEN period_key BETWEEN ?4 AND ?2 THEN bytes END), 0), \
           COALESCE(SUM(bytes), 0) \
         FROM net_usage WHERE network_id = ?1",
        rusqlite::params![network_id, today, week_start, month_start],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .map_err(|e| format!("read network usage: {e}"))
}

/// The same four windows as `network_usage`, over the bytes no adapter was
/// credited with — see `add_unattributed_usage` for why they are not in
/// `net_usage`.
pub fn unattributed_usage(conn: &Connection) -> Result<(i64, i64, i64, i64), String> {
    let (today, week_start, month_start) = usage_windows(conn)?;
    conn.query_row(
        "SELECT \
           COALESCE(SUM(CASE WHEN period_key = ?1 THEN bytes END), 0), \
           COALESCE(SUM(CASE WHEN period_key BETWEEN ?2 AND ?1 THEN bytes END), 0), \
           COALESCE(SUM(CASE WHEN period_key BETWEEN ?3 AND ?1 THEN bytes END), 0), \
           COALESCE(SUM(bytes), 0) \
         FROM unattributed_usage",
        rusqlite::params![today, week_start, month_start],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .map_err(|e| format!("read unattributed usage: {e}"))
}

/// Today, this Monday, and this month's first, as the `period_key` strings the
/// ledgers are keyed by. Shared so both windows roll at the same instant.
fn usage_windows(conn: &Connection) -> Result<(String, String, String), String> {
    conn.query_row(
        "SELECT date('now', 'localtime'), \
         date('now', 'localtime', '-' || ((strftime('%w', 'now', 'localtime') + 6) % 7) || ' days'), \
         date('now', 'localtime', 'start of month')",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .map_err(|e| format!("read usage windows: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sucr-db-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("sucr.db")
    }

    #[test]
    fn creates_schema_v4_in_wal() {
        let path = temp_db("creates");
        let conn = open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 4);
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        let sync: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
        assert_eq!(sync, 1, "WAL must run NORMAL, not FULL: per-commit fsyncs serialize the supervisor on a spinning data drive");
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 17);
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_refresh_keeps_the_row_and_the_users_choices() {
        let path = temp_db("network-refresh");
        let conn = open(&path).unwrap();
        let first =
            note_network(&conn, "AA:BB:CC:00:00:01", "Ethernet", "ethernet", true).unwrap();
        assert!(first.enabled);

        conn.execute(
            "UPDATE networks SET enabled = 0, custom_name = 'Office cable' WHERE id = ?1",
            rusqlite::params![first.id],
        )
        .unwrap();

        let again =
            note_network(&conn, "AA:BB:CC:00:00:01", "Ethernet 2", "ethernet", true).unwrap();

        assert_eq!(again.id, first.id, "the same adapter keeps its row id");
        assert!(!again.enabled, "a refresh must not re-enable what the user turned off");
        assert_eq!(again.custom_name.as_deref(), Some("Office cable"));

        let stored: String = conn
            .query_row(
                "SELECT display_name FROM networks WHERE id = ?1",
                rusqlite::params![first.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "Ethernet 2", "Windows' own name still wins");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_tunnel_is_recorded_but_not_bonded_by_default() {
        let path = temp_db("network-tunnel");
        let conn = open(&path).unwrap();
        let prefs =
            note_network(&conn, "ifindex:33", "Tailscale", "tunnel", false).unwrap();
        assert!(!prefs.enabled);
        assert!(set_network_enabled(&conn, prefs.id, true).unwrap());
        assert_eq!(set_network_enabled(&conn, prefs.id, false).unwrap(), true);
        assert!(
            !set_network_enabled(&conn, 9999, true).unwrap(),
            "an id that was never enumerated is not a silent success"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn reopening_does_not_reapply() {
        let path = temp_db("reopen");
        drop(open(&path).unwrap());
        let conn = open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 4);
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn migrates_v1_to_current() {
        let path = temp_db("migrate");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE block_attempts (\
                   id INTEGER PRIMARY KEY, download_id INTEGER NOT NULL, block_idx INTEGER NOT NULL, \
                   network_id INTEGER, started_at INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0, \
                   state TEXT NOT NULL, reason TEXT);\
                 CREATE TABLE downloads (id INTEGER PRIMARY KEY);\
                 INSERT INTO block_attempts (download_id, block_idx, started_at, bytes, state) \
                   VALUES (1, 0, 10, 5, 'done');\
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let conn = open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 4);
        let ended: Option<i64> = conn
            .query_row("SELECT ended_at FROM block_attempts WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ended, None);
        let col: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('downloads') WHERE name = 'speed_limit'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(col, 1);
        let detail: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('block_attempts') WHERE name = 'detail'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(detail, 1);
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn migrates_v2_to_current() {
        let path = temp_db("migrate-v2");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE downloads (id INTEGER PRIMARY KEY);\
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }
        let conn = open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 4);
        let col: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('downloads') WHERE name = 'speed_limit'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(col, 1);
        set_setting(&conn, "global_speed_limit", Some("1048576")).unwrap();
        assert_eq!(setting(&conn, "global_speed_limit").unwrap().as_deref(), Some("1048576"));
        set_setting(&conn, "global_speed_limit", None).unwrap();
        assert_eq!(setting(&conn, "global_speed_limit").unwrap(), None);
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The v3 -> v4 arm is the unattributed ledger alone: a library that already
    /// booked adapter usage keeps every byte of it, and the new table starts
    /// empty rather than absorbing what the old one recorded.
    ///
    /// The fixture drops the table before demoting the version, because that is
    /// what a real v3 file looks like — a file that already has the ledger is a
    /// v4 file, whatever its `user_version` says.
    #[test]
    fn migrates_v3_to_v4_without_touching_adapter_ledgers() {
        let path = temp_db("migrate-v3");
        {
            let conn = open(&path).unwrap();
            let adapter =
                note_network(&conn, "AA:BB:CC:00:00:09", "Ethernet", "ethernet", true).unwrap();
            add_net_usage(&conn, adapter.id, 4096).unwrap();
            conn.execute_batch("DROP TABLE unattributed_usage; PRAGMA user_version = 3;")
                .unwrap();
            let before = network_usage(&conn, adapter.id).unwrap();
            assert_eq!(before.3, 4096, "the v3 ledger is the fixture");
        }
        let conn = open(&path).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 4);
        let adapter_id: i64 = conn
            .query_row(
                "SELECT id FROM networks WHERE mac_hint = 'AA:BB:CC:00:00:09'",
                [],
                |r| r.get(0),
            )
            .expect("the adapter row survives the upgrade");
        assert_eq!(
            network_usage(&conn, adapter_id).unwrap().3,
            4096,
            "an upgrade moves no byte between ledgers"
        );
        let unattributed = unattributed_usage(&conn).unwrap();
        assert_eq!(unattributed, (0, 0, 0, 0), "the new ledger starts empty");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The unattributed ledger keeps the same day-keyed shape as `net_usage`, so
    /// the pane can add the two and read one set of windows off the sum.
    #[test]
    fn unattributed_usage_adds_up_its_own_days() {
        let path = temp_db("unattributed-usage");
        let conn = open(&path).unwrap();

        add_unattributed_usage(&conn, 0).unwrap();
        assert_eq!(
            unattributed_usage(&conn).unwrap(),
            (0, 0, 0, 0),
            "a zero-byte attempt writes no row, the same rule `add_net_usage` uses"
        );

        add_unattributed_usage(&conn, 1000).unwrap();
        add_unattributed_usage(&conn, 500).unwrap();
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM unattributed_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "one row per day, whatever the number of attempts");

        let yesterday = conn
            .query_row("SELECT date('now', 'localtime', '-1 day')", [], |r| r.get::<_, String>(0))
            .unwrap();
        conn.execute(
            "INSERT INTO unattributed_usage (period_key, bytes) VALUES (?1, 250)",
            [&yesterday],
        )
        .unwrap();

        let (today, week, month, lifetime) = unattributed_usage(&conn).unwrap();
        assert_eq!(today, 1500, "today is today's key alone");
        assert_eq!(lifetime, 1750, "lifetime is the whole table");
        assert!(
            week >= today && month >= today && lifetime >= week && lifetime >= month,
            "the windows nest: today {today} week {week} month {month} lifetime {lifetime}"
        );

        let adapters: i64 = conn
            .query_row("SELECT count(*) FROM net_usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(adapters, 0, "booking a proxied byte credits no interface");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn repairs_v3_downloads_missing_probe_columns() {
        let path = temp_db("repair-v3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE downloads (id INTEGER PRIMARY KEY, url TEXT NOT NULL, \
                 filename TEXT NOT NULL);\
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }
        let conn = open(&path).unwrap();
        let added: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('downloads') \
                 WHERE name IN ('content_type', 'staging_path', 'etag', 'last_modified', \
                 'streams_final')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(added, 5);
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_fresh_library_is_seeded_once() {
        let path = temp_db("categories-seed");
        let names = |conn: &Connection| {
            let mut stmt = conn
                .prepare("SELECT name FROM categories ORDER BY sort_order, id")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };

        let conn = open(&path).unwrap();
        assert_eq!(
            names(&conn),
            ["Archives", "Videos", "Audio", "Documents", "Programs"],
            "the rail's five names, now backed by rows"
        );
        drop(conn);

        let conn = open(&path).unwrap();
        assert_eq!(names(&conn).len(), 5, "a second open adds nothing");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_extension_rules_in_the_table_are_what_the_writer_uses() {
        let path = temp_db("categories-rules");
        let conn = open(&path).unwrap();
        let id_of = |name: &str| -> i64 {
            conn.query_row("SELECT id FROM categories WHERE name = ?1", [name], |r| r.get(0))
                .unwrap()
        };
        let category = |file: &str| category_for_file(&conn, file).unwrap();

        assert_eq!(category("game.iso"), Some(id_of("Archives")));
        assert_eq!(category("movie.tar.gz"), Some(id_of("Archives")), "the last dot wins");
        assert_eq!(category("SHOW.MP4"), Some(id_of("Videos")), "case is not a rule");
        assert_eq!(category("song.flac"), Some(id_of("Audio")));
        assert_eq!(category("manual.pdf"), Some(id_of("Documents")));
        assert_eq!(category("setup.msi"), Some(id_of("Programs")));
        assert_eq!(category("LICENSE"), None, "a name with no dot is not filed");
        assert_eq!(category("photo."), None, "a trailing dot is not an extension");
        assert_eq!(
            category("screenshot.png"),
            None,
            "images are not one of the five the rail lists"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_library_written_before_the_writer_is_filed_on_the_next_open() {
        let path = temp_db("categories-backfill");
        {
            let conn = open(&path).unwrap();
            for (id, file) in [(1, "a.zip"), (2, "b.mp3"), (3, "LICENSE")] {
                conn.execute(
                    "INSERT INTO downloads (id, url, filename, created_at) \
                     VALUES (?1, ?2, ?3, 1)",
                    rusqlite::params![id, format!("http://127.0.0.1/{file}"), file],
                )
                .unwrap();
            }
        }
        let conn = open(&path).unwrap();
        let filed = |id: i64| -> Option<i64> {
            conn.query_row(
                "SELECT category_id FROM downloads WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        let archives: i64 = conn
            .query_row("SELECT id FROM categories WHERE name = 'Archives'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(filed(1), Some(archives), "backfilled by extension");
        assert!(filed(2).is_some(), "every matching row is filed");
        assert_eq!(filed(3), None, "and a matching row is not");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_category_override_needs_both_rows_to_exist() {
        let path = temp_db("categories-assign");
        let conn = open(&path).unwrap();
        conn.execute(
            "INSERT INTO downloads (id, url, filename, created_at) \
             VALUES (7, 'http://127.0.0.1/x', 'x.zip', 1)",
            [],
        )
        .unwrap();
        let archives: i64 = conn
            .query_row("SELECT id FROM categories WHERE name = 'Archives'", [], |r| r.get(0))
            .unwrap();
        let audio: i64 = conn
            .query_row("SELECT id FROM categories WHERE name = 'Audio'", [], |r| r.get(0))
            .unwrap();

        assert!(assign_category(&conn, 7, Some(audio)).is_ok());
        let stored: i64 = conn
            .query_row("SELECT category_id FROM downloads WHERE id = 7", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, audio, "an override wins over the extension rule");

        assert!(assign_category(&conn, 7, None).is_ok());
        let cleared: Option<i64> = conn
            .query_row("SELECT category_id FROM downloads WHERE id = 7", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cleared, None);

        assert_eq!(
            assign_category(&conn, 8, Some(archives)).unwrap_err(),
            "download 8 not found"
        );
        assert_eq!(
            assign_category(&conn, 7, Some(99)).unwrap_err(),
            "no such category: 99"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn usage_adds_to_one_row_per_adapter_day() {
        let path = temp_db("net-usage");
        let conn = open(&path).unwrap();
        let n = note_network(&conn, "AA:BB:CC:00:00:09", "Ethernet", "ethernet", true).unwrap();

        add_net_usage(&conn, n.id, 0).unwrap();
        assert_eq!(
            network_usage(&conn, n.id).unwrap(),
            (0, 0, 0, 0),
            "a zero-byte attempt writes no row"
        );

        add_net_usage(&conn, n.id, 1000).unwrap();
        add_net_usage(&conn, n.id, 500).unwrap();
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM net_usage WHERE network_id = ?1", [n.id], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "a day is one row, not one row per attempt");
        assert_eq!(
            network_usage(&conn, n.id).unwrap(),
            (1500, 1500, 1500, 1500),
            "today is inside all three windows, and lifetime equals it"
        );

        let future: String = conn
            .query_row("SELECT date('now', 'localtime', '+1 day')", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO net_usage (network_id, period_key, bytes, day, week, month) \
             VALUES (?1, ?2, 9999, 0, 0, 0)",
            rusqlite::params![n.id, future],
        )
        .unwrap();
        assert_eq!(
            network_usage(&conn, n.id).unwrap(),
            (1500, 1500, 1500, 11499),
            "a window stops at today; lifetime counts every row in the table"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_day_outside_the_month_still_counts_as_lifetime() {
        let path = temp_db("net-lifetime");
        let conn = open(&path).unwrap();
        let n = note_network(&conn, "AA:BB:CC:00:00:10", "Ethernet", "ethernet", true).unwrap();

        add_net_usage(&conn, n.id, 4096).unwrap();
        conn.execute(
            "INSERT INTO net_usage (network_id, period_key, bytes, day, week, month) \
             VALUES (?1, '1999-12-31', 900, 0, 0, 0)",
            rusqlite::params![n.id],
        )
        .unwrap();

        let (today, week, month, lifetime) = network_usage(&conn, n.id).unwrap();
        assert_eq!(
            (today, week, month),
            (4096, 4096, 4096),
            "the old day is out of all three windows"
        );
        assert_eq!(
            lifetime, 4996,
            "and out of the windows it is still traffic the adapter carried"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn only_enabled_adapters_are_bonded_routes() {
        let path = temp_db("bond-members");
        let conn = open(&path).unwrap();
        let wired =
            note_network(&conn, "AA:BB:CC:00:00:0A", "Ethernet", "ethernet", true).unwrap();
        let tunnel = note_network(&conn, "ifindex:33", "Tailscale", "tunnel", false).unwrap();

        conn.execute(
            "UPDATE networks SET custom_name = 'Office cable' WHERE id = ?1",
            [wired.id],
        )
        .unwrap();

        let members = bond_members(&conn).unwrap();
        assert_eq!(members.len(), 1, "the tunnel the user never enabled is not a route");
        assert_eq!(members[0].id, wired.id);
        assert_eq!(members[0].label, "Office cable", "a rename is what the map should say");
        assert_eq!(members[0].color_id.as_deref(), Some("n1"));

        set_network_enabled(&conn, wired.id, false).unwrap();
        assert!(
            bond_members(&conn).unwrap().is_empty(),
            "nothing bonded means the job keeps the default route"
        );

        set_network_enabled(&conn, tunnel.id, true).unwrap();
        let members = bond_members(&conn).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].label, "Tailscale", "no rename means Windows' own name");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The stripe an adapter was given at first sight is the one it keeps, no
    /// matter what order Windows reports the adapters in next. The Sucker Map
    /// joins an old attempt row to a card by this colour, so a refresh that
    /// followed the enumeration order would repaint finished downloads with
    /// whichever cable happened to be up.
    #[test]
    fn a_stripe_survives_the_adapters_arriving_in_the_other_order() {
        let path = temp_db("stable-colour");
        let conn = open(&path).unwrap();

        let first = note_network(&conn, "AA:BB:CC:00:00:0B", "Ethernet", "ethernet", true).unwrap();
        let second = note_network(&conn, "AA:BB:CC:00:00:0C", "Wi-Fi", "wifi", true).unwrap();
        assert_eq!(first.color_id, "n1");
        assert_eq!(second.color_id, "n2", "a new adapter takes the next free stripe");

        // Same two devices, reported the other way round, and the Wi-Fi renamed.
        let again = note_network(&conn, "AA:BB:CC:00:00:0C", "Wi-Fi 5G", "wifi", true).unwrap();
        let wired = note_network(&conn, "AA:BB:CC:00:00:0B", "Ethernet", "ethernet", true).unwrap();
        assert_eq!(again.color_id, "n2", "the refresh keeps the stripe it was given");
        assert_eq!(wired.color_id, "n1", "and neither borrows the other's");
        assert_eq!(again.custom_name.as_deref(), None);
        let label: String = conn
            .query_row(
                "SELECT display_name FROM networks WHERE id = ?1",
                [again.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(label, "Wi-Fi 5G", "the live description still refreshes");

        // A third device that has never carried bytes cannot displace the two
        // that have, so it takes the next stripe rather than a borrowed one.
        let third = note_network(&conn, "ifindex:99", "Tailscale", "tunnel", false).unwrap();
        assert_eq!(third.color_id, "n3");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Two adapters wearing one stripe is damage the old build already wrote: the
    /// map joins an attempt row to a colour, and a colour with two owners cannot
    /// say which cable carried the bytes. Reopening the file separates them, and
    /// the lowest row keeps the stripe it already had so its history does not move.
    #[test]
    fn reopening_separates_adapters_wearing_the_same_stripe() {
        let path = temp_db("restripe");
        let conn = open(&path).unwrap();
        let wired = note_network(&conn, "AA:BB:CC:00:00:0D", "Ethernet", "ethernet", true).unwrap();
        let second = note_network(&conn, "AA:BB:CC:00:00:0E", "Ethernet 2", "ethernet", true).unwrap();
        let tunnel = note_network(&conn, "ifindex:7", "Tailscale", "tunnel", false).unwrap();
        assert_eq!(
            (wired.color_id.as_str(), second.color_id.as_str(), tunnel.color_id.as_str()),
            ("n1", "n2", "n3"),
            "a fresh database never had the problem to begin with"
        );

        // The state a pre-fix database is really in: every slot got the stripe
        // the enumeration handed it, so all three rows ended up on `n1`.
        for id in [second.id, tunnel.id] {
            conn.execute("UPDATE networks SET color_id = 'n1' WHERE id = ?1", [id])
                .unwrap();
        }
        drop(conn);

        let conn = open(&path).unwrap();
        assert_eq!(
            stripes(&conn),
            vec!["n1".to_string(), "n2".to_string(), "n3".to_string()],
            "the boot repair separates the rows it found colliding"
        );
        drop(conn);

        // And it is a no-op once the file is clean, so it cannot drift the
        // stripes of a database that has nothing to repair.
        let conn = open(&path).unwrap();
        assert_eq!(
            stripes(&conn),
            vec!["n1".to_string(), "n2".to_string(), "n3".to_string()],
            "a second boot changes nothing"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Six stripes is the whole palette, and a seventh row is not given a name
    /// the renderer cannot paint. It keeps a colour it shares with a row that
    /// carries no history, which is the one collision that reads correctly.
    #[test]
    fn the_palette_stops_at_six_and_says_so() {
        let path = temp_db("restripe-ceiling");
        let conn = open(&path).unwrap();
        for i in 0..7 {
            conn.execute(
                "INSERT INTO networks (mac_hint, display_name, kind, color_id, enabled) \
                 VALUES (?1, ?2, 'ethernet', 'n1', 1)",
                rusqlite::params![format!("AA:BB:CC:00:00:2{i}"), format!("Nic {i}")],
            )
            .unwrap();
        }
        let got = stripes(&conn);
        assert_eq!(
            got,
            vec!["n1"; 7]
                .iter().map(|s| s.to_string()).collect::<Vec<String>>(),
            "the positional writer left every row on the same stripe"
        );
        restripe(&conn).unwrap();
        let got = stripes(&conn);
        assert_eq!(
            got,
            vec!["n1", "n2", "n3", "n4", "n5", "n6", "n1"]
                .iter().map(|s| s.to_string()).collect::<Vec<String>>(),
            "seven rows take six stripes, and the last one repeats rather than inventing n7"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn stripes(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT color_id FROM networks ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect()
    }

    /// A pasted session is one row per host, and the second paste replaces the
    /// first rather than stacking up under it.
    #[test]
    fn a_pasted_session_replaces_the_one_it_is_pasted_over() {
        let path = temp_db("sessions");
        let conn = open(&path).unwrap();

        put_session(&conn, "example.com", "cf_clearance=first", Some("UA/1"), 100).unwrap();
        put_session(&conn, "example.com", "cf_clearance=second", None, 200).unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM credentials WHERE scheme = 'cookie'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "one host is one row");
        let session = session_for(&conn, "example.com").unwrap().unwrap();
        assert_eq!(session.cookie, "cf_clearance=second");
        assert_eq!(
            session.user_agent, None,
            "a paste without a user agent drops the old one, not keeps it"
        );

        assert!(forget_session(&conn, "example.com").unwrap());
        assert!(!forget_session(&conn, "example.com").unwrap(), "nothing left to drop");
        assert!(session_for(&conn, "example.com").unwrap().is_none());
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The validator is the only thing between a paste from devtools and a header
    /// on the wire, so it refuses what would smuggle a second header or truncate
    /// the value, and accepts what a paste actually looks like.
    #[test]
    fn the_cookie_is_checked_before_it_reaches_the_wire() {
        assert_eq!(
            validate_cookie("Cookie: cf_clearance=abc").unwrap().as_deref(),
            Some("cf_clearance=abc"),
            "devtools copies the whole line"
        );
        assert_eq!(
            validate_cookie("  a=1; b=2  ").unwrap().as_deref(),
            Some("a=1; b=2"),
            "several cookies are one header"
        );
        assert_eq!(validate_cookie("   ").unwrap(), None, "empty is no session");
        assert!(validate_cookie("cf_clearance").is_err(), "a value needs its name");
        assert!(
            validate_cookie("a=1\r\nX-Injected: yes").is_err(),
            "a newline is a second header"
        );
        assert!(validate_cookie("a=1\nb=2").is_err(), "and an interior one too");
        assert_eq!(
            validate_cookie("a=1\r\n").unwrap().as_deref(),
            Some("a=1"),
            "what a paste only wraps at the end is trimmed, not refused"
        );
        assert!(validate_cookie("a=1\tb").is_err(), "a tab splits the value");
        assert!(
            validate_cookie(&format!("a={}", "x".repeat(MAX_COOKIE_BYTES))).is_err(),
            "a whole profile pasted by mistake is not one session"
        );
        assert!(validate_cookie("Cookie:").is_err(), "the prefix alone is not a cookie");

        assert_eq!(validate_user_agent(Some("  Mozilla/5.0  ")).unwrap().as_deref(), Some("Mozilla/5.0"));
        assert_eq!(validate_user_agent(None).unwrap(), None);
        assert_eq!(validate_user_agent(Some("")).unwrap(), None, "blank clears it");
        assert!(validate_user_agent(Some("Mozilla\r\nEvil: 1")).is_err());
        assert!(validate_user_agent(Some(&"u".repeat(MAX_USER_AGENT_BYTES + 1))).is_err());
    }

    /// The settings list has to say which hosts have a session and how big it is
    /// without ever carrying the secret across the bridge.
    #[test]
    fn the_list_shows_the_host_and_not_the_cookie() {
        let path = temp_db("session-list");
        let conn = open(&path).unwrap();
        put_session(&conn, "b.example", "cf_clearness=1234567890", Some("UA/2"), 300).unwrap();
        put_session(&conn, "a.example", "s=1", None, 200).unwrap();

        let listed = list_sessions(&conn).unwrap();
        assert_eq!(
            listed,
            vec![
                SessionSummary {
                    host: "a.example".into(),
                    user_agent: None,
                    cookie_bytes: 3,
                    created_at: 200,
                },
                SessionSummary {
                    host: "b.example".into(),
                    user_agent: Some("UA/2".into()),
                    cookie_bytes: 23,
                    created_at: 300,
                },
            ],
            "oldest first, and no cookie text anywhere"
        );
        assert!(
            !format!("{listed:?}").contains("1234567890"),
            "the debug form would be the leak"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// One proxy, one credential: the row is keyed to the proxy it was typed for, a
    /// save under a different key replaces it rather than stacking beside it, and the
    /// only read that sees the secret is the one the engine mirror asks for.
    #[test]
    fn a_proxy_credential_is_one_row_keyed_to_its_proxy() {
        let path = temp_db("proxy-auth");
        let conn = open(&path).unwrap();

        assert_eq!(proxy_auth(&conn).unwrap(), None, "a fresh library has none");

        put_proxy_auth(&conn, "127.0.0.1:8127", "agent", "s3cret", 100).unwrap();
        assert_eq!(
            proxy_auth(&conn).unwrap(),
            Some(ProxyAuth {
                host: "127.0.0.1:8127".into(),
                username: "agent".into(),
                secret: "s3cret".into(),
            }),
            "'proxy' is a scheme the table's own CHECK already allows"
        );

        // A new URL is a new key, and the row for the URL before it must not stay:
        // it would read on the screen as a credential that is in use.
        put_proxy_auth(&conn, "127.0.0.1:8128", "agent2", "other", 200).unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM credentials WHERE scheme = 'proxy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "the whole app has one proxy credential");
        assert_eq!(proxy_auth(&conn).unwrap().unwrap().host, "127.0.0.1:8128");

        // A username with no secret is a real Basic pair — a proxy that takes a
        // token as the name expects exactly this shape.
        put_proxy_auth(&conn, "p:1", "tok3n", "", 300).unwrap();
        let auth = proxy_auth(&conn).unwrap().unwrap();
        assert_eq!((auth.username.as_str(), auth.secret.as_str()), ("tok3n", ""));

        assert!(forget_proxy_auth(&conn).unwrap());
        assert!(!forget_proxy_auth(&conn).unwrap(), "nothing left to drop");
        assert_eq!(proxy_auth(&conn).unwrap(), None);
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A `proxy` row with no username cannot come from this app — the write path
    /// refuses a lone password, because ureq would drop it — so it is a hand edit,
    /// and reading it as a credential would put a nameless secret on the wire.
    #[test]
    fn a_nameless_proxy_row_is_not_a_credential() {
        let path = temp_db("proxy-auth-nameless");
        let conn = open(&path).unwrap();
        conn.execute(
            "INSERT INTO credentials (scheme, host, username, secret_blob, created_at) \
             VALUES ('proxy', 'p:1', NULL, ?1, 100)",
            [&b"orphan"[..]],
        )
        .unwrap();
        assert_eq!(proxy_auth(&conn).unwrap(), None);
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The window judged minute by minute, with no database and no clock: the
    /// 22:00-06:00 case is the reason an overnight window exists at all.
    #[test]
    fn an_overnight_window_covers_both_sides_of_midnight() {
        let window = DownloadWindow {
            enabled: true,
            start_minute: 22 * 60,
            stop_minute: 6 * 60,
            days: 0,
        };
        assert!(window.open_at(0, 22 * 60), "the first minute it opens");
        assert!(window.open_at(0, 23 * 60 + 59));
        assert!(window.open_at(1, 0), "midnight belongs to the evening that opened it");
        assert!(window.open_at(1, 6 * 60 - 1));
        assert!(!window.open_at(1, 6 * 60), "the stop minute is already shut");
        assert!(!window.open_at(1, 12 * 60), "the afternoon is outside it");
        assert!(!window.open_at(1, 21 * 60 + 59), "one minute before it opens");
    }

    #[test]
    fn a_day_mask_is_monday_first_and_zero_is_every_day() {
        // 01:29-01:31 every day: the mask is the only thing being judged, and the
        // 01:30 minute the assertions ask about is always inside the hours.
        let window = |days| DownloadWindow {
            enabled: true,
            start_minute: 89,
            stop_minute: 91,
            days,
        };
        assert!(window(0).open_at(6, 90), "no mask filters no day");
        assert!(window(1).open_at(0, 90), "bit 0 is Monday");
        assert!(!window(1).open_at(1, 90), "a Monday-only mask ignores Tuesday");
        assert!(window(1 << 6).open_at(6, 90), "bit 6 is Sunday");
        assert!(
            !window(1).open_at(0, 5 * 60),
            "a day in the window is not a licence outside its hours"
        );
    }

    #[test]
    fn an_overnight_tail_is_judged_on_the_day_it_started() {
        // Wednesday 22:00 to Thursday 06:00, masked to Wednesday alone (bit 2).
        let window = DownloadWindow {
            enabled: true,
            start_minute: 22 * 60,
            stop_minute: 6 * 60,
            days: 1 << 2,
        };
        assert!(window.open_at(2, 23 * 60), "Wednesday night is inside the mask");
        assert!(window.open_at(3, 5 * 60), "Thursday 05:00 is still Wednesday's window");
        assert!(!window.open_at(3, 7 * 60), "and Thursday morning was never in it");
        assert!(!window.open_at(4, 23 * 60), "Thursday night opens on Thursday, so it is masked out");
    }

    #[test]
    fn a_disabled_switch_and_equal_times_both_block_nothing() {
        let off = DownloadWindow {
            enabled: false,
            start_minute: 22 * 60,
            stop_minute: 6 * 60,
            days: 1,
        };
        for minute in [0u32, 12 * 60, 23 * 60 + 59] {
            assert!(off.open_at(3, minute), "off means open at {minute}");
        }
        let all_day = DownloadWindow {
            enabled: true,
            start_minute: 0,
            stop_minute: 0,
            days: 0,
        };
        assert!(all_day.open_at(0, 0));
        assert!(
            all_day.open_at(0, 23 * 60 + 59),
            "an enabled switch and two empty time boxes block nothing"
        );
    }

    #[test]
    fn hhmm_parses_and_refuses() {
        assert_eq!(parse_minute("22:00"), Some(22 * 60));
        assert_eq!(parse_minute("0:07"), Some(7), "a time input gives no leading zero hour");
        assert_eq!(parse_minute(" 06:30 "), Some(6 * 60 + 30));
        assert_eq!(parse_minute("24:00"), None);
        assert_eq!(parse_minute("12:60"), None);
        assert_eq!(parse_minute("1260"), None);
        assert_eq!(parse_minute(""), None);
    }

    /// The four keys, the clock and the gate over one real database, so a key the
    /// Settings page writes is proved to reach the answer the pump acts on.
    #[test]
    fn the_stored_window_decides_now() {
        let path = temp_db("window-read");
        let conn = open(&path).unwrap();
        assert!(window_open_now(&conn), "no rows at all is no window");

        let (today, minute) = local_clock(&conn).unwrap();
        assert!(minute < 24 * 60, "the clock answers in minutes of a day");
        assert!(today < 7, "and Monday-first from zero");

        set_setting(&conn, "engine.window_enabled", Some("true")).unwrap();
        assert!(
            window_open_now(&conn),
            "an enabled switch with no times written yet blocks nothing"
        );

        // Every day except this one, all day long. The shut answer has to come from
        // the mask, so nothing about the hour this test happens to run at can move it.
        let not_today = 0x7f & !(1 << today);
        set_setting(&conn, "engine.window_days", Some(&not_today.to_string())).unwrap();
        assert!(!window_open_now(&conn), "a mask without today holds the queue");

        set_setting(&conn, "engine.window_days", Some(&(1u32 << today).to_string())).unwrap();
        assert!(window_open_now(&conn), "today's own bit opens it");

        set_setting(&conn, "engine.window_enabled", Some("false")).unwrap();
        assert!(window_open_now(&conn), "off beats a mask that excludes today");

        set_setting(&conn, "engine.window_enabled", Some("true")).unwrap();
        set_setting(&conn, "engine.window_days", Some("0")).unwrap();
        set_setting(&conn, "engine.window_start", Some("22:00")).unwrap();
        set_setting(&conn, "engine.window_stop", Some("06:00")).unwrap();
        let window = DownloadWindow::read(&conn).unwrap();
        assert_eq!((window.start_minute, window.stop_minute), (22 * 60, 6 * 60));
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The session list is what the Settings page gets wholesale. A proxy
    /// credential must not join it, and must not be reachable through it either.
    #[test]
    fn a_proxy_credential_stays_out_of_the_session_list() {
        let path = temp_db("proxy-auth-list");
        let conn = open(&path).unwrap();
        put_proxy_auth(&conn, "127.0.0.1:8127", "agent", "s3cret", 100).unwrap();
        put_session(&conn, "example.com", "cf_clearance=abc", None, 110).unwrap();

        let listed = list_sessions(&conn).unwrap();
        assert_eq!(
            listed.iter().map(|s| s.host.as_str()).collect::<Vec<_>>(),
            vec!["example.com"],
            "the cookie list is cookies, not the proxy pair"
        );
        assert!(
            !format!("{listed:?}").contains("s3cret"),
            "no read the page gets may carry the secret"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
