//! The map scene: GPU resources, the render targets and the pass order.
//!
//! One scene is built per document and reused for every frame, so the game
//! textures are uploaded once. Per-hour data arrives through the `set_*` hooks,
//! which rewrite the data textures in place and rebind the passes.

use crate::map::scene_content::MapAssets;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use viewer_foundation::map::gpu_contract::MAP_SAMPLED_TEXTURES;
use viewer_foundation::map::gpu_passes::PassTimer;
use viewer_foundation::map::lazy_icons::LazyTextures;
use viewer_foundation::map::presentation::Continuity;
use viewer_foundation::map::province_admission::side_planes;
use viewer_foundation::map::province_admission::views;
use viewer_foundation::map::province_map::ProvinceMap;
use viewer_foundation::map::province_schedule::{FrameRestore, ProvinceFrame, Shared};
use viewer_foundation::map::textures::fow_size;
use viewer_foundation::profile_writer::profiling;
use viewer_foundation::profiling::CpuLaps;

use viewer_foundation::map::gpu_contract::{
    DEPTH_FORMAT, OUTPUT_FORMAT, SCENE_FORMAT, Targets, invalid, validate_size,
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
pub use input::{SceneInput, SceneOutput};

#[cfg(any(test, feature = "test-support"))]
use super::scene_faults as failure_tests;

use eframe::wgpu;

use {
    crate::map::border_pass::BorderAssets, crate::map::border_pass::BorderFrame,
    crate::map::border_pass::BorderPass, crate::map::border_pass::BorderStage,
    crate::map::border_pass::StraitVertex, crate::map::border_pass::wrap_offsets,
    crate::map::borders::BorderMeshes, crate::map::demilitarized::DemilitarizedPass,
    crate::map::demilitarized::Zone, crate::map::political::Regions,
    crate::map::shadow::ShadowPass, crate::map::terrain::Grid, crate::map::terrain::MARGIN,
    crate::map::water::WaterPass, viewer_foundation::map::camera::Camera,
    viewer_foundation::map::frame::DayNight, viewer_foundation::map::frame::FrameConstants,
    viewer_foundation::map::textures::Bound, viewer_foundation::map::textures::ColorSpace,
    viewer_foundation::map::textures::data_texture, viewer_foundation::map::textures::upload,
    viewer_foundation::map::textures::write_data,
};
// names
use {
    crate::map::name_pass::NamePass, crate::map::post::PostPass,
    viewer_foundation::map::name_draw::Placements, viewer_foundation::map::sprites::Batch,
    viewer_foundation::map::sprites::IconTextures, viewer_foundation::map::sprites::SpritePass,
};
// rivers
use {
    crate::map::rivers::RiverAssets, crate::map::rivers::RiverMesh, crate::map::rivers::RiverPass,
};
// the map's table frame
use crate::map::mapframe::FrameAssets;
// trees
use {
    crate::map::trees::TreeAssets, crate::map::trees::TreeField, crate::map::trees::TreeFrame,
    crate::map::trees::TreeGeometry, crate::map::trees::TreePass,
};
// close zoom: the shared static-mesh pass and the layers built on it
use crate::map::railways::RailFade;
use {
    crate::map::map_buildings::BUILDING_TILE, crate::map::map_buildings::BuildingHistory,
    crate::map::map_buildings::MapBuildingAssets,
};
use {
    crate::map::models::ModelField, crate::map::models::ModelFrame, crate::map::models::ModelPass,
    crate::map::models::draws_city_sprawl,
};
use {
    crate::map::railways::Bridges, crate::map::railways::RailAssets,
    crate::map::railways::RailMesh, crate::map::railways::RailPass,
};
// orders: the recorded player's battle plan
use crate::map::order_pass::{OrderPass, PlanSubmissions};
use crate::map::sprawl::Sprawl;
use viewer_foundation::map::ribbon_content::Mode as ArrowMode;
use viewer_foundation::map::ribbon_content::{OrderAssets, OrderField};
use {
    crate::map::unit_models::UnitModelAssets,
    viewer_foundation::map::model_admission::Gates as UnitGates,
};

// Expand field borrows so rendering can also mutate the pass diagnostics.
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

/// CPU frame inputs; each compiled adapter uploads its native buffer layout.
struct FrameInputs {
    frame: FrameConstants,
    shadow_matrix: [[f32; 4]; 4],
    light_grid: [f32; 4],
}

/// CPU border plans prepared on a builder for one faction-pin province union.
pub struct FactionHighlight {
    classes: crate::map::border_pass::ClassPlan,
    zone: crate::map::demilitarized::ZonePlan,
    hover: crate::map::demilitarized::ZonePlan,
}

impl FactionHighlight {
    /// Select adjacencies leaving the union; partitions
    /// ordinary and secondary buckets, including custom material-0xff strips.
    pub fn new(
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
        provinces: &BTreeSet<u32>,
    ) -> Self {
        let highlight = |a, b| provinces.contains(&a) != provinces.contains(&b);
        Self {
            classes: crate::map::border_pass::ClassPlan::matching(meshes, regions, |a, b| {
                !zone.splits(a, b) && !highlight(a, b)
            }),
            zone: crate::map::demilitarized::ZonePlan::matching(meshes, regions, |a, b| {
                zone.splits(a, b) && !highlight(a, b)
            }),
            hover: crate::map::demilitarized::ZonePlan::custom_highlight(
                meshes, regions, highlight,
            ),
        }
    }
}

/// The uploaded game textures, the data textures and the two passes.
pub struct Scene {
    #[cfg(any(test, feature = "test-support"))]
    pub test_faults: failure_tests::Faults,
    graphics: dashboard_core::protocol::MapGraphics,
    /// The light depth map, screen-space receiver and blur targets.
    shadow: ShadowPass,
    atlas: Bound,
    atlas_normal: Bound,
    colormap: Bound,
    city_lights: Bound,
    snow: Bound,
    mud_diffuse: Bound,
    mud_normal: Bound,
    colormap_water: Bound,
    underwater: Bound,
    lean1: Bound,
    lean2: Bound,
    water_specular: Bound,
    naval_dominance: Bound,
    // fog of war
    intel_map: Bound,
    intel_installed: Option<map_display_data::Paged<u8>>,
    fow_noise: Bound,
    /// Whether this hour has a fog-of-war input at all; see
    /// [`Scene::set_fog_of_war`].
    fog: bool,
    /// What the view's map mode shows of the map's layers; see [`Scene::set_visibility`].
    visibility: viewer_foundation::map::mode_visibility::Visibility,
    point_light_maps: crate::map::pointlights::PointLightMaps,
    point_light_tiles: crate::map::pointlights::Tiles,
    reflection: Bound,
    ice_diffuse: Bound,
    ice_noise: Bound,
    height_normal: Bound,
    heightmap: Bound,
    terrain_ids: Bound,
    snow_mud: Bound,
    /// One texel per snow/mud cell, set where [`crate::map::weather::Presentation::unknown`]
    /// hatches it.
    snow_unknown: Bound,
    /// The terrain's vertex heights for the hatch, when [`Self::heightmap`] is not them.
    hatch_heights: Option<Bound>,
    /// Where the map holds a hatched cell; none skips the hatch.
    hatch_coverage: Option<crate::map::weather::HatchCoverage>,
    /// Built on the first hour with an unknown cell for the targets' sample count, or why not.
    unknown_hatch: Option<(u32, Result<crate::map::weather::UnknownHatch, String>)>,
    snow_presentation: crate::map::weather::Presentation,
    snow_render_delta: crate::map::weather::RenderDelta,
    political: Option<crate::map::political_gpu::View>,
    /// The shown map mode's camera override and outline cutoff per channel.
    mode_channels: [Option<[f32; 2]>; 2],
    gradient1: Bound,
    gradient2: Bound,
    secondary: Bound,
    province_colors: Option<super::political_gpu::ProvinceColors>,
    compiled_terrain: crate::map::terrain_shader::Terrain,
    terrain_null: Bound,
    water: WaterPass,
    arrow_effect: Arc<shader_pipeline::library::Effect>,
    tree_effect: Arc<shader_pipeline::library::Effect>,
    sky_effect: Arc<shader_pipeline::library::Effect>,
    sky: Option<Result<crate::map::sky_shader::Sky, String>>,
    /// Constants and repacked vertex streams of the frame being encoded.
    arena: shader_pipeline::gpu::Arena,
    // colour
    pub post: PostPass,
    pub last_animation_time: Option<f32>,
    /// The last rendered frame's entity events that name a sound effect.
    pub sound_events: Vec<crate::map::sound_events::SoundEvent>,
    /// This frame's sounds refused by name, with the actors refused.
    pub sound_refusals: BTreeMap<String, usize>,
    sound_names: crate::map::sound_events::Names,
    /// The railway manager's camera fade, stepped on the animation clock.
    rail_fade: RailFade,
    /// Milliseconds the last [`Scene::render`] spent animating, uploading the
    /// overlay, encoding and submitting, under `BATTLE_PLAN_VIEWER_PROFILE`.
    render_laps: [f64; 4],
    /// Per-pass GPU timestamps under `BATTLE_PLAN_VIEWER_GPU_PASSES`.
    pass_timer: Option<PassTimer>,
    /// The hour builders' threads a frame's presentation step may lend its work;
    /// the calling thread alone until the window lends them ([`Scene::use_workers`]).
    workers: viewer_foundation::parallel::Workers,
    /// Cut updates remain recoverable until the frame reaches its province walk.
    province_frame: Option<(ProvinceFrame, FrameRestore)>,
    pub pass_diagnostics: passes::Diagnostics,
    /// CPU laps inside animation and overlay preparation, under `BATTLE_PLAN_VIEWER_PROFILE`.
    detail_laps: CpuLaps,
    /// Coarse caller-side pass/finish/submit laps, independent of GPU queries.
    pass_cpu_laps: CpuLaps,
    pub targets: Option<Targets>,
    grid: Grid,
    /// The terrain the frame draws: the uniform grid, or a free eye's blocks.
    cover: crate::map::terrain::Cover,
    generation: u64,
    // borders
    /// Kept so the border pass can be built after the scene, once the document has traced it.
    borders: Option<BorderPass>,
    /// The engine's secondary bucket bank, built beside the border pass; see `crate::map::demilitarized`.
    demilitarized: Option<DemilitarizedPass>,
    /// The rendered pin currently owning a local pointer highlight.
    faction_highlight_regions: Vec<u32>,
    // names
    icon_textures: IconTextures,
    name_pass: NamePass,
    /// The names written close in ([`Placements::near`]), fading in as the country layer fades
    /// out.
    near_name_pass: NamePass,
    /// The battle-plan labels: the same `mapname` effect, drawn right after
    /// the plan's ribbons.
    order_label_pass: [NamePass; 3],
    order_labels: [Placements; 3],
    sprite_pass: SpritePass,
    sprite_camera: Option<viewer_foundation::map::sprites::CameraFrame>,
    empty_batch: Batch,
    empty_placements: Placements,
    // rivers
    /// Built once the document has traced the river bitmap; see [`Scene::set_rivers`].
    rivers: Option<RiverPass>,
    // the map's table frame
    /// Built once the document has read the frame's ambient objects; see [`Scene::set_map_frame`].
    pub native_map_frame: Option<ModelPass>,
    ambient: Option<crate::map::mapframe::Presentation>,
    /// Province effect slots drawn through the map-frame pass; see [`Scene::set_nuclear_impacts`].
    strikes: crate::map::strike_effects::Presentation,
    /// The aircraft clock up to which missile scenarios have started their
    /// explosions.
    missile_clock: Option<f32>,
    // trees
    /// Built once the document has generated the tree lattice; see [`Scene::set_trees`].
    trees: Option<TreePass>,
    // close zoom
    /// Built once the document has generated the sprawl; see [`Scene::set_sprawl`].
    sprawl: Option<ModelPass>,
    city_presentation: Option<crate::map::sprawl::Presentation>,
    /// Built once the document has read the building slots; the history
    /// applies each hour to the camera. See [`Scene::set_map_buildings`].
    map_buildings: Option<ModelPass>,
    building_history: BuildingHistory,
    /// The content's per-frame model creation allowance
    /// ([`crate::map::mesh_budget`]), shared by buildings, hulls and land figures.
    mesh_limit: i32,
    /// Built once the document has traced the rail network; see
    /// [`Scene::set_railways`].
    railways: Option<RailPass>,
    aircraft: Option<(
        ModelPass,
        Arc<crate::map::aircraft::AircraftAssets>,
        crate::map::aircraft::AircraftPlan,
    )>,
    trains: Option<(
        ModelPass,
        Arc<crate::map::trains::TrainAssets>,
        crate::map::trains::TrainPlan,
    )>,
    // orders: the recorded player's battle plan; see [`Scene::set_orders`].
    orders: Option<OrderPass>,
    radar: Option<crate::map::radar_symbols::Pass>,
    radar_refusal: Option<String>,
    air_ranges: Option<crate::map::radar_symbols::Pass>,
    air_ranges_refusal: Option<String>,
    air_transfers: Option<crate::map::radar_symbols::Pass>,
    air_transfers_refusal: Option<String>,
    pub movement_arrows: crate::map::movement_arrows::Player,
    /// The floating map text of the recorded map events; see
    /// [`Scene::install_animated_text`] and [`Scene::present_hours`].
    animated_text: Option<crate::map::animated_map_text::Player>,
    map_text_events: crate::map::animated_map_text::events::Presenter,
    map_text_refusals: viewer_foundation::table_access::Refusals,
    weather_effects: Option<crate::map::weather_effects::Pass>,
    /// The railway bridges, which are meshes rather than strip; see
    /// [`Scene::set_bridges`].
    bridges: Option<ModelPass>,
    /// Built once the document has resolved the infantry entities; its
    /// instances are rewritten every frame from the unit history, which
    /// stands the raid units at every height. See [`Scene::set_unit_models`].
    unit_models: Option<ModelPass>,
    /// The content's unit and ship camera cutoffs, set with the pass.
    unit_gates: Option<UnitGates>,
    pub unit_history: crate::map::unit_history::History,
    /// The model a free camera mode's user steers; see [`Scene::set_player`].
    player: Option<player::Player>,
}

/// One frame's worth of arguments to [`Scene::render`].
#[derive(Clone, Copy)]
pub struct Request<'a> {
    /// Render target size in device pixels.
    pub size: [u32; 2],
    pub camera: Camera,
    pub day_night: DayNight,
    /// `vGlobalTime`, and `FoWTime` beside it: the fog-of-war noise drifts on it.
    pub animation_time: f32,
    /// Map presentation clock for selection pulses, river flow and waves, separate from vGlobalTime
    /// and animation_time.
    pub map_time: f32,
    // colour
    /// Whether the `post` stage grades the frame. See [`Scene::render`].
    pub postfx: bool,
    /// Verification aid, `--dump --no-water` only: skip the water plane to
    /// expose lit terrain, as the game's `debug_water` toggle does. The
    /// separate underwater refraction target is then unused by the scene.
    pub no_water: bool,
    // Dump-only no-shadows omits the cast-shadow chain. Lit passes sample its clear value and
    // receive fully lit shadow terms.
    pub no_shadows: bool,
    // names
    pub overlay: Overlay<'a>,
    // Tree season operands for this hour. Dump-only no-trees omits canopy and tree shadow casters,
    // exposing bare lit terrain.
    pub no_trees: bool,
}

