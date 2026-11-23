use crate::terrain_shader::{SIDE, Stand};
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use std::io;
use viewer_gpu::gpu_contract::{DEPTH_FORMAT, SCENE_FORMAT, Targets, scoped};
use viewer_gpu::province_map::{HEIGHT_SCALE, WATER_HEIGHT};

pub(crate) const SHADER: &str = include_str!("unknown_hatch.wgsl");

const CELL: u32 = map_snow::SNOW_MUD_BLOCK;
const PULL: f32 = 0.05;
const TILE: u32 = SIDE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Ground,
    Water,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    tiles: Vec<u32>,
    columns: u32,
    grid: [u32; 2],
    covered: usize,
}

impl Coverage {
    pub fn new(unknown: &[u8], grid: [u32; 2]) -> Self {
        let per_tile = TILE / CELL;
        let columns = grid[0].div_ceil(per_tile);
        let mut tiles = vec![0; columns as usize * grid[1].div_ceil(per_tile) as usize];
        let cells = grid[0] as usize * grid[1] as usize;
        for (cell, _) in unknown
            .iter()
            .take(cells)
            .enumerate()
            .filter(|(_, flag)| **flag != 0)
        {
            let [x, z] = [cell as u32 % grid[0], cell as u32 / grid[0]];
            tiles[((z / per_tile) * columns + x / per_tile) as usize] += 1;
        }
        let covered = tiles.iter().filter(|&&count| count != 0).count();
        Self { tiles, columns, grid, covered }
    }

    pub fn counts(&self, grid: [u32; 2]) -> bool {
        self.grid == grid
    }

    pub fn covered(&self) -> bool {
        self.covered != 0
    }

    pub fn patch(&mut self, cell: u32, unknown: bool) {
        let per_tile = TILE / CELL;
        let [x, z] = [cell % self.grid[0], cell / self.grid[0]];
        let Some(count) = self
            .tiles
            .get_mut(((z / per_tile) * self.columns + x / per_tile) as usize)
        else {
            return;
        };
        if unknown {
            *count += 1;
            self.covered += usize::from(*count == 1);
        } else {
            *count -= 1;
            self.covered -= usize::from(*count == 0);
        }
    }

    pub fn instances(&self, drawn: &[Stand]) -> Vec<Instance> {
        let rows = self.tiles.len() as u32 / self.columns.max(1);
        drawn
            .iter()
            .filter(|block| {
                let [x, z] = block.origin.map(|origin| origin / TILE);
                let span = (SIDE * block.step).div_ceil(TILE);
                (z..(z + span).min(rows)).any(|row| {
                    (x..(x + span).min(self.columns))
                        .any(|column| self.tiles[(row * self.columns + column) as usize] != 0)
                })
            })
            .map(|block| {
                let snap = block
                    .snap
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (edge, &set)| bits | (u32::from(set) << edge));
                [block.origin[0], block.origin[1], block.step, snap, block.base]
            })
            .collect()
    }
}

pub type Instance = [u32; 5];

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct HatchParams {
    pub view_projection: [[f32; 4]; 4],
    pub look: [f32; 4],
    pub map: [u32; 4],
    pub surface: [f32; 4],
}

impl HatchParams {
    pub fn new(
        view_projection: [[f32; 4]; 4],
        look: [f32; 3],
        map: [u32; 2],
        grid: [u32; 2],
        surface: Surface,
    ) -> Self {
        Self {
            view_projection,
            look: [look[0], look[1], look[2], PULL],
            map: [map[0], map[1], grid[0], grid[1]],
            surface: [
                match surface {
                    Surface::Ground => 0.0,
                    Surface::Water => 1.0,
                },
                WATER_HEIGHT,
                HEIGHT_SCALE,
                CELL as f32,
            ],
        }
    }
}

pub struct UnknownHatch {
    pipeline: wgpu::RenderPipeline,
    indices: wgpu::Buffer,
    count: u32,
    samples: u32,
    ring: std::cell::RefCell<Ring>,
}

const PARAMS_STRIDE: u64 = 256;

