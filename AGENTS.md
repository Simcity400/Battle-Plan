# Agent guidance

Changes only with the owner's permission.

Read `PROJECT.md` before changing product behaviour, formats, or architecture.

## Conduct and workflow

- When asking questions to the owner, do not use any jargon and be concise; just make things very simple for them to understand and explain how the change would affect the end users UX, if at all.
- No backwards compat: delete replaced code in the same change, prove it dead.
- Code comments state only what the code cannot show (invariants, engine quirks, traps). No essays, no narration, no review notes in source — evidence docs carry that. Essay comments are a landing bounce (owner 2026-08-26).
- Fix what you find. Leave open only what needs the owner, a live game, or
  someone else's active work.
- Have Codex adversarially review work once you deem it ready to merge. Re-review every fix until it approves clean. You are allowed to push back on findings if you can prove they are wrong.
- Report once, numbers first, when EVERYTHING is finished. Reviewers get raw
  evidence, not summaries. Never end your turn to wait on a running task — an
  ended agent turn is never woken by a finishing task. Wait in-turn with
  TaskOutput (see Codex section). No status summaries, no partial findings
  mid-work; they bloat the orchestrator.
- Clean up your worktree, branch, build output, temp files. Never stash.
- Completion requires validated changes on pushed remote `main`; delete the task branch after landing.
- Hash-pinned committed files need `.gitattributes` `text eol=lf`.

## Tools

- Agents, subagents and lanes run NO tests and NO validation - not `cargo test`/`cargo nextest`, not `Invoke-Validation.ps1`, not the battery, not the citation verify/restamp scripts (owner directive 2026-08-24). All testing, validation AND compilation is done centrally by the orchestrator at landing (owner 2026-08-24: lanes run no cargo at all - a non-compiling commit comes back with the compiler error); a lane reports which tests the orchestrator should run.
- ONE exception (owner 2026-08-26): a lane runs the tests IT wrote or changed, once, before declaring converged - "your new test must have executed once." Scoped `cargo nextest run` on exactly those tests; full suites and everything else stay central.
- `-Mode Landing` (`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-Validation.ps1 -Mode Landing`) is the orchestrator's gate, run once on the merged tree at landing time; capture the exit code.
- Run `-Mode ReleaseManual -AllowReleaseManual` only when the owner explicitly requests release/manual validation.
- Never run a full battery or release/manual validation by default.
- `hoi4_re` is the single agent-facing RE facade; run `re_status` first, never edit its registry directly, and use its four-job offline queue.
- `candidate_census` and `cross_analyze_function` are discovery or second-opinion outputs, not proof or automatic promotion; the exact-build Ghidra broker remains the static authority. See the tool READMEs for details.
- Live instrumentation requires explicit owner authorization and an already-running matching game; one job at a time.
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
- Any long call (battery, Codex, capture) runs as ONE tracked background task
  (the Bash tool's background mode — NOT `&`, NOT Monitor; detached watchers
  die when your turn ends). For a Codex call: launch it tracked, read the
  event stream's first line and commit the thread id, then WAIT IN-TURN with
  the TaskOutput tool: `TaskOutput(task_id, block=true, timeout=600000)`,
  calling it again each time it returns with the task still running, until
  the task completes. Do NOT end your turn while the review runs: an ended
  agent turn is never woken by a finishing task, and the codex child can be
  reaped with the turn (three lanes lost their reviews this way, 2026-08-29).
  Never sleep-poll in a shell loop; a bounded long-interval wait loop is the
  fallback ONLY for a job the harness cannot track. The final report is still
  written exactly once, after everything is finished.
- Review loop: run the review, fix the findings, re-review. Keep looping until
  Codex approves clean - no round limit. Any commit after an APPROVE needs one
  more confirmation round.

## Orchestration - Only relevant if you are given the role of orchestrator by the owner

- Lanes run in worktrees, each with its own branch and build dir,
  locked with a start commit. A brief carries full context, the acceptance
  bar, the worktree as the only writable area, and this file.
- Orchestrator: verify approval and diff yourself, merge `--no-ff`, restamp
  citations if world_fold or docs changed, validate touched crates, push
  master, remove worktree + branch.
- Dead agent with a transcript: adopt its worktree, resume its reviewer thread.
- Batch owner questions; give options and a recommendation.
