"""Offline unit tests for the per-side per-equipment damage std::map walk
(NavalCombatStats.read_side_damage_map).

No live process is needed: a FakeReader serves a hand-built MSVC std::map<int,u64>
_Tree from a sparse byte store, exercising the in-order tree walk, the element-count
upper-bound cross-check, and the cycle/off-heap/torn-node fail-closed guards. The node
layout (_Left/_Parent/_Right @+0x00/+0x08/+0x10, _Isnil @+0x19, pair<const int,u64>
_Myval key @+0x20 / value @+0x28) is the same MSVC layout already validated by the
MIO-trait map walk (designs_mio_deep_live.py).

read_side_damage_map returns a STRUCTURED result {"entries": [...], "status": ...} where
status is one of complete / empty / partial / unavailable. The honest cases return
complete/empty; every adversarial corruption (cycle, torn node field, bogus sentinel,
lying count high/low, out-of-heap child) returns a NOT-complete status and NEVER a
fabricated / None-keyed row.
"""

from __future__ import annotations

import struct
import unittest

from memory_reader.combat.naval_combat_stats import (
    NavalCombatStats,
    OFF_SIDE_DMGMAP_HEAD,
    OFF_SIDE_DMGMAP_SIZE,
    NODE_LEFT,
    NODE_PARENT,
    NODE_RIGHT,
    NODE_ISNIL,
    NODE_DMGMAP_KEY,
    NODE_DMGMAP_VALUE,
    NODE_SPAN,
    DMGMAP_COMPLETE,
    DMGMAP_EMPTY,
    DMGMAP_PARTIAL,
    DMGMAP_UNAVAILABLE,
)

# combat_enum heap window = [HEAP_LO, HEAP_HI) = [0x10000000000, 0x7FF000000000).
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# Addresses within the heap window.
SIDE = 0x20000000000
HEAD = 0x20000001000
NODE_A = 0x20000002000  # key 10 (root)
NODE_B = 0x20000003000  # key 5  (left leaf)
NODE_C = 0x20000004000  # key 20 (right leaf)


class FakeReader:
    """Minimal CombatReader stand-in: a sparse {address: byte} store plus the
    u32/u64/_rpm typed reads NavalCombatStats uses. Unmapped bytes read as None
    (an off-heap / torn read), matching the real reader's fail-soft contract."""

    def __init__(self) -> None:
        self.mem: dict[int, int] = {}
        self.ship_vt = 0  # unused by the damage-map walk

    def _rpm(self, addr, n):
        out = bytearray()
        for i in range(n):
            b = self.mem.get(addr + i)
            if b is None:
                return None
            out.append(b)
        return bytes(out)

    def u32(self, addr):
        b = self._rpm(addr, 4)
        return None if b is None else struct.unpack("<I", b)[0]

    def u64(self, addr):
        b = self._rpm(addr, 8)
        return None if b is None else struct.unpack("<Q", b)[0]

    # --- test builders ---
    def write(self, addr, data: bytes) -> None:
        for i, b in enumerate(data):
            self.mem[addr + i] = b

    def wu64(self, addr, value) -> None:
        self.write(addr, struct.pack("<Q", value & 0xFFFFFFFFFFFFFFFF))

    def wu32(self, addr, value) -> None:
        self.write(addr, struct.pack("<I", value & 0xFFFFFFFF))

    def wu8(self, addr, value) -> None:
        self.mem[addr] = value & 0xFF

    def write_node(self, node, left, parent, right, is_nil, key=0, value=0) -> None:
        self.wu64(node + NODE_LEFT, left)
        self.wu64(node + NODE_PARENT, parent)
        self.wu64(node + NODE_RIGHT, right)
        self.wu8(node + NODE_ISNIL, 1 if is_nil else 0)
        self.wu32(node + NODE_DMGMAP_KEY, key & 0xFFFFFFFF)
        self.wu64(node + NODE_DMGMAP_VALUE, value)

    def clear_span(self, node) -> None:
        """Unmap the whole node span so every field read there returns None
        (a torn / unreadable node)."""
        for off in range(NODE_SPAN):
            self.mem.pop(node + off, None)


