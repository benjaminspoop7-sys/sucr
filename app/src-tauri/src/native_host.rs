//! The native-messaging relay a browser spawns.
//!
//! This process is deliberately not the app: it opens no database, creates no
//! tray icon, binds no port. It reads the door's discovery file (the same
//! `--data-dir` the running app used), optionally starts the app when the file
//! is stale, and answers over stdio. A second app process is exactly what the
//! capture design forbids, so the relay never becomes one.
//!
//! Framing is Chrome's: a 32-bit native-order length, then UTF-8 JSON.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_MESSAGE: usize = 1024 * 1024;

pub fn run() -> i32 {
    let mut input = std::io::stdin();
    let mut output = std::io::stdout();
    loop {
        let Some(body) = read_message(&mut input) else { return 0 };
        let message: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        // Firefox announces who is calling before any request and expects no
        // answer to that notice; replying would shift every later response by
        // one slot and the add-on would read a door for the wrong call.
        if message.get("nativeApp").is_some()
            && message.get("cmd").is_none()
            && message.get("op").is_none()
        {
            continue;
        }
        let reply = handle(&message);
        if write_message(&mut output, &reply).is_err() {
            return 0;
        }
    }
}

fn read_message(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    input.read_exact(&mut len).ok()?;
    let n = u32::from_ne_bytes(len) as usize;
    if n == 0 || n > MAX_MESSAGE {
        return None;
    }
    let mut buf = vec![0u8; n];
    input.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn write_message(output: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    output.write_all(&(body.len() as u32).to_ne_bytes())?;
    output.write_all(&body)?;
    output.flush()
}

fn handle(message: &Value) -> Value {
    // `op` is what the add-on sends; `cmd` is the same word from a hand-run
    // probe. Either spelling answers the same way.
    let cmd = message
        .get("cmd")
        .or_else(|| message.get("op"))
        .and_then(Value::as_str)
        .unwrap_or("hello");
    let data_dir = crate::paths::resolve_data_dir();
    match cmd {
        "register" => crate::capture::register_host(&data_dir, &current_exe()),
        "state" => crate::capture::host_state(&data_dir, &current_exe()),
        // `status` answers only what is already there. Asking whether Sucr is
        // running must not be the thing that starts it. `wake` starts it hidden
        // for a capture; `hello` starts it with a window and belongs to the
        // popup's Open button alone.
        "status" => match read_door(&data_dir).filter(|d| ping(d.port)) {
            Some(door) => door_answer(door, &data_dir),
            None => not_running(&data_dir),
        },
        "hello" => match wait_for_door(&data_dir, false) {
            Some(door) => {
                // The app may already be up hidden: a capture starts it that
                // way, and the Open button is the one ask that must surface
                // the window. Showing an already-visible window just focuses
                // it, so the ask is the same on both paths.
                ask_show(&door);
                door_answer(door, &data_dir)
            }
            None => not_running(&data_dir),
        },
        // A capture may bring the app up, but never its window: the tray and the
        // door are enough to hold a transfer, and a window the user did not ask
        // for is the complaint this command exists to answer.
        "wake" => match wait_for_door(&data_dir, true) {
            Some(door) => door_answer(door, &data_dir),
            None => not_running(&data_dir),
        },
        other => json!({ "ok": false, "code": "unknown", "message": format!("unknown command {other}") }),
    }
}

fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sucr.exe"))
}

/// The door a live process is listening on, starting the app once when there is
/// none. A file left behind by a killed process is not a door: its port refuses
/// a connection, and that is the test.
fn wait_for_door(data_dir: &Path, background: bool) -> Option<crate::capture::DoorInfo> {
    if let Some(door) = read_door(data_dir) {
        if ping(door.port) {
            return Some(door);
        }
    }
    start_app(data_dir, background);
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(120));
        if let Some(door) = read_door(data_dir) {
            if ping(door.port) {
                return Some(door);
            }
        }
    }
    None
}

fn door_answer(door: crate::capture::DoorInfo, data_dir: &Path) -> Value {
    let mut out = serde_json::to_value(door).unwrap_or_else(|_| json!({}));
    if let Some(map) = out.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("app".into(), Value::String("sucr".into()));
        map.insert(
            "version".into(),
            Value::String(env!("CARGO_PKG_VERSION").into()),
        );
        map.insert(
            "dataDir".into(),
            Value::String(data_dir.to_string_lossy().into_owned()),
        );
    }
    out
}

fn not_running(data_dir: &Path) -> Value {
    json!({
        "ok": false,
        "app": "sucr",
        "code": "not-running",
        "message": "Sucr is not running",
        "dataDir": data_dir.to_string_lossy(),
    })
}

