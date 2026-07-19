#!/usr/bin/env python3
"""Mine authenticated UI13 CPU streams for exact A26 tooltip-producer joins."""

from __future__ import annotations

import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
AUDIT = ROOT / "replay_interface/re_evidence/rendering_laws/UI13/retained_capture_audit/audit.json"
AUTHORED = ROOT / "replay_interface/re_evidence/rendering_laws/XUI02/authored_manager_census.json"
GAME = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV")
CORE_GUI = GAME / "interface/core.gui"
CAPTURE_ROOT = Path(r"C:\tmp\hoi4_rendering_law_capture")
OUT = HERE / "retained_cpu_tooltip_analysis.json"

FORMATTER_RVA = 0x2232AB0
TEXT_SETTER_RVA = 0x22B59E0
TOPBAR_TOOLTIP_START = 0x18845C0
TOPBAR_TOOLTIP_END = 0x1889E69
BATTLE_PLAN_IMAGE_SIZE = 55_680_632


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def integer(value: str | int) -> int:
    return value if isinstance(value, int) else int(value, 0)


def decode_output(event: dict) -> str:
    try:
        return bytes.fromhex(event.get("output_appended_bytes_hex", "")).decode("utf-8")
    except (ValueError, UnicodeDecodeError):
        return ""


def normalized_stack(event: dict, module_base: int) -> list[int]:
    rows = []
    for frame in event.get("stack_frames", []):
        address = integer(frame)
        if module_base <= address < module_base + BATTLE_PLAN_IMAGE_SIZE:
            rows.append(address - module_base)
    return rows


