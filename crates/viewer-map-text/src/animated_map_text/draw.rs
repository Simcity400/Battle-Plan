
use std::{
    collections::BTreeMap,
    io,
    ops::Range,
    sync::{Arc, Mutex, OnceLock},
};

use eframe::wgpu;
use content_reader_gui::pdxshader::{DepthStencilState, program::Stage};
use shader_pipeline::{
    gpu,
    library::{Effect, Selection},
    stream::NativeSubmission,
    vertices::Declaration,
};

use viewer_scene_base::shader_scene;
use viewer_native_gui::native_text::{self, Font, Glyphs, IconSprite, Special};

const STRIDE: usize = 28;
const MAX_GLYPHS: usize = 512;
const ALLOCATION: u64 = (MAX_GLYPHS * 6 * STRIDE) as u64;

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Animated map text: refused draw: {}",
        reason.into()
    ))
}

#[derive(Clone, Copy, Debug)]
pub struct Animation {
    pub origin: [f32; 3],
    pub phase: f32,
    pub limit: f32,
    pub scale: f32,
    pub reach: f32,
    pub opacity: f32,
}

impl Animation {
    fn transform(self) -> ([f32; 3], f32, f32) {
        let quarter = self.limit * 0.25;
        let fade = if self.phase > self.limit - quarter {
            (self.limit - self.phase) / quarter + 0.0
        } else {
            1.0
        };
        let size = (self.limit - (self.limit * 0.5) * fade) * (self.scale * self.reach);
        let origin = [
            self.origin[0],
            self.phase * 3.0 * self.reach + self.origin[1],
            self.origin[2],
        ];
        (origin, size, fade)
    }
}

pub struct Camera {
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub view_projection: [f32; 16],
}

pub struct Text<'a> {
    pub font: &'a Font,
    pub bytes: &'a [u8],
    pub color: [f32; 4],
    pub global_colors: &'a BTreeMap<u8, [f32; 4]>,
    pub width: i32,
    pub animation: Animation,
}

#[derive(Debug, PartialEq, Eq)]
struct Group {
    texture: usize,
    vertices: Range<u32>,
}

#[derive(Default)]
pub struct Mesh {
    bytes: Vec<u8>,
    groups: Vec<Group>,
    has_text: bool,
    pub null_pages: Vec<Vec<u8>>,
}

impl Mesh {
    pub fn vertices(&self) -> u32 {
        (self.bytes.len() / STRIDE) as u32
    }
}

fn byte(component: f32) -> u8 {
    let scaled = component * 255.0;
    if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&scaled) {
        return 0;
    }
    (scaled as i64 & 255) as u8
}

fn color_word(color: [f32; 4]) -> u32 {
    let [r, g, b, a] = color.map(byte);
    u32::from_le_bytes([b, g, r, a])
}

fn world(origin: [f32; 3], camera: &Camera, pixel: [i32; 2], size: f32) -> [f32; 3] {
    let x = pixel[0] as f32 * 0.05 * size;
    let y = pixel[1] as f32 * 0.05 * size;
    std::array::from_fn(|axis| origin[axis] + camera.right[axis] * x + camera.up[axis] * y)
}

fn vertex(bytes: &mut Vec<u8>, position: [f32; 3], uv: [f32; 2], color: u32, border: u32) {
    for value in position.into_iter().chain(uv) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&color.to_le_bytes());
    bytes.extend_from_slice(&border.to_le_bytes());
}

