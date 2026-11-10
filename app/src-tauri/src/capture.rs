//! The capture door: the only surface a browser add-on may reach.
//!
//! Three intake functions, one pre-flight, one row read, and one window ask.
//! A request either normalises an intake (`/add`, `/adopt`, `/import`), asks
//! the engine's own pre-flight (`/probe`), reads one row (`/status`), or shows
//! the window of a hidden instance (`/show`, the relay's `hello`). The
//! window-only `sucr_invoke` dispatch is not reachable from here: `app:quit`
//! and `settings:set` must never answer a socket.
//!
//! Two gates, in this order. A token minted at boot and written beside the
//! discovery file, and an `Origin` that names an extension rather than a page.
//! A page can reach 127.0.0.1; it cannot set `Origin`, and it cannot read the
//! token, because the token only travels over the browser-authenticated
//! native-messaging port.
//!
//! Nothing is read from the browser: no cookie store, no history, no profile.
//! A capture carries the link, the name, the referrer and — for a partial —
//! a path the user's own browser already wrote.

use crate::{engine, ipc, AppState};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::Manager;

pub const PROTO_MIN: u8 = 1;
pub const PROTO_MAX: u8 = 1;

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoorInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: i64,
    pub proto_min: u8,
    pub proto_max: u8,
}

static DOOR: OnceLock<DoorInfo> = OnceLock::new();

pub fn door() -> Option<&'static DoorInfo> {
    DOOR.get()
}

/// The file the native host reads. It lives beside the database, so a harness
/// run with its own `--data-dir` can be discovered by a host launched with the
/// same flag and can never see the real install's token.
pub fn discovery_path(data_dir: &Path) -> PathBuf {
    data_dir.join("capture-door.json")
}

/// Binds one loopback port for this process and starts serving.
///
/// Port zero: the OS picks, and nothing can squat a number before us. The
/// listener is never handed to a page — the port reaches the add-on through the
/// native host, which the browser itself authenticated.
pub fn start(app: tauri::AppHandle, data_dir: &Path) {
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("capture door: {e}");
            crate::debug_log::write("ERROR", &format!("capture door did not bind: {e}"));
            return;
        }
    };
    let Ok(addr) = listener.local_addr() else { return };
    let info = DoorInfo {
        port: addr.port(),
        token: new_token(),
        pid: std::process::id(),
        started_at: crate::now_ms(),
        proto_min: PROTO_MIN,
        proto_max: PROTO_MAX,
    };
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(
        discovery_path(data_dir),
        serde_json::to_string(&info).unwrap_or_default(),
    );
    if DOOR.set(info).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("sucr-capture-door".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let app = app.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &app);
                });
            }
        })
        .ok();
}

/// 128 bits from the CSPRNG Windows already imports. A failure falls back to a
/// clock-and-address mix rather than refusing to open the door; the token is one
/// gate of two and the fallback still cannot be read by a page.
fn new_token() -> String {
    let mut bytes = [0u8; 16];
    #[cfg(windows)]
    {
        #[link(name = "bcryptprimitives", kind = "raw-dylib")]
        extern "system" {
            fn ProcessPrng(data: *mut u8, len: usize) -> i32;
        }
        let ok = unsafe { ProcessPrng(bytes.as_mut_ptr(), bytes.len()) };
        if ok == 0 {
            fallback_entropy(&mut bytes);
        }
    }
    #[cfg(not(windows))]
    fallback_entropy(&mut bytes);
    hex(&bytes)
}

