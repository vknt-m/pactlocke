//! The real game interface: driven only by Game Profile data, over a game's
//! memory (normally the core adapter).

use std::sync::Arc;

use crate::caps::MAX_LEVEL;
use crate::game::{
    Actions, Badges, Battle, BattleKind, Battler, Change, Game, GameError, Memory, PartyStats, Pokemon, ScriptContext,
    ScriptStatus, Side, StoredPokemon,
};
use crate::profile::{GameProfile, Method, PatchSite, ROM_BASE};
use crate::{Area, Source, text};

/// The order of the four substructures (Growth, Attacks, EVs, Misc) for each
/// PID mod 24.
const SUBSTRUCTURE_ORDERS: [&[u8; 4]; 24] = [
    b"GAEM", b"GAME", b"GEAM", b"GEMA", b"GMAE", b"GMEA", b"AGEM", b"AGME", b"AEGM", b"AEMG", b"AMGE", b"AMEG",
    b"EGAM", b"EGMA", b"EAGM", b"EAMG", b"EMGA", b"EMAG", b"MGAE", b"MGEA", b"MAGE", b"MAEG", b"MEGA", b"MEAG",
];
const SUBSTRUCTURE_SIZE: usize = 12;
const PARTY_SIZE: u8 = 6;

/// A game read and steered through its Game Profile.
pub struct ProfileGame<M> {
    profile: Arc<GameProfile>,
    /// The clean ROM, for undoing table patches.
    rom: Arc<[u8]>,
    memory: M,
}

impl<M: Memory> ProfileGame<M> {
    pub fn new(profile: Arc<GameProfile>, rom: impl Into<Arc<[u8]>>, memory: M) -> Self {
        Self {
            profile,
            rom: rom.into(),
            memory,
        }
    }

