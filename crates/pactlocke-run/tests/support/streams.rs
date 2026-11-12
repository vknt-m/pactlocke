//! Players whose games are watched: each plays the fake game through the
//! real running game, which builds their input stream, and every other app
//! keeps a Shadow Game of them. Messages cross as bytes through a network
//! the test controls: it can hold them back (lag), lose them, or change them.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{Area, BattleKind, FakeCore, FakeGame};
use pactlocke_run::{
    InputChunk, MemoryStorage, Outgoing, PlayerKey, Ruleset, RunEngine, RunningGame, ShadowGame, StreamHub,
    StreamMessage,
};

use super::{CHARMANDER, PIDGEY, RATTATA, START_MS, TestClock, battle, new_solo_run, wild_pokemon};

/// About one GBA frame.
pub const FRAME_MS: i64 = 17;

pub fn key(i: usize) -> PlayerKey {
    [i as u8 + 1; 32]
}

/// One Player's app: their own game, streaming, and a Shadow Game of every other Player.
pub struct App {
    pub name: String,
    pub key: PlayerKey,
    pub running: RunningGame<FakeGame>,
    outbox: Arc<Mutex<Vec<InputChunk>>>,
    pub hub: StreamHub,
    pub shadows: BTreeMap<PlayerKey, ShadowGame<FakeCore>>,
    /// Frames this Player has played, so their keys vary.
    played: usize,
}

impl App {
    pub fn view_feed(&self) -> Vec<String> {
        self.running
            .engine()
            .view()
            .feed
            .iter()
            .map(|e| e.text.clone())
            .collect()
    }

    pub fn shadow_of(&mut self, key: PlayerKey) -> &mut ShadowGame<FakeCore> {
        self.shadows.get_mut(&key).expect("a Shadow of that Player")
    }
}

/// A message in flight: from, to, bytes, and the time it lands.
struct InFlight {
    from: PlayerKey,
    to: PlayerKey,
    bytes: Vec<u8>,
    lands_ms: i64,
}

/// One to three Players' apps and the network between them; Player 0 hosts.
pub struct Table {
    pub apps: Vec<App>,
    pub clock: TestClock,
    in_flight: Vec<InFlight>,
    /// Messages take this long to land.
    pub lag_ms: i64,
    /// Decides whether a message from one Player to another is lost.
    pub lose: Lose,
    /// May change a message from one Player to another before it lands.
    pub tamper: Tamper,
}

/// From, to, the message: whether it is lost.
pub type Lose = Box<dyn FnMut(PlayerKey, PlayerKey, &StreamMessage) -> bool>;
/// From, to, the message to change.
pub type Tamper = Box<dyn FnMut(PlayerKey, PlayerKey, &mut StreamMessage)>;