fn fallback_entropy(out: &mut [u8; 16]) {
    use std::hash::{BuildHasher, Hasher};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(now);
    hasher.write_u32(std::process::id());
    out.copy_from_slice(&hasher.finish().to_le_bytes());
    let mut second = std::collections::hash_map::RandomState::new().build_hasher();
    second.write_u64(hasher.finish());
    second.write_u128(now.rotate_left(17));
    out[8..].copy_from_slice(&second.finish().to_le_bytes());
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

struct Request {
    method: String,
    path: String,
    query: String,
    origin: Option<String>,
    /// A service worker may not send `Origin`; this is the only way it can say
    /// which add-on is calling, and it is trusted only after the token.
    alt_origin: Option<String>,
    token: Option<String>,
    body: Vec<u8>,
}

fn serve(mut stream: TcpStream, app: &tauri::AppHandle) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_HEAD {
            return reply(&mut stream, 431, None, &decline("too-large", "request head too large"));
        }
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let Some(request) = parse_head(&head) else {
        return reply(&mut stream, 400, None, &decline("bad-request", "unreadable request"));
    };
    let content_length = header_value(&head, "content-length")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_BODY);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > MAX_BODY {
            break;
        }
    }
    body.truncate(content_length);
    let request = Request { body, ..request };
    let origin = request.origin.clone();

    if request.method == "OPTIONS" {
        return reply(&mut stream, 204, origin.as_deref(), &Value::Null);
    }
    if let Some(origin) = &origin {
        if !is_extension_origin(origin) {
            return reply(&mut stream, 403, None, &decline("origin", "not an extension origin"));
        }
    }

    if request.path == "/ping" {
        return reply(
            &mut stream,
            200,
            origin.as_deref(),
            &json!({
                "app": "sucr",
                "proto": [PROTO_MIN, PROTO_MAX],
                "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(),
            }),
        );
    }
    // Everything past `/ping` is the data path, and the token is what makes it
    // ours: a page that reaches this port has no way to present it.
    match door_token() {
        Some(expected) if request.token.as_deref() == Some(expected.as_str()) => {}
        Some(_) => return reply(&mut stream, 401, origin.as_deref(), &decline("token", "token refused")),
        None => return reply(&mut stream, 503, origin.as_deref(), &decline("closed", "the door is not open")),
    }

    // `Origin` is a forbidden header name, so a page cannot claim an extension
    // in it — but a service worker is not guaranteed to send one at all. The
    // stand-in header is only honoured past the token, so it cannot be used to
    // get a claim in before the credential has been presented.
    let origin = origin
        .or(request.alt_origin.clone())
        .filter(|o| is_extension_origin(o));
    if origin.is_none() {
        return reply(&mut stream, 403, None, &decline("origin", "no extension origin"));
    }
    let source = source_of(origin.as_deref());

    let state = app.state::<AppState>();
    let result = match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/add") => add(&state, app, &request.body, &source),
        ("POST", "/adopt") => adopt(&state, app, &request.body, &source),
        ("POST", "/import") => import(&state, app, &request.body, &source),
        ("POST", "/mux") => mux(&request.body),
        ("POST", "/drop") => drop_legs(&state, app, &request.body),
        ("POST", "/probe") => probe(&state, &request.body),
        ("GET", "/status") => status(&state, &request.query),
        // The popup's Open button, relayed: a capture may have started this
        // instance hidden, and `hello` must surface it, not just name its port.
        #[cfg(desktop)]
        ("POST", "/show") => {
            crate::show_main_window(app);
            Ok((200, json!({ "ok": true })))
        }
        _ => Err(Decline::new("not-found", "unknown route")),
    };
    match result {
        Ok((code, value)) => reply(&mut stream, code, origin.as_deref(), &value),
        Err(e) => reply(&mut stream, e.status, origin.as_deref(), &e.body()),
    }
}

fn door_token() -> Option<String> {
    door().map(|d| d.token.clone())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_head(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };
    Some(Request {
        method,
        path,
        query,
        origin: header_value(head, "origin").map(|v| v.trim().to_string()),
        alt_origin: header_value(head, "x-sucr-origin").map(|v| v.trim().to_string()),
        token: header_value(head, "x-sucr-token").map(|v| v.trim().to_string()),
        body: Vec::new(),
    })
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    for line in head.split("\r\n").skip(1) {
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                return Some(value);
            }
        }
    }
    None
}

/// A page is `https://`, `http://`, `file://`; an add-on is one of these two
/// schemes on both browsers. This is a second gate, never the first: the token
/// already decided, and this keeps a page from being the client even if a token
/// ever leaked.
fn is_extension_origin(origin: &str) -> bool {
    let rest = origin
        .strip_prefix("chrome-extension://")
        .or_else(|| origin.strip_prefix("moz-extension://"));
    matches!(rest, Some(id) if !id.is_empty() && id.len() <= 128)
}

fn source_of(origin: Option<&str>) -> String {
    match origin {
        Some(o) if o.starts_with("moz-extension://") => "firefox".to_string(),
        Some(o) if o.starts_with("chrome-extension://") => "chrome".to_string(),
        _ => "browser".to_string(),
    }
}

struct Decline {
    status: u16,
    code: &'static str,
    message: String,
}

impl Decline {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { status: 422, code, message: message.into() }
    }
    fn with_status(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }
    fn body(&self) -> Value {
        json!({ "ok": false, "code": self.code, "message": self.message })
    }
}

fn decline(code: &'static str, message: &str) -> Value {
    json!({ "ok": false, "code": code, "message": message })
}

fn body_json(body: &[u8]) -> Result<Value, Decline> {
    serde_json::from_slice(body)
        .map_err(|_| Decline::new("bad-json", "the hand-off body is not JSON"))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn check_url(url: &str) -> Result<(), Decline> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(());
    }
    Err(Decline::with_status(
        422,
        "scheme",
        "only http and https links can be captured",
    ))
}

/// The path the browser says it wrote, or the temp file it is still writing.
///
/// A Chromium download reports its target path while the bytes are landing in
/// `<target>.crdownload`, so the add-on sends the name the user will see and
/// the file that actually exists is derived here rather than guessed there.
fn resolve_written(path: PathBuf) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    if path.is_file() {
        return Some(path);
    }
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".crdownload");
    let temp = PathBuf::from(temp);
    temp.is_file().then_some(temp)
}

