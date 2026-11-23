//! The Run engine: the deterministic centre of a Run. It owns the local
//! Player's rules engine and the Run's state, watches the Player's game at
//! every frame boundary, and is the only thing that applies writes and ROM
//! patches to it, each stamped with that boundary in the local input stream.
//!
//! Dependencies are passed in: the game (through [`LiveGame`]), storage
//! (through [`RunStorage`]) and the clock (through [`RunClock`]).
#![warn(missing_docs)]

mod areas;
mod avatars;
mod boot;
mod cheats;
mod clock;
mod ending;
mod engine;
mod event;
mod families;
mod file;
mod links;
mod live;
mod presence;
mod rules;
mod running;
mod setup;
mod shadow;
mod shadow_runner;
mod shared_log;
mod storage;
mod stream;
mod view;

pub use areas::{AreaEncounter, area_encounters};
pub use avatars::{Painter, PartnerAvatar};
pub use boot::{BootPlan, Booted, boot_core};
pub use cheats::{Cheat, CheatWatch, FAST_BURST_FRAMES, Flag};
pub use clock::{RunClock, SystemRunClock};
pub use ending::{LastDeath, Survivor, survivors};
pub use engine::{PARTY_LOOK_FRAMES, RunEngine, SUSPEND_SLOT_EVERY_MS};
pub use event::{AreaRef, CheckpointReason, DeathCause, Recorded, ResumedFrom, RunEvent, SourceRef};
pub use file::{CopiedFrom, RunFile, RunId, RunLibrary, RunSummary};
pub use links::{Judge, Link, LinkKeeper, LinkStanding, Member, WaitingOnLink, links};
pub use live::{LiveGame, PlayedGame};
pub use presence::{DROPOUT_OFFER_MS, HOST_SILENT_MS, OFFLINE_MS, Presence};
pub use running::{RetryTo, RunCommand, RunningGame};
pub use setup::{
    COUNTDOWN_MS, Locked, Mismatch, Preset, RosterEntry, RunSetup, SetupCheck, SetupError, SetupMessage, SetupPlayer,
    SetupView, native_options, native_options_in_use, settle, unavailable,
};
pub use shadow::{
    BUFFER_FRAMES, CATCH_UP_SPEED, PREDICT_FRAMES, Seen, SeenAvatar, SeenPokemon, ShadowCheckpoint, ShadowGame,
    ShadowState,
};
pub use shadow_runner::{ShadowRunner, ShadowStatus};
pub use shared_log::{
    Call, DROPOUT_MIN_PLAYERS, FlagVote, GamePoint, LogBody, LogEntry, LogError, LogFollower, Part, ReadyCheck, RunLog,
    Signer, Submission, Tallies, Vote, Wiped,
};
pub use storage::{InputBatch, MemoryStorage, RunInfo, RunStorage, Stamped, StorageError, StoredCheckpoint, StoredRun};
pub use stream::{
    Added, BootedFrom, CHUNK_FRAMES, FrameHash, HASH_EVERY, InputChunk, Mark, Outgoing, PlayerKey, Received, Rewrite,
    StreamHub, StreamMessage, StreamOut, StreamStore, StreamWant,
};
pub use view::{
    Doing, FeedEntry, FeedIcon, FeedKind, GameOverView, Grave, LevelCapView, NativeOptionView, NicknameWanted, RunView,
    WipeView, graves,
};

use pactlocke_game::LevelCap;
use serde::{Deserialize, Serialize};

