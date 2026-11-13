#!/usr/bin/env bash
# Checks a rig run's reports together: check-reports.sh DIR PLAYER...
set -euo pipefail
dir="$1"
shift
echo "Reports:"
cargo dev rig-check "$dir" "$@"
