//! Sync Gates: every battle the Ruleset chose is held, from the first safe
//! moment of each boot, until every Player is Ready there. Coming to a held
//! gate (its hold script running) readies the Player up, unless they are
//! over the level cap. The Run log says when everyone is; then the gate is
//! let go by a stamped patch, and its battle runs through the game's own
//! script.

use super::RunEngine;
use crate::{LiveGame, RunEvent};

impl RunEngine {
    /// Holds every chosen gate not yet let go, once per boot (ROM patches
    /// don't survive one), on the overworld.
    pub(super) fn keep_gates_held(&mut self, game: &mut impl LiveGame) {
        let gates = self.info.ruleset.sync_gates.clone();
        for gate in gates {
            if self.rules.gate_open(&gate) || self.gates_held.contains(&gate) {
                continue;
            }
            self.gates_held.insert(gate.clone());
            match game.close_gate(&gate) {
                Ok(changes) => self.apply(changes, game),
                Err(e) => self.record(RunEvent::GateFailed {
                    gate,
                    reason: e.to_string(),
                }),
            }
        }
    }

    /// Whether the Player has come to a held gate: the first frame its hold
    /// script runs readies them up there, or says why they can't.
    pub(super) fn watch_gates(&mut self, game: &mut impl LiveGame) {
        if self.info.ruleset.sync_gates.is_empty() {
            return;
        }
        let at = game.at_gate();
        if at == self.at_gate {
            return;
        }
        self.at_gate = at.clone();
        let Some(gate) = at.filter(|g| !self.rules.gate_open(g) && !self.rules.ready_at(g)) else {
            return;
        };
        match self.ready_up_refusal(game) {
            Some(reason) => self.record(RunEvent::GateRefused { gate, reason }),
            None => self.record(RunEvent::GateReady { gate }),
        }
    }

    /// A chosen gate not let go that the Player's game is past (its
    /// badge or defeat flag, though the gate held): it counts as Ready
    /// there, said once.
    pub(super) fn pass_gates(&mut self, game: &mut impl LiveGame) {
        let gates = self.info.ruleset.sync_gates.clone();
        for gate in gates {
            if !self.rules.gate_open(&gate) && !self.rules.ready_at(&gate) && game.gate_passed(&gate) {
                self.record(RunEvent::GatePassed { gate });
            }
        }
    }

    /// Every Player is Ready at this gate, or everyone voted to let it go
    /// (`by_vote`): its battle runs here now. With `checkpoint`, this
    /// Player's Ready completed it, so the Run Checkpoint every Sync Gate
    /// takes is called from this game.
    pub fn open_gate(&mut self, gate: &str, checkpoint: bool, by_vote: bool, game: &mut impl LiveGame) {
        if self.rules.gate_open(gate) {
            return;
        }
        match game.open_gate(gate) {
            Ok(changes) => self.apply(changes, game),
            Err(e) => {
                self.record(RunEvent::GateFailed {
                    gate: gate.to_owned(),
                    reason: e.to_string(),
                });
                return;
            }
        }
        self.record(RunEvent::GateOpened {
            gate: gate.to_owned(),
            by_vote,
        });
        if checkpoint {
            self.want_checkpoint_for(crate::CheckpointReason::SyncGate);
        }
    }
}
