//! The real game interface: driven only by Game Profile data, over a game's
//! memory (normally the core adapter).

use std::collections::HashMap;
use std::sync::Arc;

use crate::caps::MAX_LEVEL;
use crate::compressed;
use crate::game::{
    Actions, Badges, Battle, BattleKind, Battler, Change, Game, GameError, Memory, PartyStats, Pokemon, ScriptContext,
    ScriptStatus, Side, StoredPokemon,
};
use crate::profile::HOLD_SCRIPT_LEN;
use crate::profile::{BoxFormat, GameProfile, Method, PatchSite, Place, ROM_BASE, RngPin, SeedVar};
use crate::{Area, Source, text};
use crate::{AvatarRoom, OAM, OBJ_PALETTES, OBJ_TILES, SpriteImage, Whereabouts};

/// The order of the four substructures (Growth, Attacks, EVs, Misc) for each
/// PID mod 24.
const SUBSTRUCTURE_ORDERS: [&[u8; 4]; 24] = [
    b"GAEM", b"GAME", b"GEAM", b"GEMA", b"GMAE", b"GMEA", b"AGEM", b"AGME", b"AEGM", b"AEMG", b"AMGE", b"AMEG",
    b"EGAM", b"EGMA", b"EAGM", b"EAMG", b"EMGA", b"EMAG", b"MGAE", b"MGEA", b"MAGE", b"MAEG", b"MEGA", b"MEAG",
];
const SUBSTRUCTURE_SIZE: usize = 12;
const PARTY_SIZE: u8 = 6;

/// How much of the message buffer is searched for a question's words.
const MESSAGE_LENGTH: usize = 0x200;

/// A game read and steered through its Game Profile.
pub struct ProfileGame<M> {
    profile: Arc<GameProfile>,
    /// The clean ROM, for undoing table patches.
    rom: Arc<[u8]>,
    memory: M,
    /// The last Pokémon read whole at each party and PC address. A frame can
    /// end while the game has one decrypted in place or half written; the
    /// Pokémon there is then the one last read whole.
    last_whole: HashMap<u32, Pokemon>,
    /// The trainer battle whose RNG is pinned for Shared Seed, if one is coming.
    pin: Option<Pinned>,
}

impl<M: Memory> ProfileGame<M> {
    /// A game on `memory`, read through `profile`, with its clean `rom`.
    pub fn new(profile: Arc<GameProfile>, rom: impl Into<Arc<[u8]>>, memory: M) -> Self {
        Self {
            profile,
            rom: rom.into(),
            memory,
            last_whole: HashMap::new(),
            pin: None,
        }
    }

    /// The clean ROM, as loaded.
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// The memory the game is read from.
    pub fn memory(&self) -> &M {
        &self.memory
    }

    /// The memory the game is read from, to change.
    pub fn memory_mut(&mut self) -> &mut M {
        &mut self.memory
    }

    fn bytes(&mut self, addr: u32, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        self.memory.read(addr, &mut out);
        out
    }

    fn u8(&mut self, addr: u32) -> u8 {
        self.bytes(addr, 1)[0]
    }

