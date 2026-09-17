"""MSVC x64 RTTI scan of a PE file: classes, base classes, vtables and their slots. Stdlib only."""
import struct


class PE:
    def __init__(self, path):
        self.data = open(path, "rb").read()
        d = self.data
        pe = struct.unpack_from("<I", d, 0x3C)[0]
        nsec, = struct.unpack_from("<H", d, pe + 6)
        optsz, = struct.unpack_from("<H", d, pe + 20)
        opt = pe + 24
        self.base, = struct.unpack_from("<Q", d, opt + 24)
        self.sections = []  # (name, rva, vsize, raw_off, raw_size, executable)
        for i in range(nsec):
            o = opt + optsz + 40 * i
            name = d[o:o + 8].rstrip(b"\0").decode("latin1")
            vsize, rva, rsize, roff = struct.unpack_from("<IIII", d, o + 8)
            flags, = struct.unpack_from("<I", d, o + 36)
            self.sections.append((name, rva, vsize, roff, rsize, bool(flags & 0x20000000)))

    def off(self, rva):
        for _, r, vs, ro, rs, _x in self.sections:
            if r <= rva < r + min(vs, rs):
                return ro + rva - r
        return None

    def read(self, va, size):
        """Initialised bytes at a virtual address, or None when outside file-backed sections."""
        o = self.off(va - self.base)
        return None if o is None else self.data[o:o + size]

    def is_code(self, va):
        rva = va - self.base
        return any(x and r <= rva < r + vs for _, r, vs, _ro, _rs, x in self.sections)

    def cstr(self, rva, limit=1024):
        o = self.off(rva)
        if o is None:
            return None
        end = self.data.find(b"\0", o, o + limit)
        return self.data[o:end].decode("latin1") if end >= 0 else None


def demangle(m):
    """'.?AVCData@NAcclimatization@@' -> 'NAcclimatization::CData'; templates keep their head only."""
    s = m[4:] if m[:4] in (".?AV", ".?AU") else m
    if s.startswith("?$"):
        return s[2:].split("@", 1)[0] + "<>"
    parts = [p for p in s.split("@") if p]
    if any(p.startswith("?") for p in parts):
        return s.rstrip("@")
    return "::".join(reversed(parts))


def scan(path):
    """Returns {'base', 'classes': {mangled: {name, bases[mangled]}}, 'vtables': [{class, va, col_offset, slots[va]}]}."""
    pe = PE(path)
    d, base = pe.data, pe.base
    classes, vtables = {}, []
    lo = base
    hi = base + max(r + vs for _, r, vs, _o, _s, _x in pe.sections)

    def u32s(rva, n):
        o = pe.off(rva)
        return None if o is None or o + 4 * n > len(d) else struct.unpack_from("<%dI" % n, d, o)

    def td_name(td_rva):
        n = pe.cstr(td_rva + 16)
        return n if n and n[:4] in (".?AV", ".?AU") else None

    for _name, rva, vs, ro, rs, x in pe.sections:
        if x:
            continue
        size = min(vs, rs) & ~7
        words = struct.unpack_from("<%dQ" % (size // 8), d, ro)
        for i, q in enumerate(words):
            if not (lo <= q < hi) or pe.is_code(q):
                continue
            col = u32s(q - base, 6)
            if not col or col[0] != 1 or col[5] != q - base:
                continue
            cname = td_name(col[3])
            if not cname:
                continue
            slots, j = [], i + 1
            while j < len(words) and pe.is_code(words[j]):
                slots.append(words[j])
                j += 1
            if not slots:
                continue
            if cname not in classes:
                bases = []
                chd = u32s(col[4], 4)
                if chd and chd[2] < 4096:
                    for bcd in (u32s(chd[3], chd[2]) or ())[1:]:
                        b = u32s(bcd, 1)
                        bn = td_name(b[0]) if b else None
                        if bn:
                            bases.append(bn)
                classes[cname] = {"name": demangle(cname), "bases": bases}
            vtables.append({"class": cname, "va": base + rva + 8 * (i + 1), "col_offset": col[1], "slots": slots})
    return {"base": base, "classes": classes, "vtables": vtables}


if __name__ == "__main__":
    import sys
    r = scan(sys.argv[1])
    print(len(r["classes"]), "classes", len(r["vtables"]), "vtables", sum(len(v["slots"]) for v in r["vtables"]), "slots")
