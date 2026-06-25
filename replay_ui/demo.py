"""Synthetic multi-snapshot demo store for the replay UI.

Generates a tiny but *schema-valid* 3-snapshot replay so the whole 5-screen UI is
exercisable before a real multi-save replay exists. It writes into
``replay_ui/demo_snapshots/<game_id>/`` (NOT replay/snapshots — boundary: this
agent writes only under replay_ui/). The UI server is told this alternate store
root so the very same fetch/index/snapshot code path is exercised (Codex #4).

It deliberately includes the no-fabrication edge cases so the UI's "not captured"
handling is visible without a real save:
  * a frame with ``provenance.memory`` populated (so memory-tagged fields are
    captured) AND a frame backed only by ``save`` (memory == null);
  * ``casualties`` populated on one country, ``{}`` (not captured) on another;
  * ``party_popularities``/``gross_ic_by_equipment`` null on some;
  * a couple of M* combats with inferred fields and missing outcome;
  * divisions with org/strength but null movement/in_combat (M*).

The events timeline is the real diff-engine output over the 3 frames, so the
Event-timeline screen shows genuine derived events with correct source/confidence.
"""
from __future__ import annotations

import os
import sys
from typing import Any, Dict, List

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
if REPO not in sys.path:
    sys.path.insert(0, REPO)

from replay import schema  # read-only
from replay.store import SnapshotStore
from replay.diff_engine import build_timeline

DEMO_GAME_ID = "demo"
DEMO_STORE = os.path.join(HERE, "demo_snapshots")

# Three dates a few weeks apart (Clausewitz Y.M.D.H).
_DATES = ["1939.9.1.12", "1939.9.15.12", "1939.10.1.12"]

# A handful of tags with plausible map colours present in country_colors.json.
_TAGS = ["GER", "ENG", "FRA", "POL", "ITA"]


def _country(tag: str, frame: int, *, with_memory: bool) -> Dict[str, Any]:
    c = schema.empty_country()
    grow = frame  # frame index 0..2 used to vary scalars
    base = {"GER": 60, "ENG": 55, "FRA": 45, "POL": 18, "ITA": 40}[tag]
    c["economy"]["factories"] = {
        "civ": base + grow * 2, "mil": base // 2 + grow * 3,
        "dockyard": 10 + (tag == "ENG") * 20, "under_construction": 4 + grow,
        "offmap": {"civ": 0, "mil": 0, "dockyard": 0},
    }
    # gross_ic_by_equipment: present only for GER, null elsewhere (not captured)
    c["economy"]["gross_ic_by_equipment"] = (
        {"infantry_equipment": 120.0 + grow * 30} if tag == "GER" else None)
    c["economy"]["fuel"] = {"current": 5000.0 + grow * 100 if with_memory else None,
                            "max": 10000.0}
    c["economy"]["resources"] = {"produced": {"steel": 80 + grow, "oil": 12}}

    mil = c["military"]
    mil["num_ships"] = 30 + (tag == "ENG") * 50 + grow
    mil["division_count"] = 24 + grow * 4 + (tag == "GER") * 20
    mil["manpower"] = {"available": 400000 - grow * 25000}
    mil["command_power"] = 100.0 + grow * 10 if with_memory else None
    # casualties: GER/POL get them (they are at war); others stay {} = not captured
    if tag in ("GER", "POL") and frame > 0:
        mil["casualties"] = {"killed": frame * (1500 if tag == "POL" else 800),
                             "captured": frame * 200}
    mil["equipment_stockpile"] = [
        {"archetype": "infantry_equipment_1", "equip_id": 1, "equip_type": 70,
         "amount": 2000.0 + grow * 400},
        {"archetype": "artillery_equipment_1", "equip_id": 2, "equip_type": 70,
         "amount": 150.0 + grow * 30},
    ]

    pol = c["politics"]
    # stability/war_support/pp/cp are MEMORY-tagged: only present when memory captured
    pol["stability"] = round(0.70 + grow * 0.02, 3) if with_memory else None
    pol["war_support"] = round(0.55 + grow * 0.05, 3) if with_memory else None
    pol["political_power"] = round(120.0 + grow * 40, 1) if with_memory else None
    pol["command_power"] = mil["command_power"]
    pol["ruling_party"] = {"GER": "fascism", "ITA": "fascism"}.get(tag, "democratic")
    pol["laws"] = {"economy": "war_economy" if tag in ("GER", "POL") else "civilian_economy",
                   "trade": "export_focus", "conscription": "limited_conscription"}
    pol["ideas_all"] = ["war_economy", "limited_conscription", f"{tag}_spirit_demo"]
    pol["ideas_other"] = [f"{tag}_spirit_demo"]
    # party_popularities null for everyone except GER (shows "not captured")
    pol["party_popularities"] = ({"fascism": 0.6, "democratic": 0.3, "communism": 0.1}
                                 if tag == "GER" else None)
    pol["is_human"] = tag in ("GER", "ENG", "FRA")
    pol["is_major"] = tag in ("GER", "ENG", "FRA", "ITA")
    pol["capital_state"] = {"GER": 64, "ENG": 126, "FRA": 16, "POL": 9, "ITA": 2}[tag]

    res = c["research"]
    res["tech_count"] = 80 + grow * 6 + (tag == "GER") * 15
    res["research_slots"] = 3 + (tag in ("GER", "ENG", "USA"))
    res["technologies"] = [
        {"name": "infantry_weapons", "level": 1 + grow, "date": _DATES[0],
         "research_points": 120.0},
        {"name": "fleet_in_being", "level": 1, "date": _DATES[min(frame, 2)],
         "research_points": None},
    ]
    res["in_progress"] = [{"name": "infantry_weapons", "points_factor": 1.1}]
    res["research_bonuses"] = [{"name": "doctrine_bonus", "categories": ["land_doctrine"],
                                "bonus": 0.5, "kind": "research_bonus"}]

    fc = c["focus"]
    fc["focus_tree"] = f"{tag.lower()}_focus"
    completed = ["focus_a", "focus_b"][:1 + frame]
    fc["completed"] = [f"{tag}_{f}" for f in completed]
    fc["completed_count"] = len(fc["completed"])
    fc["current"] = f"{tag}_focus_c" if frame < 2 else None
    fc["progress"] = (frame * 20 if with_memory else None) if frame < 2 else None
    fc["paused"] = False
    return c


