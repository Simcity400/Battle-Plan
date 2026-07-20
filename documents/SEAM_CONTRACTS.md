# Extraction ↔ Replay seam contracts

The single two-directional list of what each half of the product is waiting on
from the other. Update THIS file when a seam item opens or closes; the full
original request texts are archived (linked below) and are not edited.

**Numbering warning:** the canonical A-numbers are the extractor **dossier
namespace** (`extractor/documents/dossiers/`, A24–A40), as tracked in
`extractor/documents/RE_REMAINING.md`. Older replay docs used a colliding
A-numbering (e.g. old "A27" = intel, canonical A27 = design catalog). Any
A-number found in an archived doc must be checked against the dossier
namespace before use.

## 1. Replay layer ← waiting on extractor

Extractor-side state is tracked in `extractor/documents/RE_REMAINING.md`;
replay-side packet states in
`replay_interface/documents/lane_reports/rendering_laws/PROGRAM_STATUS.md`.

| ID | What the replay layer needs | Extractor state | Blocks (replay packet) |
| --- | --- | --- | --- |
| A24 | Politics evaluated-tooltip inputs | CONTRACT REQUIRED | XUI11 |
| A25 | Runtime text/localization/formatting/wrap/glyph/inline-icon runs (d76 reserved) | CONTRACT REQUIRED | XUI06, XUI16, XUI17, XUI18 |
| A26 | Resolved per-instance tooltip rectangles + ordered evaluated content rows (d72) | Active RE | XUI05 |
| A28 | Authenticated dynamic-collection visible order — remaining families (d71) | Open (equipment-grid producer proven) | XUI16, XUI17 |
| A29 | Research ahead-of-time inputs | DATA BLOCKED extractor-side | — (XUI09 PROVEN with the delivered subset; residual is extractor-side only) |
| A30 | Intelligence/operative UI state | DATA BLOCKED | XUI14 |
| A31 | Focus-presentation runtime leaves | Open | — (XUI10 PROVEN with zero extractor reads; residual is extractor-side only) |
| A33 | Combat detail/losses | INTEGRATION+RE+FIXTURE | XUI20 |
| A34 | Decision-panel generic rows/costs/alerts | IN PROGRESS | XUI19 |
| A36 | Active abilities: availability/activation/cooldown/cost/target/duration/order/tooltip/disabled-reason | DATA BLOCKED | XUI23 |
| A37 | Power-balance runtime inputs (d74 reserved) | CONTRACT REQUIRED | XUI18 |
| A38 | Scripted-GUI engine-materialization carrier | CONTRACT REQUIRED | XUI22 |
| A39 | Character-presentation residuals | Open | — (board lists XUI17's upstream deps as A25/A28 only; A39 residual is extractor-side) |
| A40 | Designer/peace snapshots (live carrier) | Open | XUI08 |
| XUI21-E1 | Fired-event/popup producer (d75; contract frozen in `extractor/documents/dossiers/XUI21_FIRED_EVENT_POPUP_INPUTS.json`) | RE complete; collector attach + emission not frozen | XUI21 |
| X4-P1..P6 | Production/deployment-queue inputs | Promotion-ready | XUI12 |
| X15 | Resource trade P1–P5 integration | Open | trade surfaces |
| XUI03-rows | Controller-boundary collection rows | Open (tracked in extractor evidence) | XUI03 |

**Delivered AND consumed:** A27 (design catalog — consumed by
`game_ui::design_catalog::admit_design_catalog`, gate 1/1; XUI13's remaining
gate is designer-surface activation authentication, not catalog data), A32
(display variant), A35 (occupation/garrisons).

## 2. Extractor ← waiting on replay layer

| ID | What the extractor needs | State | Full spec |
| --- | --- | --- | --- |
| X-DECODER | The ownership and memory conflicts are mechanically closed without adding an extractor dependency to `replay_core`: the portable crate owns both certified in-memory and disk-backed request/result contracts plus the source receipt; `Container::spool_entry_to_writer` authenticates a file-backed role entry directly into a private spool; `ProductionCertifiedPrefoldDecoder` drives the existing file-backed single `WorldFold`; and `replay_core::open_recording_with_spooled_decoder` lazily reopens and rehashes one certified hour at a time. Source length/SHA, strict EOF, and `FRAME_END == WorldFold emissions == spilled files` cardinality are mandatory. Cleanup is identity/marker/reparse gated. The shipped `replay_app::recording_open` policy now statically supplies that bounded capability from interactive, headless, and validation entry points, exposes the receipt, and has no ordinary/raw fallback. Ordinary library `open_recording` remains fail-closed. | CLOSED implementation and activation; one centralized validation pass pending | `archive/SEAM_EXTRACTION_TO_REPLAY_2026-07-16.md` |
| X-ENDEDCOMBAT | Consume `ended_combats[]` (typed reader, render ended ≠ active, key by `(combat_id, generation)`) AND fix the `_merge_two_snapshots` union defect in `replay_interface/replay/diff_engine.py` (~1756) — a confirmed zero-data-loss defect | OPEN | same file, §X-ENDEDCOMBAT |

## 3. Legacy rows pending dedup

From `archive/SEAM_REPLAY_TO_EXTRACTION_2026-07-08.md` and the archived
`replay_interface/documents/archive/EXTRACTION_NEEDS.md`: R-SNOWMUD
(SnowMudData texture snapshot), R-COSMETIC,
R-IDEOLOGY, R-DLCLIST (resolved DLC display names), R-CLOCK, plus the old
A7/A8/A14–A18 rows. Each must be verified against the dossier namespace and the
current recorder schema before being treated as open; some are likely already
subsumed. Nobody has done this dedup pass yet (tracked in REMAINING_WORK.md).
