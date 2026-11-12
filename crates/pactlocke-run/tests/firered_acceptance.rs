//! Seam 2, local only: a solo Run on the real
//! FireRed (USA) 1.0, driven through the Run engine on the real game
//! interface. Skipped unless `PACTLOCKE_ROMS` names a folder holding
//! `Pokemon - FireRed Version (USA).gba`. Never runs in CI; the ROM is only
//! read, the save stays in memory, and the Run is kept in memory too.
//!
//! A few setup writes (fast text, a Potion, five Poké Balls) are made
//! straight to the core: they are the test's, not the Run's.

use std::path::PathBuf;

use pactlocke_emu::{CoreAdapter, Keys};
use pactlocke_game::{Game, LevelCap, ProfileGame, ProfileStore, RomIdentity};
use pactlocke_mgba::MgbaCore;
use pactlocke_run::{FeedKind, MemoryStorage, NewRun, Ruleset, RunEngine, RunningGame, SystemRunClock};

const FIRERED_FILE: &str = "Pokemon - FireRed Version (USA).gba";

fn firered_rom() -> Option<Vec<u8>> {
    let dir = PathBuf::from(std::env::var_os("PACTLOCKE_ROMS")?);
    match std::fs::read(dir.join(FIRERED_FILE)) {
        Ok(rom) => Some(rom),
        Err(e) => panic!("PACTLOCKE_ROMS is set but {FIRERED_FILE} can't be read: {e}"),
    }
}

// FireRed (USA) 1.0, from pret's pokefirered.
const SAVE_BLOCK_1_POINTER: u32 = 0x0300_5008;
const SAVE_BLOCK_2_POINTER: u32 = 0x0300_500C;
const BALLS_POCKET: u32 = 0x430; // in save block 1
const ITEMS_POCKET: u32 = 0x310;
const BAG_KEY: u32 = 0xF20; // in save block 2
const TEXT_SPEED: u32 = 0x14; // in save block 2
const IN_BATTLE: u32 = 0x0300_3529; // gMain.inBattle
const BATTLER_CONTROLLER: u32 = 0x0300_4FE0; // gBattlerControllerFuncs[0]
/// What that holds while the Player picks FIGHT / BAG / POKéMON / RUN.
const CHOOSE_ACTION: [u32; 2] = [0x0802_E438, 0x080E_763C];
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
const ITEMS: u32 = 0x083D_B028; // gItems
const ITEM_SIZE: usize = 44;
const ITEM_COUNT: usize = 375;
const ITEM_BATTLE_USAGE: usize = 32;

type Running = RunningGame<MgbaCore>;

struct Driver {
    game: Running,
}

