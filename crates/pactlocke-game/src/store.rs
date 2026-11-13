//! The Game Profile store: loads profiles, resolves parents, checks them, and
//! identifies a ROM by its SHA-1.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use sha1::{Digest, Sha1};

use crate::capability::CapabilityReport;
use crate::profile::{GameProfile, Method, ROM, SCHEMA_VERSION};

/// The profiles bundled with the app, as (file name, TOML).
const BUNDLED: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/bundled_profiles.rs"));

/// Fields a profile must set itself, never inherit from its parent.
const OWN_FIELDS: [&str; 6] = ["schema", "id", "name", "game_code", "sha1", "status"];

/// Why a profile couldn't be loaded. Every bundled profile must load, so CI
/// fails on any of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// Not valid TOML, or not a valid profile.
    Format { file: String, message: String },
    /// Written for a profile format this build doesn't read.
    Schema { file: String, found: i64 },
    /// Extends a profile that doesn't exist.
    UnknownParent { file: String, parent: String },
    /// Extends a profile that itself extends another; one parent at most.
    ParentChain { file: String, parent: String },
    /// Inherits a field every profile must set itself.
    MissingOwnField { file: String, field: &'static str },
    /// Two profiles share an id or a SHA-1.
    Duplicate { file: String, what: String },
    /// Well-formed, but contradicts itself.
    Invalid { file: String, message: String },
    /// A file or folder of profiles can't be read.
    Unreadable { file: String, message: String },
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format { file, message } => write!(f, "{file}: {message}"),
            Self::Schema { file, found } => {
                write!(
                    f,
                    "{file}: schema {found}, but this build reads schema {SCHEMA_VERSION}"
                )
            }
            Self::UnknownParent { file, parent } => write!(f, "{file}: no profile {parent:?} to extend"),
            Self::ParentChain { file, parent } => {
                write!(
                    f,
                    "{file}: {parent:?} extends another profile; only one parent is allowed"
                )
            }
            Self::MissingOwnField { file, field } => write!(f, "{file}: must set its own {field}"),
            Self::Duplicate { file, what } => write!(f, "{file}: another profile has the same {what}"),
            Self::Invalid { file, message } => write!(f, "{file}: {message}"),
            Self::Unreadable { file, message } => write!(f, "{file} can't be read: {message}"),
        }
    }
}

impl std::error::Error for ProfileError {}

/// What a ROM is, as far as Pactlocke knows.
#[derive(Debug, Clone, PartialEq)]
pub enum RomIdentity {
    /// A ROM with a Game Profile.
    Profiled {
        profile: Arc<GameProfile>,
        capabilities: CapabilityReport,
    },
    /// A ROM no profile matches.
    Unprofiled { sha1: String, verdict: Unprofiled },
}

/// What an unknown ROM can still be played as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unprofiled {
    /// Its header names a supported game (likely a Romhack of it): playable as
    /// Untested, with detection, penalties and red Violations.
    Untested,
    /// Not a game Pactlocke knows.
    NotSupported,
}

/// Header game codes of the supported original games.
const KNOWN_GAME_CODES: [&[u8; 4]; 5] = [b"BPRE", b"BPGE", b"BPEE", b"AXVE", b"AXPE"];

#[derive(Debug, Clone, Default)]
pub struct ProfileStore {
    profiles: Vec<Arc<GameProfile>>,
    /// The ids of the developer's local, unbundled profiles: solo Runs only.
    local: Vec<String>,
}

/// Draft profiles (from the developer scanner) have ids that start with
/// this, and are never bundled.
pub const DRAFT_PREFIX: &str = "draft-";

/// The developer setting: a folder of local Game Profiles, for solo Runs only.
pub const DEV_PROFILES_VAR: &str = "PACTLOCKE_DEV_PROFILES";

impl ProfileStore {
    /// The profiles bundled with this build.
    pub fn bundled() -> Result<Self, ProfileError> {
        let none: [(&str, &str); 0] = [];
        Self::from_sources_with_local(BUNDLED.iter().map(|(file, text)| (*file, *text)), none)
    }

