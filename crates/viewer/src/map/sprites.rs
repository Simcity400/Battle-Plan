//! The screen-space sprite batch: textured quads in **device pixels**.
//!
//! R2 section 5.4 measures the 29 x 27 victory-point frame as 28 x 27 device
//! pixels in the 2560 x 1440 screenshot, which is a 1.25x upscale of a
//! 2048 x 1152 frame (R1 section 0): the map-icon layer draws 1:1 with no GUI
//! scaling. Section 6.7 confirms it independently on the 27 px counter plate.
//! So this pass takes rectangles in device pixels, snaps them to the pixel grid
//! and converts to clip space on the CPU — there is no projection matrix.
//!
//! The one scale factor is the viewer's own interface scale
//! ([`super::ui_scale`]): callers lay the batch out in interface pixels and
//! [`device_corners`] multiplies each snapped rectangle into device pixels at
//! upload. At 1.0, which a dump uses, interface and device pixels are the same.
//!
//! Every glyph of [`super::text`] goes through the same batch, so a label's
//! sprite and its text are one draw when they share a page. Quads are grouped
//! by texture before upload, which changes the order of *unrelated* quads only;
//! the map-icon priorities of R2 5.5 are honoured by the order the caller
//! pushes them in, and everything this slice draws is priority 5.
//!
//! Depth is off and the batch runs after every world pass, so it always wins
//! against the terrain, the water and the country names.

use std::{collections::HashMap, io, sync::Arc};

use eframe::wgpu;
use installed_content::{ResolvedLayers, gfx, image::dds};

use super::{
    // counters
    counters::CounterAssets,
    lazy_icons::{DYNAMIC_BASE, LazyTextures},
    scene::{Targets, sampler_entry, texture_entry},
    text,
    textures::{Bound, ColorSpace, upload},
};

/// One frame-animated sprite: a horizontal strip of equal-width frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteSheet {
    /// The `GFX_...` name.
    pub name: String,
    /// Texture id in [`MapIcons::images`].
    pub texture: usize,
    /// `noOfFrames`.
    pub frames: u32,
    /// Whole-strip size in texels.
    pub size: [u32; 2],
}

impl SpriteSheet {
    /// One frame's size in device pixels: `floor(w / n)` texels wide.
    /// **UNVERIFIED** for strips that do not divide evenly: the game may draw
    /// the frame `w / n` wide, a third of a pixel more on the naval-base strip.
    pub fn frame_size(&self) -> [f32; 2] {
        [
            (self.size[0] / self.frames.max(1)) as f32,
            self.size[1] as f32,
        ]
    }

