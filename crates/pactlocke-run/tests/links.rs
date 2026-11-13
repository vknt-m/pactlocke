//! Seam 1: Links across Players, through the real running game
//! on the fake game, the shared Run log and every app's Shadow Games, which
//! judge the other Players' catches and Deaths.

mod support;

use pactlocke_run::{LogBody, Member, Ruleset, RunEvent, SourceRef};
use support::streams::{Table, key};
use support::{CHARMANDER, PIDGEY, RATTATA, route_1, route_2};

fn soul_link() -> Ruleset {
    Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        links: true,
        ..Ruleset::default()
    }
}

fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

/// Red catches a Pidgey (0xA1) and Blue a Rattata (0xB1) on Route 1, and both play on.
fn route_1_linked(table: &mut Table) {
    both_play(table, 100);
    for i in 0..2 {
        table.walks_into(i, route_1());
    }
    table.meets_wild(0, PIDGEY);
    table.catches(0, PIDGEY, 0xA1);
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB1);
    both_play(table, 120);
}

fn route_1_link(table: &Table, i: usize) -> pactlocke_run::Link {
    table.apps[i]
        .links()
        .into_iter()
        .find(|l| l.source.name == "Route 1")
        .expect("Route 1's Link")
}

fn feed(table: &Table, i: usize) -> Vec<String> {
    table.apps[i].view_feed()
}

#[test]
fn a_catch_is_pending_until_every_player_has_had_their_encounter_then_they_are_linked() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    table.walks_into(0, route_1());
    table.meets_wild(0, PIDGEY);
    table.catches(0, PIDGEY, 0xA1);
    both_play(&mut table, 120);
    let link = route_1_link(&table, 1);
    assert!(link.pending(), "Blue hasn't had Route 1's Encounter: {link:?}");
    assert!(
        matches!(
            link.member(&key(0)),
            Some(Member::Caught {
                pid: 0xA1,
                judged: true,
                ..
            })
        ),
        "Blue's Shadow of Red shows the Pidgey: {link:?}"
    );

    table.walks_into(1, route_1());
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB1);
    both_play(&mut table, 120);
    for i in 0..2 {
        let link = route_1_link(&table, i);
        assert!(!link.pending() && link.broken.is_none(), "{i}: {link:?}");
        assert!(matches!(link.member(&key(1)), Some(Member::Caught { pid: 0xB1, .. })));
    }
}

#[test]
fn a_link_mates_death_kills_the_benched_link_mate_at_once() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    route_1_linked(&mut table);
    table.faints(0, 0xA1);
    both_play(&mut table, 120);

    let rattata = table.game(1).party.iter().find(|p| p.pid == 0xB1).cloned();
    assert!(
        rattata.is_none() || rattata.unwrap().stats.unwrap().hp == 0,
        "fainted (and moved to the Dead box)"
    );
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "RATTATA fainted with its Link: Red's PIDGEY died, so RATTATA is Dead"),
        "{:?}",
        feed(&table, 1)
    );
    assert_eq!(table.tallies(0).deaths.get(&key(1)), Some(&1));
}

#[test]
fn a_link_mates_death_faints_the_one_out_in_battle_by_the_end_of_the_turn() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    route_1_linked(&mut table);
    // Blue's Rattata, the only one in the party, is out in a trainer battle.
    table.meets_trainer(1, CHARMANDER);
    table.faints(0, 0xA1);
    both_play(&mut table, 120);
    assert_eq!(
        table.game(1).perishing,
        [0],
        "the Perish Song path, for the battler in position 0"
    );
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "RATTATA fainted with its Link: Red's PIDGEY died, so RATTATA is Dead"),
        "{:?}",
        feed(&table, 1)
    );
}

#[test]
fn a_missed_encounter_closes_the_source_for_everyone() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    table.walks_into(0, route_1());
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    both_play(&mut table, 120);
    assert_eq!(route_1_link(&table, 1).broken.as_deref(), Some("Red didn't catch it"));
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "Route 1 is closed for you too: Red didn't catch it"),
        "{:?}",
        feed(&table, 1)
    );

    // Blue's first wild battle there can't be caught.
    table.walks_into(1, route_1());
    table.meets_wild(1, RATTATA);
    assert!(table.game(1).catches_blocked);
}

