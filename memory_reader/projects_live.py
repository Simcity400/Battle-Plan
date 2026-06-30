"""projects_live.py - LIVE SPECIAL-PROJECTS / FACILITIES / SCIENTISTS reader.

STATUS on build 16022851 (read-only, save-free): facilities + project identity +
scientist skills/name/traits PINNED and validated EXACT vs the time-aligned melted
save (save_oracle/parse_projects.py on re_scratch/melt_sov_1946.txt, 1946.5.19.9).

There is no base pointer / id-indexed array; each domain is a pool-allocated object
enumerated by reverse-scanning committed heap for the object's stable RTTI vtable
(MODULE+rva), exactly like the CDivision / CProvince / MIO pools.

PINNED + validated (see projects_offsets.draft.json for the full ledger):

  FACILITIES  (CProgram, vtable RVA 0x294AB40)         25/25 vs save
    facility_id              +0x0C  u32
    province_id              +0x1C  u32   (primary join key)
    building_name            +0x20  msvc std::string (base+0x20,size+0x30,cap+0x38)
    active_project_handle    +0x50  u32   (0x56xx static-def db-id; 0 idle; == CProject+0x18)
    active_project_flag      +0x54  u32   (1 active / 0 idle)
    scientist_char_id        +0x5C  u32   (0 unassigned)   [+0x58 == 73 tag if assigned]
    unread_prototype_rewards +0x178 u32

  PROJECTS    (CProject, vtable RVA 0x295C190)          6721 live == 6721 save
    type_tag                 +0x08  u32 (==86)
    project_id               +0x0C  u32
    static_def_handle        +0x18  u32   (which token; joins to facility+0x50)
    owner_facility_ref       +0x60  {u32 type==83, u32 facility_id}  present iff worked
    iterations               +0x74  u32
    (project_progress / phase_progress are serialize-time COMPUTED - NOT stored live)

  SCIENTISTS  (CCharacter type 73, vtable RVA 0x2957D40)
    char_id        +0x0C  u32
    display_name   +0x28  msvc std::string
    name_token     +0x48  msvc std::string   (loc key, e.g. GER_heisenberg)
    skills         char+0xC0 -> +0x110 : 4 records stride 0x20 [nuc,nav,air,land], level i32@+0x18
    traits         char+0xC0 (CScientist) -> {begin +0x30, count u32 +0x38, stride 8}

CANDIDATE-ZERO / COMPUTED / UNRESOLVED (documented, never faked):
  facility dismantle/unassigned countdowns + supply.motorization : invariant 0 this
    frame -> candidate-zero data-gap (live fields exist; no variance to validate).
  project project_progress / phase_progress : serialize-time computed (a global heap
    scan for the exact values found ZERO live hits).
  project current_state / start / completed : live in the CProjectStateMachine sub-
    object; the honest worked/idle signal is owner_facility_ref(+0x60) presence (exact).
  scientist trait NAMES : the trait elements are static-def db-id handles (not heap
    pointers); resolving handle->name needs the trait-def registry (base not located).
    The trait COUNT is pinned (311/311).
  scientist injured-state + injury cooldown : runtime-computed, absent from the save.

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes, no injection, no thread suspension. fail-closed on a build
mismatch (offsets are build-specific).
"""

import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import winmem  # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# ----- RTTI-resolved object pools (module-relative vtable RVAs, build 16022851) -----
FAC_VT_RVA = 0x294AB40    # CProgram (facility)
PROJ_VT_RVA = 0x295C190   # CProject
CHAR_VT_RVA = 0x2957D40   # CCharacter
SCI_VT_RVA = 0x2957D98    # CScientist (sub-object off char+0xC0)

BUILD_PROFILES = {
    "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120": {
        "facility_vtable_rva": 0x294AB40,
        "project_vtable_rva": 0x295C190,
        "character_vtable_rva": 0x2957D40,
        "scientist_vtable_rva": 0x2957D98,
    },
    # RTTI-derived on the current vanilla_latest_2026_06_30 executable. Field
    # offsets are inherited from the 16022851 ledger and must still be validated
    # by the compact DLL proof before this profile is treated as replay-complete.
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad": {
        "facility_vtable_rva": 0x295AC30,
        "project_vtable_rva": 0x296C270,
        "character_vtable_rva": 0x272B088,
        "scientist_vtable_rva": 0x2967E88,
    },
}


