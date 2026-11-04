//! Screen-space sprites and compiled GUI draws.
//! Callers lay out rectangles in interface pixels. The simple atlas path snaps their corners
//! to device pixels and supplies clip-space vertices; compiled GUI draws retain their own
//! geometry, constants, sampler states and scissors. Layer keys preserve icon-child ordering
//! while grouping compatible glyphs by texture. The pass draws after the world without depth.

#[cfg(test)]
use crate::map::test_support::test_tga_rgba8;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::{collections::HashMap, io, sync::Arc};

#[cfg(test)]
pub(super) mod oracle;

use eframe::wgpu;
use installed_content_core::{ResolvedLayers, gui::native::resources};
use installed_content_gfx::gfx;
use installed_content_image::{dds, native::Decode};

use super::{
    free_icons::{IconAnchor, Present},
    gpu_contract::{Targets, sampler_entry, texture_entry},
    lazy_icons::{DYNAMIC_BASE, LazyTextures},
    text,
    textures::{Bound, ColorSpace, upload},
};

mod fade;
mod packets;
mod retained;

pub use retained::CameraFrame;

/// One frame-animated sprite: a horizontal strip of equal-width frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteSheet {
    /// The `GFX_...` name.
    pub name: String,
    /// Texture id in the shared static image list.
    pub texture: usize,
    /// `noOfFrames`.
    pub frames: u32,
    /// Whole-strip size in texels.
    pub size: [u32; 2],
}

impl SpriteSheet {
    /// One frame's size, with its horizontal extent truncated to whole texels.
    pub fn frame_size(&self) -> [f32; 2] {
        [
            (self.size[0] / self.frames.max(1)) as f32,
            self.size[1] as f32,
        ]
    }

    /// The atlas rectangle of a zero-based frame, clamped to the last frame.
    /// Frames retain their evenly divided strip pitch even when their width is fractional.
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

/// The interface files as the game read them (the resources and each sprite by name), and the
/// client's texture decode state.
pub struct Interface {
    pub registry: installed_content_core::gui::native::resources::Registry,
    pub sprites: gfx::SpriteSet,
    pub decode: Decode,
}

/// What a refused GUI shader draw drew, by identity: its quad's path, effect file and
/// technique ([`shader_pipeline::gpu::Arena::refuse`]).
pub fn gui_refusal(draw: &super::native_shader::draw::Quad) -> String {
    format!(
        "GUI shader {:?}: effect {:?} technique {:?}",
        draw.path, draw.effect.asset.source.effect, draw.effect.technique
    )
}

/// Loads one declared sprite and appends its texture to `images`.
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

/// Reads and decodes one declared sprite without appending it anywhere, so
/// callers can decode sheets on scoped threads and append in a fixed order.
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
    // A sheet the visual mods break is the recording's own, named.
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

/// One textured quad in device pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    /// Texture id in the shared static image list.
    pub texture: usize,
    /// `x, y, width, height` in device pixels, `y` down from the top.
    pub rect: [f32; 4],
    /// Normalised atlas rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Straight RGBA multiplier; `[1, 1, 1, 1]` draws the texture as authored.
    pub color: [f32; 4],
    // counters
    /// Clockwise rotation about the rectangle's centre, in radians. Zero retains the snapped
    /// axis-aligned path.
    pub rotation: f32,
    // counters
    /// The sort key [`layer`] builds; see it for the field order.
    pub layer: u64,
    /// Signed brightness adjustment after tinting: positive brightens, negative darkens.
    pub shade: f32,
    /// A quad [`super::native_gui`] places and shades as the game does; `rect`, `uv`, `rotation`
    /// and `shade` are then unused.
    pub exact: Option<Exact>,
    /// The map icon the quad belongs to, which a free eye shows standing in the world
    /// ([`super::free_icons`]); `None` for screen interface.
    pub anchor: Option<IconAnchor>,
    /// How a free eye shows the quad's icon ([`super::free_icons::present`]); `None` draws the
    /// quad as laid out, in layer order.
    pub present: Option<Present>,
}

/// An unsnapped GUI draw with compiled resources, or layout-fixture geometry and metadata.
/// Its scissor clips interface-pixel corners after scaling. Compiled draws carry their own
/// complete vertex and constant payload in `native`.
#[derive(Clone, Debug, PartialEq)]
pub struct Exact {
    pub native: Option<Arc<super::native_shader::draw::Quad>>,
    /// Where the vertices land, in interface pixels.
    pub corners: [[f32; 2]; 4],
    /// Each vertex's texture coordinate as the vertex buffer holds it (before `Offset`).
    pub uv: [[f32; 2]; 4],
    /// The scissor `x0, y0, x1, y1` in whole interface pixels: the device's is each coordinate
    /// times the interface scale, truncated.
    pub scissor: Option<[i32; 4]>,
    /// Selected technique ID for compiler-independent layout fixtures.
    pub technique: u32,
    pub blend: super::native_effects::Blend,
    /// The samplers of slot 0, slot 1 and the masking slot 5, ten bits each from bit 0, 10 and
    /// 20 ([`super::native_effects::Sampler::packed`]).
    pub samplers: u32,
    /// Each slot's `MipMapLodBias` (slot 0, slot 1, slot 5).
    pub lod_bias: [f32; 3],
    /// Slot 1 (a progress bar's `TextureTwo`) and slot 5 (the masking texture).
    pub second: Option<usize>,
    pub masking: Option<usize>,
    /// `Offset` and `NextOffset`.
    pub offsets: [f32; 4],
    /// `Time - AnimationTime`, `AnimationTime` and a progress bar's `CurrentState`.
    pub inputs: [f32; 4],
    /// A progress bar's `vSecondColor`, or a text's `ModColor`.
    pub second_color: [f32; 4],
}

// counters
/// A quad's sort key: the engine's icon priority, then the icon's own place in
/// the pass, then the child's place inside its window.
///
/// Quads are uploaded sorted by `(layer, texture)`. The engine sorts map icons
/// by the content priorities and draws each icon's children in
/// the order `mapicons.gui` declares them, so **only quads that are the same
/// child of the same icon may be reordered** — which is what batches a run of
/// glyphs onto one page without ever letting one counter's flag overtake
/// another counter's plate. Each icon has [`CHILD_SLOTS`] child slots, more
/// than the land counter's 19 child kinds; a caller that would need more
/// refuses rather than share a slot.
///
/// The key is 64-bit so every `u32` icon ordinal keeps its own range under its
/// priority: the icon count grows with the countries and units a recording
/// holds, and a narrower key let icon 62,500 of one priority collide with the
/// next priority's first icon. The signed priority maps onto bands 0 to 255 in order.
pub const fn layer(priority: i8, icon: u32, child: u32) -> u64 {
    band_layer((priority as u8 ^ 0x80) as u64, icon, child)
}

/// A window containing the active GUI object adds five to its signed content priority.
/// The sum can exceed i8::MAX, so it must not wrap back below the map.
pub const fn hovered_layer(priority: i8, hovered: bool, icon: u32, child: u32) -> u64 {
    band_layer(
        (priority as i32 + 128 + if hovered { 5 } else { 0 }) as u64,
        icon,
        child,
    )
}

/// A key in the band above every map-icon priority, where the combat window
/// draws over the map icons as the GUI does.
pub const fn gui_layer(icon: u32, child: u32) -> u64 {
    band_layer(GUI_BAND, icon, child)
}

const fn band_layer(band: u64, icon: u32, child: u32) -> u64 {
    band * PRIORITY_STRIDE + icon as u64 * CHILD_SLOTS as u64 + child as u64
}

