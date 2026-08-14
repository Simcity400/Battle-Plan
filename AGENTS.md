# Agent guidance

Changes only with the owner's permission.

## Goal

Perfect clean captures, small, straight into replay, minimal perf hit.
A bad hour costs that hour, with a named reason — never the recording.

## Rules
- When asking questions to the owner, do not use any jargon and be concise; just make things very simple for them to      understand and explain how the change would affect the end users UX, if at all.
- Never make the game wait.
- Attach-and-play: find data by RE and read it passively; else derive at
  replay or refuse it, named.
- Fail closed. No hardcoded game content. Mods must work.
- Derive authored content and all presentation at replay from the certified
  installed content and recorded state; extract only what changes during play — and
  where an authored value can change, extract the divergence, not a copy.
- Assume max game speed on a modest 4-core machine.
- No backwards compat: delete replaced code in the same change, prove it dead.
- Fix what you find. Leave open only what needs the owner, a live game, or
  someone else's active work.
- One child agent only: the Codex reviewer below, after the work. Re-review
  every fix until it approves clean.
- Report once, numbers first, when EVERYTHING is finished. Reviewers get raw
  evidence, not summaries. While a tracked task is still running, end your
  turn with a single line ("WIP: waiting on <task>") and nothing else — no
  status summaries, no partial findings; they bloat the orchestrator.
- Clean up your worktree, branch, build output, temp files. Never stash.
- Generated runtime inputs are not debris: prove regeneration before purging.
- Hash-pinned committed files need `.gitattributes` `text eol=lf`.

## Tools

- Validation: your own checkout's
  `extractor\native\scripts\Invoke-CentralRustValidation.ps1`; capture the exit code.
- Ghidra and `hoi4_re` are read-only authorities (READMEs under `replay_interface/tools/`).
- Evidence: `extractor/native/runs/<run>/`, append-only.

## Codex reviewer

`codex` is on the Bash PATH. Every call, exactly:

```bash
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
printf '%s' "$PROMPT" | codex exec --json -s danger-full-access -c approval_policy=never \
  --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=high -o "$OUT" - > "$AJ"
```

- Flags are mandatory as written. Never `--ephemeral`, never `resume --last`,
  never `service_tier=priority`, never a sandboxed `-s` mode (broken on this
  machine). Use `mktemp` for `-o` — a hand-written path with brackets is eaten
  by bash as a redirection.
- Persist the `thread_id` (first line of `$AJ`) to your run dir immediately.
- The reply is `$OUT`, never stdout. Success = exit 0 AND non-empty `$OUT`.
  Retry once — except on usage-limit/credit signatures (check `$OUT`, stderr,
  and the final `$AJ` events, not substrings mid-stream): queue the thread id
  and resume it after limits reset instead of retrying.
- Resume by id only: `codex exec -s danger-full-access resume "$TID" …`
  (`-s` sits before `resume`; same model/effort flags).
- Codex has no project memory: the prompt carries the full context, the closed
  defect class, an out-of-scope list, raw evidence paths, and "this job is
  read-only — do not modify files". Feed it via stdin with a trailing `-`.
  Forbid it from consulting any machine-wide memory store.
- Any long call (battery, Codex, capture) runs as ONE tracked background task
  via the shell tool's background option — the harness wakes you when it
  finishes. Never detach with `&`, never sleep-poll in a loop, and never end
  your turn "waiting" for anything that is not a tracked background task:
  untracked children die the instant your turn ends, and no result ever comes.
- One fix round per review; if the re-review still finds defects, fix the rest
  yourself. Any commit after an APPROVE needs one more confirmation round.

## Orchestration

- Lanes run in worktrees under `.claude/worktrees/`, own branch and build dir,
  locked with a start commit. A brief carries full context, the acceptance
  bar, the worktree as the only writable area, and this file.
- Orchestrator: verify approval and diff yourself, merge `--no-ff`, restamp
  citations if world_fold or docs changed, validate touched crates, push
  master, remove worktree + branch.
- Gate stuck 15+ min: mutex wait, orphaned cargo (kill by PID, never name),
  or hung test binary.
- Dead agent with a transcript: adopt its worktree, resume its reviewer thread.
- Batch owner questions; give options and a recommendation.