    fn u16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes(self.bytes(addr, 2).try_into().unwrap())
    }

    fn u32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.bytes(addr, 4).try_into().unwrap())
    }

    fn save_block(&mut self, number: u8) -> u32 {
        let symbols = &self.profile.symbols;
        let place = match number {
            1 => symbols.save_block_1,
            2 => symbols.save_block_2,
            // The store checked a profile with options in block 3 places it.
            _ => symbols.save_block_3.expect("save block 3 is placed"),
        };
        self.place(place)
    }

    /// Where a block is now: read through its pointer, or where it always is.
    fn place(&mut self, place: Place) -> u32 {
        match place {
            Place::Pointer(pointer) => self.u32(pointer),
            Place::At(addr) => addr,
        }
    }

    /// An event flag's byte and its bit there.
    fn flag_bit(&mut self, flag: u16) -> (u32, u8) {
        let (flags, index) = match self.profile.symbols.expanded_flags {
            Some(expanded) if flag >= expanded.first => (expanded.at, flag - expanded.first),
            _ => (self.save_block(1) + self.profile.layout.event_flags, flag),
        };
        (flags + u32::from(index / 8), 1 << (index % 8))
    }

    /// Whether an event flag is set.
    fn flag(&mut self, flag: u16) -> bool {
        let (byte, bit) = self.flag_bit(flag);
        self.u8(byte) & bit != 0
    }

    /// The write that sets or clears an event flag.
    fn set_flag(&mut self, flag: u16, on: bool) -> Change {
        let (byte, bit) = self.flag_bit(flag);
        let old = self.u8(byte);
        Change::Write {
            addr: byte,
            bytes: vec![if on { old | bit } else { old & !bit }],
        }
    }

    fn party_count(&mut self) -> u8 {
        self.u8(self.profile.symbols.party_count).min(PARTY_SIZE)
    }

    fn party_slot_addr(&self, slot: u8) -> u32 {
        self.profile.symbols.party + u32::from(slot) * self.profile.layout.party_slot_size
    }

    fn box_slot_addr(&mut self, box_index: u8, slot_index: u8) -> u32 {
        let layout = &self.profile.layout;
        let size = layout.box_slot_size;
        // The store checked there is a start for every box.
        if let Some(starts) = &self.profile.symbols.pc_box_starts {
            return starts[usize::from(box_index)] + u32::from(slot_index) * size;
        }
        let index = u32::from(box_index) * u32::from(layout.pc_box_slots) + u32::from(slot_index);
        let offset = layout.pc_boxes_offset;
        self.place(self.profile.symbols.pc_storage) + offset + index * size
    }

    /// A party Pokémon's bytes as a PC slot holds them.
    fn box_slot_bytes(&self, party: &[u8]) -> Vec<u8> {
        let layout = &self.profile.layout;
        match layout.box_format {
            BoxFormat::Full => party[..layout.box_slot_size as usize].to_vec(),
            BoxFormat::Compressed => compressed::compress(party, layout.substructures as usize),
        }
    }

    /// A Pokémon in CFRU's compressed PC slot at `addr`.
    fn compressed_pokemon(&mut self, addr: u32) -> Pokemon {
        let layout = self.profile.layout.clone();
        let raw = self.bytes(addr, layout.box_slot_size as usize);
        let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        let nickname = layout.nickname as usize;
        Pokemon {
            pid: word(0),
            ot_id: word(layout.ot_id as usize),
            species: u16::from_le_bytes([raw[compressed::SPECIES], raw[compressed::SPECIES + 1]]),
            nickname: text::decode(&raw[nickname..nickname + layout.nickname_length as usize]),
            met_section: u16::from(raw[compressed::MET_LOCATION]),
            is_egg: raw[layout.flags as usize] & layout.egg_mask != 0,
            stats: None,
        }
    }

    fn has_species(&mut self, addr: u32) -> bool {
        let layout = &self.profile.layout;
        if let Some(plain) = layout.plain {
            let mask = layout.species_mask;
            return self.u16(addr + plain.species) & mask != 0;
        }
        let (flags, mask) = (layout.flags, layout.has_species_mask);
        self.u8(addr + flags) & mask != 0
    }

    fn in_battle(&mut self) -> bool {
        let bit = self.profile.symbols.in_battle;
        self.u8(bit.at) & bit.mask != 0
    }

    /// The Pokémon at `addr` if it reads whole, else the one last read whole
    /// there, if any.
    fn whole_or_last(&mut self, addr: u32, in_party: bool) -> Option<Pokemon> {
        match self.decode(addr, in_party) {
            (pokemon, true) => {
                self.last_whole.insert(addr, pokemon.clone());
                Some(pokemon)
            }
            (_, false) => self.last_whole.get(&addr).cloned(),
        }
    }

    /// Reads a Pokémon at `addr`; party Pokémon also carry their stats. Says
    /// too whether it read whole: an encrypted one's checksum matches, as
    /// stored or as the game left it decrypted in place mid-read. Otherwise
    /// the frame ended while the game was writing it.
    fn decode(&mut self, addr: u32, in_party: bool) -> (Pokemon, bool) {
        let layout = self.profile.layout.clone();
        if !in_party && layout.box_format == BoxFormat::Compressed {
            return (self.compressed_pokemon(addr), true);
        }
        let size = if in_party {
            layout.party_slot_size
        } else {
            layout.box_slot_size
        };
        let raw = self.bytes(addr, size as usize);
        let at = |offset: u32| offset as usize;
        let word = |offset: u32| u32::from_le_bytes(raw[at(offset)..at(offset) + 4].try_into().unwrap());
        let pid = word(0);
        let ot_id = word(layout.ot_id);
        let nickname_start = at(layout.nickname);
        let nickname = text::decode(&raw[nickname_start..nickname_start + layout.nickname_length as usize]);
        let is_egg = raw[at(layout.flags)] & layout.egg_mask != 0;
        let stats = in_party.then(|| PartyStats {
            level: raw[at(layout.level)],
            hp: u16::from_le_bytes([raw[at(layout.hp)], raw[at(layout.hp) + 1]]),
            max_hp: u16::from_le_bytes([raw[at(layout.max_hp)], raw[at(layout.max_hp) + 1]]),
        });

        if let Some(plain) = layout.plain {
            let species = u16::from_le_bytes([raw[at(plain.species)], raw[at(plain.species) + 1]]);
            let pokemon = Pokemon {
                pid,
                ot_id,
                species: species & layout.species_mask,
                nickname,
                met_section: u16::from(raw[at(plain.met_location)]),
                is_egg,
                stats,
            };
            return (pokemon, true);
        }

        // The four substructures, in Growth, Attacks, EVs, Misc order.
        let mut subs = [[0u8; SUBSTRUCTURE_SIZE]; 4];
        let order: &[u8; 4] = if layout.encrypted {
            SUBSTRUCTURE_ORDERS[(pid % 24) as usize]
        } else {
            b"GAEM"
        };
        let stored_sum = u16::from_le_bytes([raw[at(layout.checksum)], raw[at(layout.checksum) + 1]]);
        let mut decrypt = |key: u32| {
            for (position, kind) in order.iter().enumerate() {
                let which = b"GAEM".iter().position(|k| k == kind).unwrap();
                let start = at(layout.substructures) + position * SUBSTRUCTURE_SIZE;
                for (i, chunk) in raw[start..start + SUBSTRUCTURE_SIZE].chunks(4).enumerate() {
                    let plain = u32::from_le_bytes(chunk.try_into().unwrap()) ^ key;
                    subs[which][i * 4..i * 4 + 4].copy_from_slice(&plain.to_le_bytes());
                }
            }
            subs
        };
        let (subs, whole) = if layout.encrypted {
            let stored = decrypt(pid ^ ot_id);
            if checksum(&stored) == stored_sum {
                (stored, true)
            } else {
                let in_place = decrypt(0);
                let whole = checksum(&in_place) == stored_sum;
                (if whole { in_place } else { stored }, whole)
            }
        } else {
            (decrypt(0), true)
        };
        let [growth, _, _, misc] = subs;
        let pokemon = Pokemon {
            pid,
            ot_id,
            species: u16::from_le_bytes([growth[0], growth[1]]) & layout.species_mask,
            nickname,
            met_section: u16::from(misc[1]),
            is_egg,
            stats,
        };
        (pokemon, whole)
    }

    fn patch_sites(&self, rule: &'static str, method: Option<&Method>) -> Result<Vec<PatchSite>, GameError> {
        match method {
            Some(Method::Patch { sites }) => Ok(sites.clone()),
            _ => Err(GameError::NoMethod(rule)),
        }
    }

    /// Each site's fixed patch while `on`, its clean bytes while off.
    fn toggle(&self, rule: &'static str, method: Option<&Method>, on: bool) -> Actions {
        self.patch_sites(rule, method)?
            .into_iter()
            .map(|site| {
                let bytes = if on {
                    site.write.ok_or(GameError::NoMethod(rule))?.0
                } else {
                    site.expect.0
                };
                Ok(Change::Patch { addr: site.at, bytes })
            })
            .collect()
    }

    /// The party slot's bytes, and the writes that take it out of the party:
    /// the slots after it move up and the count goes down.
    fn take_from_party(&mut self, party_slot: u8) -> (Vec<u8>, Vec<Change>) {
        let count = self.party_count();
        let slot_size = self.profile.layout.party_slot_size as usize;
        let moved_from = self.party_slot_addr(party_slot);
        let mut rest = self.bytes(moved_from, slot_size * usize::from(count - party_slot));
        let moved: Vec<u8> = rest.drain(..slot_size).collect();
        rest.resize(rest.len() + slot_size, 0);
        let changes = vec![
            Change::Write {
                addr: moved_from,
                bytes: rest,
            },
            Change::Write {
                addr: self.profile.symbols.party_count,
                bytes: vec![count - 1],
            },
        ];
        (moved, changes)
    }

    /// Must nickname's key filter, from the Game Profile.
    fn nickname_filter(&self) -> Option<NicknameFilter> {
        match self.profile.rules.must_nickname.clone()? {
            Method::KeyFilter {
                battle_script_pointer,
                command_table,
                command,
                battle_communication,
                field_yes_no,
                question,
            } => Some(NicknameFilter {
                battle_script_pointer,
                command_table,
                command,
                battle_communication,
                field_yes_no,
                question,
            }),
            _ => None,
        }
    }

    fn check_party_slot(&mut self, slot: u8) -> Result<(), GameError> {
        if slot < self.party_count() {
            Ok(())
        } else {
            Err(GameError::NoSuchSlot(slot))
        }
    }
}

