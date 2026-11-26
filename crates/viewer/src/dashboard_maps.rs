
use job_priority::SpawnCarried as _;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use viewer_gpu::gpu_contract::SOFTWARE_RENDERER_NOTICE;
use viewer_gpu::gpu_contract::is_software;
use viewer_gpu::ui_scale::Setting;
use viewer_runtime::worker_priority::background_priority;

use dashboard_core::protocol::{DashboardMapState, DashboardMapView, ViewBounds};
use dashboard::Service;
use dashboard_base::{Source, SourceKey};
use eframe::{egui, wgpu};

use crate::{
    Config, Options,
    app::{Viewer, date, load_scene, map_overlay, partial_notice},
    document::{Content, Document, InitialHour},
};
use viewer_world::scene::Scene;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MapSourceKey {
    recording: SourceKey,
    texture_quality: u8,
    visual_mods: Option<Vec<content_reader_layers::content::visual::Choice>>,
}

struct MapSource {
    texture_quality: u8,
    visual_mods: Option<Vec<content_reader_layers::content::visual::Choice>>,
    source: Arc<Source>,
    content: Option<Arc<Content>>,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RequestedSource {
    path: PathBuf,
    revision: String,
}

impl RequestedSource {
    fn target(target: &DashboardMapView) -> Self {
        Self {
            path: target.path.clone().into(),
            revision: target.revision.clone(),
        }
    }

    fn open(&self, service: &Service) -> Result<Arc<Source>, String> {
        service.open_map(&self.path, Some(&self.revision))
    }
}

struct OpeningSource {
    requested: RequestedSource,
    result: mpsc::Receiver<Result<Arc<Source>, String>>,
    alive: Arc<AtomicBool>,
}

impl Drop for OpeningSource {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

struct MapView {
    target: DashboardMapView,
    source: Option<MapSourceKey>,
    viewer: Box<Viewer>,
    loading: Option<SceneLoad>,
    error: Option<String>,
    admission_error: Option<String>,
}

impl MapView {
    fn retire(&mut self, frame: &eframe::Frame) {
        self.viewer.export_closed("its map widget let its map go before its picture was saved");
        let load = self.loading.take();
        if let Some(load) = &load {
            load.alive.store(false, Ordering::Release);
        }
        if self.viewer.has_map() || load.is_some() {
            self.viewer.retire_map(frame, load);
        }
    }

    fn state(&self) -> DashboardMapState {
        let error = self.admission_error.as_ref().or(self.error.as_ref());
        if error.is_none() && self.viewer.has_map() {
            let tables = self.viewer.tables();
            return engine_executable::executable_tables::scope(tables, || {
                self.viewer.map_state(&self.target)
            });
        }
        DashboardMapState {
            id: self.target.id.clone(),
            clock_id: self.target.clock_id.clone(),
            source_id: Some(self.target.source_id.clone()),
            revision: Some(self.target.revision.clone()),
            requested_sequence: self.target.sequence,
            loading: error.is_none(),
            error: error.cloned(),
            ..Default::default()
        }
    }
}

struct LoadingSource {
    texture_quality: u8,
    key: MapSourceKey,
    config: Config,
    visual_mods: Option<Vec<content_reader_layers::content::visual::Choice>>,
    result: mpsc::Receiver<Result<Arc<Content>, String>>,
    alive: Arc<AtomicBool>,
}

impl Drop for LoadingSource {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

struct SceneLoad {
    samples: u32,
    content: std::sync::Weak<Content>,
    result: mpsc::Receiver<Result<(Document, Scene), String>>,
    alive: Arc<AtomicBool>,
}

impl Drop for SceneLoad {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

pub struct DashboardMaps {
    pub(crate) ui_scale: Setting,
    config: Config,
    options: Options,
    service: Arc<Service>,
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    pool: Arc<crate::worker_pool::WorkerPool>,
    sources: BTreeMap<MapSourceKey, MapSource>,
    source_bindings: BTreeMap<RequestedSource, Result<Arc<Source>, String>>,
    opening_source: Option<OpeningSource>,
    views: BTreeMap<String, MapView>,
    order: Vec<String>,
    loading: Option<LoadingSource>,
    fallbacks: crate::visual_fallback::Shared,
    visual_reports: Vec<crate::visual_fallback::Report>,
    limits: MapLimits,
    waiting_source: Option<PathBuf>,
    seeded: Option<(SourceKey, std::sync::Weak<Content>, Config)>,
    diagnostics: bool,
    muted: bool,
    audio_owner: Option<String>,
    view_events: Vec<(String, crate::app::picker::ViewEvent)>,
    clock_origins: crate::page_clock::OriginGate,
}

struct MapLimits {
    dimension: u32,
}

impl MapLimits {
    fn new(gpu: &Option<(wgpu::Device, wgpu::Queue)>) -> Self {
        Self {
            dimension: gpu
                .as_ref()
                .map_or(0, |(device, _)| device.limits().max_texture_dimension_2d),
        }
    }
}

impl DashboardMaps {
    pub(crate) fn export_view(
        &mut self,
        id: &str,
        job: crate::host::ExportJob,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
    ) {
        let Some(view) = self.views.get_mut(id) else {
            return job.answer(Err("The requested map widget is unavailable".into()));
        };
        if !view.viewer.has_map() {
            return job.answer(Err("Wait for the map widget to finish loading before exporting".into()));
        }
        let tables = view.viewer.tables();
        let mut job = job;
        job.view_id = None;
        engine_executable::executable_tables::scope(tables, || view.viewer.host_export(job, ctx, frame));
    }

    pub(crate) fn snapshot_view(&mut self, id: &str, job: crate::host::SnapshotJob) {
        let Some(view) = self.views.get(id) else {
            return job.answer(Err("The requested map widget is unavailable".into()));
        };
        if !view.viewer.has_map() {
            return job.answer(Err("Wait for the map widget to finish loading before exporting".into()));
        }
        job.answer(view.viewer.build_export_snapshot());
    }

