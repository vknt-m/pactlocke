//! Game Profiles, the capability report, and the game interface: the rules
//! engine's view of one Player's game.
#![warn(missing_docs)]

mod capability;
mod caps;
mod compressed;
mod fake;
mod fake_core;
mod game;
mod icons;
mod profile;
mod profile_game;
mod source;
mod store;
pub mod text;

pub use capability::{Capability, CapabilityReport, NativeOption, Rule, RuleOptionDef};
pub use caps::{LevelCap, LevelCapDef, MAX_LEVEL, level_cap_sites};
pub use fake::FakeGame;
pub use fake_core::{FakeCore, SAVE_KEYS};
pub use game::{
    Actions, AvatarRoom, Badges, Battle, BattleKind, Battler, Change, Game, GameError, Memory, OAM, OBJ_PALETTES,
    OBJ_TILES, PartyStats, Pokemon, ScriptContext, ScriptStatus, Side, SpriteImage, StoredPokemon, Whereabouts,
    lead_of,
};
pub use icons::{ICON_SIZE, party_icon_rgba};
pub use profile::{
    Bit, BoxFormat, ExpandedFlags, Family, GameProfile, GateDef, HexBytes, Layout, Method, OptionDef, PatchSite, Place,
    ROM_BASE, RuleMethods, SCHEMA_VERSION, SupportStatus, Symbols, SyncGates,
};
pub use profile_game::ProfileGame;
pub use source::{Source, SourceDef, SourceKind};
pub use store::{
    DEV_PROFILES_VAR, DRAFT_PREFIX, ProfileError, ProfileStore, RomIdentity, TestedRomhack, UNTESTED_PREFIX,
    Unprofiled, sha1_hex,
};
pub use text::{decode as decode_text, encode as encode_text};

/// A place that counts as one encounter location: a region-map section, after
/// the profile's merges and renames.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Area {
    /// The Area's region-map section.
    pub id: u16,
    /// The Area's name as Players see it.
    pub name: String,
}
