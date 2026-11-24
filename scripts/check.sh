#!/usr/bin/env bash
# What CI's build job runs, in the same order, on this machine: nothing
# private tracked (scripts/check-private.sh), formatting,
# clippy and the docs with warnings as errors, every crate's licence accepted
# by about.toml, then every test. Run it before every commit of a change; the
# git hooks in .githooks run the quick parts of it.
#
#   bash scripts/check.sh          # formatting, clippy, docs, licences, tests
#   bash scripts/check.sh --quick  # formatting and clippy only
#
# For CI's Linux job (time zones, the app build, an xvfb smoke launch), run
# bash scripts/ci-docker.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

quick=false
[ "${1:-}" = "--quick" ] && quick=true

echo "=== Nothing private"
bash scripts/check-private.sh
echo "=== Formatting"
cargo fmt --all --check
echo "=== Clippy"
cargo clippy --workspace --all-targets -- -D warnings
if ! $quick; then
	echo "=== Docs"
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
	echo "=== Licences"
	command -v cargo-about >/dev/null || { echo "cargo-about missing: cargo install cargo-about --locked --features cli" >&2; exit 1; }
	cargo about generate --locked --fail -m crates/pactlocke-app/Cargo.toml about.hbs -o /dev/null
	echo "=== Test ROMs"
	bash scripts/fetch-test-rom.sh >/dev/null
	echo "=== Tests"
	cargo test --workspace
fi
echo "=== Checks passed"