    /// The atlas rectangle of a zero-based frame, clamped to the last frame.
    ///
    /// Frames sit at `i / n` of the strip. The naval-base strip is 301 texels
    /// for 11 frames and its glyphs are authored on that 27.36-texel pitch;
    /// slicing at `i * floor(w / n)` drew frame `i` `0.36 * i` px right of the
    /// game (measured on frames 0-7 in the 2026-09-24 L1a pairs) and cut the
    /// gold badge's right edge.
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

/// Every texture and font the icon layer draws from.
pub struct MapIcons {
    pub combat: crate::combat::Assets,
    /// Decoded pages and strips; the batch refers to these by index.
    pub images: Vec<dds::Texture>,
    pub fonts: text::Fonts,
    pub victory_points: SpriteSheet,
    // counters: the unit-counter, base and combat sheets, and the flag atlas.
    pub counters: CounterAssets,
    /// The raid map icon's layout, sprites and raid content.
    pub raids: super::raids::RaidAssets,
    /// Flags and portraits decoded on first draw, above [`DYNAMIC_BASE`].
    pub lazy: Arc<LazyTextures>,
}

impl MapIcons {
    /// Reads the fonts and the map-icon sprites from the resolved layers.
    ///
    /// `interface` and `gfx/interface` are covered content and go through
    /// [`ResolvedLayers::read`]; `gfx/fonts` is not, and [`text::Fonts::load`]
    /// states that limitation ([archived rendering notes](#) section 4).
    pub fn load(
        layers: &ResolvedLayers,
        defines: &installed_content::defines::Defines,
        map: &installed_content::map::MapData,
        language: &str,
    ) -> io::Result<Self> {
        let mut images = Vec::new();
        // The label sprite and font are the ones `mapicons.gui` names.
        let labels = super::labels::LabelLayout::read(layers)?;
        let layouts = super::counters::Layouts::read(layers)?;
        let mut font_warnings = Vec::new();
        // The battle-plan label font is named by `maparrows.txt`'s `arrowText`.
        let arrow_font = super::orders::arrow_text_font(layers)?;
        let fonts = text::Fonts::load(
            layers,
            language,
            &text::DeclaredFonts {
                label: &labels.font,
                counter: layouts.land.count.font.as_deref(),
                ping: layouts
                    .faction_ping
                    .as_ref()
                    .ok()
                    .and_then(|ping| ping.count.font.as_deref()),
                arrow: arrow_font.as_deref(),
            },
            &mut images,
            &mut font_warnings,
        )?;
        let sprites = gfx::sprites(layers)?;
        let victory_points = load_sheet(layers, &sprites, &labels.sprite, &mut images)?;
        // Flag and portrait reads happen after this load returns, so the lazy
        // stores keep their own handle on the layers.
        let shared = Arc::new(layers.clone());
        let lazy = LazyTextures::new();
        // counters
        let mut counters =
            CounterAssets::load(&shared, layouts, defines, &sprites, &mut images, &lazy)?;
        counters.warnings.extend(font_warnings);
        let combat = crate::combat::Assets::load(
            &shared,
            defines,
            &sprites,
            &counters.flags,
            language,
            &mut images,
            &lazy,
        )?;
        let raids = super::raids::RaidAssets::load(layers, defines, map, &mut images)?;
        if images.len() >= DYNAMIC_BASE {
            return Err(invalid("more static icon pages than texture ids"));
        }
        Ok(Self {
            combat,
            images,
            fonts,
            victory_points,
            counters,
            raids,
            lazy,
        })
    }
}

/// Loads one declared sprite and appends its texture to `images`.
pub fn load_sheet(
    layers: &ResolvedLayers,
    sprites: &std::collections::BTreeMap<String, gfx::SpriteType>,
    name: &str,
    images: &mut Vec<dds::Texture>,
) -> io::Result<SpriteSheet> {
    let (texture, frames, size) = decode_sheet(layers, sprites, name)?;
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
    sprites: &std::collections::BTreeMap<String, gfx::SpriteType>,
    name: &str,
) -> io::Result<(dds::Texture, u32, [u32; 2])> {
    let sprite = sprites
        .get(name)
        .ok_or_else(|| invalid(format!("interface/*.gfx declares no sprite {name}")))?;
    let path = sprite
        .texture
        .as_deref()
        .ok_or_else(|| invalid(format!("sprite {name} names no texturefile")))?;
    let bytes = layers.read(path)?;
    let texture = dds::decode(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    /// Texture id in [`MapIcons::images`].
    pub texture: usize,
    /// `x, y, width, height` in device pixels, `y` down from the top.
    pub rect: [f32; 4],
    /// Normalised atlas rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Straight RGBA multiplier; `[1, 1, 1, 1]` draws the texture as authored.
    pub color: [f32; 4],
    // counters
    /// Clockwise rotation about the rectangle's centre, in radians.
    ///
    /// `mapicons.gui` rotates two children of the map icons this pass draws:
    /// the counter bars by `1.5708` and `battle_indicator_direction` by the
    /// combat's own heading. Everything else pushes zero and keeps the
    /// pixel-snapped axis-aligned path.
    pub rotation: f32,
    // counters
    /// The sort key [`layer`] builds; see it for the field order.
    pub layer: u64,
}

// counters
/// A quad's sort key: the engine's icon priority, then the icon's own place in
/// the pass, then the child's place inside its window.
///
/// Quads are uploaded sorted by `(layer, texture)`. The engine sorts map icons
/// by `DEFAULT_PRIORITY_*` (R2 section 5.5) and draws each icon's children in
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

/// A key in the band above every map-icon priority, where the combat window
/// draws over the map icons as the GUI does.
pub const fn gui_layer(icon: u32, child: u32) -> u64 {
    band_layer(GUI_BAND, icon, child)
}

const fn band_layer(band: u64, icon: u32, child: u32) -> u64 {
    band * PRIORITY_STRIDE + icon as u64 * CHILD_SLOTS as u64 + child as u64
}

/// `NMapIcons.<name>` as an icon's sort priority: `SetMapIconPrioritiesForMapMode`
/// (`140e02410`) stores the integer define's low byte, and every icon reads it back
/// sign-extended into `CMapIcon + 0x70` (`MOVSX`, e.g. `1418bbed9`), so 300 sorts as 44.
pub fn priority(defines: &installed_content::defines::Defines, name: &str) -> io::Result<i8> {
    Ok(defines.int32("NMapIcons", name)? as i8)
}

/// Child slots per icon in a [`layer`] key.
pub const CHILD_SLOTS: u32 = 32;

/// Keys one priority spans: room for every `u32` icon of [`CHILD_SLOTS`] children.
const PRIORITY_STRIDE: u64 = 1 << 40;
const _: () = assert!((u32::MAX as u64 + 1) * CHILD_SLOTS as u64 <= PRIORITY_STRIDE);
/// The band [`gui_layer`] keys, above the 256 map-icon priorities.
const GUI_BAND: u64 = 256;

/// The priority a [`layer`] key was built with.
#[cfg(test)]
pub const fn layer_priority(layer: u64) -> i64 {
    (layer / PRIORITY_STRIDE) as i64 - 128
}

/// The icon ordinal of a [`layer`] key, within its priority.
#[cfg(test)]
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
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// Empties the batch as [`Batch::default`] does, keeping its storage.
    pub fn clear(&mut self) {
        self.quads.clear();
        self.layer = 0;
    }

    // counters
    /// Tags every later push with this sort layer.
    pub fn at_layer(&mut self, layer: u64) {
        self.layer = layer;
    }

    /// Adds one sprite frame with its top-left corner at `at`.
    pub fn sprite(&mut self, sheet: &SpriteSheet, frame: u32, at: [f32; 2], color: [f32; 4]) {
        let size = sheet.frame_size();
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv: sheet.frame_uv(frame),
            color,
            rotation: 0.0,
            layer: self.layer,
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
        });
    }

    // counters
    /// Adds one sprite frame rotated clockwise about its own centre.
    pub fn rotated(
        &mut self,
        sheet: &SpriteSheet,
        frame: u32,
        at: [f32; 2],
        size: [f32; 2],
        rotation: f32,
        color: [f32; 4],
    ) {
        self.quads.push(Quad {
            texture: sheet.texture,
            rect: [at[0], at[1], size[0], size[1]],
            uv: sheet.frame_uv(frame),
            color,
            rotation,
            layer: self.layer,
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
        // (`super::counter_inputs::bar_pixels`); the texture is one flat colour, so the quad
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

/// The texel-centre window a quad's atlas rectangle is sampled inside, as the
/// shader's `window` computes it: the rectangle inset by half a texel on every
/// side, so no bilinear tap reaches a neighbouring frame, glyph or flag cell.
/// A rectangle narrower than a texel collapses to its centre.
///
/// A 1:1 or minified quad on whole device pixels already samples inside it,
/// so this changes nothing there. A magnified one, which a UI scale above 1.0
/// draws, samples its edge pixels past the last texel centre; clamping them to
/// the window repeats the cell's edge texels, which is what an edge-extended
/// gutter around the cell would hold. The shader's `window` is the one that
/// runs; this is its specification for the tests.
#[cfg(test)]
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
    /// `mapname.shader`'s `DiffuseTexture`: linear mag, min and mip, clamped.
    trilinear: wgpu::Sampler,
}

impl IconTextures {
    /// Uploads every page of `icons`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, icons: &MapIcons) -> io::Result<Self> {
        let mut bound = Vec::with_capacity(icons.images.len());
        for (index, image) in icons.images.iter().enumerate() {
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
            trilinear: clamp(
                "icon trilinear sampler",
                wgpu::MipmapFilterMode::Linear,
                32.0,
            ),
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

    pub fn trilinear_clamp(&self) -> &wgpu::Sampler {
        &self.trilinear
    }

    /// Pages uploaded, for reporting.
    pub fn page_count(&self) -> usize {
        self.bound.len()
    }
}

/// One run of consecutive vertices sharing a texture.
struct Run {
    texture: usize,
    range: std::ops::Range<u32>,
}

/// A lazy texture on the GPU, at the revision it was uploaded from.
struct DynamicPage {
    revision: u64,
    _texture: Bound,
    group: wgpu::BindGroup,
}

/// The screen-space pipeline and the frame's vertices.
pub struct SpritePass {
    pipeline: wgpu::RenderPipeline,
    bind_groups: Vec<wgpu::BindGroup>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
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
}

/// Vertices a freshly built pass can hold before it grows; six per quad.
const INITIAL_VERTICES: u64 = 6 * 4096;

impl SpritePass {
    /// Builds the pipeline over already-uploaded icon pages.
    pub fn new(
        device: &wgpu::Device,
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
                        4 => Float32
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            // The icon layer is drawn last and never tests depth (R2 5.5: the
            // priorities are a sort order inside the layer, not a depth test).
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            bind_groups,
            layout,
            sampler: textures.screen_clamp().clone(),
            lazy: Arc::clone(lazy),
            synced: None,
            dynamic: HashMap::new(),
            failed: HashMap::new(),
            vertices: buffer(device, INITIAL_VERTICES),
            capacity: INITIAL_VERTICES,
            runs: Vec::new(),
            count: 0,
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
                    _texture: texture,
                    group,
                },
            );
        }
        self.synced = Some(version);
    }

    /// The bind group of a static page or a resident lazy texture.
    fn group(&self, texture: usize) -> Option<&wgpu::BindGroup> {
        self.bind_groups
            .get(texture)
            .or_else(|| self.dynamic.get(&texture).map(|page| &page.group))
    }

    /// Turns one frame's quads into vertices, grouped by texture.
    ///
    /// `batch` is in interface pixels and `ui_scale` device pixels per
    /// interface pixel; see [`device_corners`] for the snapping and
    /// [`filter_mode`] for the sampling.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batch: &Batch,
        size: [u32; 2],
        ui_scale: f32,
    ) {
        self.runs.clear();
        self.count = 0;
        // The lazy textures this frame draws stay pinned in their LRU.
        self.lazy.mark_drawn(
            batch
                .quads
                .iter()
                .map(|quad| quad.texture)
                .filter(|texture| *texture >= DYNAMIC_BASE)
                .collect::<std::collections::BTreeSet<_>>(),
        );
        if batch.is_empty() || size[0] == 0 || size[1] == 0 {
            return;
        }
        let mut order: Vec<usize> = (0..batch.quads.len()).collect();
        // counters: `(layer, texture)`, so a page batch never overtakes an
        // icon child that has to draw under or over another.
        order.sort_by_key(|&index| (batch.quads[index].layer, batch.quads[index].texture));
        let width = size[0] as f32;
        let height = size[1] as f32;
        let mut vertices: Vec<Vertex> = Vec::with_capacity(order.len() * 6);
        let filter = filter_mode(ui_scale);
        for index in order {
            let quad = batch.quads[index];
            if self.group(quad.texture).is_none() {
                continue;
            }
            let [x0, y0, x1, y1] = device_corners(quad.rect, ui_scale);
            let clip = |x: f32, y: f32| [x / width * 2.0 - 1.0, 1.0 - y / height * 2.0];
            // counters: a rotated quad turns about its own centre, with `y`
            // down, so a positive angle is clockwise on screen.
            let spin = (quad.rotation != 0.0).then(|| {
                let (sin, cos) = quad.rotation.sin_cos();
                ([(x0 + x1) * 0.5, (y0 + y1) * 0.5], sin, cos)
            });
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
                Vertex {
                    position: clip(x, y),
                    uv: [u, v],
                    color: quad.color,
                    cell: quad.uv,
                    filter,
                }
            };
            let [u0, v0, u1, v1] = quad.uv;
            let start = vertices.len() as u32;
            vertices.extend([
                corner(x0, y0, u0, v0),
                corner(x1, y0, u1, v0),
                corner(x1, y1, u1, v1),
                corner(x0, y0, u0, v0),
                corner(x1, y1, u1, v1),
                corner(x0, y1, u0, v1),
            ]);
            let end = vertices.len() as u32;
            match self.runs.last_mut() {
                Some(run) if run.texture == quad.texture => run.range.end = end,
                _ => self.runs.push(Run {
                    texture: quad.texture,
                    range: start..end,
                }),
            }
        }
        self.count = vertices.len() as u32;
        if self.count == 0 {
            return;
        }
        if u64::from(self.count) > self.capacity {
            self.capacity = u64::from(self.count).next_power_of_two();
            self.vertices = buffer(device, self.capacity);
        }
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
    }

    /// Draws the uploaded quads over whatever the target already holds.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, targets: &Targets) {
        if self.count == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprites"),
            color_attachments: &[Some(targets.output_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for run in &self.runs {
            let Some(group) = self.group(run.texture) else {
                continue;
            };
            pass.set_bind_group(0, group, &[]);
            pass.draw(run.range.clone(), 0..1);
        }
    }

    /// Quads the last upload produced, for reporting.
    pub fn quad_count(&self) -> u32 {
        self.count / 6
    }
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
    let live: std::collections::HashSet<usize> = resident.iter().map(|(id, _)| *id).collect();
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

fn buffer(device: &wgpu::Device, vertices: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite vertices"),
        size: vertices * std::mem::size_of::<Vertex>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The whole pass: positions arrive in clip space, so there is no transform.
pub(super) const SHADER: &str = r#"
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) cell: vec4<f32>,
    @location(3) @interpolate(flat) sampling: f32,
}

