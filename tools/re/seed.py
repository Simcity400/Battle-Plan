"""Seed the knowledge layer from what already exists: RTTI, the engine crate, docs and the old registry.

Every seeder is re-runnable: it deletes its own rows (by source) and re-inserts. Agent-written rows are never touched,
and a seeded name never replaces a name of higher confidence.
"""
import glob, json, os, re, sqlite3
import hoi4re as H
import rtti

ADDR = re.compile(r"\b(?:FUN_|thunk_FUN_|DAT_|0x)(1[0-9a-f]{8})\b")
RVA = re.compile(r"\bRVA ?`?0x([0-9a-fA-F_]{5,9})\b")
CONST = re.compile(r"^\s*pub const ([A-Z0-9_]+): *([A-Za-z0-9_]+) *= *(0x[0-9a-fA-F_]+|\d[\d_]*);")
CLASSY = re.compile(r"^`?((?:[CNS][A-Z]\w*)(?:::\w+)*)`?[ ,;:.'(]")
NOT_FIELD = re.compile(r"(^|_)(STRIDE|SIZE|LIMIT|MAX|MIN|CAP|BYTES|ROWS|MASK|SHIFT|SLOTS?|INDEX|TOKEN|VERSION|MAGIC)($|_)")
NAMED_A = re.compile(r"FUN_(1[0-9a-f]{8})`? ?(?:=|is|:) ?`?((?:[A-Z]\w*::)+~?\w+)")
NAMED_B = re.compile(r"((?:[A-Z]\w*::)+~?\w+)(?:\([^)]{0,40}\))?[` (=@:,]{0,5}(?:at |is )?[` (]{0,2}(?:FUN_|0x)(1[0-9a-f]{8})")


def put_names(R, source, rows):
    """rows: (address, kind, name, summary, confidence, evidence). Returns how many were written."""
    kb = R.kb
    kb.execute("DELETE FROM symbols WHERE build=? AND source=?", (H.BUILD, source))
    have = dict(kb.execute("SELECT address, confidence FROM symbols WHERE build=?", (H.BUILD,)))
    # Ghidra's own symbol names (imports, CRT, thunks of them) stay; widely shared virtuals get no single-class name.
    keep = {a for (a,) in R.ix.execute("SELECT address FROM functions WHERE name NOT LIKE 'FUN_%' AND name NOT LIKE 'thunk_FUN_%'")}
    if source != "rtti":
        keep |= {a for (a,) in kb.execute("SELECT address FROM vslots WHERE build=? GROUP BY address HAVING count(DISTINCT mangled) > 8", (H.BUILD,))}
    n = 0
    for a, kind, name, summary, conf, ev in rows:
        if a in keep:
            continue
        if a in have and H.RANK[have[a]] >= H.RANK[conf]:
            continue
        kb.execute("INSERT OR REPLACE INTO symbols VALUES(?,?,?,?,?,?,?,?,?,?)", (H.BUILD, a, kind, name, summary, conf, source, ev, "seed", H.now()))
        have[a] = conf
        n += 1
    kb.commit()
    R._names = None
    return n


def seed_rtti(R, exe=None):
    r = rtti.scan(exe or H.EXE)
    kb = R.kb
    kb.execute("DELETE FROM classes WHERE build=?", (H.BUILD,))
    kb.execute("DELETE FROM vslots WHERE build=?", (H.BUILD,))
    kb.executemany("INSERT OR REPLACE INTO classes VALUES(?,?,?,?,'')",
                   [(H.BUILD, m, c["name"], json.dumps(c["bases"])) for m, c in r["classes"].items()])
    owners = {}  # function -> [(class, col_offset, slot)]
    for v in r["vtables"]:
        for i, a in enumerate(v["slots"]):
            kb.execute("INSERT OR REPLACE INTO vslots VALUES(?,?,?,?,?,?)", (H.BUILD, v["class"], v["va"], v["col_offset"], i, a))
            owners.setdefault(a, []).append((v["class"], v["col_offset"], i))
    rows = [(v["va"], "vtable", "%s::vftable%s" % (r["classes"][v["class"]]["name"], "_%x" % v["col_offset"] if v["col_offset"] else ""),
             "", "auto", "rtti") for v in r["vtables"]]
    for a, own in owners.items():
        held = {c for c, _o, _s in own}
        # The introducing class is the one every other holder derives from.
        top = [c for c in held if all(c == o or c in r["classes"][o]["bases"] for o in held)]
        if len(top) != 1:
            continue
        off, slot = min((o, s) for c, o, s in own if c == top[0])
        name = "%s::%svf%d" % (r["classes"][top[0]]["name"], "vt%x_" % off if off else "", slot)
        rows.append((a, "function", name, "virtual; implemented for %d class%s" % (len(held), "" if len(held) == 1 else "es"), "auto", "rtti"))
    n = put_names(R, "rtti", rows)
    return "rtti: %d classes, %d vtables, %d names" % (len(r["classes"]), len(r["vtables"]), n)


