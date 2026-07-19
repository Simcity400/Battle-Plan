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

## Where the evidence lives (cold-start bridge)

The RE-evidence program and its stores are NOT reachable from the specs above;
start here:

| Entry | Purpose |
| --- | --- |
| `lane_reports/rendering_laws/PROGRAM_STATUS.md` | Orchestrator status of the rendering-laws RE program + the machine-local evidence-store index (capture corpora, RenderDoc mined store, Ghidra exports). |
| `lane_reports/rendering_laws/EVIDENCE_REGISTRY.md` | Per-packet evidence registry (machine-readable: `../../re_evidence/rendering_laws/EVIDENCE_REGISTRY/registry.json`). |
| `lane_reports/rendering_laws/CORPUS_INVENTORY.md` | Capture-corpus inventory (machine-readable: `../../re_evidence/rendering_laws/CORPUS_INVENTORY/registry.json`). |
| `../../documents/GIT_CONSOLIDATION_2026-07-19.md` | Git history consolidation of record: pre-consolidation bundle locations, rescue branches, off-repo orchestration state (`C:\Users\Example\hoi4_lanes\`). |
