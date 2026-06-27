# game_defs — mod-aware STATIC GAME-DATA parsers (binding-sweep bucket-(c))

Parses the **effective mod-over-vanilla** installed content (`common/` + `events/` +
`localisation/`) into structured JSON the native replay UI binds. This closes
**bucket (c)** of `replay_ui/BINDING_MAP_COVERAGE.md` §4c — the *static
definition / world data* the views need that isn't game state (memory) at all.

It works exactly like the existing focus-tree / GUI extraction: it **reuses**
`game_assets/extract.py` (the audited Clausewitz mod-over-vanilla resolver:
file-level override by relpath + `replace_path`) and the tolerant
`game_assets/paradox_parser.py`. No new override logic is invented here.

## Resolution model (mod-over-vanilla)

- Vanilla install: `BATTLE_PLAN_VANILLA` env var or `--vanilla` (no home path baked in).
- Mod set: `game_assets/active_modset.json` (the committed, path-redacted
  manifest) re-resolved against the live user-data dir, or `--mods A.mod,B.mod`.
- **`common/*` and `events/*`**: unit of override = the **file** (a mod file with
  the same relpath replaces the vanilla one; a new stem just adds). Duplicate
  object ids across surviving files = last-loaded (mod after vanilla) wins,
  collisions logged.
- **`localisation/*`**: override is by **KEY across all `.yml`** — every effective
  loc file is loaded vanilla-first then mod, and a later key wins (`LocDB`).

## Parsers (`build(vanilla=None, mods=None)` + CLI `__main__` each)

| module | source | output |
|---|---|---|
| `parse_events` | `events/*.txt` | id → title/desc/picture/options (loc-resolved) |
| `parse_scripted_gui` | `common/scripted_guis/*` | gui → element/variable/effect map (reads/writes, `visible_vars`, `[GET_X]` ↔ scripted_localisation) |
| `parse_tech` | `common/technologies/*` | tech → resolved render `position` (own folder xy `@var`-resolved, or variant sub-tech inheriting parent xy + slot offset; non-tree/hidden techs proven & flagged), `research_cost`, prereq/XOR + `leads_to` edges w/ coeff, `dependencies`, effect rows |
| `parse_doctrines` | `common/doctrines/*` | folder/grand/track/subdoctrine → derived grid `position` (track column + in-track row), explicit `xor`/`mutually_exclusive`/`replaces` + implicit grand-doctrine one-of groups, effects + rewards |
| `parse_catalogs` | many | occupation laws, leader traits, tactics, aces, operations, opinion mods, equipment modules/archetypes/upgrades, state categories, ideas/advisors/spirits, power-balance side/range thresholds |
| `parse_focus_def` | `common/national_focus/*` | focus → xy geometry, PP cost, prereq OR-groups, mutual-exclusion, loc |
| `parse_templates` | `history/units/*` | division/garrison templates + subunit stat table + derived equipment/manpower cost |
| `parse_world_defs` | loc + `common/{terrain,wargoals,military_industrial_organization}` | state/VP NAMES, terrain modifiers, casus-belli, MIO catalog |
| `parse_characters` | `common/characters/*` | modern advisors (idea_token/slot/traits/cost) + commanders + country leaders |

Shared spine: `resolver.py` (layer build + `LocDB`), `clausewitz_ops.py`
(operator + variable-op harvesting from the tolerant parser).

## Run

```sh
BATTLE_PLAN_VANILLA=<install> python game_defs/build_all.py     # -> game_defs/out/*.json
BATTLE_PLAN_VANILLA=<install> python -m pytest game_defs/tests -q
```

`out/` is git-ignored generated data (regenerate per mod set). The app loads the
JSON keyed by object id and joins it to the diff-engine's fired-event / tech /
focus ids and the per-country `script_variables` bag.
