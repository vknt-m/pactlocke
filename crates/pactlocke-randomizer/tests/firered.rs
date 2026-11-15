//! The local check: FVX, installed for real, randomizes your own FireRed.
//! Skipped unless `PACTLOCKE_ROMS` names a folder holding
//! `Pokemon - FireRed Version (USA).gba` and `PACTLOCKE_FVX` names a folder
//! to install FVX into (downloaded there the first time, about 41 MB).
//! The output is left at `<PACTLOCKE_FVX>/firered-randomized.gba` for the
//! FireRed acceptance tests (`PACTLOCKE_ROM`).

use std::path::PathBuf;

use pactlocke_randomizer::{Recipe, asset_for_this_machine, download, install, randomize};

/// What pactlocke-v1 issue 34 got from UPR's own run: FireRed with these options and seed 12345.
const PINNED_SHA1: &str = "caf4d714e3c34c6f339f758c9040b11b52937f35";

#[test]
fn firered_randomizes_to_the_pinned_game() {
    let (Some(roms), Some(fvx_dir)) = (std::env::var_os("PACTLOCKE_ROMS"), std::env::var_os("PACTLOCKE_FVX")) else {
        eprintln!("skipped: set PACTLOCKE_ROMS and PACTLOCKE_FVX");
        return;
    };
    let rom = std::fs::read(PathBuf::from(roms).join("Pokemon - FireRed Version (USA).gba")).expect("your FireRed");
    let data = PathBuf::from(fvx_dir);
    let asset = asset_for_this_machine().expect("an FVX build for this machine");
    let fvx = install(&data, asset, download).expect("FVX installs");
    // Every switch on gives the options issue 34 tested.
    let recipe = Recipe {
        types: true,
        base_stats: true,
        ..Recipe::default()
    };
    let made = randomize(&fvx, &rom, &recipe.settings_string(), 12345, &data.join("work")).expect("FVX randomizes");
    std::fs::write(data.join("firered-randomized.gba"), &made.rom).unwrap();
    assert_eq!(made.sha1, PINNED_SHA1);
}
