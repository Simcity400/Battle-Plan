# Orchestration

How to run this project's agent workflow. AGENTS.md holds the rules every
agent follows; this file holds what the orchestrator needs. Nothing here may
live only in a session's memory — if a future orchestrator needs it, it
belongs in this file.

## The loop

Work is done by parallel worker agents ("lanes"), each in its own git
worktree under `.claude/worktrees/`, each with its own branch and its own
build directory. The orchestrator plans, briefs, verifies, merges, and pushes;
it does not implement. At most 10 lanes at once.

Every lane converges before reporting: its reviewer (Codex CLI) must approve
cleanly, its validation must pass in its own checkout, and its work must be
committed on its branch. Lanes never push or merge.

## Briefing a lane

A lane starts with no knowledge of the project. Its brief must carry:

- the task, its full context, and the acceptance bar;
- the worktree path and branch, named as the only writable area;
- pointers to AGENTS.md and the evidence it needs (paths, not summaries);
- the reviewer protocol (below) and the order to converge before reporting;
- the residual rule: fix what you find; only owner decisions, live-game
  needs, another active lane's territory, or out-of-scope items may be left.

## The reviewer (Codex CLI)

Full protocol: `C:\Users\Example\.claude\CODEX_PROTOCOL.md`. Essentials:
`codex exec --json -s danger-full-access -c approval_policy=never
--skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=high -o <file>`.
Never `--ephemeral`, never `resume --last`, never the priority service tier.
Persist every thread id immediately; resume only by thread id, flags before
`resume`. Read the reply from the `-o` file, require exit 0 and non-empty.
Re-review after every fix until a clean approve.

## Landing a finished lane

1. Verify the claims yourself: read the final approve in the lane's run
   directory and the key diff hunks.
2. `git merge --no-ff <branch>` in the main repo. Citation-marker conflicts
   in `wire_format/src/dNN.rs` are resolved by taking the side with current
   line numbers (usually HEAD), then restamping.
3. If `world_fold.rs` or documents changed: run
   `extractor/native/scripts/restamp_world_fold_citations.py`, then
   `restamp_citation_allowance.py`, then both verifiers
   (`verify_world_fold_citations.py`, `verify_citation_integrity.py`) —
   in that order, all must pass.
4. Run the lane's touched crates through the central validation script on the
   merged tree. Capture the exit code in its own step; never chain the push
   off anything but that code.
5. Push the integration branch and `HEAD:master` together, after fetching and
   comparing both origin refs.
6. Remove the worktree and delete the branch. This also reclaims its build
   directory (~10–16 GB each).
7. Update the task ledger; queue any owner decisions for a single batched
   ask.

## When a gate or agent hangs

A validation that sits idle for 15+ minutes has one of three causes — check
in this order:

1. The validation mutex is held by another run (the log says "already
   active"). Wait; it's serialized by design.
2. Orphaned cargo processes from a killed session hold the build-directory
   file lock. Cargo reports this only on stderr, so logs look silent.
   Diagnostic: cargo processes hours old with ~0 CPU and no compiler
   children. Kill them by specific PID only — never by process name.
3. A single test binary is hung (known: the inline_detour freeze test).
   Diagnostic: one test exe with ~0 CPU under the runner. Kill it; the run
   continues and reports that test failed — re-run it isolated to judge.

Before every landing gate, sweep for stale cargo/test processes.

## When an agent can't be resumed

Session kills sometimes leave an agent unresumable even though its transcript
file is intact. Its work is safe in its worktree. Launch a replacement agent
with no new worktree, instructed to adopt the existing worktree and branch,
reconstruct state from the run directory and git, resume the same reviewer
thread, and finish to the same bar.

## Standing constraints (owner)

- Lane cap: 10.
- Codex priority tier: off.
- Capture is attach-and-play; nobody stages client interactions (AGENTS.md).
- Perfect capture comes before recording size.
- Batch owner questions; ask with options and a recommendation.
