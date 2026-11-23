# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes three outcomes for every dashboard reading:

1. A known value, including a real zero.
2. Not recorded, with a reason the user can read.
3. Not applicable, where all inputs were recorded and the derived quantity has no meaningful value (for example, a share or mean with a zero denominator).

Declared computed measures in `dashboard-declare` (`supply-health` shares, `autonomy` progress, and `fuel-consumers` received share) previously returned `Option<i64>` (`None`) for both a zero denominator and an unrecorded input, making "nothing to divide" and "not recorded" indistinguishable across exact states, follow, scoreboards, summaries, and tiles.

## Decision

Keep the recording format unchanged and express the distinction in the query-layer contract:

- **Computed series (`replay-columnar`, `dashboard-declare`, `dashboard-query`, `dashboard-follow`)**:
  - `summary::Source::Computed` returns `Computed::{Value(i64), NotApplicable(&'static str), Unrecorded}` via `Source::read`, while `Source::value` keeps returning `Option<i64>` so on-disk summary pieces are unaffected. Every computed measure reads all required operands before declaring `NotApplicable`, so an unrecorded numerator or operand never masks as `NotApplicable`.
  - On the columnar follow wire (`dashboard-follow`), `Computed::NotApplicable(reason)` maps to `State::Missing` with `table.explain(reason)`, while `Computed::Unrecorded` maps to `State::Unrecorded`.
  - In `SeriesState`, add an optional `notApplicable?: boolean` flag alongside `missing: string | null` (`true` for `NotApplicable` on complete hours, `false` for unrecorded on complete hours, absent on non-complete/truncated hours so existing truncated tooltips stay unchanged).
  - For displayed series (exact keyed reads on complete hours, hourly tiles, summary-backed coarse tiles, and coarse summaries), `Line::missing_batch` walks disjoint uncovered intervals with one `TableReader` per run, and exact `Line::states` uses the row view's status directly and checks status only at requested hours whose row is absent. Scoreboards (`series/board.rs`) likewise read `Source::read` on complete hours and surface the `NotApplicable` reason directly.
  - Truncated-hour semantics remain identical to `main` (tracked in issue #286); occupation means/detail and ranking/older-summary computed-column edges remain tracked in issue #285.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings play identically while exposing honest not-applicable vs unrecorded states.
- Keyed-series, autonomy, and fuel-branches widgets render not-applicable and missing values through `components/ValueState`.
