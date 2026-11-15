//! Seam 2, local only: a Run on the real
//! FireRed or LeafGreen (USA) 1.0, driven through the Run engine on the real
//! game interface. Each game's test target runs these on its ROM, and they
//! skip without one. Never runs in CI; the ROM is only read, the save stays
//! in memory, and the Run is kept in memory too.
//!
//! A few setup writes (fast text, a Potion, five Poké Balls) are made
//! straight to the core: they are the test's, not the Run's.

use std::path::PathBuf;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::Change;
use pactlocke_game::{Game, LevelCap, ProfileGame, ProfileStore, RomIdentity};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{
    BootPlan, FeedKind, InputChunk, MemoryStorage, NewRun, Ruleset, RunCommand, RunEngine, RunningGame, ShadowGame,
    ShadowState, StreamStore, SystemRunClock,
};

use super::GAME;

/// Which FireRed-like game a test target runs these on, and the few ROM
/// addresses the tests use that LeafGreen moves.
pub struct Frlg {
    /// The ROM's file name in the folder `PACTLOCKE_ROMS` names.
    pub file: &'static str,
    /// `gItems`.
    pub items: u32,
    /// What `gBattlerControllerFuncs[0]` holds while the Player picks FIGHT /
    /// BAG / POKéMON / RUN: the Player's and the
    /// Oak tutorial's `HandleInputChooseAction`.
    pub choose_action: [u32; 2],
    /// The lab rival's `trainerbattle`, one per starter (Squirtle's, Charmander's, Bulbasaur's).
    pub rival_battles: [u32; 3],
    /// Brock's `trainerbattle`.
    pub brock_battle: u32,
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

// FireRed (USA) 1.0, from pret's pokefirered; the
// same in LeafGreen (USA) 1.0, but for those in `Frlg`.
const SAVE_BLOCK_1_POINTER: u32 = 0x0300_5008;
const SAVE_BLOCK_2_POINTER: u32 = 0x0300_500C;
const BALLS_POCKET: u32 = 0x430; // in save block 1
const ITEMS_POCKET: u32 = 0x310;
const BAG_KEY: u32 = 0xF20; // in save block 2
const TEXT_SPEED: u32 = 0x14; // in save block 2
const IN_BATTLE: u32 = 0x0300_3529; // gMain.inBattle
const BATTLER_CONTROLLER: u32 = 0x0300_4FE0; // gBattlerControllerFuncs[0]
const CATCH_SITE: u32 = 0x0804_0F6C; // IsPlayerPartyAndPokemonStorageFull
const PARTY: u32 = 0x0202_4284; // gPlayerParty
const PARTY_COUNT: u32 = 0x0202_4029;
const PARTY_SLOT_SIZE: usize = 100;
const PARTY_HP: u32 = 0x56;
const BATTLE_MONS: u32 = 0x0202_3BE4; // gBattleMons[0]
const BATTLE_MON_HP: u32 = 0x28;
const PC_STORAGE_POINTER: u32 = 0x0300_5010;
const BOX_SLOT_SIZE: u32 = 80;
const ENEMY_PARTY: u32 = 0x0202_402C; // gEnemyParty
const PIDGEY: u16 = 16;
const RATTATA: u16 = 19;
const POKE_BALL: u16 = 4;
const POTION: u16 = 13;
const RARE_CANDY: u16 = 68;
const CAP_SITES: [u32; 4] = [0x0802_1CFA, 0x0802_1FB6, 0x0804_1834, 0x0804_274E];
const BADGES: u32 = 0xFE4; // in save block 1
const BATTLE_MON_SIZE: u32 = 0x58;
const BATTLE_MON_LEVEL: u32 = 0x2A;
const MAIN_CALLBACK2: u32 = 0x0300_30F4; // gMain.callback2
const BATTLE_STYLE: u32 = 0x15; // in save block 2, bit 1: Set
const ITEM_SIZE: usize = 44;
const ITEM_COUNT: usize = 375;
const ITEM_BATTLE_USAGE: usize = 32;

type Running = RunningGame<ProfileGame<MgbaCore>>;

struct Driver {
    game: Running,
    /// The Run as stored, shared with the engine.
    storage: MemoryStorage,
    rom: Vec<u8>,
    /// Another Player's app keeping a Shadow Game of this one, if any.
    watcher: Option<Watcher>,
}

/// About one GBA frame.
const FRAME_MS: i64 = 17;

/// Another Player's app: this Player's stream arrives after `lag_ms`, goes
/// through a stream store and is replayed by a Shadow Game on its own core.
struct Watcher {
    shadow: ShadowGame<ProfileGame<MgbaCore>>,
    outbox: std::sync::Arc<std::sync::Mutex<Vec<InputChunk>>>,
    store: StreamStore,
    in_flight: std::collections::VecDeque<(i64, InputChunk)>,
    now_ms: i64,
    lag_ms: i64,
    /// What the watching app flags of this stream, as it would put them in the Run log.
    watch: pactlocke_run::CheatWatch,
    log: pactlocke_run::RunLog,
    flags: Vec<pactlocke_run::RunEvent>,
}

impl Watcher {
    /// One frame's time: chunks are sent, those due land, and the Shadow runs.
    fn tick(&mut self) {
        self.now_ms += FRAME_MS;
        let sent: Vec<InputChunk> = std::mem::take(&mut *self.outbox.lock().unwrap());
        for chunk in sent {
            self.in_flight.push_back((self.now_ms + self.lag_ms, chunk));
        }
        while let Some((lands, _)) = self.in_flight.front()
            && *lands <= self.now_ms
        {
            let (_, chunk) = self.in_flight.pop_front().unwrap();
            self.arrives(chunk);
        }
        let frames = self.shadow.frames_for(FRAME_MS);
        let before = self.shadow.state();
        self.shadow.advance(self.now_ms, frames);
        if let ShadowState::Diverged { frame } = self.shadow.state()
            && before != self.shadow.state()
        {
            let at = pactlocke_run::GamePoint {
                boot: self.shadow.boot(),
                frame,
            };
            let flag = self.watch.diverged(&self.log, [1; 32], at, self.now_ms);
            self.flags.extend(flag);
        }
    }

    /// A chunk of the stream lands: replayed if new, flagged if it rewrites frames already here.
    fn arrives(&mut self, chunk: InputChunk) {
        match self.store.add(chunk) {
            pactlocke_run::Added::New(new) => {
                for chunk in new {
                    let flags = self.watch.arrived(&chunk, &self.log, self.now_ms);
                    self.flags.extend(flags);
                    self.shadow.receive(&chunk, self.now_ms);
                }
            }
            pactlocke_run::Added::Rewritten(rewrite) => {
                let flag = self.watch.rewritten(&self.log, &rewrite, self.now_ms);
                self.flags.extend(flag);
            }
            _ => {}
        }
    }

    fn cheats(&self) -> Vec<pactlocke_run::Cheat> {
        self.flags
            .iter()
            .filter_map(|f| match f {
                pactlocke_run::RunEvent::CheatFlag { cheat, .. } => Some(*cheat),
                _ => None,
            })
            .collect()
    }
}

/// The game clock a test Run starts with: 2026-01-01T00:00:00Z.
const CLOCK: i64 = 1_767_225_600_000;

impl Driver {
    fn frame(&mut self, keys: Keys) {
        self.game.run_frame(keys);
        if let Some(watcher) = self.watcher.as_mut() {
            watcher.tick();
        }
    }

