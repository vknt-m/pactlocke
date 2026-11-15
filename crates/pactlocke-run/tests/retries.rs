//! Seam 1: Run Checkpoints at every in-game save, taken of every
//! Player's game and every Shadow Game, and Retries back to the latest one,
//! by everyone's vote (alone, with no vote), through the real running game on
//! the fake game, the shared Run log and the input streams.

mod support;

use pactlocke_run::{LogBody, Ruleset, RunCommand, ShadowState};
use support::streams::{Table, key};
use support::{PIDGEY, RATTATA, route_1};

fn with_retries(retries: u8) -> Ruleset {
    Ruleset {
        first_encounter: true,
        retries,
        ..Ruleset::default()
    }
}

fn retry_alone(table: &mut Table) {
    table.apps[0]
        .running
        .commands()
        .send(RunCommand::Retry {
            to: pactlocke_run::RetryTo::Latest,
        })
        .unwrap();
    table.plays(0, 5);
}

#[test]
fn every_preset_starts_with_retries_off() {
    for preset in pactlocke_run::Preset::ALL {
        assert_eq!(preset.ruleset(&[]).retries, 0, "{}", preset.name());
    }
}

#[test]
fn alone_a_retry_goes_back_to_the_last_in_game_save_and_undoes_everything_since() {
    let mut table = Table::start(&["Red"], with_retries(2));
    table.plays(0, 100);
    table.walks_into(0, route_1());
    table.saves(0);
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    assert_eq!(table.apps[0].running.engine().view().closed_areas(), ["Route 1"]);

    retry_alone(&mut table);
    let feed = table.apps[0].view_feed();
    assert!(
        feed[0..3]
            .iter()
            .any(|e| e == "Retry: back to the latest Run Checkpoint, and everything since is undone"),
        "{feed:?}"
    );
    assert!(
        table.apps[0].running.engine().view().closed_areas().is_empty(),
        "Route 1's Encounter is undone"
    );
    assert_eq!(table.apps[0].running.engine().retries_left(), 1);

    // Route 1's first wild battle is its Encounter again.
    table.meets_wild(0, RATTATA);
    assert!(!table.game(0).catches_blocked);
    assert_eq!(table.apps[0].running.engine().view().closed_areas(), ["Route 1"]);
}

#[test]
fn alone_with_no_retries_left_a_retry_is_refused() {
    let mut table = Table::start(&["Red"], with_retries(0));
    table.plays(0, 100);
    table.saves(0);
    retry_alone(&mut table);
    let feed = table.apps[0].view_feed();
    assert!(
        feed.iter()
            .any(|e| e == "The Retry couldn't be made: no Retries are left"),
        "{feed:?}"
    );
}

#[test]
fn before_any_run_checkpoint_a_retry_has_nowhere_to_go() {
    let mut table = Table::start(&["Red"], with_retries(1));
    table.plays(0, 100);
    retry_alone(&mut table);
    let feed = table.apps[0].view_feed();
    assert!(
        feed.iter()
            .any(|e| e == "The Retry couldn't be made: there is no Run Checkpoint to go back to yet"),
        "{feed:?}"
    );
    assert_eq!(table.apps[0].running.engine().retries_left(), 1, "nothing spent");
}

/// Red and Blue both playing a while.
fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

#[test]
fn an_in_game_save_takes_a_run_checkpoint_of_every_players_game() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(1));
    both_play(&mut table, 200);
    table.saves(0);
    both_play(&mut table, 40);
    let log = &table.apps[1].log;
    let calls = log.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].by, key(0));
    assert!(calls[0].parts.contains_key(&key(1)), "Blue's game is captured with it");
    assert_eq!(log.latest_complete(), Some(1));
}

#[test]
fn when_everyone_votes_yes_every_game_goes_back_together_and_the_shadows_follow() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(1));
    both_play(&mut table, 300);
    table.walks_into(0, route_1());
    table.saves(0);
    both_play(&mut table, 200);
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    both_play(&mut table, 200);
    assert_eq!(table.tallies(1).encounters.get(&key(0)), Some(&1));

    table.votes(0, 1, true);
    assert_eq!(table.apps[1].log.vote().map(|v| v.call), Some(1), "Red started a vote");
    table.votes(1, 1, true);
    both_play(&mut table, 20);

    let log = &table.apps[1].log;
    assert!(
        log.bodies().any(|(_, body)| body == LogBody::Retry { call: 1 }),
        "the log records the Retry"
    );
    assert_eq!(log.retries_left(), 0, "one spent from the pool");
    assert_eq!(
        table.tallies(1).encounters.get(&key(0)),
        None,
        "Red's Encounter after the checkpoint is void"
    );
    assert!(
        table.apps[0].running.engine().view().closed_areas().is_empty(),
        "and Route 1 is open again in Red's game"
    );
    for (i, app) in table.apps.iter().enumerate().take(2) {
        let feed = app.view_feed();
        assert!(feed.iter().any(|e| e.starts_with("Retry: back to")), "{i}: {feed:?}");
        assert!(app.refused.is_empty(), "{:?}", app.refused);
    }

    // After the Retry, each Shadow is back with its Player, and matches by hash.
    both_play(&mut table, 600);
    table.waits(1000);
    for (watcher, watched) in [(0, 1), (1, 0)] {
        let shadow = table.apps[watcher].shadow_of(key(watched));
        assert_eq!(shadow.state(), ShadowState::Matched, "{watcher} watching {watched}");
        assert_eq!(shadow.boot(), 2, "following the boot after the Retry");
        assert!(shadow.confirmed() > 400 && shadow.hashes_matched() >= 6);
    }
}

