//! The real core's side of checkpoints: a game rebuilt from a checkpoint
//! plus the presses and stamped changes after it matches the game that ran,
//! by hash; a suspend restores exactly. Uses jsmolka's gba-tests ROM (MIT),
//! which `scripts/fetch-test-rom.sh` puts in `target/test-roms/`, so it runs
//! in CI (where `PACTLOCKE_REQUIRE_TEST_ROM` makes a missing ROM a failure).

use std::path::{Path, PathBuf};

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::Change;
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{BootPlan, CheckpointReason, Stamped, StoredCheckpoint, boot_core};
use sha1::{Digest, Sha1};

/// 2026-01-01T00:00:00Z.
const CLOCK: i64 = 1_767_225_600_000;

fn read_rom(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("PACTLOCKE_TEST_ROMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-roms"));
    match std::fs::read(dir.join(name)) {
        Ok(rom) => Some(rom),
        Err(e) => {
            assert!(
                std::env::var_os("PACTLOCKE_REQUIRE_TEST_ROM").is_none(),
                "test ROM required but unreadable: {e}"
            );
            eprintln!("skipping: no test ROM (run scripts/fetch-test-rom.sh)");
            None
        }
    }
}

/// The keys a test holds on frame `n`: something different most frames.
fn held(n: u64) -> Keys {
    [Keys::NONE, Keys::A, Keys::B | Keys::UP, Keys::RIGHT, Keys::START][(n * 7 % 5) as usize]
}

/// The game's RAM, hashed.
fn ram_hash(core: &mut MgbaCore) -> String {
    let mut sha = Sha1::new();
    let mut ewram = vec![0; 0x4_0000];
    core.read(0x0200_0000, &mut ewram);
    let mut iwram = vec![0; 0x8000];
    core.read(0x0300_0000, &mut iwram);
    sha.update(&ewram);
    sha.update(&iwram);
    sha.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The game's RAM and picture, hashed. The picture is drawn by running a
/// frame, so only compare it after one has run since a restore.
fn hash(core: &mut MgbaCore) -> String {
    let mut sha = Sha1::new();
    sha.update(ram_hash(core).as_bytes());
    sha.update(&core.frame().rgba);
    sha.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn powered_on(rom: &[u8]) -> MgbaCore {
    let mut core = MgbaCore::new();
    core.load_rom(rom).unwrap();
    boot_core(&mut core, &BootPlan::PowerOn, CLOCK).unwrap();
    core
}

#[test]
fn a_game_rebuilt_from_a_checkpoint_and_the_presses_after_it_matches_by_hash() {
    let Some(rom) = read_rom("thumb.gba") else {
        return;
    };
    let patch_at = 0x0800_0000 + rom.len() as u32 - 4;

    // The game as it ran: a patch, a checkpoint at 200, then 300 more frames
    // with a RAM write and another patch stamped along the way.
    let mut core = powered_on(&rom);
    for n in 0..200 {
        core.run_frame(held(n));
    }
    core.patch_rom(patch_at, &[0xAA, 0xBB]);
    let checkpoint = StoredCheckpoint {
        reason: CheckpointReason::InGameSave,
        boot: 1,
        frame: 200,
        stamped_before: 0,
        clock_ms: CLOCK,
        patches: vec![(patch_at, vec![0xAA, 0xBB])],
        core: core.checkpoint(),
    };
    let stamped = vec![
        Stamped {
            frame: 250,
            change: Change::Write {
                addr: 0x0200_1000,
                bytes: vec![1, 2, 3, 4],
            },
        },
        Stamped {
            frame: 400,
            change: Change::Patch {
                addr: patch_at,
                bytes: vec![0xCC, 0xDD],
            },
        },
    ];
    let mut keys = Vec::new();
    for n in 200..500 {
        for change in stamped.iter().filter(|s| s.frame == n) {
            match &change.change {
                Change::Write { addr, bytes } => core.write(*addr, bytes),
                Change::Patch { addr, bytes } => core.patch_rom(*addr, bytes),
            }
        }
        keys.push(held(n));
        core.run_frame(held(n));
    }
    let ran = hash(&mut core);

    // After the crash: a fresh core, rebuilt.
    let mut rebuilt = MgbaCore::new();
    rebuilt.load_rom(&rom).unwrap();
    let booted = boot_core(
        &mut rebuilt,
        &BootPlan::Rebuild {
            from: checkpoint,
            keys,
            stamped,
        },
        CLOCK + 60_000,
    )
    .unwrap();

    assert_eq!(hash(&mut rebuilt), ran, "the rebuilt game matches the one that ran");
    assert_eq!(booted.clock_ms, CLOCK, "on the boot's own clock");
    assert_eq!(booted.patches[&patch_at], [0xCC, 0xDD], "the patch set as it ended");
    let mut patched = [0; 2];
    rebuilt.read(patch_at, &mut patched);
    assert_eq!(patched, [0xCC, 0xDD]);
}

#[test]
fn a_suspended_game_resumes_exactly_with_its_patches() {
    let Some(rom) = read_rom("thumb.gba") else {
        return;
    };
    let patch_at = 0x0800_0000 + rom.len() as u32 - 4;
    let mut core = powered_on(&rom);
    for n in 0..300 {
        core.run_frame(held(n));
    }
    core.patch_rom(patch_at, &[0x11, 0x22]);
    let suspended = StoredCheckpoint {
        reason: CheckpointReason::Asked,
        boot: 1,
        frame: 300,
        stamped_before: 0,
        clock_ms: CLOCK,
        patches: vec![(patch_at, vec![0x11, 0x22])],
        core: core.checkpoint(),
    };
    let at_suspend = ram_hash(&mut core);

    let mut resumed = MgbaCore::new();
    resumed.load_rom(&rom).unwrap();
    boot_core(&mut resumed, &BootPlan::Suspended(suspended), CLOCK + 3_600_000).unwrap();
    assert_eq!(ram_hash(&mut resumed), at_suspend);

    // And it plays on the same: the next minute of frames, picture and all.
    for n in 300..360 {
        core.run_frame(held(n));
        resumed.run_frame(held(n));
    }
    assert_eq!(hash(&mut resumed), hash(&mut core));
    let mut patched = [0; 2];
    resumed.read(patch_at, &mut patched);
    assert_eq!(
        patched,
        [0x11, 0x22],
        "a savestate doesn't hold ROM patches; the checkpoint does"
    );
}
