use crate::{
    Config,
    Options,
    choose_initial,
    document::Document,
    map::{
        camera::Camera,
        frame::DayNight,
        // political fill
        political::{self, PoliticalCache},
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
    // weather
    weather::WeatherCache,
};
use eframe::{egui, wgpu};
use replay::{branches::Parentage, container::SeekResult, frame::TableView, rows::TableId};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

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
/// its frame at `animation_time` 0, the still frame `--dump` renders.
struct Capture {
    path: PathBuf,
    /// Frames to draw before the readback. `--window-dump` draws the pair a
    /// headless dump draws, so the luminance adaptation has the same history.
    remaining: u32,
    /// `--window-dump`: the pixel size and camera the frame is pinned to, so
    /// neither the panel layout nor the display's scale factor decides them. A
    /// `None` camera is the fit to that size, which is the dump's default too.
    pinned: Option<([u32; 2], Option<Camera>)>,
    /// `--window-dump` closes the window once it has written its frame.
    exit: bool,
}

pub struct Viewer {
    config: Config,
    options: Options,
    game_text: String,
    user_text: String,
    settings: bool,
    document: Option<Document>,
    loading: Option<mpsc::Receiver<Result<Document, String>>>,
    error: Option<String>,
    scene: Option<Scene>,
    texture: Option<egui::TextureId>,
    generation: u64,
    map_error: Option<String>,
    camera: Option<Camera>,
    camera_manual: bool,
    control: bool,
    playing: bool,
    speed: u32,
    last_step: Instant,
    started: Instant,
    // colour
    /// Whether the map is graded through the engine's post chain.
    postfx: bool,
    // political fill
    political: PoliticalCache,
    /// Cleared with the scene: the border, river and tree geometry, and the
    /// close-zoom sprawl, rails and bridges, have to be uploaded into a new one.
    geometry_uploaded: bool,
    // close zoom
    /// The hour and perspective whose map buildings and unit models are in the
    /// scene; both come from recorded tables and are rewritten when either
    /// changes. The perspective is in the key because it decides which
    /// divisions have a model at all.
    close_zoom_hour: Option<(replay::BranchId, i32, map::perspective::Choice)>,
    // names
    placements: Option<Placements>,
    placements_at: Option<(replay::BranchId, i32, bool)>,
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
    /// `Document::overlay_batch` reads the decoded frame, the camera and the
    /// viewport and nothing else, and the window repaints continuously for the
    /// water animation, so without this it rebuilt every counter, base, label
    /// and combat icon about forty times a second for a result that had not
    /// changed: 22 of a still frame's 27 ms. Keyed like `PoliticalCache` and
    /// the name placements beside it.
    overlay: Option<Batch>,
    overlay_at: Option<(
        replay::BranchId,
        i32,
        Camera,
        [u32; 2],
        map::perspective::Choice,
    )>,
    /// Which country the counters are drawn for; the header's own control sets
    /// it and [`Document::perspective`] is kept in step with it.
    perspective: map::perspective::Choice,
    // weather
    weather: WeatherCache,
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
}

impl Viewer {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: Config,
        options: Options,
        error: Option<String>,
        // capture
        failure: Arc<Mutex<Option<String>>>,
    ) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        // capture: `--window-dump` arms the readback before the first frame, so
        // every frame it draws is a capture frame and the animation clock never
        // starts.
        let capture = options.window_dump.then(|| Capture {
            path: options.out.clone().unwrap_or_default(),
            remaining: 1,
            pinned: Some((options.size, options.camera)),
            exit: true,
        });
        // counters: the header's control starts where the command line put it.
        let perspective = options.perspective.clone();
        let mut viewer = Self {
            game_text: config.game_dir.to_string_lossy().into_owned(),
            user_text: config.user_dir.to_string_lossy().into_owned(),
            settings: config.resolve_options().is_err(),
            camera: options.camera,
            camera_manual: options.camera.is_some(),
            control: options.control,
            // colour: `--postfx off` reaches the window too, so a window and a
            // dump of the same state can be the same frame; the checkbox starts
            // where the command line put it.
            postfx: options.postfx,
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
            // political fill
            political: PoliticalCache::new(),
            geometry_uploaded: false,
            // close zoom
            close_zoom_hour: None,
            // names
            placements: None,
            placements_at: None,
            fade: None,
            last_frame: Instant::now(),
            label_report: LabelReport::default(),
            // counters
            counter_report: CounterReport::default(),
            overlay: None,
            overlay_at: None,
            perspective,
            // weather
            weather: WeatherCache::new(),
            // capture
            capture,
            capture_note: None,
            failure,
            captures: 0,
        };
        if !viewer.settings
            && let Some(path) = viewer.options.replay.clone()
        {
            viewer.open(path, &cc.egui_ctx);
        }
        viewer
    }

    fn open(&mut self, path: PathBuf, ctx: &egui::Context) {
        let resolve = match self.config.resolve_options() {
            Ok(resolve) => resolve,
            Err(error) => {
                self.error = Some(error.to_string());
                self.settings = true;
                return;
            }
        };
        self.options.replay = Some(path.clone());
        let options = self.options.clone();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = Document::load(&path, &resolve, options.allow_unverified)
                .and_then(|mut document| {
                    choose_initial(&mut document, &options)?;
                    Ok(document)
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.loading = Some(receiver);
        self.document = None;
        self.scene = None;
        // political fill
        self.political = PoliticalCache::new();
        self.geometry_uploaded = false;
        // close zoom
        self.close_zoom_hour = None;
        // weather
        self.weather = WeatherCache::new();
        self.generation = 0;
        self.error = None;
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
        self.overlay_at = None;
    }

    fn poll(&mut self) {
        let result = self.loading.as_ref().map(|receiver| receiver.try_recv());
        match result {
            Some(Ok(Ok(document))) => {
                self.document = Some(document);
                self.loading = None;
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
                    self.settings = false;
                    self.error = None;
                    if let Some(path) = self.options.replay.clone() {
                        self.open(path, ctx);
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    fn timeline(&mut self, ui: &mut egui::Ui, document: &mut Document) {
        let mut selected_session = document.playback.session();
        let mut selected_branch = document.playback.branch();
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
                            format!("Session {} · process {}", index + 1, session.process_id),
                        );
                    }
                });
            if selected_session != document.playback.session()
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
            if selected_branch != document.playback.branch() {
                self.playing = false;
                if let Err(error) = document.playback.select_branch(selected_branch) {
                    self.error = Some(error.to_string());
                }
            }
            if let Some(branch) = document
                .playback
                .replay()
                .branches
                .iter()
                .find(|branch| branch.id == document.playback.branch())
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
            match document.playback.parent_support(document.playback.branch()) {
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
        let branch = document.playback.branch();
        let Some((first, last)) = document.playback.range(branch) else {
            return;
        };
        let mut hour = document.playback.hour();
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
        if hour != document.playback.hour()
            && let Err(error) = document.playback.seek(branch, hour)
        {
            self.error = Some(error.to_string());
        }
        if self.playing
            && self.last_step.elapsed() >= Duration::from_secs_f64(1.0 / f64::from(self.speed))
        {
            self.last_step = Instant::now();
            match document.playback.step() {
                Ok(true) => {}
                Ok(false) => self.playing = false,
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.playing = false;
                }
            }
        }
        if self.playing {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(1.0 / f64::from(self.speed)));
        }
        ui.horizontal(|ui| {
            let displayed = document.playback.hour();
            ui.strong(date(displayed));
            ui.weak(format!("Hour {displayed}"));
            // colour
            ui.checkbox(&mut self.postfx, "Colour grading")
                .on_hover_text("The engine's post chain: exposure, tone map, colour LUT");
        });
    }

    // political fill
    /// Rebuilds and uploads the political layers, and re-buckets the land borders with them.
    ///
    /// Both are keyed on the Provinces rows and the mode, so a step to an hour that changed neither
    /// costs nothing (`map::political::PoliticalCache`).
    fn refresh_layers(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        document: &Document,
    ) -> Result<(), std::io::Error> {
        let mode = political::Mode::from_control(self.control);
        let rebuilt = self
            .political
            .ensure(&document.map, document.frame()?, mode)?;
        let scene = self
            .scene
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the map scene was not created"))?;
        if !self.geometry_uploaded {
            scene.set_borders(
                &state.device,
                &state.queue,
                &document.border_assets,
                &document.border_meshes,
                &document.straits,
            )?;
            // rivers and trees: both are traced from static content, so like the
            // border meshes they are uploaded once into a new scene and never
            // rebuilt. Without this the window's `Scene` keeps `rivers` and
            // `trees` at `None` and `Scene::render` skips both passes, which is
            // two whole map layers the window did not draw and `--dump` did.
            scene.set_rivers(
                &state.device,
                &state.queue,
                &document.river_assets,
                &document.river_mesh,
            )?;
            scene.set_trees(
                &state.device,
                &state.queue,
                &document.tree_assets,
                document.tree_geometry.clone(),
                &document.tree_field,
            )?;
            // close zoom: the sprawl, the rails and the bridges are static
            // content too, and take the same one-time upload. The map buildings
            // and the unit models are not: they come from recorded tables and
            // follow the hour, below.
            scene.set_sprawl(&state.device, &state.queue, &document.sprawl)?;
            scene.set_railways(
                &state.device,
                &state.queue,
                &document.rail_assets,
                &document.rail_mesh,
            )?;
            scene.set_bridges(&state.device, &state.queue, &document.bridges)?;
            self.geometry_uploaded = true;
        }
        let hour = (
            document.playback.branch(),
            document.playback.hour(),
            self.perspective.clone(),
        );
        if self.close_zoom_hour.as_ref() != Some(&hour) {
            let (buildings, _) = document.map_building_field()?;
            scene.set_map_buildings(
                &state.device,
                &state.queue,
                &document.map_buildings,
                buildings,
            )?;
            let (units, _) = document.unit_model_field(&self.perspective)?;
            scene.set_unit_models(&state.device, &state.queue, &document.unit_models, units)?;
            self.close_zoom_hour = Some(hour);
        }
        if let (true, Some(layers)) = (rebuilt, self.political.layers()) {
            let textures = &layers.textures;
            scene.set_gradient_textures(
                &state.device,
                &state.queue,
                &textures.channel1,
                &textures.channel2,
                textures.width,
                textures.height,
            )?;
            // borders: the window has no demilitarised-zone override, so the zone is the
            // recorded one, which is empty until the recorder carries the flag.
            let zone = document.demilitarized_zone(&layers.regions)?;
            scene.set_classes(
                &state.device,
                &document.border_meshes,
                &layers.regions,
                &zone,
            )?;
        }
        // weather: the snow and mud grid follows the same rebuild-then-upload
        // rule, keyed on the weather rows rather than the provinces.
        if self
            .weather
            .ensure(&document.map, &document.assets.perm_snow, document.frame()?)?
            && let Some(snow_mud) = self.weather.data()
        {
            scene.set_snow_mud(
                &state.device,
                &state.queue,
                &snow_mud.texels,
                snow_mud.width,
                snow_mud.height,
            )?;
        }
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
        let capture = self.capture.take().expect("matched above");
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
            render::write_png(scene, &state.device, &state.queue, &capture.path)
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
                    "Captured branch {}, hour {} to {} at {}x{}",
                    document.playback.branch(),
                    document.playback.hour(),
                    capture.path.display(),
                    pixels[0],
                    pixels[1]
                );
                println!(
                    "Camera: {},{},{}",
                    camera.center[0], camera.center[1], camera.height
                );
                println!(
                    "Mode: {}; colour grading {}",
                    if self.control { "controller" } else { "owner" },
                    if self.postfx { "on" } else { "off" }
                );
                self.capture_note = Some(format!("Saved {}", capture.path.display()));
            }
            Err(error) => {
                let message = format!("{}: {error}", capture.path.display());
                if capture.exit
                    && let Ok(mut failure) = self.failure.lock()
                {
                    *failure = Some(message.clone());
                }
                eprintln!("{message}");
                self.capture_note = Some(message);
            }
        }
        if capture.exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn map(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        document: &Document,
    ) -> Result<(), String> {
        let state = frame
            .wgpu_render_state()
            .ok_or("WGPU renderer is unavailable")?;
        if self.scene.is_none() {
            self.scene = Some(
                Scene::new(
                    &state.device,
                    &state.queue,
                    &document.assets,
                    // names
                    &document.icons,
                )
                .map_err(|error| error.to_string())?,
            );
            // political fill: a fresh scene holds no uploaded layers or geometry.
            self.political = PoliticalCache::new();
            self.geometry_uploaded = false;
            // close zoom
            self.close_zoom_hour = None;
            // weather
            self.weather = WeatherCache::new();
        }
        // political fill
        self.map_error = self
            .refresh_layers(state, document)
            .err()
            .map(|error| error.to_string());
        if let Some(error) = &self.map_error {
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            });
            return Ok(());
        }
        let layers = self
            .political
            .layers()
            .ok_or("The political layers are unavailable")?;
        // The hour on screen may be one the collector had not finished filling;
        // what that costs the render leads the warning area, because it is the
        // part that changes with the hour. The persistent content limitations
        // and the political layer's follow, in the order `--dump` prints them,
        // so a capture beside a dump lists the same lines in the same order.
        let limitations = document
            .limitations(&self.perspective)
            .map_err(|error| error.to_string())?;
        let mut warnings = limitations.lines().to_vec();
        warnings.extend(document.warnings.iter().cloned());
        warnings.extend(layers.warnings().iter().cloned());
        egui::ScrollArea::vertical()
            .id_salt("warnings")
            .max_height(ui.available_height() * WARNING_SHARE)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for warning in &warnings {
                    ui.colored_label(egui::Color32::YELLOW, warning);
                }
            });
        let (filled, distinct) = layers.summary();
        ui.weak(format!(
            "Political layer: {filled} provinces filled in {distinct} country colors, rebuilt in {:.0} ms",
            layers.build.as_secs_f64() * 1000.0
        ));
        // weather
        if let Some(snow_mud) = self.weather.data() {
            ui.weak(format!(
                "Snow and mud: {}; {}x{} grid rasterised in {:.0} ms",
                snow_mud.summary.line(),
                snow_mud.width,
                snow_mud.height,
                snow_mud.build.as_secs_f64() * 1000.0
            ));
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
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
        let map_size = [document.map.width as f32, document.map.height as f32];
        if !self.camera_manual {
            self.camera = Some(Camera::fit(map_size, pixels));
        }
        let camera = self
            .camera
            .get_or_insert_with(|| Camera::fit(map_size, pixels));
        // Screen positions reach the camera in physical pixels, the same space
        // the render target and the projection use.
        let scale = ui.ctx().pixels_per_point();
        let physical = |position: egui::Pos2| {
            [
                (position.x - rect.left()) * scale,
                (position.y - rect.top()) * scale,
            ]
        };
        if response.dragged()
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
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera_manual = true;
                let pointer = response.hover_pos().unwrap_or(rect.center());
                camera.zoom((scroll * 0.002).exp(), physical(pointer), pixels);
            }
        }
        let camera = match pinned {
            Some((size, camera)) => camera.unwrap_or_else(|| Camera::fit(map_size, size)),
            None => *camera,
        };
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
        let names_key = (document.playback.branch(), document.playback.hour(), states);
        if self.placements_at != Some(names_key) {
            match document.place_names(states) {
                Ok(placements) => {
                    self.placements = Some(placements);
                    self.placements_at = Some(names_key);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        // counters: one batch carries the bases, the labels, the counters and
        // the combat icons in `DEFAULT_PRIORITY_*` order.
        let overlay_key = (
            document.playback.branch(),
            document.playback.hour(),
            camera,
            pixels,
            self.perspective.clone(),
        );
        if self.overlay_at.as_ref() != Some(&overlay_key) {
            let (batch, label_report, counter_report) = document
                .overlay_batch(camera, pixels, &self.perspective)
                .map_err(|error| error.to_string())?;
            self.label_report = label_report;
            self.counter_report = counter_report;
            self.overlay = Some(batch);
            self.overlay_at = Some(overlay_key);
        }
        let label_batch = self.overlay.as_ref().expect("built above");
        // capture: the animation clock is the one input a live window cannot
        // share with a headless frame, so a capture frame renders at 0, the
        // still frame `--dump` renders. `Scene::render` then takes the
        // luminance adaptation instantaneously, because the clock did not
        // advance, which is what a dump's own pair of frames does.
        let animation_time = match self.capture {
            Some(_) => 0.0,
            None => self.started.elapsed().as_secs_f32(),
        };
        let scene = self.scene.as_mut().unwrap();
        scene
            .render(
                &state.device,
                &state.queue,
                Request {
                    size: pixels,
                    camera,
                    day_night: DayNight::at_hour(document.playback.hour()),
                    animation_time,
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
                    season: document.tree_season(),
                },
            )
            .map_err(|e| e.to_string())?;
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
        self.save_capture(state, document, camera, pixels, &warnings, ui.ctx());
        ui.painter().image(
            self.texture.unwrap(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // The water animates, so the window keeps drawing.
        ui.ctx().request_repaint();
        response.on_hover_text("Drag to pan · Scroll to zoom");
        Ok(())
    }
}

impl eframe::App for Viewer {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.poll();
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
            });
        }
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
                    self.open(path, ui.ctx());
                }
                if ui.button("Folders").clicked() {
                    self.settings = true;
                }
                ui.separator();
                // political fill
                ui.selectable_value(&mut self.control, false, "Political")
                    .on_hover_text("Fill each province in its owner's map colour");
                ui.selectable_value(&mut self.control, true, "Control")
                    .on_hover_text(
                        "Fill each province in its controller's map colour, as the game's own political map does",
                    );
                if ui.button("Fit map").clicked() {
                    self.camera = None;
                    self.camera_manual = false;
                }
                // counters: which country the unit counters are drawn for. The
                // recorded player is the default; omniscient is the explicit
                // alternative, not a fallback anyone reaches by accident.
                ui.separator();
                ui.selectable_value(
                    &mut self.perspective,
                    map::perspective::Choice::Recorded,
                    "Player view",
                )
                .on_hover_text(
                    "Draw the unit counters as the recording's own human player saw them: a stack                      it has no intelligence on carries the game's question mark and no bars",
                );
                ui.selectable_value(
                    &mut self.perspective,
                    map::perspective::Choice::Omniscient,
                    "Omniscient",
                )
                .on_hover_text(
                    "Draw every recorded stack in full, including the ones the game hid from its                      player",
                );
                // names: the header names every layer that reaches a pixel.
                ui.weak(format!(
                    "{} · Terrain, water, political fill, borders, names and labels",
                    match &self.perspective {
                        map::perspective::Choice::Omniscient => "Omniscient".to_owned(),
                        choice => format!("Counters as {choice} sees them"),
                    }
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
        self.paths(ui.ctx());
        if let Some(mut document) = self.document.take() {
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
                    SeekResult::Gap { reason, .. } => {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("Gap: {reason}"));
                    }
                    SeekResult::Frame(frame) => {
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
            egui::CentralPanel::default().show(ui, |ui| {
                if let Err(error) = self.map(ui, frame, &document) {
                    self.error = Some(error);
                }
            });
            self.document = Some(document);
        } else {
            egui::CentralPanel::default().show(ui, |ui| {
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
    }
}

fn date(hour: i32) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_uses_the_save_calendar_at_midnight() {
        assert_eq!(date(60_765_882), "1936-09-29  19:00");
        assert_eq!(date(60_765_887), "1936-09-29  24:00");
        assert_eq!(date(60_765_888), "1936-09-30  01:00");
    }
}
