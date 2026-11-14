//! A Player's input stream: the key mask held for every frame, every write
//! and patch the Run made stamped with its frame boundary, and the game's RAM
//! and save hashes every so often. It is all a Shadow Game needs to replay
//! the Player's game exactly, and all that crosses the wire about a game:
//! never video, savestates or saves.
//!
//! Messages are bounded binary, read without trusting them: anything
//! malformed is `None`, never a panic.

use std::collections::BTreeMap;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::Change;
use sha1::{Digest, Sha1};

use crate::Stamped;

/// A chunk is sent once it holds this many frames.
pub const CHUNK_FRAMES: usize = 15;
/// The game's RAM and save are hashed at every frame boundary that is a multiple of this.
pub const HASH_EVERY: u64 = 120;
/// No message is larger than this, so it fits the wire's 16 KiB with room to spare.
const MAX_MESSAGE: usize = 15 * 1024;
/// A resend merges chunks up to this many frames.
const MAX_CHUNK_FRAMES: usize = 4096;
/// A stamped change longer than this is sent as several, at the same boundary.
const MAX_CHANGE_BYTES: usize = 1024;
/// At most this many chunks are kept from after a gap, waiting for it to fill.
const MAX_EARLY_CHUNKS: usize = 256;

const MAGIC: [u8; 2] = *b"IS";
const VERSION: u8 = 2;
const CHUNK: u8 = 1;
const WANT: u8 = 2;

const WRITE: u8 = 0;
const PATCH: u8 = 1;

/// A Player's install key, as the Run log knows them.
pub type PlayerKey = [u8; 32];

/// The game's state at a frame boundary, hashed: its work RAM (EWRAM and
/// IWRAM) and its save memory, after every change stamped at that boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHash {
    /// The frame boundary: frames run before it.
    pub frame: u64,
    /// SHA-1 of EWRAM then IWRAM.
    pub ram: [u8; 20],
    /// SHA-1 of the save memory.
    pub save: [u8; 20],
}

impl FrameHash {
    /// Hashes `core` as it stands at boundary `frame`.
    pub fn of(core: &mut impl CoreAdapter, frame: u64) -> Self {
        let mut ram = Sha1::new();
        let mut block = vec![0u8; 0x4_0000];
        core.read(0x0200_0000, &mut block);
        ram.update(&block);
        block.truncate(0x8000);
        core.read(0x0300_0000, &mut block);
        ram.update(&block);
        Self {
            frame,
            ram: ram.finalize().into(),
            save: Sha1::digest(core.save_memory()).into(),
        }
    }

    /// Whether the two games match at this boundary.
    pub fn matches(&self, other: &FrameHash) -> bool {
        self.ram == other.ram && self.save == other.save
    }
}

/// A stretch of one Player's input stream: the keys held for frames
/// `first_frame..first_frame + keys.len()` of a boot, and the changes stamped
/// and hashes taken at the boundaries before each of those frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputChunk {
    /// Whose stream it is.
    pub player: PlayerKey,
    /// Which boot of their game, from 1: each boot's stream starts at frame 0.
    pub boot: u32,
    pub first_frame: u64,
    /// The keys held for each frame from `first_frame` on.
    pub keys: Vec<Keys>,
    /// Changes stamped at those frames' boundaries, in order.
    pub stamped: Vec<Stamped>,
    /// Hashes taken at those boundaries, in order.
    pub hashes: Vec<FrameHash>,
    /// Run Checkpoints taken at those boundaries, each placed among the
    /// changes stamped there, so a Shadow takes its own at the same point.
    pub marks: Vec<Mark>,
    /// For frame 0 of a boot after a Retry: the boot and frame of the
    /// Player's Run Checkpoint it began from.
    pub booted_from: Option<(u32, u64)>,
}

/// A Run Checkpoint in a Player's stream: taken at boundary `frame`, after
/// the first `after` changes this chunk stamps there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub frame: u64,
    pub after: u16,
}

impl InputChunk {
    /// The frame after its last.
    pub fn end(&self) -> u64 {
        self.first_frame + self.keys.len() as u64
    }

    fn encoded_len(&self) -> usize {
        53 + 2 * self.keys.len()
            + self
                .stamped
                .iter()
                .map(|s| 15 + change_bytes(&s.change).len())
                .sum::<usize>()
            + 48 * self.hashes.len()
            + 11 * self.marks.len()
            + 13
    }
}

