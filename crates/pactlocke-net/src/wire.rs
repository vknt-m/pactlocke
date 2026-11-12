//! The wire format between Players' apps: bounded and versioned. Each
//! message is `PK`, the protocol version (two bytes), its kind (one byte),
//! its payload's length (two bytes) and the payload. Decoding never panics:
//! anything malformed is an error, and the caller drops it.

use std::fmt;

/// The protocol this build speaks. Apps must match exactly to play together.
pub const PROTOCOL: u16 = 1;
/// No message is longer than this, header and all.
pub const MAX_MESSAGE: usize = 16 * 1024;
/// A name, version or SHA-1 is at most this many bytes.
const MAX_TEXT: usize = 64;
/// A lobby has at most this many Players.
pub const MAX_PLAYERS: usize = 3;

const MAGIC: [u8; 2] = *b"PK";
const HEADER: usize = 7;

/// What a Run is, as the Host tells joiners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAdvert {
    pub name: String,
    pub game_name: String,
    /// The SHA-1 of the ROM every Player must bring.
    pub game_sha1: String,
}

impl RunAdvert {
    /// Whether a ROM with this SHA-1 is the Run's game.
    pub fn matches_rom(&self, sha1: &str) -> bool {
        sha1.eq_ignore_ascii_case(&self.game_sha1)
    }
}

/// Whether a Player's ROM is the Run's game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomStatus {
    /// Not told yet.
    NotYet,
    Matches,
    /// A different ROM: it can't be played in this Run.
    Wrong,
}

impl RomStatus {
    fn to_byte(self) -> u8 {
        match self {
            RomStatus::NotYet => 0,
            RomStatus::Matches => 1,
            RomStatus::Wrong => 2,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(RomStatus::NotYet),
            1 => Some(RomStatus::Matches),
            2 => Some(RomStatus::Wrong),
            _ => None,
        }
    }
}

/// A Player in the lobby, as everyone sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyPlayer {
    pub name: String,
    pub host: bool,
    pub rom: RomStatus,
}

/// Why the Host closed a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No invite: the key isn't in the roster and brought no unused Run secret.
    NotInvited,
    /// Three Players already.
    RunFull,
    /// A different app version; the joiner must update to the Host's.
    Version { host_version: String },
    /// Malformed messages, or too many too fast.
    Misbehaving,
    /// The Ruleset and roster are locked: nobody joins now.
    RunStarted,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NotInvited => write!(f, "this invite was already used or isn't for this Run"),
            Refusal::RunFull => write!(f, "the Run already has {MAX_PLAYERS} Players"),
            Refusal::Version { host_version } => {
                write!(
                    f,
                    "the Host plays on Pactlocke {host_version}, and you need {host_version} to join"
                )
            }
            Refusal::Misbehaving => write!(f, "your app sent messages the Host couldn't read"),
            Refusal::RunStarted => write!(f, "the Run has started: nobody joins after the lock"),
        }
    }
}

/// Everything that crosses the wire in the lobby.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A joiner's first message.
    Hello { app_version: String, name: String },
    /// The Host lets the joiner in.
    Welcome { run: RunAdvert },
    /// The Host closes the connection, saying why.
    Refused(Refusal),
    /// A joiner says which ROM they brought.
    RomCheck { sha1: String },
    /// The lobby as it is now, from the Host to everyone.
    Lobby { players: Vec<LobbyPlayer> },
    /// Run setup (the Ruleset, Ready, the lock), which the lobby carries
    /// without reading: the Run engine's setup messages.
    Setup(Vec<u8>),
    /// An entry for the Run log, or one the Host ordered, carried unread.
    Log(Vec<u8>),
    /// A Player's input stream (key masks, stamped changes, hashes) or a
    /// request for part of one, carried unread: the Run engine's stream
    /// messages, which Shadow Games replay.
    Input(Vec<u8>),
}

/// Why bytes aren't a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    TooShort,
    TooLong,
    NotPactlocke,
    /// Another protocol version: the apps don't match.
    Protocol(u16),
    UnknownKind(u8),
    /// The payload doesn't read as that kind of message.
    BadPayload,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::TooShort => write!(f, "too short"),
            WireError::TooLong => write!(f, "longer than {MAX_MESSAGE} bytes"),
            WireError::NotPactlocke => write!(f, "not a Pactlocke message"),
            WireError::Protocol(v) => write!(f, "protocol {v}, but this app speaks {PROTOCOL}"),
            WireError::UnknownKind(k) => write!(f, "unknown message kind {k}"),
            WireError::BadPayload => write!(f, "a malformed payload"),
        }
    }
}

impl std::error::Error for WireError {}

const HELLO: u8 = 1;
/// Kept the same in every protocol version, with its payload.
const WELCOME: u8 = 2;
const REFUSED: u8 = 3;
const ROM_CHECK: u8 = 4;
const LOBBY: u8 = 5;
const SETUP: u8 = 6;
const LOG: u8 = 7;
const INPUT: u8 = 8;

