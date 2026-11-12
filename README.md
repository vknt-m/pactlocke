# Pactlocke (working title)

Play GBA Pokémon Nuzlocke and Soul Link challenges together, with the emulator built in. Licensed GPL-3.0-or-later.

So far the app opens a GBA ROM and plays it with keyboard input and sound at 1x to 4x speed.

## Build from a clean checkout

You need Rust (stable), CMake 3.13 or newer, and a C compiler: Visual Studio 2022 Build Tools with the C++ workload on Windows, gcc or clang on Linux, or the Xcode command line tools on macOS. On Linux you also need the ALSA, X11 and Wayland development packages (see `.github/workflows/ci.yml` for the list).

```bash
git clone --recurse-submodules <repo-url>
cargo run --release -p pactlocke-app
```

If you cloned without submodules, run `git submodule update --init`. libmgba lives in `third_party/mgba`, pinned to master `662dada`. Change the pin only on purpose, and re-run the determinism check when you do.

## Playing

Run `pactlocke`, or `pactlocke path/to/game.gba` to open a ROM straight away. The keys are mGBA's defaults:

| GBA | Keyboard |
|---|---|
| A / B | X / Z |
| L / R | A / S |
| Start / Select | Enter / Backspace |
| D-pad | Arrow keys |

The ROM is only read into memory. Pactlocke never copies or changes ROM, save or patch files.

## Runs

**New Run…** picks a supported ROM (FireRed (USA) 1.0 for now) and starts a named solo Run with the First Encounter rule: your first wild battle in each Area is its Encounter, and in an Area whose Encounter is used the game refuses a ball with its own "This isn't the time to use that!" and keeps the ball. Running is always allowed. The side panel's live feed says what happened.

Runs are listed on the home screen; **Resume** opens one again. Each Run is one SQLite file in a `data/runs` folder next to the app (or your user data folder if that can't be written to). It holds the Run's events, your button presses with every write and patch the Run made, and the game's in-game save, which the game boots with on resume. A Run resumes from your last in-game save; suspending mid-play comes later. Outside a Run, **Open ROM…** just plays a ROM, and its in-game save is kept in memory only.

## Tests

```bash
scripts/fetch-test-rom.sh   # jsmolka's gba-tests arm.gba and thumb.gba (MIT), pinned by SHA-256, into target/test-roms/
cargo test --workspace
```

- `pactlocke-emu` tests the emulator thread (pacing at each speed, the newest-frame slot, keys and audio) against a fake core.
- `pactlocke-mgba` tests the libmgba adapter against a tiny ROM the test builds in memory. The tests cover the clock reading UTC in any time zone, memory, ROM patches and checkpoints. A determinism check also compares frame hashes from the test ROMs with `tests/golden/test-roms.sha256`. That hash must match on every OS and in every time zone. After a deliberate change, such as a new libmgba pin, re-bless it with `PACTLOCKE_BLESS=1`.
- `pactlocke --smoke` opens the UI, draws a few frames and quits.
- `pactlocke-game` loads every bundled Game Profile (`crates/pactlocke-game/profiles/*.toml`) and checks its format, schema version and parent link, with no ROM. It tests the capability report, the profile-driven game interface over an in-memory image of a game, and the fake game the Run engine tests use.
- `pactlocke-run` plays solo Runs through the Run engine with the fake game, in-memory storage and a test-controlled clock. It also creates, lists and resumes Runs through real SQLite Run files in a temporary folder.
- Local only: with `PACTLOCKE_ROMS` set to a folder holding your own `Pokemon - FireRed Version (USA).gba`, the FireRed acceptance tests check its SHA-1 and patch sites, then play from power-on to the overworld and the starter and read the game back. The Run acceptance test (`pactlocke-run`) plays on to Route 1. Its first wild battle there is the Encounter. In a second one the ball is refused and kept, and running away works. Without the ROM they skip. They never run in CI.

No ROM, save or patch is ever committed. CI checks for them.
