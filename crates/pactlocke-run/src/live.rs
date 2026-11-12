use pactlocke_emu::CoreAdapter;
use pactlocke_game::{Change, FakeGame, Game, ProfileGame};

/// A game the Run engine can both read and change. Only the Run engine calls
/// [`LiveGame::apply`], at a frame boundary, after stamping the change.
pub trait LiveGame: Game {
    fn apply(&mut self, change: &Change);
}

impl LiveGame for FakeGame {
    fn apply(&mut self, change: &Change) {
        FakeGame::apply(self, std::slice::from_ref(change));
    }
}

impl<C: CoreAdapter> LiveGame for ProfileGame<C> {
    fn apply(&mut self, change: &Change) {
        match change {
            Change::Write { addr, bytes } => self.memory_mut().write(*addr, bytes),
            Change::Patch { addr, bytes } => self.memory_mut().patch_rom(*addr, bytes),
        }
    }
}
