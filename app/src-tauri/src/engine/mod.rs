// The download engine's protocol and file layer: probe, block plan, naming,
// publish, provenance and checksums. Everything here is independent of the UI
// and of SQLite so it can be exercised directly by tests.

pub mod auth;
pub mod bond;
pub mod job;
pub mod limit;
pub mod mux;
pub mod network;
pub mod webm;
pub mod proxy;

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use ureq::config::RedirectAuthHeaders;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, ResponseExt};

pub const DEFAULT_BLOCK: u64 = 8 * 1024 * 1024;
pub const DEFAULT_STREAMS: usize = 8;
pub const MAX_STREAMS: usize = 32;
/// How far a plan may be narrowed to reach the requested stream count. Below
/// 1 MiB a slice spends more of its life in `Range` setup than in bytes, so a
/// smaller worker count is the honest answer rather than a burst of tiny
/// requests.
pub const MIN_BLOCK: u64 = 1024 * 1024;
/// A server may ask for any wait it likes; longer than this and the job would
/// sit on one stalled block for minutes, so the request is recorded in the
/// reason text and the sleep is capped.
pub const MAX_RETRY_WAIT: i64 = 120;
/// How much of a block is buffered before writing it down.
pub const CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct JobError {
    pub code: &'static str,
    pub message: String,
    /// HTTP status, when the failure came from a server that answered.
    pub status: Option<u16>,
    /// The server's own `Retry-After`, on the 429/503 that carried one. A whole
    /// job can run as a single stream, and one stream has no workers to shed, so
    /// this is the only way a rate limit can be obeyed there.
    pub wait_secs: Option<u64>,
}

impl JobError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            status: None,
            wait_secs: None,
        }
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_wait(mut self, wait_secs: Option<u64>) -> Self {
        self.wait_secs = wait_secs;
        self
    }
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<io::Error> for JobError {
    fn from(e: io::Error) -> Self {
        JobError::new("io", e.to_string())
    }
}

/// The proxy refused to open the tunnel, which is not an answer from the site.
///
/// A 407 here asks for the credential in the Settings screen; a 407 from the origin
/// asks for something on the download. Booking both as `http` would put the same
/// word beside both, and the one thing a user can do about the first is invisible.
///
/// The status is read out of ureq's own payload, because `ConnectProxyFailed` carries
/// a `String` and not a status (`error.rs:165`): the writer formats
/// `proxy server responded {}/{} ` (`connect.rs:152`). Naming that shape is a
/// dependency on a format string, and it is stated here rather than hidden — a ureq
/// that changes it yields `proxy-failed` with no status, which is the honest
/// direction. The other payload, "proxy server did not respond", genuinely cannot be
/// told apart from a proxy that dropped the tunnel over an auth it dislikes, so it
/// reads as `proxy-failed` and nothing more.
fn proxy_connect_error(reason: &str) -> JobError {
    let status = reason
        .strip_prefix("proxy server responded ")
        .and_then(|rest| rest.split('/').next())
        .and_then(|digits| digits.parse::<u16>().ok());
    let code = if status == Some(407) {
        "proxy-auth"
    } else {
        "proxy-failed"
    };
    let error = JobError::new(code, format!("the proxy refused the tunnel: {reason}"));
    match status {
        Some(status) => error.with_status(status),
        None => error,
    }
}

impl From<ureq::Error> for JobError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::ConnectProxyFailed(ref reason) => proxy_connect_error(reason),
            other => JobError::new("http", other.to_string()),
        }
    }
}

/// Connection handshake timeout (`network.timeout_seconds`), read by every
/// new agent. 30 s matches the Settings default; the UI can move it 15/30/60.
pub static CONNECT_TIMEOUT_SECS: AtomicU64 = AtomicU64::new(30);

pub fn set_connect_timeout(seconds: u64) {
    CONNECT_TIMEOUT_SECS.store(seconds.clamp(5, 300), Ordering::Relaxed);
}

/// Sites that police bot traffic reject the crate default `ureq/3.5.0` on
/// sight, so requests go out with the same identity the embedded WebView
/// reports. This is a compatibility identity, not an attempt to impersonate a
/// user session.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";

/// The one HTTP configuration every agent in the app is built from, so a
/// bonded agent cannot drift from the default one on timeouts, proxy route or
/// TLS trust.
pub fn config() -> ureq::config::Config {
    config_session(None)
}

