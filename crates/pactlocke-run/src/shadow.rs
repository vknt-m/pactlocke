//! A Shadow Game: a copy of another Player's game, run from their input
//! stream alone, so it matches theirs exactly. It plays about
//! [`BUFFER_FRAMES`] behind the newest frame received. When the stream
//! stalls (a network hiccup) it never pauses: it plays on, guessing the last
//! keys held, from a checkpoint of its own; when the real frames arrive and
//! the guess was wrong, it rewinds to that checkpoint and replays them. Its
//! RAM and save hashes are checked against the Player's at every boundary
//! both have; a mismatch on real frames is real divergence.
//!
//! It is plain state with no thread or clock of its own: the caller gives it
//! chunks in order (see [`crate::StreamStore`]) and the time, and runs it.

use std::collections::{BTreeMap, VecDeque};

use pactlocke_emu::{Checkpoint, CoreAdapter, Frame, Keys};
use pactlocke_game::Change;

use crate::Stamped;
use crate::stream::{FrameHash, HASH_EVERY, InputChunk};

/// How far behind the newest frame received a Shadow plays.
pub const BUFFER_FRAMES: u64 = 30;
/// How far past the newest frame received a Shadow guesses before it holds:
/// longer than a hiccup, and the Player has likely paused or gone.
pub const PREDICT_FRAMES: u64 = 120;
/// How much faster than the Player a Shadow that is behind catches up.
pub const CATCH_UP_SPEED: u64 = 20;
/// Behind by more than this, a Shadow is catching up rather than watchable live.
const CATCHING_UP_FRAMES: u64 = 3 * BUFFER_FRAMES;
/// A GBA's frame rate, in frames per millisecond, before the stream shows the Player's.
const GBA_FRAMES_PER_MS: f64 = 16_777_216.0 / 280_896.0 / 1000.0;

/// Where a Shadow Game stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowState {
    /// Matching the Player's game, as far as the hashes go.
    Matched,
    /// Its hash and the Player's differed at this real frame boundary: it
    /// can't be watched until it is built again.
    Diverged { frame: u64 },
    /// The Player booted their game again (a resume, a crash rebuild): this
    /// Shadow can't follow a new boot yet, so it can't be watched.
    Restarted { boot: u32 },
}

/// A copy of one Player's game, replayed from their input stream.
pub struct ShadowGame<C: CoreAdapter> {
    core: C,
    /// Which boot of the Player's game it copies.
    boot: u32,
    /// Frames run on the Player's real keys.
    confirmed: u64,
    /// Real keys received and not yet run, from frame `confirmed + predicted.len()`
    /// on, unless guesses are pending (then from `confirmed`).
    keys: VecDeque<Keys>,
    /// Changes received and not yet applied, in order.
    stamped: VecDeque<Stamped>,
    /// Frames run past `confirmed` on guessed keys, and the checkpoint
    /// taken at `confirmed` before the first guess.
    guessed: Vec<Keys>,
    rollback: Option<Checkpoint>,
    /// The frame after the newest received.
    received: u64,
    last_keys: Keys,
    /// Every ROM patch active, so a rewind can put back exactly that set.
    patches: BTreeMap<u32, Vec<u8>>,
    theirs: BTreeMap<u64, FrameHash>,
    ours: BTreeMap<u64, FrameHash>,
    /// Hashes taken on guessed frames, kept only if the guess is confirmed.
    guessed_hashes: BTreeMap<u64, FrameHash>,
    /// When the newest chunk arrived, and the Player's pace in frames per ms.
    last_arrival_ms: Option<i64>,
    pace: f64,
    state: ShadowState,
    rewinds: u64,
    matched: u64,
}

impl<C: CoreAdapter> ShadowGame<C> {
    /// A Shadow of a game booted from power-on with its clock at `clock_ms`:
    /// `core` has the same ROM loaded and has run nothing.
    pub fn power_on(mut core: C, clock_ms: i64) -> Self {
        core.set_clock(clock_ms);
        Self {
            core,
            boot: 1,
            confirmed: 0,
            keys: VecDeque::new(),
            stamped: VecDeque::new(),
            guessed: Vec::new(),
            rollback: None,
            received: 0,
            last_keys: Keys::NONE,
            patches: BTreeMap::new(),
            theirs: BTreeMap::new(),
            ours: BTreeMap::new(),
            guessed_hashes: BTreeMap::new(),
            last_arrival_ms: None,
            pace: GBA_FRAMES_PER_MS,
            state: ShadowState::Matched,
            rewinds: 0,
            matched: 0,
        }
    }

