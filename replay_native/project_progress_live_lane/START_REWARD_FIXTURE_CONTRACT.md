# project_progress_live parked start/reward fixture contract

Scope: exact contract for the remaining project-progress backlog after the
completed-bit path. Keep these fields omitted/fail-closed until this contract
passes.

## Current parked blockers

### Exact `start`

Current bounded evidence:

- Probe artifact:
  `replay_native/runs/project_progress_live_residual_probe_20260701_v2/residual_proof.json`.
- 21,168 live projects matched the autosave oracle by `(tag, project_id)`.
- `CProjectStateMachine[0..0x280)` u8/u32 scan found:
  - save `completed`: one exact u8 separator at offset `0x10`;
  - save `start`: zero u8 separators and zero u32 separators.
- Proxy rejection:
  - `iterations != 0` mismatched save `start` 7 times.
  - Current-state selector matched save `completed` in this fixture but is not
    enough for save `start`.

Decision: no `start` field may be emitted from the current candidate set.

### Fired reward history

Current bounded evidence:

- Current-build RTTI vtable:
  - `.?AVCPrototypeReward@NProject@@`
  - vtable RVA `0x2705138`
  - 169 live objects.
- Rejected old/shifted vtable RVAs:
  - `0x26F5148`
  - `0x2705228`
  - `0x2705CD8`
  - `0x2705D78`
  - all 0 live objects on this build.
- Root narrowed:
  - `CProgramStatus + 0x58` has one-hop references to reward objects.
  - shallow bounded probe saw 141 one-hop reward refs.
- Still blocked:
  - the `+0x58` payload is not a simple begin/end/cap vector;
  - option token, prototype_reward token, date, project join, and context fields
    are not byte-pinned.

Decision: no fired reward-history row or field may be emitted until full field
pinning passes below.

## Required fixture inputs

### Shared fixture requirements

Use one deliberately controlled save/live fixture, then repeat against one
independent save/live fixture before production registration.

For each fixture, collect:

- melted save-oracle output from `save_oracle/parse_projects.py`;
- live read-only probe output from the same running `hoi4.exe` session;
- UI notes or screenshot/log notes for any manually selected reward option;
- exact game date/hour for the live sample and save;
- build hash and module base;
- tag table validation and `(tag, project_id)` matching summary.

Runtime production must remain DLL-only. Save/UI data is only development
validation truth.

### Minimum save rows

The save oracle must contain:

- at least 20,000 matched project rows or all project rows if the fixture is a
  smaller controlled scenario;
- at least one project in each class:
  - `start=false`, `completed=false`;
  - `start=true`, `completed=false`;
  - `start=true`, `completed=true`;
  - `start=false`, `completed=true` if the game can generate unlock/auto-complete
    rows in that fixture;
  - active and currently worked in a facility;
  - started but idle with no current facility assignment.
- at least 10 fired reward-history rows total;
- at least one reward-history row with non-`undefined` `option`;
- at least one reward-history row with non-`undefined` `prototype_reward`;
- at least one reward-history row with context `province >= 0`;
- at least one reward-history row with a scientist `character={...}` context;
- at least one project with multiple reward-history entries.

## Exact `start` pass criteria

A candidate start field may be accepted only if all are true:

1. Root is bounded from validated `CProject+0x28 -> CProjectStateMachine`.
2. Candidate offset, width, and encoding are declared before scoring.
3. Candidate is read from a bounded window, initially
   `CProjectStateMachine[0..0x280)`, or from a specifically justified embedded
   state subobject.
4. Candidate maps exactly to save `start` for every matched `(tag, project_id)`
   row in the fixture.
5. Candidate has both true and false values in the fixture.
6. Candidate remains stable over at least two live hourly samples where no
   relevant project transition occurs.
7. Candidate flips correctly in a controlled transition:
   - before starting a never-started project: `start=false`;
   - after starting and saving: `start=true`;
   - completing that same project must not be the first moment it becomes true.
8. Candidate does not merely duplicate:
   - `CProject+0x74 iterations`;
   - owner facility ref at `CProject+0x60`;
   - current-state selector at `CProjectStateMachine+0x08`;
   - the validated completed inverse byte at `+0x10`.

Fail/reject rules:

- Reject any candidate with one or more save-label mismatches.
- Reject any candidate that only separates proxy labels, not save `start`.
- Reject any candidate with no false or no true coverage.
- Reject any candidate that is actually the completed byte or current-state enum.
- Reject any candidate requiring save data in the runtime hot loop.
- Reject any candidate whose proof lacks exact `(tag, project_id)` matching.

## Fired reward-history pass criteria

A reward-history row may be accepted only if all are true:

1. Root is bounded from an already validated production object path:
   - preferred current candidate: `CCountry -> CProgramStatus + 0x58`;
   - any alternate root must list exact parent object, offset, container shape,
     row count, and traversal rules.
2. Container decode is deterministic:
   - no fixed shallow one-hop scan as production traversal;
   - no duplicate reward-object rows from repeated incidental references;
   - explicit cycle/depth/row-count bounds.
3. Live row count matches save oracle:
   - exact total reward-history row count;
   - exact per-tag count;
   - exact per-project count after joining to `CProject` id or static-def handle.
4. Each emitted row has pinned fields:
   - country tag or tag_db_id;
   - project id or static-def handle;
   - option token handle or resolved token;
   - prototype_reward token handle or resolved token;
   - date/game-hour encoding;
   - context province;
   - context progress raw/scale or explicit computed badge;
   - context scientist char_id where present.
5. Field comparison is per-row, not only multiset:
   - a row must join to the same save event by tag, project, date/order, and
     option/prototype identity;
   - token handles must be resolved through a stable registry or documented as
     handles with exact token-name proof.
6. Controlled UI transition proof:
   - before reward option selection: no new fired row for that option;
   - after selecting a known option: exactly one new row;
   - `unread_prototype_rewards` behavior is recorded separately and must not be
     used as a substitute for fired history.

Fail/reject rules:

- Reject vtable-only proof.
- Reject object-byte fields that cannot be joined to project and tag.
- Reject count-only or multiset-only field matches.
- Reject container traversals that produce duplicates or depend on incidental
  pointers inside a fixed byte window.
- Reject token fields unless option and prototype_reward can be resolved or
  byte-pinned as stable handles.
- Reject context fields unless province/progress/scientist match save/UI truth.
- Reject any runtime plan that depends on save-oracle or UI state.

## Required parked output before retrying registration

Produce a lane-local proof artifact containing:

- `start_semantics.accepted` true or a conclusive rejection with candidate list;
- `reward_history.accepted` true or a conclusive rejection with candidate list;
- save/live fixture paths;
- build hash;
- matched project count;
- reward row total/per-tag/per-project comparison;
- exact accepted offsets and widths;
- exact rejected offsets/RVAs/container shapes;
- proof commands and results.

Until that artifact passes, shared domain 41 must continue omitting start and
reward-history fields.
