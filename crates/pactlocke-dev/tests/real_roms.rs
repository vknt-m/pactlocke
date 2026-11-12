//! The scanner against the developer's own ROMs, local only: FireRed read
//! against its own profile finds everything where the profile has it, and
//! LeafGreen read against FireRed's drafts a profile that loads. Skipped
//! unless `PACTLOCKE_ROMS` names a folder holding the ROMs.

use std::path::PathBuf;

use pactlocke_dev::scan;
use pactlocke_game::{ProfileStore, decode_text};

fn rom(file: &str) -> Option<Vec<u8>> {
    let dir = PathBuf::from(std::env::var_os("PACTLOCKE_ROMS")?);
    std::fs::read(dir.join(file)).ok()
}

fn firered() -> std::sync::Arc<pactlocke_game::GameProfile> {
    ProfileStore::bundled()
        .unwrap()
        .by_id("firered-usa-1.0")
        .unwrap()
        .clone()
}

#[test]
fn firered_read_against_its_own_profile_finds_everything_where_the_profile_has_it() {
    let Some(rom) = rom("Pokemon - FireRed Version (USA).gba") else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own FireRed (USA)");
        return;
    };
    let base = firered();
    let findings = scan(&rom, &base);

    for table in &findings.tables {
        assert_eq!(table.found, Some(table.base), "{}", table.symbol);
    }
    assert!(
        findings.patch_sites.iter().all(|s| s.unchanged),
        "{:?}",
        findings.patch_sites
    );
    let catalogued: Vec<u32> = base.sources.iter().flat_map(|s| s.at.clone()).collect();
    for at in &catalogued {
        let found = findings.sources.iter().find(|s| s.at == *at);
        assert!(found.is_some_and(|s| s.catalogued.is_some()), "{at:#010X} isn't found");
    }
    // Brock's and Lorelei's trainerbattle.
    let battles = |trainer: u16| {
        findings
            .gates
            .iter()
            .find(|g| g.trainer == trainer)
            .unwrap()
            .battles
            .clone()
    };
    assert!(battles(414).contains(&0x0816_A5A0), "Brock");
    assert!(battles(410).contains(&0x0816_2653), "Lorelei");
}

#[test]
fn leafgreen_read_against_firered_drafts_a_profile_that_loads() {
    let Some(rom) = rom("Pokemon - LeafGreen Version (USA).gba") else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own LeafGreen (USA)");
        return;
    };
    let findings = scan(&rom, &firered());
    let species_names = findings.tables[0].found.expect("LeafGreen's species names");
    let at = (species_names - 0x0800_0000) as usize + 11;
    assert_eq!(decode_text(&rom[at..at + 11]), "BULBASAUR");

    let dir = std::env::temp_dir().join(format!("pactlocke-leafgreen-draft-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("draft.toml"), findings.draft()).unwrap();
    let store = ProfileStore::bundled_with_local(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let draft = store.find(&findings.sha1).expect("the draft loads");
    assert_eq!(draft.game_code, "BPGE");
    assert_eq!(draft.symbols.species_names, species_names);
}
