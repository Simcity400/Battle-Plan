use shader_pipeline::gpu::stats::CountedPasses as _;
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use std::{cell::RefCell, collections::BTreeMap, io, sync::Arc};
use viewer_runtime::errors::invalid;
use viewer_gpu::{
    camera::Camera,
    gpu_contract::{DEPTH_FORMAT, SCENE_FORMAT, Targets, buffer},
    ribbon_content::{ArrowType, Mode, OrderAssets, OrderDraw, OrderField},
    ribbon_native as native,
    textures::{ColorSpace, upload},
};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Slot {
    arrow_mask: [f32; 4],
    arrow_color: [f32; 4],
    arrow_secondary_color: [f32; 4],
    time_selected_fade: [f32; 4],
    ofs_x_tex_fx: [f32; 4],
    body_uv_scale: f32,
    secondary_body_uv_scale: f32,
    texture_variant_scale: f32,
    culling_offset: f32,
    shadow_factor: f32,
    normal_map_factor: f32,
    seethrough_opacity: f32,
    total_length: f32,
    uv_to_absolute_length: f32,
    _padding: [f32; 3],
}

struct TypeResources {
    compiled: BTreeMap<Mode, Arc<crate::arrow_shader::Arrow>>,
    mask: wgpu::TextureView,
    pattern: wgpu::TextureView,
    arrow: ArrowType,
    variants: u32,
}

#[derive(Clone, Copy)]
pub struct NativeInputs<'a> {
    pub retained: Option<&'a RefCell<viewer_scene_base::shader_scene::State>>,
    pub scene: &'a [u8],
    pub grid: &'a [u8],
    pub textures: [&'a wgpu::TextureView; 9],
}

struct NativeDraw<'a> {
    draw: &'a OrderDraw,
    program: &'a crate::arrow_shader::Arrow,
    bound: shader_pipeline::gpu::Bindings,
}

pub const ORDER_PARTS: usize = 4;
const AIR: usize = 3;

const LAYOUT: [usize; ORDER_PARTS] = [1, 2, 3, 0];

const CHUNK: usize = 64;

#[derive(Clone, Default)]
struct Part {
    start: u32,
    len: u32,
    draws: Vec<OrderDraw>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderUpload {
    pub bytes: usize,
    pub writes: usize,
    pub draws: bool,
}

pub struct OrderPass {
    types: BTreeMap<(String, usize), TypeResources>,
    native_vertices: Option<wgpu::Buffer>,
    native_bytes: Arc<[u8]>,
    draws: Vec<OrderDraw>,
    native_plans: BTreeMap<Mode, Vec<native::View>>,
    parts: [Part; ORDER_PARTS],
}

fn places(parts: &[Option<&OrderField>; ORDER_PARTS]) -> ([Part; ORDER_PARTS], u32) {
    let mut placed: [Part; ORDER_PARTS] = Default::default();
    let mut at = 0;
    for &part in &LAYOUT {
        let len = parts[part].map_or(0, |field| field.native_vertices.len() as u32);
        placed[part] = Part { start: at, len, draws: Vec::new() };
        at += len;
    }
    (placed, at)
}

fn changed_runs(parts: &[Option<&OrderField>; ORDER_PARTS], placed: &[Part; ORDER_PARTS], held: &[u8]) -> Vec<std::ops::Range<usize>> {
    let stride = std::mem::size_of::<viewer_gpu::ribbon_content::Vertex>();
    let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
    for &part in &LAYOUT {
        let Some(field) = parts[part] else { continue };
        let bytes: &[u8] = bytemuck::cast_slice(&field.native_vertices);
        let base = placed[part].start as usize * stride;
        for start in (0..bytes.len()).step_by(CHUNK * stride) {
            let end = (start + CHUNK * stride).min(bytes.len());
            if held.get(base + start..base + end) == Some(&bytes[start..end]) {
                continue;
            }
            match runs.last_mut() {
                Some(run) if run.end == base + start => run.end = base + end,
                _ => runs.push(base + start..base + end),
            }
        }
    }
    runs
}

fn laid_out(parts: &[Option<&OrderField>; ORDER_PARTS]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &part in &LAYOUT {
        if let Some(field) = parts[part] {
            bytes.extend_from_slice(bytemuck::cast_slice(&field.native_vertices));
        }
    }
    bytes
}

fn draw_list(parts: &[Option<&OrderField>; ORDER_PARTS], placed: &[Part; ORDER_PARTS]) -> Vec<OrderDraw> {
    let moved = |part: usize, draw: &OrderDraw, label: bool| {
        let base = placed[part].start;
        OrderDraw { native_strips: draw.native_strips.iter().map(|strip| strip.start + base..strip.end + base).collect(), label: if label { draw.label } else { None }, ..draw.clone() }
    };
    let mut draws: Vec<OrderDraw> = parts[0].map_or_else(Vec::new, |plan| plan.draws.iter().map(|draw| moved(0, draw, true)).collect());
    for part in 1..ORDER_PARTS {
        let Some(field) = parts[part] else { continue };
        if part == AIR {
            draws.retain(|draw| !draw.air_mode);
        }
        draws.extend(field.draws.iter().map(|draw| moved(part, draw, false)));
        viewer_gpu::ribbon_mesh::sort_draws(&mut draws);
    }
    draws
}

impl OrderPass {
    pub fn texel_reads(&self) -> impl Iterator<Item = &str> {
        self.types
            .values()
            .flat_map(|resources| resources.compiled.values())
            .filter_map(|arrow| arrow.texel_reads.as_deref())
    }

