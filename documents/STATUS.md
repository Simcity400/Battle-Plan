# Project status

**Updated: 2026-07-19.** This page, `REMAINING_WORK.md`, and
`SEAM_CONTRACTS.md` are the only top-level current-state documents. Everything else is spec, evidence, or
archive. If you are an agent landing cold: read this, then `REMAINING_WORK.md`,
then follow links only as needed.

**Product:** private, read-only recording + 1:1 replay of the owner's the game
multiplayer games. Pipeline:
`hoi4.exe → injected DLL collector → typed compact records → Rust recorder → .battleplan → replay app`.
Doctrine: fail-closed — anything the replay cannot reproduce exactly surfaces
as a typed blocker, never a silent approximation.

## Extractor (recording half) — broad coverage, not production-closed

- All 14 gameplay areas extracted at DONE-**base** slices — named residual
  functionality remains per the master's coverage table (market, naval,
  combat, intel, supply, event-hook residuals). 71 domains registered
  (`extractor/native/domain_registry.json` is code-authoritative).
- A production promotion tuple (A1/A2/A4/A12/A14/A15/A21/A22/B8-X6) plus
  A27/A35 has frozen promotion evidence. A32's display/language carrier is
  integrated, but its input-family authority is disabled and unbound, so A32
  and XUI25 currently reject fail-closed rather than qualify for promotion.
- **Not closed:** the product pillars P1–P5 (auto-attach, duplex control,
  campaign identity, stitching, hardening) are offline-green with activation
  deliberately OFF — the active path is still Legacy. No live speed-5
  acceptance proof exists. Path to production =
  `extractor/documents/LIVE_ACCEPTANCE_RUNBOOK.md` (5 owner-run sequences,
  ~36 launches; sequence 5 needs a second machine + player), plus its
  OWED-PRODUCER tooling that must be built first.
- Detail ledgers: `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
  (completion + backlog) and `extractor/documents/RE_REMAINING.md` (RE ledger,
  incl. the A24–A40 seam contracts).

## Replay layer (playback half) — laws complete for promoted scope, endgame gates open

- The rendering-laws RE program closed its full register: all XN01–XN17 non-UI
  packets and XUI01–XUI26 UI packets are PROVEN or DATA_BLOCKED
  (replay-side done, waiting only on extractor inputs — see
  `SEAM_CONTRACTS.md`). Board of record:
  `replay_interface/documents/lane_reports/rendering_laws/PROGRAM_STATUS.md`.
- **Not closed:** XUI27 (final pixel-parity composition over every panel
  family) has never run; the adversarial release-review wave has never run;
  the F1 performance gate was never contracted (owner must name a reference
  machine); a set of capture-gated residuals remains. The 2026-07-19
  "REPLAY LAYER COMPLETE" finalization receipt
  (`replay_interface/re_evidence/rendering_laws/replay_promotion/REPLAY_CORE_FINALIZATION_20260719.md`)
  is **invalid as a freeze authority** — 3 of its 4 SHA256s drifted the same
  day; it must be reissued after the current fix wave.
- A verified 2026-07-19 robustness audit found fail-closed holes and container
  robustness gaps; a code fix wave is addressing them (list + state in
  `REMAINING_WORK.md` §2).
- Architecture: `replay_interface/documents/REPLAY_ARCHITECTURE.md`. Specs
  (all under `replay_interface/documents/`): `HOI4REPLAY_CONTAINER_SPEC.md`,
  `REPLAY_SCHEMA.md`, `REPLAY_DATA_REQUIREMENTS.md`, the `*_LAW.md` files
  (permanent reference), `MOD_COMPATIBILITY.md`,
  `PRODUCTION_PLAYBACK_PROFILE.md`, `PRODUCTION_VFS_REPORT.md`.
- `portable_replay/native/` = the four neutral format crates both halves
  statically link (`portable_replay_format`, `portable_content_manifest`,
  `portable_render_stream`, `portable_compact_stream`). The `.battleplan` file
  is the only runtime interface between the halves.

## The seam

Two-directional dependency list (what blocks what, both ways):
`documents/SEAM_CONTRACTS.md`. This is the most production-relevant page in
the repo.

## Evidence stores

All machine-local evidence lives under **`C:\hoi4_evidence\`** (own README
manifest; junctions remain at old paths). Repo-side index: the evidence-store
table at the bottom of
`replay_interface/documents/lane_reports/rendering_laws/PROGRAM_STATUS.md`.
Known loss: the `restore_gate_1`
capture is LOST — grading/bloom/political-v2 law re-validation needs a
recapture. Pre-consolidation git history: bundle at
`<local-path>` (see
`documents/archive/GIT_CONSOLIDATION_2026-07-19.md`; its 3 rescue branches are
tracked in `REMAINING_WORK.md` §4).

## Documentation policy

Simple by design: STATUS.md + REMAINING_WORK.md + SEAM_CONTRACTS.md are the
whole current-state surface. Per-half deep ledgers stay next to their code.
Superseded material lives in `archive/` folders (evidence, not direction) and
is never the current plan. The repo intentionally carries **no process/rules
docs** (archived 2026-07-19); operating method is machine-local
(`C:\Users\Example\.claude\CODEX_PROTOCOL.md`).
