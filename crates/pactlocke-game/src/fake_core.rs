//! A deterministic fake emulator core for tests: its state follows from the
//! keys it is given and the writes and patches made to it, exactly as a real
//! core's does, so a copy driven by the same input stream matches it by hash.
//! It holds no real game; [`crate::FakeGame`] carries one, so a fake Player's
//! game can be replayed as a Shadow Game.

use std::collections::BTreeMap;

use pactlocke_emu::{Checkpoint, CoreAdapter, CoreError, Frame, Keys};

/// Where the fake shows its state in work RAM, so a RAM hash covers it:
/// frames run, then the key-driven state, then a digest of every write.
const STATE_AT: u32 = 0x0200_0000;
const STATE_LEN: usize = 24;
/// Where a ROM patch lands; patches are kept apart from RAM, as on a GBA.
const ROM_START: u32 = 0x0800_0000;

const SAMPLE_RATE: u32 = 32768;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeCore {
    frames: u64,
    /// Mixed from every frame's keys.
    state: u64,
    /// Mixed from every RAM write, in order.
    writes: u64,
    ram: BTreeMap<u32, u8>,
    rom_patches: BTreeMap<u32, u8>,
    clock_ms: i64,
    save: Vec<u8>,
    audio: Vec<i16>,
}

/// SplitMix64's finaliser: a good spread for a fold.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

impl FakeCore {
    /// Frames run since power-on.
    pub fn frames_run(&self) -> u64 {
        self.frames
    }

    /// Sets the save memory, as an in-game save would.
    pub fn set_save(&mut self, save: Vec<u8>) {
        self.save = save;
    }

    fn state_bytes(&self) -> [u8; STATE_LEN] {
        let mut out = [0; STATE_LEN];
        out[..8].copy_from_slice(&self.frames.to_le_bytes());
        out[8..16].copy_from_slice(&self.state.to_le_bytes());
        out[16..].copy_from_slice(&self.writes.to_le_bytes());
        out
    }

    fn write_byte(&mut self, addr: u32, byte: u8) {
        self.writes = mix(self.writes ^ (u64::from(addr) << 8) ^ u64::from(byte));
        self.ram.insert(addr, byte);
    }
}

impl CoreAdapter for FakeCore {
    fn load_rom(&mut self, _rom: &[u8]) -> Result<(), CoreError> {
        *self = Self {
            save: std::mem::take(&mut self.save),
            ..Self::default()
        };
        Ok(())
    }

    /// The game's code runs over its ROM, so the patches active change how
    /// every frame goes, as a patched routine would.
    fn run_frame(&mut self, keys: Keys) {
        self.frames += 1;
        let patched = self.rom_patches.iter().fold(0, |digest, (&addr, &byte)| {
            mix(digest ^ (u64::from(addr) << 8) ^ u64::from(byte))
        });
        self.state = mix(self.state ^ u64::from(keys.0) ^ (self.frames << 16) ^ self.clock_ms as u64 ^ patched);
        let sample = self.state as i16;
        self.audio.extend_from_slice(&[sample, sample]);
    }

    fn read(&mut self, addr: u32, buf: &mut [u8]) {
        buf.fill(0);
        let end = addr.saturating_add(buf.len() as u32);
        let state = self.state_bytes();
        for (i, out) in buf.iter_mut().enumerate() {
            let at = addr + i as u32;
            if (STATE_AT..STATE_AT + STATE_LEN as u32).contains(&at) {
                *out = state[(at - STATE_AT) as usize];
            }
        }
        let map = if addr >= ROM_START {
            &self.rom_patches
        } else {
            &self.ram
        };
        for (&at, &byte) in map.range(addr..end) {
            if !(STATE_AT..STATE_AT + STATE_LEN as u32).contains(&at) {
                buf[(at - addr) as usize] = byte;
            }
        }
    }

    fn write(&mut self, addr: u32, data: &[u8]) {
        for (i, &byte) in data.iter().enumerate() {
            self.write_byte(addr + i as u32, byte);
        }
    }

    fn write16(&mut self, addr: u32, value: u16) {
        self.write(addr, &value.to_le_bytes());
    }

    fn write32(&mut self, addr: u32, value: u32) {
        self.write(addr, &value.to_le_bytes());
    }

    fn patch_rom(&mut self, addr: u32, data: &[u8]) {
        for (i, &byte) in data.iter().enumerate() {
            self.rom_patches.insert(addr + i as u32, byte);
        }
    }

    /// RAM and the clock; like a real savestate, not the ROM patches.
    fn checkpoint(&mut self) -> Checkpoint {
        let mut state = Vec::with_capacity(32 + self.ram.len() * 5);
        state.extend_from_slice(&self.frames.to_le_bytes());
        state.extend_from_slice(&self.state.to_le_bytes());
        state.extend_from_slice(&self.writes.to_le_bytes());
        state.extend_from_slice(&self.clock_ms.to_le_bytes());
        for (&addr, &byte) in &self.ram {
            state.extend_from_slice(&addr.to_le_bytes());
            state.push(byte);
        }
        Checkpoint {
            save_memory: self.save.clone(),
            state,
        }
    }

    fn restore(&mut self, checkpoint: &Checkpoint) -> Result<(), CoreError> {
        let state = &checkpoint.state;
        if state.len() < 32 || !(state.len() - 32).is_multiple_of(5) {
            return Err(CoreError::BadCheckpoint);
        }
        let u64_at = |at: usize| u64::from_le_bytes(state[at..at + 8].try_into().expect("8 bytes"));
        self.frames = u64_at(0);
        self.state = u64_at(8);
        self.writes = u64_at(16);
        self.clock_ms = u64_at(24) as i64;
        self.ram = state[32..]
            .as_chunks::<5>()
            .0
            .iter()
            .map(|c| (u32::from_le_bytes([c[0], c[1], c[2], c[3]]), c[4]))
            .collect();
        self.save = checkpoint.save_memory.clone();
        self.audio.clear();
        Ok(())
    }

    fn save_memory(&mut self) -> Vec<u8> {
        self.save.clone()
    }

    fn load_save_memory(&mut self, save: &[u8]) -> Result<(), CoreError> {
        self.save = save.to_vec();
        Ok(())
    }

    fn set_clock(&mut self, epoch_ms: i64) {
        self.clock_ms = epoch_ms;
    }

    /// 2x2 pixels showing the state, so two frames of different states differ.
    fn frame(&self) -> Frame {
        let mut rgba = Vec::with_capacity(16);
        for byte in self.state.to_le_bytes().chunks(2) {
            rgba.extend_from_slice(&[byte[0], byte[1], self.frames as u8, 255]);
        }
        Frame {
            width: 2,
            height: 2,
            rgba,
        }
    }

    fn audio_sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn drain_audio(&mut self, out: &mut Vec<i16>) {
        out.append(&mut self.audio);
    }
}
