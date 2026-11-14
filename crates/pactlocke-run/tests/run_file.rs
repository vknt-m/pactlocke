//! The Run file store: one SQLite database per Run, written on
//! its own thread, in a real folder. Runs are created, named, listed and
//! resumed through it.

mod support;

use pactlocke_emu::Keys;
use pactlocke_run::{FeedKind, Ruleset, RunLibrary, RunStorage};
use support::*;

fn first_encounter() -> Ruleset {
    Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    }
}

#[test]
fn a_new_run_is_listed_with_its_name_and_game() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    assert!(library.list().is_empty());

    let file = library.create().unwrap();
    let player = Player::starts_a_run_in(Box::new(file), TestClock::at(START_MS), first_encounter());
    drop(player);

    let runs = library.list();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.name, "Kanto solo");
    assert_eq!(run.game_name, "Pokémon FireRed (USA) 1.0");
    assert_eq!(run.game_sha1, "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc");
    assert_eq!(run.rom_path.as_deref(), Some("roms/firered.gba"));
    assert_eq!(run.created_ms, START_MS);
    assert_eq!(run.ruleset, first_encounter());
}

#[test]
fn runs_are_listed_most_recently_played_first() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);

    drop(Player::starts_named_run_in(
        "Older",
        Box::new(library.create().unwrap()),
        clock.clone(),
    ));
    clock.advance_ms(60_000);
    drop(Player::starts_named_run_in(
        "Newer",
        Box::new(library.create().unwrap()),
        clock.clone(),
    ));

    let names: Vec<String> = library.list().into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["Newer", "Older"]);

    // Playing the older Run again puts it back on top.
    clock.advance_ms(60_000);
    let older = library.list().into_iter().find(|r| r.name == "Older").unwrap();
    let resumed = Player::resumes_from(Box::new(library.open_run(&older.id).unwrap()), clock.clone());
    drop(resumed);
    let runs = library.list();
    assert_eq!(runs[0].name, "Older");
    assert_eq!(runs[0].last_played_ms, START_MS + 120_000);
}

#[test]
fn a_run_resumed_from_its_file_keeps_its_closed_areas_and_feed() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);
    let mut player = Player::starts_a_run_in(Box::new(library.create().unwrap()), clock.clone(), first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, 0x0000_BEEF);
    drop(player);

    let id = library.list()[0].id.clone();
    clock.advance_ms(3_600_000);
    let mut player = Player::resumes_from(Box::new(library.open_run(&id).unwrap()), clock);
    assert_eq!(player.feed(FeedKind::Catch), ["Caught PIDGEY on Route 1"]);
    assert_eq!(player.engine.view().closed_areas, ["Route 1"]);

    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    assert!(player.game.catches_blocked);
}

#[test]
fn the_input_stream_and_save_memory_are_kept_in_the_run_file() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);
    let mut player = Player::starts_a_run_in(Box::new(library.create().unwrap()), clock.clone(), first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.engine.frame(Keys::A | Keys::UP, &mut player.game);
    player.meets_wild(RATTATA);
    player.leaves_battle();
    player.engine.save_memory_seen(&[0xAA; 1024]);
    let frames = player.engine.frames_run();
    let stamped = player.engine.stamped().to_vec();
    drop(player);

    let id = library.list()[0].id.clone();
    let stored = library.open_run(&id).unwrap().load().unwrap();
    let keys: Vec<Keys> = stored.inputs.iter().flat_map(|b| b.keys.clone()).collect();
    assert_eq!(keys.len() as u64, frames, "every frame's keys");
    assert!(keys.contains(&(Keys::A | Keys::UP)));
    let kept: Vec<_> = stored.inputs.iter().flat_map(|b| b.stamped.clone()).collect();
    assert_eq!(kept, stamped, "the catch block on and off, with their frames");
    assert_eq!(stamped.len(), 2);
    assert_eq!(stored.save_memory.as_deref(), Some(&[0xAA; 1024][..]));

    // The game boots with that save memory on resume.
    let player = Player::resumes_from(Box::new(library.open_run(&id).unwrap()), clock);
    assert_eq!(player.engine.save_memory(), Some(&[0xAA; 1024][..]));
}

#[test]
fn files_in_the_runs_folder_that_are_not_runs_are_not_listed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
    std::fs::write(dir.path().join("broken.pactrun"), "not a database").unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let _empty = library.create().unwrap();
    assert!(library.list().is_empty(), "nor is a Run file nobody started");
}

