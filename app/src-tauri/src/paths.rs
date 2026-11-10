use std::path::{Path, PathBuf};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub data_dir: String,
    pub db_path: String,
    pub logs_dir: String,
    pub downloads_dir: String,
}

/// One place decides where data lives (plan §2.3, A25).
/// Order: --data-dir=<path>, then SUCR_DATA_DIR, then the platform default.
pub fn resolve() -> AppPaths {
    let args: Vec<String> = std::env::args().collect();
    let flag = args
        .iter()
        .find_map(|a| a.strip_prefix("--data-dir="))
        .map(str::to_string);
    let override_dir = flag
        .or_else(|| std::env::var("SUCR_DATA_DIR").ok())
        .filter(|s| !s.is_empty());

    let data_dir = match override_dir {
        Some(p) => std::path::absolute(Path::new(&p)).unwrap_or_else(|_| PathBuf::from(p)),
        None => default_dir(),
    };

    // A data-dir override also moves downloads inside it, so test runs and
    // smoke gates never touch the real Downloads folder.
    let downloads_dir = if flag_is_set() {
        data_dir.join("downloads")
    } else {
        default_downloads()
    };

    AppPaths {
        db_path: data_dir.join("sucr.db").to_string_lossy().into_owned(),
        logs_dir: data_dir.join("logs").to_string_lossy().into_owned(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        downloads_dir: downloads_dir.to_string_lossy().into_owned(),
    }
}

/// The data directory alone, for the native-messaging relay: it must read the
/// same discovery file the app wrote and must never open the database to learn
/// where it is.
pub fn resolve_data_dir() -> PathBuf {
    PathBuf::from(resolve().data_dir)
}

fn flag_is_set() -> bool {
    std::env::args().any(|a| a.starts_with("--data-dir="))
        || std::env::var("SUCR_DATA_DIR").map(|v| !v.is_empty()).unwrap_or(false)
}

fn default_downloads() -> PathBuf {
    home().join("Downloads")
}

/// One quick-folder chip, already resolved to a directory that exists.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPreset {
    pub preset: &'static str,
    pub path: String,
}

/// Which standard folders this machine actually has, on the drive the add bar is
/// aimed at. `base` is the folder the dialog targets; each name is looked for
/// next to that folder first, then at the root of its drive, then in the
/// profile. The first real directory wins and a name with none is left out of
/// the answer entirely — a chip must never offer a path that only exists once
/// a download creates it. Every path the answer contains is named for its
/// preset, so a configured default that is merely the drive holding the folder
/// reports the folder, not the drive.
pub fn folder_presets(base: &str) -> Vec<FolderPreset> {
    folder_presets_in(base, &home())
}

/// The scan with its profile supplied, so a test can hand it a folder that
/// genuinely has nothing in it.
fn folder_presets_in(base: &str, profile: &Path) -> Vec<FolderPreset> {
    let trimmed = base.trim();
    let base_dir = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
    let mut out = Vec::with_capacity(3);
    for (id, name) in [
        ("downloads", "Downloads"),
        ("desktop", "Desktop"),
        ("documents", "Documents"),
    ] {
        // Downloads may be satisfied by the configured default itself, but only
        // when that default is genuinely the folder (see `preset_candidates`);
        // the other two are destinations in their own right and start at the sibling.
        let found =
            preset_candidates(name, base_dir.as_deref(), profile, id == "downloads").into_iter()
                .find(|p| p.is_dir());
        if let Some(path) = found {
            out.push(FolderPreset {
                preset: id,
                path: path.to_string_lossy().into_owned(),
            });
        }
    }
    out
}

