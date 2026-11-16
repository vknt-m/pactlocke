//! Seam 2, local only: a Run on the real Unbound 2.1.1.1,
//! driven through the Run engine on the real game interface. Skipped unless
//! `PACTLOCKE_ROMHACKS` names a folder holding `Unbound/` with the patched
//! ROM. Never runs in CI.

mod romhack;

// Unbound keeps FireRed's RAM, so these are pret's pokefirered symbols.
const GAME: romhack::Hack = romhack::Hack {
    folder: "Unbound",
    battler_controllers: 0x0300_4FE0,
    choose_action: [0x0802_E438, 0x080E_763C],
    field_lock: 0x0300_0F9C,
    // Moleman of the Elite Four.
    single_trainer: 410,
    // Past the Sync Gates' hold area: the end of the ROM is free.
    test_scripts: 0x09FF_1000,
    // Mel's gym battle: `setvar 0x8000, 4`, then the CFRU routine that picks
    // the difficulty's Mel into 0x800D, then `trainerbattle 3, 0x800D`.
    sprites: romhack::FIRERED_SPRITES,
    scripts: romhack::Scripts::Original,
    poke_ball: 4,
    intro: "",
    own_randomizer: None,
    variable_gate: Some(romhack::VariableGate {
        gate: "Mel",
        script: 0x09E7_309C,
        opponent: 0x0203_86AE,
        trainers: &[416, 78, 79, 728, 729, 730],
    }),
};