    /// Another Player's app starts keeping a Shadow Game of this game, which
    /// must be fresh from power-on. Test setup writes are stamped from now on.
    fn watched(&mut self) {
        let mut core = MgbaCore::new();
        core.load_rom(&self.rom).unwrap();
        let store = ProfileStore::from_env().unwrap();
        let RomIdentity::Profiled { profile, .. } = store.identify(&self.rom) else {
            panic!("{} should be profiled", GAME.file);
        };
        let outbox = self.game.stream_as([1; 32]);
        self.watcher = Some(Watcher {
            shadow: ShadowGame::power_on(ProfileGame::new(profile, self.rom.clone(), core), CLOCK),
            outbox,
            store: StreamStore::new(),
            in_flight: Default::default(),
            now_ms: 0,
            lag_ms: 0,
            watch: pactlocke_run::CheatWatch::new(0),
            log: pactlocke_run::RunLog::new([0; 32]),
            flags: Vec::new(),
        });
    }

    fn watcher(&mut self) -> &mut Watcher {
        self.watcher.as_mut().expect("watched")
    }

    /// A test setup write. In a watched game it is stamped into the input
    /// stream like the Run's own changes, so the Shadow Game sees it too.
    fn write(&mut self, addr: u32, bytes: &[u8]) {
        if self.watcher.is_some() {
            self.game.apply_stamped(Change::Write {
                addr,
                bytes: bytes.to_vec(),
            });
        } else {
            self.game.write(addr, bytes);
        }
    }

    fn read(&mut self, addr: u32, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        self.game.read(addr, &mut out);
        out
    }

    fn u16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes(self.read(addr, 2).try_into().unwrap())
    }

    fn u32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.read(addr, 4).try_into().unwrap())
    }

    fn in_battle(&mut self) -> bool {
        self.read(IN_BATTLE, 1)[0] & 0x02 != 0
    }

    fn at_action_menu(&mut self) -> bool {
        GAME.choose_action.contains(&(self.u32(BATTLER_CONTROLLER) & !1))
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

    /// Mashes `key` until the Player's action menu is up in battle.
    #[track_caller]
    fn until_action_menu(&mut self, key: Keys, limit: u32) {
        let mut seen = std::collections::BTreeMap::new();
        for i in 0..limit {
            *seen
                .entry((self.in_battle(), self.u32(BATTLER_CONTROLLER)))
                .or_insert(0) += 1;
            if self.at_action_menu() {
                (0..20).for_each(|_| self.frame(Keys::NONE));
                return;
            }
            self.frame(if i % 16 < 3 { key } else { Keys::NONE });
        }
        dump_screen(&self.game, "no-action-menu");
        panic!("the action menu never came up: {seen:x?}");
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
        panic!("no wild battle in {limit} frames");
    }

    /// Picks RUN from the action menu until the Player gets away.
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
        let block = self.u32(SAVE_BLOCK_2_POINTER);
        self.u16(block + BAG_KEY)
    }

    /// Puts `quantity` of `item` in a pocket (test setup, outside the Run).
    fn give(&mut self, pocket: u32, item: u16, quantity: u16) {
        let slot = self.u32(SAVE_BLOCK_1_POINTER) + pocket;
        let key = self.bag_key();
        self.write(slot, &item.to_le_bytes());
        self.write(slot + 2, &(quantity ^ key).to_le_bytes());
    }

    fn quantity(&mut self, pocket: u32, item: u16) -> u16 {
        let base = self.u32(SAVE_BLOCK_1_POINTER) + pocket;
        let key = self.bag_key();
        (0..13)
            .map(|i| base + i * 4)
            .find(|slot| self.u16(*slot) == item)
            .map_or(0, |slot| self.u16(slot + 2) ^ key)
    }

    fn fast_text(&mut self) {
        let addr = self.u32(SAVE_BLOCK_2_POINTER) + TEXT_SPEED;
        let speed = self.read(addr, 1)[0];
        self.write(addr, &[(speed & !7) | 2]);
    }

    fn feed(&self, kind: FeedKind) -> Vec<String> {
        let view = self.game.engine().view();
        view.feed
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.text.clone())
            .collect()
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

/// From power-on to choosing a starter in Oak's lab.
const FR_STARTER: &str = "m A 5100\nm B 300\nh R 64\nh U 64\nh L 24\nw 120\nh D 16\nh L 16\nh D 96\nh L 80\nh D 24\nw 120\n\
h R 64\nh U 96\nh R 32\nh U 48\nm A 1500\nm B 1500\nh R 20\nw 20\np A 90\np A 90\np A 90\np A 120\np A 120\np A 120\nm B 1500";

/// From the lab after the rival battle, out of Pallet Town into Route 1's grass.
const FR_TO_ROUTE_1: &str =
    "m B 900\nh D 96\nh L 16\nh D 24\nw 120\nh L 96\nh U 160\nh R 32\nh U 200\nm B 200\nh U 120";

fn new_run(rom: Vec<u8>) -> Driver {
    run_with(
        rom,
        Ruleset {
            first_encounter: true,
            ..Ruleset::default()
        },
    )
}

fn run_with(rom: Vec<u8>, ruleset: Ruleset) -> Driver {
    pactlocke_mgba::force_utc_time_zone();
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("{} should be profiled", GAME.file);
    };
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
    booted(engine, storage, rom)
}

/// The Run's game booted the way its engine planned, on a fresh core.
fn booted(engine: RunEngine, storage: MemoryStorage, rom: Vec<u8>) -> Driver {
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("{} should be profiled", GAME.file);
    };
    booted_on(profile, engine, storage, rom)
}

/// The same on this Game Profile.
fn booted_on(
    profile: std::sync::Arc<pactlocke_game::GameProfile>,
    engine: RunEngine,
    storage: MemoryStorage,
    rom: Vec<u8>,
) -> Driver {
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    let game = ProfileGame::new(profile, rom.clone(), core);
    Driver {
        game: RunningGame::boot(game, engine, CLOCK).unwrap(),
        storage,
        rom,
        watcher: None,
    }
}

/// The same Run resumed from `storage`, as the app does when it is opened again.
fn resumed(storage: MemoryStorage, rom: Vec<u8>) -> Driver {
    let engine = RunEngine::resume(Box::new(storage.clone()), Box::new(SystemRunClock)).unwrap();
    booted(engine, storage, rom)
}