impl Driver {
    fn frame(&mut self, keys: Keys) {
        self.game.run_frame(keys);
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
        CHOOSE_ACTION.contains(&(self.u32(BATTLER_CONTROLLER) & !1))
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
        self.game.write(slot, &item.to_le_bytes());
        self.game.write(slot + 2, &(quantity ^ key).to_le_bytes());
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
        self.game.write(addr, &[(speed & !7) | 2]);
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

fn firered_run(rom: Vec<u8>) -> Driver {
    firered_run_with(
        rom,
        Ruleset {
            first_encounter: true,
            ..Ruleset::default()
        },
    )
}

fn firered_run_with(rom: Vec<u8>, ruleset: Ruleset) -> Driver {
    pactlocke_mgba::force_utc_time_zone();
    let store = ProfileStore::bundled().unwrap();
    let RomIdentity::Profiled { profile, .. } = store.identify(&rom) else {
        panic!("FireRed should be profiled");
    };
    let mut core = MgbaCore::new();
    core.load_rom(&rom).unwrap();
    core.set_clock(1_767_225_600_000);
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
    let game = ProfileGame::new(profile, rom, core);
    Driver {
        game: RunningGame::new(game, engine),
    }
}

#[test]
fn a_closed_route_1_refuses_the_ball_and_keeps_it_and_running_still_works() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let mut d = firered_run(rom);
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
    let stamped = d.game.engine().stamped().to_vec();
    assert_eq!(
        stamped.len(),
        2,
        "both catch block sites, at one frame boundary: {stamped:?}"
    );
    assert_eq!(stamped[0].frame, stamped[1].frame);
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
    assert_eq!(d.game.engine().stamped().len(), 4, "on and off, two sites each");
    assert!(d.feed(FeedKind::Catch).is_empty(), "no catch in the feed");
    for entry in d.game.engine().view().feed.iter().rev() {
        eprintln!("feed: {}", entry.text);
    }
}

#[test]
fn a_starter_that_faints_to_the_rival_goes_to_the_dead_box_and_stays_there() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let mut d = firered_run_with(
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
    let starter_bytes = d.read(PARTY, PARTY_SLOT_SIZE);
    d.until_battle_over(Keys::A, 6000);

    let deaths = d.feed(FeedKind::Death);
    assert_eq!(deaths.len(), 1, "a Death, recorded in the battle: {deaths:?}");
    assert!(
        deaths[0].contains("fainted against a trainer's") && deaths[0].ends_with("is Dead"),
        "{deaths:?}"
    );
    let graveyard = d.game.engine().view().graveyard.clone();
    assert_eq!(graveyard.len(), 1);
    assert!(graveyard[0].killed_by.starts_with("a trainer's"), "{graveyard:?}");

    // Back on the overworld, before anything can heal it: off to box 14.
    d.play("m B 600");
    let deaths = d.feed(FeedKind::Death);
    assert_eq!(
        deaths[0],
        format!(
            "Moved {} to box 14, the Dead box: Dead Pokémon can't be used again",
            starter.nickname
        )
    );
    assert_eq!(d.read(PARTY_COUNT, 1)[0], 0, "the party count is fixed");
    assert!(d.game.game_mut().party().is_empty());
    let dead_box: Vec<u32> = d
        .game
        .game_mut()
        .pc()
        .into_iter()
        .filter(|s| s.box_number == 14)
        .map(|s| s.pokemon.pid)
        .collect();
    assert_eq!(dead_box, [starter.pid]);
    dump_screen(&d.game, "after-dead-box");

    // Withdrawn by hand (test setup: what the PC's WITHDRAW does to memory): back it goes, in red.
    let box_14_slot_1 = d.u32(PC_STORAGE_POINTER) + 4 + 13 * 30 * BOX_SLOT_SIZE;
    d.game.write(box_14_slot_1, &[0; BOX_SLOT_SIZE as usize]);
    d.game.write(PARTY, &starter_bytes);
    d.game.write(PARTY_COUNT, &[1]);
    d.play("w 30");
    assert_eq!(
        d.feed(FeedKind::Violation),
        [format!(
            "Violation: {} is Dead but was back in the party, so it went back to box 14",
            starter.nickname
        )]
    );
    assert_eq!(d.read(PARTY_COUNT, 1)[0], 0);
    for entry in d.game.engine().view().feed.iter().rev() {
        eprintln!("feed: {}", entry.text);
    }
}

#[test]
fn a_route_1_pokemon_from_a_family_already_owned_is_skipped_as_a_dupe() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let mut d = firered_run_with(
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
    assert!(d.game.engine().view().closed_areas.is_empty(), "Route 1 is still open");
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
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let mut d = firered_run_with(
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
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let cap = |name: &str, level| LevelCap {
        name: name.into(),
        level,
    };
    let mut d = firered_run_with(
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

/// Leaves the rival's Pokémon out in battle on 1 HP (test setup).
fn rivals_pokemon_on_1_hp(d: &mut Driver) {
    d.game
        .write(BATTLE_MONS + BATTLE_MON_SIZE + BATTLE_MON_HP, &1u16.to_le_bytes());
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
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let mut d = firered_run_with(
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
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let mut d = firered_run_with(rom, Ruleset::default());
    d.play(FR_STARTER);
    assert!(!battle_style_is_set(&mut d), "a new game's battle style is Shift");
    d.fast_text();
    rival_battle_with_two_pokemon_each(&mut d);
    assert!(knock_out_the_first_and_see_if_shift_is_offered(&mut d));
}

#[test]
fn with_no_items_only_balls_work_in_a_wild_battle_and_items_work_outside_battle() {
    let Some(rom) = firered_rom() else {
        eprintln!("skipped: set PACTLOCKE_ROMS to a folder with your own {FIRERED_FILE}");
        return;
    };
    let clean_items = {
        let at = (ITEMS - 0x0800_0000) as usize;
        rom[at..at + ITEM_COUNT * ITEM_SIZE].to_vec()
    };
    let mut d = firered_run_with(
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
    let patched = d.read(ITEMS, ITEM_COUNT * ITEM_SIZE);
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
