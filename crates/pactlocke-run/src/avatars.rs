//! Partner Avatars and Followers: every Player but this one, drawn walking
//! around in this Player's game, their lead Pokémon's party icon following
//! them, and this Player's own lead following them too. Where they stand and
//! how they look come from their Shadow Games (about half a second behind),
//! and they walk from tile to tile on wall-clock time, so a Player at 4x
//! still walks at a walk.
//!
//! Everything is drawn by display-only writes the core applies as each frame
//! starts to draw (see [`pactlocke_emu::CoreAdapter::overlay`]): hardware OAM
//! entries the game leaves free, OBJ tiles and OBJ palettes. Nothing reaches
//! work RAM, so the game never sees them and every Shadow Game still matches.

use std::collections::{BTreeMap, VecDeque};

use pactlocke_game::{AvatarRoom, Game, OAM, OBJ_PALETTES, OBJ_TILES, SpriteImage, lead_of};

use crate::SeenAvatar;
use crate::stream::PlayerKey;

/// A walking step: 16 pixels in 16 frames of a GBA's, in milliseconds.
const STEP_MS: f64 = 16.0 * 1000.0 / 59.7275;
/// A walking step, in frames: this Player's own camera, before a step is timed.
const STEP_FRAMES: u32 = 16;
/// Further behind than this many tiles, a Partner Avatar jumps to where it is.
const SNAP_TILES: usize = 8;
/// The most Partner Avatars drawn: a Run has three Players.
const MAX_AVATARS: usize = 2;
/// Every sprite slot takes this many 4bpp tiles: a 32x32 sprite's.
const SLOT_TILES: u16 = 16;
const SCREEN: (i32, i32) = (240, 160);

/// Another Player's Partner Avatar, as their Shadow Game shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartnerAvatar {
    pub player: PlayerKey,
    pub seen: SeenAvatar,
}

/// A tile on a map.
type Tile = (i16, i16);

/// One walker's drawn position, in map pixels, and the tiles still to walk.
#[derive(Debug, Clone)]
struct Walk {
    map: (u8, u8),
    drawn: (f64, f64),
    queue: VecDeque<Tile>,
    /// The tile the walker stands on, or the one their step started from.
    from: Tile,
    /// The tile walked from last: where a Follower stands.
    behind: Option<Tile>,
}

impl Walk {
    fn at(map: (u8, u8), tile: Tile) -> Self {
        Self {
            map,
            drawn: pixels(tile),
            queue: VecDeque::new(),
            from: tile,
            behind: None,
        }
    }

    /// Takes in where the walker is now: a new tile joins the queue.
    fn heard(&mut self, map: (u8, u8), tile: Tile) {
        if map != self.map {
            *self = Self::at(map, tile);
            return;
        }
        if tile != self.queue.back().copied().unwrap_or(self.from) {
            self.queue.push_back(tile);
        }
        if self.queue.len() > SNAP_TILES {
            self.behind = self.queue.iter().rev().nth(1).copied();
            self.drawn = pixels(tile);
            self.from = tile;
            self.queue.clear();
        }
    }

    /// Walks on for `pixels`, tile by tile, faster when behind.
    fn walk(&mut self, mut pixels_left: f64) {
        pixels_left *= self.queue.len().clamp(1, 4) as f64;
        while let Some(&next) = self.queue.front() {
            let goal = pixels(next);
            let (dx, dy) = (goal.0 - self.drawn.0, goal.1 - self.drawn.1);
            let distance = dx.abs() + dy.abs();
            if distance <= pixels_left {
                self.behind = Some(self.from);
                self.from = next;
                self.drawn = goal;
                self.queue.pop_front();
                pixels_left -= distance;
            } else {
                let along_x = pixels_left.min(dx.abs());
                self.drawn.0 += dx.signum() * along_x;
                self.drawn.1 += dy.signum() * (pixels_left - along_x).min(dy.abs());
                return;
            }
        }
    }
}

fn pixels((x, y): Tile) -> (f64, f64) {
    (f64::from(x) * 16.0, f64::from(y) * 16.0)
}

/// Draws the Partner Avatars and every Follower into one Player's game, frame by frame.
#[derive(Debug, Default)]
pub struct Painter {
    avatars: BTreeMap<PlayerKey, Walk>,
    /// This Player: their camera, which follows them a step at a time at the
    /// pace of their own last step (walking, running, cycling, any speed).
    me: Option<Walk>,
    my_step_frames: u32,
    frames_since_my_step: u32,
    last_ms: Option<i64>,
}

impl Painter {
    pub fn new() -> Self {
        Self::default()
    }

