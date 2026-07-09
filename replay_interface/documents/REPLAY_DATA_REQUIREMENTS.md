# Replay data requirements — everything the replay needs from game memory

**Audience: the extraction agent** — the one reading live the game memory and
writing the `.battleplan` recording. This file is SELF-CONTAINED: it is the
complete list of what the replay layer needs inside each recording, with
the concrete shapes inlined. You own *how* to extract; this is the *what*,
*when*, and *why it's visible on screen*. (`REPLAY_SCHEMA.md`, in this same
folder, is the detailed field-by-field contract for the snapshot format —
useful as the companion reference, but everything you need to plan work is
in this file.)

Everything visual that is NOT in this list comes from shipped installed content or
one-time GPU capture on the replay side — you never need to extract it.
The replay layer is fail-closed: anything below that hasn't landed yet
degrades gracefully (stated per item), it never blocks your work.

Priority order: items 1–3 make replays function at all; 4–8 (incl. the
`b`-suffixed inserts) make them look real; 9–12 are polish/deferred.

**Last full review: 2026-07-09**, after the engine-extraction wave (labels /
borders / terrain / UI-chrome / furniture / code-ports lanes). New consumers
landed on the replay side; items 7b, 8b and 9b were added for them.
**Reconciled 2026-07-09 against the extractor's own code + their
`extractor/documents/REPLAY_REQUESTS_RECONCILIATION_2026-07-04.md`:** several
items this file still called "needed" are LANDED extractor-side (statuses
updated per item below, with code citations). Where an item says
"extractor-side LANDED", the remaining action is replay-side: ingest a fresh
production recording and verify the rows — flag mismatches back through
`EXTRACTION_INTERFACE_REQUESTS.md`.

---

## 1. Hourly decoded world snapshots in the container (FOUNDATION)

Originally the production recorder packed only the raw compact domain
stream (`role: collector_compact_stream`, empty `"snapshots"` index), which
nothing outside `recorder_host` could decode. See the status update below --
this landed.

Need: the recorder ALSO emits decoded hourly world snapshots into the
container — `REPLAY_SCHEMA.md`-shaped JSON, one entry per canonical game
hour, `role: "snapshot"`, `hour_index` set, listed in `index.json` —
alongside the compact stream (which stays as raw evidence/audit). The
recorder already decodes every compact record for its verification gates;
folding decoded rows into a per-hour world JSON reuses that work, keeps ONE
decoder implementation (owned by you), and makes the file durable and
self-describing.

Acceptable alternative: expose the compact-stream decoders as a reusable
library crate with a documented record→world fold contract (more coupling,
second choice).

**STATUS UPDATE (reconciled 2026-07-09): extractor-side LANDED.** The
production recorder now folds the compact stream into per-game-hour world
snapshots and packs them as `role: "snapshot"` entries with `hour_index`
(`extractor/native/recorder_host/src/main.rs:2704`; their reconciliation doc
marks R1 DONE). Replay-side action: verify a fresh production recording
ingests through `replay_core` Backend 1 end-to-end. The compact-stream-only
fail-closed error stays for OLD recordings.

## 2. Per-hour ownership & control (THE POLITICAL MAP)

Inside item 1's snapshots: `owner` and `controller` per state and per
province, per hour. This drives the political fill, borders, and the
occupation stripes (replay derives stripes from owner≠controller — no
extra extraction needed). Country colors themselves come from shipped
files; only the tags are needed.

New consumer (2026-07-09, labels lane): the on-map COUNTRY NAME labels key
their text/color off the state's owner or controller (which of the two the
engine uses in wartime is still capture-pending on the replay side) — the
recording must simply carry BOTH per hour, which this item already
specifies. No new shape needed; noting the consumer so this row never gets
thinned to owner-only.

## 3. On-map land units & battles

Inside item 1's snapshots, per hour:
- Divisions (schema §6, live-fed already per the schema's collector
  contract): `div_id` + `div_type`, `owner` tag, `location_province`,
  `template_role` (drives the counter icon), `strength`, `organisation`,
  `moving` + `move_target_province` (replay interpolates movement between
  hourly snapshots from these).
- Active land combats (schema §7, live-fed): `combat_id`,
  `location_province`, attacker/defender division ids. This drives battle
  bubbles; the replay derives attack-direction arrows from participants'
  provinces. Combat `progress` for the bubble bar is wanted but currently
  unvalidated — flag when it becomes trustworthy.

