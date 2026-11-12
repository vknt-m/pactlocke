//! Seam 1: the dupes clause, played through the Run engine.

mod support;

use pactlocke_run::{FeedKind, Ruleset};
use support::*;

const PIDGEOTTO: u16 = 17;
const RATICATE: u16 = 20;
const OWNED_PIDGEY: u32 = 0xB1BD_0001;

fn dupes_clause() -> Ruleset {
    Ruleset {
        first_encounter: true,
        dupes_clause: true,
        ..Ruleset::default()
    }
}

/// A Player on Route 1 who already has a Pidgey, in a game where Pidgey
/// evolves into Pidgeotto and Rattata into Raticate.
fn player_with_a_pidgey(ruleset: Ruleset) -> Player {
    let mut player = Player::starts_a_run(ruleset);
    player.game.species_names.insert(PIDGEOTTO, "PIDGEOTTO".into());
    player.game.species_names.insert(RATICATE, "RATICATE".into());
    player.game.evolutions.insert(PIDGEY, vec![PIDGEOTTO]);
    player.game.evolutions.insert(RATTATA, vec![RATICATE]);
    player.game.party = vec![party_pokemon(PIDGEY, OWNED_PIDGEY, "BIRDY")];
    player.walks_into(route_1());
    player
}

#[test]
fn a_wild_pokemon_from_a_family_already_owned_is_skipped_and_the_next_one_counts() {
    let mut player = player_with_a_pidgey(dupes_clause());

    player.meets_wild(PIDGEOTTO);
    assert!(player.game.catches_blocked, "a dupe can't be caught");
    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild PIDGEOTTO, not counted: a dupe of your PIDGEY, so no catching it (running is fine)"]
    );
    player.leaves_battle();
    assert!(!player.game.catches_blocked);
    assert!(player.engine.view().closed_areas.is_empty(), "Route 1 is still open");

    player.meets_wild(RATTATA);
    assert!(!player.game.catches_blocked);
    assert_eq!(
        player.feed(FeedKind::Encounter)[0],
        "Route 1: a wild RATTATA, this Area's Encounter"
    );
}

#[test]
fn a_family_caught_earlier_still_counts_once_it_is_gone_from_party_and_pc() {
    let mut player = player_with_a_pidgey(dupes_clause());
    player.game.party.clear();
    player.walks_into(route_2());
    player.meets_wild(RATTATA);
    player.catches(RATTATA, 0x0000_0BAD);
    // Released, or Dead and moved off: it is no longer in the game at all.
    player.game.party.clear();
    let (storage, clock) = (player.storage.clone(), player.clock.clone());
    drop(player);

    let mut player = Player::resumes(storage, clock);
    player.game.species_names.insert(RATICATE, "RATICATE".into());
    player.game.evolutions.insert(RATTATA, vec![RATICATE]);
    player.walks_into(route_1());
    player.meets_wild(RATICATE);
    assert!(player.game.catches_blocked);
    assert!(player.feed(FeedKind::Encounter)[0].contains("a dupe of your RATTATA"));
}

/// Every species evolves into some other one, as a randomizer's "random every level" makes it.
fn evolutions_random_every_level(player: &mut Player) {
    for species in 1..=30 {
        player.game.evolutions.insert(species, vec![(species % 30) + 1]);
    }
}

#[test]
fn a_baby_and_its_evolved_forms_are_one_family() {
    const PIKACHU: u16 = 25;
    const RAICHU: u16 = 26;
    const PICHU: u16 = 172;
    let mut player = player_with_a_pidgey(dupes_clause());
    player.game.species_names.insert(PICHU, "PICHU".into());
    player.game.species_names.insert(RAICHU, "RAICHU".into());
    player.game.evolutions.insert(PICHU, vec![PIKACHU]);
    player.game.evolutions.insert(PIKACHU, vec![RAICHU]);
    player.game.party.push(party_pokemon(RAICHU, 0x0000_2626, "SPARKY"));

    player.meets_wild(PICHU);
    assert!(player.game.catches_blocked);
    assert!(player.feed(FeedKind::Encounter)[0].contains("a dupe of your RAICHU"));
}

#[test]
fn with_evolutions_random_every_level_only_the_exact_species_is_a_dupe() {
    let mut player = player_with_a_pidgey(dupes_clause());
    evolutions_random_every_level(&mut player);

    player.meets_wild(PIDGEOTTO);
    assert!(!player.game.catches_blocked, "PIDGEOTTO isn't PIDGEY");
    assert_eq!(
        player.feed(FeedKind::Encounter)[0],
        "Route 1: a wild PIDGEOTTO, this Area's Encounter"
    );
}

#[test]
fn the_same_species_is_a_dupe_with_random_evolutions_too() {
    let mut player = player_with_a_pidgey(dupes_clause());
    evolutions_random_every_level(&mut player);

    player.meets_wild(PIDGEY);
    assert!(player.game.catches_blocked);
}

#[test]
fn a_scripted_battle_with_a_dupe_is_left_alone() {
    let mut player = player_with_a_pidgey(dupes_clause());
    player.meets_scripted(PIDGEOTTO);
    assert!(!player.game.catches_blocked);
    assert!(player.feed(FeedKind::Encounter).is_empty());
}

#[test]
fn the_dupes_clause_does_nothing_without_first_encounter() {
    let ruleset = Ruleset {
        first_encounter: false,
        dupes_clause: true,
        ..Ruleset::default()
    };
    let mut player = player_with_a_pidgey(ruleset);
    player.meets_wild(PIDGEOTTO);
    assert!(!player.game.catches_blocked);
    assert!(player.feed(FeedKind::Encounter).is_empty());
}

#[test]
fn without_the_dupes_clause_a_dupe_is_the_encounter() {
    let mut player = player_with_a_pidgey(Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    });
    player.meets_wild(PIDGEOTTO);
    assert!(!player.game.catches_blocked);
    assert_eq!(
        player.feed(FeedKind::Encounter)[0],
        "Route 1: a wild PIDGEOTTO, this Area's Encounter"
    );
}

#[test]
fn a_closed_area_stays_closed_to_a_dupe() {
    let mut player = player_with_a_pidgey(dupes_clause());
    player.meets_wild(RATTATA);
    player.leaves_battle();
    player.meets_wild(PIDGEOTTO);
    assert!(player.game.catches_blocked);
    assert!(player.feed(FeedKind::Block)[0].starts_with("Route 1: Encounter already used"));
}
