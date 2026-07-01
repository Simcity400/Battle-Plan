"""Warm-walk PIN tests for the three residual closures (the "true seconds" pass):

  * advisor_roster  (politics_residuals 52s): leadership_deep_live.enumerate_advisor_addrs /
                     read_advisor_roster_from_addrs / validate_advisor_addrs + the registry
                     _pin_advisor_roster -- the CAdvisor pool is LOCATED + DECODED once and
                     the decoded roster served on warm hit (no full-heap sweep).
  * commanders      (#22 22s): _pin_commanders now caches the DECODED enumeration; a warm hit
                     returns it without re-decoding every leader body (+ the trait flat_map
                     payload fingerprint for designs_variants parity).
  * oob_command     (#22): _pin_oob_command now caches the DECODED command tree; a warm hit
                     returns it without re-running the per-division membership loop.

Each pin is asserted for HIT (warm skips the sweep/decode, output-identical), MISS
(validate False -> full re-scan + re-decode), INVALIDATE (proc-fingerprint epoch flip
clears the pin), and FAIL-CLOSED no-fabrication (an empty/un-locatable pool does NOT pin
and never serves stale data). The leadership_deep_live module functions are also unit-
tested directly against a tiny fake process so the ABA-defense (char_ptr/char_id/tag/slot/
idea_token fingerprint) is proven independent of the registry wiring.

Run:  python -m pytest live_capture/tests/test_warm_pins.py -q
"""
from __future__ import annotations

import os
import sys
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)
_MR = os.path.join(_REPO, "memory_reader")
if _MR not in sys.path:
    sys.path.insert(0, _MR)

from live_capture import registry as reg  # noqa: E402
import leadership_deep_live as LD  # noqa: E402


# ---------------------------------------------------------------------------
# shared fakes
# ---------------------------------------------------------------------------
class _FpIdentity:
    """Minimal ProcessIdentity for ctx.proc_fp() (the pin epoch key)."""
    def __init__(self, pid=4321, ct=111, sha="sha-A"):
        self.pid = pid
        self.create_time_100ns = ct
        self.exe_sha256 = sha


class _FakeTagTable:
    def __init__(self, m):
        self._m = dict(m)

    def as_map(self):
        return dict(self._m)

    def tag(self, tid):
        return self._m.get(tid)


# ===========================================================================
# 1. leadership_deep_live: the advisor-pool LOCATE / DECODE / VALIDATE split
# ===========================================================================
class _AdvProc:
    """Tiny CAdvisor-pool fake: a heap of {addr: qword} + std::string SSO blobs, with the
    module_base and the CAdvisor vtable laid out so enumerate/validate/decode all run.

    Layout per advisor at base A:
        A+0x00  vtable (== module_base + VT_ADVISOR_RVA)
        A+0x10  -> CCharacter addr CH
        A+0x30  std::string idea_token (SSO)
        A+0x80  std::string slot       (SSO)
    Per character at CH:
        CH+0x08 type (== 73)
        CH+0x0C char_id
        CH+0x68 country tag_db_id
    """
    module_base = 0x10000
    VT = module_base + LD.VT_ADVISOR_RVA

    def __init__(self):
        self.mem = {}
        self.regions = []      # list of (base, bytes) for _find_instances' iter_regions
        self.pid = 7
        # allocate within the engine heap window so _char_id_at's heap_ok() accepts the bodies.
        self._next = LD.HEAP_LO + 0x200000

    def _alloc(self, size=0x100):
        a = self._next
        self._next += max(size, 0x100)
        return a

    def add_advisor(self, char_id, tag_db_id, token, slot):
        ch = self._alloc()
        self.mem[ch + 0x08] = 73                 # CHAR_TYPE
        self.mem[ch + 0x0C] = char_id
        self.mem[ch + 0x68] = tag_db_id
        a = self._alloc()
        self.mem[a + 0x00] = self.VT
        self.mem[a + LD.ADV_CHAR] = ch
        self._put_str(a + LD.ADV_TOKEN, token)
        self._put_str(a + LD.ADV_SLOT, slot)
        # publish the vtable qword into a scan region so _find_instances locates `a`
        self.regions.append((a, self.VT.to_bytes(8, "little")))
        return a, ch

    def _put_str(self, addr, s):
        # MSVC std::string SSO: [buf(16)][size(8)][cap(8)]; cap<16 => inline buffer.
        raw = s.encode()
        self.mem[("str", addr)] = s
        # store size + a small cap so locate.read_msvc_str's SSO path triggers; we
        # intercept the read via _read below using the ("str", addr) shadow.

    # ---- winmem.Process surface used by the readers ----
    def u64(self, a):
        if a not in self.mem:
            raise OSError("unmapped 0x%X" % a)
        return self.mem[a] & 0xFFFFFFFFFFFFFFFF

    def u32(self, a):
        if a not in self.mem:
            raise OSError("unmapped 0x%X" % a)
        return self.mem[a] & 0xFFFFFFFF

    def try_read(self, a, n):
        # used only by _find_instances over self.regions
        for base, blob in self.regions:
            if base <= a < base + len(blob):
                off = a - base
                return blob[off:off + n]
        return b""

    def read(self, a, n):
        d = self.try_read(a, n)
        if len(d) < n:
            raise OSError("short read")
        return d

    def iter_regions(self, lo, hi, want_exec=False):
        for base, blob in self.regions:
            yield (base, len(blob))


