//! Seam 1, with up to three Players in one process: Run setup (presets,
//! greyed-out rules, Ready, the lock checks, the lock and countdown) and the
//! Run log every Player shares (Host-ordered, Player-signed, hash-linked).

mod support;

use pactlocke_game::{Capability, CapabilityReport, LevelCap, Rule};
use pactlocke_run::{
    COUNTDOWN_MS, LogBody, LogError, Preset, Ruleset, RunEvent, RunLog, SetupError, Signer, unavailable,
};
use support::coop::{BUILD, Coop};
use support::*;

fn everything() -> CapabilityReport {
    CapabilityReport::everything_enforceable()
}

#[test]
fn presets_start_the_ruleset_and_every_rule_stays_optional() {
    let caps = vec![LevelCap {
        name: "Brock".into(),
        level: 14,
    }];
    let classic = Preset::ClassicNuzlocke.ruleset(&caps);
    assert!(classic.first_encounter && classic.faint_is_dead && classic.dupes_clause && classic.gifts);
    assert!(!classic.links && classic.level_caps.is_none() && !classic.no_items);
    assert_eq!(
        Preset::SoulLink.ruleset(&caps),
        Ruleset {
            links: true,
            ..classic.clone()
        }
    );
    let hardcore = Preset::Hardcore.ruleset(&caps);
    assert_eq!(hardcore.level_caps, Some(caps));
    assert!(hardcore.no_items && hardcore.set_mode);

    let mut coop = Coop::host("Red", everything(), Preset::Hardcore);
    coop.setup.edit(|r| r.no_items = false).unwrap();
    assert!(!coop.setup.ruleset().no_items, "optional");
}

#[test]
fn rules_that_depend_on_others_are_greyed_out_with_the_reason() {
    let mut ruleset = Preset::SoulLink.ruleset(&[]);
    let caps = everything();
    assert_eq!(
        unavailable(Rule::Links, &ruleset, &caps, 1).as_deref(),
        Some("a solo Run has no Links")
    );
    assert_eq!(unavailable(Rule::Links, &ruleset, &caps, 2), None);
    ruleset.faint_is_dead = false;
    assert_eq!(
        unavailable(Rule::Links, &ruleset, &caps, 2).as_deref(),
        Some("it needs Faint = Dead on")
    );
    ruleset.first_encounter = false;
    for rule in [Rule::DupesClause, Rule::Statics] {
        assert_eq!(
            unavailable(rule, &ruleset, &caps, 2).as_deref(),
            Some("it needs First Encounter on")
        );
    }
}

#[test]
fn rules_the_rom_cant_enforce_are_greyed_out_with_the_capability_reports_reason() {
    let caps = everything().with(
        Rule::NoItems,
        Capability::GreyedOut {
            reason: "the ROM holds 00 00 at 0x08014414".into(),
        },
    );
    let mut coop = Coop::host("Red", caps.clone(), Preset::Hardcore);
    assert_eq!(
        coop.setup.unavailable(Rule::NoItems).as_deref(),
        Some("can't be enforced on this ROM: the ROM holds 00 00 at 0x08014414")
    );
    assert!(!coop.setup.ruleset().no_items, "turned off");
    coop.setup.edit(|r| r.no_items = true).unwrap();
    assert!(!coop.setup.ruleset().no_items, "and it can't be turned on");

    // A level cap the game can't be stopped at is still a soft cap.
    let soft = everything().with(
        Rule::LevelCap,
        Capability::GreyedOut {
            reason: "no sites".into(),
        },
    );
    assert_eq!(unavailable(Rule::LevelCap, &Ruleset::default(), &soft, 1), None);
}

