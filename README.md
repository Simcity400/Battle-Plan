# Battle Plan / Replay

Private, read-only tooling for recording and replaying the owner's Hearts of Iron
IV multiplayer games.

The repo is organized around three working areas:

| Path | Purpose |
| --- | --- |
| `extractor/` | Runtime extraction and recording work: DLL collector, Rust recorder host, compact domains, Python dev/proof harnesses, memory-reader research, save-oracle validation, static definition parsing, and extraction status documents. |
| `replay_interface/` | Replay-consumer work: `.battleplan` schema/reader code, map and visual replay work, game asset extraction, renderer/model helpers, and replay-interface documents. |
| `documents/` | Cross-cutting project docs and agent operating rules. |

Root files are intentionally minimal. `AGENTS.md` is a pointer for agent tooling,
`.gitignore` keeps generated artifacts out of Git, and this README is only a
directory map.

## Source Of Truth

- Extraction completion: `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
- Extraction domain IDs: `extractor/documents/DOMAIN_REGISTRY.md`
- Recording-layer plan: `extractor/documents/RECORDING_LAYER_COMPLETION_PLAN.md`
- Collector architecture: `extractor/documents/IN_PROCESS_COLLECTOR_PLAN.md`
- Replay container format: `replay_interface/documents/HOI4REPLAY_CONTAINER_SPEC.md`
- Replay schema notes: `replay_interface/documents/REPLAY_SCHEMA.md`
- Agent rules: `documents/AGENTS.md`

## Common Checks

```powershell
cargo metadata --manifest-path extractor\native\Cargo.toml --no-deps --format-version 1
cargo metadata --manifest-path replay_interface\native\Cargo.toml --no-deps --format-version 1
$env:PYTHONPATH = "extractor;replay_interface"
python -m pytest extractor\save_oracle extractor\game_defs replay_interface\replay extractor\live_capture extractor\coverage_audit
python extractor\coverage_audit\check_coverage.py
```

Production live capture remains:

```text
hoi4.exe -> one injected DLL collector/coordinator -> typed compact records -> Rust recorder -> .battleplan
```

Python remains development/proof/validation tooling only, not the production hot
extraction loop.
