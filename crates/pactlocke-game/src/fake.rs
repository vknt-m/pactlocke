//! An in-memory fake game for testing the rules and Run engines without an
//! emulator. Tests set its state directly. Like the real game, its actions
//! only return changes; [`FakeGame::apply`] lands them.

use std::collections::BTreeMap;

use crate::game::{
    Actions, Badges, Battle, Change, Game, GameError, Pokemon, ScriptContext, ScriptStatus, StoredPokemon,
};
use crate::{Area, text};

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
    pub badges: Badges,
    /// Option settings by name, such as `battle_style = "shift"`.
    pub options: BTreeMap<String, String>,
    /// The settings each option has.
    pub option_values: BTreeMap<String, Vec<String>>,
    pub trainer_id: u32,
    /// Primary types by species; a species not listed has type 0.
    pub primary_types: BTreeMap<u16, u8>,
    /// Species names; a species not listed is called `SPECIES <number>`.
    pub species_names: BTreeMap<u16, String>,
    pub safe_point: bool,
    pub holds_poke_balls: bool,

    // What applied changes have switched on.
    pub catches_blocked: bool,
    pub level_cap: Option<u8>,
    pub wild_items_banned: bool,
    pub bag_locked: bool,
    /// Battler positions given a perish count of zero; the game faints them
    /// at the end of the turn.
    pub perishing: Vec<u8>,
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
            badges: Badges::default(),
            options: BTreeMap::from([("battle_style".to_owned(), "shift".to_owned())]),
            option_values: BTreeMap::from([("battle_style".to_owned(), vec!["shift".to_owned(), "set".to_owned()])]),
            trainer_id: 0,
            primary_types: BTreeMap::new(),
            species_names: BTreeMap::new(),
            safe_point: true,
            holds_poke_balls: true,
            catches_blocked: false,
            level_cap: None,
            wild_items_banned: false,
            bag_locked: false,
            perishing: Vec::new(),
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
                NICKNAME => {
                    self.party[usize::from(bytes[0])].nickname = String::from_utf8(bytes[1..].to_vec()).unwrap();
                }
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

    fn badges(&mut self) -> Badges {
        self.badges
    }

    fn option(&mut self, name: &str) -> Option<String> {
        self.options.get(name).cloned()
    }

    fn trainer_id(&mut self) -> u32 {
        self.trainer_id
    }

    fn primary_type(&mut self, species: u16) -> u8 {
        self.primary_types.get(&species).copied().unwrap_or(0)
    }

    fn species_name(&mut self, species: u16) -> String {
        self.species_names
            .get(&species)
            .cloned()
            .unwrap_or_else(|| format!("SPECIES {species}"))
    }

    fn at_safe_overworld_point(&mut self) -> bool {
        self.safe_point && self.battle.is_none()
    }

    fn holds_poke_balls(&mut self) -> bool {
        self.holds_poke_balls
    }

    fn block_catches(&mut self, blocked: bool) -> Actions {
        Ok(vec![fake_patch(BLOCK_CATCHES, vec![u8::from(blocked)])])
    }

    fn set_level_cap(&mut self, level: u8) -> Actions {
        if !(1..=100).contains(&level) {
            return Err(GameError::BadLevel(level));
        }
        Ok(vec![fake_patch(LEVEL_CAP, vec![level])])
    }

    fn ban_items_in_wild_battles(&mut self, banned: bool) -> Actions {
        Ok(vec![fake_patch(BAN_WILD_ITEMS, vec![u8::from(banned)])])
    }

    fn lock_bag_in_trainer_battles(&mut self, locked: bool) -> Actions {
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
}