#[test]
fn soul_link_solo_has_no_links_until_a_second_player_joins() {
    let mut coop = Coop::host("Red", everything(), Preset::SoulLink);
    assert!(!coop.setup.ruleset().links, "solo");
    coop.join("Blue").unwrap();
    coop.setup.choose(Preset::SoulLink).unwrap();
    coop.broadcast();
    assert!(coop.setup.ruleset().links);
    let view = coop.players[1].view.clone().unwrap();
    assert!(view.ruleset.links, "Blue sees it live");
}

#[test]
fn every_player_sees_each_change_and_any_change_clears_every_ready() {
    let mut coop = Coop::with(&["Red", "Blue", "Green"]);
    coop.everyone_ready();
    for i in 0..3 {
        for seen_by in 0..3 {
            assert!(coop.is_ready(i, seen_by));
        }
    }

    coop.setup.edit(|r| r.statics = true).unwrap();
    coop.broadcast();
    for seen_by in 0..3 {
        let view = coop.players[seen_by].view.as_ref().unwrap();
        assert!(view.ruleset.statics, "Player {seen_by} sees the change");
        for i in 0..3 {
            assert!(!coop.is_ready(i, seen_by), "Ready cleared");
        }
    }
}

#[test]
fn ready_to_an_old_version_of_the_ruleset_counts_for_nothing() {
    let mut coop = Coop::with(&["Red", "Blue"]);
    let old = coop.players[1].view.as_ref().unwrap().version;
    coop.setup.edit(|r| r.type_clause = true).unwrap();
    coop.send(1, pactlocke_run::SetupMessage::Ready { version: old });
    assert!(!coop.is_ready(1, 0));
}

#[test]
fn the_lock_names_everything_that_doesnt_match() {
    let mut coop = Coop::with(&["Red", "Blue", "Green"]);
    coop.brings(1, |c| c.app_build = "0.0.9".into());
    coop.brings(2, |c| {
        c.game_sha1 = Some("574fa542ffebb14be69902d1d36f1ec0a4afd71e".into())
    });
    coop.players[0].check.hack_options = vec![("LEVEL CAP".into(), "ON".into())];
    let host_check = coop.players[0].check.clone();
    coop.send(0, pactlocke_run::SetupMessage::Check(host_check));
    coop.brings(1, |c| c.randomizer = Some("abc".into()));
    coop.everyone_ready();

    let Err(SetupError::NotYet(why)) = coop.lock() else {
        panic!("the lock goes through");
    };
    let expected = [
        format!("Blue: Pactlocke 0.0.9, not the Host's {BUILD}"),
        "Blue: the game's LEVEL CAP is not set, not ON".to_owned(),
        "Blue: a different randomizer output".to_owned(),
        "Green: a different ROM (SHA-1 574fa542ffebb14be69902d1d36f1ec0a4afd71e), not Pokémon FireRed (USA) 1.0"
            .to_owned(),
        "Green: the game's LEVEL CAP is not set, not ON".to_owned(),
    ];
    for line in &expected {
        assert!(why.contains(line), "{line:?} in {why:#?}");
    }
    let view = coop.players[2].view.as_ref().unwrap();
    assert!(
        view.mismatches.iter().any(|m| m.player == "Green"),
        "everyone sees them"
    );
}

#[test]
fn the_lock_waits_for_everyone_to_be_ready() {
    let mut coop = Coop::with(&["Red", "Blue"]);
    coop.ready(0);
    assert_eq!(coop.lock(), Err(SetupError::NotYet(vec!["Blue isn't Ready".into()])));
    coop.ready(1);
    assert!(coop.lock().is_ok());
}

#[test]
fn a_player_without_a_rom_yet_holds_the_lock() {
    let mut coop = Coop::with(&["Red", "Blue"]);
    coop.brings(1, |c| c.game_sha1 = None);
    coop.everyone_ready();
    let Err(SetupError::NotYet(why)) = coop.lock() else {
        panic!()
    };
    assert_eq!(why, ["Blue: no ROM chosen yet"]);
}

