//! FURN1 — the in-render map-furniture composite: city/victory-point name
//! labels, VP dots, and capital stars drawn over the map, **default ON** (the
//! vanilla game always renders this layer; `--no-map-furniture` opts out and
//! that path is byte-identical to the pre-FURN1 output because no furniture
//! code runs). This is the composite half of MAPICON1: the placement/tier/
//! cutoff LAW lives in `map_viewer::map_furniture` (every value file-cited);
//! this module only projects the law's markers into the frame.
//!
//! # What draws, from where (all resolver-routed shipped files)
//!
//! * Marker set + anchors: [`map_viewer::compute_furniture_markers`] over the
//!   bundle's province raster + `history/states` victory points +
//!   `history/countries` capitals + `VICTORY_POINTS_<id>` localisation.
//!   Owners come from the RECORDING (per hour), so occupied/annexed states
//!   re-attribute their markers on scrub exactly like the political fill.
//! * Icon: `GFX_onmap_victorypoints_strip` (`interface/mapicons.gfx:290-295`,
//!   20 frames of 29x27). Frame selection is the measured 4-shape x 5-color
//!   grid law [`map_viewer::vp_strip_frame_index`]; the replay renders the
//!   neutral GRAY column (the engine's diplomatic-relation -> color-column map
//!   is UNTRACED engine C++, and an observer replay has no relation).
//! * Text: the MAP font `hoi_mapfont4` (`tahoma_60`) at scale 0.40, white
//!   body + dark rim — **ENGINE-MEASURED** (see below), NOT the `hoi_20bs`
//!   the `.gui` names. The oracle screenshots (2026-07-07, vanilla solo
//!   instance, gui_scale=1.0, `w1check\furn_oracle_{far,test_out}.png`)
//!   refute the naive `hoi_20bs` reading: on-screen "Melbourne" is 149 px
//!   and "Adelaide" 119 px wide, where `hoi_20bs.fnt` advances sum to 77/62
//!   (no uniform scale fits both) while `hoi_mapfont4.fnt` sums 370/298 —
//!   x0.4027 / x0.3993, an exact two-word match at scale 0.40. Glyph ink
//!   confirms it: the oracle 'M' is 22 px wide == mapfont4's 56 x 0.4, and
//!   the merged "Me" ink cluster reproduces from mapfont4 advances. The
//!   palette is the INVERSE of the country-label atlas (white body + dark
//!   rim vs the labels' dark body + light rim), so the composite inverts the
//!   atlas RGB (alpha untouched); the GUI font color (white,
//!   `interface/core.gfx:1237`) rides the vertex tint unchanged. The engine's
//!   actual font-substitution/scale/palette pipeline is engine C++
//!   (UNTRACED); these constants are measured, cited, and re-checkable.
//! * Zoom gating: `VpIconDefines::{dot_visible, text_visible}` — the shipped
//!   `00_graphics.lua:947-963` cutoffs. The camera-distance stand-in is the
//!   visible world height (`screen_h / zoom`), the SAME documented 2D-camera
//!   approximation the country-label cutoff uses (`map_labels.rs`).
//!
//! # Anchor pivot + text offsets (ENGINE-MEASURED)
//!
//! `interface/mapicons.gui:628-652` gives the container's INTERNAL layout
//! (icon at `x=-5 y=0`, text at `x=26 y=5`) but where the engine pins the
//! container relative to the projected province anchor — and how it actually
//! lays the text out — is engine C++. Measured from the same oracle
//! screenshots (two cities, two zoom levels; the text is zoom-INVARIANT,
//! confirming screen-space GUI layout):
//! * the ICON (29x27 strip frame, 1:1 pixels — the authored 18 px disk
//!   measures ~18-19 px on screen) sits centred on the projected province
//!   position;
//! * the text PEN starts `+37.3` px right of the icon centre (36.3 / 38.3
//!   on the two cities);
//! * the text LINE-MID sits `+7.5` px below the icon centre (cap-top
//!   anchoring: oracle 'M' top = icon centre - 2.5, M cap = 55 x 0.4 = 22).
//! The shipped `(26,5)` textbox offset does NOT reproduce the on-screen
//! layout under any container pivot (it predicts a 16.5 px pen), so the
//! measured offsets stand in until the GUI text draw is ever captured.

