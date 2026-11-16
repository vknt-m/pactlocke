//! How a Run ended: the line that says whose party was wiped, where and by
//! what, and the Pokémon that survived the longest, worked out from the
//! Run's events (a solo Run's own, or the shared Run log's).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{AreaRef, DeathCause, RunEvent};

/// A Pokémon that survived longest for its Player: from being caught or
/// given to its Death, or to the end of the Run for one still alive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Survivor {
    /// Its Player's name; `None` for this Player's own in a solo Run.
    pub player: Option<String>,
    /// Its nickname at its Death, or its species name.
    pub name: String,
    /// Its species name, as the game shows it.
    pub species_name: String,
    /// Milliseconds it lived in the Run.
    pub lived_ms: i64,
    /// Still alive when the Run ended.
    pub alive: bool,
}

/// The last of a Player's Pokémon to die: its name, where, and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastDeath {
    /// Its nickname, or its species name.
    pub name: String,
    /// The Area it died in, if the Game Profile lists it.
    pub area: Option<AreaRef>,
    /// What made it faint.
    pub cause: DeathCause,
}

impl LastDeath {
    /// "Your party was wiped on Route 2: PIDGEY fainted against a trainer's
    /// CHARMANDER". `whose` is "Your" or "Red's".
    pub fn wipe_line(&self, whose: &str) -> String {
        let place = match &self.area {
            Some(area) => format!(" {}", crate::view::place(area)),
            None => String::new(),
        };
        let last = &self.name;
        let how = match &self.cause {
            DeathCause::Wild { foe } => format!("{last} fainted against a wild {foe}"),
            DeathCause::Trainer { foe } => format!("{last} fainted against a trainer's {foe}"),
            DeathCause::Poison => format!("{last} fainted from poison"),
            DeathCause::Link { why } => format!("{last} died with its Link: {why}"),
        };
        format!("{whose} party was wiped{place}: {how}")
    }
}

/// Each Player's longest survivor, in the order the Players first got a
/// Pokémon, from their events in Run order. `end_ms` is when the Run ended.
/// A Pokémon released lived until then, and isn't alive at the end.
pub fn survivors<'a>(events: impl IntoIterator<Item = (Option<String>, &'a RunEvent)>, end_ms: i64) -> Vec<Survivor> {
    struct Life {
        name: String,
        species_name: String,
        from_ms: i64,
        died_ms: Option<i64>,
        released: bool,
    }
    let mut players: Vec<Option<String>> = Vec::new();
    let mut lives: BTreeMap<(Option<String>, u32), Life> = BTreeMap::new();
    for (player, event) in events {
        match event {
            RunEvent::Caught {
                pid,
                species_name,
                at_ms,
                ..
            }
            | RunEvent::Gift {
                pid,
                species_name,
                at_ms,
                ..
            }
            | RunEvent::Traded {
                pid,
                species_name,
                at_ms,
                ..
            } => {
                if !players.contains(&player) {
                    players.push(player.clone());
                }
                lives.entry((player, *pid)).or_insert(Life {
                    name: species_name.clone(),
                    species_name: species_name.clone(),
                    from_ms: *at_ms,
                    died_ms: None,
                    released: false,
                });
            }
            RunEvent::Died { pid, name, at_ms, .. } => {
                if let Some(life) = lives.get_mut(&(player, *pid)) {
                    life.name = name.clone();
                    life.died_ms.get_or_insert(*at_ms);
                }
            }
            RunEvent::Released { pid, at_ms, .. } => {
                if let Some(life) = lives.get_mut(&(player, *pid)) {
                    life.released = true;
                    life.died_ms.get_or_insert(*at_ms);
                }
            }
            _ => {}
        }
    }
    players
        .into_iter()
        .filter_map(|player| {
            lives
                .iter()
                .filter(|((p, _), _)| *p == player)
                .map(|(_, life)| Survivor {
                    player: player.clone(),
                    name: life.name.clone(),
                    species_name: life.species_name.clone(),
                    lived_ms: (life.died_ms.unwrap_or(end_ms) - life.from_ms).max(0),
                    alive: life.died_ms.is_none() && !life.released,
                })
                .max_by_key(|s| s.lived_ms)
        })
        .collect()
}
