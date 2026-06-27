//! CORE SURFACE 1 (migrated) — the MAP + 14 modes, as a registered Background-band
//! view. It owns the pan/zoom CAMERA (its persistent input state) and a `'static`
//! reference to the GPU map data + the shared `PoliticalMap` pass (leaked once at
//! startup, as in the old shell).
//!
//! `bind` is PURE: it reads the snapshot and computes the per-hour LUT BYTES + the
//! fidelity badge (no GPU side-effects). The `Drawable` carries those bytes + the
//! `'static` map data, then at draw time UPLOADS the LUT and runs the offscreen map
//! pass + the active-mode HUD. Mirrors the old `rebuild_map_lut` + `draw_mode_badge`
//! EXACTLY so the map is byte-identical; the stateless→all-neutral rule (no stale-LUT
//! carry-over) is preserved because the bytes are recomputed every bind.
//!
//! The map is the BOTTOM layer: a click no window captures falls through (host `Miss`)
//! to the app's province pick (kept in `main.rs`: it needs the camera + raster, which
//! the map view owns and the app reads back).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use render::dynamic::{pack_province_dynamic_lut, DynamicInputs, DynamicUniforms};
use render::map::{CountryColors, MapIds, PoliticalMap, ProvinceRaster};
use render::modes::{MapMode, ModeData};
use render::overlay::{
    build_overlay_plan, OverlayDiagnostics, OverlayInputs, OverlayLinePattern, OverlayLineStyle,
    OverlayNames, OverlayOptions, OverlayPlan, OverlayProjection, OverlayStatic, ScreenPoint,
};
use render::strategic_regions::StrategicRegions;

use crate::camera::MapCamera;
use crate::host::context::BindContext;
use crate::host::draw::FrameDraw;
use crate::host::registry::{ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::host::FidelityReport;
use crate::register_view;
use widgets::{plan_tooltip, Atom as WAtom, Reason as WReason, Tooltip, TooltipPlan};

/// The static (hour-independent) map data — the raster + ids + colours + the GPU pass.
/// Leaked once at startup to `'static` so the map view (which lives across the
/// `'static` event loop) can borrow it.
pub struct MapData {
    pub raster: ProvinceRaster,
    pub ids: MapIds,
    pub colors: CountryColors,
    pub pmap: PoliticalMap,
    /// The province→strategic-region join + sea-province set, parsed once at startup from the
    /// resolved installed content (mod over vanilla). Used SOLELY by the FROZEN-SEAS dynamic branch
    /// to turn a measured per-region `arctic_water` phenomenon into the right per-province
    /// `frozen` flags (sea provinces only). `None` when the strategic-region files could not be
    /// resolved — the dynamic snow/mud feed still works; frozen seas are simply not drawn
    /// (no fabrication: an unknown region join must not invent frozen water).
    pub strategic_regions: Option<StrategicRegions>,
}

/// Factual hover data for the exposed native map. This is populated only from the
/// same CPU raster pick used by click selection; unresolved state/owner/name fields
/// stay `None` and render as unavailable rows instead of guessed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapHover {
    pub cursor: (i32, i32),
    pub raster: (u32, u32),
    pub province_id: u32,
    pub state_id: Option<u32>,
    pub owner: Option<String>,
    pub state_name_key: Option<String>,
}

impl MapHover {
    pub fn from_pick(
        pick: render::map::PickResult,
        cursor: (f32, f32),
        raster: (u32, u32),
    ) -> MapHover {
        MapHover {
            cursor: (cursor.0.round() as i32, cursor.1.round() as i32),
            raster,
            province_id: pick.province_id,
            state_id: pick.state_id,
            owner: pick.owner,
            state_name_key: pick.state_name_key,
        }
    }

    fn tooltip_plan(&self, ctx: &BindContext) -> TooltipPlan {
        let state_id = self
            .state_id
            .map(|sid| WAtom::exact(format!("#{sid}")))
            .unwrap_or_else(|| WAtom::unavailable_because(WReason::Absent));
        let owner = self
            .owner
            .as_ref()
            .map(|tag| WAtom::exact(tag.clone()))
            .unwrap_or_else(|| WAtom::unavailable_because(WReason::Absent));
        let state_name = self
            .state_name_key
            .as_deref()
            .map(|key| {
                let resolved = ctx.stage.loc.resolve(key);
                if resolved.trim().is_empty() {
                    WAtom::exact(key)
                } else {
                    WAtom::exact(resolved)
                }
            })
            .unwrap_or_else(|| WAtom::unavailable_because(WReason::Absent));
        let tooltip = Tooltip::new(WAtom::exact(format!("Province #{}", self.province_id)))
            .with_row("Province", WAtom::exact(format!("#{}", self.province_id)))
            .with_row("State", state_id)
            .with_row("State name", state_name)
            .with_row("Owner", owner)
            .with_row(
                "Raster",
                WAtom::exact(format!("{},{}", self.raster.0, self.raster.1)),
            );
        plan_tooltip(
            &tooltip,
            (self.cursor.0 as f32, self.cursor.1 as f32),
            ctx.screen,
        )
    }
}