/// Reads an icon's content priority as a sign-extended low byte.
pub fn priority(defines: &installed_content_core::defines::Defines, name: &str) -> io::Result<i8> {
    Ok(defines.int32("NMapIcons", name)? as i8)
}

/// Child slots per icon in a [`layer`] key.
pub const CHILD_SLOTS: u32 = 32;

/// Keys one priority spans: room for every `u32` icon of [`CHILD_SLOTS`] children.
const PRIORITY_STRIDE: u64 = 1 << 40;
const _: () = assert!((u32::MAX as u64 + 1) * CHILD_SLOTS as u64 <= PRIORITY_STRIDE);
/// The band [`gui_layer`] keys, above every signed-byte map-icon priority
/// including the native five-point GUI-hover lift.
const GUI_BAND: u64 = 261;

/// Tooltips use the final GUI pass above ordinary windows.
pub const TOOLTIP_LAYER: u64 = band_layer(GUI_BAND + 1, 0, 0);

/// The priority a [`layer`] key was built with.
#[cfg(any(test, feature = "test-support"))]
pub const fn layer_priority(layer: u64) -> i64 {
    (layer / PRIORITY_STRIDE) as i64 - 128
}

/// The icon ordinal of a [`layer`] key, within its priority.
#[cfg(any(test, feature = "test-support"))]
pub const fn layer_icon(layer: u64) -> u64 {
    layer % PRIORITY_STRIDE / CHILD_SLOTS as u64
}

/// What moves a [`layer`] key by `icons` whole icons.
pub const fn icon_offset(icons: u32) -> u64 {
    icons as u64 * CHILD_SLOTS as u64
}

/// A frame's worth of screen-space quads, in the order the caller pushed them.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub quads: Vec<Quad>,
    // counters
    /// The layer subsequent pushes are tagged with.
    layer: u64,
    /// The map icon subsequent pushes belong to.
    anchor: Option<IconAnchor>,
    /// A named part of the icon standing on its own anchor: parts whose element path ends with
    /// the name take it instead ([`Batch::at_part_anchor`]).
    part_anchor: Option<(String, IconAnchor)>,
    /// What the current part's pushes take ([`Batch::at_part`]).
    current: Option<IconAnchor>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// Empties the batch as [`Batch::default`] does, keeping its storage.
    pub fn clear(&mut self) {
        self.quads.clear();
        self.layer = 0;
        self.anchor = None;
        self.part_anchor = None;
        self.current = None;
    }

    // counters
    /// Tags every later push with this sort layer.
    pub fn at_layer(&mut self, layer: u64) {
        self.layer = layer;
    }

    /// Tags every later push as part of the map icon standing at `anchor`; `None` for screen
    /// interface. Forgets any named part's own anchor.
    pub fn at_anchor(&mut self, anchor: Option<IconAnchor>) {
        self.anchor = anchor;
        self.part_anchor = None;
        self.current = anchor;
    }

    /// Gives the icon's parts whose element path ends with `name` their own `anchor`, as a city
    /// label's name fades by its own cutoff. Later [`Batch::at_part`] calls apply it.
    pub fn at_part_anchor(&mut self, name: &str, anchor: IconAnchor) {
        self.part_anchor = Some((name.to_owned(), anchor));
    }

    /// Later pushes draw the GUI part at element `path`: they take its own anchor when the icon
    /// gave it one, else the icon's.
    pub fn at_part(&mut self, path: &str) {
        self.current = match &self.part_anchor {
            Some((name, anchor)) if path.ends_with(name.as_str()) => Some(*anchor),
            _ => self.anchor,
        };
    }

    /// Adds `quad` at the current layer, as part of the current icon.
    pub fn push(&mut self, quad: Quad) {
        self.quads.push(Quad {
            layer: self.layer,
            anchor: self.current,
            ..quad
        });
    }

    // counters
    /// Adds one sprite frame drawn into an explicit device-pixel size.
    ///
    /// `mapicons.gui` gives several children a `scale`, and the flag cells are
    /// drawn from an atlas whose cell is not the sprite's frame, so the size
    /// cannot always come from the sheet.
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
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        });
    }

    // counters
    /// Adds an explicit atlas rectangle of an already-known page.
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
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        });
    }

    // counters
    /// Adds the leftmost `pixels` of one frame, clipping the rectangle and the uv.
    ///
    /// This is how the counter bars are filled: `mapicons.gui` declares
    /// `bar_org` and `bar_str` with `rotation = 1.5708`, which turns the
    /// 3 x 17 texture into a 17 x 3 horizontal bar. Both textures are a
    /// **single flat colour** — `mapicon_bar_org_sprite.dds` is
    /// `(105, 195, 115)` in every one of its 51 texels and
    /// `mapicon_bar_str_sprite.dds` is `(195, 145, 75)` — so the quarter turn
    /// produces exactly the same pixels as drawing the frame into the rotated
    /// rectangle, and the caller passes the already-rotated size.
    pub fn bar(
        &mut self,
        sheet: &SpriteSheet,
        at: [f32; 2],
        size: [f32; 2],
        pixels: f32,
        color: [f32; 4],
    ) {
        // A full native counter bar covers one pixel past its sprite
        // (the counter bar geometry); the texture is one flat colour, so the quad
        // simply stretches over it.
        let width = pixels.clamp(0.0, size[0] + 1.0).floor();
        if width < 1.0 {
            return;
        }
        let fill = (width / size[0]).min(1.0);
        let [u0, v0, u1, v1] = sheet.frame_uv(0);
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], width, size[1]],
            uv: [u0, v0, u0 + (u1 - u0) * fill, v1],
            color,
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        });
    }

    /// Adds a laid-out run of glyphs.
    pub fn text(&mut self, glyphs: &[text::TextGlyph], color: [f32; 4]) {
        self.quads.extend(glyphs.iter().map(|glyph| Quad {
            texture: glyph.page,
            rect: glyph.rect,
            uv: glyph.uv,
            color,
            rotation: 0.0,
            layer: self.layer,
            shade: 0.0,
            exact: None,
            anchor: self.current,
            present: None,
        }));
    }
}

/// A quad's `x, y, width, height` in interface pixels as device-pixel corners
/// `x0, y0, x1, y1`.
///
/// The rectangle is rounded to whole interface pixels first: the layout adds up
/// `.gui` integers on top of a projected anchor, so the fractional part is the
/// anchor's alone, and keeping it would resample a 1:1 sprite. The corners are
/// then scaled and rounded to device pixels, corners rather than sizes, so
/// quads that touch in interface pixels still touch on screen and every edge
/// stays on the pixel grid. At a scale of 1.0 this is the plain rounding.
pub fn device_corners(rect: [f32; 4], ui_scale: f32) -> [f32; 4] {
    let x0 = rect[0].round();
    let y0 = rect[1].round();
    let corners = [x0, y0, x0 + rect[2].round(), y0 + rect[3].round()];
    corners.map(|value| (value * ui_scale).round())
}

/// An exact quad's scissor in device pixels, `x, y, width, height`, within a target of `size`:
/// Each coordinate is scaled and truncated, then clipped to the target bounds.
/// `None` when nothing is left.
pub fn device_scissor(scissor: [i32; 4], ui_scale: f32, size: [u32; 2]) -> Option<[u32; 4]> {
    let device = scissor.map(|value| (value as f32 * ui_scale) as i64);
    let x0 = device[0].clamp(0, i64::from(size[0]));
    let y0 = device[1].clamp(0, i64::from(size[1]));
    let x1 = device[2].clamp(0, i64::from(size[0]));
    let y1 = device[3].clamp(0, i64::from(size[1]));
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0].map(|value| value as u32))
}

