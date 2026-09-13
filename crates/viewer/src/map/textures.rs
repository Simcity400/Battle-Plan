//! Game textures on the GPU, and the map-sized data textures built on the CPU.
//!
//! The game is a DX9-era pipeline ported to DX11: every texture is sampled raw
//! as `*_UNORM` and the shaders convert with `ToGamma`/`ToLinear` where they
//! need to (R1 section 4.4). None of the formats below are `*Srgb`, and that is
//! deliberate — an sRGB view would apply a second conversion the game does not.

use std::io;

use eframe::wgpu;
use installed_content::{
    ResolvedLayers,
    image::{IndexedImage, Rgba8Image, bmp, dds},
    terrain::{DefaultMap, TerrainDefinitions},
};

use super::camera::{MAP_SIZE_X, MAP_SIZE_Y};

/// `MAP_NUM_TILES` (`constants.fxh:54`): the atlas is 4x4 tiles.
pub const MAP_NUM_TILES: u32 = 4;
/// Tiles an atlas can address, and therefore the valid `texture = N` range.
pub const ATLAS_TILES: u32 = MAP_NUM_TILES * MAP_NUM_TILES;
/// `MAP_SIZE / 6`: the FOW and snow/mud grid (R1 section 3.5).
pub const SNOW_MUD_WIDTH: u32 = 939;
/// `MAP_SIZE / 6` on the second axis.
pub const SNOW_MUD_HEIGHT: u32 = 342;
/// Width of the two gradient-border channel textures.
pub const GRADIENT_WIDTH: u32 = 2816;
/// Height of the two gradient-border channel textures: two stacked half-maps
/// plus the half-texel guards `gradient_border_apply` offsets by.
pub const GRADIENT_HEIGHT: u32 = 2050;

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

