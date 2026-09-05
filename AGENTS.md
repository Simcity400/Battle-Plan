# Agent guidance

Do not change `AGENTS.md` or `PROJECT.md` without the owner: propose the diff and wait. Read `PROJECT.md` before changing product behaviour, formats, or architecture. Every harness reads this file. Read "Everyone", then only the parts that name your role and your harness.

Roles. The **orchestrator** is the one agent the owner has explicitly named orchestrator this session; nobody assumes the role. A **lane** is an agent with its own worktree and branch that implements one scoped piece of work end to end. The orchestrator is the only reviewer.

## Everyone

- Questions to the owner: plain words, no jargon, say what the end user sees. The orchestrator batches them and gives options with a recommendation.
- No backwards compatibility. Delete what you replace in the same change and prove it dead with `git grep` from the repo root. A grep from a subdirectory proves nothing.
- Code comments state only what the code cannot show: invariants, engine quirks, traps. No essays, no history.
- Evidence is data: raw artifacts, JSON or TSV, asserting tests. One short `POINTER.md` per run dir under `extractor/native/runs/<run>/`. Claims live as tests or structured data, never prose.
- No residuals. Fix what you can now. Never park work in a task list. What you legitimately cannot fix (owner decision, live probe, missing capture, another lane's file) goes in your final report with its class and the exact artifact path.
- Report once, when everything is finished, numbers first: tip, what landed, tests run, review record, residuals.
- Git: never resolve a conflict with `git checkout --ours` or `--theirs`; it replaces the whole file. Resolve hunks by hand. Never stash. Hash-pinned files carry `.gitattributes` `text eol=lf`. The repo is CRLF: source-scanning tests use `str::lines()`, never raw `\n`.
- Build: sccache is the shared cache; never delete `C:\sccache_cache`. Target dirs are per worktree (`.cargo-target`); never point `CARGO_TARGET_DIR` at a shared path.
- Clean up: worktree, branch, build output, temp files. Scratch stays on `C:`.
- Never commit `.log`, `.exit`, or build output; scratch stays outside the repo.
- No census or dump binaries in the tree; evidence is data plus the script that produced it, under the run dir.
- `hoi4_re`: `re_status` first. Add to its registry only by running the tool, never by hand. Four-job offline queue. `hoi4_ghidra` (pinned a729/870ebf): `ghidra_status` first, `ghidra_verify` before relying on indexed evidence. Census and cross-analysis outputs are discovery, not proof. Pinned artifacts use the `engine_evidence` crate pattern with literal byte sizes and must pass `reverify_engine_evidence.py`.
- Lanes never launch the game. A needed live probe goes back to the orchestrator as a bounded SPEC.
- Testing: a lane runs the suites of the crates it touched, once, after approve. The gate runs everything else. Nobody reruns suites after a merge.

## Lane

1. Start: `git worktree lock`, one empty commit, and check `git rev-parse HEAD` equals the brief's base commit.
2. Verify the brief's premise with file:line before you build. A false premise is a valid outcome; report it.
3. Work only in your worktree. Implement everything yourself. Between review rounds run only `cargo check` on the crates you touched.
4. Review with the orchestrator until it approves with zero blocking findings. Push back on a finding you can refute with code or a measurement.
5. After approve, once: the full test modules of every crate and file you changed (no hand-picked subsets), then `bash extractor/native/scripts/lane_law_tests.sh` (`--collector` when the collector was touched). A step that starts zero tests fails. A failure is fixed and gets one confirmation round.
6. Merge `origin/master` (or the branch the brief names) before you finish. Commit early and often, push when converged, report.

Never run the full workspace suite.

### Review

- After implementation and validation, spawn exactly one reviewer subagent of your own kind and model.
- Give it the requirements, diff, and raw validation evidence. Keep it read-only, fix every finding, and re-review with the same subagent until it approves. If the commit changes afterward, get one final confirmation.

## Orchestrator

Staffs lanes, reviews, lands, runs the gate, runs captures, asks the owner. Never implements lane work itself. Cap: 7 running agents; queue past it, never drop. After a restart or limit kill, resume each agent by its id. Never relaunch a lane that has an id. Never replace a slow lane.

### Review, every lane, before any merge

You are the final reviewer and own every merged line. The lane's report is a claim, not evidence.

1. Read the full production diff, never truncated. Read every new or changed test; each must assert something that can fail.
2. Consumer sweep: `git grep` every crate that references a changed public item, wire name, contract, schema, or config. Run those suites yourself.
3. Run the touched crates' suites and the law script yourself when the lane's counts are the only evidence.
4. Simplify: duplicated logic, dead branches, one-caller abstractions, comments narrating history. Bounce with the concrete cut.
5. Correctness against `PROJECT.md`: zero silent loss, mirror the engine, typed refusals by name, no default in place of a measurement.

A finding names production or test code whose behaviour is wrong, unproven, or unsafe. Prose, naming, and wording are not findings. Bounce until zero findings. A commit after approve gets one more look.

### Landing, per batch

0. Kill orphaned `cargo.exe` and hung test binaries by PID.
1. `git merge --no-ff` each converged branch onto `landing-tmp`. Check the exit code. Assert `HEAD^2` is the lane tip.
2. Content survival: `git diff <lane tip> HEAD -- <lane files>` is empty or explained.
3. `verify_citation_integrity.py`, then `reverify_engine_evidence.py`.
4. Gate: `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-Validation.ps1 -Mode Landing`. Capture the exit code.
5. Green: push `landing-tmp:master`, verify every tip is an ancestor, retire worktrees and branches. Red: attribute each failure against pure master and bounce it to the owning lane.

### Captures

- Bounded live reads on a matching build have standing authorization; do not ask per instance. One at a time. Results as data.
- A capture is a long job: use the wait rule. Kill the leftover hooked game when done.
- Never kill `collector_record` after a capture; finalize takes minutes. Watch `RECORDER_FINALIZE_PROGRESS_V1.json`.
- Game clock stuck at 0 percent means the end-game stats screen. Relaunch with `-StopExistingGame`. Never leave the game running between captures.
- Measurement captures: `-KeepSpoolAfterSeal`, `-StreamCompression none`, one capture at a time. Check "records seen" before trusting an audit.
- Build the collector with `cargo build -p collector` and check the DLL is newer than every `.rs`. A count mismatch is version skew first.
- Capture windows are 60 seconds. No longer soaks.
- Release-candidate captures: vanilla first, solo. Modded last; the owner hosts.

## Claude Code only

- Never use the PowerShell tool. Run `powershell.exe -NoProfile -File ...` from Bash.
- Wait rule: never end your turn while your own job (review, build, test, fold, dump, capture, lane) is still running. Bound every child; a hung child is a finding. Launch inside a background bash subshell, capture bash's `$!`, and hold the turn with a foreground tail:

```bash
( job ... > log 2>&1; echo $? > log.exit ) & PID=$!
timeout 590 tail --pid=$PID -f /dev/null   # repeat while it exits 124; then read log and log.exit
```

  `$PID` must be bash's `$!`. `tail --pid` cannot follow a Windows PID from `tasklist`. No polling loops, no `sleep`. One long job at a time.
- The orchestrator is exempt from the wait rule: launch gates and lanes in the background, end the turn, act on the completion notification. Never block a turn on TaskOutput.
- Never launch a Fable agent unless the owner explicitly asks for one. Every other Claude subagent runs on Opus.
- Launching a lane (orchestrator only): Codex CLI, GPT-6 Astra, high reasoning, never the priority tier.

```bash
git fetch origin && git worktree add ".claude\worktrees\lane-<name>" -b lane-<name> origin/landing-tmp
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
cd ".claude\worktrees\lane-<name>" && codex exec --json -s danger-full-access -c approval_policy=never --skip-git-repo-check -m gpt-6-astra -c model_reasoning_effort=high -o "$OUT" - < brief.txt > "$AJ"
```

  Flags as written: never `--ephemeral`, never a sandboxed `-s` mode, never `-c service_tier=priority`. Thread id = first line of `$AJ`; the report = `$OUT`. On a usage-limit or credit signature, resume by id with the same flags: `codex exec ... resume <id> -`.
  `CODEX_HOME` selects the account (default `~/.codex`); on a usage-limit or credit signature, resume the same thread under the next usable home: `CODEX_HOME=~/.codex-t3/<account> codex exec resume <thread_id> --json --skip-git-repo-check -c approval_policy=never -c 'sandbox_mode="danger-full-access"' -o <report> "<prompt>"` (`resume` does not take `-s` and inherits no sandbox settings; the two `-c` flags and `--skip-git-repo-check` are required). Rollouts live in `~/.codex/sessions` and are shared by every account home; a fresh brief is never the answer for a lane that has a thread id. Run the exec in the foreground of one background Bash call, one lane per call; never detach it with `( ... ) &`.

## Codex only

- Lanes are your sub-agents, each in its own worktree.
- Never call the Codex CLI from inside Codex.
