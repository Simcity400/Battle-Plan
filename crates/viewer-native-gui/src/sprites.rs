#[cfg(test)]
use viewer_foundation::mode_visibility::priority;
use viewer_gpu::ui_scale::device_corners;
use shader_pipeline::gpu::stats::CountedPasses as _;
#[cfg(test)]
use viewer_runtime::test_support::test_tga_rgba8;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::{collections::HashMap, io, sync::Arc};

#[cfg(test)]
pub(super) mod oracle;

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::gui::native::resources;
use content_reader_gfx::gfx;
use content_reader_image::{dds, native::Decode};

use viewer_gpu::{
    icon_tracks::{IconAnchor, Track},
    gpu_contract::{Targets, sampler_entry, texture_entry},
    textures::{Bound, ColorSpace, GuiTexture, upload, upload_gui},
};
use super::{free_icons::Present, lazy_icons::{DYNAMIC_BASE, LazyTextures}, text};

mod fade;
mod packets;
mod resident;
mod retained;

pub use resident::failures as overlay_check_failures;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteSheet {
    pub name: String,
    pub texture: usize,
    pub frames: u32,
    pub size: [u32; 2],
}

impl SpriteSheet {
    pub fn frame_size(&self) -> [f32; 2] {
        [
            (self.size[0] / self.frames.max(1)) as f32,
            self.size[1] as f32,
        ]
    }

    pub fn frame_uv(&self, frame: u32) -> [f32; 4] {
        let frames = self.frames.max(1);
        let index = frame.min(frames - 1);
        [
            index as f32 / frames as f32,
            0.0,
            (index + 1) as f32 / frames as f32,
            1.0,
        ]
    }
}

pub struct Interface {
    pub registry: content_reader_gui::gui::native::resources::Registry,
    pub sprites: gfx::SpriteSet,
    pub decode: Decode,
}

impl Interface {
    pub fn load(layers: &ResolvedLayers, decode: Decode) -> io::Result<Self> {
        let registry = resources::Registry::load(layers)?;
        let sprites = gfx::SpriteSet::new(&registry.files);
        Ok(Self {
            registry,
            sprites,
            decode,
        })
    }
}

pub fn gui_refusal(draw: &super::native_shader::draw::Quad) -> String {
    format!(
        "GUI shader {:?}: effect {:?} technique {:?}",
        draw.path, draw.effect.asset.source.effect, draw.effect.technique
    )
}

pub fn load_sheet(
    layers: &ResolvedLayers,
    sprites: &gfx::SpriteSet,
    name: &str,
    decode: Decode,
    images: &mut Vec<dds::Texture>,
) -> io::Result<SpriteSheet> {
    let (texture, frames, size) = decode_sheet(layers, sprites, name, decode)?;
    let id = images.len();
    images.push(texture);
    Ok(SpriteSheet {
        name: name.to_owned(),
        texture: id,
        frames,
        size,
    })
}

pub fn decode_sheet(
    layers: &ResolvedLayers,
    sprites: &gfx::SpriteSet,
    name: &str,
    decode: Decode,
) -> io::Result<(dds::Texture, u32, [u32; 2])> {
    let sprite = sprites.lookup(name).map_err(invalid)?;
    let path = sprite
        .texture
        .as_deref()
        .ok_or_else(|| invalid(format!("sprite {name} names no texturefile")))?;
    let recorded = layers.without_overlays();
    let texture =
        resources::stack_texture_or_recorded(layers, recorded.as_ref(), path.as_bytes(), decode)?
            .map_err(|why| invalid(format!("sprite {name}: {why}")))?;
    let texture = texture.image;
    if sprite.frames == 0 || texture.width < sprite.frames {
        return Err(invalid(format!(
            "sprite {name} declares {} frames for a {} texel wide strip",
            sprite.frames, texture.width
        )));
    }
    let size = [texture.width, texture.height];
    Ok((texture, sprite.frames, size))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    pub texture: usize,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub rotation: Option<f32>,
    pub layer: u64,
    pub shade: f32,
    pub exact: Option<Exact>,
    pub anchor: Option<IconAnchor>,
    pub present: Option<Present>,
}

impl Quad {
    pub fn screen_bounds(&self) -> [f32; 4] {
        let Some(rotation) = self.rotation else {
            return self.rect;
        };
        if rotation == 0.0 {
            return self.rect;
        }
        let corners = plain_quad_corners(self, 1.0);
        let min_x = corners.iter().map(|c| c[0]).fold(f32::INFINITY, f32::min);
        let min_y = corners.iter().map(|c| c[1]).fold(f32::INFINITY, f32::min);
        let max_x = corners
            .iter()
            .map(|c| c[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let max_y = corners
            .iter()
            .map(|c| c[1])
            .fold(f32::NEG_INFINITY, f32::max);
        [min_x, min_y, max_x - min_x, max_y - min_y]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Exact {
    pub native: Option<Arc<super::native_shader::draw::Quad>>,
    pub corners: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub scissor: Option<[i32; 4]>,
    pub technique: u32,
    pub blend: super::native_effects::Blend,
    pub samplers: u32,
    pub lod_bias: [f32; 3],
    pub second: Option<usize>,
    pub masking: Option<usize>,
    pub offsets: [f32; 4],
    pub inputs: [f32; 4],
    pub second_color: [f32; 4],
}

pub const fn layer(priority: i8, icon: u32, child: u32) -> u64 {
    band_layer((priority as u8 ^ 0x80) as u64, icon, child)
}

pub const fn hovered_layer(priority: i8, hovered: bool, icon: u32, child: u32) -> u64 {
    band_layer(
        (priority as i32 + 128 + if hovered { 5 } else { 0 }) as u64,
        icon,
        child,
    )
}

pub const fn gui_layer(icon: u32, child: u32) -> u64 {
    band_layer(GUI_BAND, icon, child)
}

const fn band_layer(band: u64, icon: u32, child: u32) -> u64 {
    band * PRIORITY_STRIDE + icon as u64 * CHILD_SLOTS as u64 + child as u64
}

pub const CHILD_SLOTS: u32 = 32;

const PRIORITY_STRIDE: u64 = 1 << 40;
const _: () = assert!((u32::MAX as u64 + 1) * CHILD_SLOTS as u64 <= PRIORITY_STRIDE);
const GUI_BAND: u64 = 261;

pub const TOOLTIP_LAYER: u64 = band_layer(GUI_BAND + 1, 0, 0);

#[cfg(any(test, feature = "test-support"))]
pub const fn layer_priority(layer: u64) -> i64 {
    (layer / PRIORITY_STRIDE) as i64 - 128
}

#[cfg(any(test, feature = "test-support"))]
pub const fn layer_icon(layer: u64) -> u64 {
    layer % PRIORITY_STRIDE / CHILD_SLOTS as u64
}

pub const fn icon_offset(icons: u32) -> u64 {
    icons as u64 * CHILD_SLOTS as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowPool {
    Base,
    FactionPing,
    LandCombat,
    LandCounter,
    Label,
    Raid,
    AdjacencyIcon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowKey {
    pub pool: WindowPool,
    pub part: u32,
    pub owner: u64,
}

impl WindowKey {
    pub fn new(pool: WindowPool, part: u32, owner: impl Into<u64>) -> Self {
        Self {
            pool,
            part,
            owner: owner.into(),
        }
    }
}

impl std::fmt::Display for WindowKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} {} of owner {}", self.pool, self.part, self.owner)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowStamp {
    pub painting: u64,
    pub taken: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchWindow {
    pub key: WindowKey,
    pub stamp: WindowStamp,
    pub first: u32,
    pub count: u32,
    pub shift: [i32; 2],
}

impl BatchWindow {
    pub fn quads(&self) -> std::ops::Range<usize> {
        self.first as usize..(self.first + self.count) as usize
    }
}

pub fn next_painting() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Debug)]
pub struct Batch {
    pub quads: Vec<Quad>,
    windows: Vec<BatchWindow>,
    open: Option<(WindowKey, WindowStamp, u32, [i32; 2])>,
    layer: u64,
    anchor: Option<IconAnchor>,
    part_anchor: Option<(String, IconAnchor)>,
    current: Option<IconAnchor>,
    painting: u64,
    tracks: Vec<(u32, Track)>,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            quads: Vec::new(),
            windows: Vec::new(),
            open: None,
            layer: 0,
            anchor: None,
            part_anchor: None,
            current: None,
            painting: next_painting(),
            tracks: Vec::new(),
        }
    }
}

impl Batch {
    pub fn loose(quads: Vec<Quad>) -> Self {
        Self {
            quads,
            ..Self::default()
        }
    }

    pub fn of_window(key: WindowKey, stamp: WindowStamp, quads: Vec<Quad>) -> Self {
        let count = quads.len() as u32;
        Self {
            quads,
            windows: (count > 0)
                .then_some(BatchWindow { key, stamp, first: 0, count, shift: [0; 2] })
                .into_iter()
                .collect(),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    pub fn painting(&self) -> u64 {
        self.painting
    }

    pub fn clear(&mut self) {
        self.painting = next_painting();
        self.tracks.clear();
        self.quads.clear();
        self.windows.clear();
        self.open = None;
        self.layer = 0;
        self.anchor = None;
        self.part_anchor = None;
        self.current = None;
    }

    pub fn at_window(&mut self, key: WindowKey, stamp: WindowStamp) {
        self.at_window_standing(key, stamp, [0; 2]);
    }

    pub fn at_window_standing(&mut self, key: WindowKey, stamp: WindowStamp, from: [i32; 2]) {
        self.no_window();
        self.open = Some((key, stamp, self.quads.len() as u32, from));
    }

    pub fn add_track(&mut self, track: Track) -> u32 {
        let serial = viewer_gpu::icon_tracks::next_track();
        self.tracks.push((serial, track));
        serial
    }

    pub fn track_of(&self, anchor: &IconAnchor) -> Option<&Track> {
        self.track(anchor.moves?)
    }

    pub fn track(&self, serial: u32) -> Option<&Track> {
        self.tracks
            .iter()
            .rev()
            .find_map(|(at, track)| (*at == serial).then_some(track))
    }

    pub fn moves(&self) -> bool {
        !self.tracks.is_empty()
    }

    pub fn no_window(&mut self) {
        if let Some((key, stamp, first, shift)) = self.open.take()
            && self.quads.len() as u32 > first
        {
            self.windows.push(BatchWindow {
                key,
                stamp,
                first,
                count: self.quads.len() as u32 - first,
                shift,
            });
        }
    }

    pub fn standing_windows(&mut self) -> &mut [BatchWindow] {
        self.no_window();
        &mut self.windows
    }

    pub fn windows(&self) -> impl Iterator<Item = BatchWindow> + '_ {
        let open = self.open.and_then(|(key, stamp, first, shift)| {
            (self.quads.len() as u32 > first).then_some(BatchWindow {
                key,
                stamp,
                first,
                count: self.quads.len() as u32 - first,
                shift,
            })
        });
        self.windows.iter().copied().chain(open)
    }

    pub fn append(&mut self, other: &Batch) {
        self.no_window();
        self.tracks.extend_from_slice(&other.tracks);
        let shift = self.quads.len() as u32;
        self.quads.extend_from_slice(&other.quads);
        self.windows.extend(other.windows().map(|window| BatchWindow {
            first: window.first + shift,
            ..window
        }));
    }

    pub fn splice(&mut self, range: std::ops::Range<usize>, other: &Batch) -> std::ops::Range<usize> {
        self.no_window();
        self.tracks.extend_from_slice(&other.tracks);
        let (first, last) = (range.start as u32, range.end as u32);
        let after = self.windows.iter().filter(|window| window.first >= last).count();
        self.windows.retain(|window| window.first < first || window.first >= last);
        let at = self.windows.len() - after;
        let shift = other.quads.len() as i64 - range.len() as i64;
        for window in &mut self.windows[at..] {
            window.first = (i64::from(window.first) + shift) as u32;
        }
        let taken: Vec<_> = other
            .windows()
            .map(|window| BatchWindow { first: window.first + first, ..window })
            .collect();
        self.windows.splice(at..at, taken);
        let start = range.start;
        self.quads.splice(range, other.quads.iter().cloned());
        start..start + other.quads.len()
    }

    pub fn at_layer(&mut self, layer: u64) {
        self.layer = layer;
    }

    pub fn at_anchor(&mut self, anchor: Option<IconAnchor>) {
        self.anchor = anchor;
        self.part_anchor = None;
        self.current = anchor;
    }

    pub fn at_part_anchor(&mut self, name: &str, anchor: IconAnchor) {
        self.part_anchor = Some((name.to_owned(), anchor));
    }

    pub fn at_part(&mut self, path: &str) {
        self.current = match &self.part_anchor {
            Some((name, anchor)) if path.ends_with(name.as_str()) => Some(*anchor),
            _ => self.anchor,
        };
    }

    pub fn push(&mut self, quad: Quad) {
        self.quads.push(Quad {
            layer: self.layer,
            anchor: self.current,
            ..quad
        });
    }

    pub fn scaled(
        &mut self,
        sheet: &SpriteSheet,
        frame: u32,
        at: [f32; 2],
        size: [f32; 2],
        color: [f32; 4],
    ) {
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv: sheet.frame_uv(frame),
            color,
            rotation: None,
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        });
    }

    pub fn cell(
        &mut self,
        texture: usize,
        at: [f32; 2],
        size: [f32; 2],
        uv: [f32; 4],
        color: [f32; 4],
    ) {
        self.quads.push(Quad {
            texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv,
            color,
            rotation: None,
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        });
    }

