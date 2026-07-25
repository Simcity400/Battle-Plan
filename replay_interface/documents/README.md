# Replay Interface Documents

Active replay-interface documentation is intentionally limited to specs and
standards.

| File | Purpose |
| --- | --- |
| `REPLAY_ARCHITECTURE.md` | Architecture of record for the replay layer: crates, data flow, ingest boundary, sequencing. |
| `REPLAY_DATA_REQUIREMENTS.md` / `../../extractor/documents/MASTER_EXTRACTION_CONTRACT.md` | Current recording asks and extractor authority/status across the replay-extraction seam. |
| `REPLAY_DATA_REQUIREMENTS.md` | Self-contained recorded-data contract handed to the extractor. |
| `HOI4REPLAY_CONTAINER_SPEC.md` | `.battleplan` container format. |
| `REPLAY_SCHEMA.md` | Canonical replay snapshot schema. |
| `UI_SOURCE_OF_TRUTH_REPORT.md` | UI fidelity standard and evidence hierarchy. |

Do not create parallel progress ledgers in this folder. If replay UI work is
blocked by missing recorded data, update `REPLAY_DATA_REQUIREMENTS.md` and
`../../extractor/documents/MASTER_EXTRACTION_CONTRACT.md` as appropriate. If a schema or map rule
changes, update the relevant spec above. Superseded plans, ledgers, and lane
reports live in `archive/`.

## Where the evidence lives (cold-start bridge)

The RE-evidence program and its stores are NOT reachable from the specs above;
start here:

| Entry | Purpose |
| --- | --- |
| `lane_reports/rendering_laws/PROGRAM_STATUS.md` | Orchestrator status of the rendering-laws RE program + the machine-local evidence-store index (capture corpora, RenderDoc mined store, Ghidra exports). |
| `<local-path>` (machine-local) | THE single physical root for all machine-local evidence/capture/RE stores (consolidated 2026-07-19; junctions remain at old paths). Repo-side index: `lane_reports/rendering_laws/PROGRAM_STATUS.md`. |
