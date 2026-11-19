#[cfg(test)]
pub(crate) use viewer_playback::presentation::MAP_LINES_LOADING;
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
        render,
    },
    viewer_foundation::map::{
        camera::{Camera, MapSize},
        frame::{DayNight, GameSpeeds},
    },
};
// names
use eframe::{egui, wgpu};
use std::{
    collections::BTreeMap,
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


pub(crate) mod head_rows;
pub(crate) mod highlight;
pub(crate) mod laid_proof;
pub(crate) mod map_hits;
mod hover_report;
pub(crate) mod hover_rows;
pub(crate) mod fly;
mod fly_aircraft;
pub(crate) mod picker;
pub(crate) mod playback_cost;
mod presentation;
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
mod camera_mode_tests;
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

/// The most times a stopped picture lays its hour out again over art that decoded while it drew
/// ([`Viewer::draw_map`]'s settle): art arrives because a layout asked for it, so each round asks
/// for less, and art still arriving past this is refused by name rather than drawn incomplete.
const SETTLES: u32 = 8;

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

/// The camera, viewport and interface scale a frame drew at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    pub camera: Camera,
    pub viewport: [u32; 2],
    /// Device pixels per interface pixel; see [`crate::map::ui_scale`].
    pub ui_scale: f32,
}

/// Where the host's clock is: the recorded hour it is in and how far past it.
///
/// The map's layers draw the newest hour they have composed, this one unless a seek or fast
/// playback outran them; the playhead never waits for the map.
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

