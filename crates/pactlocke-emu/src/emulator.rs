use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{CoreAdapter, FrameSlot, Keys, Speed};

/// Where the emulator thread measures and waits for time.
pub trait Clock: Send + 'static {
    /// Time since some fixed start.
    fn now(&self) -> Duration;
    /// Block until `now()` reaches `deadline`.
    fn sleep_until(&self, deadline: Duration);
}

/// The real clock.
pub struct SystemClock {
    start: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self { start: Instant::now() }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
    fn sleep_until(&self, deadline: Duration) {
        let now = self.now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
    }
}

/// Receives the game's audio after every frame.
pub trait AudioSink: Send + 'static {
    /// Interleaved stereo samples at `sample_rate`.
    fn push(&mut self, samples: &[i16], sample_rate: u32);
}

pub struct EmulatorOptions {
    clock: Box<dyn Clock>,
    speed: Speed,
    audio: Option<Box<dyn AudioSink>>,
    on_frame: Option<Box<dyn Fn() + Send>>,
}

impl EmulatorOptions {
    pub fn new(clock: impl Clock) -> Self {
        Self {
            clock: Box::new(clock),
            speed: Speed::X1,
            audio: None,
            on_frame: None,
        }
    }

    /// The speed to start at.
    pub fn speed(mut self, speed: Speed) -> Self {
        self.speed = speed;
        self
    }

    /// Where the game's audio goes. Without one it is dropped.
    pub fn audio(mut self, sink: impl AudioSink) -> Self {
        self.audio = Some(Box::new(sink));
        self
    }

    /// Called after each frame is published, e.g. to ask the UI to repaint.
    pub fn on_frame(mut self, callback: impl Fn() + Send + 'static) -> Self {
        self.on_frame = Some(Box::new(callback));
        self
    }
}

/// How long one frame lasts at this speed: a GBA frame is 280,896 cycles of a
/// 16,777,216 Hz clock.
fn frame_duration(speed: Speed) -> Duration {
    Duration::from_nanos(280_896 * 1_000_000_000 / (16_777_216 * u64::from(speed.multiplier())))
}

/// If the thread falls further behind than this (a stall, a debugger), it stops
/// trying to catch up instead of running a burst of frames.
const MAX_LAG: Duration = Duration::from_millis(250);

struct Shared {
    stop: AtomicBool,
    keys: AtomicU16,
    speed: AtomicU8,
}

/// The emulator thread for one game. It runs one frame at a time with the keys
/// currently held, publishes each frame to a single slot, hands the audio to the
/// sink and waits until the next frame is due at the current speed.
pub struct Emulator<C> {
    frames: Arc<FrameSlot>,
    shared: Arc<Shared>,
    /// Taken by `stop`; `Drop` joins it otherwise.
    thread: Option<JoinHandle<C>>,
}

impl<C: CoreAdapter + 'static> Emulator<C> {
    pub fn spawn(core: C, options: EmulatorOptions) -> Self {
        let frames = Arc::new(FrameSlot::new());
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            keys: AtomicU16::new(0),
            speed: AtomicU8::new(options.speed.multiplier() as u8),
        });
        let thread = {
            let frames = frames.clone();
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("emulator".into())
                .spawn(move || run(core, options, &frames, &shared))
                .expect("spawn emulator thread")
        };
        Self {
            frames,
            shared,
            thread: Some(thread),
        }
    }

    /// The slot the newest frame is published to.
    pub fn frames(&self) -> Arc<FrameSlot> {
        self.frames.clone()
    }

    /// Hold these keys from the next frame on.
    pub fn set_keys(&self, keys: Keys) {
        self.shared.keys.store(keys.0, Ordering::Release);
    }

    /// Run at this speed from the next frame on.
    pub fn set_speed(&self, speed: Speed) {
        self.shared.speed.store(speed.multiplier() as u8, Ordering::Release);
    }

    pub fn speed(&self) -> Speed {
        Speed::from_multiplier(self.shared.speed.load(Ordering::Acquire))
    }

    /// Stop the thread at the next frame boundary and hand the core back.
    /// If the emulator thread panicked, the panic carries on here.
    pub fn stop(mut self) -> C {
        self.shared.stop.store(true, Ordering::Release);
        let thread = self.thread.take().expect("only stop takes the thread");
        match thread.join() {
            Ok(core) => core,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

impl<C> Drop for Emulator<C> {
    /// Dropping the emulator stops its thread too, and drops the core.
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.shared.stop.store(true, Ordering::Release);
            // A panic on the emulator thread has already been reported; don't
            // panic again while dropping.
            let _ = thread.join();
        }
    }
}

fn run<C: CoreAdapter>(mut core: C, options: EmulatorOptions, frames: &FrameSlot, shared: &Shared) -> C {
    let EmulatorOptions {
        clock,
        mut audio,
        on_frame,
        ..
    } = options;
    let mut samples = Vec::new();
    let mut seq = 0u64;
    let mut deadline = clock.now();
    while !shared.stop.load(Ordering::Acquire) {
        core.run_frame(Keys(shared.keys.load(Ordering::Acquire)));
        seq += 1;
        frames.publish(seq, core.frame());

        samples.clear();
        core.drain_audio(&mut samples);
        if let Some(sink) = audio.as_mut() {
            sink.push(&samples, core.audio_sample_rate());
        }
        if let Some(callback) = &on_frame {
            callback();
        }

        let speed = Speed::from_multiplier(shared.speed.load(Ordering::Acquire));
        deadline += frame_duration(speed);
        let now = clock.now();
        if now > deadline + MAX_LAG {
            deadline = now;
        }
        clock.sleep_until(deadline);
    }
    core
}