/// How many sites the Game Profile's catch block patches.
fn catch_block_sites(rom: &[u8]) -> usize {
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(rom) else {
        panic!("{} should be profiled", GAME.file);
    };
    match &profile.rules.catch_block {
        Some(pactlocke_game::Method::Patch { sites }) => sites.len(),
        other => panic!("the catch block is a patch: {other:?}"),
    }
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
fn a_closed_route_1_refuses_the_ball_and_keeps_it_and_running_still_works() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = new_run(rom);
    let clean_catch_site = [0x10, 0xB5, 0x00, 0x24];

    // The starter, then the rival battle in the lab: a trainer battle, nothing for the rules.
    d.play(FR_STARTER);
    d.fast_text();
    d.give(ITEMS_POCKET, POTION, 1);
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    d.until_battle_over(Keys::A, 4000);
    assert!(
        d.feed(FeedKind::Encounter).is_empty(),
        "the rival battle isn't an Encounter"
    );

    // Route 1's first wild battle with Poké Balls is its Encounter; catching stays on.
    d.play(FR_TO_ROUTE_1);
    assert_eq!(d.game.game_mut().area().map(|a| a.name), Some("Route 1".into()));
    // Oak hasn't handed over Poké Balls yet, so a wild battle here doesn't count.
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let not_counted = d.feed(FeedKind::Encounter);
    assert_eq!(not_counted.len(), 1, "{not_counted:?}");
    assert!(
        not_counted[0].ends_with("not counted: no Poké Balls yet"),
        "{not_counted:?}"
    );
    d.run_away();

    // With Poké Balls in the Bag, Encounters count from the next wild battle.
    d.give(BALLS_POCKET, POKE_BALL, 5);
    d.play("w 180");
    assert!(
        d.feed(FeedKind::Run)
            .iter()
            .any(|e| e.starts_with("You have Poké Balls"))
    );
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let encounters = d.feed(FeedKind::Encounter);
    assert_eq!(encounters.len(), 2, "{encounters:?}");
    assert!(
        ["Route 1: a wild PIDGEY,", "Route 1: a wild RATTATA,"]
            .iter()
            .any(|start| encounters[0].starts_with(start)),
        "one of Route 1's own Pokémon, named from the ROM: {encounters:?}"
    );
    assert_eq!(
        d.read(CATCH_SITE, 4),
        clean_catch_site,
        "no block in the Encounter battle"
    );

    // Running is always allowed.
    d.run_away();
    assert!(!d.in_battle());

    // The next wild battle on Route 1: the block is patched on as it starts.
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let blocks = d.feed(FeedKind::Block);
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert!(
        blocks[0].contains("PIDGEY") || blocks[0].contains("RATTATA"),
        "{blocks:?}"
    );
    assert_eq!(d.read(CATCH_SITE, 4), [0x01, 0x20, 0x70, 0x47], "catch block on");
    // FireRed's two catch block sites; LeafGreen's four, with the Safari line's.
    let sites = catch_block_sites(&d.rom);
    let stamped = d.game.engine().stamped().to_vec();
    assert_eq!(
        stamped.len(),
        sites,
        "every catch block site, at one frame boundary: {stamped:?}"
    );
    assert!(stamped.iter().all(|s| s.frame == stamped[0].frame));
    let party_before = d.game.game_mut().party();

    // BAG, over to the Poké Balls pocket, the ball, USE: refused, and the ball is kept.
    d.play("p R 10\np A 60\np R 30\np R 30\np A 40\np A 120");
    assert!(d.in_battle(), "the ball wasn't thrown");
    dump_screen(&d.game, "ball-refused");
    d.play("m B 200");
    assert_eq!(d.quantity(BALLS_POCKET, POKE_BALL), 5, "the ball is kept");
    assert!(d.in_battle());

    // And running away from the closed Area works; the block comes off with the battle.
    d.run_away();
    assert!(!d.in_battle());
    d.play("w 180");
    assert_eq!(
        d.read(CATCH_SITE, 4),
        clean_catch_site,
        "catch block off after the battle"
    );
    assert_eq!(d.game.game_mut().party(), party_before, "nothing was caught");
    assert_eq!(d.game.engine().stamped().len(), 2 * sites, "on and off, every site");
    assert!(d.feed(FeedKind::Catch).is_empty(), "no catch in the feed");
    for entry in d.game.engine().view().feed.iter().rev() {
        eprintln!("feed: {}", entry.text);
    }
}

/// The starter, the party's only Pokémon, faints to the rival.
/// Under Faint = Dead that wipes the party: the game over comes at that
/// Death, the game stops there, and it never runs on with an empty party
/// (the playtest's softlock: the nurse waiting for a party of none).
#[test]
fn the_party_s_last_pokemon_dying_to_the_rival_is_a_game_over_and_the_party_is_never_empty() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            faint_is_dead: true,
            ..Ruleset::default()
        },
    );

    // The rival battle in the lab, with the starter left on 1 HP (test setup).
    d.play(FR_STARTER);
    d.fast_text();
    d.play(
        "h D 64
w 60",
    );
    d.until_action_menu(Keys::A, 3000);
    d.game.write(PARTY + PARTY_HP, &1u16.to_le_bytes());
    d.game.write(BATTLE_MONS + BATTLE_MON_HP, &1u16.to_le_bytes());
    let starter = d.game.game_mut().party()[0].clone();
    for i in 0..6000 {
        if d.game.engine().view().game_over.is_some() {
            break;
        }
        d.frame(if i % 16 < 3 { Keys::A } else { Keys::NONE });
    }

    let over = d
        .game
        .engine()
        .view()
        .game_over
        .clone()
        .expect("a game over at the Death");
    assert!(
        over.how.starts_with("Your party was wiped in ") && over.how.contains("fainted against a trainer's"),
        "{}",
        over.how
    );
    assert_eq!(
        over.longest.len(),
        1,
        "the starter survived longest: {:?}",
        over.longest
    );
    let graveyard = d.game.engine().view().graveyard.clone();
    assert_eq!(graveyard.len(), 1);
    assert_eq!(graveyard[0].pid, starter.pid);

    // The game goes no further: nothing it does can reach an empty party.
    let stopped_at = d.game.engine().frames_run();
    d.play(
        "m A 600
m B 600",
    );
    assert_eq!(d.game.engine().frames_run(), stopped_at, "the game has stopped");
    assert_eq!(d.read(PARTY_COUNT, 1)[0], 1, "the starter is still in the party");
    assert!(
        d.feed(FeedKind::Death).iter().all(|e| !e.starts_with("Moved")),
        "nothing was moved to the Dead box: {:?}",
        d.feed(FeedKind::Death)
    );
    dump_screen(&d.game, "game-over");
}

#[test]
fn a_route_1_pokemon_from_a_family_already_owned_is_skipped_as_a_dupe() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            dupes_clause: true,
            ..Ruleset::default()
        },
    );
    let clean_catch_site = [0x10, 0xB5, 0x00, 0x24];

    d.play(FR_STARTER);
    d.fast_text();
    d.give(ITEMS_POCKET, POTION, 1);
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    d.until_battle_over(Keys::A, 4000);
    d.play(FR_TO_ROUTE_1);

    // Before Poké Balls nothing counts. Each wild Pokémon met is put in the
    // PC as if caught earlier (test setup), until the Player has both of
    // Route 1's species.
    let mut owned = Vec::new();
    for _ in 0..20 {
        if owned.contains(&PIDGEY) && owned.contains(&RATTATA) {
            break;
        }
        d.walk_in_grass(6000);
        d.until_action_menu(Keys::B, 3000);
        let species = d.game.game_mut().opponent_party()[0].species;
        let wild = d.read(ENEMY_PARTY, BOX_SLOT_SIZE as usize);
        d.run_away();
        // After the battle, so it isn't taken for a catch.
        if !owned.contains(&species) {
            let slot = d.u32(PC_STORAGE_POINTER) + 4 + owned.len() as u32 * BOX_SLOT_SIZE;
            d.game.write(slot, &wild);
            owned.push(species);
        }
    }
    owned.sort_unstable();
    assert_eq!(owned, [PIDGEY, RATTATA], "met both on Route 1");

    // With Poké Balls, Route 1's next wild battle is a dupe either way.
    d.give(BALLS_POCKET, POKE_BALL, 5);
    d.play("w 180");
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let encounters = d.feed(FeedKind::Encounter);
    assert!(
        encounters[0].starts_with("Route 1: a wild ") && encounters[0].contains(", not counted: a dupe of your "),
        "{encounters:?}"
    );
    assert_eq!(d.read(CATCH_SITE, 4), [0x01, 0x20, 0x70, 0x47], "no catching a dupe");
    assert!(
        d.game.engine().view().closed_areas().is_empty(),
        "Route 1 is still open"
    );
    assert!(
        d.feed(FeedKind::Catch).is_empty(),
        "the PC was filled by the test, not by catches"
    );

    d.run_away();
    d.play("w 180");
    assert_eq!(
        d.read(CATCH_SITE, 4),
        clean_catch_site,
        "the block comes off with the battle"
    );
    for entry in d.game.engine().view().feed.iter().rev() {
        eprintln!("feed: {}", entry.text);
    }
}

