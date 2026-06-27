//! `render` — wgpu OFFSCREEN render primitives for M1.
//!
//! Provides an offscreen RGBA8 render target and three draw routines:
//!   * [`Renderer::draw_sprite`] — a framed sprite from a real `.dds`.
//!   * [`Renderer::draw_nine_slice`] — a `corneredTileSpriteType` 9-slice frame
//!     with correct corner/edge/centre behaviour.
//!   * [`Renderer::draw_text`] — a BMFont string as textured glyph quads,
//!     honouring xoffset/yoffset/xadvance + kerning + multiple pages.
//!
//! Correctness choices (Codex-hardened, see §3.1):
//!   * NEAREST min/mag sampling, no mips — crisp UI pixels.
//!   * Pixel snapping (vertices placed on integer pixel boundaries).
//!   * Premultiplied-vs-straight alpha is normalised at TEXTURE LOAD: every
//!     texture reaching the GPU is straight-alpha RGBA8 (or a directly-uploaded
//!     BCn whose authoring matched). Blend then pre-multiplies in-shader-factor:
//!     `src = SrcAlpha*src + (1-SrcAlpha)*dst` for straight alpha (no halos).
//!
//! The texture upload path prefers a directly-uploadable BCn (`Bc1/2/3` under
//! `TEXTURE_COMPRESSION_BC`) and otherwise CPU-decodes via `assets::dds::decode`.

pub mod borders;
pub mod dynamic;
pub mod map;
pub mod material;
pub mod modes;
pub mod overlay;
pub mod strategic_regions;
pub mod terrain;

use std::path::Path;
use std::sync::Arc;

use assets::dds::{self, AlphaMode, DdsFormat};
use assets::BmFont;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    tint: [f32; 4],
    mode: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    _pad: [f32; 2],
}

const MODE_SPRITE: u32 = 0;
const MODE_GLYPH: u32 = 1;

/// A GPU texture plus its source size and bind group.
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub format: DdsFormat,
    pub alpha: AlphaMode,
    bind_group: Arc<wgpu::BindGroup>,
    /// True if uploaded as a compressed BCn texture (vs CPU-decoded RGBA8).
    pub gpu_native: bool,
}

/// Offscreen renderer: owns the device/queue, an RGBA8 colour target, the
/// pipeline, and accumulates quads which are flushed in [`Renderer::finish`].
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub width: u32,
    pub height: u32,
    pub adapter_info: wgpu::AdapterInfo,
    pub bc_supported: bool,

    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    globals_bg: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    tex_layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler, // NEAREST despite the name slot

    // Accumulated draw batches: (texture bind group, vertex range).
    batches: Vec<Batch>,
    verts: Vec<Vertex>,
}

struct Batch {
    bind_group: Arc<wgpu::BindGroup>,
    start: u32,
    count: u32,
}

impl Renderer {
    /// Create an offscreen renderer of the given pixel size. No window/surface.
    pub fn new(width: u32, height: u32) -> Result<Renderer, String> {
        pollster::block_on(Self::new_async(width, height))
    }

