//! The in-process multi-Player harness: one to three Players' apps in one
//! process. The Host's setup and Run log, and each Player's view of setup,
//! own copy of the log and (after the lock) own Run engine on the fake game.
//! Every message between them crosses as the bytes the network would carry.

use pactlocke_game::CapabilityReport;
use pactlocke_run::{
    Locked, LogBody, LogEntry, MemoryStorage, Preset, RunLog, RunSetup, SetupCheck, SetupError, SetupMessage,
    SetupPlayer, SetupView, Signer, Submission,
};

use super::{Player, START_MS, TestClock};

pub const FIRERED_SHA1: &str = "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc";
pub const BUILD: &str = "0.1.0+test";

/// One Player's app.
pub struct CoopPlayer {
    pub name: String,
    pub signer: Signer,
    pub check: SetupCheck,
    /// Setup as this Player last heard it from the Host.
    pub view: Option<SetupView>,
    /// This Player's own copy of the Run log, checked entry by entry.
    pub log: RunLog,
    /// After the lock: this Player's Run, on their own game.
    pub run: Option<Player>,
    /// Entries the Host refused from this Player, and why.
    pub refused: Vec<String>,
}

impl CoopPlayer {
    pub fn key(&self) -> [u8; 32] {
        self.signer.public()
    }

    pub fn run(&mut self) -> &mut Player {
        self.run.as_mut().expect("the Run is locked and started")
    }
}

/// The Host (Player 0) and everyone who joined.
pub struct Coop {
    pub setup: RunSetup,
    pub host_log: RunLog,
    pub players: Vec<CoopPlayer>,
    pub clock: TestClock,
}

fn secret(i: usize) -> [u8; 32] {
    [i as u8 + 1; 32]
}

fn check() -> SetupCheck {
    SetupCheck {
        app_build: BUILD.into(),
        game_sha1: Some(FIRERED_SHA1.into()),
        hack_options: Vec::new(),
        randomized: Vec::new(),
    }
}

impl Coop {
    /// The Host opens setup on `capabilities` from `preset`; nobody else has joined yet.
    pub fn host(name: &str, capabilities: CapabilityReport, preset: Preset) -> Self {
        let host = CoopPlayer {
            name: name.into(),
            signer: Signer::from_secret(secret(0)),
            check: check(),
            view: None,
            log: RunLog::new(Signer::from_secret(secret(0)).public()),
            run: None,
            refused: Vec::new(),
        };
        let setup = RunSetup::new(
            SetupPlayer {
                key: host.key(),
                name: name.into(),
                check: host.check.clone(),
                ready_for: None,
            },
            "Kanto with friends",
            "Pokémon FireRed (USA) 1.0",
            FIRERED_SHA1,
            capabilities,
            Vec::new(),
            preset,
        );
        let mut coop = Self {
            setup,
            host_log: RunLog::new(host.key()),
            players: vec![host],
            clock: TestClock::at(START_MS),
        };
        let genesis = LogBody::Genesis {
            nonce: [7; 16],
            run_name: "Kanto with friends".into(),
            game_name: "Pokémon FireRed (USA) 1.0".into(),
            game_sha1: FIRERED_SHA1.into(),
        };
        coop.player_submits(0, &genesis);
        coop.broadcast();
        coop
    }

    /// The Host with Classic Nuzlocke on a game that can enforce everything,
    /// and these Players joined.
    pub fn with(names: &[&str]) -> Self {
        let mut coop = Self::host(
            names[0],
            CapabilityReport::everything_enforceable(),
            Preset::ClassicNuzlocke,
        );
        for name in &names[1..] {
            coop.join(name).unwrap();
        }
        coop
    }

    /// A Player joins by invite (the lobby admitted them) and tells setup what they bring.
    pub fn join(&mut self, name: &str) -> Result<usize, SetupError> {
        let i = self.players.len();
        let player = CoopPlayer {
            name: name.into(),
            signer: Signer::from_secret(secret(i)),
            check: check(),
            view: None,
            log: RunLog::new(self.players[0].key()),
            run: None,
            refused: Vec::new(),
        };
        self.setup.join(SetupPlayer {
            key: player.key(),
            name: name.into(),
            check: player.check.clone(),
            ready_for: None,
        })?;
        self.players.push(player);
        // A Player who joins gets the whole log so far, and checks it.
        let entries: Vec<Vec<u8>> = self.host_log.entries().iter().map(LogEntry::to_bytes).collect();
        for bytes in entries {
            let entry = LogEntry::from_bytes(&bytes).unwrap();
            self.players[i].log.accept(entry).unwrap();
        }
        self.broadcast();
        Ok(i)
    }

