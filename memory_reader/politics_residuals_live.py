"""politics_residuals_live.py - LIVE per-country POLITICS RESIDUALS reader (read-only).

Closes the two OPEN TICKETS the politics_live + leadership_deep_live passes left
RESISTED, all save-free from a running PAUSED hoi4.exe (ReadProcessMemory only; no
writes / no injection / no thread suspend, so it never desyncs MP), pinned on build
16022851 (see politics_residuals_offsets.draft.json):

  1. last_election (DATE) + elections_allowed (BOOL)
     Both live on the per-country CPoliticalStatus (country_obj + 0xF90), next to the
     already-pinned election_frequency (ps+0xE8):

        last_election     i32 @ ps+0xA0   Jomini DateHour binary (+5000-year epoch)
                          stored = (((year+5000)*365 + ordinal0)*24) + (hour-1)
                          == my_naive_hours + 43799999 where
                          my_naive_hours = ((year*365 + day_of_year)*24 + hour), hour 1-based.
                          The prior pass's naive int scan missed it because of the
                          +5000-year epoch; an affine cross-country scan (slope 1.0,
                          intercept 43799999, zero error / 46 countries) pinned it.
                          Round-trip decode reproduces the exact save date string 46/46.
        elections_allowed u8 bool @ ps+0xEC (0 == 'no', 1 == 'yes').   46/46 exact.

  2. APPOINTED-cabinet advisor SUBSET (was wrongly marked RESISTED)
     The prior pass rejected this claiming an "empty intersection" - but it sampled
     only SOV, which genuinely has zero appointed advisors at the 1946 anchor. The
     correct save-grounded discriminator:

        an advisor is APPOINTED iff its advisor idea_token (CAdvisor roster,
        leadership_deep_live.read_advisor_roster) appears in the country's active
        politics.ideas list (politics_live.read_country_politics ideas[]).

     Hiring an advisor in the game add_idea's the advisor's idea_token, so it shows up
     directly in politics.ideas. NO new memory offset is needed - the appointed
     subset is the live INTERSECTION of two already-validated readers. Validated
     against the melt: only ENG has appointed advisors at this anchor (philip_kerr,
     maxwell_aitken, leslie_hore_belisha); live==save 46/46 (45 empty + ENG x3).

LOCATION (save-free, self-relocating each session): reuses politics_live's country
array locate + unique-PP TAG bridge and CPoliticalStatus @ country_obj+0xF90 (each
RTTI-verified .?AVCPoliticalStatus@@), and leadership_deep_live's CAdvisor roster.

BUILD-GATED, READ-ONLY: refuses to run unless the live exe SHA-256 matches the mapped
build (same fail-closed gate as the sibling readers); PROCESS_VM_READ | QUERY_INFORMATION.

Public API
----------
    read_election_residuals(p, bridge=None) -> { TAG: {
        "last_election": str | None,        # decoded date "Y.M.D.H" (== save string)
        "last_election_raw": int | None,    # the live i32 Jomini DateHour binary
        "elections_allowed": bool | None,   # ps+0xEC
        "_status_addr": int } }

    read_appointed_advisors(p, politics=None, roster=None) -> { TAG: {
        "appointed": [ {"char_id": int, "slot": str|None, "idea_token": str}, ... ],
        "appointed_tokens": [idea_token, ...] } }
    # the subset of the advisor ROSTER whose idea_token is in the country's active
    # politics.ideas (currently-appointed advisor ideas / cabinet occupants).

    read_residuals(p, bridge=None, rec0=None, count=None, politics=None, roster=None)
        -> { TAG: { ...election fields..., "appointed_advisors": [...] } }
    # convenience: both residual groups merged per country. `politics`/`roster` are an
    # optional SAFE pre-supply hook: when a higher layer hands in an already-resolved
    # (pinned/cached) politics map and/or advisor roster, the matching sub-read -- incl.
    # the CAdvisor committed-heap sweep in leadership_deep_live -- is SKIPPED (output
    # byte-identical). The advisor sweep itself is a STAGE-4 item: it lives in
    # leadership_deep_live.read_advisor_roster (out of this file's edit/ownership scope),
    # so it must be pinned THERE; see the read_residuals docstring for the full flag.

Usage
-----
    python politics_residuals_live.py            # dump elections + appointed advisors
    python politics_residuals_live.py --validate # live==save report
"""

import argparse
import json
import os
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)
sys.path.insert(0, os.path.dirname(_THIS))

import winmem
import politics_live as POL
import leadership_deep_live as LD

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"

# ---- CPoliticalStatus residual offsets (build 16022851) ----
PS_LAST_ELECTION = 0xA0       # i32 Jomini DateHour binary (+5000-year epoch)
PS_ELECTIONS_ALLOWED = 0xEC   # u8 bool (0 == no, 1 == yes); right after freq @ 0xE8