Extension (2026-07-09 review): faithful COUNTER BADGES at close zoom need
more per-division state than the row above: `supply_ratio`/`low_supply`
(supply warning icon), `has_orders`/`current_order_kind` (plan badge),
`dig_in` (entrenchment), `training`, attrition-active, and
`active_ability_kinds` (the on-counter active-ability icon grid). Wanted
hourly/on-change alongside the base division row; each missing field
degrades to badge-absent. Same priority as the close-zoom unit lane (A7).

## 4. Session metadata (playback pacing & player labels)

Speed/pause change events with timestamps, and the player↔country mapping
(who was human, which tag). Makes playback pacing authentic and lets the
UI label human players. Degrades to fixed-speed playback with no player
badges.

Also once per recording (2026-07-09 review): the **active DLC list** —
resolved DISPLAY names (plus descriptor paths if cheap) — and the
**localisation language**. DLC gates cosmetic identities (CAN_UK vs
CAN_ALY etc.; replay consumer `cosmetic_tags_from_resolver_with_dlc` is
LANDED and defaults to all-DLC-present until fed the real set). Long filed
as R-DLCLIST in `EXTRACTION_INTERFACE_REQUESTS.md`; folded here so this
list stays the single handoff.

**STATUS (reconciled 2026-07-09): partial extractor-side.** A live session
header with player/ideology/cosmetic/DLC/start-date/save-version metadata
exists (`world_fold.rs:1867`, domain 68 fold at `:4536`); speed/pause
events remain OPEN per their `MASTER_EXTRACTION_CONTRACT.md:143`.

## 5. Naval supremacy per strategic region (SEA-ZONE BORDERS)

The sea-zone borders/fills players see over ocean in MP are a
naval-supremacy display. Measured on the replay side: with no supremacy
data the game renders NO sea lines, and the sea-zone distance field is
static map data the replay already has. Need: per-hour naval supremacy —
or at minimum a contested/friendly/enemy tri-state — per strategic region
id (ids join against `map/strategicregions`). Degrades to open ocean
(what the game shows with no naval data).

**STATUS (reconciled 2026-07-09): extractor-side LANDED.** Domain 56 naval
supremacy rows fold into `map._r2_naval_supremacy`
(`world_fold.rs:3950`, contract registers domain 56). Replay-side ingest +
join against `map/strategicregions` pending.

## 6. Weather state per strategic region

The map's snow/mud/ice/storm visuals are driven by SIMULATED weather; the
static files ship only probabilities. Rendering laws are fully on the
replay side — it just needs to know which weather actually happened.
Need: per-hour or on-change `{region_id, weather_kind, intensity}` where
weather_kind covers clear/rain/storm/snow/blizzard/sandstorm plus any
mud/frost ground state the sim tracks. Degrades to clear weather
everywhere (visibly wrong for Eastern Front winters, but graceful).

**Refined 2026-07-07 (SNOW_LAW.md): ground snow/mud is ACCUMULATED sim
state, not a weather-kind function.** The engine's `SnowMudData` texture
holds per-area accumulators (`snow_gain_on_snowing=1.0`, blizzard 5.0,
`snow_gain_max=80`, visual floor `snow_visual_min=128`, temperature melt —
constants in common/weather.txt; the tick algorithm is engine-side). So
in addition to weather_kind above, the map snow/mud visual specifically
needs the **accumulated ground state**: per hour (or on-change)
`{region_or_province_id, snow_level_0_255, mud_level_0_255}` plus the
current `SnowMudFade` phase (CB0 offset 248) and its fade-0/fade-1
endpoints if obtainable. The replay rasterizes this into the quarter-res
SnowMudData via the derived addressing law (south-origin Y-flip) and
blends by fade. Files-only degrade: the deterministic per-region
`min_snow_level` date floor (small Arctic/deep-winter band only —
visibly missing Eastern-Front accumulation, but deterministic, mod-aware,
never crashes).

**STATUS (reconciled 2026-07-09):** the per-region weather stream is
extractor-side LANDED (`map._r2_weather_live`, `world_fold.rs:2459`, with
region intensity/phenomenon fields) -- replay-side ingest pending. The
ACCUMULATED snow/mud ground state (the SnowMudData ask above, incl. the
R-SNOWMUD texture-snapshot alternative) remains OPEN.

