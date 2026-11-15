//! Where the app keeps its data: a `data` folder next to the app when that
//! can be written to (a portable install stays self-contained), otherwise the
//! user's data folder.

use std::path::PathBuf;

/// The folder of Run files, and whether it is the fallback one.
pub fn runs_dir() -> (PathBuf, bool) {
    if let Some(dir) = beside_app()
        && writable(&dir)
    {
        return (dir.join("runs"), false);
    }
    (user_data_dir().join("runs"), true)
}

/// The file that holds this install's network key, beside the Runs folder.
pub fn identity_file() -> PathBuf {
    let (runs, _) = runs_dir();
    runs.parent()
        .map_or_else(|| runs.join("identity.key"), |dir| dir.join("identity.key"))
}

/// The app's data folder, which holds the Runs folder: the randomizer and its games live here too.
pub fn data_root() -> PathBuf {
    let (runs, _) = runs_dir();
    runs.parent().map_or_else(|| runs.clone(), std::path::Path::to_path_buf)
}

fn beside_app() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("data"))
}

fn writable(dir: &std::path::Path) -> bool {
    let probe = dir.join(".write-test");
    let ok = std::fs::create_dir_all(dir).is_ok() && std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

fn user_data_dir() -> PathBuf {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if cfg!(windows) {
        if let Some(appdata) = env("APPDATA") {
            return appdata.join("Pactlocke");
        }
    } else if cfg!(target_os = "macos") {
        if let Some(home) = env("HOME") {
            return home.join("Library/Application Support/Pactlocke");
        }
    } else if let Some(data) = env("XDG_DATA_HOME") {
        return data.join("pactlocke");
    } else if let Some(home) = env("HOME") {
        return home.join(".local/share/pactlocke");
    }
    std::env::temp_dir().join("pactlocke")
}