/// Someone is missing part of a Player's stream: everything from `from_frame` on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamWant {
    pub player: PlayerKey,
    pub boot: u32,
    pub from_frame: u64,
}

/// What the input stream's network messages carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamMessage {
    Chunk(InputChunk),
    Want(StreamWant),
}

impl StreamMessage {
    /// The message as the bounded binary the wire carries.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        match self {
            StreamMessage::Chunk(chunk) => {
                out.push(CHUNK);
                out.extend_from_slice(&chunk.player);
                out.extend_from_slice(&chunk.boot.to_le_bytes());
                out.extend_from_slice(&chunk.first_frame.to_le_bytes());
                out.extend_from_slice(&(chunk.keys.len() as u16).to_le_bytes());
                for keys in &chunk.keys {
                    out.extend_from_slice(&keys.0.to_le_bytes());
                }
                out.extend_from_slice(&(chunk.stamped.len() as u16).to_le_bytes());
                for stamped in &chunk.stamped {
                    let (kind, addr, bytes) = match &stamped.change {
                        Change::Write { addr, bytes } => (WRITE, addr, bytes),
                        Change::Patch { addr, bytes } => (PATCH, addr, bytes),
                    };
                    out.extend_from_slice(&stamped.frame.to_le_bytes());
                    out.push(kind);
                    out.extend_from_slice(&addr.to_le_bytes());
                    out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                    out.extend_from_slice(bytes);
                }
                out.push(chunk.hashes.len() as u8);
                for hash in &chunk.hashes {
                    out.extend_from_slice(&hash.frame.to_le_bytes());
                    out.extend_from_slice(&hash.ram);
                    out.extend_from_slice(&hash.save);
                }
                out.push(chunk.marks.len() as u8);
                for mark in &chunk.marks {
                    out.extend_from_slice(&mark.frame.to_le_bytes());
                    out.extend_from_slice(&mark.after.to_le_bytes());
                }
                match chunk.booted_from {
                    Some((boot, frame)) => {
                        out.push(1);
                        out.extend_from_slice(&boot.to_le_bytes());
                        out.extend_from_slice(&frame.to_le_bytes());
                    }
                    None => out.push(0),
                }
            }
            StreamMessage::Want(want) => {
                out.push(WANT);
                out.extend_from_slice(&want.player);
                out.extend_from_slice(&want.boot.to_le_bytes());
                out.extend_from_slice(&want.from_frame.to_le_bytes());
            }
        }
        out
    }

    /// The message in `bytes`, if it is exactly one well-formed message.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_MESSAGE + 64 {
            return None;
        }
        let mut r = Reader(bytes);
        if r.take(2)? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let message = match r.u8()? {
            CHUNK => {
                let player = r.key()?;
                let boot = r.u32()?;
                let first_frame = r.u64()?;
                let count = usize::from(r.u16()?);
                if count > MAX_CHUNK_FRAMES {
                    return None;
                }
                let keys = (0..count).map(|_| r.u16().map(Keys)).collect::<Option<Vec<_>>>()?;
                let end = first_frame.checked_add(count as u64)?;
                let changes = usize::from(r.u16()?);
                let mut stamped: Vec<Stamped> = Vec::with_capacity(changes.min(256));
                for _ in 0..changes {
                    let frame = r.u64()?;
                    let kind = r.u8()?;
                    let addr = r.u32()?;
                    let len = usize::from(r.u16()?);
                    if len == 0 || len > MAX_CHANGE_BYTES {
                        return None;
                    }
                    let bytes = r.take(len)?.to_vec();
                    // In order, and inside the stretch.
                    let in_order = stamped.last().is_none_or(|s| s.frame <= frame);
                    if !in_order || frame < first_frame || frame >= end {
                        return None;
                    }
                    let change = match kind {
                        WRITE => Change::Write { addr, bytes },
                        PATCH => Change::Patch { addr, bytes },
                        _ => return None,
                    };
                    stamped.push(Stamped { frame, change });
                }
                let count = usize::from(r.u8()?);
                let mut hashes: Vec<FrameHash> = Vec::with_capacity(count);
                for _ in 0..count {
                    let frame = r.u64()?;
                    let ram = r.take(20)?.try_into().ok()?;
                    let save = r.take(20)?.try_into().ok()?;
                    let in_order = hashes.last().is_none_or(|h| h.frame < frame);
                    if !in_order || frame < first_frame || frame >= end {
                        return None;
                    }
                    hashes.push(FrameHash { frame, ram, save });
                }
                let count = usize::from(r.u8()?);
                let mut marks: Vec<Mark> = Vec::with_capacity(count);
                for _ in 0..count {
                    let frame = r.u64()?;
                    let after = r.u16()?;
                    let here = stamped.iter().filter(|s| s.frame == frame).count();
                    let in_order = marks.last().is_none_or(|m| (m.frame, m.after) <= (frame, after));
                    if !in_order || frame < first_frame || frame >= end || usize::from(after) > here {
                        return None;
                    }
                    marks.push(Mark { frame, after });
                }
                let booted_from = match r.u8()? {
                    0 => None,
                    1 if first_frame == 0 => Some((r.u32()?, r.u64()?)),
                    _ => return None,
                };
                StreamMessage::Chunk(InputChunk {
                    player,
                    boot,
                    first_frame,
                    keys,
                    stamped,
                    hashes,
                    marks,
                    booted_from,
                })
            }
            WANT => StreamMessage::Want(StreamWant {
                player: r.key()?,
                boot: r.u32()?,
                from_frame: r.u64()?,
            }),
            _ => return None,
        };
        r.0.is_empty().then_some(message)
    }
}

