//! Seam 2, local only: a Run on a tested Romhack, driven
//! through the Run engine on the real game interface. Each hack's test target
//! runs these on its patched ROM, and they skip without one. Never runs in
//! CI; the ROM is only read, and the save and the Run stay in memory.
//!
//! A hack's maps and story are its own, so the tests get about it the way the
//! earlier sweeps did: the intro is mashed through to
//! the first free moment on the overworld, and from there short scripts of the
//! game's own commands, put in free ROM and started in its script context,
//! give a Pokémon and Poké Balls and start battles. The Pokémon, battles,
//! menus and rules are the hack's. Another Player's Shadow Game of the game
//! is kept all along and must match it by RAM hash.

use std::path::PathBuf;
use std::sync::Arc;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{Change, Game, GameProfile, LevelCap, Method, Place, ProfileGame, ProfileStore, RomIdentity};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{
    FeedKind, InputChunk, MemoryStorage, NewRun, Ruleset, RunCommand, RunEngine, RunningGame, ShadowGame, ShadowState,
    StreamStore, SystemRunClock,
};

use super::GAME;

/// Which hack a test target runs these on, and the addresses the tests use
/// that its Game Profile doesn't hold (its base game's, from pret).
pub struct Hack {
    /// The folder under `PACTLOCKE_ROMHACKS` holding the patched ROM.
    pub folder: &'static str,
    /// `gBattlerControllerFuncs`.
    pub battler_controllers: u32,
    /// What `gBattlerControllerFuncs[0]` holds while the Player picks an action.
    pub choose_action: [u32; 2],
    /// `sLockFieldControls`: set while a script has the Player stand still.
    pub field_lock: u32,
    /// A single trainer with a full team, for a trainer battle.
    pub single_trainer: u16,
    /// Free ROM (0xFF in the clean ROM) for the test scripts, past the Sync
    /// Gates' hold area and anything else the Game Profile writes there.
    pub test_scripts: u32,
    /// The game's sprite bookkeeping, to check the room Partner Avatars take.
    pub sprites: Sprites,
    /// A gate whose battle takes its trainer from a script variable, run from
    /// the start of its own script; `None` where every gate names its trainer.
    pub variable_gate: Option<VariableGate>,
    /// The forms its script commands take.
    pub scripts: Scripts,
    /// The Poké Ball's item number.
    pub poke_ball: u16,
    /// Presses from power-on through what the hack asks before its intro
    /// (its own option menus), in `Driver::play`'s words; then A is mashed
    /// through the rest.
    pub intro: &'static str,
    /// The hack's own randomizer, for Shared Seed: how a test turns it on and
    /// which battle shows its draws. `None` for a hack without one.
    pub own_randomizer: Option<OwnRandomizer>,
}

/// How a test turns a hack's own randomizer on, and where its draws show.
pub struct OwnRandomizer {
    pub on: RandomizerSwitch,
    pub shows_in: SeedShows,
}

/// The setting that turns a hack's randomizer on.
#[allow(dead_code)]
pub enum RandomizerSwitch {
    /// Bits of a byte in save block 3 (Heart and Soul's challenge settings).
    SaveBlock3 { offset: u32, mask: u8 },
    /// An event flag (R.O.W.E.'s randomized mode).
    Flag(u16),
}

/// The battle whose Pokémon the randomizer draws.
#[allow(dead_code)]
pub enum SeedShows {
    /// A scripted wild battle (a Static).
    Static,
    /// The single trainer's team.
    Trainer,
}

/// The forms a hack's script commands take.
#[allow(dead_code)]
pub enum Scripts {
    /// The original games': `givemon`, a 5-byte `setwildbattle` and a
    /// `trainerbattle` whose length goes with its type.
    Original,
    /// pokeemerald-expansion 1.15's (Heart and Soul): `callnative
    /// ScrCmd_createmon` at this address, a 10-byte `setwildbattle` and a
    /// fixed 41-byte `trainerbattle`.
    Expansion { createmon: u32 },
    /// R.O.W.E.'s: the original `givemon` with ability 3, a 10-byte
    /// `setwildbattle`, and the original `trainerbattle`.
    Rowe,
}

/// Where the game keeps track of its sprites (pret's sprite.c, which every
/// one of these hacks keeps): its base game's addresses.
pub struct Sprites {
    /// `gOamLimit`: how many hardware sprite entries the game fills.
    pub oam_limit: u32,
    /// `gSpriteTileAllocBitmap`: a bit per OBJ tile the game has taken.
    pub tile_bitmap: u32,
    /// `sSpritePaletteTags`: each OBJ palette slot's tag, 0xFFFF when free.
    pub palette_tags: u32,
}

/// pokefirered's (each hack uses its base game's set).
#[allow(dead_code)]
pub const FIRERED_SPRITES: Sprites = Sprites {
    oam_limit: 0x0202_1B44,
    tile_bitmap: 0x0202_1B48,
    palette_tags: 0x0300_0DE8,
};

/// pokeemerald's (each hack uses its base game's set).
#[allow(dead_code)]
pub const EMERALD_SPRITES: Sprites = Sprites {
    oam_limit: 0x0202_1B38,
    tile_bitmap: 0x0202_1B3C,
    palette_tags: 0x0300_0CF0,
};

/// A Sync Gate battle that picks its trainer at run time (Unbound's gyms:
/// `setvar 0x8000, gym`, a CFRU routine, then `trainerbattle 3, 0x800D`).
pub struct VariableGate {
    /// The gate's name in the Game Profile.
    pub gate: &'static str,
    /// Where its script starts setting up the battle.
    pub script: u32,
    /// `gTrainerBattleOpponent_A`.
    pub opponent: u32,
    /// The trainers it may pick, one per difficulty.
    pub trainers: &'static [u16],
}

