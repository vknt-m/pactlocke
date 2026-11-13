//! Hand-written bindings to `shim/pact_shim.h`. Keep the two in step.

#[repr(C)]
pub struct PactCore {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub fn pact_force_utc();

    pub fn pact_core_new() -> *mut PactCore;
    pub fn pact_core_free(pc: *mut PactCore);

    pub fn pact_core_load_rom(pc: *mut PactCore, data: *const u8, len: usize) -> bool;
    pub fn pact_core_set_clock(pc: *mut PactCore, epoch_ms: i64);

    pub fn pact_core_run_frame(pc: *mut PactCore, keys: u32);

    pub fn pact_core_video_size(pc: *const PactCore, width: *mut u32, height: *mut u32);
    pub fn pact_core_video_buffer(pc: *const PactCore) -> *const u32;

    pub fn pact_core_audio_sample_rate(pc: *const PactCore) -> u32;
    pub fn pact_core_audio_read(pc: *mut PactCore, out: *mut i16, max_frames: usize) -> usize;

    pub fn pact_core_read(pc: *mut PactCore, addr: u32, out: *mut u8, len: usize);
    pub fn pact_core_write(pc: *mut PactCore, addr: u32, data: *const u8, len: usize);
    pub fn pact_core_write16(pc: *mut PactCore, addr: u32, value: u16);
    pub fn pact_core_write32(pc: *mut PactCore, addr: u32, value: u32);
    pub fn pact_core_patch_rom(pc: *mut PactCore, addr: u32, data: *const u8, len: usize);
    pub fn pact_core_set_overlay(pc: *mut PactCore, data: *const u8, len: usize) -> bool;

    pub fn pact_core_state_size(pc: *mut PactCore) -> usize;
    pub fn pact_core_save_state(pc: *mut PactCore, out: *mut u8) -> bool;
    pub fn pact_core_load_state(pc: *mut PactCore, state: *const u8) -> bool;
    pub fn pact_core_save_memory(pc: *mut PactCore, out: *mut *mut u8) -> usize;
    pub fn pact_core_restore_save_memory(pc: *mut PactCore, data: *const u8, len: usize) -> bool;
    pub fn pact_free(p: *mut std::ffi::c_void);
}
