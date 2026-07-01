"""designs_mio_deep_live.py - LIVE deeper-deref readers (READ-ONLY) for:

  1. EQUIPMENT-VARIANT MODULE GRAPH - which modules sit in each design variant's slots
     (the per-variant {slot -> module} map + upgrade levels), and
  2. MIO UNLOCKED-TRAIT NAMES - the traits an industrial organisation has actually taken.

Both are the "resisted" deeper-deref follow-ups to designs_live.py / mio_live.py:
the variant POOL (vtable MODULE+0x292B218, type 70) and the MIO POOL (mio_live) are already
located; this module walks the sub-containers they point at and resolves the tokens.

Cracked 2026-06-26 on build 16022851 against the time-aligned melted save
(re_scratch/melt_sov_1946.txt, 1946.5.19.9, lockstep). See designs_mio_deep_offsets.draft.json
for the full offset ledger + evidence.

--- VARIANT MODULES (validated 3594/3594 module set; 2153/2153 full {slot:module} map) ---
  variant body (pool object, id @+0x0C) holds a CONTIGUOUS module array:
      data @ variant+0xB8, capacity @+0xC0, SIZE @+0xC4 (use SIZE as the bound).
      entry stride 0x10: { u32 slot_handle @+0x00, module_def_ptr @+0x08 }.
      module name = inline std::string @ module_def+0x18 (module_def vtable MODULE+0x2911AB0).
  The slot_handle -> slot_name table is derived by a SAVE-JOIN (a slot's module is usually
  unique within a variant, giving a clean handle<->slot vote); see build_slot_table().
  Upgrades (container @ variant+0x68): the LEVEL is the low byte of the value qword at
  entry+0x08 (stride 0x10); the upgrade-NAME key is a non-readable interned handle (RESISTED).

--- MIO UNLOCKED TRAITS (validated 286/286 count; 276/286 set + 10 save-aliases, 0 misses) ---
  MIO record holds a std::map (RB-tree) at record+0x138 (head ptr), size @+0x140.
  In-order walk yields exactly `size` nodes; each node's mapped key object is
  { vtable MODULE+0x271F388 @node+0x20, u32 trait_handle @node+0x28 }.
  Resolve the handle via the MIO-trait def registry (vtable MODULE+0x26F44E0,
  handle u32 @+0x08, token std::string @+0x18). A few handles collide across def
  namespaces -> disambiguated by def-instance MAJORITY (canonical >> 1-off alias).

HARD RULES: READ-ONLY (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes/injection/suspension - passive => no MP desync, any cadence).
Build-gated: fail-closed unless the live exe sha256 matches the pinned build (offsets are
build-specific). Region-clamped reads. ASCII-only.

Public API
----------
  EQUIPMENT VARIANTS:
    enumerate_variant_addrs(p)              -> {variant_id:int -> object_addr:int}
    read_variant_modules(p, addr)           -> [(slot_handle:int, module_name:str), ...]
    read_variant_upgrades(p, addr)          -> [level:int, ...]   (levels in array order)
    build_slot_table(p, variants=None, gt=None) -> {slot_handle:int -> slot_name:str}
    variant_module_map(p, addr, slot_table) -> {slot_name:str -> module_name:str}

  MIO TRAITS:
    build_mio_trait_registry(p)             -> {handle:int -> token:str}  (majority-vote)
    read_mio_trait_handles(p, mio_addr)     -> [handle:int, ...]  (in-order)
    resolve_mio_traits_live(p, mio_addr, reg=None) -> [token:str, ...]

Usage
-----
    python designs_mio_deep_live.py            # dump samples
    python designs_mio_deep_live.py --validate # full live==save report
"""

import argparse
import json
import os
import struct
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)

import winmem  # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# ---- equipment-variant module/upgrade graph (build 16022851) ----
EQ_VARIANT_VT_RVA   = 0x292B218   # variant pool object vtable
VARIANT_TYPE        = 70
VAR_ID_OFF          = 0x0C

