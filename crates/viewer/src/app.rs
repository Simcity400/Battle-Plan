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
        self.laps
            .iter()
            .filter(|(_, ms)| *ms >= 0.05)
            .map(|(name, ms)| format!("{name}={ms:.2}"))
            .collect::<Vec<_>>()
            .join(" ")
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

/// Holds a profiled capture's frames to a 60 Hz display's pace.
///
/// A `--window-dump` frame does not wait for a display, so unpaced it runs at
/// hundreds of frames a second and a sweep that advances the playhead by
/// `speed / 60` hours a frame outruns the builders, which work in wall time.
/// The wait falls before the frame's laps start, so it is not a frame's cost.
fn pace_profiled_frame() -> Duration {
    const PERIOD: Duration = Duration::from_micros(16_667);
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let started = Instant::now();
    let Ok(mut last) = LAST.lock() else {
        return Duration::ZERO;
    };
    if let Some(previous) = *last {
        let due = previous + PERIOD;
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
    political_summary: (usize, usize),
    political_build: Duration,
    limitations: map::tables::Limitations,
    snow_line: String,
    /// The header's country list and viewing tag, built with the hour.
    header: Arc<hours::Header>,
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
    map_warnings: Vec<String>,
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
    map_preferences_seen: Option<(String, Option<Vec<String>>, u32, u8)>,
    // hours
    /// The timeline's position; the map follows it as hours are built.
    playhead: Option<Playhead>,
    /// The worker threads that build every hour's layers off the frame path.
    hours: Option<hours::Pipeline>,
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
    unit_fraction: Option<(f32, bool, bool)>,
    /// Instance-buffer bytes sized for every height band this hour (see the
    /// units upload), so zooming never reallocates mid-gesture.
    unit_capacity: Option<(replay::BranchId, i32, u64)>,
    // names
    placements: Option<Placements>,
    placements_at: Option<(replay::BranchId, i32, bool)>,
    /// A placement of the hour on screen for a changed state-layer flag,
    /// running off the UI thread while the current one keeps drawing.
    names_job: Option<NamesJob>,
    /// `None` until the first frame seeds it settled at that frame's own camera
    /// height; see [`Viewer::map`].
    fade: Option<LayerFade>,
    last_frame: Instant,
    label_report: LabelReport,
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
    overlay_at: Option<(
        replay::BranchId,
        i32,
        Camera,
        [u32; 2],
        map::perspective::Choice,
    )>,
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
    /// it and [`Document::perspective`] is kept in step with it.
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
    Ok(scene)
}

impl Viewer {
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
        if !viewer.settings
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
                    241
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
        let hidden_since = embedded.as_ref().map(|_| Instant::now());
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
            hours: None,
            epoch: 0,
            ready: BTreeMap::new(),
            shown: None,
            last_view: None,
            uploaded: Uploaded::default(),
            dropper: None,
            // close zoom
            unit_models: None,
            unit_fraction: None,
            unit_capacity: None,
            // names
            placements: None,
            placements_at: None,
            names_job: None,
            fade: None,
            last_frame: Instant::now(),
            label_report: LabelReport::default(),
            // counters
            counter_report: CounterReport::default(),
            overlay: None,
            prepared_overlay: None,
            overlay_fraction: None,
            overlay_at: None,
            overlay_tables: None,
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
        viewer.dropper = Some(Arc::clone(&resources.dropper));
        viewer.shared_resources = Some(resources);
        viewer.settings = false;
        viewer
    }

    pub(crate) fn install_map(&mut self, document: Document, scene: Scene, revision: String) {
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
        self.playing = false;
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
        if self.rendered == Some(head) && self.error.is_none() && self.map_error.is_none() {
            self.rendered_sequence = target.sequence;
        }
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
        let ready = self.rendered.is_some_and(|shown| {
            shown.branch.to_string() == target.branch_id
                && shown.hour == target.hour
                && shown.fraction == target.fraction
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

    pub(crate) fn map_warnings(&self) -> &[String] {
        &self.map_warnings
    }

    fn embedded_logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(embedded) = self.embedded.as_mut() else {
            return;
        };
        let pending = embedded.poll(ctx);
        if pending.close {
            return;
        }
        if pending.standalone {
            self.dashboard_active = false;
            self.last_step = Instant::now();
            self.last_frame = Instant::now();
        }
        if let Some(views) = pending.map_views {
            self.dashboard_active = true;
            self.playing = false;
            if self.dashboard.is_none() {
                let service = embedded.service();
                let created = self
                    .shared_resources
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| std::io::Error::other("Native map workers are unavailable"))
                    .and_then(|resources| {
                        crate::dashboard_maps::DashboardMaps::new(
                            self.config.clone(),
                            self.options.clone(),
                            service,
                            self.gpu.clone(),
                            resources,
                        )
                    });
                match created {
                    Ok(dashboard) => {
                        self.embedded_open = None;
                        self.embedded_seek = None;
                        self.dashboard = Some(Box::new(dashboard));
                    }
                    Err(error) => {
                        self.error = Some(error.to_string());
                        self.report_embedded(ctx);
                        return;
                    }
                }
            }
            if let Some(dashboard) = &mut self.dashboard {
                dashboard.update_config(&self.config, frame, ctx);
                dashboard.set_views(views, frame, ctx);
            }
        }
        if let Some(source) = self.source.clone() {
            let replay = source.replay();
            let preferences = self
                .embedded
                .as_ref()
                .expect("embedded controller")
                .service()
                .map_settings(&replay);
            self.graphics = preferences.graphics;
            let selection = (
                dashboard::map_settings::campaign_key(&replay),
                preferences.visual_mods.clone(),
                preferences.graphics.samples,
                preferences.graphics.texture_quality,
            );
            if self.loading.is_none() && self.map_preferences_seen.as_ref() != Some(&selection) {
                self.map_preferences_seen = Some(selection);
                if self.document.as_ref().is_none_or(|document| {
                    document.visual_mods != preferences.visual_mods
                        || document.texture_quality != preferences.graphics.texture_quality
                }) || self
                    .scene
                    .as_ref()
                    .is_some_and(|scene| scene.samples() != preferences.graphics.samples)
                {
                    if let Some(document) = &self.document {
                        self.options.branch = Some(document.playback.branch());
                        self.options.hour = Some(document.playback.hour());
                    }
                    self.options.camera = self.camera;
                    self.playing = false;
                    self.open(
                        source.path().to_path_buf(),
                        ctx,
                        Some(source.key().revision.clone()),
                    );
                    self.source = Some(source);
                }
            }
        }
        let actions = pending.actions;
        // The web shell owns these controls; they call the same viewer state
        // that the standalone egui controls use. Rendering is unchanged.
        for action in actions {
            self.host_map_action(action, ctx, frame);
        }
        if self.dashboard_active {
            // Full-map loading may have started before the tab changed. Finish
            // it once and lend its verified content to dashboard maps.
            self.poll();
        }
        if self.dashboard_active
            && let Some(dashboard) = &mut self.dashboard
        {
            dashboard.update_config(&self.config, frame, ctx);
            dashboard.wait_for_source(self.loading.as_ref().and(self.options.replay.clone()));
            if let (Some(source), Some(document), Some(config)) =
                (&self.source, &self.document, &self.loaded_config)
            {
                dashboard.seed_content(Arc::clone(source), Arc::clone(&document.content), config);
            }
            dashboard.poll(ctx);
            if let Some(embedded) = &mut self.embedded {
                embedded.report_maps(dashboard.states());
            }
            return;
        }
        if let Some(request) = pending.open {
            self.embedded_open = Some(request);
            self.embedded_seek = None;
        }
        if let Some(seek) = pending.seek {
            let accepted = self
                .document
                .as_ref()
                .map(|document| (document.source_path.as_path(), self.revision.as_deref()));
            let pending_load = if self.loading.is_some() || self.settings {
                self.options
                    .replay
                    .as_deref()
                    .map(|path| (path, self.expected_revision.as_deref()))
            } else {
                None
            };
            // The seek is newer than any queued open: the pipe drops earlier
            // seeks when an open arrives. Its explicit snapshot supersedes
            // that queue, even if a native picker changed the local selection.
            self.embedded_open = seek_open_target(&seek, accepted, pending_load);
            self.embedded_seek = Some(seek);
        }
        // A host can select files faster than their content can load. Retain
        // one pending path and finish the active loader before starting it.
        // Each pending seek is tied to its requested file, never the old map.
        self.poll();
        if self.loading.is_none()
            && let Some(request) = self.embedded_open.take()
        {
            self.options.branch = None;
            self.options.hour = None;
            self.options.hour_fraction = 0.0;
            self.open(request.path, ctx, request.revision);
            if self.loading.is_none() && !self.settings {
                self.embedded_seek = None;
            }
        }
        if self.loading.is_none()
            && !self.settings
            && let Some(seek) = self.embedded_seek.take()
        {
            let result = self
                .document
                .as_ref()
                .filter(|document| seek.matches(&document.source_path, self.revision.as_deref()))
                .ok_or_else(|| {
                    self.error.clone().unwrap_or_else(|| {
                        "The requested replay snapshot did not open; seek was not applied"
                            .to_owned()
                    })
                })
                .and_then(|document| match document.playback.range(seek.branch) {
                    Some((first, last)) if (first..=last).contains(&seek.hour) => Ok(()),
                    Some((first, last)) => Err(format!(
                        "Hour {} is outside branch {} ({first}..{last})",
                        seek.hour, seek.branch
                    )),
                    None => Err(format!("Unknown branch {}", seek.branch)),
                });
            match result {
                Ok(()) => {
                    self.error = None;
                    self.playing = false;
                    self.last_step = Instant::now();
                    self.playhead = Some(Playhead {
                        branch: seek.branch,
                        hour: seek.hour,
                        fraction: 0.0,
                    });
                }
                Err(error) => {
                    self.error = Some(error);
                }
            }
        }
        let visible = self
            .embedded
            .as_ref()
            .is_some_and(|embedded| embedded.visible());
        let now = Instant::now();
        if visible {
            if let Some(hidden) = self.hidden_since.take() {
                self.started += now.duration_since(hidden);
                self.last_step = now;
                self.last_frame = now;
                ctx.request_repaint();
            }
        } else if self.hidden_since.is_none() {
            self.hidden_since = Some(now);
        }
        // Invalid input arrives after any earlier coalesced work. Retain its
        // failure until a later successful open/seek or native user action.
        if let Some(error) = pending.error {
            self.error = Some(error);
        }
        self.report_embedded(ctx);
    }

    pub(crate) fn host_map_action(
        &mut self,
        action: dashboard::protocol::MapAction,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
    ) {
        use dashboard::protocol::MapAction;
        match action {
            MapAction::Unload { path } => {
                if self.unload_recording(std::path::Path::new(&path)) {
                    self.retire_map(frame, ());
                }
            }
            MapAction::Vsync { enabled } => {
                if let Some(mut config) = frame.wgpu_surface_config() {
                    config.present_mode = if enabled {
                        wgpu::PresentMode::AutoVsync
                    } else {
                        wgpu::PresentMode::AutoNoVsync
                    };
                    frame.set_wgpu_surface_config(config);
                }
            }
            MapAction::Configure { options } => {
                self.perspective.country = options.country.map_or(
                    map::perspective::Country::Recorded,
                    map::perspective::Country::Tag,
                );
                self.perspective.fog = options.fog;
            }
            MapAction::Export {
                path,
                view_id: Some(id),
            } => {
                self.error = match self.dashboard.as_mut() {
                    Some(dashboard) => dashboard.export_view(&id, path, ctx, frame).err(),
                    None => Some("The requested map widget is unavailable".into()),
                };
            }
            MapAction::Export {
                path,
                view_id: None,
            } => {
                if self.document.is_none() || self.capture.is_some() {
                    self.error = Some("Open a map and wait for any active export to finish".into());
                } else {
                    self.capture = Some(Capture {
                        path: PathBuf::from(path),
                        remaining: 0,
                        pinned: None,
                        exit: false,
                        time: None,
                        step: 0.0,
                        frames: 1,
                        index: 0,
                    });
                }
            }
            MapAction::Folders { game, user } => {
                let config = Config {
                    game_dir: PathBuf::from(game),
                    user_dir: PathBuf::from(user),
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
                self.report_map_details();
            }
            MapAction::Inspect => self.report_map_details(),
        }
        ctx.request_repaint();
    }

    fn report_map_details(&self) {
        let Some(embedded) = &self.embedded else {
            return;
        };
        let mut messages = self.map_warnings.clone();
        messages.extend(self.error.iter().cloned());
        messages.extend(self.map_error.iter().cloned());
        messages.extend(self.capture_note.iter().cloned());
        if let Some(document) = &self.document {
            messages.extend(document.warnings.iter().cloned());
            match document.playback.current() {
                crate::playback::Current::Gap { reason, .. } => {
                    messages.push(format!("Gap: {reason}"))
                }
                crate::playback::Current::Frame(frame) => {
                    for table in TableId::ALL {
                        messages.push(match &frame.tables[table.index()] {
                            TableView::Complete(rows) => format!("{table:?}: {} rows", rows.len()),
                            TableView::Truncated { missing, .. } => {
                                format!("{table:?}: truncated, {missing} missing")
                            }
                            TableView::Unavailable { reason } => {
                                format!("{table:?}: unavailable ({reason})")
                            }
                        });
                    }
                    messages.push(format!(
                        "Capture {:.3} ms",
                        frame.metrics.total_ns as f64 / 1_000_000.0
                    ));
                }
            }
        }
        if let Some(shown) = &self.shown {
            messages.push(format!(
                "Political layer build {:.1} ms",
                shown.political_build.as_secs_f64() * 1000.0
            ));
        }
        let mut seen = std::collections::HashSet::new();
        messages.retain(|message| seen.insert(message.clone()));
        embedded.report_details(dashboard::protocol::MapDetails {
            content_ready: self.config.resolve_options().is_ok(),
            error: self.error.clone(),
            game: self.config.game_dir.to_string_lossy().into_owned(),
            user: self.config.user_dir.to_string_lossy().into_owned(),
            path: self
                .document
                .as_ref()
                .map(|d| d.source_path.to_string_lossy().into_owned()),
            branch: self.rendered.map(|head| head.branch.to_string()),
            hour: self.rendered.map(|head| head.hour),
            messages,
        });
    }

    fn report_embedded(&mut self, ctx: &egui::Context) {
        let Some(embedded) = self.embedded.as_mut() else {
            return;
        };
        let shown = self
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == self.epoch);
        embedded.report(
            dashboard::protocol::ViewerState {
                path: self
                    .document
                    .as_ref()
                    .map(|document| document.source_path.to_string_lossy().into_owned()),
                branch: shown.map(|shown| shown.branch.to_string()),
                hour: shown.map(|shown| shown.hour),
                revision: self.document.as_ref().and(self.revision.clone()),
            },
            self.error.as_deref().or(self.map_error.as_deref()),
            ctx,
        );
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
                self.error = Some(error.to_string());
                self.settings = true;
                return;
            }
        };
        let Some((device, queue)) = self.gpu.clone() else {
            self.error = Some("WGPU renderer is unavailable".to_owned());
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
        self.source = None;
        self.source_decoder = None;
        self.loaded_config = None;
        self.rendered = None;
        self.revision = None;
        self.scene = None;
        // hours
        self.playhead = None;
        self.hours = None;
        self.ready.clear();
        self.shown = None;
        self.last_view = None;
        // close zoom
        self.unit_models = None;
        self.unit_fraction = None;
        self.unit_capacity = None;
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
        self.names_job = None;
        self.fade = None;
        // counters
        self.overlay = None;
        self.prepared_overlay = None;
        self.overlay_fraction = None;
        self.overlay_at = None;
        self.overlay_tables = None;
    }

    /// Records a lap of the frame that is being timed, if one is.
    fn lap(&mut self, name: &'static str) {
        if let Some(laps) = &mut self.laps {
            laps.lap(name);
        }
    }

    fn poll(&mut self) {
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
                self.error = Some(error);
                self.loading = None;
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.error = Some("Replay loader stopped before returning a result".into());
                self.loading = None;
            }
            _ => {}
        }
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
                        ui.selectable_value(
                            &mut selected_session,
                            session.id,
                            format!(
                                "Session {} · process {} · {}",
                                index + 1,
                                session.process_id,
                                session.sample_boundary
                            ),
                        );
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
                        ui.selectable_value(
                            &mut selected_branch,
                            branch.id,
                            format!("Branch {} · {:?}", branch.id, branch.reason),
                        );
                    }
                });
            if selected_branch != playhead.branch {
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
                hour = first;
                self.playing = false;
            }
            if ui.button("<").on_hover_text("Previous hour").clicked() {
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
            playhead.hour = hour;
            playhead.fraction = 0.0;
        }
        // Playback is continuous, not one recorded hour at a time: the speed
        // control is recorded hours a second and the wall clock between two
        // repaints is a fraction of an hour, which `Document::hour_fraction`
        // carries into the division positions. A pause leaves the fraction
        // where it is, so the frame does not jump when it stops.
        // A profiled capture's sweep moves the playhead itself, at its paced 60 Hz.
        if self.playing && !(map::counter_move::profiling() && self.capture.is_some()) {
            let elapsed = self.last_step.elapsed().as_secs_f64();
            self.last_step = Instant::now();
            let fraction = f64::from(playhead.fraction) + elapsed * f64::from(self.speed);
            if !self.advance_playhead(&mut playhead, fraction, last) {
                self.playing = false;
            }
            ui.ctx().request_repaint();
        }
        self.playhead = Some(playhead);
        ui.horizontal(|ui| {
            let displayed = playhead.hour;
            let fraction = playhead.fraction;
            ui.strong(date(displayed));
            if fraction > 0.0 {
                ui.weak(format!("Hour {displayed} + {fraction:.2} h"));
            } else {
                ui.weak(format!("Hour {displayed}"));
            }
            if document.playback.hour() != displayed || document.playback.branch() != branch {
                ui.weak(format!("drawing hour {}", document.playback.hour()));
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

    /// Moves a playing playhead to `fraction` hours past its hour.
    ///
    /// The clock enters an hour only once that hour is built, so playback never
    /// runs ahead of what the map shows: when the builders fall behind the
    /// speed, the clock waits at the end of the hour on screen and the replay
    /// plays at the builders' pace instead. When one slow frame owes more than
    /// one hour, the clock steps through every ready hour but the map draws
    /// only the latest, so a sped-up hour may never be drawn. Returns false at
    /// the branch's last hour.
    fn advance_playhead(&self, playhead: &mut Playhead, mut fraction: f64, last: i32) -> bool {
        while fraction >= 1.0 {
            if playhead.hour >= last {
                playhead.fraction = 0.0;
                return false;
            }
            if !self.hour_ready(playhead.branch, playhead.hour + 1) {
                // Just short of the boundary, where the divisions have arrived.
                fraction = f64::from(1.0_f32.next_down());
                break;
            }
            fraction -= 1.0;
            playhead.hour += 1;
        }
        playhead.fraction = fraction as f32;
        true
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
            self.epoch = 0;
            self.ready.clear();
            self.shown = None;
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
                states: LayerFade::wants_states(camera.height),
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
            perspective: self.perspective.clone(),
            view,
            have: self
                .ready
                .keys()
                .chain(
                    self.shown
                        .as_ref()
                        .filter(|shown| shown.epoch == self.epoch)
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
        let dropper = self.dropper.as_ref().expect("started above");
        for built in pipeline.take()? {
            if built.epoch == epoch {
                if let Some(old) = self.ready.insert((built.branch, built.hour), built) {
                    dropper.drop_later(old);
                }
            } else {
                dropper.drop_later(built);
            }
        }
        let (keep, gone): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(&mut self.ready)
            .into_iter()
            .partition(|((branch, hour), _)| {
                *branch == playhead.branch
                    && (playhead.hour - behind..=playhead.hour + ahead).contains(hour)
            });
        self.ready = keep;
        if !gone.is_empty() {
            dropper.drop_later(gone);
        }
        laps.lap("hours_poll");
        let target = (playhead.branch, playhead.hour);
        let shown_at = self
            .shown
            .as_ref()
            .filter(|shown| shown.epoch == epoch)
            .map(|shown| (shown.branch, shown.hour));
        let mut next = None;
        if shown_at != Some(target) {
            next = self.ready.remove(&target);
            // A capture names its hour, and waits for it.
            let wait = self
                .capture
                .as_ref()
                .is_some_and(|capture| !map::counter_move::profiling() || capture.remaining == 0);
            if next.is_none() && wait {
                let pipeline = self.hours.as_ref().expect("started above");
                while next.is_none() {
                    let built = pipeline.wait()?;
                    if built.epoch != epoch {
                        if let Some(dropper) = &self.dropper {
                            dropper.drop_later(built);
                        }
                        continue;
                    }
                    if (built.branch, built.hour) == target {
                        next = Some(built);
                    } else {
                        self.ready.insert((built.branch, built.hour), built);
                    }
                }
            }
            if next.is_none() && self.playing {
                // The newest hour on the way from the one on screen.
                let on_the_way = self
                    .ready
                    .range(..target)
                    .next_back()
                    .map(|(key, _)| *key)
                    .filter(|key| {
                        key.0 == target.0
                            && shown_at.is_none_or(|shown| shown.0 != key.0 || shown.1 < key.1)
                    });
                if let Some(key) = on_the_way {
                    next = self.ready.remove(&key);
                    let later = self.ready.split_off(&key);
                    let passed = std::mem::replace(&mut self.ready, later);
                    if let Some(dropper) = &self.dropper {
                        dropper.drop_later(passed);
                    }
                }
            }
        }
        if let Some(built) = next {
            self.apply_hour(state, document, built, laps)?;
        }
        // The divisions glide past the playhead's hour only when it is the
        // hour on screen.
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
        let passed = document.playback.present(branch, hour, current);
        let dropper = self
            .dropper
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the hour dropper was not started"))?;
        dropper.drop_later(passed);
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
                self.shown = Some(ShownHour {
                    epoch,
                    branch,
                    hour,
                    error: Some(error),
                    political_warnings: Vec::new(),
                    political_summary: (0, 0),
                    political_build: Duration::ZERO,
                    limitations: Default::default(),
                    snow_line: String::new(),
                    header: Default::default(),
                });
                return Ok(());
            }
        };
        let hours::Layers {
            political,
            borders,
            rails,
            buildings,
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
            scene.set_map_buildings(
                &state.device,
                &state.queue,
                &document.map_buildings,
                buildings.value.clone(),
            )?;
            uploaded.buildings = Some(buildings.id);
        }
        laps.lap("buildings_upload");
        let replaced =
            scene.set_trains(&state.device, &state.queue, document.trains.clone(), trains)?;
        dropper.drop_later(replaced);
        laps.lap("trains_upload");
        let replaced = scene.set_aircraft(
            &state.device,
            &state.queue,
            document.aircraft.clone(),
            aircraft,
        )?;
        dropper.drop_later(replaced);
        laps.lap("aircraft_upload");
        dropper.drop_later(self.unit_models.replace(units));
        self.unit_fraction = None;
        scene.set_orders(&state.device, &state.queue, &document.orders, &orders)?;
        dropper.drop_later(orders);
        laps.lap("orders_upload");
        scene.set_warmed_weather_effects(
            &state.device,
            &state.queue,
            document.weather_effects.clone(),
            effects,
        )?;
        laps.lap("effects_upload");
        if uploaded.political != Some(political.id) {
            let textures = &political.layers.textures;
            let rows = political
                .rows_from
                .iter()
                .find(|(base, _)| uploaded.political == Some(*base));
            match rows {
                Some((_, rows)) => scene.write_gradient_rows(
                    &state.queue,
                    &textures.channel1,
                    &textures.channel2,
                    [textures.width, textures.height],
                    rows,
                )?,
                _ => scene.set_gradient_textures(
                    &state.device,
                    &state.queue,
                    &textures.channel1,
                    &textures.channel2,
                    textures.width,
                    textures.height,
                )?,
            }
            uploaded.political = Some(political.id);
        }
        laps.lap("gradient_upload");
        // borders: the window has no demilitarised-zone override, so the zone is the
        // recorded one.
        if uploaded.borders != Some(borders.id) {
            scene.set_border_plans(&state.device, &borders.classes, &borders.highlighted)?;
            uploaded.borders = Some(borders.id);
        }
        laps.lap("border_classes");
        // weather: an hour whose grid equals the one on the GPU writes nothing.
        let same_snow = uploaded.snow_mud.as_ref().is_some_and(|had| {
            had.width == snow_mud.width
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
        scene.set_fog_of_war(&state.queue, &intel.texels, intel.fog)?;
        laps.lap("fog_upload");
        dropper.drop_later(self.placements.replace(names));
        self.placements_at = Some((branch, hour, view.states));
        dropper.drop_later(self.prepared_overlay.replace(overlay));
        self.overlay_at = Some((
            branch,
            hour,
            view.camera,
            view.viewport,
            self.perspective.clone(),
        ));
        self.overlay_fraction = None;
        let shown = ShownHour {
            epoch,
            branch,
            hour,
            error: None,
            political_warnings: political.layers.warnings().to_vec(),
            political_summary: political.layers.summary(),
            political_build: political.layers.build,
            limitations,
            header,
            snow_line: format!(
                "Snow and mud: {}; {}x{} grid rasterised in {:.0} ms",
                snow_mud.summary.line(),
                snow_mud.width,
                snow_mud.height,
                snow_mud.build.as_secs_f64() * 1000.0
            ),
        };
        dropper.drop_later(self.shown.replace(shown));
        dropper.drop_later((political, borders, rails, buildings, snow_mud, intel));
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
        // The hour on screen may be one the collector had not finished filling;
        // what that costs the render leads the warning area, because it is the
        // part that changes with the hour. The persistent content limitations
        // and the political layer's follow, in the order `--dump` prints them,
        // so a capture beside a dump lists the same lines in the same order.
        let mut limitations = shown.limitations.clone();
        limitations.set_fraction(document.hour_fraction);
        let mut warnings = limitations.lines();
        laps.lap("limitations");
        warnings.extend(document.warnings.iter().cloned());
        warnings.extend(self.combat.warnings());
        warnings.extend(shown.political_warnings.iter().cloned());
        if !self.dashboard_map && self.embedded.is_none() {
            egui::ScrollArea::vertical()
                .id_salt("warnings")
                .max_height(ui.available_height() * WARNING_SHARE)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for warning in &warnings {
                        ui.colored_label(egui::Color32::YELLOW, warning);
                    }
                });
            let (filled, distinct) = shown.political_summary;
            ui.weak(format!(
            "Political layer: {filled} provinces filled in {distinct} country colors, rebuilt in {:.0} ms",
            shown.political_build.as_secs_f64() * 1000.0
        ));
            // weather
            ui.weak(&shown.snow_line);
        }
        let size = ui.available_size().max(egui::vec2(1.0, 1.0));
        // capture: `--window-dump` renders at the size it was asked for rather
        // than at the size the panels left over, so the frame it writes does not
        // depend on this display's scale factor or on where the warning area
        // wrapped. Every other frame renders the map area at device pixels,
        // which is the space the projection and the overlay batch already use.
        let pinned = self.capture.as_ref().and_then(|capture| capture.pinned);
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
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.combat.close();
        }
        let combat_geometry = self
            .combat
            .draw(document, pixels, &self.perspective)
            .map_err(|error| error.to_string())?;
        let point = response
            .hover_pos()
            .or_else(|| response.interact_pointer_pos())
            .map(physical);
        if response.drag_started() {
            self.combat.dragging = point.is_some_and(|point| {
                combat_geometry.as_ref().is_some_and(|window| {
                    window.contains(point) && point[1] < window.rect[1] + 35.0
                })
            });
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
        if response.clicked()
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
                self.combat.offset[0] += delta.x * scale;
                self.combat.offset[1] += delta.y * scale;
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
        if map::unit_models::draws_any(camera.height)
            && self.unit_fraction
                != Some((
                    document.hour_fraction,
                    map::unit_models::draws(camera.height),
                    camera.height < map::unit_models::SHIPS_DISTANCE_CUTOFF,
                ))
        {
            let timer = Instant::now();
            let units = self.unit_models.as_ref().expect("prepared above").field(
                &document.unit_models,
                document.hour_fraction,
                camera.height,
            );
            // Size the buffer for every height band: height 0 draws all land
            // units and ships, so zooming between bands rewrites the same
            // buffer instead of reallocating mid-gesture. The count is read
            // off the prepared hour rather than by building that field.
            let hour_key = (document.playback.branch(), document.playback.hour());
            let fresh_hour = self
                .unit_capacity
                .as_ref()
                .is_none_or(|(branch, hour, _)| (*branch, *hour) != hour_key);
            let min_capacity = if fresh_hour {
                let full = self
                    .unit_models
                    .as_ref()
                    .expect("prepared above")
                    .full_byte_len() as u64;
                let capacity = full.max(units.byte_len() as u64);
                self.unit_capacity = Some((hour_key.0, hour_key.1, capacity));
                capacity
            } else {
                let (_, _, capacity) = self.unit_capacity.as_ref().expect("checked above");
                (*capacity).max(units.byte_len() as u64)
            };
            scene
                .set_unit_models(
                    &state.device,
                    &state.queue,
                    &document.unit_models,
                    units,
                    min_capacity,
                )
                .map_err(|error| error.to_string())?;
            laps.lap("units_upload");
            self.unit_fraction = Some((
                document.hour_fraction,
                map::unit_models::draws(camera.height),
                camera.height < map::unit_models::SHIPS_DISTANCE_CUTOFF,
            ));
            if map::counter_move::profiling() {
                map::counter_move::profile_print(format!(
                    "PROFILE models {:.6}",
                    timer.elapsed().as_secs_f64() * 1000.0
                ));
            }
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
            .get_or_insert_with(|| LayerFade::instant(camera.height))
            .step(camera.height, delta);
        self.fade = Some(fade);
        let states = LayerFade::wants_states(camera.height) || fade.state > 0.0;
        laps.lap("panel_camera_units");
        let now = document.hour().map_err(|error| error.to_string())?;
        self.last_view = Some(hours::View {
            camera,
            viewport: pixels,
            states,
        });
        let names_key = (document.playback.branch(), document.playback.hour(), states);
        if self.placements_at != Some(names_key) {
            let same_hour = self
                .placements_at
                .is_some_and(|(branch, hour, _)| (branch, hour) == (names_key.0, names_key.1));
            match &document.playback.current() {
                // Only the state layer changed: place it on a worker and keep
                // drawing the hour's country layer until it arrives, so the
                // cutoff never stalls the frame on a whole-map placement.
                crate::playback::Current::Frame(frame) if same_hour => {
                    if self
                        .names_job
                        .as_ref()
                        .is_none_or(|job| job.key != names_key)
                    {
                        self.names_job = Some(NamesJob::spawn(
                            names_key,
                            Arc::clone(&document.content),
                            Arc::clone(frame),
                            ui.ctx().clone(),
                        ));
                    }
                }
                _ => match now.place_names(states) {
                    Ok(placements) => {
                        self.placements = Some(placements);
                        self.placements_at = Some(names_key);
                        self.names_job = None;
                    }
                    Err(error) => return Err(error.to_string()),
                },
            }
            if let Some(job) = self.names_job.take_if(|job| job.key == names_key)
                && let Some(result) = job.poll(&mut self.names_job)
            {
                self.placements = Some(result.map_err(|error| error.to_string())?);
                self.placements_at = Some(names_key);
            }
        }
        laps.lap("names");
        // counters: one batch carries the bases, the labels, the counters and
        // the combat icons in `DEFAULT_PRIORITY_*` order.
        let overlay_key = (
            document.playback.branch(),
            document.playback.hour(),
            camera,
            pixels,
            self.perspective.clone(),
        );
        let overlay_timer = Instant::now();
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
        if stale || self.overlay_at.as_ref() != Some(&overlay_key) {
            let tables_key = (
                document.playback.branch(),
                document.playback.hour(),
                self.perspective.clone(),
            );
            if self
                .overlay_tables
                .as_ref()
                .is_none_or(|(branch, hour, perspective, _)| {
                    (*branch, *hour, perspective) != (tables_key.0, tables_key.1, &tables_key.2)
                })
            {
                let tables = now
                    .hour_tables(&self.perspective)
                    .map_err(|error| error.to_string())?;
                self.overlay_tables = Some((tables_key.0, tables_key.1, tables_key.2, tables));
            }
            let tables = &self.overlay_tables.as_ref().expect("parsed above").3;
            self.prepared_overlay = Some(
                now.prepare_overlay_cached(camera, pixels, tables)
                    .map_err(|error| error.to_string())?,
            );
            self.overlay_at = Some(overlay_key);
            self.overlay_fraction = None;
        }
        laps.lap("overlay_prepare");
        if self.overlay_fraction != Some(document.hour_fraction) {
            let (batch, label_report, counter_report, _raids) = self
                .prepared_overlay
                .as_mut()
                .expect("prepared above")
                .draw(&now, camera, pixels, Default::default())
                .map_err(|error| error.to_string())?;
            self.label_report = label_report;
            self.counter_report = counter_report;
            self.overlay = Some(batch);
            self.overlay_fraction = Some(document.hour_fraction);
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
        let label_batch = if let Some(drawn) = self
            .combat
            .draw(document, pixels, &self.perspective)
            .map_err(|error| error.to_string())?
        {
            warnings.extend(drawn.warnings.iter().cloned());
            combat_batch = self.overlay.as_ref().expect("built above").clone();
            combat_batch.quads.extend(drawn.batch.quads);
            &combat_batch
        } else {
            self.overlay.as_ref().expect("built above")
        };
        // capture: the animation clock is the one input a live window cannot
        // share with a headless frame, so a capture frame renders at 0, the
        // still frame `--dump` renders. `Scene::render` then takes the
        // luminance adaptation instantaneously, because the clock did not
        // advance, which is what a dump's own pair of frames does.
        let animation_time = self
            .capture
            .as_ref()
            .and_then(|c| c.time)
            .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
        let scene = self.scene.as_mut().unwrap();
        scene.set_graphics(self.graphics);
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
                        names: self.placements.as_ref(),
                        layer_alpha: fade.as_array(),
                        sprites: Some(label_batch),
                    },
                    // trees
                    season: now.tree_season(),
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
                "PROFILE frame {} fraction {} height {} total {:.6} overlay {:.6} spline {:.6} hour {} frame_total {:.3} outside {:.3} scene {:.3}/{:.3}/{:.3}/{:.3} gpu {:.3} laps {}",
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
                laps.line()
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
            if !laid_out || document.icons.lazy.has_pending() {
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
        // The water animates, so the window keeps drawing.
        ui.ctx().request_repaint();
        // The game shows a moving division's arrival in its counter's tooltip, so the
        // hovered counter's line replaces the map's own hint.
        let arrival = point.filter(|_| !over_combat).and_then(|point| {
            let hit = self
                .counter_report
                .arrivals
                .iter()
                .rev()
                .find(|hit| hit.contains(point))?;
            let arrival = map::counter_eta::arrival(document.playback.hour(), hit.remaining)?;
            Some(map::counter_eta::sentence(
                &arrival,
                hit.ours,
                &date(arrival.hour),
            ))
        });
        match arrival {
            Some(text) => response.on_hover_text(text),
            None => response.on_hover_text("Drag to pan · Scroll to zoom"),
        };
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
        if self.dashboard_active
            && let Some(dashboard) = &mut self.dashboard
        {
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
        self.poll();
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
                ui.weak(format!(
                    "Viewed as {}, fog of war {} · Terrain, water, political fill, borders, names and labels",
                    viewing.as_deref().unwrap_or("no country"),
                    if self.perspective.fog { "on" } else { "off" },
                ));
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
                        crate::playback::Current::Frame(frame) => {
                            egui::ScrollArea::vertical()
                                .id_salt("tables")
                                .max_height(TABLE_LIST_HEIGHT)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        for table in TableId::ALL {
                                            match &frame.tables[table.index()] {
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
                if let Some(playhead) = &mut self.playhead {
                    if (self.profile_frame == 121 || self.profile_frame == 181) && mode == "hours" {
                        playhead.hour += if self.profile_frame == 121 { 1 } else { -1 };
                        playhead.fraction = 0.0;
                    }
                    // A cold seek: jump to the branch's last hour, whose
                    // per-branch caches (the snow and mud display cache) start
                    // empty.
                    if self.profile_frame == 121
                        && mode == "seek_end"
                        && let Some((_, last)) = document.playback.range(playhead.branch)
                    {
                        playhead.hour = last;
                        playhead.fraction = 0.0;
                    }
                }
                // Steady playback at `play:<hours per second>`, sampled as a
                // 60 Hz clock over frames 121..240, as the timeline would run.
                let playing = std::env::var("BATTLE_PLAN_VIEWER_PROFILE").or_else(|_| std::env::var("HOI4_VIEWER_PROFILE"))
                    .ok()
                    .and_then(|mode| mode.strip_prefix("play:")?.parse::<f32>().ok());
                self.playing = playing.is_some() && (121..241).contains(&self.profile_frame);
                if let Some(speed) = playing {
                    // The builders reach as far ahead as the timeline at this speed.
                    self.speed = speed as u32;
                }
                if let Some(speed) = playing
                    && (121..241).contains(&self.profile_frame)
                    && let Some(mut moved) = self.playhead
                {
                    let last = document
                        .playback
                        .range(moved.branch)
                        .map_or(moved.hour, |(_, last)| last);
                    let fraction = f64::from(moved.fraction) + f64::from(speed) / 60.0;
                    self.advance_playhead(&mut moved, fraction, last);
                    self.playhead = Some(moved);
                }
                if playing.is_none()
                    && !mode.starts_with("pan:")
                    && !mode.starts_with("zoom:")
                    && let Some(playhead) = &mut self.playhead
                {
                    playhead.fraction = if self.profile_frame < 121 || self.profile_frame >= 241 {
                        self.options.hour_fraction
                    } else {
                        ((self.profile_frame - 121) % 120) as f32 / 120.0
                    };
                }
                // A camera sweep: `pan:<world units per second>` drifts the
                // pinned camera east, `zoom:<height factor per second>`
                // compounds its height, over frames 121..240 with the hour
                // held. Every frame rebuilds the overlay synchronously for
                // its camera, which is the interactive-pan cost being
                // measured; the hour builders chase the moving view behind
                // it. Needs the pinned `--camera` the harness passes.
                let sweep = mode
                    .strip_prefix("pan:")
                    .and_then(|rate| rate.parse::<f32>().ok().map(|rate| (rate / 60.0, 1.0)))
                    .or_else(|| {
                        mode.strip_prefix("zoom:").and_then(|factor| {
                            factor
                                .parse::<f32>()
                                .ok()
                                .filter(|factor| *factor > 0.0)
                                .map(|factor| (0.0, factor.powf(1.0 / 60.0)))
                        })
                    });
                if let Some((dx, zoom)) = sweep
                    && (121..241).contains(&self.profile_frame)
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

/// One off-thread name placement for the hour on screen.
struct NamesJob {
    key: (replay::BranchId, i32, bool),
    result: mpsc::Receiver<std::io::Result<Placements>>,
}

impl NamesJob {
    fn spawn(
        key: (replay::BranchId, i32, bool),
        content: Arc<crate::document::Content>,
        frame: Arc<replay::frame::DecodedFrame>,
        ctx: egui::Context,
    ) -> Self {
        let (send, result) = mpsc::channel();
        std::thread::spawn(move || {
            let placed = crate::document::Hour::new(&content, &frame, 0.0).place_names(key.2);
            let _ = send.send(placed);
            ctx.request_repaint();
        });
        Self { key, result }
    }

    /// The finished placement, or puts the job back into `slot` to wait.
    fn poll(self, slot: &mut Option<Self>) -> Option<std::io::Result<Placements>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => {
                *slot = Some(self);
                None
            }
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(std::io::Error::other(
                "the name placement worker stopped",
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        viewer.poll();
        assert!(viewer.loading.is_none());
        assert!(viewer.error.is_none());
        assert!(viewer.options.replay.is_none());
    }

    #[test]
    fn seek_from_stale_host_state_reopens_its_explicit_snapshot() {
        let seek = crate::embedded::Seek {
            path: PathBuf::from("A.battleplan"),
            revision: "100:20:0".into(),
            branch: 1,
            hour: 12,
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
    fn display_uses_the_save_calendar_at_midnight() {
        assert_eq!(date(60_765_882), "1936-09-29  19:00");
        assert_eq!(date(60_765_887), "1936-09-29  24:00");
        assert_eq!(date(60_765_888), "1936-09-30  01:00");
    }
}
