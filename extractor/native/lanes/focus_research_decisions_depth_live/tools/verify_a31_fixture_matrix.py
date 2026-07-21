#!/usr/bin/env python3
"""Fail-closed A31 stable/transition fixture-pair regrader.

This consumes two independently authenticated receipts. It never changes the
domain-89 scheduling or live-fixture latches; a successful result is the
human-supplied evidence prerequisite for a later reviewed activation patch.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from verify_a31_acquisition_receipt import parse_nfp1, require, verify


def load_detail(receipt_path: Path) -> tuple[dict[str, object], dict[str, object], dict[str, object]]:
    public = verify(receipt_path)
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    record = receipt.get("nfp1")
    require(isinstance(record, dict), "receipt NFP1 descriptor missing")
    record_path = (receipt_path.parent / str(record.get("path"))).resolve()
    detail = parse_nfp1(record_path.read_bytes())
    return public, receipt, detail


def item_map(detail: dict[str, object]) -> dict[str, dict[str, object]]:
    items = detail.get("items")
    require(isinstance(items, list), "decoded NFP1 item list missing")
    result: dict[str, dict[str, object]] = {}
    for item in items:
        require(isinstance(item, dict), "decoded NFP1 item is not an object")
        focus_id = item.get("focus_id")
        require(isinstance(focus_id, str) and focus_id not in result, "decoded focus id is invalid")
        result[focus_id] = item
    return result


def verify_matrix(stable_path: Path, transition_path: Path) -> dict[str, object]:
    stable_public, stable_receipt, stable = load_detail(stable_path)
    transition_public, transition_receipt, transition = load_detail(transition_path)
    require(stable_path != transition_path, "stable and transition receipts are the same path")
    require(
        stable_public["receipt_body_sha256"] != transition_public["receipt_body_sha256"],
        "stable and transition receipt bodies are identical",
    )
    require(stable["country_tag"] == transition["country_tag"], "fixture country changed")
    require(
        stable_receipt.get("same_epoch_vector_publish_seen") is True,
        "stable fixture did not observe the vector publication epoch",
    )
    require(
        int(transition["frame_hour"]) > int(stable["frame_hour"]),
        "transition frame hour does not follow stable frame hour",
    )
    require(
        stable_receipt.get("selected_present_receipt_sha256")
        != transition_receipt.get("selected_present_receipt_sha256"),
        "fixture pair reuses one selected-Present receipt",
    )

    stable_items = item_map(stable)
    transition_items = item_map(transition)
    common = sorted(stable_items.keys() & transition_items.keys())
    require(common, "fixture pair has no common stable focus identity")
    leaf_names = (
        "presentation_state_code",
        "display_progress_percent",
        "presentation_flags",
        "joint_viewing_country_tag",
    )
    changes: list[dict[str, object]] = []
    for focus_id in common:
        before = stable_items[focus_id]
        after = transition_items[focus_id]
        changed = [name for name in leaf_names if before.get(name) != after.get(name)]
        if changed:
            changes.append({
                "focus_id": focus_id,
                "changed_leaves": changed,
                "before": {name: before.get(name) for name in changed},
                "after": {name: after.get(name) for name in changed},
            })
    require(
        changes,
        "fixture pair has no discriminating final item-state or transient presentation-leaf transition",
    )
    return {
        "status": "PASS_STABLE_AND_DISCRIMINATING_TRANSITION",
        "country_tag": stable["country_tag"],
        "stable_frame_hour": stable["frame_hour"],
        "transition_frame_hour": transition["frame_hour"],
        "stable_receipt_body_sha256": stable_public["receipt_body_sha256"],
        "transition_receipt_body_sha256": transition_public["receipt_body_sha256"],
        "common_focus_count": len(common),
        "changed_focus_count": len(changes),
        "changes": changes,
        "production_activation_changed": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("stable_receipt", type=Path)
    parser.add_argument("transition_receipt", type=Path)
    args = parser.parse_args()
    try:
        result = verify_matrix(args.stable_receipt.resolve(), args.transition_receipt.resolve())
    except Exception as error:  # fail-closed CLI boundary
        print(json.dumps({"status": "FAIL", "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
