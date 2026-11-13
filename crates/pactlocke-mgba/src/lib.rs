//! libmgba, pinned to master `662dada`, behind Pactlocke's C shim.
//!
//! The core always uses the HLE BIOS, keeps the save in memory, and runs a
//! fake-epoch real-time clock that reads as UTC on every machine.

mod ffi;

use std::borrow::Cow;
use std::ptr::NonNull;
use std::sync::Once;

use pactlocke_emu::{Checkpoint, CoreAdapter, CoreError, Frame, Keys};

/// Make the process's C runtime use UTC as its local time zone.
///
/// libmgba turns the clock into a date with the C library's local time, so two
/// machines in different zones would see different dates and diverge. Until a
/// small upstream patch lands, the whole process runs its C runtime in UTC.
/// Call this first thing in `main`, before other threads start; `MgbaCore::new`
/// also calls it, once.
pub fn force_utc_time_zone() {
    static ONCE: Once = Once::new();
    // SAFETY: changes the C runtime's TZ setting; callers make this the first
    // thing the process does, and Once keeps it to a single call.
    ONCE.call_once(|| unsafe { ffi::pact_force_utc() });
}

/// The GBA core.
pub struct MgbaCore {
    raw: NonNull<ffi::PactCore>,
}

// SAFETY: a libmgba core has no thread affinity; MgbaCore owns its core
// exclusively and every call needs `&mut self` or reads plain buffers.
unsafe impl Send for MgbaCore {}

impl MgbaCore {
    pub fn new() -> Self {
        force_utc_time_zone();
        // SAFETY: plain constructor; null only if allocation fails.
        let raw = unsafe { ffi::pact_core_new() };
        Self {
            raw: NonNull::new(raw).expect("libmgba core could not be created"),
        }
    }

    fn ptr(&self) -> *mut ffi::PactCore {
        self.raw.as_ptr()
    }
}

impl Default for MgbaCore {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MgbaCore {
    fn drop(&mut self) {
        // SAFETY: we own the core and it is never used again.
        unsafe { ffi::pact_core_free(self.ptr()) }
    }
}

impl CoreAdapter for MgbaCore {
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), CoreError> {
        // SAFETY: the shim copies the bytes before returning.
        if unsafe { ffi::pact_core_load_rom(self.ptr(), rom.as_ptr(), rom.len()) } {
            Ok(())
        } else {
            Err(CoreError::NotARom)
        }
    }

    fn run_frame(&mut self, keys: Keys) {
        // SAFETY: valid core.
        unsafe { ffi::pact_core_run_frame(self.ptr(), u32::from(keys.0)) }
    }

    fn read(&mut self, addr: u32, buf: &mut [u8]) {
        // SAFETY: the shim writes exactly buf.len() bytes.
        unsafe { ffi::pact_core_read(self.ptr(), addr, buf.as_mut_ptr(), buf.len()) }
    }

    fn write(&mut self, addr: u32, data: &[u8]) {
        // SAFETY: the shim reads exactly data.len() bytes.
        unsafe { ffi::pact_core_write(self.ptr(), addr, data.as_ptr(), data.len()) }
    }

    fn write16(&mut self, addr: u32, value: u16) {
        // SAFETY: valid core.
        unsafe { ffi::pact_core_write16(self.ptr(), addr, value) }
    }

    fn write32(&mut self, addr: u32, value: u32) {
        // SAFETY: valid core.
        unsafe { ffi::pact_core_write32(self.ptr(), addr, value) }
    }

    fn patch_rom(&mut self, addr: u32, data: &[u8]) {
        // SAFETY: the shim reads exactly data.len() bytes.
        unsafe { ffi::pact_core_patch_rom(self.ptr(), addr, data.as_ptr(), data.len()) }
    }

    fn overlay(&mut self, writes: &[(u32, Vec<u8>)]) {
        let mut records = Vec::new();
        // Display memory only (palette RAM, VRAM, OAM); the shim checks too.
        let display = |addr: u32, len: usize| addr >= 0x0500_0000 && u64::from(addr) + len as u64 <= 0x0800_0000;
        for (addr, bytes) in writes.iter().filter(|(addr, bytes)| display(*addr, bytes.len())) {
            let len = bytes.len().min(0xFFFE) & !1;
            records.extend_from_slice(&addr.to_le_bytes());
            records.extend_from_slice(&(len as u16).to_le_bytes());
            records.extend_from_slice(&bytes[..len]);
        }
        // SAFETY: the shim copies the bytes before returning. Out of memory
        // leaves the last overlay in place, which only affects the picture.
        unsafe {
            ffi::pact_core_set_overlay(self.ptr(), records.as_ptr(), records.len());
        }
    }

