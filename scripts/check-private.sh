#!/usr/bin/env bash
# Nothing private in the repository: no file .gitignore marks private (agent
# notes, tickets, research, Run logs), no home-folder path and no ticket
# number in a tracked file. It checks what is staged, so the pre-commit hook
# stops a commit that would add one.
#
#   bash scripts/check-private.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

failed=false
ignored=$(git ls-files --cached --ignored --exclude-standard)
if [ -n "$ignored" ]; then
	echo "Tracked, but .gitignore marks them private (git rm --cached them):" >&2
	echo "$ignored" >&2
	failed=true
fi
# A home folder's path names its user.
paths=$(git grep --cached -nIE '[A-Za-z]:[\\/]+Users[\\/]+[^\\/ ]+|/home/[a-z][^/ ]*/|/Users/[A-Za-z][^/ ]*/' -- ':!scripts/check-private.sh' || true)
if [ -n "$paths" ]; then
	echo "A home-folder path in a tracked file:" >&2
	echo "$paths" >&2
	failed=true
fi
# Tickets stay local, so a ticket number means nothing to a reader here.
tickets=$(git grep --cached -niIE '\btickets? ?#?[0-9]+' -- ':!scripts/check-private.sh' || true)
if [ -n "$tickets" ]; then
	echo "A ticket number in a tracked file:" >&2
	echo "$tickets" >&2
	failed=true
fi
if $failed; then
	exit 1
fi
echo "Nothing private is tracked."