/// The challenge rules chosen for a Run, locked for the rest of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruleset {
    /// Only the first wild battle in each Area may be caught.
    pub first_encounter: bool,
    /// A Pokémon that faints is Dead and goes to the Dead box.
    #[serde(default)]
    pub faint_is_dead: bool,
    /// A wild Encounter from an evolution family already owned doesn't count.
    /// Needs `first_encounter`.
    #[serde(default)]
    pub dupes_clause: bool,
    /// Gifts other than the starter may be kept; when off they are released.
    /// Runs from before this rule kept every Gift.
    #[serde(default = "allowed")]
    pub gifts: bool,
    /// A Static is a bonus Encounter of its own; when off it counts only as
    /// the Area's first Encounter. Needs `first_encounter`.
    #[serde(default)]
    pub statics: bool,
    /// EXP and Rare Candy stop at the current stage's cap, one stage per
    /// gym leader, Elite Four member and Champion in the Game Profile's order; `None`
    /// for no caps. The Host may edit the levels before the lock.
    #[serde(default)]
    pub level_caps: Option<Vec<LevelCap>>,
    /// The Bag is refused in trainer battles and only Poké Balls work in
    /// wild battles; items outside battle and held items still work.
    #[serde(default)]
    pub no_items: bool,
    /// The battle style is forced to Set.
    #[serde(default)]
    pub set_mode: bool,
    /// "Give a nickname?" can't be declined, and a Pokémon left with its
    /// species name is named from the side panel.
    #[serde(default)]
    pub must_nickname: bool,
    /// Only one Pokémon of each primary type in the party at a time.
    #[serde(default)]
    pub type_clause: bool,
    /// Links: each Player's Encounter from the same source (an Area, a Gift
    /// or a Static), caught or not, is bound into a Link, and a Death kills
    /// every Link-mate. Needs `faint_is_dead` and more than one Player.
    #[serde(default)]
    pub links: bool,
    /// The shared pool of Retries, 0 to [`MAX_RETRIES`]: each sends every
    /// Player back to the latest Run Checkpoint, by everyone's vote (a solo
    /// Player needs no vote). 0 turns Retries off.
    #[serde(default)]
    pub retries: u8,
    /// Sync Gates: the battles (by the Game Profile's names, such as
    /// "Brock") nobody can start until every Player is Ready there. None in
    /// a solo Run.
    #[serde(default)]
    pub sync_gates: Vec<String>,
    /// A randomized Run: the FVX settings and how seeds are picked. Every
    /// Player's app makes every Player's game from their own clean ROM.
    #[serde(default)]
    pub randomizer: Option<RandomizerChoice>,
    /// Shared Seed through a Romhack's own randomizer (Heart and Soul,
    /// R.O.W.E.): the seed every Player's game is given. `None` leaves each
    /// game to seed itself.
    #[serde(default)]
    pub romhack_seed: Option<u32>,
}

/// How a randomized Run's games are made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomizerChoice {
    /// The UPR FVX settings string.
    pub settings: String,
    /// Shared Seed: every Player gets the same game.
    pub shared_seed: bool,
    /// Where the seeds come from: picked by the Host, sent only to the roster.
    pub secret: [u8; 16],
}

/// The most Retries a Ruleset's pool may hold.
pub const MAX_RETRIES: u8 = 10;

/// The most Pokémon a party holds. A full party keeps its slots for linked Pokémon.
pub const PARTY_SIZE: usize = 6;

fn allowed() -> bool {
    true
}

impl Default for Ruleset {
    /// No rules on, and so nothing held back: Gifts are kept.
    fn default() -> Self {
        Self {
            first_encounter: false,
            faint_is_dead: false,
            dupes_clause: false,
            gifts: true,
            statics: false,
            level_caps: None,
            no_items: false,
            set_mode: false,
            must_nickname: false,
            type_clause: false,
            links: false,
            retries: 0,
            sync_gates: Vec::new(),
            randomizer: None,
            romhack_seed: None,
        }
    }
}

impl Ruleset {
    /// Whether the Ruleset has `rule` on.
    pub fn has(&self, rule: pactlocke_game::Rule) -> bool {
        use pactlocke_game::Rule;
        match rule {
            Rule::FirstEncounter => self.first_encounter,
            Rule::DupesClause => self.dupes_clause,
            Rule::Gifts => self.gifts,
            Rule::Statics => self.statics,
            Rule::FaintIsDead => self.faint_is_dead,
            Rule::Links => self.links,
            Rule::LevelCap => self.level_caps.is_some(),
            Rule::NoItems => self.no_items,
            Rule::MustNickname => self.must_nickname,
            Rule::SetMode => self.set_mode,
            Rule::TypeClause => self.type_clause,
            Rule::SyncGates => !self.sync_gates.is_empty(),
        }
    }

    /// Whether the dupes clause can be chosen: it needs first Encounter.
    pub fn dupes_clause_available(&self) -> bool {
        self.first_encounter
    }

    /// Whether the Statics rule can be chosen: it needs first Encounter.
    pub fn statics_available(&self) -> bool {
        self.first_encounter
    }
}

/// What a new Run is started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    /// The Run's name.
    pub name: String,
    /// The name of the game it is played on.
    pub game_name: String,
    /// The SHA-1 of the ROM the Run is played on.
    pub game_sha1: String,
    /// Where the ROM was when the Run started, to offer it again on resume.
    pub rom_path: Option<String>,
    /// The challenge rules chosen for it.
    pub ruleset: Ruleset,
}
