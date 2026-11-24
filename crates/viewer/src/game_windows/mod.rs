use content_reader_gui::gui::native::{self, Element, resources};
use content_reader_image::{dds, native::Decode};
use content_reader_layers::ResolvedLayers;
use content_reader_localisation::localisation::{self, Localisation, Piece, pieces};
use dashboard_base::Source;
use replay::campaign::SeekResult;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use dashboard_core::config::Config;
use eframe::wgpu;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fmt, io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_foundation::tooltip_text::{TextIcons, Tooltip};
use viewer_gpu::{
    gpu_contract::{OUTPUT_FORMAT, device_descriptor, read_texture, select_adapter},
    ui_scale::valid as valid_ui_scale,
};
use viewer_native_gui::{
    counter_hovers::gui_authored::Authored,
    lazy_icons::LazyTextures,
    native_gui::{
        self, Assets, Hit, Layout, Region, Resources, Sheets, SpriteInfo, Texture,
        lifecycle::{Instance, Tick},
    },
    native_resources::GuiResources,
    sprites::{Batch, IconTextures, SpritePass, SpriteSheet},
};

#[cfg(test)]
mod tests;

mod compose;
pub mod equipment_designer;
pub mod industrial_organisation;

pub const SETTLE_TICK: Tick = Tick {
    time: 10.0,
    dt: 10.0,
};
pub const STEADY_TICK: Tick = Tick {
    time: 10.0,
    dt: 0.0,
};
pub const DEFAULT_VIEWPORT: [u32; 2] = [2752, 1152];
const MAX_CACHED_WINDOWS: usize = 8;
const MAX_DYNAMIC_WINDOW_IMAGES: usize = 64;
const MAX_CACHED_RENDERS: usize = 16;
const MAX_DECODED_FRAMES: usize = 4;
const MAX_SURFACE_DIM: u32 = 4096;
const NO_COUNTRIES: &str =
    "a flag's country is unknown: the hour's countries (CountryAppearance) are not recorded";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameWindowError {
    Refused(String),
    Failed(String),
    Cancelled,
}

impl GameWindowError {
    pub fn refused(message: impl Into<String>) -> Self {
        Self::Refused(message.into())
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

impl fmt::Display for GameWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(text) | Self::Failed(text) => f.write_str(text),
            Self::Cancelled => f.write_str("Request cancelled"),
        }
    }
}

impl std::error::Error for GameWindowError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowRequest {
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    pub hour: i32,
    pub kind: String,
    #[serde(default)]
    pub target: serde_json::Value,
    #[serde(default = "default_scale")]
    pub scale: f32,
    #[serde(default)]
    pub sub_view: Option<String>,
    #[serde(default)]
    pub scroll: BTreeMap<String, f32>,
}

fn default_scale() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowTooltip {
    pub title: String,
    pub body: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionAlphaMask {
    pub width: u32,
    pub height: u32,
    pub bits_base64: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GameWindowScrollControl {
    List,
    Step { direction: i32 },
    Track,
    Thumb { travel: i32, thumb: i32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowScroll {
    pub list: String,
    pub vertical: bool,
    pub position: f32,
    pub range: f32,
    pub step: f32,
    pub page: f32,
    pub control: GameWindowScrollControl,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowText {
    pub path: String,
    pub label: String,
    pub value: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowRegion {
    pub path: String,
    pub rect: [f32; 4],
    pub clip: Option<[f32; 4]>,
    pub disabled: bool,
    pub close: bool,
    #[serde(default)]
    pub passthrough: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    pub tooltip: Option<GameWindowTooltip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_mask: Option<RegionAlphaMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<GameWindowScroll>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowRender {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub png_base64: String,
    pub logical_size: [f32; 2],
    pub pixel_size: [u32; 2],
    pub scale: f32,
    pub read_hour: Option<i32>,
    pub art_from_current_install: bool,
    pub art_label: Option<String>,
    pub regions: Vec<GameWindowRegion>,
    pub warnings: Vec<String>,
    pub text: Vec<GameWindowText>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceCacheKey {
    pub source_id: String,
    pub revision: String,
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub visual_revision: String,
    pub language: String,
    pub force_pow2_textures: bool,
    pub art_stack: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowCacheKey {
    pub source_id: String,
    pub revision: String,
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub visual_revision: String,
    pub language: String,
    pub force_pow2_textures: bool,
    pub art_stack: [u8; 32],
    pub window: String,
}

impl WindowCacheKey {
    pub fn interface_key(&self) -> InterfaceCacheKey {
        InterfaceCacheKey {
            source_id: self.source_id.clone(),
            revision: self.revision.clone(),
            game_dir: self.game_dir.clone(),
            user_dir: self.user_dir.clone(),
            visual_revision: self.visual_revision.clone(),
            language: self.language.clone(),
            force_pow2_textures: self.force_pow2_textures,
            art_stack: self.art_stack,
        }
    }
}

pub struct WindowCache<V> {
    capacity: usize,
    entries: HashMap<WindowCacheKey, V>,
    order: VecDeque<WindowCacheKey>,
}

impl<V> WindowCache<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn get_mut(&mut self, key: &WindowCacheKey) -> Option<&mut V> {
        if !self.entries.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.entries.get_mut(key)
    }

    pub fn insert(&mut self, key: WindowCacheKey, value: V) -> Vec<V> {
        let mut evicted = Vec::new();
        if let Some(previous) = self.entries.insert(key.clone(), value) {
            evicted.push(previous);
            self.touch(&key);
            return evicted;
        }
        while self.entries.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                if let Some(removed) = self.entries.remove(&oldest) {
                    evicted.push(removed);
                }
            } else {
                break;
            }
        }
        self.order.push_back(key);
        evicted
    }

    pub fn remove(&mut self, key: &WindowCacheKey) -> Option<V> {
        if let Some(position) = self.order.iter().position(|existing| existing == key) {
            self.order.remove(position);
        }
        self.entries.remove(key)
    }

    pub fn evict_source(&mut self, source_id: &str) -> Vec<V> {
        let keys: Vec<WindowCacheKey> = self
            .entries
            .keys()
            .filter(|key| key.source_id == source_id)
            .cloned()
            .collect();
        self.order.retain(|key| key.source_id != source_id);
        keys.into_iter()
            .filter_map(|key| self.entries.remove(&key))
            .collect()
    }

    pub fn clear(&mut self) -> Vec<V> {
        self.order.clear();
        self.entries.drain().map(|(_, value)| value).collect()
    }

    pub fn drop_unused_interfaces<I>(
        &self,
        interfaces: &mut HashMap<InterfaceCacheKey, I>,
        loading: &InterfaceCacheKey,
    ) -> Vec<I> {
        let used: HashSet<InterfaceCacheKey> =
            self.entries.keys().map(WindowCacheKey::interface_key).collect();
        let unused: Vec<InterfaceCacheKey> = interfaces
            .keys()
            .filter(|key| *key != loading && !used.contains(*key))
            .cloned()
            .collect();
        unused
            .into_iter()
            .filter_map(|key| interfaces.remove(&key))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn touch(&mut self, key: &WindowCacheKey) {
        if let Some(position) = self.order.iter().position(|existing| existing == key) {
            self.order.remove(position);
        }
        self.order.push_back(key.clone());
    }
}

pub struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    arena: shader_pipeline::gpu::Arena,
}

impl GpuContext {
    pub fn initialize() -> Result<Self, GameWindowError> {
        let instance = shader_pipeline::gpu::instance();
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let adapter =
            select_adapter(&adapters, None).map_err(|error| GameWindowError::failed(error.to_string()))?;
        let descriptor = device_descriptor(&adapter);
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).map_err(
            |error| {
                GameWindowError::failed(format!(
                    "Could not open graphics device for game window: {error}"
                ))
            },
        )?;
        let arena = shader_pipeline::gpu::Arena::new(&device);
        Ok(Self {
            device,
            queue,
            arena,
        })
    }

    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let arena = shader_pipeline::gpu::Arena::new(&device);
        Self {
            device,
            queue,
            arena,
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn arena(&self) -> &shader_pipeline::gpu::Arena {
        &self.arena
    }
}

pub struct SharedInterface {
    pub guis: Arc<native::Guis>,
    pub localisation: Arc<Localisation>,
    pub language: String,
    pub registry: Arc<resources::Registry>,
    pub recorded_registry: Option<Arc<resources::Registry>>,
    icon_content: std::sync::OnceLock<
        Result<Arc<dashboard_base::products::equipment_icon::IconContent>, String>,
    >,
}

impl SharedInterface {
    pub fn load(
        layers: &ResolvedLayers,
        viewport: [u32; 2],
        requested_language: Option<&str>,
    ) -> Result<Self, GameWindowError> {
        let art_layers = layers.presentation();
        let guis = Arc::new(
            native::read_layers_at(art_layers, viewport.map(|value| value as i32)).map_err(
                |error| {
                    GameWindowError::failed(format!("Could not load interface definitions: {error}"))
                },
            )?,
        );
        let selected = match requested_language {
            Some(lang) if !lang.is_empty() => lang.strip_prefix("l_").unwrap_or(lang).to_owned(),
            _ => localisation::selected_language(layers, None).map_err(|error| {
                GameWindowError::failed(format!("Could not determine game language: {error}"))
            })?,
        };
        let localisation = Arc::new(localisation::read(layers, &selected).map_err(|error| {
            GameWindowError::failed(format!("Could not load game localisation: {error}"))
        })?);
        let language = if localisation.language.is_empty() {
            selected
        } else {
            localisation.language.clone()
        };
        let registry = Arc::new(resources::Registry::load(art_layers).map_err(|error| {
            GameWindowError::failed(format!("Could not load interface sprite registry: {error}"))
        })?);
        let recorded_registry = art_layers
            .without_overlays()
            .and_then(|recorded| resources::Registry::load(&recorded).ok())
            .map(Arc::new);
        Ok(Self {
            guis,
            localisation,
            language,
            registry,
            recorded_registry,
            icon_content: std::sync::OnceLock::new(),
        })
    }

    pub fn icon_content(
        &self,
        layers: &ResolvedLayers,
        units: &Arc<content_reader_key_names::equipment::Database>,
        make_scripted: impl FnOnce() -> Result<dashboard_text::scripted_text::ScriptedContent, String>,
    ) -> Result<Arc<dashboard_base::products::equipment_icon::IconContent>, String> {
        self.icon_content
            .get_or_init(|| {
                let scripted = make_scripted();
                dashboard_base::products::equipment_icon::IconContent::native(
                    Arc::new(layers.clone()),
                    Arc::clone(units),
                    &scripted,
                    self.registry.as_ref(),
                )
                .map(Arc::new)
                .map_err(|error| error.to_string())
            })
            .clone()
    }
}

pub struct WindowGuiAssets<'a> {
    pub gui: &'a GuiResources,
    pub guis: &'a native::Guis,
    pub localisation: &'a Localisation,
    pub physical_viewport: Option<[i32; 2]>,
}

impl Resources for WindowGuiAssets<'_> {
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        self.physical_viewport
    }

    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.gui.sprite(name)
    }

    fn sprite_info(&self, name: &[u8]) -> Option<Cow<'_, SpriteInfo>> {
        self.gui.sprite_info(name).map(Cow::Borrowed)
    }

    fn portrait_texture(&self, _name: &[u8]) -> Option<Texture> {
        None
    }

    fn sprite_alpha(&self, name: &[u8]) -> Option<native_gui::AlphaMask> {
        self.gui.sprite_alpha(name)
    }

    fn unloaded_sprite(&self, name: &[u8]) -> bool {
        self.gui.registry().sprite(name).is_some()
    }

    fn font(&self, name: &[u8]) -> Option<&viewer_native_gui::native_text::Font> {
        self.gui.font(name)
    }

    fn localisation(&self) -> &Localisation {
        self.localisation
    }

    fn text_colors(&self) -> &native::fonts::TextColors {
        self.gui.text_colors()
    }

    fn text_effect(&self) -> Result<viewer_native_gui::native_effects::Effect, String> {
        self.gui.text_effect()
    }

    fn effect(&self, file: &str) -> Result<viewer_native_gui::native_effects::Effect, String> {
        self.gui.effect(file)
    }

    fn shield_flag(&self, _small: bool, _index: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        Err(NO_COUNTRIES.to_owned())
    }

    fn text_flag(&self, _tag: &str) -> Result<Option<Texture>, String> {
        Err(NO_COUNTRIES.to_owned())
    }

    fn resolved_text_flag(&self, _key: &[u8]) -> Result<Option<Texture>, String> {
        Err(NO_COUNTRIES.to_owned())
    }

    fn resolved_text_flag_key(&self, _text: &[char]) -> Option<usize> {
        None
    }

    fn gui_type(&self, name: &[u8]) -> Option<&Element> {
        self.guis.template(name)
    }

    fn session_text_sprite(&self, name: &[u8]) -> bool {
        self.guis.text_box_named(name)
    }

    fn texture_file(&self, path: &[u8]) -> Option<native_gui::TextureFile> {
        self.gui.texture_file(path)
    }

    fn sprite_animations(&self, sprite: &[u8]) -> Option<Vec<native_gui::AnimationRecord>> {
        self.gui.sprite_animations(sprite)
    }
}

impl Sheets for WindowGuiAssets<'_> {
    fn sheet(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.gui.sheet(sprite)
    }

    fn progress(&self, sprite: &[u8]) -> Option<[&SpriteSheet; 2]> {
        self.gui.progress(sprite)
    }

    fn masking(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.gui.masking(sprite)
    }

    fn mask(&self, sprite: &[u8]) -> Option<&SpriteSheet> {
        self.gui.mask(sprite)
    }
}

