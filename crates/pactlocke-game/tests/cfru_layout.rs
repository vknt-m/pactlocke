//! Seam 2: games built on CFRU (Unbound, Radical Red). They keep
//! Pokémon unencrypted, hold the PC as 58-byte compressed slots in boxes
//! spread over RAM, check the level cap as `level > 99`, and Radical Red
//! always plays the Set battle style.

mod support;

use pactlocke_game::{Capability, CapabilityReport, Change, Game, ProfileGame, ProfileStore, Rule, sha1_hex};
use support::*;

/// CFRU's 25 boxes: 1 to 19 one after another, then 20 to 22, 23 and 24, and 25 apart.
const BOX_STARTS: [u32; 25] = [
    0x0202_9318,
    0x0202_99E4,
    0x0202_A0B0,
    0x0202_A77C,
    0x0202_AE48,
    0x0202_B514,
    0x0202_BBE0,
    0x0202_C2AC,
    0x0202_C978,
    0x0202_D044,
    0x0202_D710,
    0x0202_DDDC,
    0x0202_E4A8,
    0x0202_EB74,
    0x0202_F240,
    0x0202_F90C,
    0x0202_FFD8,
    0x0203_06A4,
    0x0203_0D70,
    0x0203_CB44,
    0x0203_D210,
    0x0203_D8DC,
    0x0202_7434,
    0x0202_7B00,
    0x0202_4638,
];
const SLOT: u32 = 58;
/// A level-cap site CFRU writes as `cmp r3, #99`.
const CFRU_CAP_SITE: u32 = 0x0800_0210;

fn cfru_text(sha1: &str) -> String {
    let starts: Vec<String> = BOX_STARTS.iter().map(|a| format!("{a:#010X}")).collect();
    profile_toml("cfru-like", sha1)
        .replace(
            "message_buffer = 0x02021D18",
            &format!("message_buffer = 0x02021D18\npc_box_starts = [{}]", starts.join(", ")),
        )
        .replace("encrypted = true", "encrypted = false")
        .replace("box_slot_size = 80", "box_slot_size = 58\nbox_format = \"compressed\"")
        .replace("pc_boxes = 14", "pc_boxes = 25")
        .replace(
            r#"sites = [{ at = 0x08000200, expect = "64 28" }]"#,
            r#"sites = [{ at = 0x08000200, expect = "64 28" }, { at = 0x08000210, expect = "63 2B", below_cap = 1 }]"#,
        )
        .replace(
            "[rules.set_mode]\nmethod = \"option_bit\"\noption = \"battle_style\"\nvalue = \"set\"",
            "[rules.set_mode]\nmethod = \"always\"",
        )
}

fn cfru_rom() -> Vec<u8> {
    let mut rom = patchable_rom(b"BPRE");
    put(&mut rom, CFRU_CAP_SITE, &[0x63, 0x2B]);
    rom
}

fn cfru_game() -> ProfileGame<MemoryImage> {
    let rom = cfru_rom();
    let store = ProfileStore::from_sources([("cfru.toml", cfru_text(&sha1_hex(&rom)))]).unwrap();
    let profile = store.find(&sha1_hex(&rom)).unwrap().clone();
    ProfileGame::new(profile, rom.clone(), overworld_image(rom))
}

