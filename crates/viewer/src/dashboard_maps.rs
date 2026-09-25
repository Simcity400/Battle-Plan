//! Independent native map bodies on one eframe root and one graphics device.
//!
//! The host supplies absolute replay time and physical geometry. Source assets
//! and decode/work resources are shared; mutable Scene state remains per view.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use dashboard::protocol::{DashboardMapState, DashboardMapView, ViewBounds};
use dashboard::{Service, Source, SourceKey};
use eframe::{egui, wgpu};

use crate::{
    Config, Options,
    app::{Viewer, date, load_scene},
    document::{Content, Document, InitialHour},
    hours::{Resources, SharedDecoder},
    map::scene::Scene,
};

// Immutable artwork can be shared only when the recording and visual choices match.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MapSourceKey {
    recording: SourceKey,
    texture_quality: u8,
    visual_mods: Option<Vec<String>>,
}

struct MapSource {
    texture_quality: u8,
    visual_mods: Option<Vec<String>>,
    source: Arc<Source>,
    decoder: Arc<SharedDecoder>,
    content: Option<Arc<Content>>,
    error: Option<String>,
}

struct MapView {
    target: DashboardMapView,
    source: Option<MapSourceKey>,
    // Keep the large renderer out of BTreeMap nodes and their insertion/removal
    // temporaries. Moving several inline Viewers can exhaust the Windows UI stack.
    viewer: Box<Viewer>,
    loading: Option<SceneLoad>,
    permit: Option<Arc<ScenePermit>>,
    error: Option<String>,
    warnings: bool,
    admission_error: Option<String>,
}

impl MapView {
    fn retire(&mut self, frame: &eframe::Frame) {
        // SceneLoad cancellation keeps the worker's permit charged until that
        // job actually finishes. Resident permits leave with Scene on Dropper.
        let load = self.loading.take();
        if let Some(load) = &load {
            load.alive.store(false, Ordering::Release);
        }
        if self.viewer.has_map() || self.permit.is_some() || load.is_some() {
            self.viewer.retire_map(frame, (load, self.permit.take()));
        }
    }

