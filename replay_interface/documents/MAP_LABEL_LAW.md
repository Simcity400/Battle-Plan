# the game Country Map-Label Law

Status: research-only tooling, **blocked in `replay_app` product rendering** as
of the 2026-07-09 no-fudge cleanup. Current validation code uses the reverse-engineered
`countryname.cpp` placement and title-size laws in
`replay_interface/native/map_viewer/src/map_label_law.rs`: one label per
controlled connected component, engine dual-OLS/offset-trial/owned-run segment,
engine title scale constants, shipped font/string/cutoff inputs, and no PCA
segment fallback. The old 2D camera-height substitute and 25% fade ramp are
removed. The old non-home de-clutter fraction and 16-cell fit floor are also
removed: there is no app-owned threshold deciding which detached components are
"worth" naming. Remaining gaps are documented UNKNOWN-ENGINE-SIDE items below
(owner-vs-controller keying, exact component-emission limits, `Transp_OffsetX`,
collision/de-overlap, and any future runtime ideology feed).
Two independent Codex reviews (game-file sweep + capture-evidence audit)
corroborated the files-first verdict; both are summarised below.

This supersedes OFF-4's straight-label approximation
(`GAME_FILES_ATLAS.md:524` "Map label curving/layout" gap;
`GAME_FILES_EXTRACTION_LEDGER.md:91` ledger item 23).

## 1. What the shipped files DO carry (FILE-DATA, cited)

- **Font.** `interface/core.gfx:1679`-`interface/core.gfx:1680` declares
  bitmapfont `tahoma_60` with `path = "gfx/fonts/hoi_mapfont4"`,
  `color = 0xffffffff`. This is the sole country-name map font. The glyph atlas
  page is `gfx/fonts/hoi_mapfont4.dds` (2048x1024). Japanese override at
  `interface/core.gfx:1693`. Loaded through the mod-aware resolver
  (`MOD_COMPATIBILITY.md`).
- **Font metrics** (`gfx/fonts/hoi_mapfont4.fnt`):
  `info face="Tahoma" size=84 bold=1 italic=0 unicode=1 stretchH=100 aa=2
  padding=2,2,2,2 spacing=1,1`; `common lineHeight=84 base=70 scaleW=2048
  scaleH=1024 pages=1`; `chars count=575`;
  `page id=0 file="Hoi4_mapfont_tahoma84_merged_0.dds"`. Sample glyphs
  (`id x y width height xoffset yoffset xadvance`):
  `71(G) 951 477 50 57 0 16 52`, `101(e) 987 704 43 45 -1 28 41`,
  `114(r) 1373 743 31 43 2 29 30`. The `.fnt` is the size input to the label's
  glyph rasterisation; replay_app no longer reads the committed extracted
  `fonts.json` manifest and instead resolves bitmap fonts directly from
  `interface/core.gfx` through the active game/mod resolver.
  The atlas is **pre-coloured** (`common ... alphaChnl=1 redChnl=0 ...`, every
  `char chnl=15`, DDS = uncompressed 32-bit BGRA): each glyph is a baked, colour
  outlined image — coverage in ALPHA, dark body + light rim in RGB — not a
  single-channel mask. (`hoi_18mbs`, the overlay clock font, has the identical
  encoding.) The label draw preserves this RGBA; see §3 step 9.
- **Display strings.** `localisation/<language>/*.yml` country display names,
  keyed by the plain tag (`GER: "Germany"`). Ideology / cosmetic variants
  (`GER_fascism`) need game state the recording does not carry and stay out of
  the label. Merge law per `map_viewer::map_text` (mod overrides win file-by-
  file). Vanilla english yields ~129k keys (validation-bin run below).
- **Visibility scalar.** `common/defines/00_graphics.lua:781`
  `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF = 260` — the shipped comment is exactly
  "Cutoff for drawing country names on the map"; it declares a cutoff, not a
  fade. The only shipped numeric law tying **country-name** visibility to camera
  distance. The replay now uses it as a hard cutoff against the production 3D
  camera height; the old soft 25% fade ramp and 2D visible-height substitute are
  removed. (`:758 PROVINCE_NAME_DRAW_DISTANCE = 500.0`
  is the analogous *province*-name cutoff; not used for country labels.)
- **Draw shader.** `gfx/FX/mapname.shader` (`Effect mapname` at
  `gfx/FX/mapname.shader:110`). It renders a pre-built vertex buffer:
  `VS_INPUT { float3 vPosition : POSITION; float2 vTexCoord : TEXCOORD0 }` — the
  positions are **3D world coordinates on the map plane**. `ConstantBuffer(1,32)
  { float4 Transp_OffsetX }` (`gfx/FX/mapname.shader:24`-`:26`): `.x` is the
  per-label transparency (`vSample.a *= Transp_OffsetX.x`,
  `gfx/FX/mapname.shader:78`), `.y` is a world-X offset applied to every vertex
  (`vPos.x += Transp_OffsetX.y`, `gfx/FX/mapname.shader:52`) for the east-west
  map wrap. The pixel shader dims rgb by up to 0.35 via day/night
  (`vSample.rgb *= 1.0f - ( DayNightFactor( CalcGlobeNormal( v.vPrepos.xz ) ) *
  0.35f )`, `gfx/FX/mapname.shader:79`). Blend is
  `src_alpha`/`inv_src_alpha` (`gfx/FX/mapname.shader:90`-`:91`). **The shader
  only rasterises whatever glyph strip the engine hands it; it builds no
  geometry.**

## 2. What the shipped files do NOT carry (ABSENT — searched)

- `map/positions.txt` is **0 bytes** (empty) in vanilla 1.16, while
  `map/default.map:3` still binds `positions = "positions.txt"`. The province
  text-position slots the older format doc describes are not populated here — so
  no per-province/label anchor is shipped.
- No `name_position` / `text_position` / `name_rotation` / `label_position` /
  `text_angle` / `text_curve` key exists anywhere under `map/`, `common/`, or
  `history/`. `history/states/*.txt` (1081 files) carry only
  id/name/manpower/state_category/history/provinces/local_supplies — **no**
  position/rotation/label block.
