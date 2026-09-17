"""Work queue and review for fleets of agents.

  hoi4re todo fill                              rebuild the ranked queues (keeps claims and finished items)
  hoi4re todo status
  hoi4re todo next <kind> --author A [--count N]  claim the next items; prints each item and the brief to follow
  hoi4re todo done <kind> <item> --author A [--note why]   finish an item that a name alone does not close
  hoi4re review authors | sample --author A [--n 10] | drop --author A
Kinds: hot, cited, sourced (functions); class (one class); docs (one report file).
A function item closes by itself once the function has a name at confidence low or better. Claims lapse after 3 hours.
"""
import argparse, collections, datetime, glob, os, random, re
import hoi4re as H

FUNCTION_KINDS = ("hot", "cited", "sourced")
LEASE_HOURS = 3
ADDR = re.compile(r"\b(?:FUN_|0x)(1[0-9a-f]{8})\b")
DOC_ROOTS = [(r"C:\hoi4_evidence\battle_plan_pre_rewrite_2026-09-09", "**/*.md"), (r"C:\hoi4_evidence\lanes", "**/*.md"),
             (r"C:\hoi4_scratch", "*/*.md"), (r"C:\hoi4_scratch", "*/*/*.md"), (H.REPO, "reference/*.md"), (H.REPO, "crates/*/*.md")]


def table(kb):
    kb.execute("CREATE TABLE IF NOT EXISTS queue(build TEXT, kind TEXT, item TEXT, rank REAL, info TEXT, claimed_by TEXT, claimed_utc TEXT, "
               "done_utc TEXT, note TEXT, PRIMARY KEY(build, kind, item))")


def named(R):
    """Addresses that need no more naming work: a real name, or compiler plumbing."""
    low = H.RANK["low"]
    return {a for a, c, s in R.kb.execute("SELECT address, confidence, source FROM symbols WHERE build=?", (H.BUILD,))
            if H.RANK[c] >= low or s in ("plumbing", "define", "accessor", "ctor")}


def fill(R):
    kb = R.kb
    table(kb)
    skip = named(R) | {a for (a,) in R.ix.execute("SELECT address FROM functions WHERE name NOT LIKE 'FUN_%' AND name NOT LIKE 'thunk_FUN_%'")}
    callers = collections.Counter(c for (c,) in R.ix.execute("SELECT callee FROM call_edges"))
    functions = {a for (a,) in R.ix.execute("SELECT address FROM functions")}
    rows = [("hot", "%x" % a, n, "%d callers" % n) for a, n in callers.most_common() if a in functions and a not in skip][:3000]
    taken = {r[1] for r in rows}
    cited = collections.Counter(a for (a,) in kb.execute("SELECT address FROM mentions WHERE build=?", (H.BUILD,)))
    rows += [("cited", "%x" % a, n, "%d citations in the repo and reports" % n) for a, n in cited.items() if a in functions and a not in skip and "%x" % a not in taken]
    taken |= {r[1] for r in rows}
    for a, path in kb.execute("SELECT address, min(path) FROM sources WHERE build=? AND path LIKE '%.cpp' GROUP BY address", (H.BUILD,)):
        if a not in skip and "%x" % a not in taken:
            rows.append(("sourced", "%x" % a, callers[a], path))
    own = collections.Counter()
    for (name,) in kb.execute("SELECT name FROM symbols WHERE build=? AND source='rtti' AND kind='function'", (H.BUILD,)):
        own[name.rsplit("::", 1)[0]] += 1
    rows += [("class", c, n, "%d own virtual functions still unnamed" % n) for c, n in own.items()
             if n >= 3 and not re.search(r"<|lambda|^NDefines|^std::|^type_info|\?", c)]
    for root, pattern in DOC_ROOTS:
        for path in glob.glob(os.path.join(root, pattern), recursive=True):
            try:
                if os.path.getsize(path) < 3_000_000:
                    n = len(set(ADDR.findall(open(path, encoding="utf-8", errors="replace").read())))
                    if n >= 3:
                        rows.append(("docs", path, n, "%d distinct addresses" % n))
            except OSError:
                pass
    for kind, item, rank, info in rows:
        kb.execute("INSERT INTO queue(build,kind,item,rank,info) VALUES(?,?,?,?,?) ON CONFLICT(build,kind,item) DO UPDATE SET rank=excluded.rank, info=excluded.info",
                   (H.BUILD, kind, item, rank, info))
    kb.commit()
    return status(R)


def open_items(R, kind):
    done = named(R) if kind in FUNCTION_KINDS else set()
    cutoff = (datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(hours=LEASE_HOURS)).strftime("%Y-%m-%dT%H:%M:%SZ")
    for item, info, by, when in R.kb.execute("SELECT item, info, claimed_by, claimed_utc FROM queue WHERE build=? AND kind=? AND done_utc IS NULL ORDER BY rank DESC", (H.BUILD, kind)):
        if kind in FUNCTION_KINDS and int(item, 16) in done:
            continue
        yield item, info, (by if by and when > cutoff else None)