/// A Pokémon in CFRU's unencrypted party form: substructures in Growth,
/// Attacks, EVs, Misc order, each byte numbered so a test can tell them apart.
fn plain_party_mon(pid: u32, species: u16, nickname: [u8; 10], met_section: u8) -> [u8; 100] {
    let mut mon = [0u8; 100];
    mon[0..4].copy_from_slice(&pid.to_le_bytes());
    mon[4..8].copy_from_slice(&0x0BAD_F00Du32.to_le_bytes());
    mon[8..18].copy_from_slice(&nickname);
    mon[18] = 2; // language
    mon[19] = 0x02; // has species
    for (i, b) in mon[20..28].iter_mut().enumerate() {
        *b = 0x40 + i as u8; // OT name and markings
    }
    mon[0x1C..0x20].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // checksum, unknown: not kept
    // Growth: species, item, EXP, PP bonuses, friendship, ball, and a byte not kept.
    mon[0x20..0x22].copy_from_slice(&species.to_le_bytes());
    mon[0x22..0x24].copy_from_slice(&0x0123u16.to_le_bytes());
    mon[0x24..0x28].copy_from_slice(&0x0001_E240u32.to_le_bytes());
    mon[0x28..0x2C].copy_from_slice(&[0x55, 0x46, 0x04, 0xEE]);
    // Attacks: moves 33, 45, 0x3FF and 1, then their PP (not kept).
    for (i, m) in [33u16, 45, 0x3FF, 1].iter().enumerate() {
        mon[0x2C + i * 2..0x2E + i * 2].copy_from_slice(&m.to_le_bytes());
    }
    mon[0x34..0x38].copy_from_slice(&[35, 40, 5, 10]);
    // EVs, then contest stats (not kept).
    mon[0x38..0x3E].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
    mon[0x3E..0x44].copy_from_slice(&[9; 6]);
    // Misc: Pokérus, met location, origin, IVs; then ribbons (not kept).
    mon[0x44] = 0x11;
    mon[0x45] = met_section;
    mon[0x46..0x48].copy_from_slice(&0x2305u16.to_le_bytes());
    mon[0x48..0x4C].copy_from_slice(&0x3FFF_FFFFu32.to_le_bytes());
    mon[0x4C..0x50].copy_from_slice(&[0x77; 4]);
    mon[0x54] = 5; // level
    mon
}

/// The same Pokémon in CFRU's 58-byte PC form (its CreateCompressedMonFromBoxMon).
fn compressed(mon: &[u8; 100]) -> Vec<u8> {
    let mut out = mon[0..28].to_vec();
    out.extend_from_slice(&mon[0x20..0x2B]);
    let mv = |i: usize| u64::from(u16::from_le_bytes([mon[0x2C + i * 2], mon[0x2D + i * 2]]) & 0x3FF);
    let moves = mv(0) | mv(1) << 10 | mv(2) << 20 | mv(3) << 30;
    out.extend_from_slice(&moves.to_le_bytes()[..5]);
    out.extend_from_slice(&mon[0x38..0x3E]);
    out.extend_from_slice(&mon[0x44..0x4C]);
    out
}

fn set_plain_party(image: &mut MemoryImage, mons: &[[u8; 100]]) {
    image.put8(PARTY_COUNT, mons.len() as u8);
    for (i, mon) in mons.iter().enumerate() {
        image.put(PARTY + i as u32 * 100, mon);
    }
}

#[test]
fn the_pc_is_read_from_cfrus_compressed_slots_in_every_box() {
    let mut game = cfru_game();
    let bulba = plain_party_mon(0x1234_5678, 1, BULBA, 0x58);
    let rat = plain_party_mon(0x8765_4329, 19, RAT_2, 0x65);
    // Box 1 slot 3, and box 25 (apart from the rest) slot 30.
    game.memory_mut().put(BOX_STARTS[0] + 2 * SLOT, &compressed(&bulba));
    game.memory_mut().put(BOX_STARTS[24] + 29 * SLOT, &compressed(&rat));

    let pc = game.pc();
    let found: Vec<_> = pc
        .iter()
        .map(|s| {
            (
                s.box_number,
                s.slot,
                s.pokemon.species,
                s.pokemon.nickname.clone(),
                s.pokemon.met_section,
            )
        })
        .collect();
    assert_eq!(
        found,
        vec![
            (1, 3, 1, "BULBA".to_owned(), 0x58),
            (25, 30, 19, "Rat 2".to_owned(), 0x65)
        ]
    );
    assert_eq!(pc[0].pokemon.pid, 0x1234_5678);
    assert_eq!(pc[0].pokemon.ot_id, 0x0BAD_F00D);
}

