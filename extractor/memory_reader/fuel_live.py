"""fuel_live.py - READ-ONLY national-FUEL reader for Battle Plan.

STATUS: CRACKED (2026-06-26) via STATIC Ghidra xref of the fuel serializer/getters.
A portable, value-seed-FREE LIVE pin now exists. This OVERTURNS the earlier
"RESISTED" conclusion: the prior live pass only ever found f32 *display mirrors*
(no anchor), but the CANONICAL store is an i64 fixed-point field reachable by a
fixed pointer chain off the gamestate root.

THE CHAIN (build 16022851; see fuel_offsets.draft.json -> ghidra_xref for the
decompiled evidence; image base = p.module_base):
    gamestate   = u64[module_base + 0x33048C0]
    fuel_status = u64[CCountry    + 0x1580]        # CCountryFuelStatus*
    fuel        = i64[fuel_status + 0x08] / 32768  # post-clamp current fuel
    max_fuel    = i64[fuel_status + 0x10] / 32768
The CCountry pointer per TAG comes from the existing identity bridge
(intel_live.resolve_country_objects). Scale is 2**15 = 32768 (binary fixed-point;
the engine's ratio getter does (fuel<<15)/max). The getters' /100000 is a SEPARATE
display unit, NOT the raw store scale.

VALIDATION (read_fuel_live vs the melted save GT): max_fuel = 71/71 EXACT
(relerr 0.000000 every country) -> the chain + offsets + scale are proven.
current_fuel = 32/71 within 0.5%; the rest diverge because +0x08 is CLAMPED to max
live while the save's fuel field can momentarily exceed max (the just-ticked gain),
and the live frame is a different game-moment than the 1946 save (slow cap matches
exactly; current fuel drifts by the per-tick delta). Codex-confirmed (twice).

HARD RULES honored (same as the sibling live readers):
  * READ-ONLY memory (RPM + VirtualQueryEx only); no writes / injection / thread
    suspension; keep the game PAUSED; passive reads -> any cadence, never desyncs MP.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the build.
  * No personal paths; self-contained; reuses winmem.py + intel_live identity bridge.

PUBLIC API
  read_fuel_live(p) -> {TAG: {"fuel","max_fuel","fuel_status_addr"}}
      AUTHORITATIVE LIVE reader. Portable pointer chain, no save seed needed.
      fuel = post-clamp current fuel; max_fuel = storage cap. None if unresolved.
  read_fuel_from_save(melt_path) -> {TAG: {"fuel","max_fuel","fuel_gain","fuel_cost"}}
      Save oracle. Carries fuel_gain/fuel_cost which the live chain does not expose
      cleanly; use as the monthly anchor / for those two extra fields.
  locate_fuel_live(p, save_fuel) -> {TAG: {"fuel_addr","live_fuel","save_fuel","relerr"}}
      LEGACY / validation-only. f32-mirror value-seed scan (needs the save value as a
      seed). Superseded by read_fuel_live; kept for cross-checking the f32 mirrors.
  read_all(p, melt_path=None) -> {"source": "...", "fuel": {...}, "note": "..."}
      Convenience: live chain if a process is given, else the save oracle.
"""

import json
import os
import re
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
import winmem  # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
PAGE = 0x1000
MIN_FUEL_NEEDLE = 10000.0     # below this, the f32 bit pattern is too common to scan
RELTOL = 5e-3                 # live==save acceptance (sub-tick drift on a paused frame)

# --- Ghidra-derived LIVE chain (build 16022851; see fuel_offsets.draft.json) -----
FS_PTR_IN_CCOUNTRY = 0x1580   # *(CCountry + 0x1580) -> CCountryFuelStatus*
FS_FUEL = 0x08                # i64 current fuel (post-clamp), 1/FUEL_SCALE units
FS_MAX_FUEL = 0x10            # i64 storage cap
FUEL_SCALE = 32768.0          # 2**15 binary fixed-point (NOT the getters' /100000)


# ==========================================================================
# AUTHORITATIVE LIVE: portable pointer-chain reader (Ghidra-derived)
# ==========================================================================
def read_fuel_status(p, country_object):
    """Read fuel/max_fuel for ONE country given its live CCountry pointer.
    Returns {"fuel","max_fuel","fuel_status_addr"} or None if the fuel_status
    pointer is missing/out of range. READ-ONLY."""
    if not country_object:
        return None
    try:
        fs = p.u64(country_object + FS_PTR_IN_CCOUNTRY)
    except OSError:
        return None
    if not fs or fs < HEAP_LO or fs > HEAP_HI:
        return None
    try:
        fuel = p.i64(fs + FS_FUEL) / FUEL_SCALE
        mx = p.i64(fs + FS_MAX_FUEL) / FUEL_SCALE
    except OSError:
        return None
    return {"fuel": fuel, "max_fuel": mx, "fuel_status_addr": fs}


