//! Immutable world texture and shader content. The host owns ordered loading.
use super::post::PostAssets;
use installed_content_image::{IndexedImage, Rgba8Image, dds};
use std::sync::Arc;
use viewer_foundation::map::camera::MapSize;

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
    pub terrain_shader: Arc<shader_pipeline::library::Effect>,
    pub water_shader: Arc<shader_pipeline::library::Effect>,
    pub arrow_shader: Arc<shader_pipeline::library::Effect>,
    pub movement_arrows: crate::map::movement_arrows::Assets,
    pub order_label_shader: (Arc<shader_pipeline::library::Effect>, String),
    pub tree_shader: Arc<shader_pipeline::library::Effect>,
    pub sky_shader: Arc<shader_pipeline::library::Effect>,
    pub name_shader: Arc<shader_pipeline::library::Effect>,
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
    pub heightmap: Arc<IndexedImage>,
    /// The heightmap texture the passes sample (its size and `Unorm` payload) when it is not
    /// [`Self::heightmap`] as it is, as under `force_pow2_textures` ([`sampled_heights`]).
    pub height_texture: Option<([u32; 2], Vec<u8>)>,
    /// The `TerrainIDMap` payload, rows bottom-up. See [`terrain_id_map`].
    pub terrain_ids: Vec<u8>,
    // colour
    /// `gfx/posteffect_volumes.txt`, the LUTs it names and the post defines.
    pub post: PostAssets,
    pub shadow_blur: Arc<shader_pipeline::library::Effect>,
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
    pub perm_snow: Arc<[bool]>,
    // close zoom
    /// One flag per `map/terrain.bmp` pixel, rows top-down: whether the pixel's
    /// terrain type carries `spawn_city`. This supplies the city-sprawl footprint; the city bitmap
    /// selects architecture. See [`terrain_flag_mask`].
    pub spawn_city: Arc<Vec<bool>>,
    /// The heightmap's sight pyramid, which free-eye map icons test their anchors against;
    /// built on first use ([`MapAssets::terrain_sight`]).
    pub sight: std::sync::OnceLock<viewer_foundation::map::terrain_sight::TerrainSight>,
}

impl MapAssets {
    /// The heightmap bytes as an `Unorm` payload, rows top-down.
    ///
    /// `pdxwater.shader` divides the sample by `95.7 / 255`, so the byte
    /// must reach the shader unscaled.
    pub fn heightmap_texels(&self) -> &[u8] {
        &self.heightmap.indices
    }

    /// The heightmap as the drawn surface reads it.
    pub fn height_field(&self) -> viewer_foundation::map::province_map::HeightField<'_> {
        viewer_foundation::map::province_map::HeightField {
            width: self.heightmap.width,
            height: self.heightmap.height,
            values: &self.heightmap.indices,
        }
    }

    /// The heightmap's sight pyramid, built the first time a free eye shows map icons.
    pub fn terrain_sight(&self) -> &viewer_foundation::map::terrain_sight::TerrainSight {
        self.sight.get_or_init(|| {
            viewer_foundation::map::terrain_sight::TerrainSight::new(&self.height_field())
        })
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
