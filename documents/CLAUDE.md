# Claude Compatibility Rules

**Purpose:** Keep Claude-oriented handoffs aligned with the canonical repo rules and the
machine-wide Codex protocol without duplicating either.

Canonical repo operating rules live in `documents/AGENTS.md`.

This file exists only for Claude-oriented handoffs.

## Codex CLI — the sole sub-agent

The local Codex CLI is both the **delegation engine** and the **always-on adversarial reviewer**
in this repo, per the machine-wide standard in `C:\Users\Example\.claude\CODEX_PROTOCOL.md`. Read
that file in full before invoking Codex; its commands and failure handling are canonical.

**Do not spawn Claude Agent-tool sub-agents** — no Opus lanes, no Sonnet, no Fable, no
`Explore`/`general-purpose` search agents. Claude orchestrates and verifies; Codex does the
delegated work.

Every Codex call must be resumable: use `--json`, capture and persist the thread id from the first
JSONL line, and never use `--ephemeral` or `resume --last`. Pin `gpt-5.6-sol` at `xhigh`, read
the response from a unique `-o` file, and require exit zero plus a non-empty response. Use
read-only Mode A for review/research and workspace-write Mode C for implementation. One visible
background task belongs to each invocation; each implementation lane also gets its own worktree.
The exact commands remain in the protocol.

**Verify before commit, always:** run the gates yourself, `git status --porcelain` scope review
(no out-of-scope `replay_interface/` or rule-doc edits; new source files staged), read the key diff
hunks, spot-check a claim, then a Mode-A adversarial
gate on the diff. One Codex fix round per review; after that you fix it directly. Codex's reply
is not the diff — it can write out-of-scope files without reporting them.

## Rules for Claude/Codex mixed handoffs

- Read and follow `documents/AGENTS.md`.
- Current extraction completion and residuals live only in
  `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`; domain status lives in
  `extractor/native/domain_registry.json`.
- Do not use fast mode, small/fast model variants, or lower reasoning effort for this project
  unless the user explicitly requests it. That applies to Codex too: `gpt-5.6-sol` at `xhigh`,
  never `gpt-5.6-luna`/`terra` or a reduced effort.
- Omit automated attribution/co-author trailers in commits.
