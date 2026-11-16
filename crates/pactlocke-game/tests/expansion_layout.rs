//! Seam 2: games built on pokeemerald-expansion.
//! Heart and Soul keeps the encrypted Pokémon but packs other fields beside
//! the species, keeps Perish Song in each battler's own volatiles, and lists
//! each species' evolutions behind a pointer in its species info. R.O.W.E.
//! keeps Pokémon in a plain, unencrypted form of its own.

mod support;

use pactlocke_game::{Change, Game, ProfileGame, ProfileStore, sha1_hex};
use support::*;

/// The test profile with Heart and Soul's changes.
fn hns_text(sha1: &str) -> String {
    profile_toml("hns-like", sha1)
        .replace("encrypted = true", "encrypted = true\nspecies_mask = 0x07FF")
        // Perish Song is a bit of each battler's volatiles, its timer two bits of another byte.
        .replace("battle_mon_size = 0x58", "battle_mon_size = 0x88")
        .replace(
            "battler_statuses3 = 0x02023DFC",
            &format!("battler_statuses3 = {:#010X}", BATTLE_MONS + 0x5C),
        )
        .replace(
            "disable_structs = 0x02023E0C",
            &format!("disable_structs = {BATTLE_MONS:#010X}"),
        )
        .replace(
            "perish_song_status3 = 0x20",
            "perish_song_status3 = 0x80\nbattler_status_stride = 0x88",
        )
        .replace("disable_struct_size = 0x1C", "disable_struct_size = 0x88")
        .replace("perish_timer = 0x0F", "perish_timer = 0x6E\nperish_timer_mask = 0x0C")
}

/// The test profile with Heart and Soul's species info: 0x10C bytes a
/// species, each pointing at its own list of evolutions.
fn hns_species_text(sha1: &str) -> String {
    hns_text(sha1)
        .replace(
            "species_info_size = 28",
            "species_info_size = 0x10C\nevolutions_pointer = 0xA4",
        )
        .replace("species_count = 412", "species_count = 30")
        .replace("evolution_size = 8", "evolution_size = 12")
}

const BATTLE_MONS: u32 = 0x0202_3BE4;
const IN_BATTLE: u32 = 0x0300_3529;
/// Where the test image points at save block 3, and where that block is.
const SAVE_BLOCK_3_POINTER: u32 = 0x0300_0178;
const SAVE_BLOCK_3: u32 = 0x0203_0000;

/// The test profile with an option of Heart and Soul's challenge settings,
/// which it keeps in save block 3.
fn hns_options_text(sha1: &str) -> String {
    hns_text(sha1)
        .replace(
            "message_buffer = 0x02021D18",
            &format!("message_buffer = 0x02021D18\nsave_block_3 = {{ pointer = {SAVE_BLOCK_3_POINTER:#010X} }}"),
        )
        .replace(
            "[options.battle_style]",
            "[options.player_items]\nname = \"PLAYER ITEMS\"\nmenu = \"DIFFICULTY > PLAYER ITEMS\"\nsave_block = 3\noffset = 0x17\nmask = 0x20\nvalues = { allowed = 0x00, banned = 0x20 }\nwatched = true\n\n[options.battle_style]",
        )
}

#[test]
fn catches_are_blocked_by_setting_the_games_own_no_catching_flag_and_allowed_by_clearing_it() {
    let text = |sha1: &str| {
        hns_text(sha1).replace(
            "[rules.catch_block]\nmethod = \"patch\"\nsites = [{ at = 0x08000100, expect = \"10 B5 00 24\", write = \"01 20 70 47\" }]",
            "[rules.catch_block]\nmethod = \"set_flag\"\nflag = 0x96B",
        )
    };
    let mut game = game_from(text);
    // Flag 0x96B is bit 3 of the event flags' byte 0x12D (the test profile's at 0xEE0).
    let byte = SAVE_BLOCK_1 + 0xEE0 + 0x12D;
    game.memory_mut().put8(byte, 0x41);

    let changes = game.block_catches(true).unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(game.memory_mut().get(byte, 1), [0x49]);

    let changes = game.block_catches(false).unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(game.memory_mut().get(byte, 1), [0x41]);
}

