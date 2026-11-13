//! The lobby over the in-memory transport: friends join by invite with no
//! account, strangers are refused before anything they send is read, each
//! invite admits one Player, and a misbehaving app is dropped, not obeyed.

use pactlocke_net::memory::MemoryNet;
use pactlocke_net::{
    Admission, HostLobby, JoinLobby, JoinState, LobbyPlayer, Message, PeerKey, Refusal, RomStatus, RunAdvert, encode,
};

const VERSION: &str = "0.1.0";
const FIRERED: &str = "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc";
const LEAFGREEN: &str = "574fa542ffebb14be69902d1d36f1ec0a4afd71e";
const HOST: PeerKey = PeerKey([1; 32]);
const BLUE: PeerKey = PeerKey([2; 32]);
const GREEN: PeerKey = PeerKey([3; 32]);
const STRANGER: PeerKey = PeerKey([9; 32]);

fn host_lobby() -> HostLobby {
    HostLobby::new(
        RunAdvert {
            name: "Kanto with friends".into(),
            game_name: "Pokémon FireRed (USA) 1.0".into(),
            game_sha1: FIRERED.into(),
        },
        "Red",
        VERSION,
    )
}

fn joiner(net: &mut MemoryNet, name: &str, rom: Option<&str>) -> JoinLobby {
    let invite = net.host.invite(HOST, None);
    let link = invite.link();
    let invite = pactlocke_net::Invite::parse(&link).unwrap();
    JoinLobby::new(invite, name, VERSION, rom.map(str::to_owned)).unwrap()
}

fn player(name: &str, host: bool, rom: RomStatus) -> LobbyPlayer {
    LobbyPlayer {
        name: name.into(),
        host,
        rom,
    }
}

#[test]
fn friends_join_by_invite_and_everyone_sees_who_brought_the_right_rom() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    assert_eq!(net.connect(BLUE, blue), Admission::Admit);
    let green = joiner(&mut net, "Green", Some(LEAFGREEN));
    assert_eq!(net.connect(GREEN, green), Admission::Admit);

    let everyone = [
        player("Red", true, RomStatus::Matches),
        player("Blue", false, RomStatus::Matches),
        player("Green", false, RomStatus::Wrong),
    ];
    assert_eq!(net.host.players(), everyone);
    for peer in [BLUE, GREEN] {
        let JoinState::InLobby { run, players } = net.joiner(peer).state() else {
            panic!("{:?}", net.joiner(peer).state());
        };
        assert_eq!(run.game_sha1, FIRERED);
        assert_eq!(players, &everyone, "the same lobby for everyone");
    }
    assert_eq!(net.joiner(BLUE).rom_matches(), Some(true));
    assert_eq!(net.joiner(GREEN).rom_matches(), Some(false));
}

#[test]
fn a_player_without_a_rom_yet_is_shown_so_until_they_pick_one() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", None);
    net.connect(BLUE, blue);
    assert_eq!(net.host.players()[1].rom, RomStatus::NotYet);

    let message = net.joiner_mut(BLUE).rom_picked(FIRERED).expect("in the lobby");
    net.send_raw(BLUE, message);
    assert_eq!(net.host.players()[1].rom, RomStatus::Matches);
}

#[test]
fn a_stranger_without_an_invite_is_refused_before_anything_is_read() {
    let mut net = MemoryNet::new(host_lobby());
    let stranger = joiner(&mut net, "Mallory", Some(FIRERED));
    assert_eq!(
        net.connect_without_invite(STRANGER, stranger),
        Admission::Refuse(Refusal::NotInvited)
    );
    // Whatever it sends now is never read.
    net.send_raw(
        STRANGER,
        encode(&Message::Hello {
            app_version: VERSION.into(),
            name: "Mallory".into(),
        }),
    );
    assert_eq!(net.host.players().len(), 1, "only the Host");
}

#[test]
fn an_invite_admits_one_player_once() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    let copied = blue.clone();
    assert_eq!(net.connect(BLUE, blue), Admission::Admit);
    assert_eq!(
        net.connect(STRANGER, copied),
        Admission::Refuse(Refusal::NotInvited),
        "the same link, a second time"
    );
}

