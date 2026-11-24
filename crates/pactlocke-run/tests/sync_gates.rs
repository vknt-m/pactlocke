//! Seam 1: Sync Gates, through the real running game on the
//! fake game, the shared Run log and every app's Shadow Games.

mod support;

use pactlocke_game::LevelCap;
use pactlocke_run::{Ruleset, ShadowState};
use support::streams::{Table, key};

fn gated(gates: &[&str]) -> Ruleset {
    Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        sync_gates: gates.iter().map(|g| (*g).to_owned()).collect(),
        ..Ruleset::default()
    }
}

fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

fn feed(table: &Table, i: usize) -> Vec<String> {
    table.apps[i].view_feed()
}

#[test]
fn every_chosen_gate_is_held_from_the_start() {
    let mut table = Table::start(&["Red", "Blue"], gated(&["Brock"]));
    both_play(&mut table, 40);
    for i in 0..2 {
        assert!(table.game(i).gates_closed.contains("Brock"), "{i}");
        assert!(!table.game(i).gates_closed.contains("Misty"), "only the chosen ones");
    }
}

#[test]
fn a_gate_holds_each_player_until_everyone_is_ready_then_the_battle_runs() {
    let mut table = Table::start(&["Red", "Blue"], gated(&["Brock", "Misty"]));
    both_play(&mut table, 100);

    // Red comes to Brock first: held, and Ready; then walks away.
    assert!(!table.talks_to(0, "Brock"), "held: no battle yet");
    assert!(
        feed(&table, 0)
            .iter()
            .any(|e| e == "Ready at Brock: waiting for your partners to be ready"),
        "{:?}",
        feed(&table, 0)
    );
    let check = table.apps[1].log.ready_check().expect("a ready check");
    assert_eq!(check.gate, "Brock");
    assert_eq!(check.ready(), 1);
    assert_eq!(check.players, [("Red".to_owned(), true), ("Blue".to_owned(), false)]);
    both_play(&mut table, 200);
    assert!(table.game(0).gates_closed.contains("Brock"), "still held for Red");

    // Blue comes too: Brock is let go in both games, and Misty stays held.
    assert!(!table.talks_to(1, "Brock"));
    both_play(&mut table, 40);
    for i in 0..2 {
        assert!(!table.game(i).gates_closed.contains("Brock"), "{i}");
        assert!(table.game(i).gates_closed.contains("Misty"), "{i}");
        assert!(
            feed(&table, i)
                .iter()
                .any(|e| e == "Everyone is Ready at Brock: the battle is on"),
            "{i}: {:?}",
            feed(&table, i)
        );
    }
    assert_eq!(
        table.apps[0].log.ready_check().map(|c| c.gate),
        Some("Misty".to_owned())
    );

    // Red comes back to Brock: the real battle starts.
    assert!(table.talks_to(0, "Brock"));
    table.leaves_battle(0);

    // A Run Checkpoint was taken at the gate, of every Player's game.
    both_play(&mut table, 100);
    assert!(table.apps[0].log.latest_complete().is_some());

    // And the Shadows saw it all the same way.
    both_play(&mut table, 300);
    table.waits(1000);
    for (watcher, watched) in [(0, 1), (1, 0)] {
        let shadow = table.apps[watcher].shadow_of(key(watched));
        assert_eq!(shadow.state(), ShadowState::Matched, "{watcher} watching {watched}");
        assert!(!shadow.game_mut().gates_closed.contains("Brock"));
    }
}

#[test]
fn over_the_level_cap_a_player_cant_ready_up() {
    let mut ruleset = gated(&["Brock"]);
    ruleset.level_caps = Some(vec![LevelCap {
        name: "Brock".into(),
        level: 4,
    }]);
    let mut table = Table::start(&["Red", "Blue"], ruleset);
    table.happens(0, |g| g.party.push(support::party_pokemon(support::PIDGEY, 0xA1, "")));
    both_play(&mut table, 100);
    assert!(!table.talks_to(0, "Brock"));
    assert!(
        feed(&table, 0)
            .iter()
            .any(|e| e.starts_with("Can't ready up at Brock: PIDGEY is level 5, over the level cap of 4")),
        "{:?}",
        feed(&table, 0)
    );
    assert_eq!(table.apps[1].log.ready_check().map(|c| c.ready()), Some(0));
}

#[test]
fn a_solo_run_holds_no_gates() {
    let mut table = Table::start(&["Red"], Ruleset::default());
    table.plays(0, 40);
    assert!(table.game(0).gates_closed.is_empty());
    assert!(table.talks_to(0, "Brock"), "the battle just runs");
}

#[test]
fn a_ready_a_retry_undid_doesnt_count_and_the_gate_waits_for_it_again() {
    let mut ruleset = gated(&["Brock"]);
    ruleset.retries = 1;
    let mut table = Table::start(&["Red", "Blue"], ruleset);
    both_play(&mut table, 100);
    table.saves(0);
    both_play(&mut table, 100);
    // Red readies up at Brock, then everyone goes back to Red's save.
    table.talks_to(0, "Brock");
    table.votes(0, 1, true);
    table.votes(1, 1, true);
    both_play(&mut table, 100);
    assert_eq!(
        table.apps[0].log.ready_check().map(|c| c.ready()),
        Some(0),
        "Red's Ready is undone"
    );

    // Blue readies up: Red isn't Ready any more, so Brock still holds.
    table.talks_to(1, "Brock");
    both_play(&mut table, 40);
    for i in 0..2 {
        assert!(table.game(i).gates_closed.contains("Brock"), "{i}: still held");
    }

    // Red comes again: now everyone is, and Brock is let go.
    table.talks_to(0, "Brock");
    both_play(&mut table, 40);
    for i in 0..2 {
        assert!(!table.game(i).gates_closed.contains("Brock"), "{i}: let go");
    }
}

