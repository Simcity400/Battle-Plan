use crate::{
    Config,
    Options,
    document::{Document, InitialHour},
    hours,
    map::{
        camera::{Camera, MapSize},
        frame::DayNight,
        scene::Scene,
    },
    // capture
    render,
};
// names
use crate::map;
use crate::map::{
    // counters
    counters::CounterReport,
    labels::LabelReport,
    name_pass::{LayerFade, Placements},
    scene::{Overlay, Request},
    sprites::Batch,
};
use eframe::{egui, wgpu};
use replay::{branches::Parentage, frame::TableView, rows::TableId};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

mod embedded;

/// Per-rebuild CPU laps of one window frame, printed with `BATTLE_PLAN_VIEWER_PROFILE`.
struct Laps {
    enabled: bool,
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
        let now = Instant::now();
        let outside = FRAME_END
            .lock()
            .ok()
            .and_then(|end| *end)
            .map_or(0.0, |end| {
                (now.saturating_duration_since(end).saturating_sub(waited)).as_secs_f64() * 1000.0
            });
        Self {
            enabled: map::counter_move::profiling(),
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
        crate::profiling::laps_line(&self.laps, 0.05)
    }
}

/// Milliseconds the frame's submission takes on the GPU, under
/// `BATTLE_PLAN_VIEWER_GPU_WAIT`.
///
/// Waiting out the submission is the only way to see the GPU's own time without
/// timestamp queries, and it costs the frame that wait, so an ordinary run and
/// a profiled run without the variable never call it and report 0.
fn gpu_wait(device: &wgpu::Device) -> f64 {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
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
    let period = Duration::from_secs_f64(1.0 / crate::profiling::hz());
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

/// Where the timeline is: the recorded hour the clock is in and how far past it.
///
/// The map draws the last hour the builders finished (`crate::hours`), which
/// is this one unless a seek or fast playback outran them; the playhead never
/// waits for the map.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Playhead {
    branch: replay::BranchId,
    hour: i32,
    fraction: f32,
}

/// What the frame path keeps of the hour on screen once its layers are uploaded.
struct ShownHour {
    epoch: u64,
    branch: replay::BranchId,
    hour: i32,
    /// Why the hour draws nothing: a recorded gap or a failed builder part.
    error: Option<String>,
    political_warnings: Vec<String>,
    /// The battle plan's refusals and missing inputs ([`map::orders::OrderReport::warnings`]).
    plan_warnings: Vec<String>,
    /// The map buildings' refused type tokens
    /// ([`map::map_buildings::MapBuildingSummary::warnings`]).
    building_warnings: Vec<String>,
    /// The unit models' omitted stacks by reason
    /// ([`map::unit_models::ModelReport::warnings`]).
    model_warnings: Vec<String>,
    political_summary: (usize, usize),
    political_build: Duration,
    limitations: map::tables::Limitations,
    /// What the weather table held, the snow and mud grid's size and how long
    /// it took to rasterise. Kept raw so ordinary viewing formats nothing.
    snow: (map::weather::Summary, [u32; 2], Duration),
    /// The header's country list and viewing tag, built with the hour.
    header: Arc<hours::Header>,
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
/// tooltip (`map::ui_scale`) it is laid out in interface pixels: `points` is
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

/// The ids of the shared hour layers the scene holds (`hours::Shared`), so an
/// hour that shares one with the hour on screen uploads nothing for it.
#[derive(Clone, Default)]
struct Uploaded {
    political: Option<u64>,
    borders: Option<u64>,
    rails: Option<u64>,
    buildings: Option<u64>,
    /// The snow and mud grid on the GPU. Weather changes only in an hour's own
    /// update window, so most hours rasterise the same grid; comparing it is a
    /// memcmp beside a whole-texture write and the winter-volume pass.
    snow_mud: Option<Arc<map::weather::SnowMud>>,
    /// The intel map on the GPU; hours with the same fog share one.
    intel: Option<Arc<map::textures::IntelMap>>,
    /// The battle plan on the GPU; hours with the same plan tables share one
    /// (`map::orders::TABLES`).
    orders: Option<Arc<map::orders::OrderField>>,
}

/// The largest share of the central panel the warning area may take before it
/// scrolls instead of growing.
///
/// It lists what the hour could not draw, which `--dump` prints too, so all of
/// it stays reachable; but unbounded it took the map's place entirely. At
/// 1200x800 the persistent content limitations alone wrapped past the bottom of
/// the panel and left the map one pixel tall.
const WARNING_SHARE: f32 = 0.4;

/// The height the recorded-table list scrolls within, for the same reason: 148
/// tables wrap to twenty lines, and the bottom panel grew to hold every one.
const TABLE_LIST_HEIGHT: f32 = 96.0;

/// Shows or hides diagnostics in the standalone and embedded viewer; the
/// desktop's web interface uses the same key.
const DIAGNOSTICS_KEY: egui::Key = egui::Key::F3;

/// Whether `event` is the diagnostics shortcut: a first press of the key with
/// no modifier, as the web interface's handler takes it.
fn is_diagnostics_shortcut(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if *key == DIAGNOSTICS_KEY && modifiers.is_none()
    )
}

/// Whether diagnostics start shown: `--diagnostics` for this run, otherwise
/// the standalone viewer's saved choice. An embedded viewer waits for the
/// desktop host's choice instead of reading `viewer.json`.
fn initial_diagnostics(requested: bool, saved: bool, embedded: bool) -> bool {
    requested || (!embedded && saved)
}

// capture
/// A readback of the window's map target, pending until a frame has been drawn.
///
/// The window does not draw the map into the swap chain: it renders it through
/// [`Scene`] into the same offscreen `OUTPUT_FORMAT` texture a `--dump` renders
/// into, and only hands egui the finished texture. That is the 8-bit graded
/// output and never the float scene behind it (`map::scene::Targets`), so egui
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
    pinned: Option<([u32; 2], Option<map::camera::Placement>)>,
    /// `--window-dump` closes the window once it has written its frame.
    exit: bool,
    time: Option<f32>,
    step: f32,
    frames: u32,
    index: u32,
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
    accepted: Option<(&std::path::Path, Option<&str>)>,
    pending_load: Option<(&std::path::Path, Option<&str>)>,
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
type MatchGame = ((u64, usize), (f32, Option<String>));

/// What an overlay was laid out for: the hour, the camera and frame, the perspective and the
/// counter options.
#[derive(PartialEq)]
struct OverlayKey {
    branch: replay::BranchId,
    hour: i32,
    camera: Camera,
    pixels: [u32; 2],
    ui_scale: f32,
    perspective: map::perspective::Choice,
    counter_settings: map::counters::CounterSettings,
}

pub struct Viewer {
    dashboard: Option<Box<crate::dashboard_maps::DashboardMaps>>,
    dashboard_active: bool,
    dashboard_map: bool,
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
    /// Off by default: the standalone viewer saves the choice in
    /// `viewer.json`, the desktop host sends its own, and `--diagnostics`
    /// turns it on for one run.
    pub(crate) diagnostics: bool,
    /// Whether an embedded map's diagnostics box lists its data limitations.
    diagnostics_open: bool,
    /// egui points per interface pixel at the last map frame, which lays out
    /// the diagnostics box over an embedded map ([`map_overlay`]).
    pub(crate) interface_points: f32,
    embedded: Option<crate::embedded::Controller>,
    embedded_open: Option<crate::embedded::OpenRequest>,
    embedded_seek: Option<crate::embedded::Seek>,
    revision: Option<String>,
    expected_revision: Option<String>,
    hidden_since: Option<Instant>,
    combat: crate::combat::Window,
    config: Config,
    options: Options,
    game_text: String,
    user_text: String,
    settings: bool,
    document: Option<Document>,
    loading: Option<mpsc::Receiver<Result<LoadedReplay, String>>>,
    error: Option<String>,
    scene: Option<Scene>,
    texture: Option<egui::TextureId>,
    generation: u64,
    map_error: Option<String>,
    camera: Option<map::camera::Placement>,
    camera_manual: bool,
    playing: bool,
    speed: u32,
    last_step: Instant,
    started: Instant,
    /// When the replay was opened, so a profiled run reports the cold open:
    /// the loading thread's document and scene, then the first hour built.
    opened: Option<Instant>,
    // colour
    /// Whether the map is graded through the engine's post chain.
    postfx: bool,
    pub(crate) graphics: dashboard::protocol::MapGraphics,
    /// The screen-space map interface's scale setting; see [`map::ui_scale`].
    /// The desktop host sets it with `MapAction::UiScale`; the standalone
    /// window keeps it in `viewer.json`.
    pub(crate) ui_scale: map::ui_scale::Setting,
    /// The Match game scale and its note, keyed by the monitors' generation
    /// and the document's content, so a frame only re-resolves on a change.
    match_game: Option<MatchGame>,
    /// The monitor size and display scale the window last saw; a change asks
    /// for the monitors to be read again.
    monitor_seen: Option<(Option<egui::Vec2>, Option<f32>)>,
    map_preferences_seen: Option<(String, Option<Vec<String>>, u32, u8)>,
    // hours
    /// The timeline's position; the map follows it as hours are built.
    playhead: Option<Playhead>,
    /// A seek waiting for the next built hour to start a fresh animation
    /// history (`map::presentation`).
    animation_seek: bool,
    /// The last host clock target, to tell its seeks from playback.
    map_animation_target: Option<map::presentation::Target>,
    /// The worker threads that build every hour's layers off the frame path.
    hours: Option<hours::Pipeline>,
    /// The engaged-flag index of the recording last closed, for a newer
    /// snapshot of it to continue.
    engaged_carry: Option<Arc<crate::engaged_index::EngagedIndex>>,
    /// The builders' generation the window keeps hours of.
    epoch: u64,
    /// Built hours not yet shown.
    ready: BTreeMap<(replay::BranchId, i32), hours::Built>,
    /// The hour whose layers are in the scene.
    shown: Option<ShownHour>,
    /// The camera the last frame drew, which the next hours' icons are built for.
    last_view: Option<hours::View>,
    /// Which shared layers the scene holds.
    uploaded: Uploaded,
    /// Frees the hours the window let go of off the frame path.
    dropper: Option<Arc<hours::Dropper>>,
    // close zoom
    unit_models: Option<map::unit_models::PreparedModels>,
    /// Whether the scene holds `unit_models`' field. It does not move within
    /// the hour, so each installed hour uploads it once.
    unit_field_uploaded: bool,
    /// The playback fraction the scene's raid units were last placed at.
    raid_tick: Option<f32>,
    // names
    placements: Option<Arc<Placements>>,
    placements_at: Option<(replay::BranchId, i32)>,
    /// `None` until the first frame seeds it settled at that frame's own camera
    /// height; see [`Viewer::map`].
    fade: Option<LayerFade>,
    last_frame: Instant,
    label_report: LabelReport,
    /// The raid offer cycle, advanced off the frame path.
    raid_lane: crate::raid_lane::RaidLane,
    raid_tooltips: map::raids::tooltips::lane::Lane,
    raid_tooltip_view: map::raids::tooltips::view::View,
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
    /// This view's counter options (the user's `settings.txt`, switchable in the header as the
    /// game's options are) and its counters' memory from one update to the next; a new
    /// document starts it afresh.
    counter_view: map::counters::CounterView,
    /// The hour's camera-independent icon tables, parsed once per (branch,
    /// hour, perspective) and reused by every frame's icon rebuild while the
    /// camera moves. A still camera never rebuilds at all (`overlay_at`), so
    /// this only pays on pans and zooms; without it each of those frames
    /// re-parsed every table the layout reads.
    overlay_tables: Option<(
        replay::BranchId,
        i32,
        map::perspective::Choice,
        crate::document::OverlayTables,
    )>,
    /// Which country the counters are drawn for; the header's own control sets
    /// it and `Document::perspective` is kept in step with it.
    perspective: map::perspective::Choice,
    /// The viewing-country chooser's tag filter.
    country_filter: String,
    // capture
    /// The readback to run once the next frame is drawn; F12 and
    /// `--window-dump` both arm it.
    capture: Option<Capture>,
    /// What the last capture wrote, or why it failed, for the header.
    capture_note: Option<String>,
    /// How a `--window-dump` failure leaves the process; see `main::run`.
    failure: Arc<Mutex<Option<String>>>,
    /// Names the F12 captures of one session apart when `--out` did not.
    captures: u32,
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
    let mut scene = Scene::new(device, queue, &document.assets, &document.icons, graphics)?;
    scene.set_borders(
        device,
        queue,
        &document.border_assets,
        &document.border_meshes,
        &document.straits,
    )?;
    scene.set_rivers(device, queue, &document.river_assets, &document.river_mesh)?;
    scene.set_trees(
        device,
        queue,
        &document.tree_assets,
        document.tree_geometry.clone(),
        &document.tree_field,
    )?;
    scene.set_sprawl(device, queue, &document.sprawl)?;
    scene.warm_trains(device, queue, &document.trains)?;
    scene.warm_aircraft(device, queue, &document.aircraft)?;
    scene.warm_unit_models(device, queue, &document.unit_models)?;
    scene.warm_map_buildings(device, queue, &document.map_buildings)?;
    scene.warm_bridges(device, queue, &document.bridges)?;
    scene.warm_railways(device, queue, &document.rail_assets)?;
    scene.warm_orders(device, queue, &document.orders)?;
    scene.warm_weather(device, queue, document.weather_effects.clone())?;
    scene.warm_political(device, queue, &document.content)?;
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
            .prepare_overlay_cached(camera, pixels, ui_scale, tables, &self.counter_view)
            .map_err(|error| error.to_string())?;
        let (mut batch, mut report) = (Batch::default(), CounterReport::default());
        full.draw_into(
            now,
            camera,
            pixels,
            ui_scale,
            Default::default(),
            time,
            &mut batch,
            &mut report,
        )
        .map_err(|error| error.to_string())?;
        let same = crate::document::same_quads(&batch.quads, &drawn.quads)
            && full.raid_report() == prepared.raid_report()
            && full.label_report() == prepared.label_report()
            && report == self.counter_report;
        crate::map::counter_move::profile_print(format!(
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

    fn configure_service(&self) {
        if let Some(embedded) = &self.embedded {
            embedded
                .service()
                .configure_content(&self.config, self.options.allow_unverified);
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
                    viewer.settings = true;
                }
            }
        }
        if (!viewer.settings || viewer.options.window_dump)
            && let Some(path) = viewer.options.replay.clone()
        {
            viewer.open(path, &cc.egui_ctx, None);
        }
        viewer
    }

