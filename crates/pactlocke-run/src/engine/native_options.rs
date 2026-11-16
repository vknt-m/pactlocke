//! Native options: rules a game does through its own options. Pactlocke
//! doesn't enforce those rules itself; it sets the option at the first
//! overworld frame of each boot, sets a watched one back whenever the Player
//! changes it, and shows what the game reads.

use pactlocke_game::{NativeOption, Rule};

use super::RunEngine;
use crate::{LiveGame, NativeOptionView, RunEvent};

impl RunEngine {
    /// Asks the game once which rules it does through its own options, and
    /// shows them in the side panel from then on.
    pub(super) fn learn_natives(&mut self, game: &mut impl LiveGame) {
        if self.natives.is_none() {
            self.natives = Some(game.native_options());
            self.look_at_the_native_options(game);
        }
    }

    /// Whether the game does `rule` itself, through its own option, in place of Pactlocke.
    pub(super) fn is_native(&self, rule: Rule) -> bool {
        self.natives_in_use()
            .iter()
            .any(|native| native.rule == rule && !native.pactlocke_too)
    }

    /// The native options of the rules this Ruleset has on.
    fn natives_in_use(&self) -> Vec<NativeOption> {
        crate::native_options_in_use(&self.info.ruleset, self.natives.iter().flatten())
    }

    /// Sets every native option the Ruleset needs at the first overworld
    /// frame of a boot, and a watched one again whenever it changes.
    pub(super) fn keep_native_options(&mut self, game: &mut impl LiveGame) {
        let first_since_boot = !self.natives_set_this_boot;
        self.natives_set_this_boot = true;
        for native in self.natives_in_use() {
            if !first_since_boot && !native.watched {
                continue;
            }
            let rule = native.rule.name().to_owned();
            if game.option(&native.option).as_deref() != Some(native.value.as_str()) {
                match game.set_option(&native.option, &native.value) {
                    Ok(changes) => {
                        self.apply(changes, game);
                        if !first_since_boot {
                            self.record(RunEvent::NativeOptionSet {
                                rule: rule.clone(),
                                name: native.name.clone(),
                                value: native.value.clone(),
                                again: true,
                                pactlocke_too: native.pactlocke_too,
                            });
                        }
                    }
                    Err(e) => {
                        if !self.rules.native_failure_said(&native.name) {
                            self.record(RunEvent::NativeOptionFailed {
                                rule,
                                name: native.name,
                                reason: e.to_string(),
                            });
                        }
                        continue;
                    }
                }
            }
            if first_since_boot && !self.rules.native_said(&native.name) {
                self.record(RunEvent::NativeOptionSet {
                    rule,
                    name: native.name,
                    value: native.value,
                    again: false,
                    pactlocke_too: native.pactlocke_too,
                });
            }
        }
    }

    /// What the game reads for each native option in use, for the side panel.
    pub(super) fn look_at_the_native_options(&mut self, game: &mut impl LiveGame) {
        let seen: Vec<NativeOptionView> = self
            .natives_in_use()
            .into_iter()
            .map(|native| {
                let read = game.option(&native.option);
                NativeOptionView::of(&native, read)
            })
            .collect();
        if seen != self.view.native_options {
            self.view.native_options = seen;
            self.view_version += 1;
        }
    }
}
