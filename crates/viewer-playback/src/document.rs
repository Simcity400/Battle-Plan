//! A replay and the verified local game content used to draw its current hour.

#[cfg(test)]
pub mod music_inputs;
mod trade_route_read;
pub use trade_route_read::TradeRouteRead;

use crate::map_icons::MapIcons;
use crate::map_views::CounterView;
use crate::map_views::Views;
use dashboard_text::naval_battle_name::battle_name_candidates;
use viewer_combat as combat;
use viewer_foundation::map::camera::MapSize;
use viewer_foundation::map::camera::{MapDefines, motion::CameraLaw};
use viewer_foundation::map::free_icons;
use viewer_foundation::map::gui_resources::GuiAssets;
use viewer_foundation::map::gui_resources::GuiFlags;
use viewer_foundation::map::gui_resources::viewport as gui_resources_viewport;
use viewer_foundation::map::icon_windows::Retry;
use viewer_foundation::map::layers::Omissions;
use viewer_foundation::map::memo::Keyed;
use viewer_foundation::map::memo::Recent;
use viewer_foundation::map::memo::TableKey;
use viewer_foundation::map::memo::project;
use viewer_foundation::map::native_resources::Context;
use viewer_foundation::map::native_shader::draw as native_shader_draw;
use viewer_foundation::map::native_windows::NativeWindows;
use viewer_foundation::map::province_schedule::Shared;
use viewer_foundation::map::sprites::Quad;
use viewer_foundation::movement_timing::EtaCache;
use viewer_foundation::movement_timing::Etas;
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::tooltip_painter::TooltipPosition;
use viewer_foundation::tooltip_painter::draw as tooltip_painter_draw;
use viewer_raids::map::raid_arrows;
#[cfg(any(test, feature = "test-support"))]
use viewer_world::map::name_pass;
use viewer_world::map::scene_content::MapAssets;

use dashboard_base::content::check_verification;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use installed_content_core::{ResolveOptions, map, map::MapData};
use replay::{campaign::Replay, campaign::SeekResult};
use replay_data::{BranchId, rows::tables as row_tables};
use replay_frame::frame::DecodedFrame;

use {
    crate::playback::Playback, viewer_foundation::map::province_map::HeightField,
    viewer_foundation::map::province_map::ProvinceMap,
    viewer_world::map::border_pass::BorderAssets, viewer_world::map::border_pass::StraitVertex,
    viewer_world::map::border_pass::Topology, viewer_world::map::border_pass::strait_quads,
    viewer_world::map::borders, viewer_world::map::borders::BorderMeshes,
    viewer_world::map::demilitarized::Zone, viewer_world::map::political::Regions,
};
// rivers
use {
    viewer_world::map::rivers, viewer_world::map::rivers::RiverAssets,
    viewer_world::map::rivers::RiverImage, viewer_world::map::rivers::RiverMesh,
};
// trees
use viewer_world::map::mapframe::FrameAssets;
use {
    viewer_world::map::trees, viewer_world::map::trees::TreeAssets,
    viewer_world::map::trees::TreeField, viewer_world::map::trees::TreeGeometry,
    viewer_world::map::trees::TreeImage,
};
// close zoom
use viewer_world::map::map_buildings::MapBuildingAssets;
use viewer_world::map::unit_models::{ModelReport, UnitModelAssets};
use {
    viewer_world::map::railways as rail_layer, viewer_world::map::railways::Bridges,
    viewer_world::map::railways::RailAssets, viewer_world::map::railways::RailMesh,
};
use {viewer_world::map::sprawl, viewer_world::map::sprawl::Sprawl};
// names
use viewer_raids::map::intel_icons;
use viewer_raids::map::raids;
use {
    viewer_foundation::map::camera::Camera, viewer_foundation::map::name_draw::Placements,
    viewer_foundation::map::sprites::Batch, viewer_world::map::labels,
    viewer_world::map::labels::LabelContent, viewer_world::map::labels::LabelReport,
    viewer_world::map::name_pass::NameContent, viewer_world::map::name_pass::PlacementInputs,
};
// counters
use viewer_counters::map::counters::{self, CounterContent, CounterInputs, CounterReport};
// orders: the recorded player's battle plan
use {
    viewer_counter_products::map::orders,
    viewer_foundation::map::province_geometry::ProvinceGeometry,
    viewer_foundation::map::ribbon_content::OrderAssets,
    viewer_foundation::map::ribbon_content::OrderField,
};
// weather
use viewer_foundation::map::perspective;
use viewer_foundation::map::tables::{self, Limitations};
use viewer_foundation::map::textures::IntelMap;
use {viewer_world::map::weather, viewer_world::map::weather::SnowMud};

/// The recorded hour to sit on after load: `--branch` and `--hour` select it,
/// `hour_fraction` is how far past it the frame sits.
pub struct InitialHour {
    pub branch: Option<BranchId>,
    pub hour: Option<replay_data::Hour>,
    pub hour_fraction: f32,
}

/// A replay, the hour on screen and the content it is drawn with.
pub struct Document {
    /// Everything that does not move with the hour, shared with the map's layers and the
    /// workers that build their products.
    pub content: Arc<Content>,
    pub playback: Playback,
    /// How far past [`Playback::hour`] the drawn frame is, in hours, in
    /// `[0, 1)`.
    ///
    /// The recorded hour is the whole frame's state. A moving division stands
    /// at its settled route point all hour, as the game draws it, so only the
    /// fog-of-war note reads this; it is zero for every recorded hour, which is
    /// what `--dump --hour <n>` renders.
    pub hour_fraction: f32,
    /// State ids to treat as demilitarised **in addition to** the recorded
    /// ones. Empty unless `--dump --demilitarized` sets it; see
    /// [`Hour::demilitarized_zone`].
    pub demilitarized_override: BTreeSet<u32>,
}

impl std::ops::Deref for Document {
    type Target = Content;

    fn deref(&self) -> &Content {
        &self.content
    }
}

/// The load's view of the visual mod fallback ([`crate::visual_fallback`]): the recording's
/// identity and the decisions its keys hold.
pub type VisualFallback<'a> = (
    &'a str,
    &'a BTreeMap<crate::visual_fallback::FallbackKey, crate::visual_fallback::Decision>,
);

/// The decision `fallback` holds for the visual mods `selected` as they are now, when it shows
/// no visual mods: `None` once a mod changes (another revision, another key), for another
/// recording, or when the mods are shown.
fn decided(
    fallback: Option<VisualFallback<'_>>,
    options: &ResolveOptions,
    selected: &[installed_content_core::content::visual::Choice],
    recorded: &[installed_content_core::content::visual::RecordedMod],
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<(
    crate::visual_fallback::FallbackKey,
    crate::visual_fallback::Decision,
)> {
    let (recording, decisions) = fallback?;
    let key = crate::visual_fallback::FallbackKey {
        recording: recording.to_owned(),
        revision: installed_content_core::content::visual::revision(options, selected, recorded, cancel)
            .ok()?,
    };
    let decision = decisions.get(&key)?;
    decision.reason().is_some().then(|| (key, decision.clone()))
}

/// What the selected visual mods hold now ([`installed_content_core::content::visual::revision`]), with the
/// recording's own mods as the selection mounts them; `None` when it cannot be read.
fn visual_revision_now(
    options: &ResolveOptions,
    replay: &Replay,
    visual_mods: Option<&[installed_content_core::content::visual::Choice]>,
) -> Option<String> {
    let selected = visual_mods.filter(|selected| !selected.is_empty())?;
    let recorded: Vec<_> = replay
        .identity
        .mods
        .iter()
        .map(|recorded| installed_content_core::content::visual::RecordedMod {
            id: recorded.id.clone(),
            name: recorded.display_name.clone(),
        })
        .collect();
    installed_content_core::content::visual::revision(
        options,
        selected,
        &recorded,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .ok()
}

/// What `load` loads, loaded again until the revision of visual mods it loaded with is still the
/// one `now` reads as it returns: a mod that changed during a load is never installed stale.
/// Content that shows no revision (no selection, or one that could not be read) returns at once.
fn until_current<T>(
    mut load: impl FnMut() -> io::Result<(T, String)>,
    mut now: impl FnMut() -> Option<String>,
) -> io::Result<T> {
    loop {
        let (loaded, revision) = load()?;
        if revision.is_empty() || now().is_none_or(|now| now == revision) {
            return Ok(loaded);
        }
    }
}

/// The city sprawl of `plan`: placed as the recording places it, its buildings' artwork
/// loaded through the visual mods' artwork fallback ([`installed_content_core::ResolvedLayers::artwork`]),
/// with the graphics defines of `defines` (recorded, graphics) while the mods show.
fn load_sprawl(
    layers: &installed_content_core::ResolvedLayers,
    decode: installed_content_image::native::Decode,
    plan: &sprawl::SprawlPlan,
    (defines, graphics): (
        &installed_content_core::defines::Defines,
        &installed_content_core::defines::Defines,
    ),
) -> io::Result<(Sprawl, Vec<String>)> {
    layers.artwork("city sprawl", |stack| {
        let mut warnings = Vec::new();
        let defines = if stack.shows_visual_mods() {
            graphics
        } else {
            defines
        };
        Sprawl::load(stack, decode, plan, defines, &mut warnings).map(|sprawl| (sprawl, warnings))
    })
}

/// The verified local game content a replay is drawn with: everything that
/// does not change with the recorded hour.
pub struct Content {
    loading: installed_content_core::content::Loading,
    first_frame_reported: std::sync::atomic::AtomicBool,
    /// The executable tables the recording carries, which its map's work reads in their scope.
    pub tables: Option<engine_executable::executable_tables::Tables>,
    pub products: map_products::ProductContent,
    /// Fresh combat content identity, constructed with every host content load.
    pub combat: Arc<combat::Content>,
    pub counter_content: Arc<viewer_counter_products::map::counter_content::Content>,

    /// Perspective-independent compute resources, weakly cached per graphics device.
    pub political_gpu: viewer_world::map::political_gpu::Cache,
    /// The game textures scenes of this content draw, weakly cached per graphics device.
    pub scene_textures: viewer_world::map::scene::TextureCache,
    pub texture_quality: u8,
    pub visual_mods: Option<Vec<installed_content_core::content::visual::Choice>>,
    /// What the mounted visual mods held when this content loaded
    /// ([`installed_content_core::content::visual::revision`]); empty when none is mounted.
    pub visual_revision: String,
    /// The fallback key it was built for and what it was built under
    /// ([`crate::visual_fallback`]); `None` without a visual mod selection to fall back from.
    pub fallback: Option<crate::visual_fallback::Built>,
    pub map: Arc<MapData>,
    /// The content's camera and frame defines with the user's camera tilt.
    pub map_defines: MapDefines,
    /// The content's zoom defines with the user's zoom and camera speeds.
    pub camera_law: CameraLaw,
    /// The minimap's slide: `NInterface.MINIMAP_TOGGLE_DURATION` and `_SHIFT`.
    pub minimap_defines: Result<combat::minimap::Defines, String>,
    /// The recorded speeds the day/night cycle follows, or why they could not be read.
    pub game_speeds: Result<viewer_foundation::map::frame::GameSpeeds, String>,
    /// The user's `settings.txt` display facts that size the game's interface
    /// on screen, or why they could not be read; see [`crate::map::ui_scale`].
    pub game_display: Result<installed_content_settings::settings::Display, String>,
    /// The map options the user's `settings.txt` saves, which the window, a dump and the picker
    /// start from; a line the game could not read is in [`Self::warnings`].
    pub game_options: installed_content_settings::settings::MapOptions,
    /// The configured user folder's local world target size; no recorder UI state.
    pub game_full_resolution: bool,
    /// Local show-all strategic names preference; never read from the recording.
    pub game_strategic_names: bool,
    /// The user's `settings.txt` sound volumes.
    pub game_audio: dashboard_core::protocol::MapAudio,
    /// The game's second master factor (`dev_master_volume`), a fraction.
    pub game_dev_master: f32,
    /// The game's map cursors, each or why it is not shown.
    pub cursors: crate::cursors::Cursors,
    /// The game images and textures the map passes draw from.
    pub assets: MapAssets,
    /// Persistent recording compatibility, content verification and presentation limitations.
    pub warnings: Vec<String>,
    /// Shared lazy content reads, reported afresh for map diagnostics and dumps.
    pub uncovered_reads: installed_content_core::content::UncoveredReads,
    /// The content's sound definitions and the layers their samples are read
    /// from; `None` when they are unreadable, which leaves the maps silent.
    pub audio: Option<Arc<viewer_audio::Source>>,
    pub source_path: PathBuf,
    // borders
    /// `map/terrain/border_*_0.dds` and `strait.dds`.
    pub border_assets: BorderAssets,
    /// The traced border strips. Every land edge sits in the `Province` bucket until the hour's
    /// political regions re-bucket it; see `viewer_world::map::border_pass`.
    pub border_meshes: Arc<BorderMeshes>,
    /// The strait ribbons from `map/adjacencies.csv`.
    pub straits: Vec<StraitVertex>,
    /// `map/strategicregions/*.txt`, which layer 1 of the political fill is built from; see
    /// `viewer_world::map::political::strategic_regions_of`.
    pub strategic_regions: Arc<BTreeMap<u32, installed_content_regions::regions::StrategicRegion>>,
    pub naval_dominance: viewer_world::map::naval_dominance::Content,
    /// The map modes' defines, colours and the content's scripted modes.
    pub map_modes: Arc<viewer_world::map::map_mode::Content>,
    /// Wall time of the border trace, for reporting.
    pub border_trace: Duration,
    // names
    /// Fonts and map-icon sprites, uploaded once by the scene.
    pub icons: MapIcons,
    /// Province adjacency, province bounds and the localisation.
    pub names: NameContent,
    /// The label window layout and the unit-stack anchors.
    pub labels: LabelContent,
    /// The resources mode's state icons, or why their content cannot be read.
    pub resource_icons: Result<viewer_world::map::resource_icons::ResourceIconContent, String>,
    /// The strait and canal icons of the content's adjacency rules.
    pub adjacency_icons: viewer_world::map::adjacency_icons::AdjacencyIcons,
    /// The trade-route lines' defines, or why they cannot be read.
    pub trade_routes: Result<viewer_world::map::trade_routes::TradeRouteDefines, String>,
    air_assignment_colors: Result<viewer_world::map::air_assignment::Colors, String>,
    occupation_stripe_alpha: Result<viewer_world::map::occupation_stripes::Alpha, String>,
    naval_selection_colors: Result<viewer_world::map::naval_selection::Colors, String>,
    naval_region_eligibility: BTreeSet<u32>,
    air_order_colors: Result<viewer_world::map::air_orders::Colors, String>,
    air_transfer_color: Result<[f32; 4], String>,
    // rivers
    /// `map/terrain/RiverSurface_*`.
    pub river_assets: RiverAssets,
    /// The ribbons traced from `map/rivers.bmp`.
    pub river_mesh: RiverMesh,
    /// What the trace found, including the rivers the engine reports and drops.
    pub river_trace: rivers::Trace,
    /// Wall time of the river trace and widening, for reporting.
    pub river_build: Duration,
    // counters
    /// The counter, base and combat anchors from `map/unitstacks.txt` and
    /// `map/buildings.txt`.
    pub counters: Arc<CounterContent>,
    pub radar_centers: viewer_world::map::radar_symbols::Centers,
    /// Raid hover identity and its immutable content, fresh for every content rebuild.
    pub raid_hover: Arc<raids::tooltips::Content>,
    // the map's table frame
    /// Content-resolved ambient mesh actors and their materials.
    pub frame_assets: FrameAssets,
    // trees
    /// The canopy meshes, their textures and the `tree_*` season blocks.
    pub tree_assets: TreeAssets,
    /// The canopy geometry, flattened over every LOD of every tree mesh.
    pub tree_geometry: TreeGeometry,
    /// Every canopy instance on the map, bucketed by tile.
    pub tree_field: TreeField,
    /// Wall time of the instance generation, for reporting.
    pub tree_build: Duration,
    // close zoom
    /// The city sprawl: its mesh library and its placed houses.
    pub sprawl: Sprawl,
    /// How long generating the sprawl took.
    pub sprawl_build: Duration,
    /// The map-building slot table, types and meshes; the placed buildings are
    /// per hour and come from [`Document::map_building_field`].
    pub map_buildings: MapBuildingAssets,
    /// `gfx/models/supply/railroad.dds`.
    pub rail_assets: RailAssets,
    /// Static boundary crossings and anchors for the recorded railway graph.
    pub rail_geometry: rail_layer::RailGeometry,
    pub aircraft: Arc<viewer_world::map::aircraft::AircraftAssets>,
    pub trains: Arc<viewer_world::map::trains::TrainAssets>,
    /// How long constructing the static railway geography took.
    pub rail_build: Duration,
    /// The railway bridges, which are meshes rather than strip.
    pub bridges: Bridges,
    /// The land division entities, the sub-unit keys and the graphical
    /// cultures; the placed figures are per hour and come from
    /// [`Document::unit_model_field_sailing`].
    pub unit_models: UnitModelAssets,
    // orders: the recorded player's battle plan
    /// `gfx/maparrows/maparrows.txt` and the mask and pattern pages of every
    /// arrow type the battle-plan layer can select.
    pub orders: OrderAssets,
    pub weather_effects: Arc<viewer_world::map::weather_effects::Assets>,
    /// Derived ETA results shared across the hours in flight. The counter
    /// layer builds on the window thread and the close-zoom models on workers
    /// that run ahead of it; see [`Hour::etas`].
    eta_cache: Arc<EtaCache>,
    /// Each hour's battle plan, shared by the hours whose plan tables
    /// ([`orders::TABLES`]) are the same allocations, keyed also on the
    /// viewing country's slot, the only thing it reads from the perspective.
    order_memo: Recent<(TableKey, u32, bool), OrderField>,
    trade_route_memo: Recent<(TableKey, u32), viewer_world::map::trade_routes::TradeRoutes>,
    /// Each route's stops as the build before searched them, for the lines built since: a build
    /// that finds the lock held searches its own ([`trade_routes::KeptStops`]).
    trade_route_stops: std::sync::Mutex<viewer_world::map::trade_routes::KeptStops>,
    /// Each hour's counter type-icon relations, shared by the hours whose
    /// relation tables and viewing country's volunteer hosts are unchanged
    /// ([`counters::type_tint::Memo`]).
    type_tint_memo: counters::type_tint::Memo,
    mission_memo: counters::MissionMemo,
    /// The overlay tables' whole-world parts kept from hour to hour ([`counters::Need::by_key`]).
    overlay_carry: counters::Carry,
    /// The content-derived movement map and defines behind route points,
    /// stack kinds and ETAs; `None` when the content could not be derived, in
    /// which case moving units stand at their province and have no ETA.
    pub movement: Option<Arc<MovementContent>>,
    /// Province pixel centroids and boundary crossing points, the two
    /// quantities the engine builds an order's polyline from.
    pub province_geometry: Arc<ProvinceGeometry>,
    /// How long that one pass over `map/provinces.bmp` took, for reporting.
    pub province_geometry_build: Duration,
}

/// One recorded hour of a document: its decoded frame, the fraction past it,
/// and the content it is drawn with. Everything built per hour is built here,
/// on whichever thread holds the frame.
pub struct Hour<'a> {
    pub content: &'a Content,
    pub frame: &'a DecodedFrame,
    /// See [`Document::hour_fraction`].
    pub hour_fraction: f32,
    /// The last perspective read for this hour; several builders need it.
    product_hour: map_products::ProductHour<'a>,
    pool_limits: viewer_world::map::equipment_graphics::LimitValues,
    worker_etas: Option<Arc<viewer_foundation::movement_timing::WorkerEtas>>,
    air_rosters: Option<Arc<crate::playback::AirRosters>>,
    /// The hours of the publication this hour's rows end at the recording moved a mover at: the
    /// counters' layout gives each mover a place for each of them
    /// ([`viewer_foundation::map::counter_move::MovedHours`]). None for an hour laid out outside a
    /// step — a dump, a judge's own layout — whose movers are settled anyway.
    moved_hours: Option<Arc<viewer_foundation::map::counter_move::MovedHours>>,
}

impl std::ops::Deref for Hour<'_> {
    type Target = Content;

    fn deref(&self) -> &Content {
        self.content
    }
}

impl Content {
    /// The naval dominance sources of `frame` for the country in slot `ours`, made as its overlay's
    /// tables make them ([`counters::naval_dominance_sources`]).
    pub fn naval_dominance_sources(&self, frame: &DecodedFrame, ours: u32, region: u32) -> viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap {
        counters::naval_dominance_sources(frame, &self.map, &self.counters, ours, region, self.icons.raids.movement_reads(), self.icons.raids.mission_topology())
    }

    /// The heightmap as the map passes sample it.
    pub fn heights(&self) -> HeightField<'_> {
        HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        }
    }

    /// One hour's battle plan for `ours` from a frame holding its tables (all of them for
    /// invasions, active orders and labelled paradrops, [`orders::requires_full_frame`]).
    pub fn battle_plan(
        &self,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        ours: u32,
        show_allied: bool,
    ) -> io::Result<OrderField> {
        let heights = self.heights();
        orders::build(
            &orders::OrderInputs {
                assets: &self.orders,
                transport: Some(
                    viewer_foundation::map::air_transport_content::AirTransportContent {
                        geometry: &self.icons.raids.geometry,
                        state_provinces: &self.icons.raids.trigger_content.state_provinces,
                        region_by_province: &self.icons.raids.regions,
                        region_provinces: &self.icons.raids.region_provinces,
                        sub_units: self
                            .icons
                            .raids
                            .transport_sub_units
                            .as_ref()
                            .map_err(String::as_str),
                        map_scale: self.icons.raids.defines.map_scale,
                        ace_wing_size: self.icons.raids.defines.ace_wing_size,
                        ace_wing_size_max_bonus: self
                            .icons
                            .raids
                            .defines
                            .ace_wing_size_max_bonus,
                    },
                ),
                reads: Some(self.icons.raids.movement_reads()),
                geometry: &self.province_geometry,
                movement: self.movement.as_deref().map(|content| &content.map),
                map: &self.map,
                heights: &heights,
                ours,
                show_allied,
                links,
                labels: self.icons.fonts.arrow().map(|font| orders::LabelInputs {
                    font,
                    localisation: &self.names.localisation,
                }),
            },
            frame,
        )
    }
}

/// The frame's provinces for the paced report: all, those the camera admits, and those out of
/// view whose dirty bit still admits their owners' icons off screen.
fn note_scope(schedule: &viewer_foundation::map::province_schedule::Schedule, bounds: &BTreeMap<u32, [f32; 4]>) {
    use viewer_foundation::map::{province_schedule::Update, redraws::note};
    let (mut in_view, mut dirty) = (0_u64, 0_u64);
    for &province in bounds.keys() {
        match schedule.update(province) {
            Some(Update::InView) => in_view += 1,
            _ if schedule.dirty(province) => dirty += 1,
            _ => {}
        }
    }
    note("scope.provinces", bounds.len() as u64);
    note("scope.provinces_in_view", in_view);
    note("scope.provinces_dirty_off_screen", dirty);
    viewer_foundation::view_record::set("overlay", "provinces_in_view", in_view);
    viewer_foundation::view_record::set("overlay", "provinces_dirty_off_screen", dirty);
}

/// The hour's intel icons in the overlay's tables: made with them, or deferred until a layout first
/// reads them ([`counters::Need::by_key`]), so tables whose icons the map's intel layer lays in
/// ([`OverlayTables::set_intel`]) never make their own. A deferred part is made from the tables' own
/// rows and viewing country, with the document's content, as a made one is.
#[derive(Clone)]
enum IntelPart {
    Made(Arc<intel_icons::IntelIcons>),
    Deferred(Arc<DeferredIntel>),
}

struct DeferredIntel {
    frame: Arc<DecodedFrame>,
    ours: u32,
    made: std::sync::OnceLock<Arc<intel_icons::IntelIcons>>,
}

impl IntelPart {
    fn get(&self, hour: &Hour<'_>) -> Arc<intel_icons::IntelIcons> {
        match self {
            Self::Made(made) => Arc::clone(made),
            Self::Deferred(deferred) => {
                // A reader never waits on another's making: it makes its own, and the first made
                // is kept.
                if let Some(made) = deferred.made.get() {
                    return Arc::clone(made);
                }
                let made = Arc::new(intel_icons::icons(
                    &hour.icons.intel,
                    &hour.icons.raids,
                    &deferred.frame,
                    deferred.ours,
                    &hour.names.localisation,
                    &hour.raid_hover.recorded_language,
                ));
                let _ = deferred.made.set(Arc::clone(&made));
                made
            }
        }
    }
}

/// Raw country reference to canonical slot, the `CountryLinks` join.
pub fn links_of(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, u32>> {
    Ok(tables::core::<row_tables::CountryLinks>(frame)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect())
}

/// The hit shapes of the overlay's own icons as a draw laid them out: the raid icons it laid out
/// for hovering, drawn or hidden, the Operatives entries, and the occupation and adjacency icons.
/// The draw writes them beside the counters' report, so the map is resolved against what was
/// drawn until the next draw, whatever layout is installed meanwhile.
#[derive(Default)]
pub struct OverlayHits {
    pub raids: Vec<raids::tooltips::Hit>,
    pub intel: Vec<intel_icons::IntelHit>,
    pub occupation: Vec<viewer_world::map::occupation_icons::Hit>,
    pub adjacency: Vec<viewer_world::map::adjacency_icons::Hit>,
}

impl OverlayHits {
    /// The intel entry under the pointer. `seen` gives the pointer in one entry's own layout
    /// with its container's draw rank, the coordinates every consumer of a map-icon hit works in
    /// ([`crate::app::map_hits::local`]); `None` for an entry a free eye does not show.
    pub fn intel_at(
        &self,
        seen: impl Fn(&intel_icons::IntelHit) -> Option<([f32; 2], u32)>,
    ) -> Option<&intel_icons::IntelHit> {
        intel_icons::hit_seen(&self.intel, seen)
    }
}

/// Camera-dependent preparation retained while only the hour fraction moves.
///
/// The raid icons are kept apart from the labels, so that a raid batch's
/// icons replace them alone ([`PreparedOverlay::replace_raids`]): their
/// layers are their depth order among themselves, and no other part of the
/// overlay reads them.
pub struct PreparedOverlay {
    pub radar: Arc<viewer_world::map::radar_symbols::Field>,
    pub air_ranges: Arc<viewer_world::map::air_ranges::Fields>,
    pub air_assignment: viewer_world::map::air_assignment::Field,
    pub occupation_stripes: viewer_world::map::air_assignment::Field,
    pub naval_selection: viewer_world::map::naval_selection::Field,
    pub air_orders: Arc<viewer_world::map::air_orders::Prepared>,
    /// The labels' prepared values, drawn through this view's native pools.
    labels: labels::PreparedLabels,
    /// The raid icons, drawn before the labels.
    raids: Batch,
    /// Prepared raid owners; presentation windows retain their own clock.
    native_raids: raids::PreparedRaids,
    icons: counters::PreparedCounters,
    label_report: LabelReport,
    raid_report: raids::RaidReport,
    /// The Operatives map mode's state containers, and their quads and report as of the last
    /// draw.
    native_intel: intel_icons::PreparedIntel,
    intel: Batch,
    intel_report: intel_icons::IntelReport,
    /// [`crate::map::lazy_icons::LazyTextures::version`] when it was laid out; a
    /// newer version may have decoded a flag or portrait it left out.
    lazy_version: u64,
    /// Its tables' land combat flags came from the engaged index; otherwise
    /// they are what a load at the hour rebuilds, and the window lays the
    /// overlay out again once the index covers the hour.
    engaged_covered: bool,
    /// The layers' products this layout was made with
    /// ([`counters::layered::Layered::identity`]). A layer publishing another product makes a
    /// different layout of the same hour, which the overlay's gate names instead of reporting it
    /// as a difference of layouts.
    layers: [usize; 6],
    /// The counter options and the view's counter memory it was laid out with, which its
    /// redraws keep.
    counter_view: CounterView,
    /// The resources mode's state icons and the strait and canal icons, presented through this
    /// view's pools.
    resource_icons: viewer_world::map::resource_icons::PreparedResourceIcons,
    occupation_icons: viewer_world::map::occupation_icons::PreparedOccupationIcons,
    /// Who laid out the resource icons and the occupation icons.
    resource_icons_by: MadeBy,
    occupation_icons_by: MadeBy,
    adjacency_icons: viewer_world::map::adjacency_icons::PreparedAdjacencyIcons,
    /// The strait and canal icons' priority in the view's map mode.
    adjacency_priority: i8,
    /// The resource, occupation and adjacency icons as last drawn.
    mode_icons: Batch,
    /// What the last draw refused, by reason.
    mode_icon_refused: BTreeMap<String, usize>,
    /// Which build of the tables it was laid out over ([`OverlayTables::identity`]): a layout is
    /// placed again over its own publication's tables and no other's.
    tables: usize,
}

/// What an overlay holds that the camera does not move: the province fields the hour's rows
/// supply, and the labels laid out for every camera. An overlay placed again at another camera
/// takes these as they are ([`PreparedOverlay::project`]).
pub struct Still {
    labels: labels::PreparedLabels,
    air_assignment: viewer_world::map::air_assignment::Field,
    air_orders: Arc<viewer_world::map::air_orders::Prepared>,
    air_ranges: Arc<viewer_world::map::air_ranges::Fields>,
    occupation_stripes: viewer_world::map::air_assignment::Field,
    naval_selection: viewer_world::map::naval_selection::Field,
    radar: Arc<viewer_world::map::radar_symbols::Field>,
    engaged_covered: bool,
}

/// Who laid out an overlay part that a map layer can draw instead of today's maker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadeBy {
    /// Today's maker, over the hour's frame or the rows at the head.
    Today,
    /// Nobody: left empty for the layer, whose product the window lays in.
    Left,
    /// The layer, from its product at this version.
    Layer(u64),
}

