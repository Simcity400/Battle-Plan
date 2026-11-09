#!/usr/bin/env bash
set -eu
export PATH="/usr/bin:/bin:$PATH"
. "$1"
export PRIORITY=$4
take_lock "$2" "$3"
printf '%s' "$3" > "$2/owner"
