//! Seam 2, local only: a Run on the real R.O.W.E. 2.1.9.1,
//! driven through the Run engine on the real game interface. Skipped unless
//! `PACTLOCKE_ROMHACKS` names a folder holding `Emerald ROWE/` with the
//! patched ROM. Never runs in CI.

mod romhack;

// R.O.W.E.'s own RAM, found in its RAM.
const GAME: romhack::Hack = romhack::Hack {
    folder: "Emerald ROWE",
    battler_controllers: 0x0300_4AD0,
    choose_action: [0x0807_5791, 0x0807_5791],
    field_lock: 0x0203_F72C,
    // Bug Catcher Rick.
    single_trainer: 615,
    test_scripts: 0x09D0_1000,
    sprites: romhack::EMERALD_SPRITES,
    variable_gate: None,
    scripts: romhack::Scripts::Rowe,
    poke_ball: 4,
    // Its new-game questions (Hard mode and its other modes: No to each),
    // then the clock: UP to set it, A to confirm.
    intro: "m A 12000\np D 20\np D 20\np A 80\np A 80\np D 20\np D 20\np A 80\np A 80\np D 20\np D 20\np A 80\np A 80\np D 20\np D 20\np A 80\np A 80\np D 20\np D 20\np A 80\np A 80\nm A 16000\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40\np U 20\np A 40",
    // Its randomized mode (flag 0x28) randomizes trainers' teams.
    own_randomizer: Some(romhack::OwnRandomizer {
        on: romhack::RandomizerSwitch::Flag(0x28),
        shows_in: romhack::SeedShows::Trainer,
    }),
};