#[test]
fn the_real_starter_is_the_starter_gift_and_kept_with_gifts_off() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = run_with(
        rom,
        Ruleset {
            first_encounter: true,
            gifts: false,
            ..Ruleset::default()
        },
    );
    d.play(FR_STARTER);
    d.play("w 120");

    let gifts = d.feed(FeedKind::Gift);
    assert_eq!(gifts.len(), 1, "{gifts:?}");
    assert!(
        ["BULBASAUR", "CHARMANDER", "SQUIRTLE"]
            .iter()
            .any(|starter| gifts[0] == format!("Pallet Town: received {starter}, the Starter")),
        "{gifts:?}"
    );
    assert_eq!(d.game.game_mut().party().len(), 1, "the starter is always kept");
}

/// Moves the START menu's cursor, which the game remembers, from POKéMON to BAG.
const START_MENU_ON_BAG: &str = "p S 60
p D 20
p B 60";

/// START, BAG, the first item of the Items pocket, USE, on the first party
/// Pokémon, then back out to the overworld.
const USE_FIRST_ITEM: &str = "p S 60
p A 120
p A 40
p A 120
p A 180
m B 600";

#[test]
fn exp_stops_at_the_cap_and_rare_candy_works_only_below_it() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let cap = |name: &str, level| LevelCap {
        name: name.into(),
        level,
    };
    let mut d = run_with(
        rom,
        Ruleset {
            level_caps: Some(vec![cap("Brock", 7), cap("Misty", 8)]),
            ..Ruleset::default()
        },
    );
    d.play(FR_STARTER);
    assert_eq!(d.game.engine().view().level_cap.as_ref().map(|c| c.level), Some(7));
    for site in CAP_SITES {
        assert_eq!(d.read(site, 2), [7, 0x28], "cap 7 at {site:#010X}");
    }

    // The rival battle in the lab, against a level-50 foe on 1 HP (test
    // setup): a win worth many levels from level 5.
    d.fast_text();
    d.play(
        "h D 64
w 60",
    );
    d.until_action_menu(Keys::A, 3000);
    assert_eq!(d.game.game_mut().party()[0].stats.unwrap().level, 5);
    let foe = BATTLE_MONS + BATTLE_MON_SIZE;
    d.game.write(foe + BATTLE_MON_LEVEL, &[50]);
    d.game.write(foe + BATTLE_MON_HP, &1u16.to_le_bytes());
    d.until_battle_over(Keys::A, 8000);
    d.play("m B 900");
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

    // A badge (test setup) raises the cap by a patch stamped at that frame.
    let badges = d.u32(SAVE_BLOCK_1_POINTER) + BADGES;
    d.game.write(badges, &[1]);
    d.play("w 30");
    for site in CAP_SITES {
        assert_eq!(d.read(site, 2), [8, 0x28], "cap 8 at {site:#010X}");
    }
    assert_eq!(d.feed(FeedKind::Run)[0], "Level cap raised to 8 until Misty");

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
    for entry in d.game.engine().view().feed.iter().rev() {
        eprintln!("feed: {}", entry.text);
    }
}

/// Into the lab rival battle with a second Pokémon on each side (test
/// setup: copies of the first, the rival's on full HP), up to the action menu.
fn rival_battle_with_two_pokemon_each(d: &mut Driver) {
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    let rivals = d.read(ENEMY_PARTY, PARTY_SLOT_SIZE);
    d.game.write(ENEMY_PARTY + PARTY_SLOT_SIZE as u32, &rivals);
    let starter = d.read(PARTY, PARTY_SLOT_SIZE);
    d.game.write(PARTY + PARTY_SLOT_SIZE as u32, &starter);
    d.game.write(PARTY_COUNT, &[2]);
}

/// A second Pokémon in the party, so a Death isn't a wiped party (test
/// setup): a copy of the starter under a PID of its own. Its data is
/// encrypted with PID xor OT ID and ordered by PID mod 24, so the PID moves
/// by 24 and the OT ID by the same xor, and the copy decrypts exactly as
/// the starter does, checksum and all.
fn second_pokemon_in_the_party(d: &mut Driver) {
    let mut copy = d.read(PARTY, PARTY_SLOT_SIZE);
    let pid = u32::from_le_bytes(copy[0..4].try_into().unwrap());
    let ot_id = u32::from_le_bytes(copy[4..8].try_into().unwrap());
    let new_pid = pid.wrapping_add(24);
    copy[0..4].copy_from_slice(&new_pid.to_le_bytes());
    copy[4..8].copy_from_slice(&(ot_id ^ pid ^ new_pid).to_le_bytes());
    d.write(PARTY + PARTY_SLOT_SIZE as u32, &copy);
    d.write(PARTY_COUNT, &[2]);
}

/// Leaves the rival's Pokémon out in battle on 1 HP (test setup).
fn rivals_pokemon_on_1_hp(d: &mut Driver) {
    d.write(BATTLE_MONS + BATTLE_MON_SIZE + BATTLE_MON_HP, &1u16.to_le_bytes());
}

/// Whether the battle style option is on Set.
fn battle_style_is_set(d: &mut Driver) -> bool {
    let options = d.u32(SAVE_BLOCK_2_POINTER) + BATTLE_STYLE;
    d.read(options, 1)[0] & 0x02 != 0
}

/// Whether the game's text is anywhere in EWRAM, where it builds the lines it shows.
fn shows_text(d: &mut Driver, encoded: &[u8]) -> bool {
    d.read(0x0200_0000, 0x4_0000)
        .windows(encoded.len())
        .any(|w| w == encoded)
}

