//! A headless FVX run, checked: the CLI's "Randomized successfully!" isn't
//! trusted, so the settings and seed it really used are read from its log
//! and the output's SHA-1 is what setup compares before the lock.

use std::path::Path;
use std::process::Command;

use crate::{Installed, clean_dump, same_options, sha1_hex};

/// A checked randomizer output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Randomized {
    pub rom: Vec<u8>,
    /// What every Player's app compares before the lock.
    pub sha1: String,
}

/// Why a randomized ROM couldn't be made.
#[derive(Debug)]
pub enum RandomizeError {
    /// Only a clean FireRed, LeafGreen, Emerald, Ruby or Sapphire dump can be randomized.
    NotACleanDump,
    /// FVX couldn't be started, or exited with an error.
    Failed(String),
    /// FVX wrote no log, or a log without its settings or seed.
    NoLog,
    /// FVX used other settings than asked, such as after adjusting them for the ROM.
    OtherSettings,
    OtherSeed {
        used: i64,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for RandomizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RandomizeError::NotACleanDump => f.write_str(
                "only an unmodified FireRed, LeafGreen, Emerald, Ruby or Sapphire (USA 1.0) can be randomized",
            ),
            RandomizeError::Failed(why) => write!(f, "the randomizer failed: {why}"),
            RandomizeError::NoLog => f.write_str("the randomizer didn't say what it did, so its game wasn't used"),
            RandomizeError::OtherSettings => {
                f.write_str("the randomizer changed the Run's settings for this game, so its game wasn't used")
            }
            RandomizeError::OtherSeed { used } => {
                write!(
                    f,
                    "the randomizer used seed {used}, not the Run's, so its game wasn't used"
                )
            }
            RandomizeError::Io(e) => write!(f, "the randomized game couldn't be saved: {e}"),
        }
    }
}

impl From<std::io::Error> for RandomizeError {
    fn from(e: std::io::Error) -> Self {
        RandomizeError::Io(e)
    }
}

/// What an FVX log says it used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogHeader {
    pub settings: String,
    pub seed: i64,
}

/// The settings string and seed from an FVX log's header.
pub fn read_log(log: &str) -> Option<LogHeader> {
    let field = |label: &str| {
        log.lines()
            .find_map(|line| line.trim().strip_prefix(label).map(|v| v.trim().to_owned()))
    };
    Some(LogHeader {
        settings: field("Settings string:")?,
        seed: field("Random seed:")?.parse().ok()?,
    })
}

/// Randomizes `rom` with `settings` and `seed` in `work_dir`, through `fvx`.
pub fn randomize(
    fvx: &Installed,
    rom: &[u8],
    settings: &str,
    seed: i64,
    work_dir: &Path,
) -> Result<Randomized, RandomizeError> {
    if clean_dump(rom).is_none() {
        return Err(RandomizeError::NotACleanDump);
    }
    std::fs::create_dir_all(work_dir)?;
    let input = work_dir.join("in.gba");
    let output = work_dir.join("out.gba");
    for stale in [
        output.clone(),
        output.with_extension("gba.log"),
        output.with_extension("log"),
    ] {
        if stale.exists() {
            std::fs::remove_file(stale)?;
        }
    }
    std::fs::write(&input, rom)?;
    let run = Command::new(&fvx.java)
        .current_dir(&fvx.dir)
        .arg("-jar")
        .arg(&fvx.jar)
        .arg("cli")
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-S")
        .arg(settings)
        .arg("-z")
        .arg(seed.to_string())
        .arg("-l")
        .output()
        .map_err(|e| RandomizeError::Failed(e.to_string()))?;
    if !run.status.success() {
        let said = String::from_utf8_lossy(&run.stderr);
        return Err(RandomizeError::Failed(
            said.lines().last().unwrap_or("no reason given").to_owned(),
        ));
    }
    let log = [output.with_extension("gba.log"), output.with_extension("log")]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .ok_or(RandomizeError::NoLog)?;
    check(&log, settings, seed)?;
    let rom = std::fs::read(&output).map_err(|_| RandomizeError::Failed("it made no game".to_owned()))?;
    Ok(Randomized {
        sha1: sha1_hex(&rom),
        rom,
    })
}

/// Whether the log shows FVX used exactly `settings` and `seed`.
fn check(log: &str, settings: &str, seed: i64) -> Result<(), RandomizeError> {
    let used = read_log(log).ok_or(RandomizeError::NoLog)?;
    if !same_options(&used.settings, settings) {
        return Err(RandomizeError::OtherSettings);
    }
    if used.seed != seed {
        return Err(RandomizeError::OtherSeed { used: used.seed });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Recipe;

    fn log(settings: &str, seed: i64) -> String {
        format!("Randomizer version: FVX 1.6.3\nRandom seed: {seed}\nSettings string: {settings}\n\n--Starters--\n")
    }

    #[test]
    fn a_log_that_matches_passes() {
        let settings = Recipe::default().settings_string();
        assert!(check(&log(&settings, -42), &settings, -42).is_ok());
    }

    #[test]
    fn other_settings_or_seed_or_no_header_fail() {
        let settings = Recipe::default().settings_string();
        let other = Recipe {
            trainers: false,
            ..Recipe::default()
        }
        .settings_string();
        assert!(matches!(
            check(&log(&other, 1), &settings, 1),
            Err(RandomizeError::OtherSettings)
        ));
        assert!(matches!(
            check(&log(&settings, 2), &settings, 1),
            Err(RandomizeError::OtherSeed { used: 2 })
        ));
        assert!(matches!(
            check("Randomized successfully!", &settings, 1),
            Err(RandomizeError::NoLog)
        ));
    }

    #[test]
    fn only_a_clean_dump_is_run() {
        let fvx = Installed {
            dir: ".".into(),
            java: "never-run".into(),
            jar: "never-run".into(),
        };
        let work = tempfile::tempdir().unwrap();
        let got = randomize(&fvx, b"a romhack", &Recipe::default().settings_string(), 1, work.path());
        assert!(matches!(got, Err(RandomizeError::NotACleanDump)));
    }
}
