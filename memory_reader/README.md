# memory_reader

Read-only live-memory extraction code for the Battle Plan stack.

This package attaches to the owner's local `hoi4.exe` with read permissions only
and extracts game state for `live_capture/`. It must fail closed on unknown
builds or implausible reads; guessed values are worse than missing values.

## Current Role

- Host-side, read-only process-memory readers.
- Per-build rediscovery and validation using save-derived invariants.
- Domain readers for country/state/province data, divisions, combat, logistics,
  naval/air, weather, resources, focus/research, and other replay surfaces.
- Scratch reverse-engineering probes used during discovery, kept out of git.

The active product status is tracked in:

- `../coverage_audit/memory_manifest.json`
- `../coverage_audit/COVERAGE_REPORT.md`
- `../replay_ui/REPLAY_PARITY_COMPLETION_CHECKLIST.md`

## Safety Contract

- Never request write access and never call `WriteProcessMemory`.
- Prefer the game paused during scans unless running an explicit recorder
  cadence proof.
- Gate by HOI4 executable hash/build identity.
- Validate reads against save-oracle invariants where possible.
- On rediscovery failure, omit/badge the field instead of emitting a guess.
- Keep local probe scripts, `_gt` dumps, scan logs, and crack directories
  ignored and untracked.

## Layout

The directory contains both durable readers and historical/research support code.
Durable readers should use descriptive non-scratch names. One-off probes and
generated artifacts should follow the ignored scratch convention:

- top-level `memory_reader/_*.py`, `_*.json`, `_*.out`, `_*.err`, `_*.txt`
- `memory_reader/_*/`
- `memory_reader/focus_crack/`
- `memory_reader/re_scratch*`
- nested `_*.py`, `_*.json`, `.out`, `.log`, `.err`, `.pkl`

If a discovery artifact becomes part of the durable contract, rename it out of
the scratch convention and document why it belongs in source control.
