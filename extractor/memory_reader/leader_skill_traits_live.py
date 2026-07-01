"""leader_skill_traits_live.py - READ-ONLY live reader for the two remaining
characters-domain fields: the unit-leader OVERALL SKILL/level and the assigned-
TRAIT MEMBERSHIP set. Completes the characters domain.

Both are read LIVE from a running hoi4.exe via ReadProcessMemory only (passive ->
no MP desync, any cadence; keep the game PAUSED). The leader pool is located the
same way characters_live.py does (RTTI-resolved CArmyLeader / CNavyLeader vtables);
this module reuses CharactersLiveReader for pool enumeration and the build gate, and
defnames_live for trait-def -> token resolution. See leader_skill_traits_offsets.draft.json.

PINNED (both 100% vs the lockstep melted save, all 953 military leaders; Codex-concurred):

  OVERALL SKILL (save `skill`, the displayed leader level):
      skill = *(u32*)( *(u64*)(leader + 0xE60) + 0x1B8 )
    leader+0xE60 -> the leader XP/skill sub-object; +0x1B8 within it = the stored
    level. STORED (one hop off the body), NOT derived from XP thresholds nor from the
    4 category skills (proven by leaders whose overall skill != max/min/avg of the
    cats, e.g. ulid 742 skill=8 with a/d/p/l 7/7/6/6). Ghidra getter FUN_140c10290.
    -> CORRECTS the earlier character_offsets.draft.json 'level RESISTED/derived'.

  TRAIT MEMBERSHIP SET (save `traits={ tok ... }`):
      {CUnitLeaderTrait** data @ leader+0xDC8, u32 capacity @+0xDD0, u32 SIZE @+0xDD4}
    iterate data[0..SIZE); each element is a CUnitLeaderTrait* def pointer resolved to
    a token via the defnames_live CUnitLeaderTrait registry. SIZE (not capacity) is the
    live count; tail slots [size,capacity) are stale - do NOT scan to first null. The
    leader hash fn FUN_140c076b0 iterates exactly data@+0xDC8 .. +count@+0xDD4,
    corroborating the layout. DISTINCT from the in_progress modifier-coefficient bag
    (the flat_map at leader+0x288) and from the auxiliary sibling containers at
    +0xDE0/+0xDF8/+0xE10 (plausibly timed/temporary traits; not the persistent set).

HARD RULES honored (same as characters_live.py / defnames_live.py):
  * READ-ONLY memory (PROCESS_VM_READ | PROCESS_QUERY_INFORMATION); no writes/inject/
    suspend/breakpoints. Passive reads, game kept PAUSED.
  * Build-gated: refuses unless the live hoi4.exe sha256 matches the pinned build.
  * Every read is region-clamped by CharactersLiveReader._read (a read straddling a
    region boundary returns short rather than reading garbage).
  * ASCII-only output. Does not edit characters_live.py / defnames_live.py (imports
    and calls their public surface).

Public API
----------
    LeaderSkillTraitsReader(proc_name="hoi4.exe")
      .verify_build() -> (ok, sha256)
      .read_skill(leader_addr)             -> int | None
      .read_trait_set(leader_addr, reg=None, def_addr_tok=None) -> [token,...] | None
      .read_leader(leader_addr, kind, reg=None, def_addr_tok=None) -> dict | None
      .enumerate(read_skill=True, read_traits=True) -> {"leaders":[...], "counts":{...}}

  Convenience module functions (open the process for you):
    read_all(proc_name="hoi4.exe") -> {"leaders":[{unit_leader_id, kind, skill, traits[...]}], "counts":{}}
    skill_for(unit_leader_id)      -> int | None
    traits_for(unit_leader_id)     -> [token,...] | None

Usage
-----
    python leader_skill_traits_live.py            # dump counts + samples
    python leader_skill_traits_live.py --validate --gt _leader_st_gt.json
"""

import argparse
import json
import os
import struct
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))

import characters_live as CL          # leader pool + body layout + build gate
import defnames_live as DN            # CUnitLeaderTrait registry / token resolution
import winmem                          # for the def-addr->token enumeration helper

EXPECTED_SHA = CL.EXPECTED_SHA256

# --- offsets (build 16022851; see leader_skill_traits_offsets.draft.json) ---
OFF_SKILL_SUBOBJ_PTR = 0xE60     # u64 -> leader XP/skill sub-object
OFF_SKILL_IN_SUBOBJ  = 0x1B8     # u32 overall level inside that sub-object

OFF_TRAIT_DATA = 0xDC8           # u64 -> CUnitLeaderTrait* array
OFF_TRAIT_CAP  = 0xDD0           # u32 capacity
OFF_TRAIT_SIZE = 0xDD4           # u32 live count (authoritative; iterate data[0..size))

MAX_TRAITS = 64
SANE_SKILL_MAX = 15

HEAP_LO = CL.HEAP_LO
HEAP_HI = CL.HEAP_HI


def _heap(v):
    return HEAP_LO <= v < HEAP_HI


