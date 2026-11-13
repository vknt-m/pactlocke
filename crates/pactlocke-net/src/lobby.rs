//! The lobby, as two state machines with no network in them: the Host's,
//! which admits Players and keeps the roster, and a joiner's. They trade
//! bytes; the transport (iroh, or memory in tests) only carries them.

use crate::invite::{Invite, PeerKey, RunSecret};
use crate::wire::{self, LobbyPlayer, MAX_PLAYERS, Message, Refusal, RomStatus, RunAdvert};

/// Messages a Player may send in a burst, and per second after it.
const BURST: f64 = 40.0;
const PER_SECOND: f64 = 20.0;
/// Messages dropped (malformed, unexpected or too fast) before the Host
/// closes the connection.
const DROPS_ALLOWED: u32 = 20;
/// Input stream messages have a bucket of their own: a Player at 4x sends
/// several a second, and a resend after a gap comes in a burst.
const INPUT_BURST: f64 = 120.0;
const INPUT_PER_SECOND: f64 = 60.0;

/// A name as other Players see it: trimmed, with no control or
/// direction-changing characters.
fn shown_name(name: &str) -> String {
    let bidi = |c: char| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}');
    name.chars()
        .filter(|&c| !c.is_control() && !bidi(c))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// What the Host does about a connection, decided before any message is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Admit,
    Refuse(Refusal),
}

/// Something the Host's transport must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAction {
    Send {
        to: PeerKey,
        bytes: Vec<u8>,
    },
    /// Setup or a log entry from a Player, for the Host's app to read.
    Deliver {
        from: PeerKey,
        message: Message,
    },
    /// Send `bytes` (the refusal) and close the connection.
    Close {
        peer: PeerKey,
        refusal: Refusal,
        bytes: Vec<u8>,
    },
}

/// A joiner in the Host's roster.
#[derive(Debug, Clone)]
struct Member {
    key: PeerKey,
    name: Option<String>,
    rom: RomStatus,
    connected: bool,
    tokens: f64,
    input_tokens: f64,
    last_ms: i64,
    dropped: u32,
}

/// The Host's lobby: who is invited, who has joined, and whose ROM matches.
#[derive(Debug, Clone)]
pub struct HostLobby {
    run: RunAdvert,
    app_version: String,
    host_name: String,
    open_invites: Vec<RunSecret>,
    members: Vec<Member>,
    /// Locked: the roster is frozen, so nobody new joins.
    locked: bool,
}

impl HostLobby {
    pub fn new(run: RunAdvert, host_name: &str, app_version: &str) -> Self {
        Self {
            run,
            app_version: app_version.to_owned(),
            host_name: host_name.to_owned(),
            open_invites: Vec::new(),
            members: Vec::new(),
            locked: false,
        }
    }

    /// A new invite for one friend: its secret admits one Player, once.
    pub fn invite(&mut self, host: PeerKey, relay: Option<String>) -> Invite {
        let secret = RunSecret::generate();
        self.open_invites.push(secret);
        Invite {
            host,
            relay,
            secret,
            app_version: self.app_version.clone(),
        }
    }

    /// Decides on a connection from its key and the invite secret it brought,
    /// before reading any message. A key in the roster may come back; any
    /// other needs an unused secret, which it uses up.
    pub fn admit(&mut self, peer: PeerKey, proof: Option<&[u8; 16]>) -> Admission {
        if let Some(member) = self.members.iter_mut().find(|m| m.key == peer) {
            member.connected = true;
            // A new connection: what the old one sent counts no more. Its token
            // buckets carry over, so a Player who reconnects is still rate-limited;
            // after the lock only the roster can reconnect at all.
            member.dropped = 0;
            return Admission::Admit;
        }
        if self.locked {
            return Admission::Refuse(Refusal::RunStarted);
        }
        // The secret first, so a stranger can't tell a full Run from a wrong invite.
        let Some(at) = proof.and_then(|proof| self.open_invites.iter().position(|s| s.matches(proof))) else {
            return Admission::Refuse(Refusal::NotInvited);
        };
        if self.members.len() + 1 >= MAX_PLAYERS {
            return Admission::Refuse(Refusal::RunFull);
        }
        self.open_invites.remove(at);
        self.members.push(Member {
            key: peer,
            name: None,
            rom: RomStatus::NotYet,
            connected: true,
            tokens: BURST,
            input_tokens: INPUT_BURST,
            last_ms: 0,
            dropped: 0,
        });
        Admission::Admit
    }

