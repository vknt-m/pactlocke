//! A Run under Faint = Dead ends in a game over when a Player has
//! no usable Pokémon (every Pokémon in their party is Dead), caught at the
//! Death that leaves the party with none, before the game can softlock.

mod support;

use pactlocke_emu::Checkpoint;
use pactlocke_run::{FeedKind, LogBody, Ruleset, RunEvent};
use support::streams::Table;
use support::*;

const BULBASAUR: u16 = 1;
const STARTER: u32 = 0xB0B0_0001;
const CAUGHT: u32 = 0xC0C0_0002;

fn nuzlocke(retries: u8) -> Ruleset {
    Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        retries,
        ..Ruleset::default()
    }
}

/// A Player on Route 1 with a starter and a caught Pidgey, both in the Run log.
fn player_with_two(ruleset: Ruleset) -> Player {
    let mut player = Player::starts_a_run(ruleset);
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, CAUGHT);
    // As a party Pokémon, with its stats, once out of the battle.
    player.game.party = vec![
        party_pokemon(BULBASAUR, STARTER, "BULBA"),
        party_pokemon(PIDGEY, CAUGHT, "PIDGEY"),
    ];
    player.plays(10);
    player
}

/// A Run Checkpoint, after the boot checkpoint every boot takes first.
fn checkpoint(player: &mut Player) {
    let core = || Checkpoint {
        save_memory: vec![1],
        state: vec![2],
    };
    player.engine.take_checkpoint(core());
    player.engine.want_checkpoint();
    assert!(player.engine.take_checkpoint(core()), "a Run Checkpoint");
}

/// Both of the Player's Pokémon faint to a trainer's Charmander on Route 2.
fn wiped(player: &mut Player) {
    player.walks_into(route_2());
    player.meets_trainer(CHARMANDER);
    player.faints(STARTER);
    player.clock.advance_ms(60_000);
    player.faints(CAUGHT);
}

#[test]
fn a_wiped_party_with_no_retries_is_a_game_over_with_the_dead_still_in_the_party() {
    let mut player = player_with_two(nuzlocke(0));
    wiped(&mut player);
    let view = player.engine.view().clone();
    let over = view.game_over.expect("a game over");
    assert_eq!(
        over.how,
        "Your party was wiped on Route 2: PIDGEY fainted against a trainer's CHARMANDER"
    );
    assert!(player.engine.halted(), "the game goes no further");
    // The game never runs on with an empty party: nothing more is moved.
    player.leaves_battle();
    assert_eq!(player.party_pids(), [STARTER, CAUGHT]);
    assert!(
        player.feed(FeedKind::Death).iter().all(|e| !e.starts_with("Moved")),
        "{:?}",
        player.feed(FeedKind::Death)
    );
}

#[test]
fn with_a_retry_left_it_is_offered_before_the_game_over() {
    let mut player = player_with_two(nuzlocke(1));
    checkpoint(&mut player);
    wiped(&mut player);
    let wipe = player.engine.view().wipe.clone().expect("the wipe is shown");
    assert!(wipe.retry_offered);
    assert!(
        player.engine.view().game_over.is_none(),
        "not yet: a Retry is offered first"
    );
    assert!(player.engine.halted());

    player.engine.end_run();
    assert!(player.engine.view().game_over.is_some());
}

#[test]
fn with_no_run_checkpoint_to_go_back_to_a_retry_left_is_no_help() {
    let mut player = player_with_two(nuzlocke(1));
    wiped(&mut player);
    assert!(player.engine.view().game_over.is_some());
}

#[test]
fn without_faint_dead_a_whiteout_ends_nothing() {
    let mut player = player_with_two(Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    });
    wiped(&mut player);
    player.blacks_out();
    assert!(player.engine.view().game_over.is_none());
    assert!(player.engine.view().wipe.is_none());
    assert!(!player.engine.halted());
}

#[test]
fn one_living_pokemon_left_is_no_wipe() {
    let mut player = player_with_two(nuzlocke(0));
    player.walks_into(route_2());
    player.meets_trainer(CHARMANDER);
    player.faints(STARTER);
    player.leaves_battle();
    assert!(player.engine.view().game_over.is_none());
    assert_eq!(player.party_pids(), [CAUGHT], "the Dead go to the Dead box as usual");
}