def _divisions(frame: int) -> List[Dict[str, Any]]:
    """A few divisions per major; one moves and one is destroyed across frames."""
    out: List[Dict[str, Any]] = []
    plan = [
        # (div_id, type, owner, prov0, prov1, prov2, name, role)
        (1, 7, "GER", 9, 9, 9, "Infanterie-Division", "infantry"),
        (2, 7, "GER", 64, 50, 50, "Panzer-Division", "armor"),
        (3, 7, "POL", 9, 9, None, "Piechota", "infantry"),     # destroyed by frame 2
        (4, 7, "FRA", 16, 16, 16, "Division d'Infanterie", "infantry"),
        (5, 7, "ENG", 126, 126, 126, "Infantry Division", "infantry"),
    ]
    for did, dtype, owner, p0, p1, p2, name, role in plan:
        prov = [p0, p1, p2][frame]
        if prov is None:
            continue  # destroyed / not yet present
        d = schema.empty_division()
        d["div_id"] = did
        d["div_type"] = dtype
        d["owner"] = owner
        d["location_province"] = prov
        d["state_id"] = {9: 9, 64: 64, 50: 51, 16: 16, 126: 126}.get(prov, prov)
        d["template_id"] = 100 + did
        d["template_type"] = 9
        d["template_name"] = name
        d["template_role"] = role
        d["organisation"] = round(50.0 - frame * 5 + did, 2)
        d["strength"] = round(100.0 - frame * 8, 1)
        d["equipment"] = [{"archetype": "infantry_equipment_1", "equip_id": 1,
                           "equip_type": 70, "amount": 900.0 - frame * 40}]
        # M* fields stay null (not captured) — the UI must show that, not 0
        out.append(d)
    return out


def _combats(frame: int) -> List[Dict[str, Any]]:
    """One M* land combat that appears in frames 1-2 (lifecycle visible)."""
    if frame == 0:
        return []
    return [{
        "combat_id": "9001:7",
        "kind": "land",
        "location_province": 9,
        "attackers": [[1, 7], [2, 7]],   # GER divisions, as [div_id, div_type] tuples
        "defenders": [[3, 7]],           # POL division
        "combat_width": None,            # M* not captured
        "terrain": "plains",
        "progress": 22.5 + frame * 30,   # NOT normalized -1..1 (matches real oracle)
        "modifiers": {"day": frame * 14, "duration": frame * 14},
    }]


def _diplomacy_world(frame: int) -> Dict[str, Any]:
    wars = {}
    if frame >= 0:
        wars["GER-POL@1939.9.1.12"] = {
            "a": "GER", "b": "POL", "start_date": "1939.9.1.12",
            "a_casualties": frame * 800, "b_casualties": frame * 1500,
            "war_score_a_vs_b": frame * 25.0, "war_score_b_vs_a": 0.0,
        }
    if frame >= 1:  # ENG+FRA declare on GER
        for x in ("ENG", "FRA"):
            wars[f"{x}-GER@1939.9.3.12"] = {
                "a": x, "b": "GER", "start_date": "1939.9.3.12",
                "a_casualties": 0, "b_casualties": frame * 50,
                "war_score_a_vs_b": 0.0, "war_score_b_vs_a": frame * 5.0,
            }
    factions = {
        "1": {"id": 1, "ideology": "fascism", "icon": "GFX_faction_logo_axis",
              "template": "faction_template_axis",
              "members": ["GER"] + (["ITA"] if frame >= 1 else []), "leader": "GER"},
        "2": {"id": 2, "ideology": "democratic", "icon": "GFX_faction_logo_allies",
              "template": "faction_template_allies",
              "members": ["ENG", "FRA"], "leader": "ENG"},
    }
    return {"wars": wars, "factions": factions, "subjects": []}