// names
/// What the two overlay passes draw on top of the world passes this frame.
///
/// Both are built by the caller, which owns the camera the labels were
/// projected with, so the overlay and the frame it is drawn into always agree.
#[derive(Clone, Copy)]
pub struct Overlay<'a> {
    /// The placed country names, in world space.
    pub names: Option<&'a Placements>,
    /// The integrated country-name fade.
    pub layer_alpha: f32,
    pub strategic_names: super::name_pass::StrategicNames,
    /// Screen-space icon quads, in the order the caller pushed them, in
    /// interface pixels.
    pub sprites: Option<&'a Batch>,
    /// Device pixels per interface pixel of `sprites`; see
    /// [`super::ui_scale`]. The world-space names do not take it.
    pub ui_scale: f32,
    /// Whether the map's interface that stands in the world draws: borders, demilitarised zones
    /// and strait crossings, battle plans with their labels and raid paths, the selected units'
    /// movement arrows and floating map text. Off, as the fly and walk modes leave it by default,
    /// they still step and keep their state; only their drawing stops.
    pub interface: bool,
    /// Whether the country names draw: with the interface, and in the fly mode on their own.
    pub country_names: bool,
    /// What a free eye's floating map text stands on, for its sight test; `None` tests nothing.
    pub ground: Option<viewer_foundation::map::free_icons::Ground<'a>>,
}

impl Default for Overlay<'_> {
    /// Nothing to draw, at the unscaled interface size.
    fn default() -> Self {
        Self {
            names: None,
            layer_alpha: 0.0,
            strategic_names: super::name_pass::StrategicNames::All,
            sprites: None,
            ui_scale: 1.0,
            interface: true,
            country_names: true,
            ground: None,
        }
    }
}

impl Scene {
    #[cfg(any(test, feature = "test-support"))]
    pub fn offline_particle_packets(&self) -> BTreeMap<String, usize> {
        self.weather_effects
            .as_ref()
            .map(|w| w.offline_packets.clone())
            .unwrap_or_default()
    }

