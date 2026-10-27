//! Map textures resolved through the recorded content stack.
//!
//! File textures retain their decoded padding, block extent and mip chain.
//! Each draw chooses its required colour space; colour textures and linear
//! data textures keep distinct GPU views. Generated textures retain their
//! logical dimensions and row/channel layout.
//!
//! Terrain palette definitions select atlas tiles. Undeclared or unreachable
//! atlas indices use tile zero with named, counted warnings only when bitmap
//! pixels reach them. The terrain-id payload holds four neighbour tiles and an
//! all-equal flag. Snow/mud and fog use independent four- and six-pixel blocks.
//!
//! The upload helpers preserve native texture precision, sampling and loaded
//! mips. Point-light, gradient-border, tree-mask and shadow producers own their
//! respective generated inputs; texture loading does not invent their state.
#[cfg(test)]
use crate::map::gpu_contract::device_descriptor;
#[cfg(test)]
use crate::map::gpu_contract::select_adapter;
#[cfg(test)]
use crate::map::test_support::GPU_TESTS;
use std::io;

use eframe::wgpu;
use installed_content_core::{
    gui::native::resources, gui::native::resources::NoTexture, map::MapData, map::ProvinceKind,
    terrain::TerrainDefinitions,
};
use installed_content_image::{IndexedImage, Rgba8Image, dds, native::Decode};

use super::{camera::MapSize, gradient_math};

mod intel_cells;
pub use intel_cells::{IntelCells, IntelCellsCache};

/// Atlas tiles per axis.
pub const MAP_NUM_TILES: u32 = 4;
/// Tiles an atlas can address, and therefore the valid `texture = N` range.
pub const ATLAS_TILES: u32 = MAP_NUM_TILES * MAP_NUM_TILES;
// weather
/// Map pixels one SnowMudData texel covers on each axis. Generated snow/mud
/// textures remain unpadded; shader globals follow the client's texture-decode
/// settings and retain the independent fog-grid ratio.
pub use map_snow::SNOW_MUD_BLOCK;
// fog of war
/// Province-image pixels one IntelMap texel covers on each axis. Its six-by-six
/// blocks are independent of the snow/mud producer's four-by-four blocks.
pub const FOW_BLOCK: u32 = 6;
/// `MAP_SIZE / 6`, truncating: the intel map's size.
///
/// The engine truncates — 5632 / 6 is 938 on the shipped map, covering 5628 of
/// its 5632 columns — while the shader's `u` still runs the whole texture over
/// the whole map. The stretch that leaves is the engine's own and is reproduced.
pub fn fow_size(map: MapSize) -> [u32; 2] {
    [map.width / FOW_BLOCK, map.height / FOW_BLOCK]
}

/// Two gradient-border channel textures use map width divided by `gradient_math::DIVISOR` and stack
/// two downscaled layers, each followed by a zero guard row. `GB_TextureHeight` is one layer's
/// height; see [`gradient_layer_height`].
pub fn gradient_size(map: MapSize) -> [u32; 2] {
    [
        map.width / gradient_math::DIVISOR,
        2 * (map.height / gradient_math::DIVISOR + 1),
    ]
}

/// `GB_TextureHeight`: the height of **one layer**, not of the texture.
///
/// Use one layer's height to keep its final sample on the final data row, before the zero guard
/// row. The second layer applies the same span after its layer offset.
pub fn gradient_layer_height(map: MapSize) -> f32 {
    (map.height / gradient_math::DIVISOR) as f32
}

/// A texture and the view every pass binds it through.
pub struct Bound {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Bound {
    fn new(texture: wgpu::Texture, descriptor: &wgpu::TextureViewDescriptor<'_>) -> Self {
        let view = texture.create_view(descriptor);
        Self { texture, view }
    }
}

/// Which of the two DXGI formats the engine creates a texture with, and so
/// whether the hardware decodes each sample from sRGB.
///
/// Colour textures use sRGB views; normal, specular, mask and id textures use linear UNORM views.
/// Alpha remains linear in both.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ColorSpace {
    /// `..._UNORM`: the stored bytes are the values.
    Linear,
    /// `..._UNORM_SRGB`: the hardware decodes rgb at every sample, alpha not.
    Srgb,
}

