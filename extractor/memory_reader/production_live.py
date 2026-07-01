"""production_live.py - READ-ONLY live PRODUCTION reader for Battle Plan.

Reads every country's equipment STOCKPILE and military/naval PRODUCTION LINES
LIVE from a running PAUSED hoi4.exe via ReadProcessMemory only (READ-ONLY; no
writes / no injection / no thread suspension, so it never desyncs MP).

Both hang off a per-country PRODUCTION MANAGER reached from the country object:

    running_index  -> rec0 + index*0x120                      (locate.py / bridge)
    country object -> [rec + 0x08]                             (carries stability, etc.)
    prod manager   -> [country_object + 0xF68]

PINNED, validated live==save on build 16022851 against the lockstep melt
(re_scratch/melt_sov_1946.txt, save_date 1946.5.19.9):

  STOCKPILE BANK (military equipment, NOT convoys -- convoys are the separate
  cached scalar at country_obj+0x1280). A compact (non-STL) vector in the manager:
      manager+0x220  u64  data_begin
      manager+0x228  u32  capacity
      manager+0x22C  u32  size (entry count)
      entry stride 0x10:  { equip_def_ptr @ +0x00 (u64), amount @ +0x08 (i64 /100000, SIGNED) }
      equip_def_ptr -> a type-70 equipment-VARIANT object (vtable MODULE+0x292B218,
                       u32[+0x08]==70); the SAVE variant id == u32[def+0x0C].
      => stockpile[ u32[def+0x0C] ] = i64[entry+0x08] / 100000
      Validation: 459/461 common equipment ids EXACT (fixed-point) across 23
      countries; 22/23 countries 100% exact (the one imperfect = South Africa, 2
      tiny ids reading live=0 vs save 2/8 -> sub-frame production drift, not a
      parse error). Per-country count & capacity match (SOV size=113/cap=141,
      USA 52/63, ENG 64/94, JAP 34/42, ...).

  PRODUCTION LINES. A compact vector of POINTERS (stride 0x08) in the manager:
      manager+0x58   u64  data_begin
      manager+0x60   u32  capacity
      manager+0x64   u32  size (count of mil+naval line slots)
      Each slot = u64 line*. The line object carries a type tag:
          line+0x08  u32  type:  56 = MILITARY line, 57 = NAVAL line
          line+0x0C  i32  line id (matches the save *_lines.id)
          line+0x18  i32  active_factories
          line+0x28  i32  cost      /100000
          line+0x30  i32  speed     /100000
          line+0x38  i32  produced  /100000
          line+0x48  i32  amount    (target run; -1 == unlimited)
      Validation: military-line COUNT and SUM(active_factories) EXACT for 7/7
      anchor countries (SOV 17 lines / 429 factories, ENG 17/274, USA 23/235,
      JAP 13/105, FRA 11/172, SWE 6/4, BUL 7/34); naval-line count EXACT too
      (SOV 6, ENG 11, USA 10, ...). Per-line id/active/cost/speed/produced match
      the save (e.g. SOV id=349: active=3 cost=105.0 speed=31.05 produced=34.4469).

OPEN TICKETS (documented, not faked -- see production_offsets.draft.json):
  * priority / requested_factories / equipment_variant_index per line: secondary
    fields not yet offset-pinned (active==requested for nearly all lines this
    frame, so no distinguishing variance; the variant id needs a line with a
    located variant pointer). The validated metrics (count, active, cost, speed,
    produced) cover the gameplay-relevant view.
  * CONSTRUCTION QUEUE: the game has no per-country build-queue list in the save; the
    country-level `country_reports.construction` block is a periodic STATS snapshot
    (civilian_factory/military_factory/dockyard totals), and the actual building
    progress is per-STATE (owned by the state domain, already cracked). Factory
    allocation totals are state-derived (not a live country scalar) -> deferred to
    the state/save layer, same call as economy_offsets fuel/factories.

HARD RULES honored:
  * READ-ONLY memory; keep the game PAUSED; passive reads => any cadence, no desync.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the build.
  * No personal paths; self-contained; reuses winmem.py + locate.py + intel_live's
    save-free country-object location (vtable scan) + the country_index_bridge.
"""

