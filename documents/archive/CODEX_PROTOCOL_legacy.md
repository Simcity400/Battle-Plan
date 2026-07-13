# Codex Review Protocol (SUPERSEDED — historical record only)

> **Status 2026-07-10: the ban described below is WITHDRAWN.** The local Codex CLI is active
> again as the always-on adversarial reviewer. The canonical, current standard is
> `C:\Users\Example\.claude\CODEX_PROTOCOL.md` (Mode A / Mode B, `-m gpt-5.6-sol
> -c model_reasoning_effort=xhigh`). Do not follow the rules in the section below — they are
> retained only so the 2026-07-09 stance stays legible in the history.

## What this file used to say (2026-07-09 → 2026-07-10)

For roughly one day, Codex-native sessions were told to stop calling the local CLI entirely and
to rely on Codex-native sub-agents instead. The rules in force during that window were:

- Do not run the old local Codex CLI review workflow or any historical Mode A / Mode B
  invocation.
- Do not embed local Codex CLI commands in sub-agent prompts.
- Do not use fast mode, small/fast model variants, or lower reasoning effort for project
  sub-agents. Omit model, reasoning, and service-tier overrides so sub-agents inherit the
  orchestrator's current high-effort Codex settings unless the user explicitly requests a
  different model.
- Do not select sub-agent roles, prompts, or shortcuts for speed over rigor. Prefer normal
  default/worker partner agents for project work; spawn fewer agents if needed, but keep them on
  the inherited high-effort path.
- Use normal Codex sub-agents as work partners for implementation, exploration, and review.
- For substantial work, involve the partner while the approach is still being shaped so it can
  clarify assumptions, challenge weak spots, and help work through the problem. Do not reduce
  this to a final verification pass.
- If a sub-agent cannot spawn its own nested partner, ask the orchestrator to assign one or
  report the need early. Do not fall back to the old local CLI.

## What carried forward

The intent behind those rules — the project wants real pushback and independent checking, at
full reasoning effort, never traded away for speed — is unchanged. Only the *mechanism* reverted:
that pushback now comes from the local `codex` CLI again, pinned to `gpt-5.6-sol` at `xhigh`.

Note the one rule above that is now actively inverted: sub-agents no longer inherit the model
implicitly. Every call passes `-m gpt-5.6-sol` explicitly so a `~/.codex/config.toml` drift
cannot silently downgrade a reviewer.
