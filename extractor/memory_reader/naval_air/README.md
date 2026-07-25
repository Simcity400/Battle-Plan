# naval_air — live NAVAL & AIR theater reader (READ-ONLY)

Reads the data behind HOI4's **Naval** and **Air** map modes plus fleet/air positions from a
running `hoi4.exe` (build `16022851`), anchored on a time-aligned melted save. Mirrors the
land-combat / division / state-owner cracks: vtable-scan enumeration, count-driven reads,
fail-closed validators, build gate, arena-scoped read-only memory.

## Files
- `naval_air_offsets.draft.json` — every solved offset + the residuals. Authoritative.
- `naval_air_reader.py` — `NavalAirReader`, the production reader. Self-contained ctypes; does
  NOT edit `winmem.py`/`locate.py`. Run `python naval_air_reader.py` for a live dump.

## What is solved
| target | status | anchor |
|---|---|---|
| **Sea-zone control** (Naval map mode = naval supremacy/dominance) | **SOLVED 33/33** | region obj `MODULE+0x29468B8` → `+0x58` id, `+0xE8` container `MODULE+0x295DF30` → entries (`MODULE+0x295DEE0`, stride 0x50) |
| **Task-force positions** (Naval markers) | **SOLVED** (64 fleets; 131 TFs; loc 81/131) | task_force `MODULE+0x293C8B0` type 61, `current_province_id@+0x18` |
| **Air-wing enumeration** | **SOLVED 265/265** + `count@+0x6C` (18/18) | wing `MODULE+0x29540C0` type 69, id `@+0x0C` |
| **Air-wing owner** | **SOLVED 265/265** | wing`+0x28`→pool `MODULE+0x2954118`→`+0x40`→country `MODULE+0x2932248`→`+0x90` tag-db-id |
| **Air-wing mission region** | **SOLVED 56/56** | wing`+0xB0`→region obj `MODULE+0x29468B8`→`+0x58`; null = idle |
| **Air superiority** (Air map mode) | **SOLVED 12/12 buckets** (derived) | sum `effective_planes` (wing`+0x160`) by (mission region, owner); == save oracle exactly |
| **Naval / air combat** | **UNRESOLVED_NOT_LIVE** — land-mirror **REFUTED** (RE 2026-06-25) | detector enumerates `CCombatSide` (`MODULE+0x2980AC0`) + classifies participants; 0 task-force/wing sides live. Ship vtable pinned `MODULE+0x2931998` (t51). Re-anchor = active fleet-vs-fleet battle save. §5 |

## API (read-only; build-gated)
```python
from naval_air.naval_air_reader import NavalAirReader
r = NavalAirReader()
ok, sha = r.verify_build()                 # refuse unless exe sha256 matches the pinned build
szc  = r.enumerate_sea_zone_control(tag_table=tbl)  # {region_id: [{controller, current, target,
                                                    #   base_target, individual_ratio, ...}]}  NAVAL
tfs  = r.enumerate_task_forces()           # [{tf_id, current_province_id, body_addr}]
fls  = r.enumerate_fleets()                # [{fleet_id, body_addr}]
wings= r.enumerate_air_wings(tag_table=tbl)  # [{wing_id, count, owner_tagid, owner, mission_region,
                                             #   effective_planes, body_addr}]
asup = r.enumerate_air_superiority(tag_table=tbl)  # {region_id: {by_owner:{TAG:{effective_planes,
                                                   #   wings}}, leader, total}}  AIR map mode
nc   = r.enumerate_naval_combats()         # [] today (land-mirror refuted) - task-force/ship sides
ac   = r.enumerate_air_combats()           # [] today - air-wing sides; both are REAL detectors
r.reject_stats                             # per-reason fail-closed telemetry
```
`tag_table` is optional (`../live_layers/tag_table.py`) to resolve controller
tag-db-ids → 3-letter TAGs.

## Semantics (important)
`region.dominance` is **naval supremacy / sea-zone dominance** (the Naval map mode) — confirmed by
mod defines (`NNavy.DOMINANCE_DAILY_GAIN_FACTOR`, ship `naval_dominance_factor`,
`minimum_seazone_dominance`, `GFX_naval_dominance_texticon`) and by all dominance regions being
fleet-operated. It is **NOT** air superiority. Air superiority is a **derived** quantity (no stored
per-region accumulator) — recomputed per tick and viewer-relative — so `enumerate_air_superiority()`
builds it live by summing each present wing's `effective_planes` (wing`+0x160`) bucketed by mission
strategic_region (wing`+0xB0`→region`+0x58`) and owner (wing`+0x28`→pool→`+0x40`→country`+0x90`).
`effective_planes` is the faithful color proxy for the exact per-wing scalar (= `effective_planes ×
variant air_superiority stat × modifiers`, a drifting cache); validated == the save oracle 12/12
buckets exactly. See `../../documents/archive/research/RESEARCH_naval_air.md` §4b.

## Hard rules
READ-ONLY (`PROCESS_VM_READ | PROCESS_QUERY_INFORMATION`); no writes/injection/suspension;
build-gated by exe sha256; reads clamped to VirtualQuery region ends; arena-scoped scans
(`0x1F600000000..0x1FA00000000`); fail-closed + double-read revalidation of the dominance
container header; NULL/empty container → `[]` (no data), not a hard failure.

See `../../documents/archive/research/RESEARCH_naval_air.md` for the full method,
validation, Codex review, and residuals.
