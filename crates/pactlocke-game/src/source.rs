//! The Gift and Static catalogue: the script instructions that give a
//! Pokémon or start a Static battle, each belonging to one source.

use serde::Deserialize;

/// How far past a catalogued instruction a script position may be and still
/// belong to it: the game runs on through a few instructions (a message, a
/// flag) in the frame the Pokémon arrives or the battle starts.
pub const SOURCE_WINDOW: u32 = 0x100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// The first Pokémon: always allowed, whichever one is picked.
    Starter,
    Gift,
    Egg,
    Static,
}

/// A Gift or Static source the Game Profile lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Stable across Players: the same Gift or Static has the same key in every game.
    pub key: String,
    pub name: String,
    pub kind: SourceKind,
}

/// A catalogue entry as the Game Profile writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDef {
    pub key: String,
    pub name: String,
    pub kind: SourceKind,
    /// The instructions (`givemon`, `giveegg`, `setwildbattle`) that belong to it;
    /// a choice of several Pokémon is several instructions of one source.
    pub at: Vec<u32>,
}

/// The source of the catalogued instruction nearest before `position`, within [`SOURCE_WINDOW`].
pub fn source_before(catalogue: &[SourceDef], position: u32) -> Option<Source> {
    catalogue
        .iter()
        .flat_map(|def| def.at.iter().map(move |&at| (at, def)))
        .filter(|&(at, _)| at <= position && position - at <= SOURCE_WINDOW)
        .max_by_key(|&(at, _)| at)
        .map(|(_, def)| Source {
            key: def.key.clone(),
            name: def.name.clone(),
            kind: def.kind,
        })
}