pub struct CachedWindow {
    pub guis: Arc<native::Guis>,
    pub gui: GuiResources,
    pub localisation: Arc<Localisation>,
    pub images: Vec<dds::Texture>,
    pub textures: IconTextures,
    pub lazy: Arc<LazyTextures>,
    pub pass: SpritePass,
    pub warnings: Vec<String>,
    window_name: String,
    registry: Arc<resources::Registry>,
    recorded_registry: Option<Arc<resources::Registry>>,
    font_language: Vec<u8>,
    map_size: [u32; 2],
    decode: Decode,
    base_assets: Assets,
    base_image_count: usize,
}

fn resolve_map_size(recorded_layers: &ResolvedLayers, warnings: &mut Vec<String>) -> [u32; 2] {
    match shader_pipeline::artwork::map_size(recorded_layers) {
        Ok(size) => size,
        Err(error) => {
            warnings.push(format!(
                "The recorded map size is unknown ({error}): shaders that read the map size draw with a 1x1 map."
            ));
            [1, 1]
        }
    }
}

impl CachedWindow {
    pub fn load(
        gpu: &GpuContext,
        layers: &ResolvedLayers,
        shared: &SharedInterface,
        window_name: &str,
        extra_assets: &Assets,
        decode: Decode,
    ) -> Result<Self, GameWindowError> {
        let mut warnings = Vec::new();
        let art_layers = layers.presentation();
        let guis = Arc::clone(&shared.guis);
        let Some(root) = guis.template(window_name.as_bytes()) else {
            return Err(GameWindowError::refused(missing_window_refusal(window_name)));
        };
        let localisation = Arc::clone(&shared.localisation);
        let font_language = format!("l_{}", shared.language).into_bytes();
        let map_size = resolve_map_size(layers, &mut warnings);
        let mut gui = GuiResources::new(
            Arc::clone(&shared.registry),
            &font_language,
            map_size,
            decode,
        );
        if let Some(recorded_registry) = &shared.recorded_registry {
            gui = gui.with_recorded_registry(Arc::clone(recorded_registry));
        }
        let base_assets = native_gui::assets_in(
            root,
            &|name| guis.template(name),
            localisation.as_ref(),
        );
        let mut images = Vec::new();
        gui.load(art_layers, &base_assets, &mut images).map_err(|error| {
            GameWindowError::failed(format!("Could not load assets for '{window_name}': {error}"))
        })?;
        let base_image_count = images.len();
        if !extra_assets.sprites.is_empty()
            || !extra_assets.fonts.is_empty()
            || !extra_assets.textures.is_empty()
        {
            gui.load(art_layers, extra_assets, &mut images).map_err(|error| {
                GameWindowError::failed(format!("Could not load extra assets for '{window_name}': {error}"))
            })?;
        }
        if let Some(note) = gui.uncovered_note(window_name) {
            warnings.push(note);
        }
        let textures = IconTextures::new(&gpu.device, &gpu.queue, &images).map_err(|error| {
            GameWindowError::failed(format!(
                "Could not upload textures for '{window_name}': {error}"
            ))
        })?;
        let lazy = LazyTextures::new();
        let mut pass =
            SpritePass::new(&gpu.device, &gpu.queue, &textures, &lazy, OUTPUT_FORMAT).map_err(
                |error| {
                    GameWindowError::failed(format!(
                        "Could not prepare renderer for '{window_name}': {error}"
                    ))
                },
            )?;
        pass.set_layer(true);
        pass.warm(&gpu.device, gui.effects());
        Ok(Self {
            guis,
            gui,
            localisation,
            images,
            textures,
            lazy,
            pass,
            warnings,
            window_name: window_name.to_owned(),
            registry: Arc::clone(&shared.registry),
            recorded_registry: shared.recorded_registry.as_ref().map(Arc::clone),
            font_language,
            map_size,
            decode,
            base_assets,
            base_image_count,
        })
    }