def build_three_node_tree(r: FakeReader, size=3) -> None:
    """Build a 3-node std::map tree: root A(key 10), left leaf B(key 5), right
    leaf C(key 20). HEAD sentinel: _Left=leftmost(B), _Parent=root(A),
    _Right=rightmost(C), _Isnil=1. Leaf children point at HEAD (nil), exactly
    like MSVC std::map."""
    r.write_node(HEAD, left=NODE_B, parent=NODE_A, right=NODE_C, is_nil=True)
    r.write_node(NODE_A, left=NODE_B, parent=HEAD, right=NODE_C, is_nil=False,
                 key=10, value=200)
    r.write_node(NODE_B, left=HEAD, parent=NODE_A, right=HEAD, is_nil=False,
                 key=5, value=100)
    r.write_node(NODE_C, left=HEAD, parent=NODE_A, right=HEAD, is_nil=False,
                 key=20, value=300)
    r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
    r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, size)


def build_empty_tree(r: FakeReader, size=0) -> None:
    """A genuinely empty MSVC std::map: only the HEAD sentinel exists and its
    _Parent (root) points back at HEAD (nil). count defaults to 0."""
    r.write_node(HEAD, left=HEAD, parent=HEAD, right=HEAD, is_nil=True)
    r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
    r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, size)


THREE = [
    {"equip_key": 5, "damage": 100},
    {"equip_key": 10, "damage": 200},
    {"equip_key": 20, "damage": 300},
]


