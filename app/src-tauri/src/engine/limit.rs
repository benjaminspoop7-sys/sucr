// Token-bucket rate limiting for transfers.
//
// A bucket books bytes after they leave the socket and before they reach the
// file, so a capped transfer slows at the read instead of buffering unthrottled
// data in memory. Two scopes can apply to one transfer (global and per-job):
// each bucket books the charge and reports how long its own rate needs for
// those bytes, and the thread sleeps the longest wait once. Charging the waits
// one after another would make the two caps add up instead of intersecting.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::job::RUN;
use super::CHUNK;

/// Smallest read slice. A cap this low still needs a burst to make progress.
pub const MIN_SLICE: usize = 8 * 1024;
/// Sleep granularity: the longest a paused transfer waits before it notices.
const MAX_TICK: Duration = Duration::from_millis(50);

/// One scope's bucket. `None` means unlimited and costs no locking beyond a
/// pointer read on the cap.
pub struct Bucket {
    state: Mutex<State>,
}

struct State {
    cap: Option<u64>,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    pub fn new(cap: Option<u64>) -> Self {
        let cap = cap.filter(|c| *c > 0);
        Self {
            state: Mutex::new(State {
                cap,
                tokens: cap.map_or(0.0, |c| slice_of(c) as f64),
                last: Instant::now(),
            }),
        }
    }

    pub fn cap(&self) -> Option<u64> {
        self.lock().cap
    }

