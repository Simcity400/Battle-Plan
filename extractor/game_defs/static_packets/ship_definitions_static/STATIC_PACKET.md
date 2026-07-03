# Ship Definitions Static Packet

Artifact: `ship_definitions_static`

Generated payload: `extractor/game_defs/out/ship_definitions_static/ship_definitions_static/ship_definitions_catalog.json`

Gate report: `extractor/game_defs/out/ship_definitions_static/ship_definitions_gate_report.json`

Coverage report: `extractor/game_defs/out/ship_definitions_static/ship_definitions_localisation_coverage.json`

## Schema

`ship_definitions_static.static_artifact.v1`

- `ship_hulls`: keyed by hull/equipment token from `common/units/equipment/ship_hull*.txt`; includes type, archetype, interface category, module slots, default modules, module count limits, resources, stats, loc key/name, and source metadata.
- `ship_modules`: keyed by ship module token; includes category, GUI category, parent, abbreviation, stat deltas, conversion blocks, critical parts, loc key/name, and source metadata.
- `source_files`: relative definition file inventory with bytes and SHA-256.
- `coverage`: hull/module localisation coverage against R3 `static_localisation_join_r3/localisation_catalog.json`.

## Join Keys

- Static hull key: `ship_hulls.<definition_token>`.
- Static module key: `ship_modules.<module_token>`.
- Live ship key: `order_of_battle.ships[*].definition`.

Decode evidence: `world_fold.rs:2742-2747` writes ship JSON `definition`; `world_fold.rs:3802-3808` reads and validates the optional `ships definition` string; `world_fold.rs:3839-3843` stores it; `world_fold.rs:8625-8634` defines `ShipState.definition`.

## Packing

Pack the payload as replay `role="static_artifact"` named `ship_definitions_static`. Domain 29 keeps owning live ship state; this bundle resolves static hull/module definition tokens.