#[derive(Default)]
struct Ring {
    slot: usize,
    params: Option<(wgpu::Buffer, usize)>,
    blocks: Option<(wgpu::Buffer, u64)>,
    used: u64,
    groups: Vec<wgpu::BindGroup>,
    views: Option<(wgpu::TextureView, wgpu::TextureView)>,
    streams: Vec<(wgpu::Buffer, wgpu::BindGroup, bool)>,
}

impl UnknownHatch {
    pub fn new(device: &wgpu::Device, samples: u32) -> Result<Self, String> {
        let triangles = crate::terrain_shader::indices([SIDE, SIDE]);
        let (pipeline, indices) = scoped(
            device,
            || "unknown snow and mud hatch pipeline".to_owned(),
            || {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("unknown snow/mud hatch"),
                    source: wgpu::ShaderSource::Wgsl(SHADER.into()),
                });
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("unknown snow/mud hatch"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("hatch_vertex"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: size_of::<Instance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![0 => Uint32x4, 1 => Uint32],
                        })],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("hatch_pixel"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: SCENE_FORMAT,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::COLOR,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState {
                        count: samples,
                        ..Default::default()
                    },
                    multiview_mask: None,
                    cache: None,
                });
                let indices = shader_pipeline::gpu::stats::create_buffer_init(
                    device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("unknown snow/mud hatch triangles"),
                        contents: bytemuck::cast_slice(&triangles),
                        usage: wgpu::BufferUsages::INDEX,
                    },
                );
                Ok((pipeline, indices))
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            pipeline,
            indices,
            count: triangles.len() as u32,
            samples,
            ring: Default::default(),
        })
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn begin_frame(&self) {
        let mut ring = self.ring.borrow_mut();
        ring.slot = 0;
        ring.used = 0;
        ring.streams.retain_mut(|(.., drawn)| std::mem::take(drawn));
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the device, the targets, the pass's textures, its blocks and its parameters"
    )]
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        heights: &wgpu::TextureView,
        unknown: &wgpu::TextureView,
        streams: &[(wgpu::Buffer, Vec<Instance>)],
        params: &HatchParams,
    ) -> io::Result<()> {
        if streams.iter().all(|(_, blocks)| blocks.is_empty()) {
            return Ok(());
        }
        let mut ring = self.ring.borrow_mut();
        let ring = &mut *ring;
        let slot = ring.slot;
        let bytes: Vec<u8> = streams
            .iter()
            .flat_map(|(_, blocks)| bytemuck::cast_slice::<Instance, u8>(blocks))
            .copied()
            .collect();
        let length = bytes.len() as u64;
        scoped(
            device,
            || "unknown snow and mud hatch bindings".to_owned(),
            || {
                if ring.params.as_ref().is_none_or(|(_, slots)| *slots <= slot) {
                    let slots = (slot + 1).next_power_of_two().max(4);
                    ring.params = Some((
                        device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("unknown snow/mud hatch parameters"),
                            size: slots as u64 * PARAMS_STRIDE,
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        slots,
                    ));
                    ring.groups.clear();
                }
                if ring
                    .blocks
                    .as_ref()
                    .is_none_or(|(_, size)| ring.used + length > *size)
                {
                    let size = (ring.used + length).next_power_of_two().max(1 << 16);
                    ring.blocks = Some((
                        device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("unknown snow/mud hatch blocks"),
                            size,
                            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        size,
                    ));
                    ring.used = 0;
                }
                if ring
                    .views
                    .as_ref()
                    .is_none_or(|(had_heights, had_unknown)| {
                        had_heights != heights || had_unknown != unknown
                    })
                {
                    ring.views = Some((heights.clone(), unknown.clone()));
                    ring.groups.clear();
                }
                let (params_buffer, _) = ring.params.as_ref().expect("made above");
                while ring.groups.len() <= slot {
                    let at = ring.groups.len() as u64 * PARAMS_STRIDE;
                    ring.groups
                        .push(shader_pipeline::gpu::stats::create_bind_group(
                            device,
                            &wgpu::BindGroupDescriptor {
                                label: Some("unknown snow/mud hatch"),
                                layout: &self.pipeline.get_bind_group_layout(0),
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::Buffer(
                                            wgpu::BufferBinding {
                                                buffer: params_buffer,
                                                offset: at,
                                                size: std::num::NonZeroU64::new(
                                                    size_of::<HatchParams>() as u64,
                                                ),
                                            },
                                        ),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(heights),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::TextureView(unknown),
                                    },
                                ],
                            },
                        ));
                }
                for (buffer, _) in streams {
                    match ring.streams.iter_mut().find(|(held, ..)| held == buffer) {
                        Some((.., drawn)) => *drawn = true,
                        None => ring.streams.push((
                            buffer.clone(),
                            shader_pipeline::gpu::stats::create_bind_group(
                                device,
                                &wgpu::BindGroupDescriptor {
                                    label: Some("unknown snow/mud hatch terrain heights"),
                                    layout: &self.pipeline.get_bind_group_layout(1),
                                    entries: &[wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: buffer.as_entire_binding(),
                                    }],
                                },
                            ),
                            true,
                        )),
                    }
                }
                Ok(())
            },
        )?;
        let (params_buffer, _) = ring.params.as_ref().expect("made above");
        let (blocks_buffer, _) = ring.blocks.as_ref().expect("made above");
        let first = ring.used;
        shader_pipeline::gpu::stats::write_buffer(
            queue,
            params_buffer,
            slot as u64 * PARAMS_STRIDE,
            bytemuck::bytes_of(params),
        );
        shader_pipeline::gpu::stats::write_buffer(queue, blocks_buffer, first, &bytes);
        ring.used += length;
        ring.slot += 1;
        let mut pass = viewer_scene_base::shader_scene::world_pass(encoder, targets, "unknown snow/mud hatch");
        let pass = &mut pass;
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline);
        shader_pipeline::gpu::stats::bind();
        pass.set_bind_group(0, &ring.groups[slot], &[]);
        shader_pipeline::gpu::stats::state();
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        let mut at = first;
        for (buffer, blocks) in streams.iter().filter(|(_, blocks)| !blocks.is_empty()) {
            let (_, group, _) = ring
                .streams
                .iter()
                .find(|(held, ..)| held == buffer)
                .expect("bound above");
            let end = at + size_of_val(blocks.as_slice()) as u64;
            shader_pipeline::gpu::stats::bind();
            pass.set_bind_group(1, group, &[]);
            shader_pipeline::gpu::stats::state();
            pass.set_vertex_buffer(0, blocks_buffer.slice(at..end));
            shader_pipeline::gpu::stats::draw();
            pass.draw_indexed(0..self.count, 0, 0..blocks.len() as u32);
            at = end;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain_shader::{Patch, heights_into, positions};
    use viewer_gpu::province_map::HeightField;

    fn sampled(block: [u32; 4], map: [u32; 2], heights: &[u8]) -> Vec<[i16; 2]> {
        let mut stream = Vec::new();
        let patch = Patch {
            origin: [block[0], block[1]],
            step: block[2],
        };
        heights_into(&positions(), heights, map, patch, &mut stream);
        stream
    }

    fn placed(block: [u32; 4], index: u32, map: [u32; 2], heights: &[[i16; 2]]) -> [f32; 3] {
        let mut local = [index % (SIDE + 1), index / (SIDE + 1)];
        if (local[0] + local[1]) % 2 == 1 {
            if (local[1] == 0 && block[3] & 1 != 0) || (local[1] == SIDE && block[3] & 2 != 0) {
                local[0] += 1;
            } else if (local[0] == 0 && block[3] & 4 != 0)
                || (local[0] == SIDE && block[3] & 8 != 0)
            {
                local[1] += 1;
            }
        }
        let local: [u32; 2] = std::array::from_fn(|axis| {
            local[axis].min(((map[axis] - block[axis]) / block[2]).min(SIDE))
        });
        let stored = heights[(local[1] * (SIDE + 1) + local[0]) as usize][0];
        [
            (block[0] + local[0] * block[2]) as f32,
            f32::from(stored) / 10.0 * HEIGHT_SCALE,
            (block[1] + local[1] * block[2]) as f32,
        ]
    }

    fn shows(surface: Surface, ground: f32) -> bool {
        (ground < WATER_HEIGHT) == (surface == Surface::Water)
    }

    fn heightmap(height: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        (0..SIDE)
            .flat_map(|row| (0..SIDE).map(move |x| (x, SIDE - 1 - row)))
            .map(|(x, z)| height(x, z))
            .collect()
    }

    fn triangle_at(
        triangle: &[u16],
        block: [u32; 4],
        map: [u32; 2],
        heights: &[[i16; 2]],
    ) -> [[f32; 3]; 3] {
        std::array::from_fn(|corner| placed(block, u32::from(triangle[corner]), map, heights))
    }

    fn centre(corners: &[[f32; 3]; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| corners.iter().map(|corner| corner[axis]).sum::<f32>() / 3.0)
    }

    #[test]
    fn a_cells_hatch_stands_on_the_terrain_at_its_own_vertices_not_a_neighbours_peak() {
        let heights = heightmap(|x, z| if (5..8).contains(&x) && z < 4 { 200 } else { 120 });
        let map = [SIDE, SIDE];
        let triangles = crate::terrain_shader::indices([SIDE, SIDE]);
        let stream = sampled([0, 0, 1, 0], map, &heights);
        let mut checked = 0;
        for triangle in triangles.chunks_exact(3) {
            let corners = triangle_at(triangle, [0, 0, 1, 0], map, &stream);
            let [x, _, z] = centre(&corners);
            if x < CELL as f32 && z < CELL as f32 {
                for corner in corners {
                    assert!((corner[1] - 12.0).abs() < 1e-4, "{corner:?}");
                }
                checked += 1;
            }
        }
        assert_eq!(checked, (CELL * CELL * 2) as usize);
    }

    #[test]
    fn the_hatch_follows_the_terrains_triangles_over_uneven_ground() {
        let heights = heightmap(|x, z| 100 + ((x * 37 + z * 61 + x * z * 7) % 150) as u8);
        let field = HeightField {
            width: SIDE,
            height: SIDE,
            values: &heights,
        };
        let map = [SIDE, SIDE];
        let stream = sampled([0, 0, 1, 0], map, &heights);
        for triangle in crate::terrain_shader::indices([SIDE, SIDE]).chunks_exact(3) {
            let corners = triangle_at(triangle, [0, 0, 1, 0], map, &stream);
            for [x, y, z] in corners {
                assert!((y - field.surface_height(x, z)).abs() < 1e-4, "{x} {z}");
            }
            let [x, y, z] = centre(&corners);
            assert!((y - field.surface_height(x, z)).abs() < 1e-4, "{x} {z}");
        }
    }

    #[test]
    fn coarser_and_snapped_blocks_stand_where_the_terrains_do() {
        let heights = heightmap(|x, z| (x + 2 * z) as u8);
        let map = [SIDE, SIDE];
        let height = |x: u32, z: u32| f32::from((x + 2 * z) as u8) * HEIGHT_SCALE;
        let coarse = [0, 0, 2, 0];
        let stream = sampled(coarse, map, &heights);
        assert_eq!(
            placed(coarse, SIDE + 1 + 3, map, &stream),
            [6.0, height(6, 2), 2.0]
        );
        let snapped = [0, 0, 2, 1];
        assert_eq!(placed(snapped, 1, map, &stream), [4.0, height(4, 0), 0.0]);
        assert_eq!(height(4, 0), f32::from(stream[1][1]) / 10.0 * HEIGHT_SCALE);
        assert_eq!(placed(coarse, 1, map, &stream), [2.0, height(2, 0), 0.0]);
        let west = SIDE + 1;
        assert_eq!(placed(snapped, west, map, &stream), [0.0, height(0, 2), 2.0]);
        let east = [0, 0, 1, 8];
        assert_eq!(
            placed(east, west + SIDE, map, &sampled(east, map, &heights)),
            [64.0, height(0, 2), 2.0]
        );
    }

    #[test]
    fn the_hatch_stands_on_the_coast_capped_heights_the_terrain_draws() {
        use eframe::wgpu::util::DeviceExt;
        let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
        let map = [256u32, 128];
        let bytes: Vec<u8> = (0..map[1])
            .rev()
            .flat_map(|z| {
                (0..map[0]).map(move |x| if x < 128 && x % 2 == 1 && z % 2 == 1 { 40 } else { 200 })
            })
            .collect();
        let coast =
            crate::terrain_coast::Coast::new(&bytes, map, WATER_HEIGHT, HEIGHT_SCALE, 256);
        let [west, east] = [[0, 0, 2, 0], [128, 0, 2, 0]];
        let mut streams = Vec::new();
        for block in [west, east] {
            let mut heights = sampled(block, map, &bytes);
            coast.cap(&positions(), [block[0], block[1]], block[2], &mut heights);
            streams.extend(heights);
        }
        let side = (SIDE + 1) as usize;
        assert!(streams[..side * side].iter().all(|pair| *pair == [940, 940]), "west capped");
        assert_eq!(streams[side * side + side * 32 + 32], [2000, 2000], "east as sampled");
        let stream = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain heights"),
            contents: bytemuck::cast_slice(&streams),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let texture = |size: [u32; 2], data: &[u8]| {
            device
                .create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: None,
                        size: viewer_gpu::gpu_contract::extent(size),
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::R8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    data,
                )
                .create_view(&Default::default())
        };
        let grid = map.map(|side| side / CELL);
        let heights = texture(map, &bytes);
        let unknown = texture(grid, &vec![255; (grid[0] * grid[1]) as usize]);
        let size = [128, 64];
        let targets = Targets::new(&device, size, size, 1, viewer_gpu::gpu_contract::SeaBed::Half);
        let hatch = UnknownHatch::new(&device, 1).unwrap();
        let view_projection = [
            [1.0 / 128.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, -1.0 / 32.0, 0.0],
            [0.0, 1.0 / 64.0, 0.0, 0.0],
            [-1.0, -1.0, 1.0, 1.0],
        ];
        let params = HatchParams::new(view_projection, [0.0; 3], map, grid, Surface::Ground);
        let base = (side * side) as u32;
        let blocks = vec![
            [west[0], west[1], west[2], 0, 0],
            [east[0], east[1], east[2], 0, base],
        ];
        let shows = |height: f32| {
            hatch.begin_frame();
            let mut encoder = device.create_command_encoder(&Default::default());
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Clear(
                    wgpu::Color::TRANSPARENT,
                )))],
                depth_stencil_attachment: Some(
                    targets.depth_attachment(wgpu::LoadOp::Clear(1.0 - height / 32.0)),
                ),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            }));
            let streams = [(stream.clone(), blocks.clone())];
            hatch
                .draw(
                    &device,
                    &queue,
                    &mut encoder,
                    &targets,
                    &heights,
                    &unknown,
                    &streams,
                    &params,
                )
                .unwrap();
            queue.submit([encoder.finish()]);
            let pixels = viewer_gpu::gpu_contract::read_texture(
                &device,
                &queue,
                targets.scene_texture(),
                8,
            )
            .unwrap();
            let drawn = |columns: std::ops::Range<usize>| {
                (4..60).any(|row| {
                    columns.clone().any(|column| {
                        let at = (row * size[0] as usize + column) * 8;
                        pixels[at..at + 6].iter().any(|&byte| byte != 0)
                    })
                })
            };
            [drawn(4..60), drawn(68..124)]
        };
        assert_eq!(shows(9.35), [true, true], "the capped land keeps its hatch");
        assert_eq!(shows(9.45), [false, true], "the west stands at the cap, 9.4");
        assert_eq!(shows(19.95), [false, true]);
        assert_eq!(shows(20.05), [false, false], "the east stands at the heightmap's 20");
    }

    #[test]
    fn quads_past_the_maps_edge_collapse_onto_it() {
        let map = [96, SIDE];
        let heights = vec![120; (map[0] * map[1]) as usize];
        let stream = sampled([64, 0, 1, 0], map, &heights);
        assert_eq!(placed([64, 0, 1, 0], 40, map, &stream)[0], 96.0);
        assert_eq!(placed([64, 0, 1, 0], 20, map, &stream)[0], 84.0);
    }

    #[test]
    fn each_hatch_keeps_to_its_own_surface() {
        let sea = WATER_HEIGHT - HEIGHT_SCALE;
        assert!(!shows(Surface::Ground, sea));
        assert!(shows(Surface::Water, sea));
        assert!(shows(Surface::Ground, WATER_HEIGHT));
        assert!(!shows(Surface::Water, WATER_HEIGHT));
        let [low, high] = [WATER_HEIGHT - 1.0, WATER_HEIGHT + 1.0];
        for step in 0..=10 {
            let ground = low + (high - low) * step as f32 / 10.0;
            assert_ne!(shows(Surface::Ground, ground), shows(Surface::Water, ground));
        }
    }

    #[test]
    fn only_blocks_over_an_unknown_cell_are_drawn() {
        let grid = [64, 32];
        let mut unknown = vec![0; 64 * 32];
        assert!(!Coverage::new(&unknown, grid).covered());
        unknown[3 * 64 + 17] = 255;
        let coverage = Coverage::new(&unknown, grid);
        assert!(coverage.covered());
        let block = |origin: [u32; 2], step: u32, snap: [bool; 4], base: u32| Stand {
            origin,
            step,
            snap,
            base,
        };
        let drawn = [
            block([0, 0], 1, [false; 4], 0),
            block([64, 0], 1, [true, false, false, true], 4225),
            block([128, 0], 2, [false; 4], 8450),
            block([0, 0], 2, [false, true, false, false], 12675),
        ];
        assert_eq!(
            coverage.instances(&drawn),
            [[64, 0, 1, 0b1001, 4225], [0, 0, 2, 0b0010, 12675]]
        );
    }

    #[test]
    fn a_patched_coverage_counts_what_a_rebuilt_one_counts() {
        let grid = [64, 32];
        let mut unknown = vec![0u8; 64 * 32];
        for cell in [0, 17, 3 * 64 + 17, 3 * 64 + 20, 31 * 64 + 63] {
            unknown[cell] = 255;
        }
        let mut coverage = Coverage::new(&unknown, grid);
        assert!(coverage.counts(grid));
        for (cell, now) in [
            (3 * 64 + 17, false),
            (3 * 64 + 20, false),
            (0, false),
            (3 * 64 + 20, true),
            (5 * 64 + 1, true),
        ] {
            unknown[cell] = u8::from(now) * 255;
            coverage.patch(cell as u32, now);
            assert_eq!(coverage, Coverage::new(&unknown, grid), "after cell {cell}");
        }
        for cell in 0..64 * 32 {
            if unknown[cell] != 0 {
                unknown[cell] = 0;
                coverage.patch(cell as u32, false);
            }
        }
        assert!(!coverage.covered());
        assert_eq!(coverage, Coverage::new(&unknown, grid));
    }

    #[test]
    fn parameters_carry_the_camera_map_and_surface_the_shader_reads() {
        viewer_scene_base::shader_validation::validate("unknown_hatch", SHADER);
        let mut matrix = [[0.0; 4]; 4];
        for (row, values) in matrix.iter_mut().enumerate() {
            values[row] = 1.0 + row as f32;
        }
        let look = [0.0, -0.6, 0.8];
        let ground = HatchParams::new(matrix, look, [5632, 2048], [1408, 512], Surface::Ground);
        assert_eq!(ground.view_projection, matrix);
        assert_eq!(ground.look, [0.0, -0.6, 0.8, PULL]);
        assert_eq!(ground.map, [5632, 2048, 1408, 512]);
        assert_eq!(ground.surface, [0.0, WATER_HEIGHT, HEIGHT_SCALE, CELL as f32]);
        let water = HatchParams::new(matrix, look, [5632, 2048], [1408, 512], Surface::Water);
        assert_eq!(water.surface[0], 1.0);
        assert_eq!(size_of::<HatchParams>(), 112);
    }

    #[test]
    fn invalid_pipeline_samples_fail_with_named_validation_error() {
        let (_serial, device, _queue) = viewer_gpu::test_support::gpu::device();
        let error = UnknownHatch::new(&device, 3)
            .err()
            .expect("unsupported MSAA sample count must fail validation");
        assert!(
            error.contains("unknown snow and mud hatch pipeline"),
            "{error}"
        );
    }
}
