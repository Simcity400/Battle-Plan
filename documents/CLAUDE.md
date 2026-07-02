# Claude Compatibility Rules

Canonical repo operating rules live in `documents/AGENTS.md`.

This file exists only for Claude-oriented handoffs. The old local Codex CLI
review protocol has been archived in `documents/archive/CODEX_PROTOCOL_legacy.md`
and must not be used for Codex-native sessions.

Rules for Claude/Codex mixed handoffs:

- Read and follow `documents/AGENTS.md`.
- Do not run the old local Codex CLI review workflow or any historical Mode A /
  Mode B invocation.
- Do not embed old local Codex CLI commands in sub-agent prompts.
- Do not use fast mode, small/fast model variants, or lower reasoning effort for
  this project unless the user explicitly requests it.
- Omit automated attribution/co-author trailers in commits.