/// The texel-centre window a quad's atlas rectangle is sampled inside, as the
/// shader uses: the rectangle inset by half a texel on every
/// side, so no bilinear tap reaches a neighbouring frame, glyph or flag cell.
/// A rectangle narrower than a texel collapses to its centre.
///
/// A 1:1 or minified quad on whole device pixels already samples inside it,
/// so this changes nothing there. A magnified one, which a UI scale above 1.0
/// draws, samples its edge pixels past the last texel centre; clamping them to
/// the window repeats the cell's edge texels, which is what an edge-extended
/// gutter around the cell would hold. Tests use this window to check atlas bounds.
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

/// `position`, `uv` and `color`, already in clip space; `cell` is the quad's
/// whole atlas rectangle and `filter` one of [`filter_mode`]'s values.
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

/// How the shader samples a batch drawn at `ui_scale`: 0 the plain bilinear
/// tap of the unscaled layer, whose output a scale of 1.0 keeps exactly; 1 that
/// tap clamped to the cell's texel-centre window when magnified; 2 clamped and averaged over
/// every texel a pixel covers when minified, so the icon does not alias.
pub fn filter_mode(ui_scale: f32) -> f32 {
    if ui_scale == 1.0 {
        0.0
    } else if ui_scale > 1.0 {
        1.0
    } else {
        2.0
    }
}

/// Every icon page on the GPU, shared by the sprite batch and the name pass.
///
/// Both passes sample the same font atlases, so the pages are uploaded once and
/// each pass builds its own bind groups over them with the sampler its shader
/// declares.
pub struct IconTextures {
    bound: Vec<Bound>,
    /// Clamped and pinned to the top mip: what a 1:1 screen quad needs.
    screen: wgpu::Sampler,
}

impl IconTextures {
    pub fn views(&self) -> Vec<wgpu::TextureView> {
        self.bound.iter().map(|bound| bound.view.clone()).collect()
    }

    /// Uploads every page in image order.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &[dds::Texture],
    ) -> io::Result<Self> {
        let mut bound = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            bound.push(upload(
                device,
                queue,
                &format!("map icon {index}"),
                image,
                ColorSpace::Linear,
            )?);
        }
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
            // The batch samples the top mip only: at 1:1 on integer pixel
            // corners that is the only one a sample can land in, and a mip of
            // an atlas would blend neighbouring cells. Scaled quads keep to
            // their cells through the shader's window and area filter instead
            // (`SpritePass::upload`); saying so outright keeps a driver's
            // derivative estimate out of the result.
            screen: clamp("icon screen sampler", wgpu::MipmapFilterMode::Nearest, 0.0),
        })
    }

    /// One bind group per page, in page order, for a texture + sampler layout.
    pub fn bind_all(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
    ) -> Vec<wgpu::BindGroup> {
        self.bound
            .iter()
            .map(|texture| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("icon page"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                    ],
                })
            })
            .collect()
    }

    pub fn screen_clamp(&self) -> &wgpu::Sampler {
        &self.screen
    }

    /// Pages uploaded, for reporting.
    pub fn page_count(&self) -> usize {
        self.bound.len()
    }
}

/// Which pipeline a run draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pipe {
    /// The snapped screen-space sprites.
    Legacy,
    Native(usize),
    /// A fading icon's cell of a fade page, drawn over the frame ([`fade`]).
    Composite(usize),
}

/// One run of consecutive vertices sharing a pipeline, its textures and a scissor.
struct Run {
    pipe: Pipe,
    texture: usize,
    scissor: Option<[u32; 4]>,
    range: std::ops::Range<u32>,
}

/// Where a quad draws: the frame, or a fading icon's cell (`[x0, y0, x1, y1]` page pixels) of a
/// fade page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Frame,
    Page { page: usize, cell: [u32; 4] },
}

/// Opacity from which an icon draws straight onto the frame.
const OPAQUE: f32 = 1.0 - 0.5 / 255.0;

/// A lazy texture on the GPU, at the revision it was uploaded from.
struct DynamicPage {
    revision: u64,
    texture: Bound,
    group: wgpu::BindGroup,
}

/// A slot no texture fills: [`SpritePass`] binds its one transparent texel there.
pub const NO_TEXTURE: usize = usize::MAX;

/// The screen-space pipeline and the frame's vertices.
pub struct SpritePass {
    /// These submissions outlive a scene frame, so no other pass may reset their uploads.
    uploads: Option<shader_pipeline::gpu::Arena>,
    retained: Option<retained::Upload>,
    retained_valid: bool,
    retained_diagnostics: installed_content_core::content::diagnostics::Diagnostics,
    camera_moving: bool,
    packets: packets::Cache,
    packet_views: Vec<(u32, View)>,
    bake_views: Vec<(u32, View)>,
    native: super::native_shader::stream::Stream,
    native_draws: Vec<super::native_shader::stream::Submission>,
    native_format: wgpu::TextureFormat,
    native_samples: u32,
    bakes: Vec<CachedBake>,
    bake_stream: super::native_shader::stream::Stream,
    /// Refusals of the most recent frame, by effect/path/compiler reason.
    pub native_refusals: BTreeMap<String, u64>,
    pipeline: wgpu::RenderPipeline,
    bind_groups: Vec<wgpu::BindGroup>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Static page views used by compiled effect bindings.
    views: Vec<wgpu::TextureView>,
    /// One transparent texel for an empty slot.
    empty: Bound,
    /// The content's lazy textures, mirrored into `dynamic` by [`SpritePass::sync`].
    lazy: Arc<LazyTextures>,
    synced: Option<u64>,
    dynamic: HashMap<usize, DynamicPage>,
    /// Lazy textures the device refused, at the revision it refused.
    failed: HashMap<usize, u64>,
    vertices: wgpu::Buffer,
    capacity: u64,
    runs: Vec<Run>,
    count: u32,
    /// Fade pages: the composite program, the pages and the snapped program drawing into them.
    fade: fade::Pages,
    fade_legacy: wgpu::RenderPipeline,
    /// Compiled draws into fade pages, with the binds they inherit from the frame's stream.
    fade_stream: super::native_shader::stream::Stream,
    fade_draws: Vec<super::native_shader::stream::Submission>,
    /// Each fade page's runs, in draw order, by page; cleared every upload, storage kept.
    fade_runs: Vec<Vec<Run>>,
    fade_vertices: wgpu::Buffer,
    fade_capacity: u64,
    composite_vertices: wgpu::Buffer,
    composite_capacity: u64,
    /// Fade pages the last upload filled.
    fade_pages: usize,
    /// The vertex lists an upload fills, kept between frames for their storage.
    scratch: Scratch,
    /// The frame is a layer composited later ([`SpritePass::set_layer`]).
    layer: bool,
}

/// One upload's lists: the frame's snapped quads, the fade pages' and the composites' vertices,
/// and the free eye's icons.
#[derive(Default)]
struct Scratch {
    order: Vec<usize>,
    frame: Vec<Vertex>,
    pages: Vec<Vertex>,
    composites: Vec<fade::CompositeVertex>,
    icons: Vec<Icon>,
}

