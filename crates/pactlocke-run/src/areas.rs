//! Each Player's Area Encounters: in every Area they used, the Pokémon they
//! met there and whether they caught it. This Player's own come from their
//! Run; the others' from the shared Run log, both through `take`.

use std::collections::BTreeMap;

use crate::RunEvent;
use crate::shared_log::RunLog;
use crate::stream::PlayerKey;

/// One Area's Encounter, as a Player had it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaEncounter {
    /// The Area, by its shown name.
    pub area: String,
    /// The Pokémon met there.
    pub species: u16,
    pub species_name: String,
    /// Whether the Player caught it.
    pub caught: bool,
}

impl AreaEncounter {
    /// Takes one of a Player's Run events, in order, into their Area Encounters.
    pub fn take(encounters: &mut Vec<AreaEncounter>, event: &RunEvent) {
        match event {
            RunEvent::Encounter {
                area,
                species,
                species_name,
            } => encounters.push(AreaEncounter {
                area: area.name.clone(),
                species: *species,
                species_name: species_name.clone(),
                caught: false,
            }),
            // A catch in an Area is its Encounter's: ball throws are refused there after it.
            RunEvent::Caught { area: Some(area), .. } => {
                if let Some(encounter) = encounters.iter_mut().rev().find(|e| e.area == area.name) {
                    encounter.caught = true;
                }
            }
            _ => {}
        }
    }
}

/// Every Player's Area Encounters, as the shared Run log has them.
pub fn area_encounters(log: &RunLog) -> BTreeMap<PlayerKey, Vec<AreaEncounter>> {
    let mut all: BTreeMap<PlayerKey, Vec<AreaEncounter>> = BTreeMap::new();
    for (author, event) in log.standing_events() {
        AreaEncounter::take(all.entry(author).or_default(), &event);
    }
    all
}
