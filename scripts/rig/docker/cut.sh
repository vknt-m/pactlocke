#!/usr/bin/env bash
# The headless app's `cut SECS`: every packet out is dropped for SECS, then
# the scenario's shaping is back. The cut is made before this returns, and a
# failure to make it fails the step; the network comes back in the background.
set -euo pipefail
secs="$1"
if tc qdisc show dev eth0 | grep -q netem; then
    tc qdisc change dev eth0 root netem loss 100%
else
    tc qdisc add dev eth0 root netem loss 100%
fi
echo "cut: the network is off for ${secs}s" >&2
(
    sleep "$secs"
    if [[ -n "${NETEM:-}" ]]; then
        # shellcheck disable=SC2086
        tc qdisc change dev eth0 root netem $NETEM
    else
        tc qdisc del dev eth0 root
    fi
    echo "cut: the network is back" >&2
) &
