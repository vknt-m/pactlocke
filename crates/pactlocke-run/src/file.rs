//! The Run file: one SQLite database per Run (WAL, synchronous NORMAL),
//! written on a dedicated thread fed by a channel, and the folder of Run
//! files the home screen lists. Run Checkpoints are zstd blobs, written in
//! the same transaction as the log event that refers to them.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use pactlocke_emu::{Checkpoint, Keys};
use pactlocke_game::Change;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::storage::{InputBatch, RunInfo, RunStorage, Stamped, StorageError, StoredCheckpoint, StoredRun};
use crate::{CheckpointReason, LogEntry, PlayerKey, Recorded, Ruleset, ShadowCheckpoint, Submission};

/// Run files end in this.
const EXTENSION: &str = "pactrun";

/// How many Run Checkpoints the file keeps, besides the newest boot
/// checkpoint; older ones are pruned (their hashes stay in the log).
const RUN_CHECKPOINTS_KEPT: i64 = 3;

/// zstd's level for checkpoints: quick, and a savestate shrinks well.
const ZSTD_LEVEL: i32 = 3;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL,
    game_name TEXT NOT NULL,
    game_sha1 TEXT NOT NULL,
    rom_path TEXT,
    ruleset TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY,
    boot INTEGER NOT NULL,
    frame INTEGER NOT NULL,
    at_ms INTEGER NOT NULL,
    event TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS inputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    boot INTEGER NOT NULL,
    first_frame INTEGER NOT NULL,
    keys BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS stamped (
    input_id INTEGER NOT NULL REFERENCES inputs(id),
    frame INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    addr INTEGER NOT NULL,
    bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS save_memory (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS checkpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    reason TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    boot INTEGER NOT NULL,
    frame INTEGER NOT NULL,
    stamped_before INTEGER NOT NULL,
    clock_ms INTEGER NOT NULL,
    patches BLOB NOT NULL,
    core BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS run_log (
    seq INTEGER PRIMARY KEY,
    entry BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS waiting (
    id INTEGER PRIMARY KEY,
    submission BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS shadow_checkpoints (
    player BLOB PRIMARY KEY,
    boot INTEGER NOT NULL,
    frame INTEGER NOT NULL,
    patches BLOB NOT NULL,
    seen TEXT NOT NULL,
    core BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS copy_of (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL,
    at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS suspend (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    reason TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    boot INTEGER NOT NULL,
    frame INTEGER NOT NULL,
    stamped_before INTEGER NOT NULL,
    clock_ms INTEGER NOT NULL,
    patches BLOB NOT NULL,
    core BLOB NOT NULL
);
";

/// Which Run file, by its name in the folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunId(pub String);

/// A Run as the home screen lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub id: RunId,
    pub name: String,
    pub game_name: String,
    pub game_sha1: String,
    pub rom_path: Option<String>,
    pub ruleset: Ruleset,
    pub created_ms: i64,
    /// When anything last happened in it.
    pub last_played_ms: i64,
    /// Deaths recorded in it, a Retry's undone ones included.
    pub deaths: u32,
    /// Whether it is a Run with friends: its file keeps a Run log.
    pub with_friends: bool,
    /// For a solo copy, the Run with friends it was copied from, and when.
    pub copied_from: Option<CopiedFrom>,
}

/// The Run with friends a solo copy was made from: its name, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedFrom {
    pub name: String,
    /// Unix time in milliseconds.
    pub at_ms: i64,
}

/// The folder of Run files.
pub struct RunLibrary {
    dir: PathBuf,
}

impl RunLibrary {
    /// Uses `dir`, creating it if needed.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| StorageError::File(format!("{}: {e}", dir.display())))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &RunId) -> PathBuf {
        self.dir.join(format!("{}.{EXTENSION}", id.0))
    }

    /// A new, empty Run file; starting a Run in it fills it in.
    pub fn create(&self) -> Result<RunFile, StorageError> {
        RunFile::open(self.path(&self.new_id()))
    }

    /// A name for a new Run file that no file in the folder has.
    fn new_id(&self) -> RunId {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let mut id = RunId(format!("run-{stamp}"));
        let mut n = 1;
        while self.path(&id).exists() {
            n += 1;
            id = RunId(format!("run-{stamp}-{n}"));
        }
        id
    }

    /// A solo copy of the Run with friends `id`, made at `now_ms`: a new
    /// solo Run going on from this Player's game exactly where they left the
    /// Run with friends, which stays as it is. It keeps the Ruleset but for
    /// Links and Sync Gates, which are off, and starts with what is left of
    /// the shared Retry pool. Its name is the Run's with " (solo)".
    pub fn solo_copy(&self, id: &RunId, now_ms: i64) -> Result<RunId, StorageError> {
        // Opening the Run file brings an older one's tables up to date.
        let stored = self.open_run(id)?.load()?;
        let log = crate::RunLog::from_kept(stored.log.clone())
            .map_err(|e| StorageError::File(format!("its Run log can't be read: {e}")))?;
        // A solo Run counts its Retries from its own file, where every
        // Retry of the Run with friends is recorded too.
        let retried = stored
            .events
            .iter()
            .filter(|e| matches!(e.event, crate::RunEvent::Retried { .. }))
            .count();
        let ruleset = Ruleset {
            links: false,
            sync_gates: Vec::new(),
            retries: log
                .retries_left()
                .saturating_add(u8::try_from(retried).unwrap_or(u8::MAX)),
            ..stored.info.ruleset.clone()
        };
        let copy = self.new_id();
        let to = self.path(&copy);
        let written = || -> rusqlite::Result<()> {
            let from = Connection::open_with_flags(self.path(id), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            from.execute("VACUUM INTO ?1", [to.to_string_lossy()])?;
            drop(from);
            let db = Connection::open(&to)?;
            let tx = db.unchecked_transaction()?;
            tx.execute(
                "UPDATE run SET name = ?1, ruleset = ?2 WHERE id = 1",
                params![
                    format!("{} (solo)", stored.info.name),
                    serde_json::to_string(&ruleset).expect("a Ruleset serialises")
                ],
            )?;
            for table in ["run_log", "waiting", "shadow_checkpoints"] {
                tx.execute(&format!("DELETE FROM {table}"), [])?;
            }
            tx.execute(
                "INSERT OR REPLACE INTO copy_of (id, name, at_ms) VALUES (1, ?1, ?2)",
                params![stored.info.name, now_ms],
            )?;
            tx.commit()
        };
        if let Err(e) = written() {
            let _ = self.delete(&copy);
            return Err(StorageError::File(format!("the solo copy couldn't be written: {e}")));
        }
        Ok(copy)
    }

    /// An existing Run's file.
    pub fn open_run(&self, id: &RunId) -> Result<RunFile, StorageError> {
        let path = self.path(id);
        if !path.exists() {
            return Err(StorageError::NoRun);
        }
        RunFile::open(path)
    }

    /// Deletes a Run's file for good, with SQLite's journal files beside it.
    pub fn delete(&self, id: &RunId) -> std::io::Result<()> {
        let path = self.path(id);
        for extra in ["-wal", "-shm"] {
            let mut side = path.clone().into_os_string();
            side.push(extra);
            let _ = std::fs::remove_file(side);
        }
        std::fs::remove_file(path)
    }

    /// Every started Run in the folder, most recently played first. Files
    /// that aren't Runs are left out.
    pub fn list(&self) -> Vec<RunSummary> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut runs: Vec<RunSummary> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
            .filter_map(|path| summary(&path))
            .collect();
        runs.sort_by(|a, b| {
            b.last_played_ms
                .cmp(&a.last_played_ms)
                .then_with(|| a.name.cmp(&b.name))
        });
        runs
    }
}

fn summary(path: &Path) -> Option<RunSummary> {
    let id = RunId(path.file_stem()?.to_string_lossy().into_owned());
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let info = read_info(&db).ok()??;
    let last_event: Option<i64> = db
        .query_row("SELECT MAX(at_ms) FROM events", [], |row| row.get(0))
        .ok()?;
    let deaths: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM events WHERE event LIKE '%\"kind\":\"died\"%'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    // A file from before Run logs were kept has no table for one.
    let with_friends = db
        .query_row("SELECT EXISTS (SELECT 1 FROM run_log)", [], |row| row.get(0))
        .unwrap_or(false);
    let copied_from = db
        .query_row("SELECT name, at_ms FROM copy_of WHERE id = 1", [], |row| {
            Ok(CopiedFrom {
                name: row.get(0)?,
                at_ms: row.get(1)?,
            })
        })
        .ok();
    Some(RunSummary {
        id,
        deaths,
        with_friends,
        copied_from,
        last_played_ms: last_event.unwrap_or(info.created_ms),
        name: info.name,
        game_name: info.game_name,
        game_sha1: info.game_sha1,
        rom_path: info.rom_path,
        ruleset: info.ruleset,
        created_ms: info.created_ms,
    })
}

fn read_info(db: &Connection) -> rusqlite::Result<Option<RunInfo>> {
    db.query_row(
        "SELECT name, game_name, game_sha1, rom_path, ruleset, created_ms FROM run WHERE id = 1",
        [],
        |row| {
            let ruleset: String = row.get(4)?;
            Ok(RunInfo {
                name: row.get(0)?,
                game_name: row.get(1)?,
                game_sha1: row.get(2)?,
                rom_path: row.get(3)?,
                // A Ruleset that can't be read is a broken file, never a looser Ruleset.
                ruleset: serde_json::from_str(&ruleset).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
                })?,
                created_ms: row.get(5)?,
            })
        },
    )
    .optional()
}

enum Message {
    Create(RunInfo, Sender<Result<(), StorageError>>),
    Append(Recorded),
    Inputs(InputBatch),
    SaveMemory(Vec<u8>),
    Checkpoint(Recorded, StoredCheckpoint),
    Suspend(Recorded, StoredCheckpoint),
    ClearSuspend,
    KeepSuspend(StoredCheckpoint),
    KeepLog(u64, Vec<LogEntry>),
    KeepWaiting(Vec<Submission>),
    KeepShadow(PlayerKey, Box<ShadowCheckpoint>),
    Load(Sender<Result<StoredRun, StorageError>>),
    ReadCheckpoint(u64, Sender<Result<Option<StoredCheckpoint>, StorageError>>),
}

/// One Run's file. Every write goes to its own database thread, so playing
/// never waits on the disk; dropping it finishes every write first.
pub struct RunFile {
    sender: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
    /// The first write the database thread couldn't make.
    failure: Arc<Mutex<Option<String>>>,
}

impl RunFile {
    fn open(path: PathBuf) -> Result<Self, StorageError> {
        let (opened_tx, opened_rx) = mpsc::channel();
        let (sender, receiver) = mpsc::channel();
        let failure = Arc::new(Mutex::new(None));
        let failed = failure.clone();
        let thread = std::thread::Builder::new()
            .name("run file".into())
            .spawn(move || match open_database(&path) {
                Ok(db) => {
                    let _ = opened_tx.send(Ok(()));
                    serve(&db, &receiver, &failed);
                }
                Err(e) => {
                    let _ = opened_tx.send(Err(StorageError::File(format!("{}: {e}", path.display()))));
                }
            })
            .map_err(|e| StorageError::File(e.to_string()))?;
        match opened_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                sender: Some(sender),
                thread: Some(thread),
                failure,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(StorageError::File("the Run file thread stopped".into())),
        }
    }

    fn send(&self, message: Message) {
        if let Some(sender) = &self.sender {
            // The thread only stops once the sender is dropped.
            let _ = sender.send(message);
        }
    }

    fn ask<T>(&self, message: impl FnOnce(Sender<Result<T, StorageError>>) -> Message) -> Result<T, StorageError> {
        let (reply, answer) = mpsc::channel();
        self.send(message(reply));
        answer
            .recv()
            .unwrap_or_else(|_| Err(StorageError::File("the Run file thread stopped".into())))
    }
}

