//! The Run log every Player shares: Host-ordered, Player-signed and
//! hash-linked, holding Run state only. Each entry is signed by its author
//! (the Player it is about) with their install's key; the Host numbers it,
//! links it to the entry before by hash and countersigns that. So every
//! Player can check the whole log and recompute the tallies, and the Host
//! can only order entries: changing one breaks its author's signature, and
//! inventing one needs a key the Host doesn't have.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Locked, RunEvent};

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

    /// The tallies, recomputed from the entries.
    pub fn tallies(&self) -> Tallies {
        let mut tallies = Tallies::default();
        for (author, body) in self.bodies() {
            let LogBody::Event(event) = body else { continue };
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
