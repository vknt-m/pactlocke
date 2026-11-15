//! Seam 1: Gifts, trades and Statics, played through the Run engine.

mod support;

use pactlocke_game::{SourceDef, SourceKind};
use pactlocke_run::{FeedKind, Ruleset};
use support::*;

const BULBASAUR: u16 = 1;
const ABRA: u16 = 63;
const SNORLAX: u16 = 143;
const STARTER_AT: u32 = 0x0816_9C8B;
const PRIZE_AT: u32 = 0x0816_CCD4;
const SNORLAX_AT: u32 = 0x0816_8048;
const STARTER: u32 = 0x5747_0001;

fn rules(gifts: bool, statics: bool) -> Ruleset {
    Ruleset {
        first_encounter: true,
        gifts,
        statics,
        ..Ruleset::default()
    }
}

fn source(key: &str, name: &str, kind: SourceKind, at: u32) -> SourceDef {
    SourceDef {
        key: key.into(),
        name: name.into(),
        kind,
        at: vec![at],
    }
}

/// A Player in a game with a small Gift and Static catalogue.
fn player(ruleset: Ruleset) -> Player {
    let mut player = Player::starts_a_run(ruleset);
    player.game.sources = vec![
        source("starter", "Starter", SourceKind::Starter, STARTER_AT),
        source(
            "game_corner_prize_1",
            "Celadon Game Corner prize",
            SourceKind::Gift,
            PRIZE_AT,
        ),
        source(
            "snorlax_route_12",
            "Snorlax on Route 12",
            SourceKind::Static,
            SNORLAX_AT,
        ),
    ];
    player.game.species_names.insert(BULBASAUR, "BULBASAUR".into());
    player.game.species_names.insert(ABRA, "ABRA".into());
    player.game.species_names.insert(SNORLAX, "SNORLAX".into());
    player.plays(1);
    player
}

#[test]
fn the_starter_is_logged_as_the_starter_gift() {
    let mut player = player(rules(true, false));
    player.receives_gift(party_pokemon(BULBASAUR, STARTER, "BULBASAUR"), STARTER_AT + 0x1E);

    assert_eq!(
        player.feed(FeedKind::Gift),
        ["Pallet Town: received BULBASAUR, the Starter"]
    );
    assert_eq!(player.party_pids(), [STARTER]);
}

fn with_starter(ruleset: Ruleset) -> Player {
    let mut player = player(ruleset);
    player.receives_gift(party_pokemon(BULBASAUR, STARTER, "BULBASAUR"), STARTER_AT + 0x1E);
    player
}

#[test]
fn a_listed_gift_is_kept_when_gifts_are_allowed() {
    let mut player = with_starter(rules(true, false));
    player.receives_gift(party_pokemon(ABRA, 0xAB, "ABRA"), PRIZE_AT + 0x20);

    assert_eq!(
        player.feed(FeedKind::Gift)[0],
        "Pallet Town: received ABRA from the Celadon Game Corner prize"
    );
    assert_eq!(player.party_pids(), [STARTER, 0xAB]);
}

#[test]
fn with_gifts_off_a_gift_is_released_but_the_starter_is_kept() {
    let mut player = with_starter(rules(false, false));
    player.receives_gift(party_pokemon(ABRA, 0xAB, "ABRA"), PRIZE_AT + 0x20);

    assert_eq!(player.party_pids(), [STARTER], "released once the script ended");
    assert_eq!(
        player.feed(FeedKind::Gift)[..2],
        [
            "Released ABRA",
            "ABRA can't be kept: this Ruleset allows no Gifts but the starter, so it is released",
        ]
    );
}

#[test]
fn a_gift_is_released_only_once_the_script_ends() {
    let mut player = with_starter(rules(false, false));
    player.game.script = pactlocke_game::ScriptContext {
        status: pactlocke_game::ScriptStatus::Running,
        position: Some(PRIZE_AT + 0x20),
    };
    player.game.party.push(party_pokemon(ABRA, 0xAB, "ABRA"));
    player.plays(30);
    assert_eq!(player.party_pids(), [STARTER, 0xAB], "a script may be using its slot");

    player.script_ends();
    assert_eq!(player.party_pids(), [STARTER]);
}

