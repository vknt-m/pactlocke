//! Players whose games are watched: each plays the fake game through the
//! real running game, which builds their input stream, and every other app
//! keeps a Shadow Game of them. Messages cross as bytes through a network
//! the test controls: it can hold them back (lag), lose them, or change them.
//! The Run log is shared too: each app's Run state goes to the Host, who
//! orders it, and every app follows the log (joining Run Checkpoints, going
//! back on a Retry). The Host records a Retry when a vote passes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{Area, BattleKind, FakeGame};
use pactlocke_run::{
    InputChunk, Locked, LogBody, LogEntry, LogFollower, MemoryStorage, Outgoing, PlayerKey, RosterEntry, Ruleset,
    RunEngine, RunEvent, RunLog, RunningGame, ShadowGame, Signer, StreamHub, StreamMessage,
};

use super::{CHARMANDER, PIDGEY, RATTATA, START_MS, TestClock, battle, new_solo_run, wild_pokemon};

/// The battles the fake game's profile lists as Sync Gates.
pub fn gate_names() -> Vec<String> {
    vec!["Brock".into(), "Misty".into()]
}

/// About one GBA frame.
pub const FRAME_MS: i64 = 17;

fn signer(i: usize) -> Signer {
    Signer::from_secret([i as u8 + 1; 32])
}

/// Player `i`'s install key.
pub fn key(i: usize) -> PlayerKey {
    signer(i).public()
}

/// One Player's app: their own game, streaming, and a Shadow Game of every other Player.
pub struct App {
    pub name: String,
    pub key: PlayerKey,
    pub running: RunningGame<FakeGame>,
    outbox: Arc<Mutex<Vec<InputChunk>>>,
    pub hub: StreamHub,
    pub shadows: BTreeMap<PlayerKey, ShadowGame<FakeGame>>,
    /// Frames this Player has played, so their keys vary.
    played: usize,
    /// This app's own copy of the Run log, checked entry by entry.
    pub log: RunLog,
    shared: Arc<Mutex<Vec<RunEvent>>>,
    follower: LogFollower,
    links: pactlocke_run::LinkKeeper,
    /// Who this app has heard from, and when.
    pub presence: pactlocke_run::Presence,
    /// This app's Run file, as storage.
    pub storage: MemoryStorage,
    /// Partner Avatars: where this app puts them for its game to draw.
    pub partners: Option<Arc<Mutex<Vec<pactlocke_run::PartnerAvatar>>>>,
    /// What this app has seen of the others' streams, to flag cheats.
    pub watch: pactlocke_run::CheatWatch,
    /// Entries this app refused from the Host, and why.
    pub refused: Vec<String>,
}

/// Judges other Players' claims by what their Shadow Games here have shown.
struct ByShadows<'a>(&'a BTreeMap<PlayerKey, ShadowGame<FakeGame>>);

impl pactlocke_run::Judge for ByShadows<'_> {
    fn has(&self, player: &PlayerKey, pid: u32) -> bool {
        self.0.get(player).is_some_and(|s| s.seen().owned.contains(&pid))
    }

    fn fainted(&self, player: &PlayerKey, pid: u32) -> bool {
        self.0.get(player).is_some_and(|s| s.seen().fainted.contains(&pid))
    }
}

impl App {
    /// The Links as this app works them out.
    pub fn links(&self) -> Vec<pactlocke_run::Link> {
        pactlocke_run::links(&self.log, &self.key, &ByShadows(&self.shadows))
    }

    pub fn view_feed(&self) -> Vec<String> {
        self.running
            .engine()
            .view()
            .feed
            .iter()
            .map(|e| e.text.clone())
            .collect()
    }

