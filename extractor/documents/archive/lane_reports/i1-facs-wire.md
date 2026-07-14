# i1 FACS wire lane report

## Verdict

**PASS offline.** D53 now distinguishes an unavailable faction roster from a
double-read-stable measured-empty roster without changing old-stream semantics.
Live validation of FACS v2 in a real injected capture remains user-gated and is
not claimed here. The orchestrator still owes the independent adversarial diff
review required by `documents/LANE_WORKFLOW.md`.

## Mission and changes

- Collector/runtime read: `lanes/diplomacy_intel_operatives_live/src/extractor.rs:1297`
  returns `None` for unreadable, malformed, or unstable roster topology and
  `Some(rows)` only after re-reading the faction-system header and every counted
  member topology. Stable `Some(Vec::new())` sets the positive completeness
  witness at `extractor.rs:959-965`. The delta emitter preserves the witness at
  `collector/src/lib.rs:21080`.
- Wire/schema: `domain44_compact.rs:180-188` retains legacy FACS v1 and adds
  self-versioned FACS v2. V2 is emitted with its declared row count even when
  zero; the decoder accepts v1 non-empty and v2 zero/non-empty, while no trailer
  remains unwitnessed (`domain44_compact.rs:1226-1241,1743-1858`). The H4CR
  record envelope remains compact-contract version 1; this is a payload-trailer
  version addition, not a silent framing change.
- Fold: `recorder_host/src/world_fold.rs:11057-11068` maps witness+0 to
  `Measured { is_empty: true }`, witness+N to measured non-empty, and no witness
  to unavailable. Existing JSON projection emits `measured_empty` and an empty
  `faction_rows` list only for the witnessed state.
- Regressions: `recorder_host/src/world_fold/tests.rs:995-1118` proves all three
  JSON states and proves an old payload without FACS remains unavailable.
  D53 codec/reader tests prove v1 compatibility, v2 zero rows, stable empty, and
  torn-header witness suppression.

## RED evidence

The recorder regression was added first. Its initial targeted build failed with
four `E0609` errors because neither `ExtractedDomain53Rows` nor
`DiplomacyIntelSnapshot` had `faction_roster_complete`. No production file had
been changed at that point.

## Gates

- `cargo build`: PASS.
- `cargo test -q --no-fail-fast -p collector -p recorder_host -p compact_contract
  -p diplomacy_intel_operatives_live`: PASS.
  - collector: baseline 357 passed / 6 ignored; final 357 / 6 (unchanged).
  - compact_contract: baseline 29 / 0; final 29 / 0 (unchanged).
  - recorder_host: baseline 400 / 2; final 401 / 2 (+1 net regression).
  - diplomacy_intel_operatives_live: final 117 / 0 (+3 regressions; 114 baseline).
- Targeted `cargo test -p recorder_host faction_roster`: PASS.
- Targeted old-format/no-witness fold test: PASS.
- Targeted `cargo test -p diplomacy_intel_operatives_live faction`: 15 passed.
- `rustfmt --edition 2021` was run on every touched Rust file. Unrelated
  pre-existing full-file formatting drift was pruned; direct `--check` passes on
  the compact codec and recorder fold/test files that own the substantive diff.
- `git diff --check`: PASS.
- Commit: BLOCKED by the managed sandbox. `git add` could not create
  `.../.git/worktrees/i1-facs-wire/index.lock` (`Permission denied`), so all
  changes remain unstaged for the orchestrator.

## Correctness argument, NULLs, and residual risks

- Positive empty is never inferred from `rows.is_empty()`: only decoded FACS
  completeness selects measured state. A failed second read suppresses both rows
  and witness, so uncertainty cannot become absence.
- Refuted design: treating the static source-proof bit or a zero row vector as a
  runtime completeness witness. Neither proves the current live read completed.
- Old streams with no FACS trailer retain unavailable semantics; legacy v1
  non-empty trailers retain measured-nonempty semantics.
- Remaining risk: the new v2 witness has only synthetic/fake-memory and offline
  stream validation. A real advancing-game capture must confirm v2 emission and
  decode for both ordinary non-empty play and an actually zero-faction world.
- No live game, process-memory session, or injected DLL capture was available in
  this sandbox. No live closure is claimed.