# ---- Jomini DateHour encoding ----
DATE_EPOCH_CONST = 43799999   # stored = my_naive_hours + 43799999  (= 5000*365*24 - 1)
_MONTH_LEN = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]   # no leap years
_DAYS_BEFORE = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]

# plausibility bounds for a sane last_election (years ~1850..2100 in stored space)
_LE_MIN = (1850 * 365) * 24 + DATE_EPOCH_CONST
_LE_MAX = (2100 * 365) * 24 + DATE_EPOCH_CONST


# ==========================================================================
# build gate
# ==========================================================================
def check_build_gate(p, force=False):
    got = p.fingerprint().get("sha256")
    if got == EXPECTED_SHA:
        return
    msg = ("build gate: live exe SHA-256 %s != mapped build %s; offsets are "
           "build-specific." % (got, EXPECTED_SHA))
    if force:
        print(msg + "  --force given (results UNVERIFIED).")
    else:
        raise SystemExit(msg + "  Refusing (pass --force).")


# ==========================================================================
# Jomini DateHour <-> string
# ==========================================================================
def decode_datehour(stored):
    """Decode the live i32 Jomini DateHour binary -> 'Y.M.D.H' (the save string).

    Inverse of stored = (((year+5000)*365 + ordinal0)*24) + (hour-1). Returns None if
    the value is not a plausible date (filters out a misread / wrong pointer)."""
    if stored is None or not (_LE_MIN <= stored <= _LE_MAX):
        return None
    hrs = stored - DATE_EPOCH_CONST       # back to my_naive_hours (1-based hour)
    h = hrs % 24
    rem = hrs // 24
    y = rem // 365
    doy = rem - y * 365                    # 0-based day-of-year
    m = 1
    for ml in _MONTH_LEN:
        if doy < ml:
            break
        doy -= ml
        m += 1
    return "%d.%d.%d.%d" % (y, m, doy + 1, h)


def encode_datehour(date_str):
    """'Y.M.D.H' -> the i32 Jomini DateHour binary (for tests / GT building)."""
    y, m, d, h = [int(x) for x in date_str.split(".")]
    doy = _DAYS_BEFORE[m - 1] + (d - 1)
    return ((y * 365 + doy) * 24 + h) + DATE_EPOCH_CONST


# ==========================================================================
# 1. election residuals (last_election + elections_allowed)
# ==========================================================================
def read_election_residuals(p, bridge=None, strict=True, rec0=None, count=None):
    """LIVE per-country last_election (decoded) + elections_allowed. Returns {TAG: {...}}.

    Reuses politics_live's country-array locate + unique-PP bridge and the RTTI-gated
    CPoliticalStatus @ country_obj+0xF90. Each scalar is DOUBLE-READ (Codex anti-torn
    guard) and dropped if the two reads disagree.

    `rec0`/`count` (optional, supplied TOGETHER): the country-array base + count FROM THE LIVE
    CountryIndexBridge. When given, the locate is SKIPPED -- removing the redundant relocate AND
    the stale-save-PP relocate that raised (LocateError) and killed the whole walk."""
    if (rec0 is None) != (count is None):
        raise ValueError("read_election_residuals: rec0 and count must be supplied together")
    pp_path = os.path.join(os.path.dirname(_THIS), "re_scratch", "_country_pp.json")
    save_pp = json.load(open(pp_path)) if os.path.exists(pp_path) else {}
    if rec0 is None:
        rec0, count = POL._locate_country_array(p, save_pp)
    if bridge is None:
        bridge = POL._build_index_to_tag(p, rec0, count, save_pp)
        if save_pp and not POL._bridge_is_live(p, rec0, bridge, save_pp):
            import country_misc_live as cm
            sc = {t: {"political_power": v} for t, v in save_pp.items()}
            rec0, count = cm.locate_via_unique_pp(p, sc)
            bridge = POL._build_index_to_tag(p, rec0, count, save_pp)

    out = {}
    for i, tag in bridge.items():
        rec = rec0 + i * POL.REC
        try:
            if p.u16(rec + POL.OFF_INDEX) != i:
                continue
            obj = p.u64(rec + POL.OFF_NAMEPTR)
        except OSError:
            continue
        if not POL.heap_ok(obj):
            continue
        ps = POL._su64(p, obj + POL.OBJ_POLSTATUS)
        if not POL.heap_ok(ps):
            continue
        if strict and POL._obj_rtti(p, ps) != POL.RTTI_STATUS:
            continue

        # last_election (double-read i32)
        le_raw = None
        a = POL._si32(p, ps + PS_LAST_ELECTION)
        b = POL._si32(p, ps + PS_LAST_ELECTION)
        if a is not None and a == b:
            le_raw = a
        le_str = decode_datehour(le_raw)

        # elections_allowed (double-read u8 -> bool)
        ea = None
        try:
            e1 = p.u8(ps + PS_ELECTIONS_ALLOWED)
            e2 = p.u8(ps + PS_ELECTIONS_ALLOWED)
            if e1 == e2 and e1 in (0, 1):
                ea = bool(e1)
        except OSError:
            ea = None

        out[tag] = {
            "last_election": le_str,
            "last_election_raw": le_raw,
            "elections_allowed": ea,
            "_status_addr": ps,
        }
    return out