    /// The display writes for this frame: OAM entries, tiles and palettes.
    /// Nothing where the game has no room for avatars (a Romhack) or the
    /// Player isn't on the overworld.
    pub fn paint(&mut self, game: &mut impl Game, others: &[PartnerAvatar], now_ms: i64) -> Vec<(u32, Vec<u8>)> {
        let Some(room) = game.avatar_room() else {
            return Vec::new();
        };
        let Some(here) = game.whereabouts() else {
            return Vec::new();
        };
        let elapsed = self.last_ms.map_or(0, |last| (now_ms - last).clamp(0, 250));
        self.last_ms = Some(now_ms);

        // This Player's camera, a frame at a time, at their own step's pace.
        let me = self.me.get_or_insert_with(|| Walk::at(here.map, (here.x, here.y)));
        let before = me.queue.len();
        me.heard(here.map, (here.x, here.y));
        self.frames_since_my_step += 1;
        if me.queue.len() > before {
            self.my_step_frames = self.frames_since_my_step.clamp(2, STEP_FRAMES);
            self.frames_since_my_step = 0;
        }
        let pace = if self.my_step_frames == 0 {
            STEP_FRAMES
        } else {
            self.my_step_frames
        };
        me.walk(16.0 / f64::from(pace));
        let camera = me.drawn;
        let my_follower = me.behind;

        let step = elapsed as f64 * 16.0 / STEP_MS;
        self.avatars
            .retain(|player, _| others.iter().any(|o| o.player == *player));
        let mut sprites: Vec<Option<Drawn>> = Vec::new();
        for other in others.iter().take(MAX_AVATARS) {
            let at = &other.seen.at;
            let walk = self
                .avatars
                .entry(other.player)
                .or_insert_with(|| Walk::at(at.map, (at.x, at.y)));
            walk.heard(at.map, (at.x, at.y));
            walk.walk(step);
            if walk.map != here.map {
                sprites.extend([None, None]);
                continue;
            }
            sprites.push(standing(&room, camera, walk.drawn, other.seen.sprite.clone()));
            let icon = other.seen.lead.and_then(|species| game.party_icon(species));
            sprites.push(follower(&room, camera, walk.behind, icon));
        }
        sprites.resize_with(2 * MAX_AVATARS, || None);
        // This Player's own Follower last: drawn behind the Partner Avatars.
        let party = game.party();
        let icon = lead_of(&party).and_then(|lead| game.party_icon(lead.species));
        sprites.push(follower(&room, camera, my_follower, icon));
        write_out(&room, &sprites)
    }
}

/// A sprite to draw: its top-left on screen and its look.
struct Drawn {
    position: (i32, i32),
    image: SpriteImage,
}

/// Where a sprite of this look goes on screen when its walker stands at map
/// pixels `drawn`, if any of it is on screen. A 16x32 one's top-left sits
/// at the anchor offset; a 32x32 one (a bike, an icon) is centred over it.
fn standing(room: &AvatarRoom, camera: (f64, f64), drawn: (f64, f64), image: SpriteImage) -> Option<Drawn> {
    let (left, width) = if image.square { (8, 32) } else { (0, 16) };
    let x = i32::from(room.anchor.0) + (drawn.0 - camera.0).round() as i32 - left;
    let y = i32::from(room.anchor.1) + (drawn.1 - camera.1).round() as i32;
    let visible = x > -width && x < SCREEN.0 && y > -32 && y < SCREEN.1;
    visible.then_some(Drawn {
        position: (x, y),
        image,
    })
}

/// A Follower's party icon, a tile behind its walker, feet on that tile.
fn follower(room: &AvatarRoom, camera: (f64, f64), behind: Option<Tile>, icon: Option<SpriteImage>) -> Option<Drawn> {
    standing(room, camera, pixels(behind?), icon?)
}

/// The OAM entries, tiles and palettes for these sprites, in slot order:
/// entries from the room's first, each slot its own 16 tiles and palette.
/// A slot not drawn is a hidden entry.
fn write_out(room: &AvatarRoom, sprites: &[Option<Drawn>]) -> Vec<(u32, Vec<u8>)> {
    let mut writes = Vec::new();
    let mut oam = Vec::with_capacity(sprites.len() * 8);
    for (slot, sprite) in sprites.iter().enumerate() {
        let tile = room.first_tile + SLOT_TILES * slot as u16;
        let palette = room.palettes.get(slot).copied();
        let words = match (sprite, palette) {
            (Some(drawn), Some(palette)) => {
                let (x, y) = drawn.position;
                // Shape square (0) or tall (2); size 2: 32x32 or 16x32.
                let shape: u16 = if drawn.image.square { 0 } else { 2 };
                let attr0 = (y as u16 & 0xFF) | shape << 14;
                let attr1 = (x as u16 & 0x1FF) | u16::from(drawn.image.hflip) << 12 | 2 << 14;
                let attr2 = tile | u16::from(drawn.image.priority & 3) << 10 | u16::from(palette & 0xF) << 12;
                let mut tiles = drawn.image.tiles.clone();
                tiles.resize(usize::from(SLOT_TILES) * 32, 0);
                writes.push((OBJ_TILES + u32::from(tile) * 32, tiles));
                let mut colours = drawn.image.palette.clone();
                colours.resize(32, 0);
                writes.push((OBJ_PALETTES + u32::from(palette) * 32, colours));
                [attr0, attr1, attr2, 0]
            }
            // Hidden: the "disable" bit, off screen.
            _ => [0x0200 | 160, 0, 0, 0],
        };
        for word in words {
            oam.extend_from_slice(&word.to_le_bytes());
        }
    }
    writes.insert(0, (OAM + u32::from(room.first_oam) * 8, oam));
    writes
}