/// A free eye's icon in one upload: its quads (a run of the draw order), where they land, and for
/// a fading icon its frame rectangle `[x, y, width, height]`, opacity and fade-page cell.
#[derive(Clone, Debug)]
struct Icon {
    quads: std::ops::Range<usize>,
    placement: fade::Placement,
    fading: Option<([u32; 4], f32)>,
    cell: (usize, [u32; 2]),
}

/// Vertices a freshly built pass can hold before it grows; six per quad.
const INITIAL_VERTICES: u64 = 6 * 4096;

impl SpritePass {
    /// Builds the pipeline over already-uploaded icon pages.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
        textures: &IconTextures,
        lazy: &Arc<LazyTextures>,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite texture"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                sampler_entry(1),
            ],
        });
        let bind_groups = textures.bind_all(device, &layout, textures.screen_clamp());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprites"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprites"),
            bind_group_layouts: &[Some(&layout)],
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
        // A fade page keeps premultiplied colour: the program's alpha accumulates
        // `One, OneMinusSrcAlpha` and is written.
        let page_targets = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let snapped = |label, samples, targets: &[Option<wgpu::ColorTargetState>]| {
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
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
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
        let fade_legacy = snapped("sprite fade page", 1, &page_targets);
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
            // Priorities order the final icon layer; they do not participate in depth tests.
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &legacy_targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        let empty = upload(
            device,
            queue,
            "empty sprite slot",
            &dds::Texture::from_rgba8(installed_content_image::Rgba8Image {
                width: 1,
                height: 1,
                pixels: vec![0; 4],
            }),
            ColorSpace::Linear,
        )?;
        Ok(Self {
            native: Default::default(),
            uploads: Some(shader_pipeline::gpu::Arena::new(device)),
            retained: None,
            retained_valid: false,
            retained_diagnostics: Default::default(),
            camera_moving: false,
            packets: Default::default(),
            packet_views: Vec::new(),
            bake_views: Vec::new(),
            native_draws: Vec::new(),
            bakes: Vec::new(),
            bake_stream: Default::default(),
            native_format: format,
            native_samples: samples,
            native_refusals: Default::default(),
            pipeline,
            bind_groups,
            layout,
            sampler: textures.screen_clamp().clone(),
            views: textures
                .bound
                .iter()
                .map(|texture| texture.view.clone())
                .collect(),
            empty,
            lazy: Arc::clone(lazy),
            synced: None,
            dynamic: HashMap::new(),
            failed: HashMap::new(),
            vertices: buffer(device, INITIAL_VERTICES, std::mem::size_of::<Vertex>()),
            capacity: INITIAL_VERTICES,
            runs: Vec::new(),
            count: 0,
            fade: fade::Pages::new(device, format, samples),
            fade_legacy,
            fade_stream: Default::default(),
            fade_draws: Vec::new(),
            fade_runs: Vec::new(),
            fade_vertices: buffer(device, INITIAL_VERTICES, std::mem::size_of::<Vertex>()),
            fade_capacity: INITIAL_VERTICES,
            composite_vertices: buffer(
                device,
                INITIAL_VERTICES,
                std::mem::size_of::<fade::CompositeVertex>(),
            ),
            composite_capacity: INITIAL_VERTICES,
            fade_pages: 0,
            scratch: Scratch::default(),
            layer: false,
        })
    }

    /// Mirrors the content's lazy textures: uploads new and changed ones and
    /// drops evicted ones. Costs one atomic read when nothing changed.
    ///
    /// A texture the device refuses is warned about once and left undrawn,
    /// rather than failing every frame's render; a later revision of it is
    /// tried again.
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
            let texture = match upload(
                device,
                queue,
                &format!("lazy icon {}", id - DYNAMIC_BASE),
                &entry.texture,
                ColorSpace::Linear,
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
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lazy icon"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.dynamic.insert(
                id,
                DynamicPage {
                    revision: entry.revision,
                    texture,
                    group,
                },
            );
        }
        // A combination may name a page that changed or left.
        self.synced = Some(version);
    }

    /// The bind group of a static page or a resident lazy texture.
    fn group(&self, texture: usize) -> Option<&wgpu::BindGroup> {
        self.bind_groups
            .get(texture)
            .or_else(|| self.dynamic.get(&texture).map(|page| &page.group))
    }

    /// The view of a static page or a resident lazy texture, the empty texel for no texture.
    fn view(&self, texture: usize) -> Option<&wgpu::TextureView> {
        if texture == NO_TEXTURE {
            return Some(&self.empty.view);
        }
        self.views
            .get(texture)
            .or_else(|| self.dynamic.get(&texture).map(|page| &page.texture.view))
    }

    /// Decline retention for the next upload while the drawn camera changes. A moving
    /// frame uses the caller's arena; one stationary frame repopulates the retained uploads.
    /// This hint can only give up a hit, never admit one without the complete upload key.
    pub fn set_camera_moving(&mut self, moving: bool) {
        self.camera_moving = moving;
    }

    /// Turns changed quads into vertices, grouped by texture. Compiled GUI draws keep their
    /// own upload arena until the batch, viewport, scale or resident artwork changes. The
    /// scene's frame `arena` receives the same refusal names, including on retained frames.
    ///
    /// `batch` is in interface pixels and `ui_scale` device pixels per
    /// interface pixel; see [`device_corners`] for the snapping and
    /// [`filter_mode`] for the sampling. [`Exact`] quads are not snapped, keep their push
    /// order within their layer and are cut by their scissor.
    #[allow(clippy::too_many_arguments)]
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
    ) {
        let ((), diagnostics) = installed_content_core::content::diagnostics::capture(|| {
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
        // The lazy textures this frame draws stay pinned in their LRU.
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
        let moving = std::mem::take(&mut self.camera_moving);
        if moving {
            // Keep snapshot storage, but its submissions are no longer the current ones.
            self.retained_valid = false;
            self.upload_changed(device, queue, arena, batch, size, ui_scale);
            return;
        }
        let uploads = self
            .uploads
            .take()
            .expect("sprite uploads belong to the pass");
        let same = self.retained_valid
            && self
                .retained
                .as_ref()
                .is_some_and(|retained| retained.matches(batch, size, ui_scale, self.synced));
        if same {
            self.retained_diagnostics.credit();
        } else {
            uploads.reset();
            self.upload_changed(device, queue, &uploads, batch, size, ui_scale);
            uploads.flush(queue);
            match self.retained.as_mut() {
                Some(retained) => retained.update(batch, size, ui_scale, self.synced),
                None => {
                    self.retained = Some(retained::Upload::new(batch, size, ui_scale, self.synced));
                }
            }
        }
        self.retained_valid = true;
        for reason in uploads.refusals() {
            arena.refuse(&reason);
        }
        self.uploads = Some(uploads);
    }

    fn upload_changed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
    ) {
        self.packets.begin();
        self.runs.clear();
        self.native_draws.clear();
        // The fade pages' draws are this frame's alone.
        self.fade_draws.clear();
        for runs in &mut self.fade_runs {
            runs.clear();
        }
        self.fade_pages = 0;
        for bake in &mut self.bakes {
            bake.used = false;
        }
        self.native_refusals.clear();
        self.native.refused.clear();
        self.native.invalidate_bindings();
        self.count = 0;
        if batch.is_empty() || size[0] == 0 || size[1] == 0 {
            self.bakes.clear();
            self.fade.fit(device, &[]);
            self.packets.finish();
            return;
        }
        let Scratch {
            mut order,
            mut frame,
            mut pages,
            mut composites,
            mut icons,
        } = std::mem::take(&mut self.scratch);
        order.clear();
        order.extend(0..batch.quads.len());
        // counters: `(layer, texture)`, so a page batch never overtakes an
        // icon child that has to draw under or over another. An exact quad keeps its push
        // order in its layer: the game's GUI draws in attach order, whatever the texture.
        // A free eye's icons come first, farthest first, each in that order among its own quads;
        // the screen interface follows them.
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
        frame.clear();
        pages.clear();
        composites.clear();
        icons.clear();
        // The free eye's icons come first: each the run of quads sharing a rank, and for a fading
        // one the frame pixels it covers, placed and within the frame.
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
                // A fading icon with no pixels on the frame draws nothing.
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
        // The fading icons' cells, on pages as large as this frame's largest cell needs, and each
        // page then as large as what it holds; pages no icon uses this frame go.
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
                self.emit(
                    (device, queue, arena),
                    &batch.quads[index],
                    placement,
                    target,
                    ui_scale,
                    (&mut frame, &mut pages),
                );
            }
        }
        // The screen interface follows, where it was laid out.
        for &index in &order[screen..] {
            self.emit(
                (device, queue, arena),
                &batch.quads[index],
                fade::Placement::identity(size),
                Target::Frame,
                ui_scale,
                (&mut frame, &mut pages),
            );
        }
        self.bakes.retain(|bake| bake.used);
        self.packets.finish();
        self.count = frame.len() as u32;
        write_vertices(
            device,
            queue,
            &mut self.vertices,
            &mut self.capacity,
            &frame,
        );
        write_vertices(
            device,
            queue,
            &mut self.fade_vertices,
            &mut self.fade_capacity,
            &pages,
        );
        write_vertices(
            device,
            queue,
            &mut self.composite_vertices,
            &mut self.composite_capacity,
            &composites,
        );
        self.scratch = Scratch {
            order,
            frame,
            pages,
            composites,
            icons,
        };
    }

    /// Draws `quad` at `placement` onto `target`: snapped quads become vertices in `frame` or
    /// `pages`, compiled draws become submissions. A draw into a fade page takes the frame
    /// stream's binds at this point and leaves its own behind, as drawing it on the frame would.
    fn emit(
        &mut self,
        (device, queue, arena): (&wgpu::Device, &wgpu::Queue, &shader_pipeline::gpu::Arena),
        quad: &Quad,
        placement: fade::Placement,
        target: Target,
        ui_scale: f32,
        (frame, pages): (&mut Vec<Vertex>, &mut Vec<Vertex>),
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
                self.native.invalidate_bindings();
                *self
                    .native_refusals
                    .entry("GUI quad has no compiled effect payload".into())
                    .or_default() += 1;
                arena.refuse("GUI shader: a quad without a compiled effect");
                return;
            };
            let clip = (!placement.is_identity()).then(|| placement.clip());
            let prepared = match target {
                Target::Frame => {
                    self.prepare_native(device, queue, arena, native, size, ui_scale, clip, None)
                }
                Target::Page { .. } => {
                    self.fade_stream.inherit_bindings(&self.native);
                    let prepared = self.prepare_native(
                        device,
                        queue,
                        arena,
                        native,
                        size,
                        ui_scale,
                        clip,
                        Some(self.native_format),
                    );
                    self.native.inherit_bindings(&self.fade_stream);
                    prepared
                }
            };
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
                    let run = |index| Run {
                        pipe: Pipe::Native(index),
                        texture: NO_TEXTURE,
                        scissor,
                        range: 0..0,
                    };
                    match target {
                        Target::Frame => {
                            let index = self.native_draws.len();
                            self.native_draws.push(submission);
                            self.runs.push(run(index));
                        }
                        Target::Page { page, .. } => {
                            let index = self.fade_draws.len();
                            self.fade_draws.push(submission);
                            self.page_runs(page).push(run(index));
                        }
                    }
                }
                // The refused draw left the binds the game makes for it, less what the
                // replay lacks ([`prepare_draw`]).
                Err(reason) => {
                    if !self.native_refusals.contains_key(&reason) {
                        eprintln!("{reason}");
                    }
                    *self.native_refusals.entry(reason).or_default() += 1;
                    // By what was refused, whatever refused it (program selection, textures,
                    // bakes or the draw): a map compares these across loads.
                    arena.refuse(&gui_refusal(native));
                }
            }
            return;
        }
        self.native.invalidate_bindings();
        if self.group(quad.texture).is_none() {
            return;
        }
        let [x0, y0, x1, y1] = device_corners(quad.rect, ui_scale);
        // counters: a rotated quad turns about its own centre, with `y`
        // down, so a positive angle is clockwise on screen.
        let spin = (quad.rotation != 0.0).then(|| {
            let (sin, cos) = quad.rotation.sin_cos();
            ([(x0 + x1) * 0.5, (y0 + y1) * 0.5], sin, cos)
        });
        let filter = filter_mode(ui_scale * placement.scale);
        let [out_width, out_height] = placement.out.map(|v| v as f32);
        let corner = |x: f32, y: f32, u: f32, v: f32| {
            let (x, y) = match spin {
                Some((centre, sin, cos)) => {
                    let (dx, dy) = (x - centre[0], y - centre[1]);
                    (
                        centre[0] + dx * cos - dy * sin,
                        centre[1] + dx * sin + dy * cos,
                    )
                }
                None => (x, y),
            };
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
        let corners = [
            corner(x0, y0, u0, v0),
            corner(x1, y0, u1, v0),
            corner(x1, y1, u1, v1),
            corner(x0, y0, u0, v0),
            corner(x1, y1, u1, v1),
            corner(x0, y1, u0, v1),
        ];
        let (vertices, runs): (&mut Vec<Vertex>, Option<usize>) = match target {
            Target::Frame => (frame, None),
            Target::Page { page, .. } => (pages, Some(page)),
        };
        let start = vertices.len() as u32;
        vertices.extend(corners);
        let end = vertices.len() as u32;
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

    /// Fade page `page`'s runs this frame.
    fn page_runs(&mut self, page: usize) -> &mut Vec<Run> {
        if self.fade_runs.len() <= page {
            self.fade_runs.resize_with(page + 1, Vec::new);
        }
        &mut self.fade_runs[page]
    }

    /// Draws the uploaded quads over whatever the target already holds, after drawing the
    /// fading icons into their fade pages.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets) {
        if self.runs.is_empty() {
            return;
        }
        self.draw_pages(encoder);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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

    /// Draws the fading icons of the last upload into their fade pages, which the frame's
    /// composite runs then read.
    fn draw_pages(&self, encoder: &mut wgpu::CommandEncoder) {
        for (index, page) in self.fade.pages.iter().take(self.fade_pages).enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                            pass.set_pipeline(&self.fade_legacy);
                            pass.set_vertex_buffer(0, self.fade_vertices.slice(..));
                            bound = true;
                        }
                        pass.set_bind_group(0, group, &[]);
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
                    pass.set_pipeline(&self.fade.composite);
                    pass.set_vertex_buffer(0, self.composite_vertices.slice(..));
                    bound = Some(Pipe::Composite(0));
                }
                pass.set_bind_group(0, &self.fade.pages[page].group, &[]);
                pass.draw(run.range.clone(), 0..1);
                continue;
            }
            if bound != Some(run.pipe) {
                match run.pipe {
                    Pipe::Legacy => {
                        pass.set_pipeline(&self.pipeline);
                        pass.set_vertex_buffer(0, self.vertices.slice(..));
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
            pass.set_bind_group(0, group, &[]);
            pass.draw(run.range.clone(), 0..1);
        }
    }

    /// Makes the frame a layer composited later, as a GUI window drawn outside the map is:
    /// its draws keep their programs' colour blend and accumulate premultiplied colour, as a
    /// fade page's do. Takes effect at the next upload.
    pub fn set_layer(&mut self, layer: bool) {
        self.layer = layer;
    }

    /// Draws the uploaded layer ([`SpritePass::set_layer`]) into `view`, cleared first, after
    /// the fading icons' pages. Only compiled native draws accumulate as a layer: a snapped
    /// quad is refused.
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
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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

    /// Quads the last upload produced, for reporting.
    pub fn quad_count(&self) -> u32 {
        self.count / 6 + self.native_draws.len() as u32
    }

    /// Fade pages the last upload drew fading icons into, for reporting and checks.
    pub fn fade_pages(&self) -> usize {
        self.fade_pages
    }

    /// Prepares `draw` for the frame of `size`, or for a fade page (`page`, the page format) on
    /// the fade stream, which accumulates premultiplied colour; `clip` places it.
    #[allow(clippy::too_many_arguments)]
    fn prepare_native(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        draw: &super::native_shader::draw::Quad,
        size: [u32; 2],
        scale: f32,
        clip: Option<fade::ClipAffine>,
        page: Option<wgpu::TextureFormat>,
    ) -> Result<super::native_shader::stream::Submission, String> {
        let baked = draw.bake.as_ref().map(|bake| {
            self.prepare_bake(device, queue, arena, bake)
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
                    samples: self.native_samples,
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
                    samples: 1,
                    clip,
                    premultiply: true,
                    layer: false,
                },
            ),
        };
        views.clear();
        self.packet_views = views;
        prepared
    }

    /// Each slot `draw` binds: its texture, `None` for a null bind, `Err` naming a texture that
    /// is not resident.
    fn native_views(
        &self,
        draw: &super::native_shader::draw::Quad,
        storage: &mut Vec<(u32, View)>,
    ) {
        packets::views(storage, draw, |id| self.view(id).cloned());
    }

    fn prepare_bake(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arena: &shader_pipeline::gpu::Arena,
        bake: &Arc<super::native_shader::draw::Bake>,
    ) -> Result<wgpu::TextureView, String> {
        let (result, diagnostics) = installed_content_core::content::diagnostics::capture(|| {
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
        // Changed lazy pages invalidate a bake even if its layout is unchanged. The bake
        // inherits the caller's retained binds, so only a bake that reads nothing it does not
        // bind itself gives the same pixels wherever it is drawn.
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
        let [width, height] = bake.size;
        if width == 0 || height == 0 || width.max(height) > device.limits().max_texture_dimension_2d
        {
            return Err(format!(
                "text bake target {width}x{height} exceeds device limits"
            ));
        }
        let format = wgpu::TextureFormat::Bgra8Unorm;
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
                    samples: 1,
                    clip: None,
                    premultiply: false,
                    layer: false,
                },
            );
            views.clear();
            self.bake_views = views;
            submissions.push(submission?);
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            for submission in &submissions {
                submission.pipeline.draw(&mut pass, &submission.draw);
            }
        }
        // The bake is submitted before the frame: its uploads are written first.
        arena.flush(queue);
        queue.submit([encoder.finish()]);
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

