//! Seam 1: Partner Avatars and Followers, read from each partner's
//! Shadow Game and drawn into hardware OAM, tiles and palettes only, through
//! the real running game on the fake game.

mod support;

use pactlocke_game::{AvatarRoom, SpriteImage, Whereabouts};
use pactlocke_run::{Ruleset, ShadowState};
use support::streams::{Table, key};

const PALLET: (u8, u8) = (3, 0);
const ROUTE_1: (u8, u8) = (3, 19);

fn room() -> AvatarRoom {
    AvatarRoom {
        anchor: (112, 56),
        first_oam: 64,
        first_tile: 944,
        palettes: vec![11, 12, 13, 14, 15],
    }
}

fn sprite(colour: u8) -> SpriteImage {
    SpriteImage {
        tiles: vec![colour; 256],
        palette: vec![colour; 32],
        hflip: false,
        priority: 2,
        square: false,
    }
}

/// Player `i` stands at this tile, on an original game with room for avatars.
fn stands(table: &mut Table, i: usize, map: (u8, u8), x: i16, y: i16) {
    table.happens(i, move |g| {
        g.avatar_room = Some(room());
        g.whereabouts = Some(Whereabouts { map, x, y });
        g.player_sprite = Some(sprite(0x10 + i as u8));
        g.party = vec![support::party_pokemon(support::PIDGEY + i as u16, 0xA0 + i as u32, "")];
    });
}

fn both_play(table: &mut Table, frames: usize) {
    for _ in 0..frames / 10 {
        table.plays(0, 10);
        table.plays(1, 10);
    }
}

/// OAM entry `n` (from the room's first) in Player `i`'s overlay: attr0, attr1, attr2.
fn entry(table: &mut Table, i: usize, n: usize) -> (u16, u16, u16) {
    let overlay = table.game(i).core.overlay.clone();
    let (_, oam) = overlay
        .iter()
        .find(|(addr, _)| *addr == 0x0700_0200)
        .expect("OAM from entry 64");
    let word = |w: usize| u16::from_le_bytes([oam[n * 8 + w * 2], oam[n * 8 + w * 2 + 1]]);
    (word(0), word(1), word(2))
}

/// Where a drawn entry is on screen; `None` when hidden.
fn screen(attrs: (u16, u16, u16)) -> Option<(u16, u16)> {
    (attrs.0 & 0x0200 == 0).then_some((attrs.1 & 0x1FF, attrs.0 & 0xFF))
}

#[test]
fn a_partner_on_the_same_map_is_drawn_at_their_tile_from_their_shadow_game() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.draw_partner_avatars();
    stands(&mut table, 0, PALLET, 10, 10);
    stands(&mut table, 1, PALLET, 12, 9);
    both_play(&mut table, 300);
    // Blue, 2 tiles right and 1 up of Red: slot 0 is the first Partner Avatar.
    assert_eq!(screen(entry(&mut table, 0, 0)), Some((112 + 32, 56 - 16)));
    let overlay = table.game(0).core.overlay.clone();
    assert!(
        overlay.iter().any(|(_, bytes)| bytes.starts_with(&[0x11; 256])),
        "Blue's own look, from Blue's game"
    );
    // And Red, in Blue's game.
    assert_eq!(screen(entry(&mut table, 1, 0)), Some((112 - 32, 56 + 16)));
}

#[test]
fn only_hardware_oam_obj_tiles_and_obj_palettes_are_written_and_every_shadow_still_matches() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.draw_partner_avatars();
    stands(&mut table, 0, PALLET, 10, 10);
    stands(&mut table, 1, PALLET, 11, 10);
    both_play(&mut table, 600);
    for i in 0..2 {
        let overlay = table.game(i).core.overlay.clone();
        assert!(!overlay.is_empty());
        for (addr, bytes) in overlay {
            let end = addr + bytes.len() as u32;
            let oam = addr >= 0x0700_0200 && end <= 0x0700_0400;
            let tiles = addr >= 0x0601_0000 + 944 * 32 && end <= 0x0601_8000;
            let palettes = addr >= 0x0500_0200 + 11 * 32 && end <= 0x0500_0400;
            assert!(oam || tiles || palettes, "{addr:#010X}+{}", bytes.len());
        }
    }
    table.waits(1000);
    for (watcher, watched) in [(0, 1), (1, 0)] {
        let shadow = table.apps[watcher].shadow_of(key(watched));
        assert_eq!(shadow.state(), ShadowState::Matched, "{watcher} watching {watched}");
        assert!(shadow.hashes_matched() >= 4);
    }
}

#[test]
fn a_partner_on_another_map_isnt_drawn() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.draw_partner_avatars();
    stands(&mut table, 0, PALLET, 10, 10);
    stands(&mut table, 1, ROUTE_1, 10, 10);
    both_play(&mut table, 300);
    assert_eq!(screen(entry(&mut table, 0, 0)), None);
}

#[test]
fn a_partner_walks_from_tile_to_tile_on_wall_clock_time() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.draw_partner_avatars();
    stands(&mut table, 0, PALLET, 10, 10);
    stands(&mut table, 1, PALLET, 12, 10);
    both_play(&mut table, 300);
    assert_eq!(screen(entry(&mut table, 0, 0)).map(|s| s.0), Some(144));

    // Blue steps one tile right: it reaches Red's game a little later and
    // is walked over about 16 frames' time, not jumped.
    table.happens(1, |g| g.whereabouts.as_mut().unwrap().x = 13);
    let mut seen = Vec::new();
    for _ in 0..80 {
        table.plays(1, 1);
        table.plays(0, 1);
        if let Some((x, _)) = screen(entry(&mut table, 0, 0)) {
            seen.push(x);
        }
    }
    assert_eq!(seen.last(), Some(&160));
    assert!(
        seen.iter().any(|x| *x > 144 && *x < 160),
        "in between on the way: {seen:?}"
    );
}

/// Followers are off until they can walk like HGSS's, from a Player's own
/// HeartGold or SoulSilver ROM: a party icon sliding behind looked wrong.
/// Partner Avatars still walk; no Follower is drawn behind anyone.
#[test]
fn followers_are_off_and_partner_avatars_still_walk() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.draw_partner_avatars();
    stands(&mut table, 0, PALLET, 10, 10);
    stands(&mut table, 1, PALLET, 14, 10);
    both_play(&mut table, 300);
    table.happens(0, |g| g.whereabouts.as_mut().unwrap().x = 11);
    table.happens(1, |g| g.whereabouts.as_mut().unwrap().x = 15);
    both_play(&mut table, 300);
    // Blue's Partner Avatar, on (15, 10): 4 tiles right of Red.
    assert_eq!(screen(entry(&mut table, 0, 0)), Some((112 + 64, 56)));
    // No Follower: not Blue's (slot 1), nor Red's own (the last slot).
    assert_eq!(screen(entry(&mut table, 0, 1)), None);
    assert_eq!(screen(entry(&mut table, 0, 4)), None);
}

#[test]
fn on_a_game_without_room_for_avatars_none_are_drawn() {
    let mut table = Table::start(&["Red", "Blue"], Ruleset::default());
    table.draw_partner_avatars();
    stands(&mut table, 0, PALLET, 10, 10);
    stands(&mut table, 1, PALLET, 11, 10);
    table.happens(0, |g| g.avatar_room = None);
    both_play(&mut table, 300);
    assert!(table.game(0).core.overlay.is_empty(), "as on a Romhack");
}
