# project_progress_live completed-bit registration patch plan

Scope: registration-only shared hook plan for domain 41 `projects_live`.
Do not add `start` or fired reward-history fields in this patch.

## Accepted path

- Source: `CProject + 0x28 -> CProjectStateMachine`.
- Validated byte: `CProjectStateMachine + 0x10`.
- Encoding: inverse completed flag. `0` means save `completed=yes`; `1` means save `completed=no`.
- Evidence artifact:
  `extractor/native/runs/project_progress_live_residual_probe_20260701_v2/residual_proof.json`
  has `completed_boolean_validated_for_schema_expansion=true`,
  `save_completed_u8_separators=1`, `completed_proxy_mismatches=0`,
  `sampled_projects=21168`, and `save_matched_projects=21168`.
- Lane-local implementation:
  `extractor/native/project_progress_live_lane/src/lib.rs` uses
  `PROJECT_STATE_MACHINE_OFF_NOT_COMPLETED_FLAG = 0x10`,
  `PROJECT_STATE_FLAG_COMPLETED = 1 << 0`, and emits `state_flags` in the last
  u32 of the 48-byte project-state row.

## Wire shape

- Domain id remains `41`.
- Domain name remains `projects_live`.
- No new compact domain id.
- No new record type.
- Project-state row length remains 48 bytes.
- The final u32 at row offset `+44`, currently reserved/zero in shared code, becomes
  `state_flags`.
- Known row flag mask:
  - `0x00000001`: `completed`.
- `state_flags == 0` in an emitted project-state row means the project is validly
  not completed. It is not an unknown marker; unknown/invalidation must omit the
  project-state row fail-closed.

## Shared collector patch points

File: `extractor/native/collector/src/lib.rs`.

1. Add constants near the existing project state constants:
   - `PROJECT_STATE_MACHINE_OFF_NOT_COMPLETED_FLAG: usize = 0x10`
   - `COMPACT_PROJECT_STATE_ROW_FLAG_COMPLETED: u32 = 1`
   - `COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2: u32 = 4`

2. Extend `ProjectStateRow` with:
   - `state_flags: u32`

3. In `read_project_state_row`, after validating the state-machine vtable and
   before reading embedded state objects:
   - read `u8` at `state_machine_ptr + 0x10`;
   - accept only `0` or `1`;
   - on any unreadable or non-0/1 value, return `None` so the row is omitted;
   - set `state_flags = COMPACT_PROJECT_STATE_ROW_FLAG_COMPLETED` when the byte
     is `0`, otherwise `0`.

4. In `compact_projects_data_payload`:
   - write `COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2` instead of
     `COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1` once the completed bit is emitted;
   - keep the same progress counters and row sizes;
   - write `row.state_flags` at the final u32 of each project-state row instead
     of `0u32`.

5. Update only tests that construct/inspect project-state rows:
   - add `state_flags` to `ProjectStateRow` literals;
   - assert project-state row `+44` carries `1` for a completed fixture row;
   - add one negative test for unknown state flags if collector-side byte packing
     helper coverage exists locally.

## Shared recorder patch points

File: `extractor/native/recorder_host/src/main.rs`.

1. Add constants near the existing project constants:
   - `COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2: u32 = 4`
   - `COMPACT_PROJECT_STATE_ROW_FLAG_COMPLETED: u32 = 1`

2. In `decode_compact_projects_data_payload`:
   - accept header extension `4` in the same branch as extension `3`;
   - keep `COMPACT_PROJECT_PROGRESS_COUNTERS_LEN` unchanged;
   - for extension `3`, continue treating row offset `+44` as reserved and
     require zero;
   - for extension `4`, read row offset `+44` as `state_flags` and reject
     `state_flags & !COMPACT_PROJECT_STATE_ROW_FLAG_COMPLETED != 0`.

3. Add a decoded metric:
   - `project_completed_state_rows`: count project-state rows with bit 0 set.

4. Update `compact_summary_decodes_projects_data_record`:
   - emit extension `4`;
   - set the sample project-state row final u32 to `1`;
   - assert `project_completed_state_rows == 1`.

5. Add one recorder gate test:
   - extension `4` with state row `state_flags = 0x80000000` must produce a
     parse error containing `unknown state_flags` or equivalent.

## Registration proof requirements after hook lands

Run a fresh DLL-only production capture after the shared hook is serialized:

1. `cargo test -p collector`
2. `cargo test -p recorder_host`
3. domain-41-only live compact capture at speed five/current equivalent
4. `.battleplan` verify with `pack_replay -- verify`
5. Manifest acceptance:
   - `domain_id = 41`
   - compact stream decodable
   - `project_state_rows > 0`
   - `project_completed_state_rows > 0`
   - `partial_data_seen = false`
   - `source_unavailable_seen = false`
   - `bridge_unavailable_seen = false`
   - `worker_panic_seen = false`
   - hourly requirement met
   - `skipped_game_hours = 0`

## Explicit exclusions

- Do not add save `start`.
- Do not add fired reward-history option/prototype/context/date rows.
- Do not consume save-oracle data in the runtime hot loop.
- Do not allocate a new domain id.
- Do not create a second collector/DLL/coordinator.
