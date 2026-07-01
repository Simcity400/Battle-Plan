"""Build a domain-48 damaged-building fixture from a live probe JSON.

The output is consumed by construction_buildings_state_damage_fixture_gate.
This helper only builds the older damaged-building fixture; target-state root
proof now lives in target_state_bounded_root_search.py and
target_location_root_inspect.py.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


SCHEMA = "construction_buildings_state_damage_live_fixture_contract_v1"
BUILD_PROFILE = "870ebf"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--live-probe", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--max-damaged", type=int, default=8)
    args = parser.parse_args()

    live_probe_path = Path(args.live_probe)
    with live_probe_path.open(encoding="utf-8") as handle:
        live_probe = json.load(handle)

    fixture = build_fixture(live_probe, live_probe_path, max_damaged=args.max_damaged)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w", encoding="utf-8") as handle:
        json.dump(fixture, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(
        json.dumps(
            {
                "out": str(out_path),
                "damaged_rows": len(
                    [
                        row
                        for row in fixture["damaged_building_observations"]
                        if row["expected_damaged"]
                    ]
                ),
                "resource_rows": len(fixture["resource_observations"]),
            },
            sort_keys=True,
        )
    )
    return 0


def build_fixture(live_probe: dict[str, Any], live_probe_path: Path, *, max_damaged: int) -> dict[str, Any]:
    if live_probe.get("schema") != "construction_buildings_state_damage_live_probe_870ebf_v1":
        raise SystemExit(f"unexpected live probe schema {live_probe.get('schema')!r}")
    if live_probe.get("build_profile") != BUILD_PROFILE:
        raise SystemExit(f"unexpected build profile {live_probe.get('build_profile')!r}")

    building_health = live_probe.get("building_health") or {}
    damaged = building_health.get("damaged_examples") or []
    undamaged = building_health.get("undamaged_examples") or []
    if not damaged:
        raise SystemExit("live probe did not contain damaged building examples")

    state_resources = live_probe.get("state_resources") or {}
    resource_samples = state_resources.get("samples") or []
    if not resource_samples:
        raise SystemExit("live probe did not contain state resource samples")

    building_rows = []
    for row in damaged[:max_damaged]:
        building_rows.append(building_observation(row, expected_damaged=True, live_probe_path=live_probe_path))
    if undamaged:
        building_rows.append(
            building_observation(undamaged[0], expected_damaged=False, live_probe_path=live_probe_path)
        )

    return {
        "schema": SCHEMA,
        "build_profile": BUILD_PROFILE,
        "source_live_probe": str(live_probe_path).replace("\\", "/"),
        "game_hour_raw": 0,
        "observed_country_roots": int(
            live_probe.get("country_roots", {}).get("root_count") or 0
        ),
        "observed_state_roots": int(
            live_probe.get("topology", {}).get("state_count") or building_health.get("state_vectors_scanned") or 0
        ),
        "observed_province_roots": int(
            live_probe.get("topology", {}).get("province_count") or building_health.get("province_vectors_scanned") or 0
        ),
        "requires": {
            "construction_target_state": False,
            "damaged_building": True,
        },
        "damaged_building_observations": building_rows,
        "resource_observations": [
            {
                "state_id": int(resource_samples[0]["state_id"]),
                "resource_id": int(resource_samples[0]["resource_id"]),
                "amount_raw": int(resource_samples[0]["amount_raw"]),
                "source": "live_probe_870ebf state_def resource table sample",
            }
        ],
        "evidence_summary": {
            "building_entries": building_health.get("building_entries"),
            "healthy_readable": building_health.get("healthy_readable"),
            "healthy_equals_level": building_health.get("healthy_equals_level"),
            "healthy_less_than_level": building_health.get("healthy_less_than_level"),
            "healthy_greater_than_level": building_health.get("healthy_greater_than_level"),
            "state_resource_rows": state_resources.get("resource_rows"),
            "state_resource_verdict": state_resources.get("verdict"),
            "target_state_status": "not_part_of_this_fixture; remains same-frame save/UI oracle blocked",
        },
    }


def building_observation(
    row: dict[str, Any],
    *,
    expected_damaged: bool,
    live_probe_path: Path,
) -> dict[str, Any]:
    return {
        "scope": str(row["scope"]),
        "location_id": int(row["location_id"]),
        "state_id": int(row["state_id"]),
        "type_id": int(row["type_id"]),
        "level_at_0x40": int(row["level_at_0x40"]),
        "healthy_level_at_0x42": int(row["healthy_level_at_0x42"]),
        "expected_damaged": expected_damaged,
        "source": f"live_probe_870ebf current-frame sample {live_probe_path}",
    }


if __name__ == "__main__":
    raise SystemExit(main())
