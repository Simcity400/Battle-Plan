# Agent Instructions Pointer

This root file exists only because agent tooling auto-discovers `AGENTS.md` at
the repository root.

The canonical repo operating rules live in `documents/AGENTS.md`. Read and
follow that file before changing this repository.

**Every agent doing delegated work — orchestrator or delegate, any layer — must also read
`documents/LANE_WORKFLOW.md`.** It is the repo's method: measure before optimizing, refute your
own brief, prove don't assert, correctness gates before performance, the implementer never
reviews its own diff, lane isolation and lane reports, and the orchestrator's verification
battery. It is layer-agnostic and applies to work that does not exist yet.

Each layer additionally keeps a **lane brief** with that layer's hard requirements, established
facts, proven NULLs, and gates — e.g. `extractor/documents/CODEX_LANE_BRIEF.md`. Read the
workflow, then your layer's brief, then start.
