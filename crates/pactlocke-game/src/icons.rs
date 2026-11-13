//! Party icons read straight from a ROM, for the side panel: each species'
//! 32x32 icon (the first of its two frames) as RGBA, through the Game
//! Profile's icon tables. The same tables give the Followers their icons.

use crate::GameProfile;

/// The ROM is mapped here on the GBA's bus.
const ROM_BASE: u32 = 0x0800_0000;
/// A party icon's side, in pixels.
pub const ICON_SIZE: usize = 32;

/// A species' party icon as RGBA (32x32, transparent background), or `None`
/// where the profile has no icon tables or the ROM's entry points nowhere.
pub fn party_icon_rgba(profile: &GameProfile, rom: &[u8], species: u16) -> Option<Vec<u8>> {
    let avatars = profile.avatars.as_ref()?;
    let tiles_at = read_u32(rom, avatars.icon_table + u32::from(species) * 4)?;
    let index = *rom.get(offset(avatars.icon_palette_indices + u32::from(species))?)?;
    let palette_at = read_u32(rom, avatars.icon_palette_table + u32::from(index) * 8)?;
    let tiles = rom.get(offset(tiles_at)?..offset(tiles_at)? + 16 * 32)?;
    let palette = rom.get(offset(palette_at)?..offset(palette_at)? + 32)?;
    let colours: Vec<[u8; 4]> = palette
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let bgr = u16::from_le_bytes([c[0], c[1]]);
            let channel = |shift: u16| (((bgr >> shift) & 0x1F) * 255 / 31) as u8;
            // Colour 0 is transparent, as on the GBA.
            [channel(0), channel(5), channel(10), if i == 0 { 0 } else { 255 }]
        })
        .collect();
    let mut rgba = vec![0; ICON_SIZE * ICON_SIZE * 4];
    // 4x4 tiles of 8x8 pixels, row by row; 4 bits a pixel, low nibble first.
    for (t, tile) in tiles.as_chunks::<32>().0.iter().enumerate() {
        let (tx, ty) = (t % 4 * 8, t / 4 * 8);
        for (b, byte) in tile.iter().enumerate() {
            for (half, nibble) in [byte & 0x0F, byte >> 4].into_iter().enumerate() {
                let (x, y) = (tx + b % 4 * 2 + half, ty + b / 4);
                let at = (y * ICON_SIZE + x) * 4;
                rgba[at..at + 4].copy_from_slice(&colours[usize::from(nibble)]);
            }
        }
    }
    Some(rgba)
}

fn offset(addr: u32) -> Option<usize> {
    addr.checked_sub(ROM_BASE).map(|o| o as usize)
}

fn read_u32(rom: &[u8], addr: u32) -> Option<u32> {
    let at = offset(addr)?;
    Some(u32::from_le_bytes(rom.get(at..at + 4)?.try_into().ok()?))
}
