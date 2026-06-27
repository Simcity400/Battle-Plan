"""Schema-fidelity test for the replay_render adapter + province-level control.

Builds a MOCKED snapshot from ``replay.schema.empty_snapshot`` (the canonical
shape the oracle/store actually emits), fills a few states / provinces /
divisions, and pushes it through ``frame_from_replay_snapshot`` ->
``FrameRenderer``. The point is to prove:

  * the adapter consumes the REAL schema key names (``map.owner_by_state`` /
    ``states[*].owner`` / ``states[*].controller`` / ``map.control_by_province``
    / ``provinces[*].controller`` / ``divisions[*].location_province`` …), and
  * a province whose controller DIFFERS from its state owner triggers the
    contested-hatch + front-line path at the PROVINCE level (not just the
    state level) — i.e. an intra-state front renders.

Pure offline: numpy + stdlib only, no running game, no real save needed (the
only on-disk reads are the read-only map_pipeline geometry assets).

Run directly:  python -m replay_render.test_schema_adapter
"""

from __future__ import annotations

import numpy as np

from replay import schema
from .frames import (frame_from_replay_snapshot, frames_from_replay_series,
                     date_to_hour_index)
from .geometry import load_geometry
from .render import FrameRenderer, color_coverage, WATER, NO_OWNER, FRONT_LINE


# A multi-province state from the real assets so the province override lands on
# a state that is actually painted. (state 1 -> provinces 3838 / 9851 / 11804.)
STATE = 1
PROV_A = 3838    # stays owner-controlled
PROV_B = 9851    # CONTROLLER differs from the state owner -> intra-state front
OWNER_TAG = "AIF"     # real tag, real colour (80,80,80)
INVADER_TAG = "ALB"   # real tag, real colour (160,40,140)


def build_mock_snapshot() -> dict:
    """A schema-shaped snapshot with an intra-state province occupation."""
    snap = schema.empty_snapshot(game_id="mock_test", date="1940.9.10.13")

    # --- states (§4) : state 1 owned + controlled by OWNER_TAG ---
    st = schema.empty_state()
    st["owner"] = OWNER_TAG
    st["controller"] = OWNER_TAG
    snap["states"][str(STATE)] = st

    # --- provinces (§5) : PROV_B is controlled by the INVADER (the rest by owner) ---
    for pid in (PROV_A, PROV_B):
        pr = schema.empty_province()
        pr["state_id"] = STATE
        pr["controller"] = OWNER_TAG if pid == PROV_A else INVADER_TAG
        snap["provinces"][str(pid)] = pr

    # --- map convenience mirrors (§8), exactly as store._fill_map_layers emits ---
    snap["map"]["owner_by_state"] = {str(STATE): OWNER_TAG}
    snap["map"]["control_by_province"] = {
        str(PROV_A): OWNER_TAG, str(PROV_B): INVADER_TAG,
    }

    # --- divisions (§6) : one each, at PROV_A / PROV_B ---
    for pid, tag, strength in ((PROV_A, OWNER_TAG, 120.0),
                               (PROV_B, INVADER_TAG, 80.0)):
        dv = schema.empty_division()
        dv["div_id"] = pid          # any unique int
        dv["div_type"] = 1
        dv["owner"] = tag
        dv["location_province"] = pid
        dv["strength"] = strength
        dv["template_role"] = "infantry"
        snap["divisions"].append(dv)

    # snapshot must validate against its own schema (no fabrication, right shapes)
    problems = schema.validate_snapshot(snap)
    assert not problems, f"mock snapshot failed schema validation: {problems}"
    return snap


