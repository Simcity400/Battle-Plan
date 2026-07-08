//! Country-name map labels — **default ON** (LABGRAD1 graduation). The opt-out
//! `--no-map-labels` disables them, and that off path is byte-identical to the
//! pre-graduation default because no label code runs. Missing a `--game` install
//! on the default-ON path degrades to a one-line notice + labels off (fail open);
//! an explicit `--map-labels` fails closed (see [`create_if_enabled`]).
//!
//! LAB1 replaced OFF-4's straight-label approximation with the reconstructed
//! engine placement law; LAB3 proved the engine labels every controlled
//! component (UK = 8 labels) and LABGRAD1 made the law per-component. The
//! mechanism is files-first (see `documents/MAP_LABEL_LAW.md`):
//!
//! * Font: `hoi_mapfont4` (`interface/core.gfx:1679`-`interface/core.gfx:1680`,
//!   declared `tahoma_60`) -> fonts.json -> `.fnt/.dds`, loaded via the
//!   mod-aware resolver.
//! * Strings: `localisation/english/*.yml` country display names (plain tag
//!   keys; ideology variants need game state the recording does not carry).
//! * Visibility vs zoom: `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF` (vanilla 260,
//!   `common/defines/00_graphics.lua:781`) — the game stops drawing names below
//!   that camera distance. The replay camera is 2D, so visible-map-height in
//!   world units stands in for camera height (documented approximation);
//!   labels fade in over a ramp above the cutoff.
//! * Placement/orientation/curvature/size: NOT shipped (`map/positions.txt` is
//!   empty; no per-country name-position exists in any game file). Reconstructed
//!   by [`map_viewer::compute_country_label_placements`]: a PCA fit over each
//!   owner's passable owned-territory pixel cloud gives the centroid anchor,
//!   principal-axis orientation, extent-scaled length, and a quadratic bow that
//!   curves the baseline through the territory. DERIVED-BY-FIT; the engine's
//!   exact spline is UNKNOWN-ENGINE-SIDE.
//! * Rendering mirrors `gfx/FX/mapname.shader`: the `hoi_mapfont4` atlas is a
//!   pre-coloured, outlined glyph image (coverage in alpha, dark body + light
//!   rim in RGB — verified by decoding the DDS). The atlas RGBA is passed
//!   straight through ([`crate::font::AtlasMode::PreservedColor`]); the label's
//!   per-view transparency multiplies the sampled alpha (`vSample.a *= Transp`)
//!   over a src_alpha/inv_src_alpha blend, Linear-sampled like the shader. No
//!   black tint — that was the earlier bug that also mangled glyph coverage.

use std::collections::BTreeMap;
use std::path::Path;

use map_viewer::{
    compute_country_label_placements, cosmetic_tags_from_resolver,
    draw_country_names_cutoff_from_resolver, heightmap_grid_from_resolver,
    localisation_table_from_resolver, ruling_parties_from_resolver, CountryLabelPlacement,
    GameFileResolver, HeightmapGrid, LabelLawInputs, ProvinceKind,
};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::font::load_game_font_preserving_color;
use crate::render::MapRenderer;
use crate::text::TextRenderer;
use crate::Result;

/// Map font for name text: `interface/core.gfx` declares bitmapfont
/// "tahoma_60" with path `gfx/fonts/hoi_mapfont4` (fonts.json mirrors the
/// mapping) — the map-name glyph atlas despite the tahoma name.
pub const MAP_LABEL_FONT_NAME: &str = "tahoma_60";
// NAME2 unified size law: NO caller-side height constant survives. The old
// 65-unit ceiling (read off the 7B ongoing-save capture) is REFUTED by the
// frame4700 1936 oracle — SOVIET UNION draws at cap 131.9. The per-component
// cap is `map_viewer::LABEL_HEIGHT_BREADTH_K × inclusive minor extent`
// (map_label_law.rs); the fit-to-arc term lives in `LabelPass::draw_label`.
/// Fade-in ramp width as a fraction of the cutoff distance.
const FADE_RAMP_FRACTION: f32 = 0.25;
/// Maximum letterspacing stretch — how far past its natural width a name may be
/// justified across its baseline. ENGINE-OBSERVED (frame4700): no substantial
/// decoded label (cap ≥ 5 wu) exceeds ~2.0× (ITALY-boot = 2.00 exactly); beyond
/// that is the "N O R W A Y"-spread artifact of our whole-country PCA baseline.
/// Confines the word to the central portion of the baseline so hyper-elongated
/// countries render a compact name instead of a stretched one. See
/// `MAP_LABEL_LAW.md` §9.
const MAX_LETTERSPACE_STRETCH: f32 = 2.0;
/// Number of glyph-placement samples is implicit in the string length; the
/// baseline is sampled per glyph via the placement arc.

/// A resolved country label: the law placement plus the display string.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedLabel {
    pub placement: CountryLabelPlacement,
    pub text: String,
}