impl Drop for RunFile {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl RunStorage for RunFile {
    fn create(&mut self, info: &RunInfo) -> Result<(), StorageError> {
        self.ask(|reply| Message::Create(info.clone(), reply))
    }

    fn append(&mut self, event: &Recorded) {
        self.send(Message::Append(event.clone()));
    }

    fn record_inputs(&mut self, batch: &InputBatch) {
        self.send(Message::Inputs(batch.clone()));
    }

    fn store_save_memory(&mut self, save: &[u8]) {
        self.send(Message::SaveMemory(save.to_vec()));
    }

    fn append_with_checkpoint(&mut self, event: &Recorded, checkpoint: &StoredCheckpoint) {
        self.send(Message::Checkpoint(event.clone(), checkpoint.clone()));
    }

    fn store_suspend(&mut self, event: &Recorded, checkpoint: &StoredCheckpoint) {
        self.send(Message::Suspend(event.clone(), checkpoint.clone()));
    }

    fn clear_suspend(&mut self) {
        self.send(Message::ClearSuspend);
    }

    fn keep_suspend(&mut self, checkpoint: &StoredCheckpoint) {
        self.send(Message::KeepSuspend(checkpoint.clone()));
    }

    fn keep_log(&mut self, from: u64, entries: &[LogEntry]) {
        self.send(Message::KeepLog(from, entries.to_vec()));
    }

