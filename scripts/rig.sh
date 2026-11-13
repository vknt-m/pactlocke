#!/usr/bin/env bash
# The multi-machine test rig: two or three headless Pactlockes, each in its
# own Linux container with its own network delay, loss and time zone, play a
# scenario's key scripts on your own ROMs and reach each other over iroh
# through a relay of their own (iroh-relay in a container). The containers'
# network is internal: nothing reaches the internet. Each app writes a
# report, and the reports are checked against each other.
#
#   PACTLOCKE_ROMS=~/roms bash scripts/rig.sh firered/shadow-sync
#   PACTLOCKE_ROMS=~/roms bash scripts/rig.sh firered        # every FireRed scenario
#   ... --arm64 Blue      # Blue's container is ARM64, under emulation (slow to build once)
#   ... --keep            # leave the containers, to look at them afterwards
#
# Scenarios are folders in scripts/rig/scenarios/<game>/: rig.env names the
# Players and their network and time zone, and <player>.txt is each one's
# script (docs/test-rig.md). ROMs are mounted read-only and never copied
# into an image or the repo. Local only, never in CI: it needs your ROMs.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
here="$root/scripts/rig"
export MSYS_NO_PATHCONV=1

RUST_VERSION="$(grep -oE '^channel *= *"[^"]+"' rust-toolchain.toml | grep -oE '"[^"]+"' | tr -d '"')"
RELAY_VERSION="$(grep -A1 '^name = "iroh-relay"$' Cargo.lock | grep -oE '"[0-9][^"]*"' | tr -d '"')"
CI_IMAGE="pactlocke-ci:${RUST_VERSION}"
RIG_IMAGE="pactlocke-rig:${RUST_VERSION}"
NET=pactlocke-rig
RELAY=pactlocke-rig-relay
BINS=pactlocke-rig-bins
SHARED=pactlocke-rig-shared
VENDOR_VOLUME=pactlocke-ci-vendor
# The longest one scenario may take.
LIMIT_S=1500

which="" arm64="" keep=false
while [ $# -gt 0 ]; do
	case "$1" in
	--arm64) arm64="${2:?--arm64 needs a Player}"; shift ;;
	--keep) keep=true ;;
	-*) echo "unknown option $1" >&2; exit 2 ;;
	*) which="$1" ;;
	esac
	shift
