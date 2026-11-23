use content_reader_politics::faction_templates::FactionTemplateProducts as _;
use viewer_layers_political::left_to_layers::LeftToLayers;

#[cfg(test)]
pub mod music_inputs;
mod trade_route_read;
use trade_route_read::TradeRouteRead;
use viewer_layer_core::scene_content::{OverlayLayers, SceneContent, links_of};

use crate::map_icons::MapIcons;
use crate::map_views::CounterView;
use crate::map_views::Views;
use dashboard_text::naval_battle_name::battle_name_candidates;
use viewer_combat as combat;
use viewer_gpu::{camera::MapSize, layers::Omissions};
use viewer_gpu::camera::{MapDefines, motion::CameraLaw};
use viewer_native_gui::free_icons;
use viewer_native_gui::gui_resources::GuiAssets;
use viewer_native_gui::gui_resources::GuiFlags;
use viewer_native_gui::gui_resources::viewport as gui_resources_viewport;
use viewer_native_gui::icon_windows::Retry;
use viewer_runtime::memo::Keyed;
use viewer_runtime::memo::Recent;
use viewer_runtime::memo::TableKey;
use viewer_runtime::memo::project;
use viewer_native_gui::native_resources::Context;
use viewer_native_gui::native_shader::draw as native_shader_draw;
use viewer_native_gui::native_windows::NativeWindows;
use viewer_gpu::province_schedule::Shared;
use viewer_native_gui::sprites::Quad;
use viewer_foundation::movement_timing::EtaCache;
use viewer_foundation::movement_timing::Etas;
use viewer_native_gui::tooltip_painter::TooltipPosition;
use viewer_native_gui::tooltip_painter::draw as tooltip_painter_draw;
use viewer_raids::map::raid_arrows;
#[cfg(any(test, feature = "test-support"))]
use viewer_map_text::name_pass;

use dashboard_base::content::check_verification;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use content_reader_layers::ResolveOptions;
use content_reader_map::map;
use replay::{campaign::Replay, campaign::SeekResult};
use replay_data::{BranchId, rows::tables as row_tables};
use replay_frame::frame::DecodedFrame;

use crate::playback::Playback;
use viewer_gpu::province_map::{HeightField, ProvinceMap};
use viewer_borders::{
    border_pass::{BorderAssets, Topology, strait_quads},
    strait_shader::StraitVertex,
    borders,
    demilitarized::Zone,
};
use map_display_data::landscape::Regions;
use viewer_terrain::rivers::{self, RiverAssets, RiverImage, RiverMesh};
use viewer_world::mapframe::FrameAssets;
use viewer_terrain::trees::{self, TreeAssets, TreeField, TreeGeometry, TreeImage};
use viewer_buildings::map_buildings::MapBuildingAssets;
use viewer_unit_models::unit_models::{ModelReport, UnitModelAssets};
use viewer_rail::railways::{self as rail_layer, Bridges, RailAssets, RailMesh};
use viewer_buildings::sprawl::{self, Sprawl};
use viewer_raids::map::intel_icons;
use viewer_raids::map::raids;
use viewer_gpu::camera::Camera;
use viewer_runtime::name_draw::Placements;
use viewer_native_gui::sprites::Batch;
use viewer_map_icons::labels::{self, LabelContent, LabelReport};
use viewer_map_text::name_pass::{NameContent, PlacementInputs};
use viewer_counter_kit::{CounterInputs, CounterReport};
use viewer_counters::counters;
use viewer_counters::counters::CounterContent;
use viewer_counter_products::map::orders;
use viewer_gpu::{province_geometry::ProvinceGeometry, ribbon_content::{OrderAssets, OrderField}};
use viewer_foundation::perspective;
use viewer_foundation::tables::Limitations;
use viewer_gpu::textures::IntelMap;
use map_display_data::SnowMud;

pub struct InitialHour {
    pub branch: Option<BranchId>,
    pub hour: Option<replay_data::Hour>,
    pub hour_fraction: f32,
}

pub struct Document {
    pub content: Arc<Content>,
    pub playback: Playback,
    pub hour_fraction: f32,
    pub demilitarized_override: BTreeSet<u32>,
}

impl std::ops::Deref for Document {
    type Target = Content;

    fn deref(&self) -> &Content {
        &self.content
    }
}

pub type VisualFallback<'a> = (
    &'a str,
    &'a BTreeMap<crate::visual_fallback::FallbackKey, crate::visual_fallback::Decision>,
);

fn decided(
    fallback: Option<VisualFallback<'_>>,
    options: &ResolveOptions,
    selected: &[content_reader_layers::content::visual::Choice],
    recorded: &[content_reader_layers::content::visual::RecordedMod],
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<(
    crate::visual_fallback::FallbackKey,
    crate::visual_fallback::Decision,
)> {
    let (recording, decisions) = fallback?;
    let key = crate::visual_fallback::FallbackKey {
        recording: recording.to_owned(),
        revision: content_reader_layers::content::visual::revision(options, selected, recorded, cancel)
            .ok()?,
    };
    let decision = decisions.get(&key)?;
    decision.reason().is_some().then(|| (key, decision.clone()))
}

fn visual_revision_now(
    options: &ResolveOptions,
    replay: &Replay,
    visual_mods: Option<&[content_reader_layers::content::visual::Choice]>,
) -> Option<String> {
    let selected = visual_mods.filter(|selected| !selected.is_empty())?;
    let recorded: Vec<_> = replay
        .identity
        .mods
        .iter()
        .map(|recorded| content_reader_layers::content::visual::RecordedMod {
            id: recorded.id.clone(),
            name: recorded.display_name.clone(),
        })
        .collect();
    content_reader_layers::content::visual::revision(
        options,
        selected,
        &recorded,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .ok()
}

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

fn load_sprawl(
    layers: &content_reader_layers::ResolvedLayers,
    decode: content_reader_image::native::Decode,
    plan: &sprawl::SprawlPlan,
    (defines, graphics): (
        &content_reader_defines::defines::Defines,
        &content_reader_defines::defines::Defines,
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

pub struct Content {
    loading: content_reader_layers::content::Loading,
    first_frame_reported: std::sync::atomic::AtomicBool,
    scene: SceneContent,
    pub combat: Arc<combat::Content>,
    pub counter_content: Arc<viewer_counter_products::map::counter_content::Content>,

    pub political_gpu: viewer_map_modes::political_gpu::Cache,
    pub scene_textures: viewer_world::scene::TextureCache,
    pub texture_quality: u8,
    pub visual_mods: Option<Vec<content_reader_layers::content::visual::Choice>>,
    pub visual_revision: String,
    pub fallback: Option<crate::visual_fallback::Built>,
    pub camera_law: CameraLaw,
    pub minimap_defines: Result<combat::minimap::Defines, String>,
    pub game_speeds: Result<viewer_gpu::frame::GameSpeeds, String>,
    pub game_display: Result<content_reader_settings::settings::Display, String>,
    pub game_options: content_reader_settings::settings::MapOptions,
    pub game_full_resolution: bool,
    pub game_strategic_names: bool,
    pub game_audio: dashboard_core::protocol::MapAudio,
    pub game_dev_master: f32,
    pub cursors: crate::cursors::Cursors,
    pub warnings: Vec<String>,
    pub uncovered_reads: content_reader_layers::content::UncoveredReads,
    pub audio: Option<Arc<viewer_audio::Source>>,
    pub source_path: PathBuf,
    pub border_assets: BorderAssets,
    pub straits: Vec<StraitVertex>,
    pub strategic_regions: Arc<BTreeMap<u32, content_reader_regions::regions::StrategicRegion>>,
    pub naval_dominance: viewer_map_modes::naval_dominance::Content,
    pub map_modes: Arc<viewer_map_modes::map_mode::Content>,
    pub border_trace: Duration,
    pub icons: MapIcons,
    pub names: NameContent,
    pub labels: LabelContent,
    pub resource_icons: Result<viewer_map_icons::resource_icons::ResourceIconContent, String>,
    pub adjacency_icons: viewer_map_icons::adjacency_icons::AdjacencyIcons,
    pub trade_routes: Result<viewer_unit_models::trade_routes::TradeRouteDefines, String>,
    air_assignment_colors: Result<viewer_map_modes::air_assignment::Colors, String>,
    occupation_stripe_alpha: Result<viewer_map_modes::occupation_stripes::Alpha, String>,
    naval_selection_colors: Result<viewer_map_modes::naval_selection::Colors, String>,
    naval_region_eligibility: BTreeSet<u32>,
    air_order_colors: Result<viewer_aircraft::air_orders::Colors, String>,
    air_transfer_color: Result<[f32; 4], String>,
    pub river_assets: RiverAssets,
    pub river_mesh: RiverMesh,
    pub river_trace: rivers::Trace,
    pub river_build: Duration,
    pub raid_hover: Arc<raids::tooltips::Content>,
    pub frame_assets: FrameAssets,
    pub tree_assets: TreeAssets,
    pub tree_geometry: TreeGeometry,
    pub tree_field: TreeField,
    pub tree_build: Duration,
    pub sprawl: Sprawl,
    pub sprawl_build: Duration,
    pub rail_geometry: rail_layer::RailGeometry,
    pub rail_build: Duration,
    pub orders: OrderAssets,
    eta_cache: Arc<EtaCache>,
    order_memo: Recent<(TableKey, u32, bool), OrderField>,
    trade_route_memo: Recent<(TableKey, u32), viewer_unit_models::trade_routes::TradeRoutes>,
    trade_route_stops: std::sync::Mutex<viewer_unit_models::trade_routes::KeptStops>,
    type_tint_memo: viewer_counter_kit::type_tint::Memo,
    mission_memo: counters::MissionMemo,
    overlay_carry: counters::Carry,
    pub province_geometry_build: Duration,
}

pub struct Hour<'a> {
    pub content: &'a Content,
    pub frame: &'a DecodedFrame,
    pub hour_fraction: f32,
    pub(crate) product_hour: map_products::ProductHour<'a>,
    pool_limits: viewer_unit_models::equipment_graphics::LimitValues,
    worker_etas: Option<Arc<viewer_foundation::movement_timing::WorkerEtas>>,
    air_rosters: Option<Arc<crate::playback::AirRosters>>,
    moved_hours: Option<Arc<viewer_unit_reads::counter_move::MovedHours>>,
}

impl std::ops::Deref for Hour<'_> {
    type Target = Content;

    fn deref(&self) -> &Content {
        self.content
    }
}

impl std::ops::Deref for Content {
    type Target = SceneContent;

    fn deref(&self) -> &Self::Target {
        &self.scene
    }
}

impl Content {
    pub fn naval_dominance_sources(&self, frame: &DecodedFrame, ours: u32, region: u32) -> viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap {
        counters::naval_dominance_sources(frame, &self.map, &self.counters, ours, region, self.icons.raids.movement_reads(), self.icons.raids.mission_topology())
    }

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
                    viewer_unit_reads::air_transport_content::AirTransportContent {
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

fn note_scope(schedule: &viewer_gpu::province_schedule::Schedule, bounds: &BTreeMap<u32, [f32; 4]>) {
    use viewer_gpu::province_schedule::Update;
    use viewer_runtime::redraws::note;
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
    viewer_runtime::view_record::set("overlay", "provinces_in_view", in_view);
    viewer_runtime::view_record::set("overlay", "provinces_dirty_off_screen", dirty);
}

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

#[derive(Default)]
pub struct OverlayHits {
    pub raids: Vec<raids::tooltips::Hit>,
    pub intel: Vec<intel_icons::IntelHit>,
    pub occupation: Vec<viewer_map_icons::occupation_icons::Hit>,
    pub adjacency: Vec<viewer_map_icons::adjacency_icons::Hit>,
}

impl OverlayHits {
    pub fn intel_at(
        &self,
        seen: impl Fn(&intel_icons::IntelHit) -> Option<([f32; 2], u32)>,
    ) -> Option<&intel_icons::IntelHit> {
        intel_icons::hit_seen(&self.intel, seen)
    }
}

enum OverlayCounters {
    Prepared(counters::PreparedCounters),
    #[cfg(test)]
    Empty,
}

impl std::ops::Deref for OverlayCounters {
    type Target = counters::PreparedCounters;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Prepared(icons) => icons,
            #[cfg(test)]
            Self::Empty => unreachable!("test overlay has no counter layout"),
        }
    }
}

impl std::ops::DerefMut for OverlayCounters {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Prepared(icons) => icons,
            #[cfg(test)]
            Self::Empty => unreachable!("test overlay has no counter layout"),
        }
    }
}

impl OverlayCounters {
    fn keep_drawn(&mut self, previous: &mut Self) {
        match (self, previous) {
            (Self::Prepared(now), Self::Prepared(old)) => now.keep_drawn(old),
            #[cfg(test)]
            _ => {}
        }
    }

