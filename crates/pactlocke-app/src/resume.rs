//! Resume for a Run with friends, before its game boots: the Run's network
//! opens from this Player's copy of its Run log, by the Host's key with no
//! invite, and after a crash the other Players give back the presses the
//! Run file lost, so the game goes on from the frame the others have. The windowed
//! app and the headless one both resume this way.

use std::path::PathBuf;

use pactlocke_run::{BootPlan, GamePoint, InputChunk, Locked, Outgoing, RunEngine, RunLog, StreamMessage, StreamWant};

use crate::lobby_ui::Lobbies;

/// How long a Run resumed after a crash waits for another Player before
/// booting without the presses they may have: long enough for a Host whose
/// group moved on to a new Host to step down and find them.
const TAKE_BACK_WAIT_MS: i64 = 45_000;
/// How long it gathers the answers, once it has asked.
const TAKE_BACK_ANSWER_MS: i64 = 3_000;

/// A Run with friends opened with Resume, its game not booted yet.
pub struct Resuming {
    pub engine: RunEngine,
    pub locked: Locked,
    /// This Player's copy of the Run log, as the Run file kept it.
    pub log: RunLog,
    since_ms: i64,
    /// When the other Players were asked for this Player's lost presses,
    /// and what has come back so far: each may have a different amount.
    asked_ms: Option<i64>,
    taken_back: Vec<InputChunk>,
}

impl Resuming {
    /// Opens the Run's network for the Run `engine` resumed from its Run
    /// file, on the ROM at `rom`: never as a solo game.
    pub fn begin(engine: RunEngine, rom: PathBuf, lobbies: &mut Lobbies, now_ms: i64) -> Result<Self, String> {
        let log = RunLog::from_kept(engine.kept_log().to_vec())
            .map_err(|e| format!("This Run's log in its Run file can't be read: {e}."))?;
        let locked = log
            .locked()
            .ok_or("This Run's log has no lock, so it can't be resumed.")?;
        lobbies.resume(log.clone(), engine.kept_waiting().to_vec(), rom)?;
        Ok(Self {
            engine,
            locked,
            log,
            since_ms: now_ms,
            asked_ms: None,
            taken_back: Vec::new(),
        })
    }

    /// Whether the game can boot now. A game rebuilt after a crash first
    /// asks every Player it reaches for this Player's presses after what the
    /// Run file kept (a joiner asks the Host, which has every stream; a Host
    /// asks each Player, who has the Host's), plays them too, and boots once
    /// the answers are in; or, with nobody to ask, after a while. Only a
    /// resume after a crash waits: nothing here runs while the game plays.
    pub fn ready_to_boot(&mut self, lobbies: &mut Lobbies, now_ms: i64) -> bool {
        let lost = match self.engine.boot_plan() {
            BootPlan::Rebuild { from, keys, .. } => GamePoint {
                boot: from.boot,
                frame: from.frame + keys.len() as u64,
            },
            _ => return true,
        };
        let Some(me) = lobbies.public_key() else {
            return true;
        };
        let Some(asked) = self.asked_ms else {
            if now_ms - self.since_ms >= TAKE_BACK_WAIT_MS {
                log::warn!("No Player answered: the game goes on from what the Run file kept");
                return true;
            }
            if !lobbies.reaches_anyone() {
                return false;
            }
            let want = StreamWant {
                player: me,
                boot: lost.boot,
                from_frame: lost.frame,
            };
            // A Host sends it to every Player connected; a joiner to its Host.
            lobbies.send_stream(Outgoing::AllBut(me, StreamMessage::Want(want)));
            self.asked_ms = Some(now_ms);
            return false;
        };
        // Other Players' chunks here are dropped: their Shadows ask again for any gap.
        let mine: Vec<InputChunk> = lobbies
            .take_stream()
            .into_iter()
            .filter_map(|(_, bytes)| match StreamMessage::from_bytes(&bytes) {
                Some(StreamMessage::Chunk(chunk)) if chunk.player == me => Some(chunk),
                _ => None,
            })
            .collect();
        self.taken_back.extend(mine);
        if now_ms - asked < TAKE_BACK_ANSWER_MS {
            return false;
        }
        self.engine.take_back_presses(&std::mem::take(&mut self.taken_back));
        true
    }
}
