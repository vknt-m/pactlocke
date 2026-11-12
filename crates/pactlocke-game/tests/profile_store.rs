//! Seam 1: the Game Profile store, with test profiles and ROMs
//! built in memory.

mod support;

use pactlocke_game::{Method, ProfileError, ProfileStore, RomIdentity, SupportStatus, Unprofiled};
use support::{TEST_ROM_SHA1, profile_toml, test_rom_bytes};

#[test]
fn a_rom_is_found_by_its_sha1() {
    let store = ProfileStore::from_sources([("test.toml", profile_toml("test-game", TEST_ROM_SHA1))]).unwrap();

    match store.identify(&test_rom_bytes()) {
        RomIdentity::Profiled { profile, .. } => assert_eq!(profile.id, "test-game"),
        other => panic!("expected the test profile, got {other:?}"),
    }
}

#[test]
fn every_bundled_profile_loads_with_its_parent_resolved() {
    // The CI check: format, schema version and parent links, with no ROM.
    let store = ProfileStore::bundled().expect("every bundled profile is valid");
    let firered = store
        .find("41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc")
        .expect("FireRed (USA) 1.0 is bundled");
    assert_eq!(firered.name, "Pokémon FireRed (USA) 1.0");
    assert_eq!(firered.status, SupportStatus::Supported);
}

#[test]
fn a_child_profile_inherits_its_parent_and_overrides_what_it_sets() {
    let child = format!(
        r#"
schema = 1
id = "test-hack"
name = "Test Hack"
game_code = "TEST"
sha1 = "{}"
status = "untested"
parent = "test-game"

[symbols]
party = 0x02030000

[area_renames]
0x65 = "Route One"
"#,
        "ab".repeat(20)
    );
    let store = ProfileStore::from_sources([
        ("hack.toml", child),
        ("base.toml", profile_toml("test-game", TEST_ROM_SHA1)),
    ])
    .unwrap();

    let hack = store.find(&"ab".repeat(20)).unwrap();
    assert_eq!(hack.name, "Test Hack");
    assert_eq!(hack.status, SupportStatus::Untested);
    assert_eq!(hack.symbols.party, 0x0203_0000, "overridden");
    assert_eq!(hack.symbols.party_count, 0x0202_4029, "inherited");
    assert_eq!(hack.area(0x65).unwrap().name, "Route One");
    assert_eq!(hack.area(0x58).unwrap().name, "Pallet Town");
}

#[test]
fn a_profile_may_not_extend_a_profile_that_has_a_parent() {
    let child = |id: &str, parent: &str, sha1: &str| {
        format!(
            "schema = 1\nid = \"{id}\"\nname = \"x\"\ngame_code = \"TEST\"\nsha1 = \"{sha1}\"\nstatus = \"untested\"\nparent = \"{parent}\"\n"
        )
    };
    let result = ProfileStore::from_sources([
        ("base.toml", profile_toml("base", TEST_ROM_SHA1)),
        ("middle.toml", child("middle", "base", &"11".repeat(20))),
        ("leaf.toml", child("leaf", "middle", &"22".repeat(20))),
    ]);
    assert_eq!(
        result.unwrap_err(),
        ProfileError::ParentChain {
            file: "leaf.toml".into(),
            parent: "middle".into()
        }
    );
}

#[test]
fn a_profile_naming_a_missing_parent_is_refused() {
    let orphan = format!(
        "schema = 1\nid = \"orphan\"\nname = \"x\"\ngame_code = \"TEST\"\nsha1 = \"{}\"\nstatus = \"untested\"\nparent = \"nobody\"\n",
        "33".repeat(20)
    );
    assert_eq!(
        ProfileStore::from_sources([("orphan.toml", orphan)]).unwrap_err(),
        ProfileError::UnknownParent {
            file: "orphan.toml".into(),
            parent: "nobody".into()
        }
    );
}

#[test]
fn a_profile_for_another_schema_version_is_refused() {
    let future = profile_toml("future", TEST_ROM_SHA1).replace("schema = 1", "schema = 2");
    assert_eq!(
        ProfileStore::from_sources([("future.toml", future)]).unwrap_err(),
        ProfileError::Schema {
            file: "future.toml".into(),
            found: 2
        }
    );
}