use std::collections::BTreeMap;

use map_viewer::{
    compute_furniture_markers, furniture_static_data_from_resolver, heightmap_grid_from_resolver,
    load_vp_strip_rgba, vp_strip_frame_index, FurnitureLawInputs, FurnitureMarker,
    FurnitureStaticData, GameFileResolver, HeightmapGrid, VP_STRIP_COLOR_NEUTRAL_GRAY,
    VP_STRIP_FRAME_COUNT,
};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::font::load_game_font_preserving_color;
use crate::map_labels::{label_build_decision, LabelBuildDecision, MAP_LABEL_FONT_NAME};
use crate::render::MapRenderer;
use crate::text::{SpriteQuad, SpriteRenderer, TextRenderer};
use crate::Result;

/// Off-screen cull margin in window pixels: a marker whose anchor projects
/// further outside the viewport than this cannot touch it (container is
/// 160x30 GUI px; the margin covers the widest text comfortably).
const CULL_MARGIN: f32 = 200.0;

/// ENGINE-MEASURED (module doc, "Text"): the VP name text renders with the
/// map font `hoi_mapfont4` at this scale — advance-sum match x0.4027 /
/// x0.3993 on "Melbourne" (149 px) / "Adelaide" (119 px) vs the shipped
/// `.fnt` sums 370/298 (`w1check\furn_oracle_*.png`, gui_scale=1.0).
const VP_TEXT_MAPFONT_SCALE: f32 = 0.40;
/// ENGINE-MEASURED: text pen start, px right of the icon-frame centre
/// (36.3 / 38.3 measured on Melbourne / Adelaide).
const VP_TEXT_PEN_DX: f32 = 37.3;
/// ENGINE-MEASURED: text line-mid, px below the icon-frame centre (cap-top
/// anchoring: 'M' top = icon centre - 2.5 with a 22 px cap).
const VP_TEXT_LINE_MID_DY: f32 = 7.5;
/// ENGINE draw-stream truth (mined frame4700, 60 icon draws): each VP/capital
/// icon is a fixed screen-space GUI-sprite billboard drawn at its NATIVE 29x27
/// frame size with NO extra scale term — every draw's WorldViewProjectionMatrix
/// has XY scale exactly (2/1920, 2/1080), i.e. a 1:1 screen-pixel quad,
/// zoom-invariant. The earlier 1.25x was an eyeballed-screenshot fudge (the
/// "icons too big / look wrong" the user reported); the engine value is 1.0.
/// Residual (UNTRACED): the engine's icon highlight ring reads fully bright vs
/// the authored asymmetric arc — a shading difference, not scale.
const VP_ICON_SCALE: f32 = 1.0;

pub struct MapFurniture {
    text: TextRenderer,
    sprite: SpriteRenderer,
    frame_w: f32,
    frame_h: f32,
    static_data: FurnitureStaticData,
    markers: Vec<FurnitureMarker>,
    /// Terrain elevation per map pixel (heightmap.bmp, world_y = raw*0.1):
    /// anchors project through the production 3D camera AT their terrain
    /// height, so icons sit on the rendered cities (the tilted perspective
    /// camera displaces elevated terrain vs the flat 2D map mapping).
    heightmap: HeightmapGrid,
}

