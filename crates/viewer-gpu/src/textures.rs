#[cfg(test)]
use crate::gpu_contract::device_descriptor;
#[cfg(test)]
use crate::gpu_contract::select_adapter;
#[cfg(test)]
use crate::test_support::GPU_TESTS;
use std::io;

use eframe::wgpu;
use content_reader_gui::gui::native::resources::{self, NoTexture};
use content_reader_map::map::{MapData, ProvinceKind};
use content_reader_key_names::terrain::TerrainDefinitions;
use content_reader_image::{IndexedImage, Rgba8Image, dds, native::Decode};

use super::{camera::MapSize, gradient_math};

mod gui;
pub use gui::{GuiTexture, upload_gui};

mod intel_cells;
pub use intel_cells::{IntelCells, IntelCellsCache};

pub const MAP_NUM_TILES: u32 = 4;
pub use map_snow::SNOW_MUD_BLOCK;
pub const FOW_BLOCK: u32 = 6;
pub fn fow_size(map: MapSize) -> [u32; 2] {
    [map.width / FOW_BLOCK, map.height / FOW_BLOCK]
}

pub fn gradient_size(map: MapSize) -> [u32; 2] {
    [
        map.width / gradient_math::DIVISOR,
        2 * (map.height / gradient_math::DIVISOR + 1),
    ]
}

pub fn gradient_layer_height(map: MapSize) -> f32 {
    (map.height / gradient_math::DIVISOR) as f32
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Content(u64);

impl Content {
    pub fn next() -> Self {
        static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    pub fn number(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy)]
pub struct Sampled<'a> {
    pub view: &'a wgpu::TextureView,
    pub content: Option<Content>,
}

pub struct Bound {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub view_format: wgpu::TextureFormat,
    content: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Bound {
    fn new(texture: wgpu::Texture, descriptor: &wgpu::TextureViewDescriptor<'_>) -> Self {
        let view = texture.create_view(descriptor);
        let view_format = descriptor.format.unwrap_or(texture.format());
        Self::with_view(texture, view, view_format)
    }

    pub fn with_view(
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        view_format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            texture,
            view,
            view_format,
            content: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
                Content::next().number(),
            )),
        }
    }

    pub fn of(texture: wgpu::Texture) -> Self {
        Self::new(texture, &Default::default())
    }

    pub fn share(&self) -> Self {
        Self {
            texture: self.texture.clone(),
            view: self.view.clone(),
            view_format: self.view_format,
            content: std::sync::Arc::clone(&self.content),
        }
    }

    pub fn content(&self) -> Content {
        Content(
            self.content
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    pub fn sampled(&self) -> Sampled<'_> {
        Sampled {
            view: &self.view,
            content: Some(self.content()),
        }
    }

    pub fn written(&self) {
        self.content.store(
            Content::next().number(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    pub fn write(&self, queue: &wgpu::Queue, size: [u32; 2], bytes_per_texel: u32, data: &[u8]) {
        write_texels(queue, &self.texture, size, bytes_per_texel, data);
        self.written();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ColorSpace {
    Linear,
    Srgb,
}

fn format_of(format: dds::TextureFormat, space: ColorSpace) -> io::Result<wgpu::TextureFormat> {
    Ok(match (format, space) {
        (dds::TextureFormat::Bc1, ColorSpace::Linear) => wgpu::TextureFormat::Bc1RgbaUnorm,
        (dds::TextureFormat::Bc1, ColorSpace::Srgb) => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        (dds::TextureFormat::Bc2, ColorSpace::Linear) => wgpu::TextureFormat::Bc2RgbaUnorm,
        (dds::TextureFormat::Bc2, ColorSpace::Srgb) => wgpu::TextureFormat::Bc2RgbaUnormSrgb,
        (dds::TextureFormat::Bc3, ColorSpace::Linear) => wgpu::TextureFormat::Bc3RgbaUnorm,
        (dds::TextureFormat::Bc3, ColorSpace::Srgb) => wgpu::TextureFormat::Bc3RgbaUnormSrgb,
        (dds::TextureFormat::Bc7 { srgb: false }, _) => wgpu::TextureFormat::Bc7RgbaUnorm,
        (dds::TextureFormat::Bc7 { srgb: true }, _) => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
        (dds::TextureFormat::Rgba8, ColorSpace::Linear) => wgpu::TextureFormat::Rgba8Unorm,
        (dds::TextureFormat::Rgba8, ColorSpace::Srgb) => wgpu::TextureFormat::Rgba8UnormSrgb,
        (dds::TextureFormat::B5G6R5 | dds::TextureFormat::B5G5R5A1, ColorSpace::Linear) => {
            wgpu::TextureFormat::Rgba32Float
        }
        (dds::TextureFormat::B5G6R5 | dds::TextureFormat::B5G5R5A1, ColorSpace::Srgb) => {
            return Err(invalid(format!(
                "a {format:?} texture has no sRGB format in the engine"
            )));
        }
    })
}

fn sixteen_bit_texels(format: dds::TextureFormat, data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 8);
    for word in data.chunks_exact(2) {
        let channels = format.sample_16(u16::from_le_bytes([word[0], word[1]]))?;
        for value in channels {
            out.extend(value.to_le_bytes());
        }
    }
    Some(out)
}

pub type Read<'a, B = content_reader_layers::content::Bytes> = &'a dyn Fn(&[u8]) -> io::Result<B>;
pub type Locate<'a> = &'a dyn Fn(&[u8]) -> io::Result<Option<usize>>;

pub fn locate_file(name: &str, locate: Locate<'_>) -> io::Result<(String, usize)> {
    match content_reader_layers::textures::locate_texture(name.as_bytes(), locate)? {
        Ok(found) => Ok(found),
        Err(NoTexture::Missing(tried)) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name}: no layer holds any of {tried:?}"),
        )),
        Err(reason) => Err(invalid(format!("{name}: {reason}"))),
    }
}

pub fn load_file<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_image::Bytes>>(name: &str, read: Read<'_, B>, decode: Decode) -> io::Result<dds::Texture> {
    match resources::texture(name.as_bytes(), read, decode)? {
        Ok(texture) => Ok(texture.image),
        Err(NoTexture::Missing(tried)) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name}: no layer holds any of {tried:?}"),
        )),
        Err(reason) => Err(invalid(format!("{name}: {reason}"))),
    }
}

