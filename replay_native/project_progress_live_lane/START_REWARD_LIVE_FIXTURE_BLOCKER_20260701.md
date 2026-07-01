# Domain 41 start/reward residual fixture blocker

Status language: this is only the residual project start/reward/history slice.
Domain 41 `projects_live` already has validated slices for facilities,
scientists, progress, and the completed bit. The full gameplay concept remains
open for exact start semantics, fired reward history, and richer completion
history.

## Fresh fixture attempted

- Live PID: `45880`.
- Build SHA-256:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Save fixture: current `autosave.hoi4`, parsed lane-locally with
  `save_oracle/parse_projects.py`.
- Save date: `1941.5.1.1`.
- Save coverage: 21,511 projects, 36 started, 35 completed, 189 fired
  reward-history rows, 18 facilities, 557 scientists.
- Live/save project join: 21,168 live projects matched by `(tag, project_id)`;
  no live matched projects missing from save; 343 save projects not present in
  the live sampled pool.

## Results

### Start

`CProjectStateMachine[0..0x280)` was scanned against save `start` labels for
all 21,168 matched live projects.

- Exact save-start u8 separators: `0`.
- Exact save-start u32 separators: `0`.
- `CProject+0x74 iterations != 0` is rejected as a start proxy: 6 mismatches,
  all save `start=false` with live iterations present.
- The current-state selector is not start: the only accepted semantic byte in
  this window is the already-known inverse completed byte.

### Completed Bit

The validated completed slice remains valid, but this does not close the full
project gameplay concept.

- Exact save-completed u8 separator: `CProjectStateMachine+0x10`.
- Encoding: `0` means completed, `1` means not completed.
- Completed mismatches: `0`.

### Reward History

The current-build RTTI reward class was found but rejected as a fired-history
row root.

- Current RTTI: `.?AVCPrototypeReward@NProject@@`.
- Current vtable RVA: `0x2705138`.
- Heap objects: `169`.
- Legacy/shifted reward RVAs rejected in this build: `0x26F5148`,
  `0x2705228`, `0x2705CD8`, `0x2705D78`; all had 0 heap objects.
- `CProgramStatus+0x58/+0x60/+0x68` as begin/end/cap vector is rejected:
  432 invalid headers, 0 decoded entries, 0 live entries versus 189 save
  history rows.
- `CProgramStatus+0x58` payload references to `CPrototypeReward` are rejected
  as fired reward-history rows: bounded scan of `0x4000` bytes per status found
  4,640 references, but only 9 unique reward objects, all reused and all
  observed across multiple tags; max tags per object was 243. Save history has
  189 event rows. This shape behaves like reusable reward definitions/options,
  not persisted fired history events.

## Exact blocker

Read-only memory plus the current autosave can reject the above false roots, but
cannot synthesize the missing semantic transition needed to pin exact start and
fired-history event fields. The next fixture requires a controlled in-game
action:

1. Pause on a country/facility with a never-started project available.
2. Save and record the project token, country tag, project id if visible from
   save parsing, game date/hour, and facility/scientist context.
3. Start that project, advance enough for live state to update, save again, and
   rerun the bounded probe. The accepted start field must flip from false to
   true before completion and must not duplicate iterations, owner-facility, the
   current-state selector, or `+0x10`.
4. For reward history, choose a known prototype reward option from a facility
   with `unread_prototype_rewards > 0`; save immediately before and after the
   click. The accepted root must add exactly one fired row and pin tag/project,
   option or prototype reward token, date/hour, province, progress, and scientist
   context.

Read-only tooling cannot perform these actions because the lane is constrained
to observational memory reads and must not mutate the game state, click UI, start
projects, choose reward options, or force saves from inside the game process.

## Artifacts

- Save oracle:
  `proofs/start_reward_live_probe_20260701_v2/save_oracle_autosave/projects.json`.
- Residual live probe:
  `proofs/start_reward_live_probe_20260701_v2/live_probe/residual_probe.json`.
- Residual proof:
  `proofs/start_reward_live_probe_20260701_v2/live_probe/residual_proof.json`.
- Reward container shape probe:
  `proofs/start_reward_live_probe_20260701_v2/live_probe/reward_container_shape_probe.json`.