## 7. Cosmetic-tag stream (MID-GAME COLOR/NAME/FLAG CHANGES)

`cosmetic_tag` exists in the schema (§3.12) but is save-sourced today.
Cosmetic tags change a country's map color, name, and flag mid-game
(Vichy, civil-war splinters, puppet renames) — a replay that keeps
pre-switch colors is visibly wrong to every player. Need: on-change (or
hourly) `{tag, cosmetic_tag}` rows from live memory. The replay resolves
the actual colors/flags/names from shipped files — only the tag stream is
needed. Degrades to base-tag identity.

**STATUS (reconciled 2026-07-09): extractor-side LANDED** (D18;
`countries[*].cosmetic_tag` attached at `world_fold.rs:2127`). Replay-side
ingest pending.

## 7b. Ruling party (ideology) per country (LABEL TEXT + COLOR VARIANTS)

Companion to item 7, same shape and cadence: on-change (or hourly)
`{tag, ruling_party}` rows from live memory (e.g. `"fascism"`). The schema
already carries this (§3.3 `politics.ruling_party`, live-collector
sourced) — the ask is that it rides into item 1's hourly snapshots.

Why (proven 2026-07-09, labels lane LAB4): the engine's on-map country
names are IDEOLOGY-RESOLVED localisation keys — `<TAG>_<ideology>` first
("GERMAN REICH", "BRITISH RAJ"), then `<TAG>`, then the tag. A replay that
keeps the 1936 ruling party renders wrong names after every civil war,
coup, or election. Ideology can also select color variants. Degrades to
game-start ideology from history files (visibly wrong mid-game).

## 8. On-map naval & air presence

Land is covered by item 3; oceans and skies are empty without:
1. Naval (higher priority): per-hour `{task_force_id, owner_tag, location
   (sea province or naval-base province), ship_count,
   dominant_hull_class}` — enough for fleet icons/counters at sea. Full
   composition can stay in raw streams.
2. Air: per-hour `{wing_or_group_id, owner_tag, strategic_region_id or
   airbase province, mission_kind}` — enough for region air-activity
   icons.
Degrades to empty seas/skies.

**STATUS (reconciled 2026-07-09):** AIR is extractor-side LANDED
(`map._r2_air_region_activity` with owner/mission-mask/wing/plane counts,
`world_fold.rs:3920`, D31) -- replay-side ingest pending. NAVAL task-force
rows remain OPEN as specified above.

## 8b. Diplomatic pair-state, faction membership, capitals (MAP FURNITURE + WAR OVERLAYS)

Three small per-country/per-pair streams, all with existing schema shapes
(§3.3, §3.8, §7b) — the ask is that they ride into item 1's snapshots:

1. **Pair relation state**: per hour (or on-change), for each country
   pair with a non-default relation: at-war, same-faction/allied, subject/
   overlord, guarantee, non-aggression pact, military access, embargo.
   The extractor ALREADY live-extracts the relation flag set (the d53
   relation-flag container work: NAP/military-access/docking/guarantee/
   embargo — live-validated); at-war and faction membership come with it.
   Consumer (2026-07-09, furniture lane): the map-furniture victory-point
   icons carry a DIPLOMATIC COLOR COLUMN (relation of each province's
   owner to the viewed country) — without pair state the column can't
   tint. Also feeds war-state overlays (fronts/arrows) later. Degrades to
   neutral tint everywhere.
2. **Faction membership**: `{tag, faction_name_or_none, is_leader}`
   on-change (schema §3.3 `faction.*`). Consumer: the same diplomatic
   column (ally tint) + later diplomacy UI. Rides with (1) if easier.
3. **Capital state per country**: `{tag, capital_state}` on-change
   (schema §3.3 `capital_state`). Consumer: the capital-star map
   furniture must move when a government relocates (France 1940).
   Rare-event cadence; on-change is plenty.

FURN-REL refinement (2026-07-09, decompile-verified): for the VP/capital
icon color column SPECIFICALLY, the engine selector
access/embargo, and does NOT read the d53 relation-flag container. So the
MINIMUM for the icon column is just `at_war` + `same_faction` per
viewed/owner pair (column law: self/same-faction -> col 2, at-war ->
col 0, else neutral col 4; ported as `vp_icon_relation_column`, degrades
to neutral gray). The BROADER pair-state set above stays requested for the
other consumers (war overlays, diplomacy UI).