impl<M: Memory> Game for ProfileGame<M> {
    fn area(&mut self) -> Option<Area> {
        let section = self.u8(self.profile.symbols.region_map_section);
        self.profile.area(u16::from(section))
    }

    fn party(&mut self) -> Vec<Pokemon> {
        (0..self.party_count())
            .map(|slot| {
                let addr = self.party_slot_addr(slot);
                self.whole_or_last(addr, true)
                    .unwrap_or_else(|| self.decode(addr, true).0)
            })
            .collect()
    }

    fn pc(&mut self) -> Vec<StoredPokemon> {
        let (boxes, slots) = (self.profile.layout.pc_boxes, self.profile.layout.pc_box_slots);
        let mut stored = Vec::new();
        for box_index in 0..boxes {
            for slot_index in 0..slots {
                let addr = self.box_slot_addr(box_index, slot_index);
                if !self.has_species(addr) {
                    continue;
                }
                // A slot with the flag but no species holds nothing: Emerald
                // leaves its last one so for a frame while it moves its save data.
                let Some(pokemon) = self.whole_or_last(addr, false) else {
                    continue;
                };
                if pokemon.species != 0 {
                    stored.push(StoredPokemon {
                        box_number: box_index + 1,
                        slot: slot_index + 1,
                        pokemon,
                    });
                }
            }
        }
        stored
    }

    fn battle(&mut self) -> Option<Battle> {
        if !self.in_battle() {
            return None;
        }
        let (symbols, layout) = (self.profile.symbols.clone(), self.profile.layout.clone());
        let flags = self.u32(symbols.battle_type_flags);
        let double = flags & layout.battle_type_double != 0;
        let battlers = (0..if double { 4 } else { 2 })
            .map(|position: u8| {
                let mon = symbols.battle_mons + u32::from(position) * layout.battle_mon_size;
                let side = if position.is_multiple_of(2) {
                    Side::Player
                } else {
                    Side::Opponent
                };
                let party_slot =
                    (side == Side::Player).then(|| self.u8(symbols.battler_party_indexes + u32::from(position) * 2));
                Battler {
                    position,
                    side,
                    species: self.u16(mon),
                    hp: self.u16(mon + layout.battle_mon_hp),
                    party_slot,
                }
            })
            .collect();
        Some(Battle {
            kind: if flags & layout.battle_type_uncatchable != 0 {
                BattleKind::Uncatchable
            } else if flags & layout.battle_type_trainer != 0 {
                BattleKind::Trainer
            } else {
                BattleKind::Wild
            },
            double,
            battlers,
        })
    }