    async fn new_async(width: u32, height: u32) -> Result<Renderer, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| "no wgpu adapter available".to_string())?;
        let adapter_info = adapter.get_info();
        let bc_supported = adapter
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);

        let mut features = wgpu::Features::empty();
        if bc_supported {
            features |= wgpu::Features::TEXTURE_COMPRESSION_BC;
        }
        // Use the adapter's real limits, clamped to default maxima. The
        // downlevel default caps texture dimensions at 2048, but real the game UI
        // textures exceed that (the topbar background strip is 2346 px wide), so
        // we adopt the adapter's higher limits (RTX-class hardware allows 16k).
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("m1-device"),
                    required_features: features,
                    required_limits: limits,
                },
                None,
            )
            .await
            .map_err(|e| format!("request_device failed: {e}"))?;

        Ok(Self::from_device(
            device,
            queue,
            adapter_info,
            bc_supported,
            width,
            height,
        ))
    }

    /// Build a `Renderer` over an EXISTING wgpu device/queue. The interactive app
    /// (the `app` crate) creates `Instance -> Surface -> Adapter(compatible_surface)
    /// -> Device/Queue` itself, so the surface and these render pipelines share ONE
    /// device — a texture/pipeline made here can then be presented to that surface
    /// (Codex §1: cross-device resources fail wgpu validation). The offscreen path
    /// (`new`) builds its own headless device and funnels here too, so there is one
    /// construction path and the golden tests exercise the same code.
    pub fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter_info: wgpu::AdapterInfo,
        bc_supported: bool,
        width: u32,
        height: u32,
    ) -> Renderer {
        // Offscreen colour target (RGBA8 unorm; we render UI in linear-ish bytes).
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("m1-color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // RENDER_ATTACHMENT+COPY_SRC for the offscreen->PNG golden path; plus
            // TEXTURE_BINDING so the interactive app's "present" pass can SAMPLE
            // this composited target into the surface frame (Codex §2: a present
            // bind group fails validation without TEXTURE_BINDING).
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());

        // NEAREST sampler (crisp UI). No mips.
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Bind group layouts.
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals {
                screen: [width as f32, height as f32],
                _pad: [0.0; 2],
            }),
            // COPY_DST so the app's `resize` can rewrite the screen dims in place.
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bg"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("m1-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("m1-pl"),
            bind_group_layouts: &[&globals_layout, &tex_layout],
            push_constant_ranges: &[],
        });

        let vbuf_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("m1-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[vbuf_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    // Straight-alpha over: out = src.a*src + (1-src.a)*dst.
                    // No DXT halos because textures are normalised to straight
                    // alpha at load and we never double-multiply.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Renderer {
            device,
            queue,
            width,
            height,
            adapter_info,
            bc_supported,
            color,
            color_view,
            pipeline,
            globals_bg,
            globals_buf,
            tex_layout,
            linear_sampler,
            batches: Vec::new(),
            verts: Vec::new(),
        }
    }

    /// Resize the offscreen composited target to `(w, h)` and update the globals
    /// (screen dims drive the pixel→NDC transform in the shader). Called by the app
    /// when the window/surface resizes so the compositor target tracks the surface
    /// size. The old colour texture is dropped; `globals_bg` keeps pointing at the
    /// same globals buffer (only its CONTENTS change), so no bind group rebuild.
    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 || (w == self.width && h == self.height) {
            return;
        }
        self.width = w;
        self.height = h;
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("m1-color-resized"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        self.color = color;
        self.queue.write_buffer(
            &self.globals_buf,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [w as f32, h as f32],
                _pad: [0.0; 2],
            }),
        );
    }

    /// Load a `.dds` from disk into a GPU texture, choosing the BCn-native path
    /// when supported & applicable, else the CPU RGBA8 transcode fallback.
    pub fn load_dds(&self, path: &Path) -> Result<Texture, String> {
        let bytes = read_bytes_retrying(path)?;
        self.load_dds_bytes(&bytes, &path.display().to_string())
    }

    /// Load an image file as a texture, dispatching on the actual MAGIC bytes
    /// (not just the extension): a `DDS ` magic -> the DDS path; otherwise the
    /// minimal TGA decoder ([`assets::tga`]). the game country flags ship as 32-bit
    /// uncompressed `.tga`, which `load_dds` cannot read — this is the loader the
    /// flag binding uses. Returns `Err` for an unrecognised format (never a
    /// fabricated placeholder).
    pub fn load_image(&self, path: &Path) -> Result<Texture, String> {
        let bytes = read_bytes_retrying(path)?;
        let label = path.display().to_string();
        if bytes.len() >= 4 && &bytes[0..4] == b"DDS " {
            return self.load_dds_bytes(&bytes, &label);
        }
        // PNG (the extracted `game_assets/assets/flags/*.png` set).
        if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
            let (w, h, rgba) = decode_png_rgba8(&bytes).map_err(|e| format!("{label}: {e}"))?;
            return Ok(self.upload_rgba8_image(w, h, &rgba));
        }
        // Assume TGA (the game flags). The decoder rejects formats it cannot handle.
        let img = assets::tga::decode(&bytes).map_err(|e| format!("{label}: {e}"))?;
        Ok(self.upload_rgba8_image(img.width, img.height, &img.rgba))
    }

    pub fn load_dds_bytes(&self, bytes: &[u8], label: &str) -> Result<Texture, String> {
        let header = dds::parse_header(bytes).map_err(|e| format!("{label}: {e}"))?;

        // Prefer direct BCn upload (DXT1/3/5) when the device supports it AND
        // the format is straight-alpha BCn (premultiplied DXT2/4 take CPU path).
        // BCn textures are 4x4-block compressed, so wgpu rejects a BCn texture
        // whose width/height is NOT a multiple of 4 (e.g. the game's 37px flag
        // overlays). Such odd-dimension BCn assets MUST take the CPU-decode path
        // (which yields an exact RGBA8 of any dimension) — Codex-flagged edge in
        // §3.1's "odd dimensions" list. Guard the direct path on 4-aligned dims.
        if self.bc_supported
            && dds::is_gpu_native_bcn(header.format)
            && header.width % 4 == 0
            && header.height % 4 == 0
        {
            if let Ok(blocks) = dds::read_gpu_blocks(bytes) {
                if let Some(wgpu_fmt) = bcn_wgpu_format(blocks.source_format) {
                    return Ok(self.upload_bcn(&blocks, wgpu_fmt));
                }
            }
        }
        // Fallback: CPU-decode to straight-alpha RGBA8 and upload.
        let img = dds::decode(bytes).map_err(|e| format!("{label}: {e}"))?;
        Ok(self.upload_rgba8(
            img.width,
            img.height,
            &img.rgba,
            img.source_format,
            img.alpha,
        ))
    }

    fn upload_rgba8(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
        format: DdsFormat,
        alpha: AlphaMode,
    ) -> Texture {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rgba8-tex"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.make_tex_bind_group(&view);
        Texture {
            width,
            height,
            format,
            alpha,
            bind_group,
            gpu_native: false,
        }
    }

    fn upload_bcn(&self, blocks: &dds::GpuBlocks, wgpu_fmt: wgpu::TextureFormat) -> Texture {
        let bpb = match wgpu_fmt {
            wgpu::TextureFormat::Bc1RgbaUnorm => 8,
            _ => 16,
        };
        let bw = ((blocks.width + 3) / 4).max(1);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bcn-tex"),
            size: wgpu::Extent3d {
                width: blocks.width,
                height: blocks.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_fmt,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &blocks.blocks,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bw * bpb),
                rows_per_image: Some(((blocks.height + 3) / 4).max(1)),
            },
            wgpu::Extent3d {
                width: blocks.width,
                height: blocks.height,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.make_tex_bind_group(&view);
        Texture {
            width: blocks.width,
            height: blocks.height,
            format: blocks.source_format,
            alpha: blocks.alpha,
            bind_group,
            gpu_native: true,
        }
    }

    /// Upload an arbitrary straight-alpha RGBA8 buffer as a texture (used for
    /// font atlases that come from a `.dds` or `.tga`).
    pub fn upload_rgba8_image(&self, width: u32, height: u32, rgba: &[u8]) -> Texture {
        self.upload_rgba8(width, height, rgba, DdsFormat::Rgba8, AlphaMode::Straight)
    }

    fn make_tex_bind_group(&self, view: &wgpu::TextureView) -> Arc<wgpu::BindGroup> {
        Arc::new(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tex-bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
            ],
        }))
    }

    // -- Quad accumulation ----------------------------------------------------

    /// Push one textured quad (two triangles). Positions are PIXEL-SNAPPED.
    fn push_quad(
        &mut self,
        tex: &Texture,
        dst: [f32; 4], // x, y, w, h in pixels
        uv: [f32; 4],  // u0, v0, u1, v1 in 0..1
        tint: [f32; 4],
        mode: u32,
    ) {
        let x0 = dst[0].round();
        let y0 = dst[1].round();
        let x1 = (dst[0] + dst[2]).round();
        let y1 = (dst[1] + dst[3]).round();
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
        let mk = |px: f32, py: f32, u: f32, v: f32| Vertex {
            pos: [px, py],
            uv: [u, v],
            tint,
            mode,
            _pad: [0; 3],
        };
        let start = self.verts.len() as u32;
        self.verts.push(mk(x0, y0, u0, v0));
        self.verts.push(mk(x1, y0, u1, v0));
        self.verts.push(mk(x1, y1, u1, v1));
        self.verts.push(mk(x0, y0, u0, v0));
        self.verts.push(mk(x1, y1, u1, v1));
        self.verts.push(mk(x0, y1, u0, v1));
        self.push_batch(tex, start, 6);
    }

    fn push_batch(&mut self, tex: &Texture, start: u32, count: u32) {
        // Merge with the previous batch if it uses the same texture (cheap
        // pointer-identity via bind group is not available; keep separate but
        // contiguous — correctness over batching for M1).
        self.batches.push(Batch {
            bind_group: tex.clone_bind_group(),
            start,
            count,
        });
    }

    /// (a) Draw a framed sprite: the whole texture stretched to `[x,y,w,h]`.
    /// `frame`/`no_of_frames` selects a horizontal sub-strip (1 => whole image).
    pub fn draw_sprite(&mut self, tex: &Texture, x: f32, y: f32, w: f32, h: f32) {
        self.draw_sprite_frame(tex, x, y, w, h, 0, 1, [1.0; 4]);
    }

    pub fn draw_sprite_frame(
        &mut self,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        frame: u32,
        no_of_frames: u32,
        tint: [f32; 4],
    ) {
        let n = no_of_frames.max(1);
        let fw = 1.0 / n as f32;
        let u0 = frame.min(n - 1) as f32 * fw;
        self.push_quad(
            tex,
            [x, y, w, h],
            [u0, 0.0, u0 + fw, 1.0],
            tint,
            MODE_SPRITE,
        );
    }

    /// (b) Draw a `corneredTileSpriteType` 9-slice frame. `border` is the
    /// borderSize (px) on the SOURCE texture; corners are drawn 1:1, edges
    /// stretched along one axis, the centre stretched on both. With NEAREST +
    /// pixel snapping the corners stay crisp.
    pub fn draw_nine_slice(
        &mut self,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        border: [f32; 2],
    ) {
        self.draw_nine_slice_clipped(tex, x, y, w, h, border, None);
    }

    /// As [`Renderer::draw_nine_slice`] but each of the nine quads is CROPPED to
    /// `clip` (the same per-quad scissor as clipped sprites/glyphs). This preserves
    /// the corner/edge/centre layout from the FULL rect while exactly clipping a
    /// partially-visible frame (e.g. a gridbox row's chrome straddling the scroll
    /// viewport edge) — Codex (a): a whole-rect cull leaks partial rows.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_nine_slice_clipped(
        &mut self,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        border: [f32; 2],
        clip: Option<[f32; 4]>,
    ) {
        let tw = tex.width as f32;
        let th = tex.height as f32;
        let bx = border[0].min(tw / 2.0).min(w / 2.0);
        let by = border[1].min(th / 2.0).min(h / 2.0);
        // Source border in UV.
        let su0 = 0.0;
        let su1 = border[0] / tw;
        let su2 = 1.0 - border[0] / tw;
        let su3 = 1.0;
        let sv0 = 0.0;
        let sv1 = border[1] / th;
        let sv2 = 1.0 - border[1] / th;
        let sv3 = 1.0;
        // Destination columns/rows (pixels).
        let dx0 = x;
        let dx1 = x + bx;
        let dx2 = x + w - bx;
        let dx3 = x + w;
        let dy0 = y;
        let dy1 = y + by;
        let dy2 = y + h - by;
        let dy3 = y + h;

        let cols = [
            (dx0, dx1, su0, su1),
            (dx1, dx2, su1, su2),
            (dx2, dx3, su2, su3),
        ];
        let rows = [
            (dy0, dy1, sv0, sv1),
            (dy1, dy2, sv1, sv2),
            (dy2, dy3, sv2, sv3),
        ];
        for (ry0, ry1, rv0, rv1) in rows {
            for (cx0, cx1, cu0, cu1) in cols {
                if cx1 - cx0 <= 0.0 || ry1 - ry0 <= 0.0 {
                    continue;
                }
                self.push_quad_clipped(
                    tex,
                    [cx0, ry0, cx1 - cx0, ry1 - ry0],
                    [cu0, rv0, cu1, rv1],
                    [1.0; 4],
                    MODE_SPRITE,
                    clip,
                );
            }
        }
    }

    /// Push one quad, CROPPED to an optional clip rect (dst + UV lerp). A quad
    /// fully outside the clip is skipped. The shared scissor for 9-slice / progress
    /// / any clipped sprite path. `dst` = [x,y,w,h]; `uv` = [u0,v0,u1,v1].
    #[allow(clippy::too_many_arguments)]
    fn push_quad_clipped(
        &mut self,
        tex: &Texture,
        dst: [f32; 4],
        uv: [f32; 4],
        tint: [f32; 4],
        mode: u32,
        clip: Option<[f32; 4]>,
    ) {
        let (mut x0, mut y0) = (dst[0], dst[1]);
        let (mut x1, mut y1) = (dst[0] + dst[2], dst[1] + dst[3]);
        let (mut u0, mut v0, mut u1, mut v1) = (uv[0], uv[1], uv[2], uv[3]);
        if let Some([cx, cy, cw, ch]) = clip {
            // a degenerate (empty) clip rejects everything (Codex (a) #2).
            if cw <= 0.0 || ch <= 0.0 {
                return;
            }
            let (clx0, cly0, clx1, cly1) = (cx, cy, cx + cw, cy + ch);
            if x1 <= clx0 || x0 >= clx1 || y1 <= cly0 || y0 >= cly1 {
                return;
            }
            crop_axis(&mut x0, &mut x1, &mut u0, &mut u1, clx0, clx1);
            crop_axis(&mut y0, &mut y1, &mut v0, &mut v1, cly0, cly1);
        }
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        self.push_quad(
            tex,
            [x0, y0, x1 - x0, y1 - y0],
            [u0, v0, u1, v1],
            tint,
            mode,
        );
    }

    /// (c) Draw a BMFont string as glyph quads. `x,y` is the pen origin; glyphs
    /// are placed at baseline = y + base. Honours xoffset/yoffset/xadvance +
    /// kerning + per-glyph page (multiple atlas textures). `color` tints the
    /// glyph coverage. The caller passes the RESOLVED string (post-markup).
    ///
    /// `pages`: one [`Texture`] per page id (index = page id).
    pub fn draw_text(
        &mut self,
        font: &BmFont,
        pages: &[Texture],
        text: &str,
        x: f32,
        y: f32,
        color: [f32; 4],
    ) {
        self.draw_text_clipped(font, pages, text, x, y, color, None);
    }

    /// As [`Renderer::draw_text`] but each glyph quad is CROPPED to `clip` (a pixel
    /// rect): a glyph fully outside the clip is skipped, a partially-clipped glyph
    /// is cropped (dst + UV). This is the scissor for clipped/scrolled text content
    /// (e.g. a gridbox row's `level` label scrolled out of its viewport) — without
    /// it, scrolled-out row text would visually leak past the viewport (Codex (a)).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_clipped(
        &mut self,
        font: &BmFont,
        pages: &[Texture],
        text: &str,
        x: f32,
        y: f32,
        color: [f32; 4],
        clip: Option<[f32; 4]>,
    ) {
        let mut pen_x = x;
        let mut prev: Option<u32> = None;
        for ch in text.chars() {
            let cp = ch as u32;
            let g = match font.glyphs.get(&cp) {
                Some(g) => *g,
                None => {
                    prev = None;
                    continue;
                }
            };
            if let Some(p) = prev {
                pen_x += font.kerning(p, cp) as f32;
            }
            let page = g.page as usize;
            if g.width > 0 && g.height > 0 && page < pages.len() {
                let tex = &pages[page];
                let tw = tex.width as f32;
                let th = tex.height as f32;
                let (u0, v0) = (g.x as f32 / tw, g.y as f32 / th);
                let (u1, v1) = (
                    (g.x as f32 + g.width as f32) / tw,
                    (g.y as f32 + g.height as f32) / th,
                );
                let gx = pen_x + g.xoffset as f32;
                let gy = y + g.yoffset as f32;
                // glyph quad cropped through the single scissor primitive.
                self.push_quad_clipped(
                    tex,
                    [gx, gy, g.width as f32, g.height as f32],
                    [u0, v0, u1, v1],
                    color,
                    MODE_GLYPH,
                    clip,
                );
            }
            pen_x += g.xadvance as f32;
            prev = Some(cp);
        }
    }

    /// (d) Draw a `progressbartype` bar: a `fraction` (0..1) horizontal slice of
    /// the bar sprite, cropped on BOTH the destination width and the source UV so
    /// the bar reveals left-to-right (the game fill direction). `frame`/
    /// `no_of_frames` pick a sub-strip like [`Renderer::draw_sprite_frame`].
    pub fn draw_progress_bar(
        &mut self,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fraction: f32,
        frame: u32,
        no_of_frames: u32,
        tint: [f32; 4],
    ) {
        self.draw_progress_bar_clipped(tex, x, y, w, h, fraction, frame, no_of_frames, tint, None);
    }

    /// As [`Renderer::draw_progress_bar`] but the (already fractioned) fill quad is
    /// CROPPED to `clip` — so a bar inside a scrolled viewport that straddles the
    /// edge is exactly clipped, not whole-rect culled (Codex (a)).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_progress_bar_clipped(
        &mut self,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fraction: f32,
        frame: u32,
        no_of_frames: u32,
        tint: [f32; 4],
        clip: Option<[f32; 4]>,
    ) {
        let f = fraction.clamp(0.0, 1.0);
        if f <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let n = no_of_frames.max(1);
        let fw = 1.0 / n as f32;
        let u0 = frame.min(n - 1) as f32 * fw;
        let u1 = u0 + fw * f;
        self.push_quad_clipped(
            tex,
            [x, y, w * f, h],
            [u0, 0.0, u1, 1.0],
            tint,
            MODE_SPRITE,
            clip,
        );
    }

    /// Draw a sprite frame, but CLIPPED to `clip` (a pixel rect). The dst and its
    /// UVs are intersected with `clip`; a quad fully outside the clip is skipped.
    /// This is the CPU scissor used for static clipped/scroll content in M2.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_sprite_frame_clipped(
        &mut self,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        frame: u32,
        no_of_frames: u32,
        tint: [f32; 4],
        clip: Option<[f32; 4]>,
    ) {
        let n = no_of_frames.max(1);
        let fw = 1.0 / n as f32;
        let fu0 = frame.min(n - 1) as f32 * fw;
        let fu1 = fu0 + fw;
        // Single CPU scissor primitive (Codex): all clipped quad paths — sprite,
        // glyph, 9-slice, progressbar — crop through `push_quad_clipped`, so the
        // scissor math lives in exactly one place.
        self.push_quad_clipped(
            tex,
            [x, y, w, h],
            [fu0, 0.0, fu1, 1.0],
            tint,
            MODE_SPRITE,
            clip,
        );
    }

    /// The offscreen colour target's view (shared by other passes — e.g. the M4
    /// map pass renders into the SAME target so the topbar can stack above it).
    pub fn offscreen_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    /// Public wrapper over the internal colour-target readback (RGBA8, tightly
    /// packed). Used by passes that submit their own command encoder (the map).
    pub fn readback_public(&self) -> Vec<u8> {
        self.readback()
    }

    /// Clear + render all accumulated quads to the offscreen target and read it
    /// back as RGBA8. `clear` is the background colour (0..1).
    ///
    /// IMPORTANT (Codex-flagged): the blend stores STRAIGHT-alpha-over results.
    /// With an OPAQUE clear (alpha = 1.0) the read-back RGB is the correctly
    /// composited colour and every output pixel is opaque — safe to PNG as-is.
    /// If you ever clear to a TRANSPARENT background, the stored RGB becomes
    /// premultiplied and an alpha PNG would need un-premultiplying first. M1
    /// uses an opaque clear, so no un-premultiply is required here.
    pub fn finish(&mut self, clear: [f64; 4]) -> Vec<u8> {
        self.flush(Some(clear))
    }

    /// Like [`Renderer::finish`] but LOADS the existing offscreen contents instead
    /// of clearing — so accumulated quads composite OVER whatever a prior pass
    /// (e.g. the M4 map) already rendered into the shared target. This is the
    /// compositor stacking path: map renders+clears, then the topbar + state panel
    /// quads draw on top via `finish_no_clear`.
    pub fn finish_no_clear(&mut self) -> Vec<u8> {
        self.flush(None)
    }

    /// LIVE-PATH flush: render the accumulated quads to the shared offscreen target
    /// and submit, WITHOUT the CPU readback that [`finish`]/[`finish_no_clear`] do.
    /// `clear = Some(c)` clears first, `None` composites over the existing contents
    /// (the map drew first). The composited target stays on the GPU; the app then
    /// PRESENTS it to the surface via a sampling pass — so no per-frame 8 MB readback
    /// stalls the pipeline (Codex §2). Drains the accumulated batches like `flush`.
    pub fn flush_no_readback(&mut self, clear: Option<[f64; 4]>) {
        let vbuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("verts-live"),
                contents: bytemuck::cast_slice(&self.verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let load = match clear {
            Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
                r: c[0],
                g: c[1],
                b: c[2],
                a: c[3],
            }),
            None => wgpu::LoadOp::Load,
        };
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("enc-live"),
            });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rp-live"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_vertex_buffer(0, vbuf.slice(..));
            for b in &self.batches {
                rp.set_bind_group(1, b.bind_group.as_ref(), &[]);
                rp.draw(b.start..b.start + b.count, 0..1);
            }
        }
        self.queue.submit(Some(enc.finish()));
        self.batches.clear();
        self.verts.clear();
    }

    /// Drop any accumulated (un-flushed) quads without drawing them. Used by the app
    /// if it abandons a frame before presenting.
    pub fn discard_batches(&mut self) {
        self.batches.clear();
        self.verts.clear();
    }

    /// A bind group that SAMPLES the offscreen composited target (for the app's
    /// present pass). Built against this renderer's `tex_layout` + nearest sampler,
    /// so it is compatible with a present pipeline that reuses that layout.
    pub fn offscreen_bind_group(&self) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("offscreen-sample-bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
            ],
        })
    }

    /// The texture-bind-group layout (texture@0 + sampler@1) the draw pipelines use —
    /// the app's present pipeline reuses it so [`offscreen_bind_group`] binds cleanly.
    pub fn tex_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.tex_layout
    }

    /// Shared flush: `clear = Some(c)` clears first, `None` loads the existing
    /// target (composite-over). Drains the accumulated quad batches either way.
    fn flush(&mut self, clear: Option<[f64; 4]>) -> Vec<u8> {
        let vbuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("verts"),
                contents: bytemuck::cast_slice(&self.verts),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let load = match clear {
            Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
                r: c[0],
                g: c[1],
                b: c[2],
                a: c[3],
            }),
            None => wgpu::LoadOp::Load,
        };

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("enc") });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.globals_bg, &[]);
            rp.set_vertex_buffer(0, vbuf.slice(..));
            for b in &self.batches {
                rp.set_bind_group(1, b.bind_group.as_ref(), &[]);
                rp.draw(b.start..b.start + b.count, 0..1);
            }
        }
        self.queue.submit(Some(enc.finish()));
        self.readback()
    }

    /// Copy the colour target to a buffer and read it back as tightly-packed
    /// RGBA8 (handles the 256-byte row alignment requirement).
    fn readback(&self) -> Vec<u8> {
        let bytes_per_pixel = 4u32;
        let unpadded = self.width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = ((unpadded + align - 1) / align) * align;
        let buf_size = (padded * self.height) as wgpu::BufferAddress;
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: buf_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rb-enc"),
            });
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &out_buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(enc.finish()));

        let slice = out_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();

        let mut out = vec![0u8; (self.width * self.height * 4) as usize];
        for row in 0..self.height {
            let src = (row * padded) as usize;
            let dst = (row * unpadded) as usize;
            out[dst..dst + unpadded as usize].copy_from_slice(&data[src..src + unpadded as usize]);
        }
        drop(data);
        out_buf.unmap();
        out
    }
}

