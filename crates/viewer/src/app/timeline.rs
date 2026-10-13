use super::{Playhead, Viewer, advance_clock, date};
use crate::{Config, document::Document, map};
use eframe::egui;
use replay::branches::Parentage;
use std::{path::PathBuf, time::Instant};

impl Viewer {
    /// The standalone window's "UI scale" setting, saved to `viewer.json`.
    pub(super) fn ui_scale_control(&mut self, ui: &mut egui::Ui) {
        let matched = self
            .document
            .as_ref()
            .and(self.match_game.as_ref())
            .map(|(_, (raster, _))| raster.screen_scale());
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
    pub(super) fn counter_controls(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn paths(&mut self, ctx: &egui::Context) {
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

    pub(super) fn timeline(&mut self, ui: &mut egui::Ui, document: &mut Document) {
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
        // A recorded event loss marks the interval leading to its second
        // read, whose recorded events are incomplete; gaps paint over it, so
        // a lost frame at that read stays visible.
        for (after_hour, hour) in document.playback.replay().event_losses(branch) {
            let left = rect.left()
                + ((f64::from(after_hour) - f64::from(first) + 1.0) / span) as f32 * rect.width();
            let right = rect.left()
                + ((f64::from(hour) - f64::from(first) + 1.0) / span) as f32 * rect.width();
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(right.max(left + 1.0), rect.bottom()),
                ),
                0.0,
                egui::Color32::GOLD,
            );
        }
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
            if ui
                .checkbox(&mut self.graphics.day_night, "Day/night cycle")
                .changed()
            {
                match Config::save_day_night(self.graphics.day_night) {
                    Ok(()) => self.config.day_night = Some(self.graphics.day_night),
                    Err(error) => self.error = Some(format!("viewer.json: {error}")),
                }
            }
            if ui
                .checkbox(&mut self.graphics.show_allied_plans, "Allied battle plans")
                .changed()
            {
                match Config::save_allied_plans(self.graphics.show_allied_plans) {
                    Ok(()) => self.config.show_allied_plans = Some(self.graphics.show_allied_plans),
                    Err(error) => self.error = Some(format!("viewer.json: {error}")),
                }
            }
        });
    }
}
