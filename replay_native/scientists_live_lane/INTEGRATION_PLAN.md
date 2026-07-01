# scientists_live lane-local integration plan

Status: lane-local extraction, `projects_live` domain-41 scientist payload encoding,
decoder, and gate tests are ready. This lane turn did not edit shared collector or
recorder entrypoints; the current worktree already contains a shared `projects_live`
scientist route, and the proof below verifies that route against the running build.

## Current build proof (2026-07-01)

Build:

```powershell
cargo build -p collector -p recorder_host --release --manifest-path replay_native\Cargo.toml
```

Result: passed.

Live CGameState candidate probe:

```powershell
python replay_native\scientists_live_lane\cgame_roster_ref_probe.py --pid 45880
```

Result:

```text
sha256=870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad
base=0x7ff6aaff0000
gamestate=0x22ba5f48ad0
offset=+0x520 addr=0x22ba5f48ff0 u32=29 u64=0x1d heap_ptr=False direct_vectors=0 deref_vectors=0 ui_tag_vectors=1 ui_tag_char_ids=[2358]
offset=+0x524 addr=0x22ba5f48ff4 u32=0 u64=0x100000000 heap_ptr=False direct_vectors=0 deref_vectors=0 ui_tag_vectors=0 ui_tag_char_ids=[]
exact_country_ff0_vectors=39
exact_country_ff0_char_ids=121
verdict=reject_cgame_0x520_0x524_as_roster_root; use CCountry+0xFF0 -> roster+0xA0 vector instead
```

Compact proof command:

```powershell
replay_native\target\release\collector_record.exe `
  --pid 45880 `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay_native\runs\compact_scientists_live_20260701_v3 `
  --duration-s 30 `
  --sample-ms 0 `
  --min-game-hours 10 `
  --scheduled-domain projects_live `
  --stream-format compact `
  --extraction-perf-gate
```

Manifest: `replay_native\runs\compact_scientists_live_20260701_v3\native_session_manifest.json`.

Result:

- `game_hours_advanced = 48`, `unique_game_hour_count = 49`, `skipped_game_hours = 0`.
- `compact_stream.decodable = true`, `ordered = true`, `domain_coverage.ok = true`.
- `partial_data_seen = false`, `source_unavailable_seen = false`,
  `bridge_unavailable_seen = false`, `worker_panic_seen = false`.
- `special_projects_bridge_rows = 0`, `special_projects_scan = false`.
- `projects_root_source = "dll_project_cache_progress"`,
  `projects_root_discovery_debt = "none; projects_live includes anchored DLL project state/progress raw rows from CProject+0x28 and breakthrough rows from CProgramStatus+0x50"`.
- Domain id `41`, `project_scientist_rows = 5929`,
  `project_observed_scientist_rows = 5929`.
- `.battleplan` produced as
  `replay_native\runs\compact_scientists_live_20260701_v3\native_session.battleplan`.

Replay verification gate:

```powershell
cargo run --manifest-path replay_native\lanes\compact_replay_verification_gates\Cargo.toml -- `
  --run-dir replay_native\runs\compact_scientists_live_20260701_v3 `
  --min-game-hours 10 `
  --write-report replay_native\runs\compact_scientists_live_20260701_v3\compact_replay_gate_report.json