/// What the scene's order parts were last given from, and the air orders as last highlighted, so
/// a frame hands it the parts only when one of them changed.
#[derive(Clone, Default)]
struct Uploaded {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapHit {
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

/// The map icon under `point`: the top of the drawn hit shapes, by layer and pass. `raids` are
/// the drawn raid icons' hit shapes. `looks` are the icons a free eye shows: the nearest shown
/// icon whose own layout holds the pointer wins, and icons it does not show take no hits.
///
/// One point against one frame's shapes; a frame resolves each point it asks about once against
/// the index it keeps ([`map_hits::Resolved`]), which is what this takes for a single point.
#[cfg(test)]
fn top_map_hit(
    point: [f32; 2],
    report: &viewer_counters::map::counters::CounterReport,
    raids: &[viewer_raids::map::raids::tooltips::Hit],
    occupation: &[viewer_world::map::occupation_icons::Hit],
    looks: Option<&free_icons::Looks>,
    intel: Option<&viewer_raids::map::intel_icons::IntelHit>,
) -> Option<MapHit> {
    let shapes = map_hits::Shapes { report, raids, occupation, adjacency: &[], looks, shifts: None };
    map_hits::Index::of(&shapes).top(point, &shapes, intel)
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
    /// The worker threads a hosted map shares with the rest of the app.
    shared_pool: Option<Arc<crate::worker_pool::WorkerPool>>,
    source: Option<Arc<dashboard::Source>>,
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
    /// The harness measuring the frames the app's map tab runs (`--campaign-tab`): its frames go
    /// through the window's own path and present their audio to a silent system.
    pub(crate) campaign_tab: bool,
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
    /// Whether every figure is posed on the frame that draws it ([`Scene::settle_poses`]), as
    /// this window last told the scene: a tool that saves every frame it draws sets it for the
    /// whole run, so the window leaves it alone while the hours run.
    settled_poses: bool,
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
    free_presented: Option<(viewer_foundation::map::camera::Camera, [u32; 2], u32, u64, viewer_foundation::map::anchored::Head)>,
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
    /// The page's clock as this map sees it, which places each anchor by the time it was composed.
    page_clock: crate::page_clock::PageClock,
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
    /// Where the display frames of this map stand in the replay: the one head everything a frame
    /// draws is placed at, steered to stay inside the hours published as real data
    /// ([`viewer_foundation::map::frame_head`]).
    frame_head: viewer_foundation::map::frame_head::FrameHead,
    /// A seek waiting for the next built hour to start a fresh animation
    /// history (`viewer_foundation::map::presentation`).
    animation_seek: bool,
    /// The last host clock target, to tell its seeks from playback.
    map_animation_target: Option<Target>,
    /// The map's reader of the playhead of the recording on screen; none until it is made, or
    /// once it was refused ([`Viewer::playhead_refused`]).
    playhead_map: Option<crate::playhead_host::side::SideThread>,
    /// Why the map's reader was refused, by name: the map draws nothing without it.
    playhead_refused: Option<String>,
    /// A dashboard map opened its playhead reader since it was installed or last not drawn.
    playhead_opened: bool,
    /// A seek the page asked for that no reader has taken: an anchor reaches none while the map's
    /// reader is closed or still being made, and the playhead outlives the reader, so its clock
    /// stands where it stood. It stays owed until a reader takes it, at the hour the page's clock
    /// stands at then.
    seek_owed: bool,
    /// The rows at the head (hour, frame) the overlay on screen was laid out from; none before
    /// its first layout over them.
    /// The hour of the rows the overlay on screen was made from, as the side made it.
    overlay_hour: Option<replay_data::Hour>,
    /// The head tables (their rows' hour and frame) last given to the counter layers, and those
    /// they held when the reader last drew: what its settled picture shows of them.

    /// The document on screen has shown a whole picture since it was installed; until then the
    /// spinner covers what is drawn beneath it.
    shown_whole: bool,
    /// The map's reader being made on a worker; today's picture is drawn until then.
    playhead_opening: Option<crate::playhead_host::Opening>,
    /// What the window last presented of the playhead's map, for its own histories.
    presented: MapPresented,
    /// The camera, viewport and interface scale the last frame drew at.
    last_view: Option<View>,
    /// What the scene's order parts hold.
    uploaded: Uploaded,
    /// Frees the hours the window let go of off the frame path.
    dropper: Option<Arc<Dropper>>,
    /// The app's worker threads: the host's shared ones, else this window's own once first asked.
    worker_pool: Option<Arc<crate::worker_pool::WorkerPool>>,
    // close zoom
    /// The playback fraction the scene's raid units were last placed at.
    raid_tick: Option<f32>,
    /// Whether the scene's raid units last included sailing trade-route convoys, which move
    /// every frame by the presentation clock while they show.
    convoys_sailing: bool,
    /// The convoy entities that did not bake, as of the last frame that sailed them.
    convoy_refused: BTreeMap<String, usize>,
    // names
    /// The names layer's placements, which the frame draws and the warnings read.
    placements: Option<Arc<Placements>>,
    /// `None` until the first frame seeds it settled at that frame's own camera
    /// height; see [`Viewer::map`].
    fade: Option<LayerFade>,
    hovered_name_region: Option<u32>,
    last_frame: Instant,
    camera_step: Option<f32>,
    label_report: LabelReport,
    faction_highlight: faction_highlight::Lane<faction_highlight::Prepared>,
    hover_rows: hover_rows::HoverRows,
    /// The selection observers, the overlay's camera-bound inputs, the walk camera, the clock's
    /// speed and the picker, over rows at the head while the map's playhead is on.
    head_rows: head_rows::HeadRows,
    /// A faction highlight was shown or asked for at the last frame: the side's window copies the
    /// picture it is built over only then.
    highlight_wanted: bool,
    /// With the playhead's map open: the rows at the head the overlay was last made from, kept
    /// while its reader loads, and the build of its tables from them.
    overlay_frame: Option<Arc<replay_frame::frame::DecodedFrame>>,
    /// The hours of the publication the overlay the frame holds was made for at which the
    /// recording moved a mover: the frame places that layout, so it is where every mover's places
    /// are read ([`viewer_foundation::map::counter_move::MovedHours`]).
    overlay_moved: Arc<viewer_foundation::map::counter_move::MovedHours>,
    /// The tables the overlay on screen was made from, as the side made them.
    overlay_tables: Option<Arc<crate::document::OverlayTables>>,
    /// The tables' route cache is filled ahead on a worker once, for each tables made by key.
    overlay_routes_pending: bool,
    /// What the overlay on screen leaves to the map's layers, as it was laid out.
    overlay_left: crate::document::LeftToLayers,
    /// The map's layers' products the overlay the frame holds laid in, as it was made with them:
    /// the gate lays these into the layout it compares that one with.
    overlay_lay_ins: Option<std::sync::Arc<crate::playhead_host::side::LayIns>>,
    /// The counters' memory the tables' worker lays the overlay out on, over the view's schedule and
    /// movement ([`crate::map_views::Presentation::sharing`]), for the view it was made from.
    /// The counter layers' products and drawn kinds the overlay shown was laid out from
    /// ([`Self::install_overlay`]); none before the first layout.
    overlay_layered: Option<(viewer_counters::map::counters::Layered, viewer_counters::map::counters::Kinds)>,
    /// What the overlay laid out last for the open map was laid out for ([`Self::install_overlay`]).
    /// The branch and hour of the overlay on screen, and the view it was laid out for.
    overlay_shown: Option<(replay_data::BranchId, replay_data::Hour)>,
    overlay_view: Option<CounterView>,
    /// Where the camera stood when the overlay on screen was last placed;
    /// a frame whose camera stands elsewhere places it again and lays nothing out.
    overlay_placed: Option<Camera>,
    /// The frame size and interface scale the overlay was placed for: a window's layout is made
    /// for them, so only a camera moving over that same view may be answered by a move.
    overlay_placed_view: Option<([u32; 2], u32)>,
    /// While the camera moves over the map, how far each of the overlay's anchors stands from
    /// where [`Self::overlay_placed`] put it: the frame offsets the windows instead of placing
    /// and drawing them again ([`viewer_foundation::map::anchored`]). Empty while the overlay
    /// stands at the camera it was placed at.
    overlay_shifts: viewer_foundation::map::anchored::Shifts,
    /// The camera the frame before drew at: a camera that has come to rest places the overlay
    /// again, so a still picture is the layout's own and never a moved one.
    overlay_camera_before: Option<Camera>,
    /// The head the frame before moved the overlay's windows to: a head that advanced places every
    /// mover the batch carries at its own hour.
    overlay_head_before: Option<viewer_foundation::map::anchored::Head>,
    /// The painting before, when the count of what each publication changes was asked for
    /// (`BATTLE_PLAN_VIEWER_COUNT_OVERLAY`).
    overlay_counted: viewer_foundation::map::overlay_count::Watch,
    /// A publication's overlay is installed but its batch has not been painted yet: the batch the
    /// frame holds is the painting before it, standing at the camera *it* was placed at, so it is
    /// not moved to this frame's camera. The frame places and paints the new overlay instead, and
    /// every later move is measured from that painting.
    overlay_unpainted: bool,
    /// The hour fraction the overlay was last drawn at.
    overlay_drawn_fraction: Option<f32>,
    /// The head tables' lookups by key counted so far: (tables, made, kept).
    overlay_lookups: Option<(usize, u64, u64)>,
    /// Overlays installed from the map's side, and the one the overlay's gate last looked at: the
    /// gate lays each hour out afresh once, so a still camera is covered too.
    overlay_installed: u64,
    overlay_checked: u64,
    /// Looks the overlay's gate has taken, for the one in ten that compares a frame's kept windows
    /// with a whole repaint's.
    overlay_looks: u64,
    /// The overlay's reads that its rows at the head do not hold yet, by name.
    overlay_loading: Vec<String>,
    /// The tables the overlay's last layout read that its rows did not hold, until one is made.
    overlay_missed: Vec<replay_data::rows::TableId>,
    combat_rows: combat_rows::CombatRows,
    tooltip_refused: BTreeMap<String, usize>,
    // counters
    counter_report: CounterReport,
    /// The pointer resolved against the frame's hit shapes, once for each point a frame asks
    /// about ([`map_hits`]); `hits_stamp` names the shapes it was resolved against.
    map_hits: std::cell::RefCell<map_hits::Resolved>,
    /// Bumped whenever the hit shapes change: once a frame, after the overlay's draw and the free
    /// eye's placing, and whenever the map's counters are let go.
    hits_stamp: u64,
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
    /// The families of map-icon windows that asked this frame for a pass of their own
    /// ([`crate::map_views::Running::windows`]): a frame that presents the pools for their passes
    /// alone presents these and no others.
    overlay_pools: viewer_foundation::map::icon_windows::Pools,

    /// This view's counter options (the user's `settings.txt`, with the host's graphics
    /// overrides) and its counters' memory from one update to the next; a new document
    /// starts it afresh.
    counter_view: CounterView,
    unit_selection: viewer_counters::map::unit_selection::Selection,
    air_selection: viewer_counters::map::air_selection::Selection,
    fleet_selection: viewer_counters::map::naval_selection::Selection,
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
    /// The overlay the frame shows against the same hour, view and camera laid out whole
    /// (`BATTLE_PLAN_VIEWER_CHECK_OVERLAY`), quad for quad with the clock set aside and with both
    /// reports: the gate on an overlay placed at this camera rather than laid out at it, and on
    /// raid icons put in place. `what` names the case in the refusal.
    ///
    /// Each is drawn on a presentation of its own, with no window history and no unit animation,
    /// so a difference is a difference of the layouts and neither draw moves what the frame
    /// shows. The gate does overwrite the shown overlay's own hit shapes, so a run with it on is
    /// a run for the gate alone.
    #[allow(clippy::too_many_arguments, reason = "the frame's inputs the gate lays an hour out and draws with")]
    fn check_overlay(
        &mut self,
        what: &str,
        now: &crate::document::Hour<'_>,
        camera: Camera,
        pixels: [u32; 2],
        ui_scale: f32,
        time: f32,
    ) -> Result<(), String> {
        let (Some(tables), true) = (self.overlay_tables.clone(), self.prepared_overlay.is_some()) else {
            overlay_checks::note(&overlay_checks::NO_OVERLAY);
            return Ok(());
        };
        // Art that decoded since the overlay was laid out moves where a sprite sits in its sheet:
        // the two layouts would then draw one sprite at two versions, which is not a difference of
        // layouts. The side lays such a layout out again; this one is not compared.
        if self.prepared_overlay.as_ref().is_some_and(|prepared| prepared.is_stale(now)) {
            overlay_checks::note(&overlay_checks::ART_MOVED);
            profile_print(format!("CHECK overlay {what} not compared: the art moved since it was laid out"));
            return Ok(());
        }
        // With the map open, the rows at the head the overlay was laid out from, and what that
        // layout left to the map's layers: a later step's answer belongs to a later layout.
        let side = live_of(&self.playhead_map);
        let left = self.overlay_left;
        let rows = crate::document::OverlayRows::at_head(self.overlay_frame.clone().filter(|_| side.is_some())).leaving(left);
        let apart = || {
            let presentation = self.counter_view.presentation.snapshot();
            self.counter_view.presentation.lend_layered(&presentation);
            CounterView { presentation: Arc::new(presentation), ..self.counter_view.clone() }
        };
        let (for_shown, for_fresh) = (apart(), apart());
        let layers = self.overlay_layers.clone();
        let held = self.counter_view.clone();
        let full = now.prepare_overlay_at(camera, pixels, ui_scale, &tables, &for_fresh, &rows);
        if !rows.missed().is_empty() {
            overlay_checks::note(&overlay_checks::ROWS_MISSED);
            profile_print(format!(
                "CHECK overlay {what} not compared: the rows the overlay was made from hold none of {:?}",
                rows.missed()
            ));
            return Ok(());
        }
        let mut full = full.map_err(|error| format!("the overlay laid out afresh for the gate: {error}"))?;
        // A map layer publishing its own product of the hour between the painting and this layout
        // makes two layouts of two pictures: that product's combats, stacks and gaps are its own.
        // The side lays such a layout out again; this one is not compared.
        if let Some(moved) = laid_proof::layers_moved(
            self.prepared_overlay.as_ref().expect("held above").layers(),
            full.layers(),
        ) {
            overlay_checks::note(&overlay_checks::LAYERS_MOVED);
            profile_print(format!("CHECK overlay {what} not compared: the layers' product moved since it was laid out ({moved})"));
            return Ok(());
        }
        // The products the painted layout laid in, not the side's newest: a layout made with
        // another layer's product is another layout, and the gate is about the layouts.
        if let Some(lay_ins) = self.overlay_lay_ins.clone() {
            lay_ins.lay(&mut full, now, camera, pixels, ui_scale, tables.ours());
        }
        let draw = |prepared: &mut crate::document::PreparedOverlay| {
            let (mut batch, mut report) = (Batch::default(), CounterReport::default());
            prepared
                .draw_into(now, camera, pixels, ui_scale, &layers, time, &mut batch, &mut report)
                .map(|()| (batch, report, prepared.label_report().clone(), prepared.raid_report().clone()))
        };
        // Each side is drawn on a presentation whose map-icon pools start empty, and the pools
        // make the windows a draw asks for over the draws after it: a layout drawn once and the
        // same layout drawn until its windows are made are two pictures, and the gate is about
        // the layouts. So each side is drawn until its own pools have made its windows, within
        // their growth rounds, as a stopped picture's settle draws ([`Viewer::draw_map`]).
        let settle = |prepared: &mut crate::document::PreparedOverlay| {
            let mut rounds = viewer_foundation::map::icon_windows::DumpFrames::default();
            let mut drawn = draw(prepared);
            while drawn.is_ok() && rounds.next(&prepared.retries()) {
                drawn = draw(prepared);
            }
            drawn
        };
        let shown = {
            let prepared = self.prepared_overlay.as_mut().expect("held above");
            prepared.set_counter_view(for_shown);
            let drawn = settle(prepared);
            prepared.set_counter_view(held);
            drawn
        };
        let fresh = settle(&mut full);
        // Art the gate's own layout queued may decode while it draws: its version moved during the
        // comparison, which is again not a difference of layouts.
        if self.prepared_overlay.as_ref().is_some_and(|prepared| prepared.is_stale(now)) {
            overlay_checks::note(&overlay_checks::ART_MOVED);
            profile_print(format!("CHECK overlay {what} not compared: the art moved while it was compared"));
            return Ok(());
        }
        overlay_checks::note(&overlay_checks::COMPARED);
        let mut found = Vec::new();
        match (shown, fresh) {
            (Ok(shown), Ok(fresh)) => {
                if let Some(difference) = laid_proof::first_difference(&laid_proof::clockless(&shown.0), &laid_proof::clockless(&fresh.0)) {
                    found.push(difference);
                }
                if shown.1 != fresh.1 {
                    found.push(laid_proof::first_report_difference("counters' reports", &shown.1, &fresh.1));
                }
                if shown.2 != fresh.2 {
                    found.push(laid_proof::first_report_difference("labels' reports", &shown.2, &fresh.2));
                }
                if shown.3 != fresh.3 {
                    found.push(laid_proof::first_report_difference("raid icons' reports", &shown.3, &fresh.3));
                }
            }
            (shown, fresh) => {
                if shown.is_err() != fresh.is_err() {
                    found.push(format!("one draws and the other is refused (shown: {}, afresh: {})", shown.is_ok(), fresh.is_ok()));
                }
            }
        }
        // Which publication each side read: a difference between a layout and the tables the
        // frame holds is a pairing broken, not two layouts of one publication differing.
        let pairing = format!(
            "laid out over the tables of build {} and placed at {:?}, the frame holds build {} and draws at {:?}",
            self.prepared_overlay.as_ref().map_or(0, crate::document::PreparedOverlay::tables),
            self.overlay_placed.map(|placed| (placed.center, placed.height)),
            tables.identity(),
            (camera.center, camera.height)
        );
        profile_print(format!(
            "CHECK overlay {what} {}",
            if found.is_empty() { "equal".to_owned() } else { format!("DIFFER ({pairing}): {}", found.join("; ")) }
        ));
        if found.is_empty() {
            Ok(())
        } else {
            Err(format!("the overlay {what} differs from the same hour, view and camera laid out afresh ({pairing}): {}", found.join("; ")))
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
            match crate::worker_pool::WorkerPool::new() {
                Ok(pool) => {
                    viewer.dropper = Some(Arc::clone(pool.dropper()));
                    viewer.worker_pool = Some(Arc::clone(&pool));
                    viewer.shared_pool = Some(pool);
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
            shared_pool: None,
            source: None,
            loaded_config: None,
            rendered: None,
            rendered_sequence: 0,
            drawn_at: None,
            applying_options: None,
            drawn_options: None,
            hover: None,
            tooltip: None,
            tooltip_delay_ms: None,
            perf_view: None,
            map_warnings: Vec::new(),
            diagnostics: options.diagnostics,
            diagnostics_open: false,
            audio: None,
            audio_focus: true,
            campaign_tab: false,
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
            settled_poses: false,
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
            page_clock: Default::default(),
            day_night: Default::default(),
            last_step: Instant::now(),
            profile_playback: Default::default(),
            started: Instant::now(),
            opened: None,
            playhead: None,
            frame_head: Default::default(),
            animation_seek: false,
            map_animation_target: None,
            playhead_map: None,
            playhead_opened: false,
            seek_owed: false,
            overlay_hour: None,
            shown_whole: false,
            playhead_opening: None,
            playhead_refused: None,
            presented: MapPresented::default(),
            last_view: None,
            uploaded: Uploaded::default(),
            dropper: None,
            worker_pool: None,
            // close zoom
            raid_tick: None,
            convoys_sailing: false,
            convoy_refused: BTreeMap::new(),
            // names
            placements: None,
            fade: None,
            hovered_name_region: None,
            last_frame: Instant::now(),
            camera_step: None,
            label_report: LabelReport::default(),
            faction_highlight: Default::default(),
            hover_rows: Default::default(),
            head_rows: Default::default(),
            highlight_wanted: false,
            overlay_frame: None,
            overlay_moved: Arc::default(),
            overlay_tables: None,
            overlay_routes_pending: false,
            overlay_left: Default::default(),
            overlay_lay_ins: None,
            overlay_shown: None,
            overlay_view: None,
            overlay_placed_view: None,
            overlay_head_before: None,
            overlay_counted: Default::default(),
            overlay_unpainted: false,
            overlay_shifts: Default::default(),
            overlay_camera_before: None,
            overlay_placed: None,
            overlay_layered: None,

            overlay_drawn_fraction: None,
            overlay_lookups: None,


            overlay_installed: 0,
            overlay_checked: 0,
            overlay_looks: 0,
            overlay_loading: Vec::new(),
            overlay_missed: Vec::new(),
            combat_rows: Default::default(),
            tooltip_refused: Default::default(),
            // counters
            counter_report: CounterReport::default(),
            map_hits: Default::default(),
            hits_stamp: 0,
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
            overlay_pools: Default::default(),
            overlay_fraction: None,

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
        pool: Arc<crate::worker_pool::WorkerPool>,
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
        viewer.dropper = Some(Arc::clone(pool.dropper()));
        viewer.worker_pool = Some(Arc::clone(&pool));
        viewer.shared_pool = Some(pool);
        viewer
    }

    /// The app's worker threads: the host's shared ones, else this window's own, made on first
    /// use. Its dropper frees what the window retires.
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
        self.playhead_refused = None;
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
        let shown = self.playhead_map.as_ref().and_then(|side| {
            side.on_screen()
                .filter(|on| on.settled)
                .and_then(|on| Some((side.branch().unwrap_or(requested.0), on.drawn?, self.map_error.is_some())))
        });
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
        let plan = side.products().plan.as_ref().map(|(_, plan)| Arc::clone(plan));
        let routes = side.products().routes.as_ref().map(|(seq, _)| *seq);
        let raids = self.overlay_tables.as_ref().map(|tables| Arc::clone(tables.raid_arrows()));
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
            self.overlay_tables.take(),
            self.prepared_overlay.take(),
            std::mem::take(&mut self.faction_highlight),
            permit,
        );
        if let Some(dropper) = &self.dropper {
            dropper.drop_later(retired);
        }
        self.overlay_hour = None;
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
        self.frame_head.forget();
        self.playhead_opening = None;
        self.playhead_refused = None;
        self.hover_rows.end_frame(false);
        self.hover_rows.end_frame(false);
        self.playhead_opened = false;
    }

    fn drop_later(&self, side: crate::playhead_host::side::SideThread) {
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
        self.loaded_config = None;
        self.revision = None;
        self.expected_revision = None;
        self.map_preferences_seen = None;
        self.playhead = None;
        self.seek_owed = false;
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
                Some(target.composed_at),
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
        if let Some(side) = &self.playhead_map {
            side.entry().follow_rate();
        }
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
        composed_at: Option<f64>,
        now: Instant,
        last: i32,
    ) {
        let anchor = (target, playing, speed);
        self.seek_owed |= sought;
        if sought {
            // A seek is answered on the next display frame: the head stands at the hour sought,
            // exactly, rather than being steered from an hour of the history it left.
            self.frame_head.forget();
        }
        // An anchor the clock already holds changes nothing, unless a seek is still owed: the
        // reader that takes it is the one this anchor reaches.
        if !sought && !self.seek_owed && self.playhead.is_some() && self.clock_anchor == Some(anchor) {
            return;
        }
        if self.capture.is_none() {
            self.tick_playhead(now.duration_since(self.last_step).as_secs_f64(), last);
        }
        self.last_step = now;
        // A seek and a pause stand where the page composed them; a playing anchor has moved on
        // since, and never takes the map to an hour before the one it shows.
        let moving = playing && !sought;
        let age = self.page_clock.age(composed_at, now);
        self.playhead = Some(forward_only(
            self.playhead,
            placed_anchor(target, moving, speed, age, last),
            moving,
        ));
        self.speed = speed;
        self.playing = playing && self.playhead.is_some_and(|head| head.hour < last);
        self.clock_anchor = Some(anchor);
        self.follow_branch();
        self.give_anchor(target.hour, playing, speed);
    }

    /// Hands the page's newest anchor to the map's reader, seeking to `hour` while a seek no
    /// reader has taken is owed. A closed or opening reader takes nothing: the seek stays owed and
    /// is taken, exact at the hour the page's clock stands at then, by the reader that opens
    /// ([`Self::side_opened`]) or by the page's next anchor.
    fn give_anchor(&mut self, hour: i32, playing: bool, speed: f64) {
        let Some(side) = &self.playhead_map else { return };
        let sought = std::mem::take(&mut self.seek_owed);
        if sought {
            self.presented.seek = side.on_screen().map(|on| on.resets);
        }
        side.anchor(hour, playing, speed, sought);
    }

    /// The map's reader is open: it takes the page's newest anchor at once, so a seek the page
    /// asked for while no reader was there is never lost.
    fn side_opened(&mut self, side: crate::playhead_host::side::SideThread) {
        self.playhead_map = Some(side);
        if let Some(clock) = self.playhead {
            self.give_anchor(clock.hour, self.playing, self.speed);
        }
    }

    /// Whether this frame is saved — a stepped capture's frame or a window dump's — and so is
    /// whole at its own hour or refused by name, however long it takes to make.
    fn saves_a_frame(&self) -> bool {
        self.capture.as_ref().is_some_and(|capture| !capture.paced) || self.options.window_dump
    }

    /// Installs the overlay the map's side made for its step, with the drawn plates of the one it
    /// replaces when that was laid out for the same view and their stacks are the same by value.
    /// Its counters' memory is already the view's: the frame took it in the window, where the side
    /// writes nothing.
    fn install_overlay(&mut self, made: crate::playhead_host::overlay::Made) {
        let crate::playhead_host::overlay::Made { hour, branch, mut prepared, tables, rows, placed_at, on: _, published, draws, view, left, lay_ins, moved } = made;
        self.overlay_moved = moved;
        if let Some(old) = self.prepared_overlay.as_mut()
            && self.overlay_view.as_ref() == Some(&view)
        {
            prepared.carry_plates(old);
        }
        prepared.set_counter_view(self.counter_view.clone());
        let retired = (self.prepared_overlay.replace(prepared), self.overlay_tables.replace(tables), self.overlay_frame.replace(rows));
        self.retire(retired);
        self.overlay_routes_pending = true;
        self.overlay_left = left;
        self.overlay_lay_ins = Some(lay_ins);
        self.overlay_layered = Some((published, draws));
        self.overlay_view = Some(view);
        self.overlay_shown = Some((branch, hour));
        self.overlay_placed = Some(placed_at);
        self.overlay_fraction = None;
        self.overlay_unpainted = true;
        self.overlay_installed += 1;
        viewer_foundation::map::redraws::note("made.laid", 1);
    }

    /// The overlay the frame holds placed at `camera`: a layout stands for an hour and a view and
    /// is placed at any camera ([`crate::document::PreparedOverlay::project`]). Whether it moved.
    #[allow(clippy::too_many_arguments, reason = "the frame's camera and what the placing reads")]
    fn place_overlay(
        &mut self,
        now: &crate::document::Hour<'_>,
        camera: Camera,
        pixels: [u32; 2],
        ui_scale: f32,
        tables: &crate::document::OverlayTables,
        rows: &Arc<replay_frame::frame::DecodedFrame>,
        left: crate::document::LeftToLayers,
    ) -> bool {
        if self.overlay_placed == Some(camera) {
            return false;
        }
        let Some(prepared) = self.prepared_overlay.as_mut() else {
            return false;
        };
        let read = crate::document::OverlayRows::at_head(Some(Arc::clone(rows))).leaving(left);
        let placed = match prepared.project(now, camera, pixels, ui_scale, tables, &self.counter_view, &read) {
            Ok(()) => {
                viewer_foundation::map::redraws::note("made.placed", 1);
                true
            }
            Err(error) => {
                self.overlay_loading.push(format!("the overlay: {error}"));
                false
            }
        };
        self.overlay_placed = Some(camera);
        self.overlay_placed_view = Some((pixels, ui_scale.to_bits()));
        self.overlay_fraction = None;
        self.overlay_shifts.clear();
        placed
    }

    /// The painting this frame just made, placed at this frame's camera and head before it is
    /// drawn. A painting draws the layout it was made from where that layout was placed
    /// ([`Self::overlay_placed`]) and at the hour the publication was laid out for, and neither is
    /// this frame's: a frame that answered its camera by moving the windows lays nothing out, and
    /// the pools can still ask it to paint; and a publication is prepared ahead of the head. So
    /// the fresh painting is offset to this frame's own camera and head, by the mechanism every
    /// other frame uses. Refused, the painting stands where its layout was placed and the frame
    /// after this one places it.
    fn place_painting(
        &mut self,
        camera: Camera,
        pixels: [u32; 2],
        ui_scale: f32,
        at: viewer_foundation::map::anchored::Head,
    ) -> Result<(), String> {
        let Some(placed) = self.overlay_placed else {
            return Ok(());
        };
        if placed == camera && !self.overlay.as_ref().is_some_and(Batch::moves) {
            return Ok(());
        }
        let extent = viewer_foundation::map::gui_resources::viewport(pixels, ui_scale);
        let _span = viewer_foundation::profiling::span("overlay_painting_placed");
        let batch = self.overlay.as_mut().expect("the batch this frame painted");
        let Ok(moved) = self.overlay_shifts.move_to(batch, &placed, &camera, pixels, extent, at, check_anchor()) else {
            return Ok(());
        };
        viewer_foundation::map::redraws::note("made.painting_placed", 1);
        viewer_foundation::map::redraws::note("moved.movers", moved.movers as u64);
        viewer_foundation::view_record::add("overlay", "movers_placed", moved.movers as u64);
        viewer_foundation::view_record::add("overlay", "quads_moved", moved.quads as u64);
        for (name, value) in moved.named() {
            viewer_foundation::view_record::add("overlay", name, value);
        }
        self.hits_stamp += 1;
        if !self.overlay_shifts.found().is_empty() {
            return Err(format!(
                "the overlay's painting does not stand where this frame's camera and head put its anchors: {}",
                self.overlay_shifts.found().join("; ")
            ));
        }
        Ok(())
    }

    /// Answers a camera that is only looking at another point of the map by moving the overlay's
    /// windows, which are anchored in the world: each anchor is projected at this frame's camera
    /// and the windows standing on it are offset by whole interface pixels. Nothing is laid out,
    /// painted, sorted or composed again. Whether the camera was answered.
    ///
    /// Refused, so that [`Self::place_overlay`] answers the frame instead, when the camera did
    /// more than look elsewhere (a zoom, a free eye), when it has come to rest (a still picture
    /// stands at its own layout), when it has moved past the placed view's pad (a window outside
    /// that view could have reached the frame), or before the overlay is drawn.
    fn move_overlay(
        &mut self,
        camera: Camera,
        pixels: [u32; 2],
        ui_scale: f32,
        at: viewer_foundation::map::anchored::Head,
    ) -> Result<bool, String> {
        let moved = self.overlay_camera_before.is_some_and(|before| before != camera);
        self.overlay_camera_before = Some(camera);
        // The head advanced over a batch that carries movers: every one of them is placed at this
        // frame's own hour, which is what a watcher sees as the replay advancing evenly.
        let advanced = self.overlay_head_before.is_some_and(|before| before != at)
            && self.overlay.as_ref().is_some_and(Batch::moves);
        self.overlay_head_before = Some(at);
        let (Some(placed), true) = (self.overlay_placed, moved || advanced) else {
            return Ok(false);
        };
        if self.overlay_unpainted {
            viewer_foundation::map::redraws::note("made.not_moved", 1);
            if viewer_foundation::view_record::on() {
                viewer_foundation::view_record::add("overlay", "the batch is the painting before the overlay installed", 1);
            }
            return Ok(false);
        }
        if (placed == camera && !advanced)
            || self.overlay_placed_view != Some((pixels, ui_scale.to_bits()))
            || self.overlay_drawn_fraction.is_none()
        {
            return Ok(false);
        }
        let Some(batch) = self.overlay.as_mut() else {
            return Ok(false);
        };
        let extent = viewer_foundation::map::gui_resources::viewport(pixels, ui_scale);
        let _span = viewer_foundation::profiling::span("overlay_move");
        match self
            .overlay_shifts
            .move_to(batch, &placed, &camera, pixels, extent, at, check_anchor())
        {
            Ok(moved) => {
                viewer_foundation::map::redraws::note("made.moved", 1);
                viewer_foundation::map::redraws::note("moved.movers", moved.movers as u64);
                viewer_foundation::map::redraws::note("moved.pictures", u64::from(moved.quads > 0));
                viewer_foundation::view_record::add("overlay", "movers_placed", moved.movers as u64);
                viewer_foundation::view_record::add("overlay", "movers_still", moved.movers_still as u64);
                for (name, value) in moved.named() {
                    viewer_foundation::view_record::add("overlay", name, value);
                }
                viewer_foundation::view_record::set("overlay", "step_max", moved.step_max.unsigned_abs().into());
                viewer_foundation::view_record::add("overlay", "anchors_moved", moved.anchors_moved as u64);
                viewer_foundation::view_record::add("overlay", "quads_moved", moved.quads as u64);
                viewer_foundation::view_record::set("overlay", "anchors", moved.anchors as u64);
                self.hits_stamp += 1;
                if !self.overlay_shifts.found().is_empty() {
                    return Err(format!(
                        "the overlay's windows do not stand where this frame's camera puts their anchors: {}",
                        self.overlay_shifts.found().join("; ")
                    ));
                }
                Ok(true)
            }
            Err(why) => {
                viewer_foundation::map::redraws::note("made.not_moved", 1);
                if viewer_foundation::view_record::on() {
                    viewer_foundation::view_record::add("overlay", why.why(), 1);
                }
                Ok(false)
            }
        }
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
        let retired = (self.overlay_tables.take(), self.overlay_frame.take(), self.prepared_overlay.take());
        self.retire(retired);
        self.overlay_hour = None;
        self.frame_head.forget();
        self.overlay_shifts.clear();
        self.overlay_placed_view = None;
        (self.overlay_shown, self.overlay_view, self.overlay_hour, self.overlay_drawn_fraction, self.overlay_lookups) = (None, None, None, None, None);
        self.overlay_layered = None;
        self.overlay_loading.clear();
        self.overlay_missed.clear();
    }

    /// The map mode of the picture on screen: the mode its layers were composed and settled
    /// for, the chosen one ([`Self::map_mode`]) before any picture.
    fn shown_mode(&self) -> &viewer_world::map::map_mode::MapMode {
        self.live_map().and_then(|side| side.mode_on_screen()).unwrap_or(&self.map_mode)
    }

    /// This frame's own place in the replay: the one hour everything it draws stands at
    /// ([`viewer_foundation::map::frame_head`]). It follows the playback clock, so the hours
    /// advance as evenly as playback does, and is steered to keep inside the hours the newest
    /// publication has given as real data; stopped (a pause, a seek, the recording's end) and on a
    /// saved frame it is the clock's own hour exactly. None before the clock is anchored.
    fn take_frame_head(&mut self) -> Option<viewer_foundation::map::anchored::Head> {
        let clock = self.playhead?;
        // A saved frame and a stepped capture stand at the hour they were asked for, whole.
        let saved = self.options.window_dump || self.capture.as_ref().is_some_and(|capture| !capture.paced);
        // The hours real data covers end at the newest publication the frame holds, which is the
        // hour its movers' newest places and its timed layers' newest change sets stand at.
        let published = self.overlay_shown.map(|(_, hour)| hour);
        let clock = f64::from(clock.hour) + clock.fraction;
        let head = self.frame_head.take(viewer_foundation::map::frame_head::Frame {
            clock,
            published,
            speed: self.speed,
            moving: self.playing && self.speed > 0.0 && !saved,
        });
        if viewer_foundation::view_record::on() {
            use viewer_foundation::view_record::{add, set};
            let at = f64::from(head.hour) + f64::from(head.fraction);
            add("head", "frames", 1);
            set("head", "behind_the_clock_milli_hours", ((clock - at) * 1e3) as u64);
            if let Some(published) = published {
                let above = f64::from(published) - at;
                add("head", "past_the_newest_published_hour", u64::from(above < 0.0));
                set("head", "the_newest_published_hour_above_milli", (above.max(0.0) * 1e3) as u64);
            }
        }
        Some(head)
    }

    /// Whether the map's reader has composed and uploaded a first picture: until then the spinner
    /// covers the map.
    fn picture_ready(&self) -> bool {
        self.playhead_map.as_ref().is_some_and(|side| side.on_screen().is_some_and(|on| on.drawn.is_some()))
    }

    /// Whether the frame drawn answers `target` with everything on screen: exactly, or, playing,
    /// with the newest hour before it.
    fn presentation_ready(&self, target: Playhead, playing: bool) -> bool {
        self.rendered.is_some_and(|rendered| rendered.branch == target.branch)
            && self.picture_whole(target, playing)
    }

    /// Whether every part of the picture drawn shows `target`'s hour ([`whole_picture`]).
    fn picture_whole(&self, target: Playhead, playing: bool) -> bool {
        self.playhead_map.as_ref().is_some_and(|side| {
            side.on_screen().is_some_and(|on| {
                whole_picture(&on, self.overlay_hour(), target.hour, playing)
            })
        })
    }

    /// Whether the document's load is over: no lazy art decodes, the overlay was laid out since
    /// the last did, and the picture asked for is whole ([`Self::picture_whole`]). Until then the
    /// content made for it is kept.
    fn load_complete(&self, art_busy: bool, overlay_fresh: bool) -> bool {
        !art_busy && overlay_fresh && self.playhead.is_some_and(|head| self.picture_whole(head, self.playing))
    }

    /// The hour of the rows at the head the overlay on screen was made from: its tables, its
    /// layout and the counter layers' products all stand at it, because the side's step made them
    /// together. None before its first, and while a read it missed is being learned.
    pub(crate) fn overlay_hour(&self) -> Option<replay_data::Hour> {
        self.overlay_hour.filter(|_| self.overlay_missed.is_empty())
    }

    /// The playhead's map while it draws: open, its reader not stopped.
    fn live_map(&self) -> Option<&crate::playhead_host::side::SideThread> {
        self.playhead_map.as_ref().filter(|side| side.failure().is_none())
    }

    /// The branch and hour on screen: the reader's branch and drawn hour, the clock's before its
    /// first picture.
    fn hour_on_screen(&self, document: &Document) -> (replay_data::BranchId, i32) {
        self.layers_on_screen().unwrap_or_else(|| {
            self.playhead.map_or_else(|| (document.playback.branch(), document.playback.hour()), |clock| (clock.branch, clock.hour))
        })
    }

    /// While the playhead's map is open, its reader's branch (the clock's for a reader opened
    /// without one) and drawn hour; none before its first picture or once its reader stopped.
    fn layers_on_screen(&self) -> Option<(replay_data::BranchId, i32)> {
        let side = self.playhead_map.as_ref()?;
        Some((side.branch().or(self.playhead.map(|clock| clock.branch))?, side.on_screen()?.drawn?))
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

    /// The map's error view: the reader's refusal by name while there is none, its own failure by
    /// name once it stopped, else the recorded gap the clock stands in.
    fn map_error_of(&mut self, document: &Document) -> Option<String> {
        let Some(side) = &self.playhead_map else {
            return self.playhead_refused.clone();
        };
        if let Some(failure) = side.failure() {
            return Some(failure.to_owned());
        }
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
        let viewing = side.products().ours.map(|slot| slot.to_string());
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

    pub(crate) fn map_state(
        &self,
        target: &dashboard::protocol::DashboardMapView,
    ) -> dashboard::protocol::DashboardMapState {
        let error = self.error.clone().or_else(|| self.map_error.clone());
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
        let layers = self.playhead_map.as_ref().and_then(|side| side.on_screen());
        // Playing, a frame of the same branch stands in for the clock's hour; say how far back. A
        // frame of an hour the clock has not reached stands in for nothing, and trails it by no
        // hours at all: it says none, never a lag of zero.
        let lag = |hour: i32| {
            clock
                .zip(self.rendered)
                .filter(|(clock, rendered)| target.playing && clock.branch == rendered.branch)
                .and_then(|(clock, _)| u32::try_from(i64::from(clock.hour) - i64::from(hour)).ok())
        };
        let behind = layers.and_then(|on| on.drawn).and_then(lag);
        let behind_reason = behind.filter(|behind| *behind > 0).and(layers.as_ref().map(layers_behind_reason));
        // The hour on screen is the layers' drawn hour; none before their first picture.
        let (hour, fraction) = match (self.rendered, layers.and_then(|on| on.drawn)) {
            (Some(_), Some(drawn)) => (Some(drawn), Some(0.0)),
            _ => (None, None),
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
            behind,
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
    /// The picture on screen's limitation lines and its layers' reports; none before its first
    /// picture.
    fn hour_lines(&self) -> Option<[Vec<String>; 2]> {
        let side = self.playhead_map.as_ref().filter(|side| side.on_screen().is_some())?;
        let lines = side.products().lines.as_ref()?;
        Some([lines.limitations.clone(), lines.political.iter().chain(&lines.reports).cloned().collect()])
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
        if let Some(history) = self.layer_history() {
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

    /// The diagnostics an embedded map shows over itself: the hour's stats and
    /// its data limitations, collapsed behind their count.
    pub(crate) fn diagnostics_ui(&mut self, ui: &mut egui::Ui, max_height: f32) {
        let side = self.playhead_map.as_ref().filter(|side| side.on_screen().is_some());
        let stats = side.map(|side| {
            side.products().snow.map(|summary| vec![format!("Snow and mud: {}", summary.line())]).unwrap_or_default()
        });
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
        if let Some(limit) = viewer_foundation::map::frame_limit::limit() {
            ui.weak(format!("Frame rate limit: {limit:.0} a second"));
        }
        for line in crate::screen_count::readout::lines() {
            ui.weak(line);
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
        let shared_pool = self.shared_pool.clone();
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
                let (document, scene) = loop {
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
                    // The map's layers need seconds for their first picture and the UI thread
                    // idles on the spinner meanwhile; the whole hour-independent scene is cheaper
                    // here than on any frame.
                    let workers = shared_pool
                        .as_ref()
                        .map_or_else(Default::default, |pool| pool.workers());
                    let scene_stage = viewer_foundation::load_record::stage("load.scene");
                    let scene = load_scene(&device, &queue, &document, preferences.graphics, &workers);
                    drop(scene_stage);
                    match scene {
                        Ok(scene) => break (document, scene),
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
        self.loaded_config = None;
        self.rendered = None;
        self.revision = None;
        if !self.dashboard_map
            && let Some(scene) = &mut self.scene
        {
            crate::perf_log::gpu_times(scene.retire_gpu_frame_times());
        }
        self.scene = None;
        self.playhead = None;
        self.seek_owed = false;
        self.last_view = None;
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
        self.fade = None;
        // counters
        self.overlay = None;
        self.overlay_shifts.clear();
        self.overlay_written += 1;
        self.counter_report = CounterReport::default();
        self.hits_stamp += 1;
        self.counter_held = None;
        self.unit_selection = Default::default();
        self.air_selection = Default::default();
        self.fleet_selection = Default::default();
        self.overlay_fraction = None;
        self.overlay_placed = None;
        self.forget_overlay_builds();
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
                config,
            }))) => {
                let _stage = viewer_foundation::load_record::stage("install");
                self.revision = revision;
                self.source = source;
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
            refresh_monitors(viewer_foundation::worker_priority::waking(move || ctx.request_repaint()));
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

    /// How far each of the frame's icons stands from where the overlay was laid out; none while
    /// the overlay stands where it was placed ([`viewer_foundation::map::anchored::Shifts`]).
    fn hit_shifts(&self) -> Option<&viewer_foundation::map::anchored::Shifts> {
        (!self.overlay_shifts.is_empty()).then_some(&self.overlay_shifts)
    }

    /// The frame's hit shapes, as the pointer is resolved against them.
    fn hit_shapes(&self) -> map_hits::Shapes<'_> {
        let overlay = self.prepared_overlay.as_ref();
        map_hits::Shapes {
            report: &self.counter_report,
            raids: overlay.map_or(&[], |map| map.raid_hits()),
            occupation: overlay.map_or(&[], |map| map.occupation_hits()),
            adjacency: overlay.map_or(&[], |map| map.adjacency_hits()),
            looks: self.looks.as_ref(),
            shifts: self.hit_shifts(),
        }
    }

    /// The map icon under `point`, resolved once a frame for each point asked about: every
    /// consumer reads the one value the frame's index gave.
    fn map_hit(&self, point: [f32; 2]) -> Option<MapHit> {
        let shapes = self.hit_shapes();
        let overlay = self.prepared_overlay.as_ref();
        self.map_hits.borrow_mut().at(self.hits_stamp, point, &shapes, |point| {
            overlay.and_then(|map| map.intel_at(|hit| self.icon_point(free_icons::IconFamily::Intel, hit.layer, point)))
        })
    }

    /// `point` in the layout of the icon of `family` keyed `layer`, with its draw rank: the one
    /// rule every consumer of a hit works by ([`map_hits::local`]).
    fn icon_point(&self, family: free_icons::IconFamily, layer: u64, point: [f32; 2]) -> Option<([f32; 2], u32)> {
        map_hits::local(self.hit_shifts(), self.looks.as_ref(), family, layer, point)
    }

    /// The layer key of `target`'s hit shape; `None` for targets that are not one map icon.
    fn hit_layer(&self, target: MapHit) -> Option<(free_icons::IconFamily, u64)> {
        map_hits::hit_layer(&self.hit_shapes(), target)
    }

    /// The drawn raid icon under `point`, as [`Viewer::map_hit`] ranks them.
    fn raid_hit(&self, point: [f32; 2]) -> Option<&viewer_raids::map::raids::tooltips::Hit> {
        map_hits::raid_at(&self.hit_shapes(), point)
    }

    /// `point` in the layout of the icon whose hit shape is keyed `layer` in its family, as a free
    /// eye shows it; `point` itself when the overlay draws as laid out.
    fn local_point(
        &self,
        layer: Option<(free_icons::IconFamily, u64)>,
        point: [f32; 2],
    ) -> [f32; 2] {
        layer
            .and_then(|(family, layer)| self.icon_point(family, layer, point))
            .map_or(point, |(local, _)| local)
    }

    /// A laid-out rectangle `[x, y, width, height]` of the icon keyed `layer` where a free eye
    /// draws it.
    fn shown_rect(&self, layer: Option<(free_icons::IconFamily, u64)>, rect: [f32; 4]) -> [f32; 4] {
        layer
            .and_then(|(family, layer)| map_hits::shown(self.hit_shifts(), self.looks.as_ref(), family, layer, rect))
            .unwrap_or(rect)
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
        // While the political layer draws, the highlight is built over its picture, keyed by the
        // picture's change stamp, or at another mode over the mode fills layer's, keyed by its
        // product's hour.
        self.highlight_wanted = !selected.is_empty() || self.scene.as_ref().is_some_and(|scene| !scene.faction_highlight_regions().is_empty());
        let Some(side) = self.playhead_map.as_ref() else { return Ok(()) };
        // A picture copied before the borders this frame wrote stands on older borders: no
        // highlight is built over it, and one shown is taken off.
        let stale = side.products().highlight_hour.zip(side.plans_shown()).is_some_and(|(picture, plans)| picture < plans);
        let Some((picture, over)) = side.products().highlight.clone().filter(|_| !stale) else {
            // No picture stands on the borders on screen: a highlight shown is taken off, and the
            // political layer writes its own borders again over the restore, in this frame.
            if let Some(scene) = self.scene.as_mut()
                && !scene.faction_highlight_regions().is_empty()
                && let Some(restore) = self.faction_highlight.take_restore()
            {
                scene.apply(device, queue, SceneInput::SetFactionHighlight { plan: &restore, strategic_regions: &[], color: *color }).map_err(|why| why.to_string())?;
                if let Some(side) = self.playhead_map.as_mut() {
                    side.reassert(&mut crate::layers::Gpu { device, queue, scene, content: &document.content, whole: false });
                }
            }
            return Ok(());
        };
        let Some((branch, hour)) = self.layers_on_screen() else { return Ok(()) };
        let key = faction_highlight::Key {
            branch,
            hour,
            perspective: self.perspective.clone(),
            borders: picture,
            regions: Arc::clone(&selected),
            adjacency,
        };
        let scene = self.scene.as_mut().expect("scene built");
        if self.faction_highlight.requested() != Some(&key)
            && !scene.faction_highlight_regions().is_empty()
            && let Some(restore) = self.faction_highlight.take_restore()
        {
            scene
                .apply(device, queue, SceneInput::SetFactionHighlight { plan: &restore, strategic_regions: &[], color: *color })
                .map_err(|why| why.to_string())?;
            if let Some(side) = self.playhead_map.as_mut() {
                side.reassert(&mut crate::layers::Gpu { device, queue, scene, content: &document.content, whole: false });
            }
        }
        let content = Arc::clone(&document.content);
        let dropper = pool.dropper();
        if let Some(prepared) = self.faction_highlight.update_with(
            key,
            |job| pool.try_submit(job),
            move || {
                let source = over;
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
        let point = point?;
        let target = self.map_hit(point);
        // An intel entry takes the hover even when its text is empty: the province's is not
        // asked for.
        if matches!(target, Some(MapHit::Intel)) {
            let hit = self
                .prepared_overlay
                .as_ref()
                .and_then(|map| map.intel_at(|hit| self.icon_point(free_icons::IconFamily::Intel, hit.layer, point)))?;
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
        let views = self.rows_head()?;
        let head = &views;
        let repaint = || {
            let ctx = ctx.clone();
            viewer_foundation::worker_priority::waking(move || ctx.request_repaint())
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
        let routes = self.open_map().and_then(|side| side.products().routes.as_ref()).map(|(_, routes)| Arc::clone(routes));
        if self.counter_view.visibility.trade_routes
            && routes.as_ref().is_some_and(|routes| {
                viewer_foundation::map::ribbon_content::trade_route_hit(
                    &routes.field.trade_route_hovers,
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
            *sources = match &self.overlay_tables {
                Some(tables) => tables.naval_dominance_sources(),
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

    /// One frame of the Campaign map as a window draws it: the window's central panel, the map and
    /// its interface in it, and the frame's own end. The harness's `--campaign-tab` draws through
    /// this too, so a measured frame is the frame the app's map tab runs. Whether the map drew.
    pub(crate) fn window_frame(
        &mut self,
        ui: &mut egui::Ui,
        state: Option<&eframe::egui_wgpu::RenderState>,
        document: &mut Document,
    ) -> bool {
        let drawn = egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let result = match state {
                    Some(state) => self.map_with_state(ui, state, document),
                    None => Err("WGPU renderer is unavailable".to_owned()),
                };
                self.after_frame_with_visual_mods(document, result.as_ref().err());
                if let Err(error) = result {
                    self.failed_window_capture(&error, ui.ctx());
                    self.error = Some(error);
                    false
                } else {
                    self.drew_frame
                }
            })
            .inner;
        drawn
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
        // The map frame's work begins here, and what an hour brings takes the room it leaves. A
        // frame whose picture must be whole - a stepped capture, a window dump - keeps no budget,
        // so everything an hour brings lands on it.
        let whole = self.options.window_dump || self.capture.as_ref().is_some_and(|capture| !capture.paced);
        let _budget = viewer_foundation::map::frame_budget::begin(!whole);
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
            match crate::playhead_host::Opening::start(document, Some(at)) {
                Ok(opening) => self.playhead_opening = Some(opening),
                Err(error) => self.playhead_refused = Some(refusal(&error)),
            }
        }
        if let Some(made) = self.playhead_opening.as_ref().and_then(crate::playhead_host::Opening::poll) {
            self.playhead_opening = None;
            match made.and_then(crate::playhead_host::side::SideThread::new) {
                // Opened for a branch the clock has left meanwhile: opened again at the clock.
                Ok(side) if self.playhead.is_some_and(|clock| side.branch().is_some_and(|branch| branch != clock.branch)) => {
                    self.drop_later(side);
                    self.playhead_opened = false;
                }
                Ok(side) => {
                    document.playback.read_rosters_of(side.entry());
                    // With a reader open nothing walks the recording: the load's cursor and map
                    // walk go, off the frame path. The presented frame keeps its identity.
                    let released = document.playback.release_source();
                    self.retire(released);
                    self.presented.reopened = true;
                    self.playhead_refused = None;
                    self.side_opened(side);
                }
                Err(error) => self.playhead_refused = Some(refusal(&error)),
            }
        }
        // Where this frame stands in the replay: one head for everything it draws — the movers'
        // places, the timed layers' installs, the overlay it holds, the figures and the hour it
        // reports on screen — kept inside the hours the publications have given as real data.
        // Taken once, before the side's window, and used by every part of the frame after it.
        let mut shown_head = self.take_frame_head();
        // The window's readers of the side's rows read none past the hour this frame shows, paused
        // or playing: the side prepares its rows ahead of it. After a seek that is the hour sought.
        // A saved frame's head is the side's own hour.
        let taken = shown_head.map(|head| head.hour).filter(|_| !self.saves_a_frame());
        if let Some(side) = self.playhead_map.as_ref() {
            side.views().show_through(taken);
        }
        // The map's side prepares its steps on a thread of its own: this frame asks for the view
        // and head it draws at, then takes the side in a window that never waits (a stepped frame
        // waits for the picture of exactly its hour), uploads what it settled into the scene and
        // takes the products the rest of the frame draws from.
        if self.playhead_map.is_some() {
            // This frame draws the map when a first picture stands and it has placed the camera
            // and size the overlay is made for. One that draws none (the first pictures, a
            // harness's warm-up) asks for no overlay and waits for no whole picture: the side
            // composes, and the first frame that draws is the first the overlay is made for.
            let drawing = self.last_view.is_some() && self.picture_ready();
            let at = head_wanted(
                self.capture.as_ref().map(|capture| capture.paced),
                self.playhead.map(|head| head.hour),
                self.options.window_dump,
                drawing,
            );
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
            // What only the frame knows of the overlay, for the step prepared next: where the
            // frame last drew, its counter options, selection and hover. The side makes the
            // overlay of its step from them and from its own layers' products.
            // The view the step is prepared for shows the map mode of the picture on screen, as
            // this frame's draw will: the mode is resolved for the frame below, after the window.
            // A stepped frame's picture is the one asked for, mode and all.
            let shown_mode = match at {
                crate::playhead_host::Head::Stepped(_) => self.map_mode.clone(),
                _ => self.shown_mode().clone(),
            };
            let overlay_view = {
                let mut view = self.counter_view.clone();
                view.set_mode(&shown_mode);
                view
            };
            let overlay_ask = self.last_view.as_ref().filter(|_| drawing).map(|last| crate::playhead_host::overlay::Ask {
                camera: last.camera,
                target: self.zoom.target().map(|target| target.height),
                pixels: last.viewport,
                ui_scale: last.ui_scale,
                view: overlay_view.clone(),
                rosters: document.playback.air_rosters(),
            });
            let wanted = builds_warnings(self.diagnostics, self.capture.as_ref());
            let saved = self.saves_a_frame();
            // The hours this frame's own place in the replay reaches: none for a picture that must
            // be whole at the hour it stands at (a saved frame, a pause, a first picture), which
            // takes every change set handed over and waits for every product of its hour.
            let through = shown_head.map(|head| head.hour).filter(|_| !saved && self.playing);
            let mut installed = None;
            {
                let presentation = Arc::clone(&self.counter_view.presentation);
                let side = self.playhead_map.as_mut().expect("the map's side is open");
                side.ask(view, at, overlay_ask);
                let window = match at {
                    crate::playhead_host::Head::Stepped(hour) => side.stepped_window(hour),
                    _ => side.window(),
                };
                if let Some(mut window) = window {
                    // A saved frame (stepped, or a window dump) draws the plan and the combat
                    // markers of its hour.
                    if saved && window.side().failure().is_none() {
                        let hour = window.side().last_frame().head;
                        settle_models(window.side_mut(), hour, Duration::from_secs(120))?;
                        settle_plan(window.side_mut(), hour, Duration::from_secs(120))?;
                    }
                    if let Some(scene) = self.scene.as_mut() {
                        let mut gpu = crate::layers::Gpu { device: &state.device, queue: &state.queue, scene, content: &document.content, whole: through.is_none() };
                        window.upload(&mut gpu);
                    }
                    window.take(crate::playhead_host::side::Wants { warnings: wanted, highlight: self.highlight_wanted });
                    // The overlay of the step, taken while the side is held: its counters' memory
                    // becomes the view's here, the one place the side writes nothing.
                    if let Some(made) = window.take_overlay() {
                        // A saved frame's was laid out on the view's own memory: there is nothing
                        // to take. Any other is installed with the memory it was laid out on, which
                        // the maker lays nothing else out on until this frame hands the other back.
                        if !Arc::ptr_eq(&presentation, &made.on) {
                            presentation.take_memory(&made.on);
                        }
                        installed = Some(made);
                    }
                }
            }
            // Playing past the newest hour the overlay's layout stands at, every mover holds its
            // last place: the frame draws, writes and says that hour, and the head it steers by
            // goes on ([`viewer_foundation::map::frame_head::shown`]).
            if self.playing
                && !saved
                && let Some(covered) = live_of(&self.playhead_map).and_then(|side| side.products().overlay_hour)
            {
                shown_head = shown_head.map(|head| viewer_foundation::map::frame_head::shown(head, Some(covered)));
            }
            // This frame's own place in the replay: the timed layers' change sets of the hours
            // its head has reached, written whether or not the side's thread held the window, so
            // no display frame repeats an hour because the side was busy. A saved frame takes
            // every set handed over; a pause takes those of its own hour, and the side's exact
            // reset replaces any dated past it.
            if let Some(scene) = self.scene.as_mut() {
                let side = self.playhead_map.as_mut().expect("the map's side is open");
                let written = shown_head.map(|head| head.hour).filter(|_| !saved);
                let mut gpu = crate::layers::Gpu { device: &state.device, queue: &state.queue, scene, content: &document.content, whole: through.is_none() };
                side.install(&mut gpu, written);
                // What the timed layers wrote is what the frame shows: the movers are placed and the
                // hour said at that hour, behind the head until the rest is written, or past it
                // after a seek back until the hour sought replaces it.
                if let Some(installed) = side.installed_through()
                    && shown_head.is_some_and(|head| head.hour != installed)
                {
                    shown_head = Some(viewer_foundation::map::anchored::Head { hour: installed, fraction: 0.0 });
                }
                // Behind the head, the readers stand where the timed layers wrote; past it after a
                // seek back, at the hour sought, never at the history the seek left.
                side.views().show_through(taken.zip(shown_head).map(|(taken, head)| taken.min(head.hour)));
            }
            if let Some(made) = installed {
                self.install_overlay(made);
            }
            // The hour the overlay the frame draws stands at is the side's own: the layout taken
            // above, or, for an hour the overlay read nothing new in, the layout the frame already
            // holds, which the maker laid out again for nobody and said stands at this hour too.
            if let Some(side) = live_of(&self.playhead_map) {
                self.overlay_hour = side.products().overlay_hour;
            }
            if let Some(battles) = self.playhead_map.as_ref().and_then(|side| side.products().battles.clone()) {
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
        // The counter kinds the layers draw take the products this frame's window took; a kind
        // whose layer stops keeps its last until tables holding today's are on screen.
        {
            let _lap = replay_frame::cpu::scope("frame.refresh.counters", Some("other"));
            let left_out = self.overlay_tables.as_ref().map(|tables| tables.counters().left_out()).unwrap_or_default();
            // The products the overlay shown was laid out from, so the counters drawn are the ones
            // its layout placed and the kinds it left out are the ones the layers drew for it; a
            // later step's products belong to the layout the frame has not installed yet.
            let (published, draws) = match &self.overlay_layered {
                Some((published, draws)) => (published.clone(), *draws),
                None => self.playhead_map.as_ref().map(|side| (side.products().counters.clone(), side.products().counter_kinds)).unwrap_or_default(),
            };
            self.counter_view.presentation.counters.lock().unwrap_or_else(|error| error.into_inner()).set_layered(published, draws, left_out);
        }
        let map_error = self.map_error_of(document);
        self.hour_error(map_error, ui.ctx());
        self.follow_continuity(&document.playback.replay().gaps);
        // An error view otherwise stops repainting; a paused seek away from it still needs to
        // draw until the layers have composed its hour.
        if self.playhead.is_some_and(|head| {
            self.playhead_map.as_ref().is_some_and(|side| side.on_screen().is_some_and(|on| on.drawn != Some(head.hour) || !on.settled))
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
            // No picture is composed yet; keep drawing until one is.
            if profiling() {
                profile_print(format!("PROFILE no_map frame {}", self.profile_frame));
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
        // A reset composing: the picture is the one before it, drawn at the perspective its
        // reader drew it at.
        let (stale_hour, perspective) = match self.live_map() {
            Some(side) if side.composing() => (true, side.view().perspective.clone()),
            _ => (false, self.perspective.clone()),
        };
        if stale_hour && profiling() {
            profile_print(format!("PROFILE stale_hour frame {}", self.profile_frame));
        }
        // What the frame shows of the map mode's layers (counters, convoys, fog, models) is the
        // mode the picture on screen was composed for, until one composed for a newly chosen mode
        // replaces it.
        let mode = self.shown_mode().clone();
        self.counter_view.set_mode(&mode);
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
        let engaged = self.layer_engaged();
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
                        // The viewing country's infantry, chosen again for each hour and view,
                        // from rows at the head and the vision layer's viewing country; the
                        // figure is kept while either loads.
                        let vision = self.shown_vision().ok();
                        let head = head_rows::head(live_of(&self.playhead_map));
                        let branch = head_rows::branch(live_of(&self.playhead_map), document);
                        let key = self.head_rows.walk_hour(head).map(|hour| (branch, hour, perspective.clone()));
                        if let Some(key) = key.filter(|key| self.walk.stale(key))
                            && let Some(figure) = vision.and_then(|vision| self.head_rows.walk(head, None, |frame| document.unit_models.infantry_figure(frame, vision.ours())))
                        {
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
        crate::perf_log::frame_view(camera.free().is_some(), camera.height);
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
        // A stopped picture is exact: a saved frame, and a picture the hours have stopped on,
        // pose every figure on the frame that draws it, from exactly the samples it draws. Told
        // only when it changes, as the builds are, so a tool that set it for its whole run keeps
        // it ([`crate::app::playback_cost`]).
        let stopped = saving || !self.playing;
        if stopped != self.settled_poses {
            scene.settle_poses(stopped);
            self.settled_poses = stopped;
        }
        // The units layer installs the model field; the raid units and convoys are placed in it
        // on the hour, and move by the presentation clock.
        let tick = 0.0;
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
        // The units layer's models carry the raid units and the convoys.
        let posed = live_of(&self.playhead_map).and_then(|side| side.products().units.clone());
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
        // The viewing country the map's vision layer shows; while it shows none the selections
        // keep what they hold.
        let seen = self.shown_vision().ok();
        let head = head_rows::head(live_of(&self.playhead_map));
        let branch = head_rows::branch(live_of(&self.playhead_map), document);
        let frame = None;
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
        self.last_view = Some(View {
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
        // The overlay is made whole on the map's side, inside the step that prepares the picture:
        // its tables from the rows at the step's hour, the counter layers given those tables
        // before they settle, and the layout at the latest view's height band, viewport and
        // interface scale. This frame places it at its camera and draws it; it lays nothing out.
        let left = self.overlay_left;
        // Before the first overlay the side hands over: nothing is laid out yet.
        let (Some(rows), Some(tables)) = (self.overlay_frame.clone(), self.overlay_tables.clone()) else {
            self.rendered = None;
            ui.ctx().request_repaint();
            return Ok(());
        };
        let now = crate::document::Hour::new(&document.content, &rows, 0.0).with_air_rosters(Some(document.playback.air_rosters())).with_moved_hours(Arc::clone(&self.overlay_moved));
        let (overlay_ms, additions) = {
            let _other_cost = replay_frame::cpu::scope("frame.overlay.prepare", Some("other"));
            let overlay_timer = Instant::now();
            // capture: the animation clock is the one input a live window cannot
            // share with a headless frame, so a capture frame renders at 0, the
            // still frame `--dump` renders, or at its explicit sequence clock. The
            // window runs one presentation clock: the map scene and the map-icon
            // windows' GUI clock (`Time`) share it.
            // A background flag or portrait load repaints the window; the side lays the overlay
            // out again for the art that arrived.
            if !document.icons.lazy.has_wake() {
                let ctx = ui.ctx().clone();
                document.icons.lazy.set_wake(viewer_foundation::worker_priority::waking(move || ctx.request_repaint()));
            }
            // Routes made by key: a worker fills every route of the tables the side made, so a
            // free frame before them finds what it admits made.
            if std::mem::take(&mut self.overlay_routes_pending) && tables.counters().routes_by_key() {
                let pool = self.worker_pool().map_err(|why| format!("Native map workers: {why}"))?;
                let (fill, counters, movement) = (Arc::clone(&tables), Arc::clone(&document.content.counters), document.content.movement.clone());
                pool.try_submit(Box::new(move || viewer_foundation::map::redraws::apart(|| fill.counters().fill_routes(&counters, movement.as_deref()))));
            }
            // What the overlay reads that its rows do not hold, and what its reader waits for: the
            // side names both.
            let overlay_state = live_of(&self.playhead_map).map(|side| {
                let products = side.products();
                (products.overlay_missed.clone(), products.overlay_loading.clone())
            });
            if let Some((missed, loading)) = overlay_state {
                self.overlay_missed = missed;
                self.overlay_loading.extend(loading);
            }
            // The overlay of the step at this frame's camera: a camera moving over the map takes
            // the windows it already drew, anchored in the world, and offsets them; anything else
            // places the layout afresh, which draws it again too.
            // Every mover of the overlay is placed at this frame's one head, the hour the rest of
            // the picture it draws stands at too ([`Self::take_frame_head`]).
            let at = shown_head.unwrap_or_default();
            let moved_now = self.move_overlay(overlay_camera, pixels, ui_scale, at)?;
            let placed_now = !moved_now
                && self.place_overlay(&now, overlay_camera, pixels, ui_scale, &tables, &rows, left);
            // The country names the map's layer placed at the head replace the hour's.
            if let Some(side) = live_of(&self.playhead_map)
                && let Some(names) = side.products().placements.clone()
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
            overlay_prepare_cpu.finish();
            laps.lap("overlay_prepare");
            drop(_other_cost);
            let _other_cost = replay_frame::cpu::scope("frame.overlay.draw", Some("other"));
            let overlay_draw_cpu = viewer_foundation::profiling::thread_span("overlay_draw");
            let owners_span = viewer_foundation::profiling::span("overlay_owners");
            // Display choices only change what the next overlay draw keeps.
            let ours = Some(tables.ours());
            let head = head_rows::head(live_of(&self.playhead_map));
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
                let head = head_rows::head(live_of(&self.playhead_map));
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
                                head: at,
                                // The publication this frame draws, and the map it wraps on: the
                                // cut and the width its counters' own tracks were made with, so a
                                // figure placed on those places stands where its counter stands.
                                overlay_hour: now.frame.hour,
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
                // The land figures move from the units layer's own placings at the applied hour,
                // the formations from its models.
                let positions: Option<Box<dyn Iterator<Item = (viewer_foundation::unit_handle::Unit, &viewer_foundation::map::counter_move::Position, f32, f32)> + '_>> =
                    live_of(&self.playhead_map).map(|side| Box::new(side.products().movement.iter().map(|(unit, place, scale, fade)| (*unit, place, *scale, *fade))) as _);
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
            // Whether this frame painted the overlay's batch: the gate compares what a painting
            // draws, whether or not its windows were then moved to this frame's camera and head.
            let mut painted_now = false;
            if self.overlay_fraction != Some(document.hour_fraction)
                && let Some(prepared) = self.prepared_overlay.as_mut()
            {
                // Asked for by running pools alone: only their passes run.
                let quiet = self.overlay_fraction.is_some_and(f32::is_nan)
                    && self.overlay_drawn_fraction == Some(document.hour_fraction);
                if quiet {
                    viewer_foundation::map::redraws::note("made.quiet", 1);
                    prepared
                        .draw_quiet(&now, overlay_camera, pixels, ui_scale, &self.overlay_layers, animation_time, self.overlay_pools)
                        .map_err(|error| error.to_string())?;
                } else {
                    viewer_foundation::map::redraws::note("made.draw", 1);
                    painted_now = true;
                    self.overlay_written += 1;
                    prepared.set_pressed(self.counter_view.pressed, self.counter_view.plate_hovered);
                    // The first frame drawn shows the art the drawing itself asks for: the batch
                    // is drawn, the art it asked for settled and the hour laid out over it again
                    // (the side lays it out, as for any other frame) until none is left. A frame
                    // after one rendered draws once and the art arrives in a later frame.
                    // The icon windows a draw asks the view's native pools for are made over the
                    // frames after it, so a saved frame draws again until none is pending, within
                    // the pools' own growth rounds: its icons are never missing for want of one.
                    let mut rounds = viewer_foundation::map::icon_windows::DumpFrames::default();
                    // A round that takes a newer publication draws that publication whole: each
                    // reads the overlay the frame holds with the tables and the rows it was laid
                    // out over, so nothing is placed, drawn or judged over another's.
                    let mut installed = false;
                    let mut settles = 0;
                    loop {
                        let (Some(rows), Some(tables)) = (self.overlay_frame.clone(), self.overlay_tables.clone()) else {
                            break;
                        };
                        let held = crate::document::Hour::new(&document.content, &rows, 0.0).with_air_rosters(Some(document.playback.air_rosters())).with_moved_hours(Arc::clone(&self.overlay_moved));
                        if std::mem::take(&mut installed) {
                            // Laid out again at the view the side was asked with: placed at this
                            // frame's camera, as every other layout the side hands over is.
                            self.place_overlay(&held, overlay_camera, pixels, ui_scale, &tables, &rows, self.overlay_left);
                        }
                        let prepared = self.prepared_overlay.as_mut().expect("held above");
                        prepared
                            .draw_into(
                                &held,
                                overlay_camera,
                                pixels,
                                ui_scale,
                                &self.overlay_layers,
                                animation_time,
                                self.overlay.get_or_insert_with(Batch::default),
                                &mut self.counter_report,
                            )
                            .map_err(|error| error.to_string())?;
                        if self.label_report != *prepared.label_report() {
                            self.label_report.clone_from(prepared.label_report());
                        }
                        if self.rendered.is_some() {
                            break;
                        }
                        let pending = prepared.retries();
                        if rounds.next(&pending) {
                            continue;
                        }
                        if !pending.is_empty() {
                            viewer_foundation::map::redraws::note("made.icons_pending", pending.iter().map(|retry| retry.requests as u64).sum());
                        }
                        document.icons.lazy.settle();
                        if !prepared.is_stale(&held) && !document.icons.lazy.has_pending() {
                            break;
                        }
                        // The art of this hour arrived while the frame drew it, so the layout it
                        // drew is older than the art: the side lays the hour out again over the
                        // art, and a settle that cannot have that layout refuses the picture by
                        // name rather than saving one the art moved under.
                        settles += 1;
                        let laid = match self.playhead_map.as_mut() {
                            Some(_) if settles > SETTLES => Err(format!("the art of hour {} was still arriving after {SETTLES} layouts", rows.hour)),
                            Some(side) => side.lay_overlay_again(),
                            None => Err("the map's side is not open".to_owned()),
                        };
                        let made = match laid {
                            Ok(made) => made,
                            // A saved frame is whole or it is refused by name. A frame of a run
                            // that is playing draws what it has: the art reaches the frames after
                            // it, and no stopped picture is made of this one.
                            Err(why) if self.saves_a_frame() => {
                                return Err(format!("A saved frame draws the art of its own hour: the overlay of hour {} was not laid out again over the art that arrived while the frame drew it: {why}", rows.hour));
                            }
                            Err(_) => break,
                        };
                        self.install_overlay(made);
                        installed = true;
                        // A layout of its own asks the view's pools for its own windows, which have
                        // the pools' growth rounds the first layout's had.
                        rounds = viewer_foundation::map::icon_windows::DumpFrames::default();
                    }
                    if viewer_foundation::map::overlay_count::counting()
                        && let Some(batch) = self.overlay.as_ref()
                    {
                        self.overlay_counted.painted(batch).record();
                    }
                    // The batch painted carries no offset: it stands where the layout it drew was
                    // placed ([`Self::overlay_placed`], which the placing and the install keep),
                    // and the moves after it are measured from there. This frame's camera and head
                    // are not that placing's, so the painting is placed at them before it is drawn.
                    self.overlay_placed_view = Some((pixels, ui_scale.to_bits()));
                    self.overlay_shifts.clear();
                    self.overlay_unpainted = false;
                    self.place_painting(overlay_camera, pixels, ui_scale, at)?;
                }
                self.overlay_fraction = Some(document.hour_fraction);
                self.overlay_drawn_fraction = Some(document.hour_fraction);
            }
            // A round of the painting took a newer publication: what follows reads the overlay the
            // frame holds, with the tables and the rows it was laid out over.
            let rows = self.overlay_frame.clone().unwrap_or_else(|| Arc::clone(&rows));
            let tables = self.overlay_tables.clone().unwrap_or_else(|| Arc::clone(&tables));
            let now = crate::document::Hour::new(&document.content, &rows, 0.0).with_air_rosters(Some(document.playback.air_rosters())).with_moved_hours(Arc::clone(&self.overlay_moved));
            // Native windows keep ticking and drawing while active. Their clocks do not
            // change the hour/camera preparation, including the settled counter positions.
            {
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
            if let (Some((shown_branch, shown_hour)), Some((branch, drawn))) = (self.overlay_shown, self.layers_on_screen())
                && (shown_branch != branch || shown_hour != drawn)
            {
                viewer_foundation::map::redraws::note("overlay.hour_apart", 1);
                viewer_foundation::view_record::add("overlay", "hour_apart", 1);
                viewer_foundation::view_record::set("overlay", "hours_apart", u64::from(shown_hour.abs_diff(drawn)));
            }
            let running = self.counter_view.presentation.take_running();
            if running.any {
                viewer_foundation::map::redraws::note("asked.running", 1);
                // A window changed on its own: the next frame paints the batch. Nothing shown
                // changes but a pool has a pass to run: it runs the pools' passes alone. A mover
                // alone asks for the frame and nothing of the overlay: the frame answers it by
                // placing the windows at its own head, and the pools are left as they stand.
                if running.visible {
                    self.overlay_fraction = None;
                } else if running.windows.any() {
                    self.overlay_fraction = Some(f32::NAN);
                    self.overlay_pools = running.windows;
                } else {
                    viewer_foundation::map::redraws::note("asked.movement_only", 1);
                }
                ui.ctx().request_repaint();
            }
            // A free eye shows the laid-out icons standing in the world; this frame's hits and
            // drawing follow what it shows. Placed again only when what the placing reads changed.
            let presented = (overlay_camera, pixels, ui_scale.to_bits(), self.overlay_written, at);
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
                        at,
                        &mut self.free_overlay,
                    ))
                }
            } else {
                self.free_presented = None;
                None
            };
            // The frame's hit shapes stand from here: the pointer is resolved against them, once
            // for each point the rest of the frame asks about.
            self.hits_stamp += 1;
            if check_overlay() {
                // What the sprite pass's own check found while this frame uploaded: the bytes it
                // holds resident against a whole rebuild's, and a quiet frame's silence.
                let found = viewer_foundation::map::sprites::overlay_check_failures();
                if !found.is_empty() {
                    return Err(format!(
                        "the overlay's resident GPU bytes differ from the same batch uploaded afresh: {}",
                        found.join("; ")
                    ));
                }
                // The overlay's gate: the one the frame holds against the same hour, view and camera
                // laid out afresh. A still camera places nothing, so each overlay the side's maker
                // laid out is looked at once as well, which covers a run that never moves the camera.
                // And a frame that kept windows rather than painting them is looked at every tenth
                // frame: what the kept windows draw and report against a whole repaint's.
                self.overlay_looks = self.overlay_looks.wrapping_add(1);
                overlay_checks::note(&overlay_checks::LOOKED);
                // Only while the batch stands where it was painted: a batch whose windows have
                // been moved to this frame's camera and head draws the same content somewhere
                // else, so comparing it with a layout made at the placing would report the move
                // itself. The places of a moved batch are the anchor judge's
                // (`BATTLE_PLAN_VIEWER_CHECK_ANCHOR`), which checks every quad against its own anchor.
                let kept = (self.overlay_looks % 10 == 0 && self.overlay_shifts.is_empty())
                    .then_some("with its windows kept");
                if let Some(what) = painted_now
                    .then_some("as this frame painted it")
                    .or_else(|| placed_now.then_some("placed at this camera"))
                    .or_else(|| (self.overlay_checked != self.overlay_installed).then_some("as the step's maker laid it out"))
                    .or(kept)
                {
                    self.overlay_checked = self.overlay_installed;
                    // The held overlay stands for the camera it was placed at: a frame that moved
                    // its windows is judged against that layout, and its places by `check_anchor`.
                    let placed = self.overlay_placed.unwrap_or(overlay_camera);
                    self.check_overlay(what, &now, placed, pixels, ui_scale, animation_time)?;
                } else {
                    overlay_checks::note(&overlay_checks::NOTHING_NEW);
                }
            }
            // An explicit fleet choice remains separate from a task-force plate
            // click. Its roster also reaches empty fleets with no map plate.
            if *self.shown_mode()
                == viewer_world::map::map_mode::MapMode::Stock(
                    viewer_world::map::map_mode::Stock::StrategicNavy,
                )
            {
                let head = head_rows::head(live_of(&self.playhead_map));
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
            let _combat_cost = replay_frame::cpu::scope("frame.interface.combat", Some("other"));
            let engaged = self.layer_engaged();
            laps.lap("engaged");
            self.combat.set_playing(self.playing);
            let wake = ui.ctx().clone();
            self.combat
                .prepare_air(
                    &document.playback.replay_handle(),
                    viewer_foundation::worker_priority::waking(move || wake.request_repaint()),
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
            drop(_combat_cost);
            let _picker_cost = replay_frame::cpu::scope("frame.interface.picker", Some("other"));
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
                                let head = head_rows::head(live_of(&self.playhead_map));
                                // A click while the viewing country or the rows load selects nothing.
                                let read = ours.and_then(|viewer| self.head_rows.clicks(head, frame, |frame| viewer_counters::map::naval_selection::mission_members(frame, viewer, hit.region, hit.mission)));
                                let Some(members) = read_click(&mut self.overlay_loading, "the naval mission's members", read) else {
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
                                ui.ctx().request_repaint();
                            }
                            Some(MapHit::Base(index))
                                if self.counter_report.air_grid_hits.contains_key(&index) =>
                            {
                                if let Some(members) =
                                    self.counter_report.air_grid_hits[&index].members.clone()
                                {
                                    let shift = ui.input(|input| input.modifiers.shift);
                                    let head = head_rows::head(live_of(&self.playhead_map));
                                    let read = self.head_rows.clicks(head, frame, |frame| self.air_selection.grid_click(frame, &members, shift));
                                    if read_click(&mut self.overlay_loading, "the air grid's selection", read).unwrap_or(false) {
                                        self.unit_selection.clear();
                                        self.counter_view.selected =
                                            Arc::clone(self.unit_selection.units());
                                        ui.ctx().request_repaint();
                                    }
                                }
                            }
                            Some(MapHit::Base(index))
                                if self.counter_report.air_base_hits.contains_key(&index) =>
                            {
                                let (province, kind) = self.counter_report.air_base_hits[&index];
                                let shift = ui.input(|input| input.modifiers.shift);
                                let head = head_rows::head(live_of(&self.playhead_map));
                                let read = ours.and_then(|viewer| self.head_rows.clicks(head, frame, |frame| viewer_counters::map::air_selection::base_target(frame, viewer, province, kind)));
                                let target = read_click(&mut self.overlay_loading, "the air base's target", read).flatten();
                                if let Some(target) = target {
                                    self.choose_map_mode(viewer_world::map::map_mode::MapMode::Stock(
                                        viewer_world::map::map_mode::Stock::StrategicAir,
                                    ));
                                    let head = head_rows::head(live_of(&self.playhead_map));
                                    let read = self.head_rows.clicks(head, frame, |frame| self.air_selection.base_click(frame, target.clone(), shift));
                                    read_click(&mut self.overlay_loading, "the air base's selection", read);
                                    self.unit_selection.clear();
                                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
                                    ui.ctx().request_repaint();
                                }
                            }
                            Some(MapHit::AirMission(index))
                                if self.counter_report.air_mission_hits[index]
                                    .mission
                                    .is_some() =>
                            {
                                let hit = &self.counter_report.air_mission_hits[index];
                                let head = head_rows::head(live_of(&self.playhead_map));
                                let read = ours.and_then(|viewer| self.head_rows.clicks(head, frame, |frame| viewer_counters::map::air_selection::mission_members(frame, viewer, hit.region, hit.mission.unwrap())));
                                let Some(members) = read_click(&mut self.overlay_loading, "the air mission's members", read) else {
                                    continue;
                                };
                                let shift = ui.input(|input| input.modifiers.shift);
                                let read = self.head_rows.clicks(head, frame, |frame| self.air_selection.mission_click(frame, &members, shift));
                                if read_click(&mut self.overlay_loading, "the air mission's selection", read).unwrap_or(false) {
                                    self.unit_selection.clear();
                                    self.counter_view.selected = Arc::clone(self.unit_selection.units());
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
            drop(_picker_cost);
            let tooltip_point = point.filter(|_| !over_combat && !over_picker);
            let map_hit = self.map_tooltip(document, tooltip_point, extent, ui.ctx());
            self.combat_rows.end_frame();
            self.head_rows.end_frame();
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
                // Rows at the head of the open map with the vision its vision layer shows.
                let made = match (self.rows_head(), self.shown_vision()) {
                    (Some(views), Ok(vision)) => {
                        let (key, job) = inventory(&self.hover_inputs(document, &vision, ui.ctx()), &self.counter_report);
                        self.hover_rows.wait(hover_rows::Family::Capture, &views, key, job)
                    }
                    _ => None,
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
            if !self.overlay_missed.is_empty() {
                self.overlay_loading.push(format!("the overlay: loading: {:?}", self.overlay_missed));
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
        let rows = self.overlay_frame.clone().unwrap_or_else(|| Arc::clone(&rows));
        let tables = self.overlay_tables.clone().unwrap_or_else(|| Arc::clone(&tables));
        let now = crate::document::Hour::new(&document.content, &rows, 0.0).with_air_rosters(Some(document.playback.air_rosters())).with_moved_hours(Arc::clone(&self.overlay_moved));
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
        // A hovered province's state, from the rows the overlay was made from; the provinces are
        // among the tables the overlay's reader declares, so they are there with it.
        let state_of = |province: u32| {
            let provinces = now.frame.complete::<replay_data::rows::tables::Provinces>().ok()?;
            provinces.binary_search_by_key(&province, |row| row.id).ok().map(|at| provinces[at].state)
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
        let layered_radar = self.playhead_map.as_ref().and_then(|side| side.products().radar.clone());
        let layered = self.layered_orders();
        let scene = self.scene.as_mut().unwrap();
        let (device, queue) = (&state.device, &state.queue);
        // The radar symbols are the radar layer's with the map open, laid out or not.
        if let Some(field) = layered_radar.as_deref().or(self.prepared_overlay.as_ref().map(|prepared| &*prepared.radar)) {
            scene
                .apply(device, queue, SceneInput::SetRadar { field, shown: self.picker.radar, hovered: hovered_radar })
                .map_err(|e| e.to_string())?;
        }
        // The order parts: the layers' plan, the hour's raid arrows and the trade routes; the
        // overlay's air orders join them once laid out.
        let (plan, raids, routes) = layered.as_ref().map_or((None, None, None), |layered| (layered.plan.clone(), layered.raids.clone(), layered.routes));
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
                let routes_field = routes.and(live_of(&self.playhead_map)).and_then(|side| side.products().routes.as_ref()).map(|(_, routes)| &routes.field);
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
        let ours = Some(tables.ours());
        let (head, selected) = (head_rows::head(live_of(&self.playhead_map)), self.unit_selection.units().as_ref());
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
        // A window capture stays silent; measuring the map tab's own frames, and with
        // `BATTLE_PLAN_VIEWER_AUDIO_LOG`, it presents the voices it would start without opening a device.
        let silent = self.capture.is_some()
            && (self.campaign_tab || std::env::var_os("BATTLE_PLAN_VIEWER_AUDIO_LOG").or_else(|| std::env::var_os("HOI4_VIEWER_AUDIO_LOG")).is_some());
        if silent {
            viewer_audio::System::silent();
        }
        if (self.capture.is_none() || silent) && self.audio_focus {
            let _other_cost = replay_frame::cpu::scope("frame.audio", Some("other"));
            present_audio(
                &mut self.audio,
                document,
                scene,
                &camera,
                AudioInputs {
                    volumes: self.volumes,
                    muted: self.muted,
                    reset: std::mem::take(&mut self.audio_reset),
                    music: self.playhead_map.as_ref().and_then(|side| side.products().music.clone()).or_else(|| self.music.clone()),
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
        // Measurement only (`BATTLE_PLAN_VIEWER_SHOW_PICTURES`): how many of the frames this map
        // presents are a new picture, for the readout in its diagnostics strip.
        if timed {
            crate::screen_count::readout::count(&state.device, &state.queue, scene);
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
            // Laid out over the newest rows at the head with tables made from them, nothing
            // loading.
            let at_head = self.overlay_missed.is_empty() && rows.hour == shown_at;
            let laid_out = laid_out && at_head;
            // And until the raid offer cycle is complete at the shown hour: every target
            // evaluated, as a paused frame ends. That is the scripts reader's batch the tables
            // hold.
            let raids = Some(tables.raids());
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
                Some(side) => side.products().music.clone(),
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
        if software || diagnostics || partial || scoped || start_prompt.is_some() {
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
        self.map_hits.borrow_mut().end_frame();
        // The hour this frame put on screen is the head it drew everything at; without a head of
        // its own (no reader, a document's own frame) it is the hour the layers show.
        let (branch, layers_hour) = self.hour_on_screen(document);
        let (hour, fraction) = match shown_head {
            Some(head) => (head.hour, f64::from(head.fraction)),
            None => (layers_hour, f64::from(document.hour_fraction)),
        };
        self.rendered = Some(Playhead { branch, hour, fraction });
        Ok(())
    }
}

impl Viewer {
    /// Where the free mode now running stands on the map, on the map's first copy: the figure's
    /// feet, or the aircraft's place. `None` under the map camera, and while a free mode still
    /// waits for the ground it starts over.
    fn free_mode_ground(&self) -> Option<[f32; 2]> {
        match self.camera_mode {
            CameraMode::Map => None,
            CameraMode::Walk => self.walk.standing(),
            CameraMode::Fly => self
                .flight
                .as_ref()
                .map(|flight| [flight.position[0], flight.position[2]]),
        }
    }

    /// Switches how the full map's camera moves and whether the free modes draw the map's
    /// interface, and reports a change to the host, whose controls follow it. Leaving a free mode
    /// gives the pointer back and returns to the map camera it left; one chosen from the map
    /// waits for the ground it starts over, and one chosen while another free mode runs takes
    /// over the ground that one stands on, so every choice is taken at once.
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
        let standing = self.free_mode_ground();
        self.camera_mode = mode;
        self.start_pick = match (mode, standing) {
            (CameraMode::Map, _) => drop::Pick::Idle,
            (_, Some(at)) => drop::Pick::At(at),
            (_, None) => drop::Pick::Choosing,
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
        let _other_cost = replay_frame::cpu::scope("frame.logic", Some("other"));
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
        let pass = replay_frame::cpu::scope("frame.ui", Some("other"));
        engine_executable::executable_tables::scope(tables, || self.campaign_ui(ui, frame));
        drop(pass);
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
                embedded.report_maps(dashboard.states(), ui.ctx());
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
        {
            let _other_cost = replay_frame::cpu::scope("frame.load", Some("other"));
            self.poll(ui.ctx());
        }
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
                // measured. Needs the pinned `--camera` the harness passes.
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
            let drawn = self.window_frame(ui, frame.wgpu_render_state(), &mut document);
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
        {
            let _other_cost = replay_frame::cpu::scope("frame.report", Some("other"));
            self.report_embedded(ui.ctx());
        }
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

/// `BATTLE_PLAN_VIEWER_CHECK_OVERLAY`: the overlay the frame holds is laid out again against the same
/// hour, view and camera, and must draw the same quads and report the same
/// ([`Viewer::check_overlay`]). Sampled on a frame that painted the overlay, placed it at this
/// camera, took one the step's maker laid out, or, every tenth look, kept its windows.
fn check_overlay() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_OVERLAY").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_OVERLAY")).is_some())
}

/// What the overlay's content gate did over a run ([`Viewer::check_overlay`]): how often it looked
/// at a frame, how often it compared, and why it did not. A run's report carries it, so a gate that
/// judged almost nothing cannot pass for a clean run.
pub(crate) mod overlay_checks {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub(crate) static LOOKED: AtomicU64 = AtomicU64::new(0);
    pub(crate) static COMPARED: AtomicU64 = AtomicU64::new(0);
    /// Nothing was laid out, placed or kept long enough to compare on this frame.
    pub(crate) static NOTHING_NEW: AtomicU64 = AtomicU64::new(0);
    /// A flag or portrait decoded between the layout and the comparison, so the two layouts would
    /// draw one sprite at two versions.
    pub(crate) static ART_MOVED: AtomicU64 = AtomicU64::new(0);
    /// The fresh layout read a table the rows the overlay was made from do not hold.
    pub(crate) static ROWS_MISSED: AtomicU64 = AtomicU64::new(0);
    /// A map layer published its own product of the hour between the painting and the gate's own
    /// layout, so the two layouts are of two pictures.
    pub(crate) static LAYERS_MOVED: AtomicU64 = AtomicU64::new(0);
    pub(crate) static NO_OVERLAY: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn note(counter: &AtomicU64) {
        counter.fetch_add(1, Relaxed);
    }

    /// Looked, compared, nothing new, the art moved, the rows were short of a table, a layer's
    /// product moved, no overlay.
    pub(crate) fn counted() -> [u64; 7] {
        [&LOOKED, &COMPARED, &NOTHING_NEW, &ART_MOVED, &ROWS_MISSED, &LAYERS_MOVED, &NO_OVERLAY].map(|counter| counter.load(Relaxed))
    }
}

/// Whether the anchored overlay's judge runs ([`Viewer::check_anchor`]).
fn check_anchor() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_ANCHOR").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_ANCHOR")).is_some())
}

/// Where an anchor stands now: the page composed it `age` ago on its own clock, and a `moving`
/// clock has played on since, so delivery delay adds no error. A seek and a pause stand exactly
/// where the page composed them. Never past the branch's last hour.
fn placed_anchor(target: Playhead, moving: bool, speed: f64, age: Duration, last: i32) -> Playhead {
    let mut placed = target;
    if moving && speed > 0.0 {
        let fraction = placed.fraction + age.as_secs_f64() * speed;
        advance_clock(&mut placed, fraction, last);
    }
    placed
}

/// A playing clock never goes back, whatever a late anchor says and whether the speed or playing
/// changed with it: it stands at the later of where it is and where the anchor places it. A seek,
/// a pause and a branch change take the anchor exactly.
fn forward_only(current: Option<Playhead>, placed: Playhead, moving: bool) -> Playhead {
    match current {
        Some(current) if moving && current.branch == placed.branch => {
            let lead = f64::from(current.hour) - f64::from(placed.hour) + current.fraction
                - placed.fraction;
            if lead > 0.0 { current } else { placed }
        }
        _ => placed,
    }
}

fn tick_clock(head: &mut Playhead, speed: f64, seconds: f64, last: i32) -> bool {
    let step = seconds.max(0.0) * speed;
    advance_clock(head, head.fraction + step, last)
}

/// Moves a playing playhead `fraction` hours past its hour, whether or not the map's layers have
/// reached it: the map shows the newest hour they have composed at or before it. Returns false at
/// the branch's last hour.
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

/// Whether the picture the map draws answers `hour` whole.
///
/// Playing, the layers decide it: they are the hour, and the frame draws over them the newest
/// finished overlay at or before the hour they show, with the memory it was laid out on. An overlay
/// a step behind is a step of its own hours behind on the counters and never holds the picture,
/// because a step that waited for it would hold every frame of the picture instead
/// ([`crate::playhead_host::overlay`]).
///
/// Where the hour shown must be exact, the overlay is part of it again: the hours have stopped (a
/// pause, a speed of zero, the end of the recording), so the overlay reaches the layers' hour
/// within one of its maker's cycles and the picture waits that long for it; or the frame is saved,
/// or it is a seek's first picture, both of which ask the maker for exactly their hour
/// ([`crate::layers::Settling::Whole`]).
fn whole_picture(
    on: &crate::playhead_host::OnScreen,
    overlay: Option<replay_data::Hour>,
    hour: i32,
    playing: bool,
) -> bool {
    if !layers_show(on, hour, playing) {
        return false;
    }
    match playing {
        // Stopped: the hour shown is exact, the overlay's hour included.
        false => overlay == Some(hour),
        // Playing: never an overlay of a later hour than the layers show, and never one from
        // before the seek the picture is of; an earlier hour of this history holds nothing.
        true => overlay.is_none_or(|shown| on.drawn.is_none_or(|drawn| shown <= drawn) && on.sought.is_none_or(|sought| sought <= shown)),
    }
}

fn layers_show(on: &crate::playhead_host::OnScreen, hour: i32, playing: bool) -> bool {
    on.drawn.is_some_and(|drawn| match playing {
        // The seek the picture is of must be one the clock has reached: a picture of a seek beyond
        // the hour asked for answers another history, however whole it is of its own. A picture
        // past the hour asked for, or one a reset is replacing, is of the history before a seek.
        true => on.sought.is_none_or(|sought| sought <= drawn && sought <= hour) && drawn <= on.head && drawn <= hour && !on.composing,
        false => drawn == hour && on.settled,
    })
}

/// A refused map reader, by name, as the map's error view shows it.
fn refusal(error: &std::io::Error) -> String {
    format!("The map's playhead is refused: {error}")
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

/// A click's read: none while its rows load; a failed read is a named line and selects nothing.
fn read_click<T>(lines: &mut Vec<String>, what: &str, read: Option<std::io::Result<T>>) -> Option<T> {
    match read.transpose() {
        Ok(value) => value,
        Err(error) => {
            lines.push(format!("{what}: {error}"));
            None
        }
    }
}

/// A saved frame's unit figures are the figures of its hour: their models and meshes are made on
/// a worker at a cadence of its own, so the frame's own settling does not gate on them and two
/// runs of one binary drew a different number of them. The frame settles the map's layers again
/// until they stand at `hour`, within `limit`; refused by name.
fn settle_models(side: &mut crate::playhead_host::MapSide, hour: replay_data::Hour, limit: Duration) -> Result<(), String> {
    let until = Instant::now() + limit;
    loop {
        let Some(units) = side.draws("units").then(|| crate::layers::units::of(side)).flatten() else { return Ok(()) };
        if units.figures_stand(hour) {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(format!("The unit figures at hour {hour}: {} after {limit:?}", units.figures_state()));
        }
        // A whole settle waits on the workers themselves: for the rows of every stack handed and
        // for the product of the build it starts, so this loop never spins.
        side.settle_all().map_err(|error| format!("The unit figures at hour {hour}: {error}"))?;
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

/// The playhead's map while its reader has not failed: the one the window takes everything from.
fn live_of(map: &Option<crate::playhead_host::side::SideThread>) -> Option<&crate::playhead_host::side::SideThread> {
    map.as_ref().filter(|side| side.failure().is_none())
}

/// The head the side prepares its next step for: a paced capture's clock hour, the whole picture
/// of a saved frame's hour (a stepped capture, a window dump), or the playhead's own clock.
/// `capture` is whether a capture is open and paced; `drawing` is whether this frame draws the
/// map, which a frame before the first picture, or before it has placed the camera and size the
/// overlay is made for, does not: it asks for no overlay, so it waits for no whole picture.
fn head_wanted(capture: Option<bool>, hour: Option<replay_data::Hour>, window_dump: bool, drawing: bool) -> crate::playhead_host::Head {
    use crate::playhead_host::Head;
    match (capture, hour) {
        (Some(paced), Some(hour)) if paced || !drawing => Head::Paced(hour),
        (Some(_), Some(hour)) => Head::Stepped(hour),
        (None, Some(hour)) if window_dump && drawing => Head::Stepped(hour),
        _ => Head::Clock,
    }
}

/// A frame that draws no map waits for no whole picture: before the first picture is composed, or
/// before a frame has placed the camera and size the overlay is made for, there is no overlay to
/// wait for, so the side composes and the first frame that draws is the first one an overlay is
/// made for. A stepped capture's frame and a window dump that draw wait for the whole picture of
/// their hour.
#[cfg(test)]
#[test]
fn a_frame_that_draws_no_map_waits_for_no_whole_picture() {
    use crate::playhead_host::Head;
    assert_eq!(head_wanted(Some(false), Some(7), false, true), Head::Stepped(7));
    assert_eq!(head_wanted(Some(false), Some(7), false, false), Head::Paced(7));
    assert_eq!(head_wanted(Some(true), Some(7), false, true), Head::Paced(7), "a paced capture waits for no picture");
    assert_eq!(head_wanted(None, Some(7), true, true), Head::Stepped(7));
    assert_eq!(head_wanted(None, Some(7), true, false), Head::Clock);
    assert_eq!(head_wanted(None, Some(7), false, true), Head::Clock, "a live window follows its clock");
    assert_eq!(head_wanted(None, None, true, true), Head::Clock);
}

/// A deferred save may build unused warnings, but an unsavable frame never needs them.
fn builds_warnings(diagnostics: bool, capture: Option<&Capture>) -> bool {
    diagnostics || capture.is_some_and(|capture| capture.remaining == 0)
}

#[cfg(test)]
pub(crate) mod tests {
    /// Playing, the layers are the picture's hour and the overlay is not: a layout a step behind is
    /// drawn over them and holds nothing, because a step that waited for it would hold every frame
    /// instead. An overlay of a later hour than the layers show, or of a seek before this picture's,
    /// is not this picture's and does hold it. Stopped, the hour is exact again: the overlay reaches
    /// the layers' hour within one of its maker's cycles and the picture waits that long for it.
    #[test]
    fn playing_the_layers_are_the_pictures_hour_and_stopped_the_overlay_is_too() {
        let on = |drawn: Option<i32>, sought: Option<i32>| crate::playhead_host::OnScreen {
            head: 10,
            drawn,
            sought,
            composing: false,
            settled: true,
            furthest: None,
            resets: 0,
        };
        let whole = |overlay: Option<i32>, hour: i32, playing: bool| super::whole_picture(&on(Some(8), None), overlay, hour, playing);
        assert!(whole(Some(8), 8, true), "the overlay of the layers' own hour");
        assert!(whole(Some(5), 8, true), "a layout a step behind: drawn over the layers, holding nothing");
        assert!(whole(None, 8, true), "before the first layout, the layers are the picture");
        assert!(!whole(Some(9), 8, true), "never an overlay of an hour the layers have not reached");
        // A seek's picture is its own history's: an overlay from before it is not this picture's.
        assert!(!super::whole_picture(&on(Some(8), Some(7)), Some(6), 8, true));
        assert!(super::whole_picture(&on(Some(8), Some(7)), Some(7), 8, true));
        // Sought back from 8 to 6, before the reset is drawn: the picture of 8 is the history
        // before the seek, whether or not the reset has begun composing.
        assert!(!super::whole_picture(&on(Some(8), Some(6)), None, 6, true), "a picture past the hour asked for");
        let composing = crate::playhead_host::OnScreen { composing: true, ..on(Some(6), Some(6)) };
        assert!(!super::whole_picture(&composing, None, 6, true), "a picture a reset is replacing");
        // Stopped (a pause, a speed of zero, the end of the recording): exactly the hour shown.
        assert!(whole(Some(8), 8, false));
        assert!(!whole(Some(7), 8, false), "a paused picture is never left with an earlier overlay");
        assert!(!whole(None, 8, false));
    }

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
            composed_at: 0.0,
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
            assert_eq!(forward_only(Some(current), target, true), current);
            assert_eq!(forward_only(Some(current), target, false), target);
        }
        let later = Playhead {
            hour: 21,
            fraction: 0.125,
            ..current
        };
        assert_eq!(forward_only(Some(current), later, true), later);
        let branch = Playhead {
            branch: 2,
            hour: 10,
            ..current
        };
        assert_eq!(forward_only(Some(current), branch, true), branch);
        assert_eq!(forward_only(None, current, true), current);
    }

    /// An anchor the page composed while playing stands where the page's clock is now, not where
    /// it was when it composed it: a quarter of a second of delivery at 1,440 hours a second is
    /// 360 game hours. A seek and a pause stand exactly where they were composed, however late.
    #[test]
    fn a_playing_anchor_is_placed_by_the_time_the_page_composed_it() {
        let target = Playhead {
            branch: 1,
            hour: 20,
            fraction: 0.5,
        };
        let placed = placed_anchor(target, true, 1_440.0, Duration::from_millis(250), 100_000);
        assert_eq!(placed.hour, 380);
        assert!((placed.fraction - 0.5).abs() < 1e-9, "{placed:?}");
        assert_eq!(
            placed_anchor(target, true, 1_440.0, Duration::ZERO, 100_000),
            target,
            "an anchor that arrived at once stands where it was composed"
        );
        assert_eq!(
            placed_anchor(target, false, 1_440.0, Duration::from_millis(250), 100_000),
            target,
            "a seek and a pause stand exactly where the page composed them"
        );
        assert_eq!(
            placed_anchor(target, true, 1_440.0, Duration::from_secs(10), 300).hour,
            300,
            "never past the branch's last hour"
        );
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
        viewer.anchor_playback(host, true, 4.0, true, None, now, 1_000);
        for frame in 1..=60 {
            let at = now + Duration::from_secs_f64(f64::from(frame) / 60.0);
            if frame % 3 == 0 {
                let delayed = Playhead {
                    fraction: 0.0,
                    hour: 100 + (frame - 3) / 15,
                    ..host
                };
                viewer.anchor_playback(delayed, true, 4.0, false, None, at, 1_000);
            }
            viewer.tick_playhead(at.duration_since(viewer.last_step).as_secs_f64(), 1_000);
            viewer.last_step = at;
            let head = viewer.playhead.unwrap();
            let position = f64::from(head.hour) + head.fraction;
            assert!((position - (100.0 + f64::from(frame) / 15.0)).abs() < 1e-7);
        }
        viewer.anchor_playback(host, false, 4.0, false, None, now + Duration::from_secs(1), 1_000);
        assert_eq!(viewer.playhead, Some(host));
        assert!(!viewer.playing);
        viewer.anchor_playback(host, true, 0.25, false, None, now + Duration::from_secs(1), 1_000);
        assert_eq!(viewer.playhead, Some(host));
        viewer.tick_playhead(0.5, 1_000);
        assert_eq!(viewer.playhead.unwrap().fraction, 0.125);
        viewer.anchor_playback(host, true, 0.25, true, None, now + Duration::from_secs(1), 1_000);
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
        viewer.anchor_playback(head, true, 4.0, true, None, now, 100);
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
            None,
            now + Duration::from_millis(500),
            100,
        );
        assert_eq!(viewer.playhead.unwrap().hour, 100);
        assert!(!viewer.playing);
    }

    /// Every transport action, each anchor reaching the map late: a speed change and a resume
    /// never take the map to an hour before the one it shows, delivery delay adds no error, and a
    /// pause and a seek stand exactly where the page composed them. The page's clock reads 5,000
    /// more than this process's, which the map learns from the anchors themselves.
    #[test]
    fn every_transport_action_places_its_late_anchor_without_moving_the_map_backwards() {
        const LAST: i32 = 100_000;
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
        let head = |position: f64| Playhead {
            branch: 1,
            hour: position.floor() as i32,
            fraction: position - position.floor(),
        };
        let position = |viewer: &Viewer| {
            let head = viewer.playhead.expect("a clock");
            f64::from(head.hour) + head.fraction
        };
        let tick_to = |viewer: &mut Viewer, ms: u64| {
            let at = now + Duration::from_millis(ms);
            viewer.tick_playhead(at.duration_since(viewer.last_step).as_secs_f64(), LAST);
            viewer.last_step = at;
        };
        // Each anchor is stamped on the page's clock when the page composed it, and arrives later.
        let send = |viewer: &mut Viewer, at: f64, playing, speed, sought, composed: u64, arrived: u64| {
            viewer.anchor_playback(
                head(at),
                playing,
                speed,
                sought,
                Some(5_000.0 + composed as f64),
                now + Duration::from_millis(arrived),
                LAST,
            );
        };
        // The first anchor sets the page clock's offset, so its own delivery is not corrected for.
        send(&mut viewer, 1_000.0, true, 1_440.0, true, 0, 0);
        assert_eq!(position(&viewer), 1_000.0);
        assert!(viewer.playing);
        // A speed change the page composed at its hour 1,144, arriving 20 ms later: the map has
        // played to 1,172.8 meanwhile and stays there instead of jumping 28.8 hours back.
        tick_to(&mut viewer, 120);
        assert!((position(&viewer) - 1_172.8).abs() < 1e-6, "{}", position(&viewer));
        send(&mut viewer, 1_144.0, true, 24.0, false, 100, 120);
        assert!((position(&viewer) - 1_172.8).abs() < 1e-6, "a speed change never steps back: {}", position(&viewer));
        assert_eq!(viewer.speed, 24.0);
        // A pause is exact, even where the map had played past the page's own clock.
        tick_to(&mut viewer, 1_120);
        send(&mut viewer, 1_168.48, false, 24.0, false, 1_120, 1_140);
        assert_eq!(viewer.playhead, Some(head(1_168.48)), "a pause stands where it was composed");
        assert!(!viewer.playing);
        // Resuming at 1,440 hours a second, the anchor 40 ms old: the map starts where the page's
        // clock stands now, 57.6 hours on, not where the page was when it composed the anchor.
        send(&mut viewer, 1_168.48, true, 1_440.0, false, 1_200, 1_240);
        assert!((position(&viewer) - 1_226.08).abs() < 1e-4, "{}", position(&viewer));
        assert!(viewer.playing);
        // A seek is exact however late, and backwards.
        send(&mut viewer, 500.0, true, 1_440.0, true, 1_300, 1_400);
        assert_eq!(viewer.playhead, Some(head(500.0)), "a seek stands where it was composed");
        // A step while paused is a seek of one hour, exact.
        send(&mut viewer, 501.0, false, 1_440.0, true, 1_500, 1_560);
        assert_eq!(viewer.playhead, Some(head(501.0)));
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
