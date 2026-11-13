//! A game in a Run: the core, read through the game interface (its Game
//! Profile, or the fake in tests), with the Run engine looking at it after
//! every frame. It is itself a core adapter, so the emulator thread runs it
//! like any other core. Its write and patch methods are there for the
//! interface; in a Run only the engine changes the game, at frame boundaries.
//! In a Run with other Players it also builds this Player's input stream,
//! which their Shadow Games replay.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use pactlocke_emu::{Checkpoint, CoreAdapter, CoreError, Frame, Keys};
use pactlocke_game::Change;

use crate::stream::{FrameHash, HASH_EVERY, InputChunk, PlayerKey, StreamOut};
use crate::{PlayedGame, RunEngine, RunEvent, RunView, boot_core};

/// What the side panel asks of the Run, carried out at the next frame boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunCommand {
    /// Must nickname: name the Pokémon the side panel asked about.
    Nickname { pid: u32, name: String },
    /// Another Player's Shadow Game here no longer matches their game.
    OutOfSync { player: String, frame: u64 },
    /// Another Player's Run Checkpoint, the `call`th in the Run log: this
    /// game is captured with it.
    JoinCheckpoint { call: u64 },
    /// A Retry.
    Retry { to: RetryTo },
    /// Links: this Pokémon dies with its Link (`why` says how it broke).
    LinkDeath { pid: u32, why: String },
    /// Links: this source's Link broke elsewhere, so it is closed here too.
    CloseSource { source: crate::SourceRef, why: String },
    /// The Player skips this Gift, breaking its Link for everyone.
    Skip { source: crate::SourceRef },
    /// Every Player is Ready at this Sync Gate: let it go. With
    /// `checkpoint`, this Player's Ready completed it, so this game calls
    /// the gate's Run Checkpoint.
    OpenGate { gate: String, checkpoint: bool },
    /// The Player plays at this speed now (1 to 4).
    Speed(u8),
    /// Another Player plays at this speed now, as the Run log says.
    OthersSpeed { player: String, speed: u8 },
    /// The other Players dropped this Player from the Run.
    DroppedOut,
}

/// Where a Retry goes back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryTo {
    /// Alone: the latest Run Checkpoint, spending one of this Run's Retries.
    Latest,
    /// With other Players, as the Run log says: this Player's part of the
    /// Run Checkpoint everyone voted for, by its hash.
    Checkpoint(String),
}

/// How often the game's save memory is checked for an in-game save.
const SAVE_CHECK_FRAMES: u64 = 60;

pub struct RunningGame<G: PlayedGame> {
    game: G,
    engine: RunEngine,
    view: Arc<Mutex<RunView>>,
    published_version: u64,
    commands: (Sender<RunCommand>, Receiver<RunCommand>),
    /// In a Run with other Players: the Run state to put in the shared log.
    shared: Option<Arc<Mutex<Vec<RunEvent>>>>,
    /// In a Run with other Players: this Player's input stream, and where
    /// its chunks collect for the app to send on.
    stream: Option<(StreamOut, Arc<Mutex<Vec<InputChunk>>>)>,
    /// How many of the engine's stamped changes are in the stream.
    streamed: usize,
    /// In a Run with other Players on an original game: the Partner
    /// Avatars, and where the app puts them as their Shadows show them.
    avatars: Option<(crate::Painter, Arc<Mutex<Vec<crate::PartnerAvatar>>>)>,
}

impl<G: PlayedGame> RunningGame<G> {
    /// `game` is freshly booted: its ROM loaded and, on a resume, the Run's
    /// save memory loaded too. Its first checkpoint is taken now.
    pub fn new(game: G, engine: RunEngine) -> Self {
        let view = Arc::new(Mutex::new(engine.view().clone()));
        let mut running = Self {
            published_version: engine.view_version(),
            game,
            engine,
            view,
            commands: channel(),
            shared: None,
            stream: None,
            streamed: 0,
            avatars: None,
        };
        running.checkpoint_if_wanted();
        running
    }

