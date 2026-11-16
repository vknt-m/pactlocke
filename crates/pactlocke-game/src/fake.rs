//! An in-memory fake game for testing the rules and Run engines without an
//! emulator. Tests set its state directly. Like the real game, its actions
//! only return changes; [`FakeGame::apply`] lands them.

use std::collections::BTreeMap;

use crate::game::{
    Actions, Badges, Battle, Change, Game, GameError, Pokemon, ScriptContext, ScriptStatus, StoredPokemon,
};
use crate::{Area, FakeCore, Source, SourceDef, text};

/// The fake's changes are addressed here, where no real game has memory, with
/// the action in the low bits and its arguments as the bytes.
const FAKE_BASE: u32 = 0xF000_0000;

const BLOCK_CATCHES: u32 = 1;
const LEVEL_CAP: u32 = 2;
const BAN_WILD_ITEMS: u32 = 3;
const LOCK_BAG: u32 = 4;
const FAINT_ACTIVE: u32 = 5;
const FAINT_BENCHED: u32 = 6;
const MOVE_TO_BOX: u32 = 7;
const NICKNAME: u32 = 8;
const SET_OPTION: u32 = 9;
const RELEASE_FROM_PARTY: u32 = 10;
const RELEASE_FROM_PC: u32 = 11;
const PC_NICKNAME: u32 = 12;
const NICKNAME_YES: u32 = 13;
const CLOSE_GATE: u32 = 14;
const OPEN_GATE: u32 = 15;
const SHARE_SEED: u32 = 16;
const PIN_RNG: u32 = 17;

const PC_BOXES: u8 = 14;
const PC_BOX_SLOTS: u8 = 30;
const DEAD_BOX: u8 = 14;
const NICKNAME_LENGTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeGame {
    pub area: Option<Area>,
    pub party: Vec<Pokemon>,
    pub pc: Vec<StoredPokemon>,
    pub battle: Option<Battle>,
    pub opponent_party: Vec<Pokemon>,
    pub script: ScriptContext,
    /// The running script's return addresses, innermost first.
    pub script_calls: Vec<u32>,
    pub badges: Badges,
    /// The level cap stage the Player is at (see [`Game::cap_stage`]).
    pub cap_stage: usize,
    /// Whether the game has level cap sites to patch; without them only a soft cap applies.
    pub has_cap_sites: bool,
    /// Whether the game has a way to refuse the Bag and ban items in battle.
    pub has_item_ban: bool,
    /// Whether the game has a battle style option for set mode.
    pub has_set_mode: bool,
    /// Whether "Give a nickname?" is up with its YES/NO box.
    pub nickname_prompt_up: bool,
    /// Type names by type; a type not listed is called `TYPE <number>`.
    pub type_names: BTreeMap<u8, String>,
    /// Option settings by name, such as `battle_style = "shift"`.
    pub options: BTreeMap<String, String>,
    /// The settings each option has.
    pub option_values: BTreeMap<String, Vec<String>>,
    /// The rules the game does through its own options: none by default.
    pub native_options: Vec<crate::NativeOption>,
    pub trainer_id: u32,
    /// The game's play time in frames.
    pub play_time: u32,
    /// Primary types by species; a species not listed has type 0.
    pub primary_types: BTreeMap<u16, u8>,
    /// Species names; a species not listed is called `SPECIES <number>`.
    pub species_names: BTreeMap<u16, String>,
    /// What each species can evolve into; a species not listed doesn't evolve.
    pub evolutions: BTreeMap<u16, Vec<u16>>,
    /// The Gift and Static catalogue, as a Game Profile lists it.
    pub sources: Vec<SourceDef>,
    pub safe_point: bool,
    pub holds_poke_balls: bool,

    // What applied changes have switched on.
    pub catches_blocked: bool,
    pub level_cap: Option<u8>,
    pub wild_items_banned: bool,
    pub bag_locked: bool,
    /// Times the nickname prompt's cursor was kept on YES.
    pub nickname_cursor_kept: u32,
    /// Battler positions given a perish count of zero; the game faints them
    /// at the end of the turn.
    pub perishing: Vec<u8>,
    /// The battles the profile lists as possible Sync Gates.
    pub sync_gates: Vec<String>,
    /// The gates held now.
    pub gates_closed: std::collections::BTreeSet<String>,
    /// The gate whose hold script is running, as the test sets it.
    pub at_gate: Option<String>,
    /// Partner Avatars: where the Player stands, their sprite, and room to
    /// draw (`None`, as on a Romhack, draws nothing).
    pub whereabouts: Option<crate::Whereabouts>,
    pub player_sprite: Option<crate::SpriteImage>,
    pub avatar_room: Option<crate::AvatarRoom>,
    /// Whether the game has its own randomizer, the seed Pactlocke gave it,
    /// and whether a trainer battle is triggered with its party not yet made.
    pub own_randomizer: bool,
    pub shared_seed: Option<u32>,
    pub trainer_battle_coming: bool,
    /// The RNG states Pactlocke pinned, one a frame.
    pub rng_pins: Vec<u32>,

    /// The emulator under the fake: driven by keys and changed by every
    /// change applied, so a copy fed the same input stream matches it.
    pub core: FakeCore,
}