def _class_keys(R):
    return {re.sub(r"[_:<>]", "", n).lower(): n for (n,) in R.kb.execute("SELECT name FROM classes WHERE build=?", (H.BUILD,))}


def seed_engine(R, src=None):
    src = src or os.path.join(H.REPO, "crates", "engine", "src")
    keys, kb = _class_keys(R), R.kb
    known = set(keys.values())
    functions = {a for (a,) in R.ix.execute("SELECT address FROM functions")}
    kb.execute("DELETE FROM fields WHERE build=? AND source='engine'", (H.BUILD,))
    kb.execute("DELETE FROM mentions WHERE build=? AND source LIKE 'crates/engine/%'", (H.BUILD,))
    names, nf = [], 0
    for path in sorted(glob.glob(os.path.join(src, "*.rs"))):
        rel = "crates/engine/src/" + os.path.basename(path)
        stem = os.path.basename(path)[:-3]
        file_class = keys.get(stem.replace("_", ""))
        doc = []
        for ln, line in enumerate(open(path, encoding="utf-8", errors="replace"), 1):
            s = line.strip()
            if s.startswith("//"):
                text = s.lstrip("/! ").strip()
                for m in list(ADDR.finditer(text)) + list(RVA.finditer(text)):
                    h = m.group(1).replace("_", "")
                    a = int(h, 16) + (0 if m.re is ADDR else H.BASE)
                    kb.execute("INSERT OR REPLACE INTO mentions VALUES(?,?,?,?,?)", (H.BUILD, a, rel, ln, text[:300]))
                if s.startswith("///"):
                    doc.append(text)
                continue
            m = CONST.match(line)
            if m and doc:
                cname, _ty, val = m.group(1), m.group(2), int(m.group(3).replace("_", ""), 0)
                text = " ".join(doc)
                cm = CLASSY.match(text + " ")
                cls = (cm.group(1) if cm else None) or file_class or stem
                method = None
                if cls not in known and "::" in cls and cls.rsplit("::", 1)[0] in known:
                    method, cls = cls, cls.rsplit("::", 1)[0]  # the comment names a method, not a nested class
                ev = "%s:%d" % (rel, ln)
                if cname.endswith("RVA") or val >= 0x100000:
                    a = H.BASE + val
                    kind = "function" if a in functions else "vtable" if "VTABLE" in cname else "data"
                    short = re.sub(r"_?RVA$", "", cname).lower() or "rva"
                    names.append((a, kind, method if method and kind == "function" else "%s::%s" % (cls, "vftable" if short == "vtable" else short), text[:400], "low", ev))
                elif "+0x%x" % val in text.lower() or not NOT_FIELD.search(cname):
                    kb.execute("INSERT OR REPLACE INTO fields VALUES(?,?,?,?,?,?,?,?,?,?)",
                               (H.BUILD, cls, val, cname, "", text[:600], ev, "engine", "seed", H.now()))
                    nf += 1
            doc = []
    n = put_names(R, "engine", names)
    return "engine: %d fields, %d names, mentions %d" % (nf, n, kb.execute("SELECT count(*) FROM mentions WHERE build=? AND source LIKE 'crates/engine/%'", (H.BUILD,)).fetchone()[0])


def doc_files():
    pats = ["reference/*.md", "crates/*/*.md", "crates/*/src/**/*.rs", "PLAN.md", "captures/*/findings.md", "captures/*/*.md"]
    out = []
    for p in pats:
        out += glob.glob(os.path.join(H.REPO, p), recursive=True)
    for p in ("*/*.md", "*/*/*.md"):
        out += glob.glob(os.path.join(r"C:\hoi4_scratch", p))
    return [f for f in dict.fromkeys(out) if os.sep + "engine" + os.sep + "src" not in f and os.path.getsize(f) < 4_000_000]


