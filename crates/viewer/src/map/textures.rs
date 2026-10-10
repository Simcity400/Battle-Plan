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
use std::{collections::BTreeMap, io};

use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    gui::native::resources::{self, NoTexture},
    image::{IndexedImage, Rgba8Image, bmp, dds, native::Decode},
    map::{MapData, ProvinceKind},
    terrain::{DefaultMap, TerrainDefinitions},
};

use super::{camera::MapSize, gradient, post::PostAssets};

/// Atlas tiles per axis.
pub const MAP_NUM_TILES: u32 = 4;
/// Tiles an atlas can address, and therefore the valid `texture = N` range.
pub const ATLAS_TILES: u32 = MAP_NUM_TILES * MAP_NUM_TILES;
// weather
/// Map pixels one SnowMudData texel covers on each axis. Generated snow/mud
/// textures remain unpadded; shader globals follow the client's texture-decode
/// settings and retain the independent fog-grid ratio.
pub const SNOW_MUD_BLOCK: u32 = 4;
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

/// Two gradient-border channel textures use map width divided by `gradient::DIVISOR` and stack
/// two downscaled layers, each followed by a zero guard row. `GB_TextureHeight` is one layer's
/// height; see [`gradient_layer_height`].
pub fn gradient_size(map: MapSize) -> [u32; 2] {
    [
        map.width / gradient::DIVISOR,
        2 * (map.height / gradient::DIVISOR + 1),
    ]
}

