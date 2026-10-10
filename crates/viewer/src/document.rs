//! A replay and the verified local game content used to draw its current hour.

use dashboard::content::check_verification;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use installed_content::{
    ResolveOptions,
    map::{self, MapData},
};
use replay::{
    BranchId,
    container::{Replay, SeekResult},
    frame::DecodedFrame,
    rows::tables as row_tables,
};

use crate::{
    map::textures::MapAssets,
    // borders
    map::{
        border_pass::{BorderAssets, StraitVertex, Topology, strait_quads},
        borders::{self, BorderMeshes, HeightField, ProvinceMap},
        demilitarized::{self, Zone},
        political::Regions,
    },
    playback::Playback,
};
// rivers
use crate::map::rivers::{self, RiverAssets, RiverImage, RiverMesh};
// trees
use crate::map::mapframe::FrameAssets;
use crate::map::trees::{self, TreeAssets, TreeField, TreeGeometry, TreeImage};
// close zoom
use crate::map::map_buildings::MapBuildingAssets;
use crate::map::railways::{self as rail_layer, Bridges, RailAssets, RailMesh};
use crate::map::sprawl::{self, Sprawl};
use crate::map::unit_models::{ModelReport, UnitModelAssets};
// names
use crate::map::raids;
use crate::map::{
    camera::Camera,
    labels::{self, LabelContent, LabelReport},
    name_pass::{self, NameContent, PlacementInputs, Placements},
    sprites::{Batch, MapIcons},
};
// counters
use crate::map::counters::{self, CounterContent, CounterInputs, CounterReport};
// orders: the recorded player's battle plan
use crate::map::orders::{self, OrderAssets, OrderField, ProvinceGeometry};
// weather
use crate::map::perspective;
use crate::map::tables::{self, Limitations};
use crate::map::textures::{self, IntelMap};
use crate::map::weather::{self, SnowMud};

/// The recorded hour to sit on after load: `--branch` and `--hour` select it,
/// `hour_fraction` is how far past it the frame sits.
pub struct InitialHour {
    pub branch: Option<BranchId>,
    pub hour: Option<replay::Hour>,
    pub hour_fraction: f32,
}

/// A replay, the hour on screen and the content it is drawn with.
pub struct Document {
    /// Everything that does not move with the hour, shared with the window's
    /// hour builders on worker threads (`crate::hours`).
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

/// The verified local game content a replay is drawn with: everything that
/// does not change with the recorded hour.
pub struct Content {
    /// Perspective-independent compute resources, weakly cached per graphics device.
    pub political_gpu: crate::map::political_gpu::Cache,
    pub texture_quality: u8,
    pub visual_mods: Option<Vec<String>>,
    pub map: Arc<MapData>,
    /// The content's camera and frame defines with the user's camera tilt.
    pub map_defines: crate::map::camera::MapDefines,
    /// The user's `settings.txt` display facts that size the game's interface
    /// on screen, or why they could not be read; see [`crate::map::ui_scale`].
    pub game_display: Result<installed_content::settings::Display, String>,
    /// The user's `settings.txt` counter options (`counter_color_mode`, `template_counter`),
    /// which the window's toggles and a dump start from.
    pub game_counters: counters::CounterSettings,
    /// The configured user folder's local world target size; no recorder UI state.
    pub game_full_resolution: bool,
    /// The game images and textures the map passes draw from.
    pub assets: MapAssets,
    /// Persistent recording compatibility, content verification and presentation limitations.
    pub warnings: Vec<String>,
    /// Shared lazy content reads, reported afresh for map diagnostics and dumps.
    pub uncovered_reads: installed_content::content::UncoveredReads,
    pub source_path: PathBuf,
    // borders
    /// `map/terrain/border_*_0.dds` and `strait.dds`.
    pub border_assets: BorderAssets,
    /// The traced border strips. Every land edge sits in the `Province` bucket until the hour's
    /// political regions re-bucket it; see `map::border_pass`.
    pub border_meshes: BorderMeshes,
    /// The strait ribbons from `map/adjacencies.csv`.
    pub straits: Vec<StraitVertex>,
    /// `map/strategicregions/*.txt`, which layer 1 of the political fill is built from; see
    /// `map::political::strategic_regions_of`.
    pub strategic_regions: Arc<BTreeMap<u32, installed_content::regions::StrategicRegion>>,
    pub naval_dominance: crate::map::naval_dominance::Content,
    /// Each strategic region's naval battle name candidates in the game's
    /// order ([`crate::combat::battle_name_candidates`]).
    pub battle_name_provinces: BTreeMap<u32, Vec<u32>>,
    /// Each state definition's `name` key, the last
    /// state-display-name fallback, or why the game's
    /// reading of it cannot be followed.
    pub state_name_keys: BTreeMap<u32, Result<String, String>>,
    /// The recording client's game language (the replay identity's, without
    /// its `l_`): the language of text the game stored already localised
    /// ([`dashboard::scripted_text::stored_text`]). Part of the content
    /// identity.
    pub recorded_language: String,
    /// The content the scripted text of hover text reads, or why it cannot
    /// be read.
    pub scripted: Result<dashboard::scripted_text::ScriptedContent, String>,
    /// Wall time of the border trace, for reporting.
    pub border_trace: Duration,
    // names
    /// Fonts and map-icon sprites, uploaded once by the scene.
    pub icons: MapIcons,
    /// Province adjacency, province bounds and the localisation.
    pub names: NameContent,
    /// The label window layout and the unit-stack anchors.
    pub labels: LabelContent,
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
    pub counters: CounterContent,
    /// Native province hover content; recording gaps are handled per line.
    pub province_hover: crate::map::province_tooltips::Content,
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
    rail_geometry: rail_layer::RailGeometry,
    pub aircraft: std::sync::Arc<crate::map::aircraft::AircraftAssets>,
    pub trains: std::sync::Arc<crate::map::trains::TrainAssets>,
    /// How long constructing the static railway geography took.
    pub rail_build: Duration,
    /// The railway bridges, which are meshes rather than strip.
    pub bridges: Bridges,
    /// The land division entities, the sub-unit keys and the graphical
    /// cultures; the placed figures are per hour and come from
    /// [`Document::unit_model_field`].
    pub unit_models: UnitModelAssets,
    // orders: the recorded player's battle plan
    /// `gfx/maparrows/maparrows.txt` and the mask and pattern pages of every
    /// arrow type the battle-plan layer can select.
    pub orders: OrderAssets,
    pub weather_effects: std::sync::Arc<crate::map::weather_effects::Assets>,
    /// Derived ETA results shared across the hours in flight. The counter
    /// layer builds on the window thread and the close-zoom models on workers
    /// that run ahead of it; see [`Hour::etas`].
    eta_cache: crate::map::division_eta::EtaCache,
    /// Each hour's viewing-country visibility, shared by that hour's builders:
    /// the buildings, movers, fog and icon parts each read it on their own
    /// thread. Keyed on the tables it reads ([`perspective::VISION_TABLES`]),
    /// the branch and hour and the viewing choice.
    vision_memo: crate::map::memo::Keyed<perspective::Vision, VisionKey>,
    /// The combat hovers' modifier name index, built on first use from the
    /// static definition and dictionary tables and kept while an hour shares
    /// them.
    pub modifier_names: crate::combat::modifier_text::NameIndexCache,
    /// Each hour's battle plan, shared by the hours whose plan tables
    /// ([`orders::TABLES`]) are the same allocations, keyed also on the
    /// viewing country's slot, the only thing it reads from the perspective.
    order_memo: crate::map::memo::Recent<(crate::map::memo::TableKey, u32, bool), OrderField>,
    /// Each hour's counter type-icon relations, shared by the hours whose
    /// relation tables and viewing country's volunteer hosts are unchanged
    /// ([`counters::type_tint::Memo`]).
    type_tint_memo: counters::type_tint::Memo,
    mission_memo: counters::MissionMemo,
    /// Which provinces each snow and mud cell reads, built on first use.
    snow_cells: std::sync::OnceLock<Result<weather::SnowCells, (io::ErrorKind, String)>>,
    /// The content-derived movement map and defines behind route points,
    /// stack kinds and ETAs; `None` when the content could not be derived, in
    /// which case moving units stand at their province and have no ETA.
    pub movement: Option<Arc<crate::map::division_eta::MovementContent>>,
    /// Province pixel centroids and boundary crossing points, the two
    /// quantities the engine builds an order's polyline from.
    pub province_geometry: ProvinceGeometry,
    /// How long that one pass over `map/provinces.bmp` took, for reporting.
    pub province_geometry_build: Duration,
}

/// What one hour's [`perspective::Vision`] reads besides its tables: the hour
/// decides which wars are ongoing, and the viewing choice.
type VisionKey = (replay::BranchId, replay::Hour, perspective::Choice);
/// Hours whose visibility the builders share: the pool builds up to one hour
/// per builder at once, each with one perspective. An entry holds only the
/// vision tables of its hour.
const VISION_MEMO: usize = 8;

/// Battle plans kept for the hours in flight and a step back, as
/// [`VISION_MEMO`].
const ORDER_MEMO: usize = 8;

/// Type-icon relations kept as [`VISION_MEMO`].
const TYPE_TINT_MEMO: usize = 8;
const MISSION_MEMO: usize = 8;

/// One recorded hour of a document: its decoded frame, the fraction past it,
/// and the content it is drawn with. Everything built per hour is built here,
/// on whichever thread holds the frame.
pub struct Hour<'a> {
    pub content: &'a Content,
    pub frame: &'a DecodedFrame,
    /// See [`Document::hour_fraction`].
    pub hour_fraction: f32,
    /// The last perspective read for this hour; several builders need it.
    vision_cache: std::cell::RefCell<Option<(perspective::Choice, Arc<perspective::Vision>)>>,
    trigger_world: std::cell::OnceCell<dashboard::triggers::World<'a>>,
    pool_limits: crate::map::equipment_graphics::LimitValues,
}