/// Take: the add-on hands the link and lets go of its own transfer.
fn add(
    state: &AppState,
    app: &tauri::AppHandle,
    body: &[u8],
    source: &str,
) -> Result<(u16, Value), Decline> {
    let spec = body_json(body)?;
    let url = text(&spec, "url").ok_or_else(|| Decline::new("url", "the hand-off names no link"))?;
    check_url(&url)?;
    let mut add = json!({ "url": url, "source": source });
    for key in ["filename", "referer", "folder", "ua", "cookie"] {
        if let Some(value) = text(&spec, key) {
            add[key] = Value::String(value);
        }
    }
    // The sound half of a DASH pair: the add-on asks for one file, so the job
    // fetches both streams and joins them before it publishes.
    if let Some(audio) = text(&spec, "audioUrl") {
        check_url(&audio)?;
        add["audioUrl"] = Value::String(audio);
    }
    let row = ipc::download_add(state, &add)
        .map_err(|e| Decline::new("rejected", e))?;
    let id = row.get("id").and_then(Value::as_i64).unwrap_or(0);
    let started = ipc::download_start(state, app, id)
        .map_err(|e| Decline::new("not-started", e))?;
    let _ = app_emit(app, id);
    Ok((
        200,
        json!({
            "ok": true,
            "id": id,
            "status": started.get("status").cloned().unwrap_or(Value::Null),
            "filename": started.get("filename").cloned().unwrap_or(Value::Null),
        }),
    ))
}

/// Mux: the browser handed us a "video only" stream and its matching "audio
/// only" stream have both finished, so combine them into one progressive file.
/// Nothing here re-encodes; the loopback caller names the two source files and
/// the destination, and the engine rewrites only the container headers.
fn mux(body: &[u8]) -> Result<(u16, Value), Decline> {
    let spec = body_json(body)?;
    let video = text(&spec, "videoPath")
        .ok_or_else(|| Decline::new("path", "mux names no video file"))?;
    let out = text(&spec, "outPath")
        .ok_or_else(|| Decline::new("path", "mux names no output file"))?;
    let audio = text(&spec, "audioPath");

    let video = resolve_written(PathBuf::from(&video))
        .ok_or_else(|| Decline::new("path", "the named video file is not there"))?;
    let audio = match &audio {
        Some(a) => Some(
            resolve_written(PathBuf::from(a))
                .ok_or_else(|| Decline::new("path", "the named audio file is not there"))?,
        ),
        None => None,
    };
    let out = PathBuf::from(&out);
    if !out.is_absolute() {
        return Err(Decline::new("path", "the output path is not absolute"));
    }
    // The muxer truncates the destination before it streams the picture, so a
    // named output equal to a source would erase its own input. Refuse it.
    if out == video || audio.as_deref() == Some(out.as_path()) {
        return Err(Decline::new("path", "the output file would overwrite a source"));
    }
    let out = unique_out(out);

    // Two remuxers, picked by the container the caller asked for: a VP9/Opus
    // pair stays WebM, everything else becomes a progressive MP4.
    let webm_out = out
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("webm"));
    if webm_out {
        let audio = audio
            .as_deref()
            .ok_or_else(|| Decline::new("path", "a webm merge needs its audio file"))?;
        engine::webm::mux(&video, audio, &out).map_err(|e| Decline::new("mux", e.0))?;
    } else {
        engine::mux::mux(&video, audio.as_deref(), &out).map_err(|e| Decline::new("mux", e.0))?;
    }
    Ok((200, json!({ "ok": true, "outPath": out.to_string_lossy() })))
}

/// A merge writes the name the caller asked for, but never over an earlier
/// download of the same video: a taken name grows a ` (n)` marker, the same
/// shape the download folder itself uses.
fn unique_out(mut out: PathBuf) -> PathBuf {
    if !out.exists() {
        return out;
    }
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = out.extension().map(|s| s.to_string_lossy().into_owned());
    for n in 1..1000 {
        let name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = out.with_file_name(name);
        if !candidate.exists() {
            out = candidate;
            break;
        }
    }
    out
}

/// Drop: a paired Take produced one merged file, so the two legs that fed the
/// muxer leave the library (and their files leave the disk) instead of leaving
/// the user three rows for one video. The app's own delete path does the work,
/// so a leg still writing is refused rather than orphaned.
fn drop_legs(
    state: &AppState,
    app: &tauri::AppHandle,
    body: &[u8],
) -> Result<(u16, Value), Decline> {
    let spec = body_json(body)?;
    let ids = spec
        .get("ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut dropped = Vec::new();
    for id in ids.iter().filter_map(Value::as_i64) {
        match crate::ipc::download_remove(state, id, true) {
            Ok(out) => {
                let _ = app_emit(app, id);
                dropped.push(out);
            }
            Err(e) => return Err(Decline::new("drop", e)),
        }
    }
    Ok((200, json!({ "ok": true, "dropped": dropped })))
}

/// Adopt: the browser finished, and the file only needs a place in the library.
fn adopt(
    state: &AppState,
    app: &tauri::AppHandle,
    body: &[u8],
    source: &str,
) -> Result<(u16, Value), Decline> {
    let spec = body_json(body)?;
    let path = text(&spec, "path").ok_or_else(|| Decline::new("path", "no file named"))?;
    let path = resolve_written(PathBuf::from(&path))
        .ok_or_else(|| Decline::new("path", "the named file is not there"))?;
    let url = text(&spec, "url");
    if let Some(url) = &url {
        check_url(url)?;
    }
    let filename = text(&spec, "filename").unwrap_or_else(|| {
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into())
    });
    let id = engine::job::adopt(
        Path::new(&state.paths.db_path),
        &state.paths.downloads_dir,
        engine::job::AdoptSpec {
            url,
            path,
            filename,
            referer: text(&spec, "referer"),
            source: source.to_string(),
        },
    )
    .map_err(|e| Decline::new("rejected", e.message))?;
    let _ = app_emit(app, id);
    let row = ipc::download_get_public(state, id).unwrap_or(Value::Null);
    Ok((200, json!({ "ok": true, "id": id, "status": "finished", "row": row })))
}

