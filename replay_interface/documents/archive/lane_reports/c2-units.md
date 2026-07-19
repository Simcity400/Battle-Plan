# C2 — Unit placement lane report

**Plan item:** C2 (division/fleet/wing positions over time, served from the
recording and placed on the map — the DATA + PLACEMENT half of unit rendering).
The counter VISUAL law (screen-space size/anchor/stacking) is CAP6-gated and is
NOT in this lane.

**Branch/worktree:** `lane/c2-units` @ `C:\Users\Example\hoi4_lanes\c2-units`.

## What landed

- New module `replay_interface/native/map_viewer/src/unit_placement.rs`:
  timeline unit inputs → world-space placement records per hour, with a
  province→world anchor table built FILE-first from `map/unitstacks.txt`.
- Registered in `map_viewer/src/lib.rs` (additive `mod` + `pub use`).
- `replay_core` added as a **dev-dependency** of `map_viewer` (test-only) for the
  fixture-driven gate; production stays decoupled (replay_app is the wiring
  layer, matching `naval_supremacy_tint`/`map_furniture` which take primitives).
- Gate `map_viewer/tests/unit_placement_gates.rs`.
- Requirements filed: `REPLAY_DATA_REQUIREMENTS.md` item 8 **PLACEMENT GAP** +
  `EXTRACTION_NEEDS.md` A7 (air-wing airbase province).

## Placement law table

| Element | Class | Rule |
|---|---|---|
| Land division anchor province | FILE (recording) | the division's own `location_province` for the hour |
| Fleet/task-force anchor province | FILE (recording) | the task force's own `location_province` (sea / naval-base province) |
| Air-wing anchor province | **FIT** | wings carry no on-map province today → air-base **state**'s representative province (caller-supplied `state_id→province`); future recording `location_province` (airbase province) honored first |
| Province → world point | FILE | `map/unitstacks.txt` **stack-centre** row (`type == 0`) — the engine's own per-province unit-stack centre `(x,y,z)` in world space; lowest slot kind used when a province lacks `type 0` (still FILE, provenance records the kind) |
| Province → world point (fallback) | FIT | province pixel **centroid** (the `map_furniture` DERIVED_APPROXIMATION) injectable via `ProvinceAnchorTable::insert` for provinces missing from unitstacks; not auto-loaded (raster is heavy) |
| Which unitstacks **slot** a unit occupies; multi-unit **stacking**; screen-space size/offset/aggregation-by-zoom | **UNKNOWN / CAP6** | ~20 slot kinds per province (`type 0..=38`) ship, but the semantic enum (land/naval/air, sub-slot order) is NOT shipped; visual law capture-gated |

Every anchor carries an `AnchorProvenance` (UnitStackBaseSlot / UnitStackLowestSlot / ProvinceCentroid / Synthetic) and every placement an `AnchorProvinceSource` (LocationProvince / AirBaseStateRepresentative / None) + `PlacementStatus` (Placed / AnchorProvinceUnavailable / AnchorProvinceUnresolved). Nothing is invented — a missing province is Unavailable, a province absent from the table is Unresolved, both with no world point.

## Interpolation policy

**Step function — hold the last hourly sample until the next hour** (plan C2: no
invented smoothing without capture evidence). Position is the `location_province`
of the frame at-or-before the requested hour. Because
`WorldTimeline::state_at`/`seek` already resolve to the nearest snapshot
at-or-before, computing placement once per resolved frame yields the step-hold
for free, and `seek(h) == play_to(h)` placement equality is inherited from the
timeline. `moving`/`move_target_province`/`move_progress` are available for a
FUTURE evidence-based interpolation but do NOT move the anchor today.

## Gate numbers

`cargo build -p map_viewer` — clean (pre-existing warnings only).

`cargo test -p map_viewer`:
- **641 passed / 0 failed / 1 ignored** (lib; +10 new inline `unit_placement` tests)
- **5 passed / 0 failed** — `tests/unit_placement_gates.rs`:
  1. `division_movement_placement_follows_fixture_path` — unit IDs `{101:1,201:1}` preserved every hour; mover anchor province == certified `location_province`; distinct province path == `[1000, 1001, 2000]`; world point == table entry; all in map bounds.
  2. `naval_air_placement_anchors_fleet_and_wing` — fleet `501:1` at sea province 3000 (FILE); wing `800:1` via air-base state 30 (FIT); all in bounds.
  3. `wing_without_state_mapping_is_unavailable` — fail-closed, no invention.
  4. `seek_equals_play_placement_and_step_holds` — seek==play forward/backward + scrub step-hold, both unit fixtures.
  5. `gated_vanilla_unitstacks_anchor_table_places_fixture_provinces` — real `map/unitstacks.txt` → >13k stack-centre anchors; fixture provinces 1000/1001/2000/3000 all resolve FILE (UnitStackBaseSlot), in-bounds; division fixture places FILE-derived every hour. (Runs on the F: install; skips if absent.)
- All other map_viewer integration tests unchanged/green.

`cargo test -p replay_core` — **30 passed / 0 failed** (unchanged; read-only, as mandated).

No new `cargo fmt` diffs from this lane (my files formatted; the only remaining `--check` diffs — `building_files` export block, `dump_map_furniture.rs` — pre-date this lane).

## replay_core view gaps found

- **Air-wing on-map province:** `WingUnitRow` exposes `air_base` (opaque id
  string, e.g. `"3000:1"`), `state_id`, `strategic_region_id` — none is a clean
  province. Wing placement is therefore a **state-centroid FIT**. Filed:
  REPLAY_DATA_REQUIREMENTS item 8 PLACEMENT GAP + EXTRACTION_NEEDS A7. Divisions
  and fleet task forces already carry a clean `location_province` and place
  FILE-derived. (No replay_core code change requested — the gap is a recording
  field, not a view bug.)
- Otherwise the certified views (`division_units()`, `fleets()`, `wings()`) carry
  exactly what placement needs; the `Leaf` trichotomy maps cleanly to
  `Placed`/`Unavailable`.

## What CAP6 must deliver (for the visual law that slots onto this seam)

The module places every unit at the province **stack centre**; the following are
UNKNOWN until CAP6 (close-zoom no-FoW frame with land units + a naval stack):
1. **Slot selection** — which `unitstacks.txt` `type 0..=38` slot a given unit
   category (land/naval/air) occupies (semantic enum not shipped).
2. **Stacking/aggregation-by-zoom** — when multiple divisions merge into one
   counter and how the sub-slots (`type 1..=38`, the cluster around the centre)
   are assigned as a stack grows.
3. **Screen-space size / anchor offset** — rendered quad size, DPI/zoom scaling,
   and the `unit_counter_zoomed_in_offset {y=20}` activation (from
   `unit_counter_assets.rs` / UNIT_COUNTER_LAW.md).
4. **Air-wing airbase locator** — replaces the state-centroid FIT once the
   recording carries an airbase province (also a recording ask, above).

The province→world **anchor seam is stable**: adding slotting/stacking is a
selection on top of the per-province anchor, not a change to it.

## Files changed

- `replay_interface/native/map_viewer/src/unit_placement.rs` (new)
- `replay_interface/native/map_viewer/tests/unit_placement_gates.rs` (new)
- `replay_interface/native/map_viewer/src/lib.rs` (additive mod + pub use)
- `replay_interface/native/map_viewer/Cargo.toml` (dev-dependency replay_core)
- `replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md` (item 8 PLACEMENT GAP)
- `replay_interface/documents/EXTRACTION_NEEDS.md` (A7 placement note)
