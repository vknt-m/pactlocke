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
/// Free ROM in both hacks (0xFF), past the Sync Gates' hold area and the Safari line.
const TEST_SCRIPTS: u32 = 0x09F0_1000;
const PIKACHU: u16 = 25;
const RATTATA: u16 = 19;
const POKE_BALL: u16 = 4;
/// Script commands.
const GIVEMON: u8 = 0x79;
const ADDITEM: u8 = 0x44;
const SETWILDBATTLE: u8 = 0xB6;
const DOWILDBATTLE: u8 = 0xB7;
const RELEASEALL: u8 = 0x6B;
const END: u8 = 0x02;
const GOTO: u8 = 0x05;
const TRAINERBATTLE: u8 = 0x5C;

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
        next_script: TEST_SCRIPTS,
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

    /// `w` waits, `p` presses then waits.
    fn play(&mut self, script: &str) {
        for line in script.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[..] {
                ["w", n] => (0..n.parse::<u32>().unwrap()).for_each(|_| self.frame(Keys::NONE)),
                ["p", key, wait] => {
                    (0..4).for_each(|_| self.frame(keys(key)));
                    (0..wait.parse::<u32>().unwrap()).for_each(|_| self.frame(Keys::NONE));
                }
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
    for frame in 0..80_000u32 {
        if frame > 3000 && frame.is_multiple_of(60) && d.idle() {
            d.play("w 120");
            if d.idle() {
                d.fast_text();
                return;
            }
        }
        let keys = match frame % 64 {
            0..3 => Keys::A,
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
    let mut script = vec![GIVEMON];
    script.extend(PIKACHU.to_le_bytes());
    script.extend([level, 0, 0]);
    script.extend([0; 9]);
    script.push(ADDITEM);
    script.extend(POKE_BALL.to_le_bytes());
    script.extend(5u16.to_le_bytes());
    script.extend([RELEASEALL, END]);
    d.run_script(&script);
    d.until_idle(Keys::A, 3000);
}

/// A scripted wild battle with a level-`level` Rattata, up to the action menu.
fn scripted_wild_battle(d: &mut Driver, level: u8) {
    let mut script = vec![SETWILDBATTLE];
    script.extend(RATTATA.to_le_bytes());
    script.extend([level, 0, 0, DOWILDBATTLE, RELEASEALL, END]);
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);
}

/// The foe on 1 HP and the Player's on plenty: FIGHT wins it, then A through what follows.
fn win_the_battle(d: &mut Driver) {
    d.set_battler_hp(1, 1);
    d.set_battler_hp(0, 999);
    d.until_battle_over(Keys::A, 6000);
    d.until_idle(Keys::B, 6000);
}

fn catch_site(d: &Driver) -> (u32, Vec<u8>, Vec<u8>) {
    match &d.profile.rules.catch_block {
        Some(Method::Patch { sites }) => (
            sites[0].at,
            sites[0].expect.0.clone(),
            sites[0].write.clone().expect("a fixed patch").0,
        ),
        other => panic!("the catch block is a patch: {other:?}"),
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

    let (site, clean, patched) = catch_site(&d);
    assert_eq!(d.read(site, clean.len()), clean);
    scripted_wild_battle(&mut d, 2);
    assert_eq!(d.read(site, patched.len()), patched, "ball throws refused in {area}");
    win_the_battle(&mut d);
    assert_eq!(d.read(site, clean.len()), clean, "lifted after the battle");
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
                level: 5,
            }]),
            ..Ruleset::default()
        },
    );
    to_the_overworld(&mut d);
    a_pikachu_and_poke_balls(&mut d, 5);
    // Whatever leads (a starter from the intro, or the Pikachu) is at the cap
    // or under it, and a level-40 foe's EXP would take it well past.
    let lead = d.party_level(0);
    assert!(lead <= 5, "the lead is level {lead}");
    scripted_wild_battle(&mut d, 40);
    win_the_battle(&mut d);
    let after = d.party_level(0);
    eprintln!("the lead went from level {lead} to {after}");
    assert!(after <= 5, "no level past the cap: {after}");
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

/// "Items can't" in the game's characters, as "Items can't be used now." shows it.
const ITEMS_CANT: [u8; 11] = [0xC3, 0xE8, 0xD9, 0xE1, 0xE7, 0x00, 0xD7, 0xD5, 0xE2, 0xB4, 0xE8];

fn shows_text(d: &mut Driver, encoded: &[u8]) -> bool {
    let found = |ram: Vec<u8>| ram.windows(encoded.len()).any(|w| w == encoded);
    found(d.read(0x0200_0000, 0x4_0000)) || found(d.read(0x0300_0000, 0x8000))
}

/// A script starting a battle with `trainer` (no intro), its line "Go!", then `releaseall`, `end`.
fn trainer_battle(d: &Driver, trainer: u16) -> Vec<u8> {
    let text_at = d.next_script + 12;
    let mut script = vec![TRAINERBATTLE, 3];
    script.extend(trainer.to_le_bytes());
    script.extend(0u16.to_le_bytes());
    script.extend(text_at.to_le_bytes());
    script.extend([RELEASEALL, END]);
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
    assert_eq!(d.game.game_mut().option("battle_style").as_deref(), Some("set"));
    a_pikachu_and_poke_balls(&mut d, 50);
    let script = trainer_battle(&d, GAME.single_trainer);
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);

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
    let trainer = u16::from_le_bytes([clean[2], clean[3]]);
    eprintln!("{first} (trainer {trainer}) battle on");
    dump_screen(&d.game, "gate-battle");
    d.print_feed();
    d.assert_shadow_matched(10);
}
