use content_reader_gui::gui::native::{self, Element, Fields, resources};
use content_reader_image::{dds, native::Decode};
use content_reader_layers::ResolvedLayers;
use content_reader_localisation::localisation::{self, Localisation, Piece, pieces};
use dashboard_base::Source;
use dashboard_core::config::Config;
use eframe::wgpu;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use viewer_foundation::tooltip_text::{TextIcons, Tooltip};
use viewer_gpu::{
    gpu_contract::{OUTPUT_FORMAT, device_descriptor, read_texture, select_adapter},
    ui_scale::valid as valid_ui_scale,
};
use viewer_native_gui::{
    counter_hovers::gui_authored::Authored,
    gui_resources::GuiAssets,
    lazy_icons::LazyTextures,
    native_gui::{
        self, Assets, Layout, Region, Resources, Sheets,
        lifecycle::{Instance, Tick},
        part_rect,
    },
    native_resources::GuiResources,
    native_windows::NativeWindows,
    sprites::{Batch, IconTextures, SpritePass},
};

#[cfg(test)]
mod tests;

pub mod equipment_designer;

pub const STEADY_TICK: Tick = Tick {
    time: 10.0,
    dt: 10.0,
};
pub const DEFAULT_VIEWPORT: [u32; 2] = [2752, 1152];
const MAX_CACHED_WINDOWS: usize = 8;
const MAX_SURFACE_DIM: u32 = 4096;

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
    pub scroll: BTreeMap<String, f32>,
    #[serde(default)]
    pub sub_view: Option<String>,
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
pub struct GameWindowScrollRegion {
    pub target: String,
    pub vertical: bool,
    pub wheel_factor: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowRegion {
    pub path: String,
    pub rect: [f32; 4],
    pub clip: Option<[f32; 4]>,
    pub disabled: bool,
    pub close: bool,
    pub scroll: Option<GameWindowScrollRegion>,
    pub tooltip: Option<GameWindowTooltip>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowRender {
    pub png_base64: String,
    pub logical_size: [u32; 2],
    pub pixel_size: [u32; 2],
    pub scale: f32,
    pub read_hour: Option<i32>,
    pub art_from_current_install: bool,
    pub art_label: Option<String>,
    pub regions: Vec<GameWindowRegion>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceCacheKey {
    pub source_id: String,
    pub revision: String,
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub visual_revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowCacheKey {
    pub source_id: String,
    pub revision: String,
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub visual_revision: String,
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

    pub fn insert(&mut self, key: WindowCacheKey, value: V) {
        if self.entries.contains_key(&key) {
            self.entries.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        while self.entries.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    pub fn evict_source(&mut self, source_id: &str) {
        self.entries.retain(|key, _| key.source_id != source_id);
        self.order.retain(|key| key.source_id != source_id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
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
    decode: Decode,
}

impl GpuContext {
    pub fn initialize() -> Result<Self, String> {
        let instance = shader_pipeline::gpu::instance();
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let adapter = select_adapter(&adapters, None).map_err(|error| error.to_string())?;
        let descriptor = device_descriptor(&adapter);
        let decode = Decode::default();
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor))
            .map_err(|error| format!("Could not open graphics device for game window: {error}"))?;
        let arena = shader_pipeline::gpu::Arena::new(&device);
        Ok(Self {
            device,
            queue,
            arena,
            decode,
        })
    }

    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let decode = Decode::default();
        let arena = shader_pipeline::gpu::Arena::new(&device);
        Self {
            device,
            queue,
            arena,
            decode,
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

    pub fn decode(&self) -> Decode {
        self.decode
    }
}

pub struct SharedInterface {
    pub windows: Arc<NativeWindows>,
    pub localisation: Arc<Localisation>,
    pub language: String,
    pub registry: resources::Registry,
    pub recorded_registry: Option<resources::Registry>,
}

impl SharedInterface {
    pub fn load(layers: &ResolvedLayers, viewport: [u32; 2]) -> Result<Self, String> {
        let windows = Arc::new(
            NativeWindows::load_at(layers, viewport)
                .map_err(|error| format!("Could not load interface definitions: {error}"))?,
        );
        let language = localisation::selected_language(layers, None)
            .unwrap_or_else(|_| "english".to_owned());
        let localisation = Arc::new(
            localisation::read(layers, &language)
                .map_err(|error| format!("Could not load game localisation: {error}"))?,
        );
        let registry = resources::Registry::load(layers)
            .map_err(|error| format!("Could not load interface sprite registry: {error}"))?;
        let recorded_registry = layers
            .without_overlays()
            .and_then(|recorded| resources::Registry::load(&recorded).ok());
        Ok(Self {
            windows,
            localisation,
            language,
            registry,
            recorded_registry,
        })
    }
}

pub struct CachedWindow {
    pub windows: Arc<NativeWindows>,
    pub gui: GuiResources,
    pub localisation: Arc<Localisation>,
    pub images: Vec<dds::Texture>,
    pub textures: IconTextures,
    pub lazy: Arc<LazyTextures>,
    pub pass: SpritePass,
    pub warnings: Vec<String>,
}

impl CachedWindow {
    pub fn load(
        gpu: &GpuContext,
        layers: &ResolvedLayers,
        shared: &SharedInterface,
        window_name: &str,
        extra_assets: &Assets,
    ) -> Result<Self, String> {
        let mut warnings = Vec::new();
        let windows = Arc::clone(&shared.windows);
        let Some(root) = windows.gui_type(window_name.as_bytes()) else {
            return Err(missing_window_refusal(window_name));
        };
        let localisation = Arc::clone(&shared.localisation);
        let font_language = format!("l_{}", shared.language);
        let mut gui = GuiResources::new(
            shared.registry.clone(),
            font_language.as_bytes(),
            [1, 1],
            gpu.decode(),
        );
        if let Some(recorded_registry) = &shared.recorded_registry {
            gui = gui.with_recorded_registry(recorded_registry.clone());
        }
        let mut assets = native_gui::assets_in(
            root,
            &|name| windows.gui_type(name),
            localisation.as_ref(),
        );
        assets.sprites.extend(extra_assets.sprites.iter().cloned());
        assets.fonts.extend(extra_assets.fonts.iter().cloned());
        assets
            .textures
            .extend(extra_assets.textures.iter().cloned());
        let mut images = Vec::new();
        gui.load(layers, &assets, &mut images)
            .map_err(|error| format!("Could not load assets for '{window_name}': {error}"))?;
        if let Some(note) = gui.uncovered_note(window_name) {
            warnings.push(note);
        }
        let textures = IconTextures::new(&gpu.device, &gpu.queue, &images)
            .map_err(|error| format!("Could not upload textures for '{window_name}': {error}"))?;
        let lazy = LazyTextures::new();
        let mut pass = SpritePass::new(&gpu.device, &gpu.queue, &textures, &lazy, OUTPUT_FORMAT)
            .map_err(|error| format!("Could not prepare renderer for '{window_name}': {error}"))?;
        pass.warm(&gpu.device, gui.effects());
        Ok(Self {
            windows,
            gui,
            localisation,
            images,
            textures,
            lazy,
            pass,
            warnings,
        })
    }

    pub fn ensure_assets(
        &mut self,
        gpu: &GpuContext,
        layers: &ResolvedLayers,
        extra: &Assets,
    ) -> Result<(), String> {
        let before = self.images.len();
        self.gui
            .load(layers, extra, &mut self.images)
            .map_err(|error| format!("Could not load extra window assets: {error}"))?;
        if self.images.len() > before {
            self.textures = IconTextures::new(&gpu.device, &gpu.queue, &self.images)
                .map_err(|error| format!("Could not upload extra window textures: {error}"))?;
            self.pass = SpritePass::new(
                &gpu.device,
                &gpu.queue,
                &self.textures,
                &self.lazy,
                OUTPUT_FORMAT,
            )
            .map_err(|error| format!("Could not rebuild renderer for extra assets: {error}"))?;
            self.pass.warm(&gpu.device, self.gui.effects());
        }
        Ok(())
    }

    pub fn gui_assets(&self, physical_viewport: [i32; 2]) -> GuiAssets<'_> {
        GuiAssets {
            gui: &self.gui,
            windows: &self.windows,
            localisation: &self.localisation,
            flags: None,
            physical_viewport: Some(physical_viewport),
            portrait_textures: None,
        }
    }
}

pub struct OffscreenOutput {
    pub rgba: Vec<u8>,
    pub png: Vec<u8>,
    pub logical_size: [u32; 2],
    pub pixel_size: [u32; 2],
    pub origin: [i32; 2],
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

pub fn drawn_bounds(layout: &Layout) -> [i32; 4] {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for part in &layout.parts {
        let Some([x, y, w, h]) = part_rect(&part.kind) else {
            continue;
        };
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let mut x0 = x;
        let mut y0 = y;
        let mut x1 = x + w;
        let mut y1 = y + h;
        if let Some([left, top, right, bottom]) = part.clip {
            x0 = x0.max(left as f32);
            y0 = y0.max(top as f32);
            x1 = x1.min(right as f32);
            y1 = y1.min(bottom as f32);
        }
        if x1 > x0 && y1 > y0 {
            min_x = min_x.min(x0);
            min_y = min_y.min(y0);
            max_x = max_x.max(x1);
            max_y = max_y.max(y1);
        }
    }
    if !min_x.is_finite() || !min_y.is_finite() || max_x <= min_x || max_y <= min_y {
        return [0, 0, layout.size[0].max(1), layout.size[1].max(1)];
    }
    let left = min_x.floor() as i32;
    let top = min_y.floor() as i32;
    let width = (max_x.ceil() as i32 - left).max(1);
    let height = (max_y.ceil() as i32 - top).max(1);
    [left, top, width, height]
}

pub struct ComposedSubWindow {
    pub before_part: usize,
    pub parts: Vec<native_gui::Part>,
    pub regions: Vec<Region>,
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
}

pub fn container_holder(layout: &Layout, container_path: &str) -> Option<native_gui::Holder> {
    let region = layout
        .regions
        .iter()
        .find(|region| region.path == container_path)?;
    let prefix = format!("{container_path}/");
    let before = layout
        .parts
        .iter()
        .enumerate()
        .rev()
        .find(|(_, part)| part.path == container_path || part.path.starts_with(&prefix))
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
    Layout {
        size: main_layout.size,
        parts,
        holders: Vec::new(),
        regions: Vec::new(),
        diagnostics: Vec::new(),
        pointed: None,
        hovered: None,
        target: None,
        boxes: Vec::new(),
    }
}

pub fn render_composed_layout_offscreen(
    gpu: &GpuContext,
    pass: &mut SpritePass,
    main_layout: &Layout,
    sub_windows: &[ComposedSubWindow],
    sheets: &dyn Sheets,
    scale: f32,
) -> Result<OffscreenOutput, String> {
    let merged = merge_composed_layout(main_layout, sub_windows);
    render_layout_offscreen(gpu, pass, &merged, sheets, scale)
}

pub fn render_layout_offscreen(
    gpu: &GpuContext,
    pass: &mut SpritePass,
    layout: &Layout,
    sheets: &dyn Sheets,
    scale: f32,
) -> Result<OffscreenOutput, String> {
    let [min_x, min_y, width, height] = drawn_bounds(layout);
    let mut batch = Batch::default();
    native_gui::draw(layout, [-min_x, -min_y], sheets, &mut batch)
        .map_err(|refusal| format!("Could not draw game window: {refusal}"))?;
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
) -> Result<OffscreenOutput, String> {
    let scale = normalize_scale(scale);
    let pixel_size = [
        ((logical_size[0] as f32 * scale).round() as u32).clamp(1, MAX_SURFACE_DIM),
        ((logical_size[1] as f32 * scale).round() as u32).clamp(1, MAX_SURFACE_DIM),
    ];
    pass.upload(
        &gpu.device,
        &gpu.queue,
        &gpu.arena,
        batch,
        pixel_size,
        scale,
    );
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
    pass.draw_into(
        &mut encoder,
        &view,
        pixel_size,
        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
    );
    let mut submissions = pass.bake_buffers();
    submissions.push(encoder.finish());
    gpu.queue.submit(submissions);
    let rgba = read_texture(&gpu.device, &gpu.queue, &target, 4)
        .map_err(|error| format!("Could not read back rendered game window: {error}"))?;
    let png = encode_png(pixel_size[0], pixel_size[1], &rgba)?;
    Ok(OffscreenOutput {
        rgba,
        png,
        logical_size,
        pixel_size,
        origin,
    })
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("Could not start game window PNG: {error}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| format!("Could not write game window PNG: {error}"))?;
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
        let mut sprite = Vec::with_capacity(4 + name.len());
        sprite.extend_from_slice(b"GFX_");
        sprite.extend_from_slice(name.as_bytes());
        let info = self.resources.sprite_info(&sprite)?;
        Some((info.frames.max(1) as u32, 0))
    }
}

pub fn element_by_path<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    if root.path == path {
        return Some(root);
    }
    for child in &root.children {
        if let Some(found) = element_by_path(child, path) {
            return Some(found);
        }
    }
    if let Fields::DropDown(dd) = &root.fields {
        if let Some(expand) = &dd.expand_button
            && let Some(found) = element_by_path(expand, path)
        {
            return Some(found);
        }
        if let Some(found) = element_by_path(&dd.inner, path) {
            return Some(found);
        }
    }
    None
}

pub fn regions_from_layout(
    roots: &[&Element],
    regions: &[Region],
    origin: [i32; 2],
    resources: &dyn Resources,
    localisation: &Localisation,
    close_paths: &[&str],
    overrides: &BTreeMap<String, GameWindowTooltip>,
) -> Vec<GameWindowRegion> {
    let icons = RegistryIcons { resources };
    let mut out = Vec::new();
    for region in regions {
        let [rx, ry, width, height] = region.rect;
        let x0 = rx - origin[0] as f32;
        let y0 = ry - origin[1] as f32;
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let clip = region.clip.map(|[left, top, right, bottom]| {
            let cx = left as f32 - origin[0] as f32;
            let cy = top as f32 - origin[1] as f32;
            [
                cx,
                cy,
                (right as f32 - left as f32).max(0.0),
                (bottom as f32 - top as f32).max(0.0),
            ]
        });
        let close = close_paths.iter().any(|candidate| {
            region.path == *candidate
                || region.path.ends_with(&format!(".{candidate}"))
                || region.path.ends_with(&format!("/{candidate}"))
        });
        let scroll = region.scroll.map(|(vertical, wheel_factor)| {
            let target = region
                .owner
                .clone()
                .unwrap_or_else(|| region.path.clone());
            GameWindowScrollRegion {
                target,
                vertical,
                wheel_factor,
            }
        });
        let tooltip = if let Some(custom) = overrides.get(&region.path) {
            Some(custom.clone())
        } else {
            roots
                .iter()
                .find_map(|root| element_by_path(root, &region.path))
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
        if close || scroll.is_some() || tooltip.is_some() {
            out.push(GameWindowRegion {
                path: region.path.clone(),
                rect: [x0, y0, width, height],
                clip,
                disabled: region.disabled,
                close,
                scroll,
                tooltip,
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
) -> Result<Layout, String> {
    let mut instance = Instance::construct(root, viewport, resources)?;
    instance
        .gui_show(root, root)
        .map_err(|refusal| refusal.to_string())?;
    configure(&mut instance);
    instance.tick(root, resources, STEADY_TICK)?;
    let layout = instance
        .layout_with_regions(root, resources, None, true)
        .map_err(|refusals| format_refusals(&refusals))?
        .ok_or_else(|| format!("Game window '{}' did not produce a layout.", root.path))?;
    Ok(layout)
}

fn format_refusals(refusals: &[native_gui::Refusal]) -> String {
    refusals
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

struct Inner {
    gpu: Option<GpuContext>,
    interfaces: HashMap<InterfaceCacheKey, SharedInterface>,
    cache: WindowCache<CachedWindow>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            gpu: None,
            interfaces: HashMap::new(),
            cache: WindowCache::new(MAX_CACHED_WINDOWS),
        }
    }
}

#[derive(Default)]
pub struct Service {
    inner: Mutex<Inner>,
}

impl Service {
    pub fn evict_source(&self, source_id: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .interfaces
            .retain(|key, _| key.source_id != source_id);
        inner.cache.evict_source(source_id);
    }

    pub fn clear_cache(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.interfaces.clear();
        inner.cache.clear();
    }

    pub fn cache_len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cache
            .len()
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the window cache key, layers, and per-render configuration are independent inputs"
    )]
    pub fn render_window_from_layers(
        &self,
        key: WindowCacheKey,
        layers: &ResolvedLayers,
        window_name: &str,
        extra_assets: &Assets,
        scale: f32,
        read_hour: Option<i32>,
        art_from_current_install: bool,
        close_paths: &[&str],
        tooltip_overrides: &BTreeMap<String, GameWindowTooltip>,
        configure: impl FnOnce(&Element, &Localisation, &mut Instance),
    ) -> Result<GameWindowRender, String> {
        let extra = extra_assets.clone();
        self.render_composed_from_layers(
            key,
            layers,
            window_name,
            scale,
            read_hour,
            art_from_current_install,
            close_paths,
            tooltip_overrides,
            move |_, _, _| Ok(extra),
            move |root, _, localisation, instance, _| configure(root, localisation, instance),
            |_, _, _, _, _| Ok(ComposedPlan::default()),
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the window cache key, layers, and per-render callbacks are independent inputs"
    )]
    pub fn render_composed_from_layers(
        &self,
        key: WindowCacheKey,
        layers: &ResolvedLayers,
        window_name: &str,
        scale: f32,
        read_hour: Option<i32>,
        art_from_current_install: bool,
        close_paths: &[&str],
        tooltip_overrides: &BTreeMap<String, GameWindowTooltip>,
        prepare_assets: impl FnOnce(
            &NativeWindows,
            &resources::Registry,
            &Localisation,
        ) -> Result<Assets, String>,
        configure_main: impl FnOnce(
            &Element,
            &NativeWindows,
            &Localisation,
            &mut Instance,
            &mut BTreeMap<String, GameWindowTooltip>,
        ),
        compose: impl FnOnce(
            &Layout,
            &Element,
            &NativeWindows,
            &dyn Resources,
            &Localisation,
        ) -> Result<ComposedPlan, String>,
    ) -> Result<GameWindowRender, String> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.gpu.is_none() {
            inner.gpu = Some(GpuContext::initialize()?);
        }
        let Inner {
            gpu,
            interfaces,
            cache,
        } = &mut *inner;
        let gpu = gpu.as_ref().expect("gpu initialized");
        let interface_key = key.interface_key();
        if !interfaces.contains_key(&interface_key) {
            let shared = SharedInterface::load(layers, DEFAULT_VIEWPORT)?;
            interfaces.insert(interface_key.clone(), shared);
        }
        let shared = interfaces.get(&interface_key).expect("interface cached");
        let extra_assets = prepare_assets(
            shared.windows.as_ref(),
            &shared.registry,
            shared.localisation.as_ref(),
        )?;
        if cache.get_mut(&key).is_none() {
            let loaded = CachedWindow::load(gpu, layers, shared, window_name, &extra_assets)?;
            cache.insert(key.clone(), loaded);
        }
        let cached = cache.get_mut(&key).expect("window cached");
        if !extra_assets.sprites.is_empty()
            || !extra_assets.fonts.is_empty()
            || !extra_assets.textures.is_empty()
        {
            cached.ensure_assets(gpu, layers, &extra_assets)?;
        }
        let scale = normalize_scale(scale);
        let physical_viewport = [
            (DEFAULT_VIEWPORT[0] as f32 * scale).round() as i32,
            (DEFAULT_VIEWPORT[1] as f32 * scale).round() as i32,
        ];
        let root = cached
            .windows
            .gui_type(window_name.as_bytes())
            .ok_or_else(|| missing_window_refusal(window_name))?;
        let assets = GuiAssets {
            gui: &cached.gui,
            windows: &cached.windows,
            localisation: &cached.localisation,
            flags: None,
            physical_viewport: Some(physical_viewport),
            portrait_textures: None,
        };
        let mut instance = Instance::construct(
            root,
            DEFAULT_VIEWPORT.map(|value| value as i32),
            &assets,
        )?;
        instance
            .gui_show(root, root)
            .map_err(|refusal| refusal.to_string())?;
        let mut main_tooltips = tooltip_overrides.clone();
        configure_main(
            root,
            cached.windows.as_ref(),
            cached.localisation.as_ref(),
            &mut instance,
            &mut main_tooltips,
        );
        instance.tick(root, &assets, STEADY_TICK)?;
        let layout = instance
            .layout_with_regions(root, &assets, None, true)
            .map_err(|refusals| format_refusals(&refusals))?
            .ok_or_else(|| format!("Game window '{window_name}' did not produce a layout."))?;
        let plan = compose(
            &layout,
            root,
            cached.windows.as_ref(),
            &assets,
            cached.localisation.as_ref(),
        )?;
        let output = if plan.sub_windows.is_empty() {
            render_layout_offscreen(gpu, &mut cached.pass, &layout, &assets, scale)?
        } else {
            render_composed_layout_offscreen(
                gpu,
                &mut cached.pass,
                &layout,
                &plan.sub_windows,
                &assets,
                scale,
            )?
        };
        let mut regions = regions_from_layout(
            &[root],
            &layout.regions,
            output.origin,
            &assets,
            cached.localisation.as_ref(),
            close_paths,
            &main_tooltips,
        );
        for sub in &plan.sub_windows {
            let sub_roots: Vec<&Element> = cached
                .windows
                .gui_type(sub.root_window.as_bytes())
                .into_iter()
                .collect();
            let sub_close: Vec<&str> = sub.close_paths.iter().map(String::as_str).collect();
            let mut sub_regions = regions_from_layout(
                &sub_roots,
                &sub.regions,
                output.origin,
                &assets,
                cached.localisation.as_ref(),
                &sub_close,
                &sub.tooltip_overrides,
            );
            if !sub.path_prefix.is_empty() {
                for region in &mut sub_regions {
                    region.path = format!("{}{}", sub.path_prefix, region.path);
                }
            }
            regions.extend(sub_regions);
        }
        for mut region in plan.extra_regions {
            region.rect[0] -= output.origin[0] as f32;
            region.rect[1] -= output.origin[1] as f32;
            if let Some(clip) = &mut region.clip {
                clip[0] -= output.origin[0] as f32;
                clip[1] -= output.origin[1] as f32;
            }
            regions.push(region);
        }
        let mut warnings = cached.warnings.clone();
        warnings.extend(plan.warnings);
        for diagnostic in &layout.diagnostics {
            warnings.push(format!(
                "{window_name} ({}): {}",
                diagnostic.path, diagnostic.detail
            ));
        }
        Ok(GameWindowRender {
            png_base64: encode_base64(&output.png),
            logical_size: output.logical_size,
            pixel_size: output.pixel_size,
            scale,
            read_hour,
            art_from_current_install,
            art_label: art_from_current_install
                .then(|| "Artwork from current installation".to_owned()),
            regions,
            warnings,
        })
    }

    pub fn render_from_source(
        &self,
        source: &Source,
        config: &Config,
        visual_mods: &[content_reader_layers::content::visual::Choice],
        request: GameWindowRequest,
    ) -> Result<GameWindowRender, String> {
        let resolve = config.resolve_options().map_err(|_| {
            "Set your game installation folder in Settings to display game windows.".to_owned()
        })?;
        let content = source
            .content(&resolve)
            .map_err(|error| format!("Could not resolve recorded content: {error}"))?;
        let mut layers = content.layers.clone();
        let mut visual_revision = String::new();
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
            let cancel = AtomicBool::new(false);
            if let Ok(plan) = content_reader_layers::content::visual::apply(
                &mut layers,
                &resolve,
                visual_mods,
                &recorded,
                &cancel,
            ) {
                visual_revision = plan.revision;
            }
        }
        match request.kind.as_str() {
            "equipmentDesigner" => {
                equipment_designer::render(self, source, &layers, &resolve, &visual_revision, request)
            }
            other => Err(unsupported_window_kind_refusal(other)),
        }
    }
}

pub fn load_native_guis_from_bytes(name: &str, bytes: &[u8]) -> io::Result<NativeWindows> {
    NativeWindows::of(native::Guis::new(vec![(
        name.to_owned(),
        native::read_at(bytes, DEFAULT_VIEWPORT.map(|value| value as i32)),
    )]))
}