```

Result: `passed = true`, `errors = []`, replay stream bytes `41903722`
matched the raw `collector_stream.bin`.

## Lane-local proof

Command:

```powershell
cargo test --manifest-path replay_native\scientists_live_lane\Cargo.toml
```

Result:

```text
running 5 tests
5 passed; 0 failed
```

Coverage:

- Extracts rows from the validated root:
  `CCountry+0xFF0 -> roster+0xA0 -> vector count +0x0C -> wrapper+0x08 -> CCharacter`.
- Validates `CCharacter` vtable `base+0x2967E30`, type `73`, `char_id +0x0C`,
  `CScientist* +0xC0`, and `CScientist` vtable `base+0x2967E88`.
- Uses build `870ebf` skill records at `CScientist+0x110`, stride `0x20`,
  header base `0x56C6`, level at `+0x18`.
- Rejects stale skill header base `0x56F7` as partial data.
- Round-trips compact scientist rows and rejects duplicate `char_id` rows.
- Gates partial data and observed/emitted mismatches.

## Shared collector patch plan

1. Move `scientists_live_lane::ScientistProfile`, constants, extractor, row type,
   compact encoder, decoder/gate helpers into the shared native collector/host
   modules or vendor the lane-local module as an internal collector module.
2. In `SpecialProjectsCache::from_country_project_pools`, populate scientist pins
   from the rooted country roster before or independently of the program-status
   gate. The root is country-owned, not program-status-owned.
3. Replace the existing project scientist skill header constant with the build
   `870ebf` value `0x56C6`.
4. Emit existing `projects_live` scientist compact rows using the current row
   layout: `char_id`, flags, `trait_count`, four specialization levels, reserved,
   then token/display strings.
5. Keep failure semantics fail-closed:
   if observed pins exceed emitted rows, set `COMPACT_STATUS_PARTIAL_DATA`;
   if no rooted source resolves, set `COMPACT_STATUS_SOURCE_UNAVAILABLE`.
6. Add collector tests equivalent to the lane-local fixture:
   valid roster emits one scientist row; stale skill header produces partial data;
   compact payload round-trips; duplicate char IDs are rejected by host decode.
7. Update recorder manifest wording only after the compact proof passes without
   partial/source/worker/bridge failures.

## Rebase-readiness against current projects_live progress integration

Current shared state observed after the `project_progress_live` integration:

- `projects_live` uses `COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1 = 3`.
- The fixed header remains `COMPACT_PROJECT_HEADER_LEN = 64`.
- Root counters extend the header to `88`; root-reason counters extend it to `136`;
  progress counters extend it to `152`.
- Progress counters are:
  `project_state_count @ +136`, `breakthrough_count @ +140`,
  `observed_project_state_rows @ +144`, `observed_breakthrough_rows @ +148`.
- Fixed row order is:
  facilities, projects, scientists, project states, breakthroughs.
- Row lengths are:
  facility `32`, project `24`, scientist `32`, project state `48`,
  breakthrough `16`.
- Variable string order remains:
  facility building strings first, then scientist token/display string pairs.

Exact rebase patch plan when the scientist integration slot opens:

1. Preserve `COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1`; do not introduce a new
   scientist-specific project-header extension.
2. Keep `scientist_count` at existing header offset `+28` and
   `observed_scientist_rows` at existing header offset `+40`.
3. Keep scientist fixed rows between project rows and project-state rows so the
   current host decoder cursor order remains valid.
4. Insert rooted scientist extraction into `SpecialProjectsCache` only by
   populating `scientists: Vec<ProjectScientistPin>`; do not change
   `project_states`, `breakthrough`, or their counters.
5. Update `row_count` by the existing `scientist_count` field only. Do not add
   scientist rows to project-state or breakthrough counts.
6. Preserve the current host decoder validations for duplicate `char_id`,
   `trait_count`, skill ranges, and token/display string flag agreement.
7. Add a host test with all five row groups present:
   one facility, one project, one scientist, one project-state row, one
   breakthrough row. Assert row_count = 5 and all five `project_*_rows` units
   increment independently.
8. Add a collector test equivalent to the lane-local valid-root fixture but with
   non-empty `project_states` and `breakthrough` in the same
   `CompactProjectsRead`, proving the scientist patch does not shift progress
   rows or strings.

Do not edit shared `collector/src/lib.rs` or `recorder_host/src/main.rs` for
these steps until the orchestrator grants the `scientists_live` integration slot.

## Required production proof after integration slot

Run a release compact proof with no bridge and no heap-scan flag:

```powershell
cargo build -p collector -p recorder_host --release --manifest-path replay_native\Cargo.toml
replay_native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay_native\runs\compact_scientists_live_<date> `
  --duration-s 30 `
  --sample-ms 0 `
  --min-game-hours 10 `
  --scheduled-domain projects_live `
  --stream-format compact `
  --extraction-perf-gate
```

Acceptance for production completion:

- `compact_stream.decodable = true`
- `compact_stream.partial_data_seen = false`
- `compact_stream.source_unavailable_seen = false`
- `compact_stream.worker_panic_seen = false`
- `compact_stream.bridge_unavailable_seen = false`
- `project_scientist_rows > 0`
- `project_observed_scientist_rows == project_scientist_rows`
- `.battleplan` produced and host gate passes
- hourly-or-better cadence, speed-five/current-equivalent
- `skipped_game_hours = 0`