fn preset_candidates(
    name: &str,
    base: Option<&Path>,
    profile: &Path,
    prefer_base: bool,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(4);
    if let Some(dir) = base {
        // The configured default answers for a preset only when it *is* that
        // folder. `D:\Downloads` is the Downloads folder; `D:\` is a drive root
        // that merely holds one, and offering the root would hide the real
        // folder behind it — which is what the user reported.
        if prefer_base && leaf_is(dir, name) {
            push_unique(&mut out, dir.to_path_buf());
        }
        // A drive root has no sibling to drop back into: `D:\`'s parent is the
        // bare `D:`, and joining onto that yields the drive-relative `D:Desktop`
        // rather than a real path. The drive-root scan below covers it.
        if let Some(parent) = dir.parent().filter(|p| !is_bare_drive(p)) {
            push_unique(&mut out, parent.join(name));
        }
        if let Some(root) = drive_root(dir) {
            push_unique(&mut out, root.join(name));
        }
    }
    push_unique(&mut out, profile.join(name));
    out
}

fn leaf_is(path: &Path, name: &str) -> bool {
    path.file_name()
        .map(|leaf| leaf.to_string_lossy().eq_ignore_ascii_case(name))
        .unwrap_or(false)
}

/// What the shell knows about a folder the user is aiming at, before it offers to
/// make one. `path` is the absolute form the create call would use, so the notice
/// and the action can never disagree about which folder they are about.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderState {
    pub path: String,
    pub exists: bool,
    pub is_dir: bool,
}

/// `None` for an empty field: that means "use the default download folder", which
/// the shell owns and already creates, so there is nothing to offer.
pub fn folder_state(raw: &str) -> Option<FolderState> {
    let path = absolutise(raw)?;
    let meta = std::fs::metadata(&path);
    Some(FolderState {
        exists: meta.is_ok(),
        is_dir: meta.map(|m| m.is_dir()).unwrap_or(false),
        path: path.to_string_lossy().into_owned(),
    })
}

/// The outcome of asking for a folder that is not there yet. `error` is the
/// honest answer when the disk refuses — a file already holds that name, the
/// path is read-only, the drive is missing — and the UI shows it instead of
/// pretending the folder exists.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolder {
    pub path: String,
    pub created: bool,
    pub already_present: bool,
    pub error: Option<String>,
}

pub fn create_folder(raw: &str) -> CreateFolder {
    let trimmed = raw.trim();
    let fail = |msg: String| CreateFolder {
        path: trimmed.to_string(),
        created: false,
        already_present: false,
        error: Some(msg),
    };

    if trimmed.is_empty() {
        return fail("No folder path was given.".to_string());
    }
    // `D:` is not the D drive's root: it is relative to whatever the process's
    // current directory happens to be *on that drive*, so creating it would make
    // a folder somewhere nobody typed. `D:\` is the root, and it already exists.
    if is_bare_drive(Path::new(trimmed)) {
        return fail("That names a whole drive, not a folder.".to_string());
    }
    let Some(path) = absolutise(trimmed) else {
        return fail("No folder path was given.".to_string());
    };
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => {
            return CreateFolder {
                path: path.to_string_lossy().into_owned(),
                created: false,
                already_present: true,
                error: None,
            }
        }
        Ok(_) => return fail("A file already has that name.".to_string()),
        Err(_) => {}
    }
    match std::fs::create_dir_all(&path) {
        Ok(()) => CreateFolder {
            path: path.to_string_lossy().into_owned(),
            created: true,
            already_present: false,
            error: None,
        },
        Err(e) => fail(format!("Could not create it: {e}")),
    }
}

/// Normalises and anchors a typed path so a relative `Downloads` cannot mean two
/// different folders depending on the process's current directory.
fn absolutise(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(
        std::path::absolute(Path::new(trimmed))
            .unwrap_or_else(|_| PathBuf::from(trimmed)),
    )
}

fn push_unique(list: &mut Vec<PathBuf>, path: PathBuf) {
    if !list.contains(&path) {
        list.push(path);
    }
}

fn is_bare_drive(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let mut chars = s.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(c), Some(':'), None) if c.is_ascii_alphabetic()
    )
}

