# Decisions Static Definitions Packet

Artifact: `decisions_static_definitions`

Generated payload: `extractor/game_defs/out/decisions_static/decisions_static_definitions/decision_definitions_catalog.json`

Gate report: `extractor/game_defs/out/decisions_static/decisions_static_gate_report.json`

Coverage report: `extractor/game_defs/out/decisions_static/decision_localisation_coverage.json`

## Schema

`decisions_static_definitions.static_artifact.v1`

- `decisions`: keyed by decision id; includes category, loc key, localised name, icons, political-power cost, target metadata, cooldown/removal fields, visibility/availability metadata, `fire_only_once`, and source metadata.
- `categories`: keyed by category id; includes category icon/picture and visible/available/allowed metadata.
- `source_files`: relative definition file inventory with bytes and SHA-256.
- `coverage`: localisation coverage against R3 `static_localisation_join_r3/localisation_catalog.json`.

## Join Keys

- Static key: `decisions.<decision_id>`.
- Localisation key: `decisions.<decision_id>.loc_key`.
- Domain 16 live key: `countries[*].decisions.timed_live[*].decision`.
- Domain 45 live keys: `countries[*].decisions._r2_depth_events[*].decision` and `_r2_depth_targets[*].decision`.

Decode evidence: `world_fold.rs:3396-3397` reads/writes domain 16 `decision`; `world_fold.rs:9372-9383`, `9394-9405`, `10325`, and `10355` show D45 decision event/target strings.

## Packing

Pack the payload as replay `role="static_artifact"` named `decisions_static_definitions`. Missing/corrupt payloads should fail closed as absent decision definition joins, not as live decision-state failure.
