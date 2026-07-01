# Save Oracle — development save-file fixtures

The save oracle parses a Hearts of Iron IV save into clean JSON for development
fixtures, reverse-engineering anchors, and offline validation. It can expose
whole-world state from a save file — per-state **owner**, per-state and
per-province **controller**, country snapshots, divisions, and deep domains.

Runtime replay capture must not require saves. The live host `hoi4.exe` process,
via the in-process collector, is the only authoritative runtime source. Outputs
from this directory may be used to discover layouts, build fixtures, compare
development runs, and audit parser/schema behavior, but they must not silently
fill `.battleplan` gaps or substitute for live collector output.

Everything here is **offline, stdlib-only**. The only external process is
`rakaly`, used solely to melt binary saves to plaintext.

---

## Quick start

```bash
# Parse the most recently modified save found in the known locations:
python parse_save.py

# Parse a specific save (binary .hoi4 OR already-melted plaintext):
python parse_save.py "/path/to/AST_1941_07_05_07.hoi4"
python parse_save.py "/path/to/GER_1941_melted.txt"

# Point at a specific rakaly binary / output dir:
python parse_save.py --rakaly "/path/to/rakaly.exe" --out-dir ./output  save.hoi4
```

Outputs are written to `save_oracle/output/` (override with `--out-dir`).
The resolved save path and rakaly path are **printed at runtime** — no personal
path is baked into the source.

### Save discovery (when no path is given)
Discovered from `Path.home()` (and the `BATTLE_PLAN_SAVE_DIR` env override) — never
hardcoded. The search covers the standard HOI4 user save directory and a local
downloads directory for previously-melted `*_melted.txt` dumps.

`*.hoi4` saves and `*_melted.txt` plaintext dumps are considered; the most
recently modified wins.

---

## What rakaly does

HOI4 saves come in two flavours, distinguished by an 8-byte magic header:

| Header    | Meaning                | Handling                                   |
|-----------|------------------------|--------------------------------------------|
| `HOI4txt` | plaintext (Ironman off, or already melted) | parsed directly |
| `HOI4bin` | binary (compressed/Ironman) | melted to plaintext via `rakaly` first |

`rakaly melt --format hoi4 --to-stdout -u stringify <save>` converts the binary
token stream into the same plaintext Clausewitz format. The parser captures
rakaly's stdout to a temp file and parses that.

**rakaly discovery:** the `PATH`, `--rakaly` flag, `~/.cargo/bin`,
`~/AppData/Local/rakaly`, and `~/Downloads/**/rakaly.exe` are checked.
If rakaly is absent **and** the save is binary, the tool errors with a clear
message. Plaintext saves never need rakaly.

> **Known rakaly quirk (handled):** on modded saves rakaly may print a short
> warning to stderr (unknown modded token bytes) and **exit 1 while still
> producing a complete, valid melt**. The tool judges success by the *output*
> (starts with `HOI4txt`, substantial size), not the exit code, and prints a
> note when this happens.

---

## Output schema

### `state_ownership.json`
```jsonc
{
  "<state_id>": {
    "owner":      "TAG",   // authoritative state owner
    "controller": "TAG"    // state controller; falls back to owner if the save
                           // has no explicit state-level controller for it
  },
  ...
}
```

### `country_snapshot.json`
```jsonc
{
  "<TAG>": {
    "stability":       0.75,        // 0..1
    "war_support":     0.85,        // 0..1
    "political_power": 247.37,
    "command_power":   182.36,
    "ruling_party":    "fascism",
    "capital":         64,          // state id
    "major":           true,
    "manpower_ratio":  262500,      // AVAILABLE manpower pool (not total — see notes)
    "fuel":            655000.0,
    "max_fuel":        655000.0,
    "num_ships":       38,
    "factories": { "civ": 82, "mil": 280, "dockyard": 14 },  // summed from owned states
    "is_human":        true         // tag is in player_countries
  },
  ...
}
```