/// The wgpu format a decoded DDS uploads as. Always a linear UNORM.
fn format_of(format: dds::TextureFormat) -> wgpu::TextureFormat {
    match format {
        dds::TextureFormat::Bc1 => wgpu::TextureFormat::Bc1RgbaUnorm,
        dds::TextureFormat::Bc2 => wgpu::TextureFormat::Bc2RgbaUnorm,
        dds::TextureFormat::Bc3 => wgpu::TextureFormat::Bc3RgbaUnorm,
        dds::TextureFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
    }
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
) -> io::Result<Bound> {
    let cube = texture.faces == 6;
    if texture.faces != 1 && !cube {
        return Err(invalid(format!(
            "{label} has {} faces; only a plain texture or a cube map can be uploaded",
            texture.faces
        )));
    }
    let format = format_of(texture.format);
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
    for (face, levels) in texture.mips.iter().enumerate() {
        for (level, mip) in levels.iter().enumerate() {
            let row_blocks = mip.width.div_ceil(block);
            let bytes_per_row = row_blocks
                * u32::try_from(texture.format.block_bytes().unwrap_or(4))
                    .map_err(|_| invalid("block size does not fit a u32"))?;
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
                &mip.data,
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
        wgpu::TextureFormat::Rgba8Unorm => 4,
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

/// The three sampler states the map shaders declare.
pub struct Samplers {
    /// `MagFilter/MinFilter/MipFilter = Linear`, `Address = Wrap`, with the
    /// profile's `maxanisotropy=4` (R1 section 4.4).
    pub linear_wrap: wgpu::Sampler,
    /// The same filtering with `Address = Clamp`, as the gradient-border and
    /// refraction samplers declare.
    pub linear_clamp: wgpu::Sampler,
    /// `Point` with `MipFilter = None` and `Address = Clamp`: `TerrainIDMap`.
    pub point_clamp: wgpu::Sampler,
}

impl Samplers {
    pub fn new(device: &wgpu::Device) -> Self {
        let linear = |label, address, anisotropy| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                lod_min_clamp: 0.0,
                lod_max_clamp: 32.0,
                anisotropy_clamp: anisotropy,
                ..Default::default()
            })
        };
        Self {
            linear_wrap: linear("linear wrap", wgpu::AddressMode::Repeat, 4),
            linear_clamp: linear("linear clamp", wgpu::AddressMode::ClampToEdge, 1),
            point_clamp: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("point clamp"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
        }
    }
}

/// Every game file the terrain and water passes read, decoded but not uploaded.
///
/// The asset set is the one `texture_quality=0` selects (R1 section 7).
pub struct MapAssets {
    /// `map/terrain/atlas0.dds`, 2048 squared DXT5 with 10 mips.
    pub atlas: dds::Texture,
    /// `map/terrain/atlas_normal0.dds`.
    pub atlas_normal: dds::Texture,
    /// `map/terrain/colormap_rgb_cityemissivemask_a.dds`; the alpha is the
    /// city-light mask `TerrainColor.a` (`pdxmap.shader:313`).
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
    pub colormap_water: dds::Texture,
    /// `map/terrain/underwater_terrain_0.dds`.
    pub underwater: dds::Texture,
    /// `map/terrain/lean1.dds` and `lean2.dds`, the water LEAN normal pair.
    pub lean1: dds::Texture,
    pub lean2: dds::Texture,
    /// `map/terrain/fow_rgb_waterspec_a.dds`; the alpha is the water specular map.
    pub water_specular: dds::Texture,
    /// `map/terrain/reflection.dds`, a cube map.
    pub reflection: dds::Texture,
    /// `map/terrain/ice_diffuse.dds`.
    pub ice_diffuse: dds::Texture,
    /// `map/terrain/ice_noise_0.dds`.
    pub ice_noise: dds::Texture,
    /// `map/terrain/fow_noise_0.dds`. Decoded so a missing or unreadable file
    /// is reported at load, but not bound: `fow.fxh`'s noise is only sampled by
    /// the `terrainunlit` pre-pass, which the omniscient viewer does not run.
    #[allow(dead_code)]
    pub fow_noise: dds::Texture,
    /// `map/world_normal.bmp` as `HeightNormal`, 2816x1024, rows top-down.
    pub world_normal: Rgba8Image,
    /// `map/heightmap.bmp` bytes, rows top-down: vertex heights and the water
    /// pass's `HeightTexture`.
    pub heightmap: IndexedImage,
    /// The `TerrainIDMap` payload, rows bottom-up. See [`terrain_id_map`].
    pub terrain_ids: Vec<u8>,
}

impl MapAssets {
    /// Reads and decodes the whole set, appending any limitation to `warnings`.
    ///
    /// Everything below `map/` is covered content, so it goes through
    /// [`ResolvedLayers::read`] and is hash-verified.
    pub fn load(
        layers: &ResolvedLayers,
        definitions: &TerrainDefinitions,
        default_map: &DefaultMap,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let texture = |name: &str| -> io::Result<dds::Texture> {
            let path = format!("map/terrain/{name}");
            dds::decode(&layers.read(&path)?).map_err(|error| invalid(format!("{path}: {error}")))
        };
        let terrain = bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.terrain))?)
            .map_err(|error| invalid(format!("map/{}: {error}", default_map.terrain)))?;
        let heightmap =
            bmp::decode_indexed(&layers.read(&format!("map/{}", default_map.heightmap))?)
                .map_err(|error| invalid(format!("map/{}: {error}", default_map.heightmap)))?;
        let world_normal = bmp::decode_rgb(&layers.read("map/world_normal.bmp")?)
            .map_err(|error| invalid(format!("map/world_normal.bmp: {error}")))?;
        expect_size(
            "map/terrain.bmp",
            terrain.width,
            terrain.height,
            [MAP_SIZE_X as u32, MAP_SIZE_Y as u32],
        )?;
        expect_size(
            "map/heightmap.bmp",
            heightmap.width,
            heightmap.height,
            [MAP_SIZE_X as u32, MAP_SIZE_Y as u32],
        )?;
        let atlas_of_palette = atlas_lookup(definitions, &terrain, warnings);
        let terrain_ids = terrain_id_map(
            terrain.width,
            terrain.height,
            &terrain.indices,
            &atlas_of_palette,
        )?;
        Ok(Self {
            atlas: texture("atlas0.dds")?,
            atlas_normal: texture("atlas_normal0.dds")?,
            colormap: texture("colormap_rgb_cityemissivemask_a.dds")?,
            city_lights: texture("citylights_rgb_snowmask_a_0.dds")?,
            snow: texture("snow_normal_rgb_diffuse_a.dds")?,
            mud_diffuse: texture("mud_diffuse_rgb_gloss_a_0.dds")?,
            mud_normal: texture("mud_normal_rgb_spec_a_0.dds")?,
            colormap_water: texture("colormap_water_0.dds")?,
            underwater: texture("underwater_terrain_0.dds")?,
            lean1: texture("lean1.dds")?,
            lean2: texture("lean2.dds")?,
            water_specular: texture("fow_rgb_waterspec_a.dds")?,
            reflection: texture("reflection.dds")?,
            ice_diffuse: texture("ice_diffuse.dds")?,
            ice_noise: texture("ice_noise_0.dds")?,
            fow_noise: texture("fow_noise_0.dds")?,
            world_normal,
            heightmap,
            terrain_ids,
        })
    }

    /// The heightmap bytes as an `R8Unorm` payload, rows top-down.
    ///
    /// `pdxwater.shader:281` divides the sample by `95.7 / 255`, so the byte
    /// must reach the shader unscaled.
    pub fn heightmap_texels(&self) -> &[u8] {
        &self.heightmap.indices
    }
}

