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
