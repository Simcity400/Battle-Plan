use shader_pipeline::gpu::stats::CountedPasses as _;
use std::cell::{Cell, RefCell};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use viewer_gpu::gpu_contract::DEPTH_FORMAT;

use eframe::wgpu;

use viewer_gpu::camera::{Camera, WATER_HEIGHT};
use viewer_gpu::frame::FrameConstants;
use viewer_gpu::gpu_contract::extent;
use viewer_gpu::textures::{Content, Sampled};

pub const DEPTH_MAP_SIZE: u32 = 2048;

pub const DEPTH_MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

pub const BUFFER_FORMAT: wgpu::TextureFormat = super::shadow_blur::COLOR_FORMAT;

pub const BUFFER_CLEAR: wgpu::Color = wgpu::Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

const LIGHT_BOX_PAD: f32 = 10.0;
const LIGHT_BOX_BACKOFF: f32 = 30.0;

const DOWNWARD: f32 = 0.001;

const RAY_CAP: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub basis: [[f32; 3]; 3],
    pub eye: [f32; 3],
}

pub fn light_direction(camera: Camera, frame: &FrameConstants) -> [f32; 3] {
    let eye = camera.eye();
    let look = camera.look_direction();
    let reach = (eye[1] / look[1]).abs();
    let t = match camera.free() {
        None => reach,
        Some(_) => reach.min(eye[1] * RAY_CAP),
    };
    let ground = [eye[0] + t * look[0], 0.0, eye[2] + t * look[2]];
    let sun = frame.virtual_sun_pos;
    let moon = frame.virtual_moon_pos;
    let light = if distance2(eye, [moon[0], moon[1], moon[2]])
        <= distance2(eye, [sun[0], sun[1], sun[2]])
    {
        moon
    } else {
        sun
    };
    let width = camera.map.x();
    let x = [light[0] - width, light[0], light[0] + width]
        .into_iter()
        .min_by(|a, b| (a - ground[0]).abs().total_cmp(&(b - ground[0]).abs()))
        .unwrap_or(light[0]);
    normalize([ground[0] - x, -light[1], ground[2] - light[2]])
}

pub fn light_box(camera: Camera, viewport: [u32; 2], light: [f32; 3]) -> Option<LightBox> {
    let [a, b, c, d, middle] = footprint(camera, viewport)?;
    Some(light_box_from_footprint([a, b, c, d], middle, light))
}