    /// Takes in the next chunk of the Player's stream, which must follow on
    /// from the last (a [`crate::StreamStore`] puts them in order). If it
    /// shows a guess was wrong, the Shadow rewinds to before the guess.
    pub fn receive(&mut self, chunk: &InputChunk, now_ms: i64) {
        if self.state != ShadowState::Matched {
            return;
        }
        if chunk.boot > self.boot {
            self.state = ShadowState::Restarted { boot: chunk.boot };
            return;
        }
        if chunk.boot < self.boot || chunk.first_frame != self.received || chunk.keys.is_empty() {
            return;
        }
        if let Some(last) = self.last_arrival_ms {
            let elapsed = (now_ms - last).max(1) as f64;
            let pace = chunk.keys.len() as f64 / elapsed;
            // A burst (a resend, a catch-up) says nothing about the Player's pace.
            if elapsed >= 4.0 {
                self.pace = (self.pace * 0.75 + pace * 0.25).clamp(GBA_FRAMES_PER_MS / 2.0, GBA_FRAMES_PER_MS * 4.5);
            }
        }
        self.last_arrival_ms = Some(now_ms);
        self.received = chunk.end();
        self.keys.extend(chunk.keys.iter().copied());
        self.stamped.extend(chunk.stamped.iter().cloned());
        for hash in &chunk.hashes {
            self.theirs.insert(hash.frame, *hash);
        }
        self.check_guesses();
        self.compare();
    }

    /// Plays toward where the Player is now, less the buffer, running at
    /// most `max_frames`. Returns how many frames ran.
    pub fn advance(&mut self, now_ms: i64, max_frames: u64) -> u64 {
        let target = self.target(now_ms);
        let mut ran = 0;
        while ran < max_frames && self.state == ShadowState::Matched && self.frame() < target {
            if self.guessed.is_empty() && !self.keys.is_empty() {
                self.run_real();
            } else if self.frame() < self.received + PREDICT_FRAMES {
                self.run_guess();
            } else {
                break;
            }
            ran += 1;
        }
        ran
    }

    /// The frame the Shadow should be at now: the Player's estimated frame,
    /// less the buffer. Past the newest frame received it grows at the
    /// Player's pace, up to how far a Shadow guesses.
    fn target(&self, now_ms: i64) -> u64 {
        let since = self.last_arrival_ms.map_or(0, |at| (now_ms - at).max(0));
        let ahead = (since as f64 * self.pace) as u64;
        let edge = self.received + ahead.min(PREDICT_FRAMES + BUFFER_FRAMES);
        edge.saturating_sub(BUFFER_FRAMES)
    }

    /// How many frames to run in a slice of `ms` to keep up or catch up:
    /// up to [`CATCH_UP_SPEED`] times the Player's pace.
    pub fn frames_for(&self, ms: i64) -> u64 {
        ((ms.max(1) as f64 * self.pace * CATCH_UP_SPEED as f64).ceil() as u64).max(1)
    }

    /// Frames run, real and guessed.
    pub fn frame(&self) -> u64 {
        self.confirmed + self.guessed.len() as u64
    }

    /// Frames run on the Player's real keys.
    pub fn confirmed(&self) -> u64 {
        self.confirmed
    }

    /// The frame after the newest one received.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Whether it matches, and if not, why.
    pub fn state(&self) -> ShadowState {
        self.state
    }

    /// How often a guess turned out wrong and the Shadow rewound.
    pub fn rewinds(&self) -> u64 {
        self.rewinds
    }

    /// Whether it is well behind where it should be, fast-forwarding.
    pub fn catching_up(&self, now_ms: i64) -> bool {
        self.target(now_ms).saturating_sub(self.frame()) > CATCHING_UP_FRAMES
    }

    /// Boundaries whose hashes both sides have and that matched.
    pub fn hashes_matched(&self) -> u64 {
        self.matched
    }

