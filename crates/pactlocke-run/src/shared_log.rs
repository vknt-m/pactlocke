//! The Run log every Player shares: Host-ordered, Player-signed and
//! hash-linked, holding Run state only. Each entry is signed by its author
//! (the Player it is about) with their install's key; the Host numbers it,
//! links it to the entry before by hash and countersigns that. So every
//! Player can check the whole log and recompute the tallies, and the Host
//! can only order entries: changing one breaks its author's signature, and
//! inventing one needs a key the Host doesn't have.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CheckpointReason, Locked, RunEvent};

/// What an author signs before their counter and the entry: the Run, so an
/// entry can't be replayed into another Run.
const DOMAIN: &[u8] = b"pactlocke run log v1";

/// A Player's signing key: their install's ed25519 key, the same one that
/// identifies their app on the network.
pub struct Signer {
    key: SigningKey,
}

impl Signer {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self {
            key: SigningKey::from_bytes(&secret),
        }
    }

    /// The public key everyone checks this Player's entries with.
    pub fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.key.sign(message).to_bytes().to_vec()
    }
}

/// What an entry says. Its kind sits beside the content, which for an
/// event has a `kind` of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum LogBody {
    /// The first entry, by the Host: which Run this is. The nonce makes every
    /// Run's log its own.
    Genesis {
        nonce: [u8; 16],
        run_name: String,
        game_name: String,
        game_sha1: String,
    },
    /// By the Host: the Ruleset and roster are locked.
    Locked(Locked),
    /// Something that happened in its author's game.
    Event(RunEvent),
    /// By the Host, once everyone voted Yes: a Retry. Every Player goes back
    /// to their part of the `call`th Run Checkpoint, and each Player's
    /// entries after it are void.
    Retry { call: u64 },
}

/// An entry as its author signed it, before the Host orders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submission {
    pub author: [u8; 32],
    /// The author's own count of their entries, from 1: each is used once.
    pub counter: u64,
    /// The body, as JSON, exactly as signed.
    pub body: Vec<u8>,
    pub signature: Vec<u8>,
}

impl Submission {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a submission serialises")
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// An entry in the log: a submission the Host numbered and linked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Its place in the log, from 0.
    pub seq: u64,
    /// The hash of the entry before (zeros for the first).
    pub prev: [u8; 32],
    pub submission: Submission,
    /// The Host's signature on this entry's hash.
    pub host_signature: Vec<u8>,
}

impl LogEntry {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("an entry serialises")
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    /// What the next entry links to.
    fn hash(&self) -> [u8; 32] {
        let s = &self.submission;
        let mut sha = Sha256::new();
        sha.update(self.prev);
        sha.update(self.seq.to_le_bytes());
        sha.update(s.author);
        sha.update(s.counter.to_le_bytes());
        sha.update((s.body.len() as u64).to_le_bytes());
        sha.update(&s.body);
        sha.update(&s.signature);
        sha.finalize().into()
    }
}

/// Why a log, or an entry offered to it, isn't valid. Each names the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    /// Entries out of order, missing or repeated.
    Seq { expected: u64, found: u64 },
    /// It doesn't link to the entry before it.
    Link { seq: u64 },
    /// The Host didn't sign it as ordered.
    HostSignature { seq: u64 },
    /// Its author didn't sign it as it reads: it was changed or made up.
    AuthorSignature { seq: u64 },
    /// Only the Host may write this kind of entry.
    NotHost { seq: u64 },
    /// Its author isn't in the roster.
    NotInRoster { seq: u64 },
    /// Its author already used this counter.
    Replayed { seq: u64 },
    /// Its author's entry before it is missing: the Host left one out.
    Skipped { seq: u64 },
    /// Its body isn't an entry this build can read.
    Body { seq: u64 },
    /// The log doesn't start with the Run's genesis entry.
    NoGenesis,
    /// The roster was locked twice.
    LockedAgain { seq: u64 },
    /// A Retry without everyone's vote, with no Retry left, or to a Run
    /// Checkpoint that isn't the latest every Player took.
    BadRetry { seq: u64 },
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::Seq { expected, found } => write!(f, "entry {found} where entry {expected} should be"),
            LogError::Link { seq } => write!(f, "entry {seq} doesn't follow the entry before it"),
            LogError::HostSignature { seq } => write!(f, "entry {seq} wasn't ordered by the Host"),
            LogError::AuthorSignature { seq } => write!(f, "entry {seq} isn't what its Player signed"),
            LogError::NotHost { seq } => write!(f, "entry {seq} can only come from the Host"),
            LogError::NotInRoster { seq } => write!(f, "entry {seq} is from someone outside the roster"),
            LogError::Replayed { seq } => write!(f, "entry {seq} repeats one its Player already sent"),
            LogError::Skipped { seq } => write!(f, "entry {seq} comes after one of its Player's that is missing"),
            LogError::Body { seq } => write!(f, "entry {seq} can't be read"),
            LogError::NoGenesis => write!(f, "the log doesn't start with the Run"),
            LogError::LockedAgain { seq } => write!(f, "entry {seq} locks the Run a second time"),
            LogError::BadRetry { seq } => write!(
                f,
                "entry {seq} is a Retry nobody agreed to: not everyone voted Yes, no Retry is left, or it isn't the latest Run Checkpoint"
            ),
        }
    }
}