- `common/countries/*`, `common/country_tags/*`,
  `country_metadata/00_country_metadata.txt` (career-profile background GFX
  keys only), and `map/strategicregions/*` carry no name-geometry.
- The interface `.gui`/`.gfx` `country_name` fields (menubar, panels,
  diplomacy, …) are **UI-panel** text, not world-space map labels.
- `interface/core.gfx:713`-`:780` `animatedmaptext` objects (MessageMapText,
  ProvinceTaxText, CombatLoss, …) are floating combat/tax text in font
  `AnimatedMapText` (`gfx/fonts/hoi_18`) — not country labels.
- `gfx/FX/mapname.lua` referenced by `gfx/maparrows/maparrows.txt:305`-`:307`
  (`arrowText { effect="gfx/FX/mapname.lua" technique="mapname" }`) does NOT
  exist as a shipped file; the engine resolves the `mapname` technique from
  `gfx/FX/mapname.shader`. That path is the **battle-plan arrow text** (font
  `hoi_arrow_font` = Ubuntu 40), separate from country labels.
- No `LABEL`/`NAME`/`CURVE`/`SPACING`/`LETTER`/`GLYPH` size/curvature constant
  for country labels exists in any `common/defines/*.lua`.

## 3. The reconstructed placement law (DERIVED-BY-FIT)

Because placement is engine-computed, the replay reconstructs it from the only
shipped inputs — province geometry (`map/provinces.bmp` -> `province_ids`) and
ownership (`history/states/*.txt` owners, per moment). Implemented in
`map_viewer::compute_country_label_placements`
(`replay_interface/native/map_viewer/src/map_label_law.rs`). `replay_app`
blocks `--map-labels` and `--map-labels-fallback`; use `map_viewer`
`dump_map_labels` only as development tooling until the remaining engine law is
ported.

Coordinate frame: map-pixel space (`x` right, `y` down) = the province raster =
the engine world plane `x`/`z` 1:1 (`map/ambient_object.txt` world positions and
the 5632x2048 raster share units; the middle `ambient` coordinate is height).

1. **Labelable pixel cloud.** All owned, passable, non-water pixels of the
   country. A pixel counts iff `province_id != 0`, it maps to a state
   (`province_to_state`), the state is not `impassable = yes` (wasteland has no
   name), and the state's owner is the tag. Sea/lake provinces have no state
   mapping and are excluded automatically. Sampled on a stride-4 grid.
   [FILE-DATA inputs; the "impassable never labels" rule is DERIVED-BY-FIT,
   matching the engine painting wasteland nameless.]
2. **Per-component labelling (LABGRAD1).** Label **every controlled
   4-connected component** (east-west wrap included), not just the home one.
   Contiguous controlled territory is one component → one label (occupied France
   under a land-contiguous German Reich is one spanning label); a detached
   holding across water is a separate component → its own label. This is the
   LAB3 refutation of one-per-country: the captured far-zoom frame draws UNITED
   KINGDOM as **8 separate labels**, FRANCE / REGNO DEL SUD / BRITISH IRAQ twice
   each (§5). The replay now emits every component that resolves through the
   ported `countryname.cpp` dual-OLS/offset-trial/owned-run segment law. There is
   no replay-owned minimum-sample floor, VP survivor rule, or non-home area
   fraction; if the engine has an additional absolute de-clutter rule, that rule
   remains UNKNOWN-ENGINE-SIDE rather than approximated.
   The **home component** is still identified per owner — the one containing the
   single highest-VP province (its capital; London/Paris/Rome/Berlin each ~40-50
   VP), ties broken by summed VP then area — but now only to sort it first in the
   output (`is_home`). Peak VP,
   not summed VP, is the capital discriminator: a sprawling contiguous empire
   aggregates *more total* VP than the small home island, but no colonial
   province out-scores the capital. A country with zero VP anywhere scores 0 and
   its largest-area component is the home (the pre-LAB2 behaviour).
   DERIVED-BY-FIT for home ordering only. **Measured count must be regenerated
   after the fail-closed threshold removal**; old 171-label dumps came from the
   now-removed de-clutter filter.
   Whether the engine's split is strictly per-connected-component vs
   per-state/strategic-region is itself underdetermined (§5.1) — per-component is
   the leading hypothesis, consistent with (not proven by) the 8-UK symptom.
   **Note (x-wrap):** raster 4-connectivity wraps in x, but the PCA uses raw
   coords, so a component spanning BOTH map edges would be mis-fitted. This is
   benign: NO 1936-vanilla owner's land touches both x=0 and x=W-1 (the
   antimeridian seam is Pacific Ocean — verified), so a seam-unwrap is
   UNTRACED/deferred.