/// FIGHT with the first move against a rival's Pokémon left on 1 HP, then A
/// through what follows. Says whether the game offered to change Pokémon
/// (Shift mode's "Will … change POKéMON?", where A opens the party menu)
/// before the rival's second Pokémon came out.
fn knock_out_the_first_and_see_if_shift_is_offered(d: &mut Driver) -> bool {
    rivals_pokemon_on_1_hp(d);
    let battle_screen = d.u32(MAIN_CALLBACK2);
    d.play("p A 30\np A 60");
    for i in 0..6000 {
        if d.u32(MAIN_CALLBACK2) != battle_screen {
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

#[test]
fn set_mode_holds_in_a_trainer_battle_and_the_bag_is_refused_there() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = run_with(
        rom,
        Ruleset {
            no_items: true,
            set_mode: true,
            ..Ruleset::default()
        },
    );
    d.play(FR_STARTER);
    assert!(battle_style_is_set(&mut d), "the battle style is Set");
    let run = d.feed(FeedKind::Run);
    assert!(
        run.contains(&"Set mode: the battle style is set to Set".to_owned()),
        "{run:?}"
    );
    assert!(run.iter().any(|e| e.starts_with("No items in battle")), "{run:?}");
    d.fast_text();
    d.give(ITEMS_POCKET, POTION, 1);

    // BAG in a trainer battle: "Items can't be used now", and the Bag never opens.
    rival_battle_with_two_pokemon_each(&mut d);
    let battle_screen = d.u32(MAIN_CALLBACK2);
    // "Items can't" in the game's characters.
    const ITEMS_CANT: [u8; 11] = [0xC3, 0xE8, 0xD9, 0xE1, 0xE7, 0x00, 0xD7, 0xD5, 0xE2, 0xB4, 0xE8];
    assert!(!shows_text(&mut d, &ITEMS_CANT), "not before BAG");
    d.play("p R 10\np A 40");
    dump_screen(&d.game, "bag-refused");
    assert!(
        shows_text(&mut d, &ITEMS_CANT),
        "the game's own \"Items can't be used now.\""
    );
    for _ in 0..120 {
        d.frame(Keys::NONE);
        assert_eq!(d.u32(MAIN_CALLBACK2), battle_screen, "the Bag opened");
    }
    d.until_action_menu(Keys::B, 3000);
    d.play("p L 10");

    // The rival's next Pokémon comes out with no offer to change.
    assert!(
        !knock_out_the_first_and_see_if_shift_is_offered(&mut d),
        "Set mode: no offer to change Pokémon"
    );
    rivals_pokemon_on_1_hp(&mut d);
    d.until_battle_over(Keys::A, 6000);
    for entry in d.game.engine().view().feed.iter().rev() {
        eprintln!("feed: {}", entry.text);
    }
}

#[test]
fn control_without_set_mode_the_same_battle_offers_to_change_pokemon() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = run_with(rom, Ruleset::default());
    d.play(FR_STARTER);
    assert!(!battle_style_is_set(&mut d), "a new game's battle style is Shift");
    d.fast_text();
    rival_battle_with_two_pokemon_each(&mut d);
    assert!(knock_out_the_first_and_see_if_shift_is_offered(&mut d));
}

#[test]
fn with_no_items_only_balls_work_in_a_wild_battle_and_items_work_outside_battle() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let clean_items = {
        let at = (GAME.items - 0x0800_0000) as usize;
        rom[at..at + ITEM_COUNT * ITEM_SIZE].to_vec()
    };
    let mut d = run_with(
        rom,
        Ruleset {
            no_items: true,
            ..Ruleset::default()
        },
    );
    d.play(FR_STARTER);
    d.fast_text();
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    rivals_pokemon_on_1_hp(&mut d);
    d.until_battle_over(Keys::A, 6000);
    d.play(FR_TO_ROUTE_1);

    // Only battle use changed in the item table: held effects, field use
    // and everything else are as the ROM has them.
    let patched = d.read(GAME.items, ITEM_COUNT * ITEM_SIZE);
    for (i, (now, clean)) in patched.iter().zip(&clean_items).enumerate() {
        if now != clean {
            assert_eq!(
                i % ITEM_SIZE,
                ITEM_BATTLE_USAGE,
                "item {} byte {:#x} changed",
                i / ITEM_SIZE,
                i % ITEM_SIZE
            );
        }
    }
    assert_ne!(patched, clean_items, "battle use is banned");

    // A Potion works outside battle.
    d.game.write(PARTY + PARTY_HP, &5u16.to_le_bytes());
    d.give(ITEMS_POCKET, POTION, 2);
    // Once the walk has stopped, or START is missed.
    d.play("w 60");
    d.play(START_MENU_ON_BAG);
    d.play(USE_FIRST_ITEM);
    assert!(d.u16(PARTY + PARTY_HP) > 5, "the Potion healed");
    assert_eq!(d.quantity(ITEMS_POCKET, POTION), 1);

    // In a wild battle the Potion has no USE, so nothing is used...
    d.give(BALLS_POCKET, POKE_BALL, 5);
    d.game.write(PARTY + PARTY_HP, &5u16.to_le_bytes());
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    let hp = d.u16(BATTLE_MONS + BATTLE_MON_HP);
    d.play("p R 10\np A 60\np A 40");
    dump_screen(&d.game, "potion-in-wild-battle");
    d.play("p A 40\np B 60");
    d.until_action_menu(Keys::B, 3000);
    assert_eq!(d.quantity(ITEMS_POCKET, POTION), 1, "the Potion is kept");
    assert_eq!(d.u16(BATTLE_MONS + BATTLE_MON_HP), hp, "and nothing healed");

    // ...while a Poké Ball still throws.
    d.play("p A 60\np R 30\np R 30\np A 40\np A 120");
    assert_eq!(d.quantity(BALLS_POCKET, POKE_BALL), 4, "the ball was thrown");
}

/// YES on "Give a nickname?", then the naming screen's OK with the name left
/// blank, which the game keeps as the species name.
const NAMING_LEFT_BLANK: &str = "p A 120\np S 30\np A 180";

/// BAG, the Poké Balls pocket, the first ball, USE: from FIGHT, then again
/// from BAG after a ball the wild Pokémon broke out of.
const THROW_FIRST_BALL: &str = "p R 10\np A 60\np R 30\np R 30\np A 40\np A 120";
const THROW_AGAIN: &str = "p A 60\np A 40\np A 120";

/// Throws balls at a wild Pokémon on 1 HP (test setup) until it is caught,
/// then mashes B until "Give a nickname?" is up.
#[track_caller]
fn catch_until_the_nickname_prompt(d: &mut Driver) {
    let wild = BATTLE_MONS + BATTLE_MON_SIZE + BATTLE_MON_HP;
    d.game.write(wild, &1u16.to_le_bytes());
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
        d.game.write(wild, &1u16.to_le_bytes());
        d.play(THROW_AGAIN);
    }
    panic!("the wild Pokémon was never caught");
}

/// Names the one Pokémon the side panel asks about, as the side panel does.
fn name_from_the_side_panel(d: &mut Driver, name: &str) -> u32 {
    let wanted = d.game.engine().view().nickname_wanted.clone();
    assert_eq!(wanted.len(), 1, "{wanted:?}");
    d.game
        .commands()
        .send(RunCommand::Nickname {
            pid: wanted[0].pid,
            name: name.into(),
        })
        .unwrap();
    d.play("w 2");
    wanted[0].pid
}

