"""Generate and check engine-sites, the collector's attach-time site locator.

usage:
  python tools/sites.py generate               write crates/engine-sites/src/generated.rs, then check
  python tools/sites.py check                  locate every key in every known build
  python tools/sites.py relocate --exe P --build ID   locate every key in another executable

Code names each site by its position in the reference build: an `Rva(0x...)` key outside test
code. `generate` collects those keys and derives, from the reference build and the RE index, how
the collector finds each one. A function holding a code key is fingerprinted over its .pdata
range: a hash of its bytes with the fields that change when code or data moves masked out (rel32
branches, RIP-relative and image-relative displacements, jump-table entries, and the source-line
arguments of the game's log and assert calls). A unique fingerprint is found by enumerating .pdata;
an ambiguous or unwound function is reached through a rel32, RIP-relative or vtable-slot reference
from a function already found and must match its fingerprint there. Vtables come from RTTI (a hash
of the type name and the subobject offset); globals from operands of located code. Each vtable
also records its slot count and the functions that construct its class, which witness the class's
layout on builds whose layouts were not checked.

Known builds (BUILDS) are read from the RE tool's pinned executables. A non-reference build is
checked against the addresses recorded for it in HOI4RE_DATA/sites/<build>.json
({"<reference rva>": rva}); `relocate` writes that file for a new build from what the locator finds.
Builds in engine_sites::LAYOUT_CHECKED are checked with their layouts checked as well. To change the
reference: relocate the old one while it is still the reference, put the new one first in BUILDS,
move each key to its position there (its sites/<build>.json), and generate.
Needs Python 3.11, capstone and numpy.
Set HOI4RE_DATA to the RE data directory if hoi4re is not on PATH.
"""
import argparse
import bisect
import collections
import hashlib
import json
import os
import re
import shutil
import struct
import sys

import capstone
import numpy as np
from capstone import x86

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATA = os.environ.get("HOI4RE_DATA", "")
if not DATA and (hoi4re := shutil.which("hoi4re")):
    DATA = os.path.join(os.path.dirname(hoi4re), "hoi4re", "data")
INDEX = os.path.join(DATA, "builds", "{build}", "index.sqlite3")
OUTPUT = os.path.join(ROOT, "crates", "engine-sites", "src", "generated.rs")
LIB = os.path.join(ROOT, "crates", "engine-sites", "src", "lib.rs")
U64 = (1 << 64) - 1
IMAGE_BASE = 0x140000000
M1, M2, M3 = 0x9E3779B97F4A7C15, 0xBF58476D1CE4E5B9, 0x94D049BB133111EB
SOURCE_SUFFIXES = (b".cpp", b".h", b".hpp", b".c", b".inl", b".cc")
ARGS = (x86.X86_REG_RCX, x86.X86_REG_RDX, x86.X86_REG_R8, x86.X86_REG_R9)
ARGS32 = (x86.X86_REG_ECX, x86.X86_REG_EDX, x86.X86_REG_R8D, x86.X86_REG_R9D)
ORIGINAL = 32
WITNESS_LIMIT = 1024


