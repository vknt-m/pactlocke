//! UPR FVX, the randomizer for the five original games: downloaded from its
//! official GitHub release on first use and pinned by version and SHA-256,
//! run headless as a child process from a settings string Pactlocke builds,
//! with a seed derived from the Run secret. Its own success report isn't
//! trusted: the settings and seed it really used are read from its log.
//! Only clean original dumps are randomized; no ROM ever crosses the network.

mod install;
mod recipe;
mod run;
mod seed;

pub use install::{Asset, InstallError, Installed, asset_for_this_machine, download, install, installed};
pub use recipe::{Recipe, SettingsError, check_settings_string, same_options};
pub use run::{LogHeader, RandomizeError, Randomized, randomize, read_log};
pub use seed::{SeedMode, seed_for};

use sha1::{Digest, Sha1};

/// The pinned UPR FVX release.
pub const FVX_VERSION: &str = "1.6.3";

/// The SHA-1s of the clean dumps UPR FVX may randomize: FireRed, LeafGreen,
/// Emerald, Ruby and Sapphire (USA 1.0). UPR writes at fixed offsets, so it
/// would silently corrupt anything else.
pub const CLEAN_DUMPS: [(&str, &str); 5] = [
    ("FireRed", "41cb23d8dccc8ebd7c649cd8fbb58eeace6e2fdc"),
    ("LeafGreen", "574fa542ffebb14be69902d1d36f1ec0a4afd71e"),
    ("Emerald", "f3ae088181bf583e55daf962a92bb46f4f1d07b7"),
    ("Ruby", "f28b6ffc97847e94a6c21a63cacf633ee5c8df1e"),
    ("Sapphire", "3ccbbd45f8553c36463f13b938e833f652b793e4"),
];

/// The name of the original game `rom` is a clean dump of, if it is one.
pub fn clean_dump(rom: &[u8]) -> Option<&'static str> {
    let sha1 = sha1_hex(rom);
    CLEAN_DUMPS
        .iter()
        .find(|(_, clean)| *clean == sha1)
        .map(|(name, _)| *name)
}

/// Lowercase hex SHA-1, as Game Profiles and the lock write it.
pub fn sha1_hex(bytes: &[u8]) -> String {
    hex(&Sha1::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clean_dump_is_named() {
        assert_eq!(clean_dump(b"not a rom"), None);
    }

    #[test]
    fn clean_dumps_match_the_bundled_profiles() {
        for (name, sha1) in CLEAN_DUMPS {
            let file = format!(
                "{}/../pactlocke-game/profiles/{}-usa-1.0.toml",
                env!("CARGO_MANIFEST_DIR"),
                name.to_lowercase()
            );
            let profile = std::fs::read_to_string(&file).expect("the original game has a bundled profile");
            assert!(profile.contains(sha1), "{name}'s profile has another SHA-1");
        }
    }
}