/// Maps every palette index of `terrain.bmp` to its atlas tile.
///
/// `common/terrain/00_terrain.txt` gives `texture = N` for the indices it
/// declares. Two limitations are recorded for the document:
///
/// * indices the file never declares, which the game leaves to whatever its
///   own lookup table holds; tile 0 is used here;
/// * `texture = 255` on the `lakes` entry, which no 4x4 atlas can address and
///   which the `+100` "all same" encoding cannot carry either; tile 0 again.
///   Lakes are drawn over by the water plane, so this is not visible.
fn atlas_lookup(
    definitions: &TerrainDefinitions,
    terrain: &IndexedImage,
    warnings: &mut Vec<String>,
) -> [u8; 256] {
    let mut lookup = [0u8; 256];
    let mut out_of_range = Vec::new();
    for (&index, kind) in &definitions.palette {
        if kind.atlas_index < ATLAS_TILES {
            lookup[index as usize] = kind.atlas_index as u8;
        } else {
            out_of_range.push((index, kind.atlas_index));
        }
    }
    let mut used = [false; 256];
    for &index in &terrain.indices {
        used[index as usize] = true;
    }
    let unmapped: Vec<u8> = (0..=255u8)
        .filter(|index| used[*index as usize] && !definitions.palette.contains_key(index))
        .collect();
    if !unmapped.is_empty() {
        warnings.push(format!(
            "map/terrain.bmp uses {} palette indices that common/terrain declares no texture for ({}); they are drawn with atlas tile 0.",
            unmapped.len(),
            unmapped
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (index, atlas) in out_of_range {
        warnings.push(format!(
            "common/terrain gives palette index {index} texture {atlas}, which is outside the {ATLAS_TILES}-tile atlas; it is drawn with atlas tile 0."
        ));
    }
    lookup
}

/// Builds the `TerrainIDMap` payload: the four atlas tiles `PixelShaderTerrain`
/// blends between, plus the `+100` "all same" flag.
///
/// The shader point-samples this map at `uv + (-0.5/MAP_SIZE_X, -0.5/MAP_SIZE_Y)`
/// while `uv` is `(pos + 0.5) / MAP_SIZE`, so a fragment at map position
/// `(fx, fz)` reads texel `(floor(fx), floor(fz))` and blends with
/// `vFrac = frac(fx, fz)` (`pdxmap.shader:234-279`). Evaluating the two nested
/// lerps at the four corners of `vFrac` names each channel:
///
/// | `vFrac` | fragment at | channel |
/// |---|---|---|
/// | `(0, 0)` | `(X, Z)` | `z` (`ColorRU`) |
/// | `(1, 0)` | `(X + 1, Z)` | `y` (`ColorLU`) |
/// | `(0, 1)` | `(X, Z + 1)` | `x` (`ColorRD`) |
/// | `(1, 1)` | `(X + 1, Z + 1)` | `w` (the base sample) |
///
/// The second axis is `pos.y`, which the vertex shader feeds straight into
/// `prepos.z`, so it is world `z` and not a bitmap row: the texture is stored
/// bottom-up, row 0 being world `z = 0` and bitmap row `height - 1`. `x`
/// neighbours wrap because the map wraps; `z` neighbours clamp at the pole.
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
mod tests {
    use super::*;

    fn identity_lookup() -> [u8; 256] {
        let mut lookup = [0u8; 256];
        for (index, value) in lookup.iter_mut().enumerate() {
            *value = index as u8;
        }
        lookup
    }

    #[test]
    fn the_id_map_is_bottom_up_and_names_the_four_blend_neighbours() {
        // Two rows of three, bitmap order (row 0 is north, world z = 1).
        let indices = [1u8, 2, 3, 4, 5, 6];
        let map = terrain_id_map(3, 2, &indices, &identity_lookup()).unwrap();
        let texel = |x: usize, z: usize| &map[(z * 3 + x) * 4..][..4];
        // World z = 0 is bitmap row 1: 4, 5, 6. World z = 1 is row 0: 1, 2, 3.
        // At (x = 0, z = 0): base 4, right 5, up 1, up-right 2.
        assert_eq!(texel(0, 0), [1, 5, 4, 2]);
        // x wraps: the right neighbour of column 2 is column 0.
        assert_eq!(texel(2, 0), [3, 4, 6, 1]);
        // z clamps at the top row, so up and up-right repeat the row itself.
        assert_eq!(texel(0, 1), [1, 2, 1, 2]);
    }

    #[test]
    fn a_uniform_neighbourhood_sets_the_all_same_flag() {
        let indices = [7u8; 9];
        let map = terrain_id_map(3, 3, &indices, &identity_lookup()).unwrap();
        for texel in map.chunks_exact(4) {
            assert_eq!(texel, [7, 7, 107, 7]);
        }
        // `calculate_map_tex_index` recovers the tile and the flag.
        let raw = 107.0f32;
        let all_same = (raw - 98.0).clamp(0.0, 1.0);
        assert_eq!(all_same, 1.0);
        assert_eq!(raw - all_same * 100.0, 7.0);
    }

    #[test]
    fn a_mismatched_bitmap_is_refused() {
        assert!(terrain_id_map(3, 2, &[1, 2, 3], &identity_lookup()).is_err());
        assert!(terrain_id_map(0, 0, &[], &identity_lookup()).is_err());
    }

    #[test]
    fn the_largest_tile_index_survives_the_all_same_encoding() {
        let mut lookup = [0u8; 256];
        lookup[1] = (ATLAS_TILES - 1) as u8;
        let flagged = terrain_id_map(2, 2, &[1, 1, 1, 1], &lookup).unwrap();
        for texel in flagged.chunks_exact(4) {
            assert_eq!(u32::from(texel[2]), ATLAS_TILES - 1 + 100);
            let raw = f32::from(texel[2]);
            let all_same = (raw - 98.0).clamp(0.0, 1.0);
            assert_eq!(raw - all_same * 100.0, f32::from(texel[0]));
        }
        // An unflagged index must stay clear of the shader's 99 threshold.
        let mixed = terrain_id_map(2, 2, &[1, 0, 1, 1], &lookup).unwrap();
        assert!(mixed.chunks_exact(4).all(|texel| texel[2] < 99));
    }
}