#[test]
fn the_lock_freezes_the_ruleset_and_roster_and_starts_the_countdown_and_the_clock() {
    let mut coop = Coop::with(&["Red", "Blue", "Green"]);
    coop.everyone_ready();
    let locked = coop.lock().unwrap();

    assert_eq!(locked.start_ms, START_MS + COUNTDOWN_MS, "after a 3-2-1 countdown");
    let names: Vec<&str> = locked.roster.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Red", "Blue", "Green"], "in join order");
    for player in &mut coop.players {
        assert_eq!(player.log.locked(), Some(locked.clone()), "in everyone's log");
        let run = player.run();
        assert_eq!(run.engine.info().ruleset, locked.ruleset);
        assert_eq!(
            run.engine.info().created_ms,
            locked.start_ms,
            "every game's clock starts at the lock's moment"
        );
    }
    assert_eq!(coop.setup.edit(|r| r.links = true), Err(SetupError::Locked));
    assert!(coop.join("Yellow").is_err(), "nobody joins after the lock");
}

#[test]
fn a_run_has_one_to_three_players() {
    let mut solo = Coop::with(&["Red"]);
    solo.everyone_ready();
    assert!(solo.lock().is_ok(), "one");

    let mut coop = Coop::with(&["Red", "Blue", "Green"]);
    assert_eq!(coop.join("Yellow"), Err(SetupError::Full));
}

#[test]
fn every_players_run_state_goes_in_one_log_that_everyone_checks_and_tallies_alike() {
    let mut coop = Coop::with(&["Red", "Blue", "Green"]);
    coop.everyone_ready();
    coop.lock().unwrap();

    for (i, player) in coop.players.iter_mut().enumerate() {
        let run = player.run();
        run.walks_into(route_1());
        run.meets_wild(PIDGEY);
        run.catches(PIDGEY, 0x1000 + i as u32);
    }
    // Blue's catch faints (poison on the overworld): a Death, with Faint = Dead on.
    let blue = coop.players[1].run();
    blue.game.party[0].stats = Some(pactlocke_game::PartyStats {
        level: 5,
        hp: 20,
        max_hp: 20,
    });
    blue.is_poisoned_to_faint(0x1001);
    coop.sync();

    let host_log = coop.host_log.entries().to_vec();
    for player in &coop.players {
        assert_eq!(player.log.entries(), &host_log[..], "{} has the same log", player.name);
        assert!(player.refused.is_empty(), "{}: {:?}", player.name, player.refused);
        let checked = RunLog::verify(coop.players[0].key(), player.log.entries().to_vec()).unwrap();
        let tallies = checked.tallies();
        for other in &coop.players {
            assert_eq!(
                tallies.encounters.get(&other.key()),
                Some(&1),
                "{}'s Encounter",
                other.name
            );
            assert_eq!(tallies.catches.get(&other.key()), Some(&1));
        }
        assert_eq!(tallies.deaths.get(&coop.players[1].key()), Some(&1), "Blue's Death");
        assert_eq!(tallies.deaths.len(), 1, "only Blue's");
    }
    assert!(
        host_log.iter().all(|e| !e.submission.body.is_empty()),
        "every entry holds its body"
    );
}

#[test]
fn only_run_state_goes_in_the_shared_log() {
    let mut coop = Coop::with(&["Red"]);
    coop.everyone_ready();
    coop.lock().unwrap();
    let run = coop.players[0].run();
    run.walks_into(route_1());
    run.meets_wild(PIDGEY);
    run.leaves_battle();
    coop.sync();

    let events: Vec<RunEvent> = coop
        .host_log
        .bodies()
        .filter_map(|(_, body)| match body {
            LogBody::Event(event) => Some(event),
            _ => None,
        })
        .collect();
    assert!(events.iter().all(RunEvent::is_run_state), "{events:?}");
    // The Encounter, and how it ended (nothing caught): this Player's part of Route 1's Link.
    assert!(
        matches!(
            events[..],
            [RunEvent::Encounter { .. }, RunEvent::EncounterOver { pid: None, .. }]
        ),
        "{events:?}"
    );
}