/// `config()` under a browser session's identity, for the agents of a job whose
/// host has one stored.
pub fn config_session(user_agent: Option<&str>) -> ureq::config::Config {
    config_ua(
        Duration::from_secs(CONNECT_TIMEOUT_SECS.load(Ordering::Relaxed)),
        user_agent,
    )
}

/// `config()` with the connect budget as a parameter. The link check in the add
/// bar needs the same TLS trust, user agent and redirect policy as a real
/// transfer — and a much shorter wait, because it runs while the user is typing.
pub fn config_connect(connect: Duration) -> ureq::config::Config {
    config_ua(connect, None)
}

/// The same configuration with the identity swapped for the one a cookie was
/// copied with.
///
/// Cloudflare binds `cf_clearance` to the User-Agent that earned it, so a pasted
/// session only works on requests that carry the browser's own UA. A blank or
/// absent override keeps the compatibility identity above.
pub fn config_ua(connect: Duration, user_agent: Option<&str>) -> ureq::config::Config {
    let ua = user_agent
        .map(str::trim)
        .filter(|ua| !ua.is_empty())
        .unwrap_or(USER_AGENT);
    Agent::config_builder()
        .user_agent(ua)
        // The origin credential, put on the request before it leaves. Attached here
        // rather than per call site so it lands on every request the app makes — the
        // pre-flight probe, both transfer paths, the refresh, the link check — which
        // is the same reason the User-Agent is not repeated at those sites.
        .middleware(auth::middleware())
        // A redirect is inside ureq's `run`, below the middleware, so the header this
        // one writes would be dropped on the next hop by the default `Never`. SameHost
        // is the rule that matches this store: the pair is keyed to the host alone, and
        // ureq's own test for the redirect is host equality plus no downgrade
        // (`redirect.rs:121-126`) — so a password kept for `files.example` stays on
        // `files.example`, and never follows a redirect to a different host.
        .redirect_auth_headers(RedirectAuthHeaders::SameHost)
        // Written every time, including as `None`: the builder starts from
        // `Config::default()`, which seeds itself from `HTTP_PROXY`. Leaving
        // this option untouched would let the environment move bytes while the
        // UI said "direct", so the app states its answer instead.
        .proxy(proxy::resolve())
        // ureq defaults to Rustls, which is not compiled in (native-tls only),
        // so HTTPS panics unless the provider is selected explicitly.
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                // Trust the Windows store, like any other desktop downloader;
                // the bundled WebPKI list rejects some valid chains.
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_connect(Some(connect))
        .timeout_per_read(Some(Duration::from_secs(4)))
        .max_redirects(10)
        .http_status_as_error(false)
        .build()
}

/// One agent per download. Redirects are followed so `final_url` is what the
/// bytes actually came from; read timeouts restart per socket read, which is
/// how a stalled connection is told apart from a slow but live one.
pub fn agent() -> Agent {
    config().new_agent()
}

/// `agent()` under a browser session's identity, for a lone request that has to
/// look like the browser the cookie was copied from.
pub fn agent_session(user_agent: Option<&str>) -> Agent {
    config_session(user_agent).new_agent()
}

#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub final_url: String,
    /// Referer to present for this transfer: the link the user pasted, or the
    /// URL the bytes actually landed on once the probe followed redirects.
    pub referer: String,
    /// The host's pasted session, carried alongside `referer` so every later
    /// request for this transfer — resume probe, range workers, the single-stream
    /// retry — presents the same identity the pre-flight did. Not a server fact.
    pub cookie: Option<String>,
    pub status: u16,
    pub total_bytes: Option<u64>,
    pub accept_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    pub filename: Option<String>,
}