/// Compute the curved label placements via the LABGRAD1 per-component law
/// (one placement per controlled component, so a multi-region power gets several
/// labels), pairing each with its display name.
///
/// Display-name law (ENGINE-MEASURED, session_MAPNAME_7B vertex buffers):
/// * the engine renders IDEOLOGY-RESOLVED names ("MANDATORY PALESTINE",
///   "BRITISH IRAQ"; vanilla 1936 shows "GERMAN REICH" not "Germany") —
///   localisation key `<TAG>_<ideology>` (verified `GER_fascism: "German
///   Reich"`), ideology = the 1936 `ruling_party` from `history/countries`;
///   plain `<TAG>` is the fallback, the raw tag the last resort;
/// * the engine renders country names ALL-UPPERCASE (23/23 decoded engine
///   labels are caps), so the resolved string is uppercased.
pub fn resolve_country_labels(
    bundle: &MapBundle,
    owners: &BTreeMap<u32, String>,
    names: &BTreeMap<String, String>,
    ruling_parties: &BTreeMap<String, String>,
    cosmetic_tags: &BTreeMap<String, String>,
    province_type: &BTreeMap<u32, ProvinceKind>,
) -> Vec<ResolvedLabel> {
    let inputs = LabelLawInputs {
        province_ids: &bundle.province_ids,
        width: bundle.width,
        height: bundle.height,
        province_to_state: &bundle.province_to_state,
        owners,
        impassable_states: &bundle.impassable_states,
        province_importance: &bundle.province_importance,
        province_type,
    };
    compute_country_label_placements(&inputs)
        .into_iter()
        .map(|placement| {
            // NAME1 resolution chain, files-derived: a 1936 `set_cosmetic_tag`
            // (history/countries) takes priority — its ideology-resolved loc,
            // then its plain loc — before the country tag's own chain.
            // Vanilla proof: RAJ has cosmetic RAJ_UK, so 1936 India renders
            // "BRITISH RAJ" (`RAJ_UK_neutrality`) even though `RAJ_neutrality`
            // is "India".
            let party = ruling_parties.get(&placement.tag);
            let cosmetic = cosmetic_tags.get(&placement.tag);
            let mut candidates: Vec<String> = Vec::with_capacity(4);
            if let Some(cosmetic) = cosmetic {
                if let Some(party) = party {
                    candidates.push(format!("{cosmetic}_{party}"));
                }
                candidates.push(cosmetic.clone());
            }
            if let Some(party) = party {
                candidates.push(format!("{}_{party}", placement.tag));
            }
            candidates.push(placement.tag.clone());
            let text = candidates
                .iter()
                .find_map(|key| names.get(key))
                .cloned()
                .unwrap_or_else(|| placement.tag.clone())
                .to_uppercase();
            ResolvedLabel { placement, text }
        })
        .collect()
}

/// Label alpha for the current view. `visible_world_height` (screen height in
/// map units) stands in for the 3D camera height against
/// DRAW_COUNTRY_NAMES_CUTOFF.
pub fn label_alpha(visible_world_height: f32, cutoff: f32) -> f32 {
    if cutoff <= 0.0 {
        return 1.0;
    }
    ((visible_world_height - cutoff) / (cutoff * FADE_RAMP_FRACTION)).clamp(0.0, 1.0)
}

/// Load per-province land/water kind from the bundle's
/// `province_definitions.json` (the shipped `province_type`), for the RE-TITLE
/// engine flag byte. A province with an unknown / missing type defaults to
/// land, matching the placement law's fallback.
fn load_province_types(bundle_dir: &Path) -> Result<BTreeMap<u32, ProvinceKind>> {
    let path = bundle_dir.join("province_definitions.json");
    let bytes = std::fs::read(&path)
        .map_err(|err| format!("read {}: {err}", path.display()))?;
    let map: BTreeMap<String, serde_json::Value> = serde_json::from_slice(&bytes)
        .map_err(|err| format!("{} is not valid JSON: {err}", path.display()))?;
    let mut out = BTreeMap::new();
    for (key, value) in map {
        let Ok(pid) = key.parse::<u32>() else { continue };
        let kind = value
            .get("province_type")
            .and_then(|t| t.as_str())
            .map(ProvinceKind::from_type_str)
            .unwrap_or(ProvinceKind::Land);
        out.insert(pid, kind);
    }
    Ok(out)
}