/// The id the map view keys on.
pub const MAP_VIEW_ID: ViewId = ViewId("core.map");

/// The map view instance: owns the camera + the active mode + the `'static` map data.
pub struct MapView {
    pub camera: MapCamera,
    pub mode: MapMode,
    data: &'static MapData,
    hover: Option<MapHover>,
}

impl MapView {
    pub fn new(camera: MapCamera, mode: MapMode, data: &'static MapData) -> MapView {
        MapView {
            camera,
            mode,
            data,
            hover: None,
        }
    }

    pub fn apply_command(&mut self, command: &ViewCommand) -> bool {
        apply_map_command_to_mode(&mut self.mode, command)
    }

    pub fn set_hover(&mut self, hover: Option<MapHover>) -> bool {
        if self.hover == hover {
            return false;
        }
        self.hover = hover;
        true
    }

    /// Compute the per-hour LUT bytes + badge for `mode` (pure; the byte-identical
    /// mirror of the old `rebuild_map_lut`, incl. the front-line adjacency special case
    /// + the stateless→all-neutral rule via `ModeData::from_snapshot`).
    fn compute_lut(&self, snap: &snapshot::Snapshot) -> (Vec<u8>, render::modes::Fidelity) {
        let empty = serde_json::Map::new();
        let states = snap.states().unwrap_or(&empty);
        let mut md =
            ModeData::from_snapshot(states, snap.domain("map"), snap.domain("diplomacy_world"))
                .with_provinces(snap.domain("provinces"))
                .with_countries(snap.countries())
                .with_weather(snap.domain("weather"))
                .with_naval_air(snap.domain("naval_air"));
        if matches!(self.mode, MapMode::FrontLines) {
            md = md.with_front_adjacency(&self.data.raster);
        }
        let ml = render::modes::recolor(
            self.mode,
            &md,
            &self.data.ids,
            &self.data.colors,
            self.data.raster.max_id,
        );
        (ml.lut, ml.fidelity)
    }

    /// Compute the per-hour DYNAMIC map feed (snow/mud/frozen-seas/season/day-night) from the
    /// snapshot WEATHER domain. Pure: returns the packed per-province dynamic LUT bytes + the
    /// GPU uniform, plus a `gate` flag that is true ONLY when at least one province carried a
    /// MEASURED weather datum:
    ///   * a per-province `weather.provinces.*.snow` number (incl. a measured 0.0) → winter,
    ///   * a per-province `weather.provinces.*.mud` bool → mud, or
    ///   * a SEA province frozen by a measured per-region `weather.regions.*.phenomenon`
    ///     containing `arctic_water` (via the province→region join in [`MapData`]).
    /// When the gate is false the draw keeps the neutral/static map (byte-identical to the
    /// no-weather path) — the dynamic LUT is uploaded only when there is real weather to show
    /// (no-fabrication: an absent weather domain must not perturb the map). The date drives the
    /// season/day-night uniform.
    fn compute_dynamic(&self, snap: &snapshot::Snapshot) -> Option<(Vec<u8>, DynamicUniforms)> {
        let num_provinces = self.data.raster.max_id as usize + 1;
        let date = parse_date_ymdh(snap.date.as_deref().unwrap_or(""));
        let weather = snap.domain("weather");

        // FROZEN SEAS: resolve the per-region `arctic_water` phenomenon to the SEA provinces of
        // each arctic region (sea-only, via the parsed strategic-region join). Empty when there
        // is no join, no weather, or no arctic_water region — never a fabricated freeze.
        let frozen_seas = match (&self.data.strategic_regions, weather) {
            (Some(sr), Some(w)) => match w.get("regions") {
                Some(regions) => sr.frozen_sea_provinces(regions),
                None => std::collections::HashSet::new(),
            },
            _ => std::collections::HashSet::new(),
        };

        let (inp, measured_any, snow_oor) =
            DynamicInputs::from_weather_snapshot_full(num_provinces, date, weather, &frozen_seas);
        if snow_oor > 0 {
            // honesty signal (Codex (a)): measured snow values outside 0..1 were clamped to the
            // shader range. Surface it rather than let bad data become a plausible visual.
            eprintln!("[map] note: {snow_oor} measured snow value(s) outside 0..1 were clamped");
        }
        if !measured_any {
            return None; // no measured weather → keep the static/neutral map.
        }
        let lut = pack_province_dynamic_lut(&inp.provinces, inp.snow_mud_fade);
        let uniforms = DynamicUniforms::from_inputs(&inp);
        Some((lut, uniforms))
    }