def apply_build_profile(sha256):
    key = str(sha256 or "").lower()
    profile = BUILD_PROFILES.get(key)
    if not profile:
        return False
    global EXPECTED_SHA256, FAC_VT_RVA, PROJ_VT_RVA, CHAR_VT_RVA, SCI_VT_RVA
    EXPECTED_SHA256 = key
    FAC_VT_RVA = profile["facility_vtable_rva"]
    PROJ_VT_RVA = profile["project_vtable_rva"]
    CHAR_VT_RVA = profile["character_vtable_rva"]
    SCI_VT_RVA = profile["scientist_vtable_rva"]
    return True

# ----- facility (CProgram) offsets -----
FAC_TYPE = 0x08            # u32 == 83
FAC_ID = 0x0C             # u32
FAC_PROV = 0x1C           # u32
FAC_BLD_STR = 0x20        # msvc std::string base
FAC_ACT_HANDLE = 0x50     # u32 active-project static-def handle (0 idle)
FAC_ACT_FLAG = 0x54       # u32 1 active / 0 idle
FAC_SCI_TAG = 0x58        # u32 == 73 when a scientist is assigned
FAC_SCI_ID = 0x5C         # u32 scientist char_id (0 unassigned)
FAC_UNREAD = 0x178        # u32 unread_prototype_rewards
FAC_TYPE_VAL = 83

# ----- project (CProject) offsets -----
PROJ_TYPE = 0x08          # u32 == 86
PROJ_ID = 0x0C            # u32
PROJ_DEF_HANDLE = 0x18    # u32 static-def handle (which token; joins facility+0x50)
PROJ_OWNER_TAG = 0x60     # u32 == 83 when actively worked
PROJ_OWNER_FID = 0x64     # u32 owning facility id (when worked)
PROJ_ITERS = 0x74         # u32 iterations
PROJ_TYPE_VAL = 86

# ----- scientist (CCharacter type 73) offsets -----
CHAR_TYPE = 0x08          # u32 == 73
CHAR_ID = 0x0C            # u32
CHAR_DISPLAY_STR = 0x28   # msvc std::string base (display name)
CHAR_TOKEN_STR = 0x48     # msvc std::string base (name_token / loc key)
CHAR_SCI_PTR = 0xC0       # u64 -> CScientist sub-object
CHAR_TYPE_VAL = 73

# skills: [char+0xC0] -> +0x110 = skills array base; 4 records stride 0x20
SCI_SKILLS_PTR2 = 0x110
SKILL_STRIDE = 0x20
SKILL_LEVEL_OFF = 0x18
SKILL_HDR_BASE = 0x56F7
SPEC_ORDER = ("specialization_nuclear", "specialization_naval",
              "specialization_air", "specialization_land")

# traits: [char+0xC0] (CScientist) -> {begin +0x30, count u32 +0x38, stride 8}
SCI_TRAITS_BEGIN = 0x30
SCI_TRAITS_COUNT = 0x38
SCI_TRAIT_STRIDE = 8


# ---------------------------------------------------------------------------
def _verify_build(p):
    sha = p.fingerprint().get("sha256")
    apply_build_profile(sha)
    if sha != EXPECTED_SHA256:
        raise SystemExit(
            "BUILD MISMATCH: live hoi4.exe sha256=%s != expected %s. "
            "Offsets are build-specific; refusing to read." % (sha, EXPECTED_SHA256))


def _enum_vtable(p, vt_abs):
    """Yield every 8-aligned heap address whose qword == vt_abs (object base)."""
    needle = struct.pack("<Q", vt_abs)
    CH = 16 << 20
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
                pos += max(1, len(blob) - 16)
            else:
                pos += max(1, want)


