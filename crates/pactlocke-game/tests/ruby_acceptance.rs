//! Seam 3, local only: the Ruby (USA) 1.0 Game Profile against
//! the developer's own ROM. Skipped unless `PACTLOCKE_ROMS` names a folder
//! holding `Pokemon - Ruby Version (USA).gba` (or `PACTLOCKE_ROM` names it). Never runs in CI.

mod hoenn;

const GAME: hoenn::Hoenn = hoenn::Hoenn {
    file: "Pokemon - Ruby Version (USA).gba",
    id: "ruby-usa-1.0",
    // Each cap stage's ace: Nosepass, Makuhita, Magneton, Torkoal, Slaking, Altaria, Lunatone and
    // Solrock, Wallace's Milotic, then Absol, Dusclops, Walrein, Salamence and Steven's Metagross.
    caps: [
        ("Roxanne", 15),
        ("Brawly", 18),
        ("Wattson", 23),
        ("Flannery", 28),
        ("Norman", 31),
        ("Winona", 33),
        ("Tate & Liza", 42),
        ("Wallace", 43),
        ("Sidney", 49),
        ("Phoebe", 51),
        ("Glacia", 53),
        ("Drake", 55),
        ("Champion", 58),
    ],
};
