# Agent guidance

Changes only with the owner's permission.

## Goal

Perfect clean captures, small, straight into replay, minimal perf hit.
A bad hour costs that hour, with a named reason — never the recording.

## Rules
- When asking questions to the owner, do not use any jargon and be concise; just make things very simple for them to understand and explain how the change would affect the end users UX, if at all.
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
- Have Codex adversarially review work once you deem it ready to merge. Re-review every fix until it approves clean. You are allowed to push back on findings if you can prove they are wrong.
- Report once, numbers first, when EVERYTHING is finished. Reviewers get raw
  evidence, not summaries. While a tracked task is still running, end your
  turn with a single line ("WIP: waiting on <task>") and nothing else — no
  status summaries, no partial findings; they bloat the orchestrator.
- Clean up your worktree, branch, build output, temp files. Never stash.
- Hash-pinned committed files need `.gitattributes` `text eol=lf`.

## Tools

- Default: `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-Validation.ps1`; capture the exit code.
- Default mode runs affected shipping-product tests and validators only when their inputs changed; tools manage execution with no repository concurrency cap.
- `-Mode Landing` is required immediately before landing and runs all shipping-product tests.
- `-Mode ReleaseManual -AllowReleaseManual` is allowed only when the owner explicitly requests it.
- Unknown change impact falls back to all shipping-product tests.
- Never run the legacy/full battery or release/manual mode by default.
- Ghidra and `hoi4_re` are read-only authorities (READMEs under `replay_interface/tools/`).
- Evidence: `extractor/native/runs/<run>/`, append-only.

## Codex reviewer - Only relevant for Claude agents. Ignore if a Codex agent

`codex` is on the Windows PATH. Run reviewer calls from PowerShell:

```powershell
$OutPath = Join-Path ([IO.Path]::GetTempPath()) ("cdx_{0}.txt" -f [guid]::NewGuid().ToString("N"))
$EventsPath = Join-Path ([IO.Path]::GetTempPath()) ("cdx_{0}.jsonl" -f [guid]::NewGuid().ToString("N"))
$Prompt | & codex exec --json -s danger-full-access -c approval_policy=never `
  --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=high `
  -o $OutPath - | Set-Content -LiteralPath $EventsPath -Encoding utf8
$ReviewExitCode = $LASTEXITCODE
```

- Flags are mandatory as written. Never `--ephemeral`, never `resume --last`,
  never `service_tier=priority`, never a sandboxed `-s` mode (broken on this
  machine). Generate unique files under `[IO.Path]::GetTempPath()`; Bash and
  WSL are not required.
- Persist the `thread_id` (first line of `$EventsPath`) to your run dir immediately.
- The reply is `$OutPath`, never stdout. Success = `$ReviewExitCode -eq 0` AND
  a non-empty `$OutPath`. Retry once — except on usage-limit/credit signatures
  (check `$OutPath`, stderr, and the final `$EventsPath` events, not substrings
  mid-stream): queue the thread id
  and resume it after limits reset instead of retrying.
- Resume by id only: `codex exec -s danger-full-access resume $ThreadId …`
  (`-s` sits before `resume`; same model/effort flags).
- Codex has no project memory: the prompt carries the full context, the closed
  defect class, an out-of-scope list, raw evidence paths, and "this job is
  read-only — do not modify files". Feed it via stdin with a trailing `-`.
  Forbid it from consulting any machine-wide memory store.
- Any long call (battery, Codex, capture) runs as ONE tracked background task,
  and you STAY ACTIVE and wait for it with the Monitor tool (block on its
  output file / exit condition; if the wait times out, Monitor again). Never
  sleep-poll in a shell loop, never detach with `&`, and never end your turn
  while your work is unfinished — you end your turn exactly once, with the
  final report.
- One fix round per review; if the re-review still finds defects, fix the rest
  yourself. Any commit after an APPROVE needs one more confirmation round.

## Orchestration - Only relevant if you are given the role of orchestrator by the owner

- Lanes run in worktrees, each with its own branch and build dir,
  locked with a start commit. A brief carries full context, the acceptance
  bar, the worktree as the only writable area, and this file.
- Orchestrator: verify approval and diff yourself, merge `--no-ff`, restamp
  citations if world_fold or docs changed, validate touched crates, push
  master, remove worktree + branch.
- Dead agent with a transcript: adopt its worktree, resume its reviewer thread.
- Batch owner questions; give options and a recommendation.