    pub fn bar(
        &mut self,
        sheet: &SpriteSheet,
        at: [f32; 2],
        size: [f32; 2],
        pixels: f32,
        rotation: f32,
        color: [f32; 4],
    ) {
        let [fw, fh] = sheet.frame_size();
        let vertical = fh > fw;
        let length = if vertical { size[1] } else { size[0] };
        let drawn = pixels.clamp(0.0, length + 1.0).floor();
        if drawn < 1.0 {
            return;
        }
        let fill = (drawn / length).min(1.0);
        let [u0, v0, u1, v1] = sheet.frame_uv(0);
        let bottom = at[1] + size[1].trunc();
        let pivot = [at[0] + fw * 0.5, bottom - fh * 0.5];
        let (unrot_center, rw, rh, uv) = if vertical {
            (
                [at[0] + size[0] * 0.5, bottom - size[1] * 0.5],
                size[0],
                drawn,
                [u0, v0, u1, v0 + (v1 - v0) * fill],
            )
        } else {
            (
                [at[0] + drawn * 0.5, bottom - size[1] * 0.5],
                drawn,
                size[1],
                [u0, v0, u0 + (u1 - u0) * fill, v1],
            )
        };
        let [cx, cy] = if rotation == 0.0 {
            unrot_center
        } else {
            let (sin, cos) = rotation.sin_cos();
            let [dx, dy] = [unrot_center[0] - pivot[0], unrot_center[1] - pivot[1]];
            [
                pivot[0] + dx * cos + dy * sin,
                pivot[1] + dy * cos - dx * sin,
            ]
        };
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [cx - rw * 0.5, cy - rh * 0.5, rw, rh],
            uv,
            color,
            rotation: Some(rotation),
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        });
    }

    pub fn text(&mut self, glyphs: &[text::TextGlyph], color: [f32; 4]) {
        self.quads.extend(glyphs.iter().map(|glyph| Quad {
            texture: glyph.page,
            rect: glyph.rect,
            uv: glyph.uv,
            color: glyph.color.unwrap_or(color),
            rotation: None,
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        }));
    }
}

pub fn device_scissor(scissor: [i32; 4], ui_scale: f32, size: [u32; 2]) -> Option<[u32; 4]> {
    let device = scissor.map(|value| (value as f32 * ui_scale) as i64);
    let x0 = device[0].clamp(0, i64::from(size[0]));
    let y0 = device[1].clamp(0, i64::from(size[1]));
    let x1 = device[2].clamp(0, i64::from(size[0]));
    let y1 = device[3].clamp(0, i64::from(size[1]));
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0].map(|value| value as u32))
}

#[cfg(any(test, feature = "test-support"))]
pub fn sample_window(uv: [f32; 4], texels: [u32; 2]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for axis in 0..2 {
        let half = 0.5 / texels[axis].max(1) as f32;
        let low = uv[axis].min(uv[axis + 2]);
        let high = uv[axis].max(uv[axis + 2]);
        let middle = (low + high) * 0.5;
        out[axis] = (low + half).min(middle);
        out[axis + 2] = (high - half).max(middle);
    }
    out
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    cell: [f32; 4],
    filter: f32,
    shade: f32,
}

pub fn filter_mode(ui_scale: f32) -> f32 {
    if ui_scale == 1.0 {
        0.0
    } else if ui_scale > 1.0 {
        1.0
    } else {
        2.0
    }
}

pub struct IconTextures {
    bound: Vec<GuiTexture>,
    unpack: Arc<wgpu::ComputePipeline>,
    screen: wgpu::Sampler,
}

impl IconTextures {
    pub fn views(&self) -> Vec<shader_pipeline::gpu::TextureBinding> {
        self.bound.iter().map(GuiTexture::binding).collect()
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &[dds::Texture],
    ) -> io::Result<Self> {
        let unpack = viewer_gpu::textures::unpack_pipeline(device);
        let bound = upload_gui(device, queue, images, &unpack)?;
        let clamp = |label, mip, max_lod| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: mip,
                lod_min_clamp: 0.0,
                lod_max_clamp: max_lod,
                ..Default::default()
            })
        };
        Ok(Self {
            bound,
            unpack,
            screen: clamp("icon screen sampler", wgpu::MipmapFilterMode::Nearest, 0.0),
        })
    }

    pub fn bind_all(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> Vec<wgpu::BindGroup> {
        self.bound
            .iter()
            .map(|texture| {
                shader_pipeline::gpu::stats::create_bind_group(
                    &device,
                    &wgpu::BindGroupDescriptor {
                        label: Some("icon page"),
                        layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&texture.view),
                        }],
                    },
                )
            })
            .collect()
    }

    pub fn screen_clamp(&self) -> &wgpu::Sampler {
        &self.screen
    }

    pub fn page_count(&self) -> usize {
        self.bound.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pipe {
    Legacy,
    Native(usize),
    Composite(usize),
}

struct Run {
    pipe: Pipe,
    texture: usize,
    scissor: Option<[u32; 4]>,
    range: std::ops::Range<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Frame,
    Page { page: usize, cell: [u32; 4] },
}

const OPAQUE: f32 = 1.0 - 0.5 / 255.0;

struct DynamicPage {
    revision: u64,
    texture: GuiTexture,
    group: wgpu::BindGroup,
}

pub const NO_TEXTURE: usize = usize::MAX;

pub struct SpritePass {
    uploads: Option<shader_pipeline::gpu::Arena>,
    retained: Option<retained::Upload>,
    retained_valid: bool,
    prepared_packets: Vec<Option<shader_pipeline::gpu::batch::Data>>,
    retained_diagnostics: content_reader_layers::content::diagnostics::Diagnostics,
    packets: packets::Cache,
    packet_views: Vec<(u32, View)>,
    bake_views: Vec<(u32, View)>,
    native: shader_pipeline::stream::Stream,
    native_draws: Vec<shader_pipeline::stream::Submission>,
    native_packets: Vec<Option<shader_pipeline::gpu::batch::Packet>>,
    fade_packets: Vec<Option<shader_pipeline::gpu::batch::Packet>>,
    array_views: shader_pipeline::gpu::batch::Views,
    native_format: wgpu::TextureFormat,
    bakes: Vec<CachedBake>,
    bake_buffers: Vec<wgpu::CommandBuffer>,
    bake_stream: shader_pipeline::stream::Stream,
    pub native_refusals: BTreeMap<String, u64>,
    pipeline: wgpu::RenderPipeline,
    _pipelines: Arc<SpritePipelines>,
    bind_groups: Vec<wgpu::BindGroup>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::BindGroup,
    views: Vec<shader_pipeline::gpu::TextureBinding>,
    empty: Bound,
    lazy: Arc<LazyTextures>,
    synced: Option<u64>,
    dynamic: HashMap<usize, DynamicPage>,
    failed: HashMap<usize, u64>,
    vertices: Stream,
    runs: Vec<Run>,
    count: u32,
    fade: fade::Pages,
    fade_legacy: wgpu::RenderPipeline,
    fade_stream: shader_pipeline::stream::Stream,
    fade_draws: Vec<shader_pipeline::stream::Submission>,
    fade_runs: Vec<Vec<Run>>,
    fade_vertices: Stream,
    composite_vertices: Stream,
    fade_pages: usize,
    scratch: Scratch,
    layer: bool,
    builds: shader_pipeline::stream::Builds,
    waited: bool,
    logged: HashSet<String>,
    unpack: Arc<wgpu::ComputePipeline>,
    written: u64,
    prepared: [u64; 2],
    resident: resident::Record,
    recording: bool,
    window: Option<WindowKey>,
    uploaded: u64,
}

#[derive(Default)]
struct Scratch {
    order: Vec<usize>,
    frame: Vec<Vertex>,
    pages: Vec<Vertex>,
    composites: Vec<fade::CompositeVertex>,
    icons: Vec<Icon>,
}

#[derive(Clone, Debug)]
struct Icon {
    quads: std::ops::Range<usize>,
    placement: fade::Placement,
    fading: Option<([u32; 4], f32)>,
    cell: (usize, [u32; 2]),
}

const INITIAL_VERTICES: u64 = 6 * 4096;

struct SpritePipelines {
    layout: wgpu::BindGroupLayout,
    samplers: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    fade_legacy: wgpu::RenderPipeline,
}

impl SpritePipelines {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite texture"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
            ],
        });
        let samplers = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite sampler"),
            entries: &[sampler_entry(0)],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprites"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprites"),
            bind_group_layouts: &[Some(&layout), Some(&samplers)],
            immediate_size: 0,
        });
        let target = |blend| {
            [Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::COLOR,
            })]
        };
        let legacy_targets = target(Some(wgpu::BlendState::ALPHA_BLENDING));
        let page_targets = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let snapped = |label, targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4,
                            4 => Float32, 5 => Float32
                        ],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fragment_main"),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let fade_legacy = snapped("sprite fade page", &page_targets);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4,
                        4 => Float32, 5 => Float32
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &legacy_targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            layout,
            samplers,
            pipeline,
            fade_legacy,
        }
    }
}