def _patch_str(monkey_targets):
    """Patch locate.read_msvc_str + POL._su64/_su32 + _char_id_at to read the _AdvProc
    shadow so the pure-python fake needs no real SSO byte layout."""
    import locate
    import politics_live as POL
    orig = {}

    def fake_read_msvc_str(p, addr, maxlen=80):
        return getattr(p, "mem", {}).get(("str", addr))

    def fake_su64(p, addr):
        try:
            return p.u64(addr)
        except OSError:
            return 0

    orig["locate"] = locate.read_msvc_str
    orig["su64"] = POL._su64
    locate.read_msvc_str = fake_read_msvc_str
    POL._su64 = fake_su64
    return orig


def _unpatch_str(orig):
    import locate
    import politics_live as POL
    locate.read_msvc_str = orig["locate"]
    POL._su64 = orig["su64"]


class TestAdvisorRosterModule(unittest.TestCase):
    def setUp(self):
        self._orig = _patch_str(None)

    def tearDown(self):
        _unpatch_str(self._orig)

    def _proc(self):
        p = _AdvProc()
        p.add_advisor(101, 5, "philip_kerr", "political_advisor")
        p.add_advisor(102, 5, "maxwell_aitken", "political_advisor")
        p.add_advisor(103, 9, "guderian_theorist", "theorist")
        return p

    def test_enumerate_decode_roundtrip(self):
        p = self._proc()
        tagmap = {5: "ENG", 9: "GER"}
        addrs, snap = LD.enumerate_advisor_addrs(p)
        self.assertEqual(len(addrs), 3)
        self.assertEqual(len(snap), 3)
        roster = LD.read_advisor_roster_from_addrs(p, addrs, tagmap)
        self.assertEqual(sorted(roster), ["ENG", "GER"])
        self.assertEqual({r["idea_token"] for r in roster["ENG"]},
                         {"philip_kerr", "maxwell_aitken"})
        self.assertEqual(roster["GER"][0]["char_id"], 103)
        # the public read_advisor_roster (locate+decode) must match the from_addrs path
        full = LD.read_advisor_roster(p, tagmap=tagmap)
        self.assertEqual(full, roster)

    def test_validate_hit(self):
        p = self._proc()
        addrs, snap = LD.enumerate_advisor_addrs(p)
        self.assertTrue(LD.validate_advisor_addrs(p, addrs, snap))

    def test_validate_miss_on_idea_token_change(self):
        # ABA: same addr + same char, but the idea_token changed -> the DECODED row would
        # differ, so validate MUST fail (char_ptr+char_id alone would have passed).
        p = self._proc()
        addrs, snap = LD.enumerate_advisor_addrs(p)
        # mutate the first advisor's token in the shadow
        a0 = addrs[0]
        p.mem[("str", a0 + LD.ADV_TOKEN)] = "DIFFERENT_token"
        self.assertFalse(LD.validate_advisor_addrs(p, addrs, snap))

    def test_validate_miss_on_country_change(self):
        # the TAG grouping depends on CCharacter+0x68; a tag_db_id change must invalidate.
        p = self._proc()
        addrs, snap = LD.enumerate_advisor_addrs(p)
        a0 = addrs[0]
        ch0 = p.mem[a0 + LD.ADV_CHAR]
        p.mem[ch0 + 0x68] = 999                  # different country
        self.assertFalse(LD.validate_advisor_addrs(p, addrs, snap))

    def test_validate_miss_on_vtable_change(self):
        p = self._proc()
        addrs, snap = LD.enumerate_advisor_addrs(p)
        p.mem[addrs[0] + 0x00] = 0xDEAD          # not the CAdvisor vtable
        self.assertFalse(LD.validate_advisor_addrs(p, addrs, snap))

    def test_validate_empty_fails_closed(self):
        # an empty addr set is NOT a valid located frame (no vacuous pass).
        p = self._proc()
        self.assertFalse(LD.validate_advisor_addrs(p, [], {}))


