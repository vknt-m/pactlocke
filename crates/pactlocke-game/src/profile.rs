//! Game Profiles: everything Pactlocke needs to know about one exact ROM, read
//! from TOML, keyed by SHA-1, with at most one parent.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer};

/// The profile format this build reads.
pub const SCHEMA_VERSION: u32 = 1;

/// Where a GBA maps the cartridge ROM.
pub const ROM_BASE: u32 = 0x0800_0000;
/// Every address a 32 MiB ROM can occupy.
pub const ROM: std::ops::Range<u32> = ROM_BASE..0x0A00_0000;

/// One Game Profile, with its parent (if any) already resolved into it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameProfile {
    /// The profile format it is written in: see [`SCHEMA_VERSION`].
    pub schema: u32,
    /// The profile's own id, which a child profile names as its parent.
    pub id: String,
    /// The game's name as Players see it.
    pub name: String,
    /// The four-letter code in the ROM header, such as `BPRE`.
    pub game_code: String,
    /// The Game Family it belongs to.
    pub family: Family,
    /// The SHA-1 of the exact ROM, lowercase hex.
    pub sha1: String,
    /// How far the profile has been tested.
    pub status: SupportStatus,
    /// The id of the profile this one extends.
    #[serde(default)]
    pub parent: Option<String>,
    /// Where the game keeps what the rules engine reads.
    pub symbols: Symbols,
    /// How Pokémon, battlers, items and the PC are laid out.
    pub layout: Layout,
    /// The game's own options, by key.
    #[serde(default)]
    pub options: BTreeMap<String, OptionDef>,
    /// The enforcement method each game-dependent rule uses.
    #[serde(default)]
    pub rules: RuleMethods,
    /// The game's options Pactlocke sets for a rule beside its method:
    /// some only with other rules on, some while Pactlocke enforces the rule too.
    #[serde(default)]
    pub rule_options: Vec<crate::RuleOptionDef>,
    /// The game's region-map sections, by id, with the game's own names.
    #[serde(default, deserialize_with = "hex_keys")]
    pub areas: BTreeMap<u16, String>,
    /// Names Pactlocke shows instead of the game's own, by section id.
    #[serde(default, deserialize_with = "hex_keys")]
    pub area_renames: BTreeMap<u16, String>,
    /// Sections that count as another section's Area.
    #[serde(default, deserialize_with = "hex_keys")]
    pub area_merges: BTreeMap<u16, u16>,
    /// The Gift and Static catalogue.
    #[serde(default)]
    pub sources: Vec<crate::SourceDef>,
    /// The stages a level cap rises through, in order.
    #[serde(default)]
    pub level_caps: Vec<crate::LevelCapDef>,
    /// The game's type names by type number, for a game that keeps none as
    /// text (Heart and Soul shows only icons).
    #[serde(default)]
    pub type_name_list: Vec<String>,
    /// The battles that can be Sync Gates, and where their hold scripts go.
    #[serde(default)]
    pub sync_gates: Option<SyncGates>,
    /// Partner Avatars and Followers: on the original games only.
    #[serde(default)]
    pub avatars: Option<Avatars>,
    /// The Romhack's own randomizer, which Shared Seed gives every Player's game
    /// the same seed for.
    #[serde(default)]
    pub own_randomizer: Option<OwnRandomizer>,
}

/// How a Romhack's own randomizer is seeded, and what makes its draws depend on
/// the seed alone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnRandomizer {
    /// The seed is the trainer ID (Heart and Soul): every Player gets the shared one.
    #[serde(default)]
    pub seed_is_trainer_id: bool,
    /// A script variable holding the seed (R.O.W.E.'s 0x405B, 1 to 9998).
    #[serde(default)]
    pub seed_var: Option<SeedVar>,
    /// A byte of the trainer ID the randomizer also reads (R.O.W.E.'s second).
    #[serde(default)]
    pub trainer_id_byte: Option<u8>,
    /// Code patched while the seed is shared, so draws don't take the live RNG.
    #[serde(default)]
    pub sites: Vec<PatchSite>,
    /// Options kept at a setting while the seed is shared: modes that draw from
    /// the live RNG, which no seed makes the same.
    #[serde(default)]
    pub keep: Vec<OptionSetting>,
    /// The RNG pinned from a trainer battle's trigger until the enemy party
    /// changes, for a randomizer that draws teams from it (R.O.W.E.).
    #[serde(default)]
    pub rng_pin: Option<RngPin>,
}

