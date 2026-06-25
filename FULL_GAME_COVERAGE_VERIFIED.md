# Full-game coverage — CODE-VERIFIED ledger (Track E)

> Authoritative gap-list for the **"no gaps, feels like the real game 100%"** bar.
> Unlike `FULL_GAME_COVERAGE.md` (aspirational status), every cell below was checked
> against the **actual repo code** on 2026-06-25 (read-only; offline; no live process).
> A claim of PRESENT means a concrete `file:symbol` was found, not that a doc says so.
>
> **Four layers per system:**
> (a) **LIVE** = live memory reader (`memory_reader/**`)
> (b) **SAVE** = save backstop parser (`save_oracle/**`)
> (c) **REPLAY** = schema field (`replay/schema.py`) **+** diff event (`replay/diff_engine.py`)
> (d) **UI** = Layer-2 window/mode (`replay_game/static/**`)
>
> Legend per cell: **P**=present · **PARTIAL**=field/parser exists but a layer downstream
> is unwired/proxy/disabled · **MISSING**=no evidence · **WALL**=blocked by a documented
> hard limit · **IN-FLIGHT**=a running track is actively building it.
>
> Codex (Mode-A, ephemeral) was used to challenge this ledger; its blind warnings about
> "navy/air/occupation/diplomacy under-covered" were **refuted by the actual parser code**
> (see §C). Its valid catches (market/lend-lease unwired, war-overview surfacing, nukes,
> static terrain substrate) are folded into the gaps below.

---

## A. Per-system verified table