    fn state(&self) -> DashboardMapState {
        let error = self.admission_error.as_ref().or(self.error.as_ref());
        if error.is_none() && self.viewer.has_map() {
            return self.viewer.map_state(&self.target);
        }
        DashboardMapState {
            id: self.target.id.clone(),
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
    visual_mods: Option<Vec<String>>,
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
    result: mpsc::Receiver<Result<(Document, Scene), String>>,
    alive: Arc<AtomicBool>,
    // Declared after the receiver, so a queued Scene is dropped first.
    permit: Arc<ScenePermit>,
}

impl Drop for SceneLoad {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[derive(Default)]
struct SceneUsage {
    views: usize,
    pixels: u64,
}

struct SceneBudget {
    usage: Mutex<SceneUsage>,
    views: usize,
    pixels: u64,
}

impl SceneBudget {
    fn acquire(self: &Arc<Self>, pixels: u64) -> Option<Arc<ScenePermit>> {
        let mut used = self.usage.lock().unwrap_or_else(|error| error.into_inner());
        if used.views >= self.views || used.pixels.saturating_add(pixels) > self.pixels {
            return None;
        }
        used.views += 1;
        used.pixels += pixels;
        Some(Arc::new(ScenePermit {
            budget: Arc::clone(self),
            pixels: AtomicU64::new(pixels),
        }))
    }
}

struct ScenePermit {
    budget: Arc<SceneBudget>,
    pixels: AtomicU64,
}

impl ScenePermit {
    fn reserve_pixels(&self, pixels: u64) -> bool {
        let mut used = self
            .budget
            .usage
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = self.pixels.load(Ordering::Relaxed);
        // Keep the high-water charge until Scene is destroyed. Shrinking a
        // widget alone does not prove its old targets have been released yet.
        if pixels <= previous {
            return true;
        }
        let next = used.pixels.saturating_add(pixels - previous);
        if next > self.budget.pixels {
            return false;
        }
        used.pixels = next;
        self.pixels.store(pixels, Ordering::Relaxed);
        true
    }
}

impl Drop for ScenePermit {
    fn drop(&mut self) {
        let mut used = self
            .budget
            .usage
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        used.views -= 1;
        used.pixels -= self.pixels.load(Ordering::Relaxed);
    }
}

pub struct DashboardMaps {
    /// The process-wide interface scale setting every map view draws at.
    pub(crate) ui_scale: crate::map::ui_scale::Setting,
    config: Config,
    options: Options,
    service: Arc<Service>,
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    resources: Arc<Resources>,
    sources: BTreeMap<MapSourceKey, MapSource>,
    views: BTreeMap<String, MapView>,
    order: Vec<String>,
    // Content::load has its own scoped asset waves. Admit only one cold source
    // at a time instead of multiplying those waves for every map body.
    loading: Option<LoadingSource>,
    limits: MapLimits,
    budget: Arc<SceneBudget>,
    waiting_source: Option<PathBuf>,
}

struct MapLimits {
    views: usize,
    pixels: u64,
    dimension: u32,
}

impl MapLimits {
    fn new(gpu: &Option<(wgpu::Device, wgpu::Queue)>) -> Self {
        // Admission budgets, not a claim about total GPU memory: immutable
        // Scene passes still own additional per-view resources in this prototype.
        let views = std::env::var("BATTLE_PLAN_DASHBOARD_MAP_LIMIT").or_else(|_| std::env::var("HOI4_DASHBOARD_MAP_LIMIT"))
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| (1..=128).contains(value))
            .unwrap_or(4);
        let pixels = std::env::var("BATTLE_PLAN_DASHBOARD_MAP_PIXELS").or_else(|_| std::env::var("HOI4_DASHBOARD_MAP_PIXELS"))
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| (1..=128 * 1024 * 1024).contains(value))
            .unwrap_or(8 * 1024 * 1024);
        let dimension = gpu
            .as_ref()
            .map_or(0, |(device, _)| device.limits().max_texture_dimension_2d);
        Self {
            views,
            pixels,
            dimension,
        }
    }
}

impl DashboardMaps {
    pub(crate) fn export_view(
        &mut self,
        id: &str,
        path: String,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
    ) -> Result<(), String> {
        let view = self
            .views
            .get_mut(id)
            .ok_or("The requested map widget is unavailable")?;
        if !view.viewer.has_map() {
            return Err("Wait for the map widget to finish loading before exporting".into());
        }
        view.viewer.host_map_action(
            dashboard::protocol::MapAction::Export {
                path,
                view_id: None,
            },
            ctx,
            frame,
        );
        Ok(())
    }

    pub fn new(
        config: Config,
        options: Options,
        service: Arc<Service>,
        gpu: Option<(wgpu::Device, wgpu::Queue)>,
        resources: Arc<Resources>,
    ) -> io::Result<Self> {
        let limits = MapLimits::new(&gpu);
        let budget = Arc::new(SceneBudget {
            usage: Mutex::new(SceneUsage::default()),
            views: limits.views,
            pixels: limits.pixels,
        });
        Ok(Self {
            ui_scale: Default::default(),
            config,
            options,
            service,
            gpu,
            resources,
            sources: BTreeMap::new(),
            views: BTreeMap::new(),
            order: Vec::new(),
            loading: None,
            limits,
            budget,
            waiting_source: None,
        })
    }

    pub fn wait_for_source(&mut self, path: Option<PathBuf>) {
        self.waiting_source = path;
    }

