//! Seam 2, local only: a Run on the real Emerald (USA,
//! Europe), driven through the Run engine on the real game interface.
//! Skipped unless `PACTLOCKE_ROMS` names a folder holding
//! `Pokemon - Emerald Version (USA, Europe).gba` (or `PACTLOCKE_ROM` names
//! it). Never runs in CI.

mod hoenn;

const GAME: hoenn::Hoenn = hoenn::Hoenn {
    file: "Pokemon - Emerald Version (USA, Europe).gba",
    battler_controllers: 0x0300_5D60,
    choose_action: [0x0805_7588, 0x0815_93D8],
    field_lock: 0x0300_0F2C,
    safari_balls: 0x0203_A04C,
    birchs_bag: 0x081E_BE16,
    enter_safari_mode: 0xD0,
    store_in_daycare: 0xBE,
    take_from_daycare: 0xC3,
    daycare_slot: 0x0203_CED1,
    bag_key: Some(0xAC),
};
