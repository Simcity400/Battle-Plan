"""region_obj_scan.py - PER-REGION-OBJECT weather temperature pin (build 16022851).

Method 1 (most promising) from the B3 weather RE re-run. The air crack enumerated
the 304 strategic-region objects: region vtable = MOD+0x29468B8, region_id @ +0x58.

Procedure (all READ-ONLY, arena discovered LIVE):
  1. vtable_abs = module_base + 0x29468B8.
  2. fastscan the live arena for aligned qwords == vtable_abs  -> region object bases
     (the vtable pointer sits at object +0x00).
  3. For each object base, read u32 region_id @ +0x58. Build region_id -> obj map.
     Expect ~304 objects covering ids 1..304, incl. our 5 anchors.
  4. For each candidate float offset O in [0, 0x800): read f32 @ obj+O for all 5
     anchor objects; the offset where all 5 hold the expected temp (+-1.5C) is the
     live temperature field. Same sweep for f32_kelvin and fixed-point encodings.
  5. Then sweep for a PRECIP field: an offset where the 4 clear anchors share a
     value and region 31 differs (small enum / nonzero intensity).

READ-ONLY: PROCESS_VM_READ|PROCESS_QUERY_INFORMATION only.
"""
import json
import os
import sys
import uuid
import time
from collections import Counter, defaultdict

