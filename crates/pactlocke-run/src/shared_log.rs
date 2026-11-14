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
    /// By the next Player in join order, once the Host has been silent: they
    /// order the log from here on.
    Takeover,
    /// By the Host, once everyone else voted Yes: `player` leaves the Run
    /// for good. Their Pokémon leave every Link.
    Dropout { player: PlayerKey },
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
    /// A takeover by someone who isn't next in join order after the Host.
    BadTakeover { seq: u64 },
    /// A Dropout not everyone else voted for.
    BadDropout { seq: u64 },
    /// From a copy of the log without the takeover this copy has there or
    /// after: the copy with the rightful takeover wins.
    WithoutTakeover { seq: u64 },
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
            LogError::BadTakeover { seq } => {
                write!(f, "entry {seq} takes over the Run log out of turn")
            }
            LogError::BadDropout { seq } => write!(f, "entry {seq} drops a Player nobody agreed to drop"),
            LogError::WithoutTakeover { seq } => {
                write!(
                    f,
                    "entry {seq} is from a copy of the Run log without the takeover this copy has"
                )
            }
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

/// The fewest Players still in a Run for a Dropout vote: with two, one would decide alone.
pub const DROPOUT_MIN_PLAYERS: usize = 3;

/// Whether a Dropout can be voted with this many Players still in the Run.
fn dropout_allowed_with(players: usize) -> bool {
    players >= DROPOUT_MIN_PLAYERS
}

/// One copy of the Run log: the Host's, or a Player's, each checked entry by entry.
#[derive(Debug, Clone)]
pub struct RunLog {
    /// The Run's first Host, who wrote the Genesis entry.
    first_host: PlayerKey,
    host: [u8; 32],
    run_id: [u8; 32],
    entries: Vec<LogEntry>,
    roster: Option<Vec<[u8; 32]>>,
    counters: BTreeMap<[u8; 32], u64>,
    /// Players dropped from the Run, in the order they left.
    dropped: Vec<PlayerKey>,
}

impl RunLog {
    /// An empty log for the Run the Host with this key hosts.
    pub fn new(host: [u8; 32]) -> Self {
        Self {
            first_host: host,
            host,
            run_id: [0; 32],
            entries: Vec::new(),
            roster: None,
            counters: BTreeMap::new(),
            dropped: Vec::new(),
        }
    }

    /// The Run's first Host, who wrote the Genesis entry.
    pub fn first_host(&self) -> PlayerKey {
        self.first_host
    }

    /// Who orders the log now: the Host, or whoever took over.
    pub fn host(&self) -> [u8; 32] {
        self.host
    }

    /// The Players still in the Run, in join order: the roster less anyone dropped.
    pub fn active(&self) -> Vec<PlayerKey> {
        self.roster
            .iter()
            .flatten()
            .filter(|p| !self.dropped.contains(p))
            .copied()
            .collect()
    }

    /// Whether a Dropout can be voted now: only with three Players or more
    /// still in the Run, since with two one Player would decide alone.
    pub fn dropout_allowed(&self) -> bool {
        dropout_allowed_with(self.active().len())
    }

    /// Who takes over ordering the log if the Host goes silent: the next
    /// Player still in the Run after them in join order.
    pub fn successor(&self) -> Option<PlayerKey> {
        // In join order from the Host on, round to the start: the Host may
        // have been dropped, and is then skipped like anyone else dropped.
        let roster = self.roster.as_ref()?;
        let at = roster.iter().position(|p| *p == self.host).unwrap_or(0);
        let active = self.active();
        roster
            .iter()
            .cycle()
            .skip(at + 1)
            .take(roster.len())
            .find(|p| **p != self.host && active.contains(p))
            .copied()
    }

