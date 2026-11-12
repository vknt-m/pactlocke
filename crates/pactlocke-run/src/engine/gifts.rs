//! Gifts and in-game trades: a new PID in the party or PC while a script
//! runs, matched to the Game Profile's catalogue, and the release of Gifts the
//! Ruleset doesn't allow.

use std::collections::{BTreeMap, BTreeSet};

use pactlocke_game::{Pokemon, ScriptStatus};

use super::{RunEngine, display_name};
use crate::event::{AreaRef, SourceRef};
use crate::rules::{GiftOrigin, GiftVerdict};
use crate::{LiveGame, RunEvent};

/// The Player's Pokémon as the engine last saw them, to tell a Gift or a
/// trade from a Pokémon it already knew.
#[derive(Default)]
pub(super) struct SeenPokemon {
    /// Every PID seen in the party or PC since the game was booted, so a
    /// Pokémon coming back (from the Day Care, say) isn't taken for a Gift.
    pids: BTreeSet<u32>,
    /// The party's PIDs by slot, to tell a trade from a Gift.
    party: Vec<u32>,
    /// The party's names by PID, to say what was traded away.
    names: BTreeMap<u32, String>,
}

impl SeenPokemon {
    fn take_in(&mut self, party: &[Pokemon], pc: &[Pokemon], game: &mut impl LiveGame) {
        self.pids.extend(party.iter().chain(pc).map(|p| p.pid));
        self.party = party.iter().map(|p| p.pid).collect();
        self.names = party
            .iter()
            .map(|p| (p.pid, display_name(p, &game.species_name(p.species))))
            .collect();
    }
}

impl RunEngine {
    /// Out of battle, while a script runs, looks for Pokémon that weren't
    /// there before. The first safe overworld frame sets what was there
    /// before, since a game not yet loaded reads as garbage.
    pub(super) fn watch_gifts(&mut self, game: &mut impl LiveGame) {
        if self.known.is_none() {
            if game.at_safe_overworld_point() {
                let (party, pc) = party_and_pc(game);
                let mut known = SeenPokemon::default();
                known.take_in(&party, &pc, game);
                self.known = Some(known);
            }
            return;
        }
        let script = game.script();
        if script.status == ScriptStatus::Idle {
            return;
        }
        let (party, pc) = party_and_pc(game);
        let Some(known) = self.known.as_ref() else {
            return;
        };
        let still_here: BTreeSet<u32> = party.iter().chain(&pc).map(|p| p.pid).collect();
        // A trade puts the new Pokémon in the party slot of the one given away.
        let traded_away = |slot: usize| known.party.get(slot).filter(|old| !still_here.contains(old));
        let new: Vec<(Option<String>, Pokemon)> = party
            .iter()
            .enumerate()
            .map(|(slot, p)| (traded_away(slot), p))
            .chain(pc.iter().map(|p| (None, p)))
            .filter(|(_, p)| !known.pids.contains(&p.pid))
            .map(|(old, p)| (old.and_then(|old| known.names.get(old).cloned()), p.clone()))
            .collect();

        let area = game.area().as_ref().map(AreaRef::from);
        for (gave, pokemon) in new {
            let species_name = game.species_name(pokemon.species);
            let (origin, event) = match gave {
                Some(gave) => (
                    GiftOrigin::Trade,
                    RunEvent::Traded {
                        gave,
                        pid: pokemon.pid,
                        species: pokemon.species,
                        species_name: species_name.clone(),
                        area: area.clone(),
                    },
                ),
                None => {
                    let source = source_of(game, script.position);
                    let origin = match &source {
                        Some(source) => GiftOrigin::Listed(source.clone()),
                        None => GiftOrigin::Unknown,
                    };
                    let event = RunEvent::Gift {
                        pid: pokemon.pid,
                        species: pokemon.species,
                        species_name: species_name.clone(),
                        area: area.clone(),
                        starter: matches!(source, Some(ref s) if s.kind == pactlocke_game::SourceKind::Starter),
                        source: source.map(SourceRef::from),
                    };
                    (origin, event)
                }
            };
            let verdict = self.rules.gift_received(&origin);
            self.record(event);
            if let GiftVerdict::Decline(reason) = verdict {
                self.record(RunEvent::GiftDeclined {
                    pid: pokemon.pid,
                    species_name,
                    reason,
                });
            }
        }
        if let Some(known) = self.known.as_mut() {
            known.take_in(&party, &pc, game);
        }
    }

    /// After a battle, the Pokémon caught in it are known, not Gifts.
    pub(super) fn see_the_pokemon_after_battle(&mut self, game: &mut impl LiveGame) {
        if self.known.is_none() {
            return;
        }
        let (party, pc) = party_and_pc(game);
        if let Some(known) = self.known.as_mut() {
            known.take_in(&party, &pc, game);
        }
    }

    /// Releases each declined Gift, from the party or the PC.
    pub(super) fn release_the_declined(&mut self, game: &mut impl LiveGame) {
        if !self.rules.any_to_release() {
            return;
        }
        loop {
            let party = game.party();
            let in_party = party
                .iter()
                .position(|p| self.rules.to_release(p.pid) && !self.release_failed.contains(&p.pid));
            let (pokemon, changes) =
                if let Some(slot) = in_party {
                    let slot_number = u8::try_from(slot).expect("a party has six slots");
                    (party[slot].clone(), game.release_from_party(slot_number))
                } else {
                    let Some(stored) = game.pc().into_iter().find(|s| {
                        self.rules.to_release(s.pokemon.pid) && !self.release_failed.contains(&s.pokemon.pid)
                    }) else {
                        return;
                    };
                    let changes = game.release_from_pc(stored.box_number, stored.slot);
                    (stored.pokemon, changes)
                };
            let species_name = game.species_name(pokemon.species);
            match changes {
                Ok(changes) => {
                    self.apply(changes, game);
                    self.record(RunEvent::Released {
                        pid: pokemon.pid,
                        species_name,
                    });
                }
                Err(e) => {
                    self.release_failed.insert(pokemon.pid);
                    self.record(RunEvent::ReleaseFailed {
                        pid: pokemon.pid,
                        species_name,
                        reason: e.to_string(),
                    });
                }
            }
        }
    }
}

/// The catalogued source a script at `position` belongs to: the instruction
/// nearest before it, or before one of the places it was called from.
pub(super) fn source_of(game: &mut impl LiveGame, position: Option<u32>) -> Option<pactlocke_game::Source> {
    let calls = game.script_calls();
    position.into_iter().chain(calls).find_map(|at| game.source_before(at))
}

fn party_and_pc(game: &mut impl LiveGame) -> (Vec<Pokemon>, Vec<Pokemon>) {
    let party = game.party();
    let pc = game.pc().into_iter().map(|s| s.pokemon).collect();
    (party, pc)
}
