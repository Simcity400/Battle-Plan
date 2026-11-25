# 1400. Truncated-hour coverage and the keyed-row witness rule

## Context

When the collector hits a row or budget cap on an hour, columnar storage records the table's status as `Status::Truncated { missing }` together with the exact sorted list of slots that were read at that hour (`TableReader::read_rows()`, `Lane::read_rows()`, `TableView::Truncated { rows, missing }`).

Previously, keyed-row dashboard query paths disagreed on truncated hours:

- `dashboard-query/src/series/board.rs` and `dashboard-follow/src/evaluate.rs` refused every row of a truncated table, even when a caller asked for specific country keys whose rows were witnessed.
- `dashboard-follow/src/live.rs` rejected all live cells on `TableView::Truncated`, even for witnessed keys.
- `dashboard-query/src/series.rs` served witnessed scalar rows through `key_runs`, but refused computed `NotApplicable` on truncated hours, fell back to generic measure-unrecorded text or `missing: None` on unwitnessed rows, marked all truncated spans missing on keyed tiles, and missed unwitnessed truncation breaks inside bins on keyed tiles.
- `playhead::Lane` and `dashboard-follow::Follow` only invalidated on `Status` changes, so consecutive truncated hours with the same `missing` count that swapped witnessed rows (`{A}` -> `{B}`) did not re-evaluate followed keyed queries.

Whole-population queries (ranks, top-N, totals, faction sums, owner part enumerations, and multi-row comparisons) cannot use row-level witness checks: witnessing a subset of keys does not prove the rest of the population is absent or bounded.

## Decision

### Default-Table scope inversion and structural split

Whole-population functions, request shapes, and UI paths cannot accept `CoverageScope::Row`: any function or request whose output depends on the population (order, rank, top-N, total, sum, share, count, enumeration, or race) takes no `CoverageScope` argument and always uses `origin/main`'s table-wide coverage and refusal wording.

Shared helpers that previously carried a scope flag are split into dedicated table-wide and keyed-row entry points (`Line::binned`/`Line::binned_for_tile`, `Line::missing`/`Line::missing_for_tile`, `Line::part_coverage_table`/`Line::part_coverage_row`, `board::values_table`/`board::values_row`), and `dashboard-follow` splits `Query::Series` into `SeriesMode::{Keyed(CoverageScope), Top { n, least }, Parts { top }}` so population-shaped `Row` requests are unrepresentable at compile time.

Only explicitly keyed-row call sites — whose input is an explicit key list and whose output is one independent cell per key in the caller's order — opt into `CoverageScope::Row`, deciding per-key coverage through one function in `crates/dashboard-query/src/coverage.rs`:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CoverageScope {
    #[default]
    Table,
    Row,
}

pub enum Coverage {
    Recorded,
    NotRecorded(String),
}