#[test]
fn a_retry_vote_ends_at_the_first_no() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(1));
    both_play(&mut table, 100);
    table.saves(1);
    both_play(&mut table, 40);
    table.votes(0, 1, true);
    table.votes(1, 1, false);
    both_play(&mut table, 40);
    assert_eq!(table.apps[0].log.vote(), None, "the No ended it");
    assert_eq!(table.apps[0].log.retries_left(), 1, "nothing spent");
    assert!(table.apps[0].view_feed().iter().all(|e| !e.starts_with("Retry")));

    // Anyone may start another.
    table.votes(1, 1, true);
    assert!(table.apps[0].log.vote().is_some());
}

#[test]
fn the_host_cant_record_a_retry_without_everyones_vote() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(3));
    table.host_records_retries = false;
    both_play(&mut table, 100);
    table.saves(0);
    both_play(&mut table, 40);
    table.votes(0, 1, true);
    table.submit(0, &LogBody::Retry { call: 1 });
    let refused = &table.apps[0].refused;
    assert!(
        refused.len() == 1 && refused[0].contains("is a Retry nobody agreed to"),
        "{refused:?}"
    );
    assert_eq!(table.apps[1].log.retries_left(), 3);
}

#[test]
fn with_retries_off_a_unanimous_vote_does_nothing() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(0));
    both_play(&mut table, 100);
    table.saves(0);
    both_play(&mut table, 40);
    table.votes(0, 1, true);
    table.votes(1, 1, true);
    both_play(&mut table, 40);
    assert!(
        !table.apps[1]
            .log
            .bodies()
            .any(|(_, b)| matches!(b, LogBody::Retry { .. }))
    );
}

#[test]
fn three_players_need_all_three_to_say_yes() {
    let mut table = Table::start(&["Red", "Blue", "Green"], with_retries(1));
    for _ in 0..10 {
        for i in 0..3 {
            table.plays(i, 10);
        }
    }
    table.saves(2);
    for _ in 0..6 {
        for i in 0..3 {
            table.plays(i, 10);
        }
    }
    assert_eq!(table.apps[0].log.latest_complete(), Some(1));
    table.votes(0, 1, true);
    table.votes(1, 1, true);
    assert_eq!(table.apps[0].log.retries_left(), 1, "Green hasn't voted");
    table.votes(2, 1, true);
    for i in 0..3 {
        table.plays(i, 10);
    }
    assert_eq!(table.apps[0].log.retries_left(), 0);
    for _ in 0..60 {
        for i in 0..3 {
            table.plays(i, 10);
        }
    }
    table.waits(1000);
    for (watcher, app) in table.apps.iter_mut().enumerate() {
        for watched in (0..3).filter(|&w| w != watcher) {
            let shadow = app.shadow_of(key(watched));
            assert_eq!(shadow.state(), ShadowState::Matched, "{watcher} watching {watched}");
            assert_eq!(shadow.boot(), 2);
        }
    }
}

#[test]
fn alone_each_retry_spends_one_until_none_are_left() {
    let mut table = Table::start(&["Red"], with_retries(2));
    table.plays(0, 100);
    table.saves(0);
    retry_alone(&mut table);
    retry_alone(&mut table);
    assert_eq!(table.apps[0].running.engine().retries_left(), 0);
    retry_alone(&mut table);
    let feed = table.apps[0].view_feed();
    assert_eq!(feed[0], "The Retry couldn't be made: no Retries are left");
}

#[test]
fn after_one_retry_the_next_goes_back_to_the_newer_run_checkpoint() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(2));
    both_play(&mut table, 200);
    table.saves(0);
    both_play(&mut table, 100);
    table.votes(0, 1, true);
    table.votes(1, 1, true);
    both_play(&mut table, 300);
    table.saves(1);
    both_play(&mut table, 100);
    assert_eq!(table.apps[0].log.latest_complete(), Some(2));
    table.votes(1, 2, true);
    table.votes(0, 2, true);
    both_play(&mut table, 600);
    table.waits(1000);
    assert_eq!(table.apps[0].log.retries_left(), 0);
    for (watcher, watched) in [(0, 1), (1, 0)] {
        let shadow = table.apps[watcher].shadow_of(key(watched));
        assert_eq!(shadow.state(), ShadowState::Matched, "{watcher} watching {watched}");
        assert_eq!(shadow.boot(), 3, "two Retries, two new boots");
    }
    for app in &table.apps {
        assert!(app.refused.is_empty(), "{:?}", app.refused);
    }
}

#[test]
fn an_entry_sent_from_before_a_players_game_went_back_is_void_too() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(1));
    both_play(&mut table, 200);
    table.saves(0);
    both_play(&mut table, 100);
    table.votes(0, 1, true);
    table.votes(1, 1, true);
    // Blue's game hasn't gone back yet when a Death from before reaches the Host.
    table.submit(
        1,
        &LogBody::Event(pactlocke_run::RunEvent::Died {
            pid: 7,
            species: PIDGEY,
            species_name: "PIDGEY".into(),
            name: "PIDGEY".into(),
            area: None,
            cause: pactlocke_run::DeathCause::Poison,
            at_ms: 0,
        }),
    );
    both_play(&mut table, 100);
    assert_eq!(table.tallies(0).deaths.get(&key(1)), None);
}
