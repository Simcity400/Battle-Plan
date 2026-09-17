"""Patch day: export a new build, index it, and carry names, notes and fields across.

  hoi4re.py migrate export --exe <new hoi4.exe> --build <id> [--shards N]   Ghidra import, analysis and export (hours)
  hoi4re.py migrate index  --build <id>                                     build data/builds/<id>/index.sqlite3
  hoi4re.py migrate names  --from a729 --to <id> --exe <new hoi4.exe>       match functions and copy knowledge
Afterwards use the new build with HOI4RE_BUILD=<id> HOI4RE_EXE=<new exe>.
"""
import argparse, collections, glob, hashlib, json, os, shutil, sqlite3, subprocess
import hoi4re as H
import rtti

GHIDRA = H.env("HOI4RE_GHIDRA", r"C:\Users\Example\Downloads\ghidra_12.1.2_PUBLIC_20260605\ghidra_12.1.2_PUBLIC")
JAVA = H.env("JAVA_HOME", r"C:\Program Files\Microsoft\jdk-21.0.11.10-hotspot")

INDEX_SCHEMA = """
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE functions(address INTEGER PRIMARY KEY, name TEXT NOT NULL, body_min INTEGER NOT NULL, body_max INTEGER NOT NULL,
  graph_sha256 TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE call_edges(caller INTEGER NOT NULL, callee INTEGER NOT NULL, PRIMARY KEY(caller, callee));
CREATE INDEX call_edges_callee ON call_edges(callee);
CREATE TABLE fingerprints(address INTEGER PRIMARY KEY, byte_sha256 TEXT, shape_sha256 TEXT, instructions INTEGER);
CREATE TABLE fts_state(address INTEGER PRIMARY KEY, sha256 TEXT NOT NULL, fts_rowid INTEGER NOT NULL UNIQUE);
CREATE TABLE artifacts(address INTEGER, kind TEXT, canonical_relpath TEXT);
CREATE VIRTUAL TABLE decompile_fts USING fts5(address UNINDEXED, content, tokenize='unicode61');
"""


def build_dir(build):
    d = os.path.join(H.DATA, "builds", build)
    os.makedirs(d, exist_ok=True)
    return d


def export(exe, build, shards):
    d = build_dir(build)
    env = dict(os.environ, JAVA_HOME=JAVA)
    headless = os.path.join(GHIDRA, "support", "analyzeHeadless.bat")
    scripts = os.path.join(H.HERE, "ghidra")
    project = os.path.join(d, "ghidra-0")
    if not os.path.isdir(project):
        os.makedirs(project)
        subprocess.run([headless, project, "the game", "-import", exe], env=env, check=True)
    procs = []
    for s in range(shards):
        copy = os.path.join(d, "ghidra-%d" % s)
        if s and not os.path.isdir(copy):
            shutil.copytree(project, copy)
        out = os.path.join(d, "functions-%d.jsonl" % s)
        log = open(os.path.join(d, "export-%d.log" % s), "w")
        procs.append(subprocess.Popen([headless, copy, "the game", "-process", os.path.basename(exe), "-readOnly", "-noanalysis", "-scriptPath", scripts,
                                       "-postScript", "ExportAll.java", out, str(s), str(shards)], env=env, stdout=log, stderr=subprocess.STDOUT))
    failed = [p.args for p in procs if p.wait()]
    return "export failed: %s" % failed if failed else "exported %d shards to %s" % (shards, d)