impl std::error::Error for LogError {}

/// Counts every Player recomputes from the log, by Player.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tallies {
    pub encounters: BTreeMap<[u8; 32], u32>,
    /// Encounters caught: Gifts and trades aren't catches.
    pub catches: BTreeMap<[u8; 32], u32>,
    pub deaths: BTreeMap<[u8; 32], u32>,
    pub violations: BTreeMap<[u8; 32], u32>,
}

/// One copy of the Run log: the Host's, or a Player's, each checked entry by entry.
#[derive(Debug, Clone)]
pub struct RunLog {
    host: [u8; 32],
    run_id: [u8; 32],
    entries: Vec<LogEntry>,
    roster: Option<Vec<[u8; 32]>>,
    counters: BTreeMap<[u8; 32], u64>,
}

impl RunLog {
    /// An empty log for the Run the Host with this key hosts.
    pub fn new(host: [u8; 32]) -> Self {
        Self {
            host,
            run_id: [0; 32],
            entries: Vec::new(),
            roster: None,
            counters: BTreeMap::new(),
        }
    }

    /// Checks a whole log, entry by entry, as a Player who joins or resumes does.
    pub fn verify(host: [u8; 32], entries: impl IntoIterator<Item = LogEntry>) -> Result<Self, LogError> {
        let mut log = Self::new(host);
        for entry in entries {
            log.accept(entry)?;
        }
        if log.entries.is_empty() {
            return Err(LogError::NoGenesis);
        }
        Ok(log)
    }

    /// An author's entry, signed for this Run. `counter` is the author's
    /// next, from 1.
    pub fn sign(&self, signer: &Signer, counter: u64, body: &LogBody) -> Submission {
        let body = serde_json::to_vec(body).expect("a body serialises");
        let run_id = if matches!(body_kind(&body), Some(Kind::Genesis)) {
            [0; 32]
        } else {
            self.run_id
        };
        let signature = signer.sign(&author_message(&run_id, counter, &body));
        Submission {
            author: signer.public(),
            counter,
            body,
            signature,
        }
    }

    /// The Host orders a submission: numbers it, links it and countersigns,
    /// after checking it as every Player will.
    pub fn order(&mut self, host: &Signer, submission: Submission) -> Result<LogEntry, LogError> {
        let mut entry = LogEntry {
            seq: self.entries.len() as u64,
            prev: self.head(),
            submission,
            host_signature: Vec::new(),
        };
        entry.host_signature = host.sign(&entry.hash());
        self.accept(entry.clone())?;
        Ok(entry)
    }

