# The multi-machine test rig

Two or three headless Pactlockes, each in its own Linux container, play a scenario on your own ROMs. Each has its own network delay, loss and time zone. They reach each other over iroh through a relay of their own, and the rig checks what each one saw against the others. It runs locally only, never in CI, because it needs your ROMs.

```bash
PACTLOCKE_ROMS=~/roms bash scripts/rig.sh firered/shadow-sync
PACTLOCKE_ROMS=~/roms bash scripts/rig.sh firered              # every FireRed scenario
PACTLOCKE_ROMS=~/roms bash scripts/rig.sh firered/retry --arm64 Blue
```

`--arm64 PLAYER` runs that Player's container as ARM64 under emulation. The first ARM64 build compiles everything under emulation, which is slow; later runs reuse it. `--keep` leaves the containers so you can look at them afterwards.

To try a scenario quickly without Docker, `bash scripts/rig/local.sh firered/shadow-sync` runs the same scripts as processes on your machine. That run has no relay (joiners dial the Host's own addresses), no shaping and no time zones, and `cut` steps fail.

## What it does

1. **Images.** It builds the CI image (`scripts/ci-docker/Dockerfile`) if it's missing, and the rig image (`scripts/rig/docker/Dockerfile`): Debian with `tc` and every time zone, plus `iroh-relay` at the version `Cargo.lock` has. The rig image is rebuilt when its files change.
2. **The app.** It builds `pactlocke` in release, offline, from vendored crates (the same `pactlocke-ci-vendor` volume CI's Docker job uses), into the volume `pactlocke-rig-bins`. Builds are kept in `pactlocke-rig-target`.
3. **A scenario.** It makes an internal Docker network with no internet, starts `iroh-relay --dev` on it, and starts one container per Player:
   - your ROM folder mounted read-only at `/roms`, and the scenarios at `/scenarios`;
   - `tc netem` applying the scenario's delay and loss to the container's network;
   - the scenario's time zone in `TZ`. The emulator core always sees UTC (by design), so this checks that nothing else in the app depends on the machine's time zone. A game with a clock, such as Emerald, will test it further.

   Each runs `pactlocke --headless` on its script, with the relay at `http://pactlocke-rig-relay:3340`.
4. **The check.** Each app writes `report.json` and its log. The rig copies them to `target/rig/docker/<game>/<scenario>/`, with each container's output and the relay's, and runs `cargo dev rig-check`. That checks three things:
   - every script passed;
   - every Shadow Game of a Player who stayed ended matched;
   - every Run log is the same, entry for entry, or at most two entries behind the longest.

ROMs are only ever mounted read-only: they are never copied into an image, a volume or the repo.

## Scenarios

Each scenario is a folder in `scripts/rig/scenarios/<game>/`. `rig.env` names the Players in join order (the Host first), and each Player's `NETEM_<Player>` (arguments to `tc qdisc … netem`) and `TZ_<Player>`. Each Player's script is `<player>.txt`, in lower case. `common/` holds the scripts they include.

| Scenario | Players | What it checks |
|---|---|---|
| `shadow-sync` | 3 | Three Players play from power-on to the lab, and every Shadow Game stays matched. |
| `link-death` | 2 | The starters are one Link. Red's faints to the rival with Faint = Dead, and Blue's dies with it, judged from Red's Shadow Game. |
| `sync-gate` | 2 | The lab rival as a test-only Sync Gate holds each Player with its message until both are Ready there, then opens with a Run Checkpoint every Player is captured in. The battle itself isn't played: the lab's walk-in trigger can't run again after a hold. |
| `retry` | 2 | After the gate's Run Checkpoint, both walk out of the lab and vote for a Retry. Both games go back as a new boot, and each Shadow Game follows it and matches. |
| `disconnect` | 2 | Blue's network is cut for 90 seconds, long enough for the connection to drop, while they play on. Blue reconnects, their stream catches up, and Red's Shadow Game of Blue matches. |
| `savestate-cheat` | 2 | Blue's game goes back to its Run Checkpoint with no Retry in the log, and Red's app flags it as a savestate load. |

Emerald scenarios wait for Emerald's Game Profile: the app won't start a Run on a game it has no profile for.

## The headless app

`pactlocke --headless SCRIPT [--report FILE] [--data DIR]` has no window and no sound. It follows the script through a lobby and a Run with other Players, exactly as the windowed app plays one: the same lobby, Run log, input streams, Shadow Games and Run engine. It exits 0 when every step passed, 1 when one failed (the report says which, and the game's screen at that moment is saved beside it as a PNG), 2 when the script is wrong, and 3 after `vanish`. Its log is `DIR/pactlocke.log`, and every line goes to stderr too.

A script has one step a line; `#` starts a comment. `include FILE` reads another script there (relative to this one), and `${NAME}` is the environment variable `NAME`. The rig sets `PLAYER`, `RELAY`, `SHARED` (a folder every Player shares), `ROM` and `INVITE` (which invite is this joiner's, from 1).

**The lobby**

| Step | Does |
|---|---|
| `name NAME` | The name other Players see. |
| `relays n0` / `relays off` / `relays URL` | The relays to use. |
| `host ROM` | Opens a lobby on this ROM. |
| `invites DIR` | The Host writes each invite link to `DIR/invite-1`, `invite-2`, …, a new one once the last was used. |
| `join FILE ROM` | Joins with the invite in `FILE`, waiting for the file. |
| `preset classic\|soul-link\|hardcore`, `rule NAME on\|off`, `retries N`, `gate NAME on\|off` | The Host's Ruleset. Rules use their Ruleset names: `first_encounter`, `links`, `faint_is_dead`, `level_caps`, and so on. |
| `extra-gate NAME ADDR:BYTES…` | A battle the Game Profile doesn't list, made a Sync Gate for this test: its `trainerbattle` sites, each with the bytes a clean ROM holds. Every Player gives the same. |
| `ready` | Ready to the Ruleset, again whenever the Host changes it. |
| `players N` | The Host waits until N Players, them included, are in setup. Rules for several Players, such as Links, can only be turned on then. |
| `lock-when N` | The Host locks once N Players, them included, are Ready. |
| `shared DIR`, `sync LABEL N` | Waits until N Players sharing `DIR` reached `LABEL`. |

**The game** (each waits for the Run to start)

| Step | Does |
|---|---|
| `h KEYS N` / `m KEYS N` / `p KEYS WAIT` / `w N` | Hold for N frames, mash (3 frames in 16), press for 4 frames then wait, or wait. Keys: `A B U D L R S` (Start) `E` (Select) `LS RS`, joined with `+`. |
| `until memN ADDR [&MASK] (== V \| != V \| in V,V) [mash K \| hold K \| walk K K] LIMIT` | Plays until memory passes the test, failing after LIMIT frames. N is 8, 16 or 32; values are hex. |
| `speed 1`–`4` | The game's speed. |
| `poke ADDR BYTES` | A test setup write, stamped into the input stream so Shadow Games see it. `*PTR+OFF` reads the address from a pointer. |
| `cheat poke ADDR BYTES` | A memory edit the Run doesn't know about. |
| `cheat savestate-load` | Back to the latest Run Checkpoint with no Retry in the log. |
| `vote retry` | Starts a Retry vote, or says Yes to the open one. |
| `cut SECS` | The rig drops every packet this container sends for SECS while the game plays on. The step fails if the cut can't be made. |
| `shot FILE` | Saves the game's screen as a PNG. |
| `vanish` | Stops at once, as a machine that vanished does. |

**Checks** (each waits, playing on, `within SECS`: 30 by default)

| Step | Holds when |
|---|---|
| `expect matched` | Every Shadow Game matches and is caught up. |
| `expect connected` | This app reaches every other Player. |
| `expect no-errors` | Nothing went wrong with the lobby or the Run log. |
| `expect reconnected` | This joiner lost the Host, connected again, and reaches every Player now. |
| `expect boot N` | This Player's game, and every Shadow Game here, is on boot N or later. A Retry starts a new boot. |
| `expect log KIND… [FIELD=VALUE…] [by NAME] [about NAME] [x N]` | At least N Run log entries (1 by default) have these kinds, outermost first (`event died`, `retry`, `takeover`), these fields, by this author, and a `player` field naming this Player. |
