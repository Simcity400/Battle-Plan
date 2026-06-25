"""_supply_statescan.py - LIVE pin of the STATE-SUPPLY array using 5 UI anchors.

READ-ONLY. Scoped to logistics_intel research. Not committed gameplay code.

Strategy (state-indexed array hypothesis):
  anchor vector (state_id -> state_supply):
    42->2.80, 53->2.90, 157->1.40, 219->6.60, 237->0.60
  1. Find all arena addresses holding the most distinctive anchor (Moscow 6.60)
     in each candidate encoding (f32, f64, fixed x100000/x1000/x100, raw i32*100).
  2. For each such hit, HYPOTHESIZE it is state 219's slot in a stride-S array:
     array_base = hit - 219*S. Then read the slots for states 42/53/157/237 at
     array_base + sid*S and check they match their anchor (+-0.05). Test a set of
     plausible strides S per encoding (4 for f32/fixed32, 8 for f64; also 'struct'
     strides if the per-state value sits in an AoS record).
  3. A hit where ALL 5 anchors match at one (base, stride, encoding) is a candidate
     pin. Report base, stride, encoding, and read back all 5 for validation.

Uses fastscan for the heavy value-find (workers<=3 to coexist with the weather scan).
"""
import json
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem            # noqa: E402
import fastscan as fs    # noqa: E402

# state_id -> state_supply (the primary anchor vector)
ANCHORS = {42: 2.80, 53: 2.90, 157: 1.40, 219: 6.60, 237: 0.60}
# per-province "supply remaining" and hub supply, for the secondary scan
REMAINING = {42: 11.83, 53: 9.64, 157: 9.52, 219: 7.48, 237: 0.65}
HUBS = {42: 11.83, 53: 6.73, 157: 9.52, 219: 7.48, 237: 0.00}
# sub-components of state_supply for corroboration (base/infra/pop/vp)
COMPONENTS = {
    42:  {"base": 0.0, "infra": 1.20, "pop": 0.54, "vp": 1.05},
    53:  {"base": 1.40, "infra": 0.90, "pop": 0.26, "vp": 0.35},
    157: {"base": 0.0, "infra": 0.90, "pop": 0.53, "vp": 0.25},
    219: {"base": 1.20, "infra": 1.20, "pop": 1.50, "vp": 2.70},
    237: {"base": 0.0, "infra": 0.30, "pop": 0.03, "vp": 0.25},
}

TOL = 0.06   # 0.05 + a hair for fixed-point rounding


def decode(blob, off, enc):
    """Decode the scalar at blob[off:] in encoding `enc`; return float or None."""
    try:
        if enc == "f32":
            return struct.unpack_from("<f", blob, off)[0]
        if enc == "f64":
            return struct.unpack_from("<d", blob, off)[0]
        if enc == "fx100000":
            return struct.unpack_from("<i", blob, off)[0] / 100000.0
        if enc == "fx1000":
            return struct.unpack_from("<i", blob, off)[0] / 1000.0
        if enc == "fx100":
            return struct.unpack_from("<i", blob, off)[0] / 100.0
        if enc == "u32x100":
            return struct.unpack_from("<I", blob, off)[0] / 100.0
    except struct.error:
        return None
    return None


def itemsize(enc):
    return 8 if enc == "f64" else 4


def find_value_addrs(pid, arena, value, enc, workers=3):
    """Return sorted arena addresses whose scalar in `enc` ~= value (+-TOL).

    Implemented as a byte-pattern prefilter when exact (fixed-point quantizes the
    float to an int -> exact bytes), else an f32/f64 range scan via a custom
    matcher. We scan only for the *exact quantized* representation per encoding;
    +-TOL is applied at the verify step on readback, so the find can be exact."""
    lo, hi = arena
    isz = itemsize(enc)
    # Build the set of exact byte patterns that decode within TOL of value.
    pats = set()
    if enc in ("fx100000", "fx1000", "fx100", "u32x100"):
        scale = {"fx100000": 100000.0, "fx1000": 1000.0,
                 "fx100": 100.0, "u32x100": 100.0}[enc]
        center = int(round(value * scale))
        span = int(TOL * scale) + 1
        signed = enc != "u32x100"
        for iv in range(center - span, center + span + 1):
            if iv < 0 and not signed:
                continue
            pats.add(struct.pack("<i" if signed else "<I", iv))
    elif enc == "f32":
        # quantize: enumerate f32 representable values within TOL by stepping ULPs
        # is overkill; instead scan a range matcher. Use a numpy range matcher.
        return _range_scan(pid, arena, value, enc, isz, workers)
    elif enc == "f64":
        return _range_scan(pid, arena, value, enc, isz, workers)
    hits = []
    for pat in pats:
        spec = fs.make_bytes_find_matcher(pat)
        hits += fs.parallel_scan(pid, lo, hi, workers=workers, **spec)
    return sorted(set(hits))


