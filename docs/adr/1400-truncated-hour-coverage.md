# 1400. Truncated-hour coverage and the single keyed-row witness rule

## Context

When the collector hits a row or budget cap on an hour, columnar storage records the table's status as `Status::Truncated { missing }` together with the exact sorted list of slots that were read at that hour (`TableReader::read_rows()`, `Lane::read_rows()`, `TableView::Truncated { rows, missing }`).

Previously, different keyed-row dashboard query paths disagreed on truncated hours:

- `dashboard-query/src/series/board.rs` and `dashboard-follow/src/evaluate.rs` refused every row of a truncated table with a message claiming which rows were read was not held at the head.
- `dashboard-follow/src/live.rs` rejected all live cells on `TableView::Truncated`, even for witnessed keys.
- `dashboard-query/src/series.rs` served witnessed scalar rows through `key_runs`, but refused computed `NotApplicable` on truncated hours, fell back to generic measure-unrecorded text or `missing: None` on unwitnessed rows, marked all truncated spans missing in `Line::missing` (blanking witnessed charts), and missed unwitnessed truncation breaks inside bins in `Line::binned`.
- `playhead::Lane` and `dashboard-follow::Follow` only invalidated on `Status` changes, so consecutive truncated hours with the same `missing` count that swapped witnessed rows (`{A}` -> `{B}`) did not re-evaluate followed keyed queries.

## Decision

One function in `crates/dashboard-query/src/coverage.rs` defines row coverage for every keyed-row query path:

```rust
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

### Witness rule

1. `Status::Complete` -> `Coverage::Recorded`.
2. `Status::Truncated { .. }` with `witnessed == true` -> `Coverage::Recorded`. The witnessed row is read exactly as on a complete hour, including zero values, unrecorded cells, and computed `NotApplicable`.
3. `Status::Truncated { missing }` with `witnessed == false` -> `Coverage::NotRecorded` with the canonical truncation reason below. An unwitnessed row is never carried forward, never treated as absent or zero (even on `absent_none` or `absent_is_zero` tables), and never silently dropped.
4. `Status::NotRecorded` and `Status::Unavailable { .. }` -> `Coverage::NotRecorded` with the canonical reasons below.

### Canonical reason strings

Let `{Name}` be `table.words()` with its first letter capitalized (for example, `Countries`, `Country factories`, `Country experience`, `Country resources`). Every covered path uses these exact strings, free of internal terms:

- `Status::NotRecorded`:
  `"{Name} not recorded at this hour"`
- `Status::Truncated { missing }` (when `!witnessed`):
  `"{Name} read only in part at this hour ({missing} rows not read)"`
- `Status::Unavailable { .. }` with non-empty `reason`:
  `"{Name} not available at this hour: {reason}"`
- `Status::Unavailable { .. }` with absent or empty `reason`:
  `"{Name} not available at this hour"`

### Keyed-row paths governed by `row_coverage`

Every keyed-row query path below decides coverage through `row_coverage` and nothing else:

1. **Exact unresolved states (`crates/dashboard-query/src/series.rs::Line::states_of`)**: preserves `point.covered` alongside scalar runs via `key_runs_covered` so unvalued complete-hour scalars never trigger an `at_each_with_status` re-read; unresolved keys resolve through `at_each_with_status` + `row_coverage` (preserving `Computed::NotApplicable` on witnessed truncated rows and emitting the canonical truncation reason on unwitnessed rows).
2. **Scoreboard cells (`crates/dashboard-query/src/series/board.rs::values`)**: looks up each requested country key in sorted `snapshot.rows` (already filtered to witnessed slots) and passes `slot.is_some()` to `row_coverage`, serving witnessed cells and refusing unwitnessed rows (even on `absent_none` series) with the canonical reason.
3. **Follow evaluator (`crates/dashboard-follow/src/evaluate.rs`)**: keyed queries (`Rows::Keys`, `Rows::Live`, keyed `Query::Series`) check `lane.read_rows()` (`O(log W)` binary search) and decide per-key coverage through `Reasons::row_coverage -> coverage::row_coverage`. `playhead::Lane::finish` compares the sorted witnessed slot slice against the previous hour only when both previous and current hours are `Status::Truncated` with the same `Status` variant (zero comparisons on complete hours), setting `Changes::read_rows_changed()` so `Follow::touch` invalidates keyed queries when witnesses swap across consecutive truncated hours.
4. **Live rows (`crates/dashboard-follow/src/live_rows.rs`, `crates/dashboard-follow/src/follow.rs`)**: `LiveRows::evaluate` routes every key through `Reasons::row_coverage -> row_coverage` with zero allocation on complete hours.
5. **Live cells (`crates/dashboard-follow/src/live.rs::group_values`)**: binary-searches `TableView::Truncated { rows, missing }` per key and passes `row.is_some()` directly to `row_coverage`, serving witnessed cells and refusing unwitnessed keys with the canonical truncation reason (borrowing `NO_ROW` on complete hours so no allocation occurs when every key exists).
6. **Binned gaps and `Line::missing` (`crates/dashboard-query/src/series.rs::Line::part_coverage`, `Line::binned`, `Line::missing`, `crates/dashboard-query/src/tile.rs`)**:
   - `Line::missing` keeps its public signature unchanged.
   - `tile` computes `Line::binned` before `Line::missing`, and `Line::statuses` reuses any cached status read covering the requested span without calling `status_reasons` when no `Unavailable` span exists, so complete tiles perform the exact same number of storage reads as `main`.
   - `Line::part_coverage` decides every status run through `row_coverage` and computes the unwitnessed complement once per `(run, key, first, last)` in `Line::unwitnessed_spans` (cached in `Unread`), shared between `Line::binned` and `Line::missing`.
   - `Line::alias_plan` provides one time-bounded `CountryLinks` alias resolution shared by `key_runs`, `key_runs_covered`, and `witnessed_spans` so exact states and tile coverage agree on aliased keys (such as `relation-opinions`).
7. **Keyed UI consumers & whole-population guards**:
   - `ui/src/widgets/casualties/casualtiesData.ts` and `CasualtiesWidget.tsx` retain explicitly requested unread `CELL.noRow` rows with their truncation reasons instead of dropping them.
   - `crates/dashboard-naval/src/products/minefields.rs` skips registering the partial-owner `total` sum on truncated `NavalRegionalMines` frames while keeping witnessed owner cells live.
   - `ui/src/charts/race/factionTotalsData.ts` and `FactionTotalsView.tsx` refuse multi-faction rankings when any competing member has a truncation refusal and report the true `unknown` count on live renders.

### Out of scope (deferred to follow-up)

Whole-population results remain unchanged from `main` in this ADR and are left to a dedicated follow-up: ranks, totals, top-N, ranked summaries and their `membership` (`Line::ranked`, `rank_at_by`, `series/ranked.rs`), `totals_ranked`, faction lines/totals (`dashboard-wars/src/products/factions.rs`), and whole-population refusal fields in the generated TypeScript contract (`ui/src/generated/dashboard.ts`).

## Consequences

- Complete hours perform zero extra witness comparisons, zero extra tile storage reads, zero extra scalar `at_each_with_status` re-reads, and zero extra `NO_ROW` allocations.
- No on-disk recording format or TypeScript contract changes are introduced.
- Exact keyed states, followed playback, live rows, live cells, scoreboard cells, summaries, and tiles agree on values, `NotApplicable`, absence, and missing reasons across complete, truncated, unavailable, and not-recorded hours.