impl std::ops::Deref for Hour<'_> {
    type Target = Content;

    fn deref(&self) -> &Content {
        self.content
    }
}

/// Camera-dependent preparation retained while only the hour fraction moves.
///
/// The raid icons are kept apart from the labels, so that a raid batch's
/// icons replace them alone ([`PreparedOverlay::replace_raids`]): their
/// layers are their depth order among themselves, and no other part of the
/// overlay reads them.
pub struct PreparedOverlay {
    /// The labels' prepared values, drawn through this view's native pools.
    labels: labels::PreparedLabels,
    /// The raid icons, drawn before the labels.
    raids: Batch,
    /// Prepared raid owners; presentation windows retain their own clock.
    native_raids: raids::PreparedRaids,
    /// Hover rectangles of the last native raid draw.
    raid_hits: Vec<raids::tooltips::Hit>,
    /// Where the raid icons stand in the batch [`Self::draw_into`] last
    /// drew, `None` when it left them out.
    raid_range: Option<std::ops::Range<usize>>,
    icons: counters::PreparedCounters,
    label_report: LabelReport,
    raid_report: raids::RaidReport,
    /// [`crate::map::lazy_icons::LazyTextures::version`] when it was laid out; a
    /// newer version may have decoded a flag or portrait it left out.
    lazy_version: u64,
    /// Its tables' land combat flags came from the engaged index; otherwise
    /// they are what a load at the hour rebuilds, and the window lays the
    /// overlay out again once the index covers the hour.
    engaged_covered: bool,
    /// The counter options and the view's counter memory it was laid out with, which its
    /// redraws keep.
    counter_view: counters::CounterView,
}
/// A dump retains the same presented windows across its scene frames.
pub struct DumpOverlay<'a> {
    hour: Hour<'a>,
    prepared: PreparedOverlay,
    tables: OverlayTables,
    camera: Camera,
    viewport: [u32; 2],
    ui_scale: f32,
    omitted: crate::map::layers::Omissions,
    pub batch: Batch,
    pub extra: Batch,
    pub sprites: Batch,
    pub labels: LabelReport,
    pub counters: CounterReport,
    pub raids: raids::RaidReport,
    pub settlement_warnings: Vec<String>,
}