/// Import: the browser is paused mid-transfer; its contiguous prefix becomes
/// our staging file and the first Start resumes at the last byte it wrote.
fn import(
    state: &AppState,
    app: &tauri::AppHandle,
    body: &[u8],
    source: &str,
) -> Result<(u16, Value), Decline> {
    let spec = body_json(body)?;
    let url = text(&spec, "url").ok_or_else(|| Decline::new("url", "the hand-off names no link"))?;
    check_url(&url)?;
    let path = text(&spec, "path").ok_or_else(|| Decline::new("path", "no partial named"))?;
    let path = resolve_written(PathBuf::from(&path))
        .ok_or_else(|| Decline::new("path", "the partial is not there"))?;
    let id = engine::job::seed_partial(
        Path::new(&state.paths.db_path),
        &state.paths.downloads_dir,
        engine::job::PartialSpec {
            url,
            path,
            filename: text(&spec, "filename"),
            referer: text(&spec, "referer"),
            source: source.to_string(),
        },
    )
    .map_err(|e| Decline::new("rejected", e.message))?;
    let started = ipc::download_start(state, app, id)
        .map_err(|e| Decline::new("not-started", e))?;
    let _ = app_emit(app, id);
    Ok((
        200,
        json!({
            "ok": true,
            "id": id,
            "status": started.get("status").cloned().unwrap_or(Value::Null),
            "filename": started.get("filename").cloned().unwrap_or(Value::Null),
        }),
    ))
}

fn probe(state: &AppState, body: &[u8]) -> Result<(u16, Value), Decline> {
    let spec = body_json(body)?;
    let url = text(&spec, "url").ok_or_else(|| Decline::new("url", "no link to probe"))?;
    check_url(&url)?;
    // The same one-byte pre-flight the add bar uses, so the add-on's decision
    // and the app's answer cannot disagree.
    Ok((200, ipc::probe_link(Path::new(&state.paths.db_path), &url).unwrap_or(Value::Null)))
}

fn status(state: &AppState, query: &str) -> Result<(u16, Value), Decline> {
    let id = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("id="))
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(|| Decline::new("id", "no row named"))?;
    let row = ipc::download_get_public(state, id)
        .map_err(|e| Decline::new("rejected", e))?;
    Ok((200, json!({ "ok": true, "row": row })))
}

fn app_emit(app: &tauri::AppHandle, id: i64) -> Result<(), ()> {
    use tauri::Emitter;
    let _ = app.emit("download:changed", json!({ "id": id }));
    Ok(())
}

fn reply(stream: &mut TcpStream, status: u16, origin: Option<&str>, value: &Value) -> std::io::Result<()> {
    let body = if value.is_null() { Vec::new() } else { value.to_string().into_bytes() };
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        422 => "Unprocessable Content",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(origin) = origin.filter(|o| is_extension_origin(o)) {
        head.push_str(&format!("Access-Control-Allow-Origin: {origin}\r\n"));
        head.push_str("Access-Control-Allow-Headers: content-type, x-sucr-token, x-sucr-origin\r\n");
        head.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n");
        head.push_str("Access-Control-Max-Age: 600\r\n");
        head.push_str("Vary: Origin\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&body)?;
    stream.flush()
}

// ---------------------------------------------------------------------------
// Native messaging host registration (HKCU only, no elevation).
// ---------------------------------------------------------------------------

pub const HOST_NAME: &str = "app.sucr.capture";
pub const FIREFOX_ID: &str = "capture@sucr.local";
/// The stable id of the unpacked/`key`-pinned Chrome build in this repo. The
/// store assigns its own id on publication; that one is appended when known.
pub const CHROME_ID: &str = "hilnalhpghfpbphghnicaadhlegjaebf";

const CHROME_KEY: &str = r"Software\Google\Chrome\NativeMessagingHosts\app.sucr.capture";
const EDGE_KEY: &str = r"Software\Microsoft\Edge\NativeMessagingHosts\app.sucr.capture";
const BRAVE_KEY: &str = r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\app.sucr.capture";
const VIVALDI_KEY: &str = r"Software\Vivaldi\NativeMessagingHosts\app.sucr.capture";
const CHROMIUM_KEY: &str = r"Software\Chromium\NativeMessagingHosts\app.sucr.capture";
/// The root every Gecko-family browser reads, forks included — Zen, LibreWolf
/// and Waterfox keep Firefox's key rather than publishing one of their own.
const MOZILLA_KEY: &str = r"Software\Mozilla\NativeMessagingHosts\app.sucr.capture";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Chromium,
    Gecko,
}

impl Family {
    fn dir(self) -> &'static str {
        match self {
            Family::Chromium => "chrome",
            Family::Gecko => "firefox",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Family::Chromium => "Chromium",
            Family::Gecko => "Firefox",
        }
    }
    fn addons_url(self) -> &'static str {
        match self {
            // Where a person turns on Developer mode and loads the folder.
            Family::Chromium => "chrome://extensions/",
            Family::Gecko => "about:debugging#/runtime/this-firefox",
        }
    }
}

