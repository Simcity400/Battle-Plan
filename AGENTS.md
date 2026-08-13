# Repository agent guidance

## Product law (owner, 2026-08-11)

- Four pillars — everything else is cost: perfect clean captures; smallest
  pre-compression size with ≤50 MB post-compression per recording
  (Discord-shareable); plug straight into the replay layer at full fidelity;
  minimal game/computer performance hit. Dead-simple UX.
- One bad hour costs exactly that hour, with a named reason — never the recording.

## Capture law

- NEVER make the engine wait: no pausing, no synchronous in-hook capture, no
  clone/freeze tricks. This recorder is a multiplayer host.
- The hot path is copying bytes. Discovery runs once, invalidated only by proven
  change signals; validation runs post-bracket on the frozen copy. Per-hour
  rediscovery is a defect class.
- Fail-closed: unproven values are refused with a named wire reason. Hour
  misattribution is the owner stop condition. Zero-loss-typical is the goal;
  guaranteed zero is impossible and must not be claimed.
- Max speed is the normal case (48 ms sweep budget vs the 56 ms free-run hour).
  No scaling by worker threads. Ship floor: 4 cores / 16 GB.
- Mod-agnostic, no exceptions: no hardcoded game content; engine constants need
  RE proof; capacity caps are proven engine-structural or discovered at attach;
  overflow refuses loudly.

## Code hygiene

- Pre-production: NO backwards compatibility. Old captures are disposable test
  data; formats change cleanly.
- Ruthless deletion: retiring an architecture deletes the old path in the same
  wave (or leaves a dated deferred entry with exact ranges). Deadness needs
  positive proof — this repo has burned on all five hidden-reach mechanisms
  (Rust spawning Python, dot-sourced PS1 libs, sys.path injection, relative
  imports, constructed data paths). Baseline the workspace suite before purges
  and reconcile pass counts.
- Docs read as current intent only. Evidence under `extractor/native/runs/` is
  append-only; never rewrite a completed run or job directory.
- Tests carry a one-line `kills:` comment naming the production change that
  fails them. Empirical mutation batteries are banned by default. Before
  deleting a test, prove it has no distinct contract value.

## Working method (learned on this project)

- **Branches and worktrees never linger**: retire the worktree AND `git branch -d`
  the moment a lane's merge is pushed. Lane start: lock the worktree + make an
  empty commit (auto-prune deletes trees with no tracked changes), and verify
  `merge-base` against the current origin tip before any work.