impl Default for FakeGame {
    /// A Player on the overworld with an empty party, no script running,
    /// who already has Poké Balls.
    fn default() -> Self {
        Self {
            area: None,
            party: Vec::new(),
            pc: Vec::new(),
            battle: None,
            opponent_party: Vec::new(),
            script: ScriptContext {
                status: ScriptStatus::Idle,
                position: None,
            },
            script_calls: Vec::new(),
            badges: Badges::default(),
            cap_stage: 0,
            has_cap_sites: true,
            has_item_ban: true,
            has_set_mode: true,
            nickname_prompt_up: false,
            type_names: BTreeMap::new(),
            options: BTreeMap::from([("battle_style".to_owned(), "shift".to_owned())]),
            option_values: BTreeMap::from([("battle_style".to_owned(), vec!["shift".to_owned(), "set".to_owned()])]),
            native_options: Vec::new(),
            trainer_id: 0,
            play_time: 0,
            primary_types: BTreeMap::new(),
            species_names: BTreeMap::new(),
            evolutions: BTreeMap::new(),
            sources: Vec::new(),
            safe_point: true,
            holds_poke_balls: true,
            catches_blocked: false,
            level_cap: None,
            wild_items_banned: false,
            bag_locked: false,
            nickname_cursor_kept: 0,
            perishing: Vec::new(),
            sync_gates: Vec::new(),
            gates_closed: std::collections::BTreeSet::new(),
            at_gate: None,
            whereabouts: None,
            player_sprite: None,
            avatar_room: None,
            own_randomizer: false,
            shared_seed: None,
            trainer_battle_coming: false,
            rng_pins: Vec::new(),
            core: FakeCore::default(),
        }
    }
}

fn fake_write(op: u32, bytes: Vec<u8>) -> Change {
    Change::Write {
        addr: FAKE_BASE | op,
        bytes,
    }
}

fn fake_patch(op: u32, bytes: Vec<u8>) -> Change {
    Change::Patch {
        addr: FAKE_BASE | op,
        bytes,
    }
}