/// `D:\Users\x` sits on `D:\`. Nothing for a relative path or a UNC share,
/// where the sibling and profile scans already reach what exists.
fn drive_root(path: &Path) -> Option<PathBuf> {
    let s = path.to_string_lossy();
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), Some(':')) if c.is_ascii_alphabetic() => {
            Some(PathBuf::from(format!("{c}:\\")))
        }
        _ => None,
    }
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_dir() -> PathBuf {
    if cfg!(windows) {
        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join("AppData").join("Roaming"));
        appdata.join("Sucr")
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support").join("Sucr")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local").join("share"))
            .join("Sucr")
    }
}

#[cfg(test)]
mod folder_state_tests {
    use super::{create_folder, folder_state};
    use std::path::Path;

    /// An empty field means "the default download folder", which the shell owns
    /// and already creates — so there is nothing for the UI to offer.
    #[test]
    fn an_empty_field_is_not_a_folder_to_offer() {
        assert!(folder_state("").is_none());
        assert!(folder_state("   ").is_none());
    }

    /// A typed path is anchored before it is judged, so a relative `Downloads`
    /// cannot mean one folder to the notice and another to the transfer.
    #[test]
    fn a_relative_path_is_anchored_before_it_is_answered() {
        let state = folder_state("Downloads").expect("a path was given");
        assert!(
            Path::new(&state.path).is_absolute(),
            "{} was not anchored",
            state.path
        );
    }

    #[test]
    fn a_missing_folder_is_reported_missing_then_created_on_request() {
        let target = std::env::temp_dir()
            .join(format!("sucr-mkdir-{}", std::process::id()))
            .join("nested")
            .join("deep");
        let raw = target.to_str().unwrap();
        let _ = std::fs::remove_dir_all(target.parent().unwrap());

        let before = folder_state(raw).expect("state");
        assert!(!before.exists, "{before:?}");
        assert_eq!(before.path, raw);

        let made = create_folder(raw);
        assert!(made.created && made.error.is_none(), "{made:?}");
        assert!(target.is_dir());

        let after = folder_state(raw).expect("state");
        assert!(after.exists && after.is_dir, "{after:?}");

        // Asking twice is not an error, and it must not claim to have made it again.
        let again = create_folder(raw);
        assert!(!again.created && again.already_present && again.error.is_none(), "{again:?}");

        let _ = std::fs::remove_dir_all(target.parent().unwrap().parent().unwrap());
    }

