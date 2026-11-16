//! Seam 3, local only: the Heart and Soul 2.0.6 Game Profile
//! against the developer's own patched ROM, its tables read as the game
//! keeps them. Skipped unless `PACTLOCKE_ROMHACKS` names a folder holding
//! `H&S/` with the ROM (or `PACTLOCKE_ROM` names it). Never runs in CI.

use std::path::PathBuf;

use pactlocke_emu::CoreAdapter;
use pactlocke_game::{Game, ProfileGame, ProfileStore, RomIdentity};
use pactlocke_mgba::MgbaCore;

fn rom() -> Option<Vec<u8>> {
    if let Some(file) = std::env::var_os("PACTLOCKE_ROM") {
        return Some(std::fs::read(file).unwrap());
    }
    let dir = PathBuf::from(std::env::var_os("PACTLOCKE_ROMHACKS")?).join("H&S");
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "gba"))?;
    Some(std::fs::read(path).unwrap())
}

fn game() -> Option<ProfileGame<MgbaCore>> {
    let rom = rom()?;
    let store = ProfileStore::bundled().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("Heart and Soul matches its profile by SHA-1");
    };
    assert_eq!(profile.id, "heart-and-soul-2.0.6");
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    Some(ProfileGame::new(profile, rom, core))
}

macro_rules! game_or_skip {
    () => {
        match game() {
            Some(game) => game,
            None => {
                eprintln!("skipped: set PACTLOCKE_ROMHACKS to a folder with your patched Romhacks (H&S in it)");
                return;
            }
        }
    };
}

#[test]
fn each_cap_stage_defaults_to_its_leaders_strongest_pokemon() {
    let game = game_or_skip!();
    let store = ProfileStore::bundled().unwrap();
    let profile = store.by_id("heart-and-soul-2.0.6").unwrap();
    let caps = profile.default_level_caps(game.rom());
    let caps: Vec<(&str, u8)> = caps.iter().map(|c| (c.name.as_str(), c.level)).collect();
    // Noctowl, Scyther, Miltank, Gengar; the fifth to seventh leaders' teams
    // for that badge (Jasmine's Steelix 36, Scizor 36, then 38); Kingdra;
    // and Lance's Dragonite.
    assert_eq!(
        caps,
        [
            ("Falkner", 11),
            ("Bugsy", 16),
            ("Whitney", 21),
            ("Morty", 25),
            ("Fifth Gym Leader", 36),
            ("Sixth Gym Leader", 36),
            ("Seventh Gym Leader", 38),
            ("Clair", 45),
            ("Elite Four and Champion", 56),
        ]
    );
}

#[test]
fn species_types_and_evolutions_are_read_from_the_hacks_own_tables() {
    let mut game = game_or_skip!();
    assert_eq!(game.species_name(25), "PIKACHU");
    assert_eq!(game.species_name(155), "CYNDAQUIL");
    for (species, name) in [(25, "Electric"), (19, "Normal")] {
        let primary = game.primary_type(species);
        assert_eq!(game.type_name(primary), name);
    }
    let evolutions = game.evolutions();
    assert_eq!(evolutions[19], [20], "Rattata to Raticate");
    assert_eq!(evolutions[155], [156], "Cyndaquil to Quilava");
    for eeveelution in [134, 135, 136, 196, 197] {
        assert!(evolutions[133].contains(&eeveelution), "Eevee: {:?}", evolutions[133]);
    }
}
