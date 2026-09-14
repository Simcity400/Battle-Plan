//! The screen-space sprite batch: textured quads in **device pixels**.
//!
//! R2 section 5.4 measures the 29 x 27 victory-point frame as 28 x 27 device
//! pixels in the 2560 x 1440 screenshot, which is a 1.25x upscale of a
//! 2048 x 1152 frame (R1 section 0): the map-icon layer draws 1:1 with no GUI
//! scaling. Section 6.7 confirms it independently on the 27 px counter plate.
//! So this pass takes rectangles in device pixels, snaps them to the pixel grid
//! and converts to clip space on the CPU — there is no projection matrix and no
//! scale factor anywhere in it.
//!
//! Every glyph of [`super::text`] goes through the same batch, so a label's
//! sprite and its text are one draw when they share a page. Quads are grouped
//! by texture before upload, which changes the order of *unrelated* quads only;
//! the map-icon priorities of R2 5.5 are honoured by the order the caller
//! pushes them in, and everything this slice draws is priority 5.
//!
//! Depth is off and the batch runs after every world pass, so it always wins
//! against the terrain, the water and the country names.

use std::io;

use eframe::wgpu;
use installed_content::{ResolvedLayers, gfx, image::dds};

use super::{
    // counters
    counters::CounterAssets,
    scene::{SAMPLE_COUNT, Targets, sampler_entry, texture_entry},
    text,
    textures::{Bound, upload},
};

/// `interface/mapicons.gfx:290-295`: the VP and capital strip, 20 frames.
pub const VICTORY_POINT_STRIP: &str = "GFX_onmap_victorypoints_strip";

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
    /// One frame's size in device pixels.
    ///
    /// R2 section 5.7: two shipped strips do not divide evenly, and the engine
    /// slices at `floor(w / n)` — `round(i * w / n)` shifts the naval-base
    /// frames by up to 4 px. The 1 to 4 px of slack at the right edge is left
    /// unused, exactly as the engine leaves it.
    pub fn frame_size(&self) -> [f32; 2] {
        [
            (self.size[0] / self.frames.max(1)) as f32,
            self.size[1] as f32,
        ]
    }

    /// The atlas rectangle of a zero-based frame, clamped to the last frame.
    pub fn frame_uv(&self, frame: u32) -> [f32; 4] {
        let frames = self.frames.max(1);
        let index = frame.min(frames - 1);
        let width = self.size[0] / frames;
        let left = (index * width) as f32 / self.size[0].max(1) as f32;
        let right = ((index + 1) * width) as f32 / self.size[0].max(1) as f32;
        [left, 0.0, right, 1.0]
    }
}

/// Every texture and font the icon layer draws from.
pub struct MapIcons {
    /// Decoded pages and strips; the batch refers to these by index.
    pub images: Vec<dds::Texture>,
    pub fonts: text::Fonts,
    pub victory_points: SpriteSheet,
    // counters: the unit-counter, base and combat sheets, and the flag atlas.
    pub counters: CounterAssets,
}