    pub fn new(
        config: Config,
        options: Options,
        service: Arc<Service>,
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
        pool: Arc<crate::worker_pool::WorkerPool>,
    ) -> io::Result<Self> {
        let limits = MapLimits::new(&gpu);
        Ok(Self {
            ui_scale: Default::default(),
            config,
            options,
            service,
            gpu,
            pool,
            sources: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
            opening_source: None,
            views: BTreeMap::new(),
            order: Vec::new(),
            loading: None,
            fallbacks: Default::default(),
            visual_reports: Vec::new(),
            limits,
            waiting_source: None,
            seeded: None,
            diagnostics: false,
            muted: false,
            audio_owner: None,
            view_events: Vec::new(),
            clock_origins: Default::default(),
        })
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        for view in self.views.values_mut() {
            view.viewer.muted = muted;
            view.viewer.apply_audio_mix();
        }
    }

    pub fn release_audio(&mut self) {
        for view in self.views.values_mut() {
            view.viewer.release_audio();
        }
    }

    pub(crate) fn holds_views(&self) -> bool {
        !self.views.is_empty()
    }

    pub fn holds_export(&self) -> bool {
        self.views.values().any(|view| view.viewer.holds_export())
    }

    pub fn not_drawn(&mut self) {
        for view in self.views.values_mut() {
            view.viewer.export_closed("its map widget was not drawn before its picture was saved");
            view.viewer.close_playhead();
        }
    }

    pub fn set_diagnostics(&mut self, enabled: bool) {
        self.diagnostics = enabled;
        for view in self.views.values_mut() {
            view.viewer.diagnostics = enabled;
        }
    }

    pub fn wait_for_source(&mut self, path: Option<PathBuf>) {
        self.waiting_source = path;
    }

    pub fn seed_content(&mut self, source: Arc<Source>, content: Arc<Content>, config: &Config) {
        self.seeded = None;
        if !same_config(&self.config, config)
            || source.path() != content.source_path.as_path()
            || content.visual_mods != self.service.map_settings(&source.replay()).visual_mods
            || content.texture_quality
                != self
                    .service
                    .map_settings(&source.replay())
                    .graphics
                    .texture_quality
        {
            return;
        }
        if content
            .fallback
            .as_ref()
            .is_some_and(|built| !crate::visual_fallback::lock(&self.fallbacks).current(built))
        {
            return;
        }
        self.seeded = Some((source.key().clone(), Arc::downgrade(&content), config.clone()));
        for (key, entry) in &mut self.sources {
            if key.recording == *source.key()
                && entry.content.is_none()
                && key.texture_quality == content.texture_quality
                && key.visual_mods == content.visual_mods
            {
                entry.content = Some(Arc::clone(&content));
                entry.error = None;
                if self.loading.as_ref().is_some_and(|load| load.key == *key) {
                    self.loading = None;
                }
            }
        }
    }

    fn seeded(&self, key: &MapSourceKey) -> Option<Arc<Content>> {
        let (recording, content, config) = self.seeded.as_ref()?;
        let content = content.upgrade()?;
        (same_config(config, &self.config)
            && *recording == key.recording
            && content.texture_quality == key.texture_quality
            && content.visual_mods == key.visual_mods)
            .then_some(content)
    }

    pub fn content_snapshot(&self, config: &Config) -> Vec<(SourceKey, Arc<Content>)> {
        if !same_config(&self.config, config) {
            return Vec::new();
        }
        self.sources
            .iter()
            .filter_map(|(key, source)| {
                source
                    .content
                    .as_ref()
                    .map(|content| (key.recording.clone(), Arc::clone(content)))
            })
            .collect()
    }

    pub(crate) fn visual_checks(
        &self,
    ) -> Vec<(
        MapSourceKey,
        Vec<content_reader_layers::content::visual::Choice>,
        Vec<content_reader_layers::content::visual::RecordedMod>,
        String,
    )> {
        self.sources
            .iter()
            .filter_map(|(key, source)| {
                let content = source.content.as_ref()?;
                let selected = content
                    .visual_mods
                    .clone()
                    .filter(|mods| !mods.is_empty())?;
                let recorded = source
                    .source
                    .replay()
                    .identity
                    .mods
                    .iter()
                    .map(|recorded| content_reader_layers::content::visual::RecordedMod {
                        id: recorded.id.clone(),
                        name: recorded.display_name.clone(),
                    })
                    .collect();
                Some((
                    key.clone(),
                    selected,
                    recorded,
                    content.visual_revision.clone(),
                ))
            })
            .collect()
    }

    pub(crate) fn loading_content(&self) -> bool {
        self.loading.is_some()
    }

    pub(crate) fn reload_stale(&mut self, stale: &[(MapSourceKey, String)], frame: &eframe::Frame) {
        let stale = stale_content(&self.sources, stale, |source| {
            source
                .content
                .as_ref()
                .map(|content| content.visual_revision.as_str())
        });
        for key in &stale {
            if let Some(source) = self.sources.get_mut(key) {
                if let Some(content) = source.content.take() {
                    self.pool.dropper().drop_later(content);
                }
                source.error = None;
            }
        }
        for view in self.views.values_mut() {
            if view.source.as_ref().is_some_and(|key| stale.contains(key)) {
                view.retire(frame);
                view.error = None;
            }
        }
    }

    pub fn share_fallbacks(&mut self, fallbacks: crate::visual_fallback::Shared) {
        self.fallbacks = fallbacks;
    }

    fn apply_visual_fallback(&mut self, frame: &eframe::Frame) {
        let reports = std::mem::take(&mut self.visual_reports);
        let mut fallbacks = crate::visual_fallback::lock(&self.fallbacks);
        if !reports.is_empty() {
            fallbacks.reduce(reports);
        }
        let stale: Vec<MapSourceKey> = self
            .sources
            .iter()
            .filter(|(_, source)| {
                source
                    .content
                    .as_ref()
                    .and_then(|content| content.fallback.as_ref())
                    .is_some_and(|built| !fallbacks.current(built))
            })
            .map(|(key, _)| key.clone())
            .collect();
        drop(fallbacks);
        for key in stale {
            let Some(source) = self.sources.get_mut(&key) else {
                continue;
            };
            if let Some(content) = source.content.take() {
                self.pool.dropper().drop_later(content);
            }
            source.error = None;
            for view in self.views.values_mut() {
                if view.source.as_ref() == Some(&key) {
                    view.retire(frame);
                    view.error = None;
                }
            }
        }
    }