import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
import winmem   # noqa: E402
import locate   # noqa: E402
import intel_live as il   # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# --- pinned offsets (production_offsets.draft.json) ---
OBJ_PROD_MANAGER = 0xF68         # country_object + this -> production manager
VARIANT_VT_RVA = 0x292B218       # type-70 equipment-variant object vtable (designs domain)
VARIANT_TYPE = 70

# stockpile bank (compact vector inside the manager)
STOCK_BEGIN = 0x220
STOCK_CAP = 0x228                # u32
STOCK_SIZE = 0x22C               # u32
STOCK_STRIDE = 0x10
ENT_DEF = 0x00                   # u64 equip_def_ptr
ENT_AMOUNT = 0x08                # i64 fixed-point /100000 (signed)
DEF_TYPE = 0x08                  # u32 == 70
DEF_VARIANT_ID = 0x0C            # u32 == save variant id

# production-line vector (compact vector of pointers inside the manager)
LINE_BEGIN = 0x58
LINE_CAP = 0x60                  # u32
LINE_SIZE = 0x64                 # u32
LINE_STRIDE = 0x08               # one pointer per slot
LN_TYPE = 0x08                   # u32: 56=military, 57=naval
LN_ID = 0x0C                     # i32
LN_ACTIVE = 0x18                 # i32 active_factories
LN_COST = 0x28                   # i32 /100000
LN_SPEED = 0x30                  # i32 /100000
LN_PRODUCED = 0x38               # i32 /100000
LN_AMOUNT = 0x48                 # i32 (-1 == unlimited)
TYPE_MIL = 56
TYPE_NAVAL = 57

FP = 100000.0
HEAP_MIN = 0x10000000000
HEAP_MAX = 0x7FFFFFFFFFFF


def _heap(a):
    return HEAP_MIN <= a <= HEAP_MAX and a % 8 == 0


def read_stockpile(p, country_object, variant_vt, diag=None):
    """Return {variant_id: amount(float)} for one country's military equipment bank,
    or None if unreachable. Amount is i64 fixed-point /100000 (signed).

    If `diag` is a dict, it is filled with {size, cap, tombstones, bad_gated, raw}
    where `raw` is {variant_id: amount_i64} (raw fixed-point) for diagnostics. A
    'tombstone' is a null/out-of-range def-ptr within [0,size) (an empty slot the
    engine keeps, e.g. a zeroed equipment line); 'bad_gated' is a NON-null entry
    whose def-ptr fails the vtable/type-70 gate -> potential corruption, surfaced
    rather than silently dropped (Codex review)."""
    try:
        mgr = p.u64(country_object + OBJ_PROD_MANAGER)
    except OSError:
        return None
    if not _heap(mgr):
        return None
    try:
        begin = p.u64(mgr + STOCK_BEGIN)
        cap = p.u32(mgr + STOCK_CAP)
        size = p.u32(mgr + STOCK_SIZE)
    except OSError:
        return None
    if not _heap(begin) or size > cap or cap > 4096:
        return None
    raw = p.try_read(begin, size * STOCK_STRIDE)
    if len(raw) < size * STOCK_STRIDE:
        return None
    out = {}
    raw_fp = {}
    tombstones = bad_gated = 0
    for k in range(0, size * STOCK_STRIDE, STOCK_STRIDE):
        dptr = struct.unpack_from("<Q", raw, k + ENT_DEF)[0]
        amt = struct.unpack_from("<q", raw, k + ENT_AMOUNT)[0]   # SIGNED i64
        if not _heap(dptr):
            tombstones += 1
            continue                                             # empty/tombstone slot
        try:
            if p.u64(dptr) != variant_vt or p.u32(dptr + DEF_TYPE) != VARIANT_TYPE:
                bad_gated += 1                                   # NON-null but bad gate
                continue
            vid = p.u32(dptr + DEF_VARIANT_ID)
        except OSError:
            bad_gated += 1
            continue
        out[vid] = amt / FP
        raw_fp[vid] = amt
    if diag is not None:
        diag.update({"size": size, "cap": cap, "tombstones": tombstones,
                     "bad_gated": bad_gated, "raw": raw_fp})
    return out


