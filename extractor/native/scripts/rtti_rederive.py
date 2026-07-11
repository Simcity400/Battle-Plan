#!/usr/bin/env python3
"""U1/O4: RTTI auto-re-derivation for build_profile vtable RVAs (patch-day prep).

Every `*_VTABLE_RVA` in compact_contract/src/build_profile.rs pins a class
vtable in a specific hoi4.exe build. Game updates move them; re-deriving by
hand is the long tail of every patch. MSVC x64 RTTI makes them recoverable
from the exe alone:

    vtable[-1] (qword) -> CompleteObjectLocator VA
    COL = {sig=1, offset, cdOffset, pTypeDescriptor(RVA), pCHD(RVA), pSelf(RVA)}
    TypeDescriptor+0x10 = ".?AVClassName@@" mangled name

Modes:
  --derive-map   On the CURRENT exe: resolve every constant's existing RVA to
                 its RTTI class name + COL offset, writing
                 scripts/rtti_vtable_map.json (the durable manifest; the COL
                 offset disambiguates multiple-inheritance vtables).
  --verify       SELF-PROOF: re-derive every constant from the manifest against
                 the CURRENT exe and assert byte-for-byte equality with
                 build_profile.rs. Exit 1 on any mismatch.
  --rederive EXE Patch day: re-derive against a NEW exe and print the updated
                 constants block + a diff report. FAIL-CLOSED: a class that is
                 missing, renamed, or ambiguous is listed as UNRESOLVED, never
                 guessed.

Read-only except for the manifest file in --derive-map.
"""
import argparse
import io
import json
import os
import re
import struct
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BUILD_PROFILE = os.path.join(REPO, "compact_contract", "src", "build_profile.rs")
MANIFEST = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "rtti_vtable_map.json")
DEFAULT_EXE = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe"


class Pe:
    def __init__(self, path):
        self.data = open(path, "rb").read()
        d = self.data
        pe = struct.unpack_from("<I", d, 0x3C)[0]
        assert d[pe:pe + 4] == b"PE\0\0", "not a PE"
        machine, nsec = struct.unpack_from("<HH", d, pe + 4)
        opt = pe + 24
        magic = struct.unpack_from("<H", d, opt)[0]
        assert magic == 0x20B, "not PE32+"
        self.image_base = struct.unpack_from("<Q", d, opt + 24)[0]
        opt_size = struct.unpack_from("<H", d, pe + 20)[0]
        sec = opt + opt_size
        self.sections = []
        for i in range(nsec):
            off = sec + i * 40
            name = d[off:off + 8].rstrip(b"\0").decode(errors="replace")
            vsize, vaddr, rsize, raddr = struct.unpack_from("<IIII", d, off + 8)
            self.sections.append((name, vaddr, vsize, raddr, rsize))

    def rva_to_off(self, rva):
        for _, vaddr, vsize, raddr, rsize in self.sections:
            if vaddr <= rva < vaddr + max(vsize, rsize):
                return raddr + (rva - vaddr)
        return None

    def off_to_rva(self, off):
        for _, vaddr, vsize, raddr, rsize in self.sections:
            if raddr <= off < raddr + rsize:
                return vaddr + (off - raddr)
        return None

    def read_u32(self, rva):
        off = self.rva_to_off(rva)
        return None if off is None else struct.unpack_from("<I", self.data, off)[0]

    def read_u64(self, rva):
        off = self.rva_to_off(rva)
        return None if off is None else struct.unpack_from("<Q", self.data, off)[0]

    def cstr(self, rva, max_len=256):
        off = self.rva_to_off(rva)
        if off is None:
            return None
        end = self.data.find(b"\0", off, off + max_len)
        if end < 0:
            return None
        try:
            return self.data[off:end].decode("ascii")
        except UnicodeDecodeError:
            return None

    def find_all(self, needle):
        d = self.data
        pos = d.find(needle)
        while pos >= 0:
            yield pos
            pos = d.find(needle, pos + 1)


def col_at(pe, col_rva):
    """Parse + sanity-check a CompleteObjectLocator at col_rva; None if bad."""
    off = pe.rva_to_off(col_rva)
    if off is None or off + 24 > len(pe.data):
        return None
    sig, offset, cd, td_rva, chd_rva, self_rva = struct.unpack_from(
        "<6I", pe.data, off)
    if sig != 1 or self_rva != col_rva:
        return None
    return {"offset": offset, "td_rva": td_rva}


def vtable_to_identity(pe, vtable_rva):
    """CURRENT-exe direction: vtable RVA -> (class name, COL offset)."""
    col_va = pe.read_u64(vtable_rva - 8)
    if col_va is None:
        return None
    col = col_at(pe, col_va - pe.image_base)
    if col is None:
        return None
    name = pe.cstr(col["td_rva"] + 0x10)
    if not name or not name.startswith(".?A"):
        return None
    return {"class": name, "col_offset": col["offset"]}