    /// A file already wearing the folder's name is the one case where "create it"
    /// must refuse rather than rename, overwrite, or quietly succeed.
    #[test]
    fn a_file_holding_the_name_is_refused() {
        let root = std::env::temp_dir().join(format!("sucr-mkdir-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("Downloads");
        std::fs::write(&file, b"x").unwrap();

        let state = folder_state(file.to_str().unwrap()).expect("state");
        assert!(state.exists && !state.is_dir, "{state:?}");

        let made = create_folder(file.to_str().unwrap());
        assert!(!made.created && !made.already_present, "{made:?}");
        assert!(made.error.is_some(), "the refusal must be said, not swallowed");
        assert!(file.is_file(), "the file must still be there afterwards");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The bare `D:` form is refused outright: it resolves against whatever the
    /// process's directory happens to be on that drive, so a folder would appear
    /// somewhere nobody typed. A drive root that does exist is simply already
    /// there, which is the honest answer rather than an error.
    #[cfg(windows)]
    #[test]
    fn a_whole_drive_is_not_a_folder() {
        let bare = create_folder("D:");
        assert!(!bare.created, "{bare:?}");
        assert!(
            bare.error.unwrap().contains("drive"),
            "the refusal should name the drive"
        );

        let root = create_folder(r"D:\");
        assert!(
            !root.created && root.already_present && root.error.is_none(),
            "{root:?}"
        );
    }
}

#[cfg(test)]
mod preset_tests {
    use super::{folder_presets_in, preset_candidates};
    use std::path::Path;

    #[cfg(windows)]
    fn cands(base: &str, name: &str, prefer_base: bool) -> Vec<String> {
        preset_candidates(
            name,
            (!base.is_empty()).then(|| Path::new(base)),
            Path::new(r"C:\Users\ben"),
            prefer_base,
        )
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
    }

    /// The reported bug: `D:\` has no `Desktop` or `Documents` next to it, so the
    /// chip must not resolve to a drive path at all — it falls through to the
    /// profile, which is where those folders really are on this machine.
    #[cfg(windows)]
    #[test]
    fn drive_root_offers_no_sibling() {
        assert_eq!(
            cands(r"D:\", "Desktop", false),
            [r"D:\Desktop", r"C:\Users\ben\Desktop"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn nested_base_tries_its_own_neighbourhood_first() {
        assert_eq!(
            cands(r"D:\Games\Downloads", "Documents", false),
            [
                r"D:\Games\Documents",
                r"D:\Documents",
                r"C:\Users\ben\Documents"
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn downloads_prefers_the_configured_folder_only_when_it_is_the_folder() {
        assert_eq!(
            cands(r"D:\Downloads", "Downloads", true),
            [r"D:\Downloads", r"C:\Users\ben\Downloads"]
        );
        // The reported bug: the default is a drive root, so it must not answer for
        // Downloads — the folder sitting on that drive has to.
        assert_eq!(
            cands(r"D:\", "Downloads", true),
            [r"D:\Downloads", r"C:\Users\ben\Downloads"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn no_base_leaves_only_the_profile() {
        assert_eq!(cands("", "Desktop", false), [r"C:\Users\ben\Desktop"]);
    }

    /// The reported case, on a real filesystem: a name with no directory behind it
    /// gets no chip. `home()` cannot be pointed somewhere else, so the scan takes
    /// its profile as an argument. With no base there is no sibling or drive-root
    /// scan, so the answer is provably the profile alone — and a base would let the
    /// scan reach the host's real drive root, which on some machines does hold a
    /// Desktop.
    #[test]
    fn a_folder_that_exists_nowhere_is_not_offered() {
        let root = std::env::temp_dir().join(format!("sucr-presets-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile = root.join("profile");
        std::fs::create_dir_all(&profile).unwrap();

        assert!(
            folder_presets_in("", &profile).is_empty(),
            "an empty profile has no folder to offer"
        );

        std::fs::create_dir_all(profile.join("Documents")).unwrap();
        let found = folder_presets_in("", &profile);
        assert_eq!(
            found.iter().map(|p| p.preset).collect::<Vec<_>>(),
            ["documents"],
            "{found:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_on_the_base_drive_beats_the_profile() {
        let root = std::env::temp_dir().join(format!("sucr-presets-sibling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("profile\\Desktop")).unwrap();
        std::fs::create_dir_all(root.join("Desktop")).unwrap();
        std::fs::create_dir_all(root.join("downloads")).unwrap();

        let found =
            folder_presets_in(root.join("downloads").to_str().unwrap(), &root.join("profile"));
        let desktop = found
            .iter()
            .find(|p| p.preset == "desktop")
            .expect("desktop resolved");
        assert_eq!(desktop.path, root.join("Desktop").to_string_lossy());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The reported case on a real filesystem: the configured default is a folder
    /// that *holds* the standard folders instead of being one of them. Every chip
    /// has to name a directory inside it, never the default itself.
    #[test]
    fn a_default_that_only_holds_the_folders_reports_the_folders() {
        let root = std::env::temp_dir().join(format!("sucr-presets-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("profile")).unwrap();
        for name in ["Here", "Downloads", "Desktop", "Documents"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }

        let found = folder_presets_in(root.join("Here").to_str().unwrap(), &root.join("profile"));
        let paths: Vec<&str> = found.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                root.join("Downloads").to_str().unwrap(),
                root.join("Desktop").to_str().unwrap(),
                root.join("Documents").to_str().unwrap(),
            ],
            "{found:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn posix_sibling_dedupes_against_the_profile() {
        let got = preset_candidates(
            "Desktop",
            Some(Path::new("/home/ben/Downloads")),
            Path::new("/home/ben"),
            false,
        );
        assert_eq!(got, [Path::new("/home/ben/Desktop").to_path_buf()]);
    }
}
