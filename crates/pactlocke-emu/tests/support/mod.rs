#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pactlocke_emu::{Checkpoint, Clock, CoreAdapter, CoreError, Frame, Keys};

/// A fake core: each frame it draws the frame number into the first pixel and
/// produces 4 stereo samples, and it remembers the keys it was given.
pub struct FakeCore {
    pub frames_run: Arc<AtomicU32>,
    pub keys_seen: Vec<Keys>,
    audio: Vec<i16>,
}

impl FakeCore {
    pub fn new() -> Self {
        Self {
            frames_run: Arc::new(AtomicU32::new(0)),
            keys_seen: Vec::new(),
            audio: Vec::new(),
        }
    }

    pub fn probe(&self) -> Probe {
        Probe {
            frames_run: self.frames_run.clone(),
        }
    }
}

impl CoreAdapter for FakeCore {
    fn load_rom(&mut self, _rom: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
    fn run_frame(&mut self, keys: Keys) {
        self.keys_seen.push(keys);
        let n = self.frames_run.fetch_add(1, Ordering::SeqCst) + 1;
        self.audio.extend_from_slice(&[n as i16; 8]);
    }
    fn read(&mut self, _addr: u32, buf: &mut [u8]) {
        buf.fill(0);
    }
    fn write(&mut self, _addr: u32, _data: &[u8]) {}
    fn write16(&mut self, _addr: u32, _value: u16) {}
    fn write32(&mut self, _addr: u32, _value: u32) {}
    fn patch_rom(&mut self, _addr: u32, _data: &[u8]) {}
    fn checkpoint(&mut self) -> Checkpoint {
        Checkpoint {
            save_memory: Vec::new(),
            state: Vec::new(),
        }
    }
    fn restore(&mut self, _checkpoint: &Checkpoint) -> Result<(), CoreError> {
        Ok(())
    }
    fn set_clock(&mut self, _epoch_ms: i64) {}
    fn frame(&self) -> Frame {
        let n = self.frames_run.load(Ordering::SeqCst);
        let mut rgba = vec![0u8; 4 * 2 * 2];
        rgba[..4].copy_from_slice(&n.to_le_bytes());
        Frame {
            width: 2,
            height: 2,
            rgba,
        }
    }
    fn audio_sample_rate(&self) -> u32 {
        32768
    }
    fn drain_audio(&mut self, out: &mut Vec<i16>) {
        out.append(&mut self.audio);
    }
}

pub struct Probe {
    frames_run: Arc<AtomicU32>,
}

impl Probe {
    pub fn wait_for_frames(&self, n: u32) {
        let start = Instant::now();
        while self.frames_run.load(Ordering::SeqCst) < n {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "emulator thread never ran {n} frames"
            );
            std::thread::yield_now();
        }
    }
}

/// A clock where sleeping takes no real time: it jumps virtual time forward and
/// records every deadline the emulator thread waited for.
#[derive(Clone, Default)]
pub struct FakeClock {
    inner: Arc<Mutex<(Duration, Vec<Duration>)>>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn deadlines(&self) -> Vec<Duration> {
        self.inner.lock().unwrap().1.clone()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.inner.lock().unwrap().0
    }
    fn sleep_until(&self, deadline: Duration) {
        let mut g = self.inner.lock().unwrap();
        g.1.push(deadline);
        if deadline > g.0 {
            g.0 = deadline;
        }
    }
}

pub fn first_pixel_as_u32(frame: &Frame) -> u32 {
    u32::from_le_bytes(frame.rgba[..4].try_into().unwrap())
}

impl Probe {
    pub fn frames_run(&self) -> u32 {
        self.frames_run.load(Ordering::SeqCst)
    }
}

/// One push of audio: the samples and their rate.
pub type Push = (Vec<i16>, u32);

/// Records every push of audio.
#[derive(Clone, Default)]
pub struct RecordingSink {
    pushes: Arc<Mutex<Vec<Push>>>,
}

impl RecordingSink {
    pub fn pushes(&self) -> Vec<Push> {
        self.pushes.lock().unwrap().clone()
    }
}

impl pactlocke_emu::AudioSink for RecordingSink {
    fn push(&mut self, samples: &[i16], sample_rate: u32) {
        self.pushes.lock().unwrap().push((samples.to_vec(), sample_rate));
    }
}