def read_production_lines(p, country_object):
    """Return {'military': [line,...], 'naval': [line,...]} for one country, or None.
    Each line: {id, active_factories, cost, speed, produced, amount}."""
    try:
        mgr = p.u64(country_object + OBJ_PROD_MANAGER)
    except OSError:
        return None
    if not _heap(mgr):
        return None
    try:
        begin = p.u64(mgr + LINE_BEGIN)
        cap = p.u32(mgr + LINE_CAP)
        size = p.u32(mgr + LINE_SIZE)
    except OSError:
        return None
    if not _heap(begin) or size > cap or cap > 4096:
        return None
    raw = p.try_read(begin, size * LINE_STRIDE)
    if len(raw) < size * LINE_STRIDE:
        return None
    mil, nav = [], []
    for k in range(0, size * LINE_STRIDE, LINE_STRIDE):
        ptr = struct.unpack_from("<Q", raw, k)[0]
        if not _heap(ptr):
            continue
        body = p.try_read(ptr, 0x50)
        if len(body) < 0x50:
            continue
        tt = struct.unpack_from("<I", body, LN_TYPE)[0]
        if tt not in (TYPE_MIL, TYPE_NAVAL):
            continue
        line = {
            "id": struct.unpack_from("<i", body, LN_ID)[0],
            "active_factories": struct.unpack_from("<i", body, LN_ACTIVE)[0],
            "cost": struct.unpack_from("<i", body, LN_COST)[0] / FP,
            "speed": struct.unpack_from("<i", body, LN_SPEED)[0] / FP,
            "produced": struct.unpack_from("<i", body, LN_PRODUCED)[0] / FP,
            "amount": struct.unpack_from("<i", body, LN_AMOUNT)[0],
        }
        (mil if tt == TYPE_MIL else nav).append(line)
    return {"military": mil, "naval": nav}


def read_country(p, country_object, variant_vt):
    """Full per-country production snapshot."""
    stock = read_stockpile(p, country_object, variant_vt)
    lines = read_production_lines(p, country_object)
    if stock is None and lines is None:
        return None
    mil = lines["military"] if lines else []
    nav = lines["naval"] if lines else []
    return {
        "stockpile": stock or {},
        "stockpile_total": sum((stock or {}).values()),
        "military_lines": mil,
        "naval_lines": nav,
        "military_factories": sum(l["active_factories"] for l in mil),
        "naval_factories": sum(l["active_factories"] for l in nav),
    }


def resolve_country_objects(p, bridge=None):
    """{key: country_object}. PRIMARY: country-index bridge (TAG keys). FALLBACK
    (bridge's array-locate fails this session): vtable scan, keyed by localized name.

    ``bridge`` (optional): an already-resolved CountryIndexBridge to REUSE, skipping the
    per-call country-array re-resolve heap scan (steady-state perf). None preserves the
    original self-resolving behaviour."""
    loc = il.resolve_country_objects(p, bridge=bridge)
    if loc is None:
        return None
    if "bridge" in loc:
        br = loc["bridge"]
        out = {}
        for tag_db_id in br.as_map():
            obj = br.country_object(tag_db_id)
            if obj:
                out[br.tag(tag_db_id)] = obj
        return out
    return loc["objs"]   # {name: obj}


def read_all(p, bridge=None):
    """{TAG_or_name: production_snapshot} for every active country.

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge to skip the
    per-call country-array re-resolve scan (steady-state perf)."""
    objs = resolve_country_objects(p, bridge=bridge)
    if objs is None:
        raise RuntimeError("could not locate the country objects (bridge + vtable "
                           "fallback both failed)")
    variant_vt = p.module_base + VARIANT_VT_RVA
    out = {}
    for key, obj in objs.items():
        snap = read_country(p, obj, variant_vt)
        if snap is not None:
            out[key] = snap
    return out


def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


