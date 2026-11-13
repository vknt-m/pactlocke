//! A Shadow Game: a copy of another Player's game, run from their input
//! stream alone, so it matches theirs exactly. It plays about
//! [`BUFFER_FRAMES`] behind the newest frame received. When the stream
//! stalls (a network hiccup) it never pauses: it plays on, guessing the last
//! keys held, from a checkpoint of its own; when the real frames arrive and
//! the guess was wrong, it rewinds to that checkpoint and replays them. Its
//! RAM and save hashes are checked against the Player's at every boundary
//! both have; a mismatch on real frames is real divergence.
//!
//! It takes its own checkpoint wherever the Player's stream marks a Run
//! Checkpoint. When the Player's game goes back to one in a Retry, their
//! stream's new boot says which, and the Shadow goes back to its own.
//!
//! It is plain state with no thread or clock of its own: the caller gives it
//! chunks in order (see [`crate::StreamStore`]) and the time, and runs it.

use std::collections::{BTreeMap, VecDeque};

use pactlocke_emu::{Checkpoint, CoreAdapter, Frame, Keys};
use pactlocke_game::Change;

use crate::PlayedGame;
use crate::stream::{FrameHash, HASH_EVERY, InputChunk};

/// How far behind the newest frame received a Shadow plays.
pub const BUFFER_FRAMES: u64 = 30;
/// How far past the newest frame received a Shadow guesses before it holds:
/// longer than a hiccup, and the Player has likely paused or gone.
pub const PREDICT_FRAMES: u64 = 120;
/// How much faster than the Player a Shadow that is behind catches up.
pub const CATCH_UP_SPEED: u64 = 20;
/// How often (in real frames) a Shadow looks at the Player's Pokémon.
const LOOK_EVERY: u64 = 30;
/// How many of its own Run Checkpoints a Shadow keeps.
const CHECKPOINTS_KEPT: usize = 8;
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
pub struct ShadowGame<G: PlayedGame> {
    game: G,
    /// Which boot of the Player's game it copies.
    boot: u32,
    /// Frames run on the Player's real keys.
    confirmed: u64,
    /// Real keys received and not yet run, from frame `confirmed + predicted.len()`
    /// on, unless guesses are pending (then from `confirmed`).
    keys: VecDeque<Keys>,
    /// Changes and Run Checkpoints received and not yet reached, in order,
    /// with the boundary each comes at.
    items: VecDeque<(u64, Item)>,
    /// Its own Run Checkpoints of the Player's game, newest last.
    checkpoints: VecDeque<Kept>,
    /// The ROM's own bytes wherever a patch landed, to put back on a Retry.
    originals: BTreeMap<u32, Vec<u8>>,
    /// The Player's next boot after a Retry, waiting until this Shadow has
    /// its own checkpoint to go back to.
    next_boot: Option<NextBoot>,
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
    seen: Seen,
}