def status(R):
    table(R.kb)
    out = ["%-8s %8s %8s %8s" % ("kind", "open", "claimed", "total")]
    for (kind,) in R.kb.execute("SELECT DISTINCT kind FROM queue WHERE build=? ORDER BY kind", (H.BUILD,)).fetchall():
        items = list(open_items(R, kind))
        total = R.kb.execute("SELECT count(*) FROM queue WHERE build=? AND kind=?", (H.BUILD, kind)).fetchone()[0]
        out.append("%-8s %8d %8d %8d" % (kind, len(items), sum(1 for i in items if i[2]), total))
    return "\n".join(out)


def take(R, kind, author, count):
    table(R.kb)
    mine = [i for i in open_items(R, kind) if i[2] == author]
    fresh = [i for i in open_items(R, kind) if i[2] is None][:max(0, count - len(mine))]
    for item, _info, _by in fresh:
        R.kb.execute("UPDATE queue SET claimed_by=?, claimed_utc=? WHERE build=? AND kind=? AND item=?", (author, H.now(), H.BUILD, kind, item))
    R.kb.commit()
    out = ["brief: %s" % os.path.join(H.HERE, "briefs", kind + ".md")]
    for item, info, _by in (mine + fresh)[:count]:
        out.append("%s  (%s)%s" % (item, info, "  -> hoi4re fn %s" % item if kind in FUNCTION_KINDS else "  -> hoi4re class %s" % item if kind == "class" else ""))
    return "\n".join(out) if len(out) > 1 else "nothing left in %s" % kind


def finish(R, kind, item, author, note):
    table(R.kb)
    n = R.kb.execute("UPDATE queue SET done_utc=?, claimed_by=?, note=? WHERE build=? AND kind=? AND item=?", (H.now(), author, note, H.BUILD, kind, item)).rowcount
    R.kb.commit()
    return "closed %s %s" % (kind, item) if n else "no such item"


def authors(R):
    rows = collections.defaultdict(lambda: [0, 0, 0])
    for i, t in enumerate(("symbols", "notes", "fields")):
        for a, n in R.kb.execute("SELECT author, count(*) FROM %s WHERE build=? AND source='agent' GROUP BY 1" % t, (H.BUILD,)):
            rows[a][i] = n
    return "\n".join(["%-32s %7s %7s %7s" % ("author", "names", "notes", "fields")] + ["%-32s %7d %7d %7d" % (a, *v) for a, v in sorted(rows.items())])


def sample(R, author, n):
    rows = R.kb.execute("SELECT address, name, summary, confidence, evidence FROM symbols WHERE build=? AND source='agent' AND author=?", (H.BUILD, author)).fetchall()
    out = []
    for a, name, summary, conf, ev in random.sample(rows, min(n, len(rows))):
        body = (R.body(a) or "(data)").splitlines()
        out += ["=" * 100, "%s @%x (%s)" % (name, a, conf), "summary: %s" % summary, "evidence: %s" % ev, "-" * 40] + [R.rewrite(l) for l in body[:45]]
    return "\n".join(out) or "no names by %s" % author


def drop(R, author):
    kb, n = R.kb, 0
    for t in ("symbols", "notes", "fields"):
        n += kb.execute("DELETE FROM %s WHERE build=? AND source='agent' AND author=?" % t, (H.BUILD, author)).rowcount
    table(kb)
    kb.execute("UPDATE queue SET done_utc=NULL, claimed_by=NULL, claimed_utc=NULL WHERE build=? AND claimed_by=?", (H.BUILD, author))
    kb.commit()
    R.journal({"op": "drop_author", "author": author, "rows": n})
    return "removed %d rows by %s and reopened their queue items; run `hoi4re seed` and `hoi4re derive` to restore displaced names" % (n, author)


def main(R, argv, mode="todo"):
    ap = argparse.ArgumentParser(prog="hoi4re " + mode, description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    author = lambda p: p.add_argument("--author", default=H.env("HOI4RE_AUTHOR", ""))
    if mode == "todo":
        sub.add_parser("fill"); sub.add_parser("status")
        p = sub.add_parser("next"); p.add_argument("kind"); p.add_argument("--count", type=int, default=5); author(p)
        p = sub.add_parser("done"); p.add_argument("kind"); p.add_argument("item"); p.add_argument("--note", default=""); author(p)
    else:
        sub.add_parser("authors")
        p = sub.add_parser("sample"); p.add_argument("--n", type=int, default=10); author(p)
        p = sub.add_parser("drop"); author(p)
    a = ap.parse_args(argv)
    if a.cmd not in ("fill", "status", "authors") and not a.author:
        raise ValueError("--author (or HOI4RE_AUTHOR) is required")
    print({"fill": lambda: fill(R), "status": lambda: status(R), "next": lambda: take(R, a.kind, a.author, a.count),
           "done": lambda: finish(R, a.kind, a.item, a.author, a.note), "authors": lambda: authors(R),
           "sample": lambda: sample(R, a.author, a.n), "drop": lambda: drop(R, a.author)}[a.cmd]())