# ===========================================================================
# 2. registry _pin_advisor_roster (the wired pin)
# ===========================================================================
class _FakeLD:
    """Stand-in for the leadership_deep_live module's advisor-pool API used by
    _pin_advisor_roster. Counts the enumerate (sweep) calls so a test can prove a warm hit
    SKIPS the sweep."""
    def __init__(self, roster, valid=True, addrs=None):
        self._roster = roster
        self.valid = valid
        self._addrs = list(addrs if addrs is not None else [0x10, 0x20])
        self.enum_calls = 0
        self.validate_calls = 0
        self.decode_calls = 0

    def enumerate_advisor_addrs(self, p):
        self.enum_calls += 1
        snap = {a: (a, a, 5, "tok%d" % a, "slot") for a in self._addrs}
        return list(self._addrs), snap

    def read_advisor_roster_from_addrs(self, p, addrs, tagmap):
        self.decode_calls += 1
        return {t: list(rows) for t, rows in self._roster.items()}

    def validate_advisor_addrs(self, p, addrs, snapshot):
        self.validate_calls += 1
        return self.valid


class TestPinAdvisorRoster(unittest.TestCase):
    def _ctx(self, **kw):
        class _P:
            module_base = 0x10000
            pid = 4321
        return reg.CaptureContext(proc=_P(), identity=_FpIdentity(),
                                  tag_table=_FakeTagTable({5: "ENG"}),
                                  extras={"pins": {}}, **kw)

    def _run(self, ctx, fake):
        import leadership_deep_live as _ld
        saved = {k: getattr(_ld, k, None) for k in
                 ("enumerate_advisor_addrs", "read_advisor_roster_from_addrs",
                  "validate_advisor_addrs")}
        _ld.enumerate_advisor_addrs = fake.enumerate_advisor_addrs
        _ld.read_advisor_roster_from_addrs = fake.read_advisor_roster_from_addrs
        _ld.validate_advisor_addrs = fake.validate_advisor_addrs
        try:
            return reg._pin_advisor_roster(ctx)
        finally:
            for k, v in saved.items():
                setattr(_ld, k, v)

    def test_warm_hit_skips_sweep_and_is_identical(self):
        fake = _FakeLD({"ENG": [{"char_id": 1, "slot": "s", "idea_token": "t"}]})
        ctx = self._ctx()
        out1 = self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)         # cold: one sweep
        self.assertEqual(fake.decode_calls, 1)       # cold: one decode
        out2 = self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)         # warm: NO new sweep
        self.assertEqual(fake.decode_calls, 1)       # warm: NO re-decode (cached decoded roster)
        self.assertGreaterEqual(fake.validate_calls, 1)
        self.assertEqual(out1, out2)                 # output-identical

    def test_warm_miss_rescans_and_redecodes(self):
        fake = _FakeLD({"ENG": [{"char_id": 1, "slot": "s", "idea_token": "t"}]})
        ctx = self._ctx()
        self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)
        fake.valid = False                            # the pinned pool no longer validates
        self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)          # full re-sweep (fail-closed)
        self.assertEqual(fake.decode_calls, 2)        # re-decode

    def test_epoch_flip_invalidates_pin(self):
        fake = _FakeLD({"ENG": [{"char_id": 1, "slot": "s", "idea_token": "t"}]})
        ctx = self._ctx()
        self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)
        ctx.identity = _FpIdentity(pid=9999)          # different process -> epoch flip
        self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)          # whole pin cache cleared -> re-sweep

    def test_no_tagtable_returns_empty_no_pin(self):
        # no naming -> honest absence ({}), and NO sweep is even attempted (caller self-resolves).
        fake = _FakeLD({"ENG": []})
        ctx = self._ctx()
        ctx.tag_table = None
        out = self._run(ctx, fake)
        self.assertEqual(out, {})
        self.assertEqual(fake.enum_calls, 0)

    def test_empty_pool_does_not_pin(self):
        # enumerate locates nothing -> resolve returns roots=None -> not pinned; the next
        # call re-sweeps (no stale empty served as a pinned hit).
        class _EmptyLD(_FakeLD):
            def enumerate_advisor_addrs(self, p):
                self.enum_calls += 1
                return [], {}
        fake = _EmptyLD({})
        ctx = self._ctx()
        self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)
        self._run(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)          # not pinned -> re-sweep each call