    fn keep_waiting(&mut self, waiting: &[Submission]) {
        self.send(Message::KeepWaiting(waiting.to_vec()));
    }

    fn keep_shadow(&mut self, player: PlayerKey, checkpoint: &ShadowCheckpoint) {
        self.send(Message::KeepShadow(player, Box::new(checkpoint.clone())));
    }

    fn load(&mut self) -> Result<StoredRun, StorageError> {
        self.ask(Message::Load)
    }

    fn write_failure(&mut self) -> Option<String> {
        self.failure.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn checkpoint(&mut self, event_seq: u64) -> Option<StoredCheckpoint> {
        self.ask(|reply| Message::ReadCheckpoint(event_seq, reply))
            .ok()
            .flatten()
    }
}

fn open_database(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "NORMAL")?;
    db.execute_batch(SCHEMA)?;
    Ok(db)
}

/// The database thread: one message at a time until the file is dropped.
fn serve(db: &Connection, messages: &Receiver<Message>, failure: &Mutex<Option<String>>) {
    for message in messages {
        let result = match message {
            Message::Create(info, reply) => {
                let _ = reply.send(write_info(db, &info).map_err(file_error));
                Ok(())
            }
            Message::Append(event) => append(db, &event),
            Message::Inputs(batch) => record_inputs(db, &batch),
            Message::SaveMemory(save) => db
                .execute(
                    "INSERT OR REPLACE INTO save_memory (id, bytes) VALUES (1, ?1)",
                    params![save],
                )
                .map(drop),
            Message::Checkpoint(event, checkpoint) => {
                store_checkpoint(db, Some(&event), &checkpoint, Slot::Checkpoints)
            }
            Message::Suspend(event, checkpoint) => store_checkpoint(db, Some(&event), &checkpoint, Slot::Suspend),
            Message::KeepSuspend(checkpoint) => store_checkpoint(db, None, &checkpoint, Slot::Suspend),
            Message::ClearSuspend => db.execute("DELETE FROM suspend", []).map(drop),
            Message::KeepLog(from, entries) => keep_log(db, from, &entries),
            Message::KeepWaiting(waiting) => keep_waiting(db, &waiting),
            Message::KeepShadow(player, checkpoint) => keep_shadow(db, &player, &checkpoint),
            Message::Load(reply) => {
                let _ = reply.send(load(db));
                Ok(())
            }
            Message::ReadCheckpoint(seq, reply) => {
                let _ = reply.send(read_checkpoint_where(db, Slot::Checkpoints, Some(seq)));
                Ok(())
            }
        };
        if let Err(e) = result {
            failure
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert(e.to_string());
        }
    }
}

fn file_error(e: rusqlite::Error) -> StorageError {
    StorageError::File(e.to_string())
}

fn write_info(db: &Connection, info: &RunInfo) -> rusqlite::Result<()> {
    let ruleset = serde_json::to_string(&info.ruleset).expect("a Ruleset serialises");
    db.execute(
        "INSERT INTO run (id, name, game_name, game_sha1, rom_path, ruleset, created_ms)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            info.name,
            info.game_name,
            info.game_sha1,
            info.rom_path,
            ruleset,
            info.created_ms
        ],
    )
    .map(drop)
}