#[test]
fn must_nickname_names_the_starter_and_a_catch_and_the_type_clause_boxes_a_second_normal() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = run_with(
        rom,
        Ruleset {
            must_nickname: true,
            type_clause: true,
            ..Ruleset::default()
        },
    );

    // The starter's "Do you want to give a nickname…?" outlasts the
    // script's 1500 frames of B, which decline it in a game without the rule.
    d.play(FR_STARTER);
    assert!(
        d.game.game_mut().nickname_prompt_up(),
        "B can't decline the starter's nickname"
    );
    dump_screen(&d.game, "starter-nickname-prompt");
    d.play(NAMING_LEFT_BLANK);
    d.play("m B 1500");
    let starter = d.game.game_mut().party()[0].clone();
    assert_eq!(
        starter.nickname,
        d.game.game_mut().species_name(starter.species),
        "left blank"
    );
    let pid = name_from_the_side_panel(&mut d, "NIBBLES");
    assert_eq!(pid, starter.pid);
    let named = d.game.game_mut().party()[0].clone();
    assert_eq!(named.nickname, "NIBBLES");
    assert_eq!(
        named.species, starter.species,
        "no Bad Egg: the encrypted data is untouched"
    );
    assert!(d.game.engine().view().nickname_wanted.is_empty());

    // The rival, then Route 1 with Poké Balls.
    d.fast_text();
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    rivals_pokemon_on_1_hp(&mut d);
    d.until_battle_over(Keys::A, 6000);
    d.play(FR_TO_ROUTE_1);
    d.give(BALLS_POCKET, POKE_BALL, 20);

    // A catch: the battle's "Give a nickname?" can't be declined either.
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    catch_until_the_nickname_prompt(&mut d);
    d.play("m B 600");
    assert!(
        d.game.game_mut().nickname_prompt_up(),
        "B can't decline the catch's nickname"
    );
    dump_screen(&d.game, "catch-nickname-prompt");
    d.play(NAMING_LEFT_BLANK);
    d.until_battle_over(Keys::B, 3000);
    d.play("w 60");
    let caught = name_from_the_side_panel(&mut d, "BIRDY");
    let party = d.game.game_mut().party();
    assert_eq!(party.len(), 2, "a Normal type joins a party whose only type is GRASS");
    assert_eq!((party[1].pid, party[1].nickname.as_str()), (caught, "BIRDY"));

    // A second Normal type (Route 1 has only PIDGEY and RATTATA) goes to box 1.
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    catch_until_the_nickname_prompt(&mut d);
    d.play(NAMING_LEFT_BLANK);
    d.until_battle_over(Keys::B, 3000);
    d.play("w 60");
    assert_eq!(
        d.game.game_mut().party().len(),
        2,
        "the second Normal type left the party"
    );
    let blocks = d.feed(FeedKind::Block);
    assert!(
        blocks[0].starts_with("Type clause: your BIRDY is the NORMAL type in your party, so ")
            && blocks[0].ends_with(" went to box 1"),
        "{blocks:?}"
    );
    // And it is named where it is, in the PC.
    let boxed = name_from_the_side_panel(&mut d, "SECOND");
    let pc = d.game.game_mut().pc();
    let stored = pc.iter().find(|s| s.pokemon.pid == boxed).expect("in the PC");
    assert_eq!((stored.box_number, stored.pokemon.nickname.as_str()), (1, "SECOND"));
    for entry in d.game.engine().view().feed.iter().rev() {
        eprintln!("feed: {}", entry.text);
    }
}

/// Route 1 with Poké Balls and its Encounter used: the next wild battle
/// there has the catch block on, up to the action menu.
fn to_a_closed_route_1_battle(d: &mut Driver) {
    d.play(FR_STARTER);
    d.fast_text();
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    rivals_pokemon_on_1_hp(d);
    d.until_battle_over(Keys::A, 6000);
    d.play(FR_TO_ROUTE_1);
    d.give(BALLS_POCKET, POKE_BALL, 5);
    d.play("w 120");
    // The test's own writes above aren't in the input stream, so a rebuild
    // starts after them. From here the game moves only by presses.
    d.game.checkpoint();
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    d.run_away();
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    assert_eq!(d.read(CATCH_SITE, 4), [0x01, 0x20, 0x70, 0x47], "catch block on");
}

#[test]
fn after_a_crash_mid_battle_the_game_is_rebuilt_from_its_checkpoint_and_matches_by_hash() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = new_run(rom);
    to_a_closed_route_1_battle(&mut d);
    let play_time = d.game.game_mut().play_time();
    assert!(play_time > 60 * 60, "the play time runs: {play_time} frames");

    // The power goes out: the Run file is as it was at this frame.
    d.game.flush();
    let crashed = d.storage.snapshot();
    let at_crash = ram_hash(&mut d);
    let frames = d.game.engine().frames_run();
    let rom = d.rom.clone();
    drop(d);

    let mut d = resumed(crashed, rom);
    assert!(
        matches!(d.game.engine().boot_plan(), BootPlan::Rebuild { from, keys, .. } if from.frame + keys.len() as u64 == frames),
        "rebuilt from the boot's last checkpoint with every press since"
    );
    assert_eq!(
        ram_hash(&mut d),
        at_crash,
        "the rebuilt game matches the one that crashed"
    );
    assert_eq!(d.read(CATCH_SITE, 4), [0x01, 0x20, 0x70, 0x47], "with its patch set");
    assert!(d.in_battle());
    let run = d.feed(FeedKind::Run);
    assert!(
        run[0].starts_with("Run recovered after the app closed unexpectedly"),
        "{run:?}"
    );

    // And it plays on: running away lifts the block as usual.
    d.run_away();
    d.play("w 180");
    assert_eq!(d.read(CATCH_SITE, 4), [0x10, 0xB5, 0x00, 0x24]);
}

#[test]
fn suspend_and_quit_mid_battle_resumes_exactly_there() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = new_run(rom);
    to_a_closed_route_1_battle(&mut d);
    let at_suspend = ram_hash(&mut d);
    let (storage, rom) = (d.storage.clone(), d.rom.clone());
    d.game.suspend();

    let mut d = resumed(storage.clone(), rom.clone());
    assert!(matches!(d.game.engine().boot_plan(), BootPlan::Suspended(_)));
    assert_eq!(ram_hash(&mut d), at_suspend);
    assert_eq!(
        d.read(CATCH_SITE, 4),
        [0x01, 0x20, 0x70, 0x47],
        "the patch set is restored too"
    );
    d.run_away();
    drop(d);

    // The slot is used up: next time, the game boots from its last in-game save.
    let d = resumed(storage, rom);
    assert!(!matches!(d.game.engine().boot_plan(), BootPlan::Suspended(_)));
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
    let path = dir.join(format!("{name}.ppm"));
    std::fs::write(&path, ppm).unwrap();
    eprintln!("screen: {}", path.display());
}

#[test]
fn a_shadow_game_of_a_player_stays_matched_through_route_1_and_a_network_hiccup() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = new_run(rom);
    d.watched();

    // From power-on through the starter, the rival battle and Route 1's
    // Encounter to a closed Route 1 battle with the catch block patched on,
    // with the stream held up for two seconds on the way.
    d.play(FR_STARTER);
    d.watcher().lag_ms = 2000;
    d.play("w 180");
    d.watcher().lag_ms = 0;
    to_a_closed_route_1_battle_from_the_lab(&mut d);
    d.run_away();
    d.play("w 300");

    let shadow = &d.watcher.as_ref().unwrap().shadow;
    assert_eq!(shadow.state(), ShadowState::Matched);
    assert!(shadow.rewinds() >= 1, "the hiccup made it guess, then rewind");
    let checked = shadow.hashes_matched();
    let frames = d.game.engine().frames_run();
    assert!(
        checked >= frames / pactlocke_run::HASH_EVERY - 2,
        "{checked} hashes matched over {frames} frames"
    );
    let stamped = d.game.engine().stamped();
    assert!(
        stamped
            .iter()
            .any(|s| matches!(&s.change, Change::Patch { addr, .. } if *addr == CATCH_SITE)),
        "the catch block went through the stream"
    );
}

#[test]
fn a_memory_edit_makes_its_shadow_game_diverge() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = new_run(rom);
    d.watched();
    d.play("m A 1200");
    // Frames the watcher has, sent again pressed differently, as after a
    // savestate load: refused and flagged, and the Shadow plays on matched.
    let have = d.watcher().store.received(&[1; 32]).unwrap().1;
    let mut forged = d.watcher().store.wanted(&pactlocke_run::StreamWant {
        player: [1; 32],
        boot: 1,
        from_frame: have - 60,
    });
    forged[0].keys.fill(Keys::B);
    d.watcher().arrives(forged.remove(0));
    assert_eq!(d.watcher().cheats(), [pactlocke_run::Cheat::HistoryRewritten]);
    d.play("m A 300");
    assert_eq!(d.watcher().shadow.state(), ShadowState::Matched);

    // A cheat tool's edit: straight into the core, not stamped.
    d.game.write(PARTY_COUNT, &[6]);
    d.play("m A 600");
    assert!(
        matches!(d.watcher().shadow.state(), ShadowState::Diverged { .. }),
        "{:?}",
        d.watcher().shadow.state()
    );
    assert_eq!(
        d.watcher().cheats(),
        [pactlocke_run::Cheat::HistoryRewritten, pactlocke_run::Cheat::OutOfSync],
        "flagged as game out of sync, and nothing for playing at 1x"
    );
}

