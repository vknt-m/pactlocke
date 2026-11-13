#!/usr/bin/env bash
# Runs one rig scenario as headless apps on this machine, with no Docker and
# no relay: each joiner reaches the Host straight, on its local addresses.
# No network shaping, no time zones, and `cut` steps fail (they need the
# rig). Quick to try a scenario before running it in containers.
#
#   PACTLOCKE_ROMS=~/roms bash scripts/rig/local.sh firered/shadow-sync [--debug]
#
# Never in CI: it needs your own ROMs.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
scenario="${1:?usage: local.sh GAME/SCENARIO [--debug]}"
profile=release
[[ "${2:-}" == "--debug" ]] && profile=debug
dir="$here/scenarios/$scenario"
[[ -f "$dir/rig.env" ]] || { echo "no scenario at $dir" >&2; exit 2; }
# shellcheck source=/dev/null
source "$dir/rig.env"
source "$here/roms.sh"
rom="$(rom_for "${scenario%%/*}")"

if [[ "$profile" == release ]]; then
    cargo build --release -p pactlocke-app --quiet
else
    cargo build -p pactlocke-app --quiet
fi
exe="$root/target/$profile/pactlocke"
[[ -x "$exe" || -x "$exe.exe" ]] || { echo "no app at $exe" >&2; exit 2; }

out="$root/target/rig/local/$scenario"
rm -rf "$out"
mkdir -p "$out/shared"
echo "Scenario $scenario: players $PLAYERS; output in $out"

pids=()
names=()
invite=0
for player in $PLAYERS; do
    mkdir -p "$out/$player"
    script="$dir/$(echo "$player" | tr '[:upper:]' '[:lower:]').txt"
    PLAYER="$player" RELAY=off SHARED="$out/shared" ROM="$rom" INVITE="$invite" \
        "$exe" --headless "$script" --report "$out/$player/report.json" --data "$out/$player/data" \
        >"$out/$player/stderr.log" 2>&1 &
    pids+=("$!")
    names+=("$player")
    invite=$((invite + 1))
done

failed=0
for i in "${!pids[@]}"; do
    code=0
    wait "${pids[$i]}" || code=$?
    echo "  ${names[$i]}: exit $code"
    [[ $code -eq 0 || $code -eq 3 ]] || failed=1
done
bash "$here/check-reports.sh" "$out" $PLAYERS || failed=1
exit $failed