    pub fn memory(&self) -> &M {
        &self.memory
    }

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
        let pointer = if number == 1 {
            self.profile.symbols.save_block_1_pointer
        } else {
            self.profile.symbols.save_block_2_pointer
        };
        self.u32(pointer)
    }

    fn party_count(&mut self) -> u8 {
        self.u8(self.profile.symbols.party_count).min(PARTY_SIZE)
    }

    fn party_slot_addr(&self, slot: u8) -> u32 {
        self.profile.symbols.party + u32::from(slot) * self.profile.layout.party_slot_size
    }

    fn box_slot_addr(&mut self, box_index: u8, slot_index: u8) -> u32 {
        let layout = &self.profile.layout;
        let index = u32::from(box_index) * u32::from(layout.pc_box_slots) + u32::from(slot_index);
        let (offset, size) = (layout.pc_boxes_offset, layout.box_slot_size);
        self.u32(self.profile.symbols.pc_storage_pointer) + offset + index * size
    }

    fn has_species(&mut self, addr: u32) -> bool {
        let layout = &self.profile.layout;
        let (flags, mask) = (layout.flags, layout.has_species_mask);
        self.u8(addr + flags) & mask != 0
    }

    fn in_battle(&mut self) -> bool {
        let bit = self.profile.symbols.in_battle;
        self.u8(bit.at) & bit.mask != 0
    }

    /// Reads a Pokémon at `addr`; party Pokémon also carry their stats.
    fn pokemon(&mut self, addr: u32, in_party: bool) -> Pokemon {
        let layout = self.profile.layout.clone();
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

        // The four substructures, in Growth, Attacks, EVs, Misc order.
        let mut subs = [[0u8; SUBSTRUCTURE_SIZE]; 4];
        let order: &[u8; 4] = if layout.encrypted {
            SUBSTRUCTURE_ORDERS[(pid % 24) as usize]
        } else {
            b"GAEM"
        };
        let key = if layout.encrypted { pid ^ ot_id } else { 0 };
        for (position, kind) in order.iter().enumerate() {
            let which = b"GAEM".iter().position(|k| k == kind).unwrap();
            let start = at(layout.substructures) + position * SUBSTRUCTURE_SIZE;
            for (i, chunk) in raw[start..start + SUBSTRUCTURE_SIZE].chunks(4).enumerate() {
                let plain = u32::from_le_bytes(chunk.try_into().unwrap()) ^ key;
                subs[which][i * 4..i * 4 + 4].copy_from_slice(&plain.to_le_bytes());
            }
        }
        let [growth, _, _, misc] = subs;

        let nickname_start = at(layout.nickname);
        let stats = in_party.then(|| PartyStats {
            level: raw[at(layout.level)],
            hp: u16::from_le_bytes([raw[at(layout.hp)], raw[at(layout.hp) + 1]]),
            max_hp: u16::from_le_bytes([raw[at(layout.max_hp)], raw[at(layout.max_hp) + 1]]),
        });
        Pokemon {
            pid,
            ot_id,
            species: u16::from_le_bytes([growth[0], growth[1]]),
            nickname: text::decode(&raw[nickname_start..nickname_start + layout.nickname_length as usize]),
            met_section: u16::from(misc[1]),
            is_egg: raw[at(layout.flags)] & layout.egg_mask != 0,
            stats,
        }
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
            .map(|slot| self.pokemon(self.party_slot_addr(slot), true))
            .collect()
    }

    fn pc(&mut self) -> Vec<StoredPokemon> {
        let (boxes, slots) = (self.profile.layout.pc_boxes, self.profile.layout.pc_box_slots);
        let mut stored = Vec::new();
        for box_index in 0..boxes {
            for slot_index in 0..slots {
                let addr = self.box_slot_addr(box_index, slot_index);
                if self.has_species(addr) {
                    stored.push(StoredPokemon {
                        box_number: box_index + 1,
                        slot: slot_index + 1,
                        pokemon: self.pokemon(addr, false),
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
            if self.has_species(addr) {
                party.push(self.pokemon(addr, true));
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
        let block = self.save_block(1);
        Badges(self.u8(block + self.profile.layout.badges))
    }

    fn cap_stage(&mut self) -> usize {
        let badges = self.badges();
        let flags = self.save_block(1) + self.profile.layout.event_flags;
        let profile = self.profile.clone();
        profile
            .level_caps
            .iter()
            .take_while(|stage| {
                stage.passed(badges, |flag| {
                    self.u8(flags + u32::from(flag / 8)) & (1 << (flag % 8)) != 0
                })
            })
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

    fn primary_type(&mut self, species: u16) -> u8 {
        let layout = &self.profile.layout;
        let addr =
            self.profile.symbols.species_info + u32::from(species) * layout.species_info_size + layout.primary_type;
        self.u8(addr)
    }

    fn species_name(&mut self, species: u16) -> String {
        let length = self.profile.layout.species_name_length;
        let addr = self.profile.symbols.species_names + u32::from(species) * length;
        text::decode(&self.bytes(addr, length as usize))
    }

    fn evolutions(&mut self) -> Vec<Vec<u16>> {
        let layout = self.profile.layout.clone();
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

    fn holds_poke_balls(&mut self) -> bool {
        let layout = &self.profile.layout;
        let (offset, slots) = (layout.balls_pocket, layout.balls_pocket_slots);
        let pocket = self.save_block(1) + offset;
        // The game empties a slot's item when its last ball is used.
        (0..u32::from(slots)).any(|slot| self.u16(pocket + slot * 4) != 0)
    }

    fn block_catches(&mut self, blocked: bool) -> Actions {
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
                let mut bytes = site.expect.0;
                bytes[0] = level;
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
        let status_addr = symbols.battler_statuses3 + u32::from(position) * 4;
        let timer_addr =
            symbols.disable_structs + u32::from(position) * layout.disable_struct_size + layout.perish_timer;
        let perish = layout.perish_song_status3;
        let status = self.u32(status_addr) | perish;
        Ok(vec![
            Change::Write {
                addr: status_addr,
                bytes: status.to_le_bytes().to_vec(),
            },
            Change::Write {
                addr: timer_addr,
                bytes: vec![0],
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
                bytes: moved[..layout.box_slot_size as usize].to_vec(),
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

    fn force_set_mode(&mut self) -> Actions {
        let Some(Method::OptionBit { option, value }) = self.profile.rules.set_mode.clone() else {
            return Err(GameError::NoMethod("set mode"));
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
}
