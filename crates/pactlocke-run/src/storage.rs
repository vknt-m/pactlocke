//! Where a Run is kept: its details, the append-only Run log, the local input
//! stream and the game's save memory.

use std::fmt;
use std::sync::{Arc, Mutex};

use pactlocke_emu::Keys;
use pactlocke_game::Change;

use crate::{Recorded, Ruleset};

/// What a Run is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunInfo {
    pub name: String,
    pub game_name: String,
    pub game_sha1: String,
    pub rom_path: Option<String>,
    pub ruleset: Ruleset,
    /// Unix time in milliseconds.
    pub created_ms: i64,
}

/// A write or patch, stamped with the frame boundary it landed at: the
/// number of frames run before it, so a replay applies it before running
/// frame `frame`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped {
    pub frame: u64,
    pub change: Change,
}

/// A stretch of the local input stream: the keys held for each frame from
/// `first_frame` on, and the changes stamped in that stretch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBatch {
    pub boot: u32,
    pub first_frame: u64,
    pub keys: Vec<Keys>,
    pub stamped: Vec<Stamped>,
}

/// Everything kept about a Run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRun {
    pub info: RunInfo,
    pub events: Vec<Recorded>,
    pub inputs: Vec<InputBatch>,
    /// The game's save memory as last seen, to boot the game with on resume.
    pub save_memory: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// There is no Run here.
    NoRun,
    /// The Run file can't be read or written.
    File(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRun => write!(f, "there is no Run here"),
            Self::File(why) => write!(f, "the Run file: {why}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Storage for one Run. Writes never block the caller; `load` waits for them.
pub trait RunStorage: Send {
    /// Records a new Run's details.
    fn create(&mut self, info: &RunInfo) -> Result<(), StorageError>;
    fn append(&mut self, event: &Recorded);
    fn record_inputs(&mut self, batch: &InputBatch);
    fn store_save_memory(&mut self, save: &[u8]);
    fn load(&mut self) -> Result<StoredRun, StorageError>;
    /// Why a write failed, if one has since the Run was opened.
    fn write_failure(&mut self) -> Option<String>;
}

/// Storage in memory, for tests. Clones share the same Run, so a test can
/// keep one and resume the Run from it later.
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    run: Arc<Mutex<Option<StoredRun>>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl MemoryStorage {
    /// From now on, writes fail for this reason (the Run as stored so far is kept).
    pub fn fail_writes(&self, why: &str) {
        *self.failure.lock().unwrap() = Some(why.to_owned());
    }

    fn failing(&self) -> bool {
        self.failure.lock().unwrap().is_some()
    }

    fn with(&self, f: impl FnOnce(&mut StoredRun)) {
        if self.failing() {
            return;
        }
        let mut run = self.run.lock().unwrap();
        f(run.as_mut().expect("the Run was created first"));
    }
}

impl RunStorage for MemoryStorage {
    fn create(&mut self, info: &RunInfo) -> Result<(), StorageError> {
        *self.run.lock().unwrap() = Some(StoredRun {
            info: info.clone(),
            events: Vec::new(),
            inputs: Vec::new(),
            save_memory: None,
        });
        Ok(())
    }

    fn append(&mut self, event: &Recorded) {
        self.with(|run| run.events.push(event.clone()));
    }

    fn record_inputs(&mut self, batch: &InputBatch) {
        self.with(|run| run.inputs.push(batch.clone()));
    }

    fn store_save_memory(&mut self, save: &[u8]) {
        self.with(|run| run.save_memory = Some(save.to_vec()));
    }

    fn load(&mut self) -> Result<StoredRun, StorageError> {
        self.run.lock().unwrap().clone().ok_or(StorageError::NoRun)
    }

    fn write_failure(&mut self) -> Option<String> {
        self.failure.lock().unwrap().clone()
    }
}
