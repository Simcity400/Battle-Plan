
use shader_pipeline::gpu::stats::CountedPasses as _;
use viewer_gpu::gpu_contract::DEPTH_FORMAT;
use viewer_gpu::gpu_contract::SCENE_FORMAT;
use viewer_gpu::province_admission::admits_box;
mod draw;
mod geometry;

use viewer_unit_reads::movement_routes::Route;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_gui::gui::native::{resources, sprites};
use content_reader_image::{dds, native::Decode};
use shader_pipeline::gpu;

use viewer_scene_base::shader_scene;
use viewer_gpu::{gpu_contract::Targets, province_map::HeightField, textures::{self, ColorSpace}};
use viewer_runtime::unit_handle::Unit;

const STYLES: [&[u8]; 8] = [
    b"retreatArrow",
    b"withdrawArrow",
    b"navalMissionArrow",
    b"supportArrow",
    b"attackArrow",
    b"transportArrow",
    b"followOrdersArrow",
    b"moveArrow",
];

struct Style {
    order: usize,
    size: f32,
    effect: Arc<shader_pipeline::library::Effect>,
    textures: [Result<Option<dds::Texture>, String>; 4],
}

pub struct Assets {
    styles: BTreeMap<&'static [u8], Result<Style, String>>,
    orders: BTreeMap<&'static [u8], Option<usize>>,
    speed: f32,
    cutoff: f32,
}

impl Assets {
    pub fn load(
        layers: &ResolvedLayers,
        defines: &Defines,
        map: [u32; 2],
        decode: Decode,
    ) -> io::Result<Self> {
        let files = sprites::read_object_layers(layers)?;
        let registry = sprites::object_registry(&files);
        let file_refusals: Vec<_> = files
            .iter()
            .flat_map(|(_, read)| &read.refusals)
            .map(ToString::to_string)
            .collect();
        let (mut library, shader_defines) =
            viewer_scene_base::shader_sources::library(layers, map, decode);
        let recorded = layers.without_overlays();
        let mut styles = BTreeMap::new();
        let mut orders = BTreeMap::new();
        for name in STYLES {
            let order = registry
                .get(name)
                .filter(|object| matches!(object.kind, sprites::ObjectKind::Arrow(_)))
                .and_then(|object| {
                    files
                        .iter()
                        .flat_map(|(_, read)| &read.objects)
                        .filter(|object| matches!(object.kind, sprites::ObjectKind::Arrow(_)))
                        .position(|candidate| std::ptr::eq(candidate, *object))
                });
            orders.insert(name, order);
            let result = (|| -> Result<Style, String> {
                if !file_refusals.is_empty() {
                    return Err(file_refusals.join("; "));
                }
                let object = registry.get(name).ok_or_else(|| {
                    format!(
                        "object {:?} is not registered",
                        String::from_utf8_lossy(name)
                    )
                })?;
                if !object.refusals.is_empty() {
                    return Err(object
                        .refusals
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "));
                }
                let sprites::ObjectKind::Arrow(definition) = &object.kind else {
                    return Err("latest object declaration is not arrowType".into());
                };
                if !definition.size.is_finite() || definition.size <= 0.0 {
                    return Err("arrowType size is not finite and positive".into());
                }
                let effect = if definition.effect.is_empty() {
                    shader_pipeline::library::Effect::from_compiled(
                        Arc::new(content_reader_gui::pdxshader::program::Program::default()),
                        Vec::new(),
                        Vec::new(),
                    )
                } else {
                    library
                        .load(&definition.effect, &shader_defines)?
                };
                let mut images = Vec::with_capacity(4);
                for path in [
                    &definition.texture,
                    &definition.normal,
                    &definition.specular,
                    &definition.overlay,
                ] {
                    images.push(
                        arrow_texture(layers, recorded.as_ref(), path, decode)
                            .map_err(|error| error.to_string())
                            .and_then(|texture| match texture {
                                Ok(texture) => Ok(Some(texture.image)),
                                Err(reason) if reason.native_failure() => Ok(None),
                                Err(reason) => Err(reason.to_string()),
                            }),
                    );
                }
                Ok(Style {
                    order: order.expect("registered arrow belongs to its construction stream"),
                    size: definition.size,
                    effect,
                    textures: images.try_into().expect("four native texture slots"),
                })
            })();
            styles.insert(name, result);
        }
        let number = |key: &str| -> io::Result<f32> {
            let value = defines.number("NGraphics", key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(io::Error::other(format!(
                    "NDefines.NGraphics.{key} is not finite"
                )))
            }
        };
        Ok(Self {
            styles,
            orders,
            speed: number("ARROW_MOVEMENT_SPEED")?,
            cutoff: number("UNIT_ARROW_DISTANCE_CUTOFF")?,
        })
    }
}

struct Uploaded {
    order: usize,
    size: f32,
    arrow: draw::Arrow,
    textures: [draw::Texture; 4],
}

#[derive(Clone, Copy, Debug, Default)]
struct Phase {
    draw_length: f32,
    fill: f32,
    fill_scale: f32,
}

impl Phase {
    fn new() -> Self {
        Self {
            fill_scale: 1.0,
            ..Self::default()
        }
    }

