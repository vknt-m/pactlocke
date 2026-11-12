//! Seam 1: level caps, played through the Run engine. The cap is patched in
//! at the first overworld frame, rewritten by a stamped patch whenever the
//! Player passes a stage, and where the game can't be patched it is a soft
//! cap: a Player over it can't ready up.

mod support;

use pactlocke_game::{LevelCap, PartyStats};
use pactlocke_run::{FeedKind, LevelCapView, Ruleset};
use support::*;

fn cap(name: &str, level: u8) -> LevelCap {
    LevelCap {
        name: name.into(),
        level,
    }
}

/// Caps as the Host left them: not the game's defaults.
fn capped() -> Ruleset {
    Ruleset {
        level_caps: Some(vec![cap("Brock", 12), cap("Misty", 20), cap("Champion", 60)]),
        ..Ruleset::default()
    }
}

fn cap_feed(player: &Player) -> Vec<String> {
    player
        .feed(FeedKind::Run)
        .into_iter()
        .filter(|text| text.contains("level cap") || text.contains("Level cap"))
        .collect()
}

fn with_level(species: u16, pid: u32, name: &str, level: u8) -> pactlocke_game::Pokemon {
    let mut pokemon = party_pokemon(species, pid, name);
    pokemon.stats = Some(PartyStats {
        level,
        hp: 20,
        max_hp: 20,
    });
    pokemon
}

#[test]
fn the_first_stages_cap_is_patched_in_on_the_overworld() {
    let mut player = Player::starts_a_run(capped());
    player.game.safe_point = false;
    player.plays(5);
    assert_eq!(player.game.level_cap, None, "not before the game is on the overworld");

    player.game.safe_point = true;
    player.plays(1);
    assert_eq!(player.game.level_cap, Some(12));
    assert_eq!(cap_feed(&player), ["Level cap 12 until Brock"]);
    assert_eq!(
        player.engine.view().level_cap,
        Some(LevelCapView {
            level: 12,
            next: Some("Brock".into()),
            soft: false,
        })
    );
}

#[test]
fn the_cap_rises_with_each_badge_by_a_patch_stamped_at_that_frame() {
    let mut player = Player::starts_a_run(capped());
    player.plays(5);
    let patches_before = player.engine.stamped().len();

    player.game.cap_stage = 1;
    player.plays(1);
    let at = player.engine.frames_run();
    assert_eq!(player.game.level_cap, Some(20));
    let new: Vec<_> = player.engine.stamped()[patches_before..].to_vec();
    assert_eq!(new.len(), 1, "{new:?}");
    assert_eq!(new[0].frame, at, "landed at the frame the badge was seen");
    assert_eq!(cap_feed(&player)[0], "Level cap raised to 20 until Misty");

    player.plays(10);
    assert_eq!(
        player.engine.stamped().len(),
        patches_before + 1,
        "only rewritten when it changes"
    );
}

#[test]
fn past_every_stage_there_is_no_cap() {
    let mut player = Player::starts_a_run(capped());
    player.plays(5);
    player.game.cap_stage = 3;
    player.plays(1);

    assert_eq!(player.game.level_cap, Some(100));
    assert_eq!(cap_feed(&player)[0], "No level cap any more: every cap is passed");
    assert_eq!(
        player.engine.view().level_cap,
        Some(LevelCapView {
            level: 100,
            next: None,
            soft: false,
        })
    );
}

#[test]
fn a_newly_booted_game_is_patched_again_without_a_new_feed_entry() {
    let mut player = Player::starts_a_run(capped());
    player.game.cap_stage = 1;
    player.plays(5);
    player.engine.flush();
    let storage = player.storage.clone();
    drop(player);

    let mut player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    player.game.cap_stage = 1;
    player.plays(5);

    assert_eq!(player.game.level_cap, Some(20), "the ROM patch isn't in the save");
    assert_eq!(cap_feed(&player), ["Level cap 20 until Misty"]);
}

#[test]
fn with_level_caps_off_nothing_is_capped() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.game.cap_stage = 1;
    player.plays(5);

    assert_eq!(player.game.level_cap, None);
    assert!(cap_feed(&player).is_empty());
    assert_eq!(player.engine.view().level_cap, None);
}

#[test]
fn a_game_without_cap_sites_has_a_soft_cap_and_says_so() {
    let mut player = Player::starts_a_run(capped());
    player.game.has_cap_sites = false;
    player.plays(5);

    assert_eq!(player.game.level_cap, None);
    assert_eq!(
        cap_feed(&player),
        [
            "Level cap 12 until Brock",
            "The level cap is a soft cap on this game (the Game Profile has no way to enforce level caps): \
             a Player with a Pokémon over the cap can't ready up at a Sync Gate"
        ]
    );
    assert!(player.engine.view().level_cap.as_ref().unwrap().soft);

    player.game.cap_stage = 1;
    player.plays(5);
    assert_eq!(
        cap_feed(&player).len(),
        3,
        "the cap still rises, and the soft cap is said once: {:?}",
        cap_feed(&player)
    );
}

#[test]
fn a_player_over_the_cap_cant_ready_up() {
    let mut player = Player::starts_a_run(capped());
    player.game.has_cap_sites = false;
    player.game.party = vec![
        with_level(CHARMANDER, 0xC4A2, "CHAR", 12),
        with_level(PIDGEY, 0xB1BD, "BIRDY", 13),
    ];
    player.plays(5);

    assert_eq!(
        player.engine.ready_up_refusal(&mut player.game),
        Some("BIRDY is level 13, over the level cap of 12".into())
    );

    // A Pokémon at the cap is fine.
    player.game.party.pop();
    assert_eq!(player.engine.ready_up_refusal(&mut player.game), None);
}

#[test]
fn without_level_caps_anyone_can_ready_up() {
    let mut player = Player::starts_a_run(Ruleset::default());
    player.game.party = vec![with_level(CHARMANDER, 0xC4A2, "CHAR", 70)];
    player.plays(5);

    assert_eq!(player.engine.ready_up_refusal(&mut player.game), None);
}

#[test]
fn a_new_stage_at_the_same_level_is_still_announced() {
    let mut player = Player::starts_a_run(Ruleset {
        level_caps: Some(vec![cap("Koga", 43), cap("Sabrina", 43)]),
        ..Ruleset::default()
    });
    player.plays(5);
    player.game.cap_stage = 1;
    player.plays(5);

    assert_eq!(cap_feed(&player)[0], "Level cap raised to 43 until Sabrina");
    assert_eq!(
        player.engine.view().level_cap.as_ref().unwrap().next,
        Some("Sabrina".into())
    );
}

#[test]
fn the_cap_never_falls_back_when_the_games_league_flags_are_cleared() {
    let mut player = Player::starts_a_run(capped());
    player.game.cap_stage = 2;
    player.plays(5);
    // The game clears the League's defeat flags when the Player goes back in.
    player.game.cap_stage = 1;
    player.plays(5);

    assert_eq!(player.game.level_cap, Some(60));
    assert_eq!(cap_feed(&player), ["Level cap 60 until Champion"]);
}
