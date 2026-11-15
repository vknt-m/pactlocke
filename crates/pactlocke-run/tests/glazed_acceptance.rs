//! Seam 2, local only: a Run on the real Glazed 9.2.0, driven
//! through the Run engine on the real game interface. Skipped unless
//! `PACTLOCKE_ROMHACKS` names a folder holding `Glazed/` with the patched
//! ROM. Never runs in CI.

mod romhack;

// Glazed keeps Emerald's RAM, so these are pret's pokeemerald symbols.
const GAME: romhack::Hack = romhack::Hack {
    folder: "Glazed",
    battler_controllers: 0x0300_5D60,
    choose_action: [0x0805_7588, 0x0815_93D8],
    field_lock: 0x0300_0F2C,
    // Rosaline of the Elite Four, Emerald's Sidney.
    single_trainer: 261,
    test_scripts: 0x09F0_1000,
    sprites: romhack::EMERALD_SPRITES,
    variable_gate: None,
};
