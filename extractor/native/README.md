# extractor/native

Native extraction workspace for the Battle Plan recorder.

This folder owns the production recording path:

```text
hoi4.exe -> injected DLL collector/coordinator -> compact records -> Rust recorder -> .battleplan
```

## Crates

| Crate | Role |
| --- | --- |
| `collector` | In-process collector DLL, attach ABI, compact row emission, and shared domain registration. |
| `recorder_host` | Rust recorder host for DLL compact streams, manifests, speed/cadence gates, and `.battleplan` packaging. |
| `snapshot` | Snapshot-store and `.battleplan` reader/writer primitives used by the recorder. |
| `lanes/*` | Lane-local extraction mini-stacks, probes, compact schema experiments, and proof gates. |
| `project_progress_live_lane` / `scientists_live_lane` | Project/scientist lane work that has not been folded into the generic lane layout. |

## Source Of Truth

- `../documents/MASTER_EXTRACTION_CONTRACT.md`
- `../documents/DOMAIN_REGISTRY.md`
- `../documents/REVERSE_ENGINEERING_TOOLING.md`

Historical lane packets, registration plans, blocker reports, and old status
notes are archived under `../documents/archive/native_lanes/`.

## Checks

```powershell
cargo metadata --manifest-path extractor\native\Cargo.toml --no-deps --format-version 1
cargo test --manifest-path extractor\native\Cargo.toml --workspace --no-fail-fast
cargo fmt --manifest-path extractor\native\Cargo.toml --all --check
```

## Guardrails

- Keep the production hot path DLL/Rust-only.
- Python may validate and probe, but must not become hourly/sub-hourly live extraction.
- New lane work should stay lane-local until it has compact records, decoder/gate coverage, and proof evidence.
- Generated runs, proof outputs, target dirs, recordings, and local paths stay ignored.