/// One HKCU root a browser reads, and the manifest shape its family understands.
struct Root {
    name: &'static str,
    family: Family,
    key: &'static str,
}

const ROOTS: [Root; 6] = [
    Root { name: "chrome", family: Family::Chromium, key: CHROME_KEY },
    Root { name: "edge", family: Family::Chromium, key: EDGE_KEY },
    Root { name: "brave", family: Family::Chromium, key: BRAVE_KEY },
    Root { name: "vivaldi", family: Family::Chromium, key: VIVALDI_KEY },
    Root { name: "chromium", family: Family::Chromium, key: CHROMIUM_KEY },
    Root { name: "firefox", family: Family::Gecko, key: MOZILLA_KEY },
];

/// Each family has one manifest, and every root of that family names it.
fn manifest_path(data_dir: &Path, family: Family) -> PathBuf {
    data_dir
        .join("native")
        .join(format!("{HOST_NAME}.{}.json", family.dir()))
}

fn origin_setting(data_dir: &Path, key: &str) -> Vec<String> {
    std::fs::read_to_string(data_dir.join("capture-origins.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get(key).cloned())
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default()
}

/// Writes both host manifests and the three registry values. Idempotent, and
/// rewritten on every real-profile boot: a moved exe is the failure this is
/// for, and a path compared to itself is cheaper than a stale key.
pub fn register_host(data_dir: &Path, exe: &Path) -> Value {
    let native = data_dir.join("native");
    let _ = std::fs::create_dir_all(&native);
    let mut chrome_origins = vec![format!("chrome-extension://{CHROME_ID}/")];
    chrome_origins.extend(origin_setting(data_dir, "chrome"));
    chrome_origins.sort();
    chrome_origins.dedup();
    let mut firefox_ids = vec![FIREFOX_ID.to_string()];
    firefox_ids.extend(origin_setting(data_dir, "firefox"));
    firefox_ids.sort();
    firefox_ids.dedup();
    let exe_text = exe.to_string_lossy().into_owned();
    let chrome = json!({
        "name": HOST_NAME,
        "description": "Sucr capture relay",
        "path": exe_text,
        "type": "stdio",
        "allowed_origins": chrome_origins,
    });
    let firefox = json!({
        "name": HOST_NAME,
        "description": "Sucr capture relay",
        "path": exe_text,
        "type": "stdio",
        "allowed_extensions": firefox_ids,
    });
    let chrome_path = manifest_path(data_dir, Family::Chromium);
    let firefox_path = manifest_path(data_dir, Family::Gecko);
    let _ = std::fs::write(&chrome_path, serde_json::to_vec_pretty(&chrome).unwrap_or_default());
    let _ = std::fs::write(&firefox_path, serde_json::to_vec_pretty(&firefox).unwrap_or_default());

    let mut browsers = Vec::new();
    #[cfg(windows)]
    for root in &ROOTS {
        let manifest = match root.family {
            Family::Chromium => &chrome_path,
            Family::Gecko => &firefox_path,
        };
        let registered = write_sz(root.key, &manifest.to_string_lossy()).is_ok();
        browsers.push(json!({
            "name": root.name,
            "family": root.family.dir(),
            "key": root.key,
            "registered": registered,
            "host": HOST_NAME,
        }));
    }
    json!({
        "ok": true,
        "hostExe": exe_text,
        "manifests": {
            "chrome": chrome_path.to_string_lossy(),
            "firefox": firefox_path.to_string_lossy(),
        },
        "browsers": browsers,
    })
}

/// Whether each browser's key still names the running executable, read back
/// from the registry rather than inferred from a write that returned success.
pub fn host_state(data_dir: &Path, exe: &Path) -> Value {
    #[cfg(windows)]
    {
        let exe_text = exe.to_string_lossy().into_owned();
        let mut browsers = Vec::new();
        let mut host: Option<String> = None;
        for root in &ROOTS {
            let manifest = registered_path(root.key);
            // Each browser names its own manifest, and the answer is whether
            // that manifest's `path` is this executable — not whether the key
            // exists, which a stale registration from an older install would
            // also satisfy.
            let registered = manifest
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|m| m.get("path").and_then(Value::as_str).map(str::to_string))
                .is_some_and(|p| p.eq_ignore_ascii_case(&exe_text));
            if host.is_none() {
                host = manifest.clone();
            }
            browsers.push(json!({
                "name": root.name,
                "family": root.family.dir(),
                "registered": registered,
                "manifest": manifest,
            }));
        }
        json!({
            "ok": true,
            "exe": exe_text,
            "host": host,
            "hostIsCurrentExe": host.as_ref().and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|m| m.get("path").and_then(Value::as_str).map(str::to_string))
                .is_some_and(|p| p.eq_ignore_ascii_case(&exe_text)),
            "browsers": browsers,
            "addon": addon_report(data_dir, exe),
            "dataDir": data_dir.to_string_lossy(),
        })
    }
    #[cfg(not(windows))]
    {
        json!({ "ok": false, "browsers": [], "addon": addon_report(data_dir, exe) })
    }
}

