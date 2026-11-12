//! Seam 1: must nickname and the type clause, played through the Run engine.

mod support;

use pactlocke_emu::Keys;
use pactlocke_game::{SourceDef, SourceKind};
use pactlocke_run::{FeedKind, NicknameWanted, Ruleset};
use support::*;

const BULBASAUR: u16 = 1;
const STARTER_AT: u32 = 0x0816_9C8B;
const STARTER: u32 = 0x5747_0001;
const NEW_PIDGEY: u32 = 0xB1BD_0002;
const GRASS: u8 = 12;
const NORMAL: u8 = 0;

fn must_nickname() -> Ruleset {
    Ruleset {
        must_nickname: true,
        ..Ruleset::default()
    }
}

fn type_clause() -> Ruleset {
    Ruleset {
        type_clause: true,
        ..Ruleset::default()
    }
}

/// A Player whose game has a starter Gift, BULBASAUR, and Normal-type
/// PIDGEY and RATTATA.
fn player(ruleset: Ruleset) -> Player {
    let mut player = Player::starts_a_run(ruleset);
    player.game.sources = vec![SourceDef {
        key: "starter".into(),
        name: "Starter".into(),
        kind: SourceKind::Starter,
        at: vec![STARTER_AT],
    }];
    player.game.species_names.insert(BULBASAUR, "BULBASAUR".into());
    player.game.primary_types.insert(BULBASAUR, GRASS);
    player.game.primary_types.insert(PIDGEY, NORMAL);
    player.game.primary_types.insert(RATTATA, NORMAL);
    player.game.type_names.insert(GRASS, "GRASS".into());
    player.game.type_names.insert(NORMAL, "NORMAL".into());
    player.plays(1);
    player
}

fn wanted(player: &Player) -> Vec<NicknameWanted> {
    player.engine.view().nickname_wanted.clone()
}

fn wants(pid: u32, species_name: &str) -> NicknameWanted {
    NicknameWanted {
        pid,
        species_name: species_name.into(),
    }
}

/// A wild PIDGEY caught and left with its species name.
fn catches_an_unnamed_pidgey(player: &mut Player) {
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.game.party.push(party_pokemon(PIDGEY, NEW_PIDGEY, "PIDGEY"));
    player.plays(10);
    player.leaves_battle();
}

#[test]
fn while_the_nickname_prompt_is_up_b_and_down_are_dropped_and_the_cursor_kept_on_yes() {
    let mut player = player(must_nickname());
    let pressed = Keys::A | Keys::B | Keys::DOWN | Keys::UP;
    assert_eq!(player.engine.keys_for_frame(pressed, &mut player.game), pressed);

    player.game.nickname_prompt_up = true;
    assert_eq!(
        player.engine.keys_for_frame(pressed, &mut player.game),
        Keys::A | Keys::UP
    );
    assert_eq!(player.game.nickname_cursor_kept, 1);
    let stamped = player.engine.stamped().last().unwrap().frame;
    assert_eq!(
        stamped,
        player.engine.frames_run(),
        "stamped at the boundary before the frame"
    );
}

#[test]
fn without_must_nickname_the_prompt_can_be_declined() {
    let mut player = player(Ruleset::default());
    player.game.nickname_prompt_up = true;
    let pressed = Keys::B | Keys::DOWN;
    assert_eq!(player.engine.keys_for_frame(pressed, &mut player.game), pressed);
    assert_eq!(player.game.nickname_cursor_kept, 0);
}

#[test]
fn a_catch_left_with_its_species_name_is_named_from_the_side_panel() {
    let mut player = player(must_nickname());
    catches_an_unnamed_pidgey(&mut player);

    assert_eq!(wanted(&player), [wants(NEW_PIDGEY, "PIDGEY")]);
    assert_eq!(
        player.feed(FeedKind::Run)[0],
        "Must nickname: PIDGEY needs a nickname, so name it in the side panel"
    );

    player.engine.name_pokemon(NEW_PIDGEY, "Birdy", &mut player.game);
    player.plays(1);
    assert_eq!(player.game.party[0].nickname, "Birdy");
    assert!(wanted(&player).is_empty());
    assert_eq!(player.feed(FeedKind::Run)[0], "Named PIDGEY Birdy");
}

#[test]
fn a_catch_with_a_nickname_needs_nothing() {
    let mut player = player(must_nickname());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);

    assert!(wanted(&player).is_empty());
}

