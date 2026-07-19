# Replay Interface Documents

Active replay-interface documentation is intentionally limited to specs and
standards.

| File | Purpose |
| --- | --- |
| `REPLAY_ARCHITECTURE.md` | Architecture of record for the replay layer: crates, data flow, ingest boundary, sequencing. |
| `../../documents/SEAM_CONTRACTS.md` | Open requests between the replay and extraction layers (the seam contract, both directions). |
| `REPLAY_DATA_REQUIREMENTS.md` | Self-contained recorded-data contract handed to the extractor. |
| `HOI4REPLAY_CONTAINER_SPEC.md` | `.battleplan` container format. |
| `REPLAY_SCHEMA.md` | Canonical replay snapshot schema. |
| `MAP_REPLICA_PLAN.md` | Map/renderer parity plan (grounding rules and per-layer evidence). |
| `UI_SOURCE_OF_TRUTH_REPORT.md` | UI fidelity standard and evidence hierarchy. |

Do not create parallel progress ledgers in this folder. Replay status lives in
`../../documents/STATUS.md`; if replay UI work is blocked by missing recorded
data, update `../../documents/SEAM_CONTRACTS.md`. If a schema or map rule
changes, update the relevant spec above. Superseded plans, ledgers, and lane
reports live in `archive/`.

## Where the evidence lives (cold-start bridge)

The RE-evidence program and its stores are NOT reachable from the specs above;
start here:

| Entry | Purpose |
| --- | --- |
| `lane_reports/rendering_laws/PROGRAM_STATUS.md` | Orchestrator status of the rendering-laws RE program + the machine-local evidence-store index (capture corpora, RenderDoc mined store, Ghidra exports). |
| `lane_reports/rendering_laws/EVIDENCE_REGISTRY.md` | Per-packet evidence registry (machine-readable: `../re_evidence/rendering_laws/EVIDENCE_REGISTRY/registry.json`). |
| `lane_reports/rendering_laws/CORPUS_INVENTORY.md` | Capture-corpus inventory (machine-readable: `../re_evidence/rendering_laws/CORPUS_INVENTORY/registry.json`). |
| `../../documents/archive/GIT_CONSOLIDATION_2026-07-19.md` | Git history consolidation of record: pre-consolidation bundle locations, rescue branches, off-repo orchestration state. |
| `<local-path>` (machine-local) | THE single physical root for all machine-local evidence/capture/RE stores (consolidated 2026-07-19; junctions remain at old paths). Repo-side index: `lane_reports/rendering_laws/PROGRAM_STATUS.md`. |
