use viewer_scene_base::post::PostAssets;
use content_reader_image::{IndexedImage, SharedRgba8Image as Rgba8Image, dds};
use std::sync::Arc;
use viewer_gpu::camera::MapSize;

pub struct MapAssets {
    pub mesh_limit: i32,
    pub map: MapSize,
    pub terrain_shader: Arc<shader_pipeline::library::Effect>,
    pub water_shader: Arc<shader_pipeline::library::Effect>,
    pub arrow_shader: Arc<shader_pipeline::library::Effect>,
    pub movement_arrows: crate::movement_arrows::Assets,
    pub order_label_shader: (Arc<shader_pipeline::library::Effect>, String),
    pub tree_shader: Arc<shader_pipeline::library::Effect>,
    pub sky_shader: Arc<shader_pipeline::library::Effect>,
    pub name_shader: Arc<shader_pipeline::library::Effect>,
    pub heightmap: Arc<IndexedImage>,
    pub height_texture: Option<([u32; 2], Vec<u8>)>,
    pub post: PostAssets,
    pub shadow_blur: Arc<shader_pipeline::library::Effect>,
    pub perm_snow: Arc<[bool]>,
    pub spawn_city: Arc<Vec<bool>>,
    pub sight: std::sync::OnceLock<viewer_gpu::terrain_sight::TerrainSight>,
}

pub struct LoadMasks {
    pub rivers: IndexedImage,
    pub trees: IndexedImage,
}

pub struct SceneSources {
    pub atlas: dds::Texture,
    pub atlas_normal: dds::Texture,
    pub colormap: dds::Texture,
    pub city_lights: dds::Texture,
    pub snow: dds::Texture,
    pub mud_diffuse: dds::Texture,
    pub mud_normal: dds::Texture,
    pub colormap_water: dds::Texture,
    pub underwater: dds::Texture,
    pub lean1: dds::Texture,
    pub lean2: dds::Texture,
    pub water_specular: dds::Texture,
    pub naval_dominance: dds::Texture,
    pub reflection: dds::Texture,
    pub ice_diffuse: dds::Texture,
    pub ice_noise: dds::Texture,
    pub fow_noise: dds::Texture,
    pub world_normal: Rgba8Image,
    pub terrain_ids: Vec<u8>,
    pub icon_images: Vec<dds::Texture>,
}

impl MapAssets {
    pub fn heightmap_texels(&self) -> &[u8] {
        &self.heightmap.indices
    }

    pub fn height_field(&self) -> viewer_gpu::province_map::HeightField<'_> {
        viewer_gpu::province_map::HeightField {
            width: self.heightmap.width,
            height: self.heightmap.height,
            values: &self.heightmap.indices,
        }
    }

    pub fn terrain_sight(&self) -> &viewer_gpu::terrain_sight::TerrainSight {
        self.sight.get_or_init(|| {
            viewer_gpu::terrain_sight::TerrainSight::new(&self.height_field())
        })
    }

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
