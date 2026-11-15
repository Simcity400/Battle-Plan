use crate::map_views::CounterView;
use dashboard::protocol::CameraMode;
use std::path::Path;
use std::sync::OnceLock;
use viewer_combat as combat;
#[cfg(test)]
use viewer_foundation::combat_selection::Hit as CombatHit;
use viewer_foundation::combat_selection::Selection;
use viewer_foundation::map::camera::Placement;
use viewer_foundation::map::camera::Pose;
use viewer_foundation::map::camera::motion::{Input as MotionInput, Motion, Pan};
use viewer_foundation::map::free_icons;
use viewer_foundation::map::gpu_contract::SOFTWARE_RENDERER_NOTICE;
use viewer_foundation::map::native_gui::pointer::Sound;
use viewer_foundation::map::gpu_contract::device_requirements;
use viewer_foundation::map::gpu_contract::is_software;
use viewer_foundation::map::perspective::Choice;
use viewer_foundation::map::perspective::Country;
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::map::presentation::Target;
use viewer_foundation::map::province_pick::{motion_ground_point, pick, terrain_height};
use viewer_foundation::map::tables::Limitations;
use viewer_foundation::map::textures::IntelMap;
use viewer_foundation::map::ui_scale::PRESENTATION_FILTER;
use viewer_foundation::map::ui_scale::Raster;
use viewer_foundation::map::ui_scale::Setting;
use viewer_foundation::map::ui_scale::extent as ui_scale_extent;
use viewer_foundation::map::ui_scale::monitors as ui_scale_monitors;
use viewer_foundation::map::ui_scale::monitors_generation;
use viewer_foundation::map::ui_scale::points as ui_scale_points;
use viewer_foundation::map::ui_scale::refresh_monitors;
use viewer_foundation::map::ui_scale::to_interface;
use viewer_foundation::profile_writer::profile_print;
use viewer_foundation::profile_writer::profiling;
use viewer_foundation::profiling::hz;
use viewer_foundation::profiling::laps_line;
use viewer_foundation::profiling::sweep as profiling_sweep;
use viewer_foundation::retirement::Dropper;
use viewer_foundation::tooltip_text::Tooltip;
use viewer_foundation::unit_handle::Unit;
use viewer_world::map::scene::Scene;
use {
    crate::{
        Config, Options,
        document::{Document, InitialHour},
        hours, render,
    },
    viewer_foundation::map::{
        camera::{Camera, MapSize},
        frame::{DayNight, GameSpeeds},
    },
};
// names
use eframe::{egui, wgpu};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use dashboard::map_display::option_defaults;
use viewer_foundation::diagnostics_panel::tooltip_warning_lines;
use viewer_world::map::scene::SceneInput;
use {
    viewer_counters::map::counters::CounterReport,
    viewer_foundation::map::name_draw::Placements,
    viewer_foundation::map::sprites::Batch,
    viewer_world::map::labels::LabelReport,
    viewer_world::map::name_pass::{LayerFade, StrategicNames},
    viewer_world::map::scene::Overlay,
    viewer_world::map::scene::Request,
};

pub(crate) mod combat_rows;
mod drop;
mod embedded;
pub(crate) mod faction_highlight;

/// The political part's limitation lines: with the playhead's map open, the political, naval and
/// mode fills layers' own while the political layer draws (its refusal is the map's own line
/// otherwise); without it, `today`'s.
pub(crate) fn political_lines(side: Option<&crate::playhead_host::MapSide>, today: &[String]) -> Vec<String> {
    let Some(side) = side else { return today.to_vec() };
    match side.parts().filter(|_| side.draws("political")) {
        Some((_, layers)) => {
            let political = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::political::Political>()).flat_map(|layer| layer.warnings());
            let naval = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::naval::Naval>()).flat_map(|layer| layer.warnings());
            let modes = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::mode_fills::ModeFills>()).flat_map(|layer| layer.lines());
            political.chain(naval).chain(modes).collect()
        }
        None => Vec::new(),
    }
}

/// The lines the playhead's map gives in place of today's hour's: its limitation lines (the tables'
/// notes, the drawn layers' own with `raids`, the scripts reader's raid arrows, and each refused
/// layer), and the battle plan's, the map buildings' and the unit models' own reports, each from
/// its layer while it draws (the ships' within the units layer's while that draws).
pub(crate) fn map_lines(side: &crate::playhead_host::MapSide, raids: &[String]) -> [Vec<String>; 2] {
    let limitations = crate::playhead_host::limitations::lines(side, raids).unwrap_or_else(|| vec![MAP_LINES_LOADING.to_owned()]);
    let mut lines = Vec::new();
    if let Some((_, layers)) = side.parts() {
        let layer = |name: &str| layers.iter().find(|layer| layer.name() == name && side.draws(name));
        if let Some(orders) = layer("orders").and_then(|layer| layer.any().downcast_ref::<crate::layers::orders::Orders>()) {
            lines.extend(orders.warnings());
        }
        if let Some(buildings) = layer("buildings").and_then(|layer| layer.any().downcast_ref::<crate::layers::buildings::BuildingsFed>()) {
            lines.extend_from_slice(buildings.warnings());
        }
        match layer("units").and_then(|layer| layer.any().downcast_ref::<crate::layers::units::Units>()) {
            Some(units) => lines.extend(units.warnings()),
            None => {
                if let Some(ships) = layer("ships").and_then(|layer| layer.any().downcast_ref::<crate::layers::ships::Ships>()) {
                    lines.extend_from_slice(ships.warnings());
                }
            }
        }
        if let Some(aircraft) = layer("aircraft").and_then(|layer| layer.any().downcast_ref::<crate::layers::aircraft::Aircraft>()) {
            lines.extend(aircraft.warning());
        }
    }
    [limitations, lines]
}

/// The map's limitation line while its head has reached no hour.
pub(crate) const MAP_LINES_LOADING: &str = "Map limitations: loading, the playback head has reached no hour yet";
pub(crate) mod head_rows;
pub(crate) mod highlight;
pub(crate) mod laid_proof;
mod hover_report;
pub(crate) mod hover_rows;
pub(crate) mod fly;
mod fly_aircraft;
pub(crate) mod picker;
pub(crate) mod playback_cost;
mod presentation;
#[cfg(test)]
pub(crate) use presentation::{exact_capture_wait, grid_activation};
mod profile_playback;
#[cfg(test)]
pub(crate) mod readback;
mod walk;

/// Guard the README's frame-budget boundary at both production entry points.
#[cfg(test)]
#[test]
fn faction_hover_frame_path_uses_only_prepared_border_inputs() {
    let source = include_str!("app.rs");
    let body = source
        .split("\n    fn update_faction_highlight(")
        .nth(1)
        .unwrap()
        .split("\n    fn map_tooltip(")
        .next()
        .unwrap();
    for rebuild in ["political::regions(", "demilitarized_zone("] {
        assert!(
            !body.contains(rebuild),
            "faction hover reaches {rebuild} on the frame path"
        );
    }
    let source = include_str!("../../viewer-world/src/map/scene.rs");
    let body = source
        .split("\n    pub fn set_faction_highlight(")
        .nth(1)
        .or_else(|| {
            source
                .split("\n    pub(crate) fn set_faction_highlight(")
                .nth(1)
        })
        .unwrap()
        .split("pub fn demilitarized_edges(")
        .next()
        .unwrap();
    for rebuild in [
        "ClassPlan::matching(",
        "ZonePlan::matching(",
        "ZonePlan::custom_highlight(",
    ] {
        assert!(
            !body.contains(rebuild),
            "faction upload reaches {rebuild} on the frame path"
        );
    }
}

/// Escape, as the map-mode keys, reaches one dashboard map: the one owning keyboard input, or the
/// hovered one while none owns it. A lone map always takes it.
#[cfg(test)]
#[test]
fn escape_reaches_only_the_map_owning_its_keys() {
    let (a, b) = (egui::Id::new("map a"), egui::Id::new("map b"));
    assert!(takes_map_keys(true, Some(a), a, false));
    assert!(!takes_map_keys(true, Some(a), b, true));
    assert!(takes_map_keys(true, None, b, true));
    assert!(!takes_map_keys(true, None, a, false));
    assert!(takes_map_keys(false, Some(a), b, false));
    let source = include_str!("app.rs");
    let body = source.split("\n    pub(crate) fn map_ui(").nth(1).unwrap();
    assert!(
        body.contains("let escape = takes_keys && ui.input("),
        "Escape is read for every map"
    );
}

#[cfg(test)]
mod fly_render_tests;
#[cfg(test)]
mod initial_source_tests;
#[cfg(test)]
mod map_mode_fixture_tests;
#[cfg(test)]
mod map_state_tests;
#[cfg(test)]
mod warning_lines_tests;
#[cfg(test)]
mod render_failure_tests;
#[cfg(test)]
mod spine_tests;

/// The egui memory key of the dashboard map that owns keyboard input: the one last pressed.
const MAP_INPUT_OWNER: &str = "map camera input owner";

/// Whether this map takes the map-mode keys: a lone map always; on a dashboard, the map that
/// owns keyboard input, or the hovered one while no map owns it.
fn takes_map_keys(
    dashboard_map: bool,
    owner: Option<egui::Id>,
    me: egui::Id,
    hovered: bool,
) -> bool {
    !dashboard_map || owner.map_or(hovered, |owner| owner == me)
}

/// The window texture showing the scene's target, and the scene generation it shows.
#[derive(Debug, Default)]
struct Presented {
    texture: Option<egui::TextureId>,
    generation: u64,
}

impl Presented {
    /// Whether the scene's target must be registered or re-pointed: the scene made a new
    /// target, or the texture was freed with a retired map. A replacement scene counts its
    /// generations from the start again, so the generation alone cannot tell.
    fn stale(&self, generation: u64) -> bool {
        self.texture.is_none() || self.generation != generation
    }

    /// Gives up the texture for freeing, so the next frame registers the target afresh.
    fn take(&mut self) -> Option<egui::TextureId> {
        self.generation = 0;
        self.texture.take()
    }
}

#[cfg(test)]
#[test]
fn a_map_retired_and_shown_again_registers_its_new_target() {
    let mut presented = Presented::default();
    assert!(presented.stale(1));
    presented.texture = Some(egui::TextureId::User(7));
    presented.generation = 1;
    assert!(!presented.stale(1));
    assert!(presented.stale(2));
    // A resize past the map resource budget retires the map and frees its texture.
    assert_eq!(presented.take(), Some(egui::TextureId::User(7)));
    // Resizing back builds a new scene, whose first target is generation 1 again.
    assert!(presented.stale(1));
}

/// Profile captures replace only the map's hover point, in framebuffer
/// pixels. This supplies no OS pointer movement, button or camera input.
fn profile_hover_point(
    real: Option<[f32; 2]>,
    synthetic: Option<[f32; 2]>,
    enabled: bool,
    size: [u32; 2],
    ui_scale: f32,
) -> Option<[f32; 2]> {
    let point = match synthetic.filter(|_| enabled) {
        Some(point) => (point
            .into_iter()
            .zip(size)
            .all(|(p, size)| p.is_finite() && p >= 0.0 && p < size as f32))
        .then_some(point),
        None => real,
    };
    point.map(|point| to_interface(point, ui_scale))
}

/// Per-rebuild CPU laps of one window frame, printed with `BATTLE_PLAN_VIEWER_PROFILE`.
struct Laps {
    enabled: bool,
    frame_cpu: Option<viewer_foundation::profiling::ThreadSpan>,
    start: Instant,
    last: Instant,
    /// Milliseconds between the previous frame's laps ending and these
    /// starting, less the profiled pacing sleep.
    outside: f64,
    laps: Vec<(&'static str, f64)>,
}

/// When the previous frame's laps stopped, for [`Laps::outside`].
static FRAME_END: Mutex<Option<Instant>> = Mutex::new(None);

/// Marks the end of a frame's own path, so the next frame reports the gap.
fn mark_frame_end() {
    if let Ok(mut end) = FRAME_END.lock() {
        *end = Some(Instant::now());
    }
}

impl Laps {
    /// `waited` is the profiled pacing sleep, which is not a frame's cost; the
    /// rest of the gap since the previous frame ended is egui's own tessellation
    /// and paint and the present, which these laps do not cover.
    fn new(waited: Duration) -> Self {
        let enabled = profiling();
        viewer_foundation::profiling::begin_thread_frame();
        let frame_cpu = enabled.then(|| viewer_foundation::profiling::thread_span("frame"));
        let now = Instant::now();
        let outside = FRAME_END
            .lock()
            .ok()
            .and_then(|end| *end)
            .map_or(0.0, |end| {
                (now.saturating_duration_since(end).saturating_sub(waited)).as_secs_f64() * 1000.0
            });
        Self {
            enabled,
            frame_cpu,
            start: now,
            last: now,
            outside,
            laps: Vec::new(),
        }
    }

    /// Records the time since the previous lap under `name`.
    fn lap(&mut self, name: &'static str) {
        if self.enabled {
            let now = Instant::now();
            self.laps
                .push((name, now.duration_since(self.last).as_secs_f64() * 1000.0));
            self.last = now;
        }
    }

    fn total(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn line(&self) -> String {
        laps_line(&self.laps, 0.05)
    }
}

/// Milliseconds the frame's submission takes on the GPU, under
/// `BATTLE_PLAN_VIEWER_GPU_WAIT`.
///
/// Waiting out the submission is the only way to see the GPU's own time without
/// timestamp queries, and it costs the frame that wait, so an ordinary run and
/// a profiled run without the variable never call it and report 0.
fn gpu_wait(device: &wgpu::Device) -> f64 {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_GPU_WAIT").or_else(|| std::env::var_os("HOI4_VIEWER_GPU_WAIT")).is_some()) {
        return 0.0;
    }
    let start = Instant::now();
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    start.elapsed().as_secs_f64() * 1000.0
}

/// Holds a profiled capture's frames to a display's pace,
/// `BATTLE_PLAN_VIEWER_PROFILE_HZ` (60 unless set).
///
/// A `--window-dump` frame does not wait for a display, so unpaced it runs at
/// hundreds of frames a second and a sweep that advances the playhead by
/// `speed / hz` hours a frame outruns the builders, which work in wall time.
/// The wait falls before the frame's laps start, so it is not a frame's cost.
fn pace_profiled_frame() -> Duration {
    let period = Duration::from_secs_f64(1.0 / hz());
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let started = Instant::now();
    let Ok(mut last) = LAST.lock() else {
        return Duration::ZERO;
    };
    if let Some(previous) = *last {
        let due = previous + period;
        while let Some(left) = due.checked_duration_since(Instant::now()) {
            if left > Duration::from_millis(2) {
                std::thread::sleep(left - Duration::from_millis(2));
            } else {
                std::hint::spin_loop();
            }
        }
    }
    *last = Some(Instant::now());
    started.elapsed()
}

/// Where the host's clock is: the recorded hour it is in and how far past it.
///
/// The map draws the last hour the builders finished (`crate::hours`), which
/// is this one unless a seek or fast playback outran them; the playhead never
/// waits for the map.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Playhead {
    branch: replay_data::BranchId,
    hour: i32,
    fraction: f64,
}

/// The running clock as a rebuild of the same map left it ([`Viewer::reload_campaign_map`]).
#[derive(Clone, Copy, Debug)]
struct HeldClock {
    playhead: Option<Playhead>,
    playing: bool,
    anchor: Option<(Playhead, bool, f64)>,
    at: Instant,
}

/// What the window last presented of the playhead's map: its histories (day and night, the
/// counters' movement, the audio) follow today's continuity rule over these.
#[derive(Debug, Default)]
struct MapPresented {
    /// Branch, drawn hour and viewing country of the last picture presented.
    last: Option<(replay_data::BranchId, i32, Option<String>)>,
    /// The host's reset count when the window asked for a seek, until a picture composed after it
    /// is shown.
    seek: Option<u64>,
    /// A reader opened since: its first picture is a seek's.
    reopened: bool,
}

/// What the frame path keeps of the hour on screen once its layers are uploaded.
struct ShownHour {
    epoch: u64,
    branch: replay_data::BranchId,
    hour: i32,
    /// Why the hour draws nothing: a recorded gap or a failed builder part.
    error: Option<String>,
    political_warnings: Vec<String>,
    /// The battle plan's refusals and missing inputs ([`viewer_foundation::map::ribbon_content::OrderReport::warnings`]).
    plan_warnings: Vec<String>,
    /// The map buildings' refused type tokens
    /// ([`viewer_world::map::map_buildings::MapBuildingSummary::warnings`]).
    building_warnings: Vec<String>,
    /// The unit models' omitted stacks by reason
    /// ([`viewer_world::map::unit_models::ModelReport::warnings`]).
    model_warnings: Vec<String>,
    political_build: Duration,
    limitations: Limitations,
    /// What the weather table held, the snow and mud grid's size and how long
    /// it took to rasterise. Kept raw so ordinary viewing formats nothing.
    snow: (viewer_world::map::weather::Summary, [u32; 2], Duration),
    /// The viewing country's tag, built with the hour.
    header: Arc<hours::Header>,
    /// Worker-built regions, zone and ordinary plans for pointer highlights.
    borders: Option<(Arc<hours::Political>, Arc<hours::Borders>)>,
}

impl ShownHour {
    /// The hour's technical stats, shown only in diagnostics mode.
    fn stats(&self) -> [String; 2] {
        // Counted only when diagnostics show it, never on the hour's install.
        let (filled, distinct) = self
            .borders
            .as_ref()
            .map_or((0, 0), |(political, _)| political.layers.summary());
        let (summary, [width, height], build) = &self.snow;
        [
            format!(
                "Political layer: {filled} provinces filled in {distinct} country colors, region tables derived in {:.2} ms",
                self.political_build.as_secs_f64() * 1000.0
            ),
            format!(
                "Snow and mud: {}; {width}x{height} grid rasterised in {:.0} ms",
                summary.line(),
                build.as_secs_f64() * 1000.0
            ),
        ]
    }
}

/// Where [`map_overlay`] puts its box inside the map's `clip` rectangle, with
/// `points` egui points per interface pixel: an 8-pixel inset and at most 40%
/// of the map's height.
fn overlay_rect(clip: egui::Rect, points: f32) -> egui::Rect {
    let inset = 8.0 * points;
    egui::Rect::from_min_size(
        clip.min + egui::vec2(inset, inset),
        egui::vec2(
            (clip.width() - 2.0 * inset).max(1.0),
            (clip.height() * 0.4).max(1.0),
        ),
    )
}

/// Scales `style`'s text and spacing by `points` egui points per interface
/// pixel, so egui lays the text out at its final size rather than stretching it.
fn scale_style(style: &mut egui::Style, points: f32) {
    for font in style.text_styles.values_mut() {
        font.size *= points;
    }
    let spacing = &mut style.spacing;
    spacing.item_spacing *= points;
    spacing.button_padding *= points;
    spacing.interact_size *= points;
    spacing.icon_width *= points;
    spacing.icon_width_inner *= points;
    spacing.icon_spacing *= points;
    spacing.indent *= points;
    let scroll = &mut spacing.scroll;
    scroll.bar_width *= points;
    scroll.bar_inner_margin *= points;
    scroll.bar_outer_margin *= points;
    scroll.floating_width *= points;
    scroll.floating_allocated_width *= points;
    scroll.handle_min_length *= points;
}

/// Draws a translucent box over the top of a native map's `clip` rectangle for
/// its loading status and diagnostics. `add` gets the height its lists may use.
///
/// The box is part of the map's screen-space interface, so like the map's
/// tooltip (`viewer_foundation::map::ui_scale`) it is laid out in interface pixels: `points` is
/// egui points per interface pixel, the UI scale over the pixels per point.
pub(crate) fn map_overlay(
    ui: &mut egui::Ui,
    clip: egui::Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    points: f32,
    add: impl FnOnce(&mut egui::Ui, f32),
) {
    let rect = overlay_rect(clip, points);
    let mut overlay = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    overlay.set_clip_rect(clip);
    scale_style(overlay.style_mut(), points);
    egui::Frame::new()
        .fill(egui::Color32::from_black_alpha(220))
        .inner_margin(egui::Margin::same(6) * points)
        .show(&mut overlay, |ui| {
            add(ui, (rect.height() - 50.0 * points).max(24.0 * points))
        });
}

/// How to choose where a fly or walk mode chosen from the map starts.
fn start_prompt(mode: CameraMode) -> &'static str {
    match mode {
        CameraMode::Walk => {
            "Click the map where the walk starts, or press A on a controller to start in the              middle of the view. Escape goes back to the map."
        }
        _ => {
            "Click the map where the flight starts, or press A on a controller to start in the              middle of the view. Escape goes back to the map."
        }
    }
}

/// The capture mode of a recording that leaves tables out, and what it leaves out.
pub(crate) fn capture_mode_notice(ui: &mut egui::Ui, replay: &replay::campaign::Replay) {
    let mode = replay.capture_mode;
    if let Some(omits) = replay_data::scope::omits(mode) {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("{} recording:", mode_name(mode)),
            );
            ui.colored_label(egui::Color32::YELLOW, format!("not recorded: {omits}."));
        });
    }
}

fn mode_name(mode: replay_data::scope::CaptureMode) -> &'static str {
    match mode {
        replay_data::scope::CaptureMode::Full => "Full",
        replay_data::scope::CaptureMode::Map => "Map",
    }
}

pub(crate) fn partial_notice(ui: &mut egui::Ui, replay: &replay::campaign::Replay) {
    if let Some(replay::spool::ControlRecord::Partial { reason, .. }) = replay.partial() {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::YELLOW, "Partial recording:");
            ui.colored_label(egui::Color32::YELLOW, reason);
        });
    }
}

/// The ids of the shared hour layers the scene holds (`hours::Shared`), so an
/// hour that shares one with the hour on screen uploads nothing for it.
#[derive(Clone, Default)]
struct Uploaded {
    /// The shared products of the hour on screen, which another view of that hour takes.
    products: Vec<hours::Held>,
    political: Option<u64>,
    borders: Option<u64>,
    rails: Option<u64>,
    buildings: Option<u64>,
    /// Last snow/mud snapshot handed to the scene. Its graphics refresh/fade
    /// continues locally after application; equal snapshots retain that history.
    snow_mud: Option<Arc<viewer_world::map::weather::SnowMud>>,
    /// The intel map on the GPU; hours with the same fog share one.
    intel: Option<Arc<IntelMap>>,
    /// The hour's battle plan, which the frame uploads once with its air orders; hours with the
    /// same plan tables share one (`viewer_counter_products::map::orders::TABLES`).
    orders: Option<Arc<viewer_foundation::map::ribbon_content::OrderField>>,
    /// What the scene's order parts were last given from, and the air orders as last highlighted.
    order_key: Option<OrderKey>,
    air_field: Option<((usize, Vec<u64>), Arc<viewer_foundation::map::ribbon_content::OrderField>)>,
}

/// What the scene's order parts are given from: the plan, the raid arrows and the trade routes by
/// their producers' allocation and product, and the air orders by theirs and the bases they
/// highlight. A new one hands the scene the parts, which writes only what changed in them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OrderKey {
    pub plan: usize,
    pub raids: usize,
    pub routes: Option<u64>,
    pub air: Option<(usize, Vec<u64>)>,
}

/// An order part's allocation, none as zero.
pub(crate) fn order_at<T>(held: Option<&Arc<T>>) -> usize {
    held.map_or(0, |held| Arc::as_ptr(held) as usize)
}

/// The order parts the map's layers give: the orders layer's plan, the hour's raid arrows (made
/// with the overlay's tables at the head) and, while the convoys and units draw, the trade routes'
/// product.
struct LayeredOrders {
    plan: Option<Arc<viewer_foundation::map::ribbon_content::OrderField>>,
    raids: Option<Arc<viewer_foundation::map::ribbon_content::OrderField>>,
    routes: Option<u64>,
}

/// Asks the desktop host to show or hide diagnostics with Ctrl, since F3 alone selects the air
/// map mode as in the game; the desktop's web interface uses the same keys.
const DIAGNOSTICS_KEY: egui::Key = egui::Key::F3;

/// Cycles the full map's camera modes, as the controller's view button does. No interface
/// shortcut in the game's content binds it.
const CAMERA_MODE_KEY: egui::Key = egui::Key::F12;

/// Whether `event` is the diagnostics shortcut: a first press of the key with
/// Ctrl alone, as the web interface's handler takes it.
fn is_diagnostics_shortcut(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if *key == DIAGNOSTICS_KEY && modifiers.matches_exact(egui::Modifiers::CTRL)
    )
}

// capture
/// A readback of the window's map target, pending until a frame has been drawn.
///
/// The window does not draw the map into the swap chain: it renders it through
/// [`Scene`] into the same offscreen `OUTPUT_FORMAT` texture a `--dump` renders
/// into, and only hands egui the finished texture. That is the 8-bit graded
/// output and never the float scene behind it (`viewer_foundation::map::gpu_contract::Targets`), so egui
/// is handed exactly the bytes the readback writes. The surface format, the
/// MSAA resolve and egui's own colour handling sit downstream of every map pixel
/// and cannot change one, and reading that texture back with
/// [`render::write_png`] — the dump's own readback — is what makes the two paths
/// comparable without a screenshot tool.
///
/// What the two paths do not share is the animation clock, so a capture renders
/// its frame at `animation_time` 0 by default, or an explicit sequence clock.
struct Capture {
    path: PathBuf,
    /// Frames to draw before the readback. `--window-dump` draws the pair a
    /// headless dump draws, so the luminance adaptation has the same history.
    remaining: u32,
    /// `--window-dump`: the pixel size and camera the frame is pinned to, so
    /// neither the panel layout nor the display's scale factor decides them. A
    /// `None` camera is the fit to that size, which is the dump's default too.
    pinned: Option<([u32; 2], Option<Placement>)>,
    /// `--window-dump` closes the window once it has written its frame.
    exit: bool,
    time: Option<f32>,
    step: f32,
    frames: u32,
    index: u32,
    /// The harness's paced frames follow a wall clock, as a window's do: none waits for its hour,
    /// its icons or its raid cycle.
    paced: bool,
}

/// The open maps whose visual mods changed since they loaded, each with the revision it loaded
/// with: the campaign map, and dashboard maps by their source key.
pub(crate) struct VisualStale {
    campaign: Option<String>,
    maps: Vec<(crate::dashboard_maps::MapSourceKey, String)>,
    /// How many open maps showing visual mods were checked, and what could not be read.
    checked: usize,
    failures: Vec<String>,
}

struct LoadedReplay {
    document: Document,
    scene: Scene,
    revision: Option<String>,
    source: Option<Arc<dashboard::Source>>,
    decoder: Option<Arc<hours::SharedDecoder>>,
    config: Config,
}

fn seek_open_target(
    seek: &crate::embedded::Seek,
    accepted: Option<(&Path, Option<&str>)>,
    pending_load: Option<(&Path, Option<&str>)>,
) -> Option<crate::embedded::OpenRequest> {
    // An in-flight load or folder retry takes precedence over the old map.
    // An unknown native-picker revision cannot satisfy a host's exact target.
    let matches = pending_load
        .or(accepted)
        .is_some_and(|(path, revision)| seek.matches(path, revision));
    (!matches).then(|| seek.open_request())
}

/// The Match game scale and its note, keyed by the monitors' generation and
/// the document's content.
type MatchGame = ((u64, usize), (Raster, Option<String>));

/// What an overlay was laid out for: the hour, the camera and frame, the perspective and the
/// counter options.
#[derive(Clone, PartialEq)]
struct OverlayKey {
    occupation: Option<viewer_world::map::occupation_icons::Mode>,
    branch: replay_data::BranchId,
    hour: i32,
    camera: Camera,
    pixels: [u32; 2],
    ui_scale: f32,
    perspective: Choice,
    counter_settings: viewer_counters::map::counters::CounterSettings,
    selected: Arc<BTreeSet<Unit>>,
    air_selected: Arc<Vec<u64>>,
    air_bases: Arc<Vec<u64>>,
    fleets_selected: Arc<Vec<u64>>,
    hovered: Option<Unit>,
    /// The map mode's visibility, which a still camera must lay out again for.
    visibility: viewer_foundation::map::mode_visibility::Visibility,
    /// The Operatives map mode, whose intel icons the overlay then lays out.
    operatives: bool,
    /// The rows at the head the overlay's camera-bound inputs read: their hour and allocation.
    rows: Option<(replay_data::Hour, usize)>,
    /// The counter layers' products it was laid out with (their addresses, zero for a kind
    /// today's tables draw).
    layered: [usize; 6],
}

/// What the open map's overlay tables are made from: the rows at the head (their branch, hour and
/// allocation), the perspective and the vision the vision layer shows, the engaged index's cover,
/// and what the layers draw instead.
#[derive(Clone, Debug, PartialEq)]
struct TablesKey {
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    rows: usize,
    perspective: Choice,
    vision: usize,
    engaged: bool,
    left_out: viewer_counters::map::counters::Kinds,
    labels: bool,
    /// What the view needs of the tables ([`overlay_need`]).
    need: viewer_counters::map::counters::Need,
}

/// An overlay laid out on the tables' worker from the rows its tables were made from, for the
/// camera and view of the frame that asked: the frame thread takes it whole.
struct Laid {
    key: OverlayKey,
    prepared: std::io::Result<crate::document::PreparedOverlay>,
    /// What the layout read that its rows do not hold.
    missed: Vec<replay_data::rows::TableId>,
    /// The memory it laid the counters out on.
    on: Arc<crate::map_views::Presentation>,
    /// The rows it was laid out from and the parts left to the map's layers ([`laid_proof`]).
    rows: Arc<replay_frame::frame::DecodedFrame>,
    left: crate::document::LeftToLayers,
    /// Under the gate, the view's counter layers' products when the worker was asked.
    asked: Option<Arc<crate::map_views::Presentation>>,
    /// The map's layers' products laid in with it.
    lay_ins: Arc<LayIns>,
    times: LaidTimes,
}

/// How long the worker's build took, part by part: in the pool's queue, the tables (with the
/// layers' products set in them), the layout (with those laid in).
struct LaidTimes {
    asked: Instant,
    queued: Duration,
    tables: Duration,
    layout: Duration,
    /// The tables' parts, nested inclusively, while frames are measured.
    parts: Vec<viewer_foundation::profiling::BuilderSpan>,
    /// The tables' thread CPU time: beside `tables`, what of its wall time the thread ran.
    tables_cpu: Option<Duration>,
}

impl LaidTimes {
    /// Counted for the frame that takes the build, in microseconds; `age` from the ask to the take.
    fn note(&self) {
        use viewer_foundation::map::redraws::note;
        let micros = |duration: Duration| u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        note("laid.queued_us", micros(self.queued));
        note("laid.tables_us", micros(self.tables));
        note("laid.layout_us", micros(self.layout));
        note("laid.age_us", micros(self.asked.elapsed()));
        viewer_foundation::view_record::add("overlay", "tables_built", 1);
        viewer_foundation::view_record::add("overlay", "tables_us", micros(self.tables));
        viewer_foundation::view_record::add("overlay", "worker_layout_us", micros(self.layout));
        // Each part's wall time in microseconds, under its span's name.
        if let Some(cpu) = self.tables_cpu {
            note("laid.tables_cpu_us", micros(cpu));
        }
        // A part's wall time well over its CPU time waited (a lock, a shared build) or lost its core.
        for part in &self.parts {
            note(part.name, (part.ms * 1000.0) as u64);
            viewer_foundation::view_record::add("overlay tables wall", part.name, (part.ms * 1000.0) as u64);
            if let Some(cpu_ms) = part.cpu_ms {
                viewer_foundation::view_record::add("overlay tables cpu", part.name, (cpu_ms * 1000.0) as u64);
            }
        }
    }
}

/// The map's layers' products the overlay lays in over the hour's own (labels, intel icons,
/// resource icons, occupation marks), taken when the worker is asked, so the layout it makes
/// holds them and the frame that installs it lays nothing in again.
#[derive(Default)]
pub(crate) struct LayIns {
    labels: Option<(Arc<map_display_data::landscape::Labels>, u32)>,
    intel: Option<Arc<viewer_raids::map::intel_icons::IntelIcons>>,
    resources: Option<(viewer_world::map::resource_icons::HourIcons, u64)>,
    occupation: Option<(viewer_world::map::occupation_icons::Marks, u64)>,
}

impl LayIns {
    fn of(side: Option<&crate::playhead_host::MapSide>) -> Self {
        let Some(side) = side else { return Self::default() };
        Self {
            labels: crate::layers::labels::drawn(side),
            intel: crate::layers::intel_icons::drawn(side),
            resources: crate::layers::resource_icons::drawn(side).map(|layer| (layer.icons(), layer.version())),
            occupation: crate::layers::occupation_marks::drawn(side).map(|layer| (layer.marks(), layer.version())),
        }
    }

    /// Sets the labels and intel icons in `tables`, as the frame does after installing them.
    fn set(&self, tables: &mut crate::document::OverlayTables) {
        if let Some((labels, ours)) = &self.labels {
            tables.set_labels(labels, *ours);
        }
        if let Some(intel) = &self.intel {
            tables.set_intel(intel);
        }
    }

    /// Lays the resource icons and occupation marks in, as the frame's `lay_in_layers` does.
    pub(crate) fn lay(&self, prepared: &mut crate::document::PreparedOverlay, hour: &crate::document::Hour<'_>, camera: Camera, pixels: [u32; 2], ui_scale: f32, ours: u32) {
        if let Some((icons, version)) = &self.resources {
            prepared.replace_resource_icons(hour, camera, pixels, ui_scale, icons, *version);
        }
        if let Some((marks, version)) = &self.occupation {
            prepared.replace_occupation_icons(hour, camera, pixels, ui_scale, ours, marks, *version);
        }
    }
}

impl OverlayKey {
    /// The same camera and view: the two differ at most in the hour and rows they stand for and in
    /// the counter layers' products.
    fn same_view(&self, other: &Self) -> bool {
        let other = OverlayKey { branch: self.branch, hour: self.hour, rows: self.rows, layered: self.layered, ..other.clone() };
        *self == other
    }

    /// The same in all but the counter layers' products: only the counters are laid out again.
    fn same_but_layered(&self, other: &Self) -> bool {
        let layered = self.layered;
        let other = OverlayKey { layered, ..other.clone() };
        *self == other
    }
}

/// What asks for the overlay `now` against the one laid out `at`, by kind: the first, the hour
/// (the rows at the head), the camera, the view (perspective, options, selections, hover, mode),
/// and each counter layer's product (in [`viewer_counters::map::counters::Layered::identity`]'s
/// order).
fn overlay_asks(at: Option<&OverlayKey>, now: &OverlayKey) -> Vec<&'static str> {
    const LAYERED: [&str; 6] = [
        "asked.layers.land",
        "asked.layers.land_markers",
        "asked.layers.naval_markers",
        "asked.layers.bases",
        "asked.layers.guns",
        "asked.layers.air",
    ];
    let Some(at) = at else {
        return vec!["asked.first"];
    };
    let mut asks = Vec::new();
    if (at.branch, at.hour, at.rows) != (now.branch, now.hour, now.rows) {
        asks.push("asked.hour");
    }
    if (at.camera, at.pixels, at.ui_scale.to_bits()) != (now.camera, now.pixels, now.ui_scale.to_bits()) {
        asks.push("asked.camera");
    }
    if at.perspective != now.perspective
        || at.counter_settings != now.counter_settings
        || at.selected != now.selected
        || at.air_selected != now.air_selected
        || at.air_bases != now.air_bases
        || at.fleets_selected != now.fleets_selected
        || at.hovered != now.hovered
        || at.visibility != now.visibility
        || at.operatives != now.operatives
        || at.occupation != now.occupation
    {
        asks.push("asked.view");
    }
    asks.extend(
        LAYERED
            .iter()
            .zip(at.layered.iter().zip(&now.layered))
            .filter(|(_, (at, now))| at != now)
            .map(|(ask, _)| *ask),
    );
    asks
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MapHit {
    AirMission(usize),
    NavalMission(usize),
    NavalRegionHover(usize),
    Raid,
    /// An intel entry of the Operatives map mode: its tooltip replaces the province's.
    Intel,
    /// An occupation percentage or one definition badge, with its own control geometry.
    Occupation(usize),
    Adjacency(usize),
    Combat(usize),
    NavalObserver(usize),
    PortTooltip(usize),
    Unit(usize),
    Base(usize),
    Pin(usize),
    Blocked,
}

/// Prepared controls retain their own text and footprint, including controls
/// with no click action. An absent text still leaves their hit blocking the map.
fn prepared_naval_tooltip(
    report: &CounterReport,
    target: Option<MapHit>,
) -> Option<(Arc<Tooltip>, [f32; 4])> {
    let (text, rect) = match target? {
        MapHit::NavalMission(index) => {
            let hit = report.naval_mission_hits.get(index)?;
            (hit.text.as_ref()?, hit.rect)
        }
        MapHit::NavalRegionHover(index) => {
            let hit = report.naval_region_hovers.get(index)?;
            (hit.text.as_ref()?, hit.rect)
        }
        _ => return None,
    };
    Some((Arc::clone(text), rect))
}

/// Only the actual topmost base may activate a radar range.
fn radar_province(
    target: Option<MapHit>,
    report: &viewer_counters::map::counters::CounterReport,
) -> Option<u32> {
    let MapHit::Base(index) = target? else {
        return None;
    };
    report.radar_hits.get(&index).copied()
}

fn land_hover_matches_unit(
    hover: &viewer_foundation::map::counter_hovers::land::CounterHover,
    unit: &viewer_foundation::map::counter_hovers::unit_hits::Hit,
) -> bool {
    let kind = if hover.railway_gun {
        viewer_foundation::map::counter_hovers::unit_hits::Kind::RailwayGun
    } else {
        viewer_foundation::map::counter_hovers::unit_hits::Kind::Division
    };
    unit.kind == kind && unit.members == hover.members
}

/// Follow document::compose's GUI passes on equal native layer keys. Within
/// a combat window, the actual Part attachment order decides which observer wins.
/// The map icon under `point`: the top of the drawn hit shapes, by layer and pass. `raids` are
/// the drawn raid icons' hit shapes. `looks` are the icons a free eye shows: the nearest shown
/// icon whose own layout holds the pointer wins, and icons it does not show take no hits.
fn top_map_hit(
    point: [f32; 2],
    report: &viewer_counters::map::counters::CounterReport,
    raids: &[viewer_raids::map::raids::tooltips::Hit],
    occupation: &[viewer_world::map::occupation_icons::Hit],
    looks: Option<&free_icons::Looks>,
    intel: Option<&viewer_raids::map::intel_icons::IntelHit>,
) -> Option<MapHit> {
    use free_icons::IconFamily;
    // Where the pointer falls in the layout of the icon of `family` keyed `layer`, and that
    // icon's draw rank.
    let seen = |family: IconFamily, layer: u64| match looks {
        None => Some((point, 0)),
        Some(looks) => looks.hit(family, layer, point),
    };
    let mut top = None;
    let mut offer = |rank: u32, layer, pass, order, target| {
        let key = (rank, layer, pass, order);
        if top.as_ref().is_none_or(|(previous, _)| key >= *previous) {
            top = Some((key, target));
        }
    };
    for (index, hit) in report.faction_pin_hovers.iter().enumerate() {
        if let Some((point, rank)) = seen(IconFamily::Pins, hit.layer)
            && hit.contains(point)
        {
            offer(rank, hit.layer, 0, index, MapHit::Pin(index));
        }
    }
    for (index, hit) in report.base_hits.iter().enumerate() {
        let family = report
            .air_grid_hits
            .get(&index)
            .map_or(IconFamily::Bases, |hit| hit.family);
        if let Some((point, rank)) = seen(family, hit.layer)
            && hit.contains(point)
        {
            offer(
                rank,
                hit.layer,
                if report.air_grid_hits.contains_key(&index) {
                    8
                } else {
                    1
                },
                index,
                MapHit::Base(index),
            );
        }
    }
    for (index, hit) in report.air_mission_hits.iter().enumerate() {
        if let Some((point, rank)) = seen(IconFamily::Missions, hit.layer)
            && hit.contains(point)
        {
            offer(rank, hit.layer, 2, index, MapHit::AirMission(index));
        }
    }
    for (index, hit) in occupation.iter().enumerate() {
        if let Some((point, rank)) = seen(IconFamily::Occupation, hit.layer)
            && hit.contains(point)
        {
            offer(rank, hit.layer, 3, index, MapHit::Occupation(index));
        }
    }
    for (index, hit) in report.naval_mission_hits.iter().enumerate() {
        if let Some((point, rank)) = seen(IconFamily::Missions, hit.layer)
            && hit.contains(point)
        {
            offer(rank, hit.layer, 2, index, MapHit::NavalMission(index));
        }
    }
    for (index, hit) in report.naval_region_hovers.iter().enumerate() {
        if let Some((point, rank)) = seen(IconFamily::Missions, hit.layer)
            && hit.contains(point)
        {
            offer(
                rank,
                hit.layer,
                2,
                index,
                if hit.supported {
                    MapHit::NavalRegionHover(index)
                } else {
                    MapHit::Blocked
                },
            );
        }
    }
    for hit in raids {
        if let Some((point, rank)) = seen(IconFamily::Raids, hit.layer)
            && hit.contains(point)
        {
            offer(
                rank,
                hit.layer,
                4,
                0,
                if hit.supported {
                    MapHit::Raid
                } else {
                    MapHit::Blocked
                },
            );
        }
    }
    // Each intel child's CPU alpha test already ran.
    if let Some(hit) = intel
        && let Some((point, rank)) = seen(IconFamily::Intel, hit.layer)
        && hit.contains(point)
    {
        offer(rank, hit.layer, 4, 1, MapHit::Intel);
    }
    for (index, hit) in report.unit_hits.iter().enumerate() {
        if let Some((point, rank)) = seen(unit_family(report, index, hit), hit.layer)
            && hit.contains(point)
        {
            offer(
                rank,
                hit.layer,
                if hit.port { 1 } else { 5 },
                index,
                MapHit::Unit(index),
            );
        }
    }
    // The shared root handler reaches inherited children outside an unclipped
    // counter's layout-size box. Keep their plate's existing layer and draw order.
    for hover in &report.task_force_hovers {
        if let Some(hit) = report
            .unit_hits
            .get(hover.unit_hit)
            .filter(|hit| hover.matches(hit))
            && let Some((point, rank)) = seen(unit_family(report, hover.unit_hit, hit), hit.layer)
            && hover.contains(point)
        {
            offer(
                rank,
                hit.layer,
                5,
                hover.unit_hit,
                if hover.supported {
                    MapHit::Unit(hover.unit_hit)
                } else {
                    MapHit::Blocked
                },
            );
        }
    }
    for (index, hover) in report.task_force_port_hovers.iter().enumerate() {
        if let Some(hit) = report
            .unit_hits
            .get(hover.unit_hit)
            .filter(|hit| hover.matches(hit))
            && let Some((point, rank)) = seen(unit_family(report, hover.unit_hit, hit), hit.layer)
            && hover.contains(point)
        {
            let target = if !hover.supported || !hover.has_tooltip() {
                MapHit::Blocked
            } else if hover.control
                == viewer_foundation::map::counter_hovers::task_force_port::Control::Unbound
            {
                MapHit::PortTooltip(index)
            } else {
                MapHit::Unit(hover.unit_hit)
            };
            offer(rank, hit.layer, 1, hover.unit_hit, target);
        }
    }
    for (index, hit) in report.naval_marker_hovers.iter().enumerate() {
        let Some((point, rank)) = seen(IconFamily::Combats, hit.layer) else {
            continue;
        };
        if !hit.contains(point) {
            continue;
        }
        offer(
            rank,
            hit.layer,
            7,
            hit.order,
            if hit.supported {
                MapHit::NavalObserver(index)
            } else {
                MapHit::Blocked
            },
        );
    }
    // A clickable observer also appears in the naval tooltip list: the same
    // object and attachment order retains its click callback.
    for (index, hit) in report.combat_hits.iter().enumerate() {
        let Some((point, rank)) = seen(IconFamily::Combats, hit.layer) else {
            continue;
        };
        if !hit.contains(point) {
            continue;
        }
        let pass = if matches!(hit.selection, Selection::Land(_)) {
            6
        } else {
            7
        };
        offer(
            rank,
            hit.layer,
            pass,
            hit.order,
            if hit.supported {
                MapHit::Combat(index)
            } else {
                MapHit::Blocked
            },
        );
    }
    top.map(|(_, target)| target)
}

/// The icons a unit hit shape belongs to: a port's ships number with its base, other plates with
/// their column.
fn unit_family(
    report: &CounterReport,
    index: usize,
    hit: &viewer_foundation::map::counter_hovers::unit_hits::Hit,
) -> free_icons::IconFamily {
    if report.regional_naval_unit_hits.contains(&index) {
        free_icons::IconFamily::Missions
    } else if hit.port {
        free_icons::IconFamily::Bases
    } else {
        free_icons::IconFamily::Columns
    }
}

pub struct Viewer {
    dashboard: Option<Box<crate::dashboard_maps::DashboardMaps>>,
    dashboard_active: bool,
    dashboard_map: bool,
    /// Whether a host has applied the map's graphics settings ([`Self::set_embedded_graphics`]),
    /// as the desktop app does for its Campaign map: the picker's day/night and allied plans then
    /// ask it to flip them, as on a dashboard map. A viewer without a host has neither control.
    host_graphics: bool,
    shared_resources: Option<Arc<hours::Resources>>,
    source: Option<Arc<dashboard::Source>>,
    source_decoder: Option<Arc<hours::SharedDecoder>>,
    loaded_config: Option<Config>,
    rendered: Option<Playhead>,
    rendered_sequence: u64,
    /// When the latest frame was drawn, in milliseconds since the Unix epoch.
    drawn_at: Option<f64>,
    /// The dashboard view's options the frame being drawn applies, and those the latest frame drew with.
    applying_options: Option<dashboard::protocol::MapOptions>,
    drawn_options: Option<dashboard::protocol::MapOptions>,
    /// What the pointer highlights in the latest frame ([`hover_report::describe`]).
    hover: Option<String>,
    /// The map tooltip the latest frame drew, and the game's delay before one shows.
    tooltip: Option<String>,
    tooltip_delay_ms: Option<f32>,
    /// Milliseconds from planning a map hour to having it built, smoothed.
    build_latency_ms: Option<f64>,
    /// The newest hour the playing clock passed that could not be shown, and why.
    held_back: Option<(i32, String)>,
    /// The dashboard view this map draws, named in the perf log.
    perf_view: Option<String>,
    /// The last frame's data limitations, built only while they are shown or
    /// a capture will print them.
    map_warnings: Vec<String>,
    /// Whether technical stats and data limitations are shown with the map.
    /// Off by default: the desktop host sends its choice, and `--diagnostics`
    /// turns it on for one run.
    pub(crate) diagnostics: bool,
    /// Whether an embedded map's diagnostics box lists its data limitations.
    diagnostics_open: bool,
    /// This view's handle on the process's map audio, made by its first
    /// interactive frame; captures and dumps never make one.
    audio: Option<viewer_audio::Owner>,
    /// Whether this view may sound: the window's own map always, a dashboard
    /// map while it is the focused one.
    pub(crate) audio_focus: bool,
    /// The host's mute choice.
    pub(crate) muted: bool,
    /// A presentation discontinuity the next audio frame reports.
    pub(crate) audio_reset: bool,
    /// The presented hour's song chances for the viewing country.
    pub(crate) music: Option<Arc<viewer_audio::Chances>>,
    /// The presented hour's battles, for the battle ambience.
    pub(crate) battles: Option<Arc<viewer_audio::Battles>>,
    /// The map body took a press, drag or scroll this frame.
    pub(crate) interacted: bool,
    /// egui points per interface pixel at the last map frame, which lays out
    /// the diagnostics box over an embedded map ([`map_overlay`]).
    pub(crate) interface_points: f32,
    embedded: Option<crate::embedded::Controller>,
    /// The toggle harness's map service, standing in for the desktop host's ([`Self::service`]).
    harness_service: Option<Arc<dashboard::Service>>,
    embedded_open: Option<crate::embedded::OpenRequest>,
    embedded_seek: Option<crate::embedded::Seek>,
    revision: Option<String>,
    expected_revision: Option<String>,
    hidden_since: Option<Instant>,
    combat: combat::Window,
    config: Config,
    options: Options,
    /// Whether the map waits for usable folders from the host before it can
    /// load; an embedded viewer keeps its requested recording meanwhile.
    awaiting_folders: bool,
    document: Option<Document>,
    loading: Option<mpsc::Receiver<Result<LoadedReplay, String>>>,
    startup_content: Option<installed_content_core::content::Loading>,
    error: Option<String>,
    scene: Option<Scene>,
    scene_target: Presented,
    map_error: Option<String>,
    camera: Option<Placement>,
    camera_manual: bool,
    /// The scene builds in place for the frame a capture saves ([`Scene::settle_builds`]).
    settled_capture: bool,
    /// The recording a rebuild in place opens again: its next open keeps the camera, orbit,
    /// walk, flight and chosen start instead of starting over.
    held_view: Option<(PathBuf, HeldClock)>,
    /// The host clock a rebuild in place resumes once its load is installed: the host sends no
    /// anchor for it, so without this the map would stay paused while the host plays.
    held_clock: Option<HeldClock>,
    /// The game's zoom: its target camera and the wheel's part notch.
    zoom: Motion,
    /// How the full map's camera moves, as the host's control sets it.
    camera_mode: CameraMode,
    /// Whether the fly and walk modes draw the map's interface standing in the world, as the
    /// host's control sets it.
    free_overlays: bool,
    /// Where a fly or walk mode chosen from the map starts ([`drop`]).
    start_pick: drop::Pick,
    /// The overlay as a free eye shows it this frame ([`free_icons::present`]).
    free_overlay: Batch,
    /// Raised each time the overlay's batch may have been written: what [`Self::free_overlay`]
    /// was placed from.
    overlay_written: u64,
    /// What the free eye's placing last read (its camera, viewport, scale and the batch it
    /// placed): the same again places the same quads, so they stand.
    free_presented: Option<(viewer_foundation::map::camera::Camera, [u32; 2], u32, u64)>,
    /// The icons a free eye shows this frame, which the pointer's hits follow; `None` when the
    /// overlay draws as laid out.
    looks: Option<free_icons::Looks>,
    /// Keyboard, mouse and controller input for the camera controllers.
    input: crate::controls::Input,
    /// The controller's heading and tilt around the map camera.
    orbit: crate::controls::orbit::Orbit,
    /// The walking mode's figure, walker and scale.
    walk: walk::Walk,
    /// The flying mode's flight, started when the mode starts.
    flight: Option<crate::controls::flight::Flight>,
    /// The aircraft this frame's flight placed, as its library slot and instance.
    flying: Option<(usize, viewer_world::map::models::ModelInstance)>,
    /// The capture clock a window dump's flight last stepped to.
    fly_clock: Option<f32>,
    /// What a respawn under a low ceiling built for this map, kept for its next one.
    fly_footprints: crate::controls::flight::Footprints,
    /// Whether the window hides and confines the cursor for looking around.
    pointer_grabbed: bool,
    playing: bool,
    speed: f64,
    /// The host's last target, play state and rate: a repeat leaves the running clock alone.
    clock_anchor: Option<(Playhead, bool, f64)>,
    /// The drawn lighting, eased toward each presented hour as the game eases it.
    day_night: viewer_foundation::map::frame::DayNightPlayback,
    last_step: Instant,
    profile_playback: profile_playback::Clock,
    started: Instant,
    /// When the replay was opened, so a profiled run reports the cold open:
    /// the loading thread's document and scene, then the first hour built.
    opened: Option<Instant>,
    // colour
    /// Whether the map is graded through the engine's post chain.
    postfx: bool,
    pub(crate) graphics: dashboard::protocol::MapGraphics,
    /// Whose counters this view draws.
    pub(crate) counter_filter: dashboard::protocol::CounterFilter,
    /// The picker's player-only counters toggle: only the viewing country's own counters draw.
    pub(crate) player_counters: bool,
    /// This view's sound volumes. Only the focused map plays sound.
    pub(crate) volumes: dashboard::protocol::MapAudio,
    /// The overlay parts the last overlay draw used, with the counter filter resolved.
    overlay_layers: crate::document::OverlayLayers,
    /// The counter filter's countries at the shown hour.
    counter_owners: crate::map_views::CounterOwners,
    /// The host's settings for this view's scope.
    resolved_settings: crate::map_views::ResolvedSettings,
    /// The screen-space map interface's scale setting; see [`viewer_foundation::map::ui_scale`].
    /// The desktop host sets it with `MapAction::UiScale`.
    pub(crate) ui_scale: Setting,
    /// The aircraft the host chose for Fly mode and its bake.
    fly_aircraft: fly_aircraft::FlyAircraft,
    /// The Match game scale and its note, keyed by the monitors' generation
    /// and the document's content, so a frame only re-resolves on a change.
    match_game: Option<MatchGame>,
    /// The monitor size and display scale the window last saw; a change asks
    /// for the monitors to be read again.
    monitor_seen: Option<(Option<egui::Vec2>, Option<f32>)>,
    /// A running check of the visual mods the open maps show (`check_visual_mods`).
    visual_check: Option<std::sync::mpsc::Receiver<VisualStale>>,
    /// The last check's outcome, reported with the map details.
    visual_check_note: Option<String>,
    /// A check asked for while a map loaded, run again once every load is installed.
    visual_check_after_load: embedded::DeferredCheck,
    /// The visual mod revision the next open must not reuse loaded content of.
    stale_visual_revision: Option<String>,
    /// The owner of the Campaign map's visual mod fallback, shared with the dashboard's maps
    /// ([`crate::visual_fallback`]).
    pub(crate) visual_fallbacks: crate::visual_fallback::Shared,
    /// The last frame's report for the host's fallback owner ([`Self::take_visual_report`]).
    visual_report: Option<crate::visual_fallback::Report>,
    /// Whether the last `map` call drew the map ([`crate::visual_fallback::settled`]).
    drew_frame: bool,
    /// The playback-cost harness's layers-only run: today's hour pipeline is never started.
    pub(crate) layers_only: bool,
    /// The harness's warm-up: the map's first picture is composed and uploaded, the scene not
    /// rendered, so a run's first frame is the scene's first render, as without the warm-up.
    pub(crate) warming: bool,
    /// The revision a check found the Campaign map's visual mods changed from, reloaded once no
    /// load runs.
    visual_stale_campaign: Option<String>,
    map_preferences_seen: Option<(
        String,
        Option<Vec<installed_content_core::content::visual::Choice>>,
        u32,
        u8,
    )>,
    // hours
    /// The host clock's position; the map follows it as hours are built.
    playhead: Option<Playhead>,
    /// A seek waiting for the next built hour to start a fresh animation
    /// history (`viewer_foundation::map::presentation`).
    animation_seek: bool,
    /// The last host clock target, to tell its seeks from playback.
    map_animation_target: Option<Target>,
    /// The worker threads that build every hour's layers off the frame path.
    hours: Option<hours::Pipeline>,
    /// The map's reader of the playhead of the recording on screen, unless it was refused.
    playhead_map: Option<crate::playhead_host::MapSide>,
    /// A dashboard map opened its playhead reader since it was installed or last not drawn.
    playhead_opened: bool,
    /// The rows at the head (hour, frame) the overlay on screen was laid out from; none before
    /// its first layout over them.
    overlay_laid: Option<(replay_data::Hour, usize)>,
    /// The head tables (their rows' hour and frame) last given to the counter layers, and those
    /// they held when the reader last drew: what its settled picture shows of them.
    counters_given: Option<(replay_data::Hour, usize)>,
    counters_drawn: Option<(replay_data::Hour, usize)>,
    /// The document on screen has shown a whole picture since it was installed; until then the
    /// spinner covers what is drawn beneath it.
    shown_whole: bool,
    /// The map's reader being made on a worker; today's picture is drawn until then.
    playhead_opening: Option<crate::playhead_host::Opening>,
    /// What the window last presented of the playhead's map, for its own histories.
    presented: MapPresented,
    /// Today's pipeline's last error while the playhead's map is open, printed once.
    today_error: Option<String>,
    /// The engaged-flag index of the recording last closed, for a newer
    /// snapshot of it to continue.
    engaged_carry: Option<Arc<crate::engaged_index::EngagedIndex>>,
    /// The builders' generation the window keeps hours of.
    epoch: u64,
    /// Built hours not yet shown.
    ready: BTreeMap<(replay_data::BranchId, i32), hours::Built>,
    /// The hour whose layers are in the scene.
    shown: Option<ShownHour>,
    /// The camera the last frame drew, which the next hours' icons are built for.
    last_view: Option<hours::View>,
    /// Which shared layers the scene holds.
    uploaded: Uploaded,
    /// Frees the hours the window let go of off the frame path.
    dropper: Option<Arc<Dropper>>,
    /// The app's worker threads: the host's shared ones, else this window's own once first asked.
    worker_pool: Option<Arc<crate::worker_pool::WorkerPool>>,
    // close zoom
    unit_models: Option<Arc<viewer_world::map::unit_models::PreparedModels>>,
    /// While the units layer does not draw: the hour's unit models as made without ships, and the
    /// ships layer's number joined to them in `unit_models`.
    ships_shipless: Option<Arc<viewer_world::map::unit_models::PreparedModels>>,
    ships_joined: Option<u64>,
    /// Whether the ships layer stands in for today's ships and the aircraft layer draws its plan, as
    /// last seen; kept while the layers compose.
    movers_layered: (bool, bool),
    /// Whether the scene holds `unit_models`' field. It does not move within
    /// the hour, so each installed hour uploads it once.
    unit_field_uploaded: bool,
    /// The playback fraction the scene's raid units were last placed at.
    raid_tick: Option<f32>,
    /// Whether the scene's raid units last included sailing trade-route convoys, which move
    /// every frame by the presentation clock while they show.
    convoys_sailing: bool,
    /// The convoy entities that did not bake, as of the last frame that sailed them.
    convoy_refused: BTreeMap<String, usize>,
    // names
    placements: Option<Arc<Placements>>,
    placements_at: Option<(
        replay_data::BranchId,
        i32,
        viewer_world::map::map_mode::MapMode,
    )>,
    /// `None` until the first frame seeds it settled at that frame's own camera
    /// height; see [`Viewer::map`].
    fade: Option<LayerFade>,
    hovered_name_region: Option<u32>,
    last_frame: Instant,
    camera_step: Option<f32>,
    label_report: LabelReport,
    /// The raid offer cycle, advanced off the frame path.
    raid_lane: crate::raid_lane::RaidLane,
    faction_highlight: faction_highlight::Lane<faction_highlight::Prepared>,
    hover_rows: hover_rows::HoverRows,
    /// The selection observers, the overlay's camera-bound inputs, the walk camera, the clock's
    /// speed and the picker, over rows at the head while the map's playhead is on.
    head_rows: head_rows::HeadRows,
    /// With the playhead's map open: the rows at the head the overlay was last made from, kept
    /// while its reader loads, and the build of its tables from them.
    overlay_frame: Option<Arc<replay_frame::frame::DecodedFrame>>,
    overlay_tables_job: head_rows::tables::Job<TablesKey, (std::io::Result<crate::document::OverlayTables>, Option<Laid>)>,
    /// The counters' memory the tables' worker lays the overlay out on, over the view's schedule and
    /// movement ([`crate::map_views::Presentation::sharing`]), for the view it was made from.
    overlay_laying: Option<(Arc<crate::map_views::Presentation>, Arc<crate::map_views::Presentation>)>,
    /// What the overlay laid out last for the open map was laid out for ([`Self::install_overlay`]).
    overlay_shown: Option<OverlayKey>,
    /// The overlay key of the frame before, which the asks are counted against.
    overlay_last_asked: Option<OverlayKey>,
    /// The hour fraction the overlay was last drawn at.
    overlay_drawn_fraction: Option<f32>,
    /// The head tables' lookups by key counted so far: (tables, made, kept).
    overlay_lookups: Option<(usize, u64, u64)>,
    /// The equality gate of the overlay the worker lays out (`--laid-proof N`).
    pub(crate) laid_proof: Option<laid_proof::LaidProof>,
    /// The tables last made from rows at the head, which `overlay_tables` holds while the map is
    /// open (sharing every part); put back when today's install replaced them.
    head_tables: Option<(TablesKey, crate::document::OverlayTables)>,
    /// The overlay's reads that its rows at the head do not hold yet, by name.
    overlay_loading: Vec<String>,
    /// The tables the overlay's last layout read that its rows did not hold, until one is made.
    overlay_missed: Vec<replay_data::rows::TableId>,
    /// The raid offer cycle and music chances from rows at the head, while the playhead is on.
    scripts: crate::layers::scripts::Scripts,
    combat_rows: combat_rows::CombatRows,
    tooltip_refused: BTreeMap<String, usize>,
    // counters
    counter_report: CounterReport,
    /// What the map's last hover with a tooltip was ([`highlight::TooltipTarget`]).
    map_tooltip_target: Option<highlight::TooltipTarget>,
    /// The map tooltip the last drawn frame showed.
    shown_tooltip: Option<highlight::TooltipTarget>,
    /// Interface sounds since the last audio frame, and the next key one takes.
    interface_sounds: Vec<InterfaceSound>,
    interface_key: u64,
    /// The counter the last pointer step held in Over (`false`) or Down (`true`).
    counter_held: Option<(Unit, bool)>,
    /// What the held primary press went down on (its owner, or the bare map as `None`), where,
    /// when, and whether the pointer has since strayed too far for a click.
    pressed_on: Option<(Option<Holder>, egui::Pos2, f64, bool)>,
    /// The counter the held press went down on, whose release over it selects it.
    counter_selecting: Option<Unit>,
    /// The picker control the held secondary press went down on.
    secondary_on: Option<viewer_combat::picker::Control>,
    /// The last icon layer built, and the frame, camera and viewport it was
    /// built for; the two reports beside it come from the same build.
    ///
    /// The prepared layer reads the decoded frame, camera, viewport and
    /// perspective. The window repaints continuously for the
    /// water animation, so without this it rebuilt every counter, base, label
    /// and combat icon about forty times a second for a result that had not
    /// changed: 22 of a still frame's 27 ms. Keyed like `PoliticalCache` and
    /// the name placements beside it. Fraction-only changes reuse that preparation
    /// and advance the moving group anchors and sprite rectangles.
    overlay: Option<Batch>,
    prepared_overlay: Option<crate::document::PreparedOverlay>,
    overlay_fraction: Option<f32>,
    overlay_at: Option<OverlayKey>,
    /// This view's counter options (the user's `settings.txt`, with the host's graphics
    /// overrides) and its counters' memory from one update to the next; a new document
    /// starts it afresh.
    counter_view: CounterView,
    unit_selection: viewer_counters::map::unit_selection::Selection,
    air_selection: viewer_counters::map::air_selection::Selection,
    fleet_selection: viewer_counters::map::naval_selection::Selection,
    /// The hour's camera-independent icon tables, parsed once per (branch,
    /// hour, perspective) and reused by every frame's icon rebuild while the
    /// camera moves. A still camera never rebuilds at all (`overlay_at`), so
    /// this only pays on pans and zooms; without it each of those frames
    /// re-parsed every table the layout reads.
    overlay_tables: Option<(
        replay_data::BranchId,
        i32,
        Choice,
        crate::document::OverlayTables,
    )>,
    /// Which country the counters are drawn for; the host's map options set
    /// it and `Document::perspective` is kept in step with it.
    perspective: Choice,
    /// The view's map mode; the picker, its shortcuts and the host set it.
    map_mode: viewer_world::map::map_mode::MapMode,
    diplomatic_selected: Option<u32>,
    /// The map mode picker drawn over the map, its slots and what it tells the host.
    picker: picker::PickerState,
    picker_error: Option<String>,
    picker_refused: std::collections::BTreeMap<String, usize>,
    /// The controls the picker last drew and the extent it drew them at, which the next
    /// frame's input hits, as the user saw them.
    picker_controls: Option<([f32; 2], Vec<viewer_combat::picker::Placed>)>,
    // capture
    /// The readback to run once the next frame is drawn; a host export and
    /// `--window-dump` both arm it.
    capture: Option<Capture>,
    /// What the last capture wrote, or why it failed, for the map details.
    capture_note: Option<String>,
    /// How a `--window-dump` failure leaves the process; see `main::run`.
    failure: Arc<Mutex<Option<String>>>,
    /// Device and queue clones for the loading thread's GPU init. The loader
    /// builds the whole scene (static passes, textures and per-hour pass
    /// creation) while the builders build the first hour, so no frame ever
    /// pays first-time GPU creation. App-lifetime handles; eframe owns them.
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    profile_frame: u32,
    /// This frame's laps, started when the frame begins.
    laps: Option<Laps>,
}

/// Builds the whole hour-independent scene on the loading thread: the static
/// passes and textures plus every per-hour pass's creation. The builders need
/// seconds for the first hour and the UI thread idles on the spinner
/// meanwhile; paying the ~500 ms of pipeline and static-texture creation here
/// leaves the frames data uploads only. Hour data still uploads per hour
/// through the matching `set_*` as before, so pixels cannot change. Scenes of one content on
/// one device draw one upload of its game textures.
pub(crate) fn load_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    document: &Document,
    graphics: dashboard::protocol::MapGraphics,
    workers: &viewer_foundation::parallel::Workers,
) -> std::io::Result<Scene> {
    let _startup = crate::perf_log::StartupSpan::new("scene");
    let scene = engine_executable::executable_tables::scope(document.content.tables, || {
        load_scene_scoped(device, queue, document, graphics, workers)
    });
    if scene.is_err() {
        document.content.release_loading();
    }
    scene
}

fn load_scene_scoped(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    document: &Document,
    graphics: dashboard::protocol::MapGraphics,
    workers: &viewer_foundation::parallel::Workers,
) -> std::io::Result<Scene> {
    use viewer_foundation::load_record::stage;
    let textures = {
        let _stage = stage("scene.textures");
        document
            .scene_textures
            .warm(device, queue, &document.assets, &document.icons.images)?
    };
    let mut scene = {
        let _stage = stage("scene.passes");
        Scene::with_textures(
            device,
            queue,
            textures,
            &document.assets,
            &document.icons.lazy,
            graphics,
            workers,
        )?
    };
    let mut apply = |name: &'static str, input: SceneInput<'_>| {
        let _stage = stage(name);
        scene.apply(device, queue, input)
    };
    apply(
        "scene.animated_text",
        SceneInput::InstallAnimatedText {
            assets: &document.icons.animated_text,
            gui: &document.icons.counters.gui,
        },
    )?;
    apply(
        "scene.borders",
        SceneInput::SetBorders {
            assets: &document.border_assets,
            meshes: &document.border_meshes,
            straits: &document.straits,
        },
    )?;
    apply(
        "scene.rivers",
        SceneInput::SetRivers {
            assets: &document.river_assets,
            mesh: &document.river_mesh,
        },
    )?;
    apply(
        "scene.trees",
        SceneInput::SetTrees {
            assets: &document.tree_assets,
            geometry: document.tree_geometry.clone(),
            field: &document.tree_field,
        },
    )?;
    apply("scene.sprawl", SceneInput::SetSprawl(&document.sprawl))?;
    // The map's ambient objects and the nuclear impact entities share this pass,
    // as in a dump: their sounds and impacts need it in every window.
    apply("scene.map_frame", SceneInput::SetMapFrame(&document.frame_assets))?;
    apply("scene.trains", SceneInput::WarmTrains(&document.trains))?;
    apply("scene.aircraft", SceneInput::WarmAircraft(&document.aircraft))?;
    apply("scene.unit_models", SceneInput::WarmUnitModels(&document.unit_models))?;
    apply("scene.map_buildings", SceneInput::WarmMapBuildings(&document.map_buildings))?;
    apply("scene.bridges", SceneInput::WarmBridges(&document.bridges))?;
    apply("scene.railways", SceneInput::WarmRailways(&document.rail_assets))?;
    apply("scene.orders", SceneInput::WarmOrders(&document.orders))?;
    apply("scene.weather", SceneInput::WarmWeather(document.weather_effects.clone()))?;
    apply(
        "scene.political",
        SceneInput::WarmPolitical {
            map: &document.content.map,
            strategic_regions: &document.content.strategic_regions,
            field: &document.content.map_defines.gradient_field,
            cache: &document.content.political_gpu,
        },
    )?;
    let warm_started = Instant::now();
    apply("scene.pipelines", SceneInput::WarmPipelines)?;
    crate::perf_log::since("pipeline_warmup", warm_started);
    let warm_started = Instant::now();
    let icons = &document.icons;
    {
        let _stage = stage("scene.gui");
        scene.warm_gui(
            device,
            icons
                .counters
                .gui
                .effects()
                .into_iter()
                .chain(icons.combat.gui.effects()),
        );
    }
    crate::perf_log::since("gui_warmup", warm_started);
    Ok(scene)
}

impl Viewer {
    /// Lays the overlay out afresh from the current tables and draws it at
    /// this frame's fraction, on the view's own counter memory at the frame's
    /// presentation clock (the same hour and time again, so its map-icon windows
    /// advance nothing); an error unless its quads, raid report, label report and
    /// counter report equal those of the overlay with its raid icons installed in
    /// place.
    fn check_overlay(
        &mut self,
        now: &crate::document::Hour<'_>,
        camera: Camera,
        pixels: [u32; 2],
        ui_scale: f32,
        time: f32,
    ) -> Result<(), String> {
        let (Some((_, _, _, tables)), Some(prepared), Some(drawn)) = (
            self.overlay_tables.as_ref(),
            self.prepared_overlay.as_ref(),
            self.overlay.as_ref(),
        ) else {
            return Ok(());
        };
        // With the map open, the rows at the head the overlay was laid out from.
        let side = live_of(&self.playhead_map);
        let rows = crate::document::OverlayRows::at_head(self.overlay_frame.clone().filter(|_| side.is_some())).leaving(crate::document::LeftToLayers::of(side));
        let full = now.prepare_overlay_at(camera, pixels, ui_scale, tables, &self.counter_view, &Default::default(), &rows);
        if !rows.missed().is_empty() {
            return Ok(());
        }
        let mut full = full.map_err(|error| error.to_string())?;
        lay_in_layers(side, &mut full, now, camera, pixels, ui_scale, tables.ours());
        let (mut batch, mut report) = (Batch::default(), CounterReport::default());
        full.draw_into(
            now,
            camera,
            pixels,
            ui_scale,
            &self.overlay_layers,
            time,
            &Default::default(),
            &mut batch,
            &mut report,
        )
        .map_err(|error| error.to_string())?;
        let same = crate::document::same_quads(&batch.quads, &drawn.quads)
            && full.raid_report() == prepared.raid_report()
            && full.label_report() == prepared.label_report()
            && report == self.counter_report;
        profile_print(format!(
            "CHECK overlay raids in place {} quads {}",
            if same { "equal" } else { "DIFFER" },
            drawn.quads.len()
        ));
        if same {
            Ok(())
        } else {
            Err("raid icons installed in place differ from the overlay laid out afresh".into())
        }
    }

    /// The map service a load reads sources and preferences from: the desktop host's, else the
    /// toggle harness's.
    pub(crate) fn service(&self) -> Option<Arc<dashboard::Service>> {
        self.embedded
            .as_ref()
            .map(crate::embedded::Controller::service)
            .or_else(|| self.harness_service.clone())
    }

    fn configure_service(&mut self) {
        if let Some(embedded) = &self.embedded
            && let Err(error) = embedded
                .service()
                .configure_content(&self.config, self.options.allow_unverified)
        {
            self.error = Some(format!("Game tables: {error}"));
        }
    }

    pub fn new(
        ctx: &egui::Context,
        render_state: Option<&eframe::egui_wgpu::RenderState>,
        config: Config,
        options: Options,
        error: Option<String>,
        // capture
        failure: Arc<Mutex<Option<String>>>,
        embedded: Option<crate::embedded::Controller>,
    ) -> Self {
        ctx.set_theme(egui::Theme::Dark);
        let mut viewer = Self::create(
            config,
            options,
            error,
            failure,
            embedded,
            render_state.map(|state| (state.device.clone(), state.queue.clone())),
        );
        if viewer.embedded.is_some() {
            match hours::Resources::new() {
                Ok(resources) => {
                    viewer.dropper = Some(Arc::clone(resources.pool().dropper()));
                    viewer.worker_pool = Some(Arc::clone(resources.pool()));
                    viewer.shared_resources = Some(resources);
                }
                Err(error) => {
                    viewer.error = Some(format!("Native map workers: {error}"));
                    return viewer;
                }
            }
        }
        // Without usable folders the load fails at once and says so; an
        // embedded viewer keeps the path until the host sends its folders.
        if let Some(path) = viewer.options.replay.clone() {
            viewer.open(path, ctx, None);
        }
        viewer
    }

    fn create(
        config: Config,
        options: Options,
        mut error: Option<String>,
        failure: Arc<Mutex<Option<String>>>,
        embedded: Option<crate::embedded::Controller>,
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
    ) -> Self {
        if let Some((device, _)) = &gpu {
            crate::perf_log::gpu_timer(viewer_foundation::map::gpu_frame_time::support(device));
        }
        if let Some(embedded) = &embedded
            && let Err(tables_error) = embedded
                .service()
                .configure_content(&config, options.allow_unverified)
        {
            error = Some(format!("Game tables: {tables_error}"));
        }
        // capture: `--window-dump` arms the readback before the first frame, so
        // every frame it draws is a capture frame and the animation clock never
        // starts.
        let capture = options.window_dump.then(|| Capture {
            path: options.out.clone().unwrap_or_default(),
            remaining: if profiling() {
                // A cold seek is followed for thirty seconds past it: a seek
                // across a 1,400-hour branch walks its snow history for ~10 s.
                if std::env::var("BATTLE_PLAN_VIEWER_PROFILE").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE")).is_ok_and(|mode| mode == "seek_end") {
                    1921
                } else {
                    profiling_sweep().end
                }
            } else {
                1
            },
            pinned: Some((options.size, options.camera)),
            exit: true,
            time: Some(options.animation_time),
            step: options.animation_step,
            frames: options.window_frames.max(1),
            index: 0,
            paced: false,
        });
        // counters: the perspective starts where the command line put it.
        let perspective = options.perspective.clone();
        let map_mode = options.map_mode.clone();
        // Replaced with the document's `settings.txt` options when one loads.
        let counter_view = CounterView::new(
            options.counter_settings(&installed_content_settings::settings::MapOptions::DEFAULT),
        );
        let hidden_since = embedded.as_ref().map(|_| Instant::now());
        // The monitors are read off the UI thread; start before the first map.
        if options.playback_cost_end.is_none() {
            refresh_monitors(|| ());
        }
        Self {
            dashboard: None,
            dashboard_active: false,
            dashboard_map: false,
            host_graphics: false,
            shared_resources: None,
            source: None,
            source_decoder: None,
            loaded_config: None,
            rendered: None,
            rendered_sequence: 0,
            drawn_at: None,
            applying_options: None,
            drawn_options: None,
            hover: None,
            tooltip: None,
            tooltip_delay_ms: None,
            build_latency_ms: None,
            held_back: None,
            perf_view: None,
            map_warnings: Vec::new(),
            diagnostics: options.diagnostics,
            diagnostics_open: false,
            audio: None,
            audio_focus: true,
            muted: false,
            audio_reset: false,
            music: None,
            battles: None,
            interacted: false,
            interface_points: 1.0,
            embedded,
            harness_service: None,
            embedded_open: None,
            embedded_seek: None,
            revision: None,
            expected_revision: None,
            hidden_since,
            combat: combat::Window::default(),
            awaiting_folders: config.resolve_options().is_err(),
            camera: options.camera,
            camera_manual: options.camera.is_some(),
            settled_capture: false,
            held_view: None,
            held_clock: None,
            zoom: Motion::default(),
            // A window dump's placed walk starts walking, and its `--fly` flies.
            camera_mode: if options.walk.is_some() {
                CameraMode::Walk
            } else if options.fly.is_some() {
                CameraMode::Fly
            } else {
                CameraMode::Map
            },
            free_overlays: false,
            start_pick: drop::Pick::Idle,
            free_overlay: Batch::default(),
            overlay_written: 0,
            free_presented: None,
            looks: None,
            input: crate::controls::Input::default(),
            orbit: crate::controls::orbit::Orbit::default(),
            walk: walk::Walk::new(options.walk.clone()),
            flight: None,
            flying: None,
            fly_clock: None,
            fly_footprints: Default::default(),
            pointer_grabbed: false,
            // colour: `--postfx off` reaches the window too, so a window and a
            // dump of the same state can be the same frame.
            postfx: options.postfx,
            // A host's maps take its graphics preferences; a window without one takes
            // `settings.txt`'s choices when its recording opens (`install_map`).
            graphics: dashboard::protocol::MapGraphics {
                day_night: options.day_night.unwrap_or(option_defaults::DAY_NIGHT),
                show_allied_plans: options.allied_plans.unwrap_or(option_defaults::ALLIED_PLANS),
                ..Default::default()
            },
            counter_filter: Default::default(),
            player_counters: option_defaults::PLAYER_COUNTERS,
            volumes: Default::default(),
            overlay_layers: Default::default(),
            counter_owners: Default::default(),
            resolved_settings: Default::default(),
            // The host sends its setting before the map; a capture pins its own.
            ui_scale: Setting::MatchGame,
            fly_aircraft: Default::default(),
            match_game: None,
            monitor_seen: None,
            map_preferences_seen: None,
            visual_check: None,
            visual_check_after_load: Default::default(),
            visual_check_note: None,
            stale_visual_revision: None,
            visual_fallbacks: Default::default(),
            visual_report: None,
            drew_frame: false,
            layers_only: false,
            warming: false,
            visual_stale_campaign: None,
            config,
            options,
            document: None,
            loading: None,
            startup_content: installed_content_core::content::Generation::loading(),
            error,
            scene: None,
            scene_target: Presented::default(),
            map_error: None,
            playing: false,
            speed: 24.0,
            clock_anchor: None,
            day_night: Default::default(),
            last_step: Instant::now(),
            profile_playback: Default::default(),
            started: Instant::now(),
            opened: None,
            // hours
            playhead: None,
            animation_seek: false,
            map_animation_target: None,
            hours: None,
            playhead_map: None,
            playhead_opened: false,
            overlay_laid: None,
            counters_given: None,
            counters_drawn: None,
            shown_whole: false,
            playhead_opening: None,
            presented: MapPresented::default(),
            today_error: None,
            engaged_carry: None,
            epoch: 0,
            ready: BTreeMap::new(),
            shown: None,
            last_view: None,
            uploaded: Uploaded::default(),
            dropper: None,
            worker_pool: None,
            // close zoom
            unit_models: None,
            ships_shipless: None,
            ships_joined: None,
            movers_layered: (false, false),
            unit_field_uploaded: false,
            raid_tick: None,
            convoys_sailing: false,
            convoy_refused: BTreeMap::new(),
            // names
            placements: None,
            placements_at: None,
            fade: None,
            hovered_name_region: None,
            last_frame: Instant::now(),
            camera_step: None,
            label_report: LabelReport::default(),
            raid_lane: Default::default(),
            faction_highlight: Default::default(),
            hover_rows: Default::default(),
            head_rows: Default::default(),
            overlay_frame: None,
            overlay_tables_job: Default::default(),
            overlay_laying: None,
            overlay_shown: None,
            overlay_last_asked: None,
            overlay_drawn_fraction: None,
            overlay_lookups: None,
            laid_proof: None,
            head_tables: None,
            overlay_loading: Vec::new(),
            overlay_missed: Vec::new(),
            scripts: Default::default(),
            combat_rows: Default::default(),
            tooltip_refused: Default::default(),
            // counters
            counter_report: CounterReport::default(),
            map_tooltip_target: None,
            shown_tooltip: None,
            interface_sounds: Vec::new(),
            interface_key: 0,
            counter_held: None,
            pressed_on: None,
            counter_selecting: None,
            secondary_on: None,
            overlay: None,
            prepared_overlay: None,
            overlay_fraction: None,
            overlay_at: None,
            overlay_tables: None,
            counter_view,
            unit_selection: Default::default(),
            air_selection: Default::default(),
            fleet_selection: Default::default(),
            perspective,
            map_mode,
            diplomatic_selected: None,
            picker: Default::default(),
            picker_error: None,
            picker_refused: Default::default(),
            picker_controls: None,
            // capture
            capture,
            capture_note: None,
            failure,
            gpu,
            profile_frame: 0,
            laps: None,
        }
    }

    pub(crate) fn map_view(
        config: Config,
        mut options: Options,
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
        resources: Arc<hours::Resources>,
    ) -> Self {
        options.replay = None;
        options.window_dump = false;
        options.headless_dump = false;
        options.combat = None;
        let mut viewer = Self::create(config, options, None, Arc::new(Mutex::new(None)), None, gpu);
        viewer.dashboard_map = true;
        // The dashboard passes on the host's choice.
        viewer.diagnostics = false;
        // The dashboard gives one map at a time the audio.
        viewer.audio_focus = false;
        viewer.dropper = Some(Arc::clone(resources.pool().dropper()));
        viewer.worker_pool = Some(Arc::clone(resources.pool()));
        viewer.shared_resources = Some(resources);
        viewer
    }

    /// The app's worker threads, with or without today's hour pipeline: the host's shared ones,
    /// else this window's own, made on first use. Its dropper frees what the window retires.
    pub(super) fn worker_pool(&mut self) -> std::io::Result<Arc<crate::worker_pool::WorkerPool>> {
        if let Some(pool) = &self.worker_pool {
            return Ok(Arc::clone(pool));
        }
        let pool = crate::worker_pool::WorkerPool::new()?;
        self.dropper.get_or_insert_with(|| Arc::clone(pool.dropper()));
        self.worker_pool = Some(Arc::clone(&pool));
        Ok(pool)
    }

    /// The recording's `settings.txt` map options, the command line's first; a host's settings
    /// replace day/night and allied plans whenever they arrive.
    fn take_game_options(&mut self, game: &installed_content_settings::settings::MapOptions) {
        self.counter_view = CounterView::new(self.options.counter_settings(game));
        if !self.host_graphics {
            self.graphics.day_night = self.options.day_night.unwrap_or(game.day_night);
            self.graphics.show_allied_plans = self.options.allied_plans.unwrap_or(game.allied_plans);
        }
    }

    pub(crate) fn install_map(&mut self, document: Document, scene: Scene, revision: String) {
        self.clock_anchor = None;
        self.playhead = None;
        self.day_night = Default::default();
        self.take_game_options(&document.game_options);
        self.unit_selection = Default::default();
        self.air_selection = Default::default();
        self.fleet_selection = Default::default();
        self.head_rows = Default::default();
        self.forget_overlay_builds();
        if let Some(side) = self.playhead_map.take() {
            self.drop_later(side);
        }
        // The reader opens at the map's first frame, at its clock.
        self.playhead_opening = None;
        self.playhead_opened = false;
        self.presented = MapPresented::default();
        self.shown_whole = false;
        self.document = Some(document);
        if !self.dashboard_map
            && let Some(scene) = &mut self.scene
        {
            crate::perf_log::gpu_times(scene.retire_gpu_frame_times());
        }
        self.scene = Some(scene);
        self.revision = Some(revision);
        self.error = None;
        self.map_error = None;
        self.rendered = None;
        self.uploaded = Uploaded::default();
    }

    /// After each frame of a map whose content falls back from visual mods, its report for the
    /// host's fallback owner ([`crate::visual_fallback::Fallbacks::reduce`]): every refusal the
    /// scene made, by stable name, the frame's own failure, and whether it drew settled, by the
    /// viewer's render result. The owner decides; nothing is decided here.
    fn after_frame_with_visual_mods(&mut self, document: &Document, error: Option<&String>) {
        let Some(built) = document.content.fallback.clone() else {
            return;
        };
        let mut refusals = self
            .scene
            .as_ref()
            .map(|scene| scene.draw_refusals())
            .unwrap_or_default();
        if let Some(error) = error {
            refusals.insert(format!("frame: {error}"));
        }
        // What the content's loaders refused to draw (a model entity no descriptor decides).
        refusals.extend(document.content.uncovered_reads.refused());
        let requested = self.playhead.map_or(
            (document.playback.branch(), document.playback.hour()),
            |head| (head.branch, head.hour),
        );
        // With the playhead's map open, the hour on screen is its drawn hour once every drawn
        // layer is settled and uploaded.
        let shown = match &self.playhead_map {
            Some(side) => side
                .on_screen()
                .filter(|on| on.settled)
                .and_then(|on| Some((side.branch().unwrap_or(requested.0), on.drawn?, self.map_error.is_some()))),
            None => self
                .shown
                .as_ref()
                .filter(|shown| shown.epoch == self.epoch)
                .map(|shown| (shown.branch, shown.hour, shown.error.is_some())),
        };
        let settled = crate::visual_fallback::settled(
            self.drew_frame && error.is_none(),
            requested,
            self.rendered.as_ref().map(|head| (head.branch, head.hour)),
            shown,
            document.icons.lazy.busy(),
        );
        self.visual_report = Some(crate::visual_fallback::Report {
            built,
            refusals,
            settled,
        });
    }

    /// The last frame's report ([`Self::after_frame_with_visual_mods`]).
    pub(crate) fn take_visual_report(&mut self) -> Option<crate::visual_fallback::Report> {
        self.visual_report.take()
    }

    pub(crate) fn has_map(&self) -> bool {
        self.document.is_some() && self.scene.is_some()
    }

    /// Gives up the process audio if this view holds it; everything it
    /// played, music included, stops.
    pub(crate) fn release_audio(&mut self) {
        if let Some(audio) = &mut self.audio {
            audio.release();
        }
    }

    /// Stops what the map sounded while it shows no picture: an error view or a spinner. The
    /// next presented frame claims the audio again and starts its sounds afresh.
    fn silence_unshown(&mut self) {
        self.release_audio();
        self.audio_reset = true;
    }

    /// Sends this view's volumes and mute to the audio now, so a change
    /// applies even while the map does not draw.
    pub(crate) fn apply_audio_mix(&self) {
        if let (Some(audio), Some(document)) = (&self.audio, &self.document) {
            audio.set_mix(audio_mix(document, self.volumes, self.muted));
        }
    }

    /// Follow the host's settings for `scope`, resolving them only when they may have changed.
    pub(crate) fn follow_map_settings(
        &mut self,
        service: &dashboard::Service,
        scope: String,
    ) -> std::sync::Arc<dashboard::protocol::ResolvedMapSettings> {
        let settings =
            self.resolved_settings
                .get(service.map_settings_generation(), scope, |scope| {
                    service.scoped_map_settings(scope)
                });
        self.set_embedded_graphics(&settings);
        settings
    }

    /// Lighting for the frame being drawn, including a setting changed at the same replay hour.
    /// The cycle shows while it is on and the game would draw it at this playback; without the
    /// recorded speeds it shows only while paused, and the warnings name why.
    /// Today's ribbons with the orders layer's plan and the convoys layer's trade routes appended
    /// while they draw them; none while neither does.
    fn layered_orders(&self) -> Option<LayeredOrders> {
        let side = live_of(&self.playhead_map)?;
        let plan = side
            .draws("orders")
            .then(|| side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::orders::Orders>())))
            .flatten()
            .and_then(crate::layers::orders::Orders::plan)
            .map(|(_, plan)| Arc::clone(plan));
        let routes = layered_convoys(side).map(crate::layers::convoys::Convoys::product_seq);
        let raids = self.head_tables.as_ref().map(|(_, tables)| Arc::clone(tables.raid_arrows()));
        Some(LayeredOrders { plan, raids, routes })
    }

    fn frame_day_night(&self, hour: i32, speeds: &Result<GameSpeeds, String>) -> DayNight {
        let shown = speeds.as_ref().map_or(!self.playing, |speeds| {
            speeds.shows_cycle(self.playing, self.speed)
        });
        DayNight::at_hour(hour).with_cycle(self.graphics.day_night && shown)
    }

    /// The host's campaign or widget settings, including the counter setting.
    pub(crate) fn set_embedded_graphics(
        &mut self,
        settings: &dashboard::protocol::ResolvedMapSettings,
    ) {
        let graphics = settings.graphics;
        self.graphics = graphics;
        self.host_graphics = true;
        if self.counter_filter != settings.counters {
            self.counter_filter = settings.counters.clone();
        }
        if self.volumes != settings.audio {
            self.volumes = settings.audio;
            self.apply_audio_mix();
        }
        if let Some(document) = &self.document {
            self.counter_view.settings.nato = graphics
                .nato_counters
                .unwrap_or_else(|| self.options.counter_settings(&document.game_options).nato);
        }
    }

    /// Frees the registration of the scene's output, so the next frame registers it afresh.
    fn free_scene_target(&mut self, frame: &eframe::Frame) {
        if let Some(texture) = self.scene_target.take()
            && let Some(state) = frame.wgpu_render_state()
        {
            match crate::renderer_host::local(state) {
                Some(host) => host.free(texture),
                None => state.renderer.write().free_texture(&texture),
            }
        }
    }

    /// Lets go of what is sized to this hidden view; its scene stays, and the next frame shown
    /// makes its targets again.
    pub(crate) fn release_viewport(&mut self, frame: &eframe::Frame) {
        let Some(scene) = &mut self.scene else {
            return;
        };
        if scene.view().is_none() {
            return;
        }
        scene.release_viewport();
        self.free_scene_target(frame);
        self.rendered = None;
    }

    pub(crate) fn retire_map<T: Send + 'static>(&mut self, frame: &eframe::Frame, permit: T) {
        self.free_scene_target(frame);
        if !self.dashboard_map
            && let Some(scene) = &mut self.scene
        {
            crate::perf_log::gpu_times(scene.retire_gpu_frame_times());
        }
        let retired = (
            self.document.take(),
            self.scene.take(),
            self.hours.take(),
            std::mem::take(&mut self.ready),
            self.shown.take(),
            self.overlay_tables.take(),
            self.prepared_overlay.take(),
            self.unit_models.take(),
            std::mem::take(&mut self.raid_lane),
            std::mem::take(&mut self.faction_highlight),
            permit,
        );
        if let Some(dropper) = &self.dropper {
            dropper.drop_later(retired);
        }
        self.overlay_laid = None;
        self.close_playhead();
        self.rendered = None;
    }

    /// Nothing of this window is drawn: its maps stop sounding and let their playheads go.
    fn not_drawn(&mut self) {
        self.release_audio();
        self.close_playhead();
        if let Some(dashboard) = &mut self.dashboard {
            dashboard.release_audio();
            dashboard.close_playheads();
        }
    }

    /// A map not drawn holds no reader of its playhead: a reader that takes no steps would hold
    /// the playhead's ring for every other map of the recording. It opens again when drawn.
    pub(crate) fn close_playhead(&mut self) {
        if let Some(side) = self.playhead_map.take() {
            self.drop_later(side);
        }
        self.playhead_opening = None;
        self.hover_rows.end_frame(false);
        self.hover_rows.end_frame(false);
        self.playhead_opened = false;
        (self.counters_given, self.counters_drawn) = (None, None);
    }

    fn drop_later(&self, side: crate::playhead_host::MapSide) {
        match &self.dropper {
            Some(dropper) => dropper.drop_later(side),
            None => drop(side),
        }
    }

    pub(crate) fn retry_map(&mut self) {
        self.error = None;
        self.map_error = None;
    }

    fn unload_recording(&mut self, path: &Path) -> bool {
        if self
            .embedded_open
            .as_ref()
            .is_some_and(|pending| pending.path == path)
        {
            self.embedded_open = None;
        }
        if self
            .embedded_seek
            .as_ref()
            .is_some_and(|pending| pending.path == path)
        {
            self.embedded_seek = None;
        }
        if self.options.replay.as_deref() != Some(path) {
            return false;
        }
        self.options.replay = None;
        self.hover_rows = Default::default();
        self.combat_rows = Default::default();
        self.tooltip_refused.clear();
        self.source = None;
        self.source_decoder = None;
        self.loaded_config = None;
        self.revision = None;
        self.expected_revision = None;
        self.map_preferences_seen = None;
        self.playhead = None;
        self.opened = None;
        self.playing = false;
        self.capture = None;
        self.combat.close();
        self.error = None;
        self.map_error = None;
        true
    }

    pub(crate) fn map_ui(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        target: &dashboard::protocol::DashboardMapView,
    ) {
        self.perspective.country = target
            .options
            .country
            .clone()
            .map_or(Country::Recorded, Country::Tag);
        self.perspective.fog = target.options.fog;
        self.applying_options = Some(target.options.clone());
        if crate::perf_log::enabled() && self.perf_view.as_deref() != Some(target.id.as_str()) {
            self.perf_view = Some(target.id.clone());
        }
        self.apply_host_options(&target.options);
        let branch = target
            .branch_id
            .parse::<u64>()
            .expect("validated map command");
        let head = Playhead {
            branch,
            hour: target.hour,
            fraction: f64::from(target.fraction),
        };
        let animation_target = Target {
            branch,
            hour: target.hour,
            seek_sequence: target.seek_sequence,
        };
        let sought = self
            .map_animation_target
            .is_some_and(|old| animation_target.seeks_from(old));
        self.animation_seek |= sought;
        if sought && let Some(hours) = &mut self.hours {
            hours.seek();
        }
        self.map_animation_target = Some(animation_target);
        let valid = self
            .document
            .as_ref()
            .and_then(|document| document.playback.range(branch));
        if let Some((_, last)) = valid {
            self.anchor_playback(
                head,
                target.playing,
                target.hours_per_second,
                sought,
                Instant::now(),
                last,
            );
        }
        self.error = match valid {
            Some((first, last)) if (first..=last).contains(&target.hour) => None,
            Some((first, last)) => Some(format!(
                "Requested hour is outside this branch ({first}..{last})"
            )),
            None => Some("The requested branch is not present in this recording".into()),
        };
        if let Some(error) = self.error.clone() {
            self.rendered = None;
            self.silence_unshown();
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
            return;
        }
        if let Some(mut document) = self.document.take() {
            let result = self.map(ui, frame, &mut document);
            self.after_frame_with_visual_mods(&document, result.as_ref().err());
            if let Err(error) = result {
                self.error = Some(error);
                self.rendered = None;
            }
            self.document = Some(document);
        }
        if self.presentation_ready(self.playhead.unwrap_or(head), target.playing)
            && self.error.is_none()
            && self.map_error.is_none()
        {
            self.rendered_sequence = target.sequence;
        }
    }

    pub(crate) fn reset_drawn_frame(&mut self) {
        self.drew_frame = false;
    }

    pub(crate) fn drew_frame(&self) -> bool {
        self.drew_frame
    }

    /// Advances independently of the builders, for both the live host clock
    /// and the deterministic playback sweep. The branch end is always exact.
    fn tick_playhead(&mut self, seconds: f64, last: i32) {
        if self.playing
            && let Some(head) = &mut self.playhead
        {
            self.playing = tick_clock(head, self.speed, seconds, last);
        }
    }

    fn anchor_playback(
        &mut self,
        target: Playhead,
        playing: bool,
        speed: f64,
        sought: bool,
        now: Instant,
        last: i32,
    ) {
        let anchor = (target, playing, speed);
        if !sought && self.playhead.is_some() && self.clock_anchor == Some(anchor) {
            return;
        }
        if self.capture.is_none() {
            self.tick_playhead(now.duration_since(self.last_step).as_secs_f64(), last);
        }
        self.last_step = now;
        self.playhead = Some(anchor_clock(
            self.playhead,
            target,
            playing
                && !sought
                && self.speed == speed
                && (self.playing || self.clock_anchor.is_some_and(|(_, playing, _)| playing)),
        ));
        self.speed = speed;
        self.playing = playing && self.playhead.is_some_and(|head| head.hour < last);
        self.clock_anchor = Some(anchor);
        self.follow_branch();
        if sought {
            self.open_overlay_rows();
        }
        if let Some(side) = &self.playhead_map {
            if sought {
                self.presented.seek = side.on_screen().map(|on| on.resets);
            }
            side.anchor(target.hour, playing, speed, sought);
        }
    }

    /// Opens the overlay's reader of rows at the head on the open map's playhead before its next
    /// picture, when the map opens or a seek is taken: its tables are declared with the map's own
    /// and its first rows come with the map's first picture ([`head_rows::HeadRows::open_overlay`]).
    /// Installs `prepared`, laid out for `key`, with the drawn plates of the overlay it replaces
    /// when that was laid out for the same view and their stacks are the same by value.
    fn install_overlay(&mut self, mut prepared: crate::document::PreparedOverlay, key: &OverlayKey) {
        if let Some(old) = self.prepared_overlay.as_mut()
            && self.overlay_shown.as_ref().is_some_and(|shown| shown.same_view(key))
        {
            prepared.carry_plates(old);
        }
        let retired = self.prepared_overlay.replace(prepared);
        self.retire(retired);
        self.overlay_laid = key.rows;
        self.overlay_shown = Some(key.clone());
    }

    /// Frees `value` on the dropper's thread, here when there is none.
    fn retire<T: Send + 'static>(&self, value: T) {
        match &self.dropper {
            Some(dropper) => dropper.drop_later(value),
            None => drop(value),
        }
    }

    /// The overlay's builds and what they were made from belong to the content open: a build in
    /// flight for the content before gives nothing to the content after.
    fn forget_overlay_builds(&mut self) {
        let retired = (std::mem::take(&mut self.overlay_tables_job), self.head_tables.take(), self.overlay_frame.take(), self.overlay_laying.take());
        self.retire(retired);
        (self.overlay_shown, self.overlay_last_asked, self.overlay_drawn_fraction, self.overlay_lookups) = (None, None, None, None);
        self.overlay_loading.clear();
        self.overlay_missed.clear();
    }

    fn open_overlay_rows(&mut self) {
        let Some(side) = live_of(&self.playhead_map) else { return };
        let Some(branch) = side.branch() else { return };
        let left = crate::document::LeftToLayers::of(Some(side));
        if let Some(inputs) = head_rows::OverlayInputs::whole(&self.counter_view).without(left.resource_icons, left.occupation_marks) {
            self.head_rows.open_overlay(inputs, Some((side.entry(), crate::playhead_host::rows::Recording { branch })));
        }
    }

    /// The map mode of the hour on screen: the chosen one ([`Self::map_mode`]) once a picture
    /// for it is on screen, the previous one while it is made. With the playhead's map open, the
    /// mode its drawn picture was composed and settled for; today's installed hour's otherwise.
    fn shown_mode(&self) -> &viewer_world::map::map_mode::MapMode {
        match self.live_map() {
            Some(side) => side.mode_on_screen().unwrap_or(&self.map_mode),
            None => self.placements_at.as_ref().map_or(&self.map_mode, |(_, _, mode)| mode),
        }
    }

    /// A previous epoch's picture stays visible while its replacement is built. With the
    /// playhead's map open, once its reader has composed and uploaded a first picture.
    fn picture_ready(&self) -> bool {
        match &self.playhead_map {
            Some(side) => side.on_screen().is_some_and(|on| on.drawn.is_some()),
            None => self.shown.as_ref().is_some_and(|shown| shown.error.is_none()),
        }
    }

    /// Whether the frame drawn answers `target` with everything on screen: exactly, or, playing,
    /// with the newest hour before it. With the playhead's map open, its layers' answer alone.
    fn presentation_ready(&self, target: Playhead, playing: bool) -> bool {
        match &self.playhead_map {
            Some(_) => {
                self.rendered.is_some_and(|rendered| rendered.branch == target.branch)
                    && self.picture_whole(target, playing)
            }
            None => self.today_ready(target, playing),
        }
    }

    /// With the playhead's map open, whether every part of the picture drawn shows `target`'s
    /// hour ([`whole_picture`]); without it, whether today's frame answers it.
    fn picture_whole(&self, target: Playhead, playing: bool) -> bool {
        match &self.playhead_map {
            Some(side) => side.on_screen().is_some_and(|on| {
                whole_picture(&on, self.overlay_laid_hour(), self.counters_drawn_hour(), target.hour, playing)
            }),
            None => true,
        }
    }

    /// Whether the document's load is over: today's hour of this epoch is installed, no lazy art
    /// decodes, the overlay was laid out since the last did, and the picture asked for is whole
    /// ([`Self::picture_whole`]). Until then the content made for it is kept.
    fn load_complete(&self, art_busy: bool, overlay_fresh: bool) -> bool {
        self.shown.as_ref().is_some_and(|shown| shown.epoch == self.epoch)
            && !art_busy
            && overlay_fresh
            && self.playhead.is_some_and(|head| self.picture_whole(head, self.playing))
    }

    /// The hour of the rows `(hour, frame)` stand for: theirs, or a later one no step since changed
    /// them by.
    fn rows_stand(&self, (hour, rows): (replay_data::Hour, usize)) -> replay_data::Hour {
        self.head_rows
            .overlay_through(rows as *const replay_frame::frame::DecodedFrame)
            .map_or(hour, |stands| stands.max(hour))
    }

    /// The hour the overlay on screen shows: the rows at the head it was laid out from, while no
    /// read it missed is learned; none before its first layout.
    fn overlay_laid_hour(&self) -> Option<replay_data::Hour> {
        self.overlay_laid
            .filter(|_| self.overlay_missed.is_empty())
            .map(|rows| self.rows_stand(rows))
    }

    /// The hour of the head tables the counter layers held when the reader last drew.
    fn counters_drawn_hour(&self) -> Option<replay_data::Hour> {
        self.counters_drawn.map(|rows| self.rows_stand(rows))
    }

    /// Whether the picture is today's pipeline's (no playhead map), and what the playhead's
    /// layers show while its reader draws.
    fn on_screen(&self) -> (bool, Option<crate::playhead_host::OnScreen>) {
        match &self.playhead_map {
            Some(side) => (false, side.on_screen()),
            None => (true, None),
        }
    }

    /// The hour of the rows at the head the overlay on screen was laid out from: the older of its
    /// rows and those its tables were made from, each at the hour no step since changed them by.
    /// None before its first layout over rows at the head.
    pub(crate) fn overlay_hour(&self) -> Option<replay_data::Hour> {
        self.prepared_overlay.as_ref()?;
        let (hour, rows) = self.overlay_at.as_ref()?.rows?;
        let stands = |hour: replay_data::Hour, rows: usize| self.head_rows.overlay_through(rows as *const replay_frame::frame::DecodedFrame).map_or(hour, |stands| stands.max(hour));
        let laid = stands(hour, rows);
        Some(self.head_tables.as_ref().map_or(laid, |(key, _)| laid.min(stands(key.hour, key.rows))))
    }

    /// The playhead's map while it draws: open, its reader not stopped.
    fn live_map(&self) -> Option<&crate::playhead_host::MapSide> {
        self.playhead_map.as_ref().filter(|side| side.failure().is_none())
    }

    /// The branch and hour on screen: while the playhead's map is open, its reader's branch and
    /// drawn hour (the clock's before its first picture); today's installed hour otherwise.
    fn hour_on_screen(&self, document: &Document) -> (replay_data::BranchId, i32) {
        match &self.playhead_map {
            Some(_) => self.layers_on_screen().unwrap_or_else(|| self.playhead.map_or_else(|| (document.playback.branch(), document.playback.hour()), |clock| (clock.branch, clock.hour))),
            None => (document.playback.branch(), document.playback.hour()),
        }
    }

    /// While the playhead's map is open, its reader's branch (the clock's for a reader opened
    /// without one) and drawn hour; none before its first picture or once its reader stopped.
    fn layers_on_screen(&self) -> Option<(replay_data::BranchId, i32)> {
        let side = self.playhead_map.as_ref()?;
        Some((side.branch().or(self.playhead.map(|clock| clock.branch))?, side.on_screen()?.drawn?))
    }

    /// What rows at the head are stamped with: the clock's branch, which the reader follows.
    fn rows_recording(&self, document: &Document) -> crate::playhead_host::rows::Recording {
        crate::playhead_host::rows::Recording { branch: self.playhead.map_or_else(|| document.playback.branch(), |clock| clock.branch) }
    }

    /// The reader reads the clock's branch: one open on another is let go, and the next frame
    /// opens one at the clock. A playhead plays one branch's timeline, and the readers' rows carry
    /// the branch they were opened with.
    fn follow_branch(&mut self) {
        let Some(clock) = self.playhead else {
            return;
        };
        if self.playhead_map.as_ref().and_then(|side| side.branch()).is_some_and(|branch| branch != clock.branch) {
            self.close_playhead();
        }
    }

    /// A reader made over other content than the map draws (the map reopened with other visual
    /// mods) draws that content's art: it is let go, and the next frame opens one over `content`.
    fn follow_content(&mut self, content: *const crate::document::Content) {
        if self.playhead_map.as_ref().is_some_and(|side| !side.made_over(content)) {
            self.close_playhead();
        }
    }

    /// The map's error view: while the playhead's map is open, its reader's failure by name, else
    /// the recorded gap the clock stands in; today's hour's error otherwise. Today's error while
    /// the map is open is printed once, not shown.
    fn map_error_of(&mut self, document: &Document, today: Option<std::io::Error>) -> Option<String> {
        let Some(side) = &self.playhead_map else {
            return today.map(|error| error.to_string());
        };
        if let Some(failure) = side.failure() {
            return Some(failure.to_owned());
        }
        let today = today.map(|error| error.to_string());
        if today.is_some() && today != self.today_error {
            eprintln!("Today's hour pipeline: {}", today.as_deref().unwrap_or_default());
        }
        self.today_error = today;
        let clock = self.playhead?;
        let replay = document.playback.replay();
        replay.block_at(clock.branch, clock.hour).is_none().then(|| crate::playback::gap_reason(replay, clock.branch, clock.hour))
    }

    /// The window's own histories follow today's continuity rule over what the playhead's map
    /// presents: day and night and the counters' movement start again on a reset, and the audio on
    /// a reset or recreated units. Previous and next are the branch, drawn hour and viewing country
    /// of the last picture presented and of this one; a picture composed for the window's seek,
    /// or a reopened reader's first, is sought. The layers reset themselves on their compose.
    fn follow_continuity(&mut self, gaps: &[replay_frame::branches::Gap]) -> Option<(Continuity, viewer_foundation::map::presentation::UnitRestart)> {
        let side = self.live_map()?;
        let on = side.on_screen()?;
        let hour = on.drawn.filter(|_| !on.composing)?;
        let branch = side.branch().or(self.playhead.map(|clock| clock.branch))?;
        let viewing = crate::layers::vision::drawn(side).and_then(crate::layers::vision::Vision::ours).map(|slot| slot.to_string());
        let sought = self.presented.reopened || self.presented.seek.is_some_and(|asked| on.resets > asked);
        self.presented.reopened = false;
        if sought {
            self.presented.seek = None;
        }
        self.present_continuity((branch, hour, viewing), sought, gaps)
    }

    /// [`Self::follow_continuity`] for the picture `next`: none when it is the picture last
    /// presented and not sought.
    fn present_continuity(&mut self, next: (replay_data::BranchId, i32, Option<String>), sought: bool, gaps: &[replay_frame::branches::Gap]) -> Option<(Continuity, viewer_foundation::map::presentation::UnitRestart)> {
        use viewer_foundation::map::presentation::{UnitRestart, continuity, unit_restart};
        if !sought && self.presented.last.as_ref() == Some(&next) {
            return None;
        }
        let previous = self.presented.last.as_ref().map(|(branch, hour, viewing)| (*branch, *hour, viewing.as_deref()));
        let shown = (next.0, next.1, next.2.as_deref());
        let (continuity, restart) = (continuity(previous, shown, sought, gaps), unit_restart(previous, shown, sought));
        if continuity == Continuity::Reset {
            self.day_night = Default::default();
            self.counter_view.presentation.movement().lock().unwrap_or_else(|error| error.into_inner()).clear();
        }
        if continuity == Continuity::Reset || restart != UnitRestart::Keep {
            self.audio_reset = true;
        }
        self.presented.last = Some(next);
        Some((continuity, restart))
    }

    /// Whether today's frame answers `target` with a built hour of this epoch: exactly, or,
    /// playing, with the newest hour built before it.
    fn today_ready(&self, target: Playhead, playing: bool) -> bool {
        self.shown.as_ref().is_some_and(|shown| {
            !self.animation_seek
                && shown.epoch == self.epoch
                && shown.error.is_none()
                && self.rendered.is_some_and(|rendered| {
                    (rendered.branch, rendered.hour) == (shown.branch, shown.hour)
                        && presentation_matches(rendered, target, playing)
                })
        })
    }

    pub(crate) fn map_state(
        &self,
        target: &dashboard::protocol::DashboardMapView,
    ) -> dashboard::protocol::DashboardMapState {
        let error = self.error.clone().or_else(|| match &self.playhead_map {
            Some(_) => self.map_error.clone(),
            None => self
                .shown
                .as_ref()
                .filter(|shown| {
                    shown.branch.to_string() == target.branch_id && shown.hour == target.hour
                })
                .and(self.map_error.clone()),
        });
        let clock = target.branch_id.parse().ok().map(|branch| {
            let requested = Playhead {
                branch,
                hour: target.hour,
                fraction: f64::from(target.fraction),
            };
            if target.playing {
                self.playhead.unwrap_or(requested)
            } else {
                requested
            }
        });
        let ready = clock.is_some_and(|clock| self.presentation_ready(clock, target.playing))
            && self.rendered_sequence == target.sequence;
        let (today, layers) = self.on_screen();
        // Playing, a frame of the same branch stands in for the clock's hour; say how far back.
        let lag = |hour: i32| {
            clock
                .zip(self.rendered)
                .filter(|(clock, rendered)| target.playing && clock.branch == rendered.branch)
                .map(|(clock, _)| u32::try_from(i64::from(clock.hour) - i64::from(hour)).unwrap_or(0))
        };
        let today_behind = self.rendered.filter(|_| today).and_then(|rendered| lag(rendered.hour));
        let layers_behind = layers.and_then(|on| on.drawn).and_then(lag);
        let behind_reason = if layers_behind > today_behind.max(Some(0)) {
            layers.as_ref().map(layers_behind_reason)
        } else {
            (today && target.playing).then(|| self.held_back.as_ref().map(|(_, reason)| reason.clone())).flatten()
        };
        // The oldest hour on screen: the layers' while they show one older than today's frame.
        let (hour, fraction) = match (self.rendered, layers.and_then(|on| on.drawn)) {
            (None, _) => (None, None),
            (Some(rendered), Some(drawn)) if !today || drawn < rendered.hour => (Some(drawn), Some(0.0)),
            (Some(_), None) if !today => (None, None),
            (Some(rendered), _) => (Some(rendered.hour), Some(rendered.fraction as f32)),
        };
        dashboard::protocol::DashboardMapState {
            id: target.id.clone(),
            source_id: Some(target.source_id.clone()),
            revision: Some(target.revision.clone()),
            branch_id: self.rendered.map(|shown| shown.branch.to_string()),
            hour,
            fraction,
            sequence: self.rendered_sequence,
            requested_sequence: target.sequence,
            loading: error.is_none() && !ready,
            behind: today_behind.max(layers_behind),
            behind_reason,
            error,
            map_mode: self.rendered.map(|_| self.shown_mode().token()),
            drawn_at: self.rendered.and(self.drawn_at),
            hover: self.rendered.and(self.hover.clone()),
            tooltip: self.rendered.and(self.tooltip.clone()),
            tooltip_delay_ms: self.rendered.and(self.tooltip_delay_ms),
            zoom: self.rendered.and(self.last_view.as_ref().map(|view| view.camera.height)),
            center: self.rendered.and(self.last_view.as_ref().map(|view| view.camera.center)),
            options: self.rendered.and(self.drawn_options.clone()),
            applied: self.rendered.and_then(|_| self.applied_settings()),
        }
    }

    /// The settings the frame on screen drew with: the scene's anti-aliasing, the document's
    /// texture quality and visual mods, and the graphics, volumes and counters it follows.
    fn applied_settings(&self) -> Option<dashboard::protocol::AppliedMapSettings> {
        let document = self.document.as_ref()?;
        let mut graphics = self.graphics;
        graphics.texture_quality = document.texture_quality;
        if let Some(scene) = &self.scene {
            graphics.samples = scene.samples();
        }
        Some(dashboard::protocol::AppliedMapSettings {
            graphics,
            audio: self.volumes,
            counters: self.counter_filter.clone(),
            visual_mods: document.visual_mods.iter().flatten().map(|choice| choice.id.clone()).collect(),
        })
    }


    /// Everything the hour on screen could not draw exactly, collected after its overlay and
    /// hover. Current tooltip and combat gaps lead; persistent content notes follow. Each
    /// frame replaces the per-hour report, so recovered refusals go away with the next draw.
    /// The hour on screen's limitation lines and its parts' reports: the playhead's map's while it
    /// is open, today's installed hour's otherwise; none while neither shows an hour.
    fn hour_lines(&self) -> Option<[Vec<String>; 2]> {
        match self.playhead_map.as_ref().filter(|side| side.on_screen().is_some()) {
            Some(side) => {
                let ours = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::vision::Vision>())).and_then(crate::layers::vision::Vision::ours);
                let raids = ours.and_then(|ours| self.scripts.raid_notes(ours));
                let [limitations, reports] = map_lines(side, raids.as_deref().map_or(&[], Vec::as_slice));
                Some([limitations, political_lines(Some(side), &[]).into_iter().chain(reports).collect()])
            }
            None => self.shown.as_ref().map(|shown| {
                let reports = political_lines(None, &shown.political_warnings).into_iter();
                [shown.limitations.lines(), reports.chain(shown.plan_warnings.iter().chain(&shown.building_warnings).chain(&shown.model_warnings).cloned()).collect()]
            }),
        }
    }

    fn collect_warnings(&self, document: &Document) -> Vec<String> {
        let Some([limitations, reports]) = self.hour_lines() else {
            return Vec::new();
        };
        // Current hover and combat gaps lead so they stay visible above persistent content notes.
        let mut warnings: Vec<_> = tooltip_warning_lines(&self.tooltip_refused).collect();
        warnings.extend(self.counter_report.combat_gap_lines());
        warnings.extend(self.combat.warnings(&document.icons.combat.warnings));
        warnings.extend(
            self.picker_error
                .iter()
                .map(|error| format!("Map mode picker: {error}")),
        );
        warnings.extend(
            self.picker_refused
                .iter()
                .map(|(reason, count)| format!("Map mode picker: {reason} ({count} refused)")),
        );
        warnings.extend(limitations);
        // The viewer's device, when it lacks a feature some content needs to draw exactly.
        if let Some((device, _)) = &self.gpu {
            warnings.extend(device_requirements(device.features()));
        }
        warnings.extend(document.warnings.iter().cloned());
        warnings.extend(
            document
                .content
                .cursor
                .as_ref()
                .err()
                .map(|why| format!("Cursor: the game's arrow is not shown: {why}")),
        );
        warnings.extend(
            [Unit::Division(0), Unit::RailwayGun(0), Unit::TaskForce(0)]
                .into_iter()
                .filter_map(|unit| document.icons.counters.plate_sounds(unit).err())
                .map(|why| format!("Counters: their button sounds are not played: {why}")),
        );
        if self.graphics.day_night
            && let Err(error) = &document.game_speeds
        {
            warnings.push(format!(
                "Day/night cycle: {error}, so it shows only while paused"
            ));
        }
        warnings.extend(reports);
        if self.camera_mode == CameraMode::Walk {
            warnings.extend(self.walk.warning());
        }
        if let Some(placements) = &self.placements {
            warnings.extend(placements.warnings.iter().cloned());
        }
        warnings.extend(self.label_report.warnings());
        // The overlay's refused tokens: the raid icons' as laid out for this
        // camera, the counters' (with the naval mission icons' gaps) as last drawn.
        if let Some(overlay) = &self.prepared_overlay {
            warnings.extend(overlay.raid_report().warnings());
            warnings.extend(overlay.intel_report().warnings());
            warnings.extend(overlay.mode_icon_warnings());
            warnings.extend(overlay.air_orders.warnings());
            warnings.extend(
                overlay
                    .naval_selection
                    .refused
                    .iter()
                    .map(|(why, count)| format!("Navy selection: {count} withheld: {why}")),
            );
            warnings.extend(overlay.stripe_warnings());
        }
        warnings.extend(self.counter_report.warnings());
        for (reason, count) in &self.convoy_refused {
            warnings.push(format!("Trade-route convoys: {count} not drawn: {reason}."));
        }
        if let Some(scene) = &self.scene {
            warnings.extend(scene.pass_warnings());
            warnings.extend(
                scene
                    .shader_refusals()
                    .iter()
                    .map(|(reason, count)| format!("GUI shader: {count} refused draws: {reason}")),
            );
        }
        // The recorded naval engaged bytes against the flag derived for the recording's
        // player, over the hours the engaged index has folded so far.
        if let Ok(Some(history)) = self.engaged_shown(&self.perspective.country) {
            warnings.extend(history.byte_check().lines());
        }
        // A capture renders at the dump's scale, not Match game's.
        let pinned = self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.pinned.is_some());
        if self.ui_scale == Setting::MatchGame
            && !pinned
            && let Some((_, (_, Some(note)))) = &self.match_game
        {
            warnings.push(note.clone());
        }
        if self.camera_mode == CameraMode::Fly {
            warnings.extend(self.fly_refusals(document));
        }
        document.uncovered_reads.append_to(&mut warnings);
        warnings
    }

    /// The shown hour's notice that part of its snow and mud is unknown, which the map hatches.
    pub(crate) fn weather_notice(&self) -> Option<&'static str> {
        match self.playhead_map.as_ref().filter(|side| side.on_screen().is_some()) {
            Some(side) => side.parts().filter(|_| side.draws("snow")).and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::snow::Snow>())).and_then(crate::layers::snow::Snow::note),
            None => self.shown.as_ref().and_then(|shown| shown.snow.0.note()),
        }
    }

    /// The diagnostics an embedded map shows over itself: the hour's stats and
    /// its data limitations, collapsed behind their count.
    pub(crate) fn diagnostics_ui(&mut self, ui: &mut egui::Ui, max_height: f32) {
        // With the playhead's map open its layers' figures stand; today's build times are not shown.
        let side = self.playhead_map.as_ref().filter(|side| side.on_screen().is_some());
        let stats = match side {
            Some(side) => Some(side.parts().filter(|_| side.draws("snow")).and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::snow::Snow>())).and_then(crate::layers::snow::Snow::summary).map(|summary| vec![format!("Snow and mud: {}", summary.line())]).unwrap_or_default()),
            None => self.shown.as_ref().map(|shown| shown.stats().to_vec()),
        };
        if let Some(stats) = stats {
            for line in stats {
                ui.weak(line);
            }
            if let Some(target) = self.highlighted() {
                ui.weak(format!("Highlighted: {target}"));
            }
            if let Some(target) = self.shown_tooltip() {
                ui.weak(format!("Tooltip: {target}"));
            }
        }
        if self.map_warnings.is_empty() {
            return;
        }
        if ui
            .small_button(format!("{} data limitations", self.map_warnings.len()))
            .clicked()
        {
            self.diagnostics_open = !self.diagnostics_open;
        }
        if self.diagnostics_open {
            viewer_foundation::diagnostics_panel::warning_list(ui, &self.map_warnings, max_height);
        }
    }

    /// The diagnostics shortcut asks the desktop host, which owns the choice
    /// and sends it back.
    fn toggle_diagnostics(&self) {
        if let Some(embedded) = &self.embedded {
            embedded.toggle_diagnostics();
        }
    }

    fn open(&mut self, mut path: PathBuf, ctx: &egui::Context, expected_revision: Option<String>) {
        let _stage = viewer_foundation::load_record::stage("open.ui");
        if self.embedded.is_some() {
            match path.canonicalize() {
                Ok(canonical) => path = canonical,
                Err(error) => {
                    self.error = Some(format!("{}: {error}", path.display()));
                    return;
                }
            }
            if self.loading.is_some() {
                self.embedded_open = Some(crate::embedded::OpenRequest {
                    path,
                    revision: expected_revision,
                });
                self.embedded_seek = None;
                return;
            }
            self.embedded_open = None;
            if self
                .embedded_seek
                .as_ref()
                .is_some_and(|pending| !pending.matches(&path, expected_revision.as_deref()))
            {
                self.embedded_seek = None;
            }
            // Retain this selection if folders must be configured first.
            self.options.replay = Some(path.clone());
            self.expected_revision = expected_revision.clone();
        }
        let resolve = match self.config.resolve_options() {
            Ok(resolve) => resolve,
            Err(error) => {
                self.failed_load(error.to_string(), ctx);
                self.awaiting_folders = true;
                return;
            }
        };
        let Some((device, queue)) = self.gpu.clone() else {
            self.failed_load("WGPU renderer is unavailable".to_owned(), ctx);
            return;
        };
        let held_clock = self
            .held_view
            .take()
            .filter(|(held, _)| *held == path)
            .map(|(_, clock)| clock);
        let held = held_clock.is_some();
        self.options.replay = Some(path.clone());
        let options = self.options.clone();
        let service = self.service();
        let verify_revision = service.is_some();
        let resources = self.shared_resources.clone();
        let config = self.config.clone();
        let mut content = self
            .dashboard
            .as_ref()
            .map(|dashboard| dashboard.content_snapshot(&config))
            .unwrap_or_default();
        if let (Some(source), Some(document), Some(loaded)) =
            (&self.source, &self.document, &self.loaded_config)
            && loaded.game_dir == config.game_dir
            && loaded.user_dir == config.user_dir
        {
            content.push((source.key().clone(), Arc::clone(&document.content)));
        }
        // Content loaded before its visual mods changed on disk is never reused.
        if let Some(stale) = self.stale_visual_revision.take() {
            content.retain(|(_, content)| content.visual_revision != stale);
        }
        // Each load reads the fallback its recording's keys hold when its source is open.
        let fallbacks = Arc::clone(&self.visual_fallbacks);
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let startup_content = self.startup_content.take();
        job_priority::spawn(move || {
            let _stage = viewer_foundation::load_record::stage("load");
            let _generation = startup_content.as_ref().map_or_else(
                installed_content_core::content::Generation::enter,
                installed_content_core::content::Loading::enter_generation,
            );
            drop(startup_content);
            let initial = InitialHour {
                branch: options.branch,
                hour: options.hour,
                hour_fraction: options.hour_fraction,
            };
            // A withheld game table stops its reader with the reason, which the map then names.
            let load = std::panic::AssertUnwindSafe(|| -> Result<LoadedReplay, String> {
                let revision = if verify_revision {
                    let _stage = viewer_foundation::load_record::stage("load.replay_revision");
                    let revision = dashboard::protocol::replay_revision(&path)
                        .map_err(|error| error.to_string())?;
                    if expected_revision
                        .as_ref()
                        .is_some_and(|expected| *expected != revision)
                    {
                        return Err("Recording changed since it was selected; reopen it in Analytics before viewing this snapshot".into());
                    }
                    Some(revision)
                } else {
                    None
                };
                let source = {
                    let _stage = viewer_foundation::load_record::stage("load.open_map");
                    service
                        .as_ref()
                        .map(|service| service.open_map(&path, revision.as_deref()))
                        .transpose()?
                };
                let preferences = source
                    .as_ref()
                    .and_then(|source| {
                        service
                            .as_ref()
                            .map(|service| service.map_settings(&source.replay()))
                    })
                    .unwrap_or_default();
                let recording = source.as_ref().map(|source| {
                    crate::visual_fallback::recording(
                        &source.key().source_id,
                        &source.key().revision,
                    )
                });
                // A scene that cannot be built with the visual mods is built again without them,
                // named, so a broken mod never leaves the recording unviewable.
                let (document, decoder, scene) = loop {
                    let decisions = recording.as_ref().map(|recording| {
                        crate::visual_fallback::lock(&fallbacks).decisions(recording)
                    });
                    let mut document = match &source {
                        Some(source) => match content.iter().find(|(key, content)| {
                            key == source.key()
                                // Only content its fallback key holds now.
                                && content.fallback.as_ref().is_none_or(|built| {
                                    crate::visual_fallback::lock(&fallbacks).current(built)
                                })
                                && content.visual_mods == preferences.visual_mods
                                && content.texture_quality == preferences.graphics.texture_quality
                        }) {
                            Some((_, content)) => {
                                let _stage = viewer_foundation::load_record::stage("load.shared_content");
                                viewer_foundation::load_record::count("content reused", 1, 0);
                                Document::from_shared_content(
                                    Arc::clone(content),
                                    source.replay(),
                                    &initial,
                                )
                            }
                            None => Document::load_shared(
                                &path,
                                &resolve,
                                options.allow_unverified,
                                &initial,
                                source.replay(),
                                source,
                                preferences.visual_mods.as_deref(),
                                preferences.graphics.texture_quality,
                                recording.as_deref().zip(decisions.as_ref()),
                                crate::document::LoadTarget::Scene,
                            ),
                        },
                        None => Document::load(
                            &path,
                            &resolve,
                            options.allow_unverified,
                            &initial,
                            (!options.visual_mods.is_empty())
                                .then_some(options.visual_mods.as_slice()),
                            0,
                        ),
                    }
                    .map_err(|error| error.to_string())?;
                    if let Some(source) = &source {
                        document.bind_source(source);
                    }
                    let decoder_stage = viewer_foundation::load_record::stage("load.decoder");
                    let decoder = resources
                        .as_ref()
                        .map(|resources| {
                            let replay = document.playback.replay_handle();
                            let decoder = resources
                                .decoder(Arc::clone(&replay))
                                .map_err(|error| error.to_string())?;
                            let mut presented = crate::playback::Playback::unloaded(replay)
                                .map_err(|error| error.to_string())?;
                            if let Some(binding) = document.playback.map_binding() {
                                presented.bind_map(binding);
                            }
                            presented.present(
                                document.playback.branch(),
                                document.playback.hour(),
                                document.playback.current().clone(),
                            );
                            decoder
                                .seed(std::mem::replace(&mut document.playback, presented))
                                .map_err(|error| error.to_string())?;
                            Ok::<_, String>(decoder)
                        })
                        .transpose()?;
                    drop(decoder_stage);
                    // The builders need seconds for the first hour and the UI
                    // thread idles on the spinner meanwhile; the whole
                    // hour-independent scene is cheaper here than on any frame.
                    let workers = resources
                        .as_ref()
                        .map_or_else(Default::default, |resources| resources.pool().workers());
                    let scene_stage = viewer_foundation::load_record::stage("load.scene");
                    let scene = load_scene(&device, &queue, &document, preferences.graphics, &workers);
                    drop(scene_stage);
                    match scene {
                        Ok(scene) => break (document, decoder, scene),
                        // Whatever failed, content with the visual mods probes without them, as its
                        // frames then decide; only a scene that cannot be built without them
                        // either is an error.
                        Err(error) => match &document.content.fallback {
                            Some(crate::visual_fallback::Built {
                                key,
                                under: crate::visual_fallback::Under::Mods { excused },
                            }) => crate::visual_fallback::lock(&fallbacks).adopt(
                                key.clone(),
                                crate::visual_fallback::Decision::Probing {
                                    failures: [format!("scene: {error}")].into(),
                                    excused: excused.clone(),
                                    frames_needed: 0,
                                    frames_seen: 0,
                                },
                            ),
                            _ => return Err(error.to_string()),
                        },
                    }
                };
                Ok(LoadedReplay {
                    document,
                    scene,
                    revision,
                    source,
                    decoder,
                    config,
                })
            });
            let result = std::panic::catch_unwind(load).unwrap_or_else(|stopped| {
                Err(stopped
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| stopped.downcast_ref::<&str>().map(|said| (*said).to_owned()))
                    .unwrap_or_else(|| "Replay loader stopped before returning a result".into()))
            });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.loading = Some(receiver);
        self.opened = Some(Instant::now());
        self.combat.close();
        self.document = None;
        self.diplomatic_selected = None;
        self.hover_rows = Default::default();
        self.combat_rows = Default::default();
        self.tooltip_refused.clear();
        self.source = None;
        self.source_decoder = None;
        self.loaded_config = None;
        self.rendered = None;
        self.revision = None;
        if !self.dashboard_map
            && let Some(scene) = &mut self.scene
        {
            crate::perf_log::gpu_times(scene.retire_gpu_frame_times());
        }
        self.scene = None;
        // hours: a newer snapshot of this recording keeps its engaged index.
        self.playhead = None;
        self.engaged_carry = self
            .hours
            .take()
            .and_then(|hours| hours.take_engaged())
            .or(self.engaged_carry.take());
        self.ready.clear();
        if let Some(dropper) = &self.dropper {
            dropper.drop_later(self.shown.take());
        } else {
            self.shown = None;
        }
        self.last_view = None;
        // close zoom
        self.unit_models = None;
        (self.ships_shipless, self.ships_joined) = (None, None);
        self.unit_field_uploaded = false;
        self.profile_frame = 0;
        self.scene_target.generation = 0;
        if self.embedded.is_none() {
            self.error = None;
        }
        self.map_error = None;
        self.playing = false;
        self.held_clock = held_clock;
        if held {
            // The same map rebuilt: only the walk's figure comes from the content again.
            self.walk.forget_figure();
        } else {
            self.camera = self.options.camera;
            self.camera_manual = self.options.camera.is_some();
            self.zoom.reset();
            self.orbit.reset();
            self.walk.reset();
            self.flight = None;
            // A start chosen on the last map is chosen again on this one, with the pointer given
            // back: the new map's first frame shows the cursor again.
            if self.start_pick.at().is_some() {
                self.start_pick = drop::Pick::Choosing;
                self.input.release();
            }
            self.fly_clock = None;
            self.fly_footprints = Default::default();
        }
        // names
        self.placements = None;
        self.placements_at = None;
        self.fade = None;
        // counters
        self.overlay = None;
        self.overlay_written += 1;
        self.prepared_overlay = None;
        self.overlay_laid = None;
        self.counter_report = CounterReport::default();
        self.counter_held = None;
        self.unit_selection = Default::default();
        self.air_selection = Default::default();
        self.fleet_selection = Default::default();
        self.overlay_fraction = None;
        self.overlay_at = None;
        self.overlay_tables = None;
        self.forget_overlay_builds();
        self.raid_lane = Default::default();
        if let Some(dropper) = &self.dropper {
            dropper.drop_later(std::mem::take(&mut self.faction_highlight));
        }
    }

    /// Records a lap of the frame that is being timed, if one is.
    fn lap(&mut self, name: &'static str) {
        if let Some(laps) = &mut self.laps {
            laps.lap(name);
        }
    }

    /// A `--window-dump` waits for this load, so its failure ends the capture
    /// and the process; the map's thread hands the message to main, which exits 1.
    /// Other captures belong to the map on screen and stay armed.
    fn failed_load(&mut self, error: String, ctx: &egui::Context) {
        self.failed_window_capture(&format!("replay could not be loaded: {error}"), ctx);
        self.error = Some(error);
    }

    fn hour_error(&mut self, error: Option<String>, ctx: &egui::Context) {
        if let Some(error) = &error {
            self.failed_window_capture(error, ctx);
        }
        self.map_error = error;
    }

    /// A window dump cannot write an unavailable map. Surface its failure to
    /// the caller instead of leaving a capture parked on an error view.
    fn failed_window_capture(&mut self, error: &str, ctx: &egui::Context) {
        if let Some(capture) = self.capture.take_if(|capture| capture.exit) {
            let message = format!("Capture {} failed: {error}", capture.path.display());
            *self
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(message.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.capture_note = Some(message);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let result = self.loading.as_ref().map(|receiver| receiver.try_recv());
        // An older loader may finish between logic and UI. Do not briefly
        // display or announce its replay after the host selected another.
        if (self.embedded_open.is_some() || self.options.replay.is_none())
            && result
                .as_ref()
                .is_some_and(|result| !matches!(result, Err(mpsc::TryRecvError::Empty)))
        {
            self.loading = None;
            return;
        }
        match result {
            Some(Ok(Ok(LoadedReplay {
                document,
                scene,
                revision,
                source,
                decoder,
                config,
            }))) => {
                let _stage = viewer_foundation::load_record::stage("install");
                self.revision = revision;
                self.source = source;
                self.source_decoder = decoder;
                self.loaded_config = Some(config);
                if self.embedded.is_some() {
                    self.error = None;
                }
                self.combat_rows.pending = self.options.combat;
                self.playhead = Some(Playhead {
                    branch: document.playback.branch(),
                    hour: document.playback.hour(),
                    fraction: f64::from(document.hour_fraction),
                });
                self.clock_anchor = None;
                // A rebuild in place carries on with the host's clock as it ran meanwhile.
                if let Some(held) = self.held_clock.take()
                    && let Some(head) = held.playhead
                    && let Some((_, last)) = document.playback.range(head.branch)
                {
                    let now = Instant::now();
                    let mut head = head;
                    self.playing = held.playing
                        && tick_clock(
                            &mut head,
                            self.speed,
                            now.duration_since(held.at).as_secs_f64(),
                            last,
                        );
                    self.playhead = Some(head);
                    self.clock_anchor = held.anchor;
                    self.last_step = now;
                }
                self.take_game_options(&document.game_options);
                if self.embedded.is_none() {
                    self.graphics.full_resolution = document.game_full_resolution;
                    self.volumes = document.game_audio;
                }
                self.shown_whole = false;
                self.document = Some(document);
                // The loader built the whole hour-independent scene; the
                // frames only upload hour data into it from here.
                self.scene = Some(scene);
                self.uploaded = Uploaded::default();
                self.loading = None;
                if let (Some(service), Some(source)) = (self.service(), &self.source) {
                    let settings = service.map_settings(&source.replay());
                    self.set_embedded_graphics(&settings);
                }
                if let Some(opened) = self.opened
                    && profiling()
                {
                    profile_print(format!(
                        "PROFILE open document_and_scene {:.1} ms",
                        opened.elapsed().as_secs_f64() * 1000.0
                    ));
                }
            }
            Some(Ok(Err(error))) => {
                self.loading = None;
                self.failed_load(error, ctx);
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.loading = None;
                self.failed_load(
                    "Replay loader stopped before returning a result".into(),
                    ctx,
                );
            }
            _ => {}
        }
    }

    /// Asks for the monitors to be read again when this window's monitor or
    /// display scale changes; a no-op otherwise.
    fn watch_monitors(
        monitor_seen: &mut Option<(Option<egui::Vec2>, Option<f32>)>,
        ctx: &egui::Context,
    ) {
        let seen = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.monitor_size, viewport.native_pixels_per_point)
        });
        if *monitor_seen != Some(seen) {
            *monitor_seen = Some(seen);
            let ctx = ctx.clone();
            refresh_monitors(move || ctx.request_repaint());
        }
    }

    /// The Match game scale for `document` and its note, resolved again only
    /// when the monitors have been re-read or the content changed.
    fn match_game<'a>(
        match_game: &'a mut Option<MatchGame>,
        document: &Document,
    ) -> &'a (Raster, Option<String>) {
        let key = (
            monitors_generation(),
            Arc::as_ptr(&document.content) as usize,
        );
        if match_game.as_ref().is_none_or(|(cached, _)| *cached != key) {
            let monitors = ui_scale_monitors();
            let resolved = Setting::MatchGame.resolve_raster(
                document.game_display.as_ref().map_err(String::as_str),
                monitors
                    .as_ref()
                    .as_ref()
                    .map(Vec::as_slice)
                    .map_err(String::as_str),
            );
            *match_game = Some((key, resolved));
        }
        &match_game.as_ref().expect("resolved above").1
    }

    // capture
    /// Reads the map target back into a PNG once a frame has been drawn into it.
    ///
    /// Called at the end of [`Viewer::map`], where the scene holds exactly the
    /// frame the window just produced. The readback and the unverified-content
    /// banner are [`render`]'s, the ones `--dump` uses, so what lands on disk
    /// differs from a dump of the same state only where the two paths differ.
    fn save_capture(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        document: &Document,
        camera: Camera,
        pixels: [u32; 2],
        warnings: &[String],
        ctx: &egui::Context,
    ) {
        match &mut self.capture {
            Some(capture) if capture.remaining > 0 => {
                capture.remaining -= 1;
                ctx.request_repaint();
                return;
            }
            Some(_) => {}
            None => return,
        }
        let mut capture = self.capture.take().expect("matched above");
        let output = if capture.frames > 1 || capture.index > 0 {
            let stem = capture
                .path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            capture
                .path
                .with_file_name(format!("{stem}-{:04}.png", capture.index))
        } else {
            capture.path.clone()
        };
        let scene = self.scene.as_ref().expect("a frame was drawn");
        let unverified = document
            .warnings
            .iter()
            .filter(|warning| warning.starts_with("UNVERIFIED CONTENT"))
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let written = (|| -> std::io::Result<()> {
            if !unverified.is_empty() {
                render::render_banner(scene, &state.device, &state.queue, &unverified)?;
            }
            render::write_png(scene, &state.device, &state.queue, &output)
        })();
        match written {
            Ok(()) => {
                // The warning area is the window's own report of what this hour
                // could not draw, and a capture is read beside a `--dump`, which
                // prints the same list; printing it here is what lets the two be
                // compared without a human reading the window.
                for warning in warnings {
                    eprintln!("Warning: {warning}");
                }
                println!(
                    "Captured branch {}, hour {}{} to {} at {}x{}",
                    document.playback.branch(),
                    document.playback.hour(),
                    // counters: which frame between two recorded hours this is,
                    // in the words `--dump` uses, so the two can be compared.
                    if document.hour_fraction > 0.0 {
                        format!(" + {:.3} h", document.hour_fraction)
                    } else {
                        String::new()
                    },
                    output.display(),
                    pixels[0],
                    pixels[1]
                );
                println!(
                    "Camera: {},{},{}",
                    camera.center[0], camera.center[1], camera.height
                );
                if let Some(line) = self.flight_line(document) {
                    println!("{line}");
                }
                if let Some(time) = capture.time {
                    println!("Animation time: {time:.6} s");
                }
                if self.camera_mode == CameraMode::Walk
                    && let Some(walk) = self.walk.report()
                {
                    println!("{walk}; player draws {}", scene.player_drawn());
                }
                println!("Colour grading {}", if self.postfx { "on" } else { "off" });
                self.capture_note = Some(format!("Saved {}", output.display()));
            }
            Err(error) => {
                let message = format!("{}: {error}", capture.path.display());
                if capture.exit
                    && let Ok(mut failure) = self.failure.lock()
                {
                    *failure = Some(message.clone());
                }
                eprintln!("{message}");
                if self.embedded.is_some() {
                    self.error = Some(message.clone());
                }
                self.capture_note = Some(message);
            }
        }
        if capture.frames > 1 {
            capture.frames -= 1;
            capture.index += 1;
            capture.time = capture.time.map(|t| t + capture.step);
            capture.remaining = 0;
            self.capture = Some(capture);
            ctx.request_repaint();
        } else if capture.exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn map_hit(&self, point: [f32; 2]) -> Option<MapHit> {
        let top = top_map_hit(
            point,
            &self.counter_report,
            self.prepared_overlay
                .as_ref()
                .map_or(&[], |map| map.raid_hits()),
            self.prepared_overlay
                .as_ref()
                .map_or(&[], |map| map.occupation_hits()),
            self.looks.as_ref(),
            self.prepared_overlay
                .as_ref()
                .and_then(|map| map.intel_at(point, self.looks.as_ref())),
        );
        if top == Some(MapHit::Blocked) {
            return top;
        }
        let rank = |family, layer| match &self.looks {
            None => Some((point, 0)),
            Some(looks) => looks.hit(family, layer, point),
        };
        let mut key = top.and_then(|hit| self.hit_layer(hit))
            .and_then(|(family, layer)| rank(family, layer).map(|(_, rank)| (rank, layer)));
        let mut top = top;
        for (index, hit) in self.prepared_overlay.as_ref().map_or(&[][..], |map| map.adjacency_hits()).iter().enumerate() {
            if let Some((local, rank)) = rank(free_icons::IconFamily::AdjacencyRules, hit.layer)
                && hit.geometry.contains(local)
                && key.is_none_or(|key| (rank, hit.layer) >= key)
            {
                key = Some((rank, hit.layer));
                top = Some(MapHit::Adjacency(index));
            }
        }
        top
    }

    /// The layer key of `target`'s hit shape; `None` for targets that are not one map icon.
    fn hit_layer(&self, target: MapHit) -> Option<(free_icons::IconFamily, u64)> {
        use free_icons::IconFamily;
        let report = &self.counter_report;
        match target {
            MapHit::Adjacency(index) => self.prepared_overlay.as_ref()?.adjacency_hits()
                .get(index).map(|hit| (IconFamily::AdjacencyRules, hit.layer)),
            MapHit::AirMission(index) => report
                .air_mission_hits
                .get(index)
                .map(|hit| (IconFamily::Missions, hit.layer)),
            MapHit::NavalMission(index) => report
                .naval_mission_hits
                .get(index)
                .map(|hit| (IconFamily::Missions, hit.layer)),
            MapHit::NavalRegionHover(index) => report
                .naval_region_hovers
                .get(index)
                .map(|hit| (IconFamily::Missions, hit.layer)),
            MapHit::Raid | MapHit::Intel => None,
            MapHit::Occupation(index) => self
                .prepared_overlay
                .as_ref()?
                .occupation_hits()
                .get(index)
                .map(|hit| (IconFamily::Occupation, hit.layer)),
            MapHit::Combat(index) => report
                .combat_hits
                .get(index)
                .map(|hit| (IconFamily::Combats, hit.layer)),
            MapHit::NavalObserver(index) => report
                .naval_marker_hovers
                .get(index)
                .map(|hit| (IconFamily::Combats, hit.layer)),
            MapHit::PortTooltip(index) => {
                report.task_force_port_hovers.get(index).and_then(|hover| {
                    report
                        .unit_hits
                        .get(hover.unit_hit)
                        .map(|hit| (unit_family(report, hover.unit_hit, hit), hit.layer))
                })
            }
            MapHit::Unit(index) => report
                .unit_hits
                .get(index)
                .map(|hit| (unit_family(report, index, hit), hit.layer)),
            MapHit::Pin(index) => report
                .faction_pin_hovers
                .get(index)
                .map(|hit| (IconFamily::Pins, hit.layer)),
            MapHit::Base(index) => report.base_hits.get(index).map(|hit| {
                (
                    report
                        .air_grid_hits
                        .get(&index)
                        .map_or(IconFamily::Bases, |grid| grid.family),
                    hit.layer,
                )
            }),
            MapHit::Blocked => None,
        }
    }

    /// The drawn raid icon under `point`, as [`Viewer::map_hit`] ranks them.
    fn raid_hit(&self, point: [f32; 2]) -> Option<&viewer_raids::map::raids::tooltips::Hit> {
        let hits = self.prepared_overlay.as_ref()?.raid_hits();
        match &self.looks {
            None => hits
                .iter()
                .filter(|hit| hit.contains(point))
                .max_by_key(|hit| hit.layer),
            Some(looks) => hits
                .iter()
                .filter_map(|hit| {
                    let (local, rank) =
                        looks.hit(free_icons::IconFamily::Raids, hit.layer, point)?;
                    hit.contains(local).then_some(((rank, hit.layer), hit))
                })
                .max_by_key(|(key, _)| *key)
                .map(|(_, hit)| hit),
        }
    }

    /// `point` in the layout of the icon whose hit shape is keyed `layer` in its family, as a free
    /// eye shows it; `point` itself when the overlay draws as laid out.
    fn local_point(
        &self,
        layer: Option<(free_icons::IconFamily, u64)>,
        point: [f32; 2],
    ) -> [f32; 2] {
        layer
            .and_then(|(family, layer)| self.looks.as_ref()?.hit(family, layer, point))
            .map_or(point, |(local, _)| local)
    }

    /// A laid-out rectangle `[x, y, width, height]` of the icon keyed `layer` where a free eye
    /// draws it.
    fn shown_rect(&self, layer: Option<(free_icons::IconFamily, u64)>, rect: [f32; 4]) -> [f32; 4] {
        let Some(present) =
            layer.and_then(|(family, layer)| self.looks.as_ref()?.of(family, layer))
        else {
            return rect;
        };
        let [x, y] = [0, 1]
            .map(|axis| present.pivot[axis] + present.scale * (rect[axis] - present.pivot[axis]));
        [x, y, rect[2] * present.scale, rect[3] * present.scale]
    }

    /// A pin owns hover only when its presented GUI object is the top hit.
    fn faction_pin_at(
        &self,
        point: [f32; 2],
    ) -> Option<&viewer_counters::map::counters::faction_pings::PinHover> {
        let MapHit::Pin(index) = self.map_hit(point)? else {
            return None;
        };
        self.counter_report.faction_pin_hovers.get(index)
    }

    fn update_faction_highlight(
        &mut self,
        document: &Document,
        device: &eframe::wgpu::Device,
        queue: &eframe::wgpu::Queue,
        point: Option<[f32; 2]>,
    ) -> Result<(), String> {
        let pool = self.worker_pool().map_err(|why| format!("Native map workers: {why}"))?;
        let adjacency = self.map_mode == viewer_world::map::map_mode::MapMode::Stock(viewer_world::map::map_mode::Stock::StrategicNavy);
        let color = if adjacency { &document.border_assets.adjacency_highlight_color } else { &document.border_assets.faction_highlight_color };
        let Ok(color) = color else {
            return Ok(());
        };
        let selected = if adjacency {
            point.and_then(|point| match self.map_hit(point) {
                Some(MapHit::Adjacency(index)) => self.prepared_overlay.as_ref()?.adjacency_hits().get(index),
                _ => None,
            }).map(|hit| Arc::clone(&hit.area)).unwrap_or_default()
        } else {
            point.and_then(|point| self.faction_pin_at(point))
                .map(|hit| Arc::clone(&hit.regions)).unwrap_or_default()
        };
        let Some(shown) = &self.shown else {
            return Ok(());
        };
        // While the political layer draws, the highlight is built over its picture, keyed by the
        // hour it last drew whole, or at another mode over the mode fills layer's, keyed by its
        // product's hour; today's part is then empty.
        let layered = match &self.playhead_map {
            Some(side) if side.draws("political") => {
                let Some((_, layers)) = side.parts() else { return Ok(()) };
                match layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::mode_fills::ModeFills>()).filter(|layer| layer.drawn()) {
                    Some(fills) => {
                        let Some((regions, zone, hour)) = fills.picture() else { return Ok(()) };
                        Some((faction_highlight::Over::Mode(regions, zone), 1 << 62 | u64::from(hour as u32)))
                    }
                    None => {
                        let Some(layer) = layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::political::Political>()) else { return Ok(()) };
                        Some((faction_highlight::Over::Layer(layer), side.last_frame().drawn.map_or(u64::MAX, |hour| 1 << 63 | u64::from(hour as u32))))
                    }
                }
            }
            _ => None,
        };
        let today = shown.borders.as_ref().filter(|_| layered.is_none());
        let picture = match (&layered, today) {
            (Some((_, picture)), _) => *picture,
            (None, Some((_, borders))) => borders.id,
            (None, None) => return Ok(()),
        };
        let key = faction_highlight::Key {
            epoch: shown.epoch,
            branch: shown.branch,
            hour: shown.hour,
            perspective: self.perspective.clone(),
            borders: picture,
            regions: Arc::clone(&selected),
            adjacency,
        };
        let scene = self.scene.as_mut().expect("scene built");
        if self.faction_highlight.requested() != Some(&key)
            && !scene.faction_highlight_regions().is_empty()
        {
            let restored = match (self.faction_highlight.take_restore(), today) {
                (Some(restore), _) => scene.apply(device, queue, SceneInput::SetFactionHighlight { plan: &restore, strategic_regions: &[], color: *color }).map(drop),
                (None, Some((_, borders))) => scene.apply(device, queue, SceneInput::SetBorderPlans { classes: &borders.classes, zone: &borders.highlighted }).map(drop),
                (None, None) => Ok(()),
            };
            restored.map_err(|why| why.to_string())?;
        }
        let content = Arc::clone(&document.content);
        let dropper = pool.dropper();
        let over = match (layered, today) {
            (Some((over, _)), _) => over,
            (None, Some((political, borders))) => faction_highlight::Over::Today(Arc::clone(political), Arc::clone(borders)),
            (None, None) => return Ok(()),
        };
        if let Some(prepared) = self.faction_highlight.update_with(
            key,
            |job| pool.try_submit(job),
            || {
                let source = over.copied();
                move || {
                    let provinces: std::collections::BTreeSet<u32> = if adjacency {
                        selected.iter().copied().collect()
                    } else { selected
                        .iter()
                        .flat_map(|id| {
                            content
                                .strategic_regions
                                .get(id)
                                .into_iter()
                                .flat_map(|region| region.provinces.iter().copied())
                        })
                        .collect() };
                    let meshes = &content.border_meshes;
                    match source {
                        faction_highlight::Over::Layer(picture) => {
                            let (regions, zone) = picture.regions_and_zone(&content.map);
                            faction_highlight::Prepared {
                                plan: viewer_world::map::scene::FactionHighlight::new(meshes, &regions, &zone, &provinces),
                                restore: Some(viewer_world::map::scene::FactionHighlight::new(meshes, &regions, &zone, &Default::default())),
                            }
                        }
                        faction_highlight::Over::Mode(regions, zone) => faction_highlight::Prepared {
                            plan: viewer_world::map::scene::FactionHighlight::new(meshes, &regions, &zone, &provinces),
                            restore: Some(viewer_world::map::scene::FactionHighlight::new(meshes, &regions, &zone, &Default::default())),
                        },
                        faction_highlight::Over::Today(political, borders) => faction_highlight::Prepared {
                            plan: viewer_world::map::scene::FactionHighlight::new(meshes, &political.layers.regions, &borders.zone, &provinces),
                            restore: None,
                        },
                    }
                }
            },
            dropper,
        ) {
            let result = scene.apply(
                device,
                queue,
                SceneInput::SetFactionHighlight {
                    plan: &prepared.plan,
                    strategic_regions: &self
                        .faction_highlight
                        .requested()
                        .expect("requested highlight")
                        .regions,
                    color: *color,
                },
            );
            dropper.drop_later(prepared.plan);
            result.map_err(|why| why.to_string())?;
            self.faction_highlight.set_restore(prepared.restore);
            self.faction_highlight.installed();
        }
        Ok(())
    }

    /// How this view's hovers read: its counter naming, and the recorded sections only its
    /// diagnostics show.
    fn hover_style(&self) -> viewer_counter_products::map::counter_tooltips::Style {
        viewer_counter_products::map::counter_tooltips::Style {
            nato: self.counter_view.settings.nato,
            diagnostics: self.diagnostics,
        }
    }

    fn map_tooltip(
        &mut self,
        document: &Document,
        point: Option<[f32; 2]>,
        extent: [f32; 2],
        ctx: &egui::Context,
    ) -> Option<(Arc<Tooltip>, [f32; 4], Option<u32>)> {
        if self.open_map().is_none() {
            return self.map_tooltip_today(document, point, extent, ctx);
        }
        let point = point?;
        let target = self.map_hit(point);
        // An intel entry takes the hover even when its text is empty: the province's is not
        // asked for.
        if matches!(target, Some(MapHit::Intel)) {
            let hit = self
                .prepared_overlay
                .as_ref()
                .and_then(|map| map.intel_at(point, self.looks.as_ref()))?;
            let layer = Some((free_icons::IconFamily::Intel, hit.layer));
            let rect = self.shown_rect(layer, hit.bounds());
            let empty = hit.tooltip.title.is_empty()
                && hit.tooltip.body.is_empty()
                && hit.tooltip.refused.is_empty();
            return (!empty).then(|| (Arc::clone(&hit.tooltip), rect, None));
        }
        let layer = target.and_then(|target| self.hit_layer(target));
        // The pointer in the hit icon's own layout, as its hover shapes are laid out.
        let local = self.local_point(layer, point);
        // Every text below is made from rows at the head of the open map, with the vision and the
        // engagements its layers show.
        let (entry, recording) = self.rows_head()?;
        let head = (&entry, recording);
        let repaint = || {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        if let Some(MapHit::Occupation(index)) = target {
            let hit = self
                .prepared_overlay
                .as_ref()?
                .occupation_hits()
                .get(index)?;
            let rect = self.shown_rect(layer, hit.bounds()?);
            use viewer_world::map::occupation_icons::Hover;
            let hover = hit.hover.clone();
            let text = match &hover {
                Hover::Modifier { state, token } => {
                    let result = (|| {
                        let content = document.icons.occupation.as_ref().map_err(Clone::clone)?;
                        content
                            .definitions
                            .find(token)?
                            .ok_or_else(|| format!("active definition {token} is absent"))
                    })();
                    // The same catalogue chose and drew the badge; keep its exact definition.
                    match result {
                        Ok(definition) => {
                            let vision = self.shown_vision().ok()?;
                            let inputs = self.hover_inputs(document, &vision, ctx);
                            let key = inputs.key(hover_rows::What::Occupation { state: *state, token: token.clone() }, [0, 0]);
                            self.hover_rows.ask(hover_rows::Family::Occupation, head, key, |_| inputs.occupation(*state, definition.clone()))?
                        }
                        Err(why) => {
                            let mut text = Tooltip::default();
                            text.refused
                                .insert(format!("occupation badge {token}: {why}"), 1);
                            Arc::new(text)
                        }
                    }
                }
                Hover::Resistance { state } => {
                    let province = document
                        .icons
                        .raids
                        .trigger_content
                        .state_provinces
                        .get(state)
                        .and_then(|members| members.first())
                        .copied();
                    let Some(province) = province else {
                        let mut text = Tooltip::default();
                        text.refused.insert(format!("occupation marker state {state}: no member province supplies the resistance hover"), 1);
                        return Some((Arc::new(text), rect, None));
                    };
                    let vision = self.shown_vision().ok()?;
                    let inputs = self.hover_inputs(document, &vision, ctx);
                    let (mode, selected, rosters) = (viewer_counter_products::map::province_tooltips::ModeHover::OccupationMarker, self.unit_selection.ordered_units(), document.playback.roster_sampler());
                    let key = inputs.key(hover_rows::What::Province { province, mode: mode.clone() }, [hover_rows::id(selected), hover_rows::id(&rosters)]);
                    self.hover_rows.ask(hover_rows::Family::Province, head, key, |caches| inputs.province(caches, province, mode, Arc::clone(selected), rosters))?
                }
            };
            return Some((text, rect, None));
        }
        if let Some((text, rect)) = prepared_naval_tooltip(&self.counter_report, target) {
            return Some((text, self.shown_rect(layer, rect), None));
        }
        if let Some(MapHit::Base(index)) = target
            && let Some(hit) = self.counter_report.air_grid_hits.get(&index)
        {
            return Some((
                Arc::clone(&hit.text),
                self.shown_rect(layer, self.counter_report.base_hits[index].rect),
                None,
            ));
        }
        if let Some(MapHit::Base(index)) = target
            && let Some(&(province, kind)) = self.counter_report.air_base_hits.get(&index)
        {
            let vision = self.shown_vision().ok()?;
            let (inputs, wings) = (self.hover_inputs(document, &vision, ctx), self.air_selection.wings());
            let key = inputs.key(hover_rows::What::AirBase { province, kind }, [hover_rows::id(wings), 0]);
            let text = self.hover_rows.ask(hover_rows::Family::Counters, head, key, |_| inputs.air_base(province, kind, Arc::clone(wings)))?;
            return Some((
                text,
                self.shown_rect(layer, self.counter_report.base_hits[index].rect),
                None,
            ));
        }
        if let Some(MapHit::AirMission(index)) = target {
            let hit = self.counter_report.air_mission_hits.get(index)?;
            return Some((
                Arc::clone(&hit.text),
                self.shown_rect(layer, hit.rect),
                None,
            ));
        }
        if let Some(MapHit::NavalRegionHover(index)) = target {
            use viewer_counters::map::counters::NavalRegionalControl;
            let (region, control, rect) = {
                let hit = self.counter_report.naval_region_hovers.get(index)?;
                (hit.region, hit.control, hit.rect)
            };
            let text = match control {
                NavalRegionalControl::Mines | NavalRegionalControl::Accidents => {
                    let mines = matches!(control, NavalRegionalControl::Mines);
                    let vision = self.shown_vision().ok()?;
                    let inputs = self.hover_inputs(document, &vision, ctx);
                    let key = inputs.key(if mines { hover_rows::What::Mines(region) } else { hover_rows::What::Accidents(region) }, [0, 0]);
                    self.hover_rows.ask(hover_rows::Family::Naval, head, key, |_| inputs.naval(region, mines))?
                }
                _ => {
                    let mut text = Tooltip::default();
                    text.refuse(
                        "regional Navy control hover",
                        "drawn control has no prepared hover text".into(),
                    );
                    Arc::new(text)
                }
            };
            return Some((text, self.shown_rect(layer, rect), None));
        }
        if matches!(target, Some(MapHit::Raid))
            && let Some(hit) = self.raid_hit(point).cloned()
        {
            let ours = self.shown_vision().ok()?.ours();
            let text = self.combat_rows.raid(head, document, ours, self.counter_view.settings.nato, hit.key.clone(), ctx)?;
            let rect = [hit.origin[0], hit.origin[1], hit.size[0], hit.size[1]];
            let layer = Some((free_icons::IconFamily::Raids, hit.layer));
            return Some((text, self.shown_rect(layer, rect), None));
        }
        let land_hit = match target {
            Some(MapHit::Combat(index)) => {
                let hit = &self.counter_report.combat_hits[index];
                match hit.selection {
                    Selection::Land(id) => Some((id, hit.rect)),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((id, rect)) = land_hit {
            let viewer = self.shown_vision().ok()?.ours();
            let index = self.layer_engaged();
            let text = self.combat_rows.land_marker(head, document, viewer, id, index, self.counter_view.settings.nato, repaint())?;
            return Some((text, self.shown_rect(layer, rect), None));
        }
        let naval_hit = match target {
            Some(MapHit::Combat(index)) => Some(&self.counter_report.combat_hits[index]),
            Some(MapHit::NavalObserver(index)) => {
                Some(&self.counter_report.naval_marker_hovers[index])
            }
            _ => None,
        }
        .and_then(|hit| match hit.selection {
            Selection::Naval(id) => Some((id, hit.rect)),
            _ => None,
        });
        if let Some((id, rect)) = naval_hit {
            let viewer = self.shown_vision().ok()?.ours();
            let index = self.layer_engaged();
            let text = self.combat_rows.naval_marker(head, document, viewer, id, index, repaint())?;
            return Some((text, self.shown_rect(layer, rect), None));
        }
        if let Some(hit) = self.counter_report.land_hovers.iter().rev().find(|hit| {
            hit.contains(local)
                && matches!(target, Some(MapHit::Unit(index))
                if land_hover_matches_unit(hit, &self.counter_report.unit_hits[index]))
        }).cloned()
        {
            let vision = self.shown_vision().ok()?;
            let inputs = self.hover_inputs(document, &vision, ctx);
            let key = inputs.key(hover_rows::What::Land(hit.clone()), [0, 0]);
            let text = self.hover_rows.ask(hover_rows::Family::Counters, head, key, |caches| inputs.land(caches, hit.clone()))?;
            let text = if hit.ours
                && !hit.hidden
                && !hit.railway_gun
                && hit.members.len() == 1
                && hit.control
                    == viewer_foundation::map::counter_hovers::land::CounterControl::Normal
            {
                self.combat_rows.division_panel(head, document, text, hit.members[0], repaint())
            } else {
                text
            };
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if let Some(hit) = self
            .counter_report
            .task_force_hovers
            .iter()
            .rev()
            .find(|hit| {
                hit.contains(local)
                    && matches!(target, Some(MapHit::Unit(index))
                if index == hit.unit_hit && hit.matches(&self.counter_report.unit_hits[index]))
            })
            .cloned()
        {
            if !hit.supported {
                return None;
            }
            let vision = self.shown_vision().ok()?;
            let inputs = self.hover_inputs(document, &vision, ctx);
            let key = inputs.key(hover_rows::What::TaskForce(hit.clone()), [0, 0]);
            let text = self.hover_rows.ask(hover_rows::Family::Counters, head, key, |caches| inputs.task_force(caches, hit.clone()))?;
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if let Some(hit) = self.counter_report.task_force_port_hovers.iter().enumerate().rev()
            .find(|(hover_index, hit)| hit.contains(local) && hit.has_tooltip()
                && self.counter_report.unit_hits.get(hit.unit_hit).is_some_and(|unit| hit.matches(unit))
                && (matches!(target, Some(MapHit::Unit(index)) if index == hit.unit_hit)
                    || matches!(target, Some(MapHit::PortTooltip(index)) if index == *hover_index)))
            .map(|(_, hit)| hit.clone())
        {
            if !hit.supported { return None; }
            let vision = self.shown_vision().ok()?;
            let inputs = self.hover_inputs(document, &vision, ctx);
            let key = inputs.key(hover_rows::What::Port(hit.clone()), [0, 0]);
            let text = self.hover_rows.ask(hover_rows::Family::Counters, head, key, |caches| inputs.port(caches, hit.clone()))?;
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if target.is_some() {
            return None;
        }
        let view = self.last_view.as_ref()?;
        // The trade routes the convoys layer draws: none drawn, none hovered.
        let routes = self.open_map().filter(|side| side.draws("convoys") && side.draws("units")).and_then(|side| side.parts()).and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::convoys::Convoys>()));
        if self.counter_view.visibility.trade_routes
            && routes.is_some_and(|convoys| {
                viewer_foundation::map::ribbon_content::trade_route_hit(
                    &convoys.product().field.trade_route_hovers,
                    view.camera.eye(),
                    view.camera
                        .ray(point.map(|v| v * view.ui_scale), view.viewport),
                    document.map.width as f32,
                )
            })
        {
            let mut text = Tooltip::default();
            text.refused.insert("trade-route hover: transfer rows, delivery efficiency and naval-dominance explanation are not implemented".to_owned(), 1);
            return Some((Arc::new(text), [0.0, 0.0, extent[0], extent[1]], None));
        }
        let province = match pick(
            &view.camera,
            point.map(|v| v * view.ui_scale),
            view.viewport,
            &document.map,
            document.assets.heightmap_texels(),
        ) {
            Ok(province) => province?,
            Err(reason) => {
                let mut text = Tooltip::default();
                text.refused.insert(reason, 1);
                return Some((Arc::new(text), [0.0, 0.0, extent[0], extent[1]], None));
            }
        };
        let vision = self.shown_vision().ok()?;
        if *self.shown_mode()
            == viewer_world::map::map_mode::MapMode::Stock(
                viewer_world::map::map_mode::Stock::Terrain,
            )
        {
            let text = self.combat_rows.terrain(head, document, province, &vision, repaint())?;
            return Some((text, [0.0, 0.0, extent[0], extent[1]], Some(province)));
        }
        let mode = self.shown_mode().clone();
        let mut mode_hover = picker::mode_hover(&mode, document.map_modes.defines.opinion_thresholds(), || self.combat_rows.infrastructure(head, document, province, repaint()));
        if let viewer_counter_products::map::province_tooltips::ModeHover::StrategicAir(selected) =
            &mut mode_hover
        {
            *selected = Arc::clone(self.air_selection.wings());
        }
        if let viewer_counter_products::map::province_tooltips::ModeHover::StrategicNavy(sources) =
            &mut mode_hover
        {
            // The overlay's tables made from rows at the head make them; loading by name before.
            *sources = match &self.head_tables {
                Some((_, tables)) => tables.naval_dominance_sources(),
                None => Arc::new(Err("naval dominance sources: the overlay's tables at the head are loading".into())),
            };
        }
        let inputs = self.hover_inputs(document, &vision, ctx);
        let (selected, rosters) = (self.unit_selection.ordered_units(), document.playback.roster_sampler());
        let key = inputs.key(hover_rows::What::Province { province, mode: mode_hover.clone() }, [hover_rows::id(selected), hover_rows::id(&rosters)]);
        let text = self.hover_rows.ask(hover_rows::Family::Province, head, key, |caches| inputs.province(caches, province, mode_hover, Arc::clone(selected), rosters))?;
        Some((text, [0.0, 0.0, extent[0], extent[1]], Some(province)))
    }

    fn map(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        document: &mut Document,
    ) -> Result<(), String> {
        let state = frame
            .wgpu_render_state()
            .ok_or("WGPU renderer is unavailable")?;
        self.map_with_state(ui, state, document)
    }

    fn map_with_state(
        &mut self,
        ui: &mut egui::Ui,
        state: &eframe::egui_wgpu::RenderState,
        document: &mut Document,
    ) -> Result<(), String> {
        // The frame's scope was taken before a document loaded this frame could be installed.
        let tables = document.content.tables;
        let result = engine_executable::executable_tables::scope(tables, || {
            self.draw_map(ui, state, document)
        });
        if !self.drew_frame {
            self.silence_unshown();
        }
        result
    }

    fn draw_map(
        &mut self,
        ui: &mut egui::Ui,
        state: &eframe::egui_wgpu::RenderState,
        document: &mut Document,
    ) -> Result<(), String> {
        let _other_cost = replay_frame::cpu::scope("frame.refresh", Some("other"));
        let map_timer = Instant::now();
        self.drew_frame = false;
        let mut laps = self
            .laps
            .take()
            .unwrap_or_else(|| Laps::new(Duration::ZERO));
        laps.lap("before_map");
        viewer_foundation::map::counter_move::take_profile();
        if profiling() {
            static ADAPTER_ONCE: std::sync::Once = std::sync::Once::new();
            shader_pipeline::gpu::stats::mark_window_thread();
            let info = state.adapter.get_info();
            ADAPTER_ONCE.call_once(|| {
                profile_print(format!("PROFILE adapter {} {:?}", info.name, info.backend));
            });
        }
        // Wall-clock captures use the interactive tick; nominal captures keep their fixed sweep step.
        let wall_clock = self
            .profile_playback
            .sample(Instant::now)
            .filter(|_| self.profile_playback.wall_active());
        if self.capture.is_none() || wall_clock.is_some() {
            let (now, elapsed) = wall_clock.map_or_else(
                || {
                    let now = Instant::now();
                    (now, now.duration_since(self.last_step).as_secs_f64())
                },
                |clock| (clock.now, clock.seconds),
            );
            self.last_step = now;
            if let Some(head) = self.playhead
                && let Some((_, last)) = document.playback.range(head.branch)
            {
                self.tick_playhead(elapsed, last);
            }
            // The host sends only clock anchors; a playing clock draws every frame on its own.
            if self.playing {
                ui.ctx().request_repaint();
            }
        }
        // A map's reader opens on the first frame it is drawn after it was not, or after the clock
        // left its branch, on the clock's branch and hour when no other map of the recording
        // anchors the playhead already.
        self.follow_content(Arc::as_ptr(&document.content));
        self.follow_branch();
        if !self.playhead_opened
            && let Some(clock) = self.playhead
        {
            self.playhead_opened = true;
            let at = crate::playhead_host::At { branch: clock.branch, hour: clock.hour, playing: self.playing, speed: self.speed };
            self.playhead_opening = crate::playhead_host::Opening::start(document, Some(at));
        }
        if let Some(made) = self.playhead_opening.as_ref().and_then(crate::playhead_host::Opening::poll) {
            self.playhead_opening = None;
            match made {
                // Opened for a branch the clock has left meanwhile: opened again at the clock.
                Some(side) if self.playhead.is_some_and(|clock| side.branch().is_some_and(|branch| branch != clock.branch)) => {
                    self.drop_later(side);
                    self.playhead_opened = false;
                }
                made => {
                    if let Some(side) = &made {
                        document.playback.read_rosters_of(side.entry());
                    }
                    self.presented.reopened = made.is_some();
                    self.playhead_map = made;
                    self.open_overlay_rows();
                }
            }
        }
        if let Some(side) = &mut self.playhead_map {
            let at = match (&self.capture, self.playhead) {
                (Some(capture), Some(head)) if capture.paced => crate::playhead_host::Head::Paced(head.hour),
                (Some(_), Some(head)) => crate::playhead_host::Head::Stepped(head.hour),
                _ => crate::playhead_host::Head::Clock(Instant::now()),
            };
            let view = crate::layers::View {
                map_mode: self.map_mode.clone(),
                perspective: self.perspective.clone(),
                camera_height: self.last_view.as_ref().map_or(0.0, |view| view.camera.height),
                camera: self.last_view.as_ref().map(|view| view.camera.placement()),
                target: self.zoom.target(),
                viewport: self.last_view.as_ref().map_or([0, 0], |view| view.viewport),
                fog: self.perspective.fog,
                graphics: self.graphics,
                sound: self.audio_focus,
                audio: self.volumes,
                overlay: self.overlay_layers.clone(),
                counters: self.counter_filter.clone(),
                player_counters: self.player_counters,
                selected: self.diplomatic_selected,
            };
            self.counters_drawn = self.counters_given;
            crate::playhead_host::draw_frame(side, view, at);
            if let Some(battles) = crate::layers::sounds::drawn(side) {
                self.battles = battles;
            }
        }
        // The scene arrives with the document from the loading thread
        // (`load_scene`); only hour data uploads per frame from here.
        // The screen-space interface's scale. A capture keeps the dump's, 1.0
        // unless `--ui-scale` names one, so a `--window-dump` and a `--dump` of
        // the same state stay the same frame.
        let pinned = self.capture.as_ref().and_then(|capture| capture.pinned);
        if self.last_view.is_none() && let Some(embedded) = &self.embedded {
            let frame = ui.ctx().input(|input| input.viewport().inner_rect).map(|rect| [
                (rect.width() * ui.ctx().pixels_per_point()).round() as u32,
                (rect.height() * ui.ctx().pixels_per_point()).round() as u32,
            ]);
            if frame.is_none() || frame != embedded.frame_size() {
                self.rendered = None;
                ui.ctx().request_repaint();
                return Ok(());
            }
        }
        let raster = match pinned {
            Some(_) => Raster::direct(self.options.ui_scale.unwrap_or(1.0)),
            None => {
                Self::watch_monitors(&mut self.monitor_seen, ui.ctx());
                let setting = self.ui_scale;
                let (matched, _) = Self::match_game(&mut self.match_game, document);
                match setting {
                    Setting::Manual(scale) => Raster::direct(scale),
                    Setting::MatchGame => *matched,
                }
            }
        };
        let ui_scale = raster.ui_scale;
        self.interface_points =
            ui_scale_points(1.0, raster.screen_scale(), ui.ctx().pixels_per_point());
        let size = ui.available_size();
        if size.x <= 0.0 || size.y <= 0.0 {
            self.rendered = None;
            ui.ctx().request_repaint();
            return Ok(());
        }
        // capture: `--window-dump` renders at the size it was asked for rather
        // than at the size the panels left over, so the frame it writes does not
        // depend on this display's scale factor or on where the warning area
        // wrapped. Match game renders into the game's framebuffer pixel space
        // before presenting its completed image at the desktop stretch.
        let pixels = match pinned {
            Some((size, _)) => size,
            None => raster.target([
                (size.x * ui.ctx().pixels_per_point()).round() as u32,
                (size.y * ui.ctx().pixels_per_point()).round() as u32,
            ]),
        };
        let view = hours::View {
            camera: self.last_view.map_or_else(
                || {
                    let map_size = MapSize::of(&document.map);
                    let defines = document.map_defines;
                    match pinned {
                        Some((size, camera)) => camera.map_or_else(
                            || Camera::fit(map_size, defines, size),
                            |placement| placement.on(map_size, defines),
                        ),
                        None => self.orbit.view(self.camera.filter(|_| self.camera_manual).map_or_else(
                            || Camera::fit(map_size, defines, pixels),
                            |placement| placement.on(map_size, defines),
                        )),
                    }
                },
                |view| view.camera,
            ),
            viewport: pixels,
            ui_scale,
        };
        // `--layers-only` never starts today's pipeline.
        let hour_error = if self.layers_only {
            None
        } else {
            let _lap = replay_frame::cpu::scope("frame.refresh.hour", Some("other"));
            self.refresh_hour(state, document, view, &mut laps).err()
        };
        if let (Some(side), Some(scene)) = (&mut self.playhead_map, &mut self.scene) {
            let stepped = self.capture.as_ref().is_some_and(|capture| !capture.paced);
            // The land stacks read the vision layer themselves. The other counter layers are given
            // the overlay's tables made from rows at the head, whose vision is the vision layer's.
            if let Some((key, tables)) = &self.head_tables {
                let _lap = replay_frame::cpu::scope("frame.refresh.counters", Some("other"));
                let passes = || -> crate::layers::air_mission_counters::Passes { document.playback.air_rosters() };
                self.counters_given = Some((key.hour, key.rows));
                if crate::layers::counter_kinds::give(side, &key.perspective, tables.vision(), tables.counters(), key.branch, passes)
                    && stepped
                    && let Err(error) = side.settle_all()
                {
                    eprintln!("The map's counter layers could not settle a new vision: {error}");
                }
            }
            // A saved frame (stepped, or a window dump) draws the plan of its hour.
            if (stepped || self.options.window_dump) && side.failure().is_none() {
                let hour = side.last_frame().head;
                settle_plan(side, hour, Duration::from_secs(120))?;
            }
            let mut gpu = crate::layers::Gpu { device: &state.device, queue: &state.queue, scene, content: &document.content };
            crate::playhead_host::upload_frame(side, &mut gpu, stepped);
        }
        // The counter kinds the layers draw take their products; a kind whose layer stops keeps its
        // last until tables holding today's are on screen.
        {
            let _lap = replay_frame::cpu::scope("frame.refresh.counters", Some("other"));
            let tables = match self.live_map() {
                Some(_) => self.head_tables.as_ref().map(|(_, tables)| tables),
                None => self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables),
            };
            let left_out = tables.map(|tables| tables.counters().left_out()).unwrap_or_default();
            let (published, draws) = self.playhead_map.as_ref().map(|side| (crate::layers::counter_kinds::published(side), crate::layers::counter_kinds::draws(side))).unwrap_or_default();
            self.counter_view.presentation.counters.lock().unwrap_or_else(|error| error.into_inner()).set_layered(published, draws, left_out);
        }
        let map_error = self.map_error_of(document, hour_error);
        self.hour_error(map_error, ui.ctx());
        self.follow_continuity(&document.playback.replay().gaps);
        // An error view otherwise stops repainting; a paused seek away from it
        // still needs to poll the asynchronous builders until its hour arrives.
        if self.playhead.is_some_and(|head| match &self.playhead_map {
            Some(side) => side.on_screen().is_some_and(|on| on.drawn != Some(head.hour) || !on.settled),
            None => !self.hour_ready(head.branch, head.hour),
        }) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        if let Some(error) = self.map_error.clone() {
            self.rendered = None;
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
            return Ok(());
        }
        if !self.picture_ready() {
            // No picture has been built yet; keep polling for it.
            if profiling() {
                profile_print(format!(
                    "PROFILE no_map frame {} epoch {} shown {:?}",
                    self.profile_frame,
                    self.epoch,
                    self.shown.as_ref().map(|shown| shown.epoch),
                ));
            }
            self.rendered = None;
            ui.ctx().request_repaint();
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return Ok(());
        }
        // The scene's presentation histories count rendered frames (the meshes created per frame,
        // the shadows the next frame's water refracts): a warm-up render would put them a frame on.
        if self.warming {
            self.rendered = None;
            return Ok(());
        }
        // With the playhead's map open, a reset composing: the picture is the one before it, drawn
        // at the perspective its reader drew it at.
        let (stale_hour, perspective) = match self.live_map() {
            Some(side) if side.composing() => (true, side.view().perspective.clone()),
            Some(_) => (false, self.perspective.clone()),
            None if self.shown.as_ref().is_some_and(|shown| shown.epoch != self.epoch) => (true, self.overlay_tables.as_ref().expect("installed with the hour").2.clone()),
            None => (false, self.perspective.clone()),
        };
        if stale_hour && profiling() {
            profile_print(format!(
                "PROFILE stale_hour frame {} epoch {}",
                self.profile_frame, self.epoch
            ));
        }
        // What the frame shows of the map mode's layers (counters, convoys, fog, models) is the
        // mode of the hour on screen, until an hour built for a newly chosen mode replaces it;
        // the builders were told the chosen one.
        let mode = match self.playhead_map.as_ref().filter(|side| side.failure().is_none()) {
            Some(side) => Some(side.mode_on_screen().unwrap_or(&self.map_mode)),
            None => self.placements_at.as_ref().map(|(_, _, mode)| mode),
        };
        if let Some(mode) = mode {
            self.counter_view.set_mode(mode);
        }
        laps.lap("hour_misc");
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.camera", Some("other"));
        // Only diagnostics and a capture's saving frame need warning text.
        let build_warnings = builds_warnings(self.diagnostics, self.capture.as_ref());
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let map_size = MapSize::of(&document.map);
        if !self.camera_manual {
            self.camera = None;
        }
        let defines = document.map_defines;
        let mut camera = self.camera.map_or_else(
            || Camera::fit(map_size, defines, pixels),
            |placement| placement.on(map_size, defines),
        );
        let camera = &mut camera;
        // Whether the map held the pointer for looking around before this frame's input.
        let held_pointer = self.input.captured();
        // One frame of keyboard, mouse and controller input for the camera controllers. A
        // capture's pinned camera takes none; a placed walk holds its keys on the capture clock.
        let placed_walk = pinned.is_some() && self.walk.placed();
        let controls = if pinned.is_some() {
            let time = self.capture.as_ref().and_then(|capture| capture.time);
            self.walk
                .placed_frame(time.unwrap_or(0.0))
                .unwrap_or_default()
        } else {
            let (events, focused, modifiers) =
                ui.input(|input| (input.events.clone(), input.focused, input.modifiers));
            self.input.frame(crate::controls::Window {
                events: &events,
                focused,
                modifiers,
                now: Instant::now(),
            })
        };
        // The map's own shortcut and the controller's view button cycle the camera modes, so a
        // window without the host's controls can change them too.
        if pinned.is_none()
            && !self.dashboard_map
            && (controls.pressed.contains(&CAMERA_MODE_KEY)
                || controls
                    .pad
                    .pressed(dashboard::protocol::GamepadButton::Select))
        {
            self.set_camera_mode(self.camera_mode.next(), self.free_overlays, ui.ctx());
        }
        // A window dump's placed walk walks, and its `--fly` flies, its pinned frame. A mode still
        // choosing its start shows the map camera.
        let free_mode = self.camera_mode != CameraMode::Map
            && !self.start_pick.choosing()
            && (pinned.is_none() || placed_walk || self.options.fly.is_some());
        // Whether the map's interface draws: always for the game's camera and its orbit; in fly
        // and walk, a capture's placed walk, flight or free camera, only when asked for.
        let interface = match pinned {
            Some(_) if free_mode => self.options.free_overlays.unwrap_or(false),
            Some((_, Some(placement))) if placement.pose != Pose::Map => {
                self.options.free_overlays.unwrap_or(false)
            }
            Some(_) => true,
            None => !free_mode || self.free_overlays,
        };
        // The fly mode shows the country names even with the rest of the interface hidden; they
        // fade by its eye's height as the game camera's do.
        let flying_names = free_mode && self.camera_mode == CameraMode::Fly;
        // Pointer and drag deltas reach the camera in framebuffer pixels.
        let scale = ui.ctx().pixels_per_point() / raster.stretch;
        let frame_point = |position: egui::Pos2| {
            [
                (position.x - rect.left()) * scale,
                (position.y - rect.top()) * scale,
            ]
        };
        // The interface is laid out in interface pixels, so its hit tests are
        // too: counters, combat bubbles, the combat window and its lists.
        let extent = ui_scale_extent(pixels, ui_scale);
        // Map keys go to one map: the one that owns keyboard input, as the camera keys do.
        let owner = ui
            .ctx()
            .data(|data| data.get_temp::<egui::Id>(egui::Id::new(MAP_INPUT_OWNER)));
        let takes_keys = takes_map_keys(self.dashboard_map, owner, response.id, response.hovered());
        // Escape first gives back a pointer held for looking around (the input layer does that);
        // only a later one reaches the map's interface. A mode choosing its start takes it.
        let escape = takes_keys && ui.input(|input| input.key_pressed(egui::Key::Escape));
        if escape && !held_pointer && !self.start_pick.choosing() {
            // Escape closes the tactics popup before the battle window.
            if self.combat.tactics_open {
                self.combat.tactics_open = false;
            } else {
                self.combat.close();
                self.unit_selection.clear();
                self.air_selection.clear();
                self.fleet_selection.clear();
            }
        }
        let engaged = self
            .engaged_history(&perspective.country)
            .map_err(|error| error.to_string())?;
        let animation_time = self
            .capture
            .as_ref()
            .and_then(|capture| capture.time)
            .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
        // Hit tests and drawing use the same GUI clock.
        let prepared = self
            .combat_prepared(document, &perspective, engaged, ui.ctx())
            .map_err(|error| error.to_string())?;
        let combat_geometry = self
            .combat
            .draw(&document.combat, prepared, extent, false, animation_time, false, None)
            .map_err(|error| error.to_string())?;
        let point = profile_hover_point(
            response
                .hover_pos()
                .or_else(|| response.interact_pointer_pos())
                .map(frame_point),
            self.options.profile_pointer,
            // The playback-cost harness hovers its point too, to time hover.
            profiling() || self.options.playback_cost_end.is_some(),
            pixels,
            ui_scale,
        );
        // The interface's pointer, with the primary button's changes in order: a press counts
        // only where it lands on this map; a release, wherever it happens. Held for looking
        // around, the pointer is away from the interface.
        let reaches = !free_mode || (interface && !self.input.captured());
        // The pointer's moves (`None`) and its primary and secondary changes, in order.
        let (changes, held, input_time) = ui.input(|input| {
            let changes: Vec<(egui::Pos2, Option<(egui::PointerButton, bool)>)> = input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::PointerMoved(pos) => Some((*pos, None)),
                    egui::Event::PointerButton {
                        pos,
                        button: button @ (egui::PointerButton::Primary
                        | egui::PointerButton::Secondary),
                        pressed,
                        ..
                    } => Some((*pos, Some((*button, *pressed)))),
                    _ => None,
                })
                .collect();
            (changes, input.pointer.primary_down(), input.time)
        });
        let on_map = |pos: egui::Pos2| {
            response.interact_rect.contains(pos)
                && ui.ctx().layer_id_at(pos) == Some(response.layer_id)
        };
        // A button's changes with their order, the index among the frame's pointer events.
        let admitted = |button: egui::PointerButton| -> Vec<(usize, egui::Pos2, bool)> {
            changes
                .iter()
                .enumerate()
                .filter_map(|(order, &(pos, change))| {
                    let (which, pressed) = change?;
                    (which == button && (!pressed || on_map(pos))).then_some((order, pos, pressed))
                })
                .collect()
        };
        let primary = if reaches {
            admitted(egui::PointerButton::Primary)
        } else {
            Vec::new()
        };
        let secondary = if reaches {
            admitted(egui::PointerButton::Secondary)
        } else {
            Vec::new()
        };
        let interface_at = |pos: egui::Pos2| to_interface(frame_point(pos), ui_scale);
        let gui_input = Some(reaches.then(|| {
            use viewer_foundation::map::native_gui::pointer::{Button, Input, Place};
            Input {
                place: point.map(Place::new),
                buttons: primary
                    .iter()
                    .map(|&(order, pos, pressed)| Button {
                        down: pressed,
                        place: Some(Place::new(interface_at(pos))),
                        order,
                    })
                    .collect(),
                held,
            }
        }).unwrap_or_default());
        if takes_keys {
            self.picker_keys(ui, document, extent);
        }
        // The picker's lists take no scrollbar, so the wheel over them does nothing of theirs.
        // Primary clicks are the windows' own: a release over the button its press went down on,
        // acted on after the windows drew it.
        let picker_controls = match &self.picker_controls {
            Some((drawn_at, controls)) if !self.picker.hidden && *drawn_at == extent => {
                controls.clone()
            }
            _ => match self.draw_picker(document, extent, false, animation_time, false, None) {
                Some(Ok(drawn)) => drawn.controls,
                _ => Vec::new(),
            },
        };
        let picker_placed_at = |at: [f32; 2]| {
            picker_controls
                .iter()
                .rev()
                .find(|placed| placed.contains(at))
                .cloned()
        };
        let picker_at = |at: [f32; 2]| picker_placed_at(at).map(|placed| placed.control);
        let holder = if self.picker.picker.holding() {
            Some(Holder::Picker)
        } else if self.combat.holding() {
            Some(Holder::Combat)
        } else {
            self.counter_selecting.map(|_| Holder::Counters)
        };
        let owner = |at: [f32; 2]| {
            if picker_at(at).is_some() {
                Some(Holder::Picker)
            } else if combat_geometry.as_ref().is_some_and(|w| w.contains(at)) {
                Some(Holder::Combat)
            } else {
                matches!(self.map_hit(at), Some(MapHit::Unit(_))).then_some(Holder::Counters)
            }
        };
        let [picker_input, combat_input, counters_input] =
            [Holder::Picker, Holder::Combat, Holder::Counters].map(|me| {
                gui_input
                    .as_ref()
                    .map(|input| input.shared(holder, &owner, me))
            });
        // Each release ends the press it follows, which went down on its owner or the bare map,
        // and is a click within egui's distance and time of it: where the last bare-map click
        // ended, and the first click off the picker, by order.
        let (click_dist, click_time) = ui
            .ctx()
            .options(|options| {
                (options.input_options.max_click_dist, options.input_options.max_click_duration)
            });
        let mut pressed_on = self.pressed_on;
        let mut bare_clicks = Vec::new();
        let mut off_picker_clicks = Vec::new();
        for (this, &(pos, change)) in changes.iter().enumerate().filter(|_| reaches) {
            let Some((egui::PointerButton::Primary, pressed)) = change else {
                // A press the pointer strays too far from is a drag.
                if change.is_none()
                    && let Some((_, from, _, far)) = &mut pressed_on
                {
                    *far |= pos.distance(*from) > click_dist;
                }
                continue;
            };
            if pressed && !on_map(pos) {
                continue;
            }
            let at = interface_at(pos);
            if pressed {
                pressed_on = Some((owner(at), pos, input_time, false));
                continue;
            }
            if let Some((on, from, since, far)) = pressed_on.take()
                && !far
                && pos.distance(from) <= click_dist
                && input_time - since <= click_time
            {
                if on != Some(Holder::Picker) {
                    off_picker_clicks.push(this);
                }
                if on.is_none() && owner(at).is_none() {
                    bare_clicks.push((at, this));
                }
            }
        }
        if !held || !reaches {
            pressed_on = None;
        }
        self.pressed_on = pressed_on;
        // A right press on a picker slot configures it on its release over that slot, in order.
        let mut secondary_clicks = Vec::new();
        for &(order, pos, pressed) in &secondary {
            let placed = picker_placed_at(interface_at(pos));
            if pressed {
                self.secondary_on = placed.map(|placed| placed.control);
            } else if let Some(pressed_on) = self.secondary_on.take()
                && let Some(placed) = placed.filter(|placed| placed.control == pressed_on)
                && matches!(placed.control, viewer_combat::picker::Control::Slot(_))
                && !placed.disabled
            {
                secondary_clicks.push((order, placed));
            }
        }
        if !reaches || !ui.input(|input| input.pointer.secondary_down()) {
            self.secondary_on = None;
        }
        let picker_hit = point.and_then(picker_at);
        let over_picker = picker_hit.is_some();
        if response.drag_started() && !over_picker {
            self.combat.start_drag(combat_geometry.as_ref(), point);
        }
        if response.drag_stopped() {
            self.combat.dragging = false;
        }
        let over_combat = self.combat.dragging
            || over_picker
            || point.is_some_and(|point| {
                combat_geometry
                    .as_ref()
                    .is_some_and(|window| window.contains(point))
            });
        let modifiers = ui.input(|input| input.modifiers);
        self.combat.set_modifiers(modifiers.shift, modifiers.ctrl);
        if over_combat {
            if response.dragged() && self.combat.dragging {
                let delta = response.drag_delta();
                self.combat
                    .drag_by([delta.x * scale / ui_scale, delta.y * scale / ui_scale]);
            }
            if let Some(side) = point.and_then(|point| {
                combat_geometry
                    .as_ref()
                    .and_then(|window| window.list_at(point))
            }) {
                let scroll = ui.input(|input| {
                    input
                        .raw
                        .events
                        .iter()
                        .filter_map(|event| match event {
                            egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                            _ => None,
                        })
                        .sum::<f32>()
                });
                if scroll < 0.0 {
                    self.combat.scroll(side.clone(), 1, true);
                }
                if scroll > 0.0 {
                    self.combat.scroll(side, -1, true);
                }
            }
        }
        // The frame time the camera law and the name fade integrate.
        let frame_delta = self
            .camera_step
            .unwrap_or_else(|| self.last_frame.elapsed().as_secs_f32());
        self.last_frame = Instant::now();
        // The game's keys and edge scrolling go to the map that owns camera input: the window must
        // have the system's focus, and a dashboard map must also be the one last pressed with a
        // mouse button. Ownership is kept in the window's own record, apart from egui's keyboard
        // focus, so Escape or Tab never moves or drops it. The wheel goes to the map under the
        // pointer.
        let owner = egui::Id::new(MAP_INPUT_OWNER);
        if response.is_pointer_button_down_on() {
            ui.ctx()
                .data_mut(|data| data.insert_temp(owner, response.id));
        }
        let owns = ui.ctx().data(|data| data.get_temp::<egui::Id>(owner)) == Some(response.id);
        let window_focused = ui.input(|input| input.focused);
        let focused = window_focused && (!self.dashboard_map || owns);
        let pointed = response.hovered() && !over_combat;
        // A mode chosen from the map starts over the ground clicked next, or under the view's
        // middle at the controller's South button; Escape goes back to the map.
        let mut started = false;
        if self.start_pick.choosing() {
            let shown = self.orbit.view(*camera);
            let field = viewer_foundation::map::province_map::HeightField {
                width: document.map.width,
                height: document.map.height,
                values: document.assets.heightmap_texels(),
            };
            // The drawn ground the ray meets first, across the east-west wrap too; a click on no
            // ground, as above the horizon or past the map's top or bottom, chooses nothing.
            let ground = |screen: [f32; 2]| drop::ground_at(&shown, screen, pixels, &field);
            let clicked = response
                .interact_pointer_pos()
                .filter(|_| response.clicked() && !over_combat)
                .and_then(|position| ground(frame_point(position)));
            let south = controls
                .pad
                .pressed(dashboard::protocol::GamepadButton::South);
            let centre = south
                .then(|| ground([pixels[0] as f32 * 0.5, pixels[1] as f32 * 0.5]))
                .flatten();
            match drop::choose(escape, clicked, south, centre, camera.map.x()) {
                drop::Choice::Wait => {}
                drop::Choice::Cancel => {
                    self.set_camera_mode(CameraMode::Map, self.free_overlays, ui.ctx());
                }
                drop::Choice::Start(at) => {
                    self.start_pick = drop::Pick::At(at);
                    // The press that chose the start is no half of a double tap in the mode.
                    self.input.forget_pad_taps();
                    started = true;
                    ui.ctx().request_repaint();
                }
            }
        }
        // A free mode takes the pointer for looking around when the map is clicked, unless the
        // click lands on a map icon it shows or on the combat window.
        let icon_click = free_mode
            && interface
            && response.clicked()
            && !self.input.captured()
            && point.is_some_and(|point| {
                matches!(
                    self.map_hit(point),
                    Some(MapHit::Combat(_) | MapHit::Unit(_))
                )
            });
        if free_mode
            && !icon_click
            && !over_combat
            && (response.clicked() || response.drag_started())
        {
            self.input.capture();
        }
        // Drag panning is the viewer's own addition (the game has none): the middle button moves
        // the shown camera directly, and the target with it. Through an orbiting view it keeps
        // the ground point under the pointer.
        if !free_mode
            && response.dragged_by(egui::PointerButton::Middle)
            && !over_combat
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.camera_manual = true;
            let delta = response.drag_delta();
            let to = frame_point(pointer);
            let from = [to[0] - delta.x * scale, to[1] - delta.y * scale];
            let before = camera.center;
            if self.orbit.is_native() {
                camera.pan(from, to, pixels);
            } else {
                self.orbit.pan(camera, from, to, pixels);
            }
            self.zoom
                .shift([camera.center[0] - before[0], camera.center[1] - before[1]]);
        }
        let texels = document.assets.heightmap_texels();
        let raster_fits =
            texels.len() as u64 == u64::from(document.map.width) * u64::from(document.map.height);
        let floor = |x: f32, z: f32| {
            if raster_fits {
                terrain_height(&document.map, texels, [x, z])
            } else {
                0.0
            }
        };
        let law = &document.camera_law;
        // Wheel reports (in lines, as the window system gives them) and zoom-key presses, repeats
        // included, in order; the held arrow keys.
        let (lines, zoom_keys, keys) = ui.input(|input| {
            let mut lines = Vec::new();
            let mut zoom_keys = Vec::new();
            for event in &input.raw.events {
                match event {
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta,
                        ..
                    } => lines.push(delta.y),
                    egui::Event::Key {
                        key: key @ (egui::Key::PageUp | egui::Key::PageDown),
                        pressed: true,
                        ..
                    } => zoom_keys.push(*key == egui::Key::PageUp),
                    _ => {}
                }
            }
            let keys = Pan {
                up: input.key_down(egui::Key::ArrowUp),
                down: input.key_down(egui::Key::ArrowDown),
                left: input.key_down(egui::Key::ArrowLeft),
                right: input.key_down(egui::Key::ArrowRight),
            };
            (lines, zoom_keys, keys)
        });
        // The game's law takes the keyboard and the wheel while the view is the game's own. An
        // orbiting view takes the arrow keys, edge scrolling and the wheel through the orbit, and
        // a free mode leaves the map camera where it was; the zoom keys always set the height.
        let native = !free_mode && self.orbit.is_native();
        if focused && !free_mode {
            for zoom_in in zoom_keys {
                self.camera_manual = true;
                self.zoom.key(camera, law, zoom_in, &floor);
            }
        }
        let notches = if pointed && native {
            self.zoom.wheel_events(lines)
        } else {
            Vec::new()
        };
        // The map pressed, dragged or wheeled is the one that sounds.
        self.interacted = response.clicked() || response.dragged() || !notches.is_empty();
        let pointer = response.hover_pos().map(frame_point);
        let ground = match pointer {
            Some(point) if !notches.is_empty() => {
                motion_ground_point(camera, point, pixels, &document.map, texels)
                    .ok()
                    .flatten()
            }
            _ => None,
        };
        let held = if focused && !free_mode {
            keys
        } else {
            Pan::default()
        };
        let edge = pointer.filter(|&point| focused && pointed && law.edge_scrolls(point, pixels));
        let input = MotionInput {
            cursor: pointer.filter(|_| focused && pointed && native),
            notches: &notches,
            ground,
            keys: if native { held } else { Pan::default() },
        };
        let stepped = self
            .zoom
            .step(camera, law, frame_delta, pixels, &input, &floor);
        let moved = self.zoom.ease(camera, law, frame_delta, pixels, &floor);
        // Only a real camera move leaves the fitted camera; hovering or pressing does not.
        if stepped || moved {
            self.camera_manual = true;
            ui.ctx().request_repaint();
        }
        // The controller orbits the map camera; in the free modes it moves the free eye.
        if !free_mode {
            let mut orbited = false;
            let mut push = [0.0f32; 2];
            if !self.orbit.is_native() {
                let axis = |low: bool, high: bool| f32::from(i8::from(high) - i8::from(low));
                push = [axis(held.left, held.right), axis(held.down, held.up)];
                if let Some([x, y]) = edge {
                    let toward = [x - pixels[0] as f32 * 0.5, pixels[1] as f32 * 0.5 - y];
                    let length = toward[0].hypot(toward[1]);
                    if length > 0.0 {
                        push = [push[0] + toward[0] / length, push[1] + toward[1] / length];
                    }
                }
                let scroll = ui.input(|input| input.smooth_scroll_delta.y);
                if pointed
                    && scroll != 0.0
                    && let Some(point) = pointer
                {
                    self.orbit.zoom(
                        camera,
                        (scroll * crate::controls::orbit::WHEEL_ZOOM).exp(),
                        point,
                        pixels,
                    );
                    orbited = true;
                }
            }
            orbited |= self.orbit.update(&controls, camera, push);
            if orbited {
                // The controller takes over from the game's target, as a camera placed afresh.
                self.zoom.reset();
                self.camera_manual = true;
                ui.ctx().request_repaint();
            }
        }
        // As the game does each frame, a view wholly past a seam moves back a map width with its
        // target, so panning round the world never outruns the map copies the passes draw.
        let shift = viewer_foundation::map::province_admission::seam_shift(*camera, pixels);
        if shift != 0.0 {
            camera.center[0] += shift;
            self.zoom.shift([shift, 0.0]);
        }
        self.camera = Some(camera.placement());
        // The camera the map draws with, and the one its interface is laid out for: the free
        // modes draw from the free eye, and keep the map's interface laid out for the map camera
        // it left, without drawing it.
        let (camera, overlay_camera) = match pinned.filter(|_| !placed_walk) {
            Some((size, camera)) => {
                let camera = camera.map_or_else(
                    || Camera::fit(map_size, defines, size),
                    |placement| placement.on(map_size, defines),
                );
                match self.options.fly {
                    // A window dump's flight steps by the capture clock, holding its controls.
                    Some(scripted) if free_mode => {
                        let time = self.capture.as_ref().and_then(|capture| capture.time);
                        let dt = match (time, self.fly_clock) {
                            (Some(time), Some(last)) => (time - last).max(0.0),
                            _ => 0.0,
                        };
                        self.fly_clock = time;
                        let eye = self.fly(document, camera, None, &scripted.frame(dt));
                        (eye, if interface { eye } else { camera })
                    }
                    _ => (camera, camera),
                }
            }
            None => {
                let map = self.orbit.view(*camera);
                if free_mode {
                    let texels = document.assets.heightmap_texels();
                    let field = viewer_foundation::map::province_map::HeightField {
                        width: document.map.width,
                        height: document.map.height,
                        values: texels,
                    };
                    let floor = |x: f32, z: f32| field.surface_height(x, z);
                    let eye = if self.camera_mode == CameraMode::Walk {
                        // The viewing country's infantry, chosen again for each hour and view.
                        // With the map open: rows at the head and the vision layer's viewing
                        // country, the figure kept while either loads.
                        let vision = if live_of(&self.playhead_map).is_some() { self.shown_vision().ok() } else { None };
                        let head = head_rows::head(live_of(&self.playhead_map), document);
                        let key = match head {
                            Some((_, recording)) => self.head_rows.walk_hour(head).map(|hour| (recording.branch, hour, perspective.clone())),
                            None => Some((document.playback.branch(), document.playback.hour(), perspective.clone())),
                        };
                        if let Some(key) = key.filter(|key| self.walk.stale(key)) {
                            let figure = match head {
                                Some(_) => vision.and_then(|vision| self.head_rows.walk(head, None, |frame| document.unit_models.infantry_figure(frame, vision.ours()))),
                                None => Some(
                                    document
                                        .hour()
                                        .and_then(|now| now.vision_from_overlay(&perspective, self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables)).map(|vision| (now.frame, vision.ours())))
                                        .map_err(|error| error.to_string())
                                        .and_then(|(frame, ours)| document.unit_models.infantry_figure(frame, ours)),
                                ),
                            };
                            if let Some(figure) = figure {
                                self.walk.set_figure(key, figure);
                            }
                        }
                        // The wheel moves the view nearer or farther over the map.
                        let wheel = if response.hovered() {
                            ui.input(|input| input.smooth_scroll_delta.y)
                        } else {
                            0.0
                        };
                        // Walking starts on the ground chosen for it, or the ground the view looks
                        // at, facing the way the view faces.
                        let world = crate::controls::walker::World::of(&map, &floor);
                        let start = self.start_pick.at().unwrap_or(camera.center);
                        self.walk.camera(&controls, wheel, map, start, &world)
                    } else {
                        // The input layer keeps the keyboard and mouse to a focused map, and the
                        // controller to wherever the host routes it.
                        self.fly(document, map, self.start_pick.at(), &controls)
                    };
                    // Shown, the interface stands in the free eye's world; hidden, it stays laid
                    // out for the map camera the mode left, undrawn.
                    (eye, if interface { eye } else { map })
                } else {
                    (map, map)
                }
            }
        };
        self.grab_pointer(ui.ctx());
        // The game shows its own arrow wherever no other cursor is asked for; looking around hides
        // it, which a cursor image would show again.
        if let Ok(cursor) = &document.content.cursor
            && response.hovered()
            && !self.pointer_grabbed
        {
            ui.ctx()
                .output_mut(|output| output.cursor_image = Some(cursor.clone()));
        }
        laps.lap("panel_camera");
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.movers", Some("other"));
        let live = self.live_map().is_some();
        self.rejoin_ships();
        let scene = self.scene.as_mut().expect("created above");
        // The frame a capture saves draws complete, built in place; live frames build elsewhere.
        let saving = self.options.window_dump
            || self
                .capture
                .as_ref()
                .is_some_and(|capture| capture.remaining == 0);
        if saving != self.settled_capture {
            scene.settle_builds(saving);
            self.settled_capture = saving;
        }
        // A `--window-dump` settles unit models for its camera, as `--dump`
        // does; a host export keeps the live history. With the map open the units layer installs
        // the field: no field of today's is uploaded.
        if let Some(today) = self.unit_models.as_ref().filter(|_| today_field_due(live, self.unit_field_uploaded)) {
            let timer = Instant::now();
            let units = today.field(&document.unit_models);
            laps.lap("units_field");
            let units_cpu = viewer_foundation::profiling::thread_span("units_upload");
            scene
                .apply(
                    &state.device,
                    &state.queue,
                    SceneInput::SetUnitModels {
                        assets: &document.unit_models,
                        field: units,
                        steady: self.options.window_dump,
                    },
                )
                .map_err(|error| error.to_string())?;
            units_cpu.finish();
            laps.lap("units_upload");
            self.unit_field_uploaded = true;
            // The field placed the raid units at the hour's start.
            self.raid_tick = Some(0.0);
            if profiling() {
                profile_print(format!(
                    "PROFILE models {:.6}",
                    timer.elapsed().as_secs_f64() * 1000.0
                ));
            }
        }
        // Raid movement uses the replay fraction; ordinary units move on the view's animators.
        let tick = if live { 0.0 } else { document.hour_fraction };
        // The trade-route convoys sail by the presentation clock, so while they show they are
        // placed again every frame, with the raid units.
        let convoys_shown = |defines: &viewer_world::map::trade_routes::ConvoyDefines| {
            viewer_world::map::trade_routes::convoys_shown(
                camera.height,
                defines,
                self.graphics.units,
                self.counter_view.visibility.trade_routes,
            )
        };
        // With the map open its units layer's models carry the raid units and convoys, and nothing
        // of today's does.
        let posed = match self.playhead_map.as_ref().filter(|_| live) {
            Some(side) => crate::layers::units::prepared(side).cloned(),
            None => self.unit_models.clone(),
        };
        let sailing = posed.as_ref().is_some_and(|prepared| {
            prepared.has_convoys()
                && document
                    .trade_routes
                    .as_ref()
                    .is_ok_and(|defines| convoys_shown(&defines.convoys))
        });
        if (self.raid_tick != Some(tick) || sailing || self.convoys_sailing)
            && let Some(prepared) = posed.as_ref()
        {
            let heights = document.content.heights();
            let mut units = prepared.raid_units(&document.unit_models, Some((&heights, tick)));
            let time = self
                .capture
                .as_ref()
                .and_then(|capture| capture.time)
                .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
            self.convoy_refused.clear();
            units.extend(prepared.convoy_units(
                &document.unit_models,
                time,
                &|x, z| heights.surface_height(x, z),
                convoys_shown,
                &mut self.convoy_refused,
            ));
            scene
                .apply(&state.device, &state.queue, SceneInput::SetRaidUnits(units))
                .map_err(|error| error.to_string())?;
            self.raid_tick = Some(tick);
            self.convoys_sailing = sailing;
        }
        laps.lap("movers_place");
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.selection", Some("other"));
        // names: the engine integrates the layer cross-fade by the frame delta
        // and clamps to [0, 1], so it takes one second whatever the zoom speed.
        // It has to start from a settled value, and only this frame knows which:
        // a window given no `--camera` fits the map to a viewport that does not
        // exist until there is one, and a replay opened after a zoom starts at a
        // camera the last one left behind.
        let fade = self
            .fade
            .get_or_insert_with(|| LayerFade::instant(camera))
            .step(camera, frame_delta);
        self.fade = Some(fade);
        laps.lap("names_fade");
        // The viewing country: the vision the open map's vision layer shows, today's hour's with no
        // open map. While the layer shows none the selections keep what they hold.
        let today = if live { None } else { Some(document.hour().map_err(|error| error.to_string())?) };
        let seen = match &today {
            Some(now) => Some(now.vision_from_overlay(&perspective, self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables)).map_err(|error| error.to_string())?),
            None => self.shown_vision().ok(),
        };
        let head = head_rows::head(live_of(&self.playhead_map), document);
        let branch = head.map_or_else(|| document.playback.branch(), |(_, recording)| recording.branch);
        let frame = today.as_ref().map(|now| now.frame);
        if let Some(ours) = seen.as_ref().map(|vision| vision.ours()) {
            if self.unit_selection.units().is_empty() {
                self.unit_selection.context(branch, &perspective.country, ours);
            } else {
                self.head_rows.units(head, frame, |frame| self.unit_selection.observe(branch, &perspective.country, ours, frame));
            }
            if self.air_selection.wings().is_empty() && self.air_selection.bases().is_empty() {
                self.air_selection.context(branch, &perspective.country, ours);
            } else {
                self.head_rows.air(head, frame, |frame| self.air_selection.observe(branch, &perspective.country, ours, frame));
            }
            if self.fleet_selection.fleets().is_empty() {
                self.fleet_selection.context(branch, &perspective.country, ours);
            } else {
                self.head_rows.fleets(head, frame, |frame| self.fleet_selection.observe(branch, &perspective.country, ours, frame));
            }
        }
        self.counter_view.selected = Arc::clone(self.unit_selection.units());
        laps.lap("unit_selection");
        self.counter_view.air_selected = Arc::clone(self.air_selection.wings());
        self.counter_view.air_bases = Arc::clone(self.air_selection.bases());
        laps.lap("air_selection");
        self.counter_view.fleets_selected = Arc::clone(self.fleet_selection.fleets());
        laps.lap("fleet_selection");
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.name-hover", Some("other"));
        // A free mode's pointer reaches the map's interface while it is shown and the pointer is
        // not held for looking around.
        let point = point.filter(|_| !free_mode || (interface && !self.input.captured()));
        let counters_point = point.filter(|_| {
            counters_input
                .as_ref()
                .is_none_or(|input| input.place.is_some())
        });
        self.counter_view.hovered = counters_point.filter(|_| !over_combat).and_then(|point| {
            let MapHit::Unit(index) = self.map_hit(point)? else {
                return None;
            };
            self.counter_report.unit_hits[index].units().next()
        });
        let frame_sounds = self.interface_sounds.len();
        // The counter the primary button went down on, where it went down, stays pressed until
        // the release. Each step changes its plate's state as a button's, with its sounds.
        let was = (self.counter_view.pressed, self.counter_view.plate_hovered);
        let mut held = self.counter_held;
        let mut counter_sounds = Vec::new();
        let mut counter_clicks: Vec<(Unit, usize)> = Vec::new();
        let plate = |pressed: Option<Unit>, hovered: Option<Unit>| {
            pressed
                .map(|unit| (unit, true))
                .or(hovered.map(|unit| (unit, false)))
        };
        let mut change =
            |next: Option<(Unit, bool)>, order: usize, sounds: &mut Vec<(Unit, Sound, usize)>| {
                if held == next {
                    return;
                }
                if let Some((before, _)) = held
                    && next.is_none_or(|(after, _)| after != before)
                {
                    sounds.push((before, Sound::Up, order));
                }
                if let Some((after, down)) = next {
                    let sound = if down { Sound::Down } else { Sound::Over };
                    sounds.push((after, sound, order));
                }
                held = next;
            };
        // The counter under a place, and whether its `count_bg` itself is: a counter's other
        // parts forward its selection but leave its button's state.
        let counter_at = |at: [f32; 2]| {
            if picker_at(at).is_some() || combat_geometry.as_ref().is_some_and(|w| w.contains(at))
            {
                return None;
            }
            let target = self.map_hit(at)?;
            let MapHit::Unit(index) = target else {
                return None;
            };
            let hit = &self.counter_report.unit_hits[index];
            let local = self.local_point(self.hit_layer(target), at);
            Some((hit.units().next()?, hit.on_plate(local)))
        };
        let plate_of = |under: Option<(Unit, bool)>| under.filter(|(_, on)| *on).map(|(u, _)| u);
        let unders: Vec<_> = counters_input
            .iter()
            .flat_map(|input| &input.buttons)
            .map(|button| button.place.and_then(|place| counter_at(place.at)))
            .collect();
        let plate_hovered = plate_of(
            counters_point
                .filter(|_| !over_combat)
                .and_then(counter_at),
        );
        if let Some(input) = &counters_input {
            for (button, &under) in input.buttons.iter().zip(&unders) {
                if !button.down {
                    if let Some(selecting) = self.counter_selecting
                        && under.map(|(unit, _)| unit) == Some(selecting)
                    {
                        counter_clicks.push((selecting, button.order));
                    }
                    if let Some(pressed) = self.counter_view.pressed
                        && plate_of(under) == Some(pressed)
                    {
                        counter_sounds.push((pressed, Sound::Click, button.order));
                    }
                }
                self.counter_selecting = under.filter(|_| button.down).map(|(unit, _)| unit);
                self.counter_view.pressed = plate_of(under).filter(|_| button.down);
                change(
                    plate(self.counter_view.pressed, plate_of(under)),
                    button.order,
                    &mut counter_sounds,
                );
            }
            if !input.held {
                self.counter_selecting = None;
                self.counter_view.pressed = None;
            }
        }
        self.counter_view.plate_hovered = plate_hovered;
        change(
            plate(self.counter_view.pressed, self.counter_view.plate_hovered),
            viewer_foundation::map::native_gui::pointer::LAST,
            &mut counter_sounds,
        );
        self.counter_held = held;
        for (unit, sound, order) in counter_sounds {
            let control = highlight::HoverTarget::Counter(unit).to_string();
            if let Ok(sounds) = document.icons.counters.plate_sounds(unit)
                && let Some(played) = sound.played(&control, &sounds, order)
            {
                self.interface_sounds
                    .push(InterfaceSound::new("counter", &played));
            }
        }
        // A plate's state draws the laid-out counters again; it changes nothing they were
        // prepared with.
        if (self.counter_view.pressed, self.counter_view.plate_hovered) != was {
            viewer_foundation::map::redraws::note("asked.pointer", 1);
            self.overlay_fraction = None;
        }
        self.last_view = Some(hours::View {
            camera: overlay_camera,
            viewport: pixels,
            ui_scale,
        });
        let hovered_name_province = point
            .filter(|_| !document.content.game_strategic_names)
            .and_then(|point| {
                pick(
                    &camera,
                    point.map(|v| v * ui_scale),
                    pixels,
                    &document.map,
                    document.assets.heightmap_texels(),
                )
                .ok()
                .flatten()
            });
        let hovered_name_region = hovered_name_province
            .map(|province| document.content.counters.region(province).unwrap_or(0));
        let names_mode = self.shown_mode();
        let strategic_names = matches!(
            names_mode,
            viewer_world::map::map_mode::MapMode::Stock(
                viewer_world::map::map_mode::Stock::StrategicAir
                    | viewer_world::map::map_mode::Stock::StrategicNavy
            )
        );
        let show_all_names = document.content.game_strategic_names || !strategic_names;
        let strategic_names = if show_all_names {
            StrategicNames::All
        } else {
            StrategicNames::Hover(hovered_name_province)
        };
        let name_fade = fade.strategic_hover(
            show_all_names,
            hovered_name_region,
            self.hovered_name_region,
        );
        self.fade = Some(name_fade);
        self.hovered_name_region = hovered_name_region;
        laps.lap("names_pick");
        drop(_other_cost);
        let overlay_prepare_cpu = viewer_foundation::profiling::thread_span("overlay_prepare");
        // counters: one batch carries the bases, the labels, the counters and
        // the combat icons in the map mode's `<table>_PRIORITY_*` order.
        // What the map's layers draw is neither made by today's maker nor read at the head for it.
        let left = crate::document::LeftToLayers::of(live_of(&self.playhead_map));
        // With the map open the overlay is made from rows at the head alone: its reader's newest,
        // or the rows it was last made from while they load; none before its first rows.
        // A frame that is saved, one an hour (a stepped run, a window dump), has the open map's
        // overlay made from rows at the hour on screen: every table its tables and layout read is
        // learned within the frame, or the frame fails by name.
        let in_place = self.options.window_dump || self.capture.as_ref().is_some_and(|capture| !capture.paced);
        let mut settled_overlay = None;
        if in_place && live {
            // The vision layer's vision of this frame, not the hovers' (remade at most four times a second).
            let vision = live_of(&self.playhead_map)
                .and_then(crate::layers::vision::drawn)
                .ok_or_else(|| "The overlay at the head: the map draws no vision layer".to_owned())?
                .shown()
                .map(Arc::new)
                .map_err(|why| format!("The overlay at the head: the vision layer shows no vision: {why}"))?;
            let engaged = self.engaged_history(&perspective.country).map_err(|error| error.to_string())?;
            let (_, hour) = self.hour_on_screen(document);
            let labels = live_of(&self.playhead_map).is_some_and(|side| side.draws("labels"));
            let left_out = self.counter_view.layered();
            let need = overlay_need(&self.counter_view, overlay_camera.free().is_some());
            let whole = head_rows::OverlayInputs::whole(&self.counter_view);
            let inputs = whole.without(left.resource_icons, left.occupation_marks).unwrap_or(whole);
            let head = head_rows::head(live_of(&self.playhead_map), document);
            let (content, rosters, view, scripts) = (&document.content, document.playback.air_rosters(), &self.counter_view, &self.scripts);
            let made = |rows: &Arc<replay_frame::frame::DecodedFrame>| {
                let at = crate::document::Hour::new(content, rows, 0.0).with_air_rosters(Some(Arc::clone(&rosters)));
                let raids = scripts.raids(rows.branch, vision.ours()).unwrap_or_default();
                let (tables, missed) = replay_frame::frame::discovering(|| at.hour_tables_at_head(&perspective, Arc::clone(&vision), raids, engaged.as_deref(), left_out, labels, need, &mut viewer_foundation::profiling::BuilderSpans::new(false)));
                if !missed.is_empty() {
                    return (None, missed);
                }
                let tables = match tables {
                    Ok(tables) => tables,
                    Err(error) => return (Some(Err(error)), Vec::new()),
                };
                let trial = crate::document::OverlayRows::at_head(Some(Arc::clone(rows))).leaving(left);
                let (laid, mut missed) = replay_frame::frame::discovering(|| at.prepare_overlay_at(overlay_camera, pixels, ui_scale, &tables, view, &Default::default(), &trial));
                missed.extend(trial.missed());
                missed.sort_unstable_by_key(|table| table.index());
                missed.dedup();
                (Some(Ok((tables, laid))), missed)
            };
            let (rows, made) = self.head_rows.overlay_settled(inputs, head, hour, Duration::from_secs(120), made).map_err(|error| error.to_string())?;
            match made {
                Some(Ok((tables, laid))) => {
                    let key = TablesKey {
                        branch: rows.branch,
                        hour: rows.hour,
                        rows: Arc::as_ptr(&rows) as usize,
                        perspective: perspective.clone(),
                        vision: Arc::as_ptr(&vision) as usize,
                        engaged: engaged.as_deref().is_some_and(|index| index.covers(rows.branch, rows.hour)),
                        left_out,
                        labels,
                        need,
                    };
                    self.overlay_tables = Some((key.branch, key.hour, key.perspective.clone(), tables.clone()));
                    settled_overlay = Some(laid);
                    // The other counter layers take this hour's vision and tables before the
                    // layout reads their products, as today's took the hour it had just installed.
                    if let Some(side) = self.playhead_map.as_mut() {
                        let passes = || -> crate::layers::air_mission_counters::Passes { document.playback.air_rosters() };
                        if crate::layers::counter_kinds::give(side, &key.perspective, tables.vision(), tables.counters(), key.branch, passes) {
                            side.settle_all().map_err(|error| format!("The map's counter layers could not settle the vision of hour {hour}: {error}"))?;
                            settled_overlay = None;
                        }
                        // Settled here, within the saved frame.
                        (self.counters_given, self.counters_drawn) = (Some((key.hour, key.rows)), Some((key.hour, key.rows)));
                        let (published, draws) = (crate::layers::counter_kinds::published(side), crate::layers::counter_kinds::draws(side));
                        if published.identity() != self.counter_view.presentation.counters.lock().unwrap_or_else(|error| error.into_inner()).layered().identity() {
                            settled_overlay = None;
                        }
                        self.counter_view.presentation.counters.lock().unwrap_or_else(|error| error.into_inner()).set_layered(published, draws, tables.counters().left_out());
                    }
                    self.head_tables = Some((key, tables));
                    self.overlay_at = None;
                }
                Some(Err(error)) => return Err(format!("The overlay's tables at hour {hour}: {error}")),
                None => {}
            }
            self.overlay_frame = Some(rows);
        }
        let overlay_rows = match head_rows::head(live_of(&self.playhead_map), document) {
            // A saved frame's: the rows settled above.
            _ if in_place && live => self.overlay_frame.clone(),
            head @ Some(_) => {
                if let Some(rows) = self.head_rows.overlay(head_rows::OverlayInputs::whole(&self.counter_view).without(left.resource_icons, left.occupation_marks), head) {
                    self.overlay_frame = Some(rows);
                }
                self.overlay_frame.clone()
            }
            None => {
                (self.overlay_frame, self.head_tables) = (None, None);
                self.overlay_loading.clear();
                self.overlay_missed.clear();
                self.head_rows.overlay(head_rows::OverlayInputs::of(&self.counter_view).and_then(|inputs| inputs.without(left.resource_icons, left.occupation_marks)), None)
            }
        };
        let now = match (&overlay_rows, today) {
            (Some(rows), _) if live => crate::document::Hour::new(&document.content, rows, 0.0).with_air_rosters(Some(document.playback.air_rosters())),
            (_, Some(now)) => now,
            // The open map before its overlay's first rows at the head: nothing is laid out yet.
            _ => {
                self.rendered = None;
                ui.ctx().request_repaint();
                return Ok(());
            }
        };
        let overlay_key = OverlayKey {
            rows: overlay_rows.as_ref().map(|rows| (rows.hour, Arc::as_ptr(rows) as usize)),
            occupation: self.counter_view.occupation,
            operatives: self.counter_view.operatives,
            branch: if live { now.frame.branch } else { document.playback.branch() },
            hour: if live { now.frame.hour } else { document.playback.hour() },
            camera: overlay_camera,
            pixels,
            ui_scale,
            perspective: perspective.clone(),
            counter_settings: self.counter_view.settings,
            selected: Arc::clone(self.unit_selection.units()),
            air_selected: Arc::clone(self.air_selection.wings()),
            air_bases: Arc::clone(self.air_selection.bases()),
            fleets_selected: Arc::clone(self.fleet_selection.fleets()),
            hovered: self.counter_view.hovered,
            visibility: self.counter_view.visibility,
            layered: self.counter_view.presentation.counters.lock().unwrap_or_else(|error| error.into_inner()).layered().identity(),
        };
        let (overlay_ms, additions) = {
            let _other_cost = replay_frame::cpu::scope("frame.overlay.prepare", Some("other"));
            let overlay_timer = Instant::now();
            // capture: the animation clock is the one input a live window cannot
            // share with a headless frame, so a capture frame renders at 0, the
            // still frame `--dump` renders, or at its explicit sequence clock. The
            // window runs one presentation clock: the map scene and the map-icon
            // windows' GUI clock (`Time`) share it.
            // A background flag or portrait load repaints the window, which then
            // lays the overlay out again with what it decoded.
            if !document.icons.lazy.has_wake() {
                let ctx = ui.ctx().clone();
                document.icons.lazy.set_wake(move || ctx.request_repaint());
            }
            let stale = self
                .prepared_overlay
                .as_ref()
                .is_some_and(|prepared| prepared.is_stale(&now));
            // The engaged index: the engagements layer's with the map open, today's pipeline's
            // without.
            let (engaged, layer_engaged) = if live {
                (None, self.engaged_history(&perspective.country).map_err(|error| error.to_string())?)
            } else {
                (self.hours.as_ref().map(|hours| hours.engaged(&perspective.country)).transpose().map_err(|error| error.to_string())?, None)
            };
            let covering = layer_engaged.as_deref().or(engaged.as_ref().map(|index| index.shared.as_ref()));
            // An overlay laid out before the engaged index reached its hour, by the
            // window or by an hour builder, is laid out again once it does, so a
            // paused seek or a prebuilt hour settles on the exact flag.
            let engaged_arrived = engaged_relayout(
                self.prepared_overlay
                    .as_ref()
                    .zip(self.overlay_at.as_ref())
                    .map(|(prepared, key)| (prepared.engaged_covered(), key.branch, key.hour)),
                |branch, hour| covering.is_some_and(|index| index.covers(branch, hour)),
            );
            // The raid icons: the scripts reader's at the head with the map open, the raid lane's
            // for the hour on screen without; a paused frame asks the lane for one batch.
            let ctx = ui.ctx().clone();
            // Its batches borrow the app's builder threads, as do the icon layer's owners
            // below. A view's counters ride in builder jobs, so they hold no handle to the pool.
            let pool = self.worker_pool().map_err(|why| format!("Native map workers: {why}"))?;
            let overlay_workers = pool.frame_workers();
            if let Some(scene) = self.scene.as_mut() {
                scene
                    .apply(
                        &state.device,
                        &state.queue,
                        SceneInput::UseWorkers(pool.frame_workers()),
                    )
                    .map_err(|error| error.to_string())?;
            }
            let at_head = live_of(&self.playhead_map).map(|side| Arc::clone(side.entry()));
            let mut raids_changed = false;
            if let Some(entry) = &at_head {
                // The lane builds nothing and holds nothing while the map is open.
                self.raid_lane.stand_by(true);
                let ask = crate::layers::scripts::Ask { entry, recording: self.rows_recording(document), sources: crate::layers::scripts::Sources::of(&document.content), perspective: &perspective, playing: self.playing, raids: self.graphics.counter(self.graphics.raid_icons), music: self.audio_focus };
                let wake = ctx.clone();
                self.scripts.show(ask, move || wake.request_repaint());
                // The tables made from rows at the head are the overlay's: put back where today's
                // install replaced them, and none of today's laid out meanwhile.
                let head_identity = self.head_tables.as_ref().map(|(_, tables)| tables.identity());
                if self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables.identity()) != head_identity {
                    self.overlay_tables = self.head_tables.as_ref().map(|(key, tables)| (key.branch, key.hour, key.perspective.clone(), tables.clone()));
                    self.overlay_at = None;
                    if self.overlay_tables.is_none() {
                        (self.prepared_overlay, self.overlay, self.overlay_fraction) = (None, None, None);
                        self.overlay_laid = None;
                        self.counter_report = CounterReport::default();
                    }
                }
                // What the tables' worker made last: the tables, and the overlay laid out from them
                // for the camera and view of the frame that asked, taken whole while this frame has
                // that camera and view; it shows the hour it was laid out for.
                let answer = self.overlay_tables_job.take();
                if answer.is_some() || !self.playing {
                    // The job is answered (or the head held): the overlay's reader makes its next
                    // rows at the next step that changes them, and none the job would not read.
                    self.head_rows.overlay_ready();
                }
                match answer {
                    Some((key, head_rows::tables::Outcome::Made((Ok(tables), laid)))) => {
                        // Routes made by key: a worker makes every route ahead, so a free frame
                        // before its whole tables arrive finds what it admits made.
                        if tables.counters().routes_by_key() {
                            let (fill, counters, movement) = (tables.clone(), Arc::clone(&document.content.counters), document.content.movement.clone());
                            pool.try_submit(Box::new(move || viewer_foundation::map::redraws::apart(|| fill.counters().fill_routes(&counters, movement.as_deref()))));
                        }
                        let retired = (
                            self.overlay_tables.replace((key.branch, key.hour, key.perspective.clone(), tables.clone())),
                            self.head_tables.replace((key, tables)),
                        );
                        self.retire(retired);
                        self.overlay_at = None;
                        if let Some(Laid { key, prepared, missed, on, rows, left, asked, lay_ins, times }) = laid {
                            times.note();
                            if !missed.is_empty() {
                                self.head_rows.learn(&missed);
                            }
                            let shown = missed.is_empty()
                                && key.same_view(&overlay_key)
                                && self.overlay_laying.as_ref().is_some_and(|(view, laying)| Arc::ptr_eq(view, &self.counter_view.presentation) && Arc::ptr_eq(laying, &on));
                            match prepared {
                                Ok(mut prepared) if shown => {
                                    if let Some(proof) = self.laid_proof.as_mut()
                                        && proof.due(key.hour)
                                        && let Some((_, _, _, tables)) = &self.overlay_tables
                                    {
                                        // The gate's own cost, apart from the frame's scopes.
                                        let _proof = replay_frame::cpu::scope("laid", Some("proof"));
                                        proof.check(&mut prepared, laid_proof::Check {
                                            content: &document.content,
                                            rosters: document.playback.air_rosters(),
                                            rows: &rows,
                                            left,
                                            lay_ins: &lay_ins,
                                            tables,
                                            key: &key,
                                            frame_hour: overlay_key.hour,
                                            asked: asked.as_ref(),
                                            view: &self.counter_view,
                                            layers: &self.overlay_layers,
                                            time: animation_time,
                                        });
                                    }
                                    self.counter_view.presentation.take_memory(&on);
                                    prepared.set_counter_view(self.counter_view.clone());
                                    self.install_overlay(prepared, &key);
                                    viewer_foundation::map::redraws::note("made.laid", 1);
                                    self.overlay_missed.clear();
                                    self.overlay_at = Some(key);
                                    self.overlay_fraction = None;
                                }
                                Err(error) if shown => {
                                    viewer_foundation::map::redraws::note("refused.full", 1);
                                    self.overlay_loading.push(format!("the overlay: {error}"));
                                    self.overlay_at = Some(key);
                                    self.overlay_fraction = None;
                                }
                                prepared => self.retire(prepared),
                            }
                        }
                    }
                    Some((_, head_rows::tables::Outcome::Made((Err(error), _)))) => self.overlay_loading.push(format!("the overlay's tables: {error}")),
                    Some((_, head_rows::tables::Outcome::Missed(missed))) => self.head_rows.learn(&missed),
                    None => {}
                }
                // A build of the tables from these rows, for the vision the vision layer shows, and
                // of the overlay from them for this frame's camera and view, on a worker, on a
                // memory of the view's own; in place for a frame a capture saves.
                if let (Some(rows), Some(vision)) = (&overlay_rows, &seen)
                    && !stale_hour
                    && !in_place
                {
                    let labels = live_of(&self.playhead_map).is_some_and(|side| side.draws("labels"));
                    let key = TablesKey {
                        branch: rows.branch,
                        hour: rows.hour,
                        rows: Arc::as_ptr(rows) as usize,
                        perspective: perspective.clone(),
                        vision: Arc::as_ptr(vision) as usize,
                        engaged: covering.is_some_and(|index| index.covers(rows.branch, rows.hour)),
                        left_out: self.counter_view.layered(),
                        labels,
                        need: overlay_need(&self.counter_view, overlay_camera.free().is_some()),
                    };
                    let real = &self.counter_view.presentation;
                    if !self.overlay_laying.as_ref().is_some_and(|(view, _)| Arc::ptr_eq(view, real)) {
                        self.overlay_laying = Some((Arc::clone(real), Arc::new(real.sharing())));
                    }
                    let on = Arc::clone(&self.overlay_laying.as_ref().expect("made above").1);
                    // The worker's memory is its own only while no build is in flight.
                    if !self.overlay_tables_job.running() {
                        real.lend_layered(&on);
                    }
                    let asked = self.laid_proof.is_some().then(|| {
                        let asked = Arc::new(real.snapshot());
                        real.lend_layered(&asked);
                        asked
                    });
                    let view = CounterView { presentation: on, ..self.counter_view.clone() };
                    let (laid_key, camera) = (overlay_key.clone(), overlay_camera);
                    let lay_ins = Arc::new(LayIns::of(live_of(&self.playhead_map)));
                    let asked_at = Instant::now();
                    let (content, rows, choice, vision, engaged, rosters) = (Arc::clone(&document.content), Arc::clone(rows), perspective.clone(), Arc::clone(vision), layer_engaged.clone(), document.playback.air_rosters());
                    let raids = self.scripts.raids(rows.branch, vision.ours()).unwrap_or_default();
                    let (left_out, need) = (key.left_out, key.need);
                    let make = move || viewer_foundation::map::redraws::apart(|| {
                        let started = Instant::now();
                        let cpu_started = viewer_foundation::profiling::current_thread_cpu_time();
                        let hour = crate::document::Hour::new(&content, &rows, 0.0).with_air_rosters(Some(rosters));
                        let mut spans = viewer_foundation::profiling::BuilderSpans::new(replay_frame::cpu::enabled());
                        let mut tables = hour.hour_tables_at_head(&choice, vision, raids, engaged.as_deref(), left_out, labels, need, &mut spans);
                        if let Ok(tables) = &mut tables {
                            lay_ins.set(tables);
                        }
                        let tabled = Instant::now();
                        let tables_cpu = cpu_started.zip(viewer_foundation::profiling::current_thread_cpu_time()).and_then(|(before, after)| after.checked_sub(before));
                        // The memory lays out exactly the products the key names, or the build lays
                        // nothing out and the frame lays the hour out itself.
                        let lent = view.presentation.counters.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).layered().identity();
                        let laid = tables.as_ref().ok().filter(|_| lent == laid_key.layered).map(|tables| {
                            let read = crate::document::OverlayRows::at_head(Some(Arc::clone(&rows))).leaving(left);
                            // In turn on this build's thread: the builders' other thread is the frame's
                            // and the other workers'.
                            let prepared = hour.prepare_overlay_at(camera, pixels, ui_scale, tables, &view, &Default::default(), &read).map(|mut prepared| {
                                lay_ins.lay(&mut prepared, &hour, camera, pixels, ui_scale, tables.ours());
                                prepared
                            });
                            let times = LaidTimes { asked: asked_at, queued: started - asked_at, tables: tabled - started, layout: tabled.elapsed(), tables_cpu, parts: spans.into_records() };
                            Laid { key: laid_key, prepared, missed: read.missed(), on: Arc::clone(&view.presentation), rows: Arc::clone(&rows), left, asked, lay_ins: Arc::clone(&lay_ins), times }
                        });
                        (tables, laid)
                    });
                    let submit = |work: Box<dyn FnOnce() + Send>| pool.try_submit(work);
                    let wake = ctx.clone();
                    self.overlay_tables_job.ask(&key, make, &submit, move || wake.request_repaint());
                }
                if let Some((branch, _, _, tables)) = &mut self.overlay_tables
                    && let Some(icons) = self.scripts.raids(*branch, tables.ours())
                    && !Arc::ptr_eq(&icons, &tables.raids())
                {
                    raids_changed = tables.set_raids_at_head(icons);
                }
            } else {
                self.raid_lane.use_workers(pool.workers());
                self.raid_lane.stand_by(false);
                self.raid_lane.show(
                    &document.content,
                    &perspective,
                    (document.playback.branch(), document.playback.hour()),
                    self.playing,
                    move || ctx.request_repaint(),
                );
                // Only icons of the lane's current reset, built for the hour and the resolved
                // viewing country the tables are for, install.
                if let Some((branch, hour, _, tables)) = &mut self.overlay_tables
                    && let Some(icons) = self.raid_lane.icons(*branch, *hour, tables.ours())
                    && !Arc::ptr_eq(&icons, &tables.raids())
                {
                    raids_changed = tables.set_raids(icons);
                }
            }
            // A part left for a layer that no longer draws it is made by today's again this frame.
            if self.prepared_overlay.as_ref().is_some_and(|prepared| left.unmet_by(prepared.resource_icons_by(), prepared.occupation_icons_by())) {
                self.overlay_at = None;
            }
            let layout_changed =
                stale || engaged_arrived || self.overlay_at.as_ref() != Some(&overlay_key);
            // Counted for what changed since the frame before, not for an overlay still behind it.
            if layout_changed && self.overlay_last_asked.as_ref() != Some(&overlay_key) {
                use viewer_foundation::map::redraws::note;
                for asked in overlay_asks(self.overlay_at.as_ref(), &overlay_key) {
                    note(asked, 1);
                }
                note("asked.art", u64::from(stale));
                note("asked.engaged", u64::from(engaged_arrived));
            }
            if self.overlay_last_asked.as_ref() != Some(&overlay_key) {
                self.overlay_last_asked = Some(overlay_key.clone());
            }
            // Only the counter layers' products changed: the counters alone are laid out again.
            let land_only = layout_changed
                && !stale
                && !engaged_arrived
                && self.overlay_at.as_ref().is_some_and(|at| at.same_but_layered(&overlay_key));
            if land_only
                && let (Some(prepared), Some((_, _, _, tables))) =
                    (self.prepared_overlay.as_mut(), self.overlay_tables.as_ref())
            {
                let rows = crate::document::OverlayRows::at_head(overlay_rows.clone().filter(|_| live));
                let icons = now.prepare_counters(overlay_camera, pixels, ui_scale, tables, &self.counter_view, &rows);
                let missed = rows.missed();
                if missed.is_empty() {
                    match icons {
                        Ok(icons) => {
                            viewer_foundation::map::redraws::note("made.counters", 1);
                            prepared.set_counters(icons);
                        }
                        Err(error) if live => self.overlay_loading.push(format!("the overlay's counters: {error}")),
                        Err(error) => return Err(error.to_string()),
                    }
                    self.overlay_fraction = None;
                } else {
                    self.head_rows.learn(&missed);
                }
                self.overlay_missed = missed;
                self.overlay_at = Some(overlay_key.clone());
            }
            // An hour the tables' worker lays out for this camera and view is shown once laid: the
            // frame thread lays out no whole hour while the worker does.
            let laying = live
                && !in_place
                && !stale
                && !engaged_arrived
                && self.overlay_tables_job.running()
                && self.overlay_at.as_ref().is_some_and(|at| at.same_view(&overlay_key));
            if layout_changed && !land_only && laying {
                viewer_foundation::map::redraws::note("waiting.laid", 1);
            }
            let layout_changed = layout_changed && !land_only && !laying;
            // A raid batch on the laid-out hour, camera and view replaces the raid
            // icons alone: the rest of the overlay does not read them. The batch
            // drawn at this fraction takes them in place, as drawing it again
            // would; a new fraction draws it again below.
            let raids_only = raids_changed && !layout_changed;
            if raids_only
                && let (Some(prepared), Some((_, _, _, tables))) =
                    (self.prepared_overlay.as_mut(), self.overlay_tables.as_ref())
            {
                self.overlay_written += 1;
                let drawn = (self.overlay_fraction == Some(document.hour_fraction))
                    .then_some(self.overlay.as_mut())
                    .flatten();
                prepared.replace_raids(
                    &now,
                    overlay_camera,
                    pixels,
                    ui_scale,
                    &tables.raids(),
                    drawn,
                );
            }
            if layout_changed || (raids_only && self.prepared_overlay.is_none()) {
                if profiling() {
                    use viewer_foundation::profiling::record_count;
                    // Independent reason bits: one preparation can have several.
                    record_count("overlay_reprepare", 1);
                    record_count(
                        "overlay_reason_key",
                        u64::from(self.overlay_at.as_ref() != Some(&overlay_key)),
                    );
                    record_count("overlay_reason_artwork", u64::from(stale));
                    record_count("overlay_reason_engagement", u64::from(engaged_arrived));
                    record_count(
                        "overlay_reason_raid_missing",
                        u64::from(raids_only && self.prepared_overlay.is_none()),
                    );
                }
                if live && let Some(laid) = settled_overlay.take() {
                    // Settled within this saved frame over the rows at the hour on screen.
                    self.install_overlay(laid.map_err(|error| error.to_string())?, &overlay_key);
                    viewer_foundation::map::redraws::note("made.full", 1);
                    self.overlay_missed.clear();
                    self.overlay_at = Some(overlay_key);
                    self.overlay_fraction = None;
                } else if live {
                    // Laid out over the rows at the head with the tables made from them; a read
                    // they do not hold yet keeps the overlay laid out last and is learned.
                    if let Some((_, _, _, tables)) = &self.overlay_tables {
                        let rows = crate::document::OverlayRows::at_head(overlay_rows.clone()).leaving(left);
                        let made = now.prepare_overlay_at(overlay_camera, pixels, ui_scale, tables, &self.counter_view, &overlay_workers, &rows);
                        let missed = rows.missed();
                        if missed.is_empty() {
                            match made {
                                Ok(prepared) => {
                                    viewer_foundation::map::redraws::note("made.full", 1);
                                    self.install_overlay(prepared, &overlay_key);
                                }
                                Err(error) => {
                                    viewer_foundation::map::redraws::note("refused.full", 1);
                                    self.overlay_loading.push(format!("the overlay: {error}"));
                                }
                            }
                            self.overlay_fraction = None;
                        } else {
                            viewer_foundation::map::redraws::note("missed.full", 1);
                            self.head_rows.learn(&missed);
                        }
                        self.overlay_missed = missed;
                        self.overlay_at = Some(overlay_key);
                    } else {
                        // Asked with no tables made from the rows at the head yet: named, the
                        // tables the last build missed and those its rows still wait for.
                        viewer_foundation::map::redraws::note("waiting.tables", 1);
                        let mut waiting = self.head_rows.overlay_arriving();
                        waiting.extend_from_slice(self.overlay_tables_job.loading());
                        head_rows::note_waiting(waiting);
                    }
                } else {
                    let tables_key = (
                        document.playback.branch(),
                        document.playback.hour(),
                        perspective.clone(),
                    );
                    let tables_mismatch =
                        self.overlay_tables
                            .as_ref()
                            .is_none_or(|(branch, hour, perspective, _)| {
                                (*branch, *hour, perspective)
                                    != (tables_key.0, tables_key.1, &tables_key.2)
                            });
                    let refine_arrival = engaged_arrived && !tables_mismatch;
                    let mut retired_tables = None;
                    if !stale_hour && (engaged_arrived || tables_mismatch) {
                        let tables = {
                            // Raid offers are only evaluated on the raid lane: until it
                            // delivers this hour's for this view, no raid icon draws
                            // (`RaidIcons::carried`).
                            let ours = now
                                .vision(&perspective)
                                .map_err(|error| error.to_string())?
                                .ours();
                            let raids = self
                                .raid_lane
                                .icons(tables_key.0, tables_key.1, ours)
                                .unwrap_or_default();
                            if refine_arrival {
                                now.refine_overlay_tables_with_raids(
                                    &self.overlay_tables.as_ref().expect("matching base").3,
                                    &perspective,
                                    raids,
                                    engaged.as_deref(),
                                    self.counter_view.layered(),
                                )
                            } else {
                                now.hour_tables_layered(
                                    &perspective,
                                    raids,
                                    engaged.as_deref(),
                                    self.counter_view.layered(),
                                    false,
                                    &mut viewer_foundation::profiling::BuilderSpans::new(false),
                                    true,
                                )
                            }
                            .map_err(|error| error.to_string())?
                        };
                        let replaced = self.overlay_tables.replace((
                            tables_key.0,
                            tables_key.1,
                            tables_key.2,
                            tables,
                        ));
                        if refine_arrival {
                            retired_tables = replaced;
                        } else {
                            drop(replaced);
                        }
                    }
                    let tables = &self.overlay_tables.as_ref().expect("parsed above").3;
                    let rows = crate::document::OverlayRows::default().leaving(left);
                    let prepared = now
                        .prepare_overlay_at(
                            overlay_camera,
                            pixels,
                            ui_scale,
                            tables,
                            &self.counter_view,
                            &overlay_workers,
                            &rows,
                        )
                        .map_err(|error| error.to_string())?;
                    let retired_prepared = self.prepared_overlay.replace(prepared);
                    viewer_foundation::map::redraws::note("made.full", 1);
                    if refine_arrival && !stale_hour {
                        // One bounded retirement for this replacement, never a wait.
                        // The existing Dropper frees inline when its queue is full.
                        if let Some(dropper) = &self.dropper {
                            dropper.drop_later((retired_tables, retired_prepared));
                        }
                    } else {
                        drop(retired_prepared);
                    }
                    self.overlay_at = Some(overlay_key);
                    self.overlay_fraction = None;
                }
            }
            // The labels the map's layer holds at the head replace the hour's, laid out again alone.
            if let Some(side) = live_of(&self.playhead_map)
                && let Some((labels, ours)) = crate::layers::labels::drawn(side)
                && let (Some(prepared), Some((_, _, _, tables))) = (self.prepared_overlay.as_mut(), self.overlay_tables.as_mut())
                && tables.set_labels(&labels, ours)
            {
                prepared.replace_labels(&now, overlay_camera, pixels, ui_scale, &labels).map_err(|error| error.to_string())?;
                self.overlay_fraction = None;
            }
            // The country names the map's layer placed at the head replace the hour's.
            if let Some(side) = live_of(&self.playhead_map)
                && let Some(names) = crate::layers::names::drawn(side)
                && !self.placements.as_ref().is_some_and(|held| Arc::ptr_eq(held, &names))
            {
                viewer_foundation::view_record::add("names window", "taken_from_layer", 1);
                drop(self.placements.replace(names));
            }
            if viewer_foundation::view_record::on() {
                let draws = live_of(&self.playhead_map).is_some_and(|side| side.draws("names"));
                viewer_foundation::view_record::set("names window", "layer_draws", u64::from(draws));
                viewer_foundation::view_record::set("names window", "generation", self.placements.as_ref().map_or(0, |held| held.generation));
                viewer_foundation::view_record::set("names window", "names", self.placements.as_ref().map_or(0, |held| held.country.len() as u64));
            }
            // The Operatives icons the map's layer evaluated at the head replace the hour's.
            if let Some(side) = live_of(&self.playhead_map)
                && let Some(icons) = crate::layers::intel_icons::drawn(side)
                && let (Some(prepared), Some((_, _, _, tables))) = (self.prepared_overlay.as_mut(), self.overlay_tables.as_mut())
                && tables.set_intel(&icons)
            {
                let started = Instant::now();
                prepared.replace_intel(&now, overlay_camera, pixels, ui_scale, &icons);
                crate::layers::intel_icons::installed(side, started.elapsed());
                self.overlay_fraction = None;
            }
            // The resource icons and occupation marks the map's layers hold at the head replace the hour's.
            if let (Some(prepared), Some((_, _, _, tables))) = (self.prepared_overlay.as_mut(), self.overlay_tables.as_ref())
                && lay_in_layers(live_of(&self.playhead_map), prepared, &now, overlay_camera, pixels, ui_scale, tables.ours())
            {
                self.overlay_fraction = None;
            }
            overlay_prepare_cpu.finish();
            laps.lap("overlay_prepare");
            drop(_other_cost);
            let _other_cost = replay_frame::cpu::scope("frame.overlay.draw", Some("other"));
            let overlay_draw_cpu = viewer_foundation::profiling::thread_span("overlay_draw");
            let owners_span = viewer_foundation::profiling::span("overlay_owners");
            // Display choices only change what the next overlay draw keeps.
            let ours = self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables.ours()).or_else(|| seen.as_ref().map(|vision| vision.ours()));
            let head = head_rows::head(live_of(&self.playhead_map), document);
            let filtered = self.counter_owners.get(frame, head, &self.counter_filter);
            let layers = crate::document::OverlayLayers::shown(
                &self.graphics,
                match ours {
                    Some(ours) => self.shown_counter_owners(filtered, ours),
                    None => filtered,
                },
            );
            if layers != self.overlay_layers {
                self.overlay_layers = layers;
                self.overlay_fraction = None;
            }
            drop(owners_span);
            let scene_span = viewer_foundation::profiling::span("overlay_scene_inputs");
            let (_, movement_hour) = self.hour_on_screen(document);
            // The frame's province schedule, advanced before its map icons read the dirty bits
            // (province updates precede map-icon updates); the scene's
            // render takes it.
            if let Some(scene) = self.scene.as_mut() {
                scene
                    .apply(
                        &state.device,
                        &state.queue,
                        SceneInput::ShareProvinceSchedule(
                            self.counter_view.presentation.schedule(),
                        ),
                    )
                    .map_err(|error| error.to_string())?;
                let head = head_rows::head(live_of(&self.playhead_map), document);
                let selected = self.unit_selection.units().as_ref();
                if let Some(ours) = ours {
                    let mut arrows = |frame: &replay_frame::frame::DecodedFrame| {
                        crate::document::select_movement(scene, &state.device, &state.queue, &document.content, frame, ours, selected)
                    };
                    // No selection reads no row; a selection's arrows are kept while its rows load.
                    let moved = if selected.is_empty() { Some(arrows(now.frame)) } else { self.head_rows.arrows(head, frame, arrows) };
                    if let Some(Err(error)) = moved {
                        self.overlay_loading.push(format!("the selection's movement arrows: {error}"));
                    }
                }
                let clock_speed = self.head_rows.map(head, frame, viewer_foundation::map::counter_move::MovementFrame::clock_speed).flatten();
                scene
                    .apply(
                        &state.device,
                        &state.queue,
                        SceneInput::AdvanceProvinces {
                            camera,
                            size: pixels,
                        },
                    )
                    .map_err(|error| error.to_string())?;
                // The unit animators advance with this frame's province updates, before the
                // figures and map icons read them.
                let presentation = &self.counter_view.presentation;
                {
                    let schedule = presentation
                        .schedule()
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    presentation
                        .movement()
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .begin_frame(
                            viewer_foundation::map::counter_move::MovementFrame {
                                time: animation_time,
                                hour: movement_hour,
                                clock_speed,
                                speeds: document.content.counters.defines.movement_speeds,
                                steady: self.options.window_dump,
                            },
                            schedule.updates().filter_map(|(province, update)| {
                                (update == viewer_foundation::map::province_schedule::Update::InView)
                                    .then_some(province)
                            }),
                        );
                }
                // With the map open the land figures move from its units layer's own placings at
                // the applied hour, the formations from its models; nothing of today's.
                let positions: Option<Box<dyn Iterator<Item = (viewer_foundation::unit_handle::Unit, &viewer_foundation::map::counter_move::Position, f32, f32)> + '_>> = match live_of(&self.playhead_map) {
                    Some(side) => crate::layers::units::of(side).map(|units| Box::new(units.movement_positions()) as _),
                    None => self.unit_models.as_ref().map(|prepared| Box::new(prepared.movement_positions()) as _),
                };
                if let Some(positions) = positions {
                    scene
                        .apply(
                            &state.device,
                            &state.queue,
                            SceneInput::SetUnitMovement {
                                positions,
                                history: presentation.movement(),
                            },
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            drop(scene_span);
            if self.overlay_fraction != Some(document.hour_fraction)
                && let Some(prepared) = self.prepared_overlay.as_mut()
            {
                // Asked for by running pools alone: only their passes run.
                let quiet = self.overlay_fraction.is_some_and(f32::is_nan)
                    && self.overlay_drawn_fraction == Some(document.hour_fraction);
                if quiet {
                    viewer_foundation::map::redraws::note("made.quiet", 1);
                    prepared
                        .draw_quiet(&now, overlay_camera, pixels, ui_scale, &self.overlay_layers, animation_time)
                        .map_err(|error| error.to_string())?;
                } else {
                    viewer_foundation::map::redraws::note("made.draw", 1);
                    self.overlay_written += 1;
                    prepared.set_pressed(self.counter_view.pressed, self.counter_view.plate_hovered);
                    loop {
                        prepared
                            .draw_into(
                                &now,
                                overlay_camera,
                                pixels,
                                ui_scale,
                                &self.overlay_layers,
                                animation_time,
                                &overlay_workers,
                                self.overlay.get_or_insert_with(Batch::default),
                                &mut self.counter_report,
                            )
                            .map_err(|error| error.to_string())?;
                        if self.rendered.is_some() { break; }
                        document.icons.lazy.settle();
                        if !prepared.is_stale(&now) && !document.icons.lazy.has_pending() { break; }
                        *prepared = now.prepare_overlay_cached(
                            overlay_camera, pixels, ui_scale,
                            &self.overlay_tables.as_ref().expect("parsed above").3,
                            &self.counter_view, &overlay_workers,
                        ).map_err(|error| error.to_string())?;
                    }
                    if self.label_report != *prepared.label_report() {
                        self.label_report.clone_from(prepared.label_report());
                    }
                }
                self.overlay_fraction = Some(document.hour_fraction);
                self.overlay_drawn_fraction = Some(document.hour_fraction);
            }
            // Native windows keep ticking and drawing while active. Their clocks do not
            // change the hour/camera preparation, including the settled counter positions.
            if let Some((_, tables)) = &self.head_tables {
                let (identity, (made, kept)) = (tables.identity(), tables.counters().lookups());
                let (before_made, before_kept) = match self.overlay_lookups {
                    Some((held, made, kept)) if held == identity => (made, kept),
                    _ => (0, 0),
                };
                viewer_foundation::map::redraws::note("lookups.made", made.saturating_sub(before_made));
                viewer_foundation::map::redraws::note("lookups.kept", kept.saturating_sub(before_kept));
                viewer_foundation::view_record::add("overlay", "lookups_made", made.saturating_sub(before_made));
                viewer_foundation::view_record::add("overlay", "lookups_kept", kept.saturating_sub(before_kept));
                self.overlay_lookups = Some((identity, made, kept));
            }
            // The overlay installed against the map's drawn picture: the hours it stands apart (after
            // a seek, the frames past the map's own compose show here).
            if let (Some(shown), Some((branch, drawn))) = (&self.overlay_shown, self.layers_on_screen())
                && (shown.branch != branch || shown.hour != drawn)
            {
                viewer_foundation::map::redraws::note("overlay.hour_apart", 1);
                viewer_foundation::view_record::add("overlay", "hour_apart", 1);
                viewer_foundation::view_record::set("overlay", "hours_apart", u64::from(shown.hour.abs_diff(drawn)));
            }
            let running = self.counter_view.presentation.take_running();
            if running.any {
                viewer_foundation::map::redraws::note("asked.running", 1);
                // Nothing shown changes: the next frame runs the pools' passes alone, unless
                // something else asks it to draw (every other ask sets `None`).
                self.overlay_fraction = (!running.visible).then_some(f32::NAN);
                ui.ctx().request_repaint();
            }
            // A free eye shows the laid-out icons standing in the world; this frame's hits and
            // drawing follow what it shows. Placed again only when what the placing reads changed.
            let presented = (overlay_camera, pixels, ui_scale.to_bits(), self.overlay_written);
            self.looks = if interface
                && overlay_camera.free().is_some()
                && let Some(sprites) = self.overlay.as_ref()
            {
                if self.free_presented == Some(presented) && self.looks.is_some() {
                    viewer_foundation::map::redraws::note("free.placed_kept", 1);
                    self.looks.take()
                } else {
                    let _span = viewer_foundation::profiling::span("overlay_free_present");
                    viewer_foundation::map::redraws::note("free.placed", 1);
                    let ground = free_icons::Ground {
                        field: document.assets.height_field(),
                        sight: document.assets.terrain_sight(),
                    };
                    self.free_presented = Some(presented);
                    Some(free_icons::present(
                        &overlay_camera,
                        pixels,
                        ui_scale,
                        Some(&ground),
                        sprites,
                        &mut self.free_overlay,
                    ))
                }
            } else {
                self.free_presented = None;
                None
            };
            if raids_only && check_overlay() {
                self.check_overlay(&now, overlay_camera, pixels, ui_scale, animation_time)?;
            }
            // An explicit fleet choice remains separate from a task-force plate
            // click. Its roster also reaches empty fleets with no map plate.
            if *self.shown_mode()
                == viewer_world::map::map_mode::MapMode::Stock(
                    viewer_world::map::map_mode::Stock::StrategicNavy,
                )
            {
                let head = head_rows::head(live_of(&self.playhead_map), document);
                response.context_menu(|ui| {
                    let choices = match ours {
                        Some(ours) => self.head_rows.clicks(head, frame, |frame| viewer_counters::map::naval_selection::fleet_choices(frame, ours)).unwrap_or_else(|| Err(std::io::Error::other("the fleets at the head are loading"))),
                        None => Err(std::io::Error::other("the viewing country is loading")),
                    };
                    match choices {
                        Err(why) => {
                            ui.label(why.to_string());
                        }
                        Ok(choices) => {
                            for (fleet, name) in choices {
                                if ui
                                    .selectable_label(
                                        self.fleet_selection.fleets().contains(&fleet),
                                        name,
                                    )
                                    .clicked()
                                {
                                    let shift = ui.input(|input| input.modifiers.shift);
                                    self.fleet_selection.click(fleet, shift);
                                    if !shift {
                                        self.unit_selection.clear();
                                        self.air_selection.clear();
                                    }
                                    self.counter_view.selected =
                                        Arc::clone(self.unit_selection.units());
                                    self.counter_view.fleets_selected =
                                        Arc::clone(self.fleet_selection.fleets());
                                    self.overlay_at = None;
                                    ui.close();
                                    ui.ctx().request_repaint();
                                }
                            }
                        }
                    }
                });
            }
            let overlay_ms = overlay_timer.elapsed().as_secs_f64() * 1000.0;
            overlay_draw_cpu.finish();
            laps.lap("overlay_draw");
            drop(_other_cost);
            let _other_cost = replay_frame::cpu::scope("frame.interface", Some("other"));
            let engaged = self
                .engaged_history(&perspective.country)
                .map_err(|error| error.to_string())?;
            laps.lap("engaged");
            self.combat.set_playing(self.playing);
            let wake = ui.ctx().clone();
            self.combat
                .prepare_air(
                    &document.playback.replay_handle(),
                    move || wake.request_repaint(),
                );
            let prepared = self
                .combat_prepared(document, &perspective, engaged, ui.ctx())
                .map_err(|error| error.to_string())?;
            let drawn = self
                .combat
                .draw(
                    &document.combat,
                    prepared,
                    extent,
                    true,
                    animation_time,
                    true,
                    combat_input
                        .as_ref()
                        .map(|input| input.filter(|at| picker_at(at).is_none())),
                )
                .map_err(|error| error.to_string())?;
            if self.combat.gui_active() {
                ui.ctx().request_repaint();
            }
            if let Some(drawn) = &drawn {
                self.interface_sounds.extend(
                    drawn
                        .sounds
                        .iter()
                        .map(|played| InterfaceSound::new("combat", played)),
                );
            }
            laps.lap("combat_draw");
            let picker_drawn = match self.draw_picker(
                document,
                extent,
                true,
                animation_time,
                true,
                picker_input,
            ) {
                Some(Ok(drawn)) => Some(drawn),
                Some(Err(error)) => {
                    self.picker_error = Some(error);
                    None
                }
                None => None,
            };
            if self.picker.picker.pointing() {
                ui.ctx().request_repaint();
            }
            if let Some(picker_drawn) = &picker_drawn {
                self.interface_sounds.extend(
                    picker_drawn
                        .drawn
                        .sounds
                        .iter()
                        .map(|played| InterfaceSound::new("picker", played)),
                );
                self.picker_error = None;
                self.picker_refused.clone_from(&picker_drawn.drawn.refused);
            }
            match (&picker_drawn, &mut self.picker_controls) {
                (Some(drawn), Some((drawn_at, controls))) => {
                    *drawn_at = extent;
                    controls.clone_from(&drawn.controls);
                }
                (Some(drawn), held) => *held = Some((extent, drawn.controls.clone())),
                (None, held) => *held = None,
            }
            let over_picker = point.is_some_and(|point| {
                picker_drawn
                    .as_ref()
                    .is_some_and(|drawn| drawn.control_at(point).is_some())
            });
            // Clicks act in the order their button changes came, across both windows; a click
            // off the picker closes its open list there, and later list clicks find it closed.
            let mut clicks: Vec<(usize, Act)> = picker_drawn
                .iter()
                .flat_map(|drawn| drawn.clicked.iter())
                .map(|(control, order)| (*order, Act::Picker(control.clone(), false)))
                .collect();
            for (order, placed) in secondary_clicks {
                if let Some((path, effect)) = &placed.click {
                    let played = viewer_foundation::map::native_gui::pointer::Played {
                        effect: effect.clone(),
                        click: Some(path.clone()),
                        order,
                    };
                    self.interface_sounds
                        .push(InterfaceSound::new("picker", &played));
                }
                clicks.push((order, Act::Picker(placed.control, true)));
            }
            if let Some(window) = &drawn {
                clicks.extend(
                    window
                        .clicks
                        .iter()
                        .enumerate()
                        .map(|(at, (_, _, order))| (*order, Act::Combat(at))),
                );
            }
            clicks.extend(
                off_picker_clicks
                    .iter()
                    .map(|&order| (order, Act::Dismiss)),
            );
            // Resolve map clicks against this hour and camera's newly rebuilt hit regions, in order
            // with the windows'; the click that chose a free mode's start selects nothing. A counter
            // takes its button's clicks on the counters their presses went down on.
            if !started {
                clicks.extend(
                    counter_clicks
                        .iter()
                        .map(|&(unit, order)| (order, Act::Counter(unit))),
                );
                if !self.combat.dragging && (!free_mode || icon_click) {
                    clicks.extend(
                        bare_clicks
                            .iter()
                            .map(|&(point, order)| (order, Act::Map(point))),
                    );
                }
            }
            clicks.sort_by_key(|(order, _)| *order);
            let mut closed = false;
            for (_, click) in clicks {
                use viewer_combat::picker::Control;
                match click {
                    Act::Dismiss => {
                        if self.picker.open != viewer_combat::picker::Open::None {
                            self.picker.open = viewer_combat::picker::Open::None;
                            closed = true;
                        }
                    }
                    Act::Counter(unit) => {
                        let hit = self
                            .counter_report
                            .unit_hits
                            .iter()
                            .position(|hit| hit.units().next() == Some(unit));
                        if hit.is_some() {
                            self.click_units(hit, ui.ctx());
                        }
                    }
                    Act::Map(point) => {
                        if matches!(self.shown_mode(), viewer_world::map::map_mode::MapMode::Stock(viewer_world::map::map_mode::Stock::Diplomacy))
                            && self.map_hit(point).is_none()
                            && let Some(province) = pick(
                                &camera,
                                point.map(|value| value * ui_scale),
                                pixels,
                                &document.map,
                                document.assets.heightmap_texels(),
                            ).map_err(|error| error.to_string())?
                            && let Some(country) = self.counter_report.perspective.province_controller(province)
                            && self.diplomatic_selected != Some(country)
                        {
                            self.diplomatic_selected = Some(country);
                            ui.ctx().request_repaint();
                        }
                        match self.map_hit(point) {
                            Some(MapHit::NavalMission(index))
                                if self.counter_report.naval_mission_hits[index].mission != 0 =>
                            {
                                let hit = &self.counter_report.naval_mission_hits[index];
                                let head = head_rows::head(live_of(&self.playhead_map), document);
                                // A click while the viewing country or the rows load selects nothing.
                                let read = ours.and_then(|viewer| self.head_rows.clicks(head, frame, |frame| viewer_counters::map::naval_selection::mission_members(frame, viewer, hit.region, hit.mission)));
                                let Some(members) = read_click(live, &mut self.overlay_loading, "the naval mission's members", read)? else {
                                    continue;
                                };
                                let selection = viewer_foundation::map::counter_hovers::unit_hits::Hit {
                                    plate: None,
                                    ours: true,
                                    port: false,
                                    rect: hit.rect,
                                    layer: hit.layer,
                                    kind:
                                        viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce,
                                    members: members.into(),
                                };
                                self.unit_selection.click(Some(&selection), false);
                                self.air_selection.clear();
                                self.fleet_selection.clear();
                                self.counter_view.selected = Arc::clone(self.unit_selection.units());
                                self.overlay_at = None;
                                ui.ctx().request_repaint();
                            }
                            Some(MapHit::Base(index))
                                if self.counter_report.air_grid_hits.contains_key(&index) =>
                            {
                                if let Some(members) =
                                    self.counter_report.air_grid_hits[&index].members.clone()
                                {
                                    let shift = ui.input(|input| input.modifiers.shift);
                                    let head = head_rows::head(live_of(&self.playhead_map), document);
                                    let read = self.head_rows.clicks(head, frame, |frame| self.air_selection.grid_click(frame, &members, shift));
                                    if read_click(live, &mut self.overlay_loading, "the air grid's selection", read)?.unwrap_or(false) {
                                        self.unit_selection.clear();
                                        self.counter_view.selected =
                                            Arc::clone(self.unit_selection.units());
                                        self.overlay_at = None;
                                        ui.ctx().request_repaint();
                                    }
                                }
                            }
                            Some(MapHit::Base(index))
                                if self.counter_report.air_base_hits.contains_key(&index) =>
                            {
                                let (province, kind) = self.counter_report.air_base_hits[&index];
                                let shift = ui.input(|input| input.modifiers.shift);
                                let head = head_rows::head(live_of(&self.playhead_map), document);
                                let read = ours.and_then(|viewer| self.head_rows.clicks(head, frame, |frame| viewer_counters::map::air_selection::base_target(frame, viewer, province, kind)));
                                let target = read_click(live, &mut self.overlay_loading, "the air base's target", read)?.flatten();
                                if let Some(target) = target {
                                    self.choose_map_mode(viewer_world::map::map_mode::MapMode::Stock(
                                        viewer_world::map::map_mode::Stock::StrategicAir,
                                    ));
                                    let head = head_rows::head(live_of(&self.playhead_map), document);
                                    let read = self.head_rows.clicks(head, frame, |frame| self.air_selection.base_click(frame, target.clone(), shift));
                                    read_click(live, &mut self.overlay_loading, "the air base's selection", read)?;
                                    self.unit_selection.clear();
                                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
                                    self.overlay_at = None;
                                    ui.ctx().request_repaint();
                                }
                            }
                            Some(MapHit::AirMission(index))
                                if self.counter_report.air_mission_hits[index]
                                    .mission
                                    .is_some() =>
                            {
                                let hit = &self.counter_report.air_mission_hits[index];
                                let head = head_rows::head(live_of(&self.playhead_map), document);
                                let read = ours.and_then(|viewer| self.head_rows.clicks(head, frame, |frame| viewer_counters::map::air_selection::mission_members(frame, viewer, hit.region, hit.mission.unwrap())));
                                let Some(members) = read_click(live, &mut self.overlay_loading, "the air mission's members", read)? else {
                                    continue;
                                };
                                let shift = ui.input(|input| input.modifiers.shift);
                                let read = self.head_rows.clicks(head, frame, |frame| self.air_selection.mission_click(frame, &members, shift));
                                if read_click(live, &mut self.overlay_loading, "the air mission's selection", read)?.unwrap_or(false) {
                                    self.unit_selection.clear();
                                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
                                    self.overlay_at = None;
                                    ui.ctx().request_repaint();
                                }
                            }
                            Some(MapHit::AirMission(index)) => {
                                self.combat
                                    .open(viewer_foundation::combat_selection::Selection::AirRegion(
                                        self.counter_report.air_mission_hits[index].region,
                                    ))
                            }
                            Some(MapHit::Combat(index)) => self
                                .combat
                                .open(self.counter_report.combat_hits[index].selection),
                            None if matches!(
                                self.shown_mode(),
                                viewer_world::map::map_mode::MapMode::Stock(
                                    viewer_world::map::map_mode::Stock::StrategicAir
                                )
                            ) =>
                            {
                                if let Some(region) = pick(
                                    &camera,
                                    point.map(|v| v * ui_scale),
                                    pixels,
                                    &document.map,
                                    document.assets.heightmap_texels(),
                                )
                                .map_err(|e| e.to_string())?
                                .and_then(|province| document.content.counters.region(province))
                                {
                                    self.combat.open(
                                        viewer_foundation::combat_selection::Selection::AirRegion(region),
                                    );
                                }
                            }
                            None => self.click_units(None, ui.ctx()),
                            _ => {}
                        }
                    }
                    Act::Picker(Control::Entry(_) | Control::SlotEntry(_) | Control::Default, _)
                        if closed => {}
                    Act::Picker(control, secondary) => {
                        self.picker_click(document, &control, secondary);
                    }
                    Act::Combat(at) => {
                        if let Some(window) = &drawn {
                            let (path, rect, _) = &window.clicks[at];
                            self.combat.click(window, path, *rect);
                        }
                    }
                }
            }
            // The frame's interface sounds play as their changes came.
            self.interface_sounds[frame_sounds..].sort_by_key(|sound| sound.order);
            let mut drawn = drawn;
            if let Some(picker_drawn) = &picker_drawn {
                drawn
                    .get_or_insert_with(viewer_combat::Drawn::default)
                    .add_hovers(&picker_drawn.drawn);
            }
            laps.lap("picker");
            let tooltip_point = point.filter(|_| !over_combat && !over_picker);
            let map_hit = self.map_tooltip(document, tooltip_point, extent, ui.ctx());
            self.combat_rows.end_frame();
            self.head_rows.end_frame();
            self.scripts.end_frame();
            // The counter whose tooltip that is: the same rebuilt hit test, at the same point.
            let tooltip_counter = tooltip_point.and_then(|point| match self.map_hit(point)? {
                MapHit::Unit(index) => self.counter_report.unit_hits[index].units().next(),
                _ => None,
            });
            laps.lap("map_tooltip");
            self.hover = hover_report::describe(
                point.and_then(|point| picker_drawn.as_ref()?.control_at(point)),
                self.counter_view.hovered,
            );
            self.tooltip = None;
            self.tooltip_delay_ms = document
                .icons
                .combat
                .tooltip_defines
                .as_ref()
                .ok()
                .map(|defines| defines.timing[0] * 1000.0);
            let mut tooltip_refused = map_hit
                .as_ref()
                .map(|(text, _, _)| text.refused.clone())
                .unwrap_or_default();
            // The target changes with the text shown: an empty tooltip leaves the last one up.
            if let Some((text, _, province)) = &map_hit
                && (!text.title.is_empty() || !text.body.is_empty())
            {
                self.map_tooltip_target = province
                    .map(highlight::TooltipTarget::Province)
                    .or(tooltip_counter.map(highlight::TooltipTarget::Counter));
            }
            self.shown_tooltip = None;
            let mut additions = Batch::default();
            let tooltip_time = Instant::now();
            // Hidden tooltips still follow the pointer; only their painting stops.
            if let Some((text, anchor)) = self.combat.tooltip(
                drawn.as_ref(),
                document.icons.combat.tooltip_defines.as_ref().ok(),
                point,
                map_hit.as_ref().map(|(text, rect, province)| {
                    (
                        text.title.as_str(),
                        Some(text.body.as_str()),
                        *rect,
                        *province,
                    )
                }),
                tooltip_time,
            ) && self.graphics.tooltips
            {
                // The map's own tooltip, with or without its delayed body, names its province; any
                // other drawn tooltip, such as one fading after the pointer left, is only `shown`.
                self.tooltip = Some(
                    map_hit
                        .as_ref()
                        .filter(|(tip, _, _)| text.starts_with(tip.title.as_str()))
                        .and_then(|(_, _, province)| *province)
                        .map_or_else(
                            || "shown".to_owned(),
                            |province| format!("province:{province}"),
                        ),
                );
                let refused = match self.prepared_overlay.as_ref() {
                    Some(prepared) => prepared.draw_tooltip(&now, &mut additions, text, anchor, extent, animation_time).map_err(|error| error.to_string())?,
                    None => [("the overlay is not laid out yet".to_owned(), 1)].into_iter().collect(),
                };
                // A refused painting, or a hidden interface, shows no tooltip.
                self.shown_tooltip = (refused.is_empty()
                    && interface
                    && self.combat.tooltip_from_map())
                    .then(|| self.map_tooltip_target.clone())
                    .flatten();
                for (reason, count) in refused {
                    *tooltip_refused.entry(reason).or_default() += count;
                }
            }
            // Captures inventory reachable counter hovers even when the pointer left the plate,
            // on the frames they write; a profiled sweep's other frames are the window's own.
            if self
                .capture
                .as_ref()
                .is_some_and(|capture| capture.remaining == 0)
            {
                let inventory = |inputs: &hover_rows::Inputs, report: &CounterReport| {
                    let key = inputs.key(hover_rows::What::Capture, [report.land_hovers.as_ptr() as usize ^ report.land_hovers.len(), report.task_force_hovers.as_ptr() as usize ^ report.task_force_port_hovers.as_ptr() as usize]);
                    let task_forces: Vec<_> = report.task_force_hovers.iter().filter(|hover| hover.supported).cloned().collect();
                    let ports: Vec<_> = report.task_force_port_hovers.iter().filter(|hover| hover.supported && hover.has_tooltip()).cloned().collect();
                    let (inputs, land) = (inputs.clone(), report.land_hovers.clone());
                    (key, move |_: &hover_rows::Caches| inputs.capture(land.clone(), task_forces.clone(), ports.clone()))
                };
                // Rows at the head of the open map with the vision its vision layer shows; today's
                // frame and the counters' vision with no playhead map.
                let made = match self.rows_head() {
                    Some((entry, recording)) => match self.shown_vision() {
                        Ok(vision) => {
                            let (key, job) = inventory(&self.hover_inputs(document, &vision, ui.ctx()), &self.counter_report);
                            self.hover_rows.wait(hover_rows::Family::Capture, (&entry, recording), key, job)
                        }
                        Err(_) => None,
                    },
                    None if self.open_map().is_some() => None,
                    None => match document.playback.current() {
                        crate::playback::Current::Frame(frame) => {
                            let (key, job) = inventory(&self.hover_inputs(document, &self.counter_report.perspective, ui.ctx()), &self.counter_report);
                            self.hover_rows.wait_today(hover_rows::Family::Capture, frame, key, job)
                        }
                        _ => None,
                    },
                };
                for (reason, &count) in made.iter().flat_map(|text| text.refused.iter()) {
                    let current = tooltip_refused.entry(reason.clone()).or_default();
                    *current = (*current).max(count);
                }
            }
            self.hover_rows.end_frame(self.capture.is_some());
            // A reader whose rows at the head do not hold what it reads yet says so, by name.
            for (reason, count) in self.hover_rows.loading().into_iter().chain(self.combat_rows.loading()) {
                *tooltip_refused.entry(reason).or_default() += count;
            }
            // The overlay's reads that its rows at the head do not hold yet, by name.
            if live {
                let tables = self.overlay_tables_job.loading();
                if !tables.is_empty() {
                    self.overlay_loading.push(format!("the overlay's tables: loading: {tables:?}"));
                } else if self.head_tables.is_none() {
                    self.overlay_loading.push("the overlay's tables: loading: not made from rows at the head yet".to_owned());
                }
                if !self.overlay_missed.is_empty() {
                    self.overlay_loading.push(format!("the overlay: loading: {:?}", self.overlay_missed));
                }
            }
            for reason in self.overlay_loading.drain(..) {
                *tooltip_refused.entry(reason).or_default() += 1;
            }
            self.tooltip_refused = tooltip_refused;
            if let Some(delay) = self.combat.tooltip_repaint_after(
                document.icons.combat.tooltip_defines.as_ref().ok(),
                tooltip_time,
            ) {
                ui.ctx().request_repaint_after(delay);
            }
            if let Some(drawn) = drawn {
                additions.quads.extend(drawn.batch.quads);
            }
            if let Some(picker_drawn) = picker_drawn {
                additions.quads.extend(picker_drawn.drawn.batch.quads);
            }
            laps.lap("combat_hover");
            (overlay_ms, additions)
        };
        let _other_cost = replay_frame::cpu::scope("frame.scene-inputs", Some("other"));
        // Overlay and hover diagnostics belong to this frame.
        let warnings = if build_warnings {
            self.collect_warnings(document)
        } else {
            Vec::new()
        };
        laps.lap("diagnostics");
        let render_cpu = viewer_foundation::profiling::thread_span("render");
        self.update_faction_highlight(
            document,
            &state.device,
            &state.queue,
            point.filter(|_| !over_combat),
        )?;
        // capture: at a fixed clock `Scene::render` takes the luminance
        // adaptation instantaneously, because the clock did not advance, which
        // is what a dump's own pair of frames does.
        let radar_at = point
            .filter(|_| !over_combat)
            .and_then(|point| radar_province(self.map_hit(point), &self.counter_report))
            .and_then(|province| Some((province, self.counter_report.perspective.province_controller(province)?)));
        let base_at = point.filter(|_| !over_combat).and_then(|point| {
            let MapHit::Base(index) = self.map_hit(point)? else {
                return None;
            };
            self.counter_report.air_base_hits.get(&index).copied()
        });
        // A hovered province's state, from the overlay's rows (at the head with the map open, none
        // while they load).
        let mut state_of = |province: u32| {
            let state = |frame: &replay_frame::frame::DecodedFrame| {
                let provinces = frame.complete::<replay_data::rows::tables::Provinces>().ok()?;
                provinces.binary_search_by_key(&province, |row| row.id).ok().map(|at| provinces[at].state)
            };
            if live { self.head_rows.read_overlay(now.frame, state).flatten() } else { state(now.frame) }
        };
        let hovered_radar = radar_at.and_then(|(province, controller)| Some((state_of(province)?, controller)));
        let hovered_air_base = base_at.and_then(|(province, kind)| Some((state_of(province)?, kind)));
        let hovered_air_carriers = point.filter(|_| !over_combat).and_then(|point| {
            let MapHit::Unit(index) = self.map_hit(point)? else {
                return None;
            };
            let hit = &self.counter_report.unit_hits[index];
            (!hit.port
                && hit.kind == viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce)
                .then(|| Arc::clone(&hit.members))
        });
        let (_, shown_hour) = self.hour_on_screen(document);
        let target_light = self.frame_day_night(shown_hour, &document.game_speeds);
        let day_night = self.day_night.update(
            shown_hour,
            target_light,
            f64::from(animation_time),
            1.0 / self.speed,
        );
        // With the playhead on, the radar symbols come from the radar layer.
        let layered_radar = self
            .playhead_map
            .as_ref()
            .filter(|side| side.draws("radar"))
            .and_then(|side| side.parts())
            .and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::radar::Radar>()))
            .and_then(crate::layers::radar::Radar::field);
        let layered = self.layered_orders();
        let scene = self.scene.as_mut().unwrap();
        let (device, queue) = (&state.device, &state.queue);
        // The radar symbols are the radar layer's with the map open, laid out or not.
        if let Some(field) = layered_radar.as_deref().or(self.prepared_overlay.as_ref().map(|prepared| &*prepared.radar)) {
            scene
                .apply(device, queue, SceneInput::SetRadar { field, shown: self.picker.radar, hovered: hovered_radar })
                .map_err(|e| e.to_string())?;
        }
        // The order parts: with the map open the layers' plan, the hour's raid arrows and the trade
        // routes, else today's hour field; the overlay's air orders join them once laid out.
        let (plan, raids, routes) = match &layered {
            Some(layered) => (layered.plan.clone(), layered.raids.clone(), layered.routes),
            None => (self.uploaded.orders.clone(), None, None),
        };
        let mut air = None;
        if let Some(prepared) = &self.prepared_overlay {
            let carrier_ranges = hovered_air_carriers
                .as_deref()
                .map(|forces| prepared.air_ranges.carrier_ranges(forces));
            let ranges = carrier_ranges
                .as_ref()
                .unwrap_or_else(|| prepared.air_ranges.shown(hovered_air_base));
            scene
                .apply(device, queue, SceneInput::SetAirRanges(&ranges.ranges))
                .map_err(|e| e.to_string())?;
            scene
                .apply(
                    device,
                    queue,
                    SceneInput::SetAirTransfers(&prepared.air_orders.transfers.progress),
                )
                .map_err(|e| e.to_string())?;
            scene
                .apply(
                    device,
                    queue,
                    SceneInput::SetAirAssignment(prepared.province_stripes()),
                )
                .map_err(|e| e.to_string())?;
            scene
                .apply(
                    device,
                    queue,
                    SceneInput::SetStrategicStyles(&prepared.naval_selection.styles),
                )
                .map_err(|e| e.to_string())?;
            air = Some((Arc::clone(&prepared.air_orders), ranges.highlighted_bases.clone()));
        }
        if layered.is_some() || plan.is_some() {
            let key = OrderKey { plan: order_at(plan.as_ref()), raids: order_at(raids.as_ref()), routes, air: air.as_ref().map(|(orders, bases)| (order_at(Some(orders)), bases.clone())) };
            if self.uploaded.order_key.as_ref() != Some(&key) {
                let air_field = match (&air, &key.air) {
                    (Some((orders, bases)), Some(held)) => {
                        if self.uploaded.air_field.as_ref().is_none_or(|(had, _)| had != held) {
                            self.uploaded.air_field = Some((held.clone(), Arc::new(orders.highlighted(bases))));
                        }
                        self.uploaded.air_field.as_ref().map(|(_, field)| Arc::clone(field))
                    }
                    _ => None,
                };
                let routes_field = routes.and(live_of(&self.playhead_map)).and_then(layered_convoys).map(|convoys| &convoys.product().field);
                scene
                    .apply(device, queue, SceneInput::SetOrders { assets: &document.orders, parts: [plan.as_deref(), raids.as_deref(), routes_field, air_field.as_deref()] })
                    .map_err(|error| error.to_string())?;
                self.uploaded.order_key = Some(key);
            }
        }
        scene
            .apply(
                &state.device,
                &state.queue,
                SceneInput::SetGraphics(self.graphics),
            )
            .map_err(|error| error.to_string())?;
        // The walking mode's figure stands in the scene while it walks, and the flying mode's
        // aircraft where this frame's flight placed it.
        let library = document.unit_models.library();
        let aircraft = document.aircraft.library();
        let flying = self.flying.take();
        let player = match self.camera_mode {
            CameraMode::Walk if free_mode => self.walk.player(&library),
            CameraMode::Fly => Self::fly_model(flying, &aircraft),
            _ => None,
        };
        scene
            .apply(&state.device, &state.queue, SceneInput::SetPlayer(player))
            .map_err(|error| error.to_string())?;
        drop((library, aircraft));
        scene
            .apply(
                &state.device,
                &state.queue,
                SceneInput::SetVisibility(self.counter_view.visibility),
            )
            .map_err(|error| error.to_string())?;
        let ours = self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables.ours()).or_else(|| seen.as_ref().map(|vision| vision.ours()));
        let (head, selected) = (head_rows::head(live_of(&self.playhead_map), document), self.unit_selection.units().as_ref());
        if let Some(ours) = ours {
            let mut arrows = |frame: &replay_frame::frame::DecodedFrame| {
                crate::document::select_movement(scene, &state.device, &state.queue, &document.content, frame, ours, selected)
            };
            // No selection reads no row; a selection's arrows are kept while its rows load.
            let moved = if selected.is_empty() { Some(arrows(now.frame)) } else { self.head_rows.arrows(head, frame, arrows) };
            if let Some(Err(error)) = moved {
                self.overlay_loading.push(format!("the selection's movement arrows: {error}"));
            }
        }
        scene
            .apply(
                &state.device,
                &state.queue,
                SceneInput::LabelFrame(u64::from(self.profile_frame)),
            )
            .map_err(|error| error.to_string())?;
        scene
            .apply(
                &state.device,
                &state.queue,
                SceneInput::CutProvinceFrame(animation_time),
            )
            .map_err(|error| error.to_string())?;
        // The frame's tooltip, combat and picker quads draw after the overlay's own, which keeps
        // them only for this render.
        let overlay = if self.looks.is_some() {
            &mut self.free_overlay
        } else {
            self.overlay.get_or_insert_with(Batch::default)
        };
        let laid_out = overlay.quads.len();
        overlay.quads.extend(additions.quads);
        // Only the window's own map is timed; a dashboard widget's shares the window's frames.
        let timed = !self.dashboard_map;
        if timed && let Some(frame) = crate::perf_log::frame_id() {
            scene.time_frame(frame);
        }
        let rendered = scene
            .render(
                &state.device,
                &state.queue,
                Request {
                    size: pixels,
                    camera,
                    day_night,
                    animation_time,
                    // The window runs one clock, so the map scene shares it.
                    map_time: animation_time,
                    // colour
                    postfx: self.postfx,
                    // The window has no verification aids; `--dump` owns them.
                    no_water: false,
                    // shadows: the window always draws them, as the game does.
                    no_shadows: false,
                    // names
                    // Hidden, the interface still steps; only its drawing stops.
                    overlay: Overlay {
                        names: self.placements.as_deref(),
                        layer_alpha: name_fade.0,
                        strategic_names,
                        sprites: interface.then_some(&*overlay),
                        ui_scale,
                        interface,
                        country_names: interface || flying_names,
                        ground: camera.free().map(|_| free_icons::Ground {
                            field: document.assets.height_field(),
                            sight: document.assets.terrain_sight(),
                        }),
                    },
                    // trees
                    no_trees: false,
                },
            );
        overlay.quads.truncate(laid_out);
        if timed {
            crate::perf_log::gpu_times(scene.gpu_frame_times(&state.device));
        }
        rendered.map_err(|e| e.to_string())?;
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.after-render", Some("other"));
        // Both coordinator paths stamp the same submitted scene, before audio,
        // capture/readback and asynchronous PROFILE output can delay observation.
        let shown_stamp = presentation::shown_stamp(profiling(), || {
            (document.playback.branch(), document.playback.hour())
        });
        let playback_stamp =
            profiling().then(|| self.profile_playback.stamp(self.playhead, self.playing));
        // A window capture stays silent; with `BATTLE_PLAN_VIEWER_AUDIO_LOG` it traces
        // the voices it would start without opening a device.
        let trace = self.capture.is_some() && std::env::var_os("BATTLE_PLAN_VIEWER_AUDIO_LOG").or_else(|| std::env::var_os("HOI4_VIEWER_AUDIO_LOG")).is_some();
        if trace {
            viewer_audio::System::silent();
        }
        if (self.capture.is_none() || trace) && self.audio_focus {
            present_audio(
                &mut self.audio,
                document,
                scene,
                &camera,
                AudioInputs {
                    volumes: self.volumes,
                    muted: self.muted,
                    reset: std::mem::take(&mut self.audio_reset),
                    music: self.playhead_map.as_ref().and_then(|_| self.scripts.music()).or_else(|| self.music.clone()),
                    battles: self.battles.clone(),
                    interface: std::mem::take(&mut self.interface_sounds),
                    interface_key: &mut self.interface_key,
                },
            );
        } else if !self.audio_focus
            && let Some(audio) = &mut self.audio
        {
            self.interface_sounds.clear();
            // Another map sounds now: what this one played stops.
            audio.release();
        }
        let [animate, overlay_upload, encode, submit] = scene.render_laps();
        if self.scene_target.stale(scene.generation()) {
            let view = scene.view().ok_or("Map target was not created")?;
            if let Some(host) = crate::renderer_host::local(state) {
                let target = crate::renderer_host::Target {
                    view: view.clone(),
                    filter: PRESENTATION_FILTER,
                };
                if let Some(texture) = self.scene_target.texture {
                    host.rebind(texture, target);
                } else {
                    self.scene_target.texture = Some(host.register(target));
                }
            } else {
                let mut egui_renderer = state.renderer.write();
                if let Some(texture) = self.scene_target.texture {
                    egui_renderer.update_egui_texture_from_wgpu_texture(
                        &state.device,
                        view,
                        PRESENTATION_FILTER,
                        texture,
                    );
                } else {
                    self.scene_target.texture = Some(egui_renderer.register_native_texture(
                        &state.device,
                        view,
                        PRESENTATION_FILTER,
                    ));
                }
            }
            self.scene_target.generation = scene.generation();
        }
        // capture: the frame is drawn and the scene still holds it, so the
        // readback is the dump's own.
        render_cpu.finish();
        laps.lap("render");
        if profiling() {
            if let Some(cpu) = laps.frame_cpu.take() {
                cpu.finish();
            }
            let profile = format!(
                "PROFILE frame {} fraction {} height {} total {:.6} overlay {:.6} spline {:.6} hour {} frame_total {:.3} outside {:.3} scene {:.3}/{:.3}/{:.3}/{:.3} gpu {:.3} laps {} detail {} spans {} thread {} cpu_hz {} {} {} perf_frame={} commands={:?}",
                self.profile_frame,
                document.hour_fraction,
                camera.height,
                map_timer.elapsed().as_secs_f64() * 1000.0,
                overlay_ms,
                viewer_foundation::map::counter_move::take_profile(),
                document.playback.hour(),
                laps.total(),
                laps.outside,
                animate,
                overlay_upload,
                encode,
                submit,
                gpu_wait(&state.device),
                laps.line(),
                self.scene
                    .as_ref()
                    .map_or_else(String::new, |scene| scene.detail_laps()),
                viewer_foundation::profiling::take_spans(),
                viewer_foundation::profiling::take_thread_spans(),
                viewer_foundation::profiling::cpu_clock_hz()
                    .map_or_else(|| "unavailable".to_owned(), |hz| hz.to_string()),
                playback_stamp.expect("PROFILE frame sampled its playback stamp"),
                shown_stamp.expect("PROFILE frame sampled its shown stamp"),
                crate::perf_log::frame_id().map_or_else(|| "unavailable".to_owned(), |id| id.to_string()),
                shader_pipeline::gpu::stats::take(),
            );
            crate::perf_log::profile_frame(&profile);
            profile_print(profile);
        }
        // A capture waits until every flag and portrait its frame asked for
        // has decoded and been laid out, as a headless dump does. The overlay's own layout version
        // is the reference, not a version read here: a background load that landed after the
        // layout but before this check would otherwise let the frame save without it.
        let (_, shown_at) = self.hour_on_screen(document);
        if let Some(capture) = self.capture.as_mut().filter(|capture| !capture.paced) {
            // Anything the settle decodes moves the version, which makes the overlay stale.
            document.icons.lazy.settle();
            let laid_out = self
                .prepared_overlay
                .as_ref()
                .is_some_and(|prepared| !prepared.is_stale(&now));
            // With the map open, laid out over the newest rows at the head with tables made from
            // them, nothing loading.
            let at_head = !live
                || (self.overlay_missed.is_empty()
                    && self.overlay_tables_job.loading().is_empty()
                    && !self.overlay_tables_job.running()
                    && self.head_tables.as_ref().zip(overlay_rows.as_ref()).is_some_and(|((key, _), rows)| key.rows == Arc::as_ptr(rows) as usize && rows.hour == shown_at));
            let laid_out = laid_out && at_head;
            // And until the raid offer cycle is complete at the shown hour: every target
            // evaluated, as a paused frame ends. With the map open, the scripts reader's batch
            // the tables hold.
            let raids = self
                .overlay_tables
                .as_ref()
                .map(|(_, _, _, tables)| tables.raids());
            let cycled =
                raids.is_some_and(|raids| !raids.report.pending && raids.report.cycle_complete());
            if !laid_out || document.icons.lazy.has_pending() || !cycled {
                capture.remaining = capture.remaining.max(1);
            }
        }
        self.save_capture(state, document, camera, pixels, &warnings, ui.ctx());
        let mut warnings = warnings;
        if let Some(audio) = self.audio.as_ref().filter(|audio| audio.owns()) {
            let status = audio.status();
            if let Some(Err(error)) = &status.device {
                warnings.push(format!("Map audio is silent: {error}."));
            }
            warnings.extend(
                status
                    .notes
                    .iter()
                    .map(|note| format!("Map audio: {note}.")),
            );
            if let Some(scene) = &self.scene {
                warnings.extend(
                    scene.sound_refusals.iter().map(|(refusal, actors)| {
                        format!("Map audio: {refusal} (actors: {actors}).")
                    }),
                );
            }
            // The shown document: its owner holds it apart from `self` while it draws.
            if let Some(source) = &document.audio
                && let Some(why) = viewer_audio::battle::silence(&source.content)
            {
                warnings.push(format!("Map battle ambience: {why}."));
            }
            if let Some(battles) = &self.battles
                && !battles.refusals.is_empty()
            {
                warnings.push(format!(
                    "Map battle ambience: {}.",
                    battles.refusals.join("; ")
                ));
            }
            let music = match &self.playhead_map {
                Some(_) => self.scripts.music(),
                None => self.music.clone(),
            };
            if let Some(music) = &music
                && !music.refusals.is_empty()
            {
                warnings.push(format!(
                    "Map music keeps the base chance of songs whose conditions the recording cannot answer: {}.",
                    music.refusals.join("; ")
                ));
            }
        }
        self.map_warnings = warnings;
        self.profile_frame += 1;
        let texture = self
            .scene_target
            .texture
            .ok_or("Map target was not registered")?;
        ui.painter().image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // The desktop's campaign map has no panels, so its diagnostics sit
        // over the map; dashboard maps draw theirs beside their loading status.
        // Every map on the software adapter, windowed or on a dashboard, says so.
        let software = is_software(&state.adapter);
        let diagnostics = self.diagnostics && self.embedded.is_some() && !self.dashboard_map;
        let partial = document.playback.replay().is_partial()
            && self.embedded.is_some()
            && !self.dashboard_map;
        // Every map of a recording that leaves tables out says so.
        let scoped = replay_data::scope::omits(document.capture_mode()).is_some();
        // A fly or walk mode chosen from the map says how to choose where it starts.
        let start_prompt = self
            .start_pick
            .choosing()
            .then(|| start_prompt(self.camera_mode));
        // Hatched snow and mud say why in ordinary viewing too.
        let weather = self.weather_notice();
        if software
            || diagnostics
            || partial
            || scoped
            || start_prompt.is_some()
            || weather.is_some()
        {
            map_overlay(
                ui,
                rect.intersect(ui.clip_rect()),
                "diagnostics",
                self.interface_points,
                |ui, height| {
                    if let Some(prompt) = start_prompt {
                        ui.colored_label(egui::Color32::WHITE, prompt);
                    }
                    if partial {
                        partial_notice(ui, document.playback.replay());
                    }
                    if scoped {
                        capture_mode_notice(ui, document.playback.replay());
                    }
                    if let Some(weather) = weather {
                        ui.colored_label(egui::Color32::YELLOW, format!("{weather}."));
                    }
                    if software {
                        ui.colored_label(egui::Color32::YELLOW, SOFTWARE_RENDERER_NOTICE);
                    }
                    if diagnostics {
                        self.diagnostics_ui(ui, height);
                    }
                },
            );
        }
        // A document's first picture is shown whole or not at all: until its overlay and counter
        // layers show the hour with its layers, the spinner covers what is drawn beneath.
        self.shown_whole = self.shown_whole || self.playhead.is_some_and(|head| self.picture_whole(head, self.playing));
        if !self.shown_whole {
            let cover = rect.intersect(ui.clip_rect());
            ui.painter().rect_filled(cover, 0.0, ui.visuals().panel_fill);
            ui.put(egui::Rect::from_center_size(cover.center(), egui::vec2(32.0, 32.0)), egui::Spinner::new());
        }
        // The water animates, so the window keeps drawing.
        ui.ctx().request_repaint();
        self.drew_frame = true;
        let overlay_fresh = self.prepared_overlay.as_ref().is_some_and(|prepared| !prepared.is_stale(&now));
        if self.load_complete(document.icons.lazy.busy(), overlay_fresh) {
            if document.content.finish_loading()
                && let Some(source) = &self.source
                && std::env::var_os("BATTLE_PLAN_SOURCE_ADMISSION_FILE").or_else(|| std::env::var_os("HOI4_SOURCE_ADMISSION_FILE")).is_some()
                && let Ok(options) = self.config.resolve_options()
            {
                crate::perf_log::source_admission_probe(source, &options);
            }
        }
        self.drawn_at = Some(crate::perf_log::epoch_ms());
        self.drawn_options.clone_from(&self.applying_options);
        // The layers show whole hours.
        let (branch, hour) = self.hour_on_screen(document);
        let fraction = if self.playhead_map.is_some() { 0.0 } else { f64::from(document.hour_fraction) };
        self.rendered = Some(Playhead { branch, hour, fraction });
        Ok(())
    }
}

impl Viewer {
    /// Switches how the full map's camera moves and whether the free modes draw the map's
    /// interface, and reports a change to the host, whose controls follow it. Leaving a free mode
    /// gives the pointer back and returns to the map camera it left; entering one starts its free
    /// eye where the map camera looks from.
    pub(crate) fn set_camera_mode(
        &mut self,
        mode: CameraMode,
        overlays: bool,
        ctx: &egui::Context,
    ) {
        // A free mode without its interface keeps no window of it open to take clicks.
        if mode != CameraMode::Map && !overlays {
            self.combat.close();
        }
        if (mode, overlays) != (self.camera_mode, self.free_overlays)
            && let Some(embedded) = &self.embedded
        {
            embedded.report_camera_mode(mode, overlays);
        }
        self.free_overlays = overlays;
        if mode == self.camera_mode {
            return;
        }
        self.camera_mode = mode;
        // A free mode chosen from the map waits for the ground it starts over.
        self.start_pick = if mode == CameraMode::Map {
            drop::Pick::Idle
        } else {
            drop::Pick::Choosing
        };
        self.walk.leave();
        self.flight = None;
        self.input.release();
        self.grab_pointer(ctx);
    }

    /// A click on the counter `hit` (of [`CounterReport::unit_hits`]) or on no icon: selects its
    /// units, adding to the selection with shift.
    fn click_units(&mut self, hit: Option<usize>, ctx: &egui::Context) {
        let hit = hit.map(|index| &self.counter_report.unit_hits[index]);
        let shift = ctx.input(|input| input.modifiers.shift);
        let units_changed = self.unit_selection.click(hit, shift);
        let fleets_changed = if !shift {
            self.fleet_selection.clear()
        } else {
            false
        };
        let air_changed = if !shift || hit.is_some_and(|hit| hit.ours) {
            self.air_selection.clear()
        } else {
            false
        };
        if units_changed || air_changed || fleets_changed {
            self.counter_view.selected = Arc::clone(self.unit_selection.units());
            self.overlay_at = None;
            ctx.request_repaint();
        }
    }

    /// Hides and confines the cursor while the map holds the pointer, and gives it back after.
    fn grab_pointer(&mut self, ctx: &egui::Context) {
        let captured = self.input.captured() && self.camera_mode != CameraMode::Map;
        if captured == self.pointer_grabbed {
            return;
        }
        self.pointer_grabbed = captured;
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(if captured {
            egui::CursorGrab::Confined
        } else {
            egui::CursorGrab::None
        }));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!captured));
    }
}

/// A view's audio choices and state for one presented frame.
struct AudioInputs<'a> {
    volumes: dashboard::protocol::MapAudio,
    muted: bool,
    /// A presentation discontinuity since the last audio frame.
    reset: bool,
    music: Option<Arc<viewer_audio::Chances>>,
    battles: Option<Arc<viewer_audio::Battles>>,
    /// The interface's button sounds since the last audio frame, each started once.
    interface: Vec<InterfaceSound>,
    interface_key: &'a mut u64,
}

/// Sends the rendered frame's sound events to the process audio. Callers
/// skip window captures and maps that may not sound.
/// The volumes and mute the audio mixes with: the map's inherited volumes
/// and the game's own `dev_master_volume`.
fn audio_mix(
    document: &Document,
    volumes: dashboard::protocol::MapAudio,
    muted: bool,
) -> viewer_audio::Mix {
    let percent = |value: f32| value / 100.0;
    viewer_audio::Mix {
        master: percent(volumes.master),
        dev_master: document.game_dev_master,
        ambient: percent(volumes.ambient),
        units: percent(volumes.units),
        combat: percent(volumes.combat),
        interface: percent(volumes.interface),
        voice: percent(volumes.voice),
        music: percent(volumes.music),
        muted,
    }
}

/// One of a frame's interface actions, dispatched in its changes' order.
enum Act {
    /// A picker control clicked, by the secondary button when `true`.
    Picker(viewer_combat::picker::Control, bool),
    /// The combat window's click of that index.
    Combat(usize),
    /// A click off the picker, which closes its open list.
    Dismiss,
    /// A counter's click, which selects it.
    Counter(Unit),
    /// A click on the bare map there.
    Map([f32; 2]),
}

/// What takes the primary button's press on the map's interface.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holder {
    Picker,
    Combat,
    Counters,
}

/// The emitter keys of interface sounds, apart from the map's events; clicks take the upper half.
const INTERFACE_SOUND_KEYS: u64 = 1 << 63;
const INTERFACE_CLICK_KEYS: u64 = 3 << 62;

/// An interface sound to start, and for a click the key its control's clicks share.
struct InterfaceSound {
    effect: Arc<str>,
    click: Option<u64>,
    /// Its change's order in the frame ([`viewer_foundation::map::native_gui::pointer::Step`]).
    order: usize,
}

impl InterfaceSound {
    /// `played` by one of `producer`'s controls.
    fn new(producer: &str, played: &viewer_foundation::map::native_gui::pointer::Played) -> Self {
        use std::hash::{Hash, Hasher};
        Self {
            effect: Arc::from(String::from_utf8_lossy(&played.effect)),
            click: played.click.as_ref().map(|control| {
                let mut hasher = std::hash::DefaultHasher::new();
                (producer, control).hash(&mut hasher);
                INTERFACE_CLICK_KEYS | (hasher.finish() >> 2)
            }),
            order: played.order,
        }
    }
}

fn present_audio(
    audio: &mut Option<viewer_audio::Owner>,
    document: &Document,
    scene: &Scene,
    camera: &Camera,
    inputs: AudioInputs<'_>,
) {
    let AudioInputs {
        volumes,
        muted,
        reset,
        music,
        battles,
        interface,
        interface_key,
    } = inputs;
    let Some(source) = &document.audio else {
        // A document without audio definitions plays nothing: what a
        // previous document played stops.
        if let Some(owner) = audio {
            owner.release();
        }
        return;
    };
    let owner = audio.get_or_insert_with(|| viewer_audio::System::shared().owner());
    owner.claim();
    let mut emitters = scene
        .sound_events
        .iter()
        .map(|event| viewer_audio::Emitter {
            key: event.key,
            effect: Arc::clone(&event.effect),
            position: event.position,
            age: event.age,
            group: if event.combat {
                viewer_audio::Group::Combat
            } else {
                viewer_audio::Group::Units
            },
            restart: false,
        })
        .collect::<Vec<_>>();
    // Interface sounds are present for one frame; their one-shot voices play out. A control's
    // click replaces its earlier one, so a later click this frame leaves the earlier unstarted.
    for (at, sound) in interface.iter().enumerate() {
        if sound.click.is_some()
            && interface[at + 1..]
                .iter()
                .any(|later| later.click == sound.click)
        {
            continue;
        }
        let key = sound.click.unwrap_or_else(|| {
            *interface_key = interface_key.wrapping_add(1);
            INTERFACE_SOUND_KEYS | (*interface_key & ((1 << 62) - 1))
        });
        emitters.push(viewer_audio::Emitter {
            key,
            effect: Arc::clone(&sound.effect),
            position: camera.eye(),
            age: 0.0,
            group: viewer_audio::Group::Interface,
            restart: false,
        });
    }
    // The hour's battles within hearing of the camera play their ambience.
    if let Some(battles) = &battles {
        viewer_audio::battle::emitters(battles, &source.content, camera.eye(), &mut emitters);
    }
    let forward = camera.look_direction();
    // The camera's right axis: up crossed with forward, normalised.
    let right = [forward[2], 0.0, -forward[0]];
    let length = (right[0] * right[0] + right[2] * right[2]).sqrt();
    let right = if length > 0.0 {
        [right[0] / length, 0.0, right[2] / length]
    } else {
        [1.0, 0.0, 0.0]
    };
    owner.frame(
        source,
        viewer_audio::Frame {
            listener: viewer_audio::Listener {
                position: camera.eye(),
                right,
                altitude: camera.height,
            },
            emitters,
            mix: audio_mix(document, volumes, muted),
            reset,
            elapsed: 0.0,
            hour: Some(document.playback.hour()),
            music,
        },
    );
}

impl eframe::App for Viewer {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.options.window_dump {
            // Dumps keep window geometry, but never consume live input.
            input.events.clear();
            input.events.push(egui::Event::PointerGone);
            input.hovered_files.clear();
            input.dropped_files.clear();
            input.focused = false;
            return;
        }
        crate::suite_input::apply(ctx, input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        crate::perf_log::frame_start();
        // A window not shown (minimised, covered, hidden by its host) runs no UI pass, so its maps
        // draw no frame: they let their playheads go, or the rows would wait on them.
        if ctx.input(|input| input.viewport().visible()) == Some(false) || self.embedded.as_ref().is_some_and(|embedded| !embedded.visible()) {
            self.not_drawn();
        }
        engine_executable::executable_tables::scope(self.tables(), || {
            self.embedded_logic(ctx, frame)
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::perf_log::frame_ui();
        let tables = self.tables();
        engine_executable::executable_tables::scope(tables, || self.campaign_ui(ui, frame));
        crate::perf_log::frame_end();
    }
}

impl Viewer {
    /// The executable tables of the recording this viewer shows, which its work reads.
    pub(crate) fn tables(&self) -> Option<engine_executable::executable_tables::Tables> {
        self.document
            .as_ref()
            .and_then(|document| document.content.tables)
    }

    fn campaign_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(recorder) = viewer_foundation::frame_writer::recorder() {
            let mut recording = Some(recorder.pass());
            let drawn = self.update(ui, frame, &mut recording);
            if let Some(recording) = recording {
                recording.record(
                    self.document
                        .as_ref()
                        .map(|document| (document.playback.hour(), document.hour_fraction)),
                    drawn,
                    "campaign",
                );
            }
        } else {
            self.update(ui, frame, &mut None);
        }
    }
}

impl Viewer {
    fn update(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        recording: &mut Option<viewer_foundation::frame_writer::Pass<'_>>,
    ) -> bool {
        // The hide command may arrive in logic after eframe chose to draw
        // this pass. Skip it as well as all later hidden passes.
        if self
            .embedded
            .as_ref()
            .is_some_and(|embedded| !embedded.visible())
        {
            self.not_drawn();
            if self.dashboard_active
                && let Some(recording) = recording.take()
            {
                recording.record(None, false, "-");
            }
            return false;
        }
        if ui.input(|input| input.events.iter().any(is_diagnostics_shortcut)) {
            self.toggle_diagnostics();
        }
        if self.dashboard_active {
            // The Campaign map is not drawn while dashboard maps show, even
            // while they load or failed to: it stops sounding.
            self.release_audio();
            self.close_playhead();
        } else if let Some(dashboard) = &mut self.dashboard {
            // Nor are dashboard maps while the Campaign map shows, loaded or not.
            dashboard.release_audio();
            dashboard.close_playheads();
        }
        if self.dashboard_active
            && let Some(dashboard) = &mut self.dashboard
        {
            dashboard.ui_scale = self.ui_scale;
            dashboard.ui(ui, frame, recording.take());
            if let Some(embedded) = &mut self.embedded {
                embedded.report_maps(dashboard.states());
                for (view, event) in dashboard.take_view_events() {
                    match event {
                        picker::ViewEvent::Options(options) => {
                            embedded.report_view_options(&view, options)
                        }
                        picker::ViewEvent::Toggle(toggle) => {
                            embedded.report_view_toggle(&view, toggle)
                        }
                        picker::ViewEvent::Modes(modes) => embedded.report_view_modes(&view, modes),
                    }
                }
            }
            return false;
        }
        // The campaign map's picker changes go to the host as a widget's do, under its own view
        // id, and come back in its Configure options.
        let events = self.take_view_events();
        if let Some(embedded) = &self.embedded {
            for event in events {
                match event {
                    picker::ViewEvent::Options(options) => embedded
                        .report_view_options(dashboard::protocol::CAMPAIGN_MAP_VIEW, options),
                    picker::ViewEvent::Toggle(toggle) => {
                        embedded.report_view_toggle(dashboard::protocol::CAMPAIGN_MAP_VIEW, toggle)
                    }
                    picker::ViewEvent::Modes(modes) => {
                        embedded.report_view_modes(dashboard::protocol::CAMPAIGN_MAP_VIEW, modes)
                    }
                }
            }
        }
        let waited = if profiling() && self.capture.is_some() {
            pace_profiled_frame()
        } else {
            Duration::ZERO
        };
        self.laps = Some(Laps::new(waited));
        self.poll(ui.ctx());
        self.lap("poll");
        let central_panel = egui::CentralPanel::default().frame(egui::Frame::NONE);
        if !profiling() || self.capture.is_none() {
            self.profile_playback.configure(None, false);
        }
        let drawn = if let Some(mut document) = self.document.take() {
            if profiling() && self.capture.is_some() {
                // Deterministic diagnostic sweep, including at a replay's final
                // hour. Ordinary playback follows the host's clock.
                let mode = std::env::var("BATTLE_PLAN_VIEWER_PROFILE").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE")).unwrap_or_default();
                let sweep_frames = profiling_sweep();
                // `hours`: one hour forward at the sweep's start, back at its middle.
                let middle = sweep_frames.start + sweep_frames.len() as u32 / 2;
                let mut sought = false;
                if let Some(playhead) = &mut self.playhead {
                    if mode == "hours"
                        && (self.profile_frame == sweep_frames.start
                            || self.profile_frame == middle)
                    {
                        sought = true;
                        playhead.hour += if self.profile_frame == sweep_frames.start {
                            1
                        } else {
                            -1
                        };
                        playhead.fraction = 0.0;
                    }
                    // A cold seek: jump to the branch's last hour, whose
                    // per-branch caches (the snow and mud display cache) start
                    // empty.
                    if self.profile_frame == sweep_frames.start
                        && mode == "seek_end"
                        && let Some((_, last)) = document.playback.range(playhead.branch)
                    {
                        sought = true;
                        playhead.hour = last;
                        playhead.fraction = 0.0;
                    }
                }
                if sought {
                    self.animation_seek = true;
                    if let Some(hours) = &mut self.hours {
                        hours.seek();
                    }
                }
                // `play:` keeps nominal frame steps; `playwall:` measures elapsed window time.
                let playing = profile_playback::Mode::parse(&mode);
                self.playing = playing.is_some() && profiling_sweep().contains(&self.profile_frame);
                if let Some(mode) = playing {
                    self.speed = mode.speed();
                }
                self.profile_playback.configure(playing, self.playing);
                if self.playing
                    && !self.profile_playback.wall_active()
                    && let Some(head) = self.playhead
                {
                    let last = document
                        .playback
                        .range(head.branch)
                        .map_or(head.hour, |(_, last)| last);
                    self.tick_playhead(1.0 / hz(), last);
                }
                if playing.is_none()
                    && !mode.starts_with("pan:")
                    && !mode.starts_with("zoom:")
                    && let Some(playhead) = &mut self.playhead
                {
                    playhead.fraction = if !profiling_sweep().contains(&self.profile_frame) {
                        f64::from(self.options.hour_fraction)
                    } else {
                        f64::from((self.profile_frame - sweep_frames.start) % 120) / 120.0
                    };
                }
                // A camera sweep: `pan:<world units per second>` drifts the
                // pinned camera east, `zoom:<height factor per second>`
                // compounds its height, over the sweep's frames with the hour
                // held. Every frame rebuilds the overlay synchronously for
                // its camera, which is the interactive-pan cost being
                // measured; the hour builders chase the moving view behind
                // it. Needs the pinned `--camera` the harness passes.
                let sweep = mode
                    .strip_prefix("pan:")
                    .and_then(|rate| {
                        rate.parse::<f32>()
                            .ok()
                            .map(|rate| (rate / hz() as f32, 1.0))
                    })
                    .or_else(|| {
                        mode.strip_prefix("zoom:").and_then(|factor| {
                            factor
                                .parse::<f32>()
                                .ok()
                                .filter(|factor| *factor > 0.0)
                                .map(|factor| (0.0, factor.powf(1.0 / hz() as f32)))
                        })
                    });
                if let Some((dx, zoom)) = sweep
                    && profiling_sweep().contains(&self.profile_frame)
                    && let Some(capture) = self.capture.as_mut()
                    && let Some((_, camera)) = capture.pinned.as_mut()
                    && let Some(camera) = camera.as_mut()
                {
                    camera.center[0] += dx;
                    camera.height = (camera.height * zoom).clamp(40.0, 900.0);
                }
                // `mode:<token>`: the picker's choice of that map mode at the
                // sweep's start, the switch being measured.
                if self.profile_frame == sweep_frames.start
                    && let Some(next) = mode
                        .strip_prefix("mode:")
                        .and_then(viewer_world::map::map_mode::MapMode::parse)
                {
                    self.choose_map_mode(next);
                }
                // `fly`: a `--fly` dump's flight steps by the profiled pace over the
                // sweep's frames, as a window's clock would run.
                if mode == "fly"
                    && profiling_sweep().contains(&self.profile_frame)
                    && let Some(capture) = self.capture.as_mut()
                {
                    capture.time = capture.time.map(|time| time + (1.0 / hz()) as f32);
                }
            }
            self.lap("profile_sweep");
            let drawn = central_panel
                .show(ui, |ui| {
                    let result = self.map(ui, frame, &mut document);
                    self.after_frame_with_visual_mods(&document, result.as_ref().err());
                    if let Err(error) = result {
                        self.failed_window_capture(&error, ui.ctx());
                        self.error = Some(error);
                        false
                    } else {
                        self.drew_frame
                    }
                })
                .inner;
            self.document = Some(document);
            drawn
        } else {
            // No map is shown, while a replacement loads or after it failed:
            // what the previous one played stops.
            self.release_audio();
            // The host shows its own status and errors; the map only says it is loading.
            central_panel.show(ui, |ui| {
                if self.loading.is_some() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.spinner();
                        ui.heading("Opening replay");
                        ui.label("Verifying content and loading the map...");
                    });
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
            });
            false
        };
        self.report_embedded(ui.ctx());
        mark_frame_end();
        drawn
    }
}

pub(crate) fn date(hour: i32) -> String {
    let value = i64::from(hour) - 43_800_000;
    if value < 0 {
        return format!("Hour {hour}");
    }
    let days = value / 24;
    let year = days / 365;
    let mut day = days % 365;
    let mut month = 1;
    for length in [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] {
        if day < length {
            break;
        }
        day -= length;
        month += 1;
    }
    format!(
        "{year:04}-{month:02}-{:02}  {:02}:00",
        day + 1,
        value % 24 + 1
    )
}

/// Hands the raid lane the hour put on screen, with its frame, and starts
/// its offer cycle again there when the presentation history resets:
/// exactly when the trains and the weather do (a seek, same-hour ones too, a
/// step back, a branch change, a new recording, a recorded gap or a new
/// resolved viewing country); playback, one hour forward and a playback skip
/// carry it ([`viewer_foundation::map::presentation::continuity`]). The view and the reset are
/// installed together, before the lane wakes.
fn present_raids(
    lane: &crate::raid_lane::RaidLane,
    continuity: Continuity,
    presented: crate::raid_lane::Presented<'_>,
) {
    lane.present(presented, continuity == Continuity::Reset);
}

/// A built hour as the builders deliver it to the window.
trait Delivered {
    /// The [`hours::Want`] generation it was built for.
    fn epoch(&self) -> u64;
    fn at(&self) -> (replay_data::BranchId, i32);
    fn current(&self) -> &crate::playback::Current;
    fn viewing(&self) -> Option<&Arc<hours::ViewingCountry>> {
        None
    }
}

impl Delivered for hours::Built {
    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn at(&self) -> (replay_data::BranchId, i32) {
        (self.branch, self.hour)
    }

    fn current(&self) -> &crate::playback::Current {
        &self.current
    }
    fn viewing(&self) -> Option<&Arc<hours::ViewingCountry>> {
        self.viewing.as_ref()
    }
}

/// Hands a delivered hour's frame to the raid lane. Every path that
/// receives a built hour calls it before the hour is consumed, whatever its
/// epoch (raid offers do not read the fog): the lane builds its cycle only
/// from the frames it is fed, and a capture waits for that cycle.
/// `BATTLE_PLAN_VIEWER_CHECK_OVERLAY`: after every raid batch installed in place,
/// lay the whole overlay out again and require the same quads and report
/// ([`Viewer::check_overlay`]).
fn check_overlay() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_OVERLAY").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_OVERLAY")).is_some())
}

fn feed_raids(lane: &crate::raid_lane::RaidLane, built: &impl Delivered) {
    let (branch, hour) = built.at();
    lane.feed(branch, hour, built.current(), built.viewing());
}

/// Blocks on `wait` until the hour `target` of `epoch` arrives, feeding
/// every hour received to the raid lane on the way ([`feed_raids`]). Other
/// hours of `epoch` are kept in `ready`, other epochs' go to `stale`.
fn wait_for_hour<B: Delivered>(
    lane: &crate::raid_lane::RaidLane,
    (target, epoch): ((replay_data::BranchId, i32), u64),
    mut wait: impl FnMut() -> std::io::Result<B>,
    ready: &mut BTreeMap<(replay_data::BranchId, i32), B>,
    mut stale: impl FnMut(B),
) -> std::io::Result<B> {
    loop {
        let built = wait()?;
        feed_raids(lane, &built);
        if built.epoch() != epoch {
            stale(built);
        } else if built.at() == target {
            return Ok(built);
        } else {
            ready.insert(built.at(), built);
        }
    }
}

/// What the hours strictly between the hour on screen and `next` built,
/// oldest first, so the scene's building history advances through them; empty
/// for the same hour rebuilt. `built` answers `None` for an hour never built
/// and `Some(None)` for one built without buildings (a gap). The result is
/// `None` when `next` is on another branch or earlier, or a passed hour has no
/// buildings: the scene then treats it as a seek. An hour never built is
/// passed over when `passes_unbuilt`, and otherwise also makes it a seek.
fn skipped_hours<T>(
    shown: Option<(replay_data::BranchId, i32)>,
    (branch, next): (replay_data::BranchId, i32),
    passes_unbuilt: bool,
    built: impl Fn(i32) -> Option<Option<T>>,
) -> Option<Vec<T>> {
    let (shown_branch, from) = shown?;
    if shown_branch != branch || next < from {
        return None;
    }
    let mut passed = Vec::new();
    for hour in from + 1..next {
        match built(hour) {
            Some(buildings) => passed.push(buildings?),
            None if passes_unbuilt => {}
            None => return None,
        }
    }
    Some(passed)
}

// Late anchors must neither reverse nor stall a running renderer.
fn anchor_clock(current: Option<Playhead>, target: Playhead, forward: bool) -> Playhead {
    match current {
        Some(current) if forward && current.branch == target.branch => {
            let lead = f64::from(current.hour) - f64::from(target.hour) + current.fraction
                - target.fraction;
            if lead > 0.0 { current } else { target }
        }
        _ => target,
    }
}

fn tick_clock(head: &mut Playhead, speed: f64, seconds: f64, last: i32) -> bool {
    let step = seconds.max(0.0) * speed;
    advance_clock(head, head.fraction + step, last)
}

/// Moves a playing playhead `fraction` hours past its hour, whether or not the
/// builders have reached it: the map shows the newest built hour at or before
/// it ([`latest_built_key`]). Returns false at the branch's last hour.
fn advance_clock(playhead: &mut Playhead, fraction: f64, last: i32) -> bool {
    if fraction >= f64::from(last) - f64::from(playhead.hour) {
        playhead.hour = last;
        playhead.fraction = 0.0;
        return false;
    }
    playhead.hour = (f64::from(playhead.hour) + fraction.floor()) as i32;
    playhead.fraction = fraction.fract();
    true
}

/// The newest `usable` hour of `target`'s branch at or before it.
fn latest_built_key<T>(
    ready: &BTreeMap<(replay_data::BranchId, i32), T>,
    target: (replay_data::BranchId, i32),
    usable: impl Fn(&T) -> bool,
) -> Option<(replay_data::BranchId, i32)> {
    ready
        .range((target.0, i32::MIN)..=target)
        .rev()
        .find(|(_, built)| usable(built))
        .map(|(key, _)| *key)
}

/// Whether `rendered` answers `target`: exactly, or, playing, as an earlier
/// hour of its branch, which the acknowledgement reports as behind.
fn presentation_matches(rendered: Playhead, target: Playhead, playing: bool) -> bool {
    rendered == target
        || (playing && rendered.branch == target.branch && rendered.hour <= target.hour)
}

/// Whether the playhead's layers answer `hour`: held, they all show it settled for the view;
/// playing, they show an hour since the last seek and no later than the head they took.
/// Whether the picture over the playhead's map answers `hour` whole: its layers (drawn, settled),
/// the overlay laid out over the rows at the head, and the counter layers fed the tables made from
/// them, each by the hour it shows (none before its first). Paused, each shows `hour`; playing,
/// each an hour from the last seek to the head. No part is exempt and none is waited for on a clock.
fn whole_picture(
    on: &crate::playhead_host::OnScreen,
    overlay: Option<replay_data::Hour>,
    counters: Option<replay_data::Hour>,
    hour: i32,
    playing: bool,
) -> bool {
    let shows = |shown: Option<replay_data::Hour>| {
        shown.is_some_and(|shown| match playing {
            true => on.sought.is_none_or(|sought| sought <= shown) && shown <= on.head,
            false => shown == hour,
        })
    };
    layers_show(on, hour, playing) && shows(overlay) && shows(counters)
}

fn layers_show(on: &crate::playhead_host::OnScreen, hour: i32, playing: bool) -> bool {
    on.drawn.is_some_and(|drawn| match playing {
        true => on.sought.is_none_or(|sought| sought <= drawn) && drawn <= on.head,
        false => drawn == hour && on.settled,
    })
}

/// Why the playhead's layers are behind, by the layer furthest behind.
fn layers_behind_reason(on: &crate::playhead_host::OnScreen) -> String {
    let holding = match on.furthest {
        Some(layer) => layer,
        None if on.composing => "composing a reset",
        None => "no layer drawn yet",
    };
    format!("the playhead's layers: {holding}")
}

/// Whether the shown overlay, `(engaged_covered, branch, hour)`, was laid out
/// before the engaged index covered its hour (by the window or by an hour
/// builder) and the index now does: it is laid out again once, so a paused
/// seek or a prebuilt hour settles on the exact flag, and an overlay laid out
/// with the index is never redone for it.
fn engaged_relayout(
    shown: Option<(bool, replay_data::BranchId, i32)>,
    covers: impl Fn(replay_data::BranchId, i32) -> bool,
) -> bool {
    shown.is_some_and(|(covered, branch, hour)| !covered && covers(branch, hour))
}

/// A click's read: none while its rows load; with the map open a failed read is a named line and
/// selects nothing, without it the frame's error as before.
fn read_click<T>(live: bool, lines: &mut Vec<String>, what: &str, read: Option<std::io::Result<T>>) -> Result<Option<T>, String> {
    match read.transpose() {
        Ok(value) => Ok(value),
        Err(error) if live => {
            lines.push(format!("{what}: {error}"));
            Ok(None)
        }
        Err(error) => Err(error.to_string()),
    }
}

/// The order field the map's layers give the scene, made as today's hour field is: the battle plan
/// (its labels and report kept), the hour's raid arrows, then the trade-route lines.
/// A saved frame's battle plan is the plan of its hour: the map's layers settled again until the
/// orders layer's plan (or refusal) stands for the rows at `hour`, the tables a build learned
/// joining those rows (the stepped run's clock is held), within `limit`; refused by name.
fn settle_plan(side: &mut crate::playhead_host::MapSide, hour: replay_data::Hour, limit: Duration) -> Result<(), String> {
    let until = Instant::now() + limit;
    loop {
        let orders = side.draws("orders").then(|| side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::orders::Orders>()))).flatten();
        // Not drawn, or refused for want of a viewing country: nothing to wait for.
        let Some(orders) = orders.filter(|orders| orders.perspective().is_some()) else { return Ok(()) };
        let made = orders.made_for();
        if made == Some(hour) {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(format!("The battle plan at hour {hour}: the orders layer's plan stands for the rows at {made:?} after {limit:?} (none: a build or a table it learned is still coming)"));
        }
        side.settle_all().map_err(|error| format!("The battle plan at hour {hour}: {error}"))?;
        // A table it learned joins its rows on the playhead's threads.
        std::thread::sleep(Duration::from_micros(200));
    }
}

/// The convoys layer while it and the units draw: the trade routes join the order field then.
fn layered_convoys(side: &crate::playhead_host::MapSide) -> Option<&crate::layers::convoys::Convoys> {
    (side.draws("convoys") && side.draws("units")).then(|| side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::layers::convoys::Convoys>()))).flatten()
}

/// The playhead's map while its reader has not failed: the one the window takes everything from.
fn live_of(map: &Option<crate::playhead_host::MapSide>) -> Option<&crate::playhead_host::MapSide> {
    map.as_ref().filter(|side| side.failure().is_none())
}

/// Whether the window uploads today's unit field: one not uploaded yet, and never while the
/// playhead's map is open (its units layer installs the field).
fn today_field_due(live: bool, uploaded: bool) -> bool {
    !live && !uploaded
}

/// Lays the resource icons and occupation marks `side`'s layers hold at the head into `prepared`
/// where it holds another product, for viewing country `ours`; whether it laid any.
fn lay_in_layers(side: Option<&crate::playhead_host::MapSide>, prepared: &mut crate::document::PreparedOverlay, now: &crate::document::Hour<'_>, camera: Camera, pixels: [u32; 2], ui_scale: f32, ours: u32) -> bool {
    use crate::document::MadeBy;
    let Some(side) = side else { return false };
    let mut laid = false;
    if let Some(layer) = crate::layers::resource_icons::drawn(side)
        && prepared.resource_icons_by() != MadeBy::Layer(layer.version())
    {
        prepared.replace_resource_icons(now, camera, pixels, ui_scale, &layer.icons(), layer.version());
        laid = true;
    }
    if let Some(layer) = crate::layers::occupation_marks::drawn(side)
        && prepared.occupation_icons_by() != MadeBy::Layer(layer.version())
    {
        prepared.replace_occupation_icons(now, camera, pixels, ui_scale, ours, &layer.marks(), layer.version());
        laid = true;
    }
    laid
}

/// How the overlay's tables are made: the air missions deferred to the first layout in the strategic air mode (the one mode whose
/// layout reads them), made in that layout so a switch into the mode lays them out in its first
/// frame as today, and, in the game's map pose, the parts read per entity looked up by key as the
/// layout admits each. A free eye (`free`) admits up to the whole front at once, so its tables are
/// made whole on the worker, as today, and no frame looks thousands up. The same for every mode,
/// so a mode switch asks for no new tables; a change of pose asks for them.
fn overlay_need(_view: &CounterView, free: bool) -> viewer_counters::map::counters::Need {
    viewer_counters::map::counters::Need { air_missions: false, by_key: !free }
}

/// A deferred save may build unused warnings, but an unsavable frame never needs them.
fn builds_warnings(diagnostics: bool, capture: Option<&Capture>) -> bool {
    diagnostics || capture.is_some_and(|capture| capture.remaining == 0)
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn window_dumps_discard_live_input_before_egui_sees_it() {
        use eframe::App;
        for window_dump in [false, true] {
            let mut viewer = super::Viewer::create(
                super::Config::default(),
                super::Options {
                    window_dump,
                    ..Default::default()
                },
                None,
                std::sync::Arc::new(std::sync::Mutex::new(None)),
                None,
                None,
            );
            let ctx = egui::Context::default();
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0));
            for time in [0.0, 1.0] {
                let mut input = egui::RawInput {
                    screen_rect: Some(rect),
                    time: Some(time),
                    focused: true,
                    events: vec![
                        egui::Event::PointerMoved(egui::pos2(639.0, 479.0)),
                        egui::Event::PointerButton {
                            pos: egui::pos2(639.0, 479.0),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::CTRL,
                        },
                        egui::Event::Key {
                            key: egui::Key::F3,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::CTRL,
                        },
                        egui::Event::Text("live input".into()),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, 120.0),
                            phase: egui::TouchPhase::Move,
                            modifiers: egui::Modifiers::CTRL,
                        },
                    ],
                    hovered_files: vec![egui::HoveredFile::default()],
                    ..Default::default()
                };
                let events = input.events.clone();
                viewer.raw_input_hook(&ctx, &mut input);
                assert_eq!(input.screen_rect, Some(rect));
                assert_eq!(input.time, Some(time));
                assert_eq!(input.focused, !window_dump);
                assert_eq!(input.hovered_files.is_empty(), window_dump);
                if window_dump {
                    assert_eq!(input.events, vec![egui::Event::PointerGone]);
                } else {
                    assert_eq!(input.events, events);
                }
                let mut output = ctx.run_ui(input, |ui| {
                    ui.input(|input| {
                        assert_eq!(input.pointer.hover_pos().is_none(), window_dump);
                        assert_eq!(input.pointer.any_down(), !window_dump);
                        assert_eq!(input.key_down(egui::Key::F3), !window_dump);
                    });
                });
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn profile_pointer_overrides_hover_only_for_profile_captures() {
        use super::profile_hover_point;
        let real = Some([30.0, 60.0]);
        let synthetic = Some([120.0, 240.0]);
        assert_eq!(
            profile_hover_point(real, synthetic, true, [640, 480], 1.5),
            Some([80.0, 160.0])
        );
        assert_eq!(
            profile_hover_point(real, synthetic, false, [640, 480], 1.5),
            Some([20.0, 40.0])
        );
        assert_eq!(
            profile_hover_point(real, None, true, [640, 480], 1.5),
            Some([20.0, 40.0])
        );
        assert_eq!(
            profile_hover_point(None, synthetic, false, [640, 480], 1.5),
            None
        );
        assert_eq!(
            profile_hover_point(None, synthetic, true, [640, 480], 1.5),
            Some([80.0, 160.0])
        );
        for point in [[640.0, 20.0], [20.0, 480.0], [-1.0, 20.0], [f32::NAN, 20.0]] {
            assert_eq!(
                profile_hover_point(real, Some(point), true, [640, 480], 1.0),
                None
            );
        }
    }

    use super::*;

    #[test]
    fn a_capture_builds_warnings_only_on_the_frames_that_can_save_them() {
        let capture = |remaining, frames| Capture {
            path: "capture.png".into(),
            remaining,
            pinned: None,
            exit: true,
            time: None,
            step: 0.25,
            frames,
            index: 0,
            paced: false,
        };
        // Ordinary viewing builds none; diagnostics mode builds them on every frame.
        assert!(!builds_warnings(false, None));
        for remaining in [0, 1, 3_321] {
            assert!(builds_warnings(true, Some(&capture(remaining, 1))));
        }
        assert!(builds_warnings(true, None));
        // Mirror save_capture's countdown and sequence steps without rendering.
        for (remaining, frames) in [(3_321, 1), (1, 1), (0, 1), (2, 3)] {
            let mut capture = capture(remaining, frames);
            let mut built = Vec::new();
            loop {
                built.push(builds_warnings(false, Some(&capture)));
                if capture.remaining > 0 {
                    capture.remaining -= 1;
                    continue;
                }
                if capture.frames == 1 {
                    break;
                }
                capture.frames -= 1;
                capture.index += 1;
                capture.remaining = 0;
            }
            let saved: Vec<_> = (0..built.len())
                .map(|frame| frame >= remaining as usize)
                .collect();
            assert_eq!(built, saved, "remaining {remaining}, frames {frames}");
            assert_eq!(
                built.iter().filter(|&&built| built).count(),
                frames as usize,
                "one build per saved frame"
            );
        }
    }

    #[test]
    fn occupation_controls_keep_independent_hits_and_follow_free_icon_visibility() {
        use viewer_foundation::map::{
            free_icons::{IconFamily, Looks, Present},
            native_gui::HitGeometry,
            sprites::layer,
        };
        use viewer_world::map::occupation_icons::{Hit, Hover};
        let parent = layer(5, 0, 0);
        let hits = [
            Hit {
                geometry: HitGeometry::test_rectangle([0, 0], [20, 10], [100, 100]),
                layer: parent,
                hover: Hover::Resistance { state: 7 },
            },
            Hit {
                geometry: HitGeometry::test_rectangle([10, 12], [10, 10], [100, 100]),
                layer: parent,
                hover: Hover::Modifier {
                    state: 7,
                    token: "synthetic_badge".into(),
                },
            },
        ];
        let mut report = CounterReport::default();
        assert_eq!(
            top_map_hit([5., 5.], &report, &[], &hits, None, None),
            Some(MapHit::Occupation(0))
        );
        assert_eq!(
            top_map_hit([15., 17.], &report, &[], &hits, None, None),
            Some(MapHit::Occupation(1))
        );
        // The gap between the text and badge belongs to the map.
        assert_eq!(
            top_map_hit([5., 17.], &report, &[], &hits, None, None),
            None
        );
        let mut looks = Looks::default();
        let present = |alpha| Present {
            pivot: [10., 10.],
            scale: 2.,
            alpha,
            rank: 4,
        };
        looks.show(IconFamily::Occupation, parent, present(1.));
        assert_eq!(
            top_map_hit([20., 24.], &report, &[], &hits, Some(&looks), None),
            Some(MapHit::Occupation(1))
        );
        let absent = Looks::default();
        assert_eq!(
            top_map_hit([20., 24.], &report, &[], &hits, Some(&absent), None),
            None
        );
        let mut faint = Looks::default();
        faint.show(IconFamily::Occupation, parent, present(0.2));
        assert_eq!(
            top_map_hit([20., 24.], &report, &[], &hits, Some(&faint), None),
            None
        );
        let base_layer = layer(20, 0, 0);
        report
            .base_hits
            .push(viewer_foundation::map::counter_hovers::unit_hits::Blocker {
                rect: [0., 0., 40., 40.],
                layer: base_layer,
            });
        looks.show(
            IconFamily::Bases,
            base_layer,
            Present {
                pivot: [10., 10.],
                scale: 1.,
                alpha: 1.,
                rank: 3,
            },
        );
        // Nearer presented occupation geometry wins over a higher-priority distant base.
        assert_eq!(
            top_map_hit([20., 24.], &report, &[], &hits, Some(&looks), None),
            Some(MapHit::Occupation(1))
        );
        looks.show(
            IconFamily::Occupation,
            parent,
            Present {
                rank: 2,
                ..present(1.)
            },
        );
        assert_eq!(
            top_map_hit([20., 24.], &report, &[], &hits, Some(&looks), None),
            Some(MapHit::Base(0))
        );
    }

    #[test]
    fn naval_regional_controls_own_hover_footprints_and_preserve_unsupported_blocking() {
        use viewer_counters::map::counters::{
            CounterReport, NavalRegionalControl, NavalRegionalHover,
        };
        let mut report = CounterReport::default();
        report.naval_region_hovers.push(NavalRegionalHover {
            region: 7,
            control: NavalRegionalControl::Mines,
            rect: [0., 0., 10., 10.],
            layer: 8,
            hit: None,
            supported: true,
            text: None,
        });
        assert_eq!(
            top_map_hit([5., 5.], &report, &[], &[], None, None),
            Some(MapHit::NavalRegionHover(0))
        );
        assert_eq!(top_map_hit([15., 5.], &report, &[], &[], None, None), None);
        report.naval_region_hovers[0].supported = false;
        assert_eq!(
            top_map_hit([5., 5.], &report, &[], &[], None, None),
            Some(MapHit::Blocked)
        );
    }

    #[test]
    fn naval_mission_buttons_intercept_at_their_drawn_layer() {
        use viewer_counters::map::counters::{CounterReport, NavalMissionHit};
        use viewer_foundation::map::counter_hovers::unit_hits::Blocker;
        let mut report = CounterReport::default();
        report.naval_mission_hits.push(NavalMissionHit {
            region: 7,
            mission: 2,
            rect: [0., 0., 10., 10.],
            layer: 8,
            text: None,
        });
        assert_eq!(
            top_map_hit([5., 5.], &report, &[], &[], None, None),
            Some(MapHit::NavalMission(0))
        );
        assert_eq!(top_map_hit([11., 5.], &report, &[], &[], None, None), None);
        report.base_hits.push(Blocker {
            rect: [0., 0., 10., 10.],
            layer: 9,
        });
        assert_eq!(
            top_map_hit([5., 5.], &report, &[], &[], None, None),
            Some(MapHit::Base(0))
        );
    }

    #[test]
    fn prepared_naval_mission_hover_keeps_exact_text_and_missions_footprint() {
        use viewer_counters::map::counters::NavalMissionHit;
        use viewer_foundation::map::free_icons::{IconFamily, Looks, Present};
        let mut report = CounterReport::default();
        let text = Arc::new(Tooltip {
            title: "Fleet\n§YStatus§!".into(),
            body: "\u{15}active\nprepared mission detail".into(),
            refused: BTreeMap::from([("missing source".into(), 2)]),
        });
        let rect = [10., 10., 20., 20.];
        // A status child has a hover and blocks the province without selecting a mission.
        report.naval_mission_hits.push(NavalMissionHit {
            region: 7,
            mission: 0,
            rect,
            layer: 8,
            text: Some(Arc::clone(&text)),
        });
        let mut looks = Looks::default();
        looks.show(
            IconFamily::Missions,
            8,
            Present {
                pivot: [0., 0.],
                scale: 2.,
                alpha: 1.,
                rank: 0,
            },
        );
        let target = top_map_hit([50., 50.], &report, &[], &[], Some(&looks), None);
        assert_eq!(target, Some(MapHit::NavalMission(0)));
        let (shown, footprint) = prepared_naval_tooltip(&report, target).unwrap();
        assert!(Arc::ptr_eq(&shown, &text));
        assert_eq!(footprint, rect);
        assert_eq!(
            top_map_hit([65., 50.], &report, &[], &[], Some(&looks), None),
            None
        );
        report.naval_mission_hits[0].text = None;
        assert!(prepared_naval_tooltip(&report, target).is_none());
        assert_eq!(
            top_map_hit([50., 50.], &report, &[], &[], Some(&looks), None),
            target
        );
    }

    #[test]
    fn prepared_naval_convoy_hover_uses_its_own_text_and_footprint() {
        use viewer_counters::map::counters::{NavalRegionalControl, NavalRegionalHover};
        let text = Arc::new(Tooltip {
            title: "Convoys".into(),
            body: "recorded client operands".into(),
            refused: BTreeMap::new(),
        });
        let rect = [4., 6., 10., 12.];
        let mut report = CounterReport::default();
        report.naval_region_hovers.push(NavalRegionalHover {
            region: 7,
            control: NavalRegionalControl::Convoy(19),
            rect,
            layer: 8,
            hit: None,
            supported: true,
            text: Some(Arc::clone(&text)),
        });
        let target = top_map_hit([5., 7.], &report, &[], &[], None, None);
        assert_eq!(target, Some(MapHit::NavalRegionHover(0)));
        let (shown, footprint) = prepared_naval_tooltip(&report, target).unwrap();
        assert!(Arc::ptr_eq(&shown, &text));
        assert_eq!(footprint, rect);
        report.naval_region_hovers[0].supported = false;
        let target = top_map_hit([5., 7.], &report, &[], &[], None, None);
        assert_eq!(target, Some(MapHit::Blocked));
        assert!(prepared_naval_tooltip(&report, target).is_none());
    }

    #[test]
    fn regional_naval_grid_units_use_the_missions_transform() {
        use viewer_foundation::map::{
            counter_hovers::unit_hits::{Hit, Kind},
            free_icons::{IconFamily, Looks, Present},
        };
        let mut report = CounterReport::default();
        report.unit_hits.push(Hit {
            ours: true,
            port: true,
            rect: [10., 10., 20., 20.],
            layer: 8,
            kind: Kind::TaskForce,
            members: Arc::from([19]),
            plate: None,
        });
        report.regional_naval_unit_hits.insert(0);
        let mut looks = Looks::default();
        looks.show(
            IconFamily::Missions,
            8,
            Present {
                pivot: [0., 0.],
                scale: 2.,
                alpha: 1.,
                rank: 0,
            },
        );
        assert_eq!(
            top_map_hit([50., 50.], &report, &[], &[], Some(&looks), None),
            Some(MapHit::Unit(0))
        );
        // An ordinary port item still belongs to the base anchor.
        report.regional_naval_unit_hits.clear();
        assert_eq!(
            top_map_hit([50., 50.], &report, &[], &[], Some(&looks), None),
            None
        );
    }

    #[test]
    fn radar_toggle_hover_uses_the_top_base_and_never_a_covered_station() {
        use viewer_foundation::map::counter_hovers::unit_hits::Blocker;
        let mut report = viewer_counters::map::counters::CounterReport::default();
        report.base_hits.push(Blocker {
            rect: [0., 0., 20., 20.],
            layer: 1,
        });
        report.radar_hits.insert(0, 7);
        assert_eq!(
            radar_province(
                top_map_hit([5., 5.], &report, &[], &[], None, None),
                &report
            ),
            Some(7)
        );
        report.base_hits.push(Blocker {
            rect: [0., 0., 20., 20.],
            layer: 2,
        });
        assert_eq!(
            radar_province(
                top_map_hit([5., 5.], &report, &[], &[], None, None),
                &report
            ),
            None
        );
        report.radar_hits.insert(1, 8);
        assert_eq!(
            radar_province(
                top_map_hit([5., 5.], &report, &[], &[], None, None),
                &report
            ),
            Some(8)
        );
    }

    #[test]
    fn task_force_children_outside_the_plate_keep_its_priority_and_block_unknown_hit_forms() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use {
            viewer_foundation::map::counter_hovers::task_force::Control,
            viewer_foundation::map::counter_hovers::task_force::Hover,
            viewer_foundation::map::counter_hovers::unit_hits::Blocker,
            viewer_foundation::map::counter_hovers::unit_hits::Hit,
            viewer_foundation::map::counter_hovers::unit_hits::Kind,
            viewer_foundation::map::sprites::layer,
        };
        let mut report = viewer_counters::map::counters::CounterReport::default();
        report.unit_hits.push(Hit {
            ours: true,
            port: false,
            rect: [20., 20., 66., 33.],
            kind: Kind::TaskForce,
            members: Arc::from([7]),
            layer: layer(10, 0, 0),
            plate: None,
        });
        // Native carrier/order children can begin left of an unclipped root.
        report.task_force_hovers.push(Hover {
            control: Control::Carrier,
            unit_hit: 0,
            rect: [4., 20., 16., 16.],
            hit: None,
            authored: None,
            supported: true,
            ours: true,
            coarse: false,
            hidden: false,
            members: Arc::from([7]),
        });
        let point = [8., 24.];
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Unit(0))
        );
        report.base_hits.push(Blocker {
            rect: [0., 0., 40., 40.],
            layer: layer(11, 0, 0),
        });
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Base(0))
        );
        report.base_hits.clear();
        report.task_force_hovers[0].supported = false;
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Blocked)
        );
        // A stale/mismatched ordinal cannot dispatch to another class's plate.
        report.unit_hits[0].kind = Kind::Division;
        assert_eq!(top_map_hit(point, &report, &[], &[], None, None), None);

        // An opaque unbound port child stops the native walk; its authored
        // tooltip can still be shown without selecting the plate underneath.
        use viewer_foundation::map::counter_hovers::task_force_port as port;
        report.unit_hits[0].kind = Kind::TaskForce;
        report.unit_hits[0].port = true;
        report.task_force_port_hovers.push(port::Hover {
            control: port::Control::Unbound,
            unit_hit: 0,
            rect: [4., 20., 16., 16.],
            hit: None,
            authored: None,
            supported: true,
            members: Arc::from([7]),
            metadata: port::Metadata {
                country: 1,
                visibility: 250,
                enemy: Ok(false),
                hold_fire: Some(false),
                idle: Some(false),
                no_regions: Some(false),
                pride: Some(false),
            },
        });
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Blocked)
        );
        let gui = installed_content_core::gui::native::read(
            br#"guiTypes = { iconType = {
            name = unbound pdx_tooltip = TITLE
        } }"#,
        );
        report.task_force_port_hovers[0].authored =
            viewer_foundation::map::counter_hovers::gui_authored::Authored::read(
                gui.template("unbound").unwrap(),
                false,
            );
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::PortTooltip(0))
        );
    }

    #[test]
    fn todays_unit_field_is_never_uploaded_while_the_playheads_map_is_open() {
        assert!(super::today_field_due(false, false), "no open map: today's field until it is uploaded");
        assert!(!super::today_field_due(false, true));
        assert!(!super::today_field_due(true, false), "the open map's units layer installs the field");
    }

    #[test]
    fn map_hits_follow_signed_layers_compose_order_and_actual_combat_attachment() {
        use {
            viewer_foundation::map::counter_hovers::unit_hits::Blocker,
            viewer_foundation::map::counter_hovers::unit_hits::Hit,
            viewer_foundation::map::counter_hovers::unit_hits::Kind,
            viewer_foundation::map::sprites::layer,
        };
        let rect = [0.0, 0.0, 20.0, 20.0];
        let point = [5.0, 5.0];
        let mut report = viewer_counters::map::counters::CounterReport::default();
        report.unit_hits.push(Hit {
            ours: true,
            port: false,
            rect,
            kind: Kind::Division,
            members: Arc::from([7]),
            layer: layer(-3, 0, 0),
            plate: None,
        });
        report.base_hits.push(Blocker {
            rect,
            layer: layer(-3, 0, 0),
        });
        report
            .faction_pin_hovers
            .push(viewer_counters::map::counters::faction_pings::PinHover {
                rect,
                regions: Arc::from([1]),
                layer: layer(-3, 0, 0),
            });
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Unit(0))
        );
        report.base_hits[0].layer = layer(4, 0, 0);
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Base(0))
        );
        report.faction_pin_hovers[0].layer = layer(5, 0, 0);
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Pin(0))
        );
        report.faction_pin_hovers.clear();
        let raid = viewer_raids::map::raids::tooltips::Hit {
            origin: [0.0, 0.0],
            size: [20.0, 20.0],
            key: viewer_raids::map::raids::tooltips::Key::Raid {
                id: 1,
                reference_type: 4,
                incoming: false,
            },
            layer: layer(-2, 0, 0),
            supported: true,
        };
        assert_eq!(
            top_map_hit(point, &report, std::slice::from_ref(&raid), &[], None, None),
            Some(MapHit::Base(0))
        );
        report.base_hits.clear();
        assert_eq!(
            top_map_hit(point, &report, std::slice::from_ref(&raid), &[], None, None),
            Some(MapHit::Raid)
        );
        let mut same_layer_raid = raid.clone();
        same_layer_raid.layer = report.unit_hits[0].layer;
        assert_eq!(
            top_map_hit(
                point,
                &report,
                std::slice::from_ref(&same_layer_raid),
                &[],
                None,
                None
            ),
            Some(MapHit::Unit(0))
        );
        report.unit_hits[0].port = true;
        assert_eq!(
            top_map_hit(
                point,
                &report,
                std::slice::from_ref(&same_layer_raid),
                &[],
                None,
                None
            ),
            Some(MapHit::Raid)
        );
        report.unit_hits[0].port = false;
        report.combat_hits.push(CombatHit {
            selection: Selection::Land(1),
            rect,
            layer: layer(-1, 0, 0),
            order: 0,
            supported: true,
        });
        assert_eq!(
            top_map_hit(point, &report, std::slice::from_ref(&raid), &[], None, None),
            Some(MapHit::Combat(0))
        );
        report.combat_hits[0].layer = layer(-4, 0, 0);
        assert_eq!(
            top_map_hit(point, &report, std::slice::from_ref(&raid), &[], None, None),
            Some(MapHit::Raid)
        );
        report.combat_hits[0] = CombatHit {
            selection: Selection::Naval(1),
            rect,
            layer: layer(5, 0, 0),
            order: 1,
            supported: true,
        };
        let mut observer = report.combat_hits[0].clone();
        observer.order = 2;
        report.naval_marker_hovers.push(observer);
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::NavalObserver(0))
        );
        report.combat_hits[0].order = 2;
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Combat(0))
        );
        report.combat_hits[0].supported = false;
        report.naval_marker_hovers[0].supported = false;
        report
            .faction_pin_hovers
            .push(viewer_counters::map::counters::faction_pings::PinHover {
                rect,
                regions: Arc::from([1]),
                layer: layer(4, 0, 0),
            });
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Blocked)
        );
        report.combat_hits[0].supported = true;
        report.naval_marker_hovers[0].supported = true;
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Combat(0))
        );
        report.faction_pin_hovers.clear();
        report.combat_hits.clear();
        report.naval_marker_hovers.clear();
        let mut refused_raid = raid.clone();
        refused_raid.supported = false;
        assert_eq!(
            top_map_hit(
                point,
                &report,
                std::slice::from_ref(&refused_raid),
                &[],
                None,
                None
            ),
            Some(MapHit::Blocked)
        );
        assert_eq!(
            top_map_hit(
                [20.0, 5.0],
                &report,
                std::slice::from_ref(&raid),
                &[],
                None,
                None
            ),
            None
        );
        // An intel entry takes the point at its higher layer. A missing CPU mask
        // accepts the child's rectangular hit, including its tooltip.
        let viewport = [2048, 1152];
        let mut intel = viewer_raids::map::intel_icons::IntelHit {
            shapes: vec![
                viewer_foundation::map::native_gui::HitGeometry::test_rectangle(
                    [0, 0],
                    [20, 20],
                    viewport,
                ),
            ],
            layer: layer(6, 0, 0),
            tooltip: Arc::new(Tooltip::default()),
        };
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, Some(&intel)),
            Some(MapHit::Intel)
        );
        intel.shapes[0].form = viewer_foundation::map::native_gui::Hit::Alpha;
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, Some(&intel)),
            Some(MapHit::Intel)
        );
        assert_eq!(
            top_map_hit([30.0, 30.0], &report, &[], &[], None, Some(&intel)),
            None
        );
    }

    #[test]
    fn a_free_view_hits_the_nearest_shown_icon_in_its_own_layout() {
        use {
            viewer_foundation::map::counter_hovers::unit_hits::{Blocker, Hit, Kind},
            viewer_foundation::map::free_icons::{IconFamily, Looks, Present},
            viewer_foundation::map::sprites::layer,
        };
        let mut report = viewer_counters::map::counters::CounterReport::default();
        // A unit counter laid out over [0, 20) and a base over [10, 30), both at y [0, 20).
        report.unit_hits.push(Hit {
            ours: true,
            port: false,
            rect: [0.0, 0.0, 20.0, 20.0],
            kind: Kind::Division,
            members: Arc::from([7]),
            layer: layer(10, 0, 0),
            plate: None,
        });
        report.base_hits.push(Blocker {
            rect: [10.0, 0.0, 20.0, 20.0],
            layer: layer(3, 0, 0),
        });
        let point = [15.0, 5.0];
        // As laid out, the counter's higher priority wins.
        assert_eq!(
            top_map_hit(point, &report, &[], &[], None, None),
            Some(MapHit::Unit(0))
        );
        // A free eye with the base nearer: the base wins whatever its priority.
        let at = |rank, scale, alpha| Present {
            pivot: [10.0, 10.0],
            scale,
            alpha,
            rank,
        };
        let mut looks = Looks::default();
        looks.show(IconFamily::Columns, layer(10, 0, 0), at(0, 1.0, 1.0));
        looks.show(IconFamily::Bases, layer(3, 0, 0), at(1, 1.0, 1.0));
        assert_eq!(
            top_map_hit(point, &report, &[], &[], Some(&looks), None),
            Some(MapHit::Base(0))
        );
        // Shrunk to half about its anchor, the base no longer reaches the pointer; the counter does.
        let mut looks = Looks::default();
        looks.show(IconFamily::Columns, layer(10, 0, 0), at(0, 1.0, 1.0));
        looks.show(IconFamily::Bases, layer(3, 0, 0), at(1, 0.5, 1.0));
        assert_eq!(
            top_map_hit([18.0, 3.0], &report, &[], &[], Some(&looks), None),
            Some(MapHit::Unit(0))
        );
        // A faint base and an undrawn counter take no hits.
        let mut looks = Looks::default();
        looks.show(IconFamily::Bases, layer(3, 0, 0), at(1, 1.0, 0.2));
        assert_eq!(
            top_map_hit(point, &report, &[], &[], Some(&looks), None),
            None
        );
        // The counter's layer key shown for another family leaves the counter itself unshown.
        let mut looks = Looks::default();
        looks.show(IconFamily::Combats, layer(10, 0, 0), at(0, 1.0, 1.0));
        assert_eq!(
            top_map_hit(point, &report, &[], &[], Some(&looks), None),
            None
        );
    }

    #[test]
    fn carrier_wing_controls_take_the_columns_transform_and_win_over_their_parent() {
        use viewer_foundation::map::{
            counter_hovers::unit_hits::{Blocker, Hit, Kind},
            free_icons::{IconFamily, Looks, Present},
        };
        let mut report = viewer_counters::map::counters::CounterReport::default();
        report.base_hits.push(Blocker {
            rect: [10., 10., 20., 20.],
            layer: 7,
        });
        report.air_grid_hits.insert(
            0,
            viewer_counters::map::counters::AirGridHover {
                family: IconFamily::Columns,
                members: Some(Arc::new(vec![19])),
                text: Arc::default(),
            },
        );
        report.unit_hits.push(Hit {
            ours: true,
            port: false,
            rect: [0., 0., 40., 40.],
            kind: Kind::TaskForce,
            members: Arc::from([8]),
            layer: 7,
            plate: None,
        });
        let mut looks = Looks::default();
        looks.show(
            IconFamily::Columns,
            7,
            Present {
                pivot: [0., 0.],
                scale: 2.,
                alpha: 1.,
                rank: 0,
            },
        );
        assert_eq!(
            top_map_hit([50., 50.], &report, &[], &[], Some(&looks), None),
            Some(MapHit::Base(0))
        );
        report.air_grid_hits.get_mut(&0).unwrap().family = IconFamily::Bases;
        assert_eq!(
            top_map_hit([50., 50.], &report, &[], &[], Some(&looks), None),
            Some(MapHit::Unit(0))
        );
    }

    #[test]
    fn overlapping_land_and_naval_units_keep_separate_tooltip_namespaces() {
        use {
            viewer_foundation::map::counter_hovers::land::CounterControl,
            viewer_foundation::map::counter_hovers::land::CounterHover,
            viewer_foundation::map::counter_hovers::unit_hits::Hit,
            viewer_foundation::map::counter_hovers::unit_hits::Kind,
        };
        let members = Arc::from([7]);
        let hover = CounterHover {
            control: CounterControl::Normal,
            rect: [0.0, 0.0, 20.0, 20.0],
            authored: None,
            remaining: None,
            ours: true,
            coarse: false,
            hidden: false,
            railway_gun: false,
            members: Arc::clone(&members),
            timing: Arc::from([]),
        };
        let mut unit = Hit {
            ours: true,
            port: false,
            rect: hover.rect,
            kind: Kind::TaskForce,
            members,
            layer: 0,
            plate: None,
        };
        assert!(!land_hover_matches_unit(&hover, &unit));
        unit.kind = Kind::Division;
        assert!(land_hover_matches_unit(&hover, &unit));
        unit.kind = Kind::RailwayGun;
        assert!(!land_hover_matches_unit(&hover, &unit));
        let gun_hover = CounterHover {
            railway_gun: true,
            ..hover
        };
        assert!(land_hover_matches_unit(&gun_hover, &unit));
    }

    /// A prebuilt or window overlay without the index is laid out again once
    /// the index covers its hour, while paused or while advancing through
    /// prebuilt hours; one laid out with it, or one the index does not yet
    /// cover, is kept, so unchanged frames do not rebuild.
    #[test]
    fn overlays_relayout_once_when_the_engaged_index_arrives() {
        let covered_to =
            |last: i32| move |branch: replay_data::BranchId, hour: i32| branch == 1 && hour <= last;
        assert!(!engaged_relayout(None, covered_to(10)));
        // Paused at hour 5: not yet covered, then covered.
        assert!(!engaged_relayout(Some((false, 1, 5)), covered_to(4)));
        assert!(engaged_relayout(Some((false, 1, 5)), covered_to(5)));
        // After the relayout the overlay carries the index.
        assert!(!engaged_relayout(Some((true, 1, 5)), covered_to(5)));
        // Advancing through prebuilt hours: each prebuilt without the index is
        // redone once the index reaches it, one built with it is not.
        for hour in 6..9 {
            assert!(engaged_relayout(Some((false, 1, hour)), covered_to(8)));
            assert!(!engaged_relayout(Some((true, 1, hour)), covered_to(8)));
        }
        // Another branch is not covered.
        assert!(!engaged_relayout(Some((false, 2, 5)), covered_to(8)));
    }

    /// A built hour of branch 1 standing for the builders' delivery: a
    /// recorded gap, as what the lane holds is all this reads.
    struct Delivery {
        epoch: u64,
        hour: i32,
        current: crate::playback::Current,
    }

    impl Delivered for Delivery {
        fn epoch(&self) -> u64 {
            self.epoch
        }

        fn at(&self) -> (replay_data::BranchId, i32) {
            (1, self.hour)
        }

        fn current(&self) -> &crate::playback::Current {
            &self.current
        }
    }

    fn delivery(epoch: u64, hour: i32) -> Delivery {
        Delivery {
            epoch,
            hour,
            current: crate::playback::Current::Gap {
                branch: 1,
                hour,
                reason: String::new(),
            },
        }
    }

    #[test]
    fn a_stale_picture_keeps_drawing_without_claiming_the_new_epoch_is_ready() {
        let mut viewer = Viewer::create(
            Config::default(),
            Options::default(),
            None,
            Arc::new(Mutex::new(None)),
            None,
            None,
        );
        assert!(!viewer.picture_ready());
        let target = Playhead {
            branch: 1,
            hour: 10,
            fraction: 0.0,
        };
        viewer.rendered = Some(target);
        viewer.shown = Some(ShownHour {
            epoch: 0,
            branch: 1,
            hour: 10,
            error: None,
            political_warnings: Vec::new(),
            plan_warnings: Vec::new(),
            building_warnings: Vec::new(),
            model_warnings: Vec::new(),
            political_build: Duration::ZERO,
            limitations: Default::default(),
            snow: Default::default(),
            header: Default::default(),
            borders: None,
        });
        assert!(viewer.picture_ready());
        assert!(viewer.hour_ready(1, 10));
        assert!(viewer.presentation_ready(target, false));
        viewer.epoch = 1;
        assert!(viewer.picture_ready());
        assert!(!viewer.hour_ready(1, 10));
        assert!(!viewer.presentation_ready(target, false));
        viewer.shown.as_mut().unwrap().epoch = 1;
        assert!(viewer.picture_ready());
        assert!(viewer.hour_ready(1, 10));
        assert!(viewer.presentation_ready(target, false));
        viewer.animation_seek = true;
        assert!(viewer.picture_ready());
        assert!(!viewer.presentation_ready(target, false));
        viewer.shown.as_mut().unwrap().error = Some("unavailable".into());
        assert!(!viewer.picture_ready());
    }

    /// A map that shows no picture, here for an hour its recording lacks, stops what it sounded,
    /// and its next presented frame starts its sounds afresh.
    #[test]
    fn a_map_without_a_picture_stops_sounding() {
        // No device opens; the owner still claims the audio.
        viewer_audio::System::silent();
        let mut viewer = Viewer::create(
            Config::default(),
            Options::default(),
            None,
            Arc::new(Mutex::new(None)),
            None,
            None,
        );
        viewer.dashboard_map = true;
        viewer.drew_frame = true;
        let mut owner = viewer_audio::System::shared().owner();
        owner.claim();
        viewer.audio = Some(owner);
        let bounds = dashboard::protocol::ViewBounds {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let target = dashboard::protocol::DashboardMapView {
            options: Default::default(),
            id: "widget".into(),
            source_id: "source".into(),
            path: "fixture".into(),
            revision: "revision".into(),
            branch_id: "1".into(),
            playing: false,
            hours_per_second: 1.0,
            hour: 0,
            fraction: 0.,
            sequence: 1,
            seek_sequence: None,
            bounds,
            clip: bounds,
            visible: true,
        };
        let frame = eframe::Frame::_new_kittest();
        let mut output = egui::Context::default()
            .run_ui(Default::default(), |ui| viewer.map_ui(ui, &frame, &target));
        output.textures_delta.clear();
        assert!(viewer.error.is_some());
        assert!(viewer.drew_frame());
        assert!(!viewer.audio.as_ref().unwrap().owns());
        assert!(viewer.audio_reset);
        viewer.reset_drawn_frame();
        let mut output = egui::Context::default()
            .run_ui(Default::default(), |ui| viewer.map_ui(ui, &frame, &target));
        output.textures_delta.clear();
        assert!(!viewer.drew_frame());
    }

    /// A capture that waits on the builders for its hour feeds every hour it
    /// receives to the raid lane before taking it, so the lane holds the
    /// capture's hour and can cycle it: a cold `--window-dump` does not wait
    /// forever for a cycle the lane never got (the lane's side:
    /// `raid_lane::tests::a_cold_capture_cycles_its_hour_once_its_frame_is_fed`).
    #[test]
    fn a_waiting_capture_feeds_every_hour_it_receives_to_the_raid_lane() {
        let lane = crate::raid_lane::RaidLane::default();
        let mut arrivals = [
            delivery(1, 9),
            delivery(2, 11),
            delivery(2, 10),
            delivery(2, 12),
        ]
        .into_iter();
        let mut ready = BTreeMap::new();
        let mut stale = Vec::new();
        let built = wait_for_hour(
            &lane,
            ((1, 10), 2),
            || {
                arrivals
                    .next()
                    .ok_or_else(|| std::io::Error::other("the builders stopped"))
            },
            &mut ready,
            |built| stale.push(built.at()),
        )
        .unwrap();
        assert_eq!(built.at(), (1, 10));
        assert_eq!(ready.keys().copied().collect::<Vec<_>>(), [(1, 11)]);
        assert_eq!(stale, [(1, 9)]);
        for hour in 9..=11 {
            assert!(lane.has_frame(1, hour), "hour {hour} was fed");
        }
        assert!(!lane.has_frame(1, 12), "not received yet");
    }

    /// The raid cycle resets exactly when the trains and the weather do:
    /// they all take the continuity `apply_hour` computes for the hour it
    /// puts on screen. Seeks (same-hour ones and forward seeks over built
    /// hours too), a step back, a branch change, a new recording, a recorded
    /// gap and a new resolved viewing country reset them; playback, one hour
    /// forward and a playback skip carry them.
    #[test]
    fn the_raid_cycle_resets_exactly_with_the_trains_and_weather() {
        use viewer_foundation::map::presentation::{Continuity, Target, continuity};
        let gap = replay_frame::branches::Gap {
            branch: 1,
            first_hour: 45,
            last_hour: 49,
            reason: "missing".into(),
        };
        let gaps = std::slice::from_ref(&gap);
        let on_screen = Some((1, 20, Some("GER")));
        // Host seeks (a new seek counter, or none) raise the seek flag; the
        // playback clock does not. Dashboard targets tell seeks by their
        // clock's seek counter.
        let clock = Target {
            branch: 1,
            hour: 20,
            seek_sequence: Some(3),
        };
        let dashboard_skip = Target { hour: 26, ..clock }.seeks_from(clock);
        let dashboard_seek = Target {
            hour: 26,
            seek_sequence: Some(4),
            ..clock
        }
        .seeks_from(clock);
        let cases = [
            (
                "playback",
                on_screen,
                (1, 21),
                "GER",
                false,
                Continuity::Continue,
            ),
            ("+1", on_screen, (1, 21), "GER", false, Continuity::Continue),
            (
                "playback skip",
                on_screen,
                (1, 26),
                "GER",
                false,
                Continuity::Continue,
            ),
            (
                "dashboard skip",
                on_screen,
                (1, 26),
                "GER",
                dashboard_skip,
                Continuity::Continue,
            ),
            ("seek", on_screen, (1, 400), "GER", true, Continuity::Reset),
            (
                "same-hour seek",
                on_screen,
                (1, 20),
                "GER",
                true,
                Continuity::Reset,
            ),
            (
                "forward seek over built hours",
                on_screen,
                (1, 24),
                "GER",
                true,
                Continuity::Reset,
            ),
            (
                "dashboard seek",
                on_screen,
                (1, 26),
                "GER",
                dashboard_seek,
                Continuity::Reset,
            ),
            (
                "step back",
                on_screen,
                (1, 19),
                "GER",
                true,
                Continuity::Reset,
            ),
            ("branch", on_screen, (2, 20), "GER", true, Continuity::Reset),
            (
                "new recording",
                None,
                (1, 20),
                "GER",
                false,
                Continuity::Reset,
            ),
            (
                "recorded gap",
                Some((1, 44, Some("GER"))),
                (1, 50),
                "GER",
                false,
                Continuity::Reset,
            ),
            // The gap hour itself is shown as an error: nothing carries past it.
            (
                "after a gap hour",
                None,
                (1, 50),
                "GER",
                false,
                Continuity::Reset,
            ),
            (
                "country",
                on_screen,
                (1, 21),
                "SOV",
                false,
                Continuity::Reset,
            ),
        ];
        for (name, previous, next, viewing, sought, expected) in cases {
            let perspective = Choice::default();
            let shown = |at: (replay_data::BranchId, i32)| crate::playback::Current::Gap {
                branch: at.0,
                hour: at.1,
                reason: String::new(),
            };
            let lane = crate::raid_lane::RaidLane::default();
            // The lane presents the hour on screen before the move.
            if let Some((branch, hour, _)) = previous {
                let current = shown((branch, hour));
                present_raids(
                    &lane,
                    Continuity::Reset,
                    crate::raid_lane::Presented {
                        content: None,
                        perspective: &perspective,
                        at: (branch, hour),
                        playing: false,
                        current: &current,
                        viewing: None,
                    },
                );
            }
            let before = lane.generation();
            let presented = continuity(previous, (next.0, next.1, Some(viewing)), sought, gaps);
            assert_eq!(presented, expected, "{name}");
            let current = shown(next);
            present_raids(
                &lane,
                presented,
                crate::raid_lane::Presented {
                    content: None,
                    perspective: &perspective,
                    at: next,
                    playing: false,
                    current: &current,
                    viewing: None,
                },
            );
            assert!(lane.has_frame(next.0, next.1), "{name}: the hour's frame");
            let raids_reset = lane.generation() != before;
            // What the trains and the weather keep across the same move.
            let kept =
                previous.is_some_and(|(branch, hour, _)| presented.keeps((branch, hour), next));
            assert_eq!(raids_reset, !kept, "{name}");
            assert_eq!(raids_reset, expected == Continuity::Reset, "{name}");
        }
        // A forward seek over hours that are all built continues the
        // building history, which does not reset the raid cycle any more.
        let built = |hour: i32| Some(Some(hour));
        assert!(skipped_hours(Some((1, 20)), (1, 24), false, built).is_some());
    }

    #[test]
    fn playing_shows_the_newest_built_hour_until_the_playheads_own_arrives() {
        let mut ready = BTreeMap::from([((1, 11), true), ((1, 14), true), ((2, 12), true)]);
        assert_eq!(latest_built_key(&ready, (1, 15), |ok| *ok), Some((1, 14)));
        // A failed hour or one past the playhead does not stand in.
        ready.insert((1, 15), false);
        ready.insert((1, 16), true);
        assert_eq!(latest_built_key(&ready, (1, 15), |ok| *ok), Some((1, 14)));
        ready.insert((1, 15), true);
        assert_eq!(latest_built_key(&ready, (1, 15), |ok| *ok), Some((1, 15)));
        assert_eq!(latest_built_key(&ready, (3, 15), |ok| *ok), None);
        let target = Playhead {
            branch: 1,
            hour: 15,
            fraction: 0.5,
        };
        let shown = Playhead {
            hour: 14,
            fraction: 0.0,
            ..target
        };
        assert!(presentation_matches(shown, target, true));
        assert!(
            !presentation_matches(shown, target, false),
            "paused is exact"
        );
        assert!(!presentation_matches(
            Playhead { branch: 2, ..shown },
            target,
            true
        ));
        assert!(!presentation_matches(
            Playhead { hour: 16, ..shown },
            target,
            true
        ));
        assert!(presentation_matches(target, target, false));
        for fraction in [0.1, 1.0 / 3.0, 0.999_999] {
            let target = Playhead { fraction, ..target };
            let rendered = Playhead {
                fraction: f64::from(fraction as f32),
                ..target
            };
            assert!(presentation_matches(rendered, target, true));
            assert!(!presentation_matches(rendered, target, false));
        }
    }

    #[test]
    fn host_reanchors_keep_playback_forward_but_seeks_and_pauses_exact() {
        let current = Playhead {
            branch: 1,
            hour: 20,
            fraction: 0.75,
        };
        for target in [
            Playhead {
                hour: 19,
                ..current
            },
            Playhead {
                fraction: 0.25,
                ..current
            },
        ] {
            assert_eq!(anchor_clock(Some(current), target, true), current);
            assert_eq!(anchor_clock(Some(current), target, false), target);
        }
        let later = Playhead {
            hour: 21,
            fraction: 0.125,
            ..current
        };
        assert_eq!(anchor_clock(Some(current), later, true), later);
        let branch = Playhead {
            branch: 2,
            hour: 10,
            ..current
        };
        assert_eq!(anchor_clock(Some(current), branch, true), branch);
        assert_eq!(anchor_clock(None, current, true), current);
    }

    #[test]
    fn late_anchors_do_not_freeze_frames_and_controls_remain_exact() {
        let now = Instant::now();
        let mut viewer = Viewer::create(
            Config::default(),
            Options::default(),
            None,
            Arc::new(Mutex::new(None)),
            None,
            None,
        );
        let host = Playhead {
            branch: 1,
            hour: 100,
            fraction: 0.0,
        };
        viewer.last_step = now;
        viewer.anchor_playback(host, true, 4.0, true, now, 1_000);
        for frame in 1..=60 {
            let at = now + Duration::from_secs_f64(f64::from(frame) / 60.0);
            if frame % 3 == 0 {
                let delayed = Playhead {
                    fraction: 0.0,
                    hour: 100 + (frame - 3) / 15,
                    ..host
                };
                viewer.anchor_playback(delayed, true, 4.0, false, at, 1_000);
            }
            viewer.tick_playhead(at.duration_since(viewer.last_step).as_secs_f64(), 1_000);
            viewer.last_step = at;
            let head = viewer.playhead.unwrap();
            let position = f64::from(head.hour) + head.fraction;
            assert!((position - (100.0 + f64::from(frame) / 15.0)).abs() < 1e-7);
        }
        viewer.anchor_playback(host, false, 4.0, false, now + Duration::from_secs(1), 1_000);
        assert_eq!(viewer.playhead, Some(host));
        assert!(!viewer.playing);
        viewer.anchor_playback(host, true, 0.25, false, now + Duration::from_secs(1), 1_000);
        assert_eq!(viewer.playhead, Some(host));
        viewer.tick_playhead(0.5, 1_000);
        assert_eq!(viewer.playhead.unwrap().fraction, 0.125);
        viewer.anchor_playback(host, true, 0.25, true, now + Duration::from_secs(1), 1_000);
        assert_eq!(viewer.playhead, Some(host));
        assert!(!tick_clock(
            viewer.playhead.as_mut().unwrap(),
            1440.0,
            1.0,
            1_000
        ));
        assert_eq!(viewer.playhead.unwrap().hour, 1_000);
    }

    #[test]
    fn late_playing_anchors_cannot_rewind_a_clock_at_the_branch_end() {
        let now = Instant::now();
        let mut viewer = Viewer::create(
            Config::default(),
            Options::default(),
            None,
            Arc::new(Mutex::new(None)),
            None,
            None,
        );
        viewer.last_step = now;
        let head = Playhead {
            branch: 1,
            hour: 99,
            fraction: 0.0,
        };
        viewer.anchor_playback(head, true, 4.0, true, now, 100);
        viewer.tick_playhead(0.5, 100);
        assert!(!viewer.playing);
        let delayed = Playhead {
            fraction: 0.5,
            ..head
        };
        viewer.anchor_playback(
            delayed,
            true,
            4.0,
            false,
            now + Duration::from_millis(500),
            100,
        );
        assert_eq!(viewer.playhead.unwrap().hour, 100);
        assert!(!viewer.playing);
    }

    #[test]
    fn fractional_host_rates_advance_every_frame_and_finish_exactly() {
        for speed in [0.25, 24.0, 720.0, 1440.25] {
            let mut head = Playhead {
                branch: 1,
                hour: 100,
                fraction: 0.375,
            };
            for _ in 0..240 {
                let fraction = head.fraction + speed / 240.0;
                assert!(advance_clock(&mut head, fraction, 10_000));
            }
            let position = f64::from(head.hour) + head.fraction;
            assert!(
                (position - (100.375 + speed)).abs() < 0.0001,
                "{speed}: {position}"
            );
            assert!(!advance_clock(&mut head, 10_000.0, 10_000));
            assert_eq!((head.hour, head.fraction), (10_000, 0.0));
        }
    }

    #[test]
    fn the_playback_clock_runs_without_the_builders_and_stops_at_the_end() {
        let mut head = Playhead {
            branch: 1,
            hour: 10,
            fraction: 0.0,
        };
        assert!(advance_clock(&mut head, 6.25, 30));
        assert_eq!((head.hour, head.fraction), (16, 0.25));
        assert!(!advance_clock(&mut head, 100.0, 30));
        assert_eq!((head.hour, head.fraction), (30, 0.0));
        head.hour = i32::MIN;
        assert!(advance_clock(&mut head, 3_000_000_000.25, i32::MAX));
        assert_eq!((head.hour, head.fraction), (852_516_352, 0.25));
    }

    #[test]
    fn a_move_over_built_hours_is_a_skip_and_anything_else_a_seek() {
        // 13 was never built, 14 was built without buildings (a gap).
        let built = |hour: i32| match hour {
            13 => None,
            14 => Some(None),
            _ => Some(Some(hour)),
        };
        let skipped = |shown, next| skipped_hours(shown, next, false, built);
        assert_eq!(skipped(Some((1, 10)), (1, 13)), Some(vec![11, 12]));
        assert_eq!(skipped(Some((1, 10)), (1, 11)), Some(vec![]));
        assert_eq!(skipped(Some((1, 10)), (1, 10)), Some(vec![]), "same hour");
        assert_eq!(skipped(Some((1, 10)), (1, 14)), None, "13 is not built");
        assert_eq!(skipped(Some((1, 10)), (1, 9)), None, "backwards");
        assert_eq!(skipped(Some((2, 10)), (1, 12)), None, "another branch");
        assert_eq!(skipped(None, (1, 12)), None, "nothing on screen");
        // Playing, the builders passed 13 over and the history continues,
        // but not through the gap at 14.
        let playing = |shown, next| skipped_hours(shown, next, true, built);
        assert_eq!(playing(Some((1, 10)), (1, 14)), Some(vec![11, 12]));
        assert_eq!(playing(Some((1, 10)), (1, 16)), None, "14 is a gap");
        assert_eq!(playing(Some((1, 10)), (1, 9)), None, "backwards");
    }

    #[test]
    fn sparse_builds_keep_skipped_building_history_during_playback_and_paused_catchup() {
        // Only the sampled hours were built. Their building snapshots must
        // advance history in order; unbuilt hours are not fabricated snapshots.
        let built = |hour| match hour {
            16 => Some(Some("building added")),
            22 => Some(Some("building damaged")),
            _ => None,
        };
        // `passes_unbuilt` is deliberately independent of playing: the
        // final paused catch-up has the same continuity as the fast clock.
        assert_eq!(
            skipped_hours(Some((1, 10)), (1, 28), true, built),
            Some(vec!["building added", "building damaged"])
        );
        // An explicit seek cannot infer the unbuilt intermediate history.
        assert_eq!(skipped_hours(Some((1, 10)), (1, 28), false, built), None);
    }

    #[test]
    fn unload_releases_campaign_map_admission_and_discards_a_late_loader() {
        use replay::{campaign::Replay, spool::Record};
        use replay_data::{
            identity::CaptureMode, identity::Identity, identity::Roster, identity::SampleBoundary,
            identity::SessionStart, identity::Verification,
        };
        let path = PathBuf::from("memory.battleplan");
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                },
            },
        };
        let replay = Arc::new(
            Replay::from_records(
                b"unload-test".to_vec(),
                &[vec![Record::SessionStart(session)]],
            )
            .unwrap(),
        );
        let source = Arc::new(
            dashboard::Source::from_replay(
                dashboard::SourceKey {
                    source_id: "source-test".into(),
                    revision: "revision-test".into(),
                },
                path.clone(),
                replay,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap(),
        );
        let witness = Arc::downgrade(&source);
        let mut viewer = Viewer::create(
            Config::default(),
            Options {
                replay: Some(path.clone()),
                ..Default::default()
            },
            None,
            Arc::new(Mutex::new(None)),
            None,
            None,
        );
        viewer.source = Some(source);
        assert!(!viewer.unload_recording(Path::new("another.battleplan")));
        assert_eq!(witness.strong_count(), 1);
        assert!(viewer.unload_recording(&path));
        assert_eq!(witness.strong_count(), 0);
        let (send, receive) = mpsc::channel();
        viewer.loading = Some(receive);
        send.send(Err("obsolete load failure".into())).unwrap();
        viewer.poll(&egui::Context::default());
        assert!(viewer.loading.is_none());
        assert!(viewer.error.is_none());
        assert!(viewer.options.replay.is_none());
    }

    #[test]
    fn failed_replay_load_ends_a_window_dump_and_keeps_a_host_export() {
        for disconnected in [false, true] {
            for window_dump in [false, true] {
                let failure = Arc::new(Mutex::new(None));
                let mut viewer = Viewer::create(
                    Config::default(),
                    Options {
                        replay: Some("broken.battleplan".into()),
                        out: Some("capture.png".into()),
                        window_dump,
                        ..Default::default()
                    },
                    None,
                    Arc::clone(&failure),
                    None,
                    None,
                );
                if !window_dump {
                    viewer.capture = Some(Capture {
                        path: "campaign-map.png".into(),
                        remaining: 0,
                        pinned: None,
                        exit: false,
                        time: None,
                        step: 0.0,
                        frames: 1,
                        index: 0,
                        paced: false,
                    });
                }
                let (send, receive) = mpsc::channel();
                viewer.loading = Some(receive);
                let expected = if disconnected {
                    "Replay loader stopped before returning a result"
                } else {
                    send.send(Err("unsupported replay format".into())).unwrap();
                    "unsupported replay format"
                };
                drop(send);
                let ctx = egui::Context::default();
                let mut output = ctx.run_ui(Default::default(), |ui| viewer.poll(ui.ctx()));
                output.textures_delta.clear();
                assert!(viewer.loading.is_none());
                assert_eq!(viewer.capture.is_some(), !window_dump);
                assert_eq!(viewer.error.as_deref(), Some(expected));
                let closes = output.viewport_output.values().any(|viewport| {
                    viewport
                        .commands
                        .iter()
                        .any(|command| matches!(command, egui::ViewportCommand::Close))
                });
                assert_eq!(closes, window_dump);
                let reported = failure.lock().unwrap();
                if window_dump {
                    let message = reported.as_ref().unwrap();
                    assert!(message.contains("capture.png"));
                    assert!(message.contains("replay could not be loaded"));
                    assert!(message.contains(expected));
                } else {
                    assert!(reported.is_none());
                }
            }
        }
    }

    #[test]
    fn an_unavailable_map_ends_a_window_dump_instead_of_waiting_for_another_frame() {
        for window_dump in [false, true] {
            let failure = Arc::new(Mutex::new(None));
            let mut viewer = Viewer::create(
                Config::default(),
                Options {
                    out: Some("unavailable.png".into()),
                    window_dump,
                    ..Default::default()
                },
                None,
                Arc::clone(&failure),
                None,
                None,
            );
            if !window_dump {
                viewer.capture = Some(Capture {
                    path: "export.png".into(),
                    remaining: 0,
                    pinned: None,
                    exit: false,
                    time: None,
                    step: 0.0,
                    frames: 1,
                    index: 0,
                    paced: false,
                });
            }
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                viewer.failed_window_capture("hour was not recorded", ui.ctx());
            });
            output.textures_delta.clear();
            let closes = output.viewport_output.values().any(|viewport| {
                viewport
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::ViewportCommand::Close))
            });
            assert_eq!(closes, window_dump);
            assert_eq!(viewer.capture.is_none(), window_dump);
            let reported = failure.lock().unwrap();
            assert_eq!(
                reported.as_deref(),
                window_dump.then_some("Capture unavailable.png failed: hour was not recorded")
            );
        }
    }

    #[test]
    fn seek_from_stale_host_state_reopens_its_explicit_snapshot() {
        let seek = crate::embedded::Seek {
            path: PathBuf::from("A.battleplan"),
            revision: "100:20:0".into(),
            branch: 1,
            hour: 12,
            playback: None,
        };
        let target = seek.open_request();
        let a = Path::new("A.battleplan");
        let b = Path::new("B.battleplan");

        // Both recordings may contain branch 1/hour 12. A throttled host
        // state for A must not route its seek to a newly accepted native B.
        assert_eq!(
            seek_open_target(&seek, Some((b, Some("100:20:0"))), None),
            Some(target.clone())
        );
        // B may instead still be loading from the native picker, with its
        // actual revision only established when that load completes.
        assert_eq!(
            seek_open_target(&seek, None, Some((b, None))),
            Some(target.clone())
        );
        // A folder retry for B also supersedes a previously accepted A.
        assert_eq!(
            seek_open_target(
                &seek,
                Some((a, Some("100:20:0"))),
                Some((b, Some("120:30:0"))),
            ),
            Some(target.clone())
        );
        assert_eq!(seek_open_target(&seek, None, None), Some(target));
        assert_eq!(
            seek_open_target(&seek, Some((a, Some("100:20:0"))), None),
            None
        );
    }

    #[test]
    fn seek_reuses_only_the_exact_accepted_or_pending_revision() {
        let seek = crate::embedded::Seek {
            path: PathBuf::from("A.battleplan"),
            revision: "120:30:0".into(),
            branch: u64::MAX,
            hour: 12,
            playback: None,
        };
        let path = seek.path.as_path();
        for revision in [None, Some("100:20:0")] {
            assert_eq!(
                seek_open_target(&seek, Some((path, revision)), None),
                Some(seek.open_request())
            );
            assert_eq!(
                seek_open_target(
                    &seek,
                    Some((path, Some("120:30:0"))),
                    Some((path, revision)),
                ),
                Some(seek.open_request())
            );
            // The same strict predicate gates applying the seek after load.
            assert!(!seek.matches(path, revision));
        }
        assert_eq!(
            seek_open_target(&seek, None, Some((path, Some("120:30:0")))),
            None
        );
        assert!(seek.matches(path, Some("120:30:0")));
    }

    #[test]
    fn partial_recording_notice_names_the_failure_without_diagnostics() {
        use replay::{spool::ControlRecord, spool::Record};
        let healthy = crate::playback::tests::fixture();
        let partial = healthy
            .append(&[vec![
                Record::SessionStart(healthy.sessions[0].clone()),
                Record::BranchStart(healthy.branches[0].clone()),
                Record::Control(ControlRecord::Partial {
                    session: 1,
                    reason: "collector allocation failed".into(),
                    failed_hour: None,
                }),
            ]])
            .unwrap();
        let labels = |replay: &replay::campaign::Replay| {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| super::partial_notice(ui, replay));
            output.textures_delta.clear();
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(labels(&healthy).is_empty());
        let text = labels(&partial);
        assert!(text.iter().any(|text| text == "Partial recording:"));
        assert!(
            text.iter()
                .any(|text| text == "collector allocation failed")
        );
        assert!(!text.iter().any(|text| text.contains("hour")));
    }

    #[test]
    fn a_map_recording_names_its_mode_and_what_it_leaves_out() {
        use replay::{campaign::Replay, spool::Record};
        use replay_data::scope::CaptureMode;
        let healthy = crate::playback::tests::fixture();
        let mut session = healthy.sessions[0].clone();
        session.capture_mode = CaptureMode::Map;
        let map = Replay::from_records(
            healthy.campaign_id.clone(),
            &[vec![Record::SessionStart(session)]],
        )
        .unwrap();
        let labels = |replay: &Replay| {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                super::capture_mode_notice(ui, replay)
            });
            output.textures_delta.clear();
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(labels(&healthy).is_empty());
        let text = labels(&map).join(" ");
        assert!(
            text.starts_with("Map recording: not recorded: division names"),
            "{text}"
        );
        assert!(
            text.contains("fog of war") && text.contains("every dashboard"),
            "{text}"
        );
    }

    #[test]
    fn the_map_overlay_scales_with_the_interface() {
        let clip = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 600.0));
        let unscaled = super::overlay_rect(clip, 1.0);
        assert_eq!(unscaled.min, egui::pos2(108.0, 58.0));
        assert_eq!(unscaled.width(), 784.0);
        let scaled = super::overlay_rect(clip, 1.5);
        assert_eq!(scaled.min, egui::pos2(112.0, 62.0));
        assert_eq!(scaled.width(), 776.0);
        assert_eq!(scaled.height(), unscaled.height());
        let mut style = egui::Style::default();
        let body = egui::TextStyle::Body.resolve(&style).size;
        let spacing = style.spacing.item_spacing;
        let (indent, bar) = (style.spacing.indent, style.spacing.scroll.bar_width);
        super::scale_style(&mut style, 1.5);
        assert_eq!(egui::TextStyle::Body.resolve(&style).size, body * 1.5);
        assert_eq!(style.spacing.item_spacing, spacing * 1.5);
        assert_eq!(style.spacing.indent, indent * 1.5);
        assert_eq!(style.spacing.scroll.bar_width, bar * 1.5);
    }

    #[test]
    fn only_a_first_ctrl_f3_press_toggles_diagnostics() {
        let press = |pressed, repeat, modifiers| egui::Event::Key {
            key: egui::Key::F3,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        };
        assert!(super::is_diagnostics_shortcut(&press(
            true,
            false,
            egui::Modifiers::CTRL
        )));
        // F3 alone selects the air map mode, as in the game.
        assert!(!super::is_diagnostics_shortcut(&press(
            true,
            false,
            egui::Modifiers::NONE
        )));
        assert!(!super::is_diagnostics_shortcut(&press(
            false,
            false,
            egui::Modifiers::CTRL
        )));
        assert!(!super::is_diagnostics_shortcut(&press(
            true,
            true,
            egui::Modifiers::CTRL
        )));
        assert!(!super::is_diagnostics_shortcut(&press(
            true,
            false,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT
        )));
    }

    #[test]
    fn display_uses_the_save_calendar_at_midnight() {
        assert_eq!(date(60_765_882), "1936-09-29  19:00");
        assert_eq!(date(60_765_887), "1936-09-29  24:00");
        assert_eq!(date(60_765_888), "1936-09-30  01:00");
    }
}