/// The ROM `cargo dev accept` names in `PACTLOCKE_ROM`, or the patched ROM
/// in `PACTLOCKE_ROMHACKS`/`folder`.
fn rom() -> Option<Vec<u8>> {
    if let Some(file) = std::env::var_os("PACTLOCKE_ROM") {
        return Some(std::fs::read(&file).unwrap_or_else(|e| panic!("{} can't be read: {e}", file.display())));
    }
    let dir = PathBuf::from(std::env::var_os("PACTLOCKE_ROMHACKS")?).join(GAME.folder);
    let path = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} can't be read: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "gba"))?;
    Some(std::fs::read(&path).unwrap())
}

macro_rules! rom_or_skip {
    () => {
        match rom() {
            Some(rom) => rom,
            None => {
                eprintln!(
                    "skipped: set PACTLOCKE_ROMHACKS to a folder with your patched Romhacks ({} in it)",
                    GAME.folder
                );
                return;
            }
        }
    };
}

const TEXT_SPEED: u32 = 0x14; // in save block 2, bits 0-2
const PIKACHU: u16 = 25;
const RATTATA: u16 = 19;
/// Script commands.
const GIVEMON: u8 = 0x79;
const ADDITEM: u8 = 0x44;
const SETWILDBATTLE: u8 = 0xB6;
const DOWILDBATTLE: u8 = 0xB7;
const RELEASEALL: u8 = 0x6B;
const END: u8 = 0x02;
const GOTO: u8 = 0x05;
const TRAINERBATTLE: u8 = 0x5C;
const CALLNATIVE: u8 = 0x23;

type Running = RunningGame<ProfileGame<MgbaCore>>;

struct Driver {
    game: Running,
    profile: Arc<GameProfile>,
    rom: Vec<u8>,
    watcher: Option<Watcher>,
    next_script: u32,
}

/// About one GBA frame.
const FRAME_MS: i64 = 17;

/// Another Player's app: this Player's stream goes through a stream store and
/// is replayed by a Shadow Game on its own core.
struct Watcher {
    shadow: ShadowGame<ProfileGame<MgbaCore>>,
    outbox: Arc<std::sync::Mutex<Vec<InputChunk>>>,
    store: StreamStore,
    now_ms: i64,
}

impl Watcher {
    fn tick(&mut self) {
        self.now_ms += FRAME_MS;
        let sent: Vec<InputChunk> = std::mem::take(&mut *self.outbox.lock().unwrap());
        for chunk in sent {
            if let pactlocke_run::Added::New(new) = self.store.add(chunk) {
                for chunk in new {
                    self.shadow.receive(&chunk, self.now_ms);
                }
            }
        }
        let frames = self.shadow.frames_for(FRAME_MS);
        self.shadow.advance(self.now_ms, frames);
    }
}

/// The game clock a test Run starts with: 2026-01-01T00:00:00Z.
const CLOCK: i64 = 1_767_225_600_000;

fn profiled(rom: &[u8]) -> Arc<GameProfile> {
    let store = ProfileStore::bundled().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(rom) else {
        panic!("{} should be profiled", GAME.folder);
    };
    profile
}

fn run_with(rom: Vec<u8>, ruleset: Ruleset) -> Driver {
    pactlocke_mgba::force_utc_time_zone();
    let profile = profiled(&rom);
    let engine = RunEngine::start(
        NewRun {
            name: "Acceptance".into(),
            game_name: profile.name.clone(),
            game_sha1: profile.sha1.clone(),
            rom_path: None,
            ruleset,
        },
        Box::new(MemoryStorage::default()),
        Box::new(SystemRunClock),
    )
    .unwrap();
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    let game = ProfileGame::new(profile.clone(), rom.clone(), core);
    let mut d = Driver {
        game: RunningGame::boot(game, engine, CLOCK).unwrap(),
        profile,
        rom,
        watcher: None,
        next_script: GAME.test_scripts,
    };
    d.watched();
    d
}

impl Driver {
    fn frame(&mut self, keys: Keys) {
        self.game.run_frame(keys);
        if let Some(watcher) = self.watcher.as_mut() {
            watcher.tick();
        }
    }

    /// Another Player's app keeps a Shadow Game of this one from power-on.
    fn watched(&mut self) {
        let mut core = MgbaCore::new();
        core.load_rom(&self.rom).unwrap();
        let outbox = self.game.stream_as([1; 32]);
        let profile = profiled(&self.rom);
        self.watcher = Some(Watcher {
            shadow: ShadowGame::power_on(ProfileGame::new(profile, self.rom.clone(), core), CLOCK),
            outbox,
            store: StreamStore::new(),
            now_ms: 0,
        });
    }

    #[track_caller]
    fn assert_shadow_matched(&mut self, at_least: u64) {
        (0..120).for_each(|_| self.frame(Keys::NONE));
        let shadow = &self.watcher.as_ref().expect("watched").shadow;
        assert_eq!(shadow.state(), ShadowState::Matched);
        assert!(
            shadow.hashes_matched() >= at_least,
            "{} hashes matched",
            shadow.hashes_matched()
        );
        eprintln!("the Shadow Game matched {} hashes", shadow.hashes_matched());
    }

