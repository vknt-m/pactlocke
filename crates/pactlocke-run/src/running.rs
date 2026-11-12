//! A real game in a Run: the core, read through its Game Profile, with the
//! Run engine looking at it after every frame. It is itself a core adapter,
//! so the emulator thread runs it like any other core. Its write and patch
//! methods are there for the interface; in a Run only the engine changes the
//! game, at frame boundaries.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use pactlocke_emu::{Checkpoint, CoreAdapter, CoreError, Frame, Keys};
use pactlocke_game::ProfileGame;

use crate::{RunEngine, RunEvent, RunView, boot_core};

/// What the side panel asks of the Run, carried out at the next frame boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunCommand {
    /// Must nickname: name the Pokémon the side panel asked about.
    Nickname { pid: u32, name: String },
}

/// How often the game's save memory is checked for an in-game save.
const SAVE_CHECK_FRAMES: u64 = 60;

pub struct RunningGame<C: CoreAdapter> {
    game: ProfileGame<C>,
    engine: RunEngine,
    view: Arc<Mutex<RunView>>,
    published_version: u64,
    commands: (Sender<RunCommand>, Receiver<RunCommand>),
    /// In a Run with other Players: the Run state to put in the shared log.
    shared: Option<Arc<Mutex<Vec<RunEvent>>>>,
}

impl<C: CoreAdapter> RunningGame<C> {
    /// `game` is freshly booted: its ROM loaded and, on a resume, the Run's
    /// save memory loaded too. Its first checkpoint is taken now.
    pub fn new(game: ProfileGame<C>, engine: RunEngine) -> Self {
        let view = Arc::new(Mutex::new(engine.view().clone()));
        let mut running = Self {
            published_version: engine.view_version(),
            game,
            engine,
            view,
            commands: channel(),
            shared: None,
        };
        running.checkpoint_if_wanted();
        running
    }

    /// Boots `game`, whose core has the ROM loaded and nothing run, the way
    /// the Run engine planned: from power-on, the last in-game save, the
    /// suspend slot, or rebuilt after a crash.
    pub fn boot(mut game: ProfileGame<C>, mut engine: RunEngine, now_ms: i64) -> Result<Self, CoreError> {
        let booted = boot_core(game.memory_mut(), engine.boot_plan(), now_ms)?;
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
            self.engine.take_checkpoint(checkpoint);
        }
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
            }
        }
    }

    /// In a Run with other Players: where the Run state for the shared log
    /// collects, from the emulator thread, for the app to send on.
    pub fn share(&mut self) -> Arc<Mutex<Vec<RunEvent>>> {
        self.shared.get_or_insert_with(Arc::default).clone()
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

    pub fn game_mut(&mut self) -> &mut ProfileGame<C> {
        &mut self.game
    }

    fn core(&mut self) -> &mut C {
        self.game.memory_mut()
    }

    fn check_save_memory(&mut self) {
        let save = self.game.memory_mut().save_memory();
        self.engine.save_memory_seen(&save);
    }
}

impl<C: CoreAdapter> Drop for RunningGame<C> {
    fn drop(&mut self) {
        self.check_save_memory();
        self.engine.flush();
    }
}

impl<C: CoreAdapter> CoreAdapter for RunningGame<C> {
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), CoreError> {
        self.core().load_rom(rom)
    }

    fn run_frame(&mut self, keys: Keys) {
        self.take_commands();
        let keys = self.engine.keys_for_frame(keys, &mut self.game);
        self.core().run_frame(keys);
        self.engine.frame(keys, &mut self.game);
        let events = self.engine.take_shared();
        if let Some(shared) = &self.shared
            && !events.is_empty()
        {
            shared.lock().unwrap_or_else(|e| e.into_inner()).extend(events);
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
        self.game.memory().frame()
    }

    fn audio_sample_rate(&self) -> u32 {
        self.game.memory().audio_sample_rate()
    }

    fn drain_audio(&mut self, out: &mut Vec<i16>) {
        self.core().drain_audio(out);
    }
}
