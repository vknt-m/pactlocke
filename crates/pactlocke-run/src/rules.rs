//! The rules engine: pure logic for one Player. Given the locked Ruleset and
//! the Run's state, it decides what a moment in the game means. It reads
//! nothing and changes nothing itself.

use std::collections::BTreeSet;

use pactlocke_game::{Area, BattleKind};

use crate::{Ruleset, RunEvent};

/// How a battle began.
pub(crate) struct BattleStart<'a> {
    pub kind: BattleKind,
    /// No script was running when it began: the Player met it in the wild.
    pub script_idle: bool,
    pub area: Option<&'a Area>,
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
}

pub(crate) struct Rules {
    ruleset: Ruleset,
    /// Areas whose Encounter is used.
    closed: BTreeSet<u16>,
    /// Whether the Player has had Poké Balls; until then no wild battle counts.
    had_poke_balls: bool,
}

impl Rules {
    pub fn new(ruleset: Ruleset) -> Self {
        Self {
            ruleset,
            closed: BTreeSet::new(),
            had_poke_balls: false,
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
            Some(_) => Verdict::Encounter,
        }
    }

    /// Whether the game should be watched for the Player's first Poké Balls.
    pub fn waiting_for_poke_balls(&self) -> bool {
        self.ruleset.first_encounter && !self.had_poke_balls
    }

    /// Takes in an event of the Run, as it happens or when a Run is resumed.
    pub fn record(&mut self, event: &RunEvent) {
        match event {
            RunEvent::Encounter { area, .. } => {
                self.closed.insert(area.id);
            }
            RunEvent::PokeBallsObtained => self.had_poke_balls = true,
            _ => {}
        }
    }
}
