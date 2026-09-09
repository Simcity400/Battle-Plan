//! The map pipeline is shared by the window and PNG export. Egui only displays its texture.

use std::{
    fs::File,
    io,
    io::BufWriter,
    path::Path,
    sync::{Arc, mpsc},
};

use bytemuck::{Pod, Zeroable};
use eframe::wgpu::{self, util::DeviceExt};

// Egui expects gamma-encoded RGB in an unorm view. Keeping the same bytes for PNG export
// also preserves the game's country RGB colors without a second sRGB conversion.
const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub fn configuration() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut configuration = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut configuration.wgpu_setup {
        setup.device_descriptor = Arc::new(device_descriptor);
    }
    configuration
}

fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let limits = adapter.limits();
    wgpu::DeviceDescriptor {
        label: Some("Battle Plan viewer"),
        required_limits: wgpu::Limits {
            max_texture_dimension_1d: limits.max_texture_dimension_1d,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Center in province-image pixels, with the image origin at the top left.
    pub center: [f32; 2],
    pub vertical_span: f32,
}

impl Camera {
    pub fn fit(map: [u32; 2], viewport: [u32; 2]) -> Self {
        let aspect = viewport[0].max(1) as f32 / viewport[1].max(1) as f32;
        Self {
            center: [map[0] as f32 * 0.5, map[1] as f32 * 0.5],
            vertical_span: (map[1] as f32).max(map[0] as f32 / aspect),
        }
    }

    /// A positive drag moves the image right/down, so its camera moves left/up.
    pub fn pan_pixels(&mut self, delta: [f32; 2], viewport: [u32; 2]) {
        let scale = self.vertical_span / viewport[1].max(1) as f32;
        self.center[0] -= delta[0] * scale;
        self.center[1] -= delta[1] * scale;
    }

    /// Factor above one zooms in while preserving the map position under the pointer.
    pub fn zoom_at(&mut self, factor: f32, pointer_fraction: [f32; 2], viewport: [u32; 2]) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let next = (self.vertical_span / factor).clamp(2.0, 100_000.0);
        let difference = self.vertical_span - next;
        let aspect = viewport[0].max(1) as f32 / viewport[1].max(1) as f32;
        self.center[0] += (pointer_fraction[0] - 0.5) * difference * aspect;
        self.center[1] += (pointer_fraction[1] - 0.5) * difference;
        self.vertical_span = next;
    }

    fn validate(self) -> io::Result<()> {
        if self.center.iter().all(|v| v.is_finite())
            && self.vertical_span.is_finite()
            && self.vertical_span > 0.0
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "camera center must be finite and span must be positive",
            ))
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    center: [f32; 2],
    span: [f32; 2],
    viewport: [f32; 2],
    map_size: [f32; 2],
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

pub struct MapRenderer {
    map_size: [u32; 2],
    lookup_len: u32,
    colors: wgpu::Texture,
    owners: wgpu::Texture,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    target: Option<Target>,
    generation: u64,
    has_colors: bool,
}

