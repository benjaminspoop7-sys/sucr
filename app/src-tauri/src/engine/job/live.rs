//! Per-job telemetry behind the Sucker Map. The coordinator owns the state;
//! a subscriber gets one full snapshot on watch and 4 Hz deltas after that.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const UNATTRIBUTED: &str = "n0";
pub const EMIT_MS: u64 = 250;
const RECENT_ATTEMPTS: usize = 64;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveBlock {
    pub idx: usize,
    pub offset: u64,
    pub len: u64,
    pub bytes_done: u64,
    pub state: String,
    pub network_id: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveAttempt {
    pub id: i64,
    pub network_id: String,
    pub network_label: String,
    pub block_idx: usize,
    pub range_start: u64,
    pub range_end: u64,
    pub bytes: u64,
    pub state: String,
    pub started_at: i64,
    pub duration_ms: i64,
    pub reason: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveState {
    pub block_size: u64,
    pub total: u64,
    pub blocks: Vec<LiveBlock>,
    pub attempts: Vec<LiveAttempt>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveDelta {
    pub block_size: u64,
    pub total: u64,
    pub blocks: Vec<LiveBlock>,
    pub attempts: Vec<LiveAttempt>,
}

#[derive(Default)]
struct Dirty {
    blocks: BTreeSet<usize>,
    attempts: BTreeSet<i64>,
}

#[derive(Default)]
pub struct Live {
    watchers: AtomicUsize,
    rev: AtomicU64,
    last_emit: AtomicU64,
    /// The width the job is running this second. The ramp moves it, so it is the
    /// only number that can tell a reader "8–32" from "actually 11 right now".
    streams: AtomicUsize,
    state: Mutex<LiveState>,
    dirty: Mutex<Dirty>,
}

impl Live {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }

    pub fn set_streams(&self, width: usize) {
        self.streams.store(width, Ordering::Relaxed);
    }

    pub fn streams(&self) -> usize {
        self.streams.load(Ordering::Relaxed)
    }

    pub fn watch(&self) -> usize {
        self.watchers.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn unwatch(&self) {
        let _ = self.watchers.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            Some(n.saturating_sub(1))
        });
    }

    pub fn snapshot(&self) -> Option<LiveState> {
        self.state.lock().ok().map(|s| s.clone())
    }

    pub fn set_plan(&self, block_size: u64, total: u64, blocks: Vec<LiveBlock>) {
        let (Ok(mut st), Ok(mut dirty)) = (self.state.lock(), self.dirty.lock()) else {
            return;
        };
        st.block_size = block_size;
        st.total = total;
        st.blocks = blocks;
        dirty.blocks = (0..st.blocks.len()).collect();
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    /// Rewrites every block in place; only blocks whose bytes or state moved
    /// are queued for the next delta.
    pub fn update_blocks(&self, mut edit: impl FnMut(&mut LiveBlock)) {
        let (Ok(mut st), Ok(mut dirty)) = (self.state.lock(), self.dirty.lock()) else {
            return;
        };
        let mut changed = false;
        for block in st.blocks.iter_mut() {
            let before = (block.bytes_done, block.state.clone());
            edit(block);
            if before != (block.bytes_done, block.state.clone()) {
                dirty.blocks.insert(block.idx);
                changed = true;
            }
        }
        if changed {
            self.rev.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn set_state(&self, idx: usize, state: &str) {
        self.update_blocks(|b| {
            if b.idx == idx {
                b.state = state.to_string();
            }
        });
    }

    /// Marks the block active and stamps the device taking it, so the stripe on
    /// the map and the row in Connections agree from the first byte.
    pub fn claim_block(&self, idx: usize, network_id: &str) {
        let (Ok(mut st), Ok(mut dirty)) = (self.state.lock(), self.dirty.lock()) else {
            return;
        };
        let Some(block) = st.blocks.iter_mut().find(|b| b.idx == idx) else {
            return;
        };
        block.state = "active".to_string();
        block.network_id = network_id.to_string();
        dirty.blocks.insert(idx);
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    /// Upsert: an attempt is pushed when its block is claimed (state `active`)
    /// and pushed again when it ends, so the same row id updates in place.
    pub fn push_attempt(&self, attempt: LiveAttempt) {
        let (Ok(mut st), Ok(mut dirty)) = (self.state.lock(), self.dirty.lock()) else {
            return;
        };
        let attempt_id = attempt.id;
        if let Some(existing) = st.attempts.iter_mut().find(|a| a.id == attempt_id) {
            *existing = attempt;
        } else {
            st.attempts.push(attempt);
            if st.attempts.len() > RECENT_ATTEMPTS {
                // Never drop a stream that is still running; its row would go
                // silent in the pane until it finished.
                if let Some(pos) = st.attempts.iter().position(|a| a.state != "active") {
                    let dropped = st.attempts.remove(pos);
                    dirty.attempts.remove(&dropped.id);
                }
            }
        }
        dirty.attempts.insert(attempt_id);
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    /// Live bytes for one attempt, addressed by its own row id.
    ///
    /// A block can be held by two attempts at once when an idle worker takes its
    /// tail, so a block index no longer identifies a single row: it would land
    /// the parent's bytes on the child's row and leave the parent frozen.
    pub fn update_active_attempt(&self, attempt_id: i64, bytes: u64) {
        let (Ok(mut st), Ok(mut dirty)) = (self.state.lock(), self.dirty.lock()) else {
            return;
        };
        let Some(attempt) = st
            .attempts
            .iter_mut()
            .find(|a| a.id == attempt_id && a.state == "active")
        else {
            return;
        };
        if attempt.bytes != bytes {
            attempt.bytes = bytes;
            dirty.attempts.insert(attempt.id);
            self.rev.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Changed blocks and attempts since the last tick, at most 4 Hz.
    /// `force` bypasses the cadence for the final update of a job.
    pub fn take_delta(&self, force: bool) -> Option<LiveDelta> {
        if self.watchers.load(Ordering::Relaxed) == 0 {
            return None;
        }
        let now = now_ms();
        let last = self.last_emit.load(Ordering::Relaxed);
        if !force && now.saturating_sub(last) < EMIT_MS {
            return None;
        }
        let (Ok(st), Ok(mut dirty)) = (self.state.lock(), self.dirty.lock()) else {
            return None;
        };
        if dirty.blocks.is_empty() && dirty.attempts.is_empty() {
            return None;
        }
        let blocks = dirty
            .blocks
            .iter()
            .filter_map(|i| st.blocks.get(*i).cloned())
            .collect();
        let attempts = st
            .attempts
            .iter()
            .filter(|a| dirty.attempts.contains(&a.id))
            .cloned()
            .collect();
        dirty.blocks.clear();
        dirty.attempts.clear();
        self.last_emit.store(now, Ordering::Relaxed);
        Some(LiveDelta {
            block_size: st.block_size,
            total: st.total,
            blocks,
            attempts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(id: i64, block_idx: usize, state: &str, bytes: u64) -> LiveAttempt {
        LiveAttempt {
            id,
            network_id: "n1".to_string(),
            network_label: "Wi-Fi".to_string(),
            block_idx,
            range_start: 0,
            range_end: 1023,
            bytes,
            state: state.to_string(),
            started_at: 1,
            duration_ms: 0,
            reason: None,
            detail: None,
        }
    }

    fn block(idx: usize, network_id: &str) -> LiveBlock {
        LiveBlock {
            idx,
            offset: idx as u64 * 1024,
            len: 1024,
            bytes_done: 0,
            state: "pending".to_string(),
            network_id: network_id.to_string(),
        }
    }

    #[test]
    fn an_active_attempt_updates_in_place() {
        let live = Live::new();
        live.watch();
        live.push_attempt(attempt(7, 0, "active", 0));
        live.update_active_attempt(7, 4096);

        let delta = live.take_delta(true).expect("claim delta");
        assert_eq!(delta.attempts.len(), 1);
        assert_eq!(delta.attempts[0].state, "active");
        assert_eq!(delta.attempts[0].bytes, 4096);

        live.push_attempt(attempt(7, 0, "done", 8192));
        let delta = live.take_delta(true).expect("finish delta");
        assert_eq!(delta.attempts.len(), 1, "the same id is upserted, not duplicated");
        assert_eq!(delta.attempts[0].state, "done");
        assert_eq!(delta.attempts[0].bytes, 8192);
    }

    #[test]
    fn two_attempts_sharing_a_block_each_report_their_own_bytes() {
        let live = Live::new();
        live.watch();
        live.push_attempt(attempt(3, 0, "active", 0));
        live.push_attempt(attempt(4, 0, "active", 0));

        live.update_active_attempt(3, 1000);
        live.update_active_attempt(4, 250);

        let delta = live.take_delta(true).expect("both rows changed");
        let bytes = |id: i64| {
            delta
                .attempts
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.bytes)
                .unwrap_or(u64::MAX)
        };
        assert_eq!(bytes(3), 1000, "the parent keeps its own front-of-block count");
        assert_eq!(bytes(4), 250, "the stolen tail reports only its own slice");
    }

    #[test]
    fn trimming_the_ring_keeps_running_streams() {
        let live = Live::new();
        live.watch();
        live.push_attempt(attempt(1, 0, "active", 0));
        live.push_attempt(attempt(2, 1, "active", 0));
        for id in 3..(RECENT_ATTEMPTS as i64 + 3) {
            live.push_attempt(attempt(id, 2, "done", 1024));
        }

        let st = live.snapshot().expect("snapshot");
        assert_eq!(st.attempts.len(), RECENT_ATTEMPTS);
        let live_ids: Vec<i64> = st
            .attempts
            .iter()
            .filter(|a| a.state == "active")
            .map(|a| a.id)
            .collect();
        assert_eq!(live_ids, vec![1, 2], "in-flight attempts outlive the ring");
    }

    #[test]
    fn a_claimed_block_carries_the_device_that_took_it() {
        let live = Live::new();
        live.watch();
        live.set_plan(
            1024,
            2048,
            vec![
                block(0, UNATTRIBUTED),
                block(1, UNATTRIBUTED),
            ],
        );
        let _ = live.take_delta(true);

        live.claim_block(1, "n2");
        let delta = live.take_delta(true).expect("claim delta");
        assert_eq!(delta.blocks.len(), 1, "only the block that moved is sent");
        assert_eq!(delta.blocks[0].idx, 1);
        assert_eq!(delta.blocks[0].state, "active");
        assert_eq!(
            delta.blocks[0].network_id, "n2",
            "the stripe is the device that took the block, not whatever was enumerated last"
        );
    }

    #[test]
    fn idle_updates_do_not_queue_a_delta() {
        let live = Live::new();
        live.watch();
        live.update_active_attempt(0, 4096);
        assert!(live.take_delta(true).is_none());
    }
}