/// The tables the trade routes' convoy lines read ([`Hour::trade_route_lines`]).
const TRADE_ROUTE_TABLES: &[replay_data::rows::TableId] = &trade_route_read::TABLES;

/// The overlay parts the map's layers draw: today's maker leaves them empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftToLayers {
    pub resource_icons: bool,
    pub occupation_marks: bool,
    /// The radar layer draws the radar field: the hour's tables read none, since the frame takes
    /// the layer's ([`crate::app::Viewer::draw_map`]) and the hour's would only be its fallback.
    pub radar: bool,
}

impl LeftToLayers {
    /// What `side`'s layers draw; nothing while the map's playhead is off.
    pub fn of(side: Option<&crate::playhead_host::MapSide>) -> Self {
        Self {
            resource_icons: side.and_then(crate::layers::resource_icons::drawn).is_some(),
            occupation_marks: side.and_then(crate::layers::occupation_marks::drawn).is_some(),
            radar: side.is_some_and(|side| side.draws("radar")),
        }
    }

}

/// The counters laid out on `view`'s retained memory, over the rows their tables were made from.
fn prepare_counters_with(
    inputs: &CounterInputs<'_>,
    tables: &OverlayTables,
    view: &CounterView,
    lazy_version: u64,
) -> io::Result<counters::PreparedCounters> {
    let mut retained = view
        .presentation
        .counters
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    counters::prepare_retained(inputs, &tables.counters, lazy_version, &mut retained)
}

/// A dump retains the same presented windows across its scene frames.
pub struct DumpOverlay<'a> {
    hour: Hour<'a>,
    prepared: PreparedOverlay,
    tables: OverlayTables,
    camera: Camera,
    viewport: [u32; 2],
    ui_scale: f32,
    layers: OverlayLayers,
    pub batch: Batch,
    pub extra: Batch,
    pub sprites: Batch,
    /// The sprites as a free camera shows them ([`free_icons::present`]).
    presented: Batch,
    /// How a free camera shows each icon, which hover follows; `None` when drawn as laid out.
    looks: Option<free_icons::Looks>,
    pub labels: LabelReport,
    pub counters: CounterReport,
    pub hits: OverlayHits,
    pub raids: raids::RaidReport,
    pub settlement_warnings: Vec<String>,
}

impl DumpOverlay<'_> {
    /// The sprites the last [`DumpOverlay::present`] drew: as laid out, or as a free camera
    /// shows them.
    pub fn shown(&self) -> &Batch {
        if self.camera.free().is_some() {
            &self.presented
        } else {
            &self.sprites
        }
    }

    pub fn schedule(&self) -> &Shared {
        self.prepared.counter_view.presentation.schedule()
    }

    fn advance_provinces(&self) {
        let bounds = &self.hour.unit_models.bounds;
        self.schedule()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .advance_frame(
                bounds,
                bounds.keys().copied(),
                self.camera,
                self.viewport,
                true,
            );
    }

    pub fn retries(&self) -> Vec<Retry> {
        self.prepared
            .counter_view
            .presentation
            .windows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retries()
    }

    /// The overlay as of the last presented frame: its intel icons' report and its stripes.
    pub fn prepared(&self) -> &PreparedOverlay {
        &self.prepared
    }

    /// How the last presented frame's free camera showed each icon; `None` as laid out.
    pub fn looks(&self) -> Option<&free_icons::Looks> {
        self.looks.as_ref()
    }

    pub fn naval_dominance_sources(
        &self,
    ) -> Arc<viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap> {
        self.tables.naval_dominance_sources()
    }

    /// Opens the target details window of the raid offer bubble at `province`, built from the
    /// hour's rows for the viewing country as the bubble's click builds it; how many targets it
    /// lists.
    pub fn open_raid_details(&mut self, province: u32) -> Result<usize, String> {
        let targets = self
            .tables
            .raids
            .icons
            .iter()
            .find_map(|icon| match &icon.tooltip {
                Some(raids::tooltips::Key::Offer(targets)) if icon.province == province => Some(Arc::clone(targets)),
                _ => None,
            })
            .ok_or_else(|| {
                let offers: Vec<String> = self
                    .tables
                    .raids
                    .icons
                    .iter()
                    .filter(|icon| matches!(icon.tooltip, Some(raids::tooltips::Key::Offer(_))))
                    .map(|icon| icon.province.to_string())
                    .collect();
                format!("no raid offer bubble stands at province {province} for the viewing country; offers stand at [{}]", offers.join(", "))
            })?;
        let details = raids::tooltips::details(&self.hour.raid_hover, self.hour.frame, self.tables.vision.ours(), &targets);
        let count = details.entries.len();
        self.prepared.counter_view.raid_details = Some((province, Arc::new(details)));
        Ok(count)
    }

    pub fn refuse_settlement(&mut self, retries: Vec<Retry>) {
        self.settlement_warnings = retries.iter().map(Retry::warning).collect();
    }

    pub fn present(&mut self, time: f32) -> io::Result<&Batch> {
        // Advance even when models are omitted; owners still use the province schedule.
        self.advance_provinces();
        loop {
            let before = self.hour.icons.lazy.version();
            (self.batch, self.labels, self.counters, self.hits, self.raids) = self.prepared.draw(
                &self.hour,
                self.camera,
                self.viewport,
                self.ui_scale,
                &self.layers,
                time,
            )?;
            self.hour.icons.lazy.settle();
            if self.hour.icons.lazy.version() == before {
                viewer_foundation::map::sprites::dump_sprites(
                    &self.batch,
                    &self.extra,
                    &mut self.sprites,
                );
                if self.camera.free().is_none() {
                    self.looks = None;
                    return Ok(&self.sprites);
                }
                // A free camera shows the icons standing in the world.
                let ground = free_icons::Ground {
                    field: self.hour.heights(),
                    sight: self.hour.assets.terrain_sight(),
                };
                // A dump draws one hour with every unit settled, so no mover carries a track.
                let at = viewer_foundation::map::anchored::Head { hour: self.hour.frame.hour, fraction: 0.0 };
                self.looks = Some(free_icons::present(
                    &self.camera,
                    self.viewport,
                    self.ui_scale,
                    Some(&ground),
                    &self.sprites,
                    at,
                    &mut self.presented,
                ));
                return Ok(&self.presented);
            }
            self.prepared = self.hour.prepare_overlay_cached(
                self.camera,
                self.viewport,
                self.ui_scale,
                &self.tables,
                &self.prepared.counter_view,
            )?;
        }
    }
}

impl PreparedOverlay {
    /// The counter plate the primary button holds and the one under the pointer, which draw
    /// without a new preparation.
    pub fn set_pressed(
        &mut self,
        pressed: Option<viewer_foundation::unit_handle::Unit>,
        plate_hovered: Option<viewer_foundation::unit_handle::Unit>,
    ) {
        self.counter_view.pressed = pressed;
        self.counter_view.plate_hovered = plate_hovered;
    }

    /// The raid bubble whose details window is open, with what its click built, and the bubble
    /// under the pointer: they change no layout, so the frame sets them as they change.
    pub fn raid_view(&self) -> (Option<(u32, Arc<raids::tooltips::Details>)>, Option<u32>) {
        (self.counter_view.raid_details.clone(), self.counter_view.raid_hovered)
    }

    pub fn set_raid_view(&mut self, (details, hovered): (Option<(u32, Arc<raids::tooltips::Details>)>, Option<u32>)) {
        self.counter_view.raid_details = details;
        self.counter_view.raid_hovered = hovered;
    }

    /// The view the overlay presents through, for one laid out on another memory of it
    /// ([`crate::map_views::Presentation::sharing`]).
    pub fn set_counter_view(&mut self, view: CounterView) {
        self.counter_view = view;
    }

    /// Takes `old`'s drawn land plates whose stack and style this overlay's are the same by value:
    /// for an overlay of another hour laid out for `old`'s view, with the same art.
    pub fn carry_plates(&mut self, old: &mut PreparedOverlay) {
        if self.lazy_version == old.lazy_version {
            self.icons.keep_drawn_by_value(&mut old.icons);
        }
    }

    /// Whether flags or portraits have decoded since this overlay was laid out.
    /// Places this overlay at `camera`: the counters' pass over the plans the view keeps, and
    /// each icon kind's projection. The parts the camera does not move ([`Still`]) are kept, so
    /// moving the camera reads no row and lays no hour out again. On a refusal the overlay is
    /// left as it was.
    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one placing")]
    pub fn project(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        tables: &OverlayTables,
        view: &CounterView,
        rows: &OverlayRows,
    ) -> io::Result<()> {
        if tables.identity() != self.tables {
            return Err(io::Error::other(format!(
                "the overlay laid out over the tables of build {} was placed again over build {}'s: a layout is placed over its own publication's tables",
                self.tables,
                tables.identity()
            )));
        }
        let mut still = self.take_still();
        let mut spans = viewer_foundation::profiling::BuilderSpans::new(false);
        match hour.prepare_overlay_with(camera, viewport, ui_scale, tables, view, rows, &mut spans, Some(&mut still)) {
            Ok(mut placed) => {
                // A column whose plan the view kept keeps the plate it drew, at the new origin.
                placed.icons.keep_drawn(&mut self.icons);
                *self = placed;
                Ok(())
            }
            Err(error) => {
                self.put_still(still);
                Err(error)
            }
        }
    }

    fn take_still(&mut self) -> Still {
        Still {
            labels: std::mem::take(&mut self.labels),
            air_assignment: std::mem::take(&mut self.air_assignment),
            air_orders: Arc::clone(&self.air_orders),
            air_ranges: Arc::clone(&self.air_ranges),
            occupation_stripes: std::mem::take(&mut self.occupation_stripes),
            naval_selection: std::mem::take(&mut self.naval_selection),
            radar: Arc::clone(&self.radar),
            engaged_covered: self.engaged_covered,
        }
    }

    fn put_still(&mut self, still: Still) {
        let Still { labels, air_assignment, air_orders, air_ranges, occupation_stripes, naval_selection, radar, engaged_covered } = still;
        self.labels = labels;
        self.air_assignment = air_assignment;
        self.air_orders = air_orders;
        self.air_ranges = air_ranges;
        self.occupation_stripes = occupation_stripes;
        self.naval_selection = naval_selection;
        self.radar = radar;
        self.engaged_covered = engaged_covered;
    }

    pub fn is_stale(&self, hour: &Hour<'_>) -> bool {
        self.lazy_version != hour.icons.lazy.version()
    }

    /// The layers' products this layout was made with.
    pub fn layers(&self) -> [usize; 6] {
        self.layers
    }

    /// Which build of the tables it was laid out over ([`OverlayTables::identity`]).
    pub fn tables(&self) -> usize {
        self.tables
    }

    /// The icon windows the last draw asked this view's native pools for and they have not made
    /// yet: the pools make them over the frames after the one that asked, so a frame drawn once
    /// can be missing them ([`crate::render`]'s dump settles the same way).
    pub fn retries(&self) -> Vec<Retry> {
        self.counter_view.presentation.windows.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).retries()
    }

    /// The overlay presented at `time` seconds on the view's own presentation clock (see
    /// [`counters::PreparedCounters::draw`]).
    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    pub fn draw(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        layers: &OverlayLayers,
        time: f32,
    ) -> io::Result<(Batch, LabelReport, CounterReport, OverlayHits, raids::RaidReport)> {
        let mut batch = Batch::default();
        let mut report = CounterReport::default();
        let mut hits = OverlayHits::default();
        self.draw_into(hour, camera, viewport, ui_scale, layers, time, &mut batch, &mut report, &mut hits)?;
        Ok((
            batch,
            self.label_report.clone(),
            report,
            hits,
            self.raid_report.clone(),
        ))
    }

    /// The presented frame's passes alone, for a frame in which nothing the overlay shows changed:
    /// the province schedule advanced and the manager pass of each family in `pools`, with the
    /// naval windows' lifecycle, run as a drawn frame runs them
    /// ([`counters::PreparedCounters::draw_quiet`], [`crate::map_views::Views::set_quiet`]); the
    /// batch, reports and hits the last drawn frame made stand. A family with nothing running is
    /// left as it stands: its pass has nothing to do, and the next drawn frame runs it.
    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    pub fn draw_quiet(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        layers: &OverlayLayers,
        time: f32,
        pools: viewer_foundation::map::icon_windows::Pools,
    ) -> io::Result<()> {
        let logical = gui_resources_viewport(viewport, ui_scale);
        let windows = hour.icons.counters.window_cache.at(logical.map(|v| v as u32))?;
        let inputs = CounterInputs {
            content: &hour.counters,
            equipment: Ok(hour.unit_models.sub_units.equipment_content()),
            assets: &hour.icons.counters,
            windows: &windows,
            fonts: &hour.icons.fonts,
            localisation: &hour.names.localisation,
            recorded_language: &hour.counter_content.recorded_language,
            map: &hour.map,
            hour: hour.frame.hour,
            camera,
            viewport,
            ui_scale,
            moved: hour.moved_hours.as_deref().unwrap_or(&viewer_foundation::map::counter_move::NO_MOVED_HOURS),
            movement: hour.movement.as_deref(),
            heights: hour.heights(),
            settings: self.counter_view.settings,
            display: &layers.counters,
            selected: &self.counter_view.selected,
            selected_air: &self.counter_view.air_selected,
            selected_air_bases: &self.counter_view.air_bases,
            selected_fleets: &self.counter_view.fleets_selected,
            hovered: self.counter_view.hovered,
            pressed: self.counter_view.pressed,
            plate_hovered: self.counter_view.plate_hovered,
            schedule: self.counter_view.presentation.schedule(),
            movement_history: self.counter_view.presentation.movement(),
            visibility: self.counter_view.visibility,
            art: hour.icons.lazy.version(),
        };
        inputs.schedule.lock().unwrap_or_else(|p| p.into_inner()).present(&inputs.content.bounds, camera, viewport, time);
        let (dirty, cleaning) = {
            let schedule = inputs.schedule.lock().unwrap_or_else(|p| p.into_inner());
            (self.labels.dirty(&schedule), self.labels.cleaning(&schedule))
        };
        let flags = self.icons.detached_flag_source(inputs.assets);
        let assets = inputs.assets;
        let mut views = self.counter_view.presentation.windows.lock().unwrap_or_else(|p| p.into_inner());
        views.set_quiet(true);
        viewer_foundation::map::redraws::note(
            "quiet.pools",
            pools.named().iter().filter(|(running, _)| *running).count() as u64,
        );
        let Views {
            naval,
            pings,
            bases,
            land_combats,
            labels: label_view,
            raids: raid_view,
            intel: intel_view,
            resources,
            occupation,
            adjacency_rules,
        } = &mut *views;
        label_view.set_cleaning(cleaning);
        let counters = self.icons.draw_quiet(&inputs, time, pools, naval, pings, bases, land_combats);
        let portrait_textures = |name: &[u8]| assets.operative_portrait_texture(name);
        let gui = GuiAssets {
            gui: &assets.gui,
            windows: &windows,
            localisation: inputs.localisation,
            flags: flags.as_ref().map(|source| GuiFlags { source, small: &assets.flags, large: &assets.large_flags }),
            physical_viewport: Some(logical),
            portrait_textures: Some(&portrait_textures),
        };
        let context = &Context { resources: &gui, windows: &windows, viewport: logical, time: Some(time) };
        let mut scratch = Batch::default();
        if pools.labels {
            self.labels.draw_scheduled(context, &dirty, label_view);
        }
        if pools.raids {
            let mut raid_report = self.native_raids.report.clone();
            self.native_raids.draw(context, hour.icons.raids.priority, raid_view, &mut scratch, &mut Vec::new(), &mut raid_report, self.counter_view.raid_details.as_ref(), self.counter_view.raid_hovered);
        }
        if pools.intel {
            let mut intel_report = self.native_intel.report.clone();
            self.native_intel.draw(&hour.icons.intel, context, hour.icons.raids.priority, intel_view, &mut scratch, &mut Vec::new(), &mut intel_report);
        }
        if pools.resources {
            let _ = self.resource_icons.draw(context, resources, &mut scratch);
        }
        if pools.occupation {
            let _ = self.occupation_icons.draw(context, occupation, &mut scratch, &mut Vec::new());
        }
        if pools.adjacency_rules {
            self.adjacency_icons.draw_with_hits(context, self.adjacency_priority, adjacency_rules, &mut scratch, &mut BTreeMap::new(), &mut Vec::new());
        }
        views.set_quiet(false);
        counters
    }

    /// [`PreparedOverlay::draw`] into the caller's batch, counter report and hit shapes, reusing
    /// the batch's and report's storage. On an error all three are left as they were.
    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    pub fn draw_into(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        layers: &OverlayLayers,
        time: f32,
        batch: &mut Batch,
        report: &mut CounterReport,
        hits: &mut OverlayHits,
    ) -> io::Result<()> {
        let display = &layers.counters;
        let logical = gui_resources_viewport(viewport, ui_scale);
        let windows = hour
            .icons
            .counters
            .window_cache
            .at(logical.map(|v| v as u32))?;
        let inputs = CounterInputs {
            content: &hour.counters,
            equipment: Ok(hour.unit_models.sub_units.equipment_content()),
            assets: &hour.icons.counters,
            windows: &windows,
            fonts: &hour.icons.fonts,
            localisation: &hour.names.localisation,
            recorded_language: &hour.counter_content.recorded_language,
            map: &hour.map,
            hour: hour.frame.hour,
            camera,
            viewport,
            ui_scale,
            moved: hour.moved_hours.as_deref().unwrap_or(&viewer_foundation::map::counter_move::NO_MOVED_HOURS),
            movement: hour.movement.as_deref(),
            heights: hour.heights(),
            settings: self.counter_view.settings,
            display,
            selected: &self.counter_view.selected,
            selected_air: &self.counter_view.air_selected,
            selected_air_bases: &self.counter_view.air_bases,
            selected_fleets: &self.counter_view.fleets_selected,
            hovered: self.counter_view.hovered,
            pressed: self.counter_view.pressed,
            plate_hovered: self.counter_view.plate_hovered,
            schedule: self.counter_view.presentation.schedule(),
            movement_history: self.counter_view.presentation.movement(),
            visibility: self.counter_view.visibility,
            art: hour.icons.lazy.version(),
        };
        // One advance before forking. Workers snapshot/read it, never advance it.
        let schedule_profile = viewer_foundation::profiling::span("d_schedule");
        inputs
            .schedule
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .present(&inputs.content.bounds, camera, viewport, time);
        if replay_frame::cpu::enabled() || viewer_foundation::view_record::on() {
            note_scope(&inputs.schedule.lock().unwrap_or_else(|p| p.into_inner()), &inputs.content.bounds);
        }
        let (dirty, cleaning) = {
            let schedule = inputs.schedule.lock().unwrap_or_else(|p| p.into_inner());
            (self.labels.dirty(&schedule), self.labels.cleaning(&schedule))
        };
        drop(schedule_profile);
        let flags = self.icons.detached_flag_source(inputs.assets);
        let assets = inputs.assets;
        let localisation = inputs.localisation;
        let raid_priority = hour.icons.raids.priority;
        let intel_assets = &hour.icons.intel;
        let native_raids = &self.native_raids;
        let raid_details = self.counter_view.raid_details.clone();
        let raid_hovered = self.counter_view.raid_hovered;
        let native_intel = &self.native_intel;
        let labels = &self.labels;
        let resource_icons = &self.resource_icons;
        let occupation_icons = &self.occupation_icons;
        let adjacency_icons = &self.adjacency_icons;
        let adjacency_priority = self.adjacency_priority;
        let (mut occupation_drawn, mut adjacency_drawn) = (Vec::new(), Vec::new());
        let occupation_hits = &mut occupation_drawn;
        let adjacency_hits = &mut adjacency_drawn;
        let icons = &mut self.icons;
        let mut views = self
            .counter_view
            .presentation
            .windows
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Views {
            naval,
            pings,
            bases,
            land_combats,
            labels: label_view,
            raids: raid_view,
            intel: intel_view,
            resources,
            occupation,
            adjacency_rules,
        } = &mut *views;
        label_view.set_cleaning(cleaning);
        let owner_profile = viewer_foundation::profiling::FrameOwners::new();
        let (counters, other) = (
            {
                let _span = viewer_foundation::profiling::span("d_counters");
                owner_profile.measure("counters", || {
                    icons.draw_branched(&inputs, time, owner_profile, naval, pings, bases, land_combats)
                })
            },
            {
                owner_profile.measure("other", || {
                    let portrait_textures = |name: &[u8]| assets.operative_portrait_texture(name);
                    let gui = GuiAssets {
                        gui: &assets.gui,
                        windows: &windows,
                        localisation,
                        flags: flags.as_ref().map(|source| GuiFlags {
                            source,
                            small: &assets.flags,
                            large: &assets.large_flags,
                        }),
                        physical_viewport: Some(logical),
                        portrait_textures: Some(&portrait_textures),
                    };
                    let context = &Context {
                        resources: &gui,
                        windows: &windows,
                        viewport: logical,
                        time: Some(time),
                    };
                    let (_, labels_owner) = owner_profile.measure("labels", || {
                        let _span = viewer_foundation::profiling::span("d_labels");
                        labels.draw_scheduled(context, &dirty, label_view);
                    });
                    let modes_profile = viewer_foundation::profiling::span("d_modes");
                    let (raid_quads, raids_owner) = owner_profile.measure("raids", || {
                        let mut raid_batch = Batch::default();
                        let mut raid_hits = Vec::new();
                        let mut raid_report = native_raids.report.clone();
                        native_raids.draw(
                            context,
                            raid_priority,
                            raid_view,
                            &mut raid_batch,
                            &mut raid_hits,
                            &mut raid_report,
                            raid_details.as_ref(),
                            raid_hovered,
                        );
                        (raid_batch, raid_hits, raid_report)
                    });
                    let (intel_quads, intel_owner) = owner_profile.measure("intel", || {
                        let mut intel_batch = Batch::default();
                        let mut intel_hits = Vec::new();
                        let mut intel_report = native_intel.report.clone();
                        native_intel.draw(
                            intel_assets,
                            context,
                            raid_priority,
                            intel_view,
                            &mut intel_batch,
                            &mut intel_hits,
                            &mut intel_report,
                        );
                        (intel_batch, intel_hits, intel_report)
                    });
                    let ((mut mode_icons, mut refused), resources_owner) =
                        owner_profile.measure("resources", || {
                            let mut mode_icons = Batch::default();
                            let resource_report =
                                resource_icons.draw(context, resources, &mut mode_icons);
                            (mode_icons, resource_report.refused)
                        });
                    let (_, occupation_owner) = owner_profile.measure("occupation", || {
                        let occupation_report = occupation_icons.draw(
                            context,
                            occupation,
                            &mut mode_icons,
                            occupation_hits,
                        );
                        for (reason, count) in occupation_report.refused {
                            *refused.entry(reason).or_default() += count;
                        }
                    });
                    let (_, adjacency_owner) = owner_profile.measure("adjacency", || {
                        for (reason, count) in &adjacency_icons.report {
                            *refused.entry(reason.clone()).or_default() += count;
                        }
                        adjacency_icons.draw_with_hits(
                            context,
                            adjacency_priority,
                            adjacency_rules,
                            &mut mode_icons,
                            &mut refused,
                            adjacency_hits,
                        );
                    });
                    drop(modes_profile);
                    (
                        (
                            (raid_quads, intel_quads),
                            (mode_icons, refused),
                        ),
                        [
                            labels_owner,
                            raids_owner,
                            intel_owner,
                            resources_owner,
                            occupation_owner,
                            adjacency_owner,
                        ],
                    )
                })
            },
        );
        let (counters, counters_owner) = counters;
        let ((other, other_owners), other_owner) = other;
        // The counters' own branch, when they drew branched.
        let counters_branch = counters.as_ref().ok().copied().flatten();
        for sample in [counters_owner, counters_branch, other_owner]
            .into_iter()
            .chain(other_owners)
        {
            viewer_foundation::profiling::record_owner_sample(sample);
        }
        let (raid_quads, mode_icons) = other;
        counters?;
        let (raid_quads, intel_quads) = raid_quads;
        (self.mode_icons, self.mode_icon_refused) = mode_icons;
        let raid_hits;
        let intel_hits;
        (self.raids, raid_hits, self.raid_report) = raid_quads;
        (self.intel, intel_hits, self.intel_report) = intel_quads;
        let (counters, counter_report) = self.icons.drawn();
        report.assign(counter_report);
        let labels = label_view.drawn();
        self.label_report.clone_from(label_view.report());
        let raid_range = compose(
            batch,
            &OverlayParts {
                pings: &self.icons.fixed.pings,
                bases: &self.icons.fixed.bases,
                mode_icons: &self.mode_icons,
                raids: &self.raids,
                intel: &self.intel,
                labels,
                counters,
                combats: &self.icons.fixed.combats,
                naval_combats: self.icons.drawn_combats(),
            },
            layers,
        );
        // Only the raid icons the draw laid out for hovering, drawn or hidden, can own the hover.
        let raids = if raid_range.is_some() || layers.raids_hovered { raid_hits } else { Vec::new() };
        *hits = OverlayHits { raids, intel: intel_hits, occupation: occupation_drawn, adjacency: adjacency_drawn };
        Ok(())
    }

    /// The labels' report, fixed when the overlay was laid out.
    pub fn label_report(&self) -> &LabelReport {
        &self.label_report
    }

    /// The raid icons' report, as of the icons last laid out.
    pub fn raid_report(&self) -> &raids::RaidReport {
        &self.raid_report
    }

    /// The warning-area lines of the mode icons: what their last draw refused.
    pub fn mode_icon_warnings(&self) -> Vec<String> {
        self.mode_icon_refused
            .iter()
            .map(|(reason, count)| format!("Map icons: {count} not drawn: {reason}."))
            .collect()
    }

    /// All views upload this same current secondary province field, including its empty clear.
    pub fn province_stripes(&self) -> &viewer_world::map::air_assignment::Field {
        use viewer_foundation::map::mode_visibility::PriorityTable;
        match self.counter_view.visibility.priorities {
            PriorityTable::Resistance => &self.occupation_stripes,
            PriorityTable::StrategicNavy => &self.naval_selection.assignment,
            _ => &self.air_assignment,
        }
    }

    pub fn stripe_warnings(&self) -> impl Iterator<Item = String> + '_ {
        self.occupation_stripes
            .refused
            .iter()
            .map(|(why, count)| format!("Occupation country stripes: {count} withheld: {why}"))
            .chain(self.air_assignment.refused.iter().map(|(why, count)| {
                format!("Air assignment stripes: {count} refused regions: {why}")
            }))
    }

    /// The intel icons' report, as of the last draw.
    pub fn intel_report(&self) -> &intel_icons::IntelReport {
        &self.intel_report
    }

    /// ToolTip draws after the map and combat window, with this hour's native flag atlases.
    pub fn draw_tooltip(
        &self,
        hour: &Hour<'_>,
        batch: &mut Batch,
        value: &str,
        point: TooltipPosition,
        extent: [f32; 2],
        time: f32,
    ) -> io::Result<BTreeMap<String, usize>> {
        let Ok(defines) = &hour.icons.combat.tooltip_defines else {
            return Ok(Default::default());
        };
        hour.with_gui_context(
            extent.map(|v| v as i32),
            Some(time),
            |_| Ok(self.icons.flag_source(&hour.icons.counters)),
            |context| {
                tooltip_painter_draw(
                    context.resources,
                    batch,
                    value,
                    point,
                    extent,
                    defines,
                    time,
                )
            },
        )
    }

    pub fn resource_icons_by(&self) -> MadeBy {
        self.resource_icons_by
    }

    pub fn occupation_icons_by(&self) -> MadeBy {
        self.occupation_icons_by
    }

    /// Lays out the resource icons layer's `icons`, at `version`, in place of the hour's.
    pub fn replace_resource_icons(&mut self, hour: &Hour<'_>, camera: Camera, viewport: [u32; 2], ui_scale: f32, icons: &viewer_world::map::resource_icons::HourIcons, version: u64) {
        self.resource_icons = hour.resource_icons_with(camera, viewport, ui_scale, self.counter_view.visibility, |inputs| viewer_world::map::resource_icons::lay_out(inputs, icons));
        self.resource_icons_by = MadeBy::Layer(version);
    }

    /// Lays out the occupation marks layer's `marks`, at `version`, in place of the hour's, for
    /// viewing country `country`.
    #[allow(clippy::too_many_arguments, reason = "the camera, the viewing country and the layer's product")]
    pub fn replace_occupation_icons(&mut self, hour: &Hour<'_>, camera: Camera, viewport: [u32; 2], ui_scale: f32, country: u32, marks: &viewer_world::map::occupation_icons::Marks, version: u64) {
        self.occupation_icons = hour.occupation_icons_with(camera, viewport, ui_scale, self.counter_view.occupation, country, |inputs| viewer_world::map::occupation_icons::lay_out(inputs, marks));
        self.occupation_icons_by = MadeBy::Layer(version);
    }

}

