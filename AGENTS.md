# Agent guidance

Changes only with the owner's permission. Read `PROJECT.md` before changing product behaviour, formats, or architecture. This file is read by every harness (Claude Code, Codex, others); the roles below are the same everywhere, only the launch mechanics at the end differ.

## Roles

- **Orchestrator**: the top-level agent the owner has EXPLICITLY designated as orchestrator for the session. Nobody assumes this role on their own. Staffs lanes, lands their work, runs the gate, runs measurement captures, asks the owner. Never implements lane work itself.
- **Lane**: an agent with its own worktree and branch. Implements one scoped piece of work end to end, gets it reviewed, pushes, reports once.
- **Reviewer**: a separate agent with a fresh context and no memory of the implementer's work, reviewing read-only. It never implements, researches, or writes into a worktree. In Claude Code this is the Codex CLI; in Codex it is a native Claude Code background session; see "How to launch".

If the owner has not designated you orchestrator, you are a lane or a reviewer per your brief: ignore the "Orchestration" section and the launch mechanics entirely.

## How every agent behaves

- Questions to the owner: plain language, no jargon, say how it affects what the end user sees. The orchestrator batches them and gives options with a recommendation.
- No backwards compatibility: delete what you replace in the same change and prove it dead.
- Code comments state only what the code cannot show (invariants, engine quirks, traps). No essays, no narration, no review notes.
- Evidence is data: raw artifacts, JSON/TSV, asserting tests. One short `POINTER.md` per run dir under `extractor/native/runs/<run>/`; claims live as tests or structured data, never prose.
- Residuals are never left behind. Anything you can fix, fix now. Anything you legitimately cannot (owner decision, live probe, missing capture, another lane's file) goes in your final report with its class and the exact artifact path so the orchestrator staffs it immediately.
- Report once, numbers first, when everything is finished: tip, what landed, tests run, review record, residuals.
- Clean up: worktree, branch, build output, temp files. Never stash. Hash-pinned files carry `.gitattributes` `text eol=lf`.
- (Claude only) Long calls: the blocking wait from "Tools and traps"; never a monitor with an ended turn.

## Lane workflow

1. Work only in your worktree. Start pin = your branch's first commit. Merge `origin/master` (or the branch the brief names) before you finish.
2. Implement everything yourself; the reviewer is not a delegate. Between review rounds run only `cargo check` on the crates you touched.
3. Review until the reviewer approves with zero blocking findings (contract below).
4. Then, once: the full test modules of every crate and file you changed (no hand-picked subsets), plus the law-test script below. A failure is fixed and gets one confirmation round.
5. Commit early and often, push your branch when converged, report.

Never run a full workspace suite; the gate does that once per batch. Never run the whole-capture decode tests (`game_ui::uz_rc_vanilla8_operands`, the xmap00 ledger audit); they are gate-only.

After convergence run `bash extractor/native/scripts/lane_law_tests.sh` once (`--collector` when the collector was touched): the cross-crate scans the gate would otherwise catch a batch later; a step that starts zero tests fails.

**Fold equivalence** when a fold change needs proof: fold `captures/rc_vanilla8/collector_stream.bin` once with your tree and compare against `extractor/native/runs/fold_reference/<master-sha>/` (same `--game-id`, spool sha256 verified). Never re-fold master.

## Review contract

Applies to every harness. The reviewer is a fresh agent each round.

- The review prompt carries full context: the defect class, the diff scope, raw evidence paths, an out-of-scope list, and "this job is read-only, do not modify files". The reviewer has no project memory.
- Review scope is code behaviour only. A finding is valid only if it names production or test code whose behaviour is wrong, unproven, or unsafe. Prose, wording, naming, comments, and claim phrasing are discarded without a round.
- Round 1 demands every blocking finding, ranked. Later rounds review the delta plus each prior fix. Push back on findings you can refute with code or a measurement.
- No round limit while rounds fix real defects. At round 7, stop and diagnose: if one class keeps recurring, close the class structurally (a counted, asserted number) instead of playing round 8, and write the diagnosis into the run dir.
- A commit after an approve gets one confirmation round.
- Persist the review's identity (thread or transcript id) to the run dir before anything else, so a killed review is resumed, never re-run.

## Tools and traps

- `hoi4_re` is the RE facade: `re_status` first, never edit its registry, four-job offline queue. `hoi4_ghidra` (pinned a729/870ebf): `ghidra_status` first, `ghidra_verify` before relying on indexed evidence. Census and cross-analysis outputs are discovery, not proof. Pinned artifacts use the `engine_evidence` crate pattern with literal byte sizes and must pass `reverify_engine_evidence.py`.
- Live instrumentation needs explicit owner authorization and a running matching game; lanes never launch the game. A needed probe is handed back as a bounded SPEC.
- **Waiting is a blocking call, never an ended turn.** A lane that has ended its turn is finished, and a report is due; so a lane never ends its turn while a review, build, test run, fold or dump of its own is still running. Launch the long job inside a background bash subshell and capture that subshell's `$!` (`( job … > log 2>&1; echo $? > log.exit ) & PID=$!`), then hold the turn with a foreground `timeout 590 tail --pid=$PID -f /dev/null`, repeated while it exits 124, until it exits 0; then read the log and the exit file. The PID must be bash's `$!`; `tail --pid` cannot follow a native Windows PID taken from `tasklist`, and the subshell keeps `$!` alive for wrappers that detach. No polling loops, no `sleep`, no monitor-and-end-turn. One long job at a time; own scratch subdir under the session scratchpad.

## Orchestration (designated orchestrator only)

- Landing, per batch: `git merge --no-ff` each converged branch onto `landing-tmp`, check the merge exit code before anything else, assert `HEAD^2` is the lane tip, restamp citations (`restamp_world_fold_citations.py`, `restamp_citation_allowance.py`, then `--check`), `verify_citation_integrity.py`, `reverify_engine_evidence.py`, then one gate: `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-Validation.ps1 -Mode Landing`, exit code captured. Green: push `landing-tmp:master`, verify every tip is an ancestor, retire worktrees and branches. Red: attribute each failure against pure master and bounce it to the owning lane. Hold the gate for batches; never restart a running gate.
- Publish one pure-master fold reference per master commit under `runs/fold_reference/<sha>/`.
- After a restart or limit kill, resume existing agents with their transcripts; never relaunch a lane that has one.

## How to launch, per harness (designated orchestrator and its lanes only)

**Claude Code.** Lanes: `subagent_type: "lane-opus"` (Opus 5, high); verify the resolved model in `subagents/agent-*.jsonl`; resume by agent id with `SendMessage`. Reviewer: the Codex CLI, one fresh thread per round, from Bash:

```bash
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
printf '%s' "$PROMPT" | codex exec --json -s danger-full-access -c approval_policy=never --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=xhigh -c service_tier=priority -o "$OUT" - > "$AJ"
```

Flags are mandatory as written; never `--ephemeral`, never `resume --last`, never a sandboxed `-s` mode. Thread id = first line of `$AJ`; reply = `$OUT`, success is exit 0 and a non-empty file; retry once, except on a usage-limit signature: queue the thread id and resume it later by id (`codex exec -s danger-full-access … resume <id> -`, same flags).

**Codex.**
- Codex review Lanes are sub-agents. Never call the Codex CLI from inside Codex.
- If asked to use claude as the reviewer, Launch: `claude --bg --name "$REVIEW_NAME" --model fable --effort xhigh --permission-mode auto "$PROMPT"`. Never use `-p`
- Claude reviews read-only: no implementation, edits, commits, or worktrees. Record the session id, wait in-turn
- On failure, login expiry, API billing, or limits, preserve and resume the same session id; never launch a replacement. Findings return to the implementing lane.

**Any other harness.** Same roles and contract; the reviewer must be a separate agent with no shared memory of the implementation. Record how reviews are launched and resumed in the run dir's POINTER.
