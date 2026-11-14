//! Seam 1: disconnects, Host migration and Dropout, through the
//! real running game on the fake game, the shared Run log and every app's
//! Shadow Games, with apps going offline and coming back.

mod support;

use pactlocke_game::FakeGame;
use pactlocke_run::{
    FrameHash, LogBody, Member, Ruleset, RunEngine, ShadowState, StreamMessage, StreamWant, boot_core,
};
use support::streams::{Table, key};
use support::{CHARMANDER, PIDGEY, RATTATA, route_1};

fn soul_link() -> Ruleset {
    Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        links: true,
        retries: 1,
        ..Ruleset::default()
    }
}

fn everyone_plays(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        for i in 0..table.apps.len() {
            table.plays(i, 10);
        }
    }
}

fn feed(table: &Table, i: usize) -> Vec<String> {
    table.apps[i].view_feed()
}

#[test]
fn an_offline_player_plays_on_and_their_run_and_stream_arrive_when_they_are_back() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    everyone_plays(&mut table, 100);
    table.goes_offline(1);
    table.walks_into(1, route_1());
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB1);
    everyone_plays(&mut table, 400);
    assert_eq!(table.tallies(0).catches.get(&key(1)), None, "nothing reached Red yet");
    assert!(
        table.apps[0].presence.offline(&key(1), table.now()),
        "Red sees Blue offline"
    );
    let frozen = table.apps[0].shadow_of(key(1)).confirmed();

    table.comes_back(1);
    everyone_plays(&mut table, 40);
    assert_eq!(
        table.tallies(0).catches.get(&key(1)),
        Some(&1),
        "Blue's queued catch arrived"
    );
    assert!(!table.apps[0].presence.offline(&key(1), table.now()));
    everyone_plays(&mut table, 300);
    table.waits(2000);
    let shadow = table.apps[0].shadow_of(key(1));
    assert!(
        shadow.confirmed() > frozen + 600,
        "caught up: {frozen} -> {}",
        shadow.confirmed()
    );
    assert_eq!(shadow.state(), ShadowState::Matched);
}

#[test]
fn a_link_death_that_arrives_late_applies_at_once_even_mid_battle() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    everyone_plays(&mut table, 100);
    for i in 0..2 {
        table.walks_into(i, route_1());
    }
    table.meets_wild(0, PIDGEY);
    table.catches(0, PIDGEY, 0xA1);
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB1);
    everyone_plays(&mut table, 120);

    // Blue is offline when Red's Pidgey faints, and in a battle by the time it reaches Blue.
    table.goes_offline(1);
    table.faints(0, 0xA1);
    table.meets_trainer(1, CHARMANDER);
    everyone_plays(&mut table, 200);
    assert!(table.game(1).perishing.is_empty(), "nothing reached Blue yet");

    table.comes_back(1);
    everyone_plays(&mut table, 300);
    assert_eq!(table.game(1).perishing, [0], "the Perish path, mid-battle");
    assert!(
        feed(&table, 1)
            .iter()
            .any(|e| e == "RATTATA fainted with its Link: Red's PIDGEY died, so RATTATA is Dead"),
        "{:?}",
        feed(&table, 1)
    );
}

#[test]
fn when_the_host_vanishes_the_next_player_takes_over_from_the_longest_log() {
    let mut table = Table::start(&["Red", "Blue", "Green"], soul_link());
    everyone_plays(&mut table, 100);
    // Blue misses an entry the Host ordered for Green, then the Host vanishes.
    table.goes_offline(1);
    table.walks_into(2, route_1());
    table.meets_wild(2, PIDGEY);
    table.catches(2, PIDGEY, 0xC1);
    everyone_plays(&mut table, 40);
    let green_has = table.apps[2].log.entries().len();
    assert!(table.apps[1].log.entries().len() < green_has);
    table.host_vanishes();
    table.offline.remove(&1); // Blue is back, but the old Host is gone.

    // Ten seconds of the Host's silence: Blue, next in join order, takes over.
    table.waits(11_000);
    assert_eq!(table.host, 1);
    for i in [1, 2] {
        let log = &table.apps[i].log;
        assert_eq!(log.host(), key(1), "{i}");
        assert!(
            log.entries().len() > green_has,
            "{i}: Green's entries kept, then the takeover"
        );
        assert_eq!(log.tallies().catches.get(&key(2)), Some(&1), "{i}");
        assert!(table.apps[i].refused.is_empty(), "{i}: {:?}", table.apps[i].refused);
    }

    // And the Run goes on under the new Host.
    table.walks_into(1, route_1());
    table.meets_wild(1, RATTATA);
    table.catches(1, RATTATA, 0xB1);
    for _ in 0..30 {
        table.plays(1, 10);
        table.plays(2, 10);
    }
    assert_eq!(table.apps[2].log.tallies().catches.get(&key(1)), Some(&1));
}

