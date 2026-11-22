use super::*;
use map_display_data::value_index::ValueIndex;
use std::collections::BTreeMap;
use viewer_gpu::textures::Bound;

pub struct ProvinceColors {
    pub bound: Bound,
    colors: Vec<u32>,
    palette: wgpu::Buffer,
    tiles: wgpu::Buffer,
    uniform: wgpu::Buffer,
    pipeline: Arc<wgpu::ComputePipeline>,
    group: wgpu::BindGroup,
    provinces: Arc<[Province]>,
    params: [u32; 8],
    pending: bool,
    dirty: Vec<bool>,
    last: ValueIndex<u32, u32>,
    dirty_tiles: Vec<[u32; 2]>,
}

impl ProvinceColors {
    pub fn new(device: &wgpu::Device, source: &Static) -> io::Result<Self> {
        let source = &source.strategic;
        let p = source.params;
        let tile_count = (p.columns as usize)
            .checked_mul(p.rows as usize)
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let colors = vec![0_u32; source.provinces.len()];
        let palette = buffer(
            device,
            "province stripe palette",
            bytemuck::cast_slice(&colors),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        )?;
        let tiles = zero_buffer(
            device,
            "province stripe dirty tiles",
            u64::from(p.columns) * u64::from(p.rows) * 8,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        )?;
        let params = [
            p.width,
            p.height,
            p.packed_ids,
            0,
            p.dispatch_stride,
            GRID_CELL,
            0,
            0,
        ];
        let uniform = buffer(
            device,
            "province stripe parameters",
            bytemuck::cast_slice(&params),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        )?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ProvinceSecondaryColorMap"),
            size: wgpu::Extent3d {
                width: p.width,
                height: p.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        });
        let output = texture.create_view(&wgpu::TextureViewDescriptor {
            usage: Some(wgpu::TextureUsages::STORAGE_BINDING),
            ..Default::default()
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
            ..Default::default()
        });
        let pipeline =
            viewer_runtime::device_shared::shared(device, "province stripe pixels", || {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("province stripe pixels"),
                    source: wgpu::ShaderSource::Wgsl(SHADER.into()),
                });
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("province stripe pixels"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
        let group = shader_pipeline::gpu::stats::create_bind_group(
            &device,
            &wgpu::BindGroupDescriptor {
                label: Some("province stripe pixels"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: source.ids.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: palette.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: tiles.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&output),
                    },
                ],
            },
        );
        Ok(Self {
            bound: {
                let format = texture.format();
                Bound::with_view(texture, view, format)
            },
            colors,
            palette,
            tiles,
            uniform,
            pipeline,
            group,
            provinces: Arc::clone(&source.provinces),
            params,
            pending: false,
            dirty: vec![false; tile_count],
            last: ValueIndex::default(),
            dirty_tiles: Vec::with_capacity(tile_count),
        })
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        values: &BTreeMap<u32, [u8; 4]>,
    ) -> io::Result<()> {
        let mut next = self.last.clone();
        for (&id, _) in self.last.iter() {
            if !values.contains_key(&id) {
                next = next.removed(&id)?;
            }
        }
        for (&id, &color) in values {
            if (id as usize) >= self.colors.len() {
                continue;
            }
            let color = u32::from_le_bytes(color);
            next = if color == 0 {
                next.removed(&id)?
            } else {
                next.inserted(id, color)?
            };
        }
        if next.ptr_eq(&self.last) {
            return Ok(());
        }
        let columns = self.params[0].div_ceil(GRID_CELL);
        next.visit_changes(&self.last, |&id, _, new| {
            let color = new.copied().unwrap_or(0);
            self.colors[id as usize] = color;
            shader_pipeline::gpu::stats::write_buffer(
                &queue,
                &self.palette,
                u64::from(id) * 4,
                bytemuck::bytes_of(&color),
            );
            let province = &self.provinces[id as usize];
            if province.pixels == 0 {
                return;
            }
            for y in province.bounds[1]..=province.bounds[3] {
                for x in province.bounds[0]..=province.bounds[2] {
                    let at = (y * columns + x) as usize;
                    if !self.dirty[at] {
                        self.dirty[at] = true;
                        self.dirty_tiles.push([x, y]);
                    }
                }
            }
        });
        self.last = next;
        if self.dirty_tiles.is_empty() {
            return Ok(());
        }
        self.params[3] = self.dirty_tiles.len() as u32;
        shader_pipeline::gpu::stats::write_buffer(
            &queue,
            &self.tiles,
            0,
            bytemuck::cast_slice(&self.dirty_tiles),
        );
        shader_pipeline::gpu::stats::write_buffer(
            &queue,
            &self.uniform,
            0,
            bytemuck::cast_slice(&self.params),
        );
        self.pending = true;
        Ok(())
    }

    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder) -> bool {
        if !self.pending {
            return false;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("province stripe pixels"),
            timestamp_writes: None,
        });
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline);
        shader_pipeline::gpu::stats::bind();
        pass.set_bind_group(0, &self.group, &[]);
        let count = self.params[3];
        let stride = self.params[4];
        pass.dispatch_workgroups(count.min(stride), count.div_ceil(stride), 1);
        true
    }

    pub fn submitted(&mut self) {
        self.pending = false;
        self.bound.written();
        let columns = self.params[0].div_ceil(GRID_CELL);
        for [x, y] in self.dirty_tiles.drain(..) {
            self.dirty[(y * columns + x) as usize] = false;
        }
    }
}

const SHADER: &str = r#"
struct Params { width:u32, height:u32, packed:u32, count:u32, stride:u32, tile:u32, spare1:u32, spare2:u32 }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> ids:array<u32>;
@group(0) @binding(2) var<storage,read> colors:array<u32>;
@group(0) @binding(3) var<storage,read> tiles:array<vec2<u32>>;
@group(0) @binding(4) var output:texture_storage_2d<rgba8unorm,write>;
@compute @workgroup_size(8,8)
fn main(@builtin(workgroup_id) group:vec3<u32>, @builtin(local_invocation_id) local:vec3<u32>) {
    let tile=group.x+group.y*p.stride;
    if tile>=p.count {return;}
    let origin=tiles[tile]*p.tile;
    for(var y=local.y;y<p.tile;y+=8u) {
        for(var x=local.x;x<p.tile;x+=8u) {
            let point=origin+vec2<u32>(x,y);
            if point.x>=p.width || point.y>=p.height {continue;}
            let at=point.y*p.width+point.x;
            var id:u32;
            if p.packed!=0u {id=(ids[at/2u]>>((at%2u)*16u))&65535u;} else {id=ids[at];}
            textureStore(output,vec2<i32>(point),unpack4x8unorm(colors[id]));
        }
    }
}
"#;
