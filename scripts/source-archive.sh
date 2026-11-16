#!/usr/bin/env bash
# Packs the complete source of this checkout, with the mGBA submodule that
# GitHub's own source downloads leave out, as
# target/dist/pactlocke-<version>-source.tar.gz. The GPL asks that a release
# comes with all of the source it was built from.
set -euo pipefail
cd "$(dirname "$0")/.."

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
name="pactlocke-$version-source"
dist="target/dist"
test -f third_party/mgba/LICENSE || { echo "mGBA sources missing: git submodule update --init" >&2; exit 1; }

mkdir -p "$dist"
git ls-files --recurse-submodules -z |
	tar --null -T - --transform "s,^,$name/," -czf "$dist/$name.tar.gz"
echo "$dist/$name.tar.gz"