fn change_bytes(change: &Change) -> &[u8] {
    match change {
        Change::Write { bytes, .. } | Change::Patch { bytes, .. } => bytes,
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn key(&mut self) -> Option<PlayerKey> {
        self.take(32)?.try_into().ok()
    }
}

/// Builds the local Player's input stream as their game runs, one chunk at a time.
#[derive(Debug, Clone)]
pub struct StreamOut {
    open: InputChunk,
}

impl StreamOut {
    /// `player`'s stream for this boot of their game, from frame 0.
    pub fn new(player: PlayerKey, boot: u32) -> Self {
        Self {
            open: InputChunk {
                player,
                boot,
                first_frame: 0,
                keys: Vec::new(),
                stamped: Vec::new(),
                hashes: Vec::new(),
                marks: Vec::new(),
                booted_from: None,
            },
        }
    }

    /// `player`'s stream going on from `at` in their game: a Run with
    /// friends resumed where the Player left it.
    pub fn going_on_from(player: PlayerKey, at: crate::GamePoint) -> Self {
        let mut out = Self::new(player, at.boot);
        out.open.first_frame = at.frame;
        out
    }

    /// The chunks a stretch of a boot's stream, as the Run file keeps it,
    /// would have been sent in (without the hashes, which it doesn't keep).
    /// Changes stamped at the boundary after its last frame are left for
    /// the stream that goes on from there.
    pub fn chunks_of(player: PlayerKey, batches: &[crate::InputBatch]) -> Vec<InputChunk> {
        let Some(first) = batches.first() else {
            return Vec::new();
        };
        let mut out = Self::going_on_from(
            player,
            crate::GamePoint {
                boot: first.boot,
                frame: first.first_frame,
            },
        );
        let mut chunks = Vec::new();
        for batch in batches {
            for (n, &keys) in batch.keys.iter().enumerate() {
                let boundary = batch.first_frame + n as u64;
                let stamped: Vec<Stamped> = batch.stamped.iter().filter(|s| s.frame == boundary).cloned().collect();
                chunks.extend(out.stamp(&stamped));
                chunks.extend(out.frame(keys, &[]));
            }
        }
        if !out.open.keys.is_empty() {
            chunks.push(out.close());
        }
        chunks
    }

    /// A boot after a Retry, from the Player's Run Checkpoint taken at this boot and frame.
    pub fn after_retry(player: PlayerKey, boot: u32, from: (u32, u64)) -> Self {
        let mut out = Self::new(player, boot);
        out.open.booted_from = Some(from);
        out
    }

    /// Whose stream it is.
    pub fn player(&self) -> PlayerKey {
        self.open.player
    }

    /// A Run Checkpoint was taken here, after every change stamped so far.
    pub fn mark_checkpoint(&mut self, frame: u64) {
        let after = self.open.stamped.iter().filter(|s| s.frame == frame).count();
        self.open.marks.push(Mark {
            frame,
            after: u16::try_from(after).unwrap_or(u16::MAX),
        });
    }

    /// The hash taken at the boundary before this frame ran, if one was due.
    pub fn hash(&mut self, hash: FrameHash) {
        self.open.hashes.push(hash);
    }

    /// A frame ran with `keys`, and `stamped` were stamped at the boundary
    /// after it. Returns a chunk once one is full; changes at the boundary
    /// after its last frame go with the next.
    pub fn frame(&mut self, keys: Keys, stamped: &[Stamped]) -> Option<InputChunk> {
        self.open.keys.push(keys);
        let full = self.open.keys.len() >= CHUNK_FRAMES || self.open.encoded_len() > MAX_MESSAGE / 2;
        let chunk = full.then(|| self.close());
        // The open chunk was just closed if full, so stamping can't close it again.
        let early = self.stamp(stamped);
        chunk.or(early)
    }

    /// Changes stamped at the boundary before the next frame. If they would
    /// make the open chunk too big to send, it is closed first, so they
    /// start the next one with the frame they come before.
    pub fn stamp(&mut self, stamped: &[Stamped]) -> Option<InputChunk> {
        let pieces: Vec<Stamped> = stamped
            .iter()
            .flat_map(|s| {
                split(&s.change)
                    .into_iter()
                    .map(|change| Stamped { frame: s.frame, change })
            })
            .collect();
        let adding: usize = pieces.iter().map(|s| 15 + change_bytes(&s.change).len()).sum();
        let closed =
            (!self.open.keys.is_empty() && self.open.encoded_len() + adding > MAX_MESSAGE / 2).then(|| self.close());
        self.open.stamped.extend(pieces);
        closed
    }

    /// The chunk so far, even if not full; the next starts after it.
    /// What is at the boundary after its last frame (changes, a checkpoint)
    /// goes on with the next chunk, with the frame it comes before.
    fn close(&mut self) -> InputChunk {
        let end = self.open.end();
        let carried = self
            .open
            .stamped
            .iter()
            .position(|s| s.frame >= end)
            .unwrap_or(self.open.stamped.len());
        let next = InputChunk {
            player: self.open.player,
            boot: self.open.boot,
            first_frame: end,
            keys: Vec::new(),
            stamped: self.open.stamped.split_off(carried),
            hashes: Vec::new(),
            marks: self.open.marks.extract_if(.., |m| m.frame >= end).collect(),
            booted_from: None,
        };
        std::mem::replace(&mut self.open, next)
    }
}

/// A change in pieces of at most [`MAX_CHANGE_BYTES`], applied in order at
/// the same boundary: the same change.
fn split(change: &Change) -> Vec<Change> {
    let (addr, bytes, patch) = match change {
        Change::Write { addr, bytes } => (*addr, bytes, false),
        Change::Patch { addr, bytes } => (*addr, bytes, true),
    };
    bytes
        .chunks(MAX_CHANGE_BYTES)
        .enumerate()
        .map(|(i, piece)| {
            let addr = addr.wrapping_add((i * MAX_CHANGE_BYTES) as u32);
            let bytes = piece.to_vec();
            if patch {
                Change::Patch { addr, bytes }
            } else {
                Change::Write { addr, bytes }
            }
        })
        .collect()
}

/// What happened to a chunk offered to a [`StreamStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// Chunks now in order that weren't before, to replay and pass on.
    New(Vec<InputChunk>),
    /// It came after a gap: what to ask for. It is kept until the gap fills.
    Gap(StreamWant),
    /// Already here, or from an older boot.
    Old,
    /// It differs from frames already here: refused, as rewritten history.
    Rewritten(Rewrite),
}