    fn create(
        config: Config,
        options: Options,
        error: Option<String>,
        failure: Arc<Mutex<Option<String>>>,
        embedded: Option<crate::embedded::Controller>,
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
    ) -> Self {
        if let Some(embedded) = &embedded {
            embedded
                .service()
                .configure_content(&config, options.allow_unverified);
        }
        // capture: `--window-dump` arms the readback before the first frame, so
        // every frame it draws is a capture frame and the animation clock never
        // starts.
        let capture = options.window_dump.then(|| Capture {
            path: options.out.clone().unwrap_or_default(),
            remaining: if map::counter_move::profiling() {
                // A cold seek is followed for thirty seconds past it: a seek
                // across a 1,400-hour branch walks its snow history for ~10 s.
                if std::env::var("BATTLE_PLAN_VIEWER_PROFILE").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE")).is_ok_and(|mode| mode == "seek_end") {
                    1921
                } else {
                    crate::profiling::sweep().end
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
        // counters: the header's control starts where the command line put it.
        let perspective = options.perspective.clone();
        // Replaced with the document's `settings.txt` options when one loads.
        let counter_view = map::counters::CounterView::new(
            options.counter_settings(map::counters::CounterSettings::default()),
        );
        let hidden_since = embedded.as_ref().map(|_| Instant::now());
        // The monitors are read off the UI thread; start before the first map.
        map::ui_scale::refresh_monitors(|| ());
        // Embedded, the host owns the setting and sends it before the map.
        let ui_scale = if embedded.is_some() {
            map::ui_scale::Setting::MatchGame
        } else {
            map::ui_scale::Setting::from_stored(config.ui_scale)
        };
        Self {
            dashboard: None,
            dashboard_active: false,
            dashboard_map: false,
            shared_resources: None,
            source: None,
            source_decoder: None,
            loaded_config: None,
            rendered: None,
            rendered_sequence: 0,
            map_warnings: Vec::new(),
            diagnostics: initial_diagnostics(
                options.diagnostics,
                config.diagnostics,
                embedded.is_some(),
            ),
            diagnostics_open: false,
            interface_points: 1.0,
            embedded,
            embedded_open: None,
            embedded_seek: None,
            revision: None,
            expected_revision: None,
            hidden_since,
            combat: crate::combat::Window::default(),
            game_text: config.game_dir.to_string_lossy().into_owned(),
            user_text: config.user_dir.to_string_lossy().into_owned(),
            settings: config.resolve_options().is_err(),
            camera: options.camera,
            camera_manual: options.camera.is_some(),
            // colour: `--postfx off` reaches the window too, so a window and a
            // dump of the same state can be the same frame; the checkbox starts
            // where the command line put it.
            postfx: options.postfx,
            graphics: Default::default(),
            ui_scale,
            match_game: None,
            monitor_seen: None,
            map_preferences_seen: None,
            config,
            options,
            document: None,
            loading: None,
            error,
            scene: None,
            texture: None,
            generation: 0,
            map_error: None,
            playing: false,
            speed: 24,
            last_step: Instant::now(),
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
            // names
            placements: None,
            placements_at: None,
            fade: None,
            last_frame: Instant::now(),
            label_report: LabelReport::default(),
            raid_lane: Default::default(),
            raid_tooltips: Default::default(),
            raid_tooltip_view: Default::default(),
            // counters
            counter_report: CounterReport::default(),
            overlay: None,
            prepared_overlay: None,
            overlay_fraction: None,
            overlay_at: None,
            overlay_tables: None,
            counter_view,
            perspective,
            country_filter: String::new(),
            // capture
            capture,
            capture_note: None,
            failure,
            captures: 0,
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
        // The dashboard passes on the host's choice, not the standalone one.
        viewer.diagnostics = false;
        viewer.dropper = Some(Arc::clone(&resources.dropper));
        viewer.shared_resources = Some(resources);
        viewer.settings = false;
        viewer
    }

    pub(crate) fn install_map(&mut self, document: Document, scene: Scene, revision: String) {
        self.counter_view =
            map::counters::CounterView::new(self.options.counter_settings(document.game_counters));
        self.document = Some(document);
        self.scene = Some(scene);
        self.revision = Some(revision);
        self.error = None;
        self.map_error = None;
        self.rendered = None;
        self.uploaded = Uploaded::default();
    }

    pub(crate) fn has_map(&self) -> bool {
        self.document.is_some() && self.scene.is_some()
    }

    pub(crate) fn retire_map<T: Send + 'static>(&mut self, frame: &eframe::Frame, permit: T) {
        if let Some(texture) = self.texture.take()
            && let Some(state) = frame.wgpu_render_state()
        {
            state.renderer.write().free_texture(&texture);
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

    fn unload_recording(&mut self, path: &std::path::Path) -> bool {
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
        self.raid_tooltip_view = Default::default();
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
        self.perspective.country = target.options.country.clone().map_or(
            map::perspective::Country::Recorded,
            map::perspective::Country::Tag,
        );
        self.perspective.fog = target.options.fog;
        let branch = target
            .branch_id
            .parse::<u64>()
            .expect("validated map command");
        let head = Playhead {
            branch,
            hour: target.hour,
            fraction: target.fraction,
        };
        let animation_target = map::presentation::Target {
            branch,
            hour: target.hour,
            seek_sequence: target.seek_sequence,
        };
        self.animation_seek |= self
            .map_animation_target
            .is_some_and(|old| animation_target.seeks_from(old));
        self.map_animation_target = Some(animation_target);
        self.playing = target.playing;
        self.playhead = Some(head);
        let valid = self
            .document
            .as_ref()
            .and_then(|document| document.playback.range(branch));
        self.error = match valid {
            Some((first, last)) if (first..=last).contains(&target.hour) => None,
            Some((first, last)) => Some(format!(
                "Requested hour is outside this branch ({first}..{last})"
            )),
            None => Some("The requested branch is not present in this recording".into()),
        };
        if let Some(error) = &self.error {
            self.rendered = None;
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
            return;
        }
        if let Some(mut document) = self.document.take() {
            if let Err(error) = self.map(ui, frame, &mut document) {
                self.error = Some(error);
                self.rendered = None;
            }
            self.document = Some(document);
        }
        if self.presentation_ready(head, target.playing)
            && self.error.is_none()
            && self.map_error.is_none()
        {
            self.rendered_sequence = target.sequence;
        }
    }

    /// Whether the frame drawn answers `target` with a built hour of this
    /// epoch: exactly, or, playing, with the newest hour built before it.
    fn presentation_ready(&self, target: Playhead, playing: bool) -> bool {
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
        let error = self.error.clone().or_else(|| {
            self.shown
                .as_ref()
                .filter(|shown| {
                    shown.branch.to_string() == target.branch_id && shown.hour == target.hour
                })
                .and(self.map_error.clone())
        });
        let ready = target.branch_id.parse().ok().is_some_and(|branch| {
            self.presentation_ready(
                Playhead {
                    branch,
                    hour: target.hour,
                    fraction: target.fraction,
                },
                target.playing,
            )
        }) && self.rendered_sequence == target.sequence;
        dashboard::protocol::DashboardMapState {
            id: target.id.clone(),
            source_id: Some(target.source_id.clone()),
            revision: Some(target.revision.clone()),
            branch_id: self.rendered.map(|shown| shown.branch.to_string()),
            hour: self.rendered.map(|shown| shown.hour),
            fraction: self.rendered.map(|shown| shown.fraction),
            sequence: self.rendered_sequence,
            requested_sequence: target.sequence,
            loading: error.is_none() && !ready,
            error,
        }
    }

    /// Everything the hour on screen could not draw exactly. The hour's own
    /// gaps lead, because they are the part that changes with the hour; the
    /// persistent content limitations and the political layer's follow, in
    /// the order `--dump` prints them, so a capture beside a dump lists the
    /// same lines in the same order. Names, labels, raid icons and counters
    /// are drawn later in the frame, so theirs (with their refused tokens)
    /// come from the last drawn frame, which is this hour's after the first
    /// repaint. Every per-hour line is replaced with its hour, so a refusal
    /// the next hour recovers from goes. The Match game UI scale's note follows when
    /// that scale is in use. The open battle window's gaps are not included:
    /// the frame adds them after it draws the window, and
    /// `report_map_details` adds them itself.
    fn collect_warnings(&self, document: &Document) -> Vec<String> {
        let Some(shown) = &self.shown else {
            return Vec::new();
        };
        let mut warnings = shown.limitations.lines();
        // The viewer's device, when it lacks a feature some content needs to draw exactly.
        if let Some((device, _)) = &self.gpu {
            warnings.extend(render::device_requirements(device.features()));
        }
        warnings.extend(document.warnings.iter().cloned());
        warnings.extend(shown.political_warnings.iter().cloned());
        warnings.extend(shown.plan_warnings.iter().cloned());
        warnings.extend(shown.building_warnings.iter().cloned());
        warnings.extend(shown.model_warnings.iter().cloned());
        if let Some(placements) = &self.placements {
            warnings.extend(placements.warnings.iter().cloned());
        }
        warnings.extend(self.label_report.warnings());
        // Combats whose icon is undecided (for example while the engaged
        // index has not yet covered a session-start branch's hour).
        warnings.extend(self.counter_report.combat_gap_lines());
        // The overlay's refused tokens: the raid icons' as laid out for this
        // camera, the counters' (with the naval mission icons' gaps) as last drawn.
        if let Some(overlay) = &self.prepared_overlay {
            warnings.extend(overlay.raid_report().warnings());
        }
        warnings.extend(self.counter_report.warnings());
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
            warnings.extend(index.byte_check().lines());
        }
        // A capture renders at the dump's scale, not Match game's.
        let pinned = self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.pinned.is_some());
        if self.ui_scale == map::ui_scale::Setting::MatchGame
            && !pinned
            && let Some((_, (_, Some(note)))) = &self.match_game
        {
            warnings.push(note.clone());
        }
        warnings
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
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    for warning in &self.map_warnings {
                        ui.colored_label(egui::Color32::YELLOW, warning);
                    }
                });
        }
    }

