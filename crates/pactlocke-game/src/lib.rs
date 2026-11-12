//! Game Profiles, the capability report, and the game interface: the rules
//! engine's view of one Player's game.

mod capability;
mod fake;
mod game;
mod profile;
mod profile_game;
mod source;
mod store;
mod text;

pub use capability::{Capability, CapabilityReport, Rule};
pub use fake::FakeGame;
pub use game::{
    Actions, Badges, Battle, BattleKind, Battler, Change, Game, GameError, Memory, PartyStats, Pokemon, ScriptContext,
    ScriptStatus, Side, StoredPokemon,
};
pub use profile::{
    Bit, Family, GameProfile, HexBytes, Layout, Method, OptionDef, PatchSite, RuleMethods, SCHEMA_VERSION,
    SupportStatus, Symbols,
};
pub use profile_game::ProfileGame;
pub use source::{Source, SourceDef, SourceKind};
pub use store::{ProfileError, ProfileStore, RomIdentity, Unprofiled, sha1_hex};

/// A place that counts as one encounter location: a region-map section, after
/// the profile's merges and renames.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Area {
    pub id: u16,
    pub name: String,
}