/// The overlay's parts, each in its own push order, with the windows its quads belong to.
struct OverlayParts<'a> {
    pings: &'a Batch,
    bases: &'a Batch,
    /// The map mode's resource, occupation and strait icons.
    mode_icons: &'a Batch,
    raids: &'a Batch,
    /// The Operatives map mode's state containers.
    intel: &'a Batch,
    labels: &'a Batch,
    counters: &'a Batch,
    combats: &'a Batch,
    /// The naval combat icons the view's own windows drew ([`counters::PreparedCounters`]).
    naval_combats: &'a Batch,
}

/// `batch` emptied, then `parts` in draw order: pings and bases, raid icons
/// and labels, counters and combats, leaving out the `omitted` groups (raid
/// icons go with the labels). Where the raid icons stand, when drawn.
fn compose(
    batch: &mut Batch,
    parts: &OverlayParts<'_>,
    layers: &OverlayLayers,
) -> Option<std::ops::Range<usize>> {
    batch.clear();
    if layers.pins {
        batch.append(parts.pings);
    }
    if layers.bases {
        batch.append(parts.bases);
    }
    batch.append(parts.mode_icons);
    let mut raids = None;
    if layers.raids {
        let start = batch.quads.len();
        batch.append(parts.raids);
        raids = Some(start..batch.quads.len());
    }
    batch.append(parts.intel);
    if layers.labels {
        batch.append(parts.labels);
    }
    // Each counter kind and owner was filtered as it was drawn.
    batch.append(parts.counters);
    if layers.land_combats {
        batch.append(parts.combats);
    }
    if layers.naval_markers {
        batch.append(parts.naval_combats);
    }
    raids
}

/// Which parts of the screen-space overlay a view draws. Hidden parts are still laid out, so
/// their hovers, selection and presentation windows carry on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayLayers {
    /// Faction-theatre pins.
    pub pins: bool,
    /// Base, naval HQ and facility icons.
    pub bases: bool,
    pub raids: bool,
    /// Whether raid icons keep their hover while hidden; a dump's omission removes it too.
    pub raids_hovered: bool,
    /// Capital and victory-point names and symbols.
    pub labels: bool,
    pub counters: counters::CounterDisplay,
    pub land_combats: bool,
    /// Naval combat, result and mission icons.
    pub naval_markers: bool,
}

impl Default for OverlayLayers {
    fn default() -> Self {
        Self {
            pins: true,
            bases: true,
            raids: true,
            raids_hovered: true,
            labels: true,
            counters: counters::CounterDisplay::default(),
            land_combats: true,
            naval_markers: true,
        }
    }
}

impl OverlayLayers {
    /// A view's choices; `owners` are the counter filter's canonical country slots.
    pub fn shown(
        graphics: &dashboard_core::protocol::MapGraphics,
        owners: Option<Arc<std::collections::BTreeSet<u32>>>,
    ) -> Self {
        let part = |shown| graphics.counter(shown);
        Self {
            pins: part(graphics.theatre_pins),
            bases: part(graphics.base_icons),
            raids: part(graphics.raid_icons),
            raids_hovered: true,
            labels: graphics.city_names,
            counters: counters::CounterDisplay {
                land: part(graphics.land_counters),
                railway_guns: part(graphics.railway_gun_counters),
                naval: part(graphics.naval_counters),
                ports: part(graphics.port_counters),
                owners,
            },
            land_combats: part(graphics.land_combat_markers),
            naval_markers: part(graphics.naval_markers),
        }
    }

    /// A dump's omitted groups: counters take every unit, base and combat icon; labels take
    /// raid icons and victory-point labels, and their hovers.
    pub fn omitting(omitted: Omissions) -> Self {
        let counters = !omitted.counters;
        let labels = !omitted.labels;
        Self {
            pins: counters,
            bases: counters,
            raids: labels,
            raids_hovered: labels,
            labels,
            counters: counters::CounterDisplay {
                land: counters,
                railway_guns: counters,
                naval: counters,
                ports: counters,
                owners: None,
            },
            land_combats: counters,
            naval_markers: counters,
        }
    }
}

/// Whether two quad lists are the same bit for bit.
pub fn same_quads(a: &[Quad], b: &[Quad]) -> bool {
    let bits = |values: &[f32]| {
        values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    };
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.texture == b.texture
                && a.layer == b.layer
                && a.rotation.to_bits() == b.rotation.to_bits()
                && bits(&a.rect) == bits(&b.rect)
                && bits(&a.uv) == bits(&b.uv)
                && bits(&a.color) == bits(&b.color)
        })
}

/// Preserves the selection setter order through the scene input seam.
pub fn select_movement(
    scene: &mut viewer_world::map::scene::Scene,
    device: &eframe::wgpu::Device,
    queue: &eframe::wgpu::Queue,
    content: &Content,
    frame: &DecodedFrame,
    ours: u32,
    selected: &BTreeSet<viewer_foundation::unit_handle::Unit>,
) -> std::io::Result<()> {
    use viewer_world::map::scene::SceneInput;
    let epoch = (frame.branch, frame.hour, ours);
    if !scene.movement_arrows.matches(epoch, selected) {
        let heights = HeightField {
            width: content.assets.heightmap.width,
            height: content.assets.heightmap.height,
            values: content.assets.heightmap_texels(),
        };
        let (routes, warnings) = viewer_counter_products::map::movement_routes::build(
            &content.counter_content,
            heights,
            content.map.width as f32,
            frame,
            ours,
            selected,
        );
        scene.apply(
            device,
            queue,
            SceneInput::SetMovementArrows {
                epoch,
                routes,
                warnings,
                heights,
                width: content.map.width as f32,
                selected,
            },
        )?;
    }
    scene.apply(
        device,
        queue,
        SceneInput::PrepareArrowBounds {
            map: &content.map,
            known: &content.unit_models.bounds,
        },
    )?;
    scene.apply(device, queue, SceneInput::SetUnitSelection(selected))?;
    scene.apply(
        device,
        queue,
        SceneInput::SelectProvinces(
            viewer_counter_products::map::movement_routes::selected_provinces(frame, selected),
        ),
    )?;
    Ok(())
}

impl Content {
    pub fn release_loading(&self) {
        self.loading.finish();
    }

    pub fn finish_loading(&self) -> bool {
        self.release_loading();
        if self
            .first_frame_reported
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return false;
        }
        crate::perf_log::first_complete();
        let _post_load = crate::perf_log::StartupSpan::new("content_post_load");
        if let Some(path) = std::env::var_os("BATTLE_PLAN_CONTENT_COUNTS_FILE").or_else(|| std::env::var_os("HOI4_CONTENT_COUNTS_FILE")) {
            let counts = serde_json::json!({
                "stage": "complete first hour",
                "at": crate::perf_log::epoch_ms(),
                "artwork_pending": self.icons.lazy.has_pending(),
                "reads": installed_content_core::content::io_counts::snapshot(),
                "graph": self.loading.counts(),
            });
            if let Ok(bytes) = serde_json::to_vec(&counts) {
                let _ = std::fs::write(path, bytes);
            }
        }
        true
    }
}

pub enum LoadTarget {
    Content,
    Scene,
}

impl Document {
    /// Which tables the open recording holds.
    pub fn capture_mode(&self) -> replay_data::scope::CaptureMode {
        self.playback.replay().capture_mode
    }

