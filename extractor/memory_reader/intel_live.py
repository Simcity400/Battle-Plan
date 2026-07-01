"""intel_live.py - READ-ONLY live INTELLIGENCE reader for Battle Plan.

Reads every country's intelligence_agency LIVE from a running PAUSED hoi4.exe via
ReadProcessMemory only (READ-ONLY; no writes / no injection / no thread suspension,
so it never desyncs MP). The agency hangs off the country object:

    running_index  -> rec0 + index*0x120                      (locate.py)
    country object -> [rec + 0x08]                             (the obj whose +0x10 is the name)
    agency struct  -> [country_object + 0xFC0]

PINNED agency fields (offsets relative to the agency struct base; see
intel_offsets.draft.json), each validated live==save on build 16022851 against the
lockstep melt (save_date 1946.5.19.9):

    +0x80  name              msvc_std_string   (MI6 / NKVD / Abwehr ...)   13/14 EXACT
    +0xA0  icon              msvc_std_string   (GFX_intelligence_agency_logo_*)
    +0xC8  upgrade_progress  i32 /100000       (2400000 -> 24.0)            14/14 EXACT
    +0xE4  max_operative_count   i32                                       43/44 EXACT
    +0xF0  usable_operative_slots i32 (mirror +0xF4; == max this frame)    cluster confirmed
    +0x128 defense           i32 /100000       (662500 -> 6.625)           44/44 EXACT

DATA-GAPS (reported, not faked):
  * usable_operative_slots vs max_operative_count: equal this frame; the precise
    offset-within-the-{+0xE4,+0xF0,+0xF4} cluster needs a max>usable frame.
  * operatives: the GLOBAL operative table is located (record: type=4713@+0, op_id@+4,
    skill@~+0xC, sparse 0x50-stride hash) but robust enumeration + per-operative XP
    (the same internal-accumulator transform the character domain hits) are unpinned.
  * cryptology.targets / operations: not yet pinned live (save oracle is authoritative).

HARD RULES honored (same as characters_live.py / combat_enum.py):
  * READ-ONLY memory; keep the game PAUSED; passive reads => any cadence, no desync.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the build.
  * No personal paths; self-contained; reuses winmem.py + locate.py + the
    country_index_bridge for save-free TAG<->country-object resolution.
"""

import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
import winmem  # noqa: E402
import locate  # noqa: E402
sys.path.insert(0, os.path.join(_HERE, "live_layers"))
from live_layers import country_index_bridge as cib  # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# --- pinned offsets (intel_offsets.draft.json) ---
COUNTRY_VTABLE_RVA = 0x27C0E80  # all 144 country objects share this vtable (+0x00).
OBJ_NAME = 0x10                 # localized country name std::string within the country object
OBJ_AGENCY_PTR = 0xFC0          # country_object + this -> agency struct pointer
AG_NAME = 0x80                  # msvc_std_string
AG_ICON = 0xA0                  # msvc_std_string
AG_UPGRADE_PROGRESS = 0xC8      # i32 /100000
AG_MAX_OPERATIVES = 0xE4        # i32
AG_USABLE_SLOTS = 0xF0          # i32 (mirror +0xF4)
AG_DEFENSE = 0x128              # i32 /100000
FP_SCALE = 100000.0
HEAP_MIN = 0x10000000000
HEAP_MAX = 0x7FFFFFFFFFFF


def _i32_div(p, addr, scale=FP_SCALE):
    try:
        return p.i32(addr) / scale
    except OSError:
        return None


def read_agency(p, country_object):
    """Read one country's intelligence_agency. Returns a dict, or None if the country
    has no agency struct (pointer out of range)."""
    try:
        agp = p.u64(country_object + OBJ_AGENCY_PTR)
    except OSError:
        return None
    if not (HEAP_MIN <= agp <= HEAP_MAX):
        return None
    name = locate.read_msvc_str(p, agp + AG_NAME, maxlen=96)
    icon = locate.read_msvc_str(p, agp + AG_ICON, maxlen=128)
    try:
        max_ops = p.i32(agp + AG_MAX_OPERATIVES)
        slots = p.i32(agp + AG_USABLE_SLOTS)
    except OSError:
        max_ops = slots = None
    return {
        "agency_ptr": agp,
        "name": name,
        "icon": icon,
        "upgrade_progress": _i32_div(p, agp + AG_UPGRADE_PROGRESS),
        "max_operative_count": max_ops,
        "usable_operative_slots": slots,   # == max this frame (data-gap, see draft)
        "defense": _i32_div(p, agp + AG_DEFENSE),
    }


def _live_country_object(p, obj):
    """Cheap liveness/structural guard (Codex): this mod has freed-country-object
    hazards after annexation, so a committed qword matching the vtable is NOT proof of
    a live object. Accept only if the country NAME std::string resolves AND it is
    stable across a re-read (TOCTOU) AND the agency pointer (if present) is in range."""
    nm = locate.read_msvc_str(p, obj + OBJ_NAME, maxlen=64)
    if not nm:
        return None
    if locate.read_msvc_str(p, obj + OBJ_NAME, maxlen=64) != nm:   # double-read
        return None
    try:
        agp = p.u64(obj + OBJ_AGENCY_PTR)
    except OSError:
        return None
    if agp != 0 and not (HEAP_MIN <= agp <= HEAP_MAX):
        return None
    return nm


