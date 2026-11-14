//! Seam 2, local only: a Run on the real Ruby (USA) 1.0,
//! driven through the Run engine on the real game interface. Skipped unless
//! `PACTLOCKE_ROMS` names a folder holding `Pokemon - Ruby Version (USA).gba`
//! (or `PACTLOCKE_ROM` names it). Never runs in CI.

mod hoenn;

const GAME: hoenn::Hoenn = hoenn::Hoenn {
    file: "Pokemon - Ruby Version (USA).gba",
    battler_controllers: 0x0300_4330,
    choose_action: [0x0802_C098, 0x0812_B4D4],
    field_lock: 0x0300_06A4,
    safari_balls: 0x0203_8808,
    birchs_bag: 0x0814_EA7F,
    enter_safari_mode: 0xCD,
    store_in_daycare: 0xBB,
    take_from_daycare: 0xC0,
    daycare_slot: 0x0300_5CE0,
    bag_key: None,
};
