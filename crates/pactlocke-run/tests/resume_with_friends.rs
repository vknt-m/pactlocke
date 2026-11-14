//! Seam 1: resuming a Run with friends, through the real running
//! game on the fake game, the shared Run log and every app's Run file.

mod support;

use pactlocke_run::Ruleset;
use support::PIDGEY;
use support::streams::Table;

fn everyone_plays(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        for i in 0..table.apps.len() {
            table.plays(i, 10);
        }
    }
}

fn first_encounter() -> Ruleset {
    Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    }
}

#[test]
fn every_player_keeps_the_whole_run_log_in_their_run_file() {
    let mut table = Table::start(&["Red", "Blue", "Green"], first_encounter());
    everyone_plays(&mut table, 40);
    table.meets_wild(1, PIDGEY);
    everyone_plays(&mut table, 40);
    for app in &mut table.apps {
        app.running.flush();
        let kept = app.storage.load_run().log;
        assert!(kept.len() > 2, "{}: more than the Genesis and the lock", app.name);
        assert_eq!(kept, app.log.entries(), "{}", app.name);
    }
}