/// The test profile with R.O.W.E.'s Pokémon: 0x4C bytes in the party and the
/// first 0x34 of them in the PC, unencrypted, the species in the low 12 bits
/// at +0x1C, the egg bit at +0x27 and the met location at +0x2A.
fn rowe_text(sha1: &str) -> String {
    profile_toml("rowe-like", sha1)
        .replace("party_slot_size = 100", "party_slot_size = 0x4C")
        .replace("box_slot_size = 80", "box_slot_size = 0x34")
        .replace(
            "encrypted = true",
            "encrypted = false\nspecies_mask = 0x0FFF\nplain = { species = 0x1C, met_location = 0x2A }",
        )
        .replace("nickname_length = 10", "nickname_length = 12")
        .replace("flags = 0x13\negg_mask = 0x04", "flags = 0x27\negg_mask = 0x80")
        .replace(
            "level = 0x54\nhp = 0x56\nmax_hp = 0x58",
            "level = 0x3C\nhp = 0x3E\nmax_hp = 0x40",
        )
}

/// A Pokémon in R.O.W.E.'s party form; its first 0x34 bytes are its PC form.
fn rowe_mon(pid: u32, species: u16, egg: bool, hp: u16) -> [u8; 0x4C] {
    let mut mon = [0u8; 0x4C];
    mon[0..4].copy_from_slice(&pid.to_le_bytes());
    mon[4..8].copy_from_slice(&0xC5AD_1CF6u32.to_le_bytes());
    // "Pikachu", then what the game leaves past the terminator.
    mon[8..20].copy_from_slice(&[0xCA, 0xDD, 0xDF, 0xD5, 0xD7, 0xDC, 0xE9, 0xFF, 0xC7, 0xB1, 0xCA, 0xD5]);
    mon[0x14..0x18].copy_from_slice(&[0x61, 0xF6, 0x01, 0x00]); // a move and EXP
    // The species' 12 bits, with a form and a move above them.
    mon[0x1C..0x20].copy_from_slice(&(0x70C4_0000u32 | u32::from(species)).to_le_bytes());
    mon[0x27] = if egg { 0xD8 } else { 0x48 };
    mon[0x2A] = if egg { 0xFD } else { 0x10 };
    mon[0x3C] = 5;
    mon[0x3E..0x40].copy_from_slice(&hp.to_le_bytes());
    mon[0x40..0x42].copy_from_slice(&18u16.to_le_bytes());
    mon
}

#[test]
fn a_plain_pokemon_is_read_from_its_own_fields_in_the_party_and_the_pc() {
    let mut game = game_from(rowe_text);
    let image = game.memory_mut();
    image.put8(PARTY_COUNT, 2);
    image.put(PARTY, &rowe_mon(0x5D1E_6E0F, 25, false, 7));
    image.put(PARTY + 0x4C, &rowe_mon(0xE522_B2D8, 25, true, 0));
    // Box 1, slot 2 (the PC starts 4 bytes into its storage).
    image.put(PC_STORAGE + 4 + 0x34, &rowe_mon(0x0102_0304, 19, false, 0)[..0x34]);

    let party = game.party();
    assert_eq!(party.len(), 2);
    assert_eq!(party[0].pid, 0x5D1E_6E0F);
    assert_eq!(party[0].ot_id, 0xC5AD_1CF6);
    assert_eq!(party[0].species, 25);
    assert_eq!(party[0].nickname, "Pikachu");
    assert_eq!(party[0].met_section, 0x10);
    assert!(!party[0].is_egg);
    assert_eq!(party[0].stats.map(|s| (s.level, s.hp, s.max_hp)), Some((5, 7, 18)));
    assert!(party[1].is_egg);

    let pc = game.pc();
    assert_eq!(pc.len(), 1, "an empty slot is all zeros: {pc:?}");
    assert_eq!((pc[0].box_number, pc[0].slot, pc[0].pokemon.species), (1, 2, 19));
}