    fn keep_drawn_by_value(&mut self, previous: &mut Self) {
        match (self, previous) {
            (Self::Prepared(now), Self::Prepared(old)) => now.keep_drawn_by_value(old),
            #[cfg(test)]
            _ => {}
        }
    }
}

pub struct PreparedOverlay {
    pub radar: Arc<viewer_aircraft::radar_symbols::Field>,
    pub air_ranges: Arc<viewer_aircraft::air_ranges::Fields>,
    pub air_assignment: viewer_map_modes::air_assignment::Field,
    pub occupation_stripes: viewer_map_modes::air_assignment::Field,
    pub naval_selection: viewer_map_modes::naval_selection::Field,
    pub air_orders: Arc<viewer_aircraft::air_orders::Prepared>,
    labels: labels::PreparedLabels,
    raids: Batch,
    native_raids: raids::PreparedRaids,
    icons: OverlayCounters,
    label_report: LabelReport,
    raid_report: raids::RaidReport,
    native_intel: intel_icons::PreparedIntel,
    intel: Batch,
    intel_report: intel_icons::IntelReport,
    lazy_version: u64,
    engaged_covered: bool,
    layers: [usize; 6],
    counter_view: CounterView,
    pub(crate) laid_icons: LaidIcons,
    adjacency_icons: viewer_map_icons::adjacency_icons::PreparedAdjacencyIcons,
    adjacency_priority: i8,
    mode_icons: Batch,
    mode_icon_refused: BTreeMap<String, usize>,
    tables: usize,
}

pub struct Still {
    labels: labels::PreparedLabels,
    air_assignment: viewer_map_modes::air_assignment::Field,
    air_orders: Arc<viewer_aircraft::air_orders::Prepared>,
    air_ranges: Arc<viewer_aircraft::air_ranges::Fields>,
    occupation_stripes: viewer_map_modes::air_assignment::Field,
    naval_selection: viewer_map_modes::naval_selection::Field,
    radar: Arc<viewer_aircraft::radar_symbols::Field>,
    engaged_covered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadeBy {
    Today,
    Left,
    Layer(u64),
}

pub(crate) struct KeptConvoys {
    read: TradeRouteRead,
    ours: u32,
    routes: Arc<viewer_unit_models::trade_routes::TradeRoutes>,
    convoys: Arc<Vec<viewer_unit_models::trade_routes::ConvoyLine>>,
}

const TRADE_ROUTE_TABLES: &[replay_data::rows::TableId] = &trade_route_read::TABLES;

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
    presented: Batch,
    looks: Option<free_icons::Looks>,
    pub labels: LabelReport,
    pub counters: CounterReport,
    pub hits: OverlayHits,
    pub raids: raids::RaidReport,
    pub settlement_warnings: Vec<String>,
}

impl DumpOverlay<'_> {
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

    pub fn prepared(&self) -> &PreparedOverlay {
        &self.prepared
    }

    pub fn looks(&self) -> Option<&free_icons::Looks> {
        self.looks.as_ref()
    }

    pub fn naval_dominance_sources(
        &self,
    ) -> Arc<viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap> {
        self.tables.naval_dominance_sources()
    }

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
                viewer_native_gui::sprites::dump_sprites(
                    &self.batch,
                    &self.extra,
                    &mut self.sprites,
                );
                if self.camera.free().is_none() {
                    self.looks = None;
                    return Ok(&self.sprites);
                }
                let ground = free_icons::Ground {
                    field: self.hour.heights(),
                    sight: self.hour.assets.terrain_sight(),
                };
                let at = viewer_gpu::icon_tracks::Head { hour: self.hour.frame.hour, fraction: 0.0 };
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
    pub fn motion_layout(&self) -> Option<&viewer_unit_reads::counter_move::DrawnLayout> {
        self.icons.motion_layout()
    }

    pub fn set_motion_continuity(&mut self, continuity: u64) {
        self.icons.set_motion_continuity(continuity);
    }

    pub fn set_pressed(
        &mut self,
        pressed: Option<viewer_runtime::unit_handle::Unit>,
        plate_hovered: Option<viewer_runtime::unit_handle::Unit>,
    ) {
        self.counter_view.pressed = pressed;
        self.counter_view.plate_hovered = plate_hovered;
    }

    pub fn raid_view(&self) -> (Option<(u32, Arc<raids::tooltips::Details>)>, Option<u32>) {
        (self.counter_view.raid_details.clone(), self.counter_view.raid_hovered)
    }

    pub fn set_raid_view(&mut self, (details, hovered): (Option<(u32, Arc<raids::tooltips::Details>)>, Option<u32>)) {
        self.counter_view.raid_details = details;
        self.counter_view.raid_hovered = hovered;
    }

    pub fn set_counter_view(&mut self, view: CounterView) {
        self.counter_view = view;
    }

    pub fn carry_plates(&mut self, old: &mut PreparedOverlay) {
        if self.lazy_version == old.lazy_version {
            self.icons.keep_drawn_by_value(&mut old.icons);
        }
    }
}

pub trait IconHost {
    fn lay_resource_icons(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        visibility: viewer_foundation::mode_visibility::Visibility,
        icons: &viewer_map_icons::resource_icons::HourIcons,
    ) -> viewer_map_icons::resource_icons::PreparedResourceIcons;

    fn lay_occupation_icons(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        mode: Option<viewer_map_icons::occupation_icons::Mode>,
        country: u32,
        marks: &viewer_map_icons::occupation_icons::Marks,
    ) -> viewer_map_icons::occupation_icons::PreparedOccupationIcons;
}

pub struct LaidIcons {
    pub(crate) resource_icons: viewer_map_icons::resource_icons::PreparedResourceIcons,
    pub(crate) occupation_icons: viewer_map_icons::occupation_icons::PreparedOccupationIcons,
    pub(crate) resource_icons_by: MadeBy,
    pub(crate) occupation_icons_by: MadeBy,
}

impl Default for LaidIcons {
    fn default() -> Self {
        Self {
            resource_icons: Default::default(),
            occupation_icons: Default::default(),
            resource_icons_by: MadeBy::Left,
            occupation_icons_by: MadeBy::Left,
        }
    }
}

impl LaidIcons {
    pub fn resource_icons_by(&self) -> MadeBy {
        self.resource_icons_by
    }

    pub fn occupation_icons_by(&self) -> MadeBy {
        self.occupation_icons_by
    }

    #[allow(clippy::too_many_arguments, reason = "the camera, view and layer product")]
    pub fn replace_resource_icons(
        &mut self,
        hour: &impl IconHost,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        visibility: viewer_foundation::mode_visibility::Visibility,
        icons: &viewer_map_icons::resource_icons::HourIcons,
        version: u64,
    ) {
        self.resource_icons = hour.lay_resource_icons(camera, viewport, ui_scale, visibility, icons);
        self.resource_icons_by = MadeBy::Layer(version);
    }

    #[allow(clippy::too_many_arguments, reason = "the camera, viewing country, mode and layer product")]
    pub fn replace_occupation_icons(
        &mut self,
        hour: &impl IconHost,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        mode: Option<viewer_map_icons::occupation_icons::Mode>,
        country: u32,
        marks: &viewer_map_icons::occupation_icons::Marks,
        version: u64,
    ) {
        self.occupation_icons = hour.lay_occupation_icons(camera, viewport, ui_scale, mode, country, marks);
        self.occupation_icons_by = MadeBy::Layer(version);
    }
}

impl IconHost for Hour<'_> {
    fn lay_resource_icons(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        visibility: viewer_foundation::mode_visibility::Visibility,
        icons: &viewer_map_icons::resource_icons::HourIcons,
    ) -> viewer_map_icons::resource_icons::PreparedResourceIcons {
        self.resource_icons_with(camera, viewport, ui_scale, visibility, |inputs| {
            viewer_map_icons::resource_icons::lay_out(inputs, icons)
        })
    }

    fn lay_occupation_icons(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        mode: Option<viewer_map_icons::occupation_icons::Mode>,
        country: u32,
        marks: &viewer_map_icons::occupation_icons::Marks,
    ) -> viewer_map_icons::occupation_icons::PreparedOccupationIcons {
        self.occupation_icons_with(camera, viewport, ui_scale, mode, country, |inputs| {
            viewer_map_icons::occupation_icons::lay_out(inputs, marks)
        })
    }
}

impl PreparedOverlay {
    fn finish_project(
        &mut self,
        mut placed: Self,
        hour: &impl IconHost,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        ours: u32,
        lay_ins: Option<&crate::side::LayIns>,
    ) {
        placed.icons.keep_drawn(&mut self.icons);
        if let Some(lay_ins) = lay_ins {
            lay_ins.lay(&mut placed, hour, camera, viewport, ui_scale, ours);
        }
        *self = placed;
    }

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
        lay_ins: Option<&crate::side::LayIns>,
    ) -> io::Result<()> {
        if tables.identity() != self.tables {
            return Err(io::Error::other(format!(
                "the overlay laid out over the tables of build {} was placed again over build {}'s: a layout is placed over its own publication's tables",
                self.tables,
                tables.identity()
            )));
        }
        let mut still = self.take_still();
        let mut spans = viewer_runtime::profiling::BuilderSpans::new(false);
        match hour.prepare_overlay_with(camera, viewport, ui_scale, tables, view, rows, &mut spans, Some(&mut still)) {
            Ok(placed) => {
                self.finish_project(placed, hour, camera, viewport, ui_scale, tables.ours(), lay_ins);
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

    pub fn layers(&self) -> [usize; 6] {
        self.layers
    }

    pub fn tables(&self) -> usize {
        self.tables
    }

    pub fn retries(&self) -> Vec<Retry> {
        self.counter_view.presentation.windows.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).retries()
    }

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

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    pub fn draw_quiet(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        layers: &OverlayLayers,
        time: f32,
        pools: viewer_native_gui::icon_windows::Pools,
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
            moved: hour.moved_hours.as_deref().unwrap_or(&viewer_unit_reads::counter_move::NO_MOVED_HOURS),
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
        viewer_runtime::redraws::note(
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
            let _ = self.laid_icons.resource_icons.draw(context, resources, &mut scratch);
        }
        if pools.occupation {
            let _ = self.laid_icons.occupation_icons.draw(context, occupation, &mut scratch, &mut Vec::new());
        }
        if pools.adjacency_rules {
            self.adjacency_icons.draw_with_hits(context, self.adjacency_priority, adjacency_rules, &mut scratch, &mut BTreeMap::new(), &mut Vec::new());
        }
        views.set_quiet(false);
        counters
    }

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
            moved: hour.moved_hours.as_deref().unwrap_or(&viewer_unit_reads::counter_move::NO_MOVED_HOURS),
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
        let schedule_profile = viewer_runtime::profiling::span("d_schedule");
        inputs
            .schedule
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .present(&inputs.content.bounds, camera, viewport, time);
        if replay_frame::cpu::enabled() || viewer_runtime::view_record::on() {
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
        let resource_icons = &self.laid_icons.resource_icons;
        let occupation_icons = &self.laid_icons.occupation_icons;
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
        let owner_profile = viewer_runtime::profiling::FrameOwners::new();
        let (counters, other) = (
            {
                let _span = viewer_runtime::profiling::span("d_counters");
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
                        let _span = viewer_runtime::profiling::span("d_labels");
                        labels.draw_scheduled(context, &dirty, label_view);
                    });
                    let modes_profile = viewer_runtime::profiling::span("d_modes");
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
        let counters_branch = counters.as_ref().ok().copied().flatten();
        for sample in [counters_owner, counters_branch, other_owner]
            .into_iter()
            .chain(other_owners)
        {
            viewer_runtime::profiling::record_owner_sample(sample);
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
        let raids = if raid_range.is_some() || layers.raids_hovered { raid_hits } else { Vec::new() };
        *hits = OverlayHits { raids, intel: intel_hits, occupation: occupation_drawn, adjacency: adjacency_drawn };
        Ok(())
    }

    pub fn label_report(&self) -> &LabelReport {
        &self.label_report
    }

    pub fn raid_report(&self) -> &raids::RaidReport {
        &self.raid_report
    }

    pub fn mode_icon_warnings(&self) -> Vec<String> {
        self.mode_icon_refused
            .iter()
            .map(|(reason, count)| format!("Map icons: {count} not drawn: {reason}."))
            .collect()
    }

    pub fn province_stripes(&self) -> &viewer_map_modes::air_assignment::Field {
        use viewer_foundation::mode_visibility::PriorityTable;
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

    pub fn intel_report(&self) -> &intel_icons::IntelReport {
        &self.intel_report
    }

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
        self.laid_icons.resource_icons_by()
    }

    pub fn occupation_icons_by(&self) -> MadeBy {
        self.laid_icons.occupation_icons_by()
    }

    pub fn replace_resource_icons(&mut self, hour: &impl IconHost, camera: Camera, viewport: [u32; 2], ui_scale: f32, icons: &viewer_map_icons::resource_icons::HourIcons, version: u64) {
        self.laid_icons.replace_resource_icons(hour, camera, viewport, ui_scale, self.counter_view.visibility, icons, version);
    }

    #[allow(clippy::too_many_arguments, reason = "the camera, the viewing country and the layer's product")]
    pub fn replace_occupation_icons(&mut self, hour: &impl IconHost, camera: Camera, viewport: [u32; 2], ui_scale: f32, country: u32, marks: &viewer_map_icons::occupation_icons::Marks, version: u64) {
        self.laid_icons.replace_occupation_icons(hour, camera, viewport, ui_scale, self.counter_view.occupation, country, marks, version);
    }

    pub(crate) fn counter_view(&self) -> &CounterView {
        &self.counter_view
    }

}

struct OverlayParts<'a> {
    pings: &'a Batch,
    bases: &'a Batch,
    mode_icons: &'a Batch,
    raids: &'a Batch,
    intel: &'a Batch,
    labels: &'a Batch,
    counters: &'a Batch,
    combats: &'a Batch,
    naval_combats: &'a Batch,
}

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
    batch.append(parts.counters);
    if layers.land_combats {
        batch.append(parts.combats);
    }
    if layers.naval_markers {
        batch.append(parts.naval_combats);
    }
    raids
}

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
                && a.rotation.map(f32::to_bits) == b.rotation.map(f32::to_bits)
                && bits(&a.rect) == bits(&b.rect)
                && bits(&a.uv) == bits(&b.uv)
                && bits(&a.color) == bits(&b.color)
        })
}