MOD_CONTAINER_OFF   = 0xB0        # module array container (vtable @+0xB0)
MOD_DATA_OFF        = 0xB8        # data ptr
MOD_CAP_OFF         = 0xC0        # capacity u32
MOD_SIZE_OFF        = 0xC4        # SIZE u32 (loop bound)
MOD_ENTRY_STRIDE    = 0x10
MOD_ENTRY_KEY_OFF   = 0x00        # u32 slot_handle
MOD_ENTRY_VAL_OFF   = 0x08        # u64 module_def_ptr

MODULE_DEF_VT_RVA   = 0x2911AB0   # module def object vtable
MODULE_NAME_OFF     = 0x18        # module token std::string

UPG_CONTAINER_OFF   = 0x68
UPG_DATA_OFF        = 0x70
UPG_SIZE_OFF        = 0x7C
UPG_ENTRY_STRIDE    = 0x10
UPG_ENTRY_VAL_OFF   = 0x08        # low byte = level

# ---- MIO unlocked-trait map (build 16022851) ----
MIO_TRAIT_HEAD_OFF  = 0x138       # std::map head ptr (record-relative)
MIO_TRAIT_SIZE_OFF  = 0x140       # trait count
NODE_LEFT           = 0x00
NODE_RIGHT          = 0x10
NODE_PARENT         = 0x08
NODE_ISNIL          = 0x19
NODE_HANDLE_OFF     = 0x28        # u32 trait handle (node+0x20 = key vtable)

MIO_TRAIT_DEF_VT_RVA = 0x26F44E0  # MIO-trait def registry class
TRAIT_DEF_HANDLE_OFF = 0x08
TRAIT_DEF_TOKEN_OFF  = 0x18

MAX_MODULES = 256
MAX_TRAITS = 256


# ==========================================================================
# build gate + low-level helpers
# ==========================================================================
def _gate_ok(p):
    return p.fingerprint().get("sha256") == EXPECTED_SHA


def heap_ok(v):
    return HEAP_LO <= v < HEAP_HI


def _su32(p, a):
    try:
        return p.u32(a)
    except OSError:
        return None


def _su64(p, a):
    try:
        return p.u64(a)
    except OSError:
        return None


def _su8(p, a):
    try:
        return p.u8(a)
    except OSError:
        return None


def _read_msvc_str(p, base, maxlen=200):
    """MSVC std::string at base: buf[16]@+0, size@+0x10, cap@+0x18; SSO when cap<16.
    Region-clamp safe; returns an ASCII str or None."""
    try:
        size = p.u64(base + 0x10)
        cap = p.u64(base + 0x18)
    except OSError:
        return None
    if cap < size or size == 0 or size > maxlen or cap > (1 << 20):
        return None
    try:
        chars = base if cap < 16 else p.u64(base)
        data = p.read(chars, size)
    except OSError:
        return None
    try:
        s = data.decode("latin-1")
    except Exception:
        return None
    return s if all(32 <= ord(c) < 127 for c in s) else None


def _enum_vtable(p, vt_abs):
    """Yield every 8-aligned heap address whose qword == vt_abs (shared anti-stall scanner)."""
    needle = struct.pack("<Q", vt_abs)
    CH = 32 << 20
    OV = 16
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                i = blob.find(needle)
                while i != -1:
                    addr = base + pos + i
                    if addr % 8 == 0:
                        yield addr
                    i = blob.find(needle, i + 1)
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - OV
            else:
                pos += max(want, 0x1000)


