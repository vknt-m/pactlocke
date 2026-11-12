//! Seam 1: no items and set mode, played through the Run engine. Both are
//! put in place at the first overworld frame of each boot, before any battle:
//! the item patches only bite in battle, and the battle style is written
//! back whenever the Player changes it.

mod support;

use pactlocke_run::{FeedKind, Ruleset};
use support::*;

fn no_items() -> Ruleset {
    Ruleset {
        no_items: true,
        ..Ruleset::default()
    }
}

fn set_mode() -> Ruleset {
    Ruleset {
        set_mode: true,
        ..Ruleset::default()
    }
}

fn battle_style(player: &Player) -> &str {
    &player.game.options["battle_style"]
}

#[test]
fn no_items_locks_the_bag_and_bans_wild_items_from_the_first_overworld_frame() {
    let mut player = Player::starts_a_run(no_items());
    player.game.safe_point = false;
    player.plays(5);
    assert!(
        !player.game.bag_locked && !player.game.wild_items_banned,
        "not before the overworld"
    );

    player.game.safe_point = true;
    player.plays(1);
    assert!(player.game.bag_locked, "the Bag is refused in trainer battles");
    assert!(player.game.wild_items_banned, "only balls work in wild battles");
    let stamped = player.engine.stamped().to_vec();
    assert_eq!(stamped.len(), 2, "{stamped:?}");
    assert!(stamped.iter().all(|s| s.frame == player.engine.frames_run()));
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "No items in battle: the Bag is refused in trainer battles, and only Poké Balls work in wild battles"
    );

    // Through battles and back, nothing is patched again.
    player.meets_wild(RATTATA);
    player.leaves_battle();
    assert_eq!(player.engine.stamped().len(), 2);
    assert!(player.game.bag_locked && player.game.wild_items_banned);
}

#[test]
fn a_newly_booted_game_is_patched_again_without_a_new_feed_entry() {
    let mut player = Player::starts_a_run(no_items());
    player.plays(5);
    player.engine.flush();
    let storage = player.storage.clone();
    drop(player);

    let mut player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    player.plays(5);

    assert!(
        player.game.bag_locked && player.game.wild_items_banned,
        "ROM patches aren't in the save"
    );
    let entries: Vec<String> = player
        .feed(FeedKind::Run)
        .into_iter()
        .filter(|text| text.starts_with("No items"))
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
}

#[test]
fn a_game_that_cant_ban_items_says_so_once() {
    let mut player = Player::starts_a_run(no_items());
    player.game.has_item_ban = false;
    player.plays(5);

    assert!(!player.game.bag_locked && !player.game.wild_items_banned);
    assert_eq!(
        player.feed(FeedKind::Problem),
        ["Couldn't ban items in battle: the Game Profile has no way to enforce no items in trainer battles"]
    );
}

#[test]
fn without_no_items_the_bag_works() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.plays(5);
    assert!(!player.game.bag_locked && !player.game.wild_items_banned);
    assert!(player.engine.stamped().is_empty());
}

#[test]
fn set_mode_forces_the_battle_style_on_the_overworld_and_again_whenever_it_is_changed() {
    let mut player = Player::starts_a_run(set_mode());
    player.game.safe_point = false;
    player.plays(5);
    assert_eq!(battle_style(&player), "shift", "not before the overworld");

    player.game.safe_point = true;
    player.plays(1);
    assert_eq!(battle_style(&player), "set");
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "Set mode: the battle style is set to Set"
    );

    player.plays(10);
    let writes = player.engine.stamped().len();
    assert_eq!(writes, 1, "only written when it isn't Set");

    // The Player switches it back in the OPTION menu.
    player.game.options.insert("battle_style".into(), "shift".into());
    player.plays(1);
    assert_eq!(battle_style(&player), "set");
    assert_eq!(player.engine.stamped().len(), 2);
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "Set mode: the battle style was changed, so it is set back to Set"
    );
}

#[test]
fn a_game_with_no_set_mode_says_so_once() {
    let mut player = Player::starts_a_run(set_mode());
    player.game.has_set_mode = false;
    player.plays(5);

    assert_eq!(battle_style(&player), "shift");
    assert_eq!(
        player.feed(FeedKind::Problem),
        ["Couldn't force set mode: the Game Profile has no way to enforce set mode"]
    );
}

#[test]
fn without_set_mode_the_battle_style_is_left_alone() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.plays(5);
    assert_eq!(battle_style(&player), "shift");
}
