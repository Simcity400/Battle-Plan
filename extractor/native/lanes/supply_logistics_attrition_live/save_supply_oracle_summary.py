"""Summarize save-oracle supply_system_2 anchors for domain 44 residuals."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional


def _sample(items: List[Dict[str, Any]], limit: int = 12) -> List[Dict[str, Any]]:
    return items[:limit]


def summarize(path: Path) -> Dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    countries = data.get("countries", {})
    motorization: List[Dict[str, Any]] = []
    disrupted: List[Dict[str, Any]] = []
    foreign_homebase: List[Dict[str, Any]] = []
    truck_loss_countries = 0
    for tag, rec in sorted(countries.items()):
        for item in rec.get("node_motorization") or []:
            motorization.append({"tag": tag, **item})
        for item in rec.get("disrupted_supply") or []:
            disrupted.append({"tag": tag, **item})
        for item in rec.get("foreign_homebase_nodes") or []:
            foreign_homebase.append({"tag": tag, **item})
        if rec.get("lost_trucks_attrition_total") or rec.get("lost_trucks_killed_total"):
            truck_loss_countries += 1
    return {
        "source": str(path),
        "meta": data.get("meta", {}),
        "countries": len(countries),
        "node_motorization_entries": len(motorization),
        "node_motorization_countries": len({item["tag"] for item in motorization}),
        "disrupted_supply_entries": len(disrupted),
        "disrupted_supply_countries": len({item["tag"] for item in disrupted}),
        "foreign_homebase_entries": len(foreign_homebase),
        "foreign_homebase_countries": len({item["tag"] for item in foreign_homebase}),
        "truck_loss_countries": truck_loss_countries,
        "samples": {
            "node_motorization": _sample(motorization),
            "disrupted_supply": _sample(disrupted),
            "foreign_homebase_nodes": _sample(foreign_homebase),
        },
        "verdict": (
            "save_oracle_has_motorization_or_pressure_anchors"
            if motorization or disrupted or foreign_homebase
            else "save_oracle_has_no_motorization_or_pressure_anchors"
        ),
    }


def write_outputs(out_dir: Path, summary: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "supply_oracle_summary.json").write_text(
        json.dumps(summary, indent=2),
        encoding="utf-8",
    )
    meta = summary.get("meta", {})
    lines = [
        "# supply_logistics_attrition_live save supply oracle summary",
        "",
        f"- source: `{summary.get('source')}`",
        f"- save_date: `{meta.get('save_date')}`",
        f"- player: `{meta.get('player')}`",
        f"- countries: `{summary.get('countries')}`",
        f"- node_motorization_entries: `{summary.get('node_motorization_entries')}`",
        f"- node_motorization_countries: `{summary.get('node_motorization_countries')}`",
        f"- disrupted_supply_entries: `{summary.get('disrupted_supply_entries')}`",
        f"- foreign_homebase_entries: `{summary.get('foreign_homebase_entries')}`",
        f"- foreign_homebase_countries: `{summary.get('foreign_homebase_countries')}`",
        f"- truck_loss_countries: `{summary.get('truck_loss_countries')}`",
        f"- verdict: `{summary.get('verdict')}`",
        "",
        "## Samples",
        "",
        "### Node Motorization",
        "",
    ]
    for item in summary.get("samples", {}).get("node_motorization", []):
        lines.append(f"- `{item}`")
    if not summary.get("samples", {}).get("node_motorization"):
        lines.append("- none")
    lines.extend(["", "### Foreign Homebase Nodes", ""])
    for item in summary.get("samples", {}).get("foreign_homebase_nodes", []):
        lines.append(f"- `{item}`")
    if not summary.get("samples", {}).get("foreign_homebase_nodes"):
        lines.append("- none")
    lines.extend([
        "",
        "## Use In Domain 44",
        "",
        "- This is a save-oracle fixture source only. It does not satisfy production live extraction.",
        "- The current autosave contains motorization and foreign-homebase pressure anchors, so the remaining blocker is the current-build live root/function path that maps these saved records to memory.",
    ])
    (out_dir / "SUPPLY_ORACLE_SUMMARY.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("supply_json", type=Path)
    parser.add_argument("--out-dir", type=Path, default=None)
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or args.supply_json.parent
    summary = summarize(args.supply_json)
    write_outputs(out_dir, summary)
    print(json.dumps({
        "status": summary["verdict"],
        "out_dir": str(out_dir),
        "node_motorization_entries": summary["node_motorization_entries"],
        "disrupted_supply_entries": summary["disrupted_supply_entries"],
        "foreign_homebase_entries": summary["foreign_homebase_entries"],
    }, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