class DamageMapWalkTests(unittest.TestCase):
    def _stats(self, r):
        return NavalCombatStats(r)

    def _walk(self, r):
        return self._stats(r).read_side_damage_map(SIDE)

    # ----------------------------- honest cases ------------------------------
    def test_inorder_walk_yields_ascending_key_value_pairs(self):
        r = FakeReader()
        build_three_node_tree(r)
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)
        self.assertEqual(res["status"], DMGMAP_COMPLETE)

    def test_empty_tree_returns_empty_status(self):
        r = FakeReader()
        build_empty_tree(r)  # genuinely empty (root nil), count 0
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_EMPTY)

    def test_null_head_with_zero_count_is_empty(self):
        r = FakeReader()
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, 0)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 0)
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_EMPTY)

    def test_single_entry(self):
        r = FakeReader()
        # Root with both children nil (= HEAD).
        r.write_node(HEAD, left=NODE_A, parent=NODE_A, right=NODE_A, is_nil=True)
        r.write_node(NODE_A, left=HEAD, parent=HEAD, right=HEAD, is_nil=False,
                     key=70, value=123456789)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 1)
        res = self._walk(r)
        self.assertEqual(res["entries"], [{"equip_key": 70, "damage": 123456789}])
        self.assertEqual(res["status"], DMGMAP_COMPLETE)

    def test_u64_value_high_bits_preserved(self):
        r = FakeReader()
        big = 0xFFFFFFFF00000001
        r.write_node(HEAD, left=NODE_A, parent=NODE_A, right=NODE_A, is_nil=True)
        r.write_node(NODE_A, left=HEAD, parent=HEAD, right=HEAD, is_nil=False,
                     key=1, value=big)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 1)
        res = self._walk(r)
        self.assertEqual(res["entries"][0]["damage"], big)
        self.assertEqual(res["status"], DMGMAP_COMPLETE)

    # ------------------------- unavailable pre-checks ------------------------
    def test_off_heap_side_is_unavailable(self):
        r = FakeReader()
        # 0x100 is below HEAP_LO -> _ptr_ok(side) is False.
        res = self._stats(r).read_side_damage_map(0x100)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_UNAVAILABLE)

    def test_null_head_with_nonzero_count_is_unavailable(self):
        # A null head sentinel but a count that claims entries -> corrupt, fail-closed.
        r = FakeReader()
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, 0)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 3)
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_UNAVAILABLE)

    def test_torn_head_read_is_unavailable(self):
        # Head pointer bytes never written -> torn read (None) -> cannot even
        # establish emptiness.
        r = FakeReader()
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 0)
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_UNAVAILABLE)

    # ---------------------- finding 1: count is a cross-check ----------------
    def test_lying_count_low_walks_full_structure_and_flags_partial(self):
        # Finding 1: a valid 3-node tree whose count field LIES as 1 must NOT be
        # truncated to the first entry. The structural walk returns all 3 real
        # rows and the count mismatch downgrades the status to PARTIAL (not
        # complete, not empty) -- partial data never presented as complete.
        r = FakeReader()
        build_three_node_tree(r, size=1)
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)  # all three, structure-driven
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_lying_count_high_returns_real_rows_and_flags_partial(self):
        # A count of 8 over a real 3-node tree yields the 3 REAL rows (no fabricated
        # padding) and a PARTIAL status from the count cross-check.
        r = FakeReader()
        build_three_node_tree(r, size=8)
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)
        self.assertEqual(res["status"], DMGMAP_PARTIAL)
        self.assertTrue(all(e["equip_key"] is not None for e in res["entries"]))

    # ---------------------- finding 2: adversarial corruption ----------------
    def test_cycle_is_finite_and_partial(self):
        # A corrupt tree whose root right-child points back at the root must not
        # spin. The visited-set cycle guard terminates the walk; the root is
        # visited exactly once and the status is PARTIAL (not a clean result).
        r = FakeReader()
        r.write_node(HEAD, left=NODE_A, parent=NODE_A, right=NODE_A, is_nil=True)
        r.write_node(NODE_A, left=HEAD, parent=HEAD, right=NODE_A, is_nil=False,
                     key=9, value=42)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 8)
        res = self._walk(r)
        self.assertEqual(res["entries"], [{"equip_key": 9, "damage": 42}])
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_torn_node_field_emits_no_none_keyed_row(self):
        # A value node whose span is unreadable mid-walk must NOT become a row with
        # equip_key=None. The walk stops at the torn node; only the intact rows are
        # returned and the status is PARTIAL.
        r = FakeReader()
        build_three_node_tree(r)
        r.clear_span(NODE_C)  # right leaf torn (key 20)
        res = self._walk(r)
        self.assertTrue(all(e["equip_key"] is not None for e in res["entries"]))
        self.assertNotIn({"equip_key": None, "damage": None}, res["entries"])
        # B (5) and A (10) precede C in-order and are intact.
        self.assertEqual(res["entries"],
                         [{"equip_key": 5, "damage": 100},
                          {"equip_key": 10, "damage": 200}])
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_bogus_sentinel_head_is_unavailable(self):
        # The HEAD is mislabeled as a value node (_Isnil == 0). It is NOT the
        # expected nil sentinel, so the structure is rejected fail-closed with no
        # fabricated row rather than treated as a value.
        r = FakeReader()
        build_three_node_tree(r)
        r.wu8(HEAD + NODE_ISNIL, 0)  # bogus: sentinel now claims to be a value
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_UNAVAILABLE)

    def test_out_of_heap_child_is_partial_not_wild_read(self):
        # Finding 3: a child pointer of HEAP_HI-1 passes a bare base check but its
        # node span (base+0x30) runs past the heap. The full-span validation rejects
        # it, so the walk stops (no wild read past HEAP_HI) and flags PARTIAL.
        r = FakeReader()
        # Root A(key 10) with right child = HEAP_HI-1 (near-HI, span overruns).
        r.write_node(HEAD, left=NODE_A, parent=NODE_A, right=HEAP_HI - 1, is_nil=True)
        r.write_node(NODE_A, left=HEAD, parent=HEAD, right=HEAP_HI - 1, is_nil=False,
                     key=10, value=200)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 2)
        res = self._walk(r)
        # A (intact) is emitted; the near-HI right child is refused rather than read.
        self.assertEqual(res["entries"], [{"equip_key": 10, "damage": 200}])
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    # ------ finding 1 (re-review): strict ordering + sentinel identity --------
    def test_out_of_order_keys_valid_count_flags_partial_not_complete(self):
        # An INVALID BST (root 10, left child key 20, right child key 5) whose
        # element count matches the node count. The in-order walk yields 20, 10, 5
        # -- non-increasing -- so it is NOT a valid std::map. A count match must NOT
        # buy a "complete" badge: all real rows are returned, status is PARTIAL.
        r = FakeReader()
        r.write_node(HEAD, left=NODE_B, parent=NODE_A, right=NODE_C, is_nil=True)
        r.write_node(NODE_A, left=NODE_B, parent=HEAD, right=NODE_C, is_nil=False,
                     key=10, value=200)
        r.write_node(NODE_B, left=HEAD, parent=NODE_A, right=HEAD, is_nil=False,
                     key=20, value=100)  # left child but LARGER key -> bad ordering
        r.write_node(NODE_C, left=HEAD, parent=NODE_A, right=HEAD, is_nil=False,
                     key=5, value=300)   # right child but SMALLER key
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 3)  # count MATCHES the 3 nodes
        res = self._walk(r)
        self.assertEqual(res["entries"],
                         [{"equip_key": 20, "damage": 100},
                          {"equip_key": 10, "damage": 200},
                          {"equip_key": 5, "damage": 300}])
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_duplicate_keys_flag_partial_not_complete(self):
        # std::map keys are UNIQUE; an equal in-order pair (5, 5) is invalid even
        # though it is non-decreasing. Strict monotonicity must reject it.
        r = FakeReader()
        r.write_node(HEAD, left=NODE_B, parent=NODE_A, right=NODE_C, is_nil=True)
        r.write_node(NODE_A, left=NODE_B, parent=HEAD, right=NODE_C, is_nil=False,
                     key=5, value=200)
        r.write_node(NODE_B, left=HEAD, parent=NODE_A, right=HEAD, is_nil=False,
                     key=5, value=100)  # duplicate key
        r.write_node(NODE_C, left=HEAD, parent=NODE_A, right=HEAD, is_nil=False,
                     key=9, value=300)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 3)
        res = self._walk(r)
        self.assertEqual(len(res["entries"]), 3)
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_foreign_sentinel_terminator_flags_partial(self):
        # A leaf terminates on a FOREIGN in-heap nil node (NODE_D, _Isnil=1) instead
        # of the tree's real head. MSVC std::map has exactly one sentinel, so a nil
        # that is not `head` is corrupt: rows already reached are returned, status
        # PARTIAL (never complete), and no fabricated row is emitted.
        NODE_D = 0x20000005000
        r = FakeReader()
        r.write_node(HEAD, left=NODE_B, parent=NODE_A, right=NODE_C, is_nil=True)
        r.write_node(NODE_A, left=NODE_B, parent=HEAD, right=NODE_C, is_nil=False,
                     key=10, value=200)
        r.write_node(NODE_B, left=HEAD, parent=NODE_A, right=HEAD, is_nil=False,
                     key=5, value=100)
        # C's right child is the FOREIGN nil D, not HEAD.
        r.write_node(NODE_C, left=HEAD, parent=NODE_A, right=NODE_D, is_nil=False,
                     key=20, value=300)
        r.write_node(NODE_D, left=NODE_D, parent=NODE_D, right=NODE_D, is_nil=True)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 3)
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)  # 5,10,20 reached before the foreign nil
        self.assertTrue(all(e["equip_key"] is not None for e in res["entries"]))
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_corrupt_root_parent_metadata_flags_partial(self):
        # A structurally clean, in-order-ascending 3-node tree, but the root node's
        # _Parent is corrupted to point at a non-head node. The head/parent metadata
        # no longer describes the tree -> not a trustworthy-complete std::map.
        r = FakeReader()
        build_three_node_tree(r)  # valid ascending 5,10,20; count 3
        r.wu64(NODE_A + NODE_PARENT, NODE_C)  # root._Parent != head
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)  # walk itself is clean/ascending
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_corrupt_head_leftmost_metadata_flags_partial(self):
        # The head sentinel's declared leftmost (_Left) disagrees with the in-order
        # first node the walk reached. Inconsistent container metadata -> partial.
        r = FakeReader()
        build_three_node_tree(r)
        r.wu64(HEAD + NODE_LEFT, NODE_C)  # claims rightmost as leftmost
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    # --- finding 1(a): FULL parent-backlink validation (non-root parents too) --
    def test_corrupt_non_root_parent_backlink_flags_partial(self):
        # FINDING 1(a): the OLD structural check validated only the ROOT's _Parent,
        # so a corrupt NON-root parent link slipped through and badged "complete".
        # Here the LEFT leaf B lies about its parent (B._Parent = NODE_C, the right
        # leaf, instead of the root A). The tree is still ascending 5,10,20 and the
        # count matches, but B's parent backlink is internally inconsistent, so the
        # full per-node parent-backlink check downgrades to PARTIAL (never complete)
        # while still returning all three real rows.
        r = FakeReader()
        build_three_node_tree(r)  # valid ascending 5,10,20; count 3
        r.wu64(NODE_B + NODE_PARENT, NODE_C)  # NON-root leaf lies about its parent
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)  # walk is clean/ascending
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    def test_corrupt_right_leaf_parent_backlink_flags_partial(self):
        # Symmetric to the above for the RIGHT leaf: C claims a bogus parent. A
        # right-child backlink lie must also be caught by the full traversal check.
        r = FakeReader()
        build_three_node_tree(r)
        r.wu64(NODE_C + NODE_PARENT, NODE_B)  # right leaf lies about its parent
        res = self._walk(r)
        self.assertEqual(res["entries"], THREE)
        self.assertEqual(res["status"], DMGMAP_PARTIAL)

    # --- finding 1(b): empty-map _Left/_Right invariant -----------------------
    def test_empty_head_with_corrupt_left_is_not_empty(self):
        # FINDING 1(b): an EMPTY head (root == head, i.e. _Parent points back at the
        # head) is only genuinely empty if _Left == head AND _Right == head. Here the
        # head declares itself empty (_Parent = head) but its _Left points at a stray
        # node -> corrupt metadata, NOT a real empty map. It must fail closed
        # (unavailable), never badge "empty".
        r = FakeReader()
        r.write_node(HEAD, left=NODE_A, parent=HEAD, right=HEAD, is_nil=True)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 0)
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_UNAVAILABLE)

    def test_empty_head_with_corrupt_right_is_not_empty(self):
        # Symmetric: _Parent = head (empty) but _Right points elsewhere -> not empty.
        r = FakeReader()
        r.write_node(HEAD, left=HEAD, parent=HEAD, right=NODE_A, is_nil=True)
        r.wu64(SIDE + OFF_SIDE_DMGMAP_HEAD, HEAD)
        r.wu32(SIDE + OFF_SIDE_DMGMAP_SIZE, 0)
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_UNAVAILABLE)

    def test_proper_empty_head_still_empty(self):
        # Guard the honest path: a proper empty head (_Parent/_Left/_Right all == head)
        # must STILL classify as "empty", not be swept up by the 1(b) check.
        r = FakeReader()
        build_empty_tree(r)  # _Left = _Parent = _Right = HEAD
        res = self._walk(r)
        self.assertEqual(res["entries"], [])
        self.assertEqual(res["status"], DMGMAP_EMPTY)

    # ------------------------- read_side_stats threading ----------------------
    def test_read_side_stats_surfaces_status_complete(self):
        r = FakeReader()
        build_three_node_tree(r)
        side = self._stats(r).read_side_stats(SIDE)
        self.assertIsNotNone(side)
        self.assertEqual(side["damage_map_size"], 3)
        self.assertEqual(side["damage_map"], THREE)
        self.assertEqual(side["damage_map_status"], DMGMAP_COMPLETE)

    def test_read_side_stats_surfaces_status_partial_on_lying_count(self):
        # The availability flag must REACH read_side_stats' output, not be dropped:
        # a lying count surfaces damage_map_status == partial alongside the real rows.
        r = FakeReader()
        build_three_node_tree(r, size=1)
        side = self._stats(r).read_side_stats(SIDE)
        self.assertIsNotNone(side)
        self.assertEqual(side["damage_map"], THREE)
        self.assertEqual(side["damage_map_status"], DMGMAP_PARTIAL)


if __name__ == "__main__":
    unittest.main()
