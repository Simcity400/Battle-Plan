#!/usr/bin/env python3
"""Current T01 overlay gate for an append-only capture-attempt ledger."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
T01 = ROOT / "replay_interface/re_evidence/rendering_laws/T01"
EXPECTED_PREFIX = "1eb104e15f2b9be7a59ca50ab97834344aca0179e9399df630f923f546150c68"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    comp = subprocess.run(
        [sys.executable, str(T01 / "verify_t01_composition.py")],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    composition = json.loads((T01 / "consecutive_owner_composition.json").read_text(encoding="utf-8"))
    lines = (T01 / "capture_attempts.jsonl").read_bytes().splitlines(keepends=True)
    latest = json.loads(lines[-1])
    checks = {
        "composition_rerun_pass": comp.returncode == 0 and composition["status"] == "PASS",
        "frozen_ledger_prefix_exact": sha(b"".join(lines[:2])) == EXPECTED_PREFIX,
        "ledger_has_one_append": len(lines) == 3,
        "append_is_synchronized_transition": latest.get("status") == "SYNCHRONIZED_TRANSITION_CAPTURED",
        "append_preserves_proven_result": "already-PROVEN T01 composition result" in latest.get("detail", ""),
        "append_has_correlation": bool(latest.get("correlation_id")),
        "append_has_rdc_hash": len(latest.get("rdc_sha256", "")) == 64,
        "full_field_masks_exact": composition["aggregate"]["mismatches"] == 0,
    }
    result = {
        "schema": "hoi4-rendering-law-t01-current-compose-overlay-v1",
        "status": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks,
        "current_ledger_sha256": sha(b"".join(lines)),
        "frozen_prefix_sha256": EXPECTED_PREFIX,
        "classification": "APPEND_ONLY_LEDGER_DRIFT_NOT_LAW_DRIFT",
    }
    (HERE / "runs/t01_current_result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