impl Texture {
    /// Share the texture's bind group (Arc clone — cheap refcount bump).
    fn clone_bind_group(&self) -> Arc<wgpu::BindGroup> {
        Arc::clone(&self.bind_group)
    }
}

/// True if an IO error is a TRANSIENT read failure consistent with the file being
/// mid-rewrite by the concurrent extractor (the agent regenerating `game_assets/`):
/// the file briefly vanishes (`NotFound`), is locked for the rename/replace
/// (`PermissionDenied` — a Windows sharing violation surfaces here), or the read is
/// interrupted (`Interrupted`). These are RETRYABLE — the asset is expected to
/// reappear. Any other error (e.g. a genuinely malformed/short file that decodes
/// wrong) is NOT retried here; it surfaces to the caller as a hard error.
fn is_transient_io(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(e.kind(), NotFound | PermissionDenied | Interrupted)
}

/// Read a file's bytes, RETRYING a few times with a short backoff on TRANSIENT IO
/// errors (the extractor may be mid-rewrite of this exact asset — Stage 5 hardening:
/// "if an asset read transiently fails, RETRY; never edit game_assets/"). A
/// non-transient error, or exhausting the retry budget, returns `Err` so the caller
/// can decide (the texture cache treats this as a TRANSIENT miss it does NOT
/// poison-cache, so a later frame re-attempts once the rewrite settles).
fn read_bytes_retrying(path: &Path) -> Result<Vec<u8>, String> {
    // 5 attempts over ~ (2+4+8+16) = 30 ms total — long enough to span a file
    // replace, short enough to never stall a frame perceptibly. Read-only: this
    // only ever READS the asset, never writes/edits it.
    const ATTEMPTS: u32 = 5;
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..ATTEMPTS {
        match std::fs::read(path) {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                if !is_transient_io(&e) || attempt + 1 == ATTEMPTS {
                    return Err(format!("{}: {e}", path.display()));
                }
                // exponential-ish backoff: 2,4,8,16 ms.
                std::thread::sleep(std::time::Duration::from_millis(2u64 << attempt));
                last = Some(e);
            }
        }
    }
    // unreachable (the loop always returns), but keep the error if it ever falls out.
    Err(format!(
        "{}: {}",
        path.display(),
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "read failed".into())
    ))
}

