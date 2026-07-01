#!/usr/bin/env python3
"""the game save-file FULL PROVINCE CONTROL oracle.

Reconstructs the *complete* per-province controller map (and per-state
owner/controller) from a melted the game save. Province control in the
save is SPARSE — only ~hundreds of provinces and ~tens of states carry an
explicit override — so this module densifies it with the documented fallback:

    state_controller[state]  = states.<id>.controller  if present
                               else states.<id>.owner
    province_controller[prov] = provinces.<id>.controller          if present
                                else state_controller[ province_to_state[prov] ]

Outputs (under ``output/``):
  - ``province_control.json`` : ``{province_id: controller_TAG_or_null}`` for
    EVERY province in the province->state map.
  - ``state_control.json``    : ``{state_id: {"owner","controller"}}`` (controller
    falls back to owner when the save has no state-level override).

This is the bootstrap-invariant / oracle layer for the separate live memory
reader: the memory reader can be validated against this dense control map.

stdlib only. Reuses ``parse_save.py`` for the actual save parsing (its tolerant
tokenizer + state/province section walkers); this module only adds the
reconstruction + the province->state join.

Usage:
    python parse_control.py [save_or_melted_path]
    python parse_control.py --rakaly /path/to/rakaly.exe --out-dir <dir> [save]
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import parse_save as ps


def _load_province_to_state(script_dir: Path):
    """Load map_pipeline/assets/province_to_state.json -> {int prov: int state}.

    Returns (mapping, path_or_None). Read-only and required for a full province
    map; without it we can only emit the explicit province overrides.
    """
    p = (script_dir.parent / "map_pipeline" / "assets"
         / "province_to_state.json")
    if not p.is_file():
        return {}, None
    raw = json.loads(p.read_text(encoding="utf-8"))
    mapping = {}
    for k, v in raw.items():
        try:
            mapping[int(k)] = int(v)
        except (TypeError, ValueError):
            continue
    return mapping, p


def reconstruct_control(result: dict, prov_to_state: dict):
    """Build dense state + province control from a parsed-save ``result``.

    ``result`` is the dict returned by ``parse_save.parse_save_text``:
      - ``result["states"][sid]``  = {"owner","controller"(raw or None), ...}
      - ``result["provinces"][pid]`` = controller TAG (explicit overrides only)

    Returns (state_control, province_control, stats).
    """
    states = result["states"]
    prov_overrides = result["provinces"]  # {pid_str: TAG} explicit only

    # --- state control: controller override else owner --------------------
    state_control: dict = {}
    raw_state_ctrl_overrides = 0
    for sid, info in states.items():
        owner = info.get("owner")
        raw_ctrl = info.get("controller")
        if raw_ctrl:
            raw_state_ctrl_overrides += 1
        controller = raw_ctrl or owner
        state_control[sid] = {"owner": owner, "controller": controller}

    # state controller lookup keyed by int state id for the province join.
    state_ctrl_by_int: dict = {}
    for sid, sc in state_control.items():
        try:
            state_ctrl_by_int[int(sid)] = sc["controller"]
        except (TypeError, ValueError):
            continue

    # province override lookup keyed by int.
    prov_override_by_int: dict = {}
    for pid, tag in prov_overrides.items():
        try:
            prov_override_by_int[int(pid)] = tag
        except (TypeError, ValueError):
            continue

    # --- province control: override else state's controller --------------
    province_control: dict = {}
    resolved = 0
    via_override = 0
    via_state = 0
    unresolved = []  # (prov, state) pairs with no resolvable controller
    no_state_map = 0  # provinces present in map but with state not in save

    for pid, sid in prov_to_state.items():
        tag = prov_override_by_int.get(pid)
        if tag:
            via_override += 1
        else:
            tag = state_ctrl_by_int.get(sid)
            if tag:
                via_state += 1
            else:
                no_state_map += 1
                unresolved.append((pid, sid))
        # Emit null for unresolved (sea/lake/wasteland/no-owner) so downstream
        # has an entry for every mapped province; the count distinguishes them.
        province_control[str(pid)] = tag
        if tag:
            resolved += 1

    # Province overrides that fall OUTSIDE the province->state map (rare; e.g.
    # the engine wrote a controller for a province not in our generated map).
    extra_overrides = 0
    for pid, tag in prov_override_by_int.items():
        if pid not in prov_to_state:
            province_control[str(pid)] = tag
            extra_overrides += 1
            if tag:
                resolved += 1

    stats = {
        "states": len(state_control),
        "states_with_owner": sum(
            1 for s in state_control.values() if s["owner"]),
        "raw_state_controller_overrides": raw_state_ctrl_overrides,
        "province_overrides_in_save": len(prov_override_by_int),
        "provinces_in_map": len(prov_to_state),
        "provinces_emitted": len(province_control),
        "provinces_with_controller": resolved,
        "provinces_via_override": via_override,
        "provinces_via_state_fallback": via_state,
        "provinces_unresolved": len(unresolved),
        "province_overrides_outside_map": extra_overrides,
    }
    return state_control, province_control, stats, unresolved


def validation_report(state_control, province_control, stats, unresolved,
                      meta) -> str:
    lines = []
    lines.append("=== PROVINCE-CONTROL ORACLE VALIDATION ===")
    lines.append(f"source        : {meta['source_filename']} "
                 f"(melted={meta['was_melted']})")
    lines.append(f"save date     : {meta['save_date']}  "
                 f"version={meta['save_version']}")
    lines.append(f"states         : {stats['states']} "
                 f"({stats['states_with_owner']} with owner)")
    lines.append(f"raw state ctrl : {stats['raw_state_controller_overrides']} "
                 f"explicit state-level controller overrides (~83 expected)")
    lines.append(f"prov overrides : {stats['province_overrides_in_save']} "
                 f"explicit province controller overrides in save")
    lines.append(f"prov in map    : {stats['provinces_in_map']}")
    lines.append(f"prov emitted   : {stats['provinces_emitted']}")
    lines.append(f"prov w/ ctrl   : {stats['provinces_with_controller']} "
                 f"(via override={stats['provinces_via_override']}, "
                 f"via state={stats['provinces_via_state_fallback']})")
    lines.append(f"prov unresolved: {stats['provinces_unresolved']} "
                 f"(sea/lake/wasteland/no-owner -> null)")
    if stats["province_overrides_outside_map"]:
        lines.append(f"overrides outside map: "
                     f"{stats['province_overrides_outside_map']}")

    # Spot-check: occupied territory (controller != owner) at state level.
    contested = [(sid, sc) for sid, sc in state_control.items()
                 if sc["owner"] and sc["controller"]
                 and sc["controller"] != sc["owner"]]
    lines.append(f"contested states (controller != owner): {len(contested)}")
    lines.append("sample contested states (owner -> controller):")
    for sid, sc in contested[:6]:
        lines.append(f"   state {sid:>5}: {sc['owner']} -> {sc['controller']}")

    # A couple unresolved samples (should be sea/wasteland).
    if unresolved:
        lines.append("sample unresolved provinces (prov -> state):")
        for pid, sid in unresolved[:5]:
            lines.append(f"   prov {pid:>6} -> state {sid}")
    return "\n".join(lines)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(
        description="the game save -> full province control + state control "
                    "(oracle layer).")
    ap.add_argument("save", nargs="?", default=None)
    ap.add_argument("--rakaly", default=None)
    ap.add_argument("--out-dir", default=None)
    args = ap.parse_args(argv)

    script_dir = Path(__file__).resolve().parent
    out_dir = Path(args.out_dir) if args.out_dir else script_dir / "output"
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.save:
        save_path = Path(args.save)
        if not save_path.exists():
            print(f"ERROR: save not found: {save_path}", file=sys.stderr)
            return 2
    else:
        save_path = ps.find_latest_save()
        if save_path is None:
            print("ERROR: no candidate save found in known locations.",
                  file=sys.stderr)
            return 2

    print(f"[control] resolved save : {ps._scrub(str(save_path))}")
    rakaly = ps.find_rakaly(args.rakaly)
    print(f"[control] rakaly        : "
          f"{ps._scrub(str(rakaly)) if rakaly else 'NOT FOUND'}")

    try:
        text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 3

    print(f"[control] melted        : {was_melted}  "
          f"({len(text):,} chars to parse)")
    result = ps.parse_save_text(text)

    prov_to_state, ptos_path = _load_province_to_state(script_dir)
    if ptos_path is None:
        print("[control] WARNING: province_to_state.json not found; only the "
              "explicit province overrides can be emitted (no full map).",
              file=sys.stderr)
    else:
        print(f"[control] prov->state   : {ps._scrub(str(ptos_path))} "
              f"({len(prov_to_state)} provinces)")

    state_control, province_control, stats, unresolved = reconstruct_control(
        result, prov_to_state)

    meta = {
        "source_filename": save_path.name,
        "source_path_redacted": ps._redact(save_path),
        "was_melted": was_melted,
        "save_date": result["meta"].get("date"),
        "save_version": result["meta"].get("version"),
        "stats": stats,
    }

    (out_dir / "state_control.json").write_text(
        json.dumps(state_control, indent=1), encoding="utf-8")
    (out_dir / "province_control.json").write_text(
        json.dumps(province_control, indent=1), encoding="utf-8")
    (out_dir / "control_meta.json").write_text(
        json.dumps(meta, indent=2), encoding="utf-8")

    report = validation_report(
        state_control, province_control, stats, unresolved, meta)
    print()
    print(report)
    print()
    print(f"[control] wrote: {ps._scrub(str(out_dir / 'state_control.json'))}")
    print(f"[control] wrote: "
          f"{ps._scrub(str(out_dir / 'province_control.json'))}")
    print(f"[control] wrote: {ps._scrub(str(out_dir / 'control_meta.json'))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
