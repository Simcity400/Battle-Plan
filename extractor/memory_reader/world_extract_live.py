"""world_extract_live.py - LIVE world-scoped EXTRACT-domain reader (READ-ONLY).

Covers the LOWER-PRIORITY world residuals (currently save-only in the EXTRACT tier):
SCENARIO META + global FLAG/VARIABLE store + WORLD-TENSION / threat + STRATEGIC-AIR
base-capacity table. Pinned on build 16022851; see world_extract_offsets.draft.json
for the full evidence (gamestate.cpp serializer FUN_1401f2b50 + scopedvariable.cpp).

WHAT VALIDATES (clean live==save win, this frame == the 1946 save moment):
  * SCENARIO META: current_game_date (gs+0x468) + start_date (gs+0x4A8), both i32 Jomini
    DateHour. Decode == save world.json exactly (1946.5.19.9 / 1936.1.1.12). Double-read.

WHAT IS STRUCTURE-CONFIRMED BUT NOT LIVE-ROOTED (documented, NOT validated):
  * GLOBAL VARIABLE / FLAG store: the container is a 0x30-byte-node hash table
    (key std::string @ node+0x00, value @ node+0x28; elem base @ container+0x18, count
    @ container+0x24 - Ghidra FUN_140bbe830). _pVariables is SCOPE-resolved
    (scopedvariable.cpp), NOT a flat gamestate member; the canonical live ROOT was not
    isolated within budget. The reader CONFIRMS all sampled key STRINGS are present in
    the live heap (frame-stable, validatable) and reports the blocker.
  * WORLD_TENSION scalar: computed+clamped 0..100 from the threat ledger (no stored key).
  * THREAT ledger: lead only (final_threat looks i32-fixed-point x1000; not a pinned array).
  * STRATEGIC_AIR table: serialized as a polymorphic gamestate child; root not isolated.

HARD RULES honored (same as the sibling live readers):
  * READ-ONLY memory (RPM + VirtualQueryEx only); no writes / injection / thread suspend;
    keep the game PAUSED -> passive reads, any cadence, never desyncs MP.
  * Build-gated: refuses to run unless the live hoi4.exe sha256 matches the build.
  * No personal paths; self-contained; reuses winmem.py + the Jomini date codec.

PUBLIC API
  read_scenario_meta(p) -> {"current_game_date","current_game_date_raw",
                            "start_date","start_date_raw","_gamestate_addr"}
      AUTHORITATIVE. Live current date + start date (decoded "Y.M.D.H"), validated exact.
  probe_key_presence(p, keys=None) -> {key: [heap_addr, ...]}
      Confirms which global flag/variable KEY strings are present in live heap (frame-stable).
  read_world(p) -> {"scenario_meta":{...}, "key_presence":{...}, "blockers":{...}}
      Convenience bundle: the validated scenario meta + the key-presence sample + the
      precise blockers for the unreached domains.
  read_all(p) -> read_world(p)   (alias for the house-style entrypoint)

Usage
  python world_extract_live.py            # dump scenario meta + key presence
  python world_extract_live.py --validate # live==save report (scenario dates; key sample)
"""

import argparse
import json
import os
import struct
import sys

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)

import winmem  # noqa: E402

EXPECTED_SHA = "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120"
PROC_NAME = "hoi4.exe"

# --- gamestate root (build 16022851) ---
GAMESTATE_RVA = 0x33048C0          # u64[module_base + this] -> CCurrentGameState singleton
GS_CURRENT_DATE = 0x468            # i32 Jomini DateHour: current game date
GS_START_DATE = 0x4A8             # i32 Jomini DateHour: scenario start date

# --- Jomini DateHour codec (mirrors politics_residuals_live) ---
DATE_EPOCH_CONST = 43799999        # stored = my_naive_hours + 43799999  (= 5000*365*24 - 1)
_MONTH_LEN = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
_DAYS_BEFORE = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
_LE_MIN = (1850 * 365) * 24 + DATE_EPOCH_CONST
_LE_MAX = (2200 * 365) * 24 + DATE_EPOCH_CONST

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

# Default key-presence sample (stable strings from the 1946 save GT).
_DEFAULT_FLAG_KEYS = [
    "GER_has_started_war", "FRA_already_surrendered", "AAT_trotsky_in_mexico",
    "GER_lost_core_state", "BUL_fatherland_front_coup_flag",
]
_DEFAULT_VAR_KEYS = [
    "allies_score", "achievement_days_since_bus_left_stockholm",
    "tension_scaling_base_country", "asian_and_oceanian_countries^num",
]


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
    """Decode the live i32 Jomini DateHour binary -> 'Y.M.D.H'. None if implausible."""
    if stored is None or not (_LE_MIN <= stored <= _LE_MAX):
        return None
    hrs = stored - DATE_EPOCH_CONST
    h = hrs % 24
    rem = hrs // 24
    y = rem // 365
    doy = rem - y * 365
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


