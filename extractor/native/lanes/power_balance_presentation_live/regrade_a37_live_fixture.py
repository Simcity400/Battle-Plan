#!/usr/bin/env python3
"""Fail-closed A37 owner-fixture regrader.

This tool does not launch HOI4. It grades two owner-produced JSON receipts and
never changes either Rust activation latch.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


SHA256 = re.compile(r"[0-9a-f]{64}")
CASES = {
    "positive_current_negative_change.json": (1, -1),
    "negative_current_positive_change.json": (-1, 1),
}


def require(value: bool, message: str) -> None:
    if not value:
        raise ValueError(message)


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def lower_sha256(value: object) -> bool:
    return isinstance(value, str) and SHA256.fullmatch(value) is not None and set(value) != {"0"}


def grade_case(path: Path, current_sign: int, change_sign: int) -> dict[str, object]:
    root = json.loads(path.read_text(encoding="utf-8"))
    require(root.get("schema") == "hoi4.a37.raw_controller_selected_present.v1", f"{path.name}: schema")
    require(root.get("status") == "controller_spine_only_d74_unavailable", f"{path.name}: status")
    require(root.get("executable_sha256") == "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad", f"{path.name}: executable")
    require(root.get("controller_manifest_sha256") == "a540af1c030d9ef2fbb1cd290627087627771a196e7a60e4241cb5287b66e4e2", f"{path.name}: controller manifest")
    require(root.get("panel_visibility_manifest_sha256") == "2c921216fb9b825716685852c026aba92b7da88a5c88fdb625a569e49afedd9c", f"{path.name}: panel manifest")

    hook = root.get("hook_observation")
    selected = root.get("selected_present")
    require(isinstance(hook, dict) and isinstance(selected, dict), f"{path.name}: missing hook/Present join")

    current = hook.get("current_value_raw")
    change = hook.get("change_raw")
    require(isinstance(current, int) and current != 0 and (1 if current > 0 else -1) == current_sign, f"{path.name}: current sign")
    require(isinstance(change, int) and change != 0 and (1 if change > 0 else -1) == change_sign, f"{path.name}: change sign")
    require(hook.get("panel_open") is True and hook.get("gui_visible") is True, f"{path.name}: open/visible join")
    require(hook.get("selected_record_present") is True, f"{path.name}: selected record")
    require(hook.get("selected_range_present") is True, f"{path.name}: selected range")

    materialized = hook.get("materialization_qpc_ticks")
    present_qpc = selected.get("selected_qpc_ticks")
    cutoff_qpc = selected.get("cutoff_qpc_ticks")
    require(all(isinstance(value, int) and value > 0 for value in (materialized, present_qpc, cutoff_qpc)), f"{path.name}: QPC")
    require(materialized <= present_qpc <= cutoff_qpc, f"{path.name}: QPC order")
    receipt_sha = selected.get("receipt_sha256")
    require(lower_sha256(receipt_sha), f"{path.name}: selected Present receipt")
    require(hook.get("qpc_frequency_hz") == selected.get("qpc_frequency_hz"), f"{path.name}: QPC frequency")
    required_unavailable = {
        "source_record_id", "country_tag", "balance_id", "active_side_id",
        "trending_side_id", "active_range_id", "effective_icons",
        "range_indicator_frame_and_geometry", "resolved_text", "decisions",
    }
    unavailable = root.get("unavailable_d74_fields")
    require(isinstance(unavailable, list) and required_unavailable.issubset(set(unavailable)), f"{path.name}: unavailable d74 fields")
    require(root.get("pbp2_emitted") is False, f"{path.name}: PBP2 must remain unavailable")
    require(root.get("activation_authorized") is False, f"{path.name}: activation")
    return {
        "case": path.name,
        "source_sha256": sha256_file(path),
        "selected_present_receipt_sha256": receipt_sha,
        "status": "pass_controller_spine_only_d74_unavailable",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("fixture_dir", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    rows = [
        grade_case(args.fixture_dir / name, *expected)
        for name, expected in CASES.items()
    ]
    require(len({row["selected_present_receipt_sha256"] for row in rows}) == 2, "cases reused one selected Present")
    receipt = {
        "schema": "hoi4.a37.live_fixture_regrade.v1",
        "status": "pass_static_regrade_activation_unchanged",
        "activation_authorized": False,
        "cases": rows,
    }
    encoded = (json.dumps(receipt, indent=2, sort_keys=True) + "\n").encode()
    if args.output:
        args.output.write_bytes(encoded)
    else:
        print(encoded.decode(), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
