"""game_defs — mod-aware STATIC GAME-DATA parsers (binding-sweep bucket-(c)).

Parses the effective mod-over-vanilla installed content (common/ + events/ +
localisation/) into structured JSON the replay UI binds, exactly like the
existing focus-tree / GUI extraction, reusing game_assets/extract.py for
mod-over-vanilla resolution.

Parsers (each has a ``build(vanilla=None, mods=None)`` and a CLI ``__main__``):
  parse_events       — events/*.txt: id -> title/desc/picture/options (+loc)
  parse_scripted_gui — common/scripted_guis/*: element -> variable/effect map
  parse_tech         — common/technologies/*: node geometry + prereq/XOR + effects
  parse_doctrines    — common/doctrines/*: grand/track/subdoctrine catalog + effects
  parse_catalogs     — occupation laws, traits, tactics, aces, operations,
                       opinion mods, equipment modules/archetypes/upgrades,
                       state categories, ideas/advisors/spirits
  parse_focus_def    — common/national_focus/*: focus geometry + PP cost + loc
  parse_templates    — history/units/*: division/garrison templates + subunit
                       defs + derived equipment/manpower cost
  parse_world_defs   — state/VP NAMES (loc), terrain, wargoals/CB, MIO catalog
  parse_characters   — common/characters/*: modern advisors (idea_token/slot/
                       traits/cost) + commanders + country leaders

Shared spine:
  resolver           — mod-over-vanilla layer build + LocDB key-level loc merge
  clausewitz_ops     — operator + variable-op harvesting from the tolerant parser
"""