#[test]
fn soul_link_and_hardcore_tick_every_gate_once_a_second_player_joins_and_the_host_can_untick_any() {
    let gates: Vec<String> = ["Brock", "Misty", "Champion"].map(String::from).to_vec();
    let mut coop = Coop::host("Red", CapabilityReport::everything_enforceable(), Preset::SoulLink);
    coop.setup.set_sync_gates(gates.clone());
    assert!(
        coop.setup.ruleset().sync_gates.is_empty(),
        "a solo Run has no Sync Gates"
    );
    assert_eq!(
        coop.setup.unavailable(Rule::SyncGates),
        Some("a solo Run has no Sync Gates".to_owned())
    );

    coop.join("Blue").unwrap();
    assert_eq!(coop.setup.ruleset().sync_gates, gates, "every gate, once there are two");
    coop.setup.edit(|r| r.sync_gates.retain(|g| g != "Misty")).unwrap();
    assert_eq!(coop.setup.ruleset().sync_gates, ["Brock", "Champion"]);

    // Blue leaves and comes back: the Host's choice comes back with them.
    let blue = coop.players[1].key();
    coop.setup.leave(&blue);
    assert!(coop.setup.ruleset().sync_gates.is_empty());
    coop.setup
        .join(pactlocke_run::SetupPlayer {
            key: blue,
            name: "Blue".into(),
            check: coop.players[1].check.clone(),
            ready_for: None,
        })
        .unwrap();
    assert_eq!(coop.setup.ruleset().sync_gates, ["Brock", "Champion"]);

    coop.setup.choose(Preset::ClassicNuzlocke).unwrap();
    assert!(coop.setup.ruleset().sync_gates.is_empty(), "Classic Nuzlocke has none");
    coop.setup.choose(Preset::Hardcore).unwrap();
    assert_eq!(coop.setup.ruleset().sync_gates, gates);
}

/// A locked three-Player log with an entry from each Player.
fn three_player_log() -> Coop {
    let mut coop = Coop::with(&["Red", "Blue", "Green"]);
    coop.everyone_ready();
    coop.lock().unwrap();
    for player in &mut coop.players {
        let run = player.run();
        run.walks_into(route_1());
        run.meets_wild(RATTATA);
    }
    coop.sync();
    coop
}

#[test]
fn the_host_cant_change_an_entry() {
    let coop = three_player_log();
    let host = coop.players[0].key();
    let entries = coop.host_log.entries().to_vec();
    let blues = entries
        .iter()
        .position(|e| e.submission.author == coop.players[1].key())
        .unwrap();

    // A Host who rewrites Blue's Rattata as a Pidgey has to sign the entry
    // again as ordered; Blue's own signature still gives it away.
    let mut rewritten = entries[blues].submission.clone();
    let body = String::from_utf8(rewritten.body.clone()).unwrap();
    rewritten.body = body.replace("RATTATA", "PIDGEY").into_bytes();
    let mut lying_host = RunLog::verify(host, entries[..blues].to_vec()).unwrap();
    assert_eq!(
        lying_host.order(&Signer::from_secret([1; 32]), rewritten),
        Err(LogError::AuthorSignature { seq: blues as u64 })
    );

    // Changed in place, the Host's own signature no longer matches either.
    let mut tampered = entries;
    tampered[blues].submission.body = body.replace("RATTATA", "PIDGEY").into_bytes();
    assert_eq!(
        RunLog::verify(host, tampered).unwrap_err(),
        LogError::HostSignature { seq: blues as u64 }
    );
}

