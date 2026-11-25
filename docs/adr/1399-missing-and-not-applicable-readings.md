# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes three outcomes for every dashboard reading:

1. A known value, including a real zero.
2. Not recorded, with a reason the user can read.
3. Not applicable, where all inputs were recorded and the derived quantity has no meaningful value (for example, a share or ratio with a zero denominator).

`market-ledger` previously defaulted unrecorded `kind`, `ordinal`, `agreement`, and `variant` cells to sentinel values (`255`, `0`, `0000000000000000`, `"0"`) and collapsed unrecorded party cells to `LogCell::Missing`. In `ui/src/charts/table`, a hidden column's tooltip entry was formatted as a dash for an absent value without consulting the live values' missing reason.

In `construction-log`, unrecorded project classification fields (`kind`, `output_alone`, `repair_levels`) previously fell back to `Building`, `Census`, or `false`, and unrecorded `built`/`repaired` counters defaulted to `0`.
In `land-battles`, an absent end kind, end hour, `province`, or side roster previously defaulted to `0`, the settlement hour, or empty lists, and manpower loss rendering required both sides to be known together.
Declared computed measures in `dashboard-declare` (`supply-health` shares `noSupplyShare`, `strainedShare`, `inSupplyShare`; `autonomy` `progress`; and `fuel-consumers` `receivedShare`) previously returned `Option<i64>` (`None`) for both a zero denominator and an unrecorded input, making "nothing to divide" and "not recorded" indistinguishable across exact states, follow, and scoreboards.

In `naval-battles`, absent provinces previously defaulted to `0`, moving battles kept only the initial province, unread participant hours with a carried historical peak reported `0` ships, and on complete participant snapshots a missing owner hid the ship count (or a missing ship hid the owner).
In `lend-lease-history`, ended agreements previously reconstructed fields with `unwrap_or_default()`, turning unrecorded numeric and fuel columns into `0`, passing `0` as an unrecorded `receiver` into tag lookup, and returning empty pools alongside only an aggregate `whole` boolean.

## Decision

Keep the recording format unchanged and express missing and not-applicable states directly in the existing contracts:

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
- **Naval battles (`naval-battles`, `naval-battles-ranking`)**:
  - Make `provinceId` optional on `NavalBattleHistory` and add optional per-run `provinceId` on `NavalBattleRun`, setting `place.namedAt` when a province first becomes known or changes after the battle's first hour while keeping `place.hour` at the battle start, and resolving provinces in ranking only after the country filter accepts a battle.
  - Read `NavalCombatParticipants` statuses coalesced into complete, truncated, and missing spans: on `Complete`, evaluate `sides_at` independently of historical peak presence (emitting positive lower bounds with `atLeast: true` when another participant has an unread ship type, and leaving zero counts unknown); on `Truncated`, require peak presence and preserve `main`'s `sides_at_truncated` and battle/participant change-hour tag lookup verbatim; on `Unavailable` or `NotRecorded`, leave sides unknown. Truncated-roster semantics remain in issue #286.
- **Land battles (`land-battles-tail`, `land-battle-course`)**
  - A result-only battle carries the province on the result and appears when the recording first supplies it. The course distinguishes an unknown end kind, an unknown end hour and an unknown winner.
  - Each side retains its recorded losses independently. An unavailable comparison uses the shared hatched bar. Unknown country attribution appears in the filtered log's warning or dated membership gap.
- **Computed measures (`replay-columnar`, `dashboard-declare`, `dashboard-query`, `dashboard-follow`)**:
  - `summary::Source::Computed` returns `Computed::{Value(i64), NotApplicable(&'static str), Unrecorded}` via `Source::read`, while `Source::value` keeps returning `Option<i64>` so on-disk summary pieces are unaffected. Every computed measure reads all required operands before declaring `NotApplicable`, so an unrecorded numerator or operand never masks as `NotApplicable`.
  - On the columnar follow wire (`dashboard-follow/src/evaluate.rs`), `Computed::NotApplicable(reason)` maps to `State::Missing` with `table.explain(reason)`, while `Computed::Unrecorded` maps to `State::Unrecorded`.
  - On exact per-hour state reads (`Line::states` via `replay_lanes::lanes::at_each_with_status`, reading the settled `TableReader::status()` for each asked hour), add an optional `notApplicable?: boolean` flag on `SeriesState` (`true` for `NotApplicable` on complete hours, `false` for unrecorded on complete hours, absent on truncated/non-complete hours so truncated hours stay identical to `main`).
  - Scoreboards (`series/board.rs`) read `Source::read` on complete hours and surface the `NotApplicable` reason directly.
  - Summaries, tiles, and group lines remain identical to `main` (tracked in issue #285 item 14 for the time chart family); occupation means/detail and ranking/older-summary computed-column edges remain in issue #285; truncated-hour semantics remain in issue #286.
- **Lend-lease ended agreements (`lend-lease-history`)**:
  - Decode each ended agreement's fields individually (`EndedRow` / `EndedLeaseTerms` / `EndedLease`) without `unwrap_or_default()`, rejecting trailing bytes in pool decoders (`Decoder::finish`) and distinguishing unrecorded pools from corrupt pools.
  - Known values serialize as bare values identical byte-for-byte to `main`; missing values serialize as `null` paired with an optional sibling `<field>Missing: string` reason emitted only when missing (including `giverMissing` and `receiverMissing` naming any recorded country ID whose tag could not be resolved).
  - `validateLeaseHistory` accepts both current replies and older hosts' replies unchanged (where `giver`/`receiver` may be `null` without a sibling reason).
  - Ended-agreement lifetime evidence (`endedBy`, paging), standing partial agreements, and holder-filter distinctions remain identical to `main` and are tracked in issue #285 (items 15–20) to be rebuilt on issue #286's unified status function.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings remain compatible while exposing honest missing and not-applicable states.
- Market ledger rows display a missing party or amount as text carrying its own state and reason, instead of fabricated zeroes or bare dashes. A hidden column's tooltip entry shows and speaks the reason itself.
- For construction log rows, a rise from 1 to 2 at the first read of a new window now displays 2 instead of `main`'s 1 because `joinProject` no longer discards incoming `carried: true` counts.
- `ConstructionLogWidget` paints live level counts through `LiveText` (and factories through `LiveFactories`) alongside separate source-uncertainty text when `source === 'unknown'`, and `projectTooltip` formats count rows through `builtLabel`.
- `NavalBattlesWidget` validates nullable per-run `provinceId`, includes `provinceId` in its `battleStateRows` state key, and skips place-name requests for `null` provinces.
- `LandBattlesWidget`, `LandBattleList`, `LandBattleDrill`, and `BattleLog` render missing provinces, endings, rosters, and per-side losses without fabricating zeroes, hiding recorded winners, extending drill curves past settlement, or querying province `0`.
- `AutonomyWidget` passes the not-applicable reason to `formatProgress` only when `line.notApplicable.has(hour)` is true (keeping `"Not recorded"` on truncated hours and older hosts), and `FuelBranchesWidget` formats tooltip rows as `'No share'` vs `'Not recorded'` with `detail: missing` from `keyedSeriesData`.
- `LendLeaseAgreementsWidget` and `agreementsData.ts` render missing ended-agreement fields through `ValueState` (and `Table`'s `missingText` for missing Delivered and Lost cells), preserve recorded `activeSince = 0` and `lastDelivery`, keep `main`'s row keys, visibility gate, and country filtering, and accept older hosts' history replies unchanged.
