# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes three outcomes for every dashboard reading:

1. A known value, including a real zero.
2. Not recorded, with a reason the user can read.
3. Not applicable, where all inputs were recorded and the derived quantity has no meaningful value.

`market-ledger` previously defaulted unrecorded `kind`, `ordinal`, `agreement`, and `variant` cells to sentinel values (`255`, `0`, `0000000000000000`, `"0"`) and collapsed unrecorded party cells to `LogCell::Missing`. In `ui/src/charts/table`, a hidden column's tooltip entry was formatted as a dash for an absent value without consulting the live values' missing reason.

In `lend-lease-history` and `market-lendlease`, ended agreements previously reconstructed fields with `unwrap_or_default()`, turning unrecorded numeric and fuel columns into `0`, passing `0` as an unrecorded `receiver` into tag lookup, returning empty pools alongside only an aggregate `whole` boolean, and fabricating an end on an unread tail. On standing agreements, one row with an unrecorded column caused typed deserialization to mark the entire `LendLeaseAgreements` table unavailable, dropping intact sibling agreements.

## Decision

Keep the recording format unchanged and express missing states directly in the existing contracts:

- **Market ledger (`market-ledger`)**:
  - Use the existing `LogCell::NotRecorded` variant for unrecorded `kind`, `contract`, `seller`, `buyer`, and `variant` cells, and emit a specific refusal-missing message when `kind == 34` has an unrecorded `ordinal`.
  - Mode-unknown `kind == 34` rows use neutral wording and disable contract/agreement navigation when `kind`, `contract`, or delivery mode is unknown.
- **Table layout (`ui/src/charts/table`)**:
  - Preserve each column's `missing` reason formatter even when the column is hidden, so hidden-column lookups never lose their missing-value reason.
- **Lend-lease agreements (`market-lendlease`, `lend-lease-history`)**:
  - Reuse the nullable-value plus optional missing-reason shape (`LeaseAmount`, `LeaseHour`, `LeaseItems`) on `LeaseTerms` and `EndedLease`, plus `receiverMissing` on `LeaseTerms`.
  - Standing agreements decode per-field incrementally from the playhead lane via `at_hour` raw tables (`EXACT_MARKETLENDLEASE` registered in both `dated_panel_of` and `panel::<P>`), filter before collecting variant names, and keep unread-receiver rows on country-filtered replies so widgets count unattributed standing and ended agreements separately.
  - Live fuel cells read per-cell without typed whole-table deserialization, preserving `main`'s table/hour diagnostics on every non-complete status (including `Unsayable` validation on retained rows of truncated snapshots).
  - Pool decoders reject trailing bytes (`Decoder::finish`) and distinguish unrecorded pools from corrupt pools.
  - `EndedLease` carries `endedBy: i32` (the first census proving absence after `lastHour`). Unread intervals with no census sample never fabricate an end; truncated-census end semantics remain identical to `main` and move to issue #286.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings play identically while exposing honest missing states.
- Market ledger rows display a missing party or amount as text carrying its own state and reason, instead of fabricated zeroes or bare dashes. A hidden column's tooltip entry shows and speaks the reason itself.
- `LendLeaseAgreementsWidget` and `TradeMarketView` validate the updated shapes, read exact `bigint` values via `.raw()`, and display missing values through `components/ValueState`.