def main() -> int:
    print("[test] loading geometry (read-only map_pipeline assets)...")
    geom = load_geometry(half_res=True, with_centroids=True)

    snap = build_mock_snapshot()

    # ---- 1. adapter consumes the real schema ----
    fr = frame_from_replay_snapshot(snap)
    assert fr.timestamp == "1940.9.10.13", fr.timestamp
    assert fr.hour_index == date_to_hour_index("1940.9.10.13"), fr.hour_index
    assert fr.owner.get(STATE) == OWNER_TAG, fr.owner
    assert fr.controller and fr.controller.get(STATE) == OWNER_TAG, fr.controller
    # PROVINCE-level control is carried, and PROV_B's controller is the invader
    assert fr.control_by_prov is not None, "province control not carried"
    assert fr.control_by_prov.get(PROV_B) == INVADER_TAG, fr.control_by_prov
    assert fr.control_by_prov.get(PROV_A) == OWNER_TAG, fr.control_by_prov
    assert fr.divisions and len(fr.divisions) == 2, fr.divisions
    print(f"[test] adapter OK: ts={fr.timestamp} hour_index={fr.hour_index} "
          f"owner[{STATE}]={fr.owner[STATE]} "
          f"prov_ctrl[{PROV_B}]={fr.control_by_prov[PROV_B]} "
          f"divisions={len(fr.divisions)}")

    # ---- 2. the colour tables: PROV_B is contested, PROV_A is not ----
    r = FrameRenderer(geom, show_front_lines=True, show_divisions=True)
    primary, alt, contested, ctrl_of_prov = r._color_tables(fr)

    assert contested[PROV_B], "PROV_B (province controller != state owner) not contested"
    assert not contested[PROV_A], "PROV_A (province controller == owner) wrongly contested"
    # primary (owner) colour matches the REAL game colour for OWNER_TAG
    owner_rgb = tuple(int(c) for c in geom.country_colors[OWNER_TAG])
    inv_rgb = tuple(int(c) for c in geom.country_colors[INVADER_TAG])
    assert tuple(primary[PROV_B]) == owner_rgb, (tuple(primary[PROV_B]), owner_rgb)
    # alt (contested hatch toward controller) matches the INVADER's real colour
    assert tuple(alt[PROV_B]) == inv_rgb, (tuple(alt[PROV_B]), inv_rgb)
    # the two provinces have DIFFERENT controller codes -> a front exists between them
    assert ctrl_of_prov[PROV_A] != ctrl_of_prov[PROV_B], (
        ctrl_of_prov[PROV_A], ctrl_of_prov[PROV_B])
    print(f"[test] color tables OK: contested[{PROV_B}]={bool(contested[PROV_B])}, "
          f"owner colour={owner_rgb} (real game), hatch->controller={inv_rgb}, "
          f"ctrl_code A={ctrl_of_prov[PROV_A]} B={ctrl_of_prov[PROV_B]}")

    # ---- 3. render: contested hatch + a province-level front line appear ----
    img = r.render(fr)
    assert img.shape == (geom.height, geom.width, 3), img.shape
    # both the owner colour and the invader colour appear in the rendered state
    # (the invader colour appears ONLY because of the province-level override —
    # at state level the whole state is owner-controlled).
    flat = img.reshape(-1, 3)
    has_owner = np.any(np.all(flat == np.array(owner_rgb, np.uint8), axis=1))
    has_inv = np.any(np.all(flat == np.array(inv_rgb, np.uint8), axis=1))
    has_front = np.any(np.all(flat == np.array(FRONT_LINE, np.uint8), axis=1))
    assert has_owner, "owner colour missing from render"
    assert has_inv, "invader colour missing -> province-level override did NOT render"
    assert has_front, "no front-line pixels -> intra-state front did not render"
    print(f"[test] render OK: owner-colour={has_owner}, "
          f"invader-colour(province override)={has_inv}, front-line={has_front}")

    # ---- 4. state-level fallback still works (no province control) ----
    snap2 = build_mock_snapshot()
    snap2["provinces"] = {}                      # drop province control entirely
    snap2["map"]["control_by_province"] = None
    # make the WHOLE state contested at the state level instead
    snap2["states"][str(STATE)]["controller"] = INVADER_TAG
    snap2["map"] = snap2["map"]                   # owner_by_state still set
    fr2 = frame_from_replay_snapshot(snap2)
    assert fr2.control_by_prov is None, "province control should be absent here"
    assert fr2.controller and fr2.controller.get(STATE) == INVADER_TAG
    _, _, contested2, _ = r._color_tables(fr2)
    assert contested2[PROV_A] and contested2[PROV_B], (
        "state-level controller!=owner should contest ALL its provinces (fallback)")
    print("[test] state-level fallback OK: with NO province control, the whole "
          "state contests from its state controller.")

    # ---- 5. series ordering (hour_index, not lexical) ----
    early = schema.empty_snapshot("mock", "1936.2.1.0")    # Feb
    late = schema.empty_snapshot("mock", "1936.10.1.0")    # Oct (lexically < "1936.2")
    frames = frames_from_replay_series([late, early])
    assert [f.timestamp for f in frames] == ["1936.2.1.0", "1936.10.1.0"], \
        [f.timestamp for f in frames]
    print("[test] series ordering OK: sorted by hour_index, not the date string.")

    # ---- 6. colour-coverage diagnostic (real-tags clean; unknown tag flagged) ----
    cov_clean = color_coverage(fr, geom.country_colors)
    assert cov_clean["unknown_owner_tags"] == [], cov_clean
    assert cov_clean["unknown_controller_tags"] == [], cov_clean
    assert cov_clean["no_state_controllers"] is False, cov_clean
    # a dynamic tag with no entry in country_colors.json must be flagged (not faked)
    DYN = "D99"
    assert DYN not in geom.country_colors, "pick a tag genuinely absent from the asset"
    snap3 = build_mock_snapshot()
    snap3["map"]["control_by_province"][str(PROV_B)] = DYN
    snap3["provinces"][str(PROV_B)]["controller"] = DYN
    fr3 = frame_from_replay_snapshot(snap3)
    cov_dyn = color_coverage(fr3, geom.country_colors)
    assert DYN in cov_dyn["unknown_controller_tags"], cov_dyn
    print(f"[test] colour coverage OK: clean frame has 0 unknown tags; a tag "
          f"absent from country_colors.json ({DYN}) is flagged, not faked -> "
          f"renders UNKNOWN by design.")

    print("\n[test] ALL ASSERTIONS PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
