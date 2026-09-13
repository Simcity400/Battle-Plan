use crate::{
    Config, Options, choose_initial,
    document::Document,
    map::{
        camera::Camera,
        frame::DayNight,
        // political fill
        political::{self, PoliticalCache},
        scene::Scene,
    },
};
// names
use crate::map::{
    // counters
    counters::CounterReport,
    labels::LabelReport,
    name_pass::{LayerFade, Placements},
    scene::{Overlay, Request},
    // weather
    weather::WeatherCache,
};
use eframe::{egui, wgpu};
use replay::{branches::Parentage, container::SeekResult, frame::TableView, rows::TableId};
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

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
    /// Cleared with the scene: the border geometry has to be uploaded into a new one.
    borders_uploaded: bool,
    // names
    placements: Option<Placements>,
    placements_at: Option<(replay::BranchId, i32, bool)>,
    fade: LayerFade,
    last_frame: Instant,
    label_report: LabelReport,
    // counters
    counter_report: CounterReport,
    // weather
    weather: WeatherCache,
}

impl Viewer {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: Config,
        options: Options,
        error: Option<String>,
    ) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        // names: the cross-fade starts already settled at the opening height.
        let camera_height = options
            .camera
            .map_or(crate::map::camera::MAX_HEIGHT, |camera| camera.height);
        let mut viewer = Self {
            game_text: config.game_dir.to_string_lossy().into_owned(),
            user_text: config.user_dir.to_string_lossy().into_owned(),
            settings: config.resolve_options().is_err(),
            camera: options.camera,
            camera_manual: options.camera.is_some(),
            control: options.control,
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
            // colour
            postfx: true,
            // political fill
            political: PoliticalCache::new(),
            borders_uploaded: false,
            // names
            placements: None,
            placements_at: None,
            fade: LayerFade::instant(camera_height),
            last_frame: Instant::now(),
            label_report: LabelReport::default(),
            // counters
            counter_report: CounterReport::default(),
            // weather
            weather: WeatherCache::new(),
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
        self.borders_uploaded = false;
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
        if !self.borders_uploaded {
            scene.set_borders(
                &state.device,
                &state.queue,
                &document.border_assets,
                &document.border_meshes,
                &document.straits,
            )?;
            self.borders_uploaded = true;
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
            scene.set_land_classes(&state.device, &layers.regions)?;
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
            self.borders_uploaded = false;
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
        for warning in layers.warnings() {
            ui.colored_label(egui::Color32::YELLOW, warning);
        }
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
        let pixels = [
            (size.x * ui.ctx().pixels_per_point()).round() as u32,
            (size.y * ui.ctx().pixels_per_point()).round() as u32,
        ];
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
        // names: the engine integrates the layer cross-fade by the frame delta
        // and clamps to [0, 1], so it takes one second whatever the zoom speed.
        let camera = *camera;
        let delta = self.last_frame.elapsed().as_secs_f32();
        self.last_frame = Instant::now();
        self.fade = self.fade.step(camera.height, delta);
        let states = LayerFade::wants_states(camera.height) || self.fade.state > 0.0;
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
        let (label_batch, label_report, counter_report) = document
            .overlay_batch(camera, pixels)
            .map_err(|error| error.to_string())?;
        self.label_report = label_report;
        self.counter_report = counter_report;
        let scene = self.scene.as_mut().unwrap();
        scene
            .render(
                &state.device,
                &state.queue,
                Request {
                    size: pixels,
                    camera,
                    day_night: DayNight::at_hour(document.playback.hour()),
                    animation_time: self.started.elapsed().as_secs_f32(),
                    // colour
                    postfx: self.postfx,
                    // names
                    overlay: Overlay {
                        names: self.placements.as_ref(),
                        layer_alpha: self.fade.as_array(),
                        sprites: Some(&label_batch),
                    },
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
                // names: the header names every layer that reaches a pixel.
                ui.weak(
                    "Omniscient · Terrain, water, political fill, borders, names and labels",
                );
            });
            ui.add_space(5.0);
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
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
                        ui.horizontal_wrapped(|ui| {
                            for table in TableId::ALL {
                                match &frame.tables[table.index()] {
                                    TableView::Complete(rows) => {
                                        ui.weak(format!("{table:?}: {}", rows.len()));
                                    }
                                    TableView::Truncated { missing, .. } => {
                                        ui.colored_label(
                                            egui::Color32::YELLOW,
                                            format!("{table:?}: truncated, {missing} missing"),
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
                    }
                }
            });
            egui::CentralPanel::default().show(ui, |ui| {
                for warning in &document.warnings {
                    ui.colored_label(egui::Color32::YELLOW, warning);
                }
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
