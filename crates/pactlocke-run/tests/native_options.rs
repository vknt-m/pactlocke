//! Seam 1: native options, played through the Run engine. Where a game does
//! a rule through its own option, Pactlocke sets the option at the first
//! overworld frame of each boot instead of enforcing the rule itself; a
//! watched option the Player changes is set back, and the feed says so.

mod support;

use pactlocke_game::{NativeOption, Rule};
use pactlocke_run::{FeedKind, NativeOptionView, Ruleset};
use support::*;

/// A game whose battle style option is its own set mode, watched.
fn native_set_mode() -> NativeOption {
    NativeOption {
        rule: Rule::SetMode,
        option: "battle_style".into(),
        name: "BATTLE STYLE".into(),
        menu: "OPTIONS ▸ BATTLE STYLE".into(),
        value: "set".into(),
        watched: true,
        with: Vec::new(),
        pactlocke_too: false,
    }
}

/// A game whose HARDCORE option bans items in battle, set once per boot.
fn native_no_items() -> NativeOption {
    NativeOption {
        rule: Rule::NoItems,
        option: "hardcore".into(),
        name: "HARDCORE".into(),
        menu: "OPTIONS ▸ HARDCORE".into(),
        value: "on".into(),
        watched: false,
        with: Vec::new(),
        pactlocke_too: false,
    }
}

/// Heart and Soul's FAINTING option: its cemetery, part of the Nuzlocke mode
/// used only when first Encounter is on too. Pactlocke keeps the Dead itself as well.
fn cemetery() -> NativeOption {
    NativeOption {
        rule: Rule::FaintIsDead,
        option: "fainting".into(),
        name: "FAINTING".into(),
        menu: "NUZLOCKE ▸ FAINTING".into(),
        value: "cemetery".into(),
        watched: true,
        with: vec![Rule::FirstEncounter],
        pactlocke_too: true,
    }
}

/// Heart and Soul's LEVEL CAP option: its cap code runs only with it on, and
/// Pactlocke's patch of that code gives the Run's cap.
fn cap_code_on() -> NativeOption {
    NativeOption {
        rule: Rule::LevelCap,
        option: "level_cap".into(),
        name: "LEVEL CAP".into(),
        menu: "DIFFICULTY ▸ LEVEL CAP".into(),
        value: "normal".into(),
        watched: true,
        with: Vec::new(),
        pactlocke_too: true,
    }
}

fn on_a_game_with(ruleset: Ruleset, natives: &[NativeOption]) -> Player {
    let mut player = Player::starts_a_run(ruleset);
    player.game.native_options = natives.to_vec();
    // The game has no way for Pactlocke to do these rules itself.
    player.game.has_set_mode = false;
    player.game.has_item_ban = false;
    player.game.options.insert("hardcore".into(), "off".into());
    player
        .game
        .option_values
        .insert("hardcore".into(), vec!["off".into(), "on".into()]);
    for (option, values) in [("fainting", ["release", "cemetery"]), ("level_cap", ["off", "normal"])] {
        player
            .game
            .option_values
            .insert(option.into(), values.iter().map(|v| (*v).to_owned()).collect());
    }
    player
}

#[test]
fn a_native_option_is_set_at_the_first_overworld_frame_and_set_back_when_changed() {
    let ruleset = Ruleset {
        set_mode: true,
        ..Ruleset::default()
    };
    let mut player = on_a_game_with(ruleset, &[native_set_mode()]);
    player.game.safe_point = false;
    player.plays(5);
    assert_eq!(player.game.options["battle_style"], "shift", "not before the overworld");

    player.game.safe_point = true;
    player.plays(1);
    assert_eq!(player.game.options["battle_style"], "set");
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "Set mode is the game's own BATTLE STYLE option, set to set"
    );
    assert!(
        player.feed(FeedKind::Problem).is_empty(),
        "Pactlocke doesn't try to force set mode itself: {:?}",
        player.feed(FeedKind::Problem)
    );

    player.game.options.insert("battle_style".into(), "shift".into());
    player.plays(1);
    assert_eq!(player.game.options["battle_style"], "set");
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "Set mode: the game's BATTLE STYLE was changed, so it is set back to set"
    );
}

#[test]
fn an_option_checked_only_at_the_start_is_set_once_per_boot() {
    let ruleset = Ruleset {
        no_items: true,
        ..Ruleset::default()
    };
    let mut player = on_a_game_with(ruleset, &[native_no_items()]);
    player.plays(1);
    assert_eq!(player.game.options["hardcore"], "on");
    assert!(!player.game.bag_locked, "the game bans items itself");

    player.game.options.insert("hardcore".into(), "off".into());
    player.plays(5);
    assert_eq!(player.game.options["hardcore"], "off", "not watched");
    let notes = player
        .feed(FeedKind::Run)
        .into_iter()
        .filter(|text| text.contains("HARDCORE"))
        .count();
    assert_eq!(notes, 1);
}

