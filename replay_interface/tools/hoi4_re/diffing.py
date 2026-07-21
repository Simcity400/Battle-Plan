from __future__ import annotations

import difflib
import hashlib
from collections import defaultdict
from typing import Any

from registry import Registry


def _unique_index(
    rows: list[dict[str, Any]], field: str,
) -> tuple[dict[str, dict[str, Any]], set[str]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[str(row[field])].append(row)
    return (
        {key: values[0] for key, values in grouped.items() if len(values) == 1},
        {key for key, values in grouped.items() if len(values) > 1},
    )


def _fuzzy_key(row: dict[str, Any]) -> tuple[str, str, int, int]:
    tokens = str(row["shape_text"]).split()
    return (
        tokens[0] if tokens else "",
        tokens[-1] if tokens else "",
        int(row["instruction_count"]) // 8,
        int(row["call_count"]),
    )


def diff_builds(
    registry: Registry, source_build_id: str, target_build_id: str, job_id: str,
) -> dict[str, Any]:
    source = registry.functions_for_build(source_build_id)
    target = registry.functions_for_build(target_build_id)
    if not source:
        raise ValueError(f"source build has no indexed functions: {source_build_id}")
    if not target:
        raise ValueError(f"target build has no indexed functions: {target_build_id}")
    source_symbols = registry.source_symbols(source_build_id)

    _, duplicate_source_bytes = _unique_index(source, "byte_sha256")
    _, duplicate_source_shapes = _unique_index(source, "shape_sha256")
    target_by_bytes, duplicate_bytes = _unique_index(target, "byte_sha256")
    target_by_shape, duplicate_shapes = _unique_index(target, "shape_sha256")
    fuzzy_buckets: dict[tuple[str, str, int, int], list[dict[str, Any]]] = defaultdict(list)
    for row in target:
        fuzzy_buckets[_fuzzy_key(row)].append(row)

    used_targets: set[int] = set()
    matches: list[dict[str, Any]] = []
    registry_matches: list[dict[str, Any]] = []
    unresolved: list[dict[str, Any]] = []
    for row in source:
        candidate = None if row["byte_sha256"] in duplicate_source_bytes else target_by_bytes.get(row["byte_sha256"])
        method = "exact_bytes"
        score = 1.0
        disposition = "exact"
        reasons: dict[str, Any] = {"byte_sha256_equal": bool(candidate)}
        if candidate is None or int(candidate["rva"]) in used_targets:
            candidate = None if row["shape_sha256"] in duplicate_source_shapes else target_by_shape.get(row["shape_sha256"])
            method = "exact_instruction_shape"
            score = 0.985
            disposition = "high_confidence"
            reasons = {"shape_sha256_equal": bool(candidate)}
        if ((candidate is None or int(candidate["rva"]) in used_targets) and
                row["shape_sha256"] not in duplicate_source_shapes):
            candidate = None
            best: tuple[float, dict[str, Any]] | None = None
            second = 0.0
            key = _fuzzy_key(row)
            keys = [
                (key[0], key[1], bucket, calls)
                for bucket in range(max(0, key[2] - 1), key[2] + 2)
                for calls in range(max(0, key[3] - 1), key[3] + 2)
            ]
            for lookup in keys:
                for possible in fuzzy_buckets.get(lookup, []):
                    if int(possible["rva"]) in used_targets:
                        continue
                    ratio = difflib.SequenceMatcher(
                        None, str(row["shape_text"]).split(), str(possible["shape_text"]).split(),
                        autojunk=False,
                    ).ratio()
                    if best is None or ratio > best[0]:
                        second = best[0] if best else second
                        best = (ratio, possible)
                    elif ratio > second:
                        second = ratio
            if best and best[0] >= 0.90 and best[0] - second >= 0.03:
                score, candidate = best
                method = "fuzzy_instruction_shape"
                disposition = "high_confidence" if score >= 0.95 else "changed_candidate"
                reasons = {"sequence_ratio": round(score, 6), "runner_up_ratio": round(second, 6)}

        target_rva = int(candidate["rva"]) if candidate is not None else None
        if target_rva is not None:
            used_targets.add(target_rva)
        else:
            ambiguous = (
                row["byte_sha256"] in duplicate_source_bytes or
                row["shape_sha256"] in duplicate_source_shapes or
                row["byte_sha256"] in duplicate_bytes or
                row["shape_sha256"] in duplicate_shapes
            )
            method = "ambiguous_collision" if ambiguous else "unmatched"
            score = 0.0
            disposition = "ambiguous" if ambiguous else "unmatched"
            reasons = {
                "reason": "fingerprint is non-unique across the source or target build"
                if ambiguous else "no unique candidate met the matching gate"
            }
            unresolved.append({"source_rva": f"0x{int(row['rva']):x}", "source_name": row["name"]})
        match_id = "match_" + hashlib.sha256(
            f"{source_build_id}:{row['rva']}:{target_build_id}:{target_rva}:{method}".encode("utf-8")
        ).hexdigest()[:20]
        symbol_id = source_symbols[int(row["rva"])]
        registry_matches.append({
            "match_id": match_id, "symbol_id": symbol_id,
            "source_build_id": source_build_id, "target_build_id": target_build_id,
            "source_rva": int(row["rva"]), "target_rva": target_rva,
            "method": method, "score": float(score), "disposition": disposition,
            "reasons": reasons, "job_id": job_id,
        })
        matches.append({
            "match_id": match_id,
            "symbol_id": symbol_id,
            "source_rva": f"0x{int(row['rva']):x}",
            "source_name": row["name"],
            "target_rva": None if target_rva is None else f"0x{target_rva:x}",
            "target_name": None if candidate is None else candidate["name"],
            "method": method,
            "score": round(float(score), 6),
            "disposition": disposition,
            "reasons": reasons,
        })

    registry.bind_matches(registry_matches)

    summary: dict[str, int] = defaultdict(int)
    for row in matches:
        summary[row["disposition"]] += 1
    matched = len(matches) - summary["unmatched"] - summary["ambiguous"]
    target_build = registry.get_build(target_build_id)
    pack_symbols = {
        row["symbol_id"]: {
            "rva": row["target_rva"], "status": row["disposition"],
            "match_id": row["match_id"], "method": row["method"], "score": row["score"],
        }
        for row in matches if row["target_rva"] is not None
    }
    return {
        "schema": "hoi4-re-build-diff-v1",
        "source_build_id": source_build_id,
        "target_build_id": target_build_id,
        "source_function_count": len(source),
        "target_function_count": len(target),
        "matched_function_count": matched,
        "coverage": round(matched / len(source), 6),
        "dispositions": dict(sorted(summary.items())),
        "matches": matches,
        "unresolved": unresolved,
        "compatibility_pack": {
            "schema": "hoi4-re-compatibility-pack-v1",
            "status": "draft-not-promoted",
            "build_id": target_build_id,
            "binary_id": target_build["binary_id"],
            "sha256": target_build["sha256"],
            "image_base": target_build["image_base"],
            "source_build_id": source_build_id,
            "symbols": pack_symbols,
        },
        "promotion_allowed": False,
        "promotion_blocker": "diff matches are migration candidates; changed and critical dependencies require fresh evidence",
    }
