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

All named live/runtime matrices in this table are optional fail-closed feature
activation evidence, not product-promotion prerequisites. Their absence keeps
only the named surface unavailable. CID1 M3 on a second physical machine is the
sole genuine external dependency. Historical validation wording below is not
authority for the current worktree; changed rows require central validation and
content-addressed refreeze.

| ID | What the replay layer needs | Extractor state | Blocks (replay packet) |
| --- | --- | --- | --- |
| A26 | Resolved per-instance tooltip rectangle + exact final post-layout blob (d72) | Final-blob owner, TTI2 carrier, atomic selected-Present seal, fold, replay/HUD consumer, matrix assembler, and verifier are complete and centrally validated. D72 remains reserved until the autonomous H-A26 hidden/visible/lifecycle/multiline/two-resolution matrix passes, followed by its activation refreeze | XUI05 |
| A28 | Authenticated dynamic-collection visible order — remaining families (d71) | Static RE is exhausted and four exact hook families are implemented fail-closed. Autonomous owner captures remain; air-selection and three MIO graphs remain unavailable until their machine ledgers exist and are never inferred from generic widget traversal | XUI03, XUI16, XUI17 |
| A29 | Research ahead-of-time inputs | Static exact-producer RE and pointer-free same-hour carrier/fold/replay wiring complete under slot-bound `a729_observed_research_ahead_producer_v2`; branch 0 authenticates the exact no-display/`NO_RESEARCH_PENALTY` result. Base-factor inference is forbidden. H-A29 branch-1/2 evidence is optional regression coverage and never controls activation | XUI09 (wired fail-closed; unsupported observations remain unavailable) |
| A30 | Intelligence/operative UI state | Exact static RE and ordinary passive production scheduling are complete for agency/cipher controller rows and the optional selected-Present companion. The current executable and six hook sites are authenticated before observation; owner, selected target, hour, sequence, delayed-frame insertion and campaign reset are fail-closed. Unopened surfaces remain unavailable. H-A30 is optional regression evidence and no longer gates production | XUI14 (production wired fail-closed) |
| A31 | Focus-presentation runtime leaves | D89 producer/codec/fold/replay/UI and centralized validation are complete; the autonomous H-A31 stable-plus-discriminating selected-Present fixture remains | XUI10 (exact admission wired; d89 reserved pending H-A31) |
| A32 | Display/language/input launch variant | Display/language carrier, authenticated pointer-family observer, manifest-pinned materializer, pre-d77 admission bind, and centralized validation are complete. One autonomous same-launch pointer-movement receipt and authority refreeze remain; keyboard/controller traversal is explicitly unsupported | XUI25 |
| A33 | Combat detail/losses | Static RE and fail-closed carriers/consumers are complete. Activation remains split across autonomous H-A33-SUNK and H-X2 loss/width/panel observations | XUI20 |
| A34 | Decision-panel generic rows/costs/alerts | Corrected controller graph, DPN1 v2 carrier, selected-Present join, fold/replay/UI, catalog admission, and autonomous evidence transaction are complete and centrally validated. Activation waits on clean catalog packaging plus the nonempty/measured-empty matrix | XUI19 |
| A36 | Active abilities: availability/activation/cooldown/cost/target/duration/order/tooltip/disabled-reason | Static RE, exact active-record/caster trust boundary, ten-hook evidence owner, ABA2/ABL1 carrier, fold, replay and UI are complete and centrally validated. d88 remains reserved pending the mechanically regraded live transition/multi-leader/measured-empty matrix; no semantic RE remains | XUI23 (wired fail-closed; external runtime matrix pending) |
| A37 | Optional power-balance semantic companion (d74 reserved) | Visible production pixels are owned by the mandatory authenticated selected-Present UI pass. The open-panel-only PBP2 carrier remains fail-closed and cannot substitute for that pixel path; its absent ordinary sealer is not a product-completion blocker | XUI18 (optional typed companion only) |
| A38 | Scripted-GUI engine-materialization carrier | Exact owner/updater/text/sprite hooks, d84 carrier, fold, typed replay, and evaluator are implemented fail-closed and centrally validated. Activation waits only on the autonomous representative live-materialization matrix | XUI22 |
| A39 | Character-presentation residuals | Wired and centrally validated; activation-gated | Reserved d82 CPR1 v2 exact order/state/progress and final ace portrait/modifier observers; only the frozen mixed live fixture remains before scheduling |
| A40 | Designer/peace snapshots (live carrier) | Exact RE and production-schema engineering are closed. D85/peace activation waits on autonomous six-designer SDI1 observations plus one loss-free peace lifecycle; refresh-scoped allocations and explicitly unjoined terminal rows are preserved without invented identity | XUI08 |
| XUI21-E1 | Fired-event/popup producer (d75; contract frozen in `extractor/documents/dossiers/XUI21_FIRED_EVENT_POPUP_INPUTS.json`) | Common-fire/pending/visible/resolution hooks, carrier, completeness telemetry, and consumer are implemented fail-closed; H-POPUP is an autonomous coverage matrix | XUI21 |
| X4-P1..P6 | Production/deployment-queue inputs | Schema-v2 typed deployment-queue producer and replay consumer are implemented, including exact save/rehost queues; current central validation/refreeze pending | XUI12 |
| X15 | Resource trade P1–P5 integration | Semantic production route and authored production-license row binding are implemented; current central validation/refreeze pending. Selected-POV pixel evidence is optional and fail-closed | XUI15 |
| XUI03-rows | Controller-boundary collection rows | Consumer and fail-closed carrier are implemented; authentic A28 owner captures/ledgers remain autonomous activation evidence | XUI03 |
| A15-orgmax | Division-counter organization maximum or an exact normalized organization ratio | Exact organization maximum/root and normalized organization-bar binding are implemented fail-closed; current central validation/refreeze pending | counter binding / CAP6 composition |

