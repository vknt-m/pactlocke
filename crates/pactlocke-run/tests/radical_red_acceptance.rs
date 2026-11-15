//! Seam 2, local only: a Run on the real Radical Red 4.1,
//! driven through the Run engine on the real game interface. Skipped unless
//! `PACTLOCKE_ROMHACKS` names a folder holding `Radical Red/` with the patched
//! ROM. Never runs in CI.

mod romhack;

// Radical Red keeps FireRed's RAM, so these are pret's pokefirered symbols.
const GAME: romhack::Hack = romhack::Hack {
    folder: "Radical Red",
    battler_controllers: 0x0300_4FE0,
    choose_action: [0x0802_E438, 0x080E_763C],
    field_lock: 0x0300_0F9C,
    // Lorelei of the Elite Four.
    single_trainer: 410,
    // Past the Sync Gates' hold area: the end of the ROM is free.
    test_scripts: 0x09FF_1000,
    variable_gate: None,
};
