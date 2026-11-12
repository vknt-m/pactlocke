//! No items and set mode: both put in place on the overworld, before any
//! battle. The item patches only bite in battle, so they stay on for the
//! whole boot; the battle style is written back whenever it isn't Set.

use super::RunEngine;
use crate::{LiveGame, RunEvent};

impl RunEngine {
    /// Refuses the Bag in trainer battles and bans every item but Poké Balls
    /// in wild battles, once per boot (ROM patches don't survive one).
    pub(super) fn keep_items_banned(&mut self, game: &mut impl LiveGame) {
        if !self.rules.no_items() || self.items_patched {
            return;
        }
        self.items_patched = true;
        let results = [
            game.lock_bag_in_trainer_battles(true),
            game.ban_items_in_wild_battles(true),
        ];
        let mut failure = None;
        for result in results {
            match result {
                Ok(changes) => self.apply(changes, game),
                Err(e) => failure = failure.or(Some(e.to_string())),
            }
        }
        match failure {
            Some(reason) if !self.rules.item_ban_failed_said() => self.record(RunEvent::ItemBanFailed { reason }),
            None if !self.rules.items_banned_said() => self.record(RunEvent::ItemsBanned),
            _ => {}
        }
    }

    /// Writes the battle style back to Set whenever it isn't.
    pub(super) fn keep_set_mode(&mut self, game: &mut impl LiveGame) {
        if !self.rules.set_mode() {
            return;
        }
        match game.force_set_mode() {
            Ok(changes) if changes.is_empty() => {}
            Ok(changes) => {
                self.apply(changes, game);
                let again = self.rules.set_mode_forced_before();
                self.record(RunEvent::SetModeForced { again });
            }
            Err(e) if !self.rules.set_mode_failed_said() => {
                self.record(RunEvent::SetModeFailed { reason: e.to_string() });
            }
            Err(_) => {}
        }
    }
}
