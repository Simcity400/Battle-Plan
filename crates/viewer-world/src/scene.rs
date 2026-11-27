
use shader_pipeline::gpu::stats::CountedPasses as _;
use crate::scene_content::{MapAssets, SceneSources, TerrainSources};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use viewer_gpu::gpu_contract::MAP_SAMPLED_TEXTURES;
use viewer_gpu::gpu_passes::PassTimer;
use viewer_native_gui::lazy_icons::LazyTextures;
use viewer_runtime::presentation::Continuity;
use viewer_gpu::province_admission::side_planes;
use viewer_gpu::province_admission::views;
use viewer_gpu::province_map::ProvinceMap;
use viewer_gpu::province_schedule::{FrameRestore, ProvinceFrame, Shared};
use viewer_gpu::textures::fow_size;
use viewer_runtime::profile_writer::profiling;
use viewer_runtime::profiling::CpuLaps;

use viewer_gpu::gpu_contract::{
    DEPTH_FORMAT,
    OUTPUT_FORMAT,
    SCENE_FORMAT,
    Targets,
    invalid,
    validate_size,
};

use std::io;

#[path = "scene_passes.rs"]
pub mod passes;

#[path = "scene_step.rs"]
mod step;

#[path = "scene_player.rs"]
mod player;
pub use player::PlayerModel;

#[path = "scene_input.rs"]
pub mod input;
pub use input::SceneInput;

#[cfg(any(test, feature = "test-support"))]
use super::scene_faults as failure_tests;

use eframe::wgpu;

use viewer_borders::{
    border_pass::{BorderAssets, BorderFrame, BorderPass, BorderStage},
    strait_shader::StraitVertex,
    borders::BorderMeshes,
    demilitarized::{DemilitarizedPass, Zone},
};
use map_display_data::landscape::Regions;
use viewer_scene_base::{shadow::ShadowPass, line_lod::MARGIN};
use viewer_terrain::water::WaterPass;
use viewer_gpu::{
    camera::Camera,
    frame::{DayNight, FrameConstants},
    textures::{Bound, ColorSpace, data_texture, upload},
};
use viewer_map_text::name_pass::NamePass;
use viewer_scene_base::post::PostPass;
use viewer_runtime::name_draw::Placements;
use viewer_native_gui::sprites::{Batch, IconTextures, SpritePass};
use viewer_terrain::rivers::{RiverAssets, RiverMesh, RiverPass};
use crate::mapframe::FrameAssets;
use viewer_terrain::trees::{TreeAssets, TreeField, TreeFrame, TreeGeometry, TreePass};
use viewer_rail::railways::RailFade;
use viewer_buildings::map_buildings::{BUILDING_TILE, BuildingHistory, MapBuildingAssets};
use viewer_models::{ModelField, ModelFrame, ModelPass, draws_city_sprawl};
use viewer_rail::railways::{Bridges, RailAssets, RailMesh, RailPass};
use viewer_aircraft::order_pass::{OrderPass, PlanSubmissions};
use viewer_buildings::sprawl::Sprawl;
use viewer_gpu::ribbon_content::Mode as ArrowMode;
use viewer_gpu::ribbon_content::{OrderAssets, OrderField};
use viewer_unit_models::unit_models::UnitModelAssets;
use viewer_unit_reads::model_admission::Gates as UnitGates;

macro_rules! model_passes {
    ($scene:expr) => {
        [
            $scene.native_map_frame.as_ref(),
            $scene.sprawl.as_ref(),
            $scene.map_buildings.as_ref(),
            $scene.unit_models.as_ref(),
            $scene.bridges.as_ref(),
            $scene.aircraft.as_ref().map(|(pass, ..)| pass),
            $scene.trains.as_ref().map(|(pass, ..)| pass),
            $scene.player.as_ref().map(|player| &player.pass),
        ]
    };
}

#[cfg(any(test, feature = "test-support"))]
#[path = "scene_proof.rs"]
mod proof;
#[cfg(any(test, feature = "test-support"))]
pub use proof::SceneProof;

struct FrameInputs {
    frame: FrameConstants,
    shadow_matrix: [[f32; 4]; 4],
    light_grid: [f32; 4],
}

pub struct FactionHighlight {
    classes: viewer_borders::border_pass::ClassPlan,
    zone: viewer_borders::demilitarized::ZonePlan,
    hover: viewer_borders::demilitarized::ZonePlan,
    selection: Option<(viewer_borders::demilitarized::ZonePlan, [f32; 4])>,
}

impl FactionHighlight {
    pub fn new(
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
        provinces: &BTreeSet<u32>,
    ) -> Self {
        Self::with_selection(meshes, regions, zone, provinces, None)
    }

    pub fn with_selection(
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
        provinces: &BTreeSet<u32>,
        selection: Option<(&BTreeSet<u32>, [f32; 4])>,
    ) -> Self {
        let highlight = |a, b| provinces.contains(&a) != provinces.contains(&b);
        let selected = |a, b| {
            !highlight(a, b) && selection.is_some_and(|(set, _)| set.contains(&a) != set.contains(&b))
        };
        Self {
            classes: viewer_borders::border_pass::ClassPlan::matching(meshes, regions, |a, b| {
                !zone.splits(a, b) && !highlight(a, b) && !selected(a, b)
            }),
            zone: viewer_borders::demilitarized::ZonePlan::matching(meshes, regions, |a, b| {
                zone.splits(a, b) && !highlight(a, b) && !selected(a, b)
            }),
            hover: viewer_borders::demilitarized::ZonePlan::custom_highlight(
                meshes, regions, highlight,
            ),
            selection: selection.map(|(_, color)| {
                (viewer_borders::demilitarized::ZonePlan::custom_highlight(meshes, regions, selected), color)
            }),
        }
    }
}

#[derive(Default)]
pub struct HourLayers<'a> {
    pub buildings: Option<(
        &'a MapBuildingAssets,
        map_display_data::buildings::Buildings,
        Option<&'a [&'a map_display_data::buildings::Buildings]>,
    )>,
    pub political: Option<&'a Arc<Regions>>,
    pub border_plans: Option<(
        &'a viewer_borders::border_pass::ClassPlan,
        &'a viewer_borders::demilitarized::ZonePlan,
    )>,
    pub snow_mud: Option<(&'a Arc<map_display_data::SnowMud>, Continuity)>,
    pub fog: Option<(&'a map_display_data::Paged<u8>, bool)>,
    pub unit_models: Option<(&'a UnitModelAssets, viewer_unit_history::unit_history::Field, bool)>,
    pub rails: Option<(
        &'a RailAssets,
        &'a RailMesh,
        Option<&'a Arc<viewer_rail::railways::PreparedRailGeometry>>,
        &'a Bridges,
        ModelField,
    )>,
    pub names: Option<&'a Placements>,
    pub orders: Option<(
        &'a OrderAssets,
        &'a OrderField,
        &'a viewer_aircraft::air_orders::Prepared,
        &'a [u64],
    )>,
}

struct OrderParts<'a> {
    orders: &'a mut Option<OrderPass>,
    labels: &'a mut Placements,
    label_passes: &'a mut [NamePass; 3],
    radar: [(
        &'a mut Option<viewer_aircraft::radar_symbols::Pass>,
        &'a mut Option<String>,
    ); 3],
    samples: u32,
    effect: &'a Arc<shader_pipeline::library::Effect>,
}

struct MapBuffer(wgpu::CommandBuffer);

struct ShaderFrame<'a>(&'a std::cell::RefCell<viewer_scene_base::shader_scene::State>);

impl<'a> ShaderFrame<'a> {
    fn new(state: &'a std::cell::RefCell<viewer_scene_base::shader_scene::State>) -> Self {
        state.borrow_mut().begin_frame();
        Self(state)
    }
}

impl Drop for ShaderFrame<'_> {
    fn drop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.stream.cache.release_bind_groups();
        state.begin_frame();
    }
}

#[derive(Clone, Copy, PartialEq)]
struct SurfaceView {
    camera: Camera,
    fog: bool,
    pulses: bool,
    hour: Option<i32>,
    graphics: dashboard_core::protocol::MapGraphics,
    visibility: viewer_foundation::mode_visibility::Visibility,
    no_water: bool,
    interface: bool,
}

pub struct Scene {
    interaction_hour: Option<i32>,
    surface_view: Option<SurfaceView>,
    #[cfg(any(test, feature = "test-support"))]
    pub test_faults: failure_tests::Faults,
    graphics: dashboard_core::protocol::MapGraphics,
    map_size: [u32; 2],
    shadow: ShadowPass,
    atlas: Arc<Bound>,
    atlas_normal: Arc<Bound>,
    colormap: Arc<Bound>,
    city_lights: Arc<Bound>,
    snow: Arc<Bound>,
    mud_diffuse: Arc<Bound>,
    mud_normal: Arc<Bound>,
    colormap_water: Arc<Bound>,
    underwater: Arc<Bound>,
    lean1: Arc<Bound>,
    lean2: Arc<Bound>,
    water_specular: Arc<Bound>,
    naval_dominance: Arc<Bound>,
    intel_map: Bound,
    intel_installed: Option<map_display_data::Paged<u8>>,
    intel_scratch: Vec<u8>,
    settled_builds: bool,
    settled_poses: bool,
    retained_scene: RefCell<viewer_scene_base::shader_scene::State>,
    fow_noise: Arc<Bound>,
    fog: bool,
    visibility: viewer_foundation::mode_visibility::Visibility,
    point_light_maps: viewer_scene_base::pointlights::PointLightMaps,
    point_light_tiles: viewer_scene_base::pointlights::Tiles,
    reflection: Arc<Bound>,
    ice_diffuse: Arc<Bound>,
    ice_noise: Arc<Bound>,
    height_normal: Arc<Bound>,
    heightmap: Arc<Bound>,
    terrain_ids: Arc<Bound>,
    snow_mud: Bound,
    snow_unknown: Bound,
    hatch_heights: Option<Arc<Bound>>,
    unknown_hatch: Option<(u32, Result<viewer_terrain::weather::UnknownHatch, String>)>,
    snow_presentation: viewer_terrain::weather::Presentation,
    snow_render_delta: viewer_terrain::weather::RenderDelta,
    political: Option<viewer_map_modes::political_gpu::View>,
    mode_channels: [Option<[f32; 2]>; 2],
    gradient1: Bound,
    gradient2: Bound,
    secondary: Bound,
    province_colors: Option<viewer_map_modes::political_gpu::ProvinceColors>,
    compiled_terrain: viewer_terrain::terrain_shader::Terrain,
    terrain_null: Arc<Bound>,
    water: WaterPass,
    arrow_effect: Arc<shader_pipeline::library::Effect>,
    tree_effect: Arc<shader_pipeline::library::Effect>,
    sky_effect: Arc<shader_pipeline::library::Effect>,
    sky: Option<Result<viewer_terrain::sky_shader::Sky, String>>,
    arena: shader_pipeline::gpu::Arena,
    pub post: PostPass,
    pub last_animation_time: Option<f32>,
    last_transition_time: Option<f32>,
    actor_motion: viewer_scene_base::actor_motion::Motion,
    poses: viewer_models::poses::Worker,
    pose_jobs: viewer_models::poses::Jobs,
    posed_tick: Option<i64>,
    pub sound_events: Vec<viewer_models::sound_events::SoundEvent>,
    pub sound_refusals: BTreeMap<String, usize>,
    sound_names: viewer_models::sound_events::Names,
    rail_fade: RailFade,
    render_laps: [f64; 4],
    pass_timer: Option<PassTimer>,
    frame_timer: Option<viewer_gpu::gpu_frame_time::FrameTimer>,
    province_frame: Option<(ProvinceFrame, FrameRestore)>,
    pub pass_diagnostics: passes::Diagnostics,
    detail_laps: CpuLaps,
    pass_cpu_laps: CpuLaps,
    pub targets: Option<Targets>,
    cover: viewer_terrain::terrain_shader::Cover,
    cover_for: Option<(Camera, [u32; 2])>,
    admitted_against: viewer_gpu::camera::Hold,
    generation: u64,
    borders: Option<BorderPass>,
    demilitarized: Option<DemilitarizedPass>,
    faction_highlight_regions: Vec<u32>,
    selection_outlined: bool,
    icon_textures: Arc<IconTextures>,
    _shared: Arc<SceneTextures>,
    name_pass: NamePass,
    near_name_pass: NamePass,
    order_label_pass: [NamePass; 3],
    order_labels: Placements,
    sprite_pass: SpritePass,
    empty_batch: Batch,
    empty_placements: Placements,
    rivers: Option<RiverPass>,
    pub native_map_frame: Option<ModelPass>,
    ambient: Option<crate::mapframe::Presentation>,
    strikes: viewer_models::strike_effects::Presentation,
    missile_clock: Option<f32>,
    trees: Option<TreePass>,
    sprawl: Option<ModelPass>,
    city_presentation: Option<viewer_buildings::sprawl::Presentation>,
    map_buildings: Option<ModelPass>,
    building_history: BuildingHistory,
    mesh_limit: i32,
    railways: Option<RailPass>,
    aircraft: Option<(
        ModelPass,
        Arc<viewer_aircraft::aircraft::AircraftAssets>,
        viewer_aircraft::aircraft::AircraftPlan,
    )>,
    trains: Option<(
        ModelPass,
        Arc<viewer_rail::trains::TrainAssets>,
        viewer_rail::trains::TrainPlan,
    )>,
    orders: Option<OrderPass>,
    radar: Option<viewer_aircraft::radar_symbols::Pass>,
    radar_refusal: Option<String>,
    air_ranges: Option<viewer_aircraft::radar_symbols::Pass>,
    air_ranges_refusal: Option<String>,
    air_transfers: Option<viewer_aircraft::radar_symbols::Pass>,
    air_transfers_refusal: Option<String>,
    pub movement_arrows: crate::movement_arrows::Player,
    animated_text: Option<viewer_map_text::animated_map_text::Player>,
    map_text_events: viewer_map_text::animated_map_text::events::Presenter,
    map_text_refusals: dashboard_text::table_access::Refusals,
    weather_effects: Option<viewer_weather_effects::weather_effects::Pass>,
    encode_lane: viewer_scene_base::lane::Lane<wgpu::CommandBuffer>,
    models_lane: viewer_scene_base::lane::Lane<wgpu::CommandBuffer>,
    beside_lane: viewer_scene_base::lane::Lane<step::Beside>,
    bridges: Option<ModelPass>,
    unit_models: Option<ModelPass>,
    unit_gates: Option<UnitGates>,
    pub unit_history: viewer_unit_history::unit_history::History,
    player: Option<player::Player>,
}

#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub size: [u32; 2],
    pub camera: Camera,
    pub day_night: DayNight,
    pub animation_time: f32,
    pub map_time: f32,
    pub transition_time: Option<f32>,
    pub postfx: bool,
    pub no_water: bool,
    pub no_shadows: bool,
    pub overlay: Overlay<'a>,
    pub no_trees: bool,
}

#[derive(Clone, Copy)]
pub struct Overlay<'a> {
    pub names: Option<&'a Placements>,
    pub layer_alpha: f32,
    pub strategic_names: viewer_map_text::name_pass::StrategicNames,
    pub sprites: Option<&'a Batch>,
    pub ui_scale: f32,
    pub interface: bool,
    pub country_names: bool,
    pub ground: Option<viewer_native_gui::free_icons::Ground<'a>>,
}

impl Default for Overlay<'_> {
    fn default() -> Self {
        Self {
            names: None,
            layer_alpha: 0.0,
            strategic_names: viewer_map_text::name_pass::StrategicNames::All,
            sprites: None,
            ui_scale: 1.0,
            interface: true,
            country_names: true,
            ground: None,
        }
    }
}

#[derive(Clone)]
pub struct SceneTextures {
    atlas: Arc<Bound>,
    atlas_normal: Arc<Bound>,
    colormap: Arc<Bound>,
    city_lights: Arc<Bound>,
    snow: Arc<Bound>,
    mud_diffuse: Arc<Bound>,
    mud_normal: Arc<Bound>,
    colormap_water: Arc<Bound>,
    underwater: Arc<Bound>,
    lean1: Arc<Bound>,
    lean2: Arc<Bound>,
    water_specular: Arc<Bound>,
    naval_dominance: Arc<Bound>,
    fow_noise: Arc<Bound>,
    reflection: Arc<Bound>,
    ice_diffuse: Arc<Bound>,
    ice_noise: Arc<Bound>,
    height_normal: Arc<Bound>,
    heightmap: Arc<Bound>,
    terrain_ids: Arc<Bound>,
    hatch_heights: Option<Arc<Bound>>,
    terrain_null: Arc<Bound>,
    icon_textures: Arc<IconTextures>,
}

impl SceneTextures {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &MapAssets,
        sources: &TerrainSources,
        icon_textures: impl FnOnce() -> io::Result<Arc<IconTextures>>,
    ) -> io::Result<Arc<Self>> {
        check_textures(device)?;
        let fow_noise = Arc::new(upload(
            device,
            queue,
            "FOWNoise",
            &sources.fow_noise,
            ColorSpace::Linear,
        )?);
        let water_specular = Arc::new(upload(
            device,
            queue,
            "SpecularMap",
            &sources.water_specular,
            ColorSpace::Linear,
        )?);
        let height_normal = Arc::new(data_texture(
            device,
            queue,
            "HeightNormal (world_normal.bmp)",
            [sources.world_normal.width, sources.world_normal.height],
            wgpu::TextureFormat::Rgba8Unorm,
            &sources.world_normal.pixels,
        )?);
        let (height_size, height_texels) = match &sources.padded_heights {
            Some((size, texels)) => (*size, texels.as_slice()),
            None => ([assets.heightmap.width, assets.heightmap.height], assets.heightmap_texels()),
        };
        let heightmap = Arc::new(data_texture(
            device,
            queue,
            "heightmap",
            height_size,
            wgpu::TextureFormat::R8Unorm,
            height_texels,
        )?);
        let terrain_ids = Arc::new(data_texture(
            device,
            queue,
            "TerrainIDMap",
            [assets.heightmap.width, assets.heightmap.height],
            wgpu::TextureFormat::Rgba8Unorm,
            &sources.terrain_ids,
        )?);
        let hatch_heights = sources
            .padded_heights
            .is_some()
            .then(|| {
                data_texture(
                    device,
                    queue,
                    "unknown snow/mud hatch heights",
                    [assets.heightmap.width, assets.heightmap.height],
                    wgpu::TextureFormat::R8Unorm,
                    assets.heightmap_texels(),
                )
            })
            .transpose()?
            .map(Arc::new);
        Ok(Arc::new(Self {
            atlas: Arc::new(upload(
                device,
                queue,
                "atlas0",
                &sources.atlas,
                ColorSpace::Srgb,
            )?),
            atlas_normal: Arc::new(upload(
                device,
                queue,
                "atlas_normal0",
                &sources.atlas_normal,
                ColorSpace::Linear,
            )?),
            colormap: Arc::new(upload(
                device,
                queue,
                "TerrainColorTint",
                &sources.colormap,
                ColorSpace::Srgb,
            )?),
            city_lights: Arc::new(upload(
                device,
                queue,
                "CityLightsAndSnowNoise",
                &sources.city_lights,
                ColorSpace::Srgb,
            )?),
            snow: Arc::new(upload(
                device,
                queue,
                "SnowTexture",
                &sources.snow,
                ColorSpace::Linear,
            )?),
            mud_diffuse: Arc::new(upload(
                device,
                queue,
                "MudDiffuseGloss",
                &sources.mud_diffuse,
                ColorSpace::Srgb,
            )?),
            mud_normal: Arc::new(upload(
                device,
                queue,
                "MudNormalSpec",
                &sources.mud_normal,
                ColorSpace::Linear,
            )?),
            colormap_water: Arc::new(upload(
                device,
                queue,
                "colormap_water",
                &sources.colormap_water,
                ColorSpace::Srgb,
            )?),
            underwater: Arc::new(upload(
                device,
                queue,
                "underwater_terrain",
                &sources.underwater,
                ColorSpace::Srgb,
            )?),
            lean1: Arc::new(upload(
                device,
                queue,
                "LeanTexture1",
                &sources.lean1,
                ColorSpace::Linear,
            )?),
            lean2: Arc::new(upload(
                device,
                queue,
                "LeanTexture2",
                &sources.lean2,
                ColorSpace::Linear,
            )?),
            water_specular,
            naval_dominance: Arc::new(upload(
                device,
                queue,
                "naval dominance colors",
                &sources.naval_dominance,
                ColorSpace::Srgb,
            )?),
            reflection: Arc::new(upload(
                device,
                queue,
                "ReflectionCubeMap",
                &sources.reflection,
                ColorSpace::Srgb,
            )?),
            ice_diffuse: Arc::new(upload(
                device,
                queue,
                "IceDiffuse",
                &sources.ice_diffuse,
                ColorSpace::Linear,
            )?),
            ice_noise: Arc::new(upload(
                device,
                queue,
                "IceNoise",
                &sources.ice_noise,
                ColorSpace::Linear,
            )?),
            terrain_null: Arc::new(data_texture(
                device,
                queue,
                "terrain null",
                [1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
                &[0; 4],
            )?),
            fow_noise,
            height_normal,
            heightmap,
            terrain_ids,
            hatch_heights,
            icon_textures: icon_textures()?,
        }))
    }
}

