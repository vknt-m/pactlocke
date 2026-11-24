//! Cheat flags: what one Player's app sees of another's stream that their
//! game can't have done honestly. A flag says who, what and when, goes in
//! the Run log signed by the Player who saw it, and stays there: no Host or
//! Retry undoes it, and it triggers nothing. The group decides what to do
//! about it: a vote of everyone but the flagged Player clears it, or a
//! Dropout vote.
//!
//! Seen here: a stream that rewrites frames the others already have (a
//! savestate load, or rewritten history); a new boot with no Retry behind it
//! (a savestate load from before the frames the others have); frames coming
//! faster than 4x for longer than a burst; a Shadow Game that no longer
//! matches ("game out of sync", as the cause is unclear). A Player's own
//! app flags a Run Checkpoint in its Run file that doesn't match its hash
//! in the log (Run-file tampering).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::shared_log::RunLog;
use crate::stream::{InputChunk, PlayerKey, Rewrite};
use crate::{GamePoint, RunEvent};

/// The fastest a Player may play: 4x a GBA's 59.7275 frames a second, in frames per millisecond.
const MAX_FRAMES_PER_MS: f64 = 4.0 * 59.7275 / 1000.0;
/// Frames over that pace let through before a flag: ten seconds at 4x, for
/// a stream that bunches up on a slow connection. Time a Player is silent
/// adds its 4x worth on top, so their frames from then can all come at once.
pub const FAST_BURST_FRAMES: u64 = 2390;
/// Frames arriving within this long of the end of a silence at least this
/// long are its catch-up: they may spend what the silence earned. Later
/// frames get the burst alone, so a pause is no credit for playing fast.
pub const CATCH_UP_MS: i64 = 2000;
/// A new boot with no Retry in the log here is flagged only if none arrives
/// in this long: the log and the stream travel apart.
pub const RETRY_WAIT_MS: i64 = 10_000;

/// What a cheat flag says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cheat {
    /// Their stream rewrote frames the others already had: a savestate load
    /// or rewritten history.
    HistoryRewritten,
    /// Their game started again from an earlier point with no Retry in the
    /// log: a savestate load.
    RestartedWithoutRetry,
    /// Their frames came faster than 4x allows, for longer than a burst.
    TooFast,
    /// Their Shadow Game stopped matching their game: a memory edit, a cheat
    /// code or a bug. The cause is unclear.
    OutOfSync,
    /// A Run Checkpoint in their Run file doesn't match its hash in the log.
    CheckpointTampered,
}

impl Cheat {
    /// What it says in the feed.
    pub fn describe(self) -> &'static str {
        match self {
            Cheat::HistoryRewritten => {
                "presses that rewrite frames already played (a savestate load or rewritten history)"
            }
            Cheat::RestartedWithoutRetry => "the game went back to an earlier point without a Retry (a savestate load)",
            Cheat::TooFast => "played faster than 4x",
            Cheat::OutOfSync => "game out of sync",
            Cheat::CheckpointTampered => "a Run Checkpoint in the Run file doesn't match its hash in the log",
        }
    }
}

/// A cheat flag as the Run log has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    /// Whose game it is about.
    pub player: PlayerKey,
    /// What happened.
    pub cheat: Cheat,
    /// Where in their game: the boot and frame boundary.
    pub at: GamePoint,
    /// When it was seen, by the flagging app's clock (Unix milliseconds).
    pub at_ms: i64,
    /// Who saw it, in log order: the first, and any who flagged the same after.
    pub by: Vec<PlayerKey>,
}

impl Flag {
    /// Whether this is the flag for that Player, cheat and boot: one each.
    pub fn is(&self, player: &PlayerKey, cheat: Cheat, boot: u32) -> bool {
        self.player == *player && self.cheat == cheat && self.at.boot == boot
    }
}

/// What this app has seen of each other Player's stream, to flag what
/// their game can't have done honestly.
#[derive(Debug, Clone)]
pub struct CheatWatch {
    /// When the Run started, by this app's clock: every stream counts from there.
    start_ms: i64,
    players: BTreeMap<PlayerKey, Pace>,
    /// Flags already raised here: one per Player, cheat and boot.
    raised: BTreeSet<(PlayerKey, Cheat, u32)>,
}

/// One Player's stream as it arrived here.
#[derive(Debug, Clone)]
struct Pace {
    boot: u32,
    /// The frame after the last that arrived, in this boot.
    end: u64,
    /// When frames last arrived.
    last_ms: i64,
    /// Frames they may still send over the 4x pace.
    credit: f64,
    /// Until when frames are the catch-up after a silence (Unix milliseconds).
    catch_up_until: i64,
    /// The Retries their boots so far went back by, by call.
    retries: BTreeSet<u64>,
    /// A boot with no Retry in the log yet: where it began, and when it arrived.
    unexplained: Option<(GamePoint, i64)>,
    /// The Run Checkpoint that boot says it began from.
    booted_from: Option<GamePoint>,
}

impl Pace {
    /// A stream from `at`, with `credit` to spend over 4x from `last_ms`.
    fn from(at: GamePoint, last_ms: i64, credit: f64) -> Self {
        Self {
            boot: at.boot,
            end: at.frame,
            last_ms,
            credit,
            catch_up_until: last_ms,
            retries: BTreeSet::new(),
            unexplained: None,
            booted_from: None,
        }
    }

