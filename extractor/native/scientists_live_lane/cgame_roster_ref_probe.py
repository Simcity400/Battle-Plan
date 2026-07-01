"""Bounded live probe for the CGameState scientist-roster reference candidate.

This is a read-only validation helper for the scientists_live lane. It checks
whether `CGameState+0x520/+0x524` is a usable roster root on the current build
and contrasts that with the validated country-owned `CCountry+0xFF0` route.
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT))

from memory_reader.winmem import Process, find_pid  # noqa: E402
from memory_reader import projects_live  # noqa: E402
from memory_reader import scientists_roster_probe as roster_probe  # noqa: E402


CG_STATE_DATE_OFFSET = 0x3316E90
CGAMESTATE_SCIENTIST_ROSTER_PRIMARY = 0x520
CGAMESTATE_SCIENTIST_ROSTER_FALLBACK = 0x524


def _vector_hits(p: Process, header: int, character_vtable: int, scientist_vtable: int):
    if not roster_probe.heap_ptr(header):
        return []
    return roster_probe.vector_candidates_from_header(
        p, header, character_vtable, scientist_vtable
    )


def summarize(pid: int) -> dict:
    with Process(pid, "hoi4.exe") as p:
        fp = p.fingerprint()
        if not projects_live.apply_build_profile(fp.get("sha256")):
            raise SystemExit(f"unsupported hoi4.exe sha256: {fp.get('sha256')}")

        base = p.module_base
        gamestate = roster_probe.safe_u64(p, base + CG_STATE_DATE_OFFSET)
        if not gamestate or not roster_probe.heap_ptr(gamestate):
            raise SystemExit("CGameState global did not resolve to a heap pointer")

        character_vtable = base + projects_live.CHAR_VT_RVA
        scientist_vtable = base + projects_live.SCI_VT_RVA
        statuses, country_diag = roster_probe.country_statuses(p, base, gamestate)
        exact_candidates = roster_probe.scan_status_vectors(
            p,
            base,
            statuses,
            country_diag.get("country_count"),
            None,
            scan_fallbacks=False,
        )
        exact_vectors = [
            c
            for c in exact_candidates
            if c["source"] == "ghidra_exact_country_ff0_plus_a0"
            and c["count_offset"] == 12
            and c["failed_slots"] == 0
        ]
        exact_union = sorted(
            {
                int(char_id)
                for candidate in exact_vectors
                for char_id in candidate["char_ids"]
            }
        )

        offsets = {}
        for offset in (
            CGAMESTATE_SCIENTIST_ROSTER_PRIMARY,
            CGAMESTATE_SCIENTIST_ROSTER_FALLBACK,
        ):
            addr = gamestate + offset
            u32_value = roster_probe.safe_u32(p, addr)
            u64_value = roster_probe.safe_u64(p, addr)
            direct_hits = _vector_hits(p, addr, character_vtable, scientist_vtable)
            deref_hits = (
                _vector_hits(p, int(u64_value), character_vtable, scientist_vtable)
                if u64_value is not None and roster_probe.heap_ptr(int(u64_value))
                else []
            )
            ui_tag_hits = [
                c
                for c in exact_vectors
                if u32_value is not None and int(c["tag_db_id"]) == int(u32_value)
            ]
            offsets[f"+0x{offset:x}"] = {
                "addr": addr,
                "u32": u32_value,
                "u64": u64_value,
                "u64_is_heap_ptr": bool(
                    u64_value is not None and roster_probe.heap_ptr(int(u64_value))
                ),
                "direct_vector_hits": len(direct_hits),
                "deref_vector_hits": len(deref_hits),
                "ui_tag_exact_country_vectors": len(ui_tag_hits),
                "ui_tag_char_ids": sorted(
                    {
                        int(char_id)
                        for candidate in ui_tag_hits
                        for char_id in candidate["char_ids"]
                    }
                ),
            }

        return {
            "pid": pid,
            "sha256": fp.get("sha256"),
            "base": base,
            "gamestate": gamestate,
            "country_count": country_diag.get("country_count"),
            "program_status_count": len(statuses),
            "cgame_offsets": offsets,
            "exact_country_ff0_vectors": len(exact_vectors),
            "exact_country_ff0_char_ids": len(exact_union),
            "verdict": (
                "reject_cgame_0x520_0x524_as_roster_root;"
                " use CCountry+0xFF0 -> roster+0xA0 vector instead"
            ),
        }


def compact_print(result: dict) -> None:
    print(f"pid={result['pid']}")
    print(f"sha256={result['sha256']}")
    print(f"base=0x{result['base']:x}")
    print(f"gamestate=0x{result['gamestate']:x}")
    print(f"country_count={result['country_count']}")
    print(f"program_status_count={result['program_status_count']}")
    for key, row in result["cgame_offsets"].items():
        u64_value = row["u64"]
        u64_text = "None" if u64_value is None else f"0x{u64_value:x}"
        print(
            f"offset={key} addr=0x{row['addr']:x} u32={row['u32']} "
            f"u64={u64_text} heap_ptr={row['u64_is_heap_ptr']} "
            f"direct_vectors={row['direct_vector_hits']} "
            f"deref_vectors={row['deref_vector_hits']} "
            f"ui_tag_vectors={row['ui_tag_exact_country_vectors']} "
            f"ui_tag_char_ids={row['ui_tag_char_ids']}"
        )
    print(f"exact_country_ff0_vectors={result['exact_country_ff0_vectors']}")
    print(f"exact_country_ff0_char_ids={result['exact_country_ff0_char_ids']}")
    print(f"verdict={result['verdict']}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, default=None)
    args = parser.parse_args()
    pid = args.pid or find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe is not running")
    compact_print(summarize(pid))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