def _read_msvc_str(p, base):
    """MSVC std::string whose struct begins at `base`: buffer[16]@+0, size(u64)@+0x10,
    cap(u64)@+0x18; SSO when cap<16 (chars inline at +0), else +0 is a heap ptr.
    Reads fields individually (region-clamp safe). Returns str or None."""
    try:
        size = p.u64(base + 0x10)
        cap = p.u64(base + 0x18)
    except OSError:
        return None
    if cap < size or size > 256:
        return None
    if size == 0:
        return ""
    try:
        chars = base if cap < 16 else p.u64(base)
        data = p.read(chars, size)
    except OSError:
        return None
    try:
        return data.decode("latin-1")
    except Exception:
        return None


# ---------------------------------------------------------------------------
# WARM-WALK PIN (paused-game cache). The game is PAUSED for the whole capture
# session, so no object is allocated/freed/moved between walks. That lets us
# LOCATE the object addresses once (cold, full vtable sweep) and on warm walks
# SKIP the sweep, re-reading every field LIVE from the cached addresses.
#
# Each pool is split into three primitives:
#   enumerate_<pool>_addrs(p)        -> [(addr, idfp), ...] (SWEEP + type-gate only;
#                                       deterministic sweep order; carries a cold
#                                       per-object IDENTITY FINGERPRINT for ABA defense)
#   read_<pool>_from_addrs(p, addrs) -> [row, ...]          (the SAME row dicts the
#                                       public reader returns, decoded LIVE from the
#                                       supplied addrs; values never frozen)
#   validate_<pool>_addrs(p, addrs)  -> bool                (FAIL-CLOSED liveness gate)
#
# The public reader is now: enumerate -> read_from_addrs (ONE decode path), so the
# cold output is byte-identical to before. enumerate reads only the MINIMAL bytes it
# needs (type-gate + identity fingerprint), never the decoded fields, so the cold
# decode reads each emitted field exactly once (no double-read of decoded data; the
# Codex review flagged a value-mutation gap if both reads decoded the same field).
#
# ABA defense (object freed and a DIFFERENT same-pool object reallocated at the SAME
# address): vtable+type alone do NOT catch it (the replacement is the same class). So
# validate compares a per-object IDENTITY FINGERPRINT captured cold by enumerate.
# +0x0C alone is NOT sufficient for every pool - the offset ledger documents CProject
# project_id as "unique within a country's pool", NOT globally unique (Codex's concrete
# refutation: a different-country CProject reused at the same addr could share id 7). So
# the fingerprint is a COMPOSITE of the strongest cheap stable id fields each struct has:
#   facility  : (facility_id +0x0C, province_id +0x1C)    province is the unique join key
#   project   : (project_id +0x0C, static_def_handle +0x18, owner_tag +0x60, owner_fid +0x64)
#               per-country inst id + token + the globally-unique working facility (0,0 idle)
#   scientist : (char_id     +0x0C,)   CCharacter char_id is the GLOBAL char id-space
#                                       (770/770 + 183/183 unique elsewhere) - already global
# validate re-reads vtable(+0x00) + type(+0x08) + the SAME fingerprint fields and requires
# the whole tuple to match. Any mismatch / unreadable / short read / missing cold
# fingerprint -> False (fail-closed) -> caller must fall back to a full cold sweep.
#
# read_<pool>_from_addrs accepts either bare int addrs or (addr, idfp) tuples (it only
# needs the addr to re-decode); validate requires the (addr, idfp) form (a bare int has
# no cold fingerprint -> fail-closed, ABA cannot be defended).
# NOTE: validate over an EMPTY addr list returns True (vacuously - "all zero of these
# addrs are still live"). A genuinely-empty pool is a legitimate cold result; detecting
# cache LOSS/truncation (count drift) is the caller's job, not this per-addr liveness gate.

def _strict_int(v):
    """An int that is NOT a bool (bool is an int subclass). None if v is any other type.
    Used to keep addr/fingerprint parsing fail-closed and strict (no '123'/1.0/True coercion
    that could spuriously compare equal)."""
    return v if (type(v) is int) else None


