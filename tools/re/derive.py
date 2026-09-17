"""Names the scripts can work out alone: define globals, one-line accessors, and overrides of named virtuals.

Re-runnable like the seeders. Everything is written at confidence `derived`, above RTTI placeholders and below any
name an agent or the engine crate gave.
"""
import collections, json, os, re
import hoi4re as H
from seed import put_names

TYPES = {"undefined1": "u8", "undefined2": "u16", "undefined4": "u32", "undefined8": "u64", "char": "i8", "byte": "u8", "bool": "u8",
         "short": "i16", "ushort": "u16", "int": "i32", "uint": "u32", "longlong": "i64", "ulonglong": "u64", "float": "f32", "double": "f64"}
T = r"(?:\w+ )?(\w+) \*"
SHAPES = [  # kind, pattern over the single statement; groups: type, offset (or outer, inner for `via`)
    ("get", re.compile(r"^return \*\(%s\)\(param_1 \+ (0x[0-9a-f]+|\d+)\);$" % T)),
    ("get", re.compile(r"^\*param_2 = \*\(%s\)\(param_1 \+ (0x[0-9a-f]+|\d+)\); return param_2;$" % T)),
    ("set", re.compile(r"^\*\(%s\)\(param_1 \+ (0x[0-9a-f]+|\d+)\) = param_2; return;$" % T)),
    ("set", re.compile(r"^\*\(%s\)\(param_1 \+ (0x[0-9a-f]+|\d+)\) = (?:0|1); return;$" % T)),
    ("has", re.compile(r"^return \*\(%s\)\(param_1 \+ (0x[0-9a-f]+|\d+)\) != (?:0|'\\0');$" % T)),
    ("ptr", re.compile(r"^return ()param_1 \+ (0x[0-9a-f]+|\d+);$")),
]
INDEXED = [  # `param_1[n]` forms; the element size comes from param_1's declared type
    ("get", re.compile(r"^return param_1\[(0x[0-9a-f]+|\d+)\];$")),
    ("get", re.compile(r"^\*param_2 = param_1\[(0x[0-9a-f]+|\d+)\]; return param_2;$")),
    ("set", re.compile(r"^param_1\[(0x[0-9a-f]+|\d+)\] = (?:param_2|0|1); return;$")),
    ("has", re.compile(r"^return param_1\[(0x[0-9a-f]+|\d+)\] != 0;$")),
]
SIZES = {"u8": 1, "i8": 1, "u16": 2, "i16": 2, "u32": 4, "i32": 4, "f32": 4, "u64": 8, "i64": 8, "f64": 8}


def bodies(R, max_size=96):
    """(address, statements joined by a space) for small functions in every index."""
    small = [a for (a,) in R.ix.execute("SELECT address FROM functions WHERE body_max - body_min < ?", (max_size,))]
    for a in small:
        text = R.body(a)
        if text and "{" in text:
            inner = text[text.index("{") + 1:text.rindex("}")]
            first = re.search(r"\((?:\w+ )?(\w+) \*param_1[,)]", text[:text.index("{")])
            yield a, first.group(1) if first else "", " ".join(l.strip() for l in inner.splitlines() if l.strip() and not re.match(r"^\s*\w[\w \*]* \w+;$", l))


def accessors(R):
    kb = R.kb
    kb.execute("CREATE TABLE IF NOT EXISTS accessors(build TEXT, address INTEGER, kind TEXT, offset INTEGER, type TEXT, class TEXT, PRIMARY KEY(build, address))")
    kb.execute("DELETE FROM accessors WHERE build=?", (H.BUILD,))
    fields = {(c, o): n for c, o, n in kb.execute("SELECT class, offset, name FROM fields WHERE build=? ORDER BY source='agent'", (H.BUILD,))}
    rows = []
    for a, element, body in bodies(R):
        for kind, pat in SHAPES + INDEXED:
            m = pat.match(body)
            if not m:
                continue
            if pat.groups == 1:
                ty = TYPES.get(element, "")
                if ty not in SIZES:
                    break
                off = int(m.group(1), 0) * SIZES[ty]
            else:
                off, ty = int(m.group(2), 0), TYPES.get(m.group(1), m.group(1) or "")
            cls = R.class_of(a)
            field = fields.get((cls, off)) if cls else None
            kb.execute("INSERT OR REPLACE INTO accessors VALUES(?,?,?,?,?,?)", (H.BUILD, a, kind, off, ty, cls or ""))
            what = {"get": "returns", "set": "writes", "has": "tests", "ptr": "returns the address of"}[kind]
            tail = (field or "0x%x" % off) if cls else "%s0x%x" % (ty + "_" if ty else "", off)
            rows.append((a, "function", "%s%s_%s" % (cls + "::" if cls else "", kind, tail),
                         "%s %s at this+0x%x%s" % (what, ty or "bytes", off, " (%s)" % field if field else ""), "derived", "one-statement body"))
            break
    kb.commit()
    return "accessors: %d found, %d named" % (len(rows), put_names(R, "accessor", rows))


