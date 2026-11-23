use job_priority::SpawnCarried as _;
use super::{Capture, HeldClock, Playhead, Viewer, VisualStale, seek_open_target};
use eframe::{egui, wgpu};
use replay_data::rows::TableId;
use replay_frame::frame::TableView;
use std::collections::HashSet;
use std::path::Path;
use std::{path::PathBuf, sync::Arc, time::Instant};
use viewer_runtime::presentation::Target;
use viewer_gpu::ui_scale::Setting;

impl Viewer {
    pub(super) fn embedded_logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(embedded) = self.embedded.as_mut() else {
            return;
        };
        let commands = replay_frame::cpu::scope("frame.commands", Some("other"));
        let pending = embedded.poll(ctx);
        drop(commands);
        if pending.close {
            return;
        }
        if pending.campaign_map {
            self.dashboard_active = false;
            self.last_step = Instant::now();
            self.last_frame = Instant::now();
        }
        if let Some((clock_id, views)) = pending.map_views {
            self.dashboard_active = true;
            self.playing = false;
            if self.dashboard.is_none() {
                let service = embedded.service();
                let created = self
                    .shared_pool
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| std::io::Error::other("Native map workers are unavailable"))
                    .and_then(|pool| {
                        crate::dashboard_maps::DashboardMaps::new(
                            self.config.clone(),
                            self.options.clone(),
                            service,
                            self.gpu.clone(),
                            pool,
                        )
                    });
                match created {
                    Ok(mut dashboard) => {
                        dashboard.set_diagnostics(self.diagnostics);
                        dashboard.set_muted(self.muted);
                        dashboard.share_fallbacks(Arc::clone(&self.visual_fallbacks));
                        self.embedded_open = None;
                        self.embedded_seek = None;
                        self.dashboard = Some(Box::new(dashboard));
                    }
                    Err(error) => {
                        self.refuse_exports(pending.exports, &format!("The map widgets could not start: {error}"));
                        self.error = Some(error.to_string());
                        self.report_embedded(ctx);
                        return;
                    }
                }
            }
            if let Some(dashboard) = &mut self.dashboard {
                dashboard.update_config(&self.config, frame, ctx);
                dashboard.set_views(clock_id.as_deref(), views, frame, ctx);
            }
        }
        let service = self
            .embedded
            .as_ref()
            .expect("embedded controller")
            .service();
        {
            let _other_cost = replay_frame::cpu::scope("frame.preferences", Some("other"));
            self.follow_campaign_preferences(&service, ctx);
        }
        if let Some(message) = pending.gamepad {
            self.input.gamepad(message);
            ctx.request_repaint();
        }
        {
            let _other_cost = replay_frame::cpu::scope("frame.visual-checks", Some("other"));
            self.follow_visual_checks(ctx, Some(&*frame));
        }
        let actions = pending.actions;
        for action in actions {
            self.host_map_action(action, ctx, frame);
        }
        for job in pending.exports {
            self.host_export(job, ctx, frame);
        }
        {
            let _other_cost = replay_frame::cpu::scope("frame.fly-aircraft", Some("other"));
            self.poll_fly_aircraft(ctx);
        }
        if self.dashboard_active {
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
                embedded.report_maps(dashboard.states(), ctx);
            }
            return;
        }
        if let Some(request) = pending.open {
            self.embedded_open = Some(request);
            self.embedded_seek = None;
        }
        if let Some(seek) = pending.seek {
            if !seek
                .playback
                .as_ref()
                .is_some_and(|clock| self.page_clock.is_retired(&clock.clock_id))
            {
                let accepted = self
                    .document
                    .as_ref()
                    .map(|document| (document.source_path.as_path(), self.revision.as_deref()));
                let pending_load = if self.loading.is_some() || self.awaiting_folders {
                    self.options
                        .replay
                        .as_deref()
                        .map(|path| (path, self.expected_revision.as_deref()))
                } else {
                    None
                };
                self.embedded_open = seek_open_target(&seek, accepted, pending_load);
                self.embedded_seek = Some(seek);
            }
        }
        {
            let _other_cost = replay_frame::cpu::scope("frame.load", Some("other"));
            self.poll(ctx);
        }
        if self.loading.is_none()
            && let Some(request) = self.embedded_open.take()
        {
            self.options.branch = None;
            self.options.hour = None;
            self.options.hour_fraction = 0.0;
            self.open(request.path, ctx, request.revision);
            if self.loading.is_none() && !self.awaiting_folders {
                self.embedded_seek = None;
            }
        }
        if self.loading.is_none()
            && !self.awaiting_folders
            && let Some(seek) = self.embedded_seek.take()
        {
            if seek
                .playback
                .as_ref()
                .is_some_and(|clock| self.page_clock.is_retired(&clock.clock_id))
            {
            } else {
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
                        let now = Instant::now();
                        let last = self
                            .document
                            .as_ref()
                            .expect("validated above")
                            .playback
                            .range(seek.branch)
                            .expect("validated above")
                            .1;
                        let playback = seek.playback.as_ref();
                        let animation_target = Target {
                            branch: seek.branch,
                            hour: seek.hour,
                            seek_sequence: playback.map(|clock| clock.seek_sequence),
                        };
                        let sought = playback.is_none()
                            || self
                                .map_animation_target
                                .is_none_or(|old| animation_target.seeks_from(old));
                        self.animation_seek |= sought;
                        self.map_animation_target = Some(animation_target);
                        let playing = playback.is_some_and(|clock| clock.playing);
                        let target = Playhead {
                            branch: seek.branch,
                            hour: seek.hour,
                            fraction: playback.map_or(0.0, |clock| clock.fraction),
                        };
                        let speed = playback
                            .map_or(self.speed, |clock| clock.hours_per_second);
                        let composed = playback.map(|clock| crate::page_clock::ClockSample {
                            origin: &clock.clock_id,
                            composed_at: clock.composed_at,
                        });
                        self.anchor_playback(target, playing, speed, sought, composed, now, last);
                    }
                    Err(error) => {
                        self.error = Some(error);
                    }
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
                if self.capture.is_none()
                    && let (Some(head), Some(document)) = (self.playhead, &self.document)
                    && let Some((_, last)) = document.playback.range(head.branch)
                {
                    self.tick_playhead(now.duration_since(self.last_step).as_secs_f64(), last);
                }
                self.last_step = now;
                self.last_frame = now;
                self.input.restart_clock();
                ctx.request_repaint();
            }
        } else if self.hidden_since.is_none() {
            self.hidden_since = Some(now);
            self.input.release();
            self.grab_pointer(ctx);
        }
        if let Some(error) = pending.error {
            self.error = Some(error);
        }
        self.report_embedded(ctx);
    }

    pub(super) fn follow_campaign_preferences(
        &mut self,
        service: &dashboard::Service,
        ctx: &egui::Context,
    ) {
        if let Some(source) = self.source.clone() {
            let replay = source.replay();
            let preferences =
                self.follow_map_settings(service, dashboard_core::map_settings::campaign_key(&replay));
            let selection = (
                dashboard_core::map_settings::campaign_key(&replay),
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
                    self.reload_campaign_map(source, ctx);
                }
            }
        }
    }

    pub(super) fn follow_visual_checks(
        &mut self,
        ctx: &egui::Context,
        frame: Option<&eframe::Frame>,
    ) {
        if let Some(stale) = self
            .visual_check
            .as_ref()
            .and_then(|check| check.try_recv().ok())
        {
            self.visual_check = None;
            let changed = stale.maps.len() + usize::from(stale.campaign.is_some());
            let mut note = format!(
                "Visual mods checked: {changed} of {} open map(s) changed since they loaded",
                stale.checked
            );
            if let Some(dashboard) = &mut self.dashboard
                && let Some(frame) = frame
            {
                dashboard.reload_stale(&stale.maps, frame);
            }
            if stale.campaign.is_some() {
                note.push_str("; the Campaign map reloads");
            }
            self.visual_stale_campaign = stale.campaign.or(self.visual_stale_campaign.take());
            if !stale.failures.is_empty() {
                note.push_str(&format!("; not read: {}", stale.failures.join("; ")));
            }
            self.visual_check_note = Some(format!("{note}."));
        }
        if self
            .visual_check_after_load
            .due(self.loading_any_map(), self.visual_check.is_some())
        {
            self.check_visual_mods(false, ctx);
        }
        if let Some(report) = self.take_visual_report() {
            crate::visual_fallback::lock(&self.visual_fallbacks).reduce(vec![report]);
        }
        if self.loading.is_none()
            && let Some(source) = self.source.clone()
            && self
                .document
                .as_ref()
                .and_then(|document| document.content.fallback.as_ref())
                .is_some_and(|built| {
                    !crate::visual_fallback::lock(&self.visual_fallbacks).current(built)
                })
        {
            self.reload_campaign_map(source, ctx);
        }
        if self.loading.is_none()
            && let Some(revision) = self.visual_stale_campaign.take()
            && self
                .document
                .as_ref()
                .is_some_and(|document| document.visual_revision == revision)
            && let Some(source) = self.source.clone()
        {
            self.stale_visual_revision = Some(revision);
            self.reload_campaign_map(source, ctx);
        }
    }

    fn refuse_exports(&mut self, jobs: Vec<crate::host::ExportJob>, why: &str) {
        for job in jobs {
            job.answer(Err(why.to_owned()));
        }
    }

    pub(crate) fn host_export(&mut self, job: crate::host::ExportJob, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(id) = job.view_id.clone() {
            match self.dashboard.as_mut() {
                Some(dashboard) => dashboard.export_view(&id, job, ctx, frame),
                None => job.answer(Err("The requested map widget is unavailable".into())),
            }
            return;
        }
        if self.document.is_none() || self.capture.is_some() {
            job.answer(Err("Open a map and wait for any active export to finish".into()));
            return;
        }
        self.capture = Some(Capture {
            path: PathBuf::from(&job.path),
            remaining: 0,
            pinned: None,
            exit: false,
            time: crate::suite_input::capture_clock(),
            step: 0.0,
            frames: 1,
            index: 0,
            paced: false,
            export: Some(job),
        });
    }

    pub(crate) fn host_map_action(
        &mut self,
        action: dashboard_core::protocol::MapAction,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
    ) {
        use dashboard_core::protocol::MapAction;
        match action {
            MapAction::Unload { path } => {
                if self.unload_recording(Path::new(&path)) {
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
            MapAction::FrameLimit { hz } => {
                viewer_runtime::frame_limit::limit_to(hz);
            }
            MapAction::UiScale { scale } => {
                self.ui_scale = Setting::from_stored(scale);
            }
            MapAction::Diagnostics { enabled } => {
                self.diagnostics = enabled;
                if let Some(dashboard) = &mut self.dashboard {
                    dashboard.set_diagnostics(enabled);
                }
            }
            MapAction::Mute { muted } => {
                self.muted = muted;
                self.apply_audio_mix();
                if let Some(dashboard) = &mut self.dashboard {
                    dashboard.set_muted(muted);
                }
            }
            MapAction::Configure { options } => {
                self.host_perspective(&options);
                self.apply_host_options(&options);
            }
            MapAction::Folders { game, user } => {
                self.config.game_dir = PathBuf::from(game);
                self.config.user_dir = PathBuf::from(user);
                self.awaiting_folders = self.config.resolve_options().is_err();
                self.error = None;
                if self.error.is_none()
                    && let Some(path) = self.options.replay.clone()
                {
                    self.open(path, ctx, self.expected_revision.clone());
                }
                self.report_map_details();
            }
            MapAction::MapSettingsSaved => self.check_visual_mods(true, ctx),
            MapAction::RefreshVisualMods => self.check_visual_mods(false, ctx),
            MapAction::Inspect => self.report_map_details(),
            MapAction::CameraMode { mode, overlays } => self.set_camera_mode(mode, overlays, ctx),
            MapAction::FlyAircraft { aircraft } => self.fly_aircraft.choose(aircraft),
        }
        ctx.request_repaint();
    }

    fn poll_fly_aircraft(&mut self, ctx: &egui::Context) {
        let repaint = ctx.clone();
        let assets = self
            .document
            .as_ref()
            .map(|document| &document.content.aircraft);
        if let Some(status) = self
            .fly_aircraft
            .poll(assets, move || repaint.request_repaint())
            && let Some(embedded) = &self.embedded
        {
            embedded.report_fly_aircraft(status);
        }
    }

    fn reload_campaign_map(
        &mut self,
        source: std::sync::Arc<dashboard_base::Source>,
        ctx: &egui::Context,
    ) {
        if let Some((branch, hour)) = self.document.as_ref().map(|document| self.hour_on_screen(document)) {
            self.options.branch = Some(branch);
            self.options.hour = Some(hour);
        }
        let clock = HeldClock {
            playhead: self.playhead,
            playing: self.playing,
            anchor: self.clock_anchor,
            at: self.last_step,
        };
        self.held_view = Some((
            source
                .path()
                .canonicalize()
                .unwrap_or_else(|_| source.path().to_path_buf()),
            clock,
        ));
        self.playing = false;
        self.open(
            source.path().to_path_buf(),
            ctx,
            Some(source.key().revision.clone()),
        );
        self.source = Some(source);
    }

    pub(super) fn check_visual_mods(&mut self, reload_settings: bool, ctx: &egui::Context) {
        let Some(service) = self.service() else {
            return;
        };
        let loading = self.loading_any_map();
        self.visual_check_after_load.asked(loading);
        let options = self.config.resolve_options().ok();
        let recorded_mods = |replay: &replay::campaign::Replay| {
            replay
                .identity
                .mods
                .iter()
                .map(|recorded| content_reader_layers::content::visual::RecordedMod {
                    id: recorded.id.clone(),
                    name: recorded.display_name.clone(),
                })
                .collect::<Vec<_>>()
        };
        let campaign = self.document.as_ref().and_then(|document| {
            let selected = document
                .visual_mods
                .clone()
                .filter(|mods| !mods.is_empty())?;
            Some((
                selected,
                recorded_mods(&document.playback.replay_handle()),
                document.visual_revision.clone(),
            ))
        });
        let maps = self
            .dashboard
            .as_ref()
            .map(|dashboard| dashboard.visual_checks())
            .unwrap_or_default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("visual mod check".into())
            .spawn_carried(move || {
                let _stage = viewer_runtime::load_record::stage("visual_check");
                if reload_settings {
                    service.reload_map_settings();
                }
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let failures = std::cell::RefCell::new(Vec::new());
                let changed = |selected: &[content_reader_layers::content::visual::Choice],
                               recorded: &[content_reader_layers::content::visual::RecordedMod],
                               revision: &str| {
                    let Some(options) = &options else {
                        failures
                            .borrow_mut()
                            .push("the content folders are not set".to_owned());
                        return false;
                    };
                    let stage = viewer_runtime::load_record::stage("visual_check.revision");
                    let now = content_reader_layers::content::visual::revision(
                        options, selected, recorded, &cancel,
                    );
                    drop(stage);
                    match now {
                        Ok(now) => now != revision,
                        Err(error) => {
                            failures.borrow_mut().push(error.to_string());
                            false
                        }
                    }
                };
                let checked = maps.len() + usize::from(campaign.is_some());
                let campaign = campaign
                    .filter(|(selected, recorded, revision)| changed(selected, recorded, revision))
                    .map(|(_, _, revision)| revision);
                let maps = maps
                    .into_iter()
                    .filter(|(_, selected, recorded, revision)| {
                        changed(selected, recorded, revision)
                    })
                    .map(|(key, _, _, revision)| (key, revision))
                    .collect();
                let stale = VisualStale {
                    campaign,
                    maps,
                    checked,
                    failures: failures.into_inner(),
                };
                let _ = sender.send(stale);
                repaint.request_repaint();
            });
        match spawned {
            Ok(_) => self.visual_check = Some(receiver),
            Err(error) => self.error = Some(format!("Visual mods: {error}")),
        }
    }

    fn loading_any_map(&self) -> bool {
        self.loading.is_some()
            || self
                .dashboard
                .as_ref()
                .is_some_and(|dashboard| dashboard.loading_content())
    }

    fn report_map_details(&self) {
        let Some(embedded) = &self.embedded else {
            return;
        };
        let mut messages = self
            .document
            .as_ref()
            .map(|document| self.collect_warnings(document))
            .unwrap_or_default();
        messages.extend(self.error.iter().cloned());
        messages.extend(self.map_error.iter().cloned());
        messages.extend(self.capture_note.iter().cloned());
        if let Some(document) = &self.document {
            messages.extend(document.warnings.iter().cloned());
            match document.playback.current() {
                _ if self.playhead_map.is_some() => messages.extend(
                    self.playhead_map
                        .iter()
                        .flat_map(|side| side.with_side(crate::playhead_host::details::of_map)),
                ),
                viewer_playback::map_source::Current::Gap { reason, .. } => {
                    messages.push(format!("Gap: {reason}"))
                }
                viewer_playback::map_source::Current::Refused { reason, .. } => {
                    messages.push(format!("Refused: {reason}"))
                }
                viewer_playback::map_source::Current::Frame(frame) => {
                    for table in TableId::ALL {
                        messages.push(match &*frame.tables[table.index()] {
                            TableView::Complete(rows) => format!("{table:?}: {} rows", rows.len()),
                            TableView::Truncated { missing, .. } => {
                                format!("{table:?}: truncated, {missing} missing")
                            }
                            TableView::Unavailable { reason } => {
                                format!("{table:?}: unavailable ({reason})")
                            }
                            TableView::NotRecorded { mode } => {
                                format!("{table:?}: not recorded in a {mode} recording")
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
        messages.extend(self.visual_check_note.iter().cloned());
        let mut seen = HashSet::new();
        messages.retain(|message| seen.insert(message.clone()));
        embedded.report_details(dashboard_core::protocol::MapDetails {
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
        let shown = self.rendered.map(|head| (head.branch, head.hour)).or_else(|| self.layers_on_screen());
        let Some(embedded) = self.embedded.as_mut() else {
            return;
        };
        embedded.report(
            dashboard_core::protocol::ViewerState {
                path: self
                    .document
                    .as_ref()
                    .map(|document| document.source_path.to_string_lossy().into_owned()),
                branch: shown.map(|(branch, _)| branch.to_string()),
                hour: shown.map(|(_, hour)| hour),
                revision: self.document.as_ref().and(self.revision.clone()),
            },
            self.error.as_deref().or(self.map_error.as_deref()),
            ctx,
        );
    }
}

#[derive(Debug, Default)]
pub(crate) struct DeferredCheck(bool);

impl DeferredCheck {
    pub(crate) fn asked(&mut self, loading: bool) {
        self.0 |= loading;
    }

    pub(crate) fn due(&mut self, loading: bool, checking: bool) -> bool {
        let due = self.0 && !loading && !checking;
        if due {
            self.0 = false;
        }
        due
    }
}

#[cfg(test)]
mod deferred_check_tests {
    use super::DeferredCheck;

    #[test]
    fn a_refresh_during_a_load_checks_the_installed_map_again() {
        let mut deferred = DeferredCheck::default();
        deferred.asked(true);
        assert!(!deferred.due(true, false));
        assert!(!deferred.due(false, true));
        assert!(deferred.due(false, false));
        assert!(!deferred.due(false, false));
        deferred.asked(false);
        assert!(!deferred.due(false, false));
    }
}