    fn step(
        &mut self,
        delta: f32,
        total: f32,
        target: f32,
        speed: f32,
        rate: &mut Option<f32>,
        admitted: bool,
    ) {
        if !admitted {
            self.draw_length = 0.0;
            return;
        }
        self.draw_length = (self.draw_length + delta * total * speed).clamp(0.0, total);
        if (target - self.fill).abs() > 0.01 {
            self.fill_scale *= 5.0;
        }
        let rate = *rate.get_or_insert(self.fill_scale * 10.0);
        let fill = self.fill + rate * delta;
        self.fill = if fill >= 0.0 {
            if target < fill { target } else { fill }
        } else {
            0.0
        };
        if target - self.fill > 0.01 {
            self.fill_scale = 1.0;
        }
    }
}

struct Arrow {
    route: Route,
    vertices: Arc<[u8]>,
    count: u32,
    mesh_length: f32,
    bounds: Bounds,
    phase: Phase,
    serial: u64,
    admitted: bool,
}

pub struct Player {
    retained: shader_scene::State,
    styles: BTreeMap<&'static [u8], Result<Uploaded, String>>,
    orders: BTreeMap<&'static [u8], Option<usize>>,
    arrows: BTreeMap<Unit, Arrow>,
    draw_order: Vec<Unit>,
    next_serial: u64,
    epoch: Option<(u64, i32, u32)>,
    selected: BTreeSet<Unit>,
    speed: f32,
    cutoff: f32,
    fill_rate: Option<f32>,
    warnings: Vec<String>,
    refused_styles: BTreeSet<&'static [u8]>,
    refused_routes: Vec<(&'static [u8], u32)>,
    draw_refusals: BTreeMap<String, usize>,
}

impl Player {
    pub fn matches(&self, epoch: (u64, i32, u32), selected: &BTreeSet<Unit>) -> bool {
        self.epoch == Some(epoch) && &self.selected == selected
    }