## 9. Localisation / token joins

The replay UI renders tokens the recording emits (focus ids, idea tokens,
tech names, decision ids). Need: a deterministic token → localised-name
join (effective mod-over-vanilla), either packed in the container or as a
documented static bundle keyed by the recording's mod identity. First
surfaces to hit this: map labels, topbar, country panels.

**STATUS (reconciled 2026-07-09): partial extractor-side** -- their contract
lists a `localisation_join` static artifact
(`extractor/documents/MASTER_EXTRACTION_CONTRACT.md:176`,
`compact_contract/src/lib.rs:204`). Replay-side: confirm the artifact
actually rides in production recordings and covers the label/topbar keys.

## 9b. Topbar / panel scalar bindings (UI CHROME — first rendered set)

The the game UI chrome now composes from shipped `.gui` files on the replay
side (all 453 parse; topbar rects gate ≤1px vs engine draw quads). Every
NUMBER on it is recording-gated. The first fields the UI will actually
render, all already shaped in the schema (§3.1–3.3) — the ask is presence
in item 1's hourly snapshots for every country (the viewer can select any
POV):

- `politics.political_power`, `politics.stability`,
  `politics.war_support`, `politics.command_power`
- `military.manpower.available`, `military.xp.army/.navy/.air`
- `economy.factories.civ/.mil/.dockyard`, `economy.fuel.current/.max`
- `military.convoys[]` (count is enough for the topbar)
- date + speed/pause (items 4 and the clock note below)

Extension (2026-07-09 review, from the game_ui binding structs): the
topbar/HUD consumes MORE than the core scalar set -- per POV country:
`nukes_total/nukes_ready`, `supply_value/supply_ratio`, `world_tension`
(world-scoped), the ratio bars (`industrial/convoy/fuel/xp` ratios), and
the alert/notification counts (initiative, decisions available/timeout,
intel, prototypes, market contracts); plus the OUTLINER rows (army/fleet
groups with icon, text token, progress). Each degrades independently to an
unavailable badge / empty outliner section.

Degrades to an unavailable badge per missing field (never a fabricated
number). Deeper panel content (production lines, army lists — the
listbox/gridbox rows) stays deferred with items 10–12 until those panels
are scheduled.

Non-political MAP MODES (2026-07-09 note): the replay viewer selects the
map mode at playback time, so the ACTIVE mode during original play is NOT
needed (optional session trivia at most). The replay computes mode colors
itself from recorded inputs -- do NOT record evaluated per-scope colors.
The INPUTS for non-political modes (intel, resistance, supply detail)
are deferred with items 10-12 until those modes are scheduled.

## 10. Building levels per state (live feed — when building rendering lands)

Schema §4 `buildings`/`province_buildings` (state-scoped levels) are
save-sourced today. The replay will render buildings on the map from
static placement files; it needs the per-game levels (which slots are
built, hourly or on-change) to know WHICH models to show. Not urgent until
building rendering is scheduled — flagged now so it's on your radar.

Extension (2026-07-09 review): building DAMAGE/REPAIR state too -- per
state or province building row: `{scope, scope_id, building_type, level,
healthy_level, damaged/damage_fraction, repair_progress,
under_construction}` -- landmarks/dams/locks render destroyed states
(MAP_3D_OBJECTS_PLAN). Construction-line rows `{tag, building_type,
target_state/province, active_factories, progress}` feed the construction
map mode later.

**STATUS (reconciled 2026-07-09): largely extractor-side LANDED** -- D48
`construction_buildings_state_damage_live` folds state/province building
rows with `level`, `healthy_level`, `damaged` (`world_fold.rs:9957`,
`:13049`). Some construction-line fields remain open on their side.
Replay-side ingest deferred until building rendering is scheduled.

## 11. Rail/supply dynamic state (when the rail layer lands)

Static rail/supply graphs are shipped files. Per-game rail damage/level
and hub status would make the supply layer truthful. Deferred, low
priority.

## 12. Battle plans / order arrows (DEFERRED — do not build yet)