    pub fn seed_content(&mut self, source: Arc<Source>, content: Arc<Content>, config: &Config) {
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
        for (key, entry) in &mut self.sources {
            if key.recording == *source.key()
                && entry.content.is_none()
                && key.texture_quality == content.texture_quality
                && key.visual_mods == content.visual_mods
            {
                entry.content = Some(Arc::clone(&content));
                entry.error = None;
            }
        }
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
            let targets = self
                .order
                .iter()
                .filter_map(|id| self.views.get(id).map(|view| view.target.clone()))
                .collect();
            self.set_views(targets, frame, ctx);
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
        if let Some(load) = &self.loading {
            load.alive.store(false, Ordering::Release);
        }
        for source in self.sources.values_mut() {
            if let Some(content) = source.content.take() {
                self.resources.dropper.drop_later(content);
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
        views: Vec<DashboardMapView>,
        frame: &eframe::Frame,
        ctx: &egui::Context,
    ) {
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
            let resolved = self.service.source(&target.source_id).and_then(|source| {
                if source.key().revision != target.revision
                    || source.path() != Path::new(&target.path)
                {
                    return Err("The map source does not match its recording snapshot".into());
                }
                let settings = self
                    .service
                    .widget_map_settings(&source.replay(), &target.id);
                let key = MapSourceKey {
                    recording: source.key().clone(),
                    texture_quality: settings.graphics.texture_quality,
                    visual_mods: settings.visual_mods.clone(),
                };
                if !self.sources.contains_key(&key) {
                    let decoder = self
                        .resources
                        .decoder(source.replay())
                        .map_err(|error| error.to_string())?;
                    self.sources.insert(
                        key.clone(),
                        MapSource {
                            visual_mods: settings.visual_mods,
                            texture_quality: settings.graphics.texture_quality,
                            source,
                            decoder,
                            content: None,
                            error: None,
                        },
                    );
                }
                Ok(key)
            });
            let (source, error) = match resolved {
                Ok(source) => (Some(source), None),
                Err(error) => (None, Some(error)),
            };

            let retained = self.views.get(&target.id).is_some_and(|view| {
                view.target.source_id == target.source_id
                    && view.target.revision == target.revision
                    && view.target.path == target.path
            });
            if retained {
                let view = self.views.get_mut(&target.id).expect("retained map");
                if target.sequence >= view.target.sequence {
                    if view.source != source {
                        view.retire(frame);
                        view.source = source;
                        view.error = error;
                    }
                    if !view.target.visible && target.visible {
                        view.error = None;
                        view.viewer.retry_map();
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
            self.views.insert(
                target.id.clone(),
                MapView {
                    target,
                    source,
                    error,
                    warnings: false,
                    loading: None,
                    admission_error: None,
                    permit: None,
                    viewer: Box::new(Viewer::map_view(
                        self.config.clone(),
                        self.options.clone(),
                        self.gpu.clone(),
                        Arc::clone(&self.resources),
                    )),
                },
            );
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
                self.resources.dropper.drop_later(source);
            }
        }
        self.admit(frame);
        ctx.request_repaint();
    }

    fn admit(&mut self, frame: &eframe::Frame) {
        let mut used_views = 0;
        let mut used_pixels = 0_u64;
        // Retain hidden ready maps when there is room, after visible maps have
        // been admitted. A demoted map keeps its camera and source binding.
        for visible in [true, false] {
            for id in &self.order {
                let Some(view) = self.views.get_mut(id) else {
                    continue;
                };
                let shown = view.target.visible
                    && view.target.clip.width > 0
                    && view.target.clip.height > 0;
                if shown != visible {
                    continue;
                }
                let size = view.target.bounds;
                let pixels = u64::from(size.width)
                    * u64::from(size.height)
                    * u64::from(view.viewer.graphics.samples.div_ceil(4));
                view.admission_error = if size.width > self.limits.dimension
                    || size.height > self.limits.dimension
                {
                    Some("This map exceeds the graphics device's texture dimensions. Resize the widget.".into())
                } else if used_views >= self.limits.views
                    || used_pixels.saturating_add(pixels) > self.limits.pixels
                {
                    Some("The native map resource budget is full. Resize or close another map widget.".into())
                } else {
                    None
                };
                let permit = view
                    .permit
                    .as_ref()
                    .or_else(|| view.loading.as_ref().map(|load| &load.permit));
                if view.admission_error.is_none()
                    && permit.is_some_and(|permit| !permit.reserve_pixels(pixels))
                {
                    // Logical admission is valid, but obsolete work still owns
                    // some capacity. Retire this target and retry after release.
                    view.retire(frame);
                }
                if view.admission_error.is_some() {
                    view.retire(frame);
                } else if shown || view.viewer.has_map() || view.loading.is_some() {
                    used_views += 1;
                    used_pixels += pixels;
                }
            }
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
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
                (None, Ok(content)) => self.resources.dropper.drop_later(content),
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
                match result {
                    Ok((document, scene)) => {
                        view.permit = Some(Arc::clone(&load.permit));
                        view.viewer
                            .install_map(document, scene, view.target.revision.clone());
                    }
                    Err(error) => view.error = Some(error),
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
            let pixels = u64::from(view.target.bounds.width)
                * u64::from(view.target.bounds.height)
                * u64::from(graphics.samples.div_ceil(4));
            let Some(permit) = self.budget.acquire(pixels) else {
                ctx.request_repaint_after(Duration::from_millis(16));
                continue;
            };
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
            let job_permit = Arc::clone(&permit);
            if self.resources.try_submit(move || {
                // Both queued and running obsolete jobs keep this charge. The
                // permit drops after a cancelled Scene or failed send drops.
                let _permit = job_permit;
                if !job_alive.load(Ordering::Acquire) {
                    return;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    load_scene(&device, &queue, &document, graphics)
                        .map(|scene| (document, scene))
                        .map_err(|error| error.to_string())
                }))
                .unwrap_or_else(|_| Err("Native map scene creation panicked".into()));
                if job_alive.load(Ordering::Acquire) {
                    let _ = sender.send(result);
                }
                wake.request_repaint();
            }) {
                view.loading = Some(SceneLoad {
                    samples: graphics.samples,
                    result: receiver,
                    alive,
                    permit,
                });
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
    }

    fn load_source(&mut self, key: MapSourceKey, ctx: &egui::Context) {
        let Some(source) = self.sources.get_mut(&key) else {
            return;
        };
        let resolve = match self.config.resolve_options() {
            Ok(resolve) => resolve,
            Err(error) => {
                source.error = Some(error.to_string());
                return;
            }
        };
        let visual_mods = source.visual_mods.clone();
        let texture_quality = source.texture_quality;
        let load_visual_mods = visual_mods.clone();
        let source_handle = Arc::clone(&source.source);
        let decoder = Arc::clone(&source.decoder);
        let allow = self.options.allow_unverified;
        let config = self.config.clone();
        let alive = Arc::new(AtomicBool::new(true));
        let job_alive = Arc::clone(&alive);
        let (sender, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        match thread::Builder::new()
            .name("dashboard map assets".into())
            .spawn(move || {
                if !job_alive.load(Ordering::Acquire) {
                    return;
                }
                let initial = InitialHour {
                    branch: None,
                    hour: None,
                    hour_fraction: 0.0,
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let document = Document::load_shared(
                        source_handle.path(),
                        &resolve,
                        allow,
                        &initial,
                        source_handle.replay(),
                        load_visual_mods.as_deref(),
                        texture_quality,
                    )
                    .map_err(|error| error.to_string())?;
                    if !job_alive.load(Ordering::Acquire) {
                        return Err("Map asset loading was cancelled".into());
                    }
                    let Document {
                        content, playback, ..
                    } = document;
                    // A busy decoder may take time to release its source lock.
                    // Keep this wait and the old block's destruction off the UI.
                    decoder.seed(playback).map_err(|error| error.to_string())?;
                    Ok(content)
                }))
                .unwrap_or_else(|_| Err("Native map asset loading panicked".into()));
                if job_alive.load(Ordering::Acquire) {
                    let _ = sender.send(result);
                }
                wake.request_repaint();
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

    pub fn states(&self) -> Vec<DashboardMapState> {
        self.order
            .iter()
            .filter_map(|id| self.views.get(id))
            .map(MapView::state)
            .collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        self.poll(ui.ctx());
        let scale = ui.ctx().pixels_per_point();
        for id in &self.order {
            let Some(view) = self.views.get_mut(id) else {
                continue;
            };
            if !view.target.visible {
                continue;
            }
            let body = physical_rect(view.target.bounds, scale);
            let clip = physical_rect(view.target.clip, scale)
                .intersect(body)
                .intersect(ui.clip_rect());
            if !clip.is_positive() {
                continue;
            }
            let mut map_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(body)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            map_ui.set_clip_rect(clip);
            if let Some(error) = view.admission_error.as_ref().or(view.error.as_ref()) {
                map_ui.centered_and_justified(|ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                });
            } else if view.viewer.has_map() {
                if let Some(source) = view.source.as_ref().and_then(|key| self.sources.get(key)) {
                    view.viewer.graphics = self
                        .service
                        .widget_map_settings(&source.source.replay(), &view.target.id)
                        .graphics;
                }
                view.viewer.ui_scale = self.ui_scale;
                view.viewer.map_ui(&mut map_ui, frame, &view.target);
            } else {
                map_ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("Loading native map…");
                });
            }
            let state = view.state();
            let warnings = view.viewer.map_warnings();
            if state.loading || !warnings.is_empty() {
                let overlay_rect = egui::Rect::from_min_size(
                    clip.min + egui::vec2(8.0, 8.0),
                    egui::vec2(
                        (clip.width() - 16.0).max(1.0),
                        (clip.height() * 0.4).max(1.0),
                    ),
                );
                let mut overlay = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt((id, "status"))
                        .max_rect(overlay_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                overlay.set_clip_rect(clip);
                egui::Frame::new()
                    .fill(egui::Color32::from_black_alpha(220))
                    .inner_margin(6)
                    .show(&mut overlay, |ui| {
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
                        if !warnings.is_empty() {
                            if ui
                                .small_button(format!("{} data limitations", warnings.len()))
                                .clicked()
                            {
                                view.warnings = !view.warnings;
                            }
                            if view.warnings {
                                egui::ScrollArea::vertical()
                                    .max_height((overlay_rect.height() - 50.0).max(24.0))
                                    .show(ui, |ui| {
                                        for warning in warnings {
                                            ui.colored_label(egui::Color32::YELLOW, warning);
                                        }
                                    });
                            }
                        }
                    });
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_registry_entries_do_not_move_renderers_on_the_ui_stack() {
        assert!(std::mem::size_of::<MapView>() < 1024);
    }

    #[test]
    fn cancelled_scene_keeps_capacity_until_worker_and_retirement_release_it() {
        let budget = Arc::new(SceneBudget {
            usage: Mutex::new(SceneUsage::default()),
            views: 1,
            pixels: 100,
        });
        let resident = budget.acquire(60).unwrap();
        let worker = Arc::clone(&resident);
        // Removing the widget alone cannot admit another Scene while a queued
        // or running build still owns GPU allocations or its result envelope.
        drop(resident);
        assert!(budget.acquire(1).is_none());
        drop(worker);
        assert!(budget.acquire(100).is_some());
    }

    #[test]
    fn resizing_cannot_overcommit_or_release_an_existing_scene_early() {
        let budget = Arc::new(SceneBudget {
            usage: Mutex::new(SceneUsage::default()),
            views: 2,
            pixels: 100,
        });
        let first = budget.acquire(60).unwrap();
        let second = budget.acquire(40).unwrap();
        assert!(!first.reserve_pixels(61));
        assert!(first.reserve_pixels(10));
        assert!(!second.reserve_pixels(41));
        drop(first);
        assert!(second.reserve_pixels(100));
        assert!(budget.acquire(1).is_none());
        drop(second);
        assert!(budget.acquire(100).is_some());
    }
}
