//! Must nickname and the type clause. The nickname prompt's B and DOWN are
//! dropped before each frame runs; a Pokémon left with its species name is
//! named from the side panel. A Pokémon new to the party whose primary type
//! the party already holds goes to the PC. Both act at quiet moments on the
//! overworld, after the game's own naming screen and menus have had their turn.

use pactlocke_emu::Keys;
use pactlocke_game::{GameError, Pokemon};

use super::{RunEngine, display_name};
use crate::{LiveGame, RunEvent};

/// How often a Pokémon still waiting for a name is looked at, in case the
/// Player named it in the game (the Name Rater) instead.
const WANTED_CHECK_FRAMES: u64 = 60;

impl RunEngine {
    /// The keys the game gets for the frame about to run: with must nickname
    /// on, "Give a nickname?" can't be declined.
    pub fn keys_for_frame(&mut self, keys: Keys, game: &mut impl LiveGame) -> Keys {
        if !self.rules.must_nickname() || !game.nickname_prompt_up() {
            return keys;
        }
        if let Ok(changes) = game.keep_nickname_cursor_on_yes() {
            self.apply(changes, game);
        }
        keys.without(Keys::B | Keys::DOWN)
    }

    /// The Player named a Pokémon the side panel asked about. A blank name or
    /// the species name is refused at once; a good one is written at the
    /// next quiet moment on the overworld, wherever the Pokémon is by then.
    pub fn name_pokemon(&mut self, pid: u32, name: &str, game: &mut impl LiveGame) {
        let Some(species_name) = self.rules.nickname_wanted(pid).map(str::to_owned) else {
            return;
        };
        let name = name.trim();
        if name.is_empty() || name.eq_ignore_ascii_case(&species_name) {
            self.record(RunEvent::NicknameRefused {
                pid,
                species_name,
                name: name.to_owned(),
                reason: "a nickname can't be blank or the species name".to_owned(),
            });
            return;
        }
        self.names_given.push((pid, name.to_owned()));
        if self.quiet(game) {
            self.write_names_given(game);
        }
    }

    /// Writes the names the Player gave, now that the game is quiet.
    pub(super) fn write_names_given(&mut self, game: &mut impl LiveGame) {
        for (pid, name) in std::mem::take(&mut self.names_given) {
            let Some(species_name) = self.rules.nickname_wanted(pid).map(str::to_owned) else {
                continue;
            };
            let in_party = game.party().iter().position(|p| p.pid == pid);
            let changes = match in_party {
                Some(slot) => game.write_nickname(u8::try_from(slot).expect("a party has six slots"), &name),
                None => match game.pc().into_iter().find(|s| s.pokemon.pid == pid) {
                    Some(stored) => game.write_pc_nickname(stored.box_number, stored.slot, &name),
                    None => Err(GameError::BadName("it isn't in the party or the PC".to_owned())),
                },
            };
            match changes {
                Ok(changes) => {
                    self.apply(changes, game);
                    self.record(RunEvent::Nicknamed {
                        pid,
                        species_name,
                        name,
                    });
                }
                Err(e) => self.record(RunEvent::NicknameRefused {
                    pid,
                    species_name,
                    name,
                    reason: e.to_string(),
                }),
            }
        }
    }

    /// Asks for a name for each Pokémon caught or given that still has its
    /// species name, and takes a name given in the game for one already asked about.
    pub(super) fn check_nicknames(&mut self, game: &mut impl LiveGame) {
        let recheck = self.rules.any_nickname_wanted() && self.frame.is_multiple_of(WANTED_CHECK_FRAMES);
        if self.names_to_check.is_empty() && !recheck {
            return;
        }
        let party = game.party();
        let pc = game.pc().into_iter().map(|stored| stored.pokemon);
        let owned: Vec<Pokemon> = party.into_iter().chain(pc).collect();
        let mut pids = std::mem::take(&mut self.names_to_check);
        if recheck {
            pids.extend(self.rules.nicknames_wanted());
        }
        for pid in pids {
            let Some(pokemon) = owned.iter().find(|p| p.pid == pid).filter(|p| !p.is_egg) else {
                continue;
            };
            let species_name = game.species_name(pokemon.species);
            let unnamed = pokemon.nickname.trim().is_empty() || pokemon.nickname == species_name;
            match (unnamed, self.rules.nickname_wanted(pid).is_some()) {
                (true, false) => self.record(RunEvent::NicknameWanted { pid, species_name }),
                (false, true) => self.record(RunEvent::Nicknamed {
                    pid,
                    species_name,
                    name: pokemon.nickname.clone(),
                }),
                _ => {}
            }
        }
    }

