# A8 / D56 wave-5 production closeout

## Verdict

The exact D56 task-force status slice is production-ready. No human reverse-
engineering input remains for D56.

The physical schema-v1 row remains 64 bytes, but its former
`repair_progress_milli`, `retreat_state`, `convoy_route_context_id`, fixture,
and field-rejected positions are reserved. The decoder rejects any row or
header that asserts them, and the world fold no longer projects those names.
This prevents a future producer from silently assigning semantics to fields
that a729 does not carry.

## Exact carrier disposition

| Concern | Exact carrier | Production disposition |
|---|---|---|
| mission | `CTaskForce+0x364` | D56 enum `0..=9`, exact name table |
| detached activity | task-force body `+0x4A8` | D56 enum `0..=4`, exact name table |
| home base | `CFleet+0xD8 -> CProvince+0xA4` | nullable D56 province id |
| repair target | `CTaskForce+0x4B0 -> CProvince+0xA4` | nullable D56 province id |
| repair progress | no authenticated D56 scalar | reserved/rejected |
| save repair lifecycle | save task-force `repair_mode`, `repair_split`, `repair_target`, `repair_parent`, `repair_child`, `repair_last_mission` | independently typed in replay; never relabeled as progress |
| save movement route | save task-force `path[]` and `movement_progress` | independently typed province path; not a convoy route |
| save retreat | save task-force `retreat=yes/no` | independently typed boolean; not a live D56 enum |
| combat-member retreat | D50 `FEX_STATE_ESCAPING` / `FEX_STATE_RUN_AWAY` | member lifecycle only |
| current in-combat activity | same-hour D49 combat epoch plus D50 state-2 member | pointer-free epoch list and exact activity key |
| convoy loss route | D90 pre-collapse full client EntityHandle `{id,type_tag=4713}` plus typed endpoints | exclusively owned by D90; D56 route slot rejected |

## Producer-to-replay closure

- The collector emits only authenticated D56 fields.
- The compact decoder rejects unknown flags, retired flags/status bits,
  nonzero reserved values, duplicate identities, invalid enums, and invalid
  province/count bounds.
- The recorder uses full-replace lifecycle semantics for successful non-delta
  frames. Failed authority projects the domain as `null`, so stale rows are not
  replayed.
- The world fold omits unproven field names and keeps the D49/D50 activity join
  hour-bound.
- Python portable admission requires the exact authority envelope, count
  equality, row identities, enum/name equality, pointer-free epoch grammar,
  and zero retired-fixture count.
- Native replay independently validates the D56 authority envelope and every
  row before joining by numeric task-force id.
- Native replay and the selected naval-unit view surface the exact save-tick
  movement, repair, detached-activity, and retreat carriers independently.

## Validation ownership

No subagent build, test, compiler, verifier, formatter, or generator was run.
The primary agent owns the central final Rust validation and production
acceptance battery.