    /// Build the native-map label/vector overlay for this snapshot and camera. The
    /// planner is pure and accepts only static map definitions plus measured snapshot
    /// domains; missing route/arrow geometry becomes diagnostics, not drawn guesses.
    fn compute_overlay(&self, ctx: &BindContext) -> OverlayPlan {
        let Some(static_data) = overlay_static(&ctx.stage.map_dir, &self.data.raster) else {
            return OverlayPlan::default();
        };
        let projection = OverlayProjection {
            offset: self.camera.offset,
            scale: self.camera.scale,
            screen_w: ctx.screen.w.max(1.0),
            screen_h: ctx.screen.h.max(1.0),
            raster_w: self.data.raster.width as f32,
            raster_h: self.data.raster.height as f32,
        };
        let names = overlay_names(ctx, static_data);
        let mut options = OverlayOptions::default();
        options.show_front_lines = matches!(self.mode, MapMode::FrontLines);
        build_overlay_plan(
            static_data,
            OverlayInputs {
                states: ctx.snapshot().domain("states"),
                map: ctx.snapshot().domain("map"),
                divisions: ctx.snapshot().domain("divisions"),
                order_of_battle: ctx.snapshot().domain("order_of_battle"),
                diplomacy_world: ctx.snapshot().domain("diplomacy_world"),
            },
            &names,
            projection,
            options,
        )
    }
}

pub fn apply_map_command_to_mode(active: &mut MapMode, command: &ViewCommand) -> bool {
    let ViewCommand::SetMapMode { mode } = *command else {
        return false;
    };
    if *active == mode {
        return false;
    }
    *active = mode;
    true
}

/// Parse an `Y.M.D.H` in-game date string to `(year, month, day, hour)` for the dynamic
/// season/day-night uniform. A missing/garbled component falls back to a neutral summer-noon
/// component (1936/7/15/12) so the dynamic uniform is well-formed — this is a RENDER cosmetic
/// default for the season tint, NOT a fabricated DATA value (no snapshot field is invented).
fn parse_date_ymdh(date: &str) -> (i32, u32, u32, u32) {
    let mut it = date.split('.');
    let y = it.next().and_then(|s| s.parse().ok()).unwrap_or(1936);
    let m = it.next().and_then(|s| s.parse().ok()).unwrap_or(7);
    let d = it.next().and_then(|s| s.parse().ok()).unwrap_or(15);
    let h = it.next().and_then(|s| s.parse().ok()).unwrap_or(12);
    (y, m, d, h)
}

/// The map's `Drawable`: uploads the (pure-computed) LUT, runs the offscreen
/// political-map pass (bottom, CLEARS the target), then draws the active-mode HUD.
pub struct MapDrawable {
    data: &'static MapData,
    lut: Vec<u8>,
    offset: [f32; 2],
    scale: [f32; 2],
    mode_label: String,
    badge: String,
    /// the screen size at bind time — a draw input (the map fills the screen), so it is
    /// part of the digest (Codex: the dst rect is `[0,0,w,h]`).
    screen: (u32, u32),
    /// the per-hour DYNAMIC feed (snow/mud LUT + season/day-night uniform) from the snapshot
    /// weather domain, present ONLY when this hour carried a measured weather datum. `None`
    /// keeps the static/neutral map (the no-weather path stays byte-identical).
    dynamic: Option<(Vec<u8>, DynamicUniforms)>,
    /// Text/vector overlays over the native map, already projected for this camera/snapshot.
    overlay: OverlayPlan,
    /// Passive factual tooltip for the currently exposed province hover.
    tooltip: Option<TooltipPlan>,
}

impl Drawable for MapDrawable {
    fn tooltip_plan(&self) -> Option<&TooltipPlan> {
        self.tooltip.as_ref()
    }

    /// The map's pixel inputs: a hash of the per-hour LUT bytes (the colouring) + the
    /// camera uniforms (the transform) + the mode label/badge. A wrong LUT upload, a
    /// stale colouring, a changed camera, or a wrong mode all perturb this — so the
    /// digest detects a silent map-render change that an empty node digest would miss
    /// (Codex (c) mitigation, headless).
    fn draw_digest(&self) -> String {
        map_draw_digest(
            &self.lut,
            self.dynamic.as_ref().map(|(lut, _)| lut.as_slice()),
            self.offset,
            self.scale,
            self.screen,
            &self.mode_label,
            &self.badge,
            &self.overlay,
            self.tooltip.as_ref(),
        )
    }

