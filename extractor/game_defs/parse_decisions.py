#!/usr/bin/env python3
"""DECISIONS static-def parser.

Parses the effective mod-over-vanilla decision files into the decision-token
catalog the replay UI needs:

  common/decisions/categories/*.txt -> category id, icon/picture/source
  common/decisions/*.txt            -> decision id, category, loc key, icon/source

Decision loc-key rule is definition-side and fail-closed: an explicit scalar
``name = <loc_key>`` wins; if ``name`` is absent, the decision id is the loc key;
if ``name`` exists but is not scalar, the decision is omitted and counted.

stdlib only (+ resolver, paradox_parser).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node, parse
from static_artifact_common import (
    boolish,
    field_values,
    first_scalar,
    node_to_data,
    put_present,
    scalar,
    single_or_many,
    source_inventory,
    validate_clausewitz_text,
)


_CATEGORY_FIELDS = {
    "activation",
    "ai_check_interval",
    "allowed",
    "available",
    "cancel_if_not_visible",
    "complete_effect",
    "cost",
    "country_target",
    "custom_cost_text",
    "custom_cost_trigger",
    "custom_trigger_tooltip",
    "days_mission_timeout",
    "days_re_enable",
    "days_remove",
    "do_not_show_alert",
    "fire_only_once",
    "highlight_state_targets",
    "highlight_states",
    "icon",
    "is_good",
    "map_area",
    "modifier",
    "on_map_area",
    "picture",
    "priority",
    "remove_effect",
    "remember_last_selection",
    "scripted_gui",
    "sort_order",
    "state_target",
    "target_array",
    "target_non_existing",
    "target_root_trigger",
    "target_trigger",
    "targeted_decision",
    "timeout_effect",
    "use_small_name",
    "visibility_type",
    "visible",
    "war_support_modifier",
}


def _scalar(v):
    return scalar(v)


def _raw_field(node: Node, field: str):
    return single_or_many(field_values(node, field))


def _field_present(node: Node, field: str) -> bool:
    fl = field.lower()
    return any(k.lower() == fl for k, _v in node.entries if k)


def _loc_key(decision_id: str, node: Node):
    if _field_present(node, "name"):
        names = node.get_all("name")
        if len(names) > 1:
            return None, False, "duplicate_name"
        name = _scalar(names[0]) if names else None
        if name:
            return name, True, None
        return None, False, "non_scalar_name"
    return decision_id, False, None


def _source_label(layers, abspath) -> str:
    return layers[R._layer_prio_fn(layers)(abspath)].label


def _direct_files(layers, subdir: str):
    for rel, abspath in R.ordered_files(layers, subdir, (".txt",)):
        if Path(rel).parent.as_posix() == subdir:
            yield rel, abspath


def _read_clausewitz(rel: str, abspath: Path, *, strict: bool, warnings: list[str]) -> Node | None:
    try:
        text = R.read_text(abspath)
        validate_clausewitz_text(text, rel)
        return parse(text)
    except Exception as e:
        msg = f"{rel}: {e}"
        if strict:
            raise RuntimeError(f"failed to parse decisions file {msg}") from e
        warnings.append(msg)
        print(f"[decisions] WARN {msg}", file=sys.stderr)
        return None


def _target_metadata(body: Node) -> dict:
    country_target_raw = first_scalar(body, "country_target")
    state_target_raw = first_scalar(body, "state_target")

    def declared_target(raw: str | None) -> bool | None:
        if raw is None:
            return None
        parsed = boolish(raw)
        return True if parsed is None else parsed

    country_target = declared_target(country_target_raw)
    state_target = declared_target(state_target_raw)
    out = {
        "has_target_trigger": body.get("target_trigger") is not None,
        "has_target_root_trigger": body.get("target_root_trigger") is not None,
        "has_highlight_state_targets": body.get("highlight_state_targets") is not None,
        "has_highlight_states": body.get("highlight_states") is not None,
    }
    put_present(out, "targeted_decision", boolish(first_scalar(body, "targeted_decision")))
    put_present(out, "country_target", country_target)
    put_present(out, "state_target", state_target)
    if country_target_raw is not None and boolish(country_target_raw) is None:
        out["country_target_selector"] = country_target_raw
    if state_target_raw is not None and boolish(state_target_raw) is None:
        out["state_target_selector"] = state_target_raw
    put_present(out, "target_non_existing", boolish(first_scalar(body, "target_non_existing")))
    put_present(out, "target_array", first_scalar(body, "target_array"))
    put_present(out, "target_scope", "country" if country_target else "state" if state_target else None)
    put_present(out, "target_trigger", _raw_field(body, "target_trigger"))
    put_present(out, "target_root_trigger", _raw_field(body, "target_root_trigger"))
    put_present(out, "highlight_state_targets", _raw_field(body, "highlight_state_targets"))
    put_present(out, "highlight_states", _raw_field(body, "highlight_states"))
    return out


def _cooldown_metadata(body: Node) -> dict:
    out = {
        "remove_effect_present": body.get("remove_effect") is not None,
        "timeout_effect_present": body.get("timeout_effect") is not None,
        "complete_effect_present": body.get("complete_effect") is not None,
    }
    put_present(out, "days_re_enable", _raw_field(body, "days_re_enable"))
    put_present(out, "days_remove", _raw_field(body, "days_remove"))
    put_present(out, "days_mission_timeout", _raw_field(body, "days_mission_timeout"))
    put_present(out, "activation", _raw_field(body, "activation"))
    return out


def _visibility_metadata(body: Node) -> dict:
    out = {
        "has_visible": body.get("visible") is not None,
        "has_available": body.get("available") is not None,
        "has_allowed": body.get("allowed") is not None,
        "has_cancel_if_not_visible": body.get("cancel_if_not_visible") is not None,
    }
    put_present(out, "visible", _raw_field(body, "visible"))
    put_present(out, "available", _raw_field(body, "available"))
    put_present(out, "allowed", _raw_field(body, "allowed"))
    put_present(out, "cancel_if_not_visible", _raw_field(body, "cancel_if_not_visible"))
    return out


def _cost_metadata(body: Node) -> dict:
    out = {
        "has_custom_cost_trigger": body.get("custom_cost_trigger") is not None,
    }
    put_present(out, "political_power", _raw_field(body, "cost"))
    put_present(out, "custom_cost_text", first_scalar(body, "custom_cost_text"))
    put_present(out, "custom_cost_trigger", _raw_field(body, "custom_cost_trigger"))
    return out


def _parse_categories(layers, *, strict: bool, warnings: list[str]):
    categories: dict[str, dict] = {}
    collisions = []
    files_parsed = 0
    for rel, abspath in R.ordered_files(layers, "common/decisions/categories", (".txt",)):
        root = _read_clausewitz(f"category {rel}", abspath, strict=strict, warnings=warnings)
        if root is None:
            continue
        files_parsed += 1
        label = _source_label(layers, abspath)
        for cid, body in root.entries:
            if not isinstance(body, Node) or not cid:
                continue
            rec = {
                "id": cid,
                "icon": first_scalar(body, "icon"),
                "picture": first_scalar(body, "picture"),
                "has_visible": body.get("visible") is not None,
                "has_available": body.get("available") is not None,
                "has_allowed": body.get("allowed") is not None,
                "source_layer": label,
                "source_file": rel,
            }
            put_present(rec, "visible", _raw_field(body, "visible"))
            put_present(rec, "available", _raw_field(body, "available"))
            put_present(rec, "allowed", _raw_field(body, "allowed"))
            if cid in categories and categories[cid]["source_file"] != rel:
                collisions.append({
                    "id": cid,
                    "first": categories[cid]["source_file"],
                    "override": rel,
                })
            categories[cid] = rec
    return categories, collisions, files_parsed


def _parse_decision_files(layers, categories: dict[str, dict], *, strict: bool, warnings: list[str]):
    decisions: dict[str, dict] = {}
    omitted = []
    collisions = []
    files_parsed = 0
    explicit_names = 0
    for rel, abspath in _direct_files(layers, "common/decisions"):
        root = _read_clausewitz(rel, abspath, strict=strict, warnings=warnings)
        if root is None:
            continue
        files_parsed += 1
        label = _source_label(layers, abspath)
        for category, catbody in root.entries:
            if not isinstance(catbody, Node) or not category or category.startswith("@"):
                continue
            for did, body in catbody.entries:
                if not isinstance(body, Node) or not did:
                    continue
                if did.startswith("@") or did.lower() in _CATEGORY_FIELDS:
                    continue
                loc_key, explicit_name, reason = _loc_key(did, body)
                if reason:
                    omitted.append({
                        "id": did,
                        "category": category,
                        "reason": reason,
                        "source_layer": label,
                        "source_file": rel,
                    })
                    continue
                rec = {
                    "id": did,
                    "category": category,
                    "loc_key": loc_key,
                    "loc_rule": "explicit_name" if explicit_name else "id",
                    "explicit_name": explicit_name,
                    "icon": first_scalar(body, "icon"),
                    "category_icon": (categories.get(category) or {}).get("icon"),
                    "resolved_icon": first_scalar(body, "icon") or (categories.get(category) or {}).get("icon"),
                    "picture": first_scalar(body, "picture"),
                    "fire_only_once": boolish(first_scalar(body, "fire_only_once")),
                    "do_not_show_alert": boolish(first_scalar(body, "do_not_show_alert")),
                    "is_good": boolish(first_scalar(body, "is_good")),
                    "sort_order": first_scalar(body, "sort_order"),
                    "priority": first_scalar(body, "priority"),
                    "cost": _cost_metadata(body),
                    "targets": _target_metadata(body),
                    "cooldowns": _cooldown_metadata(body),
                    "visibility": _visibility_metadata(body),
                    "source_layer": label,
                    "source_file": rel,
                }
                if explicit_name:
                    explicit_names += 1
                if did in decisions and decisions[did]["source_file"] != rel:
                    collisions.append({
                        "id": did,
                        "first": decisions[did]["source_file"],
                        "override": rel,
                    })
                decisions[did] = rec
    return decisions, omitted, collisions, files_parsed, explicit_names


def parse_decisions(layers, *, strict: bool = True) -> dict:
    warnings: list[str] = []
    categories, category_collisions, category_files = _parse_categories(
        layers,
        strict=strict,
        warnings=warnings,
    )
    decisions, omitted, collisions, decision_files, explicit_names = _parse_decision_files(
        layers,
        categories,
        strict=strict,
        warnings=warnings,
    )
    decision_source_files = list(_direct_files(layers, "common/decisions"))
    category_source_files = R.ordered_files(layers, "common/decisions/categories", (".txt",))
    return {
        "meta": {
            "decision_count": len(decisions) + len(omitted),
            "decision_mapped": len(decisions),
            "decision_omitted_no_loc_key": len(omitted),
            "decision_explicit_names": explicit_names,
            "category_count": len(categories),
            "decision_files_parsed": decision_files,
            "category_files_parsed": category_files,
            "decision_files_total": len(decision_source_files),
            "category_files_total": len(category_source_files),
            "warnings": warnings,
            "id_collisions_last_wins": collisions,
            "category_collisions_last_wins": category_collisions,
        },
        "source_files": {
            "decision_files": source_inventory(decision_source_files),
            "category_files": source_inventory(category_source_files),
        },
        "decisions": dict(sorted(decisions.items())),
        "categories": dict(sorted(categories.items())),
        "omitted": sorted(omitted, key=lambda r: r["id"]),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_decisions(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "decisions.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(
        f"decisions: {m['decision_mapped']}/{m['decision_count']} decisions "
        f"({m['decision_explicit_names']} explicit names), "
        f"{m['category_count']} categories -> {outp.name}"
    )
