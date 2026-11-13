//! The core adapter interface every emulator core implements, and the emulator
//! thread that runs one game at a chosen speed and publishes its frames.
#![warn(missing_docs)]

mod emulator;
mod frame_slot;

pub use emulator::{AudioSink, Clock, Emulator, EmulatorOptions, SystemClock};
pub use frame_slot::{FrameSlot, PublishedFrame};

use std::fmt;

/// The GBA's buttons as a key mask, in the hardware's bit order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Keys(pub u16);

impl Keys {
    /// No button held.
    pub const NONE: Keys = Keys(0);
    /// The A button.
    pub const A: Keys = Keys(1 << 0);
    /// The B button.
    pub const B: Keys = Keys(1 << 1);
    /// The Select button.
    pub const SELECT: Keys = Keys(1 << 2);
    /// The Start button.
    pub const START: Keys = Keys(1 << 3);
    /// Right on the D-pad.
    pub const RIGHT: Keys = Keys(1 << 4);
    /// Left on the D-pad.
    pub const LEFT: Keys = Keys(1 << 5);
    /// Up on the D-pad.
    pub const UP: Keys = Keys(1 << 6);
    /// Down on the D-pad.
    pub const DOWN: Keys = Keys(1 << 7);
    /// The R shoulder button.
    pub const R: Keys = Keys(1 << 8);
    /// The L shoulder button.
    pub const L: Keys = Keys(1 << 9);

    /// Whether every key in `other` is held.
    pub fn contains(self, other: Keys) -> bool {
        self.0 & other.0 == other.0
    }

    /// These keys with `other` let go.
    pub fn without(self, other: Keys) -> Keys {
        Keys(self.0 & !other.0)
    }
}

impl std::ops::BitOr for Keys {
    type Output = Keys;
    fn bitor(self, rhs: Keys) -> Keys {
        Keys(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Keys {
    fn bitor_assign(&mut self, rhs: Keys) {
        self.0 |= rhs.0;
    }
}

/// How fast the game runs compared with a real GBA. Pactlocke allows 1x to 4x.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speed {
    /// A real GBA's speed.
    X1 = 1,
    /// Twice as fast.
    X2 = 2,
    /// Three times as fast.
    X3 = 3,
    /// Four times as fast, the most Pactlocke allows.
    X4 = 4,
}

impl Speed {
    /// Every speed, slowest first.
    pub const ALL: [Speed; 4] = [Speed::X1, Speed::X2, Speed::X3, Speed::X4];

    /// How many times a real GBA's speed: 1 to 4.
    pub fn multiplier(self) -> u32 {
        self as u32
    }

    pub(crate) fn from_multiplier(m: u8) -> Speed {
        match m {
            2 => Speed::X2,
            3 => Speed::X3,
            4 => Speed::X4,
            _ => Speed::X1,
        }
    }
}

impl fmt::Display for Speed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x", self.multiplier())
    }
}

/// One video frame as tightly packed RGBA8 (alpha always 255).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// In pixels.
    pub width: u32,
    /// In pixels.
    pub height: u32,
    /// Four bytes a pixel, row by row from the top left.
    pub rgba: Vec<u8>,
}

/// A checkpoint: the game's save memory plus a savestate. Restoring applies the
/// save memory first, then the savestate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// The cartridge's save memory (flash or SRAM), as the game sees it.
    pub save_memory: Vec<u8>,
    /// The core's savestate.
    pub state: Vec<u8>,
}

/// Why a core refused a ROM, a checkpoint or save memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The file isn't a ROM this core can run.
    NotARom,
    /// A savestate or save memory was refused by the core.
    BadCheckpoint,
    /// Save memory on its own was refused by the core.
    BadSave,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::NotARom => write!(f, "this file isn't a GBA ROM"),
            CoreError::BadCheckpoint => write!(f, "the checkpoint couldn't be restored"),
            CoreError::BadSave => write!(f, "the game refused the save"),
        }
    }
}

impl std::error::Error for CoreError {}

/// One plain interface over an emulator core. Memory addresses are on the
/// emulated system's bus. Implementations always use the HLE BIOS and a
/// fake-epoch real-time clock that reads as UTC.
pub trait CoreAdapter: Send {
    /// Load a ROM from memory (the file is never copied anywhere) and power on.
    fn load_rom(&mut self, rom: &[u8]) -> Result<(), CoreError>;
    /// Run exactly one frame with these keys held.
    fn run_frame(&mut self, keys: Keys);
    /// Read a block of memory byte by byte.
    fn read(&mut self, addr: u32, buf: &mut [u8]);
    /// Write a block of memory byte by byte.
    fn write(&mut self, addr: u32, data: &[u8]);
    /// One 16-bit bus write, for I/O registers that need the full width.
    fn write16(&mut self, addr: u32, value: u16);
    /// One 32-bit bus write.
    fn write32(&mut self, addr: u32, value: u32);
    /// Patch ROM bytes in memory; the ROM file is untouched.
    fn patch_rom(&mut self, addr: u32, data: &[u8]);
    /// Display-only writes (OAM, OBJ tiles, OBJ palettes) applied as every
    /// frame starts to draw, after the game's own VBlank copies, until
    /// replaced. They never touch work RAM, so the game can't see them.
    /// A core without a display to draw on ignores them.
    fn overlay(&mut self, _writes: &[(u32, Vec<u8>)]) {}
    /// Take a checkpoint (save memory plus savestate).
    fn checkpoint(&mut self) -> Checkpoint;
    /// Restore a checkpoint: save memory first, then the savestate.
    fn restore(&mut self, checkpoint: &Checkpoint) -> Result<(), CoreError>;
    /// A copy of the game's save memory (the cartridge save); empty while
    /// the game hasn't shown which kind of save it uses.
    fn save_memory(&mut self) -> Vec<u8>;
    /// Replace the game's save memory, as if the cartridge held this save.
    /// Done right after `load_rom`, the game boots with it.
    fn load_save_memory(&mut self, save: &[u8]) -> Result<(), CoreError>;
    /// Start the fake-epoch clock at this Unix time in milliseconds. It then
    /// advances with emulated time only.
    fn set_clock(&mut self, epoch_ms: i64);
    /// The current video frame.
    fn frame(&self) -> Frame;
    /// The rate of the samples `drain_audio` returns.
    fn audio_sample_rate(&self) -> u32;
    /// Move all audio produced so far into `out` as interleaved stereo.
    fn drain_audio(&mut self, out: &mut Vec<i16>);
}
