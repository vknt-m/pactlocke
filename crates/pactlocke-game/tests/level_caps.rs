//! Level caps on the profile-driven game. Default caps come from
//! the loaded ROM's trainer table, the current cap's stage from the badges
//! and defeat flags, and the cap sites are found by byte pattern.

mod support;

use pactlocke_game::{Game, LevelCap, ProfileGame, ProfileStore, level_cap_sites, sha1_hex};
use support::*;

fn game_over(image: MemoryImage) -> ProfileGame<MemoryImage> {
    let rom = image.rom.clone();
    let store = ProfileStore::from_sources([("test.toml", profile_toml("test-game", &sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap().clone();
    ProfileGame::new(profile, rom, image)
}

fn cap(name: &str, level: u8) -> LevelCap {
    LevelCap {
        name: name.into(),
        level,
    }
}

/// A trainer table entry: its party flags, then its party's levels at a
/// party pointer of its own.
fn put_trainer(rom: &mut [u8], id: u16, party_flags: u8, levels: &[u16]) {
    let entry = TRAINERS + u32::from(id) * 40;
    // Only trainers from 400 on are used here.
    let party = TRAINER_PARTIES + u32::from(id - 400) * 0x40;
    put(rom, entry, &[party_flags]);
    put(rom, entry + 32, &(levels.len() as u32).to_le_bytes());
    put(rom, entry + 36, &party.to_le_bytes());
    let size = if party_flags & 1 != 0 { 16 } else { 8 };
    for (i, level) in levels.iter().enumerate() {
        put(rom, party + i as u32 * size + 2, &level.to_le_bytes());
    }
}

fn rom_with_leaders() -> Vec<u8> {
    let mut rom = patchable_rom(b"BPRE");
    put_trainer(&mut rom, 414, 0, &[12, 14]);
    // Custom moves make each party entry twice the size.
    put_trainer(&mut rom, 415, 1, &[21, 18]);
    // The Champion's team depends on the starter: the strongest of the three.
    put_trainer(&mut rom, 438, 3, &[59, 63]);
    put_trainer(&mut rom, 439, 3, &[59, 61]);
    put_trainer(&mut rom, 440, 3, &[59, 62]);
    rom
}

#[test]
fn default_caps_are_each_stages_strongest_trainer_pokemon_in_the_loaded_rom() {
    let rom = rom_with_leaders();
    let store = ProfileStore::from_sources([("test.toml", profile_toml("test-game", &sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap();

    assert_eq!(
        profile.default_level_caps(&rom),
        [cap("Brock", 14), cap("Misty", 21), cap("Champion", 63)]
    );
}

#[test]
fn a_randomized_trainer_level_raises_its_default_cap() {
    let mut rom = rom_with_leaders();
    put_trainer(&mut rom, 414, 0, &[12, 19]);
    let store = ProfileStore::from_sources([("test.toml", profile_toml("test-game", &sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap();

    assert_eq!(profile.default_level_caps(&rom)[0], cap("Brock", 19));
}

#[test]
fn the_cap_stage_rises_with_each_badge_then_with_each_defeat_flag() {
    let mut game = game_over(overworld_image(rom_with_leaders()));
    assert_eq!(game.cap_stage(), 0, "no badges: Brock's cap");

    game.memory_mut().put8(SAVE_BLOCK_1 + 0xFE4, 0b0000_0001);
    assert_eq!(game.cap_stage(), 1, "one badge: Misty's cap");

    // The order badges are won in doesn't matter, only how many.
    game.memory_mut().put8(SAVE_BLOCK_1 + 0xFE4, 0b0000_0110);
    assert_eq!(game.cap_stage(), 2, "two badges: the Champion's cap");

    // FLAG_DEFEATED_CHAMP, 0x4BC, in save block 1's flags at 0xEE0.
    game.memory_mut()
        .put8(SAVE_BLOCK_1 + 0xEE0 + 0x4BC / 8, 1 << (0x4BC % 8));
    assert_eq!(game.cap_stage(), 3, "every cap passed");
}

#[test]
fn the_level_cap_sites_are_found_by_byte_pattern() {
    let mut rom = vec![0u8; 0x1000];
    // movs r1, #0x38 (MON_DATA_LEVEL), a call, then cmp r0, #100.
    rom[0x100..0x108].copy_from_slice(&[0x38, 0x21, 0x00, 0xF0, 0x00, 0xF8, 0x64, 0x28]);
    // The same with cmp r5, #100 a little further on.
    rom[0x200..0x20C].copy_from_slice(&[0x38, 0x21, 0x00, 0xF0, 0x00, 0xF8, 0x00, 0x06, 0x00, 0x0E, 0x64, 0x2D]);
    // A cmp #100 with no level read before it isn't a cap site.
    rom[0x300..0x302].copy_from_slice(&[0x64, 0x28]);
    // Nor is a level read followed by a compare with something else.
    rom[0x400..0x408].copy_from_slice(&[0x38, 0x21, 0x00, 0xF0, 0x00, 0xF8, 0x63, 0x28]);

    assert_eq!(level_cap_sites(&rom), [0x0800_0106, 0x0800_020A]);
}
