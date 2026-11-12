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

**New Run…** picks a supported ROM (FireRed (USA) 1.0 for now) and starts a named solo Run. With the First Encounter rule once you have Poké Balls, your first wild battle in each Area is its Encounter, and in an Area whose Encounter is used the game refuses a ball with its own "This isn't the time to use that!" and keeps the ball. Running is always allowed. With the **Dupes clause** (it needs First Encounter), a wild Pokémon from an evolution family you have alive, in your party or PC, isn't the Encounter: the ball is refused and the next wild battle in that Area counts. Dead, released and traded-away Pokémon don't make dupes. Families are read from the loaded ROM; when a randomizer makes evolutions random every level, only the exact species counts. Gifts (the starter, fossils, eggs, prizes, in-game trades) are logged from the Game Profile's catalogue; with **Gifts** unticked every Gift but the starter is released once its script ends, and a second Pokémon from a Gift already used is always released. A Pokémon a script gives that the catalogue doesn't list is logged as an unknown Gift and kept. A Static (a legendary, Snorlax) is a bonus Encounter of its own with **Statics** ticked, which needs First Encounter; unticked, it counts only as the Area's first Encounter. Either way one fled from or knocked out has used its Encounter. With **Faint = Dead** ticked (on by default), a Pokémon that faints is Dead for the rest of the Run: back on the overworld, before you can heal it, it is moved to the Dead box (box 14 on FireRed). A Dead Pokémon you put back in the party goes straight back to the Dead box, with a red Violation in the feed. The side panel's live feed says what happened, and its Graveyard tab lists the Dead and what killed them.

Runs are listed on the home screen; **Resume** opens one again. Each Run is one SQLite file in a `data/runs` folder next to the app (or your user data folder if that can't be written to). It holds the Run's events, your button presses with every write and patch the Run made, and the game's in-game save, which the game boots with on resume. A Run resumes from your last in-game save; suspending mid-play comes later. Outside a Run, **Open ROM…** just plays a ROM, and its in-game save is kept in memory only.

## Tests

```bash
scripts/fetch-test-rom.sh   # jsmolka's gba-tests arm.gba and thumb.gba (MIT), pinned by SHA-256, into target/test-roms/
cargo test --workspace
```

- `pactlocke-emu` tests the emulator thread (pacing at each speed, the newest-frame slot, keys and audio) against a fake core.
- `pactlocke-mgba` tests the libmgba adapter against a tiny ROM the test builds in memory. The tests cover the clock reading UTC in any time zone, memory, ROM patches and checkpoints. A determinism check also compares frame hashes from the test ROMs with `tests/golden/test-roms.sha256`. That hash must match on every OS and in every time zone. After a deliberate change, such as a new libmgba pin, re-bless it with `PACTLOCKE_BLESS=1`.
- `pactlocke-net` tests the lobby, invite links and the wire format through an in-memory transport: friends join by invite, strangers and reused invites are refused before anything they send is read, a fourth Player is turned away, a version mismatch names the version to update to, and malformed or flooding messages are dropped and then closed, never a crash. It also runs a Host and a joiner over real iroh endpoints on the same machine with relays off. A test through n0's public relays is ignored by default; run it with `cargo test -p pactlocke-net -- --ignored`.
- `pactlocke --smoke` opens the UI, draws a few frames and quits.
- `pactlocke-game` loads every bundled Game Profile (`crates/pactlocke-game/profiles/*.toml`) and checks its format, schema version and parent link, with no ROM. It tests the capability report, the profile-driven game interface over an in-memory image of a game, and the fake game the Run engine tests use.
- `pactlocke-run` plays solo Runs through the Run engine with the fake game, in-memory storage and a test-controlled clock. It also creates, lists and resumes Runs through real SQLite Run files in a temporary folder, with compressed checkpoints and the suspend slot. On the test ROMs it rebuilds a game from a checkpoint plus the presses and stamped changes after it, and checks it matches by hash, as a suspend does.
- Local only: with `PACTLOCKE_ROMS` set to a folder holding your own `Pokemon - FireRed Version (USA).gba`, the FireRed acceptance tests check its SHA-1 and patch sites, then play from power-on to the overworld and the starter and read the game back. The Run acceptance test (`pactlocke-run`) plays on to Route 1. Its first wild battle there is the Encounter. In a second one the ball is refused and kept, and running away works. A second Run test lets the starter faint to the rival, sees it moved to box 14 with the party count fixed, then puts it back in the party and sees it moved again with a red Violation. Another checks every catalogued Gift and Static against the ROM's script instructions, and that the real starter is logged as the Starter and kept with Gifts off. A third puts Route 1's Pidgey and Rattata in the PC and sees the next Route 1 battle skipped as a dupe with the ball refused. A level cap test beats the lab rival, set to level 50, with a cap of 7: the starter stops at exactly level 7. A Rare Candy through the Bag gets "It won't have any effect." and is kept. A badge then raises the cap to 8, and the candy works once. With no items, BAG against the lab rival gets "Items can't be used now". In a wild battle the Potion offers only CANCEL, while a Poké Ball still throws. Outside battle the Potion heals. The patched item table differs from the ROM's only in battle-use bytes, so held items are untouched. With set mode, a new game's Shift battle style is set to Set. When the rival sends out a second Pokémon (added by the test), there is no offer to change, where a control run in Shift mode opens the party menu. With must nickname, 1500 frames of B can't decline the starter's "Do you want to give a nickname…?" or a Route 1 catch's "Give a nickname?". A name left blank is asked for and written from the side panel, for the party and the PC. With the type clause, a second Normal type caught on Route 1 goes to box 1, with the reason in the feed. After a crash mid-battle on Route 1, the resumed Run rebuilds the game from its last checkpoint and every press since, and it matches the crashed game by RAM hash, catch block and all. Suspend and quit there resumes exactly, and the slot is then used up. The game tests also check that the default caps read from the ROM are each leader's strongest Pokémon, and that every cap site is found by byte pattern. Without the ROM they skip. They never run in CI.

No ROM, save or patch is ever committed. CI checks for them.

## Developer tools

Developer-only, never shipped to Players (the `pactlocke-dev` crate, behind a `cargo dev` alias):

```sh
cargo dev scan path/to/rom.gba --out draft.toml        # draft a Game Profile, against the bundled one with its game code
cargo dev scan path/to/hack.gba --base firered-usa-1.0  # or against a profile you name
cargo dev accept firered-usa-1.0 path/to/rom.gba        # run that profile's acceptance scripts against your ROM
```

- **The scanner** reads the ROM against a base profile. It finds moved tables by their contents (species and type names, items, trainers). It checks the base profile's patch sites and lists level-100 checks found by byte pattern. It lists Gift and Static candidates from `givemon`, `giveegg` and `setwildbattle`, new or already catalogued, and Sync Gate candidates by trainer class with their `trainerbattle` instructions. Its output is a draft that extends the base profile: an id starting `draft-`, status untested, and every finding as a comment to review. A draft can't be bundled.
- **The acceptance runner** checks the ROM's SHA-1 against the profile, then runs the profile's acceptance tests on it, the real game through the real game interface. It never runs in CI.
- **Local profiles:** set `PACTLOCKE_DEV_PROFILES` to a folder of profile TOML files and the app (and `cargo dev`) loads them next to the bundled ones, for solo Runs only. A local profile may extend a bundled one but not copy it: edit a bundled profile where it is.
