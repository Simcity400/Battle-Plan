#!/usr/bin/env bash
set -euo pipefail
tag=${1:?tag}; shift
: "${BATTLE_PLAN_GAME_LOCK:?the runner's .env must name BATTLE_PLAN_GAME_LOCK}"
: "${BATTLE_PLAN_PREP_LOCK:?the runner's .env must name BATTLE_PLAN_PREP_LOCK}"
while [[ -d "$BATTLE_PLAN_GAME_LOCK" && -z "$(find "$BATTLE_PLAN_GAME_LOCK" -maxdepth 0 -mmin +90)" ]]; do
  echo "Waiting: a game run holds the game lock."
  sleep 60
done
exec bash "$BATTLE_PLAN_PREP_LOCK" "$tag" "$@"
