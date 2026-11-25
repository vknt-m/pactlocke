//! The headless app: `pactlocke --headless SCRIPT [--report FILE] [--data DIR]`.
//! No window, no sound: it follows a key script (see [`script`]) through a
//! lobby and a Run with other Players, exactly as the windowed app plays
//! one, and writes what it saw to a JSON report. The multi-machine test rig
//! (`scripts/rig.sh`) runs one in each container.
//!
//! Exit codes: 0 every step passed, 1 a step failed (the report says which),
//! 2 the script or its arguments are wrong, 3 the script said `vanish`.

pub mod script;

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{CapabilityReport, Change, GameProfile, ProfileGame, ProfileStore, RomIdentity, sha1_hex};
use pactlocke_mgba::MgbaCore;
use pactlocke_net::{Relays, RunAdvert};
use pactlocke_run::{
    NewRun, PlayerKey, RetryTo, RunClock, RunCommand, RunEngine, RunEvent, RunLibrary, RunLog, RunningGame,
    ShadowState, SystemRunClock,
};
use serde_json::{Value, json};

use crate::coop::{CoopStart, HostRom};
use crate::lobby_ui::Lobbies;
use crate::spectate::Spectate;
use script::{Addr, Check, RelayArg, Step};

/// A GBA frame at 1x, in microseconds (59.73 frames a second).
const FRAME_US: u64 = 16_742;
/// The longest a lobby step (joining, locking, the Run starting, `sync`) waits.
const LOBBY_WAIT: Duration = Duration::from_secs(300);
/// The program the rig gives for cutting this app's network: it is run as `CUT SECS`.
pub const CUT_VAR: &str = "PACTLOCKE_CUT";

/// Runs the headless app with these arguments (after `--headless`).
pub fn main(args: &[String]) -> std::process::ExitCode {
    let mut args = args.iter();
    let (mut script, mut report, mut data) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--report" => report = args.next().map(PathBuf::from),
            "--data" => data = args.next().map(PathBuf::from),
            _ if script.is_none() => script = Some(PathBuf::from(arg)),
            other => return usage(&format!("unexpected argument {other}")),
        }
    }
    let Some(script_path) = script else {
        return usage("no script");
    };
    let data = data.unwrap_or_else(|| std::env::temp_dir().join(format!("pactlocke-headless-{}", std::process::id())));
    if let Err(e) = std::fs::create_dir_all(&data) {
        return usage(&format!("{} can't be made: {e}", data.display()));
    }
    crate::logger::init_at(data.join("pactlocke.log"), true);
    let steps = match script::load(&script_path).and_then(|text| script::parse(&text)) {
        Ok(steps) => steps,
        Err(e) => return usage(&e),
    };
    let mut app = match Headless::new(&data) {
        Ok(app) => app,
        Err(e) => return usage(&e),
    };
    let outcome = app.follow(&steps);
    let report_json = app.report(&outcome);
    // A failed step: the screen as it was, beside the report.
    if let (Outcome::Failed { .. }, Some(path)) = (&outcome, &report)
        && app.game.is_some()
        && let Err(e) = app.shot(&path.with_extension("png"))
    {
        log::warn!("{e}");
    }
    if let Some(path) = &report {
        let text = serde_json::to_string_pretty(&report_json).expect("a report serialises");
        if let Err(e) = std::fs::write(path, text) {
            log::error!("The report can't be written to {}: {e}", path.display());
        }
    }
    match outcome {
        Outcome::Passed => {
            log::info!("Every step passed");
            std::process::ExitCode::SUCCESS
        }
        Outcome::Failed { line, why } => {
            log::error!("Line {line} failed: {why}");
            std::process::ExitCode::from(1)
        }
        Outcome::Vanished => {
            log::info!("Vanishing");
            log::logger().flush();
            // Gone at once, the way a machine that vanished is: no goodbyes
            // on the network, nothing flushed or closed.
            std::process::exit(3)
        }
    }
}

fn usage(why: &str) -> std::process::ExitCode {
    eprintln!("pactlocke --headless SCRIPT [--report FILE] [--data DIR]: {why}");
    std::process::ExitCode::from(2)
}

/// How following the script ended.
#[derive(Debug)]
enum Outcome {
    Passed,
    Failed { line: usize, why: String },
    Vanished,
}