    fn opponent_party(&mut self) -> Vec<Pokemon> {
        let slot_size = self.profile.layout.party_slot_size;
        let base = self.profile.symbols.enemy_party;
        let mut party = Vec::new();
        for slot in 0..u32::from(PARTY_SIZE) {
            let addr = base + slot * slot_size;
            // Not the last one read whole: that may be the last battle's.
            if self.has_species(addr)
                && let (pokemon, true) = self.decode(addr, true)
            {
                party.push(pokemon);
            }
        }
        party
    }

    fn script(&mut self) -> ScriptContext {
        let status = match self.u8(self.profile.symbols.script_context_status) {
            0 => ScriptStatus::Running,
            1 => ScriptStatus::Waiting,
            _ => ScriptStatus::Idle,
        };
        let position = (status != ScriptStatus::Idle)
            .then(|| self.u32(self.profile.symbols.script_context + self.profile.layout.script_context_pointer));
        ScriptContext { status, position }
    }

    fn script_calls(&mut self) -> Vec<u32> {
        /// The game's script stack holds 20 return addresses.
        const MAX_DEPTH: u8 = 20;
        if self.script().status == ScriptStatus::Idle {
            return Vec::new();
        }
        let (context, layout) = (self.profile.symbols.script_context, self.profile.layout.clone());
        let depth = self.u8(context + layout.script_context_stack_depth).min(MAX_DEPTH);
        (0..depth)
            .rev()
            .map(|i| self.u32(context + layout.script_context_stack + u32::from(i) * 4))
            .collect()
    }

    fn badges(&mut self) -> Badges {
        let first = self.profile.layout.badge_flags;
        Badges((0..8u16).fold(0, |bits, badge| bits | (u8::from(self.flag(first + badge)) << badge)))
    }

    fn cap_stage(&mut self) -> usize {
        let badges = self.badges();
        let profile = self.profile.clone();
        profile
            .level_caps
            .iter()
            .take_while(|stage| stage.passed(badges, |flag| self.flag(flag)))
            .count()
    }

    fn option(&mut self, name: &str) -> Option<String> {
        let def = self.profile.options.get(name)?.clone();
        let block = self.save_block(def.save_block);
        let value = self.u8(block + def.offset) & def.mask;
        def.values
            .iter()
            .find(|(_, v)| **v == value)
            .map(|(name, _)| name.clone())
    }

    fn trainer_id(&mut self) -> u32 {
        let block = self.save_block(2);
        self.u32(block + self.profile.layout.trainer_id)
    }

    fn play_time(&mut self) -> u32 {
        let at = self.save_block(2) + self.profile.layout.play_time;
        let (hours, minutes) = (u32::from(self.u16(at)), u32::from(self.u8(at + 2)));
        let (seconds, frames) = (u32::from(self.u8(at + 3)), u32::from(self.u8(at + 4)));
        ((hours * 60 + minutes) * 60 + seconds) * 60 + frames
    }

    fn primary_type(&mut self, species: u16) -> u8 {
        let layout = &self.profile.layout;
        let addr =
            self.profile.symbols.species_info + u32::from(species) * layout.species_info_size + layout.primary_type;
        self.u8(addr)
    }

    fn type_name(&mut self, type_id: u8) -> String {
        let Some(table) = self.profile.symbols.type_names else {
            return self
                .profile
                .type_name_list
                .get(usize::from(type_id))
                .cloned()
                .unwrap_or_else(|| format!("type {type_id}"));
        };
        let length = self.profile.layout.type_name_length;
        let addr = table + u32::from(type_id) * length;
        text::decode(&self.bytes(addr, length as usize)).trim_end().to_owned()
    }

    fn species_name(&mut self, species: u16) -> String {
        let length = self.profile.layout.species_name_length;
        let addr = self.profile.symbols.species_names + u32::from(species) * length;
        text::decode(&self.bytes(addr, length as usize))
    }

    fn evolutions(&mut self) -> Vec<Vec<u16>> {
        let layout = self.profile.layout.clone();
        if let Some(pointer) = layout.evolutions_pointer {
            /// What ends a species' list of evolutions.
            const END: u16 = 0xFFFF;
            let info = self.profile.symbols.species_info;
            return (0..u32::from(layout.species_count))
                .map(|species| {
                    let list = self.u32(info + species * layout.species_info_size + pointer);
                    if !(ROM_BASE..ROM_BASE + 0x0200_0000).contains(&list) {
                        return Vec::new();
                    }
                    (0..layout.evolutions_per_species)
                        .map(|i| list + i * layout.evolution_size)
                        .map(|entry| (self.u16(entry), self.u16(entry + layout.evolution_target)))
                        .take_while(|&(method, _)| method != END)
                        .map(|(_, into)| into)
                        .filter(|&into| into != 0 && into < layout.species_count)
                        .collect()
                })
                .collect();
        }
        let per_species = layout.evolutions_per_species * layout.evolution_size;
        let table = self.bytes(
            self.profile.symbols.evolution_table,
            usize::from(layout.species_count) * per_species as usize,
        );
        let half = |at: usize| u16::from_le_bytes([table[at], table[at + 1]]);
        (0..usize::from(layout.species_count))
            .map(|species| {
                (0..layout.evolutions_per_species as usize)
                    .map(|i| species * per_species as usize + i * layout.evolution_size as usize)
                    .filter(|&entry| half(entry) != 0)
                    .map(|entry| half(entry + layout.evolution_target as usize))
                    .filter(|&into| into != 0 && into < layout.species_count)
                    .collect()
            })
            .collect()
    }

