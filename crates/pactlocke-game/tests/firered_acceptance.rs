//! Seam 3, local only: the FireRed (USA) 1.0 Game Profile
//! against the developer's own ROM. Skipped unless `PACTLOCKE_ROMS` names a
//! folder holding `Pokemon - FireRed Version (USA).gba`. Never runs in CI;
//! the ROM is only read, and the save stays in memory.

use std::path::PathBuf;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{
    Capability, Game, Method, ProfileGame, ProfileStore, RomIdentity, Rule, ScriptStatus, SourceKind, level_cap_sites,
};
use pactlocke_mgba::MgbaCore;

const FIRERED_FILE: &str = "Pokemon - FireRed Version (USA).gba";

/// The ROM `cargo dev accept` names in `PACTLOCKE_ROM`, or the one in the
/// folder `PACTLOCKE_ROMS` names.
fn firered_rom() -> Option<Vec<u8>> {
    let path = match std::env::var_os("PACTLOCKE_ROM") {
        Some(file) => PathBuf::from(file),
        None => PathBuf::from(std::env::var_os("PACTLOCKE_ROMS")?).join(FIRERED_FILE),
    };
    match std::fs::read(&path) {
        Ok(rom) => Some(rom),
        Err(e) => panic!("{} can't be read: {e}", path.display()),
    }
}

#[test]
fn firered_is_supported_and_every_profiled_rule_is_enforceable() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}, or PACTLOCKE_ROM to it");
        return;
    };
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, capabilities } = store.identify(&rom) else {
        panic!("FireRed (USA) 1.0 should match its profile by SHA-1");
    };
    assert_eq!(profile.id, "firered-usa-1.0");

    for rule in [
        Rule::FirstEncounter,
        Rule::DupesClause,
        Rule::FaintIsDead,
        Rule::Links,
        Rule::LevelCap,
        Rule::NoItems,
        Rule::MustNickname,
        Rule::SetMode,
        Rule::TypeClause,
    ] {
        assert_eq!(capabilities.get(rule), &Capability::Enforceable, "{rule:?}");
    }
}

#[test]
fn a_new_firered_game_reads_as_a_player_in_pallet_town() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}, or PACTLOCKE_ROM to it");
        return;
    };
    pactlocke_mgba::force_utc_time_zone();
    let store = ProfileStore::from_env().unwrap();
    let profile = store.identify(&rom);
    let RomIdentity::Profiled { profile, .. } = profile else {
        panic!("FireRed should be profiled");
    };
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    core.set_clock(1_767_225_600_000);
    let mut game = ProfileGame::new(profile, rom, core);

    // Before a game is started there is no party, battle or overworld.
    for _ in 0..60 {
        game.memory_mut().run_frame(Keys::NONE);
    }
    assert!(!game.at_safe_overworld_point());

    // Mash A through the intro, Oak's speech and the default names,
    // until the Player stands in their room.
    let mut frame = 0u32;
    while !(game.at_safe_overworld_point() && game.script().status == ScriptStatus::Idle) {
        assert!(frame < 9000, "the overworld was never reached");
        let keys = if frame % 16 < 3 { Keys::A } else { Keys::NONE };
        game.memory_mut().run_frame(keys);
        frame += 1;
    }
    for _ in 0..120 {
        game.memory_mut().run_frame(Keys::NONE);
    }

    assert_eq!(game.area().map(|a| a.name), Some("Pallet Town".into()));
    assert!(game.party().is_empty(), "no starter yet");
    assert!(game.pc().is_empty());
    assert_eq!(game.battle(), None);
    assert_eq!(game.badges().count(), 0);
    assert_ne!(game.trainer_id(), 0, "a new game draws a trainer ID");
    assert_eq!(
        game.option("battle_style").as_deref(),
        Some("shift"),
        "the game's default"
    );
    eprintln!("overworld after {frame} frames; trainer ID {:#010X}", game.trainer_id());
}

/// Plays a button script from power-on: `m` mashes, `h` holds, `w` waits,
/// `p` presses then waits.
fn play(game: &mut ProfileGame<MgbaCore>, script: &str) {
    for line in script.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let keys = |name: &str| match name {
            "A" => Keys::A,
            "B" => Keys::B,
            "U" => Keys::UP,
            "D" => Keys::DOWN,
            "L" => Keys::LEFT,
            "R" => Keys::RIGHT,
            other => panic!("no key {other}"),
        };
        let core = game.memory_mut();
        match words[..] {
            ["m", key, n] => (0..n.parse::<u32>().unwrap())
                .for_each(|i| core.run_frame(if i % 16 < 3 { keys(key) } else { Keys::NONE })),
            ["h", key, n] => (0..n.parse::<u32>().unwrap()).for_each(|_| core.run_frame(keys(key))),
            ["w", n] => (0..n.parse::<u32>().unwrap()).for_each(|_| core.run_frame(Keys::NONE)),
            ["p", key, wait] => {
                (0..4).for_each(|_| core.run_frame(keys(key)));
                (0..wait.parse::<u32>().unwrap()).for_each(|_| core.run_frame(Keys::NONE));
            }
            _ => panic!("bad script line {line:?}"),
        }
    }
}

