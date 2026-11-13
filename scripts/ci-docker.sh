#!/usr/bin/env bash
# CI's Linux job, locally, in Docker: no game files committed, the libmgba
# pin, formatting, clippy with warnings as errors, every test, the core in
# other time zones, the app build and a smoke launch under xvfb, on the Rust
# version in scripts/ci-docker/Dockerfile.
#
# The container has no network and reads nothing through Docker's file
# sharing, which is very slow for many small files on Windows: crates are
# vendored here and copied into the Docker volume pactlocke-ci-vendor (again
# only when Cargo.lock changes), and the checkout is streamed in as a tar.
# Builds are kept in the volume pactlocke-ci-target, so later runs are quick.
#
#   bash scripts/ci-docker.sh            # every step
#   bash scripts/ci-docker.sh --rebuild  # rebuild the image first (new Rust version)
#   bash scripts/ci-docker.sh --clean    # start again from an empty build volume
set -euo pipefail

IMAGE=pactlocke-ci
TARGET_VOLUME=pactlocke-ci-target
VENDOR_VOLUME=pactlocke-ci-vendor
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export MSYS_NO_PATHCONV=1

rebuild=false
for arg in "$@"; do
	case "$arg" in
	--rebuild) rebuild=true ;;
	--clean) docker volume rm -f "$TARGET_VOLUME" >/dev/null ;;
	*)
		echo "usage: $0 [--rebuild] [--clean]" >&2
		exit 2
		;;
	esac
done

docker info >/dev/null 2>&1 || {
	echo "Docker isn't running: start Docker Desktop, then run this again." >&2
	exit 1
}

# Paths Docker on Windows understands, from Git Bash too.
host_path() {
	if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else echo "$1"; fi
}

failed=()

echo "=== No game files committed"
found="$(git ls-files | grep -iE '\.(gba|agb|gb|gbc|nds|sav|srm|sgm|ss[0-9]|state|ips|ups|bps|xdelta)$' || true)"
if [ -n "$found" ]; then
	echo "Game files must never be committed:"
	echo "$found"
	failed+=("No game files committed")
else
	echo "--- ok"
fi

echo "=== libmgba pin"
pin="$(grep -oE 'rev-parse HEAD\)" = [0-9a-f]{40}' .github/workflows/ci.yml | grep -oE '[0-9a-f]{40}')"
if [ "$(git -C third_party/mgba rev-parse HEAD)" = "$pin" ]; then
	echo "--- ok"
else
	echo "third_party/mgba isn't at the pinned $pin"
	failed+=("libmgba pin")
fi

echo "=== Test ROMs"
bash scripts/fetch-test-rom.sh >/dev/null
echo "ready"

echo "=== Image $IMAGE"
if $rebuild || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
	docker build -t "$IMAGE" scripts/ci-docker
else
	echo "reusing it (--rebuild to build it again)"
fi

echo "=== Crates"
vendored="$(docker run --rm --network none -v "$VENDOR_VOLUME:/vendor" "$IMAGE" cat /vendor/.cargo-lock 2>/dev/null || true)"
if [ "$vendored" = "$(cat Cargo.lock)" ]; then
	echo "the volume matches Cargo.lock"
else
	echo "vendoring here, then copying into the volume"
	rm -rf target/ci-docker/vendor
	mkdir -p target/ci-docker
	cargo vendor --locked --versioned-dirs --quiet target/ci-docker/vendor >/dev/null
	cp Cargo.lock target/ci-docker/vendor/.cargo-lock
	tar -C target/ci-docker/vendor -cf - . |
		docker run -i --rm --network none -v "$VENDOR_VOLUME:/vendor" "$IMAGE" \
			bash -c 'find /vendor -mindepth 1 -delete && tar -C /vendor -xf -'
	echo "copied"
fi

echo "=== Linux CI in the container"
set +e
tar --exclude=./target --exclude=.git -cf - . |
	docker run -i --rm --name "pactlocke-ci-$$" --network none \
		-v "$(host_path "$root/target/test-roms"):/roms:ro" \
		-v "$VENDOR_VOLUME:/vendor:ro" \
		-v "$TARGET_VOLUME:/work/target" \
		"$IMAGE" bash -c "$(cat scripts/ci-docker/run.sh)"
status=$?
set -e

if [ ${#failed[@]} -gt 0 ]; then
	echo "=== Also FAILED here: ${failed[*]}"
	exit 1
fi
exit $status
