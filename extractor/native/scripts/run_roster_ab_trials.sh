#!/usr/bin/env bash
# A/B the true production roster against the one the harness was really using.
#
# `production_domain_roster.py` admitted three statuses that
# `compact_contract::status_in_production_profile` deliberately excludes, so
# every "full roster" capture scheduled 76 + 3 domains: 57
# session_multiplayer_metadata_live, 78 design_catalog_context_live, 84
# scripted_gui_runtime_live. Those captures crashed 6/6.
#
# arm prod76  — the contract's own production profile (no override)
# arm extra79 — that profile plus the three the scraper let in
#
# prod76 clean + extra79 crashing names the culprit outright and costs no
# refactor. Both crashing sends the investigation back to hook volume.
set -u
ROOT=
CRASH="C:/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/crashes"
OUT=$ROOT/extractor/native/runs/roster_ab
ROSTER_BIN=$ROOT/.cargo-target/release/domain_roster.exe
NULL_CALL="0x0000000000000000"
EXTRA="session_multiplayer_metadata_live,design_catalog_context_live,scripted_gui_runtime_live"
TRIALS=${1:-4}
mkdir -p "$OUT"
RECEIPT=$OUT/ab.jsonl

mapfile -t ROSTER < <("$ROSTER_BIN" --native-root "$ROOT/extractor/native" | tr -d '\r')
PROD=$(IFS=,; echo "${ROSTER[*]}")
echo "prod76 = ${#ROSTER[@]} domains; extra79 adds 3"

snapshot() { ls -1 "$CRASH" 2>/dev/null | sort; }
clear_game() {
  powershell -NoProfile -NonInteractive -Command \
    "Stop-Process -Name the game -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 3" \
    >/dev/null 2>&1
}

for arm in prod76 extra79; do
  case $arm in
    prod76)  roster="$PROD" ;;
    extra79) roster="$PROD,$EXTRA" ;;
  esac
  for t in $(seq 1 "$TRIALS"); do
    clear_game
    before=$(snapshot)
    start=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    dir="$OUT/${arm}_t${t}"
    rm -rf "$dir"

    died=$(powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass \
        -File "$ROOT/extractor/native/scripts/Invoke-HeadlessRawCapture.ps1" \
        -OutDir "extractor\\native\\runs\\roster_ab\\${arm}_t${t}" \
        -DurationSeconds 45 -ScheduledDomainOverride "$roster" 2>&1 \
        | grep -oE "game_died=(True|False)" | tail -1)

    new=$(comm -13 <(echo "$before") <(snapshot))
    addr=""; rec=""
    for n in $new; do
      a=$(grep -m1 "Unhandled Exception" "$CRASH/$n/exception.txt" 2>/dev/null | sed 's/.*at address //')
      rec="$n"; addr="$a"
      [ "$a" = "$NULL_CALL" ] && break
    done

    if ! grep -q "modset:" "$dir/collector_record.stderr.log" 2>/dev/null; then
      outcome=invalid_trial_no_injection
    elif [ -z "$rec" ]; then outcome=clean
    elif [ "$addr" = "$NULL_CALL" ]; then outcome=collector_null_call
    elif [ -n "$addr" ]; then outcome=other_crash
    else outcome=crash_without_exception_record; fi

    printf '{"arm":"%s","trial":%d,"start":"%s","end":"%s","game_died":"%s","outcome":"%s","address":"%s","record":"%s"}\n' \
      "$arm" "$t" "$start" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${died#game_died=}" \
      "$outcome" "$addr" "$rec" >> "$RECEIPT"
    echo "$arm t$t -> $outcome ${addr:+($addr)}"
  done
done

echo
echo "=== tally ==="
for arm in prod76 extra79; do
  rows=$(grep "\"arm\":\"$arm\"" "$RECEIPT" 2>/dev/null || true)
  count() { printf '%s\n' "$rows" | grep -c "$1" || true; }
  printf "  %-8s trials %2s   null_call %2s   other %2s   clean %2s   invalid %2s\n" \
    "$arm" "$(printf '%s\n' "$rows" | grep -c '"arm"' || true)" \
    "$(count collector_null_call)" "$(count other_crash)" \
    "$(count '"outcome":"clean"')" "$(count invalid_trial_no_injection)"
done
echo "receipts: $RECEIPT"