### `meta.json`
```jsonc
{
  "source_filename":       "GER_1941_melted.txt",
  "source_path_redacted":  "~/Downloads/GER_1941_melted.txt",  // home-relative, no user name
  "was_melted":            false,
  "save_date":             "1941.4.13.11",
  "save_version":          "Operation Postern v1.19.1.0.b49d (005c)",
  "player":                "GER",
  "player_countries":      ["GER", "ENG", ...],   // human slots (may repeat for co-op)
  "stats": {
    "states_parsed":             1081,
    "states_with_owner":         1081,
    "distinct_owner_tags":       31,
    "provinces_with_controller": 1032,
    "countries_parsed":          143,
    "countries_in_snapshot":     143
  }
}
```

---

## How it parses (and why)

A naive line-regex over a HOI4 save is unsafe: the **same numeric keys**
(`1={`, `2={`, …) appear inside `region={}`, `provinces={}`, **and** `states={}`,
and `provinces={...}` lists also appear nested inside combat/`front=` blocks. So
the parser is a **quote-aware streaming tokenizer** with **depth-zero section
recognition** (design reviewed with Codex):

1. **Tokenizer** — the only thing that interprets quotes and braces. Quotes and
   braces *inside* quoted strings (e.g. a name like `"A\"B"`) are literal text,
   so brace depth is never corrupted. Handles `\"`, `\\`, dates, negatives,
   `yes`/`no`, bare atoms, and number lists.
2. **Depth-0 driver** — only honours the `states` / `provinces` / `countries` /
   `player_countries` keywords when at top-level brace depth 0. Everything else
   is consumed with a **balanced `skip_value()`** so unknown structure can never
   throw the parser off.
3. **Section parsers** — per-section child walkers extract just the few fields
   we care about and skip the rest by brace counting.

Performance: ~35 MB melted save parses in **~3 seconds** (pure Python, no deps).

---

## Validation (real save: `GER_1941_melted.txt`, 1941.4.13.11)

```
states parsed : 1081        (matches the mod's land-state count exactly)
with owner    : 1081
owner tags    : 31 distinct
prov ctrl     : 1032 provinces with an explicit controller override
countries     : 143 parsed
contested     : 144 states where controller != owner (occupied territory)
```

Spot-checks (plausible):
```
state    64 -> owner=GER controller=GER     (Berlin)
state   126 -> owner=ENG controller=ENG
state     1 -> owner=ITA controller=ITA
state     6 -> owner=BEL controller=GER     (occupied Belgium)
state    10 -> owner=POL controller=GER     (occupied Poland)
```
Cross-checked against the raw save: 144 state-level `controller=` lines ↔ 144
contested states; 1081 state blocks ↔ 1081 parsed (the 9 extra raw `owner=`
lines are nested fields like `resistance.occupied_country_tag`, correctly
ignored at the wrong depth).

A synthetic fixture lives at `sample/sample_save.txt` (parses correctly,
including the escaped-quote and `front={ provinces=... states=8 }` decoys); use
it to test the parser without a real save:
```bash
python parse_save.py sample/sample_save.txt --out-dir /tmp/sample_out
```

---

## Known limitations

- **State-level controller IS present** in the saves observed here (good news —
  better than the design assumed), so `state_ownership.json` controller is
  authoritative when the save writes it. When a state has no explicit
  controller, it falls back to the owner. A *province→state* mapping
  (`map/definition.csv`) is **not** required for state control, and is out of
  scope; per-province control is exposed separately if needed (the parser reads
  it but the current output focuses on state-level).
- **`provinces_with_controller`** counts only provinces with an *explicit*
  `controller=` override in the save. Provinces without one default to their
  state's owner/controller (HOI4 has no province *ownership*, only state-level).
- **Factory totals** are summed from `buildings.{industrial_complex,arms_factory,
  dockyard}.level` over states **owned** by each tag. This is "factories located
  in owned states", not "currently operable factories" (occupation/war can make
  owned factories unusable — that needs control + occupation logic).
- **`manpower_ratio`** is the country's available manpower pool, not total
  recruitable population, deployed manpower, or losses (those need more data /
  game logic). Field is named to reflect that.
- **Victory points** captured per state only in the scalar `victory_points=`
  form; some engine versions use a list form, which is skipped. Native/replay
  consumers derive state totals from captured province/state data when needed.
- **`player_countries`** can list a tag more than once (co-op / multiple human
  slots on one country). `is_human` membership is still correct.

