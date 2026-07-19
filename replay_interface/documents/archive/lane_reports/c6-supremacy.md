# Lane C6 — Sea-zone naval-supremacy tint

**Branch:** `lane/c6-supremacy` · **Plan item:** C6 (REPLAY_PRODUCTION_PLAN.md)
**Module:** `replay_interface/native/map_viewer/src/naval_supremacy_tint.rs`
**Status:** data law implemented + gated; final pixel parity capture-gated (deferred).

The naval / strategic-navy map mode colours each sea zone by the side holding
naval supremacy. This lane owns the replay-side *data law*: sea-zone geometry
(the GB-field key) + supremacy → status → tint. Colours are computed replay-side
from inputs, never recorded (standing override).

## 1. Law table (every parameter labelled)

| Parameter | Value | Label | Citation |
|---|---|---|---|
| Sea province test | `definition.csv` type == `sea` (lakes excluded — inland, not naval) | FILE | `map/definition.csv` col 5 |
| Sea-zone membership | strategic region with ≥1 sea province | FILE | `map/strategicregions/*.txt` `provinces={…}` |
| Field geometry (GB fill key) | the zone's **sea** province set (interior island land excluded) | FILE (derived join) | strategicregions ∩ definition.csv; carrier = GB fill (`POLITICAL_COLOR_LAW.md`, `CGradientBorderManager`/`CGradientBorder::CFillBufferThreaded`) |
| Tint fill RGB | controlling country's map colour (incl. cosmetic-tag overrides) | FILE | `common/country_tags`, `common/countries/colors.txt`, `common/countries/cosmetic.txt` (existing `load_country_colors*`) |
| Tint alpha min | `NAVAL_REGION_TRANSPARENCY_RATIO_MIN` = 0.5 | FILE | `common/defines/00_graphics.lua:58` |
| Tint alpha max | `NAVAL_REGION_TRANSPARENCY_RATIO_MAX` = 0.8 | FILE | `common/defines/00_graphics.lua:57` |
| Dominance range | `NAVAL_REGION_TRANSPARENCY_RANGE` = 2000.0 | FILE | `common/defines/00_graphics.lua:59` |
| Alpha curve = `lerp(min,max, clamp(sup/range,0,1))` | linear, dominance 0→min / ≥range→max | **DERIVED** (from define comments; exact shape capture-gated) | interpretation of the three defines above |
| POV stripe: enemy | `STRATEGIC_MODE_ENEMY_STRIPES_COLOR` = {0.827,0.172,0.184,0.0} | FILE (E6-gated) | `00_graphics.lua:37` |
| POV stripe: own | `STRATEGIC_MODE_OUR_STRIPES_COLOR` = {0.427,0.619,0.858,0.0} | FILE (E6-gated) | `00_graphics.lua:38` |
| POV stripe: ally | `STRATEGIC_MODE_ALLY_STRIPES_COLOR` = {0.427,0.619,0.858,0.0} | FILE (E6-gated) | `00_graphics.lua:39` |
| Country-colour stripe transp | `STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP` = 0.0 | FILE | `00_graphics.lua:36` |
| No supremacy data | open ocean (no tint) | FILE (degrade) | `REPLAY_DATA_REQUIREMENTS.md` §5 ("no sea lines with no naval data") |

**Note on the POV stripes:** all three `STRATEGIC_MODE_*_STRIPES_COLOR` ship with
alpha 0.0 (disabled) in vanilla; they are a POV-relative overlay for observer
country-POV mode (E6), not the omniscient-observer tint. Exposed via
`pov_stripe_color` / `PovNavalRelation` but **not applied** by the default tint.
The friendly/enemy/contested tri-state is derived replay-side from `controller`
+ diplomacy, never recorded.

## 2. Gate numbers

Build: `cargo build -p map_viewer` — clean (pre-existing warnings only).
Tests: **`cargo test -p map_viewer` = 637 passed / 0 failed** (was 631; +6 new).
`cargo test -p replay_core` = **13 passed / 0 failed** (untouched — no edits).
`rustfmt --check` on the new file: clean.

New tests (`naval_supremacy_tint::tests`, 6):

| Test | Asserts |
|---|---|
| `defines_parse_matches_vanilla_constant` | `00_graphics.lua` snippet parse == `VANILLA_NAVAL_TINT_DEFINES` |
| `classify_splits_sea_land_and_islands` | sea/land split; island land excluded from field geometry; counts |
| `supremacy_alpha_law` | alpha = 0.5 @ 0, 0.65 @ 1000, 0.8 @ ≥2000 (clamped) |
| `fail_closed_open_ocean` | no controller / no supremacy / unknown-colour controller → no fill |
| `fx_naval_air_activity_zone_statuses` | fixture-driven (§3): ENG→GER flip at tick 5, alpha & fill per tick; absent region → open ocean |
| `gate_vanilla_naval_region_parse` | vanilla file counts (§4); region 301 not naval; shipped defines parse |