#[test]
fn a_run_resumed_from_its_file_keeps_its_dead_and_graveyard() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);
    let ruleset = Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        ..Ruleset::default()
    };
    let mut player = Player::starts_a_run_in(Box::new(library.create().unwrap()), clock.clone(), ruleset.clone());
    // A second Pokémon, alive, so the Death isn't a wiped party.
    player.game.party = vec![
        party_pokemon(PIDGEY, 0x0000_BEEF, "BIRDY"),
        party_pokemon(RATTATA, 0x5151, "RATTY"),
    ];
    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    player.faints(0x0000_BEEF);
    player.leaves_battle();
    drop(player);

    let run = &library.list()[0];
    assert_eq!(run.ruleset, ruleset);
    let mut player = Player::resumes_from(Box::new(library.open_run(&run.id).unwrap()), clock);
    let graveyard = &player.engine.view().graveyard;
    assert_eq!(graveyard.len(), 1);
    assert_eq!(graveyard[0].killed_by, "a wild RATTATA on Route 1");

    // Withdrawn in this boot after being moved: a Violation.
    player.game.party = vec![party_pokemon(PIDGEY, 0x0000_BEEF, "BIRDY")];
    player.plays(1);
    player.withdraws(0x0000_BEEF);
    assert_eq!(player.feed(FeedKind::Violation).len(), 1);
}

/// A core checkpoint as a stand-in: a savestate of `byte`s that compresses well.
fn core_checkpoint(byte: u8) -> pactlocke_emu::Checkpoint {
    pactlocke_emu::Checkpoint {
        save_memory: vec![0x5A; 128],
        state: vec![byte; 400_000],
    }
}

#[test]
fn checkpoints_and_the_suspend_slot_are_kept_in_the_run_file() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);
    let mut player = Player::starts_a_run_in(Box::new(library.create().unwrap()), clock.clone(), first_encounter());
    player.engine.take_checkpoint(core_checkpoint(0));
    // Five in-game saves: the file keeps the newest checkpoints only.
    for save in 1..=5u8 {
        player.plays(10);
        player.engine.save_memory_seen(&[save]);
        player.engine.take_checkpoint(core_checkpoint(save));
    }
    player.plays(10);
    player.engine.suspend(core_checkpoint(0xEE));
    drop(player);

    let id = library.list()[0].id.clone();
    let path = dir.path().join(format!("{}.pactrun", id.0));
    assert!(
        std::fs::metadata(&path).unwrap().len() < 400_000,
        "the checkpoints are compressed"
    );
    let stored = library.open_run(&id).unwrap().load().unwrap();
    let latest = stored.latest_checkpoint.expect("the newest checkpoint");
    assert_eq!((latest.frame, latest.core), (50, core_checkpoint(5)));
    let suspended = stored.suspended.expect("the suspend slot");
    assert_eq!((suspended.frame, suspended.core), (60, core_checkpoint(0xEE)));
    let db = rusqlite::Connection::open(&path).unwrap();
    let kept: i64 = db
        .query_row("SELECT COUNT(*) FROM checkpoints", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        kept, 4,
        "the boot checkpoint and the newest three Run Checkpoints; older ones are pruned"
    );
    drop(db);
    let hashes = stored.events.iter().filter(|e| e.event.is_run_checkpoint()).count();
    assert_eq!(hashes, 5, "every Run Checkpoint's hash stays in the log");
    let mut file = library.open_run(&id).unwrap();
    let first = stored.events.iter().find(|e| e.event.is_run_checkpoint()).unwrap().seq;
    let newest = stored
        .events
        .iter()
        .rev()
        .find(|e| e.event.is_run_checkpoint())
        .unwrap()
        .seq;
    assert!(
        pactlocke_run::RunStorage::checkpoint(&mut file, first).is_none(),
        "pruned"
    );
    assert_eq!(
        pactlocke_run::RunStorage::checkpoint(&mut file, newest).map(|c| c.core),
        Some(core_checkpoint(5)),
        "the newest is kept, found by its event"
    );
    drop(file);

    // Resuming uses up the slot.
    let player = Player::resumes_from(Box::new(library.open_run(&id).unwrap()), clock);
    assert!(matches!(
        player.engine.boot_plan(),
        pactlocke_run::BootPlan::Suspended(_)
    ));
    drop(player);
    assert!(library.open_run(&id).unwrap().load().unwrap().suspended.is_none());
}