    pub fn cutoff(&self) -> f32 {
        self.cutoff
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, assets: &Assets, samples: u32) -> Self {
        let styles = assets
            .styles
            .iter()
            .map(|(&name, style)| {
                let uploaded = (|| -> Result<Uploaded, String> {
                    let style = style.as_ref().map_err(Clone::clone)?;
                    let arrow =
                        draw::Arrow::new(&style.effect, samples, SCENE_FORMAT, DEPTH_FORMAT)
                            .map_err(|error| error.to_string())?;
                    let mut views = Vec::with_capacity(4);
                    for (slot, image) in style.textures.iter().enumerate() {
                        views.push(image.as_ref().map_err(Clone::clone).and_then(|image| {
                            image
                                .as_ref()
                                .map(|image| {
                                    textures::upload(
                                        device,
                                        queue,
                                        "selected movement arrow",
                                        image,
                                        if matches!(slot, 0 | 3) {
                                            ColorSpace::Srgb
                                        } else {
                                            ColorSpace::Linear
                                        },
                                    )
                                    .map(|texture| texture.view)
                                })
                                .transpose()
                                .map_err(|error| error.to_string())
                        }));
                    }
                    Ok(Uploaded {
                        order: style.order,
                        size: style.size,
                        arrow,
                        textures: views.try_into().expect("four native texture slots"),
                    })
                })();
                (name, uploaded)
            })
            .collect();
        Self {
            retained: shader_scene::State::default(),
            styles,
            orders: assets.orders.clone(),
            arrows: BTreeMap::new(),
            draw_order: Vec::new(),
            next_serial: 0,
            epoch: None,
            selected: BTreeSet::new(),
            speed: assets.speed,
            cutoff: assets.cutoff,
            fill_rate: None,
            warnings: Vec::new(),
            refused_styles: BTreeSet::new(),
            refused_routes: Vec::new(),
            draw_refusals: BTreeMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.arrows.clear();
        self.draw_order.clear();
        self.epoch = None;
        self.selected.clear();
        self.warnings.clear();
        self.refused_styles.clear();
        self.refused_routes.clear();
    }

    pub fn set(
        &mut self,
        epoch: (u64, i32, u32),
        routes: Vec<Route>,
        mut warnings: Vec<String>,
        heights: HeightField<'_>,
        width: f32,
        selected: &BTreeSet<Unit>,
    ) {
        if self.epoch == Some(epoch) && &self.selected == selected {
            return;
        }
        if self.epoch.is_some_and(|(branch, hour, owner)| {
            branch != epoch.0 || owner != epoch.2 || epoch.1 < hour
        }) {
            self.reset();
        }
        self.epoch = Some(epoch);
        self.selected.clone_from(selected);

        let mut previous = std::mem::take(&mut self.arrows);
        self.refused_styles.clear();
        self.refused_routes.clear();
        for route in routes {
            let selector = route.style;
            let unit = route.unit;
            let province = route.province;
            let result = (|| -> Result<Arrow, String> {
                let style = self
                    .styles
                    .get(route.style)
                    .ok_or("unknown native movement selector")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                let mut prior = previous.remove(&route.unit);
                if prior.as_ref().is_some_and(|arrow| {
                    arrow.route.controls == route.controls && arrow.route.style == route.style
                }) {
                    let mut arrow = prior.take().expect("matched prior route");
                    arrow.route = route;
                    return Ok(arrow);
                }
                let phase = prior
                    .filter(|arrow| arrow.route.controls == route.controls)
                    .map_or_else(Phase::new, |arrow| arrow.phase);
                let strip = geometry::build(&route.controls, style.size, heights)?;
                let count = u32::try_from(strip.vertices.len())
                    .map_err(|_| "movement arrow exceeds native vertex count")?;
                let bounds = Bounds::of(strip.bounds, width);
                self.next_serial = self.next_serial.wrapping_add(1);
                Ok(Arrow {
                    route,
                    count,
                    vertices: Arc::from(bytemuck::cast_slice(&strip.vertices)),
                    mesh_length: strip.length,
                    bounds,
                    phase,
                    serial: self.next_serial,
                    admitted: false,
                })
            })();
            match result {
                Ok(arrow) if arrow.count > 0 => {
                    self.arrows.insert(arrow.route.unit, arrow);
                }
                Ok(_) => {}
                Err(reason) => {
                    self.refused_routes.push((selector, province));
                    warnings.push(format!(
                        "selected movement arrow {unit:?} ({}): {reason}",
                        String::from_utf8_lossy(selector)
                    ));
                }
            }
        }
        self.draw_order = self.arrows.keys().copied().collect();
        self.draw_order.sort_by_key(|unit| {
            let arrow = &self.arrows[unit];
            (
                self.styles[arrow.route.style]
                    .as_ref()
                    .expect("placed style is uploaded")
                    .order,
                arrow.serial,
            )
        });
        self.warnings = warnings;
    }

    pub fn advance(
        &mut self,
        delta: f32,
        backwards: bool,
        camera_height: f32,
        in_view: impl Fn(u32) -> bool,
    ) {
        self.draw_refusals.clear();
        self.refused_styles = self
            .refused_routes
            .iter()
            .filter(|(_, province)| camera_height < self.cutoff && in_view(*province))
            .map(|(style, _)| *style)
            .collect();
        for arrow in self.arrows.values_mut() {
            arrow.admitted = camera_height < self.cutoff && in_view(arrow.route.province);
            if backwards {
                arrow.phase = Phase::new();
            }
            arrow.phase.step(
                delta,
                arrow.route.total_arc,
                arrow.route.fill_target,
                self.speed,
                &mut self.fill_rate,
                arrow.admitted,
            );
        }
    }

    #[expect(clippy::too_many_arguments, reason = "the arena, encoder and targets, plus the caller's state, matrix, planes and visibility of one draw")]
    pub fn draw(
        &mut self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        caller: &mut shader_scene::State,
        matrix: &[[f32; 4]; 4],
        planes: &viewer_gpu::province_admission::Planes,
        visible: bool,
    ) -> bool {
        self.retained.inherit(caller);
        let retained = &mut self.retained;
        let mut pixel_replaced = false;
        let mut bucket = None;
        let mut last_order = None;
        for unit in &self.draw_order {
            let arrow = &self.arrows[unit];
            if !arrow.admitted {
                continue;
            }
            let Ok(style) = &self.styles[arrow.route.style] else {
                continue;
            };
            if bucket != Some(arrow.route.style) {
                if self.refused_styles.iter().any(|name| {
                    self.orders[name].is_none_or(|order| {
                        last_order.is_none_or(|last| last < order) && order < style.order
                    })
                }) {
                    forget_unknown(retained);
                    pixel_replaced = true;
                }
                style.arrow.bind_style(retained, &style.textures);
                bucket = Some(arrow.route.style);
                last_order = Some(style.order);
            }
            for offset in arrow.bounds.offsets(planes) {
                pixel_replaced = true;
                let constants = constants(
                    matrix,
                    &arrow.phase,
                    arrow.route.free_arc,
                    arrow.route.total_arc,
                    arrow.mesh_length,
                    offset,
                );
                match style.arrow.prepare(
                    arena,
                    retained,
                    arrow.vertices.clone(),
                    arrow.count,
                    &constants,
                    &style.textures,
                ) {
                    Ok(_) if !visible => {}
                    Ok(submission) => {
                        let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("selected movement arrow"),
                            color_attachments: &[Some(
                                targets.scene_attachment(wgpu::LoadOp::Load),
                            )],
                            depth_stencil_attachment: Some(
                                targets.depth_attachment(wgpu::LoadOp::Load),
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            ..Default::default()
                        });
                        draw::draw(&mut pass, &submission, arrow.count);
                    }
                    Err(error) => {
                        *self.draw_refusals.entry(error.to_string()).or_default() += 1;
                    }
                }
            }
        }
        if self.refused_styles.iter().any(|name| {
            self.orders[name].is_none_or(|order| last_order.is_none_or(|last| last < order))
        }) {
            forget_unknown(retained);
            pixel_replaced = true;
        }
        draw::restore_depth(retained);
        caller.inherit(retained);
        pixel_replaced
    }

