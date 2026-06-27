"""variables_live.py - LIVE per-country script VARIABLES reader (read-only).

Reads, straight from a running PAUSED hoi4.exe (no save needed at runtime), each
country's full script-variable bag {name: value} pinned on build 16022851.

CONTAINER (cracked + validated 2026-06-27, 18/18 USA + 2853/2855 across 35 countries):
  country_obj + 0x218  ->  CVariables submanager (RTTI .?AVCVariables@@)
  CVariables is a Paradox open-addressing hash table:
      +0x18  u64  node-array base pointer
      +0x20  u32  size (live element count); high u32 = bucket mask (nbuckets-1)
  Each occupied bucket is a 0x30-byte node:
      +0x00  MSVC std::string  variable NAME  (SSO when cap<16, else heap ptr@+0)
             (size @ name+0x10, capacity @ name+0x18)
      +0x20  int64             VALUE, FIXED-POINT scaled by 1e5
             (e.g. 11.00673 -> 1100673 ; -10737.41542 -> -1073741542 ; 96 -> 9600000)
  The bucket array begins after a per-bucket control-byte region, so nodes do NOT
  start at base; we auto-detect the node grid phase (0..0x30 step 8) and walk every
  0x30 slot across [base, base + (nbuckets+4)*0x30), parsing occupied buckets only.
  Array variables appear as literal names "name^0","name^1",... (the ^N is part of
  the key string); they enumerate automatically with the same {name,value} shape.

  NOTE on value scale: the game script variables are stored as fixed-point x1e5. We expose
  the decoded float (raw/1e5). State-id array variables (e.g. core_states_at_game_start^N)
  carry the engine's fixed-point state encoding (~ -10737.4xxxx); they are passed
  through as their decoded float (the caller can re-interpret if needed) - NOT fabricated.

COUNTRY FLAGS are a SEPARATE manager (country_obj + 0x230, RTTI .?AVCFlagManager@@) and
are NOT decoded by this reader - see open_tickets in the module report. Flags are keyed
by an integer token (the flag-name intern id, e.g. 0xB5E5 for USA_congress_investment)
mapping to {value, date}; the runtime date encoding did not match the standard
hours-since-year-0 formula in the crack window, so flags are left UNRESOLVED rather than
emitting a guessed value/date.

BUILD-GATED: refuses to run unless the live exe SHA-256 matches the mapped build, unless
--force is given (same fail-closed gate as the sibling live readers).

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes, no injection, game untouched, MP-safe. A stable-header
re-read guards against a torn read of the table header mid-tick.

Public API
----------
    read_country_variables(p, bridge, rec0, count) -> { TAG: {name: float, ...} }
  `bridge` = {running_index: TAG}; `rec0`/`count` = the live country array base+count.
  Each country's CVariables is RTTI-gated (fail-closed: a wrong/garbage submanager
  pointer yields {} for that country, never a fabricated entry).

Usage
-----
    python variables_live.py --validate   # locate, enumerate, compare to the oracle
"""

import argparse
import json
import os
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))

import winmem
import locate as L

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# country array
REC = 0x120
OFF_NAMEPTR = 0x08
OFF_INDEX = 0x5D

# country-object -> CVariables submanager
OBJ_VARIABLES = 0x218
RTTI_VARIABLES = ".?AVCVariables@@"

# CVariables header
CV_BASE = 0x18           # u64 node-array base
CV_SIZE_MASK = 0x20      # u32 size (lo), u32 bucket mask (hi)

# node layout
NODE = 0x30              # bucket stride
NODE_VALUE = 0x20        # int64 value @ node+0x20
VALUE_SCALE = 1e5

# sanity bounds
MAX_SIZE = 200000
MAX_BUCKETS = 1 << 21
MAX_KEYLEN = 256


def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA:
        print("  build gate: exe SHA-256 matches the mapped build (%s...)" % got[:12])
        return
    msg = ("  build gate: live exe SHA-256 %s != mapped build %s; offsets are "
           "build-specific, reading blind risks SILENT MISREADS." % (got, EXPECTED_SHA))
    if force:
        print(msg + "\n  --force given: continuing (results UNVERIFIED).")
    else:
        raise SystemExit(msg + "\n  Refusing (pass --force to override).")


def heap_ok(v):
    return HEAP_LO <= v < HEAP_HI


def _rtti_name(p, obj):
    """RTTI type name of a heap object, or None (fail-closed gate)."""
    try:
        vt = p.u64(obj)
    except OSError:
        return None
    mb = p.module_base
    if not (mb <= vt < mb + 0x10000000):
        return None
    try:
        col = p.u64(vt - 8)
        if not (mb <= col < mb + 0x10000000):
            return None
        td = mb + p.u32(col + 0x0C)
        nm = p.cstr(td + 0x10, 200)
    except OSError:
        return None
    return nm if nm and nm.startswith(".?A") else None