impl MapRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        province_ids: &[u32],
    ) -> io::Result<Self> {
        validate_size([width, height], device.limits().max_texture_dimension_2d)?;
        if u64::from(width) * u64::from(height) != province_ids.len() as u64 {
            return Err(invalid(
                "province image length does not match its dimensions",
            ));
        }
        let lookup_len = province_ids
            .iter()
            .max()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("province ID exceeds the lookup range"))?;
        if lookup_len > device.limits().max_texture_dimension_1d {
            return Err(invalid(format!(
                "province lookup needs {lookup_len} entries; GPU device limit is {}",
                device.limits().max_texture_dimension_1d
            )));
        }
        let provinces = texture(
            device,
            "province IDs",
            [width, height],
            wgpu::TextureDimension::D2,
            wgpu::TextureFormat::R32Uint,
        );
        upload(
            queue,
            &provinces,
            [width, height],
            bytemuck::cast_slice(province_ids),
        );
        let colors = texture(
            device,
            "province colors",
            [lookup_len, 1],
            wgpu::TextureDimension::D1,
            OUTPUT_FORMAT,
        );
        let owners = texture(
            device,
            "province owner IDs",
            [lookup_len, 1],
            wgpu::TextureDimension::D1,
            wgpu::TextureFormat::R32Uint,
        );
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("map camera"),
            contents: bytemuck::bytes_of(&CameraUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("political map"),
            source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("political map"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("political map inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &provinces.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &colors.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &owners.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        Ok(Self {
            map_size: [width, height],
            lookup_len,
            colors,
            owners,
            uniform,
            bind_group,
            pipeline,
            target: None,
            generation: 0,
            has_colors: false,
        })
    }

    pub fn set_colors(
        &mut self,
        queue: &wgpu::Queue,
        colors: &[[u8; 4]],
        owner_keys: &[u32],
    ) -> io::Result<()> {
        if colors.len() != self.lookup_len as usize || owner_keys.len() != colors.len() {
            return Err(invalid(format!(
                "province colors and owners must each have {} entries indexed by province ID",
                self.lookup_len
            )));
        }
        upload(
            queue,
            &self.colors,
            [self.lookup_len, 1],
            bytemuck::cast_slice(colors),
        );
        upload(
            queue,
            &self.owners,
            [self.lookup_len, 1],
            bytemuck::cast_slice(owner_keys),
        );
        self.has_colors = true;
        Ok(())
    }

    /// Render to the same offscreen texture used by both egui and PNG readback.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        camera: Camera,
    ) -> io::Result<()> {
        camera.validate()?;
        validate_size(size, device.limits().max_texture_dimension_2d)?;
        if !self.has_colors {
            return Err(invalid("map frame colors have not been supplied"));
        }
        if self
            .target
            .as_ref()
            .is_none_or(|target| target.size != size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("map output"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OUTPUT_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.target = Some(Target {
                texture,
                view,
                size,
            });
            self.generation += 1;
        }
        let uniform = CameraUniform {
            center: camera.center,
            span: [
                camera.vertical_span * size[0] as f32 / size[1] as f32,
                camera.vertical_span,
            ],
            viewport: [size[0] as f32, size[1] as f32],
            map_size: [self.map_size[0] as f32, self.map_size[1] as f32],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("map render"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("political map"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.view().expect("target created above"),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Egui must update its native texture registration whenever this value changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|target| &target.view)
    }

    fn render_banner(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
    ) -> io::Result<()> {
        use eframe::{egui, egui_wgpu};

        let target = self
            .target
            .as_ref()
            .ok_or_else(|| invalid("render a map before adding a banner"))?;
        let [width, height] = target.size;
        let context = egui::Context::default();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                ..Default::default()
            },
            |ui| {
                let painter = ui.painter();
                let galley = painter.layout(
                    format!("UNVERIFIED CONTENT\n{text}"),
                    egui::FontId::proportional(16.0),
                    egui::Color32::BLACK,
                    (width as f32 - 24.0).max(1.0),
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width as f32, galley.size().y + 20.0),
                    ),
                    0.0,
                    egui::Color32::from_rgb(238, 173, 49),
                );
                painter.galley(egui::pos2(12.0, 10.0), galley, egui::Color32::BLACK);
            },
        );
        let jobs = context.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: target.size,
            pixels_per_point: output.pixels_per_point,
        };
        let mut renderer = egui_wgpu::Renderer::new(
            device,
            OUTPUT_FORMAT,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                renderer.update_texture(device, queue, id, &delta);
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("content verification banner"),
        });
        let commands = renderer.update_buffers(device, queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("content verification banner"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            renderer.render(&mut pass, &jobs, &screen);
        }
        queue.submit(commands.into_iter().chain([encoder.finish()]));
        for id in output.textures_delta.free.drain() {
            renderer.free_texture(&id);
        }
        Ok(())
    }

    fn write_png(&self, device: &wgpu::Device, queue: &wgpu::Queue, path: &Path) -> io::Result<()> {
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| invalid("render a map before saving PNG"))?;
        let [width, height] = target.size;
        let row_bytes = width * 4;
        let padded_row_bytes = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback_bytes = u64::from(padded_row_bytes) * u64::from(height);
        if readback_bytes > device.limits().max_buffer_size {
            return Err(invalid(format!(
                "PNG readback needs {readback_bytes} bytes; GPU device buffer limit is {}",
                device.limits().max_buffer_size
            )));
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map PNG readback"),
            size: readback_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("map PNG copy"),
        });
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: Some(height),
                },
            },
            extent(target.size),
        );
        let submission = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (send, receive) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .map_err(io::Error::other)?;
        receive
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(io::Error::other)?
            .map_err(io::Error::other)?;
        let mapped = slice.get_mapped_range().map_err(io::Error::other)?;
        let mut pixels = Vec::with_capacity(row_bytes as usize * height as usize);
        for row in mapped.chunks_exact(padded_row_bytes as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        drop(mapped);
        buffer.unmap();
        let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer.write_image_data(&pixels).map_err(io::Error::other)?;
        writer.finish().map_err(io::Error::other)
    }
}

