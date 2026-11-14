//! Resume for a Run with friends, before its game boots: the Run's network
//! opens from this Player's copy of its Run log, by the Host's key with no
//! invite, and after a crash the Host gives back the presses the Run file
//! lost, so the game goes on from the frame the others have. The windowed
//! app and the headless one both resume this way.

use std::path::PathBuf;

use pactlocke_run::{BootPlan, GamePoint, InputChunk, Locked, Outgoing, RunEngine, RunLog, StreamMessage, StreamWant};

use crate::lobby_ui::Lobbies;

/// How long a Run resumed after a crash waits for its Host before booting
/// without the presses the Host may have.
const TAKE_BACK_WAIT_MS: i64 = 15_000;
/// How long it waits for the Host's answer, once asked.
const TAKE_BACK_ANSWER_MS: i64 = 3_000;

/// A Run with friends opened with Resume, its game not booted yet.
pub struct Resuming {
    pub engine: RunEngine,
    pub locked: Locked,
    /// This Player's copy of the Run log, as the Run file kept it.
    pub log: RunLog,
    since_ms: i64,
    /// When the Host was asked for this Player's lost presses.
    asked_ms: Option<i64>,
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
        })
    }

    /// Whether the game can boot now. A game rebuilt after a crash first
    /// asks the Host for this Player's presses after what the Run file kept,
    /// and plays them too; it boots once they are back, or once there is no
    /// Host to ask: this Player hosts, or none answered in time.
    pub fn ready_to_boot(&mut self, lobbies: &mut Lobbies, now_ms: i64) -> bool {
        let me = lobbies.public_key();
        let host = lobbies.host_key().filter(|h| Some(*h) != me);
        let lost = match self.engine.boot_plan() {
            BootPlan::Rebuild { from, keys, .. } => Some(GamePoint {
                boot: from.boot,
                frame: from.frame + keys.len() as u64,
            }),
            _ => None,
        };
        let (Some(lost), Some(host), Some(me)) = (lost, host, me) else {
            return true;
        };
        if now_ms - self.since_ms >= TAKE_BACK_WAIT_MS {
            return true;
        }
        if !lobbies.online() {
            return false;
        }
        let Some(asked) = self.asked_ms else {
            let want = StreamWant {
                player: me,
                boot: lost.boot,
                from_frame: lost.frame,
            };
            lobbies.send_stream(Outgoing::To(host, StreamMessage::Want(want)));
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
        if mine.is_empty() && now_ms - asked < TAKE_BACK_ANSWER_MS {
            return false;
        }
        self.engine.take_back_presses(&mine);
        true
    }
}
