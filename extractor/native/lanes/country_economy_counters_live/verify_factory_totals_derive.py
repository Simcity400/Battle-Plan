#!/usr/bin/env python3
"""Independently verify factory state totals from raw save state buildings.

This verifier deliberately does not reuse parse_economy.py's state aggregation.
It parses raw state building blocks through parse_save.py, recomputes
industrial_complex/arms_factory/dockyard levels by state attribution, and then
compares those totals to the country-economy raw oracle fields.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[4]
SAVE_ORACLE_DIR = REPO_ROOT / "extractor" / "save_oracle"
DEFAULT_SAVE_DIR = (
    Path.home()
    / "OneDrive"
    / "Documents"
    / "Paradox Interactive"
    / "the game"
    / "save games"
)

if str(SAVE_ORACLE_DIR) not in sys.path:
    sys.path.insert(0, str(SAVE_ORACLE_DIR))

import parse_save as ps  # type: ignore  # noqa: E402


FIELD_MAP = {
    "civilian": {
        "factory_key": "civ",
        "from_states_key": "civilian_from_state_building_levels",
        "offmap_key": "civilian_offmap_building_levels",
        "total_key": "civilian_total",
    },
    "military": {
        "factory_key": "mil",
        "from_states_key": "military_from_state_building_levels",
        "offmap_key": "military_offmap_building_levels",
        "total_key": "military_total",
    },
    "dockyard": {
        "factory_key": "dockyard",
        "from_states_key": "dockyard_from_state_building_levels",
        "offmap_key": "dockyard_offmap_building_levels",
        "total_key": "dockyard_total",
    },
}


def write_json(path: Path, obj: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def load_save_text(save_path: Path, rakaly_arg: str | None) -> tuple[str, bool, str | None]:
    header = ps.read_save_header(save_path)
    if header.startswith(b"HOI4txt"):
        return save_path.read_text(encoding=ps.ENCODING, errors="replace"), False, None
    if not header.startswith(b"HOI4bin"):
        return save_path.read_text(encoding=ps.ENCODING, errors="replace"), False, None

    rakaly = ps.find_rakaly(rakaly_arg)
    if rakaly is None:
        raise RuntimeError(
            f"{save_path.name} is HOI4bin but rakaly was not found; pass --rakaly."
        )
    proc = subprocess.run(
        [
            str(rakaly),
            "melt",
            "--format",
            "the game",
            "--to-stdout",
            "-u",
            "stringify",
            str(save_path),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    out = proc.stdout
    if out.startswith(b"HOI4txt") and len(out) > 1024:
        warning = None
        if proc.returncode != 0:
            warning = ps._scrub(proc.stderr.decode("utf-8", "replace").strip())
        return out.decode(ps.ENCODING, errors="replace"), True, warning
    raise RuntimeError(
        f"rakaly melt failed (exit {proc.returncode}, {len(out)} bytes out): "
        f"{ps._scrub(proc.stderr.decode('utf-8', 'replace'))[:500]}"
    )


def sum_state_factories(result: dict[str, Any], attribution: str) -> dict[str, dict[str, int]]:
    totals: dict[str, dict[str, int]] = {}
    states = result.get("states", {})
    if not isinstance(states, dict):
        return totals
    for state_id, info in states.items():
        if not isinstance(info, dict):
            continue
        if attribution == "owner":
            tag = info.get("owner")
        elif attribution == "controller_with_owner_fallback":
            tag = info.get("controller") or info.get("owner")
        else:
            raise ValueError(f"unknown attribution mode: {attribution}")
        if not tag:
            continue
        factories = info.get("factories", {})
        if not isinstance(factories, dict):
            continue
        row = totals.setdefault(str(tag), {"civ": 0, "mil": 0, "dockyard": 0})
        for key in ("civ", "mil", "dockyard"):
            value = factories.get(key, 0)
            if isinstance(value, bool):
                continue
            if isinstance(value, (int, float)):
                row[key] += int(value)
    return totals


def compare_state_totals(
    totals: dict[str, dict[str, int]],
    raw_countries: dict[str, Any],
) -> dict[str, Any]:
    summary: dict[str, Any] = {}
    mismatches: dict[str, list[dict[str, Any]]] = {}
    for kind, spec in FIELD_MAP.items():
        compared = 0
        matches = 0
        missing_oracle = 0
        rows: list[dict[str, Any]] = []
        factory_key = spec["factory_key"]
        oracle_key = spec["from_states_key"]
        for tag in sorted(raw_countries):
            raw = raw_countries.get(tag, {})
            factories_raw = raw.get("factories_raw", {}) if isinstance(raw, dict) else {}
            if not isinstance(factories_raw, dict):
                continue
            target = factories_raw.get(oracle_key)
            if target is None:
                missing_oracle += 1
                continue
            compared += 1
            observed = totals.get(tag, {}).get(factory_key, 0)
            if observed == target:
                matches += 1
            else:
                rows.append(
                    {
                        "tag": tag,
                        "recomputed_from_raw_state_buildings": observed,
                        "oracle_from_states": target,
                        "delta_observed_minus_oracle": observed - target,
                    }
                )
        mismatches[kind] = rows
        summary[kind] = {
            "countries_compared": compared,
            "matches": matches,
            "mismatches": len(rows),
            "missing_oracle": missing_oracle,
        }
    return {"summary": summary, "mismatches": mismatches}


def summarize_self_consistency(raw_countries: dict[str, Any]) -> dict[str, Any]:
    summary: dict[str, Any] = {}
    residual_examples: list[dict[str, Any]] = []
    residual_country_tags: set[str] = set()
    for kind, spec in FIELD_MAP.items():
        compared = 0
        state_plus_offmap_matches = 0
        state_only_equals_total = 0
        residual_nonzero = 0
        offmap_nonzero = 0
        for tag in sorted(raw_countries):
            raw = raw_countries.get(tag, {})
            factories_raw = raw.get("factories_raw", {}) if isinstance(raw, dict) else {}
            if not isinstance(factories_raw, dict):
                continue
            state_levels = factories_raw.get(spec["from_states_key"])
            offmap_levels = factories_raw.get(spec["offmap_key"])
            total = factories_raw.get(spec["total_key"])
            if not all(isinstance(value, int) for value in (state_levels, offmap_levels, total)):
                continue
            compared += 1
            state_plus_offmap = state_levels + offmap_levels
            residual = total - state_levels
            if state_levels == total:
                state_only_equals_total += 1
            if state_plus_offmap == total:
                state_plus_offmap_matches += 1
            if residual != 0:
                residual_nonzero += 1
                residual_country_tags.add(tag)
                if len(residual_examples) < 16:
                    residual_examples.append(
                        {
                            "tag": tag,
                            "kind": kind,
                            "state_levels": state_levels,
                            "offmap_levels": offmap_levels,
                            "total": total,
                            "residual_total_minus_state": residual,
                        }
                    )
            if offmap_levels != 0:
                offmap_nonzero += 1
        summary[kind] = {
            "countries_compared": compared,
            "state_only_equals_total": state_only_equals_total,
            "residual_nonzero": residual_nonzero,
            "offmap_nonzero": offmap_nonzero,
            "state_plus_offmap_equals_total": state_plus_offmap_matches,
            "interpretation": (
                "Oracle self-consistency only: total, from_states, and offmap "
                "all come from the save-oracle economy parser."
            ),
        }
    return {
        "summary": summary,
        "residual_country_count": len(residual_country_tags),
        "residual_examples": residual_examples,
    }


def build_report(args: argparse.Namespace) -> dict[str, Any]:
    save_path = Path(args.save)
    economy_raw_path = Path(args.economy_raw)
    text, was_melted, melt_warning = load_save_text(save_path, args.rakaly)
    raw_save = ps.parse_save_text(text)
    economy_raw = json.loads(economy_raw_path.read_text(encoding="utf-8"))
    raw_countries = economy_raw.get("countries", {})
    if not isinstance(raw_countries, dict):
        raw_countries = {}

    attribution_checks = {}
    for attribution in ("owner", "controller_with_owner_fallback"):
        totals = sum_state_factories(raw_save, attribution)
        attribution_checks[attribution] = {
            "method": (
                "Sum raw states.<id>.buildings.{industrial_complex,arms_factory,"
                "dockyard}.level from parse_save.py output, independent of "
                "parse_economy.py aggregation."
            ),
            **compare_state_totals(totals, raw_countries),
        }

    states = raw_save.get("states", {})
    explicit_controller_states = 0
    if isinstance(states, dict):
        explicit_controller_states = sum(
            1
            for row in states.values()
            if isinstance(row, dict) and row.get("controller") and row.get("controller") != row.get("owner")
        )

    return {
        "schema": "country_economy_counters_live.factory_totals_derive_verify.v2",
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "source_save_redacted": ps._redact(save_path),
        "economy_raw_source": str(economy_raw_path),
        "was_melted": was_melted,
        "melt_warning": melt_warning,
        "meta": {
            "save_date": raw_save.get("meta", {}).get("date"),
            "save_version": raw_save.get("meta", {}).get("version"),
            "states_parsed": len(states) if isinstance(states, dict) else 0,
            "countries_in_economy_raw": len(raw_countries),
            "explicit_controller_not_owner_states": explicit_controller_states,
        },
        "independent_state_level_recompute": {
            "oracle_fields_compared": [
                spec["from_states_key"] for spec in FIELD_MAP.values()
            ],
            "attribution_checks": attribution_checks,
            "primary_oracle_attribution": (
                "owner; parse_economy.py's *_from_states fields are owner-attributed. "
                "controller_with_owner_fallback is reported separately for occupation edge cases."
            ),
        },
        "oracle_total_accounting_self_consistency": summarize_self_consistency(raw_countries),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save", required=True)
    parser.add_argument("--economy-raw", required=True)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--rakaly")
    args = parser.parse_args(argv)
    try:
        report = build_report(args)
    except Exception as exc:
        failure = {
            "schema": "country_economy_counters_live.factory_totals_derive_verify.v2",
            "status": "failed",
            "error": str(exc),
        }
        write_json(Path(args.out_json), failure)
        print(json.dumps({"status": "failed", "error": str(exc)}, sort_keys=True))
        return 1
    report["status"] = "ok"
    write_json(Path(args.out_json), report)
    owner_summary = report["independent_state_level_recompute"]["attribution_checks"]["owner"]["summary"]
    controller_summary = report["independent_state_level_recompute"]["attribution_checks"][
        "controller_with_owner_fallback"
    ]["summary"]
    print(
        json.dumps(
            {
                "status": "ok",
                "out": args.out_json,
                "owner": owner_summary,
                "controller_with_owner_fallback": controller_summary,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