impl FakeGame {
    /// Lands changes this fake's actions returned, the way the Run engine
    /// lands a real game's writes and patches.
    ///
    /// # Panics
    /// On a change this fake didn't make.
    pub fn apply(&mut self, changes: &[Change]) {
        for change in changes {
            let (Change::Write { addr, bytes } | Change::Patch { addr, bytes }) = change;
            assert_eq!(addr & 0xFFFF_0000, FAKE_BASE, "not a fake game change: {change:?}");
            match addr & 0xFFFF {
                BLOCK_CATCHES => self.catches_blocked = bytes[0] != 0,
                LEVEL_CAP => self.level_cap = Some(bytes[0]),
                BAN_WILD_ITEMS => self.wild_items_banned = bytes[0] != 0,
                LOCK_BAG => self.bag_locked = bytes[0] != 0,
                FAINT_ACTIVE => self.perishing.push(bytes[0]),
                SHARE_SEED => self.shared_seed = Some(u32::from_le_bytes(bytes[..4].try_into().unwrap())),
                PIN_RNG => self.rng_pins.push(u32::from_le_bytes(bytes[..4].try_into().unwrap())),
                CLOSE_GATE => {
                    self.gates_closed.insert(String::from_utf8_lossy(bytes).into_owned());
                }
                OPEN_GATE => {
                    self.gates_closed.remove(&*String::from_utf8_lossy(bytes));
                }
                FAINT_BENCHED => {
                    if let Some(stats) = self.party[usize::from(bytes[0])].stats.as_mut() {
                        stats.hp = 0;
                    }
                }
                MOVE_TO_BOX => {
                    let (slot, box_number, box_slot) = (bytes[0], bytes[1], bytes[2]);
                    let mut pokemon = self.party.remove(usize::from(slot));
                    pokemon.stats = None;
                    self.pc.push(StoredPokemon {
                        box_number,
                        slot: box_slot,
                        pokemon,
                    });
                }
                RELEASE_FROM_PARTY => {
                    self.party.remove(usize::from(bytes[0]));
                }
                RELEASE_FROM_PC => {
                    self.pc.retain(|s| (s.box_number, s.slot) != (bytes[0], bytes[1]));
                }
                NICKNAME => {
                    self.party[usize::from(bytes[0])].nickname = String::from_utf8(bytes[1..].to_vec()).unwrap();
                }
                PC_NICKNAME => {
                    let (box_number, slot) = (bytes[0], bytes[1]);
                    let name = String::from_utf8(bytes[2..].to_vec()).unwrap();
                    if let Some(stored) = self
                        .pc
                        .iter_mut()
                        .find(|s| (s.box_number, s.slot) == (box_number, slot))
                    {
                        stored.pokemon.nickname = name;
                    }
                }
                NICKNAME_YES => self.nickname_cursor_kept += 1,
                SET_OPTION => {
                    let text = String::from_utf8(bytes.clone()).unwrap();
                    let (name, value) = text.split_once('=').unwrap();
                    self.options.insert(name.to_owned(), value.to_owned());
                }
                other => panic!("unknown fake game action {other}"),
            }
        }
    }

    fn check_party_slot(&self, slot: u8) -> Result<(), GameError> {
        if usize::from(slot) < self.party.len() {
            Ok(())
        } else {
            Err(GameError::NoSuchSlot(slot))
        }
    }
}

impl Game for FakeGame {
    fn area(&mut self) -> Option<Area> {
        self.area.clone()
    }

    fn party(&mut self) -> Vec<Pokemon> {
        self.party.clone()
    }

    fn pc(&mut self) -> Vec<StoredPokemon> {
        self.pc.clone()
    }

    fn battle(&mut self) -> Option<Battle> {
        self.battle.clone()
    }

    fn opponent_party(&mut self) -> Vec<Pokemon> {
        self.opponent_party.clone()
    }

    fn script(&mut self) -> ScriptContext {
        self.script
    }

    fn script_calls(&mut self) -> Vec<u32> {
        self.script_calls.clone()
    }

    fn badges(&mut self) -> Badges {
        self.badges
    }

    fn cap_stage(&mut self) -> usize {
        self.cap_stage
    }

    fn option(&mut self, name: &str) -> Option<String> {
        self.options.get(name).cloned()
    }

    fn play_time(&mut self) -> u32 {
        self.play_time
    }

    fn trainer_id(&mut self) -> u32 {
        self.trainer_id
    }

    fn primary_type(&mut self, species: u16) -> u8 {
        self.primary_types.get(&species).copied().unwrap_or(0)
    }

    fn type_name(&mut self, type_id: u8) -> String {
        self.type_names
            .get(&type_id)
            .cloned()
            .unwrap_or_else(|| format!("TYPE {type_id}"))
    }

    fn species_name(&mut self, species: u16) -> String {
        self.species_names
            .get(&species)
            .cloned()
            .unwrap_or_else(|| format!("SPECIES {species}"))
    }