def _split_addr_fp(item):
    """Accept a cached entry as a bare int addr or an (addr, idfp) pair. TOTALLY
    exception-proof and strict: returns (addr_or_None, fingerprint_tuple_or_None) for ANY
    Python input, never raising. The fingerprint enumerate stores is always a tuple of
    plain ints; ANY other shape (bad/empty container, non-int addr, missing/scalar/
    JSON-mangled fingerprint, non-int members, even a hostile tuple/list subclass whose
    len/getitem/iter raises) yields None so the caller fails CLOSED rather than raising
    (Codex flagged IndexError/TypeError/coercion/subclass paths). read_from_addrs only
    needs the addr; validate needs the fingerprint."""
    try:
        # bare int addr (no fingerprint)
        if type(item) is int:
            return item, None
        if not isinstance(item, (tuple, list)) or len(item) == 0:
            return None, None
        addr = _strict_int(item[0])
        if len(item) < 2:
            return addr, None
        fp = item[1]
        if not isinstance(fp, (tuple, list)):   # malformed -> cannot defend ABA -> None
            return addr, None
        members = [_strict_int(x) for x in fp]
        if any(m is None for m in members):     # any non-int member -> fail closed
            return addr, None
        return addr, tuple(members)
    except Exception:
        # a hostile container subclass (len/getitem/iter that raises) -> fail closed
        return None, None


# ----- facilities (CProgram) ----------------------------------------------
# Identity fingerprint window: vtable(+0x00) .. province_id(+0x1C) -> 0x20 bytes.
_FAC_FP_LEN = FAC_PROV + 4


def _fac_fingerprint(b):
    """(facility_id, province_id) from a >= _FAC_FP_LEN byte object slice."""
    return (struct.unpack_from("<I", b, FAC_ID)[0],
            struct.unpack_from("<I", b, FAC_PROV)[0])


def enumerate_facility_addrs(p):
    """Locate the facility object addresses (cold sweep + type-gate ONLY; reads just the
    identity window, NOT the decoded fields). Returns a deterministic-order list of
    (addr, (facility_id, province_id)) pairs; the fingerprint is captured for ABA defense
    in validate_facility_addrs."""
    out = []
    vt = p.module_base + FAC_VT_RVA
    for a in _enum_vtable(p, vt):
        b = p.try_read(a, _FAC_FP_LEN)
        if len(b) < _FAC_FP_LEN:
            continue
        if struct.unpack_from("<I", b, FAC_TYPE)[0] != FAC_TYPE_VAL:
            continue
        out.append((a, _fac_fingerprint(b)))
    return out


def read_facilities_from_addrs(p, addrs):
    """Decode facility row dicts LIVE from the supplied (cached) addresses. Produces
    the SAME list read_facilities returns. `addrs` items may be bare int addrs or
    (addr, idfp) tuples (only the addr is used)."""
    out = []
    for item in addrs:
        a, _fp = _split_addr_fp(item)
        if a is None:
            continue
        b = p.try_read(a, 0x180)
        if len(b) < 0x180:
            continue
        if struct.unpack_from("<I", b, FAC_TYPE)[0] != FAC_TYPE_VAL:
            continue
        sci_id = struct.unpack_from("<I", b, FAC_SCI_ID)[0]
        act_flag = struct.unpack_from("<I", b, FAC_ACT_FLAG)[0]
        out.append({
            "id": struct.unpack_from("<I", b, FAC_ID)[0],
            "province": struct.unpack_from("<I", b, FAC_PROV)[0],
            "building": _read_msvc_str(p, a + FAC_BLD_STR),
            "active_project_handle": struct.unpack_from("<I", b, FAC_ACT_HANDLE)[0],
            "active": bool(act_flag),
            "scientist_char_id": sci_id if sci_id else None,
            "unread_prototype_rewards": struct.unpack_from("<I", b, FAC_UNREAD)[0],
            "_addr": a,
        })
    return out


