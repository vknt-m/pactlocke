//! The Run file: one SQLite database per Run (WAL, synchronous NORMAL),
//! written on a dedicated thread fed by a channel, and the folder of Run
//! files the home screen lists.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use pactlocke_emu::Keys;
use pactlocke_game::Change;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::storage::{InputBatch, RunInfo, RunStorage, Stamped, StorageError, StoredRun};
use crate::{Recorded, Ruleset};

/// Run files end in this.
const EXTENSION: &str = "pactrun";

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
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let mut id = RunId(format!("run-{stamp}"));
        let mut n = 1;
        while self.path(&id).exists() {
            n += 1;
            id = RunId(format!("run-{stamp}-{n}"));
        }
        RunFile::open(self.path(&id))
    }

    /// An existing Run's file.
    pub fn open_run(&self, id: &RunId) -> Result<RunFile, StorageError> {
        let path = self.path(id);
        if !path.exists() {
            return Err(StorageError::NoRun);
        }
        RunFile::open(path)
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
    Some(RunSummary {
        id,
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
    Load(Sender<Result<StoredRun, StorageError>>),
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

    fn load(&mut self) -> Result<StoredRun, StorageError> {
        self.ask(Message::Load)
    }

    fn write_failure(&mut self) -> Option<String> {
        self.failure.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
            Message::Load(reply) => {
                let _ = reply.send(load(db));
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

    let save_memory = db
        .query_row("SELECT bytes FROM save_memory WHERE id = 1", [], |row| row.get(0))
        .optional()
        .map_err(file_error)?;
    Ok(StoredRun {
        info,
        events,
        inputs: inputs.into_iter().map(|(_, batch)| batch).collect(),
        save_memory,
    })
}
