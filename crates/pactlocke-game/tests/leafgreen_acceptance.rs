//! Seam 3, local only: the LeafGreen (USA) 1.0 Game
//! Profile against the developer's own ROM. Skipped unless `PACTLOCKE_ROMS`
//! names a folder holding `Pokemon - LeafGreen Version (USA).gba` (or
//! `PACTLOCKE_ROM` names it). Never runs in CI.

mod frlg;

const GAME: frlg::Frlg = frlg::Frlg {
    file: "Pokemon - LeafGreen Version (USA).gba",
    id: "leafgreen-usa-1.0",
};