#[test]
fn a_player_in_the_roster_can_reconnect() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    let again = blue.clone();
    net.connect(BLUE, blue);
    net.disconnect(BLUE);
    assert_eq!(net.host.players().len(), 1, "away");

    assert_eq!(net.connect(BLUE, again), Admission::Admit, "their key is in the roster");
    assert_eq!(net.host.players().len(), 2);
}

#[test]
fn a_run_has_three_players_at_most() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);
    let green = joiner(&mut net, "Green", Some(FIRERED));
    net.connect(GREEN, green);
    let fourth = joiner(&mut net, "Yellow", Some(FIRERED));
    assert_eq!(net.connect(STRANGER, fourth), Admission::Refuse(Refusal::RunFull));
}

#[test]
fn an_invite_from_another_version_is_refused_naming_the_version_to_update_to() {
    let mut net = MemoryNet::new(host_lobby());
    let invite = net.host.invite(HOST, None);
    let error = JoinLobby::new(invite, "Blue", "0.0.9", None).unwrap_err();
    assert_eq!(
        error,
        "This invite is from Pactlocke 0.1.0, and you have 0.0.9: you need 0.1.0 to join."
    );
}

#[test]
fn a_hello_from_another_version_is_refused_by_the_host_too() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    let proof = blue.proof();
    assert_eq!(net.host.admit(BLUE, Some(&proof)), Admission::Admit);
    let actions = net.host.receive(
        BLUE,
        &encode(&Message::Hello {
            app_version: "9.9.9".into(),
            name: "Blue".into(),
        }),
        0,
    );
    assert!(
        matches!(&actions[..], [pactlocke_net::HostAction::Close { refusal: Refusal::Version { host_version }, .. }] if host_version == VERSION),
        "{actions:?}"
    );
}

#[test]
fn malformed_messages_are_dropped_and_a_flood_closes_the_connection() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);

    for junk in [vec![], vec![0xFF; 10], b"PK\x01\x00\x63\x00\x00".to_vec()] {
        net.send_raw(BLUE, junk);
    }
    assert_eq!(net.closed(BLUE), None, "a few are just dropped");
    assert_eq!(net.host.players().len(), 2, "and the lobby goes on");

    for _ in 0..100 {
        net.send_raw(BLUE, vec![0xFF; 10]);
    }
    assert_eq!(net.closed(BLUE), Some(&Refusal::Misbehaving));
    assert!(matches!(net.joiner(BLUE).state(), JoinState::Refused(why) if why.contains("couldn't read")));
    assert_eq!(net.host.players().len(), 1);
}

#[test]
fn too_many_messages_too_fast_are_dropped_and_a_steady_pace_is_fine() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);
    let check = encode(&Message::RomCheck { sha1: FIRERED.into() });

    // Twenty a second, for a minute: always within the rate.
    for _ in 0..1200 {
        net.advance_ms(50);
        net.send_raw(BLUE, check.clone());
    }
    assert_eq!(net.closed(BLUE), None);

    // A burst far past it.
    for _ in 0..200 {
        net.send_raw(BLUE, check.clone());
    }
    assert_eq!(net.closed(BLUE), Some(&Refusal::Misbehaving));
}

#[test]
fn a_player_closed_for_misbehaving_gives_up_their_place() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);
    let green = joiner(&mut net, "Green", Some(FIRERED));
    net.connect(GREEN, green);
    for _ in 0..100 {
        net.send_raw(BLUE, vec![0xFF; 10]);
    }
    assert_eq!(net.closed(BLUE), Some(&Refusal::Misbehaving));

    let yellow = joiner(&mut net, "Yellow", Some(FIRERED));
    assert_eq!(net.connect(STRANGER, yellow), Admission::Admit, "Blue's place is free");
}

#[test]
fn a_stranger_cant_tell_a_full_run_from_a_wrong_invite() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);
    let green = joiner(&mut net, "Green", Some(FIRERED));
    net.connect(GREEN, green);
    let stranger = joiner(&mut net, "Mallory", Some(FIRERED));
    assert_eq!(
        net.connect_without_invite(STRANGER, stranger),
        Admission::Refuse(Refusal::NotInvited),
        "not 'full'"
    );
}

