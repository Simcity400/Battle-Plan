# Lane report — A3-completion + A4 round-trip certification

Branch `lane/a3-roundtrip` @ session HEAD `545add5e7`. Additive-only in
`replay_core/src`; no public item removed; `replay_app` still builds.

## What landed

- **`replay_core/tests/roundtrip_gates.rs`** (new, 17 tests): opens every
  positive fixture through the real `ReplaySource` and, at **every recorded
  hour**, asserts the typed view value == the generator's ground truth (the
  value the packed snapshot carries at that raw JSON path — the fixture *is*
  the oracle). Plus A4 seek==play, A6 absence honesty, snapshot determinism,
  and an adversarial gate-can-fail proof.
- **Additive view/ingest wiring in `world.rs`** to close real gaps the fixtures
  exposed (all fail-closed; no fabricated defaults). See table.
- **`lib.rs`**: exported `SnowMudView`, `SnowMudCell`.

## Test counts (exact)

`cargo test -p replay_core` → **27 passed, 0 failed**:
- `roundtrip_gates.rs`: **17 passed** (new)
- `fixture_conformance.rs`: 3 passed (untouched)
- `world_timeline.rs`: 7 passed (untouched)

`cargo build -p replay_app` succeeds (only pre-existing `dead_code` warnings in
replay_app color math, unrelated). `cargo clippy -p replay_core --tests` clean.
`cargo fmt -p replay_core -- --check` clean.

**Load-bearing proof**: temporarily repointing the weather view to a wrong key
made `rt_weather_sweep` FAIL; restoring made it pass. The adversarial test
(`adversarial_wrong_oracle_makes_gate_fail`) additionally `catch_unwind`s a
perturbed oracle and asserts the panic, and checks Measured≠Unobtained.

## Coverage table — round-trip-certified vs gap-documented

| Req item | Contract / fixture | View surface | Status |
|---|---|---|---|
| 1,2 | owner/controller flips (`fx_owner_controller_flip`) | `owner_by_state`, `control_by_state`, `control_by_province` | **CERTIFIED** every state/province, every hour |
| 7,7b | cosmetic + ideology (`fx_cosmetic_ideology`) | `country_identity().cosmetic_tag / ruling_ideology` | **CERTIFIED** (+`misc.cosmetic_tag` alias added) |
| 8b | war/faction matrix (`fx_war_faction_matrix`) | `diplomacy().countries[*].faction_name`, `diplomatic_pair_state().at_war / same_faction` | **CERTIFIED** (war edges, faction, same-faction) |
| 8b | capital moves (`fx_capital_moves`) | `diplomacy().countries[*].capital_state` | **CERTIFIED** every hour |
| 6 | weather sweep (`fx_weather_sweep`) | `weather_by_region().weather_kind / intensity` | **CERTIFIED** (added `phenom_name` alias to weather_kind) |
| 6 | snow/mud accumulator (`fx_snow_mud_accumulator`) | **new** `snow_mud()` → `by_province` snow/mud/region + `fade_*` | **CERTIFIED** (new `SnowMudView` reading `weather.snow_mud.texture`+`fade`) |
| 4,9b | topbar scalars (`fx_topbar_scalars_sweep`) | `topbar_scalars()` pp/stability/war_support/command_power/manpower/xp×3/factories×3/fuel×2/nukes×2/convoys/supply×2 | **CERTIFIED** (added `convoys_count` alias + `scalars.supply_*` fallback) |
| 3 | division movement (`fx_division_movement`) | `division_units()` loc/moving/target/progress/in_combat/strength; `combats()` | **CERTIFIED** every hour |
| 5,8 | naval/air (`fx_naval_air_activity`) | `naval_supremacy_by_region()` controller/supremacy; `fleets()`; `wings()` | **CERTIFIED** (region control, task force, air wing) |
| 10 | building damage (`fx_building_damage`) | `buildings_by_state()` levels + damage overlay (healthy_level/damaged/damage_fraction/repair/under_construction), state & province scope | **CERTIFIED** (added `building_damage` array fold) |
| 13 | POV visibility (`fx_pov_visibility`) | `visibility_for(pov)` provinces + `spotted_units` div/fleet/wing | **CERTIFIED** (added `povs` + `visible_foreign_units` aliases) |
| 4,9 | DLC/language (`fx_dlc_language_variation`) | `session_metadata().active_dlcs / localisation_language`; `country_identity("CAN")` | **CERTIFIED** every hour |

### Documented gaps (honest, left fail-closed — never a fabricated default)

These are fields the fixtures carry under **non-canonical / no-typed-view**
shapes; the view returns `Leaf::Absent`/`Unobtained` (asserted so), never a
guess. None block A5 (which keys on owner+controller+cosmetic — all certified).