# ===========================================================================
# 3. _pin_commanders: cached DECODED enumeration (the #22 22s)
# ===========================================================================
class _FakeCmdReaderModule:
    """A CharactersLiveReader CLASS stand-in whose instances expose the pinned API; the
    factory counts enumerate (sweep) AND decode calls so a warm hit can be proven to skip
    BOTH."""
    last = None

    class _Reader:
        def __init__(self, parent):
            self.parent = parent

        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

        def close(self):
            pass

        def verify_build(self):
            return True, "sha"

        # pinned API
        def enumerate_addrs(self):
            self.parent.enum_calls += 1
            return {"army": [0xA], "navy": [], "char": [1],
                    "country_leader": [], "_ids": {0xA: 7, 1: 7},
                    "_traits": {0xA: (0x500, 2)}}

        def enumerate_from_addrs(self, addr_sets, resolve_country=True, read_traits=True):
            self.parent.decode_calls += 1
            # the decoded leader carries a MUTABLE inline scalar (experience). It is read
            # off the parent LIVE at decode time so a warm RE-DECODE reflects the current
            # value -- but a cached decode (no re-read) would serve the stale snapshot.
            return {"army_leaders": [{"char_id": 7, "country": "GER", "addr": "0xA",
                                      "experience": self.parent.experience}],
                    "navy_leaders": [], "characters": {7: {"country": "GER"}},
                    "counts": {"army_leaders": 1, "navy_leaders": 0, "characters": 1}}

        def validate_addrs(self, addr_sets):
            self.parent.validate_calls += 1
            return self.parent.valid

    def __init__(self, valid=True):
        self.valid = valid
        self.enum_calls = 0
        self.decode_calls = 0
        self.validate_calls = 0
        # a mutable inline leader scalar (experience). The body IDENTITY (vtable/type/id)
        # and the trait fingerprint are unchanged when this drifts -- exactly the case a
        # decoded-cache pin must NOT serve stale (a commander gaining XP on an unpaused tick).
        self.experience = 1.0

    def __call__(self):
        return _FakeCmdReaderModule._Reader(self)


