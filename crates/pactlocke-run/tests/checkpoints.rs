//! Seam 1: checkpoints, suspend and crash recovery, through the Run engine.
//! The engine says when it wants a checkpoint and keeps the ones it is given;
//! on a resume it says how the game is to be booted. The real core's side
//! (restoring and replaying) is in `rebuild.rs`.

mod support;

use pactlocke_emu::{Checkpoint, Keys};
use pactlocke_game::{Change, SourceDef, SourceKind};
use pactlocke_run::{BootPlan, FeedKind, MemoryStorage, Ruleset};
use support::*;

/// A core checkpoint as the fake game's stand-in: the frame it was taken at.
fn fake_core(frame: u64) -> Checkpoint {
    Checkpoint {
        save_memory: vec![0xEE; 4],
        state: frame.to_le_bytes().to_vec(),
    }
}

/// Gives the engine the checkpoint it asks for, as the running game does.
fn checkpoint_if_wanted(player: &mut Player) {
    if player.engine.checkpoint_wanted() {
        let frame = player.engine.frames_run();
        player.engine.take_checkpoint(fake_core(frame));
    }
}

fn plays_keys(player: &mut Player, keys: &[Keys]) {
    for &k in keys {
        player.engine.frame(k, &mut player.game);
        checkpoint_if_wanted(player);
    }
}

/// The Run as it was on disk at this moment, for a resume after a crash here.
fn crash_now(player: &mut Player) -> MemoryStorage {
    player.engine.flush();
    player.storage.snapshot()
}

#[test]
fn a_boot_takes_a_checkpoint_and_so_does_each_in_game_save() {
    let mut player = Player::starts_a_run(Ruleset::default());
    assert!(player.engine.checkpoint_wanted(), "at power-on");
    checkpoint_if_wanted(&mut player);
    assert!(!player.engine.checkpoint_wanted());

    player.engine.save_memory_seen(&[0xFF; 3]);
    assert!(
        !player.engine.checkpoint_wanted(),
        "a new game's blank cartridge isn't a save"
    );
    player.plays(30);
    player.engine.save_memory_seen(&[1, 2, 3]);
    assert!(player.engine.checkpoint_wanted(), "the game was saved");
    checkpoint_if_wanted(&mut player);

    let stored = player.storage.load_run();
    let checkpoint = stored.latest_checkpoint.expect("stored with its event");
    assert_eq!(checkpoint.frame, 30);
    assert_eq!(checkpoint.core, fake_core(30));
}

#[test]
fn a_clean_close_resumes_from_the_in_game_save() {
    let mut player = Player::starts_a_run(Ruleset::default());
    checkpoint_if_wanted(&mut player);
    player.plays(30);
    player.engine.save_memory_seen(&[1, 2, 3]);
    let storage = player.storage.clone();
    drop(player);

    let player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    assert_eq!(player.engine.boot_plan(), &BootPlan::FromSave(vec![1, 2, 3]));
    assert_eq!(player.feed(FeedKind::Run)[0], "Run resumed from your last in-game save");
}

#[test]
fn suspend_resumes_exactly_there_and_the_slot_is_used_up() {
    let mut player = Player::starts_a_run(Ruleset::default());
    checkpoint_if_wanted(&mut player);
    player.plays(40);
    player.engine.suspend(fake_core(40));
    let storage = player.storage.clone();
    drop(player);

    let player = Player::resumes(storage.clone(), TestClock::at(START_MS + 60_000));
    let BootPlan::Suspended(from) = player.engine.boot_plan() else {
        panic!("{:?}", player.engine.boot_plan());
    };
    assert_eq!(from.core, fake_core(40));
    assert_eq!(player.feed(FeedKind::Run)[0], "Run resumed where you suspended it");
    drop(player);

    let player = Player::resumes(storage, TestClock::at(START_MS + 120_000));
    assert!(
        !matches!(player.engine.boot_plan(), BootPlan::Suspended(_)),
        "the suspend slot was used up"
    );
}