def seed_docs(R, files=None):
    kb = R.kb
    kb.execute("DELETE FROM mentions WHERE build=? AND source NOT LIKE 'crates/engine/%'", (H.BUILD,))
    names, seen = [], {}
    for path in files or doc_files():
        rel = os.path.relpath(path, H.REPO).replace("\\", "/") if path.lower().startswith(H.REPO.lower()) else path
        for ln, line in enumerate(open(path, encoding="utf-8", errors="replace"), 1):
            if "FUN_" not in line and "0x14" not in line:
                continue
            text = line.strip()[:300]
            for m in ADDR.finditer(line):
                kb.execute("INSERT OR REPLACE INTO mentions VALUES(?,?,?,?,?)", (H.BUILD, int(m.group(1), 16), rel, ln, text))
            pairs = [(m.group(1), m.group(2), "medium") for m in NAMED_A.finditer(line)] + [(m.group(2), m.group(1), "low") for m in NAMED_B.finditer(line)]
            for h, name, conf in pairs:
                a = int(h, 16)
                if seen.get(a, "low") == "low" and (a not in seen or conf == "medium"):
                    names = [n for n in names if n[0] != a]
                    seen[a] = conf
                    names.append((a, None, name, text, conf, "%s:%d" % (rel, ln)))
    functions = {a for (a,) in R.ix.execute("SELECT address FROM functions")}
    n = put_names(R, "docs", [(a, "function" if a in functions else "data", nm, s, c, e) for a, _k, nm, s, c, e in names])
    return "docs: %d names, %d mentions" % (n, kb.execute("SELECT count(*) FROM mentions WHERE build=? AND source NOT LIKE 'crates/engine/%'", (H.BUILD,)).fetchone()[0])


def seed_registry(R, path=None):
    path = path or H.REGISTRY
    reg = sqlite3.connect("file:" + path.replace("\\", "/") + "?mode=ro", uri=True)
    kb = R.kb
    kb.execute("DELETE FROM notes WHERE build=? AND source='registry'", (H.BUILD,))
    names, notes = [], []
    for subject, pred, value, status in reg.execute("SELECT subject, predicate, value_json, status FROM claims WHERE predicate<>'rtti_vtable_slot' AND status<>'superseded'"):
        try:
            v = json.loads(value)
            v = json.loads(v) if isinstance(v, str) else v
        except ValueError:
            continue
        if not isinstance(v, dict):
            continue
        m = re.search(r"0x0*([0-9a-f]{6,9})", str(v.get("address") or subject))
        if not m:
            continue
        a = int(m.group(1), 16)
        a = a if a >= H.BASE else a + H.BASE
        if pred == "annotation":
            if re.fullmatch(r"[A-Za-z_~][\w:~<>]*", v["proposed_name"]):
                names.append((a, "function", v["proposed_name"], v["note"][:400], "low", "registry annotation"))
            notes.append((a, "%s: %s" % (v["proposed_name"], v["note"])))
        else:
            notes.append((a, "%s (%s): %s" % (pred, status, json.dumps({k: x for k, x in v.items() if "sha256" not in k})[:1500])))
    for pid, rva, offs, vt, desc in reg.execute("SELECT path_id, base_rva, offsets_json, value_type, description FROM memory_paths"):
        notes.append((H.BASE + rva, "memory path %s: offsets %s as %s. %s" % (pid, offs, vt, desc)))
    for a, t in dict.fromkeys(notes):
        kb.execute("INSERT INTO notes(build,address,text,evidence,source,author,created_utc) VALUES(?,?,?,?,?,?,?)", (H.BUILD, a, t, "", "registry", "seed", H.now()))
    n = put_names(R, "registry", names)
    return "registry: %d names, %d notes" % (n, len(set(notes)))


def seed_sources(R):
    """Source file paths that the game's assert strings leave in each function."""
    pat = re.compile(r"hoi4_merged((?:\\\\[\w.\-]+)+\.(?:cpp|h|hpp|inl|c))\b", re.I)  # paths appear C-escaped
    kb = R.kb
    kb.execute("DELETE FROM sources WHERE build=?", (H.BUILD,))
    n = 0
    rows = []
    for table in (R.ix and R.fts_tables):
        rows += R.ix.execute("SELECT address, content FROM %s WHERE %s MATCH 'hoi4_merged'" % (table, table.split(".")[-1])).fetchall()
    for a, body in rows:
        for path in {m.group(1).replace("\\\\", "/").lstrip("/").lower() for m in pat.finditer(body)}:
            kb.execute("INSERT OR REPLACE INTO sources VALUES(?,?,?)", (H.BUILD, int(a, 16), path))
            n += 1
    kb.commit()
    return "sources: %d function-file pairs, %d files" % (n, kb.execute("SELECT count(DISTINCT path) FROM sources WHERE build=?", (H.BUILD,)).fetchone()[0])


def main(R, argv):
    which = argv or ["rtti", "engine", "docs", "registry", "sources"]
    for w in which:
        print({"rtti": seed_rtti, "engine": seed_engine, "docs": seed_docs, "registry": seed_registry, "sources": seed_sources}[w](R))
    print(H.cmd_status())