    /// Boots `game`, whose core has the ROM loaded and nothing run, the way
    /// the Run engine planned: from power-on, the last in-game save, the
    /// suspend slot, or rebuilt after a crash.
    pub fn boot(mut game: G, mut engine: RunEngine, now_ms: i64) -> Result<Self, CoreError> {
        let booted = boot_core(game.core_mut(), engine.boot_plan(), now_ms)?;
        engine.booted(booted);
        Ok(Self::new(game, engine))
    }

    /// Suspend and quit: the Run's suspend slot holds exactly where the game is.
    pub fn suspend(mut self) {
        self.check_save_memory();
        let checkpoint = self.core().checkpoint();
        self.engine.suspend(checkpoint);
    }

    fn checkpoint_if_wanted(&mut self) {
        if self.engine.checkpoint_wanted() {
            let checkpoint = self.core().checkpoint();
            let run_checkpoint = self.engine.take_checkpoint(checkpoint);
            if run_checkpoint && let Some((out, outbox)) = self.stream.as_mut() {
                // Its changes at this boundary go first, so the mark comes after them.
                let stamped = self.engine.stamped()[self.streamed..].to_vec();
                self.streamed = self.engine.stamped().len();
                if let Some(chunk) = out.stamp(&stamped) {
                    outbox.lock().unwrap_or_else(|e| e.into_inner()).push(chunk);
                }
                out.mark_checkpoint(self.engine.frames_run());
            }
        }
    }

    /// A Retry: the game goes back to this Player's Run Checkpoint with
    /// hash `to` (or, alone, to the latest one, spending a Retry), as a new
    /// boot. Everything recorded after it is void.
    /// Nothing is recorded unless the game got back there.
    pub fn retry(&mut self, to: &RetryTo) -> Result<(), String> {
        let hash = match to {
            RetryTo::Latest if self.engine.retries_left() == 0 => return Err("no Retries are left".to_owned()),
            RetryTo::Latest => self
                .engine
                .latest_run_checkpoint()
                .ok_or("there is no Run Checkpoint to go back to yet")?
                .to_owned(),
            RetryTo::Checkpoint(hash) => hash.clone(),
        };
        let (seq, checkpoint) = self
            .engine
            .find_run_checkpoint(&hash)
            .ok_or("that Run Checkpoint is no longer kept")?;
        let stored = self
            .engine
            .stored()
            .map_err(|e| format!("the Run file can't be read: {e}"))?;
        let from = (checkpoint.boot, checkpoint.frame);
        // Patches made since the checkpoint come off; the checkpoint's own go back on.
        let patched: Vec<(u32, Vec<u8>)> = self.engine.patches().iter().map(|(a, b)| (*a, b.clone())).collect();
        for (addr, bytes) in &patched {
            self.game.unpatch(*addr, bytes.len());
        }
        let plan = crate::BootPlan::Retry(checkpoint.clone());
        let booted = match boot_core(self.game.core_mut(), &plan, 0) {
            Ok(booted) => booted,
            Err(e) => {
                for (addr, bytes) in &patched {
                    self.game.core_mut().patch_rom(*addr, bytes);
                }
                return Err(format!("the checkpoint couldn't be restored: {e}"));
            }
        };
        let mut engine = self.engine.retry(stored, seq, checkpoint);
        engine.booted(booted);
        self.engine = engine;
        self.streamed = 0;
        if let Some((out, _)) = self.stream.as_mut() {
            *out = StreamOut::after_retry(out.player(), self.engine.boot(), from);
        }
        self.published_version = self.engine.view_version().wrapping_sub(1);
        self.checkpoint_if_wanted();
        Ok(())
    }

    /// Where the side panel sends what it asks of the Run.
    pub fn commands(&self) -> Sender<RunCommand> {
        self.commands.0.clone()
    }

