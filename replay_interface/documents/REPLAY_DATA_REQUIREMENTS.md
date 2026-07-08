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

Priority order: items 1–3 make replays function at all; 4–8 make them look
real; 9–12 are polish/deferred.

---

## 1. Hourly decoded world snapshots in the container (FOUNDATION)

Today the production recorder packs only the raw compact domain stream
(`role: collector_compact_stream`, `index.json` with `"snapshots": []`).
Nothing outside `recorder_host` can decode that stream, so a production
recording is not yet playable by anything.

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

Until it lands: the replay refuses compact-stream-only recordings with a
precise error and develops against snapshot-flavor fixtures.

## 2. Per-hour ownership & control (THE POLITICAL MAP)

Inside item 1's snapshots: `owner` and `controller` per state and per
province, per hour. This drives the political fill, borders, and the
occupation stripes (replay derives stripes from owner≠controller — no
extra extraction needed). Country colors themselves come from shipped
files; only the tags are needed.

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

## 4. Session metadata (playback pacing & player labels)

Speed/pause change events with timestamps, and the player↔country mapping
(who was human, which tag). Makes playback pacing authentic and lets the
UI label human players. Degrades to fixed-speed playback with no player
badges.

## 5. Naval supremacy per strategic region (SEA-ZONE BORDERS)

The sea-zone borders/fills players see over ocean in MP are a
naval-supremacy display. Measured on the replay side: with no supremacy
data the game renders NO sea lines, and the sea-zone distance field is
static map data the replay already has. Need: per-hour naval supremacy —
or at minimum a contested/friendly/enemy tri-state — per strategic region
id (ids join against `map/strategicregions`). Degrades to open ocean
(what the game shows with no naval data).

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

## 7. Cosmetic-tag stream (MID-GAME COLOR/NAME/FLAG CHANGES)

`cosmetic_tag` exists in the schema (§3.12) but is save-sourced today.
Cosmetic tags change a country's map color, name, and flag mid-game
(Vichy, civil-war splinters, puppet renames) — a replay that keeps
pre-switch colors is visibly wrong to every player. Need: on-change (or
hourly) `{tag, cosmetic_tag}` rows from live memory. The replay resolves
the actual colors/flags/names from shipped files — only the tag stream is
needed. Degrades to base-tag identity.

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

## 9. Localisation / token joins

The replay UI renders tokens the recording emits (focus ids, idea tokens,
tech names, decision ids). Need: a deterministic token → localised-name
join (effective mod-over-vanilla), either packed in the container or as a
documented static bundle keyed by the recording's mod identity. First
surfaces to hit this: map labels, topbar, country panels.

## 10. Building levels per state (live feed — when building rendering lands)

Schema §4 `buildings`/`province_buildings` (state-scoped levels) are
save-sourced today. The replay will render buildings on the map from
static placement files; it needs the per-game levels (which slots are
built, hourly or on-change) to know WHICH models to show. Not urgent until
building rendering is scheduled — flagged now so it's on your radar.

## 11. Rail/supply dynamic state (when the rail layer lands)

Static rail/supply graphs are shipped files. Per-game rail damage/level
and hub status would make the supply layer truthful. Deferred, low
priority.

## 12. Battle plans / order arrows (DEFERRED — do not build yet)

Player-drawn plan arrows and frontline orders (schema §12 order_of_battle
plans). Deliberately deferred until the replay schedules plan rendering;
listed so the eventual ask isn't a surprise.

## 13. Player-POV fog of war (DEFERRED — likely never needed)

The default replay is omniscient (whole map visible) — that needs NOTHING
from you. Only if we ever add a "see what player X saw" view would we
need per-country province visibility/intel state per hour. Listed purely
so the registry is complete; do not build.

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

| # | item | status |
|---|---|---|
| 1 | hourly snapshots in container | needed (foundation) |
| 2 | owners/controllers per hour | shape COVERED (§4/§5); rides on 1 |
| 3 | divisions + land combats | shape COVERED (§6/§7), live-fed; rides on 1 |
| 4 | session metadata | needed |
| 5 | naval supremacy | needed |
| 6 | weather state | needed |
| 7 | cosmetic-tag stream | schema shape exists; live feed needed |
| 8 | naval/air presence | raw streams exist; consumable rows needed |
| 9 | localisation joins | needed |
| 10 | building levels live | schema shape exists (save-only); deferred |
| 11 | rail/supply state | deferred |
| 12 | battle plans | deferred — do not build yet |

The replay layer keeps this file current; anything new the replay needs
from memory gets added HERE (nowhere else).
