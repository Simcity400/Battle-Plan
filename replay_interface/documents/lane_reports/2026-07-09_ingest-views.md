# Lane Report - A3 replay_core design + ingest skeleton

## Scope

Changed content is limited to:
- `LANE_REPORT.md`
- `replay_interface/native/replay_core/src/lib.rs`
- `replay_interface/native/replay_core/src/world.rs`
- `replay_interface/native/replay_core/tests/world_timeline.rs`

`git status` cannot refresh the linked-worktree index from this sandbox
(`.git/worktrees/ingest-views/index.lock: Permission denied`), but
`git diff --name-only` reports only the three tracked `replay_core` files above;
`LANE_REPORT.md` is new/untracked until the orchestrator stages it.
`cargo fmt` briefly touched `game_ui`/`map_viewer`; those content changes were
reverted and `git diff --exit-code -- <those files>` reported no content diff.

## API Surface Added

All new public structs are re-exported from `replay_core::lib`:

- `ViewStatus`, `TimelineLookup`
- units: `DivisionUnitRow`, `FleetUnitRow`, `ShipCompositionRow`,
  `WingUnitRow`, `EquipmentAmount`
- session: `SessionMetadataView`, `PlayerCountry`, `SpeedEvent`
- map/runtime domains: `NavalSupremacyRow`, `WeatherRegionRow`,
  `BuildingStatus`, `StateBuildings`
- country/diplomacy/topbar: `CountryIdentity`, `CountryDiplomacy`,
  `DiplomaticPairState`, `DiplomacyView`, `TopbarScalars`
- POV visibility: `ProvinceVisibility`, `PovVisibilityView`,
  `SpottedUnitsView`

New `WorldTimeline` methods:

```rust
pub fn lookup_hour(&self, hour_index: i64) -> Result<TimelineLookup, WorldError>;
pub fn seek(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError>;
pub fn play_to(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError>;
```

New `WorldFrame` accessors:

```rust
pub fn owner_by_state_status(&self) -> ViewStatus;
pub fn control_by_state_status(&self) -> ViewStatus;
pub fn control_by_province_status(&self) -> ViewStatus;
pub fn countries_status(&self) -> ViewStatus;
pub fn division_units(&self) -> &[DivisionUnitRow];
pub fn division_units_status(&self) -> ViewStatus;
pub fn fleets(&self) -> &[FleetUnitRow];
pub fn fleets_status(&self) -> ViewStatus;
pub fn wings(&self) -> &[WingUnitRow];
pub fn wings_status(&self) -> ViewStatus;
pub fn session_metadata(&self) -> &SessionMetadataView;
pub fn naval_supremacy_by_region(&self) -> &BTreeMap<u32, NavalSupremacyRow>;
pub fn naval_supremacy_status(&self) -> ViewStatus;
pub fn weather_by_region(&self) -> &BTreeMap<u32, WeatherRegionRow>;
pub fn weather_status(&self) -> ViewStatus;
pub fn country_identities(&self) -> &BTreeMap<String, CountryIdentity>;
pub fn country_identity(&self, tag: &str) -> Option<&CountryIdentity>;
pub fn country_identities_status(&self) -> ViewStatus;
pub fn diplomacy(&self) -> &DiplomacyView;
pub fn diplomatic_pair_state(&self, owner: &str, viewed: &str) -> Option<&DiplomaticPairState>;
pub fn topbar_scalars_by_country(&self) -> &BTreeMap<String, TopbarScalars>;
pub fn topbar_scalars(&self, tag: &str) -> Option<&TopbarScalars>;
pub fn topbar_status(&self) -> ViewStatus;
pub fn buildings_by_state(&self) -> &BTreeMap<u32, StateBuildings>;
pub fn buildings_status(&self) -> ViewStatus;
pub fn visibility_for(&self, pov_tag: &str) -> PovVisibilityView;
pub fn spotted_units_for(&self, pov_tag: &str) -> SpottedUnitsView;
pub fn visibility_status(&self) -> ViewStatus;
pub fn visibility_povs(&self) -> impl Iterator<Item = &str>;
```

Existing public accessors and structs remain available.

## Design Decisions

- Added `ViewStatus::{DomainAbsent, DomainUnavailable, Present}` for
  root-level A6 honesty. `Leaf<T>` still carries value-level
  `Absent` / `Unobtained` / `Measured`.
- No silent numeric or boolean defaults were added. Missing false/zero stays
  `Leaf::Absent`; explicit null or wrong type becomes `Leaf::Unobtained`.
  Measured empty arrays can produce measured empty views/counts.
- `lookup_hour` is the A4 index seam. Today `seek(h)` and `play_to(h)` both
  resolve to the same at-or-before snapshot; future delta replay can hang off
  `TimelineLookup` while keeping `seek(h) == play_to(h)` testable.
- Schema-backed reads use existing shapes where present:
  `countries`, `states`, `divisions`, `order_of_battle`, `naval_air`.
- TODO seams for schema-not-yet-pinned payloads are internal and fail closed:
  `session` speed/player metadata, top-level `naval_supremacy`, top-level
  `weather`, top-level `diplomacy_pairs`, top-level `visibility`, and
  `world.world_tension`.
- No new runtime-state fields were discovered beyond existing
  `REPLAY_DATA_REQUIREMENTS.md`; no requirements doc row was added.

## Tests

Command:

```powershell
cd replay_interface/native
cargo test -p replay_core
```

Result:

- `src/lib.rs`: 0 passed / 0 failed
- `src/bin/make_demo_recording.rs`: 3 passed / 0 failed
- `src/bin/replay_dump.rs`: 0 passed / 0 failed
- `tests/world_timeline.rs`: 7 passed / 0 failed
- doc-tests: 0 passed / 0 failed
- total: 10 passed / 0 failed / 0 ignored

Coverage added:

- measured synthetic values for every new typed view
- explicit null/unavailable leaves
- domain-absent vs domain-present-unavailable status for every new view
- `seek(102) == play_to(102)` over the existing snapshot gap
- directory store vs packed `.battleplan` equality for the new typed views

## Downstream Build

Command:

```powershell
cd replay_interface/native
cargo build -p replay_app
```

Result: failed outside this lane.

Exact blocker:

```text
error[E0004]: non-exhaustive patterns: `AtlasMode::PreservedColor` not covered
   --> replay_app\src\font.rs:739:11
```

`replay_app` is disallowed for this lane, and `git diff --name-only` shows no
`replay_app` content changes from this work.

## Open Gaps

- Packed fixture round-trip conformance remains deferred to the fixture lane.
- The TODO seams above should be promoted into `REPLAY_SCHEMA.md` once the
  fixture generator/container contract pins exact payload shapes.
- `cargo build -p replay_app` remains blocked by the pre-existing
  `AtlasMode::PreservedColor` match arm gap in `replay_app/src/font.rs`.
