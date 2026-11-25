//! Rescues: about every four minutes the Host calls a Rescue Point that
//! every Player gives their part of at a quiet overworld moment, and on
//! everyone's vote every Player goes back to one together, as for a Retry
//! but free. Alone, the Player's own app calls them, and a Rescue needs no
//! vote. Through the real running game on the fake game, the shared Run
//! log and the input streams.

mod support;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{BattleKind, FakeGame};
use pactlocke_run::{
    LogBody, MemoryStorage, RESCUE_POINTS_KEPT, Ruleset, RunCommand, RunEngine, RunningGame, ShadowState,
};
use support::streams::{Table, key};
use support::{PIDGEY, START_MS, TestClock, battle, new_solo_run, route_1, wild_pokemon};

fn with_retries(retries: u8) -> Ruleset {
    Ruleset {
        first_encounter: true,
        retries,
        ..Ruleset::default()
    }
}

/// Red and Blue both playing a while.
fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

/// The Rescue Points the Run offers now, as Player `i`'s app sees them.
fn offered(table: &Table, i: usize) -> Vec<u64> {
    table.apps[i].log.rescue_points().iter().map(|c| c.n).collect()
}

#[test]
fn the_host_calls_a_rescue_point_every_few_minutes_and_everyone_gives_their_part() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(1));
    both_play(&mut table, 200);
    assert!(offered(&table, 1).is_empty(), "none before four minutes");

    table.walks_into(0, route_1());
    table.next_rescue_point();
    let points = table.apps[1].log.rescue_points();
    assert_eq!(points.len(), 1, "{points:?}");
    assert_eq!(points[0].by, key(0), "the Host called it");
    assert!(points[0].parts.contains_key(&key(1)), "Blue gave their part");
    assert_eq!(points[0].parts[&key(0)].area.as_deref(), Some("Route 1"));
    assert_eq!(points[0].parts[&key(1)].area.as_deref(), Some("Pallet Town"));
}

#[test]
fn a_rescue_undoes_the_events_after_its_point_and_spends_no_retry() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(1));
    both_play(&mut table, 300);
    table.walks_into(0, route_1());
    table.next_rescue_point();
    let point = offered(&table, 1)[0];
    both_play(&mut table, 200);
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    both_play(&mut table, 200);
    assert_eq!(table.tallies(1).encounters.get(&key(0)), Some(&1));

    table.votes_to_rescue(0, point, true);
    assert_eq!(
        table.apps[1].log.rescue_vote().map(|v| v.call),
        Some(point),
        "Red started a vote"
    );
    table.votes_to_rescue(1, point, true);
    both_play(&mut table, 20);

    let log = &table.apps[1].log;
    assert!(
        log.bodies().any(|(_, body)| body == LogBody::Rescue { call: point }),
        "the log records the Rescue"
    );
    assert_eq!(log.rescues(), 1);
    assert_eq!(log.retries_left(), 1, "no Retry is spent");
    assert_eq!(
        table.tallies(1).encounters.get(&key(0)),
        None,
        "Red's Encounter is undone"
    );
    for app in &table.apps {
        let feed = app.view_feed();
        assert!(
            feed.iter()
                .any(|e| e.starts_with("Rescue: back to the Rescue Point from ")
                    && e.ends_with("Everything since is undone, and no Retry was spent")),
            "{feed:?}"
        );
        assert_eq!(app.running.engine().view().rescues, 1);
    }
    assert!(table.apps[0].running.engine().view().closed_areas().is_empty());

    // Both games went on from their parts, and each Shadow Game followed.
    both_play(&mut table, 300);
    for (i, other) in [(0, 1), (1, 0)] {
        let shadow = table.apps[i].shadow_of(key(other));
        assert!(
            !matches!(shadow.state(), ShadowState::Diverged { .. }),
            "{:?}",
            shadow.state()
        );
    }
    assert!(table.apps.iter().all(|a| a.refused.is_empty()));
    let flags = table.apps[1].log.flags();
    assert!(flags.is_empty(), "a Rescue is no cheat: {flags:?}");
}

#[test]
fn a_no_ends_the_rescue_vote() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(0));
    both_play(&mut table, 200);
    table.next_rescue_point();
    let point = offered(&table, 0)[0];
    table.votes_to_rescue(1, point, true);
    table.votes_to_rescue(0, point, false);
    both_play(&mut table, 20);
    assert_eq!(table.apps[0].log.rescue_vote(), None);
    assert_eq!(table.apps[0].log.rescues(), 0);

    // A Yes after the No starts a new vote, which a lone Yes doesn't pass.
    table.votes_to_rescue(1, point, true);
    both_play(&mut table, 20);
    assert_eq!(table.apps[0].log.rescues(), 0);
    assert!(table.apps[0].log.rescue_vote().is_some());
}