pub fn load_image<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_image::Bytes>>(name: &str, read: Read<'_, B>, decode: Decode) -> io::Result<content_reader_image::SharedRgba8Image> {
    let texture = load_file(name, read, decode)?;
    if texture.format != dds::TextureFormat::Rgba8 || texture.faces != 1 {
        return Err(invalid(format!(
            "{name} loads as a {:?} texture with {} faces, not the RGBA image the map reads",
            texture.format, texture.faces
        )));
    }
    Ok(Rgba8Image {
        width: texture.width,
        height: texture.height,
        pixels: texture.base().data.clone(),
    })
}

pub fn sampled_heights(
    image: &Rgba8Image<impl std::ops::Deref<Target = [u8]>>,
    heightmap: &IndexedImage,
) -> Option<([u32; 2], Vec<u8>)> {
    let texels: Vec<u8> = image.pixels.chunks_exact(4).map(|texel| texel[0]).collect();
    let size = [image.width, image.height];
    (size != [heightmap.width, heightmap.height] || texels.as_slice() != heightmap.indices.as_ref())
        .then_some((size, texels))
}

pub fn check_upload(label: &str, texture: &dds::Texture, space: ColorSpace) -> io::Result<()> {
    let cube = texture.faces == 6;
    if texture.faces != 1 && !cube {
        return Err(invalid(format!(
            "{label} has {} faces; only a plain texture or a cube map can be uploaded",
            texture.faces
        )));
    }
    if cube && texture.width != texture.height {
        return Err(invalid(format!(
            "{label} is a {} x {} cube map; this device requires square cube-map faces",
            texture.width, texture.height
        )));
    }
    format_of(texture.format, space)?;
    if texture.width == 0 || texture.height == 0 {
        return Err(invalid(format!("{label} has an empty texture extent")));
    }
    Ok(())
}

pub fn load_uploadable<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_image::Bytes>>(
    name: &str,
    read: Read<'_, B>,
    decode: Decode,
    space: ColorSpace,
) -> io::Result<dds::Texture> {
    let texture = load_file(name, read, decode)?;
    check_upload(name, &texture, space)?;
    Ok(texture)
}

