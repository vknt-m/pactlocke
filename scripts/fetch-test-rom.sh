#!/usr/bin/env bash
# Downloads the freely licensed test ROMs the determinism check runs on:
# jsmolka's gba-tests arm.gba and thumb.gba (MIT licence), pinned by commit and
# SHA-256. They go into target/test-roms/, which git ignores. No ROM is ever
# committed.
set -euo pipefail

COMMIT=a7113b67e63f83a9b321696ddd7042ccfad6c881
ROMS=(
	"arm/arm.gba 77ee88662552bdc885c1080c0172ff119d54db791bd73b21808cf1ff1fe5b40e"
	"thumb/thumb.gba b5cb2291df4ab314b31c598acd9bff2ccfa0b38efff29daadfe97422ce369b67"
)

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="${root}/target/test-roms"
mkdir -p "$dir"

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d' ' -f1
	else
		shasum -a 256 "$1" | cut -d' ' -f1
	fi
}

for entry in "${ROMS[@]}"; do
	read -r path expected <<<"$entry"
	out="${dir}/$(basename "$path")"
	if [ ! -f "$out" ]; then
		curl --fail --silent --show-error --location \
			"https://raw.githubusercontent.com/jsmolka/gba-tests/${COMMIT}/${path}" --output "$out.part"
		mv "$out.part" "$out"
	fi
	actual="$(sha256 "$out")"
	if [ "$actual" != "$expected" ]; then
		rm -f "$out"
		echo "$(basename "$path") has SHA-256 $actual, expected $expected" >&2
		exit 1
	fi
	echo "$out"
done