    pub fn ensure_assets(
        &mut self,
        gpu: &GpuContext,
        layers: &ResolvedLayers,
        extra: &Assets,
    ) -> Result<(), GameWindowError> {
        let art_layers = layers.presentation();
        let before = self.images.len();
        self.gui
            .load(art_layers, extra, &mut self.images)
            .map_err(|error| {
                GameWindowError::failed(format!("Could not load extra window assets: {error}"))
            })?;
        if self.images.len() == before {
            return Ok(());
        }
        if self.images.len() > self.base_image_count.saturating_add(MAX_DYNAMIC_WINDOW_IMAGES) {
            let mut fresh = GuiResources::new(
                Arc::clone(&self.registry),
                &self.font_language,
                self.map_size,
                self.decode,
            );
            if let Some(recorded_registry) = &self.recorded_registry {
                fresh = fresh.with_recorded_registry(Arc::clone(recorded_registry));
            }
            let mut rebuilt = Vec::new();
            fresh
                .load(art_layers, &self.base_assets, &mut rebuilt)
                .map_err(|error| {
                    GameWindowError::failed(format!(
                        "Could not compact base assets for '{}': {error}",
                        self.window_name
                    ))
                })?;
            self.base_image_count = rebuilt.len();
            fresh
                .load(art_layers, extra, &mut rebuilt)
                .map_err(|error| {
                    GameWindowError::failed(format!(
                        "Could not load extra window assets after compaction: {error}"
                    ))
                })?;
            self.gui = fresh;
            self.images = rebuilt;
        }
        self.textures =
            IconTextures::new(&gpu.device, &gpu.queue, &self.images).map_err(|error| {
                GameWindowError::failed(format!("Could not upload extra window textures: {error}"))
            })?;
        self.pass = SpritePass::new(
            &gpu.device,
            &gpu.queue,
            &self.textures,
            &self.lazy,
            OUTPUT_FORMAT,
        )
        .map_err(|error| {
            GameWindowError::failed(format!(
                "Could not rebuild renderer for extra assets: {error}"
            ))
        })?;
        self.pass.set_layer(true);
        self.pass.warm(&gpu.device, self.gui.effects());
        Ok(())
    }

    pub fn gui_assets(&self, physical_viewport: [i32; 2]) -> WindowGuiAssets<'_> {
        WindowGuiAssets {
            gui: &self.gui,
            guis: &self.guis,
            localisation: &self.localisation,
            physical_viewport: Some(physical_viewport),
        }
    }
}

pub struct OffscreenOutput {
    pub rgba: Vec<u8>,
    pub png: Vec<u8>,
    pub logical_size: [f32; 2],
    pub pixel_size: [u32; 2],
    pub origin: [f32; 2],
}

pub fn normalize_scale(scale: f32) -> f32 {
    if scale.is_finite() && valid_ui_scale(scale) {
        (scale * 100.0).round() / 100.0
    } else if scale.is_finite() {
        scale.clamp(0.5, 3.0)
    } else {
        1.0
    }
}

pub fn missing_window_refusal(window_name: &str) -> String {
    format!(
        "Game window '{window_name}' is not declared in the loaded interface files (unsupported mod or missing game files)."
    )
}

pub fn unsupported_window_kind_refusal(kind: &str) -> String {
    format!("Game window '{kind}' is not supported yet.")
}

pub fn batch_bounds(batch: &Batch) -> Option<[i32; 4]> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for quad in &batch.quads {
        let (mut x0, mut y0, mut x1, mut y1) = if let Some(exact) = &quad.exact {
            let mut qx0 = f32::INFINITY;
            let mut qy0 = f32::INFINITY;
            let mut qx1 = f32::NEG_INFINITY;
            let mut qy1 = f32::NEG_INFINITY;
            for [cx, cy] in exact.corners {
                qx0 = qx0.min(cx);
                qy0 = qy0.min(cy);
                qx1 = qx1.max(cx);
                qy1 = qy1.max(cy);
            }
            if let Some([left, top, right, bottom]) = exact.scissor {
                qx0 = qx0.max(left as f32);
                qy0 = qy0.max(top as f32);
                qx1 = qx1.min(right as f32);
                qy1 = qy1.min(bottom as f32);
            }
            (qx0, qy0, qx1, qy1)
        } else {
            let [x, y, w, h] = quad.screen_bounds();
            (x, y, x + w, y + h)
        };
        if !x0.is_finite() || !y0.is_finite() || !x1.is_finite() || !y1.is_finite() {
            continue;
        }
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        x0 = x0.min(x1);
        y0 = y0.min(y1);
        x1 = x1.max(x0);
        y1 = y1.max(y0);
        min_x = min_x.min(x0);
        min_y = min_y.min(y0);
        max_x = max_x.max(x1);
        max_y = max_y.max(y1);
    }
    if !min_x.is_finite() || !min_y.is_finite() || max_x <= min_x || max_y <= min_y {
        return None;
    }
    let left = min_x.floor() as i32;
    let top = min_y.floor() as i32;
    let width = (max_x.ceil() as i32 - left).max(1);
    let height = (max_y.ceil() as i32 - top).max(1);
    Some([left, top, width, height])
}