/// Crop one axis of a textured quad against a clip span `[lo,hi]`, lerping the UV
/// to match the cropped destination. Shared by sprite + glyph clipping so the
/// scissor math is identical (and correct) for both.
fn crop_axis(a: &mut f32, b: &mut f32, ua: &mut f32, ub: &mut f32, lo: f32, hi: f32) {
    let span = *b - *a;
    if span <= 0.0 {
        return;
    }
    let nlo = a.max(lo);
    let nhi = b.min(hi);
    let t0 = (nlo - *a) / span;
    let t1 = (nhi - *a) / span;
    let uspan = *ub - *ua;
    let nua = *ua + uspan * t0;
    let nub = *ua + uspan * t1;
    *a = nlo;
    *b = nhi;
    *ua = nua;
    *ub = nub;
}

/// Map a straight-alpha BCn format to its wgpu equivalent (linear, non-sRGB).
fn bcn_wgpu_format(f: DdsFormat) -> Option<wgpu::TextureFormat> {
    match f {
        DdsFormat::Dxt1 => Some(wgpu::TextureFormat::Bc1RgbaUnorm),
        DdsFormat::Dxt3 => Some(wgpu::TextureFormat::Bc2RgbaUnorm),
        DdsFormat::Dxt5 => Some(wgpu::TextureFormat::Bc3RgbaUnorm),
        DdsFormat::Bc4 => Some(wgpu::TextureFormat::Bc4RUnorm),
        DdsFormat::Bc5 => Some(wgpu::TextureFormat::Bc5RgUnorm),
        DdsFormat::Bc7 => Some(wgpu::TextureFormat::Bc7RgbaUnorm),
        _ => None,
    }
}

