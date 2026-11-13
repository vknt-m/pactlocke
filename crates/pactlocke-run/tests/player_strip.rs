//! Seam 1: what the Player strip shows of each Player, through the
//! real running game on the fake game, the shared Run log and every app's
//! Shadow Games.

mod support;

use pactlocke_run::{Doing, Ruleset, RunCommand};
use support::streams::{Table, key};
use support::{PIDGEY, route_1};

fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

#[test]
fn a_speed_change_is_in_the_log_and_in_every_players_feed() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    both_play(&mut table, 40);
    for speed in [2, 2, 4] {
        table.apps[0].running.commands().send(RunCommand::Speed(speed)).unwrap();
        both_play(&mut table, 20);
    }
    assert_eq!(table.apps[1].log.speed_of(&key(0)), 4);
    assert_eq!(table.apps[1].log.speed_of(&key(1)), 1, "Blue never said: 1x");
    let red = table.apps[0].view_feed();
    assert_eq!(
        red.iter().filter(|e| e.starts_with("You play at")).count(),
        2,
        "{red:?}"
    );
    let blue = table.apps[1].view_feed();
    assert!(blue.iter().any(|e| e == "Red plays at 2x"), "{blue:?}");
    assert!(blue.iter().any(|e| e == "Red plays at 4x"), "{blue:?}");
}

#[test]
fn each_players_area_and_what_they_are_doing_show_from_their_own_game() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.walks_into(0, route_1());
    both_play(&mut table, 120);
    let view = table.apps[0].running.engine().view().clone();
    assert_eq!(
        (view.area.as_deref(), &view.doing),
        (Some("Route 1"), &Doing::Exploring)
    );
    let seen = table.apps[1].shadow_of(key(0)).seen().clone();
    assert_eq!(
        (seen.area.as_deref(), &seen.doing),
        (Some("Route 1"), &Doing::Exploring)
    );

    table.meets_wild(0, PIDGEY);
    both_play(&mut table, 120);
    assert_eq!(table.apps[0].running.engine().view().doing, Doing::InBattle);
    assert_eq!(table.apps[1].shadow_of(key(0)).seen().doing, Doing::InBattle);
}
