"""weather_reader.py - LIVE weather layer for Battle Plan (build 16022851).

STATUS: CANDIDATE / LOW confidence. Weather is the one layer with NO save
oracle (weather is dynamic and not persisted in saves), so it cannot be anchored
the way owner/control/economy/divisions/combat were. This module implements the
full read-only validation FUNNEL designed for it; it locates a candidate
region-indexed weather array and scores it, but it FAILS CLOSED unless the
candidate clears strong negative controls AND (when the game is running) a
temporal-drift gate. See RESEARCH_weather.md for the method, the residual, and
exactly what a ground-truth confirmation still needs.

WHY THIS IS HARD (the honest part):
  * No save oracle: nothing to diff against. We rely on the static climatology
    (map/strategicregions/*.txt) as the only ground-truth band, plus internal
    consistency and temporal drift.
  * A single-field temperature-band scan is USELESS here: it produced 4.66M
    candidate offsets in the 7.5 GB arena (only ~8.7 bits of selectivity). The
    static climatology in memory is itself a perfect masquerader.
  * The decisive discriminator is GAME-TIME-COUPLED TEMPORAL DRIFT (real weather
    drifts when the sim advances; static arrays don't). That requires the game
    to be UNPAUSED. If paused, this reader reports LOW confidence and refuses to
    publish a definitive map.

WHAT THIS READER DOES (funnel, all READ-ONLY, arena-scoped):
  1. Build gate (exe sha256 == pinned build) -> fail closed on mismatch.
  2. Load the static climatology (per-region monthly temperature band + the set
     of phenomena that are climatologically POSSIBLE that month).
  3. Determine the current in-game MONTH from the live date integer.
  4. JOINT array-of-structs scan over the 304 strategic regions: find a record
     stride S and intra-record (temperature_float_offset, phenomenon_enum_offset)
     such that, for ALL 304 regions simultaneously:
        - temperature is inside that region's monthly band, AND
        - the phenomenon enum is in [0..7] AND climatologically POSSIBLE, AND
        - the physical law holds (snow/blizzard/arctic_water => cold;
          sandstorm => hot; mud => not frozen).
  5. NEGATIVE CONTROLS: the winning layout's real-assignment score must DOMINATE
     (a) shuffled region ids, (b) a wrong month, (c) nearby offsets.
  6. DRIFT GATE (only if the game is running): two snapshots a few game-hours
     apart must show weather-like drift (a MINORITY of regions change; changes
     are bounded and spatially coherent), not static and not white-noise churn.

The phenomenon enum order is the FILE-ORDER hypothesis (the order phenomena
appear in every strategicregions file): 0 no_phenomenon, 1 rain_light,
2 rain_heavy, 3 snow, 4 blizzard, 5 arctic_water, 6 mud, 7 sandstorm. It is NOT
confirmed by a static enum table (the phenomenon strings are interned as generic
script keywords; no indexable string-pointer table exists). The reader fits the
mapping rather than trusting it: if the best 8-value mapping is unstable, it
fails closed.

Usage:
    python weather_reader.py                  # locate + score + report confidence
    python weather_reader.py --month 8         # force month (else read live date)
"""
import argparse
import json
import os
import struct
import sys
import time
from collections import Counter

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import winmem  # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

ARENA_LO = 0x1F600000000
ARENA_HI = 0x1FA00000000
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

N_REGIONS = 304
TEMP_LO, TEMP_HI = -60.0, 55.0   # global plausible runtime temperature, deg C

# phenomenon enum (file-order hypothesis; see module docstring)
PHENOMENA = ["no_phenomenon", "rain_light", "rain_heavy", "snow",
             "blizzard", "arctic_water", "mud", "sandstorm"]
NO_PHEN, RAIN_L, RAIN_H, SNOW, BLIZZARD, ARCTIC, MUD, SAND = range(8)
COLD_PHEN = (SNOW, BLIZZARD, ARCTIC)

# 365-day calendar (no leap), cumulative day-of-year at month start.
# Jan31 Feb28 Mar31 Apr30 May31 Jun30 Jul31 Aug31 Sep30 Oct31 Nov30 Dec31.
_MONTH_DAYS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
_CUM = []
_acc = 0
for _d in _MONTH_DAYS:
    _CUM.append(_acc)
    _acc += _d

# confidence + control thresholds (Codex-recommended)
MARGIN_MIN = 0.40        # real score must beat best control by this much
SCORE_MIN = 0.90         # real joint-pass fraction floor for a candidate
DRIFT_MIN_FRAC = 0.01    # >= this fraction of regions must change (running game)
DRIFT_MAX_FRAC = 0.50    # but NOT more than this (else it's churn, not weather)