    pub fn update_config(&mut self, config: &Config, frame: &eframe::Frame, ctx: &egui::Context) {
        let changed = self.views.values().any(|view| {
            let Some(key) = &view.source else {
                return false;
            };
            let Some(source) = self.sources.get(key) else {
                return false;
            };
            let settings = self
                .service
                .widget_map_settings(&source.source.replay(), &view.target.id);
            key.texture_quality != settings.graphics.texture_quality
                || key.visual_mods != settings.visual_mods
        });
        if changed {
            let clock_id = self.clock_origins.active().map(str::to_owned);
            let targets = self
                .order
                .iter()
                .filter_map(|id| self.views.get(id).map(|view| view.target.clone()))
                .collect();
            self.set_views(clock_id.as_deref(), targets, frame, ctx);
        }
        for view in self.views.values_mut() {
            let Some(source) = view.source.as_ref().and_then(|key| self.sources.get(key)) else {
                continue;
            };
            let samples = self
                .service
                .widget_map_settings(&source.source.replay(), &view.target.id)
                .graphics
                .samples;
            if view.viewer.graphics.samples != samples
                || view
                    .loading
                    .as_ref()
                    .is_some_and(|load| load.samples != samples)
            {
                view.retire(frame);
                view.viewer.graphics.samples = samples;
                view.error = None;
            }
        }
        if same_config(&self.config, config) {
            return;
        }
        self.config = config.clone();
        self.source_bindings.retain(|_, binding| binding.is_ok());
        if let Some(load) = &self.loading {
            load.alive.store(false, Ordering::Release);
        }
        for source in self.sources.values_mut() {
            if let Some(content) = source.content.take() {
                self.pool.dropper().drop_later(content);
            }
            source.error = None;
        }
        for view in self.views.values_mut() {
            view.retire(frame);
            view.error = None;
        }
    }

    pub fn set_views(
        &mut self,
        clock_id: Option<&str>,
        views: Vec<DashboardMapView>,
        frame: &eframe::Frame,
        ctx: &egui::Context,
    ) {
        let origin_status = match clock_id {
            Some(clock_id) => self.clock_origins.admit(clock_id),
            None if self.clock_origins.active().is_some() => {
                crate::page_clock::OriginStatus::Retired
            }
            None => crate::page_clock::OriginStatus::Current,
        };
        if origin_status == crate::page_clock::OriginStatus::Retired {
            return;
        }
        let new_origin = origin_status == crate::page_clock::OriginStatus::NewOrigin;
        let wanted: BTreeSet<_> = views.iter().map(|view| view.id.clone()).collect();
        let removed: Vec<_> = self
            .views
            .keys()
            .filter(|id| !wanted.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            if let Some(mut view) = self.views.remove(&id) {
                view.retire(frame);
            }
        }
        self.order = views.iter().map(|view| view.id.clone()).collect();
        for target in views {
            let resolved = self.resolved_source(&target);
            let (source, error) = match resolved {
                Ok(source) => (source, None),
                Err(error) => (None, Some(error)),
            };

            let retained = self.views.get(&target.id).is_some_and(|view| {
                view.target.source_id == target.source_id
                    && view.target.revision == target.revision
                    && view.target.path == target.path
            });
            if retained {
                let view = self.views.get_mut(&target.id).expect("retained map");
                if new_origin || target.sequence >= view.target.sequence {
                    if view.source != source {
                        view.retire(frame);
                        view.source = source;
                        view.error = error;
                    }
                    if !view.target.visible && target.visible {
                        view.error = None;
                        view.viewer.retry_map();
                        let requested = RequestedSource::target(&target);
                        if self
                            .source_bindings
                            .get(&requested)
                            .is_some_and(Result::is_err)
                        {
                            self.source_bindings.remove(&requested);
                        }
                        if let Some(source) = view
                            .source
                            .as_ref()
                            .and_then(|key| self.sources.get_mut(key))
                        {
                            source.error = None;
                        }
                    }
                    view.target = target;
                }
                continue;
            }
            if let Some(mut old) = self.views.remove(&target.id) {
                old.retire(frame);
            }
            let target_id = target.id.clone();
            self.views.insert(
                target.id.clone(),
                MapView {
                    target,
                    source,
                    error,
                    loading: None,
                    admission_error: None,
                    viewer: Box::new(Viewer::map_view(
                        self.config.clone(),
                        self.options.clone(),
                        self.gpu.clone(),
                        Arc::clone(&self.pool),
                    )),
                },
            );
            if let Some(view) = self.views.get_mut(&target_id) {
                view.viewer.diagnostics = self.diagnostics;
            }
        }
        let requested: BTreeSet<_> = self
            .views
            .values()
            .map(|view| RequestedSource::target(&view.target))
            .collect();
        self.source_bindings
            .retain(|key, _| requested.contains(key));
        if let Some(opening) = &self.opening_source
            && !requested.contains(&opening.requested)
        {
            opening.alive.store(false, Ordering::Release);
        }
        let live: BTreeSet<_> = self
            .views
            .values()
            .filter_map(|view| view.source.clone())
            .collect();
        let removed: Vec<_> = self
            .sources
            .keys()
            .filter(|key| !live.contains(*key))
            .cloned()
            .collect();
        for key in removed {
            if let Some(load) = self.loading.as_ref().filter(|load| load.key == key) {
                load.alive.store(false, Ordering::Release);
            }
            if let Some(source) = self.sources.remove(&key) {
                self.pool.dropper().drop_later(source);
            }
        }
        self.admit(frame);
        ctx.request_repaint();
    }

    fn admit(&mut self, frame: &eframe::Frame) {
        for view in self.views.values_mut() {
            let shown =
                view.target.visible && view.target.clip.width > 0 && view.target.clip.height > 0;
            let size = view.target.bounds;
            view.admission_error = (size.width > self.limits.dimension
                || size.height > self.limits.dimension)
                .then(|| {
                    "This map exceeds the graphics device's texture dimensions. Resize the widget."
                        .to_owned()
                });
            if view.admission_error.is_some() {
                view.retire(frame);
            } else if !shown {
                view.viewer.release_viewport(frame);
            }
        }
    }

