#!/usr/bin/env bash
# One Player's container: shapes its network as the scenario says, then runs
# the headless app on its script. The report goes to the shared folder.
set -uo pipefail
if [[ -n "${NETEM:-}" ]]; then
    # shellcheck disable=SC2086
    tc qdisc add dev eth0 root netem $NETEM || { echo "netem couldn't be applied: $NETEM" >&2; exit 4; }
fi
echo "$PLAYER on $(uname -m), TZ=${TZ:-UTC} ($(date +%Z)), netem: ${NETEM:-none}"
mkdir -p "/shared/reports/$PLAYER"
script="/scenarios/$SCENARIO/$(echo "$PLAYER" | tr '[:upper:]' '[:lower:]').txt"
"/bins/$(uname -m)/pactlocke" --headless "$script" --report "/shared/reports/$PLAYER/report.json" --data /data
code=$?
cp /data/pactlocke.log "/shared/reports/$PLAYER/" 2>/dev/null
exit $code
