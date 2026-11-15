//! Level caps: the cap for the Player's stage patched into the game's
//! level checks, rewritten when they pass a stage, and the soft cap where
//! the game can't be patched.

use pactlocke_game::{Game, MAX_LEVEL};

use super::{RunEngine, display_name};
use crate::{LiveGame, RunEvent};

impl RunEngine {
    /// Patches in the cap for the Player's stage when it isn't in already.
    /// The stage never goes back, though the game clears the League's
    /// defeat flags when the Player goes in again. Where the game can't be
    /// patched it is a soft cap, said once.
    pub(super) fn keep_level_cap(&mut self, game: &mut impl LiveGame) {
        if self.info.ruleset.level_caps.is_none() || self.is_native(pactlocke_game::Rule::LevelCap) {
            return;
        }
        let stage = game.cap_stage().max(self.rules.cap_stage().unwrap_or(0));
        let caps = self.info.ruleset.level_caps.as_ref().expect("checked above");
        let cap = caps.get(stage).cloned();
        let level = cap.as_ref().map_or(MAX_LEVEL, |cap| cap.level);
        if self.cap_patched_for != Some(level) {
            self.cap_patched_for = Some(level);
            match game.set_level_cap(level) {
                Ok(changes) => self.apply(changes, game),
                Err(e) if !self.rules.is_soft_cap() => self.record(RunEvent::SoftCap { reason: e.to_string() }),
                Err(_) => {}
            }
        }
        if self.rules.cap_stage() != Some(stage) {
            let raised = self.rules.cap_stage().is_some();
            self.record(RunEvent::LevelCapSet {
                stage,
                level,
                next: cap.map(|cap| cap.name),
                raised,
            });
        }
    }

    /// Why the Player can't ready up at a Sync Gate, if they can't: a living
    /// party Pokémon over the level cap.
    pub fn ready_up_refusal(&self, game: &mut impl Game) -> Option<String> {
        let cap = self.rules.level_cap()?;
        let over = game.party().into_iter().find(|pokemon| {
            !pokemon.is_egg && !self.rules.is_dead(pokemon.pid) && pokemon.stats.is_some_and(|s| s.level > cap)
        })?;
        let name = display_name(&over, &game.species_name(over.species));
        let level = over.stats.map_or(0, |s| s.level);
        Some(format!("{name} is level {level}, over the level cap of {cap}"))
    }
}
