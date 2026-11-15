# Where each game's ROM is: an original game in PACTLOCKE_ROMS, a patched
# Romhack in PACTLOCKE_ROMHACKS (the developer's own folders), by the file
# names the acceptance tests use. Sourced by the rig scripts.

rom_file() {
    case "$1" in
        firered) echo "Pokemon - FireRed Version (USA).gba" ;;
        emerald) echo "Pokemon - Emerald Version (USA, Europe).gba" ;;
        radical-red) echo "Radical Red/Radical_Red_4.1.gba" ;;
        *) echo "no ROM is known for game $1" >&2; return 2 ;;
    esac
}

rom_for() {
    local roms
    case "$1" in
        radical-red) roms="${PACTLOCKE_ROMHACKS:?set PACTLOCKE_ROMHACKS to the folder with your patched Romhacks}" ;;
        *) roms="${PACTLOCKE_ROMS:?set PACTLOCKE_ROMS to the folder with your own ROMs}" ;;
    esac
    local file
    file="$(rom_file "$1")"
    [[ -f "$roms/$file" ]] || { echo "no $file in $roms" >&2; return 2; }
    echo "$roms/$file"
}