def validate_facility_addrs(p, addrs):
    """FAIL-CLOSED liveness gate for cached facility addresses. For EVERY cached addr
    re-read vtable(+0x00)==module_base+FAC_VT_RVA, type(+0x08)==FAC_TYPE_VAL, and the
    identity fingerprint (facility_id +0x0C, province_id +0x1C) and require it to equal
    the cold fingerprint captured by enumerate (ABA defense - province is the unique
    facility join key, so a reused slot of a DIFFERENT facility fails). A bare-int entry
    (no cold fingerprint), a short read, or any mismatch -> False."""
    vt = p.module_base + FAC_VT_RVA
    for item in addrs:
        a, cold_fp = _split_addr_fp(item)
        if a is None or cold_fp is None:
            return False        # bad addr / no cold fingerprint -> cannot defend ABA -> fail closed
        b = p.try_read(a, _FAC_FP_LEN)
        if len(b) < _FAC_FP_LEN:
            return False
        if struct.unpack_from("<Q", b, 0)[0] != vt:
            return False
        if struct.unpack_from("<I", b, FAC_TYPE)[0] != FAC_TYPE_VAL:
            return False
        if _fac_fingerprint(b) != tuple(cold_fp):
            return False
    return True


def read_facilities(p):
    """List of facility dicts read live from the CProgram pool."""
    addrs = enumerate_facility_addrs(p)
    return read_facilities_from_addrs(p, addrs)


# ----- projects (CProject) ------------------------------------------------
# Identity fingerprint window: vtable(+0x00) .. owner_facility_ref(+0x60/+0x64) -> 0x68
# bytes. project_id alone is only per-country-unique (ledger), so the fingerprint is the
# composite (project_id, static_def_handle, owner_tag, owner_fid):
#   - static_def_handle (+0x18) = which project TOKEN (e.g. sp_air_jet_engine)
#   - owner_facility_ref (+0x60 tag==83, +0x64 facility_id) = the GLOBALLY-unique facility
#     currently working it (0,0 when idle). For a worked project this binds identity to a
#     specific facility, so a cross-country reuse with the same (id, token) still fails.
# All four are stable for the PAUSED capture session (the HARD no-mutation invariant), so
# they never cause a spurious invalidation within a session.
_PROJ_FP_LEN = PROJ_OWNER_FID + 4


def _proj_fingerprint(b):
    """(project_id, static_def_handle, owner_tag, owner_fid) from a >= _PROJ_FP_LEN slice."""
    return (struct.unpack_from("<I", b, PROJ_ID)[0],
            struct.unpack_from("<I", b, PROJ_DEF_HANDLE)[0],
            struct.unpack_from("<I", b, PROJ_OWNER_TAG)[0],
            struct.unpack_from("<I", b, PROJ_OWNER_FID)[0])


def enumerate_project_addrs(p):
    """Locate the project object addresses (cold sweep + type-gate ONLY; reads just the
    identity window, NOT the decoded fields). Returns a deterministic-order list of
    (addr, (project_id, static_def_handle)) pairs. The active_only filter is a per-ROW
    filter, NOT an enumeration gate, so it is applied in read_projects_from_addrs (the
    full pool is enumerated/pinned here)."""
    out = []
    vt = p.module_base + PROJ_VT_RVA
    for a in _enum_vtable(p, vt):
        b = p.try_read(a, _PROJ_FP_LEN)
        if len(b) < _PROJ_FP_LEN:
            continue
        if struct.unpack_from("<I", b, PROJ_TYPE)[0] != PROJ_TYPE_VAL:
            continue
        out.append((a, _proj_fingerprint(b)))
    return out


def read_projects_from_addrs(p, addrs, active_only=False):
    """Decode project row dicts LIVE from the supplied (cached) addresses. Produces the
    SAME list read_projects(active_only=...) returns, preserving the active_only filter.
    `addrs` items may be bare int addrs or (addr, idfp) tuples (only the addr is used)."""
    out = []
    for item in addrs:
        a, _fp = _split_addr_fp(item)
        if a is None:
            continue
        b = p.try_read(a, 0x78)
        if len(b) < 0x78:
            continue
        if struct.unpack_from("<I", b, PROJ_TYPE)[0] != PROJ_TYPE_VAL:
            continue
        owner_tag = struct.unpack_from("<I", b, PROJ_OWNER_TAG)[0]
        owner_fid = struct.unpack_from("<I", b, PROJ_OWNER_FID)[0]
        worked = (owner_tag == 83 and owner_fid != 0)
        if active_only and not worked:
            continue
        out.append({
            "id": struct.unpack_from("<I", b, PROJ_ID)[0],
            "static_def_handle": struct.unpack_from("<I", b, PROJ_DEF_HANDLE)[0],
            "iterations": struct.unpack_from("<I", b, PROJ_ITERS)[0],
            "worked_at_facility": owner_fid if worked else None,
            "_addr": a,
        })
    return out


