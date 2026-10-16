//! Free-eye icon placement in the sprite pass: each shown icon scales about its anchor, and an icon
//! below full opacity draws offscreen and blends in as one image.
//!
//! A fading icon's quads draw into a cell of a fade page exactly as they would on screen (same
//! programs, binds and order), with the page holding premultiplied colour: each draw's alpha
//! accumulates as `One, OneMinusSrcAlpha`. The cell is then drawn over the frame at the icon's
//! place in the draw order with its opacity, so overlapping parts of one icon fade together.

use eframe::wgpu;

use super::super::gpu_contract::{sampler_entry, texture_entry};

/// A clip-space map `x' = scale.x * x + offset.x * w` (and the same for `y`), applied after a
/// draw's own projection: a uniform scale about a point and a move to another target, in
/// normalised device coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ClipAffine {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

/// Where a quad drawn on a target of `size` device pixels lands instead: scaled by `scale` about
/// device point `pivot`, moved by `shift` device pixels, on a target of `out` device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Placement {
    pub scale: f32,
    pub pivot: [f32; 2],
    pub shift: [f32; 2],
    pub size: [u32; 2],
    pub out: [u32; 2],
}

impl Placement {
    /// Drawn where laid out.
    pub fn identity(size: [u32; 2]) -> Self {
        Self {
            scale: 1.0,
            pivot: [0.0; 2],
            shift: [0.0; 2],
            size,
            out: size,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.scale == 1.0 && self.shift == [0.0; 2] && self.size == self.out
    }

    /// A device point's new place, in device pixels of the output target.
    pub fn point(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        [
            self.scale * (x - self.pivot[0]) + self.pivot[0] + self.shift[0],
            self.scale * (y - self.pivot[1]) + self.pivot[1] + self.shift[1],
        ]
    }

    /// A device rectangle `[x0, y0, x1, y1]` placed.
    pub fn rect(&self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
        let [a, b] = self.point([x0, y0]);
        let [c, d] = self.point([x1, y1]);
        [a.min(c), b.min(d), a.max(c), b.max(d)]
    }

    /// The same map in clip space, for draws that project themselves.
    pub fn clip(&self) -> ClipAffine {
        let [width, height] = self.size.map(|v| v as f32);
        let [out_width, out_height] = self.out.map(|v| v as f32);
        let s = self.scale;
        // x_out_px = s * (x_ndc + 1) / 2 * W + (1 - s) * pivot.x + shift.x, then to NDC.
        let x_constant = (1.0 - s) * self.pivot[0] + self.shift[0];
        let y_constant = (1.0 - s) * self.pivot[1] + self.shift[1];
        ClipAffine {
            scale: [s * width / out_width, s * height / out_height],
            offset: [
                (s * width + 2.0 * x_constant) / out_width - 1.0,
                1.0 - (s * height + 2.0 * y_constant) / out_height,
            ],
        }
    }

    /// A device scissor `[x, y, width, height]` placed and cut to `within` (`[x0, y0, x1, y1]`
    /// in output device pixels); `None` when nothing is left.
    pub fn scissor(&self, [x, y, w, h]: [u32; 4], within: [u32; 4]) -> Option<[u32; 4]> {
        let [x0, y0, x1, y1] = self.rect([x as f32, y as f32, (x + w) as f32, (y + h) as f32]);
        cut([x0, y0, x1, y1], within)
    }
}

/// `[x0, y0, x1, y1]` rounded outwards and cut to `within`, as a scissor `[x, y, width, height]`.
pub(super) fn cut([x0, y0, x1, y1]: [f32; 4], within: [u32; 4]) -> Option<[u32; 4]> {
    let x0 = (x0.floor().max(within[0] as f32)) as u32;
    let y0 = (y0.floor().max(within[1] as f32)) as u32;
    let x1 = (x1.ceil().min(within[2] as f32)).max(0.0) as u32;
    let y1 = (y1.ceil().min(within[3] as f32)).max(0.0) as u32;
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0])
}