#[test]
fn a_run_checkpoint_changed_in_the_run_file_is_flagged_and_not_gone_back_to() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let ruleset = Ruleset {
        first_encounter: true,
        retries: 1,
        ..Ruleset::default()
    };
    let mut d = run_with(rom, ruleset);
    d.play(FR_STARTER);
    d.play("w 60");
    d.game.checkpoint();
    d.play("w 120");
    let (storage, rom) = (d.storage.clone(), d.rom.clone());
    d.game.suspend();

    // Someone changes the Run Checkpoint in the Run file while the app is closed.
    storage.tamper_with_checkpoints(|c| {
        if c.reason != pactlocke_run::CheckpointReason::Boot {
            c.core.save_memory[0] ^= 0xFF;
        }
    });
    let mut d = resumed(storage, rom);
    let before = ram_hash(&mut d);
    let refused = d.game.retry(&pactlocke_run::RetryTo::Latest).unwrap_err();
    assert!(refused.contains("doesn't match its hash in the log"), "{refused}");
    assert_eq!(ram_hash(&mut d), before, "the game wasn't touched");
    assert_eq!(d.game.engine().retries_left(), 1);
    let flags = d.feed(FeedKind::Cheat);
    assert_eq!(flags.len(), 1, "{flags:?}");
    assert!(
        flags[0].ends_with("a Run Checkpoint in the Run file doesn't match its hash in the log"),
        "{flags:?}"
    );
}

/// After the starter is chosen: the rival battle, then Route 1 to a closed battle there.
fn to_a_closed_route_1_battle_from_the_lab(d: &mut Driver) {
    d.fast_text();
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    rivals_pokemon_on_1_hp(d);
    d.until_battle_over(Keys::A, 6000);
    d.play(FR_TO_ROUTE_1);
    d.give(BALLS_POCKET, POKE_BALL, 5);
    d.play("w 120");
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    d.run_away();
    d.walk_in_grass(6000);
    d.until_action_menu(Keys::B, 3000);
    assert_eq!(d.read(CATCH_SITE, 4), [0x01, 0x20, 0x70, 0x47], "catch block on");
}

#[test]
fn after_a_retry_the_game_is_back_at_its_run_checkpoint_and_its_shadow_follows() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let ruleset = Ruleset {
        first_encounter: true,
        retries: 1,
        ..Ruleset::default()
    };
    let mut d = run_with(rom, ruleset);
    d.watched();
    d.play(FR_STARTER);
    d.play("w 60");
    // A Run Checkpoint (as at a Sync Gate), then on to a closed Route 1 battle.
    d.game.checkpoint();
    let at_checkpoint = ram_hash(&mut d);
    to_a_closed_route_1_battle_from_the_lab(&mut d);
    let matched_before = d.watcher().shadow.hashes_matched();

    d.game.retry(&pactlocke_run::RetryTo::Latest).unwrap();
    assert_eq!(ram_hash(&mut d), at_checkpoint, "back exactly at the checkpoint");
    assert_eq!(
        d.read(CATCH_SITE, 4),
        [0x10, 0xB5, 0x00, 0x24],
        "the catch block patched on since is off again"
    );
    assert_eq!(d.game.engine().retries_left(), 0);
    assert!(
        d.game.engine().view().closed_areas().is_empty(),
        "Route 1's Encounter is undone"
    );

    // Play on from the checkpoint: the Shadow went back to its own and matches.
    d.play("h D 64\nw 60");
    d.play("m A 900");
    let shadow = &d.watcher.as_ref().unwrap().shadow;
    assert_eq!(shadow.state(), ShadowState::Matched);
    assert_eq!(shadow.boot(), 2, "following the boot after the Retry");
    assert!(
        shadow.hashes_matched() >= matched_before + 6,
        "{} hashes matched, {matched_before} before the Retry",
        shadow.hashes_matched()
    );
}

#[test]
fn a_link_mates_death_faints_the_starter_mid_battle_through_the_games_own_faint() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let ruleset = Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        ..Ruleset::default()
    };
    let mut d = run_with(rom, ruleset);
    d.watched();
    d.play(FR_STARTER);
    d.fast_text();
    d.play("h D 64\nw 60");
    d.until_action_menu(Keys::A, 3000);
    second_pokemon_in_the_party(&mut d);
    let starter = d.game.game_mut().party()[0].clone();
    assert!(d.read(BATTLE_MONS + BATTLE_MON_HP, 2) != [0, 0]);

    // The partner's Pokémon in this Link died: the starter goes at the end of the turn.
    d.game
        .commands()
        .send(RunCommand::LinkDeath {
            pid: starter.pid,
            why: "Blue's BULBASAUR died".into(),
        })
        .unwrap();
    d.play("w 2");
    let deaths = d.feed(FeedKind::Death);
    assert!(
        deaths
            .iter()
            .any(|e| e.contains("fainted with its Link: Blue's BULBASAUR died")),
        "{deaths:?}"
    );
    assert!(
        d.in_battle(),
        "still in the battle: the faint comes at the end of the turn"
    );
    // FIGHT, the first move: the turn ends and the game's own faint sequence runs.
    for _ in 0..600 {
        if d.read(BATTLE_MONS + BATTLE_MON_HP, 2) == [0, 0] {
            break;
        }
        d.play("p A 20");
    }
    assert_eq!(d.read(BATTLE_MONS + BATTLE_MON_HP, 2), [0, 0], "fainted in battle");
    assert!(d.game.engine().view().wipe.is_none(), "the second Pokémon lives on");
    // The second Pokémon goes out (the party menu opens on the fainted
    // starter: down to the next, SEND OUT), and beats the rival's on 1 HP
    // (test setup).
    for _ in 0..60 {
        if d.read(BATTLE_MONS + BATTLE_MON_HP, 2) != [0, 0] {
            break;
        }
        d.play(
            "p B 20
p B 20
p D 20
p A 30
p A 120",
        );
    }
    assert_ne!(
        d.read(BATTLE_MONS + BATTLE_MON_HP, 2),
        [0, 0],
        "the second Pokémon is out"
    );
    rivals_pokemon_on_1_hp(&mut d);
    d.until_battle_over(Keys::A, 6000);
    d.play("m B 900");
    let shadow = &d.watcher.as_ref().unwrap().shadow;
    assert_eq!(
        shadow.state(),
        ShadowState::Matched,
        "the Perish writes went through the stream"
    );
    assert!(
        d.feed(FeedKind::Death).iter().any(|e| e.starts_with("Moved ")),
        "then to the Dead box: {:?}",
        d.feed(FeedKind::Death)
    );
}

/// The lab rival's battle, one `trainerbattle` per starter, as a test-only
/// Sync Gate: the nearest battle a script can reach again (it starts when the
/// Player walks down to leave the lab).
fn with_a_rival_gate(profile: &pactlocke_game::GameProfile) -> pactlocke_game::GameProfile {
    use pactlocke_game::{GateDef, HexBytes, PatchSite};
    let mut profile = profile.clone();
    let site = |at, trainer: u8| PatchSite {
        at,
        expect: HexBytes(vec![0x5C, 0x09, trainer, 0x01, 0x03]),
        write: None,
        below_cap: 0,
    };
    profile
        .sync_gates
        .as_mut()
        .expect("the profile lists gates")
        .gates
        .push(GateDef {
            name: "Rival".into(),
            sites: vec![
                site(GAME.rival_battles[0], 0x46),
                site(GAME.rival_battles[1], 0x48),
                site(GAME.rival_battles[2], 0x47),
            ],
        });
    profile
}