/// The Host orders a speed change of theirs into the log.
fn host_logs_speed(log: &mut pactlocke_run::RunLog, host: &pactlocke_run::Signer, speed: u8) {
    use pactlocke_run::{LogBody, RunEvent};
    let counter = log.next_counter(&host.public());
    let submission = log.sign(host, counter, &LogBody::Event(RunEvent::SpeedChanged { speed }));
    log.order(host, submission).unwrap();
}

#[test]
fn a_run_with_friends_keeps_its_run_log_in_the_run_file_and_is_listed_so() {
    use pactlocke_run::{LogBody, RunLog, Signer};
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);
    drop(Player::starts_named_run_in(
        "Alone",
        Box::new(library.create().unwrap()),
        clock.clone(),
    ));
    clock.advance_ms(1_000);
    let mut player = Player::starts_named_run_in("Together", Box::new(library.create().unwrap()), clock.clone());

    let host = Signer::from_secret([7; 32]);
    let mut log = RunLog::new(host.public());
    let genesis = LogBody::Genesis {
        nonce: [1; 16],
        run_name: "Together".into(),
        game_name: "Pokémon FireRed (USA) 1.0".into(),
        game_sha1: "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc".into(),
    };
    let submission = log.sign(&host, 1, &genesis);
    log.order(&host, submission).unwrap();
    let locked = LogBody::Locked(pactlocke_run::Locked {
        run_name: "Together".into(),
        game_name: "Pokémon FireRed (USA) 1.0".into(),
        game_sha1: "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc".into(),
        ruleset: first_encounter(),
        roster: vec![pactlocke_run::RosterEntry {
            key: host.public(),
            name: "Red".into(),
        }],
        start_ms: START_MS,
    });
    let submission = log.sign(&host, 2, &locked);
    log.order(&host, submission).unwrap();
    host_logs_speed(&mut log, &host, 2);
    let mut other = log.clone();
    host_logs_speed(&mut log, &host, 3);
    host_logs_speed(&mut log, &host, 4);
    player.engine.keep_log(0, log.entries());
    // Another copy of the log, alike for three entries, wins from there on.
    host_logs_speed(&mut other, &host, 1);
    player.engine.keep_log(3, &other.entries()[3..]);
    // An entry of this Player's waiting for a Host, signed but not in the log.
    let waiting = other.sign(
        &host,
        other.next_counter(&host.public()),
        &LogBody::Event(pactlocke_run::RunEvent::SpeedChanged { speed: 2 }),
    );
    player.engine.keep_waiting(std::slice::from_ref(&waiting));
    // This app's Shadow Game of another Player, as it stood when the app left.
    let shadow = |frame: u64| pactlocke_run::ShadowCheckpoint {
        at: pactlocke_run::GamePoint { boot: 1, frame },
        core: core_checkpoint(0x33),
        patches: vec![(0x0800_1000, vec![1, 2, 3])],
        owned: vec![0xB1, 0xB2],
        fainted: vec![0xB2],
    };
    let blue = [2; 32];
    player.engine.keep_shadow(blue, &shadow(100));
    player.engine.keep_shadow(blue, &shadow(200));
    drop(player);

    let runs = library.list();
    let together = runs.iter().find(|r| r.name == "Together").unwrap();
    let alone = runs.iter().find(|r| r.name == "Alone").unwrap();
    assert!(together.with_friends);
    assert!(!alone.with_friends);
    let stored = library.open_run(&together.id).unwrap().load().unwrap();
    assert_eq!(stored.log, other.entries());
    assert_eq!(stored.waiting, [waiting]);
    assert_eq!(
        stored.shadows,
        [(blue, shadow(200))],
        "the newest, in place of the one before"
    );
    assert!(library.open_run(&alone.id).unwrap().load().unwrap().log.is_empty());
}

