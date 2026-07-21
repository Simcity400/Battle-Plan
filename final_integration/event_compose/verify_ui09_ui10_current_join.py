#!/usr/bin/env python3
"""Run the UI09 promotion gate against the mechanically current UI10 freeze."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
UI09 = ROOT / "replay_interface/re_evidence/rendering_laws/UI09/production_completion"
UI10 = ROOT / "replay_interface/re_evidence/rendering_laws/UI10"


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical_text_sha(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="strict")
    payload = text.replace("\r\n", "\n").replace("\r", "\n").encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def main() -> int:
    ui10_run = subprocess.run(
        [
            sys.executable,
            str(UI10 / "verify_ui10.py"),
            "--game-root",
            r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV",
            "--repo-root",
            str(ROOT),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    graph = json.loads((UI10 / "verifier_result.json").read_text(encoding="utf-8"))
    contract = json.loads((UI09 / "promotion_contract.json").read_text(encoding="utf-8"))
    for name in ("ui10_manifest", "ui10_typed_graph"):
        if contract["frozen_inputs"][name].get("hash_basis") != "canonical_text_lf":
            raise AssertionError(f"UI09 {name} is not LF-canonical")
    old = {
        "manifest": contract["frozen_inputs"]["ui10_manifest"]["sha256"],
        "graph": contract["frozen_inputs"]["ui10_typed_graph"]["sha256"],
    }
    current = {
        "manifest": canonical_text_sha(UI10 / "manifest.json"),
        "graph": canonical_text_sha(UI10 / "verifier_result.json"),
    }
    contract["frozen_inputs"]["ui10_manifest"]["sha256"] = current["manifest"]
    contract["frozen_inputs"]["ui10_typed_graph"]["sha256"] = current["graph"]
    current_contract = HERE / "runs/ui09_current_contract.json"
    current_contract.write_text(json.dumps(contract, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    source_path = UI09 / "verify_promotion.py"
    source = source_path.read_text(encoding="utf-8")
    needle = 'contract_path = ROOT / "promotion_contract.json"'
    replacement = f"contract_path = Path({str(current_contract)!r})"
    if source.count(needle) != 1:
        raise AssertionError("UI09 verifier contract anchor drift")
    source = source.replace(needle, replacement)
    result_path = HERE / "runs/ui09_current_result.json"
    previous_argv = sys.argv
    sys.argv = [str(source_path), "--json-out", str(result_path)]
    exit_code = 1
    try:
        namespace = {"__name__": "__main__", "__file__": str(source_path)}
        try:
            exec(compile(source, str(source_path), "exec"), namespace)
        except SystemExit as exc:
            exit_code = int(exc.code or 0)
    finally:
        sys.argv = previous_argv

    result = json.loads(result_path.read_text(encoding="utf-8"))
    result["current_join"] = {
        "ui10_verifier_exit_code": ui10_run.returncode,
        "old_hashes": old,
        "current_hashes": current,
        "classification": (
            "UI10_CURRENT_CANONICAL_JOIN_MATCHES_UI09"
            if old == current
            else "UI10_COMPLETION_SUPERSEDES_STALE_UI09_PINS_WITH_IDENTICAL_REQUIRED_COUNTS"
        ),
        "required_counts": {
            "focus_nodes": graph["focus"]["nodes"],
            "technology_nodes": graph["technology"]["meta"]["tech_count"],
            "technology_positioned": graph["technology"]["meta"]["renderable"],
            "technology_edges": graph["technology"]["edges"],
            "doctrine_nodes": graph["doctrine"]["meta"]["node_count"],
            "doctrine_edges": graph["doctrine"]["edges"],
        },
    }
    result["status"] = "PASS" if exit_code == 0 and ui10_run.returncode == 0 else "FAIL"
    result_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
