#!/usr/bin/env bash
# Lane law tests: the cross-crate scans every lane runs ONCE after its review
# converges (AGENTS.md "Lane workflow"). A step that starts zero tests fails.
# Usage: bash extractor/native/scripts/lane_law_tests.sh [--collector]
#   --collector  also run the collector-wide laws (use when the collector was touched)
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
fail=0
py_step() {  # name, script, args...
  local name="$1"; shift
  if python "$@" >/tmp/law_py.$$ 2>&1; then echo "ok    $name"; else echo "FAIL  $name"; tail -5 /tmp/law_py.$$; fail=1; fi
  rm -f /tmp/law_py.$$
}
cd "$ROOT" || exit 2
py_step "citation integrity"        extractor/native/scripts/verify_citation_integrity.py
py_step "world-fold citations"      extractor/native/scripts/restamp_world_fold_citations.py --check
py_step "world-fold authority"       extractor/native/scripts/verify_world_fold_citations.py --repo-root .
if [ $fail -eq 0 ]; then echo "LAW LIST: all green"; else echo "LAW LIST: FAILED"; fi
exit $fail
