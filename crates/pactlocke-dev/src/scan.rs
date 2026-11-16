//! The scanner: what a ROM holds where, read against a base Game Profile,
//! and the draft profile that says so.

use std::fmt::Write as _;

use pactlocke_game::{
    DRAFT_PREFIX, GameProfile, MAX_LEVEL, Method, ROM_BASE, SourceKind, decode_text, encode_text, level_cap_sites,
    sha1_hex,
};

/// Script variables (`VAR_0x4000` on) can stand for a species in `givemon`.
const SCRIPT_VARS: std::ops::RangeInclusive<u16> = 0x4000..=0x40FF;
/// `special` and `waitstate`: a starter given from code follows these.
const SPECIAL: u8 = 0x25;
const WAITSTATE: u8 = 0x27;
/// A Romhack may expand the species list well past the original 412.
const EXPANDED_SPECIES: u16 = 0x1000;
/// A trainer table has at least this many plausible entries in a row.
const TRAINERS_AT_LEAST: usize = 100;
/// Entries in a row that may look wrong inside a trainer table (a hack's
/// placeholder, an odd party) without ending it.
const ODD_TRAINERS_ALLOWED: u32 = 2;
/// At most this many places are listed for a moved patch site.
const CANDIDATES_SHOWN: usize = 8;

/// Everything the scanner found in one ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Findings {
    /// The four-letter game code in the ROM's header.
    pub game_code: String,
    /// The title in the ROM's header.
    pub title: String,
    /// The SHA-1 of the whole ROM, in hex.
    pub sha1: String,
    /// The bundled profile it was compared with.
    pub base_id: String,
    /// The base profile's tables, looked for by their contents.
    pub tables: Vec<TableFinding>,
    /// The base profile's patch sites, checked here.
    pub patch_sites: Vec<SiteFinding>,
    /// Where the level checks a cap patches are, found by byte pattern.
    pub level_cap_sites: Vec<u32>,
    /// Script instructions that give a Pokémon or start a Static battle.
    pub sources: Vec<SourceCandidate>,
    /// Trainers who could be Sync Gates.
    pub gates: Vec<GateCandidate>,
}

/// A table found by its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableFinding {
    /// Its symbol in a Game Profile.
    pub symbol: &'static str,
    /// Where the base profile has it.
    pub base: u32,
    /// Where this ROM has it, if it was found: at the base profile's
    /// address when one of the matches is there, else the first match.
    pub found: Option<u32>,
    /// Every other place that matches: a Romhack that moved an expanded
    /// table often leaves the old one where it was.
    pub also: Vec<u32>,
}

impl TableFinding {
    fn of(symbol: &'static str, base: u32, matches: Vec<u32>) -> Self {
        let found = matches
            .iter()
            .copied()
            .find(|&at| at == base)
            .or(matches.first().copied());
        let also = matches.into_iter().filter(|&at| Some(at) != found).collect();
        Self {
            symbol,
            base,
            found,
            also,
        }
    }
}

/// One of the base profile's patch sites, checked in this ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteFinding {
    /// The patch it belongs to, as the draft names it: "Catch block".
    pub patch: &'static str,
    /// Its address in the base profile.
    pub at: u32,
    /// The ROM holds the expected bytes at the same address.
    pub unchanged: bool,
    /// Otherwise, where those bytes are found (the first few).
    pub candidates: Vec<u32>,
}

/// A script instruction that gives a Pokémon or starts a Static battle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCandidate {
    /// A Gift, a trade or a Static.
    pub kind: SourceKind,
    /// The instruction's address in the ROM.
    pub at: u32,
    /// The Pokémon it names.
    pub species: ScriptSpecies,
    /// Its level, when the instruction says.
    pub level: Option<u8>,
    /// The base profile's source at this address, if it lists one.
    pub catalogued: Option<String>,
}

