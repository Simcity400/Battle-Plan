#!/usr/bin/env bash
# Lane law tests: cross-crate scans for code changes.
# See AGENT_OPERATIONS.md for validation and landing procedures.
# Usage: bash extractor/native/scripts/lane_law_tests.sh
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
if [ $fail -eq 0 ]; then echo "LAW LIST: all green"; else echo "LAW LIST: FAILED"; fi
exit $fail