Player-drawn plan arrows and frontline orders (schema §12 order_of_battle
plans). Deliberately deferred until the replay schedules plan rendering;
listed so the eventual ask isn't a surprise.

## 13. Per-country fog of war / visibility (PROMOTED 2026-07-09 — REQUIRED)

**Was "deferred — likely never needed"; now a core product feature by user
directive (2026-07-09):** the replay defaults to omniscient observer (whole
map visible, no FoW — that part still needs nothing from you), but the viewer
can enter a country's POV — seeing that country's fog of war and only what it
could see — and toggle back to the world view, exactly like the game observer
mode when you observe as a specific country.

What the replay needs, per POV country, per hour:

- **Province visibility state** from that country's perspective — at minimum
  a fogged/visible flag per province; if the engine holds more levels (e.g.
  terrain-seen vs actively-watched), preserve the levels rather than
  collapsing them. This drives the FoW terrain darkening layer.
- **Visible-foreign-units set** — which foreign divisions/fleets/wings (item
  3/8 unit ids) are spotted from that POV that hour, so POV mode hides
  unspotted units. If the engine stores per-unit intel levels (detected vs
  identified), preserve the level.
- Alternatively: if the raw per-country intel state the engine evaluates FoW
  *from* is cheaper to record than the evaluated result, record that instead
  and tell us — the replay can apply engine rules replay-side given the
  inputs. Whichever representation is cheaper on your end wins.

**Scope bound (keeps this affordable):** POV candidates are only the
*player-controlled* countries in the session (item 4 session metadata already
names them) — typically far under 32 tags. A per-province visibility bitset
over ≤32 POVs is roughly 1.6 KB/hour raw before deltas, and visibility
changes slowly, so compact deltas should make this cheap. Do NOT record POV
state for all ~200 AI tags unless it is free.

**Cadence:** hourly-or-finer like everything else; thinning to the hour
boundary is acceptable ONLY if that matches the engine's own FoW update
cadence — otherwise record at the engine's cadence (zero-data-loss rule).

---

## Process asks (standing)

- Schema deltas welcome: if the collector fold needs to deviate from a
  `REPLAY_SCHEMA.md` shape, send the delta — the schema doc gets amended
  rather than silently diverged from. Unknown domains are preserved raw,
  so additive growth is free.
- `extractor/native/snapshot` crate: the replay consumes it read-only as
  a path dependency for container reading. Its public API (`Store`,
  `Container`, `Snapshot`, `pack`) is a shared contract — change is fine,
  silent breakage is the only hazard; a one-line heads-up in a commit
  message is enough.

## Coverage status (replay side's view, for reconciliation)

| # | item | status (reconciled 2026-07-09) |
|---|---|---|
| 1 | hourly snapshots in container | extractor LANDED (R1); replay verify pending |
| 2 | owners/controllers per hour | shape COVERED (§4/§5); rides on 1 |
| 3 | divisions + land combats | shape COVERED (§6/§7), live-fed; counter-badge extension OPEN |
| 4 | session metadata | partial (session header LANDED; speed/pause + DLC display-names OPEN) |
| 5 | naval supremacy | extractor LANDED (D56); replay ingest pending |
| 6 | weather state | weather stream LANDED (D24); SnowMud accumulators OPEN |
| 7 | cosmetic-tag stream | extractor LANDED (D18); replay ingest pending |
| 7b | ruling party / ideology | schema shape exists (§3.3, live-collector); ride into snapshots |
| 8 | naval/air presence | AIR LANDED (D31); NAVAL rows OPEN |
| 8b | pair relations + faction + capitals | relation flags already live-extracted (d53); fold into snapshots. VP-icon column minimum = just at_war + same_faction (FURN-REL decompile) |
| 9 | localisation joins | partial (static artifact in their contract; verify it ships) |
| 9b | topbar scalar bindings | schema shapes exist (§3.1–3.3); extended set + outliner OPEN |
| 10 | building levels live | largely LANDED (D48 incl. damage); replay ingest deferred |
| 11 | rail/supply state | deferred |
| 12 | battle plans | deferred — do not build yet |
| 13 | per-country FoW / visibility | **PROMOTED 2026-07-09** (observer country-POV is now core UX); per-player-country province visibility + spotted-units per hour; OPEN on both sides |

The replay layer keeps this file current; anything new the replay needs
from memory gets added HERE (nowhere else).
