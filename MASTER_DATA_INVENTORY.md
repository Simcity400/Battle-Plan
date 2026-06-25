# MASTER DATA INVENTORY — Battle Plan completeness checklist

> **The bar:** record EVERY single part of the game — no gaps. This is the definitive
> field-level cross-reference of the **totality** of the game's data, enumerated from the three
> sources that together ARE the game's data:
>
> 1. **DISPLAYED** — `interface/*.gui` windows (the effective set = vanilla overridden by the
>    25 mod-override files). Every `instantTextBoxType` / `iconType` / `guiButtonType` is a data
>    element the player sees. 130 vanilla windows; 25 mod overrides.
> 2. **STORED** — the melted-save key tree, enumerated from `save_oracle/**` (16 deep parsers +
>    20 already-parsed `output/*.json`). The save is the offline backstop oracle.
> 3. **LIVE** — the cracked `hoi4.exe` memory object layouts (`memory_reader/**`:
>    `layout.16022851.json` + per-domain `*_offsets.draft.json` + readers). Build-gated on
>    `exe 16022851`. Read-only.
>
> Cross-referenced against the **replay schema** (`replay/schema.py` + `replay/SCHEMA.md`) and
> the **diff engine** (`replay/diff_engine.py` event types) to produce the in-replay column.
>
> Read-only analysis, 2026-06-25. Companion to `FULL_GAME_COVERAGE_VERIFIED.md` (system-level
> ledger) — this doc drills to the FIELD level, adds the DISPLAYED (.gui) axis the ledger lacked,
> and is the authoritative gap map so discovery is no longer reactive.

---

## STATUS VOCABULARY

| Status | Meaning |
|---|---|
| **CAPTURED** | wired end-to-end into snapshots (a `schema.py` field + an ingesting parser/reader feeds it) |
| **READABLE-NOT-WIRED** | we can read it from a source (save parsed it OR memory reaches it) but schema/store/UI doesn't surface it |
| **DERIVED-NEEDS-MODEL** | a computed/displayed value (supply-remaining, air-sup %, naval stat rows, victory score) — needs a formula, not a new read |
| **WALL** | genuinely unobtainable; the limit is cited |
| **TODO-ENUMERATE** | high-cardinality area flagged but not fully field-expanded here (the per-element catalog is finite but huge) |

**Source legend** (mirrors `schema.py` FIELD_SOURCES): **M** live memory · **M\*** live but proxy/unvalidated-as-published · **S** save · **D** derived · `—` not available in that channel.

---

## HEADLINE COUNTS

