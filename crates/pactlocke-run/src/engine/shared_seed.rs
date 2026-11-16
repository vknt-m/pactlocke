//! Shared Seed through a Romhack's own randomizer: every Player's game is
//! given the Run's seed on the overworld, and given it again whenever it
//! changes; where the Romhack draws trainer teams from the live RNG, the RNG is
//! pinned from a trainer battle's trigger until its party is made.

use super::RunEngine;
use crate::{LiveGame, RunEvent};

impl RunEngine {
    /// Gives the game the Run's seed, if the Ruleset shares one and the game
    /// doesn't hold it already.
    pub(super) fn keep_shared_seed(&mut self, game: &mut impl LiveGame) {
        let Some(seed) = self.info.ruleset.romhack_seed else {
            return;
        };
        match game.share_seed(seed) {
            Ok(changes) => {
                self.apply(changes, game);
                if !self.rules.seed_shared_said() {
                    self.record(RunEvent::SeedShared);
                }
            }
            Err(e) => {
                if !self.rules.seed_failure_said() {
                    self.record(RunEvent::SeedShareFailed { reason: e.to_string() });
                }
            }
        }
    }

    /// Pins the RNG at this frame boundary while a trainer battle is coming.
    pub(super) fn pin_the_rng(&mut self, game: &mut impl LiveGame) {
        let Some(seed) = self.info.ruleset.romhack_seed else {
            return;
        };
        // A game with nothing to pin, or no battle coming, asks for nothing.
        if let Ok(changes) = game.pin_rng(seed) {
            self.apply(changes, game);
        }
    }
}
