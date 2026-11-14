//! Seam 2, local only: a Run on the real LeafGreen (USA)
//! 1.0, driven through the Run engine on the real game interface. Skipped
//! unless `PACTLOCKE_ROMS` names a folder holding `Pokemon - LeafGreen Version (USA).gba`
//! (or `PACTLOCKE_ROM` names it). Never runs in CI.

mod frlg;

const GAME: frlg::Frlg = frlg::Frlg {
    file: "Pokemon - LeafGreen Version (USA).gba",
    items: 0x083D_AE64,
    choose_action: [0x0802_E438, 0x080E_7614],
    rival_battles: [0x0816_9388, 0x0816_9400, 0x0816_947B],
    brock_battle: 0x0816_A57C,
};
