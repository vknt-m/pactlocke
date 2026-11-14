//! Seam 2: the profile-driven game interface, over an in-memory
//! image of a game laid out like FireRed. Actions return writes and patches;
//! the tests land them the way the Run engine will, then read the game again.

mod support;

use pactlocke_game::{
    Area, Battle, BattleKind, Battler, Change, Game, GameError, PartyStats, Pokemon, ProfileGame, ProfileStore,
    ScriptContext, ScriptStatus, Side, SourceKind, StoredPokemon, sha1_hex,
};
use support::*;

fn game_over(image: MemoryImage) -> ProfileGame<MemoryImage> {
    let rom = image.rom.clone();
    let store = ProfileStore::from_sources([("test.toml", profile_toml("test-game", &sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap().clone();
    ProfileGame::new(profile, rom, image)
}

fn at_overworld() -> ProfileGame<MemoryImage> {
    game_over(overworld_image(patchable_rom(b"BPRE")))
}

fn bulbasaur_read() -> Pokemon {
    Pokemon {
        pid: 0x1234_5678,
        ot_id: 0x0BAD_F00D,
        species: 1,
        nickname: "BULBA".into(),
        met_section: 0x58,
        is_egg: false,
        stats: Some(PartyStats {
            level: 5,
            hp: 19,
            max_hp: 21,
        }),
    }
}

#[test]
fn the_party_is_read_and_decrypted() {
    let mut game = at_overworld();
    set_party(game.memory_mut(), &[bulbasaur(), rattata()]);

    let party = game.party();
    assert_eq!(party.len(), 2);
    assert_eq!(party[0], bulbasaur_read());
    assert_eq!(party[1].species, 19);
    assert_eq!(party[1].nickname, "Rat 2");
    assert_eq!(party[1].met_section, 0x65);
    assert_eq!(party[1].stats.unwrap().hp, 0);
}

#[test]
fn the_area_is_the_current_region_map_section_after_renames() {
    let mut game = at_overworld();
    game.memory_mut().put8(0x0203_6E10, 0x8C);
    assert_eq!(
        game.area(),
        Some(Area {
            id: 0x8C,
            name: "Pokémon Tower".into()
        })
    );
    game.memory_mut().put8(0x0203_6E10, 0x77);
    assert_eq!(game.area(), None, "a section the profile doesn't list");
}

#[test]
fn a_pc_slot_flagged_as_holding_a_pokemon_but_with_no_species_is_empty() {
    // As Emerald leaves its last box slot for a frame while it moves its save
    // data: the has-species flag set over zeroed data (PID 0, species 0).
    let mut game = at_overworld();
    let last_slot = PC_STORAGE + 4 + (13 * 30 + 29) * 80;
    game.memory_mut().put8(last_slot + 0x13, 0x02);
    assert_eq!(game.pc(), vec![], "no Pokémon there: nothing a Gift could be");
}

#[test]
fn the_pc_lists_every_stored_pokemon_with_its_box_and_slot() {
    let mut game = at_overworld();
    let boxed = bulbasaur().party_bytes();
    // Box 2, slot 3 (both counted from 1): the first 80 bytes of the party form.
    game.memory_mut().put(PC_STORAGE + 4 + (30 + 2) * 80, &boxed[..80]);

    let pc = game.pc();
    assert_eq!(
        pc,
        vec![StoredPokemon {
            box_number: 2,
            slot: 3,
            pokemon: Pokemon {
                stats: None,
                ..bulbasaur_read()
            }
        }]
    );
}

#[test]
fn a_wild_battle_shows_its_battlers_and_the_players_party_slot() {
    let mut game = at_overworld();
    let image = game.memory_mut();
    image.put8(0x0300_3529, 0x02); // in battle
    image.put32(0x0202_2B4C, 0x0000_0004); // flags: neither trainer nor double
    image.put16(0x0202_3BE4, 25); // player battler: Pikachu
    image.put16(0x0202_3BE4 + 0x28, 30);
    image.put16(0x0202_3BE4 + 0x58, 16); // opponent battler: Pidgey
    image.put16(0x0202_3BE4 + 0x58 + 0x28, 12);
    image.put16(0x0202_3BCE, 1); // the player's battler is party slot 1 (from 0)

    assert_eq!(
        game.battle(),
        Some(Battle {
            kind: BattleKind::Wild,
            double: false,
            battlers: vec![
                Battler {
                    position: 0,
                    side: Side::Player,
                    species: 25,
                    hp: 30,
                    party_slot: Some(1)
                },
                Battler {
                    position: 1,
                    side: Side::Opponent,
                    species: 16,
                    hp: 12,
                    party_slot: None
                },
            ]
        })
    );
    assert!(!game.at_safe_overworld_point(), "never mid-battle");
}

#[test]
fn a_double_trainer_battle_has_four_battlers() {
    let mut game = at_overworld();
    game.memory_mut().put8(0x0300_3529, 0x02);
    game.memory_mut().put32(0x0202_2B4C, 0x0000_0009); // trainer + double
    let battle = game.battle().unwrap();
    assert_eq!(battle.kind, BattleKind::Trainer);
    assert!(battle.double);
    assert_eq!(battle.battlers.len(), 4);
    assert_eq!(battle.battlers[2].side, Side::Player);
}

#[test]
fn there_is_no_battle_and_a_safe_point_on_the_overworld() {
    let mut game = at_overworld();
    assert_eq!(game.battle(), None);
    assert!(game.at_safe_overworld_point());

    game.memory_mut().put32(0x0300_30F4, 0x0801_0001); // some other screen
    assert!(!game.at_safe_overworld_point());
}

#[test]
fn the_script_context_says_whether_a_script_runs_or_waits_and_where() {
    let mut game = at_overworld();
    assert_eq!(
        game.script(),
        ScriptContext {
            status: ScriptStatus::Idle,
            position: None
        }
    );
    game.memory_mut().put8(0x0300_0EA8, 1);
    game.memory_mut().put32(0x0300_0EB0 + 8, 0x0816_A1B0);
    assert_eq!(
        game.script(),
        ScriptContext {
            status: ScriptStatus::Waiting,
            position: Some(0x0816_A1B0)
        }
    );
    game.memory_mut().put8(0x0300_0EA8, 0);
    assert_eq!(game.script().status, ScriptStatus::Running);
}

#[test]
fn a_running_scripts_calls_are_read_innermost_first() {
    let mut game = at_overworld();
    assert!(game.script_calls().is_empty(), "no script running");
    game.memory_mut().put8(0x0300_0EA8, 0); // running
    game.memory_mut().put8(0x0300_0EB0, 2); // two calls deep
    game.memory_mut().put32(0x0300_0EB0 + 12, 0x0816_9CA0);
    game.memory_mut().put32(0x0300_0EB0 + 16, 0x081A_5000);
    assert_eq!(game.script_calls(), [0x081A_5000, 0x0816_9CA0]);
}

#[test]
fn a_battle_nobody_can_catch_in_is_neither_wild_nor_a_trainers() {
    let mut game = at_overworld();
    game.memory_mut().put8(0x0300_3529, 0x02); // in battle
    for flags in [0x0000_8000u32, 0x0000_0200, 0x0001_0000] {
        game.memory_mut().put32(0x0202_2B4C, flags);
        assert_eq!(game.battle().unwrap().kind, BattleKind::Uncatchable, "{flags:#x}");
    }
    game.memory_mut().put32(0x0202_2B4C, 0);
    assert_eq!(game.battle().unwrap().kind, BattleKind::Wild);
}

#[test]
fn badges_trainer_id_and_options_are_read_from_the_save_blocks() {
    let mut game = at_overworld();
    let image = game.memory_mut();
    image.put8(SAVE_BLOCK_1 + 0xFE4, 0b0000_0101); // Boulder and Thunder
    image.put32(SAVE_BLOCK_2 + 0x0A, 0x0BAD_F00D);
    image.put8(SAVE_BLOCK_2 + 0x15, 0x01 | 0x02 | 0x04); // battle style Set, plus other options

    let badges = game.badges();
    assert_eq!(badges.count(), 2);
    assert!(badges.has(1) && !badges.has(2) && badges.has(3));
    assert_eq!(game.trainer_id(), 0x0BAD_F00D);
    assert_eq!(game.option("battle_style").as_deref(), Some("set"));
    assert_eq!(game.option("no_such_option"), None);
}

/// The test profile laid out like Ruby and Sapphire: save blocks and the PC
/// at fixed addresses rather than behind pointers, and badge flags that
/// don't start on a byte (Ruby's FLAG_BADGE01_GET is 0x807).
fn ruby_like(image: MemoryImage) -> ProfileGame<MemoryImage> {
    let rom = image.rom.clone();
    let text = profile_toml("ruby-like", &sha1_hex(&rom))
        .replace(
            "save_block_1 = { pointer = 0x03005008 }",
            "save_block_1 = { at = 0x02025000 }",
        )
        .replace(
            "save_block_2 = { pointer = 0x0300500C }",
            "save_block_2 = { at = 0x02024F00 }",
        )
        .replace(
            "pc_storage = { pointer = 0x03005010 }",
            "pc_storage = { at = 0x02029000 }",
        )
        .replace("badge_flags = 0x820", "badge_flags = 0x807");
    let store = ProfileStore::from_sources([("ruby-like.toml", text)]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap().clone();
    ProfileGame::new(profile, rom, image)
}

#[test]
fn a_game_with_fixed_save_blocks_reads_them_where_they_are() {
    let mut image = MemoryImage::new(patchable_rom(b"AXVE"));
    // Nothing behind the pointer symbols: they would read address 0.
    image.put32(SAVE_BLOCK_2 + 0x0A, 0x0BAD_F00D);
    let flags = SAVE_BLOCK_1 + 0xEE0;
    // Badges 1 and 3: flags 0x807 and 0x809, either side of a byte boundary.
    image.put8(flags + 0x807 / 8, 1 << (0x807 % 8));
    image.put8(flags + 0x809 / 8, 1 << (0x809 % 8));
    let bulba = TestMon {
        pid: 0x1234_5678,
        ot_id: 0x0BAD_F00D,
        species: 1,
        nickname: BULBA,
        met_section: 0x10,
        egg: false,
        level: 5,
        hp: 19,
        max_hp: 21,
    };
    image.put(PC_STORAGE + 4 + 80 * 2, &bulba.party_bytes()[..80]);
    let mut game = ruby_like(image);

    assert_eq!(game.trainer_id(), 0x0BAD_F00D);
    let badges = game.badges();
    assert_eq!(badges.count(), 2);
    assert!(badges.has(1) && !badges.has(2) && badges.has(3));
    let pc = game.pc();
    assert_eq!(pc.len(), 1);
    assert_eq!((pc[0].box_number, pc[0].slot, pc[0].pokemon.species), (1, 3, 1));
}

#[test]
fn blocking_catches_patches_every_catch_site_and_unblocking_puts_it_back() {
    let mut game = at_overworld();
    assert_eq!(
        game.block_catches(true).unwrap(),
        vec![Change::Patch {
            addr: CATCH_SITE,
            bytes: vec![0x01, 0x20, 0x70, 0x47]
        }]
    );
    assert_eq!(
        game.block_catches(false).unwrap(),
        vec![Change::Patch {
            addr: CATCH_SITE,
            bytes: vec![0x10, 0xB5, 0x00, 0x24]
        }]
    );
}

#[test]
fn the_level_cap_is_written_into_every_cap_site() {
    let mut game = at_overworld();
    assert_eq!(
        game.set_level_cap(14).unwrap(),
        vec![Change::Patch {
            addr: CAP_SITE,
            bytes: vec![14, 0x28]
        }]
    );
    assert_eq!(game.set_level_cap(0), Err(GameError::BadLevel(0)));
    assert_eq!(game.set_level_cap(101), Err(GameError::BadLevel(101)));
}

#[test]
fn the_bag_lock_rewrites_the_trainer_battle_mask() {
    let mut game = at_overworld();
    assert_eq!(
        game.lock_bag_in_trainer_battles(true).unwrap(),
        vec![Change::Patch {
            addr: BAG_SITE,
            bytes: vec![0x0A, 0x09, 0x00, 0x00]
        }]
    );
}

#[test]
fn banning_items_in_wild_battles_leaves_only_balls_usable_and_lifting_it_restores_them() {
    // Items 1-3 of the test item table (44 bytes each, from 0x08001000):
    // a Potion (pocket 1, battle use 2), a Poké Ball (pocket 3, battle use 2)
    // and a Key Item with no battle use.
    let mut rom = patchable_rom(b"BPRE");
    let item = |i: u32| 0x0800_1000 + i * 44;
    for (i, pocket, usage) in [(1, 1, 2), (2, 3, 2), (3, 5, 0)] {
        put(&mut rom, item(i) + 26, &[pocket]);
        put(&mut rom, item(i) + 32, &[usage]);
    }
    let mut game = game_over(overworld_image(rom));

    assert_eq!(
        game.ban_items_in_wild_battles(true).unwrap(),
        vec![Change::Patch {
            addr: item(1) + 32,
            bytes: vec![0]
        }]
    );
    assert_eq!(
        game.ban_items_in_wild_battles(false).unwrap(),
        vec![Change::Patch {
            addr: item(1) + 32,
            bytes: vec![2]
        }]
    );
}

#[test]
fn a_benched_pokemon_faints_at_once() {
    let mut game = at_overworld();
    set_party(game.memory_mut(), &[bulbasaur(), rattata()]);
    let changes = game.faint_benched(0).unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(game.party()[0].stats.unwrap().hp, 0);
    assert_eq!(game.faint_benched(2), Err(GameError::NoSuchSlot(2)));
}

#[test]
fn an_active_battler_is_given_a_perish_count_of_zero() {
    let mut game = at_overworld();
    game.memory_mut().put8(0x0300_3529, 0x02);
    game.memory_mut().put32(0x0202_3DFC + 4, 0x0000_1000); // an existing status on battler 1
    let changes = game.faint_active_battler(1).unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(
        game.memory_mut().get32(0x0202_3DFC + 4),
        0x0000_1020,
        "Perish Song added"
    );
    assert_eq!(
        game.memory_mut().get(0x0202_3E0C + 0x1C + 0x0F, 1),
        vec![0],
        "timer at 0"
    );

    game.memory_mut().put8(0x0300_3529, 0);
    assert_eq!(game.faint_active_battler(1), Err(GameError::NotInBattle));
}

#[test]
fn a_dead_pokemon_moves_from_the_party_to_the_dead_box() {
    let mut game = at_overworld();
    set_party(game.memory_mut(), &[bulbasaur(), rattata()]);

    let changes = game.move_to_dead_box(1).unwrap();
    game.memory_mut().apply(&changes);

    assert_eq!(game.party(), vec![bulbasaur_read()]);
    let pc = game.pc();
    assert_eq!(pc.len(), 1);
    assert_eq!((pc[0].box_number, pc[0].slot), (14, 1));
    assert_eq!(pc[0].pokemon.pid, 0x8765_4329);
    assert_eq!(pc[0].pokemon.nickname, "Rat 2");
}

#[test]
fn a_script_position_is_matched_to_the_nearest_catalogued_instruction_before_it() {
    let mut game = at_overworld();
    let source = |game: &mut ProfileGame<MemoryImage>, at: u32| game.source_before(at).map(|s| (s.key, s.kind));

    assert_eq!(
        source(&mut game, 0x0800_601E),
        Some(("starter".into(), SourceKind::Starter))
    );
    assert_eq!(
        source(&mut game, 0x0800_6107),
        Some(("snorlax_route_12".into(), SourceKind::Static))
    );
    assert_eq!(
        source(&mut game, 0x0800_6210),
        Some(("mt_moon_fossil".into(), SourceKind::Gift))
    );
    assert_eq!(
        source(&mut game, 0x0800_6250),
        Some(("mt_moon_fossil".into(), SourceKind::Gift)),
        "two instructions, one source"
    );
    assert_eq!(
        source(&mut game, 0x0800_6310),
        Some(("togepi_egg".into(), SourceKind::Egg))
    );
    assert_eq!(game.source_before(0x0800_5FFF), None, "nothing catalogued before it");
    assert_eq!(game.source_before(0x0800_6500), None, "too far past the last one");
    assert_eq!(game.source_before(0x0800_6000).unwrap().name, "Starter");
}

#[test]
fn a_party_pokemon_is_released_and_the_party_compacted() {
    let mut game = at_overworld();
    set_party(game.memory_mut(), &[rattata(), bulbasaur()]);

    let changes = game.release_from_party(0).unwrap();
    game.memory_mut().apply(&changes);

    assert_eq!(game.party(), vec![bulbasaur_read()]);
    assert!(game.pc().is_empty(), "gone, not boxed");
    assert_eq!(game.release_from_party(1), Err(GameError::NoSuchSlot(1)));
}

#[test]
fn a_pokemon_in_the_pc_is_released() {
    let mut game = at_overworld();
    let boxed = rattata().party_bytes();
    game.memory_mut().put(PC_STORAGE + 4 + 30 * 80 + 80, &boxed[..80]); // box 2 slot 2
    assert_eq!(game.pc().len(), 1);

    let changes = game.release_from_pc(2, 2).unwrap();
    game.memory_mut().apply(&changes);

    assert!(game.pc().is_empty());
    assert_eq!(
        game.release_from_pc(2, 2),
        Err(GameError::NoSuchSlot(2)),
        "already empty"
    );
    assert_eq!(game.release_from_pc(15, 1), Err(GameError::NoSuchBox(15)));
}

#[test]
fn moving_to_a_box_fills_its_first_empty_slot_and_a_full_box_refuses() {
    let mut game = at_overworld();
    set_party(game.memory_mut(), &[bulbasaur(), rattata()]);
    let boxed = rattata().party_bytes();
    game.memory_mut().put(PC_STORAGE + 4, &boxed[..80]); // box 1 slot 1 taken

    let changes = game.move_to_box(0, 1).unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(game.party().len(), 1);
    assert_eq!(game.party()[0].species, 19, "the rest of the party moves up");
    assert!(
        game.pc()
            .iter()
            .any(|s| s.box_number == 1 && s.slot == 2 && s.pokemon.species == 1)
    );

    for slot in 0..30 {
        game.memory_mut()
            .put(PC_STORAGE + 4 + (30 * 2 + slot) * 80, &boxed[..80]);
    }
    assert_eq!(game.move_to_box(0, 3), Err(GameError::BoxFull(3)));
}

#[test]
fn a_nickname_is_written_in_the_games_own_characters() {
    let mut game = at_overworld();
    set_party(game.memory_mut(), &[bulbasaur()]);
    let changes = game.write_nickname(0, "Nibbles").unwrap();
    game.memory_mut().apply(&changes);
    assert_eq!(game.party()[0].nickname, "Nibbles");
    assert_eq!(game.party()[0].species, 1, "the encrypted data is untouched");

    assert!(matches!(
        game.write_nickname(0, "ElevenChars"),
        Err(GameError::BadName(_))
    ));
    assert!(matches!(game.write_nickname(0, "日本"), Err(GameError::BadName(_))));
}

#[test]
fn setting_an_option_keeps_the_other_options_in_its_byte() {
    let mut game = at_overworld();
    game.memory_mut().put8(SAVE_BLOCK_2 + 0x15, 0x05); // sound and scene bits, battle style Shift
    let changes = game.set_option("battle_style", "set").unwrap();
    assert_eq!(
        changes,
        vec![Change::Write {
            addr: SAVE_BLOCK_2 + 0x15,
            bytes: vec![0x07]
        }]
    );
    assert_eq!(
        game.set_option("battle_style", "fast"),
        Err(GameError::UnknownOption("battle_style = fast".into()))
    );
}

#[test]
fn set_mode_writes_the_battle_style_option_only_while_it_isnt_set() {
    let mut game = at_overworld();
    game.memory_mut().put8(SAVE_BLOCK_2 + 0x15, 0x05); // battle style Shift
    let changes = game.force_set_mode().unwrap();
    assert_eq!(
        changes,
        vec![Change::Write {
            addr: SAVE_BLOCK_2 + 0x15,
            bytes: vec![0x07]
        }]
    );

    game.memory_mut().apply(&changes);
    assert_eq!(game.option("battle_style"), Some("set".into()));
    assert_eq!(game.force_set_mode(), Ok(Vec::new()), "nothing to write once it is Set");
}

#[test]
fn a_species_primary_type_is_read_from_the_loaded_rom() {
    // A randomized ROM changes types, so they come from the ROM, not a list.
    let mut rom = patchable_rom(b"BPRE");
    put(&mut rom, SPECIES_INFO + 19 * 28 + 6, &[0x0D]);
    let mut game = game_over(overworld_image(rom));
    assert_eq!(game.primary_type(19), 0x0D);
}

#[test]
fn a_species_name_is_read_from_the_loaded_rom() {
    // "RATTATA" in the game's character set, then the terminator.
    let mut rom = patchable_rom(b"BPRE");
    put(
        &mut rom,
        SPECIES_NAMES + 19 * 11,
        &[0xCC, 0xBB, 0xCE, 0xCE, 0xBB, 0xCE, 0xBB, 0xFF],
    );
    let mut game = game_over(overworld_image(rom));
    assert_eq!(game.species_name(19), "RATTATA");
}

/// One evolution in the table: method, parameter, target species.
fn put_evolution(rom: &mut [u8], species: u16, index: u32, method: u16, param: u16, into: u16) {
    let mut entry = Vec::new();
    for half in [method, param, into, 0] {
        entry.extend_from_slice(&half.to_le_bytes());
    }
    put(rom, EVOLUTION_TABLE + u32::from(species) * 40 + index * 8, &entry);
}

#[test]
fn evolutions_are_read_from_the_loaded_rom() {
    let mut rom = patchable_rom(b"BPRE");
    put_evolution(&mut rom, 1, 0, 4, 16, 2); // Bulbasaur at level 16
    put_evolution(&mut rom, 2, 0, 4, 32, 3);
    put_evolution(&mut rom, 133, 0, 7, 96, 135); // Eevee by stones and friendship
    put_evolution(&mut rom, 133, 1, 7, 97, 134);
    put_evolution(&mut rom, 133, 4, 3, 0, 197);
    put_evolution(&mut rom, 19, 0, 4, 20, 9999); // no such species: ignored
    let mut game = game_over(overworld_image(rom));

    let evolutions = game.evolutions();
    assert_eq!(evolutions.len(), 412, "one entry per species");
    assert_eq!(evolutions[1], [2]);
    assert_eq!(evolutions[2], [3]);
    assert_eq!(evolutions[3], Vec::<u16>::new());
    assert_eq!(evolutions[133], [135, 134, 197]);
    assert_eq!(evolutions[19], Vec::<u16>::new());
}

#[test]
fn the_opponents_party_is_read_before_the_battle_fills_in_its_battlers() {
    let mut game = at_overworld();
    assert!(game.opponent_party().is_empty());
    set_enemy_party(game.memory_mut(), &[rattata()]);
    let opponents = game.opponent_party();
    assert_eq!(opponents.len(), 1);
    assert_eq!(opponents[0].species, 19);
    assert_eq!(opponents[0].nickname, "Rat 2");
}

#[test]
fn the_player_holds_poke_balls_once_the_balls_pocket_has_any() {
    let mut game = at_overworld();
    assert!(!game.holds_poke_balls(), "a new game starts with none");

    // A Great Ball (item 3) in the Balls pocket's third slot.
    game.memory_mut().put16(SAVE_BLOCK_1 + 0x430 + 2 * 4, 3);
    assert!(game.holds_poke_balls());
}
