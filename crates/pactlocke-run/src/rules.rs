//! The rules engine: pure logic for one Player. Given the locked Ruleset and
//! the Run's state, it decides what a moment in the game means. It reads
//! nothing and changes nothing itself.

use std::collections::{BTreeMap, BTreeSet};

use pactlocke_game::{Area, BattleKind, Source, SourceKind};

use crate::event::{DeclineReason, ResumedFrom};
use crate::families::EvolutionFamilies;
use crate::{Ruleset, RunEvent};

/// How a battle began.
pub(crate) struct BattleStart<'a> {
    pub kind: BattleKind,
    /// No script was running when it began: the Player met it in the wild.
    pub script_idle: bool,
    pub area: Option<&'a Area>,
    /// The wild Pokémon's species.
    pub foe: u16,
    /// The PID and species of every Pokémon in the Player's party and PC as it began.
    pub owned: &'a [(u32, u16)],
    /// For a battle a script started: the Static source it is, listed or not.
    pub static_key: Option<&'a str>,
}

/// Where a Gift came from.
pub(crate) enum GiftOrigin {
    /// A source the Game Profile lists.
    Listed(Source),
    /// A script gave it, but the Game Profile doesn't list the instruction.
    Unknown,
    Trade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GiftVerdict {
    Keep,
    Decline(DeclineReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Nothing for the rules here.
    Nothing,
    /// A wild battle before the Player has Poké Balls: it doesn't use the Encounter.
    NotCounted,
    /// The Area's one Encounter: catching is allowed.
    Encounter,
    /// The Area is closed: ball throws are refused for this battle.
    BlockCatches,
    /// The dupes clause: the Player has a living Pokémon of this species'
    /// family (`of`), so it isn't the Encounter, and can't be caught.
    Dupe { of: u16 },
    /// The Statics rule: a bonus Encounter of the Static's own; catching is allowed.
    StaticEncounter,
    /// A Static whose Encounter is used: ball throws are refused.
    StaticUsed,
}

pub(crate) struct Rules {
    ruleset: Ruleset,
    /// Areas whose Encounter is used.
    closed: BTreeSet<u16>,
    /// Whether the Player has had Poké Balls; until then no wild battle counts.
    had_poke_balls: bool,
    /// Dead PIDs, each with where it has got to since the game was booted.
    dead: BTreeMap<u32, DeadState>,
    /// The loaded game's evolution families, once the dupes clause needs them.
    families: Option<EvolutionFamilies>,
    /// Gift and Static sources whose one Encounter is used.
    used_sources: BTreeSet<String>,
    /// For each used Gift source, the Pokémon it gave and when.
    gift_from: BTreeMap<String, GiftGiven>,
    /// The number of events taken in so far, as a clock for when things happened.
    events_seen: u64,
    /// When the game was last reset to an older save (or booted from one).
    last_reset: Option<u64>,
    /// Declined Gifts not yet released.
    to_release: BTreeSet<u32>,
    /// The level cap stage last reached in this Run, and its cap.
    cap_stage: Option<usize>,
    level_cap: Option<u8>,
    /// Whether the cap is only a soft cap: the game's level checks can't be patched.
    soft_cap: bool,
    /// What the log has said about no items and set mode, so each is said
    /// once, and whether set mode has been forced before in this Run.
    items_banned_said: bool,
    item_ban_failed_said: bool,
    set_mode_forced_before: bool,
    set_mode_failed_said: bool,
    /// Must nickname: Pokémon waiting for the Player to name them, with their species names.
    nickname_wanted: BTreeMap<u32, String>,
}

/// The Pokémon a Gift source gave, and when (in events taken in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GiftGiven {
    pid: u32,
    at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeadState {
    /// Not moved out of the party since it died, or since the game was booted.
    NotMovedYet,
    MovedOut,
}

/// Why a Dead Pokémon is being moved to the Dead box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeadMove {
    /// It is in the party since it died, or since the game was booted.
    First,
    /// It was moved already and is back: a Violation.
    Returned,
}

impl Rules {
    pub fn new(ruleset: Ruleset) -> Self {
        Self {
            ruleset,
            closed: BTreeSet::new(),
            had_poke_balls: false,
            dead: BTreeMap::new(),
            families: None,
            used_sources: BTreeSet::new(),
            gift_from: BTreeMap::new(),
            events_seen: 0,
            last_reset: None,
            to_release: BTreeSet::new(),
            cap_stage: None,
            level_cap: None,
            soft_cap: false,
            items_banned_said: false,
            item_ban_failed_said: false,
            set_mode_forced_before: false,
            set_mode_failed_said: false,
            nickname_wanted: BTreeMap::new(),
        }
    }