class LeaderSkillTraitsReader(object):
    """Read-only handle: overall skill + trait membership for unit leaders."""

    def __init__(self, proc_name="hoi4.exe"):
        # CharactersLiveReader gives us the build gate, the RTTI-located leader
        # vtables, the region-clamped _read, and find_instances - reused as-is.
        self._cl = CL.CharactersLiveReader(proc_name)
        self.module_base = self._cl.module_base
        # defnames_live needs a winmem Process for its registry/scan helpers.
        self._p = winmem.open_process(proc_name)
        if self._p is None:
            raise RuntimeError("winmem could not open %s" % proc_name)

    # ---- build gate ----
    def verify_build(self):
        return self._cl.verify_build()

    # ---- trait-def address -> token map (def pointers stored in the leader array) ----
    def build_trait_defaddr_map(self):
        """{def-object addr -> token} for every CUnitLeaderTrait def object. The leader
        trait array stores these def ADDRESSES directly, so we resolve by address."""
        out = {}
        vt = self._p.module_base + DN.UL_TRAIT_VT_RVA
        for a in DN._enum_vtable(self._p, vt):
            tok = DN._read_msvc_str(self._p, a + DN.UL_TRAIT_TOKEN_OFF)
            if tok:
                out[a] = tok
        return out

    # ---- per-leader: overall skill ----
    def read_skill(self, leader_addr):
        """*(u32*)(*(u64*)(leader+0xE60)+0x1B8). Returns int level or None."""
        hdr = self._cl._read(leader_addr + OFF_SKILL_SUBOBJ_PTR, 8)
        if len(hdr) < 8:
            return None
        sub = struct.unpack_from("<Q", hdr, 0)[0]
        if not _heap(sub):
            return None
        b = self._cl._read(sub + OFF_SKILL_IN_SUBOBJ, 4)
        if len(b) < 4:
            return None
        val = struct.unpack_from("<I", b, 0)[0]
        # domain guard: leader level is small (observed 1..9; cap headroom to SANE_SKILL_MAX).
        # A value outside this range means the +0xE60 subobject ptr was stale/misaligned.
        if val > SANE_SKILL_MAX:
            return None
        return val

    # ---- per-leader: assigned-trait membership set ----
    def read_trait_set(self, leader_addr, def_addr_tok=None):
        """Resolved assigned-trait tokens in save order, or None on a hard read failure.
        Returns [] for a genuinely trait-less leader (size==0)."""
        if def_addr_tok is None:
            def_addr_tok = self.build_trait_defaddr_map()
        hdr = self._cl._read(leader_addr + OFF_TRAIT_DATA, 0x10)
        if len(hdr) < 0x10:
            return None
        data = struct.unpack_from("<Q", hdr, 0)[0]
        cap = struct.unpack_from("<I", hdr, 8)[0]
        size = struct.unpack_from("<I", hdr, 0xC)[0]
        if size == 0:
            return []                    # no traits
        if size > MAX_TRAITS or size > cap or not _heap(data):
            return None
        raw = self._cl._read(data, size * 8)
        if len(raw) < size * 8:
            return None                  # short read near a region edge -> fail closed
        out = []
        for k in range(size):
            e = struct.unpack_from("<Q", raw, k * 8)[0]
            tok = def_addr_tok.get(e)
            if tok is None:
                return None              # element is not a clean def pointer -> fail closed
            out.append(tok)
        return out

    def read_leader(self, leader_addr, kind, def_addr_tok=None):
        """{unit_leader_id, kind, skill, traits[...]} for one CArmyLeader/CNavyLeader."""
        b = self._cl._read(leader_addr, 0x10)
        if len(b) < 0x10 or struct.unpack_from("<I", b, 0x08)[0] != CL.UNIT_LEADER_TYPE:
            return None
        ulid = struct.unpack_from("<I", b, 0x0C)[0]
        return {
            "unit_leader_id": ulid,
            "kind": kind,
            "skill": self.read_skill(leader_addr),
            "traits": self.read_trait_set(leader_addr, def_addr_tok),
        }

    # ---- whole-pool enumeration ----
    def enumerate(self, read_skill=True, read_traits=True):
        def_addr_tok = self.build_trait_defaddr_map() if read_traits else None
        leaders = []
        for vt, kind in ((self._cl.vt["CArmyLeader"], "army"),
                         (self._cl.vt["CNavyLeader"], "navy")):
            for a in self._cl.find_instances(vt):
                bb = self._cl._read(a, 0x10)
                if len(bb) < 0x10 or struct.unpack_from("<I", bb, 0x08)[0] != CL.UNIT_LEADER_TYPE:
                    continue
                rec = {
                    "unit_leader_id": struct.unpack_from("<I", bb, 0x0C)[0],
                    "kind": kind,
                    "addr": "0x%X" % a,
                    "skill": self.read_skill(a) if read_skill else None,
                    "traits": self.read_trait_set(a, def_addr_tok) if read_traits else None,
                }
                leaders.append(rec)
        return {
            "leaders": leaders,
            "counts": {
                "army": sum(1 for r in leaders if r["kind"] == "army"),
                "navy": sum(1 for r in leaders if r["kind"] == "navy"),
                "total": len(leaders),
            },
        }