def _read_key(p, so):
    """MSVC std::string variable name at `so`. cap decides SSO (cap<16 inline). The
    NUL terminator at +size is required and every char must be printable -> a wrong
    node fails closed (None) rather than yielding a garbage name."""
    try:
        size = p.u64(so + 0x10)
        cap = p.u64(so + 0x18)
    except OSError:
        return None
    if not (1 <= size <= MAX_KEYLEN and size <= cap and cap < (1 << 20)):
        return None
    # MSVC std::string: SSO iff cap == 15 (the 16-byte buffer; cap is fixed at 15 for any
    # inline string). Heap strings have cap >= size (>= 16 in practice). cap in 0..14 or
    # 16..(size-1) would be an invalid/torn header -> reject (already gated size<=cap).
    inline = (cap == 15)
    if not inline and cap < 16:
        return None
    try:
        chars = so if inline else p.u64(so)
        data = p.read(chars, size + 1)
    except OSError:
        return None
    if data[size] != 0:
        return None
    s = data[:size].decode("latin-1", "replace")
    return s if all(32 <= ord(c) < 127 for c in s) else None


def _stable_header(p, sub):
    """Two-pass read of (base, size, mask); None on a torn read / OSError."""
    try:
        b1 = p.u64(sub + CV_BASE); w1 = p.u64(sub + CV_SIZE_MASK)
        b2 = p.u64(sub + CV_BASE); w2 = p.u64(sub + CV_SIZE_MASK)
    except OSError:
        return None
    if (b1, w1) != (b2, w2):
        return None
    return b1, (w1 & 0xFFFFFFFF), (w1 >> 32)


def enumerate_variables(p, obj, strict=True):
    """Return {name: float} for one country object's CVariables, or {} (fail-closed).

    Hardening (Codex-reviewed 2026-06-27):
      * RTTI-gate the submanager (.?AVCVariables@@).
      * Stable two-pass header read (torn-read guard); validate nb = mask+1 is a power of
        two and size <= nb (rejects a wrong-endian size/mask split).
      * Auto-detect the node-grid phase, but COMPLETENESS-GATE the result: only accept a
        phase whose occupied {name,value} node count EXACTLY equals `size` with NO duplicate
        key. A wrong phase (or a tombstone-polluted grid) yields count != size -> rejected.
        This is the fail-closed guard against (a) a false grid pairing a real name with the
        wrong +0x20 value, and (b) stale/tombstone nodes inflating the bag. If NO phase
        reconciles to exactly `size` distinct keys, return {} (incomplete) rather than a
        partial/over-full bag.
    Returns the {name: value} bag ONLY when it is provably complete (len == size); else {}."""
    try:
        sub = p.u64(obj + OBJ_VARIABLES)
    except OSError:
        return {}
    if not heap_ok(sub):
        return {}
    if strict and _rtti_name(p, sub) != RTTI_VARIABLES:
        return {}
    hdr = _stable_header(p, sub)
    if hdr is None:
        return {}
    base, size, mask = hdr
    nb = mask + 1
    if size == 0:
        return {}
    # nb must be a power of two (mask = nb-1); size cannot exceed the bucket count.
    if not heap_ok(base) or size > MAX_SIZE or nb < 1 or nb > MAX_BUCKETS:
        return {}
    if (mask & nb) != 0 or size > nb:
        return {}
    span = (nb + 4) * NODE
    end = base + span
    exact = None        # a phase whose distinct-key count == size (complete)
    best_partial = {}   # fallback for diagnostics only (NOT returned)
    for phase in range(0, NODE, 8):
        tmp = {}
        dup = False
        a = base + phase
        while a + NODE <= end:
            k = _read_key(p, a)
            if k is not None:
                if k in tmp:
                    dup = True          # duplicate key on this phase -> not the clean grid
                try:
                    raw = p.i64(a + NODE_VALUE)
                    tmp[k] = raw / VALUE_SCALE
                except OSError:
                    pass
            a += NODE
        if len(tmp) > len(best_partial):
            best_partial = tmp
        # COMPLETENESS GATE: exactly `size` distinct keys, no dup -> provably complete.
        if not dup and len(tmp) == size:
            if exact is not None and exact != tmp:
                return {}               # two phases both reconcile differently -> ambiguous
            exact = tmp
    return exact if exact is not None else {}


def read_variables_tristate(p, obj, strict=True):
    """TRI-STATE variant of enumerate_variables (Codex wave-1 fix): distinguish a
    MEASURED-EMPTY container (size == 0, a real read of an empty CVariables) from an
    UNMEASURED one (RTTI mismatch / torn read / ambiguous phase / parse failure), so a
    caller never writes a fabricated {} for a country it could not actually read.

    Returns (bag, measured):
      * ({}, True)   -> the container resolved and is genuinely EMPTY (size == 0).
      * (bag, True)  -> a provably-complete bag (len(bag) == size).
      * (None, False)-> UNMEASURED: could not resolve / verify the container; the caller
                        must leave the leaf null (no fabrication).
    """
    try:
        sub = p.u64(obj + OBJ_VARIABLES)
    except OSError:
        return None, False
    if not heap_ok(sub):
        return None, False
    if strict and _rtti_name(p, sub) != RTTI_VARIABLES:
        return None, False
    hdr = _stable_header(p, sub)
    if hdr is None:
        return None, False
    base, size, mask = hdr
    if size == 0:
        return {}, True                      # MEASURED-EMPTY (real, allocated, empty)
    bag = enumerate_variables(p, obj, strict=strict)
    # enumerate_variables returns {} on a parse miss for a NON-zero size; that is an
    # UNMEASURED failure here (we already handled the genuine size==0 case above).
    if not bag:
        return None, False
    return bag, True


