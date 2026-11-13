#!/usr/bin/env bash
# Runs inside the pactlocke-ci container: the build steps of CI's Linux job,
# in order, offline. The checkout arrives as a tar on stdin; /vendor (a
# volume) holds every crate the lockfile needs; /roms the test ROMs;
# /work/target is a volume kept between runs.
set -uo pipefail

failed=()
step() {
	local name="$1"
	shift
	echo
	echo "=== ${name}"
	local started=$SECONDS
	if "$@"; then
		echo "--- ${name}: ok ($((SECONDS - started)) s)"
	else
		echo "--- ${name}: FAILED ($((SECONDS - started)) s)"
		failed+=("$name")
	fi
}

mkdir -p /work/target/test-roms
tar -C /work -xf -
cp /roms/*.gba /work/target/test-roms/
cd /work

# Crates come from /vendor, never the network.
mkdir -p "${CARGO_HOME}"
cat >"${CARGO_HOME}/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored"

[source.vendored]
directory = "/vendor"

[net]
offline = true
EOF

rustc --version
cargo clippy --version

time_zones() {
	TZ=Pacific/Kiritimati cargo test -p pactlocke-mgba && TZ=America/Los_Angeles cargo test -p pactlocke-mgba
}

step "Formatting" cargo fmt --all --check
step "Clippy" cargo clippy --workspace --all-targets -- -D warnings
step "Tests" cargo test --workspace
step "Core tests in other time zones" time_zones
step "Build the app" cargo build -p pactlocke-app
step "Smoke launch with no ROM" xvfb-run -a target/debug/pactlocke --smoke

echo
if [ ${#failed[@]} -eq 0 ]; then
	echo "=== Linux CI: every step passed"
else
	echo "=== Linux CI: FAILED: ${failed[*]}"
	exit 1
fi