    /// A test setup change, stamped into the input stream so the Shadow Game sees it too.
    fn change(&mut self, change: Change) {
        self.game.apply_stamped(change);
    }

    fn write(&mut self, addr: u32, bytes: &[u8]) {
        self.change(Change::Write {
            addr,
            bytes: bytes.to_vec(),
        });
    }

    fn read(&mut self, addr: u32, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        self.game.read(addr, &mut out);
        out
    }

    fn u8(&mut self, addr: u32) -> u8 {
        self.read(addr, 1)[0]
    }

    fn u32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.read(addr, 4).try_into().unwrap())
    }

    fn save_block_2(&mut self) -> u32 {
        match self.profile.symbols.save_block_2 {
            Place::Pointer(pointer) => self.u32(pointer),
            Place::At(addr) => addr,
        }
    }

    fn party_level(&mut self, slot: u32) -> u8 {
        let at = self.profile.symbols.party + slot * self.profile.layout.party_slot_size + self.profile.layout.level;
        self.u8(at)
    }

    fn battle_mon(&self, position: u32) -> u32 {
        self.profile.symbols.battle_mons + position * self.profile.layout.battle_mon_size
    }

    fn set_battler_hp(&mut self, position: u32, hp: u16) {
        let at = self.battle_mon(position) + self.profile.layout.battle_mon_hp;
        self.write(at, &hp.to_le_bytes());
    }

    fn in_battle(&mut self) -> bool {
        let bit = self.profile.symbols.in_battle;
        self.u8(bit.at) & bit.mask != 0
    }

    fn main_callback(&mut self) -> u32 {
        self.u32(self.profile.symbols.main_callback2)
    }

    fn at_action_menu(&mut self) -> bool {
        let controller = self.u32(GAME.battler_controllers) & !1;
        GAME.choose_action.iter().any(|f| *f & !1 == controller)
    }

    fn idle(&mut self) -> bool {
        let lock = self.u8(GAME.field_lock);
        let game = self.game.game_mut();
        game.at_safe_overworld_point() && game.script().status == pactlocke_game::ScriptStatus::Idle && lock == 0
    }

    /// `w` waits, `p` presses then waits, `h` holds for that many frames.
    fn play(&mut self, script: &str) {
        for line in script.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[..] {
                ["w", n] => (0..n.parse::<u32>().unwrap()).for_each(|_| self.frame(Keys::NONE)),
                ["h", key, n] => (0..n.parse::<u32>().unwrap()).for_each(|_| self.frame(keys(key))),
                ["p", key, wait] => {
                    (0..4).for_each(|_| self.frame(keys(key)));
                    (0..wait.parse::<u32>().unwrap()).for_each(|_| self.frame(Keys::NONE));
                }
                // `m` mashes: the key held 3 frames of every 16.
                ["m", key, n] => (0..n.parse::<u32>().unwrap())
                    .for_each(|i| self.frame(if i % 16 < 3 { keys(key) } else { Keys::NONE })),
                _ => panic!("bad script line {line:?}"),
            }
        }
    }

    #[track_caller]
    fn until_idle(&mut self, key: Keys, limit: u32) {
        for i in 0..limit {
            if self.idle() {
                (0..30).for_each(|_| self.frame(Keys::NONE));
                if self.idle() {
                    return;
                }
            }
            self.frame(if i % 16 < 3 { key } else { Keys::NONE });
        }
        dump_screen(&self.game, "not-idle");
        panic!("the Player was never free on the overworld");
    }

    #[track_caller]
    fn until_action_menu(&mut self, key: Keys, limit: u32) {
        for i in 0..limit {
            if self.at_action_menu() {
                (0..20).for_each(|_| self.frame(Keys::NONE));
                return;
            }
            self.frame(if i % 16 < 3 { key } else { Keys::NONE });
        }
        dump_screen(&self.game, "no-action-menu");
        panic!("the action menu never came up");
    }

    #[track_caller]
    fn until_battle_over(&mut self, key: Keys, limit: u32) {
        for i in 0..limit {
            if !self.in_battle() {
                return;
            }
            self.frame(if i % 16 < 3 { key } else { Keys::NONE });
        }
        dump_screen(&self.game, "battle-not-over");
        panic!("the battle never ended");
    }

    fn fast_text(&mut self) {
        let addr = self.save_block_2() + TEXT_SPEED;
        let speed = self.u8(addr);
        self.write(addr, &[(speed & !7) | 2]);
    }

    /// Runs `script` (the game's own script commands) from free ROM in the
    /// game's script context, at a moment the Player is free.
    #[track_caller]
    fn run_script(&mut self, script: &[u8]) {
        assert!(self.idle(), "scripts start from the overworld");
        let at = self.next_script;
        self.next_script += (script.len() as u32 + 3) & !3;
        self.change(Change::Patch {
            addr: at,
            bytes: script.to_vec(),
        });
        let context = self.profile.symbols.script_context;
        let layout = self.profile.layout.clone();
        self.write(context + layout.script_context_stack_depth, &[0]);
        self.write(context + 1, &[1]); // mode: bytecode
        // pokeemerald-expansion's waitAfterCallNative and breakOnTrainerBattle
        // (padding in the original games).
        self.write(context + 3, &[0]);
        self.write(context + layout.script_context_pointer, &at.to_le_bytes());
        self.write(self.profile.symbols.script_context_status, &[0]); // running
        self.write(GAME.field_lock, &[1]);
    }

    fn feed(&self, kind: FeedKind) -> Vec<String> {
        self.game
            .engine()
            .view()
            .feed
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.text.clone())
            .collect()
    }

    fn print_feed(&self) {
        for entry in self.game.engine().view().feed.iter().rev() {
            eprintln!("feed: {}", entry.text);
        }
    }
}