/// The Run's game, played here, and this app's side of everyone else's.
struct Game {
    running: RunningGame<ProfileGame<MgbaCore>>,
    commands: Sender<RunCommand>,
    shared: Arc<Mutex<Vec<RunEvent>>>,
    spectate: Spectate,
}

/// A headless app part way through its script.
struct Headless {
    lobbies: Lobbies,
    profiles: ProfileStore,
    library: RunLibrary,
    /// The Host writes invites here, numbered, and how many it has written.
    invites: Option<(PathBuf, usize)>,
    /// The invite last written, to know when it has been used.
    last_invite: Option<String>,
    shared: Option<PathBuf>,
    /// This Player wants to be Ready; the Ruleset version they last said Ready to.
    ready: Option<Option<u64>>,
    game: Option<Game>,
    speed: u8,
    /// When the next frame is due.
    next_frame: Instant,
    /// Each step that passed, by line.
    passed: Vec<usize>,
    /// Sync Gates the script added to the Game Profile.
    extra_gates: Vec<pactlocke_game::GateDef>,
}

impl Headless {
    fn new(data: &Path) -> Result<Self, String> {
        let mut lobbies = Lobbies::new(&data.join("identity.key"));
        lobbies.name = "Player".into();
        let profiles = ProfileStore::from_env().map_err(|e| format!("The Game Profiles can't be read: {e}"))?;
        let library =
            RunLibrary::open(data.join("runs")).map_err(|e| format!("The Runs folder can't be opened: {e}"))?;
        Ok(Self {
            lobbies,
            profiles,
            library,
            invites: None,
            last_invite: None,
            shared: None,
            ready: None,
            game: None,
            speed: 1,
            next_frame: Instant::now(),
            passed: Vec::new(),
            extra_gates: Vec::new(),
        })
    }

    fn follow(&mut self, steps: &[(usize, Step)]) -> Outcome {
        for (line, step) in steps {
            log::info!("Line {line}: {step:?}");
            match self.step(step) {
                Ok(true) => self.passed.push(*line),
                Ok(false) => return Outcome::Vanished,
                Err(why) => return Outcome::Failed { line: *line, why },
            }
        }
        Outcome::Passed
    }

