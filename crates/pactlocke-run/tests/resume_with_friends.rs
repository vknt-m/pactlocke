//! Seam 1: resuming a Run with friends, through the real running
//! game on the fake game, the shared Run log and every app's Run file.

mod support;

use pactlocke_run::{BootPlan, Ruleset, SUSPEND_SLOT_EVERY_MS, ShadowState};
use support::PIDGEY;
use support::streams::{Leaving, Table, key};

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

#[test]
fn a_player_who_closes_the_app_resumes_at_the_same_frame_and_the_others_shadows_carry_on() {
    let mut table = Table::start(&["Red", "Blue"], first_encounter());
    everyone_plays(&mut table, 300);
    let boot = table.apps[1].running.engine().boot();
    let frame = table.apps[1].running.engine().frames_run();

    table.leaves_and_resumes(1, Leaving::Suspends);
    assert_eq!(table.apps[1].running.engine().boot(), boot, "the same boot goes on");
    assert_eq!(
        table.apps[1].running.engine().frames_run(),
        frame,
        "from the same frame"
    );

    everyone_plays(&mut table, 600);
    table.waits(2_000);
    let shadow = table.apps[0].shadow_of(key(1));
    assert_eq!(shadow.state(), ShadowState::Matched);
    assert_eq!(shadow.boot(), boot);
    assert!(
        shadow.confirmed() > frame + 400,
        "Red's Shadow of Blue plays on past the resume"
    );
}

#[test]
fn after_a_crash_a_player_takes_back_their_lost_presses_and_goes_on_from_the_same_frame() {
    let mut table = Table::start(&["Red", "Blue"], first_encounter());
    everyone_plays(&mut table, 330);
    let boot = table.apps[1].running.engine().boot();
    let host_has = table.apps[0].hub.received(&key(1)).unwrap().1;

    table.leaves_and_resumes(1, Leaving::Crashes);
    let engine = table.apps[1].running.engine();
    assert_eq!(engine.boot(), boot, "the same boot goes on");
    assert_eq!(engine.frames_run(), host_has, "through every press the Host had");

    everyone_plays(&mut table, 600);
    table.waits(2_000);
    let shadow = table.apps[0].shadow_of(key(1));
    assert_eq!(shadow.state(), ShadowState::Matched, "checked by hash");
    assert!(shadow.confirmed() > host_has + 400);
    assert!(table.apps[0].refused.is_empty() && table.apps[1].refused.is_empty());
    assert_eq!(table.apps[0].log.flags(), [], "no stream rewritten, no savestate load");
}

#[test]
fn in_a_run_with_friends_the_suspend_slot_is_kept_up_to_date_and_a_crash_goes_on_from_it() {
    let mut table = Table::start(&["Red", "Blue"], first_encounter());
    everyone_plays(&mut table, 40);
    let slot = |table: &Table| table.apps[1].storage.load_run().suspended.map(|s| s.frame);
    assert_eq!(slot(&table), None, "not yet");

    // Every couple of minutes.
    table.clock.advance_ms(SUSPEND_SLOT_EVERY_MS);
    table.plays(1, 1);
    let every = table.apps[1].running.engine().frames_run();
    assert_eq!(slot(&table), Some(every));

    // At an Encounter's end.
    everyone_plays(&mut table, 40);
    table.meets_wild(1, PIDGEY);
    table.leaves_battle(1);
    let after_encounter = slot(&table).unwrap();
    assert!(after_encounter > every, "{after_encounter} after {every}");

    // A crash goes on from it, not from the boot checkpoint.
    everyone_plays(&mut table, 30);
    let newest = slot(&table).unwrap();
    table.leaves_and_resumes(1, Leaving::Crashes);
    let BootPlan::Rebuild { from, .. } = table.apps[1].running.engine().boot_plan() else {
        panic!("rebuilt after the crash");
    };
    assert_eq!(from.frame, newest);
    everyone_plays(&mut table, 300);
    table.waits(2_000);
    assert_eq!(table.apps[0].shadow_of(key(1)).state(), ShadowState::Matched);
}

#[test]
fn a_resumed_app_restarts_each_shadow_game_from_its_checkpoint_in_the_run_file() {
    let mut table = Table::start(&["Red", "Blue", "Green"], first_encounter());
    everyone_plays(&mut table, 600);
    table.waits(1_000);
    let kept_at = table.apps[0].shadow_of(key(1)).confirmed();

    table.leaves_and_resumes(0, Leaving::Suspends);
    for other in [1, 2] {
        let shadow = table.apps[0].shadow_of(key(other));
        assert!(
            shadow.confirmed() >= kept_at.min(500),
            "{other}: from its checkpoint, not from power-on: {}",
            shadow.confirmed()
        );
    }
    everyone_plays(&mut table, 600);
    table.waits(2_000);
    for other in [1, 2] {
        let shadow = table.apps[0].shadow_of(key(other));
        assert_eq!(shadow.state(), ShadowState::Matched, "{other}");
        assert!(shadow.confirmed() > kept_at + 400, "{other}");
    }
    assert_eq!(table.apps[0].log.flags(), []);
}
