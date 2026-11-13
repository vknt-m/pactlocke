//! Seam 1: cheat flags. A stream that rewrites frames the others
//! have, a game started again with no Retry behind it, frames faster than
//! 4x, a Shadow Game that stops matching and a tampered Run Checkpoint are
//! each flagged to every Player, through the real running game on the fake
//! game, the shared Run log and every app's Shadow Games.

mod support;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::FakeGame;
use pactlocke_run::{Cheat, InputChunk, Ruleset, RunCommand, RunEngine, RunningGame, ShadowState};
use support::START_MS;
use support::streams::{Table, key};

fn with_retries() -> Ruleset {
    Ruleset {
        retries: 2,
        ..Ruleset::default()
    }
}

fn everyone_plays(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        for i in 0..table.apps.len() {
            table.plays(i, 10);
        }
    }
}

/// Frames `from` to `from + n` of Player `i`'s stream, with keys nobody pressed.
fn other_presses(i: usize, boot: u32, from: u64, n: usize) -> InputChunk {
    InputChunk {
        player: key(i),
        boot,
        first_frame: from,
        keys: vec![Keys(0x2AA); n],
        stamped: Vec::new(),
        hashes: Vec::new(),
        marks: Vec::new(),
        booted_from: None,
    }
}

fn feed_has(table: &Table, i: usize, line: &str) -> bool {
    table.apps[i].view_feed().iter().any(|e| e == line)
}

#[test]
fn presses_that_rewrite_frames_the_others_have_are_flagged_to_everyone_and_change_nothing() {
    let mut table = Table::start(&["Red", "Blue", "Green"], with_retries());
    everyone_plays(&mut table, 200);
    // Blue is away a while, comes back, and sends frames the others have
    // already, pressed differently: as after loading a savestate.
    table.goes_offline(1);
    table.plays(1, 60);
    table.comes_back(1);
    everyone_plays(&mut table, 60);
    let have = table.apps[0].hub.received(&key(1)).unwrap().1;
    table.forges(1, other_presses(1, 1, have - 30, 30));
    everyone_plays(&mut table, 20);

    for (i, app) in table.apps.iter().enumerate() {
        let flags = app.log.flags();
        assert_eq!(flags.len(), 1, "{i}: {flags:?}");
        assert_eq!(flags[0].player, key(1));
        assert_eq!(flags[0].cheat, Cheat::HistoryRewritten);
        assert_eq!(flags[0].by, [key(0)], "the Host saw it");
        assert!(flags[0].at.frame >= have - 30 && flags[0].at.frame < have);
    }
    let frame = table.apps[0].log.flags()[0].at.frame;
    let what = "presses that rewrite frames already played (a savestate load or rewritten history)";
    for (i, by) in [(0, "you"), (2, "Red")] {
        let line = format!("Cheat flag on Blue, at frame {frame}: {what} (seen by {by})");
        assert!(feed_has(&table, i, &line), "{i}: {:?}", table.apps[i].view_feed());
    }
    let line = format!("Cheat flag on you, at frame {frame}: {what} (seen by Red)");
    assert!(feed_has(&table, 1, &line), "{:?}", table.apps[1].view_feed());

    // The forged frames went nowhere: every Shadow of Blue still matches.
    everyone_plays(&mut table, 300);
    table.waits(1000);
    for i in [0, 2] {
        assert_eq!(table.apps[i].shadow_of(key(1)).state(), ShadowState::Matched, "{i}");
    }
    // And it triggers nothing: no vote, nobody out, no Retry spent.
    let log = &table.apps[0].log;
    assert_eq!(log.dropout_vote(), None);
    assert_eq!(log.vote(), None);
    assert_eq!(log.active().len(), 3);
    assert_eq!(log.retries_left(), 2);
}

#[test]
fn a_game_started_again_with_no_retry_behind_it_is_flagged_as_a_savestate_load() {
    let mut table = Table::start(&["Red", "Blue"], with_retries());
    everyone_plays(&mut table, 200);
    let mut restart = other_presses(1, 2, 0, 15);
    restart.booted_from = Some((1, 60));
    table.forges(1, restart);
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[1].log.flags(), [], "the log gets a while to show a Retry");
    table.waits(10_000);
    everyone_plays(&mut table, 20);
    let flags = table.apps[1].log.flags();
    assert_eq!(flags.len(), 1, "{flags:?}");
    assert_eq!(
        (flags[0].player, flags[0].cheat),
        (key(1), Cheat::RestartedWithoutRetry)
    );
    assert_eq!((flags[0].at.boot, flags[0].at.frame), (2, 0));
}