/// A script variable, and where the game keeps its variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedVar {
    pub var: u16,
    /// In save block 1: variable 0x4000's place, two bytes a variable.
    pub vars: u32,
}

/// One of the game's options at one of its settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionSetting {
    pub option: String,
    pub value: String,
}

/// Where the RNG is and what says a trainer battle is coming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RngPin {
    /// `gRngValue`, the game's 32-bit RNG state.
    pub rng: u32,
    /// `gTrainerBattleOpponent_A`: the trainer, set once a battle is triggered.
    pub opponent: u32,
}

/// What drawing Partner Avatars and Followers needs: where a game keeps the
/// Player's position, where its own Player sprite sits on screen, the hardware
/// sprite slots, tiles and palettes the game leaves free on the overworld, and
/// its party icons.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Avatars {
    /// In save block 1: the Player's tile (two s16) and, then, map group and number.
    pub position: u32,
    pub location: u32,
    /// Where the Player's own 16x32 sprite's top-left corner is on screen.
    pub anchor_x: u16,
    pub anchor_y: u16,
    /// The first hardware OAM entry the game never fills on the overworld.
    pub first_oam: u8,
    /// The first 4bpp OBJ tile Pactlocke may draw into (64 from here).
    pub first_tile: u16,
    /// OBJ palette slots Pactlocke may load (one per partner, then icons).
    pub palettes: Vec<u8>,
    /// gMonIconTable, gMonIconPaletteIndices and gMonIconPaletteTable.
    pub icon_table: u32,
    pub icon_palette_indices: u32,
    pub icon_palette_table: u32,
}

/// Sync Gates: each gate trainer's verified `trainerbattle` instructions,
/// and free ROM (a clean ROM holds 0xFF there) for the hold scripts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncGates {
    /// Where the hold message and every gate's hold script go.
    pub hold: u32,
    /// Every battle that can be a Sync Gate, in the order the game reaches them.
    pub gates: Vec<GateDef>,
}

/// One battle that can be a Sync Gate.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDef {
    /// The trainer, as the ready check names the Gate: "Brock".
    pub name: String,
    /// Its `trainerbattle` instructions (one per starter for a Champion),
    /// each with the 5 bytes a clean ROM holds there.
    pub sites: Vec<PatchSite>,
}

impl SyncGates {
    /// How many bytes of free ROM the hold message and scripts take.
    pub fn hold_len(&self) -> usize {
        HOLD_MESSAGE_LEN + HOLD_SCRIPT_LEN * self.gates.len()
    }

    /// Where gate `index`'s hold script goes.
    pub fn hold_script(&self, index: usize) -> u32 {
        self.hold + (HOLD_MESSAGE_LEN + HOLD_SCRIPT_LEN * index) as u32
    }
}

impl GameProfile {
    /// Every battle this profile can make a Sync Gate, by name, in order.
    pub fn sync_gate_names(&self) -> Vec<String> {
        self.sync_gates
            .iter()
            .flat_map(|gates| gates.gates.iter().map(|gate| gate.name.clone()))
            .collect()
    }
}

/// The hold message takes the first bytes of the hold area; each gate's
/// hold script follows, in the profile's order.
pub const HOLD_MESSAGE_LEN: usize = 64;
/// One gate's hold script: `loadword 0, message`, `callstd MSGBOX_DEFAULT`,
/// `releaseall`, `end`, rounded up.
pub const HOLD_SCRIPT_LEN: usize = 16;

impl GameProfile {
    /// The Area a region-map section counts as, by its shown name: merges
    /// followed, renames applied. `None` for a section the profile doesn't list.
    pub fn area(&self, section: u16) -> Option<crate::Area> {
        let id = self.area_merges.get(&section).copied().unwrap_or(section);
        let name = self.area_renames.get(&id).or_else(|| self.areas.get(&id))?;
        Some(crate::Area { id, name: name.clone() })
    }