fn keys(name: &str) -> Keys {
    match name {
        "A" => Keys::A,
        "B" => Keys::B,
        "D" => Keys::DOWN,
        "L" => Keys::LEFT,
        "R" => Keys::RIGHT,
        "U" => Keys::UP,
        "RS" => Keys::R,
        "S" => Keys::START,
        other => panic!("no key {other}"),
    }
}

/// Writes the screen as a PPM under the test target's folder, for a person to look at.
fn dump_screen(game: &Running, name: &str) {
    let frame = game.frame();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("acceptance");
    std::fs::create_dir_all(&dir).unwrap();
    let mut ppm = format!("P6 {} {} 255\n", frame.width, frame.height).into_bytes();
    for px in frame.rgba.as_chunks::<4>().0 {
        ppm.extend_from_slice(&px[..3]);
    }
    let path = dir.join(format!("{}-{name}.ppm", GAME.folder));
    std::fs::write(&path, ppm).unwrap();
    eprintln!("screen: {}", path.display());
}

/// From power-on through the hack's intro to the Player's first free moment
/// on the overworld: A, with START now and then to leave a naming screen.
fn to_the_overworld(d: &mut Driver) {
    d.play(GAME.intro);
    for frame in 0..80_000u32 {
        if frame > 3000 && frame.is_multiple_of(60) && d.idle() {
            d.play("w 120");
            if d.idle() {
                d.fast_text();
                return;
            }
        }
        // START leaves a naming screen; on the overworld it would open the
        // start menu (Unbound's stays open), so B closes whatever A opened.
        let keys = match frame % 64 {
            0..3 => Keys::A,
            32..35 if frame > 6000 && d.game.game_mut().at_safe_overworld_point() => Keys::B,
            32..35 if frame > 6000 => Keys::START,
            _ => Keys::NONE,
        };
        d.frame(keys);
    }
    dump_screen(&d.game, "no-overworld");
    panic!("the overworld was never reached");
}

/// `givemon` a level-`level` Pikachu and `additem` five Poké Balls, as the game's own scripts would.
fn a_pikachu_and_poke_balls(d: &mut Driver, level: u8) {
    let mut script = give_pokemon(PIKACHU, level);
    script.push(ADDITEM);
    script.extend(GAME.poke_ball.to_le_bytes());
    script.extend(5u16.to_le_bytes());
    script.extend([RELEASEALL, END]);
    d.run_script(&script);
    d.until_idle(Keys::A, 3000);
}

/// The hack's own instruction that gives a level-`level` `species`.
fn give_pokemon(species: u16, level: u8) -> Vec<u8> {
    let mut script = Vec::new();
    match GAME.scripts {
        Scripts::Original | Scripts::Rowe => {
            script.push(GIVEMON);
            script.extend(species.to_le_bytes());
            script.extend([level, 0, 0]);
            // The ability argument: R.O.W.E.'s own scripts give 3.
            let ability: u32 = if matches!(GAME.scripts, Scripts::Rowe) { 3 } else { 0 };
            script.extend(ability.to_le_bytes());
            script.extend([0; 5]);
        }
        Scripts::Expansion { createmon } => {
            script.push(CALLNATIVE);
            script.extend(createmon.to_le_bytes());
            // The Player's side, the first free slot, the species and level,
            // then the macro's defaults for nature and gender.
            script.extend([0, 6]);
            script.extend(species.to_le_bytes());
            script.extend(u16::from(level).to_le_bytes());
            script.extend(0x14u32.to_le_bytes());
            script.extend([0x1A, 0, 0x02, 0]);
        }
    }
    script
}

/// The hack's own instruction that sets up a wild battle with a level-`level` `species`.
fn set_wild_battle(species: u16, level: u8) -> Vec<u8> {
    let mut script = vec![SETWILDBATTLE];
    script.extend(species.to_le_bytes());
    script.extend([level, 0, 0]);
    // A second Pokémon, for a double: none.
    if !matches!(GAME.scripts, Scripts::Original) {
        script.extend([0; 5]);
    }
    script
}

/// A scripted wild battle with a level-`level` Rattata, up to the action menu.
fn scripted_wild_battle(d: &mut Driver, level: u8) {
    let mut script = set_wild_battle(RATTATA, level);
    script.extend([DOWILDBATTLE, RELEASEALL, END]);
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);
}

/// The foe on 1 HP with a Perish count of zero, and the Player's on plenty:
/// it faints at the end of the turn even when the lead's first move does no
/// damage (a CFRU hack's givemon teaches newer moves), then A through what follows.
fn win_the_battle(d: &mut Driver) {
    d.set_battler_hp(1, 1);
    d.set_battler_hp(0, 999);
    let perish = d.game.game_mut().faint_active_battler(1).expect("in battle");
    perish.into_iter().for_each(|change| d.change(change));
    d.until_battle_over(Keys::A, 6000);
    d.until_idle(Keys::B, 6000);
}

