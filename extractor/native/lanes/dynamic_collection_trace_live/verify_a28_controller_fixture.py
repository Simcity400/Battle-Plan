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
    "air_selection_list": {
        "job_id": "20260722T013755Z-a28-air-upstream-four-bucket-authority-285d9be0",
        "manifest_sha256": "839b5f4f9d184dfe4edcf0941797f56fd09e10753f5c8457ae748d21ad2a376e",
        "rvas": {
            "rebuild": "0x01F4A980",
            "source_selector": "0x01CDB770",
            "bucket_selector": "0x01951BA0",
            "vector_selector": "0x01951A10",
            "four_bucket_builder": "0x01952420",
            "common_archetype_gate": "0x00E505E0",
            "bucket_3_type_3_value": "0x01005DD0",
            "row_constructor": "0x01FC49D0",
        },
    },
    "mio_aggregated_bonus_smooth_list": {
        "job_id": "20260720T021840Z-w3-a28-direct-retry-5065e703",
        "manifest_sha256": "8ce0d88387228dc8a67553e6772c253f40792971760e2f2b695a87af2199b841",
        "rvas": {
            "rebuild": "0x01F05960",
            "fixed_rows": "0x01F06630",
            "equipment_groups": "0x01F05CD0",
            "separator_constructor": "0x01FA4B90",
        },
    },
    "mio_variant_history_smooth_list": {
        "job_id": "20260720T021840Z-w3-a28-direct-retry-5065e703",
        "manifest_sha256": "8ce0d88387228dc8a67553e6772c253f40792971760e2f2b695a87af2199b841",
        "rvas": {
            "source": "0x00DA1420",
            "materializer": "0x01F06360",
            "row_constructor": "0x01FA5390",
            "row_binder": "0x01FA6630",
        },
    },
    "mio_production_history_smooth_list": {
        "job_id": "20260720T021840Z-w3-a28-direct-retry-5065e703",
        "manifest_sha256": "8ce0d88387228dc8a67553e6772c253f40792971760e2f2b695a87af2199b841",
        "rvas": {
            "source": "0x00DA1680",
            "materializer": "0x01F06360",
            "row_constructor": "0x01FA5390",
            "row_binder": "0x01FA6630",
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
AIR_MIO_REJECTION_RVAS = {
    "air_selection_list": {
        "0x0195261B",
        "0x01952624",
        "0x01952631",
        "0x00E505E0",
        "0x01952837",
        "0x01952704",
        "0x01005DD0",
    },
    "mio_aggregated_bonus_smooth_list": {"0x01F06630", "0x01F05CD0"},
    "mio_variant_history_smooth_list": {"0x00DA1420"},
    "mio_production_history_smooth_list": {"0x00DA1680"},
}
AIR_REJECTION_LAW = {
    "0x0195261B": ("common_air_mask_zero", {0, 1, 2, 3}),
    "0x01952624": ("common_excluded_mask_nonzero", {0, 1, 2, 3}),
    "0x01952631": ("common_bit_33_set", {0, 1, 2, 3}),
    "0x00E505E0": ("common_archetype_gate_false", {0, 1, 2, 3}),
    "0x01952837": ("bucket_1_byte_5b4_zero", {1}),
    "0x01952704": ("bucket_2_bit_20_set", {2}),
    "0x01005DD0": ("bucket_3_type_3_value_nonpositive", {3}),
}
PAIR_TO_OBJECT_RESOLVER_JOB_ID = (
    "20260722T014524Z-a28-reference-table-lookup-final-ef179ecf"
)
PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256 = (
    "614e00757bf393f235f297fbf9519f33d038521c9ca9a97b1ef8553b4a86086c"
)
PAIR_TO_OBJECT_RESOLVER_RVA = "0x0220A3D0"
REFERENCE_TABLE_LOOKUP_RVA = "0x022083E0"
RESOLVER_SOURCES = {
    "air_selection_list": {"air-option-equipment-self-reference"},
    "mio_aggregated_bonus_smooth_list": {
        "mio-bonus-organisation-subject",
        "mio-bonus-object-reference",
    },
    "mio_variant_history_smooth_list": {
        "mio-variant-history-entry-reference"
    },
    "mio_production_history_smooth_list": {
        "mio-production-history-entry-reference"
    },
}


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


def portable_object_identity(value: Any, prefix: str, label: str) -> str:
    stable_id = portable_atom(value, label)
    parts = stable_id.removeprefix(prefix).split(":")
    require(stable_id.startswith(prefix), f"{label} has foreign prefix")
    require(
        len(parts) == 2
        and all(part.isascii() and part.isdigit() and int(part) > 0 for part in parts),
        f"{label} is not a positive numeric object identity",
    )
    return stable_id


def portable_mio_bonus_identity(value: Any, label: str) -> str:
    stable_id = portable_atom(value, label)
    if stable_id.startswith("mio-bonus:object:"):
        return portable_object_identity(stable_id, "mio-bonus:object:", label)
    parts = stable_id.removeprefix("mio-bonus:atom:").split(":")
    require(stable_id.startswith("mio-bonus:atom:"), f"{label} has foreign prefix")
    require(
        len(parts) == 4
        and len(parts[0]) == 8
        and all(ch in "0123456789abcdef" for ch in parts[0])
        and parts[0] in {"01f05960", "01f06630", "01f05cd0", "01fa4b90"}
        and parts[1].isascii()
        and parts[1].isdigit()
        and int(parts[1]) > 0
        and parts[2].isascii()
        and parts[2].isdigit()
        and int(parts[2]) > 0
        and parts[3].isascii()
        and parts[3].isdigit()
        and int(parts[3]) > 0,
        f"{label} is not an authenticated controller atom",
    )
    return stable_id


def portable_candidate_identity(
    value: Any,
    prefix: str,
    allowed_lanes: set[str],
    label: str,
    *,
    allow_controller_atom: bool = False,
) -> str:
    stable_id = portable_atom(value, label)
    base, separator, resolver_sha256 = stable_id.rpartition(":resolver:")
    require(bool(separator), f"{label} lacks resolver digest")
    sha256_text(resolver_sha256, f"{label} resolver digest")
    candidate_prefix = f"{prefix}candidate:"
    require(base.startswith(candidate_prefix), f"{label} lacks candidate wrapper")
    fields = base.removeprefix(candidate_prefix).split(":")
    require(len(fields) >= 4, f"{label} candidate wrapper is truncated")
    lane, source_key = fields[:2]
    require(lane in allowed_lanes, f"{label} has foreign source lane")
    require(
        source_key.isascii() and source_key.isdigit() and int(source_key) > 0,
        f"{label} source key is not positive decimal",
    )
    inner = prefix + ":".join(fields[2:])
    if allow_controller_atom:
        portable_mio_bonus_identity(inner, label)
    else:
        portable_object_identity(inner, f"{prefix}object:", label)
    return stable_id


def resolver_record_sha256(kind: str, record: dict[str, Any]) -> str:
    prefixes = {
        "air_selection_list": "air-selection",
        "mio_aggregated_bonus_smooth_list": "mio-bonus",
        "mio_variant_history_smooth_list": "mio-variant-history",
        "mio_production_history_smooth_list": "mio-production-history",
    }
    canonical = bytearray(b"hoi4-a28-pair-to-object-resolver-v1\0")
    canonical.extend(prefixes[kind].encode("ascii"))
    canonical.append(0)
    canonical.extend(record["source"].encode("ascii"))
    canonical.append(0)
    for field in (
        "resolver_rva_u32",
        "lookup_rva_u32",
        "raw_object_type",
        "raw_stable_id",
        "resolved_object_type",
        "resolved_stable_id",
    ):
        canonical.extend(record[field].to_bytes(4, "little", signed=False))
    canonical.append(int(record["resolved_non_null"]))
    canonical.append(int(record["round_trip_self_reference_matches"]))
    return hashlib.sha256(canonical).hexdigest()


def validate_identity_resolver(
    kind: str,
    stable_id: str,
    record: Any,
    expected_sha256: Any,
) -> None:
    require(isinstance(record, dict), "identity_resolver is absent")
    expected_keys = {
        "schema",
        "source",
        "resolver_rva",
        "lookup_rva",
        "resolver_job_id",
        "resolver_manifest_sha256",
        "raw_object_type",
        "raw_stable_id",
        "resolved_object_type",
        "resolved_stable_id",
        "resolved_non_null",
        "round_trip_self_reference_matches",
    }
    require(set(record) == expected_keys, "identity_resolver field set differs")
    require(
        record["schema"] == "hoi4-a28-pair-to-object-resolver-v1",
        "identity_resolver schema differs",
    )
    require(record["source"] in RESOLVER_SOURCES[kind], "foreign resolver source")
    require(record["resolver_rva"] == PAIR_TO_OBJECT_RESOLVER_RVA, "resolver RVA differs")
    require(record["lookup_rva"] == REFERENCE_TABLE_LOOKUP_RVA, "lookup RVA differs")
    require(
        record["resolver_job_id"] == PAIR_TO_OBJECT_RESOLVER_JOB_ID,
        "resolver job differs",
    )
    require(
        record["resolver_manifest_sha256"]
        == PAIR_TO_OBJECT_RESOLVER_MANIFEST_SHA256,
        "resolver manifest differs",
    )
    for field in (
        "raw_object_type",
        "raw_stable_id",
        "resolved_object_type",
        "resolved_stable_id",
    ):
        exact_int(record[field], f"identity_resolver.{field}", positive=True)
    require(
        record["raw_object_type"] == record["resolved_object_type"]
        and record["raw_stable_id"] == record["resolved_stable_id"],
        "resolver result pair differs from input pair",
    )
    require(record["resolved_non_null"] is True, "resolver returned null")
    require(
        record["round_trip_self_reference_matches"] is True,
        "resolved object does not round-trip to its self pair",
    )

    base, separator, suffix_sha256 = stable_id.rpartition(":resolver:")
    require(bool(separator), "portable identity lacks resolver suffix")
    sha256_text(suffix_sha256, "portable resolver suffix")
    fields = base.split(":")
    if ":object:" in base:
        expected_pair = (int(fields[-2]), int(fields[-1]))
        expected_source = {
            "air_selection_list": "air-option-equipment-self-reference",
            "mio_aggregated_bonus_smooth_list": "mio-bonus-object-reference",
            "mio_variant_history_smooth_list": "mio-variant-history-entry-reference",
            "mio_production_history_smooth_list": "mio-production-history-entry-reference",
        }[kind]
    else:
        require(kind == "mio_aggregated_bonus_smooth_list", "atom outside MIO bonus")
        expected_pair = (int(fields[-2]), int(fields[-1]))
        expected_source = "mio-bonus-organisation-subject"
    require(record["source"] == expected_source, "resolver source differs from identity kind")
    require(
        expected_pair == (record["raw_object_type"], record["raw_stable_id"]),
        "portable identity differs from raw resolver pair",
    )
    canonical_record = dict(record)
    canonical_record["resolver_rva_u32"] = int(record["resolver_rva"], 16)
    canonical_record["lookup_rva_u32"] = int(record["lookup_rva"], 16)
    digest = resolver_record_sha256(kind, canonical_record)
    require(
        sha256_text(expected_sha256, "identity_record_sha256") == digest,
        "identity resolver digest differs from canonical record",
    )
    require(suffix_sha256 == digest, "portable resolver suffix differs from ledger digest")


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
    if kind in {
        "air_selection_list",
        "mio_aggregated_bonus_smooth_list",
        "mio_variant_history_smooth_list",
        "mio_production_history_smooth_list",
    }:
        for key in (
            "candidate_universe_complete",
            "rejection_predicates_complete",
            "stable_identity_complete",
            "list_state_complete",
        ):
            require(authority.get(key) is True, f"{key} is not true")
    return binding


def validate_air_mio_ledger(
    kind: str,
    collection: dict[str, Any],
    source: list[dict[str, Any]],
) -> None:
    """Require the raw hook ledger which authenticates facts DCT1 normalizes."""
    ledger = collection.get("controller_ledger")
    require(isinstance(ledger, list), "controller_ledger is absent")
    require(len(ledger) == len(source), "controller ledger/source cardinality differs")
    candidate_bases: set[str] = set()
    for ordinal, (record, item) in enumerate(zip(ledger, source, strict=True)):
        require(isinstance(record, dict), "controller ledger record is not an object")
        require(record.get("source_ordinal") == ordinal, "controller ledger ordinal differs")
        ledger_id = portable_atom(record.get("stable_item_id"), "ledger stable_item_id")
        candidate_base, separator, _ = ledger_id.rpartition(":resolver:")
        require(bool(separator), "ledger stable_item_id lacks resolver digest")
        require(candidate_base not in candidate_bases, "controller ledger repeats candidate key")
        candidate_bases.add(candidate_base)
        validate_identity_resolver(
            kind,
            ledger_id,
            record.get("identity_resolver"),
            record.get("identity_record_sha256"),
        )
        require(
            ledger_id == item["stable_item_id"],
            "controller ledger identity differs from trace",
        )
        require(record.get("included") is item["included"], "ledger disposition differs")
        if item["included"]:
            require(record.get("predicate_rva") is None, "included ledger row has predicate RVA")
            require(
                record.get("predicate_record_sha256") is None,
                "included ledger row has predicate receipt",
            )
        else:
            predicate = record.get("predicate_rva")
            require(
                isinstance(predicate, str)
                and predicate.startswith("0x")
                and len(predicate) == 10
                and all(ch in "0123456789ABCDEF" for ch in predicate[2:]),
                "rejected ledger predicate RVA is malformed",
            )
            require(
                predicate in AIR_MIO_REJECTION_RVAS[kind],
                "rejected ledger predicate lacks family authority",
            )
            sha256_text(
                record.get("predicate_record_sha256"),
                "predicate_record_sha256",
            )
            require(
                portable_atom(record.get("reason_code"), "ledger reason_code")
                == item["filter_reason"],
                "ledger rejection reason differs from trace",
            )
            if kind == "air_selection_list":
                stable_id = record["stable_item_id"]
                lane_text = stable_id.removeprefix(
                    "air-selection:candidate:air-bucket-"
                ).split(":", 1)[0]
                require(
                    lane_text in {"0", "1", "2", "3"},
                    "air rejection lacks a frozen bucket lane",
                )
                expected_reason, allowed_buckets = AIR_REJECTION_LAW[predicate]
                require(
                    record["reason_code"] == expected_reason
                    and int(lane_text) in allowed_buckets,
                    "air rejection reason/predicate does not belong to its bucket",
                )


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

    if kind in {
        "air_selection_list",
        "mio_aggregated_bonus_smooth_list",
        "mio_variant_history_smooth_list",
        "mio_production_history_smooth_list",
    }:
        validate_air_mio_ledger(kind, collection, source)

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
    elif kind == "air_selection_list":
        require(
            trace["collection_instance_id"]
            == "interface/airselectionview.gui#collection-0001",
            "air-selection collection identity differs",
        )
        require(
            trace["row_template_id"] == "CPlaneTypeSelectionEntry"
            and all(slot["row_template_id"] == "CPlaneTypeSelectionEntry" for slot in slots),
            "air-selection runtime template differs",
        )
        require(
            all(
                portable_candidate_identity(
                    item["stable_item_id"],
                    "air-selection:",
                    {"air-bucket-0", "air-bucket-1", "air-bucket-2", "air-bucket-3"},
                    "air-selection stable_item_id",
                )
                for item in source
            ),
            "air-selection source does not use portable object identities",
        )
    elif kind == "mio_aggregated_bonus_smooth_list":
        require(
            trace["collection_instance_id"]
            == "interface/military_industrial_organization/industrial_organization_detail.gui#collection-0001",
            "MIO aggregated-bonus collection identity differs",
        )
        require(
            trace["row_template_id"] == "industrial_organisation_detail_mixed_rows",
            "MIO aggregated-bonus controller template differs",
        )
        allowed = {
            "industrial_organisation_detail_modifier_item",
            "industrial_organisation_detail_bonus_item",
            "industrial_organisation_equipment_type_window",
            "industrial_organisation_detail_separator",
        }
        require(
            all(slot["row_template_id"] in allowed for slot in slots),
            "MIO aggregated-bonus slot template differs",
        )
        require(
            all(
                portable_candidate_identity(
                    item["stable_item_id"],
                    "mio-bonus:",
                    {"mio-fixed-bonus", "mio-equipment-group"},
                    "MIO bonus stable_item_id",
                    allow_controller_atom=True,
                )
                for item in source
            ),
            "MIO aggregated-bonus source identity differs",
        )
    elif kind in {
        "mio_variant_history_smooth_list",
        "mio_production_history_smooth_list",
    }:
        expected_id = {
            "mio_variant_history_smooth_list":
                "interface/military_industrial_organization/industrial_organization_detail.gui#collection-0003",
            "mio_production_history_smooth_list":
                "interface/military_industrial_organization/industrial_organization_detail.gui#collection-0004",
        }[kind]
        expected_prefix = {
            "mio_variant_history_smooth_list": "mio-variant-history:",
            "mio_production_history_smooth_list": "mio-production-history:",
        }[kind]
        expected_lanes = {
            "mio_variant_history_smooth_list": {"mio-variant-table"},
            "mio_production_history_smooth_list": {
                "mio-production-line",
                "mio-production-table",
            },
        }[kind]
        require(trace["collection_instance_id"] == expected_id, "MIO history collection identity differs")
        require(
            trace["row_template_id"] == "industrial_organisation_history_item"
            and all(
                slot["row_template_id"] == "industrial_organisation_history_item"
                for slot in slots
            ),
            "MIO history runtime template differs",
        )
        require(
            all(
                portable_candidate_identity(
                    item["stable_item_id"],
                    expected_prefix,
                    expected_lanes,
                    "MIO history stable_item_id",
                )
                for item in source
            ),
            "MIO history source identity differs",
        )


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