    fn draw(&self, fd: &mut FrameDraw) {
        let (w, h) = fd.screen();
        // upload the per-hour LUT (GPU side-effect lives in DRAW, not bind).
        self.data.pmap.update_lut(fd.renderer, &self.lut);
        // upload the per-hour DYNAMIC weather feed ONLY when this hour carried measured
        // weather (snow/mud + season/day-night). With no weather the dynamic state stays at
        // the renderer's neutral default → the static map is unchanged (no fabrication).
        if let Some((dyn_lut, uniforms)) = &self.dynamic {
            self.data
                .pmap
                .update_dynamic(fd.renderer, dyn_lut, uniforms);
        }
        self.data.pmap.render_to_offscreen(
            fd.renderer,
            [0.0, 0.0, w as f32, h as f32],
            self.offset,
            self.scale,
            [0.05, 0.06, 0.08, 1.0],
        );
        draw_map_overlay(fd, &self.overlay);
        draw_mode_badge(fd, &self.mode_label, &self.badge);
        draw_overlay_badge(fd, &self.overlay.diagnostics);
    }
}

impl ViewInstance for MapView {
    fn view_id(&self) -> ViewId {
        MAP_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Background)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let (lut, badge) = self.compute_lut(ctx.snapshot());
        // the per-hour weather feed (snow/mud + season/day-night), gated to measured weather.
        let dynamic = self.compute_dynamic(ctx.snapshot());
        let overlay = self.compute_overlay(ctx);
        let tooltip = self.hover.as_ref().map(|h| h.tooltip_plan(ctx));
        let drawable = MapDrawable {
            data: self.data,
            lut,
            offset: self.camera.uniform_offset(),
            scale: self.camera.uniform_scale(),
            mode_label: self.mode.label(),
            badge: badge.badge().to_string(),
            screen: (ctx.screen.w as u32, ctx.screen.h as u32),
            dynamic,
            overlay: overlay.clone(),
            tooltip,
        };
        let mut fidelity = FidelityReport::default();
        match badge {
            render::modes::Fidelity::Exact => fidelity.exact += 1,
            render::modes::Fidelity::Derived | render::modes::Fidelity::Proxy => {
                fidelity.derived += 1
            }
            render::modes::Fidelity::Unavailable => fidelity.unavailable += 1,
        }
        if !overlay.labels.is_empty() || !overlay.lines.is_empty() {
            fidelity.derived += 1;
        }
        if overlay.diagnostics.unavailable_arrows > 0
            || overlay.diagnostics.unavailable_routes > 0
            || overlay.diagnostics.unavailable_front_lines > 0
        {
            fidelity.unavailable += 1;
        }
        if self.hover.is_some() {
            fidelity.derived += 1;
        }
        BoundView {
            instance: InstanceId(0),
            view_id: MAP_VIEW_ID,
            policy: self.policy(),
            // the map captures NO GUI leaf — a non-window click falls through to the
            // app's province pick (host `Miss`). Empty hit model, no block rect.
            hit: HitModel::none(),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// A tiny FNV-1a hash of a byte buffer (for the LUT digest — no crypto needed, just a
/// stable, cheap fingerprint that changes when any LUT byte changes).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn map_draw_digest(
    lut: &[u8],
    dynamic_lut: Option<&[u8]>,
    offset: [f32; 2],
    scale: [f32; 2],
    screen: (u32, u32),
    mode_label: &str,
    badge: &str,
    overlay: &OverlayPlan,
    tooltip: Option<&TooltipPlan>,
) -> String {
    let lut_hash = fnv1a(lut);
    // Include the dynamic (weather) feed in the digest so a scrub that changes the
    // snow/mud LUT (or toggles weather on/off) perturbs the headless render fingerprint.
    let dyn_hash = dynamic_lut.map(fnv1a).unwrap_or(0);
    format!(
        "MAP lut={lut_hash:016x} dyn={dyn_hash:016x} off={:?} scale={:?} screen={:?} mode={} badge={} overlay={} {}",
        offset,
        scale,
        screen,
        mode_label,
        badge,
        overlay.digest(),
        tooltip
            .map(|tt| tt.digest())
            .unwrap_or_else(|| "hover=-".to_string())
    )
}

/// Draw the active-mode HUD (top-left) — byte-identical to the old `draw_mode_badge`.
fn draw_mode_badge(fd: &mut FrameDraw, label: &str, badge: &str) {
    if label.is_empty() {
        return;
    }
    let (bx, by, bw, bh) = (12.0_f32, 64.0_f32, 250.0_f32, 46.0_f32);
    if let Some(t) = fd.textures.get(fd.renderer, "GFX_generic_box_96") {
        fd.renderer
            .draw_sprite_frame(t, bx, by, bw, bh, 0, 1, [0.10, 0.12, 0.16, 0.85]);
    }
    if let Some(loaded) = fd.fonts.get(fd.renderer, "hoi_18mbs") {
        let font = loaded.font.clone();
        let pages = &loaded.pages;
        let l1 = format!("MAP: {label}");
        fd.renderer.draw_text(
            &font,
            pages,
            &l1,
            bx + 8.0,
            by + 6.0,
            [0.92, 0.94, 0.98, 1.0],
        );
        let badge_color = match badge {
            "exact" => [0.45, 0.85, 0.50, 1.0],
            "derived" => [0.50, 0.70, 0.95, 1.0],
            "proxy" => [0.95, 0.75, 0.35, 1.0],
            "unavailable" => [0.95, 0.45, 0.45, 1.0],
            _ => [0.80, 0.80, 0.80, 1.0],
        };
        let l2 = format!("[{badge}]");
        fd.renderer
            .draw_text(&font, pages, &l2, bx + 8.0, by + 26.0, badge_color);
    }
}

static MAP_OVERLAY_STATIC: OnceLock<OverlayStatic> = OnceLock::new();

fn overlay_static(
    map_dir: &std::path::Path,
    raster: &ProvinceRaster,
) -> Option<&'static OverlayStatic> {
    if let Some(s) = MAP_OVERLAY_STATIC.get() {
        return Some(s);
    }
    match OverlayStatic::load(map_dir, raster) {
        Ok(s) => {
            let _ = MAP_OVERLAY_STATIC.set(s);
            MAP_OVERLAY_STATIC.get()
        }
        Err(e) => {
            eprintln!("[map-overlay] unavailable: {e}");
            None
        }
    }
}

