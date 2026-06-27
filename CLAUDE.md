# Battle Plan - Claude Compatibility Rules

This file is kept for older Claude-oriented handoffs. `AGENTS.md` is the
authoritative operating rules file for the repo.

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

- Never commit owner-specific absolute paths, cloud-sync paths, or the
  gitignored dev-scripts folder. Before every commit, scan staged content for
  local usernames, home-directory roots, cloud-sync folder names, and absolute
  host paths.
- No automated attribution in commits; omit co-author trailers.
- No commits unless the user explicitly asks.
- Keep large binary artifacts, map PNGs, raw/melted saves, and reader dumps out
  of git.

## Architecture

Two channels: memory reader for live read-only `ReadProcessMemory` liveness, and
save oracle for truth, validation, and deep layers. Offsets are rediscovered per
build from save-derived invariants. Fail closed if memory rediscovery fails.