    pub fn prepare(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        wraps: &[f32],
        shown: impl Fn(&OrderDraw) -> bool,
    ) {
        self.native_plans = native::plans(&self.draws, camera, size, wraps, |draw, mode| {
            self.types
                .get(&(draw.arrow_type.clone(), draw.texture_choice))
                .filter(|_| shown(draw))
                .filter(|resources| resources.compiled.contains_key(&mode))
                .map(|resources| &resources.arrow)
        });
    }

    pub fn labels(&self, mode: Mode) -> Vec<(usize, usize)> {
        self.native_plans
            .get(&mode)
            .into_iter()
            .flatten()
            .enumerate()
            .flat_map(|(view, plan)| plan.labels.iter().map(move |source| (view, *source)))
            .collect()
    }
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
        effect: &Arc<shader_pipeline::library::Effect>,
    ) -> io::Result<Self> {
        let mut pass = Self {
            types: BTreeMap::new(),
            native_vertices: None,
            native_bytes: Default::default(),
            draws: Vec::new(),
            native_plans: BTreeMap::new(),
            parts: Default::default(),
        };
        pass.upload_types(device, samples, queue, assets, effect)?;
        Ok(pass)
    }

    fn upload_types(
        &mut self,
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
        effect: &Arc<shader_pipeline::library::Effect>,
    ) -> io::Result<()> {
        let mut programs = BTreeMap::new();
        for (key, page) in &assets.pages {
            let name = &key.0;
            let Some(arrow) = assets.types.get(name) else {
                continue;
            };
            let mask = upload(
                device,
                queue,
                &format!("TexMask ({name})"),
                &page.mask,
                ColorSpace::Linear,
            )?;
            let pattern = upload(
                device,
                queue,
                &format!("TexPattern ({name})"),
                &page.pattern,
                ColorSpace::Srgb,
            )?;
            let mut compiled = BTreeMap::new();
            for mode in [Mode::Shadow, Mode::Main, Mode::Seethrough, Mode::Late] {
                if !mode.admits(arrow.draw_last) || (mode == Mode::Shadow && !arrow.draw_shadows) {
                    continue;
                }
                let name = if mode == Mode::Seethrough {
                    format!("{}Seethrough", arrow.shader_technique)
                } else {
                    arrow.shader_technique.clone()
                };
                if mode == Mode::Seethrough && effect.select(&name).map_err(invalid)?.is_none() {
                    continue;
                }
                let key = (name.clone(), mode);
                let program = if let Some(program) = programs.get(&key) {
                    Arc::clone(program)
                } else {
                    let program = Arc::new(crate::arrow_shader::Arrow::new(
                        device,
                        effect,
                        &name,
                        if mode == Mode::Shadow { 1 } else { samples },
                        if mode == Mode::Shadow {
                            viewer_scene_base::shadow::DEPTH_MAP_FORMAT
                        } else {
                            SCENE_FORMAT
                        },
                        DEPTH_FORMAT,
                        mode,
                    )?);
                    programs.insert(key, program.clone());
                    program
                };
                compiled.insert(mode, program);
            }
            self.types.insert(
                key.clone(),
                TypeResources {
                    compiled,
                    mask: mask.view,
                    pattern: pattern.view,
                    arrow: arrow.clone(),
                    variants: page.variants,
                },
            );
        }
        Ok(())
    }