    pub fn load(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        visual_mods: Option<&[installed_content_core::content::visual::Choice]>,
        texture_quality: u8,
    ) -> io::Result<Self> {
        let _generation = installed_content_core::content::Generation::inherit();
        let path = fs::canonicalize(path)?;
        let replay = Arc::new(Replay::open_timelines(&path)?);
        let source = dashboard_base::Source::from_replay(
            dashboard_base::SourceKey {
                source_id: format!("native-map:{}", path.display()),
                revision: dashboard_core::protocol::replay_revision(&path)?,
            },
            path.clone(),
            Arc::clone(&replay),
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        let mut document = Self::load_shared(
            &path,
            options,
            allow_unverified,
            initial,
            replay,
            &source,
            visual_mods,
            texture_quality,
            None,
            LoadTarget::Content,
        )?;
        document.bind_source(&source);
        // The hour read before the binding could not read a native map: read it again.
        if !matches!(
            document.playback.current(),
            crate::playback::Current::Frame(_)
        ) {
            let (branch, hour) = (document.playback.branch(), document.playback.hour());
            document.playback.seek(branch, hour)?;
        }
        Ok(document)
    }

    /// Binds worker playback to the native recordings published beside `source`'s replay.
    /// It does not start preparation or read a campaign's hourly history.
    pub fn bind_source(&mut self, source: &dashboard_base::Source) {
        self.playback
            .bind_map(crate::map_binding::MapBinding::new(source));
    }

    /// Load verified presentation assets using the same immutable recording
    /// handle as the native analytics service. Call only on a loading thread.
    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    pub fn load_shared(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        replay: Arc<Replay>,
        source: &dashboard_base::Source,
        visual_mods: Option<&[installed_content_core::content::visual::Choice]>,
        texture_quality: u8,
        visual_fallback: Option<VisualFallback<'_>>,
        target: LoadTarget,
    ) -> io::Result<Self> {
        let _startup = crate::perf_log::StartupSpan::new("document").with_loaders(path);
        // A visual mod that changed while the content loaded, when a refresh would find nothing
        // loaded to check, loads it again: what installs is what the mods hold as it installs.
        let resolved = std::cell::RefCell::new({
            let _admission = crate::perf_log::StartupSpan::new("source_admission");
            source.content(options)?
        });
        let document = until_current(
            || {
                loop {
                    let mut current = Arc::clone(&resolved.borrow());
                    let valid = {
                        let _validation =
                            crate::perf_log::StartupSpan::new("source_generation_validation");
                        current.layers.generation_is_current()
                            && current.layers.validate_generation().is_ok()
                    };
                    if !valid {
                        current = source.refresh_content(options, &current)?;
                        *resolved.borrow_mut() = Arc::clone(&current);
                    }
                    let _generation = current.layers.enter_generation();
                    let _stage = viewer_foundation::load_record::stage("document");
                    let document = Self::load_shared_once(
                        path,
                        options,
                        allow_unverified,
                        initial,
                        Arc::clone(&replay),
                        Arc::clone(&current),
                        visual_mods,
                        texture_quality,
                        visual_fallback,
                    );
                    let document = match document {
                        Ok(document) => document,
                        Err(error) => {
                            let _validation =
                                crate::perf_log::StartupSpan::new("failed_generation_validation");
                            if !current.layers.generation_is_current()
                                || current.layers.validate_generation().is_err()
                            {
                                let fresh = source.refresh_content(options, &current)?;
                                *resolved.borrow_mut() = fresh;
                                continue;
                            }
                            return Err(error);
                        }
                    };
                    let revision = document.content.visual_revision.clone();
                    return Ok((document, revision));
                }
            },
            || {
                let _revision = crate::perf_log::StartupSpan::new("visual_mod_revision");
                let _stage = viewer_foundation::load_record::stage("document.revision_after_load");
                let current = resolved.borrow();
                let _generation = current.layers.enter_generation();
                visual_revision_now(options, &replay, visual_mods)
            },
        )?;
        {
            let _sources = crate::perf_log::StartupSpan::new("source_release");
            document.content.loading.release_sources();
            if matches!(target, LoadTarget::Content) {
                document.content.release_loading();
            }
        }
        Ok(document)
    }

    /// [`Self::load_shared`], once, with the recording's own executable tables in scope.
    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    fn load_shared_once(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        replay: Arc<Replay>,
        resolved: Arc<installed_content_core::content::ResolvedContent>,
        visual_mods: Option<&[installed_content_core::content::visual::Choice]>,
        texture_quality: u8,
        visual_fallback: Option<VisualFallback<'_>>,
    ) -> io::Result<Self> {
        let tables = dashboard_base::facts::recorded(&replay)
            .ok()
            .and_then(|pack| pack.tables().ok());
        engine_executable::executable_tables::scope(tables, || {
            Self::load_shared_scoped(
                path,
                options,
                allow_unverified,
                initial,
                replay,
                resolved,
                visual_mods,
                texture_quality,
                visual_fallback,
            )
        })
    }

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    fn load_shared_scoped(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        replay: Arc<Replay>,
        resolved: Arc<installed_content_core::content::ResolvedContent>,
        visual_mods: Option<&[installed_content_core::content::visual::Choice]>,
        texture_quality: u8,
        visual_fallback: Option<VisualFallback<'_>>,
    ) -> io::Result<Self> {
        let _generation = installed_content_core::content::Generation::inherit();
        let mut warnings = dashboard_base::metadata::build_warnings(&replay);
        warnings.extend(replay.compatibility_gaps());
        warnings.extend(replay.warnings.iter().cloned());
        if let Some(replay::spool::ControlRecord::Partial { reason, .. }) = replay.partial() {
            warnings.push(format!("Partial recording: {reason}"));
        }
        check_verification(
            "replay header",
            &replay.identity.verification,
            allow_unverified,
            &mut warnings,
        )?;
        for session in &replay.sessions {
            check_verification(
                &format!("session {}", session.id),
                &session.identity.verification,
                allow_unverified,
                &mut warnings,
            )?;
        }
        let language = &replay.identity.language;
        let recorded_language = language.strip_prefix("l_").unwrap_or(language).to_owned();
        // Replay::open requires every session to have the same content identity.
        // The recorded layer ids and hashes, rather than launcher state, select it.
        viewer_foundation::load_record::count("content loaded", 1, 0);
        let mut content = {
            let _stage = viewer_foundation::load_record::stage("document.recorded_stack");
            resolved.as_ref().clone()
        };
        content.layers = content.layers.with_own_overlay_diagnostics();
        content.append_changed_to(&mut warnings);
        let uncovered_reads = content.layers.report_uncovered_reads();
        let mut visual_revision = String::new();
        let mut fallback = None;
        // Visual mods mount above the recorded stack for artwork only; geometry and every
        // recorded fact keep reading the recorded layers. A selection that cannot be shown is
        // named and the recording still opens.
        if let Some(selected) = visual_mods.filter(|selected| !selected.is_empty()) {
            let recorded: Vec<_> = replay
                .identity
                .mods
                .iter()
                .map(|recorded| installed_content_core::content::visual::RecordedMod {
                    id: recorded.id.clone(),
                    name: recorded.display_name.clone(),
                })
                .collect();
            let cancel = std::sync::atomic::AtomicBool::new(false);
            // A map that failed with these visual mods, as they are now, shows what its fallback
            // decided ([`crate::visual_fallback`]).
            let decided_stage = viewer_foundation::load_record::stage("document.visual_decided");
            let decision = decided(visual_fallback, options, selected, &recorded, &cancel);
            drop(decided_stage);
            match decision {
                Some((key, decision)) => {
                    visual_revision = key.revision.clone();
                    if let Some(reason) = decision.reason() {
                        warnings.push(format!("Visual mods are not shown: {reason}."));
                    }
                    fallback = Some(crate::visual_fallback::Built {
                        key,
                        under: decision.under(),
                    });
                }
                None => match {
                    let _stage = viewer_foundation::load_record::stage("document.visual_apply");
                    installed_content_core::content::visual::apply(
                        &mut content.layers,
                        options,
                        selected,
                        &recorded,
                        &cancel,
                    )
                } {
                    Ok(plan) => {
                        if let Some((recording, decisions)) = visual_fallback {
                            let key = crate::visual_fallback::FallbackKey {
                                recording: recording.to_owned(),
                                revision: plan.revision.clone(),
                            };
                            let excused = match decisions.get(&key) {
                                Some(crate::visual_fallback::Decision::Restored { excused }) => {
                                    excused.clone()
                                }
                                _ => BTreeSet::new(),
                            };
                            fallback = Some(crate::visual_fallback::Built {
                                key,
                                under: crate::visual_fallback::Under::Mods { excused },
                            });
                        }
                        visual_revision = plan.revision;
                        if !plan.order.is_empty() {
                            warnings.push(format!(
                                "Visual mods shown over the recorded artwork, lowest precedence first: {}. Map geometry and every recorded fact come from the recording.",
                                plan.order
                                    .iter()
                                    .map(|choice| choice.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ));
                        }
                        warnings.extend(plan.warnings);
                    }
                    Err(error) => warnings.push(format!("Visual mods are not shown: {error}.")),
                },
            }
        }
        let product_loader =
            map_products::loader::Loader::new(&content.layers).with_cache(&options.cache_dir);
        Self::load_product_content(
            path,
            options,
            initial,
            visual_mods,
            texture_quality,
            replay,
            content,
            &product_loader,
            warnings,
            recorded_language,
            visual_revision,
            fallback,
            uncovered_reads,
        )
    }

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    fn load_product_content(
        path: &Path,
        options: &ResolveOptions,
        initial: &InitialHour,
        visual_mods: Option<&[installed_content_core::content::visual::Choice]>,
        texture_quality: u8,
        replay: Arc<Replay>,
        content: installed_content_core::content::ResolvedContent,
        product_loader: &map_products::loader::Loader,
        mut warnings: Vec<String>,
        recorded_language: String,
        visual_revision: String,
        fallback: Option<crate::visual_fallback::Built>,
        uncovered_reads: installed_content_core::content::UncoveredReads,
    ) -> io::Result<Self> {
        let serial = viewer_foundation::load_record::stage("content.serial");
        let content = Arc::new(content);
        std::thread::scope(|prefix| {
            let scripted_content = Arc::clone(&content);
            let scripted_language = recorded_language.clone();
            let h_scripted = installed_content_core::parallel::spawn(prefix, move || {
                let _startup = crate::perf_log::StartupSpan::new("document_scripted_text");
                let _stage = viewer_foundation::load_record::stage("content.scripted");
                Arc::new(dashboard_text::scripted_text::ScriptedContent::read(
                    scripted_content,
                    scripted_language,
                ))
            });
            let map = {
                let _stage = viewer_foundation::load_record::stage("content.map");
                product_loader.map()?
            };
            let (terrain, default_map) = {
                let _stage = viewer_foundation::load_record::stage("content.terrain");
                product_loader.terrain()?
            };
            // The game logs a malformed terrain entry or field and loads the rest.
            for reason in &terrain.skipped {
                warnings.push(format!(
                    "Terrain content the game logs and reads past: {reason}."
                ));
            }
            let defines = {
                let _stage = viewer_foundation::load_record::stage("content.defines");
                product_loader.defines()?
            };
            let audio = {
                let _stage = viewer_foundation::load_record::stage("content.audio");
                product_loader.audio(&defines, &options.user_dir, &mut warnings)
            };
            // Graphics consumers read the recorded defines with the graphics sections of the defines
            // merged natively with the visual mods' scripts (`Defines::with_graphics_of`); every
            // simulation section stays recorded, for every consumer. A graphics value of another
            // type than the recording's keeps the recorded one, and a mod's scripts that cannot run
            // keep the recorded defines, each named.
            let art_defines = {
                let _stage = viewer_foundation::load_record::stage("content.graphics_defines");
                defines.graphics_for(&content.layers)
            };
            let game_speeds = viewer_foundation::map::frame::GameSpeeds::load(&defines);
            // The defines an artwork loader reads with the stack it was handed: graphics defines
            // with the visual mods, recorded defines when it falls back to the recording.
            let defines_for = |stack: &installed_content_core::ResolvedLayers| {
                if stack.shows_visual_mods() {
                    &art_defines
                } else {
                    &defines
                }
            };
            // Every key as the game reads it (`installed_content_settings::settings::SettingsKeys`).
            let settings = installed_content_settings::settings::read_keys(&options.user_dir)?;
            let camera_tilt = viewer_foundation::map::camera::stored_tilt(
                settings
                    .f32_or_constructed(
                        &["camera_tilt"],
                        installed_content_settings::settings::constructed::CAMERA_TILT,
                    )
                    .ok_or_else(|| invalid("settings.txt camera_tilt is not a number"))?,
            );
            // The local graphics preference defaults on when no boolean value is stored.
            // The game halves the HDR scene; the output/interface remain full size.
            let game_full_resolution = settings
                .bool_at(&["maprenderingoptions", "draw_map_full_res"])
                .unwrap_or(true);
            let game_strategic_names = settings.bool_at(&["strategic_regions"]).unwrap_or(false);
            let game_audio = dashboard_core::map_settings::game_audio(&settings);
            let game_dev_master = dashboard_core::map_settings::game_dev_master(&settings);
            let cursors = crate::cursors::Cursors::load(|path| content.layers.read_uncovered(path));
            let map_defines = crate::map_assets::graphics_define(
                &content.layers,
                "camera and map",
                &art_defines,
                &defines,
                |defines| MapDefines::load(defines, camera_tilt),
            )?;
            let camera_law = crate::map_assets::graphics_define(
                &content.layers,
                "camera motion",
                &art_defines,
                &defines,
                |defines| CameraLaw::load(defines, &settings),
            )?;
            let minimap_defines = crate::map_assets::graphics_define(
                &content.layers,
                "minimap",
                &art_defines,
                &defines,
                combat::minimap::Defines::load,
            )
            .map_err(|error| error.to_string());
            // The map is drawn as this client's game would draw it: its own `force_pow2_textures`
            // pads every texture file the passes load and sets their global defines.
            let decode = settings.texture_decode();
            if settings.is_absent() {
                warnings.push(format!(
                "settings.txt is not in {}: the map uses the game's own starting settings, as the game does until it first saves them.",
                options.user_dir.display()
            ));
            }
            let (game_options, unread) = installed_content_settings::settings::MapOptions::read(&settings);
            warnings.extend(
                unread
                    .into_iter()
                    .map(|line| format!("Map options: {line}.")),
            );
            // Only the interface scale's default needs these; without them it falls
            // back to 1.0 and says so, rather than refusing the recording.
            let game_display = installed_content_settings::settings::read_pdx(&options.user_dir)
                .and_then(|pdx| {
                    installed_content_settings::settings::Display::read(
                        &settings,
                        pdx.as_ref().map(|pdx| pdx.items.as_slice()),
                    )
                })
                .map_err(|error| error.to_string());
            // The GUI viewport the map-icon windows' asset set is read at: settings.txt's own frame
            // (the last `graphics` block's `size`, else the default 1280 x 720)
            // over its scale as each block clamps it, a desktop the
            // constructor's size standing in for display 0's. That read decides only which
            // resources load, a set no viewport changes
            // (`the_asset_set_does_not_depend_on_the_gui_viewport`); every view reads its windows
            // again at its own viewport.
            let asset_viewport = {
                const CONSTRUCTED: [u32; 2] = [1280, 720];
                let (frame, scale) = match &game_display {
                    Ok(display) => {
                        let frame = display
                            .clamps
                            .iter()
                            .rev()
                            .find_map(|clamp| clamp.size)
                            .unwrap_or(CONSTRUCTED);
                        let scale =
                            display.effective_gui_scale(installed_content_settings::settings::Desktop {
                                physical: CONSTRUCTED,
                                dpi: 96,
                            });
                        (frame, display.interface_to_frame(frame, scale))
                    }
                    Err(_) => (CONSTRUCTED, 1.0),
                };
                native_shader_draw::gui_viewport(frame, scale)
            };
            // The strategic regions are parsed once for borders, counters and aircraft.
            let installed_content_regions::regions::StrategicRegions {
                regions,
                notes: region_notes,
            } = {
                let _stage = viewer_foundation::load_record::stage("content.regions");
                installed_content_regions::regions::read(&content.layers)?
            };
            warnings.extend(region_notes);
            drop(serial);
            let mut dag = installed_content_core::parallel::dag::Dag::default();
            let layers = &content.layers;
            let h_assets = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.assets");
                let _startup = crate::perf_log::StartupSpan::new("document_assets");
                let mut warnings = Vec::new();
                let assets = crate::map_assets::load_with_products(
                    Some(product_loader),
                    layers,
                    &defines,
                    &art_defines,
                    MapSize::of(&map),
                    &terrain,
                    &default_map,
                    &mut warnings,
                    texture_quality,
                    decode,
                );
                Ok((assets?, warnings))
            });
            // Artwork, interface, fonts, models and text load from the presentation stack, whose
            // visual mods show only under artwork roots, and fall back to the recording by name.
            let h_icons = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.icons");
                let _startup = crate::perf_log::StartupSpan::new("document_icons");
                layers.artwork("map icons and interface", |stack| {
                    MapIcons::load_with_products(
                        Some(product_loader),
                        stack,
                        (&defines, defines_for(stack)),
                        &map,
                        &regions,
                        &recorded_language,
                        [map.width, map.height],
                        asset_viewport,
                        decode,
                    )
                })
            });
            let h_names = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.names");
                let _startup = crate::perf_log::StartupSpan::new("document_names");
                layers.artwork("map names", |stack| {
                    NameContent::load_with_localisation(stack, &map, || {
                        product_loader.localisation(stack, &recorded_language)
                    })
                })
            });
            let h_labels = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.labels");
                Ok(Arc::new(combat::naval_labels::RecordedLabels::prepare(
                    recorded_language.clone(),
                    &recorded_language,
                    layers,
                )))
            });
            let h_counters = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.counters");
                crate::map_assets::graphics_define(
                    layers,
                    "counters",
                    &art_defines,
                    &defines,
                    |defines| {
                        CounterContent::load_with_products(
                            Some(product_loader),
                            layers,
                            layers.presentation(),
                            &map,
                            defines,
                            &regions,
                        )
                    },
                )
            });
            let h_borders_in = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.borders_in");
                let border_assets = layers.artwork("borders", |stack| {
                    BorderAssets::load(
                        stack,
                        defines_for(stack),
                        [map.width, map.height],
                        texture_quality,
                        decode,
                    )
                })?;
                let states = installed_content_states::states::read(layers, &defines, &map.definitions)?;
                let adjacencies = installed_content_core::adjacencies::read(layers)?;
                Ok((border_assets, states, adjacencies))
            });
            let h_rail_reads = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.rail_reads");
                let (cities, city_styles) = installed_content_core::cities::read(layers)?;
                let buildings_read = installed_content_core::positions::read_buildings(layers)?;
                let building_types = product_loader.buildings()?;
                let rail_defines = crate::map_assets::graphics_define(
                    layers,
                    "railways",
                    &art_defines,
                    &defines,
                    rail_layer::RailDefines::load,
                )?;
                Ok((
                    cities,
                    city_styles,
                    buildings_read,
                    building_types,
                    rail_defines,
                ))
            });
            let h_trains = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.trains");
                let (trains, warnings) = layers.artwork("supply trains", |stack| {
                    let mut warnings = Vec::new();
                    viewer_world::map::trains::TrainAssets::load(
                        stack,
                        decode,
                        defines_for(stack),
                        &mut warnings,
                    )
                    .map(|trains| (trains, warnings))
                })?;
                Ok((Arc::new(trains), warnings))
            });
            let h_aircraft = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.aircraft");
                let (aircraft, warnings) = layers.artwork("aircraft", |stack| {
                    let mut warnings = Vec::new();
                    viewer_world::map::aircraft::AircraftAssets::load(
                        stack,
                        decode,
                        &map,
                        (&defines, defines_for(stack)),
                        &regions,
                        &mut warnings,
                    )
                    .map(|aircraft| (aircraft, warnings))
                })?;
                Ok((Arc::new(aircraft), warnings))
            });
            let h_bridges = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.bridges");
                layers.artwork("railway bridges", |stack| {
                    let mut warnings = Vec::new();
                    Bridges::load(stack, decode, defines_for(stack), &mut warnings)
                        .map(|bridges| (bridges, warnings))
                })
            });
            let h_units = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.units");
                let _startup = crate::perf_log::StartupSpan::new("document_units");
                // Its refusals are its own warnings; it never fails as a whole.
                let stack = layers.presentation();
                let mut warnings = Vec::new();
                // Defines the visual mods' scripts break fall back to the recorded ones.
                let recorded = (stack.shows_visual_mods()
                    && layers.presentation_defines().is_some())
                .then_some(&defines);
                let unit_models = UnitModelAssets::load(
                    stack,
                    decode,
                    defines_for(stack),
                    recorded,
                    &map,
                    &default_map,
                    &mut warnings,
                    |refusal| layers.note_overlay_refusal(refusal),
                );
                Ok((unit_models, warnings))
            });
            let h_orders = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.orders");
                layers.artwork("orders and arrows", |stack| {
                    let mut warnings = Vec::new();
                    OrderAssets::load(stack, decode, (&defines, defines_for(stack)), &mut warnings)
                        .map(|order_assets| (order_assets, warnings))
                })
            });
            let h_weather = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.weather");
                let region_provinces = viewer_world::map::weather_effects::RegionProvinces::new(
                    &regions,
                    |province| map.definitions.contains_key(&province),
                );
                layers.artwork("weather effects", |stack| {
                    viewer_world::map::weather_effects::Assets::load(
                        stack,
                        defines_for(stack),
                        [map.width, map.height],
                        region_provinces.clone(),
                        decode,
                    )
                })
            });
            let h_mapbuildings = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_foundation::load_record::stage("wave1.mapbuildings");
                layers.artwork("map buildings", |stack| {
                    let mut warnings = Vec::new();
                    let building_layers = product_loader.reader(stack, "display/buildings");
                    MapBuildingAssets::load(
                        &building_layers,
                        decode,
                        &map,
                        defines_for(stack),
                        &mut warnings,
                    )
                    .map(|map_buildings| (map_buildings, warnings))
                })
            });
            let h_geometry = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.geometry");
                let started = Instant::now();
                let geometry = ProvinceGeometry::build(&map);
                Ok((geometry, started.elapsed()))
            });
            let h_river_assets = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.river_assets");
                layers.artwork("rivers", |stack| {
                    RiverAssets::load(stack, [map.width, map.height], texture_quality, decode)
                })
            });
            let h_rail_assets = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.rail_assets");
                layers.artwork("railways", |stack| {
                    RailAssets::load(stack, [map.width, map.height], decode)
                })
            });
            // The ideology groups supply the tactic triggers' ideology names.
            // Loading trigger keys registers those content names.
            let h_ideologies = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.ideologies");
                product_loader.ideologies()
            });
            // The resource declarations, for the tactic triggers' resource
            // names as the content is read.
            let h_resources = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.resources");
                product_loader.resources()
            });
            // The initial seek decodes from its block start to the hour, all
            // in memory; it overlaps the file reads above on its own thread.
            // Only playback moves here, so the joins below keep their order.
            let h_initial = dag.add(&[], || {
                let _stage = viewer_foundation::load_record::stage("wave1.initial");
                Self::initial_playback(replay, initial).map(std::sync::Mutex::new)
            });
            let h_initial_flags = {
                let playback = h_initial.clone();
                let icons = h_icons.clone();
                dag.add(&[playback.id(), icons.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.initial_flags");
                    if let crate::playback::Current::Frame(frame) = playback
                        .read()?
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .current()
                    {
                        let _ = counters::warm_initial_flags(frame, &icons.read()?.counters);
                    }
                    Ok(())
                })
            };
            let h_movement = {
                let (defines, regions) = (&defines, &regions);
                let h_icons = h_icons.clone();
                let h_counters = h_counters.clone();
                dag.add(&[h_icons.id(), h_counters.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.movement");
                    let icons = h_icons.read()?;
                    let counters = h_counters.read()?;

                    let mut movement = product_loader.movement(&defines)?;
                    movement.set_task_force_terrain(
                        icons.combat.movement_terrain(),
                        &regions,
                        counters.subunit_tokens.clone(),
                        icons.combat.shared_sub_units(),
                    );
                    Ok(movement)
                })
            };
            let h_label_content = {
                let (defines, art_defines) = (&defines, &art_defines);
                dag.add(&[], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.labels");
                    crate::map_assets::graphics_define(
                        layers,
                        "map labels",
                        &art_defines,
                        &defines,
                        LabelContent::load,
                    )
                })
            };
            let h_borders = {
                let (map, regions) = (&map, &regions);
                let h_assets = h_assets.clone();
                let h_borders_in = h_borders_in.clone();
                dag.add(&[h_assets.id(), h_borders_in.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.borders");
                    let (assets, _) = h_assets.read()?;
                    let heights = HeightField {
                        width: assets.heightmap.width,
                        height: assets.heightmap.height,
                        values: assets.heightmap_texels(),
                    };
                    let (_, (states, _), adjacencies) = h_borders_in.read()?;

                    let topology =
                        Topology::new(&map, states.definitions(), &regions, &adjacencies)?;
                    let straits =
                        strait_quads(&adjacencies, &map, &|province| !topology.is_water(province))?;
                    let started = Instant::now();
                    let border_meshes = borders::build(
                        &ProvinceMap {
                            width: map.width,
                            height: map.height,
                            ids: &map.province_ids,
                        },
                        &|a, b| topology.trace_class(a, b),
                        &|province| topology.is_water(province),
                        &heights,
                        map_defines.border_width,
                    )?;
                    Ok((straits, border_meshes, started.elapsed()))
                })
            };
            let h_rivers = {
                let h_assets = h_assets.clone();
                dag.add(&[h_assets.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.rivers");
                    let (assets, _) = h_assets.read()?;
                    let heights = HeightField {
                        width: assets.heightmap.width,
                        height: assets.heightmap.height,
                        values: assets.heightmap_texels(),
                    };

                    let started = Instant::now();
                    let (river_mesh, river_trace) = rivers::build(
                        &RiverImage {
                            width: assets.rivers.width,
                            height: assets.rivers.height,
                            indices: &assets.rivers.indices,
                        },
                        &heights,
                    )?;
                    Ok((river_mesh, river_trace, started.elapsed()))
                })
            };
            let h_trees = {
                let h_assets = h_assets.clone();
                dag.add(&[h_assets.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.trees");
                    let (assets, _) = h_assets.read()?;
                    let heights = HeightField {
                        width: assets.heightmap.width,
                        height: assets.heightmap.height,
                        values: assets.heightmap_texels(),
                    };

                    let started = Instant::now();
                    let tree_field = trees::generate(
                        &TreeImage {
                            width: assets.trees.width,
                            height: assets.trees.height,
                            indices: &assets.trees.indices,
                        },
                        &heights,
                    )?;
                    let (tree_assets, warnings) = layers.artwork("trees", |stack| {
                        let mut warnings = Vec::new();
                        TreeAssets::load(
                            stack,
                            decode,
                            &tree_field.palettes,
                            &assets.rivers,
                            &mut warnings,
                        )
                        .map(|trees| (trees, warnings))
                    })?;
                    let tree_geometry = trees::geometry(&tree_assets)?;
                    Ok((
                        tree_field,
                        tree_assets,
                        tree_geometry,
                        started.elapsed(),
                        warnings,
                    ))
                })
            };
            let h_sprawl = {
                let (map, defines, art_defines) = (&map, &defines, &art_defines);
                let h_assets = h_assets.clone();
                let h_rail_reads = h_rail_reads.clone();
                dag.add(&[h_assets.id(), h_rail_reads.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.sprawl");
                    let (assets, _) = h_assets.read()?;
                    let heights = HeightField {
                        width: assets.heightmap.width,
                        height: assets.heightmap.height,
                        values: assets.heightmap_texels(),
                    };
                    let (cities, city_styles, _, _, _) = h_rail_reads.read()?;

                    let started = Instant::now();
                    let sprawl_plan = sprawl::generate(
                        &cities,
                        &city_styles,
                        &assets.spawn_city,
                        &heights,
                        &map.province_ids,
                        &assets.rivers,
                    )?;
                    let (sprawl, warnings) =
                        load_sprawl(layers, decode, &sprawl_plan, (&defines, &art_defines))?;
                    Ok((sprawl, started.elapsed(), warnings))
                })
            };
            let h_railgeom = {
                let map = &map;
                let h_assets = h_assets.clone();
                let h_rail_reads = h_rail_reads.clone();
                dag.add(&[h_assets.id(), h_rail_reads.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave2.railgeom");
                    let (assets, _) = h_assets.read()?;
                    let (_, _, buildings_read, building_types, rail_defines) =
                        h_rail_reads.read()?;

                    let started = Instant::now();
                    let rail_geometry = rail_layer::RailGeometry::new(
                        &map,
                        &buildings_read,
                        &building_types,
                        &assets.rivers,
                        Arc::clone(&assets.heightmap),
                        *rail_defines,
                    )?;
                    Ok((rail_geometry, started.elapsed()))
                })
            };
            let h_frame = {
                let icons = h_icons.clone();
                dag.add(&[icons.id()], move || {
                    let _stage = viewer_foundation::load_record::stage("wave1.frame");
                    layers.artwork("map frame", |stack| {
                        let mut warnings = Vec::new();
                        FrameAssets::load(
                            stack,
                            decode,
                            icons.read()?.raids.trigger_content.facts(),
                            &mut warnings,
                        )
                        .map(|frame| (frame, warnings))
                    })
                })
            };
            let h_hover = {
                let defines = &defines;
                let icons = h_icons.clone();
                let states = h_borders_in.clone();
                let labels = h_labels.clone();
                dag.add(&[icons.id(), states.id(), labels.id()], move || {
                    let icons = icons.read()?;
                    Ok(
                        viewer_counter_products::map::province_tooltips::Content::load(
                            layers,
                            &defines,
                            states.read()?.1.0.definitions(),
                            &icons.combat.temperature,
                            icons.combat.sub_units(),
                            Arc::clone(labels.read()?),
                        ),
                    )
                })
            };
            let products_stage = viewer_foundation::load_record::stage("products");
            dag.run();
            drop(products_stage);
            let (mut assets, notes) = h_assets.take()?;
            let mut joined_warnings = notes;
            let crate::map_icons::LoadedIcons {
                combat: mut combat_assets,
                raids: mut raid_assets,
                images: mut icon_images,
                fonts: icon_fonts,
                counters: mut counter_assets,
                animated_text,
                lazy,
            } = h_icons.take()?;
            joined_warnings.extend(counter_assets.warnings.iter().cloned());
            joined_warnings.extend(
                raid_assets
                    .warnings
                    .iter()
                    .map(|(_, message)| message.clone()),
            );
            if counter_assets.flags.is_empty() {
                joined_warnings.push(
                    "No flag of the small set resolved, so unit counters carry no country flag."
                        .into(),
                );
            }
            let names = h_names.take()?;
            let recorded_labels = h_labels.take()?;
            // The map-icon windows' resources need the localisation their texts are shown
            // through, which the names pass reads.
            let icon_windows = viewer_foundation::load_record::stage("wave1.icon_windows");
            joined_warnings.extend(layers.artwork("map icon windows", |stack| {
                MapIcons::load_gui(
                    &mut combat_assets,
                    &mut counter_assets,
                    &mut icon_images,
                    stack,
                    &names.localisation,
                )
            })?);
            drop(icon_windows);
            let mut counters = h_counters.take()?;
            counters.recorded_labels = Some(Arc::clone(&recorded_labels));
            if let Err(error) = counters.fog.as_ref() {
                joined_warnings.push(format!(
                    "Fog of war cannot be rebuilt from this content ({error}): no fog is drawn and land stacks are drawn in full."
                ));
            }
            let (border_assets, states, _) = h_borders_in.take()?;
            if let Err(why) = &border_assets.faction_highlight_color {
                joined_warnings.push(format!(
                    "Faction-theatre hover outline is unavailable: {why}"
                ));
            }
            let river_assets = h_river_assets.take()?;
            let (frame_assets, frame_warnings) = h_frame.take()?;
            joined_warnings.extend(frame_warnings);
            let (_, _, _, building_types, _) = h_rail_reads.take()?;
            let (ideologies, notes) = h_ideologies.take()?;
            joined_warnings.extend(
                notes
                    .into_iter()
                    .map(|(_, warning)| format!("Combat tactics: ideologies: {warning}")),
            );
            let (resources, notes) = h_resources.take()?;
            joined_warnings.extend(
                notes
                    .into_iter()
                    .map(|(_, warning)| format!("Combat tactics: resources: {warning}")),
            );
            // Count triggers resolve these buildings, ideologies and
            // resources; the building database is the one read above. The
            // raids' triggers read the same settled names.
            raid_assets.trigger_content.triggers = Arc::new(combat_assets.set_count_trigger_names(
                &building_types,
                &ideologies,
                &resources,
            ));
            raid_assets.trigger_content.ideologies = Some(ideologies.clone());
            // A naval counter at sea's `counter_ideology` frame is its group's number.
            counters.ideologies = ideologies;
            let (trains, notes) = h_trains.take()?;
            joined_warnings.extend(notes.into_iter().map(|(_, message)| message));
            let rail_assets = h_rail_assets.take()?;
            let (bridges, notes) = h_bridges.take()?;
            joined_warnings.extend(notes.into_iter().map(|(_, message)| message));
            let (aircraft, notes) = h_aircraft.take()?;
            joined_warnings.extend(notes.into_iter().map(|(_, message)| message));
            let (unit_models, notes) = h_units.take()?;
            joined_warnings.extend(notes);
            let (order_assets, notes) = h_orders.take()?;
            joined_warnings.extend(notes.into_iter().map(|(_, message)| message));
            let weather_effects = h_weather.take()?;
            joined_warnings.extend(
                weather_effects
                    .withheld
                    .iter()
                    .map(|why| format!("Withheld, draws nothing: {why}")),
            );
            let (map_buildings, notes) = h_mapbuildings.take()?;
            joined_warnings.extend(notes.into_iter().map(|(_, message)| message));
            let (province_geometry, province_geometry_build) = h_geometry.take()?;
            let playback = h_initial
                .take()?
                .into_inner()
                .unwrap_or_else(|error| error.into_inner());
            warnings.extend(joined_warnings);
            let (states, state_conflicts) = states;
            warnings.extend(state_conflicts.into_iter().map(|(_, message)| message));
            assets.post.volumes.set_regions(&map, &regions);
            let heights = HeightField {
                width: assets.heightmap.width,
                height: assets.heightmap.height,
                values: assets.heightmap_texels(),
            };
            // The unit-stack slots `map/unitstacks.txt` leaves out.
            counters.seed_default_anchors(
                map.definitions.keys().copied(),
                |province| province_geometry.centroid(province),
                &heights,
            );
            if !counters.missing_standstill.is_empty() {
                warnings.push(format!(
                "{} provinces have neither a Standstill unit stack in map/unitstacks.txt nor pixels for the engine's centroid default; a unit counter in one of them is not drawn.",
                counters.missing_standstill.len()
            ));
            }
            // Region icon positions also anchor the faction-theatre pins.
            let unplaced = counters.place_region_icons(
                |province| province_geometry.centroid(province),
                map.width,
                &heights,
            );
            if let Some(first) = unplaced.first() {
                warnings.push(format!(
                "{} strategic regions have no provinces or a province without pixels, so they have no icon position and a faction-theatre pin averaging one is not drawn; the first is {first}.",
                unplaced.len()
            ));
            }
            let labels = h_label_content.take()?;
            let (straits, border_meshes, border_trace) = h_borders.take()?;
            let border_meshes = Arc::new(border_meshes);
            let (river_mesh, river_trace, river_build) = h_rivers.take()?;
            let (tree_field, tree_assets, tree_geometry, tree_build, notes) = h_trees.take()?;
            let mut wave2_warnings = notes;
            let (sprawl, sprawl_build, notes) = h_sprawl.take()?;
            wave2_warnings.extend(notes);
            let (rail_geometry, rail_build) = h_railgeom.take()?;
            // The movement map failing refuses glide, stack kinds and ETAs,
            // not the document: moving units then stand at their province.
            let movement = match h_movement.take() {
                Ok(movement) => {
                    let unavailable = movement.map.unavailable_edges();
                    if let Some(((a, b), reason)) = unavailable.iter().next() {
                        wave2_warnings.push(format!(
                            "{} neighbouring province pairs have no content movement distance, so routes over them have no progress or ETA; the first is {a}-{b}: {reason}",
                            unavailable.len()
                        ));
                    }
                    Some(Arc::new(movement))
                }
                Err(error) => {
                    wave2_warnings.push(format!(
                        "Unit movement is unavailable for this content: {error}. Moving units stand at their province, and no route point or ETA is drawn."
                    ));
                    None
                }
            };
            h_initial_flags.take()?;
            warnings.extend(wave2_warnings);
            let _rest = viewer_foundation::load_record::stage("content.rest");
            // Raid `is_coastal` reads the movement map's static province terrain.
            raid_assets.trigger_content.terrain = movement.clone();
            let map_modes = {
                let mode_layers = product_loader.reader(&content.layers, "display/modes");
                let scripted_modes = installed_content_core::scripted_map_modes::read(&mode_layers)?;
                for (field, count) in scripted_modes.refusal_counts() {
                    warnings.push(format!(
                        "Scripted map modes: {count} unread entries {field}"
                    ));
                }
                let categories = installed_content_state_categories::state_categories::read(&mode_layers)?;
                let infrastructure =
                    viewer_world::map::map_mode::infrastructure::Content::new(&building_types);
                let terrain_categories = combat_assets.movement_terrain();
                let provinces: Vec<u32> = map.definitions.keys().copied().collect();
                let province_terrain = |province| {
                    movement
                        .as_ref()
                        .and_then(|movement| movement.map.terrain(province))
                };
                // The map modes' colours and ranges are graphics defines: the mods' where they read.
                let ideologies = Arc::new(counters.ideologies.clone());
                Arc::new(crate::map_assets::graphics_define(
                    &mode_layers,
                    "map modes",
                    &art_defines,
                    &defines,
                    |defines| {
                        let terrain = match (&terrain_categories, &movement) {
                        (Err(why), _) => Err(format!("the terrain database did not read: {why}")),
                        (_, None) => Err(
                            "the provinces' terrain records did not read (unit movement is unavailable)"
                                .to_owned(),
                        ),
                        (Ok(categories), Some(_)) => Ok(viewer_world::map::map_mode::terrain::Sources {
                            categories,
                            provinces: &provinces,
                            terrain: &province_terrain,
                            region_of: &counters.region_of,
                            regions: &regions,
                        }),
                    };
                        viewer_world::map::map_mode::Content::new(
                            defines,
                            Arc::clone(&ideologies),
                            &categories,
                            scripted_modes.clone(),
                            mode_layers.faction_templates(),
                            recorded_language.clone(),
                            terrain,
                            infrastructure.clone(),
                            viewer_world::map::map_mode::strategic::Content::new(
                                defines, &regions,
                            )?,
                        )
                    },
                )?)
            };
            let scripted = installed_content_core::parallel::join(h_scripted);
            // The intel icons' content, and the sprites their code sets that no template names.
            let mut intel_assets = intel_icons::IntelAssets::load(&content.layers, &defines);
            intel_assets.scripted = Some(Arc::clone(&scripted));
            warnings.extend(intel_assets.warnings.iter().cloned());
            let mut counter_assets = counter_assets;
            let mut icon_images = icon_images;
            // The occupation catalogue reads settled script and modifier names after the content
            // loaders above. Runtime badge sprites and inline trend icons share the native GUI.
            let occupation = crate::map_assets::graphics_define(
                &content.layers,
                "occupation map icons",
                &art_defines,
                &defines,
                viewer_world::map::occupation_icons::IconDefines::load,
            )
            .and_then(|defines| {
                Ok(viewer_world::map::occupation_icons::Content {
                    definitions: installed_content_core::occupation_modifiers::Definitions::read(
                        &content.layers,
                    )?,
                    defines,
                })
            })
            .map_err(|error| error.to_string());
            match &occupation {
                Ok(occupation) => {
                    warnings.extend(occupation.warnings().cloned());
                    counter_assets.gui.load(
                        &content.layers,
                        &viewer_world::map::occupation_icons::gui_assets(occupation),
                        &mut icon_images,
                    )?;
                }
                Err(why) => {
                    warnings.push(format!("Occupation map icons: 1 content refusal: {why}"))
                }
            }
            let intel_sprites = intel_assets.gui_assets(counter_assets.gui.registry());
            counter_assets
                .gui
                .load(&content.layers, &intel_sprites, &mut icon_images)?;
            if icon_images.len() >= viewer_foundation::map::lazy_icons::DYNAMIC_BASE {
                return Err(invalid("more static icon pages than texture ids"));
            }
            let icons = MapIcons {
                combat: Arc::new(combat_assets),
                raids: Arc::new(raid_assets),
                intel: Arc::new(intel_assets),
                occupation,
                images: icon_images,
                fonts: icon_fonts,
                counters: Arc::new(counter_assets),
                animated_text: Arc::new(animated_text),
                lazy,
            };
            warnings.push(
            "Unit counters are drawn for the viewing country: with fog of war on, a stack it cannot see carries the game's own question-mark icon, a question mark in place of its count and no bars. Which stacks those are is decided from the recorded faction, war, subject and UnitIntel tables, and for a land stack from the greater of its detection level and the province byte the viewer rebuilds for the viewing country from the recorded simulation. An hour missing a unit-sight table, or whose province fog cannot be rebuilt, states so and draws those stacks in full. --fog off, the game's fow console switch, draws every stack in full, still as the viewing country.".into(),
        );
            warnings.push(
            "A naval counter draws both bars full. The engine averages each task force's own ratio over the fleet's visible members. Strength ratios need a maximum bounded below by 100, and organisation ratios need a live modifier-derived maximum; the Ships table records the two current values and neither maximum, so a damaged fleet's bars are drawn full rather than guessed against the fleet's own best ship. The capital-ship diamond is drawn, but nonempty capital counts, type icons, carrier air bases and supply crates require unrecorded ship flags, sprite keys, carrier registration and mission/supply state.".into(),
        );
            warnings.push(
            "An air base draws no count beside it, and neither does a naval base. The engine fills the air base's wings_count with the planes based there over the base's capacity, and the naval base's txt_capacity with its own pair, but both blocks sit behind the map-mode selector — 2 for the air map mode and 1 for the naval one — and its other branch hides both boxes. The ordinary map mode hides both base count boxes. What the air box would say is computed and reported: the planes, the capacity, and whether the viewing country is shown the box at all, which needs a container at that base (recorded as AirBaseRegistrations), a state that is not demilitarised (recorded as StateFlags) and a positive container capacity word (recorded with the registration).".into(),
        );
            warnings.push(
            "Raid map icons are rebuilt for the viewing country from the recorded raids, raid target statuses, allowed raid types (the recorded player cache list, for the cache's own country; every other country's `allowed` evaluated at the shown hour, where an undecided one draws its offers grey and counted) and used ones, cooldowns and the raid scripts' inputs, as the game rebuilds its player cache when the player changes. An icon whose script input is unrecorded or unsupported is withheld and counted, and an offer is drawn enabled only when its launch gate is proved from recorded command power, stockpiles, units, sources and ranges; an offer the gate cannot decide, such as one waiting on an army or naval path search, is drawn grey and counted. A table the recorder reads daily or by slice is read as the recording last holds it. Offers are the player cache's rows: each built hour evaluates the next NRaids.MAX_TARGETS_TO_UPDATE_PER_FRAME targets from a rolling cursor, a paused hour one batch per frame until every target is current, and every other target shows its last evaluation. A seek, a step back, a branch change, a new recording or viewing country starts the cycle again at row 0, and a target not evaluated since draws nothing.".into(),
        );
            warnings.push(
            "Counter items use the first admitted division. Source icons are province/visual buckets, and geographic columns rebuild their unit lists with the engine's first-fit predicate. Province occupant order and alternate templates use recorded inputs. Missing rows and unrecorded presentation insertion history can change plate order, grouping, flags and icons.".into(),
        );
            // Labels read the same finalised unit-stack table, slot 38.
            let mut labels = labels;
            labels.set_anchors(
                map.definitions.keys().copied(),
                |province| counters.anchor(province, labels::ANCHOR_STACK),
                |province| counters.seeded_anchor(province, labels::ANCHOR_STACK),
            );
            if !labels.centroid_fallbacks.is_empty() {
                warnings.push(format!(
                "{} provinces author neither a victory-point nor a defending unit stack, so their label stands at the engine's seeded default, the integer pixel centroid.",
                labels.centroid_fallbacks.len()
            ));
            }
            // borders
            if !border_meshes.one_pixel_provinces.is_empty() {
                warnings.push(format!(
                "{} provinces cover a single pixel of map/provinces.bmp; the game reports these too and no border is traced through them.",
                border_meshes.one_pixel_provinces.len()
            ));
            }
            // rivers
            if river_trace.unreachable > 0 {
                warnings.push(format!(
                "{} river pixels of map/rivers.bmp are not reachable from any source or junction marker and are not drawn; the game does not draw them either.",
                river_trace.unreachable
            ));
            }
            if !river_trace.too_small.is_empty() {
                warnings.push(format!(
                "{} rivers of map/rivers.bmp are shorter than {} pixels; the game also rejects these short rivers and draws none of them.",
                river_trace.too_small.len(),
                rivers::MIN_RIVER_POINTS
            ));
            }
            // the map's table frame
            // trees
            warnings.push(
            "Meshes, animations and textures are read from gfx/models and gfx/entities, which are outside the content coverage policy and are not hash-verified; their .gfx and .asset descriptors are verified.".into(),
        );
            // close zoom
            warnings.push("Railway geography limitations: tied crossing inputs and the initial tied-pixel collection order are unrecorded. Province visibility history and the railway manager's hash-table draw order are unrecorded; strips use all provinces in ascending order. Port selection uses recorded naval-base levels instead of strategic-navy membership. Custom adjacency overrides and the transient rail fade are not reconstructed; matching native geography and presentation snapshots would settle these cases.".into());
            warnings.push(
            "A division's model follows the engine's own ladder: the template's saved model, then the equipment pair's saved model, authored graphic pools and conventional candidates, else the viewing country's fallback queues, then the recorded model number and sub-unit key under the cosmetic tag, tag, graphical culture and bare name, then the recorded camouflage. Template composition and model choices are recorded hourly, so a template edit reaches its figures within the hour; a template whose model inputs were unreadable that hour draws no model for its divisions, and its counters are unaffected. Equipment is recorded daily, with an hourly record of divisions deployed since; an existing division's mid-day equipment change reaches its figure at the next daily refresh; the country's stored best-variant candidates are daily too. A no-pair fallback whose rows name different entities takes the second row of the game's labelled option list, its labels built in the display language from the recorded variant names and country tags; a label that cannot be built refuses the figure by name. The window advances authored state clips and attachment transforms with presentation seed 0x414e494d XOR the unit handle; original clocks and RNG ordering are unrecorded. A time-zero dump preserves the default first pose. A standing figure's facing is a hash, because nothing records the heading the engine last turned it to."
                .into(),
        );
            // orders: the recorded player's battle plan
            warnings.push(
            "Map buildings keep the destroyed-mesh hysteresis, and stay drawn above the update distance once created below it, from this viewer's own camera history since the last seek; a seek starts every slot healthy and uncreated, as a freshly loaded game does. A dump has no camera history unless --building-camera supplies it. Popup growth and the shared model creation allowance run on the viewer's frame clock; native capture-time presentation remains unrecorded."
                .into(),
        );
            // Validate province membership before rebuilding region battle-name candidates.
            let battle_name_provinces =
                battle_name_candidates(&regions, states.definitions(), &|province| {
                    map.definitions.contains_key(&province)
                });
            let state_name_keys = states
                .definitions()
                .iter()
                .map(|(id, state)| (*id, state.name_key.clone()))
                .collect();
            let province_hover = h_hover.take()?;
            warnings.extend(province_hover.warnings());
            let naval_dominance = viewer_world::map::naval_dominance::Content::new(
                &map,
                &regions,
                icons
                    .combat
                    .movement_terrain()
                    .as_deref()
                    .map_err(String::as_str),
                &defines,
            );
            let raid_hover = Arc::new(raids::tooltips::Content {
                assets: Arc::clone(&icons.raids),
                flags: Arc::clone(&icons.counters.flags),
                localisation: Arc::clone(&names.localisation),
                recorded_language: recorded_language.clone(),
                scripted: Arc::clone(&scripted),
            });
            let map = Arc::new(map);
            let province_geometry = Arc::new(province_geometry);
            let battle_name_provinces = Arc::new(battle_name_provinces);
            let state_name_keys = Arc::new(state_name_keys);
            let eta_cache = Arc::new(EtaCache::default());
            let vision_memo = Arc::new(Keyed::new(perspective::VISION_TABLES));
            let modifier_names = Arc::new(combat::modifier_text::name_index_cache());
            let regions = Arc::new(regions);
            let counters = Arc::new(counters);
            let province_hover = Arc::new(province_hover);
            let counter_content =
                Arc::new(viewer_counter_products::map::counter_content::Content {
                    anchors: Arc::clone(&counters.anchors),
                    default_anchors: Arc::clone(&counters.default_anchors),
                    terrain: Arc::clone(&counters.terrain),
                    region_of: Arc::clone(&counters.region_of),
                    kinds: Arc::clone(&counters.kinds),
                    ability_sprites: Arc::clone(&icons.counters.ability_sprites),
                    fog: Arc::clone(&counters.fog),
                    aggression: Arc::clone(&counters.aggression),
                    naval_counts: counters.defines.naval_counts,
                    port_mission_defines: counters.port_mission_defines.clone(),
                    supply_grace: counters.supply_grace.clone(),
                    comms_max_distance: counters.comms_max_distance.clone(),
                    dig_in_factor: counters.dig_in_factor.clone(),
                    attrition: counters.attrition.clone(),
                    ability_comms_scaling: counters.ability_comms_scaling.clone(),
                    ability_no_hq_scaling: counters.ability_no_hq_scaling.clone(),
                    equipment: Arc::clone(&unit_models.sub_units.equipment),
                    strategic_regions: Arc::clone(&regions),
                    province_hover: Arc::clone(&province_hover),
                    localisation: Arc::clone(&names.localisation),
                    recorded_language: recorded_language.clone(),
                    scripted: Arc::clone(&scripted),
                    state_name_keys: Arc::clone(&state_name_keys),
                    text_icons: icons.combat.clone(),
                    division_names: Arc::clone(&icons.combat.division_names),
                    sub_units: icons.combat.shared_sub_units(),
                    movement_terrain: icons.combat.movement_terrain(),
                    building_facts: Arc::clone(&map_buildings.facts),
                    air_base_first_provinces: Arc::clone(&counters.air_base_first_provinces),
                    movement: movement.clone(),
                    eta_cache: Arc::clone(&eta_cache),
                });
            let combat = Arc::new(combat::Content {
                air_context: Some(Arc::clone(&counter_content)),
                medals: Default::default(),
                attrition: Default::default(),
                assets: Arc::clone(&icons.combat),
                localisation: Arc::clone(&names.localisation),
                recorded_language: recorded_language.clone(),
                recorded_labels: Arc::clone(&recorded_labels),
                scripted: Arc::clone(&scripted),
                map: Arc::clone(&map),
                strategic_regions: Arc::clone(&regions),
                battle_name_provinces: Arc::clone(&battle_name_provinces),
                state_name_keys: Arc::clone(&state_name_keys),
                province_geometry: Arc::clone(&province_geometry),
                region_of: Arc::clone(&counters.region_of),
                terrain: Arc::clone(&counters.terrain),
                scores: counters.defines.land_combat.scores,
                fog: Arc::clone(&counters.fog),
                vision_memo: Arc::clone(&vision_memo),
                eta_cache: Arc::clone(&eta_cache),
                modifier_names: Arc::clone(&modifier_names),
                movement: movement.clone(),
                window_cache: Arc::clone(&icons.counters.window_cache),
                small_flags: Arc::clone(&icons.counters.flags),
                large_flags: Arc::clone(&icons.counters.large_flags),
                lazy: Arc::clone(&icons.lazy),
                trigger_content: Arc::new(icons.raids.trigger_content.clone()),
                occupation: Arc::new(combat::Occupation::load(&content.layers)),
            });
            let resource_icons = viewer_world::map::resource_icons::ResourceIconContent::load(
                &content.layers,
                &defines,
            )
            .map_err(|error| error.to_string());
            match &resource_icons {
                Ok(icons) => warnings.extend(icons.warnings().iter().cloned()),
                Err(why) => warnings.push(format!(
                    "The resources map mode draws no state icons: {why}"
                )),
            }
            let adjacency_icons = match installed_content_core::adjacency_rules::read(&content.layers) {
                Ok(rules) => {
                    viewer_world::map::adjacency_icons::AdjacencyIcons::new(&rules, &defines)
                }
                Err(error) => {
                    warnings.push(format!(
                    "No strait or canal icons are drawn: the adjacency rules cannot be read: {error}"
                ));
                    viewer_world::map::adjacency_icons::AdjacencyIcons::new(&[], &defines)
                }
            };
            let trade_routes = crate::map_assets::graphics_define(
                &content.layers,
                "trade-route lines and convoys",
                &art_defines,
                &defines,
                viewer_world::map::trade_routes::TradeRouteDefines::read,
            )
            .map_err(|error| error.to_string());
            let air_assignment_colors = crate::map_assets::graphics_define(
                &content.layers,
                "Air assignment stripes",
                &art_defines,
                &defines,
                viewer_world::map::air_assignment::Colors::read,
            )
            .map_err(|error| error.to_string());
            let occupation_stripe_alpha = crate::map_assets::graphics_define(
                &content.layers,
                "occupation country stripes",
                &art_defines,
                &defines,
                viewer_world::map::occupation_stripes::Alpha::read,
            )
            .map_err(|error| error.to_string());
            let naval_region_eligibility = viewer_world::map::naval_selection::eligible_regions(
                &icons.raids.region_provinces,
                &icons.raids.geometry,
            );
            let naval_selection_colors = crate::map_assets::graphics_define(
                &content.layers,
                "Navy local selection colours",
                &art_defines,
                &defines,
                viewer_world::map::naval_selection::Colors::read,
            )
            .map_err(|error| error.to_string());
            let radar_centers = viewer_world::map::radar_symbols::Centers::new(
                &icons.raids.trigger_content.state_provinces,
                &counters.bounds,
                map.width as i32,
            );
            let air_order_colors = crate::map_assets::graphics_define(
                &content.layers,
                "Air mission arrows",
                &art_defines,
                &defines,
                viewer_world::map::air_orders::Colors::read,
            )
            .map_err(|error| error.to_string());
            let air_transfer_color = crate::map_assets::graphics_define(
                &content.layers,
                "Air transfer arrows",
                &art_defines,
                &defines,
                viewer_world::map::air_transfers::color,
            )
            .map_err(|error| error.to_string());
            let snow_content = map_products::display::SnowContent::new(
                &map,
                &assets.perm_snow,
                assets.post.volumes.winter(),
            );
            let products = map_products::ProductContent::from_inputs(
                product_loader.reads.clone(),
                map_products::ProductInputs {
                    map: Arc::clone(&map),
                    fog: Arc::clone(&counters.fog),
                    anchors: Arc::clone(&counters.anchors),
                    default_anchors: Arc::clone(&counters.default_anchors),
                    bounds: Arc::clone(&counters.bounds),
                    localisation: Arc::clone(&names.localisation),
                    map_text: viewer_world::map::animated_map_text::events::Content::new(
                        &building_types,
                    ),
                    audio: audio.as_ref().map(|source| Arc::clone(&source.content)),
                    triggers: Arc::new(icons.raids.trigger_content.clone()),
                },
            )
            .with_snow(snow_content);
            {
                let _validation = crate::perf_log::StartupSpan::new("generation_validation");
                content.layers.validate_generation()?;
            }
            let loading = content.layers.loading();
            if let Some(path) = std::env::var_os("BATTLE_PLAN_CONTENT_COUNTS_FILE").or_else(|| std::env::var_os("HOI4_CONTENT_COUNTS_FILE")) {
                let counts = serde_json::json!({
                    "reads": installed_content_core::content::io_counts::snapshot(),
                    "graph": content.layers.content_counts(),
                });
                std::fs::write(path, serde_json::to_vec(&counts).map_err(io::Error::other)?)?;
            }
            let content = Content {
                loading,
                first_frame_reported: std::sync::atomic::AtomicBool::new(false),
                tables: engine_executable::executable_tables::current(),
                products,
                radar_centers,
                resource_icons,
                adjacency_icons,
                trade_routes,
                air_assignment_colors,
                occupation_stripe_alpha,
                naval_selection_colors,
                naval_region_eligibility,
                air_order_colors,
                air_transfer_color,
                counter_content,
                combat,
                visual_mods: visual_mods.map(<[installed_content_core::content::visual::Choice]>::to_vec),
                visual_revision,
                fallback,
                texture_quality,
                weather_effects,
                map,
                map_defines,
                camera_law,
                minimap_defines,
                game_speeds,
                game_display,
                game_options,
                game_full_resolution,
                game_strategic_names,
                game_audio,
                game_dev_master,
                cursors,
                assets,
                warnings,
                uncovered_reads,
                audio,
                source_path: path.to_path_buf(),
                // borders
                border_assets,
                border_meshes,
                straits,
                raid_hover,
                strategic_regions: regions,
                naval_dominance,
                map_modes,
                border_trace,
                // names
                icons,
                names,
                labels,
                // rivers
                river_assets,
                river_mesh,
                river_trace,
                river_build,
                // the map's table frame
                frame_assets,
                // counters
                counters,
                // trees
                tree_assets,
                tree_geometry,
                tree_field,
                tree_build,
                // close zoom
                sprawl,
                sprawl_build,
                map_buildings,
                rail_assets,
                rail_geometry,
                trains,
                aircraft,
                rail_build,
                bridges,
                unit_models,
                // orders: the recorded player's battle plan
                orders: order_assets,
                political_gpu: viewer_world::map::political_gpu::Cache::default(),
                scene_textures: viewer_world::map::scene::TextureCache::default(),
                eta_cache,
                order_memo: Recent::new(),
                trade_route_memo: Recent::new(),
                trade_route_stops: std::sync::Mutex::default(),
                type_tint_memo: Keyed::new(counters::type_tint::TABLES),
                mission_memo: Recent::new(),
                overlay_carry: counters::Carry::default(),
                movement,
                province_geometry,
                province_geometry_build,
            };
            Ok(Self {
                content: Arc::new(content),
                playback,
                hour_fraction: initial.hour_fraction,
                demilitarized_override: BTreeSet::new(),
            })
        })
    }

    /// Presentation state over verified content; the coordinator supplies the
    /// first decoded frame without synchronous work on the UI thread.
    pub fn from_content(content: Arc<Content>, replay: Arc<Replay>) -> io::Result<Self> {
        Ok(Self {
            content,
            playback: Playback::unloaded(replay)?,
            hour_fraction: 0.0,
            demilitarized_override: BTreeSet::new(),
        })
    }

    /// A Campaign map selection over content already held by another native map.
    /// Like `load_shared`, this performs its initial seek on a loading thread.
    pub fn from_shared_content(
        content: Arc<Content>,
        replay: Arc<Replay>,
        initial: &InitialHour,
    ) -> io::Result<Self> {
        Ok(Self {
            content,
            playback: Self::initial_playback(replay, initial)?,
            hour_fraction: initial.hour_fraction,
            demilitarized_override: BTreeSet::new(),
        })
    }

    fn initial_playback(replay: Arc<Replay>, initial: &InitialHour) -> io::Result<Playback> {
        let mut playback = Playback::shared(replay)?;
        if let Some(hour) = initial.hour {
            let branch = if let Some(branch) = initial.branch {
                branch
            } else {
                let mut candidates = Vec::new();
                // Histories share the hours before they part: the main one answers for them.
                let main = playback
                    .replay()
                    .timelines()
                    .map(|timelines| timelines.main);
                if let Some(main) = main
                    && matches!(playback.replay().seek(main, hour)?, SeekResult::Frame(_))
                {
                    candidates.push(main);
                }
                for branch in &playback.replay().branches {
                    if Some(branch.id) == main || candidates.first() == main.as_ref() {
                        continue;
                    }
                    if matches!(
                        playback.replay().seek(branch.id, hour)?,
                        SeekResult::Frame(_)
                    ) {
                        candidates.push(branch.id);
                    }
                }
                match candidates.as_slice() {
                    [branch] => *branch,
                    [] => {
                        return Err(invalid(format!(
                            "no recorded frame at hour {hour}; pass --branch to inspect a gap"
                        )));
                    }
                    _ => {
                        return Err(invalid(format!(
                            "hour {hour} occurs on branches {candidates:?}; pass --branch"
                        )));
                    }
                }
            };
            playback.seek(branch, hour)?;
        } else if let Some(branch) = initial.branch {
            playback.select_branch(branch)?;
        }
        Ok(playback)
    }

    /// The hour on screen, for everything built per hour.
    pub fn hour(&self) -> io::Result<Hour<'_>> {
        Ok(Hour::new(&self.content, self.frame()?, self.hour_fraction)
            .with_air_rosters(Some(self.playback.air_rosters())))
    }

    /// What this hour's counters can see, for the combat windows and the
    /// limitation lines.
    pub fn vision(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<Arc<perspective::Vision>> {
        let _cost = replay_frame::cpu::scope("viewer", Some("fog"));
        self.hour()?.vision(perspective)
    }

    /// The viewing country's tag at the hour on screen.
    pub fn viewing_tag(&self, vision: &perspective::Vision) -> io::Result<String> {
        self.hour()?.viewing_tag(vision)
    }

    /// The cold paths a `--dump` takes; the window reads the same facts through the map's
    /// layers instead.
    pub fn limitations(&self, perspective: &perspective::Choice) -> io::Result<Limitations> {
        self.hour()?.limitations(perspective)
    }

    pub fn snow_mud(&self) -> io::Result<SnowMud> {
        self.hour()?.snow_mud()
    }

    pub fn map_building_field(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<(
        map_display_data::buildings::Buildings,
        viewer_world::map::map_buildings::MapBuildingSummary,
    )> {
        self.hour()?.map_building_field(perspective)
    }

    pub fn aircraft_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_world::map::aircraft::AircraftPlan> {
        let _cost = replay_frame::cpu::scope("viewer", Some("air/naval source choices"));
        self.hour()?.aircraft_plan(perspective)
    }

    pub fn nuclear_impacts(&self) -> io::Result<viewer_world::map::strike_effects::Impacts> {
        let _cost = replay_frame::cpu::scope("viewer", Some("engagement/strikes/text"));
        self.hour()?.nuclear_impacts()
    }

    pub fn train_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_world::map::trains::TrainPlan> {
        let _cost = replay_frame::cpu::scope("viewer", Some("rails"));
        self.hour()?.train_plan(perspective)
    }

    pub fn order_field(&self, perspective: &perspective::Choice) -> io::Result<Arc<OrderField>> {
        self.hour()?.order_field(perspective)
    }

    pub fn order_field_with_allied(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
    ) -> io::Result<Arc<OrderField>> {
        self.hour()?
            .order_field_with_allied(perspective, show_allied)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn unit_model_field(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<(viewer_world::map::unit_history::Field, ModelReport)> {
        self.hour()?
            .unit_model_field_sailing(perspective, 0.0, &|_| false)
    }

    /// See [`Hour::unit_model_field_sailing`].
    pub fn unit_model_field_sailing(
        &self,
        perspective: &perspective::Choice,
        time: f32,
        shown: &dyn Fn(&viewer_world::map::trade_routes::ConvoyDefines) -> bool,
    ) -> io::Result<(viewer_world::map::unit_history::Field, ModelReport)> {
        self.hour()?
            .unit_model_field_sailing(perspective, time, shown)
    }

    pub fn intel_map(&self, perspective: &perspective::Choice) -> io::Result<IntelMap> {
        self.hour()?.intel_map(perspective)
    }

    pub fn place_names(
        &self,
        mode: &viewer_world::map::map_mode::MapMode,
    ) -> io::Result<Placements> {
        self.hour()?.place_names(mode)
    }

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    pub fn dump_overlay(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        perspective: &perspective::Choice,
        omitted: Omissions,
        counter_settings: counters::CounterSettings,
        mode: &viewer_world::map::map_mode::MapMode,
    ) -> io::Result<DumpOverlay<'_>> {
        self.hour()?.dump_overlay(
            camera,
            viewport,
            ui_scale,
            perspective,
            omitted,
            counter_settings,
            mode,
        )
    }

    pub fn rail_mesh(&self) -> io::Result<RailMesh> {
        self.hour()?.rail_mesh()
    }

    pub fn demilitarized_zone(&self, regions: &Regions) -> io::Result<Zone> {
        let override_ids = self.demilitarized_override.clone();
        self.hour()?.demilitarized_zone(regions, &override_ids)
    }

    /// The decoded frame of the hour on screen, or a named error for a recorded gap or a refused
    /// hour.
    pub fn frame(&self) -> io::Result<&DecodedFrame> {
        match self.playback.current() {
            crate::playback::Current::Frame(frame) => Ok(frame),
            crate::playback::Current::Gap {
                branch,
                hour,
                reason,
            } => Err(invalid(format!(
                "branch {branch}, hour {hour} is a gap: {reason}"
            ))),
            crate::playback::Current::Refused {
                branch,
                hour,
                reason,
            } => Err(invalid(format!(
                "branch {branch}, hour {hour} cannot be shown: {reason}"
            ))),
        }
    }
}

/// The hour's camera-independent overlay tables, built once per hour and
/// perspective ([`Hour::hour_tables`]) and reused by every camera's
/// [`Hour::prepare_overlay_cached`].
#[derive(Clone)]
pub struct OverlayTables {
    counters: counters::HourTables,
    vision: Arc<perspective::Vision>,
    labels: Arc<map_display_data::landscape::Labels>,
    radar: Arc<viewer_world::map::radar_symbols::Field>,
    air: Arc<std::sync::Mutex<OverlayAir>>,
    /// The engaged-flag index covered this hour when these were built.
    pub engaged_covered: bool,
    /// Built with the rest on the tables' worker only: raid offer evaluation runs path
    /// searches and must never land on the frame path.
    raids: Arc<raids::RaidIcons>,
    /// The Operatives map mode's intel icons for the viewing country.
    intel: IntelPart,
    /// The branch, hour and canonical viewing country the tables are for.
    built_for: (replay_data::BranchId, replay_data::Hour, u32),
    /// Made at the head only: the viewing country's raid arrows, which today's order field carries.
    raid_arrows: Arc<OrderField>,
}

/// The rows the overlay's layout reads: the counters, resource and occupation icons, air orders,
/// ranges and assignment, occupation stripes and naval selection. Rows at the head while the map's
/// playhead is open, and nothing else: an input that read a table they do not hold yet reads it as
/// unavailable, and [`Self::missed`] names it for their reader to declare, so the window keeps the
/// overlay it laid out last. The hour's frame with no open map. Inputs may run on a worker, so
/// each read learns its tables on its own thread.
#[derive(Debug, Default)]
pub struct OverlayRows {
    rows: Option<Arc<DecodedFrame>>,
    missed: std::sync::Mutex<Vec<replay_data::rows::TableId>>,
    /// Each input read from the rows: its name, wall nanoseconds and thread cycles.
    spent: std::sync::Mutex<Vec<(&'static str, u64, u64)>>,
    /// The inputs the map's layers draw, which are not made here.
    left: LeftToLayers,
}

impl OverlayRows {
    pub fn at_head(rows: Option<Arc<DecodedFrame>>) -> Self {
        Self { rows, missed: Default::default(), spent: Default::default(), left: LeftToLayers::default() }
    }

    /// These rows, with the inputs `left` drawn by the map's layers.
    pub fn leaving(self, left: LeftToLayers) -> Self {
        Self { left, ..self }
    }

    pub fn read<T>(&self, name: &'static str, hour: &DecodedFrame, read: impl Fn(&DecodedFrame) -> T) -> T {
        let Some(rows) = &self.rows else { return read(hour) };
        let (started, cycles) = (std::time::Instant::now(), replay_frame::cpu::thread_cycles());
        let (value, missed) = replay_frame::frame::discovering(|| read(rows));
        if missed.is_empty() {
            let spent = (started.elapsed().as_nanos() as u64, replay_frame::cpu::thread_cycles().saturating_sub(cycles));
            self.spent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((name, spent.0, spent.1));
            return value;
        }
        let mut held = self.missed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for table in missed {
            if !held.contains(&table) {
                held.push(table);
            }
        }
        value
    }

    /// Which rows these are, for a product kept while they hold; 0 for the hour's frame.
    fn identity(&self) -> usize {
        self.rows.as_ref().map_or(0, |rows| Arc::as_ptr(rows) as usize)
    }

    /// Each input read from the rows: its name, wall nanoseconds and thread cycles.
    pub fn spent(&self) -> Vec<(&'static str, u64, u64)> {
        self.spent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// The tables an input read that the rows did not hold.
    pub fn missed(&self) -> Vec<replay_data::rows::TableId> {
        self.missed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

/// Selection is viewer-local, while camera changes reuse the same Air products.
#[derive(Clone)]
struct OverlayAir {
    selected: Arc<Vec<u64>>,
    /// The rows a selection's products were read from ([`OverlayRows::identity`]).
    rows: usize,
    orders: Arc<viewer_world::map::air_orders::Prepared>,
    ranges: Arc<viewer_world::map::air_ranges::Fields>,
}

impl OverlayAir {
    fn for_selection(
        &mut self,
        selected: &Arc<Vec<u64>>,
        rows: usize,
        build: impl FnOnce() -> (
            viewer_world::map::air_orders::Prepared,
            viewer_world::map::air_ranges::Fields,
        ),
    ) -> (
        Arc<viewer_world::map::air_orders::Prepared>,
        Arc<viewer_world::map::air_ranges::Fields>,
    ) {
        if self.selected != *selected || (!selected.is_empty() && self.rows != rows) {
            let (orders, ranges) = build();
            *self = Self {
                selected: Arc::clone(selected),
                rows,
                orders: Arc::new(orders),
                ranges: Arc::new(ranges),
            };
        }
        (Arc::clone(&self.orders), Arc::clone(&self.ranges))
    }
}

impl OverlayTables {
    /// Worker/export preparation only; refreshing tables on a frame does no warming.
    pub fn warm_counter_flags(&self, assets: &counters::CounterAssets) {
        self.counters.warm_flags(assets);
    }

    pub fn naval_dominance_sources(
        &self,
    ) -> Arc<viewer_foundation::map::naval_dominance_sources::RegionalDominanceSourcesMap> {
        self.counters.naval_dominance_sources()
    }

    /// See [`counters::HourTables::land_combat_gaps`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn land_combat_gaps(
        &self,
    ) -> (
        &counters::combat_markers::Gaps,
        &counters::combat_markers::Gaps,
    ) {
        self.counters.land_combat_gaps()
    }

    /// The hour's counter tables, which the counters layer is proved against.
    pub fn counters(&self) -> &counters::HourTables {
        &self.counters
    }

    /// The raid icons, to keep drawing while the builders prepare new ones.
    pub fn raids(&self) -> Arc<raids::RaidIcons> {
        Arc::clone(&self.raids)
    }

    /// The canonical viewing country the tables are for.
    pub fn ours(&self) -> u32 {
        self.built_for.2
    }

    /// The viewing country's raid arrows, made with tables made at the head; empty otherwise.
    pub fn raid_arrows(&self) -> &Arc<OrderField> {
        &self.raid_arrows
    }

    /// Which build these are: their clones share it, another build never does.
    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.air) as *const () as usize
    }

    pub fn vision(&self) -> &Arc<perspective::Vision> {
        let _cost = replay_frame::cpu::scope("viewer", Some("fog"));
        &self.vision
    }

    /// Replaces the intel icons with those the map's layer evaluated at the head; `false` when
    /// they are the ones held, or built for another viewing country.
    /// Replaces the labels with the labels layer's for the same viewing country; whether they changed.
    pub fn set_labels(&mut self, labels: &Arc<map_display_data::landscape::Labels>, ours: u32) -> bool {
        if Arc::ptr_eq(&self.labels, labels) || ours != self.built_for.2 {
            return false;
        }
        self.labels = Arc::clone(labels);
        true
    }

    pub fn set_intel(&mut self, intel: &Arc<intel_icons::IntelIcons>) -> bool {
        if matches!(&self.intel, IntelPart::Made(held) if Arc::ptr_eq(held, intel)) || intel.built_for.is_some_and(|(_, _, ours)| ours != self.built_for.2) {
            return false;
        }
        self.intel = IntelPart::Made(Arc::clone(intel));
        true
    }

}

impl<'a> Hour<'a> {
    /// A province's integer bitmap centroid with its sampled terrain height.
    fn icon_centroid(&self, province: u32) -> Option<[f32; 3]> {
        let (_, geography) = self.icons.raids.mission_topology().ok()?;
        let [x, z] = *geography.centroids.get(&province)?;
        let (x, z) = (x as f32, z as f32);
        Some([x, self.heights().surface_height(x, z), z])
    }

    /// The resource icons `lay` makes for the camera; none when their content failed to load,
    /// which is a document warning.
    fn resource_icons_with(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        visibility: viewer_foundation::map::mode_visibility::Visibility,
        lay: impl FnOnce(&viewer_world::map::resource_icons::ResourceIconInputs<'_>) -> viewer_world::map::resource_icons::PreparedResourceIcons,
    ) -> viewer_world::map::resource_icons::PreparedResourceIcons {
        let Ok(content) = &self.resource_icons else { return Default::default() };
        lay(&viewer_world::map::resource_icons::ResourceIconInputs {
            content,
            localisation: &self.names.localisation,
            state_provinces: &self.icons.raids.trigger_content.state_provinces,
            centroid: &|province| self.icon_centroid(province),
            camera,
            viewport,
            ui_scale,
            visibility,
        })
    }

    /// The occupation icons `lay` makes for the camera and viewing country `country`; none when
    /// their content failed to load.
    fn occupation_icons_with(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        mode: Option<viewer_world::map::occupation_icons::Mode>,
        country: u32,
        lay: impl FnOnce(&viewer_world::map::occupation_icons::Inputs<'_>) -> viewer_world::map::occupation_icons::PreparedOccupationIcons,
    ) -> viewer_world::map::occupation_icons::PreparedOccupationIcons {
        let Ok(content) = &self.icons.occupation else { return Default::default() };
        lay(&viewer_world::map::occupation_icons::Inputs {
            content,
            mode,
            country,
            state_provinces: &self.icons.raids.trigger_content.state_provinces,
            centroid: &|province| self.icon_centroid(province),
            camera,
            viewport,
            ui_scale,
        })
    }

    pub fn fog_products(
        &self,
        choice: &perspective::Choice,
        memos: &map_products::FogMemos,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
        intel: bool,
    ) -> io::Result<Arc<map_products::FogProduct>> {
        let _cost = replay_frame::cpu::scope("viewer", Some("fog"));
        self.product_hour.fog_with(choice, memos, spans, intel)
    }

    pub fn weather_field(&self) -> io::Result<viewer_world::map::weather_effects::Field> {
        let _cost = replay_frame::cpu::scope("viewer", Some("weather"));
        self.product_hour.weather_field()
    }

    pub fn air_ranges(
        &self,
        viewer: u32,
        selected: &[u64],
    ) -> viewer_world::map::air_ranges::Fields {
        self.air_ranges_from(self.frame, viewer, selected)
    }

    pub fn air_ranges_from(
        &self,
        frame: &DecodedFrame,
        viewer: u32,
        selected: &[u64],
    ) -> viewer_world::map::air_ranges::Fields {
        let defines = &self.icons.raids.defines;
        viewer_world::map::air_ranges::read_fields(
            frame,
            viewer,
            selected,
            viewer_foundation::map::air_range::Defines {
                map_scale: defines.map_scale,
                ace_wing_size: defines.ace_wing_size,
                ace_wing_size_max_bonus: defines.ace_wing_size_max_bonus,
            },
            &self.radar_centers,
            &self.icons.raids.geometry.centroids,
            self.heights(),
        )
    }
    pub fn new(content: &'a Content, frame: &'a DecodedFrame, hour_fraction: f32) -> Self {
        Self::from_input(
            content,
            replay_frame::frame::HourInput::new(frame),
            hour_fraction,
        )
    }

    pub fn from_input(
        content: &'a Content,
        input: replay_frame::frame::HourInput<'a>,
        hour_fraction: f32,
    ) -> Self {
        let frame = input.frame();
        Self {
            content,
            frame,
            hour_fraction,
            product_hour: map_products::ProductHour::from_input(
                &content.products,
                input,
                hour_fraction,
            ),
            pool_limits: Default::default(),
            worker_etas: None,
            air_rosters: None,
            moved_hours: None,
        }
    }

    /// The recording's daily air rosters, for the interception alerts a mod's short air log
    /// timeout can change; without them such an alert is withheld.
    pub fn with_air_rosters(
        mut self,
        air_rosters: Option<Arc<crate::playback::AirRosters>>,
    ) -> Self {
        self.air_rosters = air_rosters;
        self
    }

    /// The hours this publication moved its movers at, which its counters' layout places them by.
    pub fn with_moved_hours(
        mut self,
        moved_hours: Arc<viewer_foundation::map::counter_move::MovedHours>,
    ) -> Self {
        self.moved_hours = Some(moved_hours);
        self
    }

    pub fn trigger_world(&self) -> &dashboard_text::triggers::World<'a> {
        self.product_hour.trigger_world()
    }

    /// This hour's derived ETAs, cached and shared by both consumers.
    ///
    /// The counter layer builds on the window thread and the close-zoom models
    /// on workers that run ahead of it, so the cache lives on the shared
    /// [`Content`] and holds the hours in flight rather than only the last one.
    pub fn etas(&self) -> io::Result<Arc<Etas>> {
        let build = || self.eta_cache.etas(self.frame, self.movement.as_deref());
        match &self.worker_etas {
            Some(gate) => gate.get_or_try_init(build),
            None => build(),
        }
    }

    /// Build only the current hour's railway graph; static geography is cached.
    pub fn rail_mesh(&self) -> io::Result<RailMesh> {
        Ok(self.rail_product()?.0.clone())
    }

    /// The labels' per-hour product for the viewing slot `ours`, as the overlay tables take it.
    pub fn labels(&self, ours: u32) -> io::Result<Arc<map_display_data::landscape::Labels>> {
        self.product_hour.labels(ours)
    }

    pub fn rail_product(&self) -> io::Result<Arc<map_products::landscape::RailProduct>> {
        self.product_hour.rails(&self.rail_geometry, &self.bridges)
    }

    // orders: the viewing country's battle plan
    /// This hour's battle-plan ribbons and labels, with allied plans disabled.
    ///
    /// The viewing country decides the default plan; the explicit allied
    /// variant follows the admitted country occurrences.
    /// Labels are in the reader's language, from the `arrowText` font.
    ///
    /// Ordinary plans share the field while [`orders::TABLES`] and the
    /// viewing country are unchanged (`crate::map::memo`). Invasion search
    /// can read other tables through adjacency triggers and uses the full hour.
    ///
    /// The viewing country's raid arrows share the plan's map-arrow manager,
    /// so they are drawn from the same field: the plan's,
    /// memoised, with the hour's raid arrows appended ([`raid_arrows`]).
    pub fn order_field(&self, perspective: &perspective::Choice) -> io::Result<Arc<OrderField>> {
        self.order_field_with_allied(perspective, false)
    }

    pub fn order_field_with_allied(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
    ) -> io::Result<Arc<OrderField>> {
        Ok(self.orders_and_convoys(perspective, show_allied)?.0)
    }

    /// The viewed country's trade-route lines and convoys of the hour
    /// ([`viewer_world::map::trade_routes`]).
    pub fn trade_route_lines(&self, ours: u32) -> Arc<viewer_world::map::trade_routes::TradeRoutes> {
        self.trade_route_memo
            .get_or_insert_with((TableKey::of(self.frame, TRADE_ROUTE_TABLES), ours), || {
                Ok::<_, io::Error>(Arc::new(self.build_trade_route_lines(ours)))
            })
            .unwrap_or_else(|why| {
                let mut routes = viewer_world::map::trade_routes::TradeRoutes::default();
                routes.report.unavailable = Some(why.to_string());
                Arc::new(routes)
            })
    }

    /// The hour's convoy lines, kept in `kept` from one hour to the next: the lines and their
    /// geometry while the columns they read read the same and the viewer is the same, and the
    /// convoys they carry made again from those lines alone while nothing but the convoys assigned
    /// moved ([`TradeRouteRead`], [`trade_routes::sailed`]). The lines are built whole otherwise,
    /// and at a bounded rate beside the convoys kept, which are compared with them
    /// ([`viewer_foundation::install_check`]).
    pub fn trade_route_convoys_kept(&self, ours: u32, kept: &mut Option<KeptConvoys>) -> Arc<Vec<viewer_world::map::trade_routes::ConvoyLine>> {
        use replay_data::rows::TableId;
        use viewer_world::map::trade_routes;
        let read = TradeRouteRead::of(self.frame, kept.as_ref().map(|held| &held.read));
        if let Some(held) = kept.as_mut()
            && held.ours == ours
            && held.read.same(&read)
        {
            if held.read.assigned_same(&read) {
                held.read = read;
                viewer_foundation::view_record::add("trade routes", "convoys kept", 1);
                return Arc::clone(&held.convoys);
            }
            if let Ok(defines) = &self.trade_routes
                && let Some(made) = trade_routes::sailed(&held.routes, &defines.convoys, read.assigned_of(TableId::ResourceOrigins), read.assigned_of(TableId::ConvoyClients))
            {
                let made = Arc::new(made);
                static KEPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                if viewer_foundation::install_check::due(&KEPT) {
                    let whole = self.trade_route_lines(ours);
                    if whole.convoys != *made {
                        viewer_foundation::install_check::found("trade route convoys", format!("hour {}: {} convoy lines kept, {} built whole", self.frame.hour, made.len(), whole.convoys.len()));
                    }
                }
                (held.read, held.convoys) = (read, Arc::clone(&made));
                viewer_foundation::view_record::add("trade routes", "convoys made again", 1);
                return made;
            }
        }
        viewer_foundation::view_record::add("trade routes", "lines built whole", 1);
        let routes = self.trade_route_lines(ours);
        let convoys = Arc::new(routes.convoys.clone());
        *kept = Some(KeptConvoys { read, ours, routes, convoys: Arc::clone(&convoys) });
        convoys
    }

    fn build_trade_route_lines(&self, ours: u32) -> viewer_world::map::trade_routes::TradeRoutes {
        use viewer_world::map::trade_routes;
        let graph = match self.icons.raids.mission_topology() {
            Ok((graph, geography)) => (graph, geography),
            Err(why) => {
                let mut routes = trade_routes::TradeRoutes::default();
                routes.report.unavailable = Some(format!(
                    "the content's province graph is unavailable: {why}"
                ));
                return routes;
            }
        };
        let (graph, geography) = graph;
        let provinces = match self
            .frame
            .complete::<replay_data::rows::tables::Provinces>()
        {
            Ok(rows) => rows,
            Err(why) => {
                let mut routes = trade_routes::TradeRoutes::default();
                routes.report.unavailable = Some(why.to_string());
                return routes;
            }
        };
        let ports: BTreeMap<u32, u32> = provinces
            .iter()
            .filter(|p| p.port_sea > 0)
            .map(|p| (p.id, p.port_sea as u32))
            .collect();
        let state_of = provinces.iter().map(|p| (p.id, p.state)).collect();
        let counters = &self.counters;
        let position = |province: u32| {
            let slot = if geography.land.contains(&province) && ports.contains_key(&province) {
                viewer_foundation::map::unit_rules::SHIP_IN_PORT
            } else {
                viewer_foundation::map::unit_rules::STANDSTILL
            };
            counters.anchor(province, slot)
        };
        let pixels = |province| self.province_geometry.pixel_count(province);
        let crossing = |a, b| self.movement.as_ref()?.map.neighbour_crossing(a, b);
        let content = trade_routes::RouteContent {
            graph,
            geography,
            state_of: &state_of,
            ports: &ports,
            pixels: &pixels,
            crossing: &crossing,
            region_of: &self.icons.raids.regions,
            region_provinces: &self.icons.raids.region_provinces,
            state_provinces: &self.icons.raids.trigger_content.state_provinces,
            position: &position,
        };
        let heights = self.heights();
        let declared = |name: &str| self.unit_models.declares(name);
        // The stops are kept for whichever build holds them; one begun while another builds
        // searches its own rather than wait for them.
        let mut kept = self.trade_route_stops.try_lock().ok();
        if let Some(kept) = kept.as_deref_mut() {
            kept.begin();
        }
        trade_routes::build(
            &trade_routes::Inputs {
                orders: &self.orders,
                defines: self.trade_routes.as_ref().map_err(String::as_str),
                content: &content,
                heights: &heights,
                ours,
                declared: &declared,
            },
            self.frame,
            kept.as_deref_mut(),
        )
    }

    /// The battle plan with the hour's raid arrows and trade-route lines appended in the shared
    /// arrow manager, and the trade-route convoys the unit models sail.
    pub fn orders_and_convoys(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
    ) -> io::Result<(
        Arc<OrderField>,
        Vec<viewer_world::map::trade_routes::ConvoyLine>,
    )> {
        self.orders_and_convoys_with(perspective, show_allied, false, false)
    }

    /// [`Self::orders_and_convoys`] without the battle plan (`plan_layered`) or the trade routes
    /// (`routes_layered`): the map's orders and convoys layers draw them.
    pub fn orders_and_convoys_with(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
        plan_layered: bool,
        routes_layered: bool,
    ) -> io::Result<(
        Arc<OrderField>,
        Vec<viewer_world::map::trade_routes::ConvoyLine>,
    )> {
        let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
        let plan = if plan_layered { Arc::new(OrderField::default()) } else { self.plan_field(perspective, show_allied)? };
        let ours = self.vision(perspective)?.ours();
        let (raid_field, report) = self.raid_arrows(ours);
        let routes = if routes_layered { Arc::default() } else { self.trade_route_lines(ours) };
        let route_warnings = routes.report.warnings();
        let air = self.air_orders(ours, &[], &[]);
        let air_warnings = air.warnings();
        if raid_field.draws.is_empty()
            && report == Default::default()
            && routes.field.draws.is_empty()
            && route_warnings.is_empty()
            && air.field.draws.is_empty()
            && air_warnings.is_empty()
        {
            return Ok((plan, routes.convoys.clone()));
        }
        let mut field = viewer_foundation::map::ribbon_mesh::append(&plan, &raid_field);
        field = viewer_foundation::map::ribbon_mesh::append(&field, &routes.field);
        field = viewer_foundation::map::ribbon_mesh::append(&field, &air.field);
        field.report.raid_arrows = report;
        field.report.trade_routes = route_warnings;
        field.report.air_orders = air_warnings;
        Ok((Arc::new(field), routes.convoys.clone()))
    }

    /// The viewing country's raid arrows of the hour, in a field of their own, and their report.
    pub fn raid_arrows(&self, ours: u32) -> (OrderField, viewer_foundation::map::ribbon_content::RaidArrowReport) {
        let heights = self.heights();
        let counters = &self.counters;
        let anchor = |province: u32, kind: u32| counters.anchor(province, kind);
        let declared = |name: &str| self.unit_models.declares(name);
        let evaluate = |items: &[installed_content_core::parser::Item], scopes| {
            self.trigger_world().evaluate(items, scopes)
        };
        let variant_model = |variant: u64, country: u32| {
            self.unit_models.aircraft_entity(
                self.frame,
                viewer_world::map::equipment_graphics::Limits::new(&self.pool_limits, &evaluate),
                variant,
                country,
            )
        };
        raid_arrows::build(
            &raid_arrows::ArrowInputs {
                assets: &self.icons.raids,
                orders: &self.orders,
                heights: &heights,
                movement: self.movement.as_deref().map(|content| &content.map),
                anchor: &anchor,
                ours,
                declared: &declared,
                variant_model: &variant_model,
                hour_fraction: self.hour_fraction,
            },
            self.frame,
            self.trigger_world(),
        )
    }

    /// Air arrows read game state; the local range presentation supplies highlights.
    pub fn air_orders(
        &self,
        viewer: u32,
        selected: &[u64],
        selected_wings: &[u64],
    ) -> viewer_world::map::air_orders::Prepared {
        self.air_orders_from(self.frame, viewer, selected, selected_wings)
    }

    pub fn air_orders_from(
        &self,
        frame: &DecodedFrame,
        viewer: u32,
        selected: &[u64],
        selected_wings: &[u64],
    ) -> viewer_world::map::air_orders::Prepared {
        let mut prepared = viewer_world::map::air_orders::read(
            frame,
            &viewer_world::map::air_orders::Inputs {
                arrow: self
                    .orders
                    .types
                    .get(viewer_world::map::air_orders::MISSION_ARROW),
                colors: self
                    .air_order_colors
                    .as_ref()
                    .copied()
                    .map_err(String::as_str),
                heights: self.heights(),
                viewer,
                highlighted_bases: selected,
                base_position: &|base| self.air_order_position(base),
                region_position: &|region| self.counters.region_icon(region),
            },
        );
        prepared.transfers = viewer_world::map::air_transfers::read(
            frame,
            &viewer_world::map::air_transfers::Inputs {
                viewer,
                selected: selected_wings,
                arrow: self
                    .orders
                    .types
                    .get(viewer_world::map::air_transfers::ARROW),
                color: self
                    .air_transfer_color
                    .as_ref()
                    .copied()
                    .map_err(String::as_str),
                heights: self.heights(),
                base_position: &|base| self.air_order_position(base),
            },
        );
        prepared
    }

    fn air_order_position(
        &self,
        base: &replay_data::rows::AirBaseRow,
    ) -> Result<Option<[f32; 3]>, String> {
        let state = base.state;
        let province = if let Some(state) = state {
            let kind = match base.kind {
                1 => 1,
                2 => 2,
                _ => 0,
            };
            let province = self
                .counters
                .air_base_first_provinces
                .get(&(state, kind))
                .copied()
                .transpose()
                .map_err(str::to_owned)?;
            let Some(province) = province else {
                return Ok(None);
            };
            province
        } else {
            let ships = self
                .frame
                .complete::<row_tables::Ships>()
                .map_err(|e| e.to_string())?;
            let Some(ship) = ships
                .binary_search_by_key(&base.host, |row| row.id)
                .ok()
                .map(|at| &ships[at])
            else {
                return Ok(None);
            };
            let forces = self
                .frame
                .complete::<row_tables::TaskForces>()
                .map_err(|e| e.to_string())?;
            let Some(force) = forces
                .binary_search_by_key(&ship.task_force, |row| row.id)
                .ok()
                .map(|at| &forces[at])
            else {
                return Ok(None);
            };
            force.province
        };
        // State endpoints search the ordinary air-base bucket queue, regardless
        // of the selected base's subtype. Hosted endpoints use the province centre.
        if state.is_some()
            && self.counters.kinds.get(&province) == Some(&installed_content_core::map::ProvinceKind::Land)
            && let Some(icon) = self.counters.site_icons
                [viewer_foundation::map::building_facts::AIR_BASE]
                .get(&province)
        {
            match &icon.anchor {
                Ok(viewer_foundation::map::building_facts::IconAnchor::Record(position)) => {
                    return Ok(Some(*position));
                }
                Ok(viewer_foundation::map::building_facts::IconAnchor::Province) => (),
                Err(why) => return Err(why.clone()),
            }
        }
        let [x, z] = self
            .icons
            .raids
            .geometry
            .centroids
            .get(&province)
            .copied()
            .ok_or_else(|| format!("Air arrow province {province} has no content centroid"))?;
        Ok(Some([
            x as f32,
            self.heights().native_height(x as f32, z as f32),
            z as f32,
        ]))
    }

    /// The battle plan alone; ordinary plans are memoised on [`orders::TABLES`].
    pub fn plan_field(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
    ) -> io::Result<Arc<OrderField>> {
        let ours = self.vision(perspective)?.ours();
        let links = links_of(self.frame)?;
        let build = |frame: &DecodedFrame| {
            self.content
                .battle_plan(frame, &links, ours, show_allied)
                .map(Arc::new)
        };
        // Native adjacency-rule triggers may depend on any recorded table.
        // Invasions and labelled paradrops use the full hour for their lazy
        // trigger/transport operands; ordinary plans keep the small memo.
        if orders::requires_full_frame(
            self.frame,
            &links,
            ours,
            show_allied,
            self.icons.fonts.arrow().is_some(),
        )? {
            build(self.frame)
        } else {
            let tables = orders::memo_tables(show_allied);
            self.order_memo.get_or_insert_with(
                (TableKey::of(self.frame, &tables), ours, show_allied),
                || build(&project(self.frame, &tables)),
            )
        }
    }

    // close zoom
    /// This hour's map-building targets.
    ///
    /// The viewing country decides which slots its fog of war hides.
    pub fn map_building_field(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<(
        map_display_data::buildings::Buildings,
        viewer_world::map::map_buildings::MapBuildingSummary,
    )> {
        self.map_building_field_profiled(
            perspective,
            &mut viewer_foundation::profiling::BuilderSpans::new(false),
        )
    }

    /// The same targets with sequential parts on the hour-builder PROFILE line.
    pub fn map_building_field_profiled(
        &self,
        perspective: &perspective::Choice,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<(
        map_display_data::buildings::Buildings,
        viewer_world::map::map_buildings::MapBuildingSummary,
    )> {
        let _cost = replay_frame::cpu::scope("viewer", Some("buildings"));
        self.product_hour.buildings(
            &self.map_buildings,
            perspective,
            &self.assets.spawn_city,
            spans,
        )
    }

    // close zoom
    /// This hour's placed unit models.
    ///
    /// The perspective is the counter layer's: the engine hides a division's
    /// map model on the same visibility byte, against the game's own
    /// 150 rather than the counter's 250.
    pub fn prepare_unit_models(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_world::map::unit_models::PreparedModels> {
        self.prepare_unit_models_profiled(perspective, &mut |_| {})
    }

    /// The same preparation with sequential parts reported on the builder's line.
    pub fn prepare_unit_models_profiled(
        &self,
        perspective: &perspective::Choice,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<viewer_world::map::unit_models::PreparedModels> {
        self.prepare_unit_models_with_spans(
            perspective,
            lap,
            &mut viewer_foundation::profiling::BuilderSpans::new(false),
        )
    }

    /// Includes nested model joins and bake-worker CPU on the built hour line.
    pub fn prepare_unit_models_with_spans(
        &self,
        perspective: &perspective::Choice,
        lap: &mut dyn FnMut(&'static str),
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<viewer_world::map::unit_models::PreparedModels> {
        self.prepare_unit_models_naval_or_not(perspective, true, lap, spans)
    }

    /// The same models; without `naval` the hour's ships are not made: the map's ships layer draws
    /// them.
    pub fn prepare_unit_models_naval_or_not(
        &self,
        perspective: &perspective::Choice,
        naval: bool,
        lap: &mut dyn FnMut(&'static str),
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<viewer_world::map::unit_models::PreparedModels> {
        self.unit_models_with(perspective, true, lap, |assets, inputs, lap| {
            assets.prepare_naval_or_not(inputs, naval, lap, spans)
        })
    }

    /// The hour's unit models without its land stacks, which the map's units layer chooses itself:
    /// the ships with `naval` (otherwise the map's ships layer draws them). Without the ships it
    /// reads of the viewer only whose country it is, `ours` (its canonical slot): no vision is made
    /// and no ship input read.
    pub fn prepare_unit_models_without_land(
        &self,
        perspective: &perspective::Choice,
        naval: bool,
        ours: u32,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<viewer_world::map::unit_models::PreparedModels> {
        if !naval {
            let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
            let evaluate = |items: &[installed_content_core::parser::Item], scopes| self.trigger_world().evaluate(items, scopes);
            let limits = viewer_world::map::equipment_graphics::Limits::new(&self.pool_limits, &evaluate);
            let prepared = self.unit_models.prepare_shipless(self.frame, ours, &self.counters.terrain.combat_terrain, limits, &self.names.localisation);
            lap("um_owned");
            return prepared;
        }
        self.unit_models_with(perspective, false, lap, |assets, inputs, lap| {
            let prepared = assets.prepare_without_land(inputs, naval);
            lap("um_ships_and_owned");
            prepared
        })
    }

    /// `make` over the hour's unit model inputs; the divisions' arrival estimates only with `land`.
    fn unit_models_with(
        &self,
        perspective: &perspective::Choice,
        land: bool,
        lap: &mut dyn FnMut(&'static str),
        make: impl FnOnce(
            &viewer_world::map::unit_models::UnitModelAssets,
            viewer_world::map::unit_models::PrepareInputs<'_>,
            &mut dyn FnMut(&'static str),
        ) -> io::Result<viewer_world::map::unit_models::PreparedModels>,
    ) -> io::Result<viewer_world::map::unit_models::PreparedModels> {
        let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
        let prepared = (|| {
            let content = &self.counters;
            let definitions = &self.map.definitions;
            let etas = if land { self.etas()? } else { Arc::default() };
            lap("um_etas");
            let movement = self.movement.as_deref();
            let world = movement.map(
                |movement| viewer_foundation::map::counter_move::RouteWorld {
                    map: &movement.map,
                    heights: self.heights(),
                    width: self.map.width as f32,
                },
            );
            let anchor = |province, slot| content.anchor(province, slot);
            // Whether a province is land: the content movement map's terrain flag, else the map's
            // own kind. It picks a task force's slot 19.
            let land = |province| match movement.and_then(|movement| movement.map.terrain(province))
            {
                Some(terrain) => terrain.flags & 1 != 0,
                None => matches!(
                    definitions.get(&province).map(|it| it.kind),
                    Some(map::ProvinceKind::Land)
                ),
            };
            let motions =
                viewer_foundation::map::task_force_visual::TaskForceMotions::read(self.frame)?;
            let missions =
                viewer_foundation::map::task_force_visual::MissionSkips::read(self.frame)?;
            lap("um_ship_inputs");
            let region = |province| content.region(province);
            let evaluate = |items: &[installed_content_core::parser::Item], scopes| {
                self.trigger_world().evaluate(items, scopes)
            };
            let limits =
                viewer_world::map::equipment_graphics::Limits::new(&self.pool_limits, &evaluate);
            let vision = self.vision(perspective)?;
            lap("um_vision");
            make(
                &self.unit_models,
                viewer_world::map::unit_models::PrepareInputs {
                    frame: self.frame,
                    limits,
                    vision: &vision,
                    etas: &etas,
                    anchor: &anchor,
                    world: world.as_ref(),
                    terrain: &content.terrain.combat_terrain,
                    routes: Some(&content.routes),
                    ships: &viewer_world::map::unit_models::ShipPlaces {
                        motions: &motions,
                        movement,
                        land: &land,
                        missions: &missions,
                        region: &region,
                        scale: &|province, kind| content.stack_scale(province, kind),
                    },
                    localisation: &self.names.localisation,
                },
                lap,
            )
        })();
        // The preparation's local joins have been released before this last part.
        lap("um_finish");
        prepared
    }

    /// The hour's unit models with the raid units, and the trade-route convoys sailing at `time`
    /// seconds of presentation time where `shown` admits them
    /// ([`viewer_world::map::trade_routes::convoys_shown`]).
    pub fn unit_model_field_sailing(
        &self,
        perspective: &perspective::Choice,
        time: f32,
        shown: &dyn Fn(&viewer_world::map::trade_routes::ConvoyDefines) -> bool,
    ) -> io::Result<(viewer_world::map::unit_history::Field, ModelReport)> {
        let mut prepared = self.prepare_unit_models(perspective)?;
        let (orders, convoys) = self.orders_and_convoys(perspective, false)?;
        prepared.add_raid_units(&self.unit_models, &orders.raid_units);
        if let Ok(defines) = &self.trade_routes {
            prepared.add_convoys(convoys, defines.convoys);
        }
        let mut field = prepared.field(&self.unit_models);
        let heights = self.heights();
        let mut refused = BTreeMap::new();
        field.raids.extend(prepared.convoy_units(
            &self.unit_models,
            time,
            &|x, z| heights.surface_height(x, z),
            shown,
            &mut refused,
        ));
        let mut report = prepared.report;
        for (reason, count) in refused {
            *report.selection_errors.entry(reason).or_default() += count;
        }
        Ok((field, report))
    }

    /// This hour's nuclear impacts at their provinces' standstill anchors.
    pub fn nuclear_impacts(&self) -> io::Result<viewer_world::map::strike_effects::Impacts> {
        let _cost = replay_frame::cpu::scope("viewer", Some("engagement/strikes/text"));
        self.product_hour.nuclear_impacts()
    }

    /// A province's standstill unit-stack anchor and admission box, where
    /// its impact effects stand.
    fn impact_place(&self, province: u32) -> Option<([f32; 3], [f32; 4])> {
        self.content.products.impact_place(province)
    }

    /// This hour's map events interpreted for `vision`'s viewing country:
    /// their floating text and critical bombardment explosions.
    pub fn map_text(
        &self,
        vision: &perspective::Vision,
    ) -> viewer_world::map::animated_map_text::events::Prepared {
        self.product_hour.map_text(vision)
    }

    pub fn aircraft_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_world::map::aircraft::AircraftPlan> {
        let _cost = replay_frame::cpu::scope("viewer", Some("air/naval source choices"));
        let heights = HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        };
        let movement = self.movement.as_deref();
        let world = movement.map(
            |movement| viewer_foundation::map::counter_move::RouteWorld {
                map: &movement.map,
                heights: self.heights(),
                width: self.map.width as f32,
            },
        );
        let motions =
            viewer_foundation::map::task_force_visual::TaskForceMotions::read(self.frame)?;
        let land = |province| {
            movement
                .and_then(|movement| movement.map.terrain(province))
                .map_or_else(
                    || {
                        self.map
                            .definitions
                            .get(&province)
                            .is_some_and(|definition| definition.kind == map::ProvinceKind::Land)
                    },
                    |terrain| terrain.flags & 1 != 0,
                )
        };
        let origin = |force: &replay_data::rows::TaskForceRow| {
            motions.settled_origin(
                force,
                movement,
                world.as_ref(),
                &|province, slot| self.counters.anchor(province, slot),
                &land,
            )
        };
        let evaluate = |items: &[installed_content_core::parser::Item], scopes| {
            self.trigger_world().evaluate(items, scopes)
        };
        let models = viewer_world::map::aircraft::ModelMemo::default();
        let model = |variant, country| {
            models.resolve(variant, country, || {
                self.unit_models.aircraft_entity(
                    self.frame,
                    viewer_world::map::equipment_graphics::Limits::new(
                        &self.pool_limits,
                        &evaluate,
                    ),
                    variant,
                    country,
                )
            })
        };
        viewer_world::map::aircraft::AircraftPlan::build(
            self.frame,
            &self.aircraft,
            &self.counters.region_of,
            &self.province_geometry,
            &heights,
            self.vision(perspective)?.ours(),
            &model,
            &origin,
            &|province| self.impact_place(province),
        )
    }

    pub fn train_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_world::map::trains::TrainPlan> {
        let _cost = replay_frame::cpu::scope("viewer", Some("rails"));
        let heights = HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        };
        viewer_world::map::trains::TrainPlan::build_with_graph(
            self.frame,
            self.vision(perspective)?.ours(),
            &self.rail_geometry,
            &heights,
            &self.trains,
            &self.trains.cultures,
            || self.product_hour.rail_graph(),
        )
    }

    /// What the hour on screen could not draw, one line per degraded table.
    ///
    /// The map passes degrade on their own; this is the single list of what
    /// that cost, so the window's warning area and a `--dump` report the same
    /// thing. A core table's gap is not here: it is the frame error above.
    pub fn limitations(&self, perspective: &perspective::Choice) -> io::Result<Limitations> {
        self.product_hour.limitations(perspective)
    }

    /// The map mode's country channel this hour.
    /// `colour` is a scripted mode's borders as the previous hour kept them.
    pub fn map_mode_fill(
        &self,
        mode: &viewer_world::map::map_mode::MapMode,
        choice: &perspective::Choice,
        selected: Option<u32>,
        colour: Option<&viewer_world::map::map_mode::ColourState>,
    ) -> io::Result<viewer_world::map::map_mode::ModeFill> {
        let _cost = replay_frame::cpu::scope("viewer", Some("modes/styles"));
        self.product_hour
            .mode(&self.map_modes, mode, choice, selected, colour)
    }

    pub fn political_product(
        &self,
        mode: &viewer_world::map::map_mode::MapMode,
        choice: &perspective::Choice,
        selected: Option<u32>,
        colour: Option<&viewer_world::map::map_mode::ColourState>,
    ) -> io::Result<map_products::landscape::PoliticalProduct> {
        self.product_hour.political(
            &self.map_modes,
            &self.naval_dominance,
            mode,
            choice,
            selected,
            colour,
        )
    }

    pub fn seed_vision(
        &self,
        choice: &perspective::Choice,
        vision: Arc<perspective::Vision>,
    ) {
        self.product_hour.seed_vision(choice, vision);
    }

    // counters: the viewing country
    /// What this hour's counters can see, for the limitation lines and the viewing tag.
    pub fn vision(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<Arc<perspective::Vision>> {
        let _cost = replay_frame::cpu::scope("viewer", Some("fog"));
        self.product_hour.vision(perspective)
    }

    pub fn viewing_tag(&self, vision: &perspective::Vision) -> io::Result<String> {
        self.product_hour.viewing_tag(vision)
    }

    // borders: the demilitarised-zone bucket bank
    /// This hour's demilitarised zone, for `viewer_world::map::demilitarized` and the border pass.
    ///
    /// The recorded set is the hour's own `StateFlags` joined to its `States`
    /// ([`viewer_world::map::demilitarized::recorded_states`]). `extra` is the `--dump --demilitarized`
    /// override, empty everywhere else; it only ever *adds* to the recorded set, so an hour that
    /// records the flag is drawn from the recording whatever the command line says.
    pub fn demilitarized_zone(&self, regions: &Regions, extra: &BTreeSet<u32>) -> io::Result<Zone> {
        self.product_hour.zone(regions, extra)
    }

    // names
    pub fn place_names(
        &self,
        mode: &viewer_world::map::map_mode::MapMode,
    ) -> io::Result<Placements> {
        Ok((*self.shared_names(mode)?).clone())
    }
    pub fn shared_names(
        &self,
        mode: &viewer_world::map::map_mode::MapMode,
    ) -> io::Result<Arc<Placements>> {
        self.product_hour.names(&self.name_inputs(), mode)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn name_areas(
        &self,
        mode: &viewer_world::map::map_mode::MapMode,
    ) -> io::Result<name_pass::NameAreas> {
        let _cost = replay_frame::cpu::scope("viewer", Some("world names"));
        self.product_hour.name_areas(&self.name_inputs(), mode)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn place_name_areas(
        &self,
        areas: &name_pass::NameAreas,
        placed: &name_pass::PlacedAreas,
    ) -> Placements {
        let _cost = replay_frame::cpu::scope("viewer", Some("world names"));
        name_pass::place_areas(&self.name_inputs().placement, areas, Some(placed))
    }
    fn name_inputs(&self) -> map_products::landscape::NameInputs<'_> {
        map_products::landscape::NameInputs {
            content: &self.map_modes,
            names: &self.names,
            regions: &self.strategic_regions,
            heightmap: &self.assets.heightmap,
            counter: &self.counter_content,
            placement: PlacementInputs {
                map: &self.map,
                heightmap: &self.assets.heightmap,
                content: &self.names,
                font: &self.icons.fonts.map,
            },
        }
    }

    // counters
    /// The hour's camera-independent icon tables for the window's per-frame
    /// icon rebuilds; see [`Hour::prepare_overlay_cached`].
    pub fn hour_tables(
        &self,
        perspective: &perspective::Choice,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
    ) -> io::Result<OverlayTables> {
        let ours = self.vision(perspective)?.ours();
        self.hour_tables_with_raids(perspective, Arc::new(self.raid_icons(ours)), engaged, counters::Kinds::default())
    }

    /// The hour's raid icons for canonical country `ours`, every offer evaluated.
    fn raid_icons(&self, ours: u32) -> raids::RaidIcons {
        let paths = raids::paths::HourPaths::new(&self.icons.raids, self.frame);
        raids::icons(&self.icons.raids, self.frame, ours, &paths)
    }

    /// [`Hour::hour_tables`] with raid icons already built: the window passes
    /// the icons its raid lane built ([`crate::raid_lane`]), which draw only
    /// when they are this hour's own, so no raid offer is evaluated on the
    /// frame. Until the lane delivers this hour's none draw, and the
    /// diagnostics say so ([`raids::RaidIcons::carried`]). Today's source of each counter kind in
    /// `left_out` is left to the map's layers ([`CounterView::layered`]).
    pub fn hour_tables_with_raids(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        left_out: counters::Kinds,
    ) -> io::Result<OverlayTables> {
        self.hour_tables_with_raids_profiled(
            perspective,
            raids,
            engaged,
            left_out,
            &mut viewer_foundation::profiling::BuilderSpans::new(false),
            true,
        )
    }

    /// Builds the same immutable tables with explicit, job-owned child timings.
    pub fn hour_tables_with_raids_profiled(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        left_out: counters::Kinds,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
        radar: bool,
    ) -> io::Result<OverlayTables> {
        self.hour_tables_layered(perspective, raids, engaged, left_out, false, spans, radar)
    }

    /// [`Self::hour_tables_with_raids_profiled`], reading no labels when the map's labels layer
    /// draws them (`labels_by_layers`): the window lays the layer's out in their place.
    pub fn hour_tables_layered(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        left_out: counters::Kinds,
        labels_by_layers: bool,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
        radar: bool,
    ) -> io::Result<OverlayTables> {
        self.hour_tables_of(perspective, raids, engaged.map(|index| index.shared.as_ref()), left_out, labels_by_layers, spans, radar, counters::Need::WHOLE)
    }

    /// The open map's overlay tables over rows at the head (`self.frame`): the vision its vision
    /// layer shows, the raid icons its scripts reader built, the engagements layer's index. The
    /// parts its view needs, made as `need` says.
    #[allow(clippy::too_many_arguments, reason = "the tables' inputs, what the view draws and the timings")]
    pub fn hour_tables_at_head(
        &self,
        perspective: &perspective::Choice,
        vision: Arc<perspective::Vision>,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&viewer_foundation::engagement_history::History>,
        left_out: counters::Kinds,
        left: LeftToLayers,
        labels_by_layers: bool,
        need: counters::Need,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<OverlayTables> {
        self.seed_vision(perspective, vision);
        let mut tables = self.hour_tables_of(perspective, raids, engaged, left_out, labels_by_layers, spans, !left.radar, need)?;
        let (mut arrows, report) = spans.measure("overlay.tables.raid_arrows", |_| self.raid_arrows(tables.ours()));
        arrows.report.raid_arrows = report;
        tables.raid_arrows = Arc::new(arrows);
        Ok(tables)
    }

    #[allow(clippy::too_many_arguments, reason = "the tables' inputs, the parts left to layers and the timings")]
    fn hour_tables_of(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&viewer_foundation::engagement_history::History>,
        left_out: counters::Kinds,
        labels_by_layers: bool,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
        radar: bool,
        need: counters::Need,
    ) -> io::Result<OverlayTables> {
        let (etas, vision) = spans.measure("overlay.tables.prerequisites", |spans| {
            // Routes made by key make each admitted division's estimate as they make its route,
            // and a view that draws no counter and no combat marker reads no route at all.
            let etas = spans.measure("overlay.tables.prerequisites.etas", |_| {
                if need.routes_by_key(left_out) || !(need.shown.units || need.shown.land_combats) { Ok(Arc::default()) } else { self.etas() }
            })?;
            // One `Vision` per hour and perspective: the counters and the limitation
            // lines read the same cached one.
            let vision = spans.measure("overlay.tables.prerequisites.vision", |_| {
                self.vision(perspective)
            })?;
            Ok::<_, io::Error>((etas, vision))
        })?;
        let ours = vision.ours();
        let labels = if labels_by_layers {
            Arc::default()
        } else {
            spans.measure("overlay.tables.labels", |_| {
                let _cost = replay_frame::cpu::scope("viewer", Some("labels"));
                self.product_hour.labels(ours)
            })?
        };
        // Checked first: coverage that arrives during the build costs one
        // rebuild, never a stale flag.
        let engaged_covered =
            engaged.is_some_and(|index| index.covers(self.frame.branch, self.frame.hour));
        let (radar, empty_air) = spans.measure("overlay.tables.radar_empty_air", |_| {
            let radar = Arc::new(if radar {
                viewer_world::map::radar_symbols::Field::read(self.frame, ours, self.heights(), &self.radar_centers)
            } else {
                viewer_world::map::radar_symbols::Field::default()
            });
            let empty_air = OverlayAir {
                selected: Arc::new(Vec::new()),
                rows: 0,
                orders: Arc::new(self.air_orders(ours, &[], &[])),
                ranges: Arc::new(self.air_ranges(ours, &[])),
            };
            (radar, empty_air)
        });
        Ok(OverlayTables {
            vision: Arc::clone(&vision),
            engaged_covered,
            radar,
            air: Arc::new(std::sync::Mutex::new(empty_air)),
            counters: spans.measure("overlay.tables.counters", |spans| {
                counters::hour_tables_profiled(
                    self.frame,
                    &self.map,
                    vision,
                    &self.counters,
                    &self.icons.counters,
                    Ok(self.unit_models.sub_units.equipment_content()),
                    &self.names.localisation,
                    &etas,
                    self.movement.as_deref(),
                    engaged,
                    self.air_rosters
                        .as_deref()
                        .map(|rosters| rosters as &dyn counters::AirPasses),
                    &self.type_tint_memo,
                    &self.mission_memo,
                    self.icons.raids.movement_reads(),
                    self.icons.raids.mission_topology(),
                    left_out,
                    need,
                    &self.overlay_carry,
                    self.air_rosters.clone().map(|rosters| rosters as Arc<dyn counters::AirPasses + Send + Sync>),
                    spans,
                )
            })?,
            labels,
            raids: raids::RaidIcons::carried(raids, self.frame.branch, self.frame.hour, ours),
            intel: spans.measure("overlay.tables.intel", |_| {
                if need.by_key {
                    return IntelPart::Deferred(Arc::new(DeferredIntel { frame: Arc::new(self.frame.clone()), ours, made: std::sync::OnceLock::new() }));
                }
                IntelPart::Made(Arc::new(intel_icons::icons(
                    &self.icons.intel,
                    &self.icons.raids,
                    self.frame,
                    ours,
                    &self.names.localisation,
                    &self.raid_hover.recorded_language,
                )))
            }),
            built_for: (self.frame.branch, self.frame.hour, ours),
            raid_arrows: Arc::default(),
        })
    }

    /// Builds this frame's whole screen-space icon layer, in priority order.
    ///
    /// Content `DEFAULT_PRIORITY_*` defines sort map icons: air
    /// and naval bases 3, victory points and capitals 5, unit stacks 10, land
    /// and naval combat 20. The sprite batch draws in push order, so pushing
    /// the four groups in that order is what produces the engine's overlap.
    ///
    /// The layer is laid out in interface pixels, `viewport` divided by
    /// `ui_scale`; the sprite pass scales it back into device pixels. The
    /// tables come from [`Hour::hour_tables`]: a worker builds them from the rows at the head
    /// and the window reuses them for every camera.
    ///
    /// `view` is the map view's counter options and memory: the window's, a dashboard map's,
    /// or a fresh one for a dump.
    pub fn prepare_overlay_cached(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        tables: &OverlayTables,
        view: &CounterView,
    ) -> io::Result<PreparedOverlay> {
        self.prepare_overlay_at(camera, viewport, ui_scale, tables, view, &OverlayRows::default())
    }

    /// [`Self::prepare_overlay_cached`] with its camera-bound inputs read from `rows`.
    #[allow(clippy::too_many_arguments, reason = "the overlay's inputs and the rows they read")]
    pub fn prepare_overlay_at(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        tables: &OverlayTables,
        view: &CounterView,
        rows: &OverlayRows,
    ) -> io::Result<PreparedOverlay> {
        self.prepare_overlay_with(
            camera,
            viewport,
            ui_scale,
            tables,
            view,
            rows,
            &mut viewer_foundation::profiling::BuilderSpans::new(false),
            None,
        )
    }

    fn counter_inputs<'b>(
        &'b self,
        windows: &'b viewer_foundation::map::native_windows::NativeWindows,
        display: &'b counters::CounterDisplay,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        view: &'b CounterView,
    ) -> CounterInputs<'b> {
        CounterInputs {
            content: &self.counters,
            equipment: Ok(self.unit_models.sub_units.equipment_content()),
            assets: &self.icons.counters,
            windows,
            fonts: &self.icons.fonts,
            localisation: &self.names.localisation,
            recorded_language: &self.counter_content.recorded_language,
            map: &self.map,
            hour: self.frame.hour,
            camera,
            viewport,
            ui_scale,
            moved: self.moved_hours.as_deref().unwrap_or(&viewer_foundation::map::counter_move::NO_MOVED_HOURS),
            movement: self.movement.as_deref(),
            heights: self.heights(),
            settings: view.settings,
            display,
            selected: &view.selected,
            selected_air: &view.air_selected,
            selected_air_bases: &view.air_bases,
            selected_fleets: &view.fleets_selected,
            hovered: view.hovered,
            pressed: view.pressed,
            plate_hovered: view.plate_hovered,
            schedule: view.presentation.schedule(),
            movement_history: view.presentation.movement(),
            visibility: view.visibility,
            art: self.icons.lazy.version(),
        }
    }

    /// What an overlay for this hour and view holds at every camera: the province fields its rows
    /// supply, and its labels laid out ([`Still`]).
    fn read_still(
        &self,
        tables: &OverlayTables,
        view: &CounterView,
        rows: &OverlayRows,
    ) -> io::Result<Still> {
        let frame = self.frame;
        let hour = Hour::new(self.content, frame, self.hour_fraction);
        let air_assignment = if view.visibility.priorities
            == viewer_foundation::map::mode_visibility::PriorityTable::StrategicAir
        {
            let defines = &hour.icons.raids.defines;
            rows.read("air assignment", frame, |frame| viewer_world::map::air_assignment::read(
                frame,
                &view.air_selected,
                tables.ours(),
                viewer_foundation::map::air_range::Defines {
                    map_scale: defines.map_scale,
                    ace_wing_size: defines.ace_wing_size,
                    ace_wing_size_max_bonus: defines.ace_wing_size_max_bonus,
                },
                viewer_world::map::air_assignment::Content {
                    geometry: &hour.icons.raids.geometry,
                    states: &hour.icons.raids.trigger_content.state_provinces,
                    regions: &hour.icons.raids.region_provinces,
                    colors: hour
                        .air_assignment_colors
                        .as_ref()
                        .copied()
                        .map_err(String::as_str),
                },
            ))
        } else {
            Default::default()
        };
        let (air_orders, air_ranges) = {
            let mut air = tables
                .air
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            air.for_selection(&view.air_selected, rows.identity(), || {
                rows.read("air orders and ranges", frame, |frame| {
                    (
                        hour.air_orders_from(frame, tables.ours(), &[], &view.air_selected),
                        hour.air_ranges_from(frame, tables.ours(), &view.air_selected),
                    )
                })
            })
        };
        let occupation_stripes = if view.visibility.priorities
            == viewer_foundation::map::mode_visibility::PriorityTable::Resistance
        {
            rows.read("occupation stripes", frame, |frame| {
                viewer_world::map::occupation_stripes::read(
                    frame,
                    tables.ours(),
                    &self.map,
                    &self.icons.raids.trigger_content.state_provinces,
                    self.occupation_stripe_alpha
                        .as_ref()
                        .copied()
                        .map_err(String::as_str),
                )
            })
        } else {
            Default::default()
        };
        let naval_selection = if view.visibility.priorities
            == viewer_foundation::map::mode_visibility::PriorityTable::StrategicNavy
        {
            rows.read("naval selection", frame, |frame| {
                viewer_world::map::naval_selection::read(
                    frame,
                    tables.ours(),
                    &view.selected,
                    &view.fleets_selected,
                    &self.icons.raids.region_provinces,
                    &self.naval_region_eligibility,
                    self.naval_selection_colors
                        .as_ref()
                        .copied()
                        .map_err(String::as_str),
                )
            })
        } else {
            Default::default()
        };
        let labels = {
            let _cost = replay_frame::cpu::scope("viewer", Some("labels"));
            labels::prepare(
                &labels::LabelInputs {
                    content: &self.labels,
                    localisation: &self.names.localisation,
                    map: &self.map,
                    visibility: view.visibility,
                },
                &tables.labels,
            )?
        };
        Ok(Still {
            labels,
            air_assignment,
            air_orders,
            air_ranges,
            occupation_stripes,
            naval_selection,
            radar: Arc::clone(&tables.radar),
            engaged_covered: tables.engaged_covered,
        })
    }

    /// Lays an overlay out for this hour and view and places it at `camera`. With `still`, the
    /// parts the camera does not move are taken from the overlay being placed again, and nothing
    /// the hour holds is read or laid out afresh ([`PreparedOverlay::project`]).
    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    fn prepare_overlay_with(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        tables: &OverlayTables,
        view: &CounterView,
        rows: &OverlayRows,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
        still: Option<&mut Still>,
    ) -> io::Result<PreparedOverlay> {
        let lazy_version = self.icons.lazy.version();
        let frame = self.frame;
        let windows = spans.measure("overlay.projection.windows", |_| {
            self.icons
                .counters
                .window_cache
                .at(native_shader_draw::gui_viewport(viewport, ui_scale))
        })?;
        let display = counters::CounterDisplay::default();
        let counter_inputs = self.counter_inputs(&windows, &display, camera, viewport, ui_scale, view);
        let label_at = labels::LabelProjection { camera, viewport, ui_scale };
        let mut read = None;
        let still = match still {
            Some(still) => still,
            None => read.insert(spans.measure("overlay.projection.still", |_| self.read_still(tables, view, rows))?),
        };
        // A worker owns its Hour wrapper (and its RefCells); only immutable content
        // and the completed hour tables cross the fork.
        let content = self.content;
        let fraction = self.hour_fraction;
        let prepare_counters = || {
            let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
            let _span = viewer_foundation::profiling::span("p_counters");
            rows.read("counters", frame, |_| prepare_counters_with(&counter_inputs, tables, view, lazy_version))
        };
        let prepare_other = |labels: &mut labels::PreparedLabels| {
            let _cost = replay_frame::cpu::scope("viewer", Some("modes/styles"));
            let hour = Hour::new(content, frame, fraction);
            // Raid icons sort above the labels and the counters by their layer.
            let native_raids = hour.raid_owners(
                camera,
                viewport,
                ui_scale,
                view.visibility.raid_icons,
                &tables.raids,
            );
            let raid_report = tables.raids.report.clone();
            {
                let _span = viewer_foundation::profiling::span("p_labels");
                let _cost = replay_frame::cpu::scope("viewer", Some("labels"));
                labels.project(&label_at);
            }
            let label_report = labels.report.clone();
            let label_anchor = |province| hour.labels.anchors.get(&province).copied();
            let native_intel = intel_icons::prepare(
                &hour.icons.intel,
                &intel_icons::IntelInputs {
                    anchor: &label_anchor,
                    camera,
                    viewport,
                    ui_scale,
                    operatives: view.operatives,
                },
                &tables.intel.get(&hour),
            );
            let intel_report = native_intel.report.clone();
            // A part the map's layer draws is left empty: the window lays the layer's product in.
            let resource_icons = if rows.left.resource_icons {
                (Default::default(), MadeBy::Left)
            } else {
                let made = hour.resource_icons_with(camera, viewport, ui_scale, view.visibility, |inputs| {
                    let _cost = replay_frame::cpu::scope("viewer", Some("resource icons"));
                    rows.read("resource icons", frame, |frame| viewer_world::map::resource_icons::prepare(inputs, frame))
                });
                (made, MadeBy::Today)
            };
            let occupation_icons = if rows.left.occupation_marks {
                (Default::default(), MadeBy::Left)
            } else {
                let made = hour.occupation_icons_with(camera, viewport, ui_scale, view.occupation, tables.ours(), |inputs| {
                    let _cost = replay_frame::cpu::scope("viewer", Some("occupation icons"));
                    rows.read("occupation icons", frame, |frame| viewer_world::map::occupation_icons::prepare(inputs, frame))
                });
                (made, MadeBy::Today)
            };
            let counters = &hour.counters;
            let anchor = |province| {
                counters.anchor(province, viewer_foundation::map::unit_rules::STANDSTILL)
            };
            let adjacency_icons =
                hour.adjacency_icons
                    .prepare(&viewer_world::map::adjacency_icons::Inputs {
                        anchor: &anchor,
                        camera,
                        viewport,
                        ui_scale,
                        visibility: view.visibility,
                    });
            let adjacency_priority = viewer_world::map::adjacency_icons::priority(
                &hour.counters.defines.priorities,
                &view.visibility,
            );
            Ok::<_, io::Error>((
                label_report,
                native_raids,
                raid_report,
                native_intel,
                intel_report,
                resource_icons,
                occupation_icons,
                adjacency_icons,
                adjacency_priority,
            ))
        };
        let icons = spans.measure("overlay.projection.counters", |_| prepare_counters());
        let other = spans.measure("overlay.projection.other", |_| prepare_other(&mut still.labels));
        spans.measure("overlay.projection.compose", |_| {
            let icons = icons?;
            let (
                label_report,
                native_raids,
                raid_report,
                native_intel,
                intel_report,
                resource_icons,
                occupation_icons,
                adjacency_icons,
                adjacency_priority,
            ) = other?;
            Ok(PreparedOverlay {
                air_orders: Arc::clone(&still.air_orders),
                air_assignment: std::mem::take(&mut still.air_assignment),
                occupation_stripes: std::mem::take(&mut still.occupation_stripes),
                naval_selection: std::mem::take(&mut still.naval_selection),
                air_ranges: Arc::clone(&still.air_ranges),
                radar: Arc::clone(&still.radar),
                resource_icons: resource_icons.0,
                resource_icons_by: resource_icons.1,
                occupation_icons: occupation_icons.0,
                occupation_icons_by: occupation_icons.1,
                adjacency_icons,
                adjacency_priority,
                mode_icons: Batch::default(),
                mode_icon_refused: BTreeMap::new(),
                lazy_version,
                engaged_covered: still.engaged_covered,
                layers: view.presentation.counters.lock().unwrap_or_else(|p| p.into_inner()).layered().identity(),
                counter_view: view.clone(),
                labels: std::mem::take(&mut still.labels),
                native_raids,
                raids: Batch::default(),
                icons,
                label_report,
                raid_report,
                native_intel,
                intel: Batch::default(),
                intel_report,
                tables: tables.identity(),
            })
        })
    }

    fn raid_owners(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        reach: Option<viewer_foundation::map::mode_visibility::RaidReach>,
        raids: &raids::RaidIcons,
    ) -> raids::PreparedRaids {
        let anchor = |province| {
            self.counters
                .anchor(province, viewer_foundation::map::unit_rules::STANDSTILL)
        };
        raids::prepare(
            &raids::RaidInputs {
                anchor: &anchor,
                camera,
                viewport,
                ui_scale,
                reach,
            },
            raids,
        )
    }

    fn with_gui_context<'s, T>(
        &'s self,
        viewport: [i32; 2],
        time: Option<f32>,
        prepare: impl FnOnce(
            &NativeWindows,
        ) -> io::Result<Option<counters::gui_windows::WindowFlags<'s>>>,
        draw: impl FnOnce(&Context<'_, GuiAssets<'_>>) -> T,
    ) -> io::Result<T> {
        let windows = self
            .icons
            .counters
            .window_cache
            .at(viewport.map(|value| value as u32))?;
        let flags = prepare(&windows)?;
        let portrait_textures = |name: &[u8]| self.icons.counters.operative_portrait_texture(name);
        let resources = GuiAssets {
            gui: &self.icons.counters.gui,
            windows: &windows,
            localisation: &self.names.localisation,
            flags: flags.as_ref().map(|flags| GuiFlags {
                source: flags,
                small: flags.small,
                large: flags.large,
            }),
            physical_viewport: Some(viewport),
            portrait_textures: Some(&portrait_textures),
        };
        Ok(draw(&Context {
            resources: &resources,
            windows: &windows,
            viewport,
            time,
        }))
    }

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    fn dump_overlay(
        self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        perspective: &perspective::Choice,
        omitted: Omissions,
        counter_settings: counters::CounterSettings,
        mode: &viewer_world::map::map_mode::MapMode,
    ) -> io::Result<DumpOverlay<'a>> {
        let layers = OverlayLayers::omitting(omitted);
        let tables = self.hour_tables(perspective, None)?;
        tables.warm_counter_flags(&self.icons.counters);
        let mut view = CounterView::new(counter_settings);
        view.set_mode(mode);
        let prepared = self.prepare_overlay_cached(camera, viewport, ui_scale, &tables, &view)?;
        Ok(DumpOverlay {
            hour: self,
            prepared,
            tables,
            camera,
            viewport,
            ui_scale,
            layers,
            batch: Batch::default(),
            extra: Batch::default(),
            sprites: Batch::default(),
            presented: Batch::default(),
            looks: None,
            labels: LabelReport::default(),
            counters: CounterReport::default(),
            hits: OverlayHits::default(),
            raids: raids::RaidReport::default(),
            settlement_warnings: Vec::new(),
        })
    }

    // weather
    /// Builds this hour's `SnowMudData` grid from its recorded display bytes.
    pub fn snow_mud(&self) -> io::Result<SnowMud> {
        self.product_hour.snow_grid(self.snow_cells()?)
    }

    /// The map's snow and mud cell composition ([`weather::SnowCells`]),
    /// built once for the content on first use.
    pub fn snow_cells(&self) -> io::Result<&weather::SnowCells> {
        Ok(&self.products.snow_content()?.cells)
    }

    // fog of war
    /// Builds this hour's `IntelMap` for one perspective.
    ///
    /// The province image and the definitions are static content, so the hour
    /// and the perspective are the only inputs that move; an hour whose
    /// [`perspective::Vision::province_fog`] is `None` gets the all-clear grid
    /// and a frame that draws no fog.
    pub fn intel_map(&self, perspective: &perspective::Choice) -> io::Result<IntelMap> {
        self.product_hour.intel_map(perspective)
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Content>();
    shared::<replay::campaign::Replay>();
    shared::<replay_frame::frame::DecodedFrame>();
};

/// The recorded battles in recorded order, each with the units of its member
/// provinces' land combats, attackers and defenders, summed over each member
/// as the game sums them, and, when `layers`, the sub-units by type of those
/// that are divisions, from `composition` (each template slot one sub-unit):
/// the game counts every unit but the slots of divisions only. Also counts the
/// member combats' divisions without a recorded composition or name. A
/// battle's sounds keep its object as their identity from hour to hour, so a
/// battle that moves in the list plays on.
#[cfg(test)]
fn recorded_battles(
    recorded: &[replay_data::rows::BattleSoundRow],
    combats: &[replay_data::rows::LandCombatRow],
    layers: bool,
    composition: impl Fn(u64) -> Option<Vec<(String, usize)>>,
) -> (Vec<viewer_audio::battle::Battle>, usize) {
    #[derive(Default)]
    struct Front {
        divisions: usize,
        sub_units: BTreeMap<String, usize>,
        unknown: usize,
    }
    let mut fronts: BTreeMap<u32, Front> = BTreeMap::new();
    for combat in combats {
        let front = fronts.entry(combat.province).or_default();
        front.divisions += combat.attackers.len() + combat.defenders.len();
        if !layers {
            continue;
        }
        let members = combat
            .attackers
            .iter()
            .zip(&combat.attacker_classes)
            .chain(combat.defenders.iter().zip(&combat.defender_classes));
        for (&division, _) in members.filter(|(_, class)| **class == DIVISION_CLASS) {
            match composition(division) {
                Some(units) => {
                    for (unit, count) in units {
                        *front.sub_units.entry(unit).or_default() += count;
                    }
                }
                None => front.unknown += 1,
            }
        }
    }
    let mut unknown = 0;
    let battles = recorded
        .iter()
        .map(|row| {
            let mut battle = viewer_audio::battle::Battle {
                // The game keeps a battle's sounds on its object.
                key: row.object,
                provinces: row.provinces.clone(),
                center: [f32::from_bits(row.center_x), f32::from_bits(row.center_z)],
                radius: f32::from_bits(row.radius),
                divisions: 0,
                sub_units: BTreeMap::new(),
            };
            for province in &row.provinces {
                let Some(front) = fronts.get(province) else {
                    continue;
                };
                battle.divisions += front.divisions;
                unknown += front.unknown;
                for (unit, count) in &front.sub_units {
                    *battle.sub_units.entry(unit.clone()).or_default() += count;
                }
            }
            battle
        })
        .collect();
    (battles, unknown)
}

/// The unit class of a division; a land combat's other units have no template.
#[cfg(test)]
const DIVISION_CLASS: u32 = 0;

/// A mean-time value of one: the evaluator's fixed five-decimal scale.
#[cfg(test)]
const MUSIC_CHANCE_ONE: f64 = 100_000.0;

/// Each song's chance from `evaluate`, which reads a mean-time body as fixed
/// five-decimal value: `None` for songs of disabled stations and, named, for a
/// body `evaluate` refuses.
#[cfg(test)]
fn song_chances(
    content: &viewer_audio::Content,
    mut evaluate: impl FnMut(&[installed_content_core::parser::Item]) -> Result<i64, String>,
) -> viewer_audio::Chances {
    let mut refusals = Vec::new();
    let weights = content
        .music
        .songs
        .iter()
        .enumerate()
        .map(|(index, song)| {
            if !content.enabled(index) {
                return None;
            }
            let name = &content.music.tracks[song.track].name;
            let value = evaluate(&song.chance)
                .map_err(|why| refusals.push(format!("song {name}: {why}")))
                .ok()?;
            Some(value as f64 / MUSIC_CHANCE_ONE)
        })
        .collect();
    viewer_audio::Chances { weights, refusals }
}

/// What one hour's convoy lines are kept by from the hour before: what the lines read, the viewer
/// they were built for, the lines built whole (their geometry, which the convoys are made from
/// again) and the convoys of the hour kept now.
pub struct KeptConvoys {
    read: TradeRouteRead,
    ours: u32,
    routes: Arc<viewer_world::map::trade_routes::TradeRoutes>,
    convoys: Arc<Vec<viewer_world::map::trade_routes::ConvoyLine>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_air_reuses_camera_independent_products_for_the_exact_selection() {
        let mut cached = OverlayAir {
            selected: Arc::new(Vec::new()),
            rows: 0,
            orders: Arc::default(),
            ranges: Arc::default(),
        };
        let initial = Arc::clone(&cached.orders);
        let (_, _) =
            cached.for_selection(&Arc::new(Vec::new()), 0, || panic!("unchanged selection"));
        let (empty, _) =
            cached.for_selection(&Arc::new(Vec::new()), 7, || panic!("no selection keeps the hour's products"));
        assert!(Arc::ptr_eq(&initial, &empty));
        let selected = Arc::new((0..5000).collect());
        let mut builds = 0;
        let mut build = || {
            builds += 1;
            (Default::default(), Default::default())
        };
        let (first, first_ranges) = cached.for_selection(&selected, 0, &mut build);
        let (same, same_ranges) = cached.for_selection(&Arc::new((*selected).clone()), 0, &mut build);
        assert!(Arc::ptr_eq(&first, &same));
        assert!(Arc::ptr_eq(&first_ranges, &same_ranges));
        let (newer, _) = cached.for_selection(&selected, 7, &mut build);
        assert!(!Arc::ptr_eq(&first, &newer), "newer rows at the head read the selection again");
        let mut reordered = (*selected).clone();
        reordered.swap(0, 1);
        let (changed, _) = cached.for_selection(&Arc::new(reordered), 7, &mut build);
        assert!(!Arc::ptr_eq(&newer, &changed));
        let (cleared, _) = cached.for_selection(&Arc::new(Vec::new()), 7, &mut build);
        assert!(!Arc::ptr_eq(&changed, &cleared));
        assert_eq!(builds, 4);
    }

    /// City sprawl keeps the recording's placement but draws its buildings' artwork from the
    /// visual mods, through the artwork fallback: a mod that replaces only a city house's mesh
    /// is drawn.
    #[test]
    fn city_sprawl_draws_the_visual_mods_artwork() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_sprawl_artwork_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |relative: &str, bytes: &[u8]| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        let mesh = |reach: f32| {
            let mut bytes = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
            for (name, values) in [
                ("p", [0_f32, 0., 0., 1., 0., 0., 0., 0., reach]),
                ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
            ] {
                bytes.extend([b'!', name.len() as u8]);
                bytes.extend(name.as_bytes());
                bytes.push(b'f');
                bytes.extend((values.len() as u32).to_le_bytes());
                bytes.extend(values.into_iter().flat_map(f32::to_le_bytes));
            }
            bytes.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
            bytes.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
            bytes
        };
        write(
            "base/gfx/models/city.gfx",
            br#"objectTypes = { pdxmesh = { name = house_mesh file = "gfx/models/house.mesh" scale = 1 } }"#,
        );
        write(
            "base/gfx/entities/house.asset",
            br#"entity = { name = house pdxmesh = house_mesh scale = 1 state = { name = idle } }"#,
        );
        write("base/gfx/models/house.mesh", &mesh(1.0));
        // The city damage thresholds the sprawl reads.
        write(
            "base/common/defines/00_graphics.lua",
            b"NDefines = { NGraphics = { CITY_DAMAGED_THRESHOLD = 0.8, CITY_DESTROYED_THRESHOLD = 0.3 } }",
        );
        write("art/gfx/models/house.mesh", &mesh(2.0));
        let art = root.join("art").display().to_string().replace('\\', "/");
        write(
            "user/mod/art.mod",
            format!("name=\"Art\"\npath=\"{art}\"\n").as_bytes(),
        );
        std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
        let options = ResolveOptions {
            game_dir: root.join("steamapps/common/Game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        };
        let base = root.join("base");
        let mut layers =
            installed_content_core::ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
        let selected = [installed_content_core::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        installed_content_core::content::visual::apply(&mut layers, &options, &selected, &[], &cancel)
            .unwrap();
        let plan = viewer_world::map::sprawl::SprawlPlan {
            buildings: vec![viewer_world::map::sprawl::SprawlBuilding {
                entity: "house".into(),
                position: [0.0; 3],
                rotation: 0.0,
                province: 1,
                owner: 1,
            }],
            urban_pixels: 1,
            samples: 1,
            entities: vec!["house".into()],
            bounds: Default::default(),
        };
        let defines = installed_content_core::defines::read(&layers).unwrap();
        let (sprawl, _) =
            load_sprawl(&layers, Default::default(), &plan, (&defines, &defines)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let slot = sprawl.library.slot("house").unwrap();
        assert!(sprawl.library.meshes[slot].visual);
    }

    /// Content whose visual mods changed while it loaded is loaded again, so the content that
    /// installs is what the mods hold as it installs; content without a revision is not checked.
    #[test]
    fn a_mod_changed_during_a_load_loads_it_again() {
        let mut disk = vec!["r2", "r2"].into_iter();
        let mut loads = 0;
        let mut mounted = "r1";
        let loaded = until_current(
            || {
                loads += 1;
                let revision = mounted.to_owned();
                mounted = "r2";
                Ok((loads, revision))
            },
            || disk.next().map(str::to_owned),
        )
        .unwrap();
        // The first load mounted r1; a mod changed to r2 before it returned, so it loaded again.
        assert_eq!(loaded, 2);
        let unchecked = until_current(|| Ok(((), String::new())), || Some("r9".into()));
        assert!(unchecked.is_ok());
    }

    /// A load shows no visual mods only for the decision its own key holds: this recording's,
    /// at the revision the mods hold now. A mod changed since, or another recording, shows them.
    #[test]
    fn a_load_follows_only_its_own_keys_decision() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_visual_mods_off_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |relative: &str, bytes: &str| {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write("art/interface/maparrows.txt", "arrowText = Broken");
        let art = root.join("art").display().to_string().replace('\\', "/");
        write(
            "user/mod/art.mod",
            &format!("name=\"Art\"\npath=\"{art}\"\n"),
        );
        std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
        let options = ResolveOptions {
            game_dir: root.join("steamapps/common/Game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        };
        let selected = [installed_content_core::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let revision =
            installed_content_core::content::visual::revision(&options, &selected, &[], &cancel).unwrap();
        let key = crate::visual_fallback::FallbackKey {
            recording: "A".into(),
            revision,
        };
        let probing = crate::visual_fallback::Decision::Probing {
            failures: ["scene: missing Broken technique".to_owned()].into(),
            excused: BTreeSet::new(),
            frames_needed: 0,
            frames_seen: 0,
        };
        let decisions = BTreeMap::from([(key.clone(), probing.clone())]);
        let decided = |recording| {
            decided(
                Some((recording, &decisions)),
                &options,
                &selected,
                &[],
                &cancel,
            )
        };
        assert_eq!(decided("A"), Some((key.clone(), probing)));
        assert_eq!(decided("B"), None);
        let restored = BTreeMap::from([(
            key.clone(),
            crate::visual_fallback::Decision::Restored {
                excused: BTreeSet::new(),
            },
        )]);
        assert_eq!(
            super::decided(Some(("A", &restored)), &options, &selected, &[], &cancel),
            None
        );
        write("art/interface/maparrows.txt", "arrowText = Repaired");
        assert_eq!(decided("A"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
    use replay_data::identity::Roster;

    /// The recorded battles keep their recorded order, centres and radii
    /// (not the provinces' combat order or a grouping of their own), and
    /// each sums the divisions and template slots of every member province's
    /// combats, counting what it cannot name.
    #[test]
    fn recorded_battles_keep_their_order_and_sum_their_members_combats() {
        // Unit 99 is a railway gun: it counts toward the battle's units but
        // has no template slots and is not an unknown division.
        let unit_class = |unit: u64| if unit == 99 { 13 } else { DIVISION_CLASS };
        let combat = |id: u64, province: u32, attackers: Vec<u64>, defenders: Vec<u64>| {
            replay_data::rows::LandCombatRow {
                id,
                province,
                attacker_classes: attackers.iter().map(|&unit| unit_class(unit)).collect(),
                defender_classes: defenders.iter().map(|&unit| unit_class(unit)).collect(),
                attackers,
                defenders,
                ..Default::default()
            }
        };
        let combats = [
            combat(30, 5, vec![1, 99], vec![2, 3]),
            combat(10, 7, vec![4], vec![1]),
            combat(20, 5, vec![2], vec![]),
            combat(40, 9, vec![4], vec![]),
        ];
        let composition = |division: u64| match division {
            1 | 2 => Some(vec![
                ("infantry".to_owned(), 7),
                ("artillery".to_owned(), 1),
            ]),
            4 => Some(vec![("infantry".to_owned(), 2)]),
            _ => None,
        };
        let row = |ordinal: u32, x: f32, radius: f32, provinces: Vec<u32>| {
            replay_data::rows::BattleSoundRow {
                ordinal,
                center_x: x.to_bits(),
                center_z: (-x).to_bits(),
                radius: radius.to_bits(),
                provinces,
                sounding: false,
                object: 0x1000 + u64::from(ordinal),
            }
        };
        // The game's list: province 9's battle first, then one covering
        // provinces 5 and 7, then a battle whose combats have ended.
        let recorded = [
            row(0, 90.0, 9.0, vec![9]),
            row(1, 60.0, 31.5, vec![5, 7]),
            row(2, 11.0, 9.0, vec![]),
        ];
        let (battles, unknown) = recorded_battles(&recorded, &combats, true, composition);
        let keys: Vec<_> = battles.iter().map(|battle| battle.key).collect();
        // Each battle keeps its recorded object as its identity.
        assert_eq!(keys, [0x1000, 0x1001, 0x1002]);
        assert_eq!(battles[1].center, [60.0, -60.0]);
        assert_eq!(battles[1].radius, 31.5);
        assert_eq!(battles[1].provinces, [5, 7]);
        assert_eq!(battles[0].divisions, 1);
        assert_eq!(battles[1].divisions, 7);
        assert_eq!(battles[1].sub_units["infantry"], 7 + 7 + 7 + 2 + 7);
        assert_eq!(battles[1].sub_units["artillery"], 4);
        assert_eq!(battles[2].divisions, 0);
        // Division 3 of province 5's combat has no composition.
        assert_eq!(unknown, 1);
        let (silent, unknown) = recorded_battles(&recorded, &combats, false, composition);
        assert!(silent.iter().all(|battle| battle.sub_units.is_empty()));
        assert_eq!(silent[1].divisions, 7);
        assert_eq!(unknown, 0);
    }

    /// Song chances: a disabled station's song is not drawable, an
    /// unanswerable body is unknown and named, and the evaluator's
    /// five-decimal value scales to a weight.
    #[test]
    fn song_chances_leave_refused_songs_unknown_and_skip_disabled_stations() {
        use installed_content_sound::music::{Music, Song, Track};
        let mut music = Music::default();
        for (index, (station, chance)) in [
            ("a", &b"factor = 2 modifier = { factor = 3 }"[..]),
            ("b", b"factor = 1"),
            ("a", b"factor = 1 add = 2"),
        ]
        .into_iter()
        .enumerate()
        {
            music.tracks.push(Track {
                name: format!("t{index}"),
                path: format!("music/t{index}.ogg"),
                volume: 1.0,
                source: String::new(),
            });
            music.songs.push(Song {
                track: index,
                station: station.into(),
                chance: installed_content_core::parser::parse(chance).unwrap().items,
                source: String::new(),
            });
        }
        let content = viewer_audio::Content::new(Default::default(), None).with_music(
            music,
            Some(vec!["a".into()]),
            0,
        );
        let mut calls = 0;
        let chances = song_chances(&content, |body| {
            calls += 1;
            if body.iter().any(|item| item.keyword_is("modifier")) {
                Err("unanswered".into())
            } else {
                Ok(body.len() as i64 * 150_000)
            }
        });
        // A refused song stays unknown rather than falling back to its base chance.
        assert_eq!(chances.weights, [None, None, Some(3.0)]);
        assert_eq!(chances.refusals, ["song t0: unanswered"]);
        assert_eq!(calls, 2, "each enabled song once");
    }

    #[test]
    fn intel_and_resource_overlays_keep_draw_order_and_raid_range() {
        let quad = |texture| Quad {
            texture,
            rect: [0.0; 4],
            uv: [0.0; 4],
            color: [1.0; 4],
            rotation: 0.0,
            layer: 0,
            shade: 0.0,
            exact: None,
            anchor: None,
            present: None,
        };
        let resource = Batch::loose(vec![quad(1)]);
        let raid = Batch::loose(vec![quad(2)]);
        let intel = Batch::loose(vec![quad(3)]);
        let labels = Batch::loose(vec![quad(4)]);
        let empty = Batch::default();
        let parts = OverlayParts {
            pings: &empty,
            bases: &empty,
            mode_icons: &resource,
            raids: &raid,
            intel: &intel,
            labels: &labels,
            counters: &empty,
            combats: &empty,
            naval_combats: &empty,
        };
        let mut batch = Batch::default();
        let layers = OverlayLayers::shown(&Default::default(), None);
        assert_eq!(compose(&mut batch, &parts, &layers), Some(1..2));
        assert_eq!(
            batch
                .quads
                .iter()
                .map(|quad| quad.texture)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        let layers = OverlayLayers {
            raids: false,
            ..layers
        };
        assert_eq!(compose(&mut batch, &parts, &layers), None);
        assert_eq!(
            batch
                .quads
                .iter()
                .map(|quad| quad.texture)
                .collect::<Vec<_>>(),
            [1, 3, 4]
        );
    }

    /// Each display switch leaves out only its own overlay part; a part shows only under its
    /// main switch, and hidden raid icons keep their hover.
    #[test]
    fn display_switches_leave_out_only_their_overlay_part() {
        let quad = |group: usize| Quad {
            texture: group,
            rect: [group as f32, 0.0, 1.0, 1.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
            rotation: 0.0,
            layer: group as u64,
            shade: 0.0,
            exact: None,
            anchor: None,
            present: None,
        };
        let groups: Vec<Batch> = (0..7)
            .map(|group| Batch::loose(vec![quad(group)]))
            .collect();
        let empty = Batch::default();
        let parts = OverlayParts {
            pings: &groups[0],
            bases: &groups[1],
            mode_icons: &empty,
            raids: &groups[2],
            intel: &empty,
            labels: &groups[3],
            counters: &groups[4],
            combats: &groups[5],
            naval_combats: &groups[6],
        };
        let drawn = |graphics: dashboard_core::protocol::MapGraphics| {
            let layers = OverlayLayers::shown(&graphics, None);
            let mut batch = Batch::default();
            let raids = compose(&mut batch, &parts, &layers);
            let textures: Vec<_> = batch.quads.iter().map(|quad| quad.texture).collect();
            (textures, raids, layers)
        };
        let all = dashboard_core::protocol::MapGraphics::default();
        assert_eq!(drawn(all).0, vec![0, 1, 2, 3, 4, 5, 6]);
        for (graphics, missing) in [
            (
                dashboard_core::protocol::MapGraphics {
                    theatre_pins: false,
                    ..all
                },
                0,
            ),
            (
                dashboard_core::protocol::MapGraphics {
                    base_icons: false,
                    ..all
                },
                1,
            ),
            (
                dashboard_core::protocol::MapGraphics {
                    raid_icons: false,
                    ..all
                },
                2,
            ),
            (
                dashboard_core::protocol::MapGraphics {
                    city_names: false,
                    ..all
                },
                3,
            ),
            (
                dashboard_core::protocol::MapGraphics {
                    land_combat_markers: false,
                    ..all
                },
                5,
            ),
            (
                dashboard_core::protocol::MapGraphics {
                    naval_markers: false,
                    ..all
                },
                6,
            ),
        ] {
            let (textures, _, _) = drawn(graphics);
            let expected: Vec<_> = (0..7).filter(|group| *group != missing).collect();
            assert_eq!(textures, expected, "{graphics:?}");
        }
        // The main switch takes every counter and icon part, but not city names.
        let (textures, raids, layers) = drawn(dashboard_core::protocol::MapGraphics {
            counters: false,
            ..all
        });
        assert_eq!(textures, vec![3, 4]);
        assert_eq!(raids, None);
        assert!(layers.raids_hovered);
        assert!(!layers.counters.land && !layers.counters.naval);
        assert!(!layers.counters.railway_guns && !layers.counters.ports);
        // Kind switches reach the counter producer, which keeps each one's hovers.
        let naval = OverlayLayers::shown(
            &dashboard_core::protocol::MapGraphics {
                naval_counters: false,
                ..all
            },
            None,
        );
        assert!(naval.counters.land && !naval.counters.naval);
        // A dump's label omission removes raid hovers with the icons.
        assert!(
            !OverlayLayers::omitting(Omissions {
                labels: true,
                ..Default::default()
            })
            .raids_hovered
        );
    }
    use replay_data::identity::Verification;

    #[test]
    fn every_unverified_identity_requires_override_and_keeps_its_reason() {
        let mut warnings = Vec::new();
        let state = Verification::Unverified {
            reason: "DLC roster differs".into(),
            launcher: Roster::default(),
            memory: Roster::default(),
        };
        assert!(check_verification("session 2", &state, false, &mut warnings).is_err());
        check_verification("session 2", &state, true, &mut warnings).unwrap();
        assert!(warnings[0].contains("session 2: DLC roster differs"));
        assert!(
            check_verification("session 3", &Verification::Unchecked, false, &mut warnings)
                .is_err()
        );
        check_verification("session 3", &Verification::Unchecked, true, &mut warnings).unwrap();
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    #[ignore = "read-only historical content load with dump GPU; requires BATTLE_PLAN_COMPAT_RECORDINGS, BATTLE_PLAN_GAME_DIR, BATTLE_PLAN_USER_DIR and BATTLE_PLAN_COMPAT_CACHE"]
    fn historical_coverage_document_load_keeps_uncovered_read_notes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        crate::render::probe_dump_device().unwrap();
        let path = |name| PathBuf::from(std::env::var_os(name).expect(name));
        let options = ResolveOptions {
            game_dir: path("BATTLE_PLAN_GAME_DIR"),
            user_dir: path("BATTLE_PLAN_USER_DIR"),
            cache_dir: path("BATTLE_PLAN_COMPAT_CACHE"),
        };
        let mut policies = BTreeSet::new();
        for path in std::env::var("BATTLE_PLAN_COMPAT_RECORDINGS").or_else(|_| std::env::var("HOI4_COMPAT_RECORDINGS")).unwrap().split(';') {
            let replay = Arc::new(Replay::open(fs::read(path).unwrap()).unwrap());
            let policy = replay.identity.coverage_policy;
            let (branch, hour) = match policy {
                2 => (1, 60827917),
                3 => (2, 60827796),
                _ => continue,
            };
            let source = dashboard_base::Source::from_replay(
                dashboard_base::SourceKey {
                    source_id: path.to_owned(),
                    revision: dashboard_core::protocol::replay_revision(Path::new(path)).unwrap(),
                },
                PathBuf::from(path),
                Arc::clone(&replay),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let document = Document::load_shared(
                Path::new(path),
                &options,
                false,
                &InitialHour {
                    branch: Some(branch),
                    hour: Some(hour),
                    hour_fraction: 0.0,
                },
                Arc::clone(&replay),
                &source,
                None,
                0,
                None,
                LoadTarget::Content,
            )
            .unwrap();
            let mut warnings = document.warnings.clone();
            document.uncovered_reads.append_to(&mut warnings);
            let notes: Vec<_> = warnings
                .iter()
                .filter(|note| note.starts_with("Content: "))
                .collect();
            assert_eq!(notes.len(), 1, "policy {policy}: {notes:?}");
            assert!(notes[0].contains("gfx/posteffect_volumes.txt"));
            assert!(notes[0].contains(&format!("coverage policy {policy};")));
            assert!(notes[0].contains("these bytes are not hash-verified"));
            for family in ["source/include file(s)", "font file(s)"] {
                assert!(
                    document.warnings.iter().any(|note| {
                        note.contains(family) && note.contains("not hash-verified")
                    }),
                    "policy {policy}: missing {family} uncovered-read note"
                );
            }
            policies.insert(policy);
        }
        assert_eq!(policies, BTreeSet::from([2, 3]));
    }
}
