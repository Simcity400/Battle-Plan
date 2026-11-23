# 1399. Missing and not-applicable readings in dashboard contracts

## Context

BattlePlan distinguishes known values (including zero) from unrecorded readings. In `construction-log`, unrecorded project classification fields (`kind`, `output_alone`, `repair_levels`, `end`) previously fell back to `Building`, `Census`, `false`, or an open run, and unrecorded `built`/`repaired` counters defaulted to `0`.

## Decision

Keep the recording format unchanged and represent unrecorded project fields and counter spans explicitly in `construction-log`:

- **Construction log (`construction-log`)**:
  - Add `Unknown` variants to `ProjectKind` and `ProjectSource`, emit per-hour `builtUnread` / `repairedUnread` spans on `ConstructionProject` within each window (extending to the window's end when a project disappears without a resolving counter read), keep `repair` as `None` when `repair_levels` is unrecorded, require explicit `output_alone == 0` and `end == OPEN` before emitting an open `ProjectRun`, and preserve witnessed sources and carried built counts when joining windows.
  - Cross-window unread counter carry stays tracked in issue #285.

## Consequences

- No recording-format, collector, or recorder changes are required; older recordings play identically while exposing honest missing states.
- `ConstructionLogWidget` renders missing built/repaired counts through `components/ValueState` instead of fabricated zeroes.