    pub fn upload_parts(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, parts: [Option<&OrderField>; ORDER_PARTS]) -> io::Result<OrderUpload> {
        let stride = std::mem::size_of::<viewer_gpu::ribbon_content::Vertex>();
        let (mut placed, total) = places(&parts);
        let size = total as usize * stride;
        let mut done = OrderUpload::default();
        if size == 0 {
            done.draws = !self.draws.is_empty();
            (self.native_vertices, self.native_bytes, self.draws, self.parts) = (None, Default::default(), Vec::new(), placed);
            return Ok(done);
        }
        let runs = changed_runs(&parts, &placed, &self.native_bytes);
        let fits = self.native_vertices.as_ref().is_some_and(|held| held.size() >= size as u64);
        if !runs.is_empty() || size != self.native_bytes.len() || !fits {
            let bytes = laid_out(&parts);
            if fits {
                let held = self.native_vertices.as_ref().expect("fits");
                for run in &runs {
                    shader_pipeline::gpu::stats::write_buffer(queue, held, run.start as u64, &bytes[run.clone()]);
                    (done.bytes, done.writes) = (done.bytes + run.len(), done.writes + 1);
                }
            } else {
                let grown = (size as u64).max(self.native_vertices.as_ref().map_or(0, |held| held.size() * 2));
                let mut padded = bytes.clone();
                padded.resize(grown as usize, 0);
                self.native_vertices = Some(buffer(device, "native battle-plan ribbon vertices", wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, &padded)?);
                (done.bytes, done.writes) = (size, 1);
            }
            self.native_bytes = bytes.into();
        }
        let moved = placed.iter().zip(&self.parts).zip(&parts).any(|((now, was), field)| now.start != was.start || now.len != was.len || field.map_or(&[][..], |field| &field.draws[..]) != &was.draws[..]);
        if moved || self.draws.is_empty() {
            self.draws = draw_list(&parts, &placed);
            for (part, field) in placed.iter_mut().zip(&parts) {
                part.draws = field.map_or_else(Vec::new, |field| field.draws.clone());
            }
            self.parts = placed;
            done.draws = true;
        }
        Ok(done)
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, field: OrderField) -> io::Result<()> {
        self.upload_parts(device, queue, [Some(&field), None, None, None]).map(drop)
    }

