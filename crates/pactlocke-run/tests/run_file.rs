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
    let mut player = Player::starts_a_run_in(Box::new(library.create().unwrap()), clock.clone(), ruleset);
    player.game.party = vec![party_pokemon(PIDGEY, 0x0000_BEEF, "BIRDY")];
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