#[cfg(windows)]
fn write_sz(key: &str, value: &str) -> Result<(), ()> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::WIN32_ERROR;
    // `RegCreateKeyW` rather than the extended form: the extended one is gated
    // behind the whole `Win32_Security` feature, and nothing here wants a
    // security descriptor or a disposition flag — only "opened, and the value
    // was written".
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_SZ,
    };
    unsafe {
        let mut handle = HKEY::default();
        let created = RegCreateKeyW(HKEY_CURRENT_USER, &HSTRING::from(key), &mut handle);
        if created != WIN32_ERROR(0) {
            return Err(());
        }
        let mut wide: Vec<u16> = value.encode_utf16().collect();
        wide.push(0);
        let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
        let set = RegSetValueExW(handle, &HSTRING::from(""), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(handle);
        if set != WIN32_ERROR(0) {
            return Err(());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn registered_path(key: &str) -> Option<String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut size = 0u32;
    unsafe {
        // First call asks for the length.
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(""),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        );
        if size < 2 || size > 8192 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let got = RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(""),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        );
        if got != WIN32_ERROR(0) {
            return None;
        }
        let wide: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|c| *c != 0)
            .collect();
        Some(String::from_utf16_lossy(&wide))
    }
}

// ---------------------------------------------------------------------------
// The bundled add-on: which browsers this machine has, where the installer put
// the add-on, and the one gesture each browser needs. No store listing, no
// managed-policy force-install, and no write into a browser profile.
// ---------------------------------------------------------------------------

pub const ADDON_FAMILIES: [&str; 2] = ["chrome", "firefox"];

fn exe_dir(exe: &Path) -> PathBuf {
    exe.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where one family's add-on actually is. The installer copies it beside the
/// exe (`addon/<family>`); Tauri's resource folder is the second candidate, and
/// the repo layout is the last, so a development build stages the same folders
/// the installer ships.
fn find_addon(exe: &Path, family: &str) -> Option<PathBuf> {
    let dir = exe_dir(exe);
    let mut candidates = vec![
        dir.join("addon").join(family),
        dir.join("resources").join("addon").join(family),
    ];
    for ancestor in dir.ancestors() {
        candidates.push(ancestor.join("extension").join("dist").join(family));
    }
    candidates
        .into_iter()
        .find(|p| p.join("manifest.json").is_file())
}

fn staged_dir(data_dir: &Path, family: &str) -> PathBuf {
    data_dir.join("addon").join(family)
}

fn copy_dir(from: &Path, to: &Path) -> Result<u64, String> {
    // Replace rather than merge: a file an older build shipped and this one
    // does not would otherwise be loaded beside the new ones.
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    copy_into(from, to)
}

fn copy_into(from: &Path, to: &Path) -> Result<u64, String> {
    let mut files = 0u64;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            files += copy_into(&source, &target)?;
        } else {
            std::fs::copy(&source, &target).map_err(|e| format!("{}: {e}", source.display()))?;
            files += 1;
        }
    }
    Ok(files)
}

/// Copies both families beside the database. A sideloaded add-on is loaded from
/// a folder on disk, so that folder must outlive the installer's own temp copy
/// and stay readable when the browser starts without Sucr running.
pub fn stage_addon(data_dir: &Path, exe: &Path) -> Value {
    let mut staged = serde_json::Map::new();
    for family in ADDON_FAMILIES {
        let target = staged_dir(data_dir, family);
        let answer = match find_addon(exe, family) {
            Some(source) => match copy_dir(&source, &target) {
                Ok(files) => json!({
                    "ok": true,
                    "path": target.to_string_lossy(),
                    "files": files,
                    "source": source.to_string_lossy(),
                }),
                Err(error) => json!({
                    "ok": false,
                    "path": target.to_string_lossy(),
                    "error": error,
                }),
            },
            None => json!({ "ok": false, "error": "this install carries no add-on folder for the family" }),
        };
        staged.insert(family.to_string(), answer);
    }
    Value::Object(staged)
}