class TestTraitContentFingerprint(unittest.TestCase):
    """Codex-B caveat: the per-leader trait fingerprint must catch an IN-PLACE, SAME-SIZE trait
    edit (a trait assigned while PAUSED) -- a (begin,size)-only fp would miss it. We drive
    CharactersLiveReader._trait_block_fp + validate_addrs over a tiny in-memory body with no game."""

    def _reader(self, mem):
        import characters_live as CL
        r = CL.CharactersLiveReader.__new__(CL.CharactersLiveReader)
        r.module_base = 0x10000
        r.vt = {k: r.module_base + rva for k, rva in CL.VT_RVA.items()}
        r._mem = dict(mem)

        def _read(addr, n):
            return r._mem.get(addr, b"")[:n] if isinstance(r._mem.get(addr), (bytes, bytearray)) \
                else b"".join(r._mem.get((addr + i,), b"\x00") for i in range(n))

        # simpler: a flat byte map keyed by exact (addr) -> bytes blob
        def _read_flat(addr, n):
            blob = r._mem.get(addr)
            if isinstance(blob, (bytes, bytearray)):
                return bytes(blob[:n])
            return b""
        r._read = _read_flat
        return r, CL

    def test_trait_content_hash_changes_on_inplace_edit(self):
        import struct as _s
        CL_begin = LD.HEAP_LO + 0x5000
        leader = LD.HEAP_LO + 0x1000
        # trait header at leader+OFF_TRAITS_BLOCK0: begin@+0x10, size@+0x18
        import characters_live as CL
        hdr = bytearray(0x20)
        _s.pack_into("<Q", hdr, CL.TRAIT_BLOCK_BEGIN, CL_begin)
        _s.pack_into("<I", hdr, CL.TRAIT_BLOCK_SIZE, 1)               # 1 trait record
        rec_a = _s.pack("<Q", 5) + _s.pack("<q", 100)                # trait_id 5, xp 100
        rec_b = _s.pack("<Q", 7) + _s.pack("<q", 100)                # SAME SIZE, different trait_id
        mem = {leader + CL.OFF_TRAITS_BLOCK0: bytes(hdr), CL_begin: rec_a}
        r, _ = self._reader(mem)
        fp1 = r._trait_block_fp(leader)
        self.assertEqual(fp1[0], CL_begin)
        self.assertEqual(fp1[1], 1)
        self.assertIsNotNone(fp1[2])                                 # a content hash exists
        # mutate the trait IN PLACE (same begin, same size) -> the content hash MUST differ
        r._mem[CL_begin] = rec_b
        fp2 = r._trait_block_fp(leader)
        self.assertEqual(fp2[:2], fp1[:2])                           # begin+size identical
        self.assertNotEqual(fp2[2], fp1[2])                          # content hash CHANGED -> caught

    def test_validate_addrs_fails_on_trait_content_change(self):
        import struct as _s
        import characters_live as CL
        CL_begin = LD.HEAP_LO + 0x5000
        leader = LD.HEAP_LO + 0x1000
        # build a CArmyLeader body that passes the vtable+type+id checks too.
        body = bytearray(0x1000)
        _s.pack_into("<Q", body, CL.OFF_VT, 0x10000 + CL.VT_RVA["CArmyLeader"])
        _s.pack_into("<I", body, CL.OFF_TYPE, CL.UNIT_LEADER_TYPE)
        _s.pack_into("<I", body, CL.OFF_ID, 42)
        hdr = bytearray(0x20)
        _s.pack_into("<Q", hdr, CL.TRAIT_BLOCK_BEGIN, CL_begin)
        _s.pack_into("<I", hdr, CL.TRAIT_BLOCK_SIZE, 1)
        rec_a = _s.pack("<Q", 5) + _s.pack("<q", 100)
        mem = {leader: bytes(body),
               leader + CL.OFF_TRAITS_BLOCK0: bytes(hdr),
               CL_begin: rec_a}
        r = CL.CharactersLiveReader.__new__(CL.CharactersLiveReader)
        r.module_base = 0x10000
        r.vt = {k: r.module_base + rva for k, rva in CL.VT_RVA.items()}
        r._mem = dict(mem)
        r._read = lambda a, n: bytes(r._mem.get(a, b"")[:n]) if isinstance(r._mem.get(a), (bytes, bytearray)) else b""
        r.u64 = lambda a: _s.unpack_from("<Q", r._mem.get(leader, b"\x00" * 0x1000), a - leader)[0] if a >= leader and a < leader + 0x1000 else 0
        r.u32 = lambda a: _s.unpack_from("<I", r._mem.get(leader, b"\x00" * 0x1000), a - leader)[0] if a >= leader and a < leader + 0x1000 else 0
        addr_sets = {"army": [leader], "navy": [], "char": [], "country_leader": [],
                     "_ids": {leader: 42},
                     "_traits": {leader: r._trait_block_fp(leader)}}
        self.assertTrue(r.validate_addrs(addr_sets))                 # unchanged -> valid
        r._mem[CL_begin] = _s.pack("<Q", 9) + _s.pack("<q", 100)     # in-place same-size trait edit
        self.assertFalse(r.validate_addrs(addr_sets))               # content hash mismatch -> FAIL


