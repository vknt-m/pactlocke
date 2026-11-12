//! Seam 1: faint = Dead and the Dead box, played through the Run engine.

mod support;

use pactlocke_run::{FeedKind, Ruleset};
use support::*;

const BULBASAUR: u16 = 1;
const STARTER: u32 = 0xB0B0_0001;
const CAUGHT: u32 = 0xC0C0_0002;

fn faint_is_dead() -> Ruleset {
    Ruleset {
        first_encounter: true,
        faint_is_dead: true,
    }
}

/// A Player on Route 1 with a starter and a caught Pidgey.
fn player_with_two() -> Player {
    let mut player = Player::starts_a_run(faint_is_dead());
    player.game.party = vec![
        party_pokemon(BULBASAUR, STARTER, "BULBA"),
        party_pokemon(PIDGEY, CAUGHT, "PIDGEY"),
    ];
    player.walks_into(route_1());
    player
}

#[test]
fn a_pokemon_that_faints_in_battle_is_moved_to_the_dead_box_once_back_on_the_overworld() {
    let mut player = player_with_two();
    player.meets_wild(RATTATA);
    player.faints(STARTER);
    assert_eq!(player.party_pids(), [STARTER, CAUGHT], "not moved during the battle");

    player.leaves_battle();

    assert_eq!(player.party_pids(), [CAUGHT], "the party is compacted");
    assert_eq!(player.dead_box_pids(), [STARTER]);
    assert_eq!(
        player.feed(FeedKind::Death),
        [
            "Moved BULBA to box 14, the Dead box: Dead Pokémon can't be used again",
            "BULBA fainted against a wild RATTATA on Route 1, so BULBA is Dead",
        ]
    );
}

#[test]
fn the_graveyard_lists_each_dead_pokemon_and_what_killed_it() {
    let mut player = player_with_two();
    player.meets_wild(RATTATA);
    player.faints(STARTER);
    player.leaves_battle();
    player.clock.advance_ms(60_000);
    player.walks_into(route_2());
    player.meets_trainer(CHARMANDER);
    player.faints(CAUGHT);
    player.leaves_battle();

    let graves: Vec<(String, String, String)> = player
        .engine
        .view()
        .graveyard
        .iter()
        .map(|g| (g.name.clone(), g.species_name.clone(), g.killed_by.clone()))
        .collect();
    assert_eq!(
        graves,
        [
            (
                "PIDGEY".into(),
                "PIDGEY".into(),
                "a trainer's CHARMANDER on Route 2".into()
            ),
            ("BULBA".into(), "SPECIES 1".into(), "a wild RATTATA on Route 1".into()),
        ],
        "newest first"
    );
    assert_eq!(player.engine.view().graveyard[1].at_ms, START_MS);
}

#[test]
fn a_dead_pokemon_withdrawn_from_the_pc_goes_back_with_a_red_violation() {
    let mut player = player_with_two();
    player.meets_wild(RATTATA);
    player.faints(STARTER);
    player.leaves_battle();

    player.withdraws(STARTER);

    assert_eq!(player.party_pids(), [CAUGHT]);
    assert_eq!(player.dead_box_pids(), [STARTER]);
    assert_eq!(
        player.feed(FeedKind::Violation),
        ["Violation: BULBA is Dead but was back in the party, so it went back to box 14"]
    );
}

#[test]
fn a_dead_pokemon_is_moved_even_if_healed_before_the_overworld() {
    let mut player = player_with_two();
    player.meets_trainer(CHARMANDER);
    player.faints(STARTER);
    player.faints(CAUGHT);
    player.blacks_out();

    assert_eq!(player.party_pids(), Vec::<u32>::new());
    assert_eq!(player.dead_box_pids(), [STARTER, CAUGHT]);
    assert_eq!(player.feed(FeedKind::Violation), Vec::<String>::new());
}

#[test]
fn poison_on_the_overworld_kills_too() {
    let mut player = player_with_two();
    player.is_poisoned_to_faint(CAUGHT);

    assert_eq!(player.party_pids(), [STARTER]);
    assert!(
        player
            .feed(FeedKind::Death)
            .contains(&"PIDGEY fainted from poison on Route 1, so PIDGEY is Dead".to_owned())
    );
}

#[test]
fn with_faint_dead_off_a_fainted_pokemon_stays_in_the_party() {
    let mut player = Player::starts_a_run(Ruleset {
        first_encounter: true,
        faint_is_dead: false,
    });
    player.game.party = vec![party_pokemon(BULBASAUR, STARTER, "BULBA")];
    player.meets_wild(RATTATA);
    player.faints(STARTER);
    player.leaves_battle();

    assert_eq!(player.party_pids(), [STARTER]);
    assert!(player.engine.view().graveyard.is_empty());
}

#[test]
fn a_full_dead_box_sends_the_pokemon_to_another_box_with_a_red_violation() {
    let mut player = player_with_two();
    player
        .game
        .pc
        .extend((1..=30).map(|slot| pactlocke_game::StoredPokemon {
            box_number: 14,
            slot,
            pokemon: party_pokemon(PIDGEY, 0xDEAD_0000 + u32::from(slot), "OLD"),
        }));
    player.meets_wild(RATTATA);
    player.faints(STARTER);
    player.leaves_battle();
    player.plays(100);

    assert_eq!(player.party_pids(), [CAUGHT], "out of the party all the same");
    assert_eq!(
        player.feed(FeedKind::Violation),
        ["Violation: BULBA is Dead but the Dead box is full, so it went to box 13"]
    );
}

#[test]
fn the_dead_stay_dead_after_a_reboot_from_a_save_made_before_they_died() {
    let mut player = player_with_two();
    player.meets_wild(RATTATA);
    player.faints(STARTER);
    player.leaves_battle();
    let (storage, clock) = (player.storage.clone(), player.clock.clone());
    drop(player);

    // The in-game save is from before the faint, so the starter is in the party again.
    let mut player = Player::resumes(storage, clock);
    player.game.party = vec![
        party_pokemon(BULBASAUR, STARTER, "BULBA"),
        party_pokemon(PIDGEY, CAUGHT, "PIDGEY"),
    ];
    player.plays(1);

    assert_eq!(player.party_pids(), [CAUGHT]);
    assert_eq!(player.dead_box_pids(), [STARTER]);
    assert_eq!(player.engine.view().graveyard.len(), 1);
    assert_eq!(
        player.feed(FeedKind::Violation),
        Vec::<String>::new(),
        "the save isn't the Player's fault"
    );
}

#[test]
fn a_death_in_a_town_reads_in_the_town() {
    let mut player = Player::starts_a_run(faint_is_dead());
    player.game.party = vec![party_pokemon(BULBASAUR, STARTER, "BULBA")];
    player.meets_trainer(CHARMANDER);
    player.faints(STARTER);

    assert_eq!(
        player.engine.view().graveyard[0].killed_by,
        "a trainer's CHARMANDER in Pallet Town"
    );
}

#[test]
fn a_dead_pokemon_waits_in_the_party_until_a_running_script_ends() {
    let mut player = player_with_two();
    player.meets_trainer(CHARMANDER);
    player.faints(STARTER);
    player.game.battle = None;
    player.script_runs();
    assert_eq!(
        player.party_pids(),
        [STARTER, CAUGHT],
        "a script may be using party slots"
    );

    player.script_ends();
    assert_eq!(player.party_pids(), [CAUGHT]);
}