/// A Player's stream that rewrote a frame already here: the first that differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rewrite {
    /// Whose stream.
    pub player: PlayerKey,
    /// The boot of their game, and the first frame that differs.
    pub at: crate::GamePoint,
}

/// What came of a stream message: what to send on, the chunks now in order
/// for a Shadow Game, and a rewrite of frames already here, if it was one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Received {
    /// Messages to send on.
    pub send: Vec<Outgoing>,
    /// Chunks now in order, for a Shadow Game.
    pub new: Vec<InputChunk>,
    /// The chunk rewrote frames already here, and was refused.
    pub rewritten: Option<Rewrite>,
}

/// Every Player's input stream as this app has it, in order from frame 0
/// of their current boot, so a gap can be asked for and a request answered.
#[derive(Debug, Clone, Default)]
pub struct StreamStore {
    players: BTreeMap<PlayerKey, PlayerStream>,
}

#[derive(Debug, Clone, Default)]
struct PlayerStream {
    boot: u32,
    /// Where the stream starts here: 0, or a Shadow's checkpoint after a resume.
    start: u64,
    /// In order from `start`, with no gap.
    chunks: Vec<InputChunk>,
    /// Arrived after a gap, by first frame.
    early: BTreeMap<u64, InputChunk>,
}

impl PlayerStream {
    fn end(&self) -> u64 {
        self.chunks.last().map_or(self.start, InputChunk::end)
    }