/// The species a script instruction names: one species, or a script
/// variable the script sets first (a starter or a prize picked from a menu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSpecies {
    /// One species: its number, and its name from the ROM if it was read.
    Species {
        /// The species number.
        id: u16,
        /// Its name in the ROM's species table.
        name: Option<String>,
    },
    /// The script variable holding the species.
    Variable(u16),
    /// The Player's pick, which the game gives from code after a `special`.
    Chosen,
}

impl std::fmt::Display for ScriptSpecies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptSpecies::Species { name: Some(name), .. } => f.write_str(name),
            ScriptSpecies::Species { id, name: None } => write!(f, "species {id}"),
            ScriptSpecies::Variable(var) => write!(f, "VAR {var:#06X}"),
            ScriptSpecies::Chosen => f.write_str("the Player's pick"),
        }
    }
}

/// A trainer of a class the base profile's cap stages (gym leaders, the
/// Elite Four, the Champion) belong to, with their `trainerbattle` instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCandidate {
    /// The trainer's number in the ROM's trainer table.
    pub trainer: u16,
    /// Their name, as the ROM spells it.
    pub name: String,
    /// Their trainer class.
    pub class: u8,
    /// Where their `trainerbattle` instructions are.
    pub battles: Vec<u32>,
}

/// The trainer table and how many entries it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrainerTable {
    at: u32,
    count: u16,
}

/// Scans `rom` against `base`, a profile it is likely laid out like (the
/// original game a Romhack was made from).
pub fn scan(rom: &[u8], base: &GameProfile) -> Findings {
    let header = |range: std::ops::Range<usize>| {
        rom.get(range)
            .map(|b| String::from_utf8_lossy(b).trim_end_matches('\0').to_owned())
            .unwrap_or_default()
    };
    let trainers = find_trainers(rom, base);
    let tables = vec![
        TableFinding::of(
            "species_names",
            base.symbols.species_names,
            find_species_names(rom, base),
        ),
        // A base profile without a type names table gives none to compare against.
        TableFinding::of(
            "type_names",
            base.symbols.type_names.unwrap_or_default(),
            find_type_names(rom, base),
        ),
        TableFinding::of("items", base.symbols.items, find_items(rom, base)),
        TableFinding::of(
            "trainers",
            base.symbols.trainers,
            trainers.iter().map(|t| t.at).collect(),
        ),
    ];
    let species_names = tables[0].found;
    Findings {
        game_code: header(0xAC..0xB0),
        title: header(0xA0..0xAC),
        sha1: sha1_hex(rom),
        base_id: base.id.clone(),
        patch_sites: check_patch_sites(rom, base),
        level_cap_sites: level_cap_sites(rom),
        sources: find_sources(rom, base, species_names),
        gates: trainers.map(|table| find_gates(rom, base, table)).unwrap_or_default(),
        tables,
    }
}

