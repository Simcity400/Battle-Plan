# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes three outcomes for every dashboard reading:

1. A known value, including a real zero.
2. Not recorded, with a reason the user can read.
3. Not applicable, where all inputs were recorded and the derived quantity has no meaningful value.

`market-ledger` previously defaulted unrecorded `kind`, `ordinal`, `agreement`, and `variant` cells to sentinel values (`255`, `0`, `0000000000000000`, `"0"`) and collapsed unrecorded party cells to `LogCell::Missing`. In `ui/src/charts/table`, a hidden column's tooltip entry was formatted as a dash for an absent value without consulting the live values' missing reason.

## Decision

Keep the recording format unchanged and express missing states directly in the existing contracts:

- **Market ledger (`market-ledger`)**:
  - Use the existing `LogCell::NotRecorded` variant for unrecorded `kind`, `contract`, `seller`, `buyer`, and `variant` cells, and emit a specific refusal-missing message when `kind == 34` has an unrecorded `ordinal`.
  - Mode-unknown `kind == 34` rows use neutral wording and disable contract/agreement navigation when `kind`, `contract`, or delivery mode is unknown.
- **Table layout (`ui/src/charts/table`)**:
  - Preserve each column's `missing` reason formatter even when the column is hidden, so hidden-column lookups never lose their missing-value reason.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings play identically while exposing honest missing states.
- Market ledger rows display a missing party or amount as text carrying its own state and reason, instead of fabricated zeroes or bare dashes. A hidden column's tooltip entry shows and speaks the reason itself.