pub fn row_coverage(
    table: TableId,
    status: Status,
    unavailable_reason: Option<&str>,
    witnessed: bool,
) -> Coverage
```

#### Functions and types that still carry `CoverageScope`

1. `crates/dashboard-follow/src/query.rs::SeriesMode::Keyed(CoverageScope)` — explicitly keyed country list with no `top` and no `parts`; emits one independent cell per requested key in caller order. `SeriesMode::Top` and `SeriesMode::Parts` have no scope field.
2. `crates/dashboard-follow/src/parse.rs::parse` — parses optional outer `"coverage": "table" | "row"` on followed requests and validates the original request shape before expanding `"reads": "every"` or discarding `top`, rejecting `"row"` on `"reads": "every"`, any `top` (including `army-size`, `air-force-size`, and `navy-size` with both `countries` and `top`), and `parts`.
3. `crates/dashboard-follow/src/live.rs::Cells::add(scope: CoverageScope, ...)` (private) -> `Group.scope` — `Cells::sum` hardcodes `CoverageScope::Table` and takes no scope argument; only single-key `value`, `difference`, and `ratio` pass `CoverageScope::Row`.

No function in `dashboard-query` takes a `CoverageScope` argument.

### Witness rule (`CoverageScope::Row`)

1. `Status::Complete` -> `Coverage::Recorded`.
2. `Status::Truncated { .. }` with `witnessed == true` -> `Coverage::Recorded`. The witnessed row is read as on a complete hour, including zero values, unrecorded cells, and computed `NotApplicable`.
3. `Status::Truncated { missing }` with `witnessed == false` -> `Coverage::NotRecorded` with the canonical truncation reason below. An unwitnessed row is never carried forward, never treated as absent or zero (even on `absent_none` or `absent_is_zero` tables), and never silently dropped.
4. `Status::NotRecorded` and `Status::Unavailable { .. }` -> `Coverage::NotRecorded` with the canonical reasons below.

Note on complete-hour `absent_none`: on complete hours an absent row reads `0` on every surface (`Line::states_of`, `board::at`, `dashboard-follow`), as a rank counts it; on truncated hours, unwitnessed keys never return `0` on any surface.

### Canonical reason strings (`row_coverage`)

Let `{Name}` be `table.words()` with its first letter capitalized (for example, `Countries`, `Country factories`, `Country experience`, `Country resources`). Every `CoverageScope::Row` path uses these exact strings, free of internal terms:

- `Status::NotRecorded`:
  `"{Name} not recorded at this hour"`
- `Status::Truncated { missing }` (when `!witnessed`):
  `"{Name} read only in part at this hour ({missing} rows not read)"`
- `Status::Unavailable { .. }` with non-empty `reason`:
  `"{Name} not available at this hour: {reason}"`
- `Status::Unavailable { .. }` with absent or empty `reason`:
  `"{Name} not available at this hour"`

`CoverageScope::Table` paths keep `origin/main`'s exact table-wide refusal strings (`board::table_unrecorded`, `series.rs::status_reason`, and `dashboard-follow/src/evaluate.rs::unrecorded`).

### Complete call-site audit (`Table` vs `Row`)

#### `dashboard-query`

| Call site | Scope | Reason |
| --- | --- | --- |
| `series.rs::Line::states_of` | `Row` | Keyed per-(country, hour[, part]) reads; preserves `point.covered` and complete-hour spans via `replay_lanes::points::points_with_complete` (`initial_witness = true`, 1 storage read for unvalued or absent complete-hour scalars) and resolves unresolved truncated/computed keys via `at_each_with_status` + `row_coverage`. |
| `series.rs::Line::owned_at` | `Table` | Enumerates an owner's unknown set of parts at one hour (`status_reason`). |
| `series.rs::Line::binned`, `hourly_runs` (called by `summaries`, `summaries_within`, `summary`, `ranked`) | `Table` | Shared summary path using `replay_lanes::points::points` (`initial_witness = false`, matching `origin/main` even when ordinal 0 is truncated) for ranked top-N (`series/ranked.rs`, `army.rs`, `forces.rs`), faction sums (`factions.rs`), and resource summaries (`resources.rs`). |
| `series.rs::Line::missing` (`part_coverage_table`) | `Table` | Keeps `Line::missing`'s signature and `origin/main` table-wide coverage for all summary/ranked/faction callers. |
| `series.rs::Line::part_lives` | `Table` | Enumerates an owner's parts over a range (`part_coverage_table`). |
| `series/board.rs::values_row` (`board::at` with `Rows::Countries`) | `Row` | Explicitly requested country cells; looks up each key in sorted `snapshot.rows` and passes `slot.is_some()` to `row_coverage`. |
| `series/board.rs::values_table` (`board::at` with `Rows::Top`) and `Line::rank_at_by_checked` | `Table` | Whole-population top-N ranking and every secondary column on a top-N board (`board::table_unrecorded`). |
| `tile.rs::tile` (`Line::hourly_runs_for_tile`, `Line::binned_for_tile`, `Line::prefetch_unwitnessed`, `Line::missing_for_tile`) | `Row` | Keyed per-line tile requests; batches all uncached keys per truncated interval into one `witnessed_points` walk (`O(1)` per truncated interval), intersects `part_gaps` with `within` spans before setting `has_break`/`first_break`, reads full-tile status spans before `within` clipping (3 reads first complete tile, 2 subsequent), and uses `Line::alias_plan` so exact states and tiles agree on aliased keys. |

#### `dashboard-follow` and `dashboard-naval` / `at_hour` live cells

| Call site | Scope | Reason |
| --- | --- | --- |
| `evaluate.rs::write` (`Rows::Keys`, `Rows::Live`, `SeriesMode::Keyed(CoverageScope::Row)`) | `Row` | Explicitly keyed followed reads; checks `lane.read_rows()` (`O(log W)` binary search) via `Reasons::row_coverage -> row_coverage`. `playhead::Lane::finish` sets `Changes::read_rows_changed()` when consecutive truncated hours swap witnessed slots. |
| `evaluate.rs::write` (`Rows::All`, `Rows::LiveTable`, `SeriesMode::Top`, `SeriesMode::Parts`, `SeriesMode::Keyed(CoverageScope::Table)`) | `Table` | Whole-population, ranked, part-enumeration, and multi-row sum queries refuse the whole answer via `Reasons::unrecorded`. |
| `follow.rs::Index::witnessed` (`Follow::touch`) | `Row`-only invalidation | Only queries where `Query::witness_sensitive()` is true are queued on `Changes::read_rows_changed()`, so `Table`-only subscriptions perform zero extra evaluations when witnesses swap across consecutive truncated hours. |
| `follow.rs::Follow::evaluate` (`Rows::Live` -> `LiveRows::evaluate`) | `Row` | Keyed live-cell backing queries route per-key coverage through `Reasons::row_coverage -> row_coverage`, borrowing `NO_ROW` on complete hours. |
| `follow.rs::Follow::evaluate` (`Rows::LiveTable`) | `Table` | Multi-row live sums refuse via `Reasons::unrecorded`. |
| `live.rs::Cells::value`, `difference`, `ratio` -> `group_values` | `Row` | Single-key live cell reads (`minefields.rs` owner mines, `research/slots.rs` slot progress, per-row scalars) take a fast path on `Status::Complete` and binary-search `TableView::Truncated` per key via `row_coverage`. |
| `live.rs::Cells::sum` -> `group_values` | `Table` | Multi-row population sums (`minefields.rs` region `"total"`, `production/live.rs` line `"output"`, `faction_state/details.rs` faction `"manpower"`, `occupation/states.rs` state `"assigned"`, `world_tension.rs` and `territory.rs` `"unnamed"`) refuse via `evaluate::unrecorded`, keeping `"total"` registered with `main`'s truncation reason. |
| `parse.rs::parse` | `Table` default; `Row` opt-in | Defaults all followed requests to `CoverageScope::Table`; accepts outer `"coverage": "row"` only on explicitly keyed non-ranked, non-parts series requests and rejects it on `"reads": "every"`, any `top` (even when `countries` is also passed), and `parts`. |

#### UI followed queries (`gate.follow`)

| Call site | Scope | Reason |
| --- | --- | --- |
| `ui/src/charts/race/MeasureRace.tsx` (`!racing` head + `secondQuery`) | `Row` | Keyed country lines mode. |
| `ui/src/charts/race/MeasureRace.tsx` (`racing`, both `top` and selected-group) | `Table` | Whole-population race ranking. |
| `ui/src/charts/race/FactionTotalsView.tsx` | `Table` | Multi-faction sum and ranking. |
| `ui/src/charts/race/PartFollows.tsx`, `ui/src/dashboard/keyedSeries.ts` | `Table` | Owner part enumerations (`parts: true`). |
| `ui/src/charts/race/PartsRaceView.tsx`, `ui/src/widgets/army-size/ArmySizeWidget.tsx` | `Table` | Live race rankings (`air-force-size`, `navy-size`, `army-size`). |
| `ui/src/widgets/casualties/CasualtiesWidget.tsx` (`top == null && !race`) | `Row` | Explicitly selected countries in butterfly mode; preserves caller selection order when any selected row is incomplete (`orderCasualtyRows`), tracks per-measure reasons (`takenReason`) so `casualtyUnknownNote` never cites `dealt`, and retains unread keyed rows with truncation reasons in tooltips. |
| `ui/src/widgets/casualties/CasualtiesWidget.tsx` (`top != null || race`), `diplomatic-relations/topCountries.ts` | `Table` | Top-N ranked membership/values and all casualty race modes (`casualtyRaceEntries` returns `[]` whenever any row has `knownTaken === null`). |
| `ui/src/widgets/scoreboard/scoreboardProjection.ts` (`top == null`) | `Row` | Explicitly selected scoreboard countries (`coverage: 'row'` on columns and partials, matching exact `board::at(Rows::Countries)`). |
| `ui/src/widgets/scoreboard/scoreboardProjection.ts` (`top != null`) | `Table` | Top-N ranked scoreboard (omits `coverage`, matching exact `board::at(Rows::Top)`). |
| `ui/src/widgets/autonomy/AutonomyWidget.tsx`, `country-measure/MeasureWidget.tsx`, `experience/ExperienceWidget.tsx`, `fuel/FuelWidget.tsx`, `stability/StabilityWidget.tsx`, `supply-health/supplyHead.ts`, `supply-health/supplySeries.ts` | `Row` | Per-country keyed line heads. |
| `ui/src/widgets/army-size/ArmyBreakdown.tsx`, `player-apm/ApmWidget.tsx`, `special-projects/useProjectHistory.ts` | `Table` | Non-series panel queries. |

### A lane history at an hour read in part

`replay_lanes::lanes::history` never carries a key's value across an hour whose read did not reach it: the key's step ends there. The history also holds the table's status over the span it read (`History::statuses`, collected by the same walk, so walked blocks cost no extra decode) and the unavailable reasons. `History::reading(key, hour)` is the one lookup: the key's cells where the read reached it (a complete read, or a partial read that witnessed it), absent only where a complete read did not hold it, and unread, with the hour's status, everywhere else (a partial read that did not reach it, an hour not recorded or unavailable, an hour outside the history). `History::readings(key)` gives the same as runs. A product never takes a key's coverage from the table's complete hours alone, never shows the previous hour's value for an unread key, and never drops a partly read hour whole. `dashboard_query::coverage::reading_witness` words an unread reading with the canonical strings above.

### A rank at an hour

`Line::rank_at(hour, n, least) -> Result<Vec<Country>, reason>` is the one rank of a population at an hour, exact and followed (`dashboard-follow` `SeriesMode::Top { population: true }`) alike. It refuses with a reason, never an empty rank, when the table is not complete or when a row's value is unrecorded (a not-applicable value is not a contender): only a value that was not read can change the order. A recorded row whose holder is not resolved at the hour keeps its place by its value: in a slot-keyed series it is ranked as its slot (`Country::Slot`), `series-rank` marks it with `unnamed` (why its country is not named) and gives its recorded readings, the followed top names it by its slot key, and a ranked tile refuses by name, since a line is drawn from its holders. A row of a country-keyed series whose id no holder resolves names no slot either, so that rank is refused. A named top (`population: false`) ranks only the named rows, and an unrecorded named row refuses it.

`series-rank` (`series/ranked.rs`) is the one reply for a ranked view at an hour: `{ hour, top | countries, least?, measures? }` answers `rows` in the product's order, each with a reading per measure (the ranking measure first, then every measure the view displays), `refused`, and `from..until`. A top that is refused has no rows; chosen countries all stay, ranked first and then the ones whose ranking reading is missing, in the asked order, with `refused` naming why the order is unproven. `from..until` is where the reply holds unchanged: for chosen countries the intersection of every reading's run; for a top that, narrowed to hours whose own rank is the same order (at most `RANK_PROOF_HOURS` each side), so a country left out cannot overtake unseen; otherwise the asked hour only.

A ranked view of a group sends the group (`group: { type, id | tag, membership }`), never its member list. Rust resolves its members at the asked hour (`dashboard_world::products::group_members::members_at`). Membership that is unknown then refuses the reply and is never flattened to a union, and `from..until` ends where the membership changes.

### Out of scope (deferred to follow-up)

Whole-population results remain on `CoverageScope::Table` (`origin/main` behaviour) in this ADR and are left to a dedicated follow-up: totals, ranked summaries and their `membership` (`Line::ranked`), `totals_ranked`, faction lines/totals (`dashboard-wars/src/products/factions.rs`, `FactionTotalsView`), owner part enumerations (`part_lives`, `owned_at`), race-mode `MeasureRace` and `CasualtiesWidget`, multi-row `Cells::sum` totals, and any whole-population refusal fields in the generated TypeScript contract (`ui/src/generated/dashboard.ts`).

## Consequences

- Complete hours perform zero extra witness comparisons, zero extra tile storage reads (both unfiltered and `within`-filtered), zero extra scalar `at_each_with_status` re-reads (both unvalued and absent holders), and zero live-cell `Status::Complete` string allocations.
- Truncated tiles batch unwitnessed-key discovery once per `(run, truncated_interval)` across all requested countries (`O(1)` witness walks per truncated interval), and `Table`-only followed subscriptions perform zero extra evaluations when witnesses swap across consecutive truncated hours.
- Population-shaped `Row` queries are unrepresentable in `dashboard-query` and `dashboard-follow` types and rejected at `parse` before request expansion.
- No on-disk recording format or TypeScript contract changes are introduced.
- Every `CoverageScope::Row` path agrees on values, `NotApplicable`, absence, and canonical missing reasons across complete, truncated, unavailable, and not-recorded hours.
