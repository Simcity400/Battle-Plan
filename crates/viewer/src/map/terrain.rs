//! The terrain grid and the `terrain` and `underwater` pipelines.

use std::io;

use eframe::wgpu;

use super::{
    camera::{Camera, MAP_SIZE_Y},
    scene::{DEPTH_FORMAT, Targets, module, sampler_entry, texture_entry},
    shadow::BUFFER_FORMAT,
    textures::{Bound, Samplers},
};

/// Quads the visible grid may hold before the step doubles.
///
/// At the reference camera one map pixel is 5.73 screen pixels and the visible
/// ground is about 464 x 252 units, so a step of 1 costs about 140k quads with
/// the margin (R1 section 5.2). The ceiling only bites when the camera climbs:
/// at `CAMERA_MAX_HEIGHT` the visible ground is roughly 4400 x 2475 units,
/// which a step of 1 would turn into 10.9M quads.
pub const MAX_QUADS: u32 = 600_000;

/// World units the visible rectangle grows by on every side.
///
/// Terrain reaches 25.5 units above the water plane, and at the shallowest
/// pitch the engine allows (53.5 degrees below the horizon at height 50) a
/// 16-unit column leans about 12 units past its own ground position, so 16
/// units of margin keeps everything that can appear inside the frame.
pub const MARGIN: f32 = 16.0;

/// The uniform grid one frame draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// World `(x, z)` of the first vertex, aligned to a multiple of the step so
    /// the sampled heightmap texels do not shimmer as the camera moves.
    pub origin: [f32; 2],
    /// Spacing in map pixels; always a power of two.
    pub step: f32,
    /// Vertex columns and rows.
    pub counts: [u32; 2],
}

impl Grid {
    /// Quads, which the vertex shader expands to six vertices each.
    pub fn quads(&self) -> u32 {
        self.counts[0].saturating_sub(1) * self.counts[1].saturating_sub(1)
    }
}

/// Covers `rect` with the finest power-of-two grid that stays within `max_quads`.
///
/// `rect` is `(x0, z0, x1, z1)` in world units. The `z` range is clipped to the
/// map, which has no terrain outside it; `x` is left alone because the map wraps
/// and the shaders wrap their own lookups.
pub fn grid(rect: [f32; 4], max_quads: u32) -> Grid {
    let z0 = rect[1].max(-MARGIN);
    let z1 = rect[3].min(MAP_SIZE_Y - 1.0 + MARGIN).max(z0);
    let mut step = 1.0f32;
    loop {
        let origin = [(rect[0] / step).floor() * step, (z0 / step).floor() * step];
        let counts = [
            ((rect[2] - origin[0]) / step).ceil().max(1.0) as u32 + 1,
            ((z1 - origin[1]) / step).ceil().max(1.0) as u32 + 1,
        ];
        let candidate = Grid {
            origin,
            step,
            counts,
        };
        if candidate.quads() <= max_quads || step >= 256.0 {
            return candidate;
        }
        step *= 2.0;
    }
}

/// The grid for one camera and viewport.
pub fn grid_for(camera: Camera, viewport: [u32; 2]) -> Grid {
    match camera.visible_ground(viewport, MARGIN) {
        Some(rect) => grid(rect, MAX_QUADS),
        // A camera that cannot see the water plane cannot see the map either.
        None => Grid {
            origin: [0.0, 0.0],
            step: 1.0,
            counts: [0, 0],
        },
    }
}

/// Everything the two pipelines bind, in the order `pdxmap.shader` declares.
pub struct TerrainInputs<'a> {
    pub atlas: &'a Bound,
    pub height_normal: &'a Bound,
    pub colormap: &'a Bound,
    pub snow: &'a Bound,
    pub atlas_normal: &'a Bound,
    pub terrain_ids: &'a Bound,
    pub secondary: &'a Bound,
    pub snow_mud: &'a Bound,
    pub city_lights: &'a Bound,
    pub mud_normal: &'a Bound,
    pub mud_diffuse: &'a Bound,
    pub gradient1: &'a Bound,
    pub gradient2: &'a Bound,
    pub underwater: &'a Bound,
    pub colormap_water: &'a Bound,
    pub heightmap: &'a Bound,
}

