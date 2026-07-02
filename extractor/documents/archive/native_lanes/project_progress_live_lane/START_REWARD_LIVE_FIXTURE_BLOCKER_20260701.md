# Domain 41 start/reward residual fixture blocker

Status language: this is only the residual project start/reward/history slice.
Domain 41 `projects_live` already has validated slices for facilities,
scientists, progress, and the completed bit. The full gameplay concept remains
open for exact start semantics, fired reward history, reward lifecycle events,
and lockstep breakthrough semantic names.

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

## Continuation proof: 2026-07-01

This continuation tried to close the residuals with a live projects-only
collector run against PID `45880` and the release DLL
`extractor/native/target/release/collector.dll`.

### Save/live drift for breakthrough semantics

Current autosave was reparsed because the original save oracle no longer
matched live time.

Command:

```powershell
python save_oracle/parse_projects.py "$env:USERPROFILE\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\save games\autosave.hoi4" --rakaly "$env:USERPROFILE\Downloads\rakaly_cli\rakaly-0.8.17-x86_64-pc-windows-msvc\rakaly.exe" --out-dir extractor/native/project_progress_live_lane/proofs/start_reward_live_probe_20260701_v3_current_autosave/save_oracle
```

Result:

- Save date: `1942.11.1.1`.
- Parsed projects: `21,511`; started `58`; completed `48`.
- Facilities: `23`; scientists: `600`.

Breakthrough probe command:

```powershell
python extractor/native/project_progress_live_lane/tools/breakthrough_state_probe.py --pid 45880 --save-projects-json extractor/native/project_progress_live_lane/proofs/start_reward_live_probe_20260701_v3_current_autosave/save_oracle/projects.json --out extractor/native/project_progress_live_lane/proofs/start_reward_live_probe_20260701_v3_current_autosave/live_probe/breakthrough_state_probe.json
```

Result:

- Live date at the same point was `1942.11.7.22`, so the save was not lockstep.
- Live rows: `1,728`; parse failures: `0`; status rows: `432`.
- Handles observed exactly once per status row: `22214`, `22215`, `22216`,
  `22217`.
- Best mapping: `22214=nuclear`, `22215=naval`, `22216=air`, `22217=land`.
- Exact save/live rows: `1,522`; mismatches: `206`; live tags missing from
  save: `0`.

This validates the live row shape and blocks semantic naming on a same-hour
paused save/live fixture. The accepted row shape remains:
`tag_db_id:u32`, `specialization_handle:u32`, `points_raw:u32`, `reserved:u32`
from `CCountry -> CProgramStatus+0x50 -> CBreakthroughProgress`.

### JSON session proof

The normal repo-length output path hit Windows path limits in
`ReplayRecorder.__init__` before capture:

```text
FileNotFoundError: [WinError 3] ... projects_progress_rewards_full_residuals_20260701_abs.battleplan.part
```

Using a short proof root avoided that wrapper path issue.

Command:

```powershell
python -m live_capture.collector_session --pid 45880 --dll extractor/native/target/release/collector.dll --out-root C:\p41b --store-root C:\p41b_store --game-id p41b --duration-s 90 --sample-ms 500 --dwell-s 1.0 --worker-bursts 4 --burst-interval-s 2.0 --until-game-hours 2 --max-wall-s 180 --preflight-special-projects --allow-special-projects-cold-scan --scheduled-domain projects_live --lightweight-root-frame
python -m live_capture.collector_proof_gate C:\p41b\20260701T083740Z_pid45880\session_manifest.json --require-scheduled-domain projects_live --require-projects-live-payload --require-projects-live-links --min-game-hours 2
```

Result:

- Gate `ok=true`.
- Domains seen: `clock`, `projects_live`.
- Scheduled `projects_live` frames: `3`.
- Game hours advanced: `2`; hourly requirement met: `true`; skipped hours:
  `0`.
- Replay verified: `true`.
- Special project bridge rows supplied by Python preflight: `21,535`.
- Special project rows: `45`; facility rows: `72`; active facility/project
  links: `45`; facility/scientist links: `69`; worked-facility links: `45`.

This proves the live project payload is reachable with preflight assistance, but
it is not a production completion proof because it is JSON-session backed and
uses a Python-supplied bridge.

### Native compact stream attempts

No-bridge compact command:

```powershell
cargo run --manifest-path extractor/native/recorder_host/Cargo.toml -- --pid 45880 --dll extractor/native/target/release/collector.dll --out-dir C:\p41c --duration-s 90 --sample-ms 500 --min-game-hours 2 --scheduled-domain projects_live --stream-format compact --extraction-perf-gate --lightweight-root-frame --special-projects-scan
```

Result:

- `collector_stream.bin` and `native_session.battleplan` were written.
- Manifest: `projects_root_source="unresolved"`;
  `special_projects_bridge_rows=0`; `source_unavailable_seen=true`;
  `decodable=false`.
- Parse errors were repeated `projects has unknown source_kind 0`.

Bridge compact command:

```powershell
cargo run --manifest-path extractor/native/recorder_host/Cargo.toml -- --pid 45880 --dll extractor/native/target/release/collector.dll --out-dir C:\p41e --duration-s 20 --sample-ms 50 --min-game-hours 2 --scheduled-domain projects_live --stream-format compact --extraction-perf-gate --lightweight-root-frame --special-projects-bridge-file C:\p41b\special_projects_bridge.txt
cargo run --manifest-path extractor/native/lanes/compact_replay_verification_gates/Cargo.toml -- --run-dir C:\p41e --require-domain projects_live --min-game-hours 2 --write-report C:\p41e\compact_replay_gate_projects_live.json
```

