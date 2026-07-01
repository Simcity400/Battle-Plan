"""Compare native scheduled domain-47 cost with observed live game-hour cadence."""

from __future__ import annotations

import json
from pathlib import Path


LANE = Path(__file__).resolve().parent
REPO = LANE.parents[3]
BENCHMARK = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1"
    / "scheduled_adapter_benchmark.json"
)
CADENCE = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_live_cadence_probe_v2"
    / "live_cadence_probe.json"
)
OUT_DIR = (
    REPO
    / "extractor/native"
    / "runs"
    / "characters_leaders_cabinet_live_20260701_scheduled_cadence_budget_v1"
)


def main() -> None:
    benchmark = json.loads(BENCHMARK.read_text(encoding="utf-8"))
    cadence = json.loads(CADENCE.read_text(encoding="utf-8"))
    cadence_summary = cadence.get("summary", {})
    advanced_hours = cadence_summary.get("advanced_hours") or 0
    wall_elapsed_s = cadence.get("probe", {}).get("wall_elapsed_s") or 0
    observed_hour_wall_us = (
        int(wall_elapsed_s * 1_000_000 / advanced_hours) if advanced_hours else 0
    )
    avg_us = int(benchmark.get("avg_read_time_us") or 0)
    max_us = int(benchmark.get("max_read_time_us") or 0)
    result = {
        "schema": "characters_leaders_cabinet_live.scheduled_cadence_budget.v1",
        "result": "validated"
        if (
            benchmark.get("result") == "validated"
            and benchmark.get("status_flags_or") == 0
            and benchmark.get("parse_errors") == []
            and observed_hour_wall_us > 0
            and max_us < observed_hour_wall_us // 2
            and avg_us < observed_hour_wall_us // 4
        )
        else "failed",
        "interpretation": (
            "native cached-root read/encode/gate cost is below observed live "
            "hour cadence budget; shared cache population and scheduling remain unproven"
        ),
        "benchmark_path": str(BENCHMARK),
        "cadence_path": str(CADENCE),
        "benchmark_frames": benchmark.get("frames"),
        "benchmark_rows_per_frame": benchmark.get("last_row_count"),
        "avg_read_time_us": avg_us,
        "max_read_time_us": max_us,
        "live_cadence_advanced_hours": advanced_hours,
        "live_cadence_wall_elapsed_s": wall_elapsed_s,
        "observed_hour_wall_us": observed_hour_wall_us,
        "avg_fraction_of_observed_hour": round(avg_us / observed_hour_wall_us, 4)
        if observed_hour_wall_us
        else None,
        "max_fraction_of_observed_hour": round(max_us / observed_hour_wall_us, 4)
        if observed_hour_wall_us
        else None,
        "budget_policy": {
            "max_read_time_lt_half_observed_hour": max_us < observed_hour_wall_us // 2
            if observed_hour_wall_us
            else False,
            "avg_read_time_lt_quarter_observed_hour": avg_us < observed_hour_wall_us // 4
            if observed_hour_wall_us
            else False,
        },
        "production_scope": (
            "lane-local native timing only; not DLL-only shared scheduled capture"
        ),
    }
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "scheduled_cadence_budget.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(result, indent=2, sort_keys=True))
    if result["result"] != "validated":
        raise SystemExit(2)


if __name__ == "__main__":
    main()