    /// Takes in the next entry, if it is valid where it stands.
    pub fn accept(&mut self, entry: LogEntry) -> Result<(), LogError> {
        let seq = self.entries.len() as u64;
        if entry.seq != seq {
            return Err(LogError::Seq {
                expected: seq,
                found: entry.seq,
            });
        }
        if entry.prev != self.head() {
            return Err(LogError::Link { seq });
        }
        if !verifies(&self.host, &entry.hash(), &entry.host_signature) {
            return Err(LogError::HostSignature { seq });
        }
        let s = &entry.submission;
        let kind = body_kind(&s.body).ok_or(LogError::Body { seq })?;
        let run_id = if seq == 0 { [0; 32] } else { self.run_id };
        if !verifies(&s.author, &author_message(&run_id, s.counter, &s.body), &s.signature) {
            return Err(LogError::AuthorSignature { seq });
        }
        match kind {
            Kind::Genesis if seq == 0 && s.author == self.host => {}
            Kind::Genesis => {
                return Err(if seq == 0 {
                    LogError::NotHost { seq }
                } else {
                    LogError::NoGenesis
                });
            }
            _ if seq == 0 => return Err(LogError::NoGenesis),
            Kind::Locked if s.author != self.host => return Err(LogError::NotHost { seq }),
            Kind::Locked if self.roster.is_some() => return Err(LogError::LockedAgain { seq }),
            Kind::Locked => {}
            Kind::Retry if s.author != self.host => return Err(LogError::NotHost { seq }),
            Kind::Retry => {
                let Ok(LogBody::Retry { call }) = serde_json::from_slice(&s.body) else {
                    return Err(LogError::Body { seq });
                };
                if self.retry_due() != Some(call) {
                    return Err(LogError::BadRetry { seq });
                }
            }
            Kind::Event => {
                let in_roster = self.roster.as_ref().is_some_and(|r| r.contains(&s.author));
                if !in_roster {
                    return Err(LogError::NotInRoster { seq });
                }
            }
        }
        // Each author counts their entries 1, 2, 3…: an old number is a
        // replay, and a gap is an entry the Host left out.
        let next = self.next_counter(&s.author);
        if s.counter < next {
            return Err(LogError::Replayed { seq });
        }
        if s.counter > next {
            return Err(LogError::Skipped { seq });
        }
        let body: LogBody = serde_json::from_slice(&s.body).map_err(|_| LogError::Body { seq })?;
        if let LogBody::Locked(locked) = &body {
            self.roster = Some(locked.roster.iter().map(|r| r.key).collect());
        }
        if seq == 0 {
            self.run_id = Sha256::digest(&s.body).into();
        }
        self.counters.insert(s.author, s.counter);
        self.entries.push(entry);
        Ok(())
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// The next counter for an author's entry.
    pub fn next_counter(&self, author: &[u8; 32]) -> u64 {
        self.counters.get(author).map_or(1, |c| c + 1)
    }

    /// Every entry's author and body, in order.
    pub fn bodies(&self) -> impl Iterator<Item = ([u8; 32], LogBody)> + '_ {
        self.entries.iter().filter_map(|e| {
            let body = serde_json::from_slice(&e.submission.body).ok()?;
            Some((e.submission.author, body))
        })
    }

    /// The lock, once it is in the log.
    pub fn locked(&self) -> Option<Locked> {
        self.bodies().find_map(|(_, body)| match body {
            LogBody::Locked(locked) => Some(locked),
            _ => None,
        })
    }

    /// Every Run Checkpoint in order, from the 1st: who called it (their
    /// in-game save, or a Sync Gate) and each Player's part, the checkpoint
    /// of their own game they took for it. A Retry voids every Run
    /// Checkpoint after the one it went back to; they keep their numbers.
    pub fn calls(&self) -> Vec<Call> {
        let mut calls: Vec<Call> = Vec::new();
        for (index, (author, body)) in self.bodies().enumerate() {
            match body {
                LogBody::Event(RunEvent::Checkpoint {
                    reason,
                    hash,
                    boot,
                    frame,
                }) => {
                    let part = Part {
                        entry: index,
                        at: GamePoint { boot, frame },
                        hash,
                    };
                    match reason {
                        CheckpointReason::Boot => {}
                        CheckpointReason::Joined { call } => {
                            if let Some(called) = numbered_mut(&mut calls, call).filter(|c| !c.void) {
                                called.parts.entry(author).or_insert(part);
                            }
                        }
                        CheckpointReason::InGameSave | CheckpointReason::Asked => calls.push(Call {
                            n: calls.len() as u64 + 1,
                            by: author,
                            parts: BTreeMap::from([(author, part)]),
                            void: false,
                        }),
                    }
                }
                LogBody::Retry { call } => {
                    for later in calls.iter_mut().filter(|c| c.n > call) {
                        later.void = true;
                    }
                }
                _ => {}
            }
        }
        calls
    }