fn footprint(camera: Camera, viewport: [u32; 2]) -> Option<[[f32; 3]; 5]> {
    let points = [
        [-1.0, 1.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [1.0, -1.0],
        [0.0, 0.0],
    ]
    .map(|ndc| water_plane_point(camera, viewport, ndc));
    (!points.iter().flatten().any(|value| !value.is_finite())).then_some(points)
}

fn water_plane_point(camera: Camera, viewport: [u32; 2], ndc: [f32; 2]) -> [f32; 3] {
    let eye = camera.eye();
    let ray = camera.ray_at_ndc(ndc, viewport);
    let cap = eye[1] * RAY_CAP;
    let distance = if ray[1] < -DOWNWARD {
        (eye[1] - WATER_HEIGHT) / ray[1].abs()
    } else {
        cap
    };
    let distance = native_min(cap, native_max(0.0, distance));
    [
        ray[0] * distance + eye[0],
        WATER_HEIGHT,
        ray[2] * distance + eye[2],
    ]
}

fn light_box_from_footprint(corners: [[f32; 3]; 4], middle: [f32; 3], light: [f32; 3]) -> LightBox {
    let centre = [snap10(middle[0]), WATER_HEIGHT, snap10(middle[2])];
    let fit = normalize([light[0], 0.0, light[2]]);
    let fit_back = [-fit[0], -fit[1], -fit[2]];
    let forward = normalize(light);
    let back = [-forward[0], -forward[1], -forward[2]];
    let mut distance = dot(fit_back, subtract(corners[0], centre));
    for corner in &corners[1..] {
        distance = native_max(dot(fit_back, subtract(*corner, centre)), distance);
    }
    distance += LIGHT_BOX_BACKOFF;
    let eye = [
        centre[0] + back[0] * distance,
        centre[1] + back[1] * distance,
        centre[2] + back[2] * distance,
    ];
    let raw_forward = subtract(centre, eye);
    let raw_right = cross([0.0, 1.0, 0.0], raw_forward);
    let raw_up = cross(raw_forward, raw_right);
    let basis = [raw_right, raw_up, raw_forward].map(normalize);
    let translation = basis.map(|axis| -dot(axis, eye));

    let projected: [[f32; 3]; 4] = corners.map(|corner| {
        std::array::from_fn(|axis| {
            corner[1] * basis[axis][1]
                + corner[0] * basis[axis][0]
                + corner[2] * basis[axis][2]
                + translation[axis]
        })
    });
    let mut min = [0.0; 3];
    let mut max = [0.0; 3];
    for (axis, order) in [[0, 2, 3, 1], [2, 0, 3, 1], [2, 0, 3, 1]]
        .into_iter()
        .enumerate()
    {
        min[axis] = projected[order[0]][axis];
        max[axis] = min[axis];
        for index in &order[1..] {
            min[axis] = native_min(projected[*index][axis], min[axis]);
            max[axis] = native_max(projected[*index][axis], max[axis]);
        }
    }
    LightBox {
        min: min.map(|value| snap10(value - LIGHT_BOX_PAD)),
        max: max.map(|value| snap10(value + LIGHT_BOX_PAD)),
        basis,
        eye,
    }
}

fn native_min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn native_max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

fn snap10(value: f32) -> f32 {
    let integer = if (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    };
    (integer / 10 * 10) as f32
}

impl LightBox {
    pub fn view_projection(&self) -> [[f32; 4]; 4] {
        let [right, up, forward] = self.basis;
        let view = [
            [right[0], up[0], forward[0], 0.0],
            [right[1], up[1], forward[1], 0.0],
            [right[2], up[2], forward[2], 0.0],
            [
                -dot(right, self.eye),
                -dot(up, self.eye),
                -dot(forward, self.eye),
                1.0,
            ],
        ];
        let (l, r) = (self.min[0], self.max[0]);
        let (b, t) = (self.min[1], self.max[1]);
        let (n, f) = (self.min[2], self.max[2]);
        let ortho = [
            [2.0 / (r - l), 0.0, 0.0, 0.0],
            [0.0, 2.0 / (t - b), 0.0, 0.0],
            [0.0, 0.0, 1.0 / (f - n), 0.0],
            [(l + r) / (l - r), (t + b) / (b - t), -n / (f - n), 1.0],
        ];
        multiply(view, ortho)
    }

    pub fn texture_matrix(&self) -> [[f32; 4]; 4] {
        let bias = [
            [0.5, 0.0, 0.0, 0.0],
            [0.0, -0.5, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.5, 0.5, 0.0, 1.0],
        ];
        multiply(self.view_projection(), bias)
    }
}

pub fn draws_shadows(camera: &Camera) -> bool {
    camera.height < camera.defines.shadows[0]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Half,
    Full,
}

impl Screen {
    pub fn size(self, view: [u32; 2]) -> [u32; 2] {
        match self {
            Self::Half => view.map(|side| side.div_ceil(2)),
            Self::Full => view,
        }
    }
}

pub fn texel_reads(
    compiled: &shader_pipeline::Compiled,
    slot: u32,
    technique: &str,
) -> Option<String> {
    let why = shader_pipeline::texel::by_texel(
        &compiled.pixel.wgsl,
        shader_pipeline::binding(slot, content_reader_gui::pdxshader::program::Stage::Pixel),
        "shadow buffer",
    )?;
    Some(format!(
        "half-size shadow buffer not used: the {technique} technique {why}"
    ))
}

pub fn stages(camera: &Camera, shadows_enabled: bool) -> [bool; 2] {
    let casters = shadows_enabled && draws_shadows(camera);
    [casters, casters || camera.height < camera.defines.fog[0]]
}

struct Buffers {
    depth_map: Option<wgpu::TextureView>,
    depth_map_depth: Option<wgpu::TextureView>,
    buffer: wgpu::TextureView,
    buffer_depth: wgpu::TextureView,
    scratch: wgpu::TextureView,
    size: [u32; 2],
    screen: Screen,
    content: Cell<Content>,
    cleared: Cell<bool>,
    depth_map_held: RefCell<Held>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CasterId(u64);

impl CasterId {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CasterKey {
    light: [[u32; 4]; 4],
    camera: Camera,
    viewport: [u32; 2],
    offsets: Vec<u32>,
    casters: Vec<CasterId>,
    arrows: bool,
}

impl CasterKey {
    pub fn new(
        light: [[f32; 4]; 4],
        camera: Camera,
        viewport: [u32; 2],
        offsets: &[f32],
        casters: Vec<CasterId>,
        arrows: bool,
    ) -> Self {
        Self {
            light: light.map(|row| row.map(f32::to_bits)),
            camera,
            viewport,
            offsets: offsets.iter().map(|offset| offset.to_bits()).collect(),
            casters,
            arrows,
        }
    }
}

#[expect(clippy::large_enum_variant, reason = "one value per shadow pass, held in place in its cell and never moved in bulk")]
#[derive(Debug, Default, PartialEq)]
enum Held {
    #[default]
    Unknown,
    Clear,
    Drawn(CasterKey),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ready {
    Draw,
    Keep,
    Clear,
    Nothing,
}

impl Held {
    fn ready(&mut self, casters: Option<(CasterKey, bool)>) -> Ready {
        match casters {
            None if *self == Held::Clear => Ready::Nothing,
            None => {
                *self = Held::Clear;
                Ready::Clear
            }
            Some((key, true)) if matches!(self, Held::Drawn(held) if *held == key) => Ready::Keep,
            Some((key, _)) => {
                *self = Held::Drawn(key);
                Ready::Draw
            }
        }
    }
}

impl Buffers {
    fn new(device: &wgpu::Device, size: [u32; 2], screen: Screen) -> Self {
        let target = |label, format, extent, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let attachment =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let screen_extent = extent(screen.size(size));
        Self {
            depth_map: None,
            depth_map_depth: None,
            buffer: target("shadow buffer", BUFFER_FORMAT, screen_extent, attachment),
            buffer_depth: target(
                "shadow buffer (z)",
                super::shadow_blur::DEPTH_FORMAT,
                screen_extent,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            scratch: target(
                "shadow buffer (blur)",
                BUFFER_FORMAT,
                screen_extent,
                attachment,
            ),
            size,
            screen,
            content: Cell::new(Content::next()),
            cleared: Cell::new(false),
            depth_map_held: RefCell::default(),
        }
    }

    fn prepare_casters(&mut self, device: &wgpu::Device, [casters, receivers]: [bool; 2]) {
        let target = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent([DEPTH_MAP_SIZE; 2]),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            }).create_view(&Default::default())
        };
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if casters || receivers {
            self.depth_map.get_or_insert_with(|| target(
                "shadow depth map", DEPTH_MAP_FORMAT, attachment | wgpu::TextureUsages::TEXTURE_BINDING,
            ));
        }
        if casters {
            self.depth_map_depth.get_or_insert_with(|| target(
                "shadow depth map (z)", DEPTH_FORMAT, attachment,
            ));
        }
    }

    fn depth_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<f32>,
    ) -> Option<wgpu::RenderPass<'a>> {
        let color = self.depth_map.as_ref()?;
        Some(encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow depth map"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match load {
                        wgpu::LoadOp::Clear(_) => wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        wgpu::LoadOp::Load => wgpu::LoadOp::Load,
                        wgpu::LoadOp::DontCare(value) => wgpu::LoadOp::DontCare(value),
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: self.depth_map_depth.as_ref().map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    fn buffer_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<f32>,
        query_set: Option<&'a wgpu::QuerySet>,
    ) -> wgpu::RenderPass<'a> {
        encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow buffer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.buffer,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match load {
                        wgpu::LoadOp::Clear(_) => wgpu::LoadOp::Clear(BUFFER_CLEAR),
                        wgpu::LoadOp::Load => wgpu::LoadOp::Load,
                        wgpu::LoadOp::DontCare(value) => wgpu::LoadOp::DontCare(value),
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.buffer_depth,
                depth_ops: Some(wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: query_set,
            multiview_mask: None,
        })
    }

    fn changed(&self) {
        self.content.set(Content::next());
        self.cleared.set(false);
    }

    fn clear_buffer(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.cleared.replace(true) {
            drop(self.buffer_pass(encoder, wgpu::LoadOp::Clear(1.0), None));
            self.content.set(Content::next());
        }
    }

    fn ready_depth_map(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        casters: Option<(CasterKey, bool)>,
    ) -> bool {
        if self.depth_map.is_none() {
            return false;
        }
        let ready = self.depth_map_held.borrow_mut().ready(casters);
        if matches!(ready, Ready::Draw | Ready::Clear) {
            drop(self.depth_pass(encoder, wgpu::LoadOp::Clear(1.0)));
        }
        let record = |name| viewer_runtime::view_record::add("map scene", name, 1);
        match ready {
            Ready::Draw => record("shadow_casters_drawn"),
            Ready::Keep => record("shadow_casters_kept"),
            Ready::Clear | Ready::Nothing => {}
        }
        ready == Ready::Draw
    }
}

