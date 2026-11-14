//! Seam 3, local only: a Hoenn Game Profile (Emerald, Ruby and Sapphire)
//! against the developer's own ROM. Each game's test target runs these on its
//! ROM; they skip without one. Never run in CI; the ROM is only read, and the
//! save stays in memory.

use std::path::PathBuf;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{
    Capability, Game, GameProfile, Method, ProfileGame, ProfileStore, RomIdentity, Rule, ScriptStatus, SourceKind,
    level_cap_sites,
};
use pactlocke_mgba::MgbaCore;

use super::GAME;

/// Which Hoenn game a test target runs these on.
pub struct Hoenn {
    /// The ROM's file name in the folder `PACTLOCKE_ROMS` names.
    pub file: &'static str,
    /// Its Game Profile.
    pub id: &'static str,
    /// Each cap stage's name and the level of that trainer's strongest
    /// Pokémon, from the game's own teams.
    pub caps: [(&'static str, u8); 13],
}

/// `special ChooseStarter`: the game gives the starter from code once the
/// choice is made, so the catalogue lists this instruction.
const SPECIAL: u8 = 0x25;

/// The ROM `cargo dev accept` names in `PACTLOCKE_ROM`, or the one in the
/// folder `PACTLOCKE_ROMS` names.
fn rom() -> Option<Vec<u8>> {
    let path = match std::env::var_os("PACTLOCKE_ROM") {
        Some(file) => PathBuf::from(file),
        None => PathBuf::from(std::env::var_os("PACTLOCKE_ROMS")?).join(GAME.file),
    };
    match std::fs::read(&path) {
        Ok(rom) => Some(rom),
        Err(e) => panic!("{} can't be read: {e}", path.display()),
    }
}

fn profiled(rom: &[u8]) -> std::sync::Arc<GameProfile> {
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(rom) else {
        panic!("{} should match its profile by SHA-1", GAME.id);
    };
    assert_eq!(profile.id, GAME.id);
    profile
}

macro_rules! rom_or_skip {
    () => {
        match rom() {
            Some(rom) => rom,
            None => {
                eprintln!(
                    "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
                    GAME.file
                );
                return;
            }
        }
    };
}

#[test]
fn it_is_profiled_and_every_profiled_rule_is_enforceable() {
    let rom = rom_or_skip!();
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, capabilities } = store.identify(&rom) else {
        panic!("{} should match its profile by SHA-1", GAME.id);
    };
    assert_eq!(profile.id, GAME.id);
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
        Rule::SyncGates,
    ] {
        assert_eq!(capabilities.get(rule), &Capability::Enforceable, "{rule:?}");
    }
}

