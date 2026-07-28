# Replay-UI data sources — every panel, and where its facts come from

> **Read `documents/DECISIONS.md` first.** It is the product authority and outranks this
> file. This page states current intent for one question only: *for any country the viewer
> chooses, where does each replay-UI surface get its content?*
>
> Companion to `REPLAY_DATA_REQUIREMENTS.md` (what the replay needs from memory) and
> `REPLAY_SCHEMA.md` (the field-by-field snapshot contract). Neither of those answers the
> per-surface question, which is why this page exists. It is the written form of
> `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` closure-sweep item (2),
> "observer-UI data sufficiency".

## 1. The rule

Seven capture domains produced **POV-local presentation**: what one client's screen had
already resolved. They are out of capture scope.

| domain | what it captured | replay surface it fed |
|---|---|---|
| d72 `resolved_tooltip_instances` | the final post-layout tooltip blob + its rectangle | tooltip (XUI05) |
| d75 `fired_event_popups` | popup materialisation, chosen option, resolved text | event popup (XUI21) |
| d79 `hud_surface_activation` | the host's active-alert slot bank + topbar counters | HUD alerts / feed |
| d82 `character_presentation` | trait-tree visible order, portrait choice, modifier text | character panel (XUI17) |
| d87 `decision_panel` | the ordered decision grid with pixel rects and formatted text | decisions (XUI19) |
| d88 `active_command_abilities` | the ability-list window's rows, tooltips, disabled reasons | abilities (XUI23) |
| d89 `national_focus_presentation` | the focus view's item vector and final state codes | focus tree (XUI10) |

Truly local state — unit selection, hover, camera, which panels are open, scroll and
expansion — is completely out of scope and is not reopened by anything below. That
disposition is already written in `extractor/documents/EXTRACTION_ATLAS.md:1660`-`:1683`
and `MASTER_EXTRACTION_CONTRACT.md:313`-`:316` (C6).

**But the panels must still fill, for any country.** The model is live observer mode: you
pick a country and the game shows you its focus tree, its decisions, its officer corps.
The replay does the same by **deriving each panel from authoritative captured domains at
render time**. Panels are not replayed recordings of one player's screen; they are
recomposed from world state plus the player's own installed content.

Two consequences the rest of this page works out:

- Where a domain observed the *engine's evaluation* of world state (which focus nodes are
  selectable, which decisions are available), removing the observation removes the fact.
  Those are genuine gaps and they name new collector work, not new UI work.
- Where a domain merely observed *presentation of facts we already hold* (trait-tree
  order, tooltip text, alert list), the replay composes it itself and nothing is lost.

## 2. Classification key

| class | meaning |
|---|---|
| **A** | Derivable today from a domain already ported to `extractor/native/wire_format/src/`. |
| **B** | The collector already captures the fact; only the wire-format port is missing. Named per row. |
| **C** | **True gap.** No existing collector surface carries the fact. Each C row names the new collector work. |
| **D** | Intentionally out of scope: player-local state, or localised flavour text the replay recomposes from the player's own files. Do not reopen. |

Ported set (41 domains, `wire_format/src/lib.rs:20`-`:60`): d10 d11 d13 d16 d17 d20 d22
d23 d24 d26 d27 d28 d29 d31 d32 d34 d35 d36 d38 d39 d40 d46 d48 d49 d50 d51 d52 d54 d56
d61 d62 d63 d65 d67 d68 d70 d83 d92 d94 d95 d96. The registry
(`extractor/native/domain_registry.json`) holds 86 domains in total.

---

## 3. The seven retired surfaces, element by element

### 3.1 Tooltips — replaced by replay-side composition

`game_ui/src/tooltip_compositor.rs` already owns the composition laws; the compositor
needs facts, not a captured string.

| element | class | source |
|---|---|---|
| final resolved tooltip text | **D** | Engine-resolved, interpolated free text. `EXTRACTION_ATLAS.md:1669` — "replay tooltips are generated from captured facts and viewer hover". The replay composes from the tokens plus the player's localisation. |
| tooltip rectangle, viewport, clamp, hovered element | **D** | Host-local geometry driven by a cursor the replay does not have. |
| province and state buildings, state resources | **A** | d23 `map_detail_live` (`wire_format/src/d23.rs:1`; fold `fold_map_detail`, `world_fold.rs:22711`). d23 carries state resources/buildings and province buildings — **not** terrain and **not** victory points. |
| province terrain, victory points | **D** (FILE) | Static map data the replay reads from the player's own files (`map/`, `common/terrain`, `history/states`). No domain carries either; searching `wire_format/src/` and `world_fold.rs` for a victory-point field returns nothing. |
| province supply / attrition figures | **B** | d44 `supply_logistics_attrition` (fold `world_fold.rs:23198`); consumer already typed as `ExactProvinceSupplyTooltipView` (`replay_core/src/world.rs:851`). |
| weather figures | **A** | d24 `weather_live`. |
| division stat block | **A** | d20 + d26. |
| state occupation / resistance / compliance | **A** | d52. |

