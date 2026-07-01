# Battle Plan - Claude Compatibility Rules

This is a private, passive, offline, host-side multiplayer-stats / full-game
replay tool for the owner's own game group. It is read-only and
observational.

## Adversarial Review / Sub-Agent Protocol

The repo used to require Claude agents to call a local `codex` CLI through
`CODEX_PROTOCOL.md`. That rule is obsolete for Codex-native sessions.

- Codex-native agents and sub-agents must not run the old local Codex CLI review
  workflow or any historical Mode A / Mode B invocation.
- Codex-native agents and sub-agents must not use fast mode, small/fast model
  variants, or lower reasoning effort for this project. Omit model, reasoning,
  and service-tier overrides so sub-agents inherit the orchestrator's current
  high-effort Codex settings unless the user explicitly requests a different
  model.
- Do not select sub-agent roles, prompts, or shortcuts for speed over rigor.
- Use normal Codex sub-agents as work partners while the approach is being
  shaped, not only as a final verification pass.
- If a sub-agent session cannot spawn its own nested partner, it should ask the
  orchestrator to assign one or report the need early. It should not fall back
  to the old local CLI.
- Do not embed the old CLI command in sub-agent prompts.

## Hard Safety Rules

- No automated attribution in commits; omit co-author trailers.
- No commits unless the user explicitly asks.