import numpy as np

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
for _p in (_MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import winmem as _wm
import fastscan as fs

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
REGION_VTABLE_OFF = 0x29468B8
REGION_ID_OFF = 0x58
BODY_SCAN = 0x800
_K = 273.15

# anchor (region_id, temp_C, is_rain)
ANCHORS = [
    (20,  8.0, False),
    (31,  26.0, True),
    (129, 11.0, False),
    (160, 24.0, False),
    (292, 16.0, False),
]
CLEAR_RIDS = [20, 129, 160, 292]


def _publish_json_atomic(path, value):
    """Durably publish JSON without exposing a truncated result to readers."""
    parent = os.path.dirname(os.path.abspath(path))
    os.makedirs(parent, exist_ok=True)
    temp_path = os.path.join(parent, ".%s.%s.tmp" % (os.path.basename(path), uuid.uuid4().hex))
    try:
        with open(temp_path, "w", encoding="utf-8", newline="\n") as stream:
            json.dump(value, stream, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_path, path)
    finally:
        if os.path.exists(temp_path):
            os.unlink(temp_path)
RAIN_RID = 31


def enumerate_region_objects(pid, module_base, arena, workers=6):
    """Return {region_id: obj_base} by scanning the arena for the region vtable
    qword and reading region_id @ +0x58 of each hit."""
    vt_abs = module_base + REGION_VTABLE_OFF
    spec = fs.make_qword_eq_matcher(vt_abs)
    t0 = time.time()
    hits = fs.parallel_scan(pid, arena[0], arena[1], workers=workers, **spec)
    dt = time.time() - t0
    print("[obj] vtable 0x%X: %d object hits in %.1fs" % (vt_abs, len(hits), dt))
    if not hits:
        return {}, []
    p = _wm.Process(pid, "hoi4.exe")
    id2obj = {}
    id_counts = Counter()
    all_ids = []
    try:
        for h in hits:
            raw = p.try_read(h + REGION_ID_OFF, 4)
            if len(raw) != 4:
                continue
            rid = int(np.frombuffer(raw, "<u4")[0])
            all_ids.append(rid)
            id_counts[rid] += 1
            # keep first object per id; collisions noted below
            if rid not in id2obj:
                id2obj[rid] = h
    finally:
        p.close()
    in_range = [r for r in all_ids if 1 <= r <= 304]
    print("[obj] ids read: %d hits, %d with id in 1..304, %d distinct in-range"
          % (len(all_ids), len(in_range), len(set(in_range))))
    dups = {r: c for r, c in id_counts.items() if c > 1 and 1 <= r <= 304}
    if dups:
        print("[obj] WARN duplicate region ids (id:count):",
              dict(list(dups.items())[:20]))
    return id2obj, hits


def sweep_temp_field(pid, id2obj):
    """For each candidate body offset, read f32 across the 5 anchor objects under
    several encodings; report offsets where all 5 match within tol. Returns sorted
    list of (offset, encoding, [readbacks], maxerr)."""
    p = _wm.Process(pid, "hoi4.exe")
    # cache each anchor object's body bytes once
    bodies = {}
    try:
        for (rid, _t, _r) in ANCHORS:
            if rid not in id2obj:
                bodies[rid] = None
                continue
            bodies[rid] = p.try_read(id2obj[rid], BODY_SCAN)
    finally:
        p.close()

    missing = [rid for (rid, _, _) in ANCHORS if not bodies.get(rid)]
    if missing:
        print("[temp] MISSING anchor objects for region ids:", missing)

    encs = [
        ("f32_celsius", "<f4", 4, lambda a: a.astype(np.float64)),
        ("f32_kelvin",  "<f4", 4, lambda a: a.astype(np.float64) - _K),
        ("i32_x100",    "<i4", 4, lambda a: a.astype(np.float64) / 100.0),
        ("i32_x1000",   "<i4", 4, lambda a: a.astype(np.float64) / 1000.0),
        ("i32_x100000", "<i4", 4, lambda a: a.astype(np.float64) / 100000.0),
        ("i32_raw",     "<i4", 4, lambda a: a.astype(np.float64)),
        ("i16_x100",    "<i2", 2, lambda a: a.astype(np.float64) / 100.0),
        ("i16_x10",     "<i2", 2, lambda a: a.astype(np.float64) / 10.0),
        ("i16_raw",     "<i2", 2, lambda a: a.astype(np.float64)),
        ("f16_celsius", "<f2", 2, lambda a: a.astype(np.float64)),
    ]
    expect = {rid: t for (rid, t, _r) in ANCHORS}
    tol = 1.5
    matches = []
    for (ename, dt, eb, conv) in encs:
        for off in range(0, BODY_SCAN - eb + 1):
            ok = True
            reads = {}
            maxerr = 0.0
            for (rid, _t, _r) in ANCHORS:
                body = bodies.get(rid)
                if not body or off + eb > len(body):
                    ok = False
                    break
                raw = body[off:off + eb]
                try:
                    v = float(conv(np.frombuffer(raw, dt))[0])
                except Exception:
                    ok = False
                    break
                if not np.isfinite(v):
                    ok = False
                    break
                reads[rid] = round(v, 3)
                err = abs(v - expect[rid])
                if err > tol:
                    ok = False
                    break
                maxerr = max(maxerr, err)
            if ok:
                matches.append((off, ename, reads, round(maxerr, 3)))
    matches.sort(key=lambda m: m[3])
    return matches


def sweep_precip_field(pid, id2obj, temp_off=None):
    """Find an offset where the 4 clear anchors share a value and region 31
    differs (u8/u16/u32)."""
    p = _wm.Process(pid, "hoi4.exe")
    bodies = {}
    try:
        for rid in CLEAR_RIDS + [RAIN_RID]:
            bodies[rid] = p.try_read(id2obj[rid], BODY_SCAN) if rid in id2obj else None
    finally:
        p.close()
    out = []
    for (width, dt) in ((1, "<u1"), (2, "<u2"), (4, "<u4")):
        for off in range(0, BODY_SCAN - width + 1):
            try:
                clears = []
                for rid in CLEAR_RIDS:
                    b = bodies[rid]
                    if not b or off + width > len(b):
                        raise ValueError
                    clears.append(int(np.frombuffer(b[off:off + width], dt)[0]))
                br = bodies[RAIN_RID]
                if not br or off + width > len(br):
                    raise ValueError
                rainv = int(np.frombuffer(br[off:off + width], dt)[0])
            except ValueError:
                continue
            if len(set(clears)) == 1 and rainv != clears[0]:
                cv = clears[0]
                plausible = (cv == 0 and 1 <= rainv <= 7)
                out.append({"off": off, "width": width, "clear_val": cv,
                            "rain_val": rainv, "plausible_enum": bool(plausible),
                            "dist_from_temp": (off - temp_off) if temp_off is not None else None})
    # prefer plausible enums close to temp field
    out.sort(key=lambda d: (not d["plausible_enum"],
                            abs(d["dist_from_temp"]) if d["dist_from_temp"] is not None else 9999))
    return out


def validate_all_regions(pid, id2obj, temp_off, enc):
    """Read the temp field at temp_off for ALL region objects; return stats +
    climatology cross-check."""
    dtmap = {"f32_celsius": ("<f4", 4, lambda a: a.astype(np.float64)),
             "f32_kelvin": ("<f4", 4, lambda a: a.astype(np.float64) - _K),
             "i32_x100": ("<i4", 4, lambda a: a.astype(np.float64) / 100.0),
             "i32_x1000": ("<i4", 4, lambda a: a.astype(np.float64) / 1000.0),
             "i32_x100000": ("<i4", 4, lambda a: a.astype(np.float64) / 100000.0),
             "i16_x100": ("<i2", 2, lambda a: a.astype(np.float64) / 100.0)}
    dt, eb, conv = dtmap[enc]
    p = _wm.Process(pid, "hoi4.exe")
    vals = {}
    try:
        for rid, obj in id2obj.items():
            raw = p.try_read(obj + temp_off, eb)
            if len(raw) == eb:
                v = float(conv(np.frombuffer(raw, dt))[0])
                if np.isfinite(v):
                    vals[rid] = v
    finally:
        p.close()
    arr = np.array([vals[r] for r in vals if 1 <= r <= 304])
    stats = {"n": int(arr.size), "min": round(float(arr.min()), 2) if arr.size else None,
             "max": round(float(arr.max()), 2) if arr.size else None,
             "mean": round(float(arr.mean()), 2) if arr.size else None,
             "std": round(float(arr.std()), 2) if arr.size else None}
    # climatology pearson (May, month idx 4)
    pear = None
    try:
        climo_path = os.path.join(_HERE, "region_weather_mod.json")
        d = {int(k): v for k, v in json.load(open(climo_path)).items()}
        xs, ys = [], []
        for rid, info in d.items():
            per = info.get("periods", [])
            if not per or rid not in vals:
                continue
            pp = per[4] if len(per) > 4 else per[-1]
            if pp.get("tmin") is not None:
                xs.append((pp["tmin"] + pp["tmax"]) / 2.0)
                ys.append(vals[rid])
        if len(xs) > 50:
            xs = np.array(xs); ys = np.array(ys)
            if xs.std() > 0 and ys.std() > 0:
                pear = round(float(np.corrcoef(xs, ys)[0, 1]), 3)
    except Exception as e:
        pear = "err:%s" % e
    return stats, pear, vals


if __name__ == "__main__":
    pid = _wm.find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe not running")
    p = _wm.Process(pid, "hoi4.exe")
    mb = p.module_base
    fp = p.fingerprint().get("sha256")
    p.close()
    if fp != EXPECTED_SHA:
        print("WARN build sha mismatch: %s" % fp)
    arena = fs.discover_arena(pid)
    print("pid %d module_base 0x%X arena 0x%X..0x%X (%.2f GB)"
          % (pid, mb, arena[0], arena[1], (arena[1] - arena[0]) / 2**30))

    id2obj, hits = enumerate_region_objects(pid, mb, arena, workers=6)
    present = [rid for (rid, _, _) in ANCHORS if rid in id2obj]
    print("[obj] anchor objects present: %s of 5  -> %s"
          % (len(present), present))
    for (rid, t, _r) in ANCHORS:
        if rid in id2obj:
            print("   region %3d obj 0x%X  (expect %sC)" % (rid, id2obj[rid], t))
        else:
            print("   region %3d MISSING" % rid)

    if len(present) < 5:
        print("[obj] not all anchor objects found; temp sweep would be unreliable.")
    matches = sweep_temp_field(pid, id2obj)
    print("[temp] %d offset/encoding combos match all 5 anchors:" % len(matches))
    for (off, enc, reads, maxerr) in matches[:30]:
        print("   off +0x%03X enc=%-12s maxerr=%.2f reads=%s"
              % (off, enc, maxerr, reads))

    # dump for the report
    out = {"pid": pid, "module_base": "0x%X" % mb,
           "arena": ["0x%X" % arena[0], "0x%X" % arena[1]],
           "vtable_abs": "0x%X" % (mb + REGION_VTABLE_OFF),
           "n_object_hits": len(hits), "n_distinct_ids": len(id2obj),
           "anchors_present": present,
           "anchor_objs": {str(r): "0x%X" % id2obj[r] for r in present},
           "temp_matches": [{"off": "0x%X" % m[0], "enc": m[1], "reads": m[2],
                             "maxerr": m[3]} for m in matches[:50]]}
    _publish_json_atomic(os.path.join(_HERE, "_region_obj_result.json"), out)
    print("wrote _region_obj_result.json")
