# Codex collaboration protocol (STANDING, ALWAYS-ON)

> This file is self-contained. Any agent — with **zero** prior context — can read it and
> correctly drive the local **Codex CLI** as an adversarial second pair of eyes. This is
> not optional and you never need to ask the user's permission to use it: in this project,
> Codex review/diagnosis is **always on by default**.

## Why this exists

Claude works alongside the **Codex CLI** (OpenAI's coding agent, installed locally). Codex is
used as an **adversarial reviewer / second opinion** on reverse-engineering findings, offset
hypotheses, save-parse logic, crash diagnosis, and any non-trivial code change. This catches
real bugs that a single model misses. Use it liberally and proactively — propose a hypothesis,
have Codex try to refute it, reconcile.

## The two hard rules

1. **It is always on.** Never ask the user "should I use Codex?". For any substantive
   diagnosis, RE result, or risky change, consult Codex as a matter of course and report what
   it said. The only time you skip it is trivial mechanical edits.
2. **It must be concurrency-safe.** Many agents run their own Codex in parallel here.
   The global "most-recent session" (`resume --last`) is therefore **BANNED** — concurrent
   callers fight over it. Use Mode A (stateless one-shot) by default; use Mode B only with an
   explicit captured thread id.

## Invocation (copy exactly)

`codex` is on the Bash PATH (the npm global bin, `~/AppData/Roaming/npm/codex`). Verified on
codex-cli 0.139.0.

### Mode A — one-shot, stateless (DEFAULT; bulletproof; use this for sub-agents)

Pass *full* context in the prompt; nothing is persisted, so it can never collide:

```bash
printf '%s' "$PROMPT" | codex exec --ephemeral -s read-only -c approval_policy=never --skip-git-repo-check -o /tmp/cdx_<uniq>.txt -
```

### Mode B — multi-turn, isolated by UUID (only when you truly need carry-over)

```bash
# Turn 1 — capture the thread id (NO --ephemeral):
printf '%s' "$P1" | codex exec --json -s read-only -c approval_policy=never --skip-git-repo-check -o /tmp/cdx_a.txt - > /tmp/cdx_a.jsonl 2>/dev/null
TID=$(python -c "import json;print(json.loads(open('/tmp/cdx_a.jsonl').readline())['thread_id'])")
# Turn 2+ — resume the SAME thread by UUID, and DO NOT pass -s (resume rejects it):
printf '%s' "$P2" | codex exec resume "$TID" -c approval_policy=never --skip-git-repo-check -o /tmp/cdx_b.txt -
```

## Rules that prevent every known failure

- Read Codex's reply from the **`-o` file**, never stdout. Use a **unique** `-o` filename per
  call (parallel agents must never share one) — e.g. `/tmp/cdx_<agent>_<step>.txt`.
- `resume` does **NOT** accept `-s/--sandbox` (only `codex exec` does). Resume inherits the
  original read-only sandbox — just omit the flag.
- `-s read-only` (on `exec`) lets Codex read/grep/`git diff` but not edit — right for review.
  Use `-s workspace-write` only if Codex must edit files.
- `codex exec` + `approval_policy=never` is non-interactive → it physically cannot hang on a
  prompt. (That was the old "never finishes" cause.)
- Feed the prompt via **stdin** with a trailing `-` (no quoting/length limits). Big prompt →
  `cat prompt.txt | codex exec ... -`.
- **Verify every call:** success = exit 0 **AND** the `-o` file is non-empty. The pipeline exit
  code is the last stage's — don't pipe codex into `tail`/`head` if you need its exit code.
  On failure: retry once, then surface it to the user.
- **NEVER `--last`** when any other agent might be using Codex (essentially always, here).

## Propagation — this is how it survives with no thread context

**When you spawn ANY sub-agent, you MUST embed the Codex instruction in its prompt:**
hand it Mode A *verbatim*, tell it `codex` is on the Bash PATH, tell it to read this file
(`CODEX_PROTOCOL.md`) for the full standard, and tell it to consult Codex adversarially on its
findings before reporting back. Do not give a vague "use Codex" — give the exact command and
the verify-step. Each sub-agent uses its own `-o` filenames; ephemeral one-shots guarantee
zero cross-agent interference.

> The Claude Code `codex@openai-codex` *plugin* (the `codex:*` skills + Stop review-gate hook)
> is intentionally DISABLED. Use the standalone `codex` CLI described above; do not re-enable
> the plugin skills.