#[derive(Debug)]
pub struct MapTexture {
    pub texture: dds::Texture,
    pub unbound: bool,
}

pub fn load_map_texture<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_image::Bytes>>(
    name: &str,
    read: Read<'_, B>,
    decode: Decode,
    space: ColorSpace,
    warnings: &mut Vec<String>,
) -> io::Result<MapTexture> {
    match resources::texture(name.as_bytes(), read, decode)? {
        Ok(texture) => {
            check_upload(name, &texture.image, space)?;
            Ok(MapTexture {
                texture: texture.image,
                unbound: false,
            })
        }
        Err(reason) if reason.native_failure() => {
            warnings.push(format!(
                "{name}: {reason}; reproducing the game's unbound map texture."
            ));
            Ok(MapTexture {
                texture: dds::Texture::from_rgba8(Rgba8Image {
                    width: 1,
                    height: 1,
                    pixels: vec![0; 4],
                }),
                unbound: true,
            })
        }
        Err(reason) => Err(invalid(format!("{name}: {reason}"))),
    }
}

pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texture: &dds::Texture,
    space: ColorSpace,
) -> io::Result<Bound> {
    upload_unpacking(device, queue, label, texture, space, None, None)
}

fn upload_unpacking(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texture: &dds::Texture,
    space: ColorSpace,
    unpack: Option<&wgpu::ComputePipeline>,
    target: Option<(&wgpu::Texture, u32)>,
) -> io::Result<Bound> {
    check_upload(label, texture, space)?;
    let cube = texture.faces == 6;
    let format = format_of(texture.format, space)?;
    let limits = device.limits();
    if texture.width > limits.max_texture_dimension_2d
        || texture.height > limits.max_texture_dimension_2d
        || texture.faces > limits.max_texture_array_layers
    {
        return Err(io::Error::other(format!(
            "{label} is {} x {} with {} faces; this device accepts at most {} x {} with {} layers",
            texture.width,
            texture.height,
            texture.faces,
            limits.max_texture_dimension_2d,
            limits.max_texture_dimension_2d,
            limits.max_texture_array_layers
        )));
    }
    if texture.format.block_bytes().is_some()
        && (!texture.width.is_multiple_of(4) || !texture.height.is_multiple_of(4))
    {
        return match unpack {
            Some(unpack) => {
                upload_unaligned_bc(device, queue, label, texture, space, unpack, target)
            }
            None => upload_unaligned_bc(
                device,
                queue,
                label,
                texture,
                space,
                &unpack_pipeline(device),
                target,
            ),
        };
    }
    let gpu = target.map(|(gpu, _)| gpu.clone()).unwrap_or_else(|| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: texture.width,
                height: texture.height,
                depth_or_array_layers: texture.faces,
            },
            mip_level_count: texture.mip_count() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    });
    let block = format.block_dimensions().0;
    let unit_bytes = format
        .block_copy_size(None)
        .ok_or_else(|| invalid(format!("{label}: {format:?} has no copy size")))?;
    for (face, levels) in texture.mips.iter().enumerate() {
        for (level, mip) in levels.iter().enumerate() {
            let row_blocks = mip.width.div_ceil(block);
            let bytes_per_row = row_blocks * unit_bytes;
            let expanded = sixteen_bit_texels(texture.format, &mip.data);
            shader_pipeline::gpu::stats::write_texture(
                &queue,
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: target.map_or(0, |(_, layer)| layer) + face as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                expanded.as_deref().unwrap_or(&mip.data),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(mip.height.div_ceil(block)),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: 1,
                }
                .physical_size(format),
            );
        }
    }
    Ok(Bound::new(
        gpu,
        &wgpu::TextureViewDescriptor {
            label: Some(label),
            base_array_layer: target.map_or(0, |(_, layer)| layer),
            array_layer_count: Some(texture.faces),
            dimension: Some(if cube {
                wgpu::TextureViewDimension::Cube
            } else {
                wgpu::TextureViewDimension::D2
            }),
            ..Default::default()
        },
    ))
}