    /// Carries out one step. `Ok(false)`: the app vanishes here.
    fn step(&mut self, step: &Step) -> Result<bool, String> {
        match step {
            Step::Name(name) => self.lobbies.name = name.clone(),
            Step::ExtraGate { name, sites } => self.extra_gates.push(pactlocke_game::GateDef {
                name: name.clone(),
                sites: sites
                    .iter()
                    .map(|(at, expect)| pactlocke_game::PatchSite {
                        at: *at,
                        expect: pactlocke_game::HexBytes(expect.clone()),
                        write: None,
                        below_cap: 0,
                    })
                    .collect(),
            }),
            Step::Relays(relays) => {
                self.lobbies.relays = match relays {
                    RelayArg::N0 => Relays::N0,
                    RelayArg::Off => Relays::Off,
                    RelayArg::Url(url) => Relays::Custom(vec![url.clone()]),
                }
            }
            Step::Host { rom } => self.host(rom)?,
            Step::Join { invite, rom } => self.join(invite, rom)?,
            Step::Invites(dir) => {
                std::fs::create_dir_all(dir).map_err(|e| format!("{} can't be made: {e}", dir.display()))?;
                self.invites = Some((dir.clone(), 0));
            }
            Step::Shared(dir) => {
                std::fs::create_dir_all(dir).map_err(|e| format!("{} can't be made: {e}", dir.display()))?;
                self.shared = Some(dir.clone());
                // Where a Player who takes over hosting says how to reach them.
                self.lobbies.addresses = Some(dir.clone());
            }
            Step::Preset(preset) => self.host_setup()?.choose(*preset),
            Step::Rule { rule, on } => {
                let (rule, on) = (rule.clone(), *on);
                self.host_setup()?.edit(|ruleset, caps| {
                    script::set_rule(ruleset, &rule, on, caps);
                })?;
            }
            Step::Retries(n) => {
                let n = *n;
                self.host_setup()?.edit(|ruleset, _| ruleset.retries = n)?;
            }
            Step::Gate { gate, on } => {
                let (gate, on) = (gate.clone(), *on);
                self.host_setup()?.edit(|ruleset, _| {
                    ruleset.sync_gates.retain(|g| *g != gate);
                    if on {
                        ruleset.sync_gates.push(gate);
                    }
                })?;
            }
            Step::Ready => self.ready = Some(None),
            Step::Players(players) => self.wait_for_players(*players)?,
            Step::LockWhen(players) => self.lock_when(*players)?,
            Step::Hold { keys, frames } => self.frames(*frames, |_| *keys)?,
            Step::Mash { keys, frames } => self.frames(*frames, |i| script::mashed(*keys, i))?,
            Step::Press { keys, wait } => {
                self.frames(4, |_| *keys)?;
                self.frames(*wait, |_| Keys::NONE)?;
            }
            Step::Wait(frames) => self.frames(*frames, |_| Keys::NONE)?,
            Step::Until { cond, action, limit } => {
                self.started()?;
                for i in 0..*limit {
                    let value = self.read(cond.addr, cond.width);
                    if cond.holds(value) {
                        return Ok(true);
                    }
                    self.tick(action.keys(i));
                }
                return Err(format!("not met in {limit} frames"));
            }
            Step::Speed(speed) => {
                self.speed = *speed;
                if let Some(game) = &self.game {
                    let _ = game.commands.send(RunCommand::Speed(*speed));
                }
            }
            Step::Poke { addr, bytes } => {
                self.started()?;
                let addr = self.resolve(*addr);
                self.game_mut()?.running.apply_stamped(Change::Write {
                    addr,
                    bytes: bytes.clone(),
                });
            }
            Step::CheatPoke { addr, bytes } => {
                self.started()?;
                let addr = self.resolve(*addr);
                self.game_mut()?.running.write(addr, bytes);
            }
            Step::CheatSavestateLoad => {
                self.started()?;
                let game = self.game_mut()?;
                let hash = game
                    .running
                    .engine()
                    .latest_run_checkpoint()
                    .ok_or("there is no Run Checkpoint to load yet")?
                    .to_owned();
                game.running.retry(&RetryTo::Checkpoint(hash))?;
                log::warn!("Loaded a savestate: back to the latest Run Checkpoint, with no Retry");
            }
            Step::VoteRetry => self.vote_retry()?,
            Step::Cut { secs } => cut(*secs)?,
            Step::Sync { label, players } => self.sync(label, *players)?,
            Step::Expect { check, within_s } => self.expect(check, Duration::from_secs(*within_s))?,
            Step::Shot(path) => {
                self.started()?;
                self.shot(path)?;
            }
            Step::Vanish => return Ok(false),
            Step::Leave => self.leave(),
            Step::Away { secs } => {
                let until = Instant::now() + Duration::from_secs(*secs);
                while Instant::now() < until {
                    self.tick(Keys::NONE);
                }
            }
            Step::Resume => self.resume()?,
        }
        Ok(true)
    }

    fn host_setup(&mut self) -> Result<&mut crate::coop::HostCoop, String> {
        self.lobbies
            .host_setup()
            .ok_or_else(|| "only the Host sets the Ruleset".into())
    }

    fn game_mut(&mut self) -> Result<&mut Game, String> {
        self.game.as_mut().ok_or_else(|| "the Run hasn't started".into())
    }

    /// Opens a lobby on this ROM, as New Run → Invite friends does.
    fn host(&mut self, rom_path: &Path) -> Result<(), String> {
        let rom = std::fs::read(rom_path).map_err(|e| format!("{} can't be read: {e}", rom_path.display()))?;
        let (profile, capabilities) = self.profiled(&rom)?;
        let run = RunAdvert {
            name: "Rig Run".into(),
            game_name: profile.name.clone(),
            game_sha1: profile.sha1.clone(),
        };
        let host_rom = HostRom {
            capabilities,
            level_caps: profile.default_level_caps(&rom),
            sync_gates: profile.sync_gate_names(),
            rom,
            path: rom_path.to_owned(),
        };
        self.lobbies.host(run, host_rom);
        Ok(())
    }

    /// The ROM's Game Profile, with the script's extra Sync Gates, and what it can enforce.
    fn profiled(&self, rom: &[u8]) -> Result<(Arc<GameProfile>, CapabilityReport), String> {
        let RomIdentity::Profiled { profile, .. } = self.profiles.identify(rom) else {
            return Err("this ROM has no Game Profile".into());
        };
        let mut profile = (*profile).clone();
        if !self.extra_gates.is_empty() {
            let gates = profile
                .sync_gates
                .as_mut()
                .ok_or("this Game Profile has no Sync Gates to add to")?;
            gates.gates.extend(self.extra_gates.iter().cloned());
        }
        let capabilities = CapabilityReport::check(&profile, rom);
        Ok((Arc::new(profile), capabilities))
    }