impl SpritePass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &IconTextures,
        lazy: &Arc<LazyTextures>,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let pipelines = viewer_runtime::device_shared::shared(device, ("sprites", format), || {
            SpritePipelines::new(device, format)
        });
        let layout = pipelines.layout.clone();
        let bind_groups = textures.bind_all(device, &layout);
        let sampler = shader_pipeline::gpu::stats::create_bind_group(
            device,
            &wgpu::BindGroupDescriptor {
                label: Some("sprite sampler"),
                layout: &pipelines.samplers,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(textures.screen_clamp()),
                }],
            },
        );
        let pipeline = pipelines.pipeline.clone();
        let fade_legacy = pipelines.fade_legacy.clone();
        let empty = upload(
            device,
            queue,
            "empty sprite slot",
            &dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
                width: 1,
                height: 1,
                pixels: vec![0; 4],
            }),
            ColorSpace::Linear,
        )?;
        Ok(Self {
            native: shader_pipeline::stream::Stream::gui(),
            uploads: Some(shader_pipeline::gpu::Arena::new(device)),
            retained: None,
            retained_valid: false,
            prepared_packets: Vec::new(),
            retained_diagnostics: Default::default(),
            packets: Default::default(),
            packet_views: Vec::new(),
            bake_views: Vec::new(),
            native_draws: Vec::new(),
            native_packets: Vec::new(),
            fade_packets: Vec::new(),
            array_views: Default::default(),
            bakes: Vec::new(),
            bake_buffers: Vec::new(),
            bake_stream: shader_pipeline::stream::Stream::gui(),
            native_format: format,
            native_refusals: Default::default(),
            pipeline,
            _pipelines: pipelines,
            bind_groups,
            layout,
            sampler,
            views: textures.bound.iter().map(GuiTexture::binding).collect(),
            empty,
            lazy: Arc::clone(lazy),
            synced: None,
            dynamic: HashMap::new(),
            failed: HashMap::new(),
            vertices: Stream::new::<Vertex>(device),
            runs: Vec::new(),
            count: 0,
            fade: fade::Pages::new(device, format),
            fade_legacy,
            fade_stream: shader_pipeline::stream::Stream::gui(),
            fade_draws: Vec::new(),
            fade_runs: Vec::new(),
            fade_vertices: Stream::new::<Vertex>(device),
            composite_vertices: Stream::new::<fade::CompositeVertex>(device),
            fade_pages: 0,
            scratch: Scratch::default(),
            layer: false,
            builds: Default::default(),
            waited: false,
            logged: HashSet::new(),
            unpack: Arc::clone(&textures.unpack),
            written: 0,
            prepared: [0; 2],
            resident: Default::default(),
            recording: false,
            window: None,
            uploaded: 0,
        })
    }

    pub fn defer_builds(&mut self, defer: bool) {
        let builds = defer.then(|| self.builds.clone());
        for stream in [
            &mut self.native,
            &mut self.fade_stream,
            &mut self.bake_stream,
        ] {
            stream.defer_builds(builds.clone());
        }
    }

    pub fn warm(
        &mut self,
        device: &wgpu::Device,
        effects: impl IntoIterator<Item = Arc<shader_pipeline::library::Effect>>,
    ) {
        use wgpu::PrimitiveTopology::{TriangleList, TriangleStrip};
        let (format, layer) = (self.native_format, self.layer);
        for effect in effects {
            effect.warm();
            for index in 0..effect.source.techniques.len() {
                let selection = shader_pipeline::library::Selection {
                    effect: Arc::clone(&effect),
                    index,
                };
                for (topology, default_blend) in [TriangleStrip, TriangleList]
                    .into_iter()
                    .flat_map(|topology| [(topology, false), (topology, true)])
                {
                    self.native.warm(
                        device,
                        &selection,
                        (default_blend, layer),
                        (format, 1, topology),
                    );
                    self.fade_stream.warm(
                        device,
                        &selection,
                        (default_blend, true),
                        (format, 1, topology),
                    );
                }
                for default_blend in [false, true] {
                    self.bake_stream.warm(
                        device,
                        &selection,
                        (default_blend, false),
                        (BAKE_FORMAT, 1, TriangleList),
                    );
                }
            }
        }
    }

    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let version = self.lazy.version();
        if self.synced == Some(version) {
            return;
        }
        let resident = self.lazy.snapshot();
        let uploaded: HashMap<usize, u64> = self
            .dynamic
            .iter()
            .map(|(id, page)| (*id, page.revision))
            .collect();
        let revisions: Vec<(usize, u64)> = resident
            .iter()
            .map(|(id, entry)| (*id, entry.revision))
            .collect();
        let plan = plan_sync(&uploaded, &self.failed, &revisions);
        for id in &plan.drop {
            self.dynamic.remove(id);
            self.failed.remove(id);
        }
        for (id, entry) in resident {
            if !plan.upload.contains(&id) {
                continue;
            }
            if let Some(page) = self.dynamic.get_mut(&id)
                && !entry.cells.is_empty()
            {
                page.texture.update_cells(
                    queue,
                    &entry.texture,
                    entry.cells.iter().filter_map(|(cell, revision)| {
                        (*revision > page.revision).then_some(*cell)
                    }),
                );
                page.revision = entry.revision;
                self.failed.remove(&id);
                continue;
            }
            let texture = match GuiTexture::upload(
                device,
                queue,
                &format!("lazy icon {}", id - DYNAMIC_BASE),
                &entry.texture,
                &self.unpack,
            ) {
                Ok(texture) => texture,
                Err(error) => {
                    if self.failed.insert(id, entry.revision).is_none() {
                        eprintln!("Warning: an icon texture is not drawn: {error}");
                    }
                    self.dynamic.remove(&id);
                    continue;
                }
            };
            self.failed.remove(&id);
            let group = shader_pipeline::gpu::stats::create_bind_group(
                &device,
                &wgpu::BindGroupDescriptor {
                    label: Some("lazy icon"),
                    layout: &self.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    }],
                },
            );
            self.dynamic.insert(
                id,
                DynamicPage {
                    revision: entry.revision,
                    texture,
                    group,
                },
            );
        }
        self.synced = Some(version);
    }

    fn group(&self, texture: usize) -> Option<&wgpu::BindGroup> {
        self.bind_groups
            .get(texture)
            .or_else(|| self.dynamic.get(&texture).map(|page| &page.group))
    }

    fn view(&self, texture: usize) -> Option<shader_pipeline::gpu::TextureBinding> {
        if texture == NO_TEXTURE {
            return Some(shader_pipeline::gpu::TextureBinding {
                view: self.empty.view.clone(),
                format: self.empty.view_format,
                array: None,
            });
        }
        self.views.get(texture).cloned().or_else(|| {
            self.dynamic
                .get(&texture)
                .map(|page| page.texture.binding())
        })
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
    ) {
        let ((), diagnostics) = content_reader_layers::content::diagnostics::capture(|| {
            self.upload_inner(device, queue, arena, batch, size, ui_scale)
        });
        diagnostics.credit();
        self.retained_diagnostics = diagnostics;
    }

    fn upload_inner(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
    ) {
        self.lazy.mark_drawn(
            batch
                .quads
                .iter()
                .flat_map(|quad| {
                    let exact = quad
                        .exact
                        .as_ref()
                        .map_or([None, None], |exact| [exact.second, exact.masking]);
                    let native = quad
                        .exact
                        .as_ref()
                        .and_then(|e| e.native.as_ref())
                        .into_iter()
                        .flat_map(|draw| {
                            draw.textures
                                .iter()
                                .chain(
                                    draw.bake
                                        .iter()
                                        .flat_map(|b| b.draws.iter().flat_map(|d| &d.textures)),
                                )
                                .filter_map(|(_, id)| *id)
                        });
                    std::iter::once(Some(quad.texture))
                        .chain(exact)
                        .flatten()
                        .chain(native)
                })
                .filter(|texture| *texture >= DYNAMIC_BASE)
                .collect::<BTreeSet<_>>(),
        );
        self.written = 0;
        self.uploaded += 1;
        self.recording = resident::every().is_some();
        let uploads = self
            .uploads
            .take()
            .expect("sprite uploads belong to the pass");
        let current = self.retained_valid && !self.waited;
        let same = current
            && self
                .retained
                .as_ref()
                .is_some_and(|retained| retained.matches(batch, size, ui_scale, self.synced));
        let reordered = current && !same && self.retained.is_some() && {
            let _order = viewer_runtime::profiling::span("sprite_order");
            sort_order(batch, &mut self.scratch.order);
            self.retained.as_ref().is_some_and(|retained| {
                retained.matches_in_order(batch, &self.scratch.order, size, ui_scale, self.synced)
            })
        };
        let moved = (!same && !reordered && current)
            .then(|| {
                let _span = viewer_runtime::profiling::span("sprite_moved_windows");
                self.retained
                    .as_ref()
                    .and_then(|retained| retained.moved_windows(batch, size, ui_scale, self.synced))
            })
            .flatten()
            .filter(|windows| !windows.is_empty());
        let keeps = self.keeps_packets(batch, moved.as_deref());
        if same || reordered {
            self.retained_diagnostics.credit();
            if reordered {
                self.prepared_packets.clear();
                if let Some(retained) = self.retained.as_mut() {
                    retained.update(batch, &self.scratch.order, size, ui_scale, self.synced);
                }
            }
        } else {
            self.waited = false;
            self.logged.clear();
            uploads.reset();
            let before = uploads.written_bytes();
            self.upload_changed(device, queue, &uploads, batch, size, ui_scale, keeps);
            self.note_waiting(arena);
            uploads.flush(queue);
            self.written += uploads.written_bytes() - before;
            let order = &self.scratch.order;
            match self.retained.as_mut() {
                Some(retained) => retained.update(batch, order, size, ui_scale, self.synced),
                None => {
                    self.retained = Some(retained::Upload::new(
                        batch,
                        order,
                        size,
                        ui_scale,
                        self.synced,
                    ));
                }
            }
        }
        self.retained_valid = true;
        for reason in uploads.refusals() {
            arena.refuse(&reason);
        }
        viewer_runtime::redraws::note("sprite.written_bytes", self.written);
        self.check_resident(
            device,
            queue,
            &uploads,
            batch,
            size,
            ui_scale,
            same || reordered,
        );
        self.uploads = Some(uploads);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the upload's own inputs, which the rebuild is made with"
    )]
    fn check_resident(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
        quiet: bool,
    ) {
        let Some(every) = resident::every() else {
            return;
        };
        if quiet {
            resident::check_quiet(self.written);
        }
        if self.uploaded % every != 0 {
            return;
        }
        viewer_runtime::redraws::note("sprite.checked", 1);
        let held = std::mem::take(&mut self.resident);
        self.waited = false;
        self.logged.clear();
        arena.reset();
        self.upload_changed(device, queue, arena, batch, size, ui_scale, None);
        self.note_waiting(arena);
        arena.flush(queue);
        resident::check(&held, &self.resident, self.written);
        self.retained_valid = true;
        if let Some(retained) = self.retained.as_mut() {
            retained.update(batch, &self.scratch.order, size, ui_scale, self.synced);
        } else {
            self.retained = Some(retained::Upload::new(
                batch,
                &self.scratch.order,
                size,
                ui_scale,
                self.synced,
            ));
        }
    }

    fn keeps_packets(&self, batch: &Batch, moved: Option<&[usize]>) -> Option<Vec<bool>> {
        let moved = moved?;
        let retained = self.retained.as_ref()?;
        if self.prepared_packets.len() != batch.quads.len() {
            viewer_runtime::redraws::note("sprite.keeps_refused_no_packets", 1);
            return None;
        }
        for (quad, prepared) in batch.quads.iter().zip(&self.prepared_packets) {
            if quad.exact.is_some() && prepared.is_none() {
                viewer_runtime::redraws::note("sprite.keeps_refused_a_draw", 1);
                return None;
            }
        }
        let mut stands = vec![false; batch.quads.len()];
        for window in moved {
            for quad in retained.window(*window).quads() {
                stands[quad] = true;
            }
        }
        Some(stands)
    }

    #[expect(clippy::too_many_arguments, reason = "the upload's inputs and what it keeps")]
    fn upload_changed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
        keeps: Option<Vec<bool>>,
    ) {
        if keeps.is_none() {
            self.packets.begin();
        }
        self.array_views.begin();
        self.native_packets.clear();
        self.fade_packets.clear();
        self.runs.clear();
        self.native_draws.clear();
        self.fade_draws.clear();
        for runs in &mut self.fade_runs {
            runs.clear();
        }
        self.fade_pages = 0;
        if keeps.is_none() {
            for bake in &mut self.bakes {
                bake.used = false;
            }
        }
        self.native_refusals.clear();
        self.native.refused.clear();
        if self.recording {
            self.resident.clear();
        }
        self.reset_stream();
        self.take_waiting();
        self.count = 0;
        if batch.is_empty() || size[0] == 0 || size[1] == 0 {
            self.prepared_packets.clear();
            self.bakes.clear();
            self.array_views.finish();
            self.fade.fit(device, &[]);
            self.packets.finish();
            return;
        }
        let mut prepared = std::mem::take(&mut self.prepared_packets);
        match &keeps {
            Some(_) => viewer_runtime::redraws::note("sprite.packets_kept", 1),
            None => {
                prepared.clear();
                prepared.resize(batch.quads.len(), None);
            }
        }
        let keeping = keeps.as_deref();
        let Scratch {
            mut order,
            mut frame,
            mut pages,
            mut composites,
            mut icons,
        } = std::mem::take(&mut self.scratch);
        {
            let _order = viewer_runtime::profiling::span("sprite_order");
            sort_order(batch, &mut order);
        }
        let emit = viewer_runtime::profiling::span("sprite_emit");
        let owners = self
            .recording
            .then(|| resident::owners(&batch.windows().collect::<Vec<_>>(), batch.quads.len()))
            .unwrap_or_default();
        frame.clear();
        pages.clear();
        composites.clear();
        icons.clear();
        let mut at = 0;
        while at < order.len() {
            let Some(present) = batch.quads[order[at]].present else {
                break;
            };
            let end = at
                + order[at..]
                    .iter()
                    .take_while(|&&index| {
                        batch.quads[index]
                            .present
                            .is_some_and(|other| other.rank == present.rank)
                    })
                    .count();
            let quads = at..end;
            at = end;
            let placement = fade::Placement {
                scale: present.scale,
                pivot: present.pivot.map(|value| value * ui_scale),
                shift: [0.0; 2],
                size,
                out: size,
            };
            let fading = if present.alpha < OPAQUE {
                let bounds = order[quads.clone()]
                    .iter()
                    .filter_map(|&index| quad_bounds(&batch.quads[index], size, ui_scale))
                    .map(|rect| placement.rect(rect))
                    .reduce(|a, b| {
                        [
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            a[2].max(b[2]),
                            a[3].max(b[3]),
                        ]
                    })
                    .and_then(|rect| fade::cut(rect, [0, 0, size[0], size[1]]));
                let Some(rect) = bounds else {
                    continue;
                };
                Some((rect, present.alpha))
            } else {
                None
            };
            icons.push(Icon {
                quads,
                placement,
                fading,
                cell: (0, [0, 0]),
            });
        }
        let screen = at;
        let largest = icons
            .iter()
            .filter_map(|icon| icon.fading)
            .fold([0, 0], |most, (rect, _)| {
                [most[0].max(rect[2]), most[1].max(rect[3])]
            });
        let mut shelves = self.fade.shelves(largest);
        for icon in &mut icons {
            if let Some((rect, _)) = icon.fading {
                icon.cell = shelves.place([rect[2], rect[3]]);
            }
        }
        self.fade.fit(device, &shelves.extents);
        self.fade_pages = shelves.pages;
        for icon in &icons {
            let (placement, target) = match icon.fading {
                Some(([x, y, width, height], alpha)) => {
                    let (page, corner) = icon.cell;
                    let page_size = self.fade.size(page);
                    let cell = [corner[0], corner[1], corner[0] + width, corner[1] + height];
                    let start = composites.len() as u32;
                    composites.extend(fade::composite_quad(
                        cell.map(|v| v as f32),
                        page_size,
                        [x, y, x + width, y + height].map(|v| v as f32),
                        size,
                        alpha,
                    ));
                    self.runs.push(Run {
                        pipe: Pipe::Composite(page),
                        texture: NO_TEXTURE,
                        scissor: None,
                        range: start..composites.len() as u32,
                    });
                    if self.recording {
                        self.resident.draws.push(resident::Draw::Composite {
                            page,
                            vertices: bytemuck::cast_slice(&composites[start as usize..]).to_vec(),
                        });
                    }
                    (
                        fade::Placement {
                            shift: [corner[0] as f32 - x as f32, corner[1] as f32 - y as f32],
                            out: page_size,
                            ..icon.placement
                        },
                        Target::Page { page, cell },
                    )
                }
                None => (icon.placement, Target::Frame),
            };
            for &index in &order[icon.quads.clone()] {
                self.window = owners.get(index).copied().flatten();
                self.emit(
                    (device, queue, arena),
                    (index, &batch.quads[index]),
                    placement,
                    target,
                    ui_scale,
                    (&mut frame, &mut pages),
                    (keeping, &mut prepared),
                );
            }
        }
        for &index in &order[screen..] {
            self.window = owners.get(index).copied().flatten();
            self.emit(
                (device, queue, arena),
                (index, &batch.quads[index]),
                fade::Placement::identity(size),
                Target::Frame,
                ui_scale,
                (&mut frame, &mut pages),
                (keeping, &mut prepared),
            );
        }
        self.window = None;

        drop(emit);
        {
            use viewer_runtime::redraws::note;
            let mut native = 0;
            let mut plain = 0;
            let mut exact_no_native = 0;
            for quad in &batch.quads {
                match &quad.exact {
                    None => plain += 1,
                    Some(exact) if exact.native.is_some() => native += 1,
                    Some(_) => exact_no_native += 1,
                }
            }
            note("sprite.quads_plain", plain);
            note("sprite.quads_native", native);
            note("sprite.quads_exact_no_native", exact_no_native);
            note("sprite.packets_native", self.prepared[0]);
            note("sprite.packets_batched", self.prepared[1]);
            self.prepared = [0; 2];
        }
        let batching = viewer_runtime::profiling::span("sprite_batch");
        self.prepared_packets = prepared;
        self.runs = batch_runs(
            std::mem::take(&mut self.runs),
            &mut self.native_packets,
            &mut self.native_draws,
            &mut self.array_views,
            arena,
            size,
            &mut self.native_refusals,
        );
        for page in 0..self.fade_pages {
            self.fade_runs[page] = batch_runs(
                std::mem::take(&mut self.fade_runs[page]),
                &mut self.fade_packets,
                &mut self.fade_draws,
                &mut self.array_views,
                arena,
                self.fade.size(page),
                &mut self.native_refusals,
            );
        }
        self.array_views.finish();
        if keeps.is_none() {
            self.bakes.retain(|bake| bake.used);
            self.packets.finish();
        }
        if self.recording {
            let run = |run: &Run| resident::Run {
                native: match run.pipe {
                    Pipe::Native(index) => Some(index),
                    _ => None,
                },
                composite: match run.pipe {
                    Pipe::Composite(page) => Some(page),
                    _ => None,
                },
                texture: run.texture,
                scissor: run.scissor,
                range: run.range.clone(),
            };
            self.resident.runs = self.runs.iter().map(run).collect();
            self.resident.pages = (0..self.fade_pages)
                .map(|page| self.fade_runs[page].iter().map(run).collect())
                .collect();
            self.resident.extents = (0..self.fade_pages).map(|page| self.fade.size(page)).collect();
        }
        drop(batching);
        let _write = viewer_runtime::profiling::span("sprite_write");
        self.count = frame.len() as u32;
        self.written += self.vertices.write(device, queue, &frame)
            + self.fade_vertices.write(device, queue, &pages)
            + self.composite_vertices.write(device, queue, &composites);
        self.scratch = Scratch {
            order,
            frame,
            pages,
            composites,
            icons,
        };
        {
            use viewer_runtime::redraws::note;
            note("sprite.runs", self.runs.len() as u64);
            note("sprite.submissions", self.native_draws.len() as u64);
            note("sprite.legacy_vertices", u64::from(self.count));
        }
    }

    #[expect(clippy::too_many_arguments, reason = "one quad's own inputs and what it may keep")]
    fn emit(
        &mut self,
        (device, queue, arena): (&wgpu::Device, &wgpu::Queue, &shader_pipeline::gpu::Arena),
        (index, quad): (usize, &Quad),
        placement: fade::Placement,
        target: Target,
        ui_scale: f32,
        (frame, pages): (&mut Vec<Vertex>, &mut Vec<Vertex>),
        (keeping, prepared_packets): (
            Option<&[bool]>,
            &mut [Option<shader_pipeline::gpu::batch::Data>],
        ),
    ) {
        let size = placement.size;
        let within = match target {
            Target::Frame => [0, 0, size[0], size[1]],
            Target::Page { cell, .. } => cell,
        };
        let cell_scissor = match target {
            Target::Frame => None,
            Target::Page { cell, .. } => {
                Some([cell[0], cell[1], cell[2] - cell[0], cell[3] - cell[1]])
            }
        };
        if let Some(exact) = &quad.exact {
            let Some(native) = &exact.native else {
                self.reset_stream();
                *self
                    .native_refusals
                    .entry("GUI quad has no compiled effect payload".into())
                    .or_default() += 1;
                arena.refuse("GUI shader: a quad without a compiled effect");
                return;
            };
            let clip = (!placement.is_identity()).then(|| placement.clip());
            use shader_pipeline::gpu::stats::{Count, add};
            add(Count::GuiHanded, 1);
            let kept = keeping.and_then(|stands| {
                let held = prepared_packets[index].as_mut()?;
                if stands[index] {
                    let viewport = super::native_shader::draw::gui_viewport(size, ui_scale);
                    let constants = buffer_zero(native, viewport, clip);
                    held.rebound(&*TRANSFORM_BINDINGS, bytemuck::cast_slice(&constants));
                }
                Some(held.clone())
            });
            let prepared = match kept {
                Some(data) => Ok(shader_pipeline::gpu::batch::Packet::Batched(data)),
                None => {
                    let prepared = self
                        .prepare_target((device, queue, arena), native, (size, ui_scale, clip), target);
                    add(Count::GuiPrepared, 1);
                    prepared_packets[index] = match &prepared {
                        Ok(shader_pipeline::gpu::batch::Packet::Batched(data)) => Some(data.clone()),
                        _ => None,
                    };
                    prepared
                }
            };
            if matches!(prepared, Ok(shader_pipeline::gpu::batch::Packet::Native { .. })) {
                add(Count::GuiNative, 1);
            }
            match prepared {
                Ok(submission) => {
                    let scissor = match exact.scissor {
                        Some(clip) => match device_scissor(clip, ui_scale, size)
                            .and_then(|scissor| placement.scissor(scissor, within))
                        {
                            Some(scissor) => Some(scissor),
                            None => return,
                        },
                        None => cell_scissor,
                    };
                    if self.recording {
                        let viewport = super::native_shader::draw::gui_viewport(size, ui_scale);
                        let constants = buffer_zero(native, viewport, clip);
                        self.resident.draws.push(resident::Draw::Native {
                            window: self.window,
                            page: match target {
                                Target::Frame => None,
                                Target::Page { page, .. } => Some(page),
                            },
                            path: native.path.clone(),
                            constants: bytemuck::cast_slice(&constants).to_vec(),
                            extra: native
                                .extra_constants
                                .iter()
                                .map(|(slot, values, _)| {
                                    (*slot, bytemuck::cast_slice(values).to_vec())
                                })
                                .collect(),
                            vertices: native.vertices.clone(),
                            textures: native.textures.clone(),
                            scissor,
                            batched: !matches!(
                                submission,
                                shader_pipeline::gpu::batch::Packet::Native { .. }
                            ),
                        });
                    }
                    let run = |index| Run {
                        pipe: Pipe::Native(index),
                        texture: NO_TEXTURE,
                        scissor,
                        range: 0..0,
                    };
                    match target {
                        Target::Frame => {
                            let index = self.native_packets.len();
                            self.native_packets.push(Some(submission));
                            self.runs.push(run(index));
                        }
                        Target::Page { page, .. } => {
                            let index = self.fade_packets.len();
                            self.fade_packets.push(Some(submission));
                            self.page_runs(page).push(run(index));
                        }
                    }
                }
                Err(_) if self.take_waiting() => self.waited = true,
                Err(reason) => {
                    if self.logged.insert(reason.clone()) {
                        eprintln!("{reason}");
                    }
                    *self.native_refusals.entry(reason).or_default() += native.source_draws;
                    arena.refuse(&gui_refusal(native));
                }
            }
            return;
        }
        self.reset_stream();
        if self.group(quad.texture).is_none() {
            return;
        }
        let corners = plain_corners(quad, placement, ui_scale);
        let (vertices, runs): (&mut Vec<Vertex>, Option<usize>) = match target {
            Target::Frame => (frame, None),
            Target::Page { page, .. } => (pages, Some(page)),
        };
        let start = vertices.len() as u32;
        vertices.extend(corners);
        let end = vertices.len() as u32;
        if self.recording {
            self.resident.draws.push(resident::Draw::Plain {
                window: self.window,
                page: match target {
                    Target::Frame => None,
                    Target::Page { page, .. } => Some(page),
                },
                texture: quad.texture,
                scissor: cell_scissor,
                vertices: bytemuck::cast_slice(&vertices[start as usize..end as usize]).to_vec(),
            });
        }
        let fresh = Run {
            pipe: Pipe::Legacy,
            texture: quad.texture,
            scissor: cell_scissor,
            range: start..end,
        };
        let extends = |run: &Run| {
            run.pipe == Pipe::Legacy
                && run.texture == quad.texture
                && run.scissor == cell_scissor
                && run.range.end == start
        };
        match runs {
            None => match self.runs.last_mut() {
                Some(run) if extends(run) => run.range.end = end,
                _ => self.runs.push(fresh),
            },
            Some(page) => {
                let runs = self.page_runs(page);
                match runs.last_mut() {
                    Some(run) if extends(run) => run.range.end = end,
                    _ => runs.push(fresh),
                }
            }
        }
    }

    fn prepare_target(
        &mut self,
        (device, queue, arena): (&wgpu::Device, &wgpu::Queue, &shader_pipeline::gpu::Arena),
        native: &super::native_shader::draw::Quad,
        (size, ui_scale, clip): ([u32; 2], f32, Option<fade::ClipAffine>),
        target: Target,
    ) -> Result<shader_pipeline::gpu::batch::Packet, String> {
        match target {
            Target::Frame => {
                self.prepare_native(device, queue, arena, native, (size, ui_scale, clip), None)
            }
            Target::Page { .. } => {
                self.fade_stream.inherit_bindings(&self.native);
                let prepared = self.prepare_native(
                    device,
                    queue,
                    arena,
                    native,
                    (size, ui_scale, clip),
                    Some(self.native_format),
                );
                self.native.inherit_bindings(&self.fade_stream);
                prepared
            }
        }
    }

    fn reset_stream(&mut self) {
        self.native.invalidate_bindings();
    }

    fn page_runs(&mut self, page: usize) -> &mut Vec<Run> {
        if self.fade_runs.len() <= page {
            self.fade_runs.resize_with(page + 1, Vec::new);
        }
        &mut self.fade_runs[page]
    }

    pub fn bake_buffers(&mut self) -> Vec<wgpu::CommandBuffer> {
        std::mem::take(&mut self.bake_buffers)
    }

    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets) {
        if self.runs.is_empty() {
            return;
        }
        self.draw_pages(encoder);
        let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprites"),
            color_attachments: &[Some(targets.output_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let [width, height] = targets.size();
        self.draw_runs(&mut pass, [width, height]);
    }

    fn draw_pages(&self, encoder: &mut wgpu::CommandEncoder) {
        for (index, page) in self.fade.pages.iter().take(self.fade_pages).enumerate() {
            let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprite fade page"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &page.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut bound = false;
            let mut previous_native = None;
            let mut scissor = None;
            for run in self.fade_runs.get(index).into_iter().flatten() {
                let [x, y, w, h] = run.scissor.unwrap_or([0, 0, page.size[0], page.size[1]]);
                if scissor != Some([x, y, w, h]) {
                    pass.set_scissor_rect(x, y, w, h);
                    scissor = Some([x, y, w, h]);
                }
                match run.pipe {
                    Pipe::Native(draw) => {
                        let submission = &self.fade_draws[draw];
                        submission.pipeline.draw_after(
                            &mut pass,
                            &submission.draw,
                            previous_native,
                        );
                        previous_native = Some(&submission.draw);
                        bound = false;
                    }
                    Pipe::Legacy => {
                        let Some(group) = self.group(run.texture) else {
                            continue;
                        };
                        previous_native = None;
                        if !bound {
                            shader_pipeline::gpu::stats::pipeline();
                            pass.set_pipeline(&self.fade_legacy);
                            shader_pipeline::gpu::stats::state();
                            pass.set_vertex_buffer(0, self.fade_vertices.slice());
                            shader_pipeline::gpu::stats::bind();
                            pass.set_bind_group(1, &self.sampler, &[]);
                            bound = true;
                        }
                        shader_pipeline::gpu::stats::bind();
                        pass.set_bind_group(0, group, &[]);
                        shader_pipeline::gpu::stats::draw();
                        pass.draw(run.range.clone(), 0..1);
                    }
                    Pipe::Composite(_) => unreachable!("fade pages hold no composites"),
                }
            }
        }
    }

    fn draw_runs(&self, pass: &mut wgpu::RenderPass<'_>, [width, height]: [u32; 2]) {
        let mut bound = None;
        let mut previous_native = None;
        let mut scissor = None;
        for run in &self.runs {
            let [x, y, w, h] = run.scissor.unwrap_or([0, 0, width, height]);
            if scissor != Some([x, y, w, h]) {
                pass.set_scissor_rect(x, y, w, h);
                scissor = Some([x, y, w, h]);
            }
            if let Pipe::Native(index) = run.pipe {
                let submission = &self.native_draws[index];
                submission
                    .pipeline
                    .draw_after(pass, &submission.draw, previous_native);
                previous_native = Some(&submission.draw);
                bound = None;
                continue;
            }
            previous_native = None;
            if let Pipe::Composite(page) = run.pipe {
                if bound != Some(Pipe::Composite(0)) {
                    shader_pipeline::gpu::stats::pipeline();
                    pass.set_pipeline(&self.fade.composite);
                    shader_pipeline::gpu::stats::state();
                    pass.set_vertex_buffer(0, self.composite_vertices.slice());
                    bound = Some(Pipe::Composite(0));
                }
                shader_pipeline::gpu::stats::bind();
                pass.set_bind_group(0, &self.fade.pages[page].group, &[]);
                shader_pipeline::gpu::stats::draw();
                pass.draw(run.range.clone(), 0..1);
                continue;
            }
            if bound != Some(run.pipe) {
                match run.pipe {
                    Pipe::Legacy => {
                        shader_pipeline::gpu::stats::pipeline();
                        pass.set_pipeline(&self.pipeline);
                        shader_pipeline::gpu::stats::state();
                        pass.set_vertex_buffer(0, self.vertices.slice());
                        shader_pipeline::gpu::stats::bind();
                        pass.set_bind_group(1, &self.sampler, &[]);
                    }
                    Pipe::Native(_) | Pipe::Composite(_) => unreachable!(),
                }
                bound = Some(run.pipe);
            }
            let group = match run.pipe {
                Pipe::Legacy => self.group(run.texture),
                Pipe::Native(_) | Pipe::Composite(_) => unreachable!(),
            };
            let Some(group) = group else {
                continue;
            };
            shader_pipeline::gpu::stats::bind();
            pass.set_bind_group(0, group, &[]);
            shader_pipeline::gpu::stats::draw();
            pass.draw(run.range.clone(), 0..1);
        }
    }

    pub fn set_layer(&mut self, layer: bool) {
        self.layer = layer;
    }

    pub fn draw_layer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: [u32; 2],
    ) -> Result<(), String> {
        if !self.layer {
            return Err("the sprite pass is not drawing a layer".into());
        }
        if self.runs.iter().any(|run| run.pipe == Pipe::Legacy) {
            return Err("a layer holds a quad without a compiled effect".into());
        }
        self.draw_pages(encoder);
        let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprite layer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.draw_runs(&mut pass, size);
        Ok(())
    }

    pub fn quad_count(&self) -> u32 {
        self.count / 6 + self.native_draws.len() as u32
    }

    pub fn fade_pages(&self) -> usize {
        self.fade_pages
    }

    pub fn written_bytes(&self) -> u64 {
        self.written
    }

    fn prepare_native(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        draw: &super::native_shader::draw::Quad,
        (size, scale, clip): ([u32; 2], f32, Option<fade::ClipAffine>),
        page: Option<wgpu::TextureFormat>,
    ) -> Result<shader_pipeline::gpu::batch::Packet, String> {
        let _span = viewer_runtime::profiling::span("sprite_native");
        let baked = draw.bake.as_ref().map(|bake| {
            self.prepare_bake(device, queue, arena, bake)
                .map(|view| shader_pipeline::gpu::TextureBinding {
                    view,
                    format: BAKE_FORMAT,
                    array: None,
                })
                .map_err(|why| {
                    format!(
                        "effect {:?}, text sprite {:?}: {why}",
                        draw.effect.asset.source.effect, draw.path
                    )
                })
        });
        let mut views = std::mem::take(&mut self.packet_views);
        self.native_views(draw, &mut views);
        let viewport = super::native_shader::draw::gui_viewport(size, scale);
        let prepared = match page {
            None => prepare_draw(
                &mut self.native,
                &mut self.packets,
                arena,
                draw,
                &views,
                baked,
                Output {
                    viewport,
                    format: self.native_format,
                    clip,
                    premultiply: self.layer,
                    layer: self.layer,
                },
            ),
            Some(format) => prepare_draw(
                &mut self.fade_stream,
                &mut self.packets,
                arena,
                draw,
                &views,
                baked,
                Output {
                    viewport,
                    format,
                    clip,
                    premultiply: true,
                    layer: false,
                },
            ),
        };
        views.clear();
        self.packet_views = views;
        match &prepared {
            Ok(shader_pipeline::gpu::batch::Packet::Native { .. }) => self.prepared[0] += 1,
            Ok(_) => self.prepared[1] += 1,
            Err(_) => {}
        }
        prepared
    }

    fn note_waiting(&mut self, arena: &shader_pipeline::gpu::Arena) {
        if !self.waited {
            return;
        }
        arena.request_frame();
        let reason = if shader_pipeline::gpu::builds_under_way() == 0 {
            "a GUI draw waited for a pipeline no build was making"
        } else {
            "a GUI draw's pipeline was still being built"
        }
        .to_string();
        *self.native_refusals.entry(reason.clone()).or_default() += 1;
        arena.refuse(&reason);
        shader_pipeline::gpu::stats::add(shader_pipeline::gpu::stats::Count::GuiJoins, 1);
    }

    fn take_waiting(&mut self) -> bool {
        let native = self.native.take_waiting();
        let fade = self.fade_stream.take_waiting();
        let bake = self.bake_stream.take_waiting();
        native || fade || bake
    }

    fn native_views(
        &self,
        draw: &super::native_shader::draw::Quad,
        storage: &mut Vec<(u32, View)>,
    ) {
        packets::views(storage, draw, |id| self.view(id));
    }

    fn prepare_bake(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        bake: &Arc<super::native_shader::draw::Bake>,
    ) -> Result<wgpu::TextureView, String> {
        let (result, diagnostics) = content_reader_layers::content::diagnostics::capture(|| {
            self.prepare_bake_inner(device, queue, arena, bake)
        });
        diagnostics.credit();
        let revision = self.lazy.version();
        if result.is_ok()
            && let Some(cached) = self
                .bakes
                .iter_mut()
                .find(|cached| cached.revision == revision && cached.source == *bake)
        {
            cached.diagnostics = diagnostics;
        }
        result
    }

    fn prepare_bake_inner(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        bake: &Arc<super::native_shader::draw::Bake>,
    ) -> Result<wgpu::TextureView, String> {
        let _span = viewer_runtime::profiling::span("sprite_bake");
        let revision = self.lazy.version();
        let stable = bake.draws.iter().all(self_contained);
        if stable
            && let Some(cached) = self
                .bakes
                .iter_mut()
                .find(|c| c.revision == revision && c.source == *bake)
        {
            cached.used = true;
            cached.diagnostics.credit();
            return Ok(cached.view.clone());
        }
        let _made = viewer_runtime::profiling::span("sprite_bake_made");
        let [width, height] = bake.size;
        if width == 0 || height == 0 || width.max(height) > device.limits().max_texture_dimension_2d
        {
            return Err(format!(
                "text bake target {width}x{height} exceeds device limits"
            ));
        }
        let format = BAKE_FORMAT;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text sprite bake"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.bake_stream.inherit_bindings(&self.native);
        let mut submissions = Vec::new();
        for draw in &bake.draws {
            let mut views = std::mem::take(&mut self.bake_views);
            self.native_views(draw, &mut views);
            let submission = prepare_draw(
                &mut self.bake_stream,
                &mut self.packets,
                arena,
                draw,
                &views,
                None,
                Output {
                    viewport: bake.size,
                    format,
                    clip: None,
                    premultiply: false,
                    layer: false,
                },
            );
            views.clear();
            self.bake_views = views;
            submissions.push(submission?);
        }
        let submissions = shader_pipeline::gpu::batch::prepare(
            submissions
                .into_iter()
                .map(|packet| shader_pipeline::gpu::batch::Item {
                    packet,
                    scissor: [0, 0, width, height],
                })
                .collect(),
            &mut self.array_views,
            arena,
            [width, height],
            |reason, count| {
                arena.refuse(&reason);
                *self.native_refusals.entry(reason).or_default() += count;
            },
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text background then alpha MAX glyphs"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut previous = None;
            for (submission, scissor) in &submissions {
                let [x, y, w, h] = scissor.unwrap_or([0, 0, width, height]);
                pass.set_scissor_rect(x, y, w, h);
                submission
                    .pipeline
                    .draw_after(&mut pass, &submission.draw, previous);
                previous = Some(&submission.draw);
            }
        }
        arena.flush(queue);
        self.bake_buffers.push(encoder.finish());
        self.bakes.push(CachedBake {
            source: bake.clone(),
            revision,
            view: view.clone(),
            used: true,
            diagnostics: Default::default(),
        });
        Ok(view)
    }
}

