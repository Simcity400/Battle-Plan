# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes three outcomes for every dashboard reading:

1. A known value, including a real zero.
2. Not recorded, with a reason the user can read.
3. Not applicable, where all inputs were recorded and the derived quantity has no meaningful value (for example, a share or ratio with a zero denominator).

`market-ledger` previously defaulted unrecorded `kind`, `ordinal`, `agreement`, and `variant` cells to sentinel values (`255`, `0`, `0000000000000000`, `"0"`) and collapsed unrecorded party cells to `LogCell::Missing`. In `ui/src/charts/table`, a hidden column's tooltip entry was formatted as a dash for an absent value without consulting the live values' missing reason.

Declared computed measures in `dashboard-declare` (`supply-health` shares `noSupplyShare`, `strainedShare`, `inSupplyShare`; `autonomy` `progress`; and `fuel-consumers` `receivedShare`) previously returned `Option<i64>` (`None`) for both a zero denominator and an unrecorded input, making "nothing to divide" and "not recorded" indistinguishable across exact states, follow, and scoreboards.

## Decision

Keep the recording format unchanged and express missing and not-applicable states directly in the existing contracts:

- **Market ledger (`market-ledger`)**:
  - Use the existing `LogCell::NotRecorded` variant for unrecorded `kind`, `contract`, `seller`, `buyer`, and `variant` cells, and emit a specific refusal-missing message when `kind == 34` has an unrecorded `ordinal`.
  - Mode-unknown `kind == 34` rows use neutral wording and disable contract/agreement navigation when `kind`, `contract`, or delivery mode is unknown.
- **Table layout (`ui/src/charts/table`)**:
  - Preserve each column's `missing` reason formatter even when the column is hidden, so hidden-column lookups never lose their missing-value reason.
- **Computed measures (`replay-columnar`, `dashboard-declare`, `dashboard-query`, `dashboard-follow`)**:
  - `summary::Source::Computed` returns `Computed::{Value(i64), NotApplicable(&'static str), Unrecorded}` via `Source::read`, while `Source::value` keeps returning `Option<i64>` so on-disk summary pieces are unaffected. Every computed measure reads all required operands before declaring `NotApplicable`, so an unrecorded numerator or operand never masks as `NotApplicable`.
  - On the columnar follow wire (`dashboard-follow/src/evaluate.rs`), `Computed::NotApplicable(reason)` maps to `State::Missing` with `table.explain(reason)`, while `Computed::Unrecorded` maps to `State::Unrecorded`.
  - On exact per-hour state reads (`Line::states` via `replay_lanes::lanes::at_each_with_status`, reading the settled `TableReader::status()` for each asked hour), add an optional `notApplicable?: boolean` flag on `SeriesState` (`true` for `NotApplicable` on complete hours, `false` for unrecorded on complete hours, absent on truncated/non-complete hours so truncated hours stay identical to `main`).
  - Scoreboards (`series/board.rs`) read `Source::read` on complete hours and surface the `NotApplicable` reason directly.
  - Summaries, tiles, and group lines remain identical to `main` (tracked in issue #285 item 14 for the time chart family); occupation means/detail and ranking/older-summary computed-column edges remain in issue #285; truncated-hour semantics remain in issue #286.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings remain compatible while exposing honest missing and not-applicable states on exact reads, follow, and scoreboards.
- Market ledger rows display a missing party or amount as text carrying its own state and reason, instead of fabricated zeroes or bare dashes. A hidden column's tooltip entry shows and speaks the reason itself.
- `AutonomyWidget` passes the not-applicable reason to `formatProgress` only when `line.notApplicable.has(hour)` is true (keeping `"Not recorded"` on truncated hours and older hosts), and `FuelBranchesWidget` formats tooltip rows as `'No share'` vs `'Not recorded'` with `detail: missing` from `keyedSeriesData`.
