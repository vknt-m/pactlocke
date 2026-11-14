//! Seam 3, local only: the FireRed (USA) 1.0 Game
//! Profile against the developer's own ROM. Skipped unless `PACTLOCKE_ROMS`
//! names a folder holding `Pokemon - FireRed Version (USA).gba` (or
//! `PACTLOCKE_ROM` names it). Never runs in CI.

mod frlg;

const GAME: frlg::Frlg = frlg::Frlg {
    file: "Pokemon - FireRed Version (USA).gba",
    id: "firered-usa-1.0",
};
