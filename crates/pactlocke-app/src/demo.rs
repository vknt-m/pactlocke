//! A made-up three-Player Run, for a developer to look at the side panel
//! with (`--demo-coop ROM`): a Run log with Encounters, a Link that broke, a
//! Sync Gate ready-up and a Retry vote, and what each other Player's Shadow
//! Game would have shown. The real panel draws it. Nothing here plays the
//! others' games or reaches the network.

use pactlocke_game::LevelCap;
use pactlocke_run::{
    AreaBattle, AreaEncounter, AreaRef, AreaState, AreaStatus, CheckpointReason, DeathCause, Doing, FeedEntry, Grave,
    LevelCapView, Locked, LogBody, Met, PlayerKey, Recorded, RosterEntry, Ruleset, RunEvent, RunLog, RunView, Seen,
    SeenPokemon, ShadowState, ShadowStatus, Signer, SourceRef,
};

use crate::player_strip::Reach;

/// The made-up Run: its log, this Player's side of it, and the others'.
pub struct Demo {
    pub log: RunLog,
    /// What this Player's own Run would show.
    pub view: RunView,
    /// What each other Player's Shadow Game would show.
    pub shadows: Vec<(PlayerKey, ShadowStatus)>,
    /// How each other Player's app is reached.
    pub reaches: Vec<(PlayerKey, Reach)>,
}

/// A Pokémon in the demo: its PID, Emerald's species number, its species
/// name and the nickname it was given, if any.
#[derive(Clone, Copy)]
struct Mon(u32, u16, &'static str, &'static str);

const MUDKIP: Mon = Mon(0xA1, 283, "MUDKIP", "Swampy");
const ZIGZAGOON: Mon = Mon(0xA2, 288, "ZIGZAGOON", "Zigs");
const WURMPLE: Mon = Mon(0xA3, 290, "WURMPLE", "");
const WINGULL: Mon = Mon(0xA4, 309, "WINGULL", "");
const TAILLOW: Mon = Mon(0xA5, 304, "TAILLOW", "Swift");
const TORCHIC: Mon = Mon(0xB1, 280, "TORCHIC", "");
const POOCHYENA: Mon = Mon(0xB2, 286, "POOCHYENA", "Fang");
const LOTAD: Mon = Mon(0xB3, 295, "LOTAD", "");
const BLUE_WINGULL: Mon = Mon(0xB4, 309, "WINGULL", "Gale");
const TREECKO: Mon = Mon(0xC1, 277, "TREECKO", "Leafy");
const GREEN_WURMPLE: Mon = Mon(0xC2, 290, "WURMPLE", "");
const SEEDOT: Mon = Mon(0xC3, 298, "SEEDOT", "");
const SHROOMISH: Mon = Mon(0xC4, 306, "SHROOMISH", "");

impl Mon {
    fn name(self) -> &'static str {
        if self.3.is_empty() { self.2 } else { self.3 }
    }

    fn seen(self, level: u8, hp: u16, max_hp: u16) -> SeenPokemon {
        SeenPokemon {
            pid: self.0,
            species: self.1,
            species_name: self.2.to_owned(),
            nickname: self.name().to_owned(),
            level,
            hp,
            max_hp,
        }
    }
}

fn area(id: u16, name: &str) -> AreaRef {
    AreaRef {
        id,
        name: name.to_owned(),
    }
}

/// A Gift of the starter, as the log has it.
fn starter(mon: Mon, at_ms: i64) -> RunEvent {
    RunEvent::Gift {
        pid: mon.0,
        species: mon.1,
        species_name: mon.2.to_owned(),
        area: None,
        source: Some(SourceRef {
            key: "starter".to_owned(),
            name: "Starter".to_owned(),
        }),
        starter: true,
        egg: false,
        at_ms,
    }
}

/// An Area's Encounter caught: the Encounter, the catch, and its end.
fn caught(area: &AreaRef, mon: Mon, at_ms: i64) -> [RunEvent; 3] {
    [
        RunEvent::Encounter {
            area: area.clone(),
            species: mon.1,
            species_name: mon.2.to_owned(),
        },
        RunEvent::Caught {
            area: Some(area.clone()),
            pid: mon.0,
            species: mon.1,
            species_name: mon.2.to_owned(),
            at_ms,
        },
        RunEvent::EncounterOver {
            source: SourceRef::area(area),
            pid: Some(mon.0),
        },
    ]
}

fn died(mon: Mon, area: &AreaRef, cause: DeathCause, at_ms: i64) -> RunEvent {
    RunEvent::Died {
        pid: mon.0,
        species: mon.1,
        species_name: mon.2.to_owned(),
        name: mon.name().to_owned(),
        area: Some(area.clone()),
        cause,
        at_ms,
    }
}

