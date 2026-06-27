"""validate_snow_usa1936.py - STRICT live==save proof for per-province SNOW
(SWeatherPerProvince + 0x40, i32/1e5) against the USA-1936 same-frame melt oracle.

This is the advisor-style coincidence-free validation for Deliverable A. The live
game (hoi4.exe, build 16022851) is PAUSED at the exact frame the oracle melt was
taken (USA_1936_02_07_06), so the per-province snow array MUST match the save's
weather.provinces.<pid>.snow EXACTLY at the raw-integer level.

PROOF DESIGN (per the orchestrator's Codex A-round)
---------------------------------------------------
1. RAW-INTEGER compare (the only honest fixed-point test): the live i32 at
   struct+0x40 must equal round(save_snow_decimal * 100000) exactly (we allow a
   <=+-1 raw-unit slack for the melt's last-digit rounding only). No float epsilon.
2. FULL ARRAY over all 13414 provinces:
     FP = live nonzero where save zero/None
     FN = save nonzero where live zero
     value-mismatch = both nonzero but raw delta > 1
   REQUIRE FP == 0, FN == 0, value-mismatch == 0.
3. NEGATIVE CONTROLS - each MUST FAIL to match the snow GT, proving +0x40 is
   UNIQUELY snow (not a coincidence of a neighbouring field):
     a. neighbour offsets +0x44, +0x3C       (must NOT match snow GT)
     b. water +0x38, temp +0x118              (must NOT match snow GT)
     c. province-shift live[pid] vs save[pid+-1] (must NOT match)
     d. shuffled-pid live[pid] vs save[random] (match rate ~= chance, not ~100%)

READ-ONLY: ReadProcessMemory only; the game is never written and stays PAUSED.
This module does ONE bulk read of the per-province array (it is the sole scanner
while it runs) and then computes every comparison offline from that buffer.
"""
import argparse
import json
import os
import random
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_MR = os.path.dirname(_HERE)
_REPO = os.path.dirname(_MR)
for _p in (_REPO, _MR, _HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import winmem as wm          # noqa: E402
import fastscan as fs        # noqa: E402
from weather_temp_model import (  # noqa: E402
    EXPECTED_SHA, WMGR_VTABLE_RVA, MGR_PROV_ARRAY_OFF, PERPROV_STRIDE,
    PROV_ID_OFF, SNOW_OFF, WATER_OFF, TEMP_OFF, FIXED_POINT_SCALE,
    locate_manager, _isheap,
)

# default oracle: the pre-parsed USA-1936 GT (re_scratch/_w2wx_gt.json). Falls back
# to re-parsing the raw melt via re_scratch/_w2wx_gt.py if the json is absent.
_GT_JSON = os.path.join(_REPO, "re_scratch", "_w2wx_gt.json")

RAW_SLACK = 1                 # allowed |live_raw - save_raw| (melt last-digit rounding)


def _save_raw(decimal):
    """save snow decimal -> the i32 raw the engine stores (round half-to-even * 1e5)."""
    if decimal is None:
        return 0
    return int(round(decimal * FIXED_POINT_SCALE))


def load_gt(path=_GT_JSON):
    """Load the per-province snow GT as {pid:int -> save_raw:int} (nonzero only kept
    in `nonzero`, but `raw` holds ALL provinces incl. zeros for FP detection)."""
    if os.path.isfile(path):
        gt = json.load(open(path))
        prov = gt["provinces"]
        raw = {}
        for pid, rec in prov.items():
            raw[int(pid)] = _save_raw(rec.get("snow"))
        return raw
    # fallback: parse the melt directly
    sys.path.insert(0, os.path.join(_REPO, "re_scratch"))
    import _w2wx_gt as gtmod  # noqa: E402
    text = open(gtmod.ORACLE, "r", encoding="utf-8", errors="replace").read()
    block = gtmod.find_weather_block(text)
    psub = gtmod.split_subblock(block, "provinces")
    recs = gtmod.parse_entities(psub, "province")
    raw = {}
    for pid, lines in recs.items():
        p = gtmod.parse_province(lines)
        raw[int(pid)] = _save_raw(p.get("snow"))
    return raw


def read_perprov_buffer(pid=None):
    """Read the WHOLE per-province weather array ONCE. Returns
    (begin, size, body_by_index{k:bytes}) or None on failure. Sole-scanner read."""
    if pid is None:
        pid = wm.find_pid("hoi4.exe")
        if pid is None:
            return None
    p = wm.Process(pid, "hoi4.exe")
    mb = p.module_base
    fp = p.fingerprint().get("sha256")
    p.close()
    if fp != EXPECTED_SHA:
        print("WARN build sha mismatch: %s (expected %s)" % (fp, EXPECTED_SHA))
    arena = fs.discover_arena(pid)
    if arena is None:
        return None
    mgr = locate_manager(pid, mb, arena)
    if mgr is None:
        return None
    p = wm.Process(pid, "hoi4.exe")
    try:
        hdr = p.try_read(mgr + MGR_PROV_ARRAY_OFF, 16)
        if len(hdr) < 16:
            return None
        begin, size, cap = struct.unpack("<QII", hdr)
        if not _isheap(begin) or not (1 <= size <= 100000) or cap < size:
            return None
        bodies = {}
        for k in range(size):
            body = p.try_read(begin + k * PERPROV_STRIDE, PERPROV_STRIDE)
            if len(body) >= PERPROV_STRIDE:
                bodies[k] = body
        return {"manager": mgr, "begin": begin, "size": size, "bodies": bodies}
    finally:
        p.close()


def _i32(body, off):
    return struct.unpack_from("<i", body, off)[0]


def _live_raw_at(bodies, pid, off):
    """live i32 at struct+off for the per-province struct indexed by pid, or None."""
    body = bodies.get(pid)
    if body is None or off + 4 > len(body):
        return None
    return _i32(body, off)


def validate(gt_raw, buf):
    """Run the full snow proof. Returns a results dict (all numbers captured)."""
    bodies = buf["bodies"]
    size = buf["size"]

    # ---- index==province_id structural check (the array is keyed by index) ----
    id_ok = id_bad = 0
    for k, body in bodies.items():
        pid_read = struct.unpack_from("<H", body, PROV_ID_OFF)[0]
        if pid_read == k:
            id_ok += 1
        else:
            id_bad += 1

    # ---- PRIMARY: snow @ +0x40 raw-integer full-array compare ----
    fp = fn = mism = exact = 0
    matched_nonzero = 0
    max_abs_delta = 0
    nonzero_live = []
    nonzero_save_total = 0
    distinct_nonzero = set()
    mismatch_samples = []
    # union of all province ids in GT and in the live array
    all_pids = set(gt_raw.keys()) | set(bodies.keys())
    for pid in all_pids:
        sraw = gt_raw.get(pid, 0)
        lraw = _live_raw_at(bodies, pid, SNOW_OFF)
        if lraw is None:
            lraw = 0  # province not in live array -> treat as zero (FN if save nonzero)
        if sraw != 0:
            nonzero_save_total += 1
            distinct_nonzero.add(sraw)
        if lraw != 0:
            nonzero_live.append(pid)
        if sraw == 0 and lraw != 0:
            fp += 1
            if len(mismatch_samples) < 8:
                mismatch_samples.append(("FP", pid, sraw, lraw))
        elif sraw != 0 and lraw == 0:
            fn += 1
            if len(mismatch_samples) < 8:
                mismatch_samples.append(("FN", pid, sraw, lraw))
        elif sraw != 0 and lraw != 0:
            d = abs(lraw - sraw)
            max_abs_delta = max(max_abs_delta, d)
            if d <= RAW_SLACK:
                exact += 1
                matched_nonzero += 1
            else:
                mism += 1
                if len(mismatch_samples) < 8:
                    mismatch_samples.append(("MISM", pid, sraw, lraw))

    primary = {
        "n_save_nonzero": nonzero_save_total,
        "n_distinct_nonzero": len(distinct_nonzero),
        "n_live_nonzero": len(nonzero_live),
        "matched_nonzero": matched_nonzero,
        "FP": fp, "FN": fn, "value_mismatch": mism,
        "max_abs_raw_delta": max_abs_delta,
        "mismatch_samples": mismatch_samples,
        "snow_min_save": (min(distinct_nonzero) if distinct_nonzero else None),
        "snow_max_save": (max(distinct_nonzero) if distinct_nonzero else None),
    }

    # ---- NEGATIVE CONTROLS ----
    # helper: count exact raw matches of live[pid]@off vs save snow[pid], over the
    # provinces that have a nonzero SAVE snow (the discriminating set).
    gt_nonzero_pids = [pid for pid, r in gt_raw.items() if r != 0]

    def match_rate_at_offset(off):
        m = 0
        n = 0
        for pid in gt_nonzero_pids:
            lraw = _live_raw_at(bodies, pid, off)
            if lraw is None:
                continue
            n += 1
            if abs(lraw - gt_raw[pid]) <= RAW_SLACK:
                m += 1
        return m, n

    neg = {}
    for label, off in (("offset+0x44", SNOW_OFF + 4),
                       ("offset+0x3C", SNOW_OFF - 4),
                       ("water+0x38", WATER_OFF),
                       ("temp+0x118", TEMP_OFF)):
        m, n = match_rate_at_offset(off)
        neg[label] = {"match": m, "n": n,
                      "rate": (m / n if n else None)}

    # province-shift: live[pid] (snow off) vs save[pid+1] and save[pid-1]
    def shift_match(delta):
        m = 0
        n = 0
        for pid in gt_nonzero_pids:
            # compare the live snow at THIS pid against the save snow of the SHIFTED pid
            lraw = _live_raw_at(bodies, pid, SNOW_OFF)
            sraw_shift = gt_raw.get(pid + delta)
            if lraw is None or sraw_shift is None or sraw_shift == 0:
                continue
            n += 1
            if abs(lraw - sraw_shift) <= RAW_SLACK:
                m += 1
        return m, n

    for label, delta in (("save[pid+1]", 1), ("save[pid-1]", -1)):
        m, n = shift_match(delta)
        neg[label] = {"match": m, "n": n, "rate": (m / n if n else None)}

    # shuffled-pid: live[pid] snow vs save[random_other_pid] snow
    rng = random.Random(1936)
    shuffled = list(gt_nonzero_pids)
    rng.shuffle(shuffled)
    m = n = 0
    for i, pid in enumerate(gt_nonzero_pids):
        other = shuffled[i]
        if other == pid:
            other = shuffled[(i + 1) % len(shuffled)]
        lraw = _live_raw_at(bodies, pid, SNOW_OFF)
        sraw_other = gt_raw.get(other)
        if lraw is None or sraw_other is None or sraw_other == 0:
            continue
        n += 1
        if abs(lraw - sraw_other) <= RAW_SLACK:
            m += 1
    neg["shuffled_pid"] = {"match": m, "n": n, "rate": (m / n if n else None)}

    return {
        "index_is_province_id": {"ok": id_ok, "bad": id_bad},
        "primary": primary,
        "negative_controls": neg,
        "size": size,
        "n_live_structs": len(bodies),
    }


def wrong_oracle_check(wrong_gt_raw, buf):
    """ANTI-CONTAMINATION control (Codex Q1): compare the SAME live memory snow against
    a DIFFERENT-date save's snow GT. It MUST fail loudly -- a low match rate proves the
    live read is NOT being filled from / aliasing the (correct) oracle, and that the
    correct-frame match is content, not a self-comparison artefact.

    Returns {match, n, rate, matched_correct_frame_rate≈1.0_expected_only_for_RIGHT_oracle}.
    A high rate here would mean the two saves happen to share snow OR (the real worry) the
    live buffer is contaminated; we report the raw numbers so the caller can judge."""
    bodies = buf["bodies"]
    wrong_nonzero = [pid for pid, r in wrong_gt_raw.items() if r != 0]
    m = n = 0
    for pid in wrong_nonzero:
        lraw = _live_raw_at(bodies, pid, SNOW_OFF)
        if lraw is None:
            continue
        n += 1
        if abs(lraw - wrong_gt_raw[pid]) <= RAW_SLACK:
            m += 1
    return {"match": m, "n": n, "rate": (m / n if n else None),
            "n_wrong_nonzero": len(wrong_nonzero)}


def load_gt_from_melt(path):
    """Parse snow GT {pid:int -> save_raw:int} directly from a raw melt .txt (for the
    wrong-oracle control). Uses the same brace-scoped parser as the USA-1936 GT."""
    sys.path.insert(0, os.path.join(_REPO, "re_scratch"))
    import _w2wx_gt as gtmod  # noqa: E402
    text = open(path, "r", encoding="utf-8", errors="replace").read()
    block = gtmod.find_weather_block(text)
    if not block:
        return {}
    psub = gtmod.split_subblock(block, "provinces")
    recs = gtmod.parse_entities(psub, "province")
    raw = {}
    for pid, lines in recs.items():
        p = gtmod.parse_province(lines)
        raw[int(pid)] = _save_raw(p.get("snow"))
    return raw


def verdict(res):
    """PASS iff FP==0, FN==0, value_mismatch==0, snow uniquely matches at +0x40, and
    every negative control fails to match (rate well below 1.0)."""
    p = res["primary"]
    primary_pass = (p["FP"] == 0 and p["FN"] == 0 and p["value_mismatch"] == 0
                    and p["matched_nonzero"] == p["n_save_nonzero"]
                    and p["n_save_nonzero"] > 0)
    neg = res["negative_controls"]
    # each negative control MUST NOT reproduce the ~100% snow match. We require its
    # match rate be < 0.5 (in practice these are ~0). The snow offset itself is not
    # in this dict, so any high rate here would be a coincidence/aliasing failure.
    neg_pass = all((v["rate"] is None or v["rate"] < 0.5) for v in neg.values())
    return primary_pass and neg_pass, primary_pass, neg_pass


def main():
    ap = argparse.ArgumentParser(description="STRICT snow live==save proof (USA-1936).")
    ap.add_argument("--json", action="store_true", help="dump the full result dict as JSON")
    ap.add_argument("--gt", default=_GT_JSON, help="path to the snow GT json")
    ap.add_argument("--wrong-oracle", default=None,
                    help="path to a DIFFERENT-date melt .txt; the live memory MUST fail "
                         "to match it (anti-contamination control)")
    args = ap.parse_args()

    gt_raw = load_gt(args.gt)
    buf = read_perprov_buffer()
    if buf is None:
        raise SystemExit("could not read the per-province weather array "
                         "(game not running / build mismatch / manager not found)")
    res = validate(gt_raw, buf)
    if args.wrong_oracle:
        wrong = load_gt_from_melt(args.wrong_oracle)
        res["wrong_oracle"] = wrong_oracle_check(wrong, buf)
        res["wrong_oracle"]["melt"] = os.path.basename(args.wrong_oracle)
    ok, primary_pass, neg_pass = verdict(res)

    if args.json:
        print(json.dumps(res, indent=2))
        raise SystemExit(0 if ok else 1)

    p = res["primary"]
    print("CWeatherManager @ 0x%X  array size=%d  live structs read=%d"
          % (buf["manager"], res["size"], res["n_live_structs"]))
    print("index==province_id: ok=%d bad=%d"
          % (res["index_is_province_id"]["ok"], res["index_is_province_id"]["bad"]))
    print()
    print("=== PRIMARY snow @ +0x%X raw-integer full-array compare (13414 provinces) ==="
          % SNOW_OFF)
    print("  save nonzero-snow provinces : %d" % p["n_save_nonzero"])
    print("  distinct nonzero snow values: %d" % p["n_distinct_nonzero"])
    print("  live nonzero-snow provinces : %d" % p["n_live_nonzero"])
    print("  matched (|raw delta|<=%d)    : %d / %d"
          % (RAW_SLACK, p["matched_nonzero"], p["n_save_nonzero"]))
    print("  FP (live!=0, save==0)        : %d" % p["FP"])
    print("  FN (save!=0, live==0)        : %d" % p["FN"])
    print("  value-mismatch (delta>%d)    : %d" % (RAW_SLACK, p["value_mismatch"]))
    print("  max |raw delta|              : %d" % p["max_abs_raw_delta"])
    print("  save snow raw range          : %s .. %s"
          % (p["snow_min_save"], p["snow_max_save"]))
    if p["mismatch_samples"]:
        print("  mismatch samples (kind,pid,save_raw,live_raw):")
        for s in p["mismatch_samples"]:
            print("     %s" % (s,))
    print()
    print("=== NEGATIVE CONTROLS (each MUST fail to match the snow GT) ===")
    for label, v in res["negative_controls"].items():
        rate = "n/a" if v["rate"] is None else ("%.4f" % v["rate"])
        flag = "OK-fails" if (v["rate"] is None or v["rate"] < 0.5) else "!! MATCHES !!"
        print("  %-14s match %5d / %5d  rate=%s   %s"
              % (label, v["match"], v["n"], rate, flag))
    if "wrong_oracle" in res:
        wo = res["wrong_oracle"]
        rate = "n/a" if wo["rate"] is None else ("%.4f" % wo["rate"])
        flag = ("OK-fails" if (wo["rate"] is None or wo["rate"] < 0.5)
                else "!! CONTAMINATION: matches a WRONG-date oracle !!")
        print("  wrong-oracle[%s] match %5d / %5d  rate=%s   %s"
              % (wo["melt"], wo["match"], wo["n"], rate, flag))
    print()
    print("PRIMARY pass: %s | NEGATIVE-CONTROLS pass: %s | OVERALL: %s"
          % (primary_pass, neg_pass, "PASS" if ok else "FAIL"))
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
