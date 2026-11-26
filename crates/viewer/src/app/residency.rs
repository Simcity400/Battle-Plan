use super::{MapPresented, Uploaded, Viewer, load_scene};
use eframe::egui;
use job_priority::SpawnCarried as _;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use viewer_world::scene::Scene;

pub(super) struct SceneBuild {
    content: Weak<crate::document::Content>,
    samples: u32,
    result: mpsc::Receiver<Result<Scene, String>>,
    alive: Arc<AtomicBool>,
}

impl Drop for SceneBuild {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wanted {
    Release,
    Keep,
    Build,
}

#[derive(Clone, Copy)]
struct Shown {
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

impl Viewer {
    fn campaign_shown(&self) -> bool {
        !self.dashboard_active
            && self
                .embedded
                .as_ref()
                .is_none_or(crate::embedded::Controller::visible)
    }

    pub(super) fn follow_shown_maps(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.embedded.is_none() || self.document.is_none() || self.loading.is_some() {
            self.scene_build = None;
            return;
        }
        let shown = Shown {
            campaign: self.campaign_shown(),
            widgets: self.dashboard.as_ref().is_some_and(|dashboard| dashboard.holds_views()),
            exporting: self.capture.as_ref().is_some_and(|capture| capture.export.is_some()),
        };
        if self.scene_build.is_some() {
            self.take_built_scene(ctx);
        }
        match shown.wanted(self.scene.is_some()) {
            Wanted::Release => self.release_scene(frame),
            Wanted::Keep => {}
            Wanted::Build if self.scene_build.is_none() => {
                if self.error.is_none() {
                    self.build_scene(ctx);
                } else if let Some(error) = self.error.clone().filter(|_| self.capture.is_some()) {
                    self.refuse_capture(&error, ctx);
                }
            }
            Wanted::Build => {}
        }
    }

    fn release_scene(&mut self, frame: &eframe::Frame) {
        let Some(mut scene) = self.scene.take() else {
            return;
        };
        if let Some(document) = &self.document {
            document.content.release_loading();
        }
        crate::perf_log::gpu_times(scene.retire_gpu_frame_times());
        self.free_scene_target(frame);
        self.close_playhead();
        self.forget_overlay_builds();
        self.rendered = None;
        self.uploaded = Uploaded::default();
        self.presented = MapPresented::default();
        self.shown_whole = false;
        let retired = (scene, std::mem::take(&mut self.faction_highlight));
        match &self.dropper {
            Some(dropper) => dropper.drop_later(retired),
            None => drop(retired),
        }
    }

    fn build_scene(&mut self, ctx: &egui::Context) {
        let Some(document) = &self.document else {
            return;
        };
        let Some((device, queue)) = self.gpu.clone() else {
            self.error = Some("The native graphics device is unavailable".into());
            return;
        };
        let content = Arc::clone(&document.content);
        let graphics = self.graphics;
        let workers = self
            .shared_pool
            .as_ref()
            .map_or_else(Default::default, |pool| pool.workers());
        let alive = Arc::new(AtomicBool::new(true));
        let job_alive = Arc::clone(&alive);
        let (sender, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let build = SceneBuild {
            content: Arc::downgrade(&content),
            samples: graphics.samples,
            result,
            alive,
        };
        let spawned = std::thread::Builder::new()
            .name("native map scene".into())
            .spawn_carried(move || {
                if !job_alive.load(Ordering::Acquire) {
                    return;
                }
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    load_scene(&device, &queue, &content, graphics, &workers)
                        .map_err(|error| error.to_string())
                }))
                .unwrap_or_else(|_| Err("Native map scene creation panicked".into()));
                if job_alive.load(Ordering::Acquire) {
                    let _ = sender.send(built);
                }
                wake.request_repaint();
            });
        match spawned {
            Ok(_) => self.scene_build = Some(build),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn take_built_scene(&mut self, ctx: &egui::Context) {
        let Some(build) = &self.scene_build else {
            return;
        };
        let built = match build.result.try_recv() {
            Ok(built) => built,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Native map scene builder stopped".into()),
        };
        let current = build.content.upgrade().zip(self.document.as_ref()).is_some_and(
            |(built_for, document)| Arc::ptr_eq(&built_for, &document.content),
        ) && build.samples == self.graphics.samples;
        self.scene_build = None;
        match built {
            Ok(scene) if current => {
                self.scene = Some(scene);
                self.rendered = None;
                self.uploaded = Uploaded::default();
                self.settled_poses = false;
                self.settled_capture = false;
            }
            Ok(scene) => match &self.dropper {
                Some(dropper) => dropper.drop_later(scene),
                None => drop(scene),
            },
            Err(error) if current => {
                self.refuse_capture(&error, ctx);
                self.error = Some(error);
            }
            Err(_) => {}
        }
    }

    pub(super) fn building_scene(&self) -> bool {
        self.scene_build.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{Shown, Wanted};

    fn shown(campaign: bool, widgets: bool, exporting: bool) -> Shown {
        Shown { campaign, widgets, exporting }
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
}