    fn source_before(&mut self, position: u32) -> Option<Source> {
        crate::source::source_before(&self.profile.sources, position)
    }

    fn at_safe_overworld_point(&mut self) -> bool {
        let callback = self.u32(self.profile.symbols.main_callback2) & !1;
        callback == self.profile.symbols.overworld_callback & !1 && !self.in_battle()
    }

    fn nickname_prompt_up(&mut self) -> bool {
        let Some(NicknameFilter {
            battle_script_pointer,
            command_table,
            command,
            battle_communication,
            field_yes_no,
            question,
        }) = self.nickname_filter()
        else {
            return false;
        };
        if self.in_battle() {
            // The battle script's current byte is its command's index.
            let instruction = self.u32(battle_script_pointer);
            let index = self.u8(instruction);
            let running = self.u32(command_table + u32::from(index) * 4);
            // The command's first state draws the box, its second waits on it.
            return running == command && self.u8(battle_communication) == 1;
        }
        let script = self.script();
        let waiting = script.status == ScriptStatus::Waiting
            && script.position.is_some_and(|position| field_yes_no.contains(&position));
        let Some(question) = question.filter(|_| waiting && self.at_safe_overworld_point()) else {
            return false;
        };
        let Ok(mut word) = text::encode(&question, question.chars().count()) else {
            return false;
        };
        word.retain(|&b| b != 0xFF);
        let message = self.bytes(self.profile.symbols.message_buffer, MESSAGE_LENGTH);
        message.windows(word.len()).any(|w| w == word)
    }

    fn holds_poke_balls(&mut self) -> bool {
        let layout = &self.profile.layout;
        let (offset, slots) = (layout.balls_pocket, layout.balls_pocket_slots);
        let pocket = match self.profile.symbols.balls_pocket {
            Some(at) => at,
            None => self.save_block(1) + offset,
        };
        // The game empties a slot's item when its last ball is used.
        (0..u32::from(slots)).any(|slot| self.u16(pocket + slot * 4) != 0)
    }

    fn block_catches(&mut self, blocked: bool) -> Actions {
        if let Some(Method::SetFlag { flag }) = self.profile.rules.catch_block {
            return Ok(vec![self.set_flag(flag, blocked)]);
        }
        self.toggle("catch blocks", self.profile.rules.catch_block.as_ref(), blocked)
    }

    fn set_level_cap(&mut self, level: u8) -> Actions {
        if !(1..=MAX_LEVEL).contains(&level) {
            return Err(GameError::BadLevel(level));
        }
        Ok(self
            .patch_sites("level caps", self.profile.rules.level_cap.as_ref())?
            .into_iter()
            .map(|site| {
                // A `cmp #100` with the cap for its 100, or a function stubbed to return it.
                let mut bytes = site.write.unwrap_or(site.expect).0;
                bytes[0] = level.saturating_sub(site.below_cap);
                Change::Patch { addr: site.at, bytes }
            })
            .collect())
    }

    fn ban_items_in_wild_battles(&mut self, banned: bool) -> Actions {
        if !matches!(self.profile.rules.no_items_wild, Some(Method::ZeroTableField)) {
            return Err(GameError::NoMethod("no items in wild battles"));
        }
        let layout = &self.profile.layout;
        let table = self.profile.symbols.items;
        // The store checked the table is in ROM.
        let rom_byte = |addr: u32| self.rom.get((addr - ROM_BASE) as usize).copied().unwrap_or(0);
        Ok((1..layout.item_count)
            .filter_map(|item| {
                let entry = table + item * layout.item_size;
                let usage_addr = entry + layout.item_battle_usage;
                let usage = rom_byte(usage_addr);
                (rom_byte(entry + layout.item_pocket) != layout.ball_pocket && usage != 0).then(|| Change::Patch {
                    addr: usage_addr,
                    bytes: vec![if banned { 0 } else { usage }],
                })
            })
            .collect())
    }

    fn lock_bag_in_trainer_battles(&mut self, locked: bool) -> Actions {
        // The item table's ban (R.O.W.E.'s only one) bites in every battle.
        if matches!(self.profile.rules.no_items_trainer, Some(Method::ZeroTableField)) {
            return Ok(Vec::new());
        }
        self.toggle(
            "no items in trainer battles",
            self.profile.rules.no_items_trainer.as_ref(),
            locked,
        )
    }