fn append(db: &Connection, recorded: &Recorded) -> rusqlite::Result<()> {
    insert_event(db, recorded)
}

fn insert_event(db: &Connection, recorded: &Recorded) -> rusqlite::Result<()> {
    let event = serde_json::to_string(&recorded.event).expect("an event serialises");
    db.execute(
        "INSERT INTO events (seq, boot, frame, at_ms, event) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            recorded.seq as i64,
            recorded.boot,
            recorded.frame as i64,
            recorded.at_ms,
            event
        ],
    )
    .map(drop)
}

/// Where a checkpoint is kept in the file.
#[derive(Clone, Copy)]
enum Slot {
    /// Run Checkpoints and boot checkpoints: the newest few are kept.
    Checkpoints,
    /// The one suspend slot.
    Suspend,
}

impl Slot {
    fn table(self) -> &'static str {
        match self {
            Slot::Checkpoints => "checkpoints",
            Slot::Suspend => "suspend",
        }
    }
}

/// The event, if there is one, and its checkpoint, in one transaction. The
/// suspend slot has one row; the checkpoints table keeps the newest boot
/// checkpoint and the newest few Run Checkpoints.
fn store_checkpoint(
    db: &Connection,
    event: Option<&Recorded>,
    checkpoint: &StoredCheckpoint,
    slot: Slot,
) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    if let Some(event) = event {
        insert_event(&tx, event)?;
    }
    let core = zstd::encode_all(&encode_core(&checkpoint.core)[..], ZSTD_LEVEL)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let reason = serde_json::to_string(&checkpoint.reason).expect("a reason serialises");
    let id = match slot {
        Slot::Suspend => Some(1),
        Slot::Checkpoints => None,
    };
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO {} (id, reason, event_seq, boot, frame, stamped_before, clock_ms, patches, core)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            slot.table()
        ),
        params![
            id,
            reason,
            event.map_or(0, |e| e.seq as i64),
            checkpoint.boot,
            checkpoint.frame as i64,
            checkpoint.stamped_before as i64,
            checkpoint.clock_ms,
            encode_patches(&checkpoint.patches),
            core
        ],
    )?;
    if let Slot::Checkpoints = slot {
        let boot = serde_json::to_string(&CheckpointReason::Boot).expect("a reason serialises");
        tx.execute(
            "DELETE FROM checkpoints WHERE id NOT IN (
                 SELECT id FROM checkpoints WHERE reason != ?1 ORDER BY id DESC LIMIT ?2
             ) AND id NOT IN (SELECT MAX(id) FROM checkpoints WHERE reason = ?1)",
            params![boot, RUN_CHECKPOINTS_KEPT],
        )?;
    }
    tx.commit()
}