pub fn select_movement(
    scene: &mut viewer_world::scene::Scene,
    device: &eframe::wgpu::Device,
    queue: &eframe::wgpu::Queue,
    content: &Content,
    frame: &DecodedFrame,
    ours: u32,
    selected: &BTreeSet<viewer_runtime::unit_handle::Unit>,
) -> std::io::Result<()> {
    use viewer_world::scene::SceneInput;
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
        if let Some(path) = std::env::var_os("BATTLEPLAN_CONTENT_COUNTS_FILE").or_else(|| std::env::var_os("HOI4_CONTENT_COUNTS_FILE")) {
            let counts = serde_json::json!({
                "stage": "complete first hour",
                "at": crate::perf_log::epoch_ms(),
                "artwork_pending": self.icons.lazy.has_pending(),
                "reads": content_reader_layers::content::io_counts::snapshot(),
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
    pub fn capture_mode(&self) -> replay_data::scope::CaptureMode {
        self.playback.replay().capture_mode
    }

    pub fn load(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        visual_mods: Option<&[content_reader_layers::content::visual::Choice]>,
        texture_quality: u8,
    ) -> io::Result<Self> {
        let _generation = content_reader_layers::content::Generation::inherit();
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
        if !matches!(
            document.playback.current(),
            crate::map_source::Current::Frame(_)
        ) {
            let (branch, hour) = (document.playback.branch(), document.playback.hour());
            document.playback.seek(branch, hour)?;
        }
        Ok(document)
    }

    pub fn bind_source(&mut self, source: &dashboard_base::Source) {
        self.playback
            .bind_map(crate::map_binding::MapBinding::new(source));
    }

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    pub fn load_shared(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        replay: Arc<Replay>,
        source: &dashboard_base::Source,
        visual_mods: Option<&[content_reader_layers::content::visual::Choice]>,
        texture_quality: u8,
        visual_fallback: Option<VisualFallback<'_>>,
        target: LoadTarget,
    ) -> io::Result<Self> {
        let _startup = crate::perf_log::StartupSpan::new("document").with_loaders(path);
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
                    let _stage = viewer_runtime::load_record::stage("document");
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
                let _stage = viewer_runtime::load_record::stage("document.revision_after_load");
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

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    fn load_shared_once(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        replay: Arc<Replay>,
        resolved: Arc<content_reader_layers::content::ResolvedContent>,
        visual_mods: Option<&[content_reader_layers::content::visual::Choice]>,
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
                tables,
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
        tables: Option<engine_executable::executable_tables::Tables>,
        resolved: Arc<content_reader_layers::content::ResolvedContent>,
        visual_mods: Option<&[content_reader_layers::content::visual::Choice]>,
        texture_quality: u8,
        visual_fallback: Option<VisualFallback<'_>>,
    ) -> io::Result<Self> {
        let _generation = content_reader_layers::content::Generation::inherit();
        let mut warnings = dashboard_base::metadata::build_warnings(&replay);
        warnings.extend(replay.compatibility_gaps());
        warnings.extend(replay.warnings.iter().cloned());
        if let Some(replay_recording::control::ControlRecord::Partial { reason, .. }) = replay.partial() {
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
        viewer_runtime::load_record::count("content loaded", 1, 0);
        let mut content = {
            let _stage = viewer_runtime::load_record::stage("document.recorded_stack");
            resolved.as_ref().clone()
        };
        content.layers = content.layers.with_own_overlay_diagnostics();
        content.append_changed_to(&mut warnings);
        let uncovered_reads = content.layers.report_uncovered_reads();
        let mut visual_revision = String::new();
        let mut fallback = None;
        if let Some(selected) = visual_mods.filter(|selected| !selected.is_empty()) {
            let recorded: Vec<_> = replay
                .identity
                .mods
                .iter()
                .map(|recorded| content_reader_layers::content::visual::RecordedMod {
                    id: recorded.id.clone(),
                    name: recorded.display_name.clone(),
                })
                .collect();
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let decided_stage = viewer_runtime::load_record::stage("document.visual_decided");
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
                    let _stage = viewer_runtime::load_record::stage("document.visual_apply");
                    content_reader_layers::content::visual::apply(
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
            tables,
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
        visual_mods: Option<&[content_reader_layers::content::visual::Choice]>,
        texture_quality: u8,
        replay: Arc<Replay>,
        tables: Option<engine_executable::executable_tables::Tables>,
        content: content_reader_layers::content::ResolvedContent,
        product_loader: &map_products::loader::Loader,
        mut warnings: Vec<String>,
        recorded_language: String,
        visual_revision: String,
        fallback: Option<crate::visual_fallback::Built>,
        uncovered_reads: content_reader_layers::content::UncoveredReads,
    ) -> io::Result<Self> {
        let serial = viewer_runtime::load_record::stage("content.serial");
        let game_version = replay.identity.game_version.clone();
        let content = Arc::new(content);
        std::thread::scope(|prefix| {
            let scripted_content = Arc::clone(&content);
            let scripted_language = recorded_language.clone();
            let h_scripted = content_reader_layers::parallel::spawn(prefix, move || {
                let _startup = crate::perf_log::StartupSpan::new("document_scripted_text");
                let _stage = viewer_runtime::load_record::stage("content.scripted");
                Arc::new(dashboard_base::catalogue::read_scripted_content(
                    scripted_content,
                    scripted_language,
                ))
            });
            let map = {
                let _stage = viewer_runtime::load_record::stage("content.map");
                product_loader.map()?
            };
            let (terrain, default_map) = {
                let _stage = viewer_runtime::load_record::stage("content.terrain");
                product_loader.terrain()?
            };
            for reason in &terrain.skipped {
                warnings.push(format!(
                    "Terrain content the game logs and reads past: {reason}."
                ));
            }
            let defines = {
                let _stage = viewer_runtime::load_record::stage("content.defines");
                product_loader.defines()?
            };
            let audio = {
                let _stage = viewer_runtime::load_record::stage("content.audio");
                product_loader.audio(&defines, &options.user_dir, &mut warnings)
            };
            let art_defines = {
                let _stage = viewer_runtime::load_record::stage("content.graphics_defines");
                defines.graphics_for(&content.layers)
            };
            let game_speeds = viewer_gpu::frame::GameSpeeds::load(&defines);
            let defines_for = |stack: &content_reader_layers::ResolvedLayers| {
                if stack.shows_visual_mods() {
                    &art_defines
                } else {
                    &defines
                }
            };
            let settings = content_reader_settings::settings::read_keys(&options.user_dir)?;
            let camera_tilt = viewer_gpu::camera::stored_tilt(
                settings
                    .f32_or_constructed(
                        &["camera_tilt"],
                        content_reader_settings::settings::constructed::CAMERA_TILT,
                    )
                    .ok_or_else(|| invalid("settings.txt camera_tilt is not a number"))?,
            );
            let game_full_resolution = settings
                .bool_at(&["maprenderingoptions", "draw_map_full_res"])
                .unwrap_or(true);
            let game_strategic_names = settings.bool_at(&["strategic_regions"]).unwrap_or(false);
            let game_audio = dashboard_core::map_settings::game_audio(&settings);
            let game_dev_master = dashboard_core::map_settings::game_dev_master(&settings);
            let cursors = crate::cursors::Cursors::load_from_tables(tables, |path| {
                content.layers.read_uncovered(path)
            });
            let map_defines = viewer_world::map_assets::graphics_define(
                &content.layers,
                "camera and map",
                &art_defines,
                &defines,
                |defines| MapDefines::load(defines, camera_tilt),
            )?;
            let camera_law = viewer_world::map_assets::graphics_define(
                &content.layers,
                "camera motion",
                &art_defines,
                &defines,
                |defines| CameraLaw::load(defines, &settings),
            )?;
            let minimap_defines = viewer_world::map_assets::graphics_define(
                &content.layers,
                "minimap",
                &art_defines,
                &defines,
                combat::minimap::Defines::load,
            )
            .map_err(|error| error.to_string());
            let decode = settings.texture_decode();
            if settings.is_absent() {
                warnings.push(format!(
                "settings.txt is not in {}: the map uses the game's own starting settings, as the game does until it first saves them.",
                options.user_dir.display()
            ));
            }
            let (game_options, unread) = content_reader_settings::settings::MapOptions::read(&settings);
            warnings.extend(
                unread
                    .into_iter()
                    .map(|line| format!("Map options: {line}.")),
            );
            let game_display = content_reader_settings::settings::read_pdx(&options.user_dir)
                .and_then(|pdx| {
                    content_reader_settings::settings::Display::read(
                        &settings,
                        pdx.as_ref().map(|pdx| pdx.items.as_slice()),
                    )
                })
                .map_err(|error| error.to_string());
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
                            display.effective_gui_scale(content_reader_settings::settings::Desktop {
                                physical: CONSTRUCTED,
                                dpi: Ok(96),
                            });
                        (frame, display.interface_to_frame(frame, scale))
                    }
                    Err(_) => (CONSTRUCTED, 1.0),
                };
                native_shader_draw::gui_viewport(frame, scale)
            };
            let content_reader_regions::regions::StrategicRegions {
                regions,
                notes: region_notes,
            } = {
                let _stage = viewer_runtime::load_record::stage("content.regions");
                content_reader_regions::regions::read(&content.layers)?
            };
            warnings.extend(region_notes);
            drop(serial);
            let mut dag = content_reader_layers::parallel::dag::Dag::default();
            let layers = &content.layers;
            let h_assets = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.assets");
                let _startup = crate::perf_log::StartupSpan::new("document_assets");
                let mut warnings = Vec::new();
                let assets = viewer_world::map_assets::load_with_products(
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
            let h_icons = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.icons");
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
                let _stage = viewer_runtime::load_record::stage("wave1.names");
                let _startup = crate::perf_log::StartupSpan::new("document_names");
                layers.artwork("map names", |stack| {
                    NameContent::load_with_localisation(stack, &map, || {
                        product_loader.localisation(stack, &recorded_language)
                    })
                })
            });
            let h_labels = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.labels");
                Ok(Arc::new(combat::naval_labels::RecordedLabels::prepare(
                    recorded_language.clone(),
                    &recorded_language,
                    layers,
                )))
            });
            let h_counters = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.counters");
                viewer_world::map_assets::graphics_define(
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
                let _stage = viewer_runtime::load_record::stage("wave1.borders_in");
                let border_assets = layers.artwork("borders", |stack| {
                    BorderAssets::load(
                        stack,
                        defines_for(stack),
                        [map.width, map.height],
                        texture_quality,
                        decode,
                    )
                })?;
                let states = content_reader_states::states::read(layers, &defines, &map.definitions)?;
                let adjacencies = content_reader_map::adjacencies::read(layers)?;
                Ok((border_assets, states, adjacencies))
            });
            let h_rail_reads = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_runtime::load_record::stage("wave1.rail_reads");
                let (cities, city_styles) = content_reader_map::cities::read(layers)?;
                let buildings_read = content_reader_map::positions::read_buildings(layers)?;
                let building_types = product_loader.buildings()?;
                let rail_defines = viewer_world::map_assets::graphics_define(
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
                let _stage = viewer_runtime::load_record::stage("wave1.trains");
                let (trains, warnings) = layers.artwork("supply trains", |stack| {
                    let mut warnings = Vec::new();
                    viewer_rail::trains::TrainAssets::load(
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
                let _stage = viewer_runtime::load_record::stage("wave1.aircraft");
                let (aircraft, warnings) = layers.artwork("aircraft", |stack| {
                    let mut warnings = Vec::new();
                    viewer_aircraft::aircraft::AircraftAssets::load(
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
                let _stage = viewer_runtime::load_record::stage("wave1.bridges");
                layers.artwork("railway bridges", |stack| {
                    let mut warnings = Vec::new();
                    Bridges::load(stack, decode, defines_for(stack), &mut warnings)
                        .map(|bridges| (bridges, warnings))
                })
            });
            let h_units = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_runtime::load_record::stage("wave1.units");
                let _startup = crate::perf_log::StartupSpan::new("document_units");
                let stack = layers.presentation();
                let mut warnings = Vec::new();
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
                let _stage = viewer_runtime::load_record::stage("wave1.orders");
                layers.artwork("orders and arrows", |stack| {
                    let mut warnings = Vec::new();
                    OrderAssets::load(stack, decode, (&defines, defines_for(stack)), &mut warnings)
                        .map(|order_assets| (order_assets, warnings))
                })
            });
            let h_weather = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_runtime::load_record::stage("wave1.weather");
                let region_provinces = viewer_weather_effects::weather_effects::RegionProvinces::new(
                    &regions,
                    |province| map.definitions.contains_key(&province),
                );
                layers.artwork("weather effects", |stack| {
                    viewer_weather_effects::weather_effects::Assets::load(
                        stack,
                        defines_for(stack),
                        [map.width, map.height],
                        region_provinces.clone(),
                        decode,
                    )
                })
            });
            let h_mapbuildings = dag.add(&[], || -> io::Result<_> {
                let _stage = viewer_runtime::load_record::stage("wave1.mapbuildings");
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
                let _stage = viewer_runtime::load_record::stage("wave1.geometry");
                let started = Instant::now();
                let geometry = ProvinceGeometry::build(&map);
                Ok((geometry, started.elapsed()))
            });
            let h_river_assets = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.river_assets");
                layers.artwork("rivers", |stack| {
                    RiverAssets::load(stack, [map.width, map.height], texture_quality, decode)
                })
            });
            let h_rail_assets = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.rail_assets");
                layers.artwork("railways", |stack| {
                    RailAssets::load(stack, [map.width, map.height], decode)
                })
            });
            let h_ideologies = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.ideologies");
                product_loader.ideologies()
            });
            let h_resources = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.resources");
                product_loader.resources()
            });
            let h_initial = dag.add(&[], || {
                let _stage = viewer_runtime::load_record::stage("wave1.initial");
                Self::initial_playback(replay, initial).map(std::sync::Mutex::new)
            });
            let h_initial_flags = {
                let playback = h_initial.clone();
                let icons = h_icons.clone();
                dag.add(&[playback.id(), icons.id()], move || {
                    let _stage = viewer_runtime::load_record::stage("wave2.initial_flags");
                    if let crate::map_source::Current::Frame(frame) = playback
                        .read()?
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .current()
                    {
                        let _ = counters::initial_flags::warm_initial_flags(frame, &icons.read()?.counters);
                    }
                    Ok(())
                })
            };
            let h_movement = {
                let (defines, regions) = (&defines, &regions);
                let h_icons = h_icons.clone();
                let h_counters = h_counters.clone();
                dag.add(&[h_icons.id(), h_counters.id()], move || {
                    let _stage = viewer_runtime::load_record::stage("wave2.movement");
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
                    let _stage = viewer_runtime::load_record::stage("wave2.labels");
                    viewer_world::map_assets::graphics_define(
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
                    let _stage = viewer_runtime::load_record::stage("wave2.borders");
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
                let (defines, art_defines) = (&defines, &art_defines);
                let h_assets = h_assets.clone();
                dag.add(&[h_assets.id()], move || -> io::Result<_> {
                    let _stage = viewer_runtime::load_record::stage("wave2.rivers");
                    let (assets, _) = h_assets.read()?;
                    let heights = HeightField {
                        width: assets.heightmap.width,
                        height: assets.heightmap.height,
                        values: assets.heightmap_texels(),
                    };
                    load_rivers(
                        layers,
                        &art_defines,
                        &defines,
                        &RiverImage {
                            width: assets.rivers.width,
                            height: assets.rivers.height,
                            indices: &assets.rivers.indices,
                        },
                        &heights,
                    )
                })
            };
            let h_trees = {
                let h_assets = h_assets.clone();
                dag.add(&[h_assets.id()], move || {
                    let _stage = viewer_runtime::load_record::stage("wave2.trees");
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
                    let _stage = viewer_runtime::load_record::stage("wave2.sprawl");
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
                    let _stage = viewer_runtime::load_record::stage("wave2.railgeom");
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
                    let _stage = viewer_runtime::load_record::stage("wave1.frame");
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
            let products_stage = viewer_runtime::load_record::stage("products");
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
            let icon_windows = viewer_runtime::load_record::stage("wave1.icon_windows");
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
            raid_assets.trigger_content.triggers = Arc::new(combat_assets.set_count_trigger_names(
                &building_types,
                &ideologies,
                &resources,
            ));
            raid_assets.trigger_content.ideologies = Some(ideologies.clone());
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
            let (river_mesh, river_trace, river_build, river_warning) = h_rivers.take()?;
            let (tree_field, tree_assets, tree_geometry, tree_build, notes) = h_trees.take()?;
            let mut wave2_warnings = notes;
            wave2_warnings.extend(river_warning);
            let (sprawl, sprawl_build, notes) = h_sprawl.take()?;
            wave2_warnings.extend(notes);
            let (rail_geometry, rail_build) = h_railgeom.take()?;
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
            let _rest = viewer_runtime::load_record::stage("content.rest");
            raid_assets.trigger_content.terrain = movement.clone();
            let map_modes = {
                let mode_layers = product_loader.reader(&content.layers, "display/modes");
                let scripted_modes = content_reader_map::scripted_map_modes::read(&mode_layers)?;
                for (field, count) in scripted_modes.refusal_counts() {
                    warnings.push(format!(
                        "Scripted map modes: {count} unread entries {field}"
                    ));
                }
                let categories = content_reader_state_categories::state_categories::read(&mode_layers)?;
                let infrastructure =
                    viewer_map_modes::map_mode::infrastructure::Content::new(&building_types);
                let terrain_categories = combat_assets.movement_terrain();
                let provinces: Vec<u32> = map.definitions.keys().copied().collect();
                let province_terrain = |province| {
                    movement
                        .as_ref()
                        .and_then(|movement| movement.map.terrain(province))
                };
                let ideologies = Arc::new(counters.ideologies.clone());
                Arc::new(viewer_world::map_assets::graphics_define(
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
                        (Ok(categories), Some(_)) => Ok(viewer_map_modes::map_mode::terrain::Sources {
                            categories,
                            provinces: &provinces,
                            terrain: &province_terrain,
                            region_of: &counters.region_of,
                            regions: &regions,
                        }),
                    };
                        viewer_map_modes::map_mode::Content::new(
                            defines,
                            Arc::clone(&ideologies),
                            &categories,
                            scripted_modes.clone(),
                            mode_layers.faction_templates(),
                            recorded_language.clone(),
                            terrain,
                            infrastructure.clone(),
                            viewer_map_modes::map_mode::strategic::Content::new(
                                defines, &regions,
                            )?,
                        )
                    },
                )?)
            };
            let scripted = content_reader_layers::parallel::join(h_scripted);
            let mut intel_assets = intel_icons::IntelAssets::load(&content.layers, &defines);
            intel_assets.scripted = Some(Arc::clone(&scripted));
            warnings.extend(intel_assets.warnings.iter().cloned());
            let mut counter_assets = counter_assets;
            let mut icon_images = icon_images;
            let occupation = viewer_world::map_assets::graphics_define(
                &content.layers,
                "occupation map icons",
                &art_defines,
                &defines,
                viewer_map_icons::occupation_icons::IconDefines::load,
            )
            .and_then(|defines| {
                Ok(viewer_map_icons::occupation_icons::Content {
                    definitions: content_reader_politics::occupation_modifiers::Definitions::read(
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
                        &viewer_map_icons::occupation_icons::gui_assets(occupation),
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
            if icon_images.len() >= viewer_native_gui::lazy_icons::DYNAMIC_BASE {
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
            if !border_meshes.one_pixel_provinces.is_empty() {
                warnings.push(format!(
                "{} provinces cover a single pixel of map/provinces.bmp; the game reports these too and no border is traced through them.",
                border_meshes.one_pixel_provinces.len()
            ));
            }
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
            warnings.push(
            "Meshes, animations and textures are read from gfx/models and gfx/entities, which are outside the content coverage policy and are not hash-verified; their .gfx and .asset descriptors are verified.".into(),
        );
            warnings.push("Railway geography limitations: tied crossing inputs and the initial tied-pixel collection order are unrecorded. Province visibility history and the railway manager's hash-table draw order are unrecorded; strips use all provinces in ascending order. Port selection uses recorded naval-base levels instead of strategic-navy membership. Custom adjacency overrides and the transient rail fade are not reconstructed; matching native geography and presentation snapshots would settle these cases.".into());
            warnings.push(
            "A division's model follows the engine's own ladder: the template's saved model, then the equipment pair's saved model, authored graphic pools and conventional candidates, else the viewing country's fallback queues, then the recorded model number and sub-unit key under the cosmetic tag, tag, graphical culture and bare name, then the recorded camouflage. Template composition and model choices are recorded hourly, so a template edit reaches its figures within the hour; a template whose model inputs were unreadable that hour draws no model for its divisions, and its counters are unaffected. Equipment is recorded daily, with an hourly record of divisions deployed since; an existing division's mid-day equipment change reaches its figure at the next daily refresh; the country's stored best-variant candidates are daily too. A no-pair fallback whose rows name different entities takes the second row of the game's labelled option list, its labels built in the display language from the recorded variant names and country tags; a label that cannot be built refuses the figure by name. The window advances authored state clips and attachment transforms with presentation seed 0x414e494d XOR the unit handle; original clocks and RNG ordering are unrecorded. A time-zero dump preserves the default first pose. A standing figure's facing is a hash, because nothing records the heading the engine last turned it to."
                .into(),
        );
            warnings.push(
            "Map buildings keep the destroyed-mesh hysteresis, and stay drawn above the update distance once created below it, from this viewer's own camera history since the last seek; a seek starts every slot healthy and uncreated, as a freshly loaded game does. A dump has no camera history unless --building-camera supplies it. Popup growth and the shared model creation allowance run on the viewer's frame clock; native capture-time presentation remains unrecorded."
                .into(),
        );
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
            let naval_dominance = viewer_map_modes::naval_dominance::Content::new(
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
            let resource_icons = viewer_map_icons::resource_icons::ResourceIconContent::load(
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
            let adjacency_icons = match content_reader_map::adjacency_rules::read(&content.layers) {
                Ok(rules) => {
                    viewer_map_icons::adjacency_icons::AdjacencyIcons::new(&rules, &defines)
                        .with_build(game_version)
                }
                Err(error) => {
                    warnings.push(format!(
                    "No strait or canal icons are drawn: the adjacency rules cannot be read: {error}"
                ));
                    viewer_map_icons::adjacency_icons::AdjacencyIcons::new(&[], &defines)
                        .with_build(game_version)
                }
            };
            let trade_routes = viewer_world::map_assets::graphics_define(
                &content.layers,
                "trade-route lines and convoys",
                &art_defines,
                &defines,
                viewer_unit_models::trade_routes::TradeRouteDefines::read,
            )
            .map_err(|error| error.to_string());
            let air_assignment_colors = viewer_world::map_assets::graphics_define(
                &content.layers,
                "Air assignment stripes",
                &art_defines,
                &defines,
                viewer_map_modes::air_assignment::Colors::read,
            )
            .map_err(|error| error.to_string());
            let occupation_stripe_alpha = viewer_world::map_assets::graphics_define(
                &content.layers,
                "occupation country stripes",
                &art_defines,
                &defines,
                viewer_map_modes::occupation_stripes::Alpha::read,
            )
            .map_err(|error| error.to_string());
            let naval_region_eligibility = viewer_map_modes::naval_selection::eligible_regions(
                &icons.raids.region_provinces,
                &icons.raids.geometry,
            );
            let naval_selection_colors = viewer_world::map_assets::graphics_define(
                &content.layers,
                "Navy local selection colours",
                &art_defines,
                &defines,
                viewer_map_modes::naval_selection::Colors::read,
            )
            .map_err(|error| error.to_string());
            let radar_centers = viewer_aircraft::radar_symbols::Centers::new(
                &icons.raids.trigger_content.state_provinces,
                &counters.bounds,
                map.width as i32,
            );
            let air_order_colors = viewer_world::map_assets::graphics_define(
                &content.layers,
                "Air mission arrows",
                &art_defines,
                &defines,
                viewer_aircraft::air_orders::Colors::read,
            )
            .map_err(|error| error.to_string());
            let air_transfer_color = viewer_world::map_assets::graphics_define(
                &content.layers,
                "Air transfer arrows",
                &art_defines,
                &defines,
                viewer_aircraft::air_transfers::color,
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
                    map_text: viewer_map_text::animated_map_text::events::Content::new(
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
            if let Some(path) = std::env::var_os("BATTLEPLAN_CONTENT_COUNTS_FILE").or_else(|| std::env::var_os("HOI4_CONTENT_COUNTS_FILE")) {
                let counts = serde_json::json!({
                    "reads": content_reader_layers::content::io_counts::snapshot(),
                    "graph": content.layers.content_counts(),
                });
                std::fs::write(path, serde_json::to_vec(&counts).map_err(io::Error::other)?)?;
            }
            let content = Content {
                scene: SceneContent {
                    tables,
                    products,
                    map,
                    map_defines,
                    assets,
                    border_meshes,
                    counters,
                    radar_centers,
                    map_buildings,
                    rail_assets,
                    aircraft,
                    trains,
                    bridges,
                    unit_models,
                    weather_effects,
                    movement,
                    province_geometry,
                },
                loading,
                first_frame_reported: std::sync::atomic::AtomicBool::new(false),
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
                visual_mods: visual_mods.map(<[content_reader_layers::content::visual::Choice]>::to_vec),
                visual_revision,
                fallback,
                texture_quality,
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
                warnings,
                uncovered_reads,
                audio,
                source_path: path.to_path_buf(),
                border_assets,
                straits,
                raid_hover,
                strategic_regions: regions,
                naval_dominance,
                map_modes,
                border_trace,
                icons,
                names,
                labels,
                river_assets,
                river_mesh,
                river_trace,
                river_build,
                frame_assets,
                tree_assets,
                tree_geometry,
                tree_field,
                tree_build,
                sprawl,
                sprawl_build,
                rail_geometry,
                rail_build,
                orders: order_assets,
                political_gpu: viewer_map_modes::political_gpu::Cache::default(),
                scene_textures: viewer_world::scene::TextureCache::default(),
                eta_cache,
                order_memo: Recent::new(),
                trade_route_memo: Recent::new(),
                trade_route_stops: std::sync::Mutex::default(),
                type_tint_memo: Keyed::new(viewer_counter_kit::type_tint::TABLES),
                mission_memo: Recent::new(),
                overlay_carry: counters::Carry::default(),
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

    pub fn from_content(content: Arc<Content>, replay: Arc<Replay>) -> io::Result<Self> {
        Ok(Self {
            content,
            playback: Playback::unloaded(replay)?,
            hour_fraction: 0.0,
            demilitarized_override: BTreeSet::new(),
        })
    }

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

    pub fn hour(&self) -> io::Result<Hour<'_>> {
        Ok(Hour::new(&self.content, self.frame()?, self.hour_fraction)
            .with_air_rosters(Some(self.playback.air_rosters())))
    }

    pub fn vision(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<Arc<perspective::Vision>> {
        let _cost = replay_frame::cpu::scope("viewer", Some("fog"));
        self.hour()?.vision(perspective)
    }

    pub fn viewing_tag(&self, vision: &perspective::Vision) -> io::Result<String> {
        self.hour()?.viewing_tag(vision)
    }

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
        viewer_buildings::map_buildings::MapBuildingSummary,
    )> {
        self.hour()?.map_building_field(perspective)
    }

    pub fn aircraft_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_aircraft::aircraft::AircraftPlan> {
        let _cost = replay_frame::cpu::scope("viewer", Some("air/naval source choices"));
        self.hour()?.aircraft_plan(perspective)
    }

    pub fn nuclear_impacts(&self) -> io::Result<viewer_models::strike_effects::Impacts> {
        let _cost = replay_frame::cpu::scope("viewer", Some("engagement/strikes/text"));
        self.hour()?.nuclear_impacts()
    }

    pub fn train_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_rail::trains::TrainPlan> {
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
    ) -> io::Result<(viewer_unit_history::unit_history::Field, ModelReport)> {
        self.hour()?
            .unit_model_field_sailing(perspective, 0.0, &|_| false)
    }

    pub fn unit_model_field_sailing(
        &self,
        perspective: &perspective::Choice,
        time: f32,
        shown: &dyn Fn(&viewer_unit_models::trade_routes::ConvoyDefines) -> bool,
    ) -> io::Result<(viewer_unit_history::unit_history::Field, ModelReport)> {
        self.hour()?
            .unit_model_field_sailing(perspective, time, shown)
    }

    pub fn intel_map(&self, perspective: &perspective::Choice) -> io::Result<IntelMap> {
        self.hour()?.intel_map(perspective)
    }

    pub fn place_names(
        &self,
        mode: &viewer_map_modes::map_mode::MapMode,
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
        counter_settings: viewer_counter_kit::CounterSettings,
        mode: &viewer_map_modes::map_mode::MapMode,
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

    pub fn frame(&self) -> io::Result<&DecodedFrame> {
        match self.playback.current() {
            crate::map_source::Current::Frame(frame) => Ok(frame),
            crate::map_source::Current::Gap {
                branch,
                hour,
                reason,
            } => Err(invalid(format!(
                "branch {branch}, hour {hour} is a gap: {reason}"
            ))),
            crate::map_source::Current::Refused {
                branch,
                hour,
                reason,
            } => Err(invalid(format!(
                "branch {branch}, hour {hour} cannot be shown: {reason}"
            ))),
        }
    }
}

#[derive(Clone)]
pub struct OverlayTables {
    counters: counters::HourTables,
    vision: Arc<perspective::Vision>,
    labels: Arc<map_display_data::landscape::Labels>,
    radar: Arc<viewer_aircraft::radar_symbols::Field>,
    air: Arc<std::sync::Mutex<OverlayAir>>,
    pub engaged_covered: bool,
    raids: Arc<raids::RaidIcons>,
    intel: IntelPart,
    built_for: (replay_data::BranchId, replay_data::Hour, u32),
    raid_arrows: Arc<OrderField>,
}

#[derive(Debug, Default)]
pub struct OverlayRows {
    rows: Option<Arc<DecodedFrame>>,
    missed: std::sync::Mutex<Vec<replay_data::rows::TableId>>,
    spent: std::sync::Mutex<Vec<(&'static str, u64, u64)>>,
    left: LeftToLayers,
}

impl OverlayRows {
    pub fn at_head(rows: Option<Arc<DecodedFrame>>) -> Self {
        Self { rows, missed: Default::default(), spent: Default::default(), left: LeftToLayers::default() }
    }

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

    fn identity(&self) -> usize {
        self.rows.as_ref().map_or(0, |rows| Arc::as_ptr(rows) as usize)
    }

    pub fn spent(&self) -> Vec<(&'static str, u64, u64)> {
        self.spent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub fn missed(&self) -> Vec<replay_data::rows::TableId> {
        self.missed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

#[derive(Clone)]
struct OverlayAir {
    selected: Arc<Vec<u64>>,
    rows: usize,
    orders: Arc<viewer_aircraft::air_orders::Prepared>,
    ranges: Arc<viewer_aircraft::air_ranges::Fields>,
}

impl OverlayAir {
    fn for_selection(
        &mut self,
        selected: &Arc<Vec<u64>>,
        rows: usize,
        build: impl FnOnce() -> (
            viewer_aircraft::air_orders::Prepared,
            viewer_aircraft::air_ranges::Fields,
        ),
    ) -> (
        Arc<viewer_aircraft::air_orders::Prepared>,
        Arc<viewer_aircraft::air_ranges::Fields>,
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
    pub fn warm_counter_flags(&self, assets: &counters::CounterAssets) {
        self.counters.warm_flags(assets);
    }

    pub fn naval_dominance_sources(
        &self,
    ) -> Arc<viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap> {
        self.counters.naval_dominance_sources()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn land_combat_gaps(
        &self,
    ) -> (
        &viewer_counter_kit::combat_markers::Gaps,
        &viewer_counter_kit::combat_markers::Gaps,
    ) {
        self.counters.land_combat_gaps()
    }

    pub fn counters(&self) -> &counters::HourTables {
        &self.counters
    }

    pub fn raids(&self) -> Arc<raids::RaidIcons> {
        Arc::clone(&self.raids)
    }

    pub fn ours(&self) -> u32 {
        self.built_for.2
    }

    pub fn raid_arrows(&self) -> &Arc<OrderField> {
        &self.raid_arrows
    }

    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.air) as *const () as usize
    }

    pub fn vision(&self) -> &Arc<perspective::Vision> {
        let _cost = replay_frame::cpu::scope("viewer", Some("fog"));
        &self.vision
    }

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
    fn icon_centroid(&self, province: u32) -> Option<[f32; 3]> {
        let (_, geography) = self.icons.raids.mission_topology().ok()?;
        let [x, z] = *geography.centroids.get(&province)?;
        let (x, z) = (x as f32, z as f32);
        Some([x, self.heights().surface_height(x, z), z])
    }

    fn resource_icons_with(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        visibility: viewer_foundation::mode_visibility::Visibility,
        lay: impl FnOnce(&viewer_map_icons::resource_icons::ResourceIconInputs<'_>) -> viewer_map_icons::resource_icons::PreparedResourceIcons,
    ) -> viewer_map_icons::resource_icons::PreparedResourceIcons {
        let Ok(content) = &self.resource_icons else { return Default::default() };
        lay(&viewer_map_icons::resource_icons::ResourceIconInputs {
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

    fn occupation_icons_with(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        mode: Option<viewer_map_icons::occupation_icons::Mode>,
        country: u32,
        lay: impl FnOnce(&viewer_map_icons::occupation_icons::Inputs<'_>) -> viewer_map_icons::occupation_icons::PreparedOccupationIcons,
    ) -> viewer_map_icons::occupation_icons::PreparedOccupationIcons {
        let Ok(content) = &self.icons.occupation else { return Default::default() };
        lay(&viewer_map_icons::occupation_icons::Inputs {
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
        spans: &mut viewer_runtime::profiling::BuilderSpans,
        intel: bool,
    ) -> io::Result<Arc<map_products::FogProduct>> {
        let _cost = replay_frame::cpu::scope("viewer", Some("fog"));
        self.product_hour.fog_with(choice, memos, spans, intel)
    }

    pub fn weather_field(&self) -> io::Result<map_display_data::WeatherField> {
        let _cost = replay_frame::cpu::scope("viewer", Some("weather"));
        self.product_hour.weather_field()
    }

    pub fn air_ranges(
        &self,
        viewer: u32,
        selected: &[u64],
    ) -> viewer_aircraft::air_ranges::Fields {
        self.air_ranges_from(self.frame, viewer, selected)
    }

    pub fn air_ranges_from(
        &self,
        frame: &DecodedFrame,
        viewer: u32,
        selected: &[u64],
    ) -> viewer_aircraft::air_ranges::Fields {
        let defines = &self.icons.raids.defines;
        viewer_aircraft::air_ranges::read_fields(
            frame,
            viewer,
            selected,
            viewer_unit_reads::air_range::Defines {
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

    pub fn with_air_rosters(
        mut self,
        air_rosters: Option<Arc<crate::playback::AirRosters>>,
    ) -> Self {
        self.air_rosters = air_rosters;
        self
    }

    pub fn with_moved_hours(
        mut self,
        moved_hours: Arc<viewer_unit_reads::counter_move::MovedHours>,
    ) -> Self {
        self.moved_hours = Some(moved_hours);
        self
    }

    pub fn trigger_world(&self) -> &dashboard_text::triggers::World<'a> {
        self.product_hour.trigger_world()
    }

    pub fn etas(&self) -> io::Result<Arc<Etas>> {
        let build = || self.eta_cache.etas(self.frame, self.movement.as_deref());
        match &self.worker_etas {
            Some(gate) => gate.get_or_try_init(build),
            None => build(),
        }
    }

    pub fn rail_mesh(&self) -> io::Result<RailMesh> {
        Ok(self.rail_product()?.0.clone())
    }

    pub fn labels(&self, ours: u32) -> io::Result<Arc<map_display_data::landscape::Labels>> {
        self.product_hour.labels(ours)
    }

    pub fn rail_product(&self) -> io::Result<Arc<map_products::landscape::RailProduct>> {
        self.product_hour.rails(&self.rail_geometry, &self.bridges)
    }

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

    pub fn trade_route_lines(&self, ours: u32) -> Arc<viewer_unit_models::trade_routes::TradeRoutes> {
        self.trade_route_memo
            .get_or_insert_with((TableKey::of(self.frame, TRADE_ROUTE_TABLES), ours), || {
                Ok::<_, io::Error>(Arc::new(self.build_trade_route_lines(ours)))
            })
            .unwrap_or_else(|why| {
                let mut routes = viewer_unit_models::trade_routes::TradeRoutes::default();
                routes.report.unavailable = Some(why.to_string());
                Arc::new(routes)
            })
    }

    pub(crate) fn trade_route_convoys_kept(&self, ours: u32, kept: &mut Option<KeptConvoys>) -> Arc<Vec<viewer_unit_models::trade_routes::ConvoyLine>> {
        use replay_data::rows::TableId;
        use viewer_unit_models::trade_routes;
        let read = TradeRouteRead::of(self.frame, kept.as_ref().map(|held| &held.read));
        if let Some(held) = kept.as_mut()
            && held.ours == ours
            && held.read.same(&read)
        {
            if held.read.assigned_same(&read) {
                held.read = read;
                viewer_runtime::view_record::add("trade routes", "convoys kept", 1);
                return Arc::clone(&held.convoys);
            }
            if let Ok(defines) = &self.trade_routes
                && let Some(made) = trade_routes::sailed(&held.routes, &defines.convoys, read.assigned_of(TableId::ResourceOrigins), read.assigned_of(TableId::ConvoyClients))
            {
                let made = Arc::new(made);
                static KEPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                if viewer_runtime::install_check::due(&KEPT) {
                    let whole = self.trade_route_lines(ours);
                    if whole.convoys != *made {
                        viewer_runtime::install_check::found("trade route convoys", format!("hour {}: {} convoy lines kept, {} built whole", self.frame.hour, made.len(), whole.convoys.len()));
                    }
                }
                (held.read, held.convoys) = (read, Arc::clone(&made));
                viewer_runtime::view_record::add("trade routes", "convoys made again", 1);
                return made;
            }
        }
        viewer_runtime::view_record::add("trade routes", "lines built whole", 1);
        let routes = self.trade_route_lines(ours);
        let convoys = Arc::new(routes.convoys.clone());
        *kept = Some(KeptConvoys { read, ours, routes, convoys: Arc::clone(&convoys) });
        convoys
    }

    fn build_trade_route_lines(&self, ours: u32) -> viewer_unit_models::trade_routes::TradeRoutes {
        use viewer_unit_models::trade_routes;
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
                viewer_foundation::unit_rules::SHIP_IN_PORT
            } else {
                viewer_foundation::unit_rules::STANDSTILL
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

    pub fn orders_and_convoys(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
    ) -> io::Result<(
        Arc<OrderField>,
        Vec<viewer_unit_models::trade_routes::ConvoyLine>,
    )> {
        self.orders_and_convoys_with(perspective, show_allied, false, false)
    }

    pub fn orders_and_convoys_with(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
        plan_layered: bool,
        routes_layered: bool,
    ) -> io::Result<(
        Arc<OrderField>,
        Vec<viewer_unit_models::trade_routes::ConvoyLine>,
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
        let mut field = viewer_gpu::ribbon_mesh::append(&plan, &raid_field);
        field = viewer_gpu::ribbon_mesh::append(&field, &routes.field);
        field = viewer_gpu::ribbon_mesh::append(&field, &air.field);
        field.report.raid_arrows = report;
        field.report.trade_routes = route_warnings;
        field.report.air_orders = air_warnings;
        Ok((Arc::new(field), routes.convoys.clone()))
    }

    pub fn raid_arrows(&self, ours: u32) -> (OrderField, viewer_gpu::ribbon_content::RaidArrowReport) {
        let heights = self.heights();
        let counters = &self.counters;
        let anchor = |province: u32, kind: u32| counters.anchor(province, kind);
        let declared = |name: &str| self.unit_models.declares(name);
        let evaluate = |items: &[content_reader_parser::parser::Item], scopes| {
            self.trigger_world().evaluate(items, scopes)
        };
        let variant_model = |variant: u64, country: u32| {
            self.unit_models.aircraft_entity(
                self.frame,
                viewer_unit_models::equipment_graphics::Limits::new(&self.pool_limits, &evaluate),
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

    pub fn air_orders(
        &self,
        viewer: u32,
        selected: &[u64],
        selected_wings: &[u64],
    ) -> viewer_aircraft::air_orders::Prepared {
        self.air_orders_from(self.frame, viewer, selected, selected_wings)
    }

    pub fn air_orders_from(
        &self,
        frame: &DecodedFrame,
        viewer: u32,
        selected: &[u64],
        selected_wings: &[u64],
    ) -> viewer_aircraft::air_orders::Prepared {
        let mut prepared = viewer_aircraft::air_orders::read(
            frame,
            &viewer_aircraft::air_orders::Inputs {
                arrow: self
                    .orders
                    .types
                    .get(viewer_aircraft::air_orders::MISSION_ARROW),
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
        prepared.transfers = viewer_aircraft::air_transfers::read(
            frame,
            &viewer_aircraft::air_transfers::Inputs {
                viewer,
                selected: selected_wings,
                arrow: self
                    .orders
                    .types
                    .get(viewer_aircraft::air_transfers::ARROW),
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
        if state.is_some()
            && self.counters.kinds.get(&province) == Some(&content_reader_map::map::ProvinceKind::Land)
            && let Some(icon) = self.counters.site_icons
                [viewer_foundation::building_facts::AIR_BASE]
                .get(&province)
        {
            match &icon.anchor {
                Ok(viewer_foundation::building_facts::IconAnchor::Record(position)) => {
                    return Ok(Some(*position));
                }
                Ok(viewer_foundation::building_facts::IconAnchor::Province) => (),
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

    pub fn map_building_field(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<(
        map_display_data::buildings::Buildings,
        viewer_buildings::map_buildings::MapBuildingSummary,
    )> {
        self.map_building_field_profiled(
            perspective,
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
    }

    pub fn map_building_field_profiled(
        &self,
        perspective: &perspective::Choice,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<(
        map_display_data::buildings::Buildings,
        viewer_buildings::map_buildings::MapBuildingSummary,
    )> {
        let _cost = replay_frame::cpu::scope("viewer", Some("buildings"));
        self.product_hour.buildings(
            &self.map_buildings,
            perspective,
            &self.assets.spawn_city,
            spans,
        )
    }

    pub fn prepare_unit_models(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_unit_models::unit_models::PreparedModels> {
        self.prepare_unit_models_profiled(perspective, &mut |_| {})
    }

    pub fn prepare_unit_models_profiled(
        &self,
        perspective: &perspective::Choice,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<viewer_unit_models::unit_models::PreparedModels> {
        self.prepare_unit_models_with_spans(
            perspective,
            lap,
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
    }

    pub fn prepare_unit_models_with_spans(
        &self,
        perspective: &perspective::Choice,
        lap: &mut dyn FnMut(&'static str),
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<viewer_unit_models::unit_models::PreparedModels> {
        self.prepare_unit_models_naval_or_not(perspective, true, lap, spans)
    }

    pub fn prepare_unit_models_naval_or_not(
        &self,
        perspective: &perspective::Choice,
        naval: bool,
        lap: &mut dyn FnMut(&'static str),
        spans: &mut viewer_runtime::profiling::BuilderSpans,
    ) -> io::Result<viewer_unit_models::unit_models::PreparedModels> {
        self.unit_models_with(perspective, true, lap, |assets, inputs, lap| {
            assets.prepare_naval_or_not(inputs, naval, lap, spans)
        })
    }

    pub fn prepare_unit_models_without_land(
        &self,
        perspective: &perspective::Choice,
        naval: bool,
        ours: u32,
        lap: &mut dyn FnMut(&'static str),
    ) -> io::Result<viewer_unit_models::unit_models::PreparedModels> {
        if !naval {
            let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
            let evaluate = |items: &[content_reader_parser::parser::Item], scopes| self.trigger_world().evaluate(items, scopes);
            let limits = viewer_unit_models::equipment_graphics::Limits::new(&self.pool_limits, &evaluate);
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

    fn unit_models_with(
        &self,
        perspective: &perspective::Choice,
        land: bool,
        lap: &mut dyn FnMut(&'static str),
        make: impl FnOnce(
            &viewer_unit_models::unit_models::UnitModelAssets,
            viewer_unit_models::unit_models::PrepareInputs<'_>,
            &mut dyn FnMut(&'static str),
        ) -> io::Result<viewer_unit_models::unit_models::PreparedModels>,
    ) -> io::Result<viewer_unit_models::unit_models::PreparedModels> {
        let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
        let prepared = (|| {
            let content = &self.counters;
            let definitions = &self.map.definitions;
            let etas = if land { self.etas()? } else { Arc::default() };
            lap("um_etas");
            let movement = self.movement.as_deref();
            let world = movement.map(
                |movement| viewer_unit_reads::counter_move::RouteWorld {
                    map: &movement.map,
                    heights: self.heights(),
                    width: self.map.width as f32,
                },
            );
            let anchor = |province, slot| content.anchor(province, slot);
            let land = |province| match movement.and_then(|movement| movement.map.terrain(province))
            {
                Some(terrain) => terrain.flags & 1 != 0,
                None => matches!(
                    definitions.get(&province).map(|it| it.kind),
                    Some(map::ProvinceKind::Land)
                ),
            };
            let motions =
                viewer_unit_reads::task_force_visual::TaskForceMotions::read(self.frame)?;
            let missions =
                viewer_unit_reads::task_force_visual::MissionSkips::read(self.frame)?;
            lap("um_ship_inputs");
            let region = |province| content.region(province);
            let evaluate = |items: &[content_reader_parser::parser::Item], scopes| {
                self.trigger_world().evaluate(items, scopes)
            };
            let limits =
                viewer_unit_models::equipment_graphics::Limits::new(&self.pool_limits, &evaluate);
            let vision = self.vision(perspective)?;
            lap("um_vision");
            make(
                &self.unit_models,
                viewer_unit_models::unit_models::PrepareInputs {
                    frame: self.frame,
                    limits,
                    vision: &vision,
                    etas: &etas,
                    anchor: &anchor,
                    world: world.as_ref(),
                    terrain: &content.terrain.combat_terrain,
                    routes: Some(&content.routes),
                    ships: &viewer_unit_models::unit_models::ShipPlaces {
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
        lap("um_finish");
        prepared
    }

    pub fn unit_model_field_sailing(
        &self,
        perspective: &perspective::Choice,
        time: f32,
        shown: &dyn Fn(&viewer_unit_models::trade_routes::ConvoyDefines) -> bool,
    ) -> io::Result<(viewer_unit_history::unit_history::Field, ModelReport)> {
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

    pub fn nuclear_impacts(&self) -> io::Result<viewer_models::strike_effects::Impacts> {
        let _cost = replay_frame::cpu::scope("viewer", Some("engagement/strikes/text"));
        self.product_hour.nuclear_impacts()
    }

    fn impact_place(&self, province: u32) -> Option<([f32; 3], [f32; 4])> {
        self.content.products.impact_place(province)
    }

    pub fn map_text(
        &self,
        vision: &perspective::Vision,
    ) -> viewer_map_text::animated_map_text::events::Prepared {
        self.product_hour.map_text(vision)
    }

    pub fn aircraft_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<viewer_aircraft::aircraft::AircraftPlan> {
        let _cost = replay_frame::cpu::scope("viewer", Some("air/naval source choices"));
        let heights = HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        };
        let movement = self.movement.as_deref();
        let world = movement.map(
            |movement| viewer_unit_reads::counter_move::RouteWorld {
                map: &movement.map,
                heights: self.heights(),
                width: self.map.width as f32,
            },
        );
        let motions =
            viewer_unit_reads::task_force_visual::TaskForceMotions::read(self.frame)?;
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
        let evaluate = |items: &[content_reader_parser::parser::Item], scopes| {
            self.trigger_world().evaluate(items, scopes)
        };
        let models = viewer_aircraft::aircraft::ModelMemo::default();
        let model = |variant, country| {
            models.resolve(variant, country, || {
                self.unit_models.aircraft_entity(
                    self.frame,
                    viewer_unit_models::equipment_graphics::Limits::new(
                        &self.pool_limits,
                        &evaluate,
                    ),
                    variant,
                    country,
                )
            })
        };
        viewer_aircraft::aircraft::AircraftPlan::build(
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
    ) -> io::Result<viewer_rail::trains::TrainPlan> {
        let _cost = replay_frame::cpu::scope("viewer", Some("rails"));
        let heights = HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        };
        viewer_rail::trains::TrainPlan::build_with_graph(
            self.frame,
            self.vision(perspective)?.ours(),
            &self.rail_geometry,
            &heights,
            &self.trains,
            &self.trains.cultures,
            || self.product_hour.rail_graph(),
        )
    }

    pub fn limitations(&self, perspective: &perspective::Choice) -> io::Result<Limitations> {
        self.product_hour.limitations(perspective)
    }

    pub fn map_mode_fill(
        &self,
        mode: &viewer_map_modes::map_mode::MapMode,
        choice: &perspective::Choice,
        selected: Option<u32>,
        colour: Option<&map_display_data::landscape::ColourState>,
    ) -> io::Result<map_display_data::landscape::ModeFill> {
        let _cost = replay_frame::cpu::scope("viewer", Some("modes/styles"));
        self.product_hour
            .mode(&self.map_modes, mode, choice, selected, colour)
    }

    pub fn political_product(
        &self,
        mode: &viewer_map_modes::map_mode::MapMode,
        choice: &perspective::Choice,
        selected: Option<u32>,
        colour: Option<&map_display_data::landscape::ColourState>,
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

    pub fn demilitarized_zone(&self, regions: &Regions, extra: &BTreeSet<u32>) -> io::Result<Zone> {
        self.product_hour.zone(regions, extra)
    }

    pub fn place_names(
        &self,
        mode: &viewer_map_modes::map_mode::MapMode,
    ) -> io::Result<Placements> {
        Ok((*self.shared_names(mode)?).clone())
    }
    pub fn shared_names(
        &self,
        mode: &viewer_map_modes::map_mode::MapMode,
    ) -> io::Result<Arc<Placements>> {
        self.product_hour.names(&self.name_inputs(), mode)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn name_areas(
        &self,
        mode: &viewer_map_modes::map_mode::MapMode,
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

    pub fn hour_tables(
        &self,
        perspective: &perspective::Choice,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
    ) -> io::Result<OverlayTables> {
        let ours = self.vision(perspective)?.ours();
        self.hour_tables_with_raids(perspective, Arc::new(self.raid_icons(ours)), engaged, viewer_counter_kit::Kinds::default())
    }

    fn raid_icons(&self, ours: u32) -> raids::RaidIcons {
        let paths = raids::paths::HourPaths::new(&self.icons.raids, self.frame);
        raids::icons(&self.icons.raids, self.frame, ours, &paths)
    }

    pub fn hour_tables_with_raids(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        left_out: viewer_counter_kit::Kinds,
    ) -> io::Result<OverlayTables> {
        self.hour_tables_with_raids_profiled(
            perspective,
            raids,
            engaged,
            left_out,
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
            true,
        )
    }

    pub fn hour_tables_with_raids_profiled(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        left_out: viewer_counter_kit::Kinds,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
        radar: bool,
    ) -> io::Result<OverlayTables> {
        self.hour_tables_layered(perspective, raids, engaged, left_out, false, spans, radar)
    }

    pub fn hour_tables_layered(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
        left_out: viewer_counter_kit::Kinds,
        labels_by_layers: bool,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
        radar: bool,
    ) -> io::Result<OverlayTables> {
        self.hour_tables_of(perspective, raids, engaged.map(|index| index.shared.as_ref()), left_out, labels_by_layers, spans, radar, counters::Need::WHOLE)
    }

    #[allow(clippy::too_many_arguments, reason = "the tables' inputs, what the view draws and the timings")]
    pub fn hour_tables_at_head(
        &self,
        perspective: &perspective::Choice,
        vision: Arc<perspective::Vision>,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&viewer_engagements::engagement_history::History>,
        left_out: viewer_counter_kit::Kinds,
        left: LeftToLayers,
        labels_by_layers: bool,
        need: counters::Need,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
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
        engaged: Option<&viewer_engagements::engagement_history::History>,
        left_out: viewer_counter_kit::Kinds,
        labels_by_layers: bool,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
        radar: bool,
        need: counters::Need,
    ) -> io::Result<OverlayTables> {
        let (etas, vision) = spans.measure("overlay.tables.prerequisites", |spans| {
            let etas = spans.measure("overlay.tables.prerequisites.etas", |_| {
                if need.routes_by_key(left_out) || !(need.shown.units || need.shown.land_combats) { Ok(Arc::default()) } else { self.etas() }
            })?;
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
        let engaged_covered =
            engaged.is_some_and(|index| index.covers(self.frame.branch, self.frame.hour));
        let (radar, empty_air) = spans.measure("overlay.tables.radar_empty_air", |_| {
            let radar = Arc::new(if radar {
                viewer_aircraft::radar_symbols::Field::read(self.frame, ours, self.heights(), &self.radar_centers)
            } else {
                viewer_aircraft::radar_symbols::Field::default()
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
                        .map(|rosters| rosters as &dyn viewer_counters_naval::air_missions::AirPasses),
                    &self.type_tint_memo,
                    &self.mission_memo,
                    self.icons.raids.movement_reads(),
                    self.icons.raids.mission_topology(),
                    left_out,
                    need,
                    &self.overlay_carry,
                    self.air_rosters.clone().map(|rosters| rosters as Arc<dyn viewer_counters_naval::air_missions::AirPasses + Send + Sync>),
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
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
            None,
        )
    }

    fn counter_inputs<'b>(
        &'b self,
        windows: &'b viewer_native_gui::native_windows::NativeWindows,
        display: &'b viewer_counter_kit::CounterDisplay,
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
            moved: self.moved_hours.as_deref().unwrap_or(&viewer_unit_reads::counter_move::NO_MOVED_HOURS),
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

    fn read_still(
        &self,
        tables: &OverlayTables,
        view: &CounterView,
        rows: &OverlayRows,
    ) -> io::Result<Still> {
        let frame = self.frame;
        let hour = Hour::new(self.content, frame, self.hour_fraction);
        let air_assignment = if view.visibility.priorities
            == viewer_foundation::mode_visibility::PriorityTable::StrategicAir
        {
            let defines = &hour.icons.raids.defines;
            rows.read("air assignment", frame, |frame| viewer_map_modes::air_assignment::read(
                frame,
                &view.air_selected,
                tables.ours(),
                viewer_unit_reads::air_range::Defines {
                    map_scale: defines.map_scale,
                    ace_wing_size: defines.ace_wing_size,
                    ace_wing_size_max_bonus: defines.ace_wing_size_max_bonus,
                },
                viewer_map_modes::air_assignment::Content {
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
            == viewer_foundation::mode_visibility::PriorityTable::Resistance
        {
            rows.read("occupation stripes", frame, |frame| {
                viewer_map_modes::occupation_stripes::read(
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
            == viewer_foundation::mode_visibility::PriorityTable::StrategicNavy
        {
            rows.read("naval selection", frame, |frame| {
                viewer_map_modes::naval_selection::read(
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

    #[allow(clippy::too_many_arguments, reason = "each argument is separate state the host lends for one draw or load")]
    fn prepare_overlay_with(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        tables: &OverlayTables,
        view: &CounterView,
        rows: &OverlayRows,
        spans: &mut viewer_runtime::profiling::BuilderSpans,
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
        let display = viewer_counter_kit::CounterDisplay::default();
        let counter_inputs = self.counter_inputs(&windows, &display, camera, viewport, ui_scale, view);
        let label_at = labels::LabelProjection { camera, viewport, ui_scale };
        let mut read = None;
        let still = match still {
            Some(still) => still,
            None => read.insert(spans.measure("overlay.projection.still", |_| self.read_still(tables, view, rows))?),
        };
        let content = self.content;
        let fraction = self.hour_fraction;
        let prepare_counters = || {
            let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
            let _span = viewer_runtime::profiling::span("p_counters");
            rows.read("counters", frame, |_| prepare_counters_with(&counter_inputs, tables, view, lazy_version))
        };
        let prepare_other = |labels: &mut labels::PreparedLabels| {
            let _cost = replay_frame::cpu::scope("viewer", Some("modes/styles"));
            let hour = Hour::new(content, frame, fraction);
            let native_raids = hour.raid_owners(
                camera,
                viewport,
                ui_scale,
                view.visibility.raid_icons,
                &tables.raids,
            );
            let raid_report = tables.raids.report.clone();
            {
                let _span = viewer_runtime::profiling::span("p_labels");
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
            let resource_icons = if rows.left.resource_icons {
                (Default::default(), MadeBy::Left)
            } else {
                let made = hour.resource_icons_with(camera, viewport, ui_scale, view.visibility, |inputs| {
                    let _cost = replay_frame::cpu::scope("viewer", Some("resource icons"));
                    rows.read("resource icons", frame, |frame| viewer_map_icons::resource_icons::prepare(inputs, frame))
                });
                (made, MadeBy::Today)
            };
            let occupation_icons = if rows.left.occupation_marks {
                (Default::default(), MadeBy::Left)
            } else {
                let made = hour.occupation_icons_with(camera, viewport, ui_scale, view.occupation, tables.ours(), |inputs| {
                    let _cost = replay_frame::cpu::scope("viewer", Some("occupation icons"));
                    rows.read("occupation icons", frame, |frame| viewer_map_icons::occupation_icons::prepare(inputs, frame))
                });
                (made, MadeBy::Today)
            };
            let counters = &hour.counters;
            let anchor = |province| {
                counters.anchor(province, viewer_foundation::unit_rules::STANDSTILL)
            };
            let adjacency_icons =
                hour.adjacency_icons
                    .prepare(&viewer_map_icons::adjacency_icons::Inputs {
                        anchor: &anchor,
                        camera,
                        viewport,
                        ui_scale,
                        visibility: view.visibility,
                    });
            let adjacency_priority = viewer_map_icons::adjacency_icons::priority(
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
                laid_icons: LaidIcons {
                    resource_icons: resource_icons.0,
                    resource_icons_by: resource_icons.1,
                    occupation_icons: occupation_icons.0,
                    occupation_icons_by: occupation_icons.1,
                },
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
                icons: OverlayCounters::Prepared(icons),
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
        reach: Option<viewer_foundation::mode_visibility::RaidReach>,
        raids: &raids::RaidIcons,
    ) -> raids::PreparedRaids {
        let anchor = |province| {
            self.counters
                .anchor(province, viewer_foundation::unit_rules::STANDSTILL)
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
        ) -> io::Result<Option<viewer_counter_kit::gui_windows::WindowFlags<'s>>>,
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
        counter_settings: viewer_counter_kit::CounterSettings,
        mode: &viewer_map_modes::map_mode::MapMode,
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

    pub fn snow_mud(&self) -> io::Result<SnowMud> {
        self.product_hour.snow_grid(self.snow_cells()?)
    }

    pub fn snow_cells(&self) -> io::Result<&map_snow::SnowCells> {
        Ok(&self.products.snow_content()?.cells)
    }

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

#[cfg(test)]
const DIVISION_CLASS: u32 = 0;

#[cfg(test)]
const MUSIC_CHANCE_ONE: f64 = 100_000.0;

#[cfg(test)]
fn song_chances(
    content: &viewer_audio::Content,
    mut evaluate: impl FnMut(&[content_reader_parser::parser::Item]) -> Result<i64, String>,
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


fn load_rivers(
    layers: &content_reader_layers::ResolvedLayers,
    art_defines: &content_reader_defines::defines::Defines,
    defines: &content_reader_defines::defines::Defines,
    image: &RiverImage<'_>,
    heights: &HeightField<'_>,
) -> io::Result<(RiverMesh, rivers::Trace, Duration, Option<String>)> {
    let started = Instant::now();
    match viewer_world::map_assets::graphics_define(
        layers,
        "rivers",
        art_defines,
        defines,
        rivers::RiverDefines::load,
    ) {
        Ok(river_defines) => {
            let (river_mesh, river_trace) = rivers::build(image, heights, river_defines)?;
            Ok((river_mesh, river_trace, started.elapsed(), None))
        }
        Err(error) => {
            let river_trace = rivers::trace(image)?;
            Ok((
                RiverMesh::default(),
                river_trace,
                started.elapsed(),
                Some(format!("Rivers are withheld and draw nothing: {error}")),
            ))
        }
    }
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
            content_reader_layers::ResolvedLayers::from_folders(&[(base.as_path(), &[][..])]).unwrap();
        let selected = [content_reader_layers::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        content_reader_layers::content::visual::apply(&mut layers, &options, &selected, &[], &cancel)
            .unwrap();
        let plan = viewer_buildings::sprawl::SprawlPlan {
            buildings: vec![viewer_buildings::sprawl::SprawlBuilding {
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
        let defines = content_reader_defines::defines::read(&layers).unwrap();
        let (sprawl, _) =
            load_sprawl(&layers, Default::default(), &plan, (&defines, &defines)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let slot = sprawl.library.slot("house").unwrap();
        assert!(sprawl.library.meshes[slot].visual);
    }

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
        assert_eq!(loaded, 2);
        let unchecked = until_current(|| Ok(((), String::new())), || Some("r9".into()));
        assert!(unchecked.is_ok());
    }

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
        let selected = [content_reader_layers::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let revision =
            content_reader_layers::content::visual::revision(&options, &selected, &[], &cancel).unwrap();
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

    #[test]
    fn recorded_battles_keep_their_order_and_sum_their_members_combats() {
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
        let recorded = [
            row(0, 90.0, 9.0, vec![9]),
            row(1, 60.0, 31.5, vec![5, 7]),
            row(2, 11.0, 9.0, vec![]),
        ];
        let (battles, unknown) = recorded_battles(&recorded, &combats, true, composition);
        let keys: Vec<_> = battles.iter().map(|battle| battle.key).collect();
        assert_eq!(keys, [0x1000, 0x1001, 0x1002]);
        assert_eq!(battles[1].center, [60.0, -60.0]);
        assert_eq!(battles[1].radius, 31.5);
        assert_eq!(battles[1].provinces, [5, 7]);
        assert_eq!(battles[0].divisions, 1);
        assert_eq!(battles[1].divisions, 7);
        assert_eq!(battles[1].sub_units["infantry"], 7 + 7 + 7 + 2 + 7);
        assert_eq!(battles[1].sub_units["artillery"], 4);
        assert_eq!(battles[2].divisions, 0);
        assert_eq!(unknown, 1);
        let (silent, unknown) = recorded_battles(&recorded, &combats, false, composition);
        assert!(silent.iter().all(|battle| battle.sub_units.is_empty()));
        assert_eq!(silent[1].divisions, 7);
        assert_eq!(unknown, 0);
    }

    #[test]
    fn song_chances_leave_refused_songs_unknown_and_skip_disabled_stations() {
        use content_reader_sound::music::{Music, Song, Track};
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
                chance: content_reader_parser::parser::parse(chance).unwrap().items,
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
            rotation: None,
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

    #[test]
    fn display_switches_leave_out_only_their_overlay_part() {
        let quad = |group: usize| Quad {
            texture: group,
            rect: [group as f32, 0.0, 1.0, 1.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
            rotation: None,
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
        let (textures, raids, layers) = drawn(dashboard_core::protocol::MapGraphics {
            counters: false,
            ..all
        });
        assert_eq!(textures, vec![3, 4]);
        assert_eq!(raids, None);
        assert!(layers.raids_hovered);
        assert!(!layers.counters.land && !layers.counters.naval);
        assert!(!layers.counters.railway_guns && !layers.counters.ports);
        let naval = OverlayLayers::shown(
            &dashboard_core::protocol::MapGraphics {
                naval_counters: false,
                ..all
            },
            None,
        );
        assert!(naval.counters.land && !naval.counters.naval);
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
    #[ignore = "requires an authored game, user folder, and recording.battleplan in BATTLEPLAN_VIEWER_TEST_FIXTURES/coverage-2 and coverage-3"]
    fn historical_coverage_document_load_keeps_uncovered_read_notes() {
        let fixtures = PathBuf::from(
            std::env::var_os("BATTLEPLAN_VIEWER_TEST_FIXTURES")
                .expect("choose the authored viewer fixtures folder"),
        );
        let scratch = synthetic_game::Installation::new("historical-coverage").build();
        for policy in [2, 3] {
            let fixture = fixtures.join(format!("coverage-{policy}"));
            let game_dir = fixture.join("game");
            assert_eq!(
                fs::read(game_dir.join("hoi4.exe")).unwrap(),
                synthetic_game::DEFAULT_EXECUTABLE,
                "the historical fixture uses the synthetic game's executable"
            );
            let options = ResolveOptions {
                game_dir,
                user_dir: fixture.join("user"),
                cache_dir: scratch.cache_dir.join(policy.to_string()),
            };
            let path = fixture.join("recording.battleplan");
            let replay = Arc::new(Replay::open(fs::read(&path).unwrap()).unwrap());
            assert_eq!(replay.identity.coverage_policy, policy);
            let initial = replay.index.first().expect("the fixture records an hour");
            let source = dashboard_base::Source::from_replay(
                dashboard_base::SourceKey {
                    source_id: path.to_string_lossy().into_owned(),
                    revision: dashboard_core::protocol::replay_revision(&path).unwrap(),
                },
                path.clone(),
                Arc::clone(&replay),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let document = Document::load_shared(
                &path,
                &options,
                false,
                &InitialHour {
                    branch: Some(initial.branch),
                    hour: Some(initial.first_hour),
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
        }
    }

    #[test]
    fn missing_river_fade_defines_withhold_ribbons_without_failing_load() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_doc_missing_river_fade_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let layers =
            content_reader_layers::ResolvedLayers::from_folders(&[(root.as_path(), &[][..])])
                .unwrap();
        let defines = content_reader_defines::defines::Defines::new().unwrap();
        defines
            .load(
                "common/defines/00_graphics.lua",
                b"NDefines = { NGraphics = { RIVER_FADE_TO = 3.0 } }",
            )
            .unwrap();
        let rows = [
            "..........",
            "...4......",
            "...j......",
            ".S3366777.",
            "..........",
        ];
        let mut indices = Vec::with_capacity(50);
        for row in rows {
            for ch in row.chars() {
                indices.push(match ch {
                    '.' => 255,
                    'S' => 0,
                    'j' => 1,
                    digit => digit.to_digit(10).unwrap() as u8,
                });
            }
        }
        let values = [120u8; 50];
        let (mesh, trace, _, warning) = load_rivers(
            &layers,
            &defines,
            &defines,
            &RiverImage {
                width: 10,
                height: 5,
                indices: &indices,
            },
            &HeightField {
                width: 10,
                height: 5,
                values: &values,
            },
        )
        .unwrap();
        assert!(mesh.is_empty());
        assert_eq!(mesh.ribbons, 0);
        assert_eq!(trace.sources, 1);
        assert_eq!(trace.junctions, 1);
        assert_eq!(trace.junctions_used, 1);
        assert_eq!(trace.rivers.len(), 2);
        let warning = warning.expect("missing river fade define must report a warning");
        assert!(
            warning.contains("Rivers are withheld") && warning.contains("RIVER_FADE_FROM"),
            "{warning}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finish_project_reapplies_resource_and_occupation_icons_when_panning_past_the_pad() {
        use viewer_gpu::camera::{MapDefines, MapSize, Placement};
        use viewer_foundation::mode_visibility::{
            Priorities,
            PriorityIcon,
            PriorityTable,
            Visibility,
        };
        use viewer_map_icons::{
            occupation_icons::{self, HourMarks, IconDefines, Mark, Mode, Occupation},
            resource_icons::{self, Laid, ResourceIconContent, ResourceIconDefines, Row},
        };
        use viewer_map_modes::map_mode::ColorList;

        struct SyntheticHost {
            resource_content: ResourceIconContent,
            occupation_content: occupation_icons::Content,
            localisation: content_reader_localisation::localisation::Localisation,
            state_provinces: BTreeMap<u32, Vec<u32>>,
        }

        impl IconHost for SyntheticHost {
            fn lay_resource_icons(
                &self,
                camera: Camera,
                viewport: [u32; 2],
                ui_scale: f32,
                visibility: Visibility,
                icons: &resource_icons::HourIcons,
            ) -> resource_icons::PreparedResourceIcons {
                let centroid = |province: u32| (province == 1).then_some([1000.0, 10.0, 500.0]);
                resource_icons::lay_out(
                    &resource_icons::ResourceIconInputs {
                        content: &self.resource_content,
                        localisation: &self.localisation,
                        state_provinces: &self.state_provinces,
                        centroid: &centroid,
                        camera,
                        viewport,
                        ui_scale,
                        visibility,
                    },
                    icons,
                )
            }

            fn lay_occupation_icons(
                &self,
                camera: Camera,
                viewport: [u32; 2],
                ui_scale: f32,
                mode: Option<Mode>,
                country: u32,
                marks: &occupation_icons::Marks,
            ) -> occupation_icons::PreparedOccupationIcons {
                let centroid = |province: u32| (province == 1).then_some([1000.0, 10.0, 500.0]);
                occupation_icons::lay_out(
                    &occupation_icons::Inputs {
                        content: &self.occupation_content,
                        mode,
                        country,
                        state_provinces: &self.state_provinces,
                        centroid: &centroid,
                        camera,
                        viewport,
                        ui_scale,
                    },
                    marks,
                )
            }
        }

        let mut priorities = Priorities::uniform([0; PriorityIcon::ALL.len()]);
        priorities.set(PriorityTable::Resources, PriorityIcon::Resource, 10);
        let host = SyntheticHost {
            resource_content: ResourceIconContent {
                origins: viewer_runtime::resource_origins::Content {
                    resources: Vec::new(),
                    buildings: Default::default(),
                    bonuses: [0, 0],
                    oil: String::new(),
                    warnings: Vec::new(),
                },
                defines: ResourceIconDefines {
                    icons_cutoff: 900.0,
                    text_cutoff: 800.0,
                    priorities,
                },
            },
            occupation_content: occupation_icons::Content {
                definitions: Default::default(),
                defines: IconDefines {
                    cutoff: 900.0,
                    badge_cutoff: 700.0,
                    reenable: 2_000_000,
                    priority: 30,
                    resistance_colors: ColorList(vec![0., 1., 0., 0., 0., 100., 0., 1., 0., 0.75]),
                    compliance_colors: ColorList(vec![0., 0., 0., 1., 0., 100., 1., 1., 1., 1.]),
                },
            },
            localisation: Default::default(),
            state_provinces: BTreeMap::from([(1, vec![1])]),
        };
        let hour_icons: resource_icons::HourIcons = Ok(BTreeMap::from([(
            1,
            Ok(Some(Laid {
                rows: Ok(vec![Row {
                    text: "Steel: 12".into(),
                    flag: None,
                }]),
                flag: Ok(None),
            })),
        )]));
        let marks: occupation_icons::Marks = Ok(HourMarks {
            marks: BTreeMap::from([(
                1,
                Mark {
                    controller: 1,
                    slot: Some(1),
                    resistance: Some(Ok(Some(Occupation {
                        level: 25_000_000,
                        speed: 100_000,
                        compliance: 0,
                        badges: [Ok(Vec::new()), Ok(Vec::new())],
                    }))),
                },
            )]),
            viewers: BTreeMap::from([(1, 1)]),
            resistance: None,
        });
        let lay_ins = crate::side::LayIns::with_icons(
            Some((hour_icons, 7)),
            Some((marks, 9)),
        );
        let counter_view = CounterView {
            visibility: Visibility {
                resource_icons: true,
                ..Visibility::default()
            },
            occupation: Some(Mode::Resistance),
            ..CounterView::default()
        };
        let placed = Placement::map([1000.0, 500.0], 110.0).on(MapSize::TEST, MapDefines::test());
        let outside_pad =
            Placement::map([1020.0, 500.0], 110.0).on(MapSize::TEST, MapDefines::test());
        let bare = |view: CounterView| PreparedOverlay {
            radar: Arc::default(),
            air_ranges: Arc::default(),
            air_assignment: Default::default(),
            occupation_stripes: Default::default(),
            naval_selection: Default::default(),
            air_orders: Arc::default(),
            labels: Default::default(),
            raids: Batch::default(),
            native_raids: Default::default(),
            icons: OverlayCounters::Empty,
            label_report: Default::default(),
            raid_report: Default::default(),
            native_intel: Default::default(),
            intel: Batch::default(),
            intel_report: Default::default(),
            lazy_version: 0,
            engaged_covered: false,
            layers: [0; 6],
            counter_view: view,
            laid_icons: LaidIcons::default(),
            adjacency_icons: Default::default(),
            adjacency_priority: 0,
            mode_icons: Batch::default(),
            mode_icon_refused: BTreeMap::new(),
            tables: 0,
        };
        let mut overlay = bare(counter_view.clone());
        assert!(lay_ins.lay(&mut overlay, &host, placed, [1600, 900], 1.0, 1));
        assert_eq!(overlay.laid_icons.resource_icons_by(), MadeBy::Layer(7));
        let before_resources = overlay.laid_icons.resource_icons.placed_positions();
        assert_eq!(before_resources.len(), 1);
        assert_eq!(overlay.laid_icons.occupation_icons_by(), MadeBy::Layer(9));
        let before_occupation = overlay.laid_icons.occupation_icons.placed_positions();
        assert_eq!(before_occupation.len(), 1);

        let reprojected = bare(counter_view);
        assert_eq!(reprojected.laid_icons.resource_icons_by(), MadeBy::Left);
        assert_eq!(reprojected.laid_icons.occupation_icons_by(), MadeBy::Left);
        overlay.finish_project(
            reprojected,
            &host,
            outside_pad,
            [1600, 900],
            1.0,
            1,
            Some(&lay_ins),
        );
        assert_eq!(overlay.laid_icons.resource_icons_by(), MadeBy::Layer(7));
        let after_resources = overlay.laid_icons.resource_icons.placed_positions();
        assert_eq!(
            after_resources.len(),
            1,
            "retained resource icons must survive relayout when panning past the placement pad"
        );
        assert!(
            after_resources[0][0] < before_resources[0][0],
            "reprojected resource icon must shift to the new camera: before {:?}, after {:?}",
            before_resources[0],
            after_resources[0]
        );
        assert_eq!(overlay.laid_icons.occupation_icons_by(), MadeBy::Layer(9));
        let after_occupation = overlay.laid_icons.occupation_icons.placed_positions();
        assert_eq!(
            after_occupation.len(),
            1,
            "retained occupation icons must survive relayout when panning past the placement pad"
        );
        assert!(
            after_occupation[0][0] < before_occupation[0][0],
            "reprojected occupation icon must shift to the new camera: before {:?}, after {:?}",
            before_occupation[0],
            after_occupation[0]
        );
    }
}