    /// The keys held at this frame, if it is still here.
    fn keys_at(&self, frame: u64) -> Option<Keys> {
        let i = self.chunks.partition_point(|c| c.end() <= frame);
        let chunk = self.chunks.get(i).filter(|c| c.first_frame <= frame)?;
        chunk
            .keys
            .get(usize::try_from(frame - chunk.first_frame).ok()?)
            .copied()
    }

    /// The first frame this chunk holds other keys for than this stream has.
    fn first_difference(&self, chunk: &InputChunk) -> Option<u64> {
        (chunk.first_frame..chunk.end().min(self.end())).find(|&frame| {
            self.keys_at(frame)
                .is_some_and(|keys| keys != chunk.keys[(frame - chunk.first_frame) as usize])
        })
    }
}

impl StreamStore {
    /// No stream yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes in a chunk of a Player's stream. A newer boot starts their stream afresh.
    pub fn add(&mut self, mut chunk: InputChunk) -> Added {
        let stream = self.players.entry(chunk.player).or_default();
        if chunk.boot < stream.boot {
            return Added::Old;
        }
        if chunk.boot > stream.boot {
            *stream = PlayerStream {
                boot: chunk.boot,
                ..PlayerStream::default()
            };
        }
        let end = stream.end();
        if let Some(frame) = stream.first_difference(&chunk) {
            return Added::Rewritten(Rewrite {
                player: chunk.player,
                at: crate::GamePoint {
                    boot: chunk.boot,
                    frame,
                },
            });
        }
        if chunk.keys.is_empty() || chunk.end() <= end {
            return Added::Old;
        }
        if chunk.first_frame > end {
            let want = StreamWant {
                player: chunk.player,
                boot: chunk.boot,
                from_frame: end,
            };
            // Kept for when the gap fills, but only so many: a peer can't fill memory.
            if stream.early.len() < MAX_EARLY_CHUNKS {
                stream.early.insert(chunk.first_frame, chunk);
            }
            return Added::Gap(want);
        }
        if chunk.first_frame < end {
            trim_front(&mut chunk, end);
        }
        let mut new = vec![chunk];
        loop {
            let end = new.last().map_or(0, InputChunk::end);
            let Some((&first, _)) = stream.early.range(..=end).next_back() else {
                break;
            };
            let mut next = stream.early.remove(&first).expect("just found");
            if next.end() <= end {
                continue;
            }
            trim_front(&mut next, end);
            new.push(next);
        }
        stream
            .early
            .retain(|_, c| c.end() > new.last().map_or(0, InputChunk::end));
        stream.chunks.extend(new.iter().cloned());
        Added::New(new)
    }

    /// A Player's stream here starts at `at` in their game: a Shadow Game
    /// restored from its checkpoint needs nothing from before it.
    pub fn starts_at(&mut self, player: PlayerKey, at: crate::GamePoint) {
        self.players.insert(
            player,
            PlayerStream {
                boot: at.boot,
                start: at.frame,
                ..PlayerStream::default()
            },
        );
    }

