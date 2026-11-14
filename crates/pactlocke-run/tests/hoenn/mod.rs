//! Seam 2, local only: a Run on the real Emerald, Ruby or
//! Sapphire, driven through the Run engine on the real game interface. Each
//! game's test target runs these on its ROM, and they skip without one. Never
//! runs in CI; the ROM is only read, the save stays in memory, and the Run is
//! kept in memory too.
//!
//! The tests get about Hoenn this way: the intro is mashed
//! through, and from there a short script of the game's own commands, put in
//! free ROM and started in its script context, warps the Player (to Route
//! 101's grass, the Safari Zone) or starts a trainer's battle. The Pokémon,
//! battles, menus and rules are the real game's. The starter comes from the
//! game's own script, Route 101's Birch's bag. Test setup writes (fast text,
//! Poké Balls, a second Pokémon) go straight to the core, or into the input
//! stream when another Player keeps a Shadow Game.

use std::path::PathBuf;
use std::sync::Arc;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{Change, Game, GameProfile, Place, ProfileGame, ProfileStore, RomIdentity};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{
    FeedKind, InputChunk, MemoryStorage, NewRun, Ruleset, RunCommand, RunEngine, RunningGame, ShadowGame, ShadowState,
    StreamStore, SystemRunClock,
};

use super::GAME;

/// Which Hoenn game a test target runs these on, and the addresses the tests
/// use that its Game Profile doesn't hold (pret's symbols).
pub struct Hoenn {
    /// The ROM's file name in the folder `PACTLOCKE_ROMS` names.
    pub file: &'static str,
    /// `gBattlerControllerFuncs`.
    pub battler_controllers: u32,
    /// What `gBattlerControllerFuncs[0]` holds while the Player picks an
    /// action: `HandleInputChooseAction`, and the Safari Zone controller's own.
    pub choose_action: [u32; 2],
    /// `sLockFieldControls`: set while a script has the Player stand still.
    pub field_lock: u32,
    /// `gNumSafariBalls`.
    pub safari_balls: u32,
    /// Route 101's Birch's bag script, whose `special ChooseStarter` the catalogue lists.
    pub birchs_bag: u32,
    /// The specials `EnterSafariMode`, `StoreSelectedPokemonInDaycare` and `TakePokemonFromDaycare`.
    pub enter_safari_mode: u16,
    pub store_in_daycare: u16,
    pub take_from_daycare: u16,
    /// The party slot the Day Care's deposit reads (`gPartyMenu.slotId`, or
    /// Ruby and Sapphire's `gLastFieldPokeMenuOpened`).
    pub daycare_slot: u32,
    /// In save block 2: the key the Bag's quantities are XORed with, in Emerald.
    pub bag_key: Option<u32>,
}

/// The ROM `cargo dev accept` names in `PACTLOCKE_ROM`, or the one in the
/// folder `PACTLOCKE_ROMS` names.
fn rom() -> Option<Vec<u8>> {
    let path = match std::env::var_os("PACTLOCKE_ROM") {
        Some(file) => PathBuf::from(file),
        None => PathBuf::from(std::env::var_os("PACTLOCKE_ROMS")?).join(GAME.file),
    };
    match std::fs::read(&path) {
        Ok(rom) => Some(rom),
        Err(e) => panic!("{} can't be read: {e}", path.display()),
    }
}

macro_rules! rom_or_skip {
    () => {
        match rom() {
            Some(rom) => rom,
            None => {
                eprintln!(
                    "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
                    GAME.file
                );
                return;
            }
        }
    };
}

// The same in all three games (pret's pokeemerald and pokeruby).
const ITEMS_POCKET: u32 = 0x560; // in save block 1
const TEXT_SPEED: u32 = 0x14; // in save block 2, bits 0-2
const BATTLE_MON_LEVEL: u32 = 0x2A;
const ROUTE_101: (u8, u8) = (0, 16);
/// The first Safari Zone map, past the entrance: Emerald's SafariZone_South, Ruby's SafariZone_Southeast.
const SAFARI_ZONE: (u8, u8) = (26, 3);
const POKE_BALL: u16 = 4;
const POTION: u16 = 13;
const RARE_CANDY: u16 = 68;
/// Free ROM (0xFF in a clean ROM) for the tests' scripts, past the Sync
/// Gates' hold area and the Safari line.
const TEST_SCRIPTS: u32 = 0x08F0_1000;
/// Script commands.
const WARP: u8 = 0x39;
const WAITSTATE: u8 = 0x27;
const SPECIAL: u8 = 0x25;
const RELEASEALL: u8 = 0x6B;
const END: u8 = 0x02;
const GOTO: u8 = 0x05;
const SETVAR: u8 = 0x16;
const TRAINERBATTLE: u8 = 0x5C;
const VAR_0X8004: u16 = 0x8004;

type Running = RunningGame<ProfileGame<MgbaCore>>;

struct Driver {
    game: Running,
    profile: Arc<GameProfile>,
    rom: Vec<u8>,
    /// Another Player's app keeping a Shadow Game of this one, if any.
    watcher: Option<Watcher>,
    /// Where the next test script goes.
    next_script: u32,
}

/// About one GBA frame.
const FRAME_MS: i64 = 17;

/// Another Player's app: this Player's stream arrives after `lag_ms`, goes
/// through a stream store and is replayed by a Shadow Game on its own core.
struct Watcher {
    shadow: ShadowGame<ProfileGame<MgbaCore>>,
    outbox: Arc<std::sync::Mutex<Vec<InputChunk>>>,
    store: StreamStore,
    in_flight: std::collections::VecDeque<(i64, InputChunk)>,
    now_ms: i64,
}