def validate_project_addrs(p, addrs):
    """FAIL-CLOSED liveness gate for cached project addresses. For EVERY cached addr
    re-read vtable(+0x00)==module_base+PROJ_VT_RVA, type(+0x08)==PROJ_TYPE_VAL, and the
    composite identity fingerprint (project_id +0x0C, static_def_handle +0x18, owner_tag
    +0x60, owner_fid +0x64) and require it to equal the cold fingerprint captured by
    enumerate. project_id alone is only unique within a country's pool (offset ledger), so
    a same-class object reused at this addr from ANOTHER country could share it; pairing it
    with the project token handle AND the globally-unique working-facility back-ref closes
    that cross-country ABA hole. A bare-int entry (no cold fingerprint), a short read, or
    any mismatch -> False."""
    vt = p.module_base + PROJ_VT_RVA
    for item in addrs:
        a, cold_fp = _split_addr_fp(item)
        if a is None or cold_fp is None:
            return False        # bad addr / no cold fingerprint -> cannot defend ABA -> fail closed
        b = p.try_read(a, _PROJ_FP_LEN)
        if len(b) < _PROJ_FP_LEN:
            return False
        if struct.unpack_from("<Q", b, 0)[0] != vt:
            return False
        if struct.unpack_from("<I", b, PROJ_TYPE)[0] != PROJ_TYPE_VAL:
            return False
        if _proj_fingerprint(b) != tuple(cold_fp):
            return False
    return True


def read_projects(p, active_only=False):
    """List of project dicts from the CProject pool. With active_only, return only the
    projects currently worked at a facility (owner_facility_ref present) - cheap and the
    usually-interesting set (6721 total otherwise)."""
    addrs = enumerate_project_addrs(p)
    return read_projects_from_addrs(p, addrs, active_only=active_only)


def _read_skills(p, char_addr):
    try:
        s1 = p.u64(char_addr + CHAR_SCI_PTR)
        if not (HEAP_LO <= s1 < HEAP_HI):
            return None
        base = p.u64(s1 + SCI_SKILLS_PTR2)
        if not (HEAP_LO <= base < HEAP_HI):
            return None
        blob = p.read(base, SKILL_STRIDE * 4)
    except OSError:
        return None
    skills = {}
    for k, name in enumerate(SPEC_ORDER):
        rec = k * SKILL_STRIDE
        hdr = struct.unpack_from("<H", blob, rec)[0]
        if hdr != SKILL_HDR_BASE + k:
            return None   # structural sanity check failed -> not a scientist skills obj
        skills[name] = struct.unpack_from("<i", blob, rec + SKILL_LEVEL_OFF)[0]
    return skills


def _read_trait_count(p, char_addr):
    try:
        s1 = p.u64(char_addr + CHAR_SCI_PTR)
        if not (HEAP_LO <= s1 < HEAP_HI):
            return None
        return p.u32(s1 + SCI_TRAITS_COUNT)
    except OSError:
        return None


# ----- scientists (CCharacter type 73 w/ a valid CScientist sub-object) ----
# Identity fingerprint window: vtable(+0x00) .. char_id(+0x0C) -> 0x10 bytes. char_id is
# the GLOBAL CCharacter id-space (already unique world-wide), so a 1-field fingerprint
# is the strongest the struct needs.
_SCI_FP_LEN = CHAR_ID + 4


def _sci_fingerprint(b):
    """(char_id,) from a >= _SCI_FP_LEN byte object slice."""
    return (struct.unpack_from("<I", b, CHAR_ID)[0],)


