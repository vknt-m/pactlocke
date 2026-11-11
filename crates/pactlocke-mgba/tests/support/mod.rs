#![allow(dead_code)]

use pactlocke_emu::CoreAdapter;

/// A tiny GBA ROM built in memory: an endless loop at the entry point and a
/// header with game code `AXVE`, which libmgba recognises and gives a
/// real-time clock and 128 KiB flash save. Nothing is ever written to disk.
pub fn rtc_test_rom() -> Vec<u8> {
    test_rom_with_code(b"AXVE")
}

/// The same tiny ROM with a game code libmgba doesn't know, so it finds the
/// save type only when the game first touches its save.
pub fn unknown_game_rom() -> Vec<u8> {
    test_rom_with_code(b"ZPAC")
}

fn test_rom_with_code(code: &[u8; 4]) -> Vec<u8> {
    let mut rom = vec![0u8; 64 * 1024];
    rom[0..4].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes()); // b .
    rom[0xA0..0xAC].copy_from_slice(b"PACTLOCKTEST");
    rom[0xAC..0xB0].copy_from_slice(code);
    rom[0xB2] = 0x96;
    rom
}

/// 2026-01-01T00:00:00Z, a Thursday.
pub const NEW_YEAR_2026_UTC_MS: i64 = 1_767_225_600_000;

/// The date and time as the game reads it from the cartridge clock, in BCD.
#[derive(Debug, PartialEq, Eq)]
pub struct RtcDateTime {
    pub year: u8,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

const GPIO_DATA: u32 = 0x0800_00C4;
const GPIO_DIRECTION: u32 = 0x0800_00C6;
const GPIO_CONTROL: u32 = 0x0800_00C8;
const SCK: u16 = 1;
const SIO: u16 = 2;
const CS: u16 = 4;

/// Read the date and time the way a game does: bit-bang the RTC's serial
/// protocol through the cartridge GPIO port.
pub fn read_rtc(core: &mut impl CoreAdapter) -> RtcDateTime {
    core.write16(GPIO_CONTROL, 1);
    core.write16(GPIO_DIRECTION, SCK | SIO | CS);
    core.write16(GPIO_DATA, SCK);
    core.write16(GPIO_DATA, SCK | CS);
    // Command "read date/time", sent least significant bit first: magic 0b0110,
    // command 2, read bit.
    let command: u8 = 0x06 | (2 << 4) | 0x80;
    for bit in 0..8 {
        let sio = u16::from((command >> bit) & 1) << 1;
        core.write16(GPIO_DATA, CS | sio);
        core.write16(GPIO_DATA, CS | SCK | sio);
    }
    core.write16(GPIO_DIRECTION, SCK | CS);
    let mut bytes = [0u8; 7];
    for byte in &mut bytes {
        for bit in 0..8 {
            core.write16(GPIO_DATA, CS);
            let mut pins = [0u8; 2];
            core.read(GPIO_DATA, &mut pins);
            *byte |= ((pins[0] >> 1) & 1) << bit;
            core.write16(GPIO_DATA, CS | SCK);
        }
    }
    core.write16(GPIO_DATA, SCK);
    RtcDateTime {
        year: bytes[0],
        month: bytes[1],
        day: bytes[2],
        weekday: bytes[3],
        hour: bytes[4] & 0x3F,
        minute: bytes[5],
        second: bytes[6],
    }
}

/// Write one byte to a 128 KiB flash save the way a game does.
pub fn flash_write_byte(core: &mut impl CoreAdapter, offset: u16, value: u8) {
    core.write(0x0E00_5555, &[0xAA]);
    core.write(0x0E00_2AAA, &[0x55]);
    core.write(0x0E00_5555, &[0xA0]);
    core.write(0x0E00_0000 + u32::from(offset), &[value]);
}

pub fn flash_read_byte(core: &mut impl CoreAdapter, offset: u16) -> u8 {
    let mut b = [0u8];
    core.read(0x0E00_0000 + u32::from(offset), &mut b);
    b[0]
}