    /// Uploads every game texture and builds both pipelines.
    // Resources are shared by the name and icon layers.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &MapAssets,
        icon_images: &[installed_content_image::dds::Texture],
        lazy: &Arc<LazyTextures>,
        graphics: dashboard_core::protocol::MapGraphics,
    ) -> io::Result<Self> {
        crate::map::political_gpu::check_device(device)?;
        let samples = graphics.samples;
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
        if !device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        {
            return Err(invalid(
                "this GPU does not support BC texture compression, which the game's terrain atlas, \
                 water and border textures are authored in",
            ));
        }
        let sampled = device.limits().max_sampled_textures_per_shader_stage;
        if sampled < MAP_SAMPLED_TEXTURES {
            return Err(invalid(format!(
                "this GPU binds {sampled} sampled textures per shader stage; the map needs {}",
                MAP_SAMPLED_TEXTURES
            )));
        }
        // fog of war: the intel map is rewritten in place every hour, so it is
        // allocated at its final size here and the bind groups below outlive
        // every one of its contents. An all-zero grid is "no fog anywhere",
        // which is what an hour with no recorded visibility draws.
        let fow = fow_size(assets.map);
        let intel_map = data_texture(
            device,
            queue,
            "IntelMap",
            fow,
            // Native exposes A8_UNORM. RGBA preserves its (0,0,0,a)
            // samples for both compiled content and the remaining port consumers.
            wgpu::TextureFormat::Rgba8Unorm,
            &vec![0u8; (fow[0] * fow[1] * 4) as usize],
        )?;
        let fow_noise = upload(
            device,
            queue,
            "FOWNoise",
            &assets.fow_noise,
            ColorSpace::Linear,
        )?;
        // `FOWHeight` and the water's `SpecularMap` are the same file, which is
        // what its name says: `fow_rgb_waterspec_a`.
        let water_specular = upload(
            device,
            queue,
            "SpecularMap",
            &assets.water_specular,
            ColorSpace::Linear,
        )?;
        let point_light_maps = crate::map::pointlights::PointLightMaps::new(device, queue)?;
        let shadow = ShadowPass::new(device, &assets.shadow_blur)?;

        let height_normal = data_texture(
            device,
            queue,
            "HeightNormal (world_normal.bmp)",
            [assets.world_normal.width, assets.world_normal.height],
            wgpu::TextureFormat::Rgba8Unorm,
            &assets.world_normal.pixels,
        )?;
        let (height_size, height_texels) = assets.height_texture();
        let heightmap = data_texture(
            device,
            queue,
            "heightmap",
            height_size,
            wgpu::TextureFormat::R8Unorm,
            height_texels,
        )?;
        let terrain_ids = data_texture(
            device,
            queue,
            "TerrainIDMap",
            [assets.heightmap.width, assets.heightmap.height],
            wgpu::TextureFormat::Rgba8Unorm,
            &assets.terrain_ids,
        )?;
        let snow_mud_size = crate::map::weather::grid_size(assets.map.width, assets.map.height);
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
        let hatch_heights = assets
            .height_texture
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
            .transpose()?;
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
        // An all-zero channel 2 makes `TranspA`, `TranspB` and `FX_Alpha.g` zero,
        // so `gradient_border_apply` leaves the colour alone and reports a bloom
        // alpha of 1: no political fill until the border worker supplies fields.
        let gradient1 = gradient("GradientBorderChannel1")?;
        let gradient2 = gradient("GradientBorderChannel2")?;
        // One transparent texel: `secondary_color_mask` multiplies the stripe by
        // the mask alpha, so a zero alpha is the whole map having no occupation.
        let secondary = data_texture(
            device,
            queue,
            "ProvinceSecondaryColorMap",
            [1, 1],
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &[0, 0, 0, 0],
        )?;

        // names
        let icon_textures = IconTextures::new(device, queue, icon_images)?;
        let name_pass = NamePass::new(
            device,
            samples,
            &icon_textures,
            SCENE_FORMAT,
            &assets.name_shader,
            None,
        )?;
        let near_name_pass = NamePass::new(
            device,
            samples,
            &icon_textures,
            SCENE_FORMAT,
            &assets.name_shader,
            None,
        )?;
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
        let sprite_pass =
            SpritePass::new(device, queue, samples, &icon_textures, lazy, OUTPUT_FORMAT)?;

        let scene = Self {
            #[cfg(any(test, feature = "test-support"))]
            test_faults: Default::default(),
            graphics,
            shadow,
            atlas: upload(device, queue, "atlas0", &assets.atlas, ColorSpace::Srgb)?,
            atlas_normal: upload(
                device,
                queue,
                "atlas_normal0",
                &assets.atlas_normal,
                ColorSpace::Linear,
            )?,
            colormap: upload(
                device,
                queue,
                "TerrainColorTint",
                &assets.colormap,
                ColorSpace::Srgb,
            )?,
            city_lights: upload(
                device,
                queue,
                "CityLightsAndSnowNoise",
                &assets.city_lights,
                ColorSpace::Srgb,
            )?,
            snow: upload(
                device,
                queue,
                "SnowTexture",
                &assets.snow,
                ColorSpace::Linear,
            )?,
            mud_diffuse: upload(
                device,
                queue,
                "MudDiffuseGloss",
                &assets.mud_diffuse,
                ColorSpace::Srgb,
            )?,
            mud_normal: upload(
                device,
                queue,
                "MudNormalSpec",
                &assets.mud_normal,
                ColorSpace::Linear,
            )?,
            colormap_water: upload(
                device,
                queue,
                "colormap_water",
                &assets.colormap_water,
                ColorSpace::Srgb,
            )?,
            underwater: upload(
                device,
                queue,
                "underwater_terrain",
                &assets.underwater,
                ColorSpace::Srgb,
            )?,
            lean1: upload(
                device,
                queue,
                "LeanTexture1",
                &assets.lean1,
                ColorSpace::Linear,
            )?,
            lean2: upload(
                device,
                queue,
                "LeanTexture2",
                &assets.lean2,
                ColorSpace::Linear,
            )?,
            water_specular,
            naval_dominance: upload(
                device,
                queue,
                "naval dominance colors",
                &assets.naval_dominance,
                ColorSpace::Srgb,
            )?,
            // fog of war
            intel_map,
            intel_installed: None,
            fow_noise,
            fog: false,
            visibility: viewer_foundation::map::mode_visibility::Visibility::POLITICAL,
            point_light_maps,
            point_light_tiles: crate::map::pointlights::Tiles::default(),
            reflection: upload(
                device,
                queue,
                "ReflectionCubeMap",
                &assets.reflection,
                ColorSpace::Srgb,
            )?,
            ice_diffuse: upload(
                device,
                queue,
                "IceDiffuse",
                &assets.ice_diffuse,
                ColorSpace::Linear,
            )?,
            ice_noise: upload(
                device,
                queue,
                "IceNoise",
                &assets.ice_noise,
                ColorSpace::Linear,
            )?,
            height_normal,
            heightmap,
            terrain_ids,
            snow_mud,
            snow_unknown,
            hatch_heights,
            hatch_coverage: None,
            unknown_hatch: None,
            snow_presentation: Default::default(),
            snow_render_delta: Default::default(),
            political: None,
            mode_channels: [None; 2],
            gradient1,
            gradient2,
            secondary,
            province_colors: None,
            compiled_terrain: crate::map::terrain_shader::Terrain::new(
                device,
                &assets.terrain_shader,
                assets.heightmap_texels(),
                [assets.map.width, assets.map.height],
                samples,
            )?,
            terrain_null: data_texture(
                device,
                queue,
                "terrain null",
                [1, 1],
                wgpu::TextureFormat::Rgba8Unorm,
                &[0; 4],
            )?,
            water: WaterPass::new(
                &assets.water_shader,
                [assets.map.width, assets.map.height],
                samples,
                SCENE_FORMAT,
            ),
            // colour
            post: PostPass::new(device, samples, queue, &assets.post, OUTPUT_FORMAT)?,
            sky_effect: assets.sky_shader.clone(),
            arrow_effect: assets.arrow_shader.clone(),
            tree_effect: assets.tree_shader.clone(),
            sky: None,
            arena: shader_pipeline::gpu::Arena::new(device),
            last_animation_time: None,
            sound_events: Vec::new(),
            sound_refusals: BTreeMap::new(),
            sound_names: Default::default(),
            rail_fade: RailFade::default(),
            render_laps: [0.0; 4],
            pass_timer: PassTimer::new(device, queue),
            workers: Default::default(),
            province_frame: None,
            pass_diagnostics: Default::default(),
            detail_laps: Default::default(),
            pass_cpu_laps: Default::default(),
            targets: None,
            grid: Grid {
                origin: [0.0, 0.0],
                step: 1.0,
                counts: [0, 0],
            },
            cover: crate::map::terrain::Cover::Blocks(Vec::new()),
            generation: 0,
            // borders
            borders: None,
            demilitarized: None,
            faction_highlight_regions: Vec::new(),
            // names
            icon_textures,
            name_pass,
            near_name_pass,
            order_label_pass,
            order_labels: std::array::from_fn(|_| Placements::default()),
            sprite_pass,
            sprite_camera: None,
            empty_batch: Batch::default(),
            empty_placements: Placements::default(),
            // rivers
            rivers: None,
            // the map's table frame
            native_map_frame: None,
            ambient: None,
            strikes: Default::default(),
            missile_clock: None,
            // trees
            trees: None,
            // close zoom
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
            movement_arrows: crate::map::movement_arrows::Player::new(
                device,
                queue,
                &assets.movement_arrows,
                samples,
            ),
            weather_effects: None,
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
        assets: Arc<crate::map::weather_effects::Assets>,
        field: crate::map::weather_effects::Field,
    ) -> io::Result<()> {
        if self.weather_effects.is_none() {
            self.weather_effects = Some(crate::map::weather_effects::Pass::new(
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

    /// Install this hour's empty particle runtimes, prepared on a builder thread by
    /// [`crate::map::weather_effects::prepare`]. Return retired runtimes for disposal off the frame path.
    pub fn set_prepared_weather_effects(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: Arc<crate::map::weather_effects::Assets>,
        prepared: crate::map::weather_effects::Prepared,
        continuity: Continuity,
    ) -> io::Result<crate::map::weather_effects::Retired> {
        let pass = match self.weather_effects.take() {
            Some(pass) => pass,
            None => crate::map::weather_effects::Pass::new(
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

    /// Creates the per-hour GPU passes before the first hour arrives, so the
    /// first map frame pays only data uploads.
    ///
    /// The loader compiles pipelines and uploads static libraries and textures while builders
    /// prepare the first hour. Hourly data arrives through the matching `set_*` methods. Existing
    /// passes are retained, keeping this work off the frame path.
    pub fn warm_trains(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &Arc<crate::map::trains::TrainAssets>,
    ) -> io::Result<()> {
        if self.trains.is_none() {
            let mut pass = ModelPass::new(&assets.library, "supply trains");
            pass.upload_library(device, queue, &assets.library)?;
            self.trains = Some((
                pass,
                Arc::clone(assets),
                crate::map::trains::TrainPlan::default(),
            ));
        }
        Ok(())
    }

    /// See [`Scene::warm_trains`].
    pub fn warm_aircraft(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &Arc<crate::map::aircraft::AircraftAssets>,
    ) -> io::Result<()> {
        if self.aircraft.is_none() {
            let mut pass = ModelPass::new(&assets.library(), "flying aircraft");
            pass.upload_library(device, queue, &assets.library())?;
            self.aircraft = Some((
                pass,
                Arc::clone(assets),
                crate::map::aircraft::AircraftPlan::default(),
            ));
        }
        Ok(())
    }

    /// The content's unit camera cutoffs the scene admits unit models by;
    /// `None` before the unit pass exists.
    pub fn unit_gates(&self) -> Option<UnitGates> {
        self.unit_gates
    }

    /// See [`Scene::warm_trains`].
    pub fn warm_unit_models(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &UnitModelAssets,
    ) -> io::Result<()> {
        if self.unit_models.is_none() {
            let library = assets.library();
            let mut pass = ModelPass::new(&library, "unit models");
            pass.upload_library(device, queue, &library)?;
            drop(library);
            self.unit_models = Some(pass);
        }
        // The content's camera cutoffs come with the pass, however it was
        // created: a window warms it at load, before any hour's field.
        self.unit_gates = Some(assets.gates);
        Ok(())
    }

    /// See [`Scene::warm_trains`].
    pub fn warm_map_buildings(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &MapBuildingAssets,
    ) -> io::Result<()> {
        if self.map_buildings.is_none() {
            let mut pass = ModelPass::new(&assets.library, "map buildings");
            pass.upload_library(device, queue, &assets.library)?;
            self.map_buildings = Some(pass);
        }
        Ok(())
    }

    /// See [`Scene::warm_trains`].
    pub fn warm_bridges(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bridges: &Bridges,
    ) -> io::Result<()> {
        if self.bridges.is_none() {
            let mut pass = ModelPass::new(&bridges.library, "railway bridges");
            pass.upload_library(device, queue, &bridges.library)?;
            self.bridges = Some(pass);
        }
        Ok(())
    }

    /// See [`Scene::warm_trains`].
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

    /// See [`Scene::warm_trains`].
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
        for (pass, refusal, page, label) in [
            (
                &mut self.radar,
                &mut self.radar_refusal,
                &assets.radar,
                "Radar symbols",
            ),
            (
                &mut self.air_ranges,
                &mut self.air_ranges_refusal,
                &assets.air_range,
                "Air ranges",
            ),
            (
                &mut self.air_transfers,
                &mut self.air_transfers_refusal,
                &assets.air_transfer,
                "Air transfer progress",
            ),
        ] {
            if pass.is_some() || refusal.is_some() {
                continue;
            }
            match page.as_ref().map_err(Clone::clone).and_then(|page| {
                crate::map::radar_symbols::Pass::named(
                    device,
                    queue,
                    self.graphics.samples,
                    page,
                    &self.arrow_effect,
                    label,
                )
                .map_err(|e| e.to_string())
            }) {
                Ok(value) => *pass = Some(value),
                Err(reason) => *refusal = Some(reason),
            }
        }
    }

    pub fn set_air_ranges(&mut self, field: &crate::map::radar_symbols::Field) {
        if let Some(pass) = &mut self.air_ranges {
            pass.set(field);
            pass.shown = true;
        }
    }

    pub fn set_air_transfers(&mut self, field: &crate::map::radar_symbols::Field) {
        if let Some(pass) = &mut self.air_transfers {
            pass.set(field);
            pass.shown = true;
        }
    }

    pub fn set_radar(
        &mut self,
        field: &crate::map::radar_symbols::Field,
        shown: bool,
        hovered: Option<(u32, u32)>,
    ) {
        if let Some(pass) = &mut self.radar {
            pass.set(field);
            pass.shown = shown;
            pass.hovered = hovered;
        }
    }

    /// See [`Scene::warm_trains`].
    pub fn warm_weather(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: Arc<crate::map::weather_effects::Assets>,
    ) -> io::Result<()> {
        if self.weather_effects.is_none() {
            self.weather_effects = Some(crate::map::weather_effects::Pass::new(
                device,
                self.graphics.samples,
                queue,
                assets,
            )?);
        }
        Ok(())
    }

    // trees
    // the map's table frame
    /// Uploads content-resolved ambient mesh actors through the shared mesh adapter.
    pub fn set_map_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &FrameAssets,
    ) -> io::Result<()> {
        let mut pass = ModelPass::new(&assets.library, "map frame");
        // Every frame mesh is resident: province effects are placed later.
        pass.upload_library(device, queue, &assets.library)?;
        pass.upload(device, queue, &assets.library, assets.field.clone())?;
        self.native_map_frame = Some(pass);
        self.ambient = Some(assets.presentation.clone());
        self.strikes = crate::map::strike_effects::Presentation::new(assets.strikes.clone());
        Ok(())
    }

    // trees
    /// Builds the tree pass, uploads the canopy meshes and the instance lattice.
    ///
    /// Called once per document: `map/trees.bmp`, `map/heightmap.bmp` and the canopy meshes are all
    /// static content, so nothing about an instance changes with the hour.
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

    // close zoom

    // close zoom

    // close zoom

    // close zoom
    /// Builds the unit-model pass and uploads the hour's placed figures.
    ///
    /// The entity library is static content, so the pass is built once; the
    /// figures come from `Divisions` and are rewritten whenever the hour
    /// changes. The unit history decides each frame which of them stand
    /// ([`crate::map::unit_history`]); `steady` settles it for an offline frame.
    pub fn set_unit_models(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &UnitModelAssets,
        field: crate::map::unit_history::Field,
        steady: bool,
    ) -> io::Result<()> {
        if self.unit_models.is_none() {
            self.unit_models = Some(ModelPass::new(&assets.library(), "unit models"));
        }
        self.unit_gates = Some(assets.gates);
        self.unit_history.set_prepared_with_naval(
            field.targets,
            assets.bounds.clone(),
            steady,
            field.naval,
        );
        self.unit_history.set_raids(field.raids);
        if let Some(pass) = &mut self.unit_models {
            pass.upload(device, queue, &assets.library(), field.models)?;
        }
        Ok(())
    }

    /// Offline exports settle models even when the unit layer is omitted.
    pub fn settle_models(&mut self) {
        self.unit_history.settle();
    }

    /// Diagnostic frame sequences exercise the same history as the window.
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

    /// Runs the unit entities on `schedule`, the view's counters' own
    /// ([`super::province_schedule`]).
    pub fn share_province_schedule(&mut self, schedule: &Shared) {
        self.unit_history.share_schedule(schedule);
        if let Some(text) = &mut self.animated_text {
            text.share_schedule(schedule);
        }
    }

    /// Builds the floating map text player from the content's text types and
    /// the GUI resources that loaded their fonts onto this scene's icon pages.
    pub fn install_animated_text(
        &mut self,
        assets: &crate::map::animated_map_text::Assets,
        gui: &viewer_foundation::map::native_resources::GuiResources,
    ) {
        self.animated_text = Some(crate::map::animated_map_text::Player::new(
            assets,
            gui,
            self.icon_textures.views(),
            self.graphics.samples,
        ));
    }

    /// Starts impacts that arrive without map events, as an offline scene's
    /// synthetic ones do. A reset clears every province's effect slot first.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_nuclear_impacts<'a>(
        &mut self,
        hours: impl IntoIterator<Item = &'a crate::map::strike_effects::Impacts>,
        continuity: Continuity,
    ) {
        self.strikes.receive(
            hours,
            continuity == Continuity::Reset,
            self.last_animation_time.unwrap_or(0.0),
        );
    }

    /// Presents the hours shown since the last call, oldest first: each
    /// hour's nuclear impacts start their province effects, then its map
    /// events, interpreted for the viewing country on the hour builders,
    /// queue their floating text and start their critical bombardment
    /// explosions, each recorded event once. A discontinuity clears the
    /// effect slots and starts the queues again; coverage and refusals of the
    /// last hour stay visible.
    #[allow(clippy::too_many_arguments)] // The hour, its events and where its text stands.
    pub fn present_hours<'a>(
        &mut self,
        at: (replay_data::BranchId, i32),
        seek: bool,
        hours: impl IntoIterator<
            Item = (
                &'a crate::map::strike_effects::Impacts,
                &'a crate::map::animated_map_text::events::Prepared,
            ),
        >,
        continuity: Continuity,
        anchor: impl Fn(u32) -> Option<[f32; 3]>,
        heights: viewer_foundation::map::province_map::HeightField<'_>,
        gui: &viewer_foundation::map::native_resources::GuiResources,
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

    /// A presented hour at `at` that failed has no coherent fog
    /// reconstruction; its queues start again and the reason is shown.
    /// Consumed events are forgotten only where a built hour would forget
    /// them, so retrying the same hour presents nothing twice.
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
        viewer_foundation::table_access::refuse(&mut self.map_text_refusals, reason);
    }

    /// Advances the view's province schedule for the frame about to be
    /// presented at this camera, before its map icons read it; the frame's
    /// render takes the advance ([`crate::map::unit_history::History::advance_schedule`]).
    pub fn advance_provinces(&mut self, camera: Camera, size: [u32; 2]) {
        self.unit_history.advance_schedule(camera, size);
    }

    /// Cut after the final selection change; an unwalked frame returns its force counts.
    pub fn cut_province_frame(&mut self, animation_time: f32) {
        self.restore_province_frame();
        self.province_frame = self.unit_history.take_province_frame(animation_time);
    }

    /// Gives back a cut that no walk read.
    fn restore_province_frame(&mut self) {
        if let Some((_, restore)) = self.province_frame.take() {
            self.unit_history.untake_province_frame(restore);
        }
    }

    /// Recreates the unit entities of `provinces`, or of every province
    /// ([`viewer_foundation::map::presentation::UnitRestart`]).
    pub fn restart_unit_models(&mut self, provinces: Option<&BTreeSet<u32>>) {
        if provinces.is_none() {
            self.movement_arrows.reset();
        }
        if let Some(text) = &mut self.animated_text {
            text.reset();
        }
        self.unit_history.reset(provinces);
    }

    /// Places the raid units again, as a playing window does every frame by
    /// its playback fraction ([`crate::map::unit_models::PreparedModels::raid_units`]).
    pub fn set_raid_units(&mut self, raids: Vec<(usize, crate::map::models::ModelInstance)>) {
        self.unit_history.set_raids(raids);
    }

    /// Places the model a free camera mode's user steers, or takes it away.
    /// It draws lit and casts shadows wherever it stands, whatever the unit
    /// layer's cutoffs and display switches, and plays its entity's states on
    /// the render clock. A new label builds its pass afresh.
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
        player::Player::place(&mut self.player, device, queue, model)
    }

    /// The player's instances the last frame drew, for reporting.
    pub fn player_drawn(&self) -> u32 {
        self.player.as_ref().map_or(0, |player| player.pass.drawn())
    }

    /// What the player's entity keyed `(mesh, seed, identity)` has played, as
    /// the last frame left it
    /// ([`crate::map::models::ModelPass::root_play_position`]); `None` without
    /// a player.
    pub fn player_play_position(&self, key: (usize, u32, u32)) -> Option<Option<(f32, f32)>> {
        self.player.as_ref()?.pass.root_play_position(key)
    }

    // close zoom
    /// Keeps aircraft assets on the GPU and replaces the scenario plan each hour.
    pub fn set_aircraft(
        &mut self,
        assets: Arc<crate::map::aircraft::AircraftAssets>,
        mut plan: crate::map::aircraft::AircraftPlan,
        continuity: Continuity,
    ) -> io::Result<Option<crate::map::aircraft::AircraftPlan>> {
        // Hand back the old plan so its flights can be freed off the frame path.
        if let Some((pass, _, current)) = &mut self.aircraft {
            plan.carry_presentation(current, continuity, &|mesh, seed| {
                assets.launch_lifetime(mesh, seed)
            })?;
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

    pub fn set_trains(
        &mut self,
        assets: Arc<crate::map::trains::TrainAssets>,
        mut plan: crate::map::trains::TrainPlan,
        continuity: Continuity,
    ) -> io::Result<Option<crate::map::trains::TrainPlan>> {
        // The replaced plan holds every route's sampled curve; it is handed
        // back so the caller frees it off the frame path.
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

    // close zoom
    /// Builds the bridge pass once, then uploads this hour's placements
    /// ([`Bridges::field`] of the hour's rail mesh).
    pub fn set_bridges(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bridges: &Bridges,
        field: ModelField,
    ) -> io::Result<()> {
        if self.bridges.is_none() {
            self.bridges = Some(ModelPass::new(&bridges.library, "railway bridges"));
        }
        if let Some(pass) = &mut self.bridges {
            pass.upload(device, queue, &bridges.library, field)?;
        }
        Ok(())
    }

    /// Builds the battle-plan pass and uploads this hour's ribbons.
    ///
    /// The arrow types and their pages are static content, so the pass is built
    /// once; the ribbons come from `ArmyOrders` and are rewritten whenever the
    /// hour or the perspective changes.
    pub fn set_orders(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &OrderAssets,
        field: &OrderField,
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
        if let Some(pass) = &mut self.orders {
            pass.upload(device, field)?;
            self.order_labels = std::array::from_fn(|_| field.labels.clone());
        }
        Ok(())
    }

    // close zoom
    /// Prepares a bank for a direct render, then uses the same adoption as a built hour.
    pub fn set_railways(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RailAssets,
        mesh: &RailMesh,
    ) -> io::Result<()> {
        let geometry = crate::map::railways::PreparedRailGeometry::for_install(device, mesh, None)?;
        self.set_prepared_railways(device, queue, assets, geometry)?;
        Ok(())
    }

    /// Adopts worker-uploaded railway strips while retaining this pass's
    /// textures, shaders and all scene presentation clocks.
    pub fn set_prepared_railways(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RailAssets,
        geometry: Arc<crate::map::railways::PreparedRailGeometry>,
    ) -> io::Result<Arc<crate::map::railways::PreparedRailGeometry>> {
        if self.railways.is_none() {
            self.railways = Some(RailPass::new(
                device,
                self.graphics.samples,
                queue,
                SCENE_FORMAT,
                assets,
            )?);
        }
        Ok(self
            .railways
            .as_mut()
            .expect("created above")
            .adopt_geometry(geometry))
    }

    // close zoom
    /// Applies earlier camera views to the building history, as a session that
    /// visited them before this frame would have, at a zero frame time.
    pub fn visit_map_buildings(&mut self, cameras: &[Camera], size: [u32; 2]) {
        for camera in cameras {
            self.building_history.visit(*camera, size);
        }
    }

    /// Installs the hour's building targets in this scene's history; the
    /// camera applies them when the scene renders. `passed` holds the hours
    /// playback skipped since the hour on screen, oldest first; `None` is a
    /// seek, which starts the history cold.
    pub fn set_map_buildings(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &MapBuildingAssets,
        prepared: map_display_data::buildings::Buildings,
        passed: Option<&[&map_display_data::buildings::Buildings]>,
    ) -> io::Result<()> {
        self.warm_map_buildings(device, queue, assets)?;
        for hour in passed.unwrap_or_default() {
            let installed = self.building_history.install_display(hour, assets)?;
            self.building_history.advance(&installed);
            if let Some(presentation) = &mut self.city_presentation {
                presentation.advance_shared(&installed.city_damage);
            }
        }
        self.set_city_damage(&prepared, passed.is_none());
        let installed = self.building_history.install_display(&prepared, assets)?;
        if self.building_history.set(installed, passed.is_some())
            && let Some(pass) = &mut self.map_buildings
        {
            pass.reset_animation();
        }
        Ok(())
    }

    // close zoom
    /// Builds the city-sprawl pass. Called once per document: the sprawl is
    /// generated from static content and does not change with the hour.
    pub fn set_sprawl(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sprawl: &Sprawl,
    ) -> io::Result<()> {
        self.sprawl = Some(ModelPass::new(&sprawl.library, "city sprawl"));
        self.city_presentation = Some(sprawl.presentation());
        if let Some(pass) = &mut self.sprawl {
            let field = sprawl.field.clone();
            pass.upload(device, queue, &sprawl.library, field)?;
        }
        Ok(())
    }

    pub fn set_city_damage(
        &mut self,
        prepared: &map_display_data::buildings::Buildings,
        reset: bool,
    ) {
        if let Some(presentation) = &mut self.city_presentation {
            if reset {
                presentation.reset();
            }
            presentation.set_shared_damage(&Arc::new(crate::map::sprawl::Damage::from_display(
                &prepared.city_damage,
            )));
        }
        if reset && let Some(pass) = &mut self.sprawl {
            pass.reset_animation();
        }
    }

    // rivers
    /// Builds the river pass and uploads the traced ribbons.
    ///
    /// Called once per document: `map/rivers.bmp` and `map/heightmap.bmp` are both static content,
    /// so nothing about a river changes with the hour.
    pub fn set_rivers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RiverAssets,
        mesh: &RiverMesh,
    ) -> io::Result<()> {
        let mut pass = RiverPass::new(device, self.graphics.samples, queue, SCENE_FORMAT, assets)?;
        pass.upload(device, mesh)?;
        self.rivers = Some(pass);
        Ok(())
    }

    /// Adopts a builder-owned snow grid and its post factors. The graphics
    /// clock still decides when pending bytes replace the currently shown planes.
    pub fn set_snow_mud(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snow: &Arc<crate::map::weather::SnowMud>,
        continuity: Continuity,
    ) -> io::Result<()> {
        if self.snow_presentation.set_shared(snow, continuity)? {
            Self::upload_snow(
                device,
                queue,
                &mut self.snow_mud,
                &mut self.snow_presentation,
            )?;
        }
        Self::update_hatch(
            device,
            queue,
            &mut self.snow_unknown,
            &mut self.hatch_coverage,
            &mut self.snow_presentation,
        )?;
        if let Some(factors) = &snow.post_factors {
            self.post.set_snow_factors(factors);
        } else {
            self.post.set_snow(snow.province_snow.try_as_slice()?)?;
        }
        Ok(())
    }

    /// Rewrites the hatched cells' mask and where they lie when the presentation changed them.
    fn update_hatch(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mask: &mut Bound,
        coverage: &mut Option<crate::map::weather::HatchCoverage>,
        presentation: &mut crate::map::weather::Presentation,
    ) -> io::Result<()> {
        if !presentation.take_unknown_upload() {
            return Ok(());
        }
        let size = presentation.size;
        *coverage = crate::map::weather::HatchCoverage::new(&presentation.unknown, size);
        if coverage.is_none() {
            return Ok(());
        }
        if [mask.texture.width(), mask.texture.height()] == size {
            write_data(queue, &mask.texture, size, 1, &presentation.unknown);
        } else {
            *mask = data_texture(
                device,
                queue,
                "unknown snow/mud cells",
                size,
                wgpu::TextureFormat::R8Unorm,
                &presentation.unknown,
            )?;
        }
        Ok(())
    }

    fn upload_snow(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snow_mud: &mut Bound,
        presentation: &mut crate::map::weather::Presentation,
    ) -> io::Result<()> {
        let size = presentation.size;
        let ranges = presentation.uploads();
        if [snow_mud.texture.width(), snow_mud.texture.height()] != size {
            replace(
                device,
                queue,
                "SnowMudData",
                snow_mud,
                wgpu::TextureFormat::Rgba8Unorm,
                size,
                &presentation.texels,
            )?;
        } else {
            for range in ranges {
                write_range(
                    queue,
                    &snow_mud.texture,
                    size[0],
                    range.start / 4,
                    &presentation.texels[range.clone()],
                );
            }
        }
        presentation.clear_uploads();
        Ok(())
    }

    /// Replaces the hour's `IntelMap` and says whether this frame draws fog.
    ///
    /// `texels` is `viewer_foundation::map::textures::intel_map`'s grid, one byte a texel, and
    /// `fog` is whether it came from a recorded visibility rather than being
    /// the all-clear stand-in. The two travel together because `FOWFadeFactor`
    /// gates on the second and the shaders read the first.
    ///
    /// The texture is never reallocated: its size is fixed by the province
    /// image, so this is a write into the one the scene already built and no
    /// bind group has to be rebuilt.
    pub fn set_fog_of_war(
        &mut self,
        queue: &wgpu::Queue,
        texels: &map_display_data::Paged<u8>,
        fog: bool,
    ) -> io::Result<()> {
        let size = self.intel_map.texture.size();
        let expected = (size.width * size.height) as usize;
        if texels.len() != expected {
            return Err(invalid(format!(
                "the intel map needs {expected} bytes for {}x{}, got {}",
                size.width,
                size.height,
                texels.len()
            )));
        }
        if self
            .intel_installed
            .as_ref()
            .is_some_and(|old| texels.ptr_eq(old))
        {
            self.fog = fog;
            return Ok(());
        }
        let pages = self.intel_installed.as_ref().map_or_else(
            || (0..texels.pages().len()).collect(),
            |old| texels.changed_pages(old),
        );
        for page in pages {
            let alpha = texels.page(page);
            let bytes: Vec<_> = alpha.iter().flat_map(|&alpha| [0, 0, 0, alpha]).collect();
            write_range(
                queue,
                &self.intel_map.texture,
                size.width,
                page * map_display_data::PAGE_ITEMS,
                &bytes,
            );
        }
        self.intel_installed = Some(texels.clone());
        self.fog = fog;
        Ok(())
    }

    /// What the view's map mode shows: the fog of war only where the mode draws it, and the
    /// province borders only where the mode keeps them.
    pub fn set_visibility(
        &mut self,
        visibility: viewer_foundation::map::mode_visibility::Visibility,
    ) {
        self.visibility = visibility;
    }

    /// Compile, upload and fill static content on the loader, shared by all compatible views, and
    /// allocate this view's first fill state. Nothing is bound until the view's own first fill.
    pub fn warm_political(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        map: &installed_content_core::map::MapData,
        strategic_regions: &BTreeMap<u32, installed_content_regions::regions::StrategicRegion>,
        field: &viewer_foundation::map::gradient_math::FieldDefines,
        cache: &crate::map::political_gpu::Cache,
    ) -> io::Result<()> {
        if self.political.is_some() {
            return Ok(());
        }
        let strategic = crate::map::political::strategic_regions_of(map, strategic_regions)?;
        let base = cache.warm(
            device,
            queue,
            crate::map::political_gpu::Inputs {
                map: ProvinceMap {
                    width: map.width,
                    height: map.height,
                    ids: &map.province_ids,
                },
                strategic: &strategic,
                field,
            },
        )?;
        let colors = super::political_gpu::ProvinceColors::new(device, &base)?;
        self.secondary = Bound {
            texture: colors.bound.texture.clone(),
            view: colors.bound.view.clone(),
        };
        self.province_colors = Some(colors);
        self.political = Some(crate::map::political_gpu::View::new(base, device, queue)?);
        Ok(())
    }

    /// Current secondary province colours. Changed tiles fill before terrain.
    pub fn set_air_assignment(
        &mut self,
        queue: &wgpu::Queue,
        field: &super::air_assignment::Field,
    ) -> io::Result<()> {
        if let Some(colors) = &mut self.province_colors {
            colors.update(queue, &field.provinces)?;
        }
        Ok(())
    }

    /// Apply this view's local Navy selection without copying province or
    /// strategic-region membership from the prepared hour.
    pub fn set_strategic_styles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        styles: &BTreeMap<u32, super::map_mode::Style>,
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
            let bound = |texture: wgpu::Texture| Bound {
                view: texture.create_view(&Default::default()),
                texture,
            };
            self.gradient1 = bound(first);
            self.gradient2 = bound(second);
        }
        Ok(())
    }

    /// Show one hour's political tables; the fill is computed with the next render.
    pub fn set_political(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        regions: &Arc<Regions>,
    ) -> io::Result<()> {
        let Some(political) = &mut self.political else {
            return Err(invalid("PoliticalComputeNotWarmed"));
        };
        self.mode_channels = regions
            .mode
            .as_ref()
            .map_or([None; 2], |draw| draw.channel_constants());
        if !political.show(device, queue, regions)? {
            return Ok(());
        }
        let [first, second] = political.textures().cloned().expect("shown above");
        let bound = |texture: wgpu::Texture| Bound {
            view: texture.create_view(&Default::default()),
            texture,
        };
        self.gradient1 = bound(first);
        self.gradient2 = bound(second);
        Ok(())
    }

    // borders
    /// Builds the border pass and uploads the traced meshes and strait ribbons.
    ///
    /// Called once per document: the geometry depends only on content, and only the class of a land
    /// edge changes with the hour ([`Scene::set_land_classes`]).
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
        self.borders = Some(pass);
        self.demilitarized = Some(DemilitarizedPass::new(
            device,
            self.graphics.samples,
            queue,
            SCENE_FORMAT,
            assets,
        )?);
        self.faction_highlight_regions.clear();
        Ok(())
    }

    // borders
    /// Re-buckets every border edge for one hour's political regions and demilitarised zone.
    ///
    /// The two passes are set together because they partition the same edges: an edge the zone
    /// splits leaves the ordinary buckets and enters the secondary bank (`crate::map::demilitarized`).
    pub fn set_classes(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
    ) -> io::Result<()> {
        if let Some(pass) = &mut self.borders {
            pass.set_classes(device, meshes, regions, zone)?;
        }
        if let Some(pass) = &mut self.demilitarized {
            pass.set_zone(device, meshes, regions, zone)?;
            pass.clear_hover();
        }
        self.faction_highlight_regions.clear();
        Ok(())
    }

    /// [`Scene::set_classes`] with the edges already bucketed on a builder
    /// thread ([`crate::map::border_pass::ClassPlan`], [`crate::map::demilitarized::ZonePlan`]).
    pub fn set_border_plans(
        &mut self,
        device: &wgpu::Device,
        classes: &crate::map::border_pass::ClassPlan,
        zone: &crate::map::demilitarized::ZonePlan,
    ) -> io::Result<()> {
        if let Some(pass) = &mut self.borders {
            let _border_span = viewer_foundation::profiling::span("ordinary_border_install");
            pass.set_plan(device, classes)?;
        }
        if let Some(pass) = &mut self.demilitarized {
            let _zone_span = viewer_foundation::profiling::span("dmz_install");
            pass.set_plan(device, zone)?;
            pass.clear_hover();
        }
        self.faction_highlight_regions.clear();
        Ok(())
    }

    pub fn faction_highlight_regions(&self) -> &[u32] {
        &self.faction_highlight_regions
    }

    /// Uploads the worker's prepared faction highlight without rebuilding edges.
    pub fn set_faction_highlight(
        &mut self,
        device: &wgpu::Device,
        plan: &FactionHighlight,
        strategic_regions: &[u32],
        color: [f32; 4],
    ) -> io::Result<()> {
        if let Some(pass) = &mut self.borders {
            pass.set_plan(device, &plan.classes)?;
        }
        if let Some(pass) = &mut self.demilitarized {
            pass.set_plan(device, &plan.zone)?;
            pass.set_hover(device, &plan.hover, color)?;
        }
        self.faction_highlight_regions.clear();
        self.faction_highlight_regions
            .extend_from_slice(strategic_regions);
        Ok(())
    }

    /// Province boundaries the demilitarised-zone pass drew at the last hour set.
    pub fn demilitarized_edges(&self) -> usize {
        self.demilitarized
            .as_ref()
            .map_or(0, DemilitarizedPass::edge_count)
    }

    /// Draws the sea-bed refraction target, terrain, the water plane and
    /// the border meshes, and after them the two overlay layers of [`Overlay`].
    pub fn samples(&self) -> u32 {
        self.graphics.samples
    }

    pub fn set_graphics(&mut self, graphics: dashboard_core::protocol::MapGraphics) {
        // Pipeline sample counts only change when the scene is rebuilt.
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
        // Lent to the frame and back on every way out; a frame that fails drops its marks with
        // its encoder.
        let mut timer = self.pass_timer.take();
        let rendered = self.render_frame(device, queue, request, &mut timer);
        if rendered.is_err()
            && let Some(timer) = &mut timer
        {
            timer.abandon();
        }
        self.pass_timer = timer;
        // A frame that failed before its walk leaves the schedule as if it was never cut.
        self.restore_province_frame();
        rendered
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
            // colour
            postfx,
            no_water,
            // shadows
            no_shadows,
            overlay,
            // trees
            no_trees,
        } = request;
        let no_trees = no_trees || !self.graphics.trees;
        let no_shadows = no_shadows || !self.graphics.shadows;
        camera.validate()?;
        let weather_delta = self.snow_render_delta.advance(animation_time);
        if self
            .snow_presentation
            .advance(animation_time, weather_delta)?
        {
            Self::upload_snow(
                device,
                queue,
                &mut self.snow_mud,
                &mut self.snow_presentation,
            )?;
        }
        Self::update_hatch(
            device,
            queue,
            &mut self.snow_unknown,
            &mut self.hatch_coverage,
            &mut self.snow_presentation,
        )?;
        let snow_mud_fade = self.snow_presentation.fade();
        let mut passes = passes::Frame::new(&mut self.pass_diagnostics);
        self.arena.reset();
        let arena = &self.arena;
        let clock = profiling().then(Instant::now);
        self.detail_laps.start(clock);
        let rail_alpha = self.rail_fade.update(&camera, animation_time);
        let rails_shown = self.rail_fade.trains_visible();
        let trains_visible = rails_shown && self.graphics.supply_trains();
        let bridges_visible = rails_shown && self.graphics.railway_bridges();
        validate_size(size, device.limits().max_texture_dimension_2d)?;
        let world_size = world_size(size, self.graphics.full_resolution);
        validate_size(world_size, device.limits().max_texture_dimension_2d)?;
        // Do not clamp a zero downsample size. Refuse a viewport
        // too small for that chain instead of creating a different native path.
        validate_size(
            world_size.map(|side| side / 2),
            device.limits().max_texture_dimension_2d,
        )
        .map_err(|error| invalid(format!("Map post seed: {error}")))?;
        let generation_changed = self
            .targets
            .as_ref()
            .is_none_or(|t| t.size() != size || t.world_size() != world_size);
        if generation_changed {
            self.targets = Some(Targets::new(
                device,
                size,
                world_size,
                self.graphics.samples,
            ));
            self.generation += 1;
            #[cfg(any(test, feature = "test-support"))]
            faults.after(failure_tests::Point::Targets)?;
        }
        let samples = self.targets.as_ref().map_or(1, Targets::samples);
        if self.hatch_coverage.is_some()
            && self
                .unknown_hatch
                .as_ref()
                .is_none_or(|(built, _)| *built != samples)
        {
            self.unknown_hatch = Some((
                samples,
                crate::map::weather::UnknownHatch::new(device, samples),
            ));
        }
        // Receiver and blur stay backbuffer-sized, even when the
        // scene is half size. Camera projection/culling and GUI also retain size.
        self.shadow.resize(device, size);
        self.cover = crate::map::terrain::cover_for(camera, size);
        self.grid = self.cover.grid();
        // trees: the same rectangle the wrap offsets come from, which the tree pass culls its
        // buckets against.
        let visible = camera.visible_ground(size, MARGIN);
        self.detail_laps.lap("targets_shadow_resize");
        // The walk consumes the host's cut even when display switches hide its actors.
        let province_frame = self.province_frame.take();
        let struck = step::Presentation {
            graphics: &self.graphics,
            aircraft: self.aircraft.as_mut(),
            trains: self.trains.as_mut(),
            sprawl: self.sprawl.as_mut(),
            city_presentation: self.city_presentation.as_mut(),
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
            last_animation_time: self.last_animation_time,
            workers: &self.workers,
            province_frame: province_frame.as_ref().map(|(frame, _)| frame),
        }
        .step(camera, size, animation_time, rails_shown);
        if let Some(player) = &mut self.player {
            player.pass.animate(animation_time, None, &[0.0]);
        }
        self.detail_laps.lap("presentation_step");
        // Only a layer that draws needs its meshes uploaded.
        if let Some((pass, assets, _)) = &mut self.aircraft
            && self.graphics.aircraft()
        {
            pass.make_field_resident(device, queue, &assets.library())?;
        }
        if let Some((pass, assets, _)) = &mut self.trains
            && trains_visible
        {
            pass.make_field_resident(device, queue, &assets.library)?;
        }
        self.detail_laps.lap("aircraft_trains_resident");
        let targets = self.targets.as_ref().expect("created above");
        // Every event reaches the simulations; the display only marks what is not submitted.
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
        // Map-frame events lead; the rest are actors' own.
        let frame_events = if animation_time > 0.0 {
            self.native_map_frame
                .as_ref()
                .map_or(0, |pass| pass.events.len())
        } else {
            0
        };
        self.sound_events.clear();
        crate::map::sound_events::collect(
            &events[..frame_events],
            &mut self.sound_names,
            &mut self.sound_events,
            |event| struck.contains(&event.key.player()),
        );
        crate::map::sound_events::collect(
            &events[frame_events..],
            &mut self.sound_names,
            &mut self.sound_events,
            crate::map::entity_animation::Event::in_combat,
        );
        self.sound_refusals.clear();
        if let Some(pass) = &self.native_map_frame {
            self.sound_refusals.extend(
                pass.default_state_refusals
                    .iter()
                    .map(|(why, count)| (why.clone(), *count)),
            );
        }
        if self.graphics.weather
            && let Some(weather) = &mut self.weather_effects
        {
            weather.sound_events(
                camera,
                size,
                animation_time,
                &mut self.sound_names,
                &mut self.sound_events,
            );
        }
        static PROFILE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *PROFILE.get_or_init(|| std::env::var_os("BATTLE_PLAN_ANIMATION_PROFILE").or_else(|| std::env::var_os("HOI4_ANIMATION_PROFILE")).is_some()) {
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
        let lights_profile = viewer_foundation::profiling::thread_span("lights");
        let lights = match self.weather_effects.as_mut() {
            Some(pass) => pass.set_events_with_workers(events, &self.workers)?,
            None => {
                // Without particle runtimes no kept particle lives.
                for kept in events.iter().filter_map(|event| event.kept.as_ref()) {
                    kept.finish();
                }
                Vec::new()
            }
        };
        self.point_light_tiles.rebuild(camera, &lights);
        self.point_light_maps.write(queue, &self.point_light_tiles);
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
        // Prepare the light even when only fog receivers are admitted.
        let [shadow_casters, shadow_receivers] = crate::map::shadow::stages(&camera, !no_shadows);
        let shadow_prepared = self.shadow.prepare(&globals.frame, camera, size);
        let shadow_matrix = shadow_prepared.map(|prepared| prepared.texture_matrix);
        globals.shadow_matrix = shadow_matrix.unwrap_or([[0.0; 4]; 4]);
        // The world-wrap copies the visible rectangle reaches. Every layer cut
        // at the x seam takes the same list: the rivers and border meshes
        // below, and the name layer, which needs it before the encoder starts.
        // A free eye's terrain reaches the horizon, so its world layers take
        // every copy its far plane reaches.
        let offsets = match camera.free() {
            None => wrap_offsets(visible, camera.map.x()),
            Some(free) => crate::map::border_pass::free_wrap_offsets(
                camera.reach(
                    size,
                    free.lens.far,
                    [0.0, viewer_foundation::map::camera::frustum::TERRAIN_TOP],
                ),
                camera.map.x(),
            ),
        };
        let native_frames = offsets
            .iter()
            .map(|offset| {
                // Upload each world-wrap camera's scene block;
                // the native vertex stays at its original world position.
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
        let native_mesh_inputs = crate::map::models::NativeInputs {
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
        // Rebuild glyph buffers when placements change, then admit this frame's names. The near
        // names cross-fade with the far ones.
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
        // Plan labels have no camera cutoff: each label's own alpha is its
        // arrow colour's, so the layer factor is 1.
        if let Some(orders) = &mut self.orders {
            let graphics = self.graphics;
            // Trade-route lines share the arrow manager; the map mode shows them.
            let trade_routes = self.visibility.trade_routes;
            orders.prepare(camera, size, &offsets, |draw| {
                if draw.air_mode {
                    self.visibility.priorities
                        == viewer_foundation::map::mode_visibility::PriorityTable::StrategicAir
                } else {
                    match draw.kind {
                        dashboard_core::protocol::PlanKind::TradeRoute => trade_routes,
                        kind => graphics.plan(kind),
                    }
                }
            });
            for ((pass, labels), mode) in self
                .order_label_pass
                .iter_mut()
                .zip(&self.order_labels)
                .zip([ArrowMode::Main, ArrowMode::Seethrough, ArrowMode::Late])
            {
                pass.upload(device, labels, camera, size, 1.0, &offsets);
                pass.select_arrows(&orders.labels(mode));
            }
        }
        self.detail_laps.lap("orders_upload");
        self.sprite_pass.sync(device, queue);
        if let Some(radar) = &mut self.radar {
            radar.advance();
        }
        if let Some(ranges) = &mut self.air_ranges {
            ranges.advance();
        }
        if let Some(transfers) = &mut self.air_transfers {
            transfers.advance();
        }
        self.detail_laps.lap("sprite_sync");
        let sprite_camera = viewer_foundation::map::sprites::CameraFrame::new(camera, size);
        self.sprite_pass
            .set_camera_moving(self.sprite_camera.replace(sprite_camera) != Some(sprite_camera));
        let sprite_cpu = viewer_foundation::profiling::thread_span("sprite_upload");
        self.sprite_pass.upload(
            device,
            queue,
            arena,
            overlay.sprites.unwrap_or(&self.empty_batch),
            size,
            overlay.ui_scale,
        );
        sprite_cpu.finish();
        self.detail_laps.lap("sprite_upload");
        let prepared = clock.map(|clock| clock.elapsed());
        self.pass_cpu_laps.start(clock.map(|_| Instant::now()));
        // Both callbacks execute only on the caller. The shared recorder also lets
        // CPU-only boundaries leave the existing GPU query stream unchanged.
        let pass_cpu_laps = RefCell::new(&mut self.pass_cpu_laps);
        let cpu_mark = |label: &'static str| {
            if clock.is_some() {
                pass_cpu_laps.borrow_mut().lap_sum(label);
            }
        };
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map") });
        let mut command_buffers = Vec::new();
        let mut mark = |encoder: &mut wgpu::CommandEncoder, label: &'static str| {
            cpu_mark(if label == "model_layers" {
                "model_recycle"
            } else {
                label
            });
            if let Some(timer) = timer.as_mut() {
                timer.mark(encoder, label);
            }
        };
        self.compiled_terrain.validate_grid(self.grid)?;
        mark(&mut encoder, "start");
        // Recorded fills stay pending until the encoder is submitted: one this frame drops on a
        // failure is recorded again by the next.
        let political = self
            .political
            .as_ref()
            .and_then(|political| political.encode(&mut encoder));
        let stripes = self
            .province_colors
            .as_mut()
            .is_some_and(|colors| colors.encode(&mut encoder));
        mark(&mut encoder, "political_compute");
        // Invoke underwater in pre-frame, before the scene's
        // shadow-screen clear and receiver passes. It reads the previous
        // shadow frame, including across camera and shadow-cutoff changes.
        {
            let null = &self.terrain_null.view;
            let prepared = passes.run(passes::Pass::Underwater, || {
                #[cfg(any(test, feature = "test-support"))]
                faults.preflight(passes::Pass::Underwater)?;
                self.compiled_terrain.prepare(
                    arena,
                    crate::map::terrain_shader::Mode::Underwater,
                    crate::map::terrain_shader::Inputs {
                        retained: None,
                        frames: &native_frame_bytes,
                        offsets: &offsets,
                        cover: &self.cover,
                        matrix: [[0.; 4]; 4],
                        light_grid: globals.light_grid,
                        textures: [
                            &self.underwater.view,
                            &self.height_normal.view,
                            &self.colormap_water.view,
                            null,
                            null,
                            null,
                            null,
                            null,
                            null,
                            null,
                            &self.point_light_maps.indices.view,
                            &self.point_light_maps.data.view,
                            self.shadow.refraction_texture(),
                            null,
                            null,
                            null,
                        ],
                    },
                )
            });
            if let Some(prepared) = prepared {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("compiled underwater"),
                    color_attachments: &[Some(targets.refraction_attachment())],
                    depth_stencil_attachment: Some(targets.refraction_depth_attachment()),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                self.compiled_terrain.draw(&mut pass, &prepared);
            }
        }
        mark(&mut encoder, "terrain_refraction");
        // Whole-frame paths validate materials/wraps/buckets/packets and shadow/post stages after
        // earlier draws. An errored encoder is never submitted. Shadow casters precede screen-space
        // receivers and both blurs before lit terrain samples them. Clear depth even without
        // casters.
        drop(
            self.shadow
                .depth_pass(&mut encoder, wgpu::LoadOp::Clear(1.0)),
        );
        match (shadow_prepared, visible) {
            (Some(shadow_prepared), Some(visible)) => {
                if shadow_casters {
                    let casters = crate::map::shadow::receiver_rect(camera, size, Some(visible))
                        .map(|receivers| {
                            crate::map::shadow::caster_rect(
                                receivers,
                                crate::map::shadow::light_direction(camera, &globals.frame),
                            )
                        });
                    let caster_frame = |alpha| ModelFrame {
                        camera,
                        offsets: &offsets,
                        visible: casters,
                        alpha,
                    };
                    // Fill normal map scene constants, then replaces VP only.
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
                    let native_caster_inputs = crate::map::models::NativeInputs {
                        samples: 1,
                        frames: &caster_bytes,
                        ..native_mesh_inputs
                    };
                    if let Some(mut pass) = self.shadow.depth_pass(&mut encoder, wgpu::LoadOp::Load)
                    {
                        if let Some(frame) = &self.native_map_frame {
                            frame.draw_casters(
                                arena,
                                &mut pass,
                                ModelFrame {
                                    visible: None,
                                    ..caster_frame(1.)
                                },
                                native_caster_inputs,
                            )?;
                        }
                        if let Some(sprawl) = &self.sprawl
                            && self.graphics.city_sprawl
                            && draws_city_sprawl(&camera)
                        {
                            sprawl.draw_casters(
                                arena,
                                &mut pass,
                                caster_frame(1.0),
                                native_caster_inputs,
                            )?;
                        }
                        if let Some(buildings) = &self.map_buildings
                            && self.graphics.buildings
                        {
                            buildings.draw_casters(
                                arena,
                                &mut pass,
                                caster_frame(1.0),
                                native_caster_inputs,
                            )?;
                        }
                        if let Some(units) = &self.unit_models
                            && self.graphics.unit_figures()
                        {
                            units.draw_casters(
                                arena,
                                &mut pass,
                                caster_frame(1.0),
                                native_caster_inputs,
                            )?;
                        }
                        if let Some((aircraft, _, _)) = &self.aircraft
                            && self.graphics.aircraft()
                            && crate::map::aircraft::draws(&camera)
                        {
                            aircraft.draw_casters(
                                arena,
                                &mut pass,
                                caster_frame(1.0),
                                native_caster_inputs,
                            )?;
                        }
                        if let Some((trains, _, _)) = &self.trains
                            && trains_visible
                        {
                            trains.draw_casters(
                                arena,
                                &mut pass,
                                caster_frame(1.0),
                                native_caster_inputs,
                            )?;
                        }
                        if let Some(bridges) = &self.bridges
                            && bridges_visible
                        {
                            bridges.draw_casters(
                                arena,
                                &mut pass,
                                caster_frame(1.0),
                                native_caster_inputs,
                            )?;
                        }
                        // The player casts as every caster does: on the canonical and each seam
                        // view, with the light matrix translated for each, inside the light.
                        if let Some(player) = &self.player {
                            player.pass.draw_casters(
                                arena,
                                &mut pass,
                                caster_frame(1.0),
                                native_caster_inputs,
                            )?;
                        }
                    }
                    mark(&mut encoder, "shadow_mesh_casters");
                    if let Some(mut pass) = self.shadow.depth_pass(&mut encoder, wgpu::LoadOp::Load)
                        && let Some(trees) = &self.trees
                        && !no_trees
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
                    // Plans cast shadows exactly while they draw.
                    PlanSubmissions::new(
                        self.orders.as_ref(),
                        &self.graphics,
                        &overlay,
                        self.visibility.trade_routes || self.visibility.priorities == viewer_foundation::map::mode_visibility::PriorityTable::StrategicAir,
                    )
                    .cast_shadows(|plans| {
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
                            let scene = crate::map::arrow_shader::shadow_constants(
                                native_frame_bytes[view],
                                matrix,
                            );
                            plans.draw_shadow(
                                arena,
                                &mut pass,
                                crate::map::order_pass::NativeInputs {
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
                    mark(&mut encoder, "shadow_arrow_casters");
                }
                // Clear the screen target independently of both gates.
                self.shadow.clear_stand_in(&mut encoder);
                if shadow_receivers {
                    if let Some(mut pass) = self
                        .shadow
                        .prepass_pass(&mut encoder, wgpu::LoadOp::Clear(1.0))
                        && let Some(trees) = &self.trees
                        && !no_trees
                    {
                        trees.draw_unlit(
                            arena,
                            &mut pass,
                            TreeFrame {
                                camera,
                                offsets: &offsets,
                                planes: side_planes(camera, size),
                            },
                            crate::map::trees::ReceiverInputs {
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
                        crate::map::terrain_shader::Mode::Receiver,
                        crate::map::terrain_shader::Inputs {
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
                    if let Some(mut pass) =
                        self.shadow.prepass_pass(&mut encoder, wgpu::LoadOp::Load)
                    {
                        // Submit tree receivers before terrain.
                        self.compiled_terrain.draw(&mut pass, &terrain_receiver);
                    }
                    mark(&mut encoder, "shadow_receivers");
                    self.shadow.finish_receivers();
                }
                // Blur only when the native caster gate is enabled.
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
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene clear"),
            color_attachments: &[Some(
                targets.scene_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
            )],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Clear(1.))),
            ..Default::default()
        }));
        let border_target = || crate::map::border_shader::Target {
            color: targets.scene_attachment(wgpu::LoadOp::Load),
            depth: targets.depth_attachment(wgpu::LoadOp::Load),
        };
        let retained_scene = RefCell::new(crate::map::shader_scene::State::default());
        macro_rules! draw_world {
            ($pass:ident => $draw:expr) => {
                passes.world(passes::Pass::$pass, &retained_scene, || {
                    #[cfg(any(test, feature = "test-support"))]
                    faults.preflight(passes::Pass::$pass)?;
                    $draw
                })
            };
        }
        // Unknown snow and mud lie on the surface just drawn, under everything drawn after it.
        let (hatch_coverage, unknown_hatch) = (&self.hatch_coverage, &self.unknown_hatch);
        let hatch_heights = &self.hatch_heights.as_ref().unwrap_or(&self.heightmap).view;
        let (snow_unknown, hatch_grid) = (&self.snow_unknown.view, self.snow_presentation.size);
        let (cover, terrain_map) = (&self.cover, self.compiled_terrain.map());
        let draw_hatch = |encoder: &mut wgpu::CommandEncoder,
                          view: usize,
                          surface: crate::map::weather::Surface|
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
            // The blocks the terrain drew in this wrap, so the hatch stands where the terrain does.
            let drawn =
                crate::map::terrain_shader::drawn_blocks(cover, terrain_map, offsets[view])?;
            let frame = &native_frames[view];
            let [x, y, z, _] = frame.cam_look_at_dir_global_time;
            hatch.draw(
                device,
                encoder,
                targets,
                hatch_heights,
                snow_unknown,
                &coverage.instances(&drawn),
                &crate::map::weather::HatchParams::new(
                    frame.view_projection,
                    [x, y, z],
                    terrain_map,
                    hatch_grid,
                    surface,
                ),
            )
        };
        // Complete each admitted wrap before starting the next. Its
        // PS b2 light-grid upload is replaced by the land-border call in that wrap.
        for view in 0..offsets.len() {
            crate::map::shader_constants::WorldPixelConstants::LightGrid(globals.light_grid)
                .retain(&mut retained_scene.borrow_mut());
            let view_frames = &native_frame_bytes[view..view + 1];
            let view_offsets = &offsets[view..view + 1];
            let mode = crate::map::terrain_shader::Mode::Lit(self.graphics.high_gfx_shaders);
            {
                let prepared = draw_world!(Terrain => self.compiled_terrain.prepare(
                    arena,
                    mode,
                    crate::map::terrain_shader::Inputs {
                        retained: Some(&retained_scene),
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
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("compiled terrain"),
                        color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
                        depth_stencil_attachment: Some(
                            targets.depth_attachment(wgpu::LoadOp::Load),
                        ),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    self.compiled_terrain.draw(&mut pass, &prepared);
                }
            }
            mark(&mut encoder, "terrain");
            if hatch_coverage.is_some() {
                draw_world!(UnknownSnowMud => draw_hatch(
                    &mut encoder,
                    view,
                    crate::map::weather::Surface::Ground,
                ));
            }
            // Land borders, rivers, rails, trees, water and sea highlights share scene depth. The unlit shadow
            // pre-pass uses its own depth buffer.
            let border_inputs = || crate::map::border_shader::Inputs {
                retained: Some(&retained_scene),
                frames: view_frames,
                snow_mud: &self.snow_mud.view,
                intel: &self.intel_map.view,
                shadow: self.shadow.lit_texture(),
                time: map_time,
            };
            let mut pixel_b2 =
                crate::map::shader_constants::WorldPixelConstants::LightGrid(globals.light_grid);
            // borders
            if let Some(borders) = &self.borders
                && let Some(value) = draw_world!(LandBorders => borders.draw(
                    arena,
                    &mut encoder,
                    border_target(),
                    border_inputs(),
                    BorderFrame {
                        stage: BorderStage::Land,
                        camera_height: camera.height,
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
                    border_target(),
                    crate::map::river_shader::Inputs {
                        retained: Some(&retained_scene),
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
                    border_target(),
                    crate::map::rail_shader::Inputs {
                        retained: Some(&retained_scene),
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
                    crate::map::trees::LitInputs {
                        retained: Some(&retained_scene),
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
                    border_target(),
                    crate::map::water_shader::Inputs {
                        retained: Some(&retained_scene),
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
            // Water whose technique reads the snow/mud texture can show it, as the shipped ice does.
            if hatch_coverage.is_some()
                && !no_water
                && self.graphics.water
                && self.water.shows_snow_mud()
            {
                draw_world!(UnknownSnowMud => draw_hatch(
                    &mut encoder,
                    view,
                    crate::map::weather::Surface::Water,
                ));
            }
            mark(&mut encoder, "unknown_snow_mud");
            // borders
            if let Some(borders) = &self.borders {
                draw_world!(SeaBorders => borders.draw(
                    arena,
                    &mut encoder,
                    border_target(),
                    border_inputs(),
                    BorderFrame {
                        stage: BorderStage::Sea,
                        camera_height: camera.height,
                        fades: camera.defines.border_fades,
                        province_borders: self.visibility.province_borders,
                        visible: overlay.interface && self.graphics.sea_borders(),
                    },
                ));
            }
            mark(&mut encoder, "borders_sea");
            // Draw the secondary border bank in the final border batch.
            if let Some(demilitarized) = &self.demilitarized {
                let edges = if !overlay.interface {
                    crate::map::demilitarized::ZoneEdges::Hidden
                } else if self.graphics.demilitarized_zones() {
                    crate::map::demilitarized::ZoneEdges::Highlighted
                } else if self.graphics.land_borders() {
                    crate::map::demilitarized::ZoneEdges::Ordinary {
                        camera_height: camera.height,
                        fades: camera.defines.border_fades,
                        province_borders: self.visibility.province_borders,
                    }
                } else {
                    crate::map::demilitarized::ZoneEdges::Hidden
                };
                draw_world!(Demilitarized => demilitarized.draw(
                    arena,
                    &mut encoder,
                    border_target(),
                    border_inputs(),
                    edges,
                ));
            }
            mark(&mut encoder, "demilitarized");
            // Submit strait ribbons through their own compiled program and states.
            if let Some(borders) = &self.borders {
                draw_world!(Straits => borders.compiled_straits.draw(
                    arena,
                    &mut encoder,
                    crate::map::strait_shader::Target {
                        color: targets.scene_attachment(wgpu::LoadOp::Load),
                        depth: targets.depth_attachment(wgpu::LoadOp::Load),
                    },
                    view_frames,
                    [&self.snow_mud.view, &self.intel_map.view],
                    Some(&retained_scene),
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
                &overlay,
                self.visibility.trade_routes
                    || self.visibility.priorities
                        == viewer_foundation::map::mode_visibility::PriorityTable::StrategicAir,
            )
            .draw(|plans| {
                for view in 0..offsets.len() {
                    plans.draw(
                        arena,
                        encoder,
                        targets,
                        crate::map::order_pass::NativeInputs {
                            retained: Some(&retained_scene),
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
                    // Draw the collected text buffers after the ribbons,
                    // with the `arrowText` effect (`mapname`).
                    if !self.graphics.plan_labels() {
                        continue;
                    }
                    self.order_label_pass[label_index].draw_view(
                        arena,
                        &native_frame_bytes,
                        encoder,
                        targets,
                        Some(view),
                        Some(&retained_scene),
                    )?;
                }
                Ok(())
            })
        };
        //main arrows precede both entity queues.
        draw_orders(&mut encoder, ArrowMode::Main, 0)?;
        #[cfg(any(test, feature = "test-support"))]
        faults.after(failure_tests::Point::Arrows)?;
        mark(&mut encoder, "orders_main");
        let air = self.visibility.priorities
            == viewer_foundation::map::mode_visibility::PriorityTable::StrategicAir;
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
                    crate::map::order_pass::NativeInputs {
                        retained: Some(&retained_scene),
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
        // Presentation clocks and births advance once. All Alternate seam envelopes submit the same
        // immutable canonical particle packets. Entity envelopes retain footprint flags and carrier
        // order independently of the uniform terrain's raster list.
        let actor_views = views(camera, size);
        let actor_camera = actor_views[0];
        let actor_visible = actor_camera.visible_ground(size, MARGIN);
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
        // The player's own map copies, however high it stands above the band the map's objects
        // are admitted in, and each copy's scene constants.
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
        let particle_packets = self.weather_effects.as_mut().map(|weather| {
            weather.prepare_frame_with_workers(
                camera,
                size,
                &actor_views,
                animation_time,
                self.graphics.weather,
                &self.workers,
            )
        });
        cpu_mark("weather_packets");
        // Submit all regular materials before all alternate materials across layer boundaries. Preserve
        // enqueue order before the library's mesh/tile grouping.
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
                self.graphics.aircraft() && crate::map::aircraft::draws(&camera),
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
        let mut object = crate::map::models::NativeObjectConstants::default();
        object.retained.inherit(&retained_scene.borrow());
        for native_queue in [
            crate::map::models::NativeQueue::Regular,
            crate::map::models::NativeQueue::Alternate,
        ] {
            for view in 0..actor_views.len() {
                let wrapped = actor_views[view];
                if native_queue == crate::map::models::NativeQueue::Regular
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
                // Idle helpers register while native preparation retains the serial state chain.
                let workers = if mesh_layers.iter().any(|(model, enabled, _, _)| {
                    *enabled && model.is_some_and(ModelPass::has_placements)
                }) {
                    self.workers.threads()
                } else {
                    1
                };
                let (model_draws, buffers) =
                    self.workers
                        .with_admitted(workers, |admitted| -> io::Result<_> {
                            #[cfg(any(test, feature = "test-support"))]
                            faults.begin_native(admitted);
                            let mut model_draws = Vec::new();
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
                                        alpha,
                                    };
                                    let draws = model.prepare_native(
                                        arena,
                                        frame,
                                        crate::map::models::NativeInputs {
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
                            cpu_mark("model_prepare");
                            let buffers = ModelPass::encode_native(
                                &model_draws,
                                device,
                                targets,
                                admitted,
                                &mut encoder,
                            );
                            #[cfg(any(test, feature = "test-support"))]
                            faults.finish_native(buffers.len());
                            cpu_mark("model_encode");
                            Ok((model_draws, buffers))
                        })?;
                command_buffers.extend(buffers);
                for (model, draws) in model_draws {
                    model.recycle_native(draws);
                }
                mark(&mut encoder, "model_layers");
                if native_queue == crate::map::models::NativeQueue::Alternate {
                    // In each admitted view the native order is meshes, then
                    // animated world text, then particles.
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
                            crate::map::weather_effects::NativeInputs {
                                frame: actor_frame_bytes[view],
                                grid: native_mesh_inputs.grid,
                                textures: native_mesh_inputs.textures,
                                retained: &mut object.retained,
                            },
                            packets,
                        ) {
                            return Err(error);
                        }
                        //Particle b1 replaces PS's mesh allocation even when
                        //this view has no particles.
                        object.forget_pixel_snapshot();
                        #[cfg(any(test, feature = "test-support"))]
                        faults.after(failure_tests::Point::Weather)?;
                    }
                    mark(&mut encoder, "weather");
                }
            }
            // The player on each map copy its own bounds reach in view, not culled to the ground
            // the map's objects stand on.
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
                            alpha: 1.0,
                        },
                        crate::map::models::NativeInputs {
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
        //alternate entities/particles precede modes3 and1.
        draw_orders(&mut encoder, ArrowMode::Seethrough, 1)?;
        mark(&mut encoder, "orders_seethrough");
        draw_orders(&mut encoder, ArrowMode::Late, 2)?;
        mark(&mut encoder, "orders_late");
        // Draw world names into the floating-point scene after weather and before post restore. NamePass
        // uses an UNORM atlas and SrcAlpha/InvSrcAlpha blending.
        self.name_pass.validate_draw(&native_frame_bytes, None)?;
        self.near_name_pass
            .validate_draw(&native_frame_bytes, None)?;
        let names = overlay.country_names && self.graphics.country_names;
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
        // Draw sky once with the original camera, after map text.
        if self.graphics.sky {
            if self.sky.is_none() {
                self.sky = Some(
                    crate::map::sky_shader::Sky::new(
                        device,
                        &self.sky_effect,
                        self.graphics.samples,
                        SCENE_FORMAT,
                        DEPTH_FORMAT,
                    )
                    .map_err(|error| error.to_string()),
                );
            }
            passes.run(passes::Pass::Sky, || {
                #[cfg(any(test, feature = "test-support"))]
                faults.preflight(passes::Pass::Sky)?;
                viewer_foundation::map::gpu_contract::compiled(
                    self.sky.as_ref().expect("created above"),
                )?
                .draw(
                    arena,
                    &mut encoder,
                    border_target(),
                    bytemuck::bytes_of(&globals.frame),
                    &self.reflection.view,
                )
            });
        }
        mark(&mut encoder, "sky");
        // The post chain grades world-space names with the scene. Screen-space
        // VP text, counters and other icons are composited on the output.
        //
        // Either branch is what moves the float scene into the 8-bit output the
        // overlays, egui and the readback share, so one of them always runs.
        if postfx {
            let delta = self
                .last_animation_time
                .filter(|previous| animation_time > *previous)
                .map(|previous| animation_time - previous);
            let tau_delta = self.post.adaptation_tau(delta);
            let grading = self.post.grading(camera, day_night);
            let post = self.post.draw(
                arena,
                &mut encoder,
                targets,
                grading,
                tau_delta,
                generation_changed,
                &mut mark,
            );
            #[cfg(any(test, feature = "test-support"))]
            let post = post.and_then(|()| faults.after(failure_tests::Point::Post));
            if post.is_err() {
                self.post.invalidate();
            }
            post?;
        } else {
            self.post
                .copy_scene(device, &mut encoder, targets, generation_changed);
        }
        mark(&mut encoder, "post");
        self.last_animation_time = Some(animation_time);
        // Screen-space icons and their text are last ( 5.5).
        self.sprite_pass.draw(&mut encoder, targets);
        #[cfg(any(test, feature = "test-support"))]
        faults.draw_sentinel(device, &mut encoder, targets);
        mark(&mut encoder, "overlay_sprites");
        if let Some(timer) = timer.as_mut() {
            timer.finish(&mut encoder);
        }
        cpu_mark("timestamp_resolve");
        self.arena.flush(queue);
        cpu_mark("arena_flush");
        let encoded = clock.map(|clock| clock.elapsed());
        command_buffers.push(ModelPass::finished_buffer(encoder.finish()));
        cpu_mark("encoder_finish");
        // Joining here preserves native buffer order without earlier encoder waits.
        queue.submit(ModelPass::submitted_buffers(command_buffers));
        cpu_mark("queue_submit");
        if let Some(political) = political {
            political.submitted();
        }
        if stripes && let Some(colors) = &mut self.province_colors {
            colors.submitted();
        }
        #[cfg(any(test, feature = "test-support"))]
        faults.submissions.set(faults.submissions.get() + 1);
        if let Some(timer) = timer.as_mut() {
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

    /// Lends a frame's presentation step the hour builders' threads.
    pub fn use_workers(&mut self, workers: viewer_foundation::parallel::Workers) {
        self.workers = workers;
    }

    /// Milliseconds the last frame spent animating, uploading the overlay,
    /// encoding and submitting, under `BATTLE_PLAN_VIEWER_PROFILE`.
    pub fn render_laps(&self) -> [f64; 4] {
        self.render_laps
    }

    /// Numbers the next frame's `PROFILE gpu_passes` line.
    pub fn label_frame(&mut self, frame: u64) {
        if let Some(timer) = &mut self.pass_timer {
            timer.label(frame);
        }
    }

    /// The last frame's CPU laps, under `BATTLE_PLAN_VIEWER_PROFILE`. `pass_` fields
    /// partition caller-side encoding and submission, summing repeated views.
    /// The final encoder finish, queue submit and report belong to scene_submit.
    pub fn detail_laps(&self) -> String {
        format!(
            "{} {}",
            self.detail_laps.line(),
            self.pass_cpu_laps.line_with_prefix("pass_")
        )
    }

    /// The post grade of the last frame, for reporting.
    pub fn grading(&self) -> Option<String> {
        self.post.last_grading().map(|grading| grading.describe())
    }

    /// Whether the last frame ran the cast-shadow chain, for reporting.
    pub fn shadows(&self) -> bool {
        self.shadow.drawn()
    }

    /// The grid the last frame drew, for reporting.
    pub fn grid(&self) -> Grid {
        self.grid
    }

    /// Egui must update its native texture registration when this changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn targets(&self) -> Option<&Targets> {
        self.targets.as_ref()
    }

    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(Targets::output_view)
    }

    // names
    /// Name-layer vertices and icon quads the last frame drew, for reporting.
    pub fn overlay_counts(&self) -> (u32, u32) {
        (
            self.name_pass.vertex_count() + self.near_name_pass.vertex_count(),
            self.sprite_pass.quad_count(),
        )
    }

    pub fn pass_warnings(&self) -> Vec<String> {
        let mut warnings: Vec<_> = passes::lines(&self.pass_diagnostics).collect();
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

    /// Every refusal this scene has made, by a name stable across loads: world passes that
    /// refused, and every refused native, model and GUI shader draw not already drawn another
    /// way (a visual mod entity drawn from its recorded counterpart). A map showing visual mods
    /// compares these with the refusals it makes without them.
    pub fn draw_refusals(&self) -> std::collections::BTreeSet<String> {
        let mut refusals = self.arena.refusals();
        // By the pass refused, not its message, which can differ between loads.
        refusals.extend(
            self.pass_diagnostics
                .keys()
                .map(|pass| format!("Map shader {}: refused", pass.name())),
        );
        refusals
    }

    /// Named/counting failures of the most recently submitted GUI shader draws.
    pub fn shader_refusals(&self) -> &BTreeMap<String, u64> {
        &self.sprite_pass.native_refusals
    }

    /// Icon pages uploaded once per scene, for reporting.
    pub fn icon_page_count(&self) -> usize {
        self.icon_textures.page_count()
    }

    // trees
    /// Canopy instances the last frame drew, for reporting.
    pub fn tree_instances(&self) -> u32 {
        self.trees.as_ref().map_or(0, TreePass::drawn)
    }

    /// Canopy draw calls the last frame submitted: one per visible tile.
    pub fn tree_draw_calls(&self) -> u32 {
        self.trees.as_ref().map_or(0, TreePass::drawn_buckets)
    }

    // close zoom
    /// Sprawl houses the last frame drew, for reporting.
    pub fn sprawl_drawn(&self) -> u32 {
        self.sprawl.as_ref().map_or(0, ModelPass::drawn)
    }

    // close zoom
    /// Map buildings the last frame drew, for reporting.
    pub fn map_buildings_drawn(&self) -> u32 {
        self.map_buildings.as_ref().map_or(0, ModelPass::drawn)
    }

    // close zoom
    /// Aircraft models submitted in the last frame, for reporting.
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
    texture: &wgpu::Texture,
    width: u32,
    first: usize,
    bytes: &[u8],
) {
    let mut pixel = first;
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let x = pixel as u32 % width;
        let y = pixel as u32 / width;
        let pixels = remaining.len() / 4;
        let (columns, rows) = if x == 0 && pixels >= width as usize {
            (width, (pixels / width as usize) as u32)
        } else {
            ((width - x).min(pixels as u32), 1)
        };
        let count = columns as usize * rows as usize * 4;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &remaining[..count],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(columns * 4),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: columns,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        remaining = &remaining[count..];
        pixel += count / 4;
    }
}

/// Rewrites a data texture in place, or rebuilds it when its size changed.
///
/// Returns whether the texture object was replaced, which is the only case
/// where the bind groups that reference its view have to be rebuilt.
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
        write_data(queue, &slot.texture, size, 4, data);
        return Ok(false);
    }
    *slot = data_texture(device, queue, label, size, format, data)?;
    Ok(true)
}

/// With full-resolution rendering disabled, halve the renderer's positive signed dimensions with
/// truncation. Full-size cameras, refraction, shadow receivers and interface pixels retain their
/// dimensions. [`validate_size`] refuses a zero result.
pub fn world_size(size: [u32; 2], full_resolution: bool) -> [u32; 2] {
    if full_resolution {
        size
    } else {
        size.map(|dimension| dimension / 2)
    }
}

/// The display kind a recorded map event family's floating text is hidden with.
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

/// The map-frame actors that stay placed, keeping their admission, clocks, events' timing and
/// retirement, but do not draw: the map table and each impact whose switch is off. An impact
/// with no recorded effect is never hidden.
fn hidden_map_frame_actors(
    graphics: &dashboard_core::protocol::MapGraphics,
    table: impl IntoIterator<Item = (usize, u32, u32)>,
    impacts: impl IntoIterator<
        Item = (
            (usize, u32, u32),
            Option<crate::map::strike_effects::Effect>,
        ),
    >,
) -> BTreeSet<(usize, u32, u32)> {
    use crate::map::strike_effects::Effect;
    let table = table.into_iter().filter(|_| !graphics.map_table);
    let impacts = impacts.into_iter().filter_map(|(key, effect)| {
        match effect? {
            Effect::CriticalBombardment => !graphics.bombardment_explosions,
            Effect::MissileExplosion => !graphics.missile_explosions,
            Effect::Nuclear | Effect::Thermonuclear => !graphics.nuclear_strikes,
        }
        .then_some(key)
    });
    table.chain(impacts).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_floating_text_family_has_its_own_text_switch() {
        let kinds: std::collections::BTreeSet<_> = replay_data::rows::MapEventFamily::ALL
            .into_iter()
            .filter_map(map_text_kind)
            .collect();
        assert_eq!(kinds.len(), replay_data::rows::MapEventFamily::COUNT - 1);
        assert_eq!(
            map_text_kind(replay_data::rows::MapEventFamily::ScriptedMapRefresh),
            None
        );
        let graphics = dashboard_core::protocol::MapGraphics {
            text_generals: false,
            ..Default::default()
        };
        assert!(
            !graphics.map_text(
                map_text_kind(replay_data::rows::MapEventFamily::GeneralCaptured).unwrap()
            )
        );
        assert!(
            graphics.map_text(
                map_text_kind(replay_data::rows::MapEventFamily::BuildingDamage).unwrap()
            )
        );
    }

    #[test]
    fn map_frame_actors_hide_by_their_own_switch_and_stay_placed() {
        use crate::map::strike_effects::Effect;
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
        // An impact with no recorded effect always draws.
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

    #[test]
    fn half_resolution_truncates_world_dimensions_and_keeps_output_dimensions() {
        let output = [2049, 1153];
        assert_eq!(world_size(output, true), output);
        assert_eq!(world_size(output, false), [1024, 576]);
        assert_eq!(world_size([3440, 1440], false), [1720, 720]);
        assert!(validate_size(world_size([1, 100], false), 8192).is_err());
    }
}