pub struct TextureCache(std::sync::Mutex<Uploads>);

struct Uploads {
    first: Option<Box<TerrainSources>>,
    reader: crate::scene_content::TerrainReader,
    icons: Icons,
    device: Option<wgpu::Device>,
    terrain: std::sync::Weak<SceneTextures>,
}

enum Icons {
    Pending(Vec<content_reader_image::dds::Texture>),
    Made(Arc<IconTextures>),
}

impl TextureCache {
    pub fn new(sources: SceneSources) -> Self {
        let SceneSources { terrain, reader, icon_images } = sources;
        Self(std::sync::Mutex::new(Uploads {
            first: Some(Box::new(terrain)),
            reader,
            icons: Icons::Pending(icon_images),
            device: None,
            terrain: std::sync::Weak::new(),
        }))
    }

    pub fn uploaded_icons(&self, device: &wgpu::Device) -> Option<Arc<IconTextures>> {
        let uploads = match self.0.try_lock() {
            Ok(uploads) => uploads,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        match &uploads.icons {
            Icons::Made(icons) if uploads.device.as_ref() == Some(device) => Some(Arc::clone(icons)),
            _ => None,
        }
    }

    pub fn icons(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> io::Result<Arc<IconTextures>> {
        let mut uploads = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if uploads.device.as_ref().is_some_and(|owner| owner != device) {
            return Err(invalid("this map content's art lives on another graphics device"));
        }
        let made = match &uploads.icons {
            Icons::Made(icons) => return Ok(Arc::clone(icons)),
            Icons::Pending(images) => Arc::new(IconTextures::new(device, queue, images)?),
        };
        uploads.icons = Icons::Made(Arc::clone(&made));
        uploads.device = Some(device.clone());
        Ok(made)
    }

    pub fn warm(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &MapAssets,
    ) -> io::Result<Arc<SceneTextures>> {
        let mut uploads = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if uploads.device.as_ref().is_some_and(|owner| owner != device) {
            return Err(invalid(
                "this map content's textures live on another graphics device; the map loads its content again to draw here",
            ));
        }
        if let Some(shared) = uploads.terrain.upgrade() {
            viewer_runtime::load_record::count("scene textures reused", 1, 0);
            return Ok(shared);
        }
        let sources = match uploads.first.take() {
            Some(first) => first,
            None => {
                viewer_runtime::load_record::count("scene textures read again", 1, 0);
                Box::new(uploads.reader.read(&assets.heightmap)?)
            }
        };
        let made = SceneTextures::new(device, queue, assets, &sources, || match &uploads.icons {
            Icons::Made(icons) => Ok(Arc::clone(icons)),
            Icons::Pending(images) => Ok(Arc::new(IconTextures::new(device, queue, images)?)),
        })?;
        viewer_runtime::load_record::count("scene textures uploaded", 1, 0);
        uploads.icons = Icons::Made(Arc::clone(&made.icon_textures));
        uploads.device = Some(device.clone());
        uploads.terrain = Arc::downgrade(&made);
        Ok(made)
    }
}

fn check_textures(device: &wgpu::Device) -> io::Result<()> {
    viewer_map_modes::political_gpu::check_device(device)?;
    if !device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        return Err(invalid(
            "this GPU does not support BC texture compression, which the game's terrain atlas, \
             water and border textures are authored in",
        ));
    }
    Ok(())
}

fn check_device(device: &wgpu::Device, samples: u32) -> io::Result<()> {
    check_textures(device)?;
    match dashboard_core::map_settings::sample_counts() {
        Some(counts) if counts.contains(&samples) => {}
        Some(_) => {
            return Err(invalid(
                "Selected anti-aliasing is unavailable on this graphics device",
            ));
        }
        None => {
            return Err(invalid(
                "the map's graphics device was opened without recording its anti-aliasing \
                 support (gpu_contract::map_device_descriptor)",
            ));
        }
    }
    let sampled = device.limits().max_sampled_textures_per_shader_stage;
    if sampled < MAP_SAMPLED_TEXTURES {
        return Err(invalid(format!(
            "this GPU binds {sampled} sampled textures per shader stage; the map needs {}",
            MAP_SAMPLED_TEXTURES
        )));
    }
    Ok(())
}

impl Scene {
    #[cfg(any(test, feature = "test-support"))]
    pub fn offline_particle_packets(&self) -> BTreeMap<String, usize> {
        self.weather_effects
            .as_ref()
            .map(|w| w.offline_packets.clone())
            .unwrap_or_default()
    }

    pub fn with_textures(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: Arc<SceneTextures>,
        assets: &MapAssets,
        lazy: &Arc<LazyTextures>,
        graphics: dashboard_core::protocol::MapGraphics,
        workers: &viewer_runtime::parallel::Workers,
    ) -> io::Result<Self> {
        check_device(device, graphics.samples)?;
        let samples = graphics.samples;
        let fow = fow_size(assets.map);
        let intel_map = data_texture(
            device,
            queue,
            "IntelMap",
            fow,
            wgpu::TextureFormat::Rgba8Unorm,
            &vec![0u8; (fow[0] * fow[1] * 4) as usize],
        )?;
        let point_light_maps = viewer_scene_base::pointlights::PointLightMaps::new(device, queue)?;
        let shadow = ShadowPass::new(device, &assets.shadow_blur)?;

        let snow_mud_size = map_snow::grid_size(assets.map.width, assets.map.height);
        let snow_mud = data_texture(
            device,
            queue,
            "SnowMudData",
            snow_mud_size,
            wgpu::TextureFormat::Rgba8Unorm,
            &vec![0u8; (snow_mud_size[0] * snow_mud_size[1] * 4) as usize],
        )?;
        let snow_unknown = data_texture(
            device,
            queue,
            "unknown snow/mud cells",
            snow_mud_size,
            wgpu::TextureFormat::R8Unorm,
            &vec![0u8; (snow_mud_size[0] * snow_mud_size[1]) as usize],
        )?;
        let gradient_size = [1, 1];
        let gradient = |label| {
            data_texture(
                device,
                queue,
                label,
                gradient_size,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                &vec![0u8; (gradient_size[0] * gradient_size[1] * 4) as usize],
            )
        };
        let gradient1 = gradient("GradientBorderChannel1")?;
        let gradient2 = gradient("GradientBorderChannel2")?;
        let secondary = data_texture(
            device,
            queue,
            "ProvinceSecondaryColorMap",
            [1, 1],
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &[0, 0, 0, 0],
        )?;

        let _shared = Arc::clone(&textures);
        let SceneTextures {
            atlas,
            atlas_normal,
            colormap,
            city_lights,
            snow,
            mud_diffuse,
            mud_normal,
            colormap_water,
            underwater,
            lean1,
            lean2,
            water_specular,
            naval_dominance,
            fow_noise,
            reflection,
            ice_diffuse,
            ice_noise,
            height_normal,
            heightmap,
            terrain_ids,
            hatch_heights,
            terrain_null,
            icon_textures,
        } = SceneTextures::clone(&textures);
        let name_pass = NamePass::new(
            device,
            samples,
            &icon_textures,
            SCENE_FORMAT,
            &assets.name_shader,
            None,
        )?;
        let mut near_name_pass = NamePass::new(
            device,
            samples,
            &icon_textures,
            SCENE_FORMAT,
            &assets.name_shader,
            None,
        )?;
        near_name_pass.record_as("names near");
        let label_pass = || {
            NamePass::new(
                device,
                samples,
                &icon_textures,
                SCENE_FORMAT,
                &assets.order_label_shader.0,
                Some(&assets.order_label_shader.1),
            )
        };
        let order_label_pass = [label_pass()?, label_pass()?, label_pass()?];
        let mut sprite_pass = SpritePass::new(device, queue, &icon_textures, lazy, OUTPUT_FORMAT)?;
        sprite_pass.defer_builds(true);

        let scene = Self {
            #[cfg(any(test, feature = "test-support"))]
            test_faults: Default::default(),
            graphics,
            map_size: [assets.map.width, assets.map.height],
            shadow,
            atlas,
            atlas_normal,
            colormap,
            city_lights,
            snow,
            mud_diffuse,
            mud_normal,
            colormap_water,
            underwater,
            lean1,
            lean2,
            water_specular,
            naval_dominance,
            intel_map,
            intel_installed: None,
            intel_scratch: Vec::new(),
            settled_builds: false,
            settled_poses: false,
            retained_scene: RefCell::default(),
            fow_noise,
            fog: false,
            visibility: viewer_foundation::mode_visibility::Visibility::POLITICAL,
            point_light_maps,
            point_light_tiles: viewer_scene_base::pointlights::Tiles::default(),
            reflection,
            ice_diffuse,
            ice_noise,
            height_normal,
            heightmap,
            terrain_ids,
            snow_mud,
            snow_unknown,
            hatch_heights,
            unknown_hatch: None,
            snow_presentation: Default::default(),
            snow_render_delta: Default::default(),
            political: None,
            mode_channels: [None; 2],
            gradient1,
            gradient2,
            secondary,
            province_colors: None,
            compiled_terrain: viewer_terrain::terrain_shader::Terrain::new(
                device,
                &assets.terrain_shader,
                &assets.heightmap.indices,
                [assets.map.width, assets.map.height],
                samples,
                workers,
            )?,
            terrain_null,
            water: WaterPass::new(
                &assets.water_shader,
                [assets.map.width, assets.map.height],
                samples,
                SCENE_FORMAT,
            ),
            post: PostPass::new(device, queue, &assets.post, OUTPUT_FORMAT)?,
            sky_effect: assets.sky_shader.clone(),
            arrow_effect: assets.arrow_shader.clone(),
            tree_effect: assets.tree_shader.clone(),
            sky: None,
            arena: shader_pipeline::gpu::Arena::new(device),
            last_animation_time: None,
            last_transition_time: None,
            interaction_hour: None,
            surface_view: None,
            actor_motion: Default::default(),
            poses: viewer_models::poses::Worker::new(),
            pose_jobs: Default::default(),
            posed_tick: None,
            sound_events: Vec::new(),
            sound_refusals: BTreeMap::new(),
            sound_names: Default::default(),
            rail_fade: RailFade::default(),
            render_laps: [0.0; 4],
            pass_timer: PassTimer::new(device, queue),
            frame_timer: viewer_gpu::gpu_frame_time::FrameTimer::new(device, queue),
            province_frame: None,
            pass_diagnostics: Default::default(),
            detail_laps: Default::default(),
            pass_cpu_laps: Default::default(),
            targets: None,
            cover: viewer_terrain::terrain_shader::Cover::Blocks(Vec::new()),
            cover_for: None,
            admitted_against: Default::default(),
            generation: 0,
            borders: None,
            demilitarized: None,
            faction_highlight_regions: Vec::new(),
            selection_outlined: false,
            icon_textures,
            _shared,
            name_pass,
            near_name_pass,
            order_label_pass,
            order_labels: Placements::default(),
            sprite_pass,
            empty_batch: Batch::default(),
            empty_placements: Placements::default(),
            rivers: None,
            native_map_frame: None,
            ambient: None,
            strikes: Default::default(),
            missile_clock: None,
            trees: None,
            sprawl: None,
            city_presentation: None,
            map_buildings: None,
            building_history: BuildingHistory::default(),
            mesh_limit: assets.mesh_limit,
            railways: None,
            trains: None,
            aircraft: None,
            orders: None,
            radar: None,
            radar_refusal: None,
            air_ranges: None,
            air_ranges_refusal: None,
            air_transfers: None,
            air_transfers_refusal: None,
            animated_text: None,
            map_text_events: Default::default(),
            map_text_refusals: Default::default(),
            movement_arrows: crate::movement_arrows::Player::new(
                device,
                queue,
                &assets.movement_arrows,
                samples,
            ),
            weather_effects: None,
            encode_lane: viewer_scene_base::lane::Lane::new("map encode"),
            models_lane: viewer_scene_base::lane::Lane::new("map models encode"),
            beside_lane: viewer_scene_base::lane::Lane::new("map step beside"),
            bridges: None,
            unit_models: None,
            unit_gates: None,
            unit_history: Default::default(),
            player: None,
        };
        Ok(scene)
    }

    pub fn set_weather_effects(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: Arc<viewer_weather_effects::weather_effects::Assets>,
        field: map_display_data::WeatherField,
    ) -> io::Result<()> {
        if self.weather_effects.is_none() {
            self.weather_effects = Some(viewer_weather_effects::weather_effects::Pass::new(
                device,
                self.graphics.samples,
                queue,
                assets,
            )?);
        }
        if let Some(pass) = &mut self.weather_effects {
            pass.set(field);
        }
        Ok(())
    }

    pub fn set_prepared_weather_effects(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: Arc<viewer_weather_effects::weather_effects::Assets>,
        prepared: viewer_weather_effects::weather_effects::Prepared,
        continuity: Continuity,
    ) -> io::Result<viewer_weather_effects::weather_effects::Retired> {
        let pass = match self.weather_effects.take() {
            Some(pass) => pass,
            None => viewer_weather_effects::weather_effects::Pass::new(
                device,
                self.graphics.samples,
                queue,
                assets,
            )?,
        };
        Ok(self
            .weather_effects
            .insert(pass)
            .set_prepared(prepared, continuity))
    }

    pub fn warm_trains(
        &mut self,
        assets: &Arc<viewer_rail::trains::TrainAssets>,
    ) -> io::Result<()> {
        if self.trains.is_none() {
            let pass = ModelPass::new(&assets.library, "supply trains");
            self.trains = Some((
                pass,
                Arc::clone(assets),
                viewer_rail::trains::TrainPlan::default(),
            ));
        }
        Ok(())
    }

    pub fn warm_aircraft(
        &mut self,
        assets: &Arc<viewer_aircraft::aircraft::AircraftAssets>,
    ) -> io::Result<()> {
        if self.aircraft.is_none() {
            let library = assets.library();
            let pass = ModelPass::new(&library, "flying aircraft");
            drop(library);
            self.aircraft = Some((
                pass,
                Arc::clone(assets),
                viewer_aircraft::aircraft::AircraftPlan::default(),
            ));
        }
        Ok(())
    }

    pub fn unit_gates(&self) -> Option<UnitGates> {
        self.unit_gates
    }

    pub fn warm_unit_models(
        &mut self,
        assets: &UnitModelAssets,
    ) -> io::Result<()> {
        if self.unit_models.is_none() {
            let library = assets.library();
            let pass = ModelPass::new(&library, "unit models");
            drop(library);
            self.unit_models = Some(pass);
        }
        self.unit_gates = Some(assets.gates);
        Ok(())
    }

    pub fn warm_map_buildings(
        &mut self,
        assets: &MapBuildingAssets,
    ) -> io::Result<()> {
        if self.map_buildings.is_none() {
            let pass = ModelPass::new(&assets.library, "map buildings");
            self.map_buildings = Some(pass);
        }
        Ok(())
    }

    pub fn warm_bridges(
        &mut self,
        bridges: &Bridges,
    ) -> io::Result<()> {
        if self.bridges.is_none() {
            let pass = ModelPass::new(&bridges.library, "railway bridges");
            self.bridges = Some(pass);
        }
        Ok(())
    }

    pub fn warm_pipelines(&mut self, device: &wgpu::Device) {
        use viewer_terrain::terrain_shader::Mode;
        let high = self.graphics.high_gfx_shaders;
        self.compiled_terrain
            .warm(device, &[Mode::Underwater, Mode::Receiver, Mode::Lit(high)]);
        if self.graphics.water {
            self.water.warm(device, high, self.graphics.water_reflections);
        }
        if let Some(rivers) = self.rivers.as_ref().filter(|_| self.graphics.rivers) {
            rivers.warm(device, high);
        }
        if let Some(railways) = self.railways.as_ref().filter(|_| self.graphics.rails()) {
            railways.warm(device);
        }
        if let Some(trees) = &self.trees {
            trees.warm(device, high);
        }
        if self.graphics.sky && self.sky.is_none() {
            self.sky = Some(sky_pipeline(
                device,
                &self.sky_effect,
                self.graphics.samples,
            ));
        }
        if let Some(text) = &self.animated_text {
            text.warm();
        }
        if let Some(weather) = &mut self.weather_effects {
            weather.warm(device);
        }
        let (map, samples) = (self.map_size, self.graphics.samples);
        for pass in [
            self.native_map_frame.as_ref(),
            self.sprawl.as_ref(),
            self.map_buildings.as_ref(),
            self.bridges.as_ref(),
            self.unit_models.as_ref(),
            self.aircraft.as_ref().map(|(pass, ..)| pass),
            self.trains.as_ref().map(|(pass, ..)| pass),
        ]
        .into_iter()
        .flatten()
        {
            pass.warm_pipelines(device, map, samples);
        }
    }

    pub fn warm_railways(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RailAssets,
    ) -> io::Result<()> {
        if self.railways.is_none() {
            self.railways = Some(RailPass::new(
                device,
                self.graphics.samples,
                queue,
                SCENE_FORMAT,
                assets,
            )?);
        }
        Ok(())
    }

    pub fn warm_orders(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
    ) -> io::Result<()> {
        self.warm_radar(device, queue, assets);
        if self.orders.is_none() {
            self.orders = Some(OrderPass::new(
                device,
                self.graphics.samples,
                queue,
                assets,
                &self.arrow_effect,
            )?);
        }
        Ok(())
    }

    fn warm_radar(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, assets: &OrderAssets) {
        Self::warm_radar_parts(
            [
                (&mut self.radar, &mut self.radar_refusal),
                (&mut self.air_ranges, &mut self.air_ranges_refusal),
                (&mut self.air_transfers, &mut self.air_transfers_refusal),
            ],
            (device, queue),
            (self.graphics.samples, &self.arrow_effect),
            assets,
        );
    }

    fn warm_radar_parts(
        passes: [(
            &mut Option<viewer_aircraft::radar_symbols::Pass>,
            &mut Option<String>,
        ); 3],
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        (samples, effect): (u32, &Arc<shader_pipeline::library::Effect>),
        assets: &OrderAssets,
    ) {
        let pages = [
            (&assets.radar, "Radar symbols"),
            (&assets.air_range, "Air ranges"),
            (&assets.air_transfer, "Air transfer progress"),
        ];
        for ((pass, refusal), (page, label)) in passes.into_iter().zip(pages) {
            if pass.is_some() || refusal.is_some() {
                continue;
            }
            match page.as_ref().map_err(Clone::clone).and_then(|page| {
                viewer_aircraft::radar_symbols::Pass::named(device, queue, samples, page, effect, label)
                    .map_err(|e| e.to_string())
            }) {
                Ok(value) => *pass = Some(value),
                Err(reason) => *refusal = Some(reason),
            }
        }
    }

