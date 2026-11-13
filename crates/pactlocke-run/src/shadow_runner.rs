//! A Shadow Game on a thread of its own, behind the local game: it takes the
//! Player's stream as it arrives and plays it at their pace. While someone
//! Spectates it, it publishes its frames and plays its audio. In battery-saver
//! mode it only runs while watched, catching up from where it stopped.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pactlocke_emu::{AudioSink, FrameSlot};

use crate::stream::InputChunk;
use crate::{PlayedGame, RunClock, Seen, ShadowGame, ShadowState};

/// How long the thread sleeps between slices when there's nothing to do.
const TICK: Duration = Duration::from_millis(8);

/// A Shadow Game's state, for the side panel and Spectate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowStatus {
    pub state: ShadowState,
    /// Well behind where the Player is, fast-forwarding.
    pub catching_up: bool,
    /// Frames run, and the frame after the newest received.
    pub frame: u64,
    pub received: u64,
    /// What it has shown of the Player's Pokémon.
    pub seen: Seen,
}

enum Command {
    Chunk(InputChunk),
    Watch(Box<dyn AudioSink>),
    Unwatch,
    BatterySaver(bool),
    Tiled(bool),
    Stop,
}

/// A Shadow Game running on its own thread. Dropping it stops the thread.
pub struct ShadowRunner {
    commands: Sender<Command>,
    frames: Arc<FrameSlot>,
    status: Arc<Mutex<ShadowStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl ShadowRunner {
    /// Runs `shadow` on a thread, paced by `clock`. `on_diverged` is called
    /// once, from that thread, with the boundary where it stopped matching.
    pub fn spawn<G: PlayedGame + 'static>(
        shadow: ShadowGame<G>,
        clock: Box<dyn RunClock>,
        on_diverged: Box<dyn FnOnce(u64) + Send>,
    ) -> Self {
        let (commands, inbox) = channel();
        let frames = Arc::new(FrameSlot::new());
        let status = Arc::new(Mutex::new(ShadowStatus {
            state: shadow.state(),
            catching_up: false,
            frame: 0,
            received: 0,
            seen: Seen::default(),
        }));
        let thread = {
            let frames = frames.clone();
            let status = status.clone();
            std::thread::Builder::new()
                .name("shadow game".into())
                .spawn(move || run(shadow, clock, on_diverged, &inbox, &frames, &status))
                .expect("a thread for a Shadow Game")
        };
        Self {
            commands,
            frames,
            status,
            thread: Some(thread),
        }
    }

    /// The next chunk of the Player's stream, in order.
    pub fn receive(&self, chunk: InputChunk) {
        let _ = self.commands.send(Command::Chunk(chunk));
    }

    /// Spectate: publish frames, and play the game's audio into `sink`.
    pub fn watch(&self, sink: Box<dyn AudioSink>) {
        let _ = self.commands.send(Command::Watch(sink));
    }

    /// Spectate ends: no more frames or sound.
    pub fn unwatch(&self) {
        let _ = self.commands.send(Command::Unwatch);
    }

    /// The partner view: publish frames (no sound) while shown as a tile.
    pub fn tiled(&self, on: bool) {
        let _ = self.commands.send(Command::Tiled(on));
    }

    /// Battery saver: run only while watched.
    pub fn battery_saver(&self, on: bool) {
        let _ = self.commands.send(Command::BatterySaver(on));
    }

    /// Where its frames are published while it is watched.
    pub fn frames(&self) -> Arc<FrameSlot> {
        self.frames.clone()
    }

    /// Where it stands, as of its last slice.
    pub fn status(&self) -> ShadowStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for ShadowRunner {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run<G: PlayedGame>(
    mut shadow: ShadowGame<G>,
    clock: Box<dyn RunClock>,
    on_diverged: Box<dyn FnOnce(u64) + Send>,
    inbox: &Receiver<Command>,
    frames: &FrameSlot,
    status: &Mutex<ShadowStatus>,
) {
    let mut on_diverged = Some(on_diverged);
    let mut sink: Option<Box<dyn AudioSink>> = None;
    let mut saver = false;
    let mut tiled = false;
    let mut audio = Vec::new();
    let mut last_ms = clock.now_ms();
    loop {
        match inbox.recv_timeout(TICK) {
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(command) => {
                let mut next = Some(command);
                while let Some(command) = next.take() {
                    match command {
                        Command::Chunk(chunk) => shadow.receive(&chunk, clock.now_ms()),
                        Command::Watch(new) => sink = Some(new),
                        Command::Unwatch => sink = None,
                        Command::BatterySaver(on) => saver = on,
                        Command::Tiled(on) => tiled = on,
                        Command::Stop => return,
                    }
                    next = inbox.try_recv().ok();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = clock.now_ms();
        let slice = now - last_ms;
        last_ms = now;
        let watched = sink.is_some();
        if (!saver || watched || tiled) && shadow.advance(now, shadow.frames_for(slice)) > 0 {
            audio.clear();
            shadow.drain_audio(&mut audio);
            if sink.is_some() || tiled {
                frames.publish(shadow.frame(), shadow.video());
            }
            if let Some(sink) = sink.as_mut() {
                // Fast-forwarding, its sound would play many times too fast.
                if !shadow.catching_up(now) {
                    sink.push(&audio, shadow.audio_sample_rate());
                }
            }
        }
        if let ShadowState::Diverged { frame } = shadow.state()
            && let Some(tell) = on_diverged.take()
        {
            tell(frame);
        }
        *status.lock().unwrap_or_else(|e| e.into_inner()) = ShadowStatus {
            state: shadow.state(),
            catching_up: shadow.catching_up(now),
            frame: shadow.frame(),
            received: shadow.received(),
            seen: shadow.seen().clone(),
        };
    }
}
