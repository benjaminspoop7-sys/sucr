//! Keeping the machine awake for the transfers it is currently carrying.
//!
//! `SetThreadExecutionState` is the only Windows answer, and it is per-thread:
//! the request belongs to whichever thread made it and ends when that thread
//! does. So this module is a process-level wrapper over the one idea — hold the
//! request while a download is live, drop it when none is — and every call comes
//! from a thread that outlives the transfers it speaks for: the adapter watcher
//! (five seconds, always) and the IPC thread that starts and finishes a job.
//!
//! `ES_SYSTEM_REQUIRED` alone, not `ES_DISPLAY_REQUIRED`: a running transfer
//! needs the machine, not a lit panel. That is the same scope Plexo holds with
//! `powerSaveBlocker.start("prevent-app-suspension")`, and it is why there is no
//! setting here — an overnight download must survive a dial nobody looked at.
//! What it never claims is the right to wake the machine from sleep; starting a
//! transfer on a timer is a different, unwritten capability.

/// Requests (or releases) the system-awake state and reports what the OS says
/// the resulting state is, so a caller that wants proof can compare it.
///
/// Returns `None` where there is no such API.
#[cfg(windows)]
pub fn set(on: bool) -> Option<u32> {
    use windows::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };
    let flags = if on {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    // The call cannot fail in any way the caller could act on: it returns the
    // new state, and `0` is documented only for an invalid combination of flags
    // — which the two constants above cannot produce.
    let got = unsafe { SetThreadExecutionState(flags) };
    Some(got.0)
}

#[cfg(not(windows))]
pub fn set(_on: bool) -> Option<u32> {
    None
}
