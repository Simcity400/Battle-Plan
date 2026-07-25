# Battle Plan / Replay

Private, read-only tooling for recording and replaying the owner's Hearts of
Iron IV multiplayer games.

**Current status and remaining work are split by layer:**

- `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` — extraction completion checklist and backlog
- `replay_interface/documents/MAP_REPLICA_PLAN.md` — map and renderer parity plan

Start there. `replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md` defines
the recorded-data contract between the replay and extraction layers.

## Layout

| Path | Purpose |
| --- | --- |
| `extractor/` | Recording half: injected DLL collector, Rust recorder host, compact domains, save-oracle validation, static parsing. Deep ledgers in `extractor/documents/`. |
| `replay_interface/` | Playback half: replay app, map/UI rendering, container reader, RE evidence. Specs and the rendering-laws program board in `replay_interface/documents/`. |
| `portable_replay/` | Neutral format crates both halves statically link. |
| `documents/` | Project-wide design. |

Anything in an `archive/` folder is historical evidence, never the current
plan. The repo carries no process/rules docs (archived 2026-07-19).

## Common checks

```powershell
cargo metadata --manifest-path extractor\native\Cargo.toml --no-deps --format-version 1
cargo metadata --manifest-path replay_interface\native\Cargo.toml --no-deps --format-version 1
$env:PYTHONPATH = "extractor;replay_interface"
python -m pytest extractor\save_oracle extractor\game_defs replay_interface\replay extractor\live_capture extractor\coverage_audit
python extractor\coverage_audit\check_coverage.py
```

Production live capture:

```text
hoi4.exe -> one injected DLL collector/coordinator -> typed compact records -> Rust recorder -> .battleplan
```

Python is development/proof/validation tooling only, never the production hot
extraction loop.