#[test]
fn a_retry_is_no_savestate_load_and_a_flag_stands_through_it_and_a_dropout() {
    let mut table = Table::start(&["Red", "Blue", "Green"], with_retries());
    everyone_plays(&mut table, 200);
    let have = table.apps[0].hub.received(&key(2)).unwrap().1;
    table.forges(2, other_presses(2, 1, have - 15, 15));
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[0].log.flags().len(), 1);

    // Everyone goes back by a Retry, and every game starts its next boot.
    table.saves(0);
    everyone_plays(&mut table, 100);
    let call = table.apps[0].log.latest_complete().expect("everyone's part");
    for i in 0..3 {
        table.votes(i, call, true);
    }
    everyone_plays(&mut table, 300);
    assert_eq!(table.apps[0].log.retries_left(), 1, "the Retry happened");
    for (i, app) in table.apps.iter().enumerate() {
        assert_eq!(app.running.engine().boot(), 2, "{i}");
    }
    let flags = table.apps[0].log.flags();
    assert_eq!(
        flags.len(),
        1,
        "no flag for the Retry's new boots, and the old one stands: {flags:?}"
    );
    assert_eq!(flags[0].cheat, Cheat::HistoryRewritten);

    // Green, dropped, is still flagged.
    table.goes_offline(2);
    table.votes_to_drop(0, 2, true);
    table.votes_to_drop(1, 2, true);
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[0].log.active().len(), 2);
    assert_eq!(table.apps[0].log.flags().len(), 1);
}

#[test]
fn frames_faster_than_4x_are_flagged() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    // 8 frames in each slice of time: about 8x, for about 20 seconds.
    table.plays_fast(1, 9600, 8);
    let flags = table.apps[0].log.flags();
    assert_eq!(flags.len(), 1, "{flags:?}");
    assert_eq!((flags[0].player, flags[0].cheat), (key(1), Cheat::TooFast));
    table.plays(0, 1);
    assert!(feed_has(
        &table,
        0,
        &format!(
            "Cheat flag on Blue, at frame {}: played faster than 4x (seen by you)",
            flags[0].at.frame
        )
    ));
}

#[test]
fn four_x_for_a_long_time_and_a_burst_after_being_offline_are_not() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    // 4 frames in each slice of time is just under 4x.
    table.plays_fast(1, 8000, 4);
    // Offline a while, then everything at once.
    table.goes_offline(1);
    table.plays_fast(1, 2000, 4);
    table.comes_back(1);
    table.plays_fast(1, 400, 4);
    table.waits(2000);
    assert_eq!(table.apps[0].log.flags(), []);
}

#[test]
fn a_shadow_that_stops_matching_is_flagged_as_game_out_of_sync() {
    let mut table = Table::start(&["Red", "Blue", "Green"], Ruleset::default());
    everyone_plays(&mut table, 200);
    table.edits_memory(1, 0x0200_1000, &[99]);
    everyone_plays(&mut table, 400);
    table.waits(1000);
    everyone_plays(&mut table, 20);
    let flags = table.apps[0].log.flags();
    assert_eq!(flags.len(), 1, "one flag, however many saw it: {flags:?}");
    assert_eq!((flags[0].player, flags[0].cheat), (key(1), Cheat::OutOfSync));
    let frame = flags[0].at.frame;
    for i in [0, 2] {
        assert!(
            table.apps[i].view_feed().iter().any(|e| e.starts_with(&format!(
                "Cheat flag on Blue, at frame {frame}: game out of sync (seen by "
            ))),
            "{i}: {:?}",
            table.apps[i].view_feed()
        );
    }
}

#[test]
fn a_run_checkpoint_changed_in_the_run_file_is_flagged_and_not_gone_back_to() {
    let mut table = Table::start(&["Red"], with_retries());
    table.plays(0, 100);
    table.saves(0);
    table.plays(0, 100);
    table.apps[0].running.flush();
    let storage = table.apps[0].storage.clone();

    // The app is closed; someone changes the checkpoint in the Run file.
    storage.tamper_with_checkpoints(|c| c.core.save_memory[0] ^= 0xFF);
    let engine = RunEngine::resume(Box::new(storage.clone()), Box::new(table.clock.clone())).unwrap();
    let mut game = RunningGame::boot(FakeGame::default(), engine, START_MS).unwrap();
    game.commands()
        .send(RunCommand::Retry {
            to: pactlocke_run::RetryTo::Latest,
        })
        .unwrap();
    game.run_frame(Keys::NONE);
    let feed: Vec<String> = game.engine().view().feed.iter().map(|e| e.text.clone()).collect();
    assert!(
        feed.iter().any(|e| e.starts_with("Cheat flag on you, at frame ")
            && e.ends_with("a Run Checkpoint in the Run file doesn't match its hash in the log")),
        "{feed:?}"
    );
    assert!(
        feed.iter()
            .any(|e| e.contains("that Run Checkpoint in the Run file doesn't match its hash in the log")),
        "the Retry is refused: {feed:?}"
    );
    assert_eq!(game.engine().retries_left(), 2);
}

#[test]
fn time_played_slowly_is_no_credit_for_playing_fast_later() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    // About a minute and a half at 1x, then 8x.
    table.plays(1, 5400);
    assert_eq!(table.apps[0].log.flags(), []);
    table.plays_fast(1, 6400, 8);
    let flags = table.apps[0].log.flags();
    assert_eq!(flags.len(), 1, "{flags:?}");
    assert_eq!(flags[0].cheat, Cheat::TooFast);
    assert!(flags[0].at.frame > 5400, "during the fast part");
}
