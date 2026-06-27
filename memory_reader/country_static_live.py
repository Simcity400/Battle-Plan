"""country_static_live.py - LIVE per-country STATIC scalars (read-only).

Reads, straight from a running hoi4.exe, per-country scalar fields pinned on
build 16022851 (cracked + validated read-only vs the content-matched
USA_1936_02_07_06 melt; see re_scratch/_track1_offsets.draft.json):

    original_capital          country_obj + 0x101C  i32   (state id; 0 => none)
    original_tag              country_obj + 0x130C  u16   tag-db-id (0 => None)
    reserved_dynamic_country  country_obj + 0x145D  u8    (1 => True; active dynamic country)
    templates_locked          country_obj + 0x01B4  u8    (1 => True)

The sibling `capital` lives at +0x1018 (i32). In a fresh 1936 start capital ==
original_capital for every country, so they are value-identical here; +0x101C is
the higher of the adjacent pair, matching the save serialization order
(capital then original_capital). `radar` at +0x1148 is a self-pointing intel
handle struct (country==self, pool=3, id=1, disc=0) carrying NO per-country
coverage scalar, so it is NOT exposed as a data leaf (exposing it would be a
fabricated value).

Country index -> TAG is bridged by the country object's OWN tag-db-id at
obj+0x08 (u32) via the live tag table -- more robust than the unique-PP bridge
(which resolves only the ~40 countries with non-zero political_power). The 3
active dynamic SPR-spawns carry runtime ids beyond the static table and are
identified by (original_tag != 0 AND reserved == 1).

READ-ONLY: PROCESS_VM_READ only; 2-read sentinel on every scalar. BUILD-GATED.
"""

import os
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))

import winmem  # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

REC = 0x120
OFF_NAMEPTR = 0x08

# --- pinned country-object-relative offsets, build 16022851 ---
OFF_TAGDBID = 0x08          # u32 country's own tag-db-id
OFF_TEMPLATES_LOCKED = 0x01B4   # u8 (1 => yes)
OFF_ORIG_CAPITAL = 0x101C   # i32 state id
OFF_CAPITAL = 0x1018        # i32 sibling
OFF_ORIG_TAG = 0x130C       # u16 tag-db-id (0 => None)
OFF_RESERVED = 0x145D       # u8 (1 => yes)


def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA:
        return
    msg = ("country_static_live: exe SHA-256 %s != mapped build %s; offsets are "
           "build-specific." % (got, EXPECTED_SHA))
    if force:
        return
    raise SystemExit(msg + " (pass --force to override)")


from country_misc_live import _sentinel  # shared 2-read sentinel (read twice, agree or None)


def read_country_static(p, obj, tag_table=None):
    """Return the static-scalar dict for one country object.

    ``tag_table`` (optional): a live TagTable-like object exposing as_map()
    {tag_db_id: TAG}; used to resolve original_tag's tag-db-id -> TAG. Without
    it, original_tag is returned as a raw id (original_tag_id) only.
    """
    out = {}
    oc = _sentinel(lambda: p.i32(obj + OFF_ORIG_CAPITAL))
    out["original_capital"] = oc if (oc is not None and oc > 0) else None
    tl = _sentinel(lambda: p.u8(obj + OFF_TEMPLATES_LOCKED))
    out["templates_locked"] = None if tl is None else (tl == 1)
    rv = _sentinel(lambda: p.u8(obj + OFF_RESERVED))
    out["reserved_dynamic_country"] = None if rv is None else (rv == 1)
    otid = _sentinel(lambda: p.u16(obj + OFF_ORIG_TAG))
    out["original_tag"] = None
    if otid and tag_table is not None:
        try:
            t = tag_table.as_map().get(int(otid))
        except Exception:
            t = None
        out["original_tag"] = t if (t and t != "---") else None
    elif otid:
        # no tag table to resolve the id -> expose the raw id so a caller can resolve it
        out["original_tag_id"] = otid
    return out


def enumerate_country_static(p, rec0, count, idx_tag, tag_table=None):
    """Yield (tag, index, static-dict) for every bridged country (idx_tag:
    {running_index: TAG})."""
    for i in range(count):
        tag = idx_tag.get(i)
        if not tag:
            continue
        try:
            obj = p.u64(rec0 + i * REC + OFF_NAMEPTR)
        except OSError:
            continue
        yield tag, i, read_country_static(p, obj, tag_table=tag_table)


def bridge_via_tag_table(p, rec0, count, tag_table):
    """Build {running_index: TAG} from the country object's own tag-db-id at
    obj+0x08, resolved through the live tag table. Robust full-coverage bridge
    (no political_power dependency). Returns {} if the tag table is None."""
    if tag_table is None:
        return {}
    try:
        tmap = tag_table.as_map()
    except Exception:
        return {}
    idx_tag = {}
    for i in range(count):
        try:
            obj = p.u64(rec0 + i * REC + OFF_NAMEPTR)
            tid = p.u32(obj + OFF_TAGDBID)
        except OSError:
            continue
        tag = tmap.get(int(tid))
        if tag and tag != "---":
            idx_tag[i] = tag
    return idx_tag