#[derive(Clone)]
pub struct Browser {
    pub name: String,
    pub family: Family,
    pub exe: String,
    pub key: &'static str,
    pub exists: bool,
}

/// Every browser the shell lists as an internet client, classified by the
/// executable it opens with. A fork needs no code to be found: it only needs a
/// stem that is not already classified as the other family.
pub fn detect_browsers() -> Vec<Browser> {
    #[cfg(windows)]
    {
        let mut found: Vec<Browser> = Vec::new();
        for (encoded, command) in start_menu_entries() {
            let Some(exe) = command_exe(&command) else { continue };
            let Some((family, key)) = classify(&exe) else { continue };
            if found.iter().any(|b| b.exe.eq_ignore_ascii_case(&exe)) {
                continue;
            }
            let exists = Path::new(&exe).is_file();
            let name = browser_label(&exe, &encoded);
            found.push(Browser { name, family, exe, key, exists });
        }
        found.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        found
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn start_menu_entries() -> Vec<(String, String)> {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    };
    let mut out = Vec::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut clients = HKEY::default();
        let opened = unsafe {
            RegOpenKeyExW(
                hive,
                &HSTRING::from(r"Software\Clients\StartMenuInternet"),
                None,
                KEY_READ,
                &mut clients,
            )
        };
        if opened != WIN32_ERROR(0) {
            continue;
        }
        for index in 0..128u32 {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            let step = unsafe {
                RegEnumKeyExW(
                    clients,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if step != WIN32_ERROR(0) {
                break;
            }
            let encoded = String::from_utf16_lossy(&name[..len as usize]);
            if encoded.eq_ignore_ascii_case("IEXPLORE.EXE") {
                continue;
            }
            let sub = format!(r"Software\Clients\StartMenuInternet\{encoded}\shell\open\command");
            if let Some(command) = read_default(hive, &sub) {
                out.push((encoded, command));
            }
        }
        let _ = unsafe { RegCloseKey(clients) };
    }
    out
}

/// A key's unnamed value: what `shell\open\command` stores. The hive is the one
/// the entry was enumerated from — a machine-wide entry stores its command in
/// HKLM, and reading HKCU for it would answer about a different registration.
#[cfg(windows)]
fn read_default(hive: windows::Win32::System::Registry::HKEY, key: &str) -> Option<String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, KEY_READ};
    let mut handle = HKEY::default();
    let opened = unsafe { RegOpenKeyExW(hive, &HSTRING::from(key), None, KEY_READ, &mut handle) };
    if opened != WIN32_ERROR(0) {
        return None;
    }
    let mut size = 0u32;
    unsafe {
        let _ = RegQueryValueExW(handle, PCWSTR::null(), None, None, None, Some(&mut size));
    }
    if size < 2 || size > 32 * 1024 {
        let _ = unsafe { RegCloseKey(handle) };
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let mut got_size = size;
    let got = unsafe {
        RegQueryValueExW(
            handle,
            PCWSTR::null(),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut got_size),
        )
    };
    let _ = unsafe { RegCloseKey(handle) };
    if got != WIN32_ERROR(0) {
        return None;
    }
    let wide: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|c| *c != 0)
        .collect();
    let text = String::from_utf16_lossy(&wide);
    (!text.trim().is_empty()).then_some(text)
}

/// The executable out of a command line: `"C:\…\zen.exe"` or the bare form.
fn command_exe(command: &str) -> Option<String> {
    let text = command.trim();
    let path = match text.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => text.split_whitespace().next()?,
    };
    let end = path.to_ascii_lowercase().find(".exe").map(|i| i + 4)?;
    Some(path[..end].to_string())
}

fn classify(exe: &str) -> Option<(Family, &'static str)> {
    let stem = Path::new(exe)
        .file_stem()?
        .to_string_lossy()
        .to_ascii_lowercase();
    Some(match stem.as_str() {
        "msedge" => (Family::Chromium, EDGE_KEY),
        "brave" => (Family::Chromium, BRAVE_KEY),
        "vivaldi" => (Family::Chromium, VIVALDI_KEY),
        "chromium" => (Family::Chromium, CHROMIUM_KEY),
        // Chrome's root is also what most Chromium forks read when they publish
        // no root of their own.
        "chrome" | "thorium" | "slimjet" => (Family::Chromium, CHROME_KEY),
        "firefox" | "firefox-esr" | "zen" | "librewolf" | "waterfox" | "floorp" | "palemoon" => {
            (Family::Gecko, MOZILLA_KEY)
        }
        _ => return None,
    })
}