3. **Anchor = centroid** of that component. DERIVED-BY-FIT.
4. **Orientation = principal axis** (PCA of the component's covariance). The
   major eigenvector direction is the baseline angle, folded into
   `(-pi/2, pi/2]` so text reads left-to-right. This makes wide countries label
   horizontally and tall countries (Chile) vertically. DERIVED-BY-FIT.
5. **Length = extent along the major axis.** For a roughly-uniform strip the
   pixel span is `sqrt(12 * major_variance)`; the baseline half-length is
   `0.5 * that * LABEL_EXTENT_FRACTION` (0.62, so the text insets from the
   borders as the engine's do). DERIVED-BY-FIT.
6. **Curvature (bow).** A least-squares quadratic `v ≈ a0 + a2·s²` of the
   minor-axis offset `v` against the normalised major coordinate `s ∈ [-1,1]`,
   over the same component. The baseline is drawn as a quadratic arc whose peak
   offset is `-a2` (clamped to a fraction of the minor half-extent). This bows
   the label through the territory's mass — the "curve along it" behaviour.
   DERIVED-BY-FIT; the engine's exact spline/collision is UNKNOWN-ENGINE-SIDE.
7. **Size + arc fit.** The word is **uniformly scaled to span the baseline arc**
   (the game names stretch across their territory), so glyph size tracks country
   extent, capped so glyph height never exceeds `LABEL_WORLD_HEIGHT = 60` map px
   (further capped per country by half the minor-axis extent so glyphs stay
   inside thin countries). Crucially the fit is to the **arc length** of the
   bowed baseline, not the straight `2·half_length` chord: fitting to the chord
   over-fills a curved baseline and bunches/overlaps glyphs — the LAB2
   "Germany"→"Cenany" defect. Glyphs then advance by **uniform arc length**
   (`t_at_arc_fraction` inverts the arc-length integral) so spacing stays even
   where the curve steepens. When the height cap bites, the shorter word is
   centred on the arc. DERIVED-BY-FIT; the engine's exact size-vs-extent curve is
   UNKNOWN-ENGINE-SIDE.
8. **Visibility.** `DRAW_COUNTRY_NAMES_CUTOFF` (260) is applied as a hard cutoff
   against the production 3D camera height. The former 2D visible-map-height
   substitute and 25% fitted fade ramp are removed; if a production camera height
   is unavailable, labels fail closed instead of drawing from a proxy. Remaining
   transparency residual: the engine's per-label `Transp_OffsetX.x` write law is
   still unknown until a mapname constant-buffer capture pins it.
9. **Rendering** mirrors `mapname.shader` exactly. The `hoi_mapfont4.dds` atlas
   is **pre-coloured**: decoding the DDS shows each glyph is a full-colour,
   pre-outlined image — coverage in the ALPHA channel (interior `a≈0.5`, edge
   `a≈0.3`, background `a=0`) and a dark-body/light-rim outline baked into RGB
   (interior `rgb≈black`, edge `rgb≈white`). The `.fnt`'s `chnl=15` on every char
   (all channels used) and the shader sampling `DiffuseTexture` straight
   (`gfx/FX/mapname.shader:77`) confirm the glyphs carry their own colour. So the
   replay **passes the atlas RGBA through unchanged** (`AtlasMode::PreservedColor`)
   and draws with a white vertex tint so `Page.Sample * color` leaves RGB intact
   while the per-label transparency rides the alpha (`vSample.a *= Transp`), over
   the shader's `src_alpha`/`inv_src_alpha` blend, **Linear**-sampled to match the
   shader's `MagFilter/MinFilter/MipFilter = "Linear"`. This replaced the LAB2
   bug where the atlas was collapsed to a white coverage mask via a `255 - red`
   inversion (turning the transparent grey background into opaque **black boxes**
   and mangling glyph shapes) and then tinted flat black. Day/night dimming
   (`gfx/FX/mapname.shader:79`) and the exact `Transp_OffsetX.y` wrap offset are
   not reproduced in the 2D replay (UNKNOWN-ENGINE-SIDE for its fixed lighting).

Fail-closed: an empty/degenerate component or a component whose ported engine
segment does not resolve yields no label rather than a PCA/proxy guess; a
resolver miss on font/localisation/defines errors out naming the exact missing
piece.

## 4. What stays UNKNOWN-ENGINE-SIDE

(LAB3 update: several of these are now *bounded* by captured geometry — see §5.)

- The engine's exact baseline spline. A single quadratic bow reproduces the
  captured baselines to mean RMS 0.158 / max 1.14 world units (§5), so it is the
  right shape but not exact on the most-curved labels; whether the engine follows
  a border path is still open (§5.1).
- The exact size-vs-territory-extent curve (§5.1: two captured zooms cannot yet
  separate world- from screen-space sizing) and the exact inset fraction (0.62 is
  fit; the captured labels confirm text insets from the region but the raster to
  measure the fraction directly is not in the VB-only captures).
- Collision / de-overlap between neighbouring labels (the engine nudges labels
  apart; the replay does not).
- Per-glyph day/night dimming and the `Transp_OffsetX.y` wrap offset value.
- Whether the engine anchors on the centroid, a weighted centroid, or a
  pole-of-inaccessibility of the main body (the centroid can land in water or
  foreign territory for concave/crescent/coastal shapes; PCA can pick an axis
  that reads oddly).
- **Owner vs controller.** This law keys on legal state **owner**. During
  occupations the game may label by current **controller**/occupation state (its map
  fill does). In wartime the replay's owner-keyed labels will diverge; the
  `update_owners` hook already accepts whichever ownership map the caller feeds,
  so switching to controllers is a one-line caller change if a capture ever
  confirms the engine uses them.
- Whether raster 4-connectivity (with east-west wrap) matches the engine's
  province/state topology across one-pixel straits and isthmuses.

## 5. Validation against captured evidence — NOW MEASURED (LAB3)

The capture requested at the end of the old §5 landed. Two sessions dumped the
`mapname` pass's slot-0 vertex buffer, cb0 (`ViewProjectionMatrix` + `vCamPos` +
`vCamLookAtDir`), and cb1 (`Transp_OffsetX`):

- `session_CAPFIX2_GAPS_4` — ONE near-view label draw (66 verts, VB slot0
  1320 B, stride 20; VS `4e1b73f39c128df0`, PS `546c545ea0add178`).
- `session_MAPNAME_7B` — a far-zoom Middle-East view, **23 label draws**
  (24–120 verts = 4–20 glyphs), same VS/PS/layout.

**Decode is exact.** `map_viewer::map_label_capture` (new) + the
`decode_mapname_vb` bin decode every draw byte-for-byte:

- **VB layout (ground truth = reflected VS input signature `4e1b73f39c128df0`,
  bundle `input_layout.elements`):**

  | field    | semantic  | DXGI format          | offset | size | cite |
  |----------|-----------|----------------------|--------|------|------|
  | position | POSITION  | 6 = R32G32B32_FLOAT  | 0      | 12   | bundle `input_layout.elements[0]` / `mapname.shader:31` |
  | uv       | TEXCOORD0 | 16 = R32G32_FLOAT    | 12     | 8    | bundle `input_layout.elements[1]` / `mapname.shader:32` |

  stride 20 (= bundle `vertex_buffers[0].stride`); `6 * glyph_count` verts,
  TRIANGLELIST (topology 4), per-glyph corner order `TL,TR,BL,TR,BR,BL`.
- **Position is world-space map-plane.** Captured Y ≈ 10.0–10.24 (the map
  plane), X/Z are province-raster units. Projecting the VB through the captured
  cb0 `ViewProjectionMatrix` lands the label on-screen only when the 16 stored
  floats are **transposed for CPU `M·v`** (column-major read): row-major gives
  `w ≈ 163000` (absurd), transposed gives `w ≈ 300` — matching the view depth
  `vCamPos.y (310) − plane_y (10)` with `vCamLookAtDir ≈ (0,−0.995,0.099)`. So
  the shader's `mul(ViewProjectionMatrix, vPos)` (`mapname.shader:55`) confirms
  the VB holds pre-transformed world glyph quads and the shader does **no** arc,
  spacing, or placement math — all of it is baked CPU-side.
- **String recovery is unambiguous.** Each glyph's UV bbox (`u·2048, v·1024`)
  matches exactly one `hoi_mapfont4.fnt` char rect. Audited: the `.fnt` has
  **zero duplicate rects**, so the match is 1:1, not a glyph class. Recovered
  strings at **0.0 texel error**: GAPS_4 = `SOVIETUNION`; 7B = MANDATORY
  PALESTINE, BRITISH IRAQ ×2, UNITED KINGDOM ×8, IRAN, FRANCE ×2, SAUDI ARABIA,
  REGNO DEL SUD ×2, EMIRATE OF TRANSJORDAN, YEMEN, SULTANATE OF OMAN, ETHIOPIA,
  SHEIKHDOM OF KUWAIT. (~1943 save: Regno del Sud = post-armistice Italy.)

**What the engine geometry proves about the law:**

- **Arc model (§3.6) — CONFIRMED, quantified.** Fitting the placement law's own
  centroid-anchor + PCA-angle + quadratic-bow baseline to the engine's per-glyph
  baseline midpoints reproduces them to **mean RMS 0.158 world units across the
  23 7B labels (max residual 1.14)** and **RMS 0.035 (max 0.082)** on the
  held-out GAPS_4 `SOVIETUNION` at a different zoom. A single quadratic is the
  right baseline shape; the ≤1.1 residuals on the three most-curved labels
  (BRITISH IRAQ, FRANCE, UNITED KINGDOM #2) are where a higher-order spline would
  help — the "exact spline" gap (§4), now bounded, not open-ended. Regression
  test: `map_viewer/tests/mapname_capture_decode.rs`
  (`law_arc_model_reproduces_engine_baseline_within_tolerance`, tol RMS<0.6,
  MAX<1.2). Overlay PNGs (`decode_mapname_vb` `_before`/`_after`): the straight
  chord cuts across curved baselines; the quadratic arc hugs them.
- **Proportional advance (§3.7) — CONFIRMED as BMFont `xadvance·scale`.** On
  `SOVIETUNION`, per-letter world advances match `xadvance · scale`
  (scale ≈ 0.0751 world-units/font-px) to ≤0.4 world units; the largest residual
  is the `V→I` pair (negative kerning). So the law's `measure()`-driven advance
  is right in form; exact **kerning** is the residual UNTRACED term (§4).
- **Spaces advance (§3.7) — CONFIRMED by a gap test.** `SOVIET UNION` has no
  space quad in the VB (11 glyphs). The `T→U` world gap (5.05) matches
  `(T.xadvance 42 + space.xadvance 20)·scale = 4.66` far better than the
  no-space `3.15`, so the engine advances the pen over the space but emits no
  quad — exactly what the replay's `measure()` does.
- **Size is one uniform scale per label (§3.7) — CONFIRMED.** Glyph cap height is
  near-constant within a label (e.g. GAPS_4 4.0–4.18) and varies with region
  across labels (7B: 0.62 for a tiny UK holding … 66.95 for France).
- **One label per country is WRONG — REFUTED; per-component now SHIPPED
  (LABGRAD1).** UNITED KINGDOM draws **8 separate labels** in one 7B frame,
  FRANCE/REGNO DEL SUD/BRITISH IRAQ twice each. This killed the old §3.2 "keep
  only the home component" reduction, and §3.2 now labels every controlled
  component that resolves through the ported engine segment law. The count refutes
  one-per-country **decisively** but per-connected-component is **consistent with,
  not proven over,** per-state / per-strategic-region / component-with-threshold —
  all produce the same ≥2-labels-per-tag symptom. Settling which needs the
  capture-time owner/controller raster overlaid on each label's world bbox (still
  filed, §5.1). The LABGRAD1 acceptance gate is therefore STRUCTURAL: it checks
  camera reconstruction on all 23 draws, all-23-strings-resolvable, the
  count-refutation, and arc-model residuals on all 23 — it does NOT (cannot)
  prove the law's PCA predicts the 7B anchors from the 7B owner state, since that
  raster is absent (`map_viewer/tests/mapname_7b_acceptance.rs`).

### 5.1 What remains UNDERDETERMINED (and the capture that closes each)

- **Absolute size curve.** Two zooms (GAPS_4 near, 7B far) show world glyph
  height scales with region, but two zoom levels cannot separate a world-space
  extent law from a camera/screen-space size law, nor pick among major-extent /
  arc-fit / √area / minor-axis-cap forms. CLOSES WITH: the **same** component
  captured at ≥3 zooms (is world height zoom-invariant?), plus the province
  raster to correlate height with measured extent.
- **Per-component vs per-state/region labelling.** CLOSES WITH: the capture-time
  **owner AND controller** province rasters, to check "one label per eligible
  connected component, no unexplained components" against the 8 UK draws.
- **Owner vs controller keying.** The 7B save is wartime-ish; whether the engine
  keyed these labels on owner or controller is untested. CLOSES WITH: the
  controller raster (above) — do occupied-territory labels follow the occupier?
- **Exact anchor rule.** The engine baseline centroid is reproduced by the law's
  centroid to the fit residual, but on ONE component per label; whether the
  engine uses centroid vs weighted-centroid vs pole-of-inaccessibility is not
  separable here. CLOSES WITH: labels on concave/crescent components where the
  three anchors diverge > the fit residual.
- **Kerning.** The ≤0.4-unit advance residuals are consistent with `.fnt`
  kerning pairs; not yet fit. CLOSES WITH: solving `advance = xadvance·scale +
  kern(a,b)·scale` over all captured pairs.
- **Curvature source.** A quadratic fits to ≤1.14 units, but whether the engine's
  baseline is a border-following path (that a quadratic merely approximates) is
  open. CLOSES WITH: a strongly concave country (e.g. Norway/Chile) captured and
  its residual compared to the quadratic bound.

The arc/advance/size machinery (§3.4–§3.7) is validated as-is by LAB3 and is
UNCHANGED by LABGRAD1. LABGRAD1 changed only §3.2 (per-component instead of
home-only). As of 2026-07-09 the replay-app render flag is blocked because
remaining size/anchor/component-emission gaps are research-only.

## 6. Eyeball validation (1936 vanilla start)

The validation bin `dump_map_labels`
(`replay_interface/native/map_viewer/src/bin/dump_map_labels.rs`) renders the
reconstructed layout over a land/water outline and writes `map_label_layout.json`
(tag, name, anchor, angle_deg, half_length, bow, world_height, pixel_count,
baseline polyline). `--home-only` reproduces the pre-LABGRAD1 one-label-per-country
law for a before/after comparison. 1936 vanilla:

- **home-only (before):** 89 labels across 89 countries.
- **per-component no-threshold diagnostic (2026-07-09):** **1238 labels across
  90 countries** from `dump_map_labels` against the current vanilla bundle. This
  is development evidence, not product output: after removing the non-engine
  de-clutter fraction and 16-cell floor, the unresolved engine emission/de-
  overlap law is visible as label spray, so `replay_app` blocks `--map-labels`.

Spot-check of the three majors from that diagnostic dump (home components,
map-pixel space, `h` = diagnostic world height):

| tag | anchor (x,y)   | angle°  | half_len | bow  | px    |
|-----|----------------|---------|----------|------|-------|
| GER | (2982.0,505.0) | -13.3   | 78.1     | 0.0 | 17933 |
| SOV | (4396.0,291.0) | -7.3    | 1171.5   | 0.0 | 1167009 |
| USA | (1249.5,505.5) | 5.0     | 382.0    | 0.0 | 238982 |

GER sits on the 1936 Reich (incl. East Prussia, hence the NE-SW tilt); SOV runs
near-horizontally across contiguous Eurasia; USA's HOME label anchors on the
contiguous lower-48, and per-component now ALSO labels Alaska/Hawaii/Pacific
holdings. Before/after schematic dumps (baseline arcs + anchor crosses over a
land outline, full / Europe / UK-empire views) are in
`map_viewer/out/labgrad1_dumps/{before,after}/`.

## 7. Codex adversarial reviews (relayed)

- **Game-file sweep** (model_reasoning_effort=xhigh): independently grepped the
  game root and returned *"Found no shipped file carrying per-country/per-state
  country-name label position, angle, rotation, curve, or size … I agree the
  claim holds for this the game install."* No contradicting file named. Ruled out
  the same false leads (animatedmaptext, ambient_object rotations, arrow text,
  commented `#force_rotation` doctrine tech).
- **Capture-evidence audit** (model_reasoning_effort=xhigh): agreed there is NO
  captured mapname label geometry; the only "mapname" strings in the 8.6 GB
  `wedge_full.dmp` are effect-name/console-help/resource-path strings, not draw
  payloads.
- **Placement-law self-refutation** (model_reasoning_effort=xhigh, per the LAB1
  spec, exit 0 / non-empty): searched 8,417 shipped text-like files + 1,523
  targeted `history/states` / `common/countries` / `country_metadata` files and
  returned **contradicting-file = NO** — no shipped file carries a
  per-country/per-state map-label position, angle, rotation, curve, or size.
  Corrections it raised, all folded in above: `country_metadata/…` is the right
  path (not `common/country_metadata`); UI `country_name` textboxes have
  positions but are panel widgets, not map labels; the font claim is
  Latin-default only (JP/CN/KR overrides exist); `mapname.shader` does not tint
  glyphs black (default `0xffffffff`, the black tint is a replay choice); and
  `DRAW_COUNTRY_NAMES_CUTOFF` only says "cutoff for drawing", so the old fade
  ramp was removed rather than treated as shipped law. Its top law objections
  (owner-vs-controller,
  connected-component fragility, centroid/PCA weakness for concave shapes,
  single-quadratic-too-simple, ungrounded constants, raster topology) are
  recorded in §4 as the known UNKNOWN-ENGINE-SIDE gaps.
- **LABGRAD1 per-component + graduation self-refutation**
  (model_reasoning_effort=xhigh, read-only, exit 0 / non-empty): attacked the
  merge rule and the size floor. Verdicts: **(A) merge rule ACCEPT-WITH-CHANGES**
  — per-component is the leading but unproven hypothesis, so demote "proof"
  language (done: §5 + the acceptance test now say REFUTES-one-per-country,
  consistent-with per-component); it also flagged the seam-crossing PCA (verified
  benign — no 1936 land wraps the antimeridian; documented in §3.2 + the flood
  fill) and the owner-vs-controller divergence for wartime label rendering (kept
  owner-keyed per project rules — switching needs capture evidence, §4). **(B)
  size floor REJECT** — 16 cells is a PCA-viability floor, not a de-clutter gate,
  and spraying every viable cloud (FRA=10, tiny no-VP enclaves) is not defensible
  as a user-facing default. A later attempted non-home de-clutter filter
  (`NONHOME_MIN_AREA_FRACTION`, plus a VP survivor rule) has now been removed
  because it was not engine-derived. It also confirmed the STRUCTURAL acceptance
  gate is honestly named only if treated as a decode/arc-model + refutation gate,
  not a merge/owner/size-floor proof — the test header and §5 now say exactly
  that.

## 8. NAME2 unified SIZE law (frame4700 RenderDoc oracle, 2026-07-07)

Ground truth: the 68 engine `mapname` label vertex buffers of
`hoi4_vanilla_frame4700` (vanilla **1936 save**, political mapmode, camera
h=905 over Europe), decoded per-glyph against the shipped
`hoi_mapfont4.fnt` (uv-rect matching; CAP_PX = 55, the modal A–Z bitmap
height and the same metric `font.rs cap_height()` divides by). 38 of them
join 1:1 to our 1936 bundle placements (nearest anchor ≤ max(25, 0.6·hl),
identical squashed text, arc ratio 0.4–2.5, angle ≤ 35°) — fixture
`map_viewer/tests/fixtures/mapname/frame4700_labels.json`.

**The law.** Per label the engine draws at

```
scale = min( arc_length / natural_text_width_px ,   # fill the arc
             K_breadth × extent_mask_minor / CAP_PX )  # breadth cap
```

with additive letterspacing (the NAME2 tracking law, §5) justifying the text
to the arc whenever the breadth cap is the smaller term. Constants
(ENGINE-FITTED):

- `LABEL_EXTENT_FRACTION = 0.75` — measured engine arc / our component major
  extent: median 0.752, IQR 0.679–0.800, n=38. (The old 0.62 was
  screenshot-eyeballed and left every fill-regime label ~20% small.)
- `LABEL_HEIGHT_BREADTH_K = 0.22` — over the 8 unambiguously capped labels
  (relative extra advance > 10% AND drawn < 0.8× their own fill size):
  SWEDEN 0.221, TURKEY 0.253, ITALY-boot 0.192, LATVIA 0.214, AUSTRIA 0.192,
  ITALY-Sardinia 0.243, FRANCE-Corsica 0.251, SWEDEN-Gotland 0.196 → median
  0.218, ±14% spread; pinned 0.22 (third decimal unresolved).
- **No absolute ceiling**: SOVIET UNION draws at cap 131.9 world px,
  refuting the old 65 clamp (which had been read off the 7B ongoing-save
  capture's largest label — a sampling artifact, not a law).

**Split masks.** The breadth is measured on the EXTENT mask = owned
territory INCLUDING owner-tagged impassable (wasteland) states; placement
(anchor/angle/arc/de-clutter/home) stays on the passable cloud. Evidence
both ways from one label: the engine's ITALY over Libya draws h=45.5 at
fill — the passable coastal strip alone (minor ≈ 61) would cap it at ~13,
while `history/states/273-Italian Africa.txt` (impassable, owner=ITA)
widens the extent mask to ≈ 181 (cap ≈ 40, not binding) — yet the same
label's ARC (138 world px) matches the coastal strip's extent, not the
full desert blob's. An owner with ONLY impassable territory still fails
closed.

**Fit quality** (38 pairs, h_pred vs engine cap height): 89% within ±25%,
97% within ±40%, median ratio 0.988. Gate:
`map_viewer/tests/frame4700_label_size_gate.rs` (skips without the local
bundle) asserts ≥78% within ±25%, median in [0.90, 1.10], every strongly
capped row's cap term within [0.7, 1.4]×, and ≥36/38 joins.

**Known residual — IRAN.** The engine fills Iran at 44.7; every breadth
proxy we tested (eigen minor 164, inclusive eigen 164 — Iran has no
impassable states — full perpendicular span 207, 2–98% span ~184) caps it
15–25% lower. Iran lands at ratio 0.81, just inside the ±25% band, and is
NOT special-cased. The engine's exact breadth metric stays
UNKNOWN-ENGINE-SIDE (Codex review suggested support-width/hull variants;
none tested separates Iran from the capped cluster).

**Rejected alternatives** (data on file, `mined/name2_units.json`):
label-collision shrink (FRANCE fills at h/d_nearest = 0.955 while SWEDEN
capped at 0.30); mean-chord breadth (the pre-NAME2 law — DEN/FIN violate);
any monotone f(major, minor) without the extent mask (Libya is smaller than
Sweden in BOTH placement axes yet draws bigger).

Codex adversarial review (xhigh, 2026-07-07): confirmed the mechanism,
demanded the split masks (adopted), the breadth-metric comparison (done —
eigen minor won; span/percentile-span fit no better and violate more), and
the 0.22 pin over 0.218 (adopted).

## 9. NAME2b/2c rendering + angle-law verification (2026-07-08)

Follow-ups after the user reported the titles "look almost too big". All
three were engine-measured against the frame4700 referee (the world-unit
sizes from §8 were already correct; the *look* was off):

- **World-space projected glyphs (NAME2b).** The engine builds `mapname`
  glyph quads in WORLD space, flat on the map plane (LAB3: the captured
  TL→BL cap edge lies in the XZ plane), and the 3D view-projection scales
  them; the replay now lays out each glyph corner in world units and
  projects it per-corner (`text.rs draw_text_curved_projected`). The old
  path scaled screen quads by the 2D-zoom stand-in × (target/app width),
  which double-counted and could not express tilt foreshortening.
- **UNORM atlas, not sRGB (NAME2b).** The engine binds `hoi_mapfont4` as
  DXGI fmt 87 = `B8G8R8A8_UNORM` (frame4700 SRV, tex 322). LABHDR1's
  first pass used an `_SRGB` view, linearising the glyph bodies ~2.2-gamma
  darker = the heavy/bold "too big" read. Reverted to UNORM; raw HDR glyph
  values now near-engine (ROMANIA body 0.026 vs 0.033, outline 0.456 vs
  0.467).
- **Shipped mip chain + −0.4 LOD bias (NAME2c).** `gfx/FX/mapname.shader`
  Samplers declare trilinear + `MipMapLodBias = -0.4`, and the font DDS
  ships a 4-level mip chain; uploading mip 0 alone rendered mid-zoom titles
  too sharp. The replay now uploads the SHIPPED mip bytes
  (`dds::decode_mip_chain_bytes`, `FontPage::mips`) and biases the sampler
  −0.4.
- **Label world size is STATIC across zoom.** The `mapname` VBs are the
  SAME immutable resources at every captured ladder height (SOVIETUNION's
  VB recurs at h=3000/1359/905); draw-count varies only by frustum
  culling. So the giant SOVIET / Sahara-spanning FRANCE are authentic, and
  the 7B IRAN 32.8-vs-44.7 discrepancy was a different-save artifact, not a
  zoom law. Engine `Transp` = 1.0 at every ladder height ≥ 288.

**Angle law — PCA confirmed optimal, C2 closed.** A 38-pair angle sweep
(engine baseline direction vs candidate laws) shows the existing PCA
principal axis is the best files-derivable orientation: median 4.0°, mean
6.7°, p75 9.1°. Every alternative is worse — max-extent direction 9.8°,
2–98 percentile extent 7.7°, VP-weighted PCA 6.8°; largest-sub-blob PCA is
identical (no scattered sub-blob in the set). The earlier "FRANCE 57° off"
was a stale scrambled join; the true metropole FRANCE matches (eng −84.2°
vs PCA −85.3°). The residual is a thin tail — worst Greece 29.5° (aspect
1.52; the Aegean coast pulls PCA off the N–S mainland/Peloponnese) — that
no tested law corrects without regressing the 34 good cases. Kept PCA; no
code change. (`scratchpad/c2_sweep.json`, `mined/c2_angle_cases.png`.)

## 10. NAME2 letterspacing ceiling (C3, 2026-07-08)

The engine justifies a name across its baseline with additive tracking (§5),
but bounded: across the 68 decoded frame4700 labels no SUBSTANTIAL one
(cap ≥ 5 world units) letterspaces beyond ~2.0× its natural width — ITALY-boot
= 2.00 exactly, LATVIA 1.63, AUSTRIA 1.61, SWEDEN 1.51 (only sub-1-wu island
fragments spread further, and those are near-invisible). Our placement
baseline spans 0.75× the whole passable-territory major axis, so a
hyper-elongated country stretches its word to fill that: CHILE 8.1×, CUBA
4.9×, NORWAY 2.5× — the "N O R W A Y" eyesore.

The `MAX_LETTERSPACE_STRETCH = 2.0` law confines the word to the central
`2.0 × natural` span of the baseline (still centred on the anchor), so
over-elongated names render compact. The cap is the tightest value that does
NOT clip an engine-matched label (ITALY-boot sits exactly at 2.00). It fully
fixes the extreme cases and is safe (the size gate is unchanged — this is a
renderer-side justification bound, not a size-law change).

**NORWAY is only partially fixed** (2.53× → 2.0×, 21% shorter). Its dominant
error is not letterspacing but the ANCHOR: the engine draws NORWAY compact
over the populated SOUTH (decoded anchor y≈279, arc≈89, cap 18.2) while our
PCA centroid over the whole passable cloud — including the 1000 km northern
coast — sits at y≈194 with a 515-unit major axis.

**RESOLVED by NAME3 (§11), 2026-07-08.** The anchor law was NOT VP-weighting —
it is the DISTANCE-FIELD CORE (weight cells by `max(D − 0.20·D99, 0)²`), which
lands NORWAY's anchor ~9 px from the engine from geometry alone, no capture
needed. The letterspacing bound below stays the ARC fix (the arc length for
hyper-elongated shapes is the one Norway residual that remains — see §11).

### 10.1 Codex adversarial review + follow-up tests (2026-07-08)

Codex (xhigh, read-only) reviewed the letterspacing cap and Norway anchor.
Two of its suggested tests were run:

- **Stretch-rule vs tracking-rule (Codex q-b).** Normalised additive tracking
  per gap (`(arc − natural)/(chars−1)/cap`) was compared against the stretch
  ratio over the 30 substantial (cap ≥ 5) labels. The STRETCH ceiling is the
  tighter invariant (coefficient of variation 0.11 across the 5 letterspaced
  labels vs 0.26 for tracking-em; ITALY-boot caps both). So
  `MAX_LETTERSPACE_STRETCH = 2.0` is confirmed the better rule — a max-tracking
  law fits worse. The cap is filtered to "visible" labels (cap ≥ 5); sub-1-wu
  island fragments are a separate degenerate/near-invisible regime and are
  excluded (logged, not hidden).

- **VP-weighted anchor for Norway (Codex q-c).** Falsifiable residual test:
  does the VP-weighted province centroid `V` explain the engine anchor `E`
  better than the plain centroid `A`? A GLOBAL VP-weight regresses the general
  case (median anchor error 7.5 → 21 px — it pulls compact countries toward
  their capital province, e.g. France toward Paris). An ASPECT-GATED blend
  `alpha = smoothstep(3.5, 5.0, aspect)`, `anchor = lerp(A, V, alpha)`, moves
  Norway strongly south (error 122 → 53 px vs the directly-decoded engine
  anchor (2946, 279)) and leaves median/p90 unchanged — BUT Czechoslovakia
  (aspect 4.24) regresses ~9 px, so the only safe gate (≈4.5–5.0) is fit to
  sit between CZE and Norway, i.e. constrained by a SINGLE example above it.
  Per "never fit a law from one example," this is NOT shipped. It is the
  sharpened hypothesis: the engine likely weights the anchor toward population
  for hyper-elongated countries; confirming the gate boundary and the blend
  curve needs a capture with ≥3 such countries (Chile, Vietnam, Norway,
  Sweden) in one frame. Until then the §10 letterspacing cap is the shipped
  partial fix and the anchor stays UNTRACED. (`scratchpad/c2_sweep.json`,
  VP-blend test inline.)

Codex also confirmed: (a) keep the cap in text-fitting (the renderer uses the
capped central span for glyph vertices; the full baseline only supplies the
bow the word follows and the anchor cull — verified correct); (d) central
confinement is right (engine labels centre on their anchor, not left-justify).

## 11. NAME3 distance-weighted core ANCHOR (2026-07-08) — the Norway fix

§10 left the Norway anchor UNTRACED, guessing "VP-weighted anchor?" and parking
it on a capture. NAME3 closes it **from geometry alone** — no capture, no
population data, no per-country tuning.

**Mechanism (Codex-corroborated).** The engine anchors a label on the country's
DISTANCE-FIELD CORE, not the centroid of its cell cloud. Weight each sampled
owned cell by `w = max(D − 0.20·D99, 0)²`, where `D` is the cell's chamfer
distance to the country border (`OwnerGrid::core_distance`) and `D99` the
99th-percentile `D` over the component; the anchor is the `w`-weighted centroid.
In a limb of local half-width `r` each slice's weight scales as ≈`r³`, so a thin
appendage (Norway's 1000 km northern coast, the Aegean island fringe) is
cubically suppressed while a compact country's whole interior clears the
threshold and its anchor barely moves. This is the "visual-centre / distance
core" family (distance transform, medial axis, polylabel pole-of-inaccessibility
— Codex research); the *weighted-moments* form is the right member — a full
medial axis is overkill, and `k ≥ 4` degenerates toward "pick the single fattest
point" (the pole), which over-shoots stringy countries and mis-places
uniformly-wide ones (SWEDEN).

**Result (live-rendered, `live_PAN2`, Europe).** NORWAY's anchor moves from the
raw centroid at y≈194 (122 px NORTH of the engine, over the empty north) onto
the populated south at ~9 px from the engine's decoded (2945.7, 279.4). Gated by
`frame4700_engine_anchors_reproduce` (fixture `frame4700_placement_gate.json`,
the engine primary-label anchor per tag via the confirmed `(x, 2048 − z)`
map-space transform): NORWAY ≤ 30 px, median engine-anchor distance ≤ 40 px over
19 tags.

**Scope — ONLY the anchor.** The ANGLE and EXTENT stay on the uniform cloud.
Weighting the angle rounds near-isotropic countries and flips them: measured on
the raster, SPAIN 8°→59° and GERMANY 6°→23° under `D²` weighting (their cores are
near-circular; the principal axis becomes noise). The uniform PCA angle is
retained unchanged — the NAME2 size law is gated against it and it is correct for
all compact fits. This decoupling is the whole discipline of the fix: take the
weighting only where it helps (anchor) and refuse it where it harms (angle).

**Two documented residuals (NOT overfit).**
- **GREECE angle ~29°.** The engine draws GREECE down its N–S mainland
  (−86.5°); our uniform PCA is pulled ~29° east because the Aegean islands +
  the Thrace/Macedonia bulge balance the N–S body (aspect only 2.3 — a genuinely
  low-anisotropy shape). Every weighting that fixes it (`D²` → 5.9°, erosion-2 →
  1.4°) demonstrably breaks the near-round countries (SPAIN, GERMANY). It is a
  files-underdetermined case at the stride-4 working resolution (the islands
  bridge into the mainland); finer-resolution component separation OR a capture
  disambiguation would close it. Left as a bounded residual rather than a
  regression.
- **Hyper-elongated ARC length.** With the anchor fixed, NORWAY still
  letterspaces across most of the peninsula (the arc is the uniform 0.75× major
  extent; §10 cap confines it to 2.0×natural but that still spans a long
  country). The engine draws it more compact (arc ≈ 90). `engine_arc /
  weighted_extent` does NOT tighten the fit (CV 0.40 vs 0.38 uniform; it
  over-shrinks compact countries — AUSTRIA 1.86, SAUDI 1.58), so the arc law is
  NOT switched to the weighted extent — that would regress the gated size law.
  Visually the result is consistent with every other country label (all spread
  across their territory) and reads as a proper map; the extra engine
  compactness on hyper-elongated shapes is a minor stylistic delta. A
  width-clipped arc (clip to where the country is wider than the label height)
  is the likely closer, and pairs with the same B-series multi-elongated capture.

Constant: `map_label_law.rs LABEL_CORE_WEIGHT_THRESHOLD = 0.20`. Mod-generic (a
pure function of the provided owner raster; no country/dimension hardcoded).

**Codex adversarial review (xhigh, read-only, 2026-07-08).** All six correctness
checks passed: border seeding, no cross-country distance bleed (every chamfer
relax gates `comp[n]==comp`), no `+inf` escape, D99 nearest-rank index safe for
len 1/100, no NaN / no lost label (division only when `sum_w>0`, else uniform
fallback), the uniform moment paths byte-identical (`add` delegates to
`add_weighted(...,1.0)`; the extent mask still calls `add`), and no hardcoded
country/dimension. It found one real defect — the **bow** was fitted about the
uniform mean but drawn about the core anchor, so a country with a large
perpendicular anchor shift (measured: SOV 122 px, CAN 76, USA 32) could have its
bowed baseline translated off-territory; FIXED by fitting the bow about the core
anchor. Two inherited limitations, both vanilla-safe and mod-edge only: (i)
**seam-wrapped land** — a modded country owning strips at both x≈0 and x≈W merges
through the east-west wrap and gets a mid-map anchor + huge extent; this is the
SAME raw-`gx` seam limitation the PCA law already carries and documents (§3.2 /
the flood-fill note), inherited by the core anchor, and no 1936 land touches both
seams; (ii) the two-pass chamfer is not exact across that same wrap (a
seam-crossing shortest path can read `1+√2` instead of `2`) — dominated by (i)
and moot until seam-unwrap is done. Both close together with a pre-PCA
seam-unwrap when a mod exercises them.

Baselines re-locked (post bow-fix, 2× deterministic): world_v22
`fdaf347e9f14abd3f5bdf9d43b6c238e`, europe_v14
`9bf40ec51b02eca8f1989c667a7344c5`.