    /// Every Area of the game, once each, by section: merged sections count
    /// as the one they merge into.
    pub fn all_areas(&self) -> Vec<crate::Area> {
        self.areas
            .keys()
            .filter(|section| !self.area_merges.contains_key(section))
            .filter_map(|section| self.area(*section))
            .collect()
    }
}

/// A Game Family the profiles cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Family {
    /// FireRed and LeafGreen.
    #[serde(rename = "FRLG")]
    Frlg,
    /// Ruby, Sapphire and Emerald.
    #[serde(rename = "RSE")]
    Rse,
}

/// How far a Game Profile has been tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportStatus {
    /// Every acceptance script passed on this exact ROM.
    Supported,
    /// Bundled, but not every acceptance script has passed yet.
    Untested,
}

/// A byte at an address, of which only the bits in `mask` count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bit {
    /// The byte's address.
    pub at: u32,
    /// The bits of it that count.
    pub mask: u8,
}

/// Where a block of the game's memory is: `{ pointer = … }` for one the
/// game moves and points to, `{ at = … }` for one that stays put.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Place {
    /// The address of the game's pointer to the block.
    Pointer(u32),
    /// The block's own address.
    At(u32),
}

/// Where the game keeps what the rules engine reads.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Symbols {
    /// The Player's party.
    pub party: u32,
    /// The number of Pokémon in the party.
    pub party_count: u32,
    /// The opponent's party, filled in before a battle starts.
    pub enemy_party: u32,
    /// The PC's storage: behind a pointer in FireRed, LeafGreen and
    /// Emerald, which move it; fixed in Ruby and Sapphire.
    pub pc_storage: Place,
    /// Save block 1 (the Bag, event flags, the Player's position), placed as the PC is.
    pub save_block_1: Place,
    /// Save block 2 (trainer ID, play time, options), placed as the PC is.
    pub save_block_2: Place,
    /// pokeemerald-expansion's save block 3, where Heart and Soul keeps its
    /// challenge settings.
    #[serde(default)]
    pub save_block_3: Option<Place>,
    /// The region-map section the Player is in (a byte).
    pub region_map_section: u32,
    /// The game's main callback pointer (`gMain.callback2`).
    pub main_callback2: u32,
    /// The callback `main_callback2` holds on the overworld.
    pub overworld_callback: u32,
    /// The bit that is set while a battle is on.
    pub in_battle: Bit,
    /// The battle's type flags (`gBattleTypeFlags`).
    pub battle_type_flags: u32,
    /// The battlers (`gBattleMons`).
    pub battle_mons: u32,
    /// Each battler's party slot (`gBattlerPartyIndexes`).
    pub battler_party_indexes: u32,
    /// Each battler's status word that holds Perish Song (`gStatuses3`).
    pub battler_statuses3: u32,
    /// Each battler's disable struct (`gDisableStructs`), which holds the Perish count.
    pub disable_structs: u32,
    /// The script context's status (a byte).
    pub script_context_status: u32,
    /// The script context a running script uses.
    pub script_context: u32,
    /// The species info table, one entry per species.
    pub species_info: u32,
    /// The species names table, one fixed-length name per species.
    pub species_names: u32,
    /// The evolution table, a fixed number of entries per species.
    pub evolution_table: u32,
    /// The item table, one entry per item.
    pub items: u32,
    /// The trainer table, one entry per trainer id.
    pub trainers: u32,
    /// The type names table, one fixed-length name per type; or none, for a
    /// game that keeps none as text, which gives `type_name_list` instead.
    #[serde(default)]
    pub type_names: Option<u32>,
    /// Where the game expands the message it is showing (`gStringVar4`).
    pub message_buffer: u32,
    /// The Bag's Poké Balls pocket, for a game that keeps the Bag in RAM
    /// outside save block 1 (Gaia); otherwise `layout.balls_pocket` in save block 1.
    #[serde(default)]
    pub balls_pocket: Option<u32>,
    /// Where each PC box starts, for a game that spreads its boxes over RAM
    /// (CFRU's 25); otherwise the boxes follow one another from `pc_storage`.
    #[serde(default)]
    pub pc_box_starts: Option<Vec<u32>>,
    /// Event flags the game keeps outside save block 1 (CFRU's expanded flags in Unbound).
    #[serde(default)]
    pub expanded_flags: Option<ExpandedFlags>,
}

