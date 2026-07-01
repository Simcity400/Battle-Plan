"""Lane-local proof runner for naval combat compact schema.

This runner is intentionally separate from shared collector/recorder entrypoints.
It fails closed on an empty bridge file; once the live watcher emits bridge rows,
the same command produces a compact lane-local stream and gates the decoder.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from memory_reader.combat.naval_combat_compact_lane import (  # noqa: E402
    proof_from_bridge_text,
    schema_manifest,
)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bridge-file", type=Path, default=None)
    ap.add_argument("--synthetic-bridge", default=None)
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--game-hour-raw", type=int, default=0)
    args = ap.parse_args()

    if args.synthetic_bridge:
        bridge_text = args.synthetic_bridge
        bridge_source = "synthetic"
    elif args.bridge_file:
        bridge_text = args.bridge_file.read_text(encoding="ascii").strip()
        bridge_source = str(args.bridge_file)
    else:
        raise SystemExit("--bridge-file or --synthetic-bridge is required")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    if not bridge_text:
        report = {
            "status": "blocked_empty_bridge",
            "bridge_source": bridge_source,
            "schema": schema_manifest(),
            "reason": "bridge rows are empty; wait for watcher to emit validated rows",
        }
        out_path = args.out_dir / "naval_combat_lane_local_compact_proof.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(json.dumps({"status": report["status"], "out_path": str(out_path)}, indent=2))
        return 2

    try:
        report = proof_from_bridge_text(
            bridge_text,
            out_dir=args.out_dir,
            game_hour_raw=args.game_hour_raw,
        )
    except Exception as exc:  # fail closed for proof commands
        report = {
            "status": "failed",
            "bridge_source": bridge_source,
            "error": str(exc),
            "schema": schema_manifest(),
        }
        out_path = args.out_dir / "naval_combat_lane_local_compact_proof.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(json.dumps({"status": "failed", "out_path": str(out_path), "error": str(exc)}, indent=2))
        return 1

    report["bridge_source"] = bridge_source
    out_path = args.out_dir / "naval_combat_lane_local_compact_proof.json"
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({
        "status": report["status"],
        "out_path": str(out_path),
        "bridge_row_count": report["bridge_row_count"],
        "compact_path": report["compact_path"],
        "gate_ok": report["gate"]["ok"],
    }, indent=2, sort_keys=True))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
