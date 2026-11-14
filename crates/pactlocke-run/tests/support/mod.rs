//! Seam 1: a solo Run played through the Run engine, with the in-memory fake
//! game, in-memory storage and a clock the test moves.
#![allow(dead_code)]

pub mod coop;
pub mod streams;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use pactlocke_emu::Keys;
use pactlocke_game::{
    Area, Battle, BattleKind, Battler, FakeGame, PartyStats, Pokemon, ScriptContext, ScriptStatus, Side,
};
use pactlocke_run::{FeedKind, MemoryStorage, NewRun, Ruleset, RunClock, RunEngine, RunStorage};

/// 2026-01-01T00:00:00Z.
pub const START_MS: i64 = 1_767_225_600_000;

/// A clock that only moves when the test says so.
#[derive(Clone, Default)]
pub struct TestClock(Arc<AtomicI64>);

impl TestClock {
    pub fn at(ms: i64) -> Self {
        Self(Arc::new(AtomicI64::new(ms)))
    }

    pub fn advance_ms(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl RunClock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub fn route_1() -> Area {
    Area {
        id: 0x65,
        name: "Route 1".into(),
    }
}

pub fn route_2() -> Area {
    Area {
        id: 0x66,
        name: "Route 2".into(),
    }
}

pub const PIDGEY: u16 = 16;
pub const RATTATA: u16 = 19;
pub const CHARMANDER: u16 = 4;

pub fn new_solo_run(ruleset: Ruleset) -> NewRun {
    NewRun {
        name: "Kanto solo".into(),
        game_name: "Pokémon FireRed (USA) 1.0".into(),
        game_sha1: "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc".into(),
        rom_path: Some("roms/firered.gba".into()),
        ruleset,
    }
}

/// One Player in a solo Run, playing the fake game.
pub struct Player {
    pub engine: RunEngine,
    pub game: FakeGame,
    pub clock: TestClock,
    pub storage: MemoryStorage,
}

impl Player {
    /// A Player who has just started a solo Run with this Ruleset, standing in Pallet Town.
    pub fn starts_a_run(ruleset: Ruleset) -> Self {
        let storage = MemoryStorage::default();
        let mut player = Self::starts_a_run_in(Box::new(storage.clone()), TestClock::at(START_MS), ruleset);
        player.storage = storage;
        player
    }

    /// The same, with the Run kept in `storage`.
    pub fn starts_a_run_in(storage: Box<dyn RunStorage>, clock: TestClock, ruleset: Ruleset) -> Self {
        Self::starts(new_solo_run(ruleset), storage, clock)
    }

    /// A first-Encounter Run with this name.
    pub fn starts_named_run_in(name: &str, storage: Box<dyn RunStorage>, clock: TestClock) -> Self {
        let mut new = new_solo_run(Ruleset {
            first_encounter: true,
            ..Ruleset::default()
        });
        new.name = name.into();
        Self::starts(new, storage, clock)
    }

    /// A Player who has just started the Run `new`, standing in Pallet Town.
    pub fn starts(new: NewRun, storage: Box<dyn RunStorage>, clock: TestClock) -> Self {
        let engine = RunEngine::start(new, storage, Box::new(clock.clone())).unwrap();
        let mut player = Self::playing(engine, clock);
        player.game.area = Some(Area {
            id: 0x58,
            name: "Pallet Town".into(),
        });
        player
    }

    /// The same Player resuming this Run later from its storage, in a newly booted game.
    pub fn resumes(storage: MemoryStorage, clock: TestClock) -> Self {
        let mut player = Self::resumes_from(Box::new(storage.clone()), clock);
        player.storage = storage;
        player
    }

    pub fn resumes_from(storage: Box<dyn RunStorage>, clock: TestClock) -> Self {
        let mut engine = RunEngine::resume(storage, Box::new(clock.clone())).unwrap();
        // The game booted by the plan, as the running game does.
        engine.booted(pactlocke_run::Booted {
            clock_ms: 0,
            patches: Default::default(),
        });
        Self::playing(engine, clock)
    }

    fn playing(engine: RunEngine, clock: TestClock) -> Self {
        let mut game = FakeGame::default();
        game.species_names.insert(PIDGEY, "PIDGEY".into());
        game.species_names.insert(RATTATA, "RATTATA".into());
        game.species_names.insert(CHARMANDER, "CHARMANDER".into());
        Self {
            engine,
            game,
            clock,
            storage: MemoryStorage::default(),
        }
    }

    /// Runs `n` frames with no buttons, each followed by its frame boundary.
    pub fn plays(&mut self, n: u32) {
        for _ in 0..n {
            self.engine.frame(Keys::NONE, &mut self.game);
        }
    }

    pub fn walks_into(&mut self, area: Area) {
        self.game.area = Some(area);
        self.plays(10);
    }

    /// A wild Pokémon jumps out of the grass: no script is running.
    pub fn meets_wild(&mut self, species: u16) {
        self.game.opponent_party = vec![wild_pokemon(species)];
        self.game.battle = Some(battle(BattleKind::Wild, species));
        self.plays(10);
    }

    /// A battle a script starts and waits on, such as a Static.
    pub fn meets_scripted(&mut self, species: u16) {
        self.game.script = ScriptContext {
            status: ScriptStatus::Waiting,
            position: Some(0x0816_0000),
        };
        self.game.opponent_party = vec![wild_pokemon(species)];
        self.game.battle = Some(battle(BattleKind::Wild, species));
        self.plays(10);
    }

    pub fn meets_trainer(&mut self, species: u16) {
        self.game.opponent_party = vec![wild_pokemon(species)];
        self.game.battle = Some(battle(BattleKind::Trainer, species));
        self.plays(10);
    }

    /// The battle is over (run from, won or lost) and the Player is back on the overworld.
    pub fn leaves_battle(&mut self) {
        self.game.battle = None;
        self.game.script = ScriptContext {
            status: ScriptStatus::Idle,
            position: None,
        };
        self.plays(10);
    }

    /// The wild Pokémon is caught: it joins the party before the battle ends.
    pub fn catches(&mut self, species: u16, pid: u32) {
        let met_section = self.game.area.as_ref().map_or(0, |a| a.id);
        self.game.party.push(Pokemon {
            pid,
            ot_id: 1,
            species,
            nickname: "CAUGHT".into(),
            met_section,
            is_egg: false,
            stats: None,
        });
        self.plays(10);
        self.leaves_battle();
    }

    /// The Player's party Pokémon with this PID faints.
    pub fn faints(&mut self, pid: u32) {
        let pokemon = self.game.party.iter_mut().find(|p| p.pid == pid).expect("in the party");
        pokemon.stats.as_mut().expect("a party Pokémon").hp = 0;
        self.plays(10);
    }

    /// The Player loses a battle: the game heals the party before the
    /// Player is back on the overworld.
    pub fn blacks_out(&mut self) {
        self.game.battle = None;
        self.game.safe_point = false;
        self.plays(10);
        for pokemon in &mut self.game.party {
            pokemon.stats.as_mut().expect("a party Pokémon").hp = HEALTHY_HP;
        }
        self.game.safe_point = true;
        self.plays(10);
    }

    /// Poison takes the last HP of the party Pokémon with this PID as the Player walks.
    pub fn is_poisoned_to_faint(&mut self, pid: u32) {
        self.faints(pid);
    }

    /// A script runs, such as a trainer talking after the battle, until [`Player::script_ends`].
    pub fn script_runs(&mut self) {
        self.game.script = ScriptContext {
            status: ScriptStatus::Running,
            position: Some(0x0816_1000),
        };
        self.plays(10);
    }

    pub fn script_ends(&mut self) {
        self.game.script = ScriptContext {
            status: ScriptStatus::Idle,
            position: None,
        };
        self.plays(10);
    }

    /// A script gives the Player a Pokémon into the party, at this script
    /// position; then the script ends.
    pub fn receives_gift(&mut self, pokemon: Pokemon, position: u32) {
        self.script_at(position);
        self.game.party.push(pokemon);
        self.plays(10);
        self.script_ends();
    }

    /// A script gives the Player a Pokémon that goes to the PC (the party is full).
    pub fn receives_gift_in_pc(&mut self, pokemon: Pokemon, position: u32) {
        self.script_at(position);
        let slot = (1..=30)
            .find(|s| !self.game.pc.iter().any(|p| p.box_number == 1 && p.slot == *s))
            .unwrap();
        self.game.pc.push(pactlocke_game::StoredPokemon {
            box_number: 1,
            slot,
            pokemon,
        });
        self.plays(10);
        self.script_ends();
    }

    /// An in-game trade: the party Pokémon in `slot` is swapped for `received`.
    pub fn trades(&mut self, slot: usize, received: Pokemon) {
        self.script_at(0x0816_0000);
        self.game.party[slot] = received;
        self.plays(10);
        self.script_ends();
    }

    /// A Static: a script starts a wild battle and waits on it.
    pub fn meets_static(&mut self, species: u16, position: u32) {
        self.game.script = ScriptContext {
            status: ScriptStatus::Waiting,
            position: Some(position),
        };
        self.game.opponent_party = vec![wild_pokemon(species)];
        self.game.battle = Some(battle(BattleKind::Wild, species));
        self.plays(10);
    }

    /// A Gift's script runs at this position, until [`Player::script_ends`].
    pub fn script_at_gift(&mut self, position: u32) {
        self.script_at(position);
    }

    fn script_at(&mut self, position: u32) {
        self.game.script = ScriptContext {
            status: ScriptStatus::Running,
            position: Some(position),
        };
        self.plays(1);
    }

    /// The Player takes the Pokémon with this PID out of the PC into the party.
    pub fn withdraws(&mut self, pid: u32) {
        let at = self
            .game
            .pc
            .iter()
            .position(|s| s.pokemon.pid == pid)
            .expect("in the PC");
        let mut pokemon = self.game.pc.remove(at).pokemon;
        pokemon.stats = Some(healthy());
        self.game.party.push(pokemon);
        self.plays(10);
    }

    pub fn party_pids(&self) -> Vec<u32> {
        self.game.party.iter().map(|p| p.pid).collect()
    }

    /// The PIDs in the fake game's Dead box, box 14.
    pub fn dead_box_pids(&self) -> Vec<u32> {
        self.game
            .pc
            .iter()
            .filter(|s| s.box_number == 14)
            .map(|s| s.pokemon.pid)
            .collect()
    }

    /// The live feed's texts, newest first, of one kind.
    pub fn feed(&self, kind: FeedKind) -> Vec<String> {
        self.engine
            .view()
            .feed
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.text.clone())
            .collect()
    }
}

pub fn battle(kind: BattleKind, species: u16) -> Battle {
    Battle {
        kind,
        double: false,
        battlers: vec![
            Battler {
                position: 0,
                side: Side::Player,
                species: 1,
                hp: 20,
                party_slot: Some(0),
            },
            Battler {
                position: 1,
                side: Side::Opponent,
                species,
                hp: 12,
                party_slot: None,
            },
        ],
    }
}

/// A healthy party Pokémon.
pub fn party_pokemon(species: u16, pid: u32, nickname: &str) -> Pokemon {
    Pokemon {
        pid,
        ot_id: 1,
        species,
        nickname: nickname.into(),
        met_section: 0x65,
        is_egg: false,
        stats: Some(healthy()),
    }
}

const HEALTHY_HP: u16 = 20;

fn healthy() -> PartyStats {
    PartyStats {
        level: 5,
        hp: HEALTHY_HP,
        max_hp: HEALTHY_HP,
    }
}

/// A Pokémon in the opponent's party.
pub fn wild_pokemon(species: u16) -> Pokemon {
    Pokemon {
        pid: 0x5EED_0000 | u32::from(species),
        ot_id: 0,
        species,
        nickname: String::new(),
        met_section: 0,
        is_egg: false,
        stats: None,
    }
}

pub fn wild_battle(species: u16) -> Battle {
    battle(BattleKind::Wild, species)
}

pub trait LoadInputs {
    fn load_inputs(&self) -> Vec<pactlocke_run::InputBatch>;
}

impl LoadInputs for MemoryStorage {
    fn load_inputs(&self) -> Vec<pactlocke_run::InputBatch> {
        self.clone().load().unwrap().inputs
    }
}