    pub fn set_air_ranges(&mut self, field: &viewer_aircraft::radar_symbols::Field) {
        if let Some(pass) = &mut self.air_ranges {
            pass.set(field);
            pass.shown = true;
        }
    }

    pub fn set_air_transfers(&mut self, field: &viewer_aircraft::radar_symbols::Field) {
        if let Some(pass) = &mut self.air_transfers {
            pass.set(field);
            pass.shown = true;
        }
    }

    pub fn set_radar(
        &mut self,
        field: &viewer_aircraft::radar_symbols::Field,
        shown: bool,
        hovered: Option<(u32, u32)>,
    ) {
        if let Some(pass) = &mut self.radar {
            pass.set(field);
            pass.shown = shown;
            pass.hovered = hovered;
        }
    }

    pub fn warm_weather(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: Arc<viewer_weather_effects::weather_effects::Assets>,
    ) -> io::Result<()> {
        if self.weather_effects.is_none() {
            self.weather_effects = Some(viewer_weather_effects::weather_effects::Pass::new(
                device,
                self.graphics.samples,
                queue,
                assets,
            )?);
        }
        Ok(())
    }

    pub fn set_map_frame(
        &mut self,
        assets: &FrameAssets,
    ) -> io::Result<()> {
        let mut pass = ModelPass::new(&assets.library, "map frame");
        pass.set_field(&assets.library, assets.field.clone());
        self.native_map_frame = Some(pass);
        self.ambient = Some(assets.presentation.clone());
        self.strikes = viewer_models::strike_effects::Presentation::new(assets.strikes.clone());
        Ok(())
    }

    pub fn set_trees(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &TreeAssets,
        geometry: TreeGeometry,
        field: &TreeField,
    ) -> io::Result<()> {
        let mut pass = TreePass::new(
            device,
            self.graphics.samples,
            queue,
            SCENE_FORMAT,
            assets,
            &self.tree_effect,
        )?;
        pass.upload(device, geometry, field)?;
        self.trees = Some(pass);
        Ok(())
    }

    pub fn set_unit_models(
        &mut self,
        device: &wgpu::Device,
        assets: &UnitModelAssets,
        field: viewer_unit_history::unit_history::Field,
        steady: bool,
    ) -> io::Result<()> {
        Self::install_unit_models(
            (
                &mut self.unit_models,
                &mut self.unit_gates,
                &mut self.unit_history,
            ),
            device,
            assets,
            (field, steady),
            (self.map_size, self.graphics.samples),
        )
    }

    fn install_unit_models(
        (models, gates, history): (
            &mut Option<ModelPass>,
            &mut Option<UnitGates>,
            &mut viewer_unit_history::unit_history::History,
        ),
        device: &wgpu::Device,
        assets: &UnitModelAssets,
        (field, steady): (viewer_unit_history::unit_history::Field, bool),
        (map, samples): ([u32; 2], u32),
    ) -> io::Result<()> {
        let pass = models.get_or_insert_with(|| ModelPass::new(&assets.library(), "unit models"));
        *gates = Some(assets.gates);
        history.set_prepared_with_naval(field.targets, assets.bounds.clone(), steady, field.naval);
        history.set_raids(field.raids);
        pass.set_field(&assets.library(), field.models);
        history.field_replaced();
        pass.warm_added(device, map, samples)
    }

    pub fn settle_models(&mut self) {
        self.unit_history.settle();
    }

    pub fn restart_live_models(&mut self) {
        self.movement_arrows.reset();
        if let Some(text) = &mut self.animated_text {
            text.reset();
        }
        self.map_text_events.reset();
        self.map_text_refusals.clear();
        self.unit_history.restart_live();
        self.building_history.restart_live();
        if let Some(pass) = &mut self.unit_models {
            pass.reset_animation();
        }
        if let Some(pass) = &mut self.map_buildings {
            pass.reset_animation();
        }
    }

    pub fn share_province_schedule(&mut self, schedule: &Shared) {
        self.unit_history.share_schedule(schedule);
        if let Some(text) = &mut self.animated_text {
            text.share_schedule(schedule);
        }
    }

    pub fn install_animated_text(
        &mut self,
        assets: &viewer_map_text::animated_map_text::Assets,
        gui: &viewer_native_gui::native_resources::GuiResources,
    ) {
        self.animated_text = Some(viewer_map_text::animated_map_text::Player::new(
            assets,
            gui,
            self.icon_textures.views(),
            self.graphics.samples,
        ));
    }

    pub fn set_nuclear_impacts<'a>(
        &mut self,
        hours: impl IntoIterator<Item = &'a viewer_models::strike_effects::Impacts>,
        continuity: Continuity,
    ) {
        self.strikes.receive(
            hours,
            continuity == Continuity::Reset,
            self.last_animation_time.unwrap_or(0.0),
        );
    }