#[test]
fn a_second_pokemon_from_the_same_gift_is_released() {
    let mut player = with_starter(rules(true, false));
    player.receives_gift(party_pokemon(ABRA, 0xAB, "ABRA"), PRIZE_AT + 0x20);
    player.receives_gift(party_pokemon(ABRA, 0xAC, "ABRA"), PRIZE_AT + 0x20);

    assert_eq!(player.party_pids(), [STARTER, 0xAB]);
    assert!(
        player
            .feed(FeedKind::Gift)
            .contains(&"ABRA can't be kept: that Gift's Encounter is used, so it is released".to_owned())
    );
}

#[test]
fn a_gift_sent_to_the_pc_is_found_and_released_from_there() {
    let mut player = with_starter(rules(false, false));
    player.receives_gift_in_pc(party_pokemon(ABRA, 0xAB, "ABRA"), PRIZE_AT + 0x20);

    assert!(player.game.pc.is_empty(), "released from the PC");
    assert!(player.feed(FeedKind::Gift).contains(&"Released ABRA".to_owned()));
}

#[test]
fn a_pokemon_given_by_an_unlisted_script_is_an_unknown_gift_and_kept() {
    let mut player = with_starter(rules(false, false));
    player.receives_gift(party_pokemon(ABRA, 0xAB, "ABRA"), 0x0817_0000);

    assert_eq!(
        player.feed(FeedKind::Gift)[0],
        "Pallet Town: received ABRA, an unknown Gift (the Game Profile doesn't list it)"
    );
    assert_eq!(player.party_pids(), [STARTER, 0xAB]);
}

#[test]
fn an_in_game_trade_is_a_gift() {
    let mut player = with_starter(rules(true, false));
    player.receives_gift(party_pokemon(ABRA, 0xAB, "KADABRA?"), PRIZE_AT + 0x20);
    player.trades(1, party_pokemon(SNORLAX, 0x5A, "SNORLAX"));

    assert_eq!(
        player.feed(FeedKind::Gift)[0],
        "Pallet Town: traded KADABRA? for SNORLAX, a Gift"
    );
    assert_eq!(player.party_pids(), [STARTER, 0x5A]);
}

#[test]
fn with_gifts_off_a_traded_in_pokemon_is_released() {
    let mut player = with_starter(rules(false, false));
    player.trades(0, party_pokemon(ABRA, 0xAB, "ABRA"));
    assert!(player.party_pids().is_empty());
}

#[test]
fn a_pokemon_back_from_the_day_care_is_not_a_gift() {
    let mut player = with_starter(rules(false, false));
    let starter = player.game.party.remove(0);
    player.script_runs();
    player.script_ends();
    player.receives_gift(starter, 0x0817_0000);

    assert_eq!(player.party_pids(), [STARTER]);
    assert_eq!(player.feed(FeedKind::Gift).len(), 1, "only the starter");
}

#[test]
fn a_catch_is_not_a_gift() {
    let mut player = with_starter(rules(false, false));
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, 0x0000_B1BD);
    player.script_runs();
    player.script_ends();

    assert_eq!(player.party_pids(), [STARTER, 0x0000_B1BD]);
    assert_eq!(player.feed(FeedKind::Gift).len(), 1);
}

#[test]
fn with_statics_on_a_static_is_a_bonus_encounter_even_in_a_closed_area() {
    let mut player = with_starter(rules(true, true));
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();

    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    assert!(!player.game.catches_blocked);
    assert_eq!(
        player.feed(FeedKind::Encounter)[0],
        "Route 1: SNORLAX, the Snorlax on Route 12: a bonus Static Encounter"
    );
    player.leaves_battle();
    assert_eq!(
        player.engine.view().closed_areas(),
        ["Route 1"],
        "the Area's own Encounter is unchanged"
    );
}