def locate_country_objects_by_vtable(p, heap_lo=HEAP_MIN, heap_hi=HEAP_MAX):
    """Save-free, array-free: scan committed heap for objects whose +0x00 qword ==
    the country-object vtable (module_base + COUNTRY_VTABLE_RVA), then keep only those
    that pass _live_country_object(). Returns {country_object_addr: localized_name}.
    Same RTTI-vtable technique as the character domain; robust to the country-array
    container's owner layout (which can defeat locate.locate()). ~30s on this build.

    NOTE: COUNTRY_VTABLE_RVA is per-build (valid only under the SHA-256 gate). For a
    multi-build reader, RTTI-resolve the vtable at startup and assert it equals this
    RVA (fail closed on mismatch)."""
    vt = p.module_base + COUNTRY_VTABLE_RVA
    needle = vt.to_bytes(8, "little")
    seen = {}
    CH = 64 << 20
    for base, size in p.iter_regions(heap_lo, heap_hi):
        pos = 0
        while pos < size:
            want = min(CH, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                s = 0
                while True:
                    i = blob.find(needle, s)
                    if i < 0:
                        break
                    s = i + 1
                    a = base + pos + i
                    if a % 8 == 0 and a not in seen:
                        nm = _live_country_object(p, a)
                        if nm:
                            seen[a] = nm
                blen = len(blob)
                if pos + blen >= size or blen < want:
                    pos += max(blen, locate.PAGE)
                else:
                    pos += blen - 16
            else:
                pos += max(want, locate.PAGE)
    return seen


def resolve_country_objects(p, bridge=None):
    """{key: country_object}. PRIMARY: the country-index bridge (gives TAGs). FALLBACK
    (when the bridge's array locate fails this session): vtable scan, keyed by the
    localized country name (no tag-table join). Returns {'bridge': br} or
    {'objs': {name: obj}}.

    ``bridge`` (optional): an ALREADY-RESOLVED CountryIndexBridge (e.g. the one the
    capture context resolved once and pins). When given AND non-None it is REUSED
    verbatim, skipping the per-call CountryIndexBridge.resolve heap scan -- the single
    biggest steady-state cost for the country-object readers. The caller is responsible
    for only passing a bridge that belongs to THIS process (the capture context pins it
    under the process fingerprint, so it is fail-closed); passing None preserves the
    original self-resolving behaviour exactly."""
    if bridge is not None:
        return {"bridge": bridge}
    br = cib.CountryIndexBridge.resolve(p)
    if br is not None:
        return {"bridge": br}
    objs = {nm: obj for obj, nm in locate_country_objects_by_vtable(p).items()}
    return {"objs": objs} if objs else None


def read_all(p, bridge=None):
    """Resolve every active country (save-free) and read its agency. Returns
    {TAG_or_index: agency_dict} for countries that have an agency. When only the
    fingerprint fallback is available (no bridge), keys are the running_index +
    localized name instead of the TAG (the tag table could not be joined).

    ``bridge`` (optional): reuse an already-resolved CountryIndexBridge to skip the
    per-call country-array re-resolve scan (steady-state perf)."""
    loc = resolve_country_objects(p, bridge=bridge)
    if loc is None:
        raise RuntimeError("could not locate the country array (bridge + fingerprint "
                           "fallback both failed)")
    out = {}
    if "bridge" in loc:
        br = loc["bridge"]
        for tag_db_id in br.as_map():
            obj = br.country_object(tag_db_id)
            if not obj:
                continue
            ag = read_agency(p, obj)
            if ag is None or (ag["name"] is None and not ag["max_operative_count"]):
                continue
            out[br.tag(tag_db_id)] = ag
        return out
    # fallback: key by localized country name (no tag table joined)
    for nm, obj in loc["objs"].items():
        ag = read_agency(p, obj)
        if ag is None or (ag["name"] is None and not ag["max_operative_count"]):
            continue
        out[nm] = ag
    return out


def _build_gate(p):
    sha = p.fingerprint().get("sha256")
    if sha != EXPECTED_SHA256:
        raise SystemExit("build gate FAIL: live hoi4.exe sha256 %s != mapped build %s\n"
                         "Offsets are build-specific; refusing to read an unknown build."
                         % (sha, EXPECTED_SHA256))


def main():
    p = winmem.open_process(PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _build_gate(p)
        print("build gate OK; reading intelligence agencies (READ-ONLY)...")
        agencies = read_all(p)
        named = {t: a for t, a in agencies.items() if a["name"]}
        print("countries with an agency: %d (%d named)" % (len(agencies), len(named)))
        print("\n%-4s %-34s %-6s %-6s %-7s %s"
              % ("TAG", "agency", "maxop", "slots", "def", "upg%"))
        for tag in sorted(named):
            a = named[tag]
            print("%-4s %-34s %-6s %-6s %-7s %s"
                  % (tag, (a["name"] or "")[:34], a["max_operative_count"],
                     a["usable_operative_slots"],
                     ("%.3f" % a["defense"]) if a["defense"] is not None else "-",
                     ("%.2f" % a["upgrade_progress"]) if a["upgrade_progress"] is not None else "-"))


if __name__ == "__main__":
    main()
