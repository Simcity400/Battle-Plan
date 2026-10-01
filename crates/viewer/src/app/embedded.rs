use super::{Capture, Playhead, Viewer, seek_open_target};
use crate::{Config, map};
use eframe::{egui, wgpu};
use replay::{frame::TableView, rows::TableId};
use std::{path::PathBuf, sync::Arc, time::Instant};

impl Viewer {
    pub(super) fn embedded_logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
                    Ok(mut dashboard) => {
                        dashboard.set_diagnostics(self.diagnostics);
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
            self.poll(ctx);
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
        self.poll(ctx);
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
                    self.playing = seek.playback.is_some_and(|clock| clock.playing);
                    self.last_step = Instant::now();
                    let animation_target = map::presentation::Target {
                        branch: seek.branch,
                        hour: seek.hour,
                        seek_sequence: seek.playback.map(|clock| clock.seek_sequence),
                    };
                    self.animation_seek |= seek.playback.is_none()
                        || self
                            .map_animation_target
                            .is_none_or(|old| animation_target.seeks_from(old));
                    self.map_animation_target = Some(animation_target);
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
            MapAction::UiScale { scale } => {
                self.ui_scale = map::ui_scale::Setting::from_stored(scale);
            }
            MapAction::Diagnostics { enabled } => {
                self.diagnostics = enabled;
                if let Some(dashboard) = &mut self.dashboard {
                    dashboard.set_diagnostics(enabled);
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
        // Built here on demand, since ordinary viewing no longer keeps them.
        let mut messages = self
            .document
            .as_ref()
            .map(|document| {
                let mut warnings = self.collect_warnings(document);
                warnings.extend(self.combat.warnings(document));
                warnings
            })
            .unwrap_or_default();
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
                        messages.push(match &*frame.tables[table.index()] {
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
                "Political region tables {:.2} ms",
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

    pub(super) fn report_embedded(&mut self, ctx: &egui::Context) {
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
}
