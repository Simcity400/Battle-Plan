# E12 D04 Active Naval OOB Validation

This packet resolves the D04 live-vs-melt mismatch that previously showed
1,497 common ship ids plus 255 save-side extras. The save-side extras were a
mix of true active task-force ships and non-active `task_force` references; the
live lane was also under-reporting real active task-force ships.

## Fixture

- Game: `hoi4.exe` PID 19204
- Build SHA:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
- Pause gate: `u32[CGameState+0x468] == 60830441`
- Save oracle:
  `extractor/native/runs/division_manpower_assignment_870ebf_20260703_wave2/00_d04_1944_melted.txt`

## Active Save Parser Rule

Count a save-side task force only when the block has its own
`id={ id=<N> type=61 }` and at least one direct child `ship={...}` block with
its own `id={ id=<N> type=51 }`. Ship `definition` tokens are read only from
direct ship-block depth. Empty task-force references, history/refit/sunk-ship
definitions, and pending reference blocks are excluded from active OOB.

The ignored run evidence copy is under
`extractor/native/runs/e12_naval_per_hour_fold_verify/EVIDENCE.md`; that path is
ignored by repository rules, so this lane note is the tracked review surface.

## Live vs Melt Result

| Field | Result |
| --- | ---: |
| Melt active task forces | 206 |
| Live task forces | 206 |
| Melt active ships | 1,690 |
| Live ships | 1,690 |
| Common ship ids | 1,690 / 1,690 |
| Live-only ship ids | 0 |
| Melt-only ship ids | 0 |
| Definition-token mismatches | 0 |
| Task-force-id mismatches | 0 |
| Ignored non-active task-force refs | 103 |

The final live read took 1,069.7 ms on the paused D04 process.

## Fix Summary

The old live traversal did not expand direct `CFleet` subobject candidates the
same way as vector-path candidates, and it stopped after the first valid
`CTaskForce` body delta for a shared subobject candidate. The collector and
lane-local probe now normalize direct and vector candidates through the same
helper and insert every valid body delta.

D56 naval mission/task-force fold rows now also include D29-derived
`ship_count` and `dominant_hull_class`. These fields are emitted as `null`
unless a clean D29 ship fold is available for that hour.