def fingerprint(data, mask=()):
    """The 64-bit hash engine-sites computes: length, then 8-byte words with masked fields zeroed."""
    buf = bytearray(data)
    for k in mask:
        buf[k:k + 4] = b"\0\0\0\0"
    n = len(buf)
    buf += b"\0" * (-n % 8)
    h = (n * M1) & U64
    for w in struct.unpack("<%dQ" % (len(buf) // 8), buf):
        h = ((h ^ w) * M2) & U64
        h ^= h >> 29
    h ^= h >> 32
    h = (h * M3) & U64
    return h ^ (h >> 29)


HEAD = 16


def head(data, mask):
    """The prefilter engine-sites checks before a full fingerprint: the first 16 bytes, masked."""
    h = bytearray(data[:HEAD])
    for k in mask:
        if k >= len(h):
            break
        h[k:min(k + 4, len(h))] = bytes(min(k + 4, len(h)) - k)
    return fingerprint(h)


class Image:
    """A PE32+ image laid out at its RVAs, its sections, .pdata ranges and RTTI."""

    def __init__(self, path, build=None):
        self.build = build
        self._leaves = {}
        self._matches = {}
        raw = open(path, "rb").read()
        pe = struct.unpack_from("<I", raw, 0x3C)[0]
        nsec = struct.unpack_from("<H", raw, pe + 6)[0]
        opt = pe + 24
        self.image_base = struct.unpack_from("<Q", raw, opt + 24)[0]
        size = struct.unpack_from("<I", raw, opt + 56)[0]
        self.img = bytearray(size)
        self.sections = {}
        sec = opt + struct.unpack_from("<H", raw, pe + 20)[0]
        for i in range(nsec):
            o = sec + 40 * i
            name = raw[o:o + 8].rstrip(b"\0").decode()
            vsize, va, rsize, rptr = struct.unpack_from("<IIII", raw, o + 8)
            self.img[va:va + min(vsize, rsize)] = raw[rptr:rptr + min(vsize, rsize)]
            self.sections[name] = (va, va + vsize)
        p0, p1 = self.sections[".pdata"]
        ranges = []
        for o in range(p0, p1 - 11, 12):
            b, e, _ = struct.unpack_from("<III", self.img, o)
            if b == 0:
                break
            ranges.append((b, e))
        self.ranges = sorted(set(ranges))
        self.starts = [b for b, _ in self.ranges]
        self.ends = {b: e for b, e in self.ranges}
        self.by_len = collections.defaultdict(list)
        for b, e in self.ranges:
            self.by_len[e - b].append(b)
        self.md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
        self.md.detail = True
        self._analysis = {}
        self._rtti = None
        self._refs = None

    def section(self, rva):
        for name, (a, b) in self.sections.items():
            if a <= rva < b:
                return name

    def range_of(self, rva):
        i = bisect.bisect_right(self.starts, rva) - 1
        if i >= 0 and self.ranges[i][0] <= rva < self.ranges[i][1]:
            return self.ranges[i]

    def bounds(self, rva):
        """(start, end) of the function holding rva: its .pdata range, or for code without unwind
        data the RE index function."""
        r = self.range_of(rva)
        if r or self.build is None:
            return r
        if rva not in self._leaves:
            self._leaves[rva] = leaf_bounds(self.build, rva)
        lb = self._leaves[rva]
        if lb and self.range_of(lb[0]) is None:
            return lb

    def length(self, start):
        if start in self.ends:
            return self.ends[start] - start
        return self.bounds(start)[1] - start

    def u32(self, rva):
        return struct.unpack_from("<I", self.img, rva)[0]

    def i32(self, rva):
        return struct.unpack_from("<i", self.img, rva)[0]

    def u64(self, rva):
        return struct.unpack_from("<Q", self.img, rva)[0]

    def cstring(self, rva, limit=512):
        if not 0 <= rva < len(self.img):
            return None
        end = self.img.find(b"\0", rva, rva + limit)
        return bytes(self.img[rva:end]) if end >= 0 else None

    def source_path(self, rva):
        if self.section(rva) != ".rdata":
            return False
        s = self.cstring(rva)
        return s is not None and (s == b"" or s.lower().endswith(SOURCE_SUFFIXES))

    def fingerprint_at(self, start, length, mask):
        if start < 0 or start + length > len(self.img):
            return None
        return fingerprint(self.img[start:start + length], mask)


    def analyse(self, start, length):
        """Mask and references of the function at [start, start + length).

        Recursive descent from the start: instructions reached by fall-through, rel8/rel32
        branches inside the range and jump tables. Bytes never reached (padding, byte tables) stay
        in the fingerprint unmasked. Returns (mask, edges, instruction starts) where each edge is
        (field offset, end offset, kind, target rva) and kind is 'rel', 'rip' or 'image'.
        """
        key = (start, length)
        if key in self._analysis:
            return self._analysis[key]
        end = start + length
        code = bytes(self.img[start:end])
        seen = {}
        work = [start]
        image_regs = set()
        fields = {}
        tables = []
        while work:
            pc = work.pop()
            while start <= pc < end and pc not in seen:
                insn = next(self.md.disasm(code[pc - start:pc - start + 16], pc, 1), None)
                if insn is None or pc + insn.size > end:
                    break
                seen[pc] = insn
                op = code[pc - start:pc - start + insn.size]
                nxt = pc + insn.size
                grp = insn.groups
                is_jump = x86.X86_GRP_JUMP in grp
                is_call = x86.X86_GRP_CALL in grp
                is_ret = x86.X86_GRP_RET in grp
                if (is_jump or is_call) and insn.operands and insn.operands[0].type == x86.X86_OP_IMM:
                    target = insn.operands[0].imm
                    if insn.size >= 5 and (op[-5] in (0xE8, 0xE9) or (len(op) >= 6 and op[-6] == 0x0F and 0x80 <= op[-5] <= 0x8F)):
                        if not start <= target < end:
                            fields[nxt - 4 - start] = (nxt - start, "rel", target)
                    if is_jump and start <= target < end:
                        work.append(target)
                for o in insn.operands:
                    if o.type != x86.X86_OP_MEM:
                        continue
                    if o.mem.base == x86.X86_REG_RIP:
                        target = nxt + o.mem.disp
                        if not start <= target < end:
                            fields[pc + insn.disp_offset - start] = (nxt - start, "rip", target)
                        if insn.id == x86.X86_INS_LEA and target == 0:
                            image_regs.add(insn.operands[0].reg)
                    elif insn.disp_size == 4 and (o.mem.base in image_regs or o.mem.index in image_regs) and o.mem.index:
                        disp = o.mem.disp & 0xFFFFFFFF
                        if self.section(disp) is not None:
                            fields[pc + insn.disp_offset - start] = (nxt - start, "image", disp)
                            if o.size == 4 and o.mem.scale == 4 and start <= disp < end:
                                tables.append(disp)
                if is_ret or insn.id in (x86.X86_INS_INT3, x86.X86_INS_UD2, x86.X86_INS_HLT):
                    break
                if is_jump and insn.id == x86.X86_INS_JMP:
                    if tables and insn.operands[0].type == x86.X86_OP_REG:
                        for t in tables:
                            k = t
                            while k + 4 <= end and k not in seen:
                                entry = self.u32(k)
                                if not start <= entry < end:
                                    break
                                fields[k - start] = (k + 4 - start, "table", entry)
                                work.append(entry)
                                k += 4
                        tables = []
                    break
                pc = nxt
        table_bytes = {start + k + j for k, (_, kind, _) in fields.items() if kind == "table" for j in range(4)}
        for pc in [pc for pc, insn in seen.items() if any(q in table_bytes for q in range(pc, pc + insn.size))]:
            del seen[pc]
        for k in [k for k, (_, kind, _) in fields.items()
                  if kind != "table" and any(start + k + j in table_bytes for j in range(4))]:
            del fields[k]
        covered = set(table_bytes)
        for pc, insn in seen.items():
            covered.update(range(pc, pc + insn.size))
        pc = start
        while pc < end:
            if pc in covered or self.img[pc] == 0xCC:
                pc += 1
                continue
            insn = next(self.md.disasm(code[pc - start:pc - start + 16], pc, 1), None)
            if insn is None or pc + insn.size > end or any(q in covered for q in range(pc, pc + insn.size)):
                pc += 1
                continue
            seen[pc] = insn
            covered.update(range(pc, pc + insn.size))
            op = code[pc - start:pc - start + insn.size]
            nxt = pc + insn.size
            if insn.size >= 5 and (op[-5] in (0xE8, 0xE9) or (len(op) >= 6 and op[-6] == 0x0F and 0x80 <= op[-5] <= 0x8F)) \
                    and insn.operands and insn.operands[0].type == x86.X86_OP_IMM:
                if not start <= insn.operands[0].imm < end:
                    fields[nxt - 4 - start] = (nxt - start, "rel", insn.operands[0].imm)
            for o in insn.operands:
                if o.type == x86.X86_OP_MEM and o.mem.base == x86.X86_REG_RIP and not start <= nxt + o.mem.disp < end:
                    fields[pc + insn.disp_offset - start] = (nxt - start, "rip", nxt + o.mem.disp)
            pc = nxt
        order = sorted(seen)
        for i, pc in enumerate(order):
            insn = seen[pc]
            if x86.X86_GRP_CALL not in insn.groups:
                continue
            window = []
            for q in reversed(order[max(0, i - 14):i]):
                j = seen[q]
                if x86.X86_GRP_CALL in j.groups or x86.X86_GRP_JUMP in j.groups:
                    break
                window.append(j)
            path_arg = None
            for j in window:
                if j.id == x86.X86_INS_LEA and j.operands[1].mem.base == x86.X86_REG_RIP and j.operands[0].reg in ARGS:
                    if self.source_path(j.address + j.size + j.operands[1].mem.disp):
                        path_arg = ARGS.index(j.operands[0].reg)
                        break
            if path_arg is None or path_arg + 1 >= len(ARGS):
                continue
            line_reg = ARGS32[path_arg + 1]
            for j in window:
                if j.id == x86.X86_INS_MOV and len(j.operands) == 2 and j.operands[0].type == x86.X86_OP_REG \
                        and j.operands[0].reg == line_reg and j.operands[1].type == x86.X86_OP_IMM and j.imm_size == 4:
                    fields[j.address + j.imm_offset - start] = (j.address + j.size - start, "line", None)
                    break
                if j.operands and j.operands[0].type == x86.X86_OP_REG and j.operands[0].reg in (line_reg, ARGS[path_arg + 1]):
                    break
        mask = sorted(fields)
        for a, b in zip(mask, mask[1:]):
            if b < a + 4:
                raise ValueError("overlapping fields at %#x+%#x" % (start, a))
        edges = [(k, e, kind, t) for k, (e, kind, t) in sorted(fields.items()) if kind in ("rel", "rip", "image", "table")]
        result = (mask, edges, set(seen))
        self._analysis[key] = result
        return result

    def matches(self, length, mask, value):
        """Starts of .pdata ranges of this length whose fingerprint under the mask is value."""
        key = (length, tuple(mask), value)
        if key not in self._matches:
            ranges = self.by_len.get(length, ())
            if not ranges:
                self._matches[key] = []
            else:
                first = next((s for s in ranges if self.fingerprint_at(s, length, mask) == value), None)
                want = head(self.img[first:first + length], mask) if first is not None else None
                self._matches[key] = [s for s in ranges if want is not None and head(self.img[s:s + length], mask) == want
                                      and self.fingerprint_at(s, length, mask) == value]
        return self._matches[key]


    def rtti(self):
        """{(name hash, offset): [vtable rva]} for every vtable with a complete object locator."""
        if self._rtti is not None:
            return self._rtti
        r0, r1 = self.sections[".rdata"]
        rd = np.frombuffer(bytes(self.img[r0:r1 - (r1 - r0) % 4]), dtype="<u4")
        idx = np.nonzero(rd == 1)[0]
        cols = {}
        for i in idx:
            rva = r0 + 4 * int(i)
            if rva + 24 <= r1 and self.u32(rva + 20) == rva:
                td = self.u32(rva + 12)
                name = self.cstring(td + 16)
                if name and name.startswith(b".?A"):
                    cols[rva] = (fingerprint(name), self.u32(rva + 4))
        q0 = r0 + (-r0 % 8)
        rq = np.frombuffer(bytes(self.img[q0:r1 - (r1 - q0) % 8]), dtype="<u8")
        want = np.array(sorted(cols), dtype=np.uint64) + np.uint64(self.image_base)
        hits = np.nonzero(np.isin(rq, want))[0]
        out = collections.defaultdict(list)
        for i in hits:
            vt = q0 + 8 * int(i) + 8
            col = int(rq[i]) - self.image_base
            out[cols[col]].append(vt)
        self._rtti = out
        return out

    def vtable_key(self, vt):
        col = self.u64(vt - 8) - self.image_base
        if 0 <= col < len(self.img) and self.u32(col) == 1 and self.u32(col + 20) == col:
            name = self.cstring(self.u32(col + 12) + 16)
            if name and name.startswith(b".?A"):
                return (fingerprint(name), self.u32(col + 4))


    def referencing(self, targets):
        """{target: [(range start, field offset)]}: .text fields whose rel32/RIP target is a target.

        Candidates come from a vectorised scan of every 4-byte window; each is confirmed by
        decoding its function, so only real operands remain.
        """
        t0, t1 = self.sections[".text"]
        text = np.frombuffer(bytes(self.img[t0:t1]), dtype=np.uint8).astype(np.int64)
        d = text[:-3] | (text[1:-2] << 8) | (text[2:-1] << 16) | (text[3:] << 24)
        d = np.where(d >= 1 << 31, d - (1 << 32), d)
        pos = np.arange(t0, t0 + len(d), dtype=np.int64)
        want = np.array(sorted(targets), dtype=np.int64)
        out = collections.defaultdict(list)
        cand = set()
        for extra in (0, 1, 2, 4):
            tgt = pos + 4 + extra + d
            for i in np.nonzero(np.isin(tgt, want))[0]:
                cand.add(int(pos[i]))
        for p in sorted(cand):
            r = self.bounds(p)
            if r is None:
                continue
            _, edges, _ = self.analyse(r[0], r[1] - r[0])
            for k, _, kind, t in edges:
                if r[0] + k == p and t in targets and kind in ("rel", "rip"):
                    out[t].append((r[0], k))
        return out

    def slot_count(self, vt):
        """Slots of the vtable at vt: pointers into .text, as engine-sites counts them."""
        n = 0
        while n <= 4096:
            p = self.u64(vt + 8 * n) - self.image_base
            if not 0 <= p < len(self.img) or self.section(p) != ".text":
                break
            n += 1
        return n



def locate(image, spec, layout_checked=True):
    """(function starts, vtables, data) as {index: rva or reason}, and the layout verdict.

    Mirrors engine-sites: located parts whose own checks fail are excluded as evidence and
    everything is recomputed until no check fails.
    """
    functions, vtables, data = spec["functions"], spec["vtables"], spec["data"]
    roots = {}
    for i, f in enumerate(functions):
        if f["unique"]:
            hits = image.matches(f["len"], f["mask"], f["hash"])
            roots[i] = hits[0] if len(hits) == 1 else ("changed" if not hits else "ambiguous")
    rtti = image.rtti()
    vt = {}
    for i, v in enumerate(vtables):
        hits = rtti.get((v["name"], v["offset"]), [])
        vt[i] = hits[0] if len(hits) == 1 else ("changed" if not hits else "ambiguous")
    by_rtti = dict(vt)
    for i, v in enumerate(vtables):
        if isinstance(vt[i], int) and image.slot_count(vt[i]) != v["slots"]:
            vt[i] = "vtable shape"
    banned = {}
    while True:
        fn = {i: banned.get(i, roots.get(i, "unreached")) for i in range(len(functions))}

        def follow(edge):
            src = edge["from"]
            if edge["via"] == "slot":
                return image.u64(vt[src] + 8 * edge["slot"]) - image.image_base if isinstance(vt[src], int) else None
            if not isinstance(fn[src], int):
                return None
            if edge["via"] == "after":
                return fn[src] + edge["delta"]
            return fn[src] + edge["end"] + image.i32(fn[src] + edge["field"])

        changed = True
        while changed:
            changed = False
            for i, f in enumerate(functions):
                if isinstance(fn[i], int) or not f["from"] or fn[i] == "changed" or i in banned:
                    continue
                got = {t for t in (follow(e) for e in f["from"]) if t is not None
                       and (not f["pdata"] or image.ends.get(t) == t + f["len"])
                       and image.fingerprint_at(t, f["len"], f["mask"]) == f["hash"]}
                if len(got) > 1:
                    fn[i] = "references disagree"
                elif got:
                    fn[i] = got.pop()
                    changed = True
        newly = {}
        for i, f in enumerate(functions):
            if f["unique"] or i in banned or not isinstance(fn[i], int):
                continue
            if any(t is not None and t != fn[i] and (not f["pdata"] or image.ends.get(t) == t + f["len"])
                   and image.fingerprint_at(t, f["len"], f["mask"]) == f["hash"] for t in map(follow, f["from"])):
                newly[i] = "references disagree"
        dt = {}
        for i, d in enumerate(data):
            got = {t for t in (follow(e) for e in d["from"]) if t is not None}
            if not got:
                dt[i] = "unreached"
            elif len(got) > 1:
                dt[i] = "references disagree"
            elif image.section(next(iter(got))) != d["section"]:
                dt[i] = "leaves its section"
            else:
                dt[i] = got.pop()
        for c in spec.get("checks", ()):
            f, kind, index = c["function"], c["kind"], c["index"]
            if not isinstance(fn[f], int) or f in newly:
                continue
            if kind == "internal":
                if image.u32(fn[f] + c["field"]) != fn[f] + index:
                    newly[f] = "an operand reaches a different target than in the reference build"
                continue
            if kind == "inside":
                target = fn[index] + c["offset"] if isinstance(fn[index], int) else fn[index]
            else:
                target = (fn if kind == "function" else dt if kind == "data" else by_rtti)[index]
            if not isinstance(target, int):
                newly[f] = "an operand reaches a withheld function or global"
            elif fn[f] + c["end"] + image.i32(fn[f] + c["field"]) != target:
                newly[f] = "an operand reaches a different target than in the reference build"
        if not newly:
            break
        banned.update(newly)
    layout = None
    if not layout_checked:
        for i, v in enumerate(vtables):
            if not v["witnesses"] and isinstance(vt[i], int):
                vt[i] = "no constructor witnesses its layout"
            if any(not isinstance(fn[w], int) for w in v["witnesses"]):
                if not v["veto"] and isinstance(vt[i], int):
                    vt[i] = "the function witnessing its layout changed"
                elif v["veto"] and layout is None:
                    layout = "constructor of vtable %d changed" % i
    return fn, vt, dt, layout


def resolve_keys(image, spec, layout_checked=True):
    """{key rva: live rva or reason}."""
    fn, vt, dt, layout = locate(image, spec, layout_checked)
    out = {}
    for k in spec["keys"]:
        kind, i = k["kind"], k["index"]
        if layout:
            out[k["rva"]] = "layout: " + layout
        elif kind == "code":
            f = spec["functions"][i]
            out[k["rva"]] = fn[i] + k["rva"] - f["start"] if isinstance(fn[i], int) else fn[i]
        elif kind == "vtable":
            out[k["rva"]] = vt[i]
        else:
            out[k["rva"]] = dt[i]
    return out



KEY = re.compile(r"\bRva\(\s*(0x[0-9a-fA-F_]+)\s*\)")
CONST = re.compile(r"\b(?:const|static)\s+([A-Z][A-Z0-9_]*)\s*:")
NAME = re.compile(r'\bname:\s*"([^"]+)"')
FN = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)")
STRING = re.compile(r'"([A-Za-z][A-Za-z0-9_ ]*)"')
TEST_PATH = re.compile(r"-tests/|/tests?/|tests?\.rs$|_tests?\.rs$|test_kit|fixtures?\b|bench|/examples/")


def test_lines(text):
    """Line numbers inside #[cfg(test)] items."""
    lines = text.split("\n")
    out = set()
    i = 0
    while i < len(lines):
        if lines[i].strip().startswith("#[cfg(test)]"):
            depth, started, j = 0, False, i + 1
            while j < len(lines):
                code = lines[j].split("//")[0]
                depth += code.count("{") - code.count("}")
                started = started or "{" in code
                out.add(j + 1)
                if (started and depth <= 0) or (not started and code.rstrip().endswith(";")):
                    break
                j += 1
            i = j
        i += 1
    return out


def source_keys():
    """{rva: (name, path, line)} for every Rva literal outside test code."""
    keys = {}
    for crate in sorted(os.listdir(os.path.join(ROOT, "crates"))):
        src = os.path.join(ROOT, "crates", crate, "src")
        for root, _, files in os.walk(src):
            for f in sorted(files):
                if not f.endswith(".rs"):
                    continue
                path = os.path.join(root, f)
                rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
                if TEST_PATH.search(rel) or f == "generated.rs" and crate == "engine-sites":
                    continue
                text = open(path, encoding="utf-8").read()
                tests = test_lines(text)
                lines = text.split("\n")
                stem = rel.split("/src/", 1)[1][:-3].replace("/", "::")
                for n, line in enumerate(lines, 1):
                    if n in tests or line.lstrip().startswith("//"):
                        continue
                    for m in KEY.finditer(line):
                        rva = int(m.group(1)[2:].replace("_", ""), 16)
                        c = CONST.search(line)
                        if c:
                            name = "%s::%s" % (stem, c.group(1))
                        else:
                            near = [(abs(d), NAME.search(lines[n - 1 + d]).group(1)) for d in range(-8, 9)
                                    if 0 <= n - 1 + d < len(lines) and NAME.search(lines[n - 1 + d])]
                            s = STRING.search(line[m.end():])
                            if near:
                                name = "%s::%s" % (stem, min(near)[1])
                            elif s:
                                name = "%s::%s" % (stem, s.group(1))
                            else:
                                fns = [FN.search(lines[j]) for j in range(n - 1, -1, -1) if FN.search(lines[j])]
                                name = "%s::%s" % (stem, fns[0].group(1)) if fns else stem
                        if rva not in keys or (c and "::" in keys[rva][0] and not CONST.search(keys[rva][3])):
                            keys[rva] = (name, rel, n, line)
    return {k: v[:3] for k, v in keys.items()}



def leaf_bounds(build, rva):
    """[start, end) of the RE index function holding rva, for code without unwind data."""
    import sqlite3
    path = INDEX.format(build=build)
    db = sqlite3.connect("file:" + path.replace("\\", "/") + "?mode=ro", uri=True)
    row = db.execute("SELECT address, body_max FROM functions WHERE address <= ? AND body_max >= ? ORDER BY address DESC LIMIT 1",
                     (rva + IMAGE_BASE, rva + IMAGE_BASE)).fetchone()
    if row:
        return row[0] - IMAGE_BASE, row[1] - IMAGE_BASE + 1


def generate(keys, ref, others, reference):
    """The locator spec for `keys` ({rva: name}) from the reference image, and problems found."""
    problems = []
    functions, fidx = [], {}
    vtables, vidx = [], {}
    data, didx = [], {}
    uniq = {}
    everywhere = {}

    def unique(start, length):
        if start not in ref.ends:
            return False
        if (start, length) not in uniq:
            mask, _, _ = ref.analyse(start, length)
            h = ref.fingerprint_at(start, length, mask)
            counts = [len(o.matches(length, mask, h)) for o in others]
            uniq[(start, length)] = len(ref.matches(length, mask, h)) == 1 and all(c <= 1 for c in counts)
            everywhere[(start, length)] = all(c == 1 for c in counts)
        return uniq[(start, length)]

    def stable(start):
        """Unique, and found once in every other known build: the most dependable reference source."""
        return unique(start, ref.length(start)) and everywhere.get((start, ref.length(start)), False)

    def function(start, length, pdata):
        if start in fidx:
            return fidx[start]
        mask, edges, _ = ref.analyse(start, length)
        fidx[start] = len(functions)
        functions.append({"start": start, "len": length, "mask": mask, "hash": ref.fingerprint_at(start, length, mask),
                          "head": head(ref.img[start:start + length], mask),
                          "pdata": pdata, "edges": edges, "unique": pdata and unique(start, length), "from": []})
        return fidx[start]

    def field_edge(fs, k):
        """An edge through the field at offset k of the .pdata function at fs, if it is unique everywhere."""
        ln = ref.length(fs)
        if fs not in fidx and not unique(fs, ln):
            return None
        j = function(fs, ln, True)
        if not functions[j]["unique"]:
            return None
        end = next(e for kk, e, _, _ in functions[j]["edges"] if kk == k)
        return {"via": "field", "from": j, "field": k, "end": end}

    out = []
    pending_data, pending_vt = [], []
    for rva in sorted(keys):
        name = keys[rva]
        section = ref.section(rva)
        if section == ".text":
            r = ref.range_of(rva)
            if r:
                i = function(r[0], r[1] - r[0], True)
            else:
                lb = ref.bounds(rva)
                if lb is None:
                    problems.append("%s (%#x): no function holds it" % (name, rva))
                    continue
                i = function(lb[0], lb[1] - lb[0], False)
            f = functions[i]
            if rva not in ref.analyse(f["start"], f["len"])[2]:
                problems.append("%s (%#x): not an instruction boundary of its function" % (name, rva))
            out.append({"rva": rva, "name": name, "kind": "code", "index": i})
        elif section in (".rdata", ".data") and ref.vtable_key(rva):
            pending_vt.append(rva)
            out.append({"rva": rva, "name": name, "kind": "vtable", "index": None})
        elif section in (".rdata", ".data"):
            pending_data.append(rva)
            out.append({"rva": rva, "name": name, "kind": "data", "index": None})
        else:
            problems.append("%s (%#x): outside the image's sections" % (name, rva))
    rtti = ref.rtti()
    refs = ref.referencing(set(pending_data) | set(pending_vt))

    def edge(j, k):
        end = next(e for kk, e, _, _ in functions[j]["edges"] if kk == k)
        return {"via": "field", "from": j, "field": k, "end": end}

    def takes_address(fs, k):
        """Whether the field at fs+k belongs to a LEA: the function takes the vtable's address to store it."""
        _, _, seen = ref.analyse(fs, ref.length(fs))
        pc = max((s for s in seen if s <= fs + k), default=None)
        insn = pc is not None and next(ref.md.disasm(bytes(ref.img[pc:pc + 16]), pc, 1), None)
        return bool(insn) and insn.id == x86.X86_INS_LEA

    for rva in pending_vt:
        key = ref.vtable_key(rva)
        if len(rtti.get(key, ())) != 1 or any(len(o.rtti().get(key, ())) > 1 for o in others):
            problems.append("%s (%#x): its RTTI type name and offset are not unique" % (keys[rva], rva))
        stores = sorted({fs for fs, k in refs.get(rva, []) if takes_address(fs, k)}, key=lambda fs: ref.length(fs))
        found = [fs for fs in stores if unique(fs, ref.length(fs))]
        small = [fs for fs in found if ref.length(fs) <= WITNESS_LIMIT]
        veto = bool(small)
        witnesses = [function(fs, ref.length(fs), True) for fs in (small or found)[:3 if small else 1]]
        vidx[rva] = len(vtables)
        vtables.append({"rva": rva, "name": key[0], "offset": key[1], "slots": ref.slot_count(rva), "witnesses": witnesses, "veto": veto})
    for rva in pending_data:
        cands = sorted(refs.get(rva, []), key=lambda c: (not stable(c[0]), not unique(c[0], ref.length(c[0])), c[0] not in fidx, c))
        sources = []
        for fs, k in cands[:3]:
            j = function(fs, ref.length(fs), fs in ref.ends)
            if all(x["from"] != j for x in sources):
                sources.append(edge(j, k))
        didx[rva] = len(data)
        data.append({"rva": rva, "section": ref.section(rva), "from": sources})
    for rva in pending_data:
        if not data[didx[rva]]["from"]:
            problems.append("%s (%#x): no code references it" % (keys[rva], rva))
    for k in out:
        if k["kind"] == "vtable":
            k["index"] = vidx[k["rva"]]
        elif k["kind"] == "data":
            k["index"] = didx[k["rva"]]
    slots = collections.defaultdict(list)
    for key, vts in rtti.items():
        if len(vts) != 1 or any(len(o.rtti().get(key, ())) > 1 for o in others):
            continue
        for slot in range(ref.slot_count(vts[0])):
            slots[ref.u64(vts[0] + 8 * slot) - ref.image_base].append((vts[0], slot))
    frontier = {i for i, f in enumerate(functions) if not f["unique"]}
    for depth in range(6):
        if not frontier:
            break
        callers = ref.referencing({functions[i]["start"] for i in frontier})
        following = set()
        for i in sorted(frontier):
            t = functions[i]["start"]
            for vt, slot in slots.get(t, [])[:2]:
                if vt not in vidx:
                    vidx[vt] = len(vtables)
                    vk = ref.vtable_key(vt)
                    vtables.append({"rva": vt, "name": vk[0], "offset": vk[1], "slots": ref.slot_count(vt), "witnesses": [], "veto": False})
                functions[i]["from"].append({"via": "slot", "from": vidx[vt], "slot": slot})
            prev = ref.range_of(t - 1) if t in ref.ends else None
            if prev and prev[1] == t and not any(e["via"] == "after" for e in functions[i]["from"]):
                j = function(prev[0], prev[1] - prev[0], True)
                functions[i]["from"].append({"via": "after", "from": j, "delta": t - prev[0]})
                if not functions[j]["unique"] and not functions[j]["from"]:
                    following.add(j)
            cands = sorted(callers.get(t, []), key=lambda c: (not stable(c[0]), not unique(c[0], ref.length(c[0])), c[0] not in fidx, c))
            for fs, k in cands:
                if len(functions[i]["from"]) >= 3:
                    break
                j = function(fs, ref.length(fs), fs in ref.ends)
                if j == i or any(e["via"] == "field" and e["from"] == j for e in functions[i]["from"]):
                    continue
                functions[i]["from"].append(edge(j, k))
                if not functions[j]["unique"] and not functions[j]["from"]:
                    following.add(j)
        frontier = following
    spec = {"functions": functions, "vtables": vtables, "data": data, "keys": out}
    fn, _, dt, _ = locate(ref, spec)
    for i, f in enumerate(functions):
        if not isinstance(fn[i], int) and any(k["kind"] == "code" and k["index"] == i for k in out):
            problems.append("function at %#x: %s (%s)" % (f["start"], fn[i], ", ".join(
                k["name"] for k in out if k["kind"] == "code" and k["index"] == i)))
    for i, d in enumerate(data):
        if not isinstance(dt[i], int):
            problems.append("global %s (%#x): %s" % (keys[d["rva"]], d["rva"], dt[i]))
    order = sorted(range(len(functions)), key=lambda i: functions[i]["start"])
    remap = {old: new for new, old in enumerate(order)}
    functions = [functions[i] for i in order]
    for f in functions:
        for e in f["from"]:
            if e["via"] in ("field", "after"):
                e["from"] = remap[e["from"]]
    for d in data:
        for e in d["from"]:
            if e["via"] == "field":
                e["from"] = remap[e["from"]]
    for v in vtables:
        v["witnesses"] = [remap[w] for w in v["witnesses"]]
    for k in out:
        if k["kind"] == "code":
            k["index"] = remap[k["index"]]
    for f in functions:
        for e in f["from"]:
            if e["via"] == "field":
                e["from"] = e["from"]
    for i, f in enumerate(functions):
        need = max([k["rva"] - f["start"] + ORIGINAL - f["len"] for k in out if k["kind"] == "code" and k["index"] == i] + [0])
        f["tail"], f["tail_mask"], f["tail_hash"] = 0, [], 0
        if need <= 0:
            continue
        end = f["start"] + f["len"]
        mask = []
        nxt = next((b for b in ref.starts[bisect.bisect_left(ref.starts, end):][:1] if b < end + need), None)
        if nxt is not None:
            nmask, _, _ = ref.analyse(nxt, ref.ends[nxt] - nxt)
            for k in nmask:
                at = nxt - end + k
                if at < need:
                    mask.append(at)
                    need = max(need, at + 4)
        f["tail"], f["tail_mask"] = need, mask
        f["tail_hash"] = fingerprint(ref.img[end:end + need], mask)
    starts = {f["start"]: j for j, f in enumerate(functions)}
    drva = {d["rva"]: i for i, d in enumerate(data)}
    vrva = {v["rva"]: i for i, v in enumerate(vtables)}
    by_start = sorted(range(len(functions)), key=lambda j: functions[j]["start"])
    order = [functions[j]["start"] for j in by_start]
    checks = []
    for i, f in enumerate(functions):
        for k, end, kind, t in f["edges"]:
            if kind in ("table", "image") and f["start"] <= t < f["start"] + f["len"]:
                checks.append({"function": i, "field": k, "end": end, "kind": "internal", "index": t - f["start"]})
                continue
            if kind not in ("rel", "rip"):
                continue
            for name, table in (("function", starts), ("data", drva), ("vtable", vrva)):
                if t in table and not (name == "function" and table[t] == i):
                    checks.append({"function": i, "field": k, "end": end, "kind": name, "index": table[t]})
                    break
            else:
                j = bisect.bisect_right(order, t) - 1
                g = by_start[j] if j >= 0 else None
                if g is not None and g != i and functions[g]["start"] < t < functions[g]["start"] + functions[g]["len"]:
                    checks.append({"function": i, "field": k, "end": end, "kind": "inside",
                                   "index": g, "offset": t - functions[g]["start"]})
    return {"functions": functions, "vtables": vtables, "data": data, "keys": out, "checks": checks}, problems


def emit(spec, version):
    """generated.rs for engine-sites."""
    masks, edges, witnesses = [], [], []
    w = ["//! Generated by `python tools/sites.py generate` from the game %s, the reference build; do not edit." % version,
         "use crate::locate::{Check, Data, Edge, Function, Key, Kind, Section, Target, Vtable};", ""]

    def edge(e):
        if e["via"] == "slot":
            return "Edge::Slot { vtable: %d, slot: %d }" % (e["from"], e["slot"])
        if e["via"] == "after":
            return "Edge::After { function: %d, delta: %d }" % (e["from"], e["delta"])
        return "Edge::Field { from: %d, field: %d, end: %d }" % (e["from"], e["field"], e["end"])

    rows = []
    for f in spec["functions"]:
        m0, e0 = len(masks), len(edges)
        masks += f["mask"]
        edges += [edge(e) for e in f["from"]]
        t0 = len(masks)
        masks += f["tail_mask"]
        rows.append("    Function { reference: %#x, len: %d, head: %#018x, hash: %#018x, masks: (%d, %d), tail: %d, tail_hash: %#018x, tail_masks: (%d, %d), pdata: %s, root: %s, edges: (%d, %d) },"
                    % (f["start"], f["len"], f["head"], f["hash"], m0, len(f["mask"]), f["tail"], f["tail_hash"], t0, len(f["tail_mask"]), "true" if f["pdata"] else "false", "true" if f["unique"] else "false", e0, len(f["from"])))
    w.append("pub(crate) static FUNCTIONS: &[Function] = &[")
    w += rows + ["];", ""]
    rows = []
    for v in spec["vtables"]:
        w0 = len(witnesses)
        witnesses += v["witnesses"]
        rows.append("    Vtable { name: %#018x, offset: %d, slots: %d, witnesses: (%d, %d), veto: %s },"
                    % (v["name"], v["offset"], v["slots"], w0, len(v["witnesses"]), "true" if v["veto"] else "false"))
    w.append("pub(crate) static VTABLES: &[Vtable] = &[")
    w += rows + ["];", ""]
    rows = []
    sec = {".text": "Text", ".rdata": "Rdata", ".data": "Data"}
    for d in spec["data"]:
        e0 = len(edges)
        edges += [edge(e) for e in d["from"]]
        rows.append("    Data { section: Section::%s, edges: (%d, %d) }," % (sec[d["section"]], e0, len(d["from"])))
    w.append("pub(crate) static DATA: &[Data] = &[")
    w += rows + ["];", ""]
    w.append("pub(crate) static EDGES: &[Edge] = &[")
    w += ["    %s," % e for e in edges] + ["];", ""]

    def numbers(name, ty, values):
        out = ["pub(crate) static %s: &[%s] = &[" % (name, ty)]
        for i in range(0, len(values), 16):
            out.append("    " + ", ".join(str(v) for v in values[i:i + 16]) + ",")
        return out + ["];", ""]

    w += numbers("MASKS", "u32", masks)
    w += numbers("WITNESSES", "u32", witnesses)
    targets = {"function": "Function", "data": "Data", "vtable": "Vtable", "internal": "Internal", "inside": "Inside"}
    w.append("pub(crate) static CHECKS: &[Check] = &[")
    w += ["    Check { function: %d, field: %d, end: %d, target: Target::%s(%s) }," % (
        c["function"], c["field"], c["end"], targets[c["kind"]],
        "%d, %d" % (c["index"], c["offset"]) if c["kind"] == "inside" else "%d" % c["index"]) for c in spec["checks"]]
    w += ["];", ""]
    kinds = {"code": "Code", "vtable": "Vtable", "data": "Data"}
    w.append("pub(crate) static KEYS: &[Key] = &[")
    for k in spec["keys"]:
        w.append('    Key { rva: %#x, name: "%s", kind: Kind::%s(%d) },' % (k["rva"], k["name"], kinds[k["kind"]], k["index"]))
    w += ["];", ""]
    return "\n".join(w)



BUILDS = {"c184": "1.19.5.0", "0bdb": "1.19.4.2", "f584": "1.19.4.1"}
EXPECTED_WITHHELD = {
    "f584": {
        0xc4a420: "requisition_hq_from_army: its code really changed in 1.19.4.2",
        0x19c9a20: "add_hq_manpower: reached only through requisition_hq_from_army",
        0x29b6ce0: "OPTION_COMMAND_VTABLE_RVA: 1.19.4.2 added a virtual to its base class",
        0x11e59f0: "set_garrison_priority: reached only through a command vtable whose shape changed",
        0xe29230: "mio_queue_remove: reached only through a command vtable whose shape changed",
        0xf23420: "repair dockyard setting: reached only through a command vtable whose shape changed",
    },
}
PINNED = os.path.join(os.path.dirname(DATA), "..", "pinned")


def exe(build):
    for d in os.listdir(PINNED):
        if d.endswith("." + build):
            return os.path.join(PINNED, d, "hoi4.exe")
    raise SystemExit("no pinned executable for build %s under %s" % (build, PINNED))


def layout_checked():
    """The SHA-256s in engine_sites::LAYOUT_CHECKED: builds that resolve keys with checked layouts."""
    with open(LIB, encoding="utf-8") as f:
        block = re.search(r"const LAYOUT_CHECKED: &\[\[u8; 32\]\] = &\[(.*?)\n\];", f.read(), re.S).group(1)
    return {bytes(int(x, 16) for x in re.findall(r"0x([0-9a-f]{2})", row)) for row in re.findall(r"\[([^\]]*)\]", block)}


def addresses(build):
    """{reference rva: rva in build} recorded for a non-reference build (DATA/sites/<build>.json)."""
    path = os.path.join(DATA, "sites", build + ".json")
    if not os.path.exists(path):
        return {}
    return {int(k, 16): v for k, v in json.load(open(path)).items()}


def report(image, spec, recorded, layout_checked):
    got = resolve_keys(image, spec, layout_checked)
    reasons = collections.Counter()
    wrong = []
    withheld = set()
    for k in spec["keys"]:
        g = got[k["rva"]]
        if not isinstance(g, int):
            withheld.add(k["rva"])
        if isinstance(g, int):
            reasons["verified " + k["kind"]] += 1
            exp = k["rva"] if recorded is None else recorded.get(k["rva"])
            if exp is not None and g != exp:
                wrong.append("%s: locates %#x, expected %#x" % (k["name"], g, exp))
        else:
            reasons["withheld %s: %s" % (k["kind"], g)] += 1
    fn, _, _, _ = locate(image, spec, layout_checked)
    for i, f in enumerate(spec["functions"]):
        if f.get("tail") and isinstance(fn[i], int):
            end = fn[i] + f["len"]
            if fingerprint(image.img[end:end + f["tail"]], f["tail_mask"]) != f["tail_hash"]:
                short = sum(1 for k in spec["keys"] if k["kind"] == "code" and k["index"] == i
                            and k["rva"] - f["start"] + 16 > f["len"])
                if short:
                    reasons["keys whose hook bytes run past a changed function end"] += short
                    wrong.append("%d hooks at the end of function %#x run past a changed tail" % (short, f["start"]))
    return reasons, wrong, withheld


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("generate")
    sub.add_parser("check")
    r = sub.add_parser("relocate")
    r.add_argument("--exe", required=True)
    r.add_argument("--build", required=True)
    a = p.parse_args(argv)
    if not DATA:
        p.error("set HOI4RE_DATA to the RE data directory or put hoi4re on PATH")
    reference = next(iter(BUILDS))
    images = {b: Image(exe(b), b) for b in BUILDS}
    ref = images[reference]
    if a.cmd == "generate":
        keys = {rva: name for rva, (name, _, _) in source_keys().items()}
        spec, problems = generate(keys, ref, [images[b] for b in BUILDS if b != reference], reference)
        with open(OUTPUT, "w", encoding="utf-8", newline="\n") as f:
            f.write(emit(spec, BUILDS[reference]))
        with open(os.path.join(DATA, "sites", "spec.json"), "w") as f:
            json.dump(spec, f)
        for line in problems:
            print("problem:", line)
        print("%d keys, %d functions, %d vtables, %d globals; %d problems" % (
            len(spec["keys"]), len(spec["functions"]), len(spec["vtables"]), len(spec["data"]), len(problems)))
        if problems:
            raise SystemExit(1)
        a.cmd = "check"
    spec = json.load(open(os.path.join(DATA, "sites", "spec.json")))
    if a.cmd == "check":
        failed = False
        with open(OUTPUT, encoding="utf-8") as f:
            if f.read() != emit(spec, BUILDS[reference]):
                print("FAIL generated.rs does not match the spec; run tools/sites.py generate")
                failed = True
        unwitnessed = {k["rva"] for k in spec["keys"]
                       if k["kind"] == "vtable" and not spec["vtables"][k["index"]]["witnesses"]}
        print("%d vtable keys have no witness and are withheld on builds whose layouts were not checked"
              % len(unwitnessed))
        names = {k["rva"]: k["name"] for k in spec["keys"]}
        source = set(source_keys())
        spec_keys = {k["rva"] for k in spec["keys"]}
        for rva in sorted(source - spec_keys):
            print("FAIL key %#x is not generated; run tools/sites.py generate" % rva)
            failed = True
        checked_builds = {b for b in BUILDS if hashlib.sha256(open(exe(b), "rb").read()).digest() in layout_checked()}
        if reference not in checked_builds:
            print("FAIL the reference build %s is not in engine_sites::LAYOUT_CHECKED" % BUILDS[reference])
            failed = True
        for b in BUILDS:
            recorded = None if b == reference else addresses(b)
            for checked in ((True, False) if b in checked_builds else (False,)):
                expected = set() if checked else set(EXPECTED_WITHHELD.get(b, ())) | unwitnessed
                reasons, wrong, withheld = report(images[b], spec, recorded, checked)
                print("%s (%s), layouts %s:" % (BUILDS[b], b, "checked" if checked else "by witnesses"))
                for k, n in sorted(reasons.items()):
                    print("  %5d %s" % (n, k))
                for line in wrong:
                    print("  WRONG", line)
                for rva in sorted(withheld - expected):
                    print("  FAIL withholds %s (%#x)" % (names[rva], rva))
                for rva in sorted(expected - withheld):
                    print("  FAIL no longer withholds %s (%#x)" % (names.get(rva, "?"), rva))
                failed = failed or bool(wrong) or withheld != expected
        raise SystemExit(1 if failed else 0)
    image = Image(a.exe)
    got = resolve_keys(image, spec, layout_checked=False)
    os.makedirs(os.path.join(DATA, "sites"), exist_ok=True)
    with open(os.path.join(DATA, "sites", a.build + ".json"), "w") as f:
        json.dump({"%#x" % k: v for k, v in got.items() if isinstance(v, int)}, f, indent=0)
    print(collections.Counter("verified" if isinstance(v, int) else v for v in got.values()))


if __name__ == "__main__":
    main(sys.argv[1:])
