//! Faint = Dead's game over. At the Death that leaves no living Pokémon in
//! the party (living ones in the PC don't count), the party is wiped: the
//! game stops there, the Dead stay in the party, and an empty party never
//! reaches the game. Alone, a Retry is offered if one is left and there is
//! a Run Checkpoint to go back to; otherwise, or if the Player declines, the
//! Run is over. With friends, the shared Run log decides (see
//! [`crate::RunLog::game_over`]).

use super::RunEngine;
use crate::view::{GameOverView, WipeView};
use crate::{LastDeath, LiveGame, Recorded, RunEvent, Survivor};

/// The game's play time counts frames: the GBA shows about 60 a second.
const FRAMES_PER_SECOND: i64 = 60;

impl RunEngine {
    /// Whether the game has stopped: the party is wiped, or the Run is over.
    pub fn halted(&self) -> bool {
        self.view.wipe.is_some() || self.view.game_over.is_some()
    }

    /// The Player declines the Retry after a wipe: the Run is over.
    pub fn end_run(&mut self) {
        if let Some(wipe) = self.view.wipe.clone()
            && self.view.game_over.is_none()
        {
            let longest = self.longest();
            let play_ms = self.play_ms();
            self.record(RunEvent::GameOver {
                how: wipe.how,
                longest,
                play_ms,
            });
        }
    }

    /// With friends, the Run log says the Run is over: `how` it ended, and
    /// every Player's longest survivor.
    pub fn game_over(&mut self, how: String, longest: Vec<Survivor>) {
        if self.view.game_over.is_none() {
            let play_ms = self.play_ms();
            self.record(RunEvent::GameOver { how, longest, play_ms });
        }
    }

    /// This Player's own longest survivor, up to the wipe.
    fn longest(&self) -> Vec<Survivor> {
        let end_ms = self.wiped_at_ms.unwrap_or_else(|| self.clock.now_ms());
        crate::survivors(self.lives.iter().map(|e| (None, e)), end_ms)
    }

    /// The game's own play time at its last look, in milliseconds.
    fn play_ms(&self) -> i64 {
        i64::from(self.last_play_time.unwrap_or(0)) * 1000 / FRAMES_PER_SECOND
    }

    /// After a Death: a party whose every Pokémon is Dead is wiped.
    pub(super) fn check_wipe(&mut self, game: &mut impl LiveGame) {
        if !self.rules.faint_is_dead() || self.halted() {
            return;
        }
        let party = game.party();
        let pokemon: Vec<_> = party.iter().filter(|p| !p.is_egg).collect();
        if pokemon.is_empty() || pokemon.iter().any(|p| !self.rules.is_dead(p.pid)) {
            return;
        }
        let Some(last) = self.last_death.clone() else {
            return;
        };
        self.record(RunEvent::PartyWiped { last });
        let offered = self.view.wipe.as_ref().is_some_and(|w| w.retry_offered);
        if !self.with_friends && !offered {
            self.end_run();
        }
    }

    /// Updates what the game over needs with an event.
    pub(super) fn take_in_ending(&mut self, recorded: &Recorded) {
        match &recorded.event {
            RunEvent::Caught { .. } | RunEvent::Gift { .. } | RunEvent::Traded { .. } | RunEvent::Released { .. } => {
                self.lives.push(recorded.event.clone());
            }
            RunEvent::Died { name, area, cause, .. } => {
                self.lives.push(recorded.event.clone());
                self.last_death = Some(LastDeath {
                    name: name.clone(),
                    area: area.clone(),
                    cause: cause.clone(),
                });
            }
            RunEvent::PartyWiped { last } => {
                let retry_offered =
                    !self.with_friends && self.retries_left() > 0 && self.latest_run_checkpoint.is_some();
                self.wiped_at_ms = Some(recorded.at_ms);
                self.view.wipe = Some(WipeView {
                    how: last.wipe_line("Your"),
                    retry_offered,
                });
            }
            RunEvent::GameOver { how, longest, play_ms } => {
                self.view.game_over = Some(GameOverView {
                    how: how.clone(),
                    longest: longest.clone(),
                    at_ms: recorded.at_ms,
                    play_ms: *play_ms,
                });
            }
            _ => {}
        }
    }
}