pub fn unpack_pipeline(device: &wgpu::Device) -> std::sync::Arc<wgpu::ComputePipeline> {
    viewer_runtime::device_shared::shared(device, "BC logical extent unpack", || {
        shader_pipeline::gpu::stats::build();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("BC logical extent unpack"),
            source: wgpu::ShaderSource::Wgsl(include_str!("texture_unpack.wgsl").into()),
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("BC logical extent unpack"),
            layout: None,
            module: &module,
            entry_point: Some("unpack"),
            compilation_options: Default::default(),
            cache: None,
        })
    })
}

fn upload_unaligned_bc(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texture: &dds::Texture,
    space: ColorSpace,
    pipeline: &wgpu::ComputePipeline,
    target: Option<(&wgpu::Texture, u32)>,
) -> io::Result<Bound> {
    let format = format_of(texture.format, space)?;
    let view_format = if format.is_srgb() {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let gpu = target.map(|(gpu, _)| gpu.clone()).unwrap_or_else(|| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: texture.width,
                height: texture.height,
                depth_or_array_layers: texture.faces,
            },
            mip_level_count: texture.mip_count() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | if cfg!(test) { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() },
            view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
        })
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for (face, mips) in texture.mips.iter().enumerate() {
        for (level, mip) in mips.iter().enumerate() {
            let size = wgpu::Extent3d {
                width: mip.width.div_ceil(4) * 4,
                height: mip.height.div_ceil(4) * 4,
                depth_or_array_layers: 1,
            };
            let source = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("BC padded staging mip"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: format.remove_srgb_suffix(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            shader_pipeline::gpu::stats::write_texture(
                &queue,
                source.as_image_copy(),
                &mip.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(
                        size.width / 4 * texture.format.block_bytes().unwrap() as u32,
                    ),
                    rows_per_image: Some(size.height / 4),
                },
                size,
            );
            let source = source.create_view(&Default::default());
            let destination = gpu.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_mip_level: level as u32,
                mip_level_count: Some(1),
                base_array_layer: target.map_or(0, |(_, layer)| layer) + face as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let bindings = shader_pipeline::gpu::stats::create_bind_group(
                &device,
                &wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&destination),
                        },
                    ],
                },
            );
            let mut pass = encoder.begin_compute_pass(&Default::default());
            shader_pipeline::gpu::stats::pipeline();
            pass.set_pipeline(&pipeline);
            shader_pipeline::gpu::stats::bind();
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(mip.width.div_ceil(8), mip.height.div_ceil(8), 1);
        }
    }
    queue.submit([encoder.finish()]);
    Ok(Bound::new(
        gpu,
        &wgpu::TextureViewDescriptor {
            label: Some(label),
            base_array_layer: target.map_or(0, |(_, layer)| layer),
            array_layer_count: Some(texture.faces),
            format: Some(view_format),
            usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
            dimension: Some(if texture.faces == 6 {
                wgpu::TextureViewDimension::Cube
            } else {
                wgpu::TextureViewDimension::D2
            }),
            ..Default::default()
        },
    ))
}

#[cfg(test)]
fn bc7_mode6_block(blue: bool) -> Vec<u8> {
    let mut bits = 1_u128 << 6;
    let mut offset = 7;
    for channel in [
        if blue { 0 } else { 127 },
        0,
        if blue { 127 } else { 0 },
        127,
    ] {
        for _ in 0..2 {
            bits |= (channel as u128) << offset;
            offset += 7;
        }
    }
    bits |= 3 << offset;
    bits.to_le_bytes().to_vec()
}

