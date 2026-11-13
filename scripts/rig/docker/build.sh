#!/usr/bin/env bash
# Runs inside the CI image (scripts/ci-docker/Dockerfile): builds the app for
# the rig, offline, from the checkout streamed in on stdin, and puts it in
# /bins/<machine>/pactlocke. /vendor holds the crates; /work/target is kept.
set -euo pipefail
mkdir -p /work
tar -C /work -xf -
cd /work
mkdir -p "${CARGO_HOME}"
cat >"${CARGO_HOME}/config.toml" <<'TOML'
[source.crates-io]
replace-with = "vendored"

[source.vendored]
directory = "/vendor"

[net]
offline = true
TOML
cargo build --release -p pactlocke-app
mkdir -p "/bins/$(uname -m)"
cp target/release/pactlocke "/bins/$(uname -m)/pactlocke"
echo "built for $(uname -m)"