def _validate():
    """Validate live==save against the lockstep melt ground truth (_prod_gt.json,
    produced by _prod_gt.py from re_scratch/melt_sov_1946.txt) + the verified
    TAG->object anchor map (_prod_anchors.json). Prints per-country and grand-total
    exact-match rates for stockpile (fixed-point) and production-line metrics, plus
    a shuffled-label negative control."""
    import json
    import random

    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        gt = json.load(open(os.path.join(_HERE, "_prod_gt.json")))
        anchors = json.load(open(os.path.join(_HERE, "_prod_anchors.json")))["tag_obj"]
        variant_vt = p.module_base + VARIANT_VT_RVA
        # "Regno del Sud" (live) is a different entity than the save's main ITA block.
        exclude = {"ITA"}

        st_exact = st_total = 0
        ln_count_ok = ln_active_ok = ln_total = 0
        nav_count_ok = 0
        per_country_perfect = 0
        validated = 0
        decoded_stock = {}   # tag -> {id: amount} for the shuffle control
        for tag in sorted(anchors):
            if tag not in gt or tag in exclude:
                continue
            obj = int(anchors[tag], 16)
            snap = read_country(p, obj, variant_vt)
            if snap is None:
                continue
            validated += 1
            save_stock = {int(k): v for k, v in gt[tag]["stock"].items()}
            live_stock = snap["stockpile"]
            decoded_stock[tag] = live_stock
            common = set(live_stock) & set(save_stock)
            exact = sum(1 for k in common
                        if round(live_stock[k] * FP) == round(save_stock[k] * FP))
            st_exact += exact
            st_total += len(common)
            # production lines
            ln_total += 1
            if len(snap["military_lines"]) == gt[tag]["n_mil_lines"]:
                ln_count_ok += 1
            if snap["military_factories"] == gt[tag]["sum_active_mil"]:
                ln_active_ok += 1
            if len(snap["naval_lines"]) == gt[tag]["n_nav_lines"]:
                nav_count_ok += 1
            perfect = (exact == len(common)
                       and len(snap["military_lines"]) == gt[tag]["n_mil_lines"]
                       and snap["military_factories"] == gt[tag]["sum_active_mil"])
            if perfect:
                per_country_perfect += 1
            print("  %-4s stock %d/%d exact | mil_lines %d/%d | mil_factories %d/%d | nav_lines %d/%d"
                  % (tag, exact, len(common),
                     len(snap["military_lines"]), gt[tag]["n_mil_lines"],
                     snap["military_factories"], gt[tag]["sum_active_mil"],
                     len(snap["naval_lines"]), gt[tag]["n_nav_lines"]))

        print("\n=== STOCKPILE: %d/%d common ids exact fixed-point (%.2f%%) over %d countries"
              % (st_exact, st_total, 100.0 * st_exact / max(st_total, 1), validated))
        print("=== PRODUCTION LINES: mil-count %d/%d, sum(active) %d/%d, naval-count %d/%d"
              % (ln_count_ok, ln_total, ln_active_ok, ln_total, nav_count_ok, ln_total))
        print("=== %d/%d countries 100%% exact (stockpile + mil lines)"
              % (per_country_perfect, validated))

        # shuffled-label negative control: assign each country's live stock to a
        # RANDOM other country's save stock; the exact-match rate should collapse.
        tags = list(decoded_stock)
        shuffled = tags[:]
        random.seed(1)
        random.shuffle(shuffled)
        sh_exact = sh_total = 0
        for tag, other in zip(tags, shuffled):
            save_stock = {int(k): v for k, v in gt[other]["stock"].items()}
            live_stock = decoded_stock[tag]
            common = set(live_stock) & set(save_stock)
            sh_exact += sum(1 for k in common
                            if round(live_stock[k] * FP) == round(save_stock[k] * FP))
            sh_total += len(common)
        print("=== SHUFFLE control: %d/%d (%.2f%%) -- should be near 0 if the pin is real"
              % (sh_exact, sh_total, 100.0 * sh_exact / max(sh_total, 1)))


def main():
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        print("build gate OK; reading production (READ-ONLY)...")
        data = read_all(p)
        print("countries with production data: %d" % len(data))
        print("\n%-6s %8s %6s %6s %6s %6s"
              % ("KEY", "stockTot", "stockN", "milL", "milFac", "navL"))
        rows = sorted(data.items(), key=lambda kv: -kv[1]["stockpile_total"])
        for key, d in rows[:30]:
            print("%-6s %8d %6d %6d %6d %6d"
                  % (key[:6], round(d["stockpile_total"]), len(d["stockpile"]),
                     len(d["military_lines"]), d["military_factories"],
                     len(d["naval_lines"])))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "validate":
        _validate()
    else:
        main()
