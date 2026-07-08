# MAP COMPLETION PLAN — the path to 1:1 with the real game, everything engine/file-derived

Standing directive (user, verbatim core): *"make sure that we have a clear path
and a clear plan to fully complete the map so that it actually derives
everything from the proper vanilla engine and installed content"* — and mod-compatible
throughout: *"if I just give you screenshots, the names of the countries, and
the correct 1936 data, how could that help? This needs to be mod compatible."*

Rules of evidence (enforced by FIDELITY_LEDGER.md + merge battery):

- **Screenshots** = acceptance tests only. Never a data source.
- **Capture sessions** = one-time reverse-engineering of engine ALGORITHMS that
  exist only in engine C++. The derived law must be a pure function of
  mod-resolved installed content + recording state — never of the captured bytes.
- **Scenario data** (names, colors, owners, weather, units) resolves at runtime
  from the modset's files via `GameFileResolver` + the recording's per-hour
  state feeds (REPLAY_DATA_REQUIREMENTS). Nothing vanilla-1936 is ever baked.
- Content-dependent laws carry a **modset-bound** label + re-validation plan.

## Acceptance rubric (from the user's vanilla mid-war screenshot, 2026-07-06)

Recorded observations that the finished map must reproduce (each row names the
mechanism that will produce it — no row is satisfied by imitating the image):

| Observable | Mechanism (runtime source) |
|---|---|
| Country label text is ideology/cosmetic-resolved ("VICHY FRANCE", "LEGIONARY ROMANIA", "NATIONALIST SPAIN", "GREATER HUNGARY") | localisation join: per-hour {tag, cosmetic_tag, ideology} feed (§2/§7/§9) → name key → modset localisation |
| Labels follow CONTROL and merge contiguous controlled territory (occupied France under one spanning "GERMAN REICH"; UK-held Sardinia labelled "UNITED KINGDOM") | per-hour controller feed (§2) → controlled-region components → placement law (Phase 1, mapname_pass) |
| Multiple labels per country (German Reich appears per detached region) | placement law operates per controlled component |
| Letter-spaced, curved, semi-transparent dark glyphs w/ light outline, size scales with region | hoi_mapfont4 atlas (pre-colored, proven) + placement/size law from mapname_pass capture |
| Occupied land fills with controller color (dark grey France) | political color law (proven; user-corrected: no stripes in normal play) |
| Thin dark country borders, lighter state borders, faint sea borders, smooth corners | border_*.dds ink profiles (proven, BORD3) + polyline/corner law from borders_pass capture |
| City/VP name labels (white text: "London", "Berlin"…), capital stars, VP bullets | map furniture: map/cities + victory points + localisation via resolver (Phase 3, MAPICON1) |
| Port anchors, naval/air base icons, radar, supply hubs | building/state feeds (§10) + gfx interface icons via resolver (Phase 3) |
| Unit counters (numbered badges), battle icons | on-map unit feeds (§3/§8) + counter gfx (Phase 3/extractor-gated) |
| Weather: cloud/snow overlays (visible over the Alps/Italy) | weather state per strategic region feed (§6) + weather rendering law (Phase 3) |
| Terrain colors/relief, rivers, water look, day/night, snow | CLOSED laws (terrain/water/light/snow) — validated by capture, sourced from files |

## Phase 0 — DONE or in flight (files + existing captures)

Closed and merged: grading (LUT byte-exact, bloom 7-sig-fig, per-frame
exposure), water reflection (shipped DDS byte-identical) + underwater
refraction, complete light-rig/day-cycle laws (camera-follow sun removed),
border geometry (province edges + impassable) + border texture ink laws,
political color composition, snow laws, wall-clock animation, infinite wrap,
mod-aware resolver everywhere, capture instrumentation hardened (borders_pass
identification fail-closed; underwater_pass moment added). Graduated defaults:
grading + water render files-first BY DEFAULT (borders/labels intentionally
held back — see Phase 2). IN FLIGHT: CHUNK1 (terrain chunk mesh/LOD generated
from heightmap instead of cloned from capture — kills the largest captured
dependency; unlocks arbitrary cameras + mod maps).

## STATUS UPDATE 2026-07-07 (overnight autonomous run): PHASES 1 + 2 DONE

Phase 1 executed SOLO (dual-instance + input injection; zero user input
needed after all): sessions CAPFIX2_GAPS_4 (trees/underwater/mapname +
borders decisive-negative), MAPNAME_7B (23-label far-zoom VB via zoom-race),
NIGHT_SWEEP_6 (2-cycle unpaused CB0 hour sweep). All four Phase-1 lanes
merged: LAB3 (VB decode exact; arc law RMS 0.158 wu vs 23 engine labels),
BORD4 (vanilla borders = gradient-border system, already default-rendered;
border.shader never even created in normal play; mod-compat defines fix),
WATER5 (engine applies NO underwater CB0 absorption — constant reframed as
documented fallback), NIGHT1 (.yzw HOUR-INVARIANT; hour lives in .x only;
.z drifts with date). Plus TREES2 (missing second tree sub-draw = false
premise, it is the LOW_END_GFX tier shader, RESOLVED), MAPICON1/WEATHER1
(Phase 3 furniture laws merged, flag-gated), LEDGER2 (ledger burn-down +
render-path audit).

Phase 2: labels graduated DEFAULT ON with the per-component law (LABGRAD1;
--no-map-labels opt-out byte-identical). Borders needed no graduation —
the GB system was already the default vanilla mechanism. **Baseline v4
locked: baseline_2816_v4.bmp = 2dc14b1eb46c68817ef2db119fac17a9** (labels
on, deterministic 2x); v3 (307b88b6) remains the --no-map-labels lock.
QA 16/16 green on the final state.