    /// Flips diagnostics mode from the viewer's own control or shortcut. The
    /// standalone viewer saves the choice in `viewer.json`; an embedded viewer
    /// asks the desktop host, which owns the choice and sends it back.
    fn toggle_diagnostics(&mut self) {
        if let Some(embedded) = &self.embedded {
            embedded.toggle_diagnostics();
            return;
        }
        self.diagnostics = !self.diagnostics;
        self.save_diagnostics();
    }

    fn save_diagnostics(&mut self) {
        if let Err(error) = Config::save_diagnostics(self.diagnostics) {
            self.error = Some(format!("Could not save the diagnostics setting: {error}"));
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
                self.settings = true;
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
                let mut document = match &source {
                    Some(source) => match content.iter().find(|(key, content)| {
                        key == source.key()
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
                        ),
                    },
                    None => {
                        Document::load(&path, &resolve, options.allow_unverified, &initial, None, 0)
                    }
                }
                .map_err(|error| error.to_string())?;
                let decoder = resources
                    .as_ref()
                    .map(|resources| {
                        let replay = document.playback.replay_handle();
                        let decoder = resources
                            .decoder(Arc::clone(&replay))
                            .map_err(|error| error.to_string())?;
                        let mut presented = crate::playback::Playback::unloaded(replay)
                            .map_err(|error| error.to_string())?;
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
                let scene = load_scene(&device, &queue, &document, preferences.graphics)
                    .map_err(|error| error.to_string())?;
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
        self.raid_tooltip_view = Default::default();
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
        self.shown = None;
        self.last_view = None;
        // close zoom
        self.unit_models = None;
        self.unit_field_uploaded = false;
        self.profile_frame = 0;
        self.generation = 0;
        if self.embedded.is_none() {
            self.error = None;
        }
        self.map_error = None;
        self.playing = false;
        self.camera = self.options.camera;
        self.camera_manual = self.options.camera.is_some();
        // names
        self.placements = None;
        self.placements_at = None;
        self.fade = None;
        // counters
        self.overlay = None;
        self.prepared_overlay = None;
        self.counter_report = CounterReport::default();
        self.overlay_fraction = None;
        self.overlay_at = None;
        self.overlay_tables = None;
        self.raid_lane = Default::default();
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
        if let Some(capture) = self.capture.take_if(|capture| capture.exit) {
            let message = format!(
                "Capture {} failed: replay could not be loaded: {error}",
                capture.path.display()
            );
            *self
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(message.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.capture_note = Some(message);
        }
        self.error = Some(error);
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
                    match selector.resolve(&document) {
                        Ok(selection) => self.combat.open(selection),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
                self.playhead = Some(Playhead {
                    branch: document.playback.branch(),
                    hour: document.playback.hour(),
                    fraction: document.hour_fraction,
                });
                self.counter_view = map::counters::CounterView::new(
                    self.options.counter_settings(document.game_counters),
                );
                self.document = Some(document);
                // The loader built the whole hour-independent scene; the
                // frames only upload hour data into it from here.
                self.scene = Some(scene);
                self.uploaded = Uploaded::default();
                self.loading = None;
                if let Some(opened) = self.opened
                    && map::counter_move::profiling()
                {
                    map::counter_move::profile_print(format!(
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
            map::ui_scale::refresh_monitors(move || ctx.request_repaint());
        }
    }

    /// The Match game scale for `document` and its note, resolved again only
    /// when the monitors have been re-read or the content changed.
    fn match_game<'a>(
        match_game: &'a mut Option<MatchGame>,
        document: &Document,
    ) -> &'a (f32, Option<String>) {
        let key = (
            map::ui_scale::monitors_generation(),
            Arc::as_ptr(&document.content) as usize,
        );
        if match_game.as_ref().is_none_or(|(cached, _)| *cached != key) {
            let monitors = map::ui_scale::monitors();
            let resolved = map::ui_scale::Setting::MatchGame.resolve(
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

    /// The standalone window's "UI scale" setting, saved to `viewer.json`.
    fn ui_scale_control(&mut self, ui: &mut egui::Ui) {
        let matched = self
            .document
            .as_ref()
            .and(self.match_game.as_ref())
            .map(|(_, (scale, _))| *scale);
        let label = |setting: map::ui_scale::Setting| match setting {
            map::ui_scale::Setting::MatchGame => match matched {
                Some(scale) => format!("Match game ({:.0}%)", scale * 100.0),
                None => "Match game (default)".into(),
            },
            map::ui_scale::Setting::Manual(scale) => format!("{:.0}%", scale * 100.0),
        };
        let mut selected = self.ui_scale;
        ui.label("UI scale");
        egui::ComboBox::from_id_salt("ui_scale")
            .selected_text(label(selected))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut selected,
                    map::ui_scale::Setting::MatchGame,
                    label(map::ui_scale::Setting::MatchGame),
                );
                for percent in dashboard::protocol::MAP_UI_SCALE_CHOICES {
                    let setting = map::ui_scale::Setting::Manual(percent as f32 / 100.0);
                    ui.selectable_value(&mut selected, setting, label(setting));
                }
            })
            .response
            .on_hover_text(
                "Size of counters, map icons, labels, combat bubbles, the combat window and map \
                 tooltips. Match game draws them as large as the game shows them on this desktop.",
            );
        if selected != self.ui_scale {
            self.ui_scale = selected;
            match Config::save_ui_scale(selected.stored()) {
                Ok(()) => self.config.ui_scale = selected.stored(),
                Err(error) => self.error = Some(format!("viewer.json: {error}")),
            }
        }
    }

    /// The game's two counter options, as its settings menu and map-mode bar switch them:
    /// they start from `settings.txt` and take effect on the next frame.
    fn counter_controls(&mut self, ui: &mut egui::Ui) {
        use map::counters::CounterColorMode;
        let settings = &mut self.counter_view.settings;
        let label = |mode: CounterColorMode| match mode {
            CounterColorMode::Allegiance => "Allegiance",
            CounterColorMode::Country => "Country",
        };
        ui.label("Counter colours");
        egui::ComboBox::from_id_salt("counter_color_mode")
            .selected_text(label(settings.color_mode))
            .show_ui(ui, |ui| {
                for mode in [CounterColorMode::Allegiance, CounterColorMode::Country] {
                    ui.selectable_value(&mut settings.color_mode, mode, label(mode));
                }
            })
            .response
            .on_hover_text(
                "The game's counter colour mode (settings.txt counter_color_mode): plates by \
                 allegiance to the viewing country, or in each owner's own colour",
            );
        ui.checkbox(&mut settings.nato, "NATO counters")
            .on_hover_text(
                "The game's NATO counter option (settings.txt template_counter): unit icons that \
             have a NATO frame draw it",
            );
    }

    fn paths(&mut self, ctx: &egui::Context) {
        let mut show = self.settings;
        let mut save = false;
        egui::Window::new("Game content folders").open(&mut show).collapsible(false).resizable(false).default_width(600.0).show(ctx,|ui|{
            ui.label("Choose the installed game and the Paradox user folder. Replay content is verified before opening.");
            ui.add_space(10.0);
            ui.label("game folder");
            ui.horizontal(|ui|{
                ui.text_edit_singleline(&mut self.game_text);
                if ui.button("Browse...").clicked()&& let Some(path)=rfd::FileDialog::new().pick_folder(){self.game_text=path.to_string_lossy().into_owned();}
            });
            ui.label("Paradox user folder");
            ui.horizontal(|ui|{
                ui.text_edit_singleline(&mut self.user_text);
                if ui.button("Browse...").clicked()&& let Some(path)=rfd::FileDialog::new().pick_folder(){self.user_text=path.to_string_lossy().into_owned();}
            });
            ui.add_space(10.0);
            save=ui.button("Save folders").clicked();
        });
        self.settings = show;
        if save {
            let config = Config {
                game_dir: PathBuf::from(&self.game_text),
                user_dir: PathBuf::from(&self.user_text),
                ..self.config.clone()
            };
            match config.save() {
                Ok(()) => {
                    self.config = config;
                    self.configure_service();
                    self.settings = false;
                    self.error = None;
                    if let Some(path) = self.options.replay.clone() {
                        self.open(path, ctx, self.expected_revision.clone());
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    fn timeline(&mut self, ui: &mut egui::Ui, document: &mut Document) {
        let mut playhead = self.playhead.unwrap_or(Playhead {
            branch: document.playback.branch(),
            hour: document.playback.hour(),
            fraction: document.hour_fraction,
        });
        let playhead_session = document
            .playback
            .replay()
            .branches
            .iter()
            .find(|branch| branch.id == playhead.branch)
            .map_or(document.playback.session(), |branch| branch.session);
        let mut selected_session = playhead_session;
        let mut selected_branch = playhead.branch;
        ui.horizontal(|ui| {
            let session_number = document
                .playback
                .replay()
                .sessions
                .iter()
                .position(|session| session.id == selected_session)
                .unwrap_or(0)
                + 1;
            egui::ComboBox::from_id_salt("session")
                .selected_text(format!("Session {session_number}"))
                .show_ui(ui, |ui| {
                    for (index, session) in document.playback.replay().sessions.iter().enumerate() {
                        let label = if self.diagnostics {
                            format!(
                                "Session {} · process {} · {}",
                                index + 1,
                                session.process_id,
                                session.sample_boundary
                            )
                        } else {
                            format!("Session {}", index + 1)
                        };
                        ui.selectable_value(&mut selected_session, session.id, label);
                    }
                });
            if selected_session != playhead_session
                && let Some(branch) = document
                    .playback
                    .replay()
                    .branches
                    .iter()
                    .find(|branch| branch.session == selected_session)
            {
                selected_branch = branch.id;
            }
            egui::ComboBox::from_id_salt("branch")
                .selected_text(format!("Branch {selected_branch}"))
                .show_ui(ui, |ui| {
                    for branch in document
                        .playback
                        .replay()
                        .branches
                        .iter()
                        .filter(|branch| branch.session == selected_session)
                    {
                        // Why the recorder started the branch is diagnostics.
                        let label = if self.diagnostics {
                            format!("Branch {} · {:?}", branch.id, branch.reason)
                        } else {
                            format!("Branch {}", branch.id)
                        };
                        ui.selectable_value(&mut selected_branch, branch.id, label);
                    }
                });
            if selected_branch != playhead.branch {
                self.animation_seek = true;
                self.playing = false;
                match document.playback.range(selected_branch) {
                    Some((first, _)) => {
                        playhead = Playhead {
                            branch: selected_branch,
                            hour: first,
                            fraction: 0.0,
                        };
                    }
                    None => self.error = Some(format!("unknown branch {selected_branch}")),
                }
            }
            // Branch lineage evidence is diagnostics.
            if !self.diagnostics {
                return;
            }
            if let Some(branch) = document
                .playback
                .replay()
                .branches
                .iter()
                .find(|branch| branch.id == playhead.branch)
            {
                match branch.parent {
                    Parentage::Unknown => {
                        ui.weak("Parent unknown");
                    }
                    Parentage::Probable { branch, hour } => {
                        ui.weak(format!("Probable parent: branch {branch}, {}", date(hour)));
                    }
                }
            }
            match document.playback.parent_support(playhead.branch) {
                Ok(Some(true)) => {
                    ui.weak("Supporting hour present");
                }
                Ok(Some(false)) => {
                    ui.colored_label(egui::Color32::YELLOW, "Supporting hour unavailable");
                }
                Ok(None) => {}
                Err(error) => {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("Parent evidence unavailable: {error}"),
                    );
                }
            }
        });
        let branch = playhead.branch;
        let Some((first, last)) = document.playback.range(branch) else {
            self.playhead = Some(playhead);
            return;
        };
        let mut hour = playhead.hour;
        ui.horizontal(|ui| {
            if ui.button("|<").on_hover_text("First hour").clicked() {
                self.animation_seek = true;
                hour = first;
                self.playing = false;
            }
            if ui.button("<").on_hover_text("Previous hour").clicked() {
                self.animation_seek = true;
                hour = hour.saturating_sub(1).max(first);
                self.playing = false;
            }
            if ui
                .button(if self.playing { "Pause" } else { "Play" })
                .clicked()
            {
                self.playing = !self.playing;
                self.last_step = Instant::now();
            }
            if ui.button(">").on_hover_text("Next hour").clicked() {
                hour = hour.saturating_add(1).min(last);
                self.playing = false;
            }
            if ui.button(">|").on_hover_text("Last hour").clicked() {
                self.animation_seek = true;
                hour = last;
                self.playing = false;
            }
            egui::ComboBox::from_id_salt("speed")
                .selected_text(format!("{} h/s", self.speed))
                .show_ui(ui, |ui| {
                    for speed in [1, 6, 24, 60] {
                        ui.selectable_value(&mut self.speed, speed, format!("{speed} h/s"));
                    }
                });
        });
        let width = ui.available_width();
        let response = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = width;
                ui.add(egui::Slider::new(&mut hour, first..=last).show_value(false))
            })
            .inner;
        if response.changed() {
            self.animation_seek = true;
            self.playing = false;
            // A scrub lands on a recorded hour, not between two.
            playhead.fraction = 0.0;
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_gray(65));
        let span = f64::from(last) - f64::from(first) + 1.0;
        for gap in document
            .playback
            .replay()
            .gaps
            .iter()
            .filter(|gap| gap.branch == branch)
        {
            let left = rect.left()
                + ((f64::from(gap.first_hour) - f64::from(first)) / span) as f32 * rect.width();
            let right = rect.left()
                + ((f64::from(gap.last_hour) - f64::from(first) + 1.0) / span) as f32
                    * rect.width();
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(right.max(left + 1.0), rect.bottom()),
                ),
                0.0,
                egui::Color32::LIGHT_RED,
            );
        }
        if hour != playhead.hour {
            // Every button and the scrub land on a recorded hour.
            // A forward step continues animation; the seek controls reset it above.
            playhead.hour = hour;
            playhead.fraction = 0.0;
        }
        // Playback is continuous, not one recorded hour at a time: the speed
        // control is recorded hours a second and the wall clock between two
        // repaints is a fraction of an hour, which `Document::hour_fraction`
        // carries into the division positions. A pause leaves the fraction
        // where it is, so the frame does not jump when it stops.
        // A profiled capture's sweep moves the playhead itself, at its paced rate.
        if self.playing && !(map::counter_move::profiling() && self.capture.is_some()) {
            let elapsed = self.last_step.elapsed().as_secs_f64();
            self.last_step = Instant::now();
            let fraction = f64::from(playhead.fraction) + elapsed * f64::from(self.speed);
            if !advance_clock(&mut playhead, fraction, last) {
                self.playing = false;
            }
            ui.ctx().request_repaint();
        }
        self.playhead = Some(playhead);
        ui.horizontal(|ui| {
            // Playing, the date is the hour on screen, which trails the clock
            // when the builders cannot keep up; it is not labelled as lagging.
            let displayed = if self.playing {
                document.playback.hour()
            } else {
                playhead.hour
            };
            let fraction = if displayed == playhead.hour {
                playhead.fraction
            } else {
                0.0
            };
            ui.strong(date(displayed));
            if self.diagnostics {
                if fraction > 0.0 {
                    ui.weak(format!("Hour {displayed} + {fraction:.2} h"));
                } else {
                    ui.weak(format!("Hour {displayed}"));
                }
            }
            // colour
            ui.checkbox(&mut self.postfx, "Colour grading")
                .on_hover_text("The engine's post chain: exposure, tone map, colour LUT");
        });
    }

    // hours
    /// Whether the map can show `hour` of `branch` on the next frame: it is on
    /// screen or built and waiting.
    fn hour_ready(&self, branch: replay::BranchId, hour: i32) -> bool {
        self.ready.contains_key(&(branch, hour))
            || self.shown.as_ref().is_some_and(|shown| {
                shown.epoch == self.epoch && (shown.branch, shown.hour) == (branch, hour)
            })
    }

    /// Keeps the hour builders told where the playhead is and uploads the
    /// newest hour they finished that the window should show.
    ///
    /// Nothing here builds an hour: the political fill, railways, buildings,
    /// trains, aircraft, unit models, battle plan, weather effects, snow and
    /// mud, fog, limitations, names and icons all arrive built
    /// (`crate::hours`), and the frame that shows an hour only uploads it, all
    /// of its layers at once so no frame mixes two hours. Until the playhead's
    /// hour is built the scene keeps the hour it has; during playback it takes
    /// the newest built hour on the way. A capture waits for the hour it names.
    fn refresh_hour(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        document: &mut Document,
        laps: &mut Laps,
    ) -> Result<(), std::io::Error> {
        laps.lap("before_hour");
        if self.scene.is_none() {
            return Err(std::io::Error::other("the map scene was not created"));
        }
        // Static geometry arrives with the scene from the loading thread
        // (`load_scene`); only hour data uploads per frame from here.
        laps.lap("static_geometry");
        let replay = document.playback.replay_handle();
        if self
            .hours
            .as_ref()
            .is_none_or(|hours| !hours.serves(&document.content, &replay))
        {
            self.hours = Some(match &self.shared_resources {
                Some(resources) => {
                    hours::Pipeline::shared(Arc::clone(&document.content), replay, resources)?
                }
                None => hours::Pipeline::new(Arc::clone(&document.content), replay)?,
            });
            if let (Some(hours), Some(index)) = (&self.hours, self.engaged_carry.take()) {
                hours.adopt_engaged(index);
            }
            self.epoch = 0;
            self.ready.clear();
            self.shown = None;
            self.raid_lane = Default::default();
        }
        let playhead = self.playhead.unwrap_or(Playhead {
            branch: document.playback.branch(),
            hour: document.playback.hour(),
            fraction: document.hour_fraction,
        });
        let view = self.last_view.unwrap_or_else(|| {
            let map_size = MapSize::of(&document.map);
            let defines = document.map_defines;
            let (viewport, camera) = match self.capture.as_ref().and_then(|capture| capture.pinned)
            {
                Some((size, camera)) => {
                    let camera = camera.map_or_else(
                        || Camera::fit(map_size, defines, size),
                        |placement| placement.on(map_size, defines),
                    );
                    (size, camera)
                }
                None => {
                    let size = [1280, 720];
                    (
                        size,
                        self.camera.map_or_else(
                            || Camera::fit(map_size, defines, size),
                            |placement| placement.on(map_size, defines),
                        ),
                    )
                }
            };
            hours::View {
                camera,
                viewport,
                // The first frame lays the icons out again at its own scale.
                ui_scale: 1.0,
            }
        });
        // Playing, the builders reach far enough ahead to stay in front of the
        // clock; paused, a step either way is already built.
        let (ahead, behind) = if self.dashboard_map {
            (0, 0)
        } else if self.playing {
            ((self.speed as i32 * 3 / 4 + 2).min(96), 0)
        } else {
            (2, 2)
        };
        let want = hours::Want {
            branch: playhead.branch,
            hour: playhead.hour,
            ahead,
            behind,
            playing: self.playing,
            perspective: self.perspective.clone(),
            counters: self.counter_view.clone(),
            view,
            // Not the hour on screen after a seek, which may be to that hour.
            have: self
                .ready
                .keys()
                .chain(
                    self.shown
                        .as_ref()
                        .filter(|shown| shown.epoch == self.epoch && !self.animation_seek)
                        .map(|shown| (shown.branch, shown.hour))
                        .iter(),
                )
                .filter(|(branch, _)| *branch == playhead.branch)
                .map(|(_, hour)| *hour)
                .collect(),
            received: 0,
        };
        let pipeline = self.hours.as_mut().expect("started above");
        let epoch = pipeline.want(want)?;
        if epoch != self.epoch {
            self.epoch = epoch;
            self.ready.clear();
        }
        if self.dropper.is_none() {
            self.dropper = Some(Arc::new(hours::Dropper::new()?));
        }
        // Everything this frame lets go of, freed off the frame path at its end.
        let mut retired = hours::Retired::default();
        for built in pipeline.take()? {
            feed_raids(&self.raid_lane, &built);
            if built.epoch == epoch {
                if let Some(old) = self.ready.insert((built.branch, built.hour), built) {
                    retired.push(old);
                }
            } else {
                retired.push(built);
            }
        }
        let fallback = self
            .playing
            .then(|| {
                latest_built_key(&self.ready, (playhead.branch, playhead.hour), |built| {
                    built.layers.is_ok()
                })
            })
            .flatten();
        let (keep, gone): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(&mut self.ready)
            .into_iter()
            .partition(|((branch, hour), _)| {
                *branch == playhead.branch
                    && ((playhead.hour - behind..=playhead.hour + ahead).contains(hour)
                        || fallback == Some((*branch, *hour)))
            });
        self.ready = keep;
        let mut passed = gone;
        laps.lap("hours_poll");
        let target = (playhead.branch, playhead.hour);
        let shown_at = self
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == epoch)
            .map(|shown| (shown.branch, shown.hour));
        let mut next = None;
        // A seek to the hour on screen takes it again, rebuilt (see `have`),
        // to start a fresh animation history. The fallback below still moves
        // only forward from the hour on screen.
        if self.animation_seek || shown_at != Some(target) {
            next = self.ready.remove(&target);
            // A capture names its hour, and waits for it.
            let wait = self
                .capture
                .as_ref()
                .is_some_and(|capture| !map::counter_move::profiling() || capture.remaining == 0);
            if next.is_none() && wait {
                let pipeline = self.hours.as_ref().expect("started above");
                next = Some(wait_for_hour(
                    &self.raid_lane,
                    (target, epoch),
                    || pipeline.wait(),
                    &mut self.ready,
                    |built| retired.push(built),
                )?);
            }
            if next.is_none() && self.playing {
                // The newest hour on the way from the one on screen.
                let on_the_way = fallback
                    .filter(|key| shown_at.is_none_or(|shown| shown.0 != key.0 || shown.1 < key.1));
                if let Some(key) = on_the_way {
                    next = self.ready.remove(&key);
                    let later = self.ready.split_off(&key);
                    passed.extend(std::mem::replace(&mut self.ready, later));
                }
            }
        }
        let applied = match next {
            Some(built) => {
                // The hour on screen of any epoch: a perspective change starts
                // a new build epoch, but the scene's building history does not
                // depend on the viewing country, so it continues across it.
                let on_screen = self.shown.as_ref().map(|shown| (shown.branch, shown.hour));
                // Hours the builders passed over to keep up with the clock
                // continue the history, also on the catch-up after a pause; a
                // seek or a gap in the recording does not.
                let passes_unbuilt = !self.animation_seek
                    && on_screen.is_some_and(|(_, from)| {
                        !map::presentation::crosses_gap(
                            &document.playback.replay().gaps,
                            built.branch,
                            from,
                            built.hour,
                        )
                    });
                let skipped = skipped_hours(
                    on_screen,
                    (built.branch, built.hour),
                    passes_unbuilt,
                    |hour| {
                        let key = (built.branch, hour);
                        let passed_hour = self.ready.get(&key).or_else(|| passed.get(&key))?;
                        let layers = passed_hour.layers.as_ref().ok();
                        Some(layers.map(|layers| Arc::clone(&layers.buildings)))
                    },
                );
                self.apply_hour(state, document, built, skipped, &mut retired, laps)
            }
            None => Ok(()),
        };
        if !passed.is_empty() {
            retired.push(passed);
        }
        if let Some(dropper) = &self.dropper
            && !retired.is_empty()
        {
            dropper.drop_later(retired);
        }
        applied?;
        // The frame sits past the playhead's hour only when it is the hour on
        // screen.
        document.hour_fraction = if (document.playback.branch(), document.playback.hour()) == target
        {
            playhead.fraction
        } else {
            0.0
        };
        if let Some(error) = self
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == epoch)
            .and_then(|shown| shown.error.clone())
        {
            return Err(std::io::Error::other(error));
        }
        Ok(())
    }

    /// Uploads one built hour and makes it the hour on screen.
    fn apply_hour(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        document: &mut Document,
        built: hours::Built,
        skipped: Option<Vec<Arc<hours::Shared<map::map_buildings::PreparedBuildings>>>>,
        retired: &mut hours::Retired,
        laps: &mut Laps,
    ) -> Result<(), std::io::Error> {
        let hours::Built {
            epoch,
            branch,
            hour,
            view,
            current,
            layers,
            laps: built_laps,
            wall,
        } = built;
        if let Some(opened) = self.opened.take()
            && map::counter_move::profiling()
        {
            map::counter_move::profile_print(format!(
                "PROFILE open first_hour {:.1} ms",
                opened.elapsed().as_secs_f64() * 1000.0
            ));
        }
        // The raid lane gets the hour's own frame with the hour, cache hits
        // too: it may have pruned the frame since the builders delivered it.
        let raid_frame = current.clone();
        retired.push(document.playback.present(branch, hour, current));
        laps.lap("present");
        if map::counter_move::profiling() {
            map::counter_move::profile_print(format!(
                "PROFILE built frame {} hour {hour} wall {wall:.3} parts {}",
                self.profile_frame,
                built_laps
                    .iter()
                    .map(|(name, ms)| format!("{name}={ms:.2}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        laps.lap("built_print");
        let layers = match layers {
            Ok(layers) => layers,
            Err(error) => {
                self.animation_seek = false;
                present_raids(
                    &self.raid_lane,
                    map::presentation::Continuity::Continue,
                    crate::raid_lane::Presented {
                        content: Some(&document.content),
                        perspective: &self.perspective,
                        at: (branch, hour),
                        playing: self.playing,
                        current: &raid_frame,
                    },
                );
                self.shown = Some(ShownHour {
                    epoch,
                    branch,
                    hour,
                    error: Some(error),
                    political_warnings: Vec::new(),
                    plan_warnings: Vec::new(),
                    building_warnings: Vec::new(),
                    model_warnings: Vec::new(),
                    political_summary: (0, 0),
                    political_build: Duration::ZERO,
                    limitations: Default::default(),
                    snow: Default::default(),
                    header: Default::default(),
                });
                return Ok(());
            }
        };
        let previous = self
            .shown
            .as_ref()
            .filter(|shown| shown.error.is_none())
            .map(|shown| (shown.branch, shown.hour, shown.header.viewing.as_deref()));
        let next = (branch, hour, layers.header.viewing.as_deref());
        let continuity = map::presentation::continuity(
            previous,
            next,
            self.animation_seek,
            &document.playback.replay().gaps,
        );
        let unit_restart = map::presentation::unit_restart(previous, next, self.animation_seek);
        let hours::Layers {
            political,
            borders,
            rails,
            buildings,
            building_warnings,
            trains,
            aircraft,
            units,
            orders,
            effects,
            intel,
            snow_mud,
            limitations,
            header,
            names,
            overlay,
            overlay_tables,
        } = layers;
        let scene = self
            .scene
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the map scene was not created"))?;
        laps.lap("apply_start");
        let uploaded = &mut self.uploaded;
        if uploaded.rails != Some(rails.id) {
            let (mesh, bridges) = &rails.value;
            scene.set_railways(&state.device, &state.queue, &document.rail_assets, mesh)?;
            scene.set_bridges(
                &state.device,
                &state.queue,
                &document.bridges,
                bridges.clone(),
            )?;
            uploaded.rails = Some(rails.id);
        }
        laps.lap("rails_upload");
        if uploaded.buildings != Some(buildings.id) {
            let skipped = skipped
                .as_ref()
                .map(|hours| hours.iter().map(|hour| &hour.value).collect::<Vec<_>>());
            scene.set_map_buildings(
                &state.device,
                &state.queue,
                &document.map_buildings,
                buildings.value.clone(),
                skipped.as_deref(),
            )?;
            uploaded.buildings = Some(buildings.id);
        }
        laps.lap("buildings_upload");
        // The raid offer cycle is presentation history too: it starts again
        // exactly when the trains and the weather do.
        present_raids(
            &self.raid_lane,
            continuity,
            crate::raid_lane::Presented {
                content: Some(&document.content),
                perspective: &self.perspective,
                at: (branch, hour),
                playing: self.playing,
                current: &raid_frame,
            },
        );
        let replaced = scene.set_trains(document.trains.clone(), trains, continuity)?;
        retired.push(replaced);
        laps.lap("trains_upload");
        let replaced = scene.set_aircraft(document.aircraft.clone(), aircraft)?;
        retired.push(replaced);
        laps.lap("aircraft_upload");
        match unit_restart {
            map::presentation::UnitRestart::Keep => {}
            map::presentation::UnitRestart::ViewingCountry => {
                scene.restart_unit_models(Some(units.owned()))
            }
            map::presentation::UnitRestart::All => scene.restart_unit_models(None),
        }
        let model_warnings = units.report.warnings();
        retired.push(self.unit_models.replace(units));
        self.unit_field_uploaded = false;
        if uploaded
            .orders
            .as_ref()
            .is_none_or(|had| !Arc::ptr_eq(had, &orders))
        {
            scene.set_orders(&state.device, &state.queue, &document.orders, &orders)?;
            retired.push(uploaded.orders.replace(Arc::clone(&orders)));
        }
        let plan_warnings = orders.report.warnings();
        retired.push(orders);
        laps.lap("orders_upload");
        let replaced = scene.set_prepared_weather_effects(
            &state.device,
            &state.queue,
            document.weather_effects.clone(),
            effects,
            continuity,
        )?;
        retired.push(replaced);
        laps.lap("effects_upload");
        if uploaded.political != Some(political.id) {
            scene.set_political(&state.device, &state.queue, &political.layers.regions)?;
            uploaded.political = Some(political.id);
        }
        // borders: the window has no demilitarised-zone override, so the zone is the
        // recorded one.
        if uploaded.borders != Some(borders.id) {
            scene.set_border_plans(&state.device, &borders.classes, &borders.highlighted)?;
            uploaded.borders = Some(borders.id);
        }
        laps.lap("border_classes");
        // weather: an hour whose grid equals the one on the GPU writes nothing.
        let same_snow = uploaded.snow_mud.as_ref().is_some_and(|had| {
            Arc::ptr_eq(had, &snow_mud)
                || had.width == snow_mud.width
                    && had.height == snow_mud.height
                    && had.texels == snow_mud.texels
                    && had.province_snow == snow_mud.province_snow
        });
        if !same_snow {
            scene.set_snow_mud(
                &state.device,
                &state.queue,
                &snow_mud.texels,
                snow_mud.width,
                snow_mud.height,
                &snow_mud.province_snow,
            )?;
            uploaded.snow_mud = Some(Arc::clone(&snow_mud));
        }
        laps.lap("snow_upload");
        // fog of war: hours whose fog did not change share the grid on the GPU.
        if uploaded
            .intel
            .as_ref()
            .is_none_or(|had| !Arc::ptr_eq(had, &intel))
        {
            scene.set_fog_of_war(&state.queue, &intel.texels, intel.fog)?;
            uploaded.intel = Some(Arc::clone(&intel));
        }
        laps.lap("fog_upload");
        retired.push(self.placements.replace(names));
        self.placements_at = Some((branch, hour));
        retired.push(self.prepared_overlay.replace(overlay));
        retired.push(self.overlay_tables.replace((
            branch,
            hour,
            self.perspective.clone(),
            overlay_tables,
        )));
        self.overlay_at = Some(OverlayKey {
            branch,
            hour,
            camera: view.camera,
            pixels: view.viewport,
            ui_scale: view.ui_scale,
            perspective: self.perspective.clone(),
            counter_settings: self.counter_view.settings,
        });
        self.overlay_fraction = None;
        let shown = ShownHour {
            epoch,
            branch,
            hour,
            error: None,
            political_warnings: political.layers.warnings().to_vec(),
            plan_warnings,
            building_warnings,
            model_warnings,
            political_summary: political.layers.summary(),
            political_build: political.layers.build,
            limitations,
            header,
            snow: (
                snow_mud.summary,
                [snow_mud.width, snow_mud.height],
                snow_mud.build,
            ),
        };
        self.animation_seek = false;
        retired.push(self.shown.replace(shown));
        retired.push((political, borders, rails, buildings, snow_mud, intel));
        laps.lap("apply_end");
        Ok(())
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
                if let Some(time) = capture.time {
                    println!("Animation time: {time:.6} s");
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

    fn map(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        document: &mut Document,
    ) -> Result<(), String> {
        let map_timer = Instant::now();
        let mut laps = self
            .laps
            .take()
            .unwrap_or_else(|| Laps::new(Duration::ZERO));
        laps.lap("before_map");
        map::counter_move::take_profile();
        let state = frame
            .wgpu_render_state()
            .ok_or("WGPU renderer is unavailable")?;
        if map::counter_move::profiling() {
            static ADAPTER_ONCE: std::sync::Once = std::sync::Once::new();
            let info = state.adapter.get_info();
            ADAPTER_ONCE.call_once(|| {
                map::counter_move::profile_print(format!(
                    "PROFILE adapter {} {:?}",
                    info.name, info.backend
                ));
            });
        }
        // The scene arrives with the document from the loading thread
        // (`load_scene`); only hour data uploads per frame from here.
        self.map_error = self
            .refresh_hour(state, document, &mut laps)
            .err()
            .map(|error| error.to_string());
        // An error view otherwise stops repainting; a paused seek away from it
        // still needs to poll the asynchronous builders until its hour arrives.
        if self
            .playhead
            .is_some_and(|head| !self.hour_ready(head.branch, head.hour))
        {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        if let Some(error) = &self.map_error {
            self.rendered = None;
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
            return Ok(());
        }
        let Some(shown) = self
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == self.epoch)
        else {
            // The first hour is still being built; keep polling for it.
            self.rendered = None;
            ui.ctx().request_repaint();
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return Ok(());
        };
        laps.lap("hour_misc");
        // The screen-space interface's scale. A capture keeps the dump's, 1.0
        // unless `--ui-scale` names one, so a `--window-dump` and a `--dump` of
        // the same state stay the same frame.
        let pinned = self.capture.as_ref().and_then(|capture| capture.pinned);
        let ui_scale = match pinned {
            Some(_) => self.options.ui_scale.unwrap_or(1.0),
            None => {
                Self::watch_monitors(&mut self.monitor_seen, ui.ctx());
                let setting = self.ui_scale;
                let (matched, _) = Self::match_game(&mut self.match_game, document);
                match setting {
                    map::ui_scale::Setting::Manual(scale) => scale,
                    map::ui_scale::Setting::MatchGame => *matched,
                }
            }
        };
        // Ordinary viewing builds none of this text: only diagnostics mode
        // shows it and only a capture prints it (the desktop's on-demand
        // report rebuilds it in `report_map_details`).
        let build_warnings = self.diagnostics || self.capture.is_some();
        let mut warnings = if build_warnings {
            self.collect_warnings(document)
        } else {
            Vec::new()
        };
        laps.lap("limitations");
        self.interface_points = map::ui_scale::points(1.0, ui_scale, ui.ctx().pixels_per_point());
        if self.diagnostics && !self.dashboard_map && self.embedded.is_none() {
            egui::ScrollArea::vertical()
                .id_salt("warnings")
                .max_height(ui.available_height() * WARNING_SHARE)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for warning in &warnings {
                        ui.colored_label(egui::Color32::YELLOW, warning);
                    }
                });
            for line in shown.stats() {
                ui.weak(line);
            }
        }
        let size = ui.available_size().max(egui::vec2(1.0, 1.0));
        // capture: `--window-dump` renders at the size it was asked for rather
        // than at the size the panels left over, so the frame it writes does not
        // depend on this display's scale factor or on where the warning area
        // wrapped. Every other frame renders the map area at device pixels,
        // which is the space the projection and the overlay batch already use.
        let pixels = match pinned {
            Some((size, _)) => size,
            None => [
                (size.x * ui.ctx().pixels_per_point()).round() as u32,
                (size.y * ui.ctx().pixels_per_point()).round() as u32,
            ],
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
        // Screen positions reach the camera in physical pixels, the same space
        // the render target and the projection use.
        let scale = ui.ctx().pixels_per_point();
        let physical = |position: egui::Pos2| {
            [
                (position.x - rect.left()) * scale,
                (position.y - rect.top()) * scale,
            ]
        };
        // The interface is laid out in interface pixels, so its hit tests are
        // too: counters, combat bubbles, the combat window and its lists.
        let extent = map::ui_scale::extent(pixels, ui_scale);
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            // Escape closes the tactics popup before the battle window.
            if self.combat.tactics_open {
                self.combat.tactics_open = false;
            } else {
                self.combat.close();
            }
        }
        let engaged = self
            .hours
            .as_ref()
            .map(|hours| hours.engaged(&self.perspective.country))
            .transpose()
            .map_err(|error| error.to_string())?;
        // Hit tests only: the frame's second layout below collects the hovers.
        let combat_geometry = self
            .combat
            .draw(
                document,
                extent,
                &self.perspective,
                engaged.as_deref(),
                false,
            )
            .map_err(|error| error.to_string())?;
        let point = response
            .hover_pos()
            .or_else(|| response.interact_pointer_pos())
            .map(|position| map::ui_scale::to_interface(physical(position), ui_scale));
        if response.drag_started() {
            self.combat.start_drag(combat_geometry.as_ref(), point);
        }
        if response.drag_stopped() {
            self.combat.dragging = false;
        }
        let over_combat = self.combat.dragging
            || point.is_some_and(|point| {
                combat_geometry
                    .as_ref()
                    .is_some_and(|window| window.contains(point))
            });
        let popup_click = response.clicked()
            && point
                .zip(combat_geometry.as_ref())
                .is_some_and(|(point, drawn)| self.combat.handle_click(drawn, point));
        if response.clicked()
            && !popup_click
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
                self.combat.scroll[side] =
                    self.combat.scroll[side].saturating_add_signed(direction as isize);
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
                    self.combat.scroll[side] = self.combat.scroll[side].saturating_add(1);
                }
                if scroll > 0.0 {
                    self.combat.scroll[side] = self.combat.scroll[side].saturating_sub(1);
                }
            }
        }
        if response.dragged()
            && !over_combat
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.camera_manual = true;
            let delta = response.drag_delta();
            let to = physical(pointer);
            camera.pan(
                [to[0] - delta.x * scale, to[1] - delta.y * scale],
                to,
                pixels,
            );
        }
        if response.hovered() && !over_combat {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera_manual = true;
                let pointer = response.hover_pos().unwrap_or(rect.center());
                camera.zoom((scroll * 0.002).exp(), physical(pointer), pixels);
            }
        }
        self.camera = Some(camera.placement());
        let camera = match pinned {
            Some((size, camera)) => camera.map_or_else(
                || Camera::fit(map_size, defines, size),
                |placement| placement.on(map_size, defines),
            ),
            None => *camera,
        };
        let scene = self.scene.as_mut().expect("created above");
        // A `--window-dump` settles unit models for its camera, as `--dump`
        // does; an F12 or Export capture keeps the live history.
        if !self.unit_field_uploaded {
            let timer = Instant::now();
            let units = self
                .unit_models
                .as_ref()
                .expect("prepared above")
                .field(&document.unit_models);
            scene
                .set_unit_models(
                    &state.device,
                    &state.queue,
                    &document.unit_models,
                    units,
                    self.options.window_dump,
                )
                .map_err(|error| error.to_string())?;
            laps.lap("units_upload");
            self.unit_field_uploaded = true;
            // The field placed the raid units at the hour's start.
            self.raid_tick = Some(0.0);
            if map::counter_move::profiling() {
                map::counter_move::profile_print(format!(
                    "PROFILE models {:.6}",
                    timer.elapsed().as_secs_f64() * 1000.0
                ));
            }
        }
        // Playing, the raid units move every rendered frame by the playback
        // fraction, the tick's progress `UpdateUnitMovement@141CD7750` reads
        // each frame; paused, they stand where the hour's start put them.
        // Only they move within the hour: the figures and ships stay uploaded.
        let tick = if self.playing {
            document.hour_fraction
        } else {
            0.0
        };
        if self.raid_tick != Some(tick)
            && let Some(prepared) = self.unit_models.as_ref()
        {
            let hour = document.hour().map_err(|error| error.to_string())?;
            let heights = hour.heights();
            scene
                .set_raid_units(prepared.raid_units(&document.unit_models, Some((&heights, tick))));
            self.raid_tick = Some(tick);
        }
        // names: the engine integrates the layer cross-fade by the frame delta
        // and clamps to [0, 1], so it takes one second whatever the zoom speed.
        // It has to start from a settled value, and only this frame knows which:
        // a window given no `--camera` fits the map to a viewport that does not
        // exist until there is one, and a replay opened after a zoom starts at a
        // camera the last one left behind.
        let delta = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        let fade = self
            .fade
            .get_or_insert_with(|| LayerFade::instant(camera))
            .step(camera, delta);
        self.fade = Some(fade);
        laps.lap("panel_camera_units");
        let now = document.hour().map_err(|error| error.to_string())?;
        self.last_view = Some(hours::View {
            camera,
            viewport: pixels,
            ui_scale,
        });
        let names_key = (document.playback.branch(), document.playback.hour());
        if self.placements_at != Some(names_key) {
            match now.place_names() {
                Ok(placements) => {
                    self.placements = Some(Arc::new(placements));
                    self.placements_at = Some(names_key);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        laps.lap("names");
        // counters: one batch carries the bases, the labels, the counters and
        // the combat icons in `DEFAULT_PRIORITY_*` order.
        let overlay_key = OverlayKey {
            branch: document.playback.branch(),
            hour: document.playback.hour(),
            camera,
            pixels,
            ui_scale,
            perspective: self.perspective.clone(),
            counter_settings: self.counter_view.settings,
        };
        let overlay_timer = Instant::now();
        // capture: the animation clock is the one input a live window cannot
        // share with a headless frame, so a capture frame renders at 0, the
        // still frame `--dump` renders, or at its explicit sequence clock. The
        // window runs one presentation clock: the map scene and the map-icon
        // windows' GUI clock (`Time`, `CGraphics+0x32590`) share it.
        let animation_time = self
            .capture
            .as_ref()
            .and_then(|c| c.time)
            .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
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
            .map(|hours| hours.engaged(&self.perspective.country))
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
                    .is_some_and(|index| index.covers(branch, hour))
            },
        );
        // The raid lane's icons for the hour on screen replace the hour's
        // when they are newer; a paused frame asks the lane for one batch.
        let ctx = ui.ctx().clone();
        // Its batches borrow the hour builders' threads.
        if let Some(hours) = &self.hours {
            self.raid_lane.use_workers(hours.workers());
        }
        self.raid_lane.show(
            &document.content,
            &self.perspective,
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
            prepared.replace_raids(&now, camera, pixels, ui_scale, &tables.raids(), drawn);
        }
        if layout_changed || (raids_only && self.prepared_overlay.is_none()) {
            let tables_key = (
                document.playback.branch(),
                document.playback.hour(),
                self.perspective.clone(),
            );
            if engaged_arrived
                || self
                    .overlay_tables
                    .as_ref()
                    .is_none_or(|(branch, hour, perspective, _)| {
                        (*branch, *hour, perspective) != (tables_key.0, tables_key.1, &tables_key.2)
                    })
            {
                // Raid offers are only evaluated on the raid lane: until it
                // delivers this hour's for this view, no raid icon draws
                // (`RaidIcons::carried`).
                let ours = now
                    .vision(&self.perspective)
                    .map_err(|error| error.to_string())?
                    .ours();
                let raids = self
                    .raid_lane
                    .icons(tables_key.0, tables_key.1, ours)
                    .unwrap_or_default();
                let tables = now
                    .hour_tables_with_raids(&self.perspective, raids, engaged.as_deref())
                    .map_err(|error| error.to_string())?;
                self.overlay_tables = Some((tables_key.0, tables_key.1, tables_key.2, tables));
            }
            let tables = &self.overlay_tables.as_ref().expect("parsed above").3;
            self.prepared_overlay = Some(
                now.prepare_overlay_cached(camera, pixels, ui_scale, tables, &self.counter_view)
                    .map_err(|error| error.to_string())?,
            );
            self.overlay_at = Some(overlay_key);
            self.overlay_fraction = None;
        }
        laps.lap("overlay_prepare");
        // The frame's province schedule, advanced before its map icons read the dirty bits
        // (`CGraphicalMap::update` runs before `CMapIconManagerImpl::update`); the scene's
        // render takes it.
        if let Some(scene) = self.scene.as_mut() {
            scene.share_province_schedule(self.counter_view.presentation.schedule());
            scene.advance_provinces(camera, pixels);
        }
        if self.overlay_fraction != Some(document.hour_fraction) {
            // Redrawn into the last fraction's batch and report, so playback allocates nothing.
            let prepared = self.prepared_overlay.as_mut().expect("prepared above");
            prepared
                .draw_into(
                    &now,
                    camera,
                    pixels,
                    ui_scale,
                    Default::default(),
                    animation_time,
                    self.overlay.get_or_insert_with(Batch::default),
                    &mut self.counter_report,
                )
                .map_err(|error| error.to_string())?;
            if self.label_report != *prepared.label_report() {
                self.label_report.clone_from(prepared.label_report());
            }
            self.overlay_fraction = Some(document.hour_fraction);
        }
        // A map-icon window still fading in is laid out again on the next frame, as the game
        // ticks every window each frame (`UpdateTransitions@1422AF9F0`).
        if self.counter_view.presentation.take_running() {
            self.overlay_at = None;
            ui.ctx().request_repaint();
        }
        if raids_only && check_overlay() {
            self.check_overlay(&now, camera, pixels, ui_scale, animation_time)?;
        }
        // Resolve clicks against this hour and camera's newly rebuilt hit regions.
        if response.clicked()
            && !over_combat
            && let Some(point) = point
            && let Some(hit) = self
                .counter_report
                .combat_hits
                .iter()
                .rev()
                .find(|hit| hit.contains(point))
        {
            self.combat.open(hit.selection);
        }
        let overlay_ms = overlay_timer.elapsed().as_secs_f64() * 1000.0;
        laps.lap("overlay_draw");
        let mut combat_batch;
        let engaged = self
            .hours
            .as_ref()
            .map(|hours| hours.engaged(&self.perspective.country))
            .transpose()
            .map_err(|error| error.to_string())?;
        let label_batch = if let Some(mut drawn) = self
            .combat
            .draw(
                document,
                extent,
                &self.perspective,
                engaged.as_deref(),
                true,
            )
            .map_err(|error| error.to_string())?
        {
            self.combat
                .draw_tooltip(&mut drawn, &document.icons.combat, point, extent)
                .map_err(|error| error.to_string())?;
            if let Some(delay) = self.combat.tooltip_repaint_after(&document.icons.combat) {
                ui.ctx().request_repaint_after(delay);
            }
            if build_warnings {
                warnings.extend(self.combat.warnings(document));
            }
            combat_batch = self.overlay.as_ref().expect("built above").clone();
            combat_batch.quads.extend(drawn.batch.quads);
            &combat_batch
        } else {
            self.overlay.as_ref().expect("built above")
        };
        // capture: at a fixed clock `Scene::render` takes the luminance
        // adaptation instantaneously, because the clock did not advance, which
        // is what a dump's own pair of frames does.
        let scene = self.scene.as_mut().unwrap();
        scene.set_graphics(self.graphics);
        scene.label_frame(u64::from(self.profile_frame));
        scene
            .render(
                &state.device,
                &state.queue,
                Request {
                    size: pixels,
                    camera,
                    day_night: DayNight::at_hour(document.playback.hour()),
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
                    overlay: Overlay {
                        names: self.placements.as_deref(),
                        layer_alpha: fade.0,
                        sprites: Some(label_batch),
                        ui_scale,
                    },
                    // trees
                    no_trees: false,
                },
            )
            .map_err(|e| e.to_string())?;
        let [animate, overlay_upload, encode, submit] = scene.render_laps();
        if self.generation != scene.generation() {
            let view = scene.view().ok_or("Map target was not created")?;
            let mut egui_renderer = state.renderer.write();
            if let Some(texture) = self.texture {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    &state.device,
                    view,
                    wgpu::FilterMode::Nearest,
                    texture,
                );
            } else {
                self.texture = Some(egui_renderer.register_native_texture(
                    &state.device,
                    view,
                    wgpu::FilterMode::Nearest,
                ));
            }
            self.generation = scene.generation();
        }
        // capture: the frame is drawn and the scene still holds it, so the
        // readback is the dump's own.
        laps.lap("render");
        if map::counter_move::profiling() {
            map::counter_move::profile_print(format!(
                "PROFILE frame {} fraction {} height {} total {:.6} overlay {:.6} spline {:.6} hour {} frame_total {:.3} outside {:.3} scene {:.3}/{:.3}/{:.3}/{:.3} gpu {:.3} laps {} detail {}",
                self.profile_frame,
                document.hour_fraction,
                camera.height,
                map_timer.elapsed().as_secs_f64() * 1000.0,
                overlay_ms,
                map::counter_move::take_profile(),
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
                    .map_or_else(String::new, |scene| scene.detail_laps())
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
        self.map_warnings = warnings;
        self.profile_frame += 1;
        ui.painter().image(
            self.texture.unwrap(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // The desktop's campaign map has no panels, so its diagnostics sit
        // over the map; dashboard maps draw theirs beside their loading status.
        // Every map on the software adapter, windowed or on a dashboard, says so.
        let software = render::is_software(&state.adapter);
        let diagnostics = self.diagnostics && self.embedded.is_some() && !self.dashboard_map;
        if software || diagnostics {
            map_overlay(
                ui,
                rect.intersect(ui.clip_rect()),
                "diagnostics",
                self.interface_points,
                |ui, height| {
                    if software {
                        ui.colored_label(egui::Color32::YELLOW, render::SOFTWARE_RENDERER_NOTICE);
                    }
                    if diagnostics {
                        self.diagnostics_ui(ui, height);
                    }
                },
            );
        }
        // The water animates, so the window keeps drawing.
        ui.ctx().request_repaint();
        // The map's tooltip is part of its interface, so it is sized like the
        // sprites: egui's default text size, padding, gap and wrap width taken
        // as interface pixels, times the UI scale, in device pixels. egui lays
        // it out at that size, so its text is rasterised there, not stretched.
        let points = |interface: f32| map::ui_scale::points(interface, ui_scale, scale);
        let style = ui.style().clone();
        let mut tooltip = egui::Tooltip::for_enabled(&response);
        tooltip.popup = tooltip
            .popup
            .frame(egui::Frame::popup(&style).inner_margin(style.spacing.menu_margin * points(1.0)))
            .gap(points(4.0))
            .width(points(style.spacing.tooltip_width));
        tooltip.show(|ui| {
            let raid = point
                .filter(|_| !over_combat)
                .and_then(|point| self.prepared_overlay.as_ref()?.raid_at(point).cloned());
            if let Some(key) = raid {
                if let crate::playback::Current::Frame(frame) = document.playback.current()
                    && let Some(ours) = self.overlay_tables.as_ref().map(|t| t.3.ours())
                    && let Some(prepared) = self.raid_tooltips.request(
                        Arc::clone(&document.content),
                        Arc::clone(frame),
                        ours,
                        key,
                        ui.ctx(),
                    )
                {
                    self.raid_tooltip_view
                        .show(ui, &prepared, points(1.0), self.diagnostics);
                }
                return;
            }
            // The game shows a moving division's arrival in its counter's
            // tooltip, so the hovered counter's line replaces the map's hint.
            let arrival = point.filter(|_| !over_combat).and_then(|point| {
                let hit = self
                    .counter_report
                    .arrivals
                    .iter()
                    .rev()
                    .find(|hit| hit.contains(point))?;
                let arrival = map::counter_eta::arrival(document.playback.hour(), hit.remaining)?;
                Some(
                    map::counter_eta::sentence(
                        &document.names.localisation,
                        &arrival,
                        hit.ours,
                        &date(arrival.hour),
                    )
                    .unwrap_or_else(|reason| format!("Arrival line refused: {reason}.")),
                )
            });
            let text = arrival.unwrap_or_else(|| "Drag to pan · Scroll to zoom".into());
            ui.set_max_width(points(style.spacing.tooltip_width));
            let size = points(egui::TextStyle::Body.resolve(&style).size);
            ui.label(egui::RichText::new(text).size(size));
        });
        self.rendered = Some(Playhead {
            branch: document.playback.branch(),
            hour: document.playback.hour(),
            fraction: document.hour_fraction,
        });
        Ok(())
    }
}

impl eframe::App for Viewer {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.embedded_logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // The hide command may arrive in logic after eframe chose to draw
        // this pass. Skip it as well as all later hidden passes.
        if self
            .embedded
            .as_ref()
            .is_some_and(|embedded| !embedded.visible())
        {
            return;
        }
        if ui.input(|input| input.events.iter().any(is_diagnostics_shortcut)) {
            self.toggle_diagnostics();
        }
        if self.dashboard_active
            && let Some(dashboard) = &mut self.dashboard
        {
            dashboard.ui_scale = self.ui_scale;
            dashboard.ui(ui, frame);
            if let Some(embedded) = &mut self.embedded {
                embedded.report_maps(dashboard.states());
            }
            return;
        }
        let waited = if map::counter_move::profiling() && self.capture.is_some() {
            pace_profiled_frame()
        } else {
            Duration::ZERO
        };
        self.laps = Some(Laps::new(waited));
        self.poll(ui.ctx());
        self.lap("poll");
        // capture: F12 writes the map target as it stands, at its own pixel
        // size, through the readback `--dump` uses. It is the only way to score
        // a window frame against a dump without a screenshot tool, which would
        // measure the swap chain and the desktop compositor instead of the map.
        if self.capture.is_none() && ui.input(|input| input.key_pressed(egui::Key::F12)) {
            self.captures += 1;
            let path =
                self.options.out.clone().unwrap_or_else(|| {
                    PathBuf::from(format!("window-capture-{}.png", self.captures))
                });
            self.capture = Some(Capture {
                path,
                remaining: 0,
                pinned: None,
                exit: false,
                time: None,
                step: 0.0,
                frames: 1,
                index: 0,
            });
        }
        // counters: what the header's viewing-country chooser offers and shows,
        // read from the hour on screen before the header borrows `self`.
        let header = self
            .shown
            .as_ref()
            .filter(|_| self.document.is_some())
            .map(|shown| Arc::clone(&shown.header))
            .unwrap_or_default();
        let (country_tags, viewing) = (&header.countries, &header.viewing);
        self.lap("header_tags");
        if self.embedded.is_none() {
            egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.heading("Battle Plan");
                ui.separator();
                if ui.button("Open replay...").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Battle Plan", &["battleplan", "hoi4replay"])
                        .pick_file()
                {
                    self.options.hour = None;
                    self.options.branch = None;
                    self.open(path, ui.ctx(), None);
                }
                if ui.button("Folders").clicked() {
                    self.settings = true;
                }
                self.ui_scale_control(ui);
                self.counter_controls(ui);
                let mut diagnostics = self.diagnostics;
                if ui
                    .checkbox(&mut diagnostics, "Diagnostics")
                    .on_hover_text(
                        "Show technical stats and data limitations with the map (F3). \
                         Captures and dumps report them either way",
                    )
                    .changed()
                {
                    self.toggle_diagnostics();
                }
                ui.separator();
                // counters: the game's two map controls, a viewing country and
                // the fog-of-war switch. The chooser lists every country the
                // hour on screen holds; the recorded player is the default.
                ui.separator();
                let selected = match &self.perspective.country {
                    map::perspective::Country::Recorded => format!(
                        "Recorded player{}",
                        viewing
                            .as_deref()
                            .map(|tag| format!(" ({tag})"))
                            .unwrap_or_default()
                    ),
                    map::perspective::Country::Tag(tag) => tag.clone(),
                };
                egui::ComboBox::from_id_salt("viewing_country")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.country_filter)
                                .hint_text("Filter tags"),
                        );
                        ui.selectable_value(
                            &mut self.perspective.country,
                            map::perspective::Country::Recorded,
                            "Recorded player",
                        );
                        // Only the rows in view are laid out, and a tag is
                        // cloned only when it is picked, so the list costs the
                        // same at thousands of countries as at a few.
                        let shown = filter_tags(country_tags, &self.country_filter);
                        let row = ui.spacing().interact_size.y;
                        egui::ScrollArea::vertical()
                            .max_height(row * 16.0)
                            .show_rows(ui, row, shown.len(), |ui, range| {
                                for tag in &shown[range] {
                                    let picked = matches!(
                                        &self.perspective.country,
                                        map::perspective::Country::Tag(current) if current == tag
                                    );
                                    if ui.selectable_label(picked, *tag).clicked() && !picked {
                                        self.perspective.country =
                                            map::perspective::Country::Tag((*tag).to_owned());
                                    }
                                }
                            });
                    })
                    .response
                    .on_hover_text(
                        "View the map as this country, as the game's console tag does: allegiance \
                         colours, the battle plan, aircraft and what its intelligence shows are \
                         that country's, whoever controls it",
                    );
                ui.checkbox(&mut self.perspective.fog, "Fog of war")
                    .on_hover_text(
                        "The game's fow console switch. On, a stack the viewing country has no \
                         intelligence on carries the question mark and uncovered land is fogged; \
                         off, every stack is drawn in full and no fog is drawn, still as the \
                         viewing country",
                    );
                // names: the header names every layer that reaches a pixel.
                if self.diagnostics {
                    ui.weak(format!(
                        "Viewed as {}, fog of war {} · Terrain, water, political fill, borders, names and labels",
                        viewing.as_deref().unwrap_or("no country"),
                        if self.perspective.fog { "on" } else { "off" },
                    ));
                }
            });
            ui.add_space(5.0);
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            // capture
            if let Some(note) = &self.capture_note {
                ui.weak(note);
            }
        });
        }
        self.lap("header_panel");
        if self.embedded.is_none() {
            self.paths(ui.ctx());
        }
        let central_panel = if self.embedded.is_some() {
            egui::CentralPanel::default().frame(egui::Frame::NONE)
        } else {
            egui::CentralPanel::default()
        };
        if let Some(mut document) = self.document.take() {
            if self.embedded.is_none() {
                egui::Panel::bottom("timeline").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.weak(
                            document
                                .source_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                        );
                        if document.playback.frame().is_none() {
                            ui.colored_label(egui::Color32::LIGHT_RED, "No recorded frame");
                        }
                    });
                    self.timeline(ui, &mut document);
                    ui.add_space(4.0);
                    match document.playback.current() {
                        crate::playback::Current::Gap { reason, .. } => {
                            ui.colored_label(egui::Color32::LIGHT_RED, format!("Gap: {reason}"));
                        }
                        crate::playback::Current::Frame(_) if !self.diagnostics => {}
                        crate::playback::Current::Frame(frame) => {
                            egui::ScrollArea::vertical()
                                .id_salt("tables")
                                .max_height(TABLE_LIST_HEIGHT)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        for table in TableId::ALL {
                                            match &*frame.tables[table.index()] {
                                                TableView::Complete(rows) => {
                                                    ui.weak(format!("{table:?}: {}", rows.len()));
                                                }
                                                TableView::Truncated { missing, .. } => {
                                                    ui.colored_label(
                                                    egui::Color32::YELLOW,
                                                    format!(
                                                        "{table:?}: truncated, {missing} missing"
                                                    ),
                                                );
                                                }
                                                TableView::Unavailable { reason } => {
                                                    ui.colored_label(
                                                        egui::Color32::LIGHT_RED,
                                                        format!("{table:?}: {reason}"),
                                                    );
                                                }
                                            }
                                        }
                                        ui.weak(format!(
                                            "Capture {:.3} ms",
                                            frame.metrics.total_ns as f64 / 1_000_000.0
                                        ));
                                    });
                                });
                        }
                    }
                });
            }
            if map::counter_move::profiling() && self.capture.is_some() {
                // Deterministic diagnostic sweep, including at a replay's final
                // hour. Ordinary playback continues to use the timeline clock.
                let mode = std::env::var("BATTLE_PLAN_VIEWER_PROFILE").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE")).unwrap_or_default();
                let sweep_frames = crate::profiling::sweep();
                // `hours`: one hour forward at the sweep's start, back at its middle.
                let middle = sweep_frames.start + sweep_frames.len() as u32 / 2;
                if let Some(playhead) = &mut self.playhead {
                    if mode == "hours"
                        && (self.profile_frame == sweep_frames.start
                            || self.profile_frame == middle)
                    {
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
                        self.animation_seek = true;
                        playhead.hour = last;
                        playhead.fraction = 0.0;
                    }
                }
                // Steady playback at `play:<hours per second>`, sampled as a
                // clock at the profiled pace over the sweep, as the timeline would run.
                let playing = std::env::var("BATTLE_PLAN_VIEWER_PROFILE").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE"))
                    .ok()
                    .and_then(|mode| mode.strip_prefix("play:")?.parse::<f32>().ok());
                self.playing =
                    playing.is_some() && crate::profiling::sweep().contains(&self.profile_frame);
                if let Some(speed) = playing {
                    // The builders reach as far ahead as the timeline at this speed.
                    self.speed = speed as u32;
                }
                if let Some(speed) = playing
                    && crate::profiling::sweep().contains(&self.profile_frame)
                    && let Some(mut moved) = self.playhead
                {
                    let last = document
                        .playback
                        .range(moved.branch)
                        .map_or(moved.hour, |(_, last)| last);
                    let fraction =
                        f64::from(moved.fraction) + f64::from(speed) / crate::profiling::hz();
                    advance_clock(&mut moved, fraction, last);
                    self.playhead = Some(moved);
                }
                if playing.is_none()
                    && !mode.starts_with("pan:")
                    && !mode.starts_with("zoom:")
                    && let Some(playhead) = &mut self.playhead
                {
                    playhead.fraction = if !crate::profiling::sweep().contains(&self.profile_frame)
                    {
                        self.options.hour_fraction
                    } else {
                        ((self.profile_frame - sweep_frames.start) % 120) as f32 / 120.0
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
                            .map(|rate| (rate / crate::profiling::hz() as f32, 1.0))
                    })
                    .or_else(|| {
                        mode.strip_prefix("zoom:").and_then(|factor| {
                            factor
                                .parse::<f32>()
                                .ok()
                                .filter(|factor| *factor > 0.0)
                                .map(|factor| {
                                    (0.0, factor.powf(1.0 / crate::profiling::hz() as f32))
                                })
                        })
                    });
                if let Some((dx, zoom)) = sweep
                    && crate::profiling::sweep().contains(&self.profile_frame)
                    && let Some(capture) = self.capture.as_mut()
                    && let Some((_, camera)) = capture.pinned.as_mut()
                    && let Some(camera) = camera.as_mut()
                {
                    camera.center[0] += dx;
                    camera.height = (camera.height * zoom).clamp(40.0, 900.0);
                }
            }
            self.lap("timeline_panel");
            central_panel.show(ui, |ui| {
                if let Err(error) = self.map(ui, frame, &mut document) {
                    self.error = Some(error);
                }
            });
            self.document = Some(document);
        } else {
            central_panel.show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    if self.loading.is_some() {
                        ui.spinner();
                        ui.heading("Opening replay");
                        ui.label("Verifying content and loading the map...");
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    } else {
                        ui.heading("Replay a campaign");
                        ui.label("Open a .battleplan file to explore its recorded hours.");
                        ui.add_space(10.0);
                        ui.label("Game content is loaded from your installed files.");
                    }
                });
            });
        }
        self.report_embedded(ui.ctx());
        mark_frame_end();
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