    /// Whether every Player in the roster has their part of this Run Checkpoint, and it isn't void.
    fn is_complete(&self, called: &Call) -> bool {
        !called.void
            && self
                .roster
                .as_ref()
                .is_some_and(|roster| roster.iter().all(|p| called.parts.contains_key(p)))
    }

    /// The newest Run Checkpoint every Player has their part of: the one a Retry goes back to.
    pub fn latest_complete(&self) -> Option<u64> {
        self.calls().iter().rev().find(|c| self.is_complete(c)).map(|c| c.n)
    }

    /// Retries left in the Run's shared pool.
    pub fn retries_left(&self) -> u8 {
        let pool = self.locked().map_or(0, |l| l.ruleset.retries);
        let spent = self
            .bodies()
            .filter(|(_, b)| matches!(b, LogBody::Retry { .. }))
            .count();
        pool.saturating_sub(u8::try_from(spent).unwrap_or(u8::MAX))
    }

    /// The Retry vote open now, if any: its Run Checkpoint and who said Yes.
    /// A vote starts at a Yes with none open, for the latest complete Run
    /// Checkpoint. It ends at the first No, at the Retry, or once a newer
    /// Run Checkpoint is complete.
    pub fn vote(&self) -> Option<Vote> {
        let mut open: Option<Vote> = None;
        for (author, body) in self.bodies() {
            match body {
                LogBody::Event(RunEvent::RetryVote { call, yes }) => match &mut open {
                    None if yes => {
                        open = Some(Vote {
                            call,
                            yes: BTreeSet::from([author]),
                        });
                    }
                    Some(vote) if vote.call == call && yes => {
                        vote.yes.insert(author);
                    }
                    Some(vote) if vote.call == call => open = None,
                    _ => {}
                },
                LogBody::Retry { .. } => open = None,
                _ => {}
            }
        }
        open.filter(|vote| Some(vote.call) == self.latest_complete())
    }

    /// The Run Checkpoint a Retry should go back to now: the open vote's,
    /// once every Player said Yes and a Retry is left. The Host records it;
    /// every Player checks it.
    pub fn retry_due(&self) -> Option<u64> {
        let vote = self.vote()?;
        let roster = self.roster.as_ref()?;
        let everyone = roster.iter().all(|p| vote.yes.contains(p));
        (everyone && self.retries_left() > 0).then_some(vote.call)
    }

    /// The tallies, recomputed from the entries. A Retry voids each
    /// Player's entries after their part of its Run Checkpoint, up to the
    /// entry that says their game went back (any sent from before it on the
    /// way are void too).
    pub fn tallies(&self) -> Tallies {
        let mut standing: Vec<(usize, PlayerKey, RunEvent)> = Vec::new();
        let mut going_back: BTreeSet<PlayerKey> = BTreeSet::new();
        let mut calls: Option<Vec<Call>> = None;
        for (index, (author, body)) in self.bodies().enumerate() {
            match body {
                LogBody::Event(RunEvent::Retried { .. }) => {
                    going_back.remove(&author);
                }
                LogBody::Event(_) if going_back.contains(&author) => {}
                LogBody::Event(event) => standing.push((index, author, event)),
                LogBody::Retry { call } => {
                    let calls = calls.get_or_insert_with(|| self.calls());
                    if let Some(called) = numbered(calls, call) {
                        standing.retain(|(at, who, _)| called.parts.get(who).is_none_or(|part| *at <= part.entry));
                        going_back.extend(called.parts.keys());
                    }
                }
                _ => {}
            }
        }
        let mut tallies = Tallies::default();
        for (_, author, event) in standing {
            let counter = match event {
                RunEvent::Encounter { .. } | RunEvent::StaticEncounter { .. } => &mut tallies.encounters,
                RunEvent::Caught { .. } => &mut tallies.catches,
                RunEvent::Died { .. } => &mut tallies.deaths,
                event if event.is_violation() => &mut tallies.violations,
                _ => continue,
            };
            *counter.entry(author).or_default() += 1;
        }
        tallies
    }