#[test]
fn a_misspelt_field_is_refused_rather_than_ignored() {
    let typo = profile_toml("typo", TEST_ROM_SHA1).replace("party_count =", "party_cuont =");
    assert!(matches!(
        ProfileStore::from_sources([("typo.toml", typo)]).unwrap_err(),
        ProfileError::Format { file, .. } if file == "typo.toml"
    ));
}

#[test]
fn a_patch_that_writes_a_different_length_than_it_expects_is_refused() {
    let bad = profile_toml("bad", TEST_ROM_SHA1).replace("write = \"01 20 70 47\"", "write = \"01 20\"");
    assert!(matches!(
        ProfileStore::from_sources([("bad.toml", bad)]).unwrap_err(),
        ProfileError::Invalid { .. }
    ));
}

#[test]
fn merged_sections_count_as_one_area() {
    let merged = format!(
        "{}\n[area_merges]\n0x82 = 0x81\n",
        profile_toml("test-game", TEST_ROM_SHA1)
    );
    let store = ProfileStore::from_sources([("test.toml", merged)]).unwrap();
    let profile = store.find(TEST_ROM_SHA1).unwrap();
    assert_eq!(profile.area(0x82), profile.area(0x81));
    assert_eq!(profile.area(0x8C).unwrap().name, "Pokémon Tower", "renamed");
    assert_eq!(profile.area(0x99), None, "not a listed section");
}

#[test]
fn an_unknown_rom_of_a_supported_game_is_untested_and_anything_else_is_not_supported() {
    let store = ProfileStore::from_sources([("test.toml", profile_toml("test-game", TEST_ROM_SHA1))]).unwrap();

    let hack = support::patchable_rom(b"BPRE");
    assert_eq!(
        store.identify(&hack),
        RomIdentity::Unprofiled {
            sha1: pactlocke_game::sha1_hex(&hack),
            verdict: Unprofiled::Untested
        }
    );
    let homebrew = support::patchable_rom(b"ZPAC");
    assert!(matches!(
        store.identify(&homebrew),
        RomIdentity::Unprofiled {
            verdict: Unprofiled::NotSupported,
            ..
        }
    ));
}

#[test]
fn a_child_that_changes_a_rules_method_replaces_the_parents_entry() {
    let child = format!(
        "schema = 1\nid = \"hack\"\nname = \"x\"\ngame_code = \"TEST\"\nsha1 = \"{}\"\nstatus = \"untested\"\nparent = \"test-game\"\n\n[rules.catch_block]\nmethod = \"native\"\noption = \"battle_style\"\nvalue = \"set\"\n",
        "44".repeat(20)
    );
    let store = ProfileStore::from_sources([
        ("base.toml", profile_toml("test-game", TEST_ROM_SHA1)),
        ("hack.toml", child),
    ])
    .expect("the parent's patch sites don't leak into the child's native method");
    let hack = store.find(&"44".repeat(20)).unwrap();
    assert!(matches!(hack.rules.catch_block, Some(Method::Native { .. })));
}

#[test]
fn a_profile_without_a_schema_says_so() {
    let unversioned = profile_toml("old", TEST_ROM_SHA1).replace("schema = 1\n", "");
    assert_eq!(
        ProfileStore::from_sources([("old.toml", unversioned)]).unwrap_err(),
        ProfileError::MissingOwnField {
            file: "old.toml".into(),
            field: "schema"
        }
    );
}

#[test]
fn a_switched_rule_must_say_what_it_writes() {
    let silent = profile_toml("silent", TEST_ROM_SHA1).replace(", write = \"01 20 70 47\"", "");
    assert!(matches!(
        ProfileStore::from_sources([("silent.toml", silent)]).unwrap_err(),
        ProfileError::Invalid { message, .. } if message.contains("catch_block")
    ));
}

#[test]
fn rom_tables_must_be_in_rom() {
    let misplaced = profile_toml("misplaced", TEST_ROM_SHA1).replace("items = 0x08001000", "items = 0x02001000");
    assert!(matches!(
        ProfileStore::from_sources([("misplaced.toml", misplaced)]).unwrap_err(),
        ProfileError::Invalid { message, .. } if message.contains("items")
    ));
}

#[test]
fn firered_counts_a_routes_pokemon_center_as_the_route() {
    let store = ProfileStore::bundled().unwrap();
    let firered = store.find("41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc").unwrap();
    assert_eq!(firered.area(0x63), firered.area(0x68));
    assert_eq!(firered.area(0x63).unwrap().name, "Route 4");
    assert_eq!(firered.area(0x82), firered.area(0x81));
}
