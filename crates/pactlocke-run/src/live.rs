use pactlocke_emu::CoreAdapter;
use pactlocke_game::{Change, FakeCore, FakeGame, Game, ProfileGame};

/// A game the Run engine can both read and change. Only the Run engine calls
/// [`LiveGame::apply`], at a frame boundary, after stamping the change.
pub trait LiveGame: Game {
    fn apply(&mut self, change: &Change);
}

/// A game read through the game interface on top of an emulator core: what
/// a Player plays in a Run. The core is what runs frames, is checkpointed
/// and is hashed for the Player's Shadow Games.
pub trait PlayedGame: LiveGame + Send {
    type Core: CoreAdapter;
    /// The emulator core the game runs on.
    fn core(&self) -> &Self::Core;
    fn core_mut(&mut self) -> &mut Self::Core;
    /// Puts the ROM's own bytes back over a patch, as a Retry does before
    /// it restores a checkpoint's patch set.
    fn unpatch(&mut self, addr: u32, len: usize);
}

/// The fake's changes land on the fake game and on the fake core under it,
/// so its Shadow Games see them as bytes, as a real game's.
impl LiveGame for FakeGame {
    fn apply(&mut self, change: &Change) {
        FakeGame::apply(self, std::slice::from_ref(change));
        land(&mut self.core, change);
    }
}

impl PlayedGame for FakeGame {
    type Core = FakeCore;

    fn core(&self) -> &FakeCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut FakeCore {
        &mut self.core
    }

    fn unpatch(&mut self, addr: u32, len: usize) {
        self.core.unpatch(addr, len);
    }
}

impl<C: CoreAdapter> LiveGame for ProfileGame<C> {
    fn apply(&mut self, change: &Change) {
        land(self.memory_mut(), change);
    }
}

impl<C: CoreAdapter> PlayedGame for ProfileGame<C> {
    type Core = C;

    fn core(&self) -> &C {
        self.memory()
    }

    fn core_mut(&mut self) -> &mut C {
        self.memory_mut()
    }

    fn unpatch(&mut self, addr: u32, len: usize) {
        let start = addr.wrapping_sub(pactlocke_game::ROM_BASE) as usize;
        if let Some(original) = self.rom().get(start..start + len) {
            let original = original.to_vec();
            self.memory_mut().patch_rom(addr, &original);
        }
    }
}

fn land(core: &mut impl CoreAdapter, change: &Change) {
    match change {
        Change::Write { addr, bytes } => core.write(*addr, bytes),
        Change::Patch { addr, bytes } => core.patch_rom(*addr, bytes),
    }
}