# ==========================================================================
# 2. appointed-cabinet advisor subset
# ==========================================================================
def read_appointed_advisors(p, politics=None, roster=None):
    """LIVE per-country APPOINTED advisor subset = the roster advisors whose idea_token
    is present in the country's active politics.ideas. Returns {TAG: {...}}.

    Composes two already-validated readers (no new offset):
      politics_live.read_country_politics  -> active ideas[]
      leadership_deep_live.read_advisor_roster -> advisor-capable roster (idea_token)."""
    if politics is None:
        politics = POL.read_country_politics(p)
    if roster is None:
        roster = LD.read_advisor_roster(p)

    out = {}
    for tag, d in politics.items():
        ideas = set(d.get("ideas", []))
        roles = roster.get(tag, [])
        appointed = []
        seen = set()
        for r in roles:
            tok = r.get("idea_token")
            if tok and tok in ideas and tok not in seen:
                appointed.append({
                    "char_id": r.get("char_id"),
                    "slot": r.get("slot"),
                    "idea_token": tok,
                })
                seen.add(tok)
        out[tag] = {
            "appointed": appointed,
            "appointed_tokens": sorted(seen),
        }
    return out


# ==========================================================================
# convenience: both residual groups merged
# ==========================================================================
def read_residuals(p, bridge=None, rec0=None, count=None, politics=None, roster=None):
    """Resolve once and return both residual groups merged per country.

    `bridge`/`rec0`/`count` (optional): the shared LIVE CountryIndexBridge naming map +
    country-array base, threaded into the two PP-bridge sub-reads so they SKIP their own
    _locate_country_array (removes the redundant relocate AND the stale-save-PP relocate that
    raised LocateError and killed the whole walk).

    WARM-WALK / STAGE-4 NOTE (the dominant per-walk SWEEP is NOT local to this file)
    ---------------------------------------------------------------------------------
    Of the sub-reads this composes, exactly ONE does a full committed-heap scan EVERY call:
    `LD.read_advisor_roster(p)` -> `leadership_deep_live._find_instances(p, vt_adv, 30000)`,
    which iterates every committed region (HEAP_LO..0x7FF000000000, 16 MB chunks) hunting the
    CAdvisor vtable needle. The needle count never reaches 30000, so it scans the WHOLE heap
    on each warm walk. The other three sub-reads are cheap when the country bridge is supplied:
      * read_election_residuals  -> bridge-driven per-country fixed-offset reads (no scan)
      * POL.read_country_politics-> bridge-driven per-country CPoliticalStatus walk (no scan)
      * read_appointed_advisors  -> pure dict set-intersection (no memory access)
    The advisor sweep CANNOT be pinned from inside this file: a stable-id-validated address
    cache for the CAdvisor pool would have to live in leadership_deep_live.py (which OWNS the
    CAdvisor vtable constant, the pool-enumeration policy, and the role decode). Re-implementing
    it here would duplicate that module's pool ownership (wrong layering / two readers of the
    same pool). STAGE-4 FLAG: pin the advisor sweep in leadership_deep_live.py by adding
    enumerate_advisor_addrs / read_advisor_roster_from_addrs / validate_advisor_addrs (ABA-
    defended on CCharacter+0x0C char_id + idea_token) and thread a cached-roster up to here.

    `politics`/`roster` (optional): a SAFE in-file pre-supply hook for exactly that Stage-4
    fix. When a higher layer can hand in an already-resolved (pinned/cached) politics map
    and/or advisor roster, pass them here and the corresponding sub-read -- including the
    advisor heap sweep -- is SKIPPED. Output is byte-identical to the cold path (the same
    decode is reused). Default None preserves the current self-resolving cold behavior."""
    elections = read_election_residuals(p, bridge=bridge, rec0=rec0, count=count)
    if politics is None:
        politics = POL.read_country_politics(p, bridge=bridge, rec0=rec0, count=count)
    if roster is None:
        roster = LD.read_advisor_roster(p)
    appointed = read_appointed_advisors(p, politics=politics, roster=roster)
    out = {}
    for tag in set(elections) | set(appointed):
        e = elections.get(tag, {})
        out[tag] = {
            "last_election": e.get("last_election"),
            "last_election_raw": e.get("last_election_raw"),
            "elections_allowed": e.get("elections_allowed"),
            "appointed_advisors": appointed.get(tag, {}).get("appointed", []),
            "appointed_tokens": appointed.get(tag, {}).get("appointed_tokens", []),
            "_status_addr": e.get("_status_addr"),
        }
    return out


