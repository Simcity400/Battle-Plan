# memory_reader/combat — live active-combat capture (DESIGN Tier C)

Read-only reverse-engineered capture of HOI4's **live active land-combat** structures,
for the high-frequency combat-sampling loop in DESIGN.md §4 (Tier C) / §9 step 4.

Build-pinned to `hoi4.exe` sha256 `16022851…` ("Operation Postern v1.19.1.0.b49d").
All reads are **read-only** (`PROCESS_VM_READ | PROCESS_QUERY_INFORMATION`) — passive,
no MP desync, any cadence. Offsets are build-specific; the enumerator **refuses to run**
unless the live exe hash matches.

## Files
- `combat_offsets.draft.json` — solved offsets/strides + confidence + honest residual.
  Same draft style as `../division_offsets.draft.json` etc. Merge into the per-build
  layout during reader productionization.
- `combat_enum.py` — `CombatReader`: build gate + `enumerate_land_combats()`. Self-contained
  ctypes RPM/VirtualQuery helper (does **not** edit `winmem.py`).
- `combat_sampler.py` — reference lifecycle loop: diffs successive snapshots into
  `BATTLE_START / BATTLE_UPDATE / BATTLE_END` events keyed on `combat_id`.

## What's solved (validated 13/13 vs a time-aligned melted save)
A live land battle is a heap **CLandCombat** object:
- vtable `MODULE+0x2980CB8` @ +0x00 (enumeration anchor; scan heap for it, filter type==62)
- type tag `62` @ +0x08, **combat_id** @ +0x0C
- **duration** @ +0x30, **day** (combat age) @ +0x34
- **location** (province id): `*(body+0x28)` → front object → +0xA4
- **participants — attacker AND defender, exact, from memory:**
  `*(body+0x18)` is the **attacker side** object (`CCombatSide`, vtable `MODULE+0x2980AC0`).
  `attacker_side+0x18` → the per-combat **sides registry** (`MODULE+0x2980BD8`): `+0x18` =
  `{combat_id,62}` identity, `+0x28` = attacker side, **`+0x30` = defender side**, `+0x38` =
  front. Each side is a small-vector `{data@+0x20, u32 cap@+0x28, u32 size@+0x2C}`; read
  exactly `size` `CDivision*` from `data`.

Validation: enumeration, combat_id, location, day, duration all **13/13**; **attacker set
13/13 AND defender set 13/13** (both from memory; cross-checked through `CombatReader`). The
save is a validation backstop only — the runtime path is memory-only.

## Defender solve notes
- The earlier "defender residual" assumed a `std::vector<CDivision*>` triple and a single
  "sides container"; both were wrong. The list is a small-vector with a `{data,cap,size}`
  inline header (`+0x28` is **capacity**, not the count — use `size@+0x2C`), and the defender
  side is a *sibling* object reached via the registry, not a field of the combat body.
- `CDivision+0x1E0` (the division's front segment) is only a province-level prefilter (10/13,
  over-collects when two combats share a province) — documented, not the path used.
- Production hardening: vtable identity on every hop, registry `{cid,62}` assert, count-driven
  read, region-clamped reads, `size<=cap` sanity, **double-read revalidation** (realloc-race
  guard), fail-closed with per-reason `CombatReader.reject_stats` telemetry.

## Remaining residual
- **Naval / air combats**: unmapped — none were active at the captured snapshot. Re-run the
  save-anchored discovery during a snapshot with active naval/air battles (the land
  side/registry layout is a strong starting hypothesis).
- **day/duration semantics**: value-correlated to the save keys; exact runtime meaning
  (which increments, when) needs sub-day sampling.
- **Fundamental unobservables**: micro-engagements that start *and* end between two samples;
  same-id reuse without an age/generation discontinuity.

## Usage
```
python combat_enum.py                 # one-shot list of active land combats
python combat_sampler.py --interval 2 # live lifecycle event stream (read-only)
```
The production scheduler (snapshot store, date/tick sentinel, fail-closed validators) lives
in the reader productionization phase; this package is the combat data primitive it calls.
