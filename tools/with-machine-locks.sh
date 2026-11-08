#!/usr/bin/env bash
# Runs a command on the self-hosted runner the way agents share this machine: after any game run
# holding BATTLE_PLAN_GAME_LOCK (one older than 90 minutes is stale), inside one slot of the shared prep
# lock that BATTLE_PLAN_PREP_LOCK takes. The runner's .env names both.
# Usage: with-machine-locks.sh <tag> <command> [args...]
set -euo pipefail
tag=${1:?tag}; shift
: "${BATTLE_PLAN_GAME_LOCK:?the runner's .env must name BATTLE_PLAN_GAME_LOCK}"
: "${BATTLE_PLAN_PREP_LOCK:?the runner's .env must name BATTLE_PLAN_PREP_LOCK}"
while [[ -d "$BATTLE_PLAN_GAME_LOCK" && -z "$(find "$BATTLE_PLAN_GAME_LOCK" -maxdepth 0 -mmin +90)" ]]; do
  echo "Waiting: a game run holds the game lock."
  sleep 60
done
exec bash "$BATTLE_PLAN_PREP_LOCK" "$tag" "$@"