def _gamestate(p):
    """The live CCurrentGameState singleton pointer (or None)."""
    try:
        gs = p.u64(p.module_base + GAMESTATE_RVA)
    except OSError:
        return None
    if not gs or not (HEAP_LO <= gs <= HEAP_HI):
        return None
    return gs


def _i32(p, a):
    try:
        return p.i32(a)
    except OSError:
        return None


# ==========================================================================
# 1. SCENARIO META  (AUTHORITATIVE - validated live==save exact)
# ==========================================================================
def read_scenario_meta(p):
    """LIVE current game date + scenario start date (decoded 'Y.M.D.H'). Each scalar is
    DOUBLE-READ (anti-torn). Returns None per field if the gamestate / value is bad."""
    gs = _gamestate(p)
    out = {
        "current_game_date": None, "current_game_date_raw": None,
        "start_date": None, "start_date_raw": None,
        "_gamestate_addr": gs,
    }
    if gs is None:
        return out
    for off, key in ((GS_CURRENT_DATE, "current_game_date"),
                     (GS_START_DATE, "start_date")):
        a = gs + off
        v1 = _i32(p, a)
        v2 = _i32(p, a)
        if v1 is None or v1 != v2:
            continue
        ds = decode_datehour(v1)
        if ds is None:
            continue
        out[key] = ds
        out[key + "_raw"] = v1
    return out


# ==========================================================================
# 2. KEY PRESENCE  (frame-stable validator for the flag/variable store)
# ==========================================================================
def probe_key_presence(p, keys=None):
    """Confirm which global flag/variable KEY strings exist in live heap.

    The KEY strings are frame-independent (a flag set in 1939 keeps its name forever),
    so this is a robust validator even though the live key->VALUE root is unpinned.
    Returns {key: [heap_addr, ...]} (capped at 12 addrs/key). READ-ONLY heap scan."""
    if keys is None:
        keys = _DEFAULT_FLAG_KEYS + _DEFAULT_VAR_KEYS
    needles = [(k, (k + "\x00").encode("latin-1")) for k in keys]
    found = {k: [] for k in keys}
    for rbase, rsize in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if rsize > 64 * 1024 * 1024:
            continue
        try:
            blob = p.read(rbase, rsize)
        except OSError:
            continue
        for k, nd in needles:
            if len(found[k]) >= 12:
                continue
            s = 0
            while True:
                i = blob.find(nd, s)
                if i < 0:
                    break
                found[k].append(rbase + i)
                s = i + 1
                if len(found[k]) >= 12:
                    break
    return found


# ==========================================================================
# bundle
# ==========================================================================
_BLOCKERS = {
    "global_variable_store": (
        "STRUCTURE CRACKED (0x30-byte hash nodes, key std::string @+0x00, value @+0x28; "
        "elem base @container+0x18, count @container+0x24 - Ghidra FUN_140bbe830). "
        "_pVariables is SCOPE-resolved (scopedvariable.cpp), not a flat gamestate member; "
        "canonical live ROOT not isolated. NEXT: breakpoint/log FUN_140535a40 (or the "
        "FUN_140bbe830 caller) on a known global key (allies_score=11.0), capture the "
        "returned pVariables container + owner ptrs, walk back to a gamestate-reachable "
        "root, then validate node count==260 + a value sample (frame matches the save)."),
    "global_flag_store": (
        "Key strings + a flag-key std::string vector present in live heap, but the "
        "{value,date} parallel store (same scopedvariable.cpp subsystem) is unpinned, and "
        "the flag-key vector has no sentinel (heap-contiguous with unrelated string "
        "vectors). NEXT: same scope-resolved container root as variables."),
    "world_tension_scalar": (
        "Not a stored key - computed+clamped 0..100 from the threat ledger via NDiplomacy "
        "TENSION_TIME_SCALE_* / TENSION_STATE_VALUE defines. NEXT: find the threat-manager "
        "cached scalar, or recompute from the live threat ledger + TENSION_* defines (do "
        "NOT just sum final_threat and clamp)."),
    "threat_ledger": (
        "LEAD ONLY: final_threat 29.995 (Danzig) appears as i32==29995 (x1000 fixed-point) "
        "at 37 sites; f32/f64 absent. But 29995 is a heap-common small int and no tight "
        "cluster of consecutive GT threat values was found -> not a pinned array. NEXT: "
        "find an allocation with MULTIPLE known final_threat*1000 values at a fixed stride "
        "(the ledger record family) + negative controls."),
    "strategic_air_capacity_table": (
        "337 air_base records (save). The strategic_air manager is a POLYMORPHIC gamestate "
        "CHILD (CSelectionGroupWriter in the gamestate serializer FUN_1401f2b50), not a "
        "flat offset. NEXT: trace the gamestate child-object list / strategic_air vtable to "
        "the air_base record array (record: id{id,type}, state, capacity int, base, level, "
        "user list)."),
}


