//! Players' apps talking to each other: invite links, the lobby, and the
//! bounded, versioned wire format, over iroh (identity by key, no accounts,
//! no server). The lobby is plain state machines; the transport only carries
//! bytes, so tests run it in memory.

mod invite;
mod lobby;
pub mod memory;
mod node;
mod wire;

pub use invite::{Invite, InviteError, PeerKey, RunSecret};
pub use lobby::{Admission, HostAction, HostLobby, JoinLobby, JoinState};
pub use node::{HostNode, JoinNode, LobbyView, PeerLink, Relays, Replaced, load_or_create_key, new_key};
pub use wire::{
    LobbyPlayer, MAX_MESSAGE, MAX_PLAYERS, Message, PROTOCOL, Refusal, RomStatus, RunAdvert, WireError, decode, encode,
};

/// This build's version: Players must all run the same one.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