def enumerate_scientist_addrs(p):
    """Locate the scientist object addresses (cold sweep + gate ONLY). The scientist
    gate is type(+0x08)==73 AND a structurally-valid CScientist skills sub-object
    (_read_skills succeeds) - that second check is what separates scientists from
    leaders/other CCharacters, so it IS part of enumeration. Returns a
    deterministic-order list of (addr, (char_id,)) pairs; the char_id (+0x0C, the global
    CCharacter id) is captured for ABA defense in validate_scientist_addrs.

    NOTE: _read_skills reads only the structural skills header (a stable per-object
    shape), so its success here is an enumeration gate, not a frozen value - the actual
    skill levels are re-read live in read_scientists_from_addrs."""
    out = []
    vt = p.module_base + CHAR_VT_RVA
    for a in _enum_vtable(p, vt):
        b = p.try_read(a, _SCI_FP_LEN)
        if len(b) < _SCI_FP_LEN:
            continue
        if struct.unpack_from("<I", b, CHAR_TYPE)[0] != CHAR_TYPE_VAL:
            continue
        if _read_skills(p, a) is None:
            continue   # a non-scientist character (leader, etc.)
        out.append((a, _sci_fingerprint(b)))
    return out


def read_scientists_from_addrs(p, addrs):
    """Decode scientist row dicts LIVE from the supplied (cached) addresses. Produces
    the SAME list read_scientists returns. `addrs` items may be bare int addrs or
    (addr, idfp) tuples (only the addr is used). Re-applies the full per-row gate (type +
    valid skills) so the decode path is identical to the cold reader."""
    out = []
    for item in addrs:
        a, _fp = _split_addr_fp(item)
        if a is None:
            continue
        b = p.try_read(a, 0x10)
        if len(b) < 0x10:
            continue
        if struct.unpack_from("<I", b, CHAR_TYPE)[0] != CHAR_TYPE_VAL:
            continue
        skills = _read_skills(p, a)
        if skills is None:
            continue   # a non-scientist character (leader, etc.)
        out.append({
            "char_id": struct.unpack_from("<I", b, CHAR_ID)[0],
            "name_token": _read_msvc_str(p, a + CHAR_TOKEN_STR),
            "display_name": _read_msvc_str(p, a + CHAR_DISPLAY_STR),
            "skills": skills,
            "trait_count": _read_trait_count(p, a),
            "_addr": a,
        })
    return out


def validate_scientist_addrs(p, addrs):
    """FAIL-CLOSED liveness gate for cached scientist addresses. For EVERY cached addr
    re-read vtable(+0x00)==module_base+CHAR_VT_RVA, type(+0x08)==CHAR_TYPE_VAL, and the
    identity fingerprint (char_id +0x0C, the GLOBAL CCharacter id) and require it to equal
    the cold fingerprint captured by enumerate (TRUE ABA defense). Also re-confirms the
    CScientist skills sub-object is still structurally valid (_read_skills) so a CCharacter
    that mutated out of scientist shape fails closed. A bare-int entry (no cold
    fingerprint), a short read, or any mismatch -> False."""
    vt = p.module_base + CHAR_VT_RVA
    for item in addrs:
        a, cold_fp = _split_addr_fp(item)
        if a is None or cold_fp is None:
            return False        # bad addr / no cold fingerprint -> cannot defend ABA -> fail closed
        b = p.try_read(a, _SCI_FP_LEN)
        if len(b) < _SCI_FP_LEN:
            return False
        if struct.unpack_from("<Q", b, 0)[0] != vt:
            return False
        if struct.unpack_from("<I", b, CHAR_TYPE)[0] != CHAR_TYPE_VAL:
            return False
        if _sci_fingerprint(b) != tuple(cold_fp):
            return False
        if _read_skills(p, a) is None:
            return False        # no longer a valid scientist sub-object -> fail closed
    return True


def read_scientists(p):
    """List of scientist dicts (CCharacters with a valid CScientist skills sub-object)."""
    addrs = enumerate_scientist_addrs(p)
    return read_scientists_from_addrs(p, addrs)


# ---------------------------------------------------------------------------
def _load_save_anchor():
    """Per-domain ground truth from the lockstep save (re_scratch _proj_anchors.json
    if present, else regenerate hint). Returns dict or None."""
    import json
    here = os.path.dirname(os.path.abspath(__file__))
    cand = os.path.join(here, "_proj_anchors.json")
    if os.path.exists(cand):
        return json.load(open(cand))
    return None