impl Watcher {
    /// One frame's time: chunks are sent, those due land, and the Shadow runs.
    fn tick(&mut self) {
        self.now_ms += FRAME_MS;
        let sent: Vec<InputChunk> = std::mem::take(&mut *self.outbox.lock().unwrap());
        for chunk in sent {
            self.in_flight.push_back((self.now_ms, chunk));
        }
        while let Some((lands, _)) = self.in_flight.front()
            && *lands <= self.now_ms
        {
            let (_, chunk) = self.in_flight.pop_front().unwrap();
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
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(rom) else {
        panic!("{} should be profiled", GAME.file);
    };
    profile
}

fn run_with(rom: Vec<u8>, ruleset: Ruleset) -> Driver {
    pactlocke_mgba::force_utc_time_zone();
    let profile = profiled(&rom);
    let storage = MemoryStorage::default();
    let engine = RunEngine::start(
        NewRun {
            name: "Acceptance".into(),
            game_name: profile.name.clone(),
            game_sha1: profile.sha1.clone(),
            rom_path: None,
            ruleset,
        },
        Box::new(storage.clone()),
        Box::new(SystemRunClock),
    )
    .unwrap();
    booted_on(profile, engine, rom)
}

fn booted_on(profile: Arc<GameProfile>, engine: RunEngine, rom: Vec<u8>) -> Driver {
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    let game = ProfileGame::new(profile.clone(), rom.clone(), core);
    Driver {
        game: RunningGame::boot(game, engine, CLOCK).unwrap(),
        profile,
        rom,
        watcher: None,
        next_script: TEST_SCRIPTS,
    }
}

impl Driver {
    fn frame(&mut self, keys: Keys) {
        self.game.run_frame(keys);
        if let Some(watcher) = self.watcher.as_mut() {
            watcher.tick();
        }
    }

    /// Another Player's app starts keeping a Shadow Game of this game, which
    /// must be fresh from power-on. Test setup is stamped from now on.
    fn watched(&mut self) {
        let mut core = MgbaCore::new();
        core.load_rom(&self.rom).unwrap();
        let player: pactlocke_run::PlayerKey = [1; 32];
        let outbox = self.game.stream_as(player);
        let profile = profiled(&self.rom);
        self.watcher = Some(Watcher {
            shadow: ShadowGame::power_on(ProfileGame::new(profile, self.rom.clone(), core), CLOCK),
            outbox,
            store: StreamStore::new(),
            in_flight: Default::default(),
            now_ms: 0,
        });
    }

    /// The Shadow Game kept of this one: it must match, by RAM hash, all along.
    #[track_caller]
    fn assert_shadow_matched(&mut self, at_least: u64) {
        // Let the last of the stream land.
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

    /// A test setup change: in a watched game it is stamped into the input
    /// stream like the Run's own, so the Shadow Game sees it too.
    fn change(&mut self, change: Change) {
        if self.watcher.is_some() {
            self.game.apply_stamped(change);
        } else {
            match change {
                Change::Write { addr, bytes } => self.game.write(addr, &bytes),
                Change::Patch { addr, bytes } => self.game.patch_rom(addr, &bytes),
            }
        }
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

    fn u16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes(self.read(addr, 2).try_into().unwrap())
    }

    fn u32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.read(addr, 4).try_into().unwrap())
    }

    fn place(&mut self, place: Place) -> u32 {
        match place {
            Place::Pointer(pointer) => self.u32(pointer),
            Place::At(addr) => addr,
        }
    }

    fn save_block_1(&mut self) -> u32 {
        self.place(self.profile.symbols.save_block_1)
    }

    fn save_block_2(&mut self) -> u32 {
        self.place(self.profile.symbols.save_block_2)
    }

    fn party_slot(&self, slot: u8) -> u32 {
        self.profile.symbols.party + u32::from(slot) * self.profile.layout.party_slot_size
    }

    /// A party Pokémon's HP, as the party holds it.
    fn party_hp(&mut self, slot: u8) -> u16 {
        let at = self.party_slot(slot) + self.profile.layout.hp;
        self.u16(at)
    }

    fn set_party_hp(&mut self, slot: u8, hp: u16) {
        let at = self.party_slot(slot) + self.profile.layout.hp;
        self.write(at, &hp.to_le_bytes());
    }

    fn battle_mon(&self, position: u32) -> u32 {
        self.profile.symbols.battle_mons + position * self.profile.layout.battle_mon_size
    }

    fn battler_hp(&mut self, position: u32) -> u16 {
        let at = self.battle_mon(position) + self.profile.layout.battle_mon_hp;
        self.u16(at)
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

    /// On the overworld with no script running and the Player free to move.
    fn idle(&mut self) -> bool {
        let lock = self.u8(GAME.field_lock);
        let game = self.game.game_mut();
        game.at_safe_overworld_point() && game.script().status == pactlocke_game::ScriptStatus::Idle && lock == 0
    }

    /// `m` mashes, `h` holds, `w` waits, `p` presses then waits.
    fn play(&mut self, script: &str) {
        for line in script.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[..] {
                ["m", key, n] => (0..n.parse::<u32>().unwrap()).for_each(|i| {
                    self.frame(if i % 16 < 3 { keys(key) } else { Keys::NONE });
                }),
                ["h", key, n] => (0..n.parse::<u32>().unwrap()).for_each(|_| self.frame(keys(key))),
                ["w", n] => (0..n.parse::<u32>().unwrap()).for_each(|_| self.frame(Keys::NONE)),
                ["p", key, wait] => {
                    (0..4).for_each(|_| self.frame(keys(key)));
                    (0..wait.parse::<u32>().unwrap()).for_each(|_| self.frame(Keys::NONE));
                }
                _ => panic!("bad script line {line:?}"),
            }
        }
    }

    /// Mashes `key` until the Player is free on the overworld.
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

    /// Mashes `key` until the Player's action menu is up in battle.
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
        let controller = self.u32(GAME.battler_controllers);
        panic!("the action menu never came up (controller {controller:#010X})");
    }

    /// Mashes `key` until the battle is over.
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

    /// Walks left and right in the grass until a wild battle starts.
    #[track_caller]
    fn walk_in_grass(&mut self, limit: u32) {
        for i in 0..limit {
            if self.in_battle() {
                return;
            }
            self.frame(if (i / 16) % 2 == 1 { Keys::RIGHT } else { Keys::LEFT });
        }
        dump_screen(&self.game, "no-wild-battle");
        panic!("no wild battle in {limit} frames");
    }

    /// Picks RUN from the action menu (bottom right in the usual menu and in
    /// the Safari Zone's) until the Player gets away.
    #[track_caller]
    fn run_away(&mut self) {
        for _ in 0..10 {
            self.until_action_menu(Keys::B, 3000);
            self.play("p R 10\np D 10\np A 60");
            for i in 0..600 {
                if !self.in_battle() || self.at_action_menu() {
                    break;
                }
                self.frame(if i % 16 < 3 { Keys::B } else { Keys::NONE });
            }
            if !self.in_battle() {
                return;
            }
        }
        panic!("couldn't get away");
    }

    fn bag_key(&mut self) -> u16 {
        match GAME.bag_key {
            Some(offset) => {
                let at = self.save_block_2() + offset;
                self.u16(at)
            }
            None => 0,
        }
    }

    /// Puts `quantity` of `item` in a pocket's first slot (test setup, outside the Run).
    fn give(&mut self, pocket: u32, item: u16, quantity: u16) {
        let slot = self.save_block_1() + pocket;
        let key = self.bag_key();
        self.write(slot, &item.to_le_bytes());
        self.write(slot + 2, &(quantity ^ key).to_le_bytes());
    }

