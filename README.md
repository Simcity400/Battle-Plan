# Battle Plan / Replay Recorder Foundation

Private, read-only tooling for the owner's Hearts of Iron IV multiplayer group.

The project started as an in-game stats mod plus browser dashboards. Those legacy
surfaces have been removed. The previous native replay app, map renderer, and UI
scaffolding have also been reset so the replay surface can be rebuilt from the
real installed content instead of inherited mockups.

Runtime replay capture is moving to a single authoritative live source:
`hoi4.exe` -> in-process collector DLL -> compact raw/event/snapshot stream ->
`.battleplan` container. Saves are development fixtures and validation artifacts
only; they are not required to record or replay a live session.

Python is not a production live-extraction dependency. It may attach the DLL in
development, run proof gates, validate manifests/replay files, and perform
bounded one-time discovery, but it must not be part of the hourly/sub-hourly hot
recording loop. Any segment that still needs Python preflight/root discovery is
temporary debt until that discovery moves into the DLL/Rust host or is proven to
be one-time, bounded, cached, and outside recording.

There is currently no replay UI/map app in-tree. The kept code is the
collector/recorder/data foundation the replacement surface will consume.

## Useful Verification Commands

```powershell
cargo test --manifest-path replay_native/Cargo.toml --workspace --no-fail-fast
python -m pytest save_oracle game_defs replay live_capture coverage_audit
python coverage_audit/check_coverage.py
python live_capture/recorder/recorder_main.py --smoke
```

Some Python suites are pytest-style; do not rely on a unittest-only run as a
full-project signal.

## Source Of Truth

- `IN_PROCESS_COLLECTOR_PLAN.md` - live collector implementation plan
- `replay_native/MASTER_EXTRACTION_CONTRACT.md` - final recording-layer completion contract
- `RECORDING_LAYER_COMPLETION_PLAN.md` - working plan for completing runtime recording coverage
- `replay/HOI4REPLAY_CONTAINER_SPEC.md` - portable recording container format
- `coverage_audit/memory_manifest.json` - data completeness row status
- `coverage_audit/COVERAGE_REPORT.md` - readable data coverage report
- `AGENTS.md` - current agent workflow rules

## Repo Map

| Path | Purpose |
| --- | --- |
| `replay_native/` | Native collector DLL, Rust recorder host, extraction lanes, map viewer, terrain capture, and Rust snapshot/container support crates. |
| `live_capture/` | Python development/proof harness, schema assembly, proof gates, recorder UI/CLI experiments, and replay validation. Not production hot-loop extraction. |
| `memory_reader/` | Read-only process-memory readers and live extraction modules, now primarily research/validation inputs for the collector. |
| `coverage_audit/` | Completeness manifest, report, and checker. |
| `game_defs/` | Static HOI4 definition parsers. |
| `map_pipeline/` | Legacy static map JSON assets kept for existing recorder/research callers; new map generation lives in Rust. |
| `replay/` | Snapshot schema, store, diff engine, container spec, and tests. |
| `save_oracle/` | Development-only save parsing/oracle tooling used for fixtures, validation, and discovery. |

## Safety Rules

- Runtime capture may inject a read-only collector DLL, but game state must
  remain observational. Never use `WriteProcessMemory` or mutating game calls.
- The live game should stay paused during external scanning unless running an
  explicit recorder/collector cadence proof.
- A recording segment is not complete until the DLL/Rust path captures it at
  hourly-or-better cadence, with no skipped game hours, at speed five or a
  measured equivalent.
- Do not commit personal paths, cloud-sync paths, raw dumps, melted saves, or
  generated game art.
- Local probe outputs belong in ignored scratch paths, not in tracked source.
- Do not use the old local Codex CLI review workflow in Codex-native sessions;
  use normal native subagents as work partners.