/// Flags from `first` on, kept a bit per flag from `at` rather than after the
/// game's own flags in save block 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpandedFlags {
    /// The first flag kept there.
    pub first: u16,
    /// Where that first flag's bit is.
    pub at: u32,
}

/// The size of CFRU's compressed PC slot.
pub const COMPRESSED_SLOT_SIZE: u32 = 58;

/// How a PC slot holds a Pokémon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoxFormat {
    /// The party form's first `box_slot_size` bytes (pret's BoxPokemon).
    #[default]
    Full,
    /// CFRU's 58-byte CompressedPokemon: no checksum, PP, contest stats or
    /// ribbons, moves packed in 10 bits each, never encrypted.
    Compressed,
}

/// How Pokémon, battlers, items and the PC are laid out.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// A party Pokémon's size in bytes.
    pub party_slot_size: u32,
    /// A PC Pokémon's size in bytes.
    pub box_slot_size: u32,
    /// How a PC slot holds a Pokémon: the party form's start (the default) or CFRU's compressed form.
    #[serde(default)]
    pub box_format: BoxFormat,
    /// Whether the four substructures are shuffled and XOR-encrypted.
    pub encrypted: bool,
    /// The bits of the species field that are the species (pokeemerald-expansion
    /// packs the tera type beside it); all of them by default.
    #[serde(default = "every_bit")]
    pub species_mask: u16,
    /// A Pokémon in a plain form of the game's own, with no substructures or
    /// checksum (R.O.W.E.): where its fields are. A slot holds one when its
    /// species isn't 0.
    #[serde(default)]
    pub plain: Option<PlainPokemon>,
    /// In a Pokémon: its original trainer's ID.
    pub ot_id: u32,
    /// In a Pokémon: its nickname.
    pub nickname: u32,
    /// A nickname's length in bytes.
    pub nickname_length: u32,
    /// In a Pokémon: the byte holding its egg and has-species bits.
    pub flags: u32,
    /// The bits of `flags` that mark an egg.
    pub egg_mask: u8,
    /// The bits of `flags` that mark a slot holding a Pokémon.
    pub has_species_mask: u8,
    /// In a Pokémon: the checksum of its substructures.
    pub checksum: u32,
    /// In a Pokémon: where its four substructures start.
    pub substructures: u32,
    /// In a party Pokémon: its level.
    pub level: u32,
    /// In a party Pokémon: its HP now.
    pub hp: u32,
    /// In a party Pokémon: its HP when full.
    pub max_hp: u32,
    /// A battler's size in bytes.
    pub battle_mon_size: u32,
    /// In a battler: its HP now.
    pub battle_mon_hp: u32,
    /// The battle type flags of a trainer battle.
    pub battle_type_trainer: u32,
    /// The battle type flags of a double battle.
    pub battle_type_double: u32,
    /// Battle types nobody can catch in (catching demos, ghosts): no one's Encounter.
    pub battle_type_uncatchable: u32,
    /// The bits of a battler's status word that Perish Song sets.
    pub perish_song_status3: u32,
    /// How far apart each battler's status word is from `battler_statuses3`:
    /// `gStatuses3`'s 4, or the battler's size where it keeps its own
    /// volatiles (pokeemerald-expansion).
    #[serde(default = "status_word")]
    pub battler_status_stride: u32,
    /// A disable struct's size in bytes.
    pub disable_struct_size: u32,
    /// In a disable struct: the byte holding the Perish count.
    pub perish_timer: u32,
    /// The bits of the byte at `perish_timer` that are the Perish count.
    #[serde(default = "every_bit_of_a_byte")]
    pub perish_timer_mask: u8,
    /// The current script instruction's offset in the script context.
    pub script_context_pointer: u32,
    /// The script context's call depth (a byte) and its stack of return addresses.
    pub script_context_stack_depth: u32,
    /// In the script context: its stack of return addresses.
    pub script_context_stack: u32,
    /// One species info entry's size in bytes.
    pub species_info_size: u32,
    /// One species name's fixed length in the species names table.
    pub species_name_length: u32,
    /// Species in the species tables, counting species 0.
    pub species_count: u16,
    /// Entries per species in the evolution table.
    pub evolutions_per_species: u32,
    /// Where each species' info points at its own list of evolutions, ended
    /// by method 0xFFFF (pokeemerald-expansion), instead of `evolution_table`.
    #[serde(default)]
    pub evolutions_pointer: Option<u32>,
    /// One evolution entry: its method (0 for none) first, then the target species at `evolution_target`.
    pub evolution_size: u32,
    /// In an evolution entry: the species it evolves into.
    pub evolution_target: u32,
    /// In a species info entry: its primary type.
    pub primary_type: u32,
    /// One item table entry's size in bytes.
    pub item_size: u32,
    /// Items in the item table.
    pub item_count: u32,
    /// In an item entry: the Bag pocket it goes in.
    pub item_pocket: u32,
    /// In an item entry: its use in battle.
    pub item_battle_usage: u32,
    /// The Poké Balls pocket's number.
    pub ball_pocket: u8,
    /// How many boxes the PC has.
    pub pc_boxes: u8,
    /// Slots in each PC box.
    pub pc_box_slots: u8,
    /// In the PC's storage: where the first box starts.
    pub pc_boxes_offset: u32,
    /// In save block 2.
    pub trainer_id: u32,
    /// In save block 2: play time as hours (two bytes), minutes, seconds and frames.
    pub play_time: u32,
    /// The first of the eight badge event flags (`FLAG_BADGE01_GET`), one per badge.
    pub badge_flags: u16,
    /// In save block 1: the event flags, a bit per flag.
    pub event_flags: u32,
    /// One trainer table entry's size in bytes.
    pub trainer_size: u32,
    /// In a trainer entry: its party flags (a byte), party size and party pointer.
    pub trainer_party_flags: u32,
    /// In a trainer entry: how many Pokémon its party has.
    pub trainer_party_size: u32,
    /// In a trainer entry: the pointer to its party.
    pub trainer_party: u32,
    /// In a trainer's party entry: its level (two bytes).
    pub trainer_mon_level: u32,
    /// A trainer's party entry size for each value of its party flags (0 to 3).
    pub trainer_mon_sizes: Vec<u32>,
    /// One type name's fixed length in the type names table.
    pub type_name_length: u32,
    /// In save block 1: the Bag's Poké Balls pocket, 4 bytes a slot (item, quantity).
    pub balls_pocket: u32,
    /// Slots in the Poké Balls pocket.
    pub balls_pocket_slots: u8,
    /// The PC box (counting from 1) that holds the Dead.
    pub dead_box: u8,
}