PLUMBING = [  # compiler-generated bodies that are not game logic
    ("unwind_funclet", "exception cleanup: destroys a local of the parent frame", re.compile(r"^(?:thunk_)?FUN_(1[0-9a-f]{8})\((?:\*\(undefined8 \*\)\()?param_2(?: \+ (?:0x[0-9a-f]+|\d+))?\)?(?:,[^;]*)?\); return;$")),
    ("adjustor_thunk", "adjusts `this` for a secondary base, then forwards", re.compile(r"^(?:thunk_)?FUN_(1[0-9a-f]{8})\(param_1 \+ -(?:0x[0-9a-f]+|\d+)\); return;$")),
    ("static_dtor", "atexit destructor of a static object", re.compile(r"^(?:thunk_)?FUN_(1[0-9a-f]{8})\(&(?:DAT|PTR)[A-Za-z_]*_1[0-9a-f]{8}\); return;$")),
]


def plumbing(R):
    """Label unwind funclets, adjustor thunks and static destructors so nobody spends time on them."""
    rows = []
    for a, _element, body in bodies(R):
        for name, what, pat in PLUMBING:
            m = pat.match(body)
            if m and (name == "adjustor_thunk" or not R.ix.execute("SELECT 1 FROM call_edges WHERE callee=? LIMIT 1", (a,)).fetchone()):
                rows.append((a, "function", name, "%s (calls %s)" % (what, R.label(int(m.group(1), 16))), "derived", "one-statement body"))
                break
    return "plumbing: %d compiler-generated functions labelled" % put_names(R, "plumbing", rows)



def lua_defines():
    """{'NS.KEY': 'value  -- comment'} from the game's own defines file."""
    path = os.path.join(os.path.dirname(H.EXE), "common", "defines", "00_defines.lua")
    out, ns = {}, None
    if os.path.exists(path):
        for line in open(path, encoding="utf-8", errors="replace"):
            m = re.match(r"^\s*(N\w+)\s*=\s*\{", line)
            if m:
                ns = m.group(1)
                continue
            m = re.match(r"^\s*([A-Z][A-Z0-9_]*)\s*=\s*(.+?)\s*$", line)
            if m and ns:
                out["%s.%s" % (ns, m.group(1))] = re.sub(r",\s*(--|$)", r" \1", m.group(2)).strip()[:300]
    return out


def defines(R):
    kb, lua = R.kb, lua_defines()
    rows, missed = [], 0
    helpers = kb.execute("SELECT c.name, v.slot, v.address FROM classes c JOIN vslots v ON v.build=c.build AND v.mangled=c.mangled "
                         "WHERE c.build=? AND c.name LIKE 'NDefines::CDefineRegistryHelper%' ORDER BY c.name, v.slot", (H.BUILD,)).fetchall()
    by_class = collections.defaultdict(dict)
    for name, slot, a in helpers:
        by_class[name][slot] = a
    for cls, slots in by_class.items():
        named = re.search(r'return "([^"]+)";', R.body(slots.get(1, 0)) or "")
        load = R.body(slots.get(0, 0)) or ""
        if not named:
            missed += 1
            continue
        ns, _, key = named.group(1).partition("::")
        target = re.search(r'"%s"\s*,\s*\(?[\w \*]*\)?&?\w*?_?(1[0-9a-f]{8})\b' % re.escape(key), load)
        doc = "define %s.%s%s" % (ns, key, " = " + lua["%s.%s" % (ns, key)] if "%s.%s" % (ns, key) in lua else "")
        rows.append((slots[1], "function", cls + "::GetName", doc, "derived", "returns the literal"))
        if 0 in slots:
            rows.append((slots[0], "function", cls + "::Load", "reads " + doc, "derived", "lua read of the key"))
        if target:
            rows.append((int(target.group(1), 16), "data", "NDefines::%s::%s" % (ns, key), doc, "derived", "written by %s::Load" % cls))
        else:
            missed += 1
    n = put_names(R, "define", rows)
    return "defines: %d helper classes, %d names written, %d without a resolved global" % (len(by_class), n, missed)