/// A core checkpoint as bytes: the save memory's length, the save memory, the savestate.
fn encode_core(core: &Checkpoint) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + core.save_memory.len() + core.state.len());
    out.extend_from_slice(&(core.save_memory.len() as u32).to_le_bytes());
    out.extend_from_slice(&core.save_memory);
    out.extend_from_slice(&core.state);
    out
}

fn decode_core(bytes: &[u8]) -> Option<Checkpoint> {
    let (len, rest) = bytes.split_first_chunk::<4>()?;
    let len = u32::from_le_bytes(*len) as usize;
    let (save_memory, state) = (rest.get(..len)?, rest.get(len..)?);
    Some(Checkpoint {
        save_memory: save_memory.to_vec(),
        state: state.to_vec(),
    })
}

/// Each patch as its address, length and bytes.
fn encode_patches(patches: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (addr, bytes) in patches {
        out.extend_from_slice(&addr.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

fn decode_patches(mut bytes: &[u8]) -> Option<Vec<(u32, Vec<u8>)>> {
    let mut patches = Vec::new();
    while !bytes.is_empty() {
        let (addr, rest) = bytes.split_first_chunk::<4>()?;
        let (len, rest) = rest.split_first_chunk::<4>()?;
        let len = u32::from_le_bytes(*len) as usize;
        patches.push((u32::from_le_bytes(*addr), rest.get(..len)?.to_vec()));
        bytes = &rest[len..];
    }
    Some(patches)
}

/// The newest checkpoint in a slot, decompressed.
fn read_checkpoint(db: &Connection, slot: Slot) -> Result<Option<StoredCheckpoint>, StorageError> {
    read_checkpoint_where(db, slot, None)
}

/// The newest checkpoint in a slot, or the one stored with event `seq`, decompressed.
fn read_checkpoint_where(
    db: &Connection,
    slot: Slot,
    seq: Option<u64>,
) -> Result<Option<StoredCheckpoint>, StorageError> {
    let table = slot.table();
    let row = db
        .query_row(
            &format!(
                "SELECT reason, boot, frame, stamped_before, clock_ms, patches, core FROM {table}
                 WHERE ?1 IS NULL OR event_seq = ?1 ORDER BY id DESC LIMIT 1"
            ),
            [seq.map(|s| s as i64)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                ))
            },
        )
        .optional()
        .map_err(file_error)?;
    let Some((reason, boot, frame, stamped_before, clock_ms, patches, core)) = row else {
        return Ok(None);
    };
    let broken = || StorageError::File(format!("a checkpoint in {table} can't be read"));
    let core = zstd::decode_all(&core[..]).map_err(|_| broken())?;
    Ok(Some(StoredCheckpoint {
        reason: serde_json::from_str(&reason).map_err(|_| broken())?,
        boot,
        frame: frame as u64,
        stamped_before: stamped_before as usize,
        clock_ms,
        patches: decode_patches(&patches).ok_or_else(broken)?,
        core: decode_core(&core).ok_or_else(broken)?,
    }))
}

/// The entries replace any kept from the one numbered `from` on, in one transaction.
fn keep_log(db: &Connection, from: u64, entries: &[LogEntry]) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute("DELETE FROM run_log WHERE seq >= ?1", [from as i64])?;
    for entry in entries {
        tx.execute(
            "INSERT INTO run_log (seq, entry) VALUES (?1, ?2)",
            params![entry.seq as i64, entry.to_bytes()],
        )?;
    }
    tx.commit()
}

