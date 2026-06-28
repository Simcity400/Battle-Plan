# Battle Plan / Native Replay App

Private, read-only tooling for the owner's Hearts of Iron IV multiplayer group.

The project started as an in-game stats mod plus browser dashboards. Those legacy
surfaces have been removed. The active target is a native Rust/wgpu replay
application that lets a viewer inspect a recorded HOI4 session like the real game
UI, without any ability to mutate the game.

Runtime replay capture is moving to a single authoritative live source: an
in-process collector DLL inside the host's `hoi4.exe`, with an external recorder
writing `.battleplan` containers. Saves are development fixtures and validation
artifacts only; they are not required to record or replay a live session.

## Progress

Current progress and remaining work are tracked only in
[replay_ui/MASTER_PROGRESS.md](replay_ui/MASTER_PROGRESS.md). Keep status updates
there, not in this README.

## Run The Native Replay App

```powershell
cargo run --manifest-path replay_native/Cargo.toml --release -p app --bin replay_app
```

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

- `replay_ui/MASTER_PROGRESS.md` - product parity status and remaining work
- `IN_PROCESS_COLLECTOR_PLAN.md` - live collector implementation plan
- `RECORDING_LAYER_COMPLETION_PLAN.md` - working plan for completing runtime recording coverage
- `coverage_audit/memory_manifest.json` - data completeness row status
- `coverage_audit/COVERAGE_REPORT.md` - readable data coverage report
- `replay_ui/BINDING_MAP_COVERAGE.md` - UI binding inventory
- `AGENTS.md` - current agent workflow rules

## Repo Map

| Path | Purpose |
| --- | --- |
| `replay_native/` | Native Rust/wgpu replay app and rendering/runtime crates. |
| `live_capture/` | Existing Python recorder GUI/CLI, schema assembly, and `.battleplan` writer; target consumer for collector frames. |
| `memory_reader/` | Read-only process-memory readers and live extraction modules, now primarily research/validation inputs for the collector. |
| `coverage_audit/` | Completeness manifest, report, and checker. |
| `game_defs/` | Static HOI4 definition parsers. |
| `replay/` | Snapshot schema, store, diff engine, and tests. |
| `replay_ui/` | Master progress doc, binding maps, interaction inventory, and replay specs; no browser replay runtime remains here. |
| `save_oracle/` | Development-only save parsing/oracle tooling used for fixtures, validation, and discovery. |

## Safety Rules

- Runtime capture may inject a read-only collector DLL, but game state must
  remain observational. Never use `WriteProcessMemory` or mutating game calls.
- The live game should stay paused during external scanning unless running an
  explicit recorder/collector cadence proof.
- Do not commit personal paths, cloud-sync paths, raw dumps, melted saves, or
  generated game art.
- Local probe outputs belong in ignored scratch paths, not in tracked source.
- Do not use the old local Codex CLI review workflow in Codex-native sessions;
  use normal native subagents as work partners.