impl DumpOverlay<'_> {
    pub fn schedule(&self) -> &crate::map::province_schedule::Shared {
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

    pub(crate) fn retries(&self) -> Vec<crate::map::icon_windows::Retry> {
        self.prepared
            .counter_view
            .presentation
            .windows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retries()
    }

    pub(crate) fn refuse_settlement(&mut self, retries: Vec<crate::map::icon_windows::Retry>) {
        self.settlement_warnings = retries
            .iter()
            .map(crate::map::icon_windows::Retry::warning)
            .collect();
    }

    pub fn present(&mut self, time: f32) -> io::Result<&Batch> {
        // Advance even when models are omitted; owners still use the province schedule.
        self.advance_provinces();
        loop {
            let before = self.hour.icons.lazy.version();
            (self.batch, self.labels, self.counters, self.raids) = self.prepared.draw(
                &self.hour,
                self.camera,
                self.viewport,
                self.ui_scale,
                self.omitted,
                time,
            )?;
            self.hour.icons.lazy.settle();
            if self.hour.icons.lazy.version() == before {
                return Ok(dump_sprites(&self.batch, &self.extra, &mut self.sprites));
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

pub(crate) fn dump_sprites<'a>(batch: &Batch, extra: &Batch, sprites: &'a mut Batch) -> &'a Batch {
    sprites.clear();
    sprites.quads.extend_from_slice(&batch.quads);
    sprites.quads.extend_from_slice(&extra.quads);
    sprites
}

impl PreparedOverlay {
    pub fn counter_view(&self) -> &counters::CounterView {
        &self.counter_view
    }

    /// See the field: whether the engaged index had covered the hour.
    pub fn engaged_covered(&self) -> bool {
        self.engaged_covered
    }

    /// Whether flags or portraits have decoded since this overlay was laid out.
    pub fn is_stale(&self, hour: &Hour<'_>) -> bool {
        self.lazy_version != hour.icons.lazy.version()
    }

    /// The overlay presented at `time` seconds on the view's own presentation clock (see
    /// [`counters::PreparedCounters::draw`]).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        omitted: crate::map::layers::Omissions,
        time: f32,
    ) -> io::Result<(Batch, LabelReport, CounterReport, raids::RaidReport)> {
        let mut batch = Batch::default();
        let mut report = CounterReport::default();
        self.draw_into(
            hour,
            camera,
            viewport,
            ui_scale,
            omitted,
            time,
            &mut batch,
            &mut report,
        )?;
        Ok((
            batch,
            self.label_report.clone(),
            report,
            self.raid_report.clone(),
        ))
    }

    /// [`PreparedOverlay::draw`] into the caller's batch and counter report, reusing their
    /// storage, so the window's per-fraction redraw allocates nothing. On an error both are
    /// left as they were.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_into(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        omitted: crate::map::layers::Omissions,
        time: f32,
        batch: &mut Batch,
        report: &mut CounterReport,
    ) -> io::Result<()> {
        let (labels, label_report, raid_quads) = hour.with_gui_context(
            counters::gui_windows::viewport(viewport, ui_scale),
            Some(time),
            |windows| {
                let inputs = CounterInputs {
                    content: &hour.counters,
                    equipment: Ok(hour.unit_models.sub_units.equipment_content()),
                    assets: &hour.icons.counters,
                    windows,
                    fonts: &hour.icons.fonts,
                    localisation: &hour.names.localisation,
                    map: &hour.map,
                    camera,
                    viewport,
                    ui_scale,
                    movement: hour.movement.as_deref(),
                    heights: hour.heights(),
                    settings: self.counter_view.settings,
                    selected: &self.counter_view.selected,
                    hovered: self.counter_view.hovered,
                    presentation: &self.counter_view.presentation,
                };
                self.icons.draw(&inputs, time)?;
                Ok(self.icons.flag_source(&hour.icons.counters))
            },
            |context| {
                let mut views = self
                    .counter_view
                    .presentation
                    .windows
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut labels = Batch::default();
                let report = self.labels.draw(
                    context,
                    &self
                        .counter_view
                        .presentation
                        .schedule()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                    &mut views.labels,
                    &mut labels,
                );
                let raids = hour.raid_quads(context, &self.native_raids, &mut views.raids);
                (labels, report, raids)
            },
        )?;
        self.label_report = label_report;
        (self.raids, self.raid_hits, self.raid_report) = raid_quads;
        let (counters, counter_report) = self.icons.drawn();
        report.assign(counter_report);
        self.raid_range = compose(
            batch,
            &OverlayParts {
                pings: &self.icons.fixed.pings.quads,
                bases: &self.icons.fixed.bases.quads,
                raids: &self.raids.quads,
                labels: &labels.quads,
                counters: &counters.quads,
                combats: &self.icons.fixed.combats.quads,
                naval_combats: &self.icons.drawn_combats().quads,
            },
            omitted,
        );
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

    /// Only icons submitted by the latest draw can own the hover.
    pub fn raid_at(&self, point: [f32; 2]) -> Option<&raids::tooltips::Hit> {
        self.raid_range.as_ref()?;
        self.raid_hits
            .iter()
            .filter(|hit| hit.contains(point))
            .max_by_key(|hit| hit.layer)
    }

    /// ToolTip draws after the map and combat window, with this hour's native flag atlases.
    pub fn draw_tooltip(
        &self,
        hour: &Hour<'_>,
        batch: &mut Batch,
        value: &str,
        point: crate::combat::tooltip_painter::TooltipPosition,
        extent: [f32; 2],
        time: f32,
    ) -> io::Result<std::collections::BTreeMap<String, usize>> {
        let Ok(defines) = &hour.icons.combat.tooltip_defines else {
            return Ok(Default::default());
        };
        hour.with_gui_context(
            extent.map(|v| v as i32),
            Some(time),
            |_| Ok(self.icons.flag_source(&hour.icons.counters)),
            |context| {
                crate::combat::tooltip_painter::draw(
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

    /// Refreshes raid values in the last drawn overlay without advancing its native windows.
    pub fn replace_raids(
        &mut self,
        hour: &Hour<'_>,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        raids: &raids::RaidIcons,
        drawn: Option<&mut Batch>,
    ) {
        self.native_raids = hour.raid_owners(camera, viewport, ui_scale, raids);
        if self.raid_range.is_none() {
            self.raid_report = self.native_raids.report.clone();
            return;
        }
        let laid_out = hour.with_gui_context(
            counters::gui_windows::viewport(viewport, ui_scale),
            None,
            |_| Ok(self.icons.flag_source(&hour.icons.counters)),
            |context| {
                hour.raid_quads(
                    context,
                    &self.native_raids,
                    &mut self
                        .counter_view
                        .presentation
                        .windows
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .raids,
                )
            },
        );
        let (laid_out, hits, report) = match laid_out {
            Ok(drawn) => drawn,
            Err(why) => {
                self.raid_report = self.native_raids.report.clone();
                self.raid_report.reasons.insert(
                    format!("{} is not drawn: {why}", raids::WINDOW),
                    raids.icons.len(),
                );
                return;
            }
        };
        if let (Some(batch), Some(range)) = (drawn, self.raid_range.clone()) {
            self.raid_range = Some(splice_raids(batch, range, &laid_out.quads));
        }
        self.raids = laid_out;
        self.raid_hits = hits;
        self.raid_report = report;
    }
}

/// The overlay's parts, each in its own push order.
struct OverlayParts<'a> {
    pings: &'a [crate::map::sprites::Quad],
    bases: &'a [crate::map::sprites::Quad],
    raids: &'a [crate::map::sprites::Quad],
    labels: &'a [crate::map::sprites::Quad],
    counters: &'a [crate::map::sprites::Quad],
    combats: &'a [crate::map::sprites::Quad],
    /// The naval combat icons the view's own windows drew ([`counters::PreparedCounters`]).
    naval_combats: &'a [crate::map::sprites::Quad],
}

/// `batch` emptied, then `parts` in draw order: pings and bases, raid icons
/// and labels, counters and combats, leaving out the `omitted` groups (raid
/// icons go with the labels). Where the raid icons stand, when drawn.
fn compose(
    batch: &mut Batch,
    parts: &OverlayParts<'_>,
    omitted: crate::map::layers::Omissions,
) -> Option<std::ops::Range<usize>> {
    batch.clear();
    if !omitted.counters {
        batch.quads.extend_from_slice(parts.pings);
        batch.quads.extend_from_slice(parts.bases);
    }
    let mut raids = None;
    if !omitted.labels {
        let start = batch.quads.len();
        batch.quads.extend_from_slice(parts.raids);
        raids = Some(start..batch.quads.len());
        batch.quads.extend_from_slice(parts.labels);
    }
    if !omitted.counters {
        batch.quads.extend_from_slice(parts.counters);
        batch.quads.extend_from_slice(parts.combats);
        batch.quads.extend_from_slice(parts.naval_combats);
    }
    raids
}

/// `batch` with the raid icons at `range` replaced by `raids`: where they
/// stand now.
fn splice_raids(
    batch: &mut Batch,
    range: std::ops::Range<usize>,
    raids: &[crate::map::sprites::Quad],
) -> std::ops::Range<usize> {
    let start = range.start;
    batch.quads.splice(range, raids.iter().cloned());
    start..start + raids.len()
}

/// Whether two quad lists are the same bit for bit.
pub fn same_quads(a: &[crate::map::sprites::Quad], b: &[crate::map::sprites::Quad]) -> bool {
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

impl Document {
    pub fn load(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        visual_mods: Option<&[String]>,
        texture_quality: u8,
    ) -> io::Result<Self> {
        Self::load_shared(
            path,
            options,
            allow_unverified,
            initial,
            Arc::new(Replay::open(fs::read(path)?)?),
            visual_mods,
            texture_quality,
        )
    }

    /// Load verified presentation assets using the same immutable recording
    /// handle as the native analytics service. Call only on a loading thread.
    pub fn load_shared(
        path: &Path,
        options: &ResolveOptions,
        allow_unverified: bool,
        initial: &InitialHour,
        replay: Arc<Replay>,
        visual_mods: Option<&[String]>,
        texture_quality: u8,
    ) -> io::Result<Self> {
        let mut warnings = replay.compatibility_gaps();
        engine::executable_tables::initialize(&options.game_dir)?;
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
        let mut content = installed_content::resolve(&replay.identity, options)?;
        let uncovered_reads = content.layers.report_uncovered_reads();
        if let Some(selected) = visual_mods {
            installed_content::content::visual::apply(
                &mut content.layers,
                options,
                selected,
                &std::sync::atomic::AtomicBool::new(false),
            )?;
            warnings.push("Local cosmetic artwork overrides are enabled for this campaign; they are not the recorded artwork.".into());
        }
        let content = Arc::new(content);
        let scripted = dashboard::scripted_text::ScriptedContent::read(
            Arc::clone(&content),
            recorded_language.clone(),
        );
        let map = map::load(&content.layers)?;
        let (terrain, default_map) = installed_content::terrain::read(&content.layers)?;
        // The game logs a malformed terrain entry or field and loads the rest.
        for reason in &terrain.skipped {
            warnings.push(format!(
                "Terrain content the game logs and reads past: {reason}."
            ));
        }
        let defines = installed_content::defines::read(&content.layers)?;
        // Every key as the game reads it (`installed_content::settings::SettingsKeys`).
        let settings = installed_content::settings::read_keys(&options.user_dir)?;
        let camera_tilt = settings
            .f32_at(&["camera_tilt"])
            .ok_or_else(|| invalid("settings.txt camera_tilt is not a number"))?;
        // The local graphics preference defaults on when no boolean value is stored.
        // The game halves the HDR scene; the output/interface remain full size.
        let game_full_resolution = settings
            .bool_at(&["maprenderingoptions", "draw_map_full_res"])
            .unwrap_or(true);
        let map_defines = crate::map::camera::MapDefines::load(&defines, camera_tilt)?;
        // The map is drawn as this client's game would draw it: its own `force_pow2_textures`
        // pads every texture file the passes load and sets their global defines.
        let decode = settings.texture_decode();
        // Text is read in the user's game language, not the recording's.
        let language = settings.language()?;
        let game_counters = installed_content::settings::Counters::read(&settings)
            .map_err(invalid)?
            .into();
        // Only the interface scale's default needs these; without them it falls
        // back to 1.0 and says so, rather than refusing the recording.
        let game_display = installed_content::settings::read_pdx(&options.user_dir)
            .and_then(|pdx| {
                installed_content::settings::Display::read(
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
                    let scale = display.effective_gui_scale(installed_content::settings::Desktop {
                        physical: CONSTRUCTED,
                        dpi: 96,
                    });
                    (frame, display.interface_to_frame(frame, scale))
                }
                Err(_) => (CONSTRUCTED, 1.0),
            };
            crate::map::native_shader::draw::gui_viewport(frame, scale)
        };
        // Wave 1: everything needing only layers, map and terrain runs in
        // parallel on scoped threads. Each job returns its bundle with its own
        // warnings; the main thread joins in the serial load order below, so
        // the first error and the warning order match the serial load within
        // this wave. A wave-1 failure precedes any wave-2 one, since wave 2
        // needs wave-1 values to start.
        // The strategic regions are parsed once for borders, counters and aircraft.
        let installed_content::regions::StrategicRegions {
            regions,
            notes: region_notes,
        } = installed_content::regions::read(&content.layers)?;
        warnings.extend(region_notes);
        let wave1 = std::thread::scope(|scope| -> io::Result<_> {
            let layers = &content.layers;
            let h_assets = scope.spawn(|| {
                let mut warnings = Vec::new();
                let assets = MapAssets::load(
                    layers,
                    &defines,
                    crate::map::camera::MapSize::of(&map),
                    &terrain,
                    &default_map,
                    &mut warnings,
                    texture_quality,
                    decode,
                );
                (assets, warnings)
            });
            let h_icons = scope.spawn(|| {
                MapIcons::load(
                    layers,
                    &defines,
                    &map,
                    &regions,
                    &language,
                    [map.width, map.height],
                    asset_viewport,
                    decode,
                )
            });
            let h_names = scope.spawn(|| NameContent::load(layers, &map, &language));
            let h_counters = scope.spawn(|| CounterContent::load(layers, &map, &defines, &regions));
            let h_borders_in = scope.spawn(|| -> io::Result<_> {
                let border_assets = BorderAssets::load(
                    layers,
                    &defines,
                    [map.width, map.height],
                    texture_quality,
                    decode,
                )?;
                let states = installed_content::states::read(layers, &defines, &map.definitions)?;
                let adjacencies = installed_content::adjacencies::read(layers)?;
                Ok((border_assets, states, adjacencies))
            });
            let h_rail_reads = scope.spawn(|| -> io::Result<_> {
                let (cities, city_styles) = installed_content::cities::read(layers)?;
                let buildings_read = installed_content::positions::read_buildings(layers)?;
                let building_types = installed_content::buildings::read(layers)?;
                let rail_defines = rail_layer::RailDefines::load(&defines)?;
                Ok((
                    cities,
                    city_styles,
                    buildings_read,
                    building_types,
                    rail_defines,
                ))
            });
            let h_trains = scope.spawn(|| -> io::Result<_> {
                let mut warnings = Vec::new();
                let trains = std::sync::Arc::new(crate::map::trains::TrainAssets::load(
                    layers,
                    decode,
                    &defines,
                    &mut warnings,
                )?);
                Ok((trains, warnings))
            });
            let h_aircraft = scope.spawn(|| -> io::Result<_> {
                let mut warnings = Vec::new();
                let aircraft = std::sync::Arc::new(crate::map::aircraft::AircraftAssets::load(
                    layers,
                    decode,
                    &map,
                    &defines,
                    &regions,
                    &mut warnings,
                )?);
                Ok((aircraft, warnings))
            });
            let h_bridges = scope.spawn(|| -> io::Result<_> {
                let mut warnings = Vec::new();
                let bridges = Bridges::load(layers, decode, &defines, &mut warnings)?;
                Ok((bridges, warnings))
            });
            let h_units = scope.spawn(|| -> io::Result<_> {
                let mut warnings = Vec::new();
                let unit_models = UnitModelAssets::load(
                    layers,
                    decode,
                    &defines,
                    &map,
                    &default_map,
                    &mut warnings,
                );
                Ok((unit_models, warnings))
            });
            let h_orders = scope.spawn(|| -> io::Result<_> {
                let mut warnings = Vec::new();
                let order_assets = OrderAssets::load(layers, decode, &defines, &mut warnings)?;
                Ok((order_assets, warnings))
            });
            let h_weather = scope.spawn(|| -> io::Result<_> {
                let assets = crate::map::weather_effects::Assets::load(
                    layers,
                    &defines,
                    [map.width, map.height],
                    decode,
                )?;
                Ok(assets)
            });
            let h_mapbuildings = scope.spawn(|| -> io::Result<_> {
                let mut warnings = Vec::new();
                let map_buildings =
                    MapBuildingAssets::load(layers, decode, &map, &defines, &mut warnings)?;
                Ok((map_buildings, warnings))
            });
            let h_geometry = scope.spawn(|| {
                let started = Instant::now();
                let geometry = ProvinceGeometry::build(&map);
                (geometry, started.elapsed())
            });
            let h_river_assets = scope.spawn(|| {
                RiverAssets::load(layers, [map.width, map.height], texture_quality, decode)
            });
            let h_rail_assets =
                scope.spawn(|| RailAssets::load(layers, [map.width, map.height], decode));
            // The ideology groups supply the tactic triggers' ideology names.
            // Loading trigger keys registers those content names.
            let h_ideologies = scope.spawn(|| installed_content::ideologies::read(layers));
            // The resource declarations, for the tactic triggers' resource
            // names as the content is read.
            let h_resources = scope.spawn(|| installed_content::resources::read(layers));
            // The initial seek decodes from its block start to the hour, all
            // in memory; it overlaps the file reads above on its own thread.
            // Only playback moves here, so the joins below keep their order.
            let h_initial = scope.spawn(|| Self::initial_playback(replay, initial));
            let (assets_result, warnings) = h_assets.join().expect("assets loader panicked");
            let assets = assets_result?;
            let mut joined_warnings = warnings;
            let mut icons = h_icons.join().expect("icons loader panicked")?;
            joined_warnings.extend(icons.counters.warnings.iter().cloned());
            joined_warnings.extend(
                icons
                    .raids
                    .warnings
                    .iter()
                    .map(|(_, message)| message.clone()),
            );
            if icons.counters.flags.is_empty() {
                joined_warnings.push(
                    "No flag of the small set resolved, so unit counters carry no country flag."
                        .into(),
                );
            }
            let names = h_names.join().expect("names loader panicked")?;
            // The map-icon windows' resources need the localisation their texts are shown
            // through, which the names pass reads.
            joined_warnings.extend(icons.load_gui(layers, &names.localisation)?);
            let mut counters = h_counters.join().expect("counters loader panicked")?;
            if let Err(error) = &counters.fog {
                joined_warnings.push(format!(
                    "Fog of war cannot be rebuilt from this content ({error}): no fog is drawn and land stacks are drawn in full."
                ));
            }
            let (border_assets, states, adjacencies) =
                h_borders_in.join().expect("borders loader panicked")?;
            if let Err(why) = &border_assets.faction_highlight_color {
                joined_warnings.push(format!(
                    "Faction-theatre hover outline is unavailable: {why}"
                ));
            }
            let river_assets = h_river_assets
                .join()
                .expect("river assets loader panicked")?;
            let frame_assets =
                FrameAssets::load(layers, decode, icons.raids.facts(), &mut joined_warnings)?;
            let (cities, city_styles, buildings_read, building_types, rail_defines) =
                h_rail_reads.join().expect("rail reads loader panicked")?;
            let (ideologies, warnings) =
                h_ideologies.join().expect("ideologies loader panicked")?;
            joined_warnings.extend(
                warnings
                    .into_iter()
                    .map(|(_, warning)| format!("Combat tactics: ideologies: {warning}")),
            );
            let (resources, warnings) = h_resources.join().expect("resources loader panicked")?;
            joined_warnings.extend(
                warnings
                    .into_iter()
                    .map(|(_, warning)| format!("Combat tactics: resources: {warning}")),
            );
            // Count triggers resolve these buildings, ideologies and
            // resources; the building database is the one read above. The
            // raids' triggers read the same settled names.
            icons.raids.triggers =
                icons
                    .combat
                    .set_count_trigger_names(&building_types, &ideologies, &resources);
            icons.raids.ideologies = Some(ideologies.clone());
            // A naval counter at sea's `counter_ideology` frame is its group's number.
            counters.ideologies = ideologies;
            let (trains, warnings) = h_trains.join().expect("trains loader panicked")?;
            joined_warnings.extend(warnings.into_iter().map(|(_, message)| message));
            let rail_assets = h_rail_assets.join().expect("rail assets loader panicked")?;
            let (bridges, warnings) = h_bridges.join().expect("bridges loader panicked")?;
            joined_warnings.extend(warnings.into_iter().map(|(_, message)| message));
            let (aircraft, warnings) = h_aircraft.join().expect("aircraft loader panicked")?;
            joined_warnings.extend(warnings.into_iter().map(|(_, message)| message));
            let (unit_models, warnings) = h_units.join().expect("units loader panicked")?;
            joined_warnings.extend(warnings);
            let (order_assets, warnings) = h_orders.join().expect("orders loader panicked")?;
            joined_warnings.extend(warnings.into_iter().map(|(_, message)| message));
            let weather_effects = h_weather.join().expect("weather loader panicked")?;
            if !weather_effects.unresolved.is_empty() {
                joined_warnings.push(format!(
                    "{} pdxparticle objects name no particle block and draw nothing: {}",
                    weather_effects.unresolved.len(),
                    weather_effects.unresolved.join(", ")
                ));
            }
            let (map_buildings, warnings) = h_mapbuildings
                .join()
                .expect("map buildings loader panicked")?;
            joined_warnings.extend(warnings.into_iter().map(|(_, message)| message));
            let (province_geometry, province_geometry_build) =
                h_geometry.join().expect("geometry loader panicked");
            let playback = h_initial.join().expect("initial seek panicked")?;
            Ok((
                assets,
                icons,
                names,
                counters,
                border_assets,
                states,
                adjacencies,
                frame_assets,
                cities,
                city_styles,
                buildings_read,
                building_types,
                rail_defines,
                trains,
                aircraft,
                bridges,
                unit_models,
                order_assets,
                weather_effects,
                map_buildings,
                province_geometry,
                province_geometry_build,
                river_assets,
                rail_assets,
                joined_warnings,
                playback,
            ))
        })?;
        let (
            mut assets,
            icons,
            names,
            mut counters,
            border_assets,
            states,
            adjacencies,
            frame_assets,
            cities,
            city_styles,
            buildings_read,
            building_types,
            rail_defines,
            trains,
            aircraft,
            bridges,
            unit_models,
            order_assets,
            weather_effects,
            map_buildings,
            province_geometry,
            province_geometry_build,
            river_assets,
            rail_assets,
            wave_warnings,
            playback,
        ) = wave1;
        warnings.extend(wave_warnings);
        let (states, state_conflicts) = states;
        warnings.extend(state_conflicts.into_iter().map(|(_, message)| message));
        // Wave 2: everything needing assets, in parallel. Joins run in the
        // serial load order, so the first error and the warning order match
        // the serial load within this wave.
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
        let (
            labels,
            straits,
            border_meshes,
            border_trace,
            river_mesh,
            river_trace,
            river_build,
            tree_field,
            tree_assets,
            tree_geometry,
            tree_build,
            sprawl,
            sprawl_build,
            rail_geometry,
            rail_build,
            movement,
            wave2_warnings,
        ) = std::thread::scope(|scope| -> io::Result<_> {
            let layers = &content.layers;
            let h_movement = scope.spawn(|| -> io::Result<_> {
                let mut movement = crate::map::division_eta::MovementContent::new(
                    installed_content::movement::MovementMap::read(layers, &defines)?,
                    installed_content::movement::MovementDefines::read(&defines)?,
                );
                movement.set_task_force_terrain(
                    icons.combat.movement_terrain(),
                    &regions,
                    counters.subunit_tokens.clone(),
                );
                Ok(movement)
            });
            let h_labels = scope.spawn(|| LabelContent::load(&defines));
            let h_borders = scope.spawn(|| -> io::Result<_> {
                let topology = Topology::new(&map, states.definitions(), &regions, &adjacencies)?;
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
            });
            let h_rivers = scope.spawn(|| -> io::Result<_> {
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
            });
            let h_trees = scope.spawn(|| -> io::Result<_> {
                let started = Instant::now();
                let tree_field = trees::generate(
                    &TreeImage {
                        width: assets.trees.width,
                        height: assets.trees.height,
                        indices: &assets.trees.indices,
                    },
                    &heights,
                )?;
                let mut warnings = Vec::new();
                let tree_assets = TreeAssets::load(
                    layers,
                    decode,
                    &tree_field.palettes,
                    &assets.rivers,
                    &mut warnings,
                )?;
                let tree_geometry = trees::geometry(&tree_assets)?;
                Ok((
                    tree_field,
                    tree_assets,
                    tree_geometry,
                    started.elapsed(),
                    warnings,
                ))
            });
            let h_sprawl = scope.spawn(|| -> io::Result<_> {
                let started = Instant::now();
                let sprawl_plan = sprawl::generate(
                    &cities,
                    &city_styles,
                    &assets.spawn_city,
                    &heights,
                    &map.province_ids,
                    &assets.rivers,
                )?;
                let mut warnings = Vec::new();
                let sprawl = Sprawl::load(layers, decode, &sprawl_plan, &mut warnings)?;
                Ok((sprawl, started.elapsed(), warnings))
            });
            let h_railgeom = scope.spawn(|| -> io::Result<_> {
                let started = Instant::now();
                let rail_geometry = rail_layer::RailGeometry::new(
                    &map,
                    &buildings_read,
                    &building_types,
                    &assets.rivers,
                    &heights,
                    rail_defines,
                )?;
                Ok((rail_geometry, started.elapsed()))
            });
            let labels = h_labels.join().expect("labels loader panicked")?;
            let (straits, border_meshes, border_trace) =
                h_borders.join().expect("borders loader panicked")?;
            let (river_mesh, river_trace, river_build) =
                h_rivers.join().expect("rivers loader panicked")?;
            let (tree_field, tree_assets, tree_geometry, tree_build, warnings) =
                h_trees.join().expect("trees loader panicked")?;
            let mut joined = warnings;
            let (sprawl, sprawl_build, warnings) =
                h_sprawl.join().expect("sprawl loader panicked")?;
            joined.extend(warnings);
            let (rail_geometry, rail_build) =
                h_railgeom.join().expect("rail geometry loader panicked")?;
            // The movement map failing refuses glide, stack kinds and ETAs,
            // not the document: moving units then stand at their province.
            let movement = match h_movement.join().expect("movement loader panicked") {
                Ok(movement) => {
                    let unavailable = movement.map.unavailable_edges();
                    if let Some(((a, b), reason)) = unavailable.iter().next() {
                        joined.push(format!(
                            "{} neighbouring province pairs have no content movement distance, so routes over them have no progress or ETA; the first is {a}-{b}: {reason}",
                            unavailable.len()
                        ));
                    }
                    Some(Arc::new(movement))
                }
                Err(error) => {
                    joined.push(format!(
                        "Unit movement is unavailable for this content: {error}. Moving units stand at their province, and no route point or ETA is drawn."
                    ));
                    None
                }
            };
            Ok((
                labels,
                straits,
                border_meshes,
                border_trace,
                river_mesh,
                river_trace,
                river_build,
                tree_field,
                tree_assets,
                tree_geometry,
                tree_build,
                sprawl,
                sprawl_build,
                rail_geometry,
                rail_build,
                movement,
                joined,
            ))
        })?;
        warnings.extend(wave2_warnings);
        // Raid `is_coastal` reads the movement map's static province terrain.
        let mut icons = icons;
        icons.raids.terrain = movement.clone();
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
            "A division's model follows the engine's own ladder: the template's saved model, then the equipment pair's saved model, authored graphic pools and conventional candidates, else the viewing country's fallback queues, then the recorded model number and sub-unit key under the cosmetic tag, tag, graphical culture and bare name, then the recorded camouflage. Template composition and model choices are recorded hourly, so a template edit reaches its figures within the hour; a template whose model inputs were unreadable that hour draws no model for its divisions, and its counters are unaffected. Equipment is recorded daily, with an hourly record of divisions deployed since; an existing division's mid-day equipment change reaches its figure at the next daily refresh; the country's stored best-variant candidates are daily too, and a no-pair fallback whose order needs the unrecorded variant labels is not drawn. The window advances authored state clips and attachment transforms with presentation seed 0x414e494d XOR the unit handle; original clocks and RNG ordering are unrecorded. A time-zero dump preserves the default first pose. A standing figure's facing is a hash, because nothing records the heading the engine last turned it to."
                .into(),
        );
        // orders: the recorded player's battle plan
        warnings.push(
            "Map buildings keep the destroyed-mesh hysteresis, and stay drawn above the update distance once created below it, from this viewer's own camera history since the last seek; a seek starts every slot healthy and uncreated, as a freshly loaded game does. A dump has no camera history unless --building-camera supplies it. Popup growth and the shared model creation allowance run on the viewer's frame clock; native capture-time presentation remains unrecorded."
                .into(),
        );
        // Validate province membership before rebuilding region battle-name candidates.
        let battle_name_provinces =
            crate::combat::battle_name_candidates(&regions, states.definitions(), &|province| {
                map.definitions.contains_key(&province)
            });
        let state_name_keys = states
            .definitions()
            .iter()
            .map(|(id, state)| (*id, state.name_key.clone()))
            .collect();
        let province_hover = crate::map::province_tooltips::Content::load(
            &content.layers,
            &defines,
            states.definitions(),
            &icons.combat,
        );
        warnings.extend(province_hover.warnings());
        let naval_dominance = crate::map::naval_dominance::Content::new(
            &map,
            &regions,
            icons
                .combat
                .movement_terrain()
                .as_deref()
                .map_err(String::as_str),
            &defines,
        );
        let content = Content {
            visual_mods: visual_mods.map(<[String]>::to_vec),
            texture_quality,
            weather_effects,
            map: Arc::new(map),
            map_defines,
            game_display,
            game_counters,
            game_full_resolution,
            assets,
            warnings,
            uncovered_reads,
            source_path: path.to_path_buf(),
            // borders
            border_assets,
            border_meshes,
            straits,
            battle_name_provinces,
            state_name_keys,
            province_hover,
            recorded_language,
            scripted,
            strategic_regions: Arc::new(regions),
            naval_dominance,
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
            political_gpu: crate::map::political_gpu::Cache::default(),
            eta_cache: crate::map::division_eta::EtaCache::default(),
            vision_memo: crate::map::memo::Keyed::new(perspective::VISION_TABLES, VISION_MEMO),
            modifier_names: crate::combat::modifier_text::name_index_cache(),
            order_memo: crate::map::memo::Recent::new(ORDER_MEMO),
            type_tint_memo: crate::map::memo::Keyed::new(
                counters::type_tint::TABLES,
                TYPE_TINT_MEMO,
            ),
            mission_memo: crate::map::memo::Recent::new(MISSION_MEMO),
            snow_cells: std::sync::OnceLock::new(),
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

    /// A standalone selection over content already held by another native map.
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
                for branch in &playback.replay().branches {
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
        Ok(Hour::new(&self.content, self.frame()?, self.hour_fraction))
    }

    /// What this hour's counters can see, for the combat windows and the
    /// limitation lines.
    pub fn vision(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<Arc<perspective::Vision>> {
        self.hour()?.vision(perspective)
    }

    /// The viewing country's tag at the hour on screen.
    pub fn viewing_tag(&self, vision: &perspective::Vision) -> io::Result<String> {
        self.hour()?.viewing_tag(vision)
    }

    /// The cold paths a `--dump` takes; the window builds the same hours on
    /// its hour builders instead (`crate::hours`).
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
        crate::map::map_buildings::PreparedBuildings,
        crate::map::map_buildings::MapBuildingSummary,
    )> {
        self.hour()?.map_building_field(perspective)
    }

    pub fn aircraft_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<crate::map::aircraft::AircraftPlan> {
        self.hour()?.aircraft_plan(perspective)
    }

    pub fn train_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<crate::map::trains::TrainPlan> {
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

    pub fn unit_model_field(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<(crate::map::unit_history::Field, ModelReport)> {
        self.hour()?.unit_model_field(perspective)
    }

    pub fn intel_map(&self, perspective: &perspective::Choice) -> io::Result<IntelMap> {
        self.hour()?.intel_map(perspective)
    }

    pub fn place_names(&self) -> io::Result<Placements> {
        self.hour()?.place_names()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dump_overlay(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        perspective: &perspective::Choice,
        omitted: crate::map::layers::Omissions,
        counter_settings: counters::CounterSettings,
    ) -> io::Result<DumpOverlay<'_>> {
        self.hour()?.dump_overlay(
            camera,
            viewport,
            ui_scale,
            perspective,
            omitted,
            counter_settings,
        )
    }

    pub fn rail_mesh(&self) -> io::Result<RailMesh> {
        self.hour()?.rail_mesh()
    }

    pub fn demilitarized_zone(&self, regions: &Regions) -> io::Result<Zone> {
        let override_ids = self.demilitarized_override.clone();
        self.hour()?.demilitarized_zone(regions, &override_ids)
    }

    /// The decoded frame of the hour on screen, or a named error for a recorded gap.
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
        }
    }
}

/// The hour's camera-independent overlay tables, built once per hour and
/// perspective ([`Hour::hour_tables`]) and reused by every camera's
/// [`Hour::prepare_overlay_cached`].
#[derive(Clone)]
pub(crate) struct OverlayTables {
    counters: counters::HourTables,
    labels: labels::LabelTables,
    /// The engaged-flag index covered this hour when these were built.
    pub(crate) engaged_covered: bool,
    /// Built with the rest on the hour builders only: raid offer evaluation
    /// runs path searches and must never land on the frame path.
    raids: Arc<raids::RaidIcons>,
    /// The branch, hour and canonical viewing country the tables are for.
    built_for: (replay::BranchId, replay::Hour, u32),
}

impl OverlayTables {
    /// See [`counters::HourTables::land_combat_gaps`].
    #[cfg(test)]
    pub(crate) fn land_combat_gaps(&self) -> (&counters::Gaps, &counters::Gaps) {
        self.counters.land_combat_gaps()
    }

    /// The raid icons, to keep drawing while the builders prepare new ones.
    pub(crate) fn raids(&self) -> Arc<raids::RaidIcons> {
        Arc::clone(&self.raids)
    }

    /// The canonical viewing country the tables are for.
    pub(crate) fn ours(&self) -> u32 {
        self.built_for.2
    }

    pub(crate) fn vision(&self) -> &Arc<perspective::Vision> {
        self.counters.vision()
    }

    /// Replaces the raid icons, with more offers evaluated for the same hour
    /// and viewing country. Icons built for another hour or country are
    /// refused: `false`, and the tables keep theirs.
    pub(crate) fn set_raids(&mut self, raids: Arc<raids::RaidIcons>) -> bool {
        let (branch, hour, ours) = self.built_for;
        if !raids.drawable_at(branch, hour, ours) {
            return false;
        }
        self.raids = raids;
        true
    }
}

impl<'a> Hour<'a> {
    pub fn new(content: &'a Content, frame: &'a DecodedFrame, hour_fraction: f32) -> Self {
        Self {
            content,
            frame,
            hour_fraction,
            vision_cache: std::cell::RefCell::new(None),
            trigger_world: std::cell::OnceCell::new(),
            pool_limits: Default::default(),
        }
    }

    pub fn trigger_world(&self) -> &dashboard::triggers::World<'a> {
        self.trigger_world.get_or_init(|| {
            dashboard::triggers::World::read(self.frame, self.content.icons.raids.facts())
        })
    }

    /// This hour's derived ETAs, cached and shared by both consumers.
    ///
    /// The counter layer builds on the window thread and the close-zoom models
    /// on workers that run ahead of it, so the cache lives on the shared
    /// [`Content`] and holds the hours in flight rather than only the last one.
    pub fn etas(&self) -> io::Result<Arc<crate::map::division_eta::Etas>> {
        self.eta_cache
            .get_or_insert_with((self.frame.branch, self.frame.hour), || {
                crate::map::division_eta::Etas::read(self.frame, self.movement.as_deref())
            })
    }

    /// The heightmap as the map passes sample it.
    pub fn heights(&self) -> crate::map::borders::HeightField<'_> {
        crate::map::borders::HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        }
    }

    /// Build only the current hour's railway graph; static geography is cached.
    pub fn rail_mesh(&self) -> io::Result<RailMesh> {
        Ok(self.rail_mesh_of(&rail_layer::RailNetwork::read(self.frame)?))
    }

    /// The railway of a network read from an hour ([`rail_layer::RailNetwork::read`]).
    pub fn rail_mesh_of(&self, network: &rail_layer::RailNetwork) -> RailMesh {
        self.rail_geometry.build_network(
            network,
            &crate::map::borders::HeightField {
                width: self.assets.heightmap.width,
                height: self.assets.heightmap.height,
                values: self.assets.heightmap_texels(),
            },
        )
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
    /// memoised, with the hour's raid arrows appended ([`raids::arrows`]).
    pub fn order_field(&self, perspective: &perspective::Choice) -> io::Result<Arc<OrderField>> {
        self.order_field_with_allied(perspective, false)
    }

    pub fn order_field_with_allied(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
    ) -> io::Result<Arc<OrderField>> {
        let plan = self.plan_field(perspective, show_allied)?;
        let ours = self.vision(perspective)?.ours();
        let heights = self.heights();
        let counters = &self.counters;
        let anchor = |province: u32, kind: u32| counters.anchor(province, kind);
        let declared = |name: &str| self.unit_models.declares(name);
        let evaluate = |items: &[installed_content::parser::Item], scopes| {
            self.trigger_world().evaluate(items, scopes)
        };
        let variant_model = |variant: u64, country: u32| {
            self.unit_models.aircraft_entity(
                self.frame,
                crate::map::equipment_graphics::Limits::new(&self.pool_limits, &evaluate),
                variant,
                country,
            )
        };
        let (raid_field, report) = raids::arrows::build(
            &raids::arrows::ArrowInputs {
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
        );
        if raid_field.draws.is_empty() && report == Default::default() {
            return Ok(plan);
        }
        let mut field = orders::append(&plan, &raid_field);
        field.report.raid_arrows = report;
        Ok(Arc::new(field))
    }

    /// The battle plan alone; ordinary plans are memoised on [`orders::TABLES`].
    fn plan_field(
        &self,
        perspective: &perspective::Choice,
        show_allied: bool,
    ) -> io::Result<Arc<OrderField>> {
        let ours = self.vision(perspective)?.ours();
        let links: BTreeMap<u32, u32> = tables::core::<row_tables::CountryLinks>(self.frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let build = |frame: &DecodedFrame| {
            let heights = self.heights();
            orders::build(
                &orders::OrderInputs {
                    assets: &self.orders,
                    raids: Some(&self.icons.raids),
                    geometry: &self.province_geometry,
                    movement: self.movement.as_deref().map(|content| &content.map),
                    map: &self.map,
                    heights: &heights,
                    ours,
                    show_allied,
                    links: &links,
                    labels: self.icons.fonts.arrow().map(|font| orders::LabelInputs {
                        font,
                        localisation: &self.names.localisation,
                    }),
                },
                frame,
            )
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
                (
                    crate::map::memo::TableKey::of(self.frame, &tables),
                    ours,
                    show_allied,
                ),
                || build(&crate::map::memo::project(self.frame, &tables)),
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
        crate::map::map_buildings::PreparedBuildings,
        crate::map::map_buildings::MapBuildingSummary,
    )> {
        self.map_buildings.field(
            self.frame,
            &*self.vision(perspective)?,
            &crate::map::map_buildings::CityTerrain {
                width: self.map.width,
                height: self.map.height,
                spawn_city: &self.assets.spawn_city,
            },
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
    ) -> io::Result<crate::map::unit_models::PreparedModels> {
        let content = &self.counters;
        let definitions = &self.map.definitions;
        let etas = self.etas()?;
        let movement = self.movement.as_deref();
        let world = movement.map(|movement| crate::map::counter_move::RouteWorld {
            map: &movement.map,
            heights: self.heights(),
            width: self.map.width as f32,
        });
        let anchor = |province, slot| content.anchor(province, slot);
        // Whether a province is land: the content movement map's terrain flag, else the map's
        // own kind. It picks a task force's slot 19.
        let land = |province| match movement.and_then(|movement| movement.map.terrain(province)) {
            Some(terrain) => terrain.flags & 1 != 0,
            None => matches!(
                definitions.get(&province).map(|it| it.kind),
                Some(map::ProvinceKind::Land)
            ),
        };
        let motions = crate::map::task_force_visual::TaskForceMotions::read(self.frame)?;
        let missions = crate::map::task_force_visual::MissionSkips::read(self.frame)?;
        let region = |province| content.region(province);
        let evaluate = |items: &[installed_content::parser::Item], scopes| {
            self.trigger_world().evaluate(items, scopes)
        };
        self.unit_models
            .prepare(crate::map::unit_models::PrepareInputs {
                frame: self.frame,
                limits: crate::map::equipment_graphics::Limits::new(&self.pool_limits, &evaluate),
                vision: &*self.vision(perspective)?,
                etas: &etas,
                anchor: &anchor,
                world: world.as_ref(),
                terrain: &content.combat_terrain,
                ships: &crate::map::unit_models::ShipPlaces {
                    motions: &motions,
                    movement,
                    land: &land,
                    missions: &missions,
                    region: &region,
                    scale: &|province, kind| content.stack_scale(province, kind),
                },
            })
    }

    pub fn unit_model_field(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<(crate::map::unit_history::Field, ModelReport)> {
        let mut prepared = self.prepare_unit_models(perspective)?;
        prepared.add_raid_units(
            &self.unit_models,
            &self.order_field(perspective)?.raid_units,
        );
        Ok((prepared.field(&self.unit_models), prepared.report))
    }

    pub fn aircraft_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<crate::map::aircraft::AircraftPlan> {
        let heights = HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        };
        let movement = self.movement.as_deref();
        let world = movement.map(|movement| crate::map::counter_move::RouteWorld {
            map: &movement.map,
            heights: self.heights(),
            width: self.map.width as f32,
        });
        let motions = crate::map::task_force_visual::TaskForceMotions::read(self.frame)?;
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
        let origin = |force: &replay::rows::TaskForceRow| {
            motions.settled_origin(
                force,
                movement,
                world.as_ref(),
                &|province, slot| self.counters.anchor(province, slot),
                &land,
            )
        };
        let evaluate = |items: &[installed_content::parser::Item], scopes| {
            self.trigger_world().evaluate(items, scopes)
        };
        let model = |variant, country| {
            self.unit_models.aircraft_entity(
                self.frame,
                crate::map::equipment_graphics::Limits::new(&self.pool_limits, &evaluate),
                variant,
                country,
            )
        };
        crate::map::aircraft::AircraftPlan::build(
            self.frame,
            &self.aircraft,
            &self.counters,
            &self.province_geometry,
            &heights,
            &*self.vision(perspective)?,
            &model,
            &origin,
        )
    }

    pub fn train_plan(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<crate::map::trains::TrainPlan> {
        let heights = HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        };
        crate::map::trains::TrainPlan::build(
            self.frame,
            self.vision(perspective)?.ours(),
            &self.rail_geometry,
            &heights,
            &self.trains,
            &self.trains.cultures,
        )
    }

    /// What the hour on screen could not draw, one line per degraded table.
    ///
    /// The map passes degrade on their own; this is the single list of what
    /// that cost, so the window's warning area and a `--dump` report the same
    /// thing. A core table's gap is not here: it is the frame error above.
    pub fn limitations(&self, perspective: &perspective::Choice) -> io::Result<Limitations> {
        self.limitations_with(perspective, None)
    }

    /// [`Hour::limitations`] with this hour's intel map already built for the
    /// same perspective, so its fogged count need not rebuild it.
    pub fn limitations_with(
        &self,
        perspective: &perspective::Choice,
        intel: Option<&IntelMap>,
    ) -> io::Result<Limitations> {
        let frame = self.frame;
        let mut out = tables::limitations(frame);
        out.note_trains(frame);
        out.note_aircraft(frame);
        out.note_point_lights();
        // counters: which country the map is drawn for. Stated on every frame,
        // degraded or not, because every counter on it depends on the answer.
        let vision = self.vision(perspective)?;
        out.note_perspective(&tables::Perspective {
            tag: self.viewing_tag(&vision)?,
            resolution: vision.resolution().clone(),
            fog: vision.fog(),
        });
        // orders: what the drawn battle plan could not take from the recording.
        // The count is the perspective's own orders, which is the cheap half of
        // `Document::order_field` and needs none of its geometry.
        let links: BTreeMap<u32, u32> = tables::core::<row_tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        out.note_battle_plan(orders::own_order_count(frame, &links, vision.ours())?);
        // raids: what the raid arrows could not take from this hour.
        if let Ok(raids) = frame.complete::<row_tables::Raids>() {
            let ours: Vec<_> = raids
                .iter()
                .filter(|raid| raid.parent_slot == vision.ours() && raid.phase != 5)
                .collect();
            let launched = ours.iter().filter(|raid| raid.phase == 4).count();
            out.note_raid_arrows(ours.len(), launched);
        }
        // counters: what the fog-on sight rule could not take from this hour.
        // With fog off every stack is shown and none of it decides anything.
        if vision.fog() {
            out.note_fog_on_sight(frame);
            if let Some(table) = vision.undecided() {
                out.note_undecided_sight(table);
            }
            if let Some(input) = vision.visibility_gap() {
                out.note_visibility_gap(input);
            }
        }
        // fog of war: stated only when drawn; the fog-on gap is stated above.
        if vision.province_fog().is_some() {
            let built;
            let intel = match intel {
                Some(intel) => intel,
                None => {
                    built = textures::intel_map(&self.map, vision.province_fog())?;
                    &built
                }
            };
            out.note_fog_of_war(
                intel.texels.iter().filter(|&&b| b > 0).count(),
                self.hour_fraction,
            );
        }
        for table in vision.truncated() {
            out.note_field(
                &format!("{table} (perspective)"),
                "the rows this hour did not reach are read as no relation and no detection at \
                 all, so a stack among them carries the game's question mark where the game drew \
                 it in full",
            );
        }
        // names: bytes the port cannot lay out as text. The province keeps its static name rather
        // than being drawn with replacement marks; see `map::labels`.
        let unreadable = labels::unreadable_overrides(frame)?;
        if !unreadable.is_empty() {
            out.note_field(
                "ProvinceNames.name",
                &format!(
                    "the recorded name of {} {} ({}) is not text this port can lay out, so {} \
                     keeps the static localised name of its province",
                    unreadable.len(),
                    if unreadable.len() == 1 {
                        "province"
                    } else {
                        "provinces"
                    },
                    unreadable
                        .iter()
                        .take(8)
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    if unreadable.len() == 1 { "it" } else { "each" },
                ),
            );
        }
        Ok(out)
    }

    /// Political naval FX uses the same viewing country as fog and counters.
    pub fn naval_fill(
        &self,
        choice: &perspective::Choice,
    ) -> io::Result<crate::map::naval_dominance::Fill> {
        Ok(self
            .naval_dominance
            .read(self.frame, self.vision(choice)?.ours()))
    }

    // counters: the viewing country
    /// What this hour's counters can see, for the limitation lines and the header.
    pub fn vision(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<Arc<perspective::Vision>> {
        if let Some((choice, vision)) = self.vision_cache.borrow().as_ref()
            && choice == perspective
        {
            return Ok(Arc::clone(vision));
        }
        let key = (self.frame.branch, self.frame.hour, perspective.clone());
        let vision = self.vision_memo.get_or_build(self.frame, key, |frame| {
            let links: BTreeMap<u32, u32> = tables::core::<row_tables::CountryLinks>(frame)?
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            let tags: BTreeMap<u32, String> = tables::core::<row_tables::Countries>(frame)?
                .iter()
                .map(|country| {
                    (
                        country.canonical_slot,
                        String::from_utf8_lossy(&country.tag).into_owned(),
                    )
                })
                .collect();
            perspective::Vision::read(frame, &links, &tags, perspective, self.counters.fog())
                .map(Arc::new)
        })?;
        *self.vision_cache.borrow_mut() = Some((perspective.clone(), Arc::clone(&vision)));
        Ok(vision)
    }

    /// The viewing country's tag at the hour on screen.
    pub fn viewing_tag(&self, vision: &perspective::Vision) -> io::Result<String> {
        let ours = vision.ours();
        Ok(tables::core::<row_tables::Countries>(self.frame)?
            .iter()
            .find(|country| country.canonical_slot == ours)
            .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
            .unwrap_or_else(|| format!("country slot {ours}")))
    }

    /// Every country tag present at the hour on screen, in tag order, for the
    /// window's viewing-country chooser.
    pub fn country_tags(&self) -> io::Result<Vec<String>> {
        let mut tags: Vec<String> = tables::core::<row_tables::Countries>(self.frame)?
            .iter()
            .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
            .collect();
        tags.sort();
        tags.dedup();
        Ok(tags)
    }

    // borders: the demilitarised-zone bucket bank
    /// This hour's demilitarised zone, for `map::demilitarized` and the border pass.
    ///
    /// The recorded set is the hour's own `StateFlags` joined to its `States`
    /// ([`crate::map::demilitarized::recorded_states`]). `extra` is the `--dump --demilitarized`
    /// override, empty everywhere else; it only ever *adds* to the recorded set, so an hour that
    /// records the flag is drawn from the recording whatever the command line says.
    pub fn demilitarized_zone(&self, regions: &Regions, extra: &BTreeSet<u32>) -> io::Result<Zone> {
        let mut states = demilitarized::recorded_states(self.frame)?;
        states.extend(extra.iter().copied());
        Ok(Zone::new(&self.map, regions, &states))
    }

    // names
    /// Places the country name layer.
    pub fn place_names(&self) -> io::Result<Placements> {
        name_pass::place(&self.placement_inputs(), self.frame)
    }

    /// What this hour's name placement reads, for callers that reuse
    /// placements across hours ([`name_pass::NameAreas`]).
    pub fn name_areas(&self) -> io::Result<name_pass::NameAreas> {
        name_pass::NameAreas::read(&self.map, &self.names.localisation, self.frame)
    }

    /// Places `areas`, reusing the names of areas `placed` already holds.
    pub fn place_name_areas(
        &self,
        areas: &name_pass::NameAreas,
        placed: &name_pass::PlacedAreas,
    ) -> Placements {
        name_pass::place_areas(&self.placement_inputs(), areas, Some(placed))
    }

    fn placement_inputs(&self) -> PlacementInputs<'_> {
        PlacementInputs {
            map: &self.map,
            heightmap: &self.assets.heightmap,
            content: &self.names,
            font: &self.icons.fonts.map,
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
        self.hour_tables_with_raids(perspective, Arc::new(self.raid_icons(ours)), engaged)
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
    /// diagnostics say so ([`raids::RaidIcons::carried`]).
    pub(crate) fn hour_tables_with_raids(
        &self,
        perspective: &perspective::Choice,
        raids: Arc<raids::RaidIcons>,
        engaged: Option<&crate::engaged_index::EngagedIndex>,
    ) -> io::Result<OverlayTables> {
        let etas = self.etas()?;
        // One `Vision` per hour and perspective: the counters and the limitation
        // lines read the same cached one.
        let vision = self.vision(perspective)?;
        let ours = vision.ours();
        let labels = labels::LabelTables::read(self.frame, ours)?;
        // Checked first: coverage that arrives during the build costs one
        // rebuild, never a stale flag.
        let engaged_covered =
            engaged.is_some_and(|index| index.covers(self.frame.branch, self.frame.hour));
        Ok(OverlayTables {
            engaged_covered,
            counters: counters::hour_tables(
                self.frame,
                &self.map,
                vision,
                &self.counters,
                &etas,
                self.movement.as_deref(),
                engaged,
                &self.type_tint_memo,
                &self.mission_memo,
                &self.icons.raids,
            )?,
            labels,
            raids: raids::RaidIcons::carried(raids, self.frame.branch, self.frame.hour, ours),
            built_for: (self.frame.branch, self.frame.hour, ours),
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
    /// tables come from [`Hour::hour_tables`]: the hour builders build them
    /// with the hour and the window reuses them for every camera.
    ///
    /// `view` is the map view's counter options and memory: the window's, a dashboard map's,
    /// or a fresh one for a dump.
    pub fn prepare_overlay_cached(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        tables: &OverlayTables,
        view: &counters::CounterView,
    ) -> io::Result<PreparedOverlay> {
        let lazy_version = self.icons.lazy.version();
        let frame = self.frame;
        let windows =
            self.icons
                .counters
                .windows_at(crate::map::native_shader::draw::gui_viewport(
                    viewport, ui_scale,
                ))?;
        let icons = counters::prepare_cached(
            &CounterInputs {
                content: &self.counters,
                equipment: Ok(self.unit_models.sub_units.equipment_content()),
                assets: &self.icons.counters,
                windows: &windows,
                fonts: &self.icons.fonts,
                localisation: &self.names.localisation,
                map: &self.map,
                camera,
                viewport,
                ui_scale,
                movement: self.movement.as_deref(),
                heights: self.heights(),
                settings: view.settings,
                selected: &view.selected,
                hovered: view.hovered,
                presentation: &view.presentation,
            },
            frame,
            &tables.counters,
        )?;
        // Raid icons sort above the labels and the counters by their layer.
        let native_raids = self.raid_owners(camera, viewport, ui_scale, &tables.raids);
        let raid_report = tables.raids.report.clone();
        // Labels set their layer before every push, so they lay out alike
        // after the raid icons or on their own.
        let labels = labels::prepare(
            &labels::LabelInputs {
                content: &self.labels,
                localisation: &self.names.localisation,
                map: &self.map,
                camera,
                viewport,
                ui_scale,
            },
            frame,
            &tables.labels,
        )?;
        let label_report = labels.report.clone();
        Ok(PreparedOverlay {
            lazy_version,
            engaged_covered: tables.engaged_covered,
            counter_view: view.clone(),
            labels,
            native_raids,
            raids: Batch::default(),
            raid_hits: Vec::new(),
            raid_range: None,
            icons,
            label_report,
            raid_report,
        })
    }

    fn raid_owners(
        &self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        raids: &raids::RaidIcons,
    ) -> raids::PreparedRaids {
        let anchor = |province| self.counters.anchor(province, counters::STANDSTILL);
        raids::prepare(
            &raids::RaidInputs {
                anchor: &anchor,
                camera,
                viewport,
                ui_scale,
            },
            raids,
        )
    }

    fn with_gui_context<'s, T>(
        &'s self,
        viewport: [i32; 2],
        time: Option<f32>,
        prepare: impl FnOnce(
            &counters::NativeWindows,
        ) -> io::Result<Option<counters::gui_windows::WindowFlags<'s>>>,
        draw: impl FnOnce(
            &crate::map::native_resources::Context<'_, counters::gui_windows::GuiAssets<'_>>,
        ) -> T,
    ) -> io::Result<T> {
        let windows = self
            .icons
            .counters
            .windows_at(viewport.map(|value| value as u32))?;
        let resources = counters::gui_windows::GuiAssets {
            gui: &self.icons.counters.gui,
            windows: &windows,
            localisation: &self.names.localisation,
            flags: prepare(&windows)?,
            physical_viewport: None,
        };
        Ok(draw(&crate::map::native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport,
            time,
        }))
    }

    /// Draws prepared raid windows for a frame or an offer-value refresh.
    fn raid_quads(
        &self,
        context: &crate::map::native_resources::Context<'_, counters::gui_windows::GuiAssets<'_>>,
        raids: &raids::PreparedRaids,
        view: &mut crate::map::icon_windows::PoolView,
    ) -> (Batch, Vec<raids::tooltips::Hit>, raids::RaidReport) {
        let mut batch = Batch::default();
        let mut hits = Vec::new();
        let mut report = raids.report.clone();
        raids.draw(
            context,
            self.icons.raids.priority,
            view,
            &mut batch,
            &mut hits,
            &mut report,
        );
        (batch, hits, report)
    }

    #[allow(clippy::too_many_arguments)]
    fn dump_overlay(
        self,
        camera: Camera,
        viewport: [u32; 2],
        ui_scale: f32,
        perspective: &perspective::Choice,
        omitted: crate::map::layers::Omissions,
        counter_settings: counters::CounterSettings,
    ) -> io::Result<DumpOverlay<'a>> {
        let tables = self.hour_tables(perspective, None)?;
        let view = counters::CounterView::new(counter_settings);
        let prepared = self.prepare_overlay_cached(camera, viewport, ui_scale, &tables, &view)?;
        Ok(DumpOverlay {
            hour: self,
            prepared,
            tables,
            camera,
            viewport,
            ui_scale,
            omitted,
            batch: Batch::default(),
            extra: Batch::default(),
            sprites: Batch::default(),
            labels: LabelReport::default(),
            counters: CounterReport::default(),
            raids: raids::RaidReport::default(),
            settlement_warnings: Vec::new(),
        })
    }

    // weather
    /// Builds this hour's `SnowMudData` grid from its recorded display bytes.
    pub fn snow_mud(&self) -> io::Result<SnowMud> {
        weather::grid_from_cells(self.snow_cells()?, self.frame)
    }

    /// The map's snow and mud cell composition ([`weather::SnowCells`]),
    /// built once for the content on first use.
    pub fn snow_cells(&self) -> io::Result<&weather::SnowCells> {
        self.content
            .snow_cells
            .get_or_init(|| {
                weather::SnowCells::new(&self.map, &self.assets.perm_snow)
                    .map_err(|error| (error.kind(), error.to_string()))
            })
            .as_ref()
            .map_err(|(kind, error)| io::Error::new(*kind, error.clone()))
    }

    // fog of war
    /// Builds this hour's `IntelMap` for one perspective.
    ///
    /// The province image and the definitions are static content, so the hour
    /// and the perspective are the only inputs that move; an hour whose
    /// [`perspective::Vision::province_fog`] is `None` gets the all-clear grid
    /// and a frame that draws no fog.
    pub fn intel_map(&self, perspective: &perspective::Choice) -> io::Result<IntelMap> {
        let vision = self.vision(perspective)?;
        textures::intel_map(&self.map, vision.province_fog())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Content>();
    shared::<replay::container::Replay>();
    shared::<replay::frame::DecodedFrame>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use replay::identity::Roster;

    /// Raid icons replaced in place in a drawn overlay equal the overlay
    /// drawn again with them, bit for bit, whatever the parts' sizes and
    /// omissions, over a run of replacements that grow, shrink and empty the
    /// raid icons (a converging cycle, a reset, a country switch, a skip).
    #[test]
    fn raid_icons_replaced_in_place_equal_the_overlay_drawn_again() {
        use crate::map::sprites::Quad;
        /// A small xorshift: the run is the same every time.
        struct Rng(u64);
        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 % n.max(1) as u64) as usize
            }
            fn quads(&mut self, group: usize, n: usize) -> Vec<Quad> {
                (0..n)
                    .map(|at| Quad {
                        texture: group,
                        rect: [at as f32, self.below(1000) as f32, 1.0, 1.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: [1.0, 1.0, 1.0, if self.below(2) == 0 { 1.0 } else { -0.0 }],
                        rotation: 0.0,
                        layer: self.below(1 << 20) as u64,
                        shade: 0.0,
                        exact: None,
                    })
                    .collect()
            }
        }
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for omitted in [
            crate::map::layers::Omissions::default(),
            crate::map::layers::Omissions {
                counters: true,
                ..Default::default()
            },
            crate::map::layers::Omissions {
                labels: true,
                ..Default::default()
            },
        ] {
            let fixed: Vec<Vec<Quad>> = (0..6)
                .map(|group| {
                    let n = rng.below(40);
                    rng.quads(group, n)
                })
                .collect();
            fn parts<'a>(fixed: &'a [Vec<Quad>], raids: &'a [Quad]) -> OverlayParts<'a> {
                OverlayParts {
                    pings: &fixed[0],
                    bases: &fixed[1],
                    raids,
                    labels: &fixed[2],
                    counters: &fixed[3],
                    combats: &fixed[4],
                    naval_combats: &fixed[5],
                }
            }
            let mut drawn = Batch::default();
            let first = rng.quads(9, 3);
            let mut range = compose(&mut drawn, &parts(&fixed, &first), omitted);
            for step in 0..40 {
                let n = match step % 10 {
                    0 => 0,
                    9 => 200,
                    _ => rng.below(120),
                };
                let raids = rng.quads(9, n);
                if let Some(at) = range.clone() {
                    range = Some(splice_raids(&mut drawn, at, &raids));
                }
                let mut again = Batch::default();
                let expected = compose(&mut again, &parts(&fixed, &raids), omitted);
                assert!(
                    same_quads(&drawn.quads, &again.quads),
                    "step {step}, omitted {omitted:?}"
                );
                assert_eq!(range, expected);
            }
        }
    }
    use replay::identity::Verification;

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
        engine::executable_tables::initialize_for_tests()
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
            let document = Document::load_shared(
                Path::new(path),
                &options,
                false,
                &InitialHour {
                    branch: Some(branch),
                    hour: Some(hour),
                    hour_fraction: 0.0,
                },
                replay,
                None,
                0,
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