#[test]
fn a_miss_after_the_others_caught_theirs_kills_their_catches() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    for i in 0..2 {
        table.walks_into(i, route_2());
    }
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB2);
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    both_play(&mut table, 120);
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "RATTATA fainted with its Link: Red didn't catch it, so RATTATA is Dead"),
        "{:?}",
        feed(&table, 1)
    );
}

#[test]
fn skipping_a_gift_the_others_took_breaks_its_link_and_kills_their_copies() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    let eevee = SourceRef {
        key: "celadon_eevee".into(),
        name: "the Eevee in Celadon City".into(),
    };
    // Red takes it: it is in Red's game, and Red's app says so.
    table.happens(0, |g| g.party.push(support::party_pokemon(133, 0xE1, "")));
    table.submit(
        0,
        &LogBody::Event(RunEvent::Gift {
            pid: 0xE1,
            species: 133,
            species_name: "EEVEE".into(),
            area: None,
            source: Some(eevee.clone()),
            starter: false,
            egg: false,
        }),
    );
    both_play(&mut table, 120);
    assert!(
        matches!(
            table.apps[1]
                .links()
                .iter()
                .find(|l| l.source.key == "celadon_eevee")
                .and_then(|l| l.member(&key(1))),
            Some(Member::Pending)
        ),
        "Blue hasn't taken it"
    );

    table.apps[1]
        .running
        .commands()
        .send(pactlocke_run::RunCommand::Skip { source: eevee })
        .unwrap();
    both_play(&mut table, 120);
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "You skipped the Eevee in Celadon City: its Link is broken for everyone"),
        "{:?}",
        feed(&table, 1)
    );
    assert!(
        feed(&table, 0)
            .iter()
            .any(|e| e == "SPECIES 133 fainted with its Link: Blue skipped it, so SPECIES 133 is Dead"),
        "{:?}",
        feed(&table, 0)
    );
}

#[test]
fn a_death_the_players_own_game_never_showed_kills_nobody() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    route_1_linked(&mut table);
    // Red's app claims the Pidgey died, but Red's game (and every Shadow of it) shows it healthy.
    table.submit(
        0,
        &LogBody::Event(RunEvent::Died {
            pid: 0xA1,
            species: PIDGEY,
            species_name: "PIDGEY".into(),
            name: "PIDGEY".into(),
            area: None,
            cause: pactlocke_run::DeathCause::Poison,
        }),
    );
    both_play(&mut table, 300);
    assert!(
        feed(&table, 1).iter().all(|e| !e.contains("fainted with its Link")),
        "{:?}",
        feed(&table, 1)
    );
    assert_eq!(route_1_link(&table, 1).broken, None);
}

#[test]
fn without_the_links_rule_a_death_stays_with_its_player() {
    let mut table = Table::start(
        &["Red", "Blue"],
        Ruleset {
            links: false,
            ..soul_link()
        },
    );
    route_1_linked(&mut table);
    table.faints(0, 0xA1);
    both_play(&mut table, 120);
    assert!(feed(&table, 1).iter().all(|e| !e.contains("fainted with its Link")));
}

#[test]
fn a_catch_made_while_the_source_was_closing_dies_with_the_broken_link() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    for i in 0..2 {
        table.walks_into(i, route_1());
    }
    // Blue's Encounter battle is under way when Red's gets away.
    table.meets_wild(1, RATTATA);
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    both_play(&mut table, 120);
    table.catches(1, RATTATA, 0xB1);
    both_play(&mut table, 120);
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "RATTATA fainted with its Link: Red didn't catch it, so RATTATA is Dead"),
        "{:?}",
        feed(&table, 1)
    );
}

#[test]
fn a_catch_its_players_game_hasnt_shown_yet_is_still_pending() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    for i in 0..2 {
        table.walks_into(i, route_1());
    }
    table.meets_wild(0, PIDGEY);
    table.catches(0, PIDGEY, 0xA1);
    // Red's app says Blue caught one, but Blue's game never showed it.
    table.submit(
        1,
        &LogBody::Event(RunEvent::EncounterOver {
            source: SourceRef {
                key: format!("area:{}", route_1().id),
                name: "Route 1".into(),
            },
            pid: Some(0xBAD),
        }),
    );
    both_play(&mut table, 120);
    assert!(route_1_link(&table, 0).pending(), "{:?}", route_1_link(&table, 0));
}