fn sort_order(batch: &Batch, order: &mut Vec<usize>) {
    order.clear();
    order.extend(0..batch.quads.len());
    order.sort_by_key(|&index| {
        let quad = &batch.quads[index];
        (
            quad.present.map_or(1u8, |_| 0),
            quad.present.map_or(0, |present| present.rank),
            quad.layer,
            if quad.exact.is_some() {
                0
            } else {
                quad.texture
            },
        )
    });
}

fn batch_runs(
    runs: Vec<Run>,
    packets: &mut [Option<shader_pipeline::gpu::batch::Packet>],
    draws: &mut Vec<shader_pipeline::stream::Submission>,
    array_views: &mut shader_pipeline::gpu::batch::Views,
    arena: &shader_pipeline::gpu::Arena,
    size: [u32; 2],
    refusals: &mut BTreeMap<String, u64>,
) -> Vec<Run> {
    let mut output = Vec::new();
    let mut pending = Vec::new();
    let mut refuse = |reason: String, count| {
        arena.refuse(&reason);
        *refusals.entry(reason).or_default() += count;
    };
    let mut flush = |pending: &mut Vec<shader_pipeline::gpu::batch::Item>,
                     output: &mut Vec<Run>,
                     draws: &mut Vec<shader_pipeline::stream::Submission>,
                     array_views: &mut shader_pipeline::gpu::batch::Views| {
        if pending.is_empty() {
            return;
        }
        for (submission, scissor) in shader_pipeline::gpu::batch::prepare(
            std::mem::take(pending),
            array_views,
            arena,
            size,
            &mut refuse,
        ) {
            let index = draws.len();
            draws.push(submission);
            output.push(Run {
                pipe: Pipe::Native(index),
                texture: NO_TEXTURE,
                scissor,
                range: 0..0,
            });
        }
    };
    for run in runs {
        if let Pipe::Native(index) = run.pipe {
            pending.push(shader_pipeline::gpu::batch::Item {
                packet: packets[index].take().expect("each packet is drawn once"),
                scissor: run.scissor.unwrap_or([0, 0, size[0], size[1]]),
            });
        } else {
            flush(&mut pending, &mut output, draws, array_views);
            output.push(run);
        }
    }
    flush(&mut pending, &mut output, draws, array_views);
    output
}