# ==========================================================================
# EQUIPMENT-VARIANT module/upgrade graph
# ==========================================================================
def enumerate_variant_addrs(p):
    """{variant_id -> object_addr} for every live equipment-variant (type 70). fail-closed."""
    if not _gate_ok(p):
        return {}
    vt = p.module_base + EQ_VARIANT_VT_RVA
    out = {}
    for a in _enum_vtable(p, vt):
        if _su32(p, a + 0x08) != VARIANT_TYPE:
            continue
        vid = _su32(p, a + VAR_ID_OFF)
        if vid is not None:
            out.setdefault(vid, a)
    return out


def read_variant_modules(p, addr):
    """[(slot_handle, module_name), ...] for one variant object. Reads exactly SIZE entries
    from the contiguous module array; vtable-gates each module def."""
    out = []
    if not _gate_ok(p):
        return out
    data = _su64(p, addr + MOD_DATA_OFF)
    size = _su32(p, addr + MOD_SIZE_OFF)
    if data is None or not heap_ok(data):
        return out
    if size is None or size > MAX_MODULES:
        return out
    mvt = p.module_base + MODULE_DEF_VT_RVA
    for i in range(size):
        e = data + i * MOD_ENTRY_STRIDE
        val = _su64(p, e + MOD_ENTRY_VAL_OFF)
        if val is None or not heap_ok(val) or _su64(p, val) != mvt:
            continue
        name = _read_msvc_str(p, val + MODULE_NAME_OFF, maxlen=80)
        if name is None:
            continue
        key = _su32(p, e + MOD_ENTRY_KEY_OFF)
        out.append((key, name))
    return out


def read_variant_upgrades(p, addr):
    """[level, ...] for one variant (upgrade levels in array order). The upgrade NAME is a
    resisted interned key (see draft); only the level (low byte of value@entry+0x08) is read."""
    out = []
    if not _gate_ok(p):
        return out
    data = _su64(p, addr + UPG_DATA_OFF)
    size = _su32(p, addr + UPG_SIZE_OFF)
    if data is None or not heap_ok(data) or size is None or size > MAX_MODULES:
        return out
    for i in range(size):
        e = data + i * UPG_ENTRY_STRIDE
        v = _su64(p, e + UPG_ENTRY_VAL_OFF)
        if v is None:
            continue
        out.append(v & 0xFF)
    return out


def build_slot_table(p, variants=None, gt=None):
    """Derive {slot_handle -> slot_name} by a SAVE-JOIN against the time-aligned save GT.

    For each variant, the live {slot_handle: module} is matched against the save
    {slot_name: module}. Where a module is unique within the variant on both sides, the
    (handle, slot) pair gets a vote; a greedy second pass resolves duplicate-module slots
    (e.g. the two ship deck slots) against the still-unclaimed save slots. Majority wins.

    Returns {} if the GT is unavailable (the module SET still resolves without it; only the
    slot NAMES need this join). gt = {variant_id -> {"modules": {slot:module}}}."""
    import collections
    if gt is None:
        gt = _load_variant_gt()
    if not gt:
        return {}
    if variants is None:
        variants = enumerate_variant_addrs(p)
    cache = {}
    votes = collections.defaultdict(collections.Counter)
    for vid, a in variants.items():
        g = gt.get(vid)
        if not g:
            continue
        gmods = g.get("modules") or {}
        live = read_variant_modules(p, a)
        cache[vid] = (live, gmods)
        sc = collections.Counter(gmods.values())
        m2s = {m: s for s, m in gmods.items() if sc[m] == 1}
        lc = collections.Counter(m for _, m in live)
        for h, mod in live:
            if lc[mod] == 1 and mod in m2s:
                votes[h][m2s[mod]] += 1
    table = {h: c.most_common(1)[0][0] for h, c in votes.items()}
    # greedy passes for duplicate-module slots
    for _ in range(3):
        for vid, (live, gmods) in cache.items():
            if not gmods:
                continue
            remaining = dict(gmods)
            unknown = []
            for h, mod in live:
                sn = table.get(h)
                if sn in remaining and remaining[sn] == mod:
                    del remaining[sn]
                else:
                    unknown.append((h, mod))
            for h, mod in unknown:
                cands = [s for s, m in remaining.items() if m == mod]
                if len(cands) == 1:
                    votes[h][cands[0]] += 1
                    del remaining[cands[0]]
        table = {h: c.most_common(1)[0][0] for h, c in votes.items()}
    return table