    fn resolved_source(
        &mut self,
        target: &DashboardMapView,
    ) -> Result<Option<MapSourceKey>, String> {
        let Some(binding) = self.source_bindings.get(&RequestedSource::target(target)) else {
            return Ok(None);
        };
        let source = binding.clone()?;
        let settings = self
            .service
            .widget_map_settings(&source.replay(), &target.id);
        let key = MapSourceKey {
            recording: source.key().clone(),
            texture_quality: settings.graphics.texture_quality,
            visual_mods: settings.visual_mods.clone(),
        };
        if !self.sources.contains_key(&key) {
            self.sources.insert(
                key.clone(),
                MapSource {
                    visual_mods: settings.visual_mods,
                    texture_quality: settings.graphics.texture_quality,
                    source,
                    content: None,
                    error: None,
                },
            );
        }
        Ok(Some(key))
    }

    fn poll_sources(&mut self, ctx: &egui::Context) {
        let finished =
            self.opening_source
                .as_ref()
                .and_then(|opening| match opening.result.try_recv() {
                    Ok(result) => Some((
                        opening.requested.clone(),
                        opening.alive.load(Ordering::Acquire),
                        result,
                    )),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => Some((
                        opening.requested.clone(),
                        opening.alive.load(Ordering::Acquire),
                        Err("Native map source loader stopped".into()),
                    )),
                });
        if let Some((requested, alive, result)) = finished {
            self.opening_source = None;
            if alive
                && self
                    .views
                    .values()
                    .any(|view| RequestedSource::target(&view.target) == requested)
            {
                self.source_bindings.insert(requested, result);
            }
        }
        let pending: Vec<_> = self
            .views
            .values()
            .filter(|view| view.source.is_none() && view.error.is_none())
            .map(|view| view.target.clone())
            .collect();
        for target in pending {
            let resolved = self.resolved_source(&target);
            if let Some(view) = self.views.get_mut(&target.id) {
                match resolved {
                    Ok(source) => view.source = source,
                    Err(error) => view.error = Some(error),
                }
            }
        }
        if self.opening_source.is_none() {
            let requested = self
                .order
                .iter()
                .filter_map(|id| self.views.get(id))
                .filter(|view| {
                    view.target.visible
                        && view.target.clip.width > 0
                        && view.target.clip.height > 0
                        && view.admission_error.is_none()
                        && view.error.is_none()
                })
                .map(|view| RequestedSource::target(&view.target))
                .find(|requested| !self.source_bindings.contains_key(requested));
            if let Some(requested) = requested {
                let service = Arc::clone(&self.service);
                let load = requested.clone();
                let alive = Arc::new(AtomicBool::new(true));
                let job_alive = Arc::clone(&alive);
                let (sender, result) = mpsc::sync_channel(1);
                let wake = ctx.clone();
                match thread::Builder::new()
                    .name("native map source".into())
                    .spawn_carried(move || {
                        background_priority();
                        if !job_alive.load(Ordering::Acquire) {
                            return;
                        }
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            load.open(&service)
                        }))
                        .unwrap_or_else(|_| Err("Native map source loader panicked".into()));
                        if job_alive.load(Ordering::Acquire) {
                            let _ = sender.send(result);
                        }
                        viewer_runtime::worker_priority::at_least(Some(0), || wake.request_repaint());
                    }) {
                    Ok(_) => {
                        self.opening_source = Some(OpeningSource {
                            requested,
                            result,
                            alive,
                        })
                    }
                    Err(error) => {
                        self.source_bindings
                            .insert(requested, Err(error.to_string()));
                        ctx.request_repaint();
                    }
                }
            }
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.poll_sources(ctx);
        let finished = self
            .loading
            .as_ref()
            .and_then(|load| match load.result.try_recv() {
                Ok(result) => Some((
                    load.key.clone(),
                    same_config(&load.config, &self.config)
                        && load.alive.load(Ordering::Acquire)
                        && self.sources.get(&load.key).is_some_and(|source| {
                            source.visual_mods == load.visual_mods
                                && source.texture_quality == load.texture_quality
                        }),
                    result,
                )),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some((
                    load.key.clone(),
                    same_config(&load.config, &self.config)
                        && load.alive.load(Ordering::Acquire)
                        && self.sources.get(&load.key).is_some_and(|source| {
                            source.visual_mods == load.visual_mods
                                && source.texture_quality == load.texture_quality
                        }),
                    Err("Native map asset loader stopped".into()),
                )),
            });
        if let Some((key, valid, result)) = finished {
            self.loading = None;
            match (self.sources.get_mut(&key).filter(|_| valid), result) {
                (Some(source), Ok(content)) => source.content = Some(content),
                (Some(source), Err(error)) => source.error = Some(error),
                (None, Ok(content)) => self.pool.dropper().drop_later(content),
                (None, Err(_)) => {}
            }
        }
        if self.loading.is_none() {
            let needed = self
                .order
                .iter()
                .filter_map(|id| self.views.get(id))
                .filter(|view| {
                    view.target.visible && view.target.clip.width > 0 && view.target.clip.height > 0
                })
                .filter(|view| view.admission_error.is_none())
                .filter_map(|view| view.source.as_ref())
                .find(|key| {
                    self.sources.get(*key).is_some_and(|source| {
                        source.content.is_none()
                            && source.error.is_none()
                            && self.waiting_source.as_deref() != Some(source.source.path())
                    })
                })
                .cloned();
            if let Some(key) = needed {
                self.load_source(key, ctx);
            }
        }
        for view in self.views.values_mut() {
            let finished = view
                .loading
                .as_ref()
                .and_then(|load| match load.result.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Native map scene builder stopped".into()))
                    }
                });
            if let Some(result) = finished {
                let load = view.loading.take().expect("finished scene load");
                let current = view
                    .source
                    .as_ref()
                    .and_then(|key| self.sources.get(key))
                    .and_then(|source| source.content.as_ref());
                match result {
                    _ if !still_current(&load.content, current) => {}
                    Ok((document, scene)) => {
                        view.viewer
                            .install_map(document, scene, view.target.revision.clone());
                    }
                    Err(error) => {
                        let failed = view
                            .source
                            .as_ref()
                            .and_then(|key| {
                                self.sources.get(key)?.content.as_ref()?.fallback.clone()
                            })
                            .filter(|built| {
                                matches!(built.under, crate::visual_fallback::Under::Mods { .. })
                            });
                        match failed {
                            Some(built) => {
                                self.visual_reports.push(crate::visual_fallback::Report {
                                    built,
                                    refusals: [format!("scene: {error}")].into(),
                                    settled: false,
                                })
                            }
                            None => view.error = Some(error),
                        }
                    }
                }
            }
            if view.error.is_some()
                || view.viewer.has_map()
                || view.loading.is_some()
                || view.admission_error.is_some()
                || !view.target.visible
                || view.target.clip.width == 0
                || view.target.clip.height == 0
            {
                continue;
            }
            let Some(source) = view.source.as_ref().and_then(|key| self.sources.get(key)) else {
                continue;
            };
            if let Some(error) = &source.error {
                view.error = Some(error.clone());
                continue;
            }
            let Some(content) = source.content.as_ref() else {
                continue;
            };
            let Some((device, queue)) = self.gpu.clone() else {
                view.error = Some("The native graphics device is unavailable".into());
                continue;
            };
            let graphics = self
                .service
                .widget_map_settings(&source.source.replay(), &view.target.id)
                .graphics;
            let document = match Document::from_content(Arc::clone(content), source.source.replay())
            {
                Ok(document) => document,
                Err(error) => {
                    view.error = Some(error.to_string());
                    continue;
                }
            };
            let (sender, receiver) = mpsc::sync_channel(1);
            let wake = ctx.clone();
            let alive = Arc::new(AtomicBool::new(true));
            let job_alive = Arc::clone(&alive);
            let workers = self.pool.workers();
            if self.pool.try_submit(Box::new(move || {
                if !job_alive.load(Ordering::Acquire) {
                    return;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    load_scene(&device, &queue, &document, graphics, &workers)
                        .map(|scene| (document, scene))
                        .map_err(|error| error.to_string())
                }))
                .unwrap_or_else(|_| Err("Native map scene creation panicked".into()));
                if job_alive.load(Ordering::Acquire) {
                    let _ = sender.send(result);
                }
                viewer_runtime::worker_priority::at_least(Some(0), || wake.request_repaint());
            })) {
                view.loading = Some(SceneLoad {
                    samples: graphics.samples,
                    content: Arc::downgrade(content),
                    result: receiver,
                    alive,
                });
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
    }