def _range_scan(pid, arena, value, enc, isz, workers):
    """Float range scan via a top-level matcher in this module (spawn-importable)."""
    lo, hi = arena
    matcher = ("_supply_statescan", "_m_float_near")
    args = (value, TOL, enc)
    return fs.parallel_scan(pid, lo, hi, matcher=matcher, matcher_args=args,
                            item_size=isz, workers=workers)


def _m_float_near(base, blob, value, tol, enc):
    """Vectorized: every 4/8-aligned float in blob within tol of value."""
    import numpy as np
    if enc == "f32":
        phase = (-base) & 3
        n = (len(blob) - phase) & ~3
        if n <= 0:
            return []
        arr = np.frombuffer(blob, dtype="<f4", count=n >> 2, offset=phase)
        idx = np.nonzero(np.abs(arr - np.float32(value)) <= tol)[0]
        if idx.size == 0:
            return []
        return (base + phase + (idx.astype(np.uint64) << np.uint64(2))).tolist()
    else:  # f64
        phase = (-base) & 7
        n = (len(blob) - phase) & ~7
        if n <= 0:
            return []
        arr = np.frombuffer(blob, dtype="<f8", count=n >> 3, offset=phase)
        idx = np.nonzero(np.abs(arr - value) <= tol)[0]
        if idx.size == 0:
            return []
        return (base + phase + (idx.astype(np.uint64) << np.uint64(3))).tolist()


def test_state_array(p, moscow_addr, enc, strides):
    """Given an address that decodes to ~6.60, hypothesize it is state 219's slot.
    For each candidate stride, compute array_base = addr - 219*stride and check the
    other 4 anchors. Return list of matching (array_base, stride) tuples."""
    isz = itemsize(enc)
    out = []
    for S in strides:
        base = moscow_addr - 219 * S
        ok = True
        readvals = {}
        for sid, want in ANCHORS.items():
            slot = base + sid * S
            try:
                blob = p.read(slot, isz)
            except OSError:
                ok = False
                break
            got = decode(blob, 0, enc)
            if got is None or abs(got - want) > TOL:
                ok = False
                break
            readvals[sid] = got
        if ok:
            out.append((base, S, readvals))
    return out


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    pid = p.pid
    arena = fs.discover_arena(pid)
    print("pid %d  arena 0x%X..0x%X (%.2f GB)  build=%s" %
          (pid, arena[0], arena[1], (arena[1] - arena[0]) / float(1 << 30),
           p.fingerprint().get("sha256", "?")[:12]))

    encs = ["f32", "f64", "fx100000", "fx1000", "fx100", "u32x100"]
    # candidate strides per encoding: a tight flat array, or an AoS record.
    stride_sets = {
        4: [4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192, 256],
        8: [8, 16, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192, 256],
    }
    results = []
    for enc in encs:
        isz = itemsize(enc)
        addrs = find_value_addrs(pid, arena, ANCHORS[219], enc, workers=3)
        print("enc=%-9s Moscow(6.60) hits=%d" % (enc, len(addrs)))
        strides = stride_sets[isz]
        for a in addrs:
            for base, S, rv in test_state_array(p, a, enc, strides):
                results.append({"enc": enc, "array_base": base, "stride": S,
                                "moscow_addr": a, "readvals": rv})
                print("  CANDIDATE enc=%s base=0x%X stride=%d  vals=%s" %
                      (enc, base, S, {k: round(v, 3) for k, v in rv.items()}))
    print("\nTOTAL candidate state-supply arrays: %d" % len(results))
    out = os.path.join(_HERE, "_statescan_results.json")
    with open(out, "w") as f:
        json.dump({"pid": pid, "arena": list(arena), "anchors": ANCHORS,
                   "candidates": results}, f, indent=2)
    print("wrote", out)
    p.close()


if __name__ == "__main__":
    main()