#[test]
fn after_a_crash_the_game_is_rebuilt_from_the_last_checkpoint_and_the_presses_after_it() {
    let mut player = Player::starts_a_run(Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    });
    checkpoint_if_wanted(&mut player);
    player.engine.save_memory_seen(&[0xFF]);
    plays_keys(&mut player, &[Keys::A; 10]);
    player.engine.save_memory_seen(&[9]);
    checkpoint_if_wanted(&mut player);

    // After the checkpoint: some presses and a patch (Route 1's Encounter is
    // used, so the next battle there refuses balls).
    let after: Vec<Keys> = [Keys::B, Keys::UP, Keys::NONE].repeat(4);
    plays_keys(&mut player, &after);
    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    player.leaves_battle();
    player.meets_wild(PIDGEY);
    assert!(player.game.catches_blocked);
    let run_so_far = player.engine.frames_run();
    let storage = crash_now(&mut player);
    drop(player);

    let player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    let BootPlan::Rebuild { from, keys, stamped } = player.engine.boot_plan() else {
        panic!("{:?}", player.engine.boot_plan());
    };
    assert_eq!(from.frame, 10);
    assert_eq!(from.core, fake_core(10));
    assert_eq!(keys.len() as u64, run_so_far - 10, "every press after it");
    assert_eq!(keys[..12], after[..]);
    assert!(
        stamped
            .iter()
            .any(|s| matches!(s.change, Change::Patch { .. }) && s.frame > 10),
        "the catch block, at its frame: {stamped:?}"
    );
    assert!(
        player.feed(FeedKind::Run)[0].starts_with("Run recovered after the app closed unexpectedly"),
        "{:?}",
        player.feed(FeedKind::Run)
    );
}

#[test]
fn every_logged_event_is_covered_by_the_recorded_presses() {
    let mut player = Player::starts_a_run(Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    });
    checkpoint_if_wanted(&mut player);
    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    // No flush: the crash comes at once.
    let storage = player.storage.snapshot();
    let stored = storage.clone().load_run();
    let recorded: u64 = stored.inputs.iter().map(|b| b.keys.len() as u64).sum();
    let last_event = stored.events.last().unwrap().frame;
    assert!(
        recorded >= last_event,
        "presses up to frame {recorded}, an event at {last_event}"
    );
}

#[test]
fn a_checkpoint_holds_the_patches_active_when_it_was_taken() {
    let mut player = Player::starts_a_run(Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    });
    checkpoint_if_wanted(&mut player);
    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    player.leaves_battle();
    player.meets_wild(PIDGEY);
    player.engine.save_memory_seen(&[0xFF]);
    player.engine.save_memory_seen(&[7]);
    checkpoint_if_wanted(&mut player);

    let checkpoint = player.storage.load_run().latest_checkpoint.unwrap();
    assert_eq!(checkpoint.patches.len(), 1, "the catch block: {:?}", checkpoint.patches);
    assert_eq!(checkpoint.patches[0].1, [1], "on");
}

#[test]
fn a_soft_reset_is_a_reload_so_a_dead_pokemon_back_in_the_party_is_no_violation() {
    let mut player = Player::starts_a_run(Ruleset {
        faint_is_dead: true,
        ..Ruleset::default()
    });
    // A second Pokémon, alive, so the Death isn't a wiped party.
    player.game.party = vec![
        party_pokemon(PIDGEY, 0xB1BD, "BIRDY"),
        party_pokemon(RATTATA, 0xB2, "RATTY"),
    ];
    player.game.play_time = 5_000;
    player.plays(5);
    let alive = player.game.party.clone();
    player.is_poisoned_to_faint(0xB1BD);
    player.plays(5);
    assert_eq!(player.party_pids(), [0xB2], "moved to the Dead box");

    // A+B+START+SELECT, then CONTINUE: the save from before the Death.
    player.game.pc.clear();
    player.game.party = alive;
    player.game.play_time = 4_000;
    player.plays(5);

    assert_eq!(player.party_pids(), [0xB2], "moved again");
    assert!(
        player.feed(FeedKind::Violation).is_empty(),
        "{:?}",
        player.feed(FeedKind::Violation)
    );
    assert!(
        player
            .feed(FeedKind::Run)
            .iter()
            .any(|e| e == "The game was reset to an older save")
    );
}

fn starter_source() -> SourceDef {
    SourceDef {
        key: "eevee".into(),
        name: "Eevee in Celadon".into(),
        kind: SourceKind::Gift,
        at: vec![0x0816_9C8B],
    }
}

#[test]
fn a_gift_received_again_after_a_reload_is_kept_when_the_first_is_gone() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.game.sources = vec![starter_source()];
    player.game.play_time = 5_000;
    player.plays(5);
    player.receives_gift(party_pokemon(133, 0x0EE1, "EEVEE"), 0x0816_9C8B);
    assert_eq!(player.party_pids(), [0x0EE1]);

    // The game is reset to a save from before the Gift, and it is received again.
    player.game.party.clear();
    player.game.play_time = 4_000;
    player.plays(5);
    player.receives_gift(party_pokemon(133, 0x0EE2, "EEVEE"), 0x0816_9C8B);
    player.plays(5);
    assert_eq!(player.party_pids(), [0x0EE2], "kept");
    assert!(player.feed(FeedKind::Gift).iter().all(|e| !e.contains("can't be kept")));
}