    fn quantity(&mut self, pocket: u32, item: u16) -> u16 {
        let base = self.save_block_1() + pocket;
        let key = self.bag_key();
        (0..16)
            .map(|i| base + i * 4)
            .find(|slot| self.u16(*slot) == item)
            .map_or(0, |slot| self.u16(slot + 2) ^ key)
    }

    fn balls_pocket(&self) -> u32 {
        self.profile.layout.balls_pocket
    }

    fn fast_text(&mut self) {
        let addr = self.save_block_2() + TEXT_SPEED;
        let speed = self.u8(addr);
        self.write(addr, &[(speed & !7) | 2]);
    }

    /// Runs `script` (the game's own script commands) in the game's script
    /// context, from free ROM: test setup, at a moment the Player is free.
    #[track_caller]
    fn run_script(&mut self, script: &[u8]) -> u32 {
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
        at
    }

    /// Warps the Player to `map` at (`x`, `y`), and waits until they are free there.
    #[track_caller]
    fn warp(&mut self, map: (u8, u8), x: u16, y: u16) {
        let mut script = vec![WARP, map.0, map.1, 0xFF];
        script.extend(x.to_le_bytes());
        script.extend(y.to_le_bytes());
        script.extend([WAITSTATE, END]);
        self.run_script(&script);
        self.until_idle(Keys::NONE, 1200);
    }

    fn area(&mut self) -> Option<String> {
        self.game.game_mut().area().map(|a| a.name)
    }

    fn feed(&self, kind: FeedKind) -> Vec<String> {
        let view = self.game.engine().view();
        view.feed
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
        "U" => Keys::UP,
        "D" => Keys::DOWN,
        "L" => Keys::LEFT,
        "R" => Keys::RIGHT,
        "S" => Keys::START,
        other => panic!("no key {other}"),
    }
}

/// From power-on to the moving truck: A through the intro and Birch's
/// speech, with START now and then to leave the naming screen.
fn to_the_truck(d: &mut Driver) {
    for frame in 0..30_000u32 {
        if frame > 3000 && d.idle() {
            d.play("w 120");
            assert_eq!(d.area().as_deref(), Some("Inside of Truck"));
            return;
        }
        let keys = match frame % 64 {
            0..3 => Keys::A,
            32..35 if frame > 6000 => Keys::START,
            _ => Keys::NONE,
        };
        d.frame(keys);
    }
    dump_screen(&d.game, "no-truck");
    panic!("the truck was never reached");
}

/// From power-on to the starter: to the truck, then (test setup) to Route
/// 101, where Birch's bag script runs as the game has it when the Player
/// rescues him: the starter is chosen, Birch's Poochyena fought, and the
/// script takes the Player on to the lab.
fn to_the_starter(d: &mut Driver) {
    to_the_truck(d);
    d.fast_text();
    d.warp(ROUTE_101, 10, 8);
    assert_eq!(d.area().as_deref(), Some("Route 101"));
    let mut script = vec![GOTO];
    script.extend(GAME.birchs_bag.to_le_bytes());
    d.run_script(&script);
    // A picks the starter in the middle and confirms it, then fights.
    d.until_action_menu(Keys::A, 6000);
}

/// Birch's Poochyena on 1 HP (test setup), then A through the battle and
/// everything Birch says after, until the Player is free in the lab.
fn win_the_first_battle(d: &mut Driver) {
    d.set_battler_hp(1, 1);
    d.until_battle_over(Keys::A, 6000);
    d.until_idle(Keys::A, 20_000);
}

#[test]
fn the_real_starter_is_the_starter_gift_and_birchs_poochyena_isnt_an_encounter() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            gifts: false,
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    dump_screen(&d.game, "first-battle");
    let gifts = d.feed(FeedKind::Gift);
    assert_eq!(gifts.len(), 1, "{gifts:?}");
    assert!(
        gifts[0].starts_with("Route 101: received ") && gifts[0].ends_with(", the Starter"),
        "{gifts:?}"
    );
    let battle = d.game.game_mut().battle().expect("Birch's Poochyena");
    assert_eq!(battle.kind, pactlocke_game::BattleKind::Uncatchable);
    win_the_first_battle(&mut d);
    assert!(
        d.feed(FeedKind::Encounter).is_empty(),
        "Birch's Poochyena isn't an Encounter"
    );
    assert_eq!(d.game.game_mut().party().len(), 1, "the starter is always kept");
    d.print_feed();
    d.assert_shadow_matched(10);
}

/// Writes the screen as a PPM under `target/acceptance/` for a person to look at.
fn dump_screen(game: &Running, name: &str) {
    let frame = game.frame();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("acceptance");
    std::fs::create_dir_all(&dir).unwrap();
    let mut ppm = format!("P6 {} {} 255\n", frame.width, frame.height).into_bytes();
    for px in frame.rgba.as_chunks::<4>().0 {
        ppm.extend_from_slice(&px[..3]);
    }
    let path = dir.join(format!("{}-{name}.ppm", GAME.file.split(' ').nth(2).unwrap_or("game")));
    std::fs::write(&path, ppm).unwrap();
    eprintln!("screen: {}", path.display());
}

/// The PC box slot, counting from 0, where a Pokémon sits.
fn box_slot_addr(d: &mut Driver, box_index: u32, slot: u32) -> u32 {
    let layout = d.profile.layout.clone();
    let storage = d.place(d.profile.symbols.pc_storage);
    storage + layout.pc_boxes_offset + (box_index * u32::from(layout.pc_box_slots) + slot) * layout.box_slot_size
}

/// The catch block's first site, and what the patch writes there.
fn catch_site(d: &Driver) -> (u32, Vec<u8>, Vec<u8>) {
    match &d.profile.rules.catch_block {
        Some(pactlocke_game::Method::Patch { sites }) => (
            sites[0].at,
            sites[0].expect.0.clone(),
            sites[0].write.clone().expect("a fixed patch").0,
        ),
        other => panic!("the catch block is a patch: {other:?}"),
    }
}

fn catch_block_sites(d: &Driver) -> usize {
    match &d.profile.rules.catch_block {
        Some(pactlocke_game::Method::Patch { sites }) => sites.len(),
        other => panic!("the catch block is a patch: {other:?}"),
    }
}