#[test]
fn the_species_name_or_a_blank_name_is_refused_and_still_wanted() {
    let mut player = player(must_nickname());
    catches_an_unnamed_pidgey(&mut player);

    player.engine.name_pokemon(NEW_PIDGEY, "PIDGEY", &mut player.game);
    player.engine.name_pokemon(NEW_PIDGEY, "  ", &mut player.game);

    assert_eq!(player.game.party[0].nickname, "PIDGEY");
    assert_eq!(wanted(&player), [wants(NEW_PIDGEY, "PIDGEY")]);
    assert_eq!(
        player.feed(FeedKind::Problem)[..2],
        [
            "Couldn't name PIDGEY \"\": a nickname can't be blank or the species name",
            "Couldn't name PIDGEY \"PIDGEY\": a nickname can't be blank or the species name",
        ]
    );
}

#[test]
fn a_gift_left_unnamed_is_asked_for_once_its_script_ends() {
    let mut player = player(must_nickname());
    player.script_at_gift(STARTER_AT);
    player.game.party.push(party_pokemon(BULBASAUR, STARTER, "BULBASAUR"));
    player.plays(10);
    assert!(wanted(&player).is_empty(), "the script may still be naming it");

    player.script_ends();
    assert_eq!(wanted(&player), [wants(STARTER, "BULBASAUR")]);
}

#[test]
fn a_pokemon_in_the_pc_is_named_there() {
    let mut player = player(must_nickname());
    player.script_at_gift(STARTER_AT);
    player.game.pc.push(pactlocke_game::StoredPokemon {
        box_number: 1,
        slot: 1,
        pokemon: party_pokemon(BULBASAUR, STARTER, "BULBASAUR"),
    });
    player.plays(10);
    player.script_ends();

    player.engine.name_pokemon(STARTER, "Bulby", &mut player.game);
    player.plays(1);
    assert_eq!(player.game.pc[0].pokemon.nickname, "Bulby");
    assert!(wanted(&player).is_empty());
}

#[test]
fn a_wanted_nickname_is_still_wanted_after_a_resume() {
    let mut player = player(must_nickname());
    catches_an_unnamed_pidgey(&mut player);
    player.engine.flush();
    let storage = player.storage.clone();
    drop(player);

    let player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    assert_eq!(wanted(&player), [wants(NEW_PIDGEY, "PIDGEY")]);
}

#[test]
fn a_second_pokemon_of_a_primary_type_in_the_party_is_moved_to_the_pc_with_the_reason() {
    let mut player = player(type_clause());
    player.game.party = vec![party_pokemon(RATTATA, 0x0000_0BAD, "RATS")];
    player.plays(5);

    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);

    assert_eq!(player.party_pids(), [0x0000_0BAD], "the holder stays");
    assert_eq!(player.game.pc[0].pokemon.pid, NEW_PIDGEY);
    assert_eq!(player.game.pc[0].box_number, 1);
    assert_eq!(
        player.feed(FeedKind::Block)[0],
        "Type clause: your RATS is the NORMAL type in your party, so CAUGHT went to box 1"
    );

    // Put in the party anyway: moved back.
    player.withdraws(NEW_PIDGEY);
    assert_eq!(player.party_pids(), [0x0000_0BAD]);
    assert_eq!(player.feed(FeedKind::Block).len(), 2);
}

#[test]
fn a_pokemon_of_a_type_nobody_holds_joins() {
    let mut player = player(type_clause());
    player.game.party = vec![party_pokemon(BULBASAUR, STARTER, "BULBA")];
    player.plays(5);
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);

    assert_eq!(player.party_pids(), [STARTER, NEW_PIDGEY]);
    assert!(player.feed(FeedKind::Block).is_empty());
}

#[test]
fn once_the_holder_dies_one_of_its_type_may_join() {
    let mut player = player(Ruleset {
        type_clause: true,
        faint_is_dead: true,
        ..Ruleset::default()
    });
    player.game.party = vec![
        party_pokemon(BULBASAUR, STARTER, "BULBA"),
        party_pokemon(RATTATA, 0x0000_0BAD, "RATS"),
    ];
    player.plays(5);
    player.is_poisoned_to_faint(0x0000_0BAD);
    player.plays(5);
    assert_eq!(player.party_pids(), [STARTER], "the Dead holder went to the Dead box");

    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);
    assert_eq!(player.party_pids(), [STARTER, NEW_PIDGEY]);
}