fn overlay_names(ctx: &BindContext, static_data: &OverlayStatic) -> OverlayNames {
    let mut states = BTreeMap::new();
    for s in &static_data.states {
        if !s.name_key.is_empty() {
            states.insert(s.state_id, ctx.stage.loc.resolve(&s.name_key));
        }
    }
    let mut strategic_regions = BTreeMap::new();
    for r in &static_data.regions {
        if !r.name_key.is_empty() {
            strategic_regions.insert(r.region_id, ctx.stage.loc.resolve(&r.name_key));
        }
    }
    let mut countries = BTreeMap::new();
    if let Some(state_obj) = ctx
        .snapshot()
        .domain("states")
        .and_then(serde_json::Value::as_object)
    {
        for state in state_obj.values() {
            if let Some(tag) = state.get("owner").and_then(serde_json::Value::as_str) {
                countries
                    .entry(tag.to_string())
                    .or_insert_with(|| resolve_country_label(ctx.snapshot(), &ctx.stage.loc, tag));
            }
        }
    }
    OverlayNames {
        countries,
        states,
        strategic_regions,
    }
}

fn resolve_country_label(snap: &snapshot::Snapshot, loc: &gui_runtime::Loc, tag: &str) -> String {
    if let Some(country) = snap.country(tag) {
        for field in ["display_name", "localized_name", "name"] {
            if let Some(name) = country.get(field).and_then(serde_json::Value::as_str) {
                let resolved = loc.resolve(name);
                if !resolved.trim().is_empty() {
                    return resolved;
                }
            }
        }
    }
    let resolved = loc.resolve(tag);
    if resolved.trim().is_empty() {
        tag.to_string()
    } else {
        resolved
    }
}

fn draw_map_overlay(fd: &mut FrameDraw, overlay: &OverlayPlan) {
    if overlay.lines.is_empty() && overlay.labels.is_empty() && overlay.markers.is_empty() {
        return;
    }
    let (w, h) = fd.screen();
    let clip = Some([0.0, 0.0, w as f32, h as f32]);
    let white = widgets::common::white_texture(fd.renderer);
    for line in &overlay.lines {
        for pair in line.points.windows(2) {
            draw_overlay_segment(
                fd.renderer,
                &white,
                pair[0],
                pair[1],
                line.width,
                line.tint,
                line.style,
                clip,
            );
        }
        if line.arrow {
            draw_overlay_arrowhead(fd, &white, line, clip);
        }
    }

    for marker in &overlay.markers {
        draw_unavailable_marker(fd, &white, marker.x, marker.y, marker.tint, clip);
    }

    if let Some(loaded) = fd.fonts.get(fd.renderer, "hoi_16mbs") {
        let font = loaded.font.clone();
        let pages = &loaded.pages;
        for marker in &overlay.markers {
            let shadow = [0.01, 0.015, 0.02, 0.86];
            fd.renderer.draw_text_clipped(
                &font,
                pages,
                &marker.text,
                marker.x + 9.0,
                marker.y - 6.0,
                shadow,
                clip,
            );
            fd.renderer.draw_text_clipped(
                &font,
                pages,
                &marker.text,
                marker.x + 8.0,
                marker.y - 7.0,
                marker.tint,
                clip,
            );
        }
        for label in &overlay.labels {
            let shadow = [0.01, 0.015, 0.02, 0.82];
            fd.renderer.draw_text_clipped(
                &font,
                pages,
                &label.text,
                label.x + 1.0,
                label.y + 1.0,
                shadow,
                clip,
            );
            fd.renderer.draw_text_clipped(
                &font,
                pages,
                &label.text,
                label.x,
                label.y,
                label.tint,
                clip,
            );
        }
    }
}

