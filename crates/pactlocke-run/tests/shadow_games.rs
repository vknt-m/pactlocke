//! Seam 1: every app runs a Shadow Game of each other Player from
//! their input stream alone, through the real running game on the fake game
//! and fake core, with every message crossing as bytes.

mod support;

use pactlocke_emu::Keys;
use pactlocke_game::Change;
use pactlocke_run::{
    BUFFER_FRAMES, CHUNK_FRAMES, FrameHash, HASH_EVERY, InputChunk, PREDICT_FRAMES, Ruleset, ShadowState, Stamped,
    StreamMessage, StreamWant,
};
use support::streams::{FRAME_MS, Table, key};
use support::{PIDGEY, RATTATA, route_1};

fn first_encounter() -> Ruleset {
    Ruleset {
        first_encounter: true,
        ..Ruleset::default()
    }
}

#[test]
fn a_shadow_game_replays_a_players_game_and_matches_it_by_hash() {
    let mut table = Table::start(&["Red", "Blue"], first_encounter());

    // Red plays a while, uses Route 1's Encounter, then meets a second wild
    // Pokémon there: the Run patches the catch block on and off, stamped.
    table.plays(0, 300);
    table.walks_into(0, route_1());
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    table.meets_wild(0, RATTATA);
    assert!(table.game(0).catches_blocked);
    table.leaves_battle(0);
    table.plays(0, 900);
    let stamped = table.apps[0].running.engine().stamped().len();
    assert_eq!(stamped, 2, "the block on, and off");
    table.waits(1000);

    let shadow = table.apps[1].shadow_of(key(0));
    assert_eq!(shadow.state(), ShadowState::Matched);
    let played = 300 + 10 * 5 + 900;
    assert_eq!(shadow.received(), played - played % CHUNK_FRAMES as u64);
    let checked = (shadow.confirmed() - 1) / HASH_EVERY + 1;
    assert!(checked >= 9, "hashes at every {HASH_EVERY} frames: {checked}");
    assert_eq!(shadow.hashes_matched(), checked, "every hash both sides have matched");
}

#[test]
fn a_shadow_plays_about_thirty_frames_behind_the_newest_frame_received() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.plays(0, 600);
    let shadow = table.apps[1].shadow_of(key(0));
    let behind = shadow.received() - shadow.frame();
    assert!(
        (BUFFER_FRAMES - 2..=BUFFER_FRAMES + CHUNK_FRAMES as u64).contains(&behind),
        "{behind} frames behind"
    );
    assert_eq!(shadow.rewinds(), 0, "a steady stream needs no guesses");
}

#[test]
fn on_a_network_hiccup_a_shadow_keeps_playing_then_rewinds_and_matches() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.plays(0, 600);

    // Red's messages are held up for a second.
    table.lag_ms = 1000;
    let before = table.apps[1].shadow_of(key(0)).frame();
    table.plays(0, 30);
    let during = table.apps[1].shadow_of(key(0)).frame();
    assert!(during > before + 20, "never pauses: {before} -> {during}");
    table.plays(0, 30);
    table.lag_ms = 0;
    table.plays(0, 600);
    table.waits(500);

    let shadow = table.apps[1].shadow_of(key(0));
    assert!(shadow.rewinds() >= 1, "Red's keys changed, so the guesses were wrong");
    assert_eq!(shadow.state(), ShadowState::Matched);
    assert!(shadow.hashes_matched() >= 9, "{}", shadow.hashes_matched());
    assert!(shadow.confirmed() >= 1200, "caught up: {}", shadow.confirmed());
}

#[test]
fn a_lost_chunk_is_asked_for_and_resent_and_the_shadow_still_matches() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    let mut lost = 0;
    table.lose = Box::new(move |_, _, message| {
        let lose = matches!(message, StreamMessage::Chunk(c) if c.first_frame == 300 && lost == 0);
        lost += u32::from(lose);
        lose
    });
    table.plays(0, 900);
    table.waits(500);
    let shadow = table.apps[1].shadow_of(key(0));
    assert_eq!(shadow.state(), ShadowState::Matched);
    assert!(shadow.confirmed() >= 840, "{}", shadow.confirmed());
    assert!(shadow.hashes_matched() >= 7);
}

