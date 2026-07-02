# Codex Review Protocol

This file is retained for historical Claude handoffs and compatibility only.

The old workflow asked Claude agents to call a local Codex CLI with Mode A or
Mode B invocations. That workflow is disabled for Codex-native sessions.

## Current Rule For Codex-Native Agents

- Do not run the old local Codex CLI review workflow or any historical Mode A /
  Mode B invocation.
- Do not embed local Codex CLI commands in sub-agent prompts.
- Do not use fast mode, small/fast model variants, or lower reasoning effort for
  project sub-agents. Omit model, reasoning, and service-tier overrides so
  sub-agents inherit the orchestrator's current high-effort Codex settings
  unless the user explicitly requests a different model.
- Do not select sub-agent roles, prompts, or shortcuts for speed over rigor.
  Prefer normal default/worker partner agents for project work; spawn fewer
  agents if needed, but keep them on the inherited high-effort path.
- Use normal Codex sub-agents as work partners for implementation, exploration,
  and review.
- For substantial work, involve the partner while the approach is still being
  shaped so it can clarify assumptions, challenge weak spots, and help work
  through the problem. Do not reduce this to a final verification pass.
- If a sub-agent cannot spawn its own nested partner, ask the orchestrator to
  assign one or report the need early. Do not fall back to the old local CLI.

The project still wants pushback and independent checking. It should come from
Codex-native agents in this environment, not from spawning the old CLI.