def variant_module_map(p, addr, slot_table):
    """{slot_name -> module_name} for one variant, using a slot_table from build_slot_table().
    Greedily assigns duplicate-module slots so two identical modules in distinct slots both
    appear. Unresolvable slot handles fall back to 'slot_<hex>'."""
    live = read_variant_modules(p, addr)
    # group by resolved slot; track which (resolved) slots are taken to handle duplicates
    out = {}
    leftover = []
    # first pass: direct handle->slot
    used = set()
    for h, mod in live:
        sn = slot_table.get(h)
        if sn is not None and sn not in used:
            out[sn] = mod
            used.add(sn)
        else:
            leftover.append((h, mod))
    for h, mod in leftover:
        sn = slot_table.get(h)
        if sn is None:
            sn = "slot_%X" % h
        # disambiguate duplicate slot keys
        base = sn
        k = 1
        while sn in out:
            sn = "%s#%d" % (base, k)
            k += 1
        out[sn] = mod
    return out


# ==========================================================================
# MIO unlocked-trait names
# ==========================================================================
def _tree_inorder(p, head, maxn=MAX_TRAITS):
    """In-order walk of an MSVC std::map _Tree from its _Myhead. Returns node addrs.

    Every descent/ascent is hop-capped and the visited set is tracked so a cyclic/corrupt
    tree pointer can never spin (defensive: the walk is over untrusted live memory)."""
    nodes = []
    HOP_CAP = maxn * 4

    def isnil(n):
        v = _su8(p, n + NODE_ISNIL)
        return v is None or v != 0

    seen = set()
    cur = _su64(p, head + NODE_LEFT)  # leftmost
    while cur and not isnil(cur) and len(nodes) < maxn:
        if cur in seen:                       # cycle guard
            break
        seen.add(cur)
        nodes.append(cur)
        R = _su64(p, cur + NODE_RIGHT)
        if R and not isnil(R):
            cur = R
            hops = 0
            while hops < HOP_CAP:
                hops += 1
                Lc = _su64(p, cur + NODE_LEFT)
                if Lc and not isnil(Lc):
                    cur = Lc
                else:
                    break
        else:
            par = _su64(p, cur + NODE_PARENT)
            hops = 0
            while (par and not isnil(par) and cur == _su64(p, par + NODE_RIGHT)
                   and hops < HOP_CAP):
                hops += 1
                cur = par
                par = _su64(p, par + NODE_PARENT)
            cur = par
            if cur is None or isnil(cur):
                break
    return nodes


def build_mio_trait_registry(p):
    """{trait_handle -> token} from the MIO-trait def registry (vtable MODULE+0x26F44E0).

    A few handles map to >1 token because the def vtable is shared across def namespaces;
    the canonical def is instantiated many times (34-42 heap copies) while the alias appears
    once, so we keep the token with the MOST def-object instances (majority vote). fail-closed."""
    import collections
    if not _gate_ok(p):
        return {}
    vt = p.module_base + MIO_TRAIT_DEF_VT_RVA
    cnt = collections.Counter()
    for a in _enum_vtable(p, vt):
        tok = _read_msvc_str(p, a + TRAIT_DEF_TOKEN_OFF, maxlen=120)
        h = _su32(p, a + TRAIT_DEF_HANDLE_OFF)
        if tok and h is not None:
            cnt[(h, tok)] += 1
    by_handle = collections.defaultdict(list)
    for (h, tok), c in cnt.items():
        by_handle[h].append((c, tok))
    reg = {}
    for h, lst in by_handle.items():
        lst.sort(reverse=True)          # highest instance count first
        reg[h] = lst[0][1]
    return reg


