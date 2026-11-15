//! Seeds come from the Run secret, so every app works out every Player's
//! seed by itself and no ROM or seed needs sending.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Whether every Player gets the same randomized game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeedMode {
    /// A different game for each Player.
    PerPlayer,
    /// Shared Seed: one game for everyone.
    Shared,
}

/// The FVX seed (`-z`, a Java long) for the Player with `player_key` in the
/// Run with `run_secret`.
pub fn seed_for(run_secret: &[u8; 16], mode: SeedMode, player_key: &[u8; 32]) -> i64 {
    let mut hash = Sha256::new();
    hash.update(b"pactlocke randomizer seed v1");
    hash.update(run_secret);
    match mode {
        SeedMode::PerPlayer => hash.update(player_key),
        SeedMode::Shared => hash.update(b"shared"),
    }
    i64::from_be_bytes(hash.finalize()[..8].try_into().expect("eight bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_player_seeds_differ_and_shared_seed_is_one() {
        let secret = [7; 16];
        let (a, b) = ([1; 32], [2; 32]);
        assert_ne!(
            seed_for(&secret, SeedMode::PerPlayer, &a),
            seed_for(&secret, SeedMode::PerPlayer, &b)
        );
        assert_eq!(
            seed_for(&secret, SeedMode::Shared, &a),
            seed_for(&secret, SeedMode::Shared, &b)
        );
        assert_ne!(
            seed_for(&secret, SeedMode::Shared, &a),
            seed_for(&[8; 16], SeedMode::Shared, &a)
        );
    }

    #[test]
    fn a_seed_is_the_same_on_every_machine() {
        // Pinned: a change here would give every Player a different game from an older build's.
        assert_eq!(
            seed_for(&[1; 16], SeedMode::PerPlayer, &[2; 32]),
            8_909_696_560_346_746_072
        );
    }
}
