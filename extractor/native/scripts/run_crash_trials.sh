#!/usr/bin/env bash
# Injection-crash trials: does instrumentation volume cause the crashes?
#
# Each trial is bound to its own crash record rather than counted. A trial only
# scores as collector-caused when a NEW crash record appears inside its window
# AND its exception address is the null call. The recorded history shows why
# that matters: crashes with a real code address are interleaved with the null
# ones, and a counting instrument scores them identically.
set -u
ROOT=${ROOT:-}
CRASH="C:/Users/Example/OneDrive/Documents/Paradox Interactive/Hearts of Iron IV/crashes"
OUT=$ROOT/extractor/native/runs/crashrate
NULL_CALL="0x0000000000000000"
TRIALS=${1:-5}
# The minimal arm's roster. Override for other floors, e.g. the d20 capture
# question: MINIMAL_DOMAINS=session_header_live,divisions run_crash_trials.sh
MINIMAL_DOMAINS=${MINIMAL_DOMAINS:-session_header_live,fuel_live}
mkdir -p "$OUT"
RECEIPT=$OUT/trials.jsonl
# -OutDir must land where the scorer later reads collector_record.stderr.log.
# It was hardcoded to  while the scorer read $OUT, so any run
# with ROOT overridden graded a directory the harness had never written.
WOUT=$(cygpath -w "$OUT")
touch "$RECEIPT"
PRIOR_ROWS=$(wc -l < "$RECEIPT")

snapshot() { ls -1 "$CRASH" 2>/dev/null | sort; }

for arm in full minimal; do
  for t in $(seq 1 "$TRIALS"); do
    before=$(snapshot)
    start=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    dir="$OUT/${arm}_t${t}"
    rm -rf "$dir"

    extra=()
    [ "$arm" = minimal ] && extra=(-ScheduledDomainOverride "$MINIMAL_DOMAINS")

    # The harness only launches a game under -EvidenceOnlyDriveGame. Without it
    # the default passive path demands one already-running hoi4.exe, finds none,
    # and refuses before doing anything -- which is how six trials once scored
    # invalid with the game never started. -StopExistingGame clears a survivor
    # from a previous trial that died mid-teardown.
    log="$OUT/${arm}_t${t}.harness.log"
    powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass \
        -File "$ROOT/extractor/native/scripts/Invoke-HeadlessRawCapture.ps1" \
        -OutDir "${WOUT}\\${arm}_t${t}" \
        -EvidenceOnlyDriveGame -StopExistingGame \
        -DurationSeconds 60 "${extra[@]}" > "$log" 2>&1
    died=$(grep -oE "game_died=(True|False)" "$log" | tail -1)

    # Bind: which crash records are new since this trial began?
    new=$(comm -13 <(echo "$before") <(snapshot))
    addr=""; rec=""
    for n in $new; do
      a=$(grep -m1 "Unhandled Exception" "$CRASH/$n/exception.txt" 2>/dev/null | sed 's/.*at address //')
      rec="$n"; addr="$a"
      [ "$a" = "$NULL_CALL" ] && break
    done

    # A trial only counts if the recorder actually got past argument parsing
    # and attached: its stderr carries the modset line once injection ran. The
    # 07-26 minimal arm failed this silently — the recorder rejected a
    # comma-joined domain list and exited, and six no-op trials scored "clean".
    #
    # A trial that never injected is not automatically a harness misconfiguration.
    # If the game left a crash record while nothing of ours was yet inside it, that
    # is the control observation this experiment never had: the pre-injection phases
    # (launch, evidence setup, settle) are identical in every arm, so a death there
    # is attributable to the game or to the harness's own client-driving writes,
    # never to the collector. Scoring it as "invalid" discards exactly the datum
    # that decides whether instrumentation is the cause at all.
    if ! grep -q "modset:" "$dir/collector_record.stderr.log" 2>/dev/null; then
      if [ -n "$rec" ]; then outcome=died_before_injection
      else outcome=invalid_trial_no_injection; fi
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
for arm in full minimal; do
  # Tally only the rows this invocation appended. The receipt is cumulative, so
  # grepping the whole file folds every earlier run's rows -- including the
  # invalid ones -- into the arm counts of the current one.
  rows=$(tail -n +$((PRIOR_ROWS + 1)) "$RECEIPT" | grep "\"arm\":\"$arm\"" || true)
  # `grep -c` prints 0 AND exits 1 on no match, so `|| echo 0` would emit a
  # second 0 and corrupt the tally. Swallow the status instead.
  count() { printf '%s\n' "$rows" | grep -c "$1" || true; }
  printf "  %-8s trials %2s   collector_null_call %2s   other_crash %2s   clean %2s   died_pre_inject %2s   invalid %2s\n" \
    "$arm" "$(printf '%s\n' "$rows" | grep -c '"arm"' || true)" \
    "$(count collector_null_call)" "$(count other_crash)" "$(count '"outcome":"clean"')" \
    "$(count died_before_injection)" \
    "$(count invalid_trial_no_injection)"
done
echo "receipts: $RECEIPT"
