# Replay-layer data requests — extraction-side reconciliation (2026-07-04)

Source of the asks: `replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md` +
`EXTRACTION_INTERFACE_REQUESTS.md` (R1–R10) in the replay agent's worktree
(`nifty-jemison-bbc86f`). The replay layer does not have visibility into this
session's extractor progress, so several of their statuses are stale. This doc
is the extraction-side answer: per request, what is ALREADY DONE, what is
CRACKED-BUT-NOT-WIRED, what is a NEW work item (added to the backlog below),
and what is genuinely fixture-blocked.

Verified against HEAD `d1aca56e8` (collector/recorder code reads cited inline),
build gate `870ebf1f...`, live save D04 1944-02-11.

## Per-request status (extraction-side truth)

### R1 — Hourly world snapshots inside `.battleplan` — **DONE (their info stale)**
The production recorder already folds the compact stream into per-game-hour
world snapshots and packs them as `role:"snapshot"` entries with `hour_index`
in `index.json`, alongside the raw compact stream:
`recorder_host/src/main.rs:1008-1022` (compact `StreamFormat` → `WorldFold` →
`finish()`) and `main.rs:2313-2320` (`snapshots/{hour_index}.snapshot.json`
entries + `index.json.snapshots`). `pack_replay verify` gates it every session.
Their claim `"snapshots": []` describes the pre-world_fold recorder. Only the
JSON debug stream packs an empty snapshots array. **Action: none — tell the
replay layer to point `replay_core` at a current compact recording.**

### R2 — Schema deltas — process, ongoing
We already project non-schema shapes as `_r2_*` additive keys (see
`DOMAIN_REGISTRY.md` per-domain fold notes). Deltas continue to flow via that
convention. No new work item.

### R3 — Localisation / definition joins — **PARTIAL, packaging item added**
Shipped today as static artifacts (`DOMAIN_REGISTRY.md` "Static Artifacts"):
`idea_tech_loc_xref_static`, `decisions_static_definitions`,
`ship_definitions_static`, `gfx_icon_asset_manifest_static`, domain-59
templates/equipment catalog. NOT yet covered: focus names, state/VP names,
country names (incl. cosmetic-tag name overrides). **Backlog item N4** —
static-bundle work over shipped installed content, no memory crack needed.

### R4 — Session metadata (speed/pause/player map) — **PARTIAL, 1 real crack left**
Domain 57 is a validated partial slice in production registry: per-player
human TAG rows + pause state fold under
`_r2_session_multiplayer_metadata_live` (`DOMAIN_REGISTRY.md` row 57). What
remains fail-closed: the **game-speed root** (unvalidated) and MP-only
session-event fields. **Backlog item N3 (crack: game-speed root)**; MP-only
fields stay on the MP fixture list (needs 2nd player).

### R5 — `snapshot` crate API stability — accepted
Protocol note added: any commit touching `extractor/native/snapshot` public
API (`Store`, `Container`, `Snapshot`, `pack`) carries a heads-up line in the
commit message. No code work.

