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

### Default-Table scope inversion

Every shared function in `dashboard-query` and `dashboard-follow` defaults to `CoverageScope::Table` (`origin/main`'s table-wide coverage and refusal wording). Only explicitly keyed-row call sites opt into `CoverageScope::Row`, which decides per-key coverage through one function in `crates/dashboard-query/src/coverage.rs`:

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

### Witness rule (`CoverageScope::Row`)

1. `Status::Complete` -> `Coverage::Recorded`.
2. `Status::Truncated { .. }` with `witnessed == true` -> `Coverage::Recorded`. The witnessed row is read as on a complete hour, including zero values, unrecorded cells, and computed `NotApplicable`.
3. `Status::Truncated { missing }` with `witnessed == false` -> `Coverage::NotRecorded` with the canonical truncation reason below. An unwitnessed row is never carried forward, never treated as absent or zero (even on `absent_none` or `absent_is_zero` tables), and never silently dropped.
4. `Status::NotRecorded` and `Status::Unavailable { .. }` -> `Coverage::NotRecorded` with the canonical reasons below.

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
| `series.rs::Line::states_of` | `Row` | Keyed per-(country, hour[, part]) reads; preserves `point.covered` and complete-hour spans via `replay_lanes::points::points_with_complete` (1 storage read for unvalued or absent complete-hour scalars) and resolves unresolved truncated/computed keys via `at_each_with_status` + `row_coverage`. |
| `series.rs::Line::owned_at` | `Table` | Enumerates an owner's unknown set of parts at one hour (`status_reason`). |
| `series.rs::Line::binned` (called by `summaries`, `summaries_within`, `summary`, `ranked`) | `Table` | Shared summary path used by ranked top-N (`series/ranked.rs`, `army.rs`, `forces.rs`), faction sums (`factions.rs`), and resource summaries (`resources.rs`). |
| `series.rs::Line::missing` (called by `summaries`, `summaries_within`, `summary`, `ranked`) | `Table` | Keeps `Line::missing`'s signature and `origin/main` table-wide coverage for all summary/ranked/faction callers. |
| `series.rs::Line::part_lives` | `Table` | Enumerates an owner's parts over a range (`part_coverage(..., Table)`). |
| `series/board.rs::values` (`board::at` with `Rows::Countries`) | `Row` | Explicitly requested country cells; looks up each key in sorted `snapshot.rows` and passes `slot.is_some()` to `row_coverage`. |
| `series/board.rs::at` with `Rows::Top` and `Line::rank_at_by_checked` | `Table` | Whole-population top-N ranking (`board::table_unrecorded`). |
| `tile.rs::tile` (`Line::binned_scoped`, `Line::missing_scoped`) | `Row` | Keyed per-line tile requests; reads full-tile status spans before `within` clipping so unfiltered and filtered complete tiles share the status cache (3 reads first tile, 2 subsequent), and marks `has_break`/`first_break` in `binned_from_runs` for unwitnessed gaps including trailing truncation gaps. Uses `Line::alias_plan` so exact states and tiles agree on aliased keys. |

#### `dashboard-follow` and `dashboard-naval` / `at_hour` live cells

| Call site | Scope | Reason |
| --- | --- | --- |
| `evaluate.rs::write` (`Rows::Keys`, `Rows::Live`, `Query::Series { top: None, parts: false, scope: Row }`) | `Row` | Explicitly keyed followed reads; checks `lane.read_rows()` (`O(log W)` binary search) via `Reasons::row_coverage -> row_coverage`. `playhead::Lane::finish` sets `Changes::read_rows_changed()` when consecutive truncated hours swap witnessed slots. |
| `evaluate.rs::write` (`Rows::All`, `Rows::Top`, `Rows::LiveTable`, `parts: true`, `Query::Series { scope: Table }`) | `Table` | Whole-population, ranked, part-enumeration, and multi-row sum queries refuse the whole answer via `Reasons::unrecorded`. |
| `follow.rs::Follow::evaluate` (`Rows::Live` -> `LiveRows::evaluate`) | `Row` | Keyed live-cell backing queries route per-key coverage through `Reasons::row_coverage -> row_coverage`, borrowing `NO_ROW` on complete hours. |
| `follow.rs::Follow::evaluate` (`Rows::LiveTable`) | `Table` | Multi-row live sums refuse via `Reasons::unrecorded`. |
| `live.rs::Cells::value`, `difference`, `ratio` -> `group_values` | `Row` | Single-key live cell reads (`minefields.rs` owner mines, `research/slots.rs` slot progress, per-row scalars) binary-search `TableView::Truncated` per key via `row_coverage` (allocating `CellValue::Missing` only for missing keys). |
| `live.rs::Cells::sum` -> `group_values` | `Table` | Multi-row population sums (`minefields.rs` region `"total"`, `production/live.rs` line `"output"`, `faction_state/details.rs` faction `"manpower"`, `occupation/states.rs` state `"assigned"`, `world_tension.rs` and `territory.rs` `"unnamed"`) refuse via `evaluate::unrecorded`, keeping `"total"` registered with `main`'s truncation reason. |
| `parse.rs::parse` | `Table` default; `Row` opt-in | Defaults all followed requests to `CoverageScope::Table`; accepts outer `"coverage": "row"` only on non-ranked, non-parts keyed series requests and rejects it on `top`, `parts`, and `log-page`. |

#### UI followed queries (`gate.follow`)

| Call site | Scope | Reason |
| --- | --- | --- |
| `ui/src/charts/race/MeasureRace.tsx` (`!racing` head + `secondQuery`) | `Row` | Keyed country lines mode. |
| `ui/src/charts/race/MeasureRace.tsx` (`racing`, both `top` and selected-group) | `Table` | Whole-population race ranking. |
| `ui/src/charts/race/FactionTotalsView.tsx` | `Table` | Multi-faction sum and ranking. |
| `ui/src/charts/race/PartFollows.tsx`, `ui/src/dashboard/keyedSeries.ts` | `Table` | Owner part enumerations (`parts: true`). |
| `ui/src/charts/race/PartsRaceView.tsx`, `ui/src/widgets/army-size/ArmySizeWidget.tsx` | `Table` | Live race rankings (`air-force-size`, `navy-size`, `army-size`). |
| `ui/src/widgets/casualties/CasualtiesWidget.tsx` (`top == null`) | `Row` | Explicitly selected countries; retains unread keyed rows with truncation reasons in tooltips and surfaces unread countries in `casualtyUnknownNote`. |
| `ui/src/widgets/casualties/CasualtiesWidget.tsx` (`top != null`), `diplomatic-relations/topCountries.ts` | `Table` | Top-N ranked membership and values. |
| `ui/src/widgets/autonomy/AutonomyWidget.tsx`, `country-measure/MeasureWidget.tsx`, `experience/ExperienceWidget.tsx`, `fuel/FuelWidget.tsx`, `stability/StabilityWidget.tsx`, `supply-health/supplyHead.ts`, `supply-health/supplySeries.ts` | `Row` | Per-country keyed line heads. |
| `ui/src/widgets/army-size/ArmyBreakdown.tsx`, `player-apm/ApmWidget.tsx`, `special-projects/useProjectHistory.ts` | `Table` | Non-series panel / log-page queries. |

### Out of scope (deferred to follow-up)

Whole-population results remain on `CoverageScope::Table` (`origin/main` behaviour) in this ADR and are left to a dedicated follow-up: ranks, totals, top-N, ranked summaries and their `membership` (`Line::ranked`, `rank_at_by`, `series/ranked.rs`), `totals_ranked`, faction lines/totals (`dashboard-wars/src/products/factions.rs`, `FactionTotalsView`), owner part enumerations (`part_lives`, `owned_at`), race-mode `MeasureRace`, multi-row `Cells::sum` totals, and any whole-population refusal fields in the generated TypeScript contract (`ui/src/generated/dashboard.ts`).

## Consequences

- Complete hours perform zero extra witness comparisons, zero extra tile storage reads (both unfiltered and `within`-filtered), and zero extra scalar `at_each_with_status` re-reads (both unvalued and absent holders).
- Whole-population callers cannot accidentally inherit keyed-row coverage through shared functions.
- No on-disk recording format or TypeScript contract changes are introduced.
- Every `CoverageScope::Row` path agrees on values, `NotApplicable`, absence, and canonical missing reasons across complete, truncated, unavailable, and not-recorded hours.