#[test]
fn once_the_holder_leaves_the_party_one_of_its_type_may_join() {
    let mut player = player(type_clause());
    player.game.party = vec![party_pokemon(RATTATA, 0x0000_0BAD, "RATS")];
    player.plays(5);
    // Deposited in the PC.
    let rats = player.game.party.remove(0);
    player.game.pc.push(pactlocke_game::StoredPokemon {
        box_number: 1,
        slot: 1,
        pokemon: rats,
    });
    player.plays(5);

    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);
    assert_eq!(player.party_pids(), [NEW_PIDGEY]);
}

#[test]
fn eggs_hold_no_type() {
    let mut player = player(type_clause());
    let mut egg = party_pokemon(RATTATA, 0x0000_0E66, "EGG");
    egg.is_egg = true;
    player.game.party = vec![egg];
    player.plays(5);
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);

    assert_eq!(player.party_pids(), [0x0000_0E66, NEW_PIDGEY]);
}

#[test]
fn without_the_type_clause_a_party_can_share_types() {
    let mut player = player(Ruleset::default());
    player.game.party = vec![party_pokemon(RATTATA, 0x0000_0BAD, "RATS")];
    player.plays(5);
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);

    assert_eq!(player.party_pids(), [0x0000_0BAD, NEW_PIDGEY]);
}

#[test]
fn a_name_given_mid_battle_waits_for_the_overworld() {
    let mut player = player(must_nickname());
    catches_an_unnamed_pidgey(&mut player);
    player.meets_wild(RATTATA);

    player.engine.name_pokemon(NEW_PIDGEY, "Birdy", &mut player.game);
    player.plays(5);
    assert_eq!(player.game.party[0].nickname, "PIDGEY", "not while the battle runs");

    player.leaves_battle();
    assert_eq!(player.game.party[0].nickname, "Birdy");
}

#[test]
fn a_name_given_in_the_game_meanwhile_is_taken() {
    let mut player = player(must_nickname());
    catches_an_unnamed_pidgey(&mut player);
    // The Name Rater, say.
    player.game.party[0].nickname = "PIDGE".into();
    player.plays(60);

    assert!(wanted(&player).is_empty());
    assert_eq!(player.feed(FeedKind::Run)[0], "Named PIDGEY PIDGE");
}

#[test]
fn a_catch_not_yet_looked_at_when_the_game_closed_is_looked_at_on_resume() {
    let mut player = player(must_nickname());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.game.party.push(party_pokemon(PIDGEY, NEW_PIDGEY, "PIDGEY"));
    player.plays(10);
    // The battle ends, but the game closes before a quiet moment on the overworld.
    player.game.battle = None;
    player.game.safe_point = false;
    player.plays(5);
    let party = player.game.party.clone();
    player.engine.flush();
    let storage = player.storage.clone();
    drop(player);

    let mut player = Player::resumes(storage, TestClock::at(START_MS + 60_000));
    player.game.party = party;
    player.plays(5);
    assert_eq!(wanted(&player), [wants(NEW_PIDGEY, "PIDGEY")]);
}

#[test]
fn the_type_clause_passes_over_the_dead_box() {
    let mut player = player(type_clause());
    player.game.party = vec![party_pokemon(RATTATA, 0x0000_0BAD, "RATS")];
    // Boxes 1 to 13 are full.
    for box_number in 1..=13 {
        for slot in 1..=30 {
            player.game.pc.push(pactlocke_game::StoredPokemon {
                box_number,
                slot,
                pokemon: party_pokemon(
                    BULBASAUR,
                    0x1000_0000 + u32::from(box_number) * 100 + u32::from(slot),
                    "FILL",
                ),
            });
        }
    }
    player.plays(5);
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, NEW_PIDGEY);

    assert!(
        !player.dead_box_pids().contains(&NEW_PIDGEY),
        "a living Pokémon never goes to the Dead box"
    );
    assert!(player.party_pids().contains(&NEW_PIDGEY));
    assert!(player.feed(FeedKind::Violation)[0].starts_with("Violation: CAUGHT is a second NORMAL type"));
}

#[test]
fn a_party_member_that_evolves_into_a_held_type_stays() {
    let mut player = player(type_clause());
    player.game.party = vec![
        party_pokemon(RATTATA, 0x0000_0BAD, "RATS"),
        party_pokemon(BULBASAUR, STARTER, "BULBA"),
    ];
    player.plays(5);
    // BULBA's species is now a Normal type, as if by evolution.
    player.game.party[1].species = PIDGEY;
    player.plays(5);

    assert_eq!(player.party_pids(), [0x0000_0BAD, STARTER], "only newcomers are moved");
}