    fn load_source(&mut self, key: MapSourceKey, ctx: &egui::Context) {
        let seeded = self.seeded(&key);
        let Some(source) = self.sources.get_mut(&key) else {
            return;
        };
        if let Some(content) = seeded {
            source.content = Some(content);
            source.error = None;
            return;
        }
        let resolve = match self.config.resolve_options() {
            Ok(resolve) => resolve,
            Err(error) => {
                source.error = Some(error.to_string());
                return;
            }
        };
        let visual_mods = source.visual_mods.clone();
        let texture_quality = source.texture_quality;
        let recording =
            crate::visual_fallback::recording(&key.recording.source_id, &key.recording.revision);
        let decisions = crate::visual_fallback::lock(&self.fallbacks).decisions(&recording);
        let load_visual_mods = visual_mods.clone();
        let source_handle = Arc::clone(&source.source);
        let allow = self.options.allow_unverified;
        let config = self.config.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let job_alive = Arc::clone(&alive);
        let (sender, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        match thread::Builder::new()
            .name("dashboard map assets".into())
            .spawn_carried(move || {
                if !job_alive.load(Ordering::Acquire) {
                    return;
                }
                let initial = InitialHour {
                    branch: None,
                    hour: None,
                    hour_fraction: 0.0,
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut document = Document::load_shared(
                        source_handle.path(),
                        &resolve,
                        allow,
                        &initial,
                        source_handle.replay(),
                        &source_handle,
                        load_visual_mods.as_deref(),
                        texture_quality,
                        Some((recording.as_str(), &decisions)),
                        crate::document::LoadTarget::Content,
                    )
                    .map_err(|error| error.to_string())?;
                    document.bind_source(&source_handle);
                    if !job_alive.load(Ordering::Acquire) {
                        return Err("Map asset loading was cancelled".into());
                    }
                    let Document {
                        content, playback, ..
                    } = document;
                    drop(playback);
                    Ok(content)
                }))
                .unwrap_or_else(|_| Err("Native map asset loading panicked".into()));
                if job_alive.load(Ordering::Acquire) {
                    let _ = sender.send(result);
                }
                viewer_runtime::worker_priority::at_least(Some(0), || wake.request_repaint());
            }) {
            Ok(_) => {
                self.loading = Some(LoadingSource {
                    texture_quality,
                    key,
                    config,
                    visual_mods,
                    result,
                    alive,
                })
            }
            Err(error) => source.error = Some(error.to_string()),
        }
    }

    pub fn take_view_events(&mut self) -> Vec<(String, crate::app::picker::ViewEvent)> {
        std::mem::take(&mut self.view_events)
    }

    pub fn states(&self) -> Vec<DashboardMapState> {
        self.order
            .iter()
            .filter_map(|id| self.views.get(id))
            .map(MapView::state)
            .collect()
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        recording: Option<viewer_runtime::frame_writer::Pass<'_>>,
    ) {
        if recording.is_some() {
            self.ui_pass::<true>(ui, frame, recording);
        } else {
            self.ui_pass::<false>(ui, frame, None);
        }
    }

    fn ui_pass<const RECORDING: bool>(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        recording: Option<viewer_runtime::frame_writer::Pass<'_>>,
    ) {
        self.poll(ui.ctx());
        self.apply_visual_fallback(frame);
        let scale = ui.ctx().pixels_per_point();
        let drawn: std::collections::BTreeSet<&String> = self
            .order
            .iter()
            .filter(|id| {
                self.views.get(*id).is_some_and(|view| {
                    view.viewer.has_map()
                        && view.admission_error.is_none()
                        && view.error.is_none()
                        && view_clip(view, scale, ui.clip_rect()).is_some()
                })
            })
            .collect();
        let audio_view = audio_view(self.audio_owner.as_ref(), &self.order, |id| {
            drawn.contains(id)
        });
        for (id, view) in &mut self.views {
            if audio_view.as_ref() != Some(id) {
                view.viewer.release_audio();
            }
            if !drawn.contains(&id) {
                view.viewer.export_closed("its map widget was not drawn before its picture was saved");
                view.viewer.close_playhead();
            }
        }
        let mut interacted = None;
        let mut recorded = false;
        for id in &self.order {
            let Some(view) = self.views.get_mut(id) else {
                continue;
            };
            let body = physical_rect(view.target.bounds, scale);
            let Some(clip) = view_clip(view, scale, ui.clip_rect()) else {
                continue;
            };
            let mut map_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(body)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            map_ui.set_clip_rect(clip);
            if RECORDING {
                view.viewer.reset_drawn_frame();
            }
            if let Some(error) = view.admission_error.as_ref().or(view.error.as_ref()) {
                map_ui.centered_and_justified(|ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                });
            } else if view.viewer.has_map() {
                if let Some(source) = view.source.as_ref().and_then(|key| self.sources.get(key)) {
                    let scope = dashboard_core::map_settings::widget_key(
                        &dashboard_core::map_settings::campaign_key(&source.source.replay()),
                        &view.target.id,
                    );
                    view.viewer.follow_map_settings(&self.service, scope);
                }
                view.viewer.ui_scale = self.ui_scale;
                view.viewer.muted = self.muted;
                view.viewer.audio_focus = audio_view.as_ref() == Some(id);
                let tables = view.viewer.tables();
                engine_executable::executable_tables::scope(tables, || {
                    view.viewer.map_ui(&mut map_ui, frame, &view.target)
                });
                if view.viewer.interacted {
                    interacted = Some(id.clone());
                }
                let events = view.viewer.take_view_events();
                self.view_events
                    .extend(events.into_iter().map(|event| (id.clone(), event)));
                if let Some(report) = view.viewer.take_visual_report() {
                    self.visual_reports.push(report);
                }
            } else {
                map_ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("Loading native map…");
                });
            }
            let state = view.state();
            if RECORDING {
                recording.expect("recording pass").record(
                    state.hour.zip(state.fraction),
                    view.viewer.drew_frame(),
                    state.id,
                );
                recorded = true;
            }
            let diagnostics = view.viewer.diagnostics && view.viewer.has_map();
            let replay = view
                .source
                .as_ref()
                .and_then(|key| self.sources.get(key))
                .map(|source| source.source.replay_ref());
            let partial = replay.is_some_and(|replay| replay.is_partial());
            if state.loading || diagnostics || partial {
                let points = view.viewer.interface_points;
                map_overlay(ui, clip, (id, "status"), points, |ui, height| {
                    if partial {
                        partial_notice(ui, replay.expect("partial recording has a source"));
                        if frame
                            .wgpu_render_state()
                            .is_some_and(|state| is_software(&state.adapter))
                        {
                            ui.colored_label(egui::Color32::YELLOW, SOFTWARE_RENDERER_NOTICE);
                        }
                    }
                    if state.loading {
                        ui.label(match state.hour {
                            Some(hour) => format!(
                                "Showing {} · Loading {}",
                                date(hour),
                                date(view.target.hour)
                            ),
                            None => format!("Loading {}", date(view.target.hour)),
                        });
                    }
                    if diagnostics {
                        let tables = view.viewer.tables();
                        engine_executable::executable_tables::scope(tables, || {
                            view.viewer.diagnostics_ui(ui, height)
                        });
                    }
                });
            }
        }
        if interacted.is_some() {
            self.audio_owner = interacted;
        }
        if RECORDING && !recorded {
            recording.expect("recording pass").record(None, false, "-");
        }
    }
}