    /// Moves a Pokémon new to the party since the last look to the PC when
    /// its primary type (read from the loaded ROM) is already held there.
    /// Eggs and the Dead hold no type. The first look after a boot only sees
    /// who is in the party.
    pub(super) fn keep_type_clause(&mut self, game: &mut impl LiveGame) {
        if !self.rules.type_clause() {
            return;
        }
        loop {
            let party = game.party();
            let Some(before) = self.party_before.clone() else {
                self.party_before = Some(party.iter().map(|p| p.pid).collect());
                return;
            };
            let holds_a_type = |p: &&Pokemon| !p.is_egg && !self.rules.is_dead(p.pid);
            // Those already in the party hold their types, then newcomers in party order.
            let mut holders: Vec<(u8, Pokemon)> = Vec::new();
            for pokemon in party.iter().filter(holds_a_type).filter(|p| before.contains(&p.pid)) {
                holders.push((game.primary_type(pokemon.species), pokemon.clone()));
            }
            let mut second = None;
            for (slot, pokemon) in party.iter().enumerate() {
                if !holds_a_type(&pokemon)
                    || before.contains(&pokemon.pid)
                    || self.type_clause_failed.contains(&pokemon.pid)
                {
                    continue;
                }
                let primary = game.primary_type(pokemon.species);
                match holders.iter().find(|(held, _)| *held == primary) {
                    Some((_, holder)) => {
                        second = Some(SecondOfAType {
                            slot,
                            pokemon: pokemon.clone(),
                            holder: holder.clone(),
                            primary,
                        });
                        break;
                    }
                    None => holders.push((primary, pokemon.clone())),
                }
            }
            let Some(second) = second else {
                self.party_before = Some(party.iter().map(|p| p.pid).collect());
                return;
            };
            self.move_second_of_a_type(game, &second);
        }
    }

    fn move_second_of_a_type(&mut self, game: &mut impl LiveGame, second: &SecondOfAType) {
        let pokemon = &second.pokemon;
        let name = display_name(pokemon, &game.species_name(pokemon.species));
        let holder = display_name(&second.holder, &game.species_name(second.holder.species));
        let type_name = game.type_name(second.primary);
        let slot = u8::try_from(second.slot).expect("a party has six slots");
        match move_to_the_first_box_with_room(game, slot) {
            Ok((changes, box_number)) => {
                self.apply(changes, game);
                self.record(RunEvent::TypeClauseMoved {
                    pid: pokemon.pid,
                    name,
                    holder,
                    type_name,
                    box_number,
                });
            }
            Err(e) => {
                self.type_clause_failed.insert(pokemon.pid);
                self.record(RunEvent::TypeClauseFailed {
                    pid: pokemon.pid,
                    name,
                    type_name,
                    reason: e.to_string(),
                });
            }
        }
    }
}

/// A Pokémon new to the party whose primary type `holder` already holds there.
struct SecondOfAType {
    slot: usize,
    pokemon: Pokemon,
    holder: Pokemon,
    primary: u8,
}

/// Moves a party Pokémon to the first PC box with room, counting from box 1
/// and passing over the Dead box, which is for the Dead.
fn move_to_the_first_box_with_room(
    game: &mut impl LiveGame,
    slot: u8,
) -> Result<(Vec<pactlocke_game::Change>, u8), GameError> {
    let dead_box = game.dead_box();
    let mut box_number = 1;
    loop {
        if box_number == dead_box {
            box_number += 1;
            continue;
        }
        match game.move_to_box(slot, box_number) {
            Ok(changes) => return Ok((changes, box_number)),
            Err(GameError::BoxFull(_)) => box_number += 1,
            Err(e) => return Err(e),
        }
    }
}
