# EXTRACTION_NEEDS — the master no-gaps list (2026-07-07)

Everything the replay still needs from OUTSIDE the shipped installed content,
so nothing is discovered missing at integration time. Three sections:
data the **extractor/recording** must carry, **capture sessions** the user
can produce on request, and **law gaps** that need no new data (tracked
here so this list is the single completeness checklist).

Status legend: FILED = written into `EXTRACTION_INTERFACE_REQUESTS.md`
(the extractor team's channel) · READY = replay-side consumption already
implemented · OPEN = not yet requested/implemented.

## A. Recording / extractor-side data (per session or per hour)

| # | Need | Cadence | Why / consumer | Status |
|---|---|---|---|---|
| A1 | Owners + controllers per state | hourly | the core political map scrub | exists (recording format) |
| A2 | **SnowMudData texture** (1408×512 BGRA8) + scene `SnowMudFade` scalar | ~daily snapshot | snow/ice is accumulated simulation state (SNOW_LAW); proven to reproduce winter EXACTLY when supplied (frame4700: Germany to 3 decimals, Baltic ice fixed) | FILED (R-SNOWMUD) + READY (SRV substitution + CB0 fade write shipped) |
| A3 | Resolved cosmetic tag per country | on change | `set_cosmetic_tag` fires from events/decisions during play (annexations, puppets); static-1936 parse only covers game start | OPEN — file as R-COSMETIC |
| A4 | Ruling party / ideology per country | on change | display-name key `<TAG>_<ideology>` + ideology color variants; static-1936 from history only. CONSUMER LANDED 2026-07-09 (LAB4): on-map label texts are ideology-resolved — this is now render-blocking for mid-game correctness | FILED (R-IDEOLOGY + REPLAY_DATA_REQUIREMENTS 7b) |
| A5 | Recording's DLC list + modset | once per recording | NAME1b DLC-conditional cosmetics evaluator (CAN→CAN_UK vs CAN_ALY etc.); mod-resolver keying (MOD_COMPATIBILITY mandate) | modset partially exists; **consumer READY** — `cosmetic_tags_from_resolver_with_dlc(has_dlc)` + `merge_dlc_conditional_cosmetics` take the DLC set; default is all-DLC-present (correct for the common install + byte target). Wire the recording's real DLC set to enable the no-DLC render path. R-DLCLIST |
| A6 | Naval supremacy per strategic region: `{region_id, controller_tag, supremacy_magnitude}` (NOT a tri-state — the friendly/enemy/contested tri-state is POV-derived replay-side from `controller` + diplomacy, never recorded) | hourly | sea-zone GB-field top fills keyed by the region's sea-province set + dominance alpha (ledger R6) | **CONSUMER LANDED 2026-07-09** (`map_viewer::naval_supremacy_tint`, plan C6): tint = controller country colour, alpha = `NAVAL_REGION_TRANSPARENCY_*` law. Fold D56 into hourly snapshots as `controller` + `supremacy`. **Sharpen at integration:** magnitude scale — fixture emits normalised 0..1 while `NAVAL_REGION_TRANSPARENCY_RANGE=2000` implies raw dominance points; the extractor must state D56's unit so the alpha curve is engine-correct. |
| A7 | Division/unit positions + types per hour | hourly | close-zoom 3D unit models (the h<400 content pass; scoreboard error 0.03→0.135 below h=300 is mostly this) | OPEN — file when the unit-render lane starts; large item, needs schema design |
| A8 | Built railways / supply hubs state | on change | close-zoom railway pass (RailTexture + GB channels shader identified) | OPEN (static map rails derivable from files; BUILT rails are game state) |
| A9 | Weather state per strategic region | hourly | weather overlay animation (WEATHER1 law is files-first for rendering; the ACTIVE weather per region per hour is simulation state) | OPEN — replay-side wiring LANDED (lane/c-weather: `weather_law::from_recording_kind` maps the recorded `weather_kind` to the overlay type; `snow_mud_feed` rasterizes the accumulated feed). **PREFER the recording emit the `common/weather.txt` `weather_effects` KEY (`no_phenomenon/rain_light/rain_heavy/...`) not a friendly alias (`clear/rain/storm`)** — else the replay applies an inference map (see REPLAY_DATA_REQUIREMENTS item 6) |
| A10 | Game clock (date + hour fraction) per timeline hour | hourly | day/night terminator, seasonal sun tilt, tree/snow seasons | likely exists in recording (dates); confirm hour-of-day granularity — OPEN |
| A11 | Air/naval mission arrows, battles, fronts | hourly | war-state overlays (late-frame passes seen in the 1940 capture) | OPEN (far future; explicitly out of static-1936 scope) |
| A12 | Diplomatic pair-state (war/faction/subject/guarantee/NAP/access/embargo) + faction membership + capital_state per country | on change | map-furniture VP-icon diplomatic color column (furniture lane 2026-07-09) + capital-star relocation + war overlays; d53 relation flags already live-extracted — ask = fold into hourly snapshots | FILED (REPLAY_DATA_REQUIREMENTS item 8b) |
| A13 | Topbar scalar set per country per hour (political_power, stability, war_support, command_power, manpower.available, xp.*, factories.civ/mil/dockyard, fuel.current/max, convoy count) | hourly | UI-chrome topbar bindings (all 453 .gui parse; topbar rects gate ≤1px vs engine quads; every number is recording-gated) | FILED (REPLAY_DATA_REQUIREMENTS item 9b); schema shapes exist §3.1–3.3 |
| A14 | Per-country fog of war: province-level-or-finer visibility/intel state + spotted-foreign-units set, per PLAYER country only (session metadata names them; do NOT record all ~200 AI tags; do NOT record only a camera-space ShadowMap) | hourly (or engine FoW cadence if finer) | observer country-POV mode — core UX per user directive 2026-07-09; CAP4 proves terrain consumes a screen-space `ShadowMap` output packed with FoW color/alpha, so recording must carry the stable visibility/intel input that regenerates it for arbitrary camera | FILED (REPLAY_DATA_REQUIREMENTS item 13, PROMOTED from deferred 2026-07-09; sharpened by FoW CAP4 2026-07-09) |
| A15 | Division counter badge state: active ability kinds, low supply, orders kind, dig-in, training, attrition-active | hourly or on change | C1 unit-counter files-first pass found the on-map `active_abilities` grid plus the supply/order/dig-in/training/attrition badge slots in `interface/mapicons.gui`; missing fields degrade to badge-absent | OPEN — added to REPLAY_DATA_REQUIREMENTS item 3 |

## B. Capture sessions (user-produced, on request)

| # | Need | Unblocks | Status |
|---|---|---|---|
| B1 | Observer/no-FOW political frames (`observe` console) incl. 2–3 open-ocean mid-zoom | final clean political-sea confirmation (current sea parity is 0.94–0.98 away from ice; FOW was the referee contaminant) | WISHLISTED with user |
| B2 | Summer (June 1936) frames, standard heights | decouple snow from base terrain in full-frame gates | WISHLISTED |
| B3 | Terrain-mapmode mini ladder (3–4 heights) | mapmode-conditional CB0/state diffs (we infer political-vs-terrain from one sample each) | WISHLISTED |
| B4 | Night/dusk frames | day-night terminator + city-lights law (CityLightsAndSnowNoise consumer) | WISHLISTED |
| B5 | Static-camera time series (3 frames, seconds apart) | water/cloud animation clock laws | WISHLISTED |
| B6 | Multi-DATE frames (≥4 dates across the year) | seasonal sun-tilt law (January sun .z=+0.077 vs July −0.078 — sign flip measured, curve unknown) | OPEN — add to next wishlist |
| B7 | Close-zoom frames with units visible/moving | unit-model render lane references | OPEN — with A7 |
| B8 | qrenderdoc python-shell RT extraction | ONLY if statistical gates stop sufficing (currently NOT needed — initial-contents MainScene gives byte referees) | NOT NEEDED now |
| B9 | One political frame framing ≥3 HYPER-ELONGATED countries at once (Chile + Peru + Vietnam, or Norway + Sweden + Italy) at mid zoom | fit the NAME2 hyper-elongated ANCHOR law (C3): VP-weighted anchor moves Norway 122→53px but the aspect gate is fit to one example — needs ≥3 to pin the boundary + blend curve | OPEN — add to next wishlist |
| B10 | Follow-up FoW pair: observe-as-country (FoW ON) vs observer world view (FoW OFF), same camera and same clock, with ShadowMap producer inputs or pre-FoW intermediate captured, plus a labelled unit/counter/UI draw pass containing hidden enemy units | C8 residuals: producer-input parity + hidden-unit culling referee; CAP4 already closed the terrain carrier + shader blend | PARTIAL — CAP4 carrier mined; add producer/counter follow-up to next wishlist |

Keep all existing .rdc files until pixel-level extraction is exhausted.

## C. Law gaps (no new data needed — tracked for completeness)

| # | Gap | Evidence on disk |
|---|---|---|
| C1 | ~~NAME2 size law~~ CLOSED 2026-07-07: h = min(fill-to-arc, 0.22×extent-mask minor), extent mask includes owner-tagged impassable states, arc fraction 0.75, no ceiling; 89% of 38 joined engine labels within ±25% (gate `frame4700_label_size_gate.rs`); IRAN = documented residual (0.81×) | fixture `map_viewer/tests/fixtures/mapname/frame4700_labels.json` |
| C2 | ~~NAME2 angle law~~ RESOLVED-AS-PCA 2026-07-08: over the 38 engine pairs PCA is the BEST files-derivable orientation (median 4.0°, mean 6.7°, p75 9.1°). Every alternative tested is WORSE — max-extent direction 9.8°, 2–98 percentile extent 7.7°, VP-weighted PCA 6.8°; largest-sub-blob PCA is identical (no scattered sub-blob in the set). The earlier "FRANCE 57° off" was a stale scrambled join — the real metropole FRANCE matches (eng −84.2° vs PCA −85.3°). Residual is a thin tail (worst: Greece 29.5°, aspect 1.52, PCA pulled off the N–S mainland by the Aegean coast); no simple law fixes it without regressing the 34 good cases. NO CODE CHANGE. | 38-pair angle sweep `c2_sweep.json`, mined frame4700 |
| C3 | NAME2 letterspacing PARTIALLY closed 2026-07-08: shipped `MAX_LETTERSPACE_STRETCH=2.0` (engine ceiling for substantial labels: ITALY-boot 2.00, LATVIA 1.63, SWEDEN 1.51 — none higher over 68 labels), confining the word to the baseline's central 2.0×-natural span. Fully tames the extreme over-stretch (CHILE 8.1×→2.0×, CUBA 4.9×→2.0×). STILL OPEN: NORWAY only drops 2.53×→2.0× (21%) because its real error is the ANCHOR — the engine draws NORWAY compact over the populated SOUTH (y=279, arc 89) while our whole-country PCA centroid sits at y=194 with a 515-unit major axis. The hyper-elongated anchor law needs ≥3 such countries to fit (frame4700 has only Norway at aspect≈5) → capture ask B-series. SHARPENED 2026-07-08 (Codex-reviewed): VP-weighted anchor is the likely mechanism — an aspect-gated blend `lerp(plain_centroid, VP_centroid, smoothstep(4.5,5.0,aspect))` moves Norway 122→53px toward the engine's southern anchor with median/p90 unchanged, but the gate boundary is fit to a single example (CZE aspect 4.24 regresses below it), so NOT shipped pending a multi-elongated-country capture | frame4700 NORWAY decode; `MAX_LETTERSPACE_STRETCH`; VP-blend test |
| C4 | Furniture (VP/city icons + names) into HDR MainScene (finish LABHDR1) | engine draws mapicons pre-tonemap |
| C5 | NAME1b DLC-gated cosmetics (CAN etc.) | byte targets from 1936 GB texture (95,61,56 = Canada) |
| C6 | Sea-region border-line construction law (BORD5 skipped class) | engine draws them at mid zoom; strips in captured pool + RDOC draws |
| C7 | Shadow-render pass replay (then flip `shadow_map_bound`) | light-space CB0s present in all h<400 frames |
| C8 | Close-zoom content passes: 3D units/buildings/rails (shader family identified) | RDOC close frames |
| C9 | Seasonal sun-tilt law (date → sun .yzw) | two dates on disk; needs B6 for the curve |
| C10 | Hue-modifier unit (vanilla 0.0; assumed hue-circle fraction) | ledger note |
| C11 | RAMP_D0 for modded `GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS` | gb_field fail-closed guard |
| C12 | `ProvinceSecondaryColorMap` occupation colors vs the COL3 HSV law | secondary.bin extracted from frame4700, unchecked |
| C13 | Clouds/weather animation phase (cloud positions differ per frame) | B5 series would pin the clock |
| C14 | mapname glyph-atlas MIPS: engine binds hoi_mapfont4 with mips=4 + Linear MipFilter (fmt 87 B8G8R8A8_UNORM, frame4700 SRV), so mid-zoom titles sample ~mip 1 and render SOFTER than our single-mip upload; load the shipped DDS mip chain | frame4700 resources.json tex 322 |
| C15 | mapname per-draw CB scalar (cb slot3 float[20]: 3.08/10.35/10.74/13.77/23.31 across ladder frames, non-monotone in h — likely anim/wall-clock; identity matrix + (0,0,1,1) around it; Transp itself = 1.0 at every ladder height ≥288) | ladder cb_blobs decode 2026-07-07 |
| C16 | mapname day-night glyph dimming: `vSample.rgb *= 1 − DayNightFactor(CalcGlobeNormal(pos))×0.35` (mapname.shader:79) — labels dim 35% on the night side; needs the terminator function in the label pass (pairs with B4 night captures for the gate) | shipped shader source |
| C17 | **Mod map-SIZE genericity (2026-07-08, from the "different maps" mod-compat audit).** Country identity — tags, names, colours, cosmetics, DLC-gating — is 100% data-driven and renders ANY modset (verified: the only hardcoded tags/DLC/colour literals live in `#[cfg(test)]` gates; runtime reads all of it through `GameFileResolver` + file parsers; label size reads the loaded `.fnt` cap-height, placement reads `OwnerGrid.width/height` from the loaded raster). The one remaining vanilla assumption is the map GEOMETRY size: `PRODUCT_MAP_WIDTH/HEIGHT = 5632x2048` gates bundle load (`replay_app::bundle::validate_product_dimensions`), sets the east-west wrap period (`render.rs` `map_wrap_offsets`), and fixes the political-colour texture resolution (`political_color_synthesis` `PROVINCE_COLOR_MAP_WIDTH*2 = 5632` input assertion); camera (CAM1) and chunk (CHUNK1) laws are likewise tuned to it. Behaviour is **fail-closed**: a mod with a resized map is rejected with a clear error, never silently mis-rendered. The overwhelming majority of mods keep the vanilla map, so this holds for them. Lifting it = thread `bundle.width/height` through wrap/political-synth/camera/chunk and derive the colour-texture resolution from `provinces.bmp`; NOT done because it cannot be verified against the byte oracle without a differently-sized reference map (would violate the no-guessing / one-example discipline). **Needs a capture/bundle from a resized-map total-conversion to close** (new capture-ask candidate; the algorithms already take width/height as params, so it is a threading + re-gate job, not a re-derivation). | `bundle.rs` `PRODUCT_MAP_WIDTH`, `render.rs` `map_wrap_offsets`, `political_color_synthesis.rs:181` |

Maintenance rule: every future lane that discovers a data dependency ADDS
a row here in the same commit that discovers it; extractor-facing rows
get an R-entry in `EXTRACTION_INTERFACE_REQUESTS.md` when actionable.