    /// The longest of these copies of the log that checks out from the
    /// Run's first Host: what a Player taking over continues from.
    pub fn longest(first_host: [u8; 32], copies: impl IntoIterator<Item = Vec<LogEntry>>) -> Option<Self> {
        copies
            .into_iter()
            .filter_map(|entries| Self::verify(first_host, entries).ok())
            // Two of the same length: the same one on every machine.
            .max_by_key(|log| (log.entries.len(), log.entries.last().map(LogEntry::hash)))
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

    /// A Player's copy of the log as their Run file keeps it, checked from
    /// its Genesis entry's author, the Run's first Host.
    pub fn from_kept(entries: Vec<LogEntry>) -> Result<Self, LogError> {
        let first_host = entries.first().ok_or(LogError::NoGenesis)?.submission.author;
        Self::verify(first_host, entries)
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
        let s = &entry.submission;
        let kind = body_kind(&s.body).ok_or(LogError::Body { seq })?;
        // A takeover is countersigned by the Player taking over.
        let host = if kind == Kind::Takeover { s.author } else { self.host };
        if !verifies(&host, &entry.hash(), &entry.host_signature) {
            return Err(LogError::HostSignature { seq });
        }
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
            Kind::Takeover if self.roster.is_none() || self.successor() != Some(s.author) => {
                return Err(LogError::BadTakeover { seq });
            }
            Kind::Takeover => {}
            Kind::Dropout if s.author != self.host => return Err(LogError::NotHost { seq }),
            Kind::Dropout => {
                let Ok(LogBody::Dropout { player }) = serde_json::from_slice(&s.body) else {
                    return Err(LogError::Body { seq });
                };
                if self.dropout_due() != Some(player) {
                    return Err(LogError::BadDropout { seq });
                }
            }
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
                let in_roster = self.active().contains(&s.author);
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
        match &body {
            LogBody::Locked(locked) => self.roster = Some(locked.roster.iter().map(|r| r.key).collect()),
            LogBody::Takeover => self.host = s.author,
            LogBody::Dropout { player } => self.dropped.push(*player),
            _ => {}
        }
        if seq == 0 {
            self.run_id = Sha256::digest(&s.body).into();
        }
        self.counters.insert(s.author, s.counter);
        self.entries.push(entry);
        Ok(())
    }

    /// Takes in an entry of the Host's copy of the log, as it comes. Where
    /// this copy has a different entry there, the copies split (the Host
    /// vanished mid-send, or played on alone): the Host's copy wins from
    /// there on, and the submissions of `mine` this copy drops come back, in
    /// order, to send again. Unless this copy has a takeover from there on:
    /// the copy with the rightful takeover wins, so this one stays.
    /// An entry already here changes nothing, and one further along the
    /// Host's copy than what has come of it here (past this copy's end, or
    /// not following this copy's entry before it) waits for the entries
    /// leading to it: a Host sends its newest entry on its own to be heard.
    pub fn give_way_to(&mut self, entry: LogEntry, mine: &PlayerKey) -> Result<Vec<Submission>, LogError> {
        let at = entry.seq as usize;
        let follows = at == 0
            || self
                .entries
                .get(at - 1)
                .is_some_and(|before| before.hash() == entry.prev);
        if at > self.entries.len() || !follows {
            return Ok(Vec::new());
        }
        match self.entries.get(at) {
            None => self.accept(entry).map(|()| Vec::new()),
            Some(here) if *here == entry => Ok(Vec::new()),
            Some(_)
                if self.entries[at..]
                    .iter()
                    .any(|e| body_kind(&e.submission.body) == Some(Kind::Takeover)) =>
            {
                Err(LogError::WithoutTakeover { seq: entry.seq })
            }
            Some(_) => {
                let mut theirs = Self::new(self.first_host);
                for kept in &self.entries[..at] {
                    theirs.accept(kept.clone())?;
                }
                theirs.accept(entry)?;
                let dropped = self.entries[at..]
                    .iter()
                    .filter(|e| e.submission.author == *mine)
                    .map(|e| e.submission.clone())
                    .collect();
                *self = theirs;
                Ok(dropped)
            }
        }
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
                        CheckpointReason::InGameSave | CheckpointReason::SyncGate | CheckpointReason::Asked => calls
                            .push(Call {
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
        !called.void && self.roster.is_some() && self.active().iter().all(|p| called.parts.contains_key(p))
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

    /// The Dropout vote open now, if any: who would leave and who said Yes.
    /// It starts at a Yes with none open and ends at the first No, or at the Dropout.
    pub fn dropout_vote(&self) -> Option<(PlayerKey, BTreeSet<PlayerKey>)> {
        let mut open: Option<(PlayerKey, BTreeSet<PlayerKey>)> = None;
        let mut gone: Vec<PlayerKey> = Vec::new();
        let roster = self.roster.as_ref().map_or(0, Vec::len);
        for (author, body) in self.bodies() {
            match body {
                // The leaver has no say, a vote on someone already gone counts
                // for nothing, and with two Players left there is no Dropout.
                LogBody::Event(RunEvent::DropoutVote { player, .. })
                    if author == player
                        || gone.contains(&player)
                        || !self.roster.as_ref().is_some_and(|r| r.contains(&player))
                        || !dropout_allowed_with(roster - gone.len()) => {}
                LogBody::Event(RunEvent::DropoutVote { player, yes }) => match &mut open {
                    None if yes => open = Some((player, BTreeSet::from([author]))),
                    Some((leaving, yes_votes)) if *leaving == player && yes => {
                        yes_votes.insert(author);
                    }
                    Some((leaving, _)) if *leaving == player => open = None,
                    _ => {}
                },
                LogBody::Dropout { player } => {
                    gone.push(player);
                    open = None;
                }
                _ => {}
            }
        }
        open
    }

    /// The Player the Host should drop now: everyone else still in the Run said Yes.
    pub fn dropout_due(&self) -> Option<PlayerKey> {
        let (leaving, yes) = self.dropout_vote()?;
        self.active()
            .iter()
            .filter(|p| **p != leaving)
            .all(|p| yes.contains(p))
            .then_some(leaving)
    }

    /// The Run Checkpoint a Retry should go back to now: the open vote's,
    /// once every Player said Yes and a Retry is left. The Host records it;
    /// every Player checks it.
    pub fn retry_due(&self) -> Option<u64> {
        let vote = self.vote()?;
        self.roster.as_ref()?;
        let everyone = self.active().iter().all(|p| vote.yes.contains(p));
        (everyone && self.retries_left() > 0).then_some(vote.call)
    }

    /// Every Player's events that stand, in log order, with their authors. A
    /// Retry voids each Player's entries after their part of its Run
    /// Checkpoint, up to the entry that says their game went back (any sent
    /// from before it on the way are void too).
    pub fn standing_events(&self) -> Vec<(PlayerKey, RunEvent)> {
        let mut standing: Vec<(usize, PlayerKey, RunEvent)> = Vec::new();
        let mut going_back: BTreeSet<PlayerKey> = BTreeSet::new();
        let mut calls: Option<Vec<Call>> = None;
        for (index, (author, body)) in self.bodies().enumerate() {
            match body {
                LogBody::Event(RunEvent::Retried { .. }) => {
                    going_back.remove(&author);
                }
                LogBody::Event(event) if event.outlives_retries() => standing.push((index, author, event)),
                LogBody::Event(_) if going_back.contains(&author) => {}
                LogBody::Event(event) => standing.push((index, author, event)),
                LogBody::Retry { call } => {
                    let calls = calls.get_or_insert_with(|| self.calls());
                    if let Some(called) = numbered(calls, call) {
                        standing.retain(|(at, who, event)| {
                            event.outlives_retries() || called.parts.get(who).is_none_or(|part| *at <= part.entry)
                        });
                        going_back.extend(called.parts.keys());
                    }
                }
                _ => {}
            }
        }
        standing.into_iter().map(|(_, author, event)| (author, event)).collect()
    }

    /// Every Retry in the log, in order: its call, and where it sent each
    /// Player's game back to.
    pub fn retries(&self) -> Vec<(u64, BTreeMap<PlayerKey, GamePoint>)> {
        let calls = self.calls();
        self.bodies()
            .filter_map(|(_, body)| match body {
                LogBody::Retry { call } => {
                    let called = numbered(&calls, call)?;
                    Some((call, called.parts.iter().map(|(p, part)| (*p, part.at)).collect()))
                }
                _ => None,
            })
            .collect()
    }

    /// Every cheat flag in the log that stands, one per Player, cheat and
    /// boot, with everyone who flagged it: all but those cleared by vote.
    pub fn flags(&self) -> Vec<crate::Flag> {
        self.flag_state().standing
    }

    /// The votes under way on clearing a cheat flag: the flag, and who said Yes.
    pub fn flag_votes(&self) -> Vec<FlagVote> {
        self.flag_state().votes
    }

    /// The cheat flags everyone's vote cleared, in the order they were.
    pub fn cleared_flags(&self) -> Vec<crate::Flag> {
        self.flag_state().cleared
    }

    /// Who may clear a flag on `player` by vote: everyone else still in the Run.
    pub fn flag_voters(&self, player: &PlayerKey) -> Vec<PlayerKey> {
        self.active().into_iter().filter(|p| p != player).collect()
    }

    /// Every cheat flag, every vote on clearing one, and the flags cleared,
    /// from the log in order. A vote starts at a Yes and ends at the first
    /// No; once everyone but the flagged Player said Yes, the flag is
    /// cleared, and the same flag logged again from no later a frame stays so.
    fn flag_state(&self) -> FlagState {
        let roster = self.roster.clone().unwrap_or_default();
        let mut state = FlagState::default();
        for (author, body) in self.bodies() {
            match body {
                LogBody::Event(RunEvent::CheatFlag {
                    player,
                    cheat,
                    boot,
                    frame,
                    at_ms,
                }) => {
                    let player = player.unwrap_or(author);
                    let cleared = state
                        .cleared
                        .iter()
                        .any(|f| f.is(&player, cheat, boot) && frame <= f.at.frame);
                    if !roster.contains(&player) || cleared {
                        continue;
                    }
                    match state.standing.iter_mut().find(|f| f.is(&player, cheat, boot)) {
                        Some(flag) if !flag.by.contains(&author) => flag.by.push(author),
                        Some(_) => {}
                        None => state.standing.push(crate::Flag {
                            player,
                            cheat,
                            at: GamePoint { boot, frame },
                            at_ms,
                            by: vec![author],
                        }),
                    }
                }
                LogBody::Event(RunEvent::FlagVote {
                    player,
                    cheat,
                    boot,
                    yes,
                }) => {
                    let voters = self.flag_voters(&player);
                    let Some(at) = state.standing.iter().position(|f| f.is(&player, cheat, boot)) else {
                        continue;
                    };
                    if !voters.contains(&author) {
                        continue;
                    }
                    let open = state.votes.iter().position(|v| v.flag.is(&player, cheat, boot));
                    match (open, yes) {
                        (Some(i), true) => {
                            state.votes[i].yes.insert(author);
                        }
                        (None, true) => state.votes.push(FlagVote {
                            flag: state.standing[at].clone(),
                            yes: BTreeSet::from([author]),
                        }),
                        (Some(i), false) => {
                            state.votes.remove(i);
                        }
                        (None, false) => {}
                    }
                    let done = state
                        .votes
                        .iter()
                        .position(|v| v.flag.is(&player, cheat, boot) && voters.iter().all(|p| v.yes.contains(p)));
                    if let Some(i) = done {
                        state.votes.remove(i);
                        state.cleared.push(state.standing.remove(at));
                    }
                }
                _ => {}
            }
        }
        state
    }

    /// The speed each Player last said they play at (1x until they say).
    pub fn speed_of(&self, player: &PlayerKey) -> u8 {
        self.standing_events()
            .into_iter()
            .rev()
            .find_map(|(author, event)| match event {
                RunEvent::SpeedChanged { speed } if author == *player => Some(speed),
                _ => None,
            })
            .unwrap_or(1)
    }

    /// Who is Ready at this Sync Gate, in log order.
    pub fn ready_at(&self, gate: &str) -> Vec<PlayerKey> {
        let mut ready = Vec::new();
        for (author, event) in self.standing_events() {
            if matches!(&event, RunEvent::GateReady { gate: g } if g == gate) && !ready.contains(&author) {
                ready.push(author);
            }
        }
        ready
    }

    /// The Player whose Ready made every Player Ready at this Sync Gate,
    /// once it is let go (counting only entries that stand).
    pub fn gate_opened_by(&self, gate: &str) -> Option<PlayerKey> {
        self.roster.as_ref()?;
        let roster = self.active();
        let mut ready: Vec<PlayerKey> = Vec::new();
        for (author, event) in self.standing_events() {
            if matches!(&event, RunEvent::GateReady { gate: g } if g == gate) && !ready.contains(&author) {
                ready.push(author);
                if roster.iter().all(|p| ready.contains(p)) {
                    return Some(author);
                }
            }
        }
        None
    }

    /// The chosen Sync Gate this Player is Ready at that isn't let go yet, if any.
    pub fn ready_gate_of(&self, player: &PlayerKey) -> Option<String> {
        let gates = self.locked()?.ruleset.sync_gates;
        gates
            .into_iter()
            .find(|g| !self.gate_open(g) && self.ready_at(g).contains(player))
    }

    /// Whether every Player in the roster is Ready at this Sync Gate.
    pub fn gate_open(&self, gate: &str) -> bool {
        let ready = self.ready_at(gate);
        self.roster.is_some() && self.active().iter().all(|p| ready.contains(p))
    }

    /// The ready check: the first chosen Sync Gate someone is Ready at and
    /// not everyone is (gyms can come in either order), or else the first
    /// not let go yet; and each Player, by roster order, with whether they are.
    pub fn ready_check(&self) -> Option<ReadyCheck> {
        let locked = self.locked()?;
        let waiting: Vec<&String> = locked
            .ruleset
            .sync_gates
            .iter()
            .filter(|g| !self.gate_open(g))
            .collect();
        let gate = (*waiting
            .iter()
            .find(|g| !self.ready_at(g).is_empty())
            .or(waiting.first())?)
        .clone();
        let ready = self.ready_at(&gate);
        let active = self.active();
        Some(ReadyCheck {
            players: locked
                .roster
                .iter()
                .filter(|p| active.contains(&p.key))
                .map(|p| (p.name.clone(), ready.contains(&p.key)))
                .collect(),
            gate,
        })
    }

    /// The tallies, recomputed from the events that stand.
    pub fn tallies(&self) -> Tallies {
        let mut tallies = Tallies::default();
        for (author, event) in self.standing_events() {
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
    Takeover,
    Dropout,
}

/// A Player's install key, as the Run log knows them.
type PlayerKey = [u8; 32];

/// A moment in one Player's game: a boot of it, and the frame boundary in that boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GamePoint {
    pub boot: u32,
    pub frame: u64,
}

/// The next Sync Gate's ready check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyCheck {
    /// The gate, as the Game Profile names it.
    pub gate: String,
    /// Each Player's name, and whether they are Ready there.
    pub players: Vec<(String, bool)>,
}

impl ReadyCheck {
    /// How many Players are Ready.
    pub fn ready(&self) -> usize {
        self.players.iter().filter(|(_, ready)| *ready).count()
    }
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
        "takeover" => Some(Kind::Takeover),
        "dropout" => Some(Kind::Dropout),
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

/// The cheat flags in a Run log as they stand, the votes on clearing them,
/// and those cleared.
#[derive(Default)]
struct FlagState {
    standing: Vec<crate::Flag>,
    votes: Vec<FlagVote>,
    cleared: Vec<crate::Flag>,
}

/// A vote under way on clearing a cheat flag: the flag, and who said Yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagVote {
    pub flag: crate::Flag,
    pub yes: BTreeSet<PlayerKey>,
}

/// What a Player's app does as the shared Run log grows: it keeps the log
/// in the Run file, joins every other Player's Run Checkpoint with its own
/// game, and when the log says Retry, it goes back to its own part of that
/// Run Checkpoint.
#[derive(Debug, Clone)]
pub struct LogFollower {
    me: PlayerKey,
    seen: usize,
    /// The hash of every entry kept in the Run file so far, in order.
    kept: Vec<[u8; 32]>,
    /// How many cleared cheat flags the feed has said so of.
    cleared: usize,
    /// Cheat flags already put in the feed: by Player, cheat and boot.
    flagged: Vec<(PlayerKey, crate::Cheat, u32)>,
}

impl LogFollower {
    /// For the Player with this key, from the start of the log.
    pub fn new(me: PlayerKey) -> Self {
        Self {
            me,
            seen: 0,
            kept: Vec::new(),
            cleared: 0,
            flagged: Vec::new(),
        }
    }

    /// For the Player with this key, resuming a Run with friends whose log,
    /// as its Run file keeps it, is `log`: what is in it was followed and
    /// kept before.
    pub fn resuming(me: PlayerKey, log: &RunLog) -> Self {
        Self {
            me,
            seen: log.entries().len(),
            kept: log.entries().iter().map(LogEntry::hash).collect(),
            cleared: log.cleared_flags().len(),
            flagged: Vec::new(),
        }
    }

    /// The command that keeps the log in the Run file, if it has changed
    /// since: grown, or replaced from some entry on by another copy.
    fn keep(&mut self, log: &RunLog) -> Option<crate::RunCommand> {
        let entries = log.entries();
        let from = entries
            .iter()
            .zip(&self.kept)
            .position(|(entry, kept)| entry.hash() != *kept)
            .unwrap_or(self.kept.len().min(entries.len()));
        if from == entries.len() && from == self.kept.len() {
            return None;
        }
        self.kept.truncate(from);
        self.kept.extend(entries[from..].iter().map(LogEntry::hash));
        // The winning copy's entries from there on are new to this Player's game.
        self.seen = self.seen.min(from);
        Some(crate::RunCommand::KeepLog {
            from: from as u64,
            entries: entries[from..].to_vec(),
        })
    }

    /// The commands for this Player's Run from the entries since the last call.
    pub fn follow(&mut self, log: &RunLog) -> Vec<crate::RunCommand> {
        let mut commands: Vec<crate::RunCommand> = self.keep(log).into_iter().collect();
        if log.entries().len() <= self.seen {
            return commands;
        }
        let calls = log.calls();
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
                // A flag reaches every feed once, the first time it is logged;
                // one the Player's own app made about their own game is there already.
                LogBody::Event(RunEvent::CheatFlag {
                    player,
                    cheat,
                    boot,
                    frame,
                    ..
                }) if !(player.is_none() && author == self.me) => {
                    let about = player.unwrap_or(author);
                    if !self.flagged.contains(&(about, cheat, boot)) {
                        self.flagged.push((about, cheat, boot));
                        let name = |key: PlayerKey| {
                            if key == self.me {
                                "you".to_owned()
                            } else {
                                log.locked()
                                    .and_then(|l| l.roster.into_iter().find(|p| p.key == key))
                                    .map_or_else(|| "a Player".to_owned(), |p| p.name)
                            }
                        };
                        commands.push(crate::RunCommand::Flagged {
                            player: name(about),
                            cheat,
                            frame,
                            by: if player.is_none() {
                                "their own app".to_owned()
                            } else {
                                name(author)
                            },
                        });
                    }
                }
                LogBody::Event(RunEvent::SpeedChanged { speed }) if author != self.me => {
                    let player = log
                        .locked()
                        .and_then(|l| l.roster.into_iter().find(|p| p.key == author))
                        .map_or_else(|| "A Player".to_owned(), |p| p.name);
                    commands.push(crate::RunCommand::OthersSpeed { player, speed });
                }
                // The Ready that makes everyone Ready lets the gate go; that
                // Player's game calls its Run Checkpoint. The Run engine lets a
                // gate go once, however often this says so.
                LogBody::Event(RunEvent::GateReady { gate }) => {
                    if let Some(by) = log.gate_opened_by(&gate) {
                        commands.push(crate::RunCommand::OpenGate {
                            checkpoint: by == self.me,
                            gate,
                        });
                    }
                }
                LogBody::Dropout { player } if player == self.me => commands.push(crate::RunCommand::DroppedOut),
                LogBody::Retry { call } if log.active().contains(&self.me) => {
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
        let cleared = log.cleared_flags();
        for flag in cleared.iter().skip(self.cleared) {
            let player = if flag.player == self.me {
                "you".to_owned()
            } else {
                log.locked()
                    .and_then(|l| l.roster.into_iter().find(|p| p.key == flag.player))
                    .map_or_else(|| "a Player".to_owned(), |p| p.name)
            };
            commands.push(crate::RunCommand::FlagCleared {
                player,
                frame: flag.at.frame,
            });
        }
        self.cleared = self.cleared.max(cleared.len());
        commands
    }
}