    /// What a battle that just began means.
    pub fn battle_started(&self, start: &BattleStart) -> Verdict {
        if !self.ruleset.first_encounter || start.kind != BattleKind::Wild {
            return Verdict::Nothing;
        }
        if !start.script_idle {
            return self.static_started(start);
        }
        match start.area {
            None => Verdict::Nothing,
            Some(_) if !self.had_poke_balls => Verdict::NotCounted,
            Some(area) if self.closed.contains(&area.id) => Verdict::BlockCatches,
            Some(_) => match self.dupe_of(start.foe, start.owned) {
                Some(of) => Verdict::Dupe { of },
                None => Verdict::Encounter,
            },
        }
    }

    /// A Static: with the Statics rule a bonus Encounter of its own, without
    /// it only the Area's first Encounter (never a dupe).
    fn static_started(&self, start: &BattleStart) -> Verdict {
        if self.statics() {
            return match start.static_key {
                Some(key) if self.used_sources.contains(key) => Verdict::StaticUsed,
                _ => Verdict::StaticEncounter,
            };
        }
        match start.area {
            None => Verdict::Nothing,
            Some(_) if !self.had_poke_balls => Verdict::NotCounted,
            Some(area) if self.closed.contains(&area.id) => Verdict::BlockCatches,
            Some(_) => Verdict::Encounter,
        }
    }

    /// Whether the game was reset to an older save after the moment `at`.
    fn reset_since(&self, at: u64) -> bool {
        self.last_reset.is_some_and(|reset| reset > at)
    }

    fn statics(&self) -> bool {
        self.ruleset.statics_available() && self.ruleset.statics
    }

    /// Whether a Gift may be kept. The starter always may, and so may an
    /// unknown Gift, which can't be told apart from a Pokémon the Ruleset
    /// allows (an unlisted egg is bred at the Day Care). A used source may
    /// give again when the game reloaded a save from before its Gift and that
    /// first Pokémon (`still_owned` says) is gone.
    pub fn gift_received(&self, origin: &GiftOrigin, still_owned: impl Fn(u32) -> bool) -> GiftVerdict {
        match origin {
            GiftOrigin::Listed(source) if source.kind == SourceKind::Starter => GiftVerdict::Keep,
            GiftOrigin::Unknown => GiftVerdict::Keep,
            _ if !self.ruleset.gifts => GiftVerdict::Decline(DeclineReason::GiftsOff),
            GiftOrigin::Listed(source) if self.used_sources.contains(&source.key) => {
                let undone = self
                    .gift_from
                    .get(&source.key)
                    .is_some_and(|given| self.reset_since(given.at) && !still_owned(given.pid));
                if undone {
                    GiftVerdict::Keep
                } else {
                    GiftVerdict::Decline(DeclineReason::SourceUsed)
                }
            }
            _ => GiftVerdict::Keep,
        }
    }

    pub fn to_release(&self, pid: u32) -> bool {
        self.to_release.contains(&pid)
    }

    pub fn any_to_release(&self) -> bool {
        !self.to_release.is_empty()
    }

    fn dupes_clause(&self) -> bool {
        self.ruleset.dupes_clause_available() && self.ruleset.dupes_clause
    }

    /// Whether the rules need the game's evolution families before the next battle.
    pub fn wants_families(&self) -> bool {
        self.dupes_clause() && self.families.is_none()
    }

    pub fn set_families(&mut self, families: EvolutionFamilies) {
        self.families = Some(families);
    }

    /// A species from the same family as `foe` that the Player has alive, in
    /// the party or PC. Dead, released and traded-away Pokémon don't count.
    fn dupe_of(&self, foe: u16, owned: &[(u32, u16)]) -> Option<u16> {
        if !self.dupes_clause() {
            return None;
        }
        let families = self.families.as_ref()?;
        let family = families.of(foe);
        owned
            .iter()
            .filter(|(pid, _)| !self.is_dead(*pid))
            .map(|&(_, species)| species)
            .find(|&species| families.of(species) == family)
    }

    /// Whether the game should be watched for the Player's first Poké Balls.
    pub fn waiting_for_poke_balls(&self) -> bool {
        self.ruleset.first_encounter && !self.had_poke_balls
    }

    pub fn faint_is_dead(&self) -> bool {
        self.ruleset.faint_is_dead
    }

    pub fn is_dead(&self, pid: u32) -> bool {
        self.dead.contains_key(&pid)
    }