#[test]
fn a_dead_plain_pokemon_moves_to_the_first_empty_slot_of_the_dead_box_in_its_pc_form() {
    let mut game = game_from(rowe_text);
    let image = game.memory_mut();
    image.put8(PARTY_COUNT, 2);
    image.put(PARTY, &rowe_mon(1, 25, false, 0));
    image.put(PARTY + 0x4C, &rowe_mon(2, 19, false, 9));
    let dead_box = PC_STORAGE + 4 + 13 * 30 * 0x34;
    image.put(dead_box, &rowe_mon(3, 16, false, 0)[..0x34]);

    let changes = game.move_to_dead_box(0).unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(
        game.memory_mut().get(dead_box + 0x34, 0x34),
        rowe_mon(1, 25, false, 0)[..0x34]
    );
    let party = game.party();
    assert_eq!(party.iter().map(|p| p.pid).collect::<Vec<_>>(), [2]);
}

#[test]
fn a_game_whose_item_table_is_its_only_ban_needs_no_bag_lock_in_trainer_battles() {
    // R.O.W.E.: zeroing every non-ball item's battle use bans them in every battle.
    let text = |sha1: &str| {
        rowe_text(sha1).replace(
            "[rules.no_items_trainer]\nmethod = \"patch\"\nsites = [{ at = 0x08000300, expect = \"02 09 00 00\", write = \"0A 09 00 00\" }]",
            "[rules.no_items_trainer]\nmethod = \"zero_table_field\"",
        )
    };
    let mut game = game_from(text);
    assert_eq!(game.lock_bag_in_trainer_battles(true).unwrap(), []);
    assert_eq!(game.lock_bag_in_trainer_battles(false).unwrap(), []);
}

#[test]
fn type_names_come_from_the_profile_for_a_game_that_keeps_none_as_text() {
    let text = |sha1: &str| {
        hns_text(sha1).replace("type_names = 0x08008000\n", "").replace(
            "[symbols]",
            "type_name_list = [\"None\", \"Normal\", \"Fighting\"]\n\n[symbols]",
        )
    };
    let mut game = game_from(text);
    assert_eq!(game.type_name(1), "Normal");
    assert_eq!(game.type_name(2), "Fighting");
    assert_eq!(game.type_name(9), "type 9", "one the list doesn't name");
}

/// Where the test ROM holds stand-ins for two of Heart and Soul's `bl Random32`.
const RANDOM_CALLS: [u32; 2] = [0x0800_0240, 0x0800_0250];

/// The test profile with Heart and Soul's own randomizer: seeded by the
/// trainer ID, two calls to patch so draws depend only on it, and its CHAOS
/// option (which re-seeds from the live RNG) kept off.
fn hns_randomizer_text(sha1: &str) -> String {
    hns_options_text(sha1)
        .replace(
            "[options.battle_style]",
            "[options.chaos]\nname = \"CHAOS\"\nmenu = \"RANDOMIZER > CHAOS\"\nsave_block = 3\noffset = 0x14\nmask = 0x01\nvalues = { off = 0x00, on = 0x01 }\n\n[options.battle_style]",
        )
        .replace(
            "[rules.catch_block]",
            "[own_randomizer]\nseed_is_trainer_id = true\nsites = [{ at = 0x08000240, expect = \"F0 F7 1C FF\", write = \"00 20 C0 46\" }, { at = 0x08000250, expect = \"F1 F7 2A F8\", write = \"00 20 C0 46\" }]\nkeep = [{ option = \"chaos\", value = \"off\" }]\n\n[rules.catch_block]",
        )
}

fn hns_randomizer_game() -> ProfileGame<MemoryImage> {
    let mut rom = patchable_rom(b"BPEE");
    put(&mut rom, RANDOM_CALLS[0], &[0xF0, 0xF7, 0x1C, 0xFF]);
    put(&mut rom, RANDOM_CALLS[1], &[0xF1, 0xF7, 0x2A, 0xF8]);
    let mut game = game_over_rom(rom, hns_randomizer_text);
    game.memory_mut().put32(SAVE_BLOCK_3_POINTER, SAVE_BLOCK_3);
    game
}

