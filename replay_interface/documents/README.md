# Replay Interface Documents

Active replay-interface documentation is intentionally limited to specs and
standards.

| File | Purpose |
| --- | --- |
| `REPLAY_ARCHITECTURE.md` | Architecture of record for the replay layer: crates, data flow, ingest boundary, sequencing. |
| `EXTRACTION_INTERFACE_REQUESTS.md` | Open requests from the replay layer to the extraction layer (the seam contract). |
| `HOI4REPLAY_CONTAINER_SPEC.md` | `.battleplan` container format. |
| `REPLAY_SCHEMA.md` | Canonical replay snapshot schema. |
| `MAP_REPLICA_PLAN.md` | Map/renderer parity plan (grounding rules and per-layer evidence). |
| `UI_SOURCE_OF_TRUTH_REPORT.md` | UI fidelity standard and evidence hierarchy. |

Do not create parallel progress ledgers in this folder. If replay UI work is
blocked by missing recorded data, update
`extractor/documents/MASTER_EXTRACTION_CONTRACT.md`. If a schema or map rule
changes, update the relevant spec above.
