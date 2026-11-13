//! The lobby over real iroh endpoints on this machine, with relays off: a
//! friend with the invite joins and shows in the lobby with their ROM
//! checked; a stranger who made up a secret is refused.

use std::time::{Duration, Instant};

use pactlocke_net::{
    APP_VERSION, HostLobby, HostNode, Invite, JoinLobby, JoinNode, LobbyView, Relays, RomStatus, RunAdvert, RunSecret,
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

/// Through n0's public relays, as friends on different networks join: the
/// invite carries the Host's relay and no address. It reaches the internet,
/// so it is run by hand: `cargo test -p pactlocke-net -- --ignored`.
#[test]
#[ignore = "reaches n0's public relays"]
fn a_friend_joins_through_the_relay_in_the_invite() {
    let lobby = HostLobby::new(
        RunAdvert {
            name: "Kanto with friends".into(),
            game_name: "Pokémon FireRed (USA) 1.0".into(),
            game_sha1: FIRERED.into(),
        },
        "Red",
        APP_VERSION,
    );
    let host = HostNode::start(pactlocke_net::new_key(), lobby, Relays::N0);
    let view = wait_for(
        "the invite",
        || host.view(),
        |v| v.invite_link.is_some() || v.error.is_some(),
    );
    let link = view.invite_link.expect("an invite");
    let invite = Invite::parse(&link).unwrap();
    assert!(invite.relay.is_some(), "the Host's relay is in the invite: {link}");

    let blue = JoinLobby::new(invite, "Blue", APP_VERSION, Some(FIRERED.into())).unwrap();
    let blue = JoinNode::start(pactlocke_net::new_key(), blue, Relays::N0);
    let joined = wait_for(
        "Blue in the lobby",
        || blue.view(),
        |v| v.players.len() == 2 || v.error.is_some(),
    );
    assert_eq!(joined.error, None);
    assert_eq!(joined.players[1].name, "Blue");
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
