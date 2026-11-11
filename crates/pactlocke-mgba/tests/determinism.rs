//! The cross-OS determinism check: scripted runs of freely licensed test ROMs
//! must hash identically on Windows, Linux and macOS, in any time zone.
//!
//! The ROMs are jsmolka's gba-tests `arm.gba` and `thumb.gba` (MIT), which
//! work through hundreds of CPU instruction tests. `scripts/fetch-test-rom.sh`
//! puts them in `target/test-roms/`; set `PACTLOCKE_TEST_ROMS` to use another
//! folder. Without them the check is skipped, unless
//! `PACTLOCKE_REQUIRE_TEST_ROM=1` (as in CI). Set `PACTLOCKE_BLESS=1` to
//! rewrite the golden hashes after a deliberate change, such as bumping the
//! libmgba pin.

use std::path::{Path, PathBuf};

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_mgba::MgbaCore;
use sha2::{Digest, Sha256};

const ROMS: [(&str, &str); 2] = [
    (
        "arm.gba",
        "77ee88662552bdc885c1080c0172ff119d54db791bd73b21808cf1ff1fe5b40e",
    ),
    (
        "thumb.gba",
        "b5cb2291df4ab314b31c598acd9bff2ccfa0b38efff29daadfe97422ce369b67",
    ),
];
const FRAMES: u32 = 6000;
const HASH_VIDEO_EVERY: u32 = 10;
const HASH_RAM_EVERY: u32 = 600;
const CHECKPOINT_ROUND_TRIP_AT: u32 = 3000;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn rom_dir() -> PathBuf {
    std::env::var_os("PACTLOCKE_TEST_ROMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-roms"))
}

fn read_rom(name: &str) -> Option<Vec<u8>> {
    let path = rom_dir().join(name);
    match std::fs::read(&path) {
        Ok(rom) => Some(rom),
        Err(e) => {
            assert!(
                std::env::var_os("PACTLOCKE_REQUIRE_TEST_ROM").is_none(),
                "test ROM required but unreadable at {}: {e}",
                path.display()
            );
            eprintln!(
                "skipping: no test ROM at {} (run scripts/fetch-test-rom.sh)",
                path.display()
            );
            None
        }
    }
}

/// The keys held on frame `n`: a fixed pattern that touches every button.
fn scripted_keys(n: u32) -> Keys {
    const PATTERN: [Keys; 10] = [
        Keys::A,
        Keys::B,
        Keys::SELECT,
        Keys::START,
        Keys::RIGHT,
        Keys::LEFT,
        Keys::UP,
        Keys::DOWN,
        Keys::R,
        Keys::L,
    ];
    if n % 7 < 3 {
        PATTERN[(n / 7) as usize % PATTERN.len()]
    } else {
        Keys::NONE
    }
}

/// Plays the ROM from power-on with the scripted keys, round-trips a checkpoint
/// halfway, and hashes the picture and RAM along the way.
fn scripted_run_hash(rom: &[u8]) -> String {
    let mut core = MgbaCore::new();
    core.load_rom(rom).expect("test ROM loads");
    core.set_clock(1_767_225_600_000);

    let mut run = Sha256::new();
    let mut ewram = vec![0u8; 256 * 1024];
    let mut iwram = vec![0u8; 32 * 1024];
    for n in 1..=FRAMES {
        core.run_frame(scripted_keys(n));
        if n == CHECKPOINT_ROUND_TRIP_AT {
            let checkpoint = core.checkpoint();
            core.restore(&checkpoint).expect("checkpoint restores");
        }
        if n % HASH_VIDEO_EVERY == 0 {
            run.update(n.to_le_bytes());
            run.update(&core.frame().rgba);
        }
        if n % HASH_RAM_EVERY == 0 {
            core.read(0x0200_0000, &mut ewram);
            core.read(0x0300_0000, &mut iwram);
            run.update(&ewram);
            run.update(&iwram);
        }
    }
    hex(&run.finalize())
}

#[test]
fn scripted_runs_hash_the_same_as_the_golden_runs() {
    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/test-roms.sha256");
    let mut actual = String::new();
    for (name, rom_sha256) in ROMS {
        let Some(rom) = read_rom(name) else { return };
        assert_eq!(hex(&Sha256::digest(&rom)), rom_sha256, "wrong {name}");
        actual.push_str(&format!("{}  {name}\n", scripted_run_hash(&rom)));
    }

    if std::env::var_os("PACTLOCKE_BLESS").is_some() {
        std::fs::write(&golden_path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&golden_path)
        .expect("golden hash file")
        .replace("\r\n", "\n");
    assert_eq!(actual, expected, "frame hashes differ from the golden runs");
}
