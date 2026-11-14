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

/// Why a ROM with no Game Profile can't be played. (One laid out like a
/// supported game is played as Untested, on a profile made from that game's:
/// see [`ProfileStore::identify`].)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unprofiled {
    /// A tested Romhack whose Game Profile comes in a later update.
    ProfileComing(TestedRomhack),
    /// Not a game Pactlocke knows, or a Romhack that moved the game's tables.
    NotSupported,
}

/// A Romhack Pactlocke was tested on, as the tested list shows it. Pactlocke
/// hosts and links to nothing: a Player brings the patched ROM, matched by SHA-1.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestedRomhack {
    pub name: String,
    pub version: String,
    pub author: String,
    /// The Game Profile of the game it patches.
    pub base: String,
    /// The patched ROM's SHA-1, lowercase hex.
    pub sha1: String,
}

/// The tested Romhacks, bundled with the app.
const BUNDLED_ROMHACKS: &str = include_str!("../romhacks.toml");

/// Ids of the profiles made for Untested Romhacks start with this.
pub const UNTESTED_PREFIX: &str = "untested-";

#[derive(Debug, Clone, Default)]
pub struct ProfileStore {
    profiles: Vec<Arc<GameProfile>>,
    /// The ids of the developer's local, unbundled profiles: solo Runs only.
    local: Vec<String>,
    romhacks: Vec<TestedRomhack>,
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
        Self::from_sources_with_local(BUNDLED.iter().map(|(file, text)| (*file, *text)), none)?
            .with_tested_romhacks(BUNDLED_ROMHACKS)
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
        Self::from_sources_with_local(BUNDLED.iter().map(|(file, text)| (*file, *text)), local)?
            .with_tested_romhacks(BUNDLED_ROMHACKS)
    }

    /// The same store with this tested-Romhacks list (TOML, `[[romhacks]]`).
    /// Each must patch a game this store has a profile for.
    pub fn with_tested_romhacks(mut self, text: &str) -> Result<Self, ProfileError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct List {
            romhacks: Vec<TestedRomhack>,
        }
        let file = "romhacks.toml".to_owned();
        let list: List = toml::from_str(text).map_err(|e| ProfileError::Format {
            file: file.clone(),
            message: e.message().to_owned(),
        })?;
        for hack in &list.romhacks {
            if self.by_id(&hack.base).is_none() {
                return Err(ProfileError::UnknownParent {
                    file,
                    parent: hack.base.clone(),
                });
            }
            if !is_sha1(&hack.sha1) {
                return Err(ProfileError::Invalid {
                    file,
                    message: format!("{}: sha1 {:?} is not 40 lowercase hex digits", hack.name, hack.sha1),
                });
            }
        }
        self.romhacks = list.romhacks;
        Ok(self)
    }

    /// The tested Romhacks, for the list the app shows.
    pub fn tested_romhacks(&self) -> &[TestedRomhack] {
        &self.romhacks
    }

    /// The tested Romhack this patched ROM is, by its SHA-1.
    pub fn tested_romhack(&self, sha1: &str) -> Option<&TestedRomhack> {
        self.romhacks.iter().find(|hack| hack.sha1 == sha1)
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
            ..Self::default()
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

    /// Identifies a ROM: by its SHA-1, a profiled game or a tested Romhack;
    /// otherwise, a Romhack laid out like the game its header names is
    /// played as Untested on a profile made from that game's.
    pub fn identify(&self, rom: &[u8]) -> RomIdentity {
        let sha1 = sha1_hex(rom);
        let profiled = |profile: Arc<GameProfile>| RomIdentity::Profiled {
            capabilities: CapabilityReport::check(&profile, rom),
            profile,
        };
        if let Some(profile) = self.find(&sha1) {
            return profiled(profile.clone());
        }
        if let Some(hack) = self.tested_romhack(&sha1) {
            return RomIdentity::Unprofiled {
                verdict: Unprofiled::ProfileComing(hack.clone()),
                sha1,
            };
        }
        match self.untested_profile(rom, &sha1) {
            Some(profile) => profiled(Arc::new(profile)),
            None => RomIdentity::Unprofiled {
                sha1,
                verdict: Unprofiled::NotSupported,
            },
        }
    }

    /// An Untested Romhack's profile: the bundled profile of the game its
    /// header names, if the hack keeps that game's species and item names
    /// where the profile has them, under the hack's own SHA-1. It has no
    /// catalogue (Gifts are only seen after the fact) and no Partner Avatars;
    /// every rule whose patch sites the hack changed is greyed out by its
    /// capability report.
    fn untested_profile(&self, rom: &[u8], sha1: &str) -> Option<GameProfile> {
        let code = rom.get(0xAC..0xB0)?;
        let base = self
            .profiles
            .iter()
            .find(|p| p.game_code.as_bytes() == code && !self.is_local(&p.id))?;
        if !laid_out_like(base, rom) {
            return None;
        }
        let mut profile = (**base).clone();
        profile.id = format!("{UNTESTED_PREFIX}{}", &sha1[..8]);
        profile.name = format!("Romhack of {}", base.name);
        profile.sha1 = sha1.to_owned();
        profile.status = crate::SupportStatus::Untested;
        profile.parent = Some(base.id.clone());
        profile.sources.clear();
        profile.avatars = None;
        Some(profile)
    }
}

/// Whether a ROM keeps a profile's tables where it has them: Bulbasaur and
/// Ivysaur first in its species names, and the Master Ball first in its items.
fn laid_out_like(profile: &GameProfile, rom: &[u8]) -> bool {
    let name_at = |addr: u32, len: usize| {
        let start = addr.checked_sub(crate::ROM_BASE)? as usize;
        Some(crate::text::decode(rom.get(start..start + len)?))
    };
    let species = |id: u32| {
        let len = profile.layout.species_name_length;
        name_at(profile.symbols.species_names + id * len, len as usize)
    };
    let item = name_at(profile.symbols.items + profile.layout.item_size, 14);
    species(1).as_deref() == Some("BULBASAUR")
        && species(2).as_deref() == Some("IVYSAUR")
        && item.as_deref() == Some("MASTER BALL")
}

fn is_sha1(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
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
    if !is_sha1(&profile.sha1) {
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
    if let Some(avatars) = &profile.avatars {
        // Five sprite slots (two Partner Avatars, their Followers, and the
        // Player's own), 16 tiles and a palette each.
        if avatars.palettes.len() < 5 || avatars.palettes.iter().any(|p| *p > 15) {
            return Err("avatars: give five OBJ palette slots, 0 to 15".into());
        }
        if u32::from(avatars.first_tile) + 80 > 1024 || u32::from(avatars.first_oam) + 5 > 128 {
            return Err("avatars: 80 OBJ tiles from first_tile and 5 OAM entries from first_oam must fit".into());
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