pub struct MapLabels {
    text: TextRenderer,
    names: BTreeMap<String, String>,
    /// Tag -> 1936 ruling-party ideology (`history/countries`), for the
    /// engine's ideology-resolved display names.
    ruling_parties: BTreeMap<String, String>,
    /// Tag -> 1936 cosmetic tag (`set_cosmetic_tag` in `history/countries`);
    /// overrides the name loc chain (NAME1 — e.g. RAJ -> RAJ_UK ->
    /// "BRITISH RAJ").
    cosmetic_tags: BTreeMap<String, String>,
    cutoff: f32,
    labels: Vec<ResolvedLabel>,
    /// Per-province land/water kind (`province_definitions.json`
    /// `province_type`), for the RE-TITLE engine flag byte the placement law
    /// uses to trim owned runs. Loaded once from the bundle dir; ownership
    /// changes reuse it (province geometry/type is static).
    province_type: BTreeMap<u32, ProvinceKind>,
    /// Terrain elevation per map pixel: label baselines project through the
    /// production 3D camera AT the terrain height (same fix as the map
    /// furniture — the tilted perspective camera displaces elevated terrain
    /// vs the flat 2D mapping, so 2D-projected labels drift off their
    /// countries at close zooms).
    heightmap: HeightmapGrid,
}

impl MapLabels {
    /// Build the label renderer. Fails closed (with the exact missing piece
    /// named) rather than drawing wrong data.
    pub fn create(
        gfx: &Gfx,
        resolver: &GameFileResolver,
        bundle: &MapBundle,
        owners: &BTreeMap<u32, String>,
        bundle_dir: &Path,
    ) -> Result<MapLabels> {
        // Preserve the atlas's native RGBA: `hoi_mapfont4` bakes the outline
        // (dark body + light rim) into the glyph pixels, so the label must draw
        // that colour straight (mapname.shader analog), not a white coverage
        // mask tinted black.
        let font = load_game_font_preserving_color(resolver, MAP_LABEL_FONT_NAME)
            .map_err(|err| format!("map labels: load {MAP_LABEL_FONT_NAME}: {err}"))?;
        let text = TextRenderer::new(gfx, &font)?;
        let names = localisation_table_from_resolver(resolver, "english")
            .map_err(|err| format!("map labels: localisation: {err}"))?;
        let ruling_parties = ruling_parties_from_resolver(resolver)
            .map_err(|err| format!("map labels: ruling parties: {err}"))?;
        let cosmetic_tags = cosmetic_tags_from_resolver(resolver)
            .map_err(|err| format!("map labels: cosmetic tags: {err}"))?;
        let cutoff = draw_country_names_cutoff_from_resolver(resolver)
            .map_err(|err| format!("map labels: graphics defines: {err}"))?;
        let heightmap = heightmap_grid_from_resolver(resolver)
            .map_err(|err| format!("map labels: heightmap: {err}"))?;
        // RE-TITLE: the placement law needs each province's land/water kind
        // (the engine flag byte). Load it from the committed bundle's
        // `province_definitions.json` (static map data, generated from the
        // shipped map files). Fails closed like the other label inputs.
        let province_type = load_province_types(bundle_dir)
            .map_err(|err| format!("map labels: province definitions: {err}"))?;
        let labels = resolve_country_labels(
            bundle,
            owners,
            &names,
            &ruling_parties,
            &cosmetic_tags,
            &province_type,
        );
        // NAME2 VALIDATION-ONLY env knob: dump the resolved placements as
        // JSON so the RDOC parity harness can join OUR placement law against
        // the engine's decoded mapname vertex buffers (text, anchor, angle,
        // arc geometry per component). Never a render input.
        if let Ok(path) = std::env::var("REPLAY_DUMP_LABELS") {
            if !path.trim().is_empty() {
                let mut out = String::from("[\n");
                for (index, label) in labels.iter().enumerate() {
                    let p = &label.placement;
                    out.push_str(&format!(
                        "{}{{\"tag\":\"{}\",\"text\":\"{}\",\"component\":{},\"is_home\":{},\"anchor_x\":{},\"anchor_y\":{},\"angle_deg\":{},\"half_length\":{},\"bow\":{},\"world_height\":{},\"pixels\":{},\"major_extent\":{},\"minor_extent\":{},\"minor_extent_incl\":{},\"breadth_span\":{},\"breadth_p2_98\":{}}}",
                        if index > 0 { ",\n" } else { "" },
                        p.tag,
                        label.text.replace('"', "'"),
                        p.component_id,
                        p.is_home,
                        p.anchor_x,
                        p.anchor_y,
                        p.angle.to_degrees(),
                        p.half_length,
                        p.bow,
                        p.world_height,
                        p.pixel_count,
                        p.major_extent,
                        p.minor_extent,
                        p.minor_extent_incl,
                        p.breadth_span,
                        p.breadth_p2_98,
                    ));
                }
                out.push_str("\n]\n");
                if let Err(err) = std::fs::write(&path, out) {
                    eprintln!("map labels: REPLAY_DUMP_LABELS write {path}: {err}");
                } else {
                    eprintln!(
                        "map labels: dumped {} placements to {path} (NAME2 validation)",
                        labels.len()
                    );
                }
            }
        }
        Ok(MapLabels {
            text,
            names,
            ruling_parties,
            cosmetic_tags,
            cutoff,
            labels,
            province_type,
            heightmap,
        })
    }