/// Addresses as `0x…, 0x…`, or `none`.
fn addresses(list: &[u32]) -> String {
    if list.is_empty() {
        return "none".to_owned();
    }
    list.iter()
        .map(|at| format!("{at:#010X}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Findings {
    /// A draft Game Profile extending the base one, for a person to review.
    /// It is never trusted: its id is a draft's and its status untested.
    pub fn draft(&self) -> String {
        let mut out = String::new();
        self.write_header(&mut out);
        self.write_tables(&mut out);
        self.write_patch_sites(&mut out);
        self.write_sources(&mut out);
        self.write_gates(&mut out);
        out
    }

    fn write_header(&self, out: &mut String) {
        let code = self
            .game_code
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "");
        let lines = [
            format!(
                "# DRAFT Game Profile for {} ({}), drafted by `cargo dev scan`",
                self.title, self.game_code
            ),
            format!(
                "# against {}. Review every entry: nothing here is trusted until a person has",
                self.base_id
            ),
            "# checked it and the profile's acceptance scripts pass. Then give it its own id.".to_owned(),
            "schema = 1".to_owned(),
            format!("id = \"{DRAFT_PREFIX}{code}-{}\"", &self.sha1[..8]),
            format!("name = {:?}", format!("{} (draft)", self.title)),
            format!("game_code = {:?}", self.game_code),
            format!("sha1 = \"{}\"", self.sha1),
            "status = \"untested\"".to_owned(),
            format!("parent = \"{}\"", self.base_id),
        ];
        for line in lines {
            let _ = writeln!(out, "{line}");
        }
    }

    fn write_tables(&self, out: &mut String) {
        let _ = writeln!(out, "\n# Tables, found by their contents.");
        let moved: Vec<&TableFinding> = self
            .tables
            .iter()
            .filter(|t| t.found.is_some_and(|f| f != t.base))
            .collect();
        if !moved.is_empty() {
            let _ = writeln!(out, "[symbols]");
        }
        for table in moved {
            let found = table.found.unwrap_or_default();
            let _ = writeln!(
                out,
                "{} = {found:#010X}   # {:#010X} in {}",
                table.symbol, table.base, self.base_id
            );
        }
        for table in &self.tables {
            match table.found {
                Some(at) if at == table.base => {
                    let _ = writeln!(out, "# {} is where {} has it, {at:#010X}", table.symbol, self.base_id);
                }
                None => {
                    let _ = writeln!(out, "# {} wasn't found: check it by hand", table.symbol);
                }
                Some(_) => {}
            }
            if !table.also.is_empty() {
                let _ = writeln!(
                    out,
                    "# {} also matches at {}: a moved table may have left the old one behind",
                    table.symbol,
                    addresses(&table.also)
                );
            }
        }
    }

    fn write_patch_sites(&self, out: &mut String) {
        let _ = writeln!(out, "\n# Patch sites from {}:", self.base_id);
        for site in &self.patch_sites {
            let state = if site.unchanged {
                "unchanged".to_owned()
            } else {
                format!("MOVED; its bytes are at {}", addresses(&site.candidates))
            };
            let _ = writeln!(out, "#   {} at {:#010X}: {state}", site.patch, site.at);
        }
        let _ = writeln!(
            out,
            "# Level-100 checks by byte pattern (cmp #100 after a MON_DATA_LEVEL read), candidates only: {}",
            addresses(&self.level_cap_sites)
        );
    }

    fn write_sources(&self, out: &mut String) {
        let _ = writeln!(out, "\n# Gift and Static candidates (givemon, giveegg, setwildbattle):");
        for source in &self.sources {
            let level = source.level.map(|l| format!(" lv {l}")).unwrap_or_default();
            let listed = match &source.catalogued {
                Some(key) => format!("catalogued in {} as {key}", self.base_id),
                None => "new".to_owned(),
            };
            let instruction = match source.species {
                ScriptSpecies::Chosen => "special (the starter, given from code)",
                _ => source.kind.instruction(),
            };
            let _ = writeln!(
                out,
                "#   {:#010X} {instruction} {}{level}: {listed}",
                source.at, source.species
            );
        }
    }

    fn write_gates(&self, out: &mut String) {
        let _ = writeln!(
            out,
            "\n# Sync Gate candidates by trainer class (the classes of {}'s cap stages):",
            self.base_id
        );
        for gate in &self.gates {
            let _ = writeln!(
                out,
                "#   trainer {} {} (class {:#04X}): trainerbattle at {}",
                gate.trainer,
                gate.name,
                gate.class,
                addresses(&gate.battles)
            );
        }
    }
}

fn rom_at(rom: &[u8], addr: u32, len: usize) -> Option<&[u8]> {
    let at = addr.checked_sub(ROM_BASE)? as usize;
    rom.get(at..at + len)
}

fn u16_at(rom: &[u8], addr: u32) -> Option<u16> {
    rom_at(rom, addr, 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(rom: &[u8], addr: u32) -> Option<u32> {
    rom_at(rom, addr, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn in_rom(rom: &[u8], addr: u32) -> bool {
    addr >= ROM_BASE && ((addr - ROM_BASE) as usize) < rom.len()
}

/// Every address where `needle` is, from `step`-aligned offsets.
fn find_all(rom: &[u8], needle: &[u8], step: usize) -> Vec<u32> {
    if needle.is_empty() {
        return Vec::new();
    }
    (0..rom.len().saturating_sub(needle.len() - 1))
        .step_by(step)
        .filter(|&at| &rom[at..at + needle.len()] == needle)
        .map(|at| ROM_BASE + at as u32)
        .collect()
}

/// A name in the game's characters, ended with the terminator.
fn encoded(name: &str) -> Vec<u8> {
    let mut bytes = encode_text(name, name.chars().count()).expect("the game has these characters");
    bytes.push(0xFF);
    bytes
}

/// Every fixed-length name table with `first` at entry `index` and `second` after it.
fn find_name_table(rom: &[u8], first: &str, second: &str, index: u32, length: u32) -> Vec<u32> {
    let next = encoded(second);
    find_all(rom, &encoded(first), 1)
        .into_iter()
        .filter(|&at| rom_at(rom, at + length, next.len()) == Some(&next[..]))
        .filter_map(|at| at.checked_sub(index * length))
        .collect()
}

/// Species names: BULBASAUR then IVYSAUR, or as some Romhacks write them,
/// Bulbasaur then Ivysaur.
fn find_species_names(rom: &[u8], base: &GameProfile) -> Vec<u32> {
    let length = base.layout.species_name_length;
    let upper = find_name_table(rom, "BULBASAUR", "IVYSAUR", 1, length);
    if upper.is_empty() {
        find_name_table(rom, "Bulbasaur", "Ivysaur", 1, length)
    } else {
        upper
    }
}

/// Type names are padded with spaces, then the terminator: NORMAL, then FIGHT.
fn find_type_names(rom: &[u8], base: &GameProfile) -> Vec<u32> {
    let length = base.layout.type_name_length;
    let (Ok(first), Ok(second)) = (encode_text("NORMAL", 6), encode_text("FIGHT", 5)) else {
        return Vec::new();
    };
    find_all(rom, &first, 1)
        .into_iter()
        .filter(|&at| rom_at(rom, at + length, 5) == Some(&second[..]))
        .filter(|&at| rom_at(rom, at + length + 5, 1).is_some_and(|b| b[0] == 0x00 || b[0] == 0xFF))
        .collect()
}

fn find_items(rom: &[u8], base: &GameProfile) -> Vec<u32> {
    find_name_table(rom, "MASTER BALL", "ULTRA BALL", 1, base.layout.item_size)
}

/// The trainer table: the longest run of entries that look like trainers
/// (party flags, a party of 1 to 6 in ROM, levels 1 to 100 and a species,
/// allowing for an expanded species list), with the empty trainer before it.
/// A couple of odd entries in a row don't end the run.
fn find_trainers(rom: &[u8], base: &GameProfile) -> Option<TrainerTable> {
    let layout = &base.layout;
    let size = layout.trainer_size as usize;
    let plausible = |at: usize| -> bool {
        let addr = ROM_BASE + at as u32;
        let Some(&flags) = rom.get(at + layout.trainer_party_flags as usize) else {
            return false;
        };
        let (Some(party_size), Some(party)) = (
            u32_at(rom, addr + layout.trainer_party_size),
            u32_at(rom, addr + layout.trainer_party),
        ) else {
            return false;
        };
        let Some(&mon_size) = layout.trainer_mon_sizes.get(usize::from(flags)) else {
            return false;
        };
        (1..=6).contains(&party_size)
            && in_rom(rom, party)
            && (0..party_size).all(|i| {
                let mon = party + i * mon_size;
                let level = u16_at(rom, mon + layout.trainer_mon_level).unwrap_or(0);
                let species = u16_at(rom, mon + layout.trainer_mon_level + 2).unwrap_or(0);
                (1..=u16::from(MAX_LEVEL)).contains(&level) && species != 0 && species < EXPANDED_SPECIES
            })
    };
    // runs[i]: how many entries, from offset 4*i, until more than a couple
    // of odd ones in a row; odd[i]: how many odd ones in a row start there.
    let slots = rom.len() / 4;
    let stride = size / 4;
    let mut runs = vec![0u32; slots + stride];
    let mut odd = vec![0u32; slots + stride];
    let mut best: Option<(u32, usize)> = None;
    for slot in (0..slots).rev() {
        if plausible(slot * 4) {
            runs[slot] = 1 + runs[slot + stride];
            if best.is_none_or(|(len, _)| runs[slot] >= len) {
                best = Some((runs[slot], slot * 4));
            }
        } else {
            odd[slot] = 1 + odd[slot + stride];
            if odd[slot] <= ODD_TRAINERS_ALLOWED && runs[slot + stride] > 0 {
                runs[slot] = 1 + runs[slot + stride];
            }
        }
    }
    // Entry 0 is the empty trainer, so the table starts one entry before the run.
    let (len, at) = best.filter(|&(len, _)| len as usize >= TRAINERS_AT_LEAST)?;
    Some(TrainerTable {
        at: (ROM_BASE + at as u32).checked_sub(layout.trainer_size)?,
        count: u16::try_from(len + 1).unwrap_or(u16::MAX),
    })
}

fn check_patch_sites(rom: &[u8], base: &GameProfile) -> Vec<SiteFinding> {
    let rules = &base.rules;
    let named = [
        ("Catch block", &rules.catch_block),
        ("Level caps", &rules.level_cap),
        ("No items in trainer battles", &rules.no_items_trainer),
    ];
    let mut found = Vec::new();
    for (patch, method) in named {
        let Some(Method::Patch { sites }) = method else {
            continue;
        };
        for site in sites {
            let expect = &site.expect.0;
            let unchanged = rom_at(rom, site.at, expect.len()) == Some(&expect[..]);
            let candidates = if unchanged {
                Vec::new()
            } else {
                find_all(rom, expect, 2).into_iter().take(CANDIDATES_SHOWN).collect()
            };
            found.push(SiteFinding {
                patch,
                at: site.at,
                unchanged,
                candidates,
            });
        }
    }
    found
}

/// `givemon`, `giveegg` and `setwildbattle` anywhere in the ROM, as a
/// Romhack may have moved or rewritten its scripts: each with operands a real
/// instruction would have (a species the ROM names), and followed by a script
/// command a real script does next. And the starter where the base game gives
/// it from code after a `special`: that special anywhere it is, by its number
/// in this ROM.
fn find_sources(rom: &[u8], base: &GameProfile, species_names: Option<u32>) -> Vec<SourceCandidate> {
    let catalogue: Vec<(u32, &str)> = base
        .sources
        .iter()
        .flat_map(|s| s.at.iter().map(move |&at| (at, s.key.as_str())))
        .collect();
    let catalogued = |addr: u32| {
        catalogue
            .iter()
            .find(|(at, _)| *at == addr)
            .map(|(_, key)| (*key).to_owned())
    };
    let species = Species::new(rom, base, species_names);
    let item_count = base.layout.item_count;
    let levels = 1..=MAX_LEVEL;
    let starter_special = starter_special(rom, base);
    let mut found = Vec::new();
    for at in 0..rom.len() as u32 {
        let addr = ROM_BASE + at;
        let op = rom[at as usize];
        if let Some(special) = starter_special
            && rom_at(rom, addr, 4) == Some(&special[..])
        {
            found.push(SourceCandidate {
                kind: SourceKind::Starter,
                at: addr,
                species: ScriptSpecies::Chosen,
                level: None,
                catalogued: catalogued(addr),
            });
            continue;
        }
        let Some(kind) = [SourceKind::Gift, SourceKind::Egg, SourceKind::Static]
            .into_iter()
            .find(|kind| kind.opcode() == op)
        else {
            continue;
        };
        let Some(id) = u16_at(rom, addr + 1) else {
            continue;
        };
        let level = rom_at(rom, addr + 3, 1).map(|b| b[0]);
        let a_level = level.is_some_and(|l| levels.contains(&l));
        let an_item = u16_at(rom, addr + 4).is_some_and(|item| u32::from(item) < item_count);
        // Each instruction's length, and whether its operands could be real.
        let (length, plausible) = match kind {
            // givemon SPECIES LEVEL ITEM, then nine unused bytes (eight zero).
            SourceKind::Gift => (
                15,
                (species.named(id) || SCRIPT_VARS.contains(&id))
                    && a_level
                    && an_item
                    && rom_at(rom, addr + 6, 8).is_some_and(|b| b.iter().all(|&x| x == 0)),
            ),
            SourceKind::Egg => (3, species.named(id)),
            _ => (6, species.named(id) && a_level && an_item),
        };
        let next = addr + length;
        // An egg is common data: only what follows a real giveegg will do.
        let follows = match kind {
            SourceKind::Egg => egg_follow_up_at(rom, next, &species),
            _ => script_command_at(rom, next, &species),
        };
        if !plausible || !follows {
            continue;
        }
        found.push(SourceCandidate {
            kind,
            at: addr,
            species: species.of(id),
            level: (kind != SourceKind::Egg).then_some(level).flatten(),
            catalogued: catalogued(addr),
        });
    }
    found
}

/// The species a ROM names, read from its species names table where the
/// scanner found it.
struct Species<'a> {
    rom: &'a [u8],
    names: Option<u32>,
    name_length: u32,
    /// Without the names table: the base game's count.
    count: u16,
}

impl<'a> Species<'a> {
    fn new(rom: &'a [u8], base: &GameProfile, names: Option<u32>) -> Self {
        Self {
            rom,
            names,
            name_length: base.layout.species_name_length,
            count: base.layout.species_count,
        }
    }

    /// Its name in the ROM, if that entry reads as a name.
    fn name(&self, id: u16) -> Option<String> {
        if id == 0 || id >= EXPANDED_SPECIES {
            return None;
        }
        let table = self.names?;
        let entry = rom_at(
            self.rom,
            table + u32::from(id) * self.name_length,
            self.name_length as usize,
        )?;
        let ends = entry.contains(&pactlocke_game::text::END);
        let name = decode_text(entry);
        // FireRed's unused species slots are all named "?", and past the
        // last species the table runs into move names (" HEAL BELL").
        let readable = name.starts_with(char::is_alphanumeric) && !name.contains(char::REPLACEMENT_CHARACTER);
        (ends && readable).then_some(name)
    }

    /// Whether a script naming it could be real: the ROM names it, or,
    /// without the names table, it is one of the base game's.
    fn named(&self, id: u16) -> bool {
        match self.names {
            Some(_) => self.name(id).is_some(),
            None => (1..self.count).contains(&id),
        }
    }

    fn of(&self, id: u16) -> ScriptSpecies {
        if SCRIPT_VARS.contains(&id) {
            return ScriptSpecies::Variable(id);
        }
        ScriptSpecies::Species {
            id,
            name: self.name(id),
        }
    }
}

/// Whether a script command a Gift or Static script does next starts at
/// `addr`, with operands that could be real: a compare, goto or call, a flag
/// or variable set, a text load, a cry, the wild battle, a species name
/// buffered, a release, or a text colour.
fn script_command_at(rom: &[u8], addr: u32, species: &Species) -> bool {
    let byte = |offset: u32| rom_at(rom, addr + offset, 1).map(|b| b[0]);
    let pointer = |offset: u32| u32_at(rom, addr + offset).is_some_and(|p| in_rom(rom, p));
    let variable = |offset: u32| matches!(byte(offset + 1), Some(0x40 | 0x80));
    let Some(op) = byte(0) else {
        return false;
    };
    match op {
        // compare VAR, value
        0x21 => variable(1),
        // call, goto
        0x04 | 0x05 => pointer(1),
        // call_if, goto_if
        0x06 | 0x07 => byte(1).is_some_and(|c| c <= 5) && pointer(2),
        // setflag, clearflag
        0x29 | 0x2A => byte(2).is_some_and(|hi| hi <= 0x09),
        // setvar
        0x16 => variable(1),
        // copyvar
        0x19 => variable(1) && variable(3),
        // loadword
        0x0F => byte(1).is_some_and(|i| i <= 3) && pointer(2),
        // waitse, then playmoncry
        0x30 => byte(1) == Some(0xA1),
        // playmoncry SPECIES
        0xA1 => u16_at(rom, addr + 1).is_some_and(|id| species.named(id)),
        // dowildbattle
        0xB7 => true,
        // bufferspeciesname
        0x7D => byte(1).is_some_and(|i| i <= 2),
        // release, releaseall, then end or return
        0x6B | 0x6C => matches!(byte(1), Some(0x02 | 0x03)),
        // textcolor
        0xC7 => byte(1).is_some_and(|c| c <= 3),
        _ => false,
    }
}

/// Whether what a `giveegg` script does next starts at `addr`: a text
/// colour (FireRed, LeafGreen), a release then end (Hoenn), a text load, or a
/// fanfare. Bytes in code that pass for a `giveegg` go on to a goto or a flag.
fn egg_follow_up_at(rom: &[u8], addr: u32, species: &Species) -> bool {
    match rom_at(rom, addr, 1).map(|b| b[0]) {
        // playfanfare SONG: the games' songs are numbered below 0x400
        Some(0x31) => rom_at(rom, addr + 2, 1).is_some_and(|hi| hi[0] < 0x04),
        // textcolor, release or releaseall, loadword: as after any script
        Some(0xC7 | 0x6B | 0x6C | 0x0F) => script_command_at(rom, addr, species),
        _ => false,
    }
}

/// `special N` and `waitstate`, as this ROM has them where the base profile
/// lists its starter, when the base game gives the starter from code.
fn starter_special(rom: &[u8], base: &GameProfile) -> Option<[u8; 4]> {
    let at = *base
        .sources
        .iter()
        .find(|s| s.kind == SourceKind::Starter)?
        .at
        .first()?;
    let bytes: [u8; 4] = rom_at(rom, at, 4)?.try_into().ok()?;
    (bytes[0] == SPECIAL && bytes[3] == WAITSTATE).then_some(bytes)
}

/// Trainers of the classes the base profile's cap stages belong to, with the
/// `trainerbattle` instructions (type 0 to 9, then the trainer, two zero
/// bytes and a pointer into ROM) that start their battles.
fn find_gates(rom: &[u8], base: &GameProfile, table: TrainerTable) -> Vec<GateCandidate> {
    let layout = &base.layout;
    let entry = |id: u16| table.at + u32::from(id) * layout.trainer_size;
    let class_of = |id: u16| rom_at(rom, entry(id) + 1, 1).map(|b| b[0]);
    let mut classes: Vec<u8> = base
        .level_caps
        .iter()
        .flat_map(|stage| stage.trainers.iter().filter_map(|&id| class_of(id)))
        .collect();
    classes.sort_unstable();
    classes.dedup();
    let gate_trainers: Vec<(u16, u8)> = (1..table.count)
        .filter_map(|id| class_of(id).filter(|c| classes.contains(c)).map(|c| (id, c)))
        .collect();
    let mut battles: Vec<(u16, u32)> = Vec::new();
    for at in 0..rom.len().saturating_sub(10) {
        if rom[at] != 0x5C || rom[at + 1] > 9 || rom[at + 4] != 0 || rom[at + 5] != 0 {
            continue;
        }
        let trainer = u16::from_le_bytes([rom[at + 2], rom[at + 3]]);
        let pointer = u32::from_le_bytes([rom[at + 6], rom[at + 7], rom[at + 8], rom[at + 9]]);
        if in_rom(rom, pointer) && gate_trainers.iter().any(|&(id, _)| id == trainer) {
            battles.push((trainer, ROM_BASE + at as u32));
        }
    }
    gate_trainers
        .into_iter()
        .map(|(trainer, class)| GateCandidate {
            trainer,
            name: rom_at(rom, entry(trainer) + 4, 12).map(decode_text).unwrap_or_default(),
            class,
            battles: battles
                .iter()
                .filter(|(t, _)| *t == trainer)
                .map(|(_, at)| *at)
                .collect(),
        })
        .collect()
}
