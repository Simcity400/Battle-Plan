"""Focused offline test for the RICH replay-UI render path.

Pure stdlib, no game, no network, no browser. Asserts:
  1. the synthetic rich store is **schema-valid** (every frame, latest schema),
  2. it exercises the *new* domains a screen surfaces (fuel, laws, party_pop,
     resolved-name tokens, intel agency, commanders w/ portrait handles that
     resolve to REAL extracted PNGs, special_projects/extra, MIOs, OOB tree,
     naval_air/supply PROXY sidecars with a non-"exact" confidence, human_name,
     diplomacy_world casualties),
  3. the compact series the UI charts against carries the fuel series,
  4. the asset index maps the store's portrait handles to real files, and
  5. the no-fabrication guardrail holds: a minor's null/empty domains stay null
     (the UI renders "not captured", never 0).

Run:  python -m replay_ui.test_richdemo
"""
from __future__ import annotations

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
if REPO not in sys.path:
    sys.path.insert(0, REPO)

from replay.schema import validate_snapshot  # noqa: E402
from replay_ui.richdemo import generate, RICH_GAME_ID  # noqa: E402
from replay_ui.series import build_series  # noqa: E402


def _load_frames(store_dir):
    idx = json.load(open(os.path.join(store_dir, "index.json"), encoding="utf-8"))
    return [json.load(open(os.path.join(store_dir, s["file"]), encoding="utf-8"))
            for s in idx["snapshots"]]


def main() -> int:
    store_dir = generate()
    frames = _load_frames(store_dir)
    assert len(frames) >= 2, "rich store must be multi-frame"

    # 1. schema-valid (latest schema → zero errors)
    for f in frames:
        errs = validate_snapshot(f)
        assert not errs, f"frame {f['date']} not schema-valid: {errs[:5]}"
    print(f"[test] rich store schema-valid: {len(frames)} frames, 0 errors each")

    late = frames[-1]  # the advanced (memory + every domain) frame
    ger = late["countries"]["GER"]

    # 2. new domains populated
    assert ger["economy"]["fuel"]["current"] is not None, "fuel missing"
    laws = ger["politics"]["laws"]
    assert laws["economy"] and laws["conscription"], "laws triple missing"
    assert ger["politics"]["party_popularities"], "party_popularities missing"
    ag = ger["intel"]["agency"]
    assert ag and ag["name"] == "Abwehr" and ag["capacity"]["max_operatives"], "intel agency missing"
    lead = ger["commanders"]["leader"]
    assert lead and lead["portrait_gfx"], "leader/portrait missing"
    sp = ger["extra"]["special_projects"]
    assert sp and sp["projects_started"] is not None, "special_projects missing"
    assert ger["mios"] and ger["mios"][0]["name"], "mios missing"
    assert ger["human_name"], "human_name (MP persona) missing"
    # in_progress carries a RAW token the UI humanizes (no shipped localisation)
    assert any(p["name"] == "excavation3" for p in ger["research"]["in_progress"]), \
        "raw research token missing"
    print("[test] new per-country domains present (fuel/laws/party_pop/intel/leader/"
          "projects/mios/human_name/raw-token)")

    # OOB command tree
    oob = late["order_of_battle"]
    assert oob["army_groups"] and oob["by_country"]["GER"]["planes"], "OOB tree missing"
    # diplomacy_world wars carry casualties
    wars = late["diplomacy_world"]["wars"]
    assert any(w.get("a_casualties") for w in wars.values()), "war casualties missing"
    print("[test] world-scoped OOB tree + diplomacy_world casualties present")

    # PROXY sidecars: present AND non-exact confidence travels with the data
    na, sup = late["naval_air"], late["supply"]
    assert na["air_superiority"] and na["confidence"] and na["confidence"] != "exact", \
        "naval_air proxy/confidence missing or 'exact'"
    assert sup["by_province"] and sup["confidence"] != "exact", \
        "supply proxy/confidence missing or 'exact'"
    print(f"[test] PROXY sidecars badged non-exact: naval_air={na['confidence']!r} "
          f"supply={sup['confidence']!r}")

    # 3. compact series carries the fuel series (what the chart plots)
    series = build_series(RICH_GAME_ID, store_root=os.path.join(HERE, "demo_snapshots"))
    fuel = series["series"]["GER"]["economy.fuel.current"]
    assert fuel and any(v is not None for v in fuel), "fuel series empty"
    assert series["identity"]["GER"]["is_human"] is True, "GER human identity lost"
    print(f"[test] compact series fuel = {fuel}")

    # 4. asset index maps the store's real portrait handles to real files
    pj = os.path.join(REPO, "game_assets", "portraits.json")
    if os.path.isfile(pj):
        pd = (json.load(open(pj, encoding="utf-8")) or {}).get("portraits") or {}
        h = lead["portrait_gfx"]
        if h in pd:  # only assert when assets were extracted
            png = (pd[h].get("png") or [None])[0]
            assert png and os.path.isfile(os.path.join(REPO, "game_assets", png)), \
                f"portrait handle {h} does not resolve to a real PNG"
            print(f"[test] portrait handle {h} -> real PNG {png}")
        else:
            print(f"[test] (portrait assets not extracted; handle-map check skipped)")
    else:
        print("[test] (no portraits.json; asset check skipped)")

    # 5. no-fabrication guardrail: the minor BEL keeps null/empty domains (the UI
    # must render these as "not captured", never 0).
    bel = late["countries"]["BEL"]
    assert bel["politics"]["party_popularities"] is None, "minor party_pop should be null"
    assert bel["economy"]["fuel"] == {}, "minor fuel should be empty (not captured)"
    assert bel["military"]["casualties"] == {}, "minor casualties should be empty"
    assert not bel["intel"].get("agency"), "minor should have no agency"
    print("[test] no-fabrication: minor (BEL) null/empty domains preserved")

    print("\n[test] ALL ASSERTIONS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
