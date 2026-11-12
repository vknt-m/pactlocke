//! Seam 1: a solo Run played through the Run engine, with the in-memory fake
//! game, in-memory storage and a clock the test moves.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use pactlocke_emu::Keys;
use pactlocke_game::{Area, Battle, BattleKind, Battler, FakeGame, Pokemon, ScriptContext, ScriptStatus, Side};
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
        let mut new = new_solo_run(Ruleset { first_encounter: true });
        new.name = name.into();
        Self::starts(new, storage, clock)
    }

    fn starts(new: NewRun, storage: Box<dyn RunStorage>, clock: TestClock) -> Self {
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
        let engine = RunEngine::resume(storage, Box::new(clock.clone())).unwrap();
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

fn battle(kind: BattleKind, species: u16) -> Battle {
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
