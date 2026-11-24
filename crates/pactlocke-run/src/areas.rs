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
    /// Its species name, as the game shows it.
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

/// The most wild battles since an Area's Encounter the Area status keeps.
pub const RECENT_BATTLES: usize = 10;

/// Whether the Area this Player is in can still give them a catch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaState {
    /// The next counted wild battle here is the Area's Encounter.
    CatchAllowed,
    /// The Area's Encounter is used: ball throws are refused here.
    EncounterDone,
    /// The last wild battle here was a dupe: not counted, so the Encounter stays open.
    Dupe,
    /// The Player has no Poké Balls yet, so wild battles don't count.
    NotCountedYet,
}

/// How a wild battle in an Area went, other than its Encounter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Met {
    /// After the Area's Encounter: ball throws were refused.
    AfterEncounter,
    /// The dupes clause: a family the Player has alive, here `of`.
    Dupe {
        /// The species of that family the Player has alive.
        of: String,
    },
    /// Before the Player had Poké Balls.
    NotCounted,
}

/// A wild battle in an Area that wasn't its Encounter, none of them caught.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaBattle {
    /// The wild Pokémon's species number, for its party icon.
    pub species: u16,
    /// Its species name, as the game shows it.
    pub species_name: String,
    /// How the battle went.
    pub met: Met,
}

impl AreaBattle {
    /// What happened, for the icon's hover text: "a dupe of your PIDGEOTTO".
    pub fn what(&self) -> String {
        match &self.met {
            Met::AfterEncounter => "met after the Encounter, not caught".to_owned(),
            Met::Dupe { of } => format!("a dupe of your {of}"),
            Met::NotCounted => "no Poké Balls yet".to_owned(),
        }
    }
}

/// What the Area status shows of the Area this Player is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaStatus {
    /// The Area, by its shown name.
    pub area: String,
    /// Whether a catch is allowed here.
    pub state: AreaState,
    /// The Area's Encounter, once there is one.
    pub encounter: Option<AreaEncounter>,
    /// The wild battles here since the Encounter (before it, the uncounted
    /// ones and the dupes), newest first, at most [`RECENT_BATTLES`].
    pub battles: Vec<AreaBattle>,
    /// Every wild battle here so far, the Encounter's too: the overlay shows again at each.
    pub met: usize,
}

impl AreaStatus {
    /// The state in a few words: "Catch allowed".
    pub fn say(&self) -> &'static str {
        match self.state {
            AreaState::CatchAllowed => "Catch allowed",
            AreaState::EncounterDone => "Encounter done",
            AreaState::Dupe => "Dupe, not counted",
            AreaState::NotCountedYet => "Not counted yet",
        }
    }

    /// Why, as a sentence or two.
    pub fn why(&self) -> String {
        const REFUSED: &str = "Ball throws are refused; running is fine.";
        match self.state {
            AreaState::CatchAllowed => "The first wild battle here is this Area's Encounter.".to_owned(),
            AreaState::EncounterDone => match &self.encounter {
                Some(e) if e.caught => format!("Caught {} here. {REFUSED}", e.species_name),
                Some(e) => format!("Met {} here, not caught. {REFUSED}", e.species_name),
                None => format!("This Area's Encounter is used. {REFUSED}"),
            },
            AreaState::Dupe => match self.battles.first() {
                Some(AreaBattle {
                    species_name,
                    met: Met::Dupe { of },
                    ..
                }) => format!("{species_name} is a dupe of your {of}. The Encounter stays open for the next battle."),
                _ => "A dupe isn't counted. The Encounter stays open for the next battle.".to_owned(),
            },
            AreaState::NotCountedYet => "Wild battles count once you have Poké Balls.".to_owned(),
        }
    }
}

/// This Player's wild battles in each Area, by the Area's shown name, for the Area status.
#[derive(Debug, Clone, Default)]
pub(crate) struct AreaBattles {
    /// Each Area's battles since its Encounter, newest last.
    since: BTreeMap<String, Vec<AreaBattle>>,
    /// Every wild battle in each Area.
    met: BTreeMap<String, usize>,
}

impl AreaBattles {
    /// Takes one of this Player's Run events, in order.
    pub(crate) fn take(&mut self, event: &RunEvent) {
        let (area, battle) = match event {
            RunEvent::Encounter { area, .. } => {
                self.since.remove(&area.name);
                *self.met.entry(area.name.clone()).or_default() += 1;
                return;
            }
            RunEvent::CatchesBlocked {
                area,
                species,
                species_name,
            } => (area, (*species, species_name, Met::AfterEncounter)),
            RunEvent::DupeSkipped {
                area,
                species,
                species_name,
                dupe_of,
            } => (area, (*species, species_name, Met::Dupe { of: dupe_of.clone() })),
            RunEvent::NotCounted {
                area,
                species,
                species_name,
            } => (area, (*species, species_name, Met::NotCounted)),
            _ => return,
        };
        let (species, species_name, met) = battle;
        *self.met.entry(area.name.clone()).or_default() += 1;
        let since = self.since.entry(area.name.clone()).or_default();
        since.push(AreaBattle {
            species,
            species_name: species_name.clone(),
            met,
        });
        if since.len() > RECENT_BATTLES {
            since.remove(0);
        }
    }

    /// The Area status of `area`: `closed` when its Encounter is used,
    /// `counting` once the Player has Poké Balls. `encounters` are this
    /// Player's Area Encounters.
    pub(crate) fn status(&self, area: &str, closed: bool, counting: bool, encounters: &[AreaEncounter]) -> AreaStatus {
        let battles: Vec<AreaBattle> = self.since.get(area).into_iter().flatten().rev().cloned().collect();
        let state = if !counting {
            AreaState::NotCountedYet
        } else if closed {
            AreaState::EncounterDone
        } else if matches!(
            battles.first(),
            Some(AreaBattle {
                met: Met::Dupe { .. },
                ..
            })
        ) {
            AreaState::Dupe
        } else {
            AreaState::CatchAllowed
        };
        AreaStatus {
            area: area.to_owned(),
            state,
            encounter: encounters.iter().rev().find(|e| e.area == area).cloned(),
            battles,
            met: self.met.get(area).copied().unwrap_or(0),
        }
    }
}