/// Ask for a single byte: a 206 proves range support and carries the exact
/// total in Content-Range, a 200 means the server will only send whole bodies.
///
/// `cookie` is the pasted session for this host, if there is one. It rides on the
/// probe because that is where an edge-protection site says no.
pub fn probe(
    agent: &Agent,
    url: &str,
    referer: Option<&str>,
    cookie: Option<&str>,
) -> Result<Probe, JobError> {
    let mut req = agent
        .get(url)
        .header("Accept-Encoding", "identity")
        .header("Range", "bytes=0-0");
    if let Some(r) = referer.map(str::trim).filter(|r| !r.is_empty()) {
        req = req.header("Referer", r);
    }
    if let Some(c) = cookie.map(str::trim).filter(|c| !c.is_empty()) {
        req = req.header("Cookie", c);
    }
    let resp = req.call()?;

    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let final_url = resp.get_uri().to_string();
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };

    let mut out = Probe {
        final_url,
        referer: String::new(),
        cookie: None,
        status,
        accept_ranges: false,
        etag: header("etag"),
        last_modified: header("last-modified"),
        content_type: header("content-type"),
        filename: header("content-disposition").as_deref().and_then(disposition_filename),
        total_bytes: None,
    };

    match status {
        206 => {
            out.accept_ranges = true;
            out.total_bytes = header("content-range").as_deref().and_then(content_range_total);
        }
        200 => {
            out.total_bytes = header("content-length").and_then(|v| v.parse().ok());
        }
        416 => {
            // An empty resource answers every range with 416 and "bytes */0".
            out.accept_ranges = true;
            out.total_bytes = header("content-range")
                .as_deref()
                .and_then(content_range_total)
                .or(Some(0));
        }
        300..=399 => {
            return Err(
                JobError::new("too-many-redirects", format!("status {status}")).with_status(status)
            );
        }
        401 => {
            // The one refusal the user can answer by typing something, so it is its own
            // code rather than a number inside `http-status`. What the server offered is
            // named because an offer of Digest alone must not read as "type your password
            // and it will work": the two kinds this build can send are named as answerable,
            // and anything else is said as an offer it will not attempt. The noun is
            // `credential`, not `password`, because the host here may be answered by a pair
            // or by a token and only the store knows which.
            let offer = header("www-authenticate").as_deref().and_then(auth_schemes);
            let message = match offer {
                Some(s) if s.contains("basic") || s.contains("bearer") => {
                    format!("server needs a credential (offers {s})")
                }
                Some(s) => format!(
                    "server needs a credential, and offers {s}; this app sends Basic or a bearer token"
                ),
                None => "server needs a credential".to_string(),
            };
            return Err(JobError::new("needs-auth", message).with_status(status));
        }
        400..=599 => {
            // A 403/429 from an edge-protection vendor is not an ordinary HTTP
            // error: no retry, header or range will move it, and saying so is
            // the only useful thing the inspector can report.
            let edge = header("server")
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            let blocked = matches!(status, 403 | 429)
                && ["cloudflare", "akamai", "sucuri", "datadome"]
                    .iter()
                    .any(|vendor| edge.contains(vendor));
            let message = if blocked {
                format!("edge protection answered {status}; the link needs a browser session")
            } else {
                format!("server answered {status}")
            };
            return Err(JobError::new(if blocked { "blocked" } else { "http-status" }, message)
                .with_status(status));
        }
        _ => {}
    }
    Ok(out)
}

/// `bytes 0-0/12345` -> 12345; `bytes */0` -> 0; `bytes */"*"` -> None.
pub fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

/// The scheme names a `www-authenticate` offer mentions, from a fixed list, or `None`
/// when it mentions none.
///
/// Scanning for known names rather than splitting on commas is deliberate:
/// `Basic realm="Files, internal"` carries a comma inside quotes, and a comma-splitting
/// parse would read `internal"` as a scheme. Unknown schemes stay unnamed, which is the
/// honest direction — an offer this app cannot read is reported as "needs a credential",
/// never as a scheme it might not be.
pub fn auth_schemes(value: &str) -> Option<String> {
    const NAMED: [&str; 6] = ["basic", "digest", "ntlm", "negotiate", "bearer", "spnego"];
    let lower = value.to_ascii_lowercase();
    let found: Vec<&str> = NAMED
        .iter()
        .copied()
        .filter(|name| lower.contains(name))
        .collect();
    (!found.is_empty()).then(|| found.join(", "))
}

