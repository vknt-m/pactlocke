//! The wire format and invite links: every message round-trips; malformed
//! bytes are errors, never a crash; an invite carries no IP address.

use pactlocke_net::{
    Invite, InviteError, LobbyPlayer, MAX_MESSAGE, Message, PROTOCOL, PeerKey, Refusal, RomStatus, RunAdvert,
    RunSecret, WireError, decode, encode,
};

fn run() -> RunAdvert {
    RunAdvert {
        name: "Kanto with friends".into(),
        game_name: "Pokémon FireRed (USA) 1.0".into(),
        game_sha1: "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc".into(),
    }
}

#[test]
fn every_message_round_trips() {
    let messages = [
        Message::Hello {
            app_version: "0.1.0".into(),
            name: "Ash ✨".into(),
        },
        Message::Welcome { run: run() },
        Message::Refused(Refusal::NotInvited),
        Message::Refused(Refusal::RunFull),
        Message::Refused(Refusal::Version {
            host_version: "0.2.0".into(),
        }),
        Message::Refused(Refusal::Misbehaving),
        Message::RomCheck { sha1: run().game_sha1 },
        Message::Lobby {
            players: vec![
                LobbyPlayer {
                    name: "Red".into(),
                    host: true,
                    rom: RomStatus::Matches,
                },
                LobbyPlayer {
                    name: "Blue".into(),
                    host: false,
                    rom: RomStatus::Wrong,
                },
                LobbyPlayer {
                    name: "Green".into(),
                    host: false,
                    rom: RomStatus::NotYet,
                },
            ],
        },
    ];
    for message in messages {
        assert_eq!(decode(&encode(&message)), Ok(message));
    }
}

#[test]
fn text_is_bounded() {
    let long = "x".repeat(500);
    let bytes = encode(&Message::Hello {
        app_version: "0.1.0".into(),
        name: long,
    });
    let Ok(Message::Hello { name, .. }) = decode(&bytes) else {
        panic!("decodes");
    };
    assert_eq!(name.len(), 64, "cut short");
    assert!(bytes.len() < MAX_MESSAGE);
}

#[test]
fn malformed_bytes_are_errors() {
    let good = encode(&Message::RomCheck { sha1: "ab".into() });
    assert_eq!(decode(&good[..3]), Err(WireError::TooShort));
    assert_eq!(decode(b"XX\x01\x00\x04\x00\x00"), Err(WireError::NotPactlocke));
    let mut other = good.clone();
    other[2] = 9;
    assert_eq!(
        decode(&other),
        Err(WireError::Protocol(9)),
        "another protocol: {PROTOCOL} only"
    );
    let mut kind = good.clone();
    kind[4] = 200;
    assert_eq!(decode(&kind), Err(WireError::UnknownKind(200)));
    let mut truncated = good.clone();
    truncated.pop();
    assert_eq!(decode(&truncated), Err(WireError::BadPayload));
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing), Err(WireError::BadPayload));
    assert_eq!(decode(&vec![0u8; MAX_MESSAGE + 1]), Err(WireError::TooLong));
    // A lobby claiming more Players than a Run has.
    let lobby = [b'P', b'K', 1, 0, 5, 1, 0, 9];
    assert_eq!(decode(&lobby), Err(WireError::BadPayload));
    // Text that isn't UTF-8.
    let hello = [b'P', b'K', 1, 0, 1, 4, 0, 1, 0xFF, 1, b'a'];
    assert_eq!(decode(&hello), Err(WireError::BadPayload));
}

#[test]
fn random_bytes_never_crash_the_decoder() {
    // A simple generator: the same bytes every run.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..20_000 {
        let len = (next() % 64) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if bytes.len() >= 4 && next() % 2 == 0 {
            // Often a valid header, so payload parsing is exercised too.
            bytes[..4].copy_from_slice(&[b'P', b'K', 1, 0]);
        }
        let _ = decode(&bytes);
    }
}

fn invite() -> Invite {
    Invite {
        host: PeerKey([7; 32]),
        relay: Some("https://use1-1.relay.n0.iroh.link./".into()),
        secret: RunSecret::generate(),
        app_version: "0.1.0".into(),
    }
}

#[test]
fn an_invite_link_round_trips_and_carries_no_ip_address() {
    let invite = invite();
    let link = invite.link();
    assert!(link.starts_with("pactlocke://join?"), "{link}");
    assert_eq!(Invite::parse(&link), Ok(invite.clone()));
    assert_eq!(
        Invite::parse(&format!("  {link}\n")),
        Ok(invite),
        "pasted with whitespace"
    );
    assert!(
        !link
            .split(['?', '&', '=', '/', ':'])
            .any(|part| part.parse::<std::net::Ipv4Addr>().is_ok()),
        "{link}"
    );
}

#[test]
fn each_invite_has_its_own_128_bit_secret() {
    let (a, b) = (RunSecret::generate(), RunSecret::generate());
    assert!(!a.matches(&b.0));
    assert!(a.matches(&a.0));
}

#[test]
fn a_damaged_link_is_refused() {
    let link = invite().link();
    assert_eq!(Invite::parse("https://example.com"), Err(InviteError::NotAnInvite));
    assert_eq!(
        Invite::parse(&link.replace("secret=", "secrex=")),
        Err(InviteError::Missing("Run secret"))
    );
    let short = link.replacen("host=0707", "host=07", 1);
    assert_eq!(Invite::parse(&short), Err(InviteError::Bad("Host key")));
    let bad = link.replacen("host=07", "host=zz", 1);
    assert_eq!(Invite::parse(&bad), Err(InviteError::Bad("Host key")));
}

#[test]
fn a_refusal_reads_the_same_in_another_protocol_version() {
    let mut bytes = encode(&Message::Refused(Refusal::Version {
        host_version: "0.2.0".into(),
    }));
    bytes[2] = 7; // a build on protocol 7 sent it
    assert_eq!(
        decode(&bytes),
        Ok(Message::Refused(Refusal::Version {
            host_version: "0.2.0".into()
        }))
    );
    assert_eq!(
        Refusal::Version {
            host_version: "0.2.0".into()
        }
        .to_string(),
        "the Host plays on Pactlocke 0.2.0, and you need 0.2.0 to join"
    );
}

#[test]
fn a_key_file_is_made_once_and_a_broken_one_is_left_alone() {
    let dir = std::env::temp_dir().join(format!("pactlocke-key-{}", std::process::id()));
    let path = dir.join("identity.key");
    let made = pactlocke_net::load_or_create_key(&path).unwrap();
    assert_eq!(
        pactlocke_net::load_or_create_key(&path).unwrap(),
        made,
        "the same identity next time"
    );

    std::fs::write(&path, b"not a key").unwrap();
    assert!(pactlocke_net::load_or_create_key(&path).is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"not a key",
        "not replaced by a new identity"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
