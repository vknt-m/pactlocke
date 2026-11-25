//! Choosing a Retry point: a Retry offers the latest Run Checkpoint and the
//! Rescue Points from the last 40 minutes, so a wipe moments after the
//! newest one can go back further. Each Retry still spends one from the
//! pool, whichever point is chosen. Through the real running game on the
//! fake game, the shared Run log and the input streams.

mod support;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{BattleKind, FakeGame};
use pactlocke_run::{LogBody, MemoryStorage, RetryTo, Ruleset, RunCommand, RunEngine, RunningGame};
use support::streams::Table;
use support::{CHARMANDER, PIDGEY, RATTATA, START_MS, TestClock, battle, new_solo_run, route_1, wild_pokemon};

fn soul_link(retries: u8) -> Ruleset {
    Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        links: true,
        retries,
        ..Ruleset::default()
    }
}

fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

/// The Run Checkpoints a Retry can go back to now, as Player `i`'s app sees them.
fn offered(table: &Table, i: usize) -> Vec<u64> {
    table.apps[i].log.retry_points().iter().map(|c| c.n).collect()
}

/// Red and Blue each catch one on Route 1, so they are linked; Red saves
/// (call 1), and two Rescue Points follow (calls 2 and 3).
fn three_points(table: &mut Table) {
    both_play(table, 100);
    for i in 0..2 {
        table.walks_into(i, route_1());
    }
    table.meets_wild(0, PIDGEY);
    table.catches(0, PIDGEY, 0xA1);
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB1);
    both_play(table, 120);
    table.saves(0);
    table.next_rescue_point();
    table.next_rescue_point();
}

/// Red's Pidgey faints to a trainer: both linked parties are wiped.
fn red_wipes(table: &mut Table) {
    table.meets_trainer(0, CHARMANDER);
    table.faints(0, 0xA1);
    both_play(table, 200);
}

/// What going back puts right in a real game: the fake's party and battle
/// aren't in its checkpoint, so the test puts them back as they were.
fn back_as_at_the_point(table: &mut Table) {
    for i in 0..2 {
        table.happens(i, |g| {
            g.battle = None;
            for p in &mut g.party {
                p.stats.as_mut().expect("a party Pokémon").hp = 20;
            }
        });
    }
}

#[test]
fn a_retry_offers_the_latest_save_and_every_rescue_point() {
    let mut table = Table::start(&["Red", "Blue"], soul_link(1));
    three_points(&mut table);
    assert_eq!(offered(&table, 1), [1, 2, 3]);
}

#[test]
fn a_retry_can_go_back_further_than_the_newest_point() {
    let mut table = Table::start(&["Red", "Blue"], soul_link(1));
    three_points(&mut table);
    red_wipes(&mut table);
    assert!(table.apps[1].log.wiped().is_some());

    table.votes(1, 2, true);
    assert_eq!(table.apps[0].log.vote().map(|v| v.call), Some(2), "Blue started a vote");
    back_as_at_the_point(&mut table);
    table.votes(0, 2, true);
    both_play(&mut table, 40);

    let log = &table.apps[1].log;
    assert!(
        log.bodies().any(|(_, body)| body == LogBody::Retry { call: 2 }),
        "the log records the Retry to the older point"
    );
    assert_eq!(log.retries_left(), 0, "one spent from the pool");
    assert_eq!(log.rescues(), 0, "it is a Retry, not a Rescue");
    assert!(log.wiped().is_none(), "the wipe is undone");
    assert_eq!(offered(&table, 1), [1, 2], "the point after it is void");
    for app in &table.apps {
        assert!(app.refused.is_empty(), "{:?}", app.refused);
        assert!(app.running.engine().view().wipe.is_none());
    }
}

#[test]
fn a_vote_for_a_point_not_offered_is_no_vote() {
    let mut table = Table::start(&["Red", "Blue"], soul_link(1));
    three_points(&mut table);
    red_wipes(&mut table);
    table.votes(0, 9, true);
    assert_eq!(table.apps[1].log.vote(), None, "there is no 9th Run Checkpoint");
    assert!(table.apps[1].log.game_over().is_none());
}

#[test]
fn a_rescue_point_called_after_the_wipe_is_not_offered() {
    let mut table = Table::start(&["Red", "Blue"], soul_link(1));
    three_points(&mut table);
    // Blue's party is wiped this time, so Red, the Host, still calls points.
    table.meets_trainer(1, CHARMANDER);
    table.faints(1, 0xB1);
    both_play(&mut table, 200);
    assert!(table.apps[0].log.wiped().is_some());
    table.next_rescue_point();
    assert_eq!(offered(&table, 0), [1, 2, 3], "Blue's stopped game gives no part");
}

#[test]
fn alone_a_retry_can_go_back_to_an_older_rescue_point_and_spends_one() {
    let clock = TestClock::at(START_MS);
    let engine = RunEngine::start(
        new_solo_run(Ruleset {
            first_encounter: true,
            retries: 1,
            ..Ruleset::default()
        }),
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
    clock.advance_ms(pactlocke_run::RESCUE_POINT_EVERY_MS);
    plays(&mut running, 40);

    // Route 1's Encounter, then a second Rescue Point after it.
    running.game_mut().opponent_party = vec![wild_pokemon(PIDGEY)];
    running.game_mut().battle = Some(battle(BattleKind::Wild, PIDGEY));
    plays(&mut running, 20);
    running.game_mut().battle = None;
    plays(&mut running, 20);
    clock.advance_ms(pactlocke_run::RESCUE_POINT_EVERY_MS);
    plays(&mut running, 40);
    let points = running.engine().view().retry_points();
    assert_eq!(points.len(), 2, "{points:?}");
    assert_eq!(running.engine().view().closed_areas(), ["Route 1"]);

    // A point the Run doesn't offer is refused, and spends nothing.
    assert!(running.retry(&RetryTo::Point("not a checkpoint".into())).is_err());
    assert_eq!(running.engine().retries_left(), 1);

    running
        .commands()
        .send(RunCommand::Retry {
            to: RetryTo::Point(points[0].hash.clone()),
        })
        .unwrap();
    plays(&mut running, 5);
    let view = running.engine().view();
    assert!(view.closed_areas().is_empty(), "Route 1's Encounter is undone");
    assert_eq!(running.engine().retries_left(), 0, "one Retry spent");
    assert_eq!(view.rescues, 0, "it is a Retry, not a Rescue");
    let left: Vec<i64> = view.retry_points().iter().map(|p| p.at_ms).collect();
    assert_eq!(left, [points[0].at_ms], "the point after it is gone");

    // None left: another Retry is refused.
    assert!(running.retry(&RetryTo::Point(points[0].hash.clone())).is_err());
}