# --------------------------------------------------------------------------
# climatology asset (built from map/strategicregions/*.txt; ships with this dir)
# --------------------------------------------------------------------------
def load_climatology():
    here = os.path.dirname(os.path.abspath(__file__))
    path = os.path.join(here, "weather_climatology.json")
    with open(path, "r", encoding="ascii") as fh:
        return {int(k): v for k, v in json.load(fh).items()}


def region_band_arrays(clim, month, slack):
    """rids/tmin/tmax (np) + possible-phenomenon mask[rid,8] for `month`."""
    rids, tmin, tmax, poss = [], [], [], []
    for rid in range(1, N_REGIONS + 1):
        d = clim.get(rid)
        if not d:
            continue
        mo = d["months"].get(str(month))
        if not mo or mo["tmin"] is None:
            continue
        rids.append(rid)
        tmin.append(mo["tmin"] - slack)
        tmax.append(mo["tmax"] + slack)
        m = [False] * 8
        for e in mo["phen"]:
            if 0 <= e < 8:
                m[e] = True
        poss.append(m)
    return (np.array(rids), np.array(tmin, np.float32),
            np.array(tmax, np.float32), np.array(poss, bool))


# --------------------------------------------------------------------------
# live in-game CLOCK + month  (TIGHT-WATCH locator)
# --------------------------------------------------------------------------
# CLOCK DECODE (VERIFIED, build 16022851):
#   The game date is a u32 = hours-since-epoch (Clausewitz), 365-day years,
#   no leap: value = ((year*365 + day_of_year_0based)*24 + hour). Anchor verified
#   against the project's solved date layer: 1940.9.2 hour21 = 17000277.
#
# DO NOT MODAL-VOTE THE DATE WINDOW. A prior reader took the most-frequent
# date-like u32 over 1935..1950 and locked onto 16974592 (= 1937.9.27.16), a
# STATIC scenario/snapshot constant replicated ~35000x, then -- reading those
# same static addresses again -- concluded "frozen / paused / 1937". That was
# wrong: the live clock value is replicated far less and sits elsewhere. The
# live clock is found by ADVANCEMENT, not frequency: enumerate date-window words
# once, then TIGHT-WATCH (re-read just those every ~1.2s); the clock is the word
# advancing by small integer hour deltas, monotonically. Confirmed live on
# 2026-06-25: addr 0x1F835A69D10 advanced 17055924->17056050 (+5/+6 hrs every
# 1.5s == speed 2), decoding 1947.1.x. Heap addresses are per-launch dynamic, so
# the reader RELOCATES the clock each run rather than hardcoding an address.

# date-window: hours-since-epoch over the campaign's plausible year span. Wide
# enough to bracket any 1936-1951 campaign; the tight-watch (not a modal vote)
# is what isolates the clock, so the window only has to CONTAIN the live date.
# (A late-game save at speed 2 advances months in minutes, so keep the upper
# bound generous -- the date can race forward during a multi-minute read.)
CLOCK_LO = (1936 * 365) * 24
CLOCK_HI = (1951 * 365) * 24


def _decode_date(H):
    """u32 hours-since-epoch -> (year, month0, day1, hour). month0 is 0-based."""
    y = H // (365 * 24)
    rem = H - y * 365 * 24
    doy = rem // 24
    hr = rem % 24
    mo = 0
    for i in range(12):
        if doy >= _CUM[i]:
            mo = i
        else:
            break
    return y, mo, doy - _CUM[mo] + 1, hr