#[test]
fn a_dead_pokemon_is_compressed_into_the_dead_box_the_way_cfru_stores_it() {
    let mut game = cfru_game();
    let bulba = plain_party_mon(0x1234_5678, 1, BULBA, 0x58);
    let rat = plain_party_mon(0x8765_4329, 19, RAT_2, 0x65);
    set_plain_party(game.memory_mut(), &[bulba, rat]);

    let changes = game.move_to_dead_box(1).unwrap();
    game.memory_mut().apply(&changes);

    let dead_box = BOX_STARTS[13];
    assert_eq!(game.memory_mut().get(dead_box, SLOT as usize), compressed(&rat));
    let pc = game.pc();
    assert_eq!(pc.len(), 1);
    assert_eq!((pc[0].box_number, pc[0].slot, pc[0].pokemon.species), (14, 1, 19));
    assert_eq!(game.party().len(), 1);
    assert_eq!(game.party()[0].species, 1);
}

#[test]
fn a_box_past_the_fourteenth_is_filled_where_cfru_keeps_it() {
    let mut game = cfru_game();
    let rat = plain_party_mon(0x8765_4329, 19, RAT_2, 0x65);
    set_plain_party(game.memory_mut(), &[rat]);
    // Box 20's first slot is taken.
    game.memory_mut().put(BOX_STARTS[19], &compressed(&rat));

    let changes = game.move_to_box(0, 20).unwrap();
    game.memory_mut().apply(&changes);

    assert_eq!(
        game.memory_mut().get(BOX_STARTS[19] + SLOT, SLOT as usize),
        compressed(&rat)
    );
    let changes = game.release_from_pc(20, 2).unwrap();
    assert_eq!(
        changes,
        vec![Change::Write {
            addr: BOX_STARTS[19] + SLOT,
            bytes: vec![0; SLOT as usize]
        }]
    );
}

#[test]
fn a_cap_site_checked_as_level_over_99_is_written_one_below_the_cap() {
    let mut game = cfru_game();
    assert_eq!(
        game.set_level_cap(14).unwrap(),
        vec![
            Change::Patch {
                addr: CAP_SITE,
                bytes: vec![14, 0x28]
            },
            Change::Patch {
                addr: CFRU_CAP_SITE,
                bytes: vec![13, 0x2B]
            },
        ]
    );
}

#[test]
fn a_game_that_always_plays_set_mode_needs_nothing_done() {
    let mut game = cfru_game();
    assert_eq!(game.force_set_mode().unwrap(), vec![]);
    let report = CapabilityReport::check(
        &ProfileStore::from_sources([("cfru.toml", cfru_text(&sha1_hex(&cfru_rom())))])
            .unwrap()
            .find(&sha1_hex(&cfru_rom()))
            .unwrap()
            .clone(),
        &cfru_rom(),
    );
    assert_eq!(report.get(Rule::SetMode), &Capability::Enforceable);
}

#[test]
fn a_compressed_pc_must_be_unencrypted_58_byte_slots_with_a_start_for_every_box() {
    let sha1 = sha1_hex(&cfru_rom());
    let refused = |text: String| {
        ProfileStore::from_sources([("bad.toml", text)])
            .unwrap_err()
            .to_string()
    };

    let encrypted = cfru_text(&sha1).replace("encrypted = false", "encrypted = true");
    assert!(refused(encrypted).contains("compressed"), "an encrypted compressed PC");
    let wrong_size = cfru_text(&sha1).replace("box_slot_size = 58", "box_slot_size = 80");
    assert!(refused(wrong_size).contains("58"), "a compressed slot is 58 bytes");
    let too_few = cfru_text(&sha1).replace("pc_boxes = 25", "pc_boxes = 26");
    assert!(refused(too_few).contains("pc_box_starts"), "a start for every box");
}

#[test]
fn only_a_level_cap_site_may_be_written_below_the_cap() {
    let sha1 = sha1_hex(&cfru_rom());
    let text = cfru_text(&sha1).replace(
        r#"write = "01 20 70 47" }]"#,
        r#"write = "01 20 70 47", below_cap = 1 }]"#,
    );
    let error = ProfileStore::from_sources([("bad.toml", text)])
        .unwrap_err()
        .to_string();
    assert!(error.contains("below_cap"), "{error}");
}
