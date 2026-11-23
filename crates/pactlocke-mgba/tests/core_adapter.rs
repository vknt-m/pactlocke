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

/// Runs `frames` frames with no keys held.
fn run(core: &mut MgbaCore, frames: u32) {
    for _ in 0..frames {
        core.run_frame(Keys::NONE);
    }
}

#[test]
fn a_checkpoint_restored_on_a_core_that_ran_on_reads_the_time_it_was_taken() {
    // A Retry restores a Run Checkpoint on the core that kept playing after it.
    let mut core = powered_on();
    core.set_clock(NEW_YEAR_2026_UTC_MS);
    run(&mut core, 3600);
    let checkpoint = core.checkpoint();
    run(&mut core, 3 * 3600);

    core.set_clock(NEW_YEAR_2026_UTC_MS);
    core.restore(&checkpoint).expect("the checkpoint restores");
    core.run_frame(Keys::NONE);

    let now = read_rtc(&mut core);
    assert_eq!((now.hour, now.minute, now.second), (0x00, 0x01, 0x00));
}

#[test]
fn the_clock_reads_its_power_on_time_plus_the_time_emulated() {
    let mut core = powered_on();
    core.set_clock(NEW_YEAR_2026_UTC_MS);
    run(&mut core, 3600);
    // 3,600 frames of 280,896 cycles at 16,777,216 Hz, in whole milliseconds.
    assert_eq!(core.clock_ms(), NEW_YEAR_2026_UTC_MS + 60_273);
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
fn a_tone_the_game_plays_is_audible() {
    let mut core = powered_on();
    // What a game does: switch the sound chip on, route channel 1 to both
    // speakers at full volume, and start a square wave on it.
    core.write16(0x0400_0084, 0x0080); // SOUNDCNT_X: master enable
    core.write16(0x0400_0080, 0x1177); // SOUNDCNT_L: channel 1 left and right, volume 7
    core.write16(0x0400_0082, 0x0002); // SOUNDCNT_H: PSG at 100%
    core.write16(0x0400_0062, 0xF080); // SOUND1CNT_H: envelope 15, 50% duty
    core.write16(0x0400_0064, 0x8600); // SOUND1CNT_X: restart at a middle frequency

    let mut audio = Vec::new();
    for _ in 0..10 {
        core.run_frame(Keys::NONE);
        core.drain_audio(&mut audio);
    }
    let peak = audio.iter().map(|s| i32::from(*s).abs()).max().unwrap_or(0);
    assert!(peak > 1000, "the square wave should be audible, peak was {peak}");
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

#[test]
fn a_new_game_boots_with_save_memory_taken_from_an_earlier_one() {
    // Like moving a cartridge's save to a fresh power-on: what a resumed Run does.
    let mut first = powered_on();
    flash_write_byte(&mut first, 0x0010, 0x42);
    first.run_frame(Keys::NONE);
    let save = first.save_memory();
    assert!(!save.is_empty());

    let mut second = powered_on();
    second.load_save_memory(&save).expect("the save memory loads");
    second.run_frame(Keys::NONE);
    assert_eq!(flash_read_byte(&mut second, 0x0010), 0x42);
    assert_eq!(second.save_memory(), save);
}

#[test]
fn an_overlay_lands_in_oam_vram_and_palettes_at_every_frame_start_and_never_in_work_ram() {
    let mut plain = powered_on();
    let mut drawn = powered_on();
    let overlay = vec![
        (0x0700_0200, vec![0x40, 0x00, 0x70, 0x00, 0xC0, 0x03]), // OAM entry 64
        (0x0601_7800, vec![0x11; 32]),                           // an OBJ tile
        (0x0500_03C0, vec![0x1F, 0x00, 0xE0, 0x03]),             // OBJ palette 14
    ];
    drawn.overlay(&overlay);
    for _ in 0..30 {
        plain.run_frame(Keys::A);
        drawn.run_frame(Keys::A);
    }
    let read = |core: &mut MgbaCore, addr, len| {
        let mut out = vec![0; len];
        core.read(addr, &mut out);
        out
    };
    assert_eq!(read(&mut drawn, 0x0700_0200, 6), overlay[0].1);
    assert_eq!(read(&mut drawn, 0x0601_7800, 32), overlay[1].1);
    assert_eq!(read(&mut drawn, 0x0500_03C0, 4), overlay[2].1);
    assert_eq!(
        read(&mut drawn, 0x0200_0000, 0x4_0000),
        read(&mut plain, 0x0200_0000, 0x4_0000),
        "EWRAM untouched"
    );
    assert_eq!(
        read(&mut drawn, 0x0300_0000, 0x8000),
        read(&mut plain, 0x0300_0000, 0x8000)
    );

    // Gone once cleared, as the game's own state would have it.
    drawn.overlay(&[]);
    // (OAM takes 16-bit writes only, as on a GBA.)
    for at in [0x0700_0200, 0x0700_0202, 0x0700_0204] {
        drawn.write16(at, 0);
    }
    drawn.run_frame(Keys::NONE);
    assert_eq!(read(&mut drawn, 0x0700_0200, 6), [0; 6]);
}

#[test]
fn an_overlay_never_writes_outside_display_memory() {
    let mut plain = powered_on();
    let mut drawn = powered_on();
    drawn.overlay(&[
        (0x0200_0100, vec![0xAB; 16]),
        (0x0300_0100, vec![0xCD; 16]),
        (0x0400_0000, vec![0xFF; 2]),
    ]);
    for _ in 0..5 {
        plain.run_frame(Keys::NONE);
        drawn.run_frame(Keys::NONE);
    }
    let read = |core: &mut MgbaCore, addr, len| {
        let mut out = vec![0; len];
        core.read(addr, &mut out);
        out
    };
    assert_eq!(read(&mut drawn, 0x0200_0100, 16), read(&mut plain, 0x0200_0100, 16));
    assert_eq!(read(&mut drawn, 0x0300_0100, 16), read(&mut plain, 0x0300_0100, 16));
    assert_eq!(
        read(&mut drawn, 0x0400_0000, 2),
        read(&mut plain, 0x0400_0000, 2),
        "nor IO"
    );
}
