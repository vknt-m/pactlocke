//! Where a Run is kept: its details, the append-only Run log, the local input
//! stream and the game's save memory.

use std::fmt;
use std::sync::{Arc, Mutex};

use pactlocke_emu::{Checkpoint, Keys};
use pactlocke_game::Change;

use crate::{CheckpointReason, LogEntry, Recorded, Ruleset};

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

/// A checkpoint as the Run file keeps it: a Run Checkpoint (an in-game save,
/// a Sync Gate), a boot checkpoint, or the suspend slot. It is the core's
/// savestate and save memory, the ROM patches active then (a savestate
/// doesn't hold them), and where it falls in its boot's input stream, so a
/// crash can be rebuilt from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCheckpoint {
    pub reason: CheckpointReason,
    pub boot: u32,
    /// The frame boundary it was taken at: frames run in its boot before it.
    pub frame: u64,
    /// How many changes its boot had stamped before it was taken.
    pub stamped_before: usize,
    /// The clock its boot started with, so a rebuild reads the same time.
    pub clock_ms: i64,
    /// Every ROM patch active then, by address.
    pub patches: Vec<(u32, Vec<u8>)>,
    pub core: Checkpoint,
}

/// Everything kept about a Run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRun {
    pub info: RunInfo,
    pub events: Vec<Recorded>,
    pub inputs: Vec<InputBatch>,
    /// The game's save memory as last seen, to boot the game with on resume.
    pub save_memory: Option<Vec<u8>>,
    /// The newest checkpoint of any reason, to rebuild a crashed boot from.
    pub latest_checkpoint: Option<StoredCheckpoint>,
    /// The suspend slot: where the Player suspended and quit, until resumed.
    pub suspended: Option<StoredCheckpoint>,
    /// In a Run with friends, this Player's copy of the whole Run log; empty alone.
    pub log: Vec<LogEntry>,
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
    /// Records an event with the checkpoint it refers to, together.
    fn append_with_checkpoint(&mut self, event: &Recorded, checkpoint: &StoredCheckpoint);
    /// Records the Suspended event with the suspend slot's checkpoint, together.
    fn store_suspend(&mut self, event: &Recorded, checkpoint: &StoredCheckpoint);
    /// Empties the suspend slot: it is used up on resume.
    fn clear_suspend(&mut self);
    /// Fills the suspend slot with no event: a Run with friends keeps it up
    /// to date as it goes, so leaving in any way resumes where the Player stopped.
    fn keep_suspend(&mut self, checkpoint: &StoredCheckpoint);
    /// Keeps this Player's copy of the Run log: these entries replace any
    /// kept from the one numbered `from` on.
    fn keep_log(&mut self, from: u64, entries: &[LogEntry]);
    fn load(&mut self) -> Result<StoredRun, StorageError>;
    /// Why a write failed, if one has since the Run was opened.
    fn write_failure(&mut self) -> Option<String>;
    /// The checkpoint stored with the event numbered `event_seq`, if it
    /// hasn't been pruned. It waits for writes, like `load`.
    fn checkpoint(&mut self, event_seq: u64) -> Option<StoredCheckpoint>;
}

impl StoredCheckpoint {
    /// Its SHA-1 over everything a restore puts back: the save memory, the
    /// savestate and the patch set, as hex.
    pub fn hash(&self) -> String {
        use sha1::{Digest, Sha1};
        let mut sha = Sha1::new();
        sha.update(&self.core.save_memory);
        sha.update(&self.core.state);
        for (addr, bytes) in &self.patches {
            sha.update(addr.to_le_bytes());
            sha.update(bytes);
        }
        sha.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Storage in memory, for tests. Clones share the same Run, so a test can
/// keep one and resume the Run from it later.
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    run: Arc<Mutex<Option<StoredRun>>>,
    failure: Arc<Mutex<Option<String>>>,
    /// Every checkpoint stored, by its event: memory doesn't prune.
    checkpoints: Arc<Mutex<Vec<(u64, StoredCheckpoint)>>>,
}

impl MemoryStorage {
    /// A copy of the Run as stored now that later writes don't reach, as
    /// the Run file would be found after a crash at this moment.
    pub fn snapshot(&self) -> MemoryStorage {
        MemoryStorage {
            run: Arc::new(Mutex::new(self.run.lock().unwrap().clone())),
            failure: Arc::new(Mutex::new(None)),
            checkpoints: Arc::new(Mutex::new(self.checkpoints.lock().unwrap().clone())),
        }
    }

    /// The Run as stored.
    pub fn load_run(&self) -> StoredRun {
        self.run.lock().unwrap().clone().expect("the Run was created first")
    }

    /// Changes every stored checkpoint, as someone editing the Run file would.
    pub fn tamper_with_checkpoints(&self, mut change: impl FnMut(&mut StoredCheckpoint)) {
        for (_, checkpoint) in self.checkpoints.lock().unwrap().iter_mut() {
            change(checkpoint);
        }
    }

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
            latest_checkpoint: None,
            suspended: None,
            log: Vec::new(),
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

    fn append_with_checkpoint(&mut self, event: &Recorded, checkpoint: &StoredCheckpoint) {
        if self.failing() {
            return;
        }
        self.with(|run| {
            run.events.push(event.clone());
            run.latest_checkpoint = Some(checkpoint.clone());
        });
        self.checkpoints.lock().unwrap().push((event.seq, checkpoint.clone()));
    }

    fn store_suspend(&mut self, event: &Recorded, checkpoint: &StoredCheckpoint) {
        self.with(|run| {
            run.events.push(event.clone());
            run.suspended = Some(checkpoint.clone());
        });
    }

    fn clear_suspend(&mut self) {
        self.with(|run| run.suspended = None);
    }

    fn keep_suspend(&mut self, checkpoint: &StoredCheckpoint) {
        self.with(|run| run.suspended = Some(checkpoint.clone()));
    }

    fn keep_log(&mut self, from: u64, entries: &[LogEntry]) {
        self.with(|run| {
            run.log.retain(|e| e.seq < from);
            run.log.extend_from_slice(entries);
        });
    }

    fn load(&mut self) -> Result<StoredRun, StorageError> {
        self.run.lock().unwrap().clone().ok_or(StorageError::NoRun)
    }

    fn write_failure(&mut self) -> Option<String> {
        self.failure.lock().unwrap().clone()
    }

    fn checkpoint(&mut self, event_seq: u64) -> Option<StoredCheckpoint> {
        let checkpoints = self.checkpoints.lock().unwrap();
        checkpoints
            .iter()
            .find(|(seq, _)| *seq == event_seq)
            .map(|(_, c)| c.clone())
    }
}