/// The entries waiting for a Host replace those kept before, in one transaction.
fn keep_waiting(db: &Connection, waiting: &[Submission]) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute("DELETE FROM waiting", [])?;
    for submission in waiting {
        tx.execute("INSERT INTO waiting (submission) VALUES (?1)", [submission.to_bytes()])?;
    }
    tx.commit()
}

/// A Shadow Game's checkpoint replaces the one kept for its Player.
fn keep_shadow(db: &Connection, player: &PlayerKey, checkpoint: &ShadowCheckpoint) -> rusqlite::Result<()> {
    let core = zstd::encode_all(&encode_core(&checkpoint.core)[..], ZSTD_LEVEL)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let seen = serde_json::to_string(&(&checkpoint.owned, &checkpoint.fainted)).expect("PIDs serialise");
    db.execute(
        "INSERT OR REPLACE INTO shadow_checkpoints (player, boot, frame, patches, seen, core)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            &player[..],
            checkpoint.boot,
            checkpoint.frame as i64,
            encode_patches(&checkpoint.patches),
            seen,
            core
        ],
    )
    .map(drop)
}

/// Every Shadow Game's kept checkpoint, by Player.
fn read_shadows(db: &Connection) -> Result<Vec<(PlayerKey, ShadowCheckpoint)>, StorageError> {
    let rows = db
        .prepare("SELECT player, boot, frame, patches, seen, core FROM shadow_checkpoints ORDER BY player")
        .and_then(|mut query| {
            query
                .query_map([], |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(file_error)?;
    let broken = || StorageError::File("a Shadow Game's checkpoint can't be read".into());
    rows.into_iter()
        .map(|(player, boot, frame, patches, seen, core)| {
            let player: PlayerKey = player.try_into().map_err(|_| broken())?;
            let (owned, fainted): (Vec<u32>, Vec<u32>) = serde_json::from_str(&seen).map_err(|_| broken())?;
            let core = zstd::decode_all(&core[..]).map_err(|_| broken())?;
            Ok((
                player,
                ShadowCheckpoint {
                    boot,
                    frame: frame as u64,
                    core: decode_core(&core).ok_or_else(broken)?,
                    patches: decode_patches(&patches).ok_or_else(broken)?,
                    owned,
                    fainted,
                },
            ))
        })
        .collect()
}

fn record_inputs(db: &Connection, batch: &InputBatch) -> rusqlite::Result<()> {
    let keys: Vec<u8> = batch.keys.iter().flat_map(|k| k.0.to_le_bytes()).collect();
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO inputs (boot, first_frame, keys) VALUES (?1, ?2, ?3)",
        params![batch.boot, batch.first_frame as i64, keys],
    )?;
    let input_id = tx.last_insert_rowid();
    for stamped in &batch.stamped {
        let (patch, addr, bytes) = match &stamped.change {
            Change::Write { addr, bytes } => (false, addr, bytes),
            Change::Patch { addr, bytes } => (true, addr, bytes),
        };
        tx.execute(
            "INSERT INTO stamped (input_id, frame, patch, addr, bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![input_id, stamped.frame as i64, patch, addr, bytes],
        )?;
    }
    tx.commit()
}

fn load(db: &Connection) -> Result<StoredRun, StorageError> {
    let info = read_info(db).map_err(file_error)?.ok_or(StorageError::NoRun)?;
    let events = db
        .prepare("SELECT seq, boot, frame, at_ms, event FROM events ORDER BY seq")
        .and_then(|mut query| {
            query
                .query_map([], |row| {
                    let event: String = row.get(4)?;
                    Ok(Recorded {
                        seq: row.get::<_, i64>(0)? as u64,
                        boot: row.get(1)?,
                        frame: row.get::<_, i64>(2)? as u64,
                        at_ms: row.get(3)?,
                        event: serde_json::from_str(&event).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
                        })?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(file_error)?;

    let mut inputs = db
        .prepare("SELECT id, boot, first_frame, keys FROM inputs ORDER BY id")
        .and_then(|mut query| {
            query
                .query_map([], |row| {
                    let keys: Vec<u8> = row.get(3)?;
                    Ok((
                        row.get::<_, i64>(0)?,
                        InputBatch {
                            boot: row.get(1)?,
                            first_frame: row.get::<_, i64>(2)? as u64,
                            keys: keys
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|pair| Keys(u16::from_le_bytes(*pair)))
                                .collect(),
                            stamped: Vec::new(),
                        },
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(file_error)?;
    let mut stamped = db
        .prepare("SELECT input_id, frame, patch, addr, bytes FROM stamped ORDER BY rowid")
        .map_err(file_error)?;
    let rows = stamped
        .query_map([], |row| {
            let (addr, bytes): (u32, Vec<u8>) = (row.get(3)?, row.get(4)?);
            let change = if row.get::<_, bool>(2)? {
                Change::Patch { addr, bytes }
            } else {
                Change::Write { addr, bytes }
            };
            Ok((
                row.get::<_, i64>(0)?,
                Stamped {
                    frame: row.get::<_, i64>(1)? as u64,
                    change,
                },
            ))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(file_error)?;
    for (input_id, stamp) in rows {
        if let Some((_, batch)) = inputs.iter_mut().find(|(id, _)| *id == input_id) {
            batch.stamped.push(stamp);
        }
    }

    let log = db
        .prepare("SELECT entry FROM run_log ORDER BY seq")
        .and_then(|mut query| {
            query
                .query_map([], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(file_error)?
        .iter()
        .map(|bytes| {
            LogEntry::from_bytes(bytes).ok_or_else(|| StorageError::File("a Run log entry can't be read".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let waiting = db
        .prepare("SELECT submission FROM waiting ORDER BY id")
        .and_then(|mut query| {
            query
                .query_map([], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(file_error)?
        .iter()
        .map(|bytes| {
            Submission::from_bytes(bytes)
                .ok_or_else(|| StorageError::File("a waiting Run log entry can't be read".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let save_memory = db
        .query_row("SELECT bytes FROM save_memory WHERE id = 1", [], |row| row.get(0))
        .optional()
        .map_err(file_error)?;
    Ok(StoredRun {
        info,
        events,
        inputs: inputs.into_iter().map(|(_, batch)| batch).collect(),
        save_memory,
        latest_checkpoint: read_checkpoint(db, Slot::Checkpoints)?,
        suspended: read_checkpoint(db, Slot::Suspend)?,
        log,
        waiting,
        shadows: read_shadows(db)?,
    })
}