    /// The bundled profiles and a developer's local ones, from every `.toml`
    /// file in `dir`. A developer setting: local profiles are for solo Runs.
    pub fn bundled_with_local(dir: &std::path::Path) -> Result<Self, ProfileError> {
        let unreadable = |e: std::io::Error| ProfileError::Unreadable {
            file: dir.display().to_string(),
            message: e.to_string(),
        };
        let mut local = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(unreadable)? {
            let path = entry.map_err(unreadable)?.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                let text = std::fs::read_to_string(&path).map_err(unreadable)?;
                local.push((path.display().to_string(), text));
            }
        }
        Self::from_sources_with_local(BUNDLED.iter().map(|(file, text)| (*file, *text)), local)
    }

    /// The bundled profiles, and with the developer setting
    /// ([`DEV_PROFILES_VAR`] naming a folder) a developer's local ones too.
    pub fn from_env() -> Result<Self, ProfileError> {
        match std::env::var_os(DEV_PROFILES_VAR) {
            Some(dir) => Self::bundled_with_local(std::path::Path::new(&dir)),
            None => Self::bundled(),
        }
    }

    /// Bundled profiles and local ones. A local profile may extend a bundled
    /// one, but not share its id or SHA-1: a bundled profile is edited where
    /// it is, so a Run's profile id always says which kind it was played on.
    /// A draft is never a bundled profile.
    pub fn from_sources_with_local<F, T, G, U>(
        bundled: impl IntoIterator<Item = (F, T)>,
        local: impl IntoIterator<Item = (G, U)>,
    ) -> Result<Self, ProfileError>
    where
        F: AsRef<str>,
        T: AsRef<str>,
        G: AsRef<str>,
        U: AsRef<str>,
    {
        let local: Vec<(String, String)> = local
            .into_iter()
            .map(|(file, text)| (file.as_ref().to_owned(), text.as_ref().to_owned()))
            .collect();
        let local_ids: Vec<String> = local.iter().filter_map(|(_, text)| profile_id_of(text)).collect();
        let mut sources = Vec::new();
        for (file, text) in bundled {
            let (file, text) = (file.as_ref().to_owned(), text.as_ref().to_owned());
            let id = profile_id_of(&text);
            if id.as_deref().is_some_and(|id| id.starts_with(DRAFT_PREFIX)) {
                return Err(ProfileError::Invalid {
                    file,
                    message: "a draft profile can't be bundled: review it and give it its own id first".into(),
                });
            }
            sources.push((file, text));
        }
        sources.extend(local);
        let mut store = Self::from_sources(sources)?;
        store.local = local_ids;
        Ok(store)
    }

    /// Whether a profile is a developer's local one, which only solo Runs may use.
    pub fn is_local(&self, id: &str) -> bool {
        self.local.iter().any(|local| local == id)
    }

    /// Loads profiles from (file name, TOML) pairs, resolving parents.
    pub fn from_sources<F, T>(sources: impl IntoIterator<Item = (F, T)>) -> Result<Self, ProfileError>
    where
        F: AsRef<str>,
        T: AsRef<str>,
    {
        let mut tables = Vec::new();
        for (file, text) in sources {
            let file = file.as_ref().to_owned();
            let table: toml::Table = toml::from_str(text.as_ref()).map_err(|e| ProfileError::Format {
                file: file.clone(),
                message: e.message().to_owned(),
            })?;
            tables.push((file, table));
        }

        let by_id: BTreeMap<&str, &toml::Table> = tables
            .iter()
            .filter_map(|(_, table)| table.get("id").and_then(|v| v.as_str()).map(|id| (id, table)))
            .collect();

        let mut profiles: Vec<Arc<GameProfile>> = Vec::new();
        for (file, table) in &tables {
            for field in OWN_FIELDS {
                if !table.contains_key(field) {
                    return Err(ProfileError::MissingOwnField {
                        file: file.clone(),
                        field,
                    });
                }
            }
            let schema = table.get("schema").and_then(|v| v.as_integer());
            if schema != Some(i64::from(SCHEMA_VERSION)) {
                return Err(ProfileError::Schema {
                    file: file.clone(),
                    found: schema.unwrap_or(-1),
                });
            }
            let resolved = match table.get("parent").and_then(|v| v.as_str()) {
                None => table.clone(),
                Some(parent_id) => {
                    let parent = by_id.get(parent_id).ok_or_else(|| ProfileError::UnknownParent {
                        file: file.clone(),
                        parent: parent_id.to_owned(),
                    })?;
                    if parent.contains_key("parent") {
                        return Err(ProfileError::ParentChain {
                            file: file.clone(),
                            parent: parent_id.to_owned(),
                        });
                    }
                    let mut merged = (*parent).clone();
                    merge(&mut merged, table);
                    merged
                }
            };
            let profile: GameProfile = resolved.try_into().map_err(|e: toml::de::Error| ProfileError::Format {
                file: file.clone(),
                message: e.message().to_owned(),
            })?;
            check(&profile).map_err(|message| ProfileError::Invalid {
                file: file.clone(),
                message,
            })?;
            for other in &profiles {
                let what = if other.id == profile.id {
                    format!("id {:?}", profile.id)
                } else if other.sha1 == profile.sha1 {
                    format!("SHA-1 {}", profile.sha1)
                } else {
                    continue;
                };
                return Err(ProfileError::Duplicate {
                    file: file.clone(),
                    what,
                });
            }
            profiles.push(Arc::new(profile));
        }
        Ok(Self {
            profiles,
            local: Vec::new(),
        })
    }

    /// The profile with this id.
    pub fn by_id(&self, id: &str) -> Option<&Arc<GameProfile>> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// Every profile, bundled and local.
    pub fn profiles(&self) -> impl Iterator<Item = &Arc<GameProfile>> {
        self.profiles.iter()
    }

    /// The profile for a ROM with this SHA-1 (lowercase hex).
    pub fn find(&self, sha1: &str) -> Option<&Arc<GameProfile>> {
        self.profiles.iter().find(|p| p.sha1 == sha1)
    }

    /// Identifies a ROM by its SHA-1.
    pub fn identify(&self, rom: &[u8]) -> RomIdentity {
        let sha1 = sha1_hex(rom);
        if let Some(profile) = self.find(&sha1) {
            return RomIdentity::Profiled {
                capabilities: CapabilityReport::check(profile, rom),
                profile: profile.clone(),
            };
        }
        let known = rom
            .get(0xAC..0xB0)
            .is_some_and(|code| KNOWN_GAME_CODES.iter().any(|known| code == known.as_slice()));
        RomIdentity::Unprofiled {
            sha1,
            verdict: if known {
                Unprofiled::Untested
            } else {
                Unprofiled::NotSupported
            },
        }
    }
}