    #[expect(clippy::too_many_arguments, reason = "the branch and hour, the seek flag, the hours, the continuity, and the anchor, heights and resources")]
    pub fn present_hours<'a>(
        &mut self,
        at: (replay_data::BranchId, i32),
        seek: bool,
        hours: impl IntoIterator<
            Item = (
                &'a viewer_models::strike_effects::Impacts,
                &'a viewer_map_text::animated_map_text::events::Prepared,
            ),
        >,
        continuity: Continuity,
        anchor: impl Fn(u32) -> Option<[f32; 3]>,
        heights: viewer_gpu::province_map::HeightField<'_>,
        gui: &viewer_native_gui::native_resources::GuiResources,
    ) {
        let time = self.last_animation_time.unwrap_or(0.0);
        let mut continuity = continuity;
        let mut text = self.map_text_events.begin(at, seek);
        for (impacts, prepared) in hours {
            self.strikes
                .receive([impacts], continuity == Continuity::Reset, time);
            let batch = self.map_text_events.present(prepared, text);
            self.strikes.receive_explosions(&batch.explosions, time);
            self.map_text_refusals = batch.refusals;
            if let Some(text) = &mut self.animated_text {
                if batch.reset {
                    text.reset();
                }
                text.enqueue(&anchor, heights, gui, batch.popups);
            }
            continuity = Continuity::Continue;
            text = Continuity::Continue;
        }
    }

    pub fn missing_map_text_events(
        &mut self,
        at: (replay_data::BranchId, i32),
        seek: bool,
        reason: String,
    ) {
        if self.map_text_events.fail(at, seek) {
            self.strikes
                .receive([], true, self.last_animation_time.unwrap_or(0.0));
        }
        if let Some(text) = &mut self.animated_text {
            text.reset();
        }
        self.map_text_refusals.clear();
        dashboard_text::table_access::refuse(&mut self.map_text_refusals, reason);
    }

    pub fn advance_provinces(&mut self, camera: Camera, size: [u32; 2]) {
        self.unit_history.advance_schedule(camera, size);
    }

    pub fn cut_province_frame(&mut self, animation_time: f32) {
        self.restore_province_frame();
        self.province_frame = self.unit_history.take_province_frame(animation_time);
    }

    fn restore_province_frame(&mut self) {
        if let Some((_, restore)) = self.province_frame.take() {
            self.unit_history.untake_province_frame(restore);
        }
    }

    pub fn set_raid_units(&mut self, raids: Vec<(usize, viewer_models::ModelInstance)>) {
        self.unit_history.set_raids(raids);
    }

    pub fn set_player(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        player: Option<PlayerModel<'_>>,
    ) -> io::Result<()> {
        let Some(model) = player else {
            self.player = None;
            return Ok(());
        };
        player::Player::place(&mut self.player, device, queue, model)?;
        if let Some(player) = &self.player {
            player
                .pass
                .warm_added(device, self.map_size, self.graphics.samples)?;
        }
        Ok(())
    }

    pub fn unit_pass(&self) -> Option<&ModelPass> {
        self.unit_models.as_ref()
    }

    pub fn player_drawn(&self) -> u32 {
        self.player.as_ref().map_or(0, |player| player.pass.drawn())
    }

    pub fn player_play_position(&self, key: (usize, u32, u32)) -> Option<Option<(f32, f32)>> {
        self.player.as_ref()?.pass.root_play_position(key)
    }

    pub fn set_aircraft(
        &mut self,
        assets: Arc<viewer_aircraft::aircraft::AircraftAssets>,
        mut plan: viewer_aircraft::aircraft::AircraftPlan,
        continuity: Continuity,
    ) -> io::Result<Option<viewer_aircraft::aircraft::AircraftPlan>> {
        if let Some((pass, _, current)) = &mut self.aircraft {
            plan.carry_presentation(current, continuity)?;
            viewer_runtime::view_record::add("flights", "installs", 1);
            viewer_runtime::view_record::add("flights", "restarts", plan.restarts() as u64);
            if continuity == Continuity::Reset {
                pass.reset_animation();
            }
            Ok(Some(std::mem::replace(current, plan)))
        } else {
            let pass = ModelPass::new(&assets.library(), "flying aircraft");
            self.aircraft = Some((pass, assets, plan));
            Ok(None)
        }
    }

    pub fn warm_added_aircraft(&mut self, device: &wgpu::Device) -> io::Result<()> {
        if let Some((pass, assets, _)) = &mut self.aircraft {
            pass.sync_library(&assets.library());
            pass.warm_added(device, self.map_size, self.graphics.samples)?;
        }
        Ok(())
    }

    pub fn set_trains(
        &mut self,
        assets: Arc<viewer_rail::trains::TrainAssets>,
        mut plan: viewer_rail::trains::TrainPlan,
        continuity: Continuity,
    ) -> io::Result<Option<viewer_rail::trains::TrainPlan>> {
        if let Some((_, _, current)) = &mut self.trains {
            let mut previous = std::mem::take(current);
            plan.carry_presentation(&mut previous, continuity);
            *current = plan;
            Ok(Some(previous))
        } else {
            self.trains = Some((
                ModelPass::new(&assets.library, "supply trains"),
                assets,
                plan,
            ));
            Ok(None)
        }
    }

    pub fn set_bridges(&mut self, bridges: &Bridges, field: ModelField) -> io::Result<()> {
        Self::install_bridges(&mut self.bridges, bridges, field)
    }

    fn install_bridges(
        pass: &mut Option<ModelPass>,
        bridges: &Bridges,
        field: ModelField,
    ) -> io::Result<()> {
        pass.get_or_insert_with(|| ModelPass::new(&bridges.library, "railway bridges"))
            .set_field(&bridges.library, field);
        Ok(())
    }

    pub fn set_orders(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
        field: OrderField,
    ) -> io::Result<()> {
        self.set_order_parts(device, queue, assets, [Some(&field), None, None, None])
            .map(drop)
    }

    pub fn set_order_parts(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
        parts: [Option<&OrderField>; viewer_aircraft::order_pass::ORDER_PARTS],
    ) -> io::Result<viewer_aircraft::order_pass::OrderUpload> {
        Self::install_orders(
            OrderParts {
                orders: &mut self.orders,
                labels: &mut self.order_labels,
                label_passes: &mut self.order_label_pass,
                radar: [
                    (&mut self.radar, &mut self.radar_refusal),
                    (&mut self.air_ranges, &mut self.air_ranges_refusal),
                    (&mut self.air_transfers, &mut self.air_transfers_refusal),
                ],
                samples: self.graphics.samples,
                effect: &self.arrow_effect,
            },
            (device, queue),
            assets,
            parts,
        )
    }

    fn install_orders(
        parts: OrderParts<'_>,
        gpu: (&wgpu::Device, &wgpu::Queue),
        assets: &OrderAssets,
        fields: [Option<&OrderField>; viewer_aircraft::order_pass::ORDER_PARTS],
    ) -> io::Result<viewer_aircraft::order_pass::OrderUpload> {
        let OrderParts {
            orders,
            labels,
            label_passes,
            radar,
            samples,
            effect,
        } = parts;
        let (device, queue) = gpu;
        Self::warm_radar_parts(radar, gpu, (samples, effect), assets);
        let made = orders.is_none();
        if made {
            *orders = Some(OrderPass::new(device, samples, queue, assets, effect)?);
        }
        let Some(pass) = orders else {
            return Ok(Default::default());
        };
        let done = pass.upload_parts(device, queue, fields)?;
        let wanted = fields[0].map(|plan| &plan.labels);
        let same = match wanted {
            Some(wanted) => wanted == &*labels,
            None => *labels == Placements::default(),
        };
        if made || !same {
            *labels = wanted.cloned().unwrap_or_default();
            for pass in label_passes {
                pass.prepare_static(device, labels);
            }
        }
        Ok(done)
    }

    pub fn set_railways(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RailAssets,
        mesh: &RailMesh,
    ) -> io::Result<()> {
        let geometry = viewer_rail::railways::PreparedRailGeometry::for_install(device, mesh, None)?;
        self.set_prepared_railways(device, queue, assets, geometry)?;
        Ok(())
    }

    pub fn set_prepared_railways(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RailAssets,
        geometry: Arc<viewer_rail::railways::PreparedRailGeometry>,
    ) -> io::Result<Arc<viewer_rail::railways::PreparedRailGeometry>> {
        Self::install_railways(
            &mut self.railways,
            (device, queue),
            self.graphics.samples,
            assets,
            geometry,
        )
    }

    fn install_railways(
        railways: &mut Option<RailPass>,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        samples: u32,
        assets: &RailAssets,
        geometry: Arc<viewer_rail::railways::PreparedRailGeometry>,
    ) -> io::Result<Arc<viewer_rail::railways::PreparedRailGeometry>> {
        if railways.is_none() {
            *railways = Some(RailPass::new(device, samples, queue, SCENE_FORMAT, assets)?);
        }
        Ok(railways
            .as_mut()
            .expect("created above")
            .adopt_geometry(geometry))
    }

    pub fn visit_map_buildings(&mut self, cameras: &[Camera], size: [u32; 2]) {
        for camera in cameras {
            self.building_history.visit(*camera, size);
        }
    }

    pub fn set_map_buildings(
        &mut self,
        assets: &MapBuildingAssets,
        prepared: map_display_data::buildings::Buildings,
        passed: Option<&[&map_display_data::buildings::Buildings]>,
    ) -> io::Result<()> {
        Self::install_map_buildings(
            (
                &mut self.map_buildings,
                &mut self.building_history,
                &mut self.city_presentation,
                &mut self.sprawl,
            ),
            assets,
            prepared,
            passed,
        )
    }

    fn install_map_buildings(
        (buildings, history, cities, sprawl): (
            &mut Option<ModelPass>,
            &mut BuildingHistory,
            &mut Option<viewer_buildings::sprawl::Presentation>,
            &mut Option<ModelPass>,
        ),
        assets: &MapBuildingAssets,
        prepared: map_display_data::buildings::Buildings,
        passed: Option<&[&map_display_data::buildings::Buildings]>,
    ) -> io::Result<()> {
        if buildings.is_none() {
            *buildings = Some(ModelPass::new(&assets.library, "map buildings"));
        }
        for hour in passed.unwrap_or_default() {
            let installed = history.install_display(hour, assets)?;
            history.advance(&installed);
            if let Some(presentation) = cities.as_mut() {
                presentation.advance_shared(&installed.city_damage);
            }
        }
        Self::install_city_damage(cities, sprawl, &prepared, passed.is_none());
        let installed = history.install_display(&prepared, assets)?;
        if history.set(installed, passed.is_some())
            && let Some(pass) = buildings
        {
            pass.reset_animation();
        }
        Ok(())
    }

    pub fn set_sprawl(
        &mut self,
        sprawl: &Sprawl,
    ) -> io::Result<()> {
        self.sprawl = Some(ModelPass::new(&sprawl.library, "city sprawl"));
        self.city_presentation = Some(sprawl.presentation());
        if let Some(pass) = &mut self.sprawl {
            let field = sprawl.field.clone();
            pass.set_field(&sprawl.library, field);
        }
        Ok(())
    }

    pub fn set_city_damage(
        &mut self,
        prepared: &map_display_data::buildings::Buildings,
        reset: bool,
    ) {
        Self::install_city_damage(
            &mut self.city_presentation,
            &mut self.sprawl,
            prepared,
            reset,
        );
    }

    fn install_city_damage(
        cities: &mut Option<viewer_buildings::sprawl::Presentation>,
        sprawl: &mut Option<ModelPass>,
        prepared: &map_display_data::buildings::Buildings,
        reset: bool,
    ) {
        if let Some(presentation) = cities {
            if reset {
                presentation.reset();
            }
            presentation.set_shared_damage(&Arc::new(viewer_buildings::sprawl::Damage::from_display(
                &prepared.city_damage,
            )));
        }
        if reset && let Some(pass) = sprawl {
            pass.reset_animation();
        }
    }

    pub fn set_rivers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RiverAssets,
        mesh: &RiverMesh,
    ) -> io::Result<()> {
        let mut pass = RiverPass::new(device, self.graphics.samples, queue, SCENE_FORMAT, assets)?;
        pass.upload(device, mesh)?;
        self.surface_view = None;
        self.rivers = Some(pass);
        Ok(())
    }

    pub fn set_snow_mud(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snow: &Arc<map_display_data::SnowMud>,
        continuity: Continuity,
    ) -> io::Result<()> {
        Self::install_snow_mud(
            (
                &mut self.snow_presentation,
                &mut self.snow_mud,
                &mut self.snow_unknown,
                &mut self.post,
            ),
            (device, queue),
            snow,
            continuity,
        )
    }

    pub fn city_damage_thresholds(&self) -> Option<(i64, i64)> {
        self.city_presentation.as_ref().map(viewer_buildings::sprawl::Presentation::thresholds)
    }

    pub fn holds_country_names(&self, placements: &viewer_runtime::name_draw::Placements) -> bool {
        self.name_pass.holds_placements(placements)
    }

    pub fn country_names_keyed(&self) -> Option<u64> {
        self.name_pass.keyed_generation()
    }

    pub fn begin_country_names(&mut self, generation: u64) {
        self.name_pass.begin_keyed(generation);
    }

    pub fn set_country_names(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        generation: u64,
        key: u64,
        names: &[&viewer_runtime::name_draw::WorldName],
    ) -> io::Result<()> {
        self.name_pass.set_keyed(device, queue, generation, key, names)
    }

    pub fn snow_mud_takes_parts(
        &self,
        snow: &map_display_data::SnowMud,
        continuity: Continuity,
    ) -> bool {
        let texture = &self.snow_mud.texture;
        [texture.width(), texture.height()] == [snow.width, snow.height]
            && self.snow_presentation.takes_parts(snow, continuity)
    }

    pub fn begin_snow_mud_parts(
        &mut self,
        snow: &map_display_data::SnowMud,
        continuity: Continuity,
    ) -> io::Result<()> {
        self.snow_presentation.begin_parts(snow, continuity)
    }

    pub fn write_snow_mud_rect(
        &mut self,
        queue: &wgpu::Queue,
        snow: &map_display_data::SnowMud,
        rows: std::ops::Range<u32>,
        columns: std::ops::Range<u32>,
    ) -> io::Result<()> {
        if rows.is_empty() || columns.is_empty() {
            return Ok(());
        }
        let _take = replay_frame::cpu::scope("snow.rect.take", Some("snow"));
        let bytes = self.snow_presentation.take_rect(snow, rows.clone(), columns.clone())?;
        drop(_take);
        let _write = replay_frame::cpu::scope("snow.rect.write", Some("snow"));
        self.snow_mud.written();
        shader_pipeline::gpu::stats::write_texture(
            queue,
            wgpu::TexelCopyTextureInfo {
                texture: &self.snow_mud.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: columns.start,
                    y: rows.start,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(columns.len() as u32 * 4),
                rows_per_image: Some(rows.len() as u32),
            },
            wgpu::Extent3d {
                width: columns.len() as u32,
                height: rows.len() as u32,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    pub fn finish_snow_mud_parts(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snow: &Arc<map_display_data::SnowMud>,
        continuity: Continuity,
    ) -> io::Result<()> {
        {
            let _finish = replay_frame::cpu::scope("snow.finish", Some("snow"));
            self.snow_presentation.finish_parts(snow, continuity);
        }
        {
            let _hatch = replay_frame::cpu::scope("snow.hatch", Some("snow"));
            Self::update_hatch(device, queue, &mut self.snow_unknown, &mut self.snow_presentation)?;
        }
        let _post = replay_frame::cpu::scope("snow.post", Some("snow"));
        if let Some(factors) = &snow.post_factors {
            self.post.set_snow_factors(factors);
        } else {
            self.post.set_snow(snow.province_snow.try_as_slice()?)?;
        }
        Ok(())
    }

    fn install_snow_mud(
        (presentation, texels, unknown, post): (
            &mut viewer_terrain::weather::Presentation,
            &mut Bound,
            &mut Bound,
            &mut PostPass,
        ),
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        snow: &Arc<map_display_data::SnowMud>,
        continuity: Continuity,
    ) -> io::Result<()> {
        let _shared = replay_frame::cpu::scope("snow.whole.shared", Some("snow"));
        if presentation.set_shared(snow, continuity)? {
            let _write = replay_frame::cpu::scope("snow.whole.write", Some("snow"));
            Self::upload_snow(device, queue, texels, presentation)?;
        }
        drop(_shared);
        {
            let _hatch = replay_frame::cpu::scope("snow.hatch", Some("snow"));
            Self::update_hatch(device, queue, unknown, presentation)?;
        }
        let _post = replay_frame::cpu::scope("snow.post", Some("snow"));
        if let Some(factors) = &snow.post_factors {
            post.set_snow_factors(factors);
        } else {
            post.set_snow(snow.province_snow.try_as_slice()?)?;
        }
        Ok(())
    }

    fn update_hatch(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mask: &mut Bound,
        presentation: &mut viewer_terrain::weather::Presentation,
    ) -> io::Result<()> {
        let size = presentation.size;
        if size[0] == 0 || size[1] == 0 {
            return Ok(());
        }
        if [mask.texture.width(), mask.texture.height()] != size {
            *mask = data_texture(
                device,
                queue,
                "unknown snow/mud cells",
                size,
                wgpu::TextureFormat::R8Unorm,
                &presentation.unknown,
            )?;
        } else if presentation.hatch_whole() {
            mask.write(queue, size, 1, &presentation.unknown);
        } else {
            for range in presentation.hatch_writes() {
                write_range(
                    queue,
                    mask,
                    size[0],
                    1,
                    range.start,
                    &presentation.unknown[range.clone()],
                );
            }
        }
        presentation.clear_hatch();
        Ok(())
    }

    fn upload_snow(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snow_mud: &mut Bound,
        presentation: &mut viewer_terrain::weather::Presentation,
    ) -> io::Result<()> {
        let size = presentation.size;
        if [snow_mud.texture.width(), snow_mud.texture.height()] != size {
            replace(
                device,
                queue,
                "SnowMudData",
                snow_mud,
                wgpu::TextureFormat::Rgba8Unorm,
                size,
                presentation.planes_shown(),
            )?;
        } else {
            let (ranges, planes) = presentation.owed_planes();
            for range in ranges {
                write_range(
                    queue,
                    snow_mud,
                    size[0],
                    4,
                    range.start / 4,
                    &planes[range.clone()],
                );
            }
        }
        presentation.clear_uploads();
        Ok(())
    }

    pub fn set_fog_of_war(
        &mut self,
        queue: &wgpu::Queue,
        texels: &map_display_data::Paged<u8>,
        fog: bool,
    ) -> io::Result<()> {
        Self::install_fog_of_war(
            (
                &self.intel_map,
                &mut self.intel_installed,
                &mut self.intel_scratch,
                &mut self.fog,
            ),
            queue,
            texels,
            fog,
        )
    }

    fn install_fog_of_war(
        (intel_map, installed, scratch, shown): (
            &Bound,
            &mut Option<map_display_data::Paged<u8>>,
            &mut Vec<u8>,
            &mut bool,
        ),
        queue: &wgpu::Queue,
        texels: &map_display_data::Paged<u8>,
        fog: bool,
    ) -> io::Result<()> {
        let size = intel_map.texture.size();
        let expected = (size.width * size.height) as usize;
        if texels.len() != expected {
            return Err(invalid(format!(
                "the intel map needs {expected} bytes for {}x{}, got {}",
                size.width,
                size.height,
                texels.len()
            )));
        }
        if installed.as_ref().is_some_and(|old| texels.ptr_eq(old)) {
            *shown = fog;
            return Ok(());
        }
        let pages = installed.as_ref().map_or_else(
            || (0..texels.pages().len()).collect(),
            |old| texels.changed_pages(old),
        );
        let mut at = 0;
        while at < pages.len() {
            let first = pages[at];
            let mut end = at + 1;
            while end < pages.len() && pages[end] == pages[end - 1] + 1 {
                end += 1;
            }
            scratch.clear();
            for &page in &pages[at..end] {
                scratch.extend(texels.page(page).iter().flat_map(|&alpha| [0, 0, 0, alpha]));
            }
            write_range(
                queue,
                intel_map,
                size.width,
                4,
                first * map_display_data::PAGE_ITEMS,
                scratch,
            );
            at = end;
        }
        *installed = Some(texels.clone());
        *shown = fog;
        Ok(())
    }

    pub fn set_visibility(
        &mut self,
        visibility: viewer_foundation::mode_visibility::Visibility,
    ) {
        self.visibility = visibility;
    }

    pub fn warm_political(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        map: &content_reader_map::map::MapData,
        strategic_regions: &BTreeMap<u32, content_reader_regions::regions::StrategicRegion>,
        field: &viewer_gpu::gradient_math::FieldDefines,
        cache: &viewer_map_modes::political_gpu::Cache,
    ) -> io::Result<()> {
        if self.political.is_some() {
            return Ok(());
        }
        let strategic = viewer_map_modes::political::strategic_regions_of(map, strategic_regions)?;
        let base = cache.warm(
            device,
            queue,
            viewer_map_modes::political_gpu::Inputs {
                map: ProvinceMap {
                    width: map.width,
                    height: map.height,
                    ids: &map.province_ids,
                },
                strategic: &strategic,
                field,
            },
        )?;
        let colors = viewer_map_modes::political_gpu::ProvinceColors::new(device, &base)?;
        self.secondary = colors.bound.share();
        self.province_colors = Some(colors);
        self.political = Some(viewer_map_modes::political_gpu::View::new(base, device, queue)?);
        Ok(())
    }

    pub fn set_air_assignment(
        &mut self,
        queue: &wgpu::Queue,
        field: &viewer_map_modes::air_assignment::Field,
    ) -> io::Result<()> {
        if let Some(colors) = &mut self.province_colors {
            colors.update(queue, &field.provinces)?;
        }
        Ok(())
    }

    pub fn set_strategic_styles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        styles: &BTreeMap<u32, map_display_data::landscape::Style>,
    ) -> io::Result<()> {
        let Some(political) = &mut self.political else {
            return if styles.is_empty() {
                Ok(())
            } else {
                Err(invalid("PoliticalComputeNotWarmed"))
            };
        };
        if political.set_strategic_styles(device, queue, styles)? {
            let [first, second] = political.textures().cloned().expect("shown above");
            self.gradient1 = Bound::of(first);
            self.gradient2 = Bound::of(second);
        }
        Ok(())
    }

    pub fn update_political(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        region_of: &[u32],
        color: &dyn Fn(u32) -> [u8; 3],
        provinces: &[usize],
        keys: &[u32],
    ) -> io::Result<bool> {
        match &mut self.political {
            Some(political) => political.show_changed(device, queue, region_of, color, provinces, keys),
            None => Ok(false),
        }
    }

    pub fn update_naval(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        attributes: &dyn Fn(u32) -> map_display_data::landscape::Attributes,
        keys: &[u32],
    ) -> io::Result<bool> {
        match &mut self.political {
            Some(political) => political.show_naval(device, queue, attributes, keys),
            None => Ok(false),
        }
    }

    pub fn set_political(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        regions: &Arc<Regions>,
    ) -> io::Result<()> {
        Self::install_political(
            (
                &mut self.political,
                &mut self.mode_channels,
                &mut self.gradient1,
                &mut self.gradient2,
            ),
            (device, queue),
            regions,
        )
    }

    fn install_political(
        (political, channels, gradient1, gradient2): (
            &mut Option<viewer_map_modes::political_gpu::View>,
            &mut [Option<[f32; 2]>; 2],
            &mut Bound,
            &mut Bound,
        ),
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        regions: &Arc<Regions>,
    ) -> io::Result<()> {
        let Some(political) = political else {
            return Err(invalid("PoliticalComputeNotWarmed"));
        };
        *channels = regions
            .mode
            .as_ref()
            .map_or([None; 2], |draw| draw.channel_constants());
        if !political.show(device, queue, regions)? {
            return Ok(());
        }
        let [first, second] = political.textures().cloned().expect("shown above");
        *gradient1 = Bound::of(first);
        *gradient2 = Bound::of(second);
        Ok(())
    }

    pub fn set_borders(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &BorderAssets,
        meshes: &BorderMeshes,
        straits: &[StraitVertex],
    ) -> io::Result<()> {
        let mut pass = BorderPass::new(device, self.graphics.samples, queue, SCENE_FORMAT, assets)?;
        pass.upload(device, meshes, straits)?;
        self.surface_view = None;
        self.borders = Some(pass);
        self.demilitarized = Some(DemilitarizedPass::new(
            device,
            self.graphics.samples,
            queue,
            SCENE_FORMAT,
            assets,
        )?);
        self.faction_highlight_regions.clear();
        self.selection_outlined = false;
        Ok(())
    }

    pub fn set_classes(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
    ) -> io::Result<()> {
        self.surface_view = None;
        if let Some(pass) = &mut self.borders {
            pass.set_classes(device, meshes, regions, zone)?;
        }
        if let Some(pass) = &mut self.demilitarized {
            pass.set_zone(device, meshes, regions, zone)?;
            pass.clear_hover();
            pass.set_selection(device, None)?;
        }
        self.faction_highlight_regions.clear();
        self.selection_outlined = false;
        Ok(())
    }

    pub fn set_border_plans(
        &mut self,
        device: &wgpu::Device,
        classes: &viewer_borders::border_pass::ClassPlan,
        zone: &viewer_borders::demilitarized::ZonePlan,
    ) -> io::Result<()> {
        self.surface_view = None;
        Self::install_border_plans(
            (
                &mut self.borders,
                &mut self.demilitarized,
                &mut self.faction_highlight_regions,
                &mut self.selection_outlined,
            ),
            device,
            classes,
            zone,
        )
    }

    fn install_border_plans(
        (borders, demilitarized, highlighted, outlined): (
            &mut Option<BorderPass>,
            &mut Option<DemilitarizedPass>,
            &mut Vec<u32>,
            &mut bool,
        ),
        device: &wgpu::Device,
        classes: &viewer_borders::border_pass::ClassPlan,
        zone: &viewer_borders::demilitarized::ZonePlan,
    ) -> io::Result<()> {
        if let Some(pass) = borders {
            let _border_span = viewer_runtime::profiling::span("ordinary_border_install");
            pass.set_plan(device, classes)?;
        }
        if let Some(pass) = demilitarized {
            let _zone_span = viewer_runtime::profiling::span("dmz_install");
            pass.set_plan(device, zone)?;
            pass.clear_hover();
            pass.set_selection(device, None)?;
        }
        highlighted.clear();
        *outlined = false;
        Ok(())
    }

    pub fn set_hour_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layers: HourLayers<'_>,
    ) -> io::Result<()> {
        let HourLayers {
            buildings,
            political,
            border_plans,
            snow_mud,
            fog,
            unit_models,
            rails,
            names,
            orders,
        } = layers;
        let gpu = (device, queue);
        type Job<'a> = Box<dyn FnOnce() -> io::Result<()> + Send + 'a>;
        let mut jobs: Vec<Job<'_>> = Vec::with_capacity(6);
        if let Some((assets, prepared, passed)) = buildings {
            let parts = (
                &mut self.map_buildings,
                &mut self.building_history,
                &mut self.city_presentation,
                &mut self.sprawl,
            );
            jobs.push(Box::new(move || {
                Self::install_map_buildings(parts, assets, prepared, passed)
            }));
        }
        if let Some(regions) = political {
            let parts = (
                &mut self.political,
                &mut self.mode_channels,
                &mut self.gradient1,
                &mut self.gradient2,
            );
            jobs.push(Box::new(move || {
                Self::install_political(parts, gpu, regions)
            }));
        }
        if let Some((classes, zone)) = border_plans {
            self.surface_view = None;
            let parts = (
                &mut self.borders,
                &mut self.demilitarized,
                &mut self.faction_highlight_regions,
                &mut self.selection_outlined,
            );
            jobs.push(Box::new(move || {
                Self::install_border_plans(parts, device, classes, zone)
            }));
        }
        if let Some((snow, continuity)) = snow_mud {
            let parts = (
                &mut self.snow_presentation,
                &mut self.snow_mud,
                &mut self.snow_unknown,
                &mut self.post,
            );
            jobs.push(Box::new(move || {
                Self::install_snow_mud(parts, gpu, snow, continuity)
            }));
        }
        if let Some((texels, shown)) = fog {
            let parts = (
                &self.intel_map,
                &mut self.intel_installed,
                &mut self.intel_scratch,
                &mut self.fog,
            );
            jobs.push(Box::new(move || {
                Self::install_fog_of_war(parts, queue, texels, shown)
            }));
        }
        if let Some((assets, mesh, prepared, bridges, field)) = rails {
            let (railways, bridge_pass, samples) =
                (&mut self.railways, &mut self.bridges, self.graphics.samples);
            jobs.push(Box::new(move || {
                let geometry = viewer_rail::railways::PreparedRailGeometry::for_install(
                    device, mesh, prepared,
                )?;
                drop(Self::install_railways(
                    railways, gpu, samples, assets, geometry,
                )?);
                Self::install_bridges(bridge_pass, bridges, field)
            }));
        }
        if let Some(placements) = names {
            let (country, near, empty) = (
                &mut self.name_pass,
                &mut self.near_name_pass,
                &self.empty_placements,
            );
            jobs.push(Box::new(move || {
                country.prepare_static(device, placements);
                near.prepare_static(device, placements.near.as_deref().unwrap_or(empty));
                country.prepared_by_install();
                near.prepared_by_install();
                Ok(())
            }));
        }
        if let Some((assets, base, air, highlighted)) = orders {
            let parts = OrderParts {
                orders: &mut self.orders,
                labels: &mut self.order_labels,
                label_passes: &mut self.order_label_pass,
                radar: [
                    (&mut self.radar, &mut self.radar_refusal),
                    (&mut self.air_ranges, &mut self.air_ranges_refusal),
                    (&mut self.air_transfers, &mut self.air_transfers_refusal),
                ],
                samples: self.graphics.samples,
                effect: &self.arrow_effect,
            };
            jobs.push(Box::new(move || {
                let air = air.highlighted(highlighted);
                Self::install_orders(parts, gpu, assets, [Some(base), None, None, Some(&air)])
                    .map(drop)
            }));
        }
        if let Some((assets, field, steady)) = unit_models {
            let parts = (
                &mut self.unit_models,
                &mut self.unit_gates,
                &mut self.unit_history,
            );
            let target = (self.map_size, self.graphics.samples);
            jobs.push(Box::new(move || {
                Self::install_unit_models(parts, device, assets, (field, steady), target)
            }));
        }
        let mut refused = Ok(());
        for job in jobs {
            let result = job();
            if refused.is_ok() {
                refused = result;
            }
        }
        refused
    }

    pub fn faction_highlight_regions(&self) -> &[u32] {
        &self.faction_highlight_regions
    }

    pub fn highlight_installed(&self) -> bool {
        !self.faction_highlight_regions.is_empty() || self.selection_outlined
    }

    pub fn set_faction_highlight(
        &mut self,
        device: &wgpu::Device,
        plan: &FactionHighlight,
        strategic_regions: &[u32],
        color: [f32; 4],
    ) -> io::Result<()> {
        self.surface_view = None;
        if let Some(pass) = &mut self.borders {
            pass.set_plan(device, &plan.classes)?;
        }
        if let Some(pass) = &mut self.demilitarized {
            pass.set_plan(device, &plan.zone)?;
            pass.set_hover(device, &plan.hover, color)?;
            pass.set_selection(device, plan.selection.as_ref().map(|(plan, color)| (plan, *color)))?;
        }
        self.faction_highlight_regions.clear();
        self.faction_highlight_regions
            .extend_from_slice(strategic_regions);
        self.selection_outlined = plan.selection.is_some();
        Ok(())
    }

    pub fn demilitarized_edges(&self) -> usize {
        self.demilitarized
            .as_ref()
            .map_or(0, DemilitarizedPass::edge_count)
    }

    pub fn samples(&self) -> u32 {
        self.graphics.samples
    }

    pub fn settle_builds(&mut self, settled: bool) {
        self.settled_builds = settled;
        self.sprite_pass.defer_builds(!settled);
    }

    pub fn settle_poses(&mut self, settled: bool) {
        self.settled_poses = settled;
    }

    pub fn build_names_in_upload(&mut self, builds: bool) {
        for pass in [&mut self.name_pass, &mut self.near_name_pass]
            .into_iter()
            .chain(&mut self.order_label_pass)
        {
            pass.build_in_upload(builds);
        }
    }

    pub fn warm_gui(
        &mut self,
        device: &wgpu::Device,
        effects: impl IntoIterator<Item = Arc<shader_pipeline::library::Effect>>,
    ) {
        self.sprite_pass.warm(device, effects);
    }

    pub fn set_graphics(&mut self, graphics: dashboard_core::protocol::MapGraphics) {
        self.graphics = dashboard_core::protocol::MapGraphics {
            samples: self.graphics.samples,
            ..graphics
        };
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: Request<'_>,
    ) -> io::Result<()> {
        let _cost = replay_frame::cpu::scope("viewer", Some("render-submit"));
        shader_pipeline::gpu::stats::mark_window_thread();
        let mut timer = self.pass_timer.take();
        if timer.is_none() && viewer_gpu::gpu_passes::recorded() {
            PassTimer::unavailable();
        }
        let rendered = viewer_scene_base::lane::Frame::new(self, Self::join_lanes).render_frame(
            device,
            queue,
            request,
            &mut timer,
        );
        if rendered.is_err() {
            let bakes = self.sprite_pass.bake_buffers();
            if !bakes.is_empty() {
                queue.submit(bakes);
            }
            if let Some(timer) = &mut timer {
                timer.abandon();
            }
            if let Some(frame_timer) = &mut self.frame_timer {
                frame_timer.abandon();
            }
            self.shadow.abandon();
        }
        self.pass_timer = timer;
        self.restore_province_frame();
        rendered
    }

    fn join_lanes(&mut self) {
        drop(self.encode_lane.take());
        drop(self.models_lane.take());
        if let Some(beside) = self.beside_lane.take() {
            Self::take_back(beside, &mut self.aircraft, &mut self.sprawl, &mut self.city_presentation);
        }
        if let Some(weather) = &mut self.weather_effects {
            weather.join_frame();
        }
    }

    fn take_back(
        beside: step::Beside,
        aircraft: &mut Option<(
            ModelPass,
            Arc<viewer_aircraft::aircraft::AircraftAssets>,
            viewer_aircraft::aircraft::AircraftPlan,
        )>,
        sprawl: &mut Option<ModelPass>,
        city_presentation: &mut Option<viewer_buildings::sprawl::Presentation>,
    ) {
        *aircraft = beside.aircraft;
        *sprawl = beside.sprawl;
        *city_presentation = beside.city_presentation;
        viewer_runtime::profiling::adopt_thread_records(beside.records);
    }

    fn count_shown_actors(
        motion: &mut viewer_scene_base::actor_motion::Motion,
        passes: [(&'static str, Option<&ModelPass>); 6],
        camera: Camera,
        size: [u32; 2],
        ground: Option<[f32; 4]>,
    ) {
        if !viewer_runtime::view_record::on() {
            return;
        }
        let offsets = viewer_scene_base::line_lod::wrap_offsets(ground, camera.map.x());
        let mut shown = Vec::new();
        motion.begin();
        for (label, pass) in passes {
            let Some(pass) = pass else {
                continue;
            };
            shown.clear();
            pass.shown_actors(ground, &offsets, &mut shown);
            motion.count(label, &shown, &camera, size);
        }
        motion.end();
    }

    fn render_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: Request<'_>,
        timer: &mut Option<PassTimer>,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        let faults = &self.test_faults;
        #[cfg(any(test, feature = "test-support"))]
        faults.begin_frame();
        let Request {
            size,
            camera,
            day_night,
            animation_time,
            map_time,
            transition_time,
            postfx,
            no_water,
            no_shadows,
            overlay,
            no_trees,
        } = request;
        let transition_clock = transition_time.unwrap_or(animation_time);
        let transition_delta = self
            .last_transition_time
            .map_or(0.0, |last| (transition_clock - last).max(0.0));
        let transition_reset = self
            .last_transition_time
            .is_some_and(|last| transition_clock < last);
        let no_trees = no_trees || !self.graphics.trees;
        let no_shadows = no_shadows || !self.graphics.shadows;
        camera.validate()?;
        let weather_delta = self.snow_render_delta.advance(transition_clock);
        if self
            .snow_presentation
            .advance(transition_clock, weather_delta)?
        {
            Self::upload_snow(
                device,
                queue,
                &mut self.snow_mud,
                &mut self.snow_presentation,
            )?;
        }
        Self::update_hatch(device, queue, &mut self.snow_unknown, &mut self.snow_presentation)?;
        let snow_mud_fade = self.snow_presentation.fade();
        for model in model_passes!(self).into_iter().flatten() {
            model.settle_builds(self.settled_builds);
        }
        self.compiled_terrain.warm(
            device,
            &viewer_terrain::terrain_shader::terrain_readers(self.graphics.high_gfx_shaders),
        );
        let shadow_screen = self.shadow_screen_wanted(device);
        let mut passes = passes::Frame::new(&mut self.pass_diagnostics);
        self.arena.reset();
        let arena = &self.arena;
        let clock = profiling().then(Instant::now);
        self.detail_laps.start(clock);
        self.detail_laps.record_walls();
        let rail_alpha = step_rail_fade(
            &mut self.rail_fade,
            &camera,
            self.railways.is_some(),
            animation_time,
            transition_time,
            arena,
        );
        if self.snow_presentation.changing() {
            arena.request_frame();
        }
        let rails_shown = self.rail_fade.trains_visible();
        let trains_visible = rails_shown && self.graphics.supply_trains();
        let bridges_visible = rails_shown && self.graphics.railway_bridges();
        validate_size(size, device.limits().max_texture_dimension_2d)?;
        let world_size = world_size(size, self.graphics.full_resolution);
        validate_size(world_size, device.limits().max_texture_dimension_2d)?;
        validate_size(
            world_size.map(|side| side / 2),
            device.limits().max_texture_dimension_2d,
        )
        .map_err(|error| invalid(format!("Map post seed: {error}")))?;
        let sea_bed = if !no_water && self.graphics.water {
            self.water.sea_bed(
                device,
                self.graphics.high_gfx_shaders,
                self.graphics.water_reflections,
            )
        } else {
            viewer_gpu::gpu_contract::SeaBed::Half
        };
        let generation_changed = self.targets.as_ref().is_none_or(|t| {
            t.size() != size || t.world_size() != world_size || t.sea_bed() != sea_bed
        });
        if generation_changed
            || !self.shadow.sized(size, shadow_screen)
            || !self.post.prepared(size, world_size)
        {
            let made = viewer_gpu::gpu_contract::scoped(
                device,
                || {
                    format!(
                        "The graphics device could not allocate this map's {}x{} targets",
                        size[0], size[1]
                    )
                },
                || {
                    let made = generation_changed
                        .then(|| {
                            Targets::new(device, size, world_size, self.graphics.samples, sea_bed)
                        });
                    let targets = made
                        .as_ref()
                        .or(self.targets.as_ref())
                        .expect("made or kept");
                    self.shadow.resize(device, size, shadow_screen);
                    self.post.prepare(device, targets, generation_changed);
                    Ok(made)
                },
            );
            let made = match made {
                Ok(made) => made,
                Err(error) => {
                    self.targets = None;
                    self.shadow.release();
                    self.post.release_chain();
                    return Err(error);
                }
            };
            if let Some(targets) = made {
                self.targets = Some(targets);
                self.generation += 1;
                #[cfg(any(test, feature = "test-support"))]
                faults.after(failure_tests::Point::Targets)?;
            }
        }
        let samples = self.targets.as_ref().map_or(1, Targets::samples);
        if self
            .snow_presentation
            .coverage()
            .is_some_and(viewer_terrain::weather::HatchCoverage::covered)
            && self
                .unknown_hatch
                .as_ref()
                .is_none_or(|(built, _)| *built != samples)
        {
            self.unknown_hatch = Some((
                samples,
                viewer_terrain::weather::UnknownHatch::new(device, samples),
            ));
        }
        if self.cover_for != Some((camera, size)) {
            self.cover_for = Some((camera, size));
            self.cover = viewer_terrain::terrain::cover_for(camera, size);
        }
        let (candidates, slack) = self.admitted_against.at(camera, size);
        let cull = Some(viewer_models::GameCull::padded(&candidates, size, slack.pad));
        let seen = Some(viewer_models::GameCull::of(&camera, size));
        for pass in [
            self.unit_models.as_ref(),
            self.map_buildings.as_ref(),
            self.sprawl.as_ref(),
            self.bridges.as_ref(),
            self.native_map_frame.as_ref(),
            self.aircraft.as_ref().map(|aircraft| &aircraft.0),
            self.trains.as_ref().map(|trains| &trains.0),
            self.player.as_ref().map(|player| &player.pass),
        ]
        .into_iter()
        .flatten()
        {
            pass.set_cull(cull, seen);
        }
        let visible = candidates.held_ground(size, MARGIN, slack);
        let seen_ground = camera.visible_ground(size, MARGIN);
        self.detail_laps.lap("targets_shadow_resize");
        let posing = viewer_models::poses::tick(self.arena.frame());
        let pose_tick = posing != self.posed_tick;
        viewer_runtime::view_record::add("poses", "ticks", u64::from(pose_tick));
        let province_frame = self.province_frame.take();
        let beside = step::Beside {
            graphics: self.graphics,
            aircraft: self.aircraft.take(),
            sprawl: self.sprawl.take(),
            city_presentation: self.city_presentation.take(),
            camera,
            size,
            animation_time,
            pose_tick,
            records: None,
        };
        self.beside_lane.start(move || beside.step());
        let (beside_lane, aircraft, sprawl, city_presentation) =
            (&mut self.beside_lane, &mut self.aircraft, &mut self.sprawl, &mut self.city_presentation);
        #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
        let struck = step::Presentation {
            graphics: &self.graphics,
            trains: self.trains.as_mut(),
            unit_history: &mut self.unit_history,
            building_history: &mut self.building_history,
            map_buildings: self.map_buildings.as_mut(),
            unit_models: self.unit_models.as_mut(),
            unit_gates: self.unit_gates,
            visibility: self.visibility,
            mesh_limit: self.mesh_limit,
            movement_arrows: Some(&mut self.movement_arrows),
            animated_text: self.animated_text.as_mut(),
            strikes: &mut self.strikes,
            missile_clock: &mut self.missile_clock,
            map_frame: self.native_map_frame.as_mut(),
            ambient: self.ambient.as_mut(),
            transition_time: transition_clock,
            transition_delta,
            transition_reset,
            scale_delta: transition_time.map(|_| transition_delta),
            province_frame: province_frame.as_ref().map(|(frame, _)| frame),
            pose_tick,
        }
        .step(camera, size, animation_time, rails_shown, |from| {
            let beside = beside_lane.take().expect("the beside step was started");
            Self::take_back(beside, aircraft, sprawl, city_presentation);
            aircraft.as_mut().map(|(_, _, plan)| {
                let _missiles_span = viewer_runtime::profiling::span("step_missiles");
                plan.missile_explosions(from, animation_time)
            })
        });
        if let Some(weather) = &mut self.weather_effects {
            let schedule = self.unit_history.schedule().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            weather.produce(camera.height, &schedule, self.graphics.weather);
        }
        if let Some(player) = &mut self.player
            && pose_tick
        {
            player.pass.animate(animation_time, None, &[0.0]);
        }
        self.detail_laps.lap("presentation_step");
        Self::count_shown_actors(
            &mut self.actor_motion,
            [
                ("unit figures", self.unit_models.as_ref()),
                ("aircraft figures", self.aircraft.as_ref().map(|aircraft| &aircraft.0)),
                ("train figures", self.trains.as_ref().map(|trains| &trains.0)),
                ("city figures", self.sprawl.as_ref()),
                ("building figures", self.map_buildings.as_ref()),
                ("map frame figures", self.native_map_frame.as_ref()),
            ],
            camera,
            size,
            seen_ground,
        );
        self.detail_laps.lap("actors_shown");
        if pose_tick {
            self.posed_tick = posing;
        }
        let frame_interval = self
            .last_animation_time
            .map_or(0.0, |last| (animation_time - last).max(0.0));
        let made = self.poses.take(&mut self.pose_jobs);
        let (refused, free) = (self.poses.refusal().is_some(), self.poses.free());
        let stopped = self.settled_poses;
        let jobs = &mut self.pose_jobs;
        let pose_offsets = viewer_scene_base::line_lod::wrap_offsets(seen_ground, camera.map.x());
        let posed_layers = [
            (self.unit_models.as_mut(), self.graphics.unit_figures(), seen_ground),
            (self.map_buildings.as_mut(), self.graphics.buildings, seen_ground),
            (
                self.sprawl.as_mut(),
                self.graphics.city_sprawl && draws_city_sprawl(&camera),
                seen_ground,
            ),
            (self.bridges.as_mut(), bridges_visible, seen_ground),
            (self.native_map_frame.as_mut(), true, None),
            (
                self.aircraft.as_mut().map(|aircraft| &mut aircraft.0),
                self.graphics.aircraft() && viewer_aircraft::aircraft::draws(&camera),
                seen_ground,
            ),
            (self.trains.as_mut().map(|trains| &mut trains.0), trains_visible, seen_ground),
            (self.player.as_mut().map(|player| &mut player.pass), true, None),
        ];
        for (pass, drawn, rectangle) in posed_layers {
            let Some(pass) = pass else {
                continue;
            };
            if drawn {
                pass.make_field_resident(device, queue)?;
            }
            pass.pose_on_the_frame(stopped);
            if let Some(made) = &made {
                pass.take_poses(made);
            } else if pose_tick && refused && !stopped {
                pass.install_poses(Default::default());
            }
            if pose_tick && free && drawn {
                pass.pose_jobs(jobs, rectangle, &pose_offsets);
            }
            pass.judge_poses(animation_time, frame_interval);
        }
        viewer_runtime::view_record::add("poses", "asked", jobs.len() as u64);
        viewer_runtime::view_record::add(
            "poses",
            "worked",
            made.iter().map(viewer_models::poses::Made::made).sum(),
        );
        self.poses.give(animation_time, &mut self.pose_jobs);
        self.detail_laps.lap("poses");
        let offsets = viewer_terrain::terrain::wrap_copies(camera, size);
        let fog = self.fog && self.visibility.fog_of_war;
        let pulses = self.interaction_hour.is_some()
            && self
                .political
                .as_ref()
                .is_some_and(|view| view.style_pulses());
        let view = SurfaceView {
            camera,
            fog,
            pulses,
            hour: self.interaction_hour,
            graphics: self.graphics,
            visibility: self.visibility,
            no_water,
            interface: overlay.interface,
        };
        let changed = self.surface_view.replace(view) != Some(view);
        self.targets
            .as_mut()
            .expect("created above")
            .surface_motion
            .begin(
                device,
                changed,
                offsets.len(),
                self.interaction_hour.is_some(),
            );
        let targets = self.targets.as_ref().expect("created above");
        let events_copy_span = viewer_runtime::profiling::span("frame_events");
        let events = step::frame_events(
            step::EventSources {
                map_frame: self.native_map_frame.as_ref(),
                sprawl: self.sprawl.as_ref(),
                map_buildings: self.map_buildings.as_ref(),
                unit_models: self.unit_models.as_ref(),
                aircraft: self.aircraft.as_ref().map(|(pass, _, _)| pass),
                trains: self.trains.as_ref().map(|(pass, _, _)| pass),
                player: self.player.as_ref().map(|player| &player.pass),
            },
            &self.graphics,
            &camera,
            animation_time,
            rails_shown,
        );
        drop(events_copy_span);
        let frame_events = if animation_time > 0.0 {
            self.native_map_frame
                .as_ref()
                .map_or(0, |pass| pass.events.len())
        } else {
            0
        };
        self.sound_events.clear();
        viewer_models::sound_events::collect(
            &events[..frame_events],
            &mut self.sound_names,
            &mut self.sound_events,
            |event| struck.contains(&event.key.player()),
        );
        viewer_models::sound_events::collect(
            &events[frame_events..],
            &mut self.sound_names,
            &mut self.sound_events,
            viewer_models::entity_animation::Event::in_combat,
        );
        self.sound_refusals.clear();
        if let Some(pass) = &self.native_map_frame {
            self.sound_refusals.extend(
                pass.default_state_refusals
                    .iter()
                    .map(|(why, count)| (why.clone(), *count)),
            );
        }
        if let Some(weather) = &mut self.weather_effects {
            weather.sound_events(
                camera,
                size,
                animation_time,
                &mut self.sound_names,
                &mut self.sound_events,
            );
        }
        let actor_views = views(camera, size);
        if let Some(weather) = &mut self.weather_effects {
            weather.begin_frame(camera, size, &actor_views, animation_time);
        }
        static PROFILE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *PROFILE.get_or_init(|| std::env::var_os("BATTLEPLAN_ANIMATION_PROFILE").or_else(|| std::env::var_os("HOI4_ANIMATION_PROFILE")).is_some()) {
            let mut effects = BTreeMap::<&str, usize>::new();
            for event in &self.sound_events {
                *effects.entry(&event.effect).or_default() += 1;
            }
            eprintln!(
                "Sound events: entity events={} sounding={} {effects:?}",
                events.len(),
                self.sound_events.len()
            );
        }
        self.detail_laps.lap("events");
        let events_count = events.len() as u64;
        let lights_profile = viewer_runtime::profiling::thread_span("lights");
        let events_span = viewer_runtime::profiling::span("lights_set_events");
        let lights = match self.weather_effects.as_mut() {
            Some(pass) => pass.set_events(events)?,
            None => {
                for kept in events.iter().filter_map(|event| event.kept.as_ref()) {
                    kept.finish();
                }
                Vec::new()
            }
        };
        drop(events_span);
        let grid_span = viewer_runtime::profiling::span("point_light_grid");
        self.point_light_tiles.rebuild(camera, &lights);
        self.point_light_maps.write(queue, &self.point_light_tiles);
        drop(grid_span);
        viewer_runtime::view_record::add("point lights", "lights", lights.len() as u64);
        viewer_runtime::view_record::add("point lights", "admitted", self.point_light_tiles.count as u64);
        viewer_runtime::view_record::add("point lights", "events", events_count);
        drop(lights_profile);
        self.detail_laps.lap("lights");
        let mut globals = FrameInputs {
            frame: FrameConstants::try_new(
                camera,
                size,
                day_night,
                animation_time,
                self.fog && self.visibility.fog_of_war,
            )
            .map(|frame| {
                frame
                    .with_snow_mud_fade(snow_mud_fade)
                    .with_channels(self.mode_channels)
            })
            .ok_or_else(|| {
                invalid(
                    "Map shader inputs: 1 refused frame: camera projection has no finite inverse",
                )
            })?,
            shadow_matrix: [[0.0; 4]; 4],
            light_grid: self.point_light_tiles.grid,
        };
        let [shadow_casters, shadow_receivers] = viewer_scene_base::shadow::stages(&camera, !no_shadows);
        let shadow_prepared = self.shadow.prepare(&globals.frame, camera, size);
        let shadow_matrix = shadow_prepared.map(|prepared| prepared.texture_matrix);
        globals.shadow_matrix = shadow_matrix.unwrap_or([[0.0; 4]; 4]);
        let native_frames = offsets
            .iter()
            .map(|offset| {
                let mut wrapped = camera;
                wrapped.center[0] -= offset;
                FrameConstants::try_new(
                    wrapped,
                    size,
                    day_night,
                    animation_time,
                    self.fog && self.visibility.fog_of_war,
                )
                .map(|frame| {
                    frame
                        .with_snow_mud_fade(snow_mud_fade)
                        .with_channels(self.mode_channels)
                })
                .ok_or_else(|| {
                    invalid(
                        "Map shader scene: 1 refused frame: wrapped camera has no finite inverse",
                    )
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let native_frame_bytes: Vec<_> = native_frames.iter().map(bytemuck::bytes_of).collect();
        let native_mesh_inputs = viewer_models::NativeInputs {
            samples: self.graphics.samples,
            frames: &native_frame_bytes,
            grid: bytemuck::bytes_of(&globals.light_grid),
            textures: [
                &self.snow_mud.view,
                &self.intel_map.view,
                &self.fow_noise.view,
                &self.reflection.view,
                &self.point_light_maps.indices.view,
                &self.point_light_maps.data.view,
                &self.gradient1.view,
                &self.gradient2.view,
                &self.secondary.view,
                &self.water_specular.view,
            ],
        };
        self.detail_laps.lap("globals_shadow_prepare");
        let animated = clock.map(|clock| clock.elapsed());
        let names = overlay.names.unwrap_or(&self.empty_placements);
        self.name_pass.set_strategic_names(overlay.strategic_names);
        self.near_name_pass
            .set_strategic_names(overlay.strategic_names);
        self.name_pass
            .upload(device, names, camera, size, overlay.layer_alpha, &offsets);
        self.near_name_pass.upload(
            device,
            names.near.as_deref().unwrap_or(&self.empty_placements),
            camera,
            size,
            1.0 - overlay.layer_alpha,
            &offsets,
        );
        self.detail_laps.lap("names_upload");
        if let Some(orders) = &mut self.orders {
            let graphics = self.graphics;
            let trade_routes = self.visibility.trade_routes;
            orders.prepare(camera, size, &offsets, |draw| {
                if draw.air_mode {
                    self.visibility.priorities
                        == viewer_foundation::mode_visibility::PriorityTable::StrategicAir
                } else {
                    match draw.kind {
                        dashboard_core::protocol::PlanKind::TradeRoute => trade_routes,
                        kind => graphics.plan(kind),
                    }
                }
            });
            for (pass, mode) in self.order_label_pass.iter_mut().zip([
                ArrowMode::Main,
                ArrowMode::Seethrough,
                ArrowMode::Late,
            ]) {
                pass.upload(device, &self.order_labels, camera, size, 1.0, &offsets);
                pass.select_arrows(&orders.labels(mode));
            }
        }
        self.detail_laps.lap("orders_upload");
        self.sprite_pass.sync(device, queue);
        if let Some(radar) = &mut self.radar {
            radar.advance(frame_interval);
        }
        if let Some(ranges) = &mut self.air_ranges {
            ranges.advance(frame_interval);
        }
        if let Some(transfers) = &mut self.air_transfers {
            transfers.advance(frame_interval);
        }
        self.detail_laps.lap("sprite_sync");
        let sprite_cpu = viewer_runtime::profiling::thread_span("sprite_upload");
        let draw_cursor = std::cell::Cell::new(clock.map(|_| shader_pipeline::gpu::stats::draws()));
        let sprites = overlay.sprites.unwrap_or(&self.empty_batch);
        self.sprite_pass.upload(device, queue, arena, sprites, size, overlay.ui_scale);
        if let Some(previous) = draw_cursor.get() {
            let now = shader_pipeline::gpu::stats::draws();
            viewer_runtime::profiling::record_count("draws_gui_bake", now - previous);
            draw_cursor.set(Some(now));
        }
        sprite_cpu.finish();
        self.detail_laps.lap("sprite_upload");
        let prepared = clock.map(|clock| clock.elapsed());
        self.pass_cpu_laps.start(clock.map(|_| Instant::now()));
        let pass_cpu_laps = RefCell::new(&mut self.pass_cpu_laps);
        let lap_clock = std::cell::Cell::new(viewer_runtime::view_record::on().then(|| {
            (std::time::Instant::now(), viewer_runtime::profiling::current_thread_cpu_time())
        }));
        let cpu_mark = |label: &'static str| {
            pass_cpu_laps.borrow_mut().lap_sum(label);
            if let Some((wall, cpu)) = lap_clock.get() {
                let (now, now_cpu) = (std::time::Instant::now(), viewer_runtime::profiling::current_thread_cpu_time());
                viewer_runtime::view_record::add("render_wall_us", label, (now - wall).as_micros() as u64);
                if let (Some(cpu), Some(now_cpu)) = (cpu, now_cpu) {
                    viewer_runtime::view_record::add("render_cpu_us", label, now_cpu.saturating_sub(cpu).as_micros() as u64);
                }
                lap_clock.set(Some((now, now_cpu)));
            }
            if let Some(previous) = draw_cursor.get() {
                let now = shader_pipeline::gpu::stats::draws();
                let scope = match label {
                    "terrain_refraction" | "terrain" => "draws_terrain",
                    "shadow_mesh_casters" => "draws_shadow_meshes",
                    "shadow_tree_casters" | "shadow_tree_receivers" => "draws_shadow_trees",
                    "shadow_arrow_casters" => "draws_shadow_arrows",
                    "shadow_receivers" => "draws_shadow_ground",
                    "shadow_blur_horizontal" | "shadow_blur_vertical" => "draws_shadow_filter",
                    "borders_land" | "borders_sea" | "demilitarized" | "straits" => "draws_borders",
                    "rivers" => "draws_rivers",
                    "railways" => "draws_railways",
                    "trees" => "draws_trees",
                    "water" => "draws_water",
                    "unknown_snow_mud" => "draws_unknown_ground",
                    "orders_main" | "orders_seethrough" | "orders_late" => "draws_orders",
                    "radar_ranges" => "draws_radar",
                    "model_prepare" => "draws_model_setup_and_movement_arrows",
                    "model_encode" => "draws_models",
                    "animated_text" => "draws_world_text",
                    "weather" => "draws_weather",
                    "names" => "draws_country_names",
                    "sky" => "draws_sky",
                    "overlay_sprites" => "draws_gui_frame",
                    name if name.starts_with("post") => "draws_post",
                    _ => "draws_other",
                };
                viewer_runtime::profiling::record_count(scope, now - previous);
                draw_cursor.set(Some(now));
            }
        };
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map") });
        let mut command_buffers = Vec::new();
        if let Some(start) = self
            .frame_timer
            .as_mut()
            .and_then(|frame_timer| frame_timer.begin(device))
        {
            command_buffers.push(MapBuffer(start));
        }
        command_buffers.extend(
            self.sprite_pass
                .bake_buffers()
                .into_iter()
                .map(MapBuffer),
        );
        let timed = RefCell::new(timer);
        let mut mark = |encoder: &mut wgpu::CommandEncoder, label: &'static str| {
            cpu_mark(if matches!(label, "model_opaque" | "model_blended") {
                "model_recycle"
            } else {
                label
            });
            if let Some(timer) = timed.borrow_mut().as_mut() {
                timer.mark(encoder, label);
            }
        };

        mark(&mut encoder, "start");
        let political = {
            let _cost = replay_frame::cpu::scope("political.fill.encode", Some("political"));
            self.political
                .as_ref()
                .and_then(|political| political.encode(&mut encoder))
        };
        let stripes = self
            .province_colors
            .as_mut()
            .is_some_and(|colors| colors.encode(&mut encoder));
        mark(&mut encoder, "political_compute");
        {
            let null = self.terrain_null.sampled();
            let bound = [
                self.underwater.sampled(),
                self.height_normal.sampled(),
                self.colormap_water.sampled(),
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                self.point_light_maps.indices.sampled(),
                self.point_light_maps.data.sampled(),
                self.shadow.refraction_sampled(),
                null,
                null,
                null,
            ];
            let prepared = passes.run(passes::Pass::Underwater, || {
                #[cfg(any(test, feature = "test-support"))]
                faults.preflight(passes::Pass::Underwater)?;
                self.compiled_terrain.prepare_kept(
                    arena,
                    viewer_terrain::terrain_shader::Mode::Underwater,
                    viewer_terrain::terrain_shader::Inputs {
                        retained: None,
                        frames: &native_frame_bytes,
                        offsets: &offsets,
                        cover: &self.cover,
                        matrix: [[0.; 4]; 4],
                        light_grid: globals.light_grid,
                        textures: bound.map(|sampled| sampled.view),
                    },
                    &bound.map(|sampled| sampled.content),
                    Some(targets.refraction_content()),
                )
            });
            match prepared {
                Some(Some(prepared)) => {
                    let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("compiled underwater"),
                        color_attachments: &[Some(targets.refraction_attachment())],
                        depth_stencil_attachment: Some(targets.refraction_depth_attachment()),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    self.compiled_terrain.draw(&mut pass, &prepared);
                }
                Some(None) => {}
                None => self
                    .compiled_terrain
                    .lost(viewer_terrain::terrain_shader::Mode::Underwater),
            }
        }
        mark(&mut encoder, "terrain_refraction");
        let casting = shadow_casters && shadow_prepared.is_some() && visible.is_some();
        if !casting {
            self.shadow.ready_depth_map(&mut encoder, None);
        }
        match (shadow_prepared, visible) {
            (Some(shadow_prepared), Some(visible)) => {
                if shadow_casters {
                    let casters = Some(visible);
                    let caster_frame = |alpha| ModelFrame {
                        camera,
                        offsets: &offsets,
                        visible: casters,
                        seen: seen_ground,
                        alpha,
                    };
                    let caster_scenes: Vec<_> = native_frame_bytes
                        .iter()
                        .zip(&offsets)
                        .map(|(scene, offset)| {
                            let mut matrix = shadow_prepared.view_projection;
                            for (value, axis) in
                                matrix[3].iter_mut().zip(shadow_prepared.view_projection[0])
                            {
                                *value += axis * offset;
                            }
                            let mut bytes = scene.to_vec();
                            bytes[..64].copy_from_slice(bytemuck::bytes_of(&matrix));
                            bytes
                        })
                        .collect();
                    let caster_bytes: Vec<_> = caster_scenes.iter().map(Vec::as_slice).collect();
                    let native_caster_inputs = viewer_models::NativeInputs {
                        samples: 1,
                        frames: &caster_bytes,
                        ..native_mesh_inputs
                    };
                    let mut meshes: Vec<(&ModelPass, ModelFrame<'_>)> = Vec::new();
                    if let Some(frame) = &self.native_map_frame {
                        meshes.push((frame, ModelFrame { visible: None, seen: None, ..caster_frame(1.) }));
                    }
                    if let Some(sprawl) = &self.sprawl
                        && self.graphics.city_sprawl
                        && draws_city_sprawl(&camera)
                    {
                        meshes.push((sprawl, caster_frame(1.0)));
                    }
                    if let Some(buildings) = &self.map_buildings
                        && self.graphics.buildings
                    {
                        meshes.push((buildings, caster_frame(1.0)));
                    }
                    if let Some(units) = &self.unit_models
                        && self.graphics.unit_figures()
                    {
                        meshes.push((units, caster_frame(1.0)));
                    }
                    if let Some((aircraft, _, _)) = &self.aircraft
                        && self.graphics.aircraft()
                        && viewer_aircraft::aircraft::draws(&camera)
                    {
                        meshes.push((aircraft, caster_frame(1.0)));
                    }
                    if let Some((trains, _, _)) = &self.trains
                        && trains_visible
                    {
                        meshes.push((trains, caster_frame(1.0)));
                    }
                    if let Some(bridges) = &self.bridges
                        && bridges_visible
                    {
                        meshes.push((bridges, caster_frame(1.0)));
                    }
                    if let Some(player) = &self.player {
                        meshes.push((&player.pass, caster_frame(1.0)));
                    }
                    let trees = self.trees.as_ref().filter(|_| !no_trees);
                    let plans = PlanSubmissions::new(
                        self.orders.as_ref(),
                        &self.graphics,
                        overlay.interface,
                        self.visibility.trade_routes || self.visibility.priorities == viewer_foundation::mode_visibility::PriorityTable::StrategicAir,
                    );
                    let mut repeated = true;
                    let mut casters = Vec::with_capacity(meshes.len() + 1);
                    for &(pass, frame) in &meshes {
                        repeated &= pass.ready_casters(arena, frame, native_caster_inputs)?;
                        casters.push(pass.caster_id());
                    }
                    casters.extend(trees.map(TreePass::caster_id));
                    let arrows = plans.casts_shadows();
                    repeated &= !arrows;
                    let key = viewer_scene_base::shadow::CasterKey::new(
                        shadow_prepared.view_projection,
                        camera,
                        size,
                        &offsets,
                        casters,
                        arrows,
                    );
                    let draw = self.shadow.ready_depth_map(&mut encoder, Some((key, repeated)));
                    if draw
                        && let Some(mut pass) = self.shadow.depth_pass(&mut encoder, wgpu::LoadOp::Load)
                    {
                        for (mesh, _) in &meshes {
                            mesh.draw_casters(arena, &mut pass);
                        }
                    }
                    mark(&mut encoder, "shadow_mesh_casters");
                    if draw
                        && let Some(trees) = trees
                        && let Some(mut pass) = self.shadow.depth_pass(&mut encoder, wgpu::LoadOp::Load)
                    {
                        trees.draw_casters(
                            arena,
                            &mut pass,
                            shadow_prepared.view_projection,
                            TreeFrame {
                                camera,
                                offsets: &offsets,
                                planes: side_planes(camera, size),
                            },
                        )?;
                    }
                    mark(&mut encoder, "shadow_tree_casters");
                    if draw {
                        plans.cast_shadows(|plans| {
                            let Some(mut pass) =
                                self.shadow.depth_pass(&mut encoder, wgpu::LoadOp::Load)
                            else {
                                return Ok(());
                            };
                            for (view, offset) in offsets.iter().enumerate() {
                                let mut matrix = shadow_prepared.view_projection;
                                for (value, axis) in
                                    matrix[3].iter_mut().zip(shadow_prepared.view_projection[0])
                                {
                                    *value += axis * offset;
                                }
                                let scene = viewer_aircraft::arrow_shader::shadow_constants(
                                    native_frame_bytes[view],
                                    matrix,
                                );
                                plans.draw_shadow(
                                    arena,
                                    &mut pass,
                                    viewer_aircraft::order_pass::NativeInputs {
                                        retained: None,
                                        scene: &scene,
                                        grid: bytemuck::bytes_of(&globals.light_grid),
                                        textures: [
                                            self.shadow.lit_texture(),
                                            &self.terrain_ids.view,
                                            &self.heightmap.view,
                                            &self.atlas_normal.view,
                                            &self.height_normal.view,
                                            &self.lean1.view,
                                            &self.lean2.view,
                                            &self.point_light_maps.data.view,
                                            &self.point_light_maps.indices.view,
                                        ],
                                    },
                                    view,
                                    animation_time,
                                )?;
                            }
                            Ok(())
                        })?;
                    }
                    mark(&mut encoder, "shadow_arrow_casters");
                }
                if shadow_receivers {
                    let trees = self.trees.as_ref().filter(|_| !no_trees);
                    let tree_query = trees.and_then(|trees| targets.surface_motion.query(trees.receiver_motion(fog)));
                    if let Some(mut pass) = self
                        .shadow
                        .prepass_pass(&mut encoder, wgpu::LoadOp::Clear(1.0), tree_query)
                        && let Some(trees) = trees
                    {
                        trees.draw_unlit(
                            arena,
                            &mut pass,
                            TreeFrame {
                                camera,
                                offsets: &offsets,
                                planes: side_planes(camera, size),
                            },
                            viewer_terrain::trees::ReceiverInputs {
                                frames: &native_frame_bytes,
                                shadow_matrix: shadow_prepared.texture_matrix,
                                textures: [
                                    (2, &self.fow_noise.view),
                                    (3, &self.water_specular.view),
                                    (4, &self.intel_map.view),
                                    (6, &self.snow_mud.view),
                                    (
                                        8,
                                        self.shadow
                                            .caster_texture()
                                            .expect("created with receiver target"),
                                    ),
                                    (9, &self.reflection.view),
                                    (10, &self.point_light_maps.indices.view),
                                    (11, &self.point_light_maps.data.view),
                                    (12, &self.gradient1.view),
                                    (13, &self.gradient2.view),
                                    (15, &self.secondary.view),
                                ],
                            },
                        )?;
                    }
                    mark(&mut encoder, "shadow_tree_receivers");
                    let null = &self.terrain_null.view;
                    let terrain_receiver = self.compiled_terrain.prepare(
                        arena,
                        viewer_terrain::terrain_shader::Mode::Receiver,
                        viewer_terrain::terrain_shader::Inputs {
                            retained: None,
                            frames: &native_frame_bytes,
                            offsets: &offsets,
                            cover: &self.cover,
                            matrix: shadow_prepared.texture_matrix,
                            light_grid: globals.light_grid,
                            textures: [
                                &self.fow_noise.view,
                                &self.water_specular.view,
                                &self.intel_map.view,
                                null,
                                null,
                                null,
                                null,
                                &self.snow_mud.view,
                                null,
                                null,
                                null,
                                null,
                                self.shadow
                                    .caster_texture()
                                    .expect("created with receiver target"),
                                null,
                                null,
                                null,
                            ],
                        },
                    )?;
                    let receiver_query = targets.surface_motion.query(self.compiled_terrain.motion(viewer_terrain::terrain_shader::Mode::Receiver, fog, true));
                    if let Some(mut pass) =
                        self.shadow.prepass_pass(&mut encoder, wgpu::LoadOp::Load, receiver_query)
                    {
                        self.compiled_terrain.draw(&mut pass, &terrain_receiver);
                    }
                    mark(&mut encoder, "shadow_receivers");
                    self.shadow.finish_receivers();
                } else {
                    self.shadow.clear_stand_in(&mut encoder);
                }
                if shadow_casters {
                    self.shadow
                        .blur(arena, &mut encoder, camera.height, &mut mark)?;
                }
            }
            _ => self.shadow.clear_stand_in(&mut encoder),
        }
        mark(&mut encoder, "shadow");
        #[cfg(any(test, feature = "test-support"))]
        faults.after(failure_tests::Point::Shadow)?;
        drop(encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene clear"),
            color_attachments: &[Some(
                targets.scene_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
            )],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Clear(1.))),
            ..Default::default()
        }));
        let retained_scene = &self.retained_scene;
        let _native_frame = ShaderFrame::new(retained_scene);
        macro_rules! draw_world {
            ($pass:ident => $draw:expr) => {
                passes.world(passes::Pass::$pass, retained_scene, || {
                    #[cfg(any(test, feature = "test-support"))]
                    faults.preflight(passes::Pass::$pass)?;
                    $draw
                })
            };
        }
        let hatch_coverage = self
            .snow_presentation
            .coverage()
            .filter(|coverage| coverage.covered());
        let unknown_hatch = &self.unknown_hatch;
        let hatch_heights = &self.hatch_heights.as_ref().unwrap_or(&self.heightmap).view;
        let (snow_unknown, hatch_grid) = (&self.snow_unknown.view, self.snow_presentation.size);
        let terrain_map = self.compiled_terrain.map();
        if let Some((_, Ok(hatch))) = unknown_hatch {
            hatch.begin_frame();
        }
        let draw_hatch = |encoder: &mut wgpu::CommandEncoder,
                          view: usize,
                          surface: viewer_terrain::weather::Surface,
                          standing: &[(wgpu::Buffer, Vec<viewer_terrain::terrain_shader::Stand>)]|
         -> io::Result<()> {
            let Some(coverage) = hatch_coverage else {
                return Ok(());
            };
            let hatch = match unknown_hatch {
                Some((_, Ok(hatch))) => hatch,
                Some((_, Err(reason))) => return Err(io::Error::other(reason.clone())),
                None => {
                    return Err(io::Error::other(
                        "unknown snow and mud hatch pipeline is not built",
                    ));
                }
            };
            let streams: Vec<_> = standing
                .iter()
                .map(|(buffer, stands)| (buffer.clone(), coverage.instances(stands)))
                .collect();
            let frame = &native_frames[view];
            let [x, y, z, _] = frame.cam_look_at_dir_global_time;
            hatch.draw(
                device,
                queue,
                encoder,
                targets,
                hatch_heights,
                snow_unknown,
                &streams,
                &viewer_terrain::weather::HatchParams::new(
                    frame.view_projection,
                    [x, y, z],
                    terrain_map,
                    hatch_grid,
                    surface,
                ),
            )
        };
        self.compiled_terrain.keep_views(&offsets);
        let coarse_lines = viewer_scene_base::line_lod::coarse(&camera, targets.world_size());
        for view in 0..offsets.len() {
            viewer_scene_base::shader_constants::WorldPixelConstants::LightGrid(globals.light_grid)
                .retain(&mut retained_scene.borrow_mut());
            let view_frames = &native_frame_bytes[view..view + 1];
            let view_offsets = &offsets[view..view + 1];
            let mode = viewer_terrain::terrain_shader::Mode::Lit(self.graphics.high_gfx_shaders);
            let prepared = draw_world!(Terrain => self.compiled_terrain.prepare(
                    arena,
                    mode,
                    viewer_terrain::terrain_shader::Inputs {
                        retained: Some(retained_scene),
                        frames: view_frames,
                        offsets: view_offsets,
                        cover: &self.cover,
                        matrix: [[0.; 4]; 4],
                        light_grid: globals.light_grid,
                        textures: [
                            &self.atlas.view,
                            &self.height_normal.view,
                            &self.colormap.view,
                            &self.snow.view,
                            &self.atlas_normal.view,
                            &self.terrain_ids.view,
                            &self.secondary.view,
                            &self.snow_mud.view,
                            &self.city_lights.view,
                            &self.mud_normal.view,
                            &self.point_light_maps.indices.view,
                            &self.point_light_maps.data.view,
                            self.shadow.lit_texture(),
                            &self.mud_diffuse.view,
                            &self.gradient1.view,
                            &self.gradient2.view,
                        ],
                    },
            ));
            if let Some(prepared) = prepared {
                let animated = self.compiled_terrain.motion(mode, fog, pulses);
                let mut pass = viewer_scene_base::shader_scene::surface_pass(&mut encoder, targets, "map world", animated);
                self.compiled_terrain.draw(&mut pass, &prepared);
            }
            mark(&mut encoder, "terrain");
            if hatch_coverage.is_some() {
                draw_world!(UnknownSnowMud => {
                    let standing =
                        self.compiled_terrain.standing(device, &self.cover, offsets[view])?;
                    draw_hatch(
                        &mut encoder,
                        view,
                        viewer_terrain::weather::Surface::Ground,
                        &standing,
                    )
                });
            }
            let border_inputs = || viewer_borders::border_shader::Inputs {
                fog,
                retained: Some(retained_scene),
                frames: view_frames,
                snow_mud: &self.snow_mud.view,
                intel: &self.intel_map.view,
                shadow: self.shadow.lit_texture(),
                time: map_time,
            };
            let mut pixel_b2 =
                viewer_scene_base::shader_constants::WorldPixelConstants::LightGrid(globals.light_grid);
            if let Some(borders) = &self.borders
                && let Some(value) = draw_world!(LandBorders => borders.draw(
                    arena,
                    &mut encoder,
                    targets,
                    border_inputs(),
                    BorderFrame {
                        stage: BorderStage::Land,
                        camera_height: camera.height,
                        coarse: coarse_lines,
                        fades: camera.defines.border_fades,
                        province_borders: self.visibility.province_borders,
                        visible: overlay.interface && self.graphics.land_borders(),
                    },
                ))
            {
                pixel_b2 = value;
            }
            mark(&mut encoder, "borders_land");
            if let Some(rivers) = &self.rivers
                && self.graphics.rivers
            {
                let [diffuse, normal, masks] = rivers.textures();
                draw_world!(Rivers => rivers.draw(
                    arena,
                    &mut encoder,
                    targets,
                    viewer_terrain::river_shader::Inputs {
                        fog,
                        retained: Some(retained_scene),
                        frames: view_frames,
                        shadow_matrix: &globals.shadow_matrix,
                        time: map_time,
                        retained_pixel_b2: pixel_b2,
                        textures: [
                            &self.colormap_water.view,
                            diffuse,
                            normal,
                            masks,
                            &self.lean1.view,
                            &self.lean2.view,
                            &self.secondary.view,
                            &self.height_normal.view,
                            &self.snow_mud.view,
                            &self.city_lights.view,
                            &self.point_light_maps.indices.view,
                            &self.point_light_maps.data.view,
                            &self.gradient1.view,
                            &self.gradient2.view,
                            &self.reflection.view,
                            self.shadow.lit_texture(),
                        ],
                    },
                    self.graphics.high_gfx_shaders,
                    coarse_lines,
                ));
            }
            mark(&mut encoder, "rivers");
            if let Some(railways) = &self.railways
                && self.graphics.rails()
            {
                let [rail, null] = railways.textures();
                draw_world!(Railways => railways.draw(
                    arena,
                    &mut encoder,
                    targets,
                    viewer_rail::rail_shader::Inputs {
                        retained: Some(retained_scene),
                        frames: view_frames,
                        shadow_matrix: &globals.shadow_matrix,
                        time: map_time,
                        alpha: rail_alpha,
                        retained_pixel_b2: pixel_b2,
                        textures: [
                            rail,
                            null,
                            null,
                            null,
                            &self.lean1.view,
                            &self.lean2.view,
                            &self.secondary.view,
                            &self.height_normal.view,
                            &self.snow_mud.view,
                            &self.city_lights.view,
                            &self.point_light_maps.indices.view,
                            &self.point_light_maps.data.view,
                            &self.gradient1.view,
                            &self.gradient2.view,
                            &self.reflection.view,
                            self.shadow.lit_texture(),
                        ],
                    },
                ));
            }
            mark(&mut encoder, "railways");
            if let Some(trees) = &self.trees
                && !no_trees
                && passes.ready(passes::Pass::Trees)
            {
                trees.draw(
                    arena,
                    &mut encoder,
                    targets,
                    TreeFrame {
                        camera,
                        offsets: &offsets,
                        planes: side_planes(camera, size),
                    },
                    viewer_terrain::trees::LitInputs {
                        fog,
                        pulses,
                        retained: Some(retained_scene),
                        view,
                        frames: &native_frame_bytes,
                        retained_pixel_b2: pixel_b2,
                        textures: [
                            (4, &self.colormap.view),
                            (6, &self.snow_mud.view),
                            (8, self.shadow.lit_texture()),
                            (9, &self.reflection.view),
                            (10, &self.point_light_maps.indices.view),
                            (11, &self.point_light_maps.data.view),
                            (12, &self.gradient1.view),
                            (13, &self.gradient2.view),
                            (15, &self.secondary.view),
                        ],
                        high: self.graphics.high_gfx_shaders,
                    },
                )?;
                #[cfg(any(test, feature = "test-support"))]
                faults.after(failure_tests::Point::Trees)?;
            }
            mark(&mut encoder, "trees");
            if !no_water && self.graphics.water {
                draw_world!(Water => self.water.draw(
                    arena,
                    &mut encoder,
                    targets,
                    viewer_terrain::water_shader::Inputs {
                        fog,
                        animated_sea_bed: self.compiled_terrain.motion(viewer_terrain::terrain_shader::Mode::Underwater, fog, true) != viewer_gpu::surface_motion::Motion::Still,
                        retained: Some(retained_scene),
                        frames: view_frames,
                        time: map_time,
                        size,
                        retained_pixel_b2: pixel_b2,
                        textures: [
                            &self.heightmap.view,
                            &self.lean1.view,
                            &self.lean2.view,
                            &self.secondary.view,
                            &self.water_specular.view,
                            &targets.refraction_view,
                            &self.ice_diffuse.view,
                            &self.ice_noise.view,
                            &self.reflection.view,
                            &self.snow_mud.view,
                            &self.point_light_maps.indices.view,
                            &self.point_light_maps.data.view,
                            &self.gradient1.view,
                            &self.gradient2.view,
                            &self.naval_dominance.view,
                            self.shadow.lit_texture(),
                        ],
                    },
                    self.graphics.high_gfx_shaders,
                    self.graphics.water_reflections,
                ));
            }
            mark(&mut encoder, "water");
            if hatch_coverage.is_some()
                && !no_water
                && self.graphics.water
                && self.water.shows_snow_mud()
            {
                draw_world!(UnknownSnowMud => {
                    let standing =
                        self.compiled_terrain.standing(device, &self.cover, offsets[view])?;
                    draw_hatch(
                        &mut encoder,
                        view,
                        viewer_terrain::weather::Surface::Water,
                        &standing,
                    )
                });
            }
            mark(&mut encoder, "unknown_snow_mud");
            if let Some(borders) = &self.borders {
                draw_world!(SeaBorders => borders.draw(
                    arena,
                    &mut encoder,
                    targets,
                    border_inputs(),
                    BorderFrame {
                        stage: BorderStage::Sea,
                        camera_height: camera.height,
                        coarse: coarse_lines,
                        fades: camera.defines.border_fades,
                        province_borders: self.visibility.province_borders,
                        visible: overlay.interface && self.graphics.sea_borders(),
                    },
                ));
            }
            mark(&mut encoder, "borders_sea");
            if let Some(demilitarized) = &self.demilitarized {
                let edges = if !overlay.interface {
                    viewer_borders::demilitarized::ZoneEdges::Hidden
                } else if self.graphics.demilitarized_zones() {
                    viewer_borders::demilitarized::ZoneEdges::Highlighted
                } else if self.graphics.land_borders() {
                    viewer_borders::demilitarized::ZoneEdges::Ordinary {
                        camera_height: camera.height,
                        fades: camera.defines.border_fades,
                        province_borders: self.visibility.province_borders,
                    }
                } else {
                    viewer_borders::demilitarized::ZoneEdges::Hidden
                };
                draw_world!(Demilitarized => demilitarized.draw(
                    arena,
                    &mut encoder,
                    targets,
                    border_inputs(),
                    edges,
                    coarse_lines,
                ));
            }
            mark(&mut encoder, "demilitarized");
            if let Some(borders) = &self.borders {
                draw_world!(Straits => borders.compiled_straits.draw(
                    arena,
                    &mut encoder,
                    targets,
                    view_frames,
                    [&self.snow_mud.view, &self.intel_map.view],
                    Some(retained_scene),
                    overlay.interface && self.graphics.strait_crossings(),
                ));
            }
            mark(&mut encoder, "straits");
        }
        let draw_orders = |encoder: &mut wgpu::CommandEncoder,
                           mode: ArrowMode,
                           label_index: usize|
         -> io::Result<()> {
            PlanSubmissions::new(
                self.orders.as_ref(),
                &self.graphics,
                overlay.interface,
                self.visibility.trade_routes
                    || self.visibility.priorities
                        == viewer_foundation::mode_visibility::PriorityTable::StrategicAir,
            )
            .draw(|plans| {
                for view in 0..offsets.len() {
                    plans.draw(
                        arena,
                        encoder,
                        targets,
                        viewer_aircraft::order_pass::NativeInputs {
                            retained: Some(retained_scene),
                            scene: native_frame_bytes[view],
                            grid: bytemuck::bytes_of(&globals.light_grid),
                            textures: [
                                self.shadow.lit_texture(),
                                &self.terrain_ids.view,
                                &self.heightmap.view,
                                &self.atlas_normal.view,
                                &self.height_normal.view,
                                &self.lean1.view,
                                &self.lean2.view,
                                &self.point_light_maps.data.view,
                                &self.point_light_maps.indices.view,
                            ],
                        },
                        mode,
                        view,
                        animation_time,
                    )?;
                    if !self.graphics.plan_labels() {
                        continue;
                    }
                    self.order_label_pass[label_index].draw_view(
                        arena,
                        &native_frame_bytes,
                        encoder,
                        targets,
                        Some(view),
                        Some(retained_scene),
                    )?;
                }
                Ok(())
            })
        };
        draw_orders(&mut encoder, ArrowMode::Main, 0)?;
        #[cfg(any(test, feature = "test-support"))]
        faults.after(failure_tests::Point::Arrows)?;
        mark(&mut encoder, "orders_main");
        let air = self.visibility.priorities
            == viewer_foundation::mode_visibility::PriorityTable::StrategicAir;
        for (radar, admitted) in [
            (self.radar.as_ref(), true),
            (self.air_ranges.as_ref(), air),
            (self.air_transfers.as_ref(), air),
        ]
        .into_iter()
        .filter_map(|(pass, admitted)| pass.map(|pass| (pass, admitted)))
        {
            for view in 0..offsets.len() {
                let mut wrapped = camera;
                wrapped.center[0] -= offsets[view];
                radar.draw(
                    arena,
                    &mut encoder,
                    targets,
                    viewer_aircraft::order_pass::NativeInputs {
                        retained: Some(retained_scene),
                        scene: native_frame_bytes[view],
                        grid: bytemuck::bytes_of(&globals.light_grid),
                        textures: [
                            self.shadow.lit_texture(),
                            &self.terrain_ids.view,
                            &self.heightmap.view,
                            &self.atlas_normal.view,
                            &self.height_normal.view,
                            &self.lean1.view,
                            &self.lean2.view,
                            &self.point_light_maps.data.view,
                            &self.point_light_maps.indices.view,
                        ],
                    },
                    &side_planes(wrapped, size),
                    animation_time,
                    overlay.interface && admitted,
                )?;
            }
        }
        cpu_mark("radar_ranges");
        let actor_camera = actor_views[0];
        let mut admitted_actor = candidates;
        admitted_actor.center[0] += actor_camera.center[0] - camera.center[0];
        let actor_visible = admitted_actor.held_ground(size, MARGIN, slack);
        let actor_seen = actor_camera.visible_ground(size, MARGIN);
        let actor_offsets: Vec<_> = actor_views
            .iter()
            .map(|view| actor_camera.center[0] - view.center[0])
            .collect();
        let actor_frames = actor_views
            .iter()
            .map(|&view| {
                FrameConstants::try_new(
                    view,
                    size,
                    day_night,
                    animation_time,
                    self.fog && self.visibility.fog_of_war,
                )
                .map(|frame| {
                    frame
                        .with_snow_mud_fade(snow_mud_fade)
                        .with_channels(self.mode_channels)
                })
                .ok_or_else(|| {
                    invalid("Map shader scene: 1 refused actor frame: camera has no finite inverse")
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let actor_frame_bytes: Vec<_> = actor_frames.iter().map(bytemuck::bytes_of).collect();
        if let Some(player) = &mut self.player {
            player::copies(&actor_camera, size, player.sphere, &mut player.offsets);
            player.frames.clear();
            for offset in &player.offsets {
                let mut view = actor_camera;
                view.center[0] -= offset;
                player.frames.push(
                    FrameConstants::try_new(view, size, day_night, animation_time, self.fog)
                        .map(|frame| frame.with_snow_mud_fade(snow_mud_fade))
                        .ok_or_else(|| {
                            invalid(
                                "Map shader scene: 1 refused player frame: camera has no finite inverse",
                            )
                        })?,
                );
            }
        }
        cpu_mark("actor_envelopes");
        let particle_packets = self
            .weather_effects
            .as_mut()
            .map(|weather| weather.finish_frame(camera));
        if let Some(packets) = &particle_packets {
            viewer_runtime::view_record::add("weather packets", "packets", packets.count() as u64);
        }
        cpu_mark("weather_packets");
        let leading = std::mem::replace(
            &mut encoder,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map") }),
        );
        self.encode_lane.start(move || leading.finish());
        let [
            map_frame,
            sprawl,
            buildings,
            units,
            bridges,
            aircraft,
            trains,
            _,
        ] = model_passes!(self);
        let mesh_layers = [
            (map_frame, true, 1., "map_frame_mesh"),
            (
                sprawl,
                self.graphics.city_sprawl && draws_city_sprawl(&camera),
                1.,
                "sprawl",
            ),
            (buildings, self.graphics.buildings, 1., "buildings"),
            (units, self.graphics.unit_figures(), 1., "units"),
            (bridges, bridges_visible, 1., "bridges"),
            (
                aircraft,
                self.graphics.aircraft() && viewer_aircraft::aircraft::draws(&camera),
                1.,
                "aircraft",
            ),
            (trains, trains_visible, rail_alpha, "trains"),
        ];
        for (model, _, _, _) in mesh_layers {
            if let Some(model) = model {
                model.reset_drawn();
            }
        }
        let player = self.player.as_ref();
        if let Some(player) = player {
            player.pass.reset_drawn();
        }
        let mut object = viewer_models::NativeObjectConstants::default();
        object.retained.inherit(&retained_scene.borrow());
        for native_queue in [
            viewer_models::NativeQueue::Regular,
            viewer_models::NativeQueue::Alternate,
        ] {
            for view in 0..actor_views.len() {
                let wrapped = actor_views[view];
                if native_queue == viewer_models::NativeQueue::Regular
                    && self.movement_arrows.draw(
                        arena,
                        &mut encoder,
                        targets,
                        &mut object.retained,
                        &actor_frames[view].view_projection,
                        &side_planes(wrapped, size),
                        overlay.interface && self.graphics.movement_arrows,
                    )
                {
                    object.forget_pixel_snapshot();
                }
                object.begin();
                let model_draws = (|| -> io::Result<_> {
                    let mut model_draws = Vec::new();
                    let prepared = (|| -> io::Result<()> {
                        for (model, enabled, alpha, label) in mesh_layers {
                            if let Some(model) = model.filter(|_| enabled) {
                                let frame = ModelFrame {
                                    camera: actor_camera,
                                    offsets: &actor_offsets[view..view + 1],
                                    visible: if label == "map_frame_mesh" {
                                        None
                                    } else {
                                        actor_visible
                                    },
                                    seen: if label == "map_frame_mesh" {
                                        None
                                    } else {
                                        actor_seen
                                    },
                                    alpha,
                                };
                                let draws = model.prepare_kept(
                                    arena,
                                    frame,
                                    viewer_models::NativeInputs {
                                        frames: &actor_frame_bytes[view..view + 1],
                                        ..native_mesh_inputs
                                    },
                                    native_queue,
                                    &mut object,
                                )?;
                                model_draws.push((model, draws));
                                #[cfg(any(test, feature = "test-support"))]
                                faults.after(failure_tests::Point::Models)?;
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = prepared {
                        for (model, draws) in model_draws {
                            model.recycle_kept(draws);
                        }
                        return Err(error);
                    }
                    cpu_mark("model_prepare");
                    ModelPass::encode_native(&model_draws, targets, &mut encoder);
                    cpu_mark("model_encode");
                    Ok(model_draws)
                })()?;
                for (model, draws) in model_draws {
                    model.recycle_kept(draws);
                }
                mark(
                    &mut encoder,
                    if native_queue == viewer_models::NativeQueue::Regular {
                        "model_opaque"
                    } else {
                        "model_blended"
                    },
                );
                if native_queue == viewer_models::NativeQueue::Alternate {
                    let graphics = self.graphics;
                    if let Some(text) = &mut self.animated_text
                        && text.draw(
                            arena,
                            &mut encoder,
                            targets,
                            &mut object.retained,
                            wrapped,
                            &actor_frames[view].view_projection,
                            None,
                            &|family| {
                                overlay.interface
                                    && map_text_kind(family)
                                        .is_some_and(|kind| graphics.map_text(kind))
                            },
                            overlay.ground.as_ref(),
                        )
                    {
                        object.forget_pixel_snapshot();
                    }
                    mark(&mut encoder, "animated_text");
                    if let (Some(weather), Some(packets)) =
                        (&mut self.weather_effects, &particle_packets)
                    {
                        if let Err(error) = weather.draw_view(
                            arena,
                            &mut encoder,
                            targets,
                            wrapped,
                            view,
                            viewer_weather_effects::weather_effects::NativeInputs {
                                frame: actor_frame_bytes[view],
                                grid: native_mesh_inputs.grid,
                                textures: native_mesh_inputs.textures,
                                retained: &mut object.retained,
                            },
                            packets,
                        ) {
                            return Err(error);
                        }
                        object.forget_pixel_snapshot();
                        #[cfg(any(test, feature = "test-support"))]
                        faults.after(failure_tests::Point::Weather)?;
                    }
                    mark(&mut encoder, "weather");
                }
            }
            if let Some(player) = player {
                object.begin();
                for (offset, frame) in player.offsets.iter().zip(&player.frames) {
                    player.pass.draw_native(
                        arena,
                        &mut encoder,
                        targets,
                        ModelFrame {
                            camera: actor_camera,
                            offsets: std::slice::from_ref(offset),
                            visible: None,
                            seen: None,
                            alpha: 1.0,
                        },
                        viewer_models::NativeInputs {
                            frames: &[bytemuck::bytes_of(frame)],
                            ..native_mesh_inputs
                        },
                        native_queue,
                        &mut object,
                    )?;
                }
                mark(&mut encoder, "player");
            }
        }
        let models = std::mem::replace(
            &mut encoder,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map") }),
        );
        self.models_lane.start(move || models.finish());
        draw_orders(&mut encoder, ArrowMode::Seethrough, 1)?;
        mark(&mut encoder, "orders_seethrough");
        draw_orders(&mut encoder, ArrowMode::Late, 2)?;
        mark(&mut encoder, "orders_late");
        self.name_pass.validate_draw(&native_frame_bytes, None)?;
        self.near_name_pass
            .validate_draw(&native_frame_bytes, None)?;
        let names = overlay.country_names && self.graphics.country_names;
        if self.graphics.sky && self.sky.is_none() {
            self.sky = Some(sky_pipeline(
                device,
                &self.sky_effect,
                self.graphics.samples,
            ));
        }
        passes.run(passes::Pass::CountryNames, || {
            if !names {
                return Ok(());
            }
            #[cfg(any(test, feature = "test-support"))]
            faults.preflight(passes::Pass::CountryNames)?;
            self.name_pass
                .draw(arena, &native_frame_bytes, &mut encoder, targets)?;
            self.near_name_pass
                .draw(arena, &native_frame_bytes, &mut encoder, targets)
        });
        mark(&mut encoder, "names");
        if self.graphics.sky {
            passes.run(passes::Pass::Sky, || {
                    #[cfg(any(test, feature = "test-support"))]
                    faults.preflight(passes::Pass::Sky)?;
                    viewer_gpu::gpu_contract::compiled(
                        self.sky.as_ref().expect("created above"),
                    )?
                .draw(
                    arena,
                    &mut encoder,
                    targets,
                    bytemuck::bytes_of(&globals.frame),
                    &self.reflection.view,
                )
            });
        }
        mark(&mut encoder, "sky");
        targets.resolve_scene(&mut encoder);
        mark(&mut encoder, "scene_resolve");
        if postfx {
            let delta = self
                .last_animation_time
                .filter(|previous| animation_time > *previous)
                .map(|previous| animation_time - previous);
            let tau_delta = self.post.adaptation_tau(delta);
            let grading = self.post.grading(camera, day_night);
            let post = self
                .post
                .draw(arena, &mut encoder, targets, grading, tau_delta, &mut mark);
            #[cfg(any(test, feature = "test-support"))]
            let post = post.and_then(|()| faults.after(failure_tests::Point::Post));
            if post.is_err() {
                self.post.invalidate();
            }
            post?;
        } else {
            self.post.copy_scene(&mut encoder, targets)?;
        }
        mark(&mut encoder, "post");
        self.last_animation_time = Some(animation_time);
        self.last_transition_time = Some(transition_clock);
        self.sprite_pass.draw(&mut encoder, targets);
        #[cfg(any(test, feature = "test-support"))]
        faults.draw_sentinel(device, &mut encoder, targets);
        targets.surface_motion.finish(&mut encoder);
        mark(&mut encoder, "overlay_sprites");
        if self.interaction_hour.is_some()
            && !targets.surface_motion.live()
            && (particle_packets
                .as_ref()
                .is_some_and(|packets| packets.moving)
                || self
                    .strikes
                    .visible_effect(camera, size, |effect| shows_impact(&self.graphics, effect))
                || player.is_some_and(|player| {
                    player
                        .pass
                        .shows_animation(&actor_camera, None, &actor_offsets)
                })
                || mesh_layers.iter().any(|(pass, enabled, _, layer)| {
                    *enabled
                        && pass.is_some_and(|pass| {
                            if matches!(*layer, "trains" | "aircraft") {
                                pass.drawn() != 0
                            } else {
                                pass.shows_animation(&actor_camera, seen_ground, &pose_offsets)
                            }
                        })
                }))
        {
            arena.request_frame();
        }
        if let Some(timer) = timed.borrow_mut().as_mut() {
            timer.finish(&mut encoder);
        }
        cpu_mark("timestamp_resolve");
        if let Some(frame_timer) = &self.frame_timer {
            frame_timer.end(&mut encoder);
        }
        if let Some(leading) = self.encode_lane.take() {
            command_buffers.push(MapBuffer(leading));
        }
        self.encode_lane.start(move || encoder.finish());
        let mut uploads =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map uploads") });
        self.arena.flush_into(&mut uploads);
        command_buffers.insert(0, MapBuffer(uploads.finish()));
        cpu_mark("arena_flush");
        let encoded = clock.map(|clock| clock.elapsed());
        if let Some(models) = self.models_lane.take() {
            command_buffers.push(MapBuffer(models));
        }
        let trailing = self
            .encode_lane
            .take()
            .expect("the trailing buffer was started");
        cpu_mark("encoder_finish");
        command_buffers.push(MapBuffer(trailing));
        cpu_mark("encoder_join");
        queue.submit(command_buffers.into_iter().map(|MapBuffer(buffer)| buffer).collect::<Vec<_>>());
        if let Some(frame_timer) = &mut self.frame_timer {
            frame_timer.submitted();
        }
        self.targets
            .as_mut()
            .expect("rendered above")
            .surface_motion
            .submitted();
        cpu_mark("queue_submit");
        self.compiled_terrain.submitted();
        if let Some(political) = political {
            political.submitted();
        }
        if stripes && let Some(colors) = &mut self.province_colors {
            colors.submitted();
        }
        #[cfg(any(test, feature = "test-support"))]
        faults.submissions.set(faults.submissions.get() + 1);
        if let Some(timer) = timed.borrow_mut().as_mut() {
            timer.report(device);
        }
        cpu_mark("submit_report");
        if let (Some(clock), Some(animated), Some(prepared), Some(encoded)) =
            (clock, animated, prepared, encoded)
        {
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            self.render_laps = [
                ms(animated),
                ms(prepared - animated),
                ms(encoded - prepared),
                ms(clock.elapsed() - encoded),
            ];
        }
        Ok(())
    }

    pub fn render_laps(&self) -> [f64; 4] {
        self.render_laps
    }

    pub fn time_frame(&mut self, frame: u64) {
        if let Some(frame_timer) = &mut self.frame_timer {
            frame_timer.label(frame);
        }
    }

    pub fn retire_gpu_frame_times(
        &mut self,
    ) -> Vec<viewer_gpu::gpu_frame_time::FrameTime> {
        self.frame_timer
            .as_mut()
            .map_or_else(Vec::new, |frame_timer| frame_timer.retire())
    }

    pub fn gpu_frame_times(
        &mut self,
        device: &wgpu::Device,
    ) -> Vec<viewer_gpu::gpu_frame_time::FrameTime> {
        self.frame_timer
            .as_mut()
            .map_or_else(Vec::new, |frame_timer| frame_timer.take(device))
    }

    pub fn label_frame(&mut self, frame: u64) {
        if let Some(timer) = &mut self.pass_timer {
            timer.label(frame);
        }
    }

    pub fn detail_laps(&self) -> String {
        format!(
            "{} {}",
            self.detail_laps.line(),
            self.pass_cpu_laps.line_with_prefix("pass_")
        )
    }

    pub fn grading(&self) -> Option<String> {
        self.post.last_grading().map(|grading| grading.describe())
    }

    pub fn shadows(&self) -> bool {
        self.shadow.drawn()
    }

    pub fn shadow_screen(&self) -> Option<viewer_scene_base::shadow::Screen> {
        self.shadow.screen()
    }

    pub fn terrain_drawn(&self) -> (usize, u64) {
        (self.cover.blocks(), self.cover.quads())
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn targets(&self) -> Option<&Targets> {
        self.targets.as_ref()
    }

    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(Targets::output_view)
    }

    pub fn release_viewport(&mut self) {
        self.targets = None;
        self.shadow.release();
        self.post.release_chain();
    }

    pub fn overlay_counts(&self) -> (u32, u32) {
        (
            self.name_pass.vertex_count() + self.near_name_pass.vertex_count(),
            self.sprite_pass.quad_count(),
        )
    }

    fn shadow_screen_wanted(&self, device: &wgpu::Device) -> viewer_scene_base::shadow::Screen {
        let high = self.graphics.high_gfx_shaders;
        if self.graphics.water {
            self.water.warm(device, high, self.graphics.water_reflections);
        }
        if let Some(rivers) = self.rivers.as_ref().filter(|_| self.graphics.rivers) {
            rivers.warm(device, high);
        }
        if let Some(railways) = self.railways.as_ref().filter(|_| self.graphics.rails()) {
            railways.warm(device);
        }
        if let Some(trees) = self.trees.as_ref().filter(|_| self.graphics.trees) {
            trees.warm(device, high);
        }
        let mut full = false;
        self.shadow_texel_reads(&mut |_, _| full = true);
        if full {
            viewer_scene_base::shadow::Screen::Full
        } else {
            viewer_scene_base::shadow::Screen::Half
        }
    }

    fn shadow_texel_reads(&self, visit: &mut dyn FnMut(&str, &str)) {
        let high = self.graphics.high_gfx_shaders;
        if self.graphics.shadows {
            for reads in self.shadow.texel_reads() {
                visit("shadow blur", reads);
            }
        }
        for mode in viewer_terrain::terrain_shader::terrain_readers(high) {
            if let Some(reads) = self.compiled_terrain.texel_reads(mode) {
                visit("terrain", reads);
            }
        }
        if let Some(reads) = self.borders.as_ref().and_then(BorderPass::texel_reads) {
            visit("borders", reads);
        }
        if let Some(reads) = self.demilitarized.as_ref().and_then(DemilitarizedPass::texel_reads) {
            visit("demilitarized zone borders", reads);
        }
        if let Some(rivers) = self.rivers.as_ref().filter(|_| self.graphics.rivers) {
            rivers.texel_reads(high, &mut |reads| visit("rivers", reads));
        }
        if let Some(railways) = self.railways.as_ref().filter(|_| self.graphics.rails()) {
            railways.texel_reads(&mut |reads| visit("railways", reads));
        }
        if let Some(trees) = self.trees.as_ref().filter(|_| self.graphics.trees) {
            trees.texel_reads(high, &mut |reads| visit("tree canopy", reads));
        }
        if self.graphics.water {
            self.water.texel_reads(&mut |reads| visit("water", reads));
        }
        for reads in self.orders.iter().flat_map(OrderPass::texel_reads) {
            visit("map arrows", reads);
        }
        for (shader, pass) in [
            ("radar symbols", &self.radar),
            ("air ranges", &self.air_ranges),
            ("air transfer progress", &self.air_transfers),
        ] {
            if let Some(reads) = pass.as_ref().and_then(|pass| pass.texel_reads()) {
                visit(shader, reads);
            }
        }
    }

    pub fn pass_warnings(&self) -> Vec<String> {
        let mut warnings: Vec<_> = passes::lines(&self.pass_diagnostics).collect();
        warnings.extend(self.compiled_terrain.not_kept().map(|(technique, reason)| {
            format!("Map shader {technique}: drawn every frame: {reason}")
        }));
        if self.targets.as_ref().is_some_and(|targets| {
            targets.sea_bed() == viewer_gpu::gpu_contract::SeaBed::Full
        }) && let Some(reason) = self.water.sea_bed_refusal()
        {
            warnings.push(format!("Map shader water: {reason}"));
        }
        if self.shadow_screen() == Some(viewer_scene_base::shadow::Screen::Full) {
            let mut named = BTreeSet::new();
            self.shadow_texel_reads(&mut |shader, reason| {
                named.insert(format!("Map shader {shader}: {reason}"));
            });
            warnings.extend(named);
        }
        if let Some(reason) = self.compiled_terrain.coast_refusal() {
            warnings.push(format!(
                "Map coasts: 1 refused coarse-terrain shore cap, so zoomed-out coasts can show the \
                 sea floor: {reason}"
            ));
        }
        if let Some((_, assets, _)) = &self.aircraft {
            warnings.extend(
                assets
                    .lazy_diagnostics()
                    .into_iter()
                    .map(|(_, reason)| reason),
            );
        }
        warnings.extend(self.movement_arrows.warnings());
        if let Some(radar) = &self.radar {
            warnings.extend(radar.warnings());
        }
        if let Some(reason) = &self.radar_refusal {
            warnings.push(format!("Radar symbols: 1 refused pass: {reason}"));
        }
        if let Some(ranges) = &self.air_ranges {
            warnings.extend(ranges.warnings());
        }
        if let Some(reason) = &self.air_ranges_refusal {
            warnings.push(format!("Air ranges: 1 refused pass: {reason}"));
        }
        if let Some(transfers) = &self.air_transfers {
            warnings.extend(transfers.warnings());
        }
        if let Some(reason) = &self.air_transfers_refusal {
            warnings.push(format!("Air transfer progress: 1 refused pass: {reason}"));
        }
        if let Some(text) = &self.animated_text {
            warnings.extend(text.warnings());
        }
        warnings.extend(
            self.map_text_refusals
                .iter()
                .map(|(reason, count)| format!("{reason} ({count})")),
        );
        warnings.extend(self.unit_history.combat_warnings());
        for model in model_passes!(self).into_iter().flatten() {
            warnings.extend(model.refusal_lines(self.arena.frame()));
        }
        warnings
    }

    pub fn track_paused_motion(&mut self, hour: Option<i32>) {
        self.interaction_hour = hour;
        self.arena.track_motion(hour.is_some());
    }

    pub fn needs_frame(&self) -> bool {
        self.arena.needs_frame()
            || self.unit_history.carrying_figures()
            || (self.graphics.unit_figures() && self.unit_history.scaling_figures())
            || (self.graphics.buildings && self.building_history.growing())
            || self
                .targets
                .as_ref()
                .is_some_and(|targets| targets.surface_motion.live())
    }

    pub fn next_arrival(
        &self,
        camera: &Camera,
        size: [u32; 2],
        view: &viewer_models::motion_view::MotionView,
        seconds: f32,
    ) -> f32 {
        let mut next = f32::INFINITY;
        if self.graphics.supply_trains()
            && self.rail_fade.trains_visible()
            && let Some((_, assets, plan)) = &self.trains
        {
            next = plan.next_arrival(assets, view, seconds);
        }
        if self.graphics.aircraft()
            && viewer_aircraft::aircraft::draws(camera)
            && let Some((_, assets, plan)) = &self.aircraft
        {
            next = next.min(plan.next_arrival(assets, view, seconds, |mission| {
                self.graphics.air_mission(mission)
            }));
        }
        if self.graphics.missile_explosions
            && camera.height - viewer_models::WATER_HEIGHT < camera.defines.map_objects_cutoff
            && let Some((_, _, plan)) = &self.aircraft
        {
            let mut admission = None;
            next = next.min(plan.next_launch_impact(seconds, |bounds| {
                admission
                    .get_or_insert_with(|| {
                        viewer_gpu::province_admission::Admission::new(*camera, size)
                    })
                    .admits(bounds)
            }));
        }
        next
    }

    pub fn draw_refusals(&self) -> std::collections::BTreeSet<String> {
        let mut refusals = self.arena.refusals();
        refusals.extend(
            self.pass_diagnostics
                .keys()
                .map(|pass| format!("Map shader {}: refused", pass.name())),
        );
        refusals
    }

    pub fn shader_refusals(&self) -> &BTreeMap<String, u64> {
        &self.sprite_pass.native_refusals
    }

    pub fn icon_page_count(&self) -> usize {
        self.icon_textures.page_count()
    }

    pub fn tree_instances(&self) -> u32 {
        self.trees.as_ref().map_or(0, TreePass::drawn)
    }

    pub fn tree_draw_calls(&self) -> u32 {
        self.trees.as_ref().map_or(0, TreePass::drawn_buckets)
    }

    pub fn sprawl_drawn(&self) -> u32 {
        self.sprawl.as_ref().map_or(0, ModelPass::drawn)
    }

    pub fn map_buildings_drawn(&self) -> u32 {
        self.map_buildings.as_ref().map_or(0, ModelPass::drawn)
    }

    pub fn aircraft_drawn(&self) -> u32 {
        self.aircraft
            .as_ref()
            .map_or(0, |(pass, _, _)| pass.drawn())
    }

    pub fn unit_models_drawn(&self) -> u32 {
        self.unit_models.as_ref().map_or(0, ModelPass::drawn)
    }
}

fn write_range(
    queue: &wgpu::Queue,
    bound: &Bound,
    width: u32,
    bytes_per_texel: u32,
    first: usize,
    bytes: &[u8],
) {
    let texture = &bound.texture;
    bound.written();
    let mut pixel = first;
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let x = pixel as u32 % width;
        let y = pixel as u32 / width;
        let pixels = remaining.len() / bytes_per_texel as usize;
        let (columns, rows) = if x == 0 && pixels >= width as usize {
            (width, (pixels / width as usize) as u32)
        } else {
            ((width - x).min(pixels as u32), 1)
        };
        let count = columns as usize * rows as usize * bytes_per_texel as usize;
        shader_pipeline::gpu::stats::write_texture(
            &queue,
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &remaining[..count],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(columns * bytes_per_texel),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: columns,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        remaining = &remaining[count..];
        pixel += count / bytes_per_texel as usize;
    }
}

fn replace(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    slot: &mut Bound,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    data: &[u8],
) -> io::Result<bool> {
    let expected = size[0] as usize * size[1] as usize * 4;
    if data.len() != expected {
        return Err(invalid(format!(
            "{label} needs {expected} bytes for {}x{}, got {}",
            size[0],
            size[1],
            data.len()
        )));
    }
    if [slot.texture.width(), slot.texture.height()] == size {
        slot.write(queue, size, 4, data);
        return Ok(false);
    }
    *slot = data_texture(device, queue, label, size, format, data)?;
    Ok(true)
}

pub fn step_rail_fade(
    rail_fade: &mut viewer_rail::railways::RailFade,
    camera: &Camera,
    has_railways: bool,
    animation_time: f32,
    transition_time: Option<f32>,
    arena: &shader_pipeline::gpu::Arena,
) -> f32 {
    let rail_alpha = rail_fade.update(camera, transition_time.unwrap_or(animation_time));
    let rail_target = f32::from(u8::from(camera.height < camera.defines.railway_cutoff));
    if has_railways
        && rail_alpha != rail_target
        && camera.defines.railway_cutoff_speed > 0.0
    {
        arena.request_frame();
    }
    rail_alpha
}

pub fn world_size(size: [u32; 2], full_resolution: bool) -> [u32; 2] {
    if full_resolution {
        size
    } else {
        size.map(|side| side / 2)
    }
}

fn sky_pipeline(
    device: &wgpu::Device,
    effect: &Arc<shader_pipeline::library::Effect>,
    samples: u32,
) -> Result<viewer_terrain::sky_shader::Sky, String> {
    viewer_terrain::sky_shader::Sky::new(device, effect, samples, SCENE_FORMAT, DEPTH_FORMAT)
        .map_err(|error| error.to_string())
}

fn map_text_kind(
    family: replay_data::rows::MapEventFamily,
) -> Option<dashboard_core::protocol::MapTextKind> {
    use dashboard_core::protocol::MapTextKind;
    use replay_data::rows::MapEventFamily;
    Some(match family {
        MapEventFamily::FakeDivisionRevealed => MapTextKind::FakeDivisions,
        MapEventFamily::RailwayGunOverrun => MapTextKind::RailwayGuns,
        MapEventFamily::CriticalShoreBombardment => MapTextKind::Bombardments,
        MapEventFamily::GeneralCaptured => MapTextKind::Generals,
        MapEventFamily::ParadropLanding => MapTextKind::Paradrops,
        MapEventFamily::ResistanceActivity => MapTextKind::Resistance,
        MapEventFamily::BuildingDamage => MapTextKind::Buildings,
        MapEventFamily::ScriptedMapRefresh => return None,
    })
}

fn hidden_map_frame_actors(
    graphics: &dashboard_core::protocol::MapGraphics,
    table: impl IntoIterator<Item = (usize, u32, u32)>,
    impacts: impl IntoIterator<
        Item = (
            (usize, u32, u32),
            Option<viewer_models::strike_effects::Effect>,
        ),
    >,
) -> BTreeSet<(usize, u32, u32)> {
    let table = table.into_iter().filter(|_| !graphics.map_table);
    let impacts = impacts
        .into_iter()
        .filter_map(|(key, effect)| (!shows_impact(graphics, effect?)).then_some(key));
    table.chain(impacts).collect()
}

fn shows_impact(
    graphics: &dashboard_core::protocol::MapGraphics,
    effect: viewer_models::strike_effects::Effect,
) -> bool {
    use viewer_models::strike_effects::Effect;
    match effect {
        Effect::CriticalBombardment => graphics.bombardment_explosions,
        Effect::MissileExplosion => graphics.missile_explosions,
        Effect::Nuclear | Effect::Thermonuclear => graphics.nuclear_strikes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_frame_actors_hide_by_their_own_switch_and_stay_placed() {
        use viewer_models::strike_effects::Effect;
        let table = [(0, 1, 1)];
        let impacts = [
            ((1, 2, 2), Some(Effect::Nuclear)),
            ((1, 3, 3), Some(Effect::Thermonuclear)),
            ((2, 4, 4), Some(Effect::CriticalBombardment)),
            ((4, 6, 6), Some(Effect::MissileExplosion)),
            ((3, 5, 5), None),
        ];
        let hidden = |graphics| hidden_map_frame_actors(&graphics, table, impacts);
        let shown = dashboard_core::protocol::MapGraphics::default();
        assert!(hidden(shown).is_empty());
        assert_eq!(
            hidden(dashboard_core::protocol::MapGraphics {
                map_table: false,
                ..shown
            }),
            BTreeSet::from([(0, 1, 1)])
        );
        assert_eq!(
            hidden(dashboard_core::protocol::MapGraphics {
                nuclear_strikes: false,
                ..shown
            }),
            BTreeSet::from([(1, 2, 2), (1, 3, 3)])
        );
        assert_eq!(
            hidden(dashboard_core::protocol::MapGraphics {
                missile_explosions: false,
                ..shown
            }),
            BTreeSet::from([(4, 6, 6)])
        );
        assert_eq!(
            hidden(dashboard_core::protocol::MapGraphics {
                map_table: false,
                nuclear_strikes: false,
                bombardment_explosions: false,
                missile_explosions: false,
                ..shown
            }),
            BTreeSet::from([(0, 1, 1), (1, 2, 2), (1, 3, 3), (2, 4, 4), (4, 6, 6)])
        );
    }

    #[test]
    fn zero_and_oversized_render_sizes_are_refused() {
        assert!(validate_size([0, 100], 8192).is_err());
        assert!(validate_size([100, 0], 8192).is_err());
        assert!(validate_size([9000, 100], 8192).is_err());
        assert!(validate_size([2048, 1152], 8192).is_ok());
    }

}