#[test]
fn without_a_reload_a_used_gift_is_still_declined() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.game.sources = vec![starter_source()];
    player.plays(5);
    player.receives_gift(party_pokemon(133, 0x0EE1, "EEVEE"), 0x0816_9C8B);
    player.game.party.clear(); // released
    player.plays(5);
    player.receives_gift(party_pokemon(133, 0x0EE2, "EEVEE"), 0x0816_9C8B);
    player.plays(5);
    assert!(
        player.feed(FeedKind::Gift)[..2]
            .iter()
            .any(|e| e.contains("that Gift's Encounter is used"))
    );
}

#[test]
fn a_pokemon_back_from_the_day_care_after_a_reboot_is_no_gift() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.plays(5);
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, 0xB1BD);
    // Left at the Day Care: gone from the party and PC.
    player.game.party.clear();
    player.plays(5);
    let storage = player.storage.clone();
    drop(player);

    let mut player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    player.plays(5);
    player.script_runs();
    player.game.party.push(party_pokemon(PIDGEY, 0xB1BD, "CAUGHT"));
    player.plays(5);
    player.script_ends();
    assert!(
        player.feed(FeedKind::Gift).is_empty(),
        "{:?}",
        player.feed(FeedKind::Gift)
    );
}

#[test]
fn an_egg_no_catalogue_lists_is_from_the_day_care_and_always_kept() {
    let mut player = Player::starts_a_run(Ruleset {
        gifts: false,
        ..Ruleset::default()
    });
    player.plays(5);
    let mut egg = party_pokemon(PIDGEY, 0x0E66, "EGG");
    egg.is_egg = true;
    player.receives_gift(egg, 0x0816_5000);
    player.plays(5);

    assert_eq!(player.party_pids(), [0x0E66], "kept with Gifts off");
    assert_eq!(
        player.feed(FeedKind::Gift)[0],
        "Pallet Town: received an Egg from the Day Care"
    );
}

#[test]
fn a_panic_on_the_way_out_is_a_crash_not_a_clean_close() {
    let storage = MemoryStorage::default();
    let kept = storage.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let mut player = Player::starts_a_run_in(Box::new(storage), TestClock::at(START_MS), Ruleset::default());
        checkpoint_if_wanted(&mut player);
        player.plays(20);
        player.engine.flush();
        panic!("the emulator thread fails");
    }));
    assert!(outcome.is_err());

    let player = Player::resumes(kept, TestClock::at(START_MS + 60_000));
    assert!(
        matches!(player.engine.boot_plan(), BootPlan::Rebuild { .. }),
        "{:?}",
        player.engine.boot_plan()
    );
}

#[test]
fn the_suspend_slot_is_kept_until_the_game_is_actually_back() {
    let mut player = Player::starts_a_run(Ruleset::default());
    checkpoint_if_wanted(&mut player);
    player.plays(40);
    player.engine.suspend(fake_core(40));
    let storage = player.storage.clone();
    drop(player);

    // A resume whose boot fails (the ROM is refused, say) never reports booting.
    let engine =
        pactlocke_run::RunEngine::resume(Box::new(storage.clone()), Box::new(TestClock::at(START_MS))).unwrap();
    assert!(matches!(engine.boot_plan(), BootPlan::Suspended(_)));
    drop(engine);

    let player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    assert!(
        matches!(player.engine.boot_plan(), BootPlan::Suspended(_)),
        "still there for the next try"
    );
}

#[test]
fn a_battle_under_way_when_the_game_was_suspended_isnt_decided_again() {
    let mut player = Player::starts_a_run(Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    });
    checkpoint_if_wanted(&mut player);
    player.walks_into(route_1());
    // Route 1's Encounter battle, suspended in the middle.
    player.meets_wild(RATTATA);
    let game = player.game.clone();
    player.engine.suspend(fake_core(player.engine.frames_run()));
    let storage = player.storage.clone();
    drop(player);

    let mut player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    player.game = game;
    player.plays(10);
    assert!(!player.game.catches_blocked, "the Encounter battle stays catchable");
    assert_eq!(
        player.feed(FeedKind::Encounter),
        ["Route 1: a wild RATTATA, this Area's Encounter"],
        "nothing decided twice"
    );
    assert!(player.feed(FeedKind::Block).is_empty());
}