@group(0) @binding(0) var page: texture_2d<f32>;
@group(0) @binding(1) var page_sampler: sampler;

@vertex
fn vertex_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) cell: vec4<f32>,
    @location(4) sampling: f32,
) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    out.cell = cell;
    out.sampling = sampling;
    return out;
}

// `sample_window` in sprites.rs: the cell inset by half a texel.
fn window(cell: vec4<f32>, texels: vec2<f32>) -> vec4<f32> {
    let half = 0.5 / texels;
    let low = min(cell.xy, cell.zw);
    let high = max(cell.xy, cell.zw);
    let middle = (low + high) * 0.5;
    return vec4<f32>(min(low + half, middle), max(high - half, middle));
}

// `filter_mode` in sprites.rs: 0 one plain tap; 1 one tap clamped to the
// cell's window; 2 the texels a minified pixel covers, up to 4 x 4, each
// clamped. The derivatives are taken before any branch.
fn texel(uv: vec2<f32>, cell: vec4<f32>, sampling: f32) -> vec4<f32> {
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    if (sampling < 0.5) {
        return textureSampleLevel(page, page_sampler, uv, 0.0);
    }
    let texels = vec2<f32>(textureDimensions(page, 0));
    let bounds = window(cell, texels);
    var taps = 1u;
    if (sampling > 1.5) {
        let footprint = max(abs(dx), abs(dy)) * texels;
        taps = u32(clamp(ceil(max(footprint.x, footprint.y) - 0.001), 1.0, 4.0));
    }
    var sum = vec4<f32>(0.0);
    for (var j = 0u; j < taps; j = j + 1u) {
        for (var i = 0u; i < taps; i = i + 1u) {
            let offset = (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(taps) - 0.5;
            let at = clamp(uv + dx * offset.x + dy * offset.y, bounds.xy, bounds.zw);
            sum = sum + textureSampleLevel(page, page_sampler, at, 0.0);
        }
    }
    return sum / f32(taps * taps);
}

// The atlases are authored straight, not premultiplied (R2 5.3), so the colour
// multiplies both parts and the blend state does the compositing. A negative
// alpha selects `buttonstate.shader`'s `PixelShaderDisable`: the texel's
// luminance grey, then the colour with its alpha negated back.
@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sample = texel(in.uv, in.cell, in.sampling);
    if (in.color.a < 0.0) {
        let grey = dot(sample.rgb, vec3<f32>(0.212671, 0.715160, 0.072169));
        return vec4<f32>(grey, grey, grey, sample.a) * vec4<f32>(in.color.rgb, -in.color.a);
    }
    return sample * in.color;
}
"#;

