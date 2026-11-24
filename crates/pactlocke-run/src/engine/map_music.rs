//! Map music left paused. A game that leaves its map music paused on the
//! overworld (Gaia's evolution scene does, when the map's own song is the
//! evolution song) soft-locks the next warp to a map with other music: the
//! warp waits for the old song to fade out, and a paused song never does.
//! FireRed itself starts the map's song again after every evolution;
//! Pactlocke does the same for any game that doesn't, once the game's own
//! jingles, which pause the music on purpose, have had time to end.

use pactlocke_game::{MapMusicState, ScriptStatus};

use super::RunEngine;
use crate::{LiveGame, RunEvent};

/// About four seconds of game time: longer than any jingle (FireRed's
/// longest, the evolved fanfare, is 220 frames).
const RESTART_AFTER: u32 = 240;
/// About a second: a warp's fade-out ends well within it.
const UNSTICK_AFTER: u32 = 60;

impl RunEngine {
    /// Starts map music left paused on the quiet overworld again, and
    /// unpauses the music a stuck warp is waiting on.
    pub(super) fn keep_map_music_going(&mut self, game: &mut impl LiveGame) {
        let Some(music) = game.map_music() else {
            return;
        };
        let overworld = game.at_safe_overworld_point();

        let left_paused = overworld
            && music.paused
            && music.song != 0
            && music.state == MapMusicState::Playing
            && game.script().status == ScriptStatus::Idle;
        self.music_paused_for = if left_paused { self.music_paused_for + 1 } else { 0 };
        if self.music_paused_for >= RESTART_AFTER {
            self.music_paused_for = 0;
            if let Ok(changes) = game.restart_map_music() {
                self.apply(changes, game);
                self.record(RunEvent::MapMusicRestarted { stuck_warp: false });
            }
        }

        let stuck = overworld && music.paused && music.state == MapMusicState::WaitingToStop;
        self.warp_stuck_for = if stuck { self.warp_stuck_for + 1 } else { 0 };
        if self.warp_stuck_for >= UNSTICK_AFTER {
            self.warp_stuck_for = 0;
            if let Ok(changes) = game.unpause_map_music() {
                self.apply(changes, game);
                self.record(RunEvent::MapMusicRestarted { stuck_warp: true });
            }
        }
    }
}
