"""Copies the notice files of the crates built into the app into a release.

Reads `cargo about generate --format json` on stdin and copies, for each crate
it lists, any NOTICE, COPYRIGHT or AUTHORS file at the crate's root (Apache-2.0
section 4(d) asks for NOTICE files to ship), and the licence files of the
fonts egui builds in, into <out>/<crate>-<version>/.

Usage: cargo about generate --format json ... | python scripts/crate_notices.py <out>
"""
import json
import shutil
import sys
from pathlib import Path

PREFIXES = ("NOTICE", "COPYRIGHT", "AUTHORS")

out = Path(sys.argv[1])
about = json.loads(sys.stdin.buffer.read().decode("utf-8"))
copied = 0
for entry in about["crates"]:
    package = entry["package"]
    root = Path(package["manifest_path"]).parent
    files = [f for f in root.iterdir() if f.is_file() and f.name.upper().startswith(PREFIXES)]
    if package["name"] == "epaint_default_fonts":
        files += sorted((root / "fonts").glob("*.txt"))
    if not files:
        continue
    target = out / f"{package['name']}-{package['version']}"
    target.mkdir(parents=True, exist_ok=True)
    for f in files:
        shutil.copyfile(f, target / f.name)
        copied += 1
print(f"{copied} notice files from {len(list(out.iterdir())) if out.exists() else 0} crates")
