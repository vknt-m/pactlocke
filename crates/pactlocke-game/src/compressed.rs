//! CFRU's compressed PC slot, its 58-byte `struct CompressedPokemon`: the
//! party form's first 28 bytes, then parts of its four substructures, with
//! no checksum, PP, contest stats or ribbons. The game rebuilds the rest
//! when it takes a Pokémon out.

use crate::profile::COMPRESSED_SLOT_SIZE;

/// Personality to markings, as in the party form.
const HEADER: usize = 28;
/// Where the compressed slot keeps the species (the Growth substructure's first field).
pub(crate) const SPECIES: usize = HEADER;
/// The Growth substructure's first 11 bytes: species to Poké Ball.
const GROWTH: usize = 11;
/// Four 10-bit moves in 5 bytes.
const MOVES: usize = HEADER + GROWTH;
const EVS: usize = MOVES + 5;
/// Pokérus, met location, origin and IVs: the Misc substructure's first 8 bytes.
const MISC: usize = EVS + 6;
/// Where the compressed slot keeps the met location.
pub(crate) const MET_LOCATION: usize = MISC + 1;

/// One substructure's size in the party form.
const SUBSTRUCTURE_SIZE: usize = 12;

/// The compressed slot for an unencrypted party Pokémon whose substructures
/// (Growth, Attacks, EVs, Misc, in that order) start at `substructures`.
pub(crate) fn compress(party: &[u8], substructures: usize) -> Vec<u8> {
    let sub = |index: usize| &party[substructures + index * SUBSTRUCTURE_SIZE..][..SUBSTRUCTURE_SIZE];
    let mut slot = Vec::with_capacity(COMPRESSED_SLOT_SIZE as usize);
    slot.extend_from_slice(&party[..HEADER]);
    slot.extend_from_slice(&sub(0)[..GROWTH]);
    let attacks = sub(1);
    let moves = (0..4).fold(0u64, |packed, i| {
        let id = u16::from_le_bytes([attacks[i * 2], attacks[i * 2 + 1]]) & 0x3FF;
        packed | u64::from(id) << (10 * i)
    });
    slot.extend_from_slice(&moves.to_le_bytes()[..5]);
    slot.extend_from_slice(&sub(2)[..6]);
    slot.extend_from_slice(&sub(3)[..8]);
    slot
}
