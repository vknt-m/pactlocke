//! The Run engine: the deterministic centre of a Run. It owns the local
//! Player's rules engine and the Run's state, watches the Player's game at
//! every frame boundary, and is the only thing that applies writes and ROM
//! patches to it, each stamped with that boundary in the local input stream.
//!
//! Dependencies are passed in: the game (through [`LiveGame`]), storage
//! (through [`RunStorage`]) and the clock (through [`RunClock`]).

mod clock;
mod engine;
mod event;
mod file;
mod live;
mod rules;
mod running;
mod storage;
mod view;

pub use clock::{RunClock, SystemRunClock};
pub use engine::RunEngine;
pub use event::{AreaRef, Recorded, RunEvent};
pub use file::{RunFile, RunId, RunLibrary, RunSummary};
pub use live::LiveGame;
pub use running::RunningGame;
pub use storage::{InputBatch, MemoryStorage, RunInfo, RunStorage, Stamped, StorageError, StoredRun};
pub use view::{FeedEntry, FeedKind, RunView};

use serde::{Deserialize, Serialize};

/// The challenge rules chosen for a Run, locked for the rest of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Ruleset {
    /// Only the first wild battle in each Area may be caught.
    pub first_encounter: bool,
}

/// What a new Run is started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    pub name: String,
    pub game_name: String,
    /// The SHA-1 of the ROM the Run is played on.
    pub game_sha1: String,
    /// Where the ROM was when the Run started, to offer it again on resume.
    pub rom_path: Option<String>,
    pub ruleset: Ruleset,
}