#[test]
fn names_are_shown_without_control_or_direction_characters() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "  Bl\u{202E}ue\u{0007}  ", Some(FIRERED));
    net.connect(BLUE, blue);
    assert_eq!(net.host.players()[1].name, "Blue");
}

#[test]
fn setup_and_log_messages_cross_the_lobby_unread() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);

    let bytes = net.joiner(BLUE).send(&Message::Setup(b"ready".to_vec()));
    net.send_raw(BLUE, bytes);
    assert_eq!(net.host_inbox(), vec![(BLUE, Message::Setup(b"ready".to_vec()))]);

    net.host_sends(None, &Message::Log(b"entry".to_vec()));
    assert_eq!(net.joiner_mut(BLUE).take_inbox(), vec![Message::Log(b"entry".to_vec())]);
}

#[test]
fn input_streams_cross_the_lobby_unread_both_ways() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);

    let bytes = net.joiner(BLUE).send(&Message::Input(b"keys".to_vec()));
    net.send_raw(BLUE, bytes);
    assert_eq!(net.host_inbox(), vec![(BLUE, Message::Input(b"keys".to_vec()))]);

    net.host_sends(Some(BLUE), &Message::Input(b"red's keys".to_vec()));
    assert_eq!(
        net.joiner_mut(BLUE).take_inbox(),
        vec![Message::Input(b"red's keys".to_vec())]
    );
}

#[test]
fn a_player_at_4x_streams_input_without_starving_setup_and_log() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    net.connect(BLUE, blue);
    let input = encode(&Message::Input(vec![0; 64]));
    let log = encode(&Message::Log(b"entry".to_vec()));

    // Sixteen input messages a second (4x at one per 15 frames) and a log
    // entry every 100 ms, for a minute, plus a resend burst now and then.
    for tick in 0..600 {
        net.advance_ms(100);
        for _ in 0..2 {
            net.send_raw(BLUE, input.clone());
        }
        net.send_raw(BLUE, log.clone());
        if tick % 100 == 0 {
            for _ in 0..40 {
                net.send_raw(BLUE, input.clone());
            }
        }
    }
    assert_eq!(net.closed(BLUE), None);
    let inbox = net.host_inbox();
    assert_eq!(
        inbox.iter().filter(|(_, m)| matches!(m, Message::Log(_))).count(),
        600,
        "every log entry got through"
    );
}

#[test]
fn after_the_lock_nobody_new_joins_but_the_roster_reconnects() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    let again = blue.clone();
    let green = joiner(&mut net, "Green", Some(FIRERED));
    net.connect(BLUE, blue);
    net.host.lock();

    assert_eq!(net.connect(GREEN, green), Admission::Refuse(Refusal::RunStarted));
    net.disconnect(BLUE);
    assert_eq!(net.connect(BLUE, again), Admission::Admit, "Blue is in the roster");
}

/// Once the Run is locked a roster member's place is theirs for the Run: a
/// connection closed for a flood ends, but they can connect again, and the
/// count of dropped messages starts again on the new connection.
#[test]
fn after_the_lock_a_player_closed_for_misbehaving_keeps_their_place_in_the_roster() {
    let mut net = MemoryNet::new(host_lobby());
    let blue = joiner(&mut net, "Blue", Some(FIRERED));
    let again = blue.clone();
    net.connect(BLUE, blue);
    net.host.lock();
    for _ in 0..100 {
        net.send_raw(BLUE, vec![0xFF; 10]);
    }
    assert_eq!(net.closed(BLUE), Some(&Refusal::Misbehaving));

    assert_eq!(
        net.connect(BLUE, again),
        Admission::Admit,
        "Blue is still in the roster"
    );
    for _ in 0..10 {
        net.send_raw(BLUE, vec![0xFF; 10]);
    }
    assert_eq!(
        net.host.players().len(),
        2,
        "a few drops on the new connection don't close it"
    );
}
