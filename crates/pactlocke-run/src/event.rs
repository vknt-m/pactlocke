use pactlocke_game::Area;
use serde::{Deserialize, Serialize};

/// Something that happened in the Run, as the Run log records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    /// The Run began.
    Started { game_name: String },
    /// The Run was opened again; the game was booted afresh `from` there.
    Resumed {
        #[serde(default)]
        from: ResumedFrom,
    },
    /// A Run Checkpoint was stored with this event; `hash` is its savestate's SHA-1.
    Checkpoint { reason: CheckpointReason, hash: String },
    /// The Player suspended and quit; the suspend slot holds where.
    Suspended,
    /// The Run was closed cleanly. A boot that ends without this or
    /// `Suspended` ended in a crash.
    Closed,
    /// The game was reset to an older save, or a new game was started (its
    /// play time ran backwards).
    GameReset,
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
    /// The dupes clause: a wild battle with a Pokémon from a family the Player
    /// has alive doesn't use the Encounter, and ball throws are refused for it.
    DupeSkipped {
        area: AreaRef,
        species: u16,
        species_name: String,
        /// The species of that family the Player has alive.
        dupe_of: String,
    },
    /// A wild battle in a closed Area: ball throws are refused for it.
    CatchesBlocked {
        area: AreaRef,
        species: u16,
        species_name: String,
    },
    /// A block the rules called for couldn't be applied.
    BlockFailed { area: Option<AreaRef>, reason: String },
    /// A script gave the Player a Pokémon. `source` is the catalogued Gift it
    /// matched, or `None` for an unknown Gift the Game Profile doesn't list.
    Gift {
        pid: u32,
        species: u16,
        species_name: String,
        area: Option<AreaRef>,
        source: Option<SourceRef>,
        starter: bool,
        /// An egg; one no catalogue lists was bred at the Day Care.
        #[serde(default)]
        egg: bool,
    },
    /// An in-game trade: the Player gave `gave` and received this Pokémon, a Gift.
    Traded {
        gave: String,
        pid: u32,
        species: u16,
        species_name: String,
        area: Option<AreaRef>,
    },
    /// The Ruleset doesn't let the Player keep this Gift, so it is to be released.
    GiftDeclined {
        pid: u32,
        species_name: String,
        reason: DeclineReason,
    },
    /// A declined Gift was released.
    Released { pid: u32, species_name: String },
    /// A Violation: a declined Gift couldn't be released.
    ReleaseFailed {
        pid: u32,
        species_name: String,
        reason: String,
    },
    /// The Statics rule: a Static is a bonus Encounter of its own.
    StaticEncounter {
        area: Option<AreaRef>,
        source: SourceRef,
        species: u16,
        species_name: String,
    },
    /// A Static whose Encounter is used already: ball throws are refused for it.
    StaticBlocked {
        area: Option<AreaRef>,
        source: SourceRef,
        species: u16,
        species_name: String,
    },
    /// A Pokémon fainted, so it is Dead for the rest of the Run.
    Died {
        pid: u32,
        species: u16,
        species_name: String,
        /// Its nickname, or its species name if it has none.
        name: String,
        area: Option<AreaRef>,
        cause: DeathCause,
    },
    /// A Dead Pokémon was moved from the party to the Dead box.
    MovedToDeadBox { pid: u32, name: String, box_number: u8 },
    /// A Violation: a Dead Pokémon was back in the party, and was moved to the Dead box again.
    DeadReturned { pid: u32, name: String, box_number: u8 },
    /// A Violation: the Dead box was full, so a Dead Pokémon went to another box.
    DeadBoxFull { pid: u32, name: String, box_number: u8 },
    /// A Violation: a Dead Pokémon couldn't be moved out of the party.
    DeadBoxFailed { pid: u32, name: String, reason: String },
    /// The Player is at level cap stage `stage` (from 0), so the cap is
    /// `level` until they pass `next`; with `next` `None` every stage is
    /// passed and there is no cap. `raised` when it follows an earlier stage.
    LevelCapSet {
        stage: usize,
        level: u8,
        next: Option<String>,
        raised: bool,
    },
    /// The game's level checks can't be patched, so the cap is only a soft
    /// cap: a Player over it can't ready up at a Sync Gate.
    SoftCap { reason: String },
    /// No items: the Bag is refused in trainer battles and only Poké Balls
    /// work in wild battles. Said once per Run.
    ItemsBanned,
    /// No items couldn't be put in place on this game. Said once per Run.
    ItemBanFailed { reason: String },
    /// Set mode: the battle style was written to Set; `again` when the
    /// Player had changed it back.
    SetModeForced { again: bool },
    /// Set mode couldn't be forced on this game. Said once per Run.
    SetModeFailed { reason: String },
    /// Must nickname: a Pokémon was left with its species name, so the side
    /// panel asks the Player for one.
    NicknameWanted { pid: u32, species_name: String },
    /// The Player named it from the side panel.
    Nicknamed {
        pid: u32,
        species_name: String,
        name: String,
    },
    /// A name the Player gave couldn't be used.
    NicknameRefused {
        pid: u32,
        species_name: String,
        name: String,
        reason: String,
    },
    /// The type clause: a second Pokémon of a primary type in the party was
    /// moved to the PC. `holder` has that type in the party.
    TypeClauseMoved {
        pid: u32,
        name: String,
        holder: String,
        type_name: String,
        box_number: u8,
    },
    /// A Violation: the type clause couldn't move a Pokémon out of the party.
    TypeClauseFailed {
        pid: u32,
        name: String,
        type_name: String,
        reason: String,
    },
}

/// Where a resumed Run's game was booted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "boot")]
pub enum ResumedFrom {
    /// The last in-game save, as a console would after being switched off.
    #[default]
    Save,
    /// No in-game save yet: a new game from the start.
    Start,
    /// The suspend slot: exactly where the Player suspended.
    Suspend,
    /// After a crash: the last checkpoint and the button presses recorded after it.
    Rebuild { frames: u64 },
}

/// Why a Run Checkpoint was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReason {
    /// The game was booted: a boot checkpoint, only for rebuilding after a
    /// crash, never a Retry point.
    Boot,
    /// The Player saved in-game.
    InGameSave,
    /// Asked for at a moment that needs one, such as a Sync Gate.
    Asked,
}

/// A Gift or Static source, as the log keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// The same in every Player's game for the same Gift or Static.
    pub key: String,
    pub name: String,
}

impl From<pactlocke_game::Source> for SourceRef {
    fn from(source: pactlocke_game::Source) -> Self {
        Self {
            key: source.key,
            name: source.name,
        }
    }
}

/// Why a Gift isn't kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclineReason {
    /// The Ruleset allows no Gifts but the starter.
    GiftsOff,
    /// The Gift's source has given its one Encounter already.
    SourceUsed,
}

/// What made a Pokémon faint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum DeathCause {
    /// A wild Pokémon; `foe` is its species name.
    Wild { foe: String },
    /// A trainer's Pokémon; `foe` is its species name.
    Trainer { foe: String },
    /// Fainting outside battle, which only poison does.
    Poison,
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