impl<G: PlayedGame> ShadowGame<G> {
    /// A Shadow of a game booted from power-on with its clock at `clock_ms`:
    /// `core` has the same ROM loaded and has run nothing.
    pub fn power_on(mut game: G, clock_ms: i64) -> Self {
        game.core_mut().set_clock(clock_ms);
        Self {
            game,
            boot: 1,
            confirmed: 0,
            keys: VecDeque::new(),
            items: VecDeque::new(),
            checkpoints: VecDeque::new(),
            originals: BTreeMap::new(),
            next_boot: None,
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
            seen: Seen::default(),
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
            self.next_boot_chunk(chunk);
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
        let mut marks = chunk.marks.iter().peekable();
        let mut seen_at = (u64::MAX, 0u16);
        for stamped in &chunk.stamped {
            let count = if seen_at.0 == stamped.frame { seen_at.1 } else { 0 };
            while let Some(mark) =
                marks.next_if(|m| m.frame < stamped.frame || (m.frame == stamped.frame && m.after <= count))
            {
                self.items.push_back((mark.frame, Item::Checkpoint));
            }
            self.items
                .push_back((stamped.frame, Item::Change(stamped.change.clone())));
            seen_at = (stamped.frame, count + 1);
        }
        for mark in marks {
            self.items.push_back((mark.frame, Item::Checkpoint));
        }
        for hash in &chunk.hashes {
            self.theirs.insert(hash.frame, *hash);
        }
        self.check_guesses();
        self.compare();
    }

    /// Plays toward where the Player is now, less the buffer, running at
    /// most `max_frames`. Returns how many frames ran.
    pub fn advance(&mut self, now_ms: i64, max_frames: u64) -> u64 {
        let mut ran = 0;
        if self.next_boot.is_some() {
            ran = self.toward_next_boot(max_frames);
            if self.next_boot.is_some() {
                return ran;
            }
        }
        let target = self.target(now_ms);
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

    /// Which boot of the Player's game it copies.
    pub fn boot(&self) -> u32 {
        self.boot
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
        self.game.core().frame()
    }

    /// The rate of the samples `drain_audio` gives.
    pub fn audio_sample_rate(&self) -> u32 {
        self.game.core().audio_sample_rate()
    }

    /// Its sound since the last call, as interleaved stereo.
    pub fn drain_audio(&mut self, out: &mut Vec<i16>) {
        self.game.core_mut().drain_audio(out);
    }

    /// The game, to read through the game interface.
    pub fn game_mut(&mut self) -> &mut G {
        &mut self.game
    }

    /// What it has seen of the Player's Pokémon so far, on real frames only.
    pub fn seen(&self) -> &Seen {
        &self.seen
    }

    /// Runs the next frame on the Player's keys: the changes stamped at
    /// this boundary land first, then the hash is taken if one is due.
    fn run_real(&mut self) {
        let boundary = self.confirmed;
        self.reach(boundary);
        if boundary.is_multiple_of(HASH_EVERY) {
            let hash = FrameHash::of(self.game.core_mut(), boundary);
            self.ours.insert(boundary, hash);
            self.compare();
        }
        let keys = self.keys.pop_front().expect("a real frame to run");
        self.game.core_mut().run_frame(keys);
        self.last_keys = keys;
        self.confirmed += 1;
        if self.confirmed.is_multiple_of(LOOK_EVERY) {
            self.look();
        }
    }

    /// Runs a frame past the newest received on a guess: the last keys held,
    /// and nothing stamped. A checkpoint is taken before the first guess.
    fn run_guess(&mut self) {
        if self.rollback.is_none() {
            self.rollback = Some(self.game.core_mut().checkpoint());
        }
        let boundary = self.frame();
        if boundary.is_multiple_of(HASH_EVERY) {
            let hash = FrameHash::of(self.game.core_mut(), boundary);
            self.guessed_hashes.insert(boundary, hash);
        }
        let keys = self.keys.back().copied().unwrap_or(self.last_keys);
        self.game.core_mut().run_frame(keys);
        self.guessed.push(keys);
    }

    /// Looks at the Player's Pokémon, as their own game would show them now.
    fn look(&mut self) {
        let in_battle = self.game.battle().is_some();
        if !in_battle && !self.game.at_safe_overworld_point() {
            return;
        }
        let dead_box = self.game.dead_box();
        let party = self.game.party();
        for pokemon in &party {
            self.seen.owned.insert(pokemon.pid);
            if pokemon.stats.is_some_and(|s| s.hp == 0) {
                self.seen.fainted.insert(pokemon.pid);
            }
        }
        for stored in self.game.pc() {
            self.seen.owned.insert(stored.pokemon.pid);
            if stored.box_number == dead_box {
                self.seen.fainted.insert(stored.pokemon.pid);
            }
        }
        self.seen.party = party
            .iter()
            .map(|p| SeenPokemon {
                pid: p.pid,
                species_name: self.game.species_name(p.species),
                nickname: p.nickname.clone(),
                hp: p.stats.map_or(0, |s| s.hp),
            })
            .collect();
    }

    /// Lands every change and takes every Run Checkpoint up to this boundary, in order.
    fn reach(&mut self, boundary: u64) {
        while let Some((frame, _)) = self.items.front()
            && *frame <= boundary
        {
            let (frame, item) = self.items.pop_front().expect("just looked");
            match item {
                // Through the game, as the Player's Run engine landed it.
                Item::Change(change) => {
                    if let Change::Patch { addr, bytes } = &change {
                        if !self.originals.contains_key(addr) {
                            let mut original = vec![0; bytes.len()];
                            self.game.core_mut().read(*addr, &mut original);
                            self.originals.insert(*addr, original);
                        }
                        self.patches.insert(*addr, bytes.clone());
                    }
                    self.game.apply(&change);
                }
                Item::Checkpoint => {
                    self.checkpoints.push_back(Kept {
                        boot: self.boot,
                        frame,
                        core: self.game.core_mut().checkpoint(),
                        patches: self.patches.clone(),
                    });
                    while self.checkpoints.len() > CHECKPOINTS_KEPT {
                        self.checkpoints.pop_front();
                    }
                }
            }
        }
    }

    /// A chunk of a later boot: the first of a boot after a Retry waits
    /// (with what follows it) until this Shadow can go back to its own
    /// checkpoint. Any other new boot is one a Shadow can't follow yet.
    fn next_boot_chunk(&mut self, chunk: &InputChunk) {
        match (&mut self.next_boot, chunk.booted_from) {
            (None, Some(from)) if chunk.first_frame == 0 => {
                self.next_boot = Some(NextBoot {
                    boot: chunk.boot,
                    from,
                    chunks: vec![chunk.clone()],
                });
            }
            (Some(next), _)
                if next.boot == chunk.boot && next.chunks.last().is_some_and(|c| c.end() == chunk.first_frame) =>
            {
                next.chunks.push(chunk.clone());
            }
            _ => self.state = ShadowState::Restarted { boot: chunk.boot },
        }
    }

    /// Plays the old boot up to the checkpoint the next boot began from,
    /// then goes back to it and takes in the next boot's chunks.
    fn toward_next_boot(&mut self, max_frames: u64) -> u64 {
        let (from_boot, from_frame) = self.next_boot.as_ref().expect("a next boot").from;
        if !self.guessed.is_empty() {
            self.drop_guesses();
        }
        let mut ran = 0;
        if from_boot == self.boot {
            while ran < max_frames && self.confirmed < from_frame && !self.keys.is_empty() {
                self.run_real();
                ran += 1;
            }
            if self.confirmed == from_frame {
                self.reach(from_frame);
            }
        }
        let Some(kept) = self
            .checkpoints
            .iter()
            .rev()
            .find(|k| (k.boot, k.frame) == (from_boot, from_frame))
            .cloned()
        else {
            let stuck = from_boot != self.boot || (self.confirmed < from_frame && self.keys.is_empty());
            if stuck || self.confirmed >= from_frame {
                let boot = self.next_boot.take().expect("a next boot").boot;
                self.state = ShadowState::Restarted { boot };
            }
            return ran;
        };
        for (addr, original) in &self.originals {
            self.game.core_mut().patch_rom(*addr, original);
        }
        if self.game.core_mut().restore(&kept.core).is_err() {
            self.next_boot = None;
            self.state = ShadowState::Diverged { frame: self.confirmed };
            return ran;
        }
        for (addr, bytes) in &kept.patches {
            self.game.core_mut().patch_rom(*addr, bytes);
        }
        let next = self.next_boot.take().expect("a next boot");
        self.boot = next.boot;
        self.patches = kept.patches;
        self.confirmed = 0;
        self.received = 0;
        self.keys.clear();
        self.items.clear();
        self.theirs.clear();
        self.ours.clear();
        // What it saw after the checkpoint is undone with it.
        self.seen = Seen::default();
        let now = self.last_arrival_ms.unwrap_or(0);
        for chunk in &next.chunks {
            self.receive(chunk, now);
        }
        ran
    }

    /// Back to the checkpoint before the guesses, keeping the real frames received.
    fn drop_guesses(&mut self) {
        let checkpoint = self.rollback.take().expect("a checkpoint before the guesses");
        if self.game.core_mut().restore(&checkpoint).is_err() {
            self.state = ShadowState::Diverged { frame: self.confirmed };
            return;
        }
        for (addr, bytes) in &self.patches {
            self.game.core_mut().patch_rom(*addr, bytes);
        }
        self.guessed.clear();
        self.guessed_hashes.clear();
        self.rewinds += 1;
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
            && !self.items.iter().any(|(frame, _)| *frame < self.confirmed + n as u64);
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
        self.drop_guesses();
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

/// What a Player's stream puts at a boundary.
enum Item {
    Change(Change),
    /// A Run Checkpoint: the Shadow takes its own here.
    Checkpoint,
}

/// A Shadow's own Run Checkpoint of the Player's game.
#[derive(Clone)]
struct Kept {
    boot: u32,
    frame: u64,
    core: Checkpoint,
    patches: BTreeMap<u32, Vec<u8>>,
}

/// The Player's boot after a Retry, before the Shadow follows it.
struct NextBoot {
    boot: u32,
    /// The boot and frame of the Run Checkpoint it began from.
    from: (u32, u64),
    chunks: Vec<InputChunk>,
}

/// What a Shadow Game has shown of its Player's Pokémon, on their real frames:
/// how their own game had them, whatever their app claims.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    /// Every PID seen in their party or PC.
    pub owned: std::collections::BTreeSet<u32>,
    /// Every PID seen at 0 HP in their party, or in their Dead box.
    pub fainted: std::collections::BTreeSet<u32>,
    /// Their party at the last look.
    pub party: Vec<SeenPokemon>,
}

/// A party Pokémon as a Shadow Game last showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenPokemon {
    pub pid: u32,
    pub species_name: String,
    pub nickname: String,
    pub hp: u16,
}