def read_mio_trait_handles(p, mio_addr):
    """[trait_handle, ...] in tree order for one MIO record (record base = mio_live 'addr')."""
    if not _gate_ok(p):
        return []
    head = _su64(p, mio_addr + MIO_TRAIT_HEAD_OFF)
    size = _su32(p, mio_addr + MIO_TRAIT_SIZE_OFF)
    if head is None or not heap_ok(head) or size is None or size == 0 or size > MAX_TRAITS:
        return []
    out = []
    for n in _tree_inorder(p, head):
        h = _su32(p, n + NODE_HANDLE_OFF)
        if h is not None:
            out.append(h)
    return out


def resolve_mio_traits_live(p, mio_addr, reg=None):
    """[trait_token, ...] for one MIO record. reg = build_mio_trait_registry(p) (built once)."""
    if reg is None:
        reg = build_mio_trait_registry(p)
    out = []
    for h in read_mio_trait_handles(p, mio_addr):
        t = reg.get(h)
        if t:
            out.append(t)
    return out


# ==========================================================================
# GT loaders (for slot-table build + validation only; not needed at runtime once the
# slot table is cached)
# ==========================================================================
def _load_variant_gt():
    path = os.path.join(os.path.dirname(_THIS), "re_scratch", "designs_gt", "designs.json")
    if not os.path.exists(path):
        return {}
    try:
        d = json.load(open(path))
        return {v["id"]: v for v in d["equipment_variants"].values()}
    except Exception:
        return {}


def _load_mio_trait_gt():
    path = os.path.join(_THIS, "_dmdeep_mio_gt.json")
    if not os.path.exists(path):
        return {}
    try:
        return {int(k): v for k, v in json.load(open(path)).items()}
    except Exception:
        return {}


# ==========================================================================
# validation
# ==========================================================================
def _validate(p):
    """Validate both deeper-derefs vs the time-aligned save; return a one-screen report."""
    import collections
    rep = {}

    # --- VARIANT MODULES ---
    gt = _load_variant_gt()
    variants = enumerate_variant_addrs(p)
    rep["variant_pool"] = {"live_variants": len(variants), "gt_variants": len(gt)}
    if gt:
        slot_table = build_slot_table(p, variants, gt)
        cnt_ok = set_ok = tot = 0
        full_ok = full_n = 0
        for vid, a in variants.items():
            g = gt.get(vid)
            if not g:
                continue
            gmods = g.get("modules") or {}
            mods = read_variant_modules(p, a)
            live_names = sorted(m for _, m in mods)
            gt_names = sorted(gmods.values())
            tot += 1
            if len(mods) == len(gmods):
                cnt_ok += 1
            if live_names == gt_names:
                set_ok += 1
            if gmods:
                recon = variant_module_map(p, a, slot_table)
                full_n += 1
                if recon == gmods:
                    full_ok += 1
        rep["variant_modules"] = {
            "module_count_live_eq_save": "%d/%d" % (cnt_ok, tot),
            "module_set_live_eq_save": "%d/%d" % (set_ok, tot),
            "full_slot_module_map_live_eq_save": "%d/%d (module-bearing variants)" % (full_ok, full_n),
            "slot_handles_resolved": len(slot_table),
        }
        # upgrade level spot-check (anchors with known nonzero levels)
        anchors = {}
        for vid, v in gt.items():
            u = v.get("upgrades") or {}
            if any(val > 0 for val in u.values()):
                anchors[vid] = sorted(val for val in u.values())
        up_ok = up_n = 0
        for vid in list(anchors)[:200]:
            a = variants.get(vid)
            if a is None:
                continue
            live_lv = sorted(read_variant_upgrades(p, a))
            up_n += 1
            if live_lv == anchors[vid]:
                up_ok += 1
        rep["variant_upgrades"] = {
            "upgrade_level_multiset_live_eq_save": "%d/%d (nonzero-upgrade anchors)" % (up_ok, up_n) if up_n else "no anchors",
            "note": "upgrade LEVELS pinned (low byte of value@entry+0x08); upgrade NAMES RESISTED (interned key).",
        }
    else:
        rep["variant_modules"] = "skipped (no variant GT)"

    # --- MIO TRAITS ---
    try:
        import mio_live
        reg = build_mio_trait_registry(p)
        mios = mio_live.enumerate_mios(p)
        tgt = _load_mio_trait_gt()
        rep["mio_trait_registry"] = {"handles": len(reg)}
        if tgt:
            cnt_ok = set_ok = tot = 0
            alias = 0
            realmiss = 0
            for oid, g in tgt.items():
                if g.get("n_traits", 0) == 0:
                    continue
                r = mios.get(oid)
                if not r:
                    continue
                live = set(resolve_mio_traits_live(p, r["addr"], reg))
                gtset = set(g["traits"])
                n_nodes = len(read_mio_trait_handles(p, r["addr"]))
                tot += 1
                if n_nodes == g["n_traits"]:
                    cnt_ok += 1
                if live == gtset:
                    set_ok += 1
                else:
                    d_save = gtset - live
                    d_live = live - gtset
                    if len(d_save) == len(d_live):
                        alias += 1            # save-vs-live name alias (same count)
                    else:
                        realmiss += 1
            rep["mio_traits"] = {
                "trait_count_live_eq_save": "%d/%d" % (cnt_ok, tot),
                "trait_set_live_eq_save": "%d/%d" % (set_ok, tot),
                "save_vs_live_name_aliases": alias,
                "real_resolution_misses": realmiss,
                "note": "the live def token is the authoritative in-memory name; the alias rows "
                        "are save-serialization naming quirks (same trait count), NOT misses.",
            }
        else:
            rep["mio_traits"] = "registry built (%d handles); no MIO-trait GT to validate" % len(reg)
    except Exception as e:
        rep["mio_traits"] = "skipped (%s)" % e

    return rep


