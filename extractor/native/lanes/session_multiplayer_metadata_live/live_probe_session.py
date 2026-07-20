#!/usr/bin/env python3
"""Lane-local READ-ONLY live-root probe for domain 57 (session/multiplayer
metadata).

Passive validation harness only -- NOT the production hot loop. Uses
ReadProcessMemory (via the shared extractor/memory_reader/winmem.py) with no
writes, no injection, no debugger attach. Safe to run against a live, paused
hoi4.exe.

What it validates (per target, with an explicit oracle each):

  TARGET 1  player-country mapping (which human controls which country TAG)
  TARGET 2  pause state (and notes on game speed)
  TARGET 3  host/lobby/player-name metadata (SP observability report)

Base resolution ground truth is the shared per-build profile
(extractor/native/compact_contract/src/build_profile.rs,
COLLECTOR_PROFILE_ID = vanilla_latest_2026_06_30). Its two AOB sentinels
(daily-update prologue @ module+0x1D4390, long-string accessor @ module+0x526A40)
are checked first; if either fails the probe refuses to interpret any root
(fail closed). The RVAs below were confirmed against the running process on
2026-07-02 (live exe sha256 870ebf1f..., which differs from the memory_reader
layout.16022851 gate -- the build_profile RVAs are stable across those two
builds and are the authoritative anchors, per the collector profile).

Run:
    python live_probe_session.py            # human-readable PASS/FAIL
    python live_probe_session.py --expect-tag IRE --expect-name Ireland --expect-index 29
    python live_probe_session.py --json out.json
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

LANE = Path(__file__).resolve().parent
# extractor/native/lanes/<lane>/  -> repo/extractor/memory_reader
MEMORY_READER = LANE.parents[2] / "memory_reader"
for _p in (str(MEMORY_READER),):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import winmem  # noqa: E402

PROC_NAME = "hoi4.exe"

# --- shared build_profile ground truth (module RVAs / sentinels) -------------
#
# SOURCE OF TRUTH: extractor/native/compact_contract/src/build_profile.rs.
# Keep these lane-local copies synchronized with that file until this Python
# validation probe imports generated profile data instead of copying constants:
# - DAILY_UPDATE_SENTINEL
# - GET_LONG_STRING_SENTINEL
# - C_GAME_STATE_DAILY_UPDATE_OFFSET
# - GET_LONG_STRING_OFFSET
#
# Domain-57 is integrated. build_profile.rs now owns the clock slots
# 0x33172E0/0x33172E8, clock vtable RVA 0x2952510, paused byte +0x6C1, and
# CGameState static slot 0x3316E90. These lane-local values remain copies only
# because this Python probe does not import the Rust profile.
DAILY_UPDATE_SENTINEL = bytes(
    [0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41,
     0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D, 0xAC]
)
GET_LONG_STRING_SENTINEL = bytes(
    [0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C,
     0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57]
)
C_GAME_STATE_DAILY_UPDATE_OFFSET = 0x1D4390   # sentinel anchor
GET_LONG_STRING_OFFSET = 0x526A40             # sentinel anchor
CG_STATE_GAMESTATE_OFFSET = 0x3316E90         # static -> CGameState singleton

# --- CGameState member offsets (this build; see lane docs for derivation) ----
GS_CURRENT_DATE = 0x468   # u32 raw hour counter
GS_START_DATE = 0x4A8     # u32 raw hour counter
GS_HUMAN_STATUS_ARRAY = 0x388  # u64 -> byte[count]; A[i]!=0 => country i is human-controlled
GS_SLOT_TAG_ARRAY = 0x358      # u64 -> record[count] stride 0x20; rec+0x00 = 3-char TAG (SSO)
GS_COUNTRY_COUNT = 0x394       # i32 number of country slots
GS_HANDLE_ARRAY = 0x310        # u64 -> qword[count]; handle+8 = i32 slot index, handle+0x10 = localized name (SSO)
SLOT_STRIDE = 0x20
MAX_COUNTRY_COUNT = 4096

# --- clock/pause singleton ---------------------------------------------------
# Two adjacent static pointer slots both point at the SAME clock singleton.
CLOCK_SINGLETON_SLOTS = (0x33172E0, 0x33172E8)
CLOCK_IS_PAUSED_BYTE = 0x6C1   # bool; engine's own `gamespeed 0` path reads this via vtable+0x2e8
CLOCK_VTABLE_OFFSET = 0x2952510

# --- date decode (Clausewitz, no-leap 365d) ----------------------------------
DATE_EPOCH_CONST = 43_799_999
_MONTHS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]


def date_from_raw(stored: int):
    hrs = stored - DATE_EPOCH_CONST
    if hrs < 0:
        return None
    hour = hrs % 24
    rem = hrs // 24
    year = rem // 365
    doy = rem - year * 365
    month = 1
    for length in _MONTHS:
        if doy < length:
            break
        doy -= length
        month += 1
    return "%d.%d.%d.%d" % (year, month, doy + 1, hour)


def read_sso_string(p, addr, maxlen=64):
    """Read an MSVC std::string with SSO buffer at `addr` (buffer +0x00,
    size +0x10, capacity +0x18). Returns str or None."""
    try:
        size = p.u64(addr + 0x10)
        cap = p.u64(addr + 0x18)
    except OSError:
        return None
    if size == 0 or size > maxlen or cap < size or cap > (1 << 20):
        return None
    try:
        chars = addr if cap < 16 else p.u64(addr)
        data = p.read(chars, size + 1)
    except OSError:
        return None
    if data[size] != 0:
        return None
    s = data[:size].decode("latin-1")
    return s if all(32 <= ord(c) < 127 for c in s) else None


def read_tag(p, addr):
    raw = p.try_read(addr, 4)
    nul = raw.find(b"\x00")
    if nul != -1:
        raw = raw[:nul]
    return raw.decode("latin-1", "replace")


def main() -> int:
    ap = argparse.ArgumentParser(description="domain 57 live-root probe (read-only)")
    ap.add_argument("--json", type=Path, help="write JSON evidence to this path")
    ap.add_argument("--expect-tag", help="strict player-country TAG oracle, e.g. IRE")
    ap.add_argument("--expect-name", help="optional strict localized country name oracle")
    ap.add_argument("--expect-index", type=int, help="optional strict gs/slot index oracle")
    ap.add_argument(
        "--allow-unverified-oracle",
        action="store_true",
        help="allow a missing --expect-tag player-country oracle to remain inconclusive",
    )
    ap.add_argument(
        "--expect-paused",
        type=int,
        choices=(0, 1),
        default=1,
        help="expected pause byte value at clock+0x6C1 (default: 1)",
    )
    args = ap.parse_args()
    if (args.expect_name is not None or args.expect_index is not None) and args.expect_tag is None:
        ap.error("--expect-name and --expect-index require --expect-tag")
    expected_tag = args.expect_tag

    report = {
        "passed": False,
        "targets": {},
        "checks": [],
        "process": {},
        "args": {
            "expect_tag": expected_tag,
            "expect_name": args.expect_name,
            "expect_index": args.expect_index,
            "allow_unverified_oracle": args.allow_unverified_oracle,
            "expect_paused": args.expect_paused,
        },
    }

    def check(name, ok, detail, *, result=None, required=True):
        ok = bool(ok)
        result = result or ("PASS" if ok else "FAIL")
        report["checks"].append({
            "name": name,
            "pass": ok,
            "result": result,
            "required": bool(required),
            "detail": detail,
        })
        print("[%s] %s -- %s" % ("PASS" if ok else "FAIL", name, detail))
        return ok

    def inconclusive(name, detail, *, allowed=False):
        suffix = " (allowed by --allow-unverified-oracle)" if allowed else ""
        return check(
            name,
            False,
            "INCONCLUSIVE%s: %s" % (suffix, detail),
            result="INCONCLUSIVE",
            required=not allowed,
        )

    def skipped(name, detail):
        return check(name, False, "SKIP: %s" % detail, result="SKIP")

    p = winmem.open_process(PROC_NAME)
    if p is None:
        check("hoi4.exe running", False, "process not found")
        return _finish(report, args.json, 2)

    try:
        with p:
            base = p.module_base
            sha = (p.fingerprint() or {}).get("sha256")
            report["process"] = {"pid": p.pid, "module_base": hex(base), "sha256": sha}
            print("hoi4.exe pid=%d module_base=0x%X sha256=%s" % (p.pid, base, sha))

            # ---- base resolution gate: build_profile AOB sentinels ----------
            s1 = p.try_read(base + C_GAME_STATE_DAILY_UPDATE_OFFSET, 16) == DAILY_UPDATE_SENTINEL
            s2 = p.try_read(base + GET_LONG_STRING_OFFSET, 16) == GET_LONG_STRING_SENTINEL
            check("build_profile.daily_update_sentinel", s1,
                  "module+0x%X" % C_GAME_STATE_DAILY_UPDATE_OFFSET)
            check("build_profile.get_long_string_sentinel", s2,
                  "module+0x%X" % GET_LONG_STRING_OFFSET)
            if not (s1 and s2):
                check("FAIL_CLOSED", False, "sentinels mismatch; refusing to interpret roots")
                return _finish(report, args.json, 3)

            try:
                gs = p.u64(base + CG_STATE_GAMESTATE_OFFSET)
            except Exception as exc:
                check("CGameState resolved", False,
                      "exception reading module+0x%X: %s: %s"
                      % (CG_STATE_GAMESTATE_OFFSET, type(exc).__name__, exc))
                return _finish(report, args.json, 4)
            gs_ok = gs != 0 and not p.in_module(gs)
            check("CGameState resolved", gs_ok,
                  "u64[module+0x%X] = 0x%X" % (CG_STATE_GAMESTATE_OFFSET, gs))
            if not gs_ok:
                skipped("CGameState dependent target reads", "invalid CGameState pointer")
                return _finish(report, args.json, 4)

            # ---- date oracle (frame timestamp / game_hour_raw) -------------
            try:
                cur = p.u32(gs + GS_CURRENT_DATE)
                start = p.u32(gs + GS_START_DATE)
                cur_date = date_from_raw(cur)
                start_date = date_from_raw(start)
                report["targets"]["game_hour_raw"] = {
                    "current_raw": cur, "current_date": cur_date,
                    "start_raw": start, "start_date": start_date,
                }
                date_ok = cur_date is not None and 1936 <= int(cur_date.split(".")[0]) <= 2100
                check("date oracle plausible", date_ok,
                      "current=%s start=%s" % (cur_date, start_date))
            except Exception as exc:
                check("date oracle plausible", False,
                      "exception: %s: %s" % (type(exc).__name__, exc))

            # =============== TARGET 1: player-country mapping ===============
            try:
                count = p.i32(gs + GS_COUNTRY_COUNT)
                report["targets"]["country_count"] = count
                ok_count = 0 < count < MAX_COUNTRY_COUNT
                check("country count sane", ok_count,
                      "CGameState+0x394 = %d (required 1..%d)"
                      % (count, MAX_COUNTRY_COUNT - 1))
                if not ok_count:
                    skipped(
                        "player-country dependent reads",
                        "country_count sanity failed; not reading status array, handles, or records",
                    )
                else:
                    status_ptr = p.u64(gs + GS_HUMAN_STATUS_ARRAY)
                    slot_base = p.u64(gs + GS_SLOT_TAG_ARRAY)
                    handle_ptr = p.u64(gs + GS_HANDLE_ARRAY)
                    report["targets"]["player_country_roots"] = {
                        "status_ptr": hex(status_ptr),
                        "slot_base": hex(slot_base),
                        "handle_ptr": hex(handle_ptr),
                    }
                    roots_ok = status_ptr != 0 and slot_base != 0 and handle_ptr != 0
                    check("player-country array roots nonzero", roots_ok,
                          "status=0x%X slots=0x%X handles=0x%X"
                          % (status_ptr, slot_base, handle_ptr))
                    if not roots_ok:
                        skipped("player-country rows", "one or more array roots are null")
                    else:
                        def read_humans():
                            # country_index_bridge/tag_db_id is not readable by
                            # this lane-local RPM probe. The available live
                            # cross-check is handle+0x08 slot_index -> slot TAG;
                            # strict --expect-index requires both gs_index and
                            # slot_index to match instead of faking the bridge.
                            bounded_count = count
                            status = p.try_read(status_ptr, bounded_count)
                            if len(status) != bounded_count:
                                raise RuntimeError(
                                    "status array short read: %d/%d bytes"
                                    % (len(status), bounded_count)
                                )
                            rows = []
                            for i, status_byte in enumerate(status):
                                if status_byte:
                                    handle = p.u64(handle_ptr + i * 8)
                                    slot_idx = p.i32(handle + 8)
                                    if not 0 <= slot_idx < bounded_count:
                                        raise RuntimeError(
                                            "slot_index %d out of range for gs_index %d"
                                            % (slot_idx, i)
                                        )
                                    name = read_sso_string(p, handle + 0x10)
                                    tag = read_tag(p, slot_base + slot_idx * SLOT_STRIDE)
                                    rows.append({
                                        "gs_index": i,
                                        "status": status_byte,
                                        "slot_index": slot_idx,
                                        "tag": tag,
                                        "name": name,
                                    })
                            return rows

                        humans1 = read_humans()
                        time.sleep(0.05)
                        humans2 = read_humans()
                        stable = humans1 == humans2
                        report["targets"]["player_country_rows"] = humans1
                        check("player-country rows stable (2 reads)", stable,
                              "read1=%s read2=%s" % (
                                  [(r["tag"], r["name"]) for r in humans1],
                                  [(r["tag"], r["name"]) for r in humans2]))
                        one_human = len(humans1) == 1
                        check("single-player: exactly one human country", one_human,
                              "human rows=%d" % len(humans1))
                        row_ok = False
                        if humans1:
                            r = humans1[0]
                            tag_ok = len(r["tag"]) == 3 and r["tag"].isupper()
                            row_ok = one_human and tag_ok and r["name"] is not None
                            check("human country TAG+name resolve", row_ok,
                                  "TAG=%s name=%r (gs_index=%d slot_index=%d)"
                                  % (r["tag"], r["name"], r["gs_index"], r["slot_index"]))
                        if expected_tag is None:
                            observed = humans1[0] if one_human else humans1
                            inconclusive(
                                "player-country strict oracle",
                                "observed=%s; provide --expect-tag TAG"
                                " [--expect-name NAME] [--expect-index N]" % observed,
                                allowed=args.allow_unverified_oracle,
                            )
                        else:
                            strict_ok = False
                            if one_human:
                                r = humans1[0]
                                index_ok = (
                                    args.expect_index is None
                                    or (r["gs_index"] == args.expect_index
                                        and r["slot_index"] == args.expect_index)
                                )
                                name_ok = args.expect_name is None or r["name"] == args.expect_name
                                strict_ok = (
                                    stable
                                    and row_ok
                                    and r["tag"] == expected_tag
                                    and name_ok
                                    and index_ok
                                )
                                observed_detail = (
                                    "observed TAG=%s name=%r gs_index=%d slot_index=%d"
                                    % (r["tag"], r["name"], r["gs_index"], r["slot_index"])
                                )
                            else:
                                observed_detail = "observed human rows=%d: %s" % (len(humans1), humans1)
                            expected_detail = "expected TAG=%s" % expected_tag
                            if args.expect_name is not None:
                                expected_detail += " name=%r" % args.expect_name
                            if args.expect_index is not None:
                                expected_detail += " index=%d" % args.expect_index
                            check("player-country strict oracle", strict_ok,
                                  "%s; %s" % (observed_detail, expected_detail))
            except Exception as exc:
                check("player-country target exception", False,
                      "%s: %s" % (type(exc).__name__, exc))

            # ================= TARGET 2: pause state ========================
            try:
                clock = p.u64(base + CLOCK_SINGLETON_SLOTS[1])
                clock_alt = p.u64(base + CLOCK_SINGLETON_SLOTS[0])
                same = clock != 0 and clock == clock_alt
                expected_vtable = base + CLOCK_VTABLE_OFFSET
                clock_vtable = p.u64(clock) if clock != 0 else 0
                report["targets"]["pause"] = {
                    "clock_singleton": hex(clock),
                    "clock_singleton_alt": hex(clock_alt),
                    "both_slots_same_object": same,
                    "vtable": hex(clock_vtable),
                    "expected_vtable": hex(expected_vtable),
                    "is_paused_byte_offset": hex(CLOCK_IS_PAUSED_BYTE),
                    "expected_paused": args.expect_paused,
                    "is_paused_reads": [],
                }
                check("clock singleton both static slots agree", same,
                      "slot+0x%X=0x%X slot+0x%X=0x%X"
                      % (CLOCK_SINGLETON_SLOTS[0], clock_alt,
                         CLOCK_SINGLETON_SLOTS[1], clock))
                vtable_ok = clock_vtable == expected_vtable
                check("clock singleton vtable identity", vtable_ok,
                      "vtable=0x%X expected=0x%X (module+0x%X)"
                      % (clock_vtable, expected_vtable, CLOCK_VTABLE_OFFSET))
                if same and vtable_ok:
                    paused_reads = [p.u8(clock + CLOCK_IS_PAUSED_BYTE) for _ in range(3)]
                    report["targets"]["pause"]["is_paused_reads"] = paused_reads
                    pause_ok = len(set(paused_reads)) == 1 and paused_reads[0] == args.expect_paused
                    check("pause flag matches expected oracle", pause_ok,
                          "clock+0x%X = %s (expected %d; flip unverified, read-only)"
                          % (CLOCK_IS_PAUSED_BYTE, paused_reads, args.expect_paused))
                else:
                    skipped("pause flag reads", "clock identity failed; not trusting clock+0x6C1")
            except Exception as exc:
                check("pause target exception", False,
                      "%s: %s" % (type(exc).__name__, exc))

            # =============== TARGET 3: host/lobby metadata ==================
            report["targets"]["host_lobby_metadata"] = {
                "status": "SP-limited",
                "note": ("Single-player session: only the local human country is "
                         "observable (via CGameState+0x388 status array -> TAG). No "
                         "CGameLobby/CPlayerLobby player-connection structure is "
                         "populated, so player_name / player_key_hash / session_slot / "
                         "host / connected flags cannot be validated from SP. Fail "
                         "closed on all MP-only fields."),
            }
            print("[INFO] TARGET 3 host/lobby: SP-limited (see JSON note); MP-only fields fail closed")
    except Exception as exc:
        check("probe exception", False, "%s: %s" % (type(exc).__name__, exc))

    return _finish(report, args.json, 1)


def _finish(report, out, failure_code):
    report["passed"] = all(
        (not c.get("required", True)) or c.get("pass") is True
        for c in report.get("checks", [])
    )
    if out is not None:
        out.write_text(json.dumps(report, indent=2), encoding="utf-8")
        print("wrote %s" % out)
    return 0 if report["passed"] else failure_code


if __name__ == "__main__":
    raise SystemExit(main())
