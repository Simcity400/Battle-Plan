use super::{MapPresented, Uploaded, Viewer, load_scene};
use eframe::egui;
use job_priority::SpawnCarried as _;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use viewer_world::scene::Scene;

pub(super) struct PendingScene<C, S> {
    content: Weak<C>,
    samples: u32,
    result: mpsc::Receiver<Result<S, String>>,
    keep: Arc<AtomicBool>,
    was_hidden: bool,
}

impl<C, S> PendingScene<C, S> {
    fn matches(&self, content: &Arc<C>, samples: u32) -> bool {
        self.content
            .upgrade()
            .is_some_and(|built| Arc::ptr_eq(&built, content))
            && self.samples == samples
    }
}

impl<C, S> Drop for PendingScene<C, S> {
    fn drop(&mut self) {
        self.keep.store(false, Ordering::Release);
    }
}

pub(super) type SceneBuild = PendingScene<crate::document::Content, Scene>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wanted {
    Release,
    Keep,
    Build,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    campaign: bool,
    widgets: bool,
    exporting: bool,
}

impl Shown {
    fn wanted(self, built: bool) -> Wanted {
        let drawn_here = self.campaign || self.exporting;
        if !drawn_here && !self.widgets {
            Wanted::Release
        } else if built || !drawn_here {
            Wanted::Keep
        } else {
            Wanted::Build
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Action<S> {
    Idle,
    Release,
    Start,
    RefuseCapture,
    Install(S),
    Fail(String),
}

struct Inputs<'a, C> {
    content: Option<&'a Arc<C>>,
    loading_document: bool,
    fallback_current: bool,
    has_scene_error: bool,
    has_capture: bool,
    samples: u32,
    shown: Shown,
    has_scene: bool,
}

fn cancel<C, S>(
    build: &mut Option<PendingScene<C, S>>,
    retire: &mut impl FnMut(PendingScene<C, S>),
) {
    if let Some(pending) = build.take() {
        pending.keep.store(false, Ordering::Release);
        retire(pending);
    }
}

fn reconcile<C, S>(
    inputs: Inputs<'_, C>,
    build: &mut Option<PendingScene<C, S>>,
    retire_build: &mut impl FnMut(PendingScene<C, S>),
    retire_scene: &mut impl FnMut(S),
) -> Action<S> {
    let wanted = inputs.shown.wanted(inputs.has_scene);
    if inputs.loading_document {
        cancel(build, retire_build);
        return if wanted == Wanted::Release {
            Action::Release
        } else {
            Action::Idle
        };
    }
    let Some(content) = inputs.content else {
        cancel(build, retire_build);
        return if wanted == Wanted::Release {
            Action::Release
        } else {
            Action::Idle
        };
    };
    if build
        .as_ref()
        .is_some_and(|pending| !pending.matches(content, inputs.samples))
    {
        cancel(build, retire_build);
    }
    match wanted {
        Wanted::Release | Wanted::Keep => {
            if let Some(pending) = build.as_mut() {
                pending.keep.store(false, Ordering::Release);
                pending.was_hidden = true;
                match pending.result.try_recv() {
                    Ok(Ok(scene)) => {
                        *build = None;
                        retire_scene(scene);
                    }
                    Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                        *build = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if wanted == Wanted::Release {
                Action::Release
            } else {
                Action::Idle
            }
        }
        Wanted::Build => {
            if let Some(pending) = build.as_mut() {
                pending.keep.store(true, Ordering::Release);
                match pending.result.try_recv() {
                    Ok(Ok(scene)) => {
                        *build = None;
                        return Action::Install(scene);
                    }
                    Ok(Err(error)) => {
                        *build = None;
                        return Action::Fail(error);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        let was_hidden = pending.was_hidden;
                        *build = None;
                        if !was_hidden {
                            return Action::Fail("Native map scene builder stopped".into());
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => return Action::Idle,
                }
            }
            if !inputs.fallback_current {
                Action::Idle
            } else if !inputs.has_scene_error {
                Action::Start
            } else if inputs.has_capture {
                Action::RefuseCapture
            } else {
                Action::Idle
            }
        }
    }
}

impl Viewer {
    pub(super) fn shown_maps(&self) -> Shown {
        let host_visible = self
            .embedded
            .as_ref()
            .is_none_or(crate::embedded::Controller::visible);
        Shown {
            campaign: !self.dashboard_active && host_visible,
            widgets: self.dashboard_active
                && host_visible
                && self
                    .dashboard
                    .as_deref()
                    .is_some_and(crate::dashboard_maps::DashboardMaps::holds_views),
            exporting: self
                .capture
                .as_ref()
                .is_some_and(|capture| capture.export.is_some()),
        }
    }

    pub(super) fn document_fallback_current(&self) -> bool {
        if self.source.is_none() {
            return true;
        }
        self.document
            .as_ref()
            .and_then(|document| document.content.fallback.as_ref())
            .is_none_or(|built| crate::visual_fallback::lock(&self.visual_fallbacks).current(built))
    }

    pub(super) fn cancel_scene_build(&mut self) {
        let dropper = self.dropper.clone();
        cancel(&mut self.scene_build, &mut |build| match &dropper {
            Some(dropper) => dropper.drop_later(build),
            None => drop(build),
        });
    }

    pub(super) fn follow_shown_maps(&mut self, ctx: &egui::Context, frame: Option<&eframe::Frame>) {
        let shown = self.shown_maps();
        let fallback_current = self.document_fallback_current();
        let dropper = self.dropper.clone();
        let action = reconcile(
            Inputs {
                content: self.document.as_ref().map(|document| &document.content),
                loading_document: self.loading.is_some(),
                fallback_current,
                has_scene_error: self.scene_error.is_some(),
                has_capture: self.capture.is_some(),
                samples: self.graphics.samples,
                shown,
                has_scene: self.scene.is_some(),
            },
            &mut self.scene_build,
            &mut |build| match &dropper {
                Some(dropper) => dropper.drop_later(build),
                None => drop(build),
            },
            &mut |scene| match &dropper {
                Some(dropper) => dropper.drop_later(scene),
                None => drop(scene),
            },
        );
        match action {
            Action::Idle => {}
            Action::Release => self.release_scene(frame),
            Action::Start => self.build_scene(ctx),
            Action::RefuseCapture => {
                if let Some(error) = self.scene_error.clone() {
                    self.refuse_capture(&error, ctx);
                }
            }
            Action::Install(scene) => {
                self.scene = Some(scene);
                self.scene_error = None;
                self.rendered = None;
                self.uploaded = Uploaded::default();
                self.settled_poses = false;
                self.settled_capture = false;
                if let Some(opened) = self.opened.take()
                    && super::profiling()
                {
                    super::profile_print(format!(
                        "PROFILE open document_and_scene {:.1} ms",
                        opened.elapsed().as_secs_f64() * 1000.0
                    ));
                }
            }
            Action::Fail(error) => {
                let fallback = self.source.as_ref().and_then(|_| {
                    self.document
                        .as_ref()
                        .and_then(|document| document.content.fallback.clone())
                        .filter(|built| {
                            matches!(built.under, crate::visual_fallback::Under::Mods { .. })
                        })
                });
                if let Some(built) = fallback
                    && !crate::visual_fallback::lock(&self.visual_fallbacks)
                        .reduce(vec![crate::visual_fallback::Report {
                            built,
                            refusals: [format!("scene: {error}")].into(),
                            settled: false,
                        }])
                        .is_empty()
                {
                    ctx.request_repaint();
                } else {
                    self.scene_error = Some(error.clone());
                    self.refuse_capture(&error, ctx);
                }
            }
        }
    }

    fn release_scene(&mut self, frame: Option<&eframe::Frame>) {
        if let Some(frame) = frame {
            self.free_scene_target(frame);
        }
        if self.dashboard_active
            && !self.popout_open
            && self.dashboard.as_deref().is_none_or(|maps| !maps.holds_views())
            && let Some(document) = &self.document
            && let Some(display) = document.products.release_display()
        {
            self.retire(display);
        }
        let Some(mut scene) = self.scene.take() else {
            return;
        };
        if let Some(document) = &self.document {
            document.content.release_loading();
        }
        crate::perf_log::gpu_times(scene.retire_gpu_frame_times());
        self.close_playhead();
        self.forget_overlay_builds();
        self.rendered = None;
        self.uploaded = Uploaded::default();
        self.presented = MapPresented::default();
        self.shown_whole = false;
        let retired = (scene, std::mem::take(&mut self.faction_highlight));
        self.retire(retired);
    }

    fn build_scene(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.document else {
            return;
        };
        let Some((device, queue)) = self.gpu.clone() else {
            let error = "The native graphics device is unavailable".to_owned();
            self.scene_error = Some(error.clone());
            self.refuse_capture(&error, ctx);
            return;
        };
        let content = Arc::clone(&document.content);
        let graphics = self.graphics;
        let workers = self
            .shared_pool
            .as_ref()
            .map_or_else(Default::default, |pool| pool.workers());
        let keep = Arc::new(AtomicBool::new(true));
        let job_keep = Arc::clone(&keep);
        let (sender, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let build = SceneBuild {
            content: Arc::downgrade(&content),
            samples: graphics.samples,
            result,
            keep,
            was_hidden: false,
        };
        let spawned = std::thread::Builder::new()
            .name("native map scene".into())
            .spawn_carried(move || {
                if !job_keep.load(Ordering::Acquire) {
                    return;
                }
                let _stage = viewer_runtime::load_record::stage("load.scene");
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    load_scene(&device, &queue, &content, graphics, &workers)
                        .map_err(|error| error.to_string())
                }))
                .unwrap_or_else(|_| Err("Native map scene creation panicked".into()));
                if job_keep.load(Ordering::Acquire) {
                    let _ = sender.send(built);
                }
                wake.request_repaint();
            });
        match spawned {
            Ok(_) => self.scene_build = Some(build),
            Err(error) => {
                let error = error.to_string();
                self.scene_error = Some(error.clone());
                self.refuse_capture(&error, ctx);
            }
        }
    }

    pub(super) fn building_scene(&self) -> bool {
        self.scene_build.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Inputs, PendingScene, Shown, Wanted, reconcile};
    use std::cell::Cell;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    fn shown(campaign: bool, widgets: bool, exporting: bool) -> Shown {
        Shown { campaign, widgets, exporting }
    }

    fn pending(
        content: &Arc<u8>,
        samples: u32,
    ) -> (PendingScene<u8, u32>, mpsc::SyncSender<Result<u32, String>>, Arc<AtomicBool>) {
        let (sender, result) = mpsc::sync_channel(1);
        let keep = Arc::new(AtomicBool::new(true));
        (
            PendingScene {
                content: Arc::downgrade(content),
                samples,
                result,
                keep: Arc::clone(&keep),
                was_hidden: false,
            },
            sender,
            keep,
        )
    }

    #[test]
    fn a_host_keeps_a_scene_only_while_it_shows_a_map() {
        for built in [true, false] {
            assert_eq!(shown(false, false, false).wanted(built), Wanted::Release, "no map on screen");
            assert_eq!(shown(false, true, false).wanted(built), Wanted::Keep, "a map widget is on screen; the hidden campaign map neither goes nor builds");
        }
        assert_eq!(shown(true, false, false).wanted(false), Wanted::Build, "the campaign map is shown again");
        assert_eq!(shown(true, true, false).wanted(true), Wanted::Keep);
        assert_eq!(shown(false, false, true).wanted(false), Wanted::Build, "an export draws the campaign map while it is hidden");
        assert_eq!(shown(false, false, true).wanted(true), Wanted::Keep);
    }

    #[test]
    fn a_hidden_map_never_builds_and_opening_it_later_builds_once_from_shared_content() {
        let content = Arc::new(1u8);
        let mut build = None;
        let retired_builds = Cell::new(0usize);
        let retired_scenes = Cell::new(0usize);
        let step = |shown, has_scene, build: &mut Option<PendingScene<u8, u32>>| {
            reconcile(
                Inputs {
                    content: Some(&content),
                    loading_document: false,
                    fallback_current: true,
                    has_scene_error: false,
                    has_capture: false,
                    samples: 4,
                    shown,
                    has_scene,
                },
                build,
                &mut |_| retired_builds.set(retired_builds.get() + 1),
                &mut |_| retired_scenes.set(retired_scenes.get() + 1),
            )
        };

        assert_eq!(step(shown(false, true, false), false, &mut build), Action::Idle);
        assert!(build.is_none());
        assert_eq!(step(shown(false, false, false), false, &mut build), Action::Release);
        assert!(build.is_none());

        assert_eq!(step(shown(true, false, false), false, &mut build), Action::Start);
        let (started, sender, keep) = pending(&content, 4);
        build = Some(started);
        assert_eq!(step(shown(true, false, false), false, &mut build), Action::Idle);
        assert!(keep.load(Ordering::Acquire));

        sender.send(Ok(7)).unwrap();
        assert_eq!(step(shown(true, false, false), false, &mut build), Action::Install(7));
        assert!(build.is_none());
        assert_eq!(step(shown(true, false, false), true, &mut build), Action::Idle);
        assert_eq!(retired_builds.get(), 0);
        assert_eq!(retired_scenes.get(), 0);
    }

    #[test]
    fn hide_then_show_reuses_a_running_build_retires_hidden_results_and_releases_when_hidden() {
        let content = Arc::new(1u8);
        let retired_builds = Cell::new(0usize);
        let retired_scenes = Cell::new(0usize);
        let step = |shown, has_scene, build: &mut Option<PendingScene<u8, u32>>| {
            reconcile(
                Inputs {
                    content: Some(&content),
                    loading_document: false,
                    fallback_current: true,
                    has_scene_error: false,
                    has_capture: false,
                    samples: 4,
                    shown,
                    has_scene,
                },
                build,
                &mut |_| retired_builds.set(retired_builds.get() + 1),
                &mut |_| retired_scenes.set(retired_scenes.get() + 1),
            )
        };

        let (started, sender, keep) = pending(&content, 4);
        let mut build = Some(started);
        assert_eq!(step(shown(false, true, false), false, &mut build), Action::Idle);
        assert!(build.is_some());
        assert!(!keep.load(Ordering::Acquire));
        assert_eq!(step(shown(true, false, false), false, &mut build), Action::Idle);
        assert!(build.is_some());
        assert!(keep.load(Ordering::Acquire));
        sender.send(Ok(9)).unwrap();
        assert_eq!(step(shown(true, false, false), false, &mut build), Action::Install(9));
        assert!(build.is_none());
        assert_eq!(retired_builds.get(), 0);
        assert_eq!(retired_scenes.get(), 0);

        let (started, sender, keep) = pending(&content, 4);
        build = Some(started);
        sender.send(Ok(10)).unwrap();
        assert_eq!(step(shown(false, true, false), false, &mut build), Action::Idle);
        assert!(build.is_none());
        assert!(!keep.load(Ordering::Acquire));
        assert_eq!(retired_scenes.get(), 1);

        let (started, sender, keep) = pending(&content, 4);
        build = Some(started);
        assert_eq!(step(shown(false, false, false), false, &mut build), Action::Release);
        assert!(build.is_some());
        assert!(!keep.load(Ordering::Acquire));
        drop(sender);
        assert_eq!(step(shown(true, false, false), false, &mut build), Action::Start);
        assert!(build.is_none());

        let (started, sender, _keep) = pending(&content, 4);
        build = Some(started);
        drop(sender);
        assert_eq!(
            step(shown(true, false, false), false, &mut build),
            Action::Fail("Native map scene builder stopped".into())
        );

        assert_eq!(step(shown(false, false, false), true, &mut build), Action::Release);
        assert_eq!(step(shown(true, false, false), false, &mut build), Action::Start);
    }

    #[test]
    fn closing_or_changing_inputs_during_a_build_cancels_it_and_never_installs_stale_scenes() {
        let first = Arc::new(1u8);
        let second = Arc::new(2u8);
        let mut retired = 0usize;
        let mut retired_scenes = 0usize;

        let (started, _sender, keep) = pending(&first, 4);
        let mut build = Some(started);
        assert_eq!(
            reconcile(
                Inputs {
                    content: None,
                    loading_document: false,
                    fallback_current: true,
                    has_scene_error: false,
                    has_capture: false,
                    samples: 4,
                    shown: shown(true, false, false),
                    has_scene: false,
                },
                &mut build,
                &mut |_| retired += 1,
                &mut |_| retired_scenes += 1,
            ),
            Action::Idle
        );
        assert!(build.is_none());
        assert!(!keep.load(Ordering::Acquire));
        assert_eq!(retired, 1);

        let (started, _sender, keep) = pending(&first, 4);
        let mut build = Some(started);
        assert_eq!(
            reconcile(
                Inputs {
                    content: Some(&first),
                    loading_document: true,
                    fallback_current: true,
                    has_scene_error: false,
                    has_capture: false,
                    samples: 4,
                    shown: shown(false, false, false),
                    has_scene: false,
                },
                &mut build,
                &mut |_| retired += 1,
                &mut |_| retired_scenes += 1,
            ),
            Action::Release
        );
        assert!(build.is_none());
        assert!(!keep.load(Ordering::Acquire));
        assert_eq!(retired, 2);

        let (started, _sender, keep) = pending(&first, 4);
        let mut build = Some(started);
        assert_eq!(
            reconcile(
                Inputs {
                    content: Some(&second),
                    loading_document: false,
                    fallback_current: true,
                    has_scene_error: false,
                    has_capture: false,
                    samples: 4,
                    shown: shown(true, false, false),
                    has_scene: false,
                },
                &mut build,
                &mut |_| retired += 1,
                &mut |_| retired_scenes += 1,
            ),
            Action::Start
        );
        assert!(build.is_none());
        assert!(!keep.load(Ordering::Acquire));
        assert_eq!(retired, 3);

        let (started, _sender, keep) = pending(&second, 4);
        let mut build = Some(started);
        assert_eq!(
            reconcile(
                Inputs {
                    content: Some(&second),
                    loading_document: false,
                    fallback_current: true,
                    has_scene_error: false,
                    has_capture: false,
                    samples: 2,
                    shown: shown(true, false, false),
                    has_scene: false,
                },
                &mut build,
                &mut |_| retired += 1,
                &mut |_| retired_scenes += 1,
            ),
            Action::Start
        );
        assert!(build.is_none());
        assert!(!keep.load(Ordering::Acquire));
        assert_eq!(retired, 4);

        assert_eq!(
            reconcile(
                Inputs {
                    content: Some(&second),
                    loading_document: false,
                    fallback_current: false,
                    has_scene_error: false,
                    has_capture: false,
                    samples: 2,
                    shown: shown(true, false, false),
                    has_scene: false,
                },
                &mut build,
                &mut |_| retired += 1,
                &mut |_| retired_scenes += 1,
            ),
            Action::Idle
        );

        assert_eq!(
            reconcile(
                Inputs {
                    content: Some(&second),
                    loading_document: false,
                    fallback_current: true,
                    has_scene_error: true,
                    has_capture: true,
                    samples: 2,
                    shown: shown(true, false, false),
                    has_scene: false,
                },
                &mut build,
                &mut |_| retired += 1,
                &mut |_| retired_scenes += 1,
            ),
            Action::RefuseCapture
        );
        assert_eq!(retired_scenes, 0);
    }

    #[test]
    fn unloading_clears_scene_build_error_and_playhead_while_seek_errors_do_not_block_scene_builds() {
        let path = std::path::PathBuf::from("unloaded-during-build.battleplan");
        let mut viewer = super::Viewer::create(
            crate::Config::default(),
            crate::Options {
                replay: Some(path.clone()),
                ..Default::default()
            },
            None,
            Arc::new(std::sync::Mutex::new(None)),
            None,
            None,
        );
        assert_eq!(viewer.shown_maps(), shown(true, false, false));
        viewer.dashboard_active = true;
        assert_eq!(viewer.shown_maps(), shown(false, false, false));

        viewer.error = Some("Hour 99 is outside branch 0".into());
        assert!(viewer.scene_error.is_none());
        assert_eq!(viewer.reported_error(), Some("Hour 99 is outside branch 0"));

        viewer.scene_error = Some("scene build failed".into());
        viewer.error = None;
        assert_eq!(viewer.reported_error(), Some("scene build failed"));

        let mut settings = dashboard_core::protocol::ResolvedMapSettings::default();
        settings.graphics.samples = viewer.graphics.samples.wrapping_add(1).max(1);
        viewer.set_embedded_graphics(&settings);
        assert!(viewer.scene_error.is_none());
        assert_eq!(viewer.reported_error(), None);

        viewer.scene_error = Some("scene build failed".into());
        viewer.retry_map();
        assert!(viewer.scene_error.is_none());
        assert_eq!(viewer.reported_error(), None);

        let (_sender, result) = mpsc::sync_channel(1);
        let keep = Arc::new(AtomicBool::new(true));
        viewer.scene_build = Some(super::SceneBuild {
            content: std::sync::Weak::new(),
            samples: 4,
            result,
            keep: Arc::clone(&keep),
            was_hidden: false,
        });
        viewer.scene_error = Some("stale scene failure".into());
        viewer.playhead_opened = true;
        assert!(viewer.building_scene());
        assert!(viewer.unload_recording(&path));
        assert!(!viewer.building_scene());
        assert!(viewer.scene_error.is_none());
        assert!(!viewer.playhead_opened);
        assert!(!keep.load(Ordering::Acquire));
    }
}
