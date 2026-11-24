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

### Every `CoverageScope::Row` call site

1. **Exact keyed states (`crates/dashboard-query/src/series.rs::Line::states_of`)**: preserves `point.covered` and complete-hour spans from the single `key_runs_covered` pass (`replay_lanes::points::points_with_complete`), so both unvalued and absent scalar holders on complete hours resolve in one storage read with zero `at_each_with_status` re-read. Truncated and computed unresolved keys resolve through `at_each_with_status` + `row_coverage` (preserving `Computed::NotApplicable` on witnessed truncated rows and emitting the canonical truncation reason on unwitnessed rows). Unspecified-part owner enumerations (`owned_at`) remain `CoverageScope::Table`.
2. **Scoreboard keyed cells (`crates/dashboard-query/src/series/board.rs::values`)**: `board::at` with `Rows::Countries` looks up each requested country key in sorted `snapshot.rows` and passes `slot.is_some()` to `row_coverage`, serving witnessed cells and refusing unwitnessed rows (including `absent_none` series) with the canonical reason. `Rows::Top` remains `CoverageScope::Table`.
3. **Keyed series tiles (`crates/dashboard-query/src/tile.rs::tile`)**: explicitly passes `CoverageScope::Row` to `Line::binned_scoped` and `Line::missing_scoped`.
   - `Line::missing`, `Line::binned`, `Line::summaries`, `Line::summaries_within`, `Line::summary`, `Line::ranked`, and `Line::part_lives` keep their public signatures and default to `CoverageScope::Table`.
   - `Line::binned_scoped(..., CoverageScope::Row)` reads status spans over the full tile span before clipping to `within`, so both unfiltered and filtered complete tiles share the cached status read with `missing_scoped` and perform the same storage reads as `main` (3 on first tile, 2 on subsequent tiles).
   - `Line::binned_from_runs(..., CoverageScope::Row)` also marks `has_break` and `first_break` for unwitnessed gaps overlapping the bin (including trailing truncation gaps on `relation-opinions`).
   - `Line::alias_plan` provides one time-bounded `CountryLinks` alias resolution shared by `key_runs`, `key_runs_covered`, and `witnessed_spans` so exact states and tiles agree on aliased keys.
4. **Follow evaluator (`crates/dashboard-follow/src/evaluate.rs::write`)**: uses `row_coverage` (checking `lane.read_rows()` via binary search) only for `Rows::Keys(_)`, `Rows::Live(_)`, and `Query::Series { top: None, parts: false, scope: CoverageScope::Row }`. `Rows::All`, `Rows::Top`, `Rows::LiveTable`, part enumerations, and default `CoverageScope::Table` series queries refuse the whole answer via `Reasons::unrecorded`. `playhead::Lane::finish` compares the sorted witnessed slot slice against the previous hour only when both hours are `Status::Truncated` with the same `Status` variant, setting `Changes::read_rows_changed()` so `Follow::touch` invalidates keyed queries when witnesses swap across consecutive truncated hours.
5. **Follow parser (`crates/dashboard-follow/src/parse.rs::parse`)**: defaults `Query::Series` to `CoverageScope::Table`; accepts outer `"coverage": "row"` only on non-ranked, non-parts keyed series requests and rejects it on ranked `top`, `parts`, and `log-page` queries. UI callers pass `coverage: 'row'` only for per-country line-head reads (`MeasureRace` when `!racing`, `CasualtiesWidget` when `top == null`, and single-country/line widgets). `FactionTotalsView` and race-mode `MeasureRace` omit `coverage` and run under `CoverageScope::Table`.
6. **Live rows (`crates/dashboard-follow/src/follow.rs::Follow::evaluate`, `crates/dashboard-follow/src/live_rows.rs::LiveRows::evaluate`)**: `Rows::Live` routes per-key coverage through `Reasons::row_coverage -> row_coverage`, borrowing `NO_ROW` when absent on complete hours so no extra string allocation occurs. `Rows::LiveTable` refuses via `Reasons::unrecorded`.
7. **Live cells (`crates/dashboard-follow/src/live.rs::group_values`)**: each `Group` carries `scope: CoverageScope`. `Cells::value`, `sum`, `difference`, and `ratio` register in `CoverageScope::Row` groups (`Rows::Live`), which binary-search `TableView::Truncated` per key and pass `row.is_some()` to `row_coverage` (allocating `CellValue::Missing` only for keys that are actually missing). `Cells::table_sum` registers in a `CoverageScope::Table` group (`Rows::LiveTable`) and refuses via `evaluate::unrecorded`. `crates/dashboard-naval/src/products/minefields.rs` registers per-owner mine cells via `value` (`Row`) and the region `"total"` via `table_sum` (`Table`), so witnessed owners stay live while `"total"` always remains registered and reports `main`'s truncation reason.
8. **Casualties UI (`ui/src/widgets/casualties/casualtiesData.ts`, `CasualtiesWidget.tsx`)**: retains explicitly requested unread keyed rows with their truncation reason in tooltips and surfaces unread countries with their truncation reason in the race-mode unknown note (`casualtyUnknownNote`).

### Out of scope (deferred to follow-up)

Whole-population results remain on `CoverageScope::Table` (`origin/main` behaviour) in this ADR and are left to a dedicated follow-up: ranks, totals, top-N, ranked summaries and their `membership` (`Line::ranked`, `rank_at_by`, `series/ranked.rs`), `totals_ranked`, faction lines/totals (`dashboard-wars/src/products/factions.rs`, `FactionTotalsView`), owner part enumerations (`part_lives`, `owned_at`), race-mode `MeasureRace`, and any whole-population refusal fields in the generated TypeScript contract (`ui/src/generated/dashboard.ts`).

## Consequences

- Complete hours perform zero extra witness comparisons, zero extra tile storage reads (both unfiltered and `within`-filtered), and zero extra scalar `at_each_with_status` re-reads (both unvalued and absent holders).
- Whole-population callers cannot accidentally inherit keyed-row coverage through shared functions.
- No on-disk recording format or TypeScript contract changes are introduced.
- Every `CoverageScope::Row` path agrees on values, `NotApplicable`, absence, and canonical missing reasons across complete, truncated, unavailable, and not-recorded hours.
