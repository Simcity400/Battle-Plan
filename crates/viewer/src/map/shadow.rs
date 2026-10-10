//! The cast-shadow depth map, screen-space receivers and two blur passes.
//!
//! Depth and screen targets are prepared and cleared every frame. Casters and
//! blurs require enabled shadows and a camera below the shadow cutoff.
//! Receivers also run below the fog cutoff without casters.
//!
//! The orthographic depth target is 2_FLOAT with depth/stencil and a clear
//! value of one. Each world-wrap view submits mesh casters, tree casters and
//! admitted late arrows in that order. Terrain and the map frame are excluded.
//! The light matrix replaces the scene view-projection for these draws.
//!
//! Tree and terrain receivers draw their ordinary geometry into a
//! backbuffer-sized BGRA8 UNORM target with depth/stencil. Its clear colour is
//! fully lit with zero fog. Horizontal and vertical blur passes filter the
//! shadow channel and preserve the centre sample's fog factors. Blur offsets
//! scale with camera height.
//!
//! One ShadowMap binding carries either depth or screen-space data as required
//! by the current pass. The depth texture retains authored linear sampling;
//! unsupported FLOAT32_FILTERABLE capability is a named refusal. Caster draws
//! are also culled against [`caster_rect`].
use std::io;

use eframe::wgpu;

use super::camera::{Camera, WATER_HEIGHT};
use super::frame::FrameConstants;
use super::scene::extent;

/// Depth-map side length required by the receiver's fixed texel span.
pub const DEPTH_MAP_SIZE: u32 = 2048;

/// Single-channel 32-bit floating-point depth target.
pub const DEPTH_MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// The screen-space buffer's format. The engine's is a backbuffer-sized RGBA
/// target, and the three values it carries are all in the unit interval.
pub const BUFFER_FORMAT: wgpu::TextureFormat = super::shadow_blur::COLOR_FORMAT;

/// the screen-space buffer's clear value: fully lit, no fog.
pub const BUFFER_CLEAR: wgpu::Color = wgpu::Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// World units of caster above the water plane the caster rectangle allows for.
///
/// The heightfield reaches 25.5 (`map::terrain`), the tallest tree card about
/// 12 above its own ground and the tallest building mesh about 20, so 60 above
/// the water plane holds every caster with room to spare.
const CASTER_CEILING: f32 = 60.0;

/// The engine's padding of the light box, in world units, on all six sides.
const LIGHT_BOX_PAD: f32 = 10.0;
/// Extra distance behind the light's box, so a caster standing above the ground
/// plane the box is fitted to is still in front of the near plane.
const LIGHT_BOX_BACKOFF: f32 = 30.0;

/// The light's orthographic box, in light space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightBox {
    /// `(left, bottom, near)` of the off-centre orthographic box.
    pub min: [f32; 3],
    /// `(right, top, far)` of the same box.
    pub max: [f32; 3],
    /// The light-space basis as world rows: right, up, forward.
    pub basis: [[f32; 3]; 3],
    /// The light-space view's eye position.
    pub eye: [f32; 3],
}

/// The direction the shadow light travels in.
///
/// The light direction uses fixed-point seasonal rotation and the current hour.
/// Its virtual position derives from map dimensions and configured latitude;
/// normalization preserves the component evaluation order.
pub fn light_direction(camera: Camera, frame: &FrameConstants) -> [f32; 3] {
    let eye = camera.eye();
    let look = camera.look_direction();
    let t = (eye[1] / look[1]).abs();
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
    // The map wraps in x, so the light is taken at whichever of its three
    // copies is nearest the ground point.
    let width = camera.map.x();
    let x = [light[0] - width, light[0], light[0] + width]
        .into_iter()
        .min_by(|a, b| (a - ground[0]).abs().total_cmp(&(b - ground[0]).abs()))
        .unwrap_or(light[0]);
    normalize([ground[0] - x, -light[1], ground[2] - light[2]])
}

/// The visible ground rectangle grown by the horizontal reach of the tallest
/// caster, so a building standing just outside the frame still casts
/// into it. Every caster pass culls against this rather than against the
/// camera's own rectangle.
///
/// Caster culling expands the camera rectangle to retain off-screen casters whose shadows reach the
/// frame. Upstream queue culling remains unverified.
pub fn caster_rect(visible: [f32; 4], light: [f32; 3]) -> [f32; 4] {
    let lean = CASTER_CEILING * (light[0].hypot(light[2]) / (-light[1]).max(1e-3));
    [
        visible[0] - lean,
        visible[1] - lean,
        visible[2] + lean,
        visible[3] + lean,
    ]
}

