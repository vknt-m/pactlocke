//! A randomizer recipe and the UPR FVX settings string it makes.
//!
//! The string is FVX's own format (`Settings.java`): the settings version,
//! then base64 of 69 option bytes, the name length and name, a big-endian
//! CRC32 of everything before it, and four zero bytes. A recipe starts from
//! the base below, tested on all five original games, and sets only the
//! option bytes it has a switch for.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// FVX 1.6.3's settings version: the string's first three digits.
const SETTINGS_VERSION: &str = "429";
const OPTION_BYTES: usize = 69;
/// The name a recipe's string carries; FVX shows it, nothing more.
const NAME: &[u8] = b"Pactlocke";

/// The tested base: UPR's own CLI test settings, which the rule checks
/// passed on (46 of 46 on every original game).
const BASE: &str = "429ACOBAQIEAAcAAQAEAALkIAARAAIUAAAUAEAEAAEADwAAAAAAAADkBOQBAAgJ5AkCAOQAAgABAAEBAAAAAAAJAAABKAEADlBhY3Rsb2NrZSB0ZXN0cZZULQAAAAA=";

/// What the Host randomizes. Each switch picks between the game's own and
/// one tested FVX choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipe {
    /// The starters are any Pokémon.
    pub starters: bool,
    /// Wild Pokémon are swapped one-to-one within each named location.
    pub wild: bool,
    /// Trainers' Pokémon are random.
    pub trainers: bool,
    /// Statics become random Pokémon of matching strength.
    pub statics: bool,
    /// Every species gets random types.
    pub types: bool,
    /// Every species gets random base stats.
    pub base_stats: bool,
}

impl Default for Recipe {
    /// Starters, wild Pokémon, trainers and Statics; types and stats as the game has them.
    fn default() -> Self {
        Self {
            starters: true,
            wild: true,
            trainers: true,
            statics: true,
            types: false,
            base_stats: false,
        }
    }
}

/// One option byte a switch sets: which bits it owns, and their value off and on.
struct Switch {
    byte: usize,
    mask: u8,
    off: u8,
    on: u8,
}

// Bit positions follow FVX's `makeByteSelected` order in `Settings.toString`.
const BASE_STATS: Switch = Switch {
    byte: 1,
    mask: 0x0E,
    off: 0x08,
    on: 0x02,
};
const TYPES: Switch = Switch {
    byte: 2,
    mask: 0x07,
    off: 0x04,
    on: 0x01,
};
const STARTERS: Switch = Switch {
    byte: 4,
    mask: 0x8F,
    off: 0x04,
    on: 0x02,
};
const TRAINERS: Switch = Switch {
    byte: 13,
    mask: 0xFF,
    off: 0x01,
    on: 0x02,
};
/// Bit 0 is "don't randomize"; bit 5 is per named location either way.
const WILD: Switch = Switch {
    byte: 15,
    mask: 0x6F,
    off: 0x21,
    on: 0x20,
};
const STATICS: Switch = Switch {
    byte: 19,
    mask: 0x0F,
    off: 0x01,
    on: 0x02,
};

impl Recipe {
    /// Every recipe there is, each switch on or off.
    pub fn all() -> impl Iterator<Item = Recipe> {
        (0u8..64).map(|bits| Recipe {
            starters: bits & 1 != 0,
            wild: bits & 2 != 0,
            trainers: bits & 4 != 0,
            statics: bits & 8 != 0,
            types: bits & 16 != 0,
            base_stats: bits & 32 != 0,
        })
    }

    /// The recipe a settings string was made from, if a recipe made it.
    pub fn of(settings: &str) -> Option<Recipe> {
        Recipe::all().find(|recipe| same_options(&recipe.settings_string(), settings))
    }