def index(build):
    d = build_dir(build)
    path = os.path.join(d, "index.sqlite3")
    if not glob.glob(os.path.join(d, "functions-*.jsonl")):
        raise ValueError("no export in %s; run `migrate export` first (the a729 index is already built)" % d)
    if os.path.exists(path):
        os.remove(path)
    db = sqlite3.connect(path)
    db.executescript(INDEX_SCHEMA)
    n = 0
    for f in sorted(glob.glob(os.path.join(d, "functions-*.jsonl"))):
        for line in open(f, encoding="utf-8"):
            r = json.loads(line)
            if "meta" in r:
                db.execute("INSERT OR REPLACE INTO meta VALUES('executable_sha256', ?)", (r["meta"]["sha256"],))
                db.execute("INSERT OR REPLACE INTO meta VALUES('image_base', ?)", (r["meta"]["image_base"],))
                continue
            a = int(r["va"], 16)
            db.execute("INSERT OR REPLACE INTO functions VALUES(?,?,?,?,'',?)", (a, r["name"], int(r["body_min"], 16), int(r["body_max"], 16), H.now()))
            db.execute("INSERT OR REPLACE INTO fingerprints VALUES(?,?,?,?)", (a, r["byte_sha256"], r["shape_sha256"], r["instructions"]))
            db.executemany("INSERT OR IGNORE INTO call_edges VALUES(?,?)", [(a, int(c, 16)) for c in r["callees"] if c.startswith("0x")])
            cur = db.execute("INSERT INTO decompile_fts(address, content) VALUES(?,?)", ("0x%x" % a, r["decompile"]))
            db.execute("INSERT OR REPLACE INTO fts_state VALUES(?,?,?)", (a, hashlib.sha256(r["decompile"].encode()).hexdigest(), cur.lastrowid))
            n += 1
    db.commit()
    return "%d functions indexed in %s" % (n, path)


def fingerprints(build):
    """{address: (byte_sha, shape_sha)} and {caller: set(callees)} for a build. a729 keeps most edges in the main index."""
    ro = lambda p: sqlite3.connect("file:" + p.replace(chr(92), "/") + "?mode=ro", uri=True)
    sup = ro(os.path.join(build_dir(build), "index.sqlite3"))
    fp = {a: (b, s) for a, b, s in sup.execute("SELECT address, byte_sha256, shape_sha256 FROM fingerprints")}
    pairs = sup.execute("SELECT caller, callee FROM call_edges").fetchall()
    if build == "a729":
        pairs += ro(os.path.join(H.DATA, "function_index.sqlite3")).execute("SELECT caller, callee FROM call_edges").fetchall()
    edges = collections.defaultdict(set)
    for c, e in pairs:
        edges[c].add(e)
    return fp, edges


def match(old_fp, old_edges, new_fp, new_edges, old_exe, new_exe):
    """{old address: (new address, method)}. Order: RTTI slot, unique bytes, unique shape, then call-graph neighbours."""
    out, taken = {}, set()

    def pair(o, n, how):
        if o not in out and n not in taken:
            out[o] = (n, how)
            taken.add(n)

    ro, rn = rtti.scan(old_exe), rtti.scan(new_exe)
    nv = collections.defaultdict(list)
    for v in rn["vtables"]:
        nv[(v["class"], v["col_offset"])].append(v)
    seen = collections.Counter()
    vtables = {}
    for v in ro["vtables"]:
        key = (v["class"], v["col_offset"])
        w = nv[key][seen[key]] if seen[key] < len(nv[key]) else None
        seen[key] += 1
        if w and len(w["slots"]) == len(v["slots"]):
            vtables[v["va"]] = w["va"]
            for o, n in zip(v["slots"], w["slots"]):
                if old_fp.get(o, (0, 1))[1] == new_fp.get(n, (0, 2))[1]:  # a slot can be re-pointed by an override
                    pair(o, n, "rtti")
    for col, how in ((0, "bytes"), (1, "shape")):
        count_o = collections.Counter(v[col] for v in old_fp.values())
        count_n = collections.Counter(v[col] for v in new_fp.values())
        by_n = {v[col]: a for a, v in new_fp.items() if count_n[v[col]] == 1}
        for a, v in old_fp.items():
            if count_o[v[col]] == 1 and v[col] in by_n:
                pair(a, by_n[v[col]], how)
    rev_o, rev_n = collections.defaultdict(set), collections.defaultdict(set)
    for graph, rev in ((old_edges, rev_o), (new_edges, rev_n)):
        for c, es in graph.items():
            for e in es:
                rev[e].add(c)
    work = list(out.items())
    while work:
        o, (n, _how) = work.pop()
        for go, gn in ((old_edges, new_edges), (rev_o, rev_n)):
            cand_o = collections.defaultdict(list)
            cand_n = collections.defaultdict(list)
            for x in go.get(o, ()):
                if x not in out and x in old_fp:
                    cand_o[old_fp[x][1]].append(x)
            for x in gn.get(n, ()):
                if x not in taken and x in new_fp:
                    cand_n[new_fp[x][1]].append(x)
            for shape, xs in cand_o.items():
                if len(xs) == 1 and len(cand_n.get(shape, ())) == 1:
                    pair(xs[0], cand_n[shape][0], "neighbour")
                    work.append((xs[0], out[xs[0]]))
    return out, vtables