def mine_capture(row: dict) -> dict:
    path = Path(row["path"])
    if not path.is_file() or path.stat().st_size != row["bytes"]:
        raise RuntimeError(f"retained stream missing or changed: {path}")

    module_base = None
    active: dict[int, dict] = {}
    clusters = []
    abandoned = Counter()
    callsites = Counter()
    outputs = Counter()
    outputs_by_callsite: dict[str, Counter] = defaultdict(Counter)
    callers_after_topbar = Counter()
    call_chains = Counter()
    formatter_events = 0
    setter_edges = Counter()
    setter_edge_samples: dict[str, list[dict]] = defaultdict(list)
    line_count = 0
    digest = hashlib.sha256()

    with path.open("rb") as stream:
        for raw_line in stream:
            digest.update(raw_line)
            line_count += 1
            event = json.loads(raw_line)
            kind = event.get("type")
            thread = event.get("thread_id")
            sequence = event.get("event_sequence")

            if kind == "ui13_formatter_hook_created":
                module_base = integer(event["target"]) - integer(event["rva"])
                continue
            if module_base is None or thread is None or sequence is None:
                continue

            if kind == "ui13_formatter_call":
                stack = normalized_stack(event, module_base)
                topbar_indexes = [
                    index
                    for index, rva in enumerate(stack)
                    if TOPBAR_TOOLTIP_START <= rva <= TOPBAR_TOOLTIP_END
                ]
                if not topbar_indexes:
                    if thread in active:
                        abandoned["intervening_same_thread_formatter"] += 1
                        clusters.append(active.pop(thread))
                    continue

                formatter_events += 1
                topbar_index = topbar_indexes[0]
                callsite = stack[topbar_index]
                callsites[f"0x{callsite:X}"] += 1
                chain = stack[topbar_index : topbar_index + 6]
                call_chains[" -> ".join(f"0x{rva:X}" for rva in chain)] += 1
                if topbar_index + 1 < len(stack):
                    callers_after_topbar[f"0x{stack[topbar_index + 1]:X}"] += 1
                output = decode_output(event)
                outputs[output] += 1
                outputs_by_callsite[f"0x{callsite:X}"][output] += 1
                cluster = active.get(thread)
                if cluster is None:
                    cluster = {
                        "first_sequence": sequence,
                        "last_sequence": sequence,
                        "thread_id": thread,
                        "calls": 0,
                        "callsites": [],
                        "outputs": [],
                        "first_subsequent_same_thread_setter": None,
                    }
                    active[thread] = cluster
                cluster["last_sequence"] = sequence
                cluster["calls"] += 1
                cluster["callsites"].append(f"0x{callsite:X}")
                cluster["outputs"].append(output)
                continue

            if kind == "ui13_text_setter_call" and thread in active:
                cluster = active.pop(thread)
                edge = {
                    "event_sequence": sequence,
                    "gap_events": sequence - cluster["last_sequence"] - 1,
                    "entry_rva": event.get("entry_rva"),
                    "caller_return_rva": event.get("caller_return_rva"),
                    "widget": event.get("widget"),
                    "widget_key": event.get("widget_key"),
                    "assigned_text_utf8": event.get("assigned_text_utf8", ""),
                }
                cluster["first_subsequent_same_thread_setter"] = edge
                signature = "|".join(
                    [
                        str(edge["caller_return_rva"]),
                        str(edge["widget_key"]),
                        str(edge["gap_events"]),
                    ]
                )
                setter_edges[signature] += 1
                if len(setter_edge_samples[signature]) < 5:
                    setter_edge_samples[signature].append(
                        {
                            "cluster_first_sequence": cluster["first_sequence"],
                            "cluster_callsites": cluster["callsites"],
                            "cluster_outputs": cluster["outputs"],
                            **edge,
                        }
                    )
                clusters.append(cluster)

    for cluster in active.values():
        abandoned["end_of_stream_without_setter"] += 1
        clusters.append(cluster)

    exact_edges = sum(
        count
        for signature, count in setter_edges.items()
        if signature.rsplit("|", 1)[-1] == "0"
    )
    actual_sha256 = digest.hexdigest()
    if row["expected_sha256"] and actual_sha256 != row["expected_sha256"]:
        raise RuntimeError(f"retained stream hash changed: {path}")
    return {
        "capture": row["capture"],
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": actual_sha256,
        "expected_sha256": row["expected_sha256"],
        "manifest_or_audit_hash_match": row["expected_sha256"] == actual_sha256,
        "source_manifest": row["source_manifest"],
        "lock_files": row["lock_files"],
        "line_count": line_count,
        "module_base": f"0x{module_base:X}" if module_base is not None else None,
        "topbar_tooltip_formatter_events": formatter_events,
        "topbar_tooltip_clusters": len(clusters),
        "first_setter_edges": sum(setter_edges.values()),
        "zero_intervening_event_setter_edges": exact_edges,
        "abandoned_cluster_reasons": dict(sorted(abandoned.items())),
        "topbar_callsites": dict(callsites.most_common()),
        "first_game_frame_after_topbar": dict(callers_after_topbar.most_common()),
        "topbar_game_call_chains": dict(call_chains.most_common()),
        "topbar_output_fragments": dict(outputs.most_common(100)),
        "topbar_output_fragments_by_callsite": {
            callsite: dict(counter.most_common(100))
            for callsite, counter in sorted(outputs_by_callsite.items())
        },
        "setter_edge_signatures": [
            {
                "signature": signature,
                "count": count,
                "samples": setter_edge_samples[signature],
            }
            for signature, count in setter_edges.most_common()
        ],
    }


def capture_rows(audit: dict) -> list[dict]:
    audited = {
        str(Path(row["stream"]["path"]).resolve()).lower(): row
        for row in audit["captures"]
    }
    rows = []
    for ready_path in sorted(CAPTURE_ROOT.glob("*/cpu/cpu_capture_ready.json")):
        ready = json.loads(ready_path.read_text(encoding="utf-8"))
        if "UI13" not in ready.get("packets", []):
            continue
        root = ready_path.parents[1]
        stream_path = root / "cpu/cpu_state.jsonl"
        if not stream_path.is_file():
            continue
        key = str(stream_path.resolve()).lower()
        expected_sha256 = None
        source_manifest = None
        audit_row = audited.get(key)
        if audit_row is not None:
            expected_sha256 = audit_row["stream"]["sha256"]
            source_manifest = str(AUDIT)
        manifest_path = root / "manifest.json"
        if manifest_path.is_file():
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            entry = next(
                (
                    artifact
                    for artifact in manifest.get("artifacts", [])
                    if artifact.get("path", "").replace("\\", "/") == "cpu/cpu_state.jsonl"
                ),
                None,
            )
            if entry is not None:
                if expected_sha256 is not None and expected_sha256 != entry["sha256"]:
                    raise RuntimeError(f"audit/manifest hash disagreement: {stream_path}")
                expected_sha256 = entry["sha256"]
                source_manifest = str(manifest_path)
        if expected_sha256 is None:
            raise RuntimeError(f"no authenticated stream hash: {stream_path}")
        locks = [str(path.relative_to(root)) for path in root.rglob("*.lock")]
        rows.append(
            {
                "capture": root.name,
                "path": str(stream_path),
                "bytes": stream_path.stat().st_size,
                "expected_sha256": expected_sha256,
                "source_manifest": source_manifest,
                "lock_files": locks,
            }
        )
    return rows


