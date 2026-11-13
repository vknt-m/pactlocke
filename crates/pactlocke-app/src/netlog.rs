//! A plain-text log of this app's connections in a Run with friends: who
//! joined, left, was refused or lost, and every reconnect, with the reason
//! the network gave. It is kept in `network.log` beside the Runs folder, so a
//! dropped connection can be explained after the fact.

use std::io::Write;
use std::path::PathBuf;

/// Where the log is kept.
pub fn path() -> PathBuf {
    crate::data_dir::identity_file().with_file_name("network.log")
}

/// Adds a line, stamped with the time. A log that can't be written is skipped:
/// it never stops a Run.
pub fn note(line: &str) {
    let stamp = chrono_like_now();
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(file, "{stamp} {line}");
    }
}

/// The time now as `HH:MM:SS.mmm` UTC, enough to line two PCs' logs up.
fn chrono_like_now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let day_ms = ms % 86_400_000;
    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        day_ms / 3_600_000,
        day_ms / 60_000 % 60,
        day_ms / 1000 % 60,
        day_ms % 1000
    )
}