pub fn geometry(
    text: &Text<'_>,
    camera: &Camera,
    sprite: &dyn Fn(&[u8]) -> Option<IconSprite>,
) -> io::Result<Mesh> {
    let mut mesh = Mesh {
        has_text: !text.bytes.is_empty(),
        ..Default::default()
    };
    if !mesh.has_text {
        return Ok(mesh);
    }
    if !text.font.refusals.is_empty() {
        return Err(refuse(
            text.font
                .refusals
                .iter()
                .map(|(_, reason)| reason.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let Some(decoded) = native_text::decode(text.bytes) else {
        return Ok(mesh);
    };
    let laid = native_text::lay_out(
        text.font,
        sprite,
        &decoded,
        native_text::Limits {
            width: text.width,
            height: i32::MAX,
            border: [0; 2],
            newlines: true,
            preserve_indent: false,
            ellipsis: false,
            multiline: true,
            special: true,
            icons_add_height: text.font.facts.icons_add_height,
        },
    )
    .map_err(refuse)?;
    let start = -(laid.widths.iter().copied().max().unwrap_or(0).max(0) / 2);
    let (origin, size, fade) = text.animation.transform();
    let alpha = u32::from(byte(fade * text.animation.opacity)) << 24;
    let own = color_word(text.color);
    let mut color = own & 0x00ff_ffff | alpha;
    let colors = native_text::Colors {
        own: text.color,
        font: &text.font.facts.text_colors,
        global: text.global_colors,
    };
    let mut stack: Vec<u32> = Vec::new();
    let mut pen = start;
    let mut baseline: i32 = 15;
    let mut previous = None;
    let mut at = 0;
    while at < laid.text.len() {
        let ch = laid.text[at];
        if let Some((mut length, sequence)) = native_text::special(&laid.text[at..]) {
            let changed = match sequence {
                Special::Color(Some('!')) if stack.len() > 1 => {
                    stack.pop();
                    stack.last().copied()
                }
                Special::Color(Some(code)) => {
                    let found = if (u32::from(code) & 255) == u32::from(b'!') {
                        Some(own)
                    } else {
                        colors.lookup(code).map(color_word)
                    };
                    if let Some(found) = found {
                        stack.push(found);
                    }
                    found
                }
                Special::Icon(name) => {
                    if !name.is_empty()
                        && let Some(icon) = native_text::find_icon(&name, sprite)
                    {
                        pen = pen.wrapping_add(icon.advance().map_err(refuse)?);
                    }
                    None
                }
                Special::Currency => {
                    length = 1 + (laid.text.len() - at - 1).min(2);
                    None
                }
                Special::Color(None) | Special::Flag(_) | Special::Number(_) => None,
            };
            if let Some(changed) = changed.filter(|word| *word != 0) {
                color = changed & 0x00ff_ffff | alpha;
            }
            previous = None;
            at += length;
            continue;
        }
        if let Some(glyph) = text.font.glyph(ch) {
            pen = pen.wrapping_add(previous.map_or(0, |previous| text.font.kerning(previous, ch)));
            let left = pen.wrapping_add(glyph.xoffset);
            let right = left.wrapping_add(glyph.width);
            let top = baseline.wrapping_sub(glyph.yoffset);
            let bottom = top.wrapping_sub(glyph.height);
            pen = pen.wrapping_add(glyph.advance);
            previous = Some(glyph.ch);
            if glyph.height > 0 {
                let quad = text
                    .font
                    .quad(glyph.ch)
                    .ok_or_else(|| refuse("resolved glyph has no family record"))?;
                if let Some(page) = quad.null_page
                    && !mesh.null_pages.contains(&page)
                {
                    mesh.null_pages.push(page);
                }
                let [u0, v0, u1, v1] = quad.uv;
                let first = mesh.vertices();
                for (pixel, uv) in [
                    ([left, bottom], [u0, v1]),
                    ([left, top], [u0, v0]),
                    ([right, bottom], [u1, v1]),
                    ([right, top], [u1, v0]),
                    ([right, bottom], [u1, v1]),
                    ([left, top], [u0, v0]),
                ] {
                    vertex(
                        &mut mesh.bytes,
                        world(origin, camera, pixel, size),
                        uv,
                        color,
                        text.font.facts.border_color,
                    );
                }
                if let Some(group) = mesh
                    .groups
                    .last_mut()
                    .filter(|group| group.texture == quad.page.texture)
                {
                    group.vertices.end += 6;
                } else {
                    mesh.groups.push(Group {
                        texture: quad.page.texture,
                        vertices: first..first + 6,
                    });
                }
                if mesh.vertices() as usize == MAX_GLYPHS * 6 {
                    break;
                }
            }
        } else {
            if matches!(ch, '\n' | '\r') {
                baseline = baseline.wrapping_sub(text.font.line_height());
                pen = start;
            }
            previous = None;
        }
        at += 1;
    }
    Ok(mesh)
}

pub type Texture = Result<Option<gpu::TextureBinding>, String>;

pub struct Renderer {
    effect: Arc<Effect>,
    template: gpu::Pass,
    compiled: OnceLock<Result<Option<Selection>, String>>,
}

pub struct Prepared {
    groups: Vec<(NativeSubmission, Range<u32>)>,
}

impl Renderer {
    pub fn new(
        effect: Arc<Effect>,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> Self {
        Self {
            effect,
            template: gpu::Pass {
                vertices: Vec::new(),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(
                    gpu::depth_stencil(&disabled_depth(), &Default::default(), depth).0,
                ),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                targets: vec![Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                instanced: 0,
                stream: None,
                vertex_color_tint: None,
            },
            compiled: OnceLock::new(),
        }
    }

    pub fn warm(&self) {
        let _ = self.selected();
    }

    fn selected(&self) -> &Result<Option<Selection>, String> {
        self.compiled.get_or_init(|| {
            shader_pipeline::library::with_recorded(&self.effect, |effect| {
                effect.select("Text3D").map_err(std::io::Error::other)
            })
            .map_err(|error| error.to_string())
        })
    }

    pub fn prepare(
        &self,
        arena: &gpu::Arena,
        retained: &mut shader_scene::State,
        mesh: &Mesh,
        camera: &Camera,
        auxiliary_color: [f32; 4],
        texture: &dyn Fn(usize) -> Texture,
    ) -> io::Result<Prepared> {
        bind_depth(retained);
        let mut prepared = Prepared { groups: Vec::new() };
        if !mesh.has_text {
            return Ok(prepared);
        }
        let selected = self.selected();
        let mut first_refusal = None;
        match selected {
            Ok(selection) => {
                if let Err(reason) = retained
                    .stream
                    .bind_effect(selection.clone(), Declaration::Text)
                {
                    first_refusal = Some(refuse(reason));
                }
            }
            Err(reason) => {
                retained.stream.forget_program();
                first_refusal = Some(refuse(reason.clone()));
            }
        }
        let constants = constants(camera, auxiliary_color);
        retained
            .stream
            .bind_constants(0, Stage::Vertex, Some(&constants));
        if mesh.groups.is_empty() {
            if let Some(error) = first_refusal {
                return Err(error);
            }
            return Ok(prepared);
        }
        retained.bind_partial_vertex(
            0,
            STRIDE as u64,
            Arc::new(Mutex::new(shader_scene::Partial {
                bytes: mesh.bytes.clone(),
                size: ALLOCATION,
                known: std::iter::once(0..mesh.bytes.len() as u64).collect(),
            })),
            0,
        );
        let effect = match selected {
            Ok(Some(selection)) => &selection.effect,
            _ => &self.effect,
        };
        let sampler = effect.source.technique("Text3D").and_then(|_| {
            effect
                .source
                .common
                .samplers
                .iter()
                .chain(&effect.source.pixel.samplers)
                .rfind(|sampler| sampler.index == 0)
        });
        let native_layout = retained.stream.vertex_layouts().is_ok();
        let mut uploaded: Option<Vec<gpu::Slice>> = None;
        let program_refused = first_refusal.is_some();
        for group in &mesh.groups {
            let view = texture(group.texture);
            match &view {
                Ok(view) => {
                    retained
                        .stream
                        .bind_formatted_texture(0, Stage::Pixel, view.clone(), sampler)
                }
                Err(_) => retained.stream.forget_texture(0, Stage::Pixel, sampler),
            }
            if program_refused {
                continue;
            }
            let submission = if native_layout {
                retained.prepare(
                    arena,
                    "animated world text",
                    &self.template,
                    mesh.vertices(),
                    1,
                )
            } else if let Some(vertices) = &uploaded {
                retained
                    .stream
                    .prepare_pass(arena, "animated world text", &self.template)
                    .map(|pass| NativeSubmission {
                        pass,
                        vertices: vertices.clone(),
                    })
                    .map_err(refuse)
            } else {
                retained
                    .stream
                    .prepare_packed_pass(
                        arena,
                        "animated world text",
                        &self.template,
                        &mesh.bytes,
                        STRIDE,
                        mesh.vertices() as usize,
                    )
                    .map_err(refuse)
            };
            match submission {
                Ok(submission) => {
                    if !native_layout && uploaded.is_none() {
                        uploaded = Some(submission.vertices.clone());
                    }
                    prepared.groups.push((submission, group.vertices.clone()));
                }
                Err(error) => {
                    if first_refusal.is_none() {
                        first_refusal = Some(if let Err(reason) = &view {
                            refuse(format!("{error}; font page {}: {reason}", group.texture))
                        } else {
                            error
                        });
                    }
                }
            }
        }
        if let Some(error) = first_refusal {
            return Err(error);
        }
        Ok(prepared)
    }
}

fn constants(camera: &Camera, auxiliary_color: [f32; 4]) -> [u8; 80] {
    let mut bytes = [0; 80];
    for (out, value) in bytes
        .chunks_exact_mut(4)
        .zip(camera.view_projection.into_iter().chain(auxiliary_color))
    {
        out.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn disabled_depth() -> DepthStencilState {
    DepthStencilState {
        depth_enable: false,
        depth_write_mask: 0,
        ..Default::default()
    }
}

pub fn bind_depth(retained: &mut shader_scene::State) {
    retained.stream.bind_depth_stencil(disabled_depth());
}

pub fn draw(
    pass: &mut wgpu::RenderPass<'_>,
    prepared: &Prepared,
    target: [u32; 2],
    scissor: Option<[u32; 4]>,
) -> io::Result<()> {
    for (submission, range) in &prepared.groups {
        let rectangle = scissor_rectangle(submission.pass.scissor_enabled, target, scissor)?;
        pass.set_scissor_rect(rectangle[0], rectangle[1], rectangle[2], rectangle[3]);
        pass.set_stencil_reference(submission.pass.stencil_reference.unwrap_or(0));
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&submission.pass.pipeline.pipeline);
        submission.pass.bindings.bind(pass);
        for (slot, vertices) in submission.vertices.iter().enumerate() {
            shader_pipeline::gpu::stats::state();
            pass.set_vertex_buffer(slot as u32, vertices.slice());
        }
        shader_pipeline::gpu::stats::draw();
        pass.draw(range.clone(), 0..1);
    }
    Ok(())
}

fn scissor_rectangle(
    enabled: Option<bool>,
    target: [u32; 2],
    rectangle: Option<[u32; 4]>,
) -> io::Result<[u32; 4]> {
    match enabled {
        Some(true) => rectangle
            .ok_or_else(|| refuse("enabled native scissor rectangle has not been supplied")),
        Some(false) => Ok([0, 0, target[0], target[1]]),
        None => Err(refuse("native raster scissor enable has not been supplied")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            right: [1., 0., 0.],
            up: [0., 1., 0.],
            view_projection: [0.; 16],
        }
    }

    fn mesh(font: &Font, bytes: &[u8], phase: f32) -> Mesh {
        geometry(
            &Text {
                font,
                bytes,
                color: [0.2, 0.4, 0.6, 0.0],
                global_colors: &BTreeMap::new(),
                width: -1,
                animation: Animation {
                    origin: [0.; 3],
                    phase,
                    limit: 1.,
                    scale: 1.,
                    reach: 1.,
                    opacity: 1.,
                },
            },
            &camera(),
            &|_| None,
        )
        .unwrap()
    }

    fn f32_at(mesh: &Mesh, vertex: usize, field: usize) -> f32 {
        let at = vertex * STRIDE + field * 4;
        f32::from_le_bytes(mesh.bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn native_513th_visible_glyph_is_not_submitted_or_continued() {
        let font = native_text::monospace("world", 10, 10);
        let built = mesh(&font, &vec![b'A'; 513], 0.);
        assert_eq!(built.vertices(), 3072);
        assert_eq!(
            built.groups,
            [Group {
                texture: 0,
                vertices: 0..3072
            }]
        );
        assert_eq!(built.bytes.len() as u64, ALLOCATION);
        assert_eq!(f32_at(&built, 0, 0), -64.125);
        assert!(!mesh(&font, b"", 0.).has_text);
        assert_eq!(mesh(&font, b"\xff", 0.).vertices(), 0);
    }

    #[test]
    fn font_constants_have_exact_80_byte_allocation_and_unquantized_modcolor() {
        let mut camera = camera();
        camera.view_projection[15] = 2.;
        let bytes = constants(&camera, [0.2, 0.3, 0.4, 0.5]);
        assert_eq!(bytes.len(), 80);
        assert_eq!(&bytes[60..64], &2_f32.to_le_bytes());
        assert_eq!(&bytes[64..68], &0.2_f32.to_le_bytes());
        assert_eq!(&bytes[76..80], &0.5_f32.to_le_bytes());
    }

    #[test]
    fn empty_inline_icon_never_resolves_a_defined_empty_stem() {
        let font = native_text::monospace("world", 10, 10);
        let calls = std::cell::Cell::new(0);
        let built = geometry(
            &Text {
                font: &font,
                bytes: b"A\x13 B",
                color: [1.; 4],
                global_colors: &BTreeMap::new(),
                width: -1,
                animation: Animation {
                    origin: [0.; 3],
                    phase: 0.,
                    limit: 1.,
                    scale: 1.,
                    reach: 1.,
                    opacity: 1.,
                },
            },
            &camera(),
            &|name| {
                calls.set(calls.get() + 1);
                (name == b"GFX_").then_some(([100, 100], 1))
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 0);
        assert_eq!(built.vertices(), 12);
        assert_eq!(f32_at(&built, 0, 0), -0.25);
        assert_eq!(f32_at(&built, 6, 0), 0.);
    }

    #[test]
    fn unknown_native_scissor_is_never_replaced_by_new_pass_defaults() {
        let target = [100, 50];
        let rectangle = [2, 3, 40, 20];
        assert_eq!(
            scissor_rectangle(Some(true), target, Some(rectangle)).unwrap(),
            rectangle
        );
        assert_eq!(
            scissor_rectangle(Some(false), target, None).unwrap(),
            [0, 0, 100, 50]
        );
        assert!(scissor_rectangle(Some(true), target, None).is_err());
        assert!(scissor_rectangle(None, target, Some(rectangle)).is_err());
    }
}