#[derive(Clone, Copy)]
pub struct Prepared {
    pub texture_matrix: [[f32; 4]; 4],
    pub view_projection: [[f32; 4]; 4],
}

pub struct ShadowPass {
    clear_view: wgpu::TextureView,
    compiled_blur: super::shadow_blur::Blur,
    buffers: Option<Buffers>,
    drawn: Cell<bool>,
}

impl ShadowPass {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<shader_pipeline::library::Effect>,
    ) -> io::Result<Self> {
        if !device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
        {
            return Err(io::Error::other(
                "Map shader shadow receivers: 1 refused pass: native R32Float linear sampling requires FLOAT32_FILTERABLE",
            ));
        }
        let clear = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow buffer (clear value)"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BUFFER_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let clear_view = clear.create_view(&Default::default());
        Ok(Self {
            clear_view,
            compiled_blur: super::shadow_blur::Blur::new(device, effect)?,
            buffers: None,
            drawn: Cell::new(false),
        })
    }

    pub fn release(&mut self) {
        self.buffers = None;
    }

    pub fn sized(&self, size: [u32; 2], screen: Screen, [casters, receivers]: [bool; 2]) -> bool {
        self.buffers.as_ref().is_some_and(|b| {
            b.size == size && b.screen == screen
                && (!(casters || receivers) || b.depth_map.is_some())
                && (!casters || b.depth_map_depth.is_some())
        })
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: [u32; 2], screen: Screen, stages: [bool; 2]) {
        if self.buffers.as_ref().is_none_or(|b| b.size != size || b.screen != screen) {
            self.buffers = Some(Buffers::new(device, size, screen));
        }
        self.buffers.as_mut().expect("shadow screen targets").prepare_casters(device, stages);
    }

    pub fn screen(&self) -> Option<Screen> {
        self.buffers.as_ref().map(|buffers| buffers.screen)
    }

    pub fn texel_reads(&self) -> impl Iterator<Item = &str> {
        self.compiled_blur.texel_reads()
    }

    pub fn abandon(&self) {
        if let Some(buffers) = &self.buffers {
            buffers.changed();
            *buffers.depth_map_held.borrow_mut() = Held::Unknown;
        }
    }

    pub fn prepare(
        &self,
        frame: &FrameConstants,
        camera: Camera,
        viewport: [u32; 2],
    ) -> Option<Prepared> {
        self.buffers.as_ref()?;
        let direction = light_direction(camera, frame);
        let light = light_box(camera, viewport, direction)?;
        let matrix = light.texture_matrix();
        Some(Prepared {
            texture_matrix: matrix,
            view_projection: light.view_projection(),
        })
    }

    pub fn depth_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<f32>,
    ) -> Option<wgpu::RenderPass<'a>> {
        self.buffers.as_ref()?.depth_pass(encoder, load)
    }

    pub fn ready_depth_map(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        casters: Option<(CasterKey, bool)>,
    ) -> bool {
        self.buffers
            .as_ref()
            .is_some_and(|buffers| buffers.ready_depth_map(encoder, casters))
    }

    pub fn prepass_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<f32>,
        query: Option<(&'a wgpu::QuerySet, u32)>,
    ) -> Option<crate::shader_scene::SurfacePass<'a>> {
        let pass = self
            .buffers
            .as_ref()?
            .buffer_pass(encoder, load, query.map(|(set, _)| set));
        Some(crate::shader_scene::SurfacePass::new(
            pass,
            query.map(|(_, index)| index),
        ))
    }

    pub fn blur(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        height: f32,
        mark: &mut impl FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) -> io::Result<()> {
        let Some(buffers) = &self.buffers else {
            return Ok(());
        };
        for (axis, (source, target, label)) in [
            (&buffers.buffer, &buffers.scratch, "shadow_blur_horizontal"),
            (&buffers.scratch, &buffers.buffer, "shadow_blur_vertical"),
        ]
        .into_iter()
        .enumerate()
        {
            let bindings = self.compiled_blur.bindings(
                arena,
                axis,
                &super::shadow_blur::constants(buffers.size[axis], height),
                source,
            )?;
            let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: (axis == 1).then_some(
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &buffers.buffer_depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    },
                ),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.compiled_blur.render(&mut pass, axis, &bindings);
            drop(pass);
            mark(encoder, label);
        }
        self.drawn.set(true);
        buffers.changed();
        Ok(())
    }

    pub fn finish_receivers(&self) {
        self.drawn.set(true);
        if let Some(buffers) = &self.buffers {
            buffers.changed();
        }
    }

    pub fn clear_stand_in(&self, encoder: &mut wgpu::CommandEncoder) {
        self.drawn.set(false);
        if let Some(buffers) = &self.buffers {
            buffers.clear_buffer(encoder);
        }
        encoder
            .counted_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow buffer (clear value)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.clear_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BUFFER_CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
    }

    pub fn caster_texture(&self) -> Option<&wgpu::TextureView> {
        self.buffers.as_ref()?.depth_map.as_ref()
    }

    pub fn lit_texture(&self) -> &wgpu::TextureView {
        match (self.drawn.get(), &self.buffers) {
            (true, Some(buffers)) => &buffers.buffer,
            _ => &self.clear_view,
        }
    }

    pub fn refraction_sampled(&self) -> Sampled<'_> {
        let buffers = self.buffers.as_ref().expect("resized before rendering");
        Sampled {
            view: &buffers.buffer,
            content: Some(buffers.content.get()),
        }
    }

    pub fn drawn(&self) -> bool {
        self.drawn.get()
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    if length == 0.0 {
        return v;
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn subtract(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn distance2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = subtract(a, b);
    dot(d, d)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (row, values) in out.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = a[row][1] * b[1][column]
                + a[row][0] * b[0][column]
                + a[row][2] * b[2][column]
                + a[row][3] * b[3][column];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_gpu::camera::{MapDefines, MapSize};
    use viewer_gpu::frame::{DayNight, shadow_fade_factor};

    fn at(height: f32) -> Camera {
        Camera {
            center: [3010.0, 1568.0],
            height,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: viewer_gpu::camera::Pose::Map,
        }
    }

    use super::test_support::{VIEWPORT, reference};

    fn transform(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for (column, value) in out.iter_mut().enumerate() {
            *value = p[0] * m[0][column] + p[1] * m[1][column] + p[2] * m[2][column] + m[3][column];
        }
        out
    }

    #[test]
    fn the_light_projection_is_orthographic() {
        let (camera, frame) = reference();
        let light = light_box(camera, VIEWPORT, light_direction(camera, &frame))
            .expect("the reference camera sees the water plane");
        for point in [
            [2900.0, 0.0, 1500.0],
            [3120.0, 60.0, 1640.0],
            [3010.0, 25.5, 1568.0],
        ] {
            assert_eq!(transform(light.view_projection(), point)[3], 1.0);
            assert_eq!(transform(light.texture_matrix(), point)[3], 1.0);
        }
    }

    #[test]
    fn the_box_covers_the_visible_ground() {
        let (camera, frame) = reference();
        let light = light_box(camera, VIEWPORT, light_direction(camera, &frame)).expect("visible");
        let matrix = light.texture_matrix();
        let width = VIEWPORT[0] as f32;
        let height = VIEWPORT[1] as f32;
        for screen in [
            [0.0, 0.0],
            [width, 0.0],
            [0.0, height],
            [width, height],
            [width * 0.5, height * 0.5],
        ] {
            let ground = camera.unproject_ground(screen, VIEWPORT).expect("visible");
            let p = transform(matrix, [ground[0], WATER_HEIGHT, ground[1]]);
            assert!((0.0..=1.0).contains(&p[0]), "u {p:?} at {screen:?}");
            assert!((0.0..=1.0).contains(&p[1]), "v {p:?} at {screen:?}");
            assert!((0.0..=1.0).contains(&p[2]), "z {p:?} at {screen:?}");
        }
    }

    #[test]
    fn a_free_eye_looking_as_the_game_camera_gets_its_light() {
        use viewer_gpu::camera::{Free, Pose};
        let (map, frame) = reference();
        let eye = map.eye();
        let free = Camera {
            center: [eye[0], eye[2]],
            height: eye[1],
            pose: Pose::Free(Free::of_map(0.0, map.height, map.defines.tilt_lead)),
            ..map
        };
        let [game, own] = [map, free].map(|camera| footprint(camera, VIEWPORT).unwrap());
        for (game, own) in game.iter().zip(&own) {
            for axis in 0..3 {
                assert!((game[axis] - own[axis]).abs() < 1e-2, "{game:?} {own:?}");
            }
        }
        let [game_light, own_light] = [map, free].map(|camera| {
            let frame =
                FrameConstants::try_new(camera, VIEWPORT, DayNight::at_hour(60828490), 0.0, false)
                    .expect("valid camera");
            light_direction(camera, &frame)
        });
        for axis in 0..3 {
            assert!(
                (game_light[axis] - own_light[axis]).abs() < 1e-5,
                "{game_light:?} {own_light:?}"
            );
        }
        assert_eq!(game_light, light_direction(map, &frame));
        let [game, own] = [(map, game_light), (free, own_light)]
            .map(|(camera, light)| light_box(camera, VIEWPORT, light).unwrap());
        for axis in 0..3 {
            assert!(
                (game.min[axis] - own.min[axis]).abs() <= 10.0,
                "{game:?} {own:?}"
            );
            assert!(
                (game.max[axis] - own.max[axis]).abs() <= 10.0,
                "{game:?} {own:?}"
            );
        }
    }

    #[test]
    fn the_game_camera_s_light_anchor_is_never_capped() {
        let (reference, frame) = reference();
        let modded = MapDefines {
            tilt_lead: 100.0 * 0.3,
            heights: [10.0, reference.defines.heights[1]],
            ..reference.defines
        };
        let mut capped = false;
        for defines in [reference.defines, modded] {
            let [low, high] = defines.heights;
            for step in 0..=100 {
                let camera = Camera {
                    height: low + (high - low) * step as f32 / 100.0,
                    defines,
                    ..reference
                };
                let eye = camera.eye();
                let look = camera.look_direction();
                let reach = (eye[1] / look[1]).abs();
                capped |= reach > eye[1] * RAY_CAP;
                let ground = [eye[0] + reach * look[0], 0.0, eye[2] + reach * look[2]];
                let sun = frame.virtual_sun_pos;
                let moon = frame.virtual_moon_pos;
                let light = if distance2(eye, [moon[0], moon[1], moon[2]])
                    <= distance2(eye, [sun[0], sun[1], sun[2]])
                {
                    moon
                } else {
                    sun
                };
                let width = camera.map.x();
                let x = [light[0] - width, light[0], light[0] + width]
                    .into_iter()
                    .min_by(|a, b| (a - ground[0]).abs().total_cmp(&(b - ground[0]).abs()))
                    .unwrap();
                let game = normalize([ground[0] - x, -light[1], ground[2] - light[2]]);
                assert_eq!(
                    light_direction(camera, &frame),
                    game,
                    "at {}",
                    camera.height
                );
            }
        }
        assert!(
            capped,
            "no height reaches past a free eye's cap, so this proves nothing"
        );
    }

    #[test]
    fn a_free_eye_s_light_moves_continuously_through_the_horizon() {
        use viewer_gpu::camera::{Free, Lens, Pose};
        let (reference, frame) = reference();
        let at = |look_y: f32| {
            let camera = Camera {
                center: [3000.0, 1500.0],
                height: 30.0,
                pose: Pose::Free(Free::new(
                    0.0,
                    look_y.asin(),
                    Lens {
                        near: 1.0,
                        far: 20_000.0,
                        ..Lens::MAP
                    },
                )),
                ..reference
            };
            light_direction(camera, &frame)
        };
        let [steeper, shallower] = [at(-0.001001), at(-0.000999)];
        for axis in 0..3 {
            assert!(
                (steeper[axis] - shallower[axis]).abs() < 1e-5,
                "{steeper:?} {shallower:?}"
            );
        }
        let shadow = |light: [f32; 3]| {
            let run = 20.0 / light[1].abs();
            [light[0] * run, light[2] * run]
        };
        let [a, b] = [shadow(steeper), shadow(shallower)];
        assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-2, "{a:?} {b:?}");
        let level = at(0.0);
        for axis in 0..3 {
            assert!(
                (level[axis] - shallower[axis]).abs() < 1e-5,
                "{level:?} {shallower:?}"
            );
        }
    }

    #[test]
    fn a_free_eye_looking_at_the_sky_fits_its_capped_footprint() {
        use viewer_gpu::camera::{Free, Lens, Pose};
        let (reference, frame) = reference();
        let camera = Camera {
            center: [3000.0, 1500.0],
            height: 30.0,
            pose: Pose::Free(Free::new(
                0.3,
                0.4,
                Lens {
                    near: 1.0,
                    far: 20_000.0,
                    ..Lens::MAP
                },
            )),
            ..reference
        };
        let light = light_direction(camera, &frame);
        let fitted = light_box(camera, VIEWPORT, light).expect("a capped footprint");
        let matrix = fitted.texture_matrix();
        for point in footprint(camera, VIEWPORT).unwrap() {
            let reach = (point[0] - 3000.0).hypot(point[2] - 1500.0);
            assert!(reach <= 5.0 * 30.0 + 1e-2, "{point:?}");
            let p = transform(matrix, point);
            for value in p.into_iter().take(3) {
                assert!((0.0..=1.0).contains(&value), "{p:?}");
            }
        }
    }

    #[test]
    fn the_reference_hour_throws_shadows_north_and_west() {
        let (camera, frame) = reference();
        let light = light_direction(camera, &frame);
        assert!(light[1] < 0.0, "the light points downwards: {light:?}");
        assert!(light[2] > 0.5, "and mostly north: {light:?}");
        assert!(
            light[0] < 0.0 && light[0] > -0.4,
            "and west of north: {light:?}"
        );
        let lean = light[0].hypot(light[2]) / -light[1];
        assert!((lean - 1.288).abs() < 0.01, "lean {lean}");
        let matrix = light_box(camera, VIEWPORT, light)
            .expect("visible")
            .texture_matrix();
        let top = [3010.0, WATER_HEIGHT + 20.0, 1568.0];
        let travel = 20.0 / -light[1];
        let ground = [
            top[0] + light[0] * travel,
            WATER_HEIGHT,
            top[2] + light[2] * travel,
        ];
        assert!(ground[0] < top[0], "west: {ground:?}");
        assert!(ground[2] > top[2], "north: {ground:?}");
        let (a, b) = (transform(matrix, top), transform(matrix, ground));
        assert!((a[0] - b[0]).abs() < 1e-4, "{a:?} {b:?}");
        assert!((a[1] - b[1]).abs() < 1e-4, "{a:?} {b:?}");
        assert!(b[2] > a[2] + SHADOW_BIAS, "{a:?} {b:?}");
    }

    const SHADOW_BIAS: f32 = 0.001;

    #[test]
    fn the_light_swings_with_the_hour() {
        let camera = Camera {
            center: [3009.83, 1568.00],
            height: 163.934,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: viewer_gpu::camera::Pose::Map,
        };
        let at = |hour| {
            light_direction(
                camera,
                &FrameConstants::try_new(camera, VIEWPORT, DayNight::at_hour(hour), 0.0, false)
                    .expect("valid reference camera"),
            )
        };
        let morning = at(60828486);
        let noon = at(60828490);
        let afternoon = at(60828494);
        assert!(morning[0] != noon[0] && noon[0] != afternoon[0]);
        assert!(
            morning[0] < noon[0] && noon[0] < afternoon[0],
            "{morning:?} {noon:?} {afternoon:?}"
        );
    }

    #[test]
    fn the_box_is_snapped_to_ten_units() {
        let (camera, frame) = reference();
        let light = light_box(camera, VIEWPORT, light_direction(camera, &frame)).expect("visible");
        for value in light.min.into_iter().chain(light.max) {
            assert_eq!(value % 10.0, 0.0, "{value}");
        }
        assert_eq!(snap10(37.9), 30.0);
        assert_eq!(snap10(-37.9), -30.0, "C division truncates toward zero");
        assert_eq!(snap10(0.0), 0.0);
    }

    #[test]
    fn the_shadow_footprint_uses_exact_ndc_centre_on_an_odd_viewport() {
        let camera = at(310.0);
        let viewport = [2049, 1153];
        let centre = water_plane_point(camera, viewport, [0.0, 0.0]);
        assert_eq!(centre[0].to_bits(), camera.center[0].to_bits());
        let picking = camera.pick_ray([1024.5, 576.5], viewport);
        assert!(picking[0] < 0.0, "integer-screen ray {picking:?}");
        assert_eq!(
            camera.pick_ray([0.0, 0.0], viewport).map(f32::to_bits),
            camera.ray_at_ndc([-1.0, 1.0], viewport).map(f32::to_bits)
        );
    }

    #[test]
    fn the_native_light_fit_preserves_axis_and_world_transform_order() {
        let corners = [
            [0x4589_25eb, 0x4118_0000, 0x44c8_2a7e],
            [0x4596_2059, 0x4118_0000, 0x44c8_2a7e],
            [0x4584_be1f, 0x4118_0000, 0x44d9_70ad],
            [0x459a_8825, 0x4118_0000, 0x44d9_70ad],
        ]
        .map(|point| point.map(f32::from_bits));
        let middle = [0x458f_a322, 0x4118_0000, 0x44d4_fbc1].map(f32::from_bits);
        let direction = [0x3ef5_8fb2, 0xbf32_c98f, 0xbf08_0001].map(f32::from_bits);
        let light = light_box_from_footprint(corners, middle, direction);
        assert_eq!(light.min, [-300.0, -190.0, 90.0]);
        assert_eq!(light.max, [230.0, 160.0, 450.0]);
        assert_eq!(
            light.eye.map(f32::to_bits),
            [0x458b_1c67, 0x4353_1aa3, 0x44e7_ab66]
        );
        assert_eq!(
            light.basis.map(|axis| axis.map(f32::to_bits)),
            [
                [0xbf3e_04c9, 0x0000_0000, 0xbf2b_8c6d],
                [0x3eef_9d6e, 0x3f37_395f, 0xbf04_b4f2],
                [0x3ef5_8fa7, 0xbf32_c992, 0xbf08_0001],
            ]
        );
        assert_eq!(
            light.view_projection().map(|row| row.map(f32::to_bits)),
            [
                [0xbb37_90b3, 0x3b2f_42e0, 0x3aae_9f0b, 0],
                [0, 0x3b86_03ee, 0xbafe_4680, 0],
                [0xbb25_b8ed, 0xbb42_2191, 0xbac1_6c18, 0],
                [0x418a_4ca1, 0xc0e6_2762, 0xc042_4f0d, 0x3f80_0000],
            ]
        );
        assert_eq!(
            light.texture_matrix().map(|row| row.map(f32::to_bits)),
            [
                [0xbab7_90b3, 0xbaaf_42e0, 0x3aae_9f0b, 0],
                [0, 0xbb06_03ee, 0xbafe_4680, 0],
                [0xbaa5_b8ed, 0x3ac2_2191, 0xbac1_6c18, 0],
                [0x4112_4ca1, 0x4083_13b1, 0xc042_4f0d, 0x3f80_0000],
            ]
        );
    }

    #[test]
    fn the_native_normalizer_and_snap_keep_their_exceptional_branches() {
        assert_eq!(
            normalize([f32::from_bits(0x3080_0000), 0.0, 0.0]),
            [1.0, 0.0, 0.0]
        );
        let zero = [0.0, -0.0, 0.0];
        assert_eq!(normalize(zero).map(f32::to_bits), zero.map(f32::to_bits));
        assert!(normalize([f32::NAN, 1.0, 0.0]).into_iter().all(f32::is_nan));
        assert!(native_min(1.0, f32::NAN).is_nan());
        assert!(native_max(1.0, f32::NAN).is_nan());
        assert_eq!(native_min(f32::NAN, 1.0), 1.0);
        assert_eq!(native_max(f32::NAN, 1.0), 1.0);
        assert_eq!(native_min(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(native_max(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 2_147_483_648.0] {
            assert_eq!(snap10(value).to_bits(), 0xcf00_0000);
        }
    }

    #[test]
    fn casters_stop_at_the_shadow_cutoff() {
        assert!(draws_shadows(&at(50.178)));
        assert!(draws_shadows(&at(302.960)));
        assert!(draws_shadows(&at(399.9)));
        assert!(!draws_shadows(&at(400.0)));
        assert!(!draws_shadows(&at(627.404)));
        for height in [50.178, 302.960, 399.9] {
            assert!(shadow_fade_factor(&MapDefines::test(), height) > 0.0);
        }
        for height in [400.0, 627.404, 1589.602] {
            assert_eq!(shadow_fade_factor(&MapDefines::test(), height), 0.0);
        }
    }

    #[test]
    fn fog_receivers_keep_their_own_cutoff_with_shadows_disabled() {
        let mut camera = at(150.0);
        camera.defines.shadows[0] = 100.0;
        camera.defines.fog[0] = 200.0;
        assert!(!draws_shadows(&camera));
        assert_eq!(stages(&camera, true), [false, true]);
        assert_eq!(stages(&camera, false), [false, true]);
        camera.height = 200.0;
        assert_eq!(stages(&camera, true), [false, false]);
        assert_eq!(stages(&camera, false), [false, false]);
        camera.height = 50.0;
        camera.defines.fog[0] = 25.0;
        assert_eq!(stages(&camera, true), [true, true]);
        assert_eq!(stages(&camera, false), [false, false]);
    }

    #[test]
    fn the_blur_weights_sum_to_one() {
        let constants = super::super::shadow_blur::constants(100, 0.0);
        let sum = constants[3] + 2.0 * (constants[4] + constants[5]);
        assert!((sum - 1.0).abs() < 1e-6, "{sum}");
    }

    #[test]
    fn the_screen_buffer_is_half_the_view_rounded_up_unless_full() {
        assert_eq!(Screen::Half.size([1920, 1080]), [960, 540]);
        assert_eq!(Screen::Half.size([2049, 1153]), [1025, 577]);
        assert_eq!(Screen::Half.size([1, 1]), [1, 1]);
        assert_eq!(Screen::Full.size([2049, 1153]), [2049, 1153]);
    }

    #[test]
    fn the_screen_targets_follow_the_view_at_the_screen_size() {
        let (_serial, device, _queue) = viewer_gpu::test_support::gpu::device();
        let side = |view: &wgpu::TextureView| {
            let size = view.texture().size();
            [size.width, size.height]
        };
        for (view, screen, made) in [
            ([7, 5], Screen::Half, [4, 3]),
            ([8, 6], Screen::Half, [4, 3]),
            ([1, 1], Screen::Half, [1, 1]),
            ([7, 5], Screen::Full, [7, 5]),
        ] {
            let mut buffers = Buffers::new(&device, view, screen);
            assert_eq!(buffers.size, view);
            for target in [&buffers.buffer, &buffers.buffer_depth, &buffers.scratch] {
                assert_eq!(side(target), made, "{view:?} {screen:?}");
            }
            assert!(buffers.depth_map.is_none());
            assert!(buffers.depth_map_depth.is_none());
            buffers.prepare_casters(&device, [false, true]);
            let color = buffers.depth_map.clone().unwrap();
            assert_eq!(side(&color), [DEPTH_MAP_SIZE; 2]);
            assert!(buffers.depth_map_depth.is_none());
            buffers.prepare_casters(&device, [true, true]);
            assert_eq!(side(buffers.depth_map_depth.as_ref().unwrap()), [DEPTH_MAP_SIZE; 2]);
            assert_eq!(buffers.depth_map.as_ref(), Some(&color));
        }
    }

    #[test]
    fn a_clear_screen_buffer_is_not_cleared_again_while_it_stays_clear() {
        let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
        let buffers = Buffers::new(&device, [7, 5], Screen::Half);
        let submit = |record: &dyn Fn(&mut wgpu::CommandEncoder)| {
            let mut encoder = device.create_command_encoder(&Default::default());
            record(&mut encoder);
            queue.submit([encoder.finish()]);
        };
        let paint = || {
            submit(&|encoder: &mut wgpu::CommandEncoder| {
                drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &buffers.buffer,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                }));
            });
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                r#"
                @group(0) @binding(0) var buffer: texture_2d<f32>;
                @vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
                    let p = array<vec2f, 3>(vec2f(-1, -1), vec2f(3, -1), vec2f(-1, 3));
                    return vec4f(p[i], 0, 1);
                }
                @fragment fn pixel() -> @location(0) vec4f {
                    return textureLoad(buffer, vec2i(1, 2), 0);
                }
            "#
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("pixel"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&buffers.buffer),
            }],
        });
        let texel = || -> [u8; 4] {
            let pixels =
                viewer_gpu::test_support::gpu::render_with(&device, &queue, |pass| {
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.draw(0..3, 0..1);
                });
            pixels[..4].try_into().unwrap()
        };
        const CLEAR: [u8; 4] = [255, 0, 0, 255];
        const PAINTED: [u8; 4] = [0, 255, 0, 255];

        submit(&|encoder: &mut wgpu::CommandEncoder| buffers.clear_buffer(encoder));
        assert_eq!(texel(), CLEAR);
        let content = buffers.content.get();
        paint();
        submit(&|encoder: &mut wgpu::CommandEncoder| buffers.clear_buffer(encoder));
        assert_eq!(texel(), PAINTED, "a clear buffer was cleared again");
        assert_eq!(buffers.content.get(), content);

        buffers.changed();
        submit(&|encoder: &mut wgpu::CommandEncoder| buffers.clear_buffer(encoder));
        assert_eq!(texel(), CLEAR);
        assert_ne!(buffers.content.get(), content);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