#[test]
fn a_takeover_out_of_turn_is_refused() {
    let mut table = Table::start(&["Red", "Blue", "Green"], soul_link());
    everyone_plays(&mut table, 40);
    // Green, not next after Red, tries to take over.
    let counter = table.host_log.next_counter(&key(2));
    let submission = table.apps[2].log.sign(
        &pactlocke_run::Signer::from_secret([3; 32]),
        counter,
        &LogBody::Takeover,
    );
    let refused = table
        .host_log
        .order(&pactlocke_run::Signer::from_secret([3; 32]), submission)
        .unwrap_err();
    assert!(refused.to_string().contains("out of turn"), "{refused}");
}

#[test]
fn after_15_minutes_offline_the_others_may_drop_a_player_and_their_pokemon_leave_every_link() {
    let mut table = Table::start(&["Red", "Blue", "Green"], soul_link());
    everyone_plays(&mut table, 100);
    for i in [0, 2] {
        table.walks_into(i, route_1());
    }
    table.meets_wild(0, PIDGEY);
    table.catches(0, PIDGEY, 0xA1);
    table.meets_wild(2, RATTATA);
    table.catches(2, RATTATA, 0xC1);
    table.goes_offline(1);
    everyone_plays(&mut table, 120);
    let link = |table: &Table| {
        table.apps[0]
            .links()
            .into_iter()
            .find(|l| l.source.name == "Route 1")
            .unwrap()
    };
    assert!(link(&table).pending(), "waiting on Blue");

    let now = table.now();
    assert!(!table.apps[0].presence.dropout_offered(&key(1), now), "not yet");
    table.clock.advance_ms(15 * 60 * 1000);
    let now = table.now();
    assert!(table.apps[0].presence.dropout_offered(&key(1), now));

    // Everyone but Blue must say Yes.
    table.votes_to_drop(0, 1, true);
    assert_eq!(table.apps[0].log.active().len(), 3, "Green hasn't voted");
    table.votes_to_drop(2, 1, true);
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[0].log.active(), [key(0), key(2)]);
    let route_1 = link(&table);
    assert!(!route_1.pending() && route_1.broken.is_none(), "{route_1:?}");
    assert!(route_1.member(&key(1)).is_none(), "Blue is out of every Link");
    assert!(matches!(route_1.member(&key(2)), Some(Member::Caught { .. })));

    // A Retry from here takes only the Players still in the Run.
    table.saves(0);
    everyone_plays(&mut table, 100);
    let call = table.apps[0].log.latest_complete().expect("complete without Blue");
    table.votes(0, call, true);
    table.votes(2, call, true);
    everyone_plays(&mut table, 40);
    assert_eq!(table.apps[0].log.retries_left(), 0, "the Retry happened");

    // Blue, back, can't write to the Run any more, and their game wasn't sent back.
    table.comes_back(1);
    table.meets_wild(1, PIDGEY);
    everyone_plays(&mut table, 40);
    assert_eq!(table.apps[0].running.engine().boot(), 2, "Red went back");
    assert_eq!(table.apps[1].running.engine().boot(), 1, "Blue didn't");
    assert!(
        table.apps[1].refused.iter().any(|e| e.contains("outside the roster")),
        "{:?}",
        table.apps[1].refused
    );
}

