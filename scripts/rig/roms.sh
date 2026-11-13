# Where each game's ROM is in PACTLOCKE_ROMS (the developer's own folder),
# by the file names the acceptance tests use. Sourced by the rig scripts.

rom_file() {
    case "$1" in
        firered) echo "Pokemon - FireRed Version (USA).gba" ;;
        emerald) echo "Pokemon - Emerald Version (USA, Europe).gba" ;;
        *) echo "no ROM is known for game $1" >&2; return 2 ;;
    esac
}

rom_for() {
    local roms="${PACTLOCKE_ROMS:?set PACTLOCKE_ROMS to the folder with your own ROMs}"
    local file
    file="$(rom_file "$1")"
    [[ -f "$roms/$file" ]] || { echo "no $file in $roms" >&2; return 2; }
    echo "$roms/$file"
}
