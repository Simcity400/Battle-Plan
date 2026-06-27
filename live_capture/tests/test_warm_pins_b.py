"""Warm-walk PIN tests for Deliverable B -- the 4 newly-pinned full-scan domains:

  * state_control      (24.1s): _pin_controllers_raw reuses the SHARED state-object roots
                       (state_detail's pinned {state_id: obj_base}) and RE-READS controller
                       @+0xCC LIVE each walk -- the per-state vtable heap scan is skipped on a
                       warm hit. Validate = spread-sentinel vtable + state_id (ABA) + count.
  * construction_orders(24.5s): construction_orders_live.enumerate_armies / validate_armies /
                       read_orders_from_armies + the registry _pin_construction_armies -- the
                       army-pool arena sweep is pinned ONCE; order vectors RE-READ live.
  * decisions_market   (24.3s): decisions_market_live.enumerate_contracts / validate_contracts /
                       read_market_from_objs + _pin_market_contracts -- the whole-heap
                       CPurchaseContract scan is pinned ONCE; contracts RE-DECODED live. The
                       EMPTY (0-contract / peacetime) case is NOT pinned (re-scans, fail-closed).
  * intel_deep         (22.7s): intel_deep_live.enumerate_operatives_intel /
                       validate_operatives_intel / read_operatives_from_idents +
                       _pin_intel_operatives -- the (4713,op_id)+dual-name sweep is pinned ONCE;
                       name/owner/detected RE-READ live each walk.

Each pin is asserted for HIT (warm skips the sweep, output-identical), MISS (validate False ->
full re-scan), INVALIDATE (proc-fingerprint epoch flip clears the pin), and FAIL-CLOSED
no-fabrication (an empty/un-locatable set does NOT pin and never serves stale data). The reader
module enumerate/validate/decode splits are also unit-tested directly against tiny fake procs
so the ABA-defense is proven independent of the registry wiring.

Run:  python -m pytest live_capture/tests/test_warm_pins_b.py -q
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
# 1. state_control: shared-state-root reuse + live controller re-read
# ===========================================================================
class _StateProc:
    """A heap of per-state runtime objects so _pin_controllers_raw / enumerate_state_controllers
    can read vtable@+0, state_id@+0x58, controller@+0xCC. Counts enumerate_state_controllers
    calls so a warm hit (reusing the shared state_detail roots) can be proven to SKIP the scan."""
    module_base = 0x10000
    pid = 4321

    def __init__(self, states):
        # states: {state_id: controller_tag_db_id}
        from live_layers.province_control import (STATE_VTABLE_REL, STATE_OFF_SID,
                                                  STATE_OFF_CTRL)
        self._vt = self.module_base + STATE_VTABLE_REL
        self.mem = {}
        self.obj_of = {}
        a = 0x500000
        for sid, ctrl in states.items():
            self.mem[a + 0x00] = self._vt
            self.mem[a + STATE_OFF_SID] = sid
            self.mem[a + STATE_OFF_CTRL] = ctrl
            self.obj_of[sid] = a
            a += 0x1000

    def u64(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFFFFFFFFFF

    def u32(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFF


class _FakePC:
    """province_control stand-in: enumerate_state_controllers does the (counted) scan."""
    def __init__(self, proc, states):
        self.proc = proc
        self._states = dict(states)
        self.enum_calls = 0

    def enumerate_state_controllers(self, p):
        self.enum_calls += 1
        # mirror the real gate: only emit controllers in [0,200)
        return {sid: c for sid, c in self._states.items() if 0 <= c < 200}


class TestPinStateControl(unittest.TestCase):
    def _ctx(self, proc, *, seed_state_detail=None):
        ctx = reg.CaptureContext(proc=proc, identity=_FpIdentity(),
                                 tag_table=_FakeTagTable({5: "ENG", 9: "GER", 11: "FRA"}),
                                 extras={"pins": {}})
        if seed_state_detail is not None:
            # seed the state_detail pin's roots so _shared_state_objs reuses them (no scan)
            ctx.pins["__fp__"] = ctx.proc_fp()
            ctx.pins["state_detail"] = reg._PinEntry(
                fp=ctx.proc_fp(),
                roots={"states": dict(seed_state_detail), "catname": {}},
                pop=len(seed_state_detail))
        return ctx

    def test_warm_hit_reuses_shared_roots_no_scan(self):
        states = {1: 5, 2: 9, 3: 11}
        proc = _StateProc(states)
        pc = _FakePC(proc, states)
        ctx = self._ctx(proc, seed_state_detail=proc.obj_of)
        out1 = reg._r_state_control(ctx, pc)
        out2 = reg._r_state_control(ctx, pc)
        # with the shared state_detail roots present, the fallback scan is NEVER called.
        self.assertEqual(pc.enum_calls, 0)
        self.assertEqual(out1, out2)                       # output-identical
        self.assertEqual(out1["controllers"],
                         {"1": "ENG", "2": "GER", "3": "FRA"})

    def test_output_identical_to_cold_scan(self):
        # the pinned warm output must equal what the original full scan would emit.
        states = {1: 5, 2: 9, 3: 11}
        proc = _StateProc(states)
        pc = _FakePC(proc, states)
        ctx = self._ctx(proc, seed_state_detail=proc.obj_of)
        pinned = reg._r_state_control(ctx, pc)
        # cold reference: drive the raw scan + the same fold
        raw = pc.enumerate_state_controllers(proc)
        cold = {str(s): ctx.resolve_tag(c) for s, c in raw.items()}
        self.assertEqual(pinned["controllers"], cold)

    def test_live_controller_reread_reflects_change(self):
        # a controller flip while paused MUST be observed (no cached decoded value).
        states = {1: 5, 2: 9}
        proc = _StateProc(states)
        pc = _FakePC(proc, states)
        ctx = self._ctx(proc, seed_state_detail=proc.obj_of)
        first = reg._r_state_control(ctx, pc)
        self.assertEqual(first["controllers"]["1"], "ENG")
        from live_layers.province_control import STATE_OFF_CTRL
        proc.mem[proc.obj_of[1] + STATE_OFF_CTRL] = 9      # state 1 controller ENG -> GER
        second = reg._r_state_control(ctx, pc)
        self.assertEqual(second["controllers"]["1"], "GER")  # re-read live, not stale
        self.assertEqual(pc.enum_calls, 0)                 # still no scan (roots unchanged)

    def test_validate_miss_on_state_obj_moved_rescans(self):
        # corrupting a pinned state's vtable must FAIL the state_control validate (fail-closed)
        # and force a re-resolve -- proven via the pin trace (miss_validate_false -> resolve).
        states = {1: 5, 2: 9}
        proc = _StateProc(states)
        pc = _FakePC(proc, states)
        ctx = self._ctx(proc, seed_state_detail=proc.obj_of)
        reg._r_state_control(ctx, pc)                      # pin under "state_control"
        proc.mem[proc.obj_of[1] + 0x00] = 0xDEAD          # sentinel vtable corrupted
        ctx.extras["pin_trace"] = []
        reg._r_state_control(ctx, pc)
        branches = [b for (k, b) in ctx.extras["pin_trace"] if k == "state_control"]
        self.assertIn("miss_validate_false", branches)     # validate failed (fail-closed)
        self.assertTrue(any(b.startswith("resolve") for b in branches))  # re-resolved
        ctx.extras.pop("pin_trace", None)

    def test_no_shared_roots_falls_back_to_scan(self):
        # WITHOUT a seeded state_detail pin AND no scan cache, _shared_state_objs would scan via
        # StateDetailLive (not available on the fake) -> _pin_controllers_raw falls back to the
        # pc full scan and does NOT pin (roots=None).
        states = {1: 5, 2: 9}
        proc = _StateProc(states)
        pc = _FakePC(proc, states)
        ctx = self._ctx(proc)  # no seed
        out = reg._r_state_control(ctx, pc)
        self.assertEqual(out["controllers"], {"1": "ENG", "2": "GER"})
        self.assertGreaterEqual(pc.enum_calls, 1)          # used the fallback scan
        # not pinned -> the next call scans again (no stale empty served)
        before = pc.enum_calls
        reg._r_state_control(ctx, pc)
        self.assertGreater(pc.enum_calls, before)

    def test_epoch_flip_invalidates(self):
        states = {1: 5, 2: 9}
        proc = _StateProc(states)
        pc = _FakePC(proc, states)
        ctx = self._ctx(proc, seed_state_detail=proc.obj_of)
        reg._r_state_control(ctx, pc)
        ctx.identity = _FpIdentity(pid=99999)              # different process -> epoch flip
        # the whole pin cache (incl. the seeded state_detail) is cleared -> _shared_state_objs
        # finds no roots -> fallback scan.
        reg._r_state_control(ctx, pc)
        self.assertGreaterEqual(pc.enum_calls, 1)


# ===========================================================================
# 2. construction_orders: army-pool enumerate/validate + read_orders_from_armies
# ===========================================================================
class _ArmyProc:
    """A heap of army objects (vtable + handle + an order vector) so enumerate/validate/
    read_orders_from_armies can run over a fake without the real arena scan."""
    module_base = 0x10000
    pid = 4321

    def __init__(self):
        import construction_orders_live as CO
        self.CO = CO
        self.vt = self.module_base + CO.ARMY_VT_RVA
        self.order_vt = self.module_base + CO.ORDER_VT_RVA
        self.mem = {}
        self.blobs = {}
        self.armies = {}      # army_id -> army_obj

    def add_army(self, army_id, orders=()):
        import struct
        CO = self.CO
        # army objects + their order vectors must live in the engine HEAP band so
        # read_orders_for's _heap() gate accepts the order-vector pointers.
        a = CO.HEAP_LO + 0x700000 + army_id * 0x4000
        self.mem[a + 0x00] = self.vt
        self.mem[a + CO.ARMY_HANDLE_OFF] = (army_id << 32) | CO.ARMY_TYPE
        # order vector: a contiguous ptr array at vbegin
        vbegin = a + 0x1000
        self.mem[a + CO.ARMY_ORDERS_VEC_OFF] = vbegin
        for k, (otype, states) in enumerate(orders):
            optr = a + 0x1800 + k * 0x80
            self.mem[vbegin + k * 8] = optr
            self.mem[optr + 0x00] = self.order_vt
            self.mem[optr + CO.ORDER_ARMY_BACKREF_OFF] = a
            self.mem[optr + CO.ORDER_TYPE_OFF] = otype
            if otype == CO.TYPE5_OFFENSIVE and states:
                sbegin = optr + 0x200
                self.mem[optr + CO.ORDER_T5_STATES_BEGIN] = sbegin
                self.mem[optr + CO.ORDER_T5_STATES_SIZE] = len(states)
                self.blobs[sbegin] = b"".join(struct.pack("<i", s) for s in states)
        # terminate the vector after the orders
        self.mem[vbegin + len(orders) * 8] = 0
        self.armies[army_id] = a
        return a

    def u64(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFFFFFFFFFF

    def u32(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFF

    def i32(self, addr):
        import struct
        v = self.u32(addr)
        return struct.unpack("<i", struct.pack("<I", v))[0]

    def try_read(self, addr, n):
        blob = self.blobs.get(addr)
        return bytes(blob[:n]) if isinstance(blob, (bytes, bytearray)) else b""


class TestConstructionArmiesModule(unittest.TestCase):
    def _proc(self):
        import construction_orders_live as CO
        p = _ArmyProc()
        p.add_army(7, orders=[(CO.TYPE5_OFFENSIVE, [101, 102]), (1, None)])
        p.add_army(9, orders=[])           # army with NO orders -> excluded from output
        return p

    def test_enumerate_validate_read_roundtrip(self):
        import construction_orders_live as CO
        p = self._proc()
        armies = {aid: p.armies[aid] for aid in p.armies}  # the pinned set (skip the real scan)
        self.assertTrue(CO.validate_armies(p, armies))
        out = CO.read_orders_from_armies(p, armies)
        self.assertIn(7, out)                              # army 7 has orders
        self.assertNotIn(9, out)                           # army 9 (0 orders) excluded
        t5 = [o for o in out[7] if o["order_type"] == CO.TYPE5_OFFENSIVE]
        self.assertEqual(t5[0]["target_states"], [101, 102])

    def test_validate_miss_on_vtable_change(self):
        import construction_orders_live as CO
        p = self._proc()
        armies = dict(p.armies)
        p.mem[p.armies[7] + 0x00] = 0xDEAD                 # army vtable corrupted
        self.assertFalse(CO.validate_armies(p, armies))

    def test_validate_miss_on_army_id_change(self):
        # ABA: same vtable + type 53 but a different army_id at the handle -> must fail.
        import construction_orders_live as CO
        p = self._proc()
        armies = dict(p.armies)
        p.mem[p.armies[7] + CO.ARMY_HANDLE_OFF] = (999 << 32) | CO.ARMY_TYPE
        self.assertFalse(CO.validate_armies(p, armies))

    def test_validate_empty_fails_closed(self):
        import construction_orders_live as CO
        self.assertFalse(CO.validate_armies(self._proc(), {}))

    def test_order_added_live_is_observed(self):
        # a battle-plan order added on a pinned army while paused MUST be re-read live.
        import construction_orders_live as CO
        p = self._proc()
        armies = dict(p.armies)
        # army 9 had 0 orders; add one in place, re-read -> it now appears.
        a9 = p.armies[9]
        vbegin = p.mem[a9 + CO.ARMY_ORDERS_VEC_OFF]
        optr = a9 + 0x1800
        p.mem[vbegin + 0] = optr
        p.mem[vbegin + 8] = 0
        p.mem[optr + 0x00] = p.order_vt
        p.mem[optr + CO.ORDER_ARMY_BACKREF_OFF] = a9
        p.mem[optr + CO.ORDER_TYPE_OFF] = 1
        out = CO.read_orders_from_armies(p, armies)
        self.assertIn(9, out)                              # observed live (not cached away)


class _FakeCOMod:
    """construction_orders_live stand-in for _pin_construction_armies. Counts the
    enumerate (arena sweep) calls so a warm hit can be proven to skip the sweep."""
    def __init__(self, orders_map, armies=None, valid=True):
        self._orders = orders_map
        self._armies = dict(armies if armies is not None else {7: 0x70, 9: 0x90})
        self.valid = valid
        self.enum_calls = 0
        self.read_calls = 0
        self.validate_calls = 0

    def enumerate_armies(self, p):
        self.enum_calls += 1
        return dict(self._armies)

    def read_orders_from_armies(self, p, armies):
        self.read_calls += 1
        return {k: list(v) for k, v in self._orders.items()}

    def validate_armies(self, p, armies):
        self.validate_calls += 1
        return self.valid


class TestPinConstructionArmies(unittest.TestCase):
    def _ctx(self):
        class _P:
            module_base = 0x10000
            pid = 4321
        return reg.CaptureContext(proc=_P(), identity=_FpIdentity(), extras={"pins": {}})

    def test_warm_hit_skips_sweep_identical(self):
        fake = _FakeCOMod({7: [{"order_type": 5, "target_states": [1]}]})
        ctx = self._ctx()
        out1 = reg._pin_construction_armies(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)               # cold: one sweep
        out2 = reg._pin_construction_armies(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)               # warm: NO new sweep
        self.assertGreaterEqual(fake.read_calls, 2)        # orders RE-READ live each walk
        self.assertEqual(out1, out2)

    def test_warm_miss_rescans(self):
        fake = _FakeCOMod({7: [{"order_type": 5, "target_states": [1]}]})
        ctx = self._ctx()
        reg._pin_construction_armies(ctx, fake)
        fake.valid = False
        reg._pin_construction_armies(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)               # re-swept (fail-closed)

    def test_epoch_flip_invalidates(self):
        fake = _FakeCOMod({7: []})
        ctx = self._ctx()
        reg._pin_construction_armies(ctx, fake)
        ctx.identity = _FpIdentity(pid=222)
        reg._pin_construction_armies(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)

    def test_empty_pool_does_not_pin(self):
        fake = _FakeCOMod({}, armies={})                   # no armies located
        ctx = self._ctx()
        reg._pin_construction_armies(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)
        reg._pin_construction_armies(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)               # not pinned -> re-sweep each call


# ===========================================================================
# 3. decisions_market: contract enumerate/validate + read_market_from_objs
# ===========================================================================
class _ContractProc:
    """A heap of CPurchaseContract objects (vtable@+0, contract_id@+0x0C) so
    enumerate/validate/read can run over a fake."""
    module_base = 0x10000
    pid = 4321

    def __init__(self, contracts):
        import decisions_market_live as DM
        self.DM = DM
        self.vt = self.module_base + DM.PC_VT_RVA
        self.mem = {}
        self.addrs = []
        self.ids = {}
        a = 0x800000
        for cid in contracts:
            self.mem[a + 0x00] = self.vt
            self.mem[a + DM.PC_ID_ID] = cid
            self.addrs.append(a)
            self.ids[a] = cid
            a += 0x1000

    def u64(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFFFFFFFFFF

    def u32(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFF


class TestMarketContractsModule(unittest.TestCase):
    def test_validate_hit(self):
        import decisions_market_live as DM
        p = _ContractProc([10, 20, 30])
        snap = {a: DM.contract_id_at(p, a) for a in p.addrs}
        self.assertTrue(DM.validate_contracts(p, p.addrs, id_snapshot=snap))

    def test_validate_miss_on_id_change(self):
        # ABA: same vtable, but the contract_id at a slot changed -> must fail.
        import decisions_market_live as DM
        p = _ContractProc([10, 20])
        snap = {a: DM.contract_id_at(p, a) for a in p.addrs}
        p.mem[p.addrs[0] + DM.PC_ID_ID] = 999
        self.assertFalse(DM.validate_contracts(p, p.addrs, id_snapshot=snap))

    def test_validate_miss_on_vtable_change(self):
        import decisions_market_live as DM
        p = _ContractProc([10])
        snap = {a: DM.contract_id_at(p, a) for a in p.addrs}
        p.mem[p.addrs[0] + 0x00] = 0xDEAD
        self.assertFalse(DM.validate_contracts(p, p.addrs, id_snapshot=snap))

    def test_validate_empty_fails_closed(self):
        import decisions_market_live as DM
        self.assertFalse(DM.validate_contracts(_ContractProc([]), [], id_snapshot={}))


class _FakeDMMod:
    """decisions_market_live stand-in for _pin_market_contracts. Counts the enumerate
    (whole-heap scan) calls so a warm hit can be proven to skip the scan."""
    def __init__(self, contracts, addrs=None, valid=True):
        self._contracts = list(contracts)
        self._addrs = list(addrs if addrs is not None else [0x10, 0x20])
        self.valid = valid
        self.enum_calls = 0
        self.read_calls = 0
        self.validate_calls = 0

    def enumerate_contracts(self, p):
        self.enum_calls += 1
        return list(self._addrs)

    def contract_id_at(self, p, a):
        return a  # stable per-addr id snapshot for the fake

    def validate_contracts(self, p, addrs, id_snapshot=None):
        self.validate_calls += 1
        return self.valid

    def read_market_from_objs(self, p, addrs, bridge=None):
        self.read_calls += 1
        return [dict(c) for c in self._contracts]


class TestPinMarketContracts(unittest.TestCase):
    def _ctx(self):
        class _P:
            module_base = 0x10000
            pid = 4321
        return reg.CaptureContext(proc=_P(), identity=_FpIdentity(),
                                  bridge=None, extras={"pins": {}})

    def test_warm_hit_skips_scan_redecodes_live(self):
        fake = _FakeDMMod([{"contract_id": 1, "seller": "ENG", "buyer": "GER"}])
        ctx = self._ctx()
        out1 = reg._pin_market_contracts(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)               # cold: one scan
        out2 = reg._pin_market_contracts(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)               # warm: NO re-scan
        self.assertGreaterEqual(fake.read_calls, 2)        # contracts RE-DECODED live each walk
        self.assertEqual(out1, out2)

    def test_warm_miss_rescans(self):
        fake = _FakeDMMod([{"contract_id": 1}])
        ctx = self._ctx()
        reg._pin_market_contracts(ctx, fake)
        fake.valid = False
        reg._pin_market_contracts(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)

    def test_epoch_flip_invalidates(self):
        fake = _FakeDMMod([{"contract_id": 1}])
        ctx = self._ctx()
        reg._pin_market_contracts(ctx, fake)
        ctx.identity = _FpIdentity(pid=333)
        reg._pin_market_contracts(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)

    def test_empty_contracts_not_pinned_rescans(self):
        # PEACETIME: 0 contracts -> roots=None -> NOT pinned -> re-scans every walk (Codex ruling:
        # an empty whole-heap scan cannot be safely pinned). No stale empty served as a hit.
        fake = _FakeDMMod([], addrs=[])
        ctx = self._ctx()
        out1 = reg._pin_market_contracts(ctx, fake)
        self.assertEqual(out1, [])
        self.assertEqual(fake.enum_calls, 1)
        reg._pin_market_contracts(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)               # re-scanned (not pinned)


# ===========================================================================
# 4. intel_deep: operative-identity enumerate/validate + read_from_idents
# ===========================================================================
class _OpProc:
    """A heap of operative IDENTITY objects so enumerate/validate/read_from_idents can run.
    Layout per operative at ident A:
        A+0x00  i32 type tag (== 4713)
        A+0x04  i32 op_id
        A+IDENT_NAME / +IDENT_NAME2  : the dual-name (shadowed via _read_str_utf8 patch)
        A+IDENT_CLASSDESC u64 active-op class descriptor (== module_base+ACTIVE_OP_CLASSDESC_RVA)
        A+IDENT_LIFEFLAG  i32 lifecycle flag (== ACTIVE_OP_LIFEFLAG 47)
        A+OWNER_FROM_IDENT i32 owner tag-db-id
        A+DETECT_FROM_IDENT i32 detection state (==3 -> detected)
    """
    module_base = 0x10000
    pid = 4321

    def __init__(self, ops):
        import intel_deep_live as ID
        self.ID = ID
        self.mem = {}
        self.strs = {}
        self.idents = {}
        active_desc = self.module_base + ID.ACTIVE_OP_CLASSDESC_RVA
        a = 0x10000000000 + 0x500000
        for op_id, (name, owner, detect) in ops.items():
            self.mem[a + 0x00] = 4713
            self.mem[a + 0x04] = op_id
            self.mem[a + ID.IDENT_CLASSDESC] = active_desc      # active-owned class
            self.mem[a + ID.IDENT_LIFEFLAG] = ID.ACTIVE_OP_LIFEFLAG
            self.mem[a + ID.OWNER_FROM_IDENT] = owner
            self.mem[a + ID.DETECT_FROM_IDENT] = detect
            self.strs[a + ID.IDENT_NAME] = name
            self.strs[a + ID.IDENT_NAME2] = name      # dual-name signature (equal)
            self.idents[op_id] = a
            a += 0x2000

    def i32(self, addr):
        import struct
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return struct.unpack("<i", struct.pack("<I", self.mem[addr] & 0xFFFFFFFF))[0]

    def u32(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFF

    def u64(self, addr):
        if addr not in self.mem:
            raise OSError("unmapped 0x%X" % addr)
        return self.mem[addr] & 0xFFFFFFFFFFFFFFFF


class _FakeBridge:
    def __init__(self, m):
        self._m = dict(m)

    def tag(self, tid):
        return self._m.get(tid)

    def as_map(self):
        return dict(self._m)


def _patch_op_str(proc):
    """Patch intel_deep_live._read_str_utf8 to read the _OpProc shadow string table."""
    import intel_deep_live as ID
    orig = ID._read_str_utf8

    def fake(p, so, maxlen=64):
        return getattr(p, "strs", {}).get(so)

    ID._read_str_utf8 = fake
    return orig


def _unpatch_op_str(orig):
    import intel_deep_live as ID
    ID._read_str_utf8 = orig


class TestIntelOperativesModule(unittest.TestCase):
    def setUp(self):
        self._orig = _patch_op_str(None)

    def tearDown(self):
        _unpatch_op_str(self._orig)

    def _proc(self):
        return _OpProc({101: ("philby", 5, 3), 102: ("burgess", 9, 0)})

    def test_validate_and_read_roundtrip(self):
        import intel_deep_live as ID
        p = self._proc()
        ops = {op_id: p.idents[op_id] for op_id in p.idents}
        self.assertTrue(ID.validate_operatives_intel(p, ops))
        br = _FakeBridge({5: "ENG", 9: "GER"})
        rows = ID.read_operatives_from_idents(p, ops, bridge=br)
        self.assertEqual(rows[101]["owner_tag"], "ENG")
        self.assertEqual(rows[101]["detected"], True)
        self.assertEqual(rows[102]["detected"], False)
        self.assertEqual(rows[101]["name"], "philby")

    def test_validate_miss_on_op_id_change(self):
        import intel_deep_live as ID
        p = self._proc()
        ops = dict(p.idents)
        p.mem[p.idents[101] + 0x04] = 999             # ABA: different op_id
        self.assertFalse(ID.validate_operatives_intel(p, ops))

    def test_validate_miss_on_dual_name_break(self):
        # Codex: dual-name re-checked EVERY walk -- a freed+reused type-4713 record that no
        # longer carries the dual-name must FAIL (else it would fabricate a row).
        import intel_deep_live as ID
        p = self._proc()
        ops = dict(p.idents)
        p.strs[p.idents[101] + ID.IDENT_NAME2] = "DIFFERENT"   # break name@+0x18 == codename@+0x38
        self.assertFalse(ID.validate_operatives_intel(p, ops))

    def test_validate_empty_fails_closed(self):
        import intel_deep_live as ID
        self.assertFalse(ID.validate_operatives_intel(self._proc(), {}))

    def test_validate_miss_on_active_class_demotion(self):
        # Codex finding: an operative DEMOTED to the recruitable/AI pool while paused (handle +
        # dual-name unchanged, but classdesc/lifeflag no longer active-owned) must FAIL validate
        # -> force a re-sweep, so the cold scan's active-only set is never stale-served.
        import intel_deep_live as ID
        p = self._proc()
        ops = dict(p.idents)
        p.mem[p.idents[101] + ID.IDENT_LIFEFLAG] = 31      # 31 = recruitable/pool, not 47
        self.assertFalse(ID.validate_operatives_intel(p, ops))

    def test_demoted_operative_dropped_from_warm_output(self):
        # the warm decoder must DROP a no-longer-active operative exactly as read_operatives
        # would (output byte-identical to the cold active-only scan).
        import intel_deep_live as ID
        p = self._proc()
        ops = dict(p.idents)
        br = _FakeBridge({5: "ENG", 9: "GER"})
        full = ID.read_operatives_from_idents(p, ops, bridge=br)
        self.assertIn(101, full)
        p.mem[p.idents[101] + ID.IDENT_CLASSDESC] = 0xBADC0DE  # no longer active-op class
        after = ID.read_operatives_from_idents(p, ops, bridge=br)
        self.assertNotIn(101, after)                      # dropped (not emitted as stale active)
        self.assertIn(102, after)                         # the other operative survives

    def test_limbo_owner_dropped_from_warm_output(self):
        # owner not in the bridge (just-freed/annexed country) -> dropped, matching read_operatives.
        import intel_deep_live as ID
        p = self._proc()
        ops = dict(p.idents)
        br = _FakeBridge({5: "ENG"})                       # 9 (op 102's owner) NOT in the bridge
        out = ID.read_operatives_from_idents(p, ops, bridge=br)
        self.assertIn(101, out)
        self.assertNotIn(102, out)                         # limbo owner -> dropped

    def test_detected_owner_reread_live(self):
        # detection-state + owner are mutable+emitted -> re-read live (no cached row).
        import intel_deep_live as ID
        p = self._proc()
        ops = dict(p.idents)
        br = _FakeBridge({5: "ENG", 9: "GER", 11: "FRA"})
        first = ID.read_operatives_from_idents(p, ops, bridge=br)
        self.assertEqual(first[102]["detected"], False)
        p.mem[p.idents[102] + ID.DETECT_FROM_IDENT] = 3       # becomes detected
        p.mem[p.idents[102] + ID.OWNER_FROM_IDENT] = 11       # owner GER -> FRA
        second = ID.read_operatives_from_idents(p, ops, bridge=br)
        self.assertEqual(second[102]["detected"], True)       # re-read live
        self.assertEqual(second[102]["owner_tag"], "FRA")


class _FakeIDLMod:
    """intel_deep_live stand-in for _pin_intel_operatives. Counts the enumerate (heap sweep)
    calls so a warm hit can be proven to skip the sweep."""
    def __init__(self, roster, ops=None, valid=True):
        self._roster = roster
        self._ops = dict(ops if ops is not None else {101: 0xAA, 102: 0xBB})
        self.valid = valid
        self.enum_calls = 0
        self.read_calls = 0
        self.validate_calls = 0

    def enumerate_operatives_intel(self, p, bridge=None):
        self.enum_calls += 1
        return dict(self._ops)

    def read_operatives_from_idents(self, p, ops, bridge=None):
        self.read_calls += 1
        return {k: dict(v) for k, v in self._roster.items()}

    def validate_operatives_intel(self, p, ops):
        self.validate_calls += 1
        return self.valid


class TestPinIntelOperatives(unittest.TestCase):
    def _ctx(self):
        class _P:
            module_base = 0x10000
            pid = 4321
        return reg.CaptureContext(proc=_P(), identity=_FpIdentity(),
                                  bridge=None, extras={"pins": {}})

    def test_warm_hit_skips_sweep_reread_live(self):
        fake = _FakeIDLMod({101: {"op_id": 101, "name": "philby", "owner_tag": "ENG"}})
        ctx = self._ctx()
        out1 = reg._pin_intel_operatives(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)               # cold: one sweep
        out2 = reg._pin_intel_operatives(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)               # warm: NO sweep
        self.assertGreaterEqual(fake.read_calls, 2)        # name/owner/detected RE-READ each walk
        self.assertEqual(out1, out2)

    def test_warm_miss_rescans(self):
        fake = _FakeIDLMod({101: {"op_id": 101}})
        ctx = self._ctx()
        reg._pin_intel_operatives(ctx, fake)
        fake.valid = False
        reg._pin_intel_operatives(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)

    def test_epoch_flip_invalidates(self):
        fake = _FakeIDLMod({101: {"op_id": 101}})
        ctx = self._ctx()
        reg._pin_intel_operatives(ctx, fake)
        ctx.identity = _FpIdentity(pid=444)
        reg._pin_intel_operatives(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)

    def test_empty_pool_does_not_pin(self):
        fake = _FakeIDLMod({}, ops={})
        ctx = self._ctx()
        reg._pin_intel_operatives(ctx, fake)
        self.assertEqual(fake.enum_calls, 1)
        reg._pin_intel_operatives(ctx, fake)
        self.assertEqual(fake.enum_calls, 2)               # not pinned -> re-sweep each call


if __name__ == "__main__":
    unittest.main()