    /// Its picture now.
    pub fn video(&self) -> Frame {
        self.core.frame()
    }

    /// The rate of the samples `drain_audio` gives.
    pub fn audio_sample_rate(&self) -> u32 {
        self.core.audio_sample_rate()
    }

    /// Its sound since the last call, as interleaved stereo.
    pub fn drain_audio(&mut self, out: &mut Vec<i16>) {
        self.core.drain_audio(out);
    }

    /// The core under it, to read its memory.
    pub fn core_mut(&mut self) -> &mut C {
        &mut self.core
    }

    /// Runs the next frame on the Player's keys: the changes stamped at
    /// this boundary land first, then the hash is taken if one is due.
    fn run_real(&mut self) {
        let boundary = self.confirmed;
        self.apply_stamped(boundary);
        if boundary.is_multiple_of(HASH_EVERY) {
            let hash = FrameHash::of(&mut self.core, boundary);
            self.ours.insert(boundary, hash);
            self.compare();
        }
        let keys = self.keys.pop_front().expect("a real frame to run");
        self.core.run_frame(keys);
        self.last_keys = keys;
        self.confirmed += 1;
    }

    /// Runs a frame past the newest received on a guess: the last keys held,
    /// and nothing stamped. A checkpoint is taken before the first guess.
    fn run_guess(&mut self) {
        if self.rollback.is_none() {
            self.rollback = Some(self.core.checkpoint());
        }
        let boundary = self.frame();
        if boundary.is_multiple_of(HASH_EVERY) {
            let hash = FrameHash::of(&mut self.core, boundary);
            self.guessed_hashes.insert(boundary, hash);
        }
        let keys = self.keys.back().copied().unwrap_or(self.last_keys);
        self.core.run_frame(keys);
        self.guessed.push(keys);
    }

    fn apply_stamped(&mut self, boundary: u64) {
        while let Some(stamped) = self.stamped.front().filter(|s| s.frame <= boundary) {
            let change = stamped.change.clone();
            self.stamped.pop_front();
            match change {
                Change::Write { addr, bytes } => self.core.write(addr, &bytes),
                Change::Patch { addr, bytes } => {
                    self.core.patch_rom(addr, &bytes);
                    self.patches.insert(addr, bytes);
                }
            }
        }
    }

    /// With the real frames for the guesses here: keeps the guesses if they
    /// were right (the same keys, nothing stamped among them), and rewinds
    /// to the checkpoint before them if not. Until every guessed frame has
    /// arrived, it waits. A checkpoint the core refuses leaves nothing to
    /// trust: the Shadow is out of sync from there.
    fn check_guesses(&mut self) {
        if self.guessed.is_empty() {
            return;
        }
        let n = self.guessed.len();
        let right_so_far = self.keys.iter().zip(&self.guessed).all(|(real, guess)| real == guess)
            && !self.stamped.iter().any(|s| s.frame < self.confirmed + n as u64);
        if right_so_far && self.keys.len() < n {
            return;
        }
        if right_so_far {
            self.keys.drain(..n);
            self.last_keys = *self.guessed.last().expect("guesses");
            self.confirmed += n as u64;
            self.guessed.clear();
            self.rollback = None;
            self.ours.append(&mut self.guessed_hashes);
            return;
        }
        let checkpoint = self.rollback.take().expect("a checkpoint before the guesses");
        if self.core.restore(&checkpoint).is_err() {
            self.state = ShadowState::Diverged { frame: self.confirmed };
            return;
        }
        for (addr, bytes) in &self.patches {
            self.core.patch_rom(*addr, bytes);
        }
        self.guessed.clear();
        self.guessed_hashes.clear();
        self.rewinds += 1;
    }

    /// Compares every boundary both sides have hashed, then forgets them.
    fn compare(&mut self) {
        let both: Vec<u64> = self
            .ours
            .keys()
            .filter(|f| self.theirs.contains_key(f))
            .copied()
            .collect();
        for frame in both {
            let ours = self.ours.remove(&frame).expect("ours");
            let theirs = self.theirs.remove(&frame).expect("theirs");
            if ours.matches(&theirs) {
                self.matched += 1;
            } else if self.state == ShadowState::Matched {
                self.state = ShadowState::Diverged { frame };
            }
        }
    }
}
