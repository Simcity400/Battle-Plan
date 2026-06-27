"""mio_live.py - LIVE per-organisation MIO (Military Industrial Organisation) reader.

STATUS on build 16022851 (read-only, save-free): RESOLVED + validated. Every MIO in
the running game is a pool-allocated POD record (~0x220 bytes). There is no base
pointer and no id-indexed array (the pool interleaves orgs of different countries,
like the CDivision / CProvince pools), so the global set is enumerated by reverse-
scanning committed heap for the record's stable vtable pointer (MODULE+0x27AF0C0) at
record+0x1B8, keeping hits whose +0x00==79 (the MIO type) and +0x04 in 1..1252 (id).

PINNED + validated EXACT vs the time-aligned melted save (parse_designs.py), all 1210
orgs, 0 missing:
  id (+0x04)                     globally-unique org id
  owner (+0x118)                 country TAG-DB-ID (GER=1, ENG=2, SOV=3, ...)
  research_bonus (+0x98)         i32 x100000
  funds (+0x120)                 i32 x100000  (development currency)
  level / size (+0x128)          i32          (org development tier)
  points (+0x12C)                i32          (progress to next level)
  unlocked_traits count (+0x140) u32          (# traits TAKEN)
  allowed_policies count (+0x174)u32          (# policies ELIGIBLE)

RESISTED / residual (see mio_offsets.draft.json):
  task_capacity   - 0 for every org in this save -> no value to anchor (use the save
                    oracle, or anchor from a save where tasks are assigned).
  assigned_equipment, trait NAMES, policy NAMES, name/icon - reached via record sub-
                    pointers (deeper deref); counts are pinned, contents are a follow-up.
                    All available from the monthly melted-save oracle meanwhile.

owner is a tag-db-id (same id space as divisions/states), so an org joins to a TAG via
tag_table.py with no save needed.

READ-ONLY: PROCESS_VM_READ | PROCESS_QUERY_INFORMATION only; ReadProcessMemory +
VirtualQueryEx; no writes, no injection, no thread suspension. fail-closed on a build
mismatch (offsets are build-specific).
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import winmem  # noqa: E402

EXPECTED_SHA256 = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# --- solved record layout (build 16022851; see mio_offsets.draft.json) ---
VT_RVA = 0x27AF0C0          # MODULE-relative stable record pointer (enumeration anchor)
VT_OFF = 0x1B8             # the anchor pointer sits at record + 0x1B8
MIO_TYPE = 79             # +0x00 type constant
MAX_ID = 1252            # org ids are dense 1..~1252 this session

OFF_TYPE = 0x00
OFF_ID = 0x04
OFF_RB = 0x98
OFF_OWNER = 0x118
OFF_FUNDS = 0x120
OFF_LEVEL = 0x128
OFF_POINTS = 0x12C
OFF_TRAITS = 0x140
OFF_POLICIES = 0x174

FP = 100000.0

# Alternative anchors that ALSO uniquely recover all orgs (robustness fallback if a
# patch shifts the preferred RVA): (record_offset, module_rva).
ALT_ANCHORS = [(0x1F0, 0x2936E48), (0x1E8, 0x33119A0), (0x1D0, 0x305B1A0)]


def _verify_build(p):
    return p.fingerprint().get("sha256") == EXPECTED_SHA256


def _read_record(p, base):
    """Read one MIO record at `base` (the {type,id} handle). Returns a dict or None
    if it does not pass the type+id identity filter."""
    try:
        if p.u32(base + OFF_TYPE) != MIO_TYPE:
            return None
        oid = p.u32(base + OFF_ID)
        if not (1 <= oid <= MAX_ID):
            return None
        return {
            "id": oid,
            "owner_tag_db_id": p.u32(base + OFF_OWNER),
            "level": p.i32(base + OFF_LEVEL),
            "points": p.i32(base + OFF_POINTS),
            "funds": p.i32(base + OFF_FUNDS) / FP,
            "research_bonus": p.i32(base + OFF_RB) / FP,
            "unlocked_traits_count": p.u32(base + OFF_TRAITS),
            "allowed_policies_count": p.u32(base + OFF_POLICIES),
            "addr": base,
        }
    except OSError:
        return None


def enumerate_mios(p, vt_rva=VT_RVA, vt_off=VT_OFF):
    """Enumerate every live MIO record -> { org_id: record_dict }.

    Scans committed non-executable heap for the record's stable vtable pointer
    (MODULE+vt_rva) and accepts hits whose record passes the type(79)+id filter.
    Dedups by org id (the live pool holds one record per id). fail-closed: returns {}
    on a build mismatch.
    """
    if not _verify_build(p):
        return {}
    needle = (p.module_base + vt_rva).to_bytes(8, "little")
    out = {}
    CH = 32 << 20
    OV = 16
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
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
                    s = i + 8
                    rec_base = base + pos + i - vt_off
                    rec = _read_record(p, rec_base)
                    if rec is not None:
                        out.setdefault(rec["id"], rec)
                blen = len(blob)
                if pos + blen >= size or blen < want:   # final / short read
                    pos += max(blen, 0x1000)
                else:
                    pos += blen - OV                     # full mid-region chunk
            else:
                pos += max(want, 0x1000)
    return out


def read_mios_from_addrs(p, addrs, vt_rva=VT_RVA, vt_off=VT_OFF):
    """STEADY-STATE no-scan re-read: given a PINNED {org_id: record_base} map (from a prior
    enumerate_mios scan), re-read each record LIVE WITHOUT scanning the heap. Each record is
    re-gated by _read_record (vtable's record passes the type(79)+id filter) and we additionally
    confirm the re-read record's id == the pinned key (ABA-reuse defense: a freed slot re-used by
    another MIO would have a DIFFERENT id). A record that fails the gate OR whose id no longer
    matches its key is DROPPED (caller's pin-validate should already have forced a re-scan, but
    this is belt-and-braces: a dropped record is honest absence, never fabrication). Returns the
    same { org_id: record } shape as enumerate_mios."""
    if not _verify_build(p):
        return {}
    needle_vt = p.module_base + vt_rva
    out = {}
    for oid, base in addrs.items():
        # confirm the object STILL carries the MIO record vtable at +VT_OFF (the same anchor
        # enumerate_mios matched on); a moved/freed slot fails here.
        if p.u64(base + vt_off) != needle_vt:
            continue
        rec = _read_record(p, base)
        if rec is None or rec.get("id") != oid:      # type/id gate + ABA id-match
            continue
        out[oid] = rec
    return out


def enumerate_by_tag(p, bridge_or_tag_table=None):
    """Same data grouped by TAG. Resolves owner_tag_db_id -> TAG via the optional
    tag_table / country-index bridge (anything exposing .tag(tag_db_id)). When no
    resolver is given, groups by the raw tag-db-id integer key.

    Returns { TAG_or_tagid: [record, ...] } sorted by id within each group.
    """
    mios = enumerate_mios(p)
    by = {}
    for rec in mios.values():
        tid = rec["owner_tag_db_id"]
        key = tid
        if bridge_or_tag_table is not None:
            try:
                t = bridge_or_tag_table.tag(tid)
                if t:
                    key = t
            except Exception:
                pass
        by.setdefault(key, []).append(rec)
    for k in by:
        by[k].sort(key=lambda r: r["id"])
    return by


def main():
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        if not _verify_build(p):
            print("mio: BUILD MISMATCH - refusing to read (offsets are build-specific).")
            raise SystemExit(1)
        print("build OK; module_base=0x%X" % p.module_base)

        mios = enumerate_mios(p)
        print("LIVE MIOs enumerated: %d distinct org ids" % len(mios))

        # group by owner tag-db-id (optionally resolve TAGs if the tag table is present)
        resolver = None
        try:
            sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "live_layers"))
            import tag_table  # noqa: E402
            resolver = tag_table.TagTable.locate(p)
        except Exception:
            resolver = None

        by = enumerate_by_tag(p, resolver)
        print("countries with MIOs: %d" % len(by))

        # show the most-developed orgs
        allrecs = sorted(mios.values(), key=lambda r: (r["level"], r["funds"]), reverse=True)
        print("\ntop 12 developed organisations:")
        print("  %-5s %-6s %-5s %-12s %-7s %-6s %-6s" %
              ("id", "owner", "level", "funds", "points", "traits", "pol"))
        for r in allrecs[:12]:
            owner = r["owner_tag_db_id"]
            if resolver is not None:
                try:
                    owner = resolver.tag(r["owner_tag_db_id"]) or owner
                except Exception:
                    pass
            print("  %-5d %-6s %-5d %-12.5f %-7d %-6d %-6d" %
                  (r["id"], str(owner), r["level"], r["funds"], r["points"],
                   r["unlocked_traits_count"], r["allowed_policies_count"]))


if __name__ == "__main__":
    main()