#[test]
fn a_native_option_for_a_rule_turned_off_is_left_alone() {
    let mut player = on_a_game_with(Ruleset::default(), &[native_set_mode(), native_no_items()]);
    player.plays(5);
    assert_eq!(player.game.options["battle_style"], "shift");
    assert_eq!(player.game.options["hardcore"], "off");
    assert!(player.engine.view().native_options.is_empty());
}

#[test]
fn the_view_shows_each_native_option_with_what_the_game_reads() {
    let ruleset = Ruleset {
        set_mode: true,
        ..Ruleset::default()
    };
    let mut player = on_a_game_with(ruleset, &[native_set_mode()]);
    player.plays(pactlocke_run::PARTY_LOOK_FRAMES as u32);
    assert_eq!(
        player.engine.view().native_options,
        vec![NativeOptionView {
            rule: "Set mode".into(),
            name: "BATTLE STYLE".into(),
            menu: "OPTIONS ▸ BATTLE STYLE".into(),
            value: "set".into(),
            read: Some("set".into()),
        }]
    );
}

#[test]
fn an_option_for_a_rule_with_another_is_set_only_when_both_are_on() {
    let alone = Ruleset {
        faint_is_dead: true,
        ..Ruleset::default()
    };
    let mut player = on_a_game_with(alone, &[cemetery()]);
    player.game.options.insert("fainting".into(), "release".into());
    player.plays(5);
    assert_eq!(
        player.game.options["fainting"], "release",
        "not without first Encounter"
    );
    assert!(player.engine.view().native_options.is_empty());

    let both = Ruleset {
        faint_is_dead: true,
        first_encounter: true,
        ..Ruleset::default()
    };
    let mut player = on_a_game_with(both, &[cemetery()]);
    player.game.options.insert("fainting".into(), "release".into());
    player.plays(1);
    assert_eq!(player.game.options["fainting"], "cemetery");
}

#[test]
fn an_option_pactlockes_own_method_needs_is_set_and_pactlocke_still_enforces_the_rule() {
    let ruleset = Ruleset {
        level_caps: Some(vec![pactlocke_game::LevelCap {
            name: "Falkner".into(),
            level: 11,
        }]),
        ..Ruleset::default()
    };
    let mut player = on_a_game_with(ruleset, &[cap_code_on()]);
    player.game.options.insert("level_cap".into(), "off".into());
    player.plays(1);
    assert_eq!(player.game.options["level_cap"], "normal");
    assert_eq!(player.game.level_cap, Some(11), "Pactlocke's cap, not the game's own");
    assert!(
        player
            .feed(FeedKind::Run)
            .contains(&"Level caps: Pactlocke sets the game's LEVEL CAP option to normal as well".to_owned()),
        "{:?}",
        player.feed(FeedKind::Run)
    );
}

fn set_mode_by_the_game() -> pactlocke_game::CapabilityReport {
    pactlocke_game::CapabilityReport::everything_enforceable().with(
        Rule::SetMode,
        pactlocke_game::Capability::NativeOption(native_set_mode()),
    )
}

#[test]
fn setup_shows_an_option_that_goes_with_a_rule_only_while_its_rules_are_on() {
    let report = pactlocke_game::CapabilityReport::everything_enforceable().with_rule_option(cemetery());
    let mut coop = support::coop::Coop::host("Red", report, pactlocke_run::Preset::Hardcore);
    coop.join("Blue").unwrap();
    coop.setup
        .edit(|r| {
            r.first_encounter = true;
            r.faint_is_dead = true;
        })
        .unwrap();
    coop.broadcast();
    let shown = |coop: &support::coop::Coop| coop.players[1].view.as_ref().unwrap().native_options.clone();
    assert_eq!(
        shown(&coop),
        vec![NativeOptionView {
            rule: "Faint = Dead".into(),
            name: "FAINTING".into(),
            menu: "NUZLOCKE ▸ FAINTING".into(),
            value: "cemetery".into(),
            read: None,
        }]
    );

    coop.setup.edit(|r| r.first_encounter = false).unwrap();
    coop.broadcast();
    assert!(shown(&coop).is_empty(), "not without first Encounter");
}

#[test]
fn setup_tells_every_player_which_option_the_game_does_a_rule_with() {
    let mut coop = support::coop::Coop::host("Red", set_mode_by_the_game(), pactlocke_run::Preset::Hardcore);
    coop.join("Blue").unwrap();
    assert_eq!(
        coop.setup.unavailable(Rule::SetMode),
        None,
        "the game's own option can be chosen"
    );
    assert!(coop.setup.ruleset().set_mode);

    let setting = NativeOptionView {
        rule: "Set mode".into(),
        name: "BATTLE STYLE".into(),
        menu: "OPTIONS ▸ BATTLE STYLE".into(),
        value: "set".into(),
        read: None,
    };
    for player in &coop.players {
        let view = player.view.as_ref().unwrap();
        assert_eq!(view.native_options, vec![setting.clone()], "{}", player.name);
    }

    coop.setup.edit(|r| r.set_mode = false).unwrap();
    coop.broadcast();
    assert!(coop.players[1].view.as_ref().unwrap().native_options.is_empty());
}