def _states(frame: int) -> Dict[str, Any]:
    """A small set of states whose ownership/control flips over the frames."""
    # state -> (owner per frame, controller per frame)
    plan = {
        9:  (["POL", "POL", "GER"], ["POL", "GER", "GER"]),   # conquered Poland
        64: (["GER", "GER", "GER"], ["GER", "GER", "GER"]),
        16: (["FRA", "FRA", "FRA"], ["FRA", "FRA", "FRA"]),
        126:(["ENG", "ENG", "ENG"], ["ENG", "ENG", "ENG"]),
        2:  (["ITA", "ITA", "ITA"], ["ITA", "ITA", "ITA"]),
    }
    out: Dict[str, Any] = {}
    for sid, (owners, ctrls) in plan.items():
        st = schema.empty_state()
        st["owner"] = owners[frame]
        st["controller"] = ctrls[frame]
        st["buildings"] = {"infrastructure": 4, "industrial_complex": 3, "arms_factory": 2}
        st["victory_points_total"] = {9: 5, 64: 20, 16: 25, 126: 50, 2: 10}[sid]
        st["category"] = "large_city" if sid in (64, 16, 126) else "town"
        st["manpower_pool"] = {"available": 50000, "locked": 100000, "total": 150000}
        out[str(sid)] = st
    return out


def build_demo_snapshot(frame: int) -> Dict[str, Any]:
    date = _DATES[frame]
    snap = schema.empty_snapshot(DEMO_GAME_ID, date)
    snap["save_version"] = "Synthetic demo v1"
    # Frame 0 is save-only (memory == null) so the "not captured" path for
    # memory-tagged fields (stability/war_support/pp/cp, fuel, combats) is visible;
    # frames 1-2 are memory-backed so those same fields are captured. Both states
    # are therefore exercised across the demo.
    with_memory = (frame >= 1)
    for tag in _TAGS:
        snap["countries"][tag] = _country(tag, frame, with_memory=with_memory)
    snap["states"] = _states(frame)
    snap["divisions"] = _divisions(frame)
    snap["combats"] = _combats(frame)
    snap["diplomacy_world"] = _diplomacy_world(frame)
    # mirror owner_by_state for the map layer (D)
    snap["map"]["owner_by_state"] = {sid: st["owner"]
                                     for sid, st in snap["states"].items()
                                     if st.get("owner")} or None

    # provenance — honest per-frame channel record. combats is canonical M*
    # (SCHEMA §7) so it belongs to the MEMORY channel, never save (Codex #1). The
    # demo lights up memory on the combat frames so the combat screen has captured
    # rows, and leaves frame 0 save-only (memory==null) so the "not captured" path
    # is also exercised.
    save_domains = ["country_snapshot", "economy", "production", "tech", "focus",
                    "diplomacy", "state_ownership", "state_control", "state_detail",
                    "divisions"]
    snap["provenance"]["save"] = {
        "captured": True, "source_filename": "demo_synthetic.txt",
        "save_date": date, "domains": save_domains,
    }
    mem_on = with_memory or bool(snap["combats"])
    if mem_on:
        mem_domains = ["country_scalars", "state_control", "divisions"]
        if snap["combats"]:
            mem_domains.append("combats")
        snap["provenance"]["memory"] = {
            "captured": True, "exe_sha256": None,
            "domains": mem_domains,
            "save_date_validated_against": date,
        }
        snap["provenance"]["notes"] = ["demo frame backed by save + live memory"]
    else:
        snap["provenance"]["memory"] = None
        snap["provenance"]["notes"] = ["demo frame is save-only (memory not running)"]
    return snap


def generate(store_root: str = DEMO_STORE) -> str:
    """Build + persist the 3-frame demo store and its timeline. Returns the dir."""
    store = SnapshotStore(store_root)
    for frame in range(len(_DATES)):
        snap = build_demo_snapshot(frame)
        store.write_snapshot(snap)
    snaps = store.load_all_ordered(DEMO_GAME_ID)
    events = build_timeline(snaps)
    store.write_timeline(DEMO_GAME_ID, events)
    return store.game_dir(DEMO_GAME_ID)


if __name__ == "__main__":
    d = generate()
    print(f"demo store written: {d}")