    fn evolutions(&mut self) -> Vec<Vec<u16>> {
        // Enough species to cover every one named, as evolving or evolved into.
        let named = self
            .evolutions
            .iter()
            .flat_map(|(from, into)| std::iter::once(from).chain(into));
        let count = named.max().map_or(0, |&last| usize::from(last) + 1);
        (0..count)
            .map(|species| {
                let species = u16::try_from(species).expect("species fit in u16");
                self.evolutions.get(&species).cloned().unwrap_or_default()
            })
            .collect()
    }

    fn source_before(&mut self, position: u32) -> Option<Source> {
        crate::source::source_before(&self.sources, position)
    }

    fn at_safe_overworld_point(&mut self) -> bool {
        self.safe_point && self.battle.is_none()
    }

    fn nickname_prompt_up(&mut self) -> bool {
        self.nickname_prompt_up
    }

    fn holds_poke_balls(&mut self) -> bool {
        self.holds_poke_balls
    }

    fn block_catches(&mut self, blocked: bool) -> Actions {
        Ok(vec![fake_patch(BLOCK_CATCHES, vec![u8::from(blocked)])])
    }

    fn set_level_cap(&mut self, level: u8) -> Actions {
        if !(1..=crate::MAX_LEVEL).contains(&level) {
            return Err(GameError::BadLevel(level));
        }
        if !self.has_cap_sites {
            return Err(GameError::NoMethod("level caps"));
        }
        Ok(vec![fake_patch(LEVEL_CAP, vec![level])])
    }

    fn ban_items_in_wild_battles(&mut self, banned: bool) -> Actions {
        if !self.has_item_ban {
            return Err(GameError::NoMethod("no items in wild battles"));
        }
        Ok(vec![fake_patch(BAN_WILD_ITEMS, vec![u8::from(banned)])])
    }

    fn lock_bag_in_trainer_battles(&mut self, locked: bool) -> Actions {
        if !self.has_item_ban {
            return Err(GameError::NoMethod("no items in trainer battles"));
        }
        Ok(vec![fake_patch(LOCK_BAG, vec![u8::from(locked)])])
    }

    fn faint_active_battler(&mut self, position: u8) -> Actions {
        let battle = self.battle.as_ref().ok_or(GameError::NotInBattle)?;
        if !battle.battlers.iter().any(|b| b.position == position) {
            return Err(GameError::NoSuchBattler(position));
        }
        Ok(vec![fake_write(FAINT_ACTIVE, vec![position])])
    }

    fn faint_benched(&mut self, party_slot: u8) -> Actions {
        self.check_party_slot(party_slot)?;
        Ok(vec![fake_write(FAINT_BENCHED, vec![party_slot])])
    }

    fn move_to_box(&mut self, party_slot: u8, box_number: u8) -> Actions {
        self.check_party_slot(party_slot)?;
        if !(1..=PC_BOXES).contains(&box_number) {
            return Err(GameError::NoSuchBox(box_number));
        }
        let free = (1..=PC_BOX_SLOTS)
            .find(|slot| !self.pc.iter().any(|s| s.box_number == box_number && s.slot == *slot))
            .ok_or(GameError::BoxFull(box_number))?;
        Ok(vec![fake_write(MOVE_TO_BOX, vec![party_slot, box_number, free])])
    }

    fn release_from_party(&mut self, party_slot: u8) -> Actions {
        self.check_party_slot(party_slot)?;
        Ok(vec![fake_write(RELEASE_FROM_PARTY, vec![party_slot])])
    }

    fn release_from_pc(&mut self, box_number: u8, slot: u8) -> Actions {
        if !(1..=PC_BOXES).contains(&box_number) {
            return Err(GameError::NoSuchBox(box_number));
        }
        if !self.pc.iter().any(|s| (s.box_number, s.slot) == (box_number, slot)) {
            return Err(GameError::NoSuchSlot(slot));
        }
        Ok(vec![fake_write(RELEASE_FROM_PC, vec![box_number, slot])])
    }

    fn dead_box(&mut self) -> u8 {
        DEAD_BOX
    }

    fn move_to_dead_box(&mut self, party_slot: u8) -> Actions {
        self.move_to_box(party_slot, DEAD_BOX)
    }

