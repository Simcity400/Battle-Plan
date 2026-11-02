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
        frame::DayNight,
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
use viewer_foundation::diagnostics_panel::tooltip_warning_lines;
use viewer_world::map::scene::SceneInput;
use {
    viewer_counters::map::counters::CounterReport, viewer_foundation::map::name_draw::Placements,
    viewer_foundation::map::sprites::Batch, viewer_world::map::labels::LabelReport,
    viewer_world::map::name_pass::LayerFade, viewer_world::map::scene::Overlay,
    viewer_world::map::scene::Request,
};

mod drop;
mod embedded;
mod faction_highlight;
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
mod render_failure_tests;

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

/// What the frame path keeps of the hour on screen once its layers are uploaded.
struct ShownHour {
    epoch: u64,
    branch: replay_data::BranchId,
    hour: i32,
    /// Why the hour draws nothing: a recorded gap or a failed builder part.
    error: Option<String>,
    /// Why the hour draws nothing yet, no failure: its session is still being recorded, and
    /// the map holds its hours once the session's native recording lands.
    pending: Option<String>,
    political_warnings: Vec<String>,
    /// The battle plan's refusals and missing inputs ([`viewer_foundation::map::ribbon_content::OrderReport::warnings`]).
    plan_warnings: Vec<String>,
    /// The map buildings' refused type tokens
    /// ([`viewer_world::map::map_buildings::MapBuildingSummary::warnings`]).
    building_warnings: Vec<String>,
    /// The unit models' omitted stacks by reason
    /// ([`viewer_world::map::unit_models::ModelReport::warnings`]).
    model_warnings: Vec<String>,
    political_summary: (usize, usize),
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
        let (filled, distinct) = self.political_summary;
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
pub(crate) fn capture_mode_notice(ui: &mut egui::Ui, replay: &replay::container::Replay) {
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

pub(crate) fn partial_notice(ui: &mut egui::Ui, replay: &replay::container::Replay) {
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
    political: Option<u64>,
    borders: Option<u64>,
    rails: Option<u64>,
    buildings: Option<u64>,
    /// Last snow/mud snapshot handed to the scene. Its graphics refresh/fade
    /// continues locally after application; equal snapshots retain that history.
    snow_mud: Option<Arc<viewer_world::map::weather::SnowMud>>,
    /// The intel map on the GPU; hours with the same fog share one.
    intel: Option<Arc<IntelMap>>,
    /// The battle plan on the GPU; hours with the same plan tables share one
    /// (`viewer_counter_products::map::orders::TABLES`).
    orders: Option<Arc<viewer_foundation::map::ribbon_content::OrderField>>,
    air_order_sources: Option<(
        Arc<viewer_foundation::map::ribbon_content::OrderField>,
        Arc<viewer_world::map::air_orders::Prepared>,
        Vec<u64>,
    )>,
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
#[derive(PartialEq)]
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
    error: Option<String>,
    scene: Option<Scene>,
    scene_target: Presented,
    map_error: Option<String>,
    camera: Option<Placement>,
    camera_manual: bool,
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
    // close zoom
    unit_models: Option<Arc<viewer_world::map::unit_models::PreparedModels>>,
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
    label_report: LabelReport,
    /// The raid offer cycle, advanced off the frame path.
    raid_lane: crate::raid_lane::RaidLane,
    faction_highlight: faction_highlight::Lane,
    raid_tooltips: viewer_raids::map::raids::tooltips::lane::Lane,
    land_marker_tooltips: combat::land_marker_tooltips::Cache,
    naval_marker_tooltips: combat::naval_marker_tooltips::Cache,
    province_tooltips: viewer_counter_products::map::province_tooltips::Cache,
    /// The terrain map mode's province hover, which replaces the political one.
    terrain_tooltips: combat::terrain_mode::Cache,
    counter_tooltips: viewer_counter_products::map::counter_tooltips::Cache,
    division_panel_tooltips: viewer_combat::DivisionPanelCache,
    tooltip_refused: BTreeMap<String, usize>,
    // counters
    counter_report: CounterReport,
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
    /// The map mode picker drawn over the map, its slots and what it tells the host.
    picker: picker::PickerState,
    picker_error: Option<String>,
    picker_refused: std::collections::BTreeMap<String, usize>,
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
/// through the matching `set_*` as before, so pixels cannot change.
pub(crate) fn load_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    document: &Document,
    graphics: dashboard::protocol::MapGraphics,
) -> std::io::Result<Scene> {
    let mut scene = Scene::new(
        device,
        queue,
        &document.assets,
        &document.icons.images,
        &document.icons.lazy,
        graphics,
    )?;
    scene.apply(
        device,
        queue,
        SceneInput::InstallAnimatedText {
            assets: &document.icons.animated_text,
            gui: &document.icons.counters.gui,
        },
    )?;
    scene.apply(
        device,
        queue,
        SceneInput::SetBorders {
            assets: &document.border_assets,
            meshes: &document.border_meshes,
            straits: &document.straits,
        },
    )?;
    scene.apply(
        device,
        queue,
        SceneInput::SetRivers {
            assets: &document.river_assets,
            mesh: &document.river_mesh,
        },
    )?;
    scene.apply(
        device,
        queue,
        SceneInput::SetTrees {
            assets: &document.tree_assets,
            geometry: document.tree_geometry.clone(),
            field: &document.tree_field,
        },
    )?;
    scene.apply(device, queue, SceneInput::SetSprawl(&document.sprawl))?;
    // The map's ambient objects and the nuclear impact entities share this pass,
    // as in a dump: their sounds and impacts need it in every window.
    scene.apply(
        device,
        queue,
        SceneInput::SetMapFrame(&document.frame_assets),
    )?;
    scene.apply(device, queue, SceneInput::WarmTrains(&document.trains))?;
    scene.apply(device, queue, SceneInput::WarmAircraft(&document.aircraft))?;
    scene.apply(
        device,
        queue,
        SceneInput::WarmUnitModels(&document.unit_models),
    )?;
    scene.apply(
        device,
        queue,
        SceneInput::WarmMapBuildings(&document.map_buildings),
    )?;
    scene.apply(device, queue, SceneInput::WarmBridges(&document.bridges))?;
    scene.apply(
        device,
        queue,
        SceneInput::WarmRailways(&document.rail_assets),
    )?;
    scene.apply(device, queue, SceneInput::WarmOrders(&document.orders))?;
    scene.apply(
        device,
        queue,
        SceneInput::WarmWeather(document.weather_effects.clone()),
    )?;
    scene.apply(
        device,
        queue,
        SceneInput::WarmPolitical {
            map: &document.content.map,
            strategic_regions: &document.content.strategic_regions,
            field: &document.content.map_defines.gradient_field,
            cache: &document.content.political_gpu,
        },
    )?;
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
        let mut full = now
            .prepare_overlay_cached(
                camera,
                pixels,
                ui_scale,
                tables,
                &self.counter_view,
                &Default::default(),
            )
            .map_err(|error| error.to_string())?;
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
        cc: &eframe::CreationContext<'_>,
        config: Config,
        options: Options,
        error: Option<String>,
        // capture
        failure: Arc<Mutex<Option<String>>>,
        embedded: Option<crate::embedded::Controller>,
    ) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut viewer = Self::create(
            config,
            options,
            error,
            failure,
            embedded,
            cc.wgpu_render_state
                .as_ref()
                .map(|state| (state.device.clone(), state.queue.clone())),
        );
        if viewer.embedded.is_some() {
            match hours::Resources::new() {
                Ok(resources) => {
                    viewer.dropper = Some(Arc::clone(&resources.dropper));
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
            viewer.open(path, &cc.egui_ctx, None);
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
        });
        // counters: the perspective starts where the command line put it.
        let perspective = options.perspective.clone();
        let map_mode = options.map_mode.clone();
        // Replaced with the document's `settings.txt` options when one loads.
        let counter_view = CounterView::new(
            options.counter_settings(viewer_counters::map::counters::CounterSettings::default()),
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
            embedded_open: None,
            embedded_seek: None,
            revision: None,
            expected_revision: None,
            hidden_since,
            combat: combat::Window::default(),
            awaiting_folders: config.resolve_options().is_err(),
            camera: options.camera,
            camera_manual: options.camera.is_some(),
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
            // A host's maps take its graphics preferences when their recording
            // opens; a `--window-dump` keeps the dump's options.
            graphics: dashboard::protocol::MapGraphics {
                day_night: options.day_night.unwrap_or(true),
                show_allied_plans: options.allied_plans.unwrap_or(false),
                ..Default::default()
            },
            counter_filter: Default::default(),
            player_counters: false,
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
            visual_stale_campaign: None,
            config,
            options,
            document: None,
            loading: None,
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
            engaged_carry: None,
            epoch: 0,
            ready: BTreeMap::new(),
            shown: None,
            last_view: None,
            uploaded: Uploaded::default(),
            dropper: None,
            // close zoom
            unit_models: None,
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
            label_report: LabelReport::default(),
            raid_lane: Default::default(),
            faction_highlight: Default::default(),
            raid_tooltips: Default::default(),
            land_marker_tooltips: Default::default(),
            naval_marker_tooltips: Default::default(),
            province_tooltips: Default::default(),
            terrain_tooltips: Default::default(),
            counter_tooltips: Default::default(),
            division_panel_tooltips: Default::default(),
            tooltip_refused: Default::default(),
            // counters
            counter_report: CounterReport::default(),
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
            picker: Default::default(),
            picker_error: None,
            picker_refused: Default::default(),
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
        viewer.dropper = Some(Arc::clone(&resources.dropper));
        viewer.shared_resources = Some(resources);
        viewer
    }

    pub(crate) fn install_map(&mut self, document: Document, scene: Scene, revision: String) {
        self.clock_anchor = None;
        self.playhead = None;
        self.day_night = Default::default();
        self.counter_view = CounterView::new(self.options.counter_settings(document.game_counters));
        self.unit_selection = Default::default();
        self.air_selection = Default::default();
        self.fleet_selection = Default::default();
        self.document = Some(document);
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
        let settled = crate::visual_fallback::settled(
            self.drew_frame && error.is_none(),
            requested,
            self.rendered.as_ref().map(|head| (head.branch, head.hour)),
            self.shown
                .as_ref()
                .filter(|shown| shown.epoch == self.epoch)
                .map(|shown| (shown.branch, shown.hour, shown.error.is_some())),
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
    fn frame_day_night(&self, hour: i32) -> DayNight {
        DayNight::at_hour(hour).with_cycle(self.graphics.day_night)
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
                .unwrap_or_else(|| self.options.counter_settings(document.game_counters).nato);
        }
    }

    pub(crate) fn retire_map<T: Send + 'static>(&mut self, frame: &eframe::Frame, permit: T) {
        if let Some(texture) = self.scene_target.take()
            && let Some(state) = frame.wgpu_render_state()
        {
            match crate::renderer_host::local(state) {
                Some(host) => host.free(texture),
                None => state.renderer.write().free_texture(&texture),
            }
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
        self.rendered = None;
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
        self.raid_tooltips = Default::default();
        self.land_marker_tooltips = Default::default();
        self.naval_marker_tooltips = Default::default();
        self.province_tooltips = Default::default();
        self.terrain_tooltips = Default::default();
        self.counter_tooltips = Default::default();
        self.division_panel_tooltips = Default::default();
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
    }

    /// The map mode of the hour on screen: the chosen one ([`Self::map_mode`]) once an hour
    /// built for it is installed, the previous one while that hour is being built.
    fn shown_mode(&self) -> &viewer_world::map::map_mode::MapMode {
        self.placements_at
            .as_ref()
            .map_or(&self.map_mode, |(_, _, mode)| mode)
    }

    /// A previous epoch's picture stays visible while its replacement is built.
    fn picture_ready(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|shown| shown.error.is_none() && shown.pending.is_none())
    }

    /// Why the hour on screen, of this epoch, draws nothing yet, when it is not indexed yet.
    fn shown_pending(&self) -> Option<String> {
        self.shown
            .as_ref()
            .filter(|shown| shown.epoch == self.epoch)
            .and_then(|shown| shown.pending.clone())
    }

    /// Whether the frame drawn answers `target` with a built hour of this
    /// epoch: exactly, or, playing, with the newest hour built before it.
    fn presentation_ready(&self, target: Playhead, playing: bool) -> bool {
        self.shown.as_ref().is_some_and(|shown| {
            !self.animation_seek
                && shown.epoch == self.epoch
                && shown.error.is_none()
                && shown.pending.is_none()
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
        let error = self.error.clone().or_else(|| {
            self.shown
                .as_ref()
                .filter(|shown| {
                    shown.branch.to_string() == target.branch_id && shown.hour == target.hour
                })
                .and(self.map_error.clone())
        });
        let ready = target.branch_id.parse().ok().is_some_and(|branch| {
            let requested = Playhead {
                branch,
                hour: target.hour,
                fraction: f64::from(target.fraction),
            };
            self.presentation_ready(
                if target.playing {
                    self.playhead.unwrap_or(requested)
                } else {
                    requested
                },
                target.playing,
            )
        }) && self.rendered_sequence == target.sequence;
        let pending = self
            .shown
            .as_ref()
            .filter(|shown| {
                shown.epoch == self.epoch
                    && shown.branch.to_string() == target.branch_id
                    && shown.hour == target.hour
            })
            .and_then(|shown| shown.pending.clone());
        dashboard::protocol::DashboardMapState {
            id: target.id.clone(),
            source_id: Some(target.source_id.clone()),
            revision: Some(target.revision.clone()),
            branch_id: self.rendered.map(|shown| shown.branch.to_string()),
            hour: self.rendered.map(|shown| shown.hour),
            fraction: self.rendered.map(|shown| shown.fraction as f32),
            sequence: self.rendered_sequence,
            requested_sequence: target.sequence,
            loading: error.is_none() && pending.is_none() && !ready,
            error,
            pending,
        }
    }

    /// Everything the hour on screen could not draw exactly, collected after its overlay and
    /// hover. Current tooltip and combat gaps lead; persistent content notes follow. Each
    /// frame replaces the per-hour report, so recovered refusals go away with the next draw.
    fn collect_warnings(&self, document: &Document) -> Vec<String> {
        let Some(shown) = &self.shown else {
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
        warnings.extend(shown.limitations.lines());
        // The viewer's device, when it lacks a feature some content needs to draw exactly.
        if let Some((device, _)) = &self.gpu {
            warnings.extend(device_requirements(device.features()));
        }
        warnings.extend(document.warnings.iter().cloned());
        warnings.extend(shown.political_warnings.iter().cloned());
        warnings.extend(shown.plan_warnings.iter().cloned());
        warnings.extend(shown.building_warnings.iter().cloned());
        warnings.extend(shown.model_warnings.iter().cloned());
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
        if let Some(Ok(index)) = self
            .hours
            .as_ref()
            .map(|hours| hours.engaged(&self.perspective.country))
        {
            warnings.extend(index.shared.byte_check().lines());
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
        self.shown.as_ref().and_then(|shown| shown.snow.0.note())
    }

    /// The diagnostics an embedded map shows over itself: the hour's stats and
    /// its data limitations, collapsed behind their count.
    pub(crate) fn diagnostics_ui(&mut self, ui: &mut egui::Ui, max_height: f32) {
        if let Some(shown) = &self.shown {
            for line in shown.stats() {
                ui.weak(line);
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
        self.options.replay = Some(path.clone());
        let options = self.options.clone();
        let verify_revision = self.embedded.is_some();
        let service = self
            .embedded
            .as_ref()
            .map(crate::embedded::Controller::service);
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
        std::thread::spawn(move || {
            let initial = InitialHour {
                branch: options.branch,
                hour: options.hour,
                hour_fraction: options.hour_fraction,
            };
            let result = (|| -> Result<LoadedReplay, String> {
                let revision = if verify_revision {
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
                let source = service
                    .as_ref()
                    .map(|service| service.open_map(&path, revision.as_deref()))
                    .transpose()?;
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
                            Some((_, content)) => Document::from_shared_content(
                                Arc::clone(content),
                                source.replay(),
                                &initial,
                            ),
                            None => Document::load_shared(
                                &path,
                                &resolve,
                                options.allow_unverified,
                                &initial,
                                source.replay(),
                                preferences.visual_mods.as_deref(),
                                preferences.graphics.texture_quality,
                                recording.as_deref().zip(decisions.as_ref()),
                            ),
                        },
                        None => Document::load(
                            &path,
                            &resolve,
                            options.allow_unverified,
                            &initial,
                            None,
                            0,
                        ),
                    }
                    .map_err(|error| error.to_string())?;
                    if let Some(source) = &source {
                        document.bind_source(source);
                    }
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
                    // The builders need seconds for the first hour and the UI
                    // thread idles on the spinner meanwhile; the whole
                    // hour-independent scene is cheaper here than on any frame.
                    match load_scene(&device, &queue, &document, preferences.graphics) {
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
            })();
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.loading = Some(receiver);
        self.opened = Some(Instant::now());
        self.combat.close();
        self.document = None;
        self.raid_tooltips = Default::default();
        self.land_marker_tooltips = Default::default();
        self.naval_marker_tooltips = Default::default();
        self.province_tooltips = Default::default();
        self.terrain_tooltips = Default::default();
        self.counter_tooltips = Default::default();
        self.division_panel_tooltips = Default::default();
        self.tooltip_refused.clear();
        self.source = None;
        self.source_decoder = None;
        self.loaded_config = None;
        self.rendered = None;
        self.revision = None;
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
        self.unit_field_uploaded = false;
        self.profile_frame = 0;
        self.scene_target.generation = 0;
        if self.embedded.is_none() {
            self.error = None;
        }
        self.map_error = None;
        self.playing = false;
        self.camera = self.options.camera;
        self.camera_manual = self.options.camera.is_some();
        self.zoom.reset();
        self.orbit.reset();
        self.walk.reset();
        self.flight = None;
        // A start chosen on the last map is chosen again on this one, with the pointer given back:
        // the new map's first frame shows the cursor again.
        if self.start_pick.at().is_some() {
            self.start_pick = drop::Pick::Choosing;
            self.input.release();
        }
        self.fly_clock = None;
        self.fly_footprints = Default::default();
        // names
        self.placements = None;
        self.placements_at = None;
        self.fade = None;
        // counters
        self.overlay = None;
        self.prepared_overlay = None;
        self.counter_report = CounterReport::default();
        self.unit_selection = Default::default();
        self.air_selection = Default::default();
        self.fleet_selection = Default::default();
        self.overlay_fraction = None;
        self.overlay_at = None;
        self.overlay_tables = None;
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
    /// and the process; run_native hands the message to main, which exits 1.
    /// Other captures belong to the map on screen and stay armed.
    fn failed_load(&mut self, error: String, ctx: &egui::Context) {
        self.failed_window_capture(&format!("replay could not be loaded: {error}"), ctx);
        self.error = Some(error);
    }

    fn hour_error(&mut self, error: Option<&std::io::Error>, ctx: &egui::Context) {
        self.map_error = error.map(ToString::to_string);
        if let Some(error) = error
            && error.kind() != std::io::ErrorKind::WouldBlock
        {
            self.failed_window_capture(&error.to_string(), ctx);
        }
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
                self.revision = revision;
                self.source = source;
                self.source_decoder = decoder;
                self.loaded_config = Some(config);
                if self.embedded.is_some() {
                    self.error = None;
                }
                if let Some(selector) = self.options.combat {
                    match document.frame().and_then(|frame| selector.resolve(frame)) {
                        Ok(selection) => self.combat.open(selection),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
                self.playhead = Some(Playhead {
                    branch: document.playback.branch(),
                    hour: document.playback.hour(),
                    fraction: f64::from(document.hour_fraction),
                });
                self.clock_anchor = None;
                self.counter_view =
                    CounterView::new(self.options.counter_settings(document.game_counters));
                if self.embedded.is_none() {
                    self.graphics.full_resolution = document.game_full_resolution;
                    self.volumes = document.game_audio;
                }
                self.document = Some(document);
                // The loader built the whole hour-independent scene; the
                // frames only upload hour data into it from here.
                self.scene = Some(scene);
                self.uploaded = Uploaded::default();
                self.loading = None;
                if let (Some(embedded), Some(source)) = (&self.embedded, &self.source) {
                    let settings = embedded.service().map_settings(&source.replay());
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
        top_map_hit(
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
        )
    }

    /// The layer key of `target`'s hit shape; `None` for targets that are not one map icon.
    fn hit_layer(&self, target: MapHit) -> Option<(free_icons::IconFamily, u64)> {
        use free_icons::IconFamily;
        let report = &self.counter_report;
        match target {
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
        let Ok(color) = &document.border_assets.faction_highlight_color else {
            return Ok(());
        };
        let selected = point
            .and_then(|point| self.faction_pin_at(point))
            .map(|hit| Arc::clone(&hit.regions))
            .unwrap_or_default();
        let Some(shown) = &self.shown else {
            return Ok(());
        };
        let Some((political, borders)) = &shown.borders else {
            return Ok(());
        };
        let key = faction_highlight::Key {
            epoch: shown.epoch,
            branch: shown.branch,
            hour: shown.hour,
            perspective: self.perspective.clone(),
            borders: borders.id,
            regions: Arc::clone(&selected),
        };
        let scene = self.scene.as_mut().expect("scene built");
        if self.faction_highlight.requested() != Some(&key)
            && !scene.faction_highlight_regions().is_empty()
        {
            scene
                .apply(
                    device,
                    queue,
                    SceneInput::SetBorderPlans {
                        classes: &borders.classes,
                        zone: &borders.highlighted,
                    },
                )
                .map_err(|why| why.to_string())?;
        }
        let content = Arc::clone(&document.content);
        let political = Arc::clone(political);
        let borders = Arc::clone(borders);
        let Some(hours) = &self.hours else {
            return Ok(());
        };
        let dropper = self.dropper.as_ref().expect("hour retirement worker");
        if let Some(plan) = self.faction_highlight.update(
            key,
            |job| hours.try_submit(job),
            move || {
                let provinces = selected
                    .iter()
                    .flat_map(|id| {
                        content
                            .strategic_regions
                            .get(id)
                            .into_iter()
                            .flat_map(|region| region.provinces.iter().copied())
                    })
                    .collect();
                viewer_world::map::scene::FactionHighlight::new(
                    &content.border_meshes,
                    &political.layers.regions,
                    &borders.zone,
                    &provinces,
                )
            },
            dropper,
        ) {
            let result = scene.apply(
                device,
                queue,
                SceneInput::SetFactionHighlight {
                    plan: &plan,
                    strategic_regions: &self
                        .faction_highlight
                        .requested()
                        .expect("requested highlight")
                        .regions,
                    color: *color,
                },
            );
            dropper.drop_later(plan);
            result.map_err(|why| why.to_string())?;
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
        if let Some(MapHit::Occupation(index)) = target {
            let hit = self
                .prepared_overlay
                .as_ref()?
                .occupation_hits()
                .get(index)?;
            let rect = self.shown_rect(layer, hit.bounds()?);
            let crate::playback::Current::Frame(frame) = document.playback.current() else {
                return None;
            };
            use viewer_world::map::occupation_icons::Hover;
            let text = match &hit.hover {
                Hover::Modifier { state, token } => {
                    let result = (|| {
                        let content = document.icons.occupation.as_ref().map_err(Clone::clone)?;
                        content
                            .definitions
                            .find(token)?
                            .ok_or_else(|| format!("active definition {token} is absent"))
                    })();
                    // The same catalogue chose and drew the badge; keep its exact definition.
                    let text = match result {
                        Ok(definition) => {
                            viewer_counter_products::map::occupation_modifier_tooltips::build(
                                &document.counter_content,
                                frame,
                                *state,
                                definition,
                            )
                        }
                        Err(why) => {
                            let mut text = Tooltip::default();
                            text.refused
                                .insert(format!("occupation badge {token}: {why}"), 1);
                            text
                        }
                    };
                    Arc::new(text)
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
                    self.province_tooltips.request(
                        &document.counter_content,
                        document.icons.raids.movement_reads(),
                        frame,
                        province,
                        self.overlay_tables.as_ref()?.3.vision(),
                        self.counter_view.settings.nato,
                        self.unit_selection.ordered_units(),
                        &document.playback.replay_handle(),
                        &viewer_counter_products::map::province_tooltips::ModeHover::OccupationMarker,
                        {
                            let ctx = ctx.clone();
                            move || ctx.request_repaint()
                        },
                    )
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
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let vision = self.overlay_tables.as_ref()?.3.vision();
            let text = viewer_counter_products::map::counter_tooltips::Batch::new(
                &document.counter_content,
                document.icons.raids.movement_reads(),
                frame,
                vision,
                self.hover_style(),
            )
            .build_air_base(province, kind, self.air_selection.wings());
            return Some((
                Arc::new(text),
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
            let hit = self.counter_report.naval_region_hovers.get(index)?;
            let crate::playback::Current::Frame(frame) = document.playback.current() else {
                return None;
            };
            let vision = self.overlay_tables.as_ref()?.3.vision();
            let text = match hit.control {
                NavalRegionalControl::Mines => {
                    viewer_counter_products::map::province_tooltips::naval_mines::build(
                        &document.counter_content,
                        frame,
                        vision,
                        hit.region,
                        false,
                    )
                }
                NavalRegionalControl::Accidents => {
                    viewer_counter_products::map::province_tooltips::naval_accidents::build(
                        &document.counter_content,
                        frame,
                        vision,
                        hit.region,
                    )
                }
                _ => {
                    let mut text = Tooltip::default();
                    text.refuse(
                        "regional Navy control hover",
                        "drawn control has no prepared hover text".into(),
                    );
                    text
                }
            };
            return Some((Arc::new(text), self.shown_rect(layer, hit.rect), None));
        }
        if matches!(target, Some(MapHit::Raid))
            && let Some(hit) = self.raid_hit(point).cloned()
        {
            let crate::playback::Current::Frame(frame) = document.playback.current() else {
                return None;
            };
            let ours = self.overlay_tables.as_ref()?.3.ours();
            let text = self.raid_tooltips.request(
                Arc::clone(&document.raid_hover),
                Arc::clone(frame),
                ours,
                self.counter_view.settings.nato,
                hit.key.clone(),
                ctx,
            )?;
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
        if let Some((id, rect)) = land_hit
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let viewer = self.overlay_tables.as_ref()?.3.ours();
            let index = match self
                .hours
                .as_ref()
                .map(|hours| hours.engaged(&self.perspective.country))
                .transpose()
            {
                Ok(index) => index,
                Err(why) => {
                    let mut text = Tooltip::default();
                    text.refused
                        .insert(format!("land marker {id} engaged index: {why}"), 1);
                    return Some((Arc::new(text), self.shown_rect(layer, rect), None));
                }
            };
            let text = self.land_marker_tooltips.request(
                &document.combat,
                frame,
                viewer,
                id,
                index.as_ref().map(|index| index.shared.as_ref()),
                self.counter_view.settings.nato,
            );
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
        if let Some((id, rect)) = naval_hit
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let viewer = self.overlay_tables.as_ref()?.3.ours();
            let index = match self
                .hours
                .as_ref()
                .map(|hours| hours.engaged(&self.perspective.country))
                .transpose()
            {
                Ok(index) => index,
                Err(why) => {
                    let mut text = Tooltip::default();
                    text.refused
                        .insert(format!("naval marker {id} engaged index: {why}"), 1);
                    return Some((Arc::new(text), self.shown_rect(layer, rect), None));
                }
            };
            let text = self.naval_marker_tooltips.request(
                &document.combat,
                frame,
                viewer,
                id,
                index.as_ref().map(|index| index.shared.as_ref()),
            );
            return Some((text, self.shown_rect(layer, rect), None));
        }
        if let Some(hit) = self.counter_report.land_hovers.iter().rev().find(|hit| {
            hit.contains(local)
                && matches!(target, Some(MapHit::Unit(index))
                if land_hover_matches_unit(hit, &self.counter_report.unit_hits[index]))
        }) && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            let vision = self.overlay_tables.as_ref()?.3.vision();
            let text = self.counter_tooltips.request(
                &document.counter_content,
                document.icons.raids.movement_reads(),
                frame,
                hit,
                vision,
                self.hover_style(),
            );
            let text = if hit.ours
                && !hit.hidden
                && !hit.railway_gun
                && hit.members.len() == 1
                && hit.control
                    == viewer_foundation::map::counter_hovers::land::CounterControl::Normal
            {
                self.division_panel_tooltips
                    .request(&document.combat, frame, text, hit.members[0])
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
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            if !hit.supported {
                return None;
            }
            let vision = self.overlay_tables.as_ref()?.3.vision();
            let text = self.counter_tooltips.request_task_force(
                &document.counter_content,
                document.icons.raids.movement_reads(),
                frame,
                hit,
                vision,
                self.hover_style(),
            );
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if let Some((_, hit)) = self.counter_report.task_force_port_hovers.iter().enumerate().rev()
            .find(|(hover_index, hit)| hit.contains(local) && hit.has_tooltip()
                && self.counter_report.unit_hits.get(hit.unit_hit).is_some_and(|unit| hit.matches(unit))
                && (matches!(target, Some(MapHit::Unit(index)) if index == hit.unit_hit)
                    || matches!(target, Some(MapHit::PortTooltip(index)) if index == *hover_index)))
            && let crate::playback::Current::Frame(frame) = document.playback.current()
        {
            if !hit.supported { return None; }
            let vision = self.overlay_tables.as_ref()?.3.vision();
            let text = self.counter_tooltips.request_task_force_port(&document.counter_content, document.icons.raids.movement_reads(), frame, hit, vision, self.hover_style());
            return Some((text, self.shown_rect(layer, hit.rect), None));
        }
        if target.is_some() {
            return None;
        }
        let crate::playback::Current::Frame(frame) = document.playback.current() else {
            return None;
        };
        let view = self.last_view.as_ref()?;
        if self.counter_view.visibility.trade_routes
            && self.uploaded.orders.as_ref().is_some_and(|orders| {
                viewer_foundation::map::ribbon_content::trade_route_hit(
                    &orders.trade_route_hovers,
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
        let vision = self.overlay_tables.as_ref()?.3.vision();
        if *self.shown_mode()
            == viewer_world::map::map_mode::MapMode::Stock(
                viewer_world::map::map_mode::Stock::Terrain,
            )
        {
            let text = self
                .terrain_tooltips
                .request(&document.combat, frame, province, vision);
            return Some((text, [0.0, 0.0, extent[0], extent[1]], Some(province)));
        }
        let mut mode_hover = picker::mode_hover(
            self.shown_mode(),
            document.map_modes.defines.opinion_thresholds(),
            || {
                document
                    .map_modes
                    .infrastructure
                    .hover_level(frame, province)
            },
        );
        if let viewer_counter_products::map::province_tooltips::ModeHover::StrategicAir(selected) =
            &mut mode_hover
        {
            *selected = Arc::clone(self.air_selection.wings());
        }
        if let viewer_counter_products::map::province_tooltips::ModeHover::StrategicNavy(sources) =
            &mut mode_hover
        {
            *sources = self.overlay_tables.as_ref()?.3.naval_dominance_sources();
        }
        let text = self.province_tooltips.request(
            &document.counter_content,
            document.icons.raids.movement_reads(),
            frame,
            province,
            vision,
            self.counter_view.settings.nato,
            self.unit_selection.ordered_units(),
            &document.playback.replay_handle(),
            &mode_hover,
            {
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            },
        );
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
        let result = self.draw_map(ui, state, document);
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
        }
        // The scene arrives with the document from the loading thread
        // (`load_scene`); only hour data uploads per frame from here.
        let hour_error = self.refresh_hour(state, document, &mut laps, ui.ctx()).err();
        self.hour_error(hour_error.as_ref(), ui.ctx());
        // An error view otherwise stops repainting; a paused seek away from it
        // still needs to poll the asynchronous builders until its hour arrives.
        if self
            .playhead
            .is_some_and(|head| !self.hour_ready(head.branch, head.hour))
        {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        if let Some(error) = self.map_error.clone() {
            self.rendered = None;
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
            return Ok(());
        }
        if let Some(pending) = self.shown_pending() {
            self.rendered = None;
            ui.centered_and_justified(|ui| {
                ui.label(format!("Not indexed yet: {pending}"));
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
        let stale_hour = self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.epoch != self.epoch);
        let perspective = if stale_hour {
            self.overlay_tables
                .as_ref()
                .expect("installed with the hour")
                .2
                .clone()
        } else {
            self.perspective.clone()
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
        if let Some((_, _, mode)) = &self.placements_at {
            self.counter_view.set_mode(mode);
        }
        laps.lap("hour_misc");
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.camera", Some("other"));
        // The screen-space interface's scale. A capture keeps the dump's, 1.0
        // unless `--ui-scale` names one, so a `--window-dump` and a `--dump` of
        // the same state stay the same frame.
        let pinned = self.capture.as_ref().and_then(|capture| capture.pinned);
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
        // Only diagnostics and a capture's saving frame need warning text.
        let build_warnings = builds_warnings(self.diagnostics, self.capture.as_ref());
        self.interface_points =
            ui_scale_points(1.0, raster.screen_scale(), ui.ctx().pixels_per_point());
        let size = ui.available_size().max(egui::vec2(1.0, 1.0));
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
            .hours
            .as_ref()
            .map(|hours| hours.engaged(&perspective.country))
            .transpose()
            .map_err(|error| error.to_string())?;
        let animation_time = self
            .capture
            .as_ref()
            .and_then(|capture| capture.time)
            .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
        // Hit tests and drawing use the same GUI clock.
        let combat_geometry = self
            .combat
            .draw(
                &document.combat,
                document.playback.branch(),
                document.playback.hour(),
                document.frame(),
                extent,
                &perspective,
                engaged.as_ref().map(|index| index.shared.as_ref()),
                self.counter_view.settings.nato,
                false,
                animation_time,
                false,
            )
            .map_err(|error| error.to_string())?;
        let point = profile_hover_point(
            response
                .hover_pos()
                .or_else(|| response.interact_pointer_pos())
                .map(frame_point),
            self.options.profile_pointer,
            profiling(),
            pixels,
            ui_scale,
        );
        if takes_keys {
            self.picker_keys(ui, document, extent);
        }
        // The picker's lists take no scrollbar, so the wheel over them does nothing of theirs.
        let picker_hit = match self.draw_picker(document, extent, false, animation_time, false) {
            Some(Ok(drawn)) => point.and_then(|point| drawn.control_at(point).cloned()),
            _ => None,
        };
        if let Some(control) = &picker_hit {
            if response.clicked() {
                self.picker_click(document, control, false);
            } else if response.secondary_clicked() {
                self.picker_click(document, control, true);
            }
        } else if response.clicked() && self.picker.open != viewer_combat::picker::Open::None {
            // A click outside the open list closes it.
            self.picker.open = viewer_combat::picker::Open::None;
        }
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
        let popup_click = response.clicked()
            && !over_picker
            && point
                .zip(combat_geometry.as_ref())
                .is_some_and(|(point, drawn)| self.combat.handle_click(drawn, point));
        if response.clicked()
            && !popup_click
            && !over_picker
            && let Some(point) = point
            && combat_geometry
                .as_ref()
                .is_some_and(|window| window.closes(point))
        {
            self.combat.close();
        }
        if over_combat {
            if response.clicked()
                && let Some((side, direction)) = point.and_then(|point| {
                    combat_geometry
                        .as_ref()
                        .and_then(|window| window.scroll_action(point))
                })
            {
                self.combat.scroll(side, direction, false);
            }
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
        let frame_delta = self.last_frame.elapsed().as_secs_f32();
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
                        let key = (
                            document.playback.branch(),
                            document.playback.hour(),
                            perspective.clone(),
                        );
                        if self.walk.stale(&key) {
                            let figure = document.hour()
                                .and_then(|now| now.vision_from_overlay(
                                    &perspective,
                                    self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables),
                                ).map(|vision| (now.frame, vision.ours())))
                                .map_err(|error| error.to_string())
                                .and_then(|(frame, ours)| {
                                    document.unit_models.infantry_figure(frame, ours)
                                });
                            self.walk.set_figure(key, figure);
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
        laps.lap("panel_camera");
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.movers", Some("other"));
        let scene = self.scene.as_mut().expect("created above");
        // A `--window-dump` settles unit models for its camera, as `--dump`
        // does; a host export keeps the live history.
        if !self.unit_field_uploaded {
            let timer = Instant::now();
            let units = self
                .unit_models
                .as_ref()
                .expect("prepared above")
                .field(&document.unit_models);
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
        let tick = document.hour_fraction;
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
        let sailing = self.unit_models.as_ref().is_some_and(|prepared| {
            prepared.has_convoys()
                && document
                    .trade_routes
                    .as_ref()
                    .is_ok_and(|defines| convoys_shown(&defines.convoys))
        });
        if (self.raid_tick != Some(tick) || sailing || self.convoys_sailing)
            && let Some(prepared) = self.unit_models.as_ref()
        {
            let hour = document.hour().map_err(|error| error.to_string())?;
            let heights = hour.heights();
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
        let now = document.hour().map_err(|error| error.to_string())?;
        let vision = now
            .vision_from_overlay(
                &perspective,
                self.overlay_tables.as_ref().map(|(_, _, _, tables)| tables),
            )
            .map_err(|error| error.to_string())?;
        self.unit_selection.observe(
            document.playback.branch(),
            &perspective.country,
            vision.ours(),
            now.frame,
        );
        self.counter_view.selected = Arc::clone(self.unit_selection.units());
        laps.lap("unit_selection");
        self.air_selection.observe(
            document.playback.branch(),
            &perspective.country,
            vision.ours(),
            now.frame,
        );
        self.counter_view.air_selected = Arc::clone(self.air_selection.wings());
        self.counter_view.air_bases = Arc::clone(self.air_selection.bases());
        laps.lap("air_selection");
        self.fleet_selection.observe(
            document.playback.branch(),
            &perspective.country,
            vision.ours(),
            now.frame,
        );
        self.counter_view.fleets_selected = Arc::clone(self.fleet_selection.fleets());
        laps.lap("fleet_selection");
        drop(_other_cost);
        let _other_cost = replay_frame::cpu::scope("frame.name-hover", Some("other"));
        // A free mode's pointer reaches the map's interface while it is shown and the pointer is
        // not held for looking around.
        let point = point.filter(|_| !free_mode || (interface && !self.input.captured()));
        self.counter_view.hovered = point.filter(|_| !over_combat).and_then(|point| {
            let MapHit::Unit(index) = self.map_hit(point)? else {
                return None;
            };
            self.counter_report.unit_hits[index].units().next()
        });
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
        let names_mode = &self
            .placements_at
            .as_ref()
            .expect("installed with the hour")
            .2;
        let strategic_names = matches!(
            names_mode,
            viewer_world::map::map_mode::MapMode::Stock(
                viewer_world::map::map_mode::Stock::StrategicAir
                    | viewer_world::map::map_mode::Stock::StrategicNavy
            )
        );
        let show_all_names = document.content.game_strategic_names || !strategic_names;
        let strategic_hover = (!show_all_names).then_some(hovered_name_province).flatten();
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
        let overlay_key = OverlayKey {
            occupation: self.counter_view.occupation,
            operatives: self.counter_view.operatives,
            branch: document.playback.branch(),
            hour: document.playback.hour(),
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
            let engaged = self
                .hours
                .as_ref()
                .map(|hours| hours.engaged(&perspective.country))
                .transpose()
                .map_err(|error| error.to_string())?;
            // An overlay laid out before the engaged index reached its hour, by the
            // window or by an hour builder, is laid out again once it does, so a
            // paused seek or a prebuilt hour settles on the exact flag.
            let engaged_arrived = engaged_relayout(
                self.prepared_overlay
                    .as_ref()
                    .zip(self.overlay_at.as_ref())
                    .map(|(prepared, key)| (prepared.engaged_covered(), key.branch, key.hour)),
                |branch, hour| {
                    engaged
                        .as_ref()
                        .is_some_and(|index| index.shared.covers(branch, hour))
                },
            );
            // The raid lane's icons for the hour on screen replace the hour's
            // when they are newer; a paused frame asks the lane for one batch.
            let ctx = ui.ctx().clone();
            // Its batches borrow the hour builders' threads, as do the icon layer's owners
            // below. A view's counters ride in builder jobs, so they hold no handle to the pool.
            let overlay_workers = self
                .hours
                .as_ref()
                .map(|hours| hours.frame_workers())
                .unwrap_or_default();
            if let Some(hours) = &self.hours {
                self.raid_lane.use_workers(hours.workers());
                if let Some(scene) = self.scene.as_mut() {
                    scene
                        .apply(
                            &state.device,
                            &state.queue,
                            SceneInput::UseWorkers(hours.frame_workers()),
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            self.raid_lane.show(
                &document.content,
                &perspective,
                (document.playback.branch(), document.playback.hour()),
                self.playing,
                move || ctx.request_repaint(),
            );
            // Only icons of the lane's current reset, built for the hour and the
            // resolved viewing country the tables are for, install.
            let mut raids_changed = false;
            if let Some((branch, hour, _, tables)) = &mut self.overlay_tables
                && let Some(icons) = self.raid_lane.icons(*branch, *hour, tables.ours())
                && !Arc::ptr_eq(&icons, &tables.raids())
            {
                raids_changed = tables.set_raids(icons);
            }
            let layout_changed =
                stale || engaged_arrived || self.overlay_at.as_ref() != Some(&overlay_key);
            // A raid batch on the laid-out hour, camera and view replaces the raid
            // icons alone: the rest of the overlay does not read them. The batch
            // drawn at this fraction takes them in place, as drawing it again
            // would; a new fraction draws it again below.
            let raids_only = raids_changed && !layout_changed;
            if raids_only
                && let (Some(prepared), Some((_, _, _, tables))) =
                    (self.prepared_overlay.as_mut(), self.overlay_tables.as_ref())
            {
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
                let tables_key = (
                    document.playback.branch(),
                    document.playback.hour(),
                    perspective.clone(),
                );
                let tables_mismatch = self.overlay_tables.as_ref().is_none_or(
                    |(branch, hour, perspective, _)| {
                        (*branch, *hour, perspective)
                            != (tables_key.0, tables_key.1, &tables_key.2)
                    },
                );
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
                            )
                        } else {
                            now.hour_tables_with_raids(&perspective, raids, engaged.as_deref())
                        }
                        .map_err(|error| error.to_string())?
                    };
                    let replaced = self.overlay_tables.replace((
                        tables_key.0, tables_key.1, tables_key.2, tables,
                    ));
                    if refine_arrival {
                        retired_tables = replaced;
                    } else {
                        drop(replaced);
                    }
                }
                let tables = &self.overlay_tables.as_ref().expect("parsed above").3;
                let prepared = now
                    .prepare_overlay_cached(
                        overlay_camera,
                        pixels,
                        ui_scale,
                        tables,
                        &self.counter_view,
                        &overlay_workers,
                    )
                    .map_err(|error| error.to_string())?;
                let retired_prepared = self.prepared_overlay.replace(prepared);
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
            overlay_prepare_cpu.finish();
            laps.lap("overlay_prepare");
            drop(_other_cost);
            let _other_cost = replay_frame::cpu::scope("frame.overlay.draw", Some("other"));
            let overlay_draw_cpu = viewer_foundation::profiling::thread_span("overlay_draw");
            // Display choices only change what the next overlay draw keeps.
            let ours = self.overlay_tables.as_ref().expect("parsed above").3.ours();
            let filtered = self.counter_owners.get(now.frame, &self.counter_filter);
            let layers = crate::document::OverlayLayers::shown(
                &self.graphics,
                self.shown_counter_owners(filtered, ours),
            );
            if layers != self.overlay_layers {
                self.overlay_layers = layers;
                self.overlay_fraction = None;
            }
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
                crate::document::select_movement(
                    scene,
                    &state.device,
                    &state.queue,
                    &document.content,
                    now.frame,
                    self.overlay_tables.as_ref().expect("parsed above").3.ours(),
                    self.unit_selection.units().as_ref(),
                )
                .map_err(|error| error.to_string())?;
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
                                hour: document.playback.hour(),
                                clock_speed: document.frame().ok().and_then(
                                    viewer_foundation::map::counter_move::MovementFrame::clock_speed,
                                ),
                                speeds: document.content.counters.defines.movement_speeds,
                                steady: self.options.window_dump,
                            },
                            schedule.updates().filter_map(|(province, update)| {
                                (update == viewer_foundation::map::province_schedule::Update::InView)
                                    .then_some(province)
                            }),
                        );
                }
                if let Some(prepared) = &self.unit_models {
                    scene
                        .apply(
                            &state.device,
                            &state.queue,
                            SceneInput::SetUnitMovement {
                                prepared,
                                history: presentation.movement(),
                            },
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            if self.overlay_fraction != Some(document.hour_fraction) {
                let prepared = self.prepared_overlay.as_mut().expect("prepared above");
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
                if self.label_report != *prepared.label_report() {
                    self.label_report.clone_from(prepared.label_report());
                }
                self.overlay_fraction = Some(document.hour_fraction);
            }
            // Native windows keep ticking and drawing while active. Their clocks do not
            // change the hour/camera preparation, including the settled counter positions.
            if self.counter_view.presentation.take_running() {
                self.overlay_fraction = None;
                ui.ctx().request_repaint();
            }
            // A free eye shows the laid-out icons standing in the world; this frame's hits and
            // drawing follow what it shows.
            self.looks = if interface && overlay_camera.free().is_some() {
                let ground = free_icons::Ground {
                    field: document.assets.height_field(),
                    sight: document.assets.terrain_sight(),
                };
                Some(free_icons::present(
                    &overlay_camera,
                    pixels,
                    ui_scale,
                    Some(&ground),
                    self.overlay.as_ref().expect("drawn above"),
                    &mut self.free_overlay,
                ))
            } else {
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
                response.context_menu(|ui| {
                    let choices = now.vision(&perspective).and_then(|vision| {
                        viewer_counters::map::naval_selection::fleet_choices(
                            now.frame,
                            vision.ours(),
                        )
                    });
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
            // Resolve clicks against this hour and camera's newly rebuilt hit regions; the click
            // that chose a free mode's start selects nothing.
            if response.clicked()
                && !started
                && !over_combat
                && (!free_mode || icon_click)
                && let Some(point) = point
            {
                match self.map_hit(point) {
                    Some(MapHit::NavalMission(index))
                        if self.counter_report.naval_mission_hits[index].mission != 0 =>
                    {
                        let hit = &self.counter_report.naval_mission_hits[index];
                        let viewer = now
                            .vision(&perspective)
                            .map_err(|error| error.to_string())?
                            .ours();
                        let members = viewer_counters::map::naval_selection::mission_members(
                            now.frame,
                            viewer,
                            hit.region,
                            hit.mission,
                        )
                        .map_err(|error| error.to_string())?;
                        let selection = viewer_foundation::map::counter_hovers::unit_hits::Hit {
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
                            if self
                                .air_selection
                                .grid_click(now.frame, &members, shift)
                                .map_err(|error| error.to_string())?
                            {
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
                        let viewer = now
                            .vision(&perspective)
                            .map_err(|error| error.to_string())?
                            .ours();
                        let shift = ui.input(|input| input.modifiers.shift);
                        let target = viewer_counters::map::air_selection::base_target(
                            now.frame, viewer, province, kind,
                        )
                        .map_err(|error| error.to_string())?;
                        if let Some(target) = target {
                            self.choose_map_mode(viewer_world::map::map_mode::MapMode::Stock(
                                viewer_world::map::map_mode::Stock::StrategicAir,
                            ));
                            self.air_selection
                                .base_click(now.frame, target, shift)
                                .map_err(|error| error.to_string())?;
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
                        let viewer = now
                            .vision(&perspective)
                            .map_err(|error| error.to_string())?
                            .ours();
                        let members = viewer_counters::map::air_selection::mission_members(
                            now.frame,
                            viewer,
                            hit.region,
                            hit.mission.unwrap(),
                        )
                        .map_err(|error| error.to_string())?;
                        let shift = ui.input(|input| input.modifiers.shift);
                        if self
                            .air_selection
                            .mission_click(now.frame, &members, shift)
                            .map_err(|error| error.to_string())?
                        {
                            self.unit_selection.clear();
                            self.counter_view.selected = Arc::clone(self.unit_selection.units());
                            self.overlay_at = None;
                            ui.ctx().request_repaint();
                        }
                    }
                    Some(MapHit::AirMission(_)) => {}
                    Some(MapHit::Combat(index)) => self
                        .combat
                        .open(self.counter_report.combat_hits[index].selection),
                    target @ (Some(MapHit::Unit(_)) | None) => {
                        let hit = match target {
                            Some(MapHit::Unit(index)) => {
                                Some(&self.counter_report.unit_hits[index])
                            }
                            _ => None,
                        };
                        let shift = ui.input(|input| input.modifiers.shift);
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
                            ui.ctx().request_repaint();
                        }
                    }
                    _ => {}
                }
            }
            let overlay_ms = overlay_timer.elapsed().as_secs_f64() * 1000.0;
            overlay_draw_cpu.finish();
            laps.lap("overlay_draw");
            drop(_other_cost);
            let _other_cost = replay_frame::cpu::scope("frame.interface", Some("other"));
            let engaged = self
                .hours
                .as_ref()
                .map(|hours| hours.engaged(&perspective.country))
                .transpose()
                .map_err(|error| error.to_string())?;
            laps.lap("engaged");
            self.combat.set_playing(self.playing);
            let drawn = self
                .combat
                .draw(
                    &document.combat,
                    document.playback.branch(),
                    document.playback.hour(),
                    document.frame(),
                    extent,
                    &perspective,
                    engaged.as_ref().map(|index| index.shared.as_ref()),
                    self.counter_view.settings.nato,
                    true,
                    animation_time,
                    true,
                )
                .map_err(|error| error.to_string())?;
            if self.combat.gui_active() {
                ui.ctx().request_repaint();
            }
            laps.lap("combat_draw");
            let picker_drawn = match self.draw_picker(document, extent, true, animation_time, true)
            {
                Some(Ok(drawn)) => Some(drawn),
                Some(Err(error)) => {
                    self.picker_error = Some(error);
                    None
                }
                None => None,
            };
            if let Some(picker_drawn) = &picker_drawn {
                self.picker_error = None;
                self.picker_refused.clone_from(&picker_drawn.drawn.refused);
            }
            let over_picker = point.is_some_and(|point| {
                picker_drawn
                    .as_ref()
                    .is_some_and(|drawn| drawn.control_at(point).is_some())
            });
            let mut drawn = drawn;
            if let Some(picker_drawn) = &picker_drawn {
                drawn
                    .get_or_insert_with(viewer_combat::Drawn::default)
                    .add_hovers(&picker_drawn.drawn);
            }
            laps.lap("picker");
            let map_hit = self.map_tooltip(
                document,
                point.filter(|_| !over_combat && !over_picker),
                extent,
                ui.ctx(),
            );
            laps.lap("map_tooltip");
            let mut tooltip_refused = map_hit
                .as_ref()
                .map(|(text, _, _)| text.refused.clone())
                .unwrap_or_default();
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
                let refused = self
                    .prepared_overlay
                    .as_ref()
                    .expect("built above")
                    .draw_tooltip(&now, &mut additions, text, anchor, extent, animation_time)
                    .map_err(|error| error.to_string())?;
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
                && let crate::playback::Current::Frame(frame) = document.playback.current()
            {
                let batch = viewer_counter_products::map::counter_tooltips::Batch::new(
                    &document.counter_content,
                    document.icons.raids.movement_reads(),
                    frame,
                    &self.counter_report.perspective,
                    self.hover_style(),
                );
                let tooltips = self
                    .counter_report
                    .land_hovers
                    .iter()
                    .map(|hover| batch.build(hover))
                    .chain(
                        self.counter_report
                            .task_force_hovers
                            .iter()
                            .filter(|hover| hover.supported)
                            .map(|hover| batch.build_task_force(hover)),
                    );
                let tooltips = tooltips.chain(
                    self.counter_report
                        .task_force_port_hovers
                        .iter()
                        .filter(|hover| hover.supported && hover.has_tooltip())
                        .map(|hover| batch.build_task_force_port(hover)),
                );
                let refused =
                    viewer_counter_products::map::counter_tooltips::count_refusals(tooltips);
                for (reason, count) in refused {
                    let current = tooltip_refused.entry(reason).or_default();
                    *current = (*current).max(count);
                }
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
        let overlay = if self.looks.is_some() {
            &self.free_overlay
        } else {
            self.overlay.as_ref().expect("built above")
        };
        let mut combined;
        let label_batch = if additions.is_empty() {
            overlay
        } else {
            combined = overlay.clone();
            combined.quads.extend(additions.quads);
            &combined
        };
        // capture: at a fixed clock `Scene::render` takes the luminance
        // adaptation instantaneously, because the clock did not advance, which
        // is what a dump's own pair of frames does.
        let hovered_radar = point
            .filter(|_| !over_combat)
            .and_then(|point| radar_province(self.map_hit(point), &self.counter_report))
            .and_then(|province| {
                let row = now
                    .frame
                    .complete::<replay_data::rows::tables::Provinces>()
                    .ok()?
                    .iter()
                    .find(|row| row.id == province)?;
                Some((
                    row.state,
                    self.counter_report
                        .perspective
                        .province_controller(province)?,
                ))
            });
        let hovered_air_base = point.filter(|_| !over_combat).and_then(|point| {
            let MapHit::Base(index) = self.map_hit(point)? else {
                return None;
            };
            let (province, kind) = self.counter_report.air_base_hits.get(&index)?;
            let provinces = now
                .frame
                .complete::<replay_data::rows::tables::Provinces>()
                .ok()?;
            provinces
                .binary_search_by_key(province, |row| row.id)
                .ok()
                .map(|at| (provinces[at].state, *kind))
        });
        let hovered_air_carriers = point.filter(|_| !over_combat).and_then(|point| {
            let MapHit::Unit(index) = self.map_hit(point)? else {
                return None;
            };
            let hit = &self.counter_report.unit_hits[index];
            (!hit.port
                && hit.kind == viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce)
                .then(|| Arc::clone(&hit.members))
        });
        let target_light = self.frame_day_night(document.playback.hour());
        let day_night = self.day_night.update(
            document.playback.hour(),
            target_light,
            f64::from(animation_time),
            1.0 / self.speed,
        );
        let scene = self.scene.as_mut().unwrap();
        if let Some(prepared) = &self.prepared_overlay {
            let carrier_ranges = hovered_air_carriers
                .as_deref()
                .map(|forces| prepared.air_ranges.carrier_ranges(forces));
            let ranges = carrier_ranges
                .as_ref()
                .unwrap_or_else(|| prepared.air_ranges.shown(hovered_air_base));
            let (device, queue) = (&state.device, &state.queue);
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
                    SceneInput::SetRadar {
                        field: &prepared.radar,
                        shown: self.picker.radar,
                        hovered: hovered_radar,
                    },
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
            if let Some(base) = &self.uploaded.orders {
                let air = &prepared.air_orders;
                if self.uploaded.air_order_sources.as_ref().is_none_or(
                    |(had_base, had_air, highlights)| {
                        !Arc::ptr_eq(base, had_base)
                            || !Arc::ptr_eq(air, had_air)
                            || *highlights != ranges.highlighted_bases
                    },
                ) {
                    let mut field = (**base).clone();
                    field.draws.retain(|draw| !draw.air_mode);
                    let air_field = air.highlighted(&ranges.highlighted_bases);
                    let field = viewer_foundation::map::ribbon_mesh::append(&field, &air_field);
                    scene
                        .apply(
                            device,
                            queue,
                            SceneInput::SetOrders {
                                assets: &document.orders,
                                field: &field,
                            },
                        )
                        .map_err(|error| error.to_string())?;
                    self.uploaded.air_order_sources = Some((
                        Arc::clone(base),
                        Arc::clone(air),
                        ranges.highlighted_bases.clone(),
                    ));
                }
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
        crate::document::select_movement(
            scene,
            &state.device,
            &state.queue,
            &document.content,
            document.frame().map_err(|error| error.to_string())?,
            self.overlay_tables.as_ref().expect("parsed above").3.ours(),
            self.unit_selection.units().as_ref(),
        )
        .map_err(|error| error.to_string())?;
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
        scene
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
                        strategic_hover,
                        sprites: interface.then_some(label_batch),
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
            )
            .map_err(|e| e.to_string())?;
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
                    music: self.music.clone(),
                    battles: self.battles.clone(),
                },
            );
        } else if !self.audio_focus
            && let Some(audio) = &mut self.audio
        {
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
            profile_print(format!(
                "PROFILE frame {} fraction {} height {} total {:.6} overlay {:.6} spline {:.6} hour {} frame_total {:.3} outside {:.3} scene {:.3}/{:.3}/{:.3}/{:.3} gpu {:.3} laps {} detail {} spans {} thread {} cpu_hz {} {} {}",
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
            ));
        }
        // A capture waits until every flag and portrait its frame asked for
        // has decoded and been laid out, as a headless dump does. The overlay's own layout version
        // is the reference, not a version read here: a background load that landed after the
        // layout but before this check would otherwise let the frame save without it.
        if let Some(capture) = self.capture.as_mut() {
            // Anything the settle decodes moves the version, which makes the overlay stale.
            document.icons.lazy.settle();
            let laid_out = self
                .prepared_overlay
                .as_ref()
                .is_some_and(|prepared| !prepared.is_stale(&now));
            // And until the raid offer cycle is complete at the shown hour:
            // every target evaluated, as a paused frame ends.
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
            if let Some(music) = &self.music
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
        // The water animates, so the window keeps drawing.
        ui.ctx().request_repaint();
        self.drew_frame = true;
        self.rendered = Some(Playhead {
            branch: document.playback.branch(),
            hour: document.playback.hour(),
            fraction: f64::from(document.hour_fraction),
        });
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
struct AudioInputs {
    volumes: dashboard::protocol::MapAudio,
    muted: bool,
    /// A presentation discontinuity since the last audio frame.
    reset: bool,
    music: Option<Arc<viewer_audio::Chances>>,
    battles: Option<Arc<viewer_audio::Battles>>,
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
        voice: percent(volumes.voice),
        music: percent(volumes.music),
        muted,
    }
}

fn present_audio(
    audio: &mut Option<viewer_audio::Owner>,
    document: &Document,
    scene: &Scene,
    camera: &Camera,
    inputs: AudioInputs,
) {
    let AudioInputs {
        volumes,
        muted,
        reset,
        music,
        battles,
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
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.options.window_dump {
            // Dumps keep window geometry, but never consume live input.
            input.events.clear();
            input.events.push(egui::Event::PointerGone);
            input.hovered_files.clear();
            input.dropped_files.clear();
            input.focused = false;
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.embedded_logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
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
            // A hidden map stops sounding.
            self.release_audio();
            if let Some(dashboard) = &mut self.dashboard {
                dashboard.release_audio();
            }
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
        } else if let Some(dashboard) = &mut self.dashboard {
            // Nor are dashboard maps while the Campaign map shows, loaded or not.
            dashboard.release_audio();
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
    fn viewing(&self) -> Option<&Arc<hours::ViewingCountry>> { None }
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
    fn viewing(&self) -> Option<&Arc<hours::ViewingCountry>> { self.viewing.as_ref() }
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
/// hour of its branch.
fn presentation_matches(rendered: Playhead, target: Playhead, playing: bool) -> bool {
    rendered == target
        || (playing && rendered.branch == target.branch && rendered.hour <= target.hour)
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
            pending: None,
            political_warnings: Vec::new(),
            plan_warnings: Vec::new(),
            building_warnings: Vec::new(),
            model_warnings: Vec::new(),
            political_summary: (0, 0),
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
        use replay::{container::Replay, spool::Record};
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
    fn credit_pressure_names_the_refusal_and_keeps_a_window_capture_armed_until_recovery() {
        let failure = Arc::new(Mutex::new(None));
        let mut viewer = Viewer::create(
            Config::default(),
            Options {
                out: Some("pressure.png".into()),
                window_dump: true,
                ..Default::default()
            },
            None,
            Arc::clone(&failure),
            None,
            None,
        );
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            viewer.hour_error(
                Some(&std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "display change-fact working set remains pinned; common paging is required",
                )),
                ui.ctx(),
            );
        });
        output.textures_delta.clear();
        assert_eq!(
            viewer.map_error.as_deref(),
            Some("display change-fact working set remains pinned; common paging is required")
        );
        assert!(
            viewer
                .capture
                .as_ref()
                .is_some_and(|capture| capture.exit && capture.path == Path::new("pressure.png"))
        );
        assert!(failure.lock().unwrap().is_none());
        assert!(!output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        }));
        viewer.hour_error(None, &ctx);
        assert!(viewer.map_error.is_none());
        assert!(viewer.capture.is_some());
        assert!(failure.lock().unwrap().is_none());
        let mut output = ctx.run_ui(Default::default(), |ui| {
            viewer.hour_error(
                Some(&std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "hour was not recorded",
                )),
                ui.ctx(),
            );
        });
        output.textures_delta.clear();
        assert!(viewer.capture.is_none());
        assert_eq!(
            failure.lock().unwrap().as_deref(),
            Some("Capture pressure.png failed: hour was not recorded")
        );
        assert!(output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        }));
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
        let labels = |replay: &replay::container::Replay| {
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
        use replay::{container::Replay, spool::Record};
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
