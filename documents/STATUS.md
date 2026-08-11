# Project status

`DECISIONS.md` defines product intent. This file is the current project-status
entry point; it does not duplicate packet ledgers or development history.

The shipping pipeline is:

`hoi4.exe -> typed recorder data -> .battleplan -> Replay application`

Both layers fail closed. Missing, partial, unauthenticated, wrong-hour, or
wrong-content data remains typed unavailable and is never replaced with a zero,
another country, recorder UI state, selected pixels, hard-coded game content,
or a visual approximation.

## Current authorities

- Extractor status: `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` and
  `extractor/documents/RE_REMAINING.md`.
- Replay external-producer boundary:
  `replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json`.
- Generated human view of that boundary:
  `replay_interface/documents/EXTRACTOR_INPUT_RESIDUALS.md`.
- Replay UI product route:
  `replay_interface/re_evidence/rendering_laws/XUI27/OBSERVER_PRODUCT_ROUTE.md`.
- Replay map product status:
  `replay_interface/documents/MAP_GRAPHICS_PRODUCT_STATUS.md`.
- Non-UI map machine ledger and audit:
  `replay_interface/re_evidence/rendering_laws/XMAP00/`.
- Map release acceptance and product sequence:
  `replay_interface/re_evidence/rendering_laws/XMAP22/`.

Passive clock capture writes `PASSIVE_CLOCK_INTERVALS_V2.json` as its interval
authority. Its direct-displacement writer census is an enumerated set rather than a proved exhaustive one;
interior-pointer and bulk-copy writes remain outside that search claim.

Replay certification proves the exact Replay source closure, packet authorities,
external dependency identities, omission audits, and selected shipping behavior.
It does not certify external producer implementation or measured end-to-end
framebuffer equality. The current terminal receipt is authoritative only for the
exact source identity it names.

Machine-local evidence remains under `C:\hoi4_evidence\` and is indexed there.
Git history is the development chronology; dated handoffs and copied status
tables are not current authority.