fn draw_overlay_segment(
    r: &mut render::Renderer,
    white: &render::Texture,
    from: ScreenPoint,
    to: ScreenPoint,
    width: f32,
    tint: [f32; 4],
    style: OverlayLineStyle,
    clip: Option<[f32; 4]>,
) {
    for (a, b) in overlay_segment_spans(from, to, style) {
        widgets::common::line_seg(r, white, (a.x, a.y), (b.x, b.y), width, tint, clip);
    }
}

fn overlay_segment_spans(
    from: ScreenPoint,
    to: ScreenPoint,
    style: OverlayLineStyle,
) -> Vec<(ScreenPoint, ScreenPoint)> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if !len.is_finite() || len < 0.5 {
        return Vec::new();
    }
    if style.pattern == OverlayLinePattern::Solid {
        return vec![(from, to)];
    }
    let dash = if style.dash_px.is_finite() && style.dash_px > 0.0 {
        style.dash_px
    } else {
        len
    };
    let gap = if style.gap_px.is_finite() && style.gap_px >= 0.0 {
        style.gap_px
    } else {
        0.0
    };
    let step = dash + gap;
    if step <= 0.0 || !step.is_finite() {
        return vec![(from, to)];
    }
    let mut start = 0.0_f32;
    let mut spans = Vec::new();
    while start < len {
        let end = (start + dash).min(len);
        let a = start / len;
        let b = end / len;
        spans.push((
            ScreenPoint {
                x: from.x + dx * a,
                y: from.y + dy * a,
            },
            ScreenPoint {
                x: from.x + dx * b,
                y: from.y + dy * b,
            },
        ));
        start += step;
    }
    spans
}

fn draw_unavailable_marker(
    fd: &mut FrameDraw,
    white: &render::Texture,
    x: f32,
    y: f32,
    tint: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let r = 6.0_f32;
    widgets::common::line_seg(
        fd.renderer,
        white,
        (x - r, y - r),
        (x + r, y + r),
        2.2,
        tint,
        clip,
    );
    widgets::common::line_seg(
        fd.renderer,
        white,
        (x - r, y + r),
        (x + r, y - r),
        2.2,
        tint,
        clip,
    );
}

fn draw_overlay_arrowhead(
    fd: &mut FrameDraw,
    white: &render::Texture,
    line: &render::overlay::OverlayLine,
    clip: Option<[f32; 4]>,
) {
    let Some((prev, end)) = final_segment(&line.points) else {
        return;
    };
    let dx = end.x - prev.x;
    let dy = end.y - prev.y;
    let len = (dx * dx + dy * dy).sqrt();
    if !len.is_finite() || len < 1.0 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let wing = 10.0 + line.width * 1.5;
    let spread = 0.48_f32;
    for sign in [-1.0_f32, 1.0_f32] {
        let rx = ux * spread.cos() - sign * uy * spread.sin();
        let ry = sign * ux * spread.sin() + uy * spread.cos();
        let p = ScreenPoint {
            x: end.x - rx * wing,
            y: end.y - ry * wing,
        };
        widgets::common::line_seg(
            fd.renderer,
            white,
            (end.x, end.y),
            (p.x, p.y),
            line.width,
            line.tint,
            clip,
        );
    }
}

fn final_segment(points: &[ScreenPoint]) -> Option<(ScreenPoint, ScreenPoint)> {
    let end = *points.last()?;
    for prev in points.iter().rev().skip(1) {
        let dx = end.x - prev.x;
        let dy = end.y - prev.y;
        if (dx * dx + dy * dy).sqrt() >= 1.0 {
            return Some((*prev, end));
        }
    }
    None
}

fn draw_overlay_badge(fd: &mut FrameDraw, d: &OverlayDiagnostics) {
    let (bx, by, bw, bh) = (12.0_f32, 116.0_f32, 360.0_f32, 48.0_f32);
    if let Some(t) = fd.textures.get(fd.renderer, "GFX_generic_box_96") {
        fd.renderer
            .draw_sprite_frame(t, bx, by, bw, bh, 0, 1, [0.08, 0.10, 0.13, 0.78]);
    }
    if let Some(loaded) = fd.fonts.get(fd.renderer, "hoi_16mbs") {
        let font = loaded.font.clone();
        let pages = &loaded.pages;
        let l1 = format!(
            "OVR labels C/S/R {}/{}/{}",
            d.country_labels, d.state_labels, d.strategic_region_labels
        );
        let l2 = format!(
            "lines M/P/T/F {}/{}/{}/{}  unavail A/R/F {}/{}/{}",
            d.movement_arrows,
            d.battle_plan_arrows,
            d.trade_route_lines,
            d.front_line_lines,
            d.unavailable_arrows,
            d.unavailable_routes,
            d.unavailable_front_lines
        );
        fd.renderer.draw_text(
            &font,
            pages,
            &l1,
            bx + 8.0,
            by + 6.0,
            [0.88, 0.90, 0.92, 1.0],
        );
        let c2 = if d.unavailable_arrows > 0
            || d.unavailable_routes > 0
            || d.unavailable_front_lines > 0
        {
            [0.95, 0.72, 0.35, 1.0]
        } else {
            [0.58, 0.84, 0.62, 1.0]
        };
        fd.renderer
            .draw_text(&font, pages, &l2, bx + 8.0, by + 26.0, c2);
    }
}

