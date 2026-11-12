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
    /// The Area's one Encounter: catching is allowed.
    Encounter,
    /// The Area is closed: ball throws are refused for this battle.
    BlockCatches,
}

pub(crate) struct Rules {
    ruleset: Ruleset,
    /// Areas whose Encounter is used.
    closed: BTreeSet<u16>,
}

impl Rules {
    pub fn new(ruleset: Ruleset) -> Self {
        Self {
            ruleset,
            closed: BTreeSet::new(),
        }
    }

    /// What a battle that just began means.
    pub fn battle_started(&self, start: &BattleStart) -> Verdict {
        if !self.ruleset.first_encounter || start.kind != BattleKind::Wild || !start.script_idle {
            return Verdict::Nothing;
        }
        match start.area {
            None => Verdict::Nothing,
            Some(area) if self.closed.contains(&area.id) => Verdict::BlockCatches,
            Some(_) => Verdict::Encounter,
        }
    }

    /// Takes in an event of the Run, as it happens or when a Run is resumed.
    pub fn record(&mut self, event: &RunEvent) {
        if let RunEvent::Encounter { area, .. } = event {
            self.closed.insert(area.id);
        }
    }
}
