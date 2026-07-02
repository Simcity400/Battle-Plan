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
from paradox_parser import Node


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
    return v if isinstance(v, str) else None


def _field_present(node: Node, field: str) -> bool:
    fl = field.lower()
    return any(k.lower() == fl for k, _v in node.entries if k)


def _loc_key(decision_id: str, node: Node):
    if _field_present(node, "name"):
        name = _scalar(node.get("name"))
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


def _parse_categories(layers):
    categories: dict[str, dict] = {}
    collisions = []
    files_parsed = 0
    for rel, abspath in R.ordered_files(layers, "common/decisions/categories", (".txt",)):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[decisions] WARN category {rel}: {e}", file=sys.stderr)
            continue
        files_parsed += 1
        label = _source_label(layers, abspath)
        for cid, body in root.entries:
            if not isinstance(body, Node) or not cid:
                continue
            rec = {
                "id": cid,
                "icon": _scalar(body.get("icon")),
                "picture": _scalar(body.get("picture")),
                "source_layer": label,
                "source_file": rel,
            }
            if cid in categories and categories[cid]["source_file"] != rel:
                collisions.append({
                    "id": cid,
                    "first": categories[cid]["source_file"],
                    "override": rel,
                })
            categories[cid] = rec
    return categories, collisions, files_parsed


def _parse_decision_files(layers):
    decisions: dict[str, dict] = {}
    omitted = []
    collisions = []
    files_parsed = 0
    explicit_names = 0
    for rel, abspath in _direct_files(layers, "common/decisions"):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[decisions] WARN {rel}: {e}", file=sys.stderr)
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
                    "explicit_name": explicit_name,
                    "icon": _scalar(body.get("icon")),
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


def parse_decisions(layers) -> dict:
    categories, category_collisions, category_files = _parse_categories(layers)
    decisions, omitted, collisions, decision_files, explicit_names = _parse_decision_files(layers)
    return {
        "meta": {
            "decision_count": len(decisions) + len(omitted),
            "decision_mapped": len(decisions),
            "decision_omitted_no_loc_key": len(omitted),
            "decision_explicit_names": explicit_names,
            "category_count": len(categories),
            "decision_files_parsed": decision_files,
            "category_files_parsed": category_files,
            "id_collisions_last_wins": collisions,
            "category_collisions_last_wins": category_collisions,
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
