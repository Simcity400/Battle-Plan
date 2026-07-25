#!/usr/bin/env bash
# Injection-crash trials: does instrumentation volume cause the crashes?
#
# Each trial is bound to its own crash record rather than counted. A trial only
# scores as collector-caused when a NEW crash record appears inside its window
# AND its exception address is the null call. The recorded history shows why
# that matters: crashes with a real code address are interleaved with the null
# ones, and a counting instrument scores them identically.
set -u
ROOT=
CRASH="C:/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/crashes"
OUT=$ROOT/extractor/native/runs/crashrate
NULL_CALL="0x0000000000000000"
TRIALS=${1:-5}
mkdir -p "$OUT"
RECEIPT=$OUT/trials.jsonl

snapshot() { ls -1 "$CRASH" 2>/dev/null | sort; }

for arm in full minimal; do
  for t in $(seq 1 "$TRIALS"); do
    before=$(snapshot)
    start=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    dir="$OUT/${arm}_t${t}"
    rm -rf "$dir"

    extra=()
    [ "$arm" = minimal ] && extra=(-ScheduledDomainOverride session_header_live,fuel_live)

    died=$(powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass \
        -File "$ROOT/extractor/native/scripts/Invoke-HeadlessRawCapture.ps1" \
        -OutDir "extractor\\native\\runs\\crashrate\\${arm}_t${t}" \
        -DurationSeconds 60 "${extra[@]}" 2>&1 | grep -oE "game_died=(True|False)" | tail -1)

    # Bind: which crash records are new since this trial began?
    new=$(comm -13 <(echo "$before") <(snapshot))
    addr=""; rec=""
    for n in $new; do
      a=$(grep -m1 "Unhandled Exception" "$CRASH/$n/exception.txt" 2>/dev/null | sed 's/.*at address //')
      rec="$n"; addr="$a"
      [ "$a" = "$NULL_CALL" ] && break
    done

    if [ -z "$rec" ]; then outcome=clean
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
for arm in full minimal; do
  # `grep -c` prints 0 AND exits 1 on no match, so `|| echo 0` would emit a
  # second 0 and corrupt the tally. Swallow the status instead.
  rows=$(grep "\"arm\":\"$arm\"" "$RECEIPT" 2>/dev/null || true)
  count() { printf '%s\n' "$rows" | grep -c "$1" || true; }
  printf "  %-8s trials %2s   collector_null_call %2s   other_crash %2s   clean %2s\n" \
    "$arm" "$(printf '%s\n' "$rows" | grep -c '"arm"' || true)" \
    "$(count collector_null_call)" "$(count other_crash)" "$(count '"outcome":"clean"')"
done
echo "receipts: $RECEIPT"