#[test]
fn unaligned_bc_keeps_authored_mips_faces_and_texels() {
    let _guard = GPU_TESTS.lock().unwrap();
    pollster::block_on(async {
        let instance = shader_pipeline::gpu::instance();
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = select_adapter(&adapters, None).unwrap();
        let (device, queue) = adapter
            .request_device(&device_descriptor(&adapter))
            .await
            .unwrap();
        for format in [
            dds::TextureFormat::Bc1,
            dds::TextureFormat::Bc2,
            dds::TextureFormat::Bc3,
            dds::TextureFormat::Bc7 { srgb: false },
            dds::TextureFormat::Bc7 { srgb: true },
        ] {
            let block = |blue: bool| {
                if matches!(format, dds::TextureFormat::Bc7 { .. }) {
                    return bc7_mode6_block(blue);
                }
                let mut bytes = match format {
                    dds::TextureFormat::Bc1 => vec![],
                    dds::TextureFormat::Bc2 => vec![255; 8],
                    _ => vec![255, 0, 0, 0, 0, 0, 0, 0],
                };
                bytes.extend(if blue {
                    [31, 0, 0, 0, 0, 0, 0, 0]
                } else {
                    [0, 248, 0, 0, 0, 0, 0, 0]
                });
                bytes
            };
            let texture = dds::Texture {
                format,
                width: 5,
                height: 5,
                faces: 6,
                mips: (0..6)
                    .map(|face| {
                        (0..3)
                            .map(|level| {
                                let size = 5u32 >> level;
                                dds::MipLevel {
                                    width: size,
                                    height: size,
                                    data: (block((face + level) % 2 == 0)
                                        .repeat(size.div_ceil(4).pow(2) as usize)).into(),
                                }
                            })
                            .collect()
                    })
                    .collect(),
            };
            let bound = upload(&device, &queue, "odd cube", &texture, ColorSpace::Srgb).unwrap();
            for face in 0..6 {
                for level in 0..3 {
                    let size = 5 >> level;
                    let bytes = u64::from(size) * 256;
                    let readback = device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: bytes,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    let mut encoder = device.create_command_encoder(&Default::default());
                    encoder.copy_texture_to_buffer(
                        wgpu::TexelCopyTextureInfo {
                            texture: &bound.texture,
                            mip_level: level,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: face,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(256),
                                rows_per_image: Some(size),
                            },
                        },
                        wgpu::Extent3d {
                            width: size,
                            height: size,
                            depth_or_array_layers: 1,
                        },
                    );
                    queue.submit([encoder.finish()]);
                    let (tx, rx) = std::sync::mpsc::channel();
                    readback
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |result| {
                            tx.send(result).unwrap();
                        });
                    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    rx.recv().unwrap().unwrap();
                    let data = readback.slice(..).get_mapped_range().unwrap();
                    let mut expected = if (face + level) % 2 == 0 {
                        [0, 0, 255, 255]
                    } else {
                        [255, 0, 0, 255]
                    };
                    if matches!(format, dds::TextureFormat::Bc7 { .. }) {
                        for channel in &mut expected[..3] {
                            *channel = (*channel).max(1);
                        }
                    }
                    for row in data.chunks_exact(256) {
                        assert!(
                            row[..size as usize * 4]
                                .chunks_exact(4)
                                .all(|pixel| pixel == expected),
                            "{format:?}, face {face}, level {level}"
                        );
                    }
                }
            }
        }
    });
}

