# Claude Compatibility Rules

Canonical repo operating rules live in `documents/AGENTS.md`.

This file exists only for Claude-oriented handoffs.

## Codex CLI — ACTIVE, and the ONLY sub-agent (2026-07-10)

The local Codex CLI is both the **delegation engine** and the **always-on adversarial reviewer**
in this repo, per the machine-wide standard in `C:\Users\Example\.claude\CODEX_PROTOCOL.md`. Read
that file; it is canonical. The 2026-07-09 "do not use the local Codex CLI at all" rule is
**withdrawn** (historical text kept in `documents/archive/CODEX_PROTOCOL_legacy.md`).

**Do not spawn Claude Agent-tool sub-agents** — no Opus lanes, no Sonnet, no Fable, no
`Explore`/`general-purpose` search agents. Codex replaced Opus as the delegate on 2026-07-10.
Claude orchestrates and verifies; Codex does the delegated work.

Every Codex call pins the model and extra-high reasoning:

```bash
OUT=$(mktemp /tmp/cdx_XXXXXX.txt)
printf '%s' "$PROMPT" | codex exec --ephemeral -s read-only -c approval_policy=never --skip-git-repo-check -m gpt-5.6-sol -c model_reasoning_effort=xhigh -o "$OUT" - >/dev/null
```

- `codex` is on the Bash PATH (`/c/Users/Example/AppData/Roaming/npm/codex`).
- Read the reply from the `-o` file, not stdout. Success = exit 0 AND non-empty `-o`. Retry
  once, then surface the failure — never claim a review that did not happen.
- Mint the `-o` path with `mktemp` (a literal `/tmp/cdx_<uniq>.txt` is parsed by bash as a
  redirection); never `resume --last` (it collides across parallel agents).
- Embed the exact command above, plus the CODEX_PROTOCOL.md pointer, in every Codex job prompt.

**Delegated implementation (Mode C):** `-s workspace-write`, one lane per git worktree, one job
per worktree, disjoint crates. `cd` into the worktree *before* invoking — the sandbox roots at
the invocation cwd. Never pass `-c service_tier=priority`.

**Verify before commit, always:** run the gates yourself, `git status --porcelain` scope review
(`replay_interface/` untouched from the extractor side, `documents/AGENTS.md` untouched, new
source files staged), read the key diff hunks, spot-check a claim, then a Mode-A adversarial
gate on the diff. One Codex fix round per review; after that you fix it directly. Codex's reply
is not the diff — it can write out-of-scope files without reporting them.

## Rules for Claude/Codex mixed handoffs

- Read and follow `documents/AGENTS.md`.
- Do not use fast mode, small/fast model variants, or lower reasoning effort for this project
  unless the user explicitly requests it. That applies to Codex too: `gpt-5.6-sol` at `xhigh`,
  never `gpt-5.6-luna`/`terra` or a reduced effort.
- Omit automated attribution/co-author trailers in commits.
