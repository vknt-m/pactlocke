//! The scanner against the developer's own ROMs, local only: FireRed read
//! against its own profile finds everything where the profile has it, and
//! LeafGreen read against FireRed's drafts a profile that loads. Skipped
//! unless `PACTLOCKE_ROMS` names a folder holding the ROMs.

use std::path::PathBuf;

use pactlocke_dev::scan;
use pactlocke_game::{ProfileStore, SourceKind, decode_text};

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
    let Some(mut rom) = rom("Pokemon - LeafGreen Version (USA).gba") else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own LeafGreen (USA)");
        return;
    };
    // LeafGreen has its own bundled profile now: a draft for the same SHA-1
    // would be refused, so this is a Romhack of it that changed a free byte.
    rom[0x00F1_0000] = 0x00;
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

#[test]
fn each_original_game_read_against_its_own_profile_finds_its_whole_catalogue_and_little_else() {
    let store = ProfileStore::bundled().unwrap();
    for (file, id) in [
        ("Pokemon - FireRed Version (USA).gba", "firered-usa-1.0"),
        ("Pokemon - LeafGreen Version (USA).gba", "leafgreen-usa-1.0"),
        ("Pokemon - Emerald Version (USA, Europe).gba", "emerald-usa-1.0"),
        ("Pokemon - Ruby Version (USA).gba", "ruby-usa-1.0"),
        ("Pokemon - Sapphire Version (USA).gba", "sapphire-usa-1.0"),
    ] {
        let Some(rom) = rom(file) else {
            eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {file}");
            continue;
        };
        let base = store.by_id(id).unwrap();
        let findings = scan(&rom, base);
        for source in &base.sources {
            for at in &source.at {
                let found = findings.sources.iter().find(|s| s.at == *at);
                assert!(
                    found.is_some_and(|s| s.catalogued.as_deref() == Some(source.key.as_str())),
                    "{id}: {} at {at:#010X} isn't found",
                    source.key
                );
            }
        }
        let others: Vec<String> = findings
            .sources
            .iter()
            .filter(|s| s.catalogued.is_none())
            .map(|s| format!("{:#010X} {:?} {}", s.at, s.kind, s.species))
            .collect();
        eprintln!("{id}: {} found, not catalogued: {others:?}", findings.sources.len());
        // Event-ticket legendaries the catalogue leaves out, and little else.
        assert!(others.len() <= 2, "{id}: {others:?}");
    }
}

#[test]
fn glazed_and_gaia_read_against_their_base_games_list_their_own_gifts_and_statics() {
    let Some(dir) = std::env::var_os("PACTLOCKE_ROMHACKS") else {
        eprintln!("skipped: set PACTLOCKE_ROMHACKS to a folder with your patched Romhacks");
        return;
    };
    let store = ProfileStore::bundled().unwrap();
    for (folder, base) in [("Glazed", "emerald-usa-1.0"), ("GAIA", "firered-usa-1.0")] {
        let path = std::fs::read_dir(PathBuf::from(&dir).join(folder))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "gba"))
            .unwrap();
        let rom = std::fs::read(&path).unwrap();
        let findings = scan(&rom, store.by_id(base).unwrap());
        let count = |kind| findings.sources.iter().filter(|s| s.kind == kind).count();
        let (gifts, eggs, statics, starters) = (
            count(SourceKind::Gift),
            count(SourceKind::Egg),
            count(SourceKind::Static),
            count(SourceKind::Starter),
        );
        eprintln!("{folder}: {gifts} givemon, {eggs} giveegg, {statics} setwildbattle, {starters} starter specials");
        assert!(gifts > 0 && statics > 0, "{folder}: its own Gifts and Statics");
        assert!(
            findings
                .sources
                .iter()
                .all(|s| !s.species.to_string().contains(char::REPLACEMENT_CHARACTER)),
            "{folder}: every species named"
        );
    }
}
