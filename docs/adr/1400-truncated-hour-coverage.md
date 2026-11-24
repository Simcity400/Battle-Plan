# 1400. Truncated-hour coverage and the single witness rule

## Context

When the collector hits a row or budget cap on an hour, columnar storage records the table's status as `Status::Truncated { missing }` together with the exact sorted list of slots that were read at that hour (`TableReader::read_rows()`, `Lane::read_rows()`, `TableView::Truncated { rows, missing }`).

Previously, different dashboard query paths disagreed on truncated hours:

- `dashboard-query/src/series/board.rs` and `dashboard-follow/src/evaluate.rs` refused every row of a truncated table with a message claiming which rows were read was not held at the head.
- `dashboard-follow/src/live.rs` rejected all live cells on `TableView::Truncated`, even for witnessed keys.
- `dashboard-query/src/series.rs` served witnessed scalar rows through `key_runs`, but refused computed `NotApplicable` on truncated hours, fell back to generic measure-unrecorded text or `missing: None` on unwitnessed rows, marked all truncated spans missing in `Line::missing` (blanking witnessed charts and all-witnessed faction/group lines), missed unwitnessed truncation breaks inside bins in `fold_reads`, and served partial-population rankings on truncated interior bins in `Line::ranked` while silently returning empty lists on `<6h`, edge, and single-hour ranked queries.
- `playhead::Lane` and `dashboard-follow::Follow` only invalidated on `Status` changes, so consecutive truncated hours with the same `missing` count that swapped witnessed rows (`{A}` -> `{B}`) did not re-evaluate followed keyed queries.

## Decision

One function in `dashboard-query` defines row coverage for every query path:

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
3. `Status::Truncated { missing }` with `witnessed == false` -> `Coverage::NotRecorded` with the canonical truncation reason below. An unwitnessed row is never treated as absent or zero, even on `absent_none` or `absent_is_zero` tables.
4. `Status::NotRecorded` and `Status::Unavailable { .. }` -> `Coverage::NotRecorded` with the canonical reasons below.
5. Whole-population queries (top-N ranking, all rows, owner parts enumeration, group/faction totals missing any member, and asserting a key is absent) pass `witnessed = false`, refusing any non-`Complete` hour by name.

### Canonical reason strings

Let `{Name}` be `table.words()` with its first letter capitalized (for example, `Countries`, `Country factories`, `Country experience`, `Country resources`). Every path and UI consumer uses these exact strings, free of internal terms:

- `Status::NotRecorded`:
  `"{Name} not recorded at this hour"`
- `Status::Truncated { missing }` (when `!witnessed`):
  `"{Name} read only in part at this hour ({missing} rows not read)"`
- `Status::Unavailable { .. }` with non-empty `reason`:
  `"{Name} not available at this hour: {reason}"`
- `Status::Unavailable { .. }` with absent or empty `reason`:
  `"{Name} not available at this hour"`

### Path rules and invariants

- **Exact per-hour states (`Line::states`, `owned_at`, `ranked::states`)**: resolve missing/unwitnessed keys through `at_each_with_status` + `row_coverage`; witnessed truncated rows evaluate `Source::read` (preserving `NotApplicable`); unwitnessed rows emit the canonical truncation reason; `ranked::states` refuses non-`Complete` hours via `SeriesStateReply.missing` (omitted when `None`) rather than returning an empty list silently.
- **Follow evaluator and invalidation (`playhead::Lane`, `Changes`, `dashboard-follow`)**: keyed queries (`Rows::Keys`, `Rows::Live`, keyed `Query::Series`) check `lane.read_rows()` (`O(log W)` binary search) via `row_coverage`; population queries (`Rows::All`, `Rows::Top`, `Ranking::write`, `top: Some(_)`, `parts: true`, `listed`) pass `witnessed = false`. `Lane::note_read` compares the sorted witnessed slot slice against the previous hour only when both previous and current hours are `Status::Truncated`, setting `Changes::read_rows_changed()` (zero comparisons on complete hours).
- **Rows-at-an-hour live cells (`dashboard-follow/src/live.rs`, `at_hour`, `at_hour/incremental.rs`)**: `group_values` binary-searches `TableView::Truncated { rows, missing }` per key and applies `row_coverage`, serving witnessed cells and marking unwitnessed keys with the canonical truncation reason (never `NO_ROW`).
- **Summaries, tiles, and ranked summaries (`Line::missing`, `fold_reads`, `Line::ranked`, `SeriesRankedReply`, `totals_ranked`)**:
  - `Line::missing` keeps its public signature stable. On complete hours it does zero extra work; on `Truncated` status spans it suppresses truncation `MissingSpan`s for hours where the key was witnessed and emits the canonical reason only for unwitnessed hours.
  - `fold_reads` sets `has_break` on unwitnessed/non-complete gaps overlapping a bin, while keeping `value.hours < lived` unchanged on complete hours so a key entering or leaving mid-bin on complete hours produces `main`'s exact bucket.
  - `Line::ranked` checks table statuses per bin, blanking only non-`Complete` bins while keeping complete bins drawn, and records the refused spans and canonical reasons in `SeriesRankedReply.missing` (omitted when empty) and each `SeriesRankedLine.missing`.
- **Group and faction lines (`FactionLines`)**: inherit exact per-key `Line::missing` so all-witnessed groups plot cleanly on truncated hours while any unwitnessed member blanks only that bin with the canonical truncation reason.
- **Scoreboard (`board::at`)**: `board::values` looks up keys in sorted `snapshot.rows` (already filtered to witnessed slots) and applies `row_coverage`, serving witnessed countries and refusing unwitnessed rows (even with `absent_none`) and `Rows::Top` rankings with canonical reasons.

## Consequences

- Complete recordings pay zero extra comparisons, zero extra storage reads, and produce identical wire output (`SeriesStateReply.missing` and `SeriesRankedReply.missing` are omitted when empty/absent).
- Exact reads, followed playback, live cells, scoreboards, summaries, tiles, and faction lines agree on values, `NotApplicable`, absence, and missing reasons across complete, truncated, unavailable, and not-recorded hours.
- `crates/dashboard-query/src/head.rs` and `crates/dashboard-base/src/products/series_head.rs` remain untouched; `series-head` automatically receives per-key truncated missing spans through `Line::missing`.