**Vanilla parse gate (real files, `BATTLE_PLAN_GAME_DIR` set):**
304 strategic regions · **98 naval regions** (≥1 sea province) · 57 all-sea ·
**3133 sea provinces** covered (0 orphaned). Gate skips (does not fail) when
`BATTLE_PLAN_GAME_DIR` is unset, per repo convention. Deterministic `_from_source` /
`_from_parts` tests run unconditionally.

## 3. Fixture derivation (`fx_naval_air_activity`)

Expectations derived from the generator source
(`replay_fixture_gen/src/main.rs::fx_naval_air_activity`, SNAPSHOT_COUNT=12),
**not** by running the code under test:

```
region_control["301"] = [{controller, tri_state, supremacy}]
controller = "ENG" if tick < 5 else "GER"
supremacy  = 0.55 + tick*0.02      (ticks 0..=11)
```

Fed as `ZoneSupremacyInput{controller, supremacy}` into `zone_tint`; asserted
status `Controlled{controller, supremacy}`, fill RGB = controller colour,
alpha = the `NAVAL_REGION_TRANSPARENCY_*` law. The controller flip ENG→GER at
tick 5 is verified. A naval region with no input degrades to open ocean.

## 4. replay_core API gaps found (coded fail-closed around)

1. **`tri_state` not read.** The fixture emits `region_control[*].tri_state`,
   but `replay_core`'s `build_naval_supremacy` reads `state`/`status`
   (`world.rs:2151-2155`). So `NavalSupremacyRow.state` is `Unobtained` for this
   fixture. **Not needed** by this lane — the tint uses `controller` + `supremacy`
   (both read correctly). The tri-state is POV/E6, derived replay-side.
2. **Decoupling, deliberate.** `map_viewer` has no `replay_core` dependency (its
   Cargo.toml is unchanged), so the tint consumes a neutral `ZoneSupremacyInput`
   that structurally mirrors `NavalSupremacyRow` (`controller`→`Option`,
   `supremacy`→`Option`; leaf-present = `Some`). The live adapter
   (`NavalSupremacyRow` → `ZoneSupremacyInput`) belongs at the `replay_app`
   integration seam. This keeps the lane off the crate the parallel lane edits
   and avoids build coupling.
3. **Supremacy magnitude scale is ambiguous.** Fixture uses normalised 0..1;
   `RANGE=2000` implies raw dominance points. The alpha law is applied uniformly
   (deterministic either way) but the *engine-correct* curve needs D56's unit
   pinned — logged in `REPLAY_DATA_REQUIREMENTS.md` §5 and `EXTRACTION_NEEDS.md`
   A6 (contract clarification, not a new field).

## 5. Capture ask (final pixel parity — deferred, capture-gated)

C6's "parity vs a captured supremacy frame" needs a **strategic-navy map-mode
frame** with:

- A player controlling several sea zones at **known supremacy magnitudes**
  spanning low→high (so the alpha curve min→max is exercised, not just the ≈min
  floor the fixture values produce), plus at least one **contested/enemy** zone.
- The captured **GB-field / sea-zone fill SRV or render target** for that frame
  (the texture the tint is painted into), so fill-RGB and per-zone alpha can be
  diffed against `zone_tint` output — analogous to the `ProvinceSecondaryColorMap`
  readback in `POLITICAL_COLOR_LAW.md`.
- The frame's **camera height** (alpha/visibility interacts with
  `NAVAL_REGION_TRANSPARENCY_*` and the `NAVAL_DOMINANCE_TREND_BAR_HIDE_RATIO`
  zoom gate) and the controlling tags (to resolve fill colours).

This confirms: (a) the linear alpha shape/direction (currently DERIVED), and
(b) the supremacy magnitude unit (§4.3). Until then the product path stays
fail-closed on the DERIVED alpha curve.

## 6. What is NOT in scope / next

- Pixel rasterisation into the live GB field + parity (capture-gated, above).
- E6 POV stripe application (needs diplomacy relations + viewed-tag — Phase E6).
- No new `REPLAY_DATA_REQUIREMENTS` field added (supremacy = existing item 5);
  only the A6/§5 semantics were sharpened.