#[test]
fn with_three_players_the_host_passes_each_stream_on_and_fills_a_loss() {
    let mut table = Table::start(&["Red", "Blue", "Green"], Ruleset::default());
    // Blue's chunk at frame 300 never reaches the Host the first time.
    let mut lost = false;
    let (blue, host) = (key(1), key(0));
    table.lose = Box::new(move |from, to, message| {
        let lose =
            !lost && from == blue && to == host && matches!(message, StreamMessage::Chunk(c) if c.first_frame == 300);
        lost |= lose;
        lose
    });
    for _ in 0..60 {
        for i in 0..3 {
            table.plays(i, 10);
        }
    }
    table.waits(1000);
    for watcher in 0..3 {
        for watched in (0..3).filter(|&w| w != watcher) {
            let shadow = table.apps[watcher].shadow_of(key(watched));
            assert_eq!(shadow.state(), ShadowState::Matched, "{watcher} watching {watched}");
            assert!(
                shadow.confirmed() >= 570 && shadow.hashes_matched() >= 4,
                "{watcher} watching {watched}: {} frames, {} hashes",
                shadow.confirmed(),
                shadow.hashes_matched()
            );
        }
    }
}

#[test]
fn a_memory_edit_makes_the_shadow_diverge_and_the_watcher_is_told() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.plays(0, 200);
    table.edits_memory(0, 0x0200_1000, &[99]);
    table.plays(0, 400);
    table.waits(500);
    let shadow = table.apps[1].shadow_of(key(0));
    assert_eq!(shadow.state(), ShadowState::Diverged { frame: 240 });

    // Blue's game takes the news at its next frame.
    table.plays(1, 1);
    let feed = table.apps[1].view_feed();
    assert!(
        feed.iter()
            .any(|e| e == "Can't watch Red right now: their game and its copy here are out of sync"),
        "{feed:?}"
    );
}

#[test]
fn a_stamped_change_left_out_of_the_stream_makes_the_shadow_diverge() {
    let mut table = Table::start(&["Red", "Blue"], first_encounter());
    table.tamper = Box::new(|_, _, message| {
        if let StreamMessage::Chunk(chunk) = message {
            chunk.stamped.clear();
        }
    });
    table.walks_into(0, route_1());
    table.meets_wild(0, PIDGEY);
    table.leaves_battle(0);
    table.meets_wild(0, RATTATA);
    table.plays(0, 400);
    table.waits(500);
    assert!(matches!(
        table.apps[1].shadow_of(key(0)).state(),
        ShadowState::Diverged { .. }
    ));
}

#[test]
fn time_alone_never_runs_a_shadow_far_past_what_it_received() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.plays(0, 300);
    // Red stops (paused, or gone): Blue's Shadow guesses a little, then holds.
    table.waits(10_000);
    let shadow = table.apps[1].shadow_of(key(0));
    assert!(shadow.frame() <= shadow.received() + PREDICT_FRAMES);
    assert_eq!(shadow.state(), ShadowState::Matched);
}

#[test]
fn a_shadow_catches_up_at_about_twenty_times_the_players_pace() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.plays(0, 120);
    let per_tick = table.apps[1].shadow_of(key(0)).frames_for(FRAME_MS);
    assert!(
        (15..=25).contains(&per_tick),
        "about 20 frames per frame's time: {per_tick}"
    );
}

fn chunk() -> InputChunk {
    InputChunk {
        player: key(0),
        boot: 1,
        first_frame: 120,
        keys: vec![Keys::A, Keys::NONE, Keys::B | Keys::UP],
        stamped: vec![
            Stamped {
                frame: 121,
                change: Change::Write {
                    addr: 0x0202_4284,
                    bytes: vec![1, 2, 3],
                },
            },
            Stamped {
                frame: 121,
                change: Change::Patch {
                    addr: 0x0804_0F6C,
                    bytes: vec![1, 0x20, 0x70, 0x47],
                },
            },
        ],
        hashes: vec![FrameHash {
            frame: 120,
            ram: [7; 20],
            save: [8; 20],
        }],
        marks: vec![pactlocke_run::Mark { frame: 121, after: 1 }],
        booted_from: None,
    }
}