impl MapFurniture {
    /// Build the furniture composite. Fails closed with the exact missing
    /// piece named rather than drawing wrong data.
    pub fn create(
        gfx: &Gfx,
        resolver: &GameFileResolver,
        bundle: &MapBundle,
        owners: &BTreeMap<u32, String>,
    ) -> Result<MapFurniture> {
        // ENGINE-MEASURED font law (module doc, "Text"): the map font's glyphs
        // at 0.40 scale with the atlas RGB INVERTED (white body + dark rim,
        // the exact inverse of the country-label palette); alpha untouched.
        let mut font = load_game_font_preserving_color(resolver, MAP_LABEL_FONT_NAME)
            .map_err(|err| format!("map furniture: load {MAP_LABEL_FONT_NAME}: {err}"))?;
        for page in &mut font.pages {
            for texel in page.rgba.chunks_exact_mut(4) {
                texel[0] = 255 - texel[0];
                texel[1] = 255 - texel[1];
                texel[2] = 255 - texel[2];
                // Alpha normalization (ENGINE-MEASURED): the oracle's white
                // glyph body is OPAQUE over the map (blend residual < 0.05),
                // while the atlas body alpha is ~0.5 (the country-label
                // translucency). Doubling normalizes body 0.5 -> 1.0 and
                // keeps the softer rim proportional.
                texel[3] = texel[3].saturating_mul(2);
            }
        }
        let text = TextRenderer::new(gfx, &font)?;
        let strip = load_vp_strip_rgba(resolver)
            .map_err(|err| format!("map furniture: vp strip: {err}"))?;
        let sprite = SpriteRenderer::new(
            gfx,
            strip.width,
            strip.height,
            &strip.pixels,
            "vp mapicon strip",
        )?;
        let frame_w = (strip.width / VP_STRIP_FRAME_COUNT as usize) as f32;
        let frame_h = strip.height as f32;
        let static_data = furniture_static_data_from_resolver(resolver, "english")
            .map_err(|err| format!("map furniture: static data: {err}"))?;
        let heightmap = heightmap_grid_from_resolver(resolver)
            .map_err(|err| format!("map furniture: heightmap: {err}"))?;
        let markers = compute_markers(bundle, owners, &static_data);
        Ok(MapFurniture {
            text,
            sprite,
            frame_w,
            frame_h,
            static_data,
            markers,
            heightmap,
        })
    }

    /// Recompute marker attribution after the timeline hour (ownership)
    /// changes: capital/VP kind follows the controller's capital state.
    pub fn update_owners(&mut self, bundle: &MapBundle, owners: &BTreeMap<u32, String>) {
        self.markers = compute_markers(bundle, owners, &self.static_data);
    }

    pub fn marker_count(&self) -> usize {
        self.markers.len()
    }