#[test]
fn sharing_the_seed_gives_the_trainer_id_patches_the_live_draws_and_keeps_chaos_off() {
    let mut game = hns_randomizer_game();
    game.memory_mut().put32(SAVE_BLOCK_2 + 0x0A, 0x1111_2222);
    game.memory_mut().put8(SAVE_BLOCK_3 + 0x14, 0x03); // CHAOS on, beside another option

    let changes = game.share_seed(0xCAFE_F00D).unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(game.trainer_id(), 0xCAFE_F00D);
    for at in RANDOM_CALLS {
        assert_eq!(game.memory_mut().get(at, 4), [0x00, 0x20, 0xC0, 0x46], "{at:#010X}");
    }
    assert_eq!(
        game.memory_mut().get(SAVE_BLOCK_3 + 0x14, 1),
        [0x02],
        "CHAOS off, the other kept"
    );

    assert_eq!(
        game.share_seed(0xCAFE_F00D).unwrap(),
        [],
        "nothing more while it all holds"
    );
}

/// `gRngValue` and `gTrainerBattleOpponent_A` in the test image.
const RNG: u32 = 0x0300_4AF0;
const OPPONENT: u32 = 0x0203_8BCA;

/// The test profile with R.O.W.E.'s own randomizer: its seed in variable
/// 0x405B, the trainer ID's second byte read too, and the RNG pinned for
/// each trainer battle.
fn rowe_randomizer_text(sha1: &str) -> String {
    rowe_text(sha1).replace(
        "[rules.catch_block]",
        &format!(
            "[own_randomizer]\nseed_var = {{ var = 0x405B, vars = 0x139C }}\ntrainer_id_byte = 1\nrng_pin = {{ rng = {RNG:#010X}, opponent = {OPPONENT:#010X} }}\n\n[rules.catch_block]"
        ),
    )
}

#[test]
fn sharing_the_seed_sets_the_hacks_seed_variable_and_its_trainer_id_byte() {
    let mut game = game_from(rowe_randomizer_text);
    game.memory_mut().put32(SAVE_BLOCK_2 + 0x0A, 0x1111_2222);
    let changes = game.share_seed(0xAB00_1234).unwrap();
    game.memory_mut().apply(&changes);
    // The game's seeds run from 1 to 9998: 0xAB001234 % 9998 is 2492.
    let var = SAVE_BLOCK_1 + 0x139C + 0x5B * 2;
    assert_eq!(game.memory_mut().get(var, 2), 2493u16.to_le_bytes());
    assert_eq!(game.trainer_id(), 0x1111_AB22, "the second byte the seed's top one");
    assert_eq!(game.share_seed(0xAB00_1234).unwrap(), []);
}

#[test]
fn the_rng_is_pinned_from_a_trainer_battles_trigger_until_the_enemy_party_is_made() {
    let mut game = game_from(rowe_randomizer_text);
    assert_eq!(game.pin_rng(7).unwrap(), [], "no battle coming");

    game.memory_mut().put16(OPPONENT, 615);
    game.memory_mut().put32(ENEMY_PARTY, 0xDEAD_BEEF); // the last battle's
    let first = game.pin_rng(7).unwrap();
    let [Change::Write { addr: RNG, bytes }] = &first[..] else {
        panic!("one write to the RNG: {first:?}");
    };
    assert_eq!(game.pin_rng(7).unwrap(), first, "the same every frame");
    let mut other = game_from(rowe_randomizer_text);
    other.memory_mut().put16(OPPONENT, 615);
    assert_eq!(other.pin_rng(7).unwrap(), first, "the same in another Player's game");
    other.memory_mut().put16(OPPONENT, 616);
    assert_ne!(other.pin_rng(7).unwrap(), first, "another trainer, another state");
    assert_ne!(bytes, &[0, 0, 0, 0]);

    game.memory_mut().put32(ENEMY_PARTY, 0x1234_5678); // the party made
    assert_eq!(game.pin_rng(7).unwrap(), [], "pinned no more");
    game.memory_mut().put32(ENEMY_PARTY, 0xDEAD_BEEF);
    assert_eq!(game.pin_rng(7).unwrap(), [], "not again for the same battle");
}

