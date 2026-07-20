#!/usr/bin/env python3
"""Run the A18 domain-67 transition miner over every retained stream."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from mine_a18_dynamic_modifier_stream import mine


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    root = args.root.resolve()
    paths = sorted(root.rglob("collector_stream.bin"))
    results = []
    for index, path in enumerate(paths, 1):
        rel = path.relative_to(root).as_posix()
        print(f"[{index}/{len(paths)}] {rel}", flush=True)
        try:
            report = mine(path)
            report["source"]["relative_path"] = rel
            # The corpus report needs transitions and identities, not a copy of
            # every stable final pair from every short run.
            report.pop("right_censored_final_pairs", None)
            results.append({"status": "parsed", "report": report})
        except Exception as exc:
            results.append({
                "status": "error",
                "relative_path": rel,
                "error": f"{type(exc).__name__}: {exc}",
            })

    parsed = [item["report"] for item in results if item["status"] == "parsed"]
    report = {
        "schema": "hoi4.a18.domain67_retained_corpus_mine.v1",
        "authority": "per_stream_domain67_v1_id_set_observation_only",
        "summary": {
            "stream_files": len(paths),
            "parsed": len(parsed),
            "errors": sum(item["status"] == "error" for item in results),
            "streams_with_domain67": sum(
                item["framing"]["domain67_data_records"] > 0 for item in parsed
            ),
            "streams_with_admitted_full_replace": sum(
                item["framing"]["admitted_full_replace_records"] > 0
                for item in parsed
            ),
            "streams_partial_only": sum(
                item["framing"]["partial_status_records"] > 0
                and item["framing"]["admitted_full_replace_records"] == 0
                for item in parsed
            ),
            "domain67_records": sum(
                item["framing"]["domain67_data_records"] for item in parsed
            ),
            "admitted_full_replace_records": sum(
                item["framing"]["admitted_full_replace_records"] for item in parsed
            ),
            "partial_status_records": sum(
                item["framing"]["partial_status_records"] for item in parsed
            ),
            "additions": sum(item["observations"]["additions"] for item in parsed),
            "removals": sum(item["observations"]["removals"] for item in parsed),
            "uncensored_closed_lifetimes": sum(
                item["observations"]["uncensored_closed_lifetimes"] for item in parsed
            ),
        },
        "streams": results,
        "prohibited_inference": [
            "different stream files are independent sessions unless separately joined",
            "a domain-67 v1 removal does not prove timed expiry",
            "domain-67 v1 does not carry days, enabled state, values, or portable country tags",
        ],
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report["summary"], sort_keys=True), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