impl MapIcons {
    /// Reads the fonts and the map-icon sprites from the resolved layers.
    ///
    /// `interface` and `gfx/interface` are covered content and go through
    /// [`ResolvedLayers::read`]; `gfx/fonts` is not, and [`text::Fonts::load`]
    /// states that limitation (`RENDERING.md` section 4).
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut images = Vec::new();
        let fonts = text::Fonts::load(layers, &mut images)?;
        let sprites = gfx::sprites(layers)?;
        let victory_points = load_sheet(layers, &sprites, VICTORY_POINT_STRIP, &mut images)?;
        // counters
        let counters = CounterAssets::load(layers, &sprites, &mut images)?;
        Ok(Self {
            images,
            fonts,
            victory_points,
            counters,
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
    let sprite = sprites
        .get(name)
        .ok_or_else(|| invalid(format!("interface/*.gfx declares no sprite {name}")))?;
    let path = sprite
        .texture
        .as_deref()
        .ok_or_else(|| invalid(format!("sprite {name} names no texturefile")))?;
    let texture =
        dds::decode(&layers.read(path)?).map_err(|error| invalid(format!("{path}: {error}")))?;
    if sprite.frames == 0 || texture.width < sprite.frames {
        return Err(invalid(format!(
            "sprite {name} declares {} frames for a {} texel wide strip",
            sprite.frames, texture.width
        )));
    }
    let size = [texture.width, texture.height];
    let id = images.len();
    images.push(texture);
    Ok(SpriteSheet {
        name: name.to_owned(),
        texture: id,
        frames: sprite.frames,
        size,
    })
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
    pub layer: u32,
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
/// another counter's plate. `icon` wraps at 16 children per window, which is
/// more than any window this pass draws.
pub const fn layer(priority: u32, icon: u32, child: u32) -> u32 {
    priority * 1_000_000 + icon * 16 + child
}

/// A frame's worth of screen-space quads, in the order the caller pushed them.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub quads: Vec<Quad>,
    // counters
    /// The layer subsequent pushes are tagged with.
    layer: u32,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    // counters
    /// Tags every later push with this sort layer.
    pub fn at_layer(&mut self, layer: u32) {
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
    /// Adds the leftmost `fill` of one frame, clipping the rectangle and the uv.
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
        fill: f32,
        color: [f32; 4],
    ) {
        let fill = fill.clamp(0.0, 1.0);
        let width = (size[0] * fill).floor();
        if width < 1.0 {
            return;
        }
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

/// `position`, `uv` and `color`, already in clip space.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
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
            bound.push(upload(device, queue, &format!("map icon {index}"), image)?);
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
            // The batch draws at 1:1 on integer pixel corners, so the top mip
            // is the only one a sample can land in; saying so outright keeps a
            // driver's derivative estimate out of the result.
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

/// The screen-space pipeline and the frame's vertices.
pub struct SpritePass {
    pipeline: wgpu::RenderPipeline,
    bind_groups: Vec<wgpu::BindGroup>,
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
        textures: &IconTextures,
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
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x4
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
                count: SAMPLE_COUNT,
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
            vertices: buffer(device, INITIAL_VERTICES),
            capacity: INITIAL_VERTICES,
            runs: Vec::new(),
            count: 0,
        })
    }

    /// Turns one frame's quads into vertices, grouped by texture.
    ///
    /// Rectangles are rounded to whole device pixels first: the layout adds up
    /// `.gui` integers on top of a projected anchor, so the fractional part is
    /// the anchor's alone, and keeping it would resample a 1:1 sprite.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batch: &Batch,
        size: [u32; 2],
    ) {
        self.runs.clear();
        self.count = 0;
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
        for index in order {
            let quad = batch.quads[index];
            if quad.texture >= self.bind_groups.len() {
                continue;
            }
            let x0 = quad.rect[0].round();
            let y0 = quad.rect[1].round();
            let x1 = x0 + quad.rect[2].round();
            let y1 = y0 + quad.rect[3].round();
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
            pass.set_bind_group(0, &self.bind_groups[run.texture], &[]);
            pass.draw(run.range.clone(), 0..1);
        }
    }

    /// Quads the last upload produced, for reporting.
    pub fn quad_count(&self) -> u32 {
        self.count / 6
    }
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
const SHADER: &str = r#"
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@group(0) @binding(0) var page: texture_2d<f32>;
@group(0) @binding(1) var page_sampler: sampler;

@vertex
fn vertex_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}

// The atlases are authored straight, not premultiplied (R2 5.3), so the colour
// multiplies both parts and the blend state does the compositing.
@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sample = textureSample(page, page_sampler, in.uv);
    return sample * in.color;
}
"#;

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
    fn frames_slice_the_strip_at_floor_of_the_width() {
        // `GFX_onmap_victorypoints_strip`: 580 x 27, 20 frames of 29 x 27.
        let vp = strip(20, 580);
        assert_eq!(vp.frame_size(), [29.0, 27.0]);
        assert_eq!(vp.frame_uv(0), [0.0, 0.0, 29.0 / 580.0, 1.0]);
        assert_eq!(vp.frame_uv(19), [551.0 / 580.0, 0.0, 1.0, 1.0]);
        // A frame past the end clamps rather than sampling outside the strip.
        assert_eq!(vp.frame_uv(20), vp.frame_uv(19));
    }

    #[test]
    fn a_strip_that_does_not_divide_evenly_leaves_the_slack_at_the_right() {
        // R2 5.7: `mapicon_building_navalbase.dds` is 301 wide with 11 frames.
        let naval = strip(11, 301);
        assert_eq!(naval.frame_size()[0], 27.0);
        assert_eq!(naval.frame_uv(10), [270.0 / 301.0, 0.0, 297.0 / 301.0, 1.0]);
        // `mapicon_building_airbase_large.dds` is 309 wide with 7 frames.
        assert_eq!(strip(7, 309).frame_size()[0], 44.0);
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
}
