//! Seam 2: the libmgba core adapter, through the `CoreAdapter`
//! interface, with a ROM the test builds in memory.

mod support;

use pactlocke_emu::{CoreAdapter, CoreError, Keys};
use pactlocke_mgba::MgbaCore;
use support::{NEW_YEAR_2026_UTC_MS, RtcDateTime, flash_read_byte, flash_write_byte, read_rtc, rtc_test_rom};

fn powered_on() -> MgbaCore {
    let mut core = MgbaCore::new();
    core.load_rom(&rtc_test_rom()).expect("the test ROM loads");
    core
}

#[test]
fn the_game_clock_reads_the_fake_epoch_as_utc_whatever_the_time_zone() {
    // This machine's own zone applies too: on a non-UTC machine the clock
    // would read local time without the fix.
    let mut core = powered_on();
    core.set_clock(NEW_YEAR_2026_UTC_MS);
    core.run_frame(Keys::NONE);

    assert_eq!(
        read_rtc(&mut core),
        RtcDateTime {
            year: 0x26,
            month: 0x01,
            day: 0x01,
            weekday: 0x04,
            hour: 0x00,
            minute: 0x00,
            second: 0x00
        }
    );
}

#[test]
fn the_game_clock_advances_with_emulated_time_not_the_wall_clock() {
    let mut core = powered_on();
    core.set_clock(NEW_YEAR_2026_UTC_MS);
    // 3,600 frames of 280,896 cycles at 16,777,216 Hz is 60.27 seconds.
    for _ in 0..3600 {
        core.run_frame(Keys::NONE);
    }
    let now = read_rtc(&mut core);
    assert_eq!((now.hour, now.minute, now.second), (0x00, 0x01, 0x00));
}

#[test]
fn a_file_that_is_not_a_gba_rom_is_refused() {
    let mut core = MgbaCore::new();
    assert_eq!(core.load_rom(b"definitely not a rom"), Err(CoreError::NotARom));
}

#[test]
fn memory_written_through_the_bus_reads_back() {
    let mut core = powered_on();
    core.write(0x0200_0010, &[1, 2, 3, 4, 5]);
    core.write32(0x0300_0100, 0xDEAD_BEEF);
    core.write16(0x0300_0104, 0xCAFE);

    let mut ewram = [0u8; 5];
    core.read(0x0200_0010, &mut ewram);
    let mut iwram = [0u8; 6];
    core.read(0x0300_0100, &mut iwram);
    assert_eq!(ewram, [1, 2, 3, 4, 5]);
    assert_eq!(iwram, [0xEF, 0xBE, 0xAD, 0xDE, 0xFE, 0xCA]);
}

#[test]
fn patched_rom_bytes_are_what_the_game_reads() {
    let mut core = powered_on();
    core.patch_rom(0x0800_0200, &[0x12, 0x34]);
    core.run_frame(Keys::NONE);

    let mut bytes = [0u8; 2];
    core.read(0x0800_0200, &mut bytes);
    assert_eq!(bytes, [0x12, 0x34]);
}

#[test]
fn restoring_a_checkpoint_brings_back_ram_and_save_memory() {
    let mut core = powered_on();
    flash_write_byte(&mut core, 0x0010, 0x42);
    core.write(0x0200_0000, &[0x11]);
    for _ in 0..10 {
        core.run_frame(Keys::NONE);
    }
    let checkpoint = core.checkpoint();
    assert!(!checkpoint.state.is_empty());

    flash_write_byte(&mut core, 0x0020, 0x24);
    core.write(0x0200_0000, &[0x99]);
    for _ in 0..10 {
        core.run_frame(Keys::NONE);
    }
    assert_eq!(flash_read_byte(&mut core, 0x0020), 0x24);

    core.restore(&checkpoint).expect("checkpoint restores");
    let mut ram = [0u8];
    core.read(0x0200_0000, &mut ram);
    assert_eq!(ram, [0x11]);
    assert_eq!(flash_read_byte(&mut core, 0x0010), 0x42);
    assert_eq!(
        flash_read_byte(&mut core, 0x0020),
        0xFF,
        "the later save write is undone"
    );
    assert_eq!(core.checkpoint(), checkpoint);
}

#[test]
fn restoring_a_checkpoint_from_before_the_first_save_clears_a_later_save() {
    let mut core = MgbaCore::new();
    core.load_rom(&support::unknown_game_rom()).unwrap();
    core.run_frame(Keys::NONE);
    let before_any_save = core.checkpoint();

    flash_write_byte(&mut core, 0x0010, 0x42);
    core.run_frame(Keys::NONE);
    assert_eq!(flash_read_byte(&mut core, 0x0010), 0x42);

    core.restore(&before_any_save).expect("checkpoint restores");
    core.run_frame(Keys::NONE);
    assert_eq!(
        flash_read_byte(&mut core, 0x0010),
        0xFF,
        "the save made after the checkpoint is gone"
    );
}

#[test]
fn restoring_the_same_checkpoint_twice_gives_the_same_game() {
    let mut core = powered_on();
    flash_write_byte(&mut core, 0x0010, 0x42);
    core.run_frame(Keys::NONE);
    let checkpoint = core.checkpoint();

    for value in [0x01, 0x02] {
        flash_write_byte(&mut core, 0x0030, value);
        core.run_frame(Keys::NONE);
        core.restore(&checkpoint).expect("checkpoint restores");
        assert_eq!(core.checkpoint(), checkpoint);
    }
}

#[test]
fn a_corrupt_checkpoint_is_refused() {
    let mut core = powered_on();
    let mut checkpoint = core.checkpoint();
    checkpoint.state.truncate(16);
    assert_eq!(core.restore(&checkpoint), Err(CoreError::BadCheckpoint));
}

#[test]
fn every_frame_has_a_full_opaque_gba_picture_and_audio() {
    let mut core = powered_on();
    core.run_frame(Keys::NONE);
    let frame = core.frame();
    assert_eq!((frame.width, frame.height), (240, 160));
    assert_eq!(frame.rgba.len(), 240 * 160 * 4);
    assert!(frame.rgba.chunks(4).all(|px| px[3] == 255));

    let mut audio = Vec::new();
    core.drain_audio(&mut audio);
    assert!(core.audio_sample_rate() > 0);
    assert!(!audio.is_empty() && audio.len() % 2 == 0, "stereo samples every frame");
}
