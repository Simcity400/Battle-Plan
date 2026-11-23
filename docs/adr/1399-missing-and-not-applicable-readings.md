# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes three outcomes for every dashboard reading:

1. A known value, including a real zero.
2. Not recorded, with a reason the user can read.
3. Not applicable, where all inputs were recorded and the derived quantity has no meaningful value.

`market-ledger` previously defaulted unrecorded `kind`, `ordinal`, `agreement`, and `variant` cells to sentinel values (`255`, `0`, `0000000000000000`, `"0"`) and collapsed unrecorded party cells to `LogCell::Missing`. In `ui/src/charts/table`, a hidden column's tooltip entry was formatted as a dash for an absent value without consulting the live values' missing reason.

In `land-battles`, an absent end kind, end hour, `province`, or side roster previously defaulted to `0`, the settlement hour, or empty lists, and manpower loss rendering required both sides to be known together.

## Decision

Keep the recording format unchanged and express missing states directly in the existing contracts:

- **Market ledger (`market-ledger`)**:
  - Use the existing `LogCell::NotRecorded` variant for unrecorded `kind`, `contract`, `seller`, `buyer`, and `variant` cells, and emit a specific refusal-missing message when `kind == 34` has an unrecorded `ordinal`.
  - Mode-unknown `kind == 34` rows use neutral wording and disable contract/agreement navigation when `kind`, `contract`, or delivery mode is unknown.
- **Table layout (`ui/src/charts/table`)**:
  - Preserve each column's `missing` reason formatter even when the column is hidden, so hidden-column lookups never lose their missing-value reason.
- **Land battles (`land-battles`, `land-battles-ranking`, `land-battle-course`)**:
  - Make `LandBattleResultRead.hour` optional and `provinceId` optional on `LandBattleHistory` and `LandBattleCourse`. For a result-only key, use only the end row's own `province` and keep `None` when it is absent (never borrowing a live census province on the same handle).
  - Unread end `kind` produces `"Ending kind not recorded"` and unread `end_hour` produces `"Ending hour not recorded"` (including when `kind == UNKNOWN`, while `kind == UNKNOWN` with a recorded hour keeps `"Ending not recorded"` and `kind == RESULT_ONLY` intentionally emits `end: None` because it carries only a standalone result). A recorded winner still displays in the list, tooltip, and drill once settled, with `"Ending hour not recorded"` confined to the end-date field; the drill caps plotted curves at the settlement bound (`settledHour`) while keeping the end hour unknown. Ranking and displayed length keep `main`'s settlement-hour fallback (`result.hour ?? settledHour ?? read`); duration/length semantics are tracked in issue #285 (item 9).
  - Unread or corrupt side lists on `LandBattleEnds` (with a result ID) or `LandCombatSides` mark that side unknown in both ranking and display so country filters never exclude the battle, and winning-side labels stay visible at narrow widths (`.lb-winner-side`) without inheriting earlier run rosters. Merged result-only rows without a single result ID remain discarded as on `main` and are tracked in issue #285 (item 13).
  - Per-side manpower losses on shaped battles render and paint independently during playback with a hatched comparison bar when either side is missing.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings remain compatible while exposing honest missing states.
- Market ledger rows display a missing party or amount as text carrying its own state and reason, instead of fabricated zeroes or bare dashes. A hidden column's tooltip entry shows and speaks the reason itself.
- `LandBattlesWidget`, `LandBattleList`, `LandBattleDrill`, and `BattleLog` render missing provinces, endings, rosters, and per-side losses without fabricating zeroes, hiding recorded winners, extending drill curves past settlement, or querying province `0`.
