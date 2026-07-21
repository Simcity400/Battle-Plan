from __future__ import annotations

from typing import Any


SCALAR = (str, int, float, bool, type(None))


def flatten(value: Any, prefix: str = "$") -> dict[str, Any]:
    output: dict[str, Any] = {}
    if isinstance(value, dict):
        for key in sorted(value, key=str):
            output.update(flatten(value[key], f"{prefix}.{key}"))
    elif isinstance(value, list):
        for index, item in enumerate(value):
            output.update(flatten(item, f"{prefix}[{index}]"))
    elif isinstance(value, SCALAR):
        output[prefix] = value
    return output


def changed(before: Any, after: Any) -> list[dict[str, Any]]:
    left = flatten(before)
    right = flatten(after)
    rows = []
    for path in sorted(set(left) | set(right)):
        old = left.get(path, {"missing": True})
        new = right.get(path, {"missing": True})
        if old != new:
            delta = None
            if isinstance(old, (int, float)) and not isinstance(old, bool) and isinstance(new, (int, float)) and not isinstance(new, bool):
                delta = new - old
            rows.append({"path": path, "before": old, "after": new, "delta": delta})
    return rows


def correlate(save_before: Any, save_after: Any, memory_before: Any, memory_after: Any) -> dict[str, Any]:
    save_deltas = changed(save_before, save_after)
    memory_deltas = changed(memory_before, memory_after)
    candidates = []
    for save in save_deltas:
        for memory in memory_deltas:
            reasons = []
            score = 0.0
            if save["before"] == memory["before"] and save["after"] == memory["after"]:
                score += 1.0
                reasons.append("exact_before_after_pair")
            elif save["delta"] is not None and memory["delta"] is not None and save["delta"] == memory["delta"]:
                score += 0.65
                reasons.append("exact_numeric_delta")
            if score:
                candidates.append({
                    "save_path": save["path"], "memory_path": memory["path"],
                    "score": score, "reasons": reasons,
                    "save_before": save["before"], "save_after": save["after"],
                    "memory_before": memory["before"], "memory_after": memory["after"],
                })
    candidates.sort(key=lambda row: (-row["score"], row["save_path"], row["memory_path"]))
    return {
        "schema": "hoi4-re-save-memory-correlation-v1",
        "save_delta_count": len(save_deltas),
        "memory_delta_count": len(memory_deltas),
        "candidate_count": len(candidates),
        "save_deltas": save_deltas,
        "memory_deltas": memory_deltas,
        "candidates": candidates,
        "promotion_allowed": False,
        "promotion_blocker": "correlation is discovery; static writer and live timing evidence remain required",
    }