/// Fit an orthographic light view to the camera footprint's centre and four water-plane corners.
/// Preserve capped f32 corner rays and axis order. The fit axis zeroes its vertical component
/// before normalization; the eye uses the independently normalized full light direction. Form
/// all three raw target-eye axes before normalizing them. Transform corners with the rounded
/// view translation before projection and texel snapping; subtracting the eye first can cross
/// a snap boundary.
///
/// `None` when the camera footprint is nonfinite.
pub fn light_box(camera: Camera, viewport: [u32; 2], light: [f32; 3]) -> Option<LightBox> {
    let points = [
        [-1.0, 1.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [1.0, -1.0],
        [0.0, 0.0],
    ]
    .map(|ndc| water_plane_point(camera, viewport, ndc));
    if points.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let [a, b, c, d, middle] = points;
    let corners = [a, b, c, d];
    Some(light_box_from_footprint(corners, middle, light))
}

fn water_plane_point(camera: Camera, viewport: [u32; 2], ndc: [f32; 2]) -> [f32; 3] {
    let eye = camera.eye();
    let ray = camera.ray_at_ndc(ndc, viewport);
    let cap = eye[1] * 5.0;
    let distance = if ray[1] < -0.001 {
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
    // MINSS/MAXSS keep the second operand on unordered or equal inputs.
    // X fitting starts at a different corner from Y/Z.
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

/// Truncate to a signed integer and snap to a ten-unit multiple with division toward zero.
/// Negative values therefore snap upward.
fn snap10(value: f32) -> f32 {
    // NaN and out-of-range conversion produce i32::MIN; a saturating cast differs.
    let integer = if (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    };
    (integer / 10 * 10) as f32
}

impl LightBox {
    /// The light's view-projection combines a left-handed view with an off-centre orthographic
    /// projection. Depth spans `[0, 1]`, using the same row-vector `[row][column] convention as
    /// `Camera::view_projection`.
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

    /// `ShadowMapTextureMatrix`: the view-projection with texture bias folded in, mapping world
    /// positions directly to `(u, v, depth, 1)`. Flip `v` from upward clip space to downward texture
    /// space. Keep projected depth in `[0, 1]` for comparison with the stored depth.
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

/// Whether the engine admits casters at this camera height.
///
/// Caster admission requires a camera strictly below DRAW_SHADOWS_CUTOFF. At or above it, shadow
/// fading produces a fully lit term.
pub fn draws_shadows(camera: &Camera) -> bool {
    camera.height < camera.defines.shadows[0]
}

/// Caster and receiver camera gates. Blurs share the caster gate; receivers also run below
/// DRAW_FOW_CUTOFF with draw_shadows disabled.
pub fn stages(camera: &Camera, shadows_enabled: bool) -> [bool; 2] {
    let casters = shadows_enabled && draws_shadows(camera);
    [casters, casters || camera.height < camera.defines.fog[0]]
}

/// The per-size resources: the depth map, the screen-space buffer and the
/// buffer the horizontal blur writes on its way to the vertical one.
struct Buffers {
    depth_map: wgpu::TextureView,
    depth_map_depth: wgpu::TextureView,
    buffer: wgpu::TextureView,
    buffer_depth: wgpu::TextureView,
    scratch: wgpu::TextureView,
    size: [u32; 2],
}

/// Matrices prepared once for native casters and screen-space receivers.
#[derive(Clone, Copy)]
pub struct Prepared {
    pub texture_matrix: [[f32; 4]; 4],
    pub view_projection: [[f32; 4]; 4],
}

/// The depth pass, the pre-pass target and the two blurs.
pub struct ShadowPass {
    /// A one-texel stand-in holding the buffer's clear value, bound wherever
    /// the real buffer is not wanted: when no receivers run, and in the depth
    /// pass itself, which samples nothing.
    clear_view: wgpu::TextureView,
    compiled_blur: super::shadow_blur::Blur,
    buffers: Option<Buffers>,
    /// Whether the last frame ran the chain, for reporting.
    drawn: std::cell::Cell<bool>,
}

impl ShadowPass {
    pub fn new(
        device: &wgpu::Device,
        effect: &std::sync::Arc<shader_pipeline::library::Effect>,
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
        // The one texel is written by clearing it, which needs no queue write
        // path for a format wgpu may not let us `write_texture` into.
        Ok(Self {
            clear_view,
            compiled_blur: super::shadow_blur::Blur::new(device, effect)?,
            buffers: None,
            drawn: std::cell::Cell::new(false),
        })
    }

    /// Rebuilds the screen-sized targets.
    pub fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.buffers.as_ref().is_some_and(|b| b.size == size) {
            return;
        }
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
        let square = wgpu::Extent3d {
            width: DEPTH_MAP_SIZE,
            height: DEPTH_MAP_SIZE,
            depth_or_array_layers: 1,
        };
        let buffers = Buffers {
            depth_map: target("shadow depth map", DEPTH_MAP_FORMAT, square, attachment),
            depth_map_depth: target(
                "shadow depth map (z)",
                super::scene::DEPTH_FORMAT,
                square,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            buffer: target("shadow buffer", BUFFER_FORMAT, extent(size), attachment),
            buffer_depth: target(
                "shadow buffer (z)",
                super::shadow_blur::DEPTH_FORMAT,
                extent(size),
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            scratch: target(
                "shadow buffer (blur)",
                BUFFER_FORMAT,
                extent(size),
                attachment,
            ),
            size,
        };
        self.buffers = Some(buffers);
    }

    /// Computes the light matrices for native casters and receivers.
    ///
    /// calls BeginRender even with no casters or receivers.
    /// Returns `None` only when resources or finite light matrices are absent.
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

    /// The depth pass the casters draw into, already cleared to the "nothing
    /// here" depth of 1.
    pub fn depth_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<f32>,
    ) -> Option<wgpu::RenderPass<'a>> {
        let buffers = self.buffers.as_ref()?;
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow depth map"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &buffers.depth_map,
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
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &buffers.depth_map_depth,
                depth_ops: Some(wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                }),
                // Clear colour/depth (flags3), preserving stencil.
                // Arrow mode2 may read/write it through its authored state.
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

    /// The screen-space pre-pass the `terrainunlit` and `treeunlit` draws fill.
    /// Bind `ShadowPass::prepass` as group 0.
    pub fn prepass_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<f32>,
    ) -> Option<wgpu::RenderPass<'a>> {
        let buffers = self.buffers.as_ref()?;
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow buffer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &buffers.buffer,
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
            // The two unlit effects keep their lit twins' depth state, so the
            // pre-pass needs a depth buffer of its own: the grid is drawn from
            // the near row outwards and a far ridge would otherwise paint over
            // the near one that hides it.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &buffers.buffer_depth,
                depth_ops: Some(wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    /// The two native `shadowblur.shader` passes, buffer to scratch and back.
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
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
                // Clear colour only. The vertical native quad writes z0
                // against the receiver's retained LESS_EQUAL/write depth state.
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
        Ok(())
    }

    /// Receiver-only frames still bind the full fog target.
    pub fn finish_receivers(&self) {
        self.drawn.set(true);
    }

    /// Clears the one-texel stand-in to the buffer's own clear value, which is
    /// what a frame with neither casters nor fog receivers sees.
    pub fn clear_stand_in(&self, encoder: &mut wgpu::CommandEncoder) {
        self.drawn.set(false);
        // Clear the screen target even when receiver draws are skipped.
        // Next frame's underwater pass still binds that full-sized target.
        if let Some(pass) = self.prepass_pass(encoder, wgpu::LoadOp::Clear(1.0)) {
            drop(pass);
        }
        encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
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
        self.buffers.as_ref().map(|buffers| &buffers.depth_map)
    }

    pub fn lit_texture(&self) -> &wgpu::TextureView {
        match (self.drawn.get(), &self.buffers) {
            (true, Some(buffers)) => &buffers.buffer,
            _ => &self.clear_view,
        }
    }

    /// Pre-frame binds the retained shadow-screen target regardless of the current cutoff.
    /// Native allocation supplies no initial data. wgpu's first-frame
    /// zero initialization is not a native colour claim; dumps warm up first.
    pub fn refraction_texture(&self) -> &wgpu::TextureView {
        &self
            .buffers
            .as_ref()
            .expect("resized before rendering")
            .buffer
    }

    /// Whether the last frame ran the chain, for reporting.
    pub fn drawn(&self) -> bool {
        self.drawn.get()
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    // Skip only an exactly zero length. NaNs divide, and small finite vectors normalize.
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
    use crate::map::camera::{MapDefines, MapSize};
    use crate::map::frame::{DayNight, shadow_fade_factor};

    fn at(height: f32) -> Camera {
        Camera {
            center: [3010.0, 1568.0],
            height,
            map: MapSize::TEST,
            defines: MapDefines::test(),
        }
    }

    const VIEWPORT: [u32; 2] = [2048, 1152];

    /// Reference fixture at hour 60828490, 11:00 on 22 November 1943.
    fn reference() -> (Camera, FrameConstants) {
        let camera = Camera {
            center: [3009.83, 1568.00],
            height: 163.934,
            map: crate::map::camera::MapSize::TEST,
            defines: crate::map::camera::MapDefines::test(),
        };
        let frame =
            FrameConstants::try_new(camera, VIEWPORT, DayNight::at_hour(60828490), 0.0, false)
                .expect("valid reference camera");
        (camera, frame)
    }

    fn transform(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for (column, value) in out.iter_mut().enumerate() {
            *value = p[0] * m[0][column] + p[1] * m[1][column] + p[2] * m[2][column] + m[3][column];
        }
        out
    }

    /// An orthographic light keeps clip w equal to one throughout the footprint.
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

    /// The box is fitted to the four screen corners on the water plane, so
    /// every one of them lands inside the unit square with a depth in `[0, 1]`,
    /// and so does the ground under the screen centre.
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

    /// At the reference hour the virtual sun is south of the camera and east of
    /// it, so the light travels north-north-west and the shadows lie that way.
    ///
    /// The numbers: the hour is 60828490, whose `hour mod 24` is 10, so
    /// `vVirtualSunPos` is `((1 - 10/24) * MAP_SIZE_X, 600, 848)` =
    /// `(3285.3, 600, 848)`, and the ground point under the camera is
    /// `(3009.8, 0, 1569.8)`. The difference normalises to
    /// `(-0.282, -0.613, 0.738)`: a caster leans 1.29 times its own height, in
    /// a direction 21 degrees west of north.
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
        // A caster and the ground point its shadow falls on share a texel in
        // the depth map, and the ground point is behind the caster in depth.
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

    /// Depth-comparison bias used to separate the caster and receiver in the test above.
    const SHADOW_BIAS: f32 = 0.001;

    /// The light swings with the hour, because it is the virtual sun and not
    /// the `LIGHT_SHADOW_DIRECTION` define.
    #[test]
    fn the_light_swings_with_the_hour() {
        let camera = Camera {
            center: [3009.83, 1568.00],
            height: 163.934,
            map: crate::map::camera::MapSize::TEST,
            defines: crate::map::camera::MapDefines::test(),
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
        // The sun runs from east to west over the day, so the shadow's
        // horizontal component runs the other way.
        assert!(
            morning[0] < noon[0] && noon[0] < afternoon[0],
            "{morning:?} {noon:?} {afternoon:?}"
        );
    }

    /// The centre and the box are snapped to the engine's ten-unit grid, which
    /// is what keeps the shadows from crawling as the camera pans.
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
        // The native picking path still rounds its screen coordinate first.
        // Reusing it directly would move the shadow centre left on odd sizes.
        let picking = camera.pick_ray([1024.5, 576.5], viewport);
        assert!(picking[0] < 0.0, "integer-screen ray {picking:?}");
        assert_eq!(
            camera.pick_ray([0.0, 0.0], viewport).map(f32::to_bits),
            camera.ray_at_ndc([-1.0, 1.0], viewport).map(f32::to_bits)
        );
    }

    /// This binary32 fixture lies on a snap boundary. Subtracting the eye first or forming axes from
    /// normalized light shifts x from -300 to -290. Fitting with the full light preserves z at 20..380.
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
        // CVTTSS2SI's indefinite word, divided by ten and multiplied back,
        // rounds to -2^31 on conversion to f32.
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 2_147_483_648.0] {
            assert_eq!(snap10(value).to_bits(), 0xcf00_0000);
        }
    }

    /// The caster gate is the same gate as a nonzero shadow fade factor.
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

    /// The blur is a partition of unity, so a uniformly lit buffer survives it.
    #[test]
    fn the_blur_weights_sum_to_one() {
        let constants = super::super::shadow_blur::constants(100, 0.0);
        let sum = constants[3] + 2.0 * (constants[4] + constants[5]);
        assert!((sum - 1.0).abs() < 1e-6, "{sum}");
    }

    /// The caster rectangle reaches past the frame by the lean of the tallest
    /// caster, and by nothing at all when the light is straight overhead.
    #[test]
    fn the_caster_rectangle_grows_by_the_lean() {
        let visible = [2900.0, 1500.0, 3120.0, 1640.0];
        assert_eq!(caster_rect(visible, [0.0, -1.0, 0.0]), visible);
        let leaning = caster_rect(visible, normalize([0.0, -1.0, 1.0]));
        assert!(
            (leaning[1] - (visible[1] - CASTER_CEILING)).abs() < 1e-3,
            "{leaning:?}"
        );
        assert!(
            (leaning[3] - (visible[3] + CASTER_CEILING)).abs() < 1e-3,
            "{leaning:?}"
        );
    }
}