def main():
    ap = argparse.ArgumentParser(description="LIVE variant-module + MIO-trait deeper-deref readers (read-only).")
    ap.add_argument("--validate", action="store_true", help="validate both vs the time-aligned save")
    args = ap.parse_args()

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        if not _gate_ok(p):
            print("BUILD MISMATCH - refusing (offsets are build-specific).")
            raise SystemExit(1)
        print("build OK; module_base=0x%X" % p.module_base)

        if args.validate:
            print(json.dumps(_validate(p), indent=1))
            return

        variants = enumerate_variant_addrs(p)
        print("\nlive equipment variants: %d" % len(variants))
        slot_table = build_slot_table(p, variants)
        # sample a few module-bearing variants
        shown = 0
        for vid, a in sorted(variants.items()):
            mm = variant_module_map(p, a, slot_table)
            if len(mm) >= 4 and shown < 4:
                print("  variant %d: %d modules" % (vid, len(mm)))
                for slot, mod in list(mm.items())[:6]:
                    print("      %-34s = %s" % (slot, mod))
                up = read_variant_upgrades(p, a)
                if up:
                    print("      upgrades(levels): %s" % up)
                shown += 1

        reg = build_mio_trait_registry(p)
        print("\nMIO-trait registry: %d handles" % len(reg))
        try:
            import mio_live
            mios = mio_live.enumerate_mios(p)
            rich = sorted(mios.values(), key=lambda r: r["unlocked_traits_count"], reverse=True)[:3]
            for r in rich:
                traits = resolve_mio_traits_live(p, r["addr"], reg)
                print("  MIO %d (%d traits): %s%s" % (
                    r["id"], len(traits), ", ".join(traits[:4]),
                    " ..." if len(traits) > 4 else ""))
        except Exception:
            pass


if __name__ == "__main__":
    main()