/// Where a plain Pokémon's fields are, from its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlainPokemon {
    /// A halfword holding the species in `species_mask`'s bits.
    pub species: u32,
    /// A byte: the region-map section it was met in.
    pub met_location: u32,
}

fn every_bit() -> u16 {
    u16::MAX
}

fn every_bit_of_a_byte() -> u8 {
    u8::MAX
}

fn status_word() -> u32 {
    4
}

/// One of the game's own options, such as the battle style.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionDef {
    /// Its in-game name and where to find it, for Run setup.
    pub name: String,
    /// Where it is in the game's menus.
    pub menu: String,
    /// 1, 2 or 3: which save block holds it.
    pub save_block: u8,
    /// Its byte's offset in that save block.
    pub offset: u32,
    /// The bits of that byte that hold it.
    pub mask: u8,
    /// Its settings by name, as the masked byte.
    pub values: BTreeMap<String, u8>,
    /// Whether a change mid-Run is caught every frame, or only checked at the lock.
    #[serde(default)]
    pub watched: bool,
}

/// The enforcement method each game-dependent rule uses, if the game has one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleMethods {
    /// Refuses ball throws in a closed Area.
    pub catch_block: Option<Method>,
    /// The level-100 checks, or a function stubbed (by its `write`) to
    /// return the cap; the first byte of each site is the cap, less its `below_cap`.
    pub level_cap: Option<Method>,
    /// Refuses the Bag in trainer battles.
    pub no_items_trainer: Option<Method>,
    /// Leaves only balls a use in wild battles.
    pub no_items_wild: Option<Method>,
    /// Keeps the battle style on Set.
    pub set_mode: Option<Method>,
    /// Keeps the nickname prompt's answer on YES.
    pub must_nickname: Option<Method>,
}