- Git: NEVER `git stash` (shared repo — the stack holds other sessions' WIP);
  fetch and compare both master refs before any lockstep push — a refused sync
  is the divergence detector, reconcile by merging; never commit a file >99 MB.
- Own your review loop: converge to a clean approval before reporting, report
  ONCE at completion. Never end a turn while a review/gate/suite child runs
  (ending the turn kills it); never poll a child — bound the call, kill on
  timeout, report the hang as a finding.
- Publish the deciding number/table FIRST, before any theory or fix. Reviewers
  get the raw evidence paths, not summaries.
- Measurement: state the basis (per-invocation vs per-frame vs per-record — a
  19.6× basis error once inverted a verdict); A/B inside one process only,
  cross-session ns comparisons are void; re-measure a cost baseline on the
  current tip before acting on it; a loaded machine flatters completeness —
  take the free-run baseline as the validity control.
- Negatives: prove absence with `git grep <pat> HEAD -- <scope>`; "could not
  determine X" is a finding that names the one step that would close it, never
  a conclusion.
- Every discovered defect gets FIXED IN THE LANE THAT FOUND IT (owner directive
  2026-08-13). A residual is legitimate ONLY when it is (a) an owner decision,
  (b) needs a live capture/game session or the owner present, (c) inside
  another ACTIVE lane's territory (hand over with exact state), or (d) outside
  the lane's authorized scope. "Unstaffed", "future lane", "next session", or
  "too far from my brief" are NOT valid residual reasons for something the
  lane could fix — fix it, gate it, and have it re-reviewed before reporting.
  Never call anything complete with open residuals; each residual names its
  qualifying category and owner.
- Max 10 concurrent implementation lanes (owner 2026-08-13); queue beyond that. Lanes
  already in flight when the cap changes finish naturally.

## Agent delegation

- Implementation-lane agents do all their own research, RE, implementation,
  testing, and fixes — no child agents, with one exception: exactly one
  read-only cold reviewer, started only after the candidate is complete and
  wrapper validation is green. Findings return to the same lane; after fixes
  and renewed green validation the same sole reviewer is reused.
- Reviews converge to a clean approval; re-review after EVERY fix, including
  comment/doc fixes. Never end a turn while a review or gate child is running.

## Ghidra broker authority

- Call `ghidra_status` first. The exact-a729 broker reports `ready=true`, 20
  shards ready, read-only/no-analysis, image base `0x140000000`, SHA-256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Never reconfigure or reset the broker; historical ten-shard receipts describe
  a closed one-time replay, not the current topology. Shard count is capacity,
  not evidence provenance — validate a job by binary identity, mode, job ID,
  and hashes, never by shard census.

## Ghidra evidence: query the index before exporting

- Find: `ghidra_coverage`, `ghidra_search_decompiled`, `ghidra_functions_for`,
  `ghidra_subsystems_of`. Read: `ghidra_get_function`. Traverse:
  `ghidra_callers`/`ghidra_callees`/`ghidra_path_between`. Export
  (`ghidra_export_functions`) only after a query shows the address is unindexed
  — re-exporting indexed evidence is the biggest waste here, and exports pull
  direct callers even at depth 0.
- Record conclusions with `ghidra_annotate_function` before calling an
  investigation done — the index saves Ghidra's output automatically but not
  your analysis; an unrecorded investigation gets paid for again.
- The index is advisory (build-time state). `ghidra_verify` is the only
  cache-vs-disk check: cite its reported identity before publishing a claim.
- Never rewrite artifacts inside a completed job directory (hardlink-dedup —
  an in-place edit corrupts every citing job). Derive into a new job.
- Operating guide: `replay_interface/tools/ghidra/README.md`.

## Unified RE tooling (`hoi4_re` facade)

- Start with `re_status`. Use it for shared build/symbol lookup, patch
  migration, registered live-memory observations, bounded Frida traces,
  constrained emulation, save/memory correlation. It does not replace the
  Ghidra authority above.
- New-build analysis runs only in isolated writable projects; never open an
  authority shard read-write. `observe_runtime` is read-only; `trace_calls` is
  hook-based and never automatic promotion evidence.
- Operating guide: `replay_interface/tools/hoi4_re/README.md`.

## Rust builds and tests

- Covers `extractor/native`, `replay_interface/native`, `portable_replay`.
- Primary agent runs Rust validation once, centrally and serially — never
  invoke Cargo directly or start competing runners:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Final -Workspace All
```

- Narrower runs: `-Workspace Extractor|Replay`, `-Mode Check|Build|Fast|Integration|Test`
  (accept package/target selectors; don't broaden a targeted check). `Final` is
  the complete deterministic offline battery; `Soak`/`Evidence` only when that
  evidence is required and available.
- Default-tier tests: deterministic, distinct production invariant, no game
  install/capture corpus/live process/sleep/soak. Evidence-dependent tests go in
  the soak/evidence tiers and never silently pass when evidence is absent.
- `-CacheMode Iterative` for repeated edits, `Reusable` for clean/worktree
  builds, `-Timings` for an HTML timing report. Shared target dir is
  `.cargo-target` (do not override; first build is cold).
- Gate discipline: capture the cargo exit code before any pipe.
- Test-quality audit: `python extractor/native/scripts/Audit-RustTests.py`.