struct CachedBake {
    source: Arc<super::native_shader::draw::Bake>,
    revision: u64,
    view: wgpu::TextureView,
    used: bool,
    diagnostics: installed_content_core::content::diagnostics::Diagnostics,
}

/// Whether `draw`'s compiled program reads only what the draw binds itself: its technique is
/// declared, and every constant buffer, texture and sampler it reflects is one of its own binds
/// (with the effect's own sampler state), none retained from an earlier draw.
fn self_contained(draw: &super::native_shader::draw::Quad) -> bool {
    use super::native_shader::binding;
    use installed_content_core::pdxshader::program::Stage;
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

/// A slot's texture as the replay holds it: `Ok(None)` a null bind, `Err` one it does not hold.
type View = Result<Option<wgpu::TextureView>, String>;

/// What a compiled draw is prepared for: the GUI viewport it projects over, the target's format
/// and samples, a placement after its projection and whether the target keeps premultiplied
/// colour (a fade page).
struct Output {
    viewport: [u32; 2],
    format: wgpu::TextureFormat,
    samples: u32,
    clip: Option<fade::ClipAffine>,
    premultiply: bool,
    /// The frame is a layer composited later as a straight-alpha image
    /// ([`SpritePass::set_layer`]): only draws that flatten into one exactly are submitted.
    layer: bool,
}

/// The colour blend that flattens exactly into a layer composited later: source over.
const SOURCE_OVER: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::SrcAlpha,
    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
    operation: wgpu::BlendOperation::Add,
};

