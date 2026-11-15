//! Seam 1: a solo Run with the first-Encounter rule, played
//! through the Run engine on the fake game.

mod support;

use pactlocke_game::FakeGame;
use pactlocke_run::{FeedKind, MemoryStorage, Ruleset};
use support::*;

fn first_encounter() -> Ruleset {
    Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    }
}

#[test]
fn the_first_wild_battle_in_an_area_is_its_encounter_and_catching_stays_on() {
    let mut player = Player::starts_a_run(first_encounter());

    player.walks_into(route_1());
    player.meets_wild(PIDGEY);

    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild PIDGEY, this Area's Encounter"]
    );
    assert!(!player.game.catches_blocked);
    assert!(player.feed(FeedKind::Block).is_empty());
}

#[test]
fn a_second_wild_battle_in_a_closed_area_blocks_catches_for_that_battle_only() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();

    player.meets_wild(RATTATA);
    assert!(player.game.catches_blocked, "the ball is refused in this battle");
    assert_eq!(
        player.feed(FeedKind::Block),
        ["Route 1: Encounter already used, so no catching this RATTATA (running is fine)"]
    );

    player.leaves_battle();
    assert!(
        !player.game.catches_blocked,
        "catching works again once the battle is over"
    );

    player.walks_into(route_2());
    player.meets_wild(RATTATA);
    assert!(!player.game.catches_blocked, "Route 2 still has its Encounter");
    assert_eq!(
        player.feed(FeedKind::Encounter)[0],
        "Route 2: a wild RATTATA, this Area's Encounter"
    );
}

#[test]
fn the_catch_block_is_stamped_with_the_frame_boundary_it_lands_at() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.plays(5);

    let before = player.engine.frames_run();
    player.game.battle = Some(wild_battle(RATTATA));
    player.plays(1);
    let on_at = before + 1;
    player.plays(29);
    player.game.battle = None;
    player.plays(1);
    let off_at = before + 31;

    let mut on = FakeGame::default();
    let stamped = player.engine.stamped();
    assert_eq!(stamped.len(), 2, "{stamped:?}");
    assert_eq!(stamped[0].frame, on_at, "on at the boundary the battle was first seen");
    assert_eq!(stamped[1].frame, off_at, "off at the boundary it was seen to end");
    on.apply(&[stamped[0].change.clone()]);
    assert!(on.catches_blocked);
    on.apply(&[stamped[1].change.clone()]);
    assert!(!on.catches_blocked);
}

#[test]
fn the_area_encounter_is_used_whether_or_not_it_is_caught() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, 0xABCD_0001);

    assert_eq!(player.feed(FeedKind::Catch), ["Caught PIDGEY on Route 1"]);

    player.meets_wild(RATTATA);
    assert!(player.game.catches_blocked, "a catch used the Encounter too");
    assert_eq!(player.engine.view().closed_areas(), ["Route 1"]);
}

#[test]
fn a_battle_a_script_started_counts_as_the_areas_encounter_when_it_comes_first() {
    // Without the Statics rule, a Static is only the Area's first Encounter.
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_scripted(PIDGEY);
    player.leaves_battle();

    player.meets_wild(RATTATA);
    assert!(player.game.catches_blocked);
    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild PIDGEY, this Area's Encounter"]
    );
}

#[test]
fn trainer_battles_neither_use_nor_check_the_encounter() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_trainer(RATTATA);
    assert!(!player.game.catches_blocked);
    player.leaves_battle();
    assert!(player.feed(FeedKind::Encounter).is_empty());

    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.meets_trainer(RATTATA);
    assert!(!player.game.catches_blocked, "trainer Pokémon can't be caught anyway");
}

#[test]
fn a_place_the_game_profile_does_not_list_has_no_encounter() {
    let mut player = Player::starts_a_run(first_encounter());
    player.game.area = None;
    player.plays(10);
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.meets_wild(PIDGEY);

    assert!(!player.game.catches_blocked);
    assert!(player.feed(FeedKind::Encounter).is_empty());
}

#[test]
fn without_the_first_encounter_rule_nothing_is_blocked() {
    let mut player = Player::starts_a_run(Ruleset {
        first_encounter: false,
        ..Ruleset::default()
    });
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.meets_wild(RATTATA);

    assert!(!player.game.catches_blocked);
    assert!(player.feed(FeedKind::Encounter).is_empty());
}

#[test]
fn the_encounter_is_the_wild_pokemon_not_the_last_battles_leftovers() {
    // At the start of a battle the battlers still hold the last battle's
    // Pokémon; the opponent's party already holds the new one.
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.game.opponent_party = vec![wild_pokemon(PIDGEY)];
    player.game.battle = Some(wild_battle(CHARMANDER));
    player.plays(10);
    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild PIDGEY, this Area's Encounter"]
    );
}

#[test]
fn the_rules_wait_until_the_opponent_is_known() {
    // The game sets "in battle" a little before it fills in the battlers.
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.game.battle = Some(wild_battle(PIDGEY));
    player.plays(3);
    assert!(player.feed(FeedKind::Encounter).is_empty());

    player.game.opponent_party = vec![wild_pokemon(PIDGEY)];
    player.plays(1);
    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild PIDGEY, this Area's Encounter"]
    );
}

#[test]
fn the_feed_starts_with_the_run_and_lists_newest_first() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, 7);

    let feed: Vec<&str> = player.engine.view().feed.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        feed,
        [
            "Caught PIDGEY on Route 1",
            "Route 1: a wild PIDGEY, this Area's Encounter",
            "You have Poké Balls: wild Encounters count from now on",
            "Run started on Pokémon FireRed (USA) 1.0",
        ]
    );
    assert_eq!(player.engine.view().feed[3].at_ms, START_MS);
}

