# Agent guidance

Changes only with the owner's permission. Read `PROJECT.md` before changing product behaviour, formats, or architecture. This file is read by every harness (Claude Code, Codex, others); the roles below are the same everywhere, only the launch mechanics at the end differ.

## Roles

- **Orchestrator**: the top-level agent the owner has EXPLICITLY designated as orchestrator for the session. Nobody assumes this role on their own. Staffs lanes, lands their work, runs the gate, runs measurement captures, asks the owner. Never implements lane work itself.
- **Lane**: an agent with its own worktree and branch. Implements one scoped piece of work end to end, gets it reviewed, pushes, reports once.
- **Reviewer**: a separate agent with a fresh context and no memory of the implementer's work, reviewing read-only. It never implements, researches, or writes into a worktree. In Claude Code this is the Codex CLI; in Codex it is a sub-agent; see "How to launch".

If the owner has not designated you orchestrator, you are a lane or a reviewer per your brief: ignore the "Orchestration" section and the launch mechanics entirely.

## How every agent behaves

- Questions to the owner: plain language, no jargon, say how it affects what the end user sees. The orchestrator batches them and gives options with a recommendation.
- No backwards compatibility: delete what you replace in the same change and prove it dead.
- Code comments state only what the code cannot show (invariants, engine quirks, traps). No essays, no narration, no review notes.
- Evidence is data: raw artifacts, JSON/TSV, asserting tests. One short `POINTER.md` per run dir under `extractor/native/runs/<run>/`; claims live as tests or structured data, never prose.
- Residuals are never left behind. Anything you can fix, fix now. Anything you legitimately cannot (owner decision, live probe, missing capture, another lane's file) goes in your final report with its class and the exact artifact path so the orchestrator staffs it immediately.
- Report once, numbers first, when everything is finished: tip, what landed, tests run, review record, residuals.
- Clean up: worktree, branch, build output, temp files. Never stash. Hash-pinned files carry `.gitattributes` `text eol=lf`.

## Lane workflow

1. Work only in your worktree. Start pin = your branch's first commit. Merge `origin/master` (or the branch the brief names) before you finish.
2. Implement everything yourself; the reviewer is not a delegate. Between review rounds run only `cargo check` on the crates you touched.
3. Review until the reviewer approves with zero blocking findings (contract below).
4. Then, once: the full test modules of every crate and file you changed (no hand-picked subsets), plus the law list. A failure is fixed and gets one confirmation round.
5. Commit early and often, push your branch when converged, report.

Never run a full workspace suite; the gate does that once per batch. Never run the whole-capture decode tests (`game_ui::uz_rc_vanilla8_operands`, the xmap00 ledger audit); they are gate-only.

**Law list** (every lane, regardless of what it touched; a step that starts zero tests is a failure):
- `cargo nextest run -p replay_app -E 'test(game_file_ledger)'`
- `cargo nextest run -p collector -E 'test(xui09_joint_certification)'` and `cargo nextest run -p game_ui -E 'test(joint_certification)'`
- `cargo nextest run -p compact_contract --test compact_contract_suite -E 'test(counter_wire_census)'`
- `cargo nextest run -p replay_core -E 'test(every_country_reference_decode_is_gated_witnessed_or_named)'`
- collector-wide laws when the collector is touched: `attach_log domain_scoped_reset_tests`, `inline_detour` quiescence-sweep and detour-census laws, `d51_read_window_tests`
- `python extractor/native/scripts/verify_citation_integrity.py` and `python extractor/native/scripts/restamp_world_fold_citations.py --check`

**Fold equivalence** when a fold change needs proof: fold `captures/rc_vanilla8/collector_stream.bin` once with your tree and compare against `extractor/native/runs/fold_reference/<master-sha>/` (same `--game-id`, spool sha256 verified). Never re-fold master.

## Review contract

Applies to every harness. The reviewer is a fresh agent each round.

- The review prompt carries full context: the defect class, the diff scope, raw evidence paths, an out-of-scope list, and "this job is read-only, do not modify files". The reviewer has no project memory.
- Review scope is code behaviour only. A finding is valid only if it names production or test code whose behaviour is wrong, unproven, or unsafe. Prose, wording, naming, comments, and claim phrasing are discarded without a round.
- Round 1 demands every blocking finding, ranked. Later rounds review the delta plus each prior fix. Demonstrating mutations are applied by the lane, delta-scoped and cached by content hash; no mutation batteries. Push back on findings you can refute with code or a measurement.
- No round limit while rounds fix real defects. At round 7, stop and diagnose: if one class keeps recurring, close the class structurally (a counted, asserted number) instead of playing round 8, and write the diagnosis into the run dir.
- A commit after an approve gets one confirmation round.
- Persist the review's identity (thread or transcript id) to the run dir before anything else, so a killed review is resumed, never re-run. Wait for a review in-turn; never end a turn to wait, never poll in a loop.

## Tools and traps

- `hoi4_re` is the RE facade: `re_status` first, never edit its registry, four-job offline queue. `hoi4_ghidra` (pinned a729/870ebf): `ghidra_status` first, `ghidra_verify` before relying on indexed evidence. Census and cross-analysis outputs are discovery, not proof. Pinned artifacts use the `engine_evidence` crate pattern with literal byte sizes and must pass `reverify_engine_evidence.py`.
- Live instrumentation needs explicit owner authorization and a running matching game; lanes never launch the game. A needed probe is handed back as a bounded SPEC.
- One background task at a time; never poll a child; own scratch subdir under the session scratchpad.
- Disk: your worktree's build directory is yours; reclaim only your own incremental cache and only below 30 GB free.
- Citation scanner: citation strings at module scope; function-local items need reasoned allowance entries; never split a citation across lines; two local items sharing a name break the world-fold scanner, so rename rather than declare. Source-scan tests normalise CRLF.
- A carrier change that moves leaves in the XUI09 joint fixture regenerates it through `JO_WRITE_XUI09_FIXTURE=1` and updates the consumer half's `FIXTURE_SHA256`.
- Any `attach_log` macro change runs the counter-wire census.

## Orchestration (designated orchestrator only)

- Landing, per batch: `git merge --no-ff` each converged branch onto `landing-tmp`, check the merge exit code before anything else, assert `HEAD^2` is the lane tip, restamp citations (`restamp_world_fold_citations.py`, `restamp_citation_allowance.py`, then `--check`), `verify_citation_integrity.py`, `reverify_engine_evidence.py`, then one gate: `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-Validation.ps1 -Mode Landing`, exit code captured. Green: push `landing-tmp:master`, verify every tip is an ancestor, retire worktrees and branches. Red: attribute each failure against pure master and bounce it to the owning lane. Hold the gate for batches; never restart a running gate.
- Publish one pure-master fold reference per master commit under `runs/fold_reference/<sha>/`.
- Keep a residuals ledger; every item has an owner (a lane, an owner question, or the next measurement capture) until closed.
- After a restart or limit kill, resume existing agents with their transcripts; never relaunch a lane that has one.

## How to launch, per harness (designated orchestrator and its lanes only)

**Claude Code.** Lanes: `subagent_type: "lane-opus"` (Opus 5, high); verify the resolved model in `subagents/agent-*.jsonl`; resume by agent id with `SendMessage`. Reviewer: the Codex CLI, one fresh thread per round, from Bash:

```bash
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
printf '%s' "$PROMPT" | codex exec --json -s danger-full-access -c approval_policy=never --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=xhigh -c service_tier=priority -o "$OUT" - > "$AJ"
```

Flags are mandatory as written; never `--ephemeral`, never `resume --last`, never a sandboxed `-s` mode. Thread id = first line of `$AJ`; reply = `$OUT`, success is exit 0 and a non-empty file; retry once, except on a usage-limit signature: queue the thread id and resume it later by id (`codex exec -s danger-full-access … resume <id> -`, same flags). Run the call as one tracked background task with a Monitor on a completion marker.

**Codex.** Lanes and reviewers are sub-agents. Never call the Codex CLI from inside Codex. A reviewer is a fresh sub-agent given the review prompt above, read-only; the lane records the reviewer's transcript id in its run dir. Model and effort follow the owner's current pin for that harness.

**Any other harness.** Same roles and contract; the reviewer must be a separate agent with no shared memory of the implementation. Record how reviews are launched and resumed in the run dir's POINTER.