#[test]
fn the_host_cant_invent_an_entry_for_another_player() {
    let mut coop = three_player_log();
    let host_signer = Signer::from_secret([1; 32]);
    let blue = coop.players[1].key();
    let counter = coop.host_log.next_counter(&blue);
    let body = serde_json::to_vec(&LogBody::Event(RunEvent::PokeBallsObtained)).unwrap();
    // Signed by the Host, claiming to be Blue's.
    let mut forged = coop
        .host_log
        .sign(&host_signer, counter, &LogBody::Event(RunEvent::PokeBallsObtained));
    forged.author = blue;
    forged.body = body;
    assert_eq!(
        coop.host_log.order(&host_signer, forged),
        Err(LogError::AuthorSignature {
            seq: coop.host_log.entries().len() as u64
        })
    );
}

#[test]
fn reordered_dropped_or_replayed_entries_are_caught() {
    let coop = three_player_log();
    let host = coop.players[0].key();
    let entries = coop.host_log.entries().to_vec();

    let mut swapped = entries.clone();
    swapped.swap(2, 3);
    assert!(matches!(RunLog::verify(host, swapped), Err(LogError::Seq { .. })));

    let mut dropped = entries.clone();
    dropped.remove(2);
    assert!(matches!(
        RunLog::verify(host, dropped),
        Err(LogError::Seq { expected: 2, found: 3 })
    ));

    // An entry signed again with the same counter, ordered as a new one.
    let mut log = RunLog::verify(host, entries.clone()).unwrap();
    let replay = entries.last().unwrap().submission.clone();
    let host_signer = Signer::from_secret([1; 32]);
    assert!(matches!(
        log.order(&host_signer, replay),
        Err(LogError::Replayed { .. })
    ));

    // A Host who leaves out one of Blue's entries can't order the next.
    let blue = &coop.players[1];
    let next = log.next_counter(&blue.key());
    let left_out = LogBody::Event(RunEvent::GameReset);
    let _ = blue.log.sign(&blue.signer, next, &left_out);
    let after = blue.log.sign(&blue.signer, next + 1, &left_out);
    assert!(matches!(log.order(&host_signer, after), Err(LogError::Skipped { .. })));
}

#[test]
fn only_the_host_locks_and_only_the_roster_writes() {
    let mut coop = Coop::with(&["Red", "Blue"]);
    coop.everyone_ready();
    let locked = coop.setup.view();
    // Blue tries to write the lock.
    let fake_lock = pactlocke_run::Locked {
        run_name: locked.run_name,
        game_name: locked.game_name,
        game_sha1: locked.game_sha1,
        ruleset: locked.ruleset,
        roster: Vec::new(),
        start_ms: 0,
    };
    coop.player_submits(1, &LogBody::Locked(fake_lock));
    assert!(
        coop.players[1].refused[0].contains("can only come from the Host"),
        "{:?}",
        coop.players[1].refused
    );

    // Before the lock there is no roster: no Player's events yet.
    coop.player_submits(1, &LogBody::Event(RunEvent::PokeBallsObtained));
    assert!(coop.players[1].refused[1].contains("outside the roster"));

    // A stranger after the lock.
    coop.lock().unwrap();
    let stranger = Signer::from_secret([9; 32]);
    let entry = coop
        .host_log
        .sign(&stranger, 1, &LogBody::Event(RunEvent::PokeBallsObtained));
    let host_signer = Signer::from_secret([1; 32]);
    assert!(matches!(
        coop.host_log.order(&host_signer, entry),
        Err(LogError::NotInRoster { .. })
    ));
}

#[test]
fn a_log_from_another_host_or_run_doesnt_check() {
    let coop = three_player_log();
    let entries = coop.host_log.entries().to_vec();
    let someone_else = Signer::from_secret([42; 32]).public();
    assert!(matches!(
        RunLog::verify(someone_else, entries),
        Err(LogError::HostSignature { seq: 0 })
    ));
    assert_eq!(
        RunLog::verify(coop.players[0].key(), Vec::new()).unwrap_err(),
        LogError::NoGenesis
    );
}