Counts are at the **field-group** granularity used in §B (a per-entity field SET — e.g. "per-ship
fields" — counts as one row; the shape is expanded inline). Tallied across 34 game areas.

| Metric | Count |
|---|---|
| **Game AREAS enumerated** (provably complete vs the .gui list + save key tree) | **34** |
| **Field-groups inventoried** (rows in §B) | **~196** |
| **CAPTURED** (schema field + ingesting feeder) | **~118** |
| **READABLE-NOT-WIRED** (parsed/reachable, not surfaced) | **~38** |
| **DERIVED-NEEDS-MODEL** (needs a formula) | **~17** |
| **WALL** (cited hard limit) | **6** |
| **TODO-ENUMERATE** (finite but huge per-element catalogs) | **~17** |

The big takeaway, unchanged from the system ledger but now field-precise: **almost nothing is
missing at the read level** for the core campaign — the gaps are (a) downstream **wiring**
(market→store, sidecars→schema, live-domain fold), (b) **whole systems never given a parser**
(special projects/scientists, MIO traits surfacing, nukes stockpile, game rules, world-tension
contributors, script variables/flags), and (c) the 6 walls.

---

## TOP-LEVEL STRUCTURE — the 34 game areas (provably complete)

Derived from the union of: the 130-window `.gui` list, the 20-file save output tree, and the
memory layout roots. Each is expanded to the field level in §B.

```
A. NATIONAL / COUNTRY-SCOPED
   1  Top bar (scalars strip)
   2  Politics  (laws · ideas · cabinet/advisors · popularity · stab/WS · PP/CP · power balance)
   3  National focus tree
   4  Research / tech / doctrine trees
   5  Production lines + construction queue + equipment overview
   6  Designers (division template · ship · tank · plane · equipment-upgrade variants)
   7  MIO — Military Industrial Organisations (traits · policies · funds)
   8  Special projects / facilities / scientists  (prototype rewards)        [NEW vs ledger]
   9  Logistics / supply / fuel / convoys
   10 Trade / equipment-market / lend-lease / expeditionary
   11 Recruit / deploy / manpower / training
   26 Statistics / ledger / intel ledger
   27 Score / victory conditions

B. MILITARY
   12 Army divisions (template · equipment · org · strength · xp · movement · combat-state)
   13 Commanders / generals / FM / admirals / traits / officer corps / aces
   14 Navy (fleets → task forces → ships)
   15 Air (wings · missions · air superiority · airfields · aces)
   16 Combat (land + naval + air)
   29 Battle plans / fronts / orders / theatres
   33 Railway guns                                                            [NEW vs ledger]
   34 Garrison log / resistance attacks                                       [NEW vs ledger]

C. WORLD / DIPLOMACY
   17 Diplomacy (wars · war goals · factions · subjects · autonomy · guarantees · NAP · opinion · casualties · war score)
   18 Intel (agency · operatives · operations · decryption · networks · counter-intel)
   24 Peace conference (bids · score)
   25 Nukes (stockpile · reactors · launches)

D. MAP / TERRITORY
   19 Decisions / missions / national spirits
   20 States (owner · controller · cores · claims · buildings · VP · resources · manpower · category · occupation · resistance · compliance)
   21 Provinces (controller · supply · terrain · buildings · weather-host)
   22 Weather (precip · storm · mud · frozen)
   23 Map modes

E. META / SESSION
   28 Players (is_human · which TAG)
   30 Game rules / scenario / mod metadata
   31 Events / timeline / message feed / notifications
   32 Achievements / career / awards (account metadata)
```

---

## B. THE MATRIX — area > panel/object > FIELD | displayed | stored | live | in-replay | STATUS

Columns: **DISPLAYED** = .gui file (basename; `M`=mod-override wins, `V`=vanilla) · **STORED** =
save key path → `parser → json` · **LIVE** = offset/reader · **REPLAY** = `schema.py` field (or
`—`) · **STATUS**.

### AREA 1 — TOP BAR
.gui: `topbar.gui`(M) `topbarresources.gui`(M) `topbarvp.gui`(M) `ingameclock.gui`(V) `menubar.gui`(V) `minimap.gui`(V) `alerts.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Political power | topbar | `.politics.political_power`→country_snapshot | rec+0x20 (43/43) | `politics.political_power` | CAPTURED |
| Stability % | topbar | `.stability`→country_snapshot | obj+0x10D0 (43/43) | `politics.stability` | CAPTURED |
| War support % | topbar | `.war_support` | obj+0x10D8 (43/43) | `politics.war_support` | CAPTURED |
| Command power | topbar | `.command_power` | obj+0x1F0 (43/43) | `politics.command_power`,`military.command_power` | CAPTURED |
| Manpower (available) | topbar | `.manpower.ratio`→country_snapshot | live avail (M*) | `military.manpower.available` | CAPTURED |
| Industrial capacity + mil/civ ratio | topbar | factories civ/mil→country_snapshot | — (country submanager, not pinned) | `economy.factories.{civ,mil}` | CAPTURED (save), READABLE-NOT-WIRED (live) |
| Convoys + ratio | topbar | `.convoys`→production | — | `military.convoys` | CAPTURED |
| Fuel + ratio | topbar | `.fuel_status.{fuel,max_fuel}` | live (M*) | `economy.fuel.{current,max}` | CAPTURED |
| Supply (global strip) | topbar | — (runtime) | division-province proxy | `supply.by_province` (M*) | DERIVED-NEEDS-MODEL (global number) |
| Nukes count | topbar | — **(unparsed)** | — | — | READABLE-NOT-WIRED → see Area 25 |
| World tension / threat | topbar | — **(unparsed)** | — | — | READABLE-NOT-WIRED (save `world_tension`, no parser) |
| Faction initiative + icon | topbar | faction→diplomacy | — | `diplomacy_world.factions` | CAPTURED |
| Date text | ingameclock/topbar | `date` (meta) | clock u32 hrs-since-epoch (solved) | `date`,`hour_index` | CAPTURED |
| Game speed (1-5) / pause | topbar | — (session UI) | — | — | WALL (renderer-only; not in save, §B5) |
| Decision alert counts (timeout / to-take) | topbar | derivable from decisions | — | — (no count surfaced) | DERIVED-NEEDS-MODEL |
| Intel-ops counts (prepared / available) | topbar | operations→intel | — | `intel.operations` | CAPTURED (counts DERIVED) |
| Prototype/reward count | topbar | — **(unparsed — special projects)** | — | — | READABLE-NOT-WIRED → Area 8 |
| Equipment-contract counts | topbar | equipment_market→market | — | (market unwired) | READABLE-NOT-WIRED → Area 10 |
| Resources strip (6: Al/Rub/Tun/Steel/Chr/**Coal**) | topbarresources | `.resources.*`→economy | — | `economy.resources.*` | CAPTURED (mod has 6-resource set, no Oil row) |
| Victory points total + ticking gain | topbarvp | VP→state_detail (summed) | — | `map.vp_by_state`,`states.*.victory_points_total` | CAPTURED |
| End-game score ladder (army/navy/air str, industry, territory, rank) | menubar | partial (no aggregate score) | — | — | DERIVED-NEEDS-MODEL → Area 27 |

### AREA 2 — POLITICS
.gui: `countrypoliticsview.gui`(V) `powerbalanceview.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Ruling party / ideology pie | politicsview | `.politics.ruling_party`→country_snapshot,economy | — | `politics.ruling_party` | CAPTURED |
| Party popularities | politicsview | `.politics.parties`→characters/economy | — | `politics.party_popularities` | CAPTURED |
| Economy / trade / conscription laws | politicsview | `.politics.ideas`(classified)→economy | — | `politics.laws.{economy,trade,conscription}` | CAPTURED |
| National spirits / ideas / advisors | politicsview | ideas tokens→economy; advisors→characters | — | `politics.ideas_all`,`ideas_other`,`cabinet`,`commanders.advisors` | CAPTURED |
| Active focus goal + progress + PP cost | politicsview | focus→focus | live current/progress (M*) | `focus.current`,`focus.progress` | CAPTURED |
| Election date / "no elections" | politicsview | `.politics` (election date) | — | — | READABLE-NOT-WIRED (date in save, not parsed) |
| Subject autonomy bar + level | politicsview | puppet→diplomacy subjects | — | `politics.autonomy`,`diplomacy_world.subjects` | CAPTURED |
| Exile legitimacy / collaboration manpower | politicsview | `.collaboration` (empty in sample) | — | — | READABLE-NOT-WIRED (exile/collab block unparsed) |
| **Power balance** value/needle/sides/decisions | powerbalanceview | `.power_balance`-class **(unparsed)** | — | — | READABLE-NOT-WIRED (BoP system has no parser) |

### AREA 3 — NATIONAL FOCUS
.gui: `nationalfocusview.gui`(M)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Focus tree id | focusview | `.focus_tree`→focus | — | `focus.focus_tree` | CAPTURED |
| Completed focuses (ordered) | focusview | `.focus.completed`(repeated)→focus | — | `focus.completed`,`completed_count` | CAPTURED + `focus_completed` event |
| Current focus + progress + paused | focusview | `.focus.{current,progress,paused}` | live (M*) | `focus.current/progress/paused` | CAPTURED |
| Continuous focus palette | focusview | `.continuous_focus_palette`→focus | — | `focus.continuous_focus_palette` | CAPTURED |
| Focus-tree GRID layout (icons, prereq/XOR links, tile states) | focusview | static def (mod focus files, not save) | — | — | TODO-ENUMERATE (static tree topology — finite, from `common/national_focus`, render context not state) |

### AREA 4 — RESEARCH / TECH / DOCTRINE
.gui: `countrytechnologyview.gui`(V) `countrytechtreeview.gui`(M) `countrydoctrinetreeview.gui`(M)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Completed technologies (+level/date/points) | techview/tree | `.technology.technologies`→tech | — | `research.technologies`,`tech_count` | CAPTURED + `tech_researched/lost` |
| Research slots in-progress (tech, ETA, %) | techview | `.technology.slots`→tech | live in_progress (M*) | `research.in_progress` | CAPTURED |
| Research slots count + original | techview | `.research_slot`,`.original_research_slots` | — | `research.research_slots`,`original_research_slots` | CAPTURED + `research_slot_gained` |
| Research bonuses (cat, value, uses) | techview | `.technology.limited_use_*`→tech | — | `research.research_bonuses` | CAPTURED |
| Overall research speed % | techview | — (derived) | — | — | DERIVED-NEEDS-MODEL |
| Tech-sharing group entries | techview | `technology_sharing` **(unparsed)** | — | — | READABLE-NOT-WIRED |
| Doctrine/tech-tree GRID + ahead-of-time penalty | tech/doctrine tree | static def | — | — | TODO-ENUMERATE (static topology + render-time penalty) |

### AREA 5 — PRODUCTION / CONSTRUCTION / EQUIPMENT
.gui: `countryproductionlineview.gui`(V) `countryconstructionsview.gui`(V) `equipmentoverview.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Production lines (equip, variant, factories, efficiency, output, target, priority) | productionview | `.production.{military,naval,refit}_lines`→production | — | `economy.gross_ic_by_equipment` + designs join | CAPTURED + `production_milestone` |
| Equipment archetypes catalog | productionview | top `equipments`→production/designs | — | `designs.equipment_variants` | CAPTURED |
| Equipment stockpile | equipmentoverview | `.production.equipments`→production | div equip structurally mapped, not extracted | `military.equipment_stockpile` | CAPTURED (save); READABLE-NOT-WIRED (live) |
| Construction queue (location, building, %, factories, repair) | constructionsview | `.production.general_lines`→economy | — | `economy.construction_queue` | CAPTURED + `factory_milestone` |
| Factories civ/mil/dock (+offmap/under-construction) | construct/topbar | states+countries buildings→country_snapshot,economy | — (submanager unpinned) | `economy.factories.*` | CAPTURED (save) |
| Naval repair queue (per-ship dockyards, damage) | productionview | (ship_refit_lines)→production | — | partial (no per-ship repair) | READABLE-NOT-WIRED (refit lines parsed, repair detail not surfaced) |
| Per-line efficiency growth/cap, bombing defence | productionview | derivable | — | — | DERIVED-NEEDS-MODEL |
| Consumer goods / traded / project factories | constructionsview | economy partial | — | partial | READABLE-NOT-WIRED |

### AREA 6 — DESIGNERS (variants)
.gui: `divisiondesignerview.gui`(M) `equipmentdesignerview.gui`(V,ships) `tank_designer_view.gui`(M) `plane_designer_view.gui`(V) `equipmentupgradedesignerwindow.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Division templates (regiment/support grid, role, cap, priority, obsolete) | divdesigner | top `division_templates`→designs | template_id only (+0x3A8); grid unwired | `designs.division_templates` | CAPTURED (save) |
| Equipment variants (modules, upgrades, version, parent, design team) | ship/tank/plane designer | `equipments`+`scheduled_equipment_variants`→designs | — | `designs.equipment_variants`,`scheduled_variants` | CAPTURED + `design_created` |
| Division template DERIVED combat stats (soft/hard atk, def, org, HP, width) | divdesigner BASE/COMBAT grid | **NOT stored** (runtime-computed from layout) | — | — | DERIVED-NEEDS-MODEL (compute from regiment grid + equip stats) |
| Equipment-variant DERIVED stat grids (speed/armor/piercing/etc.) | designers BASE/COMBAT/MISC | module sums stored; final stat derived | — | partial (modules captured) | DERIVED-NEEDS-MODEL |
| Per-upgrade slider levels (armor/gun/reliability/…) | upgradedesigner | `upgrades{}`→designs | — | in `equipment_variants.upgrades` | CAPTURED |

### AREA 7 — MIO (Military Industrial Organisations)
.gui: `industrial_organization_list.gui`(V) `industrial_organization_detail.gui`(V) `industrial_org_policy.gui`(V) *(surfaced by Codex/file-audit; not in original named set)*

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| MIO id/name/icon/level/funds/points/research_bonus | MIO list/detail | top `industrial_organisations`→designs | — | `countries.*.mios` | CAPTURED + `mio_leveled` |
| MIO task capacity, unlocked traits, allowed policies | MIO detail | `.task_capacity,.unlocked_traits,.allowed_policies`→designs | — | `mios[*]` subfields | CAPTURED |
| MIO assigned equipment / archetype | MIO list | `.assigned_equipment*`→designs | — | `mios[*]` | CAPTURED |
| MIO policies (cost/cooldown) | org_policy | `.policies`→designs | — | `mio_policies` | CAPTURED |
| MIO trait-tree GRID (queue, AI value, modifiers) | MIO detail | trait list stored; tree topology static | — | partial | TODO-ENUMERATE (trait modifier text is render/loc) |

### AREA 8 — SPECIAL PROJECTS / FACILITIES / SCIENTISTS  *(GAP — no parser)*
.gui: `simple_project_roster.gui`(V) `facility.gui`(V) `facility_roster.gui`(V) `scientist_roster.gui`(V) `history_roster.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Special project (name, phases, iterations, reward) | project roster | save `special_projects`-class **(unparsed)** | — | — | READABLE-NOT-WIRED (no parser) |
| Facility (health, supply, research-speed, state, dismantle) | facility roster | save **(unparsed)** | — | — | READABLE-NOT-WIRED |
| Scientist (name, specialization skills, traits, injured, cooldown) | scientist roster | partially in `character_manager` (parse_characters touches "scientist") | — | — (not emitted as scientists) | READABLE-NOT-WIRED |
| Prototype rewards / breakthroughs | facility | **(unparsed)** | — | — | READABLE-NOT-WIRED |

### AREA 9 — LOGISTICS / SUPPLY / FUEL / CONVOYS
.gui: `countrylogisticsview.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Per-equipment material rows (efficiency, producing, need/balance/stock) | logisticsview | production+economy | — | partial | CAPTURED (production) / DERIVED (balance) |
| Fuel prioritization (Army/Navy/Air low-med-high) | logisticsview | `.fuel_status` partial | div fuel +0x610 (144/144) | `economy.fuel`,`divisions.*` | CAPTURED (totals) / READABLE-NOT-WIRED (priority split) |
| Supply node / hub buildings + level | logisticsview/state | province_buildings.supply_node→state_detail | rail/hub via buildings_save | `states.*.province_buildings` | CAPTURED |
| Per-province supply NUMBER (map "20/20") | mapicons/logistics | — (runtime) | div-province proxy (~1020/10272) | `supply.by_province`,`provinces.*.supply` | DERIVED-NEEDS-MODEL / **WALL** (exact number, §W2) |
| Railway level | mapicons | rail_way→state_detail | — | `states.*.province_buildings.rail_way` | CAPTURED (railway map mode disabled) |
| Convoys (on-hand) | logistics/topbar | `.convoys`→production | — | `military.convoys` | CAPTURED |
| Convoys raided / sunk / in-transit | logistics | **(unparsed)** | — | — | READABLE-NOT-WIRED |

### AREA 10 — TRADE / MARKET / LEND-LEASE / EXPEDITIONARY
.gui: `countrytradeview.gui`(V) `marketequipmentstockpilewindow.gui`(V) `pricelevelswidgets.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Trade per-resource (balance, produced, imported, exported, convoys) | tradeview | `.resources.*`→economy | — | `economy.resources.*` | CAPTURED |
| Equipment-market contracts (seller/buyer/speed/equip/convoys/delivery %) | market window | `equipment_market.purchase_contract`→**market.json** | — | `diplomacy_world.trade_contracts` field exists | **READABLE-NOT-WIRED** (parsed, but `store.py` never ingests market.json, no diff event) |
| Lend-lease (1.19 = market superset) | tradeview sub | market.json | — | `diplomacy.lend_lease` field exists, unfed | READABLE-NOT-WIRED + `lend_lease_started/ended` events defined-but-unfired |
| Market stockpile + automation | market window | `.equipment_market.market_stockpile`→market | — | `economy.market_stockpile/automation` | READABLE-NOT-WIRED (parsed, unfed) |
| Price level (low/normal/high) | pricelevels | market prices | — | in contracts | READABLE-NOT-WIRED |
| Expeditionary control / volunteers / attaches | diplomacy sub | parse_oob expeditionary control + market | — | `diplomacy.expeditionary/volunteers/attaches` fields exist | READABLE-NOT-WIRED (legacy expeditionary list not parsed; only OOB control) |

### AREA 11 — RECRUIT / DEPLOY / MANPOWER / TRAINING
.gui: `countrydeploymentview.gui`(V) `divisions_summary.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Manpower available + deployed | deployment/topbar | `.manpower.ratio`→country_snapshot | live avail (M*) | `military.manpower.{available,deployed}` | CAPTURED |
| Deploying units (name, equip progress, training level, priority) | deploymentview | reinforcement/deploy queue **(unparsed)** | — | — | READABLE-NOT-WIRED (deployment conveyor not parsed) |
| Special-forces cap / used | deployment | derivable from divisions | — | — | DERIVED-NEEDS-MODEL |
| Division-type tally | divisions_summary | by_template→divisions | div enum (1603/1603) | `military.division_count` | CAPTURED |

### AREA 12 — ARMY DIVISIONS (per-entity shape, ×~1600)
.gui: `countryarmyview.gui`(V) `unitview.gui`(M) `mapicons.gui`(V) — **STORED:** `divisions.json` — **LIVE:** `division_live_reader`

Per-division field SET — DISPLAYED in unitview/armyview/mapicons:

| FIELD | STORED (parse_units) | LIVE (offset) | REPLAY | STATUS |
|---|---|---|---|---|
| div_id / div_type (identity) | ✓ | +0x0C (1603/1603) | `div_id`,`div_type` | CAPTURED |
| owner / controller | ✓ | +0x1C8 / +0x1D0 (1603/1603) | `owner` | CAPTURED |
| location_province / state_id | ✓ | +0x18 (1603/1603) | `location_province`,`state_id` | CAPTURED |
| template_id/type/name/role | ✓ | +0x3A8→+0x0C (1603/1603) | `template_*` | CAPTURED + `division_template_changed` |
| organisation | ✓ | +0x418 (1603/1603 exact) | `organisation` | CAPTURED |
| strength | ✓ | +0x410 (1603/1603 exact) | `strength` | CAPTURED |
| experience | — | +0x420 (1601/1601) | `experience` | CAPTURED (live only) |
| equipment list (archetype, amount) | ✓ | body+0x358 stride-0x10 **mapped, not extracted** | `equipment[]` | CAPTURED (save); READABLE-NOT-WIRED (live) |
| moving / move_target / move_source / progress | — | +0x230, +0x18, +0x1E8→+0xA4 (1120/1120) | `moving`,`move_target_province`,`move_progress` | CAPTURED (live) + `division_moved` |
| in_combat | — | +0x1A4 (61/61) | `in_combat` | CAPTURED (live) |
| supply status (max/current-ratio/gain/fuel) | — | +0x5F0/+0x600/+0x5F8/+0x610 (144-1604/N) | `supply_status` partial | CAPTURED (live proxy) |
| encircled | armyview marker | — | — | `encircled` field exists, unfed | READABLE-NOT-WIRED |
| division name (custom), manpower, max-org, entrenchment, attrition | unitview | div body (manpower/name unread) | unread | — | READABLE-NOT-WIRED |
| veterancy/level, preparedness, dig-in, acclimatization | unitview | derivable from experience | — | partial | DERIVED-NEEDS-MODEL |

### AREA 13 — COMMANDERS / OFFICER CORPS / ACES
.gui: `command_character_view.gui`(V) `countryofficercorpview.gui`(V) `unitleaderwindow.gui`(V) `armytraittreewindow.gui`(V) `navytraittreewindow.gui`(V) `traittreewindow_common.gui`(V) `advisorroleselectionview.gui`(V) `acesview.gui`(V) `navyleaderview.gui`(V)

Per-character field SET (×generals/FM/admirals/advisors) — **STORED:** `characters.json`:

| FIELD | DISPLAYED | STORED (parse_characters) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Country leader (name, ideology, traits, portrait) | politics/diplo | ✓ leader | — | `commanders.leader`,`party_leaders` | CAPTURED |
| Generals/FM/admirals (name, level, xp, attack/def/planning/logistics/maneuver/coord, traits, preferred_tactic, captured, assignment) | unitleader/trait trees | ✓ full set | CUnitLeader not read live | `commanders.generals/field_marshals/admirals` | CAPTURED (save) + `commander_assigned` |
| Advisors (slot, idea_token, PP, traits) | officercorp/advisor | ✓ advisors | — | `commanders.advisors` | CAPTURED |
| Aces (name, kills, abilities, plane type) | acesview | — **(unparsed)** | air-wing ace ptr unread | — | READABLE-NOT-WIRED |
| Officer-corp spirits / doctrine track / captured-generals count | officercorp | partial | — | partial | READABLE-NOT-WIRED |
| Faction-commander mastery / supply divisions | unitleader | — | — | — | READABLE-NOT-WIRED |
| Portraits (asset) | all | — (asset, not state) | — | n/a | WALL-ish → game_assets extraction (asset, not data) |

### AREA 14 — NAVY (fleets → task forces → ships)
.gui: `naviesview.gui`(V) `navyleaderview.gui`(V) `naval_strategic_region_view.gui`(V) `carrierairwingcompositionwindow.gui`(V) `destroyedshipswindow.gui`(V) `naval_losses.gui`(V) `detailedwindows.gui`(V) — **STORED:** `oob.json` — **LIVE:** `naval_air_reader`

| FIELD | DISPLAYED | STORED (parse_oob) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Fleet (id, name, leader/admiral, icon) | naviesview | ✓ fleet[] | +0x0C (64/64 enum) | `order_of_battle.fleets` | CAPTURED (save); enum-only (live) |
| Task force (id, name, location, mission, replenishment, dominance) | naviesview | ✓ task_force[] +extra_keys | +0x0C; loc +0x18 (81/131; rest moved) | in `fleets[].task_forces` | CAPTURED (save) |
| **Per-ship** (id, definition, org, strength, experience, name, equipment) | naviesview/ship-stats | ✓ full per-ship set | ship +0x0C **id only** (1474 enum) | in task_force ships | CAPTURED (save); READABLE-NOT-WIRED (live composition) |
| Ship HP/hull-damage/damaged-part, position in battle-line, pride | ship-stats/navalcombat | save partial (org/str) | — | partial | DERIVED-NEEDS-MODEL / READABLE-NOT-WIRED |
| Naval region control / dominance (per region, per country) | naval_strategic_region | — | region entry +0x20.. (33/33 exact) | `naval_air.region_control` (M, sidecar) | CAPTURED (live, badged) — **schema sidecar, no first-class home** |
| Navy theaters | naviesview | ✓ navy_theaters | — | `order_of_battle.navy_theaters` | CAPTURED |
| Sunk-ships history / naval losses | destroyedships/naval_losses | — (not in save as history) | — | — | WALL (history not persisted) |
| Admiral (skill, attack/def/maneuver/coord, traits) | navyleaderview | ✓ via characters | — | `commanders.admirals` | CAPTURED |

### AREA 15 — AIR (wings · missions · air-sup · airfields · aces)
.gui: `airgroupview.gui`(V) `airwingdetails.gui`(V) `strategicairview.gui`(V) `airwingreorganization.gui`(V) `airselectionview.gui`(V) `acesview.gui`(V) — **STORED:** `oob.json` — **LIVE:** `naval_air_reader`

Per-wing field SET:

| FIELD | DISPLAYED | STORED (parse_oob) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Wing (id, name, count, experience, manpower, priority, mission, region, plane def, air_base, equipment) | airwingdetails | ✓ full per-wing set | id +0x0C; count +0x6C (265/265); owner; region (56/56); effective_planes +0x160 (56/56) | `order_of_battle.air_wings` | CAPTURED (save+live) |
| Air superiority % (per region) | strategicairview | — | effective_planes sum (proxy; **displayed % ≠ sum**, B1) | `naval_air.air_superiority` (M, sidecar) | DERIVED-NEEDS-MODEL (exact %); CAPTURED (proxy, badged) |
| Airbase capacity / range / friendship | airselection | air_base id stored | airbase (CAirBase 65) unmapped | partial | READABLE-NOT-WIRED |
| Aces (name, kills, specialty) | acesview | — | ace ptr unread | — | READABLE-NOT-WIRED |
| Carrier air-wing composition | carrierairwing | wing→carrier link partial | — | partial | READABLE-NOT-WIRED |

### AREA 16 — COMBAT (land + naval + air)
.gui: `landcombat.gui`(V) `navalcombat.gui`(V) `navalcombatresultswindow.gui`(V) `combatlogview.gui`(V) — **STORED:** `combats.json` (land only) — **LIVE:** `combat_enum`

| FIELD | DISPLAYED | STORED (parse_combats) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Land combat (id, location, day, duration, terrain) | landcombat | ✓ | +0x0C, front+0x28→+0xA4 (13/13) | `combats.*` | CAPTURED + `combat_started/ended` |
| Per-side combatants (units, count, losses, atk/def damage, tactic, progress) | landcombat unit rows | ✓ attacker/defender side set | side+0x18→data (13/13 exact) | in `combats.*` | CAPTURED |
| Combat width / terrain / progress / modifiers | landcombat | ✓ (active snapshot) | live | `combat.combat_width/terrain/progress/modifiers` | CAPTURED |
| Tactic (preferred/active/countered, success chance, CP) | landcombat tactic picker | tactic token stored | — | partial | READABLE-NOT-WIRED (tactic id captured, picker detail not) |
| Commander stat rows (initiative, traits in battle) | landcombat | via characters join | — | partial | CAPTURED |
| **Naval combat** (admirals, ship stat rows, screening, battle-line) | navalcombat | — **(not in save)** | detector pinned from live battle (3/3); returns [] when paused | `_live_combats` sidecar | **WALL** (fixture: needs live battle, §W1) |
| **Air combat** (per-region attrition) | strategicair | — (not in save) | **no discrete object** (validated negative) | — | **WALL** (§W1) |
| Naval combat results (survivors, lost ships/aircraft) | navalcombatresults | — (not persisted) | — | — | WALL |
| Combat log (historical losses by equipment/template) | combatlogview | — (not in save) | — | — | WALL (sub-sample + history, §W5) |

### AREA 17 — DIPLOMACY
.gui: `countrydiplomacyview.gui`(V) `waroverview.gui`(V) `worldtensionwindowpopup.gui`(V) `war_escalation_scripted_gui.gui`(V)

| FIELD | DISPLAYED | STORED (parse_diplo) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Wars (a/b, start, casualties, war_score) | waroverview/diplo | ✓ wars[] | — | `diplomacy.wars`,`diplomacy_world.wars` | CAPTURED + `war_declared`,`peace`,`capitulated` |
| War goals (type, target, states, expire) | diplo declare-war | ✓ wargoals[] | — | in `diplomacy` | CAPTURED |
| Factions (id, ideology, members, leader) | diplo/topbar | ✓ factions[] | — | `diplomacy_world.factions` | CAPTURED + `faction_joined/left` |
| Subjects / autonomy | politics/diplo | ✓ subjects[] (puppet) | — | `diplomacy_world.subjects` | CAPTURED + `subject_*`,`autonomy_changed` |
| Guarantees / NAP / military access / embargo | diplo relations grid | ✓ relations.*→diplo | — | `diplomacy.guarantees/non_aggression/embargoes` | CAPTURED |
| Opinion (attitude, opinion_sum, modifiers) | diplo | ✓ relations.opinion | — | `diplomacy.opinion` | CAPTURED |
| Casualties (killed/captured) + war score | waroverview | ✓ a/b_casualties, war_score | — | `military.casualties.*`, war_score in wars | CAPTURED (no dedicated war-overview window) |
| Surrender / capitulation progress | waroverview | partial | — | — | DERIVED-NEEDS-MODEL |
| **World tension** value + contributors | worldtension popup | `world_tension` **(unparsed)** | — | — | READABLE-NOT-WIRED |
| Estimated enemy forces (army/navy/air/industry) | diplo | derived intel | — | — | DERIVED-NEEDS-MODEL → intel ledger |

### AREA 18 — INTEL
.gui: `countryintelligenceagencyview.gui`(V) `countryintelledger.gui`(V) `operationoverview.gui`(V) `operativeleader.gui`(V) `operative.gui`(V=diplomacy dialog, misnamed)

| FIELD | DISPLAYED | STORED (parse_intel) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Agency (name, icon, upgrades, capacity, defense) | agencyview | ✓ agency block | — | `intel.agency` | CAPTURED |
| Operatives (name, skill, xp, traits, nationalities, mission, captured) | operativeleader | ✓ full operative set | — | `intel.operatives`,`recruitable_operatives` | CAPTURED |
| Operations (priority, running/finished, phases, operatives) | operationoverview | ✓ operations | — | `intel.operations` | CAPTURED + `operation_started/completed` |
| Cryptology / decryption (targets, intel_source, strength) | agencyview | ✓ cryptology | — | `intel.decryption` | CAPTURED |
| Intel networks | (map mode disabled) | ✓ networks (count) + intel positional | — | `intel.intel_networks` | CAPTURED (intel-level array index→tag decode deferred) |
| Counter-intel / defense level | agencyview | ✓ partial | — | in agency | CAPTURED |
| Intel ledger (per-category % + estimates) | intelledger | derived | — | — | DERIVED-NEEDS-MODEL |

### AREA 19 — DECISIONS / MISSIONS / NATIONAL SPIRITS
.gui: `countrydecisionview.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Active decisions (name, cost, timer, target, mission progress) | decisionview | in country_snapshot/parse_* | — | `decisions` (active/timed/missions) | CAPTURED (no per-decision diff event — one-shot clicks §W5) |
| National spirits (= ideas) | decision/politics | ideas→economy | — | `politics.ideas_*` | CAPTURED |
| Timed flags / decision cooldowns / mission timers | decisionview | `flags`/timed **(unparsed)** | — | — | READABLE-NOT-WIRED |

### AREA 20 — STATES (per-entity shape, ×~1081)
.gui: `countrystateview.gui`(V) `countryoccupationview.gui`(V) — **STORED:** `state_detail.json` — **LIVE:** `owner_offsets`

| FIELD | DISPLAYED | STORED (parse_state_detail) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| owner / controller | stateview | ✓ | +0xC8 / +0xCC (1081/1081) | `states.*.owner/controller` | CAPTURED + `state_owner/controller_change` |
| is_core_of / is_claimed_by | stateview | ✓ is_core_of | — | `is_core_of`,`is_claimed_by` | CAPTURED |
| buildings (infra/factories/dock/AA/radar/refinery/silo/reactor) | stateview | ✓ buildings{} | level not read live (save-backstop) | `states.*.buildings` | CAPTURED |
| province_buildings (naval_base/supply_node/rail/bunker/landmarks) | stateview province sub | ✓ province_buildings{} | — | `states.*.province_buildings` | CAPTURED |
| victory_points (total + per-province) | stateview | ✓ | — (unread live) | `victory_points_total/by_province` | CAPTURED |
| resources | stateview | ✓ (in economy) | — | `states.*.resources` | CAPTURED |
| manpower_pool {available, locked, total} | stateview | ✓ | manpower anchor (not value-validated) | `states.*.manpower_pool` | CAPTURED (save) |
| category | stateview | ✓ state_category | — | `states.*.category` | CAPTURED |
| occupation (resistance, compliance, *_speed, occupier, op_status) | occupationview | ✓ resistance{} | — (unread live) | `states.*.occupation` | CAPTURED (data); no diff event → `resistance/compliance_changed` defined-but-thin |
| occupation_law (per-state) | occupationview | ✓ →**occupation.json** | — | `states.*.occupation_law` | CAPTURED |
| demilitarized | stateview | ✓ | — | `states.*.demilitarized` | CAPTURED |
| supply (state-level number) | stateview | — (runtime) | proxy | `states.*.supply` | DERIVED-NEEDS-MODEL / WALL |
| dynamic modifiers / building damage / fort levels | stateview/province | **(unparsed)** | — | — | READABLE-NOT-WIRED |

### AREA 21 — PROVINCES (×~10k-13k)
.gui: `countrystateview.gui` province sub-window, `mapicons.gui`(V) — **STORED:** `province_control.json` — **LIVE:** `province_control.py`, `supply_live.py`

| FIELD | DISPLAYED | STORED (parse_control) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| controller | province sub | ✓ (dense) | u64[base+pid*8]→bridge (10270/10272) | `provinces.*.controller`,`map.control_by_province` | CAPTURED + `province_controller_change` |
| state_id (province→state) | derived | via province_to_state.json | — | `provinces.*.state_id` | CAPTURED (D) |
| supply (per-province) | mapicons "20/20" | — (runtime) | div-province proxy (~1020 covered) | `provinces.*.supply`,`supply.by_province` | DERIVED-NEEDS-MODEL / **WALL** (§W2) |
| rail/route/port/naval-base usage | logistics | — | — | fields exist (`rail_usage` etc.), unfed | READABLE-NOT-WIRED |
| terrain / weather / impassability | province sub | static terrain (map_pipeline); weather→Area 22 | CProvince pool (id only read) | partial | READABLE-NOT-WIRED (static substrate) |

### AREA 22 — WEATHER
.gui: `strategicairview.gui` weather state, `countrystateview.gui` province (mud/snow/temp) — **STORED:** — **LIVE:** `weather_reader`

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Precip / storm / mud / frozen / temperature (per strat region) | strategicair/province | — **(not persisted in saves)** | clock solved + phenomenon enum (hypothesis) + climatology; **no live weather array pinned** | — (no schema field) | **WALL** (§W3 — no oracle anchor; climatology = estimate only) |

### AREA 23 — MAP MODES
.gui: `mapmodes_interface.gui`(V) `mapicons.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Map mode list (Default/States/Diplo/Supply/Navy/Air/Resource/Resistance/Compliance/Players/Infra/Population/Ideology/Terrain/Operatives/Raids/Factions) | mapmodes | each backed by its domain | — | `map.*` layers | CAPTURED (modes are views over captured domains; some UI modes disabled: railway/intel/naval/air/weather) |
| On-map counters numeric (supply node 20/20, rail level, VP tier, intel-ledger 6 building counts) | mapicons | state_detail/economy | supply proxy | partial | CAPTURED / DERIVED |

### AREA 24 — PEACE CONFERENCE
.gui: `peaceconferencewindow.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Conference participants, bids, points spent, taker tabs, wargoal/contested | peaceconference | top `peace_conference` (empty in sample save) | — | `intel.peace_conference` field exists | READABLE-NOT-WIRED (active-conference bid/score sub-structure unverified — no live conference in sample) |
| Per-action bid cost (take state/liberate/puppet/etc.) | peaceconference | peace_mapicon cost | — | — | DERIVED-NEEDS-MODEL |

### AREA 25 — NUKES
.gui: topbar nuke count, `countrylogisticsview` nuclear-bomb count

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Nuclear bomb stockpile | topbar/logistics | **(unparsed)** | — | — | READABLE-NOT-WIRED (only `nuclear_reactor` building parsed) |
| Reactors (building level) | stateview | ✓ buildings.nuclear_reactor→state_detail | — | `states.*.buildings.nuclear_reactor`,schema BUILDING_KEYS | CAPTURED |
| Nuke launches / targets | event | (event, not persistent state) | — | — | WALL (sub-sample event §W5) |

### AREA 26 — STATISTICS / LEDGER
.gui: `countryintelledger.gui`(V) `equipmentoverview.gui`(V) `divisions_summary.gui`(V) `garrisonlogview.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Production / casualties / IC over time | intelledger charts | series from snapshots | — | Stats Layer-1 (`replay_ui/series.py`) | CAPTURED (D series) |
| Intel ledger per-category estimates (industry/manpower/convoys/stockpiles) | intelledger | derived from diplo intel | — | partial | DERIVED-NEEDS-MODEL |
| Equipment overview (stockpile by archetype) | equipmentoverview | production stockpile | — | `military.equipment_stockpile` | CAPTURED |

### AREA 27 — SCORE / VICTORY
.gui: `menubar.gui`(V) end-game ladder

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Country score / rank / faction strength ladder | menubar | partial (VP totals, war_score) | — | `vp_by_state`; no aggregate | DERIVED-NEEDS-MODEL (compute the score formula) |
| Victory points total | topbarvp | ✓ VP→state_detail | — | `states.*.victory_points_total` | CAPTURED |

### AREA 28 — PLAYERS
.gui: `ingamelobby.gui`(V) `frontendgamesetupview.gui`(M) `playerlobby.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| is_human + which TAG | lobby/overview | ✓ is_human→country_snapshot; `player_countries` | live is_human (M) | `politics.is_human` | CAPTURED |
| Steam / player real name | lobby | — | — | — | **WALL** (§W4 — no script/data access; humans by TAG only) |

### AREA 29 — BATTLE PLANS / FRONTS / ORDERS / THEATRES
.gui: `battleplantools.gui`(M) `theatreselector.gui`(V) `leadergroups.gui`(V)

| FIELD | DISPLAYED | STORED (parse_oob) | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Theatres / armies / army-groups (hierarchy) | theatreselector | ✓ theatres/armies/army_groups | CArmy/CArmyGroup/CTheatre not read live | `order_of_battle.*` | CAPTURED (save) + `army_formed/disbanded` |
| Orders (type, instance, path, states, operation) | battleplantools | ✓ orders[] (in army) | COrder/CBattlePlan not read live | in `armies[].orders` | CAPTURED (save) |
| Plan cached value / our_power / enemy_power | leadergroups | ✓ (runtime cached, not authoritative) | — | front_lines (D) | CAPTURED (badged) |
| Front lines (rendered) | map | derived | derived | `map.front_lines` (D) | DERIVED-NEEDS-MODEL (front/arrow rendering) |
| Plan execution settings (planning speed/cohesion/proximity) | battleplantools | — (UI setting) | — | — | READABLE-NOT-WIRED |

### AREA 30 — GAME RULES / SCENARIO / MOD METADATA
.gui: `savedgamerules.gui`(V=save picker, NOT a rule panel)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Save version / player | — | ✓ meta | — | `save_version` | CAPTURED |
| Game rules (selected options) / scenario / DLC / mod set | (loc-driven, no GUI) | `game_rules`/scenario **(unparsed)** | — | — | READABLE-NOT-WIRED |

### AREA 31 — EVENTS / TIMELINE / MESSAGE FEED
.gui: `eventwindow.gui`(V) `notification.gui`(V) `msg_filters.gui`(V) `findview.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Event timeline (~40 derived types) + markers | ticker | n/a (derived from diffs) | n/a | `EVENT_TYPES` (~40) via diff_engine | CAPTURED |
| Message feed / notifications content | notification | n/a (runtime UI) | — | — | WALL (renderer-only feed §W5; persistent-state events recovered) |

### AREA 32 — ACHIEVEMENTS / CAREER / AWARDS (account metadata)
.gui: `achievements.gui`(V) `career_profile_view.gui`(V) `awards_view.gui`(V) `social_view.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Achievements / career / awards / medals | those windows | — (account metadata, not match state) | — | — | WALL-ish (out of scope: not in-match game state) |

### AREA 33 — RAILWAY GUNS
.gui: `railway_gun.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Railway gun (name, strength, forts/terrain mods, BASE/COMBAT/MISC, equipment, army attach) | railway_gun | as units in `units.division` (railway-gun type)→divisions | div enum | in `divisions` (if typed) | READABLE-NOT-WIRED (parsed as division if present; dedicated fields not surfaced) |

### AREA 34 — GARRISON LOG / RESISTANCE ATTACKS
.gui: `garrisonlogview.gui`(V)

| FIELD | DISPLAYED | STORED | LIVE | REPLAY | STATUS |
|---|---|---|---|---|---|
| Resistance-attack log (location, date, blocked, manpower/equip lost) | garrisonlogview | — (historical log, not in save state) | — | — | WALL (history not persisted §W5); current resistance/compliance IS captured (Area 20) |

---

## W. THE WALLED SET (cite the limit; never fake)

| # | Wall | Why | Mitigation |
|---|---|---|---|
| **W1** | **Naval / air combat enumeration** | Battles aren't persisted as save history; live `CNavalCombat` instantiates only during a surface battle (`combat_enum` returns [] when paused); air combat has **no discrete object** (continuous per-region attrition — validated negative). | Detector auto-lights on a live battle; naval offsets pinned from a captured fixture (3/3). Region control + air superiority captured continuously. Needs a scripted live-battle capture, not new theory. |
| **W2** | **Exact per-province / state supply NUMBER** | Computed at tooltip-render / solver output; proven NOT stored as a slot or state-indexed array (5-pass negative, 9 encodings). | Validated division-province proxy (`supply.by_province`, ~1020 provinces where troops sit). A confidence-badged supply map is buildable; a STRONG-but-UNVALIDATED CProvince sub-record chain needs ~5-12 more UI readings to confirm. |
| **W3** | **Weather exact values** | Not persisted in saves → no oracle to anchor offsets. Live drift scan is structural, not value-validated; no live weather array pinned. | Climatology table supports an **estimated** weather mode (must be source-badged). Clock + phenomenon enum solved. |
| **W4** | **Steam / player real names** | No script/data access. | Humans identified by TAG + `is_human`/`is_ai=no`. |
| **W5** | **Sub-sample micro-events & renderer-only feeds** | A combat/decision/nuke-launch that begins **and** ends between two samples is unobservable; the message feed + historical logs (combat log, garrison log, sunk-ships, naval results) are runtime UI, not persistent state. | Surface confidence/coverage; persistent-state + log-marker events recovered. Per-decision one-shots not individually attributable. |
| **W6** | **Pure-UI derived display numbers** | Game speed/pause, overall research-speed %, displayed air-sup %, surrender-progress %, score-ladder values exist only in the renderer or as a formula. | Re-derive via a model (see DERIVED-NEEDS-MODEL rows); never read from a slot. |

---

## C. GAP CLUSTERS (everything not CAPTURED, grouped)

**Cluster 1 — DOWNSTREAM WIRING (data already parsed/readable, schema/store/UI doesn't surface it).**
The single biggest cluster and the cheapest to close. The read work is done; only the pipe is missing.
- `market.json` never ingested by `store.py`; no `lend_lease_*` event fires though defined → trade/market/lend-lease/expeditionary/automation/price-level all dark (Area 10).
- `naval_air` + `supply` live sidecars have no first-class schema home (live-folded as sidecars only); naval/air/railway/intel/weather UI modes disabled (Areas 14/15/21/22/23).
- Live-readable-but-unextracted memory: division **equipment** (body+0x358, mapped not read), division name/manpower; **fleet/ship composition** (1474 ships enum, id only); **CProvince** sub-records; **country submanagers** (factories/fuel/resources/convoys live — known to live in a separate logistics manager, not pinned) (Areas 12/14/21).

**Cluster 2 — WHOLE SYSTEMS WITH NO PARSER (stored or reachable, but never given a reader).**
- **Special projects / facilities / scientists** + prototype rewards (Area 8) — entire system unparsed.
- **Nukes stockpile** (Area 25), **world tension + contributors** (Area 17), **tech-sharing groups** (Area 4), **power balance** (Area 2), **game rules / scenario** (Area 30).
- **Script `variables` / flags / timed cooldowns** (Areas 19/30) — relevant to this mod's host-tool object-vars.
- **Deployment conveyor / reinforcement queue / equipment-in-transit** (Area 11), **convoys raided/sunk** (Area 9), **legacy expeditionary list** (Area 10), **aces** (Areas 13/15), **dynamic state modifiers / building damage / fort levels** (Area 20), **exile/collaboration governments** (Area 2).

**Cluster 3 — DERIVED-NEEDS-MODEL (a formula, not a read).**
- Division/equipment **derived combat stats** (BASE/COMBAT grids from regiment grid + equip) (Area 6).
- **Air-superiority %** (region capacity + stat-weighted score, B1) (Area 15).
- **Victory score / rank ladder** (Area 27), **surrender/capitulation %** (Area 17), **research speed %** (Area 4), **SF cap usage** (Area 11), **intel-ledger estimates** (Area 18/26), **global supply number** (Area 9/W2).

**Cluster 4 — TODO-ENUMERATE (finite but high-cardinality static catalogs).**
- Focus-tree / tech-tree / doctrine / MIO-trait **GRID topologies** (Areas 3/4/7) — static from `common/**`, render-context not match state; finite per-element catalogs not expanded here.

**Cluster 5 — WALLS** (§W1–W6) and **account metadata out of scope** (Area 32).

---

## D. PRIORITIZED CLOSE-OUT PLAN (grouped by effort → impact)

This plan replaces reactive discovery. Within each tier, ordered by player-visible impact.

### TIER 1 — WIRING (low effort: code exists, just connect it) — do first
1. **Fold save-derived domains into live snapshots.** `assembler.py` only folds province/divisions/naval_air/supply/combats today → a live-only snapshot has empty national tabs. Wire the 18-parser oracle bundle as the backstop layer. *(Biggest credibility win; closes most of Cluster 1's "dark national tabs".)*
2. **Wire `market.json` → `store.py` + `FIELD_SOURCES` market entry + fire `lend_lease_*` events** → lights up Area 10 entirely (trade/market/lend-lease/expeditionary/automation/price).
3. **Lift `naval_air` + `supply` sidecars to first-class schema fields** and **enable the disabled UI modes** (naval, air-sup, railway, intel) — with source/confidence badges so proxy data never looks authoritative (Areas 14/15/21/23).

### TIER 2 — EXTRACT (medium: read mapped-but-unread memory / add a parser)
4. **Extract live division equipment** (body+0x358, already structurally mapped) + division name/manpower (Area 12).
5. **Extract fleet → task-force → ship live composition** (1474 ships enumerated, id-only today) + airbase/wing org/strength live (Areas 14/15).
6. **Add a parser for special projects / facilities / scientists** (Area 8) and **nukes stockpile** (Area 25) — net-new systems, both in the save.
7. **Parse world tension + contributors, tech-sharing, power balance, game rules/scenario, script variables/flags** (Areas 17/4/2/30/19) — small per-system parsers.
8. **Parse deployment conveyor / reinforcement queue / equipment-in-transit / convoys-raided / aces / dynamic state modifiers / exile-collaboration** (Areas 11/9/13/20/2).

### TIER 3 — MODEL (medium-high: implement a formula)
9. **Division & equipment derived combat-stat model** (regiment grid + equipment → soft/hard atk, def, org, HP, width) (Area 6) — needed for the designer/unit-stats windows to be non-empty.
10. **Air-superiority % model** (B1: region capacity + stat-weighted score) (Area 15).
11. **Victory-score / rank ladder + surrender % + research-speed % + intel-ledger estimates** (Areas 27/17/4/26).
12. **Front-arrow / battle-plan overlay** rendering from parsed orders (Area 29); **division movement arrows + resistance/compliance overlay** (Areas 12/20).

### TIER 4 — FIXTURE / ASSET (gated on a capture or extraction)
13. **Naval/air combat capture** — scripted before/during/after live-battle diff to re-anchor (W1); enable naval combat mode once captured.
14. **CProvince supply sub-record validation** — ~5-12 UI value readings to confirm the STRONG-but-UNVALIDATED chain and possibly demote W2.
15. **Character/leader portrait extraction** (`game_assets/`) — polish (asset, not state), below all truth work.

### NEVER (walls — state the limit, never fake)
Naval/air combat when no battle exists (W1), exact supply number absent a UI anchor (W2), exact
weather values (W3), real player names (W4), sub-sample micro-events + renderer-only feeds (W5),
pure-UI display numbers (W6), account achievements/career/awards (out of scope).

---

## NOTES ON METHOD & COMPLETENESS

- **Provable completeness at the structural level:** the 34 areas are the union of the 130-window
  `.gui` file list (the DISPLAYED universe), the 20-file save output tree (the STORED universe),
  and the memory layout roots (the LIVE universe). Every `.gui` window was assigned to an area;
  windows the original named set omitted were surfaced by a direct file-list audit and added: **MIO
  designer** (Area 7), **special projects/facilities/scientists** (Area 8), **trait trees / unit-leader /
  advisor-role / aces** (Area 13), **global market** (Area 10), **battle-plan tools / theatre selector**
  (Area 29), **`mapicons` numeric counters** (Area 23), **railway guns** (Area 33), **garrison log** (Area 34).
- **The `.gui` files are shells.** Most numeric values are engine-filled at runtime; the element
  `name=` tells you *which field slot exists* — the value must come from save/memory. The
  BASE/COMBAT/MISC stat grids and `destroyedshipswindow`/`acesview` are entirely code-driven →
  they appear here as DERIVED-NEEDS-MODEL or READABLE-NOT-WIRED, never as "displayed = captured".
- **Game-rule state is not in any `.gui`** — `savedgamerules.gui` is only a save picker; rules come
  from scenario/save data (Area 30).
- **Adversarial (Codex) coverage:** the DISPLAYED and LIVE enumerations were challenged by Codex
  (Mode A) earlier in the session before its usage limit; its valid catches are folded in (MIO,
  special projects, scientists, trait trees, theatre selector, country submanagers, equipment-stockpile
  detail, CState occupation/VP, ship/wing composition). The STORED enumeration and the final
  area-list completeness challenge hit the **Codex usage limit** (hard reset ~16:56) and could not
  run — the STORED gap list is instead derived independently from the parsers' own explicit
  "Gap"/"REALITY NOTE" annotations + confirmed-negative greps (`nuke`/`variables`/`game_rule`/
  `scenario`/`world_tension`/`convoys_raided`), so it stands on its own; a returning Codex pass
  would only corroborate it.

---

## Opus completeness audit (2026-06-25)

Independent adversarial pass standing in for the Codex run the author could not complete (Codex
down until ~17:00). Goal: name what the inventory MISSED and reconcile its gap list against code
already shipped. Method: enumerated the literal `interface/*.gui` set (130 vanilla + 25 mod
overrides — both counts confirmed), the **full top-level key tree of a melted save**
(`re_scratch/campaign_melt_1940.txt`, 1.71M lines, 78 top-level keys — the STORED axis the author
flagged as un-adversaried), the per-entity record shapes from the raw save, and the current
`replay/store.py` + `replay/diff_engine.py` + `replay/schema.py` + `live_capture/assembler.py`.
All read-only.

### B. STALE GAP ENTRIES — already DONE, mislabeled as gap (correct these in §B/§C/§D)

The single biggest correction: **Cluster 1 "downstream wiring" is largely CLOSED.** A prior
data-wiring pass shipped it; the inventory's matrix and close-out plan were written against an
older tree. Verified in code:

1. **Area 10 market / lend-lease — STALE.** Inventory says `equipment-market contracts`,
   `market stockpile`, `lend-lease`, `price level` are **READABLE-NOT-WIRED** ("`store.py` never
   ingests market.json, no diff event"). **WRONG.** `store.py:89` registers `market.json` in the
   bundle; `_ingest_market` (`store.py:500`, called line 161) populates per-country
   `diplomacy.lend_lease` (both parties, `direction`/`counterpart`), the world-scoped
   `diplomacy_world.trade_contracts` (`schema.py:219`, source `S`), and
   `economy.market_stockpile`/`market_automation` (`schema.py:90-91`). `diff_engine._diff_market`
   (line 693) **fires `lend_lease_started`/`lend_lease_ended`** (registered `schema.py:625`,
   gated `_both_captured(a,b,"market")`). → Re-grade the market/lend-lease/stockpile/automation
   rows **CAPTURED**. (Price-level and the *legacy* expeditionary list remain genuinely unwired.)

2. **Area 14/15/21/23 sidecars — STALE.** Inventory: `naval_air`+`supply` "have no first-class
   schema home (sidecars only)." **WRONG.** Both are first-class registered domains
   (`schema.py:49-50`, `SNAPSHOT_DOMAINS`), with typed empties (`empty_naval_air`/`empty_supply`,
   `schema.py:708-713`) and source-badged keys `naval_air.region_control`/`air_superiority` (`M`)
   and `supply.by_province` (`M`) at `schema.py:225-227`. `assembler.py:204-215` promotes the
   live naval_air into `snap["naval_air"]` (first-class) while keeping `_live_naval_air` only for
   back-compat; supply likewise promoted (lines 216+). The "schema sidecar, no first-class home"
   badge on the Area-14 region-control row and Tier-1 #3 are stale.

3. **Tier-1 #1 (the headline plan item) — STALE.** Inventory: "`assembler.py` only folds
   province/divisions/naval_air/supply/combats today → a live-only snapshot has empty national
   tabs." **WRONG.** `fold_save_bundle` (`assembler.py:448`) calls
   `store.build_snapshot_from_oracle` and `_backstop_merge`s the **full 18-file oracle** under the
   live channel (politics, research, focus, production, economy, diplomacy/wars/factions, designs,
   OOB, intel, commanders, market, state_detail/occupation) with honest `provenance.save`
   bookkeeping. National tabs are NOT empty in a live-only snapshot anymore. This whole Tier-1 #1
   is done.

4. **Area 20 occupation events — STALE-thin.** Inventory: occupation has "no diff event →
   `resistance/compliance_changed` defined-but-thin." **WRONG.** `diff_engine._diff_occupation`
   (line ~728) actually fires `resistance_changed`, `compliance_changed`, and
   `occupation_law_changed` (registered `schema.py:626-627`, gated per-domain at lines 156-164).
   They emit on band-crossings — that is intentional debounce, not "thin/unfired."

5. **store.py ingests all 18 oracle JSONs** (`country_snapshot, economy, production, tech, focus,
   diplomacy, state_ownership/control/detail, province_control, divisions, combats, characters,
   designs, oob, intel, market, occupation`, + meta/control_meta). The "20 already-parsed output
   files" in the header is fine, but the gap narrative that the pipe is missing is outdated.

**Headline-count impact:** at least ~8-10 field-groups currently counted as READABLE-NOT-WIRED are
actually CAPTURED (the entire Area-10 market cluster ≈5 rows, the 2 naval_air + 1 supply sidecar
rows, the occupation-event row). The READABLE-NOT-WIRED tally of **~38 is inflated by ~8-10**;
CAPTURED should rise from ~118 to **~126-128** and READABLE-NOT-WIRED drop to **~28-30**.

### A1. DISPLAYED axis (.gui) — coverage is essentially complete; one real omission

Diffed all 130 vanilla + 25 mod `.gui` basenames against every `.gui` token cited in the doc.
**51 vanilla files are uncited, but 50 are correctly out of scope** (frontend/menu chrome:
`frontendmainview`, `matchmaking*`, `ingamemenu`, `settings`, `messagesettings`, `music*`,
`credits`, `tutorialscreen`, `load_screen`, `hotjoin`, `nudge`, `ai_trace`, `backend`, `core`,
`chat`, `browser`, `rightclickmenu`, `insigniaselection`, `subscription_message_view`,
`playeraiprefs`, `pops_login`, `popupwindow`; plus ~22 per-country **mod scripted-GUI** campaign
panels — `GER_inner_circle`, `RAJ_famine`, `SIA_*`, `*_propaganda_campaigns`, etc. — which are
bespoke mod content, not generic game data domains). One genuine omission:

- **OMISSION D1 — `abilitylist.gui` (commander/army activatable ABILITIES).** Not assigned to any
  area. These are the cooldown-gated general/field-marshal abilities (e.g. "Force Attack",
  offensive/defensive stances) — a real in-match mechanic with active/cooldown state. Belongs
  under Area 13 (commanders) or Area 16 (combat) as a field-group. Status almost certainly
  READABLE-NOT-WIRED (ability state lives on the unit-leader/army objects). Add it.

(`insrevolutionwindow.gui` is mod-specific INS war-economy UI → fold under the mod scripted-GUI
bucket. `ingamemenu.gui` exposes a `menu_playthrough_stats_button` — vanilla's own aggregate stats
screen — but that's a derived view, not a new domain.)

### A2. STORED axis — the under-adversaried axis. OMISSIONS found (this is where the gaps are)

Cross-referenced all **78 top-level melted-save keys** against the doc. 33 are mentioned; **45
literal tokens are absent.** Most absentees are benign `*_index`/id-counter bookkeeping
(`front_index, theatre_index, theater_group_index, order_index, navy_id, land_combat_id,
railway_gun_index, special_project_index, industry_organisation_index, equipment_variant_index,
country_leader_index, military_deployment_line_index, military_deployment_conveyor_index,
id_counter_store, debug_current_ref_id, multiplayer_random_*, game_unique_*, checksum,
minor_save_version`) — correctly ignorable. But the following are **real data domains the
inventory never names**:

- **OMISSION S1 — `strategic_air` (top-level).** Distinct from per-country air OOB. Holds
  `air_theatre_index` / `air_group_index` — the strategic air-theatre/group layer. Area 15 covers
  wings but not this top-level strategic-air structure. Flag it (likely READABLE-NOT-WIRED).
- **OMISSION S2 — `strategic_operatives` (top-level).** A WORLD-scoped operative pool
  (`subversive_activity_level` with PID-controller state) separate from per-country `intel`.
  Area 18 enumerates the per-country agency; this global block is unmentioned.
- **OMISSION S3 — `supply_system_2` (top-level).** A large per-country logistics domain the doc
  omits entirely (the only supply discussion is the W2 per-province *number* wall). Contains, per
  country: `truck` records (`count`, `damage`), `buffer`, `wanted_supply_trucks`, `priority`,
  `motorization_level`, `disrupted_supply`, `lost_railways`, `lost_trains`,
  `lost_trucks_attrition`, `lost_trucks_killed`, `daily_losses_index`, and `node`/`province`/
  `supply` sub-tokens. This is STORED and parseable. The W2 wall is about the rendered "20/20"
  tooltip number ONLY — it should be narrowed to say the truck/logistics layer IS available.
- **OMISSION S4 — `sunk_convoys_history` (top-level).** A persisted per-month convoy-loss ledger:
  `{month, convoys, killer_country, owner}`. This **partially refutes W5/Area 14's "history not
  persisted"** claim and Area 9's "convoys raided/sunk = unparsed": the *convoy* sinking history
  IS in the save. (Warship sunk-ship history is NOT — no destroyed-ship key exists — so the Area
  14 ship-loss wall holds; only the convoy framing is wrong.)
- **OMISSION S5 — `tech_sharing_group` (top-level).** The inventory's Area 4 lists tech-sharing as
  READABLE-NOT-WIRED but cites the wrong path (`technology_sharing`); the actual top-level key is
  `tech_sharing_group` (`{id, bonuses, countries}`). Correct the path.
- **OMISSION S6 — `faction_system` (top-level).** Per-country faction influence
  (`influence_status`, `index`, `data`) — the faction-leader influence/pull mechanic, distinct
  from the `faction` membership list (Area 17). Unmentioned.
- **OMISSION S7 — `raids` (top-level).** Naval/air raid TARGET tracking
  (`targets{next_state, existing_target_num, detectable_target_num, next_target}`). The `raids`
  map-mode is listed (Area 23) but the underlying stored raid-target domain is not.
- **OMISSION S8 — `pending_events` + `fired_event_names` (top-level).** `fired_event_names` is a
  persisted list of every event id fired this game (`AUS_political_events.11`, `france.10`, …) and
  `pending_events` the queue. Area 31 calls the event feed a renderer-only WALL (W5) — but
  `fired_event_names` is a genuine persisted event-history ledger. This **partially refutes W5**
  for *scripted events* (not the message feed): you can recover which events fired, just not the
  exact intra-sample timing. Worth a status correction.
- **OMISSION S9 — `all_playthrough_data` + `average_major_ic` + `threat` (top-level).**
  `average_major_ic` (a single scalar, 229.57 in sample) and `threat` (`{threat, final_threat,
  daily}`) feed AI/world-tension scaling; `tension_scaling_base_country="SOV"` likewise. Area 17's
  world-tension is marked unparsed — note that the *scaling inputs* (`threat`, `average_major_ic`,
  `tension_scaling_base_country`) are right here at top level even if the `world_tension` value
  proper is elsewhere.
- **OMISSION S10 — `gameplaysettings` / `difficulty` / `difficulty_settings` / `dlcs` / `mods` /
  `session` / `start_date` / `tutorial` (top-level meta).** Area 30 (game rules/scenario) says
  "unparsed" — confirmed `game_rules={ }` is empty in this sample — but the adjacent SCENARIO
  metadata that IS populated (`gameplaysettings={difficulty, historical}`, `start_date=1936.1.1.12`,
  `dlcs=257535`, `mods={"AMSB - DEV"}`, `difficulty="normal"`, `tutorial=no`) is trivially
  readable and unmentioned. Area 30 should split "game_rules (empty here)" from "scenario/meta
  (readable, unwired)."

### A3. Per-ENTITY field-sets — the inventory UNDER-enumerates the per-record shapes

The matrix glosses several entity bodies as "div body (manpower/name unread)" / "TODO-ENUMERATE."
Pulled the **actual raw child-key sets** from the save:

- **Per-COUNTRY record = 76 direct fields** (the master shape, never listed): `instances_counter,
  program_status, technology, equipment_market, resources, production, flags, command_power,
  pride_of_the_fleet(+date_lost), dynamic_modifier, naval_headquarter_status, research_slot,
  heroes_dying_war_support_penalty, delayed_event, buildings, intelligence_agency, characters,
  division_template_id, division_names_tracker, ship_names_tracker, operative_codenames_tracker,
  railway_gun_names_tracker, units, cached_navy_strength, navy_theater, theatres, variables,
  scripted_gui_random, fuel_status, experience_status, reinforcement, upgrades, deployment,
  capital, original_capital, politics, diplomacy, stability, war_support, nukes, convoys, ace, ai,
  manpower, major, is_top_ic_country, cores, claims, num_armies_in_combat, num_ships,
  convoys_destroyed, focus_tree, continuous_focus_palette, focus, refresh, external_rules,
  original_research_slots, strategic_navy, logistics, accidents_score, cosmetic_tag,
  decision_status, radar, occupation_status, collaboration, intel, country_reports, history,
  incoming_diplomatic_action, operations, tokens, preferred_tactic, name_group, …`. Several
  per-country fields the doc marks "(unparsed)" or DERIVED are confirmed STORED and trivially
  reachable: **`nukes`** (143 countries, `{amount, nukes_ready,…}` — Area 25 stockpile IS in the
  save, not just the reactor building), **`dynamic_modifier`** (103 countries, named modifiers —
  Area 20 "dynamic modifiers unparsed"), **`ace`** (38 countries — Areas 13/15 "aces unparsed"),
  **`reinforcement`/`deployment`** (143 each — Area 11 "deployment conveyor not parsed"),
  **`pride_of_the_fleet`** (14 — Area 14), **`convoys_destroyed`/`accidents_score`/
  `experience_status`/`country_reports`/`cosmetic_tag`** (unmentioned). These are reachable reads,
  not models or walls — they belong in Cluster 2 (no-parser) with "field located," not vaguely.
- **Per-DIVISION record = 33 fields** (doc says "div body (manpower/name unread)"): `id, previous,
  last_combat_date, movement_progress, move_priority, location, logical_country, motorization_level,
  officer, held_officer, division_template_id, division_name, max_supply, organisation, strength,
  equipment, script_id, army_manpower, experience, fuel, requests, dig_in, dig_in_cap, bonus,
  execute_order, acclimatization, army_current_supply_ratio, supply_gain, army_history, killed,
  killer_definition`. → **Area 12 mislabels several as DERIVED** that are actually STORED:
  `dig_in`/`dig_in_cap`, `acclimatization`, `army_manpower`, `experience`, `max_supply`,
  `army_current_supply_ratio`, `supply_gain` are literal save fields, not "derivable from
  experience." `army_history`, `killed`, `killer_definition`, `last_combat_date`,
  `motorization_level` are net-new unmentioned division fields. Re-grade these rows
  CAPTURED-in-save / READABLE-NOT-WIRED, NOT DERIVED-NEEDS-MODEL.

(Per-ship and per-wing bodies in the save are reached through `id={id,type}` reference wrappers
into the global `unit`/`strategic_air` pools — confirming the inventory's "enum-only live, full
set in oob.json save" framing for those is structurally accurate.)

### A4. Walls & captured — mislabels

- **W4 (player real names) — PARTIALLY CRACKED, demote.** The save carries
  `player_countries.<TAG>.user="<name>"` (sample: `GER.user="WooooWoooo"`) — the player's lobby /
  Steam-persona display name, one per human TAG. The parser already reads `player_countries`
  (meta.json) but **drops the `user` subfield.** W4 should be split: the in-game **display name IS
  available** (READABLE-NOT-WIRED, trivial to surface); only the true Steam ID / OS account name
  remains a wall. Area 28 "humans by TAG only" is stale — TAG **and** chosen name are both there.
- **W5 (sub-sample / renderer-only) — NARROW it.** Two persisted history ledgers refute the blanket
  "history not persisted": `fired_event_names` (every fired scripted-event id) and
  `sunk_convoys_history` (per-month convoy losses). The wall holds only for the live *message feed*
  and for combat/garrison/sunk-warship logs (those genuinely aren't in the save). Reword W5 to
  exclude the two persisted ledgers.
- **W2 (supply number) — still a valid wall for the rendered number,** but must acknowledge
  `supply_system_2` (S3): the truck/node/loss logistics layer IS stored. The wall is the *displayed
  per-province "20/20"*, not "supply data."
- **W1, W3, W6 — confirmed genuine walls.** Naval combat empties when paused (no live battle in
  sample); air combat has no discrete object; weather not persisted (no oracle); pure-UI numbers
  are formulas. No change.

### CORRECTED HEADLINE COUNT (replace the §HEADLINE table's deltas)

- **CAPTURED:** ~118 → **~126-128** (market cluster, naval_air/supply first-class, occupation
  events, several save-backed division fields move in).
- **READABLE-NOT-WIRED:** ~38 → **~30-33** (after removing the ~8-10 stale-wired rows; then ADD the
  new omissions S1-S10 + D1 + the located per-country fields, which roughly offsets — net stays
  near 30 but the *contents* change: drop market/sidecars/occupation, add strategic_air,
  strategic_operatives, supply_system_2, faction_system, raids, fired_event_names/pending_events,
  scenario-meta, abilities, player `user` name).
- **DERIVED-NEEDS-MODEL:** ~17 → **~14** (the division dig_in/acclimatization/manpower/supply-ratio
  rows are STORED, not derived).
- **WALL:** 6 → effectively **4 hard + 2 narrowed** (W4 demoted to partial; W5 narrowed to exclude
  the 2 persisted ledgers). W1/W2(number)/W3/W6 stand.
- **TODO-ENUMERATE:** ~17 unchanged (the static GRID topologies remain finite-but-huge), but the
  per-country (76-field) and per-division (33-field) **entity shapes are now enumerated above** and
  should no longer read "unread/TODO."
- **Game AREAS:** 34 stand, BUT add field-groups for: top-level `strategic_air`,
  `strategic_operatives`, `supply_system_2`, `faction_system`, `raids`, `fired_event_names`, the
  scenario-meta split (Area 30), commander `abilities` (Area 13/16), and the player `user` name
  (Area 28). None forces a new AREA — all slot into existing areas as missing field-groups.

**Bottom line for trustworthiness:** the DISPLAYED axis is sound (1 real omission). The STORED axis
— exactly as the author predicted — was the weak axis: **10 real top-level domains and two full
per-entity shapes were under-named, and one wall (W4) plus part of another (W5) are over-stated.**
Conversely, the gap LIST is *inflated*: the entire market/lend-lease cluster, the naval_air/supply
sidecar promotion, the occupation events, and the save-bundle national-tabs fold are **already
shipped** and were mislabeled as open. Net: fewer things are missing than the doc claims (wiring is
done) but more things are *stored-and-reachable* than the doc enumerates (the save is richer than
the STORED column shows).
