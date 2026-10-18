//! World submission of common battle-plan and raid ribbon draw data.
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use std::{cell::RefCell, collections::BTreeMap, io, sync::Arc};
use viewer_foundation::{
    errors::invalid,
    map::{
        camera::Camera,
        gpu_contract::{DEPTH_FORMAT, SCENE_FORMAT, Targets, buffer},
        ribbon_content::{ArrowType, Mode, OrderAssets, OrderDraw, OrderField},
        ribbon_native as native,
        textures::{ColorSpace, upload},
    },
};
// ------------------------------------------------------------------- the pass

/// 128-byte allocation; bytes 116..128 remain unknown to shader reads.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct Slot {
    arrow_mask: [f32; 4],
    arrow_color: [f32; 4],
    arrow_secondary_color: [f32; 4],
    /// `vTime_IsSelected_FadeInOut`.
    time_selected_fade: [f32; 4],
    /// `vOfsX_vOfsTex_vOfsFx`, with `w` unused.
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
    compiled: BTreeMap<Mode, Arc<crate::map::arrow_shader::Arrow>>,
    mask: wgpu::TextureView,
    pattern: wgpu::TextureView,
    arrow: ArrowType,
    variants: u32,
}

/// Physical PS slots 2..10; b0 is the original camera (b3 carries wrap).
#[derive(Clone, Copy)]
pub struct NativeInputs<'a> {
    pub retained: Option<&'a RefCell<crate::map::shader_scene::State>>,
    pub scene: &'a [u8],
    pub grid: &'a [u8],
    pub textures: [&'a wgpu::TextureView; 9],
}

struct NativeDraw<'a> {
    draw: &'a OrderDraw,
    program: &'a crate::map::arrow_shader::Arrow,
    bound: shader_pipeline::gpu::Bindings,
}

/// The GPU battle-plan pass.
pub struct OrderPass {
    types: BTreeMap<(String, usize), TypeResources>,
    native_vertices: Option<wgpu::Buffer>,
    native_bytes: Arc<[u8]>,
    draws: Vec<OrderDraw>,
    native_plans: BTreeMap<Mode, Vec<native::View>>,
}

impl OrderPass {
    /// Admits this frame's arrows; a hidden plan family admits none, so its labels go too.
    pub fn prepare(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        wraps: &[f32],
        shown: impl Fn(dashboard_core::protocol::PlanKind) -> bool,
    ) {
        self.native_plans = native::plans(&self.draws, camera, size, wraps, |draw, mode| {
            self.types
                .get(&(draw.arrow_type.clone(), draw.texture_choice))
                .filter(|_| shown(draw.kind))
                .filter(|resources| resources.compiled.contains_key(&mode))
                .map(|resources| &resources.arrow)
        });
    }

    /// Native labels follow arrow admission, independently of glyph bounds.
    pub fn labels(&self, mode: Mode) -> Vec<(usize, usize)> {
        self.native_plans
            .get(&mode)
            .into_iter()
            .flatten()
            .enumerate()
            .flat_map(|(view, plan)| plan.labels.iter().map(move |source| (view, *source)))
            .collect()
    }
    /// Builds the pipeline and uploads every arrow type's pages.
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
                // Append the effect suffix before lookup.
                let name = if mode == Mode::Seethrough {
                    format!("{}Seethrough", arrow.shader_technique)
                } else {
                    arrow.shader_technique.clone()
                };
                if mode == Mode::Seethrough && effect.select(&name).map_err(invalid)?.is_none() {
                    continue; // Absent optional technique skips mode 3.
                }
                let key = (name.clone(), mode);
                let program = if let Some(program) = programs.get(&key) {
                    Arc::clone(program)
                } else {
                    let program = Arc::new(crate::map::arrow_shader::Arrow::new(
                        device,
                        effect,
                        &name,
                        if mode == Mode::Shadow { 1 } else { samples },
                        if mode == Mode::Shadow {
                            crate::map::shadow::DEPTH_MAP_FORMAT
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

    /// Uploads one hour's ribbons. Called whenever the hour changes.
    pub fn upload(&mut self, device: &wgpu::Device, field: &OrderField) -> io::Result<()> {
        self.draws.clear();
        self.native_bytes = bytemuck::cast_slice(&field.native_vertices).into();
        if field.native_vertices.is_empty() {
            self.native_vertices = None;
            return Ok(());
        }
        self.native_vertices = Some(buffer(
            device,
            "native battle-plan ribbon vertices",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&field.native_vertices),
        )?);
        self.draws = field.draws.clone();
        Ok(())
    }

    fn prepare_bucket(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        native: NativeInputs<'_>,
        mode: Mode,
        time: f32,
        bucket: &[native::Draw],
    ) -> io::Result<Vec<NativeDraw<'_>>> {
        use installed_content_core::pdxshader::program::Stage;
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
                // Skip the entire loop when all RGB mask components are zero.
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

    /// Submit mode 2 after mesh/tree casters and before receivers; reached only through
    /// [`PlanSubmissions::cast_shadows`].
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
        pass.set_vertex_buffer(0, vertices.slice(..));
        for bucket in &plan.buckets {
            for draw in self.prepare_bucket(arena, native, Mode::Shadow, time, bucket)? {
                draw.program.begin(pass);
                draw.bound.bind(pass);
                for strip in &draw.draw.native_strips {
                    pass.draw(strip.clone(), 0..1);
                }
            }
        }
        Ok(())
    }

    /// Reached only through [`PlanSubmissions::draw`].
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
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("compiled map arrows"),
                    color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
                    depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_vertex_buffer(0, vertices.slice(..));
                for draw in draws {
                    draw.program.begin(&mut pass);
                    draw.bound.bind(&mut pass);
                    for strip in &draw.draw.native_strips {
                        pass.draw(strip.clone(), 0..1);
                    }
                }
            }
            if mode == Mode::Seethrough {
                continue;
            }
            // Clear the whole stencil after both masks in modes 0/1.
            let mut attachment = targets.depth_attachment(wgpu::LoadOp::Load);
            attachment.stencil_ops = Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0),
                store: wgpu::StoreOp::Store,
            });
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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