def read_country_variables(p, bridge, rec0, count, strict=True):
    """LIVE per-country variables. Returns {TAG: {name: float, ...}} for bridged tags
    that have >=1 variable. `bridge` = {running_index: TAG}; rec0/count = live array."""
    out = {}
    for i, tag in bridge.items():
        rec = rec0 + i * REC
        try:
            if p.u16(rec + OFF_INDEX) != i:
                continue
            obj = p.u64(rec + OFF_NAMEPTR)
        except OSError:
            continue
        if not heap_ok(obj):
            continue
        vd = enumerate_variables(p, obj, strict=strict)
        if vd:
            out[tag] = vd
    return out


# ==========================================================================
# CLI / validation
# ==========================================================================
def _locate_and_bridge(p):
    """Locate the live country array by localized name (no PP needed) and build the
    index->TAG bridge by matching each country's localized name to a tag table is not
    available here; instead we return rec0/count and a name map so --validate can
    fingerprint-bridge against the oracle's variable-name sets."""
    res = L.locate(p)
    if res is None:
        raise SystemExit("could not locate the live country array")
    rec0, _begin, count = res
    return rec0, count


def _validate(p, oracle_path):
    rec0, count = _locate_and_bridge(p)
    oracle = json.load(open(oracle_path, encoding="utf-8"))["countries"]
    # oracle fingerprint: non-'random' variable name->value per tag
    ora_fp = {}
    for tag, c in oracle.items():
        v = {k: val for k, val in (c.get("variables") or {}).items() if k != "random"}
        if v:
            ora_fp[tag] = v
    # gather live per index
    live = {}
    for i in range(count):
        rec = rec0 + i * REC
        try:
            if p.u16(rec + OFF_INDEX) != i:
                continue
            obj = p.u64(rec + OFF_NAMEPTR)
        except OSError:
            continue
        if not heap_ok(obj):
            continue
        vd = enumerate_variables(p, obj)
        if vd:
            live[i] = vd
    # bridge each oracle tag to the live index whose name set overlaps most
    rows = []
    total_o = total_m = 0
    used = set()
    for tag, ovars in sorted(ora_fp.items(), key=lambda kv: -len(kv[1])):
        onames = set(ovars)
        best_i, best_score = None, -1
        for i, vd in live.items():
            if i in used:
                continue
            inter = len(onames & set(vd))
            if inter > best_score:
                best_score, best_i = inter, i
        if best_i is None or best_score < max(3, len(onames) // 2):
            continue
        used.add(best_i)
        vd = live[best_i]
        match = sum(1 for k in ovars if k in vd and abs(vd[k] - float(ovars[k])) < 1e-3)
        rows.append((tag, len(ovars), match))
        total_o += len(ovars); total_m += match
    print("\n%-5s %7s %7s" % ("TAG", "oracle", "match"))
    for tag, no, nm in sorted(rows, key=lambda r: -r[1]):
        print("%-5s %7d %7d%s" % (tag, no, nm, "" if nm == no else "  <-- partial"))
    print("\nVARIABLES live-vs-save: %d / %d exact across %d bridged countries"
          % (total_m, total_o, len(rows)))


def main():
    ap = argparse.ArgumentParser(description="LIVE per-country variables reader (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe SHA-256 doesn't match the mapped build")
    ap.add_argument("--validate", action="store_true",
                    help="locate, enumerate and compare to an oracle JSON")
    ap.add_argument("--oracle", default=None,
                    help="path to the melted-save oracle JSON (countries[*].variables)")
    args = ap.parse_args()

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        check_build_gate(p, args.force)
        if args.validate:
            if not args.oracle:
                raise SystemExit("--validate needs --oracle PATH")
            _validate(p, args.oracle)
            return
        rec0, count = _locate_and_bridge(p)
        n = 0
        for i in range(count):
            rec = rec0 + i * REC
            try:
                if p.u16(rec + OFF_INDEX) != i:
                    continue
                obj = p.u64(rec + OFF_NAMEPTR)
            except OSError:
                continue
            if not heap_ok(obj):
                continue
            vd = enumerate_variables(p, obj)
            if vd:
                nm = L.read_msvc_str(p, obj + 0x10) or "?"
                print("idx %3d  %-26s  %d vars" % (i, nm[:26], len(vd)))
                n += 1
        print("\ncountries with variables: %d" % n)


if __name__ == "__main__":
    main()
