
## R-SNOWMUD (2026-07-07, replay-side request): per-session SnowMudData snapshot
The engine's snow/ice is an accumulated pseudo-random simulation (see
documents/SNOW_LAW.md) — not derivable from files+date. RDOC parity runs
prove the replay reproduces winter EXACTLY when given the engine's
SnowMudData texture (Germany land tint to 3 decimals, Baltic sea ice ratio
0.58->0.93; full-frame mean|err| 0.0279->0.0201 at h=905). REQUEST: the
extractor records the SnowMudData runtime texture (1408x512 B8G8R8A8, the
SnowMudTexture SRV, quarter-res map addressing per SNOW_LAW.md) once per
recording session snapshot (daily cadence is plenty; it evolves slowly),
plus the scene SnowMudFade scalar. Replay-side consumption is already
implemented (SRV substitution + CB0 fade write; validation knobs
REPLAY_SNOW_MUD_OVERRIDE / REPLAY_SNOW_MUD_FADE show the wiring).

## R-COSMETIC (2026-07-07): resolved cosmetic tag per country, on change
`set_cosmetic_tag` fires from events/decisions during play; the replay's
static-1936 parse only covers game start. Request: record (hour, TAG,
cosmetic_tag_or_none) on every change. Consumer: name + map-fill color
resolution (NAME1 chain). Small data; event-driven cadence.

## R-IDEOLOGY (2026-07-07): ruling party per country, on change
Display names are `<TAG>_<ideology>` localisation keys and color variants
can key off ideology. Static-1936 from history only. Request: (hour, TAG,
ruling_party) on change.

## R-DLCLIST (2026-07-07, updated 2026-07-08): the recording's active DLC list, once
NAME1b's DLC-conditional cosmetic evaluator is IMPLEMENTED and LANDED
(`cosmetic_tags_from_resolver_with_dlc(has_dlc)` + `merge_dlc_conditional_cosmetics`);
it just needs the recording's DLC set to enable the no-DLC render path
(user mandate: no-DLC compatibility). Default today = all DLC present.
The modset is already bound; please add the active DLC list.
**Name-normalization gotcha (Codex-verified 2026-07-08):** the engine's
`has_dlc = "X"` compares against the DLC's DISPLAY name (`.dlc` file
`name = "Gotterdammerung"`), while the game's `dlc_load.json` lists
DESCRIPTOR PATHS (`dlc/dlc043_.../dlc043.dlc`). If the recording sources
its DLC set from `dlc_load.json`, please emit the resolved DISPLAY names
(or both), so the replay's `has_dlc("Gotterdammerung")` predicate matches
without a path→name table. Concrete cases that depend on it: CAN
(Together for Victory → CAN_UK else CAN_ALY), COG (Gotterdammerung →
COG_belgian_congo), BUL (Battle for the Bosporus → BUL_neutrality), XSM
(NOT No Compromise No Surrender → united_ma_clique_tag).

## R-CLOCK (2026-07-07): confirm hour-of-day granularity, once per hour
Day/night terminator + seasonal sun need date AND hour fraction per
timeline step. If the recording already carries full timestamps, no
action — please confirm the field.

(Heads-up, not yet actionable: R-UNITS (division positions/types per
hour) and R-WEATHER (active weather per strategic region per hour) will
be filed when the close-zoom content lanes start; schema input welcome
early. Master list: documents/EXTRACTION_NEEDS.md.)
