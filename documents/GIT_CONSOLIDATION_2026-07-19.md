# Git consolidation record — 2026-07-19

This is the compact preservation record for the branch/worktree clean break.
It is not a project-status or rendering-law master list.

## Preservation archives

- Full Git history: `C:\tmp\hoi4-preconsolidation-20260719.bundle`
  - 625 refs, verified by `git bundle verify`
  - SHA-256 `d330d72fc7d660d32da86176ad03a11f93bb9db68c2d82fad835221ce410fa98`
- Ten genuinely dirty worktrees, excluding rebuildable compiler products:
  `C:\tmp\hoi4-dirty-worktrees-20260719.7z`
  - 32,342 files / 9,237,056,406 bytes, archive test passed
  - SHA-256 `1b9e8bc0843b2b5d08aa71ea769d7454bbfd7c7a6c94fe9d51291093eb43c3d9`

## Content imported without unsafe ancestry

- Pass-final readback authority verifier, tests, and evidence report.
- Extractor coverage-atlas/schema tools and tests, updated to the current
  446-field replay schema and portable output paths.
- Furniture miner, fixtures, test, report, and retained renderer module.
- Localisation-join coverage tool/dossier and recording/map evidence tests.
- Historical RenderDoc and Ghidra regression gates from the maximal unmerged
  tips. Ten modules tied to branch-local evidence/configuration are preserved
  under `replay_interface/re_evidence/historical_branch_regressions/` with
  non-collected names; the compatible gates remain active.
- Selective W20 fail-closed laws: no TerrainID substitution, no unproven
  terrain-category proxy, no fabricated river/tree geometry producers,
  dimension-gated terrain LOD, and capture-proven faction carrier bytes.

## Explicitly superseded or evidence-only

The following were archived but not promoted: synthetic W16 UI navigation,
fixed-capture sea-colour diagnostics, frozen vanilla resource/LUT hydration,
the parentless replay-layer snapshot (tree-identical to `70188b94`), blocked
pass-final WIP code, conflicting legacy `replay_app::map_modes` variants, and
unproven/fitted river, tree, resource-fill, or mapmode implementations.

## Cleanup and push constraints

- Rebuildable Cargo targets and test temp directories were removed.
- Accidentally tracked `scientists_live_lane/target_certification` compiler
  outputs were removed; its durable `invalid_stream_noncert.json` receipt was
  retained.
- Three required evidence blobs over GitHub's 100 MiB object limit are tracked
  with Git LFS.
- Historical branch ancestry is not merged wholesale because it contains a
  removed 769.38 MiB Ghidra database blob. The verified bundle is the authority
  for that history; `master` receives reviewed final content only.

## Consolidated lineage

- Newer local baseline: `8ec852cfab822904609b344b5bf99302cafd97db`.
- Audited dirty-tree snapshot: `b0d26bd55412296fa564bcdebba659cee289d441`.
- First consolidated commit on the baseline: `b0dbf0fe`.
- The canonical branch is `master`; the temporary local `main` integration
  name is removed during final branch cleanup and is never pushed.