#[test]
fn only_complete_points_that_no_rescue_voided_are_offered() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(0));
    both_play(&mut table, 200);
    table.next_rescue_point();
    table.next_rescue_point();
    assert_eq!(offered(&table, 0), [1, 2]);

    // Blue is in a battle: their part waits for the overworld.
    table.meets_wild(1, PIDGEY);
    table.clock.advance_ms(pactlocke_run::RESCUE_POINT_EVERY_MS);
    both_play(&mut table, 40);
    assert_eq!(table.apps[0].log.calls().len(), 3, "the Host called a third");
    assert_eq!(offered(&table, 0), [1, 2], "Blue has no part of it yet");
    table.leaves_battle(1);
    both_play(&mut table, 40);
    assert_eq!(offered(&table, 0), [1, 2, 3]);

    // A Rescue to the first voids the two after it.
    table.votes_to_rescue(0, 1, true);
    table.votes_to_rescue(1, 1, true);
    both_play(&mut table, 40);
    assert_eq!(table.apps[0].log.rescues(), 1);
    assert_eq!(offered(&table, 0), [1]);
}

#[test]
fn the_oldest_of_ten_rescue_points_is_overwritten() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(0));
    both_play(&mut table, 200);
    for _ in 0..RESCUE_POINTS_KEPT {
        table.next_rescue_point();
    }
    let ten: Vec<u64> = (1..=RESCUE_POINTS_KEPT as u64).collect();
    assert_eq!(offered(&table, 1), ten);
    table.next_rescue_point();
    let newest: Vec<u64> = (2..=RESCUE_POINTS_KEPT as u64 + 1).collect();
    assert_eq!(offered(&table, 1), newest, "the first is no longer offered");

    // The oldest still offered is still there to go back to.
    table.votes_to_rescue(0, 2, true);
    table.votes_to_rescue(1, 2, true);
    both_play(&mut table, 40);
    let feed = table.apps[1].view_feed();
    assert!(feed.iter().any(|e| e.starts_with("Rescue: back")), "{feed:?}");
}

#[test]
fn alone_a_rescue_point_comes_every_few_minutes_and_a_rescue_needs_no_vote() {
    let clock = TestClock::at(START_MS);
    let engine = RunEngine::start(
        new_solo_run(with_retries(0)),
        Box::new(MemoryStorage::default()),
        Box::new(clock.clone()),
    )
    .unwrap();
    let mut running = RunningGame::boot(FakeGame::default(), engine, START_MS).unwrap();
    let plays = |running: &mut RunningGame<FakeGame>, n: usize| {
        for _ in 0..n {
            running.run_frame(Keys::NONE);
            clock.advance_ms(17);
        }
    };
    plays(&mut running, 100);
    running.game_mut().area = Some(route_1());
    plays(&mut running, 40);
    assert!(
        running.engine().view().rescue_points.is_empty(),
        "none before four minutes"
    );
    clock.advance_ms(pactlocke_run::RESCUE_POINT_EVERY_MS);
    plays(&mut running, 40);
    let points = running.engine().view().rescue_points.clone();
    assert_eq!(points.len(), 1, "{points:?}");
    assert_eq!(points[0].area.as_deref(), Some("Route 1"));

    running.game_mut().opponent_party = vec![wild_pokemon(PIDGEY)];
    running.game_mut().battle = Some(battle(BattleKind::Wild, PIDGEY));
    plays(&mut running, 20);
    running.game_mut().battle = None;
    plays(&mut running, 20);
    assert_eq!(running.engine().view().closed_areas(), ["Route 1"]);

    running
        .commands()
        .send(RunCommand::Rescue {
            to: points[0].hash.clone(),
        })
        .unwrap();
    plays(&mut running, 5);
    let view = running.engine().view();
    assert!(view.closed_areas().is_empty(), "Route 1's Encounter is undone");
    assert_eq!(view.rescues, 1);
    assert_eq!(view.rescue_points.len(), 1, "the point gone back to is still offered");
    assert_eq!(
        running.engine().retries_left(),
        0,
        "nothing to spend, and nothing spent"
    );
}

#[test]
fn a_retry_still_goes_back_to_the_latest_save_not_a_rescue_point() {
    let mut table = Table::start(&["Red", "Blue"], with_retries(1));
    both_play(&mut table, 200);
    table.saves(0);
    table.next_rescue_point();
    let log = &table.apps[1].log;
    assert_eq!(log.calls().len(), 2, "the in-game save, then the Rescue Point");
    assert_eq!(offered(&table, 1), [2]);
    assert_eq!(table.apps[1].log.latest_complete(), Some(1), "a Retry goes to the save");
}
