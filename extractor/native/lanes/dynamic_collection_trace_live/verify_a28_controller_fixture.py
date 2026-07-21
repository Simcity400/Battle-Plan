#!/usr/bin/env python3
"""Fail-closed regrader for owner-captured A28 DCT1 controller observations."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


SCHEMA = "hoi4-a28-controller-observation-v1"
BINARY_ID = "hoi4.exe 1.19.2.0.a729"
EXECUTABLE_SHA256 = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)
AUTHORITY_BINDINGS = {
    "equipment_variant_grid": {
        "job_id": "20260720T133905Z-w5-a28-equipment-grid-owner-geometry-7d8900a4",
        "manifest_sha256": "96ad7c23413611dcc1524db1fac91a80c94324fc44f04b0e07430f927d8e76a4",
        "rvas": {
            "producer": "0x0177D780",
            "callback": "0x017797A0",
            "append": "0x002DE850",
            "finalize": "0x022B2F10",
            "scroll_sync": "0x022AFA30",
        },
    },
    "facility_specialization_smooth_list": {
        "job_id": "20260720T141140Z-w5-a28-facility-complete-hook-authority-d14d7d1d",
        "manifest_sha256": "fc5640f163fb89f9c52ae8db73344498f8f004adff5c048d913023807ea5c392",
        "rvas": {
            "controller": "0x01C9D840",
            "list_extent": "0x02295E90",
            "list_refresh": "0x02296D80",
            "scroll_current": "0x022D6CF0",
            "scroll_target": "0x022D6FE0",
        },
    },
    "technology_specialization_badge_grid": {
        "job_id": "20260720T141305Z-w5-a28-tech-badge-owner-callgraph-comple-b890bceb",
        "manifest_sha256": "f4644574640b69d5ae650ed6a68564cf7efb3b09f5a05b3888b65338f9e77d5e",
        "rvas": {
            "controller": "0x01BC1A00",
            "source_getter": "0x00B90430",
            "row_constructor": "0x01BBB9F0",
            "grid_insert": "0x002DE580",
            "grid_position": "0x022CA2E0",
        },
    },
    "named_overlap": {
        "job_id": "20260720T141317Z-w5-a28-overlap-owner-callgraph-complete-b8a10eba",
        "manifest_sha256": "74d14dced950d49ce7d47e4575bec7b04c84d5739cbc17c09681928b2d72eee4",
        "rvas": {
            "peace_owner": "0x01E34B30",
            "upgrade_owner": "0x01A00A50",
            "divisions_item_owner": "0x01CE5430",
            "divisions_pool_owner": "0x01CE6B40",
            "market_owner": "0x02034060",
            "layout": "0x02372190",
            "clip_propagation": "0x02371F90",
        },
    },
}
COMPLETENESS_KEYS = (
    "source_complete",
    "disposition_complete",
    "final_order_complete",
    "materialization_complete",
    "clip_viewport_complete",
    "scroll_complete",
)


class Rejection(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Rejection(message)


def sha256_text(value: Any, label: str) -> str:
    require(isinstance(value, str), f"{label} is not a string")
    require(
        len(value) == 64 and all(ch in "0123456789abcdef" for ch in value),
        f"{label} is not lowercase SHA-256",
    )
    return value


def portable_atom(value: Any, label: str) -> str:
    require(isinstance(value, str), f"{label} is not a string")
    require(value.strip() == value and bool(value), f"{label} is empty/untrimmed")
    require(len(value.encode("utf-8")) <= 16_384, f"{label} exceeds DCT1 bound")
    require("\x00" not in value, f"{label} contains NUL")
    require("\\" not in value, f"{label} contains a host path")
    require("0x" not in value.lower(), f"{label} contains a pointer-like atom")
    return value


def exact_int(value: Any, label: str, *, positive: bool = False) -> int:
    require(isinstance(value, int) and not isinstance(value, bool), f"{label} is not an integer")
    if positive:
        require(value > 0, f"{label} is not positive")
    return value


def validate_rect(value: Any, label: str) -> None:
    require(isinstance(value, dict), f"{label} is not an object")
    for key in ("x", "y", "width", "height"):
        exact_int(value.get(key), f"{label}.{key}")
    require(value["width"] > 0 and value["height"] > 0, f"{label} is not positive")


def validate_authority(root: dict[str, Any], kind: str) -> dict[str, Any]:
    binding = AUTHORITY_BINDINGS.get(kind)
    require(binding is not None, f"unsupported A28 collection kind {kind!r}")
    authority = root.get("authority")
    require(isinstance(authority, dict), "authority is absent")
    require(authority.get("binary_id") == BINARY_ID, "foreign binary id")
    require(
        authority.get("executable_sha256") == EXECUTABLE_SHA256,
        "foreign executable SHA-256",
    )
    require(authority.get("function_job_id") == binding["job_id"], "foreign Ghidra job")
    require(
        authority.get("function_manifest_sha256") == binding["manifest_sha256"],
        "foreign Ghidra manifest",
    )
    require(authority.get("rvas") == binding["rvas"], "producer/helper RVA set differs")
    require(authority.get("phase") == "post_return", "observation is not post-return")
    exact_int(authority.get("transition_sequence"), "transition_sequence", positive=True)
    exact_int(authority.get("observation_qpc_ticks"), "observation_qpc_ticks", positive=True)
    sha256_text(authority.get("producer_record_sha256"), "producer_record_sha256")
    return binding


def validate_collection(collection: dict[str, Any], frame_hour: int, expected_kind: str) -> None:
    kind = collection.get("kind")
    require(kind == expected_kind, "mixed or foreign A28 collection family")
    completeness = collection.get("completeness")
    require(isinstance(completeness, dict), "collection completeness is absent")
    for key in COMPLETENESS_KEYS:
        require(completeness.get(key) is True, f"{key} is not true")

    trace = collection.get("trace")
    require(isinstance(trace, dict), "trace is absent")
    portable_atom(trace.get("collection_instance_id"), "collection_instance_id")
    require(trace.get("frame_hour") == frame_hour, "collection frame hour differs")
    portable_atom(trace.get("row_template_id"), "row_template_id")
    validate_rect(trace.get("clip_rect"), "clip_rect")

    content = trace.get("content_extent_pixels")
    viewport = trace.get("viewport_extent_pixels")
    require(
        isinstance(content, list)
        and len(content) == 2
        and all(
            isinstance(value, int)
            and not isinstance(value, bool)
            and value >= 0
            for value in content
        ),
        "content extent is malformed",
    )
    require(
        isinstance(viewport, list)
        and len(viewport) == 2
        and all(
            isinstance(value, int)
            and not isinstance(value, bool)
            and value > 0
            for value in viewport
        ),
        "viewport extent is malformed",
    )

    scroll = [
        exact_int(trace.get(key), key)
        for key in (
            "scroll_min_fixed",
            "scroll_max_fixed",
            "scroll_current_fixed",
            "scroll_target_fixed",
        )
    ]
    require(scroll[0] <= scroll[2] <= scroll[1], "scroll current is unclamped")
    require(scroll[0] <= scroll[3] <= scroll[1], "scroll target is unclamped")

    source = trace.get("source_items")
    ordered = trace.get("ordered_item_ids")
    groups = trace.get("group_boundaries")
    slots = trace.get("materialized_slots")
    require(isinstance(source, list), "source_items is absent")
    require(isinstance(ordered, list), "ordered_item_ids is absent")
    require(groups == [], f"{kind} unexpectedly carries groups")
    require(isinstance(slots, list), "materialized_slots is absent")

    source_ids: set[str] = set()
    included: list[str] = []
    for ordinal, item in enumerate(source):
        require(isinstance(item, dict), "source item is not an object")
        stable_id = portable_atom(item.get("stable_item_id"), "stable_item_id")
        require(stable_id not in source_ids, "duplicate stable_item_id")
        source_ids.add(stable_id)
        require(item.get("source_ordinal") == ordinal, "source ordinal is not contiguous")
        require(isinstance(item.get("included"), bool), "included is not boolean")
        reason = item.get("filter_reason")
        if item["included"]:
            require(reason is None, "included source has a filter reason")
            included.append(stable_id)
        else:
            portable_atom(reason, "excluded filter_reason")

    for item_id in ordered:
        portable_atom(item_id, "ordered_item_id")
    require(ordered == included, "ordered IDs differ from included source order")
    require(len(slots) == len(ordered), "not every accepted row is materialized")
    slot_ids: set[str] = set()
    for index, slot in enumerate(slots):
        require(isinstance(slot, dict), "slot is not an object")
        slot_id = portable_atom(slot.get("slot_id"), "slot_id")
        require(slot_id not in slot_ids, "duplicate slot_id")
        slot_ids.add(slot_id)
        require(
            portable_atom(slot.get("bound_item_id"), "bound_item_id") == ordered[index],
            "materialized traversal differs from accepted order",
        )
        portable_atom(slot.get("row_template_id"), "slot row_template_id")
        validate_rect(slot.get("rect"), "slot.rect")

    if kind == "facility_specialization_smooth_list":
        require(
            trace["collection_instance_id"]
            == "interface/special_projects/facility_roster.gui#collection-0001",
            "facility collection identity differs",
        )
        require(all(item["included"] for item in source), "facility source unexpectedly filters")
        require(
            trace["row_template_id"] == "specialization_item"
            and all(slot["row_template_id"] == "specialization_item" for slot in slots),
            "facility specialization template differs",
        )
    elif kind == "technology_specialization_badge_grid":
        badge_prefix = "interface/countrytechtreeview.gui#collection-"
        badge_collection_id = trace["collection_instance_id"]
        badge_suffix = badge_collection_id.removeprefix(badge_prefix)
        require(
            badge_collection_id.startswith(badge_prefix)
            and len(badge_suffix) == 4
            and badge_suffix.isascii()
            and badge_suffix.isdigit()
            and 64 <= int(badge_suffix) <= 86,
            "technology badge collection is not one of the 23 authored grids",
        )
        require(len(ordered) == 1, "technology badge grid must contain one composite row")
        require(
            trace["row_template_id"] == "special_project_specialization"
            and all(
                slot["row_template_id"] == "special_project_specialization"
                for slot in slots
            ),
            "technology badge composite template differs",
        )
        require(
            all(
                item["included"]
                or item["filter_reason"] == "controller_two_badge_limit"
                for item in source
            ),
            "technology badge exclusion reason differs",
        )
        require(scroll == [0, 0, 0, 0], "technology badge grid unexpectedly scrolls")
    elif kind == "named_overlap":
        require(
            trace["collection_instance_id"]
            in {
                "interface/peaceconferencewindow.gui#collection-0003",
                "interface/countryintelligenceagencyview.gui#collection-0008",
                "interface/countrydiplomacyview.gui#collection-0034",
                "interface/unitview.gui#collection-0003",
                "interface/international_market/marketpurchasedraftwindow.gui#collection-0002",
            },
            "overlap collection is not one of the five controller-rooted instances",
        )
        require(scroll == [0, 0, 0, 0], "named overlap unexpectedly scrolls")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("observation", type=Path)
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()

    raw = args.observation.read_bytes()
    root = json.loads(raw)
    require(isinstance(root, dict), "observation root is not an object")
    require(root.get("schema") == SCHEMA, "foreign observation schema")
    frame_hour = exact_int(root.get("frame_hour"), "frame_hour")
    collections = root.get("collections")
    require(isinstance(collections, list) and collections, "no collection observations")
    require(isinstance(collections[0], dict), "collection is not an object")
    kind = collections[0].get("kind")
    require(isinstance(kind, str), "collection kind is absent")
    binding = validate_authority(root, kind)
    collection_ids: set[str] = set()
    for collection in collections:
        require(isinstance(collection, dict), "collection is not an object")
        validate_collection(collection, frame_hour, kind)
        collection_id = collection["trace"]["collection_instance_id"]
        require(collection_id not in collection_ids, "duplicate collection_instance_id")
        collection_ids.add(collection_id)

    receipt = {
        "schema": "hoi4-a28-controller-fixture-gate-v1",
        "status": "PASS",
        "observation_path": str(args.observation.resolve()),
        "observation_sha256": hashlib.sha256(raw).hexdigest(),
        "binary_id": BINARY_ID,
        "executable_sha256": EXECUTABLE_SHA256,
        "function_job_id": binding["job_id"],
        "function_manifest_sha256": binding["manifest_sha256"],
        "frame_hour": frame_hour,
        "collection_count": len(collections),
        "admitted_kinds": [kind],
    }
    payload = json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    if args.receipt:
        args.receipt.write_text(payload, encoding="utf-8")
    else:
        print(payload, end="")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, json.JSONDecodeError, Rejection) as error:
        raise SystemExit(f"A28 fixture rejected: {error}") from error
