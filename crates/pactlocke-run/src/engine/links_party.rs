//! Links keep the party for linked Pokémon: those whose Link is whole. A
//! catch waiting on another Player's stays in a party of fewer than six;
//! at six, the newest waiting one goes to the PC, one at a time, until the
//! party has five. The party's last living Pokémon is never moved.

use super::{RunEngine, display_name};
use crate::{LinkStanding, LiveGame, RunEvent};

impl RunEngine {
    /// Where the Player's Pokémon stand in their Links now.
    pub fn link_standing(&mut self, standing: LinkStanding) {
        self.link_standing = standing;
    }

    /// At a quiet moment: while the party is full, moves its newest Pokémon
    /// waiting on its Link to the PC. It waits while a catch in the party
    /// isn't in the Links yet, so the newest is known.
    pub(super) fn keep_party_for_links(&mut self, game: &mut impl LiveGame) {
        if self.link_standing.waiting.is_empty() {
            return;
        }
        loop {
            let party = game.party();
            if party.len() < crate::PARTY_SIZE {
                return;
            }
            let unknown = party
                .iter()
                .any(|p| self.link_members.contains(&p.pid) && !self.link_standing.linked.contains(&p.pid));
            if unknown {
                return;
            }
            let living = party.iter().filter(|p| !p.is_egg && !self.rules.is_dead(p.pid)).count();
            let newest = party
                .iter()
                .enumerate()
                .filter(|(_, p)| !p.is_egg && !self.rules.is_dead(p.pid) && !self.link_wait_failed.contains(&p.pid))
                .filter_map(|(slot, p)| {
                    let waiting = self.link_standing.waiting.iter().find(|w| w.pid == p.pid)?;
                    Some((slot, p.clone(), waiting.until.clone()))
                })
                .max_by_key(|(_, p, _)| self.got_order.iter().position(|pid| *pid == p.pid));
            let Some((slot, pokemon, until)) = newest else {
                return;
            };
            if living <= 1 {
                return;
            }
            let name = display_name(&pokemon, &game.species_name(pokemon.species));
            let slot = u8::try_from(slot).expect("a party has six slots");
            match super::nicknames_and_types::move_to_the_first_box_with_room(game, slot) {
                Ok((changes, box_number)) => {
                    self.apply(changes, game);
                    self.record(RunEvent::WaitsForLink {
                        pid: pokemon.pid,
                        name,
                        until,
                        box_number,
                    });
                }
                Err(e) => {
                    self.link_wait_failed.insert(pokemon.pid);
                    self.record(RunEvent::LinkWaitFailed {
                        pid: pokemon.pid,
                        name,
                        reason: e.to_string(),
                    });
                }
            }
        }
    }
}
