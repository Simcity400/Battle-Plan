# Agent guidance

Changes only with the owner's permission.

Read `PROJECT.md` before changing product behaviour, formats, or architecture.

## Conduct and workflow

- When asking questions to the owner, do not use any jargon and be concise; just make things very simple for them to understand and explain how the change would affect the end users UX, if at all.
- No backwards compat: delete replaced code in the same change, prove it dead.
- Fix what you find. Leave open only what needs the owner, a live game, or
  someone else's active work.
- Have Codex adversarially review work once you deem it ready to merge. Re-review every fix until it approves clean. You are allowed to push back on findings if you can prove they are wrong.
- Report once, numbers first, when EVERYTHING is finished. Reviewers get raw
  evidence, not summaries. While a tracked task is still running, end your
  turn with a single line ("WIP: waiting on <task>") and nothing else — no
  status summaries, no partial findings; they bloat the orchestrator.
- Clean up your worktree, branch, build output, temp files. Never stash.
- Completion requires validated changes on pushed remote `main`; delete the task branch after landing.
- Hash-pinned committed files need `.gitattributes` `text eol=lf`.

## Tools

- Agents, subagents and lanes run NO tests and NO validation - not `cargo test`/`cargo nextest`, not `Invoke-Validation.ps1`, not the battery, not the citation verify/restamp scripts (owner directive 2026-08-24). All testing and validation is done centrally by the orchestrator at landing. A lane may run `cargo check -p <crate>` on the crates it touched, in a lane-private `CARGO_TARGET_DIR`, so its commit compiles; it reports which tests the orchestrator should run.
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
- Any long call (battery, Codex, capture) runs as ONE tracked background task,
  and you STAY ACTIVE and wait for it with the Monitor tool (block on its
  output file / exit condition; if the wait times out, Monitor again). Never
  sleep-poll in a shell loop, never detach with `&`, and never end your turn
  while your work is unfinished — you end your turn exactly once, with the
  final report.
- Review loop: run the review, fix the findings, re-review. Keep looping until
  Codex approves clean - no round limit. Any commit after an APPROVE needs one
  more confirmation round.

## OpenCode runner - the implementation lane runtime

`opencode` (npm `opencode-ai`) is on the Windows PATH. Owner 2026-08-24: implementation
lanes run on OpenCode (`-Agent build`) instead of Opus subagents. It is never a reviewer;
Codex stays the only adversarial reviewer, and the LANE runs that loop itself from its
own shell until a clean APPROVE (the orchestrator never reviews lane work; it touches
Codex only for its own edits). Start the wrapper from the
lane's locked worktree (it runs in the current directory; there is no --dir). The prompt
carries everything: worktree path, job, proof rules, the no-test rule above, report shape.

```powershell
$Result = & .\tools\reviewers\Invoke-OpenCodeReview.ps1 -Prompt $Prompt -RunDir $RunDir
if ($Result.ExitCode -ne 0 -or -not $Result.ReplyText) { throw 'opencode job failed' }
```

- Defaults are fixed and not to be changed silently: `opencode/x-preview-f-free
  --variant max` (Ox Alpha, max effort, owner 2026-08-21) and the read-only `plan`
  agent. The script rejects an effort the model does not declare - the CLI drops an
  unknown one without erroring.
- Success = `ExitCode -eq 0` AND a non-empty `ReplyText`. Retry once: jobs fail
  transiently while the OpenCode desktop app holds the same database.
- OpenCode halts early a lot. The script sends `continue` into the same session until
  the turn ends cleanly, capped by `-MaxContinues` (20); `$Result.Continues` reports
  how many it needed. Nothing to do by hand.
- Session id lands in `<RunDir>\opencode_session.txt` on the first event. Resume by
  id only - `-SessionId $Result.SessionId`, never `--continue`.
- A lane that stops early (empty reply, no commit, `step_finish reason=unknown`) is NOT
  dead: re-prompt the SAME session id with "continue" (again if needed). Never start a
  new session for a stalled lane (owner 2026-08-24). Keep its inputs small: slice reads
  (grep -n, then a window), no whole files over ~1000 lines, commit after every step.
- Prompt rules are Codex's: no project memory, so the prompt carries everything.

## Orchestration - Only relevant if you are given the role of orchestrator by the owner

- Lanes run in worktrees, each with its own branch and build dir,
  locked with a start commit. A brief carries full context, the acceptance
  bar, the worktree as the only writable area, and this file.
- Orchestrator: verify approval and diff yourself, merge `--no-ff`, restamp
  citations if world_fold or docs changed, validate touched crates, push
  master, remove worktree + branch.
- Dead agent with a transcript: adopt its worktree, resume its reviewer thread.
- Batch owner questions; give options and a recommendation.
