# Battle Plan / Native Replay App

Private, read-only tooling for the owner's Hearts of Iron IV multiplayer group.

The project started as an in-game stats mod plus browser dashboards. Those legacy
surfaces have been removed. The active target is a native Rust/wgpu replay
application that lets a viewer inspect a recorded HOI4 session like the real game
UI, without any ability to mutate the game.

## Current Status

The narrow engineering definition of done is met: the native app builds, 114 real
game views are registered and menu-reachable, no-fabrication guardrails are in
place, the map/weather/rendering stack is wired, the `.battleplan` container
exists, and the coverage checker passes.

Product parity is not complete. The app is not yet a fully proven 1-to-1
inspector for every real HOI4 UI interaction and visual surface. Current data
coverage is 273 manifest rows: 162 closed, 47 closeable/provable rows still open
including 26 wartime/fixture-gated rows, 6 live/static rows, and 15
computed/visual rows, plus 2 permanent walls and 62 excluded rows.

The canonical remaining-work checklist is:

- [replay_ui/REPLAY_PARITY_COMPLETION_CHECKLIST.md](replay_ui/REPLAY_PARITY_COMPLETION_CHECKLIST.md)

The current high-level blockers are:

- live unpaused recorder proof at the intended multiplayer speed
- full read-only interaction parity for every registered view
- tooltip/localization parity beyond the first vertical slice
- side-by-side game-vs-app screenshot/interaction comparison
- remaining asset-sprite/counter chrome work, especially portraits, insignia,
  and full unit-counter chrome/art parity
- remaining map overlay parity for labels, routes, fronts, battle plans, trade,
  weather phenomenon overlays, and exact Air/Naval per-province visual parity
- data gaps tracked in `coverage_audit/memory_manifest.json`, especially
  `all_playthrough_data`, `countries.*.logistics`,
  `countries.*.strategic_navy`, `countries.*.upgrades`, computed values, and
  visual-semantics rows
- user-provided mid-war/eventful paused save coverage for wartime-only rows

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

- `replay_ui/REPLAY_PARITY_COMPLETION_CHECKLIST.md` - product parity checklist
- `replay_ui/SESSION_HANDOFF.md` - current orchestration state and known caveats
- `coverage_audit/memory_manifest.json` - data completeness row status
- `coverage_audit/COVERAGE_REPORT.md` - readable data coverage report
- `replay_ui/BINDING_MAP_COVERAGE.md` - UI binding inventory
- `AGENTS.md` - current agent workflow rules

## Repo Map

| Path | Purpose |
| --- | --- |
| `replay_native/` | Native Rust/wgpu replay app and rendering/runtime crates. |
| `live_capture/` | Python live-memory capture, recorder GUI/CLI, and `.battleplan` writer. |
| `memory_reader/` | Read-only process-memory readers and live extraction modules. |
| `coverage_audit/` | Completeness manifest, report, and checker. |
| `game_defs/` | Static HOI4 definition parsers. |
| `replay/` | Snapshot schema, store, diff engine, and tests. |
| `replay_ui/` | Product parity docs, binding maps, handoff state, and replay specs; no browser replay runtime remains here. |
| `save_oracle/` | Save parsing/oracle tooling used for validation and discovery. |

## Safety Rules

- All game access is read-only. Never use `WriteProcessMemory`.
- The live game should stay paused during scanning unless running an explicit
  recorder cadence proof.
- Do not commit personal paths, cloud-sync paths, raw dumps, melted saves, or
  generated game art.
- Local probe outputs belong in ignored scratch paths, not in tracked source.
- Do not use the old local Codex CLI review workflow in Codex-native sessions;
  use normal native subagents as work partners.
