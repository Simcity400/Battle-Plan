# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes known values (including zero) from unrecorded readings. In `naval-battles`, absent provinces previously defaulted to `0`, moving battles kept only the initial province, unread participant hours with a carried historical peak reported `0` ships, and on complete participant snapshots a missing owner hid the ship count (or a missing ship hid the owner).

## Decision

Keep the recording format unchanged and represent unrecorded naval-battle fields explicitly:

- **Naval battles (`naval-battles`, `naval-battles-ranking`)**:
  - Make `provinceId` optional on `NavalBattleHistory` and add optional per-run `provinceId` on `NavalBattleRun`, resolving provinces in ranking only after the country filter accepts a battle.
  - Require `NavalCombatParticipants` to have `Status::Complete` or `Status::Truncated` (not `Unavailable` or `NotRecorded`) in addition to historical peak presence, preventing unread participant hours from reading as zero ships.
  - On `Status::Complete` participant snapshots, count known ship/convoy types and resolve owners independently.
  - On `Status::Truncated` participant snapshots, preserve `main`'s exact counting and skip semantics verbatim (`sides_at_truncated`); truncated-roster semantics move to issue #286.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings play identically while exposing honest missing states.
- `NavalBattlesWidget` validates nullable per-run `provinceId` and skips place-name requests for `null` provinces.
