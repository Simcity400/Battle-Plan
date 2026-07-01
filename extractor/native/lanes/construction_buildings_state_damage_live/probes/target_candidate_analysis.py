"""Analyze construction target-state candidates from live_probe_870ebf.py output.

This is lane-local evidence processing. It rejects only candidates that are
all-line invariant across a multi-owner/multi-template construction queue, and
keeps all other candidates fixture-blocked until a live/save aligned expected
target-state fixture is available.
"""

from __future__ import annotations

import argparse
import json
import os
from typing import Any


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    with open(args.probe, encoding="utf-8") as handle:
        probe = json.load(handle)

    construction = probe.get("construction", {})
    target = probe.get("target_state_candidates", {})
    line_count = int(target.get("line_count", 0))
    tag_count = int(construction.get("tag_db_id_count", 0))
    template_count = int(construction.get("template_id_count", 0))
    strict_candidates = target.get("strict_all_line_candidates", [])

    rejected: list[dict[str, Any]] = []
    remaining: list[dict[str, Any]] = []
    for candidate in strict_candidates:
        mapped_values = candidate.get("mapped_state_ids", [])
        raw_values = candidate.get("raw_values", [])
        candidate_tags = candidate.get("tag_db_ids", [])
        candidate_templates = candidate.get("template_ids", [])
        all_line = candidate.get("line_count") == line_count and line_count > 0
        invariant = len(mapped_values) == 1 and len(raw_values) == 1
        broad_queue = len(candidate_tags) > 1 or len(candidate_templates) > 1
        if all_line and invariant and broad_queue:
            rejected.append(
                {
                    "base": candidate.get("base"),
                    "offset": candidate.get("offset"),
                    "kind": candidate.get("kind"),
                    "line_count": candidate.get("line_count"),
                    "tag_db_id_count": len(candidate_tags),
                    "template_id_count": len(candidate_templates),
                    "raw_values": raw_values,
                    "mapped_state_ids": mapped_values,
                    "reason": (
                        "rejected_as_per_line_target_state_root: invariant mapped state "
                        "across every live construction line while the queue spans multiple "
                        "owners/templates"
                    ),
                }
            )
        else:
            remaining.append(
                {
                    "base": candidate.get("base"),
                    "offset": candidate.get("offset"),
                    "kind": candidate.get("kind"),
                    "line_count": candidate.get("line_count"),
                    "tag_db_id_count": len(candidate_tags),
                    "template_id_count": len(candidate_templates),
                    "raw_values": raw_values,
                    "mapped_state_ids": mapped_values,
                    "blocker": (
                        "fixture_required: needs live/save-aligned expected target_state_id "
                        "for at least two construction lines that distinguish this offset"
                    ),
                }
            )

    result = {
        "schema": "construction_buildings_state_damage_live_target_candidate_analysis_v1",
        "input_probe": args.probe.replace("\\", "/"),
        "build_profile": probe.get("build_profile"),
        "fingerprint_sha256": probe.get("fingerprint", {}).get("sha256"),
        "construction_queue": {
            "line_count": line_count,
            "tag_db_id_count": tag_count,
            "template_id_count": template_count,
            "tag_db_ids_sample": construction.get("tag_db_ids", [])[:32],
            "template_ids_sample": construction.get("template_ids", [])[:32],
        },
        "target_state_candidate_summary": {
            "probe_verdict": target.get("verdict"),
            "aggregate_candidate_count": target.get("aggregate_candidate_count", 0),
            "strict_all_line_candidate_count": len(strict_candidates),
            "rejected_candidate_count": len(rejected),
            "remaining_candidate_count": len(remaining),
        },
        "rejected_candidates": rejected,
        "remaining_fixture_blocked_candidates": remaining,
        "verdict": (
            "target_state_partially_reduced_candidates_rejected"
            if rejected
            else "target_state_unreduced_fixture_blocked"
        ),
    }

    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(
        json.dumps(
            {
                "out": args.out,
                "rejected_candidate_count": len(rejected),
                "remaining_candidate_count": len(remaining),
                "verdict": result["verdict"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