/// Applies `affine` to a native draw's buffer zero, whose first 16 floats are its column-major
/// world-view-projection matrix.
pub(super) fn apply(affine: ClipAffine, constants: &mut [f32]) {
    for column in 0..4 {
        let at = column * 4;
        let w = constants[at + 3];
        constants[at] = affine.scale[0] * constants[at] + affine.offset[0] * w;
        constants[at + 1] = affine.scale[1] * constants[at + 1] + affine.offset[1] * w;
    }
}

/// Fade cells are placed this far apart, in device pixels, so filtering never reaches a
/// neighbour.
pub(super) const GAP: u32 = 2;

/// Rows of cells on pages of one size, filled left to right and top to bottom, neighbouring cells
/// [`GAP`] apart.
#[derive(Clone, Debug)]
pub(super) struct Shelves {
    page: [u32; 2],
    pub pages: usize,
    /// How far each page's cells reach, in device pixels.
    pub extents: Vec<[u32; 2]>,
    x: u32,
    y: u32,
    row: u32,
}

impl Shelves {
    pub fn new(page: [u32; 2]) -> Self {
        Self {
            page,
            pages: 0,
            extents: Vec::new(),
            x: 0,
            y: 0,
            row: 0,
        }
    }

    /// A cell of `size` device pixels: its page and top-left corner. A cell as large as a page
    /// takes a page alone; the caller makes pages as large as its largest cell
    /// ([`Pages::shelves`]).
    pub fn place(&mut self, size: [u32; 2]) -> (usize, [u32; 2]) {
        if self.pages == 0 {
            self.pages = 1;
        }
        if self.x > 0 && self.x + size[0] > self.page[0] {
            self.x = 0;
            self.y += self.row;
            self.row = 0;
        }
        if self.y > 0 && self.y + size[1] > self.page[1] {
            self.pages += 1;
            self.x = 0;
            self.y = 0;
            self.row = 0;
        }
        let at = [self.x, self.y];
        self.x += size[0] + GAP;
        self.row = self.row.max(size[1] + GAP);
        let page = self.pages - 1;
        self.extents.resize(self.pages, [0, 0]);
        let extent = &mut self.extents[page];
        *extent = [0, 1].map(|axis| extent[axis].max(at[axis] + size[axis]));
        (page, at)
    }
}

/// One fade page on the device.
pub(super) struct Page {
    pub view: wgpu::TextureView,
    pub group: wgpu::BindGroup,
    /// Its device pixels.
    pub size: [u32; 2],
}

/// The fade pages: their composite program, the layout they bind through, and the pages the last
/// upload uses, each as large as what it holds.
pub(super) struct Pages {
    /// The side cells pack within, unless a cell is larger.
    pub side: u32,
    /// The device's largest texture side.
    limit: u32,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pub composite: wgpu::RenderPipeline,
    pub pages: Vec<Page>,
}

/// Device pixels cells pack within a side, within the device's limit; a larger cell packs within
/// its own size ([`Pages::shelves`]).
const PAGE_SIDE: u32 = 2048;

/// Fade pages are sized in steps of this many device pixels a side, so a demand that changes a
/// little keeps its page.
const GRAIN: u32 = 256;