    /// Carries out what the side panel asked, at this frame boundary.
    fn take_commands(&mut self) {
        while let Ok(command) = self.commands.1.try_recv() {
            match command {
                RunCommand::Nickname { pid, name } => self.engine.name_pokemon(pid, &name, &mut self.game),
                RunCommand::OutOfSync { player, frame } => self.engine.out_of_sync(player, frame),
                RunCommand::JoinCheckpoint { call } => self.engine.join_checkpoint(call),
                RunCommand::LinkDeath { pid, why } => self.engine.link_death(pid, why, &mut self.game),
                RunCommand::CloseSource { source, why } => self.engine.close_source(source, why),
                RunCommand::Skip { source } => self.engine.skip(source),
                RunCommand::OpenGate { gate, checkpoint } => self.engine.open_gate(&gate, checkpoint, &mut self.game),
                RunCommand::Speed(speed) => self.engine.speed(speed),
                RunCommand::OthersSpeed { player, speed } => self.engine.others_speed(player, speed),
                RunCommand::DroppedOut => self.engine.dropped_out(),
                RunCommand::Retry { to } => {
                    if let Err(why) = self.retry(&to) {
                        self.engine.retry_failed(why);
                    }
                }
            }
        }
    }

    /// In a Run with other Players: where the Run state for the shared log
    /// collects, from the emulator thread, for the app to send on.
    pub fn share(&mut self) -> Arc<Mutex<Vec<RunEvent>>> {
        self.shared.get_or_insert_with(Arc::default).clone()
    }

    /// In a Run with other Players: this Player's input stream, as `player`,
    /// from this boot's first frame. Its chunks collect here, oldest first,
    /// for the app to send to every other Player.
    pub fn stream_as(&mut self, player: PlayerKey) -> Arc<Mutex<Vec<InputChunk>>> {
        let outbox: Arc<Mutex<Vec<InputChunk>>> = Arc::default();
        let mut out = StreamOut::new(player, self.engine.boot());
        let closed = out.stamp(self.engine.stamped());
        debug_assert!(closed.is_none(), "nothing to close before the first frame");
        self.streamed = self.engine.stamped().len();
        self.stream = Some((out, outbox.clone()));
        outbox
    }

    /// Draws every other Player's Partner Avatar walking around in this
    /// game, with every Follower. The app keeps them here up to date from
    /// their Shadow Games.
    pub fn draw_partner_avatars(&mut self) -> Arc<Mutex<Vec<crate::PartnerAvatar>>> {
        let partners: Arc<Mutex<Vec<crate::PartnerAvatar>>> = Arc::default();
        self.avatars = Some((crate::Painter::new(), partners.clone()));
        partners
    }

    /// A change made outside the rules, such as a test's or a developer
    /// tool's setup write, stamped at this frame boundary like the Run's
    /// own, so Shadow Games see it.
    pub fn apply_stamped(&mut self, change: Change) {
        self.engine.apply(vec![change], &mut self.game);
    }

    /// The Run's side-panel view, updated from the emulator thread.
    pub fn view(&self) -> Arc<Mutex<RunView>> {
        self.view.clone()
    }

    pub fn engine(&self) -> &RunEngine {
        &self.engine
    }

    /// Takes a Run Checkpoint now, at this frame boundary.
    pub fn checkpoint(&mut self) {
        self.engine.want_checkpoint();
        self.checkpoint_if_wanted();
    }

    /// Writes the input stream so far to the Run file.
    pub fn flush(&mut self) {
        self.engine.flush();
    }

    pub fn game_mut(&mut self) -> &mut G {
        &mut self.game
    }

    fn core(&mut self) -> &mut G::Core {
        self.game.core_mut()
    }

    fn check_save_memory(&mut self) {
        let save = self.game.core_mut().save_memory();
        self.engine.save_memory_seen(&save);
    }

