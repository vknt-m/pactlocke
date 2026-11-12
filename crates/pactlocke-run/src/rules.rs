//! The rules engine: pure logic for one Player. Given the locked Ruleset and
//! the Run's state, it decides what a moment in the game means. It reads
//! nothing and changes nothing itself.

use std::collections::{BTreeMap, BTreeSet};

use pactlocke_game::{Area, BattleKind};

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
        }
    }

    /// What a battle that just began means.
    pub fn battle_started(&self, start: &BattleStart) -> Verdict {
        if !self.ruleset.first_encounter || start.kind != BattleKind::Wild || !start.script_idle {
            return Verdict::Nothing;
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

    /// Takes in an event of the Run, as it happens or when a Run is resumed.
    pub fn record(&mut self, event: &RunEvent) {
        match event {
            RunEvent::Encounter { area, .. } => {
                self.closed.insert(area.id);
            }
            RunEvent::PokeBallsObtained => self.had_poke_balls = true,
            RunEvent::Died { pid, .. } => {
                self.dead.insert(*pid, DeadState::NotMovedYet);
            }
            RunEvent::MovedToDeadBox { pid, .. }
            | RunEvent::DeadReturned { pid, .. }
            | RunEvent::DeadBoxFull { pid, .. } => {
                self.dead.insert(*pid, DeadState::MovedOut);
            }
            // A newly booted game holds the last in-game save, which may be
            // from before a Dead Pokémon was moved.
            RunEvent::Resumed => self.dead.values_mut().for_each(|state| *state = DeadState::NotMovedYet),
            _ => {}
        }
    }
}
