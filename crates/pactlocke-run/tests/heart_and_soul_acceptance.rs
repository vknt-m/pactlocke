//! Seam 2, local only: a Run on the real Heart and Soul 2.0.6,
//! driven through the Run engine on the real game interface. Skipped unless
//! `PACTLOCKE_ROMHACKS` names a folder holding `H&S/` with the patched ROM.
//! Never runs in CI.

mod romhack;

// Heart and Soul's own RAM (pokeemerald-expansion 1.15), found in its RAM.
const GAME: romhack::Hack = romhack::Hack {
    folder: "H&S",
    battler_controllers: 0x0300_2F54,
    choose_action: [0x0805_D1C5, 0x0805_D1C5],
    field_lock: 0x0300_1FDD,
    // Steven, with a full team.
    single_trainer: 0x264,
    test_scripts: 0x09F0_1000,
    sprites: romhack::EMERALD_SPRITES,
    variable_gate: None,
    scripts: romhack::Scripts::Expansion { createmon: 0x0A20_B30D },
    poke_ball: 1,
    // Its challenge menu opens before the intro: R through its eight pages,
    // START, then A to confirm.
    intro: "m A 4000\np RS 40\np RS 40\np RS 40\np RS 40\np RS 40\np RS 40\np RS 40\np RS 40\np S 60\np A 60",
    // Its STATIC option (challenge settings +0x05, bit 0x20) randomizes Statics.
    own_randomizer: Some(romhack::OwnRandomizer {
        on: romhack::RandomizerSwitch::SaveBlock3 {
            offset: 0x15,
            mask: 0x20,
        },
        shows_in: romhack::SeedShows::Static,
    }),
};