/// The lit terrain and the underwater sea bed, which share a vertex shader.
pub struct TerrainPass {
    terrain: wgpu::RenderPipeline,
    underwater: wgpu::RenderPipeline,
    /// `pdxmap.shader`'s `terrainunlit`, the terrain half of the shadow
    /// pre-pass (`map::shadow`). Same grid, same vertex stage, a different
    /// target and no multisampling.
    unlit: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
}

impl TerrainPass {
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        globals: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let mut entries = (0..15)
            .map(|binding| {
                texture_entry(
                    binding,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                )
            })
            .collect::<Vec<_>>();
        // The heightmap is fetched per vertex, so it costs the fragment stage
        // nothing against the sixteen-texture limit the pass otherwise fills.
        entries.push(texture_entry(
            15,
            wgpu::ShaderStages::VERTEX,
            wgpu::TextureViewDimension::D2,
        ));
        for binding in 16..19 {
            entries.push(sampler_entry(binding));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain inputs"),
            entries: &entries,
        });
        let module = module(device, "terrain", include_str!("terrain.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(globals), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, entry, depth_compare, format, samples| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        // The pixel shader's alpha is the city-light bloom mask.
                        // `draw_postfx=no` means nothing consumes it (R1 4.1),
                        // so the frame keeps the opaque alpha it was cleared to.
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let unlit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrainunlit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("terrain_unlit_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: BUFFER_FORMAT,
                    // `BlendEnable = no`, `WriteMask = "RED|GREEN|BLUE|ALPHA"`
                    // (`pdxmap.shader:474-479`).
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            terrain: pipeline(
                "terrain",
                "terrain_main",
                Some(wgpu::CompareFunction::LessEqual),
                format,
                samples,
            ),
            underwater: pipeline(
                "underwater",
                "underwater_main",
                // Frame33840 event 61 uses LESS_EQUAL for the refraction draw.
                Some(wgpu::CompareFunction::LessEqual),
                super::water::REFRACTION_FORMAT,
                1,
            ),
            unlit,
            layout,
            bind_group: None,
        })
    }

    /// Rebuilds the bind group; needed whenever a bound texture is replaced.
    pub fn rebind(
        &mut self,
        device: &wgpu::Device,
        samplers: &Samplers,
        inputs: TerrainInputs<'_>,
    ) {
        let views = [
            &inputs.atlas.view,
            &inputs.height_normal.view,
            &inputs.colormap.view,
            &inputs.snow.view,
            &inputs.atlas_normal.view,
            &inputs.terrain_ids.view,
            &inputs.secondary.view,
            &inputs.snow_mud.view,
            &inputs.city_lights.view,
            &inputs.mud_normal.view,
            &inputs.mud_diffuse.view,
            &inputs.gradient1.view,
            &inputs.gradient2.view,
            &inputs.underwater.view,
            &inputs.colormap_water.view,
            &inputs.heightmap.view,
        ];
        let mut bindings = views
            .iter()
            .enumerate()
            .map(|(binding, view)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect::<Vec<_>>();
        for (offset, sampler) in [
            &samplers.linear_wrap,
            &samplers.point_clamp,
            &samplers.linear_clamp,
        ]
        .into_iter()
        .enumerate()
        {
            bindings.push(wgpu::BindGroupEntry {
                binding: 16 + offset as u32,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain inputs"),
            layout: &self.layout,
            entries: &bindings,
        }));
    }

    /// Draws the terrain grid, including beneath water, with `terrainunlit`
    /// into the shadow pre-pass the caller opened.
    pub fn draw_unlit(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        globals: &wgpu::BindGroup,
        grid: Grid,
    ) {
        let vertices = grid.quads() * 6;
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        if vertices == 0 {
            return;
        }
        pass.set_pipeline(&self.unlit);
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.draw(0..vertices, 0..1);
    }

    /// Draws the sea bed into the separate native-format water input.
    pub fn draw_refraction(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        globals: &wgpu::BindGroup,
        grid: Grid,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("water refraction"),
            color_attachments: &[Some(targets.refraction_attachment())],
            depth_stencil_attachment: Some(targets.refraction_depth_attachment()),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_pipeline(&self.underwater);
        pass.draw(0..grid.quads() * 6, 0..1);
    }

    /// Draws lit terrain, including the floor behind translucent water.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        globals: &wgpu::BindGroup,
        grid: Grid,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("terrain"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Clear(
                wgpu::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            )))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Clear(1.0))),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let vertices = grid.quads() * 6;
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        if vertices == 0 {
            return;
        }
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_pipeline(&self.terrain);
        pass.draw(0..vertices, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// World `(x, z)` of the last vertex the grid places.
    fn far_corner(grid: Grid) -> [f32; 2] {
        [
            grid.origin[0] + (grid.counts[0] - 1) as f32 * grid.step,
            grid.origin[1] + (grid.counts[1] - 1) as f32 * grid.step,
        ]
    }

    #[test]
    fn the_grid_covers_the_rectangle_at_step_one_when_it_is_small() {
        let grid = grid([100.4, 200.9, 140.0, 220.0], MAX_QUADS);
        assert_eq!(grid.step, 1.0);
        assert_eq!(grid.origin, [100.0, 200.0]);
        // 100..140 needs 41 columns, 200..220 needs 21 rows.
        assert_eq!(grid.counts, [41, 21]);
        let far = far_corner(grid);
        assert!(far[0] >= 140.0 && far[1] >= 220.0);
        assert_eq!(grid.quads(), 40 * 20);
    }

    #[test]
    fn the_step_doubles_until_the_quad_budget_holds() {
        // The full-map view at CAMERA_MAX_HEIGHT: about 4400 x 2475 units,
        // of which only the map's own 2048 rows plus the margin survive.
        let rect = [0.0, 0.0, 4400.0, 2475.0];
        let coarse = grid(rect, MAX_QUADS);
        assert!(coarse.quads() <= MAX_QUADS, "{coarse:?}");
        // Step 2 would be 2.27M quads and step 4 is 567_600, just inside.
        assert_eq!(coarse.step, 4.0);
        assert_eq!(coarse.quads(), 1100 * 516);
        // A four-times budget admits step 2, which is what makes the rule a
        // doubling against the budget and not a fixed choice.
        assert_eq!(grid(rect, MAX_QUADS * 4).step, 2.0);
        // The step never runs away, even on an impossible budget.
        assert_eq!(grid(rect, 1).step, 256.0);
    }

    #[test]
    fn the_origin_stays_on_a_multiple_of_the_step() {
        let grid = grid([-13.0, 7.0, 4400.0, 2475.0], MAX_QUADS);
        assert_eq!(grid.origin[0] % grid.step, 0.0);
        assert_eq!(grid.origin[1] % grid.step, 0.0);
        assert!(grid.origin[0] <= -13.0 && grid.origin[1] <= 7.0);
    }

    #[test]
    fn the_grid_is_clipped_to_the_map_in_z_but_not_in_x() {
        let grid = grid([-400.0, -900.0, 200.0, 9000.0], MAX_QUADS);
        assert!(grid.origin[0] < 0.0, "x is never clipped: the map wraps");
        assert!(grid.origin[1] >= -MARGIN - grid.step);
        let top = far_corner(grid)[1];
        assert!(top <= MAP_SIZE_Y - 1.0 + MARGIN + grid.step, "{top}");
    }

    #[test]
    fn the_reference_camera_grid_is_about_a_hundred_thousand_quads() {
        let camera = Camera {
            center: [3010.0, 1568.0],
            height: 310.178,
        };
        let grid = grid_for(camera, [2048, 1152]);
        assert_eq!(grid.step, 1.0);
        assert!(
            (100_000..250_000).contains(&grid.quads()),
            "{} quads",
            grid.quads()
        );
    }
}