#[test]
fn a_new_game_reads_as_a_player_in_the_moving_truck() {
    let rom = rom_or_skip!();
    pactlocke_mgba::force_utc_time_zone();
    let profile = profiled(&rom);
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    core.set_clock(1_767_225_600_000);
    let mut game = ProfileGame::new(profile, rom, core);

    for _ in 0..60 {
        game.memory_mut().run_frame(Keys::NONE);
    }
    assert!(!game.at_safe_overworld_point());

    // Mash A through the intro and Birch's speech (with
    // B now and then so a naming screen is left), until the Player
    // stands in the truck.
    let mut frame = 0u32;
    while !(game.at_safe_overworld_point() && game.script().status == ScriptStatus::Idle) {
        assert!(frame < 30_000, "the overworld was never reached");
        let keys = match frame % 64 {
            0..3 => Keys::A,
            32..35 if frame > 6000 => Keys::START,
            _ => Keys::NONE,
        };
        game.memory_mut().run_frame(keys);
        frame += 1;
    }
    for _ in 0..120 {
        game.memory_mut().run_frame(Keys::NONE);
    }

    assert_eq!(game.area().map(|a| a.name), Some("Inside of Truck".into()));
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

#[test]
fn evolutions_read_from_the_real_rom_match_the_games_families() {
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    let mut game = ProfileGame::new(profile, rom, core);

    let evolutions = game.evolutions();
    assert_eq!(evolutions.len(), 412, "NUM_SPECIES, counting species 0");
    assert_eq!(evolutions[1], [2], "Bulbasaur to Ivysaur");
    let [treecko, torchic, mudkip] = [277usize, 280, 283];
    assert_eq!(evolutions[treecko], [278], "Treecko to Grovyle");
    assert_eq!(evolutions[torchic], [281], "Torchic to Combusken");
    assert_eq!(evolutions[mudkip], [284], "Mudkip to Marshtomp");
    // Wurmple to Silcoon or Cascoon, by personality.
    let mut wurmple = evolutions[290].clone();
    wurmple.sort_unstable();
    assert_eq!(wurmple, [291, 293], "Wurmple's two");
    // Nincada to Ninjask, and Shedinja.
    assert_eq!(evolutions[301].len(), 2);
    let evolving = evolutions.iter().filter(|into| !into.is_empty()).count();
    assert!(evolving < 200, "fewer than half evolve in the real game: {evolving}");
}

#[test]
fn every_catalogued_gift_and_static_is_its_script_instruction_in_the_real_rom() {
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let starters: Vec<_> = profile
        .sources
        .iter()
        .filter(|s| s.kind == SourceKind::Starter)
        .collect();
    assert_eq!(starters.len(), 1);
    for source in &profile.sources {
        let opcode = match source.kind {
            SourceKind::Starter => SPECIAL,
            kind => kind.opcode(),
        };
        for &at in &source.at {
            let byte = rom[(at - 0x0800_0000) as usize];
            assert_eq!(byte, opcode, "{} at {at:#010X}", source.key);
            // The special waits for the choice (its `waitstate` follows).
            if source.kind == SourceKind::Starter {
                assert_eq!(
                    rom[(at - 0x0800_0000) as usize + 3],
                    0x27,
                    "waitstate after the special"
                );
                continue;
            }
            // And the instruction's species is the one the source is named
            // for (Birch's Johto starters aside), read from the ROM.
            let species = u16::from_le_bytes([
                rom[(at - 0x0800_0000) as usize + 1],
                rom[(at - 0x0800_0000) as usize + 2],
            ]);
            let name = species_name(&profile, &rom, species);
            if source.key != "johto_starter" {
                assert!(
                    source.name.to_uppercase().contains(&name),
                    "{} at {at:#010X} is {name}",
                    source.key
                );
            }
        }
    }
}

/// A species' name, as the ROM's species names table has it.
fn species_name(profile: &GameProfile, rom: &[u8], species: u16) -> String {
    let length = profile.layout.species_name_length;
    let at = (profile.symbols.species_names + u32::from(species) * length - 0x0800_0000) as usize;
    pactlocke_game::decode_text(&rom[at..at + length as usize])
}

#[test]
fn the_games_repeated_underwater_sections_are_told_apart_and_split_caves_are_one_area() {
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let name = |section: u16| profile.area(section).map(|a| a.name);
    // Each Underwater section by the route above it, as pret's maps place them.
    assert_eq!(name(0x32).as_deref(), Some("Underwater (124)"));
    assert_eq!(name(0x33).as_deref(), Some("Underwater (126)"));
    assert_eq!(name(0x34).as_deref(), Some("Underwater (127)"));
    assert_eq!(name(0x35).as_deref(), Some("Underwater (128)"));
    let underwater: Vec<String> = (0..0x100)
        .filter_map(&name)
        .filter(|n| n.starts_with("Underwater"))
        .collect();
    let mut distinct = underwater.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        underwater.len(),
        "no two Underwater sections share a name: {underwater:?}"
    );
    for (second, first, place) in [
        (0x40, 0x3F, "Meteor Falls"),
        (0x4B, 0x4A, "Fiery Path"),
        (0x4D, 0x4C, "Jagged Pass"),
    ] {
        let (a, b) = (profile.area(second).unwrap(), profile.area(first).unwrap());
        assert_eq!(
            (a.id, a.name.as_str()),
            (b.id, place),
            "{place}'s second section is the same Area"
        );
    }
    assert_eq!(name(0x39).as_deref(), Some("Safari Zone"));
}

#[test]
fn default_level_caps_are_each_leaders_strongest_pokemon_in_the_real_rom() {
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let caps: Vec<(String, u8)> = profile
        .default_level_caps(&rom)
        .into_iter()
        .map(|cap| (cap.name, cap.level))
        .collect();
    let expected: Vec<(String, u8)> = GAME
        .caps
        .iter()
        .map(|(name, level)| ((*name).to_owned(), *level))
        .collect();
    assert_eq!(caps, expected);
}

#[test]
fn every_profiled_level_cap_site_is_found_by_byte_pattern_in_the_real_rom() {
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let Some(Method::Patch { sites }) = &profile.rules.level_cap else {
        panic!("{} has level cap sites", GAME.id);
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

#[test]
fn every_sync_gate_is_its_trainers_first_battle_in_the_real_rom() {
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let gates = profile.sync_gates.as_ref().expect("gates");
    let stage_trainers: Vec<u16> = profile.level_caps.iter().flat_map(|s| s.trainers.clone()).collect();
    assert_eq!(
        gates.gates.len(),
        13,
        "eight leaders, four of the Elite Four, the Champion"
    );
    for (gate, stage) in gates.gates.iter().zip(&profile.level_caps) {
        assert_eq!(gate.name, stage.name, "gates and cap stages in the same order");
        for site in &gate.sites {
            let at = (site.at - 0x0800_0000) as usize;
            let bytes = &rom[at..at + 5];
            assert_eq!(bytes[0], 0x5C, "{}: a trainerbattle", gate.name);
            assert_ne!(bytes[1], 7, "{}: not a rematch (type 7)", gate.name);
            let trainer = u16::from_le_bytes([bytes[2], bytes[3]]);
            assert!(stage.trainers.contains(&trainer), "{}: trainer {trainer}", gate.name);
            assert!(stage_trainers.contains(&trainer));
        }
    }
}