/// Whether ball throws are refused now: the catch block's first patch site
/// patched, or the hack's own no-catching flag set.
fn catches_refused(d: &mut Driver) -> bool {
    match d.profile.rules.catch_block.clone() {
        Some(Method::Patch { sites }) => {
            let site = &sites[0];
            let now = d.read(site.at, site.expect.0.len());
            if now == site.expect.0 {
                false
            } else {
                assert_eq!(Some(now), site.write.clone().map(|w| w.0), "patched");
                true
            }
        }
        Some(Method::SetFlag { flag }) => {
            let block = match d.profile.symbols.save_block_1 {
                Place::Pointer(pointer) => d.u32(pointer),
                Place::At(addr) => addr,
            };
            let byte = block + d.profile.layout.event_flags + u32::from(flag / 8);
            d.u8(byte) & (1 << (flag % 8)) != 0
        }
        other => panic!("the catch block is a patch or a flag: {other:?}"),
    }
}

#[test]
fn a_new_game_reaches_the_overworld_in_one_of_the_hacks_areas_with_the_shadow_game_matched() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    let area = d.game.game_mut().area();
    dump_screen(&d.game, "overworld");
    let section = d.u8(d.profile.symbols.region_map_section);
    eprintln!("first free moment in {area:?} (section {section:#04X})");
    assert!(area.is_some(), "the Game Profile lists where the Player starts");
    d.assert_shadow_matched(5);
}

#[test]
fn a_second_battle_in_an_area_has_its_ball_throws_refused_and_the_first_counted() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            gifts: true,
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    a_pikachu_and_poke_balls(&mut d, 50);
    d.play("w 60");
    assert!(
        d.game.game_mut().holds_poke_balls(),
        "the Bag's Poké Balls, where the Game Profile says"
    );
    let area = d.game.game_mut().area().expect("a listed Area").name;

    scripted_wild_battle(&mut d, 2);
    let encounters = d.feed(FeedKind::Encounter);
    assert!(
        encounters.iter().any(|e| e.contains(&area)),
        "the first battle is {area}'s Encounter: {encounters:?}"
    );
    win_the_battle(&mut d);

    assert!(!catches_refused(&mut d));
    scripted_wild_battle(&mut d, 2);
    assert!(catches_refused(&mut d), "ball throws refused in {area}");
    win_the_battle(&mut d);
    assert!(!catches_refused(&mut d), "lifted after the battle");
    d.print_feed();
    d.assert_shadow_matched(10);
}

#[test]
fn exp_stops_at_the_level_cap_in_the_hacks_own_battles() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            level_caps: Some(vec![LevelCap {
                name: "the first leader".into(),
                level: 10,
            }]),
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    a_pikachu_and_poke_balls(&mut d, 10);
    // Whatever leads (a Pokémon from the intro, or the Pikachu) is at the cap
    // or under it (R.O.W.E.'s intro gives one at 10), and a level-40 foe's
    // EXP would take it well past. A lead at the cap learns no move to stop on.
    let lead = d.party_level(0);
    assert!(lead <= 10, "the lead is level {lead}");
    scripted_wild_battle(&mut d, 40);
    win_the_battle(&mut d);
    let after = d.party_level(0);
    eprintln!("the lead went from level {lead} to {after}");
    assert!(after <= 10, "no level past the cap: {after}");
    d.print_feed();
    d.assert_shadow_matched(10);
}

#[test]
fn a_pokemon_that_faints_goes_to_the_dead_box_in_the_hacks_own_pc() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            faint_is_dead: true,
            gifts: true,
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    a_pikachu_and_poke_balls(&mut d, 20);
    a_pikachu_and_poke_balls(&mut d, 20);
    let party = d.game.game_mut().party();
    let dead = party.last().expect("the Pikachu").pid;
    // Poisoned to 0 HP on the overworld (test setup): it faints there.
    let slot = party.len() as u32 - 1;
    let hp = d.profile.symbols.party + slot * d.profile.layout.party_slot_size + d.profile.layout.hp;
    d.write(hp, &[0, 0]);
    d.play("w 240");

    let dead_box = d.profile.layout.dead_box;
    let pc = d.game.game_mut().pc();
    assert!(
        pc.iter().any(|s| s.box_number == dead_box && s.pokemon.pid == dead),
        "in box {dead_box}: {pc:?}"
    );
    assert!(
        d.game.game_mut().party().iter().all(|p| p.pid != dead),
        "out of the party"
    );
    d.print_feed();
    d.assert_shadow_matched(10);
}

#[test]
fn a_pokemon_the_game_sends_to_its_pc_is_found_there() {
    let rom = rom_or_skip!();
    let mut d = run_with(rom, Ruleset::default());
    to_the_overworld(&mut d);
    while d.game.game_mut().party().len() < 6 {
        a_pikachu_and_poke_balls(&mut d, 5);
    }
    assert!(d.game.game_mut().pc().is_empty(), "an empty PC to start");
    // With the party full, the game's own givemon stores the Pokémon in its PC.
    a_pikachu_and_poke_balls(&mut d, 7);
    let pc = d.game.game_mut().pc();
    eprintln!("the PC holds {pc:?}");
    assert!(
        pc.iter().any(|s| s.box_number == 1
            && s.pokemon.species == PIKACHU
            && s.pokemon.nickname.eq_ignore_ascii_case("Pikachu")),
        "the given Pikachu, read where and how the game keeps it: {pc:?}"
    );
    d.assert_shadow_matched(10);
}

/// "Items can't" in the game's characters, as "Items can't be used now." shows it.
const ITEMS_CANT: [u8; 11] = [0xC3, 0xE8, 0xD9, 0xE1, 0xE7, 0x00, 0xD7, 0xD5, 0xE2, 0xB4, 0xE8];

fn shows_text(d: &mut Driver, encoded: &[u8]) -> bool {
    let found = |ram: Vec<u8>| ram.windows(encoded.len()).any(|w| w == encoded);
    found(d.read(0x0200_0000, 0x4_0000)) || found(d.read(0x0300_0000, 0x8000))
}

