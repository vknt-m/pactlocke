//! Local only: the tested Romhacks and Untested play on
//! the developer's own ROMs. The tested-hack tests read every `.gba` under the
//! folder `PACTLOCKE_ROMHACKS` names (the six launch hacks, patched); the
//! Untested test reads FireRed from the folder `PACTLOCKE_ROMS` names. Each
//! skips without its folder. Never run in CI; the ROMs are only read.

use std::path::{Path, PathBuf};

use pactlocke_game::{Capability, ProfileStore, RomIdentity, Rule, SupportStatus, Unprofiled, sha1_hex};

fn gba_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            gba_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "gba") {
            found.push(path);
        }
    }
}

/// The tested Romhacks with a bundled Game Profile.
const PROFILED: [&str; 2] = ["Gaia", "Glazed"];

#[test]
fn each_patched_launch_romhack_is_named_and_profiled_or_waits_for_its_game_profile() {
    let Some(dir) = std::env::var_os("PACTLOCKE_ROMHACKS") else {
        eprintln!("skipped: set PACTLOCKE_ROMHACKS to a folder with your patched Romhacks");
        return;
    };
    let mut files = Vec::new();
    gba_files(Path::new(&dir), &mut files);
    let store = ProfileStore::bundled().unwrap();
    let mut named = Vec::new();
    for file in &files {
        let rom = std::fs::read(file).unwrap();
        let hack = store.tested_romhack(&sha1_hex(&rom)).expect("a tested Romhack").clone();
        let identity = store.identify(&rom);
        if PROFILED.contains(&hack.name.as_str()) {
            let RomIdentity::Profiled { profile, capabilities } = identity else {
                panic!("{} has its Game Profile: {identity:?}", hack.name);
            };
            eprintln!("{}: {}", file.display(), profile.name);
            assert_eq!(profile.status, SupportStatus::Supported, "{}", hack.name);
            for (rule, capability) in capabilities.iter() {
                assert_eq!(capability, &Capability::Enforceable, "{}: {rule:?}", hack.name);
            }
            named.push(hack.name);
            continue;
        }
        let RomIdentity::Unprofiled {
            verdict: Unprofiled::ProfileComing(hack),
            ..
        } = identity
        else {
            panic!("{} isn't a tested Romhack: {identity:?}", file.display());
        };
        eprintln!("{}: {} {} by {}", file.display(), hack.name, hack.version, hack.author);
        // The game it patches, from its own header.
        let base = store.by_id(&hack.base).unwrap();
        assert_eq!(&rom[0xAC..0xB0], base.game_code.as_bytes(), "{}", hack.name);
        named.push(hack.name);
    }
    named.sort_unstable();
    assert_eq!(
        named,
        ["Gaia", "Glazed", "Heart and Soul", "R.O.W.E.", "Radical Red", "Unbound"],
        "every tested hack, once each"
    );
}

#[test]
fn an_unknown_firered_hack_with_the_games_layout_plays_untested_on_fireds_profile() {
    let Some(dir) = std::env::var_os("PACTLOCKE_ROMS") else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own FireRed");
        return;
    };
    let mut hack = std::fs::read(PathBuf::from(dir).join("Pokemon - FireRed Version (USA).gba")).unwrap();
    // A hack that changed one byte of free ROM, and the code the level cap patches.
    hack[0x00F1_0000] = 0x00;
    hack[0x0002_1CFA] = 0x63;
    let store = ProfileStore::bundled().unwrap();
    let RomIdentity::Profiled { profile, capabilities } = store.identify(&hack) else {
        panic!("a FireRed-layout hack plays as Untested");
    };
    assert_eq!(profile.status, SupportStatus::Untested);
    assert_eq!(profile.sha1, sha1_hex(&hack));
    assert_eq!(profile.parent.as_deref(), Some("firered-usa-1.0"));
    eprintln!("{}", profile.name);
    for rule in [
        Rule::FirstEncounter,
        Rule::FaintIsDead,
        Rule::NoItems,
        Rule::MustNickname,
        Rule::SyncGates,
    ] {
        assert_eq!(capabilities.get(rule), &Capability::Enforceable, "{rule:?}");
    }
    for rule in [Rule::LevelCap, Rule::Gifts, Rule::Statics] {
        assert!(
            matches!(capabilities.get(rule), Capability::GreyedOut { .. }),
            "{rule:?}"
        );
    }
}