    fn prepare_bucket(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        native: NativeInputs<'_>,
        mode: Mode,
        time: f32,
        bucket: &[native::Draw],
    ) -> io::Result<Vec<NativeDraw<'_>>> {
        use content_reader_gui::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let mut textures: BTreeMap<_, _> = native
            .textures
            .iter()
            .enumerate()
            .map(|(index, view)| (binding(index as u32 + 2, Stage::Pixel), (*view).clone()))
            .collect();
        let mut draws = Vec::new();
        for outer in [false, true] {
            for admitted in bucket {
                let draw = &self.draws[admitted.index];
                let resources = &self.types[&(draw.arrow_type.clone(), draw.texture_choice)];
                let mask = if outer {
                    resources.arrow.mask_outer
                } else {
                    resources.arrow.mask_inner
                };
                if mask == [0.; 3] {
                    continue;
                }
                textures.insert(binding(0, Stage::Pixel), resources.mask.clone());
                textures.insert(binding(1, Stage::Pixel), resources.pattern.clone());
                for &offset in &admitted.offsets {
                    let constants = Self::slot(draw, resources, outer, offset, time, mode);
                    let program = &resources.compiled[&mode];
                    let bound = program.bindings(
                        arena,
                        native.scene,
                        native.grid,
                        bytemuck::bytes_of(&constants),
                        &textures,
                    )?;
                    if let Some(retained) = native.retained {
                        let mut retained = retained.borrow_mut();
                        program.retain(
                            &mut retained,
                            native.scene,
                            native.grid,
                            bytemuck::bytes_of(&constants),
                            &textures,
                        );
                        retained.bind_vertex(0, 32, self.native_bytes.clone());
                    }
                    draws.push(NativeDraw {
                        draw,
                        program,
                        bound,
                    });
                }
            }
        }
        Ok(draws)
    }

    fn draw_shadow(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        native: NativeInputs<'_>,
        view: usize,
        time: f32,
    ) -> io::Result<()> {
        let Some(vertices) = &self.native_vertices else {
            return Ok(());
        };
        let Some(plan) = self
            .native_plans
            .get(&Mode::Shadow)
            .and_then(|views| views.get(view))
        else {
            return Ok(());
        };
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, vertices.slice(..));
        for bucket in &plan.buckets {
            for draw in self.prepare_bucket(arena, native, Mode::Shadow, time, bucket)? {
                draw.program.begin(pass);
                draw.bound.bind(pass);
                for strip in &draw.draw.native_strips {
                    shader_pipeline::gpu::stats::draw();
                    pass.draw(strip.clone(), 0..1);
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        native: NativeInputs<'_>,
        mode: Mode,
        view: usize,
        time: f32,
    ) -> io::Result<()> {
        let Some(vertices) = &self.native_vertices else {
            return Ok(());
        };
        let Some(plan) = self
            .native_plans
            .get(&mode)
            .and_then(|views| views.get(view))
        else {
            return Ok(());
        };
        for bucket in &plan.buckets {
            let draws = self.prepare_bucket(arena, native, mode, time, bucket)?;
            {
                let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("compiled map arrows"),
                    color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
                    depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                shader_pipeline::gpu::stats::state();
                pass.set_vertex_buffer(0, vertices.slice(..));
                for draw in draws {
                    draw.program.begin(&mut pass);
                    draw.bound.bind(&mut pass);
                    for strip in &draw.draw.native_strips {
                        shader_pipeline::gpu::stats::draw();
                        pass.draw(strip.clone(), 0..1);
                    }
                }
            }
            if mode == Mode::Seethrough {
                continue;
            }
            let mut attachment = targets.depth_attachment(wgpu::LoadOp::Load);
            attachment.stencil_ops = Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0),
                store: wgpu::StoreOp::Store,
            });
            let _clear = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("arrow definition stencil clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(attachment),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        Ok(())
    }

    fn slot(
        draw: &OrderDraw,
        resources: &TypeResources,
        outer: bool,
        offset: f32,
        time: f32,
        mode: Mode,
    ) -> Slot {
        let arrow = &resources.arrow;
        let [texture_speed, execution_fx] =
            draw.execution.shader_terms(arrow.texture_anim_speed, outer);
        Slot {
            arrow_mask: {
                let m = if outer {
                    arrow.mask_outer
                } else {
                    arrow.mask_inner
                };
                [m[0], m[1], m[2], 1.0]
            },
            arrow_color: if outer { draw.outer_color } else { draw.color },
            arrow_secondary_color: if outer {
                draw.outer_color
            } else {
                draw.secondary_color
            },
            time_selected_fade: [time, 0.0, arrow.fade_in, arrow.fade_out],
            ofs_x_tex_fx: [offset, texture_speed, execution_fx, 0.0],
            body_uv_scale: arrow.body_uv_scale,
            secondary_body_uv_scale: arrow.secondary_body_uv_scale,
            texture_variant_scale: 1.0 / resources.variants.max(1) as f32,
            culling_offset: draw.culling_offset,
            shadow_factor: mode.lighting(),
            normal_map_factor: mode.lighting(),
            total_length: draw.total_length,
            uv_to_absolute_length: draw.uv_to_length,
            seethrough_opacity: 0.2,
            _padding: [0.0; 3],
        }
    }
}

