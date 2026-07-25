# Project status

> **Read `DECISIONS.md` first.** It states what the product is supposed to be, and it
> outranks this page and every specification here. This page describes the system **as
> built**, and parts of it are being deliberately replaced — acquisition, the wire format
> and the container. Do not treat what follows as the target.
>
> Two things below are already superseded by `DECISIONS.md`: the recorder is described as
> read-only, but today's collector writes the engine pause byte before every sweep, and that
> mechanism is being removed; and the product is described as private to one owner, whereas
> it must now work for any player on any mod set with any DLC.

**Updated: 2026-07-22.** This page, `REMAINING_WORK.md`, and
`SEAM_CONTRACTS.md` describe current implementation state. `DECISIONS.md` describes intent.
If you are an agent landing cold: read `DECISIONS.md`, then this, then `REMAINING_WORK.md`.

**Product as built:** recording + replay of the game multiplayer games. Pipeline:
`hoi4.exe → injected DLL collector → typed compact records → Rust recorder → .battleplan → replay app`.
Doctrine: fail-closed — anything the replay cannot reproduce exactly surfaces
as a typed blocker, never a silent approximation.

## Extractor (recording half) — implementation landed; current validation pending

- All 14 gameplay areas extracted at DONE-**base** slices — named residual
  functionality remains per the master's coverage table (market, naval,
  combat, intel, supply, event-hook residuals). 71 domains registered
  (`extractor/native/domain_registry.json` is code-authoritative).
- A production promotion tuple (A1/A2/A4/A12/A14/A15/A21/A22/B8-X6) plus
  A27/A35 has frozen promotion evidence. A32's display/language and authenticated
  selected-Present pointer-family path are integrated through d77; d80 is a
  deliberately non-independent reserved semantic ID folded into that manifest.
  Unsupported keyboard/controller traversal remains unavailable rather than
  fabricated.
- The P1–P5 product implementation has landed: the ordinary no-argument
  desktop-window route enters auto-attach, duplex activation is enabled,
  campaign identity is enabled for same-machine use, durable append-ticket
  stitching is called from production, and dormant stitch inputs are hardened.
  These statements describe implementation, not a current promotion receipt:
  central validation/refreeze for the present worktree is still pending. CID1
  M3 rehost on a second physical machine is the sole genuine external
  dependency. All other H-* matrices are optional nonblocking regression
  evidence and never activate product code. Unsupported B boundaries remain
  feature-local omitted and fail closed.
- Detail ledgers: `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
  (completion + backlog) and `extractor/documents/RE_REMAINING.md` (RE ledger,
  incl. the A24–A40 seam contracts).

## Replay layer (playback half) — laws complete for promoted scope, endgame gates open

- The rendering-laws RE program closed its full register: all XN01–XN17 non-UI
  packets and XUI01–XUI26 UI packets are PROVEN or DATA_BLOCKED
  (replay-side done, waiting only on extractor inputs — see
  `SEAM_CONTRACTS.md`). Board of record:
  `replay_interface/documents/lane_reports/rendering_laws/PROGRAM_STATUS.md`.
- XUI27 replay-owned terminal composition is implemented and has historical
  green evidence, but the current source set requires a fresh content-addressed
  refreeze. The adversarial release-review wave has never run. The headless F1
  reference hardware and frame-time contract are defined as optional
  performance evidence, not a product-wide gate. Optional runtime evidence stays
  fail-closed and is not a product-completion blocker. The 2026-07-19
  "REPLAY LAYER COMPLETE" finalization receipt
  (`replay_interface/re_evidence/rendering_laws/replay_promotion/REPLAY_CORE_FINALIZATION_20260719.md`)
  is **invalid as a freeze authority** — 3 of its 4 SHA256s drifted the same
  day; it must be reissued after the current fix wave.
- The robustness and container-hardening implementation wave has landed. Its
  current central validation/refreeze remains pending (list + state in
  `REMAINING_WORK.md` §2); historical green counts are not current authority.
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
The original 243-file `restore_gate_1` capture has been recovered byte-for-byte;
RG-H1 through RG-H4 remain optional discriminator/regression captures, not
production blockers. Pre-consolidation git history: bundle at
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