/// The admitted arrow families a frame submits. Both draws and shadow casters respect the
/// map interface switch; trade routes have their map mode's visibility, plans their own switch.
pub struct PlanSubmissions<'a, P = OrderPass> {
    shown: Option<&'a P>,
}

/// A shown plan pass, handed to the frame's shadow casting.
pub struct ShadowCasting<'a, P = OrderPass>(&'a P);

/// A shown plan pass, handed to the frame's drawing.
pub struct Drawing<'a, P = OrderPass>(&'a P);

impl<'a, P> PlanSubmissions<'a, P> {
    pub fn new(
        plans: Option<&'a P>,
        graphics: &dashboard_core::protocol::MapGraphics,
        overlay: &crate::map::scene::Overlay<'_>,
        trade_routes: bool,
    ) -> Self {
        Self {
            shown: plans.filter(|_| (graphics.battle_plans || trade_routes) && overlay.interface),
        }
    }

    /// Casts the shown plans' shadows with `cast`.
    pub fn cast_shadows(
        &self,
        cast: impl FnOnce(ShadowCasting<'a, P>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.shown
            .map_or(Ok(()), |plans| cast(ShadowCasting(plans)))
    }

    /// Draws the shown plans with `draw`.
    pub fn draw(&self, draw: impl FnOnce(Drawing<'a, P>) -> io::Result<()>) -> io::Result<()> {
        self.shown.map_or(Ok(()), |plans| draw(Drawing(plans)))
    }
}

impl ShadowCasting<'_> {
    /// Submits view `view`'s plan shadows after mesh and tree casters and before receivers.
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
    /// Draws view `view`'s plans in `mode`.
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

    #[test]
    fn plans_submit_their_shadows_and_draws_only_while_shown() {
        // A stand-in plan pass through the one gate the frame reaches the order pass's shadows
        // and draws by, counting the shadow casts and draws a frame submits.
        struct StandIn;
        let submitted = |graphics: dashboard_core::protocol::MapGraphics,
                         interface: bool,
                         trade_routes: bool| {
            let overlay = crate::map::scene::Overlay {
                interface,
                ..Default::default()
            };
            let frame = PlanSubmissions::new(Some(&StandIn), &graphics, &overlay, trade_routes);
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
        // A free view with its overlays hidden casts no plan shadows and draws no plans.
        assert_eq!(submitted(shown, false, false), (0, 0));
        let switched_off = dashboard_core::protocol::MapGraphics {
            battle_plans: false,
            ..shown
        };
        assert_eq!(submitted(switched_off, true, false), (0, 0));
        assert_eq!(submitted(switched_off, true, true), (1, 1));
        assert_eq!(submitted(switched_off, false, true), (0, 0));
        // A scene without plans submits none, and a failing submission reaches the frame.
        let overlay = crate::map::scene::Overlay::default();
        let none = PlanSubmissions::<StandIn>::new(None, &shown, &overlay, true);
        assert!(none.cast_shadows(|_| unreachable!()).is_ok());
        let failing = PlanSubmissions::new(Some(&StandIn), &shown, &overlay, false)
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

    #[test]
    fn the_slot_matches_the_shaders_constant_buffer() {
        assert_eq!(std::mem::size_of::<Slot>(), 128);
        assert_eq!(
            std::mem::size_of::<viewer_foundation::map::ribbon_content::Vertex>(),
            32
        );
    }
}