**Nothing new is owed.** A tooltip is a view over facts the recording already carries.

### 3.2 Event popups — the one gap whose source is gamestate, not UI

Chrome is complete and FILE-derived: `game_ui/src/event_chrome.rs:1`-`:22` derives the layout
and the event-definition-to-window resolution from shipped files alone;
`REPLAY_DATA_REQUIREMENTS.md:678`-`:679` measures that at 97.4% of the 6,757 visible
vanilla events resolving to a fully renderable window.

| element | class | source |
|---|---|---|
| window variant, title/desc/option layout, picture sprite | **D** (FILE) | `interface/eventwindow.gui` + `events/*.txt` + loc, `event_chrome.rs:1`-`:22`. |
| which event fired, to which country, at which hour, in what order | **C** | Not in any recording (`event_chrome.rs:23`-`:32`). **But the source is gamestate:** the fired-event ledger is an in-place hash container at `CGameState+0x538` and the pending vector at `CGameState+0x560` (count `+0x56c`, capacity `+0x568`, stride `0x38`; recipient tag at `pending+0x10 -> CEventScope+0x08`) — `extractor/documents/domains/d75_fired_event_popups_live.md`. Work: an ordinary passive read of those two containers, no UI hook, no POV. |
| visible option ordinals (trigger-filtered set and order) | **C** | Exact pointer membership at `CEvent+0x368`/`+0x374` (same dossier). Passive read; needs the fired instance to join to. |
| chosen option ordinal | **C** | Only the option-command producer knows it. This is a *country's* action, not a client's screen, so it stays in scope — but it needs a producer hook, and there is no authenticated unselected-dismiss producer. |
| selected description branch, timeout, resolution reason | **C** | Same producer graph. Rides with the row above. |
| queued/delayed events not yet fired | **B** | d64 `delayed_events` (fold `world_fold.rs:9499`) — retained/executed/failed-trigger/cancelled lifecycle with owner, event, countdown. |
| the resolved title/description/option strings | **D** | Recomposed from the event definition + the player's localisation. Never a captured string. |

### 3.3 HUD alerts and the outliner