def names(src, dst, new_exe, old_exe=None):
    old_fp, old_edges = fingerprints(src)
    new_fp, new_edges = fingerprints(dst)
    pairs, vtables = match(old_fp, old_edges, new_fp, new_edges, old_exe or H.EXE, new_exe)
    kb = H.R.kb
    for t in ("symbols", "notes", "fields", "mentions"):
        kb.execute("DELETE FROM %s WHERE build=?" % t, (dst,))
    lost = []
    for a, kind, name, summary, conf, source, ev in kb.execute("SELECT address,kind,name,summary,confidence,source,evidence FROM symbols WHERE build=? AND source<>'rtti'", (src,)).fetchall():
        to = pairs.get(a, (vtables.get(a), "rtti"))
        if to[0] is None:
            lost.append({"address": "0x%x" % a, "kind": kind, "name": name})
            continue
        kb.execute("INSERT OR REPLACE INTO symbols VALUES(?,?,?,?,?,?,?,?,?,?)",
                   (dst, to[0], kind, name, summary, conf, source, "%s; migrated from %s 0x%x by %s" % (ev, src, a, to[1]), "migrate", H.now()))
    for a, text, ev, source, author, utc in kb.execute("SELECT address,text,evidence,source,author,created_utc FROM notes WHERE build=?", (src,)).fetchall():
        if a in pairs:
            kb.execute("INSERT INTO notes(build,address,text,evidence,source,author,created_utc) VALUES(?,?,?,?,?,?,?)", (dst, pairs[a][0], text, ev, source, author, utc))
    kb.execute("INSERT INTO fields SELECT ?,class,offset,name,type,meaning,evidence||'; unchecked on '||?,source,author,updated_utc FROM fields WHERE build=?", (dst, dst, src))
    kb.commit()
    methods = collections.Counter(how for _n, how in pairs.values())
    report = {"from": src, "to": dst, "functions_old": len(old_fp), "functions_new": len(new_fp), "matched": dict(methods),
              "vtables_matched": len(vtables), "unmatched_named": lost,
              "moved": sum(1 for o, (n, _h) in pairs.items() if o != n)}
    path = os.path.join(build_dir(dst), "migration_report.json")
    json.dump(report, open(path, "w"), indent=1)
    return "matched %d of %d old functions %s; %d named symbols need manual work; report %s\nnext: HOI4RE_BUILD=%s hoi4re.py seed rtti" % (
        len(pairs), len(old_fp), dict(methods), len(lost), path, dst)


def main(R, argv):
    ap = argparse.ArgumentParser(prog="hoi4re migrate", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("export"); p.add_argument("--exe", required=True); p.add_argument("--build", required=True); p.add_argument("--shards", type=int, default=4)
    p = sub.add_parser("index"); p.add_argument("--build", required=True)
    p = sub.add_parser("names"); p.add_argument("--from", dest="src", default="a729"); p.add_argument("--to", dest="dst", required=True)
    p.add_argument("--exe", required=True); p.add_argument("--old-exe")
    a = ap.parse_args(argv)
    H.R = R
    print(export(a.exe, a.build, a.shards) if a.cmd == "export" else index(a.build) if a.cmd == "index" else names(a.src, a.dst, a.exe, a.old_exe))