/// A script starting a battle with `trainer` (no intro), its line "Go!", then `releaseall`, `end`.
fn trainer_battle(d: &Driver, trainer: u16) -> Vec<u8> {
    let mut script = vec![TRAINERBATTLE];
    let text_at = match GAME.scripts {
        Scripts::Original | Scripts::Rowe => {
            let text_at = d.next_script + 12;
            script.push(3);
            script.extend(trainer.to_le_bytes());
            script.extend(0u16.to_le_bytes());
            script.extend(text_at.to_le_bytes());
            text_at
        }
        Scripts::Expansion { .. } => {
            // Type 3 (no intro) with its music, then trainer A's local id,
            // number, intro, defeat line and return address; trainer B's;
            // and the victory and can't-battle lines and rival flags.
            let text_at = d.next_script + 43;
            script.extend([0x34, 0]);
            script.extend(trainer.to_le_bytes());
            script.extend(0u32.to_le_bytes());
            script.extend(text_at.to_le_bytes());
            script.extend([0; 4 + 1 + 2 + 4 + 4 + 4 + 4 + 4 + 1]);
            text_at
        }
    };
    script.extend([RELEASEALL, END]);
    assert_eq!(d.next_script + script.len() as u32, text_at);
    script.extend([0xC1, 0xE3, 0xAB, 0xFF]); // "Go!"
    script
}

#[test]
fn in_a_trainer_battle_the_bag_is_refused_and_the_battle_style_is_set() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            no_items: true,
            set_mode: true,
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    // A game that always plays Set has no battle style option to set.
    if d.profile.options.contains_key("battle_style") {
        assert_eq!(d.game.game_mut().option("battle_style").as_deref(), Some("set"));
    }
    a_pikachu_and_poke_balls(&mut d, 50);
    let script = trainer_battle(&d, GAME.single_trainer);
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);

    // A game that bans items through its own option does so from its own
    // menus: Pactlocke's part is the option, set in the game.
    if let Some(Method::Native { option, value }) = d.profile.rules.no_items_trainer.clone() {
        assert_eq!(d.game.game_mut().option(&option), Some(value), "{option} in battle");
        d.print_feed();
        d.assert_shadow_matched(10);
        return;
    }
    let battle_screen = d.main_callback();
    assert!(!shows_text(&mut d, &ITEMS_CANT));
    d.play("p R 10\np A 40");
    dump_screen(&d.game, "bag-refused");
    assert!(
        shows_text(&mut d, &ITEMS_CANT),
        "the game's own \"Items can't be used now.\""
    );
    for _ in 0..120 {
        d.frame(Keys::NONE);
        assert_eq!(d.main_callback(), battle_screen, "the Bag opened");
    }
    d.print_feed();
    d.assert_shadow_matched(10);
}