/// `filename*=UTF-8''a%20b.zip` wins over `filename="a b.zip"`, which wins over
/// a bare token.
pub fn disposition_filename(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if let Some(pos) = lower.find("filename*=") {
        let rest = &value[pos + "filename*=".len()..];
        let rest = rest.trim_start();
        let rest = rest.split(';').next().unwrap_or(rest).trim();
        let (charset_and_lang, encoded) = rest.split_once("''")?;
        let _ = charset_and_lang;
        let decoded = percent_decode(encoded);
        if !decoded.is_empty() {
            return Some(decoded);
        }
    }
    if let Some(pos) = lower.find("filename=") {
        let rest = &value[pos + "filename=".len()..];
        let rest = rest.trim_start();
        let rest = rest.split(';').next().unwrap_or(rest).trim();
        let unquoted = rest
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(rest);
        let decoded = percent_decode(unquoted);
        if !decoded.is_empty() {
            return Some(decoded);
        }
    }
    None
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Best-effort name from the URL path, used until the probe supplies one.
pub fn filename_from_url(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority_and_path = after_scheme.split(['?', '#']).next().unwrap_or("");
    let path = authority_and_path.split_once('/').map(|(_, p)| p).unwrap_or("");
    let last = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    let name = sanitize_filename(&percent_decode(last));
    if name.is_empty() {
        "download".to_string()
    } else {
        name
    }
}

/// The key a browser session is stored under: the host, lowercased, with scheme,
/// userinfo, path, query, fragment and port removed.
///
/// Cookies are not scoped to a port, so `example.com:8443` and `example.com`
/// share one session; an IPv6 literal is kept without its brackets because the
/// browser sends the cookie for the address, not for the notation.
pub fn host_key(url_or_host: &str) -> String {
    let rest = url_or_host
        .trim()
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(url_or_host.trim());
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let without_userinfo = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let host = match without_userinfo.split_once(']') {
        Some((addr, _)) if addr.starts_with('[') => &addr[1..],
        _ => match without_userinfo.rsplit_once(':') {
            Some((h, port)) if !h.is_empty() && !port.is_empty() && port.len() < 6 => h,
            _ => without_userinfo,
        },
    };
    host.to_ascii_lowercase()
}

const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Windows-safe: no control or reserved characters, no trailing dots or spaces,
/// no device names, and bounded length with the extension kept.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return String::new();
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed).to_ascii_lowercase();
    let mut safe = if RESERVED.contains(&stem.as_str()) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    };
    if safe.chars().count() > 180 {
        let ext = safe
            .rsplit_once('.')
            .map(|(_, e)| format!(".{}", e))
            .unwrap_or_default();
        let keep = 180usize.saturating_sub(ext.chars().count());
        safe = format!("{}{}", safe.chars().take(keep).collect::<String>(), ext);
    }
    safe
}

/// Inclusive byte ranges covering `total` in blocks of `block_size`.
pub fn plan_blocks(total: u64, block_size: u64) -> Vec<(u64, u64)> {
    if block_size == 0 || total == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + block_size - 1).min(total - 1);
        out.push((start, end));
        start = end + 1;
    }
    out
}

pub fn block_count(total: u64, block_size: u64) -> u64 {
    if block_size == 0 {
        0
    } else {
        total.div_ceil(block_size)
    }
}

/// `DEFAULT_BLOCK` slices only reach the requested stream count on a file of at
/// least `block × streams`; below that, `streams = 32` quietly becomes 13
/// workers because the plan never had more slices. Shrink the slice until the
/// plan really has `streams` of them, and stop at `MIN_BLOCK` when the file
/// cannot pay for them.
pub fn adaptive_block(total: u64, streams: usize, default_block: u64) -> u64 {
    if total == 0 || default_block == 0 {
        return default_block;
    }
    let streams = streams.clamp(1, MAX_STREAMS) as u64;
    if total >= streams.saturating_mul(default_block) {
        return default_block;
    }
    (total / streams).clamp(MIN_BLOCK, default_block)
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// `Retry-After` is either delta-seconds or an HTTP-date, and a 429 without it
/// is still a request to slow down. Both forms are parsed here rather than by
/// adding a date crate: the size gate is a defence against an accidental
/// dependency, not a ration on bytes (5 MB ceiling, ruled 2026-11-04), and one
/// header is still a poor reason to put a calendar in the binary.
pub fn retry_after_secs(value: &str, now_unix: i64) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<i64>() {
        return Some(secs.clamp(0, MAX_RETRY_WAIT));
    }
    // "Wed, 21 Oct 2015 07:28:00 GMT", with or without the weekday.
    let parts: Vec<&str> = value.split_whitespace().collect();
    let at = if parts.first().is_some_and(|w| w.ends_with(',')) {
        1
    } else {
        0
    };
    if parts.len() < at + 4 {
        return None;
    }
    let day: u32 = parts[at].parse().ok()?;
    let month_token = parts[at + 1].get(..3).map(|m| m.to_ascii_lowercase())?;
    let month = (MONTHS.iter().position(|m| *m == month_token)? + 1) as u32;
    let year: i64 = parts[at + 2].parse().ok()?;
    let mut clock = parts[at + 3].split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    let when = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    Some((when - now_unix).clamp(0, MAX_RETRY_WAIT))
}