Remaining (tracked, evidence-gated, none blocking the map's current
files-first state): FoW.w!=0 night-look capture validation; .z(date) map
(same-hour/different-date captures); label size-floor/kerning/anchor rule
(multi-zoom capture); underwater TerrainColorTint parity oracle (then
remove the fallback constant); mapname owner-raster acceptance gate;
Phase 4 mod smoke test + extractor-fed dynamics.

## STATUS UPDATE 2026-07-07 (parity shift): FURN1 — furniture composite DEFAULT ON

Rubric row 7 (city/VP labels, capital stars, VP dots) now RENDERS in the
default frame (`replay_app/src/map_furniture.rs`; baseline v7 = 688a1451,
opt-out chain v6/v5/v2 byte-identical). Live-oracle acceptance on a solo
vanilla instance confirmed the shape law against shipped VP values
(Melbourne 20→circle, Adelaide 15→pentagon, Canberra 30+capital→star) and
produced two ENGINE-MEASURED corrections the naive .gui reading gets
wrong: VP text = hoi_mapfont4 x0.40 white-inverted (advance-sum exact on
two words), icon = authored frame x1.25 (see FIDELITY_LEDGER §2.15b).
Also SEA1 (evidence-only): sea-zone lines OFF at the 1936 start is
ENGINE-FAITHFUL — ch2-top green==0 gates the GB sea layer off; the lines
appear late-game via naval-supremacy writes (extractor R6).
Residual furniture slices: FURN2 port anchors / air bases (containers at
mapicons.gui:887/:1020, cutoffs 900, file-derivable static levels);
observer color-column pin (needs a 1936-observer screenshot); units /
counters (extractor-gated).

## Phase 1 — ONE 5-minute vanilla capture session (DONE — see status; original plan kept below)

Recipe (final): `--capture-moments restore_pass,terrain_active_pass,water_pass,
rivers_pass,mapname_pass,borders_pass,underwater_pass --enable-map-frame-rounds
--cb0-sample-ms 2000 --dwell-ms 300000`; user: launch, unpause, one zoom-out;
5-minute cap. Extracts four engine algorithms in one shot:

1. **mapname_pass** → label placement/curvature/size/count algorithm (~90
   worked examples to reverse). → lane LAB3: placement law as f(controlled
   components), names from localisation joins. HARD GATE: reproduce the
   captured geometry + acceptance rubric rows 1-4.
2. **borders_pass** (identification now fail-closed) → real border VB: corner
   treatment, per-class width behavior, vTime_Transparency.y fade curve,
   runtime BorderDiffuse compositing. → lane BORD4: geometry/corner law as
   f(provinces.bmp). Kills the "jagged staircase" defect lawfully.
3. **underwater_pass** → engine's underwater CB0 light state + t0. → lane
   WATER5 (DONE 2026-07-07): decoded the engine's OWN underwater pass CB0
   (`session_CAPFIX2_GAPS_4`) — it carries the scene light rig UNCHANGED (white
   un-absorbed sun, day-gated), so the "modset-bound absorption constant" is
   NOT an engine light law (WATER3 hypothesis refuted). The composition law is:
   *underwater pass = scene CB0 light rig, no absorption*; the dim blue is the
   shipped PS's `lerp(TerrainColorTint×0.5, lit, vWaterAlpha≈0.4675)`. The
   `(0.133,0.234,0.570)` constant is retained as an output-matching fallback
   (default pixels unchanged) pending a texture-parity oracle. See
   `WATER_LAW.md` §2a-ter, bin `decode_underwater_state`.
4. **SAMP1 samples across game-hours** (game runs unpaused at speed) → night-
   side sun curve, DayNight .yzw(hour), FoW.w≠0 night look. → lane NIGHT1:
   night validation + city lights (CITY_LIGHTS_* constants already in
   constants.fxh).

## Phase 2 — graduation to default + re-lock

When LAB3/BORD4 land their laws: borders + labels graduate to default ON
(same pattern as GRAD1: opt-outs, fail-open defaults on missing evidence,
explicit flags fail-closed). Baseline re-lock v4. At that point the DEFAULT
app renders every layer files-first.

## Phase 3 — map furniture (files + extractor feeds; rubric rows 7-10)

- MAPICON1: city/VP name labels, capital stars, VP markers — map files +
  localisation via resolver. File-derivable now; placement conventions may
  need one ui-ish capture moment if not in files (report-then-crack).
- Ports/airbases/radar/supply icons: building levels per state (feed §10) +
  interface gfx. Weather overlay: feed §6 + weather render law (shipped
  cloud/snow assets; engine compositing law may need one capture moment).
- Units/counters/battles/rail/arrows: extractor feeds §3/§8/§11 (other
  agent) + counter rendering. EXTRACTOR-GATED — enumerate, do not fake.

## Phase 4 — acceptance + mod proof

1. Side-by-side vs the real 1936 save at fixed cameras (the standing
   comparison standard) + this rubric.
2. **Mod smoke test**: run a modded recording (different map/countries/
   localisation) end-to-end; grep-level assertion that no vanilla-literal
   path or 1936-literal string exists in the render path; modset-bound laws
   re-validated or flagged.

## Dependency summary (what blocks what)

- USER: the one 5-min session → Phase 1 lanes → Phase 2 graduation.
- EXTRACTOR (other agent): feeds §2/§3/§6/§7/§8/§9/§10 → Phase 3 furniture
  + time-dynamic political map. All itemized in REPLAY_DATA_REQUIREMENTS.md.
- NOTHING ELSE blocks: Phases 0 code lanes proceed autonomously.