    /// Draw the furniture for the current view. Returns `(icons, texts)`
    /// drawn. Icons batch into one alpha-blended draw; text uses the same
    /// screen-space pipeline as the overlay. Anchors project through the
    /// SAME production 3D camera the terrain renders with (at the anchor's
    /// heightmap elevation), so icons sit on the rendered cities; the flat
    /// 2D mapping is the fallback when no 3D pass exists.
    pub fn draw(
        &self,
        gfx: &Gfx,
        renderer: &MapRenderer,
        camera: &Camera,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<(usize, usize)> {
        let zoom = camera.zoom as f32;
        if zoom <= 0.0 {
            return Ok((0, 0));
        }
        // Camera distance for the shipped zoom cutoffs: the REAL production
        // 3D camera height (the engine convention those defines compare
        // against), falling back to the visible-world-height stand-in when
        // there is no 3D pass.
        let cam_dist = renderer
            .production_camera_height(camera, f64::from(screen_w))?
            .unwrap_or(screen_h / zoom);
        let defines = &self.static_data.vp_icon_defines;
        let mut icon_quads = Vec::new();
        let mut texts: Vec<(f32, f32, &str)> = Vec::new();
        for marker in &self.markers {
            let dot = defines.dot_visible(marker.kind, marker.vp_value, cam_dist);
            let text = defines.text_visible(marker.kind, marker.dot_tier, cam_dist);
            if !dot && !text {
                continue;
            }
            let elevation = self.heightmap.world_y(marker.anchor_x, marker.anchor_y);
            let projected = renderer.map_to_screen_3d(
                camera,
                f64::from(marker.anchor_x),
                f64::from(marker.anchor_y),
                elevation,
                f64::from(screen_w),
                f64::from(screen_h),
            )?;
            let (ax, ay) = match projected {
                Some(point) => point,
                None => camera.map_to_screen(
                    f64::from(marker.anchor_x),
                    f64::from(marker.anchor_y),
                    f64::from(screen_w),
                    f64::from(screen_h),
                ),
            };
            let (ax, ay) = (ax as f32, ay as f32);
            if ax < -CULL_MARGIN
                || ax > screen_w + CULL_MARGIN
                || ay < -CULL_MARGIN
                || ay > screen_h + CULL_MARGIN
            {
                continue;
            }
            // Icon-frame centre pinned on the projected province position
            // (ENGINE-MEASURED pivot, module doc); integer-snapped so the
            // point-sampled 1:1 blit stays byte-stable.
            let icon_cx = ax.round();
            let icon_cy = ay.round();
            if dot {
                let frame =
                    vp_strip_frame_index(marker.kind, marker.dot_tier, VP_STRIP_COLOR_NEUTRAL_GRAY);
                let u0 = frame as f32 / VP_STRIP_FRAME_COUNT as f32;
                let u1 = (frame + 1) as f32 / VP_STRIP_FRAME_COUNT as f32;
                let draw_w = self.frame_w * VP_ICON_SCALE;
                let draw_h = self.frame_h * VP_ICON_SCALE;
                let x0 = (icon_cx - draw_w * 0.5).round();
                let y0 = (icon_cy - draw_h * 0.5).round();
                icon_quads.push(SpriteQuad {
                    x0,
                    y0,
                    x1: x0 + draw_w,
                    y1: y0 + draw_h,
                    u0,
                    v0: 0.0,
                    u1,
                    v1: 1.0,
                    // White tint: the strip's authored pixels pass through
                    // (same convention as the GUI sprite pipeline).
                    color: 0xFFFF_FFFF,
                });
            }
            if text && !marker.label.is_empty() {
                texts.push((icon_cx, icon_cy, marker.label.as_str()));
            }
        }
        let icons = icon_quads.len();
        if !icon_quads.is_empty() {
            self.sprite.draw(gfx, &icon_quads)?;
        }
        // White vertex tint: the inverted pre-coloured atlas carries the
        // measured white-body/dark-rim palette, so RGB passes through (the
        // GUI font colour is white, `interface/core.gfx:1237`).
        for (icon_cx, icon_cy, label) in &texts {
            let pen_x = icon_cx + VP_TEXT_PEN_DX;
            let line_mid_y = icon_cy + VP_TEXT_LINE_MID_DY;
            self.text
                .draw_text_curved(gfx, VP_TEXT_MAPFONT_SCALE, label, 0xFFFF_FFFF, |u| {
                    (pen_x + u, line_mid_y, 0.0)
                })?;
        }
        Ok((icons, texts.len()))
    }
}

fn compute_markers(
    bundle: &MapBundle,
    owners: &BTreeMap<u32, String>,
    data: &FurnitureStaticData,
) -> Vec<FurnitureMarker> {
    compute_furniture_markers(&FurnitureLawInputs {
        province_ids: &bundle.province_ids,
        width: bundle.width,
        height: bundle.height,
        province_to_state: &bundle.province_to_state,
        state_owners: owners,
        state_victory_points: &data.state_victory_points,
        capital_states: &data.capital_states,
        localisation: &data.localisation,
        vp_icon_defines: Some(&data.vp_icon_defines),
    })
}

/// Build the resolver-backed furniture for the dump/app paths, mirroring the
/// LABGRAD1 graduation policy exactly (shared [`label_build_decision`]):
///
/// * `--no-map-furniture` -> `None` before any game file is read (the render
///   is byte-identical to the pre-FURN1 output).
/// * default-ON without a game install -> one-line downgrade notice, `None`.
/// * explicit `--map-furniture` without a game install -> fail closed.
pub fn create_if_enabled(
    enabled: bool,
    explicit: bool,
    gfx: &Gfx,
    game_resolver: Option<&GameFileResolver>,
    bundle: &MapBundle,
    owners: &BTreeMap<u32, String>,
) -> Result<Option<MapFurniture>> {
    match label_build_decision(enabled, explicit, game_resolver.is_some()) {
        LabelBuildDecision::Skip => Ok(None),
        LabelBuildDecision::Degrade => {
            eprintln!(
                "replay_app: notice: the city/VP map-furniture layer is on by default but \
                 needs --game <the game install dir> or BATTLE_PLAN_VANILLA for the mapicon sprite, \
                 hoi_20bs font, history and localisation; none found, so furniture is off \
                 (pass --no-map-furniture to silence this)."
            );
            Ok(None)
        }
        LabelBuildDecision::FailClosed => Err("--map-furniture needs --game <the game install dir> \
             or BATTLE_PLAN_VANILLA (the mapicon sprite, hoi_20bs font, history and localisation \
             are installed content)"
            .into()),
        LabelBuildDecision::Build => match game_resolver {
            Some(resolver) => Ok(Some(MapFurniture::create(gfx, resolver, bundle, owners)?)),
            None => Ok(None),
        },
    }
}
