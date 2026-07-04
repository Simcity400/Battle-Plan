#!/usr/bin/env python3
"""Build and verify the R3 localisation static-artifact (token -> localised
name join), packed + byte-identity verified into a ``.battleplan`` exactly
like the domain-59 gate
(``extractor/native/lanes/templates_equipment_catalog_live/templates_equipment_catalog_gate.py``).

This is NOT a compact domain (no domain id is assigned — R3 asks for "a
documented static bundle keyed by the recording's mod identity", not a new
live wire-format row) and it does not touch ``extractor/native/compact_contract``.
It reuses the SAME container mechanics as domain 59: a snapshot-store dir with
``index.json`` + payload files, packed via the prebuilt ``pack_replay`` binary,
then verified for byte-identity against the source store.

Usage:
  BATTLE_PLAN_VANILLA=<install> python extractor/game_defs/localisation_gate.py
  python extractor/game_defs/localisation_gate.py --vanilla <dir> --mods mod/test.mod
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))

import resolver as R  # noqa: E402
import parse_localisation  # noqa: E402
import parse_focus_def  # noqa: E402
import parse_world_defs  # noqa: E402
import parse_catalogs  # noqa: E402
import parse_characters  # noqa: E402
import parse_idea_tech_loc  # noqa: E402
import parse_decisions  # noqa: E402
from paradox_parser import Node  # noqa: E402

REPO_ROOT = _HERE.parent.parent
GAME_ID = "static_localisation_join_r3"
DEFAULT_OUT = _HERE / "out" / "localisation_gate"
_WIN_ABS_PATH = re.compile(r"[A-Za-z]:[\\/][^\s\"'<>|]+")
_FORBIDDEN_REPORT_PATTERNS = (
    "C:\\Users",
    "C:/Users",
    "OneDrive",
    "SteamLibrary",
)
_DRIVE_ABS_IN_JSON = re.compile(r"[A-Za-z]:[\\/]")
_LOC_KEY_TOKEN_RE = re.compile(r"^[A-Za-z0-9_.]+$")
_DYNAMIC_COUNTRY_TAG_RE = re.compile(r"^D(?:0[1-9]|[1-6][0-9]|7[0-5])$")


def canonical_json(data: Any) -> bytes:
    return json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def redact_path(p: Any) -> str:
    """Personal/home paths must not leak into emitted metadata — keep basenames
    only (e.g. an install root becomes just its directory name)."""
    path = Path(str(p))
    if not path.is_absolute():
        return path.as_posix()
    try:
        return path.resolve().relative_to(REPO_ROOT.resolve()).as_posix()
    except ValueError:
        return path.name


def _redact_text(text: str | None) -> str:
    if not text:
        return ""
    out = str(text)
    repo = str(REPO_ROOT.resolve())
    for root in (repo, repo.replace("\\", "/")):
        out = out.replace(root + "\\", "")
        out = out.replace(root + "/", "")
        out = out.replace(root, ".")
    return _WIN_ABS_PATH.sub(lambda m: redact_path(m.group(0)), out)


def _redact_cmd(cmd: list[str]) -> list[str]:
    return [_redact_text(part) for part in cmd]


def assert_no_report_path_leaks(report: dict[str, Any]) -> None:
    text = json.dumps(report, ensure_ascii=False, sort_keys=True)
    leaked = [pat for pat in _FORBIDDEN_REPORT_PATTERNS if pat in text]
    if _DRIVE_ABS_IN_JSON.search(text):
        leaked.append("drive-letter absolute path")
    if leaked:
        raise AssertionError(f"localisation gate report leaked path metadata: {sorted(set(leaked))}")


def _source_label(layers, abspath) -> str:
    return layers[R._layer_prio_fn(layers)(abspath)].label


def _is_loc_key_shaped(value: Any) -> bool:
    return isinstance(value, str) and bool(_LOC_KEY_TOKEN_RE.fullmatch(value))


def _literal_display_name(value: Any, token_kind: str | None) -> str | None:
    if token_kind != "str" or not isinstance(value, str):
        return None
    return None if _is_loc_key_shaped(value) else value


def _literal_from_record(rec: dict[str, Any], loc_key: Any) -> str | None:
    literal = _literal_display_name(loc_key, rec.get("name_token_kind"))
    if literal:
        return literal
    literal = rec.get("literal_name")
    return literal if isinstance(literal, str) and not _is_loc_key_shaped(literal) else None


def _loc_candidates(item: dict[str, Any]) -> list[dict[str, str]]:
    candidates = []
    for cand in item.get("loc_candidates") or []:
        key = cand.get("key")
        rule = cand.get("rule")
        if key and rule:
            candidates.append({"key": key, "rule": rule})
    loc_key = item.get("loc_key")
    if loc_key and not any(c["key"] == loc_key for c in candidates):
        candidates.insert(0, {"key": loc_key, "rule": item.get("loc_rule") or "loc_key"})
    return candidates


def _detail_with_candidates(item: dict[str, Any], reason: str, candidates: list[dict[str, str]]) -> dict[str, Any]:
    detail = {
        "id": item["id"],
        "loc_key": item.get("loc_key"),
        "reason": reason,
        "source_layer": item.get("source_layer"),
        "source_file": item.get("source_file"),
    }
    if candidates and len(candidates) <= 8:
        detail["loc_candidates"] = candidates
    elif candidates:
        detail["candidate_rules"] = sorted({cand["rule"] for cand in candidates})
    return detail


def _runtime_required_reason(item: dict[str, Any]) -> str | None:
    if item.get("runtime_required"):
        return item.get("reason") or "runtime_required"
    reason = item.get("reason")
    if reason == "dynamic_country_tag_requires_runtime_origin":
        return reason
    return None


def _quoted_key_missing_reason(item: dict[str, Any], loc_keys: set[str]) -> str | None:
    loc_key = item.get("loc_key")
    if (
        item.get("name_token_kind") == "str"
        and _is_loc_key_shaped(loc_key)
        and loc_key not in loc_keys
    ):
        return "quoted_key_missing_loc"
    literal_text = item.get("literal_name")
    if _is_loc_key_shaped(literal_text) and literal_text not in loc_keys:
        return "quoted_key_missing_loc"
    return None


def _coverage_entry(category: str, items: list[dict[str, Any]], loc_keys: set[str]) -> dict[str, Any]:
    unresolved = []
    runtime_required = []
    resolved = 0
    resolved_by_rule: dict[str, int] = {}
    candidate_rule_hits: dict[str, int] = {}
    resolved_by_literal = 0
    resolved_by_loc_key = 0
    for item in sorted(items, key=lambda r: r["id"]):
        candidates = _loc_candidates(item)
        runtime_reason = _runtime_required_reason(item)
        if runtime_reason:
            runtime_required.append(_detail_with_candidates(item, runtime_reason, candidates))
            continue
        hits = [cand for cand in candidates if cand["key"] in loc_keys]
        for hit in hits:
            candidate_rule_hits[hit["rule"]] = candidate_rule_hits.get(hit["rule"], 0) + 1
        if hits:
            resolved += 1
            resolved_by_loc_key += 1
            rule = hits[0]["rule"]
            resolved_by_rule[rule] = resolved_by_rule.get(rule, 0) + 1
            continue
        literal_text = item.get("literal_name")
        if literal_text:
            resolved += 1
            resolved_by_literal += 1
            rule = item.get("literal_rule") or "quoted_literal_name"
            resolved_by_rule[rule] = resolved_by_rule.get(rule, 0) + 1
            continue
        loc_key = item.get("loc_key")
        quoted_missing = _quoted_key_missing_reason(item, loc_keys)
        reason = "no_derivable_loc_key" if not candidates else "missing_localisation_key"
        unresolved.append(_detail_with_candidates(
            item,
            item.get("reason") or quoted_missing or reason,
            candidates,
        ))
    total_seen = len(items)
    static_resolvable_total = total_seen - len(runtime_required)
    out: dict[str, Any] = {
        "category": category,
        "total_seen": total_seen,
        "total": total_seen,
        "static_resolvable_total": static_resolvable_total,
        "resolved": resolved,
        "unresolved": len(unresolved),
        "resolved_by_loc_key": resolved_by_loc_key,
        "resolved_by_literal": resolved_by_literal,
        "resolved_by_rule": dict(sorted(resolved_by_rule.items())),
        "candidate_rule_hits": dict(sorted(candidate_rule_hits.items())),
        "loc_resolvable_percent": (
            round((resolved * 100.0 / static_resolvable_total), 2)
            if static_resolvable_total else 100.0
        ),
        "unresolvable": len(unresolved),
        "runtime_required": len(runtime_required),
        "missing_localisation_key": sum(
            1 for r in unresolved
            if r.get("loc_key") or r.get("loc_candidates") or r.get("candidate_rules")
        ),
        "quoted_key_missing_loc": sum(
            1 for r in unresolved
            if r.get("reason") == "quoted_key_missing_loc"
        ),
        "no_derivable_loc_key": sum(
            1 for r in unresolved
            if not r.get("loc_key") and not r.get("loc_candidates") and not r.get("candidate_rules")
        ),
    }
    out["unresolvable_details"] = unresolved
    out["runtime_required_details"] = runtime_required
    return out


def _state_items(layers) -> list[dict[str, Any]]:
    items = []
    for rel, abspath in R.ordered_files(layers, "history/states", (".txt",)):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[localisation_gate] WARN state {rel}: {e}", file=sys.stderr)
            continue
        label = _source_label(layers, abspath)
        for state in root.get_all("state"):
            if not isinstance(state, Node):
                continue
            sid = state.get("id")
            if not isinstance(sid, str) or not sid:
                continue
            items.append({
                "id": sid,
                "loc_key": f"STATE_{sid}",
                "source_layer": label,
                "source_file": rel,
            })
    return items


def _country_tag_items(layers) -> list[dict[str, Any]]:
    ideology_groups = _ideology_groups(layers)
    by_tag: dict[str, dict[str, Any]] = {}
    for rel, abspath in R.ordered_files(layers, "common/country_tags", (".txt",)):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[localisation_gate] WARN country_tags {rel}: {e}", file=sys.stderr)
            continue
        label = _source_label(layers, abspath)
        dynamic_section = False
        for tag, value in root.entries:
            if not tag or tag.startswith("@"):
                continue
            if tag.lower() == "dynamic_tags":
                dynamic_section = True
                continue
            if not isinstance(value, str):
                continue
            dynamic_tag = dynamic_section or bool(_DYNAMIC_COUNTRY_TAG_RE.fullmatch(tag))
            by_tag[tag] = {
                "id": tag,
                "loc_key": None,
                "loc_candidates": _country_tag_loc_candidates(tag, ideology_groups),
                "reason": "dynamic_country_tag_requires_runtime_origin" if dynamic_tag else None,
                "runtime_required": dynamic_tag,
                "dynamic_tag": dynamic_tag,
                "source_layer": label,
                "source_file": rel,
            }
    return [by_tag[tag] for tag in sorted(by_tag)]


def _ideology_groups(layers) -> list[str]:
    groups: set[str] = set()
    for rel, abspath in R.ordered_files(layers, "common/ideologies", (".txt",)):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[localisation_gate] WARN ideologies {rel}: {e}", file=sys.stderr)
            continue
        block = root.get("ideologies")
        if not isinstance(block, Node):
            continue
        for key, value in block.entries:
            if key and not key.startswith("@") and isinstance(value, Node):
                groups.add(key)
    return sorted(groups)


def _country_tag_loc_candidates(tag: str, ideology_groups: list[str]) -> list[dict[str, str]]:
    candidates = [{"key": tag, "rule": "bare_tag"}]
    for ideology in ideology_groups:
        candidates.append({"key": f"{tag}_{ideology}", "rule": "ideology_name"})
    candidates.extend([
        {"key": f"{tag}_DEF", "rule": "bare_def"},
        {"key": f"{tag}_ADJ", "rule": "bare_adj"},
    ])
    for ideology in ideology_groups:
        candidates.append({"key": f"{tag}_{ideology}_DEF", "rule": "ideology_def"})
        candidates.append({"key": f"{tag}_{ideology}_ADJ", "rule": "ideology_adj"})
    return candidates


# --- N4/E11 RESOLVED JOIN TABLES ------------------------------------------
#
# The coverage report above records HOW MANY ids resolve; the replay UI needs
# the actual id -> display-name lookups. These tables are the consumable form:
# focus_id -> name, state_id -> name, victory-point provid -> name, and country
# TAG -> name PLUS cosmetic-tag -> name (the live compact domain emits a
# country's active cosmetic_tag string; the replay joins it flat, falling back
# cosmetic_tags[active] ?? tags[base_tag] ?? tag). No fabrication: an id whose
# best loc candidate is absent from every effective file is simply OMITTED from
# the table (never guessed, never defaulted to the raw token). All mod-over-
# vanilla via the shared layer priority.


def _best_country_name(candidates: list[dict[str, str]], loc_names: dict[str, str]) -> str | None:
    """First candidate key (in bare -> ideology -> DEF/ADJ priority order) whose
    loc value exists is the display name; None if none resolve."""
    for cand in candidates:
        name = loc_names.get(cand["key"])
        if name:
            return name
    return None


def _cosmetic_tag_ids(layers) -> list[str]:
    """Cosmetic-tag ids from common/countries/cosmetic.txt (top-level blocks:
    ``COSMETIC_ID = { color = ... }``). Mod-over-vanilla union (a mod may add
    or replace cosmetic tags); a bare id with a Node body is a cosmetic tag."""
    ids: set[str] = set()
    for rel, abspath in R.ordered_files(layers, "common/countries", (".txt",)):
        if Path(rel).name != "cosmetic.txt":
            continue
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[localisation_gate] WARN cosmetic {rel}: {e}", file=sys.stderr)
            continue
        for key, value in root.entries:
            if key and not key.startswith("@") and isinstance(value, Node):
                ids.add(key)
    return sorted(ids)


def build_resolved_joins(layers, loc_out: dict[str, Any]) -> dict[str, Any]:
    """The consumable id -> display-name join tables (R3/N4)."""
    loc_names = {key: entry["name"] for key, entry in loc_out["keys"].items()}
    loc = R.load_loc(layers)
    ideology_groups = _ideology_groups(layers)

    # Focus: parse_focus_def already loc-resolves each focus title.
    focus_out = parse_focus_def.parse_focus_defs(layers, loc)
    focus = {
        fid: rec["loc"]["title"]
        for fid, rec in focus_out["focuses"].items()
        if isinstance(rec.get("loc"), dict) and rec["loc"].get("title")
    }

    # State + victory-point names (STATE_<id> / VICTORY_POINTS_<provid>).
    world_out = parse_world_defs.parse_world_defs(layers, loc)
    state = dict(world_out["state_names"])
    victory_point = dict(world_out["vp_names"])

    # Country base tags -> best-resolving name.
    tags: dict[str, str] = {}
    for item in _country_tag_items(layers):
        if item.get("dynamic_tag"):
            # dynamic country tags (D01..) get their origin name at runtime.
            continue
        name = _best_country_name(item.get("loc_candidates") or [], loc_names)
        if name:
            tags[item["id"]] = name

    # Cosmetic tags -> name (same candidate rules; the id itself is the loc key
    # shape, e.g. MAL_UK / MAL_UK_DEF / MAL_UK_ADJ + ideology variants).
    cosmetic_tags: dict[str, str] = {}
    for cid in _cosmetic_tag_ids(layers):
        name = _best_country_name(_country_tag_loc_candidates(cid, ideology_groups), loc_names)
        if name:
            cosmetic_tags[cid] = name

    return {
        "schema": "static_localisation_join_r3.resolved_joins.v1",
        "join_note": (
            "consumable id -> english display-name lookups, mod-over-vanilla; "
            "an id whose best loc candidate is absent is OMITTED (never faked). "
            "Country display: cosmetic_tags[active_cosmetic_tag] else "
            "tags[country_tag] else the raw tag; dynamic (D01..) tags resolve at runtime."
        ),
        "focus": dict(sorted(focus.items())),
        "state": dict(sorted(state.items(), key=lambda kv: int(kv[0]))),
        "victory_point": dict(sorted(victory_point.items(), key=lambda kv: int(kv[0]))),
        "country": {
            "tags": dict(sorted(tags.items())),
            "cosmetic_tags": dict(sorted(cosmetic_tags.items())),
        },
        "counts": {
            "focus": len(focus),
            "state": len(state),
            "victory_point": len(victory_point),
            "country_tags": len(tags),
            "cosmetic_tags": len(cosmetic_tags),
        },
    }


def _items_from_records(records: dict[str, dict[str, Any]], *, default_loc_key: bool = False) -> list[dict[str, Any]]:
    items = []
    for token_id, rec in records.items():
        loc_key = rec.get("loc_key")
        if default_loc_key and not loc_key:
            loc_key = token_id
        item = {
            "id": token_id,
            "loc_key": loc_key,
            "loc_rule": rec.get("loc_rule"),
            "literal_name": _literal_from_record(rec, loc_key),
            "literal_rule": None,
            "name_token_kind": rec.get("name_token_kind"),
            "source_layer": rec.get("source_layer"),
            "source_file": rec.get("source_file"),
        }
        if item["literal_name"]:
            item["literal_rule"] = "quoted_literal_name"
        if rec.get("enable_equipments") or rec.get("enable_equipment_modules"):
            candidates = []
            if loc_key:
                candidates.append({"key": loc_key, "rule": rec.get("loc_rule") or "loc_key"})
            candidates.extend(
                {"key": key, "rule": "resolved_via_equipment_loc"}
                for key in rec.get("enable_equipments", [])
            )
            candidates.extend(
                {"key": key, "rule": "resolved_via_equipment_loc"}
                for key in rec.get("enable_equipment_modules", [])
            )
            item["loc_candidates"] = candidates
        items.append(item)
    return items


def build_coverage_report(layers, loc_catalog_keys: dict[str, Any]) -> dict[str, Any]:
    loc_keys = set(loc_catalog_keys)
    focus_out = parse_focus_def.parse_focus_defs(layers, None)
    idea_tech_out = parse_idea_tech_loc.parse_idea_tech_loc_keys(layers)
    decisions_out = parse_decisions.parse_decisions(layers)
    catalogs_out = parse_catalogs.parse_catalogs(layers)
    characters_out = parse_characters.parse_characters(layers)

    idea_items = _items_from_records(idea_tech_out["tokens"]["ideas"])
    idea_items.extend({
        "id": r["id"],
        "loc_key": None,
        "reason": r["reason"],
        "source_layer": r.get("source_layer"),
        "source_file": r.get("source_file"),
    } for r in idea_tech_out["omitted"]["ideas"])

    tech_items = _items_from_records(idea_tech_out["tokens"]["technologies"])
    tech_items.extend({
        "id": r["id"],
        "loc_key": None,
        "reason": r["reason"],
        "source_layer": r.get("source_layer"),
        "source_file": r.get("source_file"),
    } for r in idea_tech_out["omitted"]["technologies"])

    decision_items = _items_from_records(decisions_out["decisions"])
    decision_items.extend({
        "id": r["id"],
        "loc_key": None,
        "reason": r["reason"],
        "source_layer": r.get("source_layer"),
        "source_file": r.get("source_file"),
    } for r in decisions_out["omitted"])

    focus_items = _items_from_records(focus_out["focuses"], default_loc_key=True)
    equipment_items = _items_from_records(
        catalogs_out["catalogs"].get("equipment_archetypes", {}),
        default_loc_key=True,
    )
    country_tag_items = _country_tag_items(layers)
    character_items = [
        {
            "id": cid,
            "loc_key": rec.get("name"),
            "literal_name": _literal_from_record(rec, rec.get("name")),
            "literal_rule": (
                "quoted_literal_name"
                if _literal_from_record(rec, rec.get("name"))
                else None
            ),
            "name_token_kind": rec.get("name_token_kind"),
            "reason": None if rec.get("name") else "missing_name_field",
            "source_layer": rec.get("source_layer"),
            "source_file": rec.get("source_file"),
        }
        for cid, rec in characters_out["characters"].items()
    ]

    categories = {
        "focus": _coverage_entry("focus", focus_items, loc_keys),
        "idea": _coverage_entry("idea", idea_items, loc_keys),
        "tech": _coverage_entry("tech", tech_items, loc_keys),
        "decision": _coverage_entry("decision", decision_items, loc_keys),
        "equipment": _coverage_entry("equipment", equipment_items, loc_keys),
        "state": _coverage_entry("state", _state_items(layers), loc_keys),
        "character": _coverage_entry("character", character_items, loc_keys),
        "country_tag": _coverage_entry("country_tag", country_tag_items, loc_keys),
    }
    categories["country_tag"]["derivation_rule"] = (
        "common/country_tags TAG tested against bare, ideology, DEF, ADJ, "
        "ideology_DEF, and ideology_ADJ keys derived from common/ideologies"
    )
    categories["country_tag"]["dynamic_runtime_required"] = sum(
        1 for item in country_tag_items if item.get("dynamic_tag")
    )
    return {
        "schema": "static_localisation_join_r3.token_coverage.v1",
        "categories": categories,
        "source_artifacts": {
            "focus": "parse_focus_def.focuses",
            "idea": "parse_idea_tech_loc.tokens.ideas",
            "tech": "parse_idea_tech_loc.tokens.technologies",
            "decision": "parse_decisions.decisions",
            "equipment": "parse_catalogs.catalogs.equipment_archetypes",
            "state": "history/states STATE_<id>",
            "character": "parse_characters.characters.name",
            "country_tag": "common/country_tags TAG + common/ideologies country-name suffix rules",
        },
    }


def build_catalog(vanilla: str | None, mods: list[str] | None) -> dict[str, Any]:
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    loc_out = parse_localisation.parse_localisation(layers)
    coverage = build_coverage_report(layers, loc_out["keys"])
    resolved_joins = build_resolved_joins(layers, loc_out)

    catalog = {
        "schema": "static_localisation_join_r3.static_artifact.v1",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "request": "R3",
        "scope": {
            "closed_rows": [
                "english localisation key -> {name, raw, source_layer, source_file}"
                " effective mod-over-vanilla join",
                # N4/E11: the consumable resolved id -> display-name join tables.
                "focus id -> display name (resolved_joins.focus)",
                "state id -> name (resolved_joins.state)",
                "victory-point province id -> name (resolved_joins.victory_point)",
                "country TAG -> name and cosmetic-tag -> name"
                " (resolved_joins.country.{tags,cosmetic_tags})",
            ],
            "not_closed_rows": [
                "$VAR$ / [Scope.Func] interpolation resolution (runtime job, not static)",
                "non-english languages (english only, per R3 scope)",
                "dynamic (D01..) country-tag names (runtime origin, not static)",
            ],
        },
        "layers": [
            {"label": ly.label, "root": redact_path(ly.root), "replace_path": list(ly.replace_paths)}
            for ly in layers
        ],
        "meta": loc_out["meta"],
        "coverage": coverage,
        # N4/E11: consumable id -> display-name join tables (additive; the
        # recorder hashes the whole file and reads only schema/layers, and
        # consumers read known keys — existing `keys`/`coverage` unchanged).
        "resolved_joins": resolved_joins,
        "keys": loc_out["keys"],
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog


def write_store(out_dir: Path, catalog: dict[str, Any]) -> dict[str, Path]:
    game_dir = out_dir / GAME_ID
    game_dir.mkdir(parents=True, exist_ok=True)
    catalog_bytes = json.dumps(catalog, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8")
    catalog_path = game_dir / "localisation_catalog.json"
    catalog_path.write_bytes(catalog_bytes)

    snapshot = {
        "schema_version": "static_localisation_join_r3.static_snapshot.v1",
        "provenance": {
            "mod": {
                "request": "R3",
                "artifact_sha256": sha256_bytes(catalog_bytes),
                "layers": catalog["layers"],
            }
        },
        "catalog_file": catalog_path.name,
        "catalog_sha256": sha256_bytes(catalog_bytes),
        "meta": catalog["meta"],
    }
    snapshot_bytes = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8")
    snapshot_path = game_dir / "snapshot_000000.json"
    snapshot_path.write_bytes(snapshot_bytes)

    index = {
        "schema": "static_localisation_join_r3.static_index.v1",
        "game_id": GAME_ID,
        "snapshots": [
            {
                "hour_index": 0,
                "file": snapshot_path.name,
                "role": "localisation_static_catalog_manifest",
                "sha256": sha256_bytes(snapshot_bytes),
            },
            {
                "hour_index": 0,
                "file": catalog_path.name,
                "role": "localisation_static_catalog_payload",
                "sha256": sha256_bytes(catalog_bytes),
            },
        ],
    }
    index_path = game_dir / "index.json"
    index_path.write_bytes(json.dumps(index, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8"))
    return {"game_dir": game_dir, "catalog": catalog_path, "snapshot": snapshot_path, "index": index_path}


def run_pack_verify(out_dir: Path) -> dict[str, Any]:
    replay_path = out_dir / f"{GAME_ID}.battleplan"
    pack_cmd_raw = [
        "cargo", "run", "--manifest-path", str(REPO_ROOT / "extractor/native" / "Cargo.toml"),
        "-p", "snapshot", "--bin", "pack_replay", "--",
        "pack", str(out_dir), GAME_ID, str(replay_path),
    ]
    verify_cmd_raw = [
        "cargo", "run", "--manifest-path", str(REPO_ROOT / "extractor/native" / "Cargo.toml"),
        "-p", "snapshot", "--bin", "pack_replay", "--",
        "verify", str(replay_path), str(out_dir), GAME_ID,
    ]
    pack = subprocess.run(pack_cmd_raw, cwd=REPO_ROOT, text=True, capture_output=True)
    verify = subprocess.run(verify_cmd_raw, cwd=REPO_ROOT, text=True, capture_output=True) if pack.returncode == 0 else None
    return {
        "replay_path": redact_path(replay_path),
        "pack": {
            "cmd": _redact_cmd(pack_cmd_raw),
            "returncode": pack.returncode,
            "stdout": _redact_text(pack.stdout),
            "stderr": _redact_text(pack.stderr),
        },
        "verify": None if verify is None else {
            "cmd": _redact_cmd(verify_cmd_raw),
            "returncode": verify.returncode,
            "stdout": _redact_text(verify.stdout),
            "stderr": _redact_text(verify.stderr),
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--vanilla", default=None, help="the game install dir (else BATTLE_PLAN_VANILLA env).")
    ap.add_argument("--mods", default=None, help="comma-separated .mod tokens (else active_modset.json).")
    ap.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--skip-pack", action="store_true")
    args = ap.parse_args()
    mods = args.mods.split(",") if args.mods else None

    catalog = build_catalog(args.vanilla, mods)
    paths = write_store(args.out_dir, catalog)

    joins = catalog["resolved_joins"]["counts"]
    proof: dict[str, Any] = {
        "schema": "static_localisation_join_r3.static_gate_report.v1",
        "request": "R3",
        "passed": False,
        "paths": {key: redact_path(path) for key, path in paths.items()},
        "meta": catalog["meta"],
        "coverage": catalog["coverage"],
        "resolved_joins_counts": joins,
        "artifact_sha256": catalog["artifact_sha256"],
        "pack_verify": None,
    }
    if not args.skip_pack:
        proof["pack_verify"] = run_pack_verify(args.out_dir)
    verify_rc = ((proof.get("pack_verify") or {}).get("verify") or {}).get("returncode")
    # Fail closed if the resolved-join tables came back implausibly empty (a
    # broken parser/loc join must NOT ship a hollow bundle that passed only on
    # the raw key count). Vanilla has hundreds of focuses/states/VPs/tags.
    joins_ok = (
        joins["focus"] > 100
        and joins["state"] > 100
        and joins["victory_point"] > 100
        and joins["country_tags"] > 100
    )
    proof["resolved_joins_ok"] = joins_ok
    proof["passed"] = bool(
        catalog["meta"]["key_count"] > 10000
        and joins_ok
        and (args.skip_pack or verify_rc == 0)
    )
    assert_no_report_path_leaks(proof)
    proof_path = args.out_dir / "localisation_gate_report.json"
    proof_path.parent.mkdir(parents=True, exist_ok=True)
    proof_path.write_text(json.dumps(proof, ensure_ascii=False, sort_keys=True, indent=2), encoding="utf-8")
    print(json.dumps(
        {k: v for k, v in proof.items() if k != "pack_verify"} |
        {"pack_verify_summary": None if proof["pack_verify"] is None else {
            "pack_rc": proof["pack_verify"]["pack"]["returncode"],
            "verify_rc": verify_rc,
        }},
        ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if proof["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