    /// Recompute placements after the timeline hour (ownership) changes.
    pub fn update_owners(&mut self, bundle: &MapBundle, owners: &BTreeMap<u32, String>) {
        self.labels = resolve_country_labels(
            bundle,
            owners,
            &self.names,
            &self.ruling_parties,
            &self.cosmetic_tags,
            &self.province_type,
        );
    }

    /// Number of labels laid out (kept as `anchor_count` for API stability with
    /// the OFF-4 callers that print it).
    pub fn anchor_count(&self) -> usize {
        self.labels.len()
    }

    /// Draw the labels. LABHDR1 projection split: `cam_screen_w` is the app
    /// window width the SCENE's production camera derived from (the
    /// `height_for_screen_zoom` law input) — it must match the scene render.
    /// `target_w`/`target_h` are the dimensions of the surface being drawn
    /// into (the internal HDR MainScene on the product path). When drawing
    /// onto the window itself, pass `cam_screen_w == target_w`.
    pub fn draw(
        &self,
        gfx: &Gfx,
        renderer: &MapRenderer,
        camera: &Camera,
        cam_screen_w: f32,
        target_w: f32,
        target_h: f32,
        scene_target: Option<*mut winapi::um::d3d11::ID3D11RenderTargetView>,
    ) -> Result<usize> {
        // Screen px per world unit AT THE TARGET's scale: camera.zoom is
        // defined at the app-window scale; projecting the same clip coords
        // onto a wider target multiplies x by target_w/cam_screen_w.
        let zoom = camera.zoom as f32 * (target_w / cam_screen_w.max(1.0));
        if zoom <= 0.0 {
            return Ok(0);
        }
        // Camera distance vs DRAW_COUNTRY_NAMES_CUTOFF: the REAL production
        // 3D camera height when a 3D pass exists (the engine convention the
        // define compares against), else the visible-height stand-in.
        let visible_world_height = renderer
            .production_camera_height(camera, f64::from(cam_screen_w))?
            .unwrap_or(target_h / zoom);
        let alpha = label_alpha(visible_world_height, self.cutoff);
        if alpha <= 0.0 {
            return Ok(0);
        }
        let alpha_byte = (alpha * 255.0).round().clamp(0.0, 255.0) as u32;
        // mapname.shader law analog: the pre-coloured atlas carries the glyph
        // colour + outline, so the vertex tint is white (RGB passes through
        // unchanged) and only the per-label transparency rides the alpha
        // (`vSample.a *= Transp`), src_alpha/inv_src_alpha blend.
        let color = 0x00FF_FFFF | (alpha_byte << 24);
        // LABHDR1: route glyph draws into the HDR MainScene when given; the
        // override is cleared before any error propagates.
        if let Some(rtv) = scene_target {
            self.text.set_scene_target(rtv, target_w, target_h);
        }
        let mut drawn = 0usize;
        let mut result = Ok(());
        for label in &self.labels {
            match self.draw_label(
                gfx,
                renderer,
                camera,
                cam_screen_w,
                target_w,
                target_h,
                color,
                label,
            ) {
                Ok(true) => drawn += 1,
                Ok(false) => {}
                Err(err) => {
                    result = Err(err);
                    break;
                }
            }
        }
        self.text.clear_scene_target();
        result?;
        Ok(drawn)
    }

    /// Project a map point to the screen through the production 3D camera at
    /// its terrain elevation (the transform the terrain itself renders with),
    /// falling back to the flat 2D mapping when no 3D pass exists.
    fn project(
        &self,
        renderer: &MapRenderer,
        camera: &Camera,
        map_x: f64,
        map_y: f64,
        cam_screen_w: f32,
        target_w: f32,
        target_h: f32,
    ) -> (f64, f64) {
        let elevation = self.heightmap.world_y(map_x as f32, map_y as f32);
        match renderer.map_to_screen_3d_at(
            camera,
            map_x,
            map_y,
            elevation,
            f64::from(cam_screen_w),
            f64::from(target_w),
            f64::from(target_h),
        ) {
            Ok(Some(point)) => point,
            _ => camera.map_to_screen(map_x, map_y, f64::from(target_w), f64::from(target_h)),
        }
    }

