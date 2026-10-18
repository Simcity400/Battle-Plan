//! Independent native map bodies on one eframe root and one graphics device.
//!
//! The host supplies absolute replay time and physical geometry. Source assets
//! and decode/work resources are shared; mutable Scene state remains per view.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use viewer_foundation::map::gpu_contract::SOFTWARE_RENDERER_NOTICE;
use viewer_foundation::map::gpu_contract::is_software;
use viewer_foundation::map::ui_scale::Setting;
use viewer_foundation::worker_priority::background_priority;

use dashboard::protocol::{DashboardMapState, DashboardMapView, ViewBounds};
use dashboard::{Service, Source, SourceKey};
use eframe::{egui, wgpu};

use crate::{
    Config, Options,
    app::{Viewer, date, load_scene, map_overlay, partial_notice},
    document::{Content, Document, InitialHour},
    hours::{Resources, SharedDecoder},
};
use viewer_world::map::scene::Scene;

// Immutable artwork can be shared only when the recording and visual choices match.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MapSourceKey {
    recording: SourceKey,
    texture_quality: u8,
    visual_mods: Option<Vec<installed_content_core::content::visual::Choice>>,
}

struct MapSource {
    texture_quality: u8,
    visual_mods: Option<Vec<installed_content_core::content::visual::Choice>>,
    source: Arc<Source>,
    decoder: Arc<SharedDecoder>,
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
    // Keep the large renderer out of BTreeMap nodes and their insertion/removal
    // temporaries. Moving several inline Viewers can exhaust the Windows UI stack.
    viewer: Box<Viewer>,
    loading: Option<SceneLoad>,
    permit: Option<Arc<ScenePermit>>,
    error: Option<String>,
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
    visual_mods: Option<Vec<installed_content_core::content::visual::Choice>>,
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
    /// The content the scene is built from ([`still_current`]).
    content: std::sync::Weak<Content>,
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
    pub(crate) ui_scale: Setting,
    config: Config,
    options: Options,
    service: Arc<Service>,
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    resources: Arc<Resources>,
    sources: BTreeMap<MapSourceKey, MapSource>,
    source_bindings: BTreeMap<RequestedSource, Result<Arc<Source>, String>>,
    opening_source: Option<OpeningSource>,
    views: BTreeMap<String, MapView>,
    order: Vec<String>,
    // Content::load has its own scoped asset waves. Admit only one cold source
    // at a time instead of multiplying those waves for every map body.
    loading: Option<LoadingSource>,
    /// The one owner of every map's visual mod fallback, shared with the Campaign map
    /// ([`crate::visual_fallback`]).
    fallbacks: crate::visual_fallback::Shared,
    /// This frame's reports for it, reduced together once every map has drawn.
    visual_reports: Vec<crate::visual_fallback::Report>,
    limits: MapLimits,
    budget: Arc<SceneBudget>,
    waiting_source: Option<PathBuf>,
    /// The host's diagnostics choice, passed on to every map.
    diagnostics: bool,
    /// The host's mute choice, passed on to every map.
    muted: bool,
    /// The map that sounds: the last one the user pressed, dragged or
    /// scrolled, else the first visible map in layout order.
    audio_owner: Option<String>,
    /// The pickers' messages for the host, by view.
    view_events: Vec<(String, crate::app::picker::ViewEvent)>,
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
            source_bindings: BTreeMap::new(),
            opening_source: None,
            views: BTreeMap::new(),
            order: Vec::new(),
            loading: None,
            fallbacks: Default::default(),
            visual_reports: Vec::new(),
            limits,
            budget,
            waiting_source: None,
            diagnostics: false,
            muted: false,
            audio_owner: None,
            view_events: Vec::new(),
        })
    }

    /// Shows or hides every map's stats and data limitations.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        // Applied at once: the sounding map may not draw again soon.
        for view in self.views.values_mut() {
            view.viewer.muted = muted;
            view.viewer.apply_audio_mix();
        }
    }

    /// Silences every map, as when the dashboard is hidden.
    pub fn release_audio(&mut self) {
        for view in self.views.values_mut() {
            view.viewer.release_audio();
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
        // Only content its fallback key holds now: never content a map failed with, nor content
        // built for another decision.
        if content
            .fallback
            .as_ref()
            .is_some_and(|built| !crate::visual_fallback::lock(&self.fallbacks).current(built))
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

    /// Each loaded map that shows visual mods: its key, selection, the recording's mods and the
    /// revision it mounted, for [`Self::reload_stale`].
    pub(crate) fn visual_checks(
        &self,
    ) -> Vec<(
        MapSourceKey,
        Vec<installed_content_core::content::visual::Choice>,
        Vec<installed_content_core::content::visual::RecordedMod>,
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
                    .map(|recorded| installed_content_core::content::visual::RecordedMod {
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

    /// Whether a source's content is loading: a check of its visual mods then misses it until it
    /// is installed.
    pub(crate) fn loading_content(&self) -> bool {
        self.loading.is_some()
    }

    /// Reloads the maps whose visual mods changed since they loaded, as a folder change does:
    /// their content is dropped off the UI thread and their views load again. Each map comes with
    /// the revision the check found changed; content loaded since carries another and is kept.
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
                    self.resources.dropper.drop_later(content);
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

    /// Shares the Campaign map's fallback owner, so every map of a recording follows one
    /// decision per key ([`crate::visual_fallback`]).
    pub fn share_fallbacks(&mut self, fallbacks: crate::visual_fallback::Shared) {
        self.fallbacks = fallbacks;
    }

    /// Reduces this frame's reports from every map at once, then drops each source's content
    /// that is no longer what its fallback key holds, so its maps load again: without the
    /// visual mods for a failure, or with them back.
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
                self.resources.dropper.drop_later(content);
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
            let target_id = target.id.clone();
            self.views.insert(
                target.id.clone(),
                MapView {
                    target,
                    source,
                    error,
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
                    .spawn(move || {
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
                        wake.request_repaint();
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
                // A scene built from content its source has dropped meanwhile (for a probe of
                // its visual mods) is neither installed nor taken to have failed: the view
                // builds again from what the source holds now.
                let current = view
                    .source
                    .as_ref()
                    .and_then(|key| self.sources.get(key))
                    .and_then(|source| source.content.as_ref());
                match result {
                    _ if !still_current(&load.content, current) => {}
                    Ok((document, scene)) => {
                        view.permit = Some(Arc::clone(&load.permit));
                        view.viewer
                            .install_map(document, scene, view.target.revision.clone());
                    }
                    Err(error) => {
                        // A scene that cannot be built with the visual mods is its content's
                        // failure, reported to the owner, which probes without them; one that
                        // cannot be built without them either is an error.
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
                    content: Arc::downgrade(content),
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
        // The decisions the recording's fallback keys hold as the load starts.
        let recording =
            crate::visual_fallback::recording(&key.recording.source_id, &key.recording.revision);
        let decisions = crate::visual_fallback::lock(&self.fallbacks).decisions(&recording);
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
                    let mut document = Document::load_shared(
                        source_handle.path(),
                        &resolve,
                        allow,
                        &initial,
                        source_handle.replay(),
                        load_visual_mods.as_deref(),
                        texture_quality,
                        Some((recording.as_str(), &decisions)),
                    )
                    .map_err(|error| error.to_string())?;
                    document.bind_source(Arc::clone(&source_handle), resolve.clone(), allow);
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

    /// What the views' pickers told the host since the last call, by view.
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

    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        self.poll(ui.ctx());
        self.apply_visual_fallback(frame);
        let scale = ui.ctx().pixels_per_point();
        // The maps drawn this frame: shown, loaded and not scrolled out.
        let drawn: std::collections::BTreeSet<&String> = self
            .order
            .iter()
            .filter(|id| {
                self.views.get(*id).is_some_and(|view| {
                    view.viewer.has_map()
                        && view.admission_error.is_none()
                        && view.error.is_none()
                        && drawn_clip(&view.target, scale, ui.clip_rect()).is_some()
                })
            })
            .collect();
        // The one map that may sound; see [`DashboardMaps::audio_owner`].
        let audio_view = audio_view(self.audio_owner.as_ref(), &self.order, |id| {
            drawn.contains(id)
        });
        // Every other map, drawn or not, stops sounding.
        for (id, view) in &mut self.views {
            if audio_view.as_ref() != Some(id) {
                view.viewer.release_audio();
            }
        }
        let mut interacted = None;
        for id in &self.order {
            let Some(view) = self.views.get_mut(id) else {
                continue;
            };
            let body = physical_rect(view.target.bounds, scale);
            let Some(clip) = drawn_clip(&view.target, scale, ui.clip_rect()) else {
                continue;
            };
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
                    let scope = dashboard::map_settings::widget_key(
                        &dashboard::map_settings::campaign_key(&source.source.replay()),
                        &view.target.id,
                    );
                    view.viewer.follow_map_settings(&self.service, scope);
                }
                view.viewer.ui_scale = self.ui_scale;
                view.viewer.muted = self.muted;
                view.viewer.audio_focus = audio_view.as_ref() == Some(id);
                view.viewer.map_ui(&mut map_ui, frame, &view.target);
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
            // Diagnostics show only when the user turned them on; the loading
            // status always shows.
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
                        view.viewer.diagnostics_ui(ui, height);
                    }
                });
            }
        }
        if interacted.is_some() {
            self.audio_owner = interacted;
        }
    }
}

/// The last interacted map while it is shown, else the first shown map in
/// layout order.
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

/// Where a map draws this frame: its clip within its body and the panel,
/// or `None` when it is hidden or scrolled out.
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

/// The checked maps whose loaded content still carries the revision the check found changed;
/// a map loading, or loaded since the check, is left alone.
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

/// Whether a scene load built from `built` completes for content its source still holds
/// (`current`).
fn still_current<T>(built: &std::sync::Weak<T>, current: Option<&Arc<T>>) -> bool {
    current.is_some_and(|current| std::ptr::eq(Arc::as_ptr(current), built.as_ptr()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene built from content its source dropped meanwhile, for a probe of its visual
    /// mods, is never installed: only one built from the content the source holds now is.
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
        let revision = dashboard::protocol::replay_revision(&path).unwrap();
        let service = Service::new(Arc::new(|_| true)).unwrap();
        let mut target = DashboardMapView {
            options: Default::default(),
            id: "synthetic-widget".into(),
            source_id: "host-only-lease".into(),
            path: path.to_string_lossy().into_owned(),
            revision: revision.clone(),
            branch_id: "1".into(),
            playing: false,
            hour: 105,
            fraction: 0.0,
            sequence: 7,
            seek_sequence: Some(2),
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
        // A hidden owner gives way to the first shown map.
        assert_eq!(
            super::audio_view(Some(&c), &order, shown(&["c"])),
            Some("a".into())
        );
        assert_eq!(
            super::audio_view(Some(&c), &order, shown(&["a", "b", "c"])),
            None
        );
    }

    /// A map scrolled out of the panel or hidden is not drawn, so it cannot
    /// be the map that sounds.
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
            hour: 0,
            fraction: 0.0,
            sequence: 0,
            seek_sequence: None,
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
