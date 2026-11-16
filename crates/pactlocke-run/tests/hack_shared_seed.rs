//! Seam 1: Shared Seed through a Romhack's own
//! randomizer, played through the Run engine. Every Player's game is given
//! the Run's seed on the overworld (and given it back if it changes), and for
//! a hack whose trainer teams draw on the live RNG, the RNG is pinned from a
//! trainer battle's trigger until its party is made.

mod support;

use pactlocke_run::{FeedKind, Ruleset};
use support::*;

const SEED: u32 = 0x5EED_CAFE;

fn on_a_hack_with_its_own_randomizer(ruleset: Ruleset) -> Player {
    let mut player = Player::starts_a_run(ruleset);
    player.game.own_randomizer = true;
    player
}

fn shared() -> Ruleset {
    Ruleset {
        romhack_seed: Some(SEED),
        ..Ruleset::default()
    }
}

#[test]
fn the_runs_seed_is_given_to_the_game_on_the_overworld_and_given_back_if_it_changes() {
    let mut player = on_a_hack_with_its_own_randomizer(shared());
    player.game.safe_point = false;
    player.plays(5);
    assert_eq!(player.game.shared_seed, None, "not before the overworld");

    player.game.safe_point = true;
    player.plays(1);
    assert_eq!(player.game.shared_seed, Some(SEED));
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "Shared Seed: the game's own randomizer has the Run's seed"
    );

    player.game.shared_seed = Some(1);
    player.plays(1);
    assert_eq!(player.game.shared_seed, Some(SEED), "given back");
    let said = player
        .feed(FeedKind::Run)
        .iter()
        .filter(|line| line.starts_with("Shared Seed"))
        .count();
    assert_eq!(said, 1, "said once");
}

#[test]
fn the_rng_is_pinned_every_frame_while_a_trainer_battle_is_coming() {
    let mut player = on_a_hack_with_its_own_randomizer(shared());
    player.plays(3);
    assert!(player.game.rng_pins.is_empty());

    player.game.trainer_battle_coming = true;
    player.game.safe_point = false;
    player.plays(4);
    assert_eq!(player.game.rng_pins, [SEED; 4], "a pin at every frame boundary");
}

#[test]
fn setup_offers_shared_seed_only_on_a_game_with_its_own_randomizer() {
    use pactlocke_game::CapabilityReport;
    let hack = CapabilityReport::everything_enforceable().with_own_randomizer();
    let mut coop = support::coop::Coop::host("Red", hack, pactlocke_run::Preset::Hardcore);
    coop.join("Blue").unwrap();
    assert!(coop.setup.own_randomizer());
    coop.setup.edit(|r| r.romhack_seed = Some(SEED)).unwrap();
    assert_eq!(coop.setup.ruleset().romhack_seed, Some(SEED));

    let original = CapabilityReport::everything_enforceable();
    let mut coop = support::coop::Coop::host("Red", original, pactlocke_run::Preset::Hardcore);
    assert!(!coop.setup.own_randomizer());
    coop.setup.edit(|r| r.romhack_seed = Some(SEED)).unwrap();
    assert_eq!(
        coop.setup.ruleset().romhack_seed,
        None,
        "no randomizer of its own to seed"
    );
}

#[test]
fn without_shared_seed_the_game_seeds_itself() {
    let mut player = on_a_hack_with_its_own_randomizer(Ruleset::default());
    player.game.trainer_battle_coming = true;
    player.plays(5);
    assert_eq!(player.game.shared_seed, None);
    assert!(player.game.rng_pins.is_empty());
}