/// The chooser's tags that contain `filter`, ignoring ASCII case, in order.
fn filter_tags<'a>(tags: &'a [String], filter: &str) -> Vec<&'a str> {
    let filter = filter.trim().to_ascii_uppercase();
    tags.iter()
        .map(String::as_str)
        .filter(|tag| filter.is_empty() || tag.to_ascii_uppercase().contains(&filter))
        .collect()
}

/// Hands the raid lane the hour put on screen, with its frame, and starts
/// its offer cycle again there when the presentation history resets:
/// exactly when the trains and the weather do (a seek, same-hour ones too, a
/// step back, a branch change, a new recording, a recorded gap or a new
/// resolved viewing country); playback, one hour forward and a playback skip
/// carry it ([`map::presentation::continuity`]). The view and the reset are
/// installed together, before the lane wakes.
fn present_raids(
    lane: &crate::raid_lane::RaidLane,
    continuity: map::presentation::Continuity,
    presented: crate::raid_lane::Presented<'_>,
) {
    lane.present(
        presented,
        continuity == map::presentation::Continuity::Reset,
    );
}

/// A built hour as the builders deliver it to the window.
trait Delivered {
    /// The [`hours::Want`] generation it was built for.
    fn epoch(&self) -> u64;
    fn at(&self) -> (replay::BranchId, i32);
    fn current(&self) -> &crate::playback::Current;
}

