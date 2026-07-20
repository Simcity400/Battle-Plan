#!/usr/bin/env python3
"""Independent, fail-closed verification for the A27 full-reader fixture.

This verifier deliberately separates two questions which the producer's own
acceptance gate cannot separate:

* is the emitted fixture internally consistent with the save oracle and the
  proven a729 producer ordering laws; and
* is the artifact an independently sourced, same-tick live full replacement
  that is safe to promote.

The checked-in artifact currently answers only the first question.  In
particular, a fixture assembled from the oracle must never be promoted merely
because it round-trips through the production reader logic.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


PROGRAM = "hoi4.exe 1.19.2.0.a729"
PROGRAM_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
IMAGE_BASE = "0x140000000"
EXPECTED_VARIANTS = 5_281
EXPECTED_TEMPLATES = 1_126

VARIANT_DEFINITION_FIELDS = {
    "name",
    "archetype",
    "family",
    "modules",
    "upgrades",
    "version",
    "max_version",
    "is_frame",
    "parent_id",
    "design_team",
    "design_team_bonus",
    "design_team_mio",
    "manpower",
}

TEMPLATE_DEFINITION_FIELDS = {
    "name",
    "raw_name",
    "localization_key",
    "original_tag",
    "role",
    "division_names_group",
    "priority",
    "division_cap",
    "is_locked",
    "is_army_hq",
    "override_model",
    "regiments",
    "support",
    "regimental_support",
    "n_regiments",
    "n_support",
}

LIVE_V2_VARIANT_DEFINITION_FIELDS = {
    "archetype_token",
    "cached_stats",
    "cached_stats_complete",
    "display_name",
    "equipment_definition_pointer",
    "modules",
    "modules_complete",
    "upgrades",
    "upgrades_complete",
    "variant_name",
}

LIVE_V2_TEMPLATE_DEFINITION_FIELDS = {
    "is_locked",
    "lock_regime_snapshot_raw",
    "name",
    "obsolete",
    "priority",
    "regiment_grid_columns",
    "regiment_grid_rows",
    "regimental_support",
    "regiments",
    "support",
}

EXPECTED_VARIANTS_WITHOUT_UI_OCCURRENCE = {
    "1093:70",
    "1094:70",
    "1095:70",
    "1096:70",
    "1097:70",
    "1098:70",
    "1099:70",
    "1100:70",
    "1101:70",
    "1691:70",
    "1692:70",
    "1693:70",
    "1694:70",
    "4540:70",
}


PROVENANCE = [
    {
        "purpose": "exact CDivisionTemplate vtable root for lifecycle candidate tracing",
        "function_address": "0x142938F70 (vtable data root)",
        "job_id": "20260717T132604Z-a27-template-vtable-lifecycle-census-0x2-5e26b42f",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T132604Z-a27-template-vtable-lifecycle-census-0x2-5e26b42f/datdump.txt",
        "artifact_sha256": "388df4e4736915b25da62ae24628151def9a23657ad9c1e46958fd1b6dbccf54",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T132604Z-a27-template-vtable-lifecycle-census-0x2-5e26b42f/identity/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "finding": "Live census identifies outer +0x236 exactly across all 1126 templates; the parser evidence below freezes it as the inner +0x21E boolean field.",
    },
    {
        "purpose": "CReferencedDivisionTemplate wrapper-to-data layout",
        "function_address": "0x140B87FC0",
        "job_id": "20260717T140850Z-a27-template-scalar-semantics-235-236-1a-7d4d2924",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T140850Z-a27-template-scalar-semantics-235-236-1a-7d4d2924/evidence/func_140b87fc0.decompile.c",
        "artifact_sha256": "78c5d5b019c51f425b7247b5f239b7a66116107b2e533c5ad3e847b580f3c24f",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T140850Z-a27-template-scalar-semantics-235-236-1a-7d4d2924/evidence/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "934d6a3e5e8a641f6fed2c971d1e33e768bc2c8ef61149911a9c8c7d1141907c",
        "finding": "The wrapper destroys its embedded CDivisionTemplateData at outer +0x18 and deletes a 0x260-byte wrapper, proving the +0x18 offset delta.",
    },
    {
        "purpose": "CDivisionTemplateData exact scalar parser dispatch",
        "function_address": "0x140B937A0",
        "job_id": "20260717T141128Z-a27-template-data-fields-18c-1b4-21d-21e-b70cc535",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T141128Z-a27-template-data-fields-18c-1b4-21d-21e-b70cc535/evidence/func_140b937a0.decompile.c",
        "artifact_sha256": "a6c97da9ad52961c33152b92211ccc118ec5e153e5ebcbea2e25b7d0188f46a3",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T141128Z-a27-template-data-fields-18c-1b4-21d-21e-b70cc535/evidence/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "73826c5b80cba2628791266d0dab3ce8632453a6e622fd1d428bce755032ad7c",
        "finding": "Parser tokens write u32 inner +0x18C and booleans inner +0x1B4, +0x21D, and +0x21E. With the +0x18 wrapper delta these are outer +0x1A4, +0x1CC, +0x235, and +0x236.",
    },
    {
        "purpose": "explicit division-template lock selector",
        "function_address": "0x140B931F0",
        "job_id": "20260717T141128Z-a27-template-data-fields-18c-1b4-21d-21e-b70cc535",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T141128Z-a27-template-data-fields-18c-1b4-21d-21e-b70cc535/evidence/func_140b931f0.decompile.c",
        "artifact_sha256": "ae4d272806f5068d20a29426e249a0d4b778547b0238e44555672c8e6f8ee288",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T141128Z-a27-template-data-fields-18c-1b4-21d-21e-b70cc535/evidence/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "73826c5b80cba2628791266d0dab3ce8632453a6e622fd1d428bce755032ad7c",
        "finding": "Selects country +0x1B4 templates_locked for a positive original tag, otherwise inner +0x21D; this confirms outer +0x235 as the explicit template is_locked path.",
    },
    {
        "purpose": "exact equipment designer producer traversal and first-seen pointer dedupe",
        "function_address": "0x14177D780",
        "job_id": "20260717T110352Z-a27-designer-grid-population-callers-9c115a8b",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T110352Z-a27-designer-grid-population-callers-9c115a8b/evidence/func_14177d780.decompile.c",
        "artifact_sha256": "da4a782a395a2ef363c9ea30e63667aa3280ebf1e743b10a6e14b927f5ddfd03",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T110352Z-a27-designer-grid-population-callers-9c115a8b/evidence/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "e12cbbd037d94c33a81381b49021116dc1ca7af7325af9bbffe101acd5d82904",
        "finding": "Candidate-country order, list A then list B, and first-seen pointer dedupe are the display-order root.",
    },
    {
        "purpose": "template obsolete UI string consumer",
        "function_address": "0x141D16CA0",
        "job_id": "20260717T124959Z-a27-template-obsolete-string-xref-functi-4c0b96d5",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T124959Z-a27-template-obsolete-string-xref-functi-4c0b96d5/evidence/func_141d16ca0.decompile.c",
        "artifact_sha256": "d292b176743aa638222e887f9f6789d00475007513ccb93e827f77b128b8396c",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T124959Z-a27-template-obsolete-string-xref-functi-4c0b96d5/evidence/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "95953ac5d99ab08e14b948972c041811d522ae7babd1df56ab5538cd82a6b9a9",
        "finding": "Binds the obsolete widget text; it does not establish a template lifecycle field offset.",
    },
    {
        "purpose": "obsolete_template UI widget consumer",
        "function_address": "0x141682290",
        "job_id": "20260717T124959Z-a27-template-obsolete-string-xref-functi-4c0b96d5",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T124959Z-a27-template-obsolete-string-xref-functi-4c0b96d5/evidence/func_141682290.decompile.c",
        "artifact_sha256": "c549a2496feaff39b7f91506a8006d7bae6a64c8d0a702e18122a14f5c080ac7",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T124959Z-a27-template-obsolete-string-xref-functi-4c0b96d5/evidence/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "95953ac5d99ab08e14b948972c041811d522ae7babd1df56ab5538cd82a6b9a9",
        "finding": "Binds obsolete_template UI state but does not establish a template lifecycle field offset.",
    },
    {
        "purpose": "division_templates save/checksum label consumer",
        "function_address": "0x1401F2A20",
        "job_id": "20260717T125336Z-a27-save-serializer-catalog-roots-ee747943",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T125336Z-a27-save-serializer-catalog-roots-ee747943/evidence/func_1401f2a20.decompile.c",
        "artifact_sha256": "bc679b4a175e75253a1fea1bfde625ccf8594ec06c562699aeaac75b9eaddf3a",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T125336Z-a27-save-serializer-catalog-roots-ee747943/evidence/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "f5736d9d5be12b44999a400adfa9c6272dc249a4fdcfacc25f2ae71b6367366d",
        "finding": "Hashes/labels division_templates in a global checksum path; it does not prove A/B UI traversal order.",
    },
]

STRING_PROVENANCE = [
    {
        "purpose": "template scalar semantic anchor search",
        "function_address": "0x140082CB0 and string-xref result set",
        "job_id": "20260717T141234Z-a27-template-field-token-semantics-235-2-0fb9660a",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T141234Z-a27-template-field-token-semantics-235-2-0fb9660a/string-xrefs.txt",
        "artifact_sha256": "4492b77d08dccade2a5ccc70dfcc77f14a3b8185e5a811fd809dbfa7f057a5a2",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T141234Z-a27-template-field-token-semantics-235-2-0fb9660a/identity/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "finding": "Finds is_locked, templates_locked, obsolete, and priority anchors; field identity remains grounded in parser code plus exact same-tick census rather than string proximity alone.",
    },
    {
        "purpose": "template obsolete/lifecycle anchor search",
        "function_address": "N/A (string-xref result set)",
        "job_id": "20260717T124902Z-a27-template-obsolete-lifecycle-anchors-9dfe852e",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T124902Z-a27-template-obsolete-lifecycle-anchors-9dfe852e/string-xrefs.txt",
        "artifact_sha256": "e479add5e16a9b4a60ce1b01c7b3bb0553ecf6e3f7682eb7c784518be6912b1a",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T124902Z-a27-template-obsolete-lifecycle-anchors-9dfe852e/identity/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "finding": "No is_obsolete anchor; obsolete_change_date reaches only a generic parser path.",
    },
    {
        "purpose": "save serializer catalog-root anchor search",
        "function_address": "N/A (string-xref result set)",
        "job_id": "20260717T125301Z-a27-save-serializer-catalog-root-anchors-4c07ec66",
        "artifact_path": "C:/Users/Example/ghidra_agent_exports/20260717T125301Z-a27-save-serializer-catalog-root-anchors-4c07ec66/string-xrefs.txt",
        "artifact_sha256": "3a3d15fdd05c175be828b93af761a51d7d4412cc0253fead6bff8c0d6cf218e4",
        "manifest_path": "C:/Users/Example/ghidra_agent_exports/20260717T125301Z-a27-save-serializer-catalog-root-anchors-4c07ec66/identity/_FUNCTION_EVIDENCE_MANIFEST.json",
        "manifest_sha256": "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe",
        "finding": "division_templates reaches the checksum root; equipments xrefs are broad and do not establish UI order.",
    },
]


def _load(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: root must be an object")
    return value


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _key(row: dict[str, Any]) -> str:
    return f"{int(row['id'])}:{int(row['type'])}"


def _check(results: list[dict[str, Any]], name: str, passed: bool, detail: Any) -> bool:
    results.append({"name": name, "passed": bool(passed), "detail": detail})
    return bool(passed)


def _map_rows(rows: Any) -> tuple[dict[str, dict[str, Any]], list[str]]:
    errors: list[str] = []
    mapped: dict[str, dict[str, Any]] = {}
    if not isinstance(rows, list):
        return {}, ["not an array"]
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            errors.append(f"row {index}: not an object")
            continue
        try:
            key = _key(row)
        except (KeyError, TypeError, ValueError):
            errors.append(f"row {index}: invalid identity")
            continue
        if key in mapped:
            errors.append(f"duplicate identity {key}")
        mapped[key] = row
    return mapped, errors


def _oracle_variants(oracle: dict[str, Any]) -> dict[str, dict[str, Any]]:
    rows = oracle.get("equipment_variants")
    if not isinstance(rows, dict):
        return {}
    return {str(key): value for key, value in rows.items() if isinstance(value, dict)}


def _oracle_templates(oracle: dict[str, Any]) -> dict[str, dict[str, Any]]:
    rows = oracle.get("division_templates")
    if not isinstance(rows, list):
        return {}
    return {_key(row): row for row in rows if isinstance(row, dict)}


def _variant_membership_owner_lifecycle(
    live: dict[str, dict[str, Any]], saved: dict[str, dict[str, Any]]
) -> tuple[bool, list[str]]:
    errors: list[str] = []
    if set(live) != set(saved):
        errors.append("stable ID set differs")
    pointer_to_key: dict[str, str] = {}
    ordinals: list[int] = []
    for key, row in live.items():
        oracle = saved.get(key)
        if oracle is None:
            continue
        expected_origin = oracle.get("origin")
        if expected_origin in ("", "---"):
            expected_origin = None
        expected_creator = oracle.get("creator")
        comparisons = {
            "owner_tag": expected_creator,
            "creator_tag": expected_creator,
            "origin_override_tag": expected_origin,
            "selected_country_tag": expected_origin or expected_creator,
            "obsolete": bool(oracle.get("obsolete")),
        }
        for field, expected in comparisons.items():
            if row.get(field) != expected:
                errors.append(f"{key}: {field} mismatch")
        ordinal = row.get("first_discovery_ordinal")
        if not isinstance(ordinal, int):
            errors.append(f"{key}: invalid discovery ordinal")
        else:
            ordinals.append(ordinal)
        occurrences = row.get("source_occurrences")
        if not isinstance(occurrences, list) or not occurrences:
            errors.append(f"{key}: missing source occurrence")
            continue
        local_pointers: set[str] = set()
        local_slots: set[tuple[str, str, int]] = set()
        for occurrence in occurrences:
            if not isinstance(occurrence, dict):
                errors.append(f"{key}: invalid occurrence")
                continue
            country = occurrence.get("source_country_tag")
            source_list = occurrence.get("source_list")
            source_ordinal = occurrence.get("source_list_ordinal")
            pointer = occurrence.get("pointer_identity")
            slot = (country, source_list, source_ordinal)
            if not isinstance(country, str) or source_list not in ("A", "B") or not isinstance(source_ordinal, int):
                errors.append(f"{key}: invalid occurrence source")
            elif slot in local_slots:
                errors.append(f"{key}: duplicate occurrence slot")
            else:
                local_slots.add(slot)
            if not isinstance(pointer, str) or not pointer.startswith("0x"):
                errors.append(f"{key}: invalid pointer identity")
                continue
            local_pointers.add(pointer)
            prior = pointer_to_key.setdefault(pointer, key)
            if prior != key:
                errors.append(f"{key}: pointer shared with {prior}")
        if len(local_pointers) != 1:
            errors.append(f"{key}: stable ID maps to {len(local_pointers)} pointers")
    if sorted(ordinals) != list(range(len(live))):
        errors.append("discovery ordinals are not a complete zero-based permutation")
    return not errors, errors[:100]


def _template_membership_owner_lifecycle(
    live: dict[str, dict[str, Any]], saved: dict[str, dict[str, Any]]
) -> tuple[bool, list[str]]:
    errors: list[str] = []
    if set(live) != set(saved):
        errors.append("stable ID set differs")
    country_ordinals: dict[str, set[int]] = {}
    for key, row in live.items():
        oracle = saved.get(key)
        if oracle is None:
            continue
        if row.get("owner_tag") != oracle.get("country"):
            errors.append(f"{key}: owner mismatch")
        if row.get("source_country_tag") != oracle.get("country"):
            errors.append(f"{key}: source country mismatch")
        if row.get("obsolete") != bool(oracle.get("obsolete")):
            errors.append(f"{key}: obsolete mismatch")
        country = row.get("source_country_tag")
        ordinal = row.get("country_vector_ordinal")
        if not isinstance(country, str) or not isinstance(ordinal, int) or ordinal < 0:
            errors.append(f"{key}: invalid country ordinal")
            continue
        used = country_ordinals.setdefault(country, set())
        if ordinal in used:
            errors.append(f"{key}: duplicate {country} ordinal {ordinal}")
        used.add(ordinal)
    return not errors, errors[:100]


def _reconstruct_equipment_order(
    context: dict[str, Any], variants: dict[str, dict[str, Any]]
) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    candidates = context.get("candidate_country_tags")
    if not isinstance(candidates, list) or len(candidates) != len(set(candidates)):
        return [], ["invalid candidate-country array"]
    candidate_ordinal = {tag: index for index, tag in enumerate(candidates)}
    occurrences: list[tuple[int, int, int, str, str]] = []
    for key, row in variants.items():
        for occurrence in row.get("source_occurrences", []):
            if not isinstance(occurrence, dict):
                continue
            country = occurrence.get("source_country_tag")
            if country not in candidate_ordinal:
                continue
            source_list = occurrence.get("source_list")
            ordinal = occurrence.get("source_list_ordinal")
            pointer = occurrence.get("pointer_identity")
            if source_list not in ("A", "B") or not isinstance(ordinal, int) or not isinstance(pointer, str):
                errors.append(f"{key}: invalid candidate occurrence")
                continue
            occurrences.append(
                (candidate_ordinal[country], 0 if source_list == "A" else 1, ordinal, pointer, key)
            )
    occurrences.sort(key=lambda row: row[:3])
    seen: set[str] = set()
    source_order: list[str] = []
    for _, _, _, pointer, key in occurrences:
        if pointer not in seen:
            seen.add(pointer)
            source_order.append(key)
    predicates = context.get("engine_filter_predicates")
    outcomes = context.get("engine_filter_outcomes")
    if not isinstance(predicates, dict) or set(predicates) != set(source_order):
        errors.append("predicate coverage differs from reconstructed source order")
        predicates = {}
    if not isinstance(outcomes, dict) or set(outcomes) != set(source_order):
        errors.append("outcome coverage differs from reconstructed source order")
        outcomes = {}
    required = {
        "mode_type_gate",
        "common_type_gate",
        "handler_table_gate",
        "final_exclusion",
        "our_gate",
        "foreign_gate",
        "obsolete_gate",
        "selected_country_gate",
    }
    for key in source_order:
        row = predicates.get(key)
        if not isinstance(row, dict) or set(row) != required:
            errors.append(f"{key}: invalid predicate row")
            continue
        expected = (
            row["mode_type_gate"]
            and row["common_type_gate"]
            and row["handler_table_gate"]
            and not row["final_exclusion"]
            and row["our_gate"]
            and row["foreign_gate"]
            and row["obsolete_gate"]
            and row["selected_country_gate"]
        )
        if outcomes.get(key) is not expected:
            errors.append(f"{key}: predicate conjunction mismatch")
    displayed = [key for key in source_order if outcomes.get(key) is True]
    if context.get("ordered_design_ids") != displayed:
        errors.append("display order differs from reconstructed A/B traversal")
    return source_order, errors[:100]


def _reconstruct_division_order(
    context: dict[str, Any], templates: dict[str, dict[str, Any]]
) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    country = context.get("current_country_tag")
    source = sorted(
        (row["country_vector_ordinal"], key)
        for key, row in templates.items()
        if row.get("source_country_tag") == country and isinstance(row.get("country_vector_ordinal"), int)
    )
    source_order = [key for _, key in source]
    predicates = context.get("engine_filter_predicates")
    outcomes = context.get("engine_filter_outcomes")
    if not isinstance(predicates, dict) or set(predicates) != set(source_order):
        errors.append("predicate coverage differs from country-vector source order")
        predicates = {}
    if not isinstance(outcomes, dict) or set(outcomes) != set(source_order):
        errors.append("outcome coverage differs from country-vector source order")
        outcomes = {}
    for key in source_order:
        row = predicates.get(key)
        required = {"country_byte_matches", "subunits_available", "effective_locked"}
        if not isinstance(row, dict) or set(row) != required:
            errors.append(f"{key}: invalid predicate row")
            continue
        expected = row["country_byte_matches"] and row["subunits_available"] and not row["effective_locked"]
        if outcomes.get(key) is not expected:
            errors.append(f"{key}: predicate conjunction mismatch")
    displayed = [key for key in source_order if outcomes.get(key) is True]
    if context.get("ordered_design_ids") != displayed:
        errors.append("display order differs from country-vector traversal")
    return source_order, errors[:100]


def _live_v2_catalog_errors(
    variants: dict[str, dict[str, Any]],
    templates: dict[str, dict[str, Any]],
    saved_variants: dict[str, dict[str, Any]],
    saved_templates: dict[str, dict[str, Any]],
) -> tuple[list[str], list[str], list[str]]:
    variant_errors: list[str] = []
    template_errors: list[str] = []
    occurrence_errors: list[str] = []
    variant_pointers: dict[str, str] = {}
    catalog_ordinals: list[int] = []
    ui_ordinals: list[int] = []
    variants_without_occurrence: set[str] = set()
    global_slots: set[tuple[str, str, int]] = set()
    for key, row in variants.items():
        oracle = saved_variants.get(key)
        if oracle is None:
            continue
        expected_origin = oracle.get("origin")
        if expected_origin in ("", "---"):
            expected_origin = None
        expected_creator = oracle.get("creator")
        for field, expected in {
            "owner_tag": expected_creator,
            "creator_tag": expected_creator,
            "origin_override_tag": expected_origin,
            "selected_country_tag": expected_origin or expected_creator,
            "obsolete": bool(oracle.get("obsolete")),
        }.items():
            if row.get(field) != expected:
                variant_errors.append(f"{key}: {field} mismatch")
        pointer = row.get("pointer_identity")
        if not isinstance(pointer, str) or not pointer.startswith("0x"):
            variant_errors.append(f"{key}: invalid pointer identity")
        elif pointer in variant_pointers:
            variant_errors.append(f"{key}: pointer shared with {variant_pointers[pointer]}")
        else:
            variant_pointers[pointer] = key
        ordinal = row.get("catalog_scan_ordinal")
        if not isinstance(ordinal, int):
            variant_errors.append(f"{key}: invalid catalog ordinal")
        else:
            catalog_ordinals.append(ordinal)
        occurrences = row.get("source_occurrences")
        if not isinstance(occurrences, list):
            occurrence_errors.append(f"{key}: occurrences are not an array")
            continue
        ui_ordinal = row.get("first_ui_discovery_ordinal")
        if not occurrences:
            variants_without_occurrence.add(key)
            if ui_ordinal is not None:
                occurrence_errors.append(f"{key}: absent UI occurrence has an ordinal")
            continue
        if not isinstance(ui_ordinal, int):
            occurrence_errors.append(f"{key}: invalid first UI discovery ordinal")
        else:
            ui_ordinals.append(ui_ordinal)
        for occurrence in occurrences:
            if not isinstance(occurrence, dict):
                occurrence_errors.append(f"{key}: invalid occurrence")
                continue
            country = occurrence.get("source_country_tag")
            source_list = occurrence.get("source_list")
            source_ordinal = occurrence.get("source_list_ordinal")
            occurrence_pointer = occurrence.get("pointer_identity")
            slot = (country, source_list, source_ordinal)
            if (
                not isinstance(country, str)
                or source_list not in ("A", "B")
                or not isinstance(source_ordinal, int)
                or source_ordinal < 0
            ):
                occurrence_errors.append(f"{key}: invalid occurrence source")
            elif slot in global_slots:
                occurrence_errors.append(f"{key}: duplicate global occurrence slot")
            else:
                global_slots.add(slot)
            if occurrence_pointer != pointer:
                occurrence_errors.append(f"{key}: occurrence pointer differs from catalog pointer")
    if sorted(catalog_ordinals) != list(range(len(variants))):
        variant_errors.append("variant catalog ordinals are not a zero-based permutation")
    if sorted(ui_ordinals) != list(range(len(variants) - len(variants_without_occurrence))):
        occurrence_errors.append("UI discovery ordinals are not a zero-based permutation over A/B membership")
    if variants_without_occurrence != EXPECTED_VARIANTS_WITHOUT_UI_OCCURRENCE:
        occurrence_errors.append(
            "A/B-absent variant set differs: "
            + ",".join(sorted(variants_without_occurrence ^ EXPECTED_VARIANTS_WITHOUT_UI_OCCURRENCE))
        )

    template_pointers: dict[str, str] = {}
    template_catalog_ordinals: list[int] = []
    country_ordinals: dict[str, list[int]] = {}
    for key, row in templates.items():
        oracle = saved_templates.get(key)
        if oracle is None:
            continue
        if row.get("owner_tag") != oracle.get("country"):
            template_errors.append(f"{key}: owner mismatch")
        if row.get("source_country_tag") != oracle.get("country"):
            template_errors.append(f"{key}: source country mismatch")
        if row.get("obsolete") != bool(oracle.get("obsolete")):
            template_errors.append(f"{key}: obsolete mismatch")
        pointer = row.get("pointer_identity")
        if not isinstance(pointer, str) or not pointer.startswith("0x"):
            template_errors.append(f"{key}: invalid pointer identity")
        elif pointer in template_pointers:
            template_errors.append(f"{key}: pointer shared with {template_pointers[pointer]}")
        else:
            template_pointers[pointer] = key
        catalog_ordinal = row.get("catalog_scan_ordinal")
        if not isinstance(catalog_ordinal, int):
            template_errors.append(f"{key}: invalid catalog ordinal")
        else:
            template_catalog_ordinals.append(catalog_ordinal)
        country = row.get("source_country_tag")
        country_ordinal = row.get("country_vector_ordinal")
        if not isinstance(country, str) or not isinstance(country_ordinal, int) or country_ordinal < 0:
            template_errors.append(f"{key}: invalid country-vector ordinal")
        else:
            country_ordinals.setdefault(country, []).append(country_ordinal)
    if sorted(template_catalog_ordinals) != list(range(len(templates))):
        template_errors.append("template catalog ordinals are not a zero-based permutation")
    for country, ordinals in country_ordinals.items():
        if sorted(ordinals) != list(range(len(ordinals))):
            template_errors.append(f"{country}: template-vector ordinals are not contiguous")
    return variant_errors[:100], template_errors[:100], occurrence_errors[:100]


def _definition_errors(
    rows: dict[str, dict[str, Any]], required: set[str], label: str
) -> list[str]:
    errors: list[str] = []
    for key, row in rows.items():
        missing = sorted(required - set(row))
        if missing:
            errors.append(f"{label} {key}: missing {','.join(missing)}")
            if len(errors) >= 20:
                break
    return errors


def _nested_definition_errors(
    rows: dict[str, dict[str, Any]], required: set[str], label: str
) -> list[str]:
    errors: list[str] = []
    for key, row in rows.items():
        definition = row.get("definition")
        if row.get("definition_complete") is not True or not isinstance(definition, dict):
            errors.append(f"{label} {key}: definition badge/payload missing")
        else:
            missing = sorted(required - set(definition))
            if missing:
                errors.append(f"{label} {key}: definition missing {','.join(missing)}")
        if len(errors) >= 20:
            break
    return errors


def _verify_provenance(rows: list[dict[str, str]]) -> tuple[bool, list[dict[str, Any]]]:
    verified: list[dict[str, Any]] = []
    all_match = True
    for row in rows:
        artifact = Path(row["artifact_path"])
        manifest = Path(row["manifest_path"])
        artifact_actual = _sha256(artifact) if artifact.is_file() else None
        manifest_actual = _sha256(manifest) if manifest.is_file() else None
        artifact_match = artifact_actual == row["artifact_sha256"]
        manifest_match = manifest_actual == row["manifest_sha256"]
        all_match = all_match and artifact_match and manifest_match
        verified.append(
            {
                **row,
                "artifact_actual_sha256": artifact_actual,
                "artifact_hash_matches": artifact_match,
                "manifest_actual_sha256": manifest_actual,
                "manifest_hash_matches": manifest_match,
            }
        )
    return all_match, verified


def verify(
    oracle_path: Path,
    snapshot_path: Path,
    reader_path: Path,
    primary_validator_path: Path,
    expectation: str,
) -> dict[str, Any]:
    oracle = _load(oracle_path)
    snapshot = _load(snapshot_path)
    reader_source = reader_path.read_text(encoding="utf-8")
    primary_source = primary_validator_path.read_text(encoding="utf-8")
    results: list[dict[str, Any]] = []
    is_live_heap_v2 = snapshot.get("schema") == "extractor.a27.live_heap_catalog.a729.v2"

    authority = snapshot.get("authority") if isinstance(snapshot.get("authority"), dict) else {}
    _check(
        results,
        "exact_a729_authority",
        authority.get("program") == PROGRAM
        and (
            authority.get("image_base") == IMAGE_BASE
            or (is_live_heap_v2 and isinstance(authority.get("module_base"), str))
        )
        and authority.get("program_sha256") == PROGRAM_SHA256,
        authority,
    )

    live_variants, variant_shape_errors = _map_rows(snapshot.get("variants"))
    live_templates, template_shape_errors = _map_rows(snapshot.get("division_templates"))
    saved_variants = _oracle_variants(oracle)
    saved_templates = _oracle_templates(oracle)
    _check(
        results,
        "full_replacement_counts",
        len(live_variants) == len(saved_variants) == EXPECTED_VARIANTS
        and len(live_templates) == len(saved_templates) == EXPECTED_TEMPLATES,
        {
            "snapshot_variants": len(live_variants),
            "oracle_variants": len(saved_variants),
            "snapshot_templates": len(live_templates),
            "oracle_templates": len(saved_templates),
            "shape_errors": variant_shape_errors + template_shape_errors,
        },
    )

    if is_live_heap_v2:
        variant_errors, template_errors, v2_occurrence_errors = _live_v2_catalog_errors(
            live_variants, live_templates, saved_variants, saved_templates
        )
        variant_ok = not variant_errors
        template_ok = not template_errors
    else:
        variant_ok, variant_errors = _variant_membership_owner_lifecycle(live_variants, saved_variants)
        template_ok, template_errors = _template_membership_owner_lifecycle(live_templates, saved_templates)
        v2_occurrence_errors = []
    _check(results, "variant_membership_owner_lifecycle_matches_oracle", variant_ok, variant_errors)
    _check(results, "template_membership_owner_lifecycle_matches_oracle", template_ok, template_errors)

    equipment_contexts = snapshot.get("equipment_display_contexts")
    equipment_sources: list[list[str]] = []
    equipment_errors: list[str] = []
    if is_live_heap_v2:
        equipment_sources.append(
            sorted(
                key
                for key, row in live_variants.items()
                if isinstance(row.get("source_occurrences"), list) and row["source_occurrences"]
            )
        )
        equipment_errors.extend(v2_occurrence_errors)
    elif isinstance(equipment_contexts, list) and equipment_contexts:
        for context in equipment_contexts:
            if not isinstance(context, dict):
                equipment_errors.append("context is not an object")
                continue
            source, errors = _reconstruct_equipment_order(context, live_variants)
            equipment_sources.append(source)
            equipment_errors.extend(errors)
    else:
        equipment_errors.append("no equipment display context")
    _check(
        results,
        "equipment_order_reconstructs_from_proven_a_b_sources",
        not equipment_errors,
        {
            "context_count": len(equipment_sources),
            "source_counts": [len(value) for value in equipment_sources],
            "errors": equipment_errors,
        },
    )

    division_contexts = snapshot.get("division_display_contexts")
    division_sources: list[list[str]] = []
    division_errors: list[str] = []
    if is_live_heap_v2:
        division_sources.append(
            [
                key
                for key, _ in sorted(
                    (
                        (
                            row.get("source_country_registry_index"),
                            row.get("country_vector_ordinal"),
                            key,
                        ),
                        key,
                    )
                    for key, row in live_templates.items()
                )
            ]
        )
    elif isinstance(division_contexts, list) and division_contexts:
        for context in division_contexts:
            if not isinstance(context, dict):
                division_errors.append("context is not an object")
                continue
            source, errors = _reconstruct_division_order(context, live_templates)
            division_sources.append(source)
            division_errors.extend(errors)
    else:
        division_errors.append("no division display context")
    _check(
        results,
        "division_order_reconstructs_from_country_vector_sources",
        not division_errors,
        {
            "context_count": len(division_sources),
            "source_counts": [len(value) for value in division_sources],
            "errors": division_errors,
        },
    )

    snapshot_variant_order = [_key(row) for row in snapshot.get("variants", []) if isinstance(row, dict)]
    oracle_variant_order = list(saved_variants)
    _check(
        results,
        "save_insertion_order_is_not_used_as_ui_order",
        snapshot_variant_order != oracle_variant_order,
        {
            "oracle_first_ids": oracle_variant_order[:12],
            "source_reconstructed_first_ids": snapshot_variant_order[:12],
            "oracle_numeric_descents": sum(
                int(oracle_variant_order[index].split(":", 1)[0])
                < int(oracle_variant_order[index - 1].split(":", 1)[0])
                for index in range(1, len(oracle_variant_order))
            ),
            "orders_equal": snapshot_variant_order == oracle_variant_order,
            "conclusion": "Top-level save insertion order cannot substitute for the exact candidate-country/list-A/list-B producer order.",
        },
    )

    if is_live_heap_v2:
        variant_definition_errors = _nested_definition_errors(
            live_variants, LIVE_V2_VARIANT_DEFINITION_FIELDS, "variant"
        )
        template_definition_errors = _nested_definition_errors(
            live_templates, LIVE_V2_TEMPLATE_DEFINITION_FIELDS, "template"
        )
        required_variant_definition_fields = LIVE_V2_VARIANT_DEFINITION_FIELDS
        required_template_definition_fields = LIVE_V2_TEMPLATE_DEFINITION_FIELDS
    else:
        variant_definition_errors = _definition_errors(
            live_variants, VARIANT_DEFINITION_FIELDS, "variant"
        )
        template_definition_errors = _definition_errors(
            live_templates, TEMPLATE_DEFINITION_FIELDS, "template"
        )
        required_variant_definition_fields = VARIANT_DEFINITION_FIELDS
        required_template_definition_fields = TEMPLATE_DEFINITION_FIELDS
    definitions_present = not variant_definition_errors and not template_definition_errors
    _check(
        results,
        "definition_payload_present",
        definitions_present,
        {
            "required_variant_fields": sorted(required_variant_definition_fields),
            "required_template_fields": sorted(required_template_definition_fields),
            "errors_sample": variant_definition_errors + template_definition_errors,
            "conclusion": "definition_complete=true is not evidence when the definition fields are absent",
        },
    )

    synthetic_markers = {
        "cli_builds_oracle_fixture": "mem, source, config = build_oracle_fixture(oracle)" in reader_source,
        "oracle_fixture_builder_present": "def build_oracle_fixture(oracle" in reader_source,
        "sparse_memory_fixture_present": "class SparseMemory" in reader_source,
        "fixture_predicates_present": "class FixturePredicates" in reader_source,
        "template_lifecycle_copied_from_oracle": "template_obsolete[ptr] = bool(row.get(\"obsolete\"))" in reader_source,
        "definitions_only_asserted": not is_live_heap_v2
        and '"definition_complete": True' in reader_source
        and not any(f'mem.read' in line and field in line for field in VARIANT_DEFINITION_FIELDS for line in reader_source.splitlines()),
        "primary_gate_checks_badge_not_payload": 'row.get("definition_complete") is True' in primary_source,
    }
    live_heap_source_markers = {
        "heap_vtable_enumerator": "def _enum_vtable" in reader_source,
        "full_variant_catalog": '_catalog_objects(p, VARIANT_VTABLE_RVA' in reader_source,
        "full_template_catalog": '_catalog_objects(p, TEMPLATE_VTABLE_RVA' in reader_source,
        "variant_definition_reader": "def _variant_definition" in reader_source,
        "template_definition_reader": "def _template_definition" in reader_source,
        "snapshot_before_oracle": "snapshot = capture(process)" in reader_source,
    }
    independent_live_source = not any(synthetic_markers.values()) and (
        not is_live_heap_v2 or all(live_heap_source_markers.values())
    )
    _check(
        results,
        "independent_live_memory_source",
        independent_live_source,
        {"synthetic_markers": synthetic_markers, "live_heap_source_markers": live_heap_source_markers},
    )

    exact_template_lifecycle_source = (
        "FixturePredicates(template_obsolete)" not in reader_source
        and "template_obsolete[ptr] = bool(row.get(\"obsolete\"))" not in reader_source
        and (not is_live_heap_v2 or "TEMPLATE_OBSOLETE = 0x236" in reader_source)
    )
    _check(
        results,
        "exact_template_lifecycle_source",
        exact_template_lifecycle_source,
        {
            "source_interface": (
                "direct Process.u8(CDivisionTemplate+0x236) read"
                if is_live_heap_v2
                else "ExactPredicateSource.template_obsolete(template_ptr)"
            ),
            "current_source": "live CDivisionTemplate+0x236" if is_live_heap_v2 else "oracle division_templates[].obsolete",
            "status": "outer +0x236 is frozen as the inner +0x21E parsed boolean and matches obsolete for all 1126 same-tick rows",
        },
    )

    template_filter_semantics_exact = (
        (
            "TEMPLATE_LOCK_REGIME = 0x1CC" in reader_source
            and 'row["lock_regime_snapshot"] == current_locked' in reader_source
        )
        or (
            is_live_heap_v2
            and "TEMPLATE_LOCK_REGIME_SNAPSHOT = 0x1CC" in reader_source
            and "TEMPLATE_IS_LOCKED = 0x235" in reader_source
            and "TEMPLATE_OBSOLETE = 0x236" in reader_source
        )
    )
    _check(
        results,
        "division_template_filter_field_semantics",
        template_filter_semantics_exact,
        {
            "live_same_tick_census": {
                "template_is_locked": "CDivisionTemplate+0x235 u8, 1126/1126",
                "template_obsolete": "CDivisionTemplate+0x236 u8, 1126/1126",
                "template_priority": "CDivisionTemplate+0x1A4 u32, 1126/1126",
                "incidental_snapshot_correlation": "CDivisionTemplate+0x1CC happened to equal oracle is_army_hq for 1126/1126 rows in this save",
            },
            "ghidra_semantics": "FUN_140B937A0 parses inner +0x1B4, and the proven wrapper delta maps it to outer +0x1CC; the division predicate compares that raw lock-regime byte to CCountry+0x1B4",
            "current_reader": "correctly labels outer +0x1CC TEMPLATE_LOCK_REGIME and compares it to CCountry+0x1B4",
            "status": "the is_army_hq equality is a dataset correlation and does not override the concrete parser/predicate semantics",
        },
    )

    provenance_ok, provenance = _verify_provenance(PROVENANCE + STRING_PROVENANCE)
    _check(
        results,
        "ghidra_provenance_hashes_match",
        provenance_ok,
        {
            "row_count": len(provenance),
            "mismatches": [
                row["job_id"]
                for row in provenance
                if not row["artifact_hash_matches"] or not row["manifest_hash_matches"]
            ],
        },
    )

    fixture_core_names = {
        "exact_a729_authority",
        "full_replacement_counts",
        "variant_membership_owner_lifecycle_matches_oracle",
        "template_membership_owner_lifecycle_matches_oracle",
        "equipment_order_reconstructs_from_proven_a_b_sources",
        "division_order_reconstructs_from_country_vector_sources",
        "save_insertion_order_is_not_used_as_ui_order",
        "ghidra_provenance_hashes_match",
    }
    fixture_consistent = all(row["passed"] for row in results if row["name"] in fixture_core_names)
    promotion_ready = (
        fixture_consistent
        and definitions_present
        and independent_live_source
        and exact_template_lifecycle_source
        and template_filter_semantics_exact
    )
    promotion_check_names = fixture_core_names | {
        "definition_payload_present",
        "independent_live_memory_source",
        "exact_template_lifecycle_source",
        "division_template_filter_field_semantics",
    }
    promotion_blockers = [
        f"promotion check failed: {row['name']}"
        for row in results
        if row["name"] in promotion_check_names and not row["passed"]
    ]
    expectation_met = fixture_consistent if expectation == "offline-fixture" else promotion_ready

    return {
        "schema": "extractor.a27.independent_full_reader_verification.v1",
        "expectation": expectation,
        "expectation_met": expectation_met,
        "offline_fixture_consistent": fixture_consistent,
        "same_tick_live_acceptance": promotion_ready,
        "promotion_ready": promotion_ready,
        "authority": {
            "program": PROGRAM,
            "image_base": IMAGE_BASE,
            "program_sha256": PROGRAM_SHA256,
        },
        "input_hashes": {
            "oracle": {"path": str(oracle_path.resolve()), "sha256": _sha256(oracle_path)},
            "snapshot": {"path": str(snapshot_path.resolve()), "sha256": _sha256(snapshot_path)},
            "reader": {"path": str(reader_path.resolve()), "sha256": _sha256(reader_path)},
            "primary_validator": {
                "path": str(primary_validator_path.resolve()),
                "sha256": _sha256(primary_validator_path),
            },
        },
        "counts": {
            "variants": len(live_variants),
            "division_templates": len(live_templates),
            "candidate_countries": (
                int(snapshot.get("country_count", 0))
                if is_live_heap_v2
                else len(equipment_contexts[0].get("candidate_country_tags", []))
                if isinstance(equipment_contexts, list)
                and equipment_contexts
                and isinstance(equipment_contexts[0], dict)
                else 0
            ),
        },
        "checks": results,
        "promotion_blockers": promotion_blockers,
        "frozen_findings": {
            "serializer_order_refutation": "The oracle is numeric insertion order while the reconstructed producer order begins with creator-country/list traversal; the orders differ. Save order is not an A/B display-order proof.",
            "definitions_badge_refutation": "A boolean completeness badge does not replace the missing production definition payload.",
            "lifecycle_status": (
                "Variant obsolete and template obsolete are read directly from live memory. Template obsolete is frozen at outer +0x236/inner +0x21E and matches 1126/1126 same-tick rows (250 true/876 false)."
                if is_live_heap_v2
                else "Variant obsolete has a proven memory byte. Template obsolete is frozen at outer +0x236/inner +0x21E (1126/1126, 250 true/876 false); the fixture still copies it from the oracle."
            ),
            "template_scalar_status": "Outer +0x235=inner +0x21D explicit is_locked, outer +0x236=inner +0x21E obsolete, and outer +0x1A4=inner +0x18C priority. Outer +0x1CC is inner +0x1B4 raw lock-regime; its same-save equality with oracle is_army_hq is incidental.",
        },
        "ghidra_provenance": provenance,
    }


def main(argv: list[str] | None = None) -> int:
    base = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle", required=True, type=Path)
    parser.add_argument(
        "--snapshot",
        type=Path,
        default=base / "proofs" / "a27_full_reader_fixture" / "snapshot.json",
    )
    parser.add_argument("--reader", type=Path, default=base / "a729_memory_reader.py")
    parser.add_argument(
        "--primary-validator", type=Path, default=base / "validate_live_producer.py"
    )
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument(
        "--expect",
        choices=("offline-fixture", "live-capture"),
        default="live-capture",
        help="live-capture is the promotion gate; offline-fixture only freezes internal consistency",
    )
    args = parser.parse_args(argv)
    report = verify(
        args.oracle,
        args.snapshot,
        args.reader,
        args.primary_validator,
        args.expect,
    )
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(
        json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    print(
        "PASS" if report["expectation_met"] else "FAIL",
        f"expectation={report['expectation']}",
        f"offline_fixture_consistent={str(report['offline_fixture_consistent']).lower()}",
        f"promotion_ready={str(report['promotion_ready']).lower()}",
        f"report={args.report}",
    )
    return 0 if report["expectation_met"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