| # | System | (a) LIVE memory | (b) SAVE parser | (c) REPLAY schema+diff | (d) UI window | Verdict |
|---|---|---|---|---|---|---|
| 1 | **Politics scalars** (stab/ws/PP/CP) | P `live_layers/*` (M); **not yet folded by assembler** | P `country_snapshot.json` (stability/war_support/political_power/command_power live in sample) | P `schema.py:98-101` + diff `government_changed` | P `national.js` politics tab + `topbar.js` | **P** (live-fold pending = IN-FLIGHT) |
| 2 | **Ruling party / popularities / govt** | — | P `parse_economy`/`country_snapshot` ruling_party | P `schema.py:102-103` + `_diff_government` | P politics tab | **P** |
| 3 | **Laws** (econ/trade/conscript) | — | P `parse_economy.py:71` laws tokens | P `schema.py:104-106` + `_diff_laws_ideas` | P politics tab | **P** |
| 4 | **Ideas / spirits / cabinet / advisors** | — | P `parse_economy` ideas + `parse_characters` advisors | P `schema.py:107-109,149` + `idea_gained/lost` | P politics + commanders tabs | **P** |
| 5 | **Research / tech** | P `in_progress` live progress (M*) | P `tech.json` (`parse_tech`) | P `schema.py:116-121` + `tech_researched/lost`, `research_slot_gained` | P research tab | **P** |
| 6 | **National focus tree** | P `current/progress` (M*) | P `focus.json` (`parse_focus`) | P `schema.py:123-129` + `focus_completed` | P focus tab | **P** |
| 7 | **Decisions / missions** | — | P in `country_snapshot`/`parse_*` | P `schema.py:131` (`decisions`) | P decisions tab | **P** (no per-decision diff event — see WALL §B5 one-shot clicks) |
| 8 | **Production lines + gross IC** | — | P `production.json` (`parse_production`) | P `schema.py:84` + `production_milestone`, `_gross_by_archetype` | P production tab | **P** |
| 9 | **Construction queue** | — | P `country_snapshot` construction; state buildings `state_detail` | P `schema.py:76` + `factory_milestone` | P production/buildings mode | **P** (per-building construction *progress*/repair not separately surfaced — minor PARTIAL) |
| 10 | **Factories civ/mil/dock (+offmap/UC)** | — | P `country_snapshot` factories{civ,mil,dockyard} | P `schema.py:72-75` | P topbar + buildings mode | **P** |
| 11 | **Resources produced/exported/avail** | — | P `economy.json` (`parse_economy`) | P `schema.py:77-81` | P resources map mode + logistics | **P** |
| 12 | **Fuel (current/max)** | P live (M*) | P `country_snapshot` fuel/max_fuel | P `schema.py:82-83` | P topbar (`Fuel` panel) + logistics | **P** |
| 13 | **Trade deals / routes / lend-lease / market** | — | **P parser exists** `parse_market.py` → `market.json` (purchase_contract, lend-lease, expeditionary control) | **PARTIAL** schema has `Diplomacy.lend_lease/volunteers/expeditionary` (`schema.py:380-383`) **but** no `market`/`trade` entry in `FIELD_SOURCES`, **`store.py` does NOT ingest `market.json`**, no diff event | **MISSING** (no trade map mode) | **PARTIAL → READY-TO-BUILD** |
| 14 | **Equipment stockpile / convoys** | — | P `country_snapshot`/`parse_units` | P `schema.py:91-92` | P military/logistics tabs | **P** (convoys-raided / equipment-in-transit not separately surfaced — minor) |
| 15 | **Manpower (avail/deployed + law)** | P avail (M*) | P `country_snapshot` manpower_ratio | P `schema.py:88-89` | P recruit/military tab | **P** |
| 16 | **Army/navy/air XP, num_ships, ships-by-hull** | P num_ships (M) | P `country_snapshot` num_ships; `parse_units` | P `schema.py:86,90,93` | P military tab | **P** |
| 17 | **Casualties (killed/captured) + war score** | — | P `parse_diplo` per-war casualties + `war_score_*` (`parse_diplo.py:11,113`) | P `schema.py:94-95`; war score in `diplomacy.wars` | PARTIAL military/diplomacy tab (no dedicated **war-overview/surrender/score** window) | **PARTIAL** (data P, surfacing thin) |
| 18 | **Division templates / equip / ship / plane variants / MIOs** | — | P `designs.json` (`parse_designs`) | P `schema.py:156-157` (`designs.*`) + `design_created`, `mio_leveled` | P designs (designers) tab + `gamewindows.js` | **P** |
| 19 | **Wars / war goals** | — | P `diplomacy.json` (`parse_diplo`) | P `schema.py:132` + `war_declared`, `peace`, `capitulated` | P diplomacy tab | **P** |
| 20 | **Factions / subjects / autonomy / guarantees / NAP / opinion** | — | P `parse_diplo` (guarantee/non_aggression_pact/military_access/embargo/opinion `parse_diplo.py:189-195`) | P `schema.py:114,133` + `faction_joined/left`, `subject_*`, `autonomy_changed` | P diplomacy tab + diplomatic map mode | **P** |
| 21 | **Lend-lease / volunteers / attaches / expeditionary** | — | P `parse_market` + `parse_oob` (expeditionary control) | **PARTIAL** schema fields exist, ingestion unwired (same as #13) | MISSING | **PARTIAL → READY-TO-BUILD** |
| 22 | **Intel agency / operatives / operations / decryption / peace conf** | — | P `intel.json` (`parse_intel`) | P `schema.py:135-141` + `operation_started/completed` | P intel tab | **P** |
| 23 | **Commanders** (leader/generals/FM/admirals/advisors/traits) | — | P `characters.json` (`parse_characters`) | P `schema.py:143-150` + `commander_assigned` | P commanders tab + `gamewindows.js` leaderCell | **P** (portraits MISSING — see #34) |
| 24 | **Order of battle: armies/groups/corps→divisions** | P live division enum (M) | P `oob.json` (`parse_oob` army_groups/armies) | P `schema.py:159-167` + `army_formed/disbanded` | P oob tab + `gamewindows.js` | **P** |
| 25 | **Navy: fleets→task forces→ships** | PARTIAL (region control only, not per-ship live) | **P** `parse_oob.py:15` `fleet[]→task_force[]→ship[]` + admiral leader + `navy_theaters` | P `schema.py:161-162` (`fleets`,`navy_theaters`) | P oob tab | **P** (live per-ship positions not read — WALL-ish, see §B) |
| 26 | **Air: wings / missions / airfields** | P air superiority + wings (M) `naval_air_reader` | **P** `parse_oob.py:21` air `wings` (count, plane def, mission+region) | P `schema.py:163` (`air_wings`) | P oob tab; air mode disabled | **P** (airfield capacity/range not surfaced — minor) |
| 27 | **Battle plans / fronts / theaters / orders** | PARTIAL front_lines derived (`map.front_lines`) | PARTIAL `oob.json` theatres; `parse_oob.py:38` plan_value/our_power/enemy_power | PARTIAL `schema.py:209` front_lines (D) | P frontlines map mode (front arrows/orders MISSING) | **PARTIAL → READY-TO-BUILD** (persisted plans parseable; arrows unbuilt) |
| 28 | **State owner / controller / cores / claims** | P owner+controller+cores (M/S) | P `state_ownership/control.json` + `parse_state_detail` is_core_of | P `schema.py:169-171` + `state_owner_change`, `state_controller_change` | P political + controller modes | **P** |
| 29 | **State buildings / VP / resources / manpower / category** | P buildings (M*) | P `state_detail.json` (`parse_state_detail`) | P `schema.py:172-178` | P state panel + vp/resources/manpower/buildings modes | **P** |
| 30 | **Occupation / resistance / compliance** | — | **P** `parse_state_detail.py:19,100` resistance/compliance/resistance_speed/occupied_country_tag/operational_status | PARTIAL `schema.py:179` (`occupation`) — resistance sub-fields present in JSON but no diff event | PARTIAL (controller mode shows occupation; no resistance/compliance overlay) | **PARTIAL** (data P; UI/diff thin) |
| 31 | **Province controller** | P (M) | P `province_control.json` (`parse_control`) | P `schema.py:183` + `province_controller_change` | P controller mode | **P** |
| 32 | **Supply (per-province) / railway** | PARTIAL `supply_live.py` proxy where troops are; rail `buildings_save.py` | PARTIAL `state_detail` supply_node/rail_way buildings | PARTIAL `schema.py:181,185` supply (S/M*) | PARTIAL supply mode present; **railway mode disabled** | **PARTIAL / WALL** (exact per-province supply NUMBER = UI-anchoring wall §B2) |
| 33 | **Intel level on others** | PARTIAL `logistics_intel/intel_save.py` | P intel networks in `intel.json` | P `schema.py:141` intel_networks | **intel map mode DISABLED** | **PARTIAL → READY-TO-BUILD** (enable mode) |
| 34 | **Character portraits** | — | — | n/a (asset, not state) | MISSING `ui.js:127-131` shows placeholder initials, "portrait asset not extracted" | **MISSING → READY-TO-BUILD** (`game_assets/` extraction) |
| 35 | **Divisions: enum/owner/loc/template/org/strength/equip** | P `division_live_reader` (org/strength/equip/moving/in_combat/experience, validated) | P `divisions.json` (`parse_units`) | P `schema.py:187-201` + `division_created/destroyed/moved/template_changed` | P unit counters + division panel | **P** |
| 36 | **Division live-state: movement arrows / encircled / supply** | P moving/in_combat/supply_status (M*) | — | PARTIAL `schema.py:200-201` moving/in_combat; arrows derived | PARTIAL division panel; **map movement arrows MISSING** | **PARTIAL → READY-TO-BUILD** (arrows) |
| 37 | **Land combat: enum/loc/attackers/defenders** | P `combat/combat_enum.py` 13/13 validated | P `combats.json` (`parse_combats`) active battles | P `schema.py:203` (`combats.*`) + `combat_started/ended` | P combat panel + combat bubbles | **P** |
| 38 | **Land combat width/terrain/progress/casualties** | P (M*) | PARTIAL (active snapshot only) | P combats fields | P combat panel | **P** (finished-between-samples micro-battles = WALL §B5) |
| 39 | **Naval & air COMBAT** | **WALL** `naval_air_reader` validated **DETECTOR**, returns [] until a live battle exists | — (not in save as history) | PARTIAL `_live_combats` sidecar in assembler | naval mode DISABLED | **WALL (fixture)** §B1 |
| 40 | **Naval region control / air superiority / fleet+wing positions** | P region control 265/265, air sup 56/56 (validated) `naval_air_reader` | — | PARTIAL `_live_naval_air` sidecar (`assembler.py:156`) — **no first-class schema home** | naval + air modes DISABLED | **PARTIAL → READY-TO-BUILD** (schema home + enable modes) |
| 41 | **Weather (precip/storm/mud/frozen)** | **WALL** `weather_reader.py` — no save anchor; climatology fallback + live drift-gate scan, not value-validated | — (not persisted) | MISSING (no schema field) | weather mode DISABLED | **WALL** §B3 (climatology UI mode is buildable as *estimate*) |
| 42 | **Terrain (static) / rivers / adjacency / strat regions** | — | PARTIAL `map_pipeline` static + `derive/adjacency.py` | PARTIAL `schema.py:209` (map static) | PARTIAL terrain not its own mode; adjacency in derive | **PARTIAL → READY-TO-BUILD** (static substrate import) |
| 43 | **Players (is_human) + which TAG** | P is_human (M) | P `country_snapshot` is_human | P `schema.py:110` (is_human S/M) | P lobby/overview | **P** |
| 44 | **Game rules / scenario / mod** | — | PARTIAL `meta.json` save_version/player | MISSING (no scenario/rules field) | MISSING | **MISSING → READY-TO-BUILD** (low effort, low impact) |
| 45 | **Event timeline (40 types) + markers** | n/a (derived) | n/a | P `EVENT_TYPES` (~40, `schema.py:544-570`) via `diff_engine` | P `ticker.js` event ticker | **P** |
| 46 | **Statistics ledger (production/casualties/IC over time)** | n/a | P (series from snapshots) | P (D series) | P Stats Layer-1 (`replay_ui/series.py`, `app.js` Layer toggle) | **P** |
| 47 | **Score / victory conditions** | — | PARTIAL VP totals in `state_detail`; war_score in diplomacy | PARTIAL `schema.py:174` vp; no aggregate victory/score | MISSING (no score/victory window) | **PARTIAL → READY-TO-BUILD** |
| 48 | **Nukes (stockpile / reactors / launches)** | — | PARTIAL `nuclear_reactor` building only (`schema.py:55`); no stockpile/launch | MISSING | MISSING | **MISSING → READY-TO-BUILD** (low impact) |

---

## B. The WALLED set (blocked, not a backlog item — state the limit, never fake)

| # | Wall | Why | Mitigation in code |
|---|---|---|---|
| **B1** | **Naval / air combat enumeration** | Battles aren't persisted as history and the live CCombatSide structures only instantiate when a real naval/air battle is in progress. `naval_air_reader` is a **validated detector** returning [] in a paused/no-battle save. | Detector auto-lights when a live battle occurs. Codex reframes this as a **fixture wall** (needs a scripted live-battle capture, not new theory) — true. Region control + air superiority **are** captured continuously. |
| **B2** | **Exact per-province supply NUMBER** | The displayed supply value is a UI-computed number with no stable in-memory slot found; proxy-match pass 2026-06-25 failed to pin it without a UI value-anchor (`supply_live.py:9,15`). | Validated division-province proxy gives a correct supply LAYER where troops are; rail/hub from `buildings_save`. A confidence-badged supply map is buildable; the exact tooltip number is the wall. |
| **B3** | **Weather exact values** | Weather is dynamic and **not persisted in saves**, so there is no oracle to anchor offsets (`weather_reader.py:4`). Live drift-gate scan is structural, not value-validated. | Climatology table (`weather_climatology.json`) supports an **estimated** weather map mode (must be source-badged, not authoritative). |
| **B4** | **Steam / player real names** | No script/data access (DESIGN §4 hard-impossible). | Identify humans by TAG + `is_human`/`is_ai=no`. (Steam *display* names only via SteamID+API — out of scope, per Codex.) |
| **B5** | **Sub-sample micro-events** | A combat/skirmish or one-shot decision click that begins **and** ends between two samples is unobservable. | Surface confidence/coverage; persistent-state and log-marker events still recovered. Per-decision one-shots not individually attributable. |

---

## C. Codex challenge outcome (adversarial, refuted vs accepted)

Codex reviewed **blind** (sandbox only exposed the mod tree, not these Python dirs), so it
reasoned from the supplied grep facts. Net:

**REFUTED by actual parser code (Codex over-warned):**
- "Navy under-covered / oob land-only" → **false**: `parse_oob.py:15` parses
  `fleet[]→task_force[]→ship[]` with admiral leaders + `navy_theaters`.
- "Air under-covered" → **false**: `parse_oob.py:21` parses wings (plane def, mission,
  strategic_region, equipment) + `naval_air_reader` air superiority.
- "Diplomacy missing guarantees/NAP/access" → **false**: `parse_diplo.py:189-195`.
- "Occupation/resistance/compliance missing" → **false**: `parse_state_detail.py:19,100`.
- "War score/casualties missing" → **false**: `parse_diplo.py:11,113`.

**ACCEPTED (valid catches, folded into table above):**
- **market/lend-lease/trade unwired** — verified real: `parse_market.py`→`market.json` exists
  and `schema.py:380-383` has fields, but `store.py` never ingests it and no diff event (#13/#21).
- **War-overview/score/surrender** has data but **no dedicated surfacing window** (#17/#47).
- **Nukes** beyond the reactor building are unmodeled (#48).
- **Static map substrate** (terrain/rivers/adjacency/regions) should rank **above portraits**
  for replay truth (#42, backlog re-ordered accordingly).
- "18 populated JSONs ≠ system-complete" — correct caution; per-field PARTIALs above reflect it.

---

## D. PRIORITIZED ready-to-build backlog (highest player-visible impact first)

> Excludes WALLED items (§B) and IN-FLIGHT items (§E). Ordered for the "feels like the real
> game 100%" bar — replay *truth* and empty-tab credibility before polish.

1. **Fold save-derived domains into live snapshots** (economy, politics scalars, research,
   focus, diplomacy, designs, oob, intel, commanders). Today `assembler.py` only folds
   province_control / divisions / naval_air / supply / combats — so a live-only snapshot has
   **empty national tabs**. Wire the oracle bundle as the backstop layer. *Biggest credibility win.*
2. **Wire `market.json` → store.py + schema source-map + diff event** and add a **trade map
   mode** (#13/#21). Trade/lend-lease/volunteers/expeditionary are high-impact MP systems
   currently parsed-but-dark.
3. **Give naval/air-superiority/region-control a first-class schema home** (lift the
   `_live_naval_air` sidecar into `schema.py` + ingestion) and **enable the disabled UI modes**
   (naval, air superiority, railway, intel) now their readers validate — **with source/confidence
   badges** so proxy/estimated data never looks authoritative (#33/#40).
4. **Static map substrate import** — terrain / rivers / adjacency / strategic regions / supply
   hubs as a terrain mode + replay context (#42). Ranked above portraits per Codex.
5. **Battle-plan / front-arrow rendering** — persisted plans + theatres are parseable
   (`parse_oob.py:38`); build the front/arrow overlay for "feels like the game" (#27).
6. **War-overview / score window** — aggregate war_score + casualties + surrender progress +
   victory conditions into a dedicated window (#17/#47). Data already in `diplomacy.json`.
7. **Division movement arrows + encircled/supply overlay on the map** (#36) — live state exists,
   map rendering doesn't.
8. **Resistance/compliance occupation overlay** + diff events (#30) — data present in
   `state_detail`, no map overlay/event.
9. **Character/leader portrait extraction** (`game_assets/`) — `ui.js` already has the
   placeholder hook (#23/#34). Polish, not truth — deliberately below substrate/navy.
10. **Low-impact fills:** game rules/scenario in meta (#44); nuke stockpile/launch (#48);
    convoys-raided / equipment-in-transit surfacing (#14); per-building construction progress (#9).

---

## E. IN-FLIGHT (other tracks actively building — do NOT list as MISSING)

Per `FULL_GAME_COVERAGE.md` orchestration block + DESIGN §3 ("In flight") + the live dirs:
- **live_capture snapshot writer** (`live_capture/assembler.py`, `capture.py`) — folding live
  readers into schema snapshots. Backlog #1 (save-domain fold) is the natural extension.
- **Layer-2 game-authentic UI** (`replay_game/static/**`) — windows + map modes already
  scaffolded; disabled modes (railway/intel/naval/air/weather) await reader wiring (backlog #3).
- **economy→TAG country-index bridge** (`memory_reader/live_layers/country_index_bridge.py`) —
  needed to attach live economy/politics scalars to TAGs (enables part of backlog #1 live-side).
- **weather / clock** (`memory_reader/weather/**`, topbar clock) — climatology mode (within §B3 wall).
- **save backstop (Track D)** — the 18-parser oracle is essentially complete; `parse_market`
  is the one parser not yet wired into `store.py` (backlog #2).
- **asset extraction (#16)** — `game_assets/`; portraits are the follow-on (#34/backlog #9).
- naval/air, division-live, supply/rail/intel, commanders+intel, designs, OOB+plans readers —
  all validated and landing; their **ingestion + UI surfacing** is the remaining work, captured
  as PARTIALs above.

---

### One-line verdict
Save extraction + province/control/divisions/land-combat/region-control/air-superiority are
**genuinely complete and validated**. The real gaps are **downstream wiring** (market→store,
sidecars→schema, disabled UI modes) and **map-overlay rendering** (arrows, fronts, resistance,
trade, terrain) — not missing data. Four true walls (naval/air *combat* fixture, exact supply
number, exact weather, real names) plus sub-sample micro-events. Nothing is "abandoned."