/// A profile's `id`, if its TOML has one.
fn profile_id_of(text: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(text).ok()?;
    table.get("id")?.as_str().map(str::to_owned)
}

/// The SHA-1 of a ROM, as lowercase hex.
pub fn sha1_hex(rom: &[u8]) -> String {
    Sha1::digest(rom).iter().map(|b| format!("{b:02x}")).collect()
}

/// Lays `child` over `parent`: tables merge key by key, anything else replaces.
/// A table naming a `method` (a rule's enforcement) replaces the parent's
/// whole, since a different method takes different parameters.
fn merge(parent: &mut toml::Table, child: &toml::Table) {
    for (key, value) in child {
        match (parent.get_mut(key), value) {
            (Some(toml::Value::Table(base)), toml::Value::Table(over)) if !over.contains_key("method") => {
                merge(base, over)
            }
            _ => {
                parent.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Checks that a resolved profile doesn't contradict itself.
fn check(profile: &GameProfile) -> Result<(), String> {
    if profile.sha1.len() != 40 || !profile.sha1.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(format!("sha1 {:?} is not 40 lowercase hex digits", profile.sha1));
    }
    if profile.game_code.len() != 4 {
        return Err(format!("game_code {:?} is not four letters", profile.game_code));
    }
    for (name, option) in &profile.options {
        if !matches!(option.save_block, 1 | 2) {
            return Err(format!("option {name}: save_block must be 1 or 2"));
        }
        if let Some((value, _)) = option.values.iter().find(|(_, v)| **v & !option.mask != 0) {
            return Err(format!("option {name}: value {value:?} has bits outside its mask"));
        }
    }
    for (table, addr) in [
        ("items", profile.symbols.items),
        ("species_info", profile.symbols.species_info),
        ("species_names", profile.symbols.species_names),
        ("evolution_table", profile.symbols.evolution_table),
        ("trainers", profile.symbols.trainers),
    ] {
        if !ROM.contains(&addr) {
            return Err(format!("symbol {table} {addr:#010X} is not in ROM"));
        }
    }
    if profile.layout.trainer_mon_sizes.len() != 4 {
        return Err("layout trainer_mon_sizes must give a size for each of the 4 party flag values".into());
    }
    if let Some(gates) = &profile.sync_gates {
        if !ROM.contains(&gates.hold) {
            return Err(format!("sync_gates hold {:#010X} is not in ROM", gates.hold));
        }
        for gate in &gates.gates {
            if gate.sites.is_empty() || gate.sites.iter().any(|s| s.expect.0.len() != 5 || s.write.is_some()) {
                return Err(format!(
                    "sync gate {:?}: give each trainerbattle site with the 5 bytes a clean ROM holds, and no write",
                    gate.name
                ));
            }
        }
    }
    for stage in &profile.level_caps {
        if stage.trainers.is_empty() {
            return Err(format!("level cap {:?}: no trainers", stage.name));
        }
        if stage.badges.is_some() == stage.flag.is_some() {
            return Err(format!(
                "level cap {:?}: give exactly one of badges or flag for when it is passed",
                stage.name
            ));
        }
    }
    let rules = &profile.rules;
    // Whether each rule's patch writes fixed bytes (switched on and off) or
    // a value Pactlocke fills in (the cap).
    let named = [
        ("catch_block", &rules.catch_block, true),
        ("level_cap", &rules.level_cap, false),
        ("no_items_trainer", &rules.no_items_trainer, true),
        ("no_items_wild", &rules.no_items_wild, true),
        ("set_mode", &rules.set_mode, true),
        ("must_nickname", &rules.must_nickname, true),
    ];
    for (rule, method, switched) in named {
        match method {
            Some(Method::Patch { sites }) => {
                if sites.is_empty() {
                    return Err(format!("rule {rule}: no patch sites"));
                }
                for site in sites {
                    if !ROM.contains(&site.at) {
                        return Err(format!("rule {rule}: site {:#010X} is not in ROM", site.at));
                    }
                    if switched != site.write.is_some() {
                        let should = if switched { "must" } else { "must not" };
                        return Err(format!(
                            "rule {rule}: site {:#010X} {should} say what it writes",
                            site.at
                        ));
                    }
                    if let Some(write) = &site.write
                        && write.0.len() != site.expect.0.len()
                    {
                        return Err(format!(
                            "rule {rule}: site {:#010X} writes a different length than it expects",
                            site.at
                        ));
                    }
                }
            }
            Some(Method::OptionBit { option, value } | Method::Native { option, value }) => {
                let def = profile
                    .options
                    .get(option)
                    .ok_or_else(|| format!("rule {rule}: no option {option:?}"))?;
                if !def.values.contains_key(value) {
                    return Err(format!("rule {rule}: option {option:?} has no value {value:?}"));
                }
            }
            Some(Method::KeyFilter { command_table, .. }) if !ROM.contains(command_table) => {
                return Err(format!(
                    "rule {rule}: command table {command_table:#010X} is not in ROM"
                ));
            }
            _ => {}
        }
    }
    for (from, to) in &profile.area_merges {
        if !profile.areas.contains_key(from) || !profile.areas.contains_key(to) {
            return Err(format!(
                "area merge {from:#04X} -> {to:#04X} names a section not in [areas]"
            ));
        }
        if profile.area_merges.contains_key(to) {
            return Err(format!(
                "area merge {from:#04X} -> {to:#04X} points at a merged section"
            ));
        }
    }
    if let Some(id) = profile.area_renames.keys().find(|id| !profile.areas.contains_key(id)) {
        return Err(format!("area rename {id:#04X} names a section not in [areas]"));
    }
    Ok(())
}