def _validate(p):
    """Read live, compare to the save anchor, print a one-screen report. Returns a dict."""
    import json
    save = _load_save_anchor()
    facs = read_facilities(p)
    projs_worked = read_projects(p, active_only=True)
    projs_total = sum(1 for _ in _enum_vtable(p, p.module_base + PROJ_VT_RVA))
    scis = read_scientists(p)

    rep = {
        "live_counts": {
            "facilities": len(facs),
            "projects_total_objects": projs_total,
            "projects_worked": len(projs_worked),
            "scientists": len(scis),
        }
    }

    if save is None:
        rep["validation"] = "no _proj_anchors.json -> live-only (regenerate via the save oracle)"
        return rep

    # --- facilities: join by province ---
    # The anchor stores facilities flat as {TAG: [facility,...]} (see _proj_anchors.json);
    # fall back to the nested countries.<TAG>.facilities layout (save_oracle output).
    save_fac = {}
    fac_src = save.get("facilities")
    if isinstance(fac_src, dict):
        fac_iter = [(tag, f) for tag, lst in fac_src.items() for f in lst]
    else:
        fac_iter = [(tag, f) for tag, c in save.get("countries", {}).items()
                    for f in c.get("facilities", [])]
    for tag, f in fac_iter:
        save_fac[f["province"]] = (tag, f["id"], f.get("building"),
                                   f.get("scientist_id") or 0,
                                   1 if f.get("active_project") else 0,
                                   f.get("unread_prototype_rewards") or 0)
    live_fac = {f["province"]: f for f in facs}
    fok = {"id": 0, "bld": 0, "sci": 0, "act": 0, "unread": 0, "n": 0}
    for prov, (tag, fid, bld, sci, act, unr) in save_fac.items():
        lf = live_fac.get(prov)
        if lf is None:
            continue
        fok["n"] += 1
        fok["id"] += (lf["id"] == fid)
        fok["bld"] += (lf["building"] == bld)
        fok["sci"] += ((lf["scientist_char_id"] or 0) == sci)
        fok["act"] += (int(lf["active"]) == act)
        fok["unread"] += (lf["unread_prototype_rewards"] == unr)
    rep["facilities_vs_save"] = fok

    # --- projects: worked-set count + facility back-link ---
    n_save_worked = sum(1 for _tag, f in fac_iter if f.get("active_project"))
    rep["projects_worked_vs_save"] = {"live": len(projs_worked), "save": n_save_worked,
                                      "match": len(projs_worked) == n_save_worked}

    # --- scientists: skills + name_token + trait count ---
    save_sci = {}
    for tag, lst in save.get("scientists", {}).items():
        for s in lst:
            if s["char_id"] is not None:
                save_sci[s["char_id"]] = (s["name_token"], s["skills"],
                                          len(s.get("traits", [])))
    live_sci = {s["char_id"]: s for s in scis}
    sok = {"skills": 0, "name_semantic": 0, "trait_count": 0, "n": 0, "found": 0}
    for cid, (tok, skills, tc) in save_sci.items():
        ls = live_sci.get(cid)
        if ls is None:
            continue
        sok["found"] += 1
        sok["n"] += 1
        sv = [skills.get(k, 0) for k in SPEC_ORDER]
        lv = [ls["skills"].get(k, 0) for k in SPEC_ORDER]
        sok["skills"] += (sv == lv)
        # name: exact OR save-mojibake-equivalent (dynamic-name scientists)
        same = (ls["name_token"] == tok)
        if not same and ls["name_token"]:
            try:
                same = ls["name_token"].encode("latin-1").decode("utf-8", "replace") == \
                    tok.encode("latin-1").decode("utf-8", "replace")
            except Exception:
                same = False
        sok["name_semantic"] += same
        sok["trait_count"] += (ls.get("trait_count") == tc)
    rep["scientists_vs_save"] = sok
    return rep


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        _verify_build(p)
        import json
        print(json.dumps(_validate(p), indent=1))


if __name__ == "__main__":
    main()