# --------------------------------------------------------------------------------------
# convenience module-level API
# --------------------------------------------------------------------------------------
def read_all(proc_name="hoi4.exe"):
    r = LeaderSkillTraitsReader(proc_name)
    ok, _ = r.verify_build()
    if not ok:
        raise SystemExit("build gate: live exe sha256 != pinned build")
    return r.enumerate()


def skill_for(unit_leader_id, proc_name="hoi4.exe"):
    for rec in read_all(proc_name)["leaders"]:
        if rec["unit_leader_id"] == unit_leader_id:
            return rec["skill"]
    return None


def traits_for(unit_leader_id, proc_name="hoi4.exe"):
    for rec in read_all(proc_name)["leaders"]:
        if rec["unit_leader_id"] == unit_leader_id:
            return rec["traits"]
    return None


# --------------------------------------------------------------------------------------
# validation vs the lockstep save GT
# --------------------------------------------------------------------------------------
def _validate(reader, gt_path):
    """Validate live skill + trait-set vs the save GT (_leader_st_gt.json:
    {ulid -> {skill, traits[], ...}}). Returns a one-screen report dict."""
    gt = json.load(open(gt_path))
    out = reader.enumerate()
    by_ul = {r["unit_leader_id"]: r for r in out["leaders"]}

    skill_ok = skill_n = 0
    set_ok = set_n = 0
    size_ok = order_ok = 0
    empty_ok = empty_n = 0
    skill_miss = []
    set_miss = []
    for ul_s, g in gt.items():
        ul = int(ul_s)
        live = by_ul.get(ul)
        if live is None:
            continue
        # skill
        if g.get("skill") is not None and live.get("skill") is not None:
            skill_n += 1
            if live["skill"] == g["skill"]:
                skill_ok += 1
            elif len(skill_miss) < 15:
                skill_miss.append((ul, live["skill"], g["skill"]))
        # trait set
        want = list(g.get("traits") or [])
        got = live.get("traits")
        if got is not None:
            set_n += 1
            if set(got) == set(want):
                set_ok += 1
                if len(got) == len(want):
                    size_ok += 1
                if got == want:
                    order_ok += 1
                if not want:
                    empty_n += 1
                    empty_ok += 1
            else:
                if not want:
                    empty_n += 1
                if len(set_miss) < 15:
                    set_miss.append((ul, len(want), len(got),
                                     len(set(got) & set(want))))

    return {
        "counts_live": out["counts"],
        "skill": {
            "live_eq_save": "%d/%d (%.1f%%)" % (skill_ok, skill_n,
                                                100.0 * skill_ok / max(1, skill_n)),
            "getter": "*(u32*)(*(u64*)(leader+0xE60)+0x1B8)  [Ghidra FUN_140c10290]",
            "misses": skill_miss,
        },
        "trait_set": {
            "set_live_eq_save": "%d/%d (%.1f%%)" % (set_ok, set_n,
                                                    100.0 * set_ok / max(1, set_n)),
            "size_live_eq_save": "%d/%d" % (size_ok, set_n),
            "order_live_eq_save": "%d/%d" % (order_ok, set_n),
            "empty_trait_leaders_correct": "%d/%d" % (empty_ok, empty_n),
            "container": "{data* @ leader+0xDC8, cap u32 @+0xDD0, size u32 @+0xDD4}; "
                         "elements = CUnitLeaderTrait* -> token via defnames_live",
            "misses": set_miss,
        },
    }


def main():
    ap = argparse.ArgumentParser(description="Live the game leader skill + trait-set reader (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe sha256 doesn't match the pinned build")
    ap.add_argument("--validate", action="store_true",
                    help="validate live skill + trait-set vs the save GT")
    ap.add_argument("--gt", default=os.path.join(_THIS, "_leader_st_gt.json"),
                    help="path to the per-leader GT (_leader_st_gt.json)")
    args = ap.parse_args()

    r = LeaderSkillTraitsReader()
    ok, digest = r.verify_build()
    if not ok and not args.force:
        raise SystemExit("build gate: live exe sha256 %s != pinned %s (pass --force)"
                         % (digest, EXPECTED_SHA))
    print("module_base 0x%X sha %s%s" % (r.module_base, digest[:16],
                                         "" if ok else "  (MISMATCH - --force)"))

    if args.validate:
        if not os.path.exists(args.gt):
            raise SystemExit("GT not found: %s (run _leader_st_gt.py first)" % args.gt)
        print(json.dumps(_validate(r, args.gt), indent=1))
        return

    out = r.enumerate()
    c = out["counts"]
    print("live leaders: army=%d navy=%d total=%d" % (c["army"], c["navy"], c["total"]))
    print("\nsample leaders (skill + first traits):")
    for rec in out["leaders"][:8]:
        tr = rec.get("traits") or []
        print("   ul_id=%-6d %-4s skill=%-3s ntraits=%-2d %s"
              % (rec["unit_leader_id"], rec["kind"], rec["skill"], len(tr),
                 " ".join(tr[:5]) + (" ..." if len(tr) > 5 else "")))


if __name__ == "__main__":
    main()