done
[ -n "$which" ] || { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

host_path() {
	if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else echo "$1"; fi
}

docker info >/dev/null 2>&1 || { echo "Docker isn't running: start Docker Desktop, then run this again." >&2; exit 1; }
# shellcheck source=scripts/rig/roms.sh
source "$here/roms.sh"
game="${which%%/*}"
rom="$(rom_for "$game")"
rom_name="$(basename "$rom")"
roms_dir="$(host_path "$(dirname "$rom")")"

if [ "$which" = "$game" ]; then
	scenarios=()
	for dir in "$here/scenarios/$game"/*/; do
		name="$(basename "$dir")"
		[ "$name" = common ] || scenarios+=("$game/$name")
	done
else
	scenarios=("$which")
fi
for scenario in "${scenarios[@]}"; do
	[ -f "$here/scenarios/$scenario/rig.env" ] || { echo "no scenario at scripts/rig/scenarios/$scenario" >&2; exit 2; }
done

echo "=== Images"
if ! docker image inspect "$CI_IMAGE" >/dev/null 2>&1; then
	docker build --build-arg "RUST_VERSION=${RUST_VERSION}" -t "$CI_IMAGE" scripts/ci-docker
fi
# Built again whenever its files change: their hash is a label on the image.
sum="$(cat scripts/rig/docker/* | sha1sum | cut -c1-40)"
stale() {
	[ "$(docker image inspect -f '{{index .Config.Labels "rig.sum"}}' "$1" 2>/dev/null)" != "$sum" ]
}
if stale "$RIG_IMAGE"; then
	docker build --target rig --label "rig.sum=$sum" --build-arg "RUST_VERSION=${RUST_VERSION}" \
		--build-arg "IROH_RELAY_VERSION=${RELAY_VERSION}" -t "$RIG_IMAGE" scripts/rig/docker
fi
echo "$RIG_IMAGE, with iroh-relay $RELAY_VERSION"

echo "=== Crates"
# shellcheck source=scripts/ci-docker/vendor.sh
source scripts/ci-docker/vendor.sh
ensure_vendor "$CI_IMAGE" "$VENDOR_VOLUME"

echo "=== The app, for x86_64"
build() {
	local platform="$1" image="$2" target="$3"
	tar --exclude=./target --exclude=.git --exclude=./roms --exclude=./.claude -cf - . |
		docker run -i --rm --platform "$platform" --network none \
			-v "$VENDOR_VOLUME:/vendor:ro" -v "$target:/work/target" -v "$BINS:/bins" \
			"$image" bash -c "$(cat scripts/rig/docker/build.sh)"
}
build linux/amd64 "$CI_IMAGE" pactlocke-rig-target
if [ -n "$arm64" ]; then
	echo "=== The app, for ARM64 (under emulation: the first build is slow)"
	machine="$(docker run --rm --platform linux/arm64 debian:bookworm-slim uname -m 2>/dev/null || true)"
	if [ "$machine" != aarch64 ]; then
		echo "This Docker can't run ARM64 containers (no emulation set up), so --arm64 can't be used here." >&2
		exit 1
	fi
	if ! docker image inspect "$CI_IMAGE-arm64" >/dev/null 2>&1; then
		docker build --platform linux/arm64 --build-arg "RUST_VERSION=${RUST_VERSION}" -t "$CI_IMAGE-arm64" scripts/ci-docker
	fi
	if stale "$RIG_IMAGE-arm64"; then
		docker build --platform linux/arm64 --target player --label "rig.sum=$sum" \
			--build-arg "RUST_VERSION=${RUST_VERSION}" -t "$RIG_IMAGE-arm64" scripts/rig/docker
	fi
	build linux/arm64 "$CI_IMAGE-arm64" pactlocke-rig-target-arm64
fi

cleanup() {
	docker rm -f $(docker ps -aq --filter "name=^pactlocke-rig-") >/dev/null 2>&1 || true
	docker network rm "$NET" >/dev/null 2>&1 || true
}

run_scenario() {
	local scenario="$1"
	local out="$root/target/rig/docker/$scenario"
	rm -rf "$out"
	mkdir -p "$out"
	# shellcheck source=/dev/null
	PLAYERS="" && source "$here/scenarios/$scenario/rig.env"
	echo "=== $scenario: $PLAYERS"
	cleanup
	docker volume rm -f "$SHARED" >/dev/null
	docker network create --internal "$NET" >/dev/null
	docker run -d --name "$RELAY" --network "$NET" "$RIG_IMAGE" iroh-relay --dev >/dev/null

	local invite=0 player
	for player in $PLAYERS; do
		local netem_var="NETEM_$player" tz_var="TZ_$player" platform=linux/amd64 image="$RIG_IMAGE"
		if [ "$player" = "$arm64" ]; then platform=linux/arm64 image="$RIG_IMAGE-arm64"; fi
		docker run -d --name "pactlocke-rig-$player" --network "$NET" --platform "$platform" --cap-add NET_ADMIN \
			-e "PLAYER=$player" -e "SCENARIO=$scenario" -e "INVITE=$invite" \
			-e "NETEM=${!netem_var:-}" -e "TZ=${!tz_var:-UTC}" \
			-e "RELAY=http://$RELAY:3340" -e "SHARED=/shared" -e "ROM=/roms/$rom_name" \
			-e "PACTLOCKE_CUT=/rig/cut.sh" \
			-v "$roms_dir:/roms:ro" -v "$(host_path "$here/scenarios"):/scenarios:ro" \
			-v "$BINS:/bins:ro" -v "$SHARED:/shared" \
			"$image" /rig/player.sh >/dev/null
		echo "  $player: $platform, TZ ${!tz_var:-UTC}, netem: ${!netem_var:-none}"
		invite=$((invite + 1))
	done

	local started=$SECONDS failed=0
	for player in $PLAYERS; do
		while [ "$(docker inspect -f '{{.State.Running}}' "pactlocke-rig-$player")" = true ]; do
			if [ $((SECONDS - started)) -gt $LIMIT_S ]; then
				echo "  $player is still running after ${LIMIT_S}s: stopping it"
				docker stop -t 2 "pactlocke-rig-$player" >/dev/null
			fi
			sleep 2
		done
		local code
		code="$(docker inspect -f '{{.State.ExitCode}}' "pactlocke-rig-$player")"
		docker logs "pactlocke-rig-$player" >"$out/$player.log" 2>&1
		echo "  $player: exit $code ($((SECONDS - started)) s)"
		case "$code" in 0 | 3) ;; *) failed=1 ;; esac
	done
	docker logs "$RELAY" >"$out/relay.log" 2>&1
	docker run --rm -v "$SHARED:/shared:ro" "$RIG_IMAGE" tar -C /shared/reports -cf - . | tar -C "$out" -xf -
	# shellcheck disable=SC2086
	bash "$here/check-reports.sh" "$(host_path "$out")" $PLAYERS || failed=1
	$keep || cleanup
	echo "  output in target/rig/docker/$scenario"
	return $failed
}

passed=() failures=()
for scenario in "${scenarios[@]}"; do
	if run_scenario "$scenario"; then passed+=("$scenario"); else failures+=("$scenario"); fi
done
echo
echo "=== Passed: ${passed[*]:-none}"
if [ ${#failures[@]} -gt 0 ]; then
	echo "=== FAILED: ${failures[*]}"
	exit 1
fi
