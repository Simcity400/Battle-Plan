"""parse_weather.py - per-province WEATHER from a melted the game save.

The save's top-level `weather={ provinces={ <id>={...} } }` block holds, PER PROVINCE,
the exact in-game weather state (validated 4/4 against in-game debug tooltips on the
1946.5.19 save: save `temperature` == the displayed degC to 2 dp):

    weather.provinces.<id> = {
        province          : <id>
        temperature       : float  -- the DISPLAYED temperature (deg C)
        temperature_offset: float  -- the displayed "Temperature change" (rate this tick)
        water             : float  -- precipitation / rain amount (optional)
        snow              : float  -- snow amount (optional)
    }

This is AUTHORITATIVE and deterministic (it's in the save) -- no live-memory read,
no climatology estimate. Emits output/weather.json: {province_id: {temp_c, temp_offset,
water, snow}} plus a small summary.

stdlib only; reuses the project's melt path via deep_common when run as a parser.
Standalone usage:
    python parse_weather.py [melted_save_or_dir]
"""
import json
import os
import re
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))


def parse_weather(text):
    """Parse the weather block out of a melted-save string.

    Returns {"provinces": {pid: {temp_c, temp_offset, water, snow}}, "summary": {...}}.
    Robust to key order and optional fields; brace-depth scoped to the weather block so
    a stray `temperature=` elsewhere can't leak in.
    """
    # locate the top-level weather block start
    m = re.search(r'(^|\n)weather=\{', text)
    if not m:
        return {"provinces": {}, "summary": {"found": False}}
    start = m.end() - 1  # at the '{'
    depth = 0
    i = start
    n = len(text)
    # find matching close for the weather block
    while i < n:
        c = text[i]
        if c == '{':
            depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    block = text[start:i + 1]

    provinces = {}
    cur = None
    rec = {}

    def flush():
        if cur is not None and ("temp_c" in rec):
            provinces[cur] = dict(rec)

    for line in block.splitlines():
        s = line.strip()
        mp = re.match(r'^province=(\d+)$', s)
        if mp:
            flush()
            cur = int(mp.group(1))
            rec = {}
            continue
        if cur is None:
            continue
        if s.startswith('temperature_offset='):
            try:
                rec["temp_offset"] = float(s.split('=', 1)[1])
            except ValueError:
                pass
        elif s.startswith('temperature='):
            try:
                rec["temp_c"] = float(s.split('=', 1)[1])
            except ValueError:
                pass
        elif s.startswith('water='):
            try:
                rec["water"] = float(s.split('=', 1)[1])
            except ValueError:
                pass
        elif s.startswith('snow='):
            try:
                rec["snow"] = float(s.split('=', 1)[1])
            except ValueError:
                pass
    flush()

    temps = [r["temp_c"] for r in provinces.values() if "temp_c" in r]
    summary = {
        "found": True,
        "province_count": len(provinces),
        "with_temp": len(temps),
        "temp_min_c": min(temps) if temps else None,
        "temp_max_c": max(temps) if temps else None,
        "temp_mean_c": (sum(temps) / len(temps)) if temps else None,
        "with_snow": sum(1 for r in provinces.values() if r.get("snow")),
        "with_water": sum(1 for r in provinces.values() if r.get("water")),
        "provenance": "save",  # authoritative; the value the game displays
    }
    return {"provinces": provinces, "summary": summary}


def _load_text(path):
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        return fh.read()


def main(argv):
    arg = argv[1] if len(argv) > 1 else None
    # accept a melted .txt directly; else look for the standard melt in re_scratch
    candidates = []
    if arg and os.path.isfile(arg):
        candidates = [arg]
    else:
        repo = os.path.dirname(_HERE)
        for c in ("re_scratch/melt_sov_1946.txt", "re_scratch/regen_melt_1946.txt",
                  "re_scratch/campaign_melt_1940.txt"):
            p = os.path.join(repo, c)
            if os.path.isfile(p):
                candidates.append(p)
    if not candidates:
        print("no melted save found; pass a melted-save .txt path")
        return 2
    text = _load_text(candidates[0])
    out = parse_weather(text)
    outdir = os.path.join(_HERE, "output")
    os.makedirs(outdir, exist_ok=True)
    with open(os.path.join(outdir, "weather.json"), "w", encoding="utf-8") as fh:
        json.dump({str(k): v for k, v in out["provinces"].items()}, fh)
    print("source:", os.path.relpath(candidates[0], os.path.dirname(_HERE)))
    print("summary:", json.dumps(out["summary"]))
    # spot-check the four validated anchors if present
    for pid, disp in ((1896, 20.09), (6421, 11.57), (3544, 18.16), (9244, 10.46)):
        r = out["provinces"].get(pid)
        if r:
            print("  prov %-5d displayed=%-6.2f save_temp=%.4f %s"
                  % (pid, disp, r["temp_c"],
                     "OK" if abs(r["temp_c"] - disp) <= 0.05 else "MISMATCH"))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