const BAKE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

struct CachedBake {
    source: Arc<super::native_shader::draw::Bake>,
    revision: u64,
    view: wgpu::TextureView,
    used: bool,
    diagnostics: content_reader_layers::content::diagnostics::Diagnostics,
}

fn self_contained(draw: &super::native_shader::draw::Quad) -> bool {
    use shader_pipeline::binding;
    use content_reader_gui::pdxshader::program::Stage;
    let Ok(Some(selection)) = draw.effect.asset.select(&draw.effect.technique) else {
        return false;
    };
    let Ok(compiled) = selection.effect.compiled(selection.index) else {
        return false;
    };
    let textures: Vec<u32> = draw
        .textures
        .iter()
        .map(|(slot, _)| binding(*slot, Stage::Pixel))
        .collect();
    let mut buffers = vec![binding(0, Stage::Vertex)];
    if draw.pixel_constants {
        buffers.push(binding(0, Stage::Pixel));
    }
    for (slot, _, _) in &draw.extra_constants {
        buffers.extend([binding(*slot, Stage::Vertex), binding(*slot, Stage::Pixel)]);
    }
    let source = &selection.effect.source;
    [&compiled.vertex, &compiled.pixel]
        .into_iter()
        .all(|shader| {
            shader.buffers.iter().all(|b| buffers.contains(&b.binding))
                && shader.textures.iter().all(|(b, _)| textures.contains(b))
                && shader
                    .samplers
                    .iter()
                    .all(|b| textures.contains(b) && source.sampler(b / 2).is_some())
        })
}

type View = Result<Option<shader_pipeline::gpu::TextureBinding>, String>;

struct Output {
    viewport: [u32; 2],
    format: wgpu::TextureFormat,
    clip: Option<fade::ClipAffine>,
    premultiply: bool,
    layer: bool,
}

const SOURCE_OVER: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::SrcAlpha,
    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
    operation: wgpu::BlendOperation::Add,
};

fn unflattenable(blend: Option<wgpu::BlendState>, mask: wgpu::ColorWrites) -> Option<String> {
    match blend {
        Some(blend) if blend.color == SOURCE_OVER && mask.contains(wgpu::ColorWrites::COLOR) => {
            None
        }
        Some(blend) if blend.color == SOURCE_OVER => Some(format!(
            "it writes only {mask:?} of its colour, which a composited image cannot keep"
        )),
        Some(blend) => Some(format!(
            "it blends colour as {:?}, which a composited image cannot reproduce",
            blend.color
        )),
        None => Some("it draws without blending, which a composited image cannot reproduce".into()),
    }
}

fn bind_draw(
    native: &mut shader_pipeline::stream::Stream,
    packets: &mut packets::Cache,
    draw: &super::native_shader::draw::Quad,
    views: &[(u32, View)],
    baked: Option<Result<shader_pipeline::gpu::TextureBinding, String>>,
    &Output {
        viewport,
        clip,
        premultiply,
        layer,
        ..
    }: &Output,
) -> (packets::Text, Option<String>) {
    use content_reader_gui::pdxshader::program::Stage;
    let packets::Metadata { label, selection } = packets.metadata(draw);
    let mut refused = baked
        .as_ref()
        .and_then(|baked| baked.as_ref().err().cloned());
    if refused.is_none() {
        refused = views
            .iter()
            .find_map(|(_, view)| view.as_ref().err().cloned());
    }
    let declared = !matches!(selection, Ok(None));
    match selection {
        Ok(selection) => {
            if let Err(why) = native.bind_effect(selection, draw.declaration) {
                refused.get_or_insert(format!("native GUI draw {label:?}: {why}"));
            }
        }
        Err(why) => {
            native.forget_program();
            refused.get_or_insert(why);
        }
    }
    if draw.default_blend {
        native.bind_blend(None, wgpu::ColorWrites::ALL);
    }
    if layer {
        let (blend, mask) = native.blend();
        if let Some(why) = unflattenable(blend, mask) {
            refused.get_or_insert(format!(
                "native GUI draw {label:?} (effect {:?}, technique {}): {why}",
                draw.effect.asset.source.effect, draw.effect.technique
            ));
        }
    }
    if premultiply {
        native.accumulate_alpha();
    }
    let constants = buffer_zero(draw, viewport, clip);
    let known = [std::ops::Range {
        start: 0,
        end: draw.known,
    }];
    for stage in [Stage::Vertex, Stage::Pixel] {
        if stage == Stage::Pixel && !draw.pixel_constants {
            continue;
        }
        native.bind_partial_constants(0, stage, Some(bytemuck::cast_slice(&constants)), &known);
    }
    for (slot, constants, known) in &draw.extra_constants {
        for stage in [Stage::Vertex, Stage::Pixel] {
            native.bind_partial_constants(
                *slot,
                stage,
                Some(bytemuck::cast_slice(constants)),
                known,
            );
        }
    }
    for (slot, view) in views {
        let slot = *slot;
        let view = match (&baked, slot) {
            (Some(baked), 0) => baked.clone().map(Some),
            _ => view.clone(),
        };
        let sampler = declared
            .then(|| draw.effect.asset.source.sampler(slot))
            .flatten();
        match view {
            Ok(view) => native.bind_formatted_texture(slot, Stage::Pixel, view, sampler),
            Err(_) => native.forget_texture(slot, Stage::Pixel, sampler),
        }
    }
    (label, refused)
}

fn buffer_zero(
    draw: &super::native_shader::draw::Quad,
    viewport: [u32; 2],
    clip: Option<fade::ClipAffine>,
) -> Vec<f32> {
    let mut constants = draw.projected_constants(viewport);
    if let Some(clip) = clip {
        fade::apply(clip, &mut constants);
    }
    if draw.declaration == shader_pipeline::vertices::Declaration::LineChart {
        for row in 0..4 {
            for column in row + 1..4 {
                constants.swap(row * 4 + column, column * 4 + row);
            }
        }
    }
    constants
}

fn prepare_draw(
    native: &mut shader_pipeline::stream::Stream,
    packets: &mut packets::Cache,
    arena: &shader_pipeline::gpu::Arena,
    draw: &super::native_shader::draw::Quad,
    views: &[(u32, View)],
    baked: Option<Result<shader_pipeline::gpu::TextureBinding, String>>,
    output: Output,
) -> Result<shader_pipeline::gpu::batch::Packet, String> {
    let (label, refused) = bind_draw(native, packets, draw, views, baked, &output);
    if let Some(reason) = refused {
        return Err(reason);
    }
    native
        .prepare_packet(
            arena,
            &label,
            &draw.vertices,
            draw.stride,
            draw.vertices.len() / draw.stride,
            draw.topology,
            output.format,
            1,
        )
        .map(|mut packet| {
            packet.source_draws(draw.source_draws);
            packet
        })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SyncPlan {
    drop: Vec<usize>,
    upload: Vec<usize>,
}

fn plan_sync(
    uploaded: &HashMap<usize, u64>,
    failed: &HashMap<usize, u64>,
    resident: &[(usize, u64)],
) -> SyncPlan {
    let live: HashSet<usize> = resident.iter().map(|(id, _)| *id).collect();
    let mut drop: Vec<usize> = uploaded
        .keys()
        .chain(failed.keys())
        .filter(|id| !live.contains(id))
        .copied()
        .collect();
    drop.sort_unstable();
    drop.dedup();
    let upload = resident
        .iter()
        .filter(|(id, revision)| {
            uploaded.get(id) != Some(revision) && failed.get(id) != Some(revision)
        })
        .map(|(id, _)| *id)
        .collect();
    SyncPlan { drop, upload }
}

struct Stream {
    buffer: wgpu::Buffer,
    capacity: u64,
    held: Vec<u8>,
    stride: usize,
}

impl Stream {
    fn new<T>(device: &wgpu::Device) -> Self {
        let stride = std::mem::size_of::<T>();
        Self {
            buffer: buffer(device, INITIAL_VERTICES, stride),
            capacity: INITIAL_VERTICES,
            held: Vec::new(),
            stride,
        }
    }

    fn slice(&self) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(..)
    }

    fn write<T: bytemuck::Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[T],
    ) -> u64 {
        if vertices.is_empty() {
            return 0;
        }
        let count = vertices.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.buffer = buffer(device, self.capacity, self.stride);
            self.held.clear();
        }
        let fresh: &[u8] = bytemuck::cast_slice(vertices);
        let known = self.held.len().min(fresh.len());
        let mut written = 0;
        for span in shader_pipeline::gpu::unheld(0, &fresh[..known], &self.held[..known]) {
            written += (span.end - span.start) as u64;
            shader_pipeline::gpu::stats::write_buffer(
                &queue,
                &self.buffer,
                span.start as u64,
                &fresh[span.clone()],
            );
            self.held[span.clone()].copy_from_slice(&fresh[span]);
        }
        if fresh.len() > known {
            written += (fresh.len() - known) as u64;
            shader_pipeline::gpu::stats::write_buffer(
                &queue,
                &self.buffer,
                known as u64,
                &fresh[known..],
            );
            self.held.resize(fresh.len(), 0);
            self.held[known..].copy_from_slice(&fresh[known..]);
        }
        written
    }
}