    /// The allowance as `n` more frames arrive at `now_ms`: it refills at 4x
    /// up to a burst, and a silence adds its 4x worth on top for its catch-up.
    fn spend(&mut self, n: u64, now_ms: i64) {
        let gap = (now_ms - self.last_ms).max(0);
        if now_ms > self.catch_up_until {
            self.credit = self.credit.min(FAST_BURST_FRAMES as f64);
        }
        if gap >= CATCH_UP_MS {
            self.catch_up_until = now_ms + CATCH_UP_MS;
        }
        self.credit += gap as f64 * MAX_FRAMES_PER_MS - n as f64;
        self.last_ms = now_ms;
    }
}

impl CheatWatch {
    /// For a Run that started at `start_ms` (Unix milliseconds).
    pub fn new(start_ms: i64) -> Self {
        Self {
            start_ms,
            players: BTreeMap::new(),
            raised: BTreeSet::new(),
        }
    }

    /// On a resume at `now_ms`: this app has another Player's stream from
    /// `at`, in a boot of theirs that began at `boot_ms` (Unix milliseconds,
    /// by the clock their boot set; `None` for the Run's start). The whole
    /// boot counts against 4x from then, so what they played while this app
    /// was away comes at once as a catch-up.
    pub fn resumed(&mut self, player: PlayerKey, at: GamePoint, boot_ms: Option<i64>, now_ms: i64) {
        let since = boot_ms
            .unwrap_or(self.start_ms)
            .clamp(self.start_ms, now_ms.max(self.start_ms));
        let credit = FAST_BURST_FRAMES as f64 - at.frame as f64;
        self.players.insert(player, Pace::from(at, since, credit));
    }

    /// A chunk of another Player's stream, now in order here. Flags a pace
    /// over 4x, and notes a new boot to check against the log's Retries.
    pub fn arrived(&mut self, chunk: &InputChunk, log: &RunLog, now_ms: i64) -> Vec<RunEvent> {
        let player = chunk.player;
        let start_ms = self.start_ms;
        let pace = self
            .players
            .entry(player)
            .or_insert_with(|| Pace::from(GamePoint { boot: 1, frame: 0 }, start_ms, FAST_BURST_FRAMES as f64));
        if chunk.boot > pace.boot {
            pace.unexplained = Some((
                GamePoint {
                    boot: chunk.boot,
                    frame: chunk.first_frame,
                },
                now_ms,
            ));
            pace.boot = chunk.boot;
            pace.end = chunk.first_frame;
            pace.booted_from = chunk.booted_from.map(|from| GamePoint {
                boot: from.boot,
                frame: from.frame,
            });
        }
        pace.spend(chunk.end().saturating_sub(pace.end), now_ms);
        pace.end = pace.end.max(chunk.end());
        let mut flags = Vec::new();
        if pace.credit < 0.0 {
            pace.credit = 0.0;
            let at = GamePoint {
                boot: chunk.boot,
                frame: chunk.end(),
            };
            flags.extend(self.raise(log, player, Cheat::TooFast, at, now_ms));
        }
        flags.extend(self.due(log, now_ms));
        flags
    }

    /// New boots the log has no Retry for: they are flagged once the log
    /// has had time to catch up with the stream.
    pub fn due(&mut self, log: &RunLog, now_ms: i64) -> Vec<RunEvent> {
        let mut flags = Vec::new();
        let players: Vec<PlayerKey> = self.players.keys().copied().collect();
        for player in players {
            let pace = self.players.get_mut(&player).expect("listed");
            let Some((at, since)) = pace.unexplained else {
                continue;
            };
            if let Some(call) = pace
                .booted_from
                .and_then(|from| retry_to(log, &player, from, &pace.retries))
            {
                pace.retries.insert(call);
                pace.unexplained = None;
            } else if now_ms - since >= RETRY_WAIT_MS {
                pace.unexplained = None;
                flags.extend(self.raise(log, player, Cheat::RestartedWithoutRetry, at, now_ms));
            }
        }
        flags
    }

    /// Another Player's stream rewrote a frame this app already had.
    pub fn rewritten(&mut self, log: &RunLog, rewrite: &Rewrite, now_ms: i64) -> Option<RunEvent> {
        self.raise(log, rewrite.player, Cheat::HistoryRewritten, rewrite.at, now_ms)
    }

    /// Another Player's Shadow Game here stopped matching their game.
    pub fn diverged(&mut self, log: &RunLog, player: PlayerKey, at: GamePoint, now_ms: i64) -> Option<RunEvent> {
        self.raise(log, player, Cheat::OutOfSync, at, now_ms)
    }

    /// A flag, unless this app or the log has it already for that boot.
    fn raise(&mut self, log: &RunLog, player: PlayerKey, cheat: Cheat, at: GamePoint, now_ms: i64) -> Option<RunEvent> {
        let flagged = log.flags().iter().any(|f| f.is(&player, cheat, at.boot));
        if flagged || !self.raised.insert((player, cheat, at.boot)) {
            return None;
        }
        let who: String = player.iter().take(4).map(|b| format!("{b:02x}")).collect();
        log::warn!(
            "Cheat flag raised: {cheat:?} on {who}, boot {} frame {}",
            at.boot,
            at.frame
        );
        Some(RunEvent::CheatFlag {
            player: Some(player),
            cheat,
            boot: at.boot,
            frame: at.frame,
            at_ms: now_ms,
        })
    }
}

/// The Retry in the log that sent this Player's game back to `at`, not one
/// of `used` already.
fn retry_to(log: &RunLog, player: &PlayerKey, at: GamePoint, used: &BTreeSet<u64>) -> Option<u64> {
    log.retries()
        .into_iter()
        .find(|(call, to)| !used.contains(call) && to.get(player) == Some(&at))
        .map(|(call, _)| call)
}
