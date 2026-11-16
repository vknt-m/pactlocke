#!/usr/bin/env bash
# What CI's build job runs, in the same order, on this machine: formatting,
# clippy and the docs with warnings as errors, then every test. Run it before every commit
# of a change; the git hooks in .githooks run the quick parts of it.
#
#   bash scripts/check.sh          # formatting, clippy, docs, tests
#   bash scripts/check.sh --quick  # formatting and clippy only
#
# For CI's Linux job (time zones, the app build, an xvfb smoke launch), run
# bash scripts/ci-docker.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

quick=false
[ "${1:-}" = "--quick" ] && quick=true

echo "=== Formatting"
cargo fmt --all --check
echo "=== Clippy"
cargo clippy --workspace --all-targets -- -D warnings
if ! $quick; then
	echo "=== Docs"
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
	echo "=== Test ROMs"
	bash scripts/fetch-test-rom.sh >/dev/null
	echo "=== Tests"
	cargo test --workspace
fi
echo "=== Checks passed"
