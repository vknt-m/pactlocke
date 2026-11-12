//! A developer's local, unbundled Game Profiles. They load next
//! to the bundled ones, are marked local (solo Runs only), may extend a
//! bundled profile but not copy one, and a draft is never bundled.

mod support;

use pactlocke_game::{ProfileError, ProfileStore};
use support::*;

const OTHER_SHA1: &str = "1111111111111111111111111111111111111111";

#[test]
fn a_local_profile_loads_next_to_the_bundled_ones_and_is_marked_local() {
    let bundled = [("test.toml", profile_toml("test-game", TEST_ROM_SHA1))];
    let local = [("mine.toml", profile_toml("my-hack", OTHER_SHA1))];
    let store = ProfileStore::from_sources_with_local(bundled, local).unwrap();

    assert!(store.find(TEST_ROM_SHA1).is_some());
    assert!(store.find(OTHER_SHA1).is_some());
    assert!(!store.is_local("test-game"));
    assert!(store.is_local("my-hack"));
}

#[test]
fn a_local_copy_of_a_bundled_profile_is_refused() {
    let bundled = [("test.toml", profile_toml("test-game", TEST_ROM_SHA1))];
    let edited =
        profile_toml("test-game", TEST_ROM_SHA1).replace("name = \"Test Game\"", "name = \"Test Game, edited\"");
    // A bundled profile is edited where it is, so a Run's profile id always
    // says whether it was played on a bundled or a local one.
    assert!(matches!(
        ProfileStore::from_sources_with_local(bundled, [("test.toml", edited)]),
        Err(ProfileError::Duplicate { .. })
    ));
}

#[test]
fn a_local_profile_may_extend_a_bundled_one() {
    let bundled = [("test.toml", profile_toml("test-game", TEST_ROM_SHA1))];
    let child = format!(
        r#"
schema = 1
id = "draft-test-hack"
name = "Test Hack (draft)"
game_code = "TEST"
sha1 = "{OTHER_SHA1}"
status = "untested"
parent = "test-game"

[symbols]
species_names = 0x08004800
"#
    );
    let store = ProfileStore::from_sources_with_local(bundled, [("draft.toml", child)]).unwrap();
    let hack = store.find(OTHER_SHA1).unwrap();
    assert_eq!(hack.symbols.species_names, 0x0800_4800, "its own");
    assert_eq!(hack.symbols.items, 0x0800_1000, "the rest from its parent");
}

#[test]
fn a_draft_is_never_a_bundled_profile() {
    let bundled = [("draft.toml", profile_toml("draft-test-game", TEST_ROM_SHA1))];
    let none: [(&str, &str); 0] = [];
    assert!(matches!(
        ProfileStore::from_sources_with_local(bundled, none),
        Err(ProfileError::Invalid { .. })
    ));
}

#[test]
fn local_profiles_are_read_from_a_folder() {
    let dir = std::env::temp_dir().join(format!("pactlocke-local-profiles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mine.toml"), profile_toml("my-hack", OTHER_SHA1)).unwrap();
    std::fs::write(dir.join("notes.txt"), "not a profile").unwrap();

    let store = ProfileStore::bundled_with_local(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(store.is_local("my-hack"));
    assert!(store.find(OTHER_SHA1).is_some());
}
