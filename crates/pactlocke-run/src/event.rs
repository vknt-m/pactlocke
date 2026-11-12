use pactlocke_game::Area;
use serde::{Deserialize, Serialize};

/// Something that happened in the Run, as the Run log records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    /// The Run began.
    Started { game_name: String },
    /// The Run was opened again; the game was booted afresh.
    Resumed,
    /// The Player first holds Poké Balls; wild Encounters count from here on.
    PokeBallsObtained,
    /// A wild battle before the Player had Poké Balls: no Encounter is used.
    NotCounted {
        area: AreaRef,
        species: u16,
        species_name: String,
    },
    /// The first wild battle in an Area: its one Encounter is used.
    Encounter {
        area: AreaRef,
        species: u16,
        species_name: String,
    },
    /// A Pokémon joined the party or PC from a wild battle.
    Caught {
        area: Option<AreaRef>,
        pid: u32,
        species: u16,
        species_name: String,
    },
    /// A wild battle in a closed Area: ball throws are refused for it.
    CatchesBlocked {
        area: AreaRef,
        species: u16,
        species_name: String,
    },
    /// A block the rules called for couldn't be applied.
    BlockFailed { area: AreaRef, reason: String },
}

/// An Area as the log keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AreaRef {
    pub id: u16,
    pub name: String,
}

impl From<&Area> for AreaRef {
    fn from(area: &Area) -> Self {
        Self {
            id: area.id,
            name: area.name.clone(),
        }
    }
}

/// An event with where and when it happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    /// Its place in the Run log, from 1.
    pub seq: u64,
    /// Which time the Run was opened (1 for the first), since frames count
    /// from power-on in each.
    pub boot: u32,
    /// The frame boundary it was decided at.
    pub frame: u64,
    /// Unix time in milliseconds.
    pub at_ms: i64,
    pub event: RunEvent,
}