/// A copy of the party's first Pokémon under a PID of its own, appended to
/// the party (test setup, in battle, where it isn't taken for a Gift). Its
/// data is encrypted with PID xor OT ID and ordered by PID mod 24, so the
/// PID moves by 24 and the OT ID by the same xor, and the copy decrypts
/// exactly as the first does, checksum and all.
fn another_pokemon_in_the_party(d: &mut Driver) {
    let size = d.profile.layout.party_slot_size as usize;
    let count = d.u8(d.profile.symbols.party_count);
    let mut copy = d.read(d.party_slot(0), size);
    let pid = u32::from_le_bytes(copy[0..4].try_into().unwrap());
    let ot_id = u32::from_le_bytes(copy[4..8].try_into().unwrap());
    let new_pid = pid.wrapping_add(24 * u32::from(count));
    copy[0..4].copy_from_slice(&new_pid.to_le_bytes());
    copy[4..8].copy_from_slice(&(ot_id ^ pid ^ new_pid).to_le_bytes());
    let at = d.party_slot(count);
    d.write(at, &copy);
    d.write(d.profile.symbols.party_count, &[count + 1]);
}

/// The order of the four substructures for each PID mod 24: where Growth is.
const GROWTH_AT: [usize; 24] = [0, 0, 0, 0, 0, 0, 1, 1, 2, 3, 2, 3, 1, 1, 2, 3, 2, 3, 1, 1, 2, 3, 2, 3];

/// Sets a party Pokémon's EXP (test setup), re-encrypting it and fixing its checksum.
fn set_exp(d: &mut Driver, slot: u8, exp: u32) {
    let at = d.party_slot(slot);
    let mon = d.read(at, 80);
    let word = |o: usize| u32::from_le_bytes(mon[o..o + 4].try_into().unwrap());
    let (pid, ot_id) = (word(0), word(4));
    let key = pid ^ ot_id;
    let mut plain: Vec<u32> = (0..12).map(|i| word(0x20 + i * 4) ^ key).collect();
    plain[GROWTH_AT[(pid % 24) as usize] * 3 + 1] = exp;
    let checksum = plain.iter().fold(0u16, |sum, w| {
        sum.wrapping_add(*w as u16).wrapping_add((w >> 16) as u16)
    });
    let encrypted: Vec<u8> = plain.iter().flat_map(|w| (w ^ key).to_le_bytes()).collect();
    d.write(at + 0x20, &encrypted);
    d.write(at + 0x1C, &checksum.to_le_bytes());
}

/// To the starter, through Birch's Poochyena (adding `extra` copies of the
/// starter to the party in that battle), and on to Route 101's grass with
/// five Poké Balls.
fn to_route_101(d: &mut Driver, extra: usize) {
    to_the_starter(d);
    for _ in 0..extra {
        another_pokemon_in_the_party(d);
    }
    win_the_first_battle(d);
    d.warp(ROUTE_101, 2, 15);
    let pocket = d.balls_pocket();
    d.give(pocket, POKE_BALL, 5);
    d.play("w 180");
}

/// BAG, the Poké Balls pocket (the next one over from Items), the first
/// ball, USE: from FIGHT, then again from BAG. The Bag opens on the pocket
/// it last showed, so first left as far as it goes, to Items.
const THROW_FIRST_BALL: &str = "p R 10\np A 60\np L 30\np L 30\np L 30\np L 30\np L 30\np R 30\np A 40\np A 120";
const THROW_AGAIN: &str = "p A 60\np L 30\np L 30\np L 30\np L 30\np L 30\np R 30\np A 40\np A 120";
const TO_ITEMS_POCKET: &str = "p L 30\np L 30\np L 30\np L 30\np L 30";

/// "time and place" in the game's characters, from Dad's line.
const TIME_AND_PLACE: [u8; 8] = [0xE8, 0xDD, 0xE1, 0xD9, 0x00, 0xD5, 0xE2, 0xD8];

#[test]
fn a_closed_route_101_refuses_the_ball_and_keeps_it_and_running_still_works() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            ..Ruleset::default()
        },
    );
    d.watched();
    to_route_101(&mut d, 0);
    let (site, clean, blocked) = catch_site(&d);
    assert_eq!(d.area().as_deref(), Some("Route 101"));

    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let encounters = d.feed(FeedKind::Encounter);
    assert_eq!(encounters.len(), 1, "{encounters:?}");
    assert!(
        ["ZIGZAGOON", "WURMPLE", "POOCHYENA"]
            .iter()
            .any(|s| encounters[0].starts_with(&format!("Route 101: a wild {s},"))),
        "one of Route 101's own Pokémon: {encounters:?}"
    );
    assert_eq!(d.read(site, clean.len()), clean, "no block in the Encounter battle");
    d.run_away();

    let stamped_before = d.game.engine().stamped().len();
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    assert_eq!(d.feed(FeedKind::Block).len(), 1, "{:?}", d.feed(FeedKind::Block));
    assert_eq!(d.read(site, blocked.len()), blocked, "catch block on");
    let stamped: Vec<_> = d.game.engine().stamped()[stamped_before..]
        .iter()
        .filter(|s| matches!(s.change, Change::Patch { .. }))
        .cloned()
        .collect();
    assert_eq!(stamped.len(), catch_block_sites(&d), "every site: {stamped:?}");
    assert!(
        stamped.iter().all(|s| s.frame == stamped[0].frame),
        "at one frame boundary"
    );
    let party_before = d.game.game_mut().party();

    // Dad's line, where the catch block points the Bag's "BOX is full" line.
    let dads_line = match &d.profile.rules.catch_block {
        Some(pactlocke_game::Method::Patch { sites }) => {
            u32::from_le_bytes(sites[1].write.clone().expect("a fixed patch").0.try_into().unwrap())
        }
        _ => unreachable!(),
    };
    assert!(!showing(&mut d, &TIME_AND_PLACE, dads_line));
    d.play(THROW_FIRST_BALL);
    dump_screen(&d.game, "ball-refused");
    assert!(
        showing(&mut d, &TIME_AND_PLACE, dads_line),
        "Dad's \"time and place\" line"
    );
    assert!(d.in_battle(), "the ball wasn't thrown");
    d.play("m B 200");
    let pocket = d.balls_pocket();
    assert_eq!(d.quantity(pocket, POKE_BALL), 5, "the ball is kept");

    d.run_away();
    d.play("w 180");
    assert_eq!(d.read(site, clean.len()), clean, "catch block off after the battle");
    assert_eq!(d.game.game_mut().party(), party_before, "nothing was caught");
    d.print_feed();
    d.assert_shadow_matched(20);
}