    /// What it randomizes, for setup to show: "starters, wild Pokémon and trainers".
    pub fn describe(&self) -> String {
        let parts: Vec<&str> = [
            (self.starters, "starters"),
            (self.wild, "wild Pokémon"),
            (self.trainers, "trainers"),
            (self.statics, "Statics"),
            (self.types, "types"),
            (self.base_stats, "base stats"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        match parts.as_slice() {
            [] => "nothing".to_owned(),
            [one] => (*one).to_owned(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    }

    /// The FVX settings string for this recipe.
    pub fn settings_string(&self) -> String {
        let mut options = decode(BASE).expect("the base settings string is valid").options;
        for (switch, on) in [
            (STARTERS, self.starters),
            (WILD, self.wild),
            (TRAINERS, self.trainers),
            (STATICS, self.statics),
            (TYPES, self.types),
            (BASE_STATS, self.base_stats),
        ] {
            let value = if on { switch.on } else { switch.off };
            options[switch.byte] = (options[switch.byte] & !switch.mask) | value;
        }
        encode(&options, NAME)
    }
}

/// Why a settings string isn't one FVX 1.6.3 takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// Made by another FVX version, which 1.6.3 would re-read with a warning.
    Version(String),
    /// Not base64 of the right shape, or its checksum doesn't match.
    Damaged,
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Version(v) => write!(
                f,
                "these settings are from another randomizer version ({v}); make them in FVX {} instead",
                crate::FVX_VERSION
            ),
            SettingsError::Damaged => f.write_str("these settings are cut short or changed; copy them from FVX again"),
        }
    }
}

/// Checks a settings string a Player pasted from FVX's own window.
pub fn check_settings_string(settings: &str) -> Result<(), SettingsError> {
    decode(settings).map(|_| ())
}

/// Whether two settings strings set the same options, whatever name each carries.
pub fn same_options(a: &str, b: &str) -> bool {
    matches!((decode(a), decode(b)), (Ok(a), Ok(b)) if a.options == b.options)
}

struct Decoded {
    options: [u8; OPTION_BYTES],
}

fn decode(settings: &str) -> Result<Decoded, SettingsError> {
    let settings = settings.trim();
    let (version, body) = settings.split_at_checked(3).ok_or(SettingsError::Damaged)?;
    if version != SETTINGS_VERSION {
        return Err(SettingsError::Version(version.to_owned()));
    }
    let bytes = STANDARD.decode(body).map_err(|_| SettingsError::Damaged)?;
    let name_len = *bytes.get(OPTION_BYTES).ok_or(SettingsError::Damaged)? as usize;
    let signed = OPTION_BYTES + 1 + name_len;
    if bytes.len() != signed + 8 {
        return Err(SettingsError::Damaged);
    }
    let crc = u32::from_be_bytes(bytes[signed..signed + 4].try_into().expect("four bytes"));
    if crc != crc32fast::hash(&bytes[..signed]) {
        return Err(SettingsError::Damaged);
    }
    Ok(Decoded {
        options: bytes[..OPTION_BYTES].try_into().expect("69 option bytes"),
    })
}

fn encode(options: &[u8; OPTION_BYTES], name: &[u8]) -> String {
    let mut bytes = options.to_vec();
    bytes.push(u8::try_from(name.len()).expect("a short name"));
    bytes.extend_from_slice(name);
    let crc = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&crc.to_be_bytes());
    bytes.extend_from_slice(&[0; 4]);
    format!("{SETTINGS_VERSION}{}", STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(recipe: Recipe) -> [u8; OPTION_BYTES] {
        decode(&recipe.settings_string())
            .expect("a recipe makes a valid string")
            .options
    }

    #[test]
    fn everything_on_keeps_the_tested_options() {
        let all = Recipe {
            types: true,
            base_stats: true,
            ..Recipe::default()
        };
        assert!(same_options(&all.settings_string(), BASE));
    }

    #[test]
    fn each_switch_sets_only_its_own_bits() {
        let on = options(Recipe {
            types: true,
            base_stats: true,
            ..Recipe::default()
        });
        let off = options(Recipe {
            starters: false,
            wild: false,
            trainers: false,
            statics: false,
            types: false,
            base_stats: false,
        });
        for (byte, (a, b)) in on.iter().zip(off).enumerate() {
            let owned = [&STARTERS, &WILD, &TRAINERS, &STATICS, &TYPES, &BASE_STATS]
                .iter()
                .find(|s| s.byte == byte)
                .map_or(0, |s| s.mask);
            assert_eq!(a & !owned, b & !owned, "byte {byte} changed outside its switch");
        }
        assert_eq!(off[STARTERS.byte] & STARTERS.mask, STARTERS.off);
        assert_eq!(off[WILD.byte] & 0x01, 0x01, "wild Pokémon left alone");
        assert_eq!(off[TRAINERS.byte], TRAINERS.off);
    }

    #[test]
    fn a_pasted_string_from_another_version_or_damaged_is_refused() {
        let good = Recipe::default().settings_string();
        assert_eq!(check_settings_string(&good), Ok(()));
        assert_eq!(
            check_settings_string(&format!("428{}", &good[3..])),
            Err(SettingsError::Version("428".into()))
        );
        let mut damaged = good.clone();
        damaged.replace_range(10..11, if &good[10..11] == "A" { "B" } else { "A" });
        assert_eq!(check_settings_string(&damaged), Err(SettingsError::Damaged));
        assert_eq!(check_settings_string("42"), Err(SettingsError::Damaged));
    }

    #[test]
    fn a_recipe_is_read_back_from_its_string() {
        for recipe in Recipe::all() {
            assert_eq!(Recipe::of(&recipe.settings_string()), Some(recipe));
        }
        assert_eq!(
            Recipe::default().describe(),
            "starters, wild Pokémon, trainers and Statics"
        );
    }

    #[test]
    fn the_name_doesnt_change_the_options() {
        let options = decode(BASE).unwrap().options;
        assert!(same_options(&encode(&options, b"Other name"), BASE));
        assert!(!same_options(&Recipe::default().settings_string(), BASE));
    }
}
