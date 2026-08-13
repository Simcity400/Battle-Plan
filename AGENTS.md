# Agent guidance

## Goals

Perfect clean captures, small recordings, straight into replay, minimal
performance hit. A bad hour costs that hour with a named reason — never the
recording.

## Rules

- Never make the game wait, for anything.
- Capture is attach-and-play. Nobody interacts with the client to help the
  recorder. Find where data lives by reverse engineering and read it
  passively; if that's impossible, derive it at replay or refuse it with a
  named reason.
- Fail closed: unproven values are refused, named. Never publish data under
  the wrong hour.
- No hardcoded game content. Constants and caps need proof. Mods must work.
- Assume max game speed and a modest 4-core machine.
- No backwards compatibility yet; delete replaced code in the same change,
  with proof it's dead.
- Fix what you find. Leave something open only if it needs the owner, a live
  game, or belongs to someone else's active work.
- Do your own work; the only child agent allowed is one read-only reviewer,
  after the work is done. Re-review after every fix until it approves clean.
- Report once, when finished, leading with the deciding numbers. Reviewers
  get raw evidence, not summaries.
- Clean up after yourself: worktree, branch, build output, temp files.
- Never git stash. Check against origin before pushing.

## Tools

- Validation: run **your own checkout's**
  `extractor\native\scripts\Invoke-CentralRustValidation.ps1` (`-Mode Final
  -Workspace All` for the full battery; narrower modes exist). One runner at
  a time. Always capture the exit code itself.
- Ghidra (static) and `hoi4_re` (live) are read-only authorities — check
  status first, search before exporting, record what you conclude. Guides:
  `replay_interface/tools/ghidra/README.md`, `replay_interface/tools/hoi4_re/README.md`.
- Evidence goes in `extractor/native/runs/<your-run>/`, append-only.

## Orchestration

For whoever runs the workflow. Nothing durable lives only in a session's
memory — if a future orchestrator needs it, it goes here.

- Work runs in parallel worker agents ("lanes"), each in its own worktree
  under `.claude/worktrees/` with its own branch and build dir. The
  orchestrator briefs, verifies, merges, pushes — never implements.
- A lane's brief must carry everything: full context, acceptance bar, the
  worktree as the only writable area, pointers to this file and to raw
  evidence, the reviewer protocol, and the residual rule.
- Reviewer is Codex CLI (protocol: `C:\Users\Example\.claude\CODEX_PROTOCOL.md`):
  `codex exec --json -s danger-full-access -c approval_policy=never
  --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=high -o <file>`.
  Never `--ephemeral`, never `resume --last`, never the priority tier. Persist
  thread ids immediately; resume by id with flags before `resume`; read the
  `-o` file, require exit 0 and non-empty.
- Landing a finished lane: verify its approval and diff yourself → merge
  `--no-ff` → if `world_fold.rs` or docs changed, run
  `restamp_world_fold_citations.py` then `restamp_citation_allowance.py` then
  both verify scripts, all green → run the touched crates through validation
  on the merged tree, read the exit code in its own step → push integration
  and `HEAD:master` together after comparing origin → remove worktree, delete
  branch → update the ledger. Citation-marker merge conflicts: take the
  current-tree line numbers, restamp.
- A gate idle 15+ minutes is one of: the serialized validation mutex (wait);
  orphaned cargo processes holding the build-dir file lock (old, ~0 CPU, no
  compiler children — kill by specific PID, never by name); or a hung test
  binary (kill it, re-run that test isolated). Sweep for stale processes
  before every landing gate.
- If an agent can't be resumed but its transcript exists, its work is safe in
  its worktree: launch a replacement that adopts the same worktree and
  branch, reconstructs state from the run dir and git, resumes the same
  reviewer thread, and finishes to the same bar.
- Batch owner questions; ask with options and a recommendation. Perfect
  capture outranks recording size.