/// The wgpu format a decoded DDS uploads as, in the colour space the engine
/// creates the same file's texture with.
///
/// WebGPU lacks packed 16-bit formats. Upload five/six-bit channels as exact f32 normalized values in
/// RGBA32_FLOAT; narrower formats can cross shader thresholds. Filter in shader when
/// FLOAT32_FILTERABLE is absent. These formats have no sRGB view.
fn format_of(format: dds::TextureFormat, space: ColorSpace) -> io::Result<wgpu::TextureFormat> {
    Ok(match (format, space) {
        (dds::TextureFormat::Bc1, ColorSpace::Linear) => wgpu::TextureFormat::Bc1RgbaUnorm,
        (dds::TextureFormat::Bc1, ColorSpace::Srgb) => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        (dds::TextureFormat::Bc2, ColorSpace::Linear) => wgpu::TextureFormat::Bc2RgbaUnorm,
        (dds::TextureFormat::Bc2, ColorSpace::Srgb) => wgpu::TextureFormat::Bc2RgbaUnormSrgb,
        (dds::TextureFormat::Bc3, ColorSpace::Linear) => wgpu::TextureFormat::Bc3RgbaUnorm,
        (dds::TextureFormat::Bc3, ColorSpace::Srgb) => wgpu::TextureFormat::Bc3RgbaUnormSrgb,
        // BC7 keeps its DX10 header's colour space whatever the role (owner exception).
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

/// A 16-bit format's level as [`format_of`]'s `Rgba32Float` takes it: each word's channels
/// ([`dds::TextureFormat::sample_16`], `c / (2^n - 1)` as the device's `_UNORM` conversion makes
/// it) as `f32`s. `None` for any other format, whose bytes go up as decoded.
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

/// Reads a file a texture load asks for, by its path's bytes: `ResolvedLayers::read_path` for
/// covered content, `read_uncovered_path` otherwise.
pub type Read<'a> = &'a dyn Fn(&[u8]) -> io::Result<Vec<u8>>;

/// Resolve texture stems in DDS, TGA, then authored-path order. Refuse files past 16 MiB. Decode
/// block-rounded extents, retained mips and cube faces through resources::texture. File textures pad
/// only under the client force_pow2_textures setting; generated textures retain dimensions. Missing
/// names return NotFound.
pub fn load_file(name: &str, read: Read<'_>, decode: Decode) -> io::Result<dds::Texture> {
    match resources::texture(name.as_bytes(), read, decode)? {
        Ok(texture) => Ok(texture.image),
        Err(NoTexture::Missing(tried)) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name}: no layer holds any of {tried:?}"),
        )),
        Err(reason) => Err(invalid(format!("{name}: {reason}"))),
    }
}

/// [`load_file`] of an image the map reads as straight RGBA rows (a bitmap or a Targa): the game
/// loads it as `B8G8R8A8` or `B8G8R8X8`, never as blocks.
pub fn load_image(name: &str, read: Read<'_>, decode: Decode) -> io::Result<Rgba8Image> {
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

/// Heightmap sampling texture size and linear payload. Its first channel is resolved palette red.
/// None means it matches the vertex heightmap exactly.
pub fn sampled_heights(
    image: &Rgba8Image,
    heightmap: &IndexedImage,
) -> Option<([u32; 2], Vec<u8>)> {
    let texels: Vec<u8> = image.pixels.chunks_exact(4).map(|texel| texel[0]).collect();
    let size = [image.width, image.height];
    (size != [heightmap.width, heightmap.height] || texels != heightmap.indices)
        .then_some((size, texels))
}

/// What [`upload`] refuses about a texture itself on every device: a face count other than a
/// plain texture's or a cube map's, non-square cube faces, a format without the colour space
/// the texture is used in, or an empty extent. Device limits are checked at upload.
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

/// [`load_file`] of a texture uploaded in `space`, refused here when [`check_upload`] would
/// refuse it at upload.
pub fn load_uploadable(
    name: &str,
    read: Read<'_>,
    decode: Decode,
    space: ColorSpace,
) -> io::Result<dds::Texture> {
    let texture = load_file(name, read, decode)?;
    check_upload(name, &texture, space)?;
    Ok(texture)
}

/// Uploads a decoded DDS with every mip level it carries.
///
/// Block-compressed payloads stay compressed, which is why the device must
/// support `TEXTURE_COMPRESSION_BC`; a cube map becomes a six-layer `D2Array`
/// with a cube view, in the DDS face order `+X -X +Y -Y +Z -Z`.
pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texture: &dds::Texture,
    space: ColorSpace,
) -> io::Result<Bound> {
    check_upload(label, texture, space)?;
    let cube = texture.faces == 6;
    let format = format_of(texture.format, space)?;
    // Device limits are fatal upload failures, separate from invalid content.
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
        return upload_unaligned_bc(device, queue, label, texture, space);
    }
    let gpu = device.create_texture(&wgpu::TextureDescriptor {
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
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: face as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                expanded.as_deref().unwrap_or(&mip.data),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(mip.height.div_ceil(block)),
                },
                // A block-compressed mip below four texels still occupies one
                // whole block, and a copy has to name that physical size.
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
            dimension: Some(if cube {
                wgpu::TextureViewDimension::Cube
            } else {
                wgpu::TextureViewDimension::D2
            }),
            ..Default::default()
        },
    ))
}