def identity_to_vtables(pe, class_name, col_offset):
    """NEW-exe direction: class name + COL offset -> candidate vtable RVAs."""
    needle = class_name.encode("ascii") + b"\0"
    tds = []
    for pos in pe.find_all(needle):
        td_off = pos - 0x10
        td_rva = pe.off_to_rva(td_off)
        if td_rva is not None:
            tds.append(td_rva)
    results = []
    for td_rva in tds:
        # find COLs whose pTypeDescriptor == td_rva (u32 at COL+12) with a
        # valid self-referential pSelf — scan for the raw u32 and validate.
        for pos in pe.find_all(struct.pack("<I", td_rva)):
            col_rva = pe.off_to_rva(pos - 12)
            if col_rva is None:
                continue
            col = col_at(pe, col_rva)
            if col is None or col["td_rva"] != td_rva:
                continue
            if col["offset"] != col_offset:
                continue
            # vtable = location whose preceding qword is the COL's VA
            col_va = pe.image_base + col_rva
            for vpos in pe.find_all(struct.pack("<Q", col_va)):
                vt_rva = pe.off_to_rva(vpos + 8)
                if vt_rva is not None:
                    results.append(vt_rva)
    return sorted(set(results))


def profile_constants():
    src = io.open(BUILD_PROFILE, encoding="utf-8").read()
    return {
        m.group(1): int(m.group(2), 16)
        for m in re.finditer(
            r"pub const ([A-Z0-9_]*VTABLE_RVA): usize = 0x([0-9A-Fa-f]+);", src)
    }


def cmd_derive_map(exe):
    pe = Pe(exe)
    consts = profile_constants()
    manifest, failed = {}, []
    for name, rva in sorted(consts.items()):
        ident = vtable_to_identity(pe, rva)
        if ident is None:
            failed.append(name)
            continue
        manifest[name] = {"class": ident["class"],
                          "col_offset": ident["col_offset"],
                          "derived_from_rva": hex(rva)}
        print(f"{name:56s} {hex(rva):>10s}  {ident['class']}"
              f"  (col_offset {ident['col_offset']})")
    if failed:
        print("\nUNRESOLVED (no RTTI identity — these need manual notes):")
        for name in failed:
            print("  ", name, hex(consts[name]))
    with open(MANIFEST, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
    print(f"\nmanifest: {MANIFEST} ({len(manifest)}/{len(consts)} resolved)")
    return 0 if not failed else 1


def rederive(exe):
    pe = Pe(exe)
    manifest = json.load(open(MANIFEST, encoding="utf-8"))
    resolved, unresolved = {}, {}
    for name, spec in sorted(manifest.items()):
        cands = identity_to_vtables(pe, spec["class"], spec["col_offset"])
        if len(cands) == 1:
            resolved[name] = cands[0]
        else:
            unresolved[name] = cands
    return resolved, unresolved


def cmd_verify(exe):
    consts = profile_constants()
    resolved, unresolved = rederive(exe)
    mismatch = []
    for name, rva in sorted(resolved.items()):
        current = consts.get(name)
        ok = current == rva
        if not ok:
            mismatch.append((name, current, rva))
        print(f"{name:56s} profile={hex(current) if current else '?':>10s} "
              f"rederived={hex(rva):>10s}  {'OK' if ok else 'MISMATCH'}")
    for name, cands in unresolved.items():
        print(f"{name:56s} UNRESOLVED candidates={[hex(c) for c in cands]}")
    total = len(consts)
    print(f"\nself-proof: {len(resolved) - len(mismatch)}/{total} exact, "
          f"{len(mismatch)} mismatched, {len(unresolved)} unresolved")
    return 0 if not mismatch and not unresolved and len(resolved) == total else 1


def cmd_rederive(exe):
    resolved, unresolved = rederive(exe)
    consts = profile_constants()
    print("// re-derived constants block (paste into build_profile.rs):")
    for name, rva in sorted(resolved.items()):
        changed = "" if consts.get(name) == rva else \
            f"  // CHANGED (was {hex(consts.get(name, 0))})"
        print(f"pub const {name}: usize = {hex(rva).upper().replace('0X','0x')};{changed}")
    if unresolved:
        print("\n// FAIL-CLOSED UNRESOLVED — re-derive manually, never guess:")
        for name, cands in sorted(unresolved.items()):
            print(f"//   {name}: candidates {[hex(c) for c in cands]}")
    return 0 if not unresolved else 1


def main():
    ap = argparse.ArgumentParser()
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--derive-map", action="store_true")
    mode.add_argument("--verify", action="store_true")
    mode.add_argument("--rederive", metavar="EXE")
    ap.add_argument("--exe", default=DEFAULT_EXE)
    args = ap.parse_args()
    if args.derive_map:
        sys.exit(cmd_derive_map(args.exe))
    if args.verify:
        sys.exit(cmd_verify(args.exe))
    sys.exit(cmd_rederive(args.rederive))


if __name__ == "__main__":
    main()
