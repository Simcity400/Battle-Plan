#!/usr/bin/env bash
# Lane law tests: the cross-crate scans every lane runs ONCE after its review
# converges (AGENTS.md "Lane workflow"). A step that starts zero tests fails.
# Usage: bash extractor/native/scripts/lane_law_tests.sh [--collector]
#   --collector  also run the collector-wide laws (use when the collector was touched)
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
EXTRACTOR="$ROOT/extractor/native/Cargo.toml"
REPLAY="$ROOT/replay_interface/native/Cargo.toml"
COLLECTOR=0; [ "${1:-}" = "--collector" ] && COLLECTOR=1
fail=0
nextest_step() {  # name, manifest, args...
  local name="$1" manifest="$2"; shift 2
  local out; out=$(cargo nextest run --manifest-path "$manifest" "$@" 2>&1); local rc=$?
  local started; started=$(printf '%s\n' "$out" | grep -oE 'Starting [0-9]+ tests?' | grep -oE '[0-9]+' | head -1)
  if [ $rc -ne 0 ]; then echo "FAIL  $name (exit $rc)"; printf '%s\n' "$out" | grep -E 'FAIL \[|panicked|error' | head -20; fail=1
  elif [ -z "$started" ] || [ "$started" = "0" ]; then echo "FAIL  $name (started zero tests: false green)"; fail=1
  else echo "ok    $name ($started tests)"; fi
}
py_step() {  # name, script, args...
  local name="$1"; shift
  if python "$@" >/tmp/law_py.$$ 2>&1; then echo "ok    $name"; else echo "FAIL  $name"; tail -5 /tmp/law_py.$$; fail=1; fi
  rm -f /tmp/law_py.$$
}
cd "$ROOT" || exit 2
nextest_step "game_file_ledger"          "$REPLAY"    -p replay_app -E 'test(game_file_ledger)'
nextest_step "xui09 producer half"       "$EXTRACTOR" -p collector -E 'test(xui09_joint_certification)'
nextest_step "xui09 consumer half"       "$REPLAY"    -p game_ui -E 'test(joint_certification)'
nextest_step "counter_wire_census"       "$EXTRACTOR" -p compact_contract --test compact_contract_suite -E 'test(counter_wire_census)'
nextest_step "country-reference decode"  "$REPLAY"    -p replay_core -E 'test(every_country_reference_decode_is_gated_witnessed_or_named)'
if [ $COLLECTOR -eq 1 ]; then
  nextest_step "collector laws: attach_log reset" "$EXTRACTOR" -p collector -E 'test(domain_scoped_reset_tests)'
  nextest_step "collector laws: inline_detour"    "$EXTRACTOR" -p inline_detour -E 'test(remove_quiescence_sweep) or test(census)'
  nextest_step "collector laws: d51 read window"  "$EXTRACTOR" -p collector -E 'test(d51_read_window_tests)'
fi
py_step "citation integrity"        extractor/native/scripts/verify_citation_integrity.py
py_step "world-fold citations"      extractor/native/scripts/restamp_world_fold_citations.py --check
if [ $fail -eq 0 ]; then echo "LAW LIST: all green"; else echo "LAW LIST: FAILED"; fi
exit $fail
