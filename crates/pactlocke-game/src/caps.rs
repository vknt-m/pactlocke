//! Level caps: the stages a Run's cap rises through, their default levels
//! read from the loaded ROM's trainers, and the level-100 checks a cap patches.

use serde::{Deserialize, Serialize};

use crate::Badges;
use crate::profile::{GameProfile, ROM_BASE};

/// One stage's cap: the level Pokémon stop at until the stage is passed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelCap {
    /// Who the cap is for, such as "Brock".
    pub name: String,
    pub level: u8,
}

/// A cap stage as the Game Profile writes it: the next gym leader or Elite
/// Four member, and when the Player is past them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelCapDef {
    pub name: String,
    /// The trainers of this stage (several where the team depends on the
    /// starter); the default cap is the strongest Pokémon among them.
    pub trainers: Vec<u16>,
    /// Passed once the Player holds this many badges.
    #[serde(default)]
    pub badges: Option<u8>,
    /// Passed once this event flag is set (an Elite Four member's defeat).
    #[serde(default)]
    pub flag: Option<u16>,
    /// The default cap where the ROM's teams have no fixed levels (R.O.W.E.
    /// makes them as the Player goes): its trainers' strongest Pokémon, as
    /// measured in the game.
    #[serde(default)]
    pub level: Option<u8>,
}

impl LevelCapDef {
    pub(crate) fn passed(&self, badges: Badges, mut flag_set: impl FnMut(u16) -> bool) -> bool {
        match (self.badges, self.flag) {
            (Some(count), _) => badges.count() >= u32::from(count),
            (None, Some(flag)) => flag_set(flag),
            // The store refuses a stage with neither.
            (None, None) => false,
        }
    }
}

impl GameProfile {
    /// Each cap stage's default: its trainers' strongest Pokémon in `rom`,
    /// so a randomizer's trainer levels move the caps with them.
    pub fn default_level_caps(&self, rom: &[u8]) -> Vec<LevelCap> {
        self.level_caps
            .iter()
            .map(|stage| LevelCap {
                name: stage.name.clone(),
                level: stage.level.unwrap_or_else(|| {
                    stage
                        .trainers
                        .iter()
                        .filter_map(|&trainer| self.strongest_of(rom, trainer))
                        .max()
                        .unwrap_or(MAX_LEVEL)
                }),
            })
            .collect()
    }

    /// The highest level in a trainer's party, read from the trainer table.
    fn strongest_of(&self, rom: &[u8], trainer: u16) -> Option<u8> {
        let layout = &self.layout;
        let u32_at = |addr: u32| -> Option<u32> {
            let at = addr.checked_sub(ROM_BASE)? as usize;
            Some(u32::from_le_bytes(rom.get(at..at + 4)?.try_into().ok()?))
        };
        let entry = self.symbols.trainers + u32::from(trainer) * layout.trainer_size;
        let party_flags = *rom.get((entry + layout.trainer_party_flags - ROM_BASE) as usize)?;
        let size = u32_at(entry + layout.trainer_party_size)? & 0xFF;
        let party = u32_at(entry + layout.trainer_party)?;
        let mon_size = *layout.trainer_mon_sizes.get(usize::from(party_flags & 3))?;
        (0..size)
            .filter_map(|i| {
                // A byte: the low one of the original games' u16, or the whole
                // of pokeemerald-expansion's, which the ball follows.
                let at = (party + i * mon_size + layout.trainer_mon_level).checked_sub(ROM_BASE)? as usize;
                rom.get(at).copied().filter(|l| (1..=MAX_LEVEL).contains(l))
            })
            .max()
    }
}

/// The highest level a Pokémon can reach: a cap of this is no cap.
pub const MAX_LEVEL: u8 = 100;

/// How many instructions after the level read the compare may come.
const COMPARE_WITHIN: usize = 8;

/// Candidate level-100 checks: a `cmp rX, #100` that is the first compare
/// within a few Thumb instructions after `movs r1, #0x38` (reading
/// `MON_DATA_LEVEL`). Some candidates are outside the EXP and item-use code,
/// so a Game Profile lists the checked sites it patches.
pub fn level_cap_sites(rom: &[u8]) -> Vec<u32> {
    let halfword = |at: usize| rom.get(at..at + 2).map(|b| [b[0], b[1]]);
    (0..rom.len().saturating_sub(1))
        .step_by(2)
        .filter(|&at| halfword(at) == Some([0x38, 0x21]))
        .filter_map(|at| {
            (1..=COMPARE_WITHIN)
                .map(|k| at + 2 * k)
                .map_while(|next| halfword(next).map(|h| (next, h)))
                .find(|(_, [_, op])| (0x28..=0x2F).contains(op))
                .filter(|(_, [imm, _])| *imm == MAX_LEVEL)
                .map(|(next, _)| ROM_BASE + next as u32)
        })
        .collect()
}