/// Days from 1970-01-01 to a civil date. Howard Hinnant's algorithm, so the
/// arithmetic is exact across month lengths and leap years in about ten lines.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = if shifted >= 0 { shifted } else { shifted - 399 } / 400;
    let year_of_era = shifted - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * month_prime + 2) / 5 + day as i64 - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// `name.ext` -> `name (1).ext` until the path is free.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} (9999){ext}"))
}

/// Windows hands out transient sharing violations for freshly written files
/// (indexer, antivirus); five 200 ms retries turn those into a rename.
pub fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = None;
    for attempt in 0..5 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                if attempt < 4 {
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
    Err(last.unwrap())
}

/// Mark the published file as internet-sourced, the zone browsers stamp, so
/// SmartScreen and Explorer behave predictably.
pub fn write_zone_identifier(path: &Path, referrer: &str, host: &str) -> io::Result<()> {
    let mut ads = path.as_os_str().to_os_string();
    ads.push(":Zone.Identifier");
    let content = format!(
        "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl={referrer}\r\nHostUrl={host}\r\n"
    );
    std::fs::write(PathBuf::from(ads), content)
}

pub fn is_algo(algo: &str) -> bool {
    matches!(algo, "md5" | "sha1" | "sha256" | "sha512")
}

/// Hex digest of a file. Read in 1 MiB pieces so a multi-gigabyte download
/// does not need to fit in memory.
pub fn file_digest(path: &Path, algo: &str) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; 1024 * 1024];
    macro_rules! digest {
        ($hasher:ty) => {{
            let mut hasher = <$hasher>::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        }};
    }
    match algo {
        "md5" => Ok(digest!(md5::Md5)),
        "sha1" => Ok(digest!(Sha1)),
        "sha256" => Ok(digest!(Sha256)),
        "sha512" => Ok(digest!(Sha512)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown checksum {algo}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parses_both_forms() {
        assert_eq!(content_range_total("bytes 0-0/1048576"), Some(1_048_576));
        assert_eq!(content_range_total("bytes */0"), Some(0));
        assert_eq!(content_range_total("nonsense"), None);
    }

    #[test]
    fn disposition_prefers_rfc5987() {
        assert_eq!(
            disposition_filename("attachment; filename=\"a b.zip\"; filename*=UTF-8''c%20d.zip"),
            Some("c d.zip".into())
        );
        assert_eq!(
            disposition_filename("attachment; filename=\"plain.bin\""),
            Some("plain.bin".into())
        );
        assert_eq!(disposition_filename("inline"), None);
    }

    /// The key a pasted session is stored under is the host alone: cookies are not
    /// scoped to a port or a path, and a site reached both ways is one site.
    #[test]
    fn a_session_key_is_the_host_alone() {
        assert_eq!(host_key("https://Example.COM/a/b?q=1#frag"), "example.com");
        assert_eq!(host_key("https://example.com:8443/x"), "example.com");
        assert_eq!(host_key("http://user:pass@cdn.example/"), "cdn.example");
        assert_eq!(host_key("example.com"), "example.com");
        assert_eq!(host_key("example.com:8099"), "example.com");
        assert_eq!(host_key("https://[2001:db8::1]:443/f"), "2001:db8::1");
        assert_eq!(host_key("   "), "");
        assert_eq!(host_key("https://127.0.0.1:9/x.bin"), "127.0.0.1");
    }

    #[test]
    fn url_names_are_sanitized() {
        assert_eq!(filename_from_url("https://x.test/a/b%20c.tar.gz?x=1"), "b c.tar.gz");
        assert_eq!(filename_from_url("https://x.test/"), "download");
        assert_eq!(filename_from_url("https://x.test/CON.txt"), "_CON.txt");
        assert_eq!(filename_from_url("https://x.test/a<b>.bin"), "a_b_.bin");
        assert!(sanitize_filename("..").is_empty());
    }

    /// What an origin 401 can tell the user, from the offer header alone. The quoted-
    /// comma case is the reason this is a scan and not a parse, and the unknown-scheme
    /// case is the reason an answer of `None` reads as "needs a credential" rather than as
    /// a scheme this build could try.
    #[test]
    fn an_auth_offer_is_read_without_splitting_on_quotes() {
        assert_eq!(auth_schemes("Basic realm=\"Files\""), Some("basic".into()));
        assert_eq!(
            auth_schemes("Basic realm=\"Files, internal\""),
            Some("basic".into()),
            "a comma inside a quoted realm is not a second challenge"
        );
        assert_eq!(auth_schemes("Digest realm=x, qop=\"auth\""), Some("digest".into()));
        assert_eq!(
            auth_schemes("Negotiate, NTLM"),
            Some("ntlm, negotiate".into()),
            "both are named, and neither is one this app can answer"
        );
        assert_eq!(auth_schemes("Custom-TOKEN algo=2"), None);
        assert_eq!(auth_schemes(""), None);
    }

    /// `&` and `^` are legal in a Windows filename, so the sanitizer keeps them.
    /// That is why a stored name must never reach a shell command line: to us
    /// `report.pdf&calc.exe` is one file, to cmd.exe it is two commands.
    #[test]
    fn shell_metacharacters_survive_sanitizing() {
        assert_eq!(sanitize_filename("report.pdf&calc.exe"), "report.pdf&calc.exe");
        assert_eq!(sanitize_filename("up^grade.bin"), "up^grade.bin");
    }

    /// The two shapes ureq's CONNECT writer can produce (`connect.rs:139` and
    /// `:152`), each read the way the card needs it: a 407 from the proxy is its own
    /// code, and anything else is a refusal by a proxy that answered.
    #[test]
    fn a_proxy_that_refuses_the_tunnel_says_which_refusal_it_is() {
        let needs_credential =
            JobError::from(ureq::Error::ConnectProxyFailed(
                "proxy server responded 407/Proxy Authentication Required".to_string(),
            ));
        assert_eq!(needs_credential.code, "proxy-auth");
        assert_eq!(needs_credential.status, Some(407));
        assert!(needs_credential.message.contains("407"));
        // The message is what lands in `error_hint` and in the history row, so it
        // must not be able to carry a password even by accident.
        assert!(!needs_credential.message.contains('@'));

        let refused = JobError::from(ureq::Error::ConnectProxyFailed(
            "proxy server responded 403/Forbidden".to_string(),
        ));
        assert_eq!(refused.code, "proxy-failed");
        assert_eq!(refused.status, Some(403));
    }

    /// "proxy server did not respond" carries no status, and a payload this parse
    /// does not recognise carries none either: both read as a proxy that failed, and
    /// neither is allowed to invent a number it did not read.
    #[test]
    fn a_proxy_failure_without_a_status_is_not_given_one() {
        let silent = JobError::from(ureq::Error::ConnectProxyFailed(
            "proxy server did not respond".to_string(),
        ));
        assert_eq!(silent.code, "proxy-failed");
        assert_eq!(silent.status, None);

        let reworded = JobError::from(ureq::Error::ConnectProxyFailed(
            "proxy said no".to_string(),
        ));
        assert_eq!(reworded.code, "proxy-failed");
        assert_eq!(reworded.status, None);

        // Digits that are not a status, and a status too wide for the field, both
        // fall back rather than reporting a wrong number.
        for reason in [
            "proxy server responded not-a-number/Nope",
            "proxy server responded 70000/Over",
            "Proxy server responded 407/x",
        ] {
            let e = JobError::from(ureq::Error::ConnectProxyFailed(reason.to_string()));
            assert_eq!(e.code, "proxy-failed", "{reason}");
            assert_eq!(e.status, None, "{reason}");
        }
    }

    /// Every other ureq failure still books as `http`, exactly as before: the typed
    /// proxy codes are an added arm, not a re-mapping of the error surface.
    #[test]
    fn a_non_proxy_failure_is_still_http() {
        let e = JobError::from(ureq::Error::InvalidProxyUrl);
        assert_eq!(e.code, "http");
        assert_eq!(e.status, None);
    }

    #[test]
    fn blocks_cover_every_byte_once() {
        let total = 20 * 1024 * 1024 + 7;
        let blocks = plan_blocks(total, DEFAULT_BLOCK);
        assert_eq!(blocks.len() as u64, block_count(total, DEFAULT_BLOCK));
        let mut covered = 0;
        let mut prev_end = None;
        for (s, e) in &blocks {
            if let Some(p) = prev_end {
                assert_eq!(*s, p + 1, "gap or overlap");
            }
            assert!(e >= s);
            covered += e - s + 1;
            prev_end = Some(*e);
        }
        assert_eq!(covered, total);
        assert_eq!(blocks.last().unwrap().1, total - 1);
        assert!(plan_blocks(0, DEFAULT_BLOCK).is_empty());
    }

    const MB: u64 = 1024 * 1024;

    /// The point of sizing the slice from the stream count: a plan must contain
    /// at least as many blocks as workers, or the Settings control is a lie.
    #[test]
    fn adaptive_block_reaches_the_stream_count() {
        // 100 MB at the 8 MiB default is 13 slices, so 32 streams cannot run.
        assert_eq!(block_count(100 * MB, DEFAULT_BLOCK), 13);
        let sized = adaptive_block(100 * MB, 32, DEFAULT_BLOCK);
        assert_eq!(block_count(100 * MB, sized), 32);
        // A file that already affords the default keeps it, byte for byte.
        assert_eq!(adaptive_block(256 * MB, 32, DEFAULT_BLOCK), DEFAULT_BLOCK);
        assert_eq!(adaptive_block(64 * MB, 8, DEFAULT_BLOCK), DEFAULT_BLOCK);
        // Eight streams over 8 MB cannot be eight 8 MiB slices: the shrinks to
        // exactly one MiB each, which is what makes all eight workers real.
        assert_eq!(adaptive_block(8 * MB, 8, DEFAULT_BLOCK), MIN_BLOCK);
        assert_eq!(block_count(8 * MB, adaptive_block(8 * MB, 8, DEFAULT_BLOCK)), 8);
        // Below one slice per stream at the floor the floor wins: fewer workers
        // is honest, 256 KB requests are not.
        assert_eq!(adaptive_block(10 * MB, 32, DEFAULT_BLOCK), MIN_BLOCK);
        assert_eq!(adaptive_block(0, 8, DEFAULT_BLOCK), DEFAULT_BLOCK);
        // Covers must still be exact after resizing.
        let covered: u64 = plan_blocks(100 * MB, sized).iter().map(|(s, e)| e - s + 1).sum();
        assert_eq!(covered, 100 * MB);
    }

    #[test]
    fn retry_after_accepts_both_forms() {
        let now = 1_445_400_000; // 2015-10-21 04:00:00 UTC
        assert_eq!(retry_after_secs("30", now), Some(30));
        assert_eq!(retry_after_secs(" 120 ", now), Some(120));
        // A date 300 s out is shortened to the same cap a day-long request is.
        assert_eq!(retry_after_secs("Wed, 21 Oct 2015 04:01:00 GMT", now), Some(60));
        assert_eq!(
            retry_after_secs("Wed, 21 Oct 2015 04:05:00 GMT", now),
            Some(MAX_RETRY_WAIT)
        );
        assert_eq!(retry_after_secs("86400", now), Some(MAX_RETRY_WAIT));
        assert_eq!(retry_after_secs("21 Oct 2015 03:00:00 GMT", now), Some(0));
        assert_eq!(retry_after_secs("not a date", now), None);
        assert_eq!(retry_after_secs("", now), None);
        assert_eq!(retry_after_secs("-5", now), Some(0));
    }

    #[test]
    fn civil_days_match_the_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
        // 2000 was a leap year; 2023 and 2024 differ by one day over the turn
        // from February.
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(days_from_civil(2024, 3, 1), days_from_civil(2024, 2, 29) + 1);
        assert_eq!(days_from_civil(2023, 3, 1), days_from_civil(2023, 2, 28) + 1);
    }

    #[test]
    fn unique_names_never_collide() {
        let dir = std::env::temp_dir().join(format!("sucr-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "a.txt");
        assert_eq!(first.file_name().unwrap(), "a.txt");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "a.txt");
        assert_eq!(second.file_name().unwrap(), "a (1).txt");
        std::fs::write(&second, b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt").file_name().unwrap(), "a (2).txt");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn digests_match_known_vectors() {
        let dir = std::env::temp_dir().join(format!("sucr-digest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc.txt");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            file_digest(&file, "sha256").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(file_digest(&file, "md5").unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