/// Why a draw with `blend` and `mask` cannot be flattened into a layer composited later as one
/// straight-alpha image: the layer keeps premultiplied colour and accumulates coverage, which
/// reproduces a source-over colour blend over anything beneath it. Its alpha write is the
/// layer's own; a draw that leaves colour channels unwritten, or blends colour another way
/// (additive or multiplied over transparent pixels, or no blend), has no such image.
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

/// Applies the draw's program, constants, textures and samplers before preparing submission.
/// A refusal retains available bindings and forgets missing resources, so later inherited
/// draws cannot accidentally use resources from an earlier successful draw. Missing techniques
/// retain the program; a declared technique that fails compilation makes it unavailable.
#[allow(clippy::too_many_arguments)]
fn prepare_draw(
    native: &mut super::native_shader::stream::Stream,
    packets: &mut packets::Cache,
    arena: &shader_pipeline::gpu::Arena,
    draw: &super::native_shader::draw::Quad,
    views: &[(u32, View)],
    baked: Option<Result<wgpu::TextureView, String>>,
    Output {
        viewport,
        format,
        samples,
        clip,
        premultiply,
        layer,
    }: Output,
) -> Result<super::native_shader::stream::Submission, String> {
    use installed_content_core::pdxshader::program::Stage;
    let packets::Metadata { label, selection } = packets.metadata(draw);
    let mut refused = baked
        .as_ref()
        .and_then(|baked| baked.as_ref().err().cloned());
    if refused.is_none() {
        refused = views
            .iter()
            .find_map(|(_, view)| view.as_ref().err().cloned());
    }
    // A declared technique's record carries the effect's samplers, compiled or not.
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
    let mut constants = draw.projected_constants(viewport);
    if let Some(clip) = clip {
        fade::apply(clip, &mut constants);
    }
    // Line charts multiply a row vector; the other GUI effects multiply a column.
    if draw.declaration == shader_pipeline::vertices::Declaration::LineChart {
        for row in 0..4 {
            for column in row + 1..4 {
                constants.swap(row * 4 + column, column * 4 + row);
            }
        }
    }
    let known = [std::ops::Range {
        start: 0,
        end: draw.known,
    }];
    for stage in [Stage::Vertex, Stage::Pixel] {
        if stage == Stage::Pixel && !draw.pixel_constants {
            continue;
        }
        // The whole native allocation, of which the draw writes only `known` bytes: a read of
        // the rest is refused by name where the draw is prepared (`Stream::check_constants`),
        // through retained and null-technique binds alike.
        native.bind_partial_constants(0, stage, Some(bytemuck::cast_slice(&constants)), &known);
    }
    // Animation records' b1: the whole allocation, known only where the game writes it.
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
            Ok(view) => native.bind_texture(slot, Stage::Pixel, view, sampler),
            Err(_) => native.forget_texture(slot, Stage::Pixel, sampler),
        }
    }
    if let Some(reason) = refused {
        return Err(reason);
    }
    native.prepare(
        arena,
        &label,
        &draw.vertices,
        draw.stride,
        draw.vertices.len() / draw.stride,
        draw.topology,
        format,
        samples,
    )
}

