#!/usr/bin/env bash
# Builds the app for release on this machine and packs it with every licence
# and notice it must ship with, into target/dist:
#   pactlocke-<version>-<platform>.zip      (Windows)
#   pactlocke-<version>-<platform>.tar.gz   (Linux, macOS)
# Needs cargo-about (cargo install cargo-about --locked --features cli) and
# Python 3.
#
# Usage: bash scripts/package.sh <platform>, e.g. windows-x86_64, linux-x86_64,
# macos-arm64 or macos-x86_64.
set -euo pipefail
cd "$(dirname "$0")/.."

platform="${1:?usage: bash scripts/package.sh <platform>, e.g. windows-x86_64}"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
repo="${GITHUB_REPOSITORY:-vknt-m/pactlocke}"
name="pactlocke-$version-$platform"
dist="target/dist"
out="$dist/$name"

test -f third_party/mgba/LICENSE || { echo "mGBA sources missing: git submodule update --init" >&2; exit 1; }
command -v cargo-about >/dev/null || { echo "cargo-about missing: cargo install cargo-about --locked --features cli" >&2; exit 1; }
# Python reads cargo-about's crate list; PYTHON picks one, else python3 or python.
python="${PYTHON:-$(command -v python3 || command -v python || true)}"
"$python" -c "" 2>/dev/null || { echo "Python missing: set PYTHON to a Python 3" >&2; exit 1; }

echo "=== Building Pactlocke $version for $platform"
cargo build --release --locked -p pactlocke-app

rm -rf "$out"
mkdir -p "$out/licences"
case "$platform" in
	windows-*) cp target/release/pactlocke.exe "$out/" ;;
	*) cp target/release/pactlocke "$out/" ;;
esac

echo "=== Licences"
cp LICENSE NOTICE.md "$out/"
cargo about generate --locked --fail -m crates/pactlocke-app/Cargo.toml about.hbs -o "$out/THIRD-PARTY-LICENSES.html"
cargo about generate --locked --format json -m crates/pactlocke-app/Cargo.toml |
	"$python" scripts/crate_notices.py "$out/licences/crates"
cp third_party/mgba/LICENSE "$out/licences/mGBA-MPL-2.0.txt"
cp third_party/mgba/src/third-party/inih/LICENSE.txt "$out/licences/inih-BSD-3-Clause.txt"
mgba_commit="$(git -C third_party/mgba rev-parse HEAD)"
cat > "$out/SOURCE.txt" <<EOF
Pactlocke $version

Pactlocke is free software under the GNU General Public License, version 3
or later (LICENSE). NOTICE.md and THIRD-PARTY-LICENSES.html list what it is
built with, under their own licences; the licences folder holds the rest.

The complete source of this release, including the mGBA source it is built
from, is pactlocke-$version-source.tar.gz on its release page:
  https://github.com/$repo/releases/tag/v$version
and the tagged repository:
  https://github.com/$repo/tree/v$version

mGBA (Mozilla Public License 2.0) is built unmodified from commit
$mgba_commit of https://github.com/mgba-emu/mgba
EOF

echo "=== Packing"
case "$platform" in
	windows-*)
		rm -f "$dist/$name.zip"
		powershell -NoProfile -Command "Compress-Archive -Path '$out' -DestinationPath '$dist/$name.zip'"
		echo "$dist/$name.zip"
		;;
	*)
		tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"
		echo "$dist/$name.tar.gz"
		;;
esac
