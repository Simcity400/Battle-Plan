#!/usr/bin/env python3
"""hoi4re: query the game decompile index and the shared knowledge layer (names, notes, classes, fields).

Stdlib only. Usage: `hoi4re.py <command> ...`; see README.md.
"""
import argparse, collections, datetime, json, os, re, sqlite3, sys

HERE = os.path.dirname(os.path.realpath(__file__))
sys.path.insert(0, HERE)


def env(key, default):
    return os.environ.get(key, default)


def main_checkout():
    """Root of the main checkout, also when this file runs from a git worktree (worktrees do not hold the data)."""
    root = os.path.dirname(os.path.dirname(HERE))
    git = os.path.join(root, ".git")
    if os.path.isfile(git):
        target = open(git, encoding="utf-8").read().split("gitdir:", 1)[1].strip().replace("/", os.sep)
        root = target.split(os.sep + ".git" + os.sep)[0]
    return root


REPO = env("HOI4RE_REPO", main_checkout())
DATA = env("HOI4RE_DATA", os.path.join(REPO, "tools", "re", "data"))
KNOWLEDGE = os.path.join(DATA, "knowledge.sqlite3")
EXPORTS = os.path.join(DATA, "ghidra_exports")
REGISTRY = os.path.join(DATA, "hoi4_re_server", "registry.sqlite3")
EXE = env("HOI4RE_EXE", r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")
BUILD = env("HOI4RE_BUILD", "a729")
SUPPLEMENT = os.path.join(DATA, "builds", BUILD, "index.sqlite3")
INDEX = os.path.join(DATA, "function_index.sqlite3") if BUILD == "a729" else SUPPLEMENT
BASE = 0x140000000
RANK = {"auto": 0, "derived": 1, "low": 2, "medium": 3, "high": 4, "verified": 5}
LABEL = re.compile(r"\b((?:thunk_|PTR_)?(?:FUN|DAT|LAB|PTR|s|u)_(?:\w*?_)?)(1[0-9a-f]{8})\b")

SCHEMA = """
CREATE TABLE IF NOT EXISTS symbols(build TEXT, address INTEGER, kind TEXT, name TEXT, summary TEXT, confidence TEXT,
  source TEXT, evidence TEXT, author TEXT, updated_utc TEXT, PRIMARY KEY(build, address));
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(build, name COLLATE NOCASE);
CREATE TABLE IF NOT EXISTS notes(id INTEGER PRIMARY KEY, build TEXT, address INTEGER, text TEXT, evidence TEXT,
  source TEXT, author TEXT, created_utc TEXT);
CREATE INDEX IF NOT EXISTS notes_addr ON notes(build, address);
CREATE TABLE IF NOT EXISTS classes(build TEXT, mangled TEXT, name TEXT, bases_json TEXT, summary TEXT,
  PRIMARY KEY(build, mangled));
CREATE INDEX IF NOT EXISTS classes_name ON classes(build, name COLLATE NOCASE);
CREATE TABLE IF NOT EXISTS vslots(build TEXT, mangled TEXT, vtable INTEGER, col_offset INTEGER, slot INTEGER,
  address INTEGER, PRIMARY KEY(build, vtable, slot));
CREATE INDEX IF NOT EXISTS vslots_addr ON vslots(build, address);
CREATE INDEX IF NOT EXISTS vslots_class ON vslots(build, mangled);
CREATE TABLE IF NOT EXISTS fields(build TEXT, class TEXT, offset INTEGER, name TEXT, type TEXT, meaning TEXT,
  evidence TEXT, source TEXT, author TEXT, updated_utc TEXT, PRIMARY KEY(build, class, offset, name));
CREATE TABLE IF NOT EXISTS mentions(build TEXT, address INTEGER, source TEXT, line INTEGER, text TEXT,
  PRIMARY KEY(build, address, source, line));
CREATE TABLE IF NOT EXISTS sources(build TEXT, address INTEGER, path TEXT, PRIMARY KEY(build, address, path));
CREATE INDEX IF NOT EXISTS sources_path ON sources(build, path);
"""


def now():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class Re:
    def __init__(self):
        self._ix = self._kb = self._names = self._pe = None

    @property
    def ix(self):
        if self._ix is None:
            self._ix = sqlite3.connect("file:" + INDEX.replace("\\", "/") + "?mode=ro", uri=True)
            self.fts_tables = ["decompile_fts"]
            if os.path.exists(SUPPLEMENT) and os.path.abspath(SUPPLEMENT) != os.path.abspath(INDEX):
                # Functions the first index missed (mostly tiny accessors), exported later into a second file.
                self._ix.execute("ATTACH DATABASE ? AS sup", ("file:" + SUPPLEMENT.replace("\\", "/") + "?mode=ro",))
                for t, cols in (("functions", "address, name, body_min, body_max"), ("call_edges", "caller, callee")):
                    self._ix.execute("CREATE TEMP VIEW %s AS SELECT %s FROM main.%s UNION ALL SELECT %s FROM sup.%s" % (t, cols, t, cols, t))
                self.fts_tables.append("sup.decompile_fts")
        return self._ix

    def fts(self, query, limit, snippet=False):
        """(address, snippet or None) for an FTS5 query over every decompile table."""
        out = []
        for table in (self.ix and self.fts_tables):
            bare = table.split(".")[-1]
            col = "snippet(%s,1,'[',']','...',16)" % bare if snippet else "NULL"
            out += [(int(a, 16), sn) for a, sn in self.ix.execute("SELECT address, %s FROM %s WHERE %s MATCH ? LIMIT ?" % (col, table, bare), (query, limit - len(out)))]
            if len(out) >= limit:
                break
        return out

    @property
    def kb(self):
        if self._kb is None:
            self._kb = sqlite3.connect(KNOWLEDGE, timeout=30)
            self._kb.execute("PRAGMA journal_mode=WAL")
            self._kb.executescript(SCHEMA)
        return self._kb

    @property
    def pe(self):
        if self._pe is None:
            import rtti
            self._pe = rtti.PE(EXE)
        return self._pe

    # ---- lookups -------------------------------------------------------------------------------
    def names(self):
        if self._names is None:
            self._names = dict(self.kb.execute("SELECT address, name FROM symbols WHERE build=?", (BUILD,)))
        return self._names

    def label(self, a):
        n = self.names().get(a)
        if n:
            return "%s@%x" % (n, a)
        r = self.ix.execute("SELECT name FROM functions WHERE address=?", (a,)).fetchone()
        return r[0] if r else "0x%x" % a

    def resolve(self, text):
        t = text.strip()
        m = re.fullmatch(r"(?i)rva:?(?:0x)?([0-9a-f]+)", t)
        if m:
            return BASE + int(m.group(1), 16)
        m = re.fullmatch(r"(?i)(?:\w*?_)?(?:0x)?(1[0-9a-f]{8})", t) or re.fullmatch(r".*@(1[0-9a-f]{8})", t)
        if m:
            return int(m.group(1), 16)
        rows = self.kb.execute("SELECT address FROM symbols WHERE build=? AND name=? COLLATE NOCASE", (BUILD, t)).fetchall()
        if len(rows) == 1:
            return rows[0][0]
        r = self.ix.execute("SELECT address FROM functions WHERE name=?", (t,)).fetchone()
        if r:
            return r[0]
        hint = self.kb.execute("SELECT name FROM symbols WHERE build=? AND name LIKE ? LIMIT 8", (BUILD, "%" + t + "%")).fetchall()
        raise ValueError("cannot resolve %r%s" % (text, "; did you mean: " + ", ".join(h[0] for h in hint) if hint else ""))

    def containing(self, a):
        r = self.ix.execute("SELECT address FROM functions WHERE address<=? ORDER BY address DESC LIMIT 1", (a,)).fetchone()
        if r:
            ok = self.ix.execute("SELECT 1 FROM functions WHERE address=? AND body_max>=?", (r[0], a)).fetchone()
            return r[0] if ok else None

    def body(self, a):
        for table in (self.ix and self.fts_tables):
            schema = table.split(".")[0] + "." if "." in table else ""
            r = self.ix.execute("SELECT fts_rowid FROM %sfts_state WHERE address=?" % schema, (a,)).fetchone()
            if r:
                return self.ix.execute("SELECT content FROM %s WHERE rowid=?" % table, (r[0],)).fetchone()[0]
        return None

    def rewrite(self, text):
        names = self.names()

        def sub(m):
            n = names.get(int(m.group(2), 16))
            return "%s@%s" % (n, m.group(2)) if n else m.group(0)
        return LABEL.sub(sub, text)

    def class_of(self, a):
        """Class a function belongs to: from its name, else from a vtable it sits in alone."""
        n = self.names().get(a)
        if n and "::" in n:
            return n.rsplit("::", 1)[0]
        rows = self.kb.execute("SELECT DISTINCT c.name FROM vslots v JOIN classes c ON c.build=v.build AND c.mangled=v.mangled "
                               "WHERE v.build=? AND v.address=? LIMIT 2", (BUILD, a)).fetchall()
        return rows[0][0] if len(rows) == 1 else None

    def lineage(self, cls):
        """The class, its bases and its descendants (names)."""
        out = {cls}
        for name, bases in self.kb.execute("SELECT name, bases_json FROM classes WHERE build=?", (BUILD,)):
            bn = {self._cname(b) for b in json.loads(bases)}
            if name == cls:
                out |= bn
            elif cls in bn:
                out.add(name)
        return out

    def _cname(self, mangled):
        import rtti
        return rtti.demangle(mangled)

    # ---- writes --------------------------------------------------------------------------------
    def journal(self, rec):
        rec = dict(rec, build=BUILD, utc=now())
        with open(os.path.splitext(KNOWLEDGE)[0] + ".journal.jsonl", "a", encoding="utf-8") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")

    def set_name(self, a, name, summary="", confidence="medium", source="agent", evidence="", author="", kind=None, log=True):
        if confidence not in RANK:
            raise ValueError("confidence must be one of " + "/".join(RANK))
        capped = ""
        if source == "agent":
            self.need_author(author)
            if RANK[confidence] > RANK["medium"] and env("HOI4RE_TRUSTED", "") != "1":
                confidence, capped = "medium", " (confidence capped at medium; high and verified need HOI4RE_TRUSTED=1)"
            if self.kb.execute("SELECT 1 FROM vslots WHERE build=? AND address=? LIMIT 1", (BUILD, a)).fetchone():
                self.virtual_named = True
        if not re.fullmatch(r"[A-Za-z_~][\w:~<>,*&\-\[\]]*", name):
            raise ValueError("bad name %r" % name)
        kind = kind or ("function" if self.ix.execute("SELECT 1 FROM functions WHERE address=?", (a,)).fetchone() else "data")
        old = self.kb.execute("SELECT name, confidence, summary FROM symbols WHERE build=? AND address=?", (BUILD, a)).fetchone()
        if log:
            self.journal({"op": "name", "target": "0x%x" % a, "name": name, "summary": summary, "confidence": confidence,
                          "source": source, "evidence": evidence, "author": author, "kind": kind})
        if old and RANK[old[1]] > RANK[confidence]:
            if old[0] != name:
                self.add_note(a, "alternative name proposed (%s): %s. %s" % (confidence, name, summary), evidence, source, author, log=False)
            return "kept %s (%s outranks %s); proposal saved as a note" % (old[0], old[1], confidence)
        self.kb.execute("INSERT OR REPLACE INTO symbols VALUES(?,?,?,?,?,?,?,?,?,?)",
                        (BUILD, a, kind, name, summary or (old[2] if old else ""), confidence, source, evidence, author, now()))
        self.kb.commit()
        self._names = None
        return "0x%x = %s%s%s" % (a, name, " (was %s)" % old[0] if old and old[0] != name else "", capped)

    def need_author(self, author):
        if not author:
            raise ValueError("an author is required: pass --author or set HOI4RE_AUTHOR (it is how work is reviewed)")

    def after_writes(self):
        """Naming a virtual names every override of its slot."""
        if getattr(self, "virtual_named", False):
            import derive
            self.virtual_named = False
            return "\n" + derive.virtuals(self)
        return ""

    def add_note(self, a, text, evidence="", source="agent", author="", log=True):
        if source == "agent" and log:
            self.need_author(author)
        if log:
            self.journal({"op": "note", "target": "0x%x" % a, "text": text, "evidence": evidence, "source": source, "author": author})
        dup = self.kb.execute("SELECT 1 FROM notes WHERE build=? AND address=? AND text=?", (BUILD, a, text)).fetchone()
        if not dup:
            self.kb.execute("INSERT INTO notes(build,address,text,evidence,source,author,created_utc) VALUES(?,?,?,?,?,?,?)",
                            (BUILD, a, text, evidence, source, author, now()))
            self.kb.commit()
        return "note saved on 0x%x" % a

    def set_field(self, cls, offset, name, type_="", meaning="", evidence="", source="agent", author="", log=True):
        if source == "agent":
            self.need_author(author)
        if log:
            self.journal({"op": "field", "cls": cls, "offset": "0x%x" % offset, "name": name, "type": type_,
                          "meaning": meaning, "evidence": evidence, "source": source, "author": author})
        self.kb.execute("INSERT OR REPLACE INTO fields VALUES(?,?,?,?,?,?,?,?,?,?)",
                        (BUILD, cls, offset, name, type_, meaning, evidence, source, author, now()))
        self.kb.commit()
        return "%s+0x%x = %s" % (cls, offset, name)


R = Re()


def hexint(s):
    return int(str(s), 16) if str(s).lower().startswith("0x") else int(str(s), 0 if str(s).isdigit() else 16)


# ---- commands: each returns text -----------------------------------------------------------------
def cmd_status():
    ix, kb = R.ix, R.kb
    meta = dict(ix.execute("SELECT key, value FROM meta"))
    out = ["build %s  exe sha256 %s" % (BUILD, meta.get("executable_sha256", "?")),
           "index %s: %d functions, %d call edges" % (INDEX, ix.execute("SELECT count(*) FROM functions").fetchone()[0],
                                                     ix.execute("SELECT count(*) FROM call_edges").fetchone()[0]),
           "knowledge %s:" % KNOWLEDGE]
    for t in ("symbols", "notes", "classes", "vslots", "fields", "mentions", "sources"):
        out.append("  %-9s %d" % (t, kb.execute("SELECT count(*) FROM %s WHERE build=?" % t, (BUILD,)).fetchone()[0]))
    for conf, n in kb.execute("SELECT confidence, count(*) FROM symbols WHERE build=? GROUP BY 1", (BUILD,)):
        out.append("  names at %-8s %d" % (conf, n))
    return "\n".join(out)


def cmd_fn(target, raw=False, body=True, max_lines=0):
    a = R.resolve(target)
    f = R.ix.execute("SELECT name, body_min, body_max FROM functions WHERE address=?", (a,)).fetchone()
    if not f:
        c = R.containing(a)
        if c is None:
            return cmd_data(target)
        return "0x%x is inside %s\n\n" % (a, R.label(c)) + cmd_fn("%x" % c, raw, body, max_lines)
    sym = R.kb.execute("SELECT name, summary, confidence, source, evidence FROM symbols WHERE build=? AND address=?", (BUILD, a)).fetchone()
    out = ["== %s  [%s]  va 0x%x  rva 0x%x  size %d" % (sym[0] if sym else f[0], f[0], a, a - BASE, f[2] - f[1] + 1)]
    if sym:
        out.append("name: %s confidence, source %s%s" % (sym[2], sym[3], ", evidence " + sym[4] if sym[4] else ""))
        if sym[1]:
            out.append("summary: " + sym[1])
    vs = R.kb.execute("SELECT c.name, v.vtable, v.slot, v.col_offset FROM vslots v JOIN classes c ON c.build=v.build AND c.mangled=v.mangled "
                      "WHERE v.build=? AND v.address=? ORDER BY length(c.bases_json) LIMIT 200", (BUILD, a)).fetchall()
    if vs:
        out.append("virtual: " + ", ".join("%s slot %d (vtable 0x%x)" % (c, s, v) for c, v, s, _ in vs[:6]) +
                   (" and %d more" % (len(vs) - 6) if len(vs) > 6 else ""))
    src = [r[0] for r in R.kb.execute("SELECT path FROM sources WHERE build=? AND address=? ORDER BY path NOT LIKE '%.cpp'", (BUILD, a))]
    if src:
        out.append("source files (from assert strings): " + ", ".join(src[:6]))
    callers = [r[0] for r in R.ix.execute("SELECT caller FROM call_edges WHERE callee=?", (a,))]
    callees = [r[0] for r in R.ix.execute("SELECT callee FROM call_edges WHERE caller=?", (a,))]
    if not sym or "::" not in sym[0]:
        tally = collections.Counter(filter(None, (R.class_of(n) for n in list(dict.fromkeys(callers + callees))[:300])))
        if tally:
            out.append("neighbour classes: " + ", ".join("%s %d" % kv for kv in tally.most_common(4)))
        near = set((callers + callees)[:2000])
        files = collections.Counter(p for x, p in R.kb.execute("SELECT address, path FROM sources WHERE build=? AND path LIKE '%.cpp'", (BUILD,)) if x in near) if not src and near else None
        if files:
            out.append("neighbour source files: " + ", ".join("%s %d" % kv for kv in files.most_common(4)))
    for (t, ev, src, au, utc) in R.kb.execute("SELECT text, evidence, source, author, created_utc FROM notes WHERE build=? AND address=? ORDER BY id", (BUILD, a)):
        out.append("note (%s %s %s): %s%s" % (src, au, utc[:10], t[:1500], "  [" + ev + "]" if ev else ""))
    for (s, l, t) in R.kb.execute("SELECT source, line, text FROM mentions WHERE build=? AND address=? LIMIT 12", (BUILD, a)):
        out.append("mentioned %s:%d  %s" % (s, l, t))
    for title, lst in (("callers", callers), ("callees", callees)):
        out.append("%s (%d): %s%s" % (title, len(lst), ", ".join(R.label(x) for x in lst[:20]), " ..." if len(lst) > 20 else ""))
    text = R.body(a)
    cls = R.class_of(a)
    if cls and text:
        seen = {int(h, 16) for h in re.findall(r"\b0x([0-9a-f]{1,5})\b", text)}
        family = R.lineage(cls)
        rows = [r for r in R.kb.execute("SELECT class, offset, name, type, meaning FROM fields WHERE build=? ORDER BY offset", (BUILD,))
                if r[0] in family and r[1] in seen and r[1] >= 8]
        for c, o, n, t, m in rows[:40]:
            out.append("field? %s+0x%x %s %s  %s" % (c, o, n, t or "", (m or "")[:140]))
    if body:
        out.append("---- decompile ----")
        if text is None:
            out.append("(no decompile body in the index)")
        else:
            text = text if raw else R.rewrite(text)
            lines = text.splitlines()
            while lines and (not lines[0].strip() or lines[0].startswith("// ")):  # export header, already shown above
                lines.pop(0)
            if max_lines and len(lines) > max_lines:
                lines = lines[:max_lines] + ["... %d more lines (raise max_lines)" % (len(lines) - max_lines)]
            out += lines
    return "\n".join(out)


def cmd_asm(target):
    a = R.resolve(target)
    r = R.ix.execute("SELECT canonical_relpath FROM artifacts WHERE address=? AND kind='instructions'", (a,)).fetchone()
    if not r:
        return "no instruction listing for 0x%x" % a
    return R.rewrite(open(os.path.join(EXPORTS, r[0].replace("/", os.sep)), encoding="utf-8", errors="replace").read())


def cmd_search(query, limit=30):
    """FTS5 query over decompiled bodies. Hex addresses are plain tokens: `143316e90` finds every reference to DAT_143316e90."""
    rows = R.fts(query, limit, snippet=True)
    out = ["%s :: %s" % (R.label(a), R.rewrite(" ".join(s.split()))) for a, s in rows]
    return "\n".join(out + ["%d rows%s" % (len(rows), " (limit reached)" if len(rows) == limit else "")])


def cmd_find(pattern, limit=50):
    """Names, classes, fields and source files matching a substring."""
    like = "%" + pattern + "%"
    out = []
    for a, k, n, c, s in R.kb.execute("SELECT address, kind, name, confidence, summary FROM symbols WHERE build=? AND (name LIKE ? OR summary LIKE ?) "
                                      "ORDER BY length(name) LIMIT ?", (BUILD, like, like, limit)):
        out.append("%-8s 0x%x %s (%s) %s" % (k, a, n, c, (s or "")[:120]))
    for n, in R.kb.execute("SELECT DISTINCT name FROM classes WHERE build=? AND name LIKE ? ORDER BY length(name) LIMIT ?", (BUILD, like, limit)):
        out.append("class    " + n)
    for c, o, n, m in R.kb.execute("SELECT class, offset, name, meaning FROM fields WHERE build=? AND (name LIKE ? OR meaning LIKE ?) LIMIT ?", (BUILD, like, like, limit)):
        out.append("field    %s+0x%x %s  %s" % (c, o, n, (m or "")[:120]))
    for path, n in R.kb.execute("SELECT path, count(*) FROM sources WHERE build=? AND path LIKE ? GROUP BY 1 ORDER BY 2 DESC LIMIT 15", (BUILD, like)):
        some = [R.label(a) for (a,) in R.kb.execute("SELECT address FROM sources WHERE build=? AND path=? LIMIT 12", (BUILD, path))]
        out.append("source   %s (%d functions): %s" % (path, n, ", ".join(some)))
    return "\n".join(out) or "nothing matches %r" % pattern


def _walk(a, column, other, depth):
    out, seen = [], {a}

    def go(x, d, pad):
        rows = [r[0] for r in R.ix.execute("SELECT %s FROM call_edges WHERE %s=?" % (column, other), (x,))]
        for c in rows[:40]:
            out.append(pad + R.label(c) + (" (seen)" if c in seen else ""))
            if c not in seen and d < depth:
                seen.add(c)
                go(c, d + 1, pad + "  ")
        if len(rows) > 40:
            out.append(pad + "... %d more" % (len(rows) - 40))
    go(a, 1, "  ")
    return out


def cmd_callers(target, depth=1):
    a = R.resolve(target)
    out = _walk(a, "caller", "callee", depth)
    if not out:
        vs = R.kb.execute("SELECT count(*) FROM vslots WHERE build=? AND address=?", (BUILD, a)).fetchone()[0]
        out = ["  none direct" + ("; it is a virtual (see `fn`), so callers go through a vtable" if vs else "; try `xrefs` for data references")]
    return "callers of %s\n" % R.label(a) + "\n".join(out)


def cmd_callees(target, depth=1):
    a = R.resolve(target)
    return "callees of %s\n" % R.label(a) + "\n".join(_walk(a, "callee", "caller", depth) or ["  none"])


def cmd_path(source, target, max_depth=8):
    a, b = R.resolve(source), R.resolve(target)
    prev, frontier = {a: None}, [a]
    for _ in range(max_depth):
        nxt = []
        for x in frontier:
            for (c,) in R.ix.execute("SELECT callee FROM call_edges WHERE caller=?", (x,)):
                if c not in prev:
                    prev[c] = x
                    nxt.append(c)
        if b in prev or not nxt:
            break
        frontier = nxt
    if b not in prev:
        return "no direct-call path within %d calls (virtual calls are not edges)" % max_depth
    chain = []
    while b is not None:
        chain.append(R.label(b))
        b = prev[b]
    return " -> ".join(reversed(chain))


def cmd_xrefs(target, limit=60):
    """Every function whose decompile mentions the address (data, vtable, string or function)."""
    a = R.resolve(target)
    return "references to %s\n" % R.label(a) + cmd_search('"%x"' % a, limit)


def cmd_strings(text, limit=40):
    """Find a literal in the executable and the functions that use it."""
    out, d, pe = [], R.pe.data, R.pe
    for enc in (text.encode("latin1", "ignore"), text.encode("utf-16-le")):
        pos = d.find(enc)
        while pos >= 0 and len(out) < limit:
            for _n, rva, vs, ro, rs, _x in pe.sections:
                if ro <= pos < ro + rs:
                    va = pe.base + rva + pos - ro
                    start = d.rfind(b"\0", max(ro, pos - 200), pos) + 1
                    whole = d[start:d.find(b"\0", pos)][:120].decode("latin1", "replace")
                    sva = va - (pos - start)
                    users = [a for a, _s in R.fts('"%x"' % sva, 12)]
                    words = " ".join(re.findall(r"[0-9A-Za-z]+", whole))
                    if not users and words:  # Ghidra often inlines the literal instead of its address
                        users = [a for a, _s in R.fts('"%s"' % words, 12)]
                    out.append("0x%x %r used by: %s" % (sva, whole, ", ".join(R.label(u) for u in users) or "-"))
            pos = d.find(enc, pos + 1)
    return "\n".join(dict.fromkeys(out)) or "literal not found in the executable"


def cmd_data(target, size=64):
    """Bytes of initialised data at an address, with qwords labelled when they point at known symbols."""
    import struct
    a = R.resolve(target)
    raw = R.pe.read(a, size)
    if raw is None:
        return "0x%x is not file-backed (uninitialised .data/.bss or outside the image); name: %s" % (a, R.label(a))
    out = ["data at %s" % R.label(a)]
    for i in range(0, len(raw) - 7, 8):
        q, = struct.unpack_from("<Q", raw, i)
        f4, = struct.unpack_from("<f", raw, i)
        tag = R.label(q) if BASE <= q < BASE + 0x8000000 else "f32 %.6g" % f4
        out.append("  +0x%02x  %s  %016x  %s" % (i, raw[i:i + 8].hex(" "), q, tag))
    return "\n".join(out)


def cmd_class(name):
    """Bases, vtables, known fields and named methods of a class."""
    rows = R.kb.execute("SELECT mangled, name, bases_json, summary FROM classes WHERE build=? AND name=? COLLATE NOCASE", (BUILD, name)).fetchall()
    out = []
    for mangled, n, bases, summary in rows:
        out.append("class %s  (%s)%s" % (n, mangled, "  " + summary if summary else ""))
        out.append("  bases: " + (", ".join(R._cname(b) for b in json.loads(bases)) or "-"))
        for vt, off, cnt in R.kb.execute("SELECT vtable, col_offset, count(*) FROM vslots WHERE build=? AND mangled=? GROUP BY 1,2", (BUILD, mangled)):
            out.append("  vtable 0x%x (rva 0x%x) at object +0x%x, %d slots" % (vt, vt - BASE, off, cnt))
    name = rows[0][1] if rows else name
    for o, n, t, m in R.kb.execute("SELECT offset, name, type, meaning FROM fields WHERE build=? AND class=? COLLATE NOCASE ORDER BY offset", (BUILD, name)):
        out.append("  +0x%04x %-28s %-8s %s" % (o, n, t or "", (m or "")[:160]))
    for a, n, s in R.kb.execute("SELECT address, name, summary FROM symbols WHERE build=? AND name LIKE ? AND source<>'rtti' ORDER BY name LIMIT 80", (BUILD, name + "::%")):
        out.append("  method 0x%x %s  %s" % (a, n, (s or "")[:120]))
    return "\n".join(out) or "unknown class %r; try `find`" % name


def cmd_vtable(name, vtable=""):
    """Slots of a class's vtables (or of one vtable address)."""
    if vtable:
        where, args = "v.vtable=?", (BUILD, R.resolve(vtable))
    else:
        where, args = "c.name=? COLLATE NOCASE", (BUILD, name)
    out, last = [], None
    for cn, vt, slot, a in R.kb.execute("SELECT c.name, v.vtable, v.slot, v.address FROM vslots v JOIN classes c ON c.build=v.build AND c.mangled=v.mangled "
                                        "WHERE v.build=? AND " + where + " ORDER BY v.vtable, v.slot", args):
        if vt != last:
            out.append("%s vtable 0x%x" % (cn, vt))
            last = vt
        out.append("  [%3d] +0x%03x %s" % (slot, slot * 8, R.label(a)))
    return "\n".join(out) or "no vtable for %r" % (vtable or name)


def cmd_field_users(cls, offset, limit=40):
    """Functions that touch an offset, those tied to the class (or its bases and descendants) first."""
    off = hexint(offset)
    family = R.lineage(cls)
    hits = [a for a, _s in R.fts('"0x%x"' % off, 5000)]
    tied = [a for a in hits if R.class_of(a) in family]
    known = R.kb.execute("SELECT name, meaning FROM fields WHERE build=? AND class=? COLLATE NOCASE AND offset=?", (BUILD, cls, off)).fetchall()
    out = ["%s+0x%x %s" % (cls, off, "; ".join("%s: %s" % k for k in known) or "(no field recorded)"),
           "%d functions mention 0x%x; %d are tied to %s:" % (len(hits), off, len(tied), cls)]
    out += ["  " + R.label(a) for a in tied[:limit]]
    if len(tied) < limit:
        out.append("others (unattributed, first %d):" % (limit - len(tied)))
        out += ["  " + R.label(a) for a in [h for h in hits if h not in set(tied)][:limit - len(tied)]]
    return "\n".join(out)


def cmd_name(target, name, summary="", confidence="medium", evidence="", author=""):
    """Name a function, global or vtable. Shown everywhere afterwards, including inside other functions' bodies."""
    return R.set_name(R.resolve(target), name, summary, confidence, "agent", evidence, author or env("HOI4RE_AUTHOR", "")) + R.after_writes()


def cmd_note(target, text, evidence="", author=""):
    return R.add_note(R.resolve(target), text, evidence, "agent", author or env("HOI4RE_AUTHOR", ""))


def cmd_field(cls, offset, name, type="", meaning="", evidence="", author=""):
    """Record a struct field: class, offset, name, type, meaning."""
    return R.set_field(cls, hexint(offset), name, type, meaning, evidence, "agent", author or env("HOI4RE_AUTHOR", ""))


def cmd_batch(items):
    """Save many findings at once. items: JSON list (or JSONL text) of {op:name|note|field, ...same keys as the single commands}."""
    if isinstance(items, str):
        items = json.loads(items) if items.lstrip().startswith("[") else [json.loads(l) for l in items.splitlines() if l.strip()]
    out = []
    for it in items:
        it = dict(it)
        op, author = it.pop("op"), it.pop("author", "") or env("HOI4RE_AUTHOR", "")
        src, ev = it.pop("source", "agent"), it.pop("evidence", "")
        try:
            if op == "name":
                out.append(R.set_name(R.resolve(it["target"]), it["name"], it.get("summary", ""), it.get("confidence", "medium"), src, ev, author, it.get("kind")))
            elif op == "note":
                out.append(R.add_note(R.resolve(it["target"]), it["text"], ev, src, author))
            else:
                out.append(R.set_field(it["cls"], hexint(it["offset"]), it["name"], it.get("type", ""), it.get("meaning", ""), ev, src, author))
        except Exception as e:
            out.append("FAILED %s %s: %s" % (op, it, e))
    return "\n".join(out) + R.after_writes()


def cmd_export(path=""):
    """Dump agent-written knowledge (names, notes, fields) as JSONL; seeds are reproducible and left out."""
    path = path or os.path.splitext(KNOWLEDGE)[0] + ".export.jsonl"
    n = 0
    with open(path, "w", encoding="utf-8") as f:
        for a, k, nm, s, c, src, ev, au in R.kb.execute("SELECT address,kind,name,summary,confidence,source,evidence,author FROM symbols WHERE build=? AND source<>'rtti'", (BUILD,)):
            f.write(json.dumps({"op": "name", "target": "0x%x" % a, "name": nm, "summary": s, "confidence": c, "evidence": ev, "author": au, "source": src, "kind": k}) + "\n"); n += 1
        for a, t, ev, src, au in R.kb.execute("SELECT address,text,evidence,source,author FROM notes WHERE build=?", (BUILD,)):
            f.write(json.dumps({"op": "note", "target": "0x%x" % a, "text": t, "evidence": ev, "author": au, "source": src}) + "\n"); n += 1
        for c, o, nm, t, m, ev, src, au in R.kb.execute("SELECT class,offset,name,type,meaning,evidence,source,author FROM fields WHERE build=?", (BUILD,)):
            f.write(json.dumps({"op": "field", "cls": c, "offset": "0x%x" % o, "name": nm, "type": t, "meaning": m, "evidence": ev, "author": au, "source": src}) + "\n"); n += 1
    return "%d rows -> %s" % (n, path)


# name, function, [(param, type, required, help)]
COMMANDS = [
    ("status", cmd_status, [], "Index and knowledge counts."),
    ("fn", cmd_fn, [("target", str, True, "address (VA, FUN_x, rva:x) or name"), ("raw", bool, False, "leave FUN_/DAT_ labels unrewritten"),
                    ("body", bool, False, "include the decompile (default true)"), ("max_lines", int, False, "truncate the body")],
     "Everything known about a function: name, summary, class, notes, mentions, callers, callees, matching fields, decompile."),
    ("asm", cmd_asm, [("target", str, True, "address or name")], "Instruction listing with cross references."),
    ("search", cmd_search, [("query", str, True, "FTS5 query, e.g. '\"instant_build\"' or 'NEAR(supply homebase)' or a bare hex address"), ("limit", int, False, "")],
     cmd_search.__doc__),
    ("find", cmd_find, [("pattern", str, True, "substring"), ("limit", int, False, "")], cmd_find.__doc__),
    ("callers", cmd_callers, [("target", str, True, ""), ("depth", int, False, "levels, default 1")], "Direct callers, optionally as a tree."),
    ("callees", cmd_callees, [("target", str, True, ""), ("depth", int, False, "levels, default 1")], "Direct callees, optionally as a tree."),
    ("path", cmd_path, [("source", str, True, ""), ("target", str, True, ""), ("max_depth", int, False, "")], "Shortest direct-call path between two functions."),
    ("xrefs", cmd_xrefs, [("target", str, True, "any address"), ("limit", int, False, "")], cmd_xrefs.__doc__),
    ("strings", cmd_strings, [("text", str, True, "literal to find"), ("limit", int, False, "")], cmd_strings.__doc__),
    ("data", cmd_data, [("target", str, True, "address"), ("size", int, False, "bytes, default 64")], cmd_data.__doc__),
    ("class", cmd_class, [("name", str, True, "class name, e.g. CCountry")], cmd_class.__doc__),
    ("vtable", cmd_vtable, [("name", str, True, "class name"), ("vtable", str, False, "one vtable address instead")], cmd_vtable.__doc__),
    ("field-users", cmd_field_users, [("cls", str, True, "class"), ("offset", str, True, "hex offset"), ("limit", int, False, "")], cmd_field_users.__doc__),
    ("name", cmd_name, [("target", str, True, "address"), ("name", str, True, "Class::Method or a global's name"), ("summary", str, False, "one line: what it does"),
                        ("confidence", str, False, "low|medium|high|verified (default medium)"), ("evidence", str, False, "what proves it: addresses, file:line, capture"),
                        ("author", str, False, "")], cmd_name.__doc__),
    ("note", cmd_note, [("target", str, True, "address"), ("text", str, True, "the finding"), ("evidence", str, False, ""), ("author", str, False, "")],
     "Attach a finding to an address."),
    ("field", cmd_field, [("cls", str, True, "class"), ("offset", str, True, "hex offset"), ("name", str, True, ""), ("type", str, False, "i32, i64 fixed5, CCountry*, ..."),
                          ("meaning", str, False, ""), ("evidence", str, False, ""), ("author", str, False, "")], cmd_field.__doc__),
    ("batch", cmd_batch, [("items", str, True, "JSON list or JSONL; CLI also accepts @file or - for stdin")], cmd_batch.__doc__),
    ("export", cmd_export, [("path", str, False, "output file")], cmd_export.__doc__),
]


def run_cli(argv):
    ap = argparse.ArgumentParser(prog="hoi4re", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name, _fn, params, help_ in COMMANDS:
        p = sub.add_parser(name, help=(help_ or "").split("\n")[0])
        for pn, pt, req, ph in params:
            if req:
                p.add_argument(pn, help=ph)
            elif pt is bool:
                p.add_argument("--" + pn.replace("_", "-"), dest=pn, action=argparse.BooleanOptionalAction, default=None, help=ph)
            else:
                p.add_argument("--" + pn.replace("_", "-"), dest=pn, type=pt, default=None, help=ph)
    for extra in ("seed", "derive", "migrate", "todo", "review"):
        sub.add_parser(extra, help="see tools/re/README.md", add_help=False)
    if argv and argv[0] in ("seed", "derive", "migrate"):
        mod = __import__(argv[0])
        return mod.main(R, argv[1:])
    if argv and argv[0] in ("todo", "review"):
        import work
        return work.main(R, argv[1:], argv[0])
    args = vars(ap.parse_args(argv))
    cmd = args.pop("cmd")
    if cmd == "batch":
        s = args["items"]
        args["items"] = sys.stdin.read() if s == "-" else open(s[1:], encoding="utf-8").read() if s.startswith("@") else s
    fn = {n: f for n, f, _p, _h in COMMANDS}[cmd]
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    print(fn(**{k: v for k, v in args.items() if v is not None}))


if __name__ == "__main__":
    try:
        run_cli(sys.argv[1:])
    except (ValueError, sqlite3.Error, OSError) as e:
        sys.exit("hoi4re: %s" % e)
