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

## Post-consolidation archaeology (2026-07-19, replay-layer orchestrator)

A full disposition of the ~756 unreachable commits, 3 dropped stashes, the
8 uncommitted-work patches in `C:\Users\Example\hoi4_worktree_uncommitted_backup\`,
and the 12-lane `EXTRACTOR_SESSION_UNSHIPPED_WORK.md` (2026-07-14) table was
run after consolidation. Verdict: the consolidation is trustworthy — the
amend-predecessor snapshot, all 2026-07-16 extractor feature tips, 7 of 8
backup patches, all 12 unshipped-work rows (incl. auto_attach and stitching),
and the pre-07-15 lane pool are subsumed by `master` (symbols verified present,
often relocated into shared crates such as `compact_contract`).

**Genuinely stranded work, preserved as local rescue branches (do not prune):**

| Branch | Source | Content | Action owed |
| --- | --- | --- | --- |
| `rescue/weather-leaf-soundness` | stash `7e499eca` (port/w1-atlas) | coverage_audit open-vocab weather-leaf fail-closed fix (`_owed_open_vocab_weather_leaf`, `_WEATHER_SUBCONTAINERS`) + 3 tests. **Master's rewritten `save_oracle.py` normalizes `weather.regions.*.<flag>` to the `*.*` blanket this fix exists to reject — possible soundness regression.** | Run the 3 rescued tests against master's oracle; port the fix if they fail. |
| `rescue/acceptance-telemetry-f5` | commit `3ea756d8` (2026-07-16) | Recorder-side F5 telemetry (`WriteLatencyHistogram`, `ClockSkewSample`, percentile summaries + tests) — absent from master's Rust (PS-side ladder F6 did ship). | Port or explicitly descope. |
| `rescue/replay-layer-checkpoint` | tip `7f68bebd` (claude/replay-layer) | The pre-consolidation granular game_ui architecture (~20 per-domain `*_binding.rs`/`*_panels.rs`, `compositor*.rs`) + the 15-file `replay_interface/audit/` mutation-test harness. Master's consolidated game_ui covers the same domains; functional parity not proven by archaeology. | Owner parity review of game_ui laws + decide whether to port the mutation harness; then drop. |

Superseded-but-kept: `worktree_uncommitted_backup\agent-add8ba76*.patch`
(transitional CCSS scan-audit scaffold; the shipped anchored-walk supersedes it).

**History safety net:** the pre-consolidation bundle now exists in TWO
locations — `<local-path>`
(durable copy, `git bundle verify` OK 2026-07-19) and
`C:\tmp\hoi4-preconsolidation-20260719.bundle` (original; expendable).
`refs/codex/turn-diffs/checkpoints/*` (11 refs) point at
working-tree snapshots, not commits; low value, safe to ignore.

**Off-repo orchestration state a future agent may need** (all machine-local
evidence was physically consolidated 2026-07-19 under **`C:\hoi4_evidence\`**
— see its `README.md` manifest; junctions remain at every pre-consolidation
path): `<local-path>` (lane logs, verdicts, Codex thread ids in
`threads/*.tid`, `FIX_LIST.md` handoff snapshot, wave boards) and
`<local-path>` (07-08 patch set).
