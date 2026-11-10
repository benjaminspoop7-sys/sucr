//! The debug log: one file in `logs/` beside the database, written only while
//! the user leaves the switch on (it ships on).
//!
//! A windowed build has no console, so every `eprintln!` in the engine reaches
//! nothing a user can read and nothing survives the process. This module is the
//! place a crash, an IPC refusal or a job that never started can still be read
//! after the window is gone. It is deliberately not a logging framework: one
//! sink, one line per event, no levels beyond a word at the start of the line.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ON: AtomicBool = AtomicBool::new(false);
static SINK: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// One roll at this size. A log nobody reads must not grow without bound, and
/// the previous file is kept exactly once: `sucr.log` and `sucr.previous.log`
/// are the whole footprint.
const MAX_BYTES: u64 = 1 << 20;

pub const FILE: &str = "sucr.log";

pub fn path(dir: &str) -> std::path::PathBuf {
    std::path::Path::new(dir).join(FILE)
}

fn open(dir: &str) -> Option<std::fs::File> {
    let dir = std::path::Path::new(dir);
    let _ = std::fs::create_dir_all(dir);
    let file = dir.join(FILE);
    if std::fs::metadata(&file).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&file, dir.join("sucr.previous.log"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .ok()
}

fn lock() -> std::sync::MutexGuard<'static, Option<std::fs::File>> {
    SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Turn the log on or off at runtime. Off also drops the handle, so a file a
/// user wants to delete is no longer held open by Sucr.
pub fn switch(dir: &str, on: bool) {
    let mut sink = lock();
    *sink = if on { open(dir) } else { None };
    ON.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Best effort by design: a log that cannot be written must never be the reason
/// the caller fails, so every error here is dropped.
pub fn write(level: &str, message: &str) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let mut sink = lock();
    emit(sink.as_mut(), level, message);
}

/// The panic hook's own writer. `try_lock` because a panic while the sink is
/// held (a worker thread mid-write) must not deadlock the abort path; that one
/// line is lost, which is better than losing the message.
fn write_panic(level: &str, message: &str) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut sink) = SINK.try_lock() {
        emit(sink.as_mut(), level, message);
    }
}

fn emit(sink: Option<&mut std::fs::File>, level: &str, message: &str) {
    let Some(file) = sink else { return };
    let _ = writeln!(file, "{} {level:<5} {message}", stamp());
    let _ = file.flush();
}

/// Empty the log. The handle is dropped and the file reopened with truncate:
/// an append-mode handle carries `FILE_APPEND_DATA`, and asking it to shrink is
/// not the same thing as asking a handle that may write from the start.
pub fn truncate(dir: &str) {
    let mut sink = lock();
    *sink = None;
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path(dir));
    if ON.load(Ordering::Relaxed) {
        *sink = open(dir);
    }
}

/// The last `count` lines, oldest first. Read through the handle's path rather
/// than the handle so a caller never seeks the sink out from under a writer.
pub fn tail(dir: &str, count: usize) -> String {
    let text = match std::fs::read_to_string(path(dir)) {
        Ok(text) => text,
        Err(_) => return String::new(),
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join("\n")
}

pub fn bytes(dir: &str) -> u64 {
    std::fs::metadata(path(dir)).map(|m| m.len()).unwrap_or(0)
}

/// Record anything that ends the process without a chance to say why. The
/// previous hook is kept: under a console (`cargo test`, `--smoke` runs) its
/// stderr line is still the fastest thing to read.
pub fn install_hook() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic with a non-text payload".to_string());
        let site = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        write_panic("PANIC", &format!("{payload}{site}"));
        previous(info);
    }));
}

/// `1970-01-01T00:00:00Z`-shaped UTC. No date crate: the roll of `civil` below
/// is a dozen lines of integer arithmetic, and a local-time zone would need a
/// Windows call this app otherwise does not make.
fn stamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (y, m, d) = civil(seconds.div_euclid(86_400));
    let rem = seconds.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since the epoch to (year, month, day). Howard Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stamp has to be a real UTC date, not just digits: a log a human reads
    /// after a crash is useless if the day is off by one, and 2024-02-29 is the
    /// case a naive month table gets wrong.
    #[test]
    fn the_stamp_names_the_right_utc_day() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(civil(20_000), (2024, 10, 4));
        assert_eq!(civil(-1), (1969, 12, 31));
        let line = stamp();
        assert!(line.ends_with('Z'), "{line}");
        assert_eq!(line.len(), 20, "{line}");
    }

    #[test]
    fn writes_go_to_the_file_only_while_the_switch_is_on() {
        let dir = std::env::temp_dir().join(format!("sucr-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_s = dir.display().to_string();

        switch(&dir_s, false);
        write("INFO", "must not land");
        assert_eq!(bytes(&dir_s), 0);

        switch(&dir_s, true);
        write("INFO", "first line");
        write("ERROR", "second line");
        let text = std::fs::read_to_string(path(&dir_s)).expect("log written");
        assert!(text.contains("first line"), "{text}");
        assert!(text.contains("ERROR second line"), "{text}");
        assert!(!text.contains("must not land"), "{text}");

        truncate(&dir_s);
        assert_eq!(bytes(&dir_s), 0);
        write("INFO", "after clear");
        assert!(tail(&dir_s, 10).contains("after clear"));

        switch(&dir_s, false);
        let before = bytes(&dir_s);
        write("INFO", "dropped");
        assert_eq!(bytes(&dir_s), before);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_keeps_the_newest_lines_in_order() {
        let dir = std::env::temp_dir().join(format!("sucr-log-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_s = dir.display().to_string();
        switch(&dir_s, true);
        for i in 0..50 {
            write("INFO", &format!("line {i}"));
        }
        let tail = tail(&dir_s, 3);
        let lines: Vec<&str> = tail.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("line 49"), "{tail}");
        assert!(lines[0].contains("line 47"), "{tail}");
        switch(&dir_s, false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