| element | class | source |
|---|---|---|
| the active-alert list itself | **D** | Host UI. `EXTRACTION_ATLAS.md:1670` — "OUT-OF-SCOPE AS HOST UI; REPLAY-DERIVED"; `MASTER_EXTRACTION_CONTRACT.md:313`-`:316` C6. The d79 record carries no fire time — `+0x460` is display order, not time (`domains/d79_*.md`) — so it was never a history anyway. The replay derives its own alert set from the facts below. |
| outliner expansion / scroll / filters | **D** | Host-local. |
| outliner *content* (armies, fleets, wings, production, construction) | **A**/**B** | d28 OOB (A), d20 (A), d29 (A), d11 (A), d48 (A); air wings d30 (B). |
| prototype-reward badge | **A** | d65 ports `UNREAD_PROTOTYPE_REWARDS` as schema field 4 (`wire_format/src/d65.rs:117`, `:174`, row offset `+12` at `:332`) and the fold emits it (`world_fold.rs:12592`, `:12609`); replay already repeats the engine's own sum (`replay_core/src/world.rs:12839`-`:12845`). |
| intel badge (available / prepared-or-completed) | **B** | d53 operation rows. The *counts* were engine-evaluated; the underlying operations are captured, so the replay counts them itself under its own stated rule rather than copying a UI integer. |
| market-contract badges (non-delivering / ongoing) | **B** | d43 `market_trade` contract rows (fold `world_fold.rs:22962`). Same re-derivation. |
| decisions-available / decisions-timeout badges | **C** | Depends entirely on §3.5's decision-membership gap. |
| initiative badge | **C** | `TopbarAlerts.initiative` (`replay_core/src/world.rs:1734`) is read from `scalars.alerts.initiative` (`:12780`-`:12782`); no fold has ever written it. Small: a per-country scalar read. |

**Residue that is not presentation and must be re-homed.** d79 had a second job with no POV
in it: it drives the effective-VFS **content dependency closure** for HUD roots —
`game_ui/src/hud_role_index.rs:1`, "In-process effective-VFS dependency-closure emitter for
authenticated HUD roots", and the d79 dossier's "derive exact GUI/GFX/texture/font/
localisation dependency closures through the recording's authenticated effective VFS".
Dropping d79 from capture scope must not drop that. It re-homes onto the authored surface
set plus the effective VFS, which the replay resolves anyway.

### 3.4 Character / officer-corps panel — derived from d47 and d46, with three residues

`game_ui/src/character_ui.rs:1`-`:10` already states the observer-mode pattern: "Exact live
identity/stats come from replay_core's authenticated domain-47 and domain-46 sidecar
views... Commander assignment identity comes from the exact d28 OOB join, and experience is
exposed as an exact fixed-point rational using the effective game-file skill cost rather
than being recaptured as presentation data."

| element | class | source |
|---|---|---|
| character roster, ids, names, tokens | **B** | d47 `characters_leaders_cabinet_live` (fold `world_fold.rs:16695`), `CharacterRecord`. |
| unit-leader level, skills, traits | **B** | d47 `UnitLeaderRecord` / `UnitLeaderTraitRecord` (`lanes/characters_leaders_cabinet_live/src/extractor.rs:104`,`:123`). |
| country leader role, active ruler, advisors/cabinet | **B** | d47 `CountryLeaderRoleRecord`, `ActiveRulerRecord`, `AdvisorRecord` (`:130`,`:138`,`:149`). |
| portrait | **B** | d47 `CharacterPortraitRecord` (`:158`); the fold gates it on static name-closure authority (`world_fold.rs:16738`-`:16748`). Portrait is a country fact, not a screen fact. |
| army assignment | **A** | d28 OOB join. |
| fleet assignment (which admiral commands which fleet) | **C** | d28 attempts a leader on **army rows only** — army-group, fleet and task-force rows all set `leader_status: "none"` (`wire_format/src/d28.rs:203`-`:207`). Naval leaders are fixture-blocked with candidates narrowed but no promoted offset (`lanes/naval_oob_fleets_ships_live/README.md:117`-`:124`). |
| air-wing assignment (which ace flies with which wing) | **B** | Carried by d46's stop-reported `ACE1` trailer, same blocker as the ace roster. |
| ace roster, ace stats | **B** | d46's `ACE1` ace-roster trailer is **stop-reported, not mapped** — `wire_format/src/d46.rs:17`-`:18` (a stop condition, reported rather than forced into a keyed set) and `:268` (no collection carries it). d46's archetype table is ported; the ace roster still needs a wire-format answer. Joins d47. |
| experience bar | **B**+FILE | d47 experience over `common/unit_leader/00_skills.txt` cost (`character_ui.rs:38`). |
| trait-tree node *visible order* | **D** | Presentation. The replay lays the tree out from `interface/armytraittreewindow.gui` / `navytraittreewindow.gui` / `traittreewindow_common.gui`, which it parses anyway. |
| trait-tree node state: **picked** | **B** | Derivable from d47's assigned-trait list (`UnitLeaderTraitRecord`, `lanes/characters_leaders_cabinet_live/src/extractor.rs:123`-`:128`). |
| trait-tree node state: **blocked vs pickable** | **C** | d82's `TraitNodeState` enum (`lanes/character_presentation_live/src/lib.rs:105`-`:107`) was the only source. Distinguishing the two requires the engine's trigger evaluation, exactly like decision and focus availability. |
| **per-trait XP progress** (the trait-node progress numerator) | **C** | d82 carried `progress_raw`, "Exact selected-leader trait-XP numerator for definition kinds 2/3" (`lanes/character_presentation_live/src/lib.rs:118`-`:120`), and the UI consumes it (`game_ui/src/character_ui.rs:797`). d47 carries only the leader's **overall** `experience_raw` and a bare trait-token list (`extractor.rs:103`-`:128`) — no per-trait progress. This is shared leader state, not presentation, and no non-retired collector surface carries it. See G8. |
| ace modifier text, blocked-reason text | **D** | Runtime-formatted flavour text; recomposed from the trait/modifier token plus the player's localisation. |
| the ace's *effective* modifier magnitude, and the **evaluated cause** behind a blocked reason | **C** | Two facts of the same shape as the ability disabled reason above. The token and the authored modifier are FILE, but the effective applied value is engine-computed, and d82's `reason_context_id` (`lanes/character_presentation_live/src/lib.rs:113`, consumed at `game_ui/src/character_ui.rs:801`) pointed at an engine-evaluated cause the authored key cannot reproduce. d67 `country_dynamic_modifiers` is country-scoped and does not cover per-ace modifiers. Both are named in G8's scope. |
| selected leader / selected wing context | **D** | Host-local selection. |

**Verdict, scoped precisely — and retiring d82 is not free here.** Most of what d82 captured
is replaceable: visible order is presentation, portrait choice is a d47 fact, modifier text is
recomposed. But d82 also carried three things that are **shared leader state, not presentation**,
and nothing else carries them: the blocked-vs-pickable trait-node distinction, the per-trait
XP numerator, and the evaluated cause behind a blocked trait (`reason_context_id`,
`lanes/character_presentation_live/src/lib.rs:113`, consumed at
`game_ui/src/character_ui.rs:807`). All three become genuine gaps the moment d82 retires (G8). The surface additionally
has two pre-existing C rows — fleet-leader assignment (G11) and the effective ace-modifier
magnitude. And d47 alone does not finish the job: the ace roster also needs d46's
stop-reported `ACE1` trailer, and `character_ui.rs:3`-`:4` requires both sidecar views.

This is the one surface where the ruling has a real cost, and it should be paid deliberately
rather than discovered later. The fix is small and squarely in scope: read the trait-node
state, the per-trait XP numerator and the evaluated block cause off the unit leader in
gamestate, for every country, instead of off the selected leader's tree widget.

### 3.5 Decisions panel — the largest true gap

Layout and definition→card mapping are complete and FILE-derived
(`game_ui/src/decision_chrome.rs:1`-`:31`), and the authored clauses ship as a static
artifact: `decisions_static_definitions`, "9,525 decisions / 611 categories with
costs/targets/cooldowns/visibility" (`MASTER_EXTRACTION_CONTRACT.md:777`).

| element | class | source |
|---|---|---|
| category tabs, card templates, icon law, loc keys | **D** (FILE) | `interface/countrydecisionview.gui` + `common/decisions/**`, `decision_chrome.rs:1`-`:31`. |
| authored cost / target scope / cooldown / clause presence | **D** (FILE) | `decisions_static_definitions`; typed as `DecisionStaticEntry` (`game_ui/src/decision_replay.rs:33`-`:45`). |
| counts of active timed / targeted / targeted-timed decisions | **A** | d16 `decisions_live` (`world_fold.rs:14107`-`:14110`). |
| active timed decisions: id, `days_raw` (unit unproven), state | **B** | The fold captures the decision identity, raw integer and state (`world_fold.rs:14125`-`:14133`). The integer's bounds are proven, but not that it is the UI countdown or that its unit is days; treat it as `days_raw` and do not populate `remaining_timer_days` from it (`extractor/documents/domains/d16_decisions_live.md:45`). d16 is a **split** port: the `countries` collection is ported and `timed_live` is not. `wire_format/src/d16.rs:11` calls the domain a split deliverable, `:203` notes there is deliberately no collection id for the stopped collection, and the schema test at `:750` asserts `timed_live is stopped, not ported`. |
| decision take/expire/re-enable transitions | **B** | d45 `FrdDecisionEvent` (`replay_core/src/frd_depth.rs:71`-`:77`). d45 carries more than transitions: see the target-list row below. |
| **which decisions are visible and which are available, per country** | **C** | The engine evaluates scripted `visible`/`available`/`allowed` per country per tick. d16 carries counters only, and d45 carries transitions plus the target and cooldown snapshots in the rows below — but no visible/available membership. `domains/d79_*.md` confirms d87 was the only evaluated-membership source. `decision_replay.rs:5`-`:7`: the replay "never evaluates scripted triggers". **Work: read the engine's evaluated per-country decision membership containers off `CCountry` — not off `CCountryDecisionView`.** |
| remaining cooldown per decision | **B** | d45 captures re-enable records at `CCountry+0xFA0 -> CDecisionStatus+0x58` and removal records at status `+0x70`, both `CDecisionCooldown {definition:+0x08, days:+0x10}` (`extractor/documents/domains/d45_focus_research_decisions_depth_live.md:40`-`:41`). Captured, unported. |
| effective (modified) cost per decision | **C** | Rides on the membership read above. Authored cost is FILE; the *effective* number is engine-computed and nothing carries it. |
| targeted-decision target lists | **B** | d45 already captures `targeted_live` and `targeted_timed_live` snapshots and folds them to `_r2_depth_targets` (`world_fold.rs:3839`), carrying decision identity, country-or-numeric target, lifecycle state, ignore flag and days per row (`replay_core/src/frd_depth.rs:86`-`:110`). Captured, unported — there is no `wire_format/src/d45.rs`. |
| resolved card text / formatted timer text | **D** | Recomposed from the definition + loc. |

### 3.6 Active command abilities — a true gap with no traced gamestate root

| element | class | source |
|---|---|---|
| ability definitions: id, target kind, loc keys, normal/active icon, base cost, authored duration/cooldown | **D** (FILE) | `game_ui/src/active_abilities.rs:20`-`:30`. |
| **available / active / activation hour / cooldown remaining / duration remaining** | **C** | d88's carrier "admits only a source that authenticates itself as the pinned a729 `CAbilityListWindow` post-refresh boundary" (`lanes/active_command_abilities_live/src/lib.rs:1`-`:5`); the owner is that window at battle-plan controller `+0x1C8` (`domains/d88_*.md:25`). Searching `extractor/documents/` for `CCommandAbility` / `command_ability` / `CAbility` outside that dossier returns nothing — **no gamestate ability container is traced at all.** Work: RE the ability container hanging off `CUnitLeader` / `CArmy` / `CArmyGroup` and read it passively for every country. |
| effective command-power cost | **C** | Same. Authored `base_cost_fraction` is FILE; the effective value is `0x140FF4C30`'s output, currently reachable only through the window. |
| target kind / target id / affected divisions | **C** | Same read; d20 carries no ability field (`wire_format/src/d20.rs:63`-`:153`). |
| ability tooltip *wording*, and the final localised rendering of a disabled reason | **D** | Runtime-formatted text, recomposed from the authored description key. |
| the **evaluated cause** behind a disabled reason (e.g. "cannot use while retreating") | **C** | Not recoverable from the authored key. The engine evaluates it into an accumulator that the tooltip callback appends (`domains/d88_active_command_abilities_live.md:29`); the hook's own fixture shows a dynamic reason string (`lanes/active_command_abilities_live/src/a729_hook.rs:2791`-`:2792`), and `game_ui/src/active_abilities.rs:3`-`:7` states no disabled reason is reconstructed. Same read as the rest of G5. |
| on-counter ability icon grid | **C** | Same gap as the row above; it is the counter-badge consumer of it. |
| row order in the ability list | **D** | Presentation; the replay orders by the authored definition order it already parses. |

### 3.7 National focus tree

| element | class | source |
|---|---|---|
| tree topology, node icons, authored style families and overlays | **D** (FILE) | `game_ui/src/national_focus.rs:23`-`:53`, `static_focus_defs.rs`; the d89 dossier records all 22 style families and 93 authored overlays already retained as files. |
| active focus tree id | **B** | d12 `focus_live` (`world_fold.rs:13457`-`:13500`). |
| completed focus set, in engine order | **B** | d12 completed token list (`world_fold.rs:13571`-`:13588`), count/list agreement enforced at `:13590`-`:13600`. |
| current focus, current continuous focus, progress, paused | **B** | d12 (`world_fold.rs:13517`-`:13558`). |
| focus completion transitions | **B** | d45 `FrdFocusDepthEvent` (`replay_core/src/frd_depth.rs:24`-`:37`). |
| **per-node available / unavailable state** | **C** | `REPLAY_SCHEMA.md:392` declares `focus.available` source `C` and explicitly forbids prerequisite inference — "static `available={}` presence or prerequisite inference is forbidden". `replay_core/src/world.rs:19310` reads it. **No fold writes it**; the only producer was d89's `CNationalFocusTreeItem+0x540` final state code. Work: read the engine's evaluated selectable-focus set per country from `CCountry`'s focus manager. |
| **engine visible order after conditional/shared/joint topology selection** | **C** | `REPLAY_SCHEMA.md:393`; `replay_core/src/world.rs:19311`. Same absence, same read. |
| highlight, search markers | **D** | Transient presentation leaves (`domains/d89_*.md`). |
| display percent on the current focus | **B** + FILE | Not a lost presentation leaf. d12 carries `progress` — progress **days** on the active national or continuous focus (`world_fold.rs:13544`, `REPLAY_SCHEMA.md:390`) — and the authored focus `cost` is FILE, so the percent is that quotient. d89's transient display-percent leaf was a second view of the same number. |
| selected node, hovered node, canvas scroll | **D** | Host-local. |

---

## 4. The rest of the panel set

Same model, stated compactly. Every row is a panel an observer expects to open for any
country.

| surface | primary sources | class | notes |
|---|---|---|---|
| Topbar scalars (PP, stability, war support, manpower, fuel, factories) | d63, d11, d10, d14 | A + **B** (d14) | d14 `politics_live` (fold `world_fold.rs:13902`) is unported. |
| Country identity, flag, cosmetic tag | d18 | **B** | Unported; drives flag precedence (`UI_LAYOUT_LAW.md` §2). d18 carries the `tag_db_id`-to-label join, cosmetic tag, claims/cores and major status — **not** ideology. |
| Ruling party / ideology | d14 | **B** | `SEAM_CONTRACTS.md` §3 R-IDEOLOGY: consumed from the exact country-politics `ruling_party`. Ideology suffixes the flag name, so the two rows are joined at render time. |
| Political map, borders, occupation stripes | d22 | A | owner≠controller derives stripes replay-side. |
| Unit counters on map | d20, d26, d28 | A | Badge gaps below. |
| Counter badges: entrenchment | d20 | **A** | `DIG_IN` and `DIG_IN_CAP` are ported fields 18 and 19 (`wire_format/src/d20.rs:37`-`:38`, `:118`-`:125`). |
| Counter badges: order kind | d20, d54 | **A** | d20 ports `ORDER_INSTANCE_ID`/`ORDER_LINK` (`d20.rs:148`-`:153`), and the join that turns those into a kind runs against the **battleplans** view, not plain army orders (`game_ui/src/binding_counters.rs:704`, `exact_order_kind(row, battleplans)`). d27 carries `ORDER_TYPE` but not the engine order-instance identity or division membership needed to reach one division; d54 ports both alongside it (`wire_format/src/d54.rs:83`, `:528`). |
| Counter badges: supply warning, training, dig-in frame, invalid-orders | d20 fold | **B** | The fold writes `counter_supply_status` (`world_fold.rs:19681`), `counter_training_visible` (`:19724`), `counter_dig_in_frame` (`:19732`) and `counter_invalid_orders_visible` (`:19746`); none of the four is in the ported d20 schema. Captured, unported. |
| Counter badges: on-counter active-ability icon grid | — | **C** | Rides on G5; no gamestate ability source exists. |
| Battle bubbles | d21 | **B** | d21 fold (`world_fold.rs:19826`) carries id/generation/location/day/duration/participants. |
| Battle-bubble progress, bombardment, air-superiority frame | — | **C** | Absent from `fold_land_combats`; C3 fields still OPEN. |
| Land-combat detail / combat log | d21, d55 | **B** | d55 is an event stream (§9-shaped). |
| Naval combat + members | d49, d50 | A | |
| Naval missions per task force | d56 | **A** | Ported collection 1. |
| Naval supremacy per strategic region (the sea-zone tint) | d56 | **B** | d56 is a split port: the naval-supremacy sidecar is **stop-reported, not ported** — it is an ordered occurrence list with no unique key (`wire_format/src/d56.rs:117`-`:120`), and collection id 2 is reserved carrying no records (`:158`-`:160`). The fold captures it; the wire format has no shape for it yet. This is the input `REPLAY_DATA_REQUIREMENTS.md` item 5 needs for the sea-zone tint. |
| Air region activity, superiority, losses | d31, d38, d61, d42 | A + **B** (d42) | |
| Sunk ships / destroyed-ships window | d86 | **B** | `ship_name` is **also B, not D**: a player-assigned ship name is persistent shared match state every client agrees on (`lanes/sunk_ship_history_live/src/lib.rs:92`), so it is captured-but-unported, not presentation. It is a wire-format text problem, not a scope question. |
| Sunk convoys / convoy raiding | d70, d90 | A + **B** (d90) | |
| Production panel: lines, stockpile, convoy bank, licences | d11, d40, d39, d35 | **A** | Ported collections. |
| Production panel: the templates/equipment catalogue | — | **D** (FILE) | Reaches the recording as the packed static artifact `templates_equipment_catalog` (`MASTER_EXTRACTION_CONTRACT.md:774`), not as a captured domain — d59 is `reserved` and not schedulable. |
| Production panel: the military **deployment queue** | d11 | **B** | d11 is a split port: the `PDQ1` deployment queue is **stop-reported** (`wire_format/src/d11.rs:76`), as are the convoy priority reservations (`:3169`). Captured, not on the wire. Tracked as X4-P1..P6 in `SEAM_CONTRACTS.md`. |
| Construction panel | d48 | A | Includes damage and repair progress. |
| Research panel: slot count, in-progress, live progress, ahead-of-time | d13 | A | |
| **Research panel: which technologies are researched** | — | **C** | `REPLAY_SCHEMA.md:371` marks `technologies[]` `S/D` (save-derived); no fold writes it (`fold_tech`, `world_fold.rs:13618`, emits only tech_count / research_slots / in_progress_live). The tree cannot be coloured for any country. Work: read the completed-technology vector off `CTechnologyStatus` (the same object d13 already sizes for `tech_count`). |
| Research-bonus rows, tech sharing | d69 | **B** | Bonus is the dynamic adjustment only, not the total. |
| Trade panel | d43, d11 | A (production licences, d11) + **B** (d43 market/contracts) | |
| Diplomacy: per-country standing, faction, at-war count, casualties | d17 | **A** | Ported collection. |
| War overview: the war graph (who is at war with whom) | d17, d60 | **B** | d17 is a split port like d16 — its war-graph collection is **stopped, not encoded** (`wire_format/src/d17.rs:1`, `:171`, and the schema test `the_war_graph_collection_is_stopped_not_encoded` at `:742`). d60 `capitulation_war_state` is captured and unported. Relation flags feed map furniture (SEAM 8b). |
| Diplomatic relations, subjects, intel-bearing relations | d53 | **B** | |
| Intelligence agency, operations, networks, cipher | d53 | **B** | The whole surface rides on one domain. |
| Power balance | — | **C** | Nothing carries it. `fold_politics` (d14, `world_fold.rs:13902`) has no power-balance state; `world.power_balances[]` is read by `replay_core/src/world.rs:22913` but no fold writes it; d74 is `reserved`, `schedulable: false`, and its dossier records "no d74 offset, structure layout, or independent acquisition law" (`domains/d74_power_balance_presentation_live.md:8`-`:11`, `:25`-`:29`). The visible pixels belonged to the retired selected-Present UI pass (`SEAM_CONTRACTS.md` A37), so the ruling leaves this surface with no source. |
| Occupation / garrison / resistance | d52, d66 | A + **B** (d66) | Also feeds the resistance/compliance map modes. |
| Battle plans and orders | d27, d54 | A | Path polyline order is a recorded decision, not a stop. |
| Designer surfaces (division, ship, plane, tank) | d40, d39, d35, d46, d78, d85 | A + **B** | Selected designer identity (d85) is **D** where it is a selection. |
| Scripted-GUI national mechanics | d51, d84 | A + **B** (d84) | d51 carries the generic flag/variable map — mod-extensible by construction. |
| Country modifier list (active dynamic modifiers on a country) | d67 | **A** | Ported keyed set. |
| Modifier *removal* events (a modifier observed at zero inside the engine's erase path) | d67 | **B** | Stop-reported: `zero_removals` are §9 happened-at-tick events and `wire_format` has no code for §9 at all, so `fold_payload` refuses a frame carrying one rather than dropping it silently (`wire_format/src/d67.rs:164`-`:170`). Only matters to a panel that wants to show *when* a modifier lapsed. |
| Special projects | d65, d41 | A (program roster) + **B** (project progress) | d65 ports only owner, program id, province and unread rewards (`wire_format/src/d65.rs:109`-`:117`). Project state, progress, target and completion come from d41 `projects_live` through its `PPGF` view — `game_ui/src/special_projects.rs:1`-`:5` admits rows "only from the owner-joined D41 `PPGF` view". d41 is captured and registered but has no `wire_format/src/d41.rs`. |
| Supply, hubs, rail | d44, d62, d81 | A + **B** | d81 is fully numeric and portable; zero coverage on batch-3. |
| Fog of war / spotted units, **for the recorded POV** | d94, d95 | A | |
| Fog of war / spotted units, **for any other country** | — | **C** | d94 and d95 hold exactly one observing country per hour: `pov_visibility_this_hour` is a single `Option` replaced each hour, and a POV absent from the d18 roster is refused (`world_fold.rs:10805`-`:10832`); d95 applies the same rule. The engine only computes visibility for the client's own country, so an arbitrary-country fog view has no source. See G12. |
| Weather, snow/mud | d24 | A | |
| Peace conference: lifecycle, refresh sequence/epoch, conference reference, current turn, completion | d97 | **B** | The fold does carry these (`world_fold.rs:11925`-`:11945`), and the replay already admits the captured inactive observation (`game_ui/src/readonly_design_peace.rs:906`). There is no `wire_format/src/d97.rs`. |
| Peace conference: participants, allocations, results | d97 | **C** | The producer hard-codes all three to `null` (`world_fold.rs:11943`-`:11945`). The semantic table does not exist on the wire; implement the producer before porting it. |
| Raids | d91 | **B** | Fully numeric, portable; zero coverage on batch-3. |
| Army upgrade requests | d93 | **B** | Fully numeric; zero coverage on batch-3. |
| Preferred tactic / combat modifiers | d92, d96 | A | |
| Session metadata, players, DLC, start date, save version | d68 | **A** | Ported. Note d68 is a split port: its three length-prefixed strings `ideology`, `cosmetic_tag` and `version` are parsed but **stop-reported, never mapped** (`wire_format/src/d68.rs:13`-`:18`). That costs nothing here — ideology and cosmetic tag have their own authoritative rows above (d14, d18). |
| Multiplayer session detail, speed/pause transitions | d57 | **B** | `validated_partial_slice`, unported. |

---

## 5. The true gaps, ranked by replay-UI value

Ranked by how much of the observer experience is unavailable without them.

| # | gap | surfaces blocked | what it takes |
|---|---|---|---|
| **G1** | Evaluated per-country **decision membership** — which decisions are *visible* and which *available* — plus each one's **effective (modified) cost**. Neither remaining cooldown nor target lists belong to this gap: d45 captures both. | Decisions panel (whole), decision alert badges, decision map locators | Read the evaluated visible/available containers off `CCountry`, not off `CCountryDecisionView`. Authored clauses already ship as `decisions_static_definitions`; d45 already supplies cooldowns and targets. |
| **G2** | Evaluated per-country **focus availability** + engine visible order | Focus tree node states, the focus alert, focus previews | Read the selectable-focus set and the conditional/shared/joint selection off `CCountry`'s focus manager. Schema slots already exist and forbid inference. |
| **G3** | **Fired-event stream**: which event fired to whom, when, in what order, with which option chosen | Event popups (whole), event history, the narrative spine of a replay | Passive reads of `CGameState+0x538` (fired ledger) and `CGameState+0x560` (pending vector); an option-command producer hook for the chosen ordinal. Chrome is already 97.4% complete. |
| **G4** | **Completed technology set** per country | Research tree colouring, doctrine tree, any tech ledger | Read the completed-technology vector off `CTechnologyStatus` — the same object d13 already reads for `tech_count`. |
| **G5** | **Active command ability state** from gamestate — availability, activation, timers, effective cost, target, **and the evaluated cause behind a disabled reason** | Ability bar, on-counter ability icons | RE the ability container on `CUnitLeader`/`CArmy`/`CArmyGroup`. Nothing is traced today; this is the only gap needing fresh RE from zero. The disabled *cause* is engine-evaluated and rides with it — only the wording is recomposable. |
| **G6** | **Battle-bubble display fields**: combat progress, bombardment_active, air_superiority_state | Close-zoom map fidelity, battle bubbles | Extend `fold_land_combats` (`world_fold.rs:19826`). Already OPEN in `REPLAY_DATA_REQUIREMENTS.md` item 3. The counter badges are *not* part of this gap: entrenchment and order kind are ported (A), and supply/training/dig-in-frame/invalid-orders are captured-but-unported (B). |
| **G7** | **Alert derivation inputs**: `initiative`, and the re-homing of d79's content-dependency closure | Topbar alert strip; content-manifest completeness | `initiative` is a per-country scalar read. The closure re-homes onto the authored surface set + effective VFS. |
| **G8** | **Unit-leader trait-tree node availability, per-trait XP, the evaluated blocked-trait cause, and the effective ace-modifier magnitude** | Officer-corps trait trees for every leader; ace rows and their blocked reasons | Two reads, both on gamestate rather than on the selected leader's tree widget: (a) the trait-node state (blocked vs pickable), the per-trait XP numerator and the evaluated block cause, off the unit leader; (b) the effective applied modifier on an ace — d67 `country_dynamic_modifiers` is country-scoped and does not reach it. Ranked here because these are visible panel elements, the reads are small, and (a) is the one gap the ruling itself creates. |
| **G9** | **Live power-balance state** | Power-balance panel (BBA), power-balance decision grid | Nothing carries it: d14 has no power-balance state, `world.power_balances[]` has no writer, and d74 is reserved with no acquisition law. Needs the semantic read d74 was reserved for. |
| **G10** | **Peace-conference semantics** — participants, allocations, results | Peace conference panel's substance (its lifecycle/turn header is captured, just unported) | The d97 producer hard-codes all three to null (`world_fold.rs:11943`-`:11945`); implement it before any port. |
| **G11** | **Fleet-leader assignment** | Officer corps admiral rows, fleet panels, naval counters | d28 attempts a leader on army rows only. Fixture-blocked on the extractor side with `CNavyLeader` candidates already narrowed — a known unit of work, not an unknown. |
| **G12** | **Fog of war for a country other than the recorded POV** | Observer fog view when the viewer picks a non-recorded country | d94/d95 carry one observing country per hour by construction. The engine computes visibility only for the client's own country, so this is not a porting gap but a capture-model one: either capture per-country visibility (expensive — d94 already re-sends 13,414 bytes per hour for one POV) or decide the observer shows full information outside the recorded POV. Lowest rank because no panel depends on it. |

## 6. Structural-domain priority

The 13 structural/unported domains, ordered by what they unblock. This is the wave order.

| rank | domain | UI surfaces unblocked | non-UI replay needs unblocked |
|---:|---|---|---|
| 1 | **d47** characters_leaders_cabinet | Officer corps, unit-leader window, trait trees, cabinet/advisors, portraits — most of §3.4. The **ace roster additionally needs d46's `ACE1` trailer**; trait-node availability, per-trait XP, the evaluated blocked-trait cause (all G8) and fleet assignment (G11) stay gaps | Commander identity on counters and army panels; leader traits as combat-modifier inputs; d82's and d46's owned dependency |
| 2 | **d53** diplomacy_intel_operatives | Intelligence agency, operations, networks, cipher, intel alert badges, equipment-purchase contract context on the trade panel | Pair-relation flags for map furniture (SEAM 8b); diplomacy panel relation rows |
| 3 | **d21** land_combats_live | Battle bubbles, land-combat panel, combat log locations | The map's combat indicators (M02); attack-direction arrows derived from participants |
| 4 | **d55** land_combat_detail_events | Combat detail/losses panel, per-side equipment-loss tables, terminal-combat results window | Casualty and equipment-loss ledgers |
| 5 | **d64** delayed_events | Queued-event display; the nearest existing substitute for G3 until the fired stream lands | Event-driven state transitions invisible to hourly snapshots |
| 6 | **d81** rail_replay_state | Supply/rail overlays on the map and in the logistics panel | The renderer's `min(level_a, level_b)` rail-level law — a *rendering* input, not a panel input |
| 7 | **d86** sunk_ship_history | Destroyed-ships window, naval-loss ledgers | Naval attribution history; `ship_name` is captured shared state whose port needs the wire format's text answer |
| 8 | **d90** convoy_transport_attack | Convoy-raiding panel, naval-loss detail | Cross-drain continuity with d70's sunk-convoy table |
| 9 | **d69** tech_sharing_groups | Research-bonus rows, tech-sharing group membership | Effective research-speed inputs |
| 10 | **d66** occupation_state_extras | Occupation detail rows beyond d52 | Resistance/compliance map-mode detail |
| 11 | **d93** country_army_upgrade_requests | Upgrade queue on the production/army panels | Equipment-delivery accounting |
| 12 | **d91** raid_instances | Raid panel (`RaidStateView`) | Strategic-raid state |
| 13 | **d97** peace_conference | Peace-conference panel header only — lifecycle, refresh sequence/epoch, conference reference, current turn, completion | **Partly blocked upstream**: the header is captured and portable today, but the producer emits `participants`/`allocations`/`results` as null, so the panel's substance needs the producer fixed first. |

## 7. What this page does not decide

- It does not change any domain's capture status. `extractor/native/domain_registry.json`
  and `MASTER_EXTRACTION_CONTRACT.md` remain the extractor's authority.
- It does not authorise inference. Where a fact is engine-evaluated, deriving it from
  static clauses plus world state is forbidden by the same rule that already forbids it for
  `focus.available` (`REPLAY_SCHEMA.md:392`) and for decision triggers
  (`decision_replay.rs:5`-`:7`).
- It does not reopen player-local state. Selection, hover, camera, open panels, scroll and
  expansion are out, permanently.