/// From power-on to choosing a starter in Oak's lab.
const FR_STARTER: &str = "m A 5100\nm B 300\nh R 64\nh U 64\nh L 24\nw 120\nh D 16\nh L 16\nh D 96\nh L 80\nh D 24\nw 120\n\
h R 64\nh U 96\nh R 32\nh U 48\nm A 1500\nm B 1500\nh R 20\nw 20\np A 90\np A 90\np A 90\np A 120\np A 120\np A 120\nm B 1500";

#[test]
fn the_starter_reads_back_decrypted_from_the_real_party() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}, or PACTLOCKE_ROM to it");
        return;
    };
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("FireRed should be profiled");
    };
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    let mut game = ProfileGame::new(profile, rom, core);

    play(&mut game, FR_STARTER);

    let party = game.party();
    assert_eq!(party.len(), 1, "one starter: {party:?}");
    let starter = &party[0];
    let expected_name = match starter.species {
        1 => "BULBASAUR",
        4 => "CHARMANDER",
        7 => "SQUIRTLE",
        other => panic!("species {other} is not a FireRed starter"),
    };
    assert_eq!(starter.nickname, expected_name);
    assert_eq!(starter.met_section, 0x58, "met in Pallet Town");
    assert!(!starter.is_egg);
    let stats = starter.stats.unwrap();
    assert_eq!(stats.level, 5);
    assert!(stats.hp > 0 && stats.hp <= stats.max_hp);
    assert_eq!(starter.ot_id, game.trainer_id(), "the Player is its original trainer");
}

#[test]
fn evolutions_read_from_the_real_rom_match_the_games_families() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}, or PACTLOCKE_ROM to it");
        return;
    };
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("FireRed should be profiled");
    };
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    let mut game = ProfileGame::new(profile, rom, core);

    let evolutions = game.evolutions();
    assert_eq!(evolutions.len(), 412, "NUM_SPECIES, counting species 0");
    assert_eq!(evolutions[1], [2], "Bulbasaur to Ivysaur");
    assert_eq!(evolutions[2], [3]);
    assert!(evolutions[3].is_empty(), "Venusaur is the last");
    assert_eq!(evolutions[16], [17], "Pidgey to Pidgeotto");
    assert_eq!(evolutions[19], [20], "Rattata to Raticate");
    assert_eq!(evolutions[172], [25], "Pichu to Pikachu");
    let mut eevee = evolutions[133].clone();
    eevee.sort_unstable();
    assert_eq!(eevee, [134, 135, 136, 196, 197], "Eevee's five");
    assert_eq!(evolutions[290].len(), 2, "Nincada to Ninjask, and Shedinja");
    let evolving = evolutions.iter().filter(|into| !into.is_empty()).count();
    assert!(evolving < 200, "fewer than half evolve in the real game: {evolving}");
}

#[test]
fn every_catalogued_gift_and_static_is_its_script_instruction_in_the_real_rom() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}, or PACTLOCKE_ROM to it");
        return;
    };
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("FireRed should be profiled");
    };
    assert!(profile.sources.iter().any(|s| s.kind == SourceKind::Starter));
    for source in &profile.sources {
        let opcode = source.kind.opcode();
        for &at in &source.at {
            let byte = rom[(at - 0x0800_0000) as usize];
            assert_eq!(byte, opcode, "{} at {at:#010X}", source.key);
        }
    }
}

#[test]
fn default_level_caps_are_each_leaders_strongest_pokemon_in_the_real_rom() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}, or PACTLOCKE_ROM to it");
        return;
    };
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("FireRed should be profiled");
    };
    let caps: Vec<(String, u8)> = profile
        .default_level_caps(&rom)
        .into_iter()
        .map(|cap| (cap.name, cap.level))
        .collect();
    // Each leader's ace in FireRed: Onix 14, Starmie 21, Raichu 24,
    // Vileplume 29, Weezing 43, Alakazam 43, Arcanine 47, Rhydon 50, then
    // the League's Lapras 54, Machamp 56, Gengar 58, Dragonite 60 and the
    // Champion's starter 63.
    let expected = [
        ("Brock", 14),
        ("Misty", 21),
        ("Lt. Surge", 24),
        ("Erika", 29),
        ("Koga", 43),
        ("Sabrina", 43),
        ("Blaine", 47),
        ("Giovanni", 50),
        ("Lorelei", 54),
        ("Bruno", 56),
        ("Agatha", 58),
        ("Lance", 60),
        ("Champion", 63),
    ]
    .map(|(name, level)| (name.to_owned(), level));
    assert_eq!(caps, expected);
}

#[test]
fn every_profiled_level_cap_site_is_found_by_byte_pattern_in_the_real_rom() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}, or PACTLOCKE_ROM to it");
        return;
    };
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("FireRed should be profiled");
    };
    let Some(Method::Patch { sites }) = &profile.rules.level_cap else {
        panic!("FireRed has level cap sites");
    };
    let found = level_cap_sites(&rom);
    for site in sites {
        assert!(
            found.contains(&site.at),
            "{:#010X} isn't a pattern match: {found:x?}",
            site.at
        );
    }
}
