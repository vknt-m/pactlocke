//! The scanner against a ROM built in memory and laid out like FireRed, with
//! some tables moved, as a Romhack might: what it finds, and that the draft
//! it writes loads as a local profile extending FireRed's, never as trusted.

use pactlocke_dev::scan;
use pactlocke_game::{ProfileStore, SupportStatus, encode_text};

const ROM_BASE: u32 = 0x0800_0000;
const MOVED_SPECIES_NAMES: u32 = 0x0830_0000;
const MOVED_TRAINERS: u32 = 0x0840_0000;
const PARTIES: u32 = 0x0850_0000;
const BROCK: u16 = 414;
const BROCKS_BATTLE: u32 = 0x0817_0000;
const NEW_GIFT: u32 = 0x0816_5000;
/// The FireRed catalogue's first Celadon Game Corner prize.
const PRIZE: u32 = 0x0816_CCD4;

fn put(rom: &mut [u8], addr: u32, bytes: &[u8]) {
    let at = (addr - ROM_BASE) as usize;
    rom[at..at + bytes.len()].copy_from_slice(bytes);
}

fn name(text: &str, len: usize) -> Vec<u8> {
    encode_text(text, len).unwrap()
}

/// FireRed's leaders, Elite Four and Champion share a class; others don't.
fn gate_ids() -> Vec<u16> {
    let mut ids = vec![350];
    ids.extend(410..=420);
    ids.extend(438..=440);
    ids
}

fn hack_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x100_0000];
    rom[0xA0..0xAC].copy_from_slice(b"POKEMON HACK");
    rom[0xAC..0xB0].copy_from_slice(b"BPRE");

    // Species names, moved.
    put(&mut rom, MOVED_SPECIES_NAMES + 11, &name("BULBASAUR", 11));
    put(&mut rom, MOVED_SPECIES_NAMES + 22, &name("IVYSAUR", 11));
    put(&mut rom, MOVED_SPECIES_NAMES + 133 * 11, &name("EEVEE", 11));
    // Type names, where FireRed has them.
    put(&mut rom, 0x0824_F1A0, &name("NORMAL", 7));
    put(&mut rom, 0x0824_F1A0 + 7, &name("FIGHT", 7));

    // A trainer table, moved: every entry a one-Pokémon party.
    for id in 1..450u32 {
        let entry = MOVED_TRAINERS + id * 40;
        let party = PARTIES + id * 8;
        let class = if gate_ids().contains(&(id as u16)) { 0x54 } else { 0x10 };
        put(&mut rom, entry, &[0, class]);
        put(&mut rom, entry + 32, &1u32.to_le_bytes());
        put(&mut rom, entry + 36, &party.to_le_bytes());
        put(&mut rom, party + 2, &10u16.to_le_bytes());
        put(&mut rom, party + 4, &16u16.to_le_bytes());
    }
    put(&mut rom, MOVED_TRAINERS + u32::from(BROCK) * 40 + 4, &name("BROCK", 12));
    // trainerbattle 1, BROCK, 0, <intro text>.
    put(&mut rom, BROCKS_BATTLE, &[0x5C, 0x01, 0x9E, 0x01, 0x00, 0x00]);
    put(&mut rom, BROCKS_BATTLE + 6, &0x0819_0000u32.to_le_bytes());

    // givemon EEVEE lv 10, no item, at a new place; and the prize where FireRed has it.
    put(&mut rom, NEW_GIFT, &[0x79, 133, 0, 10, 0, 0]);
    put(&mut rom, PRIZE, &[0x79, 0x01, 0x40, 9, 0, 0]);

    // One level-100 check where FireRed has it: movs r1, #0x38; …; cmp r0, #100.
    put(&mut rom, 0x0802_1CF4, &[0x38, 0x21, 0x00, 0xF0, 0x00, 0xF8, 0x64, 0x28]);
    rom
}

fn firered() -> std::sync::Arc<pactlocke_game::GameProfile> {
    ProfileStore::bundled()
        .unwrap()
        .by_id("firered-usa-1.0")
        .unwrap()
        .clone()
}

#[test]
fn moved_tables_are_found_by_their_contents() {
    let findings = scan(&hack_rom(), &firered());
    let found = |symbol: &str| findings.tables.iter().find(|t| t.symbol == symbol).unwrap().found;
    assert_eq!(found("species_names"), Some(MOVED_SPECIES_NAMES));
    assert_eq!(found("trainers"), Some(MOVED_TRAINERS));
    assert_eq!(found("type_names"), Some(0x0824_F1A0), "where FireRed has it");
    assert_eq!(found("items"), None, "not in this ROM");
}

#[test]
fn patch_sites_are_checked_and_level_cap_sites_found_by_pattern() {
    let findings = scan(&hack_rom(), &firered());
    let site = |at: u32| findings.patch_sites.iter().find(|s| s.at == at).unwrap();
    assert!(site(0x0802_1CFA).unchanged, "the level cap site");
    assert!(!site(0x0804_0F6C).unchanged, "the catch block's bytes aren't there");
    assert!(findings.level_cap_sites.contains(&0x0802_1CFA));
}

#[test]
fn gifts_are_listed_new_or_catalogued() {
    let findings = scan(&hack_rom(), &firered());
    let at = |addr: u32| findings.sources.iter().find(|s| s.at == addr);
    let eevee = at(NEW_GIFT).expect("a new givemon");
    assert_eq!((eevee.species.to_string(), eevee.level), ("EEVEE".to_owned(), Some(10)));
    assert_eq!(eevee.catalogued, None);
    let prize = at(PRIZE).expect("FireRed's prize");
    assert!(prize.catalogued.is_some(), "{prize:?}");
}

#[test]
fn sync_gate_candidates_are_the_cap_stages_classes_with_their_trainerbattles() {
    let findings = scan(&hack_rom(), &firered());
    let brock = findings
        .gates
        .iter()
        .find(|g| g.trainer == BROCK)
        .expect("Brock's class");
    assert_eq!(brock.name, "BROCK");
    assert_eq!(brock.battles, [BROCKS_BATTLE]);
    assert!(findings.gates.iter().all(|g| g.class == 0x54), "only the gate classes");
    assert_eq!(findings.gates.len(), gate_ids().len());
}

#[test]
fn the_draft_is_a_local_untested_profile_extending_the_base() {
    let rom = hack_rom();
    let draft = scan(&rom, &firered()).draft();
    assert!(draft.contains("Review every entry"), "{draft}");
    assert!(draft.contains("parent = \"firered-usa-1.0\""));

    let dir = std::env::temp_dir().join(format!("pactlocke-scan-draft-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("draft.toml"), &draft).unwrap();
    let store = ProfileStore::bundled_with_local(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let profile = store.find(&pactlocke_game::sha1_hex(&rom)).expect("the draft loads");
    assert!(profile.id.starts_with("draft-"));
    assert!(store.is_local(&profile.id), "a developer's local profile only");
    assert_eq!(profile.status, SupportStatus::Untested);
    assert_eq!(profile.symbols.species_names, MOVED_SPECIES_NAMES);
    assert_eq!(profile.symbols.trainers, MOVED_TRAINERS);
    assert_eq!(profile.symbols.party, firered().symbols.party, "the rest from FireRed");
}