/// A Run with the lab rival and Brock as Sync Gates, watched, to the lab
/// with the starter chosen.
fn gated_to_the_lab(rom: Vec<u8>) -> Driver {
    pactlocke_mgba::force_utc_time_zone();
    let store = ProfileStore::from_env().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("{} should be profiled", GAME.file);
    };
    let profile = std::sync::Arc::new(with_a_rival_gate(&profile));
    let report = pactlocke_game::CapabilityReport::check(&profile, &rom);
    assert_eq!(
        report.get(pactlocke_game::Rule::SyncGates),
        &pactlocke_game::Capability::Enforceable,
        "every gate site and the hold area check out on this ROM"
    );
    let storage = MemoryStorage::default();
    let ruleset = Ruleset {
        first_encounter: true,
        sync_gates: vec!["Rival".into(), "Brock".into()],
        ..Ruleset::default()
    };
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
    let mut d = booted_on(profile, engine, storage, rom);
    d.watched();
    d.play(FR_STARTER);
    d.fast_text();
    assert_eq!(
        d.read(GAME.rival_battles[0], 1),
        [0x05],
        "held: a goto to the hold script"
    );
    assert_eq!(d.read(GAME.brock_battle, 1), [0x05], "Brock is held too");
    d
}

#[test]
fn a_held_sync_gate_shows_its_message_lets_the_player_go_and_readies_them_up() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = gated_to_the_lab(rom);
    // Leaving the lab starts the rival battle's script: it reaches the held gate.
    d.play(
        "h D 64
w 60",
    );
    let mut saw_message = false;
    for _ in 0..40 {
        saw_message |= shows_text(&mut d, &pactlocke_game::encode_text("partners", 8).unwrap());
        d.play("p A 30");
    }
    dump_screen(&d.game, "gate-held");
    assert!(saw_message, "\"Waiting for your partners to be ready...\" on screen");
    assert!(!d.in_battle(), "no battle while the gate holds");
    let run = d.feed(FeedKind::Run);
    assert!(
        run.iter()
            .any(|e| e == "Ready at Rival: waiting for your partners to be ready"),
        "{run:?}"
    );
    // Let go: the Player walks away, out of the lab.
    let script = d.game.game_mut().script().status;
    assert_eq!(script, pactlocke_game::ScriptStatus::Idle, "the hold script ended");
    d.play("m B 300");
    let shadow = &d.watcher.as_ref().unwrap().shadow;
    assert_eq!(shadow.state(), ShadowState::Matched);
}

#[test]
fn a_sync_gate_everyone_is_ready_at_lets_the_real_battle_run() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = gated_to_the_lab(rom);
    // Everyone is Ready at the gate before this Player gets there.
    d.game
        .commands()
        .send(RunCommand::OpenGate {
            gate: "Rival".into(),
            checkpoint: true,
        })
        .unwrap();
    d.play("w 30");
    assert_eq!(
        d.read(GAME.rival_battles[0], 5),
        [0x5C, 0x09, 0x46, 0x01, 0x03],
        "let go: the ROM's own bytes"
    );
    assert_eq!(d.read(GAME.brock_battle, 1), [0x05], "Brock is still held");
    assert!(
        d.feed(FeedKind::Run)
            .iter()
            .any(|e| e == "Everyone is Ready at Rival: the battle is on")
    );
    d.play(
        "h D 64
w 60",
    );
    d.until_action_menu(Keys::A, 3000);
    assert!(d.in_battle(), "the real rival battle, through the game's own script");
    d.play("m A 300");
    let shadow = &d.watcher.as_ref().unwrap().shadow;
    assert_eq!(shadow.state(), ShadowState::Matched);
}

#[test]
fn a_partner_avatar_is_drawn_from_their_shadow_game_and_nothing_else_changes() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    // A draws partners; a control gets exactly A's presses and draws none; B walks off.
    let mut a = new_run(rom.clone());
    a.watched();
    let mut control = new_run(rom.clone());
    let mut b = new_run(rom);
    b.watched();
    for d in [&mut a, &mut control, &mut b] {
        d.play(FR_STARTER);
        d.play("w 60");
    }
    b.play("h L 32\nw 120");
    let avatars = a.game.draw_partner_avatars();
    for _ in 0..180 {
        let seen = b.watcher().shadow.seen().avatar.clone();
        *avatars.lock().unwrap() = seen
            .map(|seen| vec![pactlocke_run::PartnerAvatar { player: [2; 32], seen }])
            .unwrap_or_default();
        a.frame(Keys::NONE);
        control.frame(Keys::NONE);
        b.frame(Keys::NONE);
    }

    let mine = a.game.game_mut().whereabouts().expect("A on the overworld");
    let theirs = b.watcher().shadow.seen().avatar.clone().expect("B's Shadow shows B");
    assert_eq!(theirs.at.map, mine.map, "the same map: Oak's lab");
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
    assert_eq!(a.watcher().shadow.state(), ShadowState::Matched);
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

#[test]
fn a_crashed_app_takes_its_presses_back_from_the_host_and_rebuilds_its_game_exactly() {
    let Some(rom) = rom() else {
        eprintln!(
            "skipped: set PACTLOCKE_ROMS to a folder with your own {}, or PACTLOCKE_ROM to it",
            GAME.file
        );
        return;
    };
    let mut d = new_run(rom);
    d.watched();
    d.play(FR_STARTER);
    d.play("w 60");
    // The Run file as of its last flush; the stream goes on to the Host.
    d.game.flush();
    let crashed = d.storage.snapshot();
    to_a_closed_route_1_battle_from_the_lab(&mut d);
    // Up to the end of a chunk the Host has.
    for _ in 0..pactlocke_run::CHUNK_FRAMES * 2 {
        if d.watcher().store.received(&[1; 32]) == Some((1, d.game.engine().frames_run())) {
            break;
        }
        d.frame(Keys::NONE);
    }
    let frames = d.game.engine().frames_run();
    assert_eq!(d.watcher().store.received(&[1; 32]), Some((1, frames)));
    let at_crash = ram_hash(&mut d);
    let rom = d.rom.clone();
    let host_has = d.watcher().store.wanted(&pactlocke_run::StreamWant {
        player: [1; 32],
        boot: 1,
        from_frame: 0,
    });
    drop(d);

    let mut engine = RunEngine::resume(Box::new(crashed.clone()), Box::new(SystemRunClock)).unwrap();
    let BootPlan::Rebuild { from, keys, .. } = engine.boot_plan().clone() else {
        panic!("rebuilt after a crash");
    };
    let flushed = from.frame + keys.len() as u64;
    assert!(flushed < frames, "the Run file is behind: {flushed} of {frames}");
    engine.take_back_presses(&host_has);
    assert!(
        matches!(engine.boot_plan(), BootPlan::Rebuild { from, keys, .. } if from.frame + keys.len() as u64 == frames),
        "every press the Host had"
    );
    let mut d = booted(engine, crashed, rom);
    assert_eq!(ram_hash(&mut d), at_crash, "the game as it was");
    assert!(d.in_battle());
    assert_eq!(d.read(CATCH_SITE, 4), [0x01, 0x20, 0x70, 0x47], "with its patch set");
}
