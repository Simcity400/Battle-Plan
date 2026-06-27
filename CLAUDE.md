# Battle Plan — agent operating rules

This is a **private** repo: a passive, offline, host-side multiplayer-stats / **full-game
replay** tool for the owner's own game group. Read `DESIGN.md` for the vision (maximal-
depth replay: economy, military, divisions, provinces, combat, focuses, decisions, diplomacy —
"everything you'd see in a save game"). It is the owner's own game, read-only/observational —
there is no anti-cheat concern.

## Adversarial review / sub-agent protocol

This repo used to require Claude agents to call the local `codex` CLI through
`CODEX_PROTOCOL.md`. That rule is obsolete for Codex-native sessions.

- Codex-native agents and sub-agents **must not** run the old local Codex CLI
  review workflow or any historical Mode A/Mode B invocation.
- Codex-native agents and sub-agents **must not** use fast mode, small/fast
  model variants, or lower reasoning effort for this project. Omit model,
  reasoning, and service-tier overrides so sub-agents inherit the orchestrator's
  current high-effort Codex settings unless the user explicitly requests a
  different model.
- Do not select sub-agent roles, prompts, or shortcuts for speed over rigor.
  Prefer normal `default`/`worker` partner agents for project work; spawn fewer
  agents if needed, but keep them on the inherited high-effort path.
- Use normal Codex sub-agents as work partners. For substantial work, bring in a
  Codex-native partner while the approach is still being shaped so it can
  clarify assumptions, challenge weak spots, and help work through problems.
- Do not reduce this to a final verification pass. The partner review should
  influence the implementation, tests, and gap classification before reporting.
- If a sub-agent session cannot spawn its own nested partner, it should ask the
  orchestrator to assign one or report the need early. It should not fall back
  to the old local CLI.
- Do not embed the old CLI command in sub-agent prompts.
- `CODEX_PROTOCOL.md` is retained only for historical Claude handoffs and must
  not override this section in Codex-native sessions.

## Hard safety rules (these caused a history rewrite once — do not repeat)

- **NEVER** commit owner-specific absolute paths, cloud-sync paths, or the
  gitignored dev-scripts folder. Before every commit, scan staged content for
  local usernames, home-directory roots, cloud-sync folder names, and absolute
  host paths.
- **No automated attribution** in commits — omit the `Co-Authored-By` trailer.
- **No commits unless the user explicitly asks.**
- Keep large binary artifacts (map PNGs, raw/melted saves, reader dumps) out of git — gitignore
  them.

## Architecture (one line)

Two channels: **memory reader** (live, daily/hourly cadence, read-only `ReadProcessMemory` on
the host's `hoi4.exe`) for liveness, **save oracle** (rakaly-melted binary saves) for truth /
validation / deep layers. Offsets are **rediscovered per build** from save-derived invariants —
never hardcoded blindly. Fail closed: if memory rediscovery fails after a patch, fall back to
saves rather than emit guessed data.