Result:

- Compact stream: `C:\p41e\collector_stream.bin`, `34,133,253` bytes.
- Replay: `C:\p41e\native_session.battleplan`, `2,360,635` bytes.
- Manifest: decodable `true`; ordered `true`; domain coverage `ok=true`.
- Hours: first raw `60822761`, last raw `60822793`, unique hours `33`,
  skipped hours `0`, hourly requirement met `true`.
- Failures: partial `false`; source unavailable `false`; bridge unavailable
  `false`; worker panic `false`.
- `projects_live` units: project rows `495`; facility rows `792`;
  state rows `709,863`; completed-state rows `1,782`; bridge rows supplied
  `710,655`.
- Compact replay gate verified the `.battleplan` container and stream bytes, but
  failed production acceptance only on:
  `special_projects_bridge_rows is 21535, expected 0 for DLL-only proof` and
  `projects_root_source="control_bridge" ... not merge-clean DLL-only root evidence`.

This is a compact-stream/replay/cadence validated slice, not a production
completion proof. The exact production blocker is DLL-only special-project root
discovery for the compact recorder.

### DLL-only root discovery resolved

The control-bridge blocker above was reduced by rerunning the native compact
recorder without `--lightweight-root-frame` and without any
`--special-projects-bridge-file`. The prior no-bridge run failed because
lightweight root frames skipped country identity, which is required for
`CProgramStatus` traversal. The successful run used the existing in-DLL country
path.

Command:

```powershell
cargo run --manifest-path extractor/native/recorder_host/Cargo.toml -- --pid 45880 --dll extractor/native/target/release/collector.dll --out-dir C:\p41f --duration-s 20 --sample-ms 50 --min-game-hours 2 --scheduled-domain projects_live --stream-format compact --extraction-perf-gate
cargo run --manifest-path extractor/native/lanes/compact_replay_verification_gates/Cargo.toml -- --run-dir C:\p41f --require-domain projects_live --min-game-hours 2 --write-report C:\p41f\compact_replay_gate_projects_live.json
```

Result:

- Compact replay gate: `passed=true`.
- `special_projects_bridge_rows=0`.
- `projects_root_source="dll_project_cache_progress"`.
- `projects_root_discovery_debt="none; projects_live includes anchored DLL project state/progress raw rows from CProject+0x28 and breakthrough rows from CProgramStatus+0x50"`.
- Compact stream: `C:\p41f\collector_stream.bin`, `29,998,197` bytes.
- Replay: `C:\p41f\native_session.battleplan`, `2,295,101` bytes.
- Manifest: decodable `true`; ordered `true`; domain coverage `ok=true`.
- Hours: first raw `60823303`, last raw `60823337`, unique hours `35`,
  skipped hours `0`, hourly requirement met `true`.
- Failures: partial `false`; source unavailable `false`; bridge unavailable
  `false`; worker panic `false`.
- `projects_live` units: project rows `508`; facility rows `840`;
  scientist rows `5,145`; state rows `608,825`; completed-state rows `1,977`;
  breakthrough rows `24,850`.

Validated root/path:

- `module_base + 0x3316E90` -> `CGameState`.
- `module_base + 0x33189C8` -> country database.
- `CGameState + 0x310` -> country table.
- `CCountry` vtable: `module_base + 0x27D0E90`; tag id at `CCountry+0x08`.
- `CCountry + 0xFA8` -> `CProgramStatus`.
- `CProgramStatus` vtables: `module_base + 0x295AC88` and
  `module_base + 0x295ACA8`.
- Project roots:
  `CProgramStatus+0x18` project pool,
  `CProgramStatus+0x20` programs/facility roots, and
  `CProject+0x28` state machine.
- Breakthrough roots:
  `CProgramStatus+0x50` embedded `CBreakthroughProgress`;
  vtable `module_base + 0x2A152E0`;
  tree node specialization handle at `node+0x1C`;
  points at `node+0x20`.

Relevant in-DLL functions:

- `CollectorState::resolve`.
- `CountryIdentityCache::from_gamestate_country_table`.
- `SpecialProjectsCache::from_country_project_pools`.
- `program_status_from_country_object`.
- `read_project_breakthrough_rows`.
- `read_project_breakthrough_progress_rows`.

This removes the DLL-only special-project/CProgramStatus root discovery blocker.

### Lifecycle event proof

Command:

```powershell
cargo run --manifest-path extractor/native/project_progress_live_lane/Cargo.toml --bin project_progress_lifecycle_stream_proof -- C:\p41f\collector_stream.bin C:\p41f\lifecycle_stream_proof.json C:\p41f\lifecycle_events.bin
```

Result:

- Rust decoded: `true`; event gate `ok=true`; parse errors `[]`.
- Project data records: `35`; transition pairs: `34`.
- State rows per frame: `17,395`.
- Breakthrough rows per frame: `710`.
- Lifecycle events: total `50`; completion changed `1`; breakthrough delta
  `49`.
- First completion event:
  `{kind:1, key_a:2320, key_b:22154, before:0, after:1, delta:1}`.
- First breakthrough delta:
  `{kind:2, key_a:1, key_b:22214, before:11160, after:11172, delta:12}`.

Completion transition proof is no longer fixture-blocked. Reward events remain
blocked until the fired reward-history root is pinned by the before/after
reward-option fixture. Breakthrough compact state and deltas are captured;
semantic specialization names remain blocked on a same-hour save/live fixture.
