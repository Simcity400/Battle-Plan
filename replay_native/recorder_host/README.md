# recorder_host

Native proof host for the production recording path.

This crate is the first Rust-side replacement for the Python hot loop. It does
not walk HOI4 memory. It writes a bounded `collector_control.json`, installs the
collector DLL through the existing read-only Windows hook attach path, keeps the
hook alive for the worker duration, reads the DLL named-pipe stream, writes
`collector_frames.jsonl`, and emits `native_session_manifest.json` with cadence
and skipped-hour diagnostics.

Current scope:

- Production-shaped transport: `hoi4.exe` -> DLL collector -> native host raw
  stream.
- No Python in the live capture loop.
- No save input.
- No Python preflight bridge.
- Optional DLL-side project scanning through `--special-projects-scan`; current
  live evidence shows this is not yet bounded enough for production capture.
- Compact country-frame recording through `--compact-country-frame`; filtered
  sessions emit root snapshots only at scheduled game-hour boundaries and keep
  worker polls to raw clock plus explicitly requested heavy raw domains.
- For speed-five proof runs, use the release DLL/host and `--sample-ms 1`.
  The default remains 10 ms.
- No `.battleplan` packing yet. Python may validate the raw output, but a
  segment is not production-complete until this native path writes the replay
  file and passes hourly-or-better speed-five proof.

Example:

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p recorder_host --bin collector_record -- `
  --pid <hoi4-pid> `
  --dll replay_native\target\debug\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\clock_lightweight `
  --duration-s 180 `
  --sample-ms 10 `
  --min-game-hours 12 `
  --scheduled-domain clock `
  --lightweight-root-frame
```

Project probe example:

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p recorder_host --bin collector_record -- `
  --pid <hoi4-pid> `
  --dll replay_native\target\debug\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\projects_lightweight `
  --duration-s 15 `
  --sample-ms 10 `
  --min-game-hours 120 `
  --scheduled-domain projects_live `
  --lightweight-root-frame `
  --special-projects-scan
```

The command exits nonzero when the manifest fails the requested hourly gate.

Compact release country-domain proof example:

```powershell
cargo build --release --manifest-path replay_native/Cargo.toml -p collector -p recorder_host

replay_native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\politics_live_compact_release `
  --duration-s 15 `
  --sample-ms 1 `
  --dwell-s 0.25 `
  --min-game-hours 120 `
  --scheduled-domain politics_live `
  --compact-country-frame `
  --idea-group-filter-file replay\snapshots\_native_collector_session\static_contracts\idea_group_filter.txt `
  --advisor-token-filter-file replay\snapshots\_native_collector_session\static_contracts\advisor_token_filter.txt
```

When `--scheduled-domain` is supplied, heavy raw domains such as
`land_combats`, `divisions`, `army_orders`, `naval_air_combats`,
`naval_combat_members`, `air_wings`, `ships`, and `oob_command` are captured
only if explicitly requested. This keeps independent country-domain proof gates
from being slowed by unrelated late-game combat traffic. With no domain filter,
the DLL keeps the legacy default of enabling all domains.
