#!/usr/bin/env python3
"""Mine country dynamic-modifier instances from a retained-save census.

The miner uses the existing save-oracle tokenizer and rakaly discovery but
keeps only the A18 literal save fields.  It is intentionally an evidence
inventory, not a live-memory or promotion verifier: cross-save differences can
be caused by script actions as well as elapsed duration.
"""

from __future__ import annotations

import argparse
import gc
import hashlib
import json
from datetime import datetime, timedelta
from pathlib import Path

import deep_common as dc
from deep_common import LBRACE, _skip_block_body, iter_block_entries
import parse_country_extra as country_extra


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _parse_country(tz):
    modifiers = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key == "dynamic_modifier":
            modifiers = country_extra._parse_dynamic_modifier(tz)
        else:
            _skip_block_body(tz)
    return modifiers


def _parse_countries(tz, out):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        modifiers = _parse_country(tz)
        if modifiers:
            out[tag] = modifiers


def _parse_text(text: str) -> tuple[str | None, dict[str, list[dict]]]:
    countries: dict[str, list[dict]] = {}
    meta = dc.top_level_pass(
        text,
        {"countries": lambda tz: _parse_countries(tz, countries)},
        scalar_keys=("date",),
    )
    return meta.get("date"), countries


def _hour(date_text: str | None) -> int | None:
    if not date_text:
        return None
    try:
        year, month, day, hour = (int(part) for part in date_text.split("."))
        base = datetime(year, month, day) + timedelta(hours=hour)
        epoch = datetime(1, 1, 1)
        return int((base - epoch).total_seconds() // 3600)
    except (TypeError, ValueError, OverflowError):
        return None


def _flat(countries: dict[str, list[dict]]) -> dict[tuple[str, str, str | None], dict]:
    out = {}
    for tag, modifiers in countries.items():
        for row in modifiers:
            modifier = row.get("modifier")
            if modifier:
                out[(tag, modifier, row.get("tag"))] = row
    return out


def _transitions(saves: list[dict]) -> list[dict]:
    by_campaign: dict[str, list[dict]] = {}
    for save in saves:
        if save.get("status") == "parsed" and save.get("save_hour") is not None:
            by_campaign.setdefault(save["campaign_id_v1"], []).append(save)

    out = []
    for campaign, rows in sorted(by_campaign.items()):
        rows.sort(key=lambda row: (row["save_hour"], row["basename"]))
        for before, after in zip(rows, rows[1:]):
            left = _flat(before["countries"])
            right = _flat(after["countries"])
            changes = []
            for key in sorted(left.keys() | right.keys()):
                old = left.get(key)
                new = right.get(key)
                old_days = None if old is None else old.get("days")
                new_days = None if new is None else new.get("days")
                if old is None and new_days is not None:
                    kind = "timed_added"
                elif new is None and old_days is not None:
                    kind = "timed_removed_between_saves"
                elif old is not None and new is not None and old_days != new_days:
                    kind = "days_changed"
                elif old is not None and new is not None and (
                    old.get("enabled") != new.get("enabled")
                    or old.get("value") != new.get("value")
                ):
                    kind = "payload_changed"
                else:
                    continue
                changes.append({
                    "kind": kind,
                    "country": key[0],
                    "modifier": key[1],
                    "source_tag": key[2],
                    "before_days": old_days,
                    "after_days": new_days,
                    "before_enabled": None if old is None else old.get("enabled"),
                    "after_enabled": None if new is None else new.get("enabled"),
                    "value_changed": (
                        old is not None and new is not None
                        and old.get("value") != new.get("value")
                    ),
                })
            if changes:
                out.append({
                    "campaign_id_v1": campaign,
                    "before": before["basename"],
                    "after": after["basename"],
                    "before_date": before["save_date"],
                    "after_date": after["save_date"],
                    "elapsed_game_hours": after["save_hour"] - before["save_hour"],
                    "changes": changes,
                })
    return out


def mine(census_path: Path, save_root: Path, rakaly: Path | None) -> dict:
    census = json.loads(census_path.read_text(encoding="utf-8"))
    candidates = []
    for group in census["candidate_groups"]:
        for source in group["sources"]:
            candidates.append((group["campaign_id_v1"], source))
    candidates.sort(key=lambda item: (item[0], item[1]["name"]))

    rows = []
    temp_dir = census_path.parent
    for index, (campaign, source) in enumerate(candidates, 1):
        path = save_root / source["name"]
        print(f"[{index}/{len(candidates)}] {source['name']}", flush=True)
        row = {
            "campaign_id_v1": campaign,
            "basename": source["name"],
            "size_bytes": source["size_bytes"],
            "census_prefix_sha256": source["prefix_sha256"],
        }
        if not path.is_file():
            row.update(status="missing")
            rows.append(row)
            continue
        try:
            text, was_melted = dc.ps.melt_if_needed(path, rakaly, temp_dir)
            save_date, countries = _parse_text(text)
            instances = [item for values in countries.values() for item in values]
            row.update(
                status="parsed",
                full_sha256=_sha256(path),
                was_melted=was_melted,
                save_date=save_date,
                save_hour=_hour(save_date),
                countries=countries,
                instance_count=len(instances),
                timed_instance_count=sum(item.get("days") is not None for item in instances),
                permanent_instance_count=sum(item.get("days") is None for item in instances),
                value_instance_count=sum(bool(item.get("value")) for item in instances),
                disabled_instance_count=sum(item.get("enabled") is False for item in instances),
                source_tag_instance_count=sum(item.get("tag") is not None for item in instances),
            )
            del text
            gc.collect()
        except Exception as exc:  # inventory every retained input, including failures
            row.update(status="error", error=f"{type(exc).__name__}: {exc}")
        rows.append(row)

    transitions = _transitions(rows)
    parsed = [row for row in rows if row["status"] == "parsed"]
    return {
        "schema": "hoi4.a18.retained_save_dynamic_modifier_mine.v1",
        "authority": "save_persisted_literal_fields_only",
        "inputs": {
            "census_basename": census_path.name,
            "census_sha256": _sha256(census_path),
            "pinned_executable_sha256": census.get("pinned_executable_sha256"),
            "candidate_count": len(candidates),
        },
        "summary": {
            "parsed": len(parsed),
            "missing": sum(row["status"] == "missing" for row in rows),
            "errors": sum(row["status"] == "error" for row in rows),
            "instances": sum(row.get("instance_count", 0) for row in parsed),
            "timed_instances": sum(row.get("timed_instance_count", 0) for row in parsed),
            "permanent_instances": sum(row.get("permanent_instance_count", 0) for row in parsed),
            "cross_save_pairs_with_changes": len(transitions),
            "days_changed": sum(
                change["kind"] == "days_changed"
                for transition in transitions for change in transition["changes"]
            ),
            "timed_removed_between_saves": sum(
                change["kind"] == "timed_removed_between_saves"
                for transition in transitions for change in transition["changes"]
            ),
        },
        "saves": rows,
        "cross_save_transitions": transitions,
        "prohibited_inference": [
            "save-persisted days do not prove the same live-memory field without a runtime join",
            "a modifier absent from a later save may have expired or been removed by script",
            "non-consecutive retained saves do not prove daily decrement cadence",
            "save value vectors are portable literals but do not establish live fixed-point scale",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("census", type=Path)
    parser.add_argument("save_root", type=Path)
    parser.add_argument("--rakaly", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    rakaly = args.rakaly or dc.ps.find_rakaly(None)
    if rakaly is None:
        raise SystemExit("rakaly is required for retained binary saves")
    report = mine(args.census.resolve(), args.save_root.resolve(), rakaly.resolve())
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report["summary"], sort_keys=True), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