#[test]
fn a_held_sync_gate_at_the_hacks_first_leader_lets_the_real_battle_run_once_open() {
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let gates = &profile.sync_gates.as_ref().expect("gates").gates;
    let (first, site) = (gates[0].name.clone(), gates[0].sites[0].at);
    let mut d = run_with(
        rom,
        Ruleset {
            sync_gates: gates.iter().map(|g| g.name.clone()).collect(),
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    a_pikachu_and_poke_balls(&mut d, 50);
    let clean = d.rom[(site - 0x0800_0000) as usize..][..5].to_vec();
    assert_eq!(d.read(site, 1), [GOTO], "{first}'s battle is held");

    let mut script = vec![GOTO];
    script.extend(site.to_le_bytes());
    d.run_script(&script);
    for _ in 0..40 {
        d.play("p A 30");
    }
    assert!(!d.in_battle(), "no battle while the gate holds");
    d.until_idle(Keys::B, 1200);
    let ready = format!("Ready at {first}: waiting for your partners to be ready");
    assert!(d.feed(FeedKind::Run).contains(&ready), "{:?}", d.feed(FeedKind::Run));

    d.game
        .commands()
        .send(RunCommand::OpenGate {
            gate: first.clone(),
            checkpoint: true,
        })
        .unwrap();
    d.play("w 30");
    assert_eq!(d.read(site, 5), clean, "let go: the ROM's own bytes");
    let mut script = vec![GOTO];
    script.extend(site.to_le_bytes());
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);
    let battle = d.game.game_mut().battle().expect("the leader's battle");
    assert_eq!(battle.kind, pactlocke_game::BattleKind::Trainer);
    // The trainer follows the type in the original form, the flags and local id in expansion's.
    let at = if matches!(GAME.scripts, Scripts::Expansion { .. }) {
        3
    } else {
        2
    };
    let trainer = u16::from_le_bytes([clean[at], clean[at + 1]]);
    eprintln!("{first} (trainer {trainer}) battle on");
    dump_screen(&d.game, "gate-battle");
    d.print_feed();
    d.assert_shadow_matched(10);
}

#[test]
fn a_held_gate_that_picks_its_trainer_at_run_time_lets_that_trainers_battle_run_once_open() {
    let Some(gate) = GAME.variable_gate.as_ref() else {
        eprintln!("skipped: every gate in this hack names its trainer");
        return;
    };
    let rom = rom_or_skip!();
    let profile = profiled(&rom);
    let gates = &profile.sync_gates.as_ref().expect("gates").gates;
    let mut d = run_with(
        rom,
        Ruleset {
            sync_gates: gates.iter().map(|g| g.name.clone()).collect(),
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    a_pikachu_and_poke_balls(&mut d, 50);
    let mut script = vec![GOTO];
    script.extend(gate.script.to_le_bytes());
    d.run_script(&script);
    for _ in 0..40 {
        d.play("p A 30");
    }
    assert!(!d.in_battle(), "no battle while the gate holds");
    d.until_idle(Keys::B, 1200);
    let ready = format!("Ready at {}: waiting for your partners to be ready", gate.gate);
    assert!(d.feed(FeedKind::Run).contains(&ready), "{:?}", d.feed(FeedKind::Run));

    d.game
        .commands()
        .send(RunCommand::OpenGate {
            gate: gate.gate.into(),
            checkpoint: true,
        })
        .unwrap();
    d.play("w 30");
    let mut script = vec![GOTO];
    script.extend(gate.script.to_le_bytes());
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);
    let trainer = u16::from_le_bytes(d.read(gate.opponent, 2).try_into().unwrap());
    eprintln!("{} (trainer {trainer}) battle on", gate.gate);
    dump_screen(&d.game, "variable-gate-battle");
    assert!(
        gate.trainers.contains(&trainer),
        "{}'s battle, not trainer {trainer}'s",
        gate.gate
    );
    d.print_feed();
    d.assert_shadow_matched(10);
}

/// Turns the hack's own randomizer on, as the Player would in its menus.
fn randomizer_on(d: &mut Driver, on: &RandomizerSwitch) {
    let block = |d: &mut Driver, place: Option<Place>| match place.expect("the save block is placed") {
        Place::Pointer(pointer) => d.u32(pointer),
        Place::At(addr) => addr,
    };
    let (block_3, block_1, flags) = (
        d.profile.symbols.save_block_3,
        d.profile.symbols.save_block_1,
        d.profile.layout.event_flags,
    );
    match *on {
        RandomizerSwitch::SaveBlock3 { offset, mask } => {
            let at = block(d, block_3) + offset;
            let now = d.u8(at);
            d.write(at, &[now | mask]);
        }
        RandomizerSwitch::Flag(flag) => {
            let at = block(d, Some(block_1)) + flags + u32::from(flag / 8);
            let now = d.u8(at);
            d.write(at, &[now | 1 << (flag % 8)]);
        }
    }
}

/// A Player's game with Shared Seed `seed`, the hack's randomizer on, and
/// `wait` frames idled before the battle that shows its draws: the species
/// of that battle's opponents.
fn drawn_with(rom: &[u8], seed: u32, wait: u32) -> Vec<u16> {
    let own = GAME.own_randomizer.as_ref().expect("the hack has its own randomizer");
    let mut d = run_with(
        rom.to_vec(),
        Ruleset {
            romhack_seed: Some(seed),
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    randomizer_on(&mut d, &own.on);
    a_pikachu_and_poke_balls(&mut d, 50);
    d.play(&format!("w {wait}"));
    match own.shows_in {
        SeedShows::Static => scripted_wild_battle(&mut d, 5),
        SeedShows::Trainer => {
            let script = trainer_battle(&d, GAME.single_trainer);
            d.run_script(&script);
            d.until_action_menu(Keys::A, 6000);
        }
    }
    let drawn = d.game.game_mut().opponent_party().iter().map(|p| p.species).collect();
    dump_screen(&d.game, &format!("seed-{seed:08x}-{wait}"));
    d.print_feed();
    d.assert_shadow_matched(10);
    drawn
}

#[test]
fn shared_seed_draws_the_same_pokemon_in_every_players_game_whenever_they_meet_them() {
    if GAME.own_randomizer.is_none() {
        eprintln!("skipped: this hack has no randomizer of its own");
        return;
    }
    let rom = rom_or_skip!();
    let early = drawn_with(&rom, 0x0BAD_5EED, 10);
    let late = drawn_with(&rom, 0x0BAD_5EED, 377);
    eprintln!("drawn: {early:?} and {late:?}");
    assert!(!early.is_empty());
    assert_eq!(early, late, "the same seed draws the same, at any moment");
    let other = drawn_with(&rom, 0x0123_4567, 10);
    eprintln!("another seed: {other:?}");
    assert_ne!(early, other, "another seed draws otherwise");
}

/// The sprite room the Game Profile gives Partner Avatars, checked against
/// the game's own bookkeeping now: what the game fills is printed, and any
/// overlap is returned.
fn room_taken(d: &mut Driver) -> Vec<String> {
    let avatars = d
        .profile
        .avatars
        .clone()
        .expect("the Game Profile gives Partner Avatars room");
    let mut taken = Vec::new();
    let limit = d.u8(GAME.sprites.oam_limit);
    if limit > avatars.first_oam {
        taken.push(format!("gOamLimit is {limit}, past entry {}", avatars.first_oam));
    }
    let bitmap = d.read(GAME.sprites.tile_bitmap, 128);
    let used: Vec<u16> = (0..1024u16)
        .filter(|t| bitmap[usize::from(t / 8)] & (1 << (t % 8)) != 0)
        .collect();
    let last = used.last().copied().unwrap_or(0);
    let room = avatars.first_tile..avatars.first_tile + 80;
    if let Some(t) = used.iter().find(|t| room.contains(t)) {
        taken.push(format!("OBJ tile {t} is the game's"));
    }
    let tags: Vec<u16> = d
        .read(GAME.sprites.palette_tags, 32)
        .chunks(2)
        .map(|w| u16::from_le_bytes([w[0], w[1]]))
        .collect();
    for slot in &avatars.palettes {
        let tag = tags[usize::from(*slot)];
        if tag != 0xFFFF {
            taken.push(format!("OBJ palette slot {slot} holds tag {tag:#06X}"));
        }
    }
    let in_use: Vec<usize> = (0..16).filter(|i| tags[*i] != 0xFFFF).collect();
    eprintln!("the game: gOamLimit {limit}, tiles up to {last}, palette slots {in_use:?}");
    taken
}

#[test]
fn the_room_partner_avatars_take_is_free_on_the_hacks_overworld() {
    let rom = rom_or_skip!();
    if profiled(&rom).avatars.is_none() {
        eprintln!("skipped: this hack's Game Profile has no Partner Avatars yet");
        return;
    }
    let mut d = run_with(rom, Ruleset::default());
    to_the_overworld(&mut d);
    let mut taken = room_taken(&mut d);
    // Walking about loads people and field effects: look again after each try.
    for walk in ["h L 48", "h R 96", "h U 48", "h D 96"] {
        d.play(&format!("{walk}\nw 60"));
        taken.extend(room_taken(&mut d));
    }
    dump_screen(&d.game, "room");
    assert!(taken.is_empty(), "{taken:?}");
}

/// The game's RAM, hashed.
fn ram_hash(d: &mut Driver) -> String {
    use sha1::{Digest, Sha1};
    let mut sha = Sha1::new();
    sha.update(d.read(0x0200_0000, 0x4_0000));
    sha.update(d.read(0x0300_0000, 0x8000));
    sha.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_partner_avatar_is_drawn_from_their_shadow_game_and_nothing_else_changes() {
    let rom = rom_or_skip!();
    if profiled(&rom).avatars.is_none() {
        eprintln!("skipped: this hack's Game Profile has no Partner Avatars yet");
        return;
    }
    // A draws partners; a control gets exactly A's presses and draws none; B walks off.
    let mut a = run_with(rom.clone(), Ruleset::default());
    let mut control = run_with(rom.clone(), Ruleset::default());
    let mut b = run_with(rom, Ruleset::default());
    for d in [&mut a, &mut control, &mut b] {
        to_the_overworld(d);
    }
    let start = b.game.game_mut().whereabouts().expect("B on the overworld");
    for walk in ["h L 32", "h R 64", "h D 32", "h U 64"] {
        b.play(&format!("{walk}\nw 120"));
        let moved = b
            .game
            .game_mut()
            .whereabouts()
            .is_some_and(|w| (w.x, w.y) != (start.x, start.y));
        if moved {
            break;
        }
    }
    let avatars = a.game.draw_partner_avatars();
    for _ in 0..180 {
        let seen = b.watcher.as_ref().unwrap().shadow.seen().avatar.clone();
        *avatars.lock().unwrap() = seen
            .map(|seen| vec![pactlocke_run::PartnerAvatar { player: [2; 32], seen }])
            .unwrap_or_default();
        a.frame(Keys::NONE);
        control.frame(Keys::NONE);
        b.frame(Keys::NONE);
    }

    let mine = a.game.game_mut().whereabouts().expect("A on the overworld");
    let theirs = b
        .watcher
        .as_ref()
        .unwrap()
        .shadow
        .seen()
        .avatar
        .clone()
        .expect("B's Shadow shows B");
    assert_eq!(theirs.at.map, mine.map, "the same map");
    let (dx, dy) = (i32::from(theirs.at.x - mine.x), i32::from(theirs.at.y - mine.y));
    assert_ne!((dx, dy), (0, 0), "B walked off");
    let room = a.profile.avatars.clone().expect("room for Partner Avatars");
    // Slot 0: the first Partner Avatar.
    let entry = a.read(0x0700_0000 + u32::from(room.first_oam) * 8, 6);
    let attr0 = u16::from_le_bytes([entry[0], entry[1]]);
    let attr1 = u16::from_le_bytes([entry[2], entry[3]]);
    let attr2 = u16::from_le_bytes([entry[4], entry[5]]);
    // A 32x32 sprite (Unbound's own Player is one) is centred over the tile.
    let (width, inset) = if theirs.sprite.square { (32, 8) } else { (16, 0) };
    let (x, y) = (
        i32::from(room.anchor_x) + 16 * dx - inset,
        i32::from(room.anchor_y) + 16 * dy,
    );
    assert_eq!(attr0 & 0x0200, 0, "drawn");
    assert_eq!(
        (i32::from(attr1 & 0x1FF), i32::from(attr0 & 0xFF)),
        (x, y),
        "at B's tile"
    );
    let tile = u32::from(attr2 & 0x3FF);
    assert_eq!(
        a.read(0x0601_0000 + tile * 32, theirs.sprite.tiles.len()),
        theirs.sprite.tiles,
        "B's own look, from B's game"
    );

    // The game never saw it: A's RAM is the control's, and A's Shadow still matches.
    assert_eq!(ram_hash(&mut a), ram_hash(&mut control));
    a.assert_shadow_matched(10);
    // But A's picture has B in it, where the control's doesn't.
    let (drawn, plain) = (a.game.frame(), control.game.frame());
    let differs = (y.max(0)..(y + 32).min(160)).any(|row| {
        (x.max(0)..(x + width).min(240)).any(|col| {
            let at = ((row * 240 + col) * 4) as usize;
            drawn.rgba[at..at + 3] != plain.rgba[at..at + 3]
        })
    });
    dump_screen(&a.game, "partner-avatar");
    assert!(differs, "B is on A's screen");
}
