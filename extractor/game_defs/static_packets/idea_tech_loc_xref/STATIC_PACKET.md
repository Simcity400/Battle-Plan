# Idea/Technology Localisation Cross-Reference Packet

Artifact: `idea_tech_loc_xref_static`

Generated payload: `extractor/game_defs/out/idea_tech_loc_xref/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json`

Gate report: `extractor/game_defs/out/idea_tech_loc_xref/idea_tech_static_gate_report.json`

Coverage report: `extractor/game_defs/out/idea_tech_loc_xref/idea_tech_localisation_coverage.json`

## Schema

`idea_tech_loc_xref_static.static_artifact.v1`

- `tokens.ideas`: keyed by idea token; includes category, loc key, localised name, resolution rule, picture, and source metadata.
- `tokens.technologies`: keyed by technology token; includes loc key, localised name, resolution rule, folders/categories, desc loc key, enabled equipment/module fallback candidates, and source metadata.
- `omitted`: tokens with no safe definition-side loc key.
- `source_files`: relative definition file inventory with bytes and SHA-256.
- `coverage`: idea and technology coverage against R3 `static_localisation_join_r3/localisation_catalog.json`.
- `meta.syntax_allowlist_hits`: documented base-game syntax quirks accepted by the strict precheck. Current known entries are vanilla `common/ideas/persia.txt` and `common/ideas/switzerland.txt`, whose installed files are missing the final top-level `ideas` brace while the tolerant parser still recovers the complete block.
- `meta.wrapper_allowlist_hits`: documented wrapperless files accepted as handled inputs. Current known entries are mod-layer removed-country override stubs for Afghanistan, Chile, communist China, and Paraguay.

## Join Keys

- Static idea key: `tokens.ideas.<idea_id>`.
- Static technology key: `tokens.technologies.<technology_id>`.
- Localisation key: row `loc_key`; technology rows may also resolve through source-ordered `enable_equipments` / `enable_equipment_modules` candidates.

## Packing

Pack the payload as replay `role="static_artifact"` named `idea_tech_loc_xref_static`. This is a definition-side enrichment bundle only; it does not create a compact domain.