    pub fn warnings(&self) -> Vec<String> {
        self.warnings
            .iter()
            .cloned()
            .chain(self.draw_refusals.iter().map(|(reason, count)| {
                format!("selected movement arrows: {count} refused draws: {reason}")
            }))
            .collect()
    }
}

fn forget_unknown(retained: &mut shader_scene::State) {
    use content_reader_gui::pdxshader::program::Stage;
    retained.stream.forget_program();
    for slot in 0..4 {
        retained.stream.forget_texture(slot, Stage::Pixel, None);
    }
    for stage in [Stage::Vertex, Stage::Pixel] {
        retained
            .stream
            .bind_partial_constants(1, stage, Some(&[0; 112]), &[]);
    }
}

fn constants(
    matrix: &[[f32; 4]; 4],
    phase: &Phase,
    free: f32,
    total: f32,
    mesh: f32,
    offset: f32,
) -> [u8; 112] {
    let factor = if mesh > 0.0 { 2.0 - total / mesh } else { 1.0 };
    let mut bytes = [0; 112];
    bytes[..64].copy_from_slice(bytemuck::bytes_of(matrix));
    bytes[64..88].copy_from_slice(bytemuck::cast_slice(&[
        factor * phase.draw_length,
        factor * phase.fill,
        factor * free,
        -100.0,
        -1.0,
        -1.0,
    ]));
    bytes[96..100].copy_from_slice(&offset.to_le_bytes());
    bytes
}

#[derive(Clone, Copy)]
struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
    companion: Option<f32>,
}

impl Bounds {
    fn of([min, max]: [[f32; 3]; 2], width: f32) -> Self {
        Self {
            min,
            max,
            companion: if min[0] < 0.0 {
                Some(width)
            } else if max[0] > width {
                Some(-width)
            } else {
                None
            },
        }
    }