    fn faint_active_battler(&mut self, position: u8) -> Actions {
        if !self.in_battle() {
            return Err(GameError::NotInBattle);
        }
        let count = self.battle().map_or(0, |battle| battle.battlers.len());
        if usize::from(position) >= count {
            return Err(GameError::NoSuchBattler(position));
        }
        let (symbols, layout) = (&self.profile.symbols, &self.profile.layout);
        let status_addr = symbols.battler_statuses3 + u32::from(position) * layout.battler_status_stride;
        let timer_addr =
            symbols.disable_structs + u32::from(position) * layout.disable_struct_size + layout.perish_timer;
        let (perish, timer_mask) = (layout.perish_song_status3, layout.perish_timer_mask);
        let status = self.u32(status_addr) | perish;
        let timer = self.u8(timer_addr) & !timer_mask;
        Ok(vec![
            Change::Write {
                addr: status_addr,
                bytes: status.to_le_bytes().to_vec(),
            },
            Change::Write {
                addr: timer_addr,
                bytes: vec![timer],
            },
        ])
    }

    fn faint_benched(&mut self, party_slot: u8) -> Actions {
        self.check_party_slot(party_slot)?;
        Ok(vec![Change::Write {
            addr: self.party_slot_addr(party_slot) + self.profile.layout.hp,
            bytes: vec![0, 0],
        }])
    }

    fn move_to_box(&mut self, party_slot: u8, box_number: u8) -> Actions {
        self.check_party_slot(party_slot)?;
        let layout = self.profile.layout.clone();
        if !(1..=layout.pc_boxes).contains(&box_number) {
            return Err(GameError::NoSuchBox(box_number));
        }
        let mut free = None;
        for slot in 0..layout.pc_box_slots {
            let addr = self.box_slot_addr(box_number - 1, slot);
            if !self.has_species(addr) {
                free = Some(addr);
                break;
            }
        }
        let free = free.ok_or(GameError::BoxFull(box_number))?;

        let (moved, mut changes) = self.take_from_party(party_slot);
        changes.insert(
            0,
            Change::Write {
                addr: free,
                bytes: self.box_slot_bytes(&moved),
            },
        );
        Ok(changes)
    }

    fn release_from_party(&mut self, party_slot: u8) -> Actions {
        self.check_party_slot(party_slot)?;
        Ok(self.take_from_party(party_slot).1)
    }

    fn release_from_pc(&mut self, box_number: u8, slot: u8) -> Actions {
        let layout = self.profile.layout.clone();
        if !(1..=layout.pc_boxes).contains(&box_number) {
            return Err(GameError::NoSuchBox(box_number));
        }
        if !(1..=layout.pc_box_slots).contains(&slot) {
            return Err(GameError::NoSuchSlot(slot));
        }
        let addr = self.box_slot_addr(box_number - 1, slot - 1);
        if !self.has_species(addr) {
            return Err(GameError::NoSuchSlot(slot));
        }
        Ok(vec![Change::Write {
            addr,
            bytes: vec![0; layout.box_slot_size as usize],
        }])
    }

    fn dead_box(&mut self) -> u8 {
        self.profile.layout.dead_box
    }

    fn move_to_dead_box(&mut self, party_slot: u8) -> Actions {
        self.move_to_box(party_slot, self.profile.layout.dead_box)
    }

    fn write_nickname(&mut self, party_slot: u8, name: &str) -> Actions {
        self.check_party_slot(party_slot)?;
        let layout = &self.profile.layout;
        let bytes = text::encode(name, layout.nickname_length as usize).map_err(GameError::BadName)?;
        Ok(vec![Change::Write {
            addr: self.party_slot_addr(party_slot) + layout.nickname,
            bytes,
        }])
    }

    fn write_pc_nickname(&mut self, box_number: u8, slot: u8, name: &str) -> Actions {
        let layout = self.profile.layout.clone();
        if !(1..=layout.pc_boxes).contains(&box_number) {
            return Err(GameError::NoSuchBox(box_number));
        }
        if !(1..=layout.pc_box_slots).contains(&slot) {
            return Err(GameError::NoSuchSlot(slot));
        }
        let addr = self.box_slot_addr(box_number - 1, slot - 1);
        if !self.has_species(addr) {
            return Err(GameError::NoSuchSlot(slot));
        }
        let bytes = text::encode(name, layout.nickname_length as usize).map_err(GameError::BadName)?;
        Ok(vec![Change::Write {
            addr: addr + layout.nickname,
            bytes,
        }])
    }