#[test]
fn a_dropout_vote_ends_at_the_first_no() {
    let mut table = Table::start(&["Red", "Blue", "Green"], soul_link());
    everyone_plays(&mut table, 40);
    table.votes_to_drop(0, 1, true);
    table.votes_to_drop(2, 1, false);
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[0].log.dropout_vote(), None);
    assert_eq!(table.apps[0].log.active().len(), 3);
}

#[test]
fn a_crashed_app_takes_its_own_presses_back_from_the_host() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    // Blue plays; every frame's state is noted, as Blue's game was.
    let mut states = vec![FrameHash::of(&mut table.game(1).core, 0)];
    for _ in 0..330 {
        table.plays(1, 1);
        table.plays(0, 1);
        let frame = table.apps[1].running.engine().frames_run();
        states.push(FrameHash::of(&mut table.game(1).core, frame));
    }
    // The power goes out: the Run file has the presses up to its last flush only.
    let crashed = table.apps[1].storage.snapshot();
    let resumed = RunEngine::resume(Box::new(crashed), Box::new(table.clock.clone())).unwrap();
    let mut engine = resumed;
    let pactlocke_run::BootPlan::Rebuild { from, keys, .. } = engine.boot_plan().clone() else {
        panic!("rebuilt after a crash");
    };
    let flushed = from.frame + keys.len() as u64;
    let host_has = table.apps[0].hub.received(&key(1)).unwrap().1;
    assert!(
        host_has > flushed,
        "the Host has more of Blue's stream than Blue's Run file"
    );

    // Blue asks the Host for the rest of their own stream, and rebuilds through it.
    let want = StreamMessage::Want(StreamWant {
        player: key(1),
        boot: 1,
        from_frame: flushed,
    });
    let out = table.apps[0].hub.receive(key(1), want).send;
    let chunks: Vec<_> = out
        .into_iter()
        .filter_map(|o| match o {
            pactlocke_run::Outgoing::To(_, StreamMessage::Chunk(chunk)) => Some(chunk),
            _ => None,
        })
        .collect();
    engine.take_back_presses(&chunks);
    let mut game = FakeGame::default();
    boot_core(&mut game.core, engine.boot_plan(), 0).unwrap();
    let rebuilt = game.core.frames_run();
    assert_eq!(rebuilt, host_has, "every press the Host had");
    assert!(
        FrameHash::of(&mut game.core, rebuilt).matches(&states[rebuilt as usize]),
        "the rebuilt game is Blue's game as it was at that frame"
    );
}

#[test]
fn the_leaver_has_no_say_and_a_vote_on_someone_already_gone_counts_for_nothing() {
    let mut table = Table::start(&["Red", "Blue", "Green"], soul_link());
    everyone_plays(&mut table, 40);
    table.votes_to_drop(0, 1, true);
    table.votes_to_drop(1, 1, false);
    assert!(
        table.apps[0].log.dropout_vote().is_some(),
        "Blue's own No doesn't end it"
    );
    table.votes_to_drop(2, 1, true);
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[0].log.active(), [key(0), key(2)]);

    // A late Yes on Blue opens nothing, and with two Players left a vote
    // on Red can't start: Green would decide alone.
    table.votes_to_drop(0, 1, true);
    assert_eq!(table.apps[0].log.dropout_vote(), None);
    table.votes_to_drop(2, 0, true);
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[2].log.dropout_vote(), None);
    assert_eq!(table.apps[2].log.active(), [key(0), key(2)]);
}

#[test]
fn with_two_players_there_is_no_dropout() {
    let mut table = Table::start(&["Red", "Blue"], soul_link());
    everyone_plays(&mut table, 40);
    for log in [&table.apps[0].log, &table.apps[1].log] {
        assert!(!log.dropout_allowed(), "one Player would decide alone");
    }
    table.votes_to_drop(0, 1, true);
    everyone_plays(&mut table, 20);
    assert_eq!(table.apps[0].log.dropout_vote(), None);
    assert_eq!(table.apps[1].log.active(), [key(0), key(1)]);
}