    /// Joins with the invite in `invite` once the Host has written it.
    fn join(&mut self, invite: &Path, rom_path: &Path) -> Result<(), String> {
        let rom = std::fs::read(rom_path).map_err(|e| format!("{} can't be read: {e}", rom_path.display()))?;
        let started = Instant::now();
        let link = loop {
            if let Ok(link) = std::fs::read_to_string(invite) {
                break link.trim().to_owned();
            }
            if started.elapsed() > LOBBY_WAIT {
                return Err(format!("no invite at {}", invite.display()));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        // Beside the invite, the Host's direct addresses, for a rig with no relay.
        let direct = std::fs::read_to_string(invite.with_extension("addrs"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        self.lobbies
            .join(&link, Some((rom_path.to_owned(), sha1_hex(&rom))), direct)
    }

    /// The Host waits for `players` in setup.
    fn wait_for_players(&mut self, players: usize) -> Result<(), String> {
        let started = Instant::now();
        while self.host_setup()?.players() < players {
            if started.elapsed() > LOBBY_WAIT {
                return Err(format!(
                    "{} of {players} Players in setup",
                    self.host_setup()?.players()
                ));
            }
            self.tick(Keys::NONE);
        }
        Ok(())
    }

    /// The Host waits for `players` in setup, every one Ready, then locks.
    fn lock_when(&mut self, players: usize) -> Result<(), String> {
        let started = Instant::now();
        loop {
            self.tick(Keys::NONE);
            let setup = self.host_setup()?;
            if setup.players() >= players && setup.not_ready().is_empty() {
                break;
            }
            if started.elapsed() > LOBBY_WAIT {
                let why = setup.not_ready().join("; ");
                return Err(format!("{} of {players} Players in setup: {why}", setup.players()));
            }
        }
        self.lobbies.lock(SystemRunClock.now_ms())
    }

    /// Waits for the Run to start, if it hasn't.
    fn started(&mut self) -> Result<(), String> {
        let started = Instant::now();
        while self.game.is_none() {
            if started.elapsed() > LOBBY_WAIT {
                return Err(format!(
                    "the Run never started{}",
                    self.lobbies.error().map(|e| format!(": {e}")).unwrap_or_default()
                ));
            }
            self.tick(Keys::NONE);
        }
        Ok(())
    }

    /// Plays `frames` frames, with `keys(i)` held on frame `i`.
    fn frames(&mut self, frames: u32, keys: impl Fn(u32) -> Keys) -> Result<(), String> {
        self.started()?;
        for i in 0..frames {
            self.tick(keys(i));
        }
        Ok(())
    }

    /// One frame's time: the game runs a frame if the Run has started, and
    /// the lobby, the streams and the Run log do their work.
    fn tick(&mut self, keys: Keys) {
        let frame = Duration::from_micros(FRAME_US / u64::from(self.speed));
        let now = Instant::now();
        if self.next_frame > now {
            std::thread::sleep(self.next_frame - now);
        } else if now - self.next_frame > Duration::from_millis(100) {
            // Too far behind to catch up: carry on from now, never in a burst.
            self.next_frame = now;
        }
        self.next_frame += frame;
        if let Some(game) = self.game.as_mut() {
            game.running.run_frame(keys);
        }
        if let Some(start) = self.lobbies.pump(SystemRunClock.now_ms())
            && let Err(e) = self.start(start)
        {
            log::error!("The Run can't start: {e}");
        }
        self.write_invites();
        self.keep_ready();
        if let Some(game) = self.game.as_mut() {
            game.spectate.pump(&mut self.lobbies);
            let events = std::mem::take(&mut *game.shared.lock().unwrap_or_else(|e| e.into_inner()));
            if !events.is_empty() {
                self.lobbies.share(events);
            }
            if let Some(log) = self.lobbies.log() {
                for command in game.spectate.follow(log) {
                    let _ = game.commands.send(command);
                }
            }
        }
    }

    /// The lock's countdown is over: this Player's Run starts from a new
    /// game, as the windowed app starts it.
    fn start(&mut self, start: CoopStart) -> Result<(), String> {
        let locked = start.locked;
        let file = self.library.create().map_err(|e| e.to_string())?;
        let new = NewRun {
            name: locked.run_name.clone(),
            game_name: locked.game_name.clone(),
            game_sha1: locked.game_sha1.clone(),
            rom_path: Some(start.path.display().to_string()),
            ruleset: locked.ruleset.clone(),
        };
        let engine = RunEngine::start(new, Box::new(file), Box::new(SystemRunClock)).map_err(|e| e.to_string())?;
        let start_ms = locked.start_ms;
        self.play(engine, start.rom, &locked, start_ms, None)?;
        log::info!("The Run started: {} on {}", locked.run_name, locked.game_name);
        Ok(())
    }

    /// Plays the Run `engine` is the engine of, booted as it planned on
    /// `rom`, with Shadow Games of the others; a resumed Run comes with its
    /// log as its Run file kept it.
    fn play(
        &mut self,
        engine: RunEngine,
        rom: Vec<u8>,
        locked: &pactlocke_run::Locked,
        clock_ms: i64,
        resumed: Option<&pactlocke_run::RunLog>,
    ) -> Result<(), String> {
        let (profile, _) = self.profiled(&rom)?;
        let mut core = MgbaCore::new();
        core.load_rom(&rom).map_err(|e| e.to_string())?;
        let rom: Arc<[u8]> = rom.into();
        let mut running = RunningGame::boot(ProfileGame::new(profile.clone(), rom.clone(), core), engine, clock_ms)
            .map_err(|e| format!("the Run's game couldn't be booted: {e}"))?;
        let commands = running.commands();
        let shared = running.share();
        let me = self.lobbies.public_key().ok_or("no network key")?;
        let outbox = running.stream_as(me);
        let kept = running.engine().kept_shadows().to_vec();
        let mut spectate = Spectate::new(me, locked, &|_| rom.clone(), &profile, outbox, &commands, &kept)?;
        if let Some(log) = resumed {
            spectate.resumed(log, running.streamed_before());
        }
        spectate.draw_partner_avatars(running.draw_partner_avatars());
        let _ = commands.send(RunCommand::Speed(self.speed));
        self.game = Some(Game {
            running,
            commands,
            shared,
            spectate,
        });
        self.next_frame = Instant::now();
        Ok(())
    }

    /// Close Run, as the windowed app closes a Run with friends: each
    /// Shadow Game's checkpoint is kept, the game suspended, the network left.
    fn leave(&mut self) {
        if let Some(game) = self.game.take() {
            game.spectate.keep_checkpoints(true);
            game.running.suspend();
        }
        self.lobbies.leave_run();
        log::info!("Left the Run");
    }

    /// Resume, as the windowed app resumes a Run with friends: the newest
    /// one in this app's Runs, on the ROM it was played on.
    fn resume(&mut self) -> Result<(), String> {
        let summary = self
            .library
            .list()
            .into_iter()
            .find(|r| r.with_friends)
            .ok_or("there is no Run with friends to resume")?;
        let path = PathBuf::from(summary.rom_path.clone().ok_or("the Run has no ROM path")?);
        let rom = std::fs::read(&path).map_err(|e| format!("{} can't be read: {e}", path.display()))?;
        let file = self.library.open_run(&summary.id).map_err(|e| e.to_string())?;
        let engine = RunEngine::resume(Box::new(file), Box::new(SystemRunClock)).map_err(|e| e.to_string())?;
        let mut resuming = crate::resume::Resuming::begin(engine, path, &mut self.lobbies, SystemRunClock.now_ms())?;
        while !resuming.ready_to_boot(&mut self.lobbies, SystemRunClock.now_ms()) {
            self.tick(Keys::NONE);
        }
        let crate::resume::Resuming {
            engine, locked, log, ..
        } = resuming;
        self.play(engine, rom, &locked, SystemRunClock.now_ms(), Some(&log))?;
        log::info!("Resumed the Run {}", locked.run_name);
        Ok(())
    }

    /// The Host writes each invite to the next numbered file once the last was used.
    fn write_invites(&mut self) {
        let Some((dir, written)) = self.invites.as_mut() else {
            return;
        };
        let Some((link, addrs)) = self.lobbies.invite() else {
            return;
        };
        if self.last_invite.as_deref() == Some(link.as_str()) {
            // Used once someone new is in setup: then a fresh link for the next friend.
            let used = self.lobbies.host_setup().is_some_and(|s| s.players() > *written);
            if used {
                self.lobbies.new_invite();
            }
            return;
        }
        *written += 1;
        let file = dir.join(format!("invite-{written}"));
        // With no relay, joiners need the Host's addresses; with one, they
        // reach the Host through it, as friends over the internet do.
        let addrs: Vec<String> = match self.lobbies.relays {
            Relays::Off => addrs.iter().map(ToString::to_string).collect(),
            _ => Vec::new(),
        };
        // The address file first: a joiner reads it once the invite is there.
        let _ = std::fs::write(file.with_extension("addrs"), addrs.join("\n"));
        let temp = file.with_extension("tmp");
        if std::fs::write(&temp, &link)
            .and_then(|()| std::fs::rename(&temp, &file))
            .is_ok()
        {
            log::info!("Wrote {}", file.display());
            self.last_invite = Some(link);
        }
    }

    /// Ready to the Ruleset as it is now, again whenever the Host changes it.
    fn keep_ready(&mut self) {
        let Some(said) = self.ready else {
            return;
        };
        if self.lobbies.log().is_some_and(|log| log.locked().is_some()) {
            self.ready = None;
            return;
        }
        let Some(version) = self.lobbies.setup_version() else {
            return;
        };
        if said != Some(version) {
            self.lobbies.ready(true);
            self.ready = Some(Some(version));
        }
    }

    fn vote_retry(&mut self) -> Result<(), String> {
        self.started()?;
        let log = self.lobbies.log().ok_or("not in a Run with others")?;
        let call = match log.vote() {
            Some(vote) => vote.call,
            None => log
                .retry_points()
                .last()
                .map(|c| c.n)
                .ok_or("there is no Run Checkpoint to go back to yet")?,
        };
        self.lobbies.share(vec![RunEvent::RetryVote { call, yes: true }]);
        Ok(())
    }

    /// Waits until every Player reached `label`, playing on meanwhile.
    fn sync(&mut self, label: &str, players: usize) -> Result<(), String> {
        let dir = self.shared.clone().ok_or("sync needs a shared folder")?;
        let me = self.lobbies.name.replace(['/', '\\'], "_");
        std::fs::write(dir.join(format!("sync-{label}-{me}")), b"")
            .map_err(|e| format!("{} can't be written: {e}", dir.display()))?;
        let prefix = format!("sync-{label}-");
        let started = Instant::now();
        loop {
            let there = std::fs::read_dir(&dir)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                        .count()
                })
                .unwrap_or(0);
            if there >= players {
                return Ok(());
            }
            if started.elapsed() > LOBBY_WAIT {
                return Err(format!("{there} of {players} Players reached {label}"));
            }
            self.tick(Keys::NONE);
        }
    }

    /// Plays on until `check` holds, or fails after `within`.
    fn expect(&mut self, check: &Check, within: Duration) -> Result<(), String> {
        let started = Instant::now();
        loop {
            let Some(why) = self.why_not(check) else {
                return Ok(());
            };
            if started.elapsed() > within {
                return Err(why);
            }
            self.tick(Keys::NONE);
        }
    }

    /// Why `check` doesn't hold now, or `None` if it does.
    fn why_not(&self, check: &Check) -> Option<String> {
        match check {
            Check::Matched => {
                let Some(game) = &self.game else {
                    return Some("the Run hasn't started".into());
                };
                let off: Vec<String> = game
                    .spectate
                    .shadows
                    .iter()
                    .filter_map(|s| {
                        let status = s.status();
                        let fine = status.state == ShadowState::Matched && !status.catching_up;
                        (!fine).then(|| format!("{}: {:?}, catching up {}", s.name, status.state, status.catching_up))
                    })
                    .collect();
                (!off.is_empty()).then(|| off.join("; "))
            }
            Check::Connected => {
                let Some(locked) = self.lobbies.log().and_then(RunLog::locked) else {
                    return Some("the Run isn't locked".into());
                };
                let me = self.lobbies.public_key()?;
                let missing: Vec<&str> = locked
                    .roster
                    .iter()
                    .filter(|p| p.key != me && (!self.lobbies.online() || self.lobbies.reach(p.key).is_none()))
                    .map(|p| p.name.as_str())
                    .collect();
                (!missing.is_empty()).then(|| format!("can't reach {}", missing.join(", ")))
            }
            Check::NoErrors => self.lobbies.error(),
            Check::Reconnected => {
                if self.lobbies.reconnects() == 0 {
                    return Some("the Host was never lost".into());
                }
                self.why_not(&Check::Connected)
            }
            Check::Boot(boot) => {
                let Some(game) = &self.game else {
                    return Some("the Run hasn't started".into());
                };
                let own = game.running.engine().boot();
                let behind: Vec<String> = game
                    .spectate
                    .shadows
                    .iter()
                    .filter(|s| s.status().boot < *boot)
                    .map(|s| format!("the Shadow Game of {} is on boot {}", s.name, s.status().boot))
                    .chain((own < *boot).then(|| format!("this game is on boot {own}")))
                    .collect();
                (!behind.is_empty()).then(|| behind.join("; "))
            }
            Check::Log {
                path,
                by,
                about,
                fields,
                count,
            } => {
                let Some(log) = self.lobbies.log() else {
                    return Some("not in a lobby".into());
                };
                let key_of = |name: &str| {
                    log.locked()
                        .and_then(|l| l.roster.iter().find(|p| p.name == name).map(|p| p.key))
                };
                let (by, about) = (by.as_deref().map(key_of), about.as_deref().map(key_of));
                if by.is_some_and(|k| k.is_none()) || about.is_some_and(|k| k.is_none()) {
                    return Some("no such Player in the roster".into());
                }
                let found = log
                    .entries()
                    .iter()
                    .filter(|entry| by.is_none_or(|k| k == Some(entry.submission.author)))
                    .filter_map(|entry| serde_json::from_slice::<Value>(&entry.submission.body).ok())
                    .filter(|body| matches(body, path, fields, about.flatten()))
                    .count();
                (found < *count).then(|| format!("{found} of {count} such entries in the Run log"))
            }
        }
    }

    /// Saves the game's screen now.
    fn shot(&self, path: &Path) -> Result<(), String> {
        let game = self.game.as_ref().ok_or("the Run hasn't started")?;
        crate::write_png(path, &game.running.frame()).map_err(|e| format!("{} can't be written: {e}", path.display()))
    }

    fn resolve(&mut self, addr: Addr) -> u32 {
        match addr {
            Addr::At(addr) => addr,
            Addr::Deref { pointer, offset } => self.read(Addr::At(pointer), 4).wrapping_add(offset),
        }
    }

    /// `width` bytes at `addr`, little-endian.
    fn read(&mut self, addr: Addr, width: u8) -> u32 {
        let addr = self.resolve(addr);
        let Some(game) = self.game.as_mut() else {
            return 0;
        };
        let mut bytes = [0; 4];
        game.running.read(addr, &mut bytes[..usize::from(width)]);
        u32::from_le_bytes(bytes)
    }

    /// What this app saw, for the rig to compare across Players.
    fn report(&self, outcome: &Outcome) -> Value {
        let log = self.lobbies.log();
        let roster = log.and_then(RunLog::locked).map(|l| l.roster).unwrap_or_default();
        let name_of = |key: &PlayerKey| {
            roster
                .iter()
                .find(|p| p.key == *key)
                .map_or_else(|| hex(key), |p| p.name.clone())
        };
        let entries: Vec<Value> = log
            .map(|log| {
                log.entries()
                    .iter()
                    .map(|entry| {
                        let body: Value = serde_json::from_slice(&entry.submission.body).unwrap_or(Value::Null);
                        json!({ "seq": entry.seq, "by": name_of(&entry.submission.author), "kind": kinds(&body) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let log_hash = log.map(|log| {
            let all: Vec<u8> = log.entries().iter().flat_map(|e| e.to_bytes()).collect();
            sha1_hex(&all)
        });
        let shadows: Vec<Value> = self
            .game
            .as_ref()
            .map(|game| {
                game.spectate
                    .shadows
                    .iter()
                    .map(|s| {
                        let status = s.status();
                        json!({
                            "name": s.name,
                            "state": format!("{:?}", status.state),
                            "frame": status.frame,
                            "received": status.received,
                            "catching_up": status.catching_up,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Every Link as this app judges it, for reading a failed scenario.
        let links: Vec<String> = match (&self.game, log) {
            (Some(game), Some(log)) => game.spectate.links(log).iter().map(|l| format!("{l:?}")).collect(),
            _ => Vec::new(),
        };
        let (failed, vanished) = match outcome {
            Outcome::Failed { line, why } => (json!({ "line": line, "why": why }), false),
            Outcome::Passed => (Value::Null, false),
            Outcome::Vanished => (Value::Null, true),
        };
        json!({
            "name": self.lobbies.name,
            "key": self.lobbies.public_key().map(|k| hex(&k)),
            "passed": self.passed,
            "failed": failed,
            "vanished": vanished,
            "frames": self.game.as_ref().map(|g| g.running.engine().frames_run()),
            "error": self.lobbies.error(),
            "log": { "entries": entries, "hash": log_hash },
            "shadows": shadows,
            "links": links,
        })
    }
}

/// Asks the rig to cut this app's network for `secs`; the game plays on. The
/// command returns once the cut is made, and brings the network back itself.
fn cut(secs: u64) -> Result<(), String> {
    let program = std::env::var_os(CUT_VAR).ok_or_else(|| format!("cut needs {CUT_VAR}, which the rig sets"))?;
    let status = std::process::Command::new(program)
        .arg(secs.to_string())
        .status()
        .map_err(|e| format!("the cut command didn't start: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the cut command failed ({status}): the network wasn't cut"))
    }
}

/// Whether a log entry's body follows `path` (each level's `kind`) and has
/// every field given, and a `player` field naming `about` if given.
fn matches(body: &Value, path: &[String], fields: &[(String, String)], about: Option<PlayerKey>) -> bool {
    let mut at = body;
    for kind in path {
        if at.get("kind").and_then(Value::as_str) != Some(kind.as_str()) {
            return false;
        }
        // An entry's body is under `body`; an event's fields sit beside its kind.
        if let Some(inner) = at.get("body") {
            at = inner;
        }
    }
    let fields_hold = fields.iter().all(|(key, want)| {
        at.get(key).is_some_and(|v| match v {
            Value::String(s) => s == want,
            other => *other == serde_json::from_str::<Value>(want).unwrap_or(Value::Null),
        })
    });
    let about_holds = about.is_none_or(|key| {
        at.get("player")
            .and_then(|v| serde_json::from_value::<PlayerKey>(v.clone()).ok())
            == Some(key)
    });
    fields_hold && about_holds
}

/// An entry's kinds, outermost first: `event died`.
fn kinds(body: &Value) -> String {
    let mut out = Vec::new();
    let mut at = body;
    while let Some(kind) = at.get("kind").and_then(Value::as_str) {
        out.push(kind.to_owned());
        match at.get("body") {
            Some(inner) => at = inner,
            None => break,
        }
    }
    out.join(" ")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pactlocke_run::{Cheat, LogBody};

    fn body(b: &LogBody) -> Value {
        serde_json::to_value(b).unwrap()
    }

    #[test]
    fn expect_log_matches_kinds_fields_and_whom_a_flag_is_about() {
        let blue = [2; 32];
        let flag = body(&LogBody::Event(RunEvent::CheatFlag {
            player: Some(blue),
            cheat: Cheat::RestartedWithoutRetry,
            boot: 2,
            frame: 900,
            at_ms: 0,
        }));
        let path = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let field = |k: &str, v: &str| vec![(k.to_owned(), v.to_owned())];
        assert_eq!(kinds(&flag), "event cheat_flag");
        assert!(matches(&flag, &path(&["event", "cheat_flag"]), &[], Some(blue)));
        assert!(matches(
            &flag,
            &path(&["event", "cheat_flag"]),
            &field("cheat", "restarted_without_retry"),
            None
        ));
        assert!(matches(&flag, &path(&["event"]), &field("boot", "2"), None));
        assert!(!matches(&flag, &path(&["event", "cheat_flag"]), &[], Some([3; 32])));
        assert!(!matches(&flag, &path(&["event", "died"]), &[], None));
        assert!(!matches(
            &flag,
            &path(&["event", "cheat_flag"]),
            &field("cheat", "too_fast"),
            None
        ));

        let retry = body(&LogBody::Retry { call: 1 });
        assert!(matches(&retry, &path(&["retry"]), &field("call", "1"), None));
        assert_eq!(kinds(&body(&LogBody::Takeover)), "takeover");
    }
}