    fn offsets(
        self,
        planes: &viewer_gpu::province_admission::Planes,
    ) -> impl Iterator<Item = f32> {
        [Some(0.0), self.companion]
            .into_iter()
            .flatten()
            .filter(move |offset| admits_box([self.min, self.max], *offset, planes))
    }
}

fn arrow_texture(
    layers: &ResolvedLayers,
    recorded: Option<&ResolvedLayers>,
    path: &[u8],
    decode: Decode,
) -> io::Result<Result<resources::Texture, resources::NoTexture>> {
    let shown = resources::texture(path, &|path: &[u8]| layers.read_path(path), decode);
    let why = match &shown {
        Ok(Ok(_)) => None,
        Ok(Err(why)) => Some(why.to_string()),
        Err(error) => Some(error.to_string()),
    };
    if let Some(why) = why
        && let Some(recorded) = recorded
        && let Ok(Ok(found)) =
            resources::texture(path, &|path: &[u8]| recorded.read_path(path), decode)
    {
        layers.note_overlay_refusal(format!(
            "movement arrow texture {} ({why})",
            String::from_utf8_lossy(path)
        ));
        return Ok(Ok(found));
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_phase_expands_fills_and_resets_at_native_admission() {
        let mut phase = Phase::new();
        let mut rate = None;
        phase.step(0.0, 100.0, 20.0, 2.0, &mut rate, true);
        assert_eq!((phase.draw_length, phase.fill), (0.0, 0.0));
        phase.step(0.1, 100.0, 20.0, 2.0, &mut rate, true);
        assert_eq!((phase.draw_length, phase.fill), (20.0, 5.0));
        phase.step(1.0, 100.0, 20.0, 2.0, &mut rate, true);
        assert_eq!((phase.draw_length, phase.fill), (100.0, 20.0));
        phase.step(0.1, 100.0, 20.0, 2.0, &mut rate, false);
        assert_eq!((phase.draw_length, phase.fill), (0.0, 20.0));
    }

    #[test]
    fn physical_constants_preserve_native_callback_and_wrap_offsets() {
        let matrix = [[7.0; 4]; 4];
        let bytes = constants(
            &matrix,
            &Phase {
                draw_length: 30.0,
                fill: 12.0,
                fill_scale: 1.0,
            },
            1.0,
            100.0,
            200.0,
            -1000.0,
        );
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        assert_eq!(&floats[..16], &[7.0; 16]);
        assert_eq!(&floats[16..22], &[45.0, 18.0, 1.5, -100.0, -1.0, -1.0]);
        assert_eq!(&floats[22..25], &[0.0, 0.0, -1000.0]);
    }

    #[test]
    fn signed_fill_target_uses_native_ordered_clamp_without_panicking() {
        let mut phase = Phase::new();
        let mut rate = None;
        phase.step(0.1, 100.0, -20.0, 2.0, &mut rate, true);
        assert_eq!(phase.fill, -20.0);
        phase.step(0.0, 100.0, -20.0, 2.0, &mut rate, true);
        assert_eq!(phase.fill, 0.0);
    }
}
