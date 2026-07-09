# Lane reports

One file per lane: `<lane-name>.md`. Written by the lane, never by another lane — a fresh file
per lane cannot merge-conflict.

**Lanes must NOT edit `EXTRACTION_OPTIMIZATION_PLAN.md`.** Concurrent lanes all appending to the
plan of record is what produced the only merge conflict of the 2026-07-10 wave. The orchestrator
reads these reports and folds the findings into the plan, keeping its narrative coherent.

A lane report states, with numbers:

- **Mission** and what actually changed (files, `file:line`).
- **Findings** — including hypotheses that were *refuted*. A refuted brief is a valuable result:
  the most useful outcomes of the 2026-07-10 wave were all lanes disproving their own premise
  (d48's root cause was not the suspected tracker stall; d53's per-field delta premise was false;
  the engine dirty-signal hunt was a dead end; the fold's bottleneck was not serialization).
- **Losslessness argument** for any emission/fold/read-path change, and the gate that proves it.
- **Gate evidence** — exact test counts, hashes, before/after measurements. Numbers, not adjectives.
- **NULL results** — a lever that did not help, measured and reverted, documented honestly.
- **What must still be validated live**, and how.

Context every lane needs first: [`../CODEX_LANE_BRIEF.md`](../CODEX_LANE_BRIEF.md).