/// The fixed vocabulary of enforcement methods.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Method {
    /// ROM bytes checked at load, then rewritten while the rule applies.
    Patch {
        /// Every place patched.
        sites: Vec<PatchSite>,
    },
    /// A field of every entry in a ROM table zeroed (the item table's battle use).
    ZeroTableField,
    /// One of the game's options forced to a value by Pactlocke.
    OptionBit {
        /// The option's key in the Game Profile.
        option: String,
        /// The setting the rule needs.
        value: String,
    },
    /// One of the game's own event flags, set while the rule applies
    /// (Heart and Soul's `FLAG_NO_WILD_CATCHING`).
    SetFlag {
        /// The event flag's number.
        flag: u16,
    },
    /// Buttons filtered while a battle script command waits for an answer.
    KeyFilter {
        /// Where the game keeps the battle script instruction it is at.
        battle_script_pointer: u32,
        /// The battle script command table.
        command_table: u32,
        /// The command that draws the yes/no box and waits on it.
        command: u32,
        /// The command's state byte; the box's cursor is the byte after it.
        battle_communication: u32,
        /// Script positions where a field yes/no box waits for its answer.
        #[serde(default)]
        field_yes_no: Vec<u32>,
        /// Words that mark the field yes/no question this filter answers.
        #[serde(default)]
        question: Option<String>,
    },
    /// The game's own option does it; the Player turns it on and the lock checks it.
    Native {
        /// The option's key in the Game Profile.
        option: String,
        /// The setting the rule needs.
        value: String,
    },
    /// The game always plays this way (Radical Red's Set battle style): nothing to do.
    Always,
}

/// One place in the ROM a method patches.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSite {
    /// The site's address.
    pub at: u32,
    /// What a clean ROM holds there.
    pub expect: HexBytes,
    /// What Pactlocke writes there while the rule applies, where it is fixed.
    #[serde(default)]
    pub write: Option<HexBytes>,
    /// For a level cap site: how far below the cap its first byte is written
    /// (1 where the game checks `level > 99`, as CFRU's EXP code does).
    #[serde(default)]
    pub below_cap: u8,
}

/// Bytes written in a profile as hex pairs, such as `"10 B5 00 24"`.
#[derive(Clone, PartialEq, Eq)]
pub struct HexBytes(pub Vec<u8>);

impl fmt::Display for HexBytes {
    /// `10 B5 00 24`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex: Vec<String> = self.0.iter().map(|b| format!("{b:02X}")).collect();
        f.write_str(&hex.join(" "))
    }
}

impl fmt::Debug for HexBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{self}\"")
    }
}

impl<'de> Deserialize<'de> for HexBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = text
            .split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).map_err(|_| format!("{pair:?} is not a hex byte")))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(serde::de::Error::custom)?;
        if bytes.is_empty() {
            return Err(serde::de::Error::custom("no bytes given"));
        }
        Ok(HexBytes(bytes))
    }
}

/// Reads a table whose keys are section ids such as `0x58`.
fn hex_keys<'de, D, V>(deserializer: D) -> Result<BTreeMap<u16, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    let raw = BTreeMap::<String, V>::deserialize(deserializer)?;
    raw.into_iter()
        .map(|(key, value)| {
            let id = key
                .strip_prefix("0x")
                .and_then(|hex| u16::from_str_radix(hex, 16).ok())
                .ok_or_else(|| serde::de::Error::custom(format!("{key:?} is not a section id like 0x58")))?;
            Ok((id, value))
        })
        .collect()
}
