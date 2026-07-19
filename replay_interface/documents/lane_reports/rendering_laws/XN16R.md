# XN16R — `FINAL_full` rivers pass-close RTV recovery

## Disposition

**HUMAN_CAPTURE_REQUIRED — EXACT RETAINED PAYLOAD PROVEN ABSENT.** This is not a partial reverse-engineering result. The required engine-produced bytes do not exist in the retained native bundles or raw RDC stores, and transport from another capture fails exact authentication.

## Mechanical result

The frozen `live_FINAL_full` event stream records 60 successive incomplete rivers bundle snapshots, with draw counts exactly 1 through 60, followed by `capture_budget_exceeded` inside `rivers_pass` at draw 60. The session never reached the pass-close boundary. Its final surviving bundle has provenance `early_d3d11_active_river_draw_sequence_capture_in_progress`, retains no post-pass RTV payload, and carries the two explicit blockers already reported by XN16.

The recovery audit expanded the old 199-file count to every raw store now present:

| Store | RDCs |
| --- | ---: |
| `%LOCALAPPDATA%\hoi4_renderdoc` | 199 |
| `C:\tmp\hoi4_rendering_law_capture` | 15 |
| Total directly converted/mined | 214 |

The wider filesystem census found no additional RDCs in AppData outside the known store, Roaming, Codex temporary/worktree storage, OneDrive, Documents, Downloads, the preserved corpus, `battle-plan_main`, or any top-level `the game*` worktree/corpus directory.

All 214 RDCs have a per-file path, byte count, timestamp, SHA-256, direct-mine path, draw/pass summary, river-shader draw census, OM transition summary, and clear counts in `raw_store_census.json`. Across them, 2,966 serialized 416-byte scene-CB writes were hashed. None equals the frozen `FINAL_full` rivers scene-CB FNV-1a-64 `5866dccf4c7cb126`.

Two native sessions do contain a complete rivers pass and pass-close RTV (`live_PAN2` and `live_PAN_chunks`). Both use the same river shader pair, but both have different scene CB0 and pre-draw render-target histories. Their payloads are therefore different-capture results and are rejected; no `ResourceId` or RTV payload is transported across captures.

## Promotion rule

The production compose gate remains fail-closed for `live_FINAL_full`. A replacement native capture must reach `finalized_by: pass_close` and freeze the engine RTV payload after the final rivers draw. A replay-rendered output, a different capture's complete payload, or the last retained pre-draw snapshot is not an admissible substitute.

The exact replacement instructions are frozen in `CAPTURE_CARD_FINAL_FULL.md`. No `PROGRAM_STATUS` edit and no adversarial review were performed.

## Ghidra provenance

No new binary claim was made. Function address, job ID, artifact path, and manifest hash are therefore all null with the explicit reason recorded in the manifest.