impl Delivered for hours::Built {
    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn at(&self) -> (replay::BranchId, i32) {
        (self.branch, self.hour)
    }

    fn current(&self) -> &crate::playback::Current {
        &self.current
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
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_OVERLAY").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_OVERLAY")).is_some())
}

fn feed_raids(lane: &crate::raid_lane::RaidLane, built: &impl Delivered) {
    let (branch, hour) = built.at();
    lane.feed(branch, hour, built.current());
}

/// Blocks on `wait` until the hour `target` of `epoch` arrives, feeding
/// every hour received to the raid lane on the way ([`feed_raids`]). Other
/// hours of `epoch` are kept in `ready`, other epochs' go to `stale`.
fn wait_for_hour<B: Delivered>(
    lane: &crate::raid_lane::RaidLane,
    (target, epoch): ((replay::BranchId, i32), u64),
    mut wait: impl FnMut() -> std::io::Result<B>,
    ready: &mut BTreeMap<(replay::BranchId, i32), B>,
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
    shown: Option<(replay::BranchId, i32)>,
    (branch, next): (replay::BranchId, i32),
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

/// Moves a playing playhead `fraction` hours past its hour, whether or not the
/// builders have reached it: the map shows the newest built hour at or before
/// it ([`latest_built_key`]). Returns false at the branch's last hour.
fn advance_clock(playhead: &mut Playhead, fraction: f64, last: i32) -> bool {
    if fraction >= f64::from(last.saturating_sub(playhead.hour)) {
        playhead.hour = last;
        playhead.fraction = 0.0;
        return false;
    }
    playhead.hour += fraction.floor() as i32;
    playhead.fraction = fraction.fract() as f32;
    true
}

/// The newest `usable` hour of `target`'s branch at or before it.
fn latest_built_key<T>(
    ready: &BTreeMap<(replay::BranchId, i32), T>,
    target: (replay::BranchId, i32),
    usable: impl Fn(&T) -> bool,
) -> Option<(replay::BranchId, i32)> {
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
        || (playing && rendered.branch == target.branch && rendered.hour < target.hour)
}

/// Whether the shown overlay, `(engaged_covered, branch, hour)`, was laid out
/// before the engaged index covered its hour (by the window or by an hour
/// builder) and the index now does: it is laid out again once, so a paused
/// seek or a prebuilt hour settles on the exact flag, and an overlay laid out
/// with the index is never redone for it.
fn engaged_relayout(
    shown: Option<(bool, replay::BranchId, i32)>,
    covers: impl Fn(replay::BranchId, i32) -> bool,
) -> bool {
    shown.is_some_and(|(covered, branch, hour)| !covered && covers(branch, hour))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prebuilt or window overlay without the index is laid out again once
    /// the index covers its hour, while paused or while advancing through
    /// prebuilt hours; one laid out with it, or one the index does not yet
    /// cover, is kept, so unchanged frames do not rebuild.
    #[test]
    fn overlays_relayout_once_when_the_engaged_index_arrives() {
        let covered_to =
            |last: i32| move |branch: replay::BranchId, hour: i32| branch == 1 && hour <= last;
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

        fn at(&self) -> (replay::BranchId, i32) {
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
        use map::presentation::{Continuity, Target, continuity};
        let gap = replay::branches::Gap {
            branch: 1,
            first_hour: 45,
            last_hour: 49,
            reason: "missing".into(),
        };
        let gaps = std::slice::from_ref(&gap);
        let on_screen = Some((1, 20, Some("GER")));
        // The timeline's seek controls (|<, <, >|, the scrub, the branch
        // chooser) raise the seek flag; > and the playback clock do not.
        // Dashboard targets tell seeks by their clock's seek counter.
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
            let perspective = map::perspective::Choice::default();
            let shown = |at: (replay::BranchId, i32)| crate::playback::Current::Gap {
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
    fn the_chooser_filters_thousands_of_tags_without_cloning_them() {
        let tags: Vec<String> = (0..15_000).map(|index| format!("T{index:04}")).collect();
        assert_eq!(filter_tags(&tags, "").len(), 15_000);
        // Case and surrounding space are ignored; the order is the tag order.
        let found = filter_tags(&tags, " t1234 ");
        assert_eq!(found.first(), Some(&"T1234"));
        assert_eq!(found.len(), 1 + 10);
        let found = filter_tags(&tags, "999");
        assert!(found.iter().all(|tag| tag.contains("999")));
        assert!(std::ptr::eq(
            found[0],
            tags.iter()
                .find(|tag| tag.contains("999"))
                .unwrap()
                .as_str()
        ));
        assert!(filter_tags(&tags, "ZZZ").is_empty());
    }

    #[test]
    fn unload_releases_standalone_admission_and_discards_a_late_loader() {
        use replay::{
            container::Replay,
            identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
            spool::Record,
        };
        let path = PathBuf::from("memory.battleplan");
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
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
        assert!(!viewer.unload_recording(std::path::Path::new("another.battleplan")));
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
    fn failed_replay_load_ends_a_window_dump_and_keeps_an_interactive_capture() {
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
                        path: "window-capture-1.png".into(),
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
    fn seek_from_stale_host_state_reopens_its_explicit_snapshot() {
        let seek = crate::embedded::Seek {
            path: PathBuf::from("A.battleplan"),
            revision: "100:20:0".into(),
            branch: 1,
            hour: 12,
            playback: None,
        };
        let target = seek.open_request();
        let a = std::path::Path::new("A.battleplan");
        let b = std::path::Path::new("B.battleplan");

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
    fn the_diagnostics_flag_shows_them_for_a_run_over_the_saved_choice() {
        use super::initial_diagnostics;
        // Standalone: the flag shows them whatever is saved; otherwise the
        // saved choice decides.
        assert!(initial_diagnostics(true, false, false));
        assert!(initial_diagnostics(false, true, false));
        assert!(!initial_diagnostics(false, false, false));
        // Embedded: `viewer.json` is the standalone viewer's, so only the flag
        // counts until the host sends its choice.
        assert!(!initial_diagnostics(false, true, true));
        assert!(initial_diagnostics(true, false, true));
    }

    #[test]
    fn only_a_bare_first_f3_press_toggles_diagnostics() {
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
            egui::Modifiers::NONE
        )));
        assert!(!super::is_diagnostics_shortcut(&press(
            false,
            false,
            egui::Modifiers::NONE
        )));
        assert!(!super::is_diagnostics_shortcut(&press(
            true,
            true,
            egui::Modifiers::NONE
        )));
        assert!(!super::is_diagnostics_shortcut(&press(
            true,
            false,
            egui::Modifiers::CTRL
        )));
        assert!(!super::is_diagnostics_shortcut(&press(
            true,
            false,
            egui::Modifiers::SHIFT
        )));
    }

    #[test]
    fn display_uses_the_save_calendar_at_midnight() {
        assert_eq!(date(60_765_882), "1936-09-29  19:00");
        assert_eq!(date(60_765_887), "1936-09-29  24:00");
        assert_eq!(date(60_765_888), "1936-09-30  01:00");
    }
}