# ==========================================================================
# validation vs the lockstep save
# ==========================================================================
def _load_politics_gt():
    scr = os.path.join(os.path.dirname(_THIS), "re_scratch")
    return json.load(open(os.path.join(scr, "_politics_gt.json")))


def _load_appointed_gt():
    """Appointed-advisor GT = {TAG: [idea_token, ...]} derived from the melt
    (politics.ideas intersect global advisor-token pool). Built by
    re_scratch/_polres_appointed_gt2.py."""
    scr = os.path.join(os.path.dirname(_THIS), "re_scratch")
    path = os.path.join(scr, "_polres_appointed_tokens_gt.json")
    return json.load(open(path)) if os.path.exists(path) else {}


def _validate(p):
    """Read live, compare to the save anchors, return a one-screen report dict."""
    gt = _load_politics_gt()
    appt_gt = _load_appointed_gt()

    # ---- 1. election residuals ----
    elections = read_election_residuals(p)
    le_ok = le_n = 0
    ea_ok = ea_n = 0
    le_bad = []
    ea_bad = []
    for tag, d in elections.items():
        g = gt.get(tag)
        if not g:
            continue
        if g.get("last_election") is not None:
            le_n += 1
            if d["last_election"] == g["last_election"]:
                le_ok += 1
            else:
                le_bad.append((tag, d["last_election"], g["last_election"]))
        if g.get("elections_allowed") in ("yes", "no"):
            ea_n += 1
            want = (g["elections_allowed"] == "yes")
            if d["elections_allowed"] == want:
                ea_ok += 1
            else:
                ea_bad.append((tag, d["elections_allowed"], g["elections_allowed"]))

    # ---- 2. appointed advisors ----
    appointed = read_appointed_advisors(p)
    appt_ok = appt_n = 0
    appt_bad = []
    appt_nonempty_live = {}
    for tag, d in appointed.items():
        g = set(appt_gt.get(tag, []))
        live = set(d["appointed_tokens"])
        if live:
            appt_nonempty_live[tag] = sorted(live)
        # only score countries the GT knows about (i.e. live-covered & in politics GT)
        if tag not in gt:
            continue
        appt_n += 1
        if live == g:
            appt_ok += 1
        else:
            appt_bad.append((tag, sorted(live - g), sorted(g - live)))

    return {
        "live_countries": len(elections),
        "last_election_vs_save": "%d/%d exact" % (le_ok, le_n),
        "elections_allowed_vs_save": "%d/%d exact" % (ea_ok, ea_n),
        "appointed_advisor_set_vs_save": "%d/%d exact" % (appt_ok, appt_n),
        "appointed_nonempty_live": appt_nonempty_live,
        "last_election_mismatches": le_bad[:6],
        "elections_allowed_mismatches": ea_bad[:6],
        "appointed_mismatches": appt_bad[:6],
        "notes": {
            "last_election": "i32 @ ps+0xA0, Jomini DateHour binary; stored = my_naive_hours + 43799999 (+5000y epoch, hour-1). Round-trip decoded to the save string.",
            "elections_allowed": "u8 bool @ ps+0xEC (right after election_frequency @ ps+0xE8).",
            "appointed_advisors": "appointed = advisor roster idea_token in active politics.ideas. At this 1946 anchor only ENG has any (x3); all other countries empty. Save-grounded discriminator (NOT the prior 'RESISTED').",
        },
    }


def main():
    ap = argparse.ArgumentParser(description="LIVE politics-residuals reader (read-only).")
    ap.add_argument("--force", action="store_true",
                    help="run even if the exe SHA-256 doesn't match the mapped build")
    ap.add_argument("--validate", action="store_true",
                    help="compare live reads to the save anchor and report match rate")
    args = ap.parse_args()

    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        check_build_gate(p, args.force)
        if args.validate:
            print(json.dumps(_validate(p), indent=1))
            return
        res = read_residuals(p)
        print("\n%-4s %-14s %-4s %s" % ("TAG", "last_election", "vote", "appointed_advisors"))
        for tag in sorted(res):
            d = res[tag]
            appt = ",".join(d["appointed_tokens"]) or "-"
            print("%-4s %-14s %-4s %s"
                  % (tag, d["last_election"] or "?",
                     "yes" if d["elections_allowed"] else "no",
                     appt))
        print("\ncountries read: %d" % len(res))


if __name__ == "__main__":
    main()
