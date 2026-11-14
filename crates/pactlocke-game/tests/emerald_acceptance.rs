//! Seam 3, local only: the Emerald (USA, Europe) Game Profile against
//! the developer's own ROM. Skipped unless `PACTLOCKE_ROMS` names a folder
//! holding `Pokemon - Emerald Version (USA, Europe).gba` (or `PACTLOCKE_ROM` names it). Never runs in CI.

mod hoenn;

const GAME: hoenn::Hoenn = hoenn::Hoenn {
    file: "Pokemon - Emerald Version (USA, Europe).gba",
    id: "emerald-usa-1.0",
    // Each cap stage's ace: Nosepass, Makuhita, Manectric, Torkoal, Slaking, Altaria, Lunatone and
    // Solrock, Kingdra, then Absol, Dusclops, Walrein, Salamence and Wallace's Milotic.
    caps: [
        ("Roxanne", 15),
        ("Brawly", 19),
        ("Wattson", 24),
        ("Flannery", 29),
        ("Norman", 31),
        ("Winona", 33),
        ("Tate & Liza", 42),
        ("Juan", 46),
        ("Sidney", 49),
        ("Phoebe", 51),
        ("Glacia", 53),
        ("Drake", 55),
        ("Champion", 58),
    ],
};
