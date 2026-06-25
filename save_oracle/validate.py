"""Cross-consistency validator for the save_oracle JSON outputs.

Read-only QA: loads every ``output/*.json`` produced by the parsers and checks
that they agree with one another *before* the data flows into the replay layer.
stdlib-only. Defensive: a missing file is a WARN, not a crash.

Design (hardened after an adversarial review):
- A replay preflight must guard against a STALE MIX of outputs from different
  saves. Source coherence is therefore a **FAIL** by default; pass
  ``--allow-stale-mix`` to demote it to WARN for parser-debug work.
- Source identity is a fingerprint of ``(source_filename, save_date,
  save_version)`` — not the filename alone (two unrelated ``melted.hoi4`` must
  not look coherent). Metaless outputs are *associated* with the parser run
  that emits them (parse_save -> meta.json; parse_control -> control_meta.json).
- Cross-file checks (ownership/detail, detail/control, combat→division→war) are
  only hard FAILs when the two files share a fingerprint; otherwise WARN.

Exit code 0 iff there are no FAILs.  Usage: python validate.py [--output-dir DIR] [--allow-stale-mix]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

TAG_RE = re.compile(r"^[A-Z][A-Z0-9]{2}$")  # GER, ENG, and dynamic tags like D01
SENTINELS = {"", "---"}                       # legal "no country" markers
LEAK_HARD = ("redacted", "C:\\Users\\", "C:/Users/", "/c/Users/")

# Metaless outputs inherit the fingerprint of the run that produced them.
ASSOC = {
    "country_snapshot.json": "meta.json",
    "state_ownership.json": "meta.json",
    "province_control.json": "control_meta.json",
    "state_control.json": "control_meta.json",
}

ALL_FILES = (
    "country_snapshot.json", "state_ownership.json", "state_detail.json",
    "province_control.json", "state_control.json", "divisions.json",
    "diplomacy.json", "economy.json", "focus.json", "tech.json",
    "production.json", "combats.json", "oob.json", "meta.json", "control_meta.json",
    "market.json", "occupation.json",
)


class Report:
    def __init__(self):
        self.rows = []

    def _add(self, level, name, detail):
        self.rows.append((level, name, detail))

    def ok(self, n, d=""):
        self._add("PASS", n, d)

    def warn(self, n, d=""):
        self._add("WARN", n, d)

    def fail(self, n, d=""):
        self._add("FAIL", n, d)

    def gate(self, cond, n, ok_d="", bad_d="", soft=False):
        """PASS if cond else (WARN if soft else FAIL)."""
        (self.ok if cond else (self.warn if soft else self.fail))(n, ok_d if cond else bad_d)

    def dump(self):
        w = max((len(r[1]) for r in self.rows), default=0)
        for level, name, detail in self.rows:
            mark = {"PASS": "[ok]  ", "WARN": "[warn]", "FAIL": "[FAIL]"}[level]
            print(f"{mark} {name.ljust(w)}" + ("  " + detail if detail else ""))
        nf = sum(r[0] == "FAIL" for r in self.rows)
        nw = sum(r[0] == "WARN" for r in self.rows)
        print(f"\n{len(self.rows)} checks  -  {nf} FAIL, {nw} WARN")
        return nf


def load(odir, name, rep):
    path = os.path.join(odir, name)
    if not os.path.isfile(path):
        rep.warn(f"load {name}", "missing - skipped")
        return None
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception as exc:  # noqa: BLE001
        rep.fail(f"load {name}", f"unreadable: {exc}")
        return None


def _meta(doc):
    if isinstance(doc, dict):
        m = doc.get("meta")
        return m if isinstance(m, dict) else doc
    return {}


def fingerprint(doc):
    m = _meta(doc)
    fn = m.get("source_filename")
    if not fn:
        return None
    return (fn, m.get("save_date"), m.get("save_version"))


def stats_of(doc):
    return (_meta(doc) or {}).get("stats") or {}


def is_tag(x):
    return isinstance(x, str) and TAG_RE.match(x) is not None


def main(argv=None):
    ap = argparse.ArgumentParser(description="save_oracle cross-consistency validator")
    here = os.path.dirname(os.path.abspath(__file__))
    ap.add_argument("--output-dir", default=os.path.join(here, "output"))
    ap.add_argument("--allow-stale-mix", action="store_true",
                    help="demote source-incoherence from FAIL to WARN (parser-debug)")
    args = ap.parse_args(argv)
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:  # noqa: BLE001
        pass

    odir = args.output_dir
    rep = Report()
    docs = {n: load(odir, n, rep) for n in ALL_FILES}

    # fingerprint per file, with association for metaless outputs
    fps = {}
    for n in ALL_FILES:
        fp = fingerprint(docs.get(n))
        if fp is None and n in ASSOC:
            fp = fingerprint(docs.get(ASSOC[n]))
        fps[n] = fp

    def same_src(a, b):
        return fps.get(a) is not None and fps.get(a) == fps.get(b)

    # --- 1. source coherence (FAIL on stale mix) -----------------------------
    groups = {}
    for n, fp in fps.items():
        if fp is not None and docs.get(n) is not None:
            groups.setdefault(fp, []).append(n)
    if len(groups) > 1:
        desc = " | ".join(f"{fp[0]}@{fp[1]} <- {len(v)}f" for fp, v in groups.items())
        rep.gate(False, "source coherence", bad_d=f"STALE MIX: {desc}", soft=args.allow_stale_mix)
    elif groups:
        fp = next(iter(groups))
        rep.ok("source coherence", f"all from {fp[0]}@{fp[1]}")
    else:
        rep.warn("source coherence", "no source fingerprints found")

    # --- known country universe ----------------------------------------------
    cs = docs["country_snapshot.json"] or {}
    known = {t for t in cs if is_tag(t)}
    dom_sizes = {"country_snapshot": len(known)}
    domain_sets = {}
    # Only FULL-coverage domains may define the legitimate universe. production
    # is partial (only countries with production lines), so it must be a SUBSET
    # and is NOT allowed to launder a bogus production-only tag into "known".
    for dom, sub in (("economy.json", "countries"), ("focus.json", "countries"),
                     ("tech.json", "countries"), ("diplomacy.json", "countries")):
        d = docs.get(dom) or {}
        ctry = d.get(sub) if isinstance(d.get(sub), dict) else {}
        these = {t for t in ctry if is_tag(t)}
        domain_sets[dom] = these
        dom_sizes[dom] = len(these)
        known |= these
    prod = docs.get("production.json") or {}
    prod_ctry = prod.get("countries") if isinstance(prod.get("countries"), dict) else {}
    prod_tags = {t for t in prod_ctry if is_tag(t)}
    dom_sizes["production.json"] = len(prod_tags)
    rep.gate(bool(known), "known country universe",
             ok_d=f"{len(known)} tags; " + ", ".join(f"{k}={v}" for k, v in dom_sizes.items()),
             bad_d="no country tags found - cannot validate refs")
    if known:
        prod_only = sorted(prod_tags - known)
        rep.gate(not prod_only, "production tags subset of roster",
                 ok_d=f"{len(prod_tags)} production tags all known",
                 bad_d=f"{len(prod_only)} production-only tags: {prod_only[:8]}", soft=True)

    # cross-domain country-set agreement (catches a tag laundering itself in)
    if known and cs:
        base = {t for t in cs if is_tag(t)}
        odd = {dom: sorted(s ^ base)[:8] for dom, s in domain_sets.items()
               if s and s != base and dom != "production.json"}  # production legitimately partial
        rep.gate(not odd, "country-set agreement",
                 ok_d="all full-coverage domains share the roster",
                 bad_d=f"domains differ from country_snapshot: {odd}", soft=True)

    referenced, malformed, sentinel_refs = {}, {}, 0

    def ref(tag, where):
        nonlocal sentinel_refs
        if tag is None:
            return
        if not isinstance(tag, str):
            malformed.setdefault(repr(tag), set()).add(where)
        elif tag in SENTINELS:
            sentinel_refs += 1
        elif is_tag(tag):
            if known and tag not in known:
                referenced.setdefault(tag, set()).add(where)
        else:
            malformed.setdefault(tag, set()).add(where)

    # --- states: counts + ownership/detail/control agreement -----------------
    so = docs["state_ownership.json"] or {}
    sd_doc = docs["state_detail.json"] or {}
    sd = (sd_doc.get("states") or {}) if isinstance(sd_doc, dict) else {}
    sc = docs["state_control.json"] or {}

    rep.gate(len(so) == 1081, "state count (ownership)", f"{len(so)} states",
             f"{len(so)} (expected 1081)", soft=True)
    rep.gate(len(sd) == 1081, "state count (detail)", f"{len(sd)} states",
             f"{len(sd)} (expected 1081)", soft=True)

    for sid, rec in (sd.items() if isinstance(sd, dict) else []):
        if isinstance(rec, dict):
            ref(rec.get("owner"), "state_detail.owner")
            ref(rec.get("controller"), "state_detail.controller")
    for sid, rec in (so.items() if isinstance(so, dict) else []):
        if isinstance(rec, dict):
            ref(rec.get("owner"), "state_ownership.owner")
            ref(rec.get("controller"), "state_ownership.controller")

    def eff(rec):
        owner = rec.get("owner")
        c = rec.get("controller")
        return owner, (owner if c is None else c)

    def compare_states(a, b, label, fa, fb):
        if not (a and b):
            return
        omm = cmm = 0
        for sid, rec in b.items():
            other = a.get(sid)
            if not (isinstance(rec, dict) and isinstance(other, dict)):
                continue
            (do, dc), (oo, oc) = eff(rec), eff(other)
            omm += do != oo
            cmm += dc != oc
        coherent = same_src(fa, fb)
        rep.gate(omm == 0 and cmm == 0, label,
                 ok_d="owner + effective controller match",
                 bad_d=(f"{omm} owner, {cmm} controller mismatches "
                        + ("on SAME source - parser disagreement" if coherent
                           else "- different saves (see source coherence), not a parser bug")),
                 soft=not coherent)

    compare_states(so, sd, "ownership vs detail agree", "state_ownership.json", "state_detail.json")
    compare_states(sc, sd, "detail vs state_control agree", "state_control.json", "state_detail.json")

    # raw controller-override count vs control_meta (same-source only)
    cm_stats = stats_of(docs["control_meta.json"])
    if sd and "raw_state_controller_overrides" in cm_stats and same_src("state_detail.json", "control_meta.json"):
        raw = sum(1 for r in sd.values() if isinstance(r, dict) and r.get("controller") is not None)
        exp = cm_stats["raw_state_controller_overrides"]
        rep.gate(raw == exp, "raw controller overrides vs meta", f"{raw}", f"{raw} != meta {exp}")

    # --- province control -----------------------------------------------------
    pc = docs["province_control.json"] or {}
    for pid, tag in (pc.items() if isinstance(pc, dict) else []):
        ref(tag, "province_control")
    if pc:
        exp = cm_stats.get("provinces_emitted")
        if exp is not None:
            rep.gate(len(pc) == exp, "province control coverage", f"{len(pc)} provinces",
                     f"{len(pc)} != meta {exp}")
        else:
            rep.ok("province control coverage", f"{len(pc)} provinces")

    # --- divisions: count, template join, equipment join, state_id, ranges ----
    dv = docs["divisions.json"] or {}
    divs = dv.get("divisions") or []
    by_t = dv.get("by_template")
    dv_stats = stats_of(dv)
    if divs:
        meta_n = next((dv_stats[k] for k in ("divisions", "count", "total") if k in dv_stats), None)
        if meta_n is not None:
            rep.gate(meta_n == len(divs), "division count vs meta",
                     f"{len(divs)}", f"list={len(divs)} meta={meta_n}")

        # template join (empty by_template is itself a failure, not a free pass)
        if not by_t:
            rep.fail("division->template join", "by_template index missing/empty")
        else:
            bad = sum(1 for d in divs if isinstance(d, dict)
                      and f"{d.get('template_id')}:{d.get('template_type')}" not in by_t)
            rep.gate(bad == 0, "division->template join", f"0/{len(divs)} unresolved",
                     f"{bad}/{len(divs)} unresolved")

        # equipment id join into production.equipment_archetypes
        arch = (docs["production.json"] or {}).get("equipment_archetypes") or {}
        if arch and same_src("divisions.json", "production.json"):
            refs = unres = 0
            for d in divs:
                for e in (d.get("equipment") or []) if isinstance(d, dict) else []:
                    refs += 1
                    if f"{e.get('equip_id')}:{e.get('equip_type')}" not in arch:
                        unres += 1
            rep.gate(unres == 0, "division equipment->archetype join",
                     f"{refs} refs, 0 unresolved", f"{unres}/{refs} unresolved")

        # owners + state_id sanity
        bad_org = bad_str = bad_state = at_sea = 0
        for d in divs:
            if not isinstance(d, dict):
                continue
            ref(d.get("owner"), "division.owner")
            org, stf, st = d.get("organisation"), d.get("strength"), d.get("state_id")
            bad_org += isinstance(org, (int, float)) and not (0 <= org <= 200)
            bad_str += isinstance(stf, (int, float)) and not (0 <= stf <= 1000)
            if st is None:
                at_sea += 1
            elif not (isinstance(st, int) and st > 0):
                bad_state += 1
        rep.gate(bad_org == 0, "division org range", "all in [0,200]", f"{bad_org} out of range", soft=True)
        rep.gate(bad_str == 0, "division strength range", "all in [0,1000]", f"{bad_str} out of range", soft=True)
        rep.gate(bad_state == 0, "division state_id valid",
                 f"all valid ({at_sea} at-sea/null)", f"{bad_state} invalid state_id")

    # --- diplomacy: refs, faction leader/membership bidirectional -------------
    dp = docs["diplomacy.json"] or {}
    factions = dp.get("factions") or []
    dp_countries = dp.get("countries") or {}
    fac_by_id, leader_bad = {}, 0
    for f in factions:
        if not isinstance(f, dict):
            continue
        fac_by_id[f.get("id")] = set(f.get("members") or [])
        for m in (f.get("members") or []):
            ref(m, "faction.member")
        ref(f.get("leader"), "faction.leader")
        if f.get("leader") and f.get("leader") not in (f.get("members") or []):
            leader_bad += 1
    if factions:
        rep.gate(leader_bad == 0, "faction leader is member", f"{len(factions)} factions",
                 f"{leader_bad} faction(s) whose leader is not a member")
        # bidirectional: country.faction_id <-> faction.members
        bad_fwd = bad_rev = 0
        for tag, info in dp_countries.items():
            if not isinstance(info, dict):
                continue
            fid = info.get("faction_id")
            if fid in (None, 0):
                continue
            if tag not in fac_by_id.get(fid, set()):
                bad_fwd += 1
        for fid, members in fac_by_id.items():
            for m in members:
                ci = dp_countries.get(m)
                # a member missing from diplomacy.countries, or with a null/other
                # faction_id, is a real reverse-mismatch (not a free pass)
                if not (isinstance(ci, dict) and ci.get("faction_id") == fid):
                    bad_rev += 1
        rep.gate(bad_fwd == 0 and bad_rev == 0, "faction_id <-> membership",
                 "bidirectionally consistent", f"{bad_fwd} fwd, {bad_rev} rev mismatches", soft=True)
    for w in (dp.get("wars") or []):
        if isinstance(w, dict):
            ref(w.get("a"), "war.a")
            ref(w.get("b"), "war.b")
    for s in (dp.get("subjects") or []):
        if isinstance(s, dict):
            ref(s.get("overlord"), "subject.overlord")
            ref(s.get("subject"), "subject.subject")

    # --- combats: unit handles -> units, owners -> wars ----------------------
    # Combat units are [id, type] handles. LAND combats reference divisions
    # (divisions.json); NAVAL combats reference fleets/task-forces/ships and AIR
    # combats reference air wings (oob.json). Build one "id:type"-keyed handle
    # map across all three so a combat of any kind resolves.
    combats = (docs["combats.json"] or {}).get("combats") or []
    if combats:
        handle_owner = {}
        for d in (divs or []):
            if isinstance(d, dict) and d.get("div_id") is not None:
                handle_owner[f"{d.get('div_id')}:{d.get('div_type')}"] = d.get("owner")
        oob = docs.get("oob.json") or {}
        for fleet in (oob.get("fleets") or []):
            if not isinstance(fleet, dict):
                continue
            fo = fleet.get("owner")
            if fleet.get("id") is not None:
                handle_owner.setdefault(str(fleet["id"]), fo)
            for tf in (fleet.get("task_forces") or []):
                if not isinstance(tf, dict):
                    continue
                to = tf.get("owner") or fo
                if tf.get("id") is not None:
                    handle_owner.setdefault(str(tf["id"]), to)
                for sh in (tf.get("ships") or []):
                    if isinstance(sh, dict) and sh.get("id") is not None:
                        handle_owner.setdefault(str(sh["id"]), to)
        for w in (oob.get("air_wings") or []):
            if isinstance(w, dict) and w.get("id") is not None:
                handle_owner.setdefault(str(w["id"]), w.get("owner"))

        def _ukey(u):
            if isinstance(u, (list, tuple)) and len(u) >= 2:
                return f"{u[0]}:{u[1]}"
            return str(u)

        war_pairs = {frozenset((w.get("a"), w.get("b")))
                     for w in (dp.get("wars") or []) if isinstance(w, dict)}
        unit_refs = unit_unres = not_at_war = 0
        coherent = same_src("combats.json", "divisions.json")
        for c in combats:
            if not isinstance(c, dict):
                continue
            side_owners = {"attacker": set(), "defender": set()}
            for side in ("attacker", "defender"):
                blk = c.get(side) or {}
                for u in (blk.get("units") or []):
                    unit_refs += 1
                    key = _ukey(u)
                    if key in handle_owner:
                        side_owners[side].add(handle_owner[key])
                    else:
                        unit_unres += 1
            for a in side_owners["attacker"]:
                for b in side_owners["defender"]:
                    if a and b and a != b and frozenset((a, b)) not in war_pairs:
                        not_at_war += 1
        if unit_refs:
            rep.gate(unit_unres == 0, "combat unit -> unit resolve",
                     f"{unit_refs} unit refs (land+naval+air), 0 unresolved",
                     f"{unit_unres}/{unit_refs} unresolved", soft=not coherent)
            # at-war is advisory: expeditionaries/volunteers fight under other tags
            rep.gate(not_at_war == 0, "combat owners are at war",
                     "all engaged owner-pairs are at war",
                     f"{not_at_war} engaged pair(s) not found in wars", soft=True)

    # --- per-country domains define the universe; nothing extra to ref --------
    # (economy/focus/tech/production country keys already folded into `known`)

    # --- tag integrity verdict -----------------------------------------------
    if known:
        if referenced:
            sample = "; ".join(f"{t}({','.join(sorted(w))[:30]})"
                               for t, w in sorted(referenced.items())[:12])
            rep.fail("tag integrity (no dangling refs)",
                     f"{len(referenced)} unknown TAGs: {sample}")
        else:
            rep.ok("tag integrity (no dangling refs)",
                   f"all referenced TAGs known ({sentinel_refs} sentinel refs)")
    if malformed:
        sample = "; ".join(f"{t!r}({','.join(sorted(w))[:30]})"
                           for t, w in sorted(malformed.items(), key=lambda kv: str(kv[0]))[:10])
        rep.fail("tag well-formedness", f"{len(malformed)} malformed refs: {sample}")
    else:
        rep.ok("tag well-formedness", "no malformed country references")

    # --- leak guard on recorded source paths ---------------------------------
    leaks = []
    for n in ("meta.json", "control_meta.json", "divisions.json", "combats.json"):
        d = docs.get(n)
        if d is None:
            continue
        blob = json.dumps(d)
        for needle in LEAK_HARD:
            if needle in blob:
                leaks.append(f"{n}:{needle}")
    rep.gate(not leaks, "source-path redaction", "clean", "LEAK: " + ", ".join(leaks))

    return 1 if rep.dump() else 0


if __name__ == "__main__":
    sys.exit(main())