/// The quad colour that draws a sprite in the GUI's disabled state (`buttonstate.shader`'s
/// `PixelShaderDisable`, luminance weights `0.212671, 0.715160, 0.072169`); the negative alpha is
/// the flag and its magnitude the multiplier.
pub const DISABLED: [f32; 4] = [1.0, 1.0, 1.0, -1.0];

#[cfg(test)]
mod tests {
    use super::*;

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
        // `GFX_onmap_victorypoints_strip`: 580 x 27, 20 frames of 29 x 27.
        let vp = strip(20, 580);
        assert_eq!(vp.frame_size(), [29.0, 27.0]);
        assert_eq!(vp.frame_uv(0), [0.0, 0.0, 29.0 / 580.0, 1.0]);
        assert_eq!(vp.frame_uv(19), [551.0 / 580.0, 0.0, 1.0, 1.0]);
        // A frame past the end clamps rather than sampling outside the strip.
        assert_eq!(vp.frame_uv(20), vp.frame_uv(19));
    }

    #[test]
    fn a_strip_that_does_not_divide_evenly_keeps_the_authored_pitch() {
        // `mapicon_building_navalbase.dds` is 301 wide with 11 frames.
        let naval = strip(11, 301);
        assert_eq!(naval.frame_size()[0], 27.0);
        assert_eq!(naval.frame_uv(10), [10.0 / 11.0, 0.0, 1.0, 1.0]);
        // `mapicon_building_airbase_large.dds` is 309 wide with 7 frames.
        assert_eq!(strip(7, 309).frame_size()[0], 44.0);
    }

    #[test]
    fn layer_keys_stay_distinct_and_ordered_past_62500_icons() {
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
        let defines = installed_content::defines::Defines::new().unwrap();
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
        assert_eq!(layer_priority(layer(-56, 3, 0)), -56);
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

    #[test]
    fn a_batch_keeps_the_push_order_and_names_its_texture() {
        let mut batch = Batch::default();
        assert!(batch.is_empty());
        batch.sprite(&strip(20, 580), 2, [10.0, 20.0], [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(batch.quads[0].rect, [10.0, 20.0, 29.0, 27.0]);
        assert_eq!(batch.quads[0].texture, 0);
        batch.text(
            &[text::TextGlyph {
                page: 3,
                rect: [1.0, 2.0, 3.0, 4.0],
                uv: [0.0, 0.0, 1.0, 1.0],
            }],
            [0.5, 0.5, 0.5, 1.0],
        );
        assert_eq!(batch.quads[1].texture, 3);
        assert_eq!(batch.quads[1].color, [0.5, 0.5, 0.5, 1.0]);
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
