# Replay map and graphics product status

This page is the human current-state entry point for the non-UI Replay map.
Detailed rendering laws remain packet-local under
`replay_interface/re_evidence/rendering_laws/XMAP00` through `XMAP22`.

## Shipping route

Windowed playback, headless frame export, and the recording benchmark all use
`ProductMapCompositionSession::execute_on_target`. The shared route performs
content admission before device or UI setup, executes the same ordered native
composition on the caller-owned target, waits for GPU completion, reads the
target back, and seals the XMAP22 receipt. Windowed playback additionally binds
the actual `PresentOutcome`; skipped, occluded, or failed presentation cannot
advance interaction state.

XMAP22 requires every mandatory visual family, including XMAP11 trees and
XMAP14 combat indicators, and independently rebuilds packet witnesses plus the
foundation envelope. The canonical order and acceptance rules are:

- `replay_interface/re_evidence/rendering_laws/XMAP22/COMPOSITION_SEQUENCE_CONTRACT.md`
- `replay_interface/re_evidence/rendering_laws/XMAP22/FINAL_RELEASE_ACCEPTANCE.md`

## Semantic admission

The machine authority for map inputs is
`replay_interface/re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv`.
Its audit and consumer census are in the same XMAP00 directory. Each input has
an explicit owner and disposition. Missing, incomplete, wrong-hour, or
wrong-content data stays feature-locally unavailable; Replay never substitutes
recorded camera or UI state, selected-Present draws, installed-tree guesses,
game-start values, representative entities, neutral colours, empty collections,
or tuned constants.

External producer requirements are canonical only in
`replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json`.
`EXTRACTOR_INPUT_RESIDUALS.md` is its generated human view. This page does not
copy volatile row counts.

## Completion boundary

Replay owns and continuously tests the consumer schemas, effective-VFS
admission, rendering laws, native execution, ordering, lifecycle, receipts, and
feature-local failure behavior. "Continuously tests" is the exact claim: the
gates are the Rust battery over the current tree plus the ledger checker in
`XUI27/external_dependency_ledger.py`, not a frozen certification receipt. The
receipt that used to back the stronger word was retired by lane KK on
2026-08-15 as machinery no gate invoked. Replay does not claim that an external
producer is implemented and does not use exhaustive framebuffer byte identity as
its completion standard. Exact engine/game-file behavior is the
standard; fabricated or visually fitted substitutes are forbidden.