fn unpremultiply_rgba(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3];
        if a == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
        } else if a < 255 {
            let alpha = u32::from(a);
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

fn trim_transparent_edges(
    rgba: Vec<u8>,
    pixel_size: [u32; 2],
    origin: [i32; 2],
    scale: f32,
) -> Result<(Vec<u8>, [f32; 2], [u32; 2], [f32; 2]), GameWindowError> {
    let width = pixel_size[0] as usize;
    let height = pixel_size[1] as usize;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut any_visible = false;
    for y in 0..height {
        let row = y * width * 4;
        for x in 0..width {
            if rgba[row + x * 4 + 3] > 0 {
                any_visible = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if !any_visible {
        return Err(GameWindowError::refused(
            "Game window produced no visible pixels (missing artwork or refused shader).",
        ));
    }
    let crop_x0 = min_x;
    let crop_y0 = min_y;
    let crop_x1 = max_x + 1;
    let crop_y1 = max_y + 1;
    let new_w = crop_x1 - crop_x0;
    let new_h = crop_y1 - crop_y0;
    let new_logical = [new_w as f32 / scale, new_h as f32 / scale];
    let new_origin = [
        origin[0] as f32 + crop_x0 as f32 / scale,
        origin[1] as f32 + crop_y0 as f32 / scale,
    ];
    if crop_x0 == 0 && crop_y0 == 0 && crop_x1 == width && crop_y1 == height {
        return Ok((rgba, new_logical, pixel_size, new_origin));
    }
    let mut cropped = Vec::with_capacity(new_w * new_h * 4);
    for y in crop_y0..crop_y1 {
        let start = (y * width + crop_x0) * 4;
        cropped.extend_from_slice(&rgba[start..start + new_w * 4]);
    }
    Ok((cropped, new_logical, [new_w as u32, new_h as u32], new_origin))
}

pub struct ComposedSubWindow {
    pub before_part: usize,
    pub parts: Vec<native_gui::Part>,
    pub regions: Vec<Region>,
    pub diagnostics: Vec<native_gui::Diagnostic>,
    pub root_window: String,
    pub path_prefix: String,
    pub close_paths: Vec<String>,
    pub tooltip_overrides: BTreeMap<String, GameWindowTooltip>,
}

#[derive(Default)]
pub struct ComposedPlan {
    pub sub_windows: Vec<ComposedSubWindow>,
    pub extra_regions: Vec<GameWindowRegion>,
    pub warnings: Vec<String>,
    pub title: Option<String>,
    pub read_hour: Option<i32>,
    pub art_label: Option<String>,
    pub view_links: BTreeMap<String, String>,
    scroll: BTreeMap<String, f32>,
    scrollbars: Vec<ComposedSubWindow>,
    text: Vec<GameWindowText>,
}

impl ComposedPlan {
    pub fn scrolled(scroll: BTreeMap<String, f32>) -> Self {
        Self {
            scroll,
            ..Self::default()
        }
    }

    pub fn text(&mut self, path: &str, label: &str, value: Result<String, String>) {
        let (value, missing) = match value {
            Ok(value) => (Some(value), None),
            Err(missing) => (None, Some(missing)),
        };
        self.text.push(GameWindowText {
            path: path.to_owned(),
            label: label.to_owned(),
            value,
            missing,
        });
    }

    pub fn list_rows(
        &mut self,
        list: &Element,
        holder: &native_gui::Holder,
        sizes: &[[i32; 2]],
        resources: &dyn Resources,
    ) -> Result<Vec<Option<[i32; 2]>>, GameWindowError> {
        let native::Fields::ListBox(fields) = &list.fields else {
            return Err(GameWindowError::failed(format!("{} is not a list box", list.path)));
        };
        let mut bindings = native_gui::Bindings::default();
        bindings.viewport = Some(DEFAULT_VIEWPORT.map(|value| value as i32));
        bindings.regions = true;
        let smooth = list.kind == native::Kind::SmoothListBox;
        let vertical = !(smooth && fields.horizontal != 0);
        let range = list_overflow(smooth, vertical, fields, sizes) as f32;
        let position = self
            .scroll
            .get(&list.path)
            .copied()
            .filter(|position| position.is_finite())
            .unwrap_or(0.0)
            .clamp(0.0, range);
        let mut rows = native_gui::widgets::list_box_rows(
            list,
            sizes,
            (position > 0.0).then_some(position),
            resources,
            &bindings,
        )
        .map_err(|refusals| GameWindowError::failed(format_refusals(&refusals)))?;
        let [x, y, width, height] = rows.rect;
        let own = [
            holder.logical[0] + x,
            holder.logical[1] + y,
            holder.logical[0] + x + width,
            holder.logical[1] + y + height,
        ];
        let [left, top, right, bottom] = match holder.clip {
            Some(clip) => [
                own[0].max(clip[0]),
                own[1].max(clip[1]),
                own[2].min(clip[2]),
                own[3].min(clip[3]),
            ],
            None => own,
        };
        if rows.range > 0 {
            let scroll = |control| {
                Some(GameWindowScroll {
                    list: list.path.clone(),
                    vertical,
                    position,
                    range,
                    step: list_row_step(smooth, vertical, sizes),
                    page: rows.page.max(1) as f32,
                    control,
                })
            };
            self.extra_regions.push(GameWindowRegion {
                path: list.path.clone(),
                rect: [
                    left as f32,
                    top as f32,
                    (right - left).max(0) as f32,
                    (bottom - top).max(0) as f32,
                ],
                clip: None,
                disabled: false,
                close: false,
                passthrough: true,
                view: None,
                tooltip: None,
                alpha_mask: None,
                scroll: scroll(GameWindowScrollControl::List),
            });
            let (controls, regions): (Vec<_>, Vec<_>) = rows.controls.drain(..).unzip();
            for (control, region) in controls.into_iter().zip(holder.place_regions([0, 0], &regions)) {
                let control = match control {
                    native_gui::widgets::ListControl::Step(direction) => {
                        GameWindowScrollControl::Step { direction }
                    }
                    native_gui::widgets::ListControl::Track => GameWindowScrollControl::Track,
                    native_gui::widgets::ListControl::Thumb(slide) => GameWindowScrollControl::Thumb {
                        travel: slide.travel,
                        thumb: slide.thumb,
                    },
                };
                self.extra_regions.push(GameWindowRegion {
                    path: format!("{}:scrollbar/{}", list.path, region.path),
                    rect: region.rect,
                    clip: region.clip.map(|[left, top, right, bottom]| {
                        [
                            left as f32,
                            top as f32,
                            (right - left).max(0) as f32,
                            (bottom - top).max(0) as f32,
                        ]
                    }),
                    disabled: region.disabled,
                    close: false,
                    passthrough: false,
                    view: None,
                    tooltip: None,
                    alpha_mask: None,
                    scroll: scroll(control),
                });
            }
            self.scrollbars.push(ComposedSubWindow {
                before_part: holder.before,
                parts: holder.place_scrollbar(&rows),
                regions: Vec::new(),
                diagnostics: std::mem::take(&mut rows.diagnostics),
                root_window: list.path.clone(),
                path_prefix: format!("{}:scrollbar/", list.path),
                close_paths: Vec::new(),
                tooltip_overrides: BTreeMap::new(),
            });
        }
        Ok(rows
            .places
            .iter()
            .zip(sizes)
            .map(|(place, &[width, height])| {
                place.filter(|&[x, y]| {
                    let x = holder.logical[0] + x;
                    let y = holder.logical[1] + y;
                    x < right && x + width > left && y < bottom && y + height > top
                })
            })
            .collect())
    }
}

fn list_overflow(smooth: bool, vertical: bool, fields: &native::ListBox, sizes: &[[i32; 2]]) -> i32 {
    if smooth {
        let axis = usize::from(vertical);
        let run: i64 = sizes.iter().map(|size| i64::from(size[axis])).sum();
        (run - i64::from(fields.size[axis])).clamp(0, i64::from(i32::MAX)) as i32
    } else {
        let fit = match sizes.last() {
            Some(&[_, height]) if height > 0 => fields.size[1] / height,
            _ => 0,
        };
        (sizes.len() as i32).saturating_sub(fit).max(0)
    }
}

fn list_row_step(smooth: bool, vertical: bool, sizes: &[[i32; 2]]) -> f32 {
    match (smooth, sizes.first()) {
        (true, Some(&[width, height])) => (if vertical { height } else { width }).max(1) as f32,
        _ => 1.0,
    }
}

pub fn container_holder(layout: &Layout, container_path: &str) -> Option<native_gui::Holder> {
    let region = layout
        .regions
        .iter()
        .find(|region| region.path == container_path)?;
    let dot_prefix = format!("{container_path}.");
    let slash_prefix = format!("{container_path}/");
    let before = layout
        .parts
        .iter()
        .enumerate()
        .rev()
        .find(|(_, part)| {
            part.path == container_path
                || part.path.starts_with(&dot_prefix)
                || part.path.starts_with(&slash_prefix)
        })
        .map_or(layout.parts.len(), |(index, _)| index + 1);
    Some(native_gui::Holder {
        path: container_path.to_owned(),
        child: None,
        content: [region.rect[2].round() as i32, region.rect[3].round() as i32],
        origin: [region.rect[0], region.rect[1]],
        scale: 1.0,
        hit_scales: Some(Vec::new()),
        logical: [region.rect[0].round() as i32, region.rect[1].round() as i32],
        clip: region.clip,
        opacity: 1.0,
        before,
        position: None,
    })
}

pub fn merge_composed_layout(main_layout: &Layout, sub_windows: &[ComposedSubWindow]) -> Layout {
    let extra_parts: usize = sub_windows.iter().map(|sub| sub.parts.len()).sum();
    let mut parts = Vec::with_capacity(main_layout.parts.len() + extra_parts);
    let main_len = main_layout.parts.len();
    for index in 0..=main_len {
        for sub in sub_windows {
            if sub.before_part.min(main_len) == index {
                parts.extend(sub.parts.iter().cloned());
            }
        }
        if let Some(part) = main_layout.parts.get(index) {
            parts.push(part.clone());
        }
    }
    let mut diagnostics = main_layout.diagnostics.clone();
    for sub in sub_windows {
        diagnostics.extend(sub.diagnostics.iter().cloned());
    }
    Layout {
        size: main_layout.size,
        parts,
        holders: Vec::new(),
        regions: Vec::new(),
        diagnostics,
        pointed: None,
        hovered: None,
        target: None,
        boxes: Vec::new(),
    }
}

fn empty_art_refusal(layout: &Layout) -> GameWindowError {
    let details: Vec<String> = layout
        .diagnostics
        .iter()
        .take(4)
        .map(|d| format!("{}: {}", d.path, d.detail))
        .collect();
    if details.is_empty() {
        GameWindowError::refused(
            "Game window produced no drawable art (missing artwork or unsupported interface layout).",
        )
    } else {
        GameWindowError::refused(format!(
            "Game window produced no drawable art ({}).",
            details.join("; ")
        ))
    }
}

pub fn render_composed_layout_offscreen(
    gpu: &GpuContext,
    pass: &mut SpritePass,
    main_layout: &Layout,
    sub_windows: &[ComposedSubWindow],
    sheets: &dyn Sheets,
    scale: f32,
) -> Result<OffscreenOutput, GameWindowError> {
    let merged = merge_composed_layout(main_layout, sub_windows);
    render_layout_offscreen(gpu, pass, &merged, sheets, scale)
}

pub fn render_layout_offscreen(
    gpu: &GpuContext,
    pass: &mut SpritePass,
    layout: &Layout,
    sheets: &dyn Sheets,
    scale: f32,
) -> Result<OffscreenOutput, GameWindowError> {
    if layout.parts.is_empty() {
        return Err(empty_art_refusal(layout));
    }
    let mut probe = Batch::default();
    native_gui::draw(layout, [0, 0], sheets, &mut probe)
        .map_err(|refusal| GameWindowError::failed(format!("Could not draw game window: {refusal}")))?;
    let Some([min_x, min_y, width, height]) = batch_bounds(&probe) else {
        return Err(empty_art_refusal(layout));
    };
    let batch = if min_x == 0 && min_y == 0 {
        probe
    } else {
        let mut shifted = Batch::default();
        native_gui::draw(layout, [-min_x, -min_y], sheets, &mut shifted).map_err(|refusal| {
            GameWindowError::failed(format!("Could not draw game window: {refusal}"))
        })?;
        shifted
    };
    render_batch_offscreen(
        gpu,
        pass,
        &batch,
        [width as u32, height as u32],
        [min_x, min_y],
        scale,
    )
}

pub(crate) fn render_batch_offscreen(
    gpu: &GpuContext,
    pass: &mut SpritePass,
    batch: &Batch,
    logical_size: [u32; 2],
    origin: [i32; 2],
    scale: f32,
) -> Result<OffscreenOutput, GameWindowError> {
    let scale = normalize_scale(scale);
    let pixel_w = (logical_size[0] as f32 * scale).round() as u64;
    let pixel_h = (logical_size[1] as f32 * scale).round() as u64;
    if pixel_w == 0 || pixel_h == 0 {
        return Err(GameWindowError::refused(
            "Game window has an empty drawable extent.",
        ));
    }
    if pixel_w > u64::from(MAX_SURFACE_DIM) || pixel_h > u64::from(MAX_SURFACE_DIM) {
        return Err(GameWindowError::refused(format!(
            "Game window surface {}×{} at {:.2}× scale exceeds the {}×{} off-screen limit.",
            logical_size[0], logical_size[1], scale, MAX_SURFACE_DIM, MAX_SURFACE_DIM
        )));
    }
    let pixel_size = [pixel_w as u32, pixel_h as u32];
    pass.set_layer(true);
    pass.upload(
        &gpu.device,
        &gpu.queue,
        &gpu.arena,
        batch,
        pixel_size,
        scale,
    );
    if !pass.native_refusals.is_empty() {
        let reasons: Vec<String> = pass
            .native_refusals
            .iter()
            .map(|(reason, count)| format!("{reason} ({count}×)"))
            .collect();
        return Err(GameWindowError::failed(format!(
            "Game window GPU render refused draws: {}",
            reasons.join("; ")
        )));
    }
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("game window offscreen"),
        size: wgpu::Extent3d {
            width: pixel_size[0],
            height: pixel_size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OUTPUT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("game window render"),
        });
    pass.draw_layer(&mut encoder, &view, pixel_size)
        .map_err(|error| GameWindowError::failed(format!("Could not render game window layer: {error}")))?;
    let mut submissions = pass.bake_buffers();
    submissions.push(encoder.finish());
    gpu.queue.submit(submissions);
    let mut rgba = read_texture(&gpu.device, &gpu.queue, &target, 4).map_err(|error| {
        GameWindowError::failed(format!("Could not read back rendered game window: {error}"))
    })?;
    unpremultiply_rgba(&mut rgba);
    let (rgba, logical_size, pixel_size, origin) =
        trim_transparent_edges(rgba, pixel_size, origin, scale)?;
    let png = encode_png(pixel_size[0], pixel_size[1], &rgba)?;
    Ok(OffscreenOutput {
        rgba,
        png,
        logical_size,
        pixel_size,
        origin,
    })
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, GameWindowError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|error| {
            GameWindowError::failed(format!("Could not start game window PNG: {error}"))
        })?;
        writer.write_image_data(rgba).map_err(|error| {
            GameWindowError::failed(format!("Could not write game window PNG: {error}"))
        })?;
    }
    Ok(out)
}

pub fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in pieces(text) {
        if let Piece::Text(segment) = piece {
            out.push_str(segment);
        }
    }
    out.trim().to_owned()
}

