# Notices

Pactlocke
Copyright (C) 2026 vknt-m

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License along with
this program, in [LICENSE](LICENSE). If not, see <https://www.gnu.org/licenses/>.

## Third-party software in the app

Pactlocke is built with the following. Each keeps its own licence and copyright;
none of them is relicensed by Pactlocke's GPL.

- **mGBA** (libmgba, the emulator core). Copyright © 2013–2026 Jeffrey Pfau.
  Mozilla Public License 2.0. Its source, unmodified, is the `third_party/mgba`
  submodule, from <https://github.com/mgba-emu/mgba>. As Pactlocke builds it,
  mGBA includes two pieces of its own bundled code: **inih**, copyright ©
  2009–2020 Ben Hoyt, under the BSD 3-Clause licence, and the **LZMA SDK** by
  Igor Pavlov, in the public domain. Its other bundled libraries are not built.
- **egui and eframe** (the interface), by Emil Ernerfeldt and contributors.
  MIT or Apache-2.0.
- **iroh** (networking), by n0 and contributors. MIT or Apache-2.0.
- **Rust crates.** The app uses about 425 crates from crates.io, with the exact
  versions in [Cargo.lock](Cargo.lock). They are under permissive licences, all
  compatible with the GPL: mostly MIT and Apache-2.0, and also BSD-2-Clause,
  BSD-3-Clause, ISC, Zlib, Unicode-3.0, BSL-1.0, CDLA-Permissive-2.0 and
  MPL-2.0. Each crate's licence and copyright notice ships with its source.

Every release of the app carries these licences in full: THIRD-PARTY-LICENSES.html
(made by [cargo-about](https://github.com/EmbarkStudios/cargo-about) from
[about.toml](about.toml)) holds each crate's licence text, and the `licences`
folder holds mGBA's and inih's licences, the crates' notice files and the
fonts' licences. `cargo about` fails the build when a crate's licence isn't one
of those accepted in about.toml.

### Fonts

egui's default fonts are built into the app:

- **Ubuntu Light**, copyright Canonical Ltd. Ubuntu Font Licence 1.0.
- **Noto Emoji**, copyright Google LLC. SIL Open Font License 1.1.
- **emoji-icon-font** and its subset egui-icons, copyright © 2014 John Slegers.
  MIT.
- **Hack**, copyright 2018 Source Foundry Authors. MIT, with the Bitstream Vera
  licence for the glyphs it takes from Bitstream Vera; DejaVu's changes are in
  the public domain.

## Used, not included

- **Universal Pokémon Randomizer FVX**
  (<https://github.com/upr-fvx/universal-pokemon-randomizer-fvx>), by its
  authors, under the GNU General Public License v3.0. Pactlocke downloads it
  only when a Player turns on randomizing; it is not part of this repository or
  the app.
- **Romhacks.** Pactlocke has Game Profiles for these hacks but includes none of
  them: Radical Red by soupercell, Unbound by Skeli, Gaia by Spherical Ice,
  Glazed by Gamer2020, Heart and Soul by the Pokémon HnS Development team,
  R.O.W.E. by Rioluwott.

## Games and trademarks

Pactlocke includes no game, ROM or BIOS, and does not condone piracy: play only
games you own and have dumped yourself.

Pactlocke is a fan project. It is not affiliated with or endorsed by Nintendo,
Game Freak, Creatures or The Pokémon Company. Pokémon and its characters are
their trademarks.
