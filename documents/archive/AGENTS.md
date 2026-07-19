# Battle Plan — agent operating rules

**Purpose:** Provide the canonical repo-level operating entry point and route work to the
project vision, workflow, extraction status, and machine-wide Codex protocol.

This is a **private** repo: a passive, offline, host-side multiplayer-stats / **full-game
replay** tool for the owner's own game group. Read `documents/DESIGN.md` for the vision (maximal-
depth replay: economy, military, divisions, provinces, combat, focuses, decisions, diplomacy —
"everything you'd see in a save game"). It is Windows-only and read-only/observational.

Codex is the sole delegation engine. Use `documents/LANE_WORKFLOW.md` for the repo method and
`C:\Users\Example\.claude\CODEX_PROTOCOL.md` for invocation/resumability rules; do not create
Claude Agent-tool subagents. Extraction completion status lives only in
`extractor/documents/MASTER_EXTRACTION_CONTRACT.md`, and per-domain status is
code-authoritative in `extractor/native/domain_registry.json`.