pub fn data_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    data: &[u8],
) -> io::Result<Bound> {
    let bytes_per_texel = match format {
        wgpu::TextureFormat::R8Unorm => 1,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => 4,
        wgpu::TextureFormat::Rgba32Float => 16,
        other => return Err(invalid(format!("{label}: {other:?} is not a data format"))),
    };
    let expected = size[0] as usize * size[1] as usize * bytes_per_texel as usize;
    if data.len() != expected {
        return Err(invalid(format!(
            "{label} needs {expected} bytes for {}x{}, got {}",
            size[0],
            size[1],
            data.len()
        )));
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_texels(queue, &texture, size, bytes_per_texel, data);
    Ok(Bound::new(texture, &Default::default()))
}

fn write_texels(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: [u32; 2],
    bytes_per_texel: u32,
    data: &[u8],
) {
    shader_pipeline::gpu::stats::write_texture(
        &queue,
        texture.as_image_copy(),
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * bytes_per_texel),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
}

pub fn terrain_flag_mask(
    definitions: &TerrainDefinitions,
    terrain: &IndexedImage,
    flag: impl Fn(&content_reader_key_names::terrain::TerrainType) -> bool,
) -> Vec<bool> {
    let mut lookup = [false; 256];
    for (&index, kind) in &definitions.palette {
        lookup[index as usize] = flag(kind);
    }
    terrain
        .indices
        .iter()
        .map(|&index| lookup[index as usize])
        .collect()
}

pub fn atlas_lookup(
    definitions: &TerrainDefinitions,
    terrain: &IndexedImage,
    warnings: &mut Vec<String>,
) -> [u8; 256] {
    let mut counts = [0usize; 256];
    for &index in &terrain.indices {
        counts[index as usize] += 1;
    }
    let mut lookup = [0u8; 256];
    for (&index, kind) in &definitions.palette {
        lookup[index as usize] = kind.atlas_index as u8;
    }
    let unmapped: Vec<u8> = (0..=255u8)
        .filter(|index| counts[*index as usize] > 0 && !definitions.palette.contains_key(index))
        .collect();
    if !unmapped.is_empty() {
        let drawn: usize = unmapped.iter().map(|index| counts[*index as usize]).sum();
        warnings.push(format!(
            "map/terrain.bmp uses {} palette indices that common/terrain declares no texture for ({}); they are drawn with atlas tile 0, over {drawn} of the bitmap's {} pixels.",
            unmapped.len(),
            unmapped
                .iter()
                .map(|index| format!("{index} on {} pixels", counts[*index as usize]))
                .collect::<Vec<_>>()
                .join(", "),
            terrain.indices.len()
        ));
    }
    lookup
}

pub fn terrain_id_map(
    width: u32,
    height: u32,
    indices: &[u8],
    atlas_of_palette: &[u8; 256],
) -> io::Result<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if indices.len() != w * h || w == 0 || h == 0 {
        return Err(invalid(format!(
            "terrain bitmap is {width}x{height} but carries {} indices",
            indices.len()
        )));
    }
    let mut out = vec![0u8; w * h * 4];
    let at = |x: usize, z: usize| -> u8 {
        let row = h - 1 - z.min(h - 1);
        atlas_of_palette[indices[row * w + x.min(w - 1)] as usize]
    };
    for z in 0..h {
        for x in 0..w {
            let base = at(x, z);
            let right = at(x + 1, z);
            let up = at(x, z + 1);
            let up_right = at(x + 1, z + 1);
            let same = base == right && base == up && base == up_right;
            let texel = &mut out[(z * w + x) * 4..][..4];
            texel[0] = up;
            texel[1] = right;
            texel[2] = if same { base.wrapping_add(100) } else { base };
            texel[3] = up_right;
        }
    }
    Ok(out)
}

pub fn clear_intel_map(map: &MapData) -> io::Result<IntelMap> {
    intel_image_size(map)?;
    let [width, height] = fow_size(MapSize {
        width: map.width,
        height: map.height,
    });
    Ok(IntelMap {
        texels: map_display_data::Paged::try_from_iter(
            width as usize * height as usize,
            std::iter::repeat_n(0, width as usize * height as usize),
        )?,
        fog: false,
        fogged_texels: 0,
    })
}

pub use map_display_data::Intel as IntelMap;

pub fn intel_line(
    intel: &IntelMap,
    defines: &super::camera::MapDefines,
    camera_height: f32,
    switch: bool,
) -> String {
    if !switch {
        return "Fog of war: switched off, so none is drawn, as the game's fow console \
                    switch draws none"
            .into();
    }
    if !intel.fog {
        return "Fog of war: none drawn; this frame has no recorded per-province visibility \
                    for its viewing country"
            .into();
    }
    let fogged = intel.texels.iter().filter(|&&b| b > 0).count();
    let mean =
        intel.texels.iter().map(|&b| f64::from(b)).sum::<f64>() / intel.texels.len().max(1) as f64;
    format!(
        "Fog of war: {fogged} of {} intel texels carry fog ({:.1}% of the map), mean intel \
             {:.3}; FOWFadeFactor {:.4} at camera height {camera_height:.1}",
        intel.texels.len(),
        100.0 * fogged as f64 / intel.texels.len().max(1) as f64,
        mean / 255.0,
        super::frame::fow_fade_factor(defines, camera_height, true),
    )
}
fn intel_image_size(map: &MapData) -> io::Result<[usize; 2]> {
    let (width, height) = (map.width as usize, map.height as usize);
    if width.checked_mul(height) != Some(map.province_ids.len()) || width == 0 || height == 0 {
        return Err(invalid(format!(
            "the province image is {}x{} but carries {} ids",
            map.width,
            map.height,
            map.province_ids.len()
        )));
    }
    Ok([width, height])
}

pub fn expect_size(name: &str, width: u32, height: u32, expected: [u32; 2]) -> io::Result<()> {
    if [width, height] == expected {
        Ok(())
    } else {
        Err(invalid(format!(
            "{name} is {width}x{height}; the map shaders are compiled for {}x{}",
            expected[0], expected[1]
        )))
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;