struct RegistryIcons<'a> {
    resources: &'a dyn Resources,
}

impl TextIcons for RegistryIcons<'_> {
    fn icon(&self, name: &str) -> Option<(u32, i64)> {
        let (candidates, frame) = localisation::icon_lookup(name);
        let info = candidates
            .iter()
            .find_map(|candidate| self.resources.sprite_info(candidate))?;
        Some((info.frames.max(1) as u32, i64::from(frame.unwrap_or(0))))
    }
}

pub fn element_by_path<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    if root.path == path {
        return Some(root);
    }
    for child in root.nested() {
        if let Some(found) = element_by_path(child, path) {
            return Some(found);
        }
    }
    None
}

fn element_always_transparent(element: &Element) -> bool {
    element
        .button()
        .is_some_and(|button| button.always_transparent)
        || element.icon().is_some_and(|icon| icon.always_transparent)
        || element
            .text_box()
            .is_some_and(|text| text.always_transparent)
}

fn region_alpha_mask(
    hit: &native_gui::HitGeometry,
    rect: [f32; 4],
) -> Option<Option<RegionAlphaMask>> {
    if hit.form != Hit::Alpha {
        return Some(None);
    }
    let width = rect[2].ceil().max(1.0) as u32;
    let height = rect[3].ceil().max(1.0) as u32;
    let total = (width as usize).checked_mul(height as usize)?;
    let mut bits = vec![0u8; total.div_ceil(8)];
    let mut any_opaque = false;
    let mut any_transparent = false;
    for py in 0..height {
        let y = rect[1] + py as f32 + 0.5;
        for px in 0..width {
            let x = rect[0] + px as f32 + 0.5;
            let opaque = hit.contains([x, y]);
            if opaque {
                any_opaque = true;
                let idx = (py * width + px) as usize;
                bits[idx >> 3] |= 1 << (idx & 7);
            } else {
                any_transparent = true;
            }
        }
    }
    if !any_opaque {
        return None;
    }
    if !any_transparent {
        return Some(None);
    }
    Some(Some(RegionAlphaMask {
        width,
        height,
        bits_base64: encode_base64(&bits),
    }))
}

pub struct RegionActions<'a> {
    pub close_paths: &'a [&'a str],
    pub tooltips: &'a BTreeMap<String, GameWindowTooltip>,
    pub views: &'a BTreeMap<String, String>,
}