**Delivered AND consumed:** A27 (design catalog — consumed by
`game_ui::design_catalog::admit_design_catalog`, gate 1/1; XUI13's remaining
gate is designer-surface activation authentication, not catalog data), A35
(occupation/garrisons). A32 is intentionally listed above: its display and
language carrier is consumed, but its input-family authority is not active.

## 2. Extractor ← waiting on replay layer

| ID | What the extractor needs | State | Full spec |
| --- | --- | --- | --- |
| X-DECODER | The ownership and memory conflicts are mechanically closed without adding an extractor dependency to `replay_core`: the portable crate owns both certified in-memory and disk-backed request/result contracts plus the source receipt; `Container::spool_entry_to_writer` authenticates a file-backed role entry directly into a private spool; `ProductionCertifiedPrefoldDecoder` drives the existing file-backed single `WorldFold`; and `replay_core::open_recording_with_spooled_decoder` lazily reopens and rehashes one certified hour at a time. Source length/SHA, strict EOF, and `FRAME_END == WorldFold emissions == spilled files` cardinality are mandatory. Cleanup is identity/marker/reparse gated. The shipped `replay_app::recording_open` policy now statically supplies that bounded capability from interactive, headless, and validation entry points, exposes the receipt, and has no ordinary/raw fallback. Ordinary library `open_recording` remains fail-closed. | CLOSED implementation; current central validation/refreeze pending. Prior green receipts are historical. | `archive/SEAM_EXTRACTION_TO_REPLAY_2026-07-16.md` |
| X-ENDEDCOMBAT | `ended_combats[]` is consumed as a typed `BTreeMap<(combat_id, generation), EndedCombatRow>` and exposed through a terminal-only XUI20 surface; the active combat renderer still reads only `combats[]`. Same-hour coalescing unions terminal rows by the complete identity, preserves absent/null/known-empty authority, null-aware merges complementary leaves, and deterministically prefers `side_detail_availability="present"` over `"unavailable"` in either input order. | CLOSED implementation; central regression validation required | `archive/SEAM_EXTRACTION_TO_REPLAY_2026-07-16.md`, §X-ENDEDCOMBAT |
| X-DLCLIST-NAME1B | The container-authenticated recorder manifest maps every enabled DLC descriptor path to its exact nonblank display name, binds that list into `session.active_dlcs`, and production NAME1b consumes the resulting exact case-sensitive predicate. Measured-empty selects the no-DLC branch; missing, unobtained, blank, or conflicting metadata fails closed. | CLOSED implementation; current central validation/refreeze pending. Prior green receipts are historical. | archived R-DLCLIST |
| X-COUNTER-BIND | Join the existing exact supply/dig-in/training/active-ability/order-kind inputs into `game_ui::binding_counters`; keep unavailable fields fail-closed | CLOSED implementation, including exact organization maximum/ratio binding; current central validation/refreeze pending | archived old A15 |

## 3. Legacy-row deduplication (completed 2026-07-22)

The archived rows are not additional seams. Their current dispositions are:

| Archived row | Current authority / disposition |
| --- | --- |
| R-SNOWMUD | Static RE and producer/upload/consumer wiring are closed; temporal promotion is canonical autonomous card H-SNOWMUD |
| R-COSMETIC | Consumed from the exact Domain 18 country identity `cosmetic_tag`; closed |
| R-IDEOLOGY | Consumed from the exact country-politics `ruling_party`; closed |
| R-DLCLIST | Superseded and closed by X-DLCLIST-NAME1B: authenticated descriptor display names reach the production NAME1b consumer with exact case-sensitive matching and fail-closed absence/mismatch behavior; H-MODSET-CAPTURE remains the chosen-modset acquisition gate |
| R-CLOCK | Exact replay hour plus X12 sub-hour/pause transitions supersede the old confirmation request; H-X12-SPEED is the autonomous activation receipt |
| old A7 (unit positions/types) | Superseded by the typed M01 inputs and `air_base_province`; M01 is `PROVEN` |
| old A8 (rail/hub state) | Superseded by promotion-ready D81 rail replay state |
| old A14 (POV/FoW) | Superseded by production-complete D94 FOW1 and D95 SPU1 typed POV inputs; CAP6/G2 observations are fidelity gates, not a missing schema |
| old A15 (counter badges) | The legacy umbrella is retired, but its surviving work is re-filed explicitly as A15-orgmax and X-COUNTER-BIND; CAP6 H1..H5 remains external precedence/fidelity evidence rather than a substitute for per-hour typed inputs |
| old A16 (combat indicators) | Superseded by M02, `PROVEN` with its portable carrier and exact capture |
| old A17 (world tension) | Superseded by the exact d57 scalar/details carrier, fold, and strict replay presentation |
| old A18 (alert sub-counts/feed) | Superseded by d79/TBA1 schema 2 active-alert snapshot plus XUI21-E1 for fired-event chronology; the engine has no per-alert fire-time field |

No undeduplicated legacy row remains open.
