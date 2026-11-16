//! Seam 3, local only: the R.O.W.E. 2.1.9.1 Game Profile
//! against the developer's own patched ROM, its tables read as the game
//! keeps them. Skipped unless `PACTLOCKE_ROMHACKS` names a folder holding
//! `Emerald ROWE/` with the ROM (or `PACTLOCKE_ROM` names it). Never runs in CI.

use std::path::PathBuf;

use pactlocke_emu::CoreAdapter;
use pactlocke_game::{Capability, Game, ProfileGame, ProfileStore, RomIdentity, Rule};
use pactlocke_mgba::MgbaCore;

const ID: &str = "rowe-2.1.9.1";

fn rom() -> Option<Vec<u8>> {
    if let Some(file) = std::env::var_os("PACTLOCKE_ROM") {
        return Some(std::fs::read(file).unwrap());
    }
    let dir = PathBuf::from(std::env::var_os("PACTLOCKE_ROMHACKS")?).join("Emerald ROWE");
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "gba"))?;
    Some(std::fs::read(path).unwrap())
}

macro_rules! rom_or_skip {
    () => {
        match rom() {
            Some(rom) => rom,
            None => {
                eprintln!(
                    "skipped: set PACTLOCKE_ROMHACKS to a folder with your patched Romhacks (Emerald ROWE in it)"
                );
                return;
            }
        }
    };
}

fn game(rom: Vec<u8>) -> ProfileGame<MgbaCore> {
    let store = ProfileStore::bundled().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("R.O.W.E. matches its profile by SHA-1");
    };
    assert_eq!(profile.id, ID);
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    ProfileGame::new(profile, rom, core)
}

#[test]
fn every_rule_is_pactlockes_but_set_mode_which_is_the_games_own_option() {
    let rom = rom_or_skip!();
    let store = ProfileStore::bundled().unwrap();
    let RomIdentity::Profiled { capabilities, .. } = store.identify(&rom) else {
        panic!("profiled");
    };
    for (rule, capability) in capabilities.iter() {
        match rule {
            Rule::SetMode => assert!(matches!(capability, Capability::NativeOption(_)), "{capability:?}"),
            _ => assert_eq!(capability, &Capability::Enforceable, "{rule:?}"),
        }
    }
    assert!(capabilities.own_randomizer(), "Shared Seed through its own randomizer");
}

#[test]
fn each_cap_stage_defaults_to_its_leaders_strongest_pokemon_measured_on_normal() {
    let rom = rom_or_skip!();
    let store = ProfileStore::bundled().unwrap();
    let caps = store.by_id(ID).unwrap().default_level_caps(&rom);
    let caps: Vec<(&str, u8)> = caps.iter().map(|c| (c.name.as_str(), c.level)).collect();
    assert_eq!(
        caps,
        [
            ("Roxanne", 15),
            ("Brawly", 20),
            ("Wattson", 25),
            ("Flannery", 28),
            ("Norman", 31),
            ("Winona", 35),
            ("Tate & Liza", 38),
            ("Juan", 41),
            ("Sidney", 68),
            ("Phoebe", 69),
            ("Glacia", 69),
            ("Drake", 68),
            ("Champion", 74),
        ]
    );
}

#[test]
fn species_types_and_evolutions_are_read_from_the_hacks_own_tables() {
    let mut game = game(rom_or_skip!());
    assert_eq!(game.species_name(25), "Pikachu");
    assert_eq!(game.species_name(1882), "Delphox", "its last species");
    for (species, name) in [(25, "Electric"), (19, "Normal")] {
        let primary = game.primary_type(species);
        assert_eq!(game.type_name(primary), name);
    }
    let evolutions = game.evolutions();
    assert_eq!(evolutions[19], [20], "Rattata to Raticate");
    for eeveelution in [134, 135, 136, 196, 197, 470, 471, 700] {
        assert!(evolutions[133].contains(&eeveelution), "Eevee: {:?}", evolutions[133]);
    }
}