pub fn regions_from_layout(
    roots: &[&Element],
    regions: &[Region],
    origin: [f32; 2],
    resources: &dyn Resources,
    localisation: &Localisation,
    actions: &RegionActions<'_>,
) -> Vec<GameWindowRegion> {
    let RegionActions { close_paths, tooltips: overrides, views } = *actions;
    let icons = RegistryIcons { resources };
    let mut out = Vec::new();
    for region in regions {
        let [rx, ry, width, height] = region.rect;
        let x0 = rx - origin[0];
        let y0 = ry - origin[1];
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let has_override = overrides.contains_key(&region.path);
        let element = roots
            .iter()
            .find_map(|root| element_by_path(root, &region.path));
        let through = region
            .hit
            .as_ref()
            .is_some_and(|geometry| geometry.form == Hit::Through)
            || element.is_some_and(element_always_transparent);
        if through && !has_override {
            continue;
        }
        let close = !through
            && close_paths.iter().any(|candidate| {
                region.path == *candidate
                    || region.path.ends_with(&format!(".{candidate}"))
                    || region.path.ends_with(&format!("/{candidate}"))
            });
        let alpha_mask = if !through
            && let Some(hit) = region.hit.as_ref()
        {
            match region_alpha_mask(hit, region.rect) {
                Some(mask) => mask,
                None => continue,
            }
        } else {
            None
        };
        let clip = region.clip.map(|[left, top, right, bottom]| {
            let cx = left as f32 - origin[0];
            let cy = top as f32 - origin[1];
            [
                cx,
                cy,
                (right as f32 - left as f32).max(0.0),
                (bottom as f32 - top as f32).max(0.0),
            ]
        });
        let tooltip = if let Some(custom) = overrides.get(&region.path) {
            Some(custom.clone())
        } else {
            element
                .and_then(|element| Authored::read(element, region.disabled))
                .and_then(|authored| {
                    let mut tip = Tooltip::default();
                    authored.fallback(localisation, &icons, &mut tip);
                    let title = strip_markup(&tip.title);
                    let body = strip_markup(&tip.body);
                    let detail = (!tip.refused.is_empty()).then(|| {
                        tip.refused
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                    if title.is_empty() && body.is_empty() {
                        None
                    } else if title.is_empty() {
                        Some(GameWindowTooltip {
                            title: body,
                            body: None,
                            detail,
                        })
                    } else {
                        Some(GameWindowTooltip {
                            title,
                            body: (!body.is_empty()).then_some(body),
                            detail,
                        })
                    }
                })
        };
        let view = (!through && !close).then(|| views.get(&region.path).cloned()).flatten();
        if close || tooltip.is_some() || view.is_some() {
            out.push(GameWindowRegion {
                path: region.path.clone(),
                rect: [x0, y0, width, height],
                clip,
                disabled: region.disabled,
                close,
                passthrough: through,
                view,
                tooltip,
                alpha_mask,
                scroll: None,
            });
        }
    }
    out
}

pub fn layout_window(
    root: &Element,
    resources: &dyn Resources,
    viewport: [i32; 2],
    configure: impl FnOnce(&mut Instance),
) -> Result<Layout, GameWindowError> {
    let mut instance = Instance::construct(root, viewport, resources)
        .map_err(GameWindowError::failed)?;
    instance.set_size_cornered_buttons(true);
    instance
        .gui_show(root, root)
        .map_err(GameWindowError::failed)?;
    configure(&mut instance);
    instance
        .tick(root, resources, SETTLE_TICK)
        .map_err(GameWindowError::failed)?;
    instance
        .tick(root, resources, STEADY_TICK)
        .map_err(GameWindowError::failed)?;
    let layout = instance
        .layout_with_regions(root, resources, None, true)
        .map_err(|refusals| GameWindowError::failed(format_refusals(&refusals)))?
        .ok_or_else(|| {
            GameWindowError::refused(format!(
                "Game window '{}' did not produce a layout.",
                root.path
            ))
        })?;
    Ok(layout)
}

fn paint_ordered_regions(
    part_paths: &[&str],
    main: Vec<GameWindowRegion>,
    mut subs: Vec<(usize, Vec<GameWindowRegion>)>,
) -> Vec<GameWindowRegion> {
    let mut last_part: HashMap<&str, usize> = HashMap::with_capacity(part_paths.len());
    for (index, path) in part_paths.iter().enumerate() {
        last_part.insert(path, index);
    }
    let mut floor = 0;
    let mut placed: Vec<(usize, usize, GameWindowRegion)> = Vec::with_capacity(main.len());
    for region in main {
        floor = last_part.get(region.path.as_str()).copied().unwrap_or(floor).max(floor);
        placed.push((floor, 1, region));
    }
    for (before, regions) in subs.drain(..) {
        let at = before.min(part_paths.len());
        placed.extend(regions.into_iter().map(|region| (at, 0, region)));
    }
    placed.sort_by_key(|(slot, rank, _)| (*slot, *rank));
    placed.into_iter().map(|(_, _, region)| region).collect()
}

fn format_refusals(refusals: &[native_gui::Refusal]) -> String {
    refusals
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RenderFrameKey {
    window: WindowCacheKey,
    scale_bits: u32,
    fingerprint: [u8; 32],
}

pub struct Prepared<S> {
    pub assets: Assets,
    pub state: S,
    pub fingerprint: Option<[u8; 32]>,
}

impl<S> Prepared<S> {
    pub fn uncached(assets: Assets, state: S) -> Self {
        Self {
            assets,
            state,
            fingerprint: None,
        }
    }
}

fn frame_fingerprint(
    window: [u8; 32],
    read_hour: Option<i32>,
    art_from_current_install: bool,
    warnings: &[String],
    scroll: &BTreeMap<String, f32>,
) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(window);
    hash.update(read_hour.map_or([0; 5], |hour| {
        let [a, b, c, d] = hour.to_le_bytes();
        [1, a, b, c, d]
    }));
    hash.update([u8::from(art_from_current_install)]);
    for warning in warnings {
        hash.update((warning.len() as u64).to_le_bytes());
        hash.update(warning.as_bytes());
    }
    for (list, position) in scroll {
        hash.update((list.len() as u64).to_le_bytes());
        hash.update(list.as_bytes());
        hash.update(position.to_bits().to_le_bytes());
    }
    hash.finalize().into()
}

struct RenderFrameCache {
    capacity: usize,
    entries: HashMap<RenderFrameKey, GameWindowRender>,
    order: VecDeque<RenderFrameKey>,
}

impl RenderFrameCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn touch(&mut self, key: &RenderFrameKey) {
        if let Some(pos) = self.order.iter().position(|existing| existing == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.clone());
    }

    fn get(&mut self, key: &RenderFrameKey) -> Option<GameWindowRender> {
        let value = self.entries.get(key)?.clone();
        self.touch(key);
        Some(value)
    }

    fn insert(&mut self, key: RenderFrameKey, value: GameWindowRender) {
        self.entries.insert(key.clone(), value);
        self.touch(&key);
        while self.entries.len() > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn evict_source(&mut self, source_id: &str) {
        self.order.retain(|key| key.window.source_id != source_id);
        self.entries.retain(|key, _| key.window.source_id != source_id);
    }

    fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodedFrameKey {
    source_id: String,
    revision: String,
    branch: u64,
    hour: i32,
    tables: Vec<TableId>,
}

struct DecodedFrames<T> {
    capacity: usize,
    entries: VecDeque<(DecodedFrameKey, Arc<T>)>,
}

impl<T> DecodedFrames<T> {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: VecDeque::new(),
        }
    }

    fn get(&mut self, key: &DecodedFrameKey) -> Option<Arc<T>> {
        let at = self.entries.iter().position(|(held, _)| held == key)?;
        let entry = self.entries.remove(at)?;
        let frame = Arc::clone(&entry.1);
        self.entries.push_back(entry);
        Some(frame)
    }

    fn insert(&mut self, key: DecodedFrameKey, frame: Arc<T>) {
        self.entries.retain(|(held, _)| *held != key);
        self.entries.push_back((key, frame));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    fn evict_source(&mut self, source_id: &str) {
        self.entries.retain(|(held, _)| held.source_id != source_id);
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

struct CacheState {
    interfaces: HashMap<InterfaceCacheKey, Arc<SharedInterface>>,
    windows: WindowCache<Arc<Mutex<CachedWindow>>>,
    renders: RenderFrameCache,
    frames: DecodedFrames<DecodedFrame>,
    source_generations: HashMap<String, u64>,
    global_generation: u64,
    cancels: HashMap<String, Arc<AtomicBool>>,
}

impl Default for CacheState {
    fn default() -> Self {
        Self {
            interfaces: HashMap::new(),
            windows: WindowCache::new(MAX_CACHED_WINDOWS),
            renders: RenderFrameCache::new(MAX_CACHED_RENDERS),
            frames: DecodedFrames::new(MAX_DECODED_FRAMES),
            source_generations: HashMap::new(),
            global_generation: 0,
            cancels: HashMap::new(),
        }
    }
}

pub struct CancelRegistration {
    service: Arc<Service>,
    request_id: String,
    token: Arc<AtomicBool>,
}

impl CancelRegistration {
    pub fn token(&self) -> &AtomicBool {
        &self.token
    }
}

impl Drop for CancelRegistration {
    fn drop(&mut self) {
        let mut cache = self
            .service
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache.cancels.remove(&self.request_id);
    }
}

pub struct RenderParams<'a> {
    pub window_name: &'a str,
    pub scale: f32,
    pub read_hour: Option<i32>,
    pub art_from_current_install: bool,
    pub close_paths: &'a [&'a str],
    pub tooltip_overrides: &'a BTreeMap<String, GameWindowTooltip>,
    pub extra_warnings: Vec<String>,
    pub admitted_gen: Option<(u64, u64)>,
    pub scroll: &'a BTreeMap<String, f32>,
}

struct WindowLoadRequest<'a> {
    key: &'a WindowCacheKey,
    layers: &'a ResolvedLayers,
    shared: &'a SharedInterface,
    window_name: &'a str,
    extra_assets: &'a Assets,
    expected_gen: (u64, u64),
}

#[derive(Default)]
pub struct Service {
    cache: Mutex<CacheState>,
    gpu: Mutex<Option<GpuContext>>,
}

impl Service {
    pub fn register_cancel(self: &Arc<Self>, request_id: String) -> CancelRegistration {
        let token = Arc::new(AtomicBool::new(false));
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.cancels.insert(request_id.clone(), Arc::clone(&token));
        CancelRegistration {
            service: Arc::clone(self),
            request_id,
            token,
        }
    }

    pub fn cancel_request(&self, request_id: &str) {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(token) = cache.cancels.get(request_id) {
            token.store(true, Ordering::SeqCst);
        }
    }

    pub fn evict_source(&self, source_id: &str) {
        let (dropped_interfaces, dropped_windows) = {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            *cache
                .source_generations
                .entry(source_id.to_owned())
                .or_default() += 1;
            let keys: Vec<InterfaceCacheKey> = cache
                .interfaces
                .keys()
                .filter(|key| key.source_id == source_id)
                .cloned()
                .collect();
            let interfaces: Vec<Arc<SharedInterface>> = keys
                .into_iter()
                .filter_map(|key| cache.interfaces.remove(&key))
                .collect();
            let windows = cache.windows.evict_source(source_id);
            cache.renders.evict_source(source_id);
            cache.frames.evict_source(source_id);
            (interfaces, windows)
        };
        drop(dropped_windows);
        drop(dropped_interfaces);
    }

    pub fn clear_cache(&self) {
        let (dropped_interfaces, dropped_windows) = {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.global_generation += 1;
            let interfaces: Vec<Arc<SharedInterface>> =
                cache.interfaces.drain().map(|(_, value)| value).collect();
            let windows = cache.windows.clear();
            cache.renders.clear();
            cache.frames.clear();
            (interfaces, windows)
        };
        drop(dropped_windows);
        drop(dropped_interfaces);
    }

    pub fn cache_len(&self) -> usize {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .windows
            .len()
    }

    pub fn capture_generation(&self, source_id: &str) -> (u64, u64) {
        self.generations(source_id)
    }

    pub fn admit<T, E>(
        &self,
        source_id: &str,
        acquire: impl FnOnce() -> Result<T, E>,
    ) -> Result<Option<(T, (u64, u64))>, E> {
        let generation = self.generations(source_id);
        let value = acquire()?;
        Ok(self
            .is_fresh(source_id, generation)
            .then_some((value, generation)))
    }

    pub fn cached_render_count(&self) -> usize {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .renders
            .len()
    }

    fn cached_render(
        &self,
        key: &RenderFrameKey,
        expected_gen: (u64, u64),
    ) -> Option<GameWindowRender> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let current = (
            cache
                .source_generations
                .get(&key.window.source_id)
                .copied()
                .unwrap_or(0),
            cache.global_generation,
        );
        if current != expected_gen {
            return None;
        }
        cache.renders.get(key)
    }

    fn store_render(
        &self,
        key: RenderFrameKey,
        expected_gen: (u64, u64),
        render: &GameWindowRender,
    ) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let current = (
            cache
                .source_generations
                .get(&key.window.source_id)
                .copied()
                .unwrap_or(0),
            cache.global_generation,
        );
        if current == expected_gen {
            cache.renders.insert(key, render.clone());
        }
    }

    pub fn frame_at(
        &self,
        source: &Source,
        request: &GameWindowRequest,
        admitted_gen: (u64, u64),
        branch: u64,
        tables: &[TableId],
    ) -> Result<Arc<DecodedFrame>, GameWindowError> {
        let key = DecodedFrameKey {
            source_id: request.source_id.clone(),
            revision: request.revision.clone(),
            branch,
            hour: request.hour,
            tables: tables.to_vec(),
        };
        if let Some(frame) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).frames.get(&key) {
            return Ok(frame);
        }
        let frame: Arc<DecodedFrame> = match source
            .replay_ref()
            .seek_selected(branch, request.hour, Some(tables))
            .map_err(|error| {
                GameWindowError::failed(format!(
                    "Could not seek recording to hour {}: {error}",
                    request.hour
                ))
            })? {
            SeekResult::Frame(frame) => Arc::from(frame),
            SeekResult::Gap { reason, .. } => {
                return Err(GameWindowError::refused(format!(
                    "Hour {} is not recorded on this branch: {reason}",
                    request.hour
                )));
            }
        };
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let current = (
            cache
                .source_generations
                .get(&request.source_id)
                .copied()
                .unwrap_or(0),
            cache.global_generation,
        );
        if current == admitted_gen {
            cache.frames.insert(key, Arc::clone(&frame));
        }
        Ok(frame)
    }

    fn generations(&self, source_id: &str) -> (u64, u64) {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        (
            cache
                .source_generations
                .get(source_id)
                .copied()
                .unwrap_or(0),
            cache.global_generation,
        )
    }

    fn is_fresh(&self, source_id: &str, expected: (u64, u64)) -> bool {
        self.generations(source_id) == expected
    }

    fn load_and_cache_window(
        &self,
        req: &WindowLoadRequest<'_>,
        cancel: &AtomicBool,
    ) -> Result<Arc<Mutex<CachedWindow>>, GameWindowError> {
        let decode = Decode {
            force_pow2_textures: req.key.force_pow2_textures,
        };
        let loaded = {
            let mut gpu_guard = self.gpu.lock().unwrap_or_else(|e| e.into_inner());
            if gpu_guard.is_none() {
                *gpu_guard = Some(GpuContext::initialize()?);
            }
            let gpu = gpu_guard.as_ref().expect("gpu initialized");
            CachedWindow::load(
                gpu,
                req.layers,
                req.shared,
                req.window_name,
                req.extra_assets,
                decode,
            )?
        };
        if cancel.load(Ordering::SeqCst) {
            return Err(GameWindowError::Cancelled);
        }
        let wrapped = Arc::new(Mutex::new(loaded));
        let evicted = {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let current_gen = (
                cache
                    .source_generations
                    .get(&req.key.source_id)
                    .copied()
                    .unwrap_or(0),
                cache.global_generation,
            );
            if current_gen == req.expected_gen {
                let windows = cache.windows.insert(req.key.clone(), Arc::clone(&wrapped));
                let state = &mut *cache;
                let interfaces = state
                    .windows
                    .drop_unused_interfaces(&mut state.interfaces, &req.key.interface_key());
                (windows, interfaces)
            } else {
                return Err(GameWindowError::Cancelled);
            }
        };
        drop(evicted);
        Ok(wrapped)
    }

    fn discard_poisoned_window(
        &self,
        key: &WindowCacheKey,
        poisoned: &Arc<Mutex<CachedWindow>>,
    ) {
        let removed = {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let same = cache
                .windows
                .entries
                .get(key)
                .is_some_and(|existing| Arc::ptr_eq(existing, poisoned));
            if same { cache.windows.remove(key) } else { None }
        };
        drop(removed);
    }

    pub fn render_composed_from_layers<S>(
        &self,
        key: WindowCacheKey,
        layers: &ResolvedLayers,
        params: RenderParams<'_>,
        cancel: &AtomicBool,
        prepare_assets: impl FnOnce(&SharedInterface) -> Result<Prepared<S>, GameWindowError>,
        configure_main: impl FnOnce(
            &Element,
            &native::Guis,
            &dyn Resources,
            &Localisation,
            &mut Instance,
            &mut BTreeMap<String, GameWindowTooltip>,
            &S,
        ),
        compose: impl FnOnce(
            &mut ComposedPlan,
            &Layout,
            &Element,
            &native::Guis,
            &dyn Resources,
            &Localisation,
            &S,
        ) -> Result<(), GameWindowError>,
    ) -> Result<GameWindowRender, GameWindowError> {
        if cancel.load(Ordering::SeqCst) {
            return Err(GameWindowError::Cancelled);
        }
        let RenderParams {
            window_name,
            scale,
            read_hour,
            art_from_current_install,
            close_paths,
            tooltip_overrides,
            extra_warnings,
            admitted_gen,
            scroll,
        } = params;
        let interface_key = key.interface_key();
        let (expected_gen, cached_interface, cached_window) = {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let current_gen = (
                cache
                    .source_generations
                    .get(&key.source_id)
                    .copied()
                    .unwrap_or(0),
                cache.global_generation,
            );
            if let Some(expected) = admitted_gen
                && expected != current_gen
            {
                return Err(GameWindowError::Cancelled);
            }
            let iface = cache.interfaces.get(&interface_key).map(Arc::clone);
            let win = cache.windows.get_mut(&key).map(|entry| Arc::clone(entry));
            (admitted_gen.unwrap_or(current_gen), iface, win)
        };

        let shared = match cached_interface {
            Some(shared) => shared,
            None => {
                let requested = (!key.language.is_empty()).then_some(key.language.as_str());
                let loaded = Arc::new(SharedInterface::load(layers, DEFAULT_VIEWPORT, requested)?);
                if cancel.load(Ordering::SeqCst) {
                    return Err(GameWindowError::Cancelled);
                }
                let replaced = {
                    let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                    let current_gen = (
                        cache
                            .source_generations
                            .get(&key.source_id)
                            .copied()
                            .unwrap_or(0),
                        cache.global_generation,
                    );
                    if current_gen != expected_gen {
                        return Err(GameWindowError::Cancelled);
                    }
                    cache
                        .interfaces
                        .insert(interface_key, Arc::clone(&loaded))
                };
                drop(replaced);
                loaded
            }
        };

        if shared.guis.template(window_name.as_bytes()).is_none() {
            return Err(GameWindowError::refused(missing_window_refusal(window_name)));
        }

        let Prepared {
            assets: extra_assets,
            state,
            fingerprint,
        } = prepare_assets(shared.as_ref())?;
        if cancel.load(Ordering::SeqCst) {
            return Err(GameWindowError::Cancelled);
        }
        let frame_key = fingerprint.map(|window| RenderFrameKey {
            window: key.clone(),
            scale_bits: normalize_scale(scale).to_bits(),
            fingerprint: frame_fingerprint(
                window,
                read_hour,
                art_from_current_install,
                &extra_warnings,
                scroll,
            ),
        });
        if let Some(frame_key) = &frame_key
            && let Some(hit) = self.cached_render(frame_key, expected_gen)
        {
            return Ok(hit);
        }

        let load_req = WindowLoadRequest {
            key: &key,
            layers,
            shared: &shared,
            window_name,
            extra_assets: &extra_assets,
            expected_gen,
        };
        let window_entry = match cached_window {
            Some(existing) => existing,
            None => self.load_and_cache_window(&load_req, cancel)?,
        };

        let scale = normalize_scale(scale);
        let physical_viewport = [
            (DEFAULT_VIEWPORT[0] as f32 * scale).round() as i32,
            (DEFAULT_VIEWPORT[1] as f32 * scale).round() as i32,
        ];

        let (output, regions, warnings, title, effective_read_hour, effective_art_label, text) = {
            let reloaded_entry: Option<Arc<Mutex<CachedWindow>>>;
            let mut cached = match window_entry.lock() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    drop(poisoned);
                    self.discard_poisoned_window(&key, &window_entry);
                    reloaded_entry = Some(self.load_and_cache_window(&load_req, cancel)?);
                    reloaded_entry
                        .as_ref()
                        .expect("reloaded window entry")
                        .lock()
                        .map_err(|_| {
                            GameWindowError::failed(
                                "Game window renderer state became poisoned after reload.",
                            )
                        })?
                }
            };
            if !extra_assets.sprites.is_empty()
                || !extra_assets.fonts.is_empty()
                || !extra_assets.textures.is_empty()
            {
                let mut gpu_guard = self.gpu.lock().unwrap_or_else(|e| e.into_inner());
                if gpu_guard.is_none() {
                    *gpu_guard = Some(GpuContext::initialize()?);
                }
                let gpu = gpu_guard.as_ref().expect("gpu initialized");
                cached.ensure_assets(gpu, layers, &extra_assets)?;
            }
            if cancel.load(Ordering::SeqCst) {
                return Err(GameWindowError::Cancelled);
            }
            let guis = Arc::clone(&cached.guis);
            let root = guis
                .template(window_name.as_bytes())
                .ok_or_else(|| GameWindowError::refused(missing_window_refusal(window_name)))?;
            let assets = cached.gui_assets(physical_viewport);
            let mut instance = Instance::construct(
                root,
                DEFAULT_VIEWPORT.map(|value| value as i32),
                &assets,
            )
            .map_err(GameWindowError::failed)?;
            instance.set_size_cornered_buttons(true);
            instance
                .gui_show(root, root)
                .map_err(GameWindowError::failed)?;
            let mut main_tooltips = tooltip_overrides.clone();
            configure_main(
                root,
                cached.guis.as_ref(),
                &assets,
                cached.localisation.as_ref(),
                &mut instance,
                &mut main_tooltips,
                &state,
            );
            instance
                .tick(root, &assets, SETTLE_TICK)
                .map_err(GameWindowError::failed)?;
            instance
                .tick(root, &assets, STEADY_TICK)
                .map_err(GameWindowError::failed)?;
            let layout = instance
                .layout_with_regions(root, &assets, None, true)
                .map_err(|refusals| GameWindowError::failed(format_refusals(&refusals)))?
                .ok_or_else(|| {
                    GameWindowError::refused(format!(
                        "Game window '{window_name}' did not produce a layout."
                    ))
                })?;
            let mut plan = ComposedPlan::scrolled(scroll.clone());
            compose(
                &mut plan,
                &layout,
                root,
                cached.guis.as_ref(),
                &assets,
                cached.localisation.as_ref(),
                &state,
            )?;
            let mut scrollbars = std::mem::take(&mut plan.scrollbars);
            plan.sub_windows.append(&mut scrollbars);
            if cancel.load(Ordering::SeqCst) {
                return Err(GameWindowError::Cancelled);
            }
            let output = {
                let mut gpu_guard = self.gpu.lock().unwrap_or_else(|e| e.into_inner());
                if gpu_guard.is_none() {
                    *gpu_guard = Some(GpuContext::initialize()?);
                }
                let gpu = gpu_guard.as_ref().expect("gpu initialized");
                let CachedWindow {
                    guis,
                    gui,
                    localisation,
                    pass,
                    ..
                } = &mut *cached;
                let draw_assets = WindowGuiAssets {
                    gui,
                    guis,
                    localisation,
                    physical_viewport: Some(physical_viewport),
                };
                if plan.sub_windows.is_empty() {
                    render_layout_offscreen(gpu, pass, &layout, &draw_assets, scale)?
                } else {
                    render_composed_layout_offscreen(
                        gpu,
                        pass,
                        &layout,
                        &plan.sub_windows,
                        &draw_assets,
                        scale,
                    )?
                }
            };
            let assets = cached.gui_assets(physical_viewport);
            let main_regions = regions_from_layout(
                &[root],
                &layout.regions,
                output.origin,
                &assets,
                cached.localisation.as_ref(),
                &RegionActions { close_paths, tooltips: &main_tooltips, views: &plan.view_links },
            );
            let sub_region_groups: Vec<(usize, Vec<GameWindowRegion>)> = plan
                .sub_windows
                .iter()
                .map(|sub| {
                    let sub_roots: Vec<&Element> = cached
                        .guis
                        .template(sub.root_window.as_bytes())
                        .into_iter()
                        .collect();
                    let sub_close: Vec<&str> =
                        sub.close_paths.iter().map(String::as_str).collect();
                    let mut sub_regions = regions_from_layout(
                        &sub_roots,
                        &sub.regions,
                        output.origin,
                        &assets,
                        cached.localisation.as_ref(),
                        &RegionActions { close_paths: &sub_close, tooltips: &sub.tooltip_overrides, views: &BTreeMap::new() },
                    );
                    if !sub.path_prefix.is_empty() {
                        for region in &mut sub_regions {
                            region.path = format!("{}{}", sub.path_prefix, region.path);
                        }
                    }
                    (sub.before_part, sub_regions)
                })
                .collect();
            let part_paths: Vec<&str> = layout.parts.iter().map(|part| part.path.as_str()).collect();
            let mut regions = paint_ordered_regions(&part_paths, main_regions, sub_region_groups);
            for mut region in plan.extra_regions {
                region.rect[0] -= output.origin[0];
                region.rect[1] -= output.origin[1];
                if let Some(clip) = &mut region.clip {
                    clip[0] -= output.origin[0];
                    clip[1] -= output.origin[1];
                }
                regions.push(region);
            }
            let mut warnings = extra_warnings;
            warnings.extend(cached.warnings.iter().cloned());
            warnings.extend(plan.warnings);
            for diagnostic in &layout.diagnostics {
                warnings.push(format!(
                    "{window_name} ({}): {}",
                    diagnostic.path, diagnostic.detail
                ));
            }
            for sub in &plan.sub_windows {
                for diagnostic in &sub.diagnostics {
                    warnings.push(format!(
                        "{} ({}): {}",
                        sub.root_window, diagnostic.path, diagnostic.detail
                    ));
                }
            }
            (
                output,
                regions,
                warnings,
                plan.title,
                plan.read_hour.or(read_hour),
                plan.art_label,
                plan.text,
            )
        };

        if cancel.load(Ordering::SeqCst) {
            return Err(GameWindowError::Cancelled);
        }
        if !self.is_fresh(&key.source_id, expected_gen) {
            return Err(GameWindowError::Cancelled);
        }

        let render = GameWindowRender {
            title,
            png_base64: encode_base64(&output.png),
            logical_size: output.logical_size,
            pixel_size: output.pixel_size,
            scale,
            read_hour: effective_read_hour,
            art_from_current_install,
            art_label: effective_art_label.or_else(|| {
                art_from_current_install.then(|| "Artwork from current installation".to_owned())
            }),
            regions,
            warnings,
            text,
        };
        if let Some(frame_key) = frame_key {
            self.store_render(frame_key, expected_gen, &render);
        }
        Ok(render)
    }

    pub fn render_from_source_at_generation(
        &self,
        source: &Source,
        config: &Config,
        visual_mods: &[content_reader_layers::content::visual::Choice],
        request: GameWindowRequest,
        admitted_gen: (u64, u64),
        cancel: &AtomicBool,
    ) -> Result<GameWindowRender, GameWindowError> {
        if cancel.load(Ordering::SeqCst) {
            return Err(GameWindowError::Cancelled);
        }
        if !self.is_fresh(&request.source_id, admitted_gen) {
            return Err(GameWindowError::Cancelled);
        }
        let resolve = config.resolve_options().map_err(|error| {
            GameWindowError::refused(format!(
                "Could not open game or user content folders: {error}"
            ))
        })?;
        let content = source.content(&resolve).map_err(|error| {
            GameWindowError::refused(format!("Could not resolve recorded content: {error}"))
        })?;
        let mut layers = content.layers.clone();
        let mut visual_revision = String::new();
        let mut visual_warnings = Vec::new();
        let decode = match content_reader_settings::settings::read_keys(&resolve.user_dir) {
            Ok(keys) => keys.texture_decode(),
            Err(error) => {
                visual_warnings.push(format!(
                    "settings.txt is unavailable ({error}); using default texture decode settings."
                ));
                Decode::default()
            }
        };
        if !visual_mods.is_empty() {
            let recorded: Vec<_> = source
                .replay_ref()
                .identity
                .mods
                .iter()
                .map(|entry| content_reader_layers::content::visual::RecordedMod {
                    id: entry.id.clone(),
                    name: entry.display_name.clone(),
                })
                .collect();
            let plan = content_reader_layers::content::visual::apply(
                &mut layers,
                &resolve,
                visual_mods,
                &recorded,
                cancel,
            )
            .map_err(|error| {
                if cancel.load(Ordering::SeqCst) {
                    GameWindowError::Cancelled
                } else {
                    GameWindowError::failed(format!("Could not apply visual mods: {error}"))
                }
            })?;
            visual_revision = plan.revision;
            visual_warnings.extend(plan.warnings);
        }
        if !self.is_fresh(&request.source_id, admitted_gen) {
            return Err(GameWindowError::Cancelled);
        }
        let language = dashboard_base::content::recorded_language(source.replay_ref());
        match request.kind.as_str() {
            "equipmentDesigner" => equipment_designer::render(
                self,
                equipment_designer::RenderContext {
                    source,
                    content: &content,
                    layers: &layers,
                    resolve: &resolve,
                    visual_revision: &visual_revision,
                    language: &language,
                    decode,
                    admitted_gen,
                    extra_warnings: visual_warnings,
                },
                request,
                cancel,
            ),
            "industrialOrganisation" => industrial_organisation::render(
                self,
                equipment_designer::RenderContext {
                    source,
                    content: &content,
                    layers: &layers,
                    resolve: &resolve,
                    visual_revision: &visual_revision,
                    language: &language,
                    decode,
                    admitted_gen,
                    extra_warnings: visual_warnings,
                },
                request,
                cancel,
            ),
            other => Err(GameWindowError::refused(unsupported_window_kind_refusal(
                other,
            ))),
        }
    }
}

pub fn load_native_guis_from_bytes(name: &str, bytes: &[u8]) -> io::Result<native::Guis> {
    Ok(native::Guis::new(vec![(
        name.to_owned(),
        native::read_at(bytes, DEFAULT_VIEWPORT.map(|value| value as i32)),
    )]))
}