    fn keep_nickname_cursor_on_yes(&mut self) -> Actions {
        let filter = self.nickname_filter().ok_or(GameError::NoMethod("must nickname"))?;
        let cursor = filter.battle_communication + 1;
        // The field's YES/NO box opens on YES, and DOWN is what moves it.
        if !self.in_battle() || self.u8(cursor) == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![Change::Write {
            addr: cursor,
            bytes: vec![0],
        }])
    }

    fn force_set_mode(&mut self) -> Actions {
        let (option, value) = match self.profile.rules.set_mode.clone() {
            Some(Method::OptionBit { option, value }) => (option, value),
            Some(Method::Always) => return Ok(Vec::new()),
            _ => return Err(GameError::NoMethod("set mode")),
        };
        if self.option(&option).as_deref() == Some(value.as_str()) {
            return Ok(Vec::new());
        }
        self.set_option(&option, &value)
    }

    fn set_option(&mut self, name: &str, value: &str) -> Actions {
        let def = self
            .profile
            .options
            .get(name)
            .cloned()
            .ok_or_else(|| GameError::UnknownOption(name.to_owned()))?;
        let setting = *def
            .values
            .get(value)
            .ok_or_else(|| GameError::UnknownOption(format!("{name} = {value}")))?;
        let addr = self.save_block(def.save_block) + def.offset;
        let current = self.u8(addr);
        Ok(vec![Change::Write {
            addr,
            bytes: vec![(current & !def.mask) | setting],
        }])
    }

    fn native_options(&mut self) -> Vec<crate::NativeOption> {
        self.profile.native_options()
    }

    fn whereabouts(&mut self) -> Option<Whereabouts> {
        let avatars = self.profile.avatars.clone()?;
        if !self.at_safe_overworld_point() {
            return None;
        }
        let block = self.save_block(1);
        let at = |offset: u32| block.wrapping_add(offset);
        Some(Whereabouts {
            map: (self.u8(at(avatars.location)), self.u8(at(avatars.location + 1))),
            x: self.u16(at(avatars.position)) as i16,
            y: self.u16(at(avatars.position + 2)) as i16,
        })
    }

    fn player_sprite(&mut self) -> Option<SpriteImage> {
        let avatars = self.profile.avatars.clone()?;
        // The game's own Player sprite: 16x32 at the anchor, or 32x32 (a bike,
        // surfing) centred over it; surfing bobs it a few pixels up and down.
        for entry in 0..u32::from(avatars.first_oam) {
            let at = OAM + entry * 8;
            let (a0, a1, a2) = (self.u16(at), self.u16(at + 2), self.u16(at + 4));
            if a0 & 0x0300 == 0x0200 || a1 >> 14 != 2 {
                continue;
            }
            let square = match a0 >> 14 {
                0 => true,
                2 => false,
                _ => continue,
            };
            let x = a1 & 0x1FF;
            let y = a0 & 0xFF;
            let left = if square {
                avatars.anchor_x.wrapping_sub(8)
            } else {
                avatars.anchor_x
            };
            if x != left || y.abs_diff(avatars.anchor_y) > 8 {
                continue;
            }
            let tile = u32::from(a2 & 0x3FF);
            let palette = u32::from(a2 >> 12);
            let tiles = if square { 16 } else { 8 };
            return Some(SpriteImage {
                tiles: self.bytes(OBJ_TILES + tile * 32, tiles * 32),
                palette: self.bytes(OBJ_PALETTES + palette * 32, 32),
                hflip: a1 & 0x1000 != 0,
                priority: ((a2 >> 10) & 3) as u8,
                square,
            });
        }
        None
    }

    fn party_icon(&mut self, species: u16) -> Option<SpriteImage> {
        let avatars = self.profile.avatars.clone()?;
        let tiles_at = self.u32(avatars.icon_table + u32::from(species) * 4);
        if !(ROM_BASE..ROM_BASE + 0x0200_0000).contains(&tiles_at) {
            return None;
        }
        let index = u32::from(self.u8(avatars.icon_palette_indices + u32::from(species)));
        let palette_at = self.u32(avatars.icon_palette_table + index * 8);
        if !(ROM_BASE..ROM_BASE + 0x0200_0000).contains(&palette_at) {
            return None;
        }
        Some(SpriteImage {
            // The first of its two 32x32 frames.
            tiles: self.bytes(tiles_at, 16 * 32),
            palette: self.bytes(palette_at, 32),
            hflip: false,
            priority: 2,
            square: true,
        })
    }

    fn avatar_room(&mut self) -> Option<AvatarRoom> {
        let avatars = self.profile.avatars.as_ref()?;
        Some(AvatarRoom {
            anchor: (avatars.anchor_x, avatars.anchor_y),
            first_oam: avatars.first_oam,
            first_tile: avatars.first_tile,
            palettes: avatars.palettes.clone(),
        })
    }

    fn sync_gates(&mut self) -> Vec<String> {
        self.profile.sync_gate_names()
    }

    fn close_gate(&mut self, gate: &str) -> Actions {
        let gates = self
            .profile
            .sync_gates
            .as_ref()
            .ok_or(GameError::NoMethod("Sync Gates"))?;
        let (index, def) = gates
            .gates
            .iter()
            .enumerate()
            .find(|(_, g)| g.name == gate)
            .ok_or_else(|| GameError::NoSuchGate(gate.to_owned()))?;
        let script_at = gates.hold_script(index);
        let mut script = vec![0x0F, 0x00]; // loadword 0, the message
        script.extend_from_slice(&gates.hold.to_le_bytes());
        script.extend_from_slice(&[0x09, 0x04]); // callstd MSGBOX_DEFAULT
        script.extend_from_slice(&[0x6B, 0x02]); // releaseall, end
        let mut changes = vec![
            Change::Patch {
                addr: gates.hold,
                bytes: hold_message(),
            },
            Change::Patch {
                addr: script_at,
                bytes: script,
            },
        ];
        for site in &def.sites {
            let mut goto = vec![0x05]; // goto the hold script
            goto.extend_from_slice(&script_at.to_le_bytes());
            changes.push(Change::Patch {
                addr: site.at,
                bytes: goto,
            });
        }
        Ok(changes)
    }

    fn open_gate(&mut self, gate: &str) -> Actions {
        let gates = self
            .profile
            .sync_gates
            .as_ref()
            .ok_or(GameError::NoMethod("Sync Gates"))?;
        let def = gates
            .gates
            .iter()
            .find(|g| g.name == gate)
            .ok_or_else(|| GameError::NoSuchGate(gate.to_owned()))?;
        Ok(def
            .sites
            .iter()
            .map(|site| Change::Patch {
                addr: site.at,
                bytes: site.expect.0.clone(),
            })
            .collect())
    }

    fn at_gate(&mut self) -> Option<String> {
        let first = self.profile.sync_gates.as_ref()?.hold_script(0);
        let context = self.script();
        let mut places = self.script_calls();
        places.extend(context.position);
        let index = places
            .into_iter()
            .find_map(|at| Some(at.checked_sub(first)? as usize / HOLD_SCRIPT_LEN))?;
        let gates = self.profile.sync_gates.as_ref()?;
        gates.gates.get(index).map(|g| g.name.clone())
    }

    fn has_own_randomizer(&mut self) -> bool {
        self.profile.own_randomizer.is_some()
    }

    fn share_seed(&mut self, seed: u32) -> Actions {
        let own = self
            .profile
            .own_randomizer
            .clone()
            .ok_or(GameError::NoMethod("Shared Seed"))?;
        let mut changes = Vec::new();
        let mut write_if_changed = |game: &mut Self, addr: u32, bytes: Vec<u8>| {
            if game.bytes(addr, bytes.len()) != bytes {
                changes.push(Change::Write { addr, bytes });
            }
        };
        let trainer_id = self.save_block(2) + self.profile.layout.trainer_id;
        if own.seed_is_trainer_id {
            write_if_changed(self, trainer_id, seed.to_le_bytes().to_vec());
        }
        if let Some(byte) = own.trainer_id_byte {
            write_if_changed(self, trainer_id + u32::from(byte), vec![(seed >> 24) as u8]);
        }
        if let Some(SeedVar { var, vars }) = own.seed_var {
            // The game's own seeds run from 1 to 9998.
            let value = (1 + seed % 9998) as u16;
            let addr = self.save_block(1) + vars + u32::from(var.wrapping_sub(0x4000)) * 2;
            write_if_changed(self, addr, value.to_le_bytes().to_vec());
        }
        for site in own.sites {
            let write = site.write.ok_or(GameError::NoMethod("Shared Seed"))?.0;
            if self.bytes(site.at, write.len()) != write {
                changes.push(Change::Patch {
                    addr: site.at,
                    bytes: write,
                });
            }
        }
        for kept in own.keep {
            if self.option(&kept.option).as_deref() != Some(kept.value.as_str()) {
                changes.extend(self.set_option(&kept.option, &kept.value)?);
            }
        }
        Ok(changes)
    }

    fn pin_rng(&mut self, seed: u32) -> Actions {
        let Some(RngPin { rng, opponent }) = self.profile.own_randomizer.as_ref().and_then(|own| own.rng_pin) else {
            return Ok(Vec::new());
        };
        let trainer = self.u16(opponent);
        if trainer == 0 {
            self.pin = None;
            return Ok(Vec::new());
        }
        let party_word = self.u32(self.profile.symbols.enemy_party);
        let pin = match self.pin {
            Some(pin) if pin.trainer == trainer => pin,
            // Triggered: the enemy party as it is now, before the battle makes it.
            _ => *self.pin.insert(Pinned {
                trainer,
                party_word,
                made: false,
            }),
        };
        if pin.made || party_word != pin.party_word {
            self.pin = Some(Pinned { made: true, ..pin });
            return Ok(Vec::new());
        }
        Ok(vec![Change::Write {
            addr: rng,
            bytes: pinned_rng(seed, trainer).to_le_bytes().to_vec(),
        }])
    }
}