---

## Files

```
save_oracle/
├── parse_save.py            # state owner / controller + country snapshot (stdlib only)
├── parse_units.py           # DIVISIONS (order of battle) oracle      ← added
├── parse_control.py         # FULL province + state control oracle    ← added
├── README.md                # this file
├── sample/
│   └── sample_save.txt      # tiny synthetic plaintext fixture
└── output/                  # generated JSON (see schemas above + below)
```

---

## Division oracle — `parse_units.py`

Extracts the **fielded land divisions** (order of battle), joined to their
template name/role and equipment archetype names. Reuses `parse_save.py`'s
tokenizer; adds no risk to it.

```bash
python parse_units.py                          # latest save
python parse_units.py "/path/to/save.hoi4"     # explicit
python parse_units.py --out-dir ./output "/path/to/melted.hoi4"
```

**Where divisions live (verified):** `countries.<TAG>.units.division[]`. Each
country block has exactly one `units={ ... }` whose *direct* children
`division={ ... }` are the real fielded divisions. The parser descends only into
that direct `units={}` block (never recursing into other country sub-blocks), so
the `division={ id type }` **handle-ref decoys** inside
`experience_status.xp_by_template` are never seen; a `logical_country`-present
guard is applied as a second line of defence. Joins use the full Clausewitz
object key `(id, type)`, not bare id.

### `divisions.json`
```jsonc
{
  "meta": {
    "source_filename": "GER_1940_09_10_13.hoi4",
    "source_path_redacted": "~/.../save games/GER_1940_09_10_13.hoi4",
    "was_melted": true,
    "save_date": "1940.9.10.13",
    "save_version": "Operation Postern v1.19.1.0.b49d (d4f6)",
    "player": "GER",
    "stats": {
      "divisions": 1654,
      "distinct_owners": 19,
      "templates_in_save": 583,
      "equipment_archetypes": 2189,
      "divisions_template_resolved": 1654,    // template_id matched a template
      "divisions_with_template_name": 1654,   // name present (HQ templates fall back to localization_key)
      "template_resolve_pct": 100.0,
      "divisions_with_state": 1589,       // rest are at-sea / in-transit provinces
      "unknown_template_ids": 0,
      "unknown_equipment_ids": 0
    }
  },
  "by_country":  { "SOV": { "count": 354, "total_strength": 53750.36 }, ... },
  "by_template": { "<template_id>:<template_type>": {
                        "count": 78, "total_strength": 11829.09,
                        "name": "Infantry 1936 L", "role": "infantry",
                        "country": "GER" }, ... },   // keyed by full id:type
  "divisions": [
    {
      "div_id": 2279, "div_type": 4713,
      "owner": "GER",                     // logical_country
      "location_province": 11596,
      "state_id": 23,                     // joined via province_to_state.json (null if at sea / unmapped)
      "template_id": 878, "template_type": 52,
      "template_name": "Motorized Infantry 1936 C",
      "template_role": "motorized",
      "organisation": 99.87856,
      "strength": 229.4,
      "equipment": [
        { "archetype": "artillery_equipment_2", "equip_id": 1912, "equip_type": 70, "amount": 12.0 },
        ...
      ]
    },
    ...
  ]
}
```

`equipment[].archetype` is resolved by joining the division's equipment
`{id,type=70}` into the top-level `equipments={ <archetype_name>={ id=... } }`
index. Any unresolved id is left as `archetype: null` and counted in
`unknown_equipment_ids` (0 on the validated save).

---

## Province-control oracle — `parse_control.py`

Reconstructs the **complete** per-province controller map and per-state
owner/controller from the SPARSE save data, using the documented fallback:

```
state_controller[state]   = states.<id>.controller  if present else states.<id>.owner
province_controller[prov]  = provinces.<id>.controller            if present
                             else state_controller[ province_to_state[prov] ]
```

`province_to_state.json` (read-only, from `map_pipeline/assets/`) supplies the
province→state join. Reuses `parse_save.parse_save_text` for the raw state/
province parse.

```bash
python parse_control.py                        # latest save
python parse_control.py "/path/to/melted.hoi4"
```