/// Decode a PNG byte stream to straight-alpha RGBA8 `(w, h, rgba)`. Handles
/// 8-bit RGB / RGBA / grayscale(+alpha) by expanding to 4 channels. Used for the
/// extracted `game_assets` flag PNGs (which `load_dds`/TGA can't read).
fn decode_png_rgba8(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    let ct = info.color_type;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("unsupported PNG bit depth {:?}", info.bit_depth));
    }
    let ch = ct.samples();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let data = &buf[..frame.buffer_size()];
    let n = (w as usize) * (h as usize);
    let mut rgba = vec![0u8; n * 4];
    for i in 0..n {
        let o = i * ch;
        let (r, g, b, a) = match ct {
            png::ColorType::Rgba => (data[o], data[o + 1], data[o + 2], data[o + 3]),
            png::ColorType::Rgb => (data[o], data[o + 1], data[o + 2], 255),
            png::ColorType::GrayscaleAlpha => (data[o], data[o], data[o], data[o + 1]),
            png::ColorType::Grayscale => (data[o], data[o], data[o], 255),
            png::ColorType::Indexed => return Err("indexed PNG unsupported".into()),
        };
        let d = i * 4;
        rgba[d] = r;
        rgba[d + 1] = g;
        rgba[d + 2] = b;
        rgba[d + 3] = a;
    }
    Ok((w, h, rgba))
}

