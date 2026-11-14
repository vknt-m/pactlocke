//! The lobby over real iroh endpoints on this machine, with relays off: a
//! friend with the invite joins and shows in the lobby with their ROM
//! checked; a stranger who made up a secret is refused.

use std::time::{Duration, Instant};

use pactlocke_net::{
    APP_VERSION, HostLobby, HostNode, Invite, JoinLobby, JoinNode, LobbyView, Relays, Replaced, RomStatus, RunAdvert,
    RunSecret,
};

const FIRERED: &str = "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc";

fn key(byte: u8) -> [u8; 32] {
    // Any 32 bytes are an ed25519 secret key.
    [byte; 32]
}

/// Waits until `done` says the view is as wanted, or fails after a while.
#[track_caller]
fn wait_for(what: &str, mut view: impl FnMut() -> LobbyView, done: impl Fn(&LobbyView) -> bool) -> LobbyView {
    let start = Instant::now();
    loop {
        let now = view();
        if done(&now) {
            return now;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "{what}: {now:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn host() -> HostNode {
    let lobby = HostLobby::new(
        RunAdvert {
            name: "Kanto with friends".into(),
            game_name: "Pokémon FireRed (USA) 1.0".into(),
            game_sha1: FIRERED.into(),
        },
        "Red",
        APP_VERSION,
    );
    HostNode::start(key(1), lobby, Relays::Off)
}

#[test]
fn a_friend_with_the_invite_joins_over_iroh_and_a_stranger_is_refused() {
    let host = host();
    let view = wait_for("the invite", || host.view(), |v| v.invite_link.is_some());
    let invite = Invite::parse(view.invite_link.as_deref().unwrap()).unwrap();
    assert!(invite.relay.is_none(), "relays are off for this test");
    let direct = wait_for_addrs(&host);

    let blue = JoinLobby::new(invite.clone(), "Blue", APP_VERSION, Some(FIRERED.into())).unwrap();
    let blue = JoinNode::start_with(key(2), blue, Relays::Off, direct.clone());
    // Blue shows in the lobby first, then with their ROM checked: a later
    // update, which under load can come a moment after.
    let joined = wait_for(
        "Blue in the lobby with their ROM checked",
        || blue.view(),
        |v| v.players.len() == 2 && v.players[1].rom != RomStatus::NotYet,
    );
    assert_eq!(joined.players[0].name, "Red");
    assert_eq!(joined.players[1].name, "Blue");
    assert_eq!(joined.players[1].rom, RomStatus::Matches);
    assert_eq!(joined.rom_matches, Some(true));
    wait_for("Blue in the Host's lobby", || host.view(), |v| v.players.len() == 2);

    // A stranger with the Host's key but a secret of their own making.
    let forged = Invite {
        secret: RunSecret::generate(),
        ..invite
    };
    let mallory = JoinLobby::new(forged, "Mallory", APP_VERSION, Some(FIRERED.into())).unwrap();
    let mallory = JoinNode::start_with(key(3), mallory, Relays::Off, direct);
    let refused = wait_for("Mallory refused", || mallory.view(), |v| v.error.is_some());
    assert!(
        refused
            .error
            .as_deref()
            .unwrap()
            .contains("invite was already used or isn't for this Run"),
        "{refused:?}"
    );
    assert!(refused.players.is_empty());
    assert_eq!(host.view().players.len(), 2, "still only Red and Blue");
}

#[test]
fn a_friend_with_the_wrong_rom_is_shown_so() {
    let host = host();
    let view = wait_for("the invite", || host.view(), |v| v.invite_link.is_some());
    let invite = Invite::parse(view.invite_link.as_deref().unwrap()).unwrap();
    let direct = wait_for_addrs(&host);
    let green = JoinLobby::new(invite, "Green", APP_VERSION, None).unwrap();
    let green = JoinNode::start_with(key(4), green, Relays::Off, direct);
    wait_for("Green in the lobby", || green.view(), |v| v.players.len() == 2);

    green.rom_picked("574fa542ffebb14be69902d1d36f1ec0a4afd71e");
    let seen = wait_for(
        "the wrong ROM",
        || host.view(),
        |v| v.players.get(1).is_some_and(|p| p.rom == RomStatus::Wrong),
    );
    assert_eq!(seen.players[1].name, "Green");
    wait_for("Green sees it too", || green.view(), |v| v.rom_matches == Some(false));
}

#[test]
fn a_player_reconnecting_while_their_old_connection_lingers_is_live_on_the_new_one() {
    let host = host();
    let view = wait_for("the invite", || host.view(), |v| v.invite_link.is_some());
    let invite = Invite::parse(view.invite_link.as_deref().unwrap()).unwrap();
    let direct = wait_for_addrs(&host);
    let blue = JoinLobby::new(invite.clone(), "Blue", APP_VERSION, None).unwrap();
    let first = JoinNode::start_with(key(5), blue.clone(), Relays::Off, direct.clone());
    wait_for("Blue in the lobby", || first.view(), |v| v.players.len() == 2);

    // The same key again, the first connection still open: it is in the roster.
    let second = JoinNode::start_with(key(5), blue, Relays::Off, direct);
    wait_for("Blue back in the lobby", || second.view(), |v| v.players.len() == 2);
    drop(first);
    second.rom_picked(FIRERED);
    let seen = wait_for(
        "Blue's ROM from the new connection",
        || host.view(),
        |v| v.players.get(1).is_some_and(|p| p.rom == RomStatus::Matches),
    );
    assert_eq!(seen.players.len(), 2, "still in, on the new connection");
}

fn wait_for_addrs(host: &HostNode) -> Vec<std::net::SocketAddr> {
    let start = Instant::now();
    loop {
        let addrs = host.direct_addrs();
        if !addrs.is_empty() {
            return addrs;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "the Host never bound");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A node that takes another's place on the same key starts once the old
/// one has shut down, even one still trying to reach a Host that never
/// answers: the relay never sees two connections on one key.
#[test]
fn a_node_replacing_another_on_the_same_key_joins_once_the_old_one_has_shut_down() {
    let host = host();
    let view = wait_for("the invite", || host.view(), |v| v.invite_link.is_some());
    let invite = Invite::parse(view.invite_link.as_deref().unwrap()).unwrap();
    let direct = wait_for_addrs(&host);

    // Blue first dials an address where nothing answers, for 30 seconds.
    let nowhere = Invite {
        host: iroh_public(key(9)),
        ..invite.clone()
    };
    let lost = JoinLobby::new(nowhere, "Blue", APP_VERSION, None).unwrap();
    let lost = JoinNode::start_with(key(7), lost, Relays::Off, vec!["192.0.2.1:9".parse().unwrap()]);
    std::thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    let blue = JoinLobby::new(invite, "Blue", APP_VERSION, None).unwrap();
    let blue = JoinNode::start_replacing(key(7), blue, Relays::Off, direct, Replaced::from(lost));
    assert!(started.elapsed() < Duration::from_millis(100), "the caller never waits");
    wait_for("Blue in the lobby", || blue.view(), |v| v.players.len() == 2);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the old node stopped trying at once: {:?}",
        started.elapsed()
    );
}

fn iroh_public(secret: [u8; 32]) -> pactlocke_net::PeerKey {
    pactlocke_net::PeerKey(*iroh::SecretKey::from_bytes(&secret).public().as_bytes())
}

fn run() -> RunAdvert {
    RunAdvert {
        name: "Kanto with friends".into(),
        game_name: "Pokémon FireRed (USA) 1.0".into(),
        game_sha1: FIRERED.into(),
    }
}

/// An invite as a link from before the block was.
fn old_link(invite: &Invite) -> String {
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "pactlocke://join?host={}&secret={}&v={}",
        invite.host,
        hex(&invite.secret.0),
        invite.app_version
    )
}

/// Through n0's public relays, as friends on different networks join: the
/// link carries the Host's key and no address or relay, and the joiner finds
/// the Host by key. One friend joins with the link, one with a link of the
/// old kind. It reaches the internet, so it is run by hand:
/// `cargo test -p pactlocke-net -- --ignored`.
#[test]
#[ignore = "reaches n0's public relays"]
fn friends_join_through_n0_with_the_link_and_with_an_old_one() {
    let host = HostNode::start(
        pactlocke_net::new_key(),
        HostLobby::new(run(), "Red", APP_VERSION),
        Relays::N0,
    );
    let view = wait_for(
        "the invite",
        || host.view(),
        |v| v.invite_link.is_some() || v.error.is_some(),
    );
    let link = view.invite_link.expect("an invite");
    assert_eq!(link.len(), 78, "{link}");
    let invite = Invite::parse(&link).unwrap();
    assert_eq!(invite.relay, None, "n0's relays are left out: {link}");

    let blue = JoinLobby::new(invite, "Blue", APP_VERSION, Some(FIRERED.into())).unwrap();
    let blue = JoinNode::start(pactlocke_net::new_key(), blue, Relays::N0);
    let joined = wait_for(
        "Blue in the lobby",
        || blue.view(),
        |v| v.players.len() == 2 || v.error.is_some(),
    );
    assert_eq!(joined.error, None);
    assert_eq!(joined.players[1].name, "Blue");

    host.new_invite();
    let view = wait_for(
        "a second invite",
        || host.view(),
        |v| v.invite_link.as_ref() != Some(&link),
    );
    let old = old_link(&Invite::parse(view.invite_link.as_deref().unwrap()).unwrap());
    let green = JoinLobby::new(Invite::parse(&old).unwrap(), "Green", APP_VERSION, None).unwrap();
    let green = JoinNode::start(pactlocke_net::new_key(), green, Relays::N0);
    let joined = wait_for(
        "Green in the lobby",
        || green.view(),
        |v| v.players.len() == 3 || v.error.is_some(),
    );
    assert_eq!(joined.error, None);
    assert_eq!(joined.players[2].name, "Green");
}

#[test]
fn setup_and_log_cross_iroh_and_after_the_lock_nobody_new_joins() {
    use pactlocke_net::{Message, PeerKey};

    let host = host();
    let view = wait_for("the invite", || host.view(), |v| v.invite_link.is_some());
    let invite = Invite::parse(view.invite_link.as_deref().unwrap()).unwrap();
    let direct = wait_for_addrs(&host);
    let blue = JoinLobby::new(invite, "Blue", APP_VERSION, Some(FIRERED.into())).unwrap();
    let blue = JoinNode::start_with(key(6), blue, Relays::Off, direct.clone());
    wait_for("Blue in the lobby", || blue.view(), |v| v.players.len() == 2);

    // The roster names Blue by the key the Run log will check Blue's signatures with.
    let roster = wait_for("Blue in the roster", || host.view(), |v| !v.roster.is_empty()).roster;
    let blue_key = PeerKey(*iroh::SecretKey::from_bytes(&key(6)).public().as_bytes());
    assert_eq!(roster, [(blue_key, "Blue".to_owned())]);
    assert_eq!(
        pactlocke_run::Signer::from_secret(key(6)).public(),
        blue_key.0,
        "the Run log key is the network key"
    );

    blue.send(Message::Setup(b"ready".to_vec()));
    let start = Instant::now();
    let mut inbox = Vec::new();
    while inbox.is_empty() && start.elapsed() < Duration::from_secs(30) {
        inbox = host.take_inbox();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(inbox, [(blue_key, Message::Setup(b"ready".to_vec()))]);

    host.send(None, Message::Log(b"entry".to_vec()));
    let start = Instant::now();
    let mut inbox = Vec::new();
    while inbox.is_empty() && start.elapsed() < Duration::from_secs(30) {
        inbox = blue.take_inbox();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(inbox, [Message::Log(b"entry".to_vec())]);

    // On one machine the connection goes straight, never through a relay.
    let link = blue.link().expect("Blue's link to the Host");
    assert!(link.direct, "{link:?}");
    let links = host.links();
    assert!(
        links.iter().any(|(peer, link)| *peer == blue_key && link.direct),
        "{links:?}"
    );

    host.lock_roster();
    wait_for("no invite after the lock", || host.view(), |v| v.invite_link.is_none());
}

/// Large messages arrive whole and in order while the joiner sends its own.
/// (The cut-short read this guards against needs a slow network to show:
/// `node::tests` reproduces it, and the test rig runs it over a shaped one.)
#[test]
fn large_messages_arrive_whole_while_the_joiner_sends() {
    let _ = log::set_logger(&Stderr).map(|()| log::set_max_level(log::LevelFilter::Debug));
    let host = host();
    let view = wait_for("the invite", || host.view(), |v| v.invite_link.is_some());
    let invite = Invite::parse(view.invite_link.as_deref().unwrap()).unwrap();
    let direct = wait_for_addrs(&host);
    let blue = JoinLobby::new(invite, "Blue", APP_VERSION, Some(FIRERED.into())).unwrap();
    let blue = JoinNode::start_with(key(2), blue, Relays::Off, direct);
    wait_for("Blue in the Host's lobby", || host.view(), |v| v.players.len() == 2);

    // Input streams flow once the Run is locked.
    host.lock_roster();
    std::thread::sleep(Duration::from_millis(200));
    const SENT: usize = 200;
    for i in 0..SENT {
        let big: Vec<u8> = (0..15_000).map(|b| (b + i) as u8).collect();
        host.send(None, pactlocke_net::Message::Input(big));
        // The joiner sends meanwhile, within the Host's rate for a Player.
        blue.send(pactlocke_net::Message::Input(vec![i as u8; 40]));
        std::thread::sleep(Duration::from_millis(25));
    }
    let start = Instant::now();
    let mut got = Vec::new();
    while got.len() < SENT {
        assert!(
            blue.view().error.is_none(),
            "Blue lost the Host: {:?}",
            blue.view().error
        );
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "{} of {SENT} arrived",
            got.len()
        );
        got.extend(blue.take_inbox());
        std::thread::sleep(Duration::from_millis(5));
    }
    for (i, message) in got.iter().enumerate() {
        let pactlocke_net::Message::Input(bytes) = message else {
            panic!("message {i} isn't input: {message:?}");
        };
        assert_eq!(bytes.len(), 15_000, "message {i} whole");
        assert_eq!(bytes[1], (1 + i) as u8, "message {i} in order");
    }
}

struct Stderr;
impl log::Log for Stderr {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("pactlocke")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("LOG {} {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

/// A Player's game streams on while they are offline. Input sent before the
/// connection is up is dropped (the Host asks again for what it misses), not
/// queued and flushed all at once, which floods the Host into closing them.
#[test]
fn input_sent_while_connecting_is_dropped_not_flushed_as_a_flood() {
    let host = host();
    let view = wait_for("the invite", || host.view(), |v| v.invite_link.is_some());
    let invite = Invite::parse(view.invite_link.as_deref().unwrap()).unwrap();
    let direct = wait_for_addrs(&host);
    let blue = JoinLobby::new(invite, "Blue", APP_VERSION, Some(FIRERED.into())).unwrap();
    let blue = JoinNode::start_with(key(2), blue, Relays::Off, direct);
    for i in 0..500 {
        blue.send(pactlocke_net::Message::Input(vec![i as u8; 40]));
    }
    wait_for(
        "Blue connected",
        || blue.view(),
        |v| !v.players.is_empty() || v.error.is_some(),
    );
    std::thread::sleep(Duration::from_secs(1));
    assert!(blue.view().error.is_none(), "Blue was closed: {:?}", blue.view().error);
    let arrived = host
        .take_inbox()
        .iter()
        .filter(|(_, m)| matches!(m, pactlocke_net::Message::Input(_)))
        .count();
    assert_eq!(arrived, 0, "none of the input sent while connecting");
}