#[test]
fn stream_messages_round_trip_and_anything_malformed_is_dropped() {
    let messages = [
        StreamMessage::Chunk(chunk()),
        StreamMessage::Want(StreamWant {
            player: key(2),
            boot: 1,
            from_frame: 450,
        }),
    ];
    for message in messages {
        let bytes = message.to_bytes();
        assert_eq!(StreamMessage::from_bytes(&bytes), Some(message.clone()));
        for cut in 0..bytes.len() {
            assert_eq!(StreamMessage::from_bytes(&bytes[..cut]), None, "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(StreamMessage::from_bytes(&longer), None);
    }
    // A change stamped outside the chunk's frames, or a hash out of order.
    let mut outside = chunk();
    outside.stamped[0].frame = 123;
    assert_eq!(
        StreamMessage::from_bytes(&StreamMessage::Chunk(outside).to_bytes()),
        None
    );
    let mut early = chunk();
    early.hashes[0].frame = 119;
    assert_eq!(StreamMessage::from_bytes(&StreamMessage::Chunk(early).to_bytes()), None);

    // Random bytes, and random damage to a good message, never panic.
    let good = StreamMessage::Chunk(chunk()).to_bytes();
    let mut seed = 0x1234_5678_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for _ in 0..5000 {
        let mut bytes = good.clone();
        let at = next() as usize % bytes.len();
        bytes[at] = next() as u8;
        let _ = StreamMessage::from_bytes(&bytes);
        let noise: Vec<u8> = (0..next() % 200).map(|_| next() as u8).collect();
        let _ = StreamMessage::from_bytes(&noise);
    }
}

/// A Player's stream of `frames` frames on the fake core, as their running game builds it.
fn stream_of(frames: u64) -> Vec<InputChunk> {
    use pactlocke_emu::CoreAdapter;
    let mut core = pactlocke_game::FakeCore::default();
    core.set_clock(support::START_MS);
    let mut out = pactlocke_run::StreamOut::new(key(0), 1);
    let mut chunks = Vec::new();
    for frame in 0..frames {
        if frame.is_multiple_of(HASH_EVERY) {
            out.hash(FrameHash::of(&mut core, frame));
        }
        let keys = Keys((frame * 37 % 0x400) as u16);
        core.run_frame(keys);
        chunks.extend(out.frame(keys, &[]));
    }
    chunks
}

#[derive(Clone, Default)]
struct Heard(std::sync::Arc<std::sync::Mutex<usize>>);

impl pactlocke_emu::AudioSink for Heard {
    fn push(&mut self, samples: &[i16], _sample_rate: u32) {
        *self.0.lock().unwrap() += samples.len();
    }
}

#[track_caller]
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !done() {
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "{what}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn in_battery_saver_a_shadow_waits_until_watched_then_catches_up_with_picture_and_sound() {
    use pactlocke_run::{ShadowGame, ShadowRunner, SystemRunClock};
    let shadow = ShadowGame::power_on(pactlocke_game::FakeCore::default(), support::START_MS);
    let runner = ShadowRunner::spawn(shadow, Box::new(SystemRunClock), Box::new(|_| {}));
    runner.battery_saver(true);
    for chunk in stream_of(600) {
        runner.receive(chunk);
    }
    until("the stream arrived", || runner.status().received == 600);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(runner.status().frame, 0, "not watched: it waits");
    assert!(runner.frames().latest().is_none());

    let heard = Heard::default();
    runner.watch(Box::new(heard.clone()));
    until("caught up while watched", || {
        runner.status().frame >= 600 - BUFFER_FRAMES
    });
    assert_eq!(runner.status().state, ShadowState::Matched);
    assert!(runner.frames().latest().is_some(), "Spectate has its picture");
    assert!(*heard.0.lock().unwrap() > 0, "and its sound");
}

#[test]
fn without_battery_saver_a_shadow_always_runs() {
    use pactlocke_run::{ShadowGame, ShadowRunner, SystemRunClock};
    let shadow = ShadowGame::power_on(pactlocke_game::FakeCore::default(), support::START_MS);
    let runner = ShadowRunner::spawn(shadow, Box::new(SystemRunClock), Box::new(|_| {}));
    for chunk in stream_of(300) {
        runner.receive(chunk);
    }
    until("caught up unwatched", || runner.status().frame >= 300 - BUFFER_FRAMES);
    assert!(
        runner.frames().latest().is_none(),
        "nothing published while nobody watches"
    );
}

#[test]
fn a_shadow_thread_reports_divergence_once() {
    use pactlocke_run::{ShadowGame, ShadowRunner, SystemRunClock};
    let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let shadow = ShadowGame::power_on(pactlocke_game::FakeCore::default(), support::START_MS);
    let runner = {
        let told = told.clone();
        ShadowRunner::spawn(
            shadow,
            Box::new(SystemRunClock),
            Box::new(move |frame| told.lock().unwrap().push(frame)),
        )
    };
    let mut chunks = stream_of(600);
    // A hash that can't match, as from a game edited in memory.
    let hashed = chunks
        .iter_mut()
        .find(|c| c.hashes.iter().any(|h| h.frame == 240))
        .unwrap();
    hashed.hashes[0].ram = [0; 20];
    for chunk in chunks {
        runner.receive(chunk);
    }
    until("diverged", || {
        matches!(runner.status().state, ShadowState::Diverged { .. })
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(*told.lock().unwrap(), [240]);
}

#[test]
fn many_changes_at_one_boundary_still_cross_in_messages_the_wire_carries() {
    // A boundary with 6 KiB of stamped changes after 14 frames of keys.
    let mut out = pactlocke_run::StreamOut::new(key(0), 1);
    let mut chunks = Vec::new();
    for frame in 0..14 {
        chunks.extend(out.frame(Keys::NONE, &[]));
        let _ = frame;
    }
    let big: Vec<Stamped> = (0..6)
        .map(|i| Stamped {
            frame: 14,
            change: Change::Write {
                addr: 0x0202_0000 + i * 0x400,
                bytes: vec![i as u8; 1024],
            },
        })
        .collect();
    chunks.extend(out.stamp(&big));
    for _ in 0..30 {
        chunks.extend(out.frame(Keys::A, &[]));
    }
    for chunk in &chunks {
        let bytes = StreamMessage::Chunk(chunk.clone()).to_bytes();
        assert!(bytes.len() <= 15 * 1024, "{} bytes", bytes.len());
        assert_eq!(
            StreamMessage::from_bytes(&bytes),
            Some(StreamMessage::Chunk(chunk.clone()))
        );
    }
    let stamped: usize = chunks.iter().map(|c| c.stamped.len()).sum();
    assert_eq!(stamped, 6, "every change went out");
}

#[test]
fn when_a_player_boots_their_game_again_the_shadow_says_it_cant_be_watched_yet() {
    let mut shadow = pactlocke_run::ShadowGame::power_on(pactlocke_game::FakeCore::default(), support::START_MS);
    for chunk in stream_of(60) {
        shadow.receive(&chunk, 0);
    }
    let mut again = stream_of(15).remove(0);
    again.boot = 2;
    shadow.receive(&again, 100);
    assert_eq!(shadow.state(), ShadowState::Restarted { boot: 2 });
}

#[test]
fn a_diverged_shadow_keeps_nothing_more_of_the_stream() {
    let mut shadow = pactlocke_run::ShadowGame::power_on(pactlocke_game::FakeCore::default(), support::START_MS);
    let mut chunks = stream_of(600);
    chunks[0].hashes[0].ram = [0; 20];
    shadow.receive(&chunks[0], 0);
    shadow.advance(10_000, 100);
    assert!(matches!(shadow.state(), ShadowState::Diverged { frame: 0 }));
    let received = shadow.received();
    for chunk in &chunks[1..] {
        shadow.receive(chunk, 20_000);
    }
    assert_eq!(shadow.received(), received);
}
