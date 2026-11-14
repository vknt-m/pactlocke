//! Seam 2, local only: a Run on the real FireRed (USA)
//! 1.0, driven through the Run engine on the real game interface. Skipped
//! unless `PACTLOCKE_ROMS` names a folder holding `Pokemon - FireRed Version (USA).gba`
//! (or `PACTLOCKE_ROM` names it). Never runs in CI.

mod frlg;

const GAME: frlg::Frlg = frlg::Frlg {
    file: "Pokemon - FireRed Version (USA).gba",
    items: 0x083D_B028,
    choose_action: [0x0802_E438, 0x080E_763C],
    rival_battles: [0x0816_93AC, 0x0816_9424, 0x0816_949F],
    brock_battle: 0x0816_A5A0,
};
