//! On the profile-driven game: the "Give a nickname?" prompts the
//! key filter watches, in battle and on the overworld, naming a Pokémon in
//! the PC, and type names read from the loaded ROM.

mod support;

use pactlocke_game::{Change, Game, GameError, ProfileGame, ProfileStore, sha1_hex};
use support::*;

fn game_over(image: MemoryImage) -> ProfileGame<MemoryImage> {
    let rom = image.rom.clone();
    let store = ProfileStore::from_sources([("test.toml", profile_toml("test-game", &sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap().clone();
    ProfileGame::new(profile, rom, image)
}

const BATTLE_SCRIPT_POINTER: u32 = 0x0202_3D74;
const BATTLE_COMMUNICATION: u32 = 0x0202_3E82;
/// Where a test battle script sits: one byte, the command's index.
const BATTLE_SCRIPT: u32 = 0x0200_1000;
const MESSAGE: u32 = 0x0202_1D18;
const SCRIPT_CONTEXT_STATUS: u32 = 0x0300_0EA8;
const SCRIPT_POSITION: u32 = 0x0300_0EB0 + 0x08;
/// The test profile's field yes/no wait.
const FIELD_YES_NO: u32 = 0x0800_6500;
/// "give a nickname" in the game's characters.
const GIVE_A_NICKNAME: [u8; 15] = [
    0xDB, 0xDD, 0xEA, 0xD9, 0x00, 0xD5, 0x00, 0xE2, 0xDD, 0xD7, 0xDF, 0xE2, 0xD5, 0xE1, 0xD9,
];
/// "rate the nicknames", the Name Rater's question.
const RATE_THE_NICKNAMES: [u8; 18] = [
    0xE6, 0xD5, 0xE8, 0xD9, 0x00, 0xE8, 0xDC, 0xD9, 0x00, 0xE2, 0xDD, 0xD7, 0xDF, 0xE2, 0xD5, 0xE1, 0xD9, 0xE7,
];

fn in_battle(game: &mut ProfileGame<MemoryImage>) {
    let memory = game.memory_mut();
    memory.put8(0x0300_3529, 0x02); // gMain.inBattle
    memory.put32(0x0300_30F4, 0x0800_0001); // not the overworld
}

#[test]
fn the_battle_prompt_is_up_while_the_script_waits_on_the_nickname_command_with_its_yes_no_box() {
    let mut game = game_over(overworld_image(patchable_rom(b"BPRE")));
    in_battle(&mut game);
    game.memory_mut().put32(BATTLE_SCRIPT_POINTER, BATTLE_SCRIPT);
    game.memory_mut().put8(BATTLE_SCRIPT, 0xF0); // the nickname command's index

    game.memory_mut().put8(BATTLE_COMMUNICATION, 0);
    assert!(!game.nickname_prompt_up(), "the box isn't drawn yet");

    game.memory_mut().put8(BATTLE_COMMUNICATION, 1);
    assert!(game.nickname_prompt_up());
    assert_eq!(game.keep_nickname_cursor_on_yes(), Ok(Vec::new()), "already on YES");
    game.memory_mut().put8(BATTLE_COMMUNICATION + 1, 1); // moved to NO
    assert_eq!(
        game.keep_nickname_cursor_on_yes(),
        Ok(vec![Change::Write {
            addr: BATTLE_COMMUNICATION + 1,
            bytes: vec![0]
        }])
    );

    game.memory_mut().put8(BATTLE_SCRIPT, 0x01); // another command
    assert!(!game.nickname_prompt_up());
}

#[test]
fn the_field_prompt_is_up_while_a_yes_no_box_waits_on_a_question_about_a_nickname() {
    let mut game = game_over(overworld_image(patchable_rom(b"BPRE")));
    game.memory_mut().put8(SCRIPT_CONTEXT_STATUS, 1);
    game.memory_mut().put32(SCRIPT_POSITION, FIELD_YES_NO);
    // "Do you want to give a nickname…"
    game.memory_mut().put(MESSAGE + 18, &GIVE_A_NICKNAME);
    assert!(game.nickname_prompt_up());
    assert_eq!(
        game.keep_nickname_cursor_on_yes(),
        Ok(Vec::new()),
        "the field box starts on YES"
    );

    game.memory_mut().put32(SCRIPT_POSITION, FIELD_YES_NO - 3);
    assert!(!game.nickname_prompt_up(), "the question is still being printed");

    game.memory_mut().put32(SCRIPT_POSITION, FIELD_YES_NO);
    game.memory_mut().put(MESSAGE, &[0xFF; 0x40]);
    game.memory_mut().put(MESSAGE + 30, &RATE_THE_NICKNAMES);
    assert!(!game.nickname_prompt_up(), "the Name Rater's yes/no is left alone");
}

#[test]
fn a_pokemon_in_the_pc_can_be_named() {
    let mut game = game_over(overworld_image(patchable_rom(b"BPRE")));
    let boxed = rattata().party_bytes();
    game.memory_mut().put(PC_STORAGE + 4 + 30 * 80 + 80, &boxed[..80]); // box 2 slot 2

    let changes = game.write_pc_nickname(2, 2, "Nibbles").unwrap();
    game.memory_mut().apply(&changes);

    let stored = &game.pc()[0];
    assert_eq!(stored.pokemon.nickname, "Nibbles");
    assert_eq!(stored.pokemon.species, 19, "the encrypted data is untouched");
    assert_eq!(game.write_pc_nickname(2, 3, "Nibbles"), Err(GameError::NoSuchSlot(3)));
}

#[test]
fn type_names_are_read_from_the_loaded_rom() {
    let mut rom = patchable_rom(b"BPRE");
    // "GRASS" padded with spaces, as the game stores it, at type 12.
    put(
        &mut rom,
        TYPE_NAMES + 12 * 7,
        &[0xC1, 0xCC, 0xBB, 0xCD, 0xCD, 0x00, 0xFF],
    );
    let mut game = game_over(overworld_image(rom));

    assert_eq!(game.type_name(12), "GRASS");
}