    /// Why a Pokémon in the party must go to the Dead box, if it must.
    pub fn dead_move(&self, pid: u32) -> Option<DeadMove> {
        self.dead.get(&pid).map(|state| match state {
            DeadState::NotMovedYet => DeadMove::First,
            DeadState::MovedOut => DeadMove::Returned,
        })
    }

    pub fn must_nickname(&self) -> bool {
        self.ruleset.must_nickname
    }

    /// The species name of a Pokémon waiting for a name, if it is waiting.
    pub fn nickname_wanted(&self, pid: u32) -> Option<&str> {
        self.nickname_wanted.get(&pid).map(String::as_str)
    }

    pub fn any_nickname_wanted(&self) -> bool {
        !self.nickname_wanted.is_empty()
    }

    pub fn nicknames_wanted(&self) -> impl Iterator<Item = u32> + '_ {
        self.nickname_wanted.keys().copied()
    }

    pub fn type_clause(&self) -> bool {
        self.ruleset.type_clause
    }

    pub fn no_items(&self) -> bool {
        self.ruleset.no_items
    }

    pub fn set_mode(&self) -> bool {
        self.ruleset.set_mode
    }

    pub fn items_banned_said(&self) -> bool {
        self.items_banned_said
    }

    pub fn item_ban_failed_said(&self) -> bool {
        self.item_ban_failed_said
    }

    pub fn set_mode_forced_before(&self) -> bool {
        self.set_mode_forced_before
    }

    pub fn set_mode_failed_said(&self) -> bool {
        self.set_mode_failed_said
    }

    /// The level cap now, once one is set.
    pub fn level_cap(&self) -> Option<u8> {
        self.level_cap
    }

    pub fn cap_stage(&self) -> Option<usize> {
        self.cap_stage
    }

    pub fn is_soft_cap(&self) -> bool {
        self.soft_cap
    }

    /// Takes in an event of the Run, as it happens or when a Run is resumed.
    pub fn record(&mut self, event: &RunEvent) {
        self.events_seen += 1;
        match event {
            RunEvent::Encounter { area, .. } => {
                self.closed.insert(area.id);
            }
            RunEvent::PokeBallsObtained => self.had_poke_balls = true,
            RunEvent::Gift {
                source: Some(source),
                pid,
                ..
            } => {
                self.used_sources.insert(source.key.clone());
                self.gift_from.insert(
                    source.key.clone(),
                    GiftGiven {
                        pid: *pid,
                        at: self.events_seen,
                    },
                );
            }
            RunEvent::StaticEncounter { source, .. } => {
                self.used_sources.insert(source.key.clone());
            }
            RunEvent::GiftDeclined { pid, .. } => {
                self.to_release.insert(*pid);
            }
            RunEvent::LevelCapSet { stage, level, .. } => {
                self.cap_stage = Some(*stage);
                self.level_cap = Some(*level);
            }
            RunEvent::SoftCap { .. } => self.soft_cap = true,
            RunEvent::ItemsBanned => self.items_banned_said = true,
            RunEvent::ItemBanFailed { .. } => self.item_ban_failed_said = true,
            RunEvent::SetModeForced { .. } => self.set_mode_forced_before = true,
            RunEvent::SetModeFailed { .. } => self.set_mode_failed_said = true,
            RunEvent::Released { pid, .. } => {
                self.to_release.remove(pid);
                self.nickname_wanted.remove(pid);
            }
            RunEvent::NicknameWanted { pid, species_name } => {
                self.nickname_wanted.insert(*pid, species_name.clone());
            }
            RunEvent::Nicknamed { pid, .. } => {
                self.nickname_wanted.remove(pid);
            }
            RunEvent::Died { pid, .. } => {
                self.dead.insert(*pid, DeadState::NotMovedYet);
            }
            RunEvent::MovedToDeadBox { pid, .. }
            | RunEvent::DeadReturned { pid, .. }
            | RunEvent::DeadBoxFull { pid, .. } => {
                self.dead.insert(*pid, DeadState::MovedOut);
            }
            // A game booted from its last in-game save, or reset to it, may
            // hold a Dead Pokémon from before it was moved: no Violation.
            // A suspend or a rebuild goes on exactly where the game was.
            RunEvent::Resumed {
                from: ResumedFrom::Save | ResumedFrom::Start,
            }
            | RunEvent::GameReset => {
                self.dead.values_mut().for_each(|state| *state = DeadState::NotMovedYet);
                self.last_reset = Some(self.events_seen);
            }
            _ => {}
        }
    }
}