class TestPinCommandersDecodedCache(unittest.TestCase):
    def _ctx(self):
        class _P:
            module_base = 0x10000
            pid = 4321
        return reg.CaptureContext(proc=_P(), identity=_FpIdentity(),
                                  extras={"pins": {}})

    def _patch_verify(self):
        # _pin_commanders calls _verify_same_process(ctx, reader); stub it to a no-op.
        self._sv = reg._verify_same_process
        reg._verify_same_process = lambda ctx, reader: None

    def _unpatch_verify(self):
        reg._verify_same_process = self._sv

    def test_warm_hit_skips_sweep_but_rereads_live(self):
        # A warm hit MUST skip the expensive four-arena heap SWEEP (enum_calls stays 1) but
        # MUST re-read the leader fields LIVE (decode each walk) -- it is an ADDRESS-ONLY pin,
        # NOT a cached-decode pin. Caching the decoded enumeration and serving it on a warm hit
        # would return stale mutable scalars (experience/skills) that the body-identity validate
        # does NOT cover (Codex-confirmed stale-data risk).
        self._patch_verify()
        try:
            fake = _FakeCmdReaderModule()
            ctx = self._ctx()
            out1 = reg._pin_commanders(ctx, fake)
            self.assertEqual(fake.enum_calls, 1)     # cold: one sweep
            self.assertEqual(fake.decode_calls, 1)   # cold: one decode
            out2 = reg._pin_commanders(ctx, fake)
            self.assertEqual(fake.enum_calls, 1)     # warm: NO sweep (addresses pinned)
            self.assertEqual(fake.decode_calls, 2)   # warm: RE-DECODE live (no stale cache)
            self.assertEqual(out1, out2)             # output-identical when nothing drifted
        finally:
            self._unpatch_verify()

    def test_warm_hit_reflects_mutated_inline_scalar(self):
        # THE no-fabrication / no-stale-data invariant: an inline leader scalar (experience)
        # drifts between the pinning walk and a warm walk while the body IDENTITY and trait
        # fingerprint are unchanged (validate stays True). The warm read MUST reflect the NEW
        # value -- a cached decode would serve the stale OLD value as "live".
        self._patch_verify()
        try:
            fake = _FakeCmdReaderModule()
            ctx = self._ctx()
            fake.experience = 1.0
            out1 = reg._pin_commanders(ctx, fake)
            self.assertEqual(out1["army_leaders"][0]["experience"], 1.0)
            fake.experience = 9.0                    # commander gains XP (body identity same)
            out2 = reg._pin_commanders(ctx, fake)    # validate True -> warm path
            self.assertEqual(out2["army_leaders"][0]["experience"], 9.0)  # LIVE, not stale
        finally:
            self._unpatch_verify()

    def test_warm_miss_rescans_and_redecodes(self):
        self._patch_verify()
        try:
            fake = _FakeCmdReaderModule()
            ctx = self._ctx()
            reg._pin_commanders(ctx, fake)
            self.assertEqual(fake.decode_calls, 1)
            fake.valid = False
            reg._pin_commanders(ctx, fake)
            self.assertEqual(fake.enum_calls, 2)     # re-sweep
            self.assertEqual(fake.decode_calls, 2)   # re-decode
        finally:
            self._unpatch_verify()

    def test_epoch_flip_invalidates(self):
        self._patch_verify()
        try:
            fake = _FakeCmdReaderModule()
            ctx = self._ctx()
            reg._pin_commanders(ctx, fake)
            ctx.identity = _FpIdentity(pid=12345)
            reg._pin_commanders(ctx, fake)
            self.assertEqual(fake.enum_calls, 2)
            self.assertEqual(fake.decode_calls, 2)
        finally:
            self._unpatch_verify()


# ===========================================================================
# 4. _pin_oob_command: cached DECODED tree (the #22 per-division loop)
# ===========================================================================
class _OobProc:
    """Tiny proc exposing u64/u32/try_read over a {addr:val} map so _oob_struct_fp (the Codex-C
    edge fingerprint) can read the mutable inline division/army/tf fields + the ship vector."""
    module_base = 0x10000
    pid = 4321

    def __init__(self, mem, blobs=None):
        self.mem = dict(mem)
        self.blobs = dict(blobs or {})     # addr -> bytes (for try_read, e.g. ship vector)

    def u64(self, a):
        if a not in self.mem:
            raise OSError("unmapped 0x%X" % a)
        return self.mem[a] & 0xFFFFFFFFFFFFFFFF

    def u32(self, a):
        if a not in self.mem:
            raise OSError("unmapped 0x%X" % a)
        return self.mem[a] & 0xFFFFFFFF

    def try_read(self, a, n):
        blob = self.blobs.get(a)
        return bytes(blob[:n]) if isinstance(blob, (bytes, bytearray)) else b""


