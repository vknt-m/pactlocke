//! `cargo dev`: developer-only tools, never shipped to Players.
//!
//!   cargo dev scan <rom> [--base <profile id>] [--out <file>]
//!       Drafts a Game Profile for a ROM against a bundled one (by default
//!       the one with the ROM's header game code), for a person to review.
//!
//!   cargo dev accept <profile id> <rom>
//!       Runs that profile's acceptance scripts against your own ROM: the
//!       real game, through the real game interface. Local only.
//!
//!   cargo dev rig-check <dir> <player>...
//!       Checks the headless apps' reports from a test rig run against each
//!       other: every script passed, every Shadow Game matched, one Run log.
//!
//! Profiles from `PACTLOCKE_DEV_PROFILES` (a folder) are loaded too.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use pactlocke_game::{GameProfile, ProfileStore, sha1_hex};

/// Each profile's acceptance scripts: the test targets, in `pactlocke-game`
/// and `pactlocke-run`, that drive its game.
const ACCEPTANCE: &[(&str, &str)] = &[("firered-usa-1.0", "firered_acceptance")];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("scan") => scan(&args[1..]),
        Some("accept") => accept(&args[1..]),
        Some("rig-check") => rig_check(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage:
  cargo dev scan <rom> [--base <profile id>] [--out <file>]
  cargo dev accept <profile id> <rom>
  cargo dev rig-check <dir> <player>...";

/// Reads `<dir>/<player>/report.json` for each Player and checks them together.
fn rig_check(args: &[String]) -> Result<(), String> {
    let (dir, players) = args.split_first().ok_or(USAGE)?;
    if players.is_empty() {
        return Err(USAGE.to_owned());
    }
    let reports: Vec<(String, Option<serde_json::Value>)> = players
        .iter()
        .map(|player| {
            let path = PathBuf::from(dir).join(player).join("report.json");
            let report = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok());
            (player.clone(), report)
        })
        .collect();
    let (summary, problems) = pactlocke_dev::rig::check(&reports);
    for line in summary {
        println!("  {line}");
    }
    if problems.is_empty() {
        println!("PASS");
        return Ok(());
    }
    for problem in &problems {
        println!("  FAIL {problem}");
    }
    Err(format!("{} problem(s)", problems.len()))
}

fn store() -> Result<ProfileStore, String> {
    ProfileStore::from_env().map_err(|e| format!("the Game Profiles don't load: {e}"))
}

fn read_rom(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

/// The value after `flag`, if it is given.
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn scan(args: &[String]) -> Result<(), String> {
    let rom_path = args.first().filter(|a| !a.starts_with("--")).ok_or(USAGE)?;
    let rom = read_rom(rom_path)?;
    let store = store()?;
    let base = match flag_value(args, "--base") {
        Some(id) => profile(&store, id)?,
        None => {
            let code = rom.get(0xAC..0xB0).unwrap_or_default();
            store
                .profiles()
                .find(|p| p.game_code.as_bytes() == code)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "no bundled profile has game code {:?}: name one with --base",
                        String::from_utf8_lossy(code)
                    )
                })?
        }
    };
    let draft = pactlocke_dev::scan(&rom, &base).draft();
    match flag_value(args, "--out") {
        Some(out) => {
            std::fs::write(out, &draft).map_err(|e| format!("{out}: {e}"))?;
            eprintln!("Draft written to {out}. Review every entry before it is trusted.");
        }
        None => print!("{draft}"),
    }
    Ok(())
}

fn profile(store: &ProfileStore, id: &str) -> Result<std::sync::Arc<GameProfile>, String> {
    store
        .by_id(id)
        .cloned()
        .ok_or_else(|| format!("no Game Profile {id:?}"))
}

fn accept(args: &[String]) -> Result<(), String> {
    let [id, rom_path] = args else {
        return Err(USAGE.to_owned());
    };
    let store = store()?;
    let profile = profile(&store, id)?;
    let rom = read_rom(rom_path)?;
    let sha1 = sha1_hex(&rom);
    if sha1 != profile.sha1 {
        return Err(format!(
            "{rom_path} isn't the ROM {id} is for: its SHA-1 is {sha1}, the profile's {}",
            profile.sha1
        ));
    }
    let (_, target) = ACCEPTANCE
        .iter()
        .find(|(profile_id, _)| profile_id == id)
        .ok_or_else(|| format!("{id} has no acceptance scripts yet"))?;
    let rom = std::path::absolute(rom_path).map_err(|e| format!("{rom_path}: {e}"))?;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    eprintln!("Running {id}'s acceptance scripts ({target}) against {}", rom.display());
    let status = Command::new(cargo)
        .current_dir(workspace)
        .env("PACTLOCKE_ROM", &rom)
        .args([
            "test",
            "--release",
            "-p",
            "pactlocke-game",
            "-p",
            "pactlocke-run",
            "--test",
            target,
        ])
        .status()
        .map_err(|e| format!("cargo couldn't be run: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{id}'s acceptance scripts failed"))
    }
}