/// What [`SpritePass::sync`] does with the lazy textures.
#[derive(Debug, Default, PartialEq, Eq)]
struct SyncPlan {
    /// Uploaded or refused textures that are no longer resident.
    drop: Vec<usize>,
    /// Resident textures to upload: new, changed, or a new revision of one
    /// the device refused. A refused revision is not tried again.
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

/// Uploads `vertices` into `target`, growing it to the next power of two when it is too small.
fn write_vertices<T: bytemuck::Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &mut wgpu::Buffer,
    capacity: &mut u64,
    vertices: &[T],
) {
    if vertices.is_empty() {
        return;
    }
    let count = vertices.len() as u64;
    if count > *capacity {
        *capacity = count.next_power_of_two();
        *target = buffer(device, *capacity, std::mem::size_of::<T>());
    }
    queue.write_buffer(target, 0, bytemuck::cast_slice(vertices));
}

/// The device pixels `[x0, y0, x1, y1]` a quad covers as laid out, on a target of `size` at
/// `ui_scale`, with a pixel to spare on every side for filtering; `None` for a compiled quad
/// without geometry.
fn quad_bounds(quad: &Quad, size: [u32; 2], ui_scale: f32) -> Option<[f32; 4]> {
    let corners: [[f32; 2]; 4] = match &quad.exact {
        Some(exact) => {
            let native = exact.native.as_ref()?;
            // A compiled draw projects over its font viewport or the GUI viewport.
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
        None => {
            let [x0, y0, x1, y1] = device_corners(quad.rect, ui_scale);
            let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
            if quad.rotation == 0.0 {
                corners
            } else {
                let (sin, cos) = quad.rotation.sin_cos();
                let centre = [(x0 + x1) * 0.5, (y0 + y1) * 0.5];
                corners.map(|[x, y]| {
                    let (dx, dy) = (x - centre[0], y - centre[1]);
                    [
                        centre[0] + dx * cos - dy * sin,
                        centre[1] + dx * sin + dy * cos,
                    ]
                })
            }
        }
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

/// The screen-space atlas program.
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
@group(0) @binding(1) var sprite_sampler: sampler;

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

/// Disabled-state tint: negative alpha selects luminance greyscale, its magnitude the opacity.
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

    /// The sprite pass's draws on one device: opening it is most of each check's cost.
    #[test]
    fn sprite_pass_draws() {
        let (_serial, device, queue) = crate::map::test_support::gpu::device();
        a_gui_draw_refused_before_the_stream_is_recorded(device.clone(), queue.clone());
        moving_sprite_upload_matches_original_across_retention_transitions(
            device.clone(),
            queue.clone(),
        );
        a_fading_icon_blends_in_as_one_image_scaled_about_its_anchor(device.clone(), queue.clone());
        an_icon_wider_than_a_default_fade_page_still_fades_whole(device.clone(), queue.clone());
        each_fade_page_draws_this_upload_s_icons_alone(device.clone(), queue.clone());
        fade_pages_follow_each_upload_s_demand(device.clone(), queue.clone());
        every_upload_draws_its_own_fading_icons_alone(device.clone(), queue.clone());
    }

    /// A GUI draw refused before it reaches the shader stream (here a quad without a compiled
    /// effect) is recorded in the frame's arena by what was refused, as every other refusal
    /// is, so a map comparing its refusals with and without visual mods sees it.
    fn a_gui_draw_refused_before_the_stream_is_recorded(device: wgpu::Device, queue: wgpu::Queue) {
        let page = dds::Texture::from_rgba8(installed_content_image::Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![255; 4],
        });
        let images = IconTextures::new(&device, &queue, &[page]).unwrap();
        let mut pass = SpritePass::new(
            &device,
            &queue,
            1,
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
            rotation: 0.0,
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

    fn moving_sprite_upload_matches_original_across_retention_transitions(
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) {
        let mut batch = Batch::default();
        batch.scaled(
            &SpriteSheet {
                name: "synthetic motion".into(),
                texture: 0,
                frames: 1,
                size: [4, 4],
            },
            0,
            [2.0, 3.0],
            [7.0, 8.0],
            [0.75, 0.5, 0.25, 1.0],
        );
        // A refused native quad stays refused on moving, rebuild and retained frames alike.
        let mut refused = batch.quads[0].clone();
        refused.exact = Some(Exact {
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
        });
        batch.quads.push(refused);
        let (pixels, refusals, _) = oracle::render_batch(&device, &queue, &batch);
        assert!(pixels.iter().any(|byte| *byte != 0));
        assert_eq!(refusals.values().sum::<u64>(), 1);
    }

    #[test]
    fn counter_sheet_uses_the_native_texture_decoder_and_client_setting() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
            installed_content_core::gui::native::sprites::read_sprites(
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
    fn frames_slice_the_strip_evenly() {
        // Twenty frames of 29 by 27 pixels share one strip.
        let vp = strip(20, 580);
        assert_eq!(vp.frame_size(), [29.0, 27.0]);
        assert_eq!(vp.frame_uv(0), [0.0, 0.0, 29.0 / 580.0, 1.0]);
        assert_eq!(vp.frame_uv(19), [551.0 / 580.0, 0.0, 1.0, 1.0]);
        // A frame past the end clamps rather than sampling outside the strip.
        assert_eq!(vp.frame_uv(20), vp.frame_uv(19));
    }

    #[test]
    fn a_strip_that_does_not_divide_evenly_keeps_the_authored_pitch() {
        // A fractional frame pitch retains the full strip extent.
        let naval = strip(11, 301);
        assert_eq!(naval.frame_size()[0], 27.0);
        assert_eq!(naval.frame_uv(10), [10.0 / 11.0, 0.0, 1.0, 1.0]);
        // `mapicon_building_airbase_large.dds` is 309 wide with 7 frames.
        assert_eq!(strip(7, 309).frame_size()[0], 44.0);
    }

    #[test]
    fn layer_keys_stay_distinct_and_ordered_past_62500_icons() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // The old `priority * 1_000_000 + icon * 16 + child` key made icon
        // 62,500 of priority 5 equal to priority 6's first key.
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
        let defines = installed_content_core::defines::Defines::new().unwrap();
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
        // base + 1 changed, base + 2 still at its refused revision, base + 3 new,
        // and base was evicted.
        let resident = [(base + 1, 4), (base + 2, 5), (base + 3, 0)];
        assert_eq!(
            plan_sync(&uploaded, &failed, &resident),
            SyncPlan {
                drop: vec![base],
                upload: vec![base + 1, base + 3],
            }
        );
        // A later revision of the refused texture is tried again, and an
        // evicted refused texture is forgotten.
        assert_eq!(
            plan_sync(&uploaded, &failed, &[(base + 2, 6)]).upload,
            [base + 2]
        );
        assert_eq!(plan_sync(&HashMap::new(), &failed, &[]).drop, [base + 2]);
    }

    /// Every device pixel of a quad: its bilinear footprint in texels along x,
    /// with the sample clamped to `window` when one is given.
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
        // A 41-texel flag cell at texels 82..123 of a 512-texel page, with other
        // flags on both sides, drawn 1:1 at 41 interface pixels.
        let texels = 512;
        let uv = [82.0 / 512.0, 0.0, 123.0 / 512.0, 1.0];
        let rect = [100.0, 0.0, 41.0, 26.0];
        let inside = |(low, high): (f32, f32)| low >= 82.0 - 1e-3 && high <= 123.0 + 1e-3;
        // At 1.25 the edge pixels' taps reach a quarter texel into each neighbour.
        let unclamped = footprints(rect, 1.25, uv, texels, None);
        assert!(!inside(unclamped[0]) && !inside(*unclamped.last().unwrap()));
        let window = sample_window(uv, [texels, 1]);
        for scale in [0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
            let pixels = footprints(rect, scale, uv, texels, Some(window));
            assert!(pixels.iter().copied().all(inside), "{scale}");
        }
        // At 1:1 the window moves no sample: every pixel is a texel centre.
        let clamped = footprints(rect, 1.0, uv, texels, Some(window));
        let free = footprints(rect, 1.0, uv, texels, None);
        assert_eq!(clamped.len(), 41);
        for (a, b) in clamped.iter().zip(&free) {
            assert!((a.0 - b.0).abs() < 1e-3, "{a:?} {b:?}");
        }
    }

    #[test]
    fn only_a_scaled_batch_changes_how_it_samples() {
        assert_eq!(filter_mode(1.0), 0.0);
        assert_eq!(filter_mode(1.25), 1.0);
        assert_eq!(filter_mode(0.75), 2.0);
    }

    #[test]
    fn a_window_narrower_than_a_texel_collapses_to_its_centre() {
        let window = sample_window([0.5, 0.25, 0.5, 0.75], [4, 4]);
        assert_eq!(window, [0.5, 0.375, 0.5, 0.625]);
        // Flipped rectangles keep their bounds ordered.
        assert_eq!(sample_window([0.75, 0.0, 0.25, 1.0], [4, 1])[0], 0.375);
    }

    /// A one-colour quad over `rect` on image page `texture`, shown as `present`.
    fn shown(texture: usize, rect: [f32; 4], layer: u64, present: Option<Present>) -> Quad {
        Quad {
            texture,
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
            rotation: 0.0,
            layer,
            shade: 0.0,
            exact: None,
            anchor: None,
            present,
        }
    }

    fn solid(rgba: [u8; 4]) -> dds::Texture {
        dds::Texture::from_rgba8(installed_content_image::Rgba8Image {
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
        // A red plate under a green one covering it, as one icon at half opacity: the green
        // shows at half and the red under it not at all, as a whole-icon fade gives.
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
        // Whole, the same icon draws straight onto the frame, and at half its size about its
        // anchor it covers the middle alone.
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
        // Nearer icons draw over farther ones whatever their layers: rank 1 is nearer.
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
        // A plate as wide as a 2,200-pixel frame, as a mod's layout or a long column may draw,
        // with a red part under a green one, at a quarter opacity.
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

    /// A pass over one red and one green image page.
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
            1,
            &images,
            &LazyTextures::new(),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap()
    }

    /// Icon `rank`'s quad of image page `texture` over `rect`, at half opacity.
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
        // Cells pack within 20 pixels a side: no two of these icons share a page.
        pass.fade.side = 20;
        let arena = shader_pipeline::gpu::Arena::new(&device);
        // One upload and draw on the same pass: its pixels, fade pages and runs on each page.
        let frame = |pass: &mut SpritePass, quads: Vec<Quad>| {
            let batch = Batch {
                quads,
                ..Batch::default()
            };
            arena.reset();
            pass.upload(&device, &queue, &arena, &batch, [16, 16], 1.);
            arena.flush(&queue);
            let mut encoder = device.create_command_encoder(&Default::default());
            pass.draw_pages(&mut encoder);
            queue.submit([encoder.finish()]);
            let pixels = crate::map::test_support::gpu::render_with(&device, &queue, |render| {
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
        // A far red icon at the top left and a near green one at the bottom right over part of
        // it, each on its own page.
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
        // The next upload's two icons on the same two pages are two corners each, with holes
        // between them: nothing of the last upload's icons, nor of each other, shows there. The
        // red icon's second corner lands on its page where the green icon's hole is on the next.
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
        // An upload with no fading icon draws none and keeps no page.
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
        // Then the corners again, on fresh pages.
        let (pixels, used, runs, kept) = frame(&mut pass, corners());
        assert_eq!((used, runs, kept), (2, vec![1, 1], 2));
        check(&pixels, &changed);
    }

    fn fade_pages_follow_each_upload_s_demand(device: wgpu::Device, queue: wgpu::Queue) {
        let mut pass = red_and_green(&device, &queue);
        let arena = shader_pipeline::gpu::Arena::new(&device);
        let sizes = |pass: &mut SpritePass, quads: Vec<Quad>, size: [u32; 2]| {
            let batch = Batch {
                quads,
                ..Batch::default()
            };
            arena.reset();
            pass.upload(&device, &queue, &arena, &batch, size, 1.);
            pass.fade
                .pages
                .iter()
                .map(|page| page.size)
                .collect::<Vec<_>>()
        };
        // Demand alternating by a pixel between the two sides (bounds of 256 by 257 and 257 by 256,
        // a pixel to spare around each quad) grows the short side once, keeping the other, and then
        // keeps the page: no upload makes a new one.
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
        // A wide frame's wide icon, a tall frame's tall icon, then a small frame's small icon: each
        // page is as large as this upload's icons need, never the larger of earlier uploads'.
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
        // A little more than a step keeps a page within twice what it needs; a page far larger
        // than needed is replaced.
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
        // Pages the upload does not fill go: two icons on two pages, then one, then none, then an
        // empty batch.
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
            1,
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
        // Two icons, each one run on the one page and one composite on the frame, every frame.
        assert_eq!(seen, vec![(1, 2, 0, 2); 3]);
        // A frame without fading icons keeps none of an earlier frame's.
        batch.quads = vec![shown(0, [2.0, 2.0, 4.0, 4.0], layer(10, 0, 0), None)];
        arena.reset();
        pass.upload(&device, &queue, &arena, &batch, [16, 16], 1.);
        assert_eq!(counts(&pass), (0, 0, 0, 1));
    }

    #[test]
    fn at_scale_one_the_corners_are_the_plain_rounding() {
        assert_eq!(
            device_corners([10.4, 20.6, 29.0, 27.0], 1.0),
            [10.0, 21.0, 39.0, 48.0]
        );
    }

    #[test]
    fn scaled_quads_that_touch_in_interface_pixels_still_touch_on_screen() {
        // Two 10-pixel cells side by side at 1.25: rounding sizes would give
        // 13 + 13 from 13 and 25, overlapping by a pixel.
        let left = device_corners([10.0, 0.0, 10.0, 10.0], 1.25);
        let right = device_corners([20.0, 0.0, 10.0, 10.0], 1.25);
        assert_eq!(left, [13.0, 0.0, 25.0, 13.0]);
        assert_eq!(right[0], left[2]);
        assert_eq!(right, [25.0, 0.0, 38.0, 13.0]);
        // The owner's display: 2048 interface pixels fill 2560 device pixels.
        assert_eq!(
            device_corners([0.0, 0.0, 2048.0, 1152.0], 1.25),
            [0.0, 0.0, 2560.0, 1440.0]
        );
    }
}

#[cfg(test)]
mod layer_tests {
    use super::*;

    /// Only a source-over colour blend writing every colour channel flattens into a layer; its
    /// alpha write is the layer's own.
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