def main() -> None:
    audit = json.loads(AUDIT.read_text(encoding="utf-8"))
    authored = json.loads(AUTHORED.read_text(encoding="utf-8"))
    captures = []
    for row in capture_rows(audit):
        print(f"mining {row['capture']}", flush=True)
        captures.append(mine_capture(row))

    aggregate_callsites = Counter()
    aggregate_callers_after_topbar = Counter()
    aggregate_call_chains = Counter()
    aggregate_edges = Counter()
    total_formatter_events = 0
    for capture in captures:
        aggregate_callsites.update(capture["topbar_callsites"])
        aggregate_callers_after_topbar.update(capture["first_game_frame_after_topbar"])
        aggregate_call_chains.update(capture["topbar_game_call_chains"])
        total_formatter_events += capture["topbar_tooltip_formatter_events"]
        for edge in capture["setter_edge_signatures"]:
            aggregate_edges[edge["signature"]] += edge["count"]

    payload = {
        "schema": "extractor-a26-retained-cpu-tooltip-analysis-v2",
        "authority": audit["authority"],
        "method": {
            "formatter_entry_rva": f"0x{FORMATTER_RVA:X}",
            "text_setter_entry_rva": f"0x{TEXT_SETTER_RVA:X}",
            "topbar_tooltip_function": "FUN_1418845c0",
            "topbar_tooltip_body": [
                f"0x{TOPBAR_TOOLTIP_START:X}",
                f"0x{TOPBAR_TOOLTIP_END:X}",
            ],
            "hoi4_image_size": BATTLE_PLAN_IMAGE_SIZE,
            "join_rule": "A formatter record is attributed only when a normalized exact stack frame lies inside the complete Ghidra function body. The first later same-thread setter is recorded as sequence evidence, not asserted causal unless a future controller export proves the call chain.",
        },
        "game_files": {
            "core_gui": str(CORE_GUI),
            "core_gui_sha256": sha256(CORE_GUI),
            "tooltip_template": {
                "name": "ToolTip",
                "textureFile": r"gfx\interface\tiles\tiles_dialog.tga",
                "font": "cg_16b",
                "borderSize": [18, 18],
                "maxWidth": 400,
                "maxHeight": 256,
            },
            "authored_manager_census": str(AUTHORED),
            "authored_manager_census_sha256": sha256(AUTHORED),
            "tooltip_assignment_count": authored["group_occurrence_counts"]["tooltip_ownership"],
            "tooltip_assignment_key_counts": authored["group_key_counts"]["tooltip_ownership"],
        },
        "captures": captures,
        "aggregate": {
            "capture_count": len(captures),
            "topbar_tooltip_formatter_events": total_formatter_events,
            "topbar_callsites": dict(aggregate_callsites.most_common()),
            "first_game_frame_after_topbar": dict(
                aggregate_callers_after_topbar.most_common()
            ),
            "topbar_game_call_chains": dict(aggregate_call_chains.most_common()),
            "first_same_thread_setter_edge_signatures": dict(aggregate_edges.most_common()),
        },
        "interpretation": {
            "proven": [
                "retained authenticated UI13 streams can attribute formatter calls to exact topbar tooltip producer stack sites",
                "all authenticated topbar tooltip formatter events share one exact virtual-dispatch caller chain",
                "authored installed content provide stable source file, hierarchy path, and authored tooltip-key identities",
            ],
            "not_yet_proven": [
                "the first subsequent same-thread setter is the generic tooltip controller edge",
                "a complete cross-view tooltip producer set",
                "the engine-resolved rectangle and semantic evaluation context for every instance",
            ],
        },
    }
    OUT.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(payload["aggregate"], indent=2), flush=True)


if __name__ == "__main__":
    main()