fn audio_view(
    owner: Option<&String>,
    order: &[String],
    shown: impl Fn(&String) -> bool,
) -> Option<String> {
    owner
        .filter(|id| shown(id))
        .or_else(|| order.iter().find(|id| shown(id)))
        .cloned()
}

fn view_clip(view: &MapView, scale: f32, panel: egui::Rect) -> Option<egui::Rect> {
    if view.viewer.holds_export() {
        let body = physical_rect(view.target.bounds, scale);
        return body.is_positive().then_some(body);
    }
    drawn_clip(&view.target, scale, panel)
}

fn drawn_clip(target: &DashboardMapView, scale: f32, panel: egui::Rect) -> Option<egui::Rect> {
    if !target.visible {
        return None;
    }
    let body = physical_rect(target.bounds, scale);
    let clip = physical_rect(target.clip, scale)
        .intersect(body)
        .intersect(panel);
    clip.is_positive().then_some(clip)
}

fn physical_rect(bounds: ViewBounds, scale: f32) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(bounds.x as f32 / scale, bounds.y as f32 / scale),
        egui::vec2(bounds.width as f32 / scale, bounds.height as f32 / scale),
    )
}

fn same_config(left: &Config, right: &Config) -> bool {
    left.game_dir == right.game_dir && left.user_dir == right.user_dir
}

fn stale_content<S>(
    sources: &BTreeMap<MapSourceKey, S>,
    stale: &[(MapSourceKey, String)],
    revision: impl Fn(&S) -> Option<&str>,
) -> Vec<MapSourceKey> {
    stale
        .iter()
        .filter(|(key, checked)| {
            sources
                .get(key)
                .and_then(&revision)
                .is_some_and(|loaded| loaded == checked)
        })
        .map(|(key, _)| key.clone())
        .collect()
}

