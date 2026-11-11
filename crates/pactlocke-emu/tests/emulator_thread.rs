//! Seam 1: the emulator thread, driven through its public interface
//! with a fake core and a fake clock.

mod support;

use pactlocke_emu::{Emulator, EmulatorOptions, Keys, Speed};
use support::{FakeClock, FakeCore, first_pixel_as_u32};

#[test]
fn the_slot_holds_the_newest_frame_the_core_ran() {
    let core = FakeCore::new();
    let probe = core.probe();
    let emu = Emulator::spawn(core, EmulatorOptions::new(FakeClock::new()));
    let slot = emu.frames();
    probe.wait_for_frames(50);
    let core = emu.stop();

    let ran = core.keys_seen.len() as u32;
    let published = slot.latest().expect("a frame was published");
    assert!(ran >= 50);
    assert_eq!(first_pixel_as_u32(&published.frame), ran);
    assert_eq!(published.seq, u64::from(ran));
}

/// A GBA frame is 280,896 cycles of a 16,777,216 Hz clock: about 16.7427 ms.
const GBA_FRAME_NANOS: u64 = 16_742_706;

fn gaps_between_deadlines(clock: &FakeClock) -> Vec<u64> {
    let d = clock.deadlines();
    d.windows(2).map(|w| (w[1] - w[0]).as_nanos() as u64).collect()
}

fn assert_close(actual: u64, expected: u64) {
    assert!(
        actual.abs_diff(expected) <= 2,
        "frame gap {actual} ns, expected {expected} ns"
    );
}

#[test]
fn each_speed_runs_frames_that_many_times_faster_than_a_gba() {
    for (speed, expected_gap) in [
        (Speed::X1, GBA_FRAME_NANOS),
        (Speed::X2, GBA_FRAME_NANOS / 2),
        (Speed::X3, GBA_FRAME_NANOS / 3),
        (Speed::X4, GBA_FRAME_NANOS / 4),
    ] {
        let core = FakeCore::new();
        let probe = core.probe();
        let clock = FakeClock::new();
        let emu = Emulator::spawn(core, EmulatorOptions::new(clock.clone()).speed(speed));
        probe.wait_for_frames(30);
        emu.stop();

        let gaps = gaps_between_deadlines(&clock);
        assert!(gaps.len() >= 28, "{speed}: too few frames paced");
        for gap in gaps {
            assert_close(gap, expected_gap);
        }
    }
}

#[test]
fn changing_speed_takes_effect_at_a_frame_boundary() {
    let core = FakeCore::new();
    let probe = core.probe();
    let clock = FakeClock::new();
    let emu = Emulator::spawn(core, EmulatorOptions::new(clock.clone()));
    probe.wait_for_frames(20);
    emu.set_speed(Speed::X4);
    let switched_at = probe.frames_run();
    probe.wait_for_frames(switched_at + 20);
    emu.stop();

    let gaps = gaps_between_deadlines(&clock);
    let first_fast = gaps
        .iter()
        .position(|&g| g < GBA_FRAME_NANOS / 2)
        .expect("speed changed");
    // A change made while frame 20 runs already governs the wait after it.
    assert!(
        first_fast >= 18,
        "first fast gap at {first_fast}: {:?}",
        &gaps[..first_fast.min(25) + 1]
    );
    gaps[..first_fast]
        .iter()
        .for_each(|&g| assert_close(g, GBA_FRAME_NANOS));
    gaps[first_fast..]
        .iter()
        .for_each(|&g| assert_close(g, GBA_FRAME_NANOS / 4));
}

#[test]
fn held_keys_reach_the_core_on_the_following_frames() {
    let core = FakeCore::new();
    let probe = core.probe();
    let emu = Emulator::spawn(core, EmulatorOptions::new(FakeClock::new()));
    probe.wait_for_frames(10);
    emu.set_keys(Keys::A | Keys::UP);
    let pressed_at = probe.frames_run();
    probe.wait_for_frames(pressed_at + 10);
    let core = emu.stop();

    let first_held = core
        .keys_seen
        .iter()
        .position(|&k| k == Keys::A | Keys::UP)
        .expect("keys arrived");
    assert!(first_held >= 10);
    assert!(core.keys_seen[..first_held].iter().all(|&k| k == Keys::NONE));
    assert!(core.keys_seen[first_held..].iter().all(|&k| k == Keys::A | Keys::UP));
}

#[test]
fn the_game_audio_goes_to_the_sink_after_every_frame() {
    let core = FakeCore::new();
    let probe = core.probe();
    let sink = support::RecordingSink::default();
    let emu = Emulator::spawn(core, EmulatorOptions::new(FakeClock::new()).audio(sink.clone()));
    probe.wait_for_frames(5);
    let core = emu.stop();

    let pushes = sink.pushes();
    assert_eq!(pushes.len(), core.keys_seen.len(), "one push per frame");
    assert_eq!(pushes[0], (vec![1i16; 8], 32768));
    assert_eq!(pushes[4], (vec![5i16; 8], 32768));
}