#[test]
fn the_ready_check_shows_the_gate_someone_is_waiting_at() {
    let mut table = Table::start(&["Red", "Blue"], gated(&["Brock", "Misty"]));
    both_play(&mut table, 100);
    table.talks_to(1, "Misty");
    let check = table.apps[0].log.ready_check().unwrap();
    assert_eq!((check.gate.as_str(), check.ready()), ("Misty", 1));
}

#[test]
fn a_player_past_a_gate_alone_counts_as_ready_there_and_the_others_battle_runs() {
    let mut table = Table::start(&["Red", "Blue"], gated(&["Brock", "Misty"]));
    both_play(&mut table, 100);
    assert!(!table.talks_to(1, "Brock"), "Blue is held at Brock");
    // Red's game got past Brock without the gate (the badge is theirs).
    table.happens(0, |g| {
        g.gates_passed.insert("Brock".to_owned());
    });
    both_play(&mut table, 200);

    assert!(!table.game(1).gates_closed.contains("Brock"), "let go for Blue");
    assert!(table.game(1).gates_closed.contains("Misty"), "Misty still holds");
    assert!(table.talks_to(1, "Brock"), "Blue's battle runs");
    let passed = "Went past Brock's Sync Gate alone, so it counts as Ready there".to_owned();
    let own = feed(&table, 0);
    assert_eq!(own.iter().filter(|e| **e == passed).count(), 1, "once: {own:?}");
    // Blue's side panel shows Red's line from the Run log, by Red's name.
    let theirs: Vec<String> = table.apps[1]
        .log
        .standing_events()
        .into_iter()
        .filter(|(author, _)| *author == key(0))
        .filter_map(|(_, event)| pactlocke_run::FeedEntry::of_others(&event, 0))
        .map(|line| line.text)
        .collect();
    assert_eq!(theirs.iter().filter(|e| **e == passed).count(), 1, "{theirs:?}");
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "Everyone is Ready at Brock: the battle is on")
    );
}

#[test]
fn every_player_saying_yes_lets_a_gate_go() {
    let mut table = Table::start(&["Red", "Blue"], gated(&["Brock"]));
    both_play(&mut table, 100);
    assert!(!table.talks_to(0, "Brock"), "Red is held at Brock");
    table.votes_on_gate(1, "Brock", true);
    let vote = table.apps[0].log.gate_vote().expect("a vote under way");
    assert_eq!((vote.gate.as_str(), vote.yes.len()), ("Brock", 1));
    assert!(
        table.game(0).gates_closed.contains("Brock"),
        "not yet: Red hasn't said Yes"
    );

    table.votes_on_gate(0, "Brock", true);
    both_play(&mut table, 40);
    for i in 0..2 {
        assert!(!table.game(i).gates_closed.contains("Brock"), "{i}: let go");
        assert!(
            feed(&table, i)
                .iter()
                .any(|e| e == "Everyone said Yes: Brock's Sync Gate is let go"),
            "{i}: {:?}",
            feed(&table, i)
        );
    }
    assert_eq!(table.apps[0].log.gate_vote(), None);
    assert!(table.talks_to(0, "Brock"), "Red's battle runs");
}

#[test]
fn a_no_ends_a_vote_to_let_a_gate_go() {
    let mut table = Table::start(&["Red", "Blue"], gated(&["Brock"]));
    both_play(&mut table, 100);
    table.votes_on_gate(0, "Brock", true);
    table.votes_on_gate(1, "Brock", false);
    assert_eq!(table.apps[0].log.gate_vote(), None, "the No ended it");
    table.votes_on_gate(0, "Brock", true);
    both_play(&mut table, 40);
    assert!(table.game(0).gates_closed.contains("Brock"), "one Yes isn't everyone");
}

#[test]
fn a_gate_vote_the_others_finished_lets_the_gate_go_when_the_last_player_is_dropped() {
    let mut table = Table::start(&["Red", "Blue", "Green"], gated(&["Brock"]));
    for _ in 0..10 {
        for i in 0..3 {
            table.plays(i, 10);
        }
    }
    table.votes_on_gate(0, "Brock", true);
    table.votes_on_gate(2, "Brock", true);
    assert!(table.game(0).gates_closed.contains("Brock"), "Blue hasn't said");
    // Blue is dropped from the Run: everyone left has said Yes.
    table.votes_to_drop(0, 1, true);
    table.votes_to_drop(2, 1, true);
    for _ in 0..4 {
        table.plays(0, 10);
        table.plays(2, 10);
    }
    for i in [0, 2] {
        assert!(!table.game(i).gates_closed.contains("Brock"), "{i}: let go");
    }
}
