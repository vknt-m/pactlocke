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

The ROM is only read into memory. Pactlocke never copies or changes ROM, save or patch files, and in this early build the in-game save is kept in memory only.

## Tests

```bash
scripts/fetch-test-rom.sh   # jsmolka's gba-tests arm.gba and thumb.gba (MIT), pinned by SHA-256, into target/test-roms/
cargo test --workspace
```

- `pactlocke-emu` tests the emulator thread (pacing at each speed, the newest-frame slot, keys and audio) against a fake core.
- `pactlocke-mgba` tests the libmgba adapter against a tiny ROM the test builds in memory. The tests cover the clock reading UTC in any time zone, memory, ROM patches and checkpoints. A determinism check also compares frame hashes from the test ROMs with `tests/golden/test-roms.sha256`. That hash must match on every OS and in every time zone. After a deliberate change, such as a new libmgba pin, re-bless it with `PACTLOCKE_BLESS=1`.
- `pactlocke --smoke` opens the UI, draws a few frames and quits.

No ROM, save or patch is ever committed. CI checks for them.