fn still_current<T>(built: &std::sync::Weak<T>, current: Option<&Arc<T>>) -> bool {
    current.is_some_and(|current| std::ptr::eq(Arc::as_ptr(current), built.as_ptr()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_built_from_dropped_content_is_not_installed() {
        let built = Arc::new(1u8);
        let probe = Arc::new(1u8);
        let load = Arc::downgrade(&built);
        assert!(still_current(&load, Some(&built)));
        assert!(!still_current(&load, Some(&probe)));
        assert!(!still_current(&load, None));
    }

    #[test]
    fn a_late_visual_check_keeps_maps_loaded_since() {
        let key = |id: &str| MapSourceKey {
            recording: SourceKey {
                source_id: id.into(),
                revision: "r".into(),
            },
            texture_quality: 0,
            visual_mods: Some(Vec::new()),
        };
        let sources = BTreeMap::from([
            (key("checked"), Some("old")),
            (key("reloaded"), Some("new")),
            (key("loading"), None),
        ]);
        let stale = [
            (key("checked"), "old".to_owned()),
            (key("reloaded"), "old".to_owned()),
            (key("loading"), "old".to_owned()),
            (key("closed"), "old".to_owned()),
        ];
        assert_eq!(
            stale_content(&sources, &stale, |revision| *revision),
            [key("checked")]
        );
    }

    #[test]
    fn host_foreign_lease_binds_by_path_revision_and_keeps_owner_only_in_target() {
        let path = std::env::temp_dir().join(format!(
            "native-map-binding-{}.battleplan",
            uuid::Uuid::new_v4()
        ));
        let replay = crate::playback::tests::fixture();
        std::fs::write(&path, replay.bytes()).unwrap();
        let revision = dashboard_core::protocol::replay_revision(&path).unwrap();
        let service = Service::new(Arc::new(|_| true)).unwrap();
        let mut target = DashboardMapView {
            options: Default::default(),
            id: "synthetic-widget".into(),
            source_id: "host-only-lease".into(),
            path: path.to_string_lossy().into_owned(),
            revision: revision.clone(),
            branch_id: "1".into(),
            playing: false,
            hours_per_second: 1.0,
            hour: 105,
            fraction: 0.0,
            sequence: 7,
            seek_sequence: Some(2),
            composed_at: 0.0,
            clock_id: "page-1".into(),
            bounds: ViewBounds {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            clip: ViewBounds {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            visible: true,
        };
        assert!(service.source(&target.source_id).is_err());
        let requested = RequestedSource::target(&target);
        let source = requested.open(&service).unwrap();
        assert_eq!(source.key().revision, revision);
        target.source_id = "different-host-owner".into();
        assert_eq!(RequestedSource::target(&target), requested);
        assert!(Arc::ptr_eq(&source, &requested.open(&service).unwrap()));
        target.revision = "released-or-stale-revision".into();
        assert!(RequestedSource::target(&target).open(&service).is_err());
        drop(source);
        drop(service);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn showing_a_map_again_reopens_a_recording_that_failed_to_open() {
        fn settle(
            maps: &mut DashboardMaps,
            ctx: &egui::Context,
            id: &str,
            done: impl Fn(&MapView) -> bool,
        ) {
            let deadline =
                std::time::Instant::now() + viewer_runtime::test_support::TEST_HANG_GUARD;
            while !done(&maps.views[id]) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the source did not settle"
                );
                maps.poll_sources(ctx);
                thread::sleep(Duration::from_millis(1));
            }
        }
        let path = std::env::temp_dir().join(format!(
            "native-map-retry-{}.battleplan",
            uuid::Uuid::new_v4()
        ));
        let away = path.with_extension("away");
        let replay = crate::playback::tests::fixture();
        std::fs::write(&path, replay.bytes()).unwrap();
        let revision = dashboard_core::protocol::replay_revision(&path).unwrap();
        std::fs::rename(&path, &away).unwrap();
        let mut maps = DashboardMaps::new(
            Config::default(),
            Options::default(),
            Service::new(Arc::new(|_| true)).unwrap(),
            None,
            crate::worker_pool::WorkerPool::new().unwrap(),
        )
        .unwrap();
        maps.limits.dimension = u32::MAX;
        let bounds = ViewBounds {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let mut target = DashboardMapView {
            options: Default::default(),
            id: "retried-widget".into(),
            source_id: "source".into(),
            path: path.to_string_lossy().into_owned(),
            revision,
            branch_id: "1".into(),
            playing: false,
            hours_per_second: 1.0,
            hour: 105,
            fraction: 0.0,
            sequence: 1,
            seek_sequence: None,
            composed_at: 0.0,
            clock_id: "page-1".into(),
            bounds,
            clip: bounds,
            visible: true,
        };
        let ctx = egui::Context::default();
        let frame = eframe::Frame::_new_kittest();
        maps.set_views(Some("page-1"), vec![target.clone()], &frame, &ctx);
        settle(&mut maps, &ctx, &target.id, |view| view.error.is_some());
        let failed = maps.views[&target.id].error.clone();
        std::fs::rename(&away, &path).unwrap();
        maps.poll_sources(&ctx);
        assert_eq!(maps.views[&target.id].error, failed);
        for visible in [false, true] {
            target.visible = visible;
            target.sequence += 1;
            maps.set_views(Some("page-1"), vec![target.clone()], &frame, &ctx);
        }
        settle(&mut maps, &ctx, &target.id, |view| {
            view.source.is_some() || view.error.is_some()
        });
        let view = &maps.views[&target.id];
        assert!(view.error.is_none(), "{:?}", view.error);
        assert!(view.source.is_some());
        drop(maps);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_export_of_a_widget_that_went_is_still_answered() {
        let mut maps = DashboardMaps::new(
            Config::default(),
            Options::default(),
            Service::new(Arc::new(|_| true)).unwrap(),
            None,
            crate::worker_pool::WorkerPool::new().unwrap(),
        )
        .unwrap();
        maps.limits.dimension = u32::MAX;
        let bounds = ViewBounds { x: 0, y: 0, width: 100, height: 100 };
        let target = DashboardMapView {
            options: Default::default(),
            id: "exported-widget".into(),
            source_id: "source".into(),
            path: String::new(),
            revision: String::new(),
            branch_id: "1".into(),
            playing: false,
            hours_per_second: 1.0,
            hour: 0,
            fraction: 0.0,
            sequence: 1,
            seek_sequence: None,
            composed_at: 0.0,
            clock_id: "page-1".into(),
            bounds,
            clip: bounds,
            visible: true,
        };
        let ctx = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();
        maps.set_views(Some("page-1"), vec![target.clone()], &frame, &ctx);
        let (job, answers) = crate::host::tests::job("widget.png");
        maps.export_view(&target.id, job, &ctx, &mut frame);
        assert_eq!(crate::host::tests::taken(&answers), [Err("Wait for the map widget to finish loading before exporting".to_owned())], "a widget with no map yet refuses at once, by name");
        let (job, answers) = crate::host::tests::job("gone.png");
        maps.export_view("no-such-widget", job, &ctx, &mut frame);
        assert_eq!(crate::host::tests::taken(&answers), [Err("The requested map widget is unavailable".to_owned())]);

        let answers = maps.views.get_mut(&target.id).unwrap().viewer.arm_export("widget.png");
        maps.set_views(Some("page-1"), Vec::new(), &frame, &ctx);
        assert!(maps.views.is_empty());
        assert_eq!(crate::host::tests::taken(&answers), [Err("Capture widget.png failed: its map widget let its map go before its picture was saved".to_owned())], "a widget that goes answers its export");

        maps.set_views(Some("page-1"), vec![target.clone()], &frame, &ctx);
        let answers = maps.views.get_mut(&target.id).unwrap().viewer.arm_export("widget.png");
        assert!(maps.holds_export(), "a widget’s admitted export keeps the map drawing while BattlePlan’s window is hidden");
        maps.not_drawn();
        assert_eq!(crate::host::tests::taken(&answers), [Err("Capture widget.png failed: its map widget was not drawn before its picture was saved".to_owned())]);
    }

    #[test]
    fn one_map_sounds_the_last_interacted_or_the_first_shown() {
        let order: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let shown =
            |hidden: &'static [&'static str]| move |id: &String| !hidden.contains(&id.as_str());
        assert_eq!(
            super::audio_view(None, &order, shown(&[])),
            Some("a".into())
        );
        assert_eq!(
            super::audio_view(None, &order, shown(&["a"])),
            Some("b".into())
        );
        let c = "c".to_owned();
        assert_eq!(
            super::audio_view(Some(&c), &order, shown(&[])),
            Some(c.clone())
        );
        assert_eq!(
            super::audio_view(Some(&c), &order, shown(&["c"])),
            Some("a".into())
        );
        assert_eq!(
            super::audio_view(Some(&c), &order, shown(&["a", "b", "c"])),
            None
        );
    }

    #[test]
    fn a_scrolled_out_or_hidden_map_is_not_drawn() {
        let bounds = |y: i32| ViewBounds {
            x: 0,
            y,
            width: 100,
            height: 100,
        };
        let mut target = DashboardMapView {
            options: Default::default(),
            id: "w".into(),
            source_id: "s".into(),
            path: String::new(),
            revision: String::new(),
            branch_id: "1".into(),
            playing: false,
            hours_per_second: 1.0,
            hour: 0,
            fraction: 0.0,
            sequence: 0,
            seek_sequence: None,
            composed_at: 0.0,
            clock_id: "page-1".into(),
            bounds: bounds(0),
            clip: bounds(0),
            visible: true,
        };
        let panel = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 200.0));
        assert!(super::drawn_clip(&target, 1.0, panel).is_some());
        target.bounds = bounds(500);
        target.clip = bounds(500);
        assert!(
            super::drawn_clip(&target, 1.0, panel).is_none(),
            "scrolled out"
        );
        target.bounds = bounds(0);
        target.clip = bounds(0);
        target.visible = false;
        assert!(super::drawn_clip(&target, 1.0, panel).is_none(), "hidden");
    }

    #[test]
    fn set_views_admits_reloaded_page_at_sequence_zero_and_ignores_late_old_page_snapshots() {
        let mut maps = DashboardMaps::new(
            Config::default(),
            Options::default(),
            Service::new(Arc::new(|_| true)).unwrap(),
            None,
            crate::worker_pool::WorkerPool::new().unwrap(),
        )
        .unwrap();
        maps.limits.dimension = u32::MAX;
        let bounds = ViewBounds {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let make = |id: &str, clock_id: &str, sequence: u64, hour: i32, playing: bool| DashboardMapView {
            options: Default::default(),
            id: id.into(),
            source_id: "source".into(),
            path: String::new(),
            revision: "rev-1".into(),
            branch_id: "1".into(),
            playing,
            hours_per_second: 24.0,
            hour,
            fraction: 0.0,
            sequence,
            seek_sequence: Some(sequence),
            composed_at: 1000.0,
            clock_id: clock_id.into(),
            bounds,
            clip: bounds,
            visible: true,
        };
        let ctx = egui::Context::default();
        let frame = eframe::Frame::_new_kittest();
        maps.set_views(
            Some("page-A"),
            vec![
                make("playing-widget", "page-A", 80, 100, true),
                make("paused-widget", "page-A", 80, 100, false),
            ],
            &frame,
            &ctx,
        );
        assert_eq!(maps.views["playing-widget"].target.sequence, 80);
        assert_eq!(maps.views["paused-widget"].target.sequence, 80);

        maps.set_views(
            Some("page-B"),
            vec![
                make("playing-widget", "page-B", 0, 250, true),
                make("paused-widget", "page-B", 0, 260, false),
                make("new-widget", "page-B", 0, 270, false),
            ],
            &frame,
            &ctx,
        );
        assert_eq!(maps.views["playing-widget"].target.clock_id, "page-B");
        assert_eq!(maps.views["playing-widget"].target.sequence, 0);
        assert_eq!(maps.views["playing-widget"].target.hour, 250);
        assert_eq!(maps.views["paused-widget"].target.clock_id, "page-B");
        assert_eq!(maps.views["paused-widget"].target.sequence, 0);
        assert_eq!(maps.views["paused-widget"].target.hour, 260);
        assert!(maps.views.contains_key("new-widget"));
        assert!(
            maps.states().iter().all(|state| state.clock_id == "page-B"),
            "map-state replies must echo the accepted target's clock_id"
        );

        maps.set_views(
            Some("page-A"),
            vec![make("playing-widget", "page-A", 81, 999, true)],
            &frame,
            &ctx,
        );
        assert_eq!(maps.views.len(), 3, "a retired page snapshot must not remove reloaded widgets");
        assert_eq!(maps.views["playing-widget"].target.clock_id, "page-B");
        assert_eq!(maps.views["playing-widget"].target.sequence, 0);
        assert_eq!(maps.views["playing-widget"].target.hour, 250);
        assert_eq!(maps.views["paused-widget"].target.clock_id, "page-B");
        assert_eq!(maps.views["paused-widget"].target.hour, 260);
        assert!(maps.views.contains_key("new-widget"));

        maps.set_views(Some("page-A"), Vec::new(), &frame, &ctx);
        assert_eq!(
            maps.views.len(),
            3,
            "a late empty clear from retired page A must not wipe page B's widgets"
        );

        maps.set_views(Some("page-B"), Vec::new(), &frame, &ctx);
        assert!(maps.views.is_empty(), "a legitimate empty clear from active page B clears widgets");

        maps.set_views(
            Some("page-A"),
            vec![make("playing-widget", "page-A", 82, 1000, true)],
            &frame,
            &ctx,
        );
        assert!(
            maps.views.is_empty(),
            "a late populated snapshot from retired page A after page B's clear must be ignored"
        );
    }
}