#[test]
fn the_pokemon_that_survived_longest_is_worked_out_from_the_log() {
    let mut player = player_with_two(nuzlocke(0));
    wiped(&mut player);
    let over = player.engine.view().game_over.clone().unwrap();
    let longest = over.longest.first().expect("the Player's longest survivor");
    assert_eq!(longest.name, "PIDGEY");
    assert_eq!(longest.lived_ms, 60_000, "from its catch to its Death");
}

fn soul_link(retries: u8) -> Ruleset {
    Ruleset {
        links: true,
        ..nuzlocke(retries)
    }
}

fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

/// Red catches a Pidgey on Route 1, Blue a Rattata, so they are linked;
/// then Red's Pidgey faints in a battle, and both parties are wiped.
fn red_wipes(table: &mut Table) {
    both_play(table, 100);
    for i in 0..2 {
        table.walks_into(i, route_1());
    }
    table.meets_wild(0, PIDGEY);
    table.catches(0, PIDGEY, 0xA1);
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB1);
    both_play(table, 120);
    table.meets_trainer(0, CHARMANDER);
    table.faints(0, 0xA1);
    both_play(table, 200);
}

#[test]
fn with_friends_one_players_wipe_is_a_game_over_for_everyone() {
    let mut table = Table::start(&["Red", "Blue"], soul_link(0));
    red_wipes(&mut table);
    // Red's game stopped at the Death, and its last frames still reached
    // Blue's Shadow of Red: Blue's Link-mate died with it.
    assert_eq!(
        table.tallies(1).deaths.get(&support::streams::key(1)),
        Some(&1),
        "{:?}",
        table.apps[1].view_feed()
    );
    let stopped_at = table.apps[0].running.engine().frames_run();
    let reached = table.apps[1]
        .hub
        .received(&support::streams::key(0))
        .map(|(_, frame)| frame);
    assert_eq!(
        reached,
        Some(stopped_at),
        "Red's stream reaches where Red's game stopped"
    );
    for (i, app) in table.apps.iter().enumerate() {
        let over = app.running.engine().view().game_over.clone();
        let over = over.unwrap_or_else(|| panic!("{i}: a game over: {:?}", app.view_feed()));
        // Written to each Player: Red reads it as their own.
        let whose = if i == 0 { "Your" } else { "Red's" };
        assert!(
            over.how.starts_with(&format!("{whose} party was wiped on Route 1")),
            "{i}: {}",
            over.how
        );
        assert!(app.log.game_over().is_some(), "{i}: the Run log says so");
    }
}

#[test]
fn with_friends_and_a_retry_left_a_no_vote_brings_the_game_over() {
    let mut table = Table::start(&["Red", "Blue"], soul_link(1));
    both_play(&mut table, 60);
    table.saves(0);
    both_play(&mut table, 60);
    red_wipes(&mut table);
    assert!(table.apps[1].log.game_over().is_none(), "a Retry vote comes first");
    let wiped = table.apps[1].log.wiped().expect("the wipe is in the log");
    assert_eq!(wiped.player, support::streams::key(0));

    let call = table.apps[1]
        .log
        .latest_complete()
        .expect("a Run Checkpoint to vote for");
    table.submit(1, &LogBody::Event(RunEvent::RetryVote { call, yes: false }));
    both_play(&mut table, 100);
    for (i, app) in table.apps.iter().enumerate() {
        assert!(app.log.game_over().is_some(), "{i}");
        assert!(app.running.engine().view().game_over.is_some(), "{i}");
    }
}

#[test]
fn a_pokemon_released_or_still_alive_is_timed_to_its_release_or_the_end() {
    let caught = |pid, at_ms| RunEvent::Caught {
        area: None,
        pid,
        species: PIDGEY,
        species_name: "PIDGEY".into(),
        at_ms,
    };
    let events = [
        caught(1, 0),
        caught(2, 10_000),
        RunEvent::Released {
            pid: 1,
            species_name: "PIDGEY".into(),
            at_ms: 50_000,
        },
    ];
    let longest = pactlocke_run::survivors(events.iter().map(|e| (None, e)), 30_000);
    assert_eq!(longest.len(), 1);
    assert_eq!(longest[0].lived_ms, 50_000, "the released one, up to its release");
    assert!(!longest[0].alive, "released, so not alive at the end");
}
