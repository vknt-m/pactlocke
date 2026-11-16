//! Seam 2, local only: a Run on the real Gaia 3.2, driven
//! through the Run engine on the real game interface. Skipped unless
//! `PACTLOCKE_ROMHACKS` names a folder holding `GAIA/` with the patched ROM.
//! Never runs in CI.

mod romhack;

// Gaia keeps FireRed's RAM, so these are pret's pokefirered symbols.
const GAME: romhack::Hack = romhack::Hack {
    folder: "GAIA",
    battler_controllers: 0x0300_4FE0,
    choose_action: [0x0802_E438, 0x080E_763C],
    field_lock: 0x0300_0F9C,
    // Nicola of the Elite Four, FireRed's Lorelei.
    single_trainer: 410,
    test_scripts: 0x09F0_1000,
    sprites: romhack::FIRERED_SPRITES,
    scripts: romhack::Scripts::Original,
    poke_ball: 4,
    intro: "",
    own_randomizer: None,
    variable_gate: None,
};