- **topbar `world_tension`**: fixture parks it at
  `world.world_tension_inputs.fixture_topbar_world_tension` (a fixture-only key);
  the canonical topbar reader (`world.world_tension` / `world.tension` /
  `world_events.world_tension`) leaves it `Absent`. Gate asserts `Absent`.
- **topbar `scalars.alerts.*`** (initiative/decisions/intel/market_contracts):
  no typed field; raw-probe only in `fixture_conformance.rs`.
- **`naval_air.air_superiority`** (air controller / mission_mask / wing/plane
  counts): no typed view; `naval_supremacy_by_region` serves the region-control
  controller/supremacy instead. Air-superiority surface is a separate view not
  yet modeled.
- **`diplomacy_live.war_relations`** (live casualties) and
  **`countries.*.diplomacy.relation_flags`** (per-tag string arrays): no typed
  view; the canonical `wars[]` / relation lists already drive the boolean
  matrix that is certified.
- **`session_metadata.player_countries`**: fixtures nest players under
  `world.scenario.players`; the session view reads `session.player_countries`
  (a top-level `session` domain the fixtures don't emit) → `Absent`. Not a
  probed contract.

## A4 — seek == play (`seek_equals_play_on_every_fixture`)

For all 12 fixtures: forward pass over every exact hour, backward pass,
in-between scrub (arbitrary hour between anchors resolves to the nearest
snapshot at-or-before, identically for seek/play, and equals the anchor
frame), and a mid-file cold-start (fresh timeline first-accessed at a middle
hour matches a warm one). Equality is total: raw domain map + hour/date/
save_version + a spread of typed views (owner/control/units/fleets/wings/
combats/countries/identities/diplomacy/topbar/weather/snow_mud/buildings/
naval/session/visibility×3 POVs).

## A6 — absence honesty (`absence_honesty_absent_vs_unavailable_vs_present`)

Three-way distinction gated as distinct:
- **DomainAbsent** — `visibility` / `snow_mud` never captured in
  `fx_owner_controller_flip` (domain key absent) → `DomainAbsent`, incl.
  `visibility_for(pov).status`.
- **DomainUnavailable** — `weather` / `naval_air` present but null contents in
  the base snapshot → `DomainUnavailable` (captured-but-unobtained), distinct
  from absent.
- **Present** — same domains in their exercising fixtures → `Present`.
- Bonus: an unlisted POV (`USA`) inside a present visibility domain →
  `DomainAbsent` (distinct from the domain being absent).

## Delta == snapshot cross-check

**N/A — fixtures are snapshot-flavor.** Each carries per-hour `role:"snapshot"`
entries; the `collector_raw` stream is an audit placeholder, not a field-level
delta encoding, and `replay_core` has no delta-reconstruction path (Backend 2
is fail-closed in `source.rs`). Instead `snapshot_path_is_deterministic` gates
the snapshot path: re-opening a fixture yields byte-identical world state at
every hour (full `assert_state_equal`).

## New view/ingest code (all additive)

`world.rs`:
- `SnowMudView` + `SnowMudCell` structs, `build_snow_mud`, `WorldFrame.snow_mud`
  field, `snow_mud()` / `snow_mud_status()` accessors.
- `fold_building_damage` helper; `build_buildings` now overlays a per-state
  `building_damage[]` array (state & province scope) onto the level maps.
- `weather_kind` accepts `phenom_name` alias.
- `leaf_convoys` accepts `military.convoys_count` scalar.
- topbar `supply_value` / `supply_ratio` fall back to the `scalars` bag.
- `build_visibility` accepts `povs` (alias of `by_pov`); `parse_pov_visibility`
  accepts `visible_foreign_units` (alias of `spotted_units`).
- `build_country_identities` cosmetic_tag accepts `misc.cosmetic_tag` alias.

`lib.rs`: `SnowMudView`, `SnowMudCell` exported.

`REPLAY_DATA_REQUIREMENTS.md` **not modified**: no genuinely new runtime-state
field was discovered — items 6/10/13 already cover these surfaces. The wiring
above broadened the reader to accept the recorder's own key shapes (aliases),
it did not add a hard new required field. Recommend the extractor emit the
canonical keys documented in `REPLAY_SCHEMA.md`; the alias tolerance is a
safety net, not a contract change.

## A5 readiness

**A5 (renderer keying swap) is UNBLOCKED.** Its inputs — per-state owner,
per-state/province controller, and per-country cosmetic tag — are round-trip
certified at every hour on their flip fixtures, with the absent/unavailable/
present trichotomy gated. No open gap touches owner/controller/cosmetic.

## Do NOT git commit — left staged in the working tree for orchestrator review.