    /// Draw one curved label; returns whether any glyph was emitted (i.e. the
    /// label fell inside the view).
    #[allow(clippy::too_many_arguments)]
    fn draw_label(
        &self,
        gfx: &Gfx,
        renderer: &MapRenderer,
        camera: &Camera,
        cam_screen_w: f32,
        target_w: f32,
        target_h: f32,
        color: u32,
        label: &ResolvedLabel,
    ) -> Result<bool> {
        let placement = &label.placement;
        // Measure the string so we can fit it to the baseline. The label text
        // spans the country: the glyphs are uniformly scaled so the whole word
        // fills the baseline arc (the game's names stretch across their territory),
        // with the per-country world height as an upper cap so a short name in a
        // big country does not get gigantic. Fitting to the *arc* length (not the
        // straight chord) is what keeps glyphs from overlapping on a bowed
        // baseline — the earlier "Germany"->"Cenany" bunching.
        let (text_w, _) = self.text.font().measure(&label.text);
        if text_w == 0 {
            return Ok(false);
        }
        let arc_len_map = placement.arc_length();
        // ENGINE title SIZE law (countryname.cpp, RE-verified — see
        // `map_viewer::CountryLabelPlacement::engine_glyph_scale`): the glyph
        // scale (map px per font px) is min( 0.8·0.9·len/textW,
        // 0.8·0.5·(pixels/len)/textH ), where len = the owned segment
        // (`arc_len_map`), textW = the measured natural width, and textH = the
        // font line height (mod-correct). The breadth term uses MEAN breadth
        // (pixels/len), which caps a long thin strip (French North Africa) small
        // — the fitted `0.22×minor_extent` law drew such labels ~1.4× too big.
        // Everything below is laid out in WORLD units — the engine builds
        // world-space glyph quads once and lets the 3D camera scale them, so no
        // screen `zoom` may enter the glyph geometry.
        let line_height = self.text.font().line_height.max(1) as f32;
        let glyph_scale = placement.engine_glyph_scale(text_w as f32, line_height);
        let s_world = glyph_scale.scale;
        // NAME2 (ENGINE-MEASURED, session_MAPNAME_7B, 23 decoded labels): when
        // the height cap bites (word narrower than the baseline at cap scale),
        // the engine JUSTIFIES the word across the baseline with uniform
        // ADDITIVE letter tracking — measured per-gap pen deltas exceed the
        // font xadvance by a constant per label (IRAN +61 fnt px/gap, FRANCE
        // +12, a tiny island UNITED KINGDOM +121; its across-the-space D->K
        // gap measures +239 ≈ 2 increments, so every character SLOT tracks,
        // spaces included) while long names lay out at natural advances
        // (residuals ±2 px, no kerning pairs in the .fnt, word gaps = the
        // space xadvance exactly). Tracking never compresses below natural.
        // NAME2 letterspacing CEILING (frame4700): the engine justifies a name
        // across its baseline, but only up to a bound — across all 68 decoded
        // labels NO substantial one (cap ≥ 5 wu) letterspaces beyond ~2.0×
        // its natural width (ITALY-boot = 2.00, LATVIA 1.63, SWEDEN 1.51).
        // Our PCA baseline spans 0.75× the whole passable major axis, so a
        // hyper-elongated country (Norway aspect 4.96, Chile 11.1) would
        // stretch its word 2.5–8× to fill that — the visible "N O R W A Y"
        // eyesore. We confine the word to the central [`MAX_LETTERSPACE_STRETCH`]
        // × natural span of the baseline (still centred on the anchor). This
        // removes behaviour the engine never exhibits; it does NOT reproduce
        // the engine's separate southern-Norway ANCHOR (that hyper-elongated
        // placement law is UNTRACED — one example can't fix it; C3 stays open
        // for a multi-elongated-country capture).
        let slot_gaps = label.text.chars().count().saturating_sub(1);
        let natural_arc = text_w as f32 * s_world;
        // The engine letterspaces a name across its segment ONLY when the breadth
        // CAP term binds (a short name in a wide country). A FIT-bound name draws
        // at natural advances, filling ~72% of the owned segment — it must NOT be
        // stretched to fill the whole baseline, which is what reintroduced the
        // "N O R W A Y over the sea" letterspacing eyesore.
        let word_arc = if glyph_scale.cap_bound {
            arc_len_map.min(natural_arc * MAX_LETTERSPACE_STRETCH)
        } else {
            natural_arc
        };
        // Fraction of the full baseline the word occupies (1.0 = fills it).
        let span_frac = if arc_len_map > f32::MIN_POSITIVE {
            (word_arc / arc_len_map).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let target_px = word_arc / s_world;
        let tracking = if slot_gaps > 0 {
            ((target_px - text_w as f32) / slot_gaps as f32).max(0.0)
        } else {
            0.0
        };
        let text_w_tracked = text_w as f32 + tracking * slot_gaps as f32;
        if text_w_tracked <= 0.0 {
            return Ok(false);
        }
        // Cheap cull: project the anchor; skip labels far outside the view.
        let (ax, ay) = self.project(
            renderer,
            camera,
            f64::from(placement.anchor_x),
            f64::from(placement.anchor_y),
            cam_screen_w,
            target_w,
            target_h,
        );
        let (ax, ay) = (ax as f32, ay as f32);
        if ax < -target_w || ax > target_w * 2.0 || ay < -target_h || ay > target_h * 2.0 {
            return Ok(false);
        }
        // Glyph geometry, engine-style (NAME2 + LAB3 VB decode): each corner is
        // laid out in WORLD space — pen centre at its arc-length fraction of
        // the baseline, the glyph box rotated into the local tangent frame,
        // everything scaled by `s_world` (map px per font px), flat on the map
        // plane at the terrain's elevation — and projected through the
        // production 3D camera PER CORNER. Screen size then comes from the
        // projection alone: tilt foreshortens glyphs, distance shrinks the far
        // end of a long label, and no window/target width factor can double up
        // (the old uniform screen scale drew mid-frame titles ~40% oversized
        // vs the frame4700 referee).
        let corner = |pen_u: f32, lx: f32, ly: f32| -> Option<[f32; 2]> {
            // Word occupies the central `span_frac` of the baseline, centred on
            // the anchor (span_frac = 1.0 fills it end to end — the usual case).
            let word_frac = (pen_u / text_w_tracked).clamp(0.0, 1.0);
            let arc_frac = 0.5 + (word_frac - 0.5) * span_frac;
            let t = placement.t_at_arc_fraction(arc_frac);
            let (bx, by) = placement.baseline_point(t);
            // Tangent frame in map (y-down) space; +ly runs toward descenders
            // (map south for an unrotated label), matching the screen-space
            // convention this replaces.
            let angle = placement.tangent_angle(t);
            let (sin, cos) = angle.sin_cos();
            let wx = bx + (lx * cos - ly * sin) * s_world;
            let wy = by + (lx * sin + ly * cos) * s_world;
            let (sx, sy) = self.project(
                renderer,
                camera,
                f64::from(wx),
                f64::from(wy),
                cam_screen_w,
                target_w,
                target_h,
            );
            Some([sx as f32, sy as f32])
        };
        self.text
            .draw_text_curved_projected(gfx, &label.text, color, tracking, corner)?;
        Ok(true)
    }
}

/// Build the resolver-backed labels for the dump/app paths.
///
/// LABGRAD1 graduation: country-name labels are **default ON**. The opt-out
/// `--no-map-labels` sets `enabled = false` and short-circuits to `None` before
/// any game file is read — that path is byte-identical to the pre-graduation
/// default (no label code runs). Missing-evidence policy mirrors GRAD1:
///
/// * `enabled && explicit` (user typed `--map-labels`): **fail closed** — a
///   missing `--game`/font/localisation/defines errors out naming the piece.
/// * `enabled && !explicit` (the default-ON path): **fail open** — a missing
///   `--game` resolver logs a one-line downgrade notice and returns `None`
///   (labels off) so a recording without a game install still renders.
pub fn create_if_enabled(
    enabled: bool,
    explicit: bool,
    gfx: &Gfx,
    game_resolver: Option<&GameFileResolver>,
    bundle: &MapBundle,
    owners: &BTreeMap<u32, String>,
    bundle_dir: &Path,
) -> Result<Option<MapLabels>> {
    match label_build_decision(enabled, explicit, game_resolver.is_some()) {
        LabelBuildDecision::Skip => Ok(None),
        LabelBuildDecision::Degrade => {
            // Default-ON degrade: no game install -> no fonts/strings/defines, so
            // labels can't render. Log once and fall back to labels off (same
            // pattern as GRAD1's water/postfx downgrade).
            eprintln!(
                "replay_app: notice: country-name map labels are on by default but need \
                 --game <the game install dir> or BATTLE_PLAN_VANILLA for the map font, localisation \
                 and 00_graphics.lua; none found, so labels are off (pass --no-map-labels \
                 to silence this)."
            );
            Ok(None)
        }
        LabelBuildDecision::FailClosed => Err("--map-labels needs --game <the game install dir> \
             or BATTLE_PLAN_VANILLA (fonts, localisation and 00_graphics.lua are installed content)"
            .into()),
        // `Build` is only returned when `has_resolver`, so this arm always has
        // one; matching on it (rather than unwrapping) keeps the code panic-free.
        LabelBuildDecision::Build => match game_resolver {
            Some(resolver) => Ok(Some(MapLabels::create(
                gfx, resolver, bundle, owners, bundle_dir,
            )?)),
            None => Ok(None),
        },
    }
}

/// The pure LABGRAD1 opt-out / graduation decision, factored out so the fail-open
/// vs fail-closed vs byte-identical-skip policy is unit-testable without a D3D
/// device. Given the resolved flags and whether a game resolver is present:
///
/// * `!enabled` -> **Skip**: `--no-map-labels` (or the pre-graduation default).
///   No label code runs; the render is byte-identical to the old flag-off path.
/// * `enabled && has_resolver` -> **Build**: draw the labels.
/// * `enabled && !has_resolver && explicit` -> **FailClosed**: the user asked for
///   labels but there is no game install to supply the font/strings/defines.
/// * `enabled && !has_resolver && !explicit` -> **Degrade**: the default-ON path
///   with no game install; log a notice and fall back to labels off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelBuildDecision {
    Skip,
    Build,
    Degrade,
    FailClosed,
}