impl Pages {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let limit = device.limits().max_texture_dimension_2d;
        let side = PAGE_SIDE.min(limit);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite fade page"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                ),
                sampler_entry(1),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite fade page"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite fade composite"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite fade composite"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite fade composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<CompositeVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32],
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
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: premultiplied,
                        alpha: premultiplied,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            side,
            limit,
            format,
            layout,
            sampler,
            composite,
            pages: Vec::new(),
        }
    }

    /// Shelves for one upload whose largest cell is `largest` device pixels: cells pack within
    /// the standard side, or within the largest cell where it is larger. A cell is cut to the
    /// frame, a target on the same device, so it is within the device's limit.
    pub fn shelves(&self, largest: [u32; 2]) -> Shelves {
        Shelves::new(largest.map(|side| side.max(self.side).min(self.limit)))
    }

    /// Makes the pages hold `extents` (each page's cells' reach, from [`Shelves`]). A side of a
    /// page stays while it holds its cells and is at most twice the size they need; a side that
    /// does not takes the size they need, and a page with such a side is replaced, keeping its
    /// other side if that stays. Pages past the last are dropped.
    pub fn fit(&mut self, device: &wgpu::Device, extents: &[[u32; 2]]) {
        self.pages.truncate(extents.len());
        for (index, extent) in extents.iter().enumerate() {
            let wanted = extent.map(|side| side.max(1).next_multiple_of(GRAIN).min(self.limit));
            let current = self.pages.get(index).map(|page| page.size);
            let size = [0, 1].map(|axis| match current {
                Some(have) if have[axis] >= extent[axis] && have[axis] <= 2 * wanted[axis] => {
                    have[axis]
                }
                _ => wanted[axis],
            });
            if current == Some(size) {
                continue;
            }
            let page = self.page(device, size);
            if index < self.pages.len() {
                self.pages[index] = page;
            } else {
                self.pages.push(page);
            }
        }
    }

    /// Page `index`'s device pixels.
    pub fn size(&self, index: usize) -> [u32; 2] {
        self.pages[index].size
    }

    /// A page of `size` device pixels.
    fn page(&self, device: &wgpu::Device, size: [u32; 2]) -> Page {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite fade page"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite fade page"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Page { view, group, size }
    }
}

/// One corner of a cell drawn over the frame: its clip position, its page coordinate and the
/// icon's opacity.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct CompositeVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub alpha: f32,
}

/// The six corners drawing the page cell `cell` (`[x0, y0, x1, y1]` page pixels of a page of
/// `page` pixels) over the frame rectangle `at` (`[x0, y0, x1, y1]` device pixels of a target of
/// `size`) at opacity `alpha`.
pub(super) fn composite_quad(
    cell: [f32; 4],
    page: [u32; 2],
    at: [f32; 4],
    size: [u32; 2],
    alpha: f32,
) -> [CompositeVertex; 6] {
    let clip = |x: f32, y: f32| {
        [
            x / size[0] as f32 * 2.0 - 1.0,
            1.0 - y / size[1] as f32 * 2.0,
        ]
    };
    let uv = |x: f32, y: f32| [x / page[0] as f32, y / page[1] as f32];
    let corner = |[x, y]: [f32; 2], [u, v]: [f32; 2]| CompositeVertex {
        position: clip(x, y),
        uv: uv(u, v),
        alpha,
    };
    let a = corner([at[0], at[1]], [cell[0], cell[1]]);
    let b = corner([at[2], at[1]], [cell[2], cell[1]]);
    let c = corner([at[2], at[3]], [cell[2], cell[3]]);
    let d = corner([at[0], at[3]], [cell[0], cell[3]]);
    [a, b, c, a, c, d]
}

/// Draws a page cell, premultiplied, at the icon's opacity.
const COMPOSITE: &str = r#"
struct CompositeVertex {
    @location(0) clip_xy: vec2<f32>,
    @location(1) texcoord: vec2<f32>,
    @location(2) alpha: f32,
}

struct CompositeFragment {
    @builtin(position) position: vec4<f32>,
    @location(0) texcoord: vec2<f32>,
    @location(1) @interpolate(flat) alpha: f32,
}

@group(0) @binding(0) var page_texture: texture_2d<f32>;
@group(0) @binding(1) var page_sampler: sampler;

@vertex
fn vertex_main(vertex: CompositeVertex) -> CompositeFragment {
    var fragment: CompositeFragment;
    fragment.position = vec4<f32>(vertex.clip_xy, 0.0, 1.0);
    fragment.texcoord = vertex.texcoord;
    fragment.alpha = vertex.alpha;
    return fragment;
}

@fragment
fn fragment_main(fragment: CompositeFragment) -> @location(0) vec4<f32> {
    return textureSampleLevel(page_texture, page_sampler, fragment.texcoord, 0.0) * fragment.alpha;
}
"#;

#[cfg(test)]
mod tests;