    pub fn shadow_of(&mut self, key: PlayerKey) -> &mut ShadowGame<FakeGame> {
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
    /// The Host's log, which orders every entry.
    pub host_log: RunLog,
    /// The Player who orders the log now.
    pub host: usize,
    /// Players whose apps can't reach anyone (and nobody them).
    pub offline: std::collections::BTreeSet<usize>,
    /// Whether the Host records a Retry by itself when a vote passes.
    pub host_records_retries: bool,
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
                let storage = MemoryStorage::default();
                let engine = RunEngine::start(
                    new_solo_run(ruleset.clone()),
                    Box::new(storage.clone()),
                    Box::new(clock.clone()),
                )
                .unwrap();
                let mut game = FakeGame {
                    area: Some(Area {
                        id: 0x58,
                        name: "Pallet Town".into(),
                    }),
                    sync_gates: gate_names(),
                    ..FakeGame::default()
                };
                for (species, name) in [(PIDGEY, "PIDGEY"), (RATTATA, "RATTATA"), (CHARMANDER, "CHARMANDER")] {
                    game.species_names.insert(species, name.into());
                }
                let mut running = RunningGame::boot(game, engine, START_MS).unwrap();
                let outbox = running.stream_as(key(i));
                let shared = running.share();
                let shadows = (0..names.len())
                    .filter(|&j| j != i)
                    .map(|j| {
                        let game = FakeGame {
                            sync_gates: gate_names(),
                            ..FakeGame::default()
                        };
                        (key(j), ShadowGame::power_on(game, START_MS))
                    })
                    .collect();
                App {
                    name: (*name).into(),
                    key: key(i),
                    running,
                    outbox,
                    hub: StreamHub::new(key(i), host),
                    shadows,
                    played: 0,
                    log: RunLog::new(host),
                    shared,
                    follower: LogFollower::new(key(i)),
                    links: pactlocke_run::LinkKeeper::new(key(i)),
                    partners: None,
                    watch: pactlocke_run::CheatWatch::new(START_MS),
                    presence: pactlocke_run::Presence::new(START_MS),
                    storage,
                    refused: Vec::new(),
                }
            })
            .collect();
        let mut table = Self {
            apps,
            clock,
            in_flight: Vec::new(),
            lag_ms: 0,
            lose: Box::new(|_, _, _| false),
            tamper: Box::new(|_, _, _| {}),
            host_log: RunLog::new(host),
            host_records_retries: true,
            host: 0,
            offline: std::collections::BTreeSet::new(),
        };
        table.submit(
            0,
            &LogBody::Genesis {
                nonce: [9; 16],
                run_name: "Kanto with friends".into(),
                game_name: "Pokémon FireRed (USA) 1.0".into(),
                game_sha1: super::coop::FIRERED_SHA1.into(),
            },
        );
        let roster = names
            .iter()
            .enumerate()
            .map(|(i, name)| RosterEntry {
                key: key(i),
                name: (*name).into(),
            })
            .collect();
        table.submit(
            0,
            &LogBody::Locked(Locked {
                run_name: "Kanto with friends".into(),
                game_name: "Pokémon FireRed (USA) 1.0".into(),
                game_sha1: super::coop::FIRERED_SHA1.into(),
                ruleset,
                roster,
                start_ms: START_MS,
            }),
        );
        table
    }

    /// Player `i` signs an entry and the Host orders it; every app checks it.
    pub fn submit(&mut self, i: usize, body: &LogBody) {
        let counter = self.host_log.next_counter(&key(i));
        let submission = self.apps[i].log.sign(&signer(i), counter, body);
        let host = self.host;
        match self.host_log.order(&signer(host), submission) {
            Ok(entry) => self.send_entry(&entry),
            Err(e) => self.apps[i].refused.push(e.to_string()),
        }
    }

    /// An entry, as bytes, to every app online, each of which checks it.
    pub fn send_entry(&mut self, entry: &LogEntry) {
        let bytes = entry.to_bytes();
        let host = key(self.host);
        let now = self.now();
        for (i, app) in self.apps.iter_mut().enumerate() {
            if self.offline.contains(&i) {
                continue;
            }
            app.presence.heard(host, now);
            if let Err(e) = app.log.accept(LogEntry::from_bytes(&bytes).unwrap()) {
                app.refused.push(e.to_string());
            }
        }
    }

    /// Player `i`'s app loses its connection: it plays on alone.
    pub fn goes_offline(&mut self, i: usize) {
        self.offline.insert(i);
    }

    /// Player `i`'s app is back: it gets every entry it missed, and sends
    /// what it queued.
    pub fn comes_back(&mut self, i: usize) {
        self.offline.remove(&i);
        let missed: Vec<LogEntry> = self.host_log.entries()[self.apps[i].log.entries().len()..].to_vec();
        for entry in missed {
            if let Err(e) = self.apps[i].log.accept(entry) {
                self.apps[i].refused.push(e.to_string());
            }
        }
    }

    /// The Host's app vanishes for good.
    pub fn host_vanishes(&mut self) {
        let host = self.host;
        self.goes_offline(host);
    }

    /// The next Player in join order takes over once the Host has been
    /// silent long enough, from the longest log any app online has.
    fn take_over_if_due(&mut self) {
        let now = self.now();
        let host = key(self.host);
        if !self.offline.contains(&self.host) {
            return;
        }
        let Some(next) = (0..self.apps.len())
            .filter(|i| !self.offline.contains(i))
            .find_map(|i| self.apps[i].log.successor())
        else {
            return;
        };
        let Some(k) = self.apps.iter().position(|a| a.key == next) else {
            return;
        };
        if self.offline.contains(&k) || !self.apps[k].presence.host_gone(&host, now) {
            return;
        }
        let copies: Vec<Vec<LogEntry>> = (0..self.apps.len())
            .filter(|i| !self.offline.contains(i))
            .map(|i| self.apps[i].log.entries().to_vec())
            .collect();
        self.host_log = RunLog::longest(key(0), copies).expect("a valid log");
        self.host = k;
        // Everyone online catches up with the longest log, then the takeover.
        for i in 0..self.apps.len() {
            if !self.offline.contains(&i) {
                self.comes_back(i);
            }
        }
        self.submit(k, &LogBody::Takeover);
        for app in &mut self.apps {
            app.hub.host_is(next);
        }
    }

    /// Player `i` votes on dropping Player `leaving` from the Run.
    pub fn votes_to_drop(&mut self, i: usize, leaving: usize, yes: bool) {
        self.submit(
            i,
            &LogBody::Event(RunEvent::DropoutVote {
                player: key(leaving),
                yes,
            }),
        );
        self.tick();
    }

    /// Player `i` votes on a Retry back to the `call`th Run Checkpoint.
    pub fn votes(&mut self, i: usize, call: u64, yes: bool) {
        self.submit(i, &LogBody::Event(RunEvent::RetryVote { call, yes }));
        self.tick();
    }

    /// Player `i` saves in-game (the fake saves on START and SELECT together).
    pub fn saves(&mut self, i: usize) {
        self.apps[i].running.run_frame(pactlocke_game::SAVE_KEYS);
        self.tick();
        // The running game looks for a save every second.
        self.plays(i, 70);
    }

    /// The Run's shared tallies as Player `i`'s app recomputes them.
    pub fn tallies(&self, i: usize) -> pactlocke_run::Tallies {
        self.apps[i].log.tallies()
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
            // Never SELECT: with START it would save the fake game.
            let keys = Keys(((app.played * 37) % 0x400) as u16 & 0x3FF).without(Keys::SELECT);
            app.played += 1;
            app.running.run_frame(keys);
            self.tick();
        }
    }

    /// Player `i` plays `n` frames, `per_tick` of them in each slice of
    /// time: at `per_tick` times a GBA's speed, or so.
    pub fn plays_fast(&mut self, i: usize, n: usize, per_tick: usize) {
        for _ in 0..n / per_tick {
            for _ in 0..per_tick {
                let app = &mut self.apps[i];
                let keys = Keys(((app.played * 37) % 0x400) as u16 & 0x3FF).without(Keys::SELECT);
                app.played += 1;
                app.running.run_frame(keys);
            }
            self.tick();
        }
    }

    /// Player `i`'s app sends this chunk as its own, as a changed build might.
    pub fn forges(&mut self, i: usize, chunk: InputChunk) {
        let from = self.apps[i].key;
        let message = StreamMessage::Chunk(chunk);
        let out = if i == self.host {
            Outgoing::AllBut(from, message)
        } else {
            Outgoing::To(self.apps[self.host].key, message)
        };
        self.post(from, vec![out]);
        self.tick();
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
        self.share_the_log();
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
            self.apps[to].presence.heard(message.from, now);
            let Some(decoded) = StreamMessage::from_bytes(&message.bytes) else {
                continue;
            };
            let received = self.apps[to].hub.receive(message.from, decoded);
            let app = &mut self.apps[to];
            let mut flags = Vec::new();
            if let Some(rewrite) = &received.rewritten {
                flags.extend(app.watch.rewritten(&app.log, rewrite, now));
            }
            for chunk in received.new {
                app.presence.heard(chunk.player, now);
                flags.extend(app.watch.arrived(&chunk, &app.log, now));
                if let Some(shadow) = app.shadows.get_mut(&chunk.player) {
                    shadow.receive(&chunk, now);
                }
            }
            app.shared.lock().unwrap().extend(flags);
            let from = self.apps[to].key;
            self.post(from, received.send);
        }
        for app in &mut self.apps {
            if let Some(partners) = &app.partners {
                let now: Vec<pactlocke_run::PartnerAvatar> = app
                    .shadows
                    .iter()
                    .filter_map(|(key, shadow)| {
                        Some(pactlocke_run::PartnerAvatar {
                            player: *key,
                            seen: shadow.seen().avatar.clone()?,
                        })
                    })
                    .collect();
                *partners.lock().unwrap() = now;
            }
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
            let due = app.watch.due(&app.log, now);
            app.shared.lock().unwrap().extend(due);
            for (key, frame) in diverged {
                let at = pactlocke_run::GamePoint {
                    boot: app.shadows[&key].boot(),
                    frame,
                };
                let flag = app.watch.diverged(&app.log, key, at, now);
                app.shared.lock().unwrap().extend(flag);
                let player = names[&key].clone();
                app.running
                    .commands()
                    .send(pactlocke_run::RunCommand::OutOfSync { player, frame })
                    .unwrap();
            }
        }
    }

    /// Each app's Run state into the log, then each app follows the log; the
    /// Host records a Retry when a vote has passed.
    fn share_the_log(&mut self) {
        self.take_over_if_due();
        for i in 0..self.apps.len() {
            // An app offline keeps its Run state queued until it is back.
            if self.offline.contains(&i) {
                continue;
            }
            let events = std::mem::take(&mut *self.apps[i].shared.lock().unwrap());
            for event in events {
                self.submit(i, &LogBody::Event(event));
            }
        }
        let host = self.host;
        if self.host_records_retries
            && let Some(call) = self.host_log.retry_due()
        {
            self.submit(host, &LogBody::Retry { call });
        }
        if let Some(player) = self.host_log.dropout_due() {
            self.submit(host, &LogBody::Dropout { player });
        }
        for app in &mut self.apps {
            let mut commands = app.follower.follow(&app.log);
            let judge = ByShadows(&app.shadows);
            let links = pactlocke_run::links(&app.log, &app.key, &judge);
            commands.extend(app.links.keep(&app.log, &links));
            for command in commands {
                app.running.commands().send(command).unwrap();
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
            let reachable = |key: PlayerKey| {
                self.apps
                    .iter()
                    .position(|a| a.key == key)
                    .is_some_and(|i| !self.offline.contains(&i))
            };
            if !reachable(from) {
                continue;
            }
            for to in targets {
                if !reachable(to) || (self.lose)(from, to, &message) {
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

    /// Every app draws its Partner Avatars, as the app does on an original game.
    pub fn draw_partner_avatars(&mut self) {
        for app in &mut self.apps {
            app.partners = Some(app.running.draw_partner_avatars());
        }
    }

    /// Player `i`'s game, to set what happens in it.
    pub fn game(&mut self, i: usize) -> &mut FakeGame {
        self.apps[i].running.game_mut()
    }

    /// Something happens in Player `i`'s game: in their game, and in every
    /// Shadow Game of it, which replays the same game (the fake's state isn't
    /// driven by keys, so the test sets it in each copy).
    pub fn happens(&mut self, i: usize, change: impl Fn(&mut FakeGame)) {
        change(self.apps[i].running.game_mut());
        let key = self.apps[i].key;
        for app in &mut self.apps {
            if let Some(shadow) = app.shadows.get_mut(&key) {
                change(shadow.game_mut());
            }
        }
    }

    pub fn walks_into(&mut self, i: usize, area: Area) {
        self.happens(i, |g| g.area = Some(area.clone()));
        self.plays(i, 10);
    }

    pub fn meets_wild(&mut self, i: usize, species: u16) {
        self.happens(i, |g| {
            g.opponent_party = vec![wild_pokemon(species)];
            g.battle = Some(battle(BattleKind::Wild, species));
        });
        self.plays(i, 10);
    }

    /// A trainer battle with the Player's first party Pokémon out.
    pub fn meets_trainer(&mut self, i: usize, species: u16) {
        self.happens(i, |g| {
            g.opponent_party = vec![wild_pokemon(species)];
            // The battle puts the first party Pokémon out, in position 0.
            g.battle = Some(battle(BattleKind::Trainer, species));
        });
        self.plays(i, 10);
    }

    /// The wild Pokémon is caught and joins the party, then the battle ends.
    pub fn catches(&mut self, i: usize, species: u16, pid: u32) {
        self.happens(i, |g| g.party.push(super::party_pokemon(species, pid, "")));
        self.plays(i, 10);
        self.leaves_battle(i);
    }

    /// The party Pokémon with this PID faints.
    pub fn faints(&mut self, i: usize, pid: u32) {
        self.happens(i, |g| {
            if let Some(p) = g.party.iter_mut().find(|p| p.pid == pid) {
                p.stats.as_mut().expect("a party Pokémon").hp = 0;
            }
        });
        self.plays(i, 10);
    }

    /// Player `i` talks to the gym leader at `gate`. A held gate runs its
    /// hold script ("Waiting for your partners…") and lets them go; a gate
    /// let go starts the battle. Says whether the battle started.
    pub fn talks_to(&mut self, i: usize, gate: &str) -> bool {
        if self.game(i).gates_closed.contains(gate) {
            let gate = gate.to_owned();
            self.happens(i, move |g| g.at_gate = Some(gate.clone()));
            self.plays(i, 10);
            self.happens(i, |g| g.at_gate = None);
            self.plays(i, 10);
            false
        } else {
            self.meets_trainer(i, super::CHARMANDER);
            true
        }
    }

    pub fn leaves_battle(&mut self, i: usize) {
        self.happens(i, |g| g.battle = None);
        self.plays(i, 10);
    }

    /// A memory edit, as a cheat tool makes: straight into the core, unstamped.
    pub fn edits_memory(&mut self, i: usize, addr: u32, bytes: &[u8]) {
        self.game(i).core.write(addr, bytes);
    }
}