#[test]
fn a_resumed_run_still_has_its_closed_areas_and_feed() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    let (storage, clock) = (player.storage.clone(), player.clock.clone());
    drop(player);

    clock.advance_ms(3_600_000);
    let mut player = Player::resumes(storage, clock);
    assert_eq!(player.engine.view().name, "Kanto solo");
    assert_eq!(
        player.feed(FeedKind::Run),
        [
            "Run resumed from the start: the game hasn't been saved yet",
            "You have Poké Balls: wild Encounters count from now on",
            "Run started on Pokémon FireRed (USA) 1.0"
        ]
    );
    assert_eq!(player.engine.view().closed_areas(), ["Route 1"]);

    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    assert!(player.game.catches_blocked, "Route 1 stays closed after a resume");
}

#[test]
fn every_frame_and_stamped_change_reaches_storage_within_a_second() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.meets_wild(RATTATA);
    // Each event writes the presses up to it, so a crash rebuild reaches it;
    // quiet frames after it wait for the second.
    let at_last_event = player.engine.view_version();
    player.plays(5);
    assert_eq!(player.engine.view_version(), at_last_event, "no events in those frames");
    let written: usize = player.storage.load_inputs().iter().map(|b| b.keys.len()).sum();
    let last_event = player.storage.load_run().events.last().unwrap().frame;
    assert_eq!(written as u64, last_event, "written up to the last event");
    assert!(
        last_event < player.engine.frames_run(),
        "and not after it before a second has passed"
    );

    player.clock.advance_ms(1000);
    player.plays(1);

    let batches = player.storage.load_inputs();
    let frames: usize = batches.iter().map(|b| b.keys.len()).sum();
    assert_eq!(frames as u64, player.engine.frames_run());
    let stamped: Vec<_> = batches.iter().flat_map(|b| b.stamped.clone()).collect();
    assert_eq!(stamped, player.engine.stamped());
}

#[test]
fn a_run_file_that_cannot_be_written_is_shown_in_the_feed() {
    let storage = MemoryStorage::default();
    let mut player = Player::starts_a_run_in(Box::new(storage.clone()), TestClock::at(START_MS), first_encounter());
    storage.fail_writes("disk full");
    player.clock.advance_ms(1000);
    player.plays(1);
    player.clock.advance_ms(1000);
    player.plays(1);

    assert_eq!(
        player.feed(FeedKind::Problem),
        ["The Run file can't be written, so this Run may not resume as it is now: disk full"],
        "said once"
    );
}

#[test]
fn wild_battles_before_the_player_has_poke_balls_do_not_count() {
    let mut player = Player::starts_a_run(first_encounter());
    player.game.holds_poke_balls = false;
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();

    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild PIDGEY, not counted: no Poké Balls yet"]
    );
    assert!(player.engine.view().closed_areas().is_empty());

    // Oak hands over Poké Balls: from then on Encounters count.
    player.game.holds_poke_balls = true;
    player.plays(10);
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "You have Poké Balls: wild Encounters count from now on"
    );
    player.meets_wild(RATTATA);
    assert!(!player.game.catches_blocked, "Route 1 still has its Encounter");
    assert_eq!(
        player.feed(FeedKind::Encounter)[0],
        "Route 1: a wild RATTATA, this Area's Encounter"
    );
}

#[test]
fn encounters_keep_counting_after_the_last_ball_is_thrown() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();

    player.game.holds_poke_balls = false;
    player.meets_wild(RATTATA);
    assert!(player.game.catches_blocked, "out of balls doesn't reopen Route 1");
    player.leaves_battle();
    player.walks_into(route_2());
    player.meets_wild(RATTATA);
    assert_eq!(
        player.feed(FeedKind::Encounter)[0],
        "Route 2: a wild RATTATA, this Area's Encounter"
    );
}

#[test]
fn having_poke_balls_is_remembered_when_a_run_is_resumed() {
    let mut player = Player::starts_a_run(first_encounter());
    player.plays(1);
    let (storage, clock) = (player.storage.clone(), player.clock.clone());
    drop(player);

    let mut player = Player::resumes(storage, clock);
    player.game.holds_poke_balls = false;
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild PIDGEY, this Area's Encounter"]
    );
}

#[test]
fn poke_balls_are_only_looked_for_on_the_overworld() {
    // Before the title screen is passed, the save the Bag lives in holds garbage.
    let mut player = Player::starts_a_run(first_encounter());
    player.game.safe_point = false;
    player.plays(10);
    assert!(!player.feed(FeedKind::Run)[0].starts_with("You have Poké Balls"));

    player.game.safe_point = true;
    player.plays(1);
    assert!(player.feed(FeedKind::Run)[0].starts_with("You have Poké Balls"));
}

#[test]
fn each_areas_encounter_keeps_what_was_met_there_and_whether_it_was_caught() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, 0xABCD_0001);
    player.walks_into(route_2());
    player.meets_wild(RATTATA);
    player.leaves_battle();

    let met: Vec<_> = player
        .engine
        .view()
        .encounters
        .iter()
        .map(|e| (e.area.clone(), e.species, e.species_name.clone(), e.caught))
        .collect();
    assert_eq!(
        met,
        [
            ("Route 1".to_owned(), PIDGEY, "PIDGEY".to_owned(), true),
            ("Route 2".to_owned(), RATTATA, "RATTATA".to_owned(), false),
        ]
    );
    assert_eq!(player.engine.view().closed_areas(), ["Route 1", "Route 2"]);
}