// BC7 keeps logical extents. Crop each padded staging mip without
// changing authored levels or the hardware's BC interpolation.
fn upload_unaligned_bc(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    texture: &dds::Texture,
    space: ColorSpace,
) -> io::Result<Bound> {
    let format = format_of(texture.format, space)?;
    let view_format = if format.is_srgb() {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let gpu = device.create_texture(&wgpu::TextureDescriptor {
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
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("BC logical extent unpack"),
        source: wgpu::ShaderSource::Wgsl(include_str!("texture_unpack.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("BC logical extent unpack"),
        layout: None,
        module: &module,
        entry_point: Some("unpack"),
        compilation_options: Default::default(),
        cache: None,
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
                // Copy stored channel values; the final view applies sRGB decoding.
                format: format.remove_srgb_suffix(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
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
                base_array_layer: face as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(mip.width.div_ceil(8), mip.height.div_ceil(8), 1);
        }
    }
    queue.submit([encoder.finish()]);
    Ok(Bound::new(
        gpu,
        &wgpu::TextureViewDescriptor {
            label: Some(label),
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
    // Equal 7-bit endpoints, both p-bits 1, all indices 0.
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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
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
                                    data: block((face + level) % 2 == 0)
                                        .repeat(size.div_ceil(4).pow(2) as usize),
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
                        // Mode 6 shares one p-bit per endpoint across channels: 255 needs 1,
                        // so that endpoint's zero channels also decode to 1.
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

/// Creates a single-mip data texture the viewer builds itself, and fills it.
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
    write_data(queue, &texture, size, bytes_per_texel, data);
    Ok(Bound::new(texture, &Default::default()))
}

/// Replaces the contents of a data texture created by [`data_texture`].
pub fn write_data(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: [u32; 2],
    bytes_per_texel: u32,
    data: &[u8],
) {
    queue.write_texture(
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

/// A per-pixel terrain flag mask, in top-down bitmap order. Content definitions
/// supply both permanent-snow and city-spawn flags. Permanent snow overrides
/// province weather; undeclared palette indices have neither flag.
pub fn terrain_flag_mask(
    definitions: &TerrainDefinitions,
    terrain: &IndexedImage,
    flag: impl Fn(&installed_content_core::terrain::TerrainType) -> bool,
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

/// Maps declared terrain palette indices to atlas tiles.
///
/// Reached undeclared indices and reached tiles outside the atlas use tile zero
/// and report their index and pixel counts. Unused invalid entries do not warn.
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
    let mut out_of_range = Vec::new();
    for (&index, kind) in &definitions.palette {
        if kind.atlas_index < ATLAS_TILES {
            lookup[index as usize] = kind.atlas_index as u8;
        } else if counts[index as usize] > 0 {
            out_of_range.push((index, kind.atlas_index, counts[index as usize]));
        }
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
    for (index, atlas, drawn) in out_of_range {
        warnings.push(format!(
            "common/terrain gives palette index {index} texture {atlas}, which is outside the {ATLAS_TILES}-tile atlas; its {drawn} pixels of map/terrain.bmp are drawn with atlas tile 0."
        ));
    }
    lookup
}

/// Builds the four-channel TerrainIDMap payload. Channels hold the upper, right,
/// base and upper-right atlas tiles in that order. The base channel adds 100
/// when all four agree. Rows run bottom-up; horizontal neighbours wrap and
/// vertical neighbours clamp at the pole.
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
        atlas_of_palette[indices[row * w + x % w] as usize]
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
            // `calculate_map_tex_index` subtracts 100 back off the z channel.
            texel[2] = if same { base + 100 } else { base };
            texel[3] = up_right;
        }
    }
    Ok(out)
}

/// An all-clear grid for disabled fog or unavailable visibility. Validate the
/// province image without demanding the content's cell-composition table.
pub fn clear_intel_map(map: &MapData) -> io::Result<IntelMap> {
    intel_image_size(map)?;
    let [width, height] = fow_size(MapSize {
        width: map.width,
        height: map.height,
    });
    Ok(IntelMap::new(
        vec![0; width as usize * height as usize],
        false,
    ))
}

/// One hour's `IntelMap` grid and whether it is a recorded fog or the all-clear
/// stand-in; [`IntelCells::intel_map`] builds recorded fog grids.
pub use map_display_data::Intel as IntelMap;

/// The dump's one-line report, beside the weather grid's and the
/// perspective's. `switch` is the fog-of-war switch the frame was asked for.
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