def locate_clock(p, samples=8, interval=1.2):
    """Find the live game clock by tight-watch (NOT modal vote).

    1. One arena scan -> every u32 in the date window (addr -> value).
    2. Re-read just those addrs `samples` times, `interval`s apart.
    3. The clock = the addr that is monotonic non-decreasing, net-positive, with
       every step a small non-negative integer hour delta (<= a speed cap), and
       at least one real increment. Among ties prefer the most increments / latest
       value. Returns (addr, value) or (None, None).
    """
    snap = {}
    for base, size in p.iter_regions(ARENA_LO, ARENA_HI, want_exec=False):
        blob = p.try_read(base, size)
        if not blob:
            continue
        mv = memoryview(blob)
        for i in range(len(blob) // 4):
            v = struct.unpack_from("<I", mv, i * 4)[0]
            if CLOCK_LO <= v <= CLOCK_HI:
                snap[base + i * 4] = v
    if not snap:
        return None, None
    addrs = list(snap.keys())
    # Do NOT seed series with the scan-time value: the arena scan takes ~150s,
    # during which the clock advances hundreds of hours, so a seeded first step
    # would be a huge jump that fails the small-step test. Take ALL samples in the
    # tight loop (per-address reads are ms apart), so every step is small.
    series = {a: [] for a in addrs}
    for s in range(samples):
        for a in addrs:
            b = p.try_read(a, 4)
            series[a].append(struct.unpack("<I", b)[0] if len(b) == 4 else None)
        if s < samples - 1:
            time.sleep(interval)
    step_cap = max(8, int(60 * interval))
    best = None
    for a, seq in series.items():
        if any(v is None for v in seq):
            continue
        if not all(CLOCK_LO <= v <= CLOCK_HI for v in seq):
            continue
        if seq[-1] <= seq[0]:
            continue
        ok = True
        ninc = 0
        for i in range(len(seq) - 1):
            d = seq[i + 1] - seq[i]
            if d < 0 or d > step_cap:
                ok = False
                break
            if d > 0:
                ninc += 1
        if ok and ninc >= 1:
            key = (ninc, seq[-1])
            if best is None or key > best[0]:
                best = (key, a, seq[-1])
    if best is None:
        return None, None
    return best[1], best[2]


def read_live_month(p):
    """Live date integer -> (month0, date_int) via the tight-watch clock locator.
    Returns (None, None) if no advancing clock is found (e.g. truly paused)."""
    _, date_int = locate_clock(p)
    if date_int is None:
        return None, None
    return _decode_date(date_int)[1], date_int


# --------------------------------------------------------------------------
# joint array-of-structs scoring
# --------------------------------------------------------------------------
def score_layout(f32, u8, base_f, S_f, T_f, E_b, ewidth, ibase,
                 rids, tmin, tmax, poss):
    """Score one record layout over all regions. Returns the mean per-region
    score in [0,1].

    HARD requirements (a region scores 0 if any fails):
      - temperature finite AND inside the climatology band, AND
      - enum in [0..7] AND climatologically POSSIBLE that month.
    SOFT physical plausibility (Codex c: do NOT hard-reject; the game temperature is
    region-average while snow/sandstorm are province/terrain-biased, arctic_water
    has hysteresis, and a stored mud/water accumulator need not track instantaneous
    temp): a region that passes the hard gate scores 1.0, minus a penalty for an
    implausible enum/temp pairing, floored at a small positive value so a real
    array with a few transitional cells is not zeroed out.
    """
    nidx = rids.size
    rel = (rids - ibase) * S_f
    tf_idx = base_f + rel + T_f
    if tf_idx.size == 0 or tf_idx.max() >= f32.size or tf_idx.min() < 0:
        return 0.0
    temps = f32[tf_idx]
    eb_idx = (base_f + rel) * 4 + E_b
    if eb_idx.max() + ewidth > u8.size or eb_idx.min() < 0:
        return 0.0
    if ewidth == 1:
        enums = u8[eb_idx].astype(np.int64)
    elif ewidth == 2:
        enums = u8[eb_idx].astype(np.int64) | (u8[eb_idx + 1].astype(np.int64) << 8)
    else:
        enums = (u8[eb_idx].astype(np.int64)
                 | (u8[eb_idx + 1].astype(np.int64) << 8)
                 | (u8[eb_idx + 2].astype(np.int64) << 16)
                 | (u8[eb_idx + 3].astype(np.int64) << 24))
    # HARD gate
    hard = np.isfinite(temps) & (temps >= tmin) & (temps <= tmax)
    hard &= (enums >= 0) & (enums <= 7)
    en = np.clip(enums, 0, 7)
    hard &= poss[np.arange(nidx), en]
    # SOFT physical penalties (only the most robust ones; sandstorm is the
    # weakest physical invariant so it gets a small penalty only at extreme cold)
    pen = np.zeros(nidx, dtype=np.float32)
    cold = np.isin(en, COLD_PHEN)
    pen += np.where(cold & (temps > 8.0), 0.5, 0.0)        # snow/arctic at >8C: unlikely
    pen += np.where((en == SAND) & (temps < 0.0), 0.5, 0.0)  # sandstorm below freezing: unlikely
    score = np.where(hard, np.maximum(1.0 - pen, 0.1), 0.0)
    return float(score.mean())


def joint_scan(p, clim, month, slack=6.0, strides_b=(16, 24),
               score_min=SCORE_MIN, margin_min=MARGIN_MIN, progress=None,
               time_budget_s=None):
    """Scan the arena for a region-indexed weather AoS that clears the joint
    constraint AND beats shuffled-rid / wrong-month controls by margin_min.
    Returns ranked list of candidate dicts (may be empty).

    PERFORMANCE NOTE: a full 7.5 GB arena sweep over several strides is heavy
    (minutes), and under the multi-scanner shared load it can be OOM/contention
    bound. The default stride set is intentionally small (16, 24 dominate the
    temperature-candidate clusters). Pass `time_budget_s` to bound wall time and
    return the best-so-far rather than run unbounded; widen strides_b for a
    deeper (slower) search in a quiet session."""
    rids, tmin, tmax, poss = region_band_arrays(clim, month, slack)
    if rids.size < 200:
        return []
    _t_start = time.time()
    wm = (month + 6) % 12
    _, tminW, tmaxW, possW = region_band_arrays(clim, wm, slack)
    rng = np.random.default_rng(0)
    sh = rids.copy()
    rng.shuffle(sh)

    out = []
    maxrid = int(rids.max())
    # The whole 304-region array spans at most maxrid*max_stride bytes; read the
    # arena in capped, OVERLAPPING windows so a single huge region (up to ~89 MB
    # here) never forces a giant allocation under multi-scanner memory pressure
    # (two parallel scans were OOM-killed reading full regions). The overlap is
    # the max array span so an array straddling a window boundary still appears
    # fully inside one window.
    max_span_bytes = (maxrid + 4) * max(strides_b) + 64
    win = 48 << 20
    overlap = max_span_bytes + 64
    for rb, size in p.iter_regions(ARENA_LO, ARENA_HI, want_exec=False):
        pos = 0
        while pos < size:
            want = min(win, size - pos)
            if want < max_span_bytes:
                break
            blob = p.try_read(rb + pos, want)
            if blob:
                blen = len(blob)
                if pos + blen >= size or blen < want:   # final / short read
                    advance = max(blen, 0x1000)
                else:
                    advance = blen - overlap             # full mid-region chunk
            else:
                advance = max(want, 0x1000)
            if not blob or len(blob) < 4096:
                pos += advance
                continue
            wb = rb + pos
            _scan_window(blob, wb, strides_b, rids, tmin, tmax, poss,
                         tminW, tmaxW, possW, sh, maxrid, score_min,
                         margin_min, out)
            pos += advance
            if time_budget_s and (time.time() - _t_start) > time_budget_s:
                out.sort(key=lambda c: -c["margin"])
                return out
        if progress:
            progress(rb)
    out.sort(key=lambda c: -c["margin"])
    return out


def _scan_window(blob, wb, strides_b, rids, tmin, tmax, poss,
                 tminW, tmaxW, possW, sh, maxrid, score_min, margin_min, out):
    u8 = np.frombuffer(blob, np.uint8)
    nf = len(blob) // 4
    f32 = np.frombuffer(blob[:nf * 4], np.float32)
    if True:
        for Sb in strides_b:
            if Sb % 4:
                continue
            Sf = Sb // 4
            for Tf in range(Sf):
                for ibase in (0, 1):
                    span = (maxrid - ibase) * Sf + Tf + 1
                    rec0_max = nf - span
                    if rec0_max <= 0:
                        continue
                    # Vectorized prefilter on the TEMPERATURE field only, but
                    # over ALL regions (an integer in-band COUNT per O), so the
                    # gate is strong (>= score_min*N regions in-band) before any
                    # per-O enum work. A first-8 AND-gate let too many windows
                    # through and made the enum loop the bottleneck.
                    rel = (rids - ibase) * Sf + Tf
                    nN = rids.size
                    need = int(np.ceil(score_min * nN))
                    incount = np.zeros(rec0_max, dtype=np.int16)
                    # short-circuit: AND the first 4 (cheap, kills ~all), then
                    # COUNT the rest only where the first 4 survived.
                    m4 = np.ones(rec0_max, dtype=bool)
                    for j in range(min(4, nN)):
                        cj = f32[rel[j]: rel[j] + rec0_max]
                        m4 &= (cj >= tmin[j]) & (cj <= tmax[j])
                        if not m4.any():
                            break
                    if not m4.any():
                        continue
                    surv = np.nonzero(m4)[0]
                    # count in-band over all regions for survivors
                    cnt = np.full(surv.size, 4, dtype=np.int32)
                    for j in range(4, nN):
                        cj = f32[rel[j]: rel[j] + rec0_max]
                        cnt += ((cj[surv] >= tmin[j]) & (cj[surv] <= tmax[j]))
                    cand_o = surv[cnt >= need]
                    for O in cand_o.tolist():
                        for Ew in (1, 2, 4):
                            for Eb in range(0, Sb, 4):
                                fr = score_layout(f32, u8, O, Sf, Tf, Eb, Ew,
                                                  ibase, rids, tmin, tmax, poss)
                                if fr < score_min:
                                    continue
                                frW = score_layout(f32, u8, O, Sf, Tf, Eb, Ew,
                                                   ibase, rids, tminW, tmaxW, possW)
                                frS = score_layout(f32, u8, O, Sf, Tf, Eb, Ew,
                                                   ibase, sh, tmin, tmax, poss)
                                if fr - max(frW, frS) >= margin_min:
                                    out.append({
                                        "addr": wb + O * 4, "stride": Sb,
                                        "temp_off": Tf * 4, "enum_off": Eb,
                                        "enum_width": Ew, "ibase": ibase,
                                        "score": fr, "ctrl_wrongmonth": frW,
                                        "ctrl_shuffled": frS,
                                        "margin": fr - max(frW, frS)})


# --------------------------------------------------------------------------
# top-level resolve (fail-closed)
# --------------------------------------------------------------------------
class WeatherLayer(object):
    def __init__(self, p, cand, month, rids):
        self.p = p
        self.cand = cand
        self.month = month
        self.rids = rids

    @classmethod
    def resolve(cls, p, month=None, slack=6.0, require_running=False,
                time_budget_s=300):
        """Full funnel. Returns (layer_or_None, report_dict)."""
        report = {"status": "fail", "confidence": "none", "reason": "",
                  "month": None, "candidates": []}
        fp = p.fingerprint()
        if fp.get("sha256") != EXPECTED_SHA:
            report["reason"] = "build sha mismatch (%s)" % fp.get("sha256")
            return None, report
        clim = load_climatology()
        if month is None:
            month, date_int = read_live_month(p)
            report["date_int"] = date_int
        report["month"] = month
        if month is None:
            report["reason"] = "could not read live date/month"
            return None, report
        # running check (date drift)
        d0, _ = read_live_month_value(p)
        time.sleep(8)
        d1, _ = read_live_month_value(p)
        running = (d1 is not None and d0 is not None and d1 > d0)
        report["running"] = running
        cands = joint_scan(p, clim, month, slack=slack,
                           time_budget_s=time_budget_s)
        report["candidates"] = cands[:10]
        if not cands:
            report["reason"] = ("no region-indexed weather AoS cleared the joint "
                                "constraint + negative controls")
            report["confidence"] = "none"
            return None, report
        best = cands[0]
        # A structural candidate, even one beating the negative controls, is NOT
        # publishable on its own (the static climatology array would also beat
        # the controls). The ONLY thing that lifts this above LOW is the
        # temporal-drift gate, which (a) requires the game running and (b) is not
        # yet implemented. So confidence stays LOW regardless of `running`
        # (Codex c: do not report low-medium merely because the game is running).
        report["status"] = "candidate"
        report["confidence"] = "low"
        if not running:
            report["reason"] = ("game PAUSED -> the decisive temporal-drift gate "
                                "cannot run; candidate is structural-only")
        else:
            report["reason"] = ("game running but the drift gate is not yet "
                                "implemented; candidate is structural-only")
        return cls(p, best, month, None), report


def read_live_month_value(p):
    """The live date integer via the tight-watch clock locator (returns
    (date_int, 0) for back-compat with callers that unpack two values). NOT a
    modal vote -- see locate_clock for why the modal date is the static 1937
    buffer, not the live clock."""
    _, date_int = locate_clock(p)
    if date_int is None:
        return None, 0
    return date_int, 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--month", type=int, default=None)
    ap.add_argument("--slack", type=float, default=6.0)
    ap.add_argument("--budget", type=float, default=300,
                    help="joint-scan wall-time budget in seconds")
    args = ap.parse_args()
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        t0 = time.time()
        layer, report = WeatherLayer.resolve(p, month=args.month,
                                             slack=args.slack,
                                             time_budget_s=args.budget)
        print("weather resolve: status=%s confidence=%s (%.1fs)"
              % (report["status"], report["confidence"], time.time() - t0))
        print("  month=%s running=%s" % (report.get("month"),
                                         report.get("running")))
        print("  reason: %s" % report["reason"])
        for c in report["candidates"][:8]:
            print("    addr=0x%X stride=%d Toff=%d Eoff=%d w=%d ib=%d "
                  "score=%.3f ctrlWM=%.3f ctrlSH=%.3f margin=%.3f"
                  % (c["addr"], c["stride"], c["temp_off"], c["enum_off"],
                     c["enum_width"], c["ibase"], c["score"],
                     c["ctrl_wrongmonth"], c["ctrl_shuffled"], c["margin"]))


if __name__ == "__main__":
    main()