#[test]
fn in_a_closed_safari_zone_the_safari_ball_is_refused_and_kept() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    win_the_first_battle(&mut d);
    let pocket = d.balls_pocket();
    d.give(pocket, POKE_BALL, 5);
    d.play("w 60");
    // The entrance's own special: Safari mode and 30 Safari Balls, then into the zone's grass.
    let mut script = vec![SPECIAL];
    script.extend(GAME.enter_safari_mode.to_le_bytes());
    script.extend([WARP, SAFARI_ZONE.0, SAFARI_ZONE.1, 0xFF, 8, 0, 28, 0, WAITSTATE, END]);
    d.run_script(&script);
    d.until_idle(Keys::NONE, 1200);
    assert_eq!(d.area().as_deref(), Some("Safari Zone"));
    assert_eq!(d.u8(GAME.safari_balls), 30);

    // The zone's Encounter, then away (RUN, bottom right of the Safari menu too).
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    assert_eq!(
        d.feed(FeedKind::Encounter).len(),
        1,
        "{:?}",
        d.feed(FeedKind::Encounter)
    );
    d.run_away();

    // Closed: BALL (top left) is refused, and no Safari Ball is used.
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let (site, _, blocked) = catch_site(&d);
    assert_eq!(d.read(site, blocked.len()), blocked, "catch block on");
    d.play("p A 120");
    dump_screen(&d.game, "safari-ball-refused");
    // Dad's line, from the copy the profile puts in free ROM.
    assert!(
        shows_text(&mut d, &TIME_AND_PLACE),
        "\"There's a time and place for everything!\""
    );
    d.play("m B 300");
    assert!(d.in_battle(), "nothing was caught");
    assert_eq!(d.u8(GAME.safari_balls), 30, "the Safari Ball is kept");
    d.run_away();
    d.print_feed();
    d.assert_shadow_matched(20);
}

/// Moves the START menu's cursor, which the game remembers, from POKéMON to BAG.
const START_MENU_ON_BAG: &str = "p S 60\np D 20\np B 60";

/// START, BAG, the first item of the Items pocket, USE, on the first party
/// Pokémon, then back out to the overworld.
const USE_FIRST_ITEM: &str =
    "p S 60\np A 120\np L 30\np L 30\np L 30\np L 30\np L 30\np A 40\np A 120\np A 180\nm B 600";

