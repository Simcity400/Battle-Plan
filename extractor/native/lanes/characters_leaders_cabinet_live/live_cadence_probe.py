"""Bounded live game-hour cadence probe for domain 47.

This is a read-only proof harness. It does not replace DLL capture; it records
whether the currently running hoi4.exe advances game hours while lane-local
domain-47 proofs are attempted.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

import proof_live_roots as PROOF


REPO = Path(__file__).resolve().parents[4]


def _sample_clock(cl, started: float) -> dict:
    sample = PROOF.read_current_game_hour_sample(cl)
    sample["wall_elapsed_s"] = round(time.perf_counter() - started, 3)
    sample["utc"] = PROOF._utc_stamp()
    return sample


def _summarize(samples: list[dict]) -> dict:
    ok_samples = [sample for sample in samples if sample.get("ok")]
    raw_hours = [int(sample["game_hour_raw"]) for sample in ok_samples]
    unique_hours = []
    for raw in raw_hours:
        if not unique_hours or unique_hours[-1] != raw:
            unique_hours.append(raw)
    deltas = [
        unique_hours[index] - unique_hours[index - 1]
        for index in range(1, len(unique_hours))
    ]
    positive_deltas = [delta for delta in deltas if delta > 0]
    skipped_hours = sum(max(delta - 1, 0) for delta in positive_deltas)
    backwards = [delta for delta in deltas if delta < 0]
    advanced_hours = (
        max(0, raw_hours[-1] - raw_hours[0])
        if len(raw_hours) >= 2
        else 0
    )
    first = ok_samples[0] if ok_samples else None
    last = ok_samples[-1] if ok_samples else None
    return {
        "sample_count": len(samples),
        "ok_sample_count": len(ok_samples),
        "first": first,
        "last": last,
        "unique_game_hours": unique_hours,
        "advanced_hours": advanced_hours,
        "max_positive_hour_gap": max(positive_deltas) if positive_deltas else 0,
        "skipped_hours": skipped_hours,
        "zero_skipped_hours_observed": skipped_hours == 0 and not backwards,
        "backwards_deltas": backwards,
    }


def run(args: argparse.Namespace) -> dict:
    started = time.perf_counter()
    cl = PROOF.CL.CharactersLiveReader(args.process)
    build_ok, digest = cl.verify_build()
    if not build_ok and not args.force:
        raise SystemExit(f"build gate failed: {digest} != {PROOF.CL.EXPECTED_SHA256}")

    samples = []
    deadline = started + max(0.0, args.duration_s)
    while True:
        samples.append(_sample_clock(cl, started))
        summary = _summarize(samples)
        if summary["advanced_hours"] >= args.target_advanced_hours:
            break
        now = time.perf_counter()
        if now >= deadline:
            break
        time.sleep(min(max(args.poll_s, 0.05), max(0.0, deadline - now)))

    summary = _summarize(samples)
    if summary["ok_sample_count"] == 0:
        result = "blocked_clock_root_unreadable"
    elif summary["advanced_hours"] <= 0:
        result = "blocked_no_game_hour_advance_observed"
    elif summary["skipped_hours"] == 0 and not summary["backwards_deltas"]:
        result = "validated_clock_cadence_observed"
    else:
        result = "partial_skipped_or_unstable_hours_observed"

    return {
        "schema": "characters_leaders_cabinet_live.live_cadence_probe.v1",
        "lane": PROOF.DOMAIN_ID,
        "domain": "characters_leaders_cabinet_live",
        "result": result,
        "process": args.process,
        "build": {
            "pid": getattr(cl, "pid", None),
            "module_base": f"0x{cl.module_base:X}",
            "exe_path": getattr(cl, "exe_path", None),
            "sha256": digest,
            "sha256_matches_expected": bool(build_ok),
            "expected_sha256": PROOF.CL.EXPECTED_SHA256,
            "force": bool(args.force),
        },
        "probe": {
            "duration_s": args.duration_s,
            "poll_s": args.poll_s,
            "target_advanced_hours": args.target_advanced_hours,
            "wall_elapsed_s": round(time.perf_counter() - started, 3),
            "root_candidates": [f"0x{rva:X}" for rva in PROOF.GAMESTATE_RVA_CANDIDATES],
            "date_offset": f"+0x{PROOF.GAMESTATE_CURRENT_DATE_OFF:X}",
        },
        "summary": summary,
        "samples": samples,
        "production_interpretation": (
            "read-only lane-local clock evidence only; DLL-only domain-47 capture "
            "still requires shared scheduler/root-cache integration"
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--process", default="hoi4.exe")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--duration-s", type=float, default=30.0)
    parser.add_argument("--poll-s", type=float, default=1.0)
    parser.add_argument("--target-advanced-hours", type=int, default=2)
    parser.add_argument(
        "--out-dir",
        default=None,
        help="default extractor/native/runs/characters_leaders_cabinet_live_<UTC>_cadence",
    )
    args = parser.parse_args()
    out_dir = (
        Path(args.out_dir)
        if args.out_dir
        else REPO
        / "extractor/native"
        / "runs"
        / f"characters_leaders_cabinet_live_{PROOF._utc_stamp()}_cadence"
    )
    out_dir.mkdir(parents=True, exist_ok=True)
    result = run(args)
    PROOF._write_json(out_dir / "live_cadence_probe.json", result)
    print(str(out_dir))
    print(json.dumps({
        "result": result["result"],
        "advanced_hours": result["summary"]["advanced_hours"],
        "skipped_hours": result["summary"]["skipped_hours"],
        "ok_sample_count": result["summary"]["ok_sample_count"],
        "out": str(out_dir / "live_cadence_probe.json"),
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