#[test]
fn an_option_in_save_block_3_is_read_and_set_there() {
    let mut game = game_from(hns_options_text);
    game.memory_mut().put32(SAVE_BLOCK_3_POINTER, SAVE_BLOCK_3);
    game.memory_mut().put8(SAVE_BLOCK_3 + 0x17, 0x9F);
    assert_eq!(game.option("player_items").as_deref(), Some("allowed"));

    let changes = game.set_option("player_items", "banned").unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(
        game.memory_mut().get(SAVE_BLOCK_3 + 0x17, 1),
        [0xBF],
        "the other bits kept"
    );
    assert_eq!(game.option("player_items").as_deref(), Some("banned"));
}

fn game_from(text: impl Fn(&str) -> String) -> ProfileGame<MemoryImage> {
    game_over_rom(patchable_rom(b"BPEE"), text)
}

fn game_over_rom(rom: Vec<u8>, text: impl Fn(&str) -> String) -> ProfileGame<MemoryImage> {
    let store = ProfileStore::from_sources([("test.toml", text(&sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap().clone();
    ProfileGame::new(profile, rom.clone(), overworld_image(rom))
}

#[test]
fn a_species_packed_with_other_fields_reads_as_its_own_bits() {
    let mut game = game_from(hns_text);
    // Heart and Soul's species:11 shares its halfword with the tera type.
    let tera = TestMon {
        species: (13 << 11) | 25,
        ..bulbasaur()
    };
    set_party(game.memory_mut(), &[tera]);
    assert_eq!(game.party()[0].species, 25);
}

/// One evolution as Heart and Soul lists it: method, parameter, target, then
/// a pointer to further conditions.
fn evolution(method: u16, target: u16) -> Vec<u8> {
    let mut entry = method.to_le_bytes().to_vec();
    entry.extend(20u16.to_le_bytes());
    entry.extend(target.to_le_bytes());
    entry.extend([0; 6]);
    entry
}

#[test]
fn each_species_evolutions_are_read_through_its_species_info_up_to_the_end_marker() {
    let mut rom = patchable_rom(b"BPEE");
    let list_at = |species: u32| 0x0800_9000 + species * 0x40;
    for (species, targets) in [(19u32, vec![20u16]), (25, vec![26, 27])] {
        put(
            &mut rom,
            SPECIES_INFO + species * 0x10C + 0xA4,
            &list_at(species).to_le_bytes(),
        );
        let mut list: Vec<u8> = targets.iter().flat_map(|&t| evolution(3, t)).collect();
        list.extend(evolution(0xFFFF, 0));
        // What follows the end marker isn't the species'.
        list.extend(evolution(1, 21));
        put(&mut rom, list_at(species), &list);
    }
    let mut game = game_over_rom(rom, hns_species_text);
    let evolutions = game.evolutions();
    assert_eq!(evolutions[19], [20]);
    assert_eq!(evolutions[25], [26, 27]);
    assert!(evolutions[1].is_empty(), "no list, no evolutions");
}

/// The test profile with Heart and Soul's trainer table: 0x34-byte trainers,
/// their party pointer at +8 and size at +0x2B, and 0x24-byte party entries
/// whose level byte is followed by the ball.
fn hns_trainers_text(sha1: &str) -> String {
    hns_text(sha1)
        .replace("trainer_size = 40", "trainer_size = 0x34")
        .replace("trainer_party_flags = 0", "trainer_party_flags = 0x2B")
        .replace("trainer_party_size = 32", "trainer_party_size = 0x2B")
        .replace("trainer_party = 36", "trainer_party = 0x08")
        .replace("trainer_mon_level = 2", "trainer_mon_level = 0x1A")
        .replace(
            "trainer_mon_sizes = [8, 16, 8, 16]",
            "trainer_mon_sizes = [0x24, 0x24, 0x24, 0x24]",
        )
}

#[test]
fn a_cap_stage_defaults_to_its_trainers_strongest_pokemon_read_a_byte_a_level() {
    let mut rom = patchable_rom(b"BPEE");
    // Brock (trainer 414): levels 12 and 14, each followed by its ball (Poké Ball, 4).
    let brock = TRAINERS + 414 * 0x34;
    put(&mut rom, brock + 0x08, &TRAINER_PARTIES.to_le_bytes());
    put(&mut rom, brock + 0x2B, &[2]);
    put(&mut rom, TRAINER_PARTIES + 0x1A, &[12, 4]);
    put(&mut rom, TRAINER_PARTIES + 0x24 + 0x1A, &[14, 4]);
    let store = ProfileStore::from_sources([("test.toml", hns_trainers_text(&sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap();
    let caps = profile.default_level_caps(&rom);
    assert_eq!((caps[0].name.as_str(), caps[0].level), ("Brock", 14));
}

#[test]
fn a_cap_stage_whose_trainers_levels_are_scaled_in_game_defaults_to_the_level_its_profile_gives() {
    // R.O.W.E. makes its leaders' teams and levels as the Player goes.
    let rom = patchable_rom(b"BPEE");
    let text = rowe_text(&sha1_hex(&rom)).replace(
        "name = \"Brock\"\ntrainers = [414]\nbadges = 1",
        "name = \"Brock\"\ntrainers = [414]\nbadges = 1\nlevel = 15",
    );
    let store = ProfileStore::from_sources([("test.toml", text)]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap();
    let caps = profile.default_level_caps(&rom);
    assert_eq!((caps[0].name.as_str(), caps[0].level), ("Brock", 15));
}

/// Where the test ROM holds a stand-in for Heart and Soul's `GetCurrentLevelCap`.
const CAP_FUNCTION: u32 = 0x0800_0220;

#[test]
fn a_cap_function_stubbed_to_return_the_cap_takes_it_beside_the_level_100_checks() {
    let mut rom = patchable_rom(b"BPEE");
    put(&mut rom, CAP_FUNCTION, &[0x0E, 0x4B, 0x1B, 0x68]);
    let text = |sha1: &str| {
        hns_text(sha1).replace(
            r#"sites = [{ at = 0x08000200, expect = "64 28" }]"#,
            r#"sites = [{ at = 0x08000200, expect = "64 28" }, { at = 0x08000220, expect = "0E 4B 1B 68", write = "00 20 70 47" }]"#,
        )
    };
    let mut game = game_over_rom(rom, text);
    let changes = game.set_level_cap(30).unwrap();
    assert_eq!(
        changes,
        [
            Change::Patch {
                addr: 0x0800_0200,
                bytes: vec![30, 0x28]
            },
            Change::Patch {
                addr: CAP_FUNCTION,
                bytes: vec![30, 0x20, 0x70, 0x47]
            },
        ]
    );
}

#[test]
fn an_active_battler_gets_perish_song_in_its_own_volatiles_and_a_count_of_zero() {
    let mut game = game_from(hns_text);
    let foe = BATTLE_MONS + 0x88;
    let image = game.memory_mut();
    image.put8(IN_BATTLE, 0x02);
    image.put32(foe + 0x5C, 0x0000_0101); // other volatiles
    image.put8(foe + 0x6E, 0xF3); // a Perish count of 3 among other bits

    let changes = game.faint_active_battler(1).unwrap();
    game.memory_mut().apply(&changes);

    let image = game.memory_mut();
    assert_eq!(image.get32(foe + 0x5C), 0x0000_0181, "Perish Song on, the rest kept");
    assert_eq!(
        image.get(foe + 0x6E, 1),
        [0xF3 & !0x0C],
        "the count at zero, the rest kept"
    );
    assert_eq!(image.get32(BATTLE_MONS + 0x5C), 0, "the Player's battler untouched");
}
