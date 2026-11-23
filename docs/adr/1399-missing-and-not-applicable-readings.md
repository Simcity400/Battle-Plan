# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes three outcomes for every dashboard reading:

1. A known value, including a real zero.
2. Not recorded, with a reason the user can read.
3. Not applicable, where all inputs were recorded and the derived quantity has no meaningful value.

`market-ledger` previously defaulted unrecorded `kind`, `ordinal`, `agreement`, and `variant` cells to sentinel values (`255`, `0`, `0000000000000000`, `"0"`) and collapsed unrecorded party cells to `LogCell::Missing`. In `ui/src/charts/table`, a hidden column's tooltip entry was formatted as a dash for an absent value without consulting the live values' missing reason.

In `construction-log`, unrecorded project classification fields (`kind`, `output_alone`, `repair_levels`) previously fell back to `Building`, `Census`, or `false`, and unrecorded `built`/`repaired` counters defaulted to `0`.

In `lend-lease-history` and `market-lendlease`, ended agreements previously reconstructed fields with `unwrap_or_default()`, turning unrecorded numeric and fuel columns into `0`, passing `0` as an unrecorded `receiver` into tag lookup, returning empty pools alongside only an aggregate `whole` boolean, and fabricating an end on an unread tail. On standing agreements, one row with an unrecorded column caused typed deserialization to mark the entire `LendLeaseAgreements` table unavailable, dropping intact sibling agreements.

## Decision

Keep the recording format unchanged and express missing states directly in the existing contracts:

- **Market ledger (`market-ledger`)**:
  - Use the existing `LogCell::NotRecorded` variant for unrecorded `kind`, `contract`, `seller`, `buyer`, and `variant` cells, and emit a specific refusal-missing message when `kind == 34` has an unrecorded `ordinal`.
  - Mode-unknown `kind == 34` rows use neutral wording and disable contract/agreement navigation when `kind`, `contract`, or delivery mode is unknown.
- **Table layout (`ui/src/charts/table`)**:
  - Preserve each column's `missing` reason formatter even when the column is hidden, so hidden-column lookups never lose their missing-value reason.
- **Construction log (`construction-log`)**:
  - Add `Unknown` variants to `ProjectKind` and `ProjectSource`, emit per-hour `builtUnread` / `repairedUnread` spans on `ConstructionProject` within each window (closing before truncated omissions so truncated hours keep `main`'s behavior, and extending to the window's end only on non-truncated disappearance without a resolving counter read).
  - Keep every witnessed value when an unrelated column is unread: retain witnessed factories and remaining levels when only `END` is unread, inspect recorded rise lists when a counter first becomes readable after an unread step, preserve witnessed `source`/`kind`/`censusRead` within a window using the same latest-witnessed rule as `joinProject`, and evaluate count availability from counter evidence while displaying source uncertainty separately.
  - Keep `repair` as `None` when `repair_levels` is unrecorded, and reconcile incoming `carried: true` counts against prior windows in `joinProject` by adding only the unseen positive delta (`levels.count - prior[side]`).
  - Cross-window unread counter carry is tracked in issue #285 (item 12); truncated-hour semantics remain in issue #286.
- **Lend-lease agreements (`market-lendlease`, `lend-lease-history`)**:
  - Keep every known field on `LeaseTerms` and `EndedLease` as the bare wire value `main` sends, representing missing values as `null` paired with an optional sibling `<field>Missing: string` reason emitted only when missing (matching `receiverMissing` and adding zero bytes to fully recorded rows). Require `<field>Missing` whenever any nullable field (including `receiver`) is `null`, distinguishing unrecorded receivers from recorded country IDs whose tag could not be resolved.
  - Standing agreements decode per-field incrementally from the playhead lane via `at_hour` raw tables (`EXACT_MARKETLENDLEASE` registered in both `dated_panel_of` and `panel::<P>`), filter via `link_slot` before collecting variant names, retain unread-receiver rows only when the country selection has a possible holder, and credit decoded raw rows to `at_hour::counted`.
  - Live fuel cells read per-cell without typed whole-table deserialization, preserving `main`'s table/hour diagnostics on every non-complete status (including `Unsayable` validation on retained rows of truncated snapshots).
  - Pool decoders reject trailing bytes (`Decoder::finish`) and distinguish unrecorded pools from corrupt pools.
  - `EndedLease` carries `endedBy: i32` (the first complete census proving absence after `lastHour`, decided from `lanes::statuses`). Refused, unavailable, or truncated hours inside a gap never fabricate an end; a mid-life truncated gap resets the witnessed `firstHour` of any later agreement while suppressing an end for the earlier interval; final truncated-census end semantics remain identical to `main` and move to issue #286.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings remain compatible while exposing honest missing states.
- Market ledger rows display a missing party or amount as text carrying its own state and reason, instead of fabricated zeroes or bare dashes. A hidden column's tooltip entry shows and speaks the reason itself.
- For construction log rows, a rise from 1 to 2 at the first read of a new window now displays 2 instead of `main`'s 1 because `joinProject` no longer discards incoming `carried: true` counts.
- `ConstructionLogWidget` paints live level counts through `LiveText` (and factories through `LiveFactories`) alongside separate source-uncertainty text when `source === 'unknown'`, and `projectTooltip` formats count rows through `builtLabel`.
- `LendLeaseAgreementsWidget` and `TradeMarketView` validate the bare-value + sibling-reason contract, preserve recorded start hour `0`, key ended rows by witnessed lifetime (`key:firstHour:lastHour`) gated on `endedBy`, and read exact `bigint` values via `.raw()`. Missing Delivered and Lost cells in `LendLeaseAgreementsWidget` supply `missingText` and paint directly through `Table`'s `cell.text.put`; non-live missing fields and `TradeMarketView` render through `components/ValueState`.