### `province_control.json`
```jsonc
{ "<province_id>": "TAG_or_null", ... }   // dense: an entry for EVERY mapped province
```

### `state_control.json`
```jsonc
{ "<state_id>": { "owner": "FRA", "controller": "GER" }, ... }
```

### `control_meta.json`
```jsonc
{
  "source_filename": "...", "save_date": "1940.9.10.13", "was_melted": true,
  "stats": {
    "states": 1081, "states_with_owner": 1081,
    "raw_state_controller_overrides": 83,    // explicit state-level overrides
    "province_overrides_in_save": 653,       // explicit province-level overrides
    "provinces_in_map": 10272, "provinces_emitted": 10272,
    "provinces_with_controller": 10272,      // ≈ all land provinces
    "provinces_via_override": 653, "provinces_via_state_fallback": 9619,
    "provinces_unresolved": 0,               // sea/lake/wasteland/no-owner -> null
    "province_overrides_outside_map": 0
  }
}
```

---

## Validation (real save `GER_1940_09_10_13.hoi4`, 1940.9.10.13)

**Divisions:**
```
divisions      : 1654          (matches expected ~1654 active land divisions)
distinct owners: 19
templates      : 583 in save, 1654 template_ids resolved (100%), 1654 named
                 (3 army-HQ templates carry a localization_key, not a name= — used as fallback)
equipment      : 2189 archetypes; 0 unknown equip ids; 0 unknown template ids
div w/ state   : 1589          (the remaining ~65 are at-sea / in-transit provinces)
top owners     : SOV 354, ENG 269, ITA 223, JAP 160, GER 126, USA 98, HUN 62, RAJ 56
sample         : #2279 GER prov 11596 (state 23) "Motorized Infantry 1936 C" / motorized,
                 org 99.88, str 229.4, eq0 artillery_equipment_2
```

**Province control:**
```
states         : 1081 (1081 with owner)
raw state ctrl : 83 explicit state-level controller overrides   (= 83 contested states)
prov overrides : 653 explicit province controller overrides
provinces      : 10272 mapped, 10272 with a resolved controller, 0 unresolved
               : 653 via province override, 9619 via state fallback
spot-check     : every province of state 14 (FRA owner, GER controller — occupied France)
                 resolves to controller=GER in province_control.json
```
On this save the 83 raw state-level controller overrides exactly match the 83
contested states (`controller != owner`) — a smoke test that the reused parse
exposes *raw* (not fallback-filled) state controllers. The robust invariant is
`contested_states <= raw_state_controller_overrides` (a save could in principle
write an explicit `controller=owner`, which would be a non-contested override).

---

# Deep layers — exhaustive save fixtures

The base parsers above give owner / controller / divisions / a cheap
snapshot from a save fixture. The *deep-layer* modules below parse every
remaining data layer a replay reviewer could want, into structured JSON under
`output/`. They all
**reuse `parse_save.py`'s tolerant tokenizer** (via `deep_common.py`) and add
nothing to it; each does a single top-level streaming pass and joins `(id,type)`
Clausewitz keys.

