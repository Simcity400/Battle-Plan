# Project Vision

This repo is a private, read-only the game replay recorder and viewer
for the owner's own multiplayer group.

The goal is a full-game replay: economy, military, divisions, provinces,
combat, focuses, decisions, diplomacy, logistics, map state, UI-relevant state,
and enough provenance to know exactly what was captured, rejected, or blocked.

## Active Product Path

The production recording path is:

```text
hoi4.exe -> one injected DLL collector/coordinator -> typed compact records -> Rust recorder -> .battleplan
```

Python, saves, and memory-reader scripts are development/proof/validation tools.
They are not the production hot extraction loop.

## Source Of Truth

- Extraction completion and remaining backlog:
  `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
- Compact domain IDs:
  `extractor/documents/DOMAIN_REGISTRY.md`
- Reverse-engineering/tooling setup:
  `extractor/documents/REVERSE_ENGINEERING_TOOLING.md`
- Replay file/container:
  `replay_interface/documents/HOI4REPLAY_CONTAINER_SPEC.md`
- Replay snapshot schema:
  `replay_interface/documents/REPLAY_SCHEMA.md`
- Replay map/UI standards:
  `replay_interface/documents/MAP_REPLICA_PLAN.md` and
  `replay_interface/documents/UI_SOURCE_OF_TRUTH_REPORT.md`
- Agent operating rules:
  `documents/AGENTS.md`

Historical plans, old status reports, research notes, and lane packets live
under `documents/archive/` or `extractor/documents/archive/`. They are evidence,
not active direction.
