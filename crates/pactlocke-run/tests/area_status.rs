//! Seam 1: the Area status, what the side panel and the game screen show of
//! the Area this Player is in, played through the Run engine on the fake game.

mod support;

use pactlocke_run::{AreaState, AreaStatus, PARTY_LOOK_FRAMES, Ruleset};
use support::*;

fn first_encounter() -> Ruleset {
    Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    }
}

/// The Area status once the engine has looked at where the Player is.
fn status(player: &mut Player) -> Option<AreaStatus> {
    player.plays(PARTY_LOOK_FRAMES as u32);
    player.engine.view().area_status.clone()
}

/// Each wild battle since the Encounter as "SPECIES: what happened", newest first.
fn battles(status: &AreaStatus) -> Vec<String> {
    status
        .battles
        .iter()
        .map(|b| format!("{}: {}", b.species_name, b.what()))
        .collect()
}

#[test]
fn a_new_area_allows_a_catch_until_its_encounter_then_says_it_is_done() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    let open = status(&mut player).expect("Route 1 is listed");
    assert_eq!((open.area.as_str(), open.state), ("Route 1", AreaState::CatchAllowed));
    assert_eq!(open.say(), "Catch allowed");
    assert_eq!(open.why(), "The first wild battle here is this Area's Encounter.");
    assert!(open.encounter.is_none());

    player.meets_wild(PIDGEY);
    player.catches(PIDGEY, 0xCA7C_0001);
    let done = status(&mut player).unwrap();
    assert_eq!(done.state, AreaState::EncounterDone);
    assert_eq!(done.say(), "Encounter done");
    assert_eq!(
        done.why(),
        "Caught PIDGEY here. Ball throws are refused; running is fine."
    );
    let encounter = done.encounter.as_ref().expect("the Encounter shows");
    assert_eq!((encounter.species, encounter.caught), (PIDGEY, true));
    assert!(done.battles.is_empty());
}

#[test]
fn battles_after_the_encounter_show_newest_first_and_no_feed_line_each() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    for species in [RATTATA, PIDGEY] {
        player.meets_wild(species);
        player.leaves_battle();
    }
    let done = status(&mut player).unwrap();
    assert_eq!(
        done.why(),
        "Met PIDGEY here, not caught. Ball throws are refused; running is fine."
    );
    assert_eq!(
        battles(&done),
        [
            "PIDGEY: met after the Encounter, not caught",
            "RATTATA: met after the Encounter, not caught"
        ]
    );
    assert_eq!(done.met, 3, "every wild battle here, the Encounter's too");
    assert!(
        player.feed(pactlocke_run::FeedKind::Block).is_empty(),
        "no feed line for a battle in a used Area"
    );
}

#[test]
fn the_card_keeps_only_the_newest_battles() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    for _ in 0..pactlocke_run::RECENT_BATTLES + 2 {
        player.meets_wild(RATTATA);
        player.leaves_battle();
    }
    let done = status(&mut player).unwrap();
    assert_eq!(done.battles.len(), pactlocke_run::RECENT_BATTLES);
}

#[test]
fn before_poke_balls_battles_are_not_counted_yet() {
    let mut player = Player::starts_a_run(first_encounter());
    player.game.holds_poke_balls = false;
    player.walks_into(route_1());
    player.meets_wild(RATTATA);
    player.leaves_battle();
    let early = status(&mut player).unwrap();
    assert_eq!(early.state, AreaState::NotCountedYet);
    assert_eq!(early.say(), "Not counted yet");
    assert_eq!(early.why(), "Wild battles count once you have Poké Balls.");
    assert_eq!(battles(&early), ["RATTATA: no Poké Balls yet"]);

    player.game.holds_poke_balls = true;
    let now = status(&mut player).unwrap();
    assert_eq!(now.state, AreaState::CatchAllowed);
    player.meets_wild(PIDGEY);
    let done = status(&mut player).unwrap();
    assert!(
        done.battles.is_empty(),
        "the battles shown are those since the Encounter"
    );
}

#[test]
fn a_dupe_says_so_and_the_encounter_stays_open() {
    const PIDGEOTTO: u16 = 17;
    let mut player = Player::starts_a_run(Ruleset {
        first_encounter: true,
        dupes_clause: true,
        ..Ruleset::default()
    });
    player.game.species_names.insert(PIDGEOTTO, "PIDGEOTTO".into());
    player.game.evolutions.insert(PIDGEY, vec![PIDGEOTTO]);
    player.game.party = vec![party_pokemon(PIDGEOTTO, 0xB1BD_0001, "BIRDY")];
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    let dupe = status(&mut player).unwrap();
    assert_eq!(dupe.state, AreaState::Dupe);
    assert_eq!(dupe.say(), "Dupe, not counted");
    assert_eq!(
        dupe.why(),
        "PIDGEY is a dupe of your PIDGEOTTO. The Encounter stays open for the next battle."
    );
    assert_eq!(battles(&dupe), ["PIDGEY: a dupe of your PIDGEOTTO"]);
}

#[test]
fn the_status_follows_the_player_and_a_run_without_the_rule_has_none() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.walks_into(route_2());
    assert_eq!(status(&mut player).unwrap().state, AreaState::CatchAllowed);
    player.walks_into(route_1());
    assert_eq!(status(&mut player).unwrap().state, AreaState::EncounterDone);

    let mut free = Player::starts_a_run(Ruleset::default());
    free.walks_into(route_1());
    assert_eq!(status(&mut free), None);
}

#[test]
fn a_resumed_run_shows_the_same_status() {
    let mut player = Player::starts_a_run(first_encounter());
    player.walks_into(route_1());
    player.meets_wild(PIDGEY);
    player.leaves_battle();
    player.meets_wild(RATTATA);
    player.leaves_battle();
    let before = status(&mut player).unwrap();
    let storage = player.storage.clone();
    let clock = player.clock.clone();
    drop(player);

    let mut again = Player::resumes(storage, clock);
    again.walks_into(route_1());
    assert_eq!(status(&mut again).unwrap(), before);
}

#[test]
fn another_players_feed_shows_their_in_game_saves_and_not_their_blocked_battles() {
    use pactlocke_run::{AreaRef, CheckpointReason, FeedEntry, RunEvent};
    let checkpoint = |reason| RunEvent::Checkpoint {
        reason,
        hash: "00".into(),
        boot: 1,
        frame: 10,
    };
    let saved = FeedEntry::of_others(&checkpoint(CheckpointReason::InGameSave), START_MS).unwrap();
    assert_eq!(saved.text, "Saved in-game: a Run Checkpoint is kept");
    for reason in [
        CheckpointReason::Joined { call: 1 },
        CheckpointReason::SyncGate,
        CheckpointReason::Asked,
    ] {
        assert_eq!(FeedEntry::of_others(&checkpoint(reason), START_MS), None, "{reason:?}");
    }
    let blocked = RunEvent::CatchesBlocked {
        area: AreaRef {
            id: 0x65,
            name: "Route 1".into(),
        },
        species: RATTATA,
        species_name: "RATTATA".into(),
    };
    assert_eq!(FeedEntry::of_others(&blocked, START_MS), None);
}
