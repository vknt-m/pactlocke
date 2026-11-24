//! Seam 1: map music left paused, played through the Run engine. A game
//! that leaves its map music paused on the overworld (Gaia's evolution scene
//! does) soft-locks the next warp to a map with other music: the warp waits
//! for the paused song to fade, and it never does. Pactlocke restarts the
//! song the way FireRed does after an evolution, after long enough that the
//! game's own jingles, which pause the music on purpose, have ended. A warp
//! already stuck on the paused song gets it unpaused.

mod support;

use pactlocke_game::{MapMusic, MapMusicState};
use pactlocke_run::{FeedKind, Ruleset};
use support::*;

/// About four seconds of game time: longer than any jingle.
const RESTART_AFTER: u32 = 240;

fn playing(song: u16, paused: bool) -> MapMusic {
    MapMusic {
        song,
        state: MapMusicState::Playing,
        paused,
    }
}

fn on_a_game_with_music() -> Player {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.game.map_music = Some(playing(0x108, false));
    player.plays(1);
    player
}

#[test]
fn map_music_left_paused_on_the_overworld_is_restarted_after_four_seconds() {
    let mut player = on_a_game_with_music();
    player.game.map_music = Some(playing(0x108, true));

    player.plays(RESTART_AFTER - 1);
    assert!(player.game.map_music.unwrap().paused, "a jingle may still be playing");

    player.plays(1);
    assert_eq!(player.game.map_music, Some(playing(0x108, false)), "restarted");
    assert_eq!(player.game.map_music_restarts, 1);
    assert!(
        player.feed(FeedKind::Run)[0].contains("music"),
        "the feed says so: {:?}",
        player.feed(FeedKind::Run)
    );
}

#[test]
fn a_jingle_that_ends_in_time_is_left_alone() {
    let mut player = on_a_game_with_music();
    player.game.map_music = Some(playing(0x108, true));
    player.plays(220);
    player.game.map_music = Some(playing(0x108, false));
    player.plays(RESTART_AFTER * 2);
    assert_eq!(player.game.map_music_restarts, 0);
    assert!(player.feed(FeedKind::Run).iter().all(|text| !text.contains("music")));
}

#[test]
fn music_paused_in_a_battle_or_a_script_is_left_alone() {
    let mut player = on_a_game_with_music();
    player.game.map_music = Some(playing(0x108, true));

    player.script_runs();
    player.plays(RESTART_AFTER * 2);
    assert_eq!(player.game.map_music_restarts, 0, "a script may pause the music");

    player.script_ends();
    player.game.safe_point = false;
    player.plays(RESTART_AFTER * 2);
    assert_eq!(player.game.map_music_restarts, 0, "off the overworld");

    // The wait starts again once back on the overworld.
    player.game.safe_point = true;
    player.plays(RESTART_AFTER - 1);
    assert_eq!(player.game.map_music_restarts, 0);
    player.plays(1);
    assert_eq!(player.game.map_music_restarts, 1);
}

#[test]
fn no_song_or_a_song_changing_is_left_alone() {
    let mut player = on_a_game_with_music();
    player.game.map_music = Some(playing(0, true));
    player.plays(RESTART_AFTER * 2);
    player.game.map_music = Some(MapMusic {
        song: 0x108,
        state: MapMusicState::Other(6),
        paused: true,
    });
    player.plays(RESTART_AFTER * 2);
    assert_eq!(player.game.map_music_restarts, 0);
}

#[test]
fn a_warp_stuck_on_paused_music_gets_it_unpaused() {
    let mut player = on_a_game_with_music();
    // The warp faded out and waits for the old song to stop; it is paused.
    let stuck = MapMusic {
        song: 0,
        state: MapMusicState::WaitingToStop,
        paused: true,
    };
    player.game.map_music = Some(stuck);
    player.plays(59);
    assert_eq!(player.game.map_music, Some(stuck), "a fade-out takes a moment");

    player.plays(1);
    assert!(
        !player.game.map_music.unwrap().paused,
        "unpaused, so it can fade and stop"
    );
    assert!(player.feed(FeedKind::Run)[0].contains("music"));
}

#[test]
fn a_game_without_map_music_symbols_is_left_alone() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.game.map_music = None;
    player.plays(RESTART_AFTER * 2);
    assert_eq!(player.game.map_music_restarts, 0);
}