### R6 — Naval supremacy per strategic region — **CRACKED (33/33), port+wire needed**
Already solved in the python prototype layer
(`extractor/memory_reader/naval_air/README.md`: "Sea-zone control … SOLVED
33/33"): region object `MODULE+0x29468B8` → `+0x58` region id, `+0xE8`
container `MODULE+0x295DF30` → entries (`MODULE+0x295DEE0`, stride `0x50`),
per-TAG dominance values `{current, target, base_target, previous,
decline_from, individual_ratio}` — semantics confirmed as naval supremacy via
mod defines (`RESEARCH_naval_air.md`). Never ported to the native collector.
**Backlog item N2 (port python chain → compact domain + fold)** — satisfies
the replay tri-state ask and more.

### R7 — Weather state per strategic region — **COVERED (their info stale)**
Domain 24 `weather_live` emits 305 per-region rows with phenomenon flags
(rain light/heavy, snow, blizzard, arctic water, sandstorm, phenomenon-active)
plus per-province temperature/precipitation raw values, folded under
`map._r2_weather_live` (`world_fold.rs:243-271`, registry row 24). This meets
`{region_id, weather_kind, intensity}`. Open weather items (enum/particle
parity, causal internals) do not block the replay ask. **Action: none — point
replay layer at `map._r2_weather_live`.**

### R8 — Live cosmetic-tag stream — **DONE (wired + live-verified, commit 5f7c75666)**
cosmetic_tag now flows live in domain-18 `country_identity` →
`countries[*].cosmetic_tag`. Live paused capture: 29 present byte-exact vs the
melt (FRA→FRA_FREE, DEN→DEN_occupied_denmark, D04→FRA_VICHY, …), 0 false
positives; the only melt-only 3 are dynamic observers D01/D02/D03.

⚠️ **SCHEMA DELTA for the replay layer (please update §3.12 binding):** the live
feed makes `cosmetic_tag` a **tri-state**, richer than §3.12's save-based `""`
convention:
- present → the cosmetic tag string.
- **`null`** → measured, country has NO override (was `""` in the save-based §3.12).
- **key absent** → not measured / unreadable this frame (a state the save never had).

Please bind `cosmetic_tag` as `string | null | missing`. If the renderer wants the
old convention, project `null`/missing → `""` at your boundary (both mean "use the
base tag's color/flag/name"). Emitting `null` instead of `""` is deliberate
(fail-closed: absent ≠ null ≠ empty) — the recorder will not fabricate `""`.

### R9.1 — On-map naval presence — **LARGELY COVERED + wave-4 wiring queued**
- Task-force position rows are production-integrated: domain 56
  `naval_missions_status_live` emits full-replace rows
  `{task_force_id, fleet_id, owner, current province, derived region,
  sunk-convoy aggregate}` (`collector/src/lib.rs:3072`, `10739+`; registry
  row 56).
- Ship composition: domain 29 `ships_live` carries per-ship rows with
  task-force linkage (registry row 29) — `ship_count` and hull mix are
  derivable.
- Mission kind / home base / repair: cracked THIS SESSION without fixtures
  (`WAVE4_ATTEMPTCRACK_870EBF_D04.md`: CTaskForce+0x364 mission enum, +0x1E0
  location, CFleet+0xD8 home_base, +0x4B0 repair_target) — wiring already
  queued as wave-4.
**Backlog item N5 (verify/fold): confirm the folded per-hour shape gives the
replay `{task_force_id, owner_tag, location, ship_count,
dominant_hull_class}` directly; add the D29→D56 count/hull aggregation to the
fold if it does not.**

### R9.2 — On-map air presence — **COVERED (their info stale)**
Domain 31 `air_region_activity_live` folds per-region, per-owner rows
`{region_id, owner_tag_db_id, mission_mask, wing_count, plane_count,
effective_planes}` into §13.1/14 (`world_fold.rs:9278-9285`). Domain 30 owns
OOB air-wing existence; domain 34 covers air bases. This meets the "region
air-activity icons" ask. **Action: none — point replay layer at
`_r2_air_region_activity`.**

### R10 — Battle-plan arrows — deferred (their call), partial data exists
Domain 27 `army_orders_live` (battle_plan_orders) and domain 54
`army_battleplans_depth_live` (order path geometry `+0x70/+0x78`, type-5
target states) already carry live slices. When the replay layer schedules plan
rendering, reconciliation starts from those two domains. No new item now.

### Other rows in their section A–H (quick truth check)
- Political map owner/controller (§A): covered — domain 22 `map_control_live`.
- Naval battle icons (§C): **now live** — naval 49/50 wired this session and
  validated on D04's 4 active naval battles (commit `f68a6deed`). Their
  "fixture-dependent" is stale for naval. AIR combat rows remain genuinely
  fixture-blocked (need live air combat — already on the user fixture list).
- Land battle `progress` validation (§C): needs live combat ticks — already on
  the user fixture list.
- Sub-hour movement deltas (§B): replay layer's own DERIVED fallback stands;
  raw delta stream stays a later slice. No new item.
- Topbar/production/research/focus panels, player names, flags art, mod
  identity (§G/H): covered as they already state; flags picking needs N1.

## NEW BACKLOG ITEMS (added to the extraction list)

Priority order within the existing queue (after the wave-4 wiring already
queued, before user-fixture items — except N3 which can join any wiring batch):

- **N1 [WIRE] cosmetic_tag → compact production (R8).** Add to D18
  `country_identity` rows (offset `CCountry+0x1488`, validated reader exists in
  collector legacy path) + world_fold into `countries[*].cosmetic_tag` (§3.12).
- **N2 [PORT+WIRE] naval supremacy per region (R6).** Port the SOLVED python
  chain (`naval_air` README) into a native compact domain (new id or D56
  sibling) emitting per-region per-TAG dominance (or at minimum the tri-state);
  fold under a `_r2_*` key joined on strategic region id.
- **N3 [CRACK] game-speed root (R4).** The only genuinely uncracked memory item
  in the replay list. Pause state is already validated in D57; find + validate
  the speed integer root on build 870ebf (paused save gives a fixed oracle:
  speed value visible in UI).
- **N4 [PACKAGE] localisation bundles (R3).** Extend the static-artifact
  family with focus-name, state/VP-name, and country-name (incl. cosmetic
  overrides) localisation joins, mod-over-vanilla, keyed by recording mod
  identity. Shipped-file work, no live process needed.
- **N5 [VERIFY/FOLD] consumable naval per-hour rows (R9.1).** Confirm folded
  shape supplies `ship_count` + `dominant_hull_class` per task force per hour;
  if not, aggregate D29 ship rows onto D56 task-force rows in world_fold.

## Schema deltas shipped by E1 (capitulation / war state, domain 60)

- `countries[*].capitulation` (NEW, additive): `{capitulated: bool,
  capitulated_date: "Y.M.D.H"|null, last_surrender_date: "Y.M.D.H"|null,
  faction_join_date: "Y.M.D.H"|null, legitimacy: float, legitimacy_raw: int
  (x1e5), exile_army_leaders: int}`. Absent key = country not measured
  (absent != false). Persistent last-proven-frame semantics across hours.
- `diplomacy_world.wars` (CHANGED: was a dead `{}` placeholder, now
  **nullable + populated**): `null` when no proven domain-60 frame landed
  that hour (unknown), else an object keyed `"FIRST|SECOND"` (resolved TAGs)
  with `{first, second, first_tag_db_id, second_tag_db_id, start_date,
  start_date_raw, first_casualties, second_casualties,
  casualties_semantics: "live_running_accumulator"}`. A proven frame with
  zero wars is a genuine empty object, distinct from null. Casualty fields
  are LIVE running accumulators: byte-exact vs the save for frozen wars,
  legitimately above the save snapshot on active wars.
- Date strings use the repo-canonical renderer (hours 0-23): the save melt's
  day-boundary "hour 24" (`Y.M.D.24`) renders as next-day hour 0 — same raw
  instant; consumers comparing against melt text must normalize.

## Schema delta shipped by E13 (measured-country observability, infra)

- `measured_countries` (NEW top-level snapshot field): the set of countries the
  capture RESOLVED this frame (domain-18 full-replace roster). Shape:
  `{known: bool, source: "country_identity", count: N, tag_db_ids: [..],
  tags: [..], note}` when a proven domain-18 record landed this frame; else
  `{known: false, source, reason}` — the measured set is UNKNOWN, never
  reported as empty/complete. This is DISTINCT from `tag_db_id_index` (which
  reports whether the ids REFERENCED this hour resolved to TAGs). Consumer
  rule: with `known: true`, a country ABSENT from a domain's rows but PRESENT
  in `tag_db_ids`/`tags` has NO DATA for that domain (it was measured); a
  country absent from `tag_db_ids` was NOT RESOLVED this frame (the systemic
  country-roots bound excludes dynamic observers e.g. D01/D02/D03 — ~356 of
  ~437 measured). With `known: false`, neither can be inferred this frame.
  Additive (existing snapshot fields unchanged); consumers on older
  containers see the field absent.

## Messages back to the replay layer (stale statuses to update on their side)

1. R1 is shipped — compact production recordings contain hourly
   `role:"snapshot"` entries; `"snapshots": []` only occurs for the JSON debug
   stream.
2. R7 weather is live today under `map._r2_weather_live` (kind flags +
   temp/precip intensities per region).
3. R9.2 air region presence is live today under `_r2_air_region_activity`
   (incl. `mission_mask`).
4. Naval combat rows (§C) are live-fed as of commit `f68a6deed` (naval 49/50
   production integration); air combat rows remain fixture-blocked.
5. R6/R8 are cracked; wiring is queued (N1/N2). R4 speed root is the only new
   reverse-engineering item their list produced.