pub struct MapImage<'a> {
    pub width: u32,
    pub height: u32,
    pub province_ids: &'a [u32],
    pub colors: &'a [[u8; 4]],
    pub owner_keys: &'a [u32],
}

/// Returns the adapter name so headless output can identify the GPU actually used.
pub fn dump_png(
    image: MapImage<'_>,
    camera: Camera,
    size: [u32; 2],
    path: &Path,
    banner: Option<&str>,
) -> io::Result<String> {
    camera.validate()?;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(io::Error::other)?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&device_descriptor(&adapter))
            .await
            .map_err(io::Error::other)?;
        let mut renderer = MapRenderer::new(
            &device,
            &queue,
            image.width,
            image.height,
            image.province_ids,
        )?;
        renderer.set_colors(&queue, image.colors, image.owner_keys)?;
        renderer.render(&device, &queue, size, camera)?;
        if let Some(text) = banner {
            renderer.render_banner(&device, &queue, text)?;
        }
        renderer.write_png(&device, &queue, path)?;
        Ok(format!("{} ({:?})", info.name, info.backend))
    })
}

fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    }
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    dimension: wgpu::TextureDimension,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, size: [u32; 2], data: &[u8]) {
    queue.write_texture(
        texture.as_image_copy(),
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 4),
            rows_per_image: Some(size[1]),
        },
        extent(size),
    );
}

fn validate_size(size: [u32; 2], limit: u32) -> io::Result<()> {
    if size
        .into_iter()
        .any(|dimension| dimension == 0 || dimension > limit)
    {
        Err(invalid(format!(
            "texture dimensions {}×{} must be nonzero and no larger than the GPU limit {limit}",
            size[0], size[1]
        )))
    } else {
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::Camera;

    #[test]
    fn fit_preserves_map_aspect_and_pan_uses_viewport_scale() {
        let mut camera = Camera::fit([6000, 2000], [1000, 1000]);
        assert_eq!(camera.center, [3000.0, 1000.0]);
        assert_eq!(camera.vertical_span, 6000.0);
        camera.pan_pixels([10.0, -20.0], [1000, 1000]);
        assert_eq!(camera.center, [2940.0, 1120.0]);
    }

    #[test]
    fn zoom_preserves_map_point_under_pointer() {
        let mut camera = Camera {
            center: [2000.0, 700.0],
            vertical_span: 1000.0,
        };
        let viewport = [2000, 1000];
        let pointer = [0.8, 0.2];
        let point = |camera: Camera| {
            [
                camera.center[0] + (pointer[0] - 0.5) * camera.vertical_span * 2.0,
                camera.center[1] + (pointer[1] - 0.5) * camera.vertical_span,
            ]
        };
        let before = point(camera);
        camera.zoom_at(2.0, pointer, viewport);
        let after = point(camera);
        for (a, b) in before.into_iter().zip(after) {
            assert!((a - b).abs() < 0.001);
        }
        assert_eq!(camera.vertical_span, 500.0);
    }

    #[test]
    fn invalid_camera_is_refused() {
        assert!(
            Camera {
                center: [f32::NAN, 0.0],
                vertical_span: 10.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Camera {
                center: [0.0, 0.0],
                vertical_span: 0.0
            }
            .validate()
            .is_err()
        );
    }
}