    /// What a request asks for, merged into as few chunks as fit a message.
    pub fn wanted(&self, want: &StreamWant) -> Vec<InputChunk> {
        let Some(stream) = self.players.get(&want.player).filter(|s| s.boot == want.boot) else {
            return Vec::new();
        };
        let mut out: Vec<InputChunk> = Vec::new();
        for chunk in stream.chunks.iter().filter(|c| c.end() > want.from_frame) {
            let mut chunk = chunk.clone();
            trim_front(&mut chunk, want.from_frame);
            match out.last_mut() {
                Some(last)
                    if last.keys.len() + chunk.keys.len() <= MAX_CHUNK_FRAMES
                        && last.hashes.len() + chunk.hashes.len() <= usize::from(u8::MAX)
                        && last.stamped.len() + chunk.stamped.len() <= usize::from(u16::MAX)
                        && last.marks.len() + chunk.marks.len() <= usize::from(u8::MAX)
                        && last.encoded_len() + chunk.encoded_len() <= MAX_MESSAGE =>
                {
                    last.keys.extend(chunk.keys);
                    last.stamped.extend(chunk.stamped);
                    last.hashes.extend(chunk.hashes);
                    last.marks.extend(chunk.marks);
                }
                _ => out.push(chunk),
            }
        }
        out
    }

    /// How far a Player's stream is here, in order: their boot and the frame after the last.
    pub fn received(&self, player: &PlayerKey) -> Option<(u32, u64)> {
        self.players.get(player).map(|s| (s.boot, s.end()))
    }
}

/// Drops the frames before `from` (the changes stamped before it and the
/// hashes taken before it went with them).
fn trim_front(chunk: &mut InputChunk, from: u64) {
    if from <= chunk.first_frame {
        return;
    }
    let skip = (from - chunk.first_frame).min(chunk.keys.len() as u64);
    chunk.keys.drain(..skip as usize);
    chunk.first_frame += skip;
    chunk.stamped.retain(|s| s.frame >= chunk.first_frame);
    chunk.hashes.retain(|h| h.frame >= chunk.first_frame);
    chunk.marks.retain(|m| m.frame >= chunk.first_frame);
    chunk.booted_from = None;
}

/// Where a stream message goes. Joiners talk only to the Host, who passes
/// each Player's stream on to everyone else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    To(PlayerKey, StreamMessage),
    /// To every Player in the Run but this one.
    AllBut(PlayerKey, StreamMessage),
}

/// One app's side of the input streams: its own, and every other Player's
/// as it arrives, kept in order. It says what to send where, and which
/// chunks are new for the Shadow Games to replay.
#[derive(Debug, Clone)]
pub struct StreamHub {
    me: PlayerKey,
    host: PlayerKey,
    store: StreamStore,
    /// The gap last asked for in each stream, and how many chunks have
    /// arrived after it since.
    asked: BTreeMap<PlayerKey, (StreamWant, u64)>,
}

/// Chunks that may arrive after a gap before it is asked for again: about
/// four seconds of a stream at 4x. The answer to one ask is the whole gap.
const REASK_CHUNKS: u64 = 64;

impl StreamHub {
    /// The streams as the app of `me` sees them, in a Run that `host` hosts.
    pub fn new(me: PlayerKey, host: PlayerKey) -> Self {
        Self {
            me,
            host,
            store: StreamStore::new(),
            asked: BTreeMap::new(),
        }
    }

    fn hosting(&self) -> bool {
        self.me == self.host
    }

    /// A chunk of this Player's own stream: the Host sends it to everyone,
    /// a joiner to the Host.
    pub fn own(&mut self, chunk: InputChunk) -> Vec<Outgoing> {
        let Added::New(new) = self.store.add(chunk) else {
            return Vec::new();
        };
        new.into_iter()
            .map(|chunk| {
                let message = StreamMessage::Chunk(chunk);
                if self.hosting() {
                    Outgoing::AllBut(self.me, message)
                } else {
                    Outgoing::To(self.host, message)
                }
            })
            .collect()
    }

    /// Another Player's stream here starts at `at` in their game, where
    /// this app's Shadow Game of them was restored from its checkpoint.
    pub fn starts_at(&mut self, player: PlayerKey, at: crate::GamePoint) {
        self.store.starts_at(player, at);
    }

    /// Chunks of this Player's own stream sent before this app started (a
    /// Run with friends resumed): kept to answer requests, not sent again.
    pub fn already_sent(&mut self, chunks: Vec<InputChunk>) {
        for chunk in chunks {
            let _ = self.store.add(chunk);
        }
    }