class _FakeOobMod:
    # surface the offset constants _oob_struct_fp reads off the module.
    import oob_command_live as _real
    DIV_ARMY_PARENT = _real.DIV_ARMY_PARENT
    DIV_OWNER = _real.DIV_OWNER
    ARMY_AG_PARENT = _real.ARMY_AG_PARENT
    ARMY_LEADER_PTR = _real.ARMY_LEADER_PTR
    ARMY_MEMBER_COUNT = _real.ARMY_MEMBER_COUNT
    TF_FLEET_PARENT = _real.TF_FLEET_PARENT
    TF_SHIP_BEGIN = _real.TF_SHIP_BEGIN
    TF_SHIP_COUNT = _real.TF_SHIP_COUNT

    def __init__(self, tree, valid=True):
        self._tree = tree
        self.valid = valid
        self.enum_calls = 0
        self.decode_calls = 0
        self.validate_calls = 0

    def enumerate_command_objs(self, p):
        self.enum_calls += 1
        return {"_arena": (0x1, 0x2), "army": [(0x10, (1, 53))], "ag": [], "fleet": [],
                "tf": [], "theatre": [], "div": [(0x40, (9, 11))], "ship": []}

    def read_command_tree_from_objs(self, p, objs):
        self.decode_calls += 1
        return {k: v for k, v in self._tree.items()}

    def validate_command_objs(self, p, objs):
        self.validate_calls += 1
        return self.valid


class TestPinOobDecodedCache(unittest.TestCase):
    def _mem(self):
        # the inline edges _oob_struct_fp reads for the single army(0x10) + div(0x40).
        return {
            0x40 + _FakeOobMod.DIV_ARMY_PARENT: 0x10,   # div assigned to army 0x10
            0x40 + _FakeOobMod.DIV_OWNER: 53,
            0x10 + _FakeOobMod.ARMY_AG_PARENT: 0,
            0x10 + _FakeOobMod.ARMY_LEADER_PTR: 0,
            0x10 + _FakeOobMod.ARMY_MEMBER_COUNT: 1,
        }

    def _ctx(self):
        return reg.CaptureContext(proc=_OobProc(self._mem()), identity=_FpIdentity(),
                                  extras={"pins": {}})

    def _tree(self):
        return {"armies": {1: {"id": 1, "member_div_ids": [9], "owner_tag_db_id": 53}},
                "army_groups": {}, "fleets": {}, "task_forces": {}, "theatres": {},
                "_meta": {"counts": {"armies": 1}}}

    def test_warm_hit_skips_decode(self):
        fake = _FakeOobMod(self._tree())
        ctx = self._ctx()
        out1 = reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)
        self.assertEqual(fake.decode_calls, 1)
        out2 = reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)         # warm: NO re-locate
        self.assertEqual(fake.decode_calls, 1)       # warm: NO re-decode (the #22 loop is cached)
        self.assertEqual(out1, out2)

    def test_warm_miss_rescans_and_redecodes(self):
        fake = _FakeOobMod(self._tree())
        ctx = self._ctx()
        reg._pin_oob_command(ctx, fake)
        fake.valid = False
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)
        self.assertEqual(fake.decode_calls, 2)

    def test_epoch_flip_invalidates(self):
        fake = _FakeOobMod(self._tree())
        ctx = self._ctx()
        reg._pin_oob_command(ctx, fake)
        ctx.identity = _FpIdentity(pid=222)
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)
        self.assertEqual(fake.decode_calls, 2)

    def test_empty_locate_does_not_pin(self):
        class _EmptyOob(_FakeOobMod):
            def enumerate_command_objs(self, p):
                self.enum_calls += 1
                return {"_arena": (0x1, 0x2), "army": [], "ag": [], "fleet": [],
                        "tf": [], "theatre": [], "div": [], "ship": []}
        fake = _EmptyOob(self._tree())
        ctx = self._ctx()
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)         # not pinned -> re-locate

    def test_division_reassignment_invalidates_cache(self):
        # Codex C MUST-FIX: the division object identity (vtable+handle) is UNCHANGED, but its
        # army-parent ptr (div+0xB0) changed (the user reassigned it to another army while paused).
        # validate_command_objs would still pass -> the struct fp MUST catch it and force a re-decode,
        # so the stale cached membership is NEVER served.
        fake = _FakeOobMod(self._tree())          # validate_command_objs stays True (identity ok)
        ctx = self._ctx()
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.decode_calls, 1)
        ctx.proc.mem[0x40 + _FakeOobMod.DIV_ARMY_PARENT] = 0x99   # reassign: same obj, new edge
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.decode_calls, 2)    # struct-fp mismatch -> re-decode (fail-closed)

    def test_division_owner_change_invalidates_cache(self):
        # owner-consensus depends on div+DIV_OWNER; a change with object identity intact must
        # invalidate the cached tree too.
        fake = _FakeOobMod(self._tree())
        ctx = self._ctx()
        reg._pin_oob_command(ctx, fake)
        ctx.proc.mem[0x40 + _FakeOobMod.DIV_OWNER] = 77
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.decode_calls, 2)

    def test_struct_fp_stable_warm_hit_still_skips(self):
        # control: with NO edge change, the warm walk still HITS (no re-decode) -- the struct fp
        # must not spuriously invalidate a genuinely-unchanged frame.
        fake = _FakeOobMod(self._tree())
        ctx = self._ctx()
        reg._pin_oob_command(ctx, fake)
        reg._pin_oob_command(ctx, fake)
        self.assertEqual(fake.decode_calls, 1)    # warm hit: cached decode served