    fn head(&self) -> [u8; 32] {
        self.entries.last().map_or([0; 32], LogEntry::hash)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Genesis,
    Locked,
    Event,
    Retry,
}

/// A Player's install key, as the Run log knows them.
type PlayerKey = [u8; 32];

/// A moment in one Player's game: a boot of it, and the frame boundary in that boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GamePoint {
    pub boot: u32,
    pub frame: u64,
}

/// A Run Checkpoint in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// Its place among the Run's Run Checkpoints, from 1.
    pub n: u64,
    /// Whose in-game save (or Sync Gate) called it.
    pub by: PlayerKey,
    /// Each Player's part of it, by Player.
    pub parts: BTreeMap<PlayerKey, Part>,
    /// A Retry went back to an earlier one: it can't be gone back to.
    pub void: bool,
}

/// One Player's part of a Run Checkpoint: the checkpoint of their own game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The log entry that says so, counting from the first.
    pub entry: usize,
    /// Where in their game it was taken.
    pub at: GamePoint,
    /// The checkpoint's hash, as their Run file has it.
    pub hash: String,
}

/// An open Retry vote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    /// The Run Checkpoint it would go back to.
    pub call: u64,
    /// Who said Yes so far.
    pub yes: BTreeSet<PlayerKey>,
}

/// The `n`th Run Checkpoint (from 1), if there is one.
fn numbered(calls: &[Call], n: u64) -> Option<&Call> {
    calls.get(usize::try_from(n).ok()?.checked_sub(1)?)
}

fn numbered_mut(calls: &mut [Call], n: u64) -> Option<&mut Call> {
    calls.get_mut(usize::try_from(n).ok()?.checked_sub(1)?)
}

/// The kind of a body, read without trusting the rest of it.
fn body_kind(body: &[u8]) -> Option<Kind> {
    #[derive(Deserialize)]
    struct Tag {
        kind: String,
    }
    let tag: Tag = serde_json::from_slice(body).ok()?;
    match tag.kind.as_str() {
        "genesis" => Some(Kind::Genesis),
        "locked" => Some(Kind::Locked),
        "event" => Some(Kind::Event),
        "retry" => Some(Kind::Retry),
        _ => None,
    }
}

fn author_message(run_id: &[u8; 32], counter: u64, body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(DOMAIN.len() + 40 + body.len());
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(run_id);
    message.extend_from_slice(&counter.to_le_bytes());
    message.extend_from_slice(body);
    message
}

fn verifies(key: &[u8; 32], message: &[u8], signature: &[u8]) -> bool {
    let (Ok(key), Ok(signature)) = (VerifyingKey::from_bytes(key), Signature::from_slice(signature)) else {
        return false;
    };
    key.verify_strict(message, &signature).is_ok()
}

/// What a Player's app does as the shared Run log grows: it joins every
/// other Player's Run Checkpoint with its own game, and when the log says
/// Retry, it goes back to its own part of that Run Checkpoint.
#[derive(Debug, Clone)]
pub struct LogFollower {
    me: PlayerKey,
    seen: usize,
}

impl LogFollower {
    /// For the Player with this key, from the start of the log.
    pub fn new(me: PlayerKey) -> Self {
        Self { me, seen: 0 }
    }

    /// The commands for this Player's Run from the entries since the last call.
    pub fn follow(&mut self, log: &RunLog) -> Vec<crate::RunCommand> {
        if log.entries().len() <= self.seen {
            return Vec::new();
        }
        let calls = log.calls();
        let mut commands = Vec::new();
        for (index, (author, body)) in log.bodies().enumerate().skip(self.seen) {
            match body {
                LogBody::Event(RunEvent::Checkpoint { .. }) if author != self.me => {
                    let called = calls
                        .iter()
                        .find(|c| c.by == author && c.parts.get(&author).is_some_and(|p| p.entry == index));
                    if let Some(called) = called.filter(|c| !c.void) {
                        commands.push(crate::RunCommand::JoinCheckpoint { call: called.n });
                    }
                }
                LogBody::Retry { call } => {
                    if let Some(part) = numbered(&calls, call).and_then(|c| c.parts.get(&self.me)) {
                        commands.push(crate::RunCommand::Retry {
                            to: crate::RetryTo::Checkpoint(part.hash.clone()),
                        });
                    }
                }
                _ => {}
            }
        }
        self.seen = log.entries().len();
        commands
    }
}