pub struct PlanSubmissions<'a, P = OrderPass> {
    shown: Option<&'a P>,
}

pub struct ShadowCasting<'a, P = OrderPass>(&'a P);

pub struct Drawing<'a, P = OrderPass>(&'a P);

impl<'a, P> PlanSubmissions<'a, P> {
    pub fn new(
        plans: Option<&'a P>,
        graphics: &dashboard_core::protocol::MapGraphics,
        interface: bool,
        trade_routes: bool,
    ) -> Self {
        Self {
            shown: plans.filter(|_| (graphics.battle_plans || trade_routes) && interface),
        }
    }

    pub fn cast_shadows(
        &self,
        cast: impl FnOnce(ShadowCasting<'a, P>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.shown
            .map_or(Ok(()), |plans| cast(ShadowCasting(plans)))
    }

    pub fn draw(&self, draw: impl FnOnce(Drawing<'a, P>) -> io::Result<()>) -> io::Result<()> {
        self.shown.map_or(Ok(()), |plans| draw(Drawing(plans)))
    }
}

impl PlanSubmissions<'_> {
    pub fn casts_shadows(&self) -> bool {
        self.shown.is_some_and(|plans| {
            plans.native_vertices.is_some()
                && plans
                    .native_plans
                    .get(&Mode::Shadow)
                    .is_some_and(|views| {
                        views.iter().flat_map(|view| &view.buckets).any(|bucket| !bucket.is_empty())
                    })
        })
    }
}

impl ShadowCasting<'_> {
    pub fn draw_shadow(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        native: NativeInputs<'_>,
        view: usize,
        time: f32,
    ) -> io::Result<()> {
        self.0.draw_shadow(arena, pass, native, view, time)
    }
}

