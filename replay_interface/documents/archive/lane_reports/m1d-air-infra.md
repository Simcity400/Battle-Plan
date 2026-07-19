# LANE REPORT — M1d: Air + Infrastructure map modes

Lane `lane/m1d-air-infra`. Derives map-mode fill laws for **Air** (row 3) and
**Infrastructure** (row 10) of `MAPMODE_COVERAGE_MATRIX.md` (plan E3, wave M2).
Both recording-fed. `map_viewer` only; `replay_core` READ-ONLY;
`naval_supremacy_tint` reused, not edited.

## Deliverables

- `replay_interface/native/map_viewer/src/mapmode_air.rs` — strategic-air tint.
- `replay_interface/native/map_viewer/src/mapmode_infrastructure.rs` — infra gradient.
- `lib.rs`: two `mod` lines + two `pub use` blocks (single-line-scope shared edit).
- Docs: `MAPMODE_COVERAGE_MATRIX.md` rows 3/10 → **law-known**; `FIDELITY_LEDGER.md`
  new §2.20 (air) + §2.21 (infra); `REPLAY_PRODUCTION_PLAN.md` §6a M2 row.

## Air (`mapmode_air.rs`)

- **Geometry (FILE):** an air region = a strategic region with its FULL province
  set (land + sea), unlike naval which keeps sea provinces only. Reuses the same
  `crate::load_strategic_regions` loader the naval tint rides (no edit to that
  module; a thin re-parse keeping all provinces). Vanilla = **304 air regions**
  (region 301, land-only, is present — excluded from naval's 98).
- **Colours (FILE):** `STRATEGIC_AIR_COLOR_{BAD,GOOD,AVERAGE,NEUTRAL}`
  (`00_graphics.lua:906-909`), all alpha 1.0. NEUTRAL ships `140.0/255` division
  tokens; the shared `parse_lua_number_list` can't evaluate those, so the module
  carries its own boundary-checked token evaluator.
- **Data:** mirrors `replay_core::AirSuperiorityRow`
  (`naval_air.air_superiority.<region>`: controller / mission_mask / wing_count /
  plane_count).
- **Tint law:** no controller / no planes → NEUTRAL (FILE, fail-closed). Active
  region → **controller's country colour** at the file overlay alpha. This
  omniscient-observer fill is a **DERIVED** product choice (parallel to C6
  naval), NOT a claim that vanilla's air mode colours by controller.
- **UNKNOWN (capture/RE ask):** vanilla paints GOOD/AVERAGE/BAD *relative to the
  viewed country's* air-superiority ratio; the ratio cutoffs are engine runtime,
  not in defines (only `AIR_REGION_SUPERIORITY_PIXEL_SCALE=0.04` is file-visible).
  `pov_air_status_color`/`PovAirRelation` map an already-decided relation to its
  FILE colour but the decision is E6-gated + threshold-UNKNOWN. Overlay-alpha
  curve also unconfirmed (no `AIR_REGION_TRANSPARENCY_*` define). → needs a
  strategic-air capture with known per-side plane counts spanning
  contested→dominated.

## Infrastructure (`mapmode_infrastructure.rs`)

- **Gradient (FILE):** `MAP_MODE_INFRA_RANGE_COLOR_FROM = {1,0.125,0,0.1}` /
  `_TO = {0.1,0.9,0.1,0.6}` (`00_graphics.lua:23-24`), RGBA, endpoints exact.
- **Ceiling (FILE):** infra has NO `MAP_MODE_INFRA_RANGE_MAX` define; the
  file-derivable denominator is the infrastructure building's
  `level_cap.state_max = 5` (`common/buildings/00_buildings.txt`). Parsed with an
  identifier-boundary scan that rejects `energy_`/`industrial_infrastructure`, the
  `infrastructure = yes` flag, and `infrastructure_construction_effect`.
- **Data:** `replay_core` buildings view
  (`buildings_by_state()[s].state_buildings["infrastructure"].level`, `Leaf<f64>`).
  Verified present (bare number → `level_only`). A state with no `infrastructure`
  entry (or no buildings block) → no tint (fail-closed).
- **Tint law:** `rgba = lerp(FROM, TO, clamp(level/state_max, 0, 1))`.
- **UNKNOWN (capture ask):** endpoints + ceiling are FILE, but the assumption the
  engine interpolates *linearly over exactly [0, state_max]* (vs a dynamic
  per-map max, or a non-linear curve) is DERIVED → needs an infra-mode frame with
  states spanning infra 0..5 to confirm shape + denominator.

## Fixture gates (expectations from generator SOURCE, not code-under-test)

- **Air → `fx_naval_air_activity`:** `air_superiority["301"]` controller flips
  ENG (tick<7) → GER (tick≥7), wing_count 2+tick, plane_count 120+tick·10. Test
  asserts Contested status + controller-colour fill each tick, NEUTRAL when the
  region has no input.
- **Infra → `fx_building_damage`:** `states.10.buildings.infrastructure = 5`
  (constant); states 20/30 have no buildings block. Test asserts state 10 → t=1.0
  → TO endpoint; states 20/30 → fail-closed (no tint).

## Test result

`cargo build -p map_viewer` green. `cargo test -p map_viewer mapmode` = **13/13**
new tests pass (7 air + 6 infra); the two `gate_vanilla_*` tests exercise the real
shipped files when `BATTLE_PLAN_GAME_DIR` is set (304 air regions; infra state_max 5;
both colour sets byte-match the file).

## Conflict-rule compliance (Wave-5)

Only own new module + test files touched, plus: `lib.rs` single `mod`/`pub use`
lines; `FIDELITY_LEDGER.md` new §2 rows (no §1 totals); `MAPMODE_COVERAGE_MATRIX.md`
own rows' status; plan §6a own row. `replay_core` and `naval_supremacy_tint`
untouched.