#[test]
fn exp_stops_at_the_cap_and_rare_candy_works_only_below_it() {
    let rom = rom_or_skip!();
    let cap = |name: &str, level| pactlocke_game::LevelCap {
        name: name.into(),
        level,
    };
    let mut d = run_with(
        rom,
        Ruleset {
            level_caps: Some(vec![cap("Roxanne", 7), cap("Brawly", 8)]),
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    assert_eq!(d.game.engine().view().level_cap.as_ref().map(|c| c.level), Some(7));
    // Birch's Poochyena at level 50 on 1 HP (test setup): a win worth many levels from 5.
    let foe = d.battle_mon(1);
    d.write(foe + BATTLE_MON_LEVEL, &[50]);
    win_the_first_battle(&mut d);
    assert_eq!(
        d.game.game_mut().party()[0].stats.unwrap().level,
        7,
        "EXP stops exactly at the cap"
    );

    // At the cap a Rare Candy is refused and kept.
    d.give(ITEMS_POCKET, RARE_CANDY, 2);
    d.play(START_MENU_ON_BAG);
    d.play(USE_FIRST_ITEM);
    dump_screen(&d.game, "candy-at-cap");
    assert_eq!(d.quantity(ITEMS_POCKET, RARE_CANDY), 2, "the Rare Candy is kept");
    assert_eq!(d.game.game_mut().party()[0].stats.unwrap().level, 7);

    // A badge (test setup) raises the cap.
    let flag = d.profile.layout.badge_flags;
    let flags = d.save_block_1() + d.profile.layout.event_flags + u32::from(flag / 8);
    let byte = d.u8(flags);
    d.write(flags, &[byte | 1 << (flag % 8)]);
    d.play("w 30");
    assert_eq!(d.feed(FeedKind::Run)[0], "Level cap raised to 8 until Brawly");

    // Below the cap the Rare Candy works, up to the cap and no further.
    d.play(USE_FIRST_ITEM);
    dump_screen(&d.game, "candy-below-cap");
    assert_eq!(d.game.game_mut().party()[0].stats.unwrap().level, 8);
    assert_eq!(d.quantity(ITEMS_POCKET, RARE_CANDY), 1);
    d.play(USE_FIRST_ITEM);
    assert_eq!(
        d.game.game_mut().party()[0].stats.unwrap().level,
        8,
        "refused at the new cap"
    );
    assert_eq!(d.quantity(ITEMS_POCKET, RARE_CANDY), 1);
    d.print_feed();
    d.assert_shadow_matched(20);
}

#[test]
fn the_day_care_returns_a_pokemon_over_the_cap_and_its_player_cant_ready_up_at_a_real_gate() {
    let rom = rom_or_skip!();
    let gate = d_first_gate(&rom);
    let mut d = run_with(
        rom,
        Ruleset {
            level_caps: Some(vec![pactlocke_game::LevelCap {
                name: gate.0.clone(),
                level: 10,
            }]),
            sync_gates: vec![gate.0.clone()],
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    another_pokemon_in_the_party(&mut d);
    win_the_first_battle(&mut d);
    assert_eq!(d.read(gate.1, 1), [GOTO], "the first gym leader's battle is held");
    set_exp(&mut d, 1, 40_000);
    assert_eq!(
        d.game.game_mut().party()[1].stats.unwrap().level,
        5,
        "EXP alone doesn't level"
    );

    // Deposited and taken back through the Day Care's own specials: it
    // comes back levelled by its EXP, past the cap (only the soft cap holds there).
    d.write(GAME.daycare_slot, &[1]);
    let mut script = vec![SPECIAL];
    script.extend(GAME.store_in_daycare.to_le_bytes());
    script.extend([SETVAR]);
    script.extend(VAR_0X8004.to_le_bytes());
    script.extend(0u16.to_le_bytes());
    script.push(SPECIAL);
    script.extend(GAME.take_from_daycare.to_le_bytes());
    script.extend([RELEASEALL, END]);
    d.run_script(&script);
    d.until_idle(Keys::B, 1200);
    let party = d.game.game_mut().party();
    assert_eq!(party.len(), 2, "back in the party");
    let level = party[1].stats.unwrap().level;
    assert!(level > 10, "levelled past the cap: {level}");

    // At the real gym leader's held battle the hold message shows, and the
    // Player can't ready up.
    let mut script = vec![GOTO];
    script.extend(gate.1.to_le_bytes());
    d.run_script(&script);
    d.play("w 60");
    dump_screen(&d.game, "held-gate");
    d.until_idle(Keys::B, 1200);
    let blocks = d.feed(FeedKind::Block);
    assert!(
        blocks
            .iter()
            .any(|b| b.starts_with(&format!("Can't ready up at {}: ", gate.0))
                && b.ends_with(&format!("is level {level}, over the level cap of 10"))),
        "{blocks:?}"
    );
    assert!(!d.in_battle(), "the held battle doesn't start");
    d.print_feed();
    d.assert_shadow_matched(20);
}

/// The first Sync Gate's name and its trainerbattle instruction.
fn d_first_gate(rom: &[u8]) -> (String, u32) {
    let profile = profiled(rom);
    let gate = &profile.sync_gates.as_ref().expect("gates").gates[0];
    (gate.name.clone(), gate.sites[0].at)
}

/// A script starting a battle with `trainer`: `trainerbattle` of `kind`
/// (3, no intro; 4, a double), each line "Go!", then `releaseall`, `end`.
fn trainer_battle(d: &Driver, kind: u8, trainer: u16) -> Vec<u8> {
    let texts = if kind == 4 { 3 } else { 1 };
    let length = 6 + 4 * texts + 2;
    let text_at = d.next_script + length as u32;
    let mut script = vec![TRAINERBATTLE, kind];
    script.extend(trainer.to_le_bytes());
    script.extend(0u16.to_le_bytes());
    for _ in 0..texts {
        script.extend(text_at.to_le_bytes());
    }
    script.extend([RELEASEALL, END]);
    script.extend([0xC1, 0xE3, 0xAB, 0xFF]); // "Go!"
    script
}

/// "Items can't" in the game's characters, as "Items can't be used now." shows it.
const ITEMS_CANT: [u8; 11] = [0xC3, 0xE8, 0xD9, 0xE1, 0xE7, 0x00, 0xD7, 0xD5, 0xE2, 0xB4, 0xE8];

/// Whether the game shows the line at `line` in ROM: Emerald expands it
/// into RAM, which `encoded` (some of its words) finds; Ruby and Sapphire
/// print it from ROM, so a text printer in RAM points into it.
fn showing(d: &mut Driver, encoded: &[u8], line: u32) -> bool {
    if shows_text(d, encoded) {
        return true;
    }
    let points_in = |ram: Vec<u8>| {
        ram.as_chunks::<4>()
            .0
            .iter()
            .any(|w| (line..line + 0x40).contains(&u32::from_le_bytes(*w)))
    };
    points_in(d.read(0x0200_0000, 0x4_0000)) || points_in(d.read(0x0300_0000, 0x8000))
}

fn shows_text(d: &mut Driver, encoded: &[u8]) -> bool {
    let found = |ram: Vec<u8>| ram.windows(encoded.len()).any(|w| w == encoded);
    found(d.read(0x0200_0000, 0x4_0000)) || found(d.read(0x0300_0000, 0x8000))
}

/// The Elite Four's Sidney (TRAINER_SIDNEY, 261 in all three): a single
/// trainer with five Pokémon. And Tate & Liza's first battle (271), a double.
const SINGLE_TRAINER: u16 = 261;
const DOUBLE_TRAINER: u16 = 271;

/// FIGHT with the first move against a foe left on 1 HP at level 2 (so the
/// win levels nothing up), then A through what follows: whether the game
/// offers to change Pokémon (Shift mode, where A opens the party menu)
/// before the trainer's next comes out.
fn knock_out_the_first_and_see_if_shift_is_offered(d: &mut Driver) -> bool {
    d.set_battler_hp(1, 1);
    let foe = d.battle_mon(1);
    d.write(foe + BATTLE_MON_LEVEL, &[2]);
    // And the Player's on plenty of HP, against the foe's real attack.
    d.set_battler_hp(0, 999);
    let battle_screen = d.main_callback();
    d.play("p A 30\np A 60");
    for i in 0..6000 {
        if d.main_callback() != battle_screen {
            (0..90).for_each(|_| d.frame(Keys::NONE));
            dump_screen(&d.game, "shift-offered");
            return true;
        }
        if i > 200 && d.at_action_menu() {
            return false;
        }
        d.frame(if i % 16 < 3 { Keys::A } else { Keys::NONE });
    }
    panic!("neither the party menu nor the action menu came up");
}

fn battle_style_is_set(d: &mut Driver) -> bool {
    let at = d.save_block_2() + 0x15;
    d.u8(at) & 0x02 != 0
}

#[test]
fn set_mode_holds_in_a_trainer_battle_and_the_bag_is_refused_there() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            no_items: true,
            set_mode: true,
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    another_pokemon_in_the_party(&mut d);
    win_the_first_battle(&mut d);
    assert!(battle_style_is_set(&mut d), "the battle style is Set");
    d.give(ITEMS_POCKET, POTION, 1);
    let script = trainer_battle(&d, 3, SINGLE_TRAINER);
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);

    // BAG: "Items can't be used now", and the Bag never opens.
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
    d.until_action_menu(Keys::B, 3000);
    d.play("p L 10");
    assert!(
        !knock_out_the_first_and_see_if_shift_is_offered(&mut d),
        "Set mode: no offer to change Pokémon"
    );
    d.print_feed();
    d.assert_shadow_matched(20);
}

#[test]
fn control_without_set_mode_the_same_battle_offers_to_change_pokemon() {
    let rom = rom_or_skip!();
    let mut d = run_with(rom, Ruleset::default());
    to_the_starter(&mut d);
    another_pokemon_in_the_party(&mut d);
    win_the_first_battle(&mut d);
    assert!(!battle_style_is_set(&mut d), "a new game's battle style is Shift");
    let script = trainer_battle(&d, 3, SINGLE_TRAINER);
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);
    assert!(knock_out_the_first_and_see_if_shift_is_offered(&mut d));
}

#[test]
fn with_no_items_only_balls_work_in_a_wild_battle_and_items_work_outside_battle() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            no_items: true,
            ..Ruleset::default()
        },
    );
    to_route_101(&mut d, 0);

    // A Potion works outside battle.
    d.set_party_hp(0, 5);
    d.give(ITEMS_POCKET, POTION, 2);
    d.play("w 60");
    d.play(START_MENU_ON_BAG);
    d.play(USE_FIRST_ITEM);
    assert!(d.party_hp(0) > 5, "the Potion healed");
    assert_eq!(d.quantity(ITEMS_POCKET, POTION), 1);

    // In a wild battle the Potion has no use...
    d.set_party_hp(0, 5);
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let hp = d.battler_hp(0);
    d.play("p R 10\np A 60");
    d.play(TO_ITEMS_POCKET);
    d.play("p A 40");
    dump_screen(&d.game, "potion-in-wild-battle");
    d.play("p A 40\np B 60");
    d.until_action_menu(Keys::B, 3000);
    assert_eq!(d.quantity(ITEMS_POCKET, POTION), 1, "the Potion is kept");
    assert_eq!(d.battler_hp(0), hp, "and nothing healed");

    // ...while a Poké Ball still throws.
    d.play(THROW_AGAIN);
    let pocket = d.balls_pocket();
    assert_eq!(d.quantity(pocket, POKE_BALL), 4, "the ball was thrown");
}