/// A readable name. A fork that ships under a generic `chrome.exe` (Helium) is
/// named by the registration key the browser itself wrote, so the list says
/// "Helium", not a second "Chrome".
fn browser_label(exe: &str, encoded: &str) -> String {
    let stem = Path::new(exe)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let generic = matches!(stem.as_str(), "chrome" | "chromium" | "msedge" | "firefox" | "firefox-esr");
    if !generic {
        return title(&stem);
    }
    let base = encoded.split(['-', '.']).next().unwrap_or(encoded);
    let base = base
        .strip_prefix("Google ")
        .or_else(|| base.strip_prefix("Microsoft "))
        .unwrap_or(base);
    title(base)
}

fn title(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(windows)]
fn browser_registered(key: &str, exe: &str) -> bool {
    registered_path(key)
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|m| m.get("path").and_then(Value::as_str).map(str::to_string))
        .is_some_and(|p| p.eq_ignore_ascii_case(exe))
}

#[cfg(not(windows))]
fn browser_registered(_key: &str, _exe: &str) -> bool {
    false
}

/// What the Settings card and the snapshot both show. Detection is live, so
/// installing a browser later needs no reinstall of Sucr — only a refresh.
pub fn addon_report(data_dir: &Path, exe: &Path) -> Value {
    let exe_text = exe.to_string_lossy().into_owned();
    let browsers: Vec<Value> = detect_browsers()
        .into_iter()
        .map(|b| {
            json!({
                "name": b.name,
                "family": b.family.dir(),
                "familyLabel": b.family.label(),
                "exe": b.exe,
                "exists": b.exists,
                "registered": browser_registered(b.key, &exe_text),
                "addonsUrl": b.family.addons_url(),
            })
        })
        .collect();
    let chrome = staged_dir(data_dir, "chrome");
    let firefox = staged_dir(data_dir, "firefox");
    json!({
        "staged": {
            "chrome": chrome.to_string_lossy(),
            "firefox": firefox.to_string_lossy(),
            "chromePresent": chrome.join("manifest.json").is_file(),
            "firefoxPresent": firefox.join("manifest.json").is_file(),
        },
        "browsers": browsers,
    })
}

/// The installer's whole ask: stage the add-on and register the relay for every
/// root a browser here reads. Idempotent, and safe to run while the app is up —
/// it touches no database, no window and no tray.
pub fn install_addon(data_dir: &Path, exe: &Path) -> Value {
    let staged = stage_addon(data_dir, exe);
    let host = register_host(data_dir, exe);
    json!({
        "ok": true,
        "staged": staged,
        "host": host,
        "addon": addon_report(data_dir, exe),
    })
}

/// The uninstaller's ask: the registrations this app wrote, the manifests it
/// wrote, and the staged folder. Nothing else in the data directory is touched.
pub fn remove_addon(data_dir: &Path) -> Value {
    let mut keys = Vec::new();
    #[cfg(windows)]
    for root in &ROOTS {
        keys.push(json!({ "key": root.key, "removed": delete_tree(root.key) }));
    }
    #[cfg(not(windows))]
    let _ = &mut keys;
    for family in [Family::Chromium, Family::Gecko] {
        let _ = std::fs::remove_file(manifest_path(data_dir, family));
    }
    let _ = std::fs::remove_dir_all(data_dir.join("addon"));
    json!({ "ok": true, "keys": keys })
}

#[cfg(windows)]
fn delete_tree(key: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};
    // 2 is ERROR_FILE_NOT_FOUND: a root no browser here ever wrote is already
    // in the state removal is asking for.
    let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
    code.0 == 0 || code.0 == 2
}

/// The Settings card's verbs, in one channel. `target` can only select a
/// browser the machine itself registered, never a path a caller supplied.
pub fn addon_command(data_dir: &Path, exe: &Path, action: &str, target: Option<&str>) -> Value {
    match action {
        "install" => install_addon(data_dir, exe),
        "open" => open_browser(target),
        "folder" => reveal_addon(data_dir),
        _ => json!({ "ok": true, "addon": addon_report(data_dir, exe) }),
    }
}

fn open_browser(target: Option<&str>) -> Value {
    let Some(wanted) = target else {
        return json!({ "ok": false, "error": "no browser was named" });
    };
    let Some(browser) = detect_browsers().into_iter().find(|b| {
        b.exe.eq_ignore_ascii_case(wanted) || b.name.eq_ignore_ascii_case(wanted)
    }) else {
        return json!({ "ok": false, "error": "that browser is not registered on this machine" });
    };
    match std::process::Command::new(&browser.exe)
        .arg(browser.family.addons_url())
        .spawn()
    {
        Ok(_) => json!({
            "ok": true,
            "browser": browser.name,
            "url": browser.family.addons_url(),
        }),
        Err(e) => json!({ "ok": false, "error": format!("could not start {}: {e}", browser.name) }),
    }
}

fn reveal_addon(data_dir: &Path) -> Value {
    let dir = data_dir.join("addon");
    let _ = std::fs::create_dir_all(&dir);
    match std::process::Command::new("explorer.exe").arg(&dir).spawn() {
        Ok(_) => json!({ "ok": true, "path": dir.to_string_lossy() }),
        Err(e) => json!({ "ok": false, "error": format!("could not open the folder: {e}") }),
    }
}