impl Drawing<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        native: NativeInputs<'_>,
        mode: Mode,
        view: usize,
        time: f32,
    ) -> io::Result<()> {
        self.0
            .draw(arena, encoder, targets, native, mode, view, time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_gpu::ribbon_content::Vertex;

    fn draw(arrow: &str, declaration_index: usize, air_mode: bool, label: Option<usize>, strips: std::ops::Range<u32>) -> OrderDraw {
        OrderDraw {
            air_mode,
            arrow_type: arrow.to_owned(),
            texture_choice: 0,
            color: [1.0; 4],
            total_length: 1.0,
            uv_to_length: 1.0,
            culling_offset: 0.0,
            declaration_index,
            width: declaration_index as f32,
            label,
            native_strips: vec![strips],
            bounds: [[0.0; 3]; 2],
            secondary_color: [1.0; 4],
            outer_color: [1.0; 4],
            execution: Default::default(),
            kind: Default::default(),
        }
    }

    fn field(arrow: &str, declaration_index: usize, air_mode: bool, seed: f32, vertices: u32) -> OrderField {
        OrderField {
            native_vertices: (0..vertices).map(|at| Vertex { position: [seed, at as f32, 0.0], padding: 0, uv: [0.0; 4] }).collect(),
            draws: vec![draw(arrow, declaration_index, air_mode, (arrow == "plan").then_some(0), 0..vertices)],
            ..OrderField::default()
        }
    }

    #[test]
    fn equal_parts_write_nothing_and_a_changed_plan_writes_its_changed_chunks_alone() {
        let (plan, raids) = (field("plan", 2, false, 1.0, 300), field("raid", 1, false, 2.0, 70));
        let held = laid_out(&[Some(&plan), Some(&raids), None, None]);
        let (again, raids_again) = (plan.clone(), raids.clone());
        let parts = [Some(&again), Some(&raids_again), None, None];
        let (placed, _) = places(&parts);
        assert!(changed_runs(&parts, &placed, &held).is_empty(), "the same content anew writes nothing");

        let mut moved = plan.clone();
        moved.native_vertices[200].position[2] = 9.0;
        let parts = [Some(&moved), Some(&raids_again), None, None];
        let (placed, _) = places(&parts);
        let stride = std::mem::size_of::<Vertex>();
        let chunk = (placed[0].start as usize + 192) * stride..(placed[0].start as usize + 256) * stride;
        assert_eq!(changed_runs(&parts, &placed, &held), [chunk], "the changed vertex's chunk of the plan, nothing of the raids");

        let mut longer = plan.clone();
        longer.native_vertices.extend((0..10).map(|_| Vertex { position: [5.0; 3], padding: 0, uv: [0.0; 4] }));
        let parts = [Some(&longer), Some(&raids_again), None, None];
        let (placed, _) = places(&parts);
        let runs = changed_runs(&parts, &placed, &held);
        assert!(runs.iter().all(|run| run.start >= placed[0].start as usize * stride), "a longer plan writes past the raids: {runs:?}");
        assert_eq!(runs.len(), 1, "its new tail alone: {runs:?}");
    }

    #[test]
    fn plans_submit_their_shadows_and_draws_only_while_shown() {
        struct StandIn;
        let submitted = |graphics: dashboard_core::protocol::MapGraphics,
                         interface: bool,
                         trade_routes: bool| {
            let frame = PlanSubmissions::new(Some(&StandIn), &graphics, interface, trade_routes);
            let (mut casts, mut draws) = (0, 0);
            frame
                .cast_shadows(|_| {
                    casts += 1;
                    Ok(())
                })
                .unwrap();
            frame
                .draw(|_| {
                    draws += 1;
                    Ok(())
                })
                .unwrap();
            (casts, draws)
        };
        let shown = dashboard_core::protocol::MapGraphics::default();
        assert_eq!(submitted(shown, true, false), (1, 1));
        assert_eq!(submitted(shown, false, false), (0, 0));
        let switched_off = dashboard_core::protocol::MapGraphics {
            battle_plans: false,
            ..shown
        };
        assert_eq!(submitted(switched_off, true, false), (0, 0));
        assert_eq!(submitted(switched_off, true, true), (1, 1));
        assert_eq!(submitted(switched_off, false, true), (0, 0));
        let none = PlanSubmissions::<StandIn>::new(None, &shown, true, true);
        assert!(none.cast_shadows(|_| unreachable!()).is_ok());
        let failing = PlanSubmissions::new(Some(&StandIn), &shown, true, false)
            .cast_shadows(|_| Err(io::Error::other("refused")));
        assert!(failing.is_err());
    }
    #[test]
    fn native_arrow_tail_preserves_opacity_length_and_scale_offsets() {
        let slot = Slot {
            shadow_factor: 1.,
            normal_map_factor: 1.,
            total_length: 73.,
            uv_to_absolute_length: 19.,
            seethrough_opacity: 0.2,
            ..Default::default()
        };
        let bytes = bytemuck::bytes_of(&slot);
        assert_eq!(&bytes[104..108], &0.2_f32.to_le_bytes());
        assert_eq!(&bytes[108..112], &73_f32.to_le_bytes());
        assert_eq!(&bytes[112..116], &19_f32.to_le_bytes());
    }

}