/// The factory is registry-required, but the map view is SEEDED by the binary (it needs
/// the leaked `'static MapData` + the startup camera). The binary opens it directly via
/// [`MapView::new`]; this factory is a guard that should not be reached through the menu
/// (it panics loudly rather than fabricate a 1×1 map).
fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    panic!("core.map is seeded by the binary at startup, not opened via the registry");
}

register_view!(ViewDescriptor {
    id: MAP_VIEW_ID,
    gui_root: None,
    // `weather` joins states/map: the map now drives its dynamic snow/mud layer from the
    // (optional) weather domain — absent → the static map, present → the per-hour feed.
    required_domains: &[
        "states",
        "map",
        "diplomacy_world",
        "provinces",
        "countries",
        "weather",
        "naval_air",
    ],
    route: None, // seeded at startup; the mode switcher is the map's own surface
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Background),
});

/// Keep the module linked (belt-and-suspenders; `inventory` works without it). No-op.
pub fn link() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_stage() -> crate::host::context::Stage {
        crate::host::context::Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest {
                sprites: std::collections::HashMap::new(),
            },
            loc: gui_runtime::Loc::new(),
            gui_files: std::collections::HashMap::new(),
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    fn test_context<'a>(
        stage: &'a crate::host::context::Stage,
        snap: &'a snapshot::Snapshot,
    ) -> BindContext<'a> {
        BindContext {
            stage,
            snapshot: snap,
            country: "GER",
            screen: gui_runtime::Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
            date: "1936.1.1.12",
            cursor: (50.0, 60.0),
            map_camera: None,
        }
    }

    #[test]
    fn parse_date_ymdh_components() {
        assert_eq!(parse_date_ymdh("1940.6.27.18"), (1940, 6, 27, 18));
        // a garbled/short date falls back to neutral summer-noon components (render cosmetic,
        // not a fabricated data value).
        assert_eq!(parse_date_ymdh(""), (1936, 7, 15, 12));
        assert_eq!(parse_date_ymdh("1942.3"), (1942, 3, 15, 12));
    }

    #[test]
    fn map_descriptor_lists_domains_used_by_render_modes() {
        let reg = crate::host::registry::ViewRegistry::from_inventory();
        let desc = reg.view(MAP_VIEW_ID).expect("core.map is registered");
        for domain in [
            "states",
            "map",
            "diplomacy_world",
            "provinces",
            "countries",
            "weather",
            "naval_air",
        ] {
            assert!(
                desc.required_domains.contains(&domain),
                "core.map descriptor must list {domain}"
            );
        }
    }

    #[test]
    fn map_hover_tooltip_surfaces_factual_pick_rows() {
        let stage = test_stage();
        let snap: snapshot::Snapshot =
            serde_json::from_value(serde_json::json!({ "date": "1936.1.1.12" })).unwrap();
        let ctx = test_context(&stage, &snap);
        let hover = MapHover {
            cursor: (50, 60),
            raster: (10, 11),
            province_id: 123,
            state_id: Some(64),
            owner: Some("GER".to_string()),
            state_name_key: Some("STATE_64".to_string()),
        };

        let digest = hover.tooltip_plan(&ctx).digest();
        assert!(digest.contains("Province #123"), "{digest}");
        assert!(digest.contains("#64"), "{digest}");
        assert!(digest.contains("STATE_64"), "{digest}");
        assert!(digest.contains("GER"), "{digest}");
        assert!(digest.contains("10,11"), "{digest}");
    }

    #[test]
    fn map_hover_tooltip_keeps_unresolved_land_metadata_unavailable() {
        let stage = test_stage();
        let snap: snapshot::Snapshot =
            serde_json::from_value(serde_json::json!({ "date": "1936.1.1.12" })).unwrap();
        let ctx = test_context(&stage, &snap);
        let hover = MapHover {
            cursor: (75, 80),
            raster: (3, 4),
            province_id: 999,
            state_id: None,
            owner: None,
            state_name_key: None,
        };

        let digest = hover.tooltip_plan(&ctx).digest();
        assert!(digest.contains("Province #999"), "{digest}");
        assert!(digest.contains(widgets::common::NOT_CAPTURED), "{digest}");
        assert!(!digest.contains("STATE_0"), "{digest}");
        assert!(!digest.contains("UNKNOWN"), "{digest}");
    }

    /// The map's weather wiring (mirroring `compute_dynamic`): a snapshot WITH measured
    /// province mud yields a NON-neutral, gated dynamic LUT; a snapshot WITHOUT weather is
    /// ungated (None) so the static map is preserved (no fabrication). This exercises the
    /// exact render path `compute_dynamic` uses, without a GPU.
    #[test]
    fn weather_feed_drives_dynamic_lut_and_gates_on_measured() {
        let num_provinces = 6usize;
        let date = parse_date_ymdh("1941.1.15.12");

        // WITH weather: province 2 is muddy → gated on, and its LUT mud byte (G channel) is 255.
        let weather = serde_json::json!({
            "provinces": { "2": { "mud": true }, "3": { "mud": false } }
        });
        let (inp, gated) =
            DynamicInputs::from_weather_snapshot(num_provinces, date, Some(&weather));
        assert!(gated, "measured province mud must gate the dynamic feed on");
        let lut = pack_province_dynamic_lut(&inp.provinces, inp.snow_mud_fade);
        // province 2 G-channel (mud) = 255; province 3 = 0 (measured no-mud).
        assert_eq!(lut[2 * 4 + 1], 255, "muddy province lights the mud channel");
        assert_eq!(lut[3 * 4 + 1], 0, "measured no-mud province stays dry");

        // WITHOUT weather: ungated → the map keeps the static path (compute_dynamic returns None).
        let (_, ungated) = DynamicInputs::from_weather_snapshot(num_provinces, date, None);
        assert!(
            !ungated,
            "absent weather must NOT perturb the map (static path preserved)"
        );
    }

    #[test]
    fn set_map_mode_command_changes_digest_label_without_faking_data() {
        let overlay = OverlayPlan::default();
        let mut active = MapMode::Political;
        let before = map_draw_digest(
            &[0, 1, 2, 3],
            None,
            [0.0, 0.0],
            [1.0, 1.0],
            (800, 600),
            &active.label(),
            "exact",
            &overlay,
            None,
        );

        let command = ViewCommand::SetMapMode {
            mode: MapMode::Weather,
        };
        assert!(apply_map_command_to_mode(&mut active, &command));
        let after = map_draw_digest(
            &[0, 1, 2, 3],
            None,
            [0.0, 0.0],
            [1.0, 1.0],
            (800, 600),
            &active.label(),
            "unavailable",
            &overlay,
            None,
        );

        assert_ne!(before, after, "active mode changes must perturb map digest");
        assert!(
            after.contains("mode=Weather badge=unavailable"),
            "mode label changes while the unavailable badge remains honest: {after}"
        );
        assert!(
            !apply_map_command_to_mode(&mut active, &command),
            "reapplying the active mode is a no-op"
        );
    }

    #[test]
    fn overlay_segment_spans_honor_line_style_metadata() {
        let a = ScreenPoint { x: 0.0, y: 0.0 };
        let b = ScreenPoint { x: 50.0, y: 0.0 };

        assert_eq!(
            overlay_segment_spans(a, b, OverlayLineStyle::solid()),
            vec![(a, b)]
        );

        let dashed = overlay_segment_spans(a, b, OverlayLineStyle::dashed(10.0, 5.0));
        let expected = vec![
            (
                ScreenPoint { x: 0.0, y: 0.0 },
                ScreenPoint { x: 10.0, y: 0.0 },
            ),
            (
                ScreenPoint { x: 15.0, y: 0.0 },
                ScreenPoint { x: 25.0, y: 0.0 },
            ),
            (
                ScreenPoint { x: 30.0, y: 0.0 },
                ScreenPoint { x: 40.0, y: 0.0 },
            ),
            (
                ScreenPoint { x: 45.0, y: 0.0 },
                ScreenPoint { x: 50.0, y: 0.0 },
            ),
        ];
        assert_eq!(dashed.len(), expected.len());
        for (actual, expected) in dashed.iter().zip(expected.iter()) {
            assert_screen_point_close(actual.0, expected.0);
            assert_screen_point_close(actual.1, expected.1);
        }
        assert_eq!(
            dashed,
            overlay_segment_spans(a, b, OverlayLineStyle::dashed(10.0, 5.0))
        );
    }

    fn assert_screen_point_close(actual: ScreenPoint, expected: ScreenPoint) {
        assert!(
            (actual.x - expected.x).abs() < 0.01,
            "{actual:?} != {expected:?}"
        );
        assert!(
            (actual.y - expected.y).abs() < 0.01,
            "{actual:?} != {expected:?}"
        );
    }
}
