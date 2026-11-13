//! The app's log: one plain-text file, `pactlocke.log`, beside the Runs
//! folder. Every crate writes to it through the `log` facade, and iroh's own
//! warnings come through too. A panic is written there before the app
//! closes. The file from the previous start is kept as `pactlocke.old.log`.
//!
//! Pactlocke's own lines are written from Info up, other crates' from Warn
//! up. `PACTLOCKE_LOG=debug` writes Pactlocke's Debug lines and other
//! crates' Info lines too.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Where the log is kept.
pub fn path() -> PathBuf {
    crate::data_dir::identity_file().with_file_name("pactlocke.log")
}

/// Starts the log for this run of the app. A log that can't be opened is
/// skipped: it never stops the app.
pub fn init() {
    let path = path();
    let _ = std::fs::rename(&path, path.with_file_name("pactlocke.old.log"));
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let verbose = std::env::var("PACTLOCKE_LOG").is_ok_and(|v| v.eq_ignore_ascii_case("debug"));
    let logger = FileLogger {
        file: Mutex::new(file),
        ours: if verbose { Level::Debug } else { Level::Info },
        others: if verbose { Level::Info } else { Level::Warn },
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(if verbose { LevelFilter::Debug } else { LevelFilter::Info });
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!(target: "pactlocke::panic", "{info}");
        log::logger().flush();
        previous(info);
    }));
    log::info!("Pactlocke {} started", env!("CARGO_PKG_VERSION"));
}

struct FileLogger {
    file: Mutex<File>,
    /// The lowest level written for Pactlocke's own crates, and for the rest.
    ours: Level,
    others: Level,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let floor = if metadata.target().starts_with("pactlocke") {
            self.ours
        } else {
            self.others
        };
        metadata.level() <= floor
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}: {}\n",
            now_utc(),
            record.level(),
            record.target(),
            record.args()
        );
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

/// The time now as `YYYY-MM-DD HH:MM:SS.mmmZ`, to line two PCs' logs up.
fn now_utc() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis()) as i64;
    let (days, day_ms) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}Z",
        day_ms / 3_600_000,
        day_ms / 60_000 % 60,
        day_ms / 1000 % 60,
        day_ms % 1000
    )
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn dates_come_out_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_405), (2025, 11, 13));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