    fn checkpoint(&mut self) -> Checkpoint {
        // SAFETY: the state buffer is exactly the size the core asked for, and
        // the save memory copy is freed by the shim's allocator.
        unsafe {
            let mut state = vec![0u8; ffi::pact_core_state_size(self.ptr())];
            let saved = ffi::pact_core_save_state(self.ptr(), state.as_mut_ptr());
            assert!(saved, "libmgba failed to write a savestate");

            Checkpoint {
                save_memory: self.save_memory(),
                state,
            }
        }
    }

    fn restore(&mut self, checkpoint: &Checkpoint) -> Result<(), CoreError> {
        // SAFETY: the state length is checked against what the core expects
        // before it reads it; the shim copies the save memory.
        unsafe {
            if checkpoint.state.len() != ffi::pact_core_state_size(self.ptr()) {
                return Err(CoreError::BadCheckpoint);
            }
            // Save memory first, then the savestate. A checkpoint from before
            // the game ever saved has none, so any save made since is erased
            // (erased flash and SRAM read 0xFF).
            let save_memory = if checkpoint.save_memory.is_empty() {
                match self.save_memory().len() {
                    0 => None,
                    len => Some(Cow::Owned(vec![0xFF; len])),
                }
            } else {
                Some(Cow::Borrowed(checkpoint.save_memory.as_slice()))
            };
            if let Some(save) = save_memory
                && !ffi::pact_core_restore_save_memory(self.ptr(), save.as_ptr(), save.len())
            {
                return Err(CoreError::BadCheckpoint);
            }
            if !ffi::pact_core_load_state(self.ptr(), checkpoint.state.as_ptr()) {
                return Err(CoreError::BadCheckpoint);
            }
        }
        Ok(())
    }

    fn save_memory(&mut self) -> Vec<u8> {
        // SAFETY: the shim hands over a malloc'd copy of `len` bytes (or null),
        // which is copied and then released with the shim's allocator.
        unsafe {
            let mut sram: *mut u8 = std::ptr::null_mut();
            let len = ffi::pact_core_save_memory(self.ptr(), &mut sram);
            if sram.is_null() {
                return Vec::new();
            }
            let copy = std::slice::from_raw_parts(sram, len).to_vec();
            ffi::pact_free(sram.cast());
            copy
        }
    }

    fn load_save_memory(&mut self, save: &[u8]) -> Result<(), CoreError> {
        // SAFETY: the shim copies the bytes before returning.
        if unsafe { ffi::pact_core_restore_save_memory(self.ptr(), save.as_ptr(), save.len()) } {
            Ok(())
        } else {
            Err(CoreError::BadSave)
        }
    }

    fn set_clock(&mut self, epoch_ms: i64) {
        // SAFETY: valid core.
        unsafe { ffi::pact_core_set_clock(self.ptr(), epoch_ms) }
    }

    fn frame(&self) -> Frame {
        let (mut width, mut height) = (0u32, 0u32);
        // SAFETY: the video buffer holds width*height 32-bit pixels for as long
        // as the core lives, and nothing writes to it during this `&self` call.
        let pixels = unsafe {
            ffi::pact_core_video_size(self.ptr(), &mut width, &mut height);
            std::slice::from_raw_parts(ffi::pact_core_video_buffer(self.ptr()), (width * height) as usize)
        };
        let mut rgba = Vec::with_capacity(pixels.len() * 4);
        for &px in pixels {
            // Bytes R, G, B, X in memory, so the low byte is red on any endianness.
            rgba.extend_from_slice(&[px as u8, (px >> 8) as u8, (px >> 16) as u8, 255]);
        }
        Frame { width, height, rgba }
    }

    fn audio_sample_rate(&self) -> u32 {
        // SAFETY: valid core.
        unsafe { ffi::pact_core_audio_sample_rate(self.ptr()) }
    }

    fn drain_audio(&mut self, out: &mut Vec<i16>) {
        const CHUNK_FRAMES: usize = 1024;
        loop {
            let start = out.len();
            out.resize(start + CHUNK_FRAMES * 2, 0);
            // SAFETY: room for CHUNK_FRAMES stereo frames was just reserved.
            let got = unsafe { ffi::pact_core_audio_read(self.ptr(), out[start..].as_mut_ptr(), CHUNK_FRAMES) };
            out.truncate(start + got * 2);
            if got < CHUNK_FRAMES {
                break;
            }
        }
    }
}