/// A trainer battle's RNG pin: which trainer, the enemy party's first word
/// when it was triggered, and whether the party has been made since.
#[derive(Debug, Clone, Copy)]
struct Pinned {
    trainer: u16,
    party_word: u32,
    made: bool,
}

/// The RNG state a pinned trainer battle starts from: the same in every
/// Player's game for the same seed and trainer.
pub fn pinned_rng(seed: u32, trainer: u16) -> u32 {
    seed.rotate_left(16) ^ u32::from(trainer).wrapping_mul(0x9E37_79B1)
}

/// Where must nickname's key filter looks, in battle and on the overworld.
struct NicknameFilter {
    battle_script_pointer: u32,
    command_table: u32,
    command: u32,
    battle_communication: u32,
    field_yes_no: Vec<u32>,
    question: Option<String>,
}

/// "Waiting for your partners / to be ready..." in the game's characters.
fn hold_message() -> Vec<u8> {
    let line = |text: &str| crate::text::encode(text, text.chars().count()).expect("the hold message is plain text");
    let mut message = line("Waiting for your partners");
    message.push(0xFE); // a new line
    message.extend(line("to be ready..."));
    message.push(crate::text::END);
    message
}

/// The game's checksum of a Pokémon: its decrypted substructures summed as
/// 16-bit words.
fn checksum(subs: &[[u8; SUBSTRUCTURE_SIZE]; 4]) -> u16 {
    subs.iter().flat_map(|sub| sub.chunks(2)).fold(0u16, |sum, word| {
        sum.wrapping_add(u16::from_le_bytes([word[0], word[1]]))
    })
}