pub fn label_build_decision(
    enabled: bool,
    explicit: bool,
    has_resolver: bool,
) -> LabelBuildDecision {
    if !enabled {
        return LabelBuildDecision::Skip;
    }
    if has_resolver {
        return LabelBuildDecision::Build;
    }
    if explicit {
        LabelBuildDecision::FailClosed
    } else {
        LabelBuildDecision::Degrade
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn tiny_bundle() -> MapBundle {
        // 64x16 map: province 1 (state 10, owned GER) fills the left half,
        // province 3 (state 30, impassable) the next quarter, province 2
        // (state 20, owned FRA) the right quarter. Sized so each owned state
        // clears MIN_LABEL_PIXELS at the law's sample stride.
        let width = 64usize;
        let height = 16usize;
        let mut province_ids = vec![0u32; width * height];
        for y in 0..height {
            for x in 0..width {
                province_ids[y * width + x] = if x < 32 {
                    1
                } else if x < 48 {
                    3
                } else {
                    2
                };
            }
        }
        MapBundle {
            width,
            height,
            province_ids,
            map_visual: vec![0; width * height],
            province_to_state: BTreeMap::from([(1, 10), (2, 20), (3, 30)]),
            state_owner_1936: BTreeMap::from([
                (10, "GER".to_string()),
                (20, "FRA".to_string()),
                (30, "GER".to_string()),
            ]),
            country_colors: BTreeMap::new(),
            impassable_states: BTreeSet::from([30]),
            province_importance: BTreeMap::new(),
        }
    }

    /// Land types for the tiny_bundle provinces (all land: 1, 2, 3). The
    /// RE-TITLE placement law needs a province_type table.
    fn tiny_ptypes() -> BTreeMap<u32, ProvinceKind> {
        BTreeMap::from([
            (1u32, ProvinceKind::Land),
            (2u32, ProvinceKind::Land),
            (3u32, ProvinceKind::Land),
        ])
    }

    #[test]
    fn labels_use_law_placement_and_skip_impassable() {
        let bundle = tiny_bundle();
        let owners = bundle.state_owner_1936.clone();
        let names = BTreeMap::from([
            ("GER".to_string(), "Germany".to_string()),
            ("FRA".to_string(), "France".to_string()),
        ]);
        // GER's 1936 ruling party resolves the ideology name (engine law:
        // vanilla 1936 shows "GERMAN REICH"); FRA has no entry -> plain key.
        // Both render UPPERCASE (23/23 decoded engine labels are caps).
        let ruling = BTreeMap::from([("GER".to_string(), "fascism".to_string())]);
        let names = {
            let mut names = names;
            names.insert("GER_fascism".to_string(), "German Reich".to_string());
            names
        };
        let labels = resolve_country_labels(
            &bundle,
            &owners,
            &names,
            &ruling,
            &BTreeMap::new(),
            &tiny_ptypes(),
        );
        // Two owners label (impassable state 30 excluded, so GER's cloud is
        // only the left half).
        assert_eq!(labels.len(), 2);
        let ger = labels.iter().find(|l| l.placement.tag == "GER").unwrap();
        assert_eq!(ger.text, "GERMAN REICH");
        let fra_text = &labels
            .iter()
            .find(|l| l.placement.tag == "FRA")
            .unwrap()
            .text;
        assert_eq!(fra_text, "FRANCE");
        // GER's labelable territory is the left half -> anchor left of centre.
        assert!(ger.placement.anchor_x < 32.0, "{}", ger.placement.anchor_x);
        let fra = labels.iter().find(|l| l.placement.tag == "FRA").unwrap();
        assert!(fra.placement.anchor_x >= 48.0, "{}", fra.placement.anchor_x);
    }

    #[test]
    fn cosmetic_tag_name_beats_tag_chain() {
        // NAME1 vanilla-shaped fixture: GER carries a cosmetic tag whose
        // ideology-resolved loc must win over the tag's own ideology loc
        // (India: RAJ_UK_neutrality "British Raj" beats RAJ_neutrality
        // "India").
        let bundle = tiny_bundle();
        let owners = bundle.state_owner_1936.clone();
        let ruling = BTreeMap::from([("GER".to_string(), "neutrality".to_string())]);
        let cosmetic = BTreeMap::from([("GER".to_string(), "GER_COS".to_string())]);
        let names = BTreeMap::from([
            ("GER".to_string(), "Germany".to_string()),
            ("GER_neutrality".to_string(), "German Republic".to_string()),
            (
                "GER_COS_neutrality".to_string(),
                "Cosmetic Reich".to_string(),
            ),
            ("GER_COS".to_string(), "Cosmetic Base".to_string()),
        ]);
        let labels =
            resolve_country_labels(&bundle, &owners, &names, &ruling, &cosmetic, &tiny_ptypes());
        let ger = labels.iter().find(|l| l.placement.tag == "GER").unwrap();
        assert_eq!(ger.text, "COSMETIC REICH");
        // Without the ideology entry the plain cosmetic loc is next.
        let names_no_ideology = {
            let mut names = names.clone();
            names.remove("GER_COS_neutrality");
            names
        };
        let labels = resolve_country_labels(
            &bundle,
            &owners,
            &names_no_ideology,
            &ruling,
            &cosmetic,
            &tiny_ptypes(),
        );
        let ger = labels.iter().find(|l| l.placement.tag == "GER").unwrap();
        assert_eq!(ger.text, "COSMETIC BASE");
        // No cosmetic locs at all: falls through to the tag chain.
        let names_tag_only = BTreeMap::from([
            ("GER".to_string(), "Germany".to_string()),
            ("GER_neutrality".to_string(), "German Republic".to_string()),
        ]);
        let labels = resolve_country_labels(
            &bundle,
            &owners,
            &names_tag_only,
            &ruling,
            &cosmetic,
            &tiny_ptypes(),
        );
        let ger = labels.iter().find(|l| l.placement.tag == "GER").unwrap();
        assert_eq!(ger.text, "GERMAN REPUBLIC");
    }

    #[test]
    fn labels_fall_back_to_tag_without_localisation() {
        let bundle = tiny_bundle();
        let owners = bundle.state_owner_1936.clone();
        let labels = resolve_country_labels(
            &bundle,
            &owners,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &tiny_ptypes(),
        );
        let ger = labels.iter().find(|l| l.placement.tag == "GER").unwrap();
        assert_eq!(ger.text, "GER");
    }

    #[test]
    fn labels_follow_ownership_changes() {
        let bundle = tiny_bundle();
        let mut owners = bundle.state_owner_1936.clone();
        owners.insert(20, "GER".to_string()); // FRA annexed
        let labels = resolve_country_labels(
            &bundle,
            &owners,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &tiny_ptypes(),
        );
        // Only GER remains (FRA gone). GER's left half (x<32) and right half
        // (x>=48) are joined through the east-west map wrap (col 0 is adjacent to
        // col 63), so they form ONE component -> one merged GER label. (The
        // impassable middle band splits them left-to-right, but the wrap seam
        // reconnects them.) LABGRAD1 per-component semantics: one label here.
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].placement.tag, "GER");
    }

    #[test]
    fn opt_out_skips_labels_byte_identically() {
        // LABGRAD1 opt-out gate: --no-map-labels (enabled=false) SKIPS the label
        // pass unconditionally — no label code runs, so the render is
        // byte-identical to the pre-graduation flag-off default. This holds
        // regardless of `explicit` or whether a game resolver is present.
        for &explicit in &[false, true] {
            for &has_resolver in &[false, true] {
                assert_eq!(
                    label_build_decision(false, explicit, has_resolver),
                    LabelBuildDecision::Skip,
                    "opt-out must always skip (explicit={explicit}, resolver={has_resolver})"
                );
            }
        }
    }

    #[test]
    fn default_on_builds_with_resolver_degrades_without() {
        // Default-ON (enabled, not explicit): build when a game install is
        // present, else DEGRADE to labels-off + notice (fail open) so a recording
        // without --game still renders.
        assert_eq!(
            label_build_decision(true, false, true),
            LabelBuildDecision::Build
        );
        assert_eq!(
            label_build_decision(true, false, false),
            LabelBuildDecision::Degrade
        );
    }

    #[test]
    fn explicit_request_fails_closed_without_resolver() {
        // Explicit --map-labels: build with a resolver, else FAIL CLOSED (the
        // user asked for labels; a missing game install is an error, not a
        // silent downgrade).
        assert_eq!(
            label_build_decision(true, true, true),
            LabelBuildDecision::Build
        );
        assert_eq!(
            label_build_decision(true, true, false),
            LabelBuildDecision::FailClosed
        );
    }

    #[test]
    fn label_alpha_fades_in_above_cutoff() {
        // DRAW_COUNTRY_NAMES_CUTOFF law (vanilla 260): nothing below the
        // cutoff, fully visible past the 25% ramp.
        assert_eq!(label_alpha(200.0, 260.0), 0.0);
        assert_eq!(label_alpha(260.0, 260.0), 0.0);
        assert_eq!(label_alpha(260.0 + 65.0, 260.0), 1.0);
        assert_eq!(label_alpha(2048.0, 260.0), 1.0);
        let mid = label_alpha(260.0 + 32.5, 260.0);
        assert!((mid - 0.5).abs() < 1e-4, "{mid}");
    }
}