    /// The engine's changes stamped since the stream last took them.
    fn newly_stamped(&mut self) -> &[crate::Stamped] {
        let all = self.engine.stamped();
        let new = &all[self.streamed.min(all.len())..];
        self.streamed = all.len();
        new
    }
}

impl<G: PlayedGame> Drop for RunningGame<G> {
    fn drop(&mut self) {
        self.check_save_memory();
        self.engine.flush();
    }
}

impl<G: PlayedGame> CoreAdapter for RunningGame<G> {
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), CoreError> {
        self.core().load_rom(rom)
    }

    fn run_frame(&mut self, keys: Keys) {
        self.take_commands();
        let keys = self.engine.keys_for_frame(keys, &mut self.game);
        if self.stream.is_some() {
            // Everything stamped at this boundary, then its hash if one is due.
            let boundary = self.engine.frames_run();
            let stamped = self.newly_stamped().to_vec();
            let hash = boundary
                .is_multiple_of(HASH_EVERY)
                .then(|| FrameHash::of(self.game.core_mut(), boundary));
            let (out, outbox) = self.stream.as_mut().expect("streaming");
            if let Some(chunk) = out.stamp(&stamped) {
                outbox.lock().unwrap_or_else(|e| e.into_inner()).push(chunk);
            }
            if let Some(hash) = hash {
                out.hash(hash);
            }
        }
        self.core().run_frame(keys);
        self.engine.frame(keys, &mut self.game);
        if self.stream.is_some() {
            let stamped = self.newly_stamped().to_vec();
            let (out, outbox) = self.stream.as_mut().expect("streaming");
            if let Some(chunk) = out.frame(keys, &stamped) {
                outbox.lock().unwrap_or_else(|e| e.into_inner()).push(chunk);
            }
        }
        let events = self.engine.take_shared();
        if let Some(shared) = &self.shared
            && !events.is_empty()
        {
            shared.lock().unwrap_or_else(|e| e.into_inner()).extend(events);
        }
        if let Some((painter, partners)) = self.avatars.as_mut() {
            let avatars = partners.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let now = self.engine.now_ms();
            let writes = painter.paint(&mut self.game, &avatars, now);
            self.game.core_mut().overlay(&writes);
        }
        if self.engine.frames_run().is_multiple_of(SAVE_CHECK_FRAMES) {
            self.check_save_memory();
        }
        self.checkpoint_if_wanted();
        if self.engine.view_version() != self.published_version {
            self.published_version = self.engine.view_version();
            *self.view.lock().unwrap_or_else(|e| e.into_inner()) = self.engine.view().clone();
        }
    }

    fn read(&mut self, addr: u32, buf: &mut [u8]) {
        self.core().read(addr, buf);
    }

    fn write(&mut self, addr: u32, data: &[u8]) {
        self.core().write(addr, data);
    }

    fn write16(&mut self, addr: u32, value: u16) {
        self.core().write16(addr, value);
    }

    fn write32(&mut self, addr: u32, value: u32) {
        self.core().write32(addr, value);
    }

    fn patch_rom(&mut self, addr: u32, data: &[u8]) {
        self.core().patch_rom(addr, data);
    }

    fn checkpoint(&mut self) -> Checkpoint {
        self.core().checkpoint()
    }

    fn restore(&mut self, checkpoint: &Checkpoint) -> Result<(), CoreError> {
        self.core().restore(checkpoint)
    }

    fn save_memory(&mut self) -> Vec<u8> {
        self.core().save_memory()
    }

    fn load_save_memory(&mut self, save: &[u8]) -> Result<(), CoreError> {
        self.core().load_save_memory(save)
    }

    fn set_clock(&mut self, epoch_ms: i64) {
        self.core().set_clock(epoch_ms);
    }

    fn frame(&self) -> Frame {
        self.game.core().frame()
    }

    fn audio_sample_rate(&self) -> u32 {
        self.game.core().audio_sample_rate()
    }

    fn drain_audio(&mut self, out: &mut Vec<i16>) {
        self.core().drain_audio(out);
    }
}