/// Save a tightly-packed RGBA8 buffer to a PNG.
pub fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(path)?;
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    Ok(())
}

#[cfg(test)]
mod transient_read_tests {
    use super::*;

    #[test]
    fn classifies_transient_vs_hard_io() {
        use std::io::{Error, ErrorKind};
        // the three race-shaped kinds (file mid-rewrite) are transient/retryable.
        assert!(is_transient_io(&Error::from(ErrorKind::NotFound)));
        assert!(is_transient_io(&Error::from(ErrorKind::PermissionDenied)));
        assert!(is_transient_io(&Error::from(ErrorKind::Interrupted)));
        // a hard, non-race error is NOT retried.
        assert!(!is_transient_io(&Error::from(ErrorKind::InvalidData)));
        assert!(!is_transient_io(&Error::from(ErrorKind::Other)));
    }

    #[test]
    fn read_retrying_succeeds_on_present_file() {
        let p = std::env::temp_dir().join(format!("render_retry_ok_{}.bin", std::process::id()));
        std::fs::write(&p, b"hello").unwrap();
        let bytes = read_bytes_retrying(&p).expect("present file must read");
        assert_eq!(bytes, b"hello");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn read_retrying_errors_after_budget_on_missing_file() {
        // A file that never appears: the retry budget is exhausted and we return Err
        // (NOT a panic, NOT a fabricated empty buffer). The caller treats this as a
        // transient miss it does not poison-cache.
        let p = std::env::temp_dir().join(format!(
            "render_retry_missing_{}_never.bin",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        let start = std::time::Instant::now();
        let r = read_bytes_retrying(&p);
        assert!(
            r.is_err(),
            "a never-present file must error, not fabricate bytes"
        );
        // it actually backed off across attempts (≥ the 2+4+8+16 ms schedule minus slack).
        assert!(start.elapsed() >= std::time::Duration::from_millis(20));
    }

    #[test]
    fn read_retrying_recovers_when_file_appears_late() {
        // Simulate the extractor finishing mid-retry: the file is absent on the first
        // attempts then created by a background thread. The retry loop must RECOVER
        // (this is the whole point — a transient miss is not permanent).
        let p = std::env::temp_dir().join(format!("render_retry_late_{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let p2 = p.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(6));
            std::fs::write(&p2, b"late").unwrap();
        });
        let bytes = read_bytes_retrying(&p).expect("must recover once the file appears");
        assert_eq!(bytes, b"late");
        h.join().unwrap();
        let _ = std::fs::remove_file(&p);
    }
}