static TRANSFORM_BINDINGS: std::sync::LazyLock<[u32; 2]> = std::sync::LazyLock::new(|| {
    use content_reader_gui::pdxshader::program::Stage;
    [
        shader_pipeline::binding(0, Stage::Vertex),
        shader_pipeline::binding(0, Stage::Pixel),
    ]
});

fn plain_quad_corners(quad: &Quad, ui_scale: f32) -> [[f32; 2]; 4] {
    let Some(rotation) = quad.rotation else {
        let [x0, y0, x1, y1] = device_corners(quad.rect, ui_scale);
        return [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    };
    let x0 = quad.rect[0] * ui_scale;
    let y0 = quad.rect[1] * ui_scale;
    let x1 = (quad.rect[0] + quad.rect[2]) * ui_scale;
    let y1 = (quad.rect[1] + quad.rect[3]) * ui_scale;
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    if rotation == 0.0 {
        return corners;
    }
    let centre = [(x0 + x1) * 0.5, (y0 + y1) * 0.5];
    let (sin, cos) = rotation.sin_cos();
    corners.map(|[x, y]| {
        let (dx, dy) = (x - centre[0], y - centre[1]);
        [
            centre[0] + dx * cos + dy * sin,
            centre[1] + dy * cos - dx * sin,
        ]
    })
}

fn plain_corners(quad: &Quad, placement: fade::Placement, ui_scale: f32) -> [Vertex; 6] {
    let corners = plain_quad_corners(quad, ui_scale);
    let filter = filter_mode(ui_scale * placement.scale);
    let [out_width, out_height] = placement.out.map(|v| v as f32);
    let corner = |[x, y]: [f32; 2], u: f32, v: f32| {
        let [x, y] = placement.point([x, y]);
        Vertex {
            position: [x / out_width * 2.0 - 1.0, 1.0 - y / out_height * 2.0],
            uv: [u, v],
            color: quad.color,
            cell: quad.uv,
            filter,
            shade: quad.shade,
        }
    };
    let [u0, v0, u1, v1] = quad.uv;
    [
        corner(corners[0], u0, v0),
        corner(corners[1], u1, v0),
        corner(corners[2], u1, v1),
        corner(corners[0], u0, v0),
        corner(corners[2], u1, v1),
        corner(corners[3], u0, v1),
    ]
}

fn quad_bounds(quad: &Quad, size: [u32; 2], ui_scale: f32) -> Option<[f32; 4]> {
    let corners: [[f32; 2]; 4] = match &quad.exact {
        Some(exact) => {
            let native = exact.native.as_ref()?;
            let viewport = native
                .font_viewport
                .map(|font| font.map(|v| v as f32))
                .unwrap_or_else(|| {
                    super::native_shader::draw::gui_viewport(size, ui_scale).map(|v| v as f32)
                });
            exact.corners.map(|[x, y]| {
                [
                    x / viewport[0] * size[0] as f32,
                    y / viewport[1] * size[1] as f32,
                ]
            })
        }
        None => plain_quad_corners(quad, ui_scale),
    };
    let [x, y] = corners[0];
    let mut bounds = [x, y, x, y];
    for [x, y] in corners {
        bounds = [
            bounds[0].min(x),
            bounds[1].min(y),
            bounds[2].max(x),
            bounds[3].max(y),
        ];
    }
    Some([
        bounds[0] - 1.0,
        bounds[1] - 1.0,
        bounds[2] + 1.0,
        bounds[3] + 1.0,
    ])
}

fn buffer(device: &wgpu::Device, vertices: u64, stride: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite vertices"),
        size: vertices * stride as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub const SHADER: &str = r#"
struct SpriteVertex {
    @location(0) clip_xy: vec2<f32>,
    @location(1) texcoord: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) cell: vec4<f32>,
    @location(4) sampling: f32,
    @location(5) shade: f32,
}

struct SpriteFragment {
    @builtin(position) position: vec4<f32>,
    @location(0) texcoord: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) @interpolate(flat) cell: vec4<f32>,
    @location(3) @interpolate(flat) sampling: f32,
    @location(4) @interpolate(flat) shade: f32,
}

@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(1) @binding(0) var sprite_sampler: sampler;

@vertex
fn vertex_main(vertex: SpriteVertex) -> SpriteFragment {
    var fragment: SpriteFragment;
    fragment.position = vec4<f32>(vertex.clip_xy, 0.0, 1.0);
    fragment.texcoord = vertex.texcoord;
    fragment.tint = vertex.tint;
    fragment.cell = vertex.cell;
    fragment.sampling = vertex.sampling;
    fragment.shade = vertex.shade;
    return fragment;
}

fn read_sprite(
    texcoord: vec2<f32>,
    cell: vec4<f32>,
    sampling: f32,
    horizontal: vec2<f32>,
    vertical: vec2<f32>,
) -> vec4<f32> {
    if sampling < 0.5 {
        return textureSampleLevel(sprite_texture, sprite_sampler, texcoord, 0.0);
    }

    let dimensions = vec2<f32>(textureDimensions(sprite_texture, 0));
    let first_corner = min(cell.xy, cell.zw);
    let last_corner = max(cell.xy, cell.zw);
    let middle = (first_corner + last_corner) * 0.5;
    let inset = vec2<f32>(0.5) / dimensions;
    let lower = min(first_corner + inset, middle);
    let upper = max(last_corner - inset, middle);

    if sampling <= 1.5 {
        return textureSampleLevel(
            sprite_texture,
            sprite_sampler,
            clamp(texcoord, lower, upper),
            0.0,
        );
    }

    let footprint = max(abs(horizontal), abs(vertical)) * dimensions;
    let longest_axis = max(footprint.x, footprint.y);
    let count = u32(clamp(ceil(longest_axis - 0.001), 1.0, 4.0));
    let taps_per_axis = f32(count);
    var total = vec4<f32>(0.0);

    for (var row = 0u; row < count; row += 1u) {
        let row_center = (f32(row) + 0.5) / taps_per_axis - 0.5;
        for (var column = 0u; column < count; column += 1u) {
            let column_center = (f32(column) + 0.5) / taps_per_axis - 0.5;
            let horizontal_uv = texcoord + horizontal * column_center;
            let tap = clamp(horizontal_uv + vertical * row_center, lower, upper);
            total += textureSampleLevel(sprite_texture, sprite_sampler, tap, 0.0);
        }
    }
    return total / f32(count * count);
}

fn shade_sprite(source: vec4<f32>, tint: vec4<f32>, shade: f32) -> vec4<f32> {
    if tint.a < 0.0 {
        let luminance = dot(source.rgb, vec3<f32>(0.212671, 0.715160, 0.072169));
        return vec4<f32>(luminance * tint.rgb, source.a * abs(tint.a));
    }

    let tinted = source * tint;
    let brightened = tinted.rgb + shade * (tinted.rgb + vec3<f32>(0.5));
    return vec4<f32>(brightened, tinted.a);
}

@fragment
fn fragment_main(fragment: SpriteFragment) -> @location(0) vec4<f32> {
    let horizontal = dpdx(fragment.texcoord);
    let vertical = dpdy(fragment.texcoord);
    let source = read_sprite(
        fragment.texcoord,
        fragment.cell,
        fragment.sampling,
        horizontal,
        vertical,
    );
    return shade_sprite(source, fragment.tint, fragment.shade);
}
"#;

pub const DISABLED: [f32; 4] = [1.0, 1.0, 1.0, -1.0];

pub fn dump_sprites<'a>(batch: &Batch, extra: &Batch, sprites: &'a mut Batch) -> &'a Batch {
    sprites.clear();
    sprites.quads.extend_from_slice(&batch.quads);
    sprites.quads.extend_from_slice(&extra.quads);
    sprites
}

#[cfg(test)]
mod tests {
    use super::oracle::{render_presented, render_presented_at};
    use super::*;

    #[test]
    fn sprite_pass_draws() {
        let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
        a_gui_draw_refused_before_the_stream_is_recorded(device.clone(), queue.clone());
        a_fading_icon_blends_in_as_one_image_scaled_about_its_anchor(device.clone(), queue.clone());
        an_icon_wider_than_a_default_fade_page_still_fades_whole(device.clone(), queue.clone());
        each_fade_page_draws_this_upload_s_icons_alone(device.clone(), queue.clone());
        fade_pages_follow_each_upload_s_demand(device.clone(), queue.clone());
        every_upload_draws_its_own_fading_icons_alone(device.clone(), queue.clone());
        patterned_bar_renders_rotated_and_unrotated_fills_with_overrun(device, queue);
    }