> **Honesty over coverage.** Every module reports what it *could* and *couldn't*
> extract per domain (each output's `meta.notes`), and never emits a guessed
> value. Gaps that are live-memory-only are marked explicitly, not invented.

Run any of them like the base parser:
```bash
python parse_focus.py            # most-recent save
python parse_tech.py "/path/to/save.hoi4"
python parse_production.py --rakaly "/path/to/rakaly.exe" save.hoi4
python parse_economy.py
python parse_diplo.py
python parse_state_detail.py
python parse_combats.py
```

`deep_common.py` holds the shared scaffolding (typed readers, a generic
top-level streaming pass, the CLI runner) so each domain module stays focused on
its own section. It is a helper, not a parser — it imports and re-exports
`parse_save`'s primitives.

## `focus.json` — national focus (`parse_focus.py`)
Per country: `focus_tree`, `continuous_focus_palette`, `completed` (list of focus
ids **in completion order** — the engine appends each as it completes),
`completed_count`, `current`, `progress` (whole-focus counter on the current
focus), `paused`.
- Key path: `countries.<TAG>.focus_tree`, `countries.<TAG>.focus.{completed
  (repeated), current, progress, paused}`.
- **Gap:** completion *dates* are not in the save — only the order is recoverable.

## `tech.json` — research / technology (`parse_tech.py`)
Per country: `technologies` (list of `{name, level, date, research_points}`),
`tech_count`, `in_progress` (active research slots, each `{name, points_factor}`),
`research_slots` / `original_research_slots`, `research_bonuses` (merged
`limited_use_bonus` + `limited_use_cost_reduction_bonus`, each with name,
bonus/ahead_reduction/claim/cost_reduction/uses + categories).
- Key paths: `countries.<TAG>.technology.technologies.<tech>`, `…technology.slots.<tech>.points_factor`,
  `…technology.limited_use_bonus`, `…technology.limited_use_cost_reduction_bonus`,
  `countries.<TAG>.research_slot` / `original_research_slots`.
- Active techs also appear in `technologies` with `date=1.1.1.1`, no level,
  and (in current fixtures) accumulated `research_points`. `parse_tech.py`
  keeps the raw facts; `models.research_progress` computes fraction/ETA by
  joining them to static tech costs and fails closed when an input is absent.

## `production.json` — production & equipment (`parse_production.py`)
Top-level `equipment_archetypes` (the `(id:type) -> {name, creator, obsolete,
is_frame}` variant index everything joins to). Per country: `equipment_lines`
(military + naval + ship-refit lines, each `{kind, line_id, equip_id/type,
archetype, produced, active_factories, requested_factories, priority, amount,
speed, cost}`), `stockpile` (on-hand equipment by archetype+amount), `convoys`,
`factory_totals` (mil/naval factories assigned).
- Key paths: top-level `equipments.<archetype>={id={id type} creator obsolete}`,
  `countries.<TAG>.production.{military_lines,naval_lines,ship_refit_lines}`,
  `…production.equipments.equipment={id={id type} amount}`,
  `countries.<TAG>.convoys.equipment.equipment`.
- **Note:** per-line `produced` is the line's lifetime accumulator; there is no
  per-country lifetime "equipment produced" total in the melt. 58/143 countries
  carry production data (the rest are minor tags with empty blocks).

## `economy.json` — economy / resources / laws (`parse_economy.py`)
Per country: `factories` (`civ/mil/dockyard` = owned-state buildings **+** off-map
`buildings`, with `*_from_states` matching `country_snapshot.json`, plus
`under_construction`), `construction_queue` (civil builds from
`production.general_lines.building` — `{template, location_state,
active_factories, produced, cost, priority, created_date}`), `resources`
(`produced`, `exported`, `to_export`, and derived `available = produced -
exported`), `fuel` (`current`/`max`), `laws` (economy / trade / conscription,
classified out of the flat ideas list), `ideas_all` + `ideas_other`
(spirits/advisors/traits), `stability`, `war_support`, `political_power`,
`command_power`, `ruling_party`, `available_manpower`.
- Key paths: `states.<id>.buildings.*.level`, `countries.<TAG>.buildings.*.level`,
  `…production.general_lines.building`, `…resources.{produced,exported,to_export}`,
  `…fuel_status.{fuel,max_fuel}`, `…politics.ideas` (flat tokens),
  `…politics.political_power`, `…{stability,war_support,command_power}`,
  `…manpower.ratio`.
- **Gap:** no literal per-resource `available` key (derived); laws/spirits/
  advisors are all flat tokens in `politics.ideas` (classified by substring).

## `diplomacy.json` — diplomacy & intel (`parse_diplo.py`)
Top-level: `factions` (`{id, ideology, icon, members[], leader (=first member),
template, leader_change_date}`), `wars` (de-duplicated pairwise from
`war_relation`: `{a, b, start_date, a/b_casualties, war_score_a_vs_b/b_vs_a}`),
`subjects` (`{overlord, subject, autonomy_state, value, start_date}` from
`puppet`). Per country: `faction_id`, `relations` (per other tag: `attitude`,
`opinion_sum`, `opinion_modifiers`, and boolean `guarantee` / `non_aggression_pact`
/ `military_access` / `embargo` / `at_war`), `wargoals`
(`{type, target, states[], expire}`), `agency_name` + `operatives`
(`{name, skill, experience, traits[], state, nationalities[]}`), `intel_rows`.
- Key paths: top-level `faction={…members…}`,
  `countries.<TAG>.diplomacy.active_relations.<OTHER>.{attitude, cached_sum,
  opinion, guarantee, non_aggression_pact, military_access, embargo,
  war_relation, puppet}`, `…diplomacy.wargoals.<type>`,
  `countries.<TAG>.intelligence_agency.{name, operative}`.
- **Gaps:** no top-level `war={}` section (wars are pairwise `war_relation`,
  collapsed); `war_score_*` reported = the `province_capture` component
  (dominant term). `intel.intel` is a **positional** array by internal country
  index, not tag — only the row count is reported (index→tag is a live-layer
  join we don't invent).

## `state_detail.json` — state buildings & occupation (`parse_state_detail.py`)
Per state: `owner` / `controller`, `buildings` (state-level levels: infra, civ/
mil/dock, air_base, radar, AA, synthetic_refinery, fuel_silo, …),
`province_buildings` (forts / naval_base / coastal_bunker / supply_node /
rail_way summed from the state's provinces), `victory_points` (summed from
provinces), `manpower` (`available`/`locked`/`total`), `resistance`
(`resistance`, `compliance`, speeds, `occupied_country_tag`,
`operational_status`), `state_category`, `demilitarized`.
- Key paths: `states.<id>.{owner, controller, buildings.*.level,
  manpower_pool.*, resistance.*, state_category, demilitarized}`,
  `provinces.<pid>.{victory_points, buildings.*.level}` joined via
  `map_pipeline/assets/province_to_state.json`.
- **Gap:** per-state **supply** is runtime-computed; `supply_node` levels (in
  `province_buildings`) are the in-save proxy. VP and forts/naval bases live on
  *provinces* and are folded into their state.

## `combats.json` — active combats (`parse_combats.py`)
List of active battles: `{combat_id, kind, location, day, duration, terrain,
attacker, defender}`; each side `{units[(id,type)], unit_count, losses,
air/ground_damage_str/org, tactic, progress}`.
- Key paths: `combat.land_combat.{id, location, day, duration, terrain,
  attacker, defender}`, side `…attacker.unit` (repeated) + `…attacker.log.progress`.
- **Gap (important for the high-freq combat work):** the melt exposes **only
  `land_combat`**. Naval and air battles are transient and are **not persisted**
  to saves — they are live-memory only. The parser scans for
  `naval_combat`/`air_combat` keys anyway (future-proof) and reports they were
  absent. Side `progress` is a per-side accumulating engine figure, not a
  0-100 percent; unit ids are handle refs that join to `divisions.json` by id.

## Deep-layer validation (real save `GER_1940_09_10_13.hoi4`, 1940.9.10.13)
```
focus      : 143 countries w/ focus; GER 55 completed, current GER_the_swiss_gold (progress 12)
tech       : 143 countries, 2747 techs total; GER 122 techs, 5 slots, 5 in-progress, 20 bonuses
production : 2189 archetypes (0 unknown joins); 58 countries; GER 24 lines (183 mil + 29 naval fac), 58 stockpile
economy    : 143 countries, 119 build lines; GER civ110/mil131/dock16 (108/131/16 from states + 2 offmap),
             laws economy=war_economy conscription=extensive_conscription, fuel 849000, stab .75
diplomacy  : 4 factions (Axis 6 / Allies 15 / Comintern 1 / USA-Commonwealth 1), 75 wars, 5 subjects;
             GER 143 relations, Abwehr agency, 8 operatives, 6 wargoals, 15 wars (incl. GER vs POL since 1939.9.2)
state_detail: 1081 states (all owned), 400 occupied; state 64 (Berlin) megalopolis, vp56, full building set
combats    : 13 active battles, all land (naval/air not persisted); combat 5457 @prov 12740 (hills)
```
The economy factory cross-check matches the base parser exactly: `parse_save`
sums GER state factories to `civ108/mil131/dock16`, identical to economy.json's
`*_from_states`. (`country_snapshot.json` on disk was generated from a *different*
1941 save, so its raw numbers differ — not a discrepancy.)
