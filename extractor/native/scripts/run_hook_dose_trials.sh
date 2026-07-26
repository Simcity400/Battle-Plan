#!/usr/bin/env bash
# Hook-volume dose response: is the injection crash caused by the NUMBER of
# instrumented domains, or by one specific domain among them?
#
# The 2026-07-26 arms (79 domains 6/6 crash, 2 domains 8/8 clean) cannot tell
# those apart. This walks prefixes of the production roster: a smooth rise in
# crash rate with domain count is a volume effect; a sharp jump when one
# particular domain enters names that domain.
#
# Prefixes are taken from the roster's own stable order, so every arm contains
# `divisions` (#10) once the prefix reaches it and each arm is a superset of
# the smaller ones — that is what makes the comparison a dose curve rather than
# 5 unrelated configurations.
#
#   run_hook_dose_trials.sh [trials_per_point] [size ...]
set -u
ROOT=
CRASH="C:/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/crashes"
OUT=$ROOT/extractor/native/runs/hookdose
NULL_CALL="0x0000000000000000"
TRIALS=${1:-3}
shift || true
SIZES=${*:-"5 10 20 40"}
mkdir -p "$OUT"
RECEIPT=$OUT/dose.jsonl

# `tr -d '\r'` is belt-and-braces: the Rust roster binary emits bare LF, but a
# stray CR here reaches the recorder as "fuel_live\r" and is rejected, which is
# exactly the silent no-op the validity gate below exists to catch.
mapfile -t ROSTER < <("$ROOT/.cargo-target/release/domain_roster.exe" \
  --native-root "$ROOT/extractor/native" | tr -d '\r')
echo "roster: ${#ROSTER[@]} domains"

snapshot() { ls -1 "$CRASH" 2>/dev/null | sort; }

# A trial killed mid-run leaves its game alive, and the capture wrapper refuses
# to launch while one is running — which aborts every remaining trial in about
# a second each. Clear the field before each trial so an interrupted run cannot
# silently invalidate the next one.
clear_game() {
  powershell -NoProfile -NonInteractive -Command \
    "Stop-Process -Name the game -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 3" \
    >/dev/null 2>&1
}

for size in $SIZES; do
  # Prefix of `size` domains, comma-joined (the wrapper splits it).
  roster=$(IFS=,; echo "${ROSTER[*]:0:$size}")
  for t in $(seq 1 "$TRIALS"); do
    clear_game
    before=$(snapshot)
    start=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    dir="$OUT/n${size}_t${t}"
    rm -rf "$dir"

    died=$(powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass \
        -File "$ROOT/extractor/native/scripts/Invoke-HeadlessRawCapture.ps1" \
        -OutDir "extractor\\native\\runs\\hookdose\\n${size}_t${t}" \
        -DurationSeconds 45 -ScheduledDomainOverride "$roster" 2>&1 \
        | grep -oE "game_died=(True|False)" | tail -1)

    new=$(comm -13 <(echo "$before") <(snapshot))
    addr=""; rec=""
    for n in $new; do
      a=$(grep -m1 "Unhandled Exception" "$CRASH/$n/exception.txt" 2>/dev/null | sed 's/.*at address //')
      rec="$n"; addr="$a"
      [ "$a" = "$NULL_CALL" ] && break
    done

    # Same validity gate as run_crash_trials.sh: no attach receipt, no score.
    if ! grep -q "modset:" "$dir/collector_record.stderr.log" 2>/dev/null; then
      outcome=invalid_trial_no_injection
    elif [ -z "$rec" ]; then outcome=clean
    elif [ "$addr" = "$NULL_CALL" ]; then outcome=collector_null_call
    elif [ -n "$addr" ]; then outcome=other_crash
    else outcome=crash_without_exception_record; fi

    printf '{"domains":%d,"trial":%d,"start":"%s","end":"%s","game_died":"%s","outcome":"%s","address":"%s","record":"%s","last_domain":"%s"}\n' \
      "$size" "$t" "$start" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${died#game_died=}" \
      "$outcome" "$addr" "$rec" "${ROSTER[$((size-1))]}" >> "$RECEIPT"
    echo "n=$size t$t -> $outcome ${addr:+($addr)}"
  done
done

echo
echo "=== dose curve ==="
for size in $SIZES; do
  rows=$(grep "\"domains\":$size," "$RECEIPT" 2>/dev/null || true)
  count() { printf '%s\n' "$rows" | grep -c "$1" || true; }
  printf "  %3s domains   trials %2s   null_call %2s   other %2s   clean %2s   invalid %2s\n" \
    "$size" "$(printf '%s\n' "$rows" | grep -c '"domains"' || true)" \
    "$(count collector_null_call)" "$(count other_crash)" \
    "$(count '"outcome":"clean"')" "$(count invalid_trial_no_injection)"
done
echo "receipts: $RECEIPT"