/// `GB_TextureHeight`: the height of **one layer**, not of the texture.
///
/// Use one layer's height to keep its final sample on the final data row, before the zero guard
/// row. The second layer applies the same span after its layer offset.
pub fn gradient_layer_height(map: MapSize) -> f32 {
    (map.height / gradient::DIVISOR) as f32
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
fn sampled_heights(image: &Rgba8Image, heightmap: &IndexedImage) -> Option<([u32; 2], Vec<u8>)> {
    let texels: Vec<u8> = image.pixels.chunks_exact(4).map(|texel| texel[0]).collect();
    let size = [image.width, image.height];
    (size != [heightmap.width, heightmap.height] || texels != heightmap.indices)
        .then_some((size, texels))
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
    let format = format_of(texture.format, space)?;
    if texture.width == 0 || texture.height == 0 {
        return Err(invalid(format!("{label} has an empty texture extent")));
    }
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
    let _guard = super::shader_tests::GPU_TESTS.lock().unwrap();
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        let adapter = crate::render::select_adapter(&adapters, None).unwrap();
        let (device, queue) = adapter
            .request_device(&crate::render::device_descriptor(&adapter))
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

/// The two point-light textures, shared by every lit effect. Rewritten in place
/// so camera movement does not allocate or rebuild bind groups.
pub struct PointLightMaps {
    pub indices: Bound,
    pub data: Bound,
}

impl PointLightMaps {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> io::Result<Self> {
        use super::pointlights::{DATA_WIDTH, INDEX_SIZE, Tiles};
        let tiles = Tiles::default();
        Ok(Self {
            indices: data_texture(
                device,
                queue,
                "LightIndexMap",
                [INDEX_SIZE, INDEX_SIZE],
                wgpu::TextureFormat::Rgba8Unorm,
                &tiles.indices,
            )?,
            data: data_texture(
                device,
                queue,
                "LightDataMap",
                [DATA_WIDTH, 1],
                wgpu::TextureFormat::Rgba32Float,
                bytemuck::cast_slice(&tiles.data),
            )?,
        })
    }

    pub fn write(&self, queue: &wgpu::Queue, tiles: &super::pointlights::Tiles) {
        use super::pointlights::{DATA_WIDTH, INDEX_SIZE};
        write_data(
            queue,
            &self.indices.texture,
            [INDEX_SIZE, INDEX_SIZE],
            4,
            &tiles.indices,
        );
        write_data(
            queue,
            &self.data.texture,
            [DATA_WIDTH, 1],
            16,
            bytemuck::cast_slice(&tiles.data),
        );
    }
}

/// Every game file the terrain and water passes read, decoded but not uploaded.
///
/// The asset set is the one `texture_quality=0` selects.
pub struct MapAssets {
    /// Shared model creation allowance from the resolved content defines.
    pub mesh_limit: i32,
    /// The provinces bitmap's size, which every map-sized input must match.
    pub map: MapSize,
    /// Terrain diffuse atlas with loaded dimensions and mip levels.
    pub atlas: dds::Texture,
    /// `map/terrain/atlas_normal0.dds`.
    pub atlas_normal: dds::Texture,
    /// `map/terrain/colormap_rgb_cityemissivemask_a.dds`; the alpha is the
    /// city-light mask `TerrainColor.a` (`pdxmap.shader`).
    pub colormap: dds::Texture,
    /// `map/terrain/citylights_rgb_snowmask_a_0.dds`; the alpha is the snow noise.
    pub city_lights: dds::Texture,
    /// `map/terrain/snow_normal_rgb_diffuse_a.dds`.
    pub snow: dds::Texture,
    /// `map/terrain/mud_diffuse_rgb_gloss_a_0.dds`.
    pub mud_diffuse: dds::Texture,
    /// `map/terrain/mud_normal_rgb_spec_a_0.dds`.
    pub mud_normal: dds::Texture,
    /// `map/terrain/colormap_water_0.dds`, the underwater tint.
    ///
    /// Underwater tint retains the loaded texture and its authored sampling coordinates.
    pub colormap_water: dds::Texture,
    /// `map/terrain/underwater_terrain_0.dds`, the sea bed.
    ///
    /// Sea-bed diffuse artwork from resolved content.
    pub underwater: dds::Texture,
    /// `map/terrain/lean1.dds` and `lean2.dds`, the water LEAN normal pair.
    pub lean1: dds::Texture,
    pub lean2: dds::Texture,
    /// `map/terrain/fow_rgb_waterspec_a.dds`; the alpha is the water specular map.
    pub water_specular: dds::Texture,
    /// Naval dominance artwork at physical water texture slot 14.
    pub naval_dominance: dds::Texture,
    pub terrain_shader: std::sync::Arc<shader_pipeline::library::Effect>,
    pub water_shader: std::sync::Arc<shader_pipeline::library::Effect>,
    pub arrow_shader: std::sync::Arc<shader_pipeline::library::Effect>,
    pub movement_arrows: super::movement_arrows::Assets,
    pub order_label_shader: (std::sync::Arc<shader_pipeline::library::Effect>, String),
    pub tree_shader: std::sync::Arc<shader_pipeline::library::Effect>,
    pub sky_shader: std::sync::Arc<shader_pipeline::library::Effect>,
    pub name_shader: std::sync::Arc<shader_pipeline::library::Effect>,
    /// `map/terrain/reflection.dds`, a cube map.
    pub reflection: dds::Texture,
    /// `map/terrain/ice_diffuse.dds`.
    pub ice_diffuse: dds::Texture,
    /// `map/terrain/ice_noise_0.dds`.
    pub ice_noise: dds::Texture,
    /// Fog noise texture bound by map::scene.
    pub fow_noise: dds::Texture,
    /// World-normal bitmap, rows top-down; dimensions come from loaded content.
    pub world_normal: Rgba8Image,
    /// `map/heightmap.bmp` bytes, rows top-down: vertex heights and the water
    /// pass's `HeightTexture`.
    pub heightmap: IndexedImage,
    /// The heightmap texture the passes sample (its size and `Unorm` payload) when it is not
    /// [`Self::heightmap`] as it is, as under `force_pow2_textures` ([`sampled_heights`]).
    height_texture: Option<([u32; 2], Vec<u8>)>,
    /// The `TerrainIDMap` payload, rows bottom-up. See [`terrain_id_map`].
    pub terrain_ids: Vec<u8>,
    // colour
    /// `gfx/posteffect_volumes.txt`, the LUTs it names and the post defines.
    pub post: PostAssets,
    pub shadow_blur: std::sync::Arc<shader_pipeline::library::Effect>,
    // rivers
    /// River bitmap palette indices, rows top-down: the tracer input.
    pub rivers: IndexedImage,
    // trees
    /// Tree bitmap palette indices, rows top-down. The lattice can be coarser than the map;
    /// generation refuses an aspect ratio producing unequal axis pitches.
    pub trees: IndexedImage,
    // weather
    /// One flag per `map/terrain.bmp` pixel, rows top-down: whether the pixel's
    /// terrain type carries `perm_snow`. See [`perm_snow_mask`].
    pub perm_snow: std::sync::Arc<[bool]>,
    // close zoom
    /// One flag per `map/terrain.bmp` pixel, rows top-down: whether the pixel's
    /// terrain type carries `spawn_city`. This supplies the city-sprawl footprint; the city bitmap
    /// selects architecture. See [`terrain_flag_mask`].
    pub spawn_city: Vec<bool>,
}

impl MapAssets {
    /// Reads and decodes the whole set, appending any limitation to `warnings`.
    ///
    /// Everything below `map/` is covered content, so it goes through
    /// [`ResolvedLayers::read`] and is hash-verified.
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        layers: &ResolvedLayers,
        defines: &installed_content::defines::Defines,
        map: MapSize,
        definitions: &TerrainDefinitions,
        default_map: &DefaultMap,
        warnings: &mut Vec<String>,
        quality: u8,
        decode: Decode,
    ) -> io::Result<Self> {
        if quality > 2 {
            return Err(invalid("Invalid texture quality"));
        }
        super::shader_sources::check(layers, warnings)?;
        warnings.push("Selected arrow routes use viewer-local selection and animation; particle retained state follows the viewer's submitted world, arrows, labels and meshes.".into());
        let read = |path: &[u8]| layers.read_path(path);
        let texture = |name: &str| load_file(&format!("map/terrain/{name}"), &read, decode);
        let terrain = bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.terrain))?)
            .map_err(|error| invalid(format!("map/{}: {error}", default_map.terrain)))?;
        let heightmap =
            bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.heightmap))?)
                .map_err(|error| invalid(format!("map/{}: {error}", default_map.heightmap)))?;
        // Load the sampled heightmap as a texture file.
        let height_texture =
            sampled_heights(&load_image("map/heightmap.bmp", &read, decode)?, &heightmap);
        let world_normal = installed_content::terrain::read_world_normal(layers, decode)?;
        // rivers
        let rivers = bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.rivers))?)
            .map_err(|error| invalid(format!("map/{}: {error}", default_map.rivers)))?;
        // The tree_definition names the tree bitmap. Draw an instance on every nonzero pixel, regardless
        // of palette index; automatic terrain-assignment indices do not restrict tree placement.
        let trees =
            bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.tree_definition))?)
                .map_err(|error| {
                    invalid(format!("map/{}: {error}", default_map.tree_definition))
                })?;
        expect_size(
            "map/terrain.bmp",
            terrain.width,
            terrain.height,
            [map.width, map.height],
        )?;
        expect_size(
            "map/heightmap.bmp",
            heightmap.width,
            heightmap.height,
            [map.width, map.height],
        )?;
        // rivers
        expect_size(
            "map/rivers.bmp",
            rivers.width,
            rivers.height,
            [map.width, map.height],
        )?;
        let atlas_of_palette = atlas_lookup(definitions, &terrain, warnings);
        let terrain_ids = terrain_id_map(
            terrain.width,
            terrain.height,
            &terrain.indices,
            &atlas_of_palette,
        )?;
        let reflection = texture("reflection.dds")?;
        if reflection.faces == 1 {
            // Retain DDS view type; do not convert strips into cube faces.
            warnings.push("1 2D reflection texture (map/terrain/reflection.dds): cube samples reproduce the native hardware GPU's zero result; native D3D11 software-driver results are device-dependent.".into());
        }
        Ok(Self {
            map,
            mesh_limit: super::mesh_budget::Budget::read(defines)?,
            atlas: texture(&format!("atlas{quality}.dds"))?,
            atlas_normal: texture(&format!("atlas_normal{quality}.dds"))?,
            colormap: texture("colormap_rgb_cityemissivemask_a.dds")?,
            city_lights: texture(&format!("citylights_rgb_snowmask_a_{quality}.dds"))?,
            snow: texture("snow_normal_rgb_diffuse_a.dds")?,
            mud_diffuse: texture(&format!("mud_diffuse_rgb_gloss_a_{quality}.dds"))?,
            mud_normal: texture(&format!("mud_normal_rgb_spec_a_{quality}.dds"))?,
            colormap_water: texture(&format!("colormap_water_{quality}.dds"))?,
            underwater: texture(&format!("underwater_terrain_{quality}.dds"))?,
            lean1: texture("lean1.dds")?,
            lean2: texture("lean2.dds")?,
            water_specular: texture("fow_rgb_waterspec_a.dds")?,
            naval_dominance: texture("naval_dominance_fx.dds")?,
            terrain_shader: super::terrain_shader::load(layers, [map.width, map.height], decode)?,
            water_shader: super::water_shader::load(layers, [map.width, map.height], decode)?,
            sky_shader: super::sky_shader::load(layers, [map.width, map.height], decode)?,
            name_shader: super::name_shader::load(layers, [map.width, map.height], decode)?,
            arrow_shader: super::arrow_shader::load(
                layers,
                [map.width, map.height],
                decode,
                "map arrows",
            )?,
            movement_arrows: super::movement_arrows::Assets::load(
                layers,
                defines,
                [map.width, map.height],
                decode,
            )?,
            order_label_shader: (
                super::arrow_shader::load(layers, [map.width, map.height], decode, "order labels")?,
                super::orders::ArrowTypes::parse(
                    &layers.read_uncovered(super::orders::MAPARROWS)?,
                )?
                .text_technique,
            ),
            tree_shader: super::tree_caster::load(layers, [map.width, map.height], decode)?,
            reflection,
            ice_diffuse: texture("ice_diffuse.dds")?,
            ice_noise: texture(&format!("ice_noise_{quality}.dds"))?,
            fow_noise: texture(&format!("fow_noise_{quality}.dds"))?,
            world_normal,
            heightmap,
            height_texture,
            terrain_ids,
            // colour
            post: PostAssets::load(layers, defines, map, decode)?,
            shadow_blur: super::shadow_blur::load(layers, [map.width, map.height], decode)?,
            // rivers
            rivers,
            // trees
            trees,
            // weather
            perm_snow: terrain_flag_mask(definitions, &terrain, |kind| kind.perm_snow).into(),
            // close zoom
            spawn_city: terrain_flag_mask(definitions, &terrain, |kind| kind.spawn_city),
        })
    }

    /// The heightmap bytes as an `Unorm` payload, rows top-down.
    ///
    /// `pdxwater.shader` divides the sample by `95.7 / 255`, so the byte
    /// must reach the shader unscaled.
    pub fn heightmap_texels(&self) -> &[u8] {
        &self.heightmap.indices
    }

    /// The size and `Unorm` payload of the heightmap texture the passes sample
    /// (`HeightTexture`, `HeightMap`): the file as the game loads it ([`sampled_heights`]),
    /// padded when the client's `force_pow2_textures` pads it. Every reader takes `.x`, which the
    /// padding's zeroed texel holds as 0 in either native zero.
    pub fn height_texture(&self) -> ([u32; 2], &[u8]) {
        match &self.height_texture {
            Some((size, texels)) => (*size, texels),
            None => (
                [self.heightmap.width, self.heightmap.height],
                self.heightmap_texels(),
            ),
        }
    }
}