    fn a_gui_draw_refused_before_the_stream_is_recorded(device: wgpu::Device, queue: wgpu::Queue) {
        let page = dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![255; 4],
        });
        let images = IconTextures::new(&device, &queue, &[page]).unwrap();
        let mut pass = SpritePass::new(
            &device,
            &queue,
            &images,
            &LazyTextures::new(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap();
        let mut batch = Batch::default();
        batch.quads.push(Quad {
            texture: 0,
            rect: [0.0, 0.0, 4.0, 4.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
            rotation: None,
            layer: 0,
            shade: 0.0,
            anchor: None,
            present: None,
            exact: Some(Exact {
                native: None,
                corners: [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
                uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                scissor: None,
                technique: 0,
                blend: super::super::native_effects::Blend::Off,
                samplers: 0,
                lod_bias: [0.0; 3],
                second: None,
                masking: None,
                offsets: [0.0; 4],
                inputs: [0.0; 4],
                second_color: [0.0; 4],
            }),
        });
        let arena = shader_pipeline::gpu::Arena::new(&device);
        arena.reset();
        pass.upload(&device, &queue, &arena, &batch, [16, 16], 1.0);
        assert_eq!(pass.native_refusals.len(), 1);
        assert_eq!(
            arena.refusals(),
            ["GUI shader: a quad without a compiled effect".to_owned()].into()
        );
        let next_frame = shader_pipeline::gpu::Arena::new(&device);
        pass.upload(&device, &queue, &next_frame, &batch, [16, 16], 1.0);
        assert_eq!(next_frame.refusals(), arena.refusals());
        assert_eq!(pass.native_refusals.values().sum::<u64>(), 1);
    }

    #[test]
    fn counter_sheet_uses_the_native_texture_decoder_and_client_setting() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_counter_native_texture_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        std::fs::write(root.join("gfx/test.tga"), test_tga_rgba8(3, 2)).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let sprites = gfx::SpriteSet::new(&[(
            "interface/test.gfx".into(),
            content_reader_gui::gui::native::sprites::read_sprites(
                b"spriteTypes = { spriteType = { name = GFX_test texturefile = gfx/test.tga } }",
            ),
        )]);
        let plain = decode_sheet(&layers, &sprites, "GFX_test", Decode::default()).unwrap();
        let padded = decode_sheet(
            &layers,
            &sprites,
            "GFX_test",
            Decode {
                force_pow2_textures: true,
            },
        )
        .unwrap();
        assert_eq!(plain.2, [3, 2]);
        assert_eq!(padded.2, [4, 2]);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn strip(frames: u32, width: u32) -> SpriteSheet {
        SpriteSheet {
            name: "GFX_test".into(),
            texture: 0,
            frames,
            size: [width, 27],
        }
    }

    #[test]
    fn a_strip_that_does_not_divide_evenly_keeps_the_authored_pitch() {
        let naval = strip(11, 301);
        assert_eq!(naval.frame_size()[0], 27.0);
        assert_eq!(naval.frame_uv(10), [10.0 / 11.0, 0.0, 1.0, 1.0]);
        assert_eq!(strip(7, 309).frame_size()[0], 44.0);
    }

    #[test]
    fn layer_keys_stay_distinct_and_ordered_past_62500_icons() {
        content_reader_script::test_game::enter();
        assert_ne!(layer(5, 62_500, 0), layer(6, 0, 0));
        assert!(layer(5, 62_500, 0) < layer(6, 0, 0));
        assert!(layer(5, u32::MAX, CHILD_SLOTS - 1) < layer(6, 0, 0));
        let mut keys: Vec<u64> = (0..200_000u32)
            .flat_map(|icon| [layer(10, icon, 0), layer(10, icon, CHILD_SLOTS - 1)])
            .collect();
        let count = keys.len();
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        keys.dedup();
        assert_eq!(keys.len(), count);
        let key = layer(20, 150_000, 3);
        assert_eq!(layer_priority(key), 20);
        assert_eq!(layer_icon(key), 150_000);
        assert_eq!(layer(10, 7, 2) + icon_offset(70_000), layer(10, 70_007, 2));
    }

    #[test]
    fn priorities_are_the_defines_signed_low_byte_in_order_under_the_gui() {
        let defines = content_reader_defines::defines::Defines::new().unwrap();
        defines
            .load(
                "mod.lua",
                b"NDefines = { NMapIcons = { A = 20.9, B = 300, C = 200, D = -1.5 } }",
            )
            .unwrap();
        let read = |name| priority(&defines, name).unwrap();
        assert_eq!(
            [read("A"), read("B"), read("C"), read("D")],
            [20, 44, -56, -1]
        );
        assert!(layer(-128, u32::MAX, CHILD_SLOTS - 1) < layer(-127, 0, 0));
        assert!(layer(-1, u32::MAX, CHILD_SLOTS - 1) < layer(0, 0, 0));
        assert!(layer(i8::MAX, u32::MAX, CHILD_SLOTS - 1) < gui_layer(0, 0));
        assert!(gui_layer(u32::MAX, CHILD_SLOTS - 1) < TOOLTIP_LAYER);
        assert_eq!(layer_priority(layer(-56, 3, 0)), -56);
        assert_eq!(layer_priority(hovered_layer(i8::MAX, true, 0, 0)), 132);
        assert!(hovered_layer(i8::MAX, true, u32::MAX, CHILD_SLOTS - 1) < gui_layer(0, 0));
    }

    #[test]
    fn a_refused_upload_is_skipped_until_its_texture_changes() {
        let base = DYNAMIC_BASE;
        let uploaded = HashMap::from([(base, 1), (base + 1, 3)]);
        let failed = HashMap::from([(base + 2, 5)]);
        let resident = [(base + 1, 4), (base + 2, 5), (base + 3, 0)];
        assert_eq!(
            plan_sync(&uploaded, &failed, &resident),
            SyncPlan {
                drop: vec![base],
                upload: vec![base + 1, base + 3],
            }
        );
        assert_eq!(
            plan_sync(&uploaded, &failed, &[(base + 2, 6)]).upload,
            [base + 2]
        );
        assert_eq!(plan_sync(&HashMap::new(), &failed, &[]).drop, [base + 2]);
    }

    fn footprints(
        rect: [f32; 4],
        ui_scale: f32,
        uv: [f32; 4],
        texels: u32,
        window: Option<[f32; 4]>,
    ) -> Vec<(f32, f32)> {
        let [x0, _, x1, _] = device_corners(rect, ui_scale);
        (x0 as u32..x1 as u32)
            .map(|pixel| {
                let t = (pixel as f32 + 0.5 - x0) / (x1 - x0);
                let mut u = uv[0] + (uv[2] - uv[0]) * t;
                if let Some(window) = window {
                    u = u.clamp(window[0], window[2]);
                }
                let texel = u * texels as f32;
                (texel - 0.5, texel + 0.5)
            })
            .collect()
    }

    #[test]
    fn a_scaled_flag_does_not_bleed_into_the_next_cell() {
        let texels = 512;
        let uv = [82.0 / 512.0, 0.0, 123.0 / 512.0, 1.0];
        let rect = [100.0, 0.0, 41.0, 26.0];
        let inside = |(low, high): (f32, f32)| low >= 82.0 - 1e-3 && high <= 123.0 + 1e-3;
        let unclamped = footprints(rect, 1.25, uv, texels, None);
        assert!(!inside(unclamped[0]) && !inside(*unclamped.last().unwrap()));
        let window = sample_window(uv, [texels, 1]);
        for scale in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            let pixels = footprints(rect, scale, uv, texels, Some(window));
            assert!(pixels.iter().copied().all(inside), "{scale}");
        }
        let clamped = footprints(rect, 1.0, uv, texels, Some(window));
        let free = footprints(rect, 1.0, uv, texels, None);
        assert_eq!(clamped.len(), 41);
        for (a, b) in clamped.iter().zip(&free) {
            assert!((a.0 - b.0).abs() < 1e-3, "{a:?} {b:?}");
        }
    }

    #[test]
    fn a_window_narrower_than_a_texel_collapses_to_its_centre() {
        let window = sample_window([0.5, 0.25, 0.5, 0.75], [4, 4]);
        assert_eq!(window, [0.5, 0.375, 0.5, 0.625]);
        assert_eq!(sample_window([0.75, 0.0, 0.25, 1.0], [4, 1])[0], 0.375);
    }

    #[test]
    fn a_batch_drawn_in_the_same_order_keeps_its_upload_whatever_its_layers() {
        let batch = |layers: [u64; 2], textures: [usize; 2]| Batch {
            quads: vec![
                shown(textures[0], [0., 0., 4., 4.], layers[0], None),
                shown(textures[1], [4., 0., 4., 4.], layers[1], None),
            ],
            ..Batch::default()
        };
        let mut order = Vec::new();
        let first = batch([3, 5], [1, 2]);
        sort_order(&first, &mut order);
        let kept = retained::Upload::new(&first, &order, [8, 8], 1.0, None);
        let drawn = |next: &Batch| {
            let mut order = Vec::new();
            sort_order(next, &mut order);
            kept.matches_in_order(next, &order, [8, 8], 1.0, None)
        };
        assert!(drawn(&batch([7, 9], [1, 2])), "other places among other windows, same order");
        assert!(!kept.matches(&batch([7, 9], [1, 2]), [8, 8], 1.0, None), "not the same batch");
        assert!(!drawn(&batch([9, 7], [1, 2])), "the same quads in another order");
        assert!(!drawn(&batch([7, 9], [1, 3])), "another quad");
    }

    fn shown(texture: usize, rect: [f32; 4], layer: u64, present: Option<Present>) -> Quad {
        Quad {
            texture,
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
            rotation: None,
            layer,
            shade: 0.0,
            exact: None,
            anchor: None,
            present,
        }
    }

    fn solid(rgba: [u8; 4]) -> dds::Texture {
        dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
            width: 1,
            height: 1,
            pixels: rgba.to_vec(),
        })
    }

    fn a_fading_icon_blends_in_as_one_image_scaled_about_its_anchor(
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) {
        let pages = [solid([255, 0, 0, 255]), solid([0, 255, 0, 255])];
        let pixel = |pixels: &[u8], x: usize, y: usize| -> [u8; 4] {
            pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4]
                .try_into()
                .unwrap()
        };
        let near = |a: [u8; 4], b: [u8; 3]| (0..3).all(|i| a[i].abs_diff(b[i]) <= 2);
        let half = Present {
            pivot: [8.0, 8.0],
            scale: 1.0,
            alpha: 0.5,
            rank: 0,
        };
        let mut batch = Batch::default();
        batch.quads.extend([
            shown(0, [2.0, 2.0, 12.0, 12.0], layer(10, 0, 0), Some(half)),
            shown(1, [2.0, 2.0, 12.0, 12.0], layer(10, 0, 1), Some(half)),
        ]);
        let (pixels, pages_used) = render_presented(&device, &queue, &batch, &pages);
        assert_eq!(pages_used, 1);
        assert!(
            near(pixel(&pixels, 8, 8), [0, 128, 0]),
            "{:?}",
            pixel(&pixels, 8, 8)
        );
        assert!(near(pixel(&pixels, 0, 0), [0, 0, 0]));
        let whole = Present {
            scale: 0.5,
            alpha: 1.0,
            ..half
        };
        batch.quads = vec![shown(
            1,
            [0.0, 0.0, 16.0, 16.0],
            layer(10, 0, 0),
            Some(whole),
        )];
        let (pixels, pages_used) = render_presented(&device, &queue, &batch, &pages);
        assert_eq!(pages_used, 0);
        assert!(near(pixel(&pixels, 8, 8), [0, 255, 0]));
        assert!(near(pixel(&pixels, 4, 4), [0, 255, 0]));
        assert!(
            near(pixel(&pixels, 2, 2), [0, 0, 0]),
            "{:?}",
            pixel(&pixels, 2, 2)
        );
        assert!(near(pixel(&pixels, 13, 13), [0, 0, 0]));
        let far = Present {
            rank: 0,
            alpha: 1.0,
            ..half
        };
        let nearer = Present {
            rank: 1,
            alpha: 1.0,
            ..half
        };
        batch.quads = vec![
            shown(1, [2.0, 2.0, 12.0, 12.0], layer(1, 0, 0), Some(nearer)),
            shown(0, [2.0, 2.0, 12.0, 12.0], layer(20, 0, 0), Some(far)),
        ];
        let (pixels, _) = render_presented(&device, &queue, &batch, &pages);
        assert!(
            near(pixel(&pixels, 8, 8), [0, 255, 0]),
            "{:?}",
            pixel(&pixels, 8, 8)
        );
    }

    fn an_icon_wider_than_a_default_fade_page_still_fades_whole(
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) {
        let size = [2200, 8];
        let pages = [solid([255, 0, 0, 255]), solid([0, 255, 0, 255])];
        let quarter = Present {
            pivot: [1100.0, 4.0],
            scale: 1.0,
            alpha: 0.25,
            rank: 0,
        };
        let mut batch = Batch::default();
        batch.quads.extend([
            shown(0, [0.0, 0.0, 2200.0, 8.0], layer(10, 0, 0), Some(quarter)),
            shown(1, [0.0, 0.0, 2200.0, 8.0], layer(10, 0, 1), Some(quarter)),
        ]);
        let (pixels, pages_used) = render_presented_at(&device, &queue, &batch, &pages, size);
        assert_eq!(pages_used, 1);
        let width = size[0] as usize;
        for x in [0, 1100, 2199] {
            let at = (4 * width + x) * 4;
            let pixel = &pixels[at..at + 4];
            assert!(
                pixel[0] <= 2 && pixel[1].abs_diff(64) <= 2 && pixel[2] <= 2,
                "{x}: {pixel:?}"
            );
        }
    }

    fn red_and_green(device: &wgpu::Device, queue: &wgpu::Queue) -> SpritePass {
        let images = IconTextures::new(
            device,
            queue,
            &[solid([255, 0, 0, 255]), solid([0, 255, 0, 255])],
        )
        .unwrap();
        SpritePass::new(
            device,
            queue,
            &images,
            &LazyTextures::new(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap()
    }

    fn half(texture: usize, rect: [f32; 4], rank: u32, child: u32) -> Quad {
        let present = Present {
            pivot: [8.0, 8.0],
            scale: 1.0,
            alpha: 0.5,
            rank,
        };
        shown(texture, rect, layer(10, rank, child), Some(present))
    }

    fn each_fade_page_draws_this_upload_s_icons_alone(device: wgpu::Device, queue: wgpu::Queue) {
        let mut pass = red_and_green(&device, &queue);
        pass.fade.side = 20;
        let arena = shader_pipeline::gpu::Arena::new(&device);
        let at = std::cell::Cell::new(0);
        let frame = |pass: &mut SpritePass, quads: Vec<Quad>| {
            let batch = Batch {
                quads,
                ..Batch::default()
            };
            arena.reset();
            at.set(at.get() + 1);
            pass.upload(&device, &queue, &arena, &batch, [16, 16], 1.);
            arena.flush(&queue);
            let mut encoder = device.create_command_encoder(&Default::default());
            pass.draw_pages(&mut encoder);
            queue.submit([encoder.finish()]);
            let pixels = viewer_gpu::test_support::gpu::render_with(&device, &queue, |render| {
                pass.draw_runs(render, [16, 16])
            });
            let runs: Vec<usize> = pass.fade_runs.iter().map(Vec::len).collect();
            (pixels, pass.fade_pages(), runs, pass.fade.pages.len())
        };
        let check = |pixels: &[u8], expected: &[([usize; 2], [u8; 3])]| {
            for &([x, y], colour) in expected {
                let at = (y * 16 + x) * 4;
                let pixel = &pixels[at..at + 3];
                assert!(
                    (0..3).all(|i| pixel[i].abs_diff(colour[i]) <= 2),
                    "({x}, {y}): {pixel:?} for {colour:?}"
                );
            }
        };
        let (pixels, used, runs, kept) = frame(
            &mut pass,
            vec![
                half(0, [0.0, 0.0, 12.0, 12.0], 0, 0),
                half(1, [4.0, 4.0, 12.0, 12.0], 1, 0),
            ],
        );
        assert_eq!((used, runs, kept), (2, vec![1, 1], 2));
        check(
            &pixels,
            &[
                ([2, 2], [128, 0, 0]),
                ([14, 14], [0, 128, 0]),
                ([8, 8], [64, 128, 0]),
            ],
        );
        let corners = || {
            vec![
                half(0, [0.0, 0.0, 2.0, 2.0], 0, 0),
                half(0, [8.0, 8.0, 2.0, 2.0], 0, 1),
                half(1, [4.0, 4.0, 2.0, 2.0], 1, 0),
                half(1, [14.0, 14.0, 2.0, 2.0], 1, 1),
            ]
        };
        let changed = [
            ([1, 1], [128, 0, 0]),
            ([9, 9], [128, 0, 0]),
            ([5, 5], [0, 128, 0]),
            ([15, 15], [0, 128, 0]),
            ([7, 7], [0, 0, 0]),
            ([12, 12], [0, 0, 0]),
            ([3, 3], [0, 0, 0]),
        ];
        let (pixels, used, runs, kept) = frame(&mut pass, corners());
        assert_eq!((used, runs, kept), (2, vec![1, 1], 2));
        check(&pixels, &changed);
        let (pixels, used, runs, kept) =
            frame(&mut pass, vec![shown(0, [6.0, 6.0, 4.0, 4.0], 0, None)]);
        assert_eq!((used, runs, kept), (0, vec![0, 0], 0));
        check(
            &pixels,
            &[
                ([7, 7], [255, 0, 0]),
                ([1, 1], [0, 0, 0]),
                ([12, 12], [0, 0, 0]),
                ([15, 15], [0, 0, 0]),
            ],
        );
        let (pixels, used, runs, kept) = frame(&mut pass, corners());
        assert_eq!((used, runs, kept), (2, vec![1, 1], 2));
        check(&pixels, &changed);
    }

    fn fade_pages_follow_each_upload_s_demand(device: wgpu::Device, queue: wgpu::Queue) {
        let mut pass = red_and_green(&device, &queue);
        let arena = shader_pipeline::gpu::Arena::new(&device);
        let at = std::cell::Cell::new(0);
        let sizes = |pass: &mut SpritePass, quads: Vec<Quad>, size: [u32; 2]| {
            let batch = Batch {
                quads,
                ..Batch::default()
            };
            arena.reset();
            at.set(at.get() + 1);
            pass.upload(&device, &queue, &arena, &batch, size, 1.);
            pass.fade
                .pages
                .iter()
                .map(|page| page.size)
                .collect::<Vec<_>>()
        };
        let mut alternating = red_and_green(&device, &queue);
        let tall_by_one = vec![half(1, [1.0, 1.0, 254.0, 255.0], 0, 0)];
        let wide_by_one = vec![half(1, [1.0, 1.0, 255.0, 254.0], 0, 0)];
        assert_eq!(
            sizes(&mut alternating, tall_by_one.clone(), [300, 300]),
            [[256, 512]]
        );
        assert_eq!(
            sizes(&mut alternating, wide_by_one.clone(), [300, 300]),
            [[512, 512]]
        );
        let kept = alternating.fade.pages[0].view.clone();
        for quads in [
            tall_by_one.clone(),
            wide_by_one.clone(),
            tall_by_one,
            wide_by_one,
        ] {
            assert_eq!(sizes(&mut alternating, quads, [300, 300]), [[512, 512]]);
            assert_eq!(alternating.fade.pages[0].view, kept);
        }
        let wide = sizes(
            &mut pass,
            vec![half(1, [0.0, 0.0, 2500.0, 8.0], 0, 0)],
            [2500, 8],
        );
        assert_eq!(wide, [[2560, 256]]);
        let tall = sizes(
            &mut pass,
            vec![half(1, [0.0, 0.0, 8.0, 2500.0], 0, 0)],
            [8, 2500],
        );
        assert_eq!(tall, [[256, 2560]]);
        let small = sizes(
            &mut pass,
            vec![half(1, [2.0, 2.0, 4.0, 4.0], 0, 0)],
            [16, 16],
        );
        assert_eq!(small, [[256, 256]]);
        let grown = sizes(
            &mut pass,
            vec![half(1, [0.0, 0.0, 300.0, 8.0], 0, 0)],
            [300, 8],
        );
        assert_eq!(grown, [[512, 256]]);
        let back = sizes(
            &mut pass,
            vec![half(1, [2.0, 2.0, 4.0, 4.0], 0, 0)],
            [16, 16],
        );
        assert_eq!(back, [[512, 256]]);
        pass.fade.side = 20;
        let two = vec![
            half(0, [0.0, 0.0, 12.0, 12.0], 0, 0),
            half(1, [4.0, 4.0, 12.0, 12.0], 1, 0),
        ];
        assert_eq!(sizes(&mut pass, two.clone(), [16, 16]).len(), 2);
        assert_eq!(sizes(&mut pass, two[..1].to_vec(), [16, 16]).len(), 1);
        let plain = vec![shown(0, [6.0, 6.0, 4.0, 4.0], 0, None)];
        assert_eq!(sizes(&mut pass, plain, [16, 16]).len(), 0);
        assert_eq!(sizes(&mut pass, two, [16, 16]).len(), 2);
        assert_eq!(sizes(&mut pass, Vec::new(), [16, 16]).len(), 0);
    }

    fn every_upload_draws_its_own_fading_icons_alone(device: wgpu::Device, queue: wgpu::Queue) {
        let images = IconTextures::new(&device, &queue, &[solid([0, 255, 0, 255])]).unwrap();
        let mut pass = SpritePass::new(
            &device,
            &queue,
            &images,
            &LazyTextures::new(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap();
        let arena = shader_pipeline::gpu::Arena::new(&device);
        let faded = Present {
            pivot: [8.0, 8.0],
            scale: 1.0,
            alpha: 0.5,
            rank: 0,
        };
        let mut batch = Batch::default();
        batch.quads.extend([
            shown(0, [2.0, 2.0, 4.0, 4.0], layer(10, 0, 0), Some(faded)),
            shown(
                0,
                [9.0, 9.0, 4.0, 4.0],
                layer(10, 1, 0),
                Some(Present { rank: 1, ..faded }),
            ),
        ]);
        let counts = |pass: &SpritePass| {
            (
                pass.fade_pages(),
                pass.fade_runs.iter().map(Vec::len).sum::<usize>(),
                pass.fade_draws.len(),
                pass.runs.len(),
            )
        };
        let mut seen = Vec::new();
        for _ in 0..3 {
            arena.reset();
            pass.upload(&device, &queue, &arena, &batch, [16, 16], 1.);
            seen.push(counts(&pass));
        }
        assert_eq!(seen, vec![(1, 2, 0, 2); 3]);
        batch.quads = vec![shown(0, [2.0, 2.0, 4.0, 4.0], layer(10, 0, 0), None)];
        arena.reset();
        pass.upload(&device, &queue, &arena, &batch, [16, 16], 1.);
        assert_eq!(counts(&pass), (0, 0, 0, 1));
    }

    #[test]
    fn scaled_quads_that_touch_in_interface_pixels_still_touch_on_screen() {
        let left = device_corners([10.0, 0.0, 10.0, 10.0], 1.25);
        let right = device_corners([20.0, 0.0, 10.0, 10.0], 1.25);
        assert_eq!(left, [13.0, 0.0, 25.0, 13.0]);
        assert_eq!(right[0], left[2]);
        assert_eq!(right, [25.0, 0.0, 38.0, 13.0]);
        assert_eq!(
            device_corners([0.0, 0.0, 2048.0, 1152.0], 1.25),
            [0.0, 0.0, 2560.0, 1440.0]
        );
    }

    #[test]
    fn bar_preserves_authored_rotation_fill_and_overrun() {
        let vertical = SpriteSheet {
            name: "GFX_bar_v".into(),
            texture: 0,
            frames: 1,
            size: [3, 17],
        };
        let mut batch = Batch::default();
        batch.bar(
            &vertical,
            [26.5, 21.0],
            [3.0, 17.0],
            8.5,
            std::f32::consts::FRAC_PI_2,
            [1.0; 4],
        );
        let partial = &batch.quads[0];
        assert_eq!(partial.rotation, Some(std::f32::consts::FRAC_PI_2));
        assert_eq!(partial.uv, [0.0, 0.0, 1.0, 8.0 / 17.0]);
        let bounds = partial.screen_bounds();
        for (got, want) in bounds.iter().zip([24.0, 28.0, 8.0, 3.0]) {
            assert!((got - want).abs() < 1e-4, "{bounds:?}");
        }
        batch.bar(
            &vertical,
            [31.5, 21.0],
            [3.0, 17.0],
            30.0,
            std::f32::consts::FRAC_PI_2,
            [1.0; 4],
        );
        let full = &batch.quads[1];
        assert_eq!(full.uv, [0.0, 0.0, 1.0, 1.0]);
        let bounds = full.screen_bounds();
        for (got, want) in bounds.iter().zip([24.0, 28.0, 18.0, 3.0]) {
            assert!((got - want).abs() < 1e-4, "{bounds:?}");
        }
        batch.bar(
            &vertical,
            [26.5, 21.0],
            [3.0, 17.0],
            0.0,
            std::f32::consts::FRAC_PI_2,
            [1.0; 4],
        );
        assert_eq!(batch.quads.len(), 2);
        batch.bar(&vertical, [14.0, 8.0], [3.0, 17.0], 10.0, 0.0, [1.0; 4]);
        assert_eq!(batch.quads[2].rotation, Some(0.0));
        assert_eq!(batch.quads[2].rect, [14.0, 11.5, 3.0, 10.0]);
        assert_eq!(batch.quads[2].uv, [0.0, 0.0, 1.0, 10.0 / 17.0]);
        let horizontal = SpriteSheet {
            name: "GFX_bar_h".into(),
            texture: 1,
            frames: 1,
            size: [17, 3],
        };
        batch.bar(&horizontal, [24.0, 28.0], [17.0, 3.0], 8.5, 0.0, [1.0; 4]);
        assert_eq!(batch.quads[3].rotation, Some(0.0));
        assert_eq!(batch.quads[3].rect, [24.0, 28.0, 8.0, 3.0]);
        assert_eq!(batch.quads[3].uv, [0.0, 0.0, 8.0 / 17.0, 1.0]);
    }

    #[test]
    fn bar_preserves_fractional_geometry_at_zero_rotation_under_2x_ui_scale() {
        let vertical = SpriteSheet {
            name: "GFX_bar_v".into(),
            texture: 0,
            frames: 1,
            size: [3, 17],
        };
        let mut batch = Batch::default();
        batch.bar(&vertical, [18.0, 8.0], [3.0, 17.0], 10.0, 0.0, [1.0; 4]);
        batch.bar(&vertical, [18.0, 8.0], [3.0, 17.0], 10.0, 1e-6, [1.0; 4]);
        let zero_corners = plain_quad_corners(&batch.quads[0], 2.0);
        let tiny_corners = plain_quad_corners(&batch.quads[1], 2.0);
        assert_eq!(
            zero_corners,
            [[36.0, 23.0], [42.0, 23.0], [42.0, 43.0], [36.0, 43.0]]
        );
        for (a, b) in zero_corners.iter().zip(tiny_corners.iter()) {
            assert!(
                (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3,
                "zero {zero_corners:?} vs tiny {tiny_corners:?}"
            );
        }
        let placement = fade::Placement::identity([64, 64]);
        let zero_verts = plain_corners(&batch.quads[0], placement, 2.0);
        let tiny_verts = plain_corners(&batch.quads[1], placement, 2.0);
        assert_eq!(zero_verts[0].position, [36.0 / 32.0 - 1.0, 1.0 - 23.0 / 32.0]);
        for (a, b) in zero_verts.iter().zip(tiny_verts.iter()) {
            assert!(
                (a.position[0] - b.position[0]).abs() < 1e-4
                    && (a.position[1] - b.position[1]).abs() < 1e-4,
                "{:?} vs {:?}",
                a.position,
                b.position
            );
        }
    }

    fn patterned_bar_renders_rotated_and_unrotated_fills_with_overrun(
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) {
        let mut vertical_pixels = Vec::with_capacity(3 * 17 * 4);
        for row in 0..17u8 {
            for col in 0..3u8 {
                vertical_pixels.extend_from_slice(&[col * 80 + 40, row * 14 + 10, 200, 255]);
            }
        }
        let mut horizontal_pixels = Vec::with_capacity(17 * 3 * 4);
        for row in 0..3u8 {
            for col in 0..17u8 {
                horizontal_pixels.extend_from_slice(&[row * 80 + 40, col * 14 + 10, 150, 255]);
            }
        }
        let pages = [
            dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
                width: 3,
                height: 17,
                pixels: vertical_pixels,
            }),
            dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
                width: 17,
                height: 3,
                pixels: horizontal_pixels,
            }),
        ];
        let vertical = SpriteSheet {
            name: "GFX_pattern_v".into(),
            texture: 0,
            frames: 1,
            size: [3, 17],
        };
        let horizontal = SpriteSheet {
            name: "GFX_pattern_h".into(),
            texture: 1,
            frames: 1,
            size: [17, 3],
        };
        let target = [24u32, 12];
        let pixel = |pixels: &[u8], x: usize, y: usize| -> [u8; 4] {
            let at = (y * target[0] as usize + x) * 4;
            pixels[at..at + 4].try_into().unwrap()
        };
        let near = |got: [u8; 4], want: [u8; 3]| {
            (0..3).all(|i| got[i].abs_diff(want[i]) <= 2)
        };

        let mut batch = Batch::default();
        batch.bar(
            &vertical,
            [4.5, -3.0],
            [3.0, 17.0],
            8.0,
            15708.0_f32 / 10_000.0,
            [1.0; 4],
        );
        let (pixels, _) = render_presented_at(&device, &queue, &batch, &pages, target);
        for dx in 0..8usize {
            for dy in 0..3usize {
                let got = pixel(&pixels, 2 + dx, 4 + dy);
                let want = [((2 - dy) as u8) * 80 + 40, (dx as u8) * 14 + 10, 200];
                assert!(near(got, want), "rot partial ({dx},{dy}): {got:?} vs {want:?}");
            }
        }
        assert!(near(pixel(&pixels, 10, 4), [0, 0, 0]));

        batch.clear();
        batch.bar(
            &vertical,
            [9.5, -3.0],
            [3.0, 17.0],
            18.0,
            std::f32::consts::FRAC_PI_2,
            [1.0; 4],
        );
        let (pixels, _) = render_presented_at(&device, &queue, &batch, &pages, target);
        assert!(
            near(pixel(&pixels, 2, 4), [200, 10, 200]),
            "{:?}",
            pixel(&pixels, 2, 4)
        );
        assert!(
            near(pixel(&pixels, 2, 6), [40, 10, 200]),
            "{:?}",
            pixel(&pixels, 2, 6)
        );
        assert!(
            near(pixel(&pixels, 19, 4), [200, 234, 200]),
            "{:?}",
            pixel(&pixels, 19, 4)
        );
        assert!(
            near(pixel(&pixels, 19, 6), [40, 234, 200]),
            "{:?}",
            pixel(&pixels, 19, 6)
        );
        assert!(near(pixel(&pixels, 20, 4), [0, 0, 0]));

        batch.clear();
        batch.bar(
            &vertical,
            [13.5, -3.0],
            [3.0, 17.0],
            8.0,
            -std::f32::consts::FRAC_PI_2,
            [1.0; 4],
        );
        let (pixels, _) = render_presented_at(&device, &queue, &batch, &pages, target);
        assert!(
            near(pixel(&pixels, 18, 4), [40, 10, 200]),
            "{:?}",
            pixel(&pixels, 18, 4)
        );
        assert!(
            near(pixel(&pixels, 11, 6), [200, 108, 200]),
            "{:?}",
            pixel(&pixels, 11, 6)
        );
        assert!(near(pixel(&pixels, 10, 4), [0, 0, 0]));

        batch.clear();
        batch.bar(&horizontal, [2.0, 4.0], [17.0, 3.0], 8.0, 0.0, [1.0; 4]);
        let (pixels, _) = render_presented_at(&device, &queue, &batch, &pages, target);
        for dx in 0..8usize {
            for dy in 0..3usize {
                let got = pixel(&pixels, 2 + dx, 4 + dy);
                let want = [(dy as u8) * 80 + 40, (dx as u8) * 14 + 10, 150];
                assert!(near(got, want), "unrot partial ({dx},{dy}): {got:?} vs {want:?}");
            }
        }
        assert!(near(pixel(&pixels, 10, 4), [0, 0, 0]));

        batch.clear();
        batch.bar(&horizontal, [2.0, 4.0], [17.0, 3.0], 18.0, 0.0, [1.0; 4]);
        let (pixels, _) = render_presented_at(&device, &queue, &batch, &pages, target);
        assert!(
            near(pixel(&pixels, 2, 4), [40, 10, 150]),
            "{:?}",
            pixel(&pixels, 2, 4)
        );
        assert!(
            near(pixel(&pixels, 19, 6), [200, 234, 150]),
            "{:?}",
            pixel(&pixels, 19, 6)
        );
        assert!(near(pixel(&pixels, 20, 4), [0, 0, 0]));
    }
}

#[cfg(test)]
mod layer_tests {
    use super::*;

    #[test]
    fn only_source_over_colour_flattens_into_a_layer() {
        let over = wgpu::BlendState {
            color: SOURCE_OVER,
            alpha: wgpu::BlendComponent::REPLACE,
        };
        assert_eq!(unflattenable(Some(over), wgpu::ColorWrites::ALL), None);
        assert_eq!(unflattenable(Some(over), wgpu::ColorWrites::COLOR), None);
        assert!(
            unflattenable(Some(over), wgpu::ColorWrites::RED)
                .unwrap()
                .contains("writes only")
        );
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        assert!(
            unflattenable(Some(additive), wgpu::ColorWrites::ALL)
                .unwrap()
                .contains("blends colour")
        );
        assert!(
            unflattenable(None, wgpu::ColorWrites::ALL)
                .unwrap()
                .contains("without blending")
        );
    }
}