#[test]
fn in_a_double_battle_the_bag_is_locked_and_two_link_deaths_faint_both_battlers() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            faint_is_dead: true,
            no_items: true,
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    another_pokemon_in_the_party(&mut d);
    another_pokemon_in_the_party(&mut d);
    win_the_first_battle(&mut d);
    let script = trainer_battle(&d, 4, DOUBLE_TRAINER);
    d.run_script(&script);
    d.until_action_menu(Keys::A, 6000);
    let battle = d.game.game_mut().battle().expect("in battle");
    assert!(battle.double, "a double battle");
    d.play("p R 10\np A 40");
    assert!(shows_text(&mut d, &ITEMS_CANT), "the Bag is locked in doubles too");
    d.until_action_menu(Keys::B, 3000);
    d.play("p L 10");

    // Both of the Player's battlers' Link-mates died: both faint by the end of the turn.
    let party = d.game.game_mut().party();
    for slot in [0, 1] {
        d.game
            .commands()
            .send(RunCommand::LinkDeath {
                pid: party[slot].pid,
                why: format!("Blue's partner {slot} died"),
            })
            .unwrap();
    }
    d.play("w 2");
    // The third, sent in for the first to faint, outlasts the foes'
    // Earthquakes (test setup), or the party is wiped before the second's turn ends.
    d.set_party_hp(2, 999);
    let battle_screen = d.main_callback();
    for i in 0..9000 {
        if !d.in_battle() || (d.party_hp(0) == 0 && d.party_hp(1) == 0) {
            break;
        }
        // The first to faint is replaced by the third Pokémon (the party
        // menu's right-hand column), and the turn goes on.
        if d.main_callback() != battle_screen {
            d.play(
                "w 60
p R 20
p A 40
p A 90",
            );
            continue;
        }
        d.frame(if i % 16 < 3 { Keys::A } else { Keys::NONE });
    }
    dump_screen(&d.game, "double-faint");
    let deaths = d.feed(FeedKind::Death);
    for slot in [0, 1] {
        assert!(
            deaths
                .iter()
                .any(|e| e.contains(&format!("fainted with its Link: Blue's partner {slot} died"))),
            "{deaths:?}"
        );
    }
    let hp: Vec<u16> = (0..2).map(|slot| d.party_hp(slot)).collect();
    assert_eq!(hp, [0, 0], "both fainted through the game's own faint");
    d.print_feed();
    d.assert_shadow_matched(20);
}

#[test]
fn a_benched_pokemon_whose_link_mate_died_faints_at_once_and_goes_to_the_dead_box() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            faint_is_dead: true,
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    another_pokemon_in_the_party(&mut d);
    let benched = d.game.game_mut().party()[1].clone();
    d.game
        .commands()
        .send(RunCommand::LinkDeath {
            pid: benched.pid,
            why: "Blue's partner died".into(),
        })
        .unwrap();
    d.play("w 4");
    assert_eq!(d.party_hp(1), 0, "fainted on the bench at once");
    assert!(d.in_battle());
    win_the_first_battle(&mut d);
    d.play("w 120");

    let party = d.game.game_mut().party();
    assert_eq!(party.len(), 1, "gone from the party");
    let dead_box = u32::from(d.profile.layout.dead_box) - 1;
    let first = box_slot_addr(&mut d, dead_box, 0);
    assert_eq!(d.u32(first), benched.pid, "in the Dead box");
    assert!(
        d.feed(FeedKind::Death)
            .iter()
            .any(|e| e.contains("fainted with its Link: Blue's partner died")),
        "{:?}",
        d.feed(FeedKind::Death)
    );
    d.print_feed();
    d.assert_shadow_matched(20);
}

/// YES on "Give a nickname?", then the naming screen's OK with the name left
/// blank, which the game keeps as the species name.
const NAMING_LEFT_BLANK: &str = "p A 120\np S 30\np A 180";

/// Throws balls at a wild Pokémon on 1 HP (test setup) until it is caught,
/// then mashes B until "Give a nickname?" is up.
#[track_caller]
fn catch_until_the_nickname_prompt(d: &mut Driver) {
    d.set_battler_hp(1, 1);
    d.play(THROW_FIRST_BALL);
    for _ in 0..10 {
        for i in 0..4000 {
            if d.game.game_mut().nickname_prompt_up() {
                return;
            }
            if i > 200 && d.at_action_menu() {
                break;
            }
            d.frame(if i % 16 < 3 { Keys::B } else { Keys::NONE });
        }
        assert!(d.in_battle(), "the battle ended without the nickname prompt");
        d.set_battler_hp(1, 1);
        d.play(THROW_AGAIN);
    }
    panic!("the wild Pokémon was never caught");
}

/// Names every Pokémon the side panel asks about, as the side panel does.
fn name_from_the_side_panel(d: &mut Driver, name: &str) -> Vec<u32> {
    let wanted = d.game.engine().view().nickname_wanted.clone();
    for (i, w) in wanted.iter().enumerate() {
        d.game
            .commands()
            .send(RunCommand::Nickname {
                pid: w.pid,
                name: format!("{name}{i}"),
            })
            .unwrap();
    }
    d.play("w 2");
    wanted.iter().map(|w| w.pid).collect()
}

