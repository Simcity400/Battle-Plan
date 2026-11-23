# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes known values (including zero) from unrecorded readings. In `land-battles`, an absent end kind, end hour, `province`, or side roster previously defaulted to `0`, the settlement hour, or empty lists, and manpower loss rendering required both sides to be known together.

## Decision

Keep the recording format unchanged and represent unrecorded land-battle fields explicitly:

- **Land battles (`land-battles`, `land-battles-ranking`, `land-battle-course`)**:
  - Make `LandBattleResultRead.hour` optional and `provinceId` optional on `LandBattleHistory` and `LandBattleCourse`.
  - Unread end `kind` or `end_hour` produces `BattleEnd::Unknown` with a specific reason (never defaulting to `0` or copying `written`), while ranking and displayed battle length preserve `main`'s settlement-hour fallback (`result.hour ?? settledHour ?? read`) so length never regresses; duration/length semantics are tracked in issue #285.
  - Unread or corrupt side lists on `LandBattleEnds` or `LandCombatSides` mark that side unknown in both ranking and display so country filters never exclude the battle and winning-side display names only rosters read at the result.
  - Per-side manpower losses render and paint independently during playback with a hatched comparison bar when either side is missing.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings play identically while exposing honest missing states.
- `LandBattlesWidget` and `BattleLog` render missing provinces, endings, rosters, and per-side losses without fabricating zeroes or querying province `0`.