class TestOobShipVectorFingerprint(unittest.TestCase):
    """Codex-C caveat: the TF ship-vector fingerprint must hash the element handles, not just
    (begin,count) -- a SAME-COUNT ship swap while paused would otherwise serve a stale TF->ship set.
    Drives reg._oob_struct_fp directly with a tf carrying a 2-ship vector."""
    import struct as _s

    def _objs(self, tf_addr=0x80):
        return {"_arena": (0x1, 0x2), "army": [], "ag": [], "fleet": [],
                "tf": [(tf_addr, (3, 61))], "theatre": [], "div": [], "ship": []}

    def _proc(self, tf_addr, ship_begin, ships):
        import oob_command_live as ocl
        blob = b"".join(self._s.pack("<Q", s) for s in ships)
        mem = {
            tf_addr + ocl.TF_FLEET_PARENT: 0,
            tf_addr + ocl.TF_SHIP_BEGIN: ship_begin,
            tf_addr + ocl.TF_SHIP_COUNT: len(ships),
        }
        return _OobProc(mem, blobs={ship_begin: blob})

    def test_same_count_ship_swap_changes_fp(self):
        import oob_command_live as ocl
        tf_addr, ship_begin = 0x80, LD.HEAP_LO + 0x9000
        objs = self._objs(tf_addr)
        p1 = self._proc(tf_addr, ship_begin, [111, 222])
        fp1 = reg._oob_struct_fp(p1, ocl, objs)
        # SAME count (2 ships), but one element swapped -> the content hash MUST differ.
        p2 = self._proc(tf_addr, ship_begin, [111, 999])
        fp2 = reg._oob_struct_fp(p2, ocl, objs)
        self.assertIsNotNone(fp1)
        self.assertNotEqual(fp1, fp2)

    def test_identical_ship_vector_same_fp(self):
        import oob_command_live as ocl
        tf_addr, ship_begin = 0x80, LD.HEAP_LO + 0x9000
        objs = self._objs(tf_addr)
        p1 = self._proc(tf_addr, ship_begin, [111, 222])
        p2 = self._proc(tf_addr, ship_begin, [111, 222])
        self.assertEqual(reg._oob_struct_fp(p1, ocl, objs),
                         reg._oob_struct_fp(p2, ocl, objs))


if __name__ == "__main__":
    unittest.main()
