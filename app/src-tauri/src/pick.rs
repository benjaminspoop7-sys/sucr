//! The shell calls this app makes: the folder picker and the default-verb open.
//!
//! The picker is `IFileOpenDialog` with `FOS_PICKFOLDERS` — the same "Select
//! Folder" dialog Explorer shows. The `windows` crate is already in the graph
//! (tao, wry and tauri-runtime-wry all use it, with `Win32_UI_Shell` and
//! `Win32_System_Com` already enabled for the whole set), so this is an `use`,
//! not a dependency — and it generates the COM vtables, which is the one part
//! worth not hand-rolling: a wrong method slot is undefined behaviour, not a
//! compile error.

/// Open a path with whatever the user has associated with it.
///
/// `ShellExecuteW` rather than `cmd /c start "" <path>`: cmd re-parses the
/// command line it is handed, and Rust only quotes arguments containing spaces,
/// so a `&` in a stored filename would split the line into two commands and run
/// the second one. `&` is legal in a Windows filename, so `sanitize_filename`
/// keeps it, and filenames arrive from URLs.
///
/// `Err` carries the `SE_ERR_*` code (2 = not found, 31 = no association).
#[cfg(windows)]
pub fn open(path: &std::path::Path) -> Result<(), i32> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let verb = wide(std::ffi::OsStr::new("open"));
    let file = wide(path.as_os_str());
    let blank = wide(std::ffi::OsStr::new(""));
    let ret = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(blank.as_ptr()),
            PCWSTR(blank.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    // Values up to 32 are the SE_ERR_ codes; 33 and above is a recycled HINSTANCE.
    // windows 0.62 hands handles back as pointers, hence the cast.
    let code = ret.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(code as i32)
    }
}

/// Nothing to open a file with off Windows.
#[cfg(not(windows))]
pub fn open(_path: &std::path::Path) -> Result<(), i32> {
    Err(-1)
}

/// Absolute path the user picked, or None when they cancelled.
#[cfg(windows)]
pub fn folder() -> Option<String> {
    use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    // The dialog needs an apartment. S_FALSE means this thread already had one
    // and we still owe the balancing CoUninitialize; RPC_E_CHANGED_MODE means
    // someone else owns this thread's model, and uninitialising here would pull
    // the rug out from under them. Any other failure: no dialog.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr != S_OK && hr != S_FALSE && hr != RPC_E_CHANGED_MODE {
        return None;
    }
    let ours = hr == S_OK || hr == S_FALSE;

    let chosen = unsafe { select() };
    if ours {
        unsafe { CoUninitialize() };
    }
    chosen.filter(|p| !p.trim().is_empty())
}

/// Cancel is an error HRESULT, so every `?` below means "the user backed out".
#[cfg(windows)]
unsafe fn select() -> Option<String> {
    use std::ffi::c_void;
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    let dialog: IFileOpenDialog =
        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
    // FORCEFILESYSTEM is what keeps the answer a path: a library node or "This
    // PC" can otherwise be selected and has no file-system name to give back.
    dialog
        .SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
        .ok()?;
    dialog.Show(None).ok()?;
    let item = dialog.GetResult().ok()?;
    let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let text = path.display().to_string();
    CoTaskMemFree(Some(path.0 as *const c_void));
    Some(text)
}

/// No shell dialog off Windows. The folder field stays editable and the
/// Downloads/Desktop/Documents presets still resolve, so only Browse is inert.
#[cfg(not(windows))]
pub fn folder() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    /// Hand-driven, like `network::tests::dump_this_machine`: this opens the
    /// real shell dialog and blocks until it is closed, so no gate can run it.
    /// `cargo test --release pick -- --ignored` — pick any folder and check the
    /// printed path is the one you chose, absolute, not just its name.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn the_folder_dialog_answers_with_an_absolute_path() {
        let picked = super::folder();
        println!("picked: {picked:?}");
        if let Some(path) = &picked {
            assert!(std::path::Path::new(path).is_absolute(), "{path} is not absolute");
        }
    }

    /// Also hand-driven: a file the shell cannot find is answered with an
    /// `SE_ERR_*` code instead of the success we would report to the UI. Out of
    /// the gate because on a machine with no association for `.txt` the shell may
    /// answer by asking the user, and a modal in `cargo test` is a hung test.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn a_missing_path_is_answered_with_an_error_code() {
        let code = super::open(std::path::Path::new("Q:\\sucr-not-here.txt")).err();
        assert!(code.is_some(), "the shell claimed to open a file on an unmounted drive");
    }

    /// Hand-driven, like the picker above: this really opens a file. The name
    /// carries `&`, which is legal in a filename and survives sanitize_filename,
    /// so it is exactly the string that used to split a `cmd /c start` line in
    /// two. Confirm Notepad (or whatever owns .txt) shows this file's contents.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn a_name_the_shell_would_split_still_opens() {
        let dir = std::env::temp_dir();
        let path = dir.join("sucr & proof.txt");
        std::fs::write(&path, "opened by the default handler, not by cmd\n").unwrap();
        let result = super::open(&path);
        println!("open {:?} -> {result:?}", path);
        assert!(result.is_ok(), "{result:?}");
    }
}