def _resolve_country_objects(p, bridge=None):
    """{TAG: CCountry_ptr} via the shared identity bridge (intel_live). Falls back
    to the verified _fuel_anchors.json tag_obj map when the bridge's array-locate
    fails this session (the known stale-ARENA case). Returns {} if neither works.

    ``bridge`` (optional): an ALREADY-RESOLVED CountryIndexBridge to REUSE (the one the
    capture context resolved once at attach), skipping this reader's per-call country-array
    re-resolve heap scan -- the single biggest steady-state cost. ``None`` preserves the
    original self-resolving behaviour EXACTLY (and the caller MUST pass None when the shared
    bridge failed/partially resolved, so a bad bridge is never threaded -- fail-closed)."""
    out = {}
    try:
        import intel_live as il
        loc = il.resolve_country_objects(p, bridge=bridge)
    except Exception:  # noqa: BLE001 - bridge import/resolve is best-effort
        loc = None
    if loc and "bridge" in loc:
        br = loc["bridge"]
        try:
            for tag_db_id in br.as_map():
                try:
                    obj = br.country_object(tag_db_id)
                    if obj:
                        out[br.tag(tag_db_id)] = obj
                except Exception:  # noqa: BLE001 - skip a bad bridge entry, keep going
                    continue
        except Exception:  # noqa: BLE001 - bridge enumeration itself failed
            out = {}
        if out:
            return out
    # fallback: prior session's verified TAG->CCountry anchor map (fingerprint path).
    # NOTE: cross-session anchors may be stale; read_fuel_status's heap-range guard is
    # the safety net, and the bridge above is preferred when available this session.
    anchor_path = os.path.join(_HERE, "_fuel_anchors.json")
    if os.path.exists(anchor_path):
        try:
            with open(anchor_path) as fh:
                anch = json.load(fh)
            tag_obj = anch.get("tag_obj", {}) if isinstance(anch, dict) else {}
            for tag, hexptr in tag_obj.items():
                try:
                    out[tag] = int(hexptr, 16)
                except (ValueError, TypeError):
                    continue
        except (ValueError, OSError, AttributeError, TypeError):
            pass
    return out


def read_fuel_live(p, bridge=None):
    """AUTHORITATIVE LIVE per-country fuel via the Ghidra-derived pointer chain.
    Returns {TAG: {"fuel","max_fuel","fuel_status_addr"}} for every resolved country.
    No save seed required (portable). READ-ONLY. 'fuel' is the post-clamp current
    fuel; for the just-ticked uncapped value or fuel_gain/fuel_cost use the save
    oracle. Raises RuntimeError only if no country objects could be resolved at all.

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge to skip the
    per-call country-array re-resolve scan (steady-state perf); None self-resolves."""
    objs = _resolve_country_objects(p, bridge=bridge)
    if not objs:
        raise RuntimeError("could not resolve any country objects (bridge + anchor "
                           "fallback both unavailable)")
    out = {}
    for tag, obj in objs.items():
        snap = read_fuel_status(p, obj)
        if snap is not None:
            out[tag] = snap
    return out