impl Table {
    /// Players with these names start a Run with `ruleset`, each in Pallet Town.
    pub fn start(names: &[&str], ruleset: Ruleset) -> Self {
        let clock = TestClock::at(START_MS);
        let host = key(0);
        let apps = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let engine = RunEngine::start(
                    new_solo_run(ruleset.clone()),
                    Box::new(MemoryStorage::default()),
                    Box::new(clock.clone()),
                )
                .unwrap();
                let mut game = FakeGame {
                    area: Some(Area {
                        id: 0x58,
                        name: "Pallet Town".into(),
                    }),
                    ..FakeGame::default()
                };
                for (species, name) in [(PIDGEY, "PIDGEY"), (RATTATA, "RATTATA"), (CHARMANDER, "CHARMANDER")] {
                    game.species_names.insert(species, name.into());
                }
                let mut running = RunningGame::boot(game, engine, START_MS).unwrap();
                let outbox = running.stream_as(key(i));
                let shadows = (0..names.len())
                    .filter(|&j| j != i)
                    .map(|j| (key(j), ShadowGame::power_on(FakeCore::default(), START_MS)))
                    .collect();
                App {
                    name: (*name).into(),
                    key: key(i),
                    running,
                    outbox,
                    hub: StreamHub::new(key(i), host),
                    shadows,
                    played: 0,
                }
            })
            .collect();
        Self {
            apps,
            clock,
            in_flight: Vec::new(),
            lag_ms: 0,
            lose: Box::new(|_, _, _| false),
            tamper: Box::new(|_, _, _| {}),
        }
    }

    fn index(&self, key: PlayerKey) -> usize {
        self.apps
            .iter()
            .position(|a| a.key == key)
            .expect("a Player at the table")
    }

    /// Player `i` plays `n` frames, pressing a changing mix of buttons,
    /// while time moves and every app passes messages and runs its Shadows.
    pub fn plays(&mut self, i: usize, n: usize) {
        for _ in 0..n {
            let app = &mut self.apps[i];
            let keys = Keys(((app.played * 37) % 0x400) as u16 & 0x3FF);
            app.played += 1;
            app.running.run_frame(keys);
            self.tick();
        }
    }

    /// Everyone stands still for `ms`: no game runs, but the network and the Shadows do.
    pub fn waits(&mut self, ms: i64) {
        for _ in 0..ms / FRAME_MS {
            self.tick();
        }
    }

    /// One slice of time: every app sends its new chunks, messages land, and Shadows run.
    fn tick(&mut self) {
        self.clock.advance_ms(FRAME_MS);
        let now = self.now();
        for i in 0..self.apps.len() {
            let chunks: Vec<InputChunk> = std::mem::take(&mut *self.apps[i].outbox.lock().unwrap());
            for chunk in chunks {
                let out = self.apps[i].hub.own(chunk);
                self.post(self.apps[i].key, out);
            }
        }
        let (landing, flying): (Vec<_>, Vec<_>) = std::mem::take(&mut self.in_flight)
            .into_iter()
            .partition(|m| m.lands_ms <= now);
        self.in_flight = flying;
        for message in landing {
            let to = self.index(message.to);
            let Some(decoded) = StreamMessage::from_bytes(&message.bytes) else {
                continue;
            };
            let (out, new) = self.apps[to].hub.receive(message.from, decoded);
            for chunk in new {
                let app = &mut self.apps[to];
                if let Some(shadow) = app.shadows.get_mut(&chunk.player) {
                    shadow.receive(&chunk, now);
                }
            }
            let from = self.apps[to].key;
            self.post(from, out);
        }
        let names: BTreeMap<PlayerKey, String> = self.apps.iter().map(|a| (a.key, a.name.clone())).collect();
        for app in &mut self.apps {
            let mut diverged = Vec::new();
            for (key, shadow) in &mut app.shadows {
                let before = shadow.state();
                shadow.advance(now, shadow.frames_for(FRAME_MS));
                if before != shadow.state()
                    && let pactlocke_run::ShadowState::Diverged { frame } = shadow.state()
                {
                    diverged.push((*key, frame));
                }
            }
            for (key, frame) in diverged {
                let player = names[&key].clone();
                app.running
                    .commands()
                    .send(pactlocke_run::RunCommand::OutOfSync { player, frame })
                    .unwrap();
            }
        }
    }

    fn post(&mut self, from: PlayerKey, out: Vec<Outgoing>) {
        let now = self.now();
        for outgoing in out {
            let (targets, mut message): (Vec<PlayerKey>, StreamMessage) = match outgoing {
                Outgoing::To(to, message) => (vec![to], message),
                Outgoing::AllBut(but, message) => (
                    self.apps
                        .iter()
                        .map(|a| a.key)
                        .filter(|k| *k != but && *k != from)
                        .collect(),
                    message,
                ),
            };
            for to in targets {
                if (self.lose)(from, to, &message) {
                    continue;
                }
                (self.tamper)(from, to, &mut message);
                self.in_flight.push(InFlight {
                    from,
                    to,
                    bytes: message.to_bytes(),
                    lands_ms: now + self.lag_ms,
                });
            }
        }
    }

    pub fn now(&self) -> i64 {
        use pactlocke_run::RunClock;
        self.clock.now_ms()
    }

    /// Player `i`'s game, to set what happens in it.
    pub fn game(&mut self, i: usize) -> &mut FakeGame {
        self.apps[i].running.game_mut()
    }

    pub fn walks_into(&mut self, i: usize, area: Area) {
        self.game(i).area = Some(area);
        self.plays(i, 10);
    }

    pub fn meets_wild(&mut self, i: usize, species: u16) {
        let game = self.game(i);
        game.opponent_party = vec![wild_pokemon(species)];
        game.battle = Some(battle(BattleKind::Wild, species));
        self.plays(i, 10);
    }

    pub fn leaves_battle(&mut self, i: usize) {
        self.game(i).battle = None;
        self.plays(i, 10);
    }

    /// A memory edit, as a cheat tool makes: straight into the core, unstamped.
    pub fn edits_memory(&mut self, i: usize, addr: u32, bytes: &[u8]) {
        self.game(i).core.write(addr, bytes);
    }
}