/// A message as bytes. Text longer than the format allows is cut short.
pub fn encode(message: &Message) -> Vec<u8> {
    let mut payload = Vec::new();
    let kind = match message {
        Message::Hello { app_version, name } => {
            put_text(&mut payload, app_version);
            put_text(&mut payload, name);
            HELLO
        }
        Message::Welcome { run } => {
            put_text(&mut payload, &run.name);
            put_text(&mut payload, &run.game_name);
            put_text(&mut payload, &run.game_sha1);
            WELCOME
        }
        Message::Refused(refusal) => {
            match refusal {
                Refusal::NotInvited => payload.push(0),
                Refusal::RunFull => payload.push(1),
                Refusal::Version { host_version } => {
                    payload.push(2);
                    put_text(&mut payload, host_version);
                }
                Refusal::Misbehaving => payload.push(3),
                Refusal::RunStarted => payload.push(4),
            }
            REFUSED
        }
        Message::RomCheck { sha1 } => {
            put_text(&mut payload, sha1);
            ROM_CHECK
        }
        Message::Setup(bytes) => {
            payload.extend_from_slice(&bytes[..bytes.len().min(MAX_MESSAGE - HEADER)]);
            SETUP
        }
        Message::Log(bytes) => {
            payload.extend_from_slice(&bytes[..bytes.len().min(MAX_MESSAGE - HEADER)]);
            LOG
        }
        Message::Input(bytes) => {
            payload.extend_from_slice(&bytes[..bytes.len().min(MAX_MESSAGE - HEADER)]);
            INPUT
        }
        Message::Lobby { players } => {
            let players = &players[..players.len().min(MAX_PLAYERS)];
            payload.push(players.len() as u8);
            for player in players {
                put_text(&mut payload, &player.name);
                payload.push(u8::from(player.host));
                payload.push(player.rom.to_byte());
            }
            LOBBY
        }
    };
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&PROTOCOL.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Whether `bytes` claim to be an input stream message, by their header
/// alone, so the Host can rate-limit those apart from the rest.
pub fn is_input(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER && bytes[..2] == MAGIC && bytes[4] == INPUT
}

/// The message in `bytes`, which must be exactly one message.
pub fn decode(bytes: &[u8]) -> Result<Message, WireError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(WireError::TooLong);
    }
    if bytes.len() < HEADER {
        return Err(WireError::TooShort);
    }
    if bytes[..2] != MAGIC {
        return Err(WireError::NotPactlocke);
    }
    let protocol = u16::from_le_bytes([bytes[2], bytes[3]]);
    let kind = bytes[4];
    // A refusal reads the same in every protocol version, so a Player told
    // to update can read why.
    if protocol != PROTOCOL && kind != REFUSED {
        return Err(WireError::Protocol(protocol));
    }
    let len = usize::from(u16::from_le_bytes([bytes[5], bytes[6]]));
    if bytes.len() != HEADER + len {
        return Err(WireError::BadPayload);
    }
    let mut reader = Reader(&bytes[HEADER..]);
    let message = match kind {
        HELLO => Message::Hello {
            app_version: reader.text()?,
            name: reader.text()?,
        },
        WELCOME => Message::Welcome {
            run: RunAdvert {
                name: reader.text()?,
                game_name: reader.text()?,
                game_sha1: reader.text()?,
            },
        },
        REFUSED => Message::Refused(match reader.byte()? {
            0 => Refusal::NotInvited,
            1 => Refusal::RunFull,
            2 => Refusal::Version {
                host_version: reader.text()?,
            },
            3 => Refusal::Misbehaving,
            4 => Refusal::RunStarted,
            _ => return Err(WireError::BadPayload),
        }),
        ROM_CHECK => Message::RomCheck { sha1: reader.text()? },
        SETUP => Message::Setup(std::mem::take(&mut reader.0).to_vec()),
        LOG => Message::Log(std::mem::take(&mut reader.0).to_vec()),
        INPUT => Message::Input(std::mem::take(&mut reader.0).to_vec()),
        LOBBY => {
            let count = usize::from(reader.byte()?);
            if count > MAX_PLAYERS {
                return Err(WireError::BadPayload);
            }
            let mut players = Vec::with_capacity(count);
            for _ in 0..count {
                let name = reader.text()?;
                let host = match reader.byte()? {
                    0 => false,
                    1 => true,
                    _ => return Err(WireError::BadPayload),
                };
                let rom = RomStatus::from_byte(reader.byte()?).ok_or(WireError::BadPayload)?;
                players.push(LobbyPlayer { name, host, rom });
            }
            Message::Lobby { players }
        }
        other => return Err(WireError::UnknownKind(other)),
    };
    if !reader.0.is_empty() {
        return Err(WireError::BadPayload);
    }
    Ok(message)
}

/// Text as one length byte and UTF-8, cut short at a character boundary.
fn put_text(out: &mut Vec<u8>, text: &str) {
    let mut end = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push(end as u8);
    out.extend_from_slice(&text.as_bytes()[..end]);
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, WireError> {
        let (&first, rest) = self.0.split_first().ok_or(WireError::BadPayload)?;
        self.0 = rest;
        Ok(first)
    }

    fn text(&mut self) -> Result<String, WireError> {
        let len = usize::from(self.byte()?);
        if len > MAX_TEXT || len > self.0.len() {
            return Err(WireError::BadPayload);
        }
        let (text, rest) = self.0.split_at(len);
        self.0 = rest;
        String::from_utf8(text.to_vec()).map_err(|_| WireError::BadPayload)
    }
}