#[test]
fn must_nickname_names_a_catch_and_the_type_clause_boxes_a_second_of_a_type() {
    let rom = rom_or_skip!();
    let mut d = run_with(
        rom,
        Ruleset {
            must_nickname: true,
            type_clause: true,
            ..Ruleset::default()
        },
    );
    d.watched();
    to_route_101(&mut d, 0);
    // The starter: named in the game's own prompt, or by the side panel if
    // left with its species name.
    name_from_the_side_panel(&mut d, "STARTER");
    assert!(d.game.engine().view().nickname_wanted.is_empty());
    let pocket = d.balls_pocket();
    d.give(pocket, POKE_BALL, 30);

    // Catches: the battle's "Give a nickname?" can't be declined, until a
    // second of a type the party has goes to box 1.
    for _ in 0..8 {
        d.walk_in_grass(6000);
        d.until_action_menu(Keys::B, 3000);
        catch_until_the_nickname_prompt(&mut d);
        d.play("m B 600");
        assert!(
            d.game.game_mut().nickname_prompt_up(),
            "B can't decline the catch's nickname"
        );
        d.play(NAMING_LEFT_BLANK);
        d.until_battle_over(Keys::B, 3000);
        d.play("w 120");
        let named = name_from_the_side_panel(&mut d, "CAUGHT");
        assert_eq!(named.len(), 1, "each catch is named");
        if !d.feed(FeedKind::Block).is_empty() {
            break;
        }
    }
    let blocks = d.feed(FeedKind::Block);
    assert!(
        blocks
            .iter()
            .any(|b| b.starts_with("Type clause: ") && b.ends_with(" went to box 1")),
        "{blocks:?}"
    );
    let types: Vec<u8> = {
        let party = d.game.game_mut().party();
        party
            .iter()
            .map(|p| d.game.game_mut().primary_type(p.species))
            .collect()
    };
    let mut distinct = types.clone();
    distinct.dedup();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), types.len(), "one of each type in the party: {types:?}");
    d.print_feed();
    d.assert_shadow_matched(20);
}

/// Runs the gate's battle script from its real `trainerbattle` (test setup,
/// in place of walking up to the leader and talking to them).
fn talk_to_the_leader(d: &mut Driver, site: u32) {
    let mut script = vec![GOTO];
    script.extend(site.to_le_bytes());
    d.run_script(&script);
}

#[test]
fn a_held_sync_gate_readies_the_player_up_and_once_everyone_is_ready_the_real_battle_runs() {
    let rom = rom_or_skip!();
    let gates: Vec<(String, u32)> = profiled(&rom).sync_gates.as_ref().expect("gates").gates[..2]
        .iter()
        .map(|g| (g.name.clone(), g.sites[0].at))
        .collect();
    let hold = profiled(&rom).sync_gates.as_ref().unwrap().hold;
    let (first, site) = gates[0].clone();
    let mut d = run_with(
        rom,
        Ruleset {
            sync_gates: gates.iter().map(|g| g.0.clone()).collect(),
            ..Ruleset::default()
        },
    );
    d.watched();
    to_the_starter(&mut d);
    win_the_first_battle(&mut d);
    let clean = d.rom[(site - 0x0800_0000) as usize..][..5].to_vec();
    for (name, at) in &gates {
        assert_eq!(d.read(*at, 1), [GOTO], "{name}'s battle is held");
    }

    // At the held gate: the hold message, no battle, and the Player is Ready there.
    talk_to_the_leader(&mut d, site);
    let partners = pactlocke_game::encode_text("partners", 8).unwrap();
    let mut saw_message = false;
    for _ in 0..40 {
        saw_message |= showing(&mut d, &partners, hold);
        d.play("p A 30");
    }
    dump_screen(&d.game, "gate-held");
    assert!(saw_message, "\"Waiting for your partners to be ready...\" on screen");
    assert!(!d.in_battle(), "no battle while the gate holds");
    d.until_idle(Keys::B, 1200);
    let run = d.feed(FeedKind::Run);
    let ready = format!("Ready at {first}: waiting for your partners to be ready");
    assert!(run.contains(&ready), "{run:?}");

    // Everyone is Ready: the gate is let go, and the next gate still holds.
    d.game
        .commands()
        .send(RunCommand::OpenGate {
            gate: first.clone(),
            checkpoint: true,
        })
        .unwrap();
    d.play("w 30");
    assert_eq!(d.read(site, 5), clean, "let go: the ROM's own bytes");
    assert_eq!(d.read(gates[1].1, 1), [GOTO], "{} is still held", gates[1].0);
    let run = d.feed(FeedKind::Run);
    let on = format!("Everyone is Ready at {first}: the battle is on");
    assert!(run.contains(&on), "{run:?}");

    // The leader's own battle now runs.
    talk_to_the_leader(&mut d, site);
    d.until_action_menu(Keys::A, 6000);
    let battle = d.game.game_mut().battle().expect("the leader's battle");
    assert_eq!(battle.kind, pactlocke_game::BattleKind::Trainer);
    dump_screen(&d.game, "gate-battle");
    d.print_feed();
    d.assert_shadow_matched(20);
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
    // A draws partners; a control gets exactly A's presses and draws none; B walks off.
    let mut a = run_with(rom.clone(), Ruleset::default());
    a.watched();
    let mut control = run_with(rom.clone(), Ruleset::default());
    let mut b = run_with(rom, Ruleset::default());
    b.watched();
    for d in [&mut a, &mut control, &mut b] {
        to_the_starter(d);
        win_the_first_battle(d);
    }
    b.play("h L 32\nw 120");
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
    assert_eq!(theirs.at.map, mine.map, "the same map: Birch's lab");
    let (dx, dy) = (i32::from(theirs.at.x - mine.x), i32::from(theirs.at.y - mine.y));
    assert_ne!((dx, dy), (0, 0), "B walked off");
    // Slot 0, OAM entry 64: the first Partner Avatar.
    let entry = a.read(0x0700_0000 + 64 * 8, 6);
    let attr0 = u16::from_le_bytes([entry[0], entry[1]]);
    let attr1 = u16::from_le_bytes([entry[2], entry[3]]);
    let attr2 = u16::from_le_bytes([entry[4], entry[5]]);
    let (x, y) = (112 + 16 * dx, 56 + 16 * dy);
    assert_eq!(attr0 & 0x0200, 0, "drawn");
    assert_eq!(
        (i32::from(attr1 & 0x1FF), i32::from(attr0 & 0xFF)),
        (x, y),
        "at B's tile"
    );
    let tile = u32::from(attr2 & 0x3FF);
    assert_eq!(
        a.read(0x0601_0000 + tile * 32, 256),
        theirs.sprite.tiles,
        "B's own look, from B's game"
    );

    // The game never saw it: A's RAM is the control's, and A's Shadow still matches.
    assert_eq!(ram_hash(&mut a), ram_hash(&mut control));
    a.assert_shadow_matched(20);
    // But A's picture has B in it, where the control's doesn't.
    let (drawn, plain) = (a.game.frame(), control.game.frame());
    let differs = (y.max(0)..(y + 32).min(160)).any(|row| {
        (x.max(0)..(x + 16).min(240)).any(|col| {
            let at = ((row * 240 + col) * 4) as usize;
            drawn.rgba[at..at + 3] != plain.rgba[at..at + 3]
        })
    });
    dump_screen(&a.game, "partner-avatar");
    assert!(differs, "B is on A's screen");
}
