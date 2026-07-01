"""buildings_save.py - SAVE-anchored per-PROVINCE rail + per-STATE building levels.

The logistics map layers split into a SAVE-anchored part and a LIVE part:

  * RAIL (Railway map mode) and STATE BUILDINGS (infrastructure, arms_factory,
    industrial_complex, air_base, dockyard, ...) are read from the time-aligned
    melted save. They are *low-volatility*: they change only by construction
    completion / bomb damage / scorched-earth, not every tick. The save gives
    FULL, EXACT coverage (validated 1081/1081 state rail sums; 943 infra states;
    3377 rail provinces) - far denser than any partial live read.

  * SUPPLY (Supply map mode) is runtime-computed and has NO save value; it is the
    genuinely live-only target (see supply_live.py for the status / wall).

Why save-anchored here (and not live): an exhaustive read-only memory hunt on
build 16022851 (see RESEARCH_logistics_intel.md) established that building LEVELS
are NOT stored as a dense province/state-indexed scalar array, NOT as AoS records
keyed by the raw province_id, and NOT with an inline building-type name string per
instance. They live in a building manager keyed by a building-type *id* with the
location stored as an object pointer - no cheap read-only anchor was found within
budget. Because buildings are low-volatility, the save backstop is the correct,
high-fidelity source for this layer; the residual (a live before/after diff around
a construction completion) is documented as future work.

This module parses the building levels straight out of the melted save text. It is
pure-stdlib text parsing of a file on disk (NOT a memory read), so it needs no live
process. It is the deterministic baseline the replay binds the live layers onto.

  province rail level   : provinces.<pid>.buildings.rail_way.level   (per province)
  province supply_node  : provinces.<pid>.buildings.supply_node.level (per province)
  province naval_base   : provinces.<pid>.buildings.naval_base.level  (per province)
  state buildings       : state_detail.json states.<sid>.buildings.<bld>  (per state)

Usage:
    from logistics_intel import buildings_save as bs
    layer = bs.BuildingsSave.from_melt(r"...\\melted.hoi4")          # rail per province
    layer.rail(2068)                 # -> int level (0 if none)
    layer.rail_state_sum(375)        # -> 68  (sum of province rail in the state)
    # state buildings come from the save_oracle state_detail output:
    sl = bs.StateBuildings.from_state_detail(r"...\\state_detail.json")
    sl.infrastructure(2)             # -> 5
"""

import json
import os
import re


# ---------------------------------------------------------------------------
# Per-PROVINCE building levels (parsed from the melt's provinces={} block)
# ---------------------------------------------------------------------------
_PROV_BUILDINGS = ("rail_way", "supply_node", "naval_base", "bunker",
                   "coastal_bunker")


def _balanced_block(s, open_idx):
    """Return (text, end_index) of the {...} block starting at s[open_idx]=='{'."""
    depth = 0
    i = open_idx
    n = len(s)
    while i < n:
        c = s[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return s[open_idx:i + 1], i + 1
        i += 1
    return s[open_idx:], n


class BuildingsSave(object):
    """Per-province building levels from a melted (plaintext) the game save."""

    def __init__(self, prov):
        # prov[building_type] = {province_id: level}
        self.prov = prov

    @classmethod
    def from_melt(cls, melt_path):
        """Parse provinces.<pid>.buildings.<bld>.level out of a melted save.
        Reads at the byte level (saves are Windows-1252/mixed) and decodes
        latin-1 to stay lossless for the numeric fields we care about."""
        with open(melt_path, "rb") as fh:
            data = fh.read().decode("latin-1")
        pi = data.find("\nprovinces={")
        if pi < 0:
            raise ValueError("no provinces={} block in save")
        open_idx = data.find("{", pi)
        provblock, _ = _balanced_block(data, open_idx)

        out = {b: {} for b in _PROV_BUILDINGS}
        # top-level province entries: \n\t<pid>={ ... }
        for m in re.finditer(r"\n\t(\d+)=\{", provblock):
            pid = int(m.group(1))
            sub, _ = _balanced_block(provblock, provblock.find("{", m.start()))
            for bm in re.finditer(
                    r"(rail_way|supply_node|naval_base|coastal_bunker|bunker)"
                    r"=\{\s*level=(\d+)", sub):
                out[bm.group(1)][pid] = int(bm.group(2))
        return cls(out)

    # -- per-province accessors ----------------------------------------
    def rail(self, province_id):
        return self.prov["rail_way"].get(province_id, 0)

    def supply_node(self, province_id):
        return self.prov["supply_node"].get(province_id, 0)

    def naval_base(self, province_id):
        return self.prov["naval_base"].get(province_id, 0)

    def rail_map(self):
        """{province_id: rail_level} for every province with rail>0."""
        return dict(self.prov["rail_way"])

    # -- per-state aggregation (matches save_oracle's folded state value) --
    def rail_state_sum(self, state_id, province_to_state):
        total = 0
        for pid, lvl in self.prov["rail_way"].items():
            if province_to_state.get(pid) == state_id:
                total += lvl
        return total


# ---------------------------------------------------------------------------
# Per-STATE building levels (from the save_oracle state_detail output)
# ---------------------------------------------------------------------------
class StateBuildings(object):
    """Per-state building levels from save_oracle/output/state_detail.json."""

    def __init__(self, states):
        self.states = states          # {sid:int -> {bld:str -> level:int}}

    @classmethod
    def from_state_detail(cls, path):
        with open(path, "r", encoding="utf-8") as fh:
            doc = json.load(fh)
        out = {}
        for sid, s in doc.get("states", {}).items():
            b = dict(s.get("buildings", {}) or {})
            # fold the per-province building sums in too (rail_way, supply_node)
            for k, v in (s.get("province_buildings", {}) or {}).items():
                b.setdefault(k, v)
            out[int(sid)] = b
        return cls(out)

    def building(self, state_id, kind):
        return self.states.get(state_id, {}).get(kind, 0)

    def infrastructure(self, state_id):
        return self.building(state_id, "infrastructure")

    def state_map(self, kind):
        """{state_id: level} for the given building kind."""
        return {sid: b.get(kind, 0) for sid, b in self.states.items()}


def _default_province_to_state():
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.abspath(os.path.join(
        here, "..", "..", "map_pipeline", "assets", "province_to_state.json"))


def load_province_to_state(path=None):
    path = path or _default_province_to_state()
    with open(path, "r", encoding="utf-8") as fh:
        raw = json.load(fh)
    return {int(k): int(v) for k, v in raw.items()}