    /// Runtime control. A fresh bucket starts full so a download begins with a
    /// single burst, then settles into the cap.
    pub fn set_cap(&self, cap: Option<u64>) {
        let cap = cap.filter(|c| *c > 0);
        let mut st = self.lock();
        st.cap = cap;
        st.tokens = cap.map_or(0.0, |c| slice_of(c) as f64);
        st.last = Instant::now();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Books `n` bytes and returns how long the bucket's rate needs for them.
    /// Tokens may go negative and stay there: every thread that reads ahead
    /// books its slice, and discarding that debt would let parallel streams
    /// jointly exceed the cap.
    fn reserve(&self, n: u64) -> Duration {
        let mut st = self.lock();
        let Some(cap) = st.cap else {
            return Duration::ZERO;
        };
        let cap_f = cap as f64;
        let capacity = slice_of(cap) as f64;
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(st.last).as_secs_f64();
        st.last = now;
        st.tokens = (st.tokens + elapsed * cap_f).min(capacity);
        let wait = ((n as f64 - st.tokens).max(0.0) / cap_f).min(60.0);
        st.tokens -= n as f64;
        Duration::from_secs_f64(wait)
    }
}

/// Read size for a cap: the bucket drains a slice in about 1/20 s, within a
/// 8 KiB..256 KiB window.
pub fn slice_of(cap: u64) -> usize {
    (cap / 20).clamp(MIN_SLICE as u64, CHUNK as u64) as usize
}

/// Read size that satisfies every scope on a transfer.
pub fn read_slice(buckets: &[&Bucket]) -> usize {
    buckets
        .iter()
        .filter_map(|b| b.cap())
        .map(slice_of)
        .min()
        .unwrap_or(CHUNK)
}

/// Books `n` bytes against every scope, then sleeps the longest wait in
/// <=50 ms steps. Returns Err as soon as the job leaves RUN so a pause stops
/// traffic instead of waiting out the debt.
pub fn charge(buckets: &[&Bucket], n: usize, cancel: &AtomicU8) -> Result<(), ()> {
    let mut wait = Duration::ZERO;
    for bucket in buckets {
        wait = wait.max(bucket.reserve(n as u64));
    }
    sleep(wait, cancel)
}

fn sleep(mut left: Duration, cancel: &AtomicU8) -> Result<(), ()> {
    while left > Duration::ZERO {
        if cancel.load(Ordering::SeqCst) != RUN {
            return Err(());
        }
        let step = left.min(MAX_TICK);
        std::thread::sleep(step);
        left -= step;
    }
    if cancel.load(Ordering::SeqCst) != RUN {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_tracks_the_cap() {
        assert_eq!(slice_of(100), MIN_SLICE);
        assert_eq!(slice_of(400 * 1024), 20 * 1024);
        assert_eq!(slice_of(64 * 1024 * 1024), CHUNK);

        let unlimited = Bucket::new(None);
        assert_eq!(read_slice(&[&unlimited]), CHUNK);
        let slow = Bucket::new(Some(400 * 1024));
        let fast = Bucket::new(Some(100_000_000));
        assert_eq!(read_slice(&[&slow, &fast]), 20 * 1024);
        assert_eq!(read_slice(&[&unlimited, &slow]), 20 * 1024);
    }

    #[test]
    fn unlimited_transfer_never_sleeps() {
        let unlimited = Bucket::new(None);
        let cancel = AtomicU8::new(RUN);
        let t = Instant::now();
        for _ in 0..64 {
            charge(&[&unlimited], CHUNK, &cancel).unwrap();
        }
        assert!(t.elapsed() < Duration::from_millis(200), "{:?}", t.elapsed());
    }

    #[test]
    fn cap_paces_sustained_bytes() {
        let bucket = Bucket::new(Some(400 * 1024));
        let cancel = AtomicU8::new(RUN);
        let slice = read_slice(&[&bucket]);
        assert_eq!(slice, 20 * 1024);
        let t = Instant::now();
        for _ in 0..5 {
            charge(&[&bucket], slice, &cancel).unwrap();
        }
        let expected = Duration::from_secs_f64(4.0 * slice as f64 / (400.0 * 1024.0));
        let elapsed = t.elapsed();
        assert!(elapsed >= expected.mul_f64(0.9), "{elapsed:?} vs {expected:?}");
        assert!(
            elapsed < expected.mul_f64(2.0) + Duration::from_millis(100),
            "{elapsed:?} vs {expected:?}"
        );
    }

    #[test]
    fn scopes_intersect_instead_of_adding_up() {
        let job = Bucket::new(Some(400 * 1024));
        let global = Bucket::new(Some(400 * 1024));
        let cancel = AtomicU8::new(RUN);
        let slice = read_slice(&[&job, &global]);
        charge(&[&job, &global], slice, &cancel).unwrap();
        let t = Instant::now();
        charge(&[&job, &global], slice, &cancel).unwrap();
        let elapsed = t.elapsed();
        assert!(elapsed >= Duration::from_millis(30), "not paced: {elapsed:?}");
        assert!(elapsed < Duration::from_millis(90), "waits summed: {elapsed:?}");
    }

    /// Step 3 puts a third scope in the slice. Three caps have to intersect the
    /// way two already do, or a per-adapter cap would silently halve the speed
    /// of a job that also has its own.
    #[test]
    fn three_scopes_intersect_instead_of_adding_up() {
        let job = Bucket::new(Some(400 * 1024));
        let global = Bucket::new(Some(400 * 1024));
        let net = Bucket::new(Some(400 * 1024));
        let cancel = AtomicU8::new(RUN);
        let slice = read_slice(&[&job, &global, &net]);
        assert_eq!(slice, 20 * 1024);
        charge(&[&job, &global, &net], slice, &cancel).unwrap();
        let t = Instant::now();
        charge(&[&job, &global, &net], slice, &cancel).unwrap();
        let elapsed = t.elapsed();
        assert!(elapsed >= Duration::from_millis(30), "not paced: {elapsed:?}");
        assert!(elapsed < Duration::from_millis(90), "waits summed: {elapsed:?}");
    }

    #[test]
    fn cancel_stops_a_sleeping_transfer() {
        let bucket = Bucket::new(Some(64 * 1024));
        let cancel = AtomicU8::new(RUN);
        let slice = read_slice(&[&bucket]);
        charge(&[&bucket], slice, &cancel).unwrap();
        cancel.store(1, Ordering::SeqCst);
        let t = Instant::now();
        assert!(charge(&[&bucket], slice, &cancel).is_err());
        assert!(t.elapsed() < Duration::from_millis(30), "{:?}", t.elapsed());
    }

    #[test]
    fn cap_swap_applies_to_the_next_read() {
        let bucket = Bucket::new(None);
        assert_eq!(read_slice(&[&bucket]), CHUNK);
        bucket.set_cap(Some(400 * 1024));
        assert_eq!(read_slice(&[&bucket]), 20 * 1024);
        bucket.set_cap(Some(0));
        assert_eq!(bucket.cap(), None);
        assert_eq!(read_slice(&[&bucket]), CHUNK);
    }
}