    fn write_nickname(&mut self, party_slot: u8, name: &str) -> Actions {
        self.check_party_slot(party_slot)?;
        text::encode(name, NICKNAME_LENGTH).map_err(GameError::BadName)?;
        let mut bytes = vec![party_slot];
        bytes.extend_from_slice(name.as_bytes());
        Ok(vec![fake_write(NICKNAME, bytes)])
    }

    fn write_pc_nickname(&mut self, box_number: u8, slot: u8, name: &str) -> Actions {
        if !self.pc.iter().any(|s| (s.box_number, s.slot) == (box_number, slot)) {
            return Err(GameError::NoSuchSlot(slot));
        }
        text::encode(name, NICKNAME_LENGTH).map_err(GameError::BadName)?;
        let mut bytes = vec![box_number, slot];
        bytes.extend_from_slice(name.as_bytes());
        Ok(vec![fake_write(PC_NICKNAME, bytes)])
    }

    fn keep_nickname_cursor_on_yes(&mut self) -> Actions {
        Ok(vec![fake_write(NICKNAME_YES, vec![0])])
    }

    fn force_set_mode(&mut self) -> Actions {
        if !self.has_set_mode {
            return Err(GameError::NoMethod("set mode"));
        }
        if self.options.get("battle_style").map(String::as_str) == Some("set") {
            return Ok(Vec::new());
        }
        self.set_option("battle_style", "set")
    }

    fn set_option(&mut self, name: &str, value: &str) -> Actions {
        let values = self
            .option_values
            .get(name)
            .ok_or_else(|| GameError::UnknownOption(name.to_owned()))?;
        if !values.iter().any(|v| v == value) {
            return Err(GameError::UnknownOption(format!("{name} = {value}")));
        }
        Ok(vec![fake_write(SET_OPTION, format!("{name}={value}").into_bytes())])
    }

    fn native_options(&mut self) -> Vec<crate::NativeOption> {
        self.native_options.clone()
    }

    fn whereabouts(&mut self) -> Option<crate::Whereabouts> {
        self.whereabouts
    }

    fn player_sprite(&mut self) -> Option<crate::SpriteImage> {
        self.player_sprite.clone()
    }

    /// Every species' icon is its number, over and over, with a palette of its number.
    fn party_icon(&mut self, species: u16) -> Option<crate::SpriteImage> {
        let [low, high] = species.to_le_bytes();
        Some(crate::SpriteImage {
            tiles: [low, high].repeat(256),
            palette: [high, low].repeat(16),
            hflip: false,
            priority: 2,
            square: true,
        })
    }

    fn avatar_room(&mut self) -> Option<crate::AvatarRoom> {
        self.avatar_room.clone()
    }

    fn sync_gates(&mut self) -> Vec<String> {
        self.sync_gates.clone()
    }

    fn close_gate(&mut self, gate: &str) -> Actions {
        if !self.sync_gates.iter().any(|g| g == gate) {
            return Err(GameError::NoSuchGate(gate.to_owned()));
        }
        Ok(vec![fake_patch(CLOSE_GATE, gate.as_bytes().to_vec())])
    }

    fn open_gate(&mut self, gate: &str) -> Actions {
        if !self.sync_gates.iter().any(|g| g == gate) {
            return Err(GameError::NoSuchGate(gate.to_owned()));
        }
        Ok(vec![fake_patch(OPEN_GATE, gate.as_bytes().to_vec())])
    }

    fn at_gate(&mut self) -> Option<String> {
        self.at_gate.clone()
    }

    fn has_own_randomizer(&mut self) -> bool {
        self.own_randomizer
    }

    fn share_seed(&mut self, seed: u32) -> Actions {
        if !self.own_randomizer {
            return Err(GameError::NoMethod("Shared Seed"));
        }
        if self.shared_seed == Some(seed) {
            return Ok(Vec::new());
        }
        Ok(vec![fake_write(SHARE_SEED, seed.to_le_bytes().to_vec())])
    }

    fn pin_rng(&mut self, seed: u32) -> Actions {
        if !self.trainer_battle_coming {
            return Ok(Vec::new());
        }
        Ok(vec![fake_write(PIN_RNG, seed.to_le_bytes().to_vec())])
    }
}
