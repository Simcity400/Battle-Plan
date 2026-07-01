"""intel_save.py - per-(observer, target) INTEL levels (the Intel map mode).

The Intel map mode shows how much intel each country has on every other country,
in four channels: civilian, army, navy, airforce (0..100-ish).

In the save this is a POSITIONAL array per observer:

    countries.<TAG>.intel = { intel = {
        { }                              # target index 0 (null tag) -> empty
        { civilian=.. army=.. navy=.. airforce=.. }   # target index 1
        { civilian=.. army=.. navy=.. airforce=.. }   # target index 2
        ...
    } }

Each row's INDEX is the engine's internal country index of the target (it lines up
with the tag-db-id space used by state owner/controller; see RESEARCH). Empty rows
`{ }` mean zero accumulated intel on that target. In the time-aligned melt, 43
observers carry intel (1779 non-empty rows total).

IMPORTANT - this fixes a save_oracle bug: `save_oracle/parse_diplo.py` looked at the
wrong nesting and reported `intel_rows: 0` for every country (its diplomacy.json
output has empty intel). The data is actually present and rich; this module parses it
correctly straight from the melt as the intel BACKSTOP.

LIVE STATUS: NOT RESOLVED on build 16022851 by value-anchoring. A full-arena read-only
search for the save's distinctive intel floats (e.g. 60.64557, 48.63421) found ZERO
occurrences at f32, f64, and /100 scales. The reason is volatility: intel accrues/
decays DAILY, so the 1940.9.10 save values have drifted far from the (much later) live
state - there is nothing exact to anchor to, and intel is too low-entropy to anchor by
shape. Per the adversarial review, a live intel array needs live-UI validation (and a
target-index->TAG mapping that must be UI-confirmed, not assumed == tag-db-id). That is
the documented residual; see supply_live.py for the analogous live-hunt harness if a
future session validates intel against the running UI.

So for the replay, intel is SAVE-anchored (best available, full per-observer coverage),
with the live read flagged as future work.

stdlib-only text parsing of a melt on disk (not a memory read).
"""

import json
import os
import re


def _balanced_block(s, open_idx):
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


class IntelSave(object):
    """Per-observer positional intel rows parsed from a melted save.

    rows[observer_TAG] = { target_index:int -> {civ, army, navy, air} }
    Only non-empty target rows are stored (empty {} == zero intel, omitted).
    """

    def __init__(self, rows):
        self.rows = rows

    @classmethod
    def from_melt(cls, melt_path):
        with open(melt_path, "rb") as fh:
            data = fh.read().decode("latin-1")
        ci = data.find("\ncountries={")
        if ci < 0:
            raise ValueError("no countries={} block in save")
        countries_blk, _ = _balanced_block(data, data.find("{", ci))

        out = {}
        for m in re.finditer(r"\n\t([A-Z]{3})=\{", countries_blk):
            tag = m.group(1)
            cblk, _ = _balanced_block(countries_blk,
                                      countries_blk.find("{", m.start()))
            im = re.search(r"\n\t\tintel=\{", cblk)
            if not im:
                continue
            outer, _ = _balanced_block(cblk, cblk.find("{", im.start()))
            iim = re.search(r"intel=\{", outer)
            if not iim:
                continue
            inner, _ = _balanced_block(outer, outer.find("{", iim.start()))
            rows = cls._parse_positional(inner)
            if rows:
                out[tag] = rows
        return cls(out)

    @staticmethod
    def _parse_positional(inner):
        """Walk the depth-1 { ... } rows of the inner intel array; index by position."""
        rows = {}
        idx = 0
        d = 0
        i = 0
        rowstart = None
        # find the opening brace of `inner`
        p = inner.find("{")
        if p < 0:
            return rows
        i = p
        n = len(inner)
        d = 0
        while i < n:
            c = inner[i]
            if c == "{":
                d += 1
                if d == 2:
                    rowstart = i
            elif c == "}":
                if d == 2 and rowstart is not None:
                    row = inner[rowstart:i + 1]
                    civ = re.search(r"civilian=(-?[\d.]+)", row)
                    arm = re.search(r"army=(-?[\d.]+)", row)
                    nav = re.search(r"navy=(-?[\d.]+)", row)
                    air = re.search(r"airforce=(-?[\d.]+)", row)
                    if civ or arm or nav or air:
                        rows[idx] = {
                            "civ": float(civ.group(1)) if civ else 0.0,
                            "army": float(arm.group(1)) if arm else 0.0,
                            "navy": float(nav.group(1)) if nav else 0.0,
                            "air": float(air.group(1)) if air else 0.0,
                        }
                    idx += 1
                    rowstart = None
                d -= 1
                if d == 0:
                    break
            i += 1
        return rows

    # -- accessors ------------------------------------------------------
    def observers(self):
        return sorted(self.rows)

    def intel(self, observer_tag, target_index):
        """{civ,army,navy,air} the observer has on the target (by internal index),
        or None if no accumulated intel."""
        return self.rows.get(observer_tag, {}).get(target_index)

    def observer_map(self, observer_tag):
        """{target_index: {civ,army,navy,air}} for one observer."""
        return dict(self.rows.get(observer_tag, {}))

    def stats(self):
        tot = sum(len(v) for v in self.rows.values())
        return {"observers": len(self.rows), "total_rows": tot}


def main():
    import sys
    if len(sys.argv) < 2:
        print("usage: python intel_save.py <melted.hoi4>")
        raise SystemExit(2)
    layer = IntelSave.from_melt(sys.argv[1])
    print("intel (save):", layer.stats())
    for t in layer.observers()[:3]:
        mp = layer.observer_map(t)
        sample = {i: round(r["civ"], 2) for i, r in sorted(mp.items())[:5]}
        print("  %s: %d targets, civ sample %s" % (t, len(mp), sample))


if __name__ == "__main__":
    main()