/// A Run log locked for `ruleset`, with its Host the only Player in it.
fn friends_log(host: &pactlocke_run::Signer, ruleset: &Ruleset) -> pactlocke_run::RunLog {
    use pactlocke_run::{LogBody, RunLog};
    let mut log = RunLog::new(host.public());
    let genesis = LogBody::Genesis {
        nonce: [2; 16],
        run_name: "Kanto with friends".into(),
        game_name: "Pokémon FireRed (USA) 1.0".into(),
        game_sha1: "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc".into(),
    };
    let locked = LogBody::Locked(pactlocke_run::Locked {
        run_name: "Kanto with friends".into(),
        game_name: "Pokémon FireRed (USA) 1.0".into(),
        game_sha1: "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc".into(),
        ruleset: ruleset.clone(),
        roster: vec![pactlocke_run::RosterEntry {
            key: host.public(),
            name: "Red".into(),
        }],
        start_ms: START_MS,
    });
    for (counter, body) in [(1, genesis), (2, locked)] {
        let submission = log.sign(host, counter, &body);
        log.order(host, submission).unwrap();
    }
    log
}

#[test]
fn a_solo_copy_of_a_run_with_friends_goes_on_alone_from_there_and_the_run_with_friends_stays() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);
    let ruleset = Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        links: true,
        retries: 3,
        sync_gates: vec!["Brock".into()],
        ..Ruleset::default()
    };
    let mut new = new_solo_run(ruleset.clone());
    new.name = "Kanto with friends".into();
    let mut player = Player::starts(new, Box::new(library.create().unwrap()), clock.clone());
    player.engine.take_checkpoint(core_checkpoint(0));
    let host = pactlocke_run::Signer::from_secret([7; 32]);
    player.engine.keep_log(0, friends_log(&host, &ruleset).entries());
    // A second Pokémon, alive, so the Death isn't a wiped party.
    player.game.party = vec![
        party_pokemon(PIDGEY, 0x0000_BEEF, "BIRDY"),
        party_pokemon(RATTATA, 0x5151, "RATTY"),
    ];
    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    player.faints(0x0000_BEEF);
    player.leaves_battle();
    player.engine.suspend(core_checkpoint(0x44));
    drop(player);
    let original = library.list()[0].id.clone();

    clock.advance_ms(86_400_000);
    let copy = library.solo_copy(&original, START_MS + 86_400_000).unwrap();
    let runs = library.list();
    let summary = runs.iter().find(|r| r.id == copy).expect("the copy is listed");
    assert_eq!(summary.name, "Kanto with friends (solo)");
    assert!(!summary.with_friends);
    assert_eq!(
        summary.copied_from,
        Some(pactlocke_run::CopiedFrom {
            name: "Kanto with friends".into(),
            at_ms: START_MS + 86_400_000,
        })
    );
    assert_eq!(
        summary.ruleset,
        Ruleset {
            links: false,
            sync_gates: Vec::new(),
            ..ruleset.clone()
        },
        "Links and Sync Gates off, every other rule kept, and the Retries left"
    );

    // The Run with friends stays as it was, and can still be resumed from there.
    let stored = library.open_run(&original).unwrap().load().unwrap();
    assert!(!stored.log.is_empty());
    assert_eq!(stored.suspended.map(|s| s.core), Some(core_checkpoint(0x44)));
    assert_eq!(stored.info.name, "Kanto with friends");

    // The copy goes on alone, exactly where the Player left: the Dead stay Dead.
    let player = Player::resumes_from(Box::new(library.open_run(&copy).unwrap()), clock);
    let pactlocke_run::BootPlan::Suspended(from) = player.engine.boot_plan() else {
        panic!("from where the Player left the Run with friends");
    };
    assert_eq!(from.core, core_checkpoint(0x44));
    assert_eq!(player.engine.view().graveyard.len(), 1);
    assert_eq!(player.engine.view().closed_areas, ["Route 1"]);
    assert!(player.engine.kept_log().is_empty(), "alone");
}

#[test]
fn a_run_lost_to_a_game_over_is_listed_so_with_how_it_ended() {
    let dir = tempfile::tempdir().unwrap();
    let library = RunLibrary::open(dir.path()).unwrap();
    let clock = TestClock::at(START_MS);
    let ruleset = Ruleset {
        faint_is_dead: true,
        ..Ruleset::default()
    };
    let mut player = Player::starts_a_run_in(Box::new(library.create().unwrap()), clock.clone(), ruleset);
    player.game.party = vec![party_pokemon(PIDGEY, 0x0000_BEEF, "BIRDY")];
    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    player.faints(0x0000_BEEF);
    drop(player);

    let run = &library.list()[0];
    let over = run.game_over.as_ref().expect("listed as a game over");
    assert_eq!(
        over.how,
        "Your party was wiped on Route 1: BIRDY fainted against a wild RATTATA"
    );
    assert!(over.at_ms >= START_MS);
}