    /// A stream message from `from`. The Host takes a Player's stream only
    /// from that Player; a joiner takes streams only from the Host. A chunk
    /// that rewrites frames already here is refused, not passed on.
    pub fn receive(&mut self, from: PlayerKey, message: StreamMessage) -> Received {
        match message {
            StreamMessage::Chunk(chunk) => {
                let trusted = if self.hosting() {
                    chunk.player == from
                } else {
                    from == self.host
                };
                if !trusted || chunk.player == self.me {
                    return Received::default();
                }
                let source = chunk.player;
                match self.store.add(chunk) {
                    Added::New(new) => {
                        let out = if self.hosting() {
                            new.iter()
                                .map(|c| Outgoing::AllBut(source, StreamMessage::Chunk(c.clone())))
                                .collect()
                        } else {
                            Vec::new()
                        };
                        Received {
                            send: out,
                            new,
                            rewritten: None,
                        }
                    }
                    // Ask whoever sends this stream here: its Player, or the Host,
                    // once for each gap, and again only if it is still there a while later.
                    Added::Gap(want) => {
                        let asked = self.asked.entry(want.player).or_insert((want, REASK_CHUNKS));
                        if asked.0 == want && asked.1 < REASK_CHUNKS {
                            asked.1 += 1;
                            return Received::default();
                        }
                        *asked = (want, 1);
                        Received {
                            send: vec![Outgoing::To(from, StreamMessage::Want(want))],
                            ..Received::default()
                        }
                    }
                    Added::Old => Received::default(),
                    Added::Rewritten(rewrite) => Received {
                        rewritten: Some(rewrite),
                        ..Received::default()
                    },
                }
            }
            StreamMessage::Want(want) => {
                if want.player != self.me && !self.hosting() {
                    return Received::default();
                }
                let send = self
                    .store
                    .wanted(&want)
                    .into_iter()
                    .map(|chunk| Outgoing::To(from, StreamMessage::Chunk(chunk)))
                    .collect();
                Received {
                    send,
                    ..Received::default()
                }
            }
        }
    }

    /// How far a Player's stream is here: their boot and the frame after the last.
    pub fn received(&self, player: &PlayerKey) -> Option<(u32, u64)> {
        self.store.received(player)
    }

    /// Another Player took over hosting: streams go through them from now on.
    pub fn host_is(&mut self, host: PlayerKey) {
        self.host = host;
    }

    /// The Host this hub sends through.
    pub fn host(&self) -> PlayerKey {
        self.host
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(player: PlayerKey, first_frame: u64) -> InputChunk {
        InputChunk {
            player,
            boot: 1,
            first_frame,
            keys: vec![Keys::NONE; CHUNK_FRAMES],
            stamped: Vec::new(),
            hashes: Vec::new(),
            marks: Vec::new(),
            booted_from: None,
        }
    }

    /// How many Wants `chunks` more of Blue's stream make the Host send.
    fn wants(hub: &mut StreamHub, blue: PlayerKey, chunks: std::ops::Range<u64>) -> usize {
        chunks
            .flat_map(|i| {
                hub.receive(blue, StreamMessage::Chunk(chunk(blue, i * CHUNK_FRAMES as u64)))
                    .send
            })
            .filter(|out| matches!(out, Outgoing::To(_, StreamMessage::Want(_))))
            .count()
    }

    /// A Player back after a while offline sends their live chunks at once,
    /// each one after the gap. Asking for the gap with every one of them made
    /// the Player resend the whole gap again and again, a flood the Host's
    /// rate limit closed them for.
    #[test]
    fn a_gap_is_asked_for_once_not_with_every_chunk_after_it_and_again_only_after_a_while() {
        let (host, blue) = ([1; 32], [2; 32]);
        let mut hub = StreamHub::new(host, host);
        assert_eq!(wants(&mut hub, blue, 0..1), 0, "the start of the stream");
        // Chunks 1 to 9 are lost; 10 on arrive.
        assert_eq!(wants(&mut hub, blue, 10..10 + REASK_CHUNKS), 1, "asked once");
        assert_eq!(
            wants(&mut hub, blue, 10 + REASK_CHUNKS..11 + REASK_CHUNKS),
            1,
            "asked again after a while"
        );
    }
}
