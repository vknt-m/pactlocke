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
        // Each part carries its species, for the Links tab's icons.
        assert!(matches!(link.member(&key(0)), Some(Member::Caught { species, .. }) if *species == PIDGEY));
        assert!(matches!(link.member(&key(1)), Some(Member::Caught { species, .. }) if *species == RATTATA));
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
            at_ms: 0,
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
            at_ms: 0,
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

fn area(id: u16, name: &str) -> pactlocke_game::Area {
    pactlocke_game::Area { id, name: name.into() }
}

/// Player `i` catches a Pidgey with this PID on `area`.
fn catches_on(table: &mut Table, i: usize, at: pactlocke_game::Area, pid: u32) {
    table.walks_into(i, at);
    table.meets_wild(i, PIDGEY);
    table.catches(i, PIDGEY, pid);
}

fn party_pids(table: &mut Table, i: usize) -> Vec<u32> {
    table.game(i).party.iter().map(|p| p.pid).collect()
}

#[test]
fn a_catch_waiting_on_its_link_stays_in_a_party_of_fewer_than_six() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    catches_on(&mut table, 0, route_1(), 0xA1);
    both_play(&mut table, 200);
    assert!(route_1_link(&table, 0).pending());
    assert_eq!(party_pids(&mut table, 0), [0xA1]);
}

#[test]
fn at_a_full_party_the_newest_pokemon_waiting_on_its_link_goes_to_the_pc_until_its_link_is_whole() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    let areas: Vec<_> = (0..6u16)
        .map(|n| area(0x70 + n, &format!("Route {}", 10 + n)))
        .collect();
    // Blue catches on the first four; Red on all six, so two of Red's wait.
    for (n, at) in areas.iter().enumerate().take(4) {
        catches_on(&mut table, 1, at.clone(), 0xB0 + n as u32);
    }
    for (n, at) in areas.iter().enumerate() {
        catches_on(&mut table, 0, at.clone(), 0xA0 + n as u32);
    }
    both_play(&mut table, 300);
    assert_eq!(
        party_pids(&mut table, 0),
        [0xA0, 0xA1, 0xA2, 0xA3, 0xA4],
        "the newest waiting one went to the PC; the other waiting one stays in a party of five"
    );
    assert!(table.game(0).pc.iter().any(|s| s.pokemon.pid == 0xA5));
    assert!(
        feed(&table, 0)
            .iter()
            .any(|e| e == "PIDGEY waits in the PC until Blue catches on Route 15: your party is full"),
        "{:?}",
        feed(&table, 0)
    );

    // Blue catches on Route 15, and Red takes that PIDGEY back into the party.
    catches_on(&mut table, 1, areas[5].clone(), 0xB5);
    both_play(&mut table, 300);
    table.happens(0, |g| {
        let at = g.pc.iter().position(|s| s.pokemon.pid == 0xA5).unwrap();
        let stored = g.pc.remove(at);
        g.party.push(stored.pokemon);
    });
    both_play(&mut table, 300);
    let party = party_pids(&mut table, 0);
    assert!(party.contains(&0xA5), "its Link is whole, so it stays: {party:x?}");
    assert!(
        !party.contains(&0xA4),
        "Route 14's PIDGEY still waits, and the party is full again: {party:x?}"
    );
}

/// A script gives Player `i` this Pokémon at `position`, called from `calls`
/// (innermost first), then ends.
fn given(table: &mut Table, i: usize, species: u16, pid: u32, position: u32, calls: &[u32]) {
    use pactlocke_game::{ScriptContext, ScriptStatus};
    table.happens(i, |g| {
        g.script = ScriptContext {
            status: ScriptStatus::Running,
            position: Some(position),
        };
        g.script_calls = calls.to_vec();
        g.party.push(support::party_pokemon(species, pid, ""));
    });
    both_play(table, 20);
    table.happens(i, |g| {
        g.script = ScriptContext {
            status: ScriptStatus::Idle,
            position: None,
        };
        g.script_calls.clear();
    });
    both_play(table, 20);
}

fn gift_links(table: &Table, i: usize) -> Vec<pactlocke_run::Link> {
    table.apps[i].links().into_iter().filter(|l| l.gift).collect()
}

#[test]
fn the_first_pokemon_each_player_gets_is_their_starter_and_the_starters_are_linked() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    both_play(&mut table, 100);
    // No profile source at either place, and each picked a different one of three.
    given(&mut table, 0, 1, 0xA0, 0x0817_0000, &[]);
    given(&mut table, 1, CHARMANDER, 0xB0, 0x0817_0010, &[]);
    both_play(&mut table, 120);

    for (i, app) in table.apps.iter().enumerate() {
        let starter = app
            .links()
            .into_iter()
            .find(|l| l.source.key == "starter")
            .expect("the starters' Link");
        assert!(!starter.pending() && starter.broken.is_none(), "{i}: {starter:?}");
        assert!(matches!(
            starter.member(&key(0)),
            Some(Member::Caught { pid: 0xA0, .. })
        ));
        assert!(matches!(
            starter.member(&key(1)),
            Some(Member::Caught { pid: 0xB0, .. })
        ));
    }
}

#[test]
fn a_gift_the_profile_doesnt_list_is_linked_by_its_area_and_species() {
    for (case, red, blue) in [
        // Seen inside the shared "received a Pokémon" script it called.
        (
            "called",
            (0x081A_5000, &[0x081A_5010, 0x0816_0040][..]),
            (0x081A_5008, &[0x081A_5010, 0x0816_0040][..]),
        ),
        // As on Unbound's Route 2: no calls, seen right after the routine
        // that gave it in one game and a message later in the other.
        ("no calls", (0x09E8_7067, &[][..]), (0x09E8_7081, &[][..])),
    ] {
        let mut table = Table::start(&["Red", "Blue"], soul_link());
        route_1_linked(&mut table);
        given(&mut table, 0, 133, 0xA2, red.0, red.1);
        given(&mut table, 1, 133, 0xB2, blue.0, blue.1);
        both_play(&mut table, 120);

        for i in 0..2 {
            let gifts = gift_links(&table, i);
            assert_eq!(gifts.len(), 1, "{case}, {i}: {gifts:?}");
            let gift = &gifts[0];
            assert!(!gift.pending() && gift.broken.is_none(), "{case}, {i}: {gift:?}");
            assert!(matches!(gift.member(&key(0)), Some(Member::Caught { pid: 0xA2, .. })));
            assert!(matches!(gift.member(&key(1)), Some(Member::Caught { pid: 0xB2, .. })));
            assert_eq!(gift.source.name, "unknown Gift in Route 1");
        }
    }
}

#[test]
fn unlisted_gifts_of_different_species_are_not_linked() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    route_1_linked(&mut table);
    given(&mut table, 0, 133, 0xA2, 0x081A_5000, &[0x081A_5010, 0x0816_0040]);
    given(&mut table, 1, 147, 0xB2, 0x081A_5000, &[0x081A_5010, 0x0816_0080]);
    both_play(&mut table, 120);

    let gifts = gift_links(&table, 0);
    assert_eq!(gifts.len(), 2, "{gifts:?}");
    assert!(gifts.iter().all(pactlocke_run::Link::pending), "{gifts:?}");
}