# ==========================================================================
# fuel from the melted save (carries fuel_gain / fuel_cost)
# ==========================================================================
_COUNTRY_RE = re.compile(r"^\t([A-Z][A-Z0-9]{2})=\{\s*$")
_KEYOPEN_RE = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)=\{\s*$")
_TOPKEY_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)=\{\s*$")
_FUELFLD_RE = re.compile(r"^\s*(fuel|max_fuel|fuel_gain|fuel_cost)=(-?[\d.eE+]+)\s*$")


def read_fuel_from_save(melt_path):
    """AUTHORITATIVE per-country fuel from a rakaly-melted save. Returns
    {TAG: {"fuel","max_fuel","fuel_gain","fuel_cost"}} for countries that have a
    fuel_status block. Last country block wins (the real one). READ-ONLY of a file."""
    countries = {}
    stack = []
    depth = 0
    cur_tag = None
    cur = None
    cur_country_depth = None
    top0_key = None
    in_fuel = False
    with open(melt_path, "r", encoding="latin-1") as fh:
        for line in fh:
            opens = line.count("{")
            closes = line.count("}")
            if depth == 0:
                tk = _TOPKEY_RE.match(line)
                if tk:
                    top0_key = tk.group(1)
            cm = _COUNTRY_RE.match(line)
            if cm and depth == 1 and top0_key == "countries":
                cur_tag = cm.group(1)
                cur = {}
                countries[cur_tag] = cur
                cur_country_depth = depth
                stack.append((cur_tag, depth))
                depth += opens - closes
                in_fuel = False
                continue
            if cur is not None:
                ko = _KEYOPEN_RE.match(line)
                key = ko.group(1) if ko else None
                if key == "fuel_status":
                    in_fuel = True
                if in_fuel:
                    fm = _FUELFLD_RE.match(line)
                    if fm:
                        cur[fm.group(1)] = float(fm.group(2))
            if opens and not closes and _KEYOPEN_RE.match(line):
                stack.append((_KEYOPEN_RE.match(line).group(1), depth))
            new_depth = depth + opens - closes
            if closes:
                while stack and stack[-1][1] >= new_depth:
                    popped, _ = stack.pop()
                    if popped == "fuel_status":
                        in_fuel = False
            depth = new_depth
            if cur is not None and depth <= cur_country_depth:
                cur = None
                cur_tag = None
                in_fuel = False
    return {t: d for t, d in countries.items() if "fuel" in d or "max_fuel" in d}


# ==========================================================================
# EXPERIMENTAL / validation-only: relocate the live f32 fuel given the save seed
# ==========================================================================
def _is_ptr_alias(p, X):
    """True if the 4 bytes at X lie inside an 8-aligned heap pointer (the pointer-alias
    false-positive trap: heap addrs 0x0000024A.... have a low dword that as f32 lands in
    the fuel magnitude band). Such hits are NOT fuel."""
    base = (X // 8) * 8
    for q in (base, base - 8, base + 8):
        qb = p.try_read(q, 8)
        if len(qb) < 8:
            continue
        v = struct.unpack("<Q", qb)[0]
        if HEAP_LO <= v <= HEAP_HI and q <= X < q + 8 and (X + 4) <= q + 8:
            return True
    return False


def _range_scan_f32(p, targets):
    """Vectorized whole-heap f32 RANGE scan. targets = {value: [tag,...]} for distinct
    high-variance values. Returns {tag: [addr,...]} of NON-pointer-alias hits."""
    import numpy as np
    tvals = np.array(sorted(targets), dtype=np.float64)
    if len(tvals) == 0:
        return {}
    rel = 1e-4
    lo = tvals * (1 - rel) - 0.5
    hi = tvals * (1 + rel) + 0.5
    out = {}
    CH = 64 << 20
    OV = 16
    gmin = tvals[0] * (1 - rel) - 1
    gmax = tvals[-1] * (1 + rel) + 1
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                blen = len(blob)
                n = blen // 4
                if n:
                    arr = np.frombuffer(blob, dtype="<f4", count=n)
                    with np.errstate(invalid="ignore"):
                        inband = (arr >= gmin) & (arr <= gmax)
                    for ix in np.nonzero(inband)[0]:
                        x = float(arr[ix])
                        j = np.searchsorted(tvals, x)
                        for jj in (j - 1, j):
                            if 0 <= jj < len(tvals) and lo[jj] <= x <= hi[jj]:
                                a = base + pos + int(ix) * 4
                                if not _is_ptr_alias(p, a):
                                    for tg in targets[float(tvals[jj])]:
                                        out.setdefault(tg, []).append(a)
                if pos + blen >= size or blen < want:
                    pos += max(blen, PAGE)
                else:
                    pos += blen - OV
            else:
                pos += max(want, PAGE)
    return out


def locate_fuel_live(p, save_fuel):
    """EXPERIMENTAL (validation/debug only). Given save_fuel = {TAG: fuel_value}, relocate
    each high-variance country's live f32 fuel (de-aliased) and return
    {TAG: {"fuel_addr","live_fuel","save_fuel","relerr"}}. NOTE: this needs the save value
    as a seed -> it is NOT a portable standalone live reader (see module docstring /
    fuel_offsets.draft.json). High-variance only (fuel >= %g); low/zero-fuel values flood
    the scan.""" % MIN_FUEL_NEEDLE
    targets = {}
    for tg, fv in save_fuel.items():
        if fv is not None and fv >= MIN_FUEL_NEEDLE:
            targets.setdefault(float(fv), []).append(tg)
    hits = _range_scan_f32(p, targets)
    out = {}
    for tg, addrs in hits.items():
        fv = save_fuel[tg]
        best = None
        for X in addrs:
            try:
                v1 = p.f32(X)
                v2 = p.f32(X)
            except OSError:
                continue
            if v1 != v2:
                continue
            err = abs(v1 - fv) / max(1.0, abs(fv))
            if best is None or err < best[2]:
                best = (X, v1, err)
        if best is not None:
            out[tg] = {"fuel_addr": best[0], "live_fuel": best[1],
                       "save_fuel": fv, "relerr": best[2]}
    return out


# ==========================================================================
# Public convenience
# ==========================================================================
def read_all(p, melt_path=None):
    """Return per-country fuel. PRIMARY source = the LIVE pointer chain (read_fuel_live)
    when a process is given. The save oracle (if a melt path is supplied) provides
    fuel_gain/fuel_cost and serves as the monthly anchor."""
    result = {"source": None, "fuel": {}, "save_extra": {}, "note": ""}
    if p is not None:
        try:
            result["fuel"] = read_fuel_live(p)
            result["source"] = "live_chain (CCountry+0x1580 fuel_status; i64/32768)"
        except Exception as e:  # noqa: BLE001 - live is best-effort; fall back to save
            result["note"] = "live chain failed: %s" % e
    if melt_path and os.path.exists(melt_path):
        save = read_fuel_from_save(melt_path)
        if not result["fuel"]:
            result["fuel"] = save
            result["source"] = result["source"] or "save_oracle"
        # always expose gain/cost from the save (the live chain doesn't surface them)
        result["save_extra"] = {t: {k: d[k] for k in ("fuel_gain", "fuel_cost") if k in d}
                                for t, d in save.items()}
    return result


# ==========================================================================
# build gate + validation
# ==========================================================================
def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


def _default_melt():
    return os.path.abspath(os.path.join(_HERE, "..", "re_scratch", "melt_sov_1946.txt"))


def _validate(p, melt_path=None):
    """Validate the LIVE pointer chain (read_fuel_live) against the melt ground truth.
    max_fuel is the proof field (a slow cap -> should match exactly); current fuel may
    drift by the per-tick delta / clamp (live frame != save frame). Returns True iff
    max_fuel matches for every resolved country."""
    melt_path = melt_path or _default_melt()
    save = read_fuel_from_save(melt_path)
    live = read_fuel_live(p)
    fuel_ok = fuel_n = max_ok = max_n = 0
    ferr = []
    merr = []
    for tag, lv in live.items():
        gt = save.get(tag)
        if not gt:
            continue
        gf = gt.get("fuel")
        gm = gt.get("max_fuel")
        if gf is not None:
            e = abs(lv["fuel"] - gf) / max(1.0, abs(gf))
            fuel_n += 1
            fuel_ok += e <= RELTOL
            ferr.append(e)
        if gm is not None:
            e = abs(lv["max_fuel"] - gm) / max(1.0, abs(gm))
            max_n += 1
            max_ok += e <= RELTOL
            merr.append(e)
    print("  max_fuel live==save: %d/%d (PROOF field)" % (max_ok, max_n))
    print("  fuel     live==save: %d/%d (drifts by tick/clamp vs save frame)"
          % (fuel_ok, fuel_n))
    if merr:
        merr.sort()
        print("  max_fuel median rel-err=%.6f  max=%.6f" % (merr[len(merr)//2], merr[-1]))
    if ferr:
        ferr.sort()
        print("  fuel     median rel-err=%.6f  max=%.6f" % (ferr[len(ferr)//2], ferr[-1]))
    return max_n > 0 and max_ok == max_n


def main():
    import argparse
    ap = argparse.ArgumentParser(description="the game national fuel reader (read-only).")
    ap.add_argument("--validate", action="store_true",
                    help="validate the LIVE pointer chain == the anchor melt")
    ap.add_argument("--live", action="store_true",
                    help="print live per-country fuel via the pointer chain")
    ap.add_argument("--melt", default=None, help="path to a rakaly-melted save")
    args = ap.parse_args()
    melt = args.melt or _default_melt()
    if args.validate:
        p = winmem.open_process(PROC_NAME)
        if p is None:
            raise SystemExit("hoi4.exe not running")
        with p:
            _build_gate(p)
            print("build gate OK; validating LIVE fuel chain == melt (READ-ONLY)...")
            _validate(p, melt)
        return
    if args.live:
        p = winmem.open_process(PROC_NAME)
        if p is None:
            raise SystemExit("hoi4.exe not running")
        with p:
            _build_gate(p)
            fuel = read_fuel_live(p)
        print("national fuel (LIVE pointer chain), %d countries:" % len(fuel))
        print("%-4s %14s %14s" % ("TAG", "fuel", "max_fuel"))
        for t in sorted(fuel, key=lambda x: -fuel[x]["max_fuel"])[:20]:
            d = fuel[t]
            print("%-4s %14.1f %14.1f" % (t, d["fuel"], d["max_fuel"]))
        return
    # default: print save-oracle fuel (carries gain/cost)
    fuel = read_fuel_from_save(melt)
    print("national fuel (save oracle; carries gain/cost), %d countries:" % len(fuel))
    print("%-4s %14s %14s %9s" % ("TAG", "fuel", "max_fuel", "gain"))
    majors = ["SOV", "GER", "ENG", "USA", "JAP", "ITA", "FRA", "POL", "RAJ"]
    for t in majors:
        if t in fuel:
            d = fuel[t]
            print("%-4s %14.1f %14.1f %9s" %
                  (t, d.get("fuel", -1), d.get("max_fuel", -1), d.get("fuel_gain", "-")))


if __name__ == "__main__":
    main()