/// A per-pixel terrain flag mask, in top-down bitmap order. Content definitions
/// supply both permanent-snow and city-spawn flags. Permanent snow overrides
/// province weather; undeclared palette indices have neither flag.
fn terrain_flag_mask(
    definitions: &TerrainDefinitions,
    terrain: &IndexedImage,
    flag: impl Fn(&installed_content::terrain::TerrainType) -> bool,
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
fn atlas_lookup(
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

/// IntelMap is the per-province fog input, with one byte per texel.
///
/// Average the in-bounds province-image pixels in each six-by-six block using
/// integer division, then complement the byte. Water and disabled fog contribute
/// 255 before averaging. The output retains bottom-up row order.
///
/// Fog bytes complement the truncated block average. Only land contributes visibility values; water
/// contributes 255. Boundary blocks average all pixels, forming the six-pixel fog ramp before linear
/// sampling.
///
/// Visibility holds rebuilt province bytes for the viewing country. None means disabled fog or
/// incomplete inputs and produces an all-clear grid. Unavailable historical inputs remain diagnosed
/// limitations.
///
/// Output rows run bottom-up, with row zero at world z zero. Source province-image rows run north-
/// first.
pub fn intel_map(map: &MapData, visibility: Option<&BTreeMap<u32, u8>>) -> io::Result<IntelMap> {
    let (width, height) = (map.width as usize, map.height as usize);
    if map.province_ids.len() != width * height || width == 0 || height == 0 {
        return Err(invalid(format!(
            "the province image is {}x{} but carries {} ids",
            map.width,
            map.height,
            map.province_ids.len()
        )));
    }
    // The size is not checked here: `super::scene::Scene::set_fog_of_war`
    // refuses a grid its texture cannot hold, and naming it once is enough.
    let grid_width = map.width / FOW_BLOCK;
    let grid_height = map.height / FOW_BLOCK;
    let mut texels = vec![0u8; grid_width as usize * grid_height as usize];
    let Some(visibility) = visibility else {
        return Ok(IntelMap { texels, fog: false });
    };
    // The land bit, per province id, so the inner loop is two array reads.
    let largest = map.definitions.keys().copied().max().unwrap_or(0);
    let mut land_of = vec![false; largest as usize + 1];
    for (id, definition) in &map.definitions {
        land_of[*id as usize] = definition.kind == ProvinceKind::Land;
    }
    // The recorded byte per province id. A province the hour does not record is
    // not a recorded zero: the engine would have read whatever its own buffer
    // held, and the honest stand-in for "no byte" is the unfogged 0xff, which
    // claims nothing the recording does not hold. `super::tables` names the
    // hours where that matters.
    let mut byte_of = vec![u8::MAX; largest as usize + 1];
    for (province, seen) in visibility {
        if let Some(slot) = byte_of.get_mut(*province as usize) {
            *slot = *seen;
        }
    }
    for cell_z in 0..grid_height as usize {
        for cell_x in 0..grid_width as usize {
            let mut sum = 0u32;
            let mut count = 0u32;
            for step_z in 0..FOW_BLOCK as usize {
                let world_z = cell_z * FOW_BLOCK as usize + step_z;
                if world_z >= height {
                    break;
                }
                for step_x in 0..FOW_BLOCK as usize {
                    let column = cell_x * FOW_BLOCK as usize + step_x;
                    if column >= width {
                        break;
                    }
                    let province = map.province_ids[(height - 1 - world_z) * width + column];
                    let land = land_of.get(province as usize).copied().unwrap_or(false);
                    sum += u32::from(if land {
                        byte_of.get(province as usize).copied().unwrap_or(u8::MAX)
                    } else {
                        u8::MAX
                    });
                    count += 1;
                }
            }
            if let Some(mean) = sum.checked_div(count) {
                texels[cell_z * grid_width as usize + cell_x] = !(mean as u8);
            }
        }
    }
    Ok(IntelMap { texels, fog: true })
}

/// One hour's `IntelMap` grid and whether it is a recorded fog or the all-clear
/// stand-in; [`intel_map`] builds it.
pub struct IntelMap {
    /// [`fow_size`] texels, one byte each, rows bottom-up.
    pub texels: Vec<u8>,
    /// Whether this frame draws fog of war at all, which is what
    /// `map::frame::fow_fade_factor` gates on.
    pub fog: bool,
}

impl IntelMap {
    /// The dump's one-line report, beside the weather grid's and the
    /// perspective's. `switch` is the fog-of-war switch the frame was asked for.
    pub fn line(
        &self,
        defines: &super::camera::MapDefines,
        camera_height: f32,
        switch: bool,
    ) -> String {
        if !switch {
            return "Fog of war: switched off, so none is drawn, as the game's fow console \
                    switch draws none"
                .into();
        }
        if !self.fog {
            return "Fog of war: none drawn; this frame has no recorded per-province visibility \
                    for its viewing country"
                .into();
        }
        let fogged = self.texels.iter().filter(|&&b| b > 0).count();
        let mean = self.texels.iter().map(|&b| f64::from(b)).sum::<f64>()
            / self.texels.len().max(1) as f64;
        format!(
            "Fog of war: {fogged} of {} intel texels carry fog ({:.1}% of the map), mean intel \
             {:.3}; FOWFadeFactor {:.4} at camera height {camera_height:.1}",
            self.texels.len(),
            100.0 * fogged as f64 / self.texels.len().max(1) as f64,
            mean / 255.0,
            super::frame::fow_fade_factor(defines, camera_height, true),
        )
    }
}

fn expect_size(name: &str, width: u32, height: u32, expected: [u32; 2]) -> io::Result<()> {
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