fn checkpoint(reason: CheckpointReason, frame: u64) -> RunEvent {
    RunEvent::Checkpoint {
        reason,
        hash: "demo".to_owned(),
        boot: 1,
        frame,
    }
}

/// The made-up Run, with this install (`secret`, called `name`) as its Host
/// and two more Players, on a game with these level caps and Sync Gates.
pub fn coop(
    secret: [u8; 32],
    name: &str,
    game_name: &str,
    game_sha1: &str,
    level_caps: &[LevelCap],
    sync_gates: Vec<String>,
    now_ms: i64,
) -> Result<Demo, String> {
    let (red, blue, green) = (
        Signer::from_secret(secret),
        Signer::from_secret([0xB1; 32]),
        Signer::from_secret([0xC1; 32]),
    );
    let first_gate = sync_gates.first().cloned();
    let ruleset = Ruleset {
        first_encounter: true,
        faint_is_dead: true,
        dupes_clause: true,
        gifts: true,
        links: true,
        must_nickname: true,
        level_caps: (!level_caps.is_empty()).then(|| level_caps.to_vec()),
        sync_gates,
        retries: 3,
        ..Ruleset::default()
    };
    let locked = Locked {
        run_name: "Hoenn with friends".to_owned(),
        game_name: game_name.to_owned(),
        game_sha1: game_sha1.to_owned(),
        ruleset,
        roster: [(&red, name), (&blue, "Blue"), (&green, "Green")]
            .into_iter()
            .map(|(signer, name)| RosterEntry {
                key: signer.public(),
                name: name.to_owned(),
            })
            .collect(),
        start_ms: now_ms - 40 * MINUTE,
    };

    let (r101, r102, r103, r104, woods) = (
        area(0x10, "Route 101"),
        area(0x11, "Route 102"),
        area(0x12, "Route 103"),
        area(0x13, "Route 104"),
        area(0x3B, "Petalburg Woods"),
    );
    let ago = |minutes: i64| now_ms - minutes * MINUTE;
    let link_died = || DeathCause::Link {
        why: format!("{name}'s WURMPLE died"),
    };

    // This Player's own events, oldest first, each with when it happened.
    let mut mine: Vec<(i64, RunEvent)> = vec![(ago(38), starter(MUDKIP, ago(38)))];
    mine.extend(caught(&r101, ZIGZAGOON, ago(33)).map(|e| (ago(33), e)));
    mine.extend(caught(&r102, WURMPLE, ago(27)).map(|e| (ago(27), e)));
    mine.extend(caught(&r103, WINGULL, ago(21)).map(|e| (ago(21), e)));
    mine.push((ago(16), checkpoint(CheckpointReason::InGameSave, 40_000)));
    mine.extend(caught(&r104, TAILLOW, ago(9)).map(|e| (ago(9), e)));
    let foe = DeathCause::Wild {
        foe: "POOCHYENA".to_owned(),
    };
    mine.push((ago(2), died(WURMPLE, &r104, foe, ago(2))));

    let mut blues: Vec<(i64, RunEvent)> = vec![(ago(38), starter(TORCHIC, ago(38)))];
    blues.extend(caught(&r101, POOCHYENA, ago(32)).map(|e| (ago(32), e)));
    blues.extend(caught(&r102, LOTAD, ago(26)).map(|e| (ago(26), e)));
    blues.extend(caught(&r103, BLUE_WINGULL, ago(20)).map(|e| (ago(20), e)));
    blues.push((ago(16), checkpoint(CheckpointReason::Joined { call: 1 }, 39_000)));
    blues.push((ago(2), died(LOTAD, &woods, link_died(), ago(2))));
    blues.push((ago(1), RunEvent::RetryVote { call: 1, yes: true }));

    let mut greens: Vec<(i64, RunEvent)> = vec![(ago(38), starter(TREECKO, ago(38)))];
    greens.extend(caught(&r101, GREEN_WURMPLE, ago(34)).map(|e| (ago(34), e)));
    greens.extend(caught(&r102, SEEDOT, ago(25)).map(|e| (ago(25), e)));
    greens.push((ago(16), checkpoint(CheckpointReason::Joined { call: 1 }, 41_000)));
    greens.extend(caught(&woods, SHROOMISH, ago(6)).map(|e| (ago(6), e)));
    greens.push((ago(2), died(SEEDOT, &woods, link_died(), ago(2))));
    if let Some(gate) = first_gate.clone() {
        greens.push((ago(0), RunEvent::GateReady { gate }));
    }

    // The log: the Host's genesis and lock, then everyone's events in time order.
    let mut log = RunLog::new(red.public());
    let mut say = |signer: &Signer, body: LogBody| {
        let submission = log.sign(signer, log.next_counter(&signer.public()), &body);
        log.order(&red, submission)
            .map(|_| ())
            .map_err(|e| format!("The demo's Run log: {e}"))
    };
    say(
        &red,
        LogBody::Genesis {
            nonce: [7; 16],
            run_name: locked.run_name.clone(),
            game_name: locked.game_name.clone(),
            game_sha1: locked.game_sha1.clone(),
        },
    )?;
    say(&red, LogBody::Locked(locked))?;
    let mut all: Vec<(i64, &Signer, &RunEvent)> = Vec::new();
    for (signer, events) in [(&red, &mine), (&blue, &blues), (&green, &greens)] {
        all.extend(events.iter().map(|(at, event)| (*at, signer, event)));
    }
    all.sort_by_key(|(at, ..)| *at);
    for (_, signer, event) in all {
        // Only Run state goes in the shared log.
        if event.is_run_state() {
            say(signer, LogBody::Event(event.clone()))?;
        }
    }

    // This Player's side: the feed, Areas and Graveyard their own events make.
    let mut view = RunView {
        name: "Hoenn with friends".to_owned(),
        game_name: game_name.to_owned(),
        party: vec![
            MUDKIP.seen(14, 31, 39),
            ZIGZAGOON.seen(9, 6, 27),
            WINGULL.seen(8, 24, 24),
            TAILLOW.seen(7, 22, 22),
        ],
        area: Some(r104.name.clone()),
        doing: Doing::Exploring,
        level_cap: level_caps.first().map(|cap| LevelCapView {
            level: cap.level,
            next: Some(cap.name.clone()),
            soft: false,
        }),
        run_checkpoint: true,
        ..RunView::default()
    };
    for (seq, (at_ms, event)) in mine.iter().enumerate() {
        let recorded = Recorded {
            seq: seq as u64 + 1,
            boot: 1,
            frame: seq as u64 * 5_000,
            at_ms: *at_ms,
            event: event.clone(),
        };
        AreaEncounter::take(&mut view.encounters, event);
        view.feed.extend(FeedEntry::of(&recorded));
        view.graveyard.extend(Grave::of(&recorded));
    }
    view.feed.reverse();
    // Route 104's Encounter is used, with wild battles since.
    let battles = [POOCHYENA, WURMPLE, POOCHYENA, ZIGZAGOON, LOTAD, POOCHYENA, WINGULL];
    view.area_status = Some(AreaStatus {
        area: r104.name.clone(),
        state: AreaState::EncounterDone,
        encounter: view.encounters.iter().rev().find(|e| e.area == r104.name).cloned(),
        battles: battles
            .iter()
            .map(|mon| AreaBattle {
                species: mon.1,
                species_name: mon.2.to_owned(),
                met: Met::AfterEncounter,
            })
            .collect(),
        met: battles.len() + 1,
    });

    // What the others' Shadow Games would have shown: every Pokémon they
    // caught, the ones that fainted, and their parties now.
    let shadow = |owned: &[Mon], fainted: Mon, party: Vec<SeenPokemon>, area: &AreaRef, doing: Doing| ShadowStatus {
        state: ShadowState::Matched,
        catching_up: false,
        frame: 120_000,
        received: 120_030,
        boot: 1,
        seen: Seen {
            owned: owned.iter().map(|m| m.0).collect(),
            fainted: [fainted.0].into(),
            party,
            area: Some(area.name.clone()),
            doing,
            avatar: None,
        },
    };
    let shadows = vec![
        (
            blue.public(),
            shadow(
                &[TORCHIC, POOCHYENA, LOTAD, BLUE_WINGULL],
                LOTAD,
                vec![
                    TORCHIC.seen(15, 28, 41),
                    POOCHYENA.seen(8, 26, 26),
                    BLUE_WINGULL.seen(7, 20, 20),
                ],
                &woods,
                Doing::InBattle,
            ),
        ),
        (
            green.public(),
            shadow(
                &[TREECKO, GREEN_WURMPLE, SEEDOT, SHROOMISH],
                SEEDOT,
                vec![
                    TREECKO.seen(13, 35, 38),
                    GREEN_WURMPLE.seen(6, 21, 21),
                    SHROOMISH.seen(6, 23, 23),
                ],
                &area(0x0A, "Rustboro City"),
                Doing::Exploring,
            ),
        ),
    ];
    let reaches = vec![
        (blue.public(), Reach::Direct { ping_ms: 38 }),
        (green.public(), Reach::Relayed { ping_ms: 142 }),
    ];
    Ok(Demo {
        log,
        view,
        shadows,
        reaches,
    })
}

const MINUTE: i64 = 60_000;