    /// A message from an admitted Player. Malformed, unexpected or too many
    /// too fast, it is dropped; enough of those and the Player is closed.
    pub fn receive(&mut self, peer: PeerKey, bytes: &[u8], now_ms: i64) -> Vec<HostAction> {
        let Some(at) = self.members.iter().position(|m| m.key == peer && m.connected) else {
            return Vec::new();
        };
        let message = {
            let member = &mut self.members[at];
            let elapsed = (now_ms - member.last_ms).max(0) as f64 / 1000.0;
            member.tokens = (member.tokens + elapsed * PER_SECOND).min(BURST);
            member.input_tokens = (member.input_tokens + elapsed * INPUT_PER_SECOND).min(INPUT_BURST);
            member.last_ms = now_ms;
            let bucket = if wire::is_input(bytes) {
                &mut member.input_tokens
            } else {
                &mut member.tokens
            };
            if *bucket >= 1.0 {
                *bucket -= 1.0;
                wire::decode(bytes).ok()
            } else {
                None
            }
        };
        let named = self.members[at].name.is_some();
        match message {
            // A rejoin says hello again on its new connection, and is welcomed back.
            Some(Message::Hello { app_version, name }) => {
                if app_version != self.app_version {
                    let host_version = self.app_version.clone();
                    return vec![self.close(peer, Refusal::Version { host_version })];
                }
                let name = shown_name(&name);
                self.members[at].name = Some(if name.is_empty() {
                    format!("Player {}", at + 2)
                } else {
                    name
                });
                let mut actions = vec![HostAction::Send {
                    to: peer,
                    bytes: wire::encode(&Message::Welcome { run: self.run.clone() }),
                }];
                actions.extend(self.broadcast());
                actions
            }
            Some(Message::RomCheck { sha1 }) if named => {
                self.members[at].rom = if self.run.matches_rom(&sha1) {
                    RomStatus::Matches
                } else {
                    RomStatus::Wrong
                };
                self.broadcast()
            }
            Some(message @ (Message::Setup(_) | Message::Log(_) | Message::Input(_))) if named => {
                vec![HostAction::Deliver { from: peer, message }]
            }
            _ => {
                self.members[at].dropped += 1;
                if self.members[at].dropped > DROPS_ALLOWED {
                    vec![self.close(peer, Refusal::Misbehaving)]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// The Ruleset and roster locked: from now on only the roster may connect.
    /// Players who never said hello leave the roster, and open invites are void.
    pub fn lock(&mut self) {
        self.locked = true;
        self.open_invites.clear();
        self.members.retain(|m| m.name.is_some());
    }

    /// `message` to one Player, or to every Player in the lobby.
    pub fn send(&self, to: Option<PeerKey>, message: &Message) -> Vec<HostAction> {
        let bytes = wire::encode(message);
        self.members
            .iter()
            .filter(|m| m.connected && m.name.is_some() && to.is_none_or(|to| to == m.key))
            .map(|m| HostAction::Send {
                to: m.key,
                bytes: bytes.clone(),
            })
            .collect()
    }

    /// A Player's connection ended; they stay in the roster and may come back.
    pub fn disconnected(&mut self, peer: PeerKey) -> Vec<HostAction> {
        match self.members.iter_mut().find(|m| m.key == peer) {
            Some(member) if member.connected => {
                member.connected = false;
                self.broadcast()
            }
            _ => Vec::new(),
        }
    }

    /// The lobby as everyone sees it: the Host, then each Player connected
    /// and named, in join order.
    pub fn players(&self) -> Vec<LobbyPlayer> {
        let host = LobbyPlayer {
            name: self.host_name.clone(),
            host: true,
            rom: RomStatus::Matches,
        };
        let joined = self.members.iter().filter(|m| m.connected).filter_map(|m| {
            Some(LobbyPlayer {
                name: m.name.clone()?,
                host: false,
                rom: m.rom,
            })
        });
        std::iter::once(host).chain(joined).collect()
    }

    /// Every Player who has said hello, connected or away, in join order:
    /// their key and name, for Run setup.
    pub fn roster(&self) -> Vec<(PeerKey, String)> {
        self.members
            .iter()
            .filter_map(|m| Some((m.key, m.name.clone()?)))
            .collect()
    }

    pub fn run(&self) -> &RunAdvert {
        &self.run
    }

    /// Closes a Player's connection. Before the lock their place is freed (the
    /// invite they used stays used); after it, the place is theirs for the
    /// Run and they may connect again.
    fn close(&mut self, peer: PeerKey, refusal: Refusal) -> HostAction {
        if self.locked {
            if let Some(member) = self.members.iter_mut().find(|m| m.key == peer) {
                member.connected = false;
            }
        } else {
            self.members.retain(|m| m.key != peer);
        }
        HostAction::Close {
            peer,
            bytes: wire::encode(&Message::Refused(refusal.clone())),
            refusal,
        }
    }

    fn broadcast(&self) -> Vec<HostAction> {
        let bytes = wire::encode(&Message::Lobby {
            players: self.players(),
        });
        self.members
            .iter()
            .filter(|m| m.connected && m.name.is_some())
            .map(|m| HostAction::Send {
                to: m.key,
                bytes: bytes.clone(),
            })
            .collect()
    }
}

/// Where a joiner is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinState {
    /// Connecting, or waiting for the Host's welcome.
    Joining,
    InLobby {
        run: RunAdvert,
        players: Vec<LobbyPlayer>,
    },
    /// The Host closed the connection; what to tell the Player.
    Refused(String),
}

/// A joiner's lobby.
#[derive(Debug, Clone)]
pub struct JoinLobby {
    invite: Invite,
    name: String,
    app_version: String,
    rom_sha1: Option<String>,
    state: JoinState,
    /// Setup and log messages from the Host, for the app to read.
    inbox: Vec<Message>,
}

impl JoinLobby {
    /// Joining by `invite`. An invite from a Host on another version is
    /// refused here, before connecting, naming the version to update to.
    pub fn new(invite: Invite, name: &str, app_version: &str, rom_sha1: Option<String>) -> Result<Self, String> {
        if invite.app_version != app_version {
            return Err(format!(
                "This invite is from Pactlocke {host}, and you have {app_version}: you need {host} to join.",
                host = invite.app_version
            ));
        }
        Ok(Self {
            invite,
            name: name.to_owned(),
            app_version: app_version.to_owned(),
            rom_sha1,
            state: JoinState::Joining,
            inbox: Vec::new(),
        })
    }

    pub fn invite(&self) -> &Invite {
        &self.invite
    }

    /// What the Host reads before anything else: the invite's secret.
    pub fn proof(&self) -> [u8; 16] {
        self.invite.secret.0
    }

    /// The first message, once admitted.
    pub fn hello(&self) -> Vec<u8> {
        wire::encode(&Message::Hello {
            app_version: self.app_version.clone(),
            name: self.name.clone(),
        })
    }

    /// A message from the Host; returns what to send back.
    pub fn receive(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        match wire::decode(bytes) {
            Ok(Message::Welcome { run }) => {
                self.state = JoinState::InLobby {
                    run,
                    players: Vec::new(),
                };
                self.rom_check().into_iter().collect()
            }
            Ok(Message::Lobby { players: now }) => {
                if let JoinState::InLobby { players, .. } = &mut self.state {
                    *players = now;
                }
                Vec::new()
            }
            Ok(Message::Refused(refusal)) => {
                self.state = JoinState::Refused(format!("The Host closed the connection: {refusal}."));
                Vec::new()
            }
            Ok(message @ (Message::Setup(_) | Message::Log(_) | Message::Input(_))) => {
                self.inbox.push(message);
                Vec::new()
            }
            // Anything else from the Host is dropped.
            _ => Vec::new(),
        }
    }

    /// The Player picked a ROM; returns the message telling the Host, if in the lobby.
    pub fn rom_picked(&mut self, sha1: &str) -> Option<Vec<u8>> {
        self.rom_sha1 = Some(sha1.to_owned());
        self.rom_check()
    }

    /// Whether the Player's ROM is the Run's game, once both are known.
    pub fn rom_matches(&self) -> Option<bool> {
        match (&self.state, &self.rom_sha1) {
            (JoinState::InLobby { run, .. }, Some(sha1)) => Some(run.matches_rom(sha1)),
            _ => None,
        }
    }

    /// The Host's setup and log messages since the last call, oldest first.
    pub fn take_inbox(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.inbox)
    }

    /// A setup or log message to the Host.
    pub fn send(&self, message: &Message) -> Vec<u8> {
        wire::encode(message)
    }

    pub fn state(&self) -> &JoinState {
        &self.state
    }

    /// The connection ended without a refusal.
    pub fn lost(&mut self, why: &str) {
        if !matches!(self.state, JoinState::Refused(_)) {
            self.state = JoinState::Refused(format!("The connection to the Host was lost: {why}."));
        }
    }

    fn rom_check(&self) -> Option<Vec<u8>> {
        let sha1 = self.rom_sha1.clone()?;
        matches!(self.state, JoinState::InLobby { .. }).then(|| wire::encode(&Message::RomCheck { sha1 }))
    }
}