    /// A Player's setup message, as bytes to the Host, then the Host's new state to everyone.
    pub fn send(&mut self, i: usize, message: SetupMessage) {
        let bytes = message.to_bytes();
        let from = self.players[i].key();
        self.setup.receive(&from, SetupMessage::from_bytes(&bytes).unwrap());
        self.broadcast();
    }

    /// A Player changes what they bring (a different ROM, say).
    pub fn brings(&mut self, i: usize, change: impl FnOnce(&mut SetupCheck)) {
        change(&mut self.players[i].check);
        let check = self.players[i].check.clone();
        self.send(i, SetupMessage::Check(check));
    }

    /// The Player says Ready to the Ruleset as they see it.
    pub fn ready(&mut self, i: usize) {
        let version = self.players[i].view.as_ref().expect("a view").version;
        self.send(i, SetupMessage::Ready { version });
    }

    pub fn everyone_ready(&mut self) {
        for i in 0..self.players.len() {
            self.ready(i);
        }
    }

    /// Whether Player `i` is Ready, as Player `seen_by` sees it.
    pub fn is_ready(&self, i: usize, seen_by: usize) -> bool {
        let view = self.players[seen_by].view.as_ref().expect("a view");
        view.is_ready(&self.players[i].key())
    }

    /// The Host's state, as bytes, to every Player.
    pub fn broadcast(&mut self) {
        let bytes = SetupMessage::State(Box::new(self.setup.view())).to_bytes();
        for player in &mut self.players {
            if let Some(SetupMessage::State(view)) = SetupMessage::from_bytes(&bytes) {
                player.view = Some(*view);
            }
        }
    }

    /// The Host locks: the lock goes in the log, every Player checks it, and
    /// every Player's Run starts with the locked Ruleset on the lock's clock.
    pub fn lock(&mut self) -> Result<Locked, SetupError> {
        let locked = self.setup.lock(self.clock_now())?;
        self.player_submits(0, &LogBody::Locked(locked.clone()));
        self.broadcast();
        for player in &mut self.players {
            let clock = TestClock::at(locked.start_ms);
            let storage = MemoryStorage::default();
            let mut run = Player::starts_a_run_in(Box::new(storage.clone()), clock, locked.ruleset.clone());
            run.storage = storage;
            player.run = Some(run);
        }
        Ok(locked)
    }

    fn clock_now(&self) -> i64 {
        use pactlocke_run::RunClock;
        self.clock.now_ms()
    }

    /// Player `i`'s Run state so far goes to the Host, who orders it; every
    /// Player then gets the new entries and checks them.
    pub fn sync(&mut self) {
        for i in 0..self.players.len() {
            let Some(run) = self.players[i].run.as_mut() else {
                continue;
            };
            let events = run.engine.take_shared();
            for event in events {
                self.player_submits(i, &LogBody::Event(event));
            }
        }
    }

    /// Player `i` signs an entry and sends it to the Host, who orders it and sends it to everyone.
    pub fn player_submits(&mut self, i: usize, body: &LogBody) {
        let counter = self.host_log.next_counter(&self.players[i].key());
        let submission = self.players[i].log.sign(&self.players[i].signer, counter, body);
        self.deliver(i, submission);
    }

    /// A submission as bytes to the Host; the ordered entry as bytes to everyone.
    pub fn deliver(&mut self, from: usize, submission: Submission) {
        let bytes = serde_json::to_vec(&submission).unwrap();
        let submission: Submission = serde_json::from_slice(&bytes).unwrap();
        let host = Signer::from_secret(secret(0));
        match self.host_log.order(&host, submission) {
            Ok(entry) => self.send_to_everyone(entry),
            Err(e) => self.players[from].refused.push(e.to_string()),
        }
    }

    /// An entry as the Host sends it to every Player, each of whom checks it.
    pub fn send_to_everyone(&mut self, entry: LogEntry) {
        let bytes = entry.to_bytes();
        for player in &mut self.players {
            let entry = LogEntry::from_bytes(&bytes).unwrap();
            if let Err(e) = player.log.accept(entry) {
                player.refused.push(e.to_string());
            }
        }
    }
}