fn read_door(data_dir: &Path) -> Option<crate::capture::DoorInfo> {
    let text = std::fs::read_to_string(crate::capture::discovery_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

fn ping(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// The relay's own door request: the token it just read, and an extension
/// origin naming this host rather than a browser add-on. `/show` keeps no
/// attribution, so the synthetic identity costs nothing; the token gate is
/// still the first thing the request answers.
fn ask_show(door: &crate::capture::DoorInfo) {
    use std::io::Write;
    use std::net::TcpStream;
    let Ok(mut stream) = TcpStream::connect((std::net::Ipv4Addr::new(127, 0, 0, 1), door.port)) else { return };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let req = format!(
        "POST /show HTTP/1.1\r\nHost: 127.0.0.1\r\nx-sucr-token: {}\r\nx-sucr-origin: chrome-extension://sucr-native-relay\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        door.token
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return;
    }
    let mut buf = [0u8; 512];
    let _ = stream.read(&mut buf);
}

/// Launches the window exactly as the user would, detached from the browser's
/// pipes so the browser cannot close the app by closing the host.
fn start_app(data_dir: &Path, background: bool) {
    let exe = current_exe();
    let mut command = std::process::Command::new(&exe);
    // Only a harness override travels with the child: passing the platform
    // default as `--data-dir` would move the user's downloads inside AppData
    // and mark the install as a harness profile.
    let overridden = std::env::args().any(|a| a.starts_with("--data-dir="))
        || std::env::var("SUCR_DATA_DIR").map(|v| !v.is_empty()).unwrap_or(false);
    if overridden {
        command.arg(format!("--data-dir={}", data_dir.display()));
    }
    if background {
        command.arg("--background");
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let _ = command.spawn();
}

/// True when this process was launched by a browser rather than by a person.
///
/// Chromium 156 passes the caller's origin then `--parent-window=0`, started
/// through `cmd.exe /d /s /c "<exe> <origin> --parent-window=0"` with named
/// pipes for stdio; Firefox passes no flag at all, only the manifest it read
/// and the id of the add-on that asked, as two bare arguments. Both shapes are
/// witnessed here, in a real browser, from `Win32_Process.CommandLine`. The
/// legacy `-parentHWND <handle>` is matched on Chrome's and Edge's published
/// contract only: no browser on this machine passes it, so that leg is
/// unit-tested, not browser-witnessed. Recognising every shape is
/// what lets one executable be the window and the host, with no second binary
/// to bundle. Getting this wrong is not silent: the spawn opens a second app
/// window against the user's own data directory instead of answering the
/// browser.
pub fn launched_by_browser(args: &[String]) -> bool {
    if args.iter().skip(1).any(|arg| {
        arg == "--native-host"
            || arg == "-parentHWND"
            || arg.starts_with("chrome-extension://")
            || arg.starts_with("moz-extension://")
    }) {
        return true;
    }
    let Some(manifest) = args.get(1) else {
        return false;
    };
    let name = manifest.rsplit(|c| c == '\\' || c == '/').next().unwrap_or(manifest);
    args.get(2).is_some()
        && name.starts_with("app.sucr.capture.")
        && name.ends_with(".json")
        && Path::new(manifest).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn firefox_shape_is_a_host_launch() {
        let dir = std::env::temp_dir().join(format!("sucr-host-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("app.sucr.capture.firefox.json");
        std::fs::write(&manifest, "{}").unwrap();
        assert!(launched_by_browser(&a(&[
            "sucr.exe",
            &manifest.to_string_lossy(),
            "capture@sucr.local"
        ])));
        // The same two arguments naming a manifest that is not there is a person.
        let missing = dir.join("app.sucr.capture.firefox.json.bak");
        assert!(!launched_by_browser(&a(&[
            "sucr.exe",
            &missing.to_string_lossy(),
            "capture@sucr.local"
        ])));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chrome_and_plain_launches_are_unchanged() {
        // Contract-only: no browser installed here produces this argv.
        assert!(launched_by_browser(&a(&["sucr.exe", "-parentHWND", "1234"])));
        assert!(launched_by_browser(&a(&["sucr.exe", "--native-host"])));
        assert!(!launched_by_browser(&a(&["sucr.exe"])));
        assert!(!launched_by_browser(&a(&["sucr.exe", "--smoke"])));
        // A hidden capture-driven start is the app, not the relay.
        assert!(!launched_by_browser(&a(&["sucr.exe", "--background"])));
        assert!(!launched_by_browser(&a(&["sucr.exe", "http://x/y.pdf"])));
    }

    /// Caught from the OS command line of a host a real Chromium 156 had just
    /// started: the caller's origin first, then `--parent-window=0`. The origin
    /// is the leg that decides, which is why the window switch is not matched.
    #[test]
    fn the_chromium_shape_a_real_browser_passes_is_a_host_launch() {
        assert!(launched_by_browser(&a(&[
            r"D:\RE\Sucr\sucr.exe",
            "chrome-extension://hilnalhpghfpbphghnicaadhlegjaebf/",
            "--parent-window=0"
        ])));
    }
}