#[test]
fn a_static_fled_from_has_used_its_encounter() {
    let mut player = with_starter(rules(true, true));
    player.walks_into(route_1());
    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    player.leaves_battle();

    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    assert!(player.game.catches_blocked);
    assert_eq!(
        player.feed(FeedKind::Block)[0],
        "Route 1: SNORLAX, the Snorlax on Route 12: this Static's Encounter is used, so no catching it (running is fine)"
    );
}

#[test]
fn with_statics_off_a_static_in_a_closed_area_cant_be_caught() {
    let mut player = with_starter(rules(true, false));
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();

    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    assert!(player.game.catches_blocked);
}

#[test]
fn the_statics_rule_does_nothing_without_first_encounter() {
    let mut player = with_starter(Ruleset {
        statics: true,
        gifts: true,
        ..Ruleset::default()
    });
    player.walks_into(route_1());
    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    assert!(!player.game.catches_blocked);
    assert!(player.feed(FeedKind::Encounter).is_empty());
}

#[test]
fn a_used_gift_stays_used_after_a_reboot_and_a_static_comes_back() {
    let mut player = with_starter(rules(true, true));
    player.receives_gift(party_pokemon(ABRA, 0xAB, "ABRA"), PRIZE_AT + 0x20);
    player.walks_into(route_1());
    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    player.leaves_battle();
    let (storage, clock) = (player.storage.clone(), player.clock.clone());
    let game = player.game.clone();
    drop(player);

    let mut player = Player::resumes(storage, clock);
    player.game = game;
    player.plays(1);
    player.receives_gift(party_pokemon(ABRA, 0xAC, "ABRA"), PRIZE_AT + 0x20);
    assert!(
        !player.party_pids().contains(&0xAC),
        "the prize's Encounter was used before the reboot"
    );
    // A reboot from the in-game save is a reset: the Static is back in the game.
    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    assert!(!player.game.catches_blocked);
}

#[test]
fn a_static_met_before_a_reset_to_an_older_save_is_an_encounter_again() {
    let mut player = with_starter(rules(true, true));
    player.game.play_time = 5_000;
    player.walks_into(route_1());
    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    player.leaves_battle();

    // Soft reset to a save from before Snorlax.
    player.game.play_time = 4_000;
    player.plays(5);
    player.meets_static(SNORLAX, SNORLAX_AT + 7);
    assert!(!player.game.catches_blocked);
    assert_eq!(player.feed(FeedKind::Encounter).len(), 2);
}

#[test]
fn a_gift_given_from_inside_a_called_script_is_still_matched() {
    let mut player = with_starter(rules(false, false));
    // The prize script has called the game's shared "sent to the PC" script.
    player.game.script_calls = vec![0x081A_5010, PRIZE_AT + 0x24];
    player.receives_gift(party_pokemon(ABRA, 0xAB, "ABRA"), 0x081A_5000);

    assert_eq!(player.party_pids(), [STARTER], "released: Gifts are off");
    assert!(
        player
            .feed(FeedKind::Gift)
            .contains(&"Pallet Town: received ABRA from the Celadon Game Corner prize".to_owned())
    );
}

#[test]
fn a_battle_nobody_can_catch_in_is_no_ones_encounter() {
    let mut player = with_starter(rules(true, false));
    player.walks_into(route_1());
    // The catching demo or the Pokémon Tower ghost: a script starts it and waits.
    player.game.script = pactlocke_game::ScriptContext {
        status: pactlocke_game::ScriptStatus::Waiting,
        position: Some(0x0817_0000),
    };
    player.game.opponent_party = vec![wild_pokemon(SNORLAX)];
    let mut battle = wild_battle(SNORLAX);
    battle.kind = pactlocke_game::BattleKind::Uncatchable;
    player.game.battle = Some(battle);
    player.plays(10);
    player.leaves_battle();

    assert!(player.feed(FeedKind::Encounter).is_empty());
    assert!(player.engine.view().closed_areas().is_empty(), "Route 1 is still open");
}