#[test]
fn a_host_dropped_from_the_run_is_followed_by_the_next_player_in_join_order() {
    let mut table = Table::start(&["Red", "Blue", "Green"], soul_link());
    everyone_plays(&mut table, 40);
    table.votes_to_drop(1, 0, true);
    table.votes_to_drop(2, 0, true);
    everyone_plays(&mut table, 20);
    let log = &table.apps[1].log;
    assert_eq!(log.active(), [key(1), key(2)]);
    assert_eq!(log.successor(), Some(key(1)));
}

/// A Player's copy of the Run log meets the new Host's, which
/// lacks an entry this Player had from the old Host. The new
/// Host's copy wins from there on, and this Player's own entries it lacks
/// come back, in order, to send again.
#[test]
fn a_copy_of_the_log_gives_way_to_the_new_hosts_and_its_own_lost_entries_come_back() {
    use pactlocke_run::{DeathCause, Locked, RosterEntry, RunEvent, RunLog, Signer};
    let signers: Vec<Signer> = (1..=3).map(|i| Signer::from_secret([i; 32])).collect();
    let [red, blue, green] = [0, 1, 2].map(|i| signers[i].public());
    let died = |pid| {
        LogBody::Event(RunEvent::Died {
            pid,
            species: 16,
            species_name: "PIDGEY".into(),
            name: "PIDGEY".into(),
            area: None,
            cause: DeathCause::Poison,
        })
    };
    let mut host = RunLog::new(red);
    let submit = |log: &mut RunLog, by: usize, orderer: usize, body: &LogBody| {
        let counter = log.next_counter(&signers[by].public());
        let submission = log.sign(&signers[by], counter, body);
        log.order(&signers[orderer], submission).unwrap()
    };
    let genesis = LogBody::Genesis {
        nonce: [9; 16],
        run_name: "Kanto".into(),
        game_name: "FireRed".into(),
        game_sha1: "00".into(),
    };
    submit(&mut host, 0, 0, &genesis);
    let roster = ["Red", "Blue", "Green"]
        .iter()
        .zip([red, blue, green])
        .map(|(name, key)| RosterEntry {
            key,
            name: (*name).into(),
        })
        .collect();
    let locked = Locked {
        run_name: "Kanto".into(),
        game_name: "FireRed".into(),
        game_sha1: "00".into(),
        ruleset: soul_link(),
        roster,
        start_ms: 0,
    };
    submit(&mut host, 0, 0, &LogBody::Locked(locked));
    // Blue, next after Red, has the log this far; Green also has two of
    // their own deaths, the last entries Red ordered before vanishing.
    let mut blues = host.clone();
    submit(&mut host, 2, 0, &died(1));
    submit(&mut host, 2, 0, &died(2));
    let mut greens = host.clone();
    let dropped_by_green: Vec<_> = greens.entries()[2..].iter().map(|e| e.submission.clone()).collect();

    // Blue takes over from the shorter copy, and orders one of their own.
    let takeover = submit(&mut blues, 1, 1, &LogBody::Takeover);
    let after = submit(&mut blues, 1, 1, &died(7));

    // Green hears the new Host's copy, entry by entry.
    for entry in blues.entries().to_vec() {
        let back = greens.give_way_to(entry, &green).unwrap();
        if back.is_empty() {
            continue;
        }
        assert_eq!(back, dropped_by_green, "Green's own two deaths, in order");
    }
    assert_eq!(greens.entries(), blues.entries());
    assert_eq!(greens.host(), blue);
    assert_eq!(greens.entries()[2], takeover);
    assert_eq!(greens.entries()[3], after);

    // Sent again, the new Host orders them as they were signed.
    for submission in dropped_by_green {
        blues.order(&signers[1], submission).unwrap();
    }
    assert_eq!(blues.tallies().deaths.get(&green), Some(&2));

    // The same copy again changes nothing.
    for entry in blues.entries().to_vec() {
        assert!(greens.give_way_to(entry, &green).unwrap().is_empty());
    }

    // The old Host's copy, without the takeover, doesn't win over Green's,
    // which has it: Green keeps theirs.
    let had = greens.entries().to_vec();
    let stale = host.entries()[2].clone();
    let refused = greens.give_way_to(stale, &green).unwrap_err();
    assert!(refused.to_string().contains("takeover"), "{refused}");
    assert_eq!(greens.entries(), had);
}