def ctors(R):
    """A function that stores a class's vftable into `this` is that class's constructor or destructor."""
    kb = R.kb
    bases = {n: {R._cname(b) for b in json.loads(bj)} for n, bj in kb.execute("SELECT name, bases_json FROM classes WHERE build=?", (H.BUILD,))}
    in_vtable = collections.defaultdict(set)
    for n, a in kb.execute("SELECT c.name, v.address FROM vslots v JOIN classes c ON c.build=v.build AND c.mangled=v.mangled WHERE v.build=?", (H.BUILD,)):
        in_vtable[a].add(n)
    write = re.compile(r"BS*(?:BS([^)]*BS))?param_1 = ((?:BSw+::)*BSw+)::vftable;".replace("BS", chr(92)))
    rows = []
    for table in (R.ix and R.fts_tables):
        for addr, body in R.ix.execute("SELECT address, content FROM %s WHERE %s MATCH 'vftable'" % (table, table.split(".")[-1])):
            a = int(addr, 16)
            order = [c for c in write.findall(body) if c in bases]
            top = [c for c in set(order) if all(c == o or o in bases[c] for o in order)]
            if len(top) != 1:
                continue
            cls = top[0]
            callers = {x for (x,) in R.ix.execute("SELECT caller FROM call_edges WHERE callee=?", (a,))}
            dtor = cls in in_vtable.get(a, ()) or (len(order) > 1 and order[0] == cls) or any(cls in in_vtable.get(x, ()) for x in callers)
            rows.append((a, "function", "%s::%s%s" % (cls, "~" if dtor else "", cls.rsplit("::", 1)[-1]),
                         "%s: stores %s::vftable into this" % ("destructor" if dtor else "constructor", cls), "derived", "vftable write"))
    return "ctors: %d constructors and destructors named" % put_names(R, "ctor", rows)


def virtuals(R):
    """A named virtual gives every override of the same slot the same method name."""
    kb = R.kb
    classes = {m: (n, json.loads(b)) for m, n, b in kb.execute("SELECT mangled, name, bases_json FROM classes WHERE build=?", (H.BUILD,))}
    primary = {}  # class -> slots of its first vtable at object offset 0
    for m, vt, slot, a in kb.execute("SELECT mangled, vtable, slot, address FROM vslots WHERE build=? AND col_offset=0 ORDER BY mangled, vtable, slot", (H.BUILD,)):
        first = primary.setdefault(m, (vt, []))
        if first[0] == vt:
            first[1].append(a)

    def root(m, n):
        while classes[m][1] and classes[m][1][0] in primary and len(primary[classes[m][1][0]][1]) > n:
            m = classes[m][1][0]
        return m

    keys = collections.defaultdict(set)      # function -> {(root class, slot)}
    members = collections.defaultdict(list)  # (root class, slot) -> [(class, function)]
    for m, (_vt, slots) in primary.items():
        for n, a in enumerate(slots):
            k = (root(m, n), n)
            keys[a].add(k)
            members[k].append((m, a))
    syms = {a: (n, c) for a, n, c in kb.execute("SELECT address, name, confidence FROM symbols WHERE build=? AND source NOT IN ('rtti','virtual','accessor','define','plumbing','ctor')", (H.BUILD,))}
    method = {}
    for a, (name, conf) in syms.items():
        if "::" in name and len(keys.get(a, ())) == 1 and H.RANK[conf] >= H.RANK["low"]:
            k = next(iter(keys[a]))
            best = method.get(k)
            if not best or H.RANK[conf] > best[1]:
                method[k] = (name.rsplit("::", 1)[1], H.RANK[conf], name)
    rows = []
    for k, (meth, _rank, origin) in method.items():
        holders = collections.defaultdict(set)
        for m, a in members[k]:
            holders[a].add(m)
        for a, held in holders.items():
            if a in syms or len(keys[a]) != 1:
                continue
            top = [c for c in held if all(c == o or c in classes[o][1] for o in held)]
            if len(top) == 1:
                rows.append((a, "function", "%s::%s" % (classes[top[0]][0], meth), "override of %s (vtable slot %d)" % (origin, k[1]), "derived", "same slot as " + origin))
    return "virtuals: %d named slots gave %d override names" % (len(method), put_names(R, "virtual", rows))


def main(R, argv):
    for w in argv or ["defines", "plumbing", "accessors", "ctors", "virtuals"]:
        print({"defines": defines, "plumbing": plumbing, "accessors": accessors, "ctors": ctors, "virtuals": virtuals}[w](R))