def read_world(p):
    """Convenience bundle: validated scenario meta + key-presence sample + blockers."""
    meta = read_scenario_meta(p)
    presence = probe_key_presence(p)
    return {
        "source": "live",
        "scenario_meta": meta,
        "key_presence": {k: [hex(a) for a in v] for k, v in presence.items()},
        "key_presence_counts": {k: len(v) for k, v in presence.items()},
        "blockers": _BLOCKERS,
    }


def read_all(p):
    """House-style entrypoint alias."""
    return read_world(p)


# ==========================================================================
# validation vs the time-aligned melt
# ==========================================================================
def _load_save_world(melt_path):
    """Parse the world oracle from the melt for GT (scenario dates + key set)."""
    oracle = os.path.join(os.path.dirname(_THIS), "save_oracle")
    sys.path.insert(0, oracle)
    try:
        import deep_common  # noqa: F401  (ensures shared tokenizer importable)
        import parse_world
    except Exception as e:  # noqa: BLE001
        return None, "could not import save_oracle/parse_world: %s" % e
    from pathlib import Path
    data = open(melt_path, "r", encoding="utf-8", errors="replace").read()
    bundle, _ = parse_world.build(data, Path(melt_path), True)
    return bundle.get("world.json"), None


def _validate(p, melt_path=None):
    """live==save report. Scenario dates EXACT; flag/var key sample present in heap."""
    if melt_path is None:
        melt_path = os.path.join(os.path.dirname(_THIS),
                                 "re_scratch", "melt_sov_1946.txt")
    lines = ["=== WORLD-EXTRACT live==save validation ==="]
    meta = read_scenario_meta(p)

    gt, err = (None, None)
    if os.path.exists(melt_path):
        gt, err = _load_save_world(melt_path)
    if err:
        lines.append("WARN: %s" % err)

    # --- scenario dates ---
    date_ok = 0
    date_tot = 0
    if gt is not None:
        save_cur = gt.get("meta", {}).get("save_date")
        save_start = gt.get("scenario", {}).get("start_date")
        for label, live_v, save_v in (
                ("current_game_date", meta.get("current_game_date"), save_cur),
                ("start_date", meta.get("start_date"), save_start)):
            date_tot += 1
            ok = (live_v is not None and save_v is not None and live_v == save_v)
            date_ok += 1 if ok else 0
            lines.append("  %-18s live=%s save=%s  %s"
                         % (label, live_v, save_v, "EXACT" if ok else "MISMATCH"))
    else:
        lines.append("  (no melt GT; live dates: current=%s start=%s)"
                     % (meta.get("current_game_date"), meta.get("start_date")))

    # --- key presence ---
    sample_keys = list(_DEFAULT_FLAG_KEYS + _DEFAULT_VAR_KEYS)
    # cross-check: only count keys that actually exist in the save GT (sanity)
    if gt is not None:
        save_flags = set(gt.get("flags", {}).keys())
        save_vars = set(gt.get("variables", {}).keys())
        sample_keys = [k for k in sample_keys
                       if k in save_flags or k in save_vars] or sample_keys
    presence = probe_key_presence(p, sample_keys)
    present = sum(1 for k in sample_keys if presence.get(k))
    lines.append("  key presence (live heap): %d/%d sampled keys present"
                 % (present, len(sample_keys)))
    for k in sample_keys:
        lines.append("    %-42s %s"
                     % (k, "present (%d)" % len(presence[k]) if presence.get(k)
                        else "ABSENT"))

    lines.append("")
    lines.append("SUMMARY: scenario dates %d/%d EXACT; key-presence %d/%d present. "
                 "Other world domains RESISTED (see blockers in the draft / read_world)."
                 % (date_ok, date_tot, present, len(sample_keys)))
    return "\n".join(lines)


# ==========================================================================
# CLI
# ==========================================================================
def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--validate", action="store_true",
                    help="live==save report (scenario dates + key sample)")
    ap.add_argument("--melt", default=None, help="path to the melt GT (.txt)")
    ap.add_argument("--force", action="store_true",
                    help="run even if the build sha mismatches (UNVERIFIED)")
    args = ap.parse_args()

    p = winmem.open_process(PROC_NAME)
    check_build_gate(p, force=args.force)

    if args.validate:
        print(_validate(p, args.melt))
        return 0

    print(json.dumps(read_world(p), indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
