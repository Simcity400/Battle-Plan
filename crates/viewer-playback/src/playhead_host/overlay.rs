//! The open map's overlay, made by a maker of its own: a worker that owns the overlay's rows
//! reader, builds the tables of the hour the step asked for and lays the overlay out at the view
//! the frame posted, then publishes both.
//!
//! The step asks at its apply ([`Overlay::ask_step`]), so the rows, the tables and the layout are
//! made while its layers settle, and takes what the maker published at its join
//! ([`Overlay::join`]): the tables are given to the counter layers before they settle and the
//! layout is handed to the frame in its window ([`Made`]). The frame places it at its camera
//! ([`crate::document::PreparedOverlay::project`]) and draws it; it lays nothing out.
//!
//! A build begun is always finished and published, only the newest ask is ever taken, and the
//! maker lays nothing out while the overlay it published is untaken (the layout is on the counters'
//! memory the frame swaps in). So the overlay shown only moves forward, each build is of the newest
//! hour asked, and the overlay stands at most one build behind the layers.
//!
//! The step waits for the overlay of its own hour only while the maker's last cycle fits the frame
//! it is prepared for ([`waits`]). Past that it goes on with the overlay it has and the build it
//! asked for joins a later step, so at the top speeds the step pays nothing for it. A saved frame
//! ([`Settling::Whole`]) asks for the rows of exactly its hour and waits however long they take.

use super::{
    lock,
    views::Need as RowNeed,
    side::{LayInKeys, LayIns},
    timeline::{self, Mark},
    views,
};
use crate::{
    document::{Content, LeftToLayers, OverlayRows, OverlayTables, PreparedOverlay},
    layers::{Settling, counter_kinds},
    map_views::{CounterView, Presentation},
};
use replay_data::{BranchId, Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use viewer_counters::map::counters::{Kinds, Layered, Need, Shown};
use viewer_foundation::{
    engagement_history::History,
    map::counter_move::MovedHours,
    map::{
        camera::Camera,
        perspective::{Choice, Vision},
    },
};

/// How long a whole settle waits for the overlay's rows and for the vision it is made over: a
/// saved frame, which shows exactly its hour or is refused by name.
const WHOLE: Duration = Duration::from_secs(120);

/// How long the maker waits for the rows of the hour asked before looking at what it was asked
/// again. The step's ask and its join both wake it, so this only bounds how long it waits on rows
/// that do not come: a quarter of a frame of a 120 Hz display.
const LOOK: Duration = Duration::from_millis(2);


/// The tables **only** the unit counters and the parts laid out with them read: their plates, the
/// orders' groups, the railway guns, the movement operands. Asked for only while the view can draw
/// a counter ([`Shown::units`]).
///
/// `Divisions` and `DivisionMovement` are not among them: the vision reads both at every view
/// ([`viewer_foundation::map::perspective::VISION_TABLES`]), so every view's rows hold them and a
/// family cannot claim them.
const UNIT_COUNTERS: &[TableId] = &[
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::DivisionModelInputs,
    TableId::DivisionTemplateIcons,
    TableId::DivisionTransportSpeeds,
    TableId::ArmyIconInputs,
    TableId::ActiveAbilities,
    TableId::UnitLeaders,
    TableId::LeaderPortraits,
    TableId::LeaderPortraitPaths,
    TableId::CharacterPortraits,
    TableId::OrdersGroups,
    TableId::OrdersGroupPlans,
    TableId::OrdersGroupCounterInputs,
    TableId::ArmyOrderPlans,
    TableId::ArmyOrders,
    TableId::RailCooldowns,
    TableId::RailwayGunCounterInputs,
    TableId::ProvinceUnitOrder,
    TableId::CountryUnitSpeedEntries,
    TableId::MovementStateTerms,
    TableId::CoreSupplyAccess,
    TableId::RailEdges,
    TableId::RiverRelations,
];

/// The tables only the land combat markers read ([`Shown::land_combats`]).
const LAND_COMBATS: &[TableId] = &[TableId::LandCombatDivisions, TableId::LandCombatSides, TableId::LandCombatDetails];

/// The tables only the naval mission icons read ([`Shown::naval_missions`]): the ledgers and the
/// weather their day and night is read from.
const NAVAL_MISSIONS: &[TableId] = &[
    TableId::NavalMissionLedgers,
    TableId::NavalMissionTaskForces,
    TableId::NavalMissionCommanders,
    TableId::NavalMissionCountries,
    TableId::NavalDominanceTaskForceRanges,
    TableId::SliceReads,
    TableId::ModifierDefinitions,
    TableId::WeatherProvinces,
];

/// The tables only the base, headquarters, facility and strategic-location icons and their port
/// counters read ([`Shown::bases`]).
const BASE_ICONS: &[TableId] = &[
    TableId::AirBases,
    TableId::AirBaseRegistrations,
    TableId::AirWings,
    TableId::AirReferenceWitnesses,
    TableId::AirMissionInputs,
    TableId::AirGroundCrews,
    TableId::AirRegionCombat,
    TableId::AirCountryRelations,
    TableId::WingEquipment,
    TableId::FacilityMapFlags,
    TableId::AirSortieBaseCountries,
    TableId::AirSortieModifierMetadata,
    TableId::AirSortieModifiers,
    TableId::AirSortieSpecialBases,
];

/// The tables the open map's overlay is known to read at every view, besides the vision's
/// ([`viewer_foundation::map::perspective::VISION_TABLES`]) and the families' above: declared when
/// its reader opens, so its first rows hold them and its first build is made at once. A table a
/// script reads beyond them is learned.
const OVERLAY: &[TableId] = &[
    // The fleets, their missions, mines and naval combats and results. The land combats carry the
    // engaged flags every view's markers and layers read.
    TableId::LandCombats,
    TableId::AceRangeInputs,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::TaskForceCounterInputs,
    TableId::TaskForceMissions,
    TableId::Fleets,
    TableId::FleetCounterInputs,
    TableId::NavalBases,
    TableId::NavalCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    TableId::NavalCombatTransfers,
    TableId::NavalProvinceCombats,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::NavalTransferUnits,
    TableId::NavalAccidentReports,
    TableId::Capitulation,
    // The air wings' regional missions, the raids' sites and the volunteers.
    TableId::AirRegionalMissions,
    TableId::VariantDefinitions,
    TableId::RaidAirSites,
    TableId::RaidAirScores,
    TableId::RaidCountryInputs,
    TableId::VolunteerHosts,
    TableId::SentVolunteers,
    // The base windows, the faction pings and the radar.
    TableId::ProvinceBuildings,
    TableId::StateCores,
    TableId::StateFlags,
    TableId::FactionTheatres,
    TableId::WeatherRegions,
    TableId::RadarCoverage,
    // The countries every counter names, tints and flags, and the occupation icons.
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::CountryCounterInputs,
    TableId::FactionMembers,
    TableId::LexicalTokens,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::ReservedCountryNaming,
    TableId::Subjects,
    TableId::WarRelations,
    TableId::SimulationFeatures,
    TableId::Provinces,
    TableId::States,
    TableId::StateResistance,
    TableId::Autonomy,
    // The raid arrows.
    TableId::Raids,
    TableId::RaidTargets,
    TableId::RaidEquipment,
    TableId::RaidSpeedInputs,
    TableId::AvailableEquipmentVariants,
    TableId::CountryCommandPower,
    // The intel icons.
    TableId::IntelligenceAgencies,
    TableId::IntelNetworks,
    TableId::IntelNetworkAuxiliaries,
    TableId::Operations,
    TableId::OperativeMissions,
    TableId::StrategicOperatives,
    TableId::CharacterOperatives,
    TableId::AgencyOperationBuffers,
    TableId::RaidIntelInputs,
    TableId::CountryStabilityInputs,
    // What the default view's build was found to read beyond them in every paced run: the
    // characters it names, the saved targets its scripts read and the naval repairs.
    TableId::Characters,
    TableId::GlobalSavedEventTargets,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::ScriptVariables,
    TableId::TaskForceMapMotion,
];


/// The sets the maker's reader of the side's view store opens with: every column of the tables
/// [`overlay_tables`] names, which the side then holds in its mirrors.
fn overlay_needs() -> Vec<RowNeed> {
    overlay_tables().into_iter().map(RowNeed::every).collect()
}

/// The tables the open map's overlay reads at any view: [`OVERLAY`], every family's and the
/// vision's, ascending. What a tooltip, a panel or a click asks of the rows at the head is this
/// set, whatever the camera ([`crate::app::head_rows`]).
pub fn overlay_tables() -> Vec<TableId> {
    tables_shown(Shown::EVERY)
}

/// The tables the overlay reads at a view that can draw `shown`: a family's tables are not asked
/// for while nothing can draw it, and a table the maker reads beyond this set is learned
/// ([`Maker::learn`]), so a family grouped wrongly costs one publication and shows in the run's
/// `view_store.learned`.
fn tables_shown(shown: Shown) -> Vec<TableId> {
    let family = |held: bool, tables: &'static [TableId]| held.then_some(tables).unwrap_or_default();
    let mut tables: Vec<TableId> = OVERLAY
        .iter()
        .chain(viewer_foundation::map::perspective::VISION_TABLES)
        .chain(family(shown.units, UNIT_COUNTERS))
        .chain(family(shown.land_combats, LAND_COMBATS))
        .chain(family(shown.naval_missions, NAVAL_MISSIONS))
        .chain(family(shown.bases, BASE_ICONS))
        .copied()
        .collect();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// What only the frame knows of the overlay, posted with the view and taken by the side at its
/// next step: where the frame draws, how big, at what interface scale, its counter options,
/// selection and hover, and the recording's air rosters.
#[derive(Clone)]
pub struct Ask {
    pub camera: Camera,
    /// Where a zoom is sending the camera, which it eases towards: a family the target reaches is
    /// made during that ease, so nothing is missing when the zoom lands.
    pub target: Option<f32>,
    pub pixels: [u32; 2],
    pub ui_scale: f32,
    pub view: CounterView,
    pub rosters: Arc<crate::playback::AirRosters>,
}

impl Ask {
    /// What the overlay's tables need of the view: by key unless the eye is free, which lays every
    /// icon out in the world, and only the icon families this camera and map mode can draw.
    fn need(&self, defines: &viewer_counters::map::counters::CounterDefines) -> Need {
        let height = self.camera.free().is_none().then_some(self.camera.height);
        Need { air_missions: false, by_key: self.camera.free().is_none(), shown: Shown::at(height, self.target, &self.view.visibility, defines) }
    }
}

/// The overlay of one step, handed to the frame in its window: laid out for its hour, placed at
/// the camera the frame asked with, on a counters' memory the frame swaps in.
pub struct Made {
    /// The hour it stands at: the hour of the rows it was made from, and then every later hour no
    /// step changed a table of those rows by.
    pub hour: Hour,
    pub branch: BranchId,
    pub prepared: PreparedOverlay,
    pub tables: Arc<OverlayTables>,
    /// The rows at the head it was made from, which the frame's own reads of the overlay's hour
    /// take ([`crate::app::Viewer::check_overlay`]).
    pub rows: Arc<DecodedFrame>,
    pub placed_at: Camera,
    /// The counters' memory it was laid out on, which the frame's view takes.
    pub on: Arc<Presentation>,
    /// The counter layers' products it was laid out from, and the kinds those layers draw: the
    /// frame's memory keeps them while this layout is shown, so the counters drawn are the ones
    /// its layout placed and the kinds it left out are the ones the layers drew.
    pub published: Layered,
    pub draws: Kinds,
    /// The view it was laid out for, as the frame posted it.
    pub view: CounterView,
    pub left: LeftToLayers,
    /// The map's layers' products laid into it, which the frame's gate lays into the layout it
    /// compares this one with: a layout made with another product is another layout.
    pub lay_ins: Arc<LayIns>,
    /// The hours of its publication the recording moved a mover at: the frame places the layout at
    /// its camera ([`crate::document::PreparedOverlay::project`]) and gives every mover a place
    /// for each of them there.
    pub moved: Arc<MovedHours>,
}

/// What the overlay's tables were made from: the rows at the head (their branch, hour and
/// allocation), the perspective and the vision the vision layer shows, the engaged index's cover,
/// the scripts layer's raid icons, and what the layers draw instead.
#[derive(Clone, Debug, PartialEq)]
struct Tabled {
    branch: BranchId,
    hour: Hour,
    rows: usize,
    perspective: Choice,
    vision: usize,
    engaged: bool,
    left_out: Kinds,
    labels: bool,
    need: Need,
    raids: Option<usize>,
}

/// What a layout stands for. Where the camera stands is not one of them: a layout made for this
/// key is placed at any camera ([`crate::document::PreparedOverlay::project`]).
#[derive(Clone, Debug, PartialEq)]
struct Laid {
    /// The tables it was laid out over, by the count of tables the maker has made: they are built
    /// in place, so their address says nothing.
    tables: u64,
    view: CounterView,
    pixels: [u32; 2],
    ui_scale: u32,
    /// The counter layers' products it was laid out with (their addresses, zero for a kind
    /// today's tables draw).
    layered: [usize; 6],
    /// The map's layers' products laid into it.
    lay_ins: LayInKeys,
    left: LeftToLayers,
    /// The lazily decoded art it was laid out over: a flag or portrait that arrives lays again.
    art: u64,
}

/// The ask the maker takes next: the newest it has been given, and nothing at all while the build
/// it published is untaken, so a build is never overwritten before the step has had it. One ask is
/// held at a time, so every build is of the newest hour asked and the overlay stands at most one
/// build behind the steps.
fn next_ask<T>(asked: &mut Option<T>, published: bool) -> Option<T> {
    if published { None } else { asked.take() }
}

/// Whether the overlay a build published is the one the step shows: this picture's (`generation`),
/// and newer than the one the step holds (`taken`). So the overlay shown only moves forward, and a
/// build in flight across a reset is never shown.
fn shows(built: (u64, u64), generation: u64, taken: u64) -> bool {
    built.0 == generation && built.1 > taken
}

/// Whether a layout the maker published is installed: it is this picture's (`generation`), and
/// never of an hour before the one the overlay the frame holds stands at, so the overlay only moves
/// forward. A reset clears that hour ([`Overlay::reset`]), so the first layout of the history after
/// a seek is installed however far back it stands.
fn installs(laid: (u64, Hour), generation: u64, held: Option<Hour>) -> bool {
    laid.0 == generation && held.is_none_or(|held| held <= laid.1)
}

/// Whether the join waits on for what the maker has not published yet. A step never waits: it
/// ends when its layers and their makers are done, and the counter layers settle over the tables
/// it has, taking the hour's at the step the maker publishes them to. Only a whole settle waits
/// (`whole`), and it waits for the ask it posted to be answered (`answered`) or for the wait to
/// run out (`out`). The overlay standing at the hour is not a reason to stop: a saved frame asks
/// for its own hour again, over the art that arrived while it drew
/// ([`crate::playhead_host::MapSide::lay_overlay_again`]), and the overlay has stood at that hour
/// since its first layout, so a join that stopped there took no layout at all.
fn joins_on(whole: bool, answered: bool, out: bool) -> bool {
    whole && !answered && !out
}

/// Whether the overlay the frame draws stands at `head`: a layout was published for that hour or a
/// later one (`hour`), and no table its tables read is still missing. The layout the maker has
/// published and no frame has taken is not what this asks about. An hour the overlay reads nothing
/// new in publishes its hour and no layout at all, because the layout the frame already holds
/// stands at that hour too, so a settle that waited for a layout to take would wait for one nobody
/// is going to make.
fn stands(hour: Option<Hour>, missed: usize, head: Hour) -> bool {
    hour.is_some_and(|hour| hour >= head) && missed == 0
}

/// What a maker waiting for the rows of its ask does, its hour not reached yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waits {
    /// Wait for rows newer than the ones it has.
    Rows,
    /// Build from the rows it has: a later ask takes the hour this one waited for.
    Build,
    /// Give this ask up: a newer one stands, or the maker is stopping.
    Give,
}

/// What an ask whose rows have not reached its hour does. A newer ask, or the maker stopping
/// (`superseded`), ends it wherever it waits: with the rows it has when they are newer than the
/// tables it holds, and by name otherwise — a seek back leaves rows whose hour this ask will never
/// be reached from, and a wait that went on would take neither the new picture's ask nor the stop
/// a drop posts. A saved frame's ask is of exactly its hour, so a newer ask ends it by name.
fn waits(exact: bool, superseded: bool, rows: Option<Hour>, tabled: Option<Hour>) -> Waits {
    if !superseded {
        return Waits::Rows;
    }
    match rows {
        Some(rows) if !exact && tabled.is_none_or(|tabled| tabled < rows) => Waits::Build,
        _ => Waits::Give,
    }
}

/// Whether the step asks for a layout. A layout is laid out on the counters' memory the frame
/// swaps in with it, so the maker lays the next one out only once the frame has taken the one it
/// published (`untaken`) and handed the other memory back. A whole settle is exempt: it lays out
/// on the view's own memory, and the frame it settles for waits for it and nothing else.
fn lays(lay: bool, whole: bool, untaken: bool) -> bool {
    lay && (whole || !untaken)
}

/// `started`'s elapsed microseconds under `name`, for the frame the step is prepared for.
fn note_us(name: &'static str, started: Instant) {
    viewer_foundation::map::redraws::note(name, u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
}

/// What one step asks the maker to make, posted as one latest-wins message ([`post`]): the hour
/// applied, its moves, and `asking`, what the tables and the layout read, reached through it.
struct Request<P> {
    /// Bumped by every reset: nothing built for the picture before is published as this one's.
    generation: u64,
    /// Bumped by every ask: the maker publishes the stamp it answered, so a whole settle tells its
    /// own answer from the one before it.
    stamp: u64,
    hour: Hour,
    /// The hours of this step at which the recording moved the movers the drawn layers place, so
    /// the layout gives each a place for each of them ([`crate::layers::Layer::moved_at`]).
    moved: Arc<MovedHours>,
    /// A saved frame's ask: the rows of exactly this hour, every table its reads learned, waited
    /// for however long they take and refused by name.
    whole: bool,
    /// Whether the overlay is laid out over the tables this ask makes. A whole settle asks for the
    /// tables alone first, so the counter layers settle over them before the overlay is laid out:
    /// the layout of an hour is made once, as a step's is.
    lay: bool,
    asking: P,
}

impl<P> std::ops::Deref for Request<P> {
    type Target = P;
    fn deref(&self) -> &P {
        &self.asking
    }
}

type Wanted = Request<Asking>;

/// What only the frame knows of the overlay and what the map's layers hold for it.
struct Asking {
    ask: Ask,
    perspective: Choice,
    /// The vision the map's layers show, as the step before copied it out.
    vision: Arc<Vision>,
    /// The engaged index the combat markers read, asked at the build: the tables are made over the
    /// hours it covers then, not the hours it covered when the step asked.
    engaged: Option<Arc<History>>,
    labels: bool,
    lay_ins: Arc<LayIns>,
    keys: LayInKeys,
    published: Layered,
    draws: Kinds,
    left: LeftToLayers,
}

/// `want` posted as the newest request. A request the maker never took is replaced, its movement
/// carried into this one: the layout that takes this one places every move both recorded.
fn post<P>(slot: &mut Option<Request<P>>, mut want: Request<P>) {
    if let Some(old) = slot.take() {
        let mut carried = Carried { generation: old.generation, moved: old.moved };
        carried.join(want.generation, &want.moved);
        want.moved = carried.moved;
    }
    *slot = Some(want);
}

/// The picture the maker answers and the moves of its requests no layout has drawn yet: a request
/// given up, refused or answered with tables alone leaves its moves to the layout that follows.
#[derive(Default)]
struct Carried {
    generation: u64,
    moved: Arc<MovedHours>,
}

impl Carried {
    /// `moved` of `generation` added after the moves held; a new picture drops the old one's.
    /// Whether the picture changed.
    fn join(&mut self, generation: u64, moved: &MovedHours) -> bool {
        let fresh = generation != self.generation;
        if fresh {
            *self = Self { generation, moved: Arc::default() };
        }
        // Appended in place once no request holds the moves carried: a long run of refused
        // layouts does not copy every move held at every request.
        if moved.counts().0 > 0 {
            Arc::make_mut(&mut self.moved).extend(moved);
        }
        fresh
    }

    /// The maker takes `want`: its moves joined to the picture's, and its layout reads them all.
    /// Whether the picture changed.
    fn take<P>(&mut self, want: &mut Request<P>) -> bool {
        let fresh = self.join(want.generation, &want.moved);
        want.moved = Arc::clone(&self.moved);
        fresh
    }

    /// A request answered: a layout made over rows at `stands` retires the moves up to that hour
    /// and keeps the later ones for the next; anything else retires none.
    fn ended<L>(&mut self, made: &Result<(Hour, Option<L>), Gave>) {
        if let Ok((stands, Some(_))) = made {
            self.moved = Arc::new(self.moved.after(*stands));
        }
    }
}

/// The ask of a layout no frame has taken yet ([`lays`]): its counters' memory is not free.
#[derive(Default)]
struct Owed(Option<u64>);

impl Owed {
    /// Whether the step's ask lays out ([`lays`]). A layout asked for and not yet taken holds the
    /// counters' memory the next one would be laid out on: being made, published and not
    /// installed, or installed and not taken (`made`). The ask is let go once the maker answered
    /// it with no layout pending: `held` reads the newest answer and whether a published layout
    /// is uninstalled.
    fn lays(&mut self, lay: bool, whole: bool, made: bool, held: impl FnOnce() -> (u64, bool)) -> bool {
        if let Some(owed) = self.0 {
            let (answered, published) = held();
            if answered >= owed && !published && !made {
                self.0 = None;
            }
        }
        lays(lay, whole, made || self.0.is_some())
    }

    fn asked(&mut self, lay: bool, whole: bool, stamp: u64) {
        if lay && !whole {
            self.0 = Some(stamp);
        }
    }
}

/// Whether the ask a join posted (`stamp`) failed to make what it asked for, or a strict settle's
/// ask was not answered at all: what the overlay holds is then another ask's, however its hour
/// reads.
fn fails(strict: bool, answered: u64, stamp: u64, failed: bool) -> bool {
    (answered >= stamp && failed) || (strict && answered < stamp)
}

/// What this step asked the maker for, which its join waits for.
#[derive(Clone, Copy)]
struct Asked {
    stamp: u64,
    hour: Hour,
    /// The ask was for a layout, so the join is answered only by one ([`Wanted::lay`]).
    lay: bool,
}

/// What the maker published, taken by the step at its join.
struct Built {
    /// The newest ask it answers.
    stamp: u64,
    generation: u64,
    /// The hour the tables stand at.
    hour: Hour,
    tables: Arc<OverlayTables>,
    lay_ins: Arc<LayIns>,
}

/// The layout the maker published, taken by the first step that joins after it: a step never waits
/// for one ([`Overlay::join`]).
struct LaidOut {
    generation: u64,
    /// The hour the layout stands at.
    hour: Hour,
    /// The layout, when anything it stands for changed; none leaves the frame the one it holds,
    /// which was laid out over tables that stand at this hour too.
    made: Option<Made>,
}

/// What the step and the maker share: the ask, what was built from it, and what the maker is doing.
#[derive(Default)]
struct Held {
    stop: bool,
    want: Option<Wanted>,
    /// The tables of the hour asked, published as soon as they are made: the counter layers settle
    /// over them, so the step's join waits for these and for nothing else.
    built: Option<Built>,
    /// The layout, published when it is done: a step installs it if it is there and never waits.
    laid_out: Option<LaidOut>,
    /// The newest ask it has answered, and whether it failed to make what it asked for: tables
    /// refused, or a layout asked for and not made.
    answered: u64,
    failed: bool,
    loading: Vec<String>,
    missed: Vec<TableId>,
}

struct Shared {
    held: Mutex<Held>,
    /// The maker published, or the step asked.
    signal: Condvar,
}

/// The counter kinds' first layer in `names`: the step gives the overlay's tables to the counter
/// layers just before it settles them, with the vision and engagements layers (which the tables
/// read) settled before it. None for a map drawing no counter kind.
fn counters_at(names: &[&'static str]) -> Option<usize> {
    names.iter().position(|name| counter_kinds::LAYERS.iter().any(|(held, _)| held == name))
}

/// What one step lends the maker: the playhead, the layers as they stand at the apply, the hour
/// applied and what the side knows of its layers.
pub struct Lend<'a> {
    /// The side's view store, which makes the maker's rows from the side's own mirrors.
    pub views: &'a views::Store,
    pub applied: Option<Hour>,
    pub branch: Option<BranchId>,
    pub perspective: &'a Choice,
    /// The vision the map's layers show, as a step copied it out; why there is none, by name.
    pub vision: Result<Arc<Vision>, String>,
    /// The engaged index the combat markers read.
    pub engaged: Option<Arc<History>>,
    pub draws_labels: bool,
    pub lay_ins: Arc<LayIns>,
    pub keys: LayInKeys,
    pub published: Layered,
    pub draws: Kinds,
    pub left: LeftToLayers,
    /// The hours of this step the recording moved the movers at ([`Wanted::moved`]).
    pub moved: Arc<MovedHours>,
    /// A saved frame's: the rows of exactly this hour, waited for however long they take.
    pub whole: bool,
    /// Whether the overlay is laid out over the tables asked for ([`Wanted::lay`]).
    pub lay: bool,
}

/// The open map's overlay as the side's step holds it: the maker's worker, the ask posted to it and
/// what it published.
pub struct Overlay {
    content: Arc<Content>,
    /// The counter kinds' first layer: the tables are given just before it settles.
    counters_at: Option<usize>,
    /// The frame's latest ask; nothing is made before its first.
    ask: Option<Ask>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    /// Bumped by every reset, so a build in flight across one is never shown.
    generation: u64,
    stamp: u64,
    /// The ask this step posted: what its join waits for.
    asked: Option<Asked>,
    /// The tables the step gave the counter layers, as the maker published them, and the hour they
    /// stand at.
    tables: Option<Arc<OverlayTables>>,
    stands: Option<Hour>,
    taken: u64,
    /// The overlay made, until the frame takes it in its window.
    made: Option<Made>,
    lay_owed: Owed,
    /// The hour of the rows the overlay the frame holds was made from.
    hour: Option<Hour>,
    lay_ins: Arc<LayIns>,
    loading: Vec<String>,
    missed: Vec<TableId>,
    maker: timeline::Maker,
}

impl Overlay {
    pub fn new(content: Arc<Content>) -> Self {
        Self {
            content,
            counters_at: None,
            ask: None,
            shared: Arc::new(Shared { held: Mutex::new(Held::default()), signal: Condvar::new() }),
            worker: None,
            generation: 0,
            stamp: 0,
            asked: None,
            tables: None,
            stands: None,
            taken: 0,
            made: None,
            lay_owed: Owed::default(),
            hour: None,
            lay_ins: Arc::default(),
            loading: Vec::new(),
            missed: Vec::new(),
            maker: timeline::maker("overlay"),
        }
    }

    /// [`counters_at`] for this side's layers, found once.
    pub fn counters_at(&mut self, names: &[&'static str]) -> Option<usize> {
        if self.counters_at.is_none() {
            self.counters_at = counters_at(names);
        }
        self.counters_at
    }

    /// The frame's newest ask, taken at the next step.
    pub fn ask(&mut self, ask: Ask) {
        self.ask = Some(ask);
    }

    /// A reset: the picture changes, so a build in flight for the one before is never published as
    /// this one's and what was built for it is dropped.
    pub fn reset(&mut self) {
        self.generation += 1;
        let mut held = lock(&self.shared.held);
        held.want = None;
        held.built = None;
        held.laid_out = None;
        drop(held);
        self.shared.signal.notify_all();
        (self.tables, self.stands, self.asked, self.lay_owed) = (None, None, None, Owed::default());
        // The overlay of the picture before this one is not the overlay of this one, at any hour:
        // the layout the frame holds is replaced by the first of this picture, which may stand at
        // an earlier hour (a seek back).
        (self.made, self.hour) = (None, None);
    }

    /// The overlay made for a step; the frame takes it in its window and holds it from then on.
    /// What the maker published since the last step joined is installed here, so the frame takes
    /// the newest layout there is and the maker is free to lay the next one out at once.
    pub fn take(&mut self) -> Option<Made> {
        let laid = lock(&self.shared.held).laid_out.take();
        if let Some(laid) = laid {
            self.install_layout(laid);
        }
        let made = self.made.take();
        if let Some(made) = &made {
            self.hour = Some(made.hour);
            self.lay_owed = Owed::default();
        }
        made
    }

    /// The hour of the rows the overlay on screen was made from; none before its first.
    pub fn hour(&self) -> Option<Hour> {
        self.hour
    }

    /// The map's layers' products the last layout laid in.
    pub fn lay_ins(&self) -> Arc<LayIns> {
        Arc::clone(&self.lay_ins)
    }

    /// The overlay's refusals and what its tables are loading, by name.
    pub fn loading(&self) -> &[String] {
        &self.loading
    }

    /// The tables the layout read that its rows do not hold: while any is named, the overlay does
    /// not stand at its hour.
    pub fn missed(&self) -> &[TableId] {
        &self.missed
    }

    /// The tables of the step's hour, as the maker published them; none before its first build.
    pub fn tables(&self) -> Option<&Arc<OverlayTables>> {
        self.tables.as_ref()
    }

    /// The recording's air rosters, which the counter layers are given with the tables.
    pub fn rosters(&self) -> Option<Arc<crate::playback::AirRosters>> {
        self.ask.as_ref().map(|ask| Arc::clone(&ask.rosters))
    }

    /// Whether the overlay made stands at `hour`: a stepped frame's gate. A layout standing past
    /// the hour shows it, since the hour it stands at is every later hour no step changed a table
    /// of its rows by.
    pub fn stands_at(&self, hour: Hour) -> bool {
        stands(self.hour, self.missed.len(), hour)
    }

    /// The step asks the maker for the overlay of its hour at its apply, so the rows, the tables
    /// and the layout are made while its layers settle. Whether anything was asked for.
    pub fn ask_step(&mut self, lend: Lend<'_>) -> bool {
        let Lend { views, applied, branch, perspective, vision, engaged, draws_labels, lay_ins, keys, published, draws, left, moved, whole, lay } = lend;
        self.loading.clear();
        // The overlay published is taken before this step asks, so the maker is free to begin the
        // step's own hour at its apply instead of waiting for the join to take the one before.
        let built = lock(&self.shared.held).built.take();
        if let Some(built) = built {
            self.install(built);
        }
        if !lay {
            // A caught-up step lays nothing out, and no frame will take a layout made before it.
            self.made = None;
        }
        let shared = &self.shared;
        let lay = self.lay_owed.lays(lay, whole, self.made.is_some(), || {
            let held = lock(&shared.held);
            (held.answered, held.laid_out.is_some())
        });
        let (Some(ask), Some(hour), true) = (self.ask.clone(), applied, branch.is_some()) else { return false };
        let vision = match vision {
            Ok(vision) => vision,
            Err(why) => {
                self.loading.push(format!("the overlay's vision: {why}"));
                return false;
            }
        };
        self.start(views);
        if self.worker.is_none() {
            return false;
        }
        self.stamp += 1;
        let want = Wanted {
            generation: self.generation,
            stamp: self.stamp,
            hour,
            moved,
            whole,
            lay,
            asking: Asking { ask, perspective: perspective.clone(), vision, engaged, labels: draws_labels, lay_ins, keys, published, draws, left },
        };
        post(&mut lock(&self.shared.held).want, want);
        self.shared.signal.notify_all();
        self.lay_owed.asked(lay, whole, self.stamp);
        self.asked = Some(Asked { stamp: self.stamp, hour, lay });
        timeline::mark(self.maker, Mark::Asked);
        true
    }

    /// The maker's worker, started with the first step that asks it. The overlay's tables and
    /// layout read the recording's executable tables, so they are in scope on it throughout.
    fn start(&mut self, store: &views::Store) {
        if self.worker.is_some() {
            return;
        }
        let tables = self.content.tables;
        let maker = Maker {
            content: Arc::clone(&self.content),
            shared: Arc::clone(&self.shared),
            rows: store.reader("overlay", &overlay_needs()),
            declared: overlay_tables(),
            learned: Vec::new(),
            shown: Shown::EVERY,
            tables: None,
            tabled: None,
            tabled_count: 0,
            laying: None,
            laid: None,
            left_out: Kinds::default(),
            lay_ins: Arc::default(),
            lay_in_keys: LayInKeys::default(),
            carried: Carried::default(),
            marks: self.maker,
        };
        let started = std::thread::Builder::new().name("map overlay".into()).spawn(move || {
            // A step's join takes its product: at the head's priority, never below normal.
            viewer_foundation::worker_priority::enter(viewer_foundation::worker_priority::Class::Kept);
            if tables.is_some() {
                engine_executable::executable_tables::enter(tables);
            }
            let mut maker = maker;
            maker.work();
        });
        match started {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => self.loading.push(format!("the overlay's maker cannot start: {error}")),
        }
    }

    /// The overlay of the step joined: what the maker published is taken, and nothing is waited
    /// for. A step ends when its layers and their makers are done, so the counter layers settle
    /// over the tables the step has — a step behind, when this step's are not published yet — and
    /// take the hour's tables at the step that joins them. A whole settle waits for the overlay of
    /// exactly its hour, however long it takes.
    ///
    /// Whether the overlay taken stands at the step's hour.
    pub fn join(&mut self, settling: Settling) -> bool {
        let Some(Asked { stamp, hour, lay }) = self.asked else { return false };
        let began = Instant::now();
        timeline::mark(self.maker, Mark::JoinBegan);
        let until = began + WHOLE;
        let shared = Arc::clone(&self.shared);
        let mut held = lock(&shared.held);
        loop {
            if let Some(built) = held.built.take() {
                self.install(built);
            }
            // The layout is taken whenever the maker has it: this step's, or the one it published
            // while an earlier step drew. Nothing waits for it.
            if let Some(laid) = held.laid_out.take() {
                self.install_layout(laid);
            }
            let now = Instant::now();
            // A whole settle that has been answered asks again, which is how a table the tables
            // read but the rows did not hold is learned.
            if held.stop || !joins_on(settling.strict(), held.answered >= stamp, now >= until) {
                break;
            }
            held = shared.signal.wait_timeout(held, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
        timeline::mark(self.maker, Mark::Waited);
        let failed = fails(settling.strict(), held.answered, stamp, held.failed);
        self.loading.extend(held.loading.iter().cloned());
        if self.missed != held.missed {
            self.missed.clone_from(&held.missed);
        }
        drop(held);
        shared.signal.notify_all();
        timeline::mark(self.maker, Mark::Joined);
        self.record_join(began, hour);
        // An ask for a layout is answered by one: a whole settle asks again until the overlay the
        // frame will take stands at the hour too.
        !failed && self.stands.is_some_and(|stands| stands >= hour) && self.missed.is_empty() && (!lay || self.stands_at(hour))
    }

    /// The flight record's overlay: the joins a step made, those that waited at all, and those
    /// that did not reach the step's hour.
    fn record_join(&self, began: Instant, hour: Hour) {
        if !viewer_foundation::view_record::on() {
            return;
        }
        let at = self.stands.is_some_and(|stands| stands >= hour);
        viewer_foundation::view_record::add("overlay", "joins", 1);
        viewer_foundation::view_record::add("overlay", "join_us", began.elapsed().as_micros() as u64);
        viewer_foundation::view_record::add("overlay", if at { "joins_at_the_hour" } else { "joins_behind" }, 1);
        if let Some(behind) = self.hour.map(|held| hour.saturating_sub(held)).filter(|&behind| behind > 0) {
            viewer_foundation::view_record::set("overlay", "hours_behind", u64::try_from(behind).unwrap_or(0));
        }
    }

    /// What the maker published, taken: its tables are the ones the counter layers are given, and
    /// its layout is the one the frame takes in its window.
    fn install(&mut self, built: Built) {
        if !shows((built.generation, built.stamp), self.generation, self.taken) {
            return;
        }
        self.taken = built.stamp;
        self.tables = Some(built.tables);
        self.stands = Some(built.hour);
        self.lay_ins = built.lay_ins;
    }

    /// The layout the maker published, taken: the frame draws it in its window. Never waited for,
    /// so the one taken may be of an earlier hour than the tables just installed.
    fn install_layout(&mut self, laid: LaidOut) {
        if !installs((laid.generation, laid.hour), self.generation, self.hour) {
            return;
        }
        // The overlay stands at the hour whether or not a layout came with it: nothing laid out
        // again means the layout the frame holds was made over tables that stand at this hour too.
        self.hour = Some(laid.hour);
        if let Some(made) = laid.made {
            self.made = Some(made);
        }
    }

    /// The step is over: its ask is answered or passed over, and the next step asks afresh.
    pub fn stepped(&mut self) {
        self.asked = None;
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        lock(&self.shared.held).stop = true;
        self.shared.signal.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Why the maker gave up on the ask it took.
enum Gave {
    /// A newer ask: the next build is of the newest hour asked.
    Superseded,
    /// Refused, by name.
    Refused(String),
}

/// The maker on its own thread: the overlay's rows reader, the tables it built and the layout it
/// laid out, all of which only it touches.
struct Maker {
    content: Arc<Content>,
    shared: Arc<Shared>,
    /// The overlay's rows, made by the side's own view store from its mirrors inside the step.
    rows: views::Reader,
    /// Every table it has declared to that reader: a declaration names the whole list, so a table
    /// learned here is added to it, never declared alone.
    declared: Vec<TableId>,
    /// The tables its builds read beyond the families' lists, kept for every view after.
    learned: Vec<TableId>,
    /// The families the list it declared was for.
    shown: Shown,
    tables: Option<OverlayTables>,
    tabled: Option<Tabled>,
    /// How many tables it has made: what a layout's key names them by.
    tabled_count: u64,
    /// The counters' memory it lays out on: the view's other one, swapped in the frame's window
    /// ([`crate::map_views::Presentation::take_memory`]).
    laying: Option<(usize, Arc<Presentation>)>,
    laid: Option<Laid>,
    /// The counter kinds the map's layers draw and hold a product for, which the tables then leave
    /// out: what the last layout's memory was given, never a memory's own state.
    left_out: Kinds,
    lay_ins: Arc<LayIns>,
    lay_in_keys: LayInKeys,
    carried: Carried,
    marks: timeline::Maker,
}

impl Maker {
    /// Every ask it is given, answered: the newest one is taken, its rows waited for, its tables
    /// built and its overlay laid out and published. An ask is taken only once the step's join has
    /// taken the tables published before it; whether that ask lays out is the step's to say
    /// ([`lays`]).
    fn work(&mut self) {
        loop {
            let want = {
                let shared = Arc::clone(&self.shared);
                let mut held = lock(&shared.held);
                loop {
                    if held.stop {
                        return;
                    }
                    let published = held.built.is_some();
                    if let Some(want) = next_ask(&mut held.want, published) {
                        break want;
                    }
                    held = shared.signal.wait_timeout(held, LOOK).unwrap_or_else(std::sync::PoisonError::into_inner).0;
                }
            };
            self.cycle(want);
        }
    }

    /// One ask answered: the rows of its hour, the tables made over them and the overlay laid out,
    /// published as one. An ask a newer one superseded is dropped where it stands.
    fn cycle(&mut self, mut want: Wanted) {
        if self.carried.take(&mut want) {
            (self.tables, self.tabled, self.laid) = (None, None, None);
        }
        let mut loading = Vec::new();
        let made = self.make(&want, &mut loading);
        self.carried.ended(&made);
        let laid = match made {
            Ok((stands, laid)) => laid.map(|made| (stands, made)),
            Err(Gave::Superseded) => {
                self.answered(&want, None, loading, false);
                return;
            }
            Err(Gave::Refused(why)) => {
                loading.push(why);
                self.answered(&want, None, loading, true);
                return;
            }
        };
        let failed = want.lay && laid.is_none();
        self.answered(&want, laid, loading, failed);
    }

    /// The tables of the hour asked and the overlay laid out over them. The inner none is a
    /// layout unchanged, which leaves the frame the overlay it holds, standing at this hour too;
    /// the outer none is a layout this ask did not make — not asked for, or refused — which says
    /// nothing about the hour the overlay stands at.
    fn make(&mut self, want: &Wanted, loading: &mut Vec<String>) -> Result<(Hour, Option<Option<Made>>), Gave> {
        // The rows asked for are the families this view can draw: declared before they are waited
        // for, so a view that stops drawing a family stops paying for its rows at the next step.
        self.declare(want.ask.need(&self.content.counters.defines).shown);
        let (rows, stands, waited_from) = self.rows_at(want, loading)?;
        timeline::mark(self.marks, Mark::Rows);
        note_us("overlay.rows_us", waited_from);
        timeline::mark(self.marks, Mark::BuildBegan);
        let built = Instant::now();
        let tabled = Tabled {
            branch: rows.branch,
            hour: rows.hour,
            rows: Arc::as_ptr(&rows) as usize,
            perspective: want.perspective.clone(),
            vision: Arc::as_ptr(&want.vision) as usize,
            engaged: want.engaged.as_deref().is_some_and(|index| index.covers(rows.branch, rows.hour)),
            left_out: self.left_out,
            labels: want.labels,
            need: want.ask.need(&self.content.counters.defines),
            raids: want.lay_ins.raids.as_ref().map(|icons| Arc::as_ptr(icons) as usize),
        };
        // Tables made for other inputs are never published as these: a build that learned a table
        // or failed leaves the publication before it standing at its own hour.
        let made = self.tabled.as_ref() == Some(&tabled) || self.make_tables(want, &rows, &tabled, loading);
        if !made || self.tables.is_none() {
            timeline::mark(self.marks, Mark::BuildEnded);
            return Err(Gave::Refused(format!("the overlay's tables at hour {} were not made", rows.hour)));
        }
        note_us("overlay.tables_us", built);
        // The counter layers settle over the tables of their own hour: published here, as soon as
        // they are made, so the step that asked joins them without waiting for the layout below.
        self.publish_tables(want, stands);
        if !want.lay {
            timeline::mark(self.marks, Mark::BuildEnded);
            return Ok((stands, None));
        }
        let made = self.lay(want, &rows, stands, loading);
        timeline::mark(self.marks, Mark::BuildEnded);
        Ok((stands, made))
    }

    /// The overlay's rows and the hour they stand at: the side's view store makes them from the
    /// side's own mirrors inside the step that asked, so the rows of the hour asked are made while
    /// its layers settle and this waits only for the make to finish. A saved frame waits for the
    /// rows of exactly its hour, however long they take, and is refused by name.
    fn rows_at(&mut self, want: &Wanted, loading: &mut Vec<String>) -> Result<(Arc<DecodedFrame>, Hour, Instant), Gave> {
        let started = Instant::now();
        let until = started + WHOLE;
        let (exact, hour) = (want.whole, want.hour);
        loop {
            let seen = self.rows.published();
            let held = self.rows.frame();
            if let Some(rows) = &held {
                let stands = self.stands(rows);
                let reached = if exact { rows.hour == hour } else { stands >= hour };
                // Rows past this ask's hour carry moves only a newer ask holds: when one waits, it
                // builds them, so no layout claims rows it has not the moves of.
                if reached && !exact && rows.hour > hour && self.superseded(want) {
                    return Err(Gave::Superseded);
                }
                if reached {
                    return Ok((Arc::clone(rows), stands, started));
                }
            }
            let tabled = self.tabled.as_ref().map(|tabled| tabled.hour);
            match waits(exact, self.superseded(want), held.as_ref().map(|rows| rows.hour), tabled) {
                Waits::Rows => {}
                // Built from these already and a newer ask stands: the step's hour is this ask's
                // or later, so the rows this ask waits for are the ones a later ask will take.
                Waits::Build => {
                    let rows = held.expect("the rows the ask is given up for");
                    let stands = self.stands(&rows);
                    return Ok((rows, stands, started));
                }
                Waits::Give => {
                    if !exact {
                        loading.push(format!("the overlay's rows have not reached hour {hour}"));
                    }
                    return Err(Gave::Superseded);
                }
            }
            if exact {
                if started.elapsed() > Duration::from_secs(1) {
                    lock(&self.shared.held).loading = vec![format!("the overlay's rows have not reached hour {hour}: {}", self.rows_state())];
                }
                if Instant::now() >= until {
                    return Err(Gave::Refused(format!("The overlay's rows did not reach hour {hour}: {}", self.rows_state())));
                }
            }
            self.rows.wait_newer(seen, LOOK.min(until.saturating_duration_since(Instant::now())).max(Duration::from_micros(200)));
        }
    }

    /// What the side's view store has made for this maker, for a refusal that says why its rows
    /// did not arrive: the hour of the frame it holds, how many it has been given, and the sets it
    /// reads.
    fn rows_state(&self) -> String {
        let held = match self.rows.frame() {
            Some(rows) => format!("the store holds rows of hour {}", rows.hour),
            None => "the store holds none".to_owned(),
        };
        format!("{held}, {} made for it, {} tables read, {} declared", self.rows.published(), self.rows.reads(), self.declared.len())
    }

    /// Declares `missed` as well: a table the build read that its rows did not hold is kept for
    /// every view from now on, whatever family it belongs to.
    fn learn(&mut self, missed: &[TableId]) {
        let before = self.learned.len();
        self.learned.extend(missed.iter().copied());
        self.learned.sort_unstable_by_key(|table| table.index());
        self.learned.dedup();
        if self.learned.len() != before {
            self.declare_now(self.shown);
        }
    }

    /// The tables of the families `shown` can draw declared, when they are not the families the
    /// list it declared was for.
    fn declare(&mut self, shown: Shown) {
        if self.shown != shown {
            self.declare_now(shown);
        }
    }

    /// The tables of the families `shown` can draw, with every table learned, declared as the one
    /// list the store's reader holds; nothing is said while the list stands.
    fn declare_now(&mut self, shown: Shown) {
        let mut tables = tables_shown(shown);
        tables.extend(self.learned.iter().copied());
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        self.shown = shown;
        if self.declared != tables {
            self.rows.declare_only(&tables);
            self.declared = tables;
        }
    }

    fn stands(&self, rows: &DecodedFrame) -> Hour {
        self.rows.through(rows).unwrap_or(rows.hour).max(rows.hour)
    }

    /// A newer ask has been posted for this picture.
    fn superseded(&self, want: &Wanted) -> bool {
        let held = lock(&self.shared.held);
        held.stop || held.want.as_ref().is_some_and(|newer| newer.generation != want.generation || newer.stamp > want.stamp)
    }


    /// The tables of `rows`, built here on the maker's thread; a table they do not hold is learned
    /// and the tables made last are kept. Whether these were made.
    fn make_tables(&mut self, want: &Wanted, rows: &Arc<DecodedFrame>, tabled: &Tabled, loading: &mut Vec<String>) -> bool {
        let at = crate::document::Hour::new(&self.content, rows, 0.0).with_air_rosters(Some(Arc::clone(&want.ask.rosters)));
        let raids = want.lay_ins.raids.clone().unwrap_or_default();
        let mut spans = viewer_foundation::profiling::BuilderSpans::new(replay_frame::cpu::enabled());
        let engaged = want.engaged.as_deref();
        let (made, missed) = replay_frame::frame::discovering(|| {
            at.hour_tables_at_head(&want.perspective, Arc::clone(&want.vision), raids, engaged, tabled.left_out, want.left, tabled.labels, tabled.need, &mut spans)
        });
        if !missed.is_empty() {
            self.learn(&missed);
            if want.whole {
                loading.push(format!("The overlay's tables at hour {} read tables its rows do not hold: {missed:?}", rows.hour));
            }
            return false;
        }
        let built = match made {
            Ok(tables) => {
                self.tables = Some(tables);
                self.tabled = Some(tabled.clone());
                self.tabled_count += 1;
                viewer_foundation::view_record::add("overlay", "tables_built", 1);
                true
            }
            Err(error) => {
                loading.push(format!("the overlay's tables: {error}"));
                false
            }
        };
        for part in spans.into_records() {
            viewer_foundation::map::redraws::note(part.name, (part.ms * 1000.0) as u64);
        }
        built
    }

    /// The layout of the ask: the map's layers' products laid into the tables, their counters taken,
    /// and the overlay laid out for the view the frame posted when anything it stands for changed.
    /// `Some(None)` when nothing it stands for changed and none when it could not be made
    /// ([`Self::make`]).
    fn lay(&mut self, want: &Wanted, rows: &Arc<DecodedFrame>, stands: Hour, loading: &mut Vec<String>) -> Option<Option<Made>> {
        let started = Instant::now();
        if self.lay_in_keys != want.keys {
            self.lay_ins = Arc::clone(&want.lay_ins);
            self.lay_in_keys = want.keys.clone();
        }
        let lay_ins = Arc::clone(&self.lay_ins);
        let tabled_count = self.tabled_count;
        let (ask, tables) = (&want.ask, self.tables.as_mut()?);
        lay_ins.set(tables);
        let left_out = tables.counters().left_out();
        // The memory of the view's own the maker lays out on, made once for each view: the frame
        // swaps it with the view's in its window, and nothing is laid out while the layout
        // published stands untaken ([`lays`]), so the two alternate and neither is written while
        // the other draws. A saved frame lays out on the view's own memory, with the map
        // icon windows it has retained: nothing else of the window runs while it waits for this
        // picture, and its windows are the ones the window itself would lay out.
        let laying = match &self.laying {
            _ if want.whole => Arc::clone(&ask.view.presentation),
            Some((made_for, laying)) if *made_for == Arc::as_ptr(&ask.view.presentation) as usize => Arc::clone(laying),
            _ => {
                let laying = Arc::new(ask.view.presentation.sharing());
                self.laying = Some((Arc::as_ptr(&ask.view.presentation) as usize, Arc::clone(&laying)));
                laying
            }
        };
        let layered = {
            let mut memory = laying.counters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            memory.set_layered(want.published.clone(), want.draws, left_out);
            self.left_out = memory.drawing();
            memory.layered().identity()
        };
        let laid = Laid {
            tables: tabled_count,
            view: ask.view.clone(),
            pixels: ask.pixels,
            ui_scale: ask.ui_scale.to_bits(),
            layered,
            lay_ins: want.keys.clone(),
            left: want.left,
            art: self.content.icons.lazy.version(),
        };
        // Nothing the layout stands for changed: the frame places the overlay it holds, which
        // stands at this hour too. A whole settle lays out whatever this says: a saved frame's
        // overlay is its own hour's, and this key read a layout unchanged at an hour whose
        // counters had moved, which left the hour before's counters in the frame saved for it.
        if !want.whole && self.laid.as_ref() == Some(&laid) {
            return Some(None);
        }
        let view = CounterView { presentation: Arc::clone(&laying), ..ask.view.clone() };
        let at = crate::document::Hour::new(&self.content, rows, 0.0)
            .with_air_rosters(Some(Arc::clone(&ask.rosters)))
            .with_moved_hours(Arc::clone(&want.moved));
        let read = OverlayRows::at_head(Some(Arc::clone(rows))).leaving(want.left);
        let made = at.prepare_overlay_at(ask.camera, ask.pixels, ask.ui_scale, tables, &view, &read);
        let missed = read.missed();
        if !missed.is_empty() {
            self.learn(&missed);
            lock(&self.shared.held).missed = missed;
            viewer_foundation::map::redraws::note("missed.full", 1);
            return None;
        }
        lock(&self.shared.held).missed.clear();
        let mut prepared = match made {
            Ok(prepared) => prepared,
            Err(error) => {
                viewer_foundation::map::redraws::note("refused.full", 1);
                loading.push(format!("the overlay: {error}"));
                return None;
            }
        };
        lay_ins.lay(&mut prepared, &at, ask.camera, ask.pixels, ask.ui_scale, tables.ours());
        // A saved frame shows the art of its hour: the art that decoded while it was laid out is
        // settled and the hour laid out again, until none is left.
        if want.whole {
            for _ in 0..8 {
                self.content.icons.lazy.settle();
                if !prepared.is_stale(&at) {
                    break;
                }
                match at.prepare_overlay_at(ask.camera, ask.pixels, ask.ui_scale, tables, &view, &read) {
                    Ok(again) => prepared = again,
                    Err(error) => {
                        loading.push(format!("the overlay: {error}"));
                        return None;
                    }
                }
                lay_ins.lay(&mut prepared, &at, ask.camera, ask.pixels, ask.ui_scale, tables.ours());
            }
        }
        viewer_foundation::map::redraws::note("made.full", 1);
        note_us("overlay.layout_us", started);
        self.laid = Some(laid);
        Some(Some(Made {
            hour: stands,
            branch: rows.branch,
            prepared,
            tables: Arc::new(tables.clone()),
            rows: Arc::clone(rows),
            placed_at: ask.camera,
            on: laying,
            published: want.published.clone(),
            draws: want.draws,
            view: ask.view.clone(),
            left: want.left,
            lay_ins,
            moved: Arc::clone(&want.moved),
        }))
    }

    /// The tables of the hour asked, published the moment they are made, with what the rows and
    /// the tables took: the step's join waits for them and the counter layers settle over them.
    fn publish_tables(&mut self, want: &Wanted, stands: Hour) {
        let Some(tables) = &self.tables else { return };
        if want.generation != self.carried.generation {
            return;
        }
        let mut held = lock(&self.shared.held);
        held.built = Some(Built {
            stamp: want.stamp,
            generation: want.generation,
            hour: stands,
            tables: Arc::new(tables.clone()),
            lay_ins: Arc::clone(&self.lay_ins),
        });
        drop(held);
        timeline::mark(self.marks, Mark::Published);
        self.shared.signal.notify_all();
    }

    /// The layout, published when it is done: the first step to join after takes it, and none
    /// waits for it. `made` is none when nothing the layout stands for changed, which leaves the
    /// frame the layout it holds, standing at this hour too.
    fn publish_layout(&mut self, want: &Wanted, stands: Hour, made: Option<Made>) {
        if want.generation != self.carried.generation {
            return;
        }
        let mut held = lock(&self.shared.held);
        held.laid_out = Some(LaidOut { generation: want.generation, hour: stands, made });
        drop(held);
        self.shared.signal.notify_all();
    }

    /// The ask is answered however it ended: refused, superseded or built. A step waiting for its
    /// tables goes on at once rather than out to its budget, and a whole settle asks again. `laid`
    /// is the hour the overlay stands at with the layout it was laid out for, when this ask laid
    /// one out at all.
    fn answered(&mut self, want: &Wanted, laid: Option<(Hour, Option<Made>)>, loading: Vec<String>, failed: bool) {
        if let Some((stands, made)) = laid {
            self.publish_layout(want, stands, made);
        }
        let mut held = lock(&self.shared.held);
        held.loading = loading;
        (held.answered, held.failed) = (want.stamp, failed);
        drop(held);
        self.shared.signal.notify_all();
    }
}

/// The vision the map's layers show, copied out of the vision layer whenever it moved and made on a
/// thread of its own, which writes only the provinces whose byte changed since the copy before: the
/// counters stand in the fog the map shows. The tables and the counter layers hold the one made
/// until the next comes. A saved frame asks for it `exact`: the layer's as it stands, waited for.
#[derive(Default)]
pub struct ShownVision {
    held: Option<(VisionKey, Instant, Result<Arc<Vision>, String>)>,
    /// The copy being made, with the count of seeks it was asked under ([`Self::forget`]).
    making: Option<(VisionKey, Instant, u64)>,
    forgotten: u64,
    maker: Option<(std::sync::mpsc::Sender<crate::layers::vision::SeenCopy>, std::sync::mpsc::Receiver<Arc<Vision>>)>,
}

/// What moves the vision layer's value: a step applied, its worker's product, its units still
/// settling, and the perspective.
#[derive(Clone, Debug, PartialEq)]
struct VisionKey {
    applied: Option<Hour>,
    published: crate::layers::Published,
    carried: usize,
    choice: Choice,
}

impl ShownVision {
    /// The vision `layer` shows, copied when it moved and no copy is being made; the one held
    /// meanwhile. `exact`: nothing until the copy of the layer as it stands now stands.
    pub fn shown(&mut self, layer: Option<&crate::layers::vision::Vision>, applied: Option<Hour>, parts_home: bool, exact: bool) -> Result<Arc<Vision>, String> {
        use crate::layers::Layer as _;
        self.take_made();
        let layer = match (parts_home, layer) {
            (true, Some(layer)) => layer,
            (true, None) => return Err("the map draws no vision layer".into()),
            (false, _) => return self.kept(),
        };
        let key = VisionKey { applied, published: layer.published(), carried: layer.carried().entities, choice: layer.choice().clone() };
        let now = Instant::now();
        let due = self.held.as_ref().is_none_or(|(held, ..)| *held != key);
        if due && self.making.is_none() {
            match layer.shown_copy() {
                Err(why) => self.held = Some((key.clone(), now, Err(why))),
                Ok(copy) => self.make(key.clone(), now, copy),
            }
        }
        // A saved frame shows the vision of its own hour or is refused by name: the copy of the
        // layer as it stands now, not the one held from before.
        match &self.held {
            Some((held, _, value)) if !exact || *held == key => value.clone(),
            Some((_, _, Err(why))) => Err(why.clone()),
            _ => Err("the vision of this step is still being copied out of the layer".into()),
        }
    }

    /// `copy` made into the vision on the maker's thread, started on first use.
    fn make(&mut self, key: VisionKey, at: Instant, copy: crate::layers::vision::SeenCopy) {
        if self.maker.is_none() {
            let (jobs, receive) = std::sync::mpsc::channel::<crate::layers::vision::SeenCopy>();
            let (send, made) = std::sync::mpsc::channel();
            let started = std::thread::Builder::new().name("shown vision".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                let mut before = None;
                for copy in receive {
                    let (seen, made) = copy.make_after(before.as_ref());
                    before = made;
                    if send.send(Arc::new(seen)).is_err() {
                        return;
                    }
                }
            });
            if started.is_ok() {
                self.maker = Some((jobs, made));
            }
        }
        match &self.maker {
            Some((jobs, _)) if jobs.send(copy).is_ok() => self.making = Some((key, at, self.forgotten)),
            _ => self.held = Some((key, at, Err("no thread to make the vision on".into()))),
        }
    }

    /// The vision made, once it has arrived.
    fn take_made(&mut self) {
        let Some((_, made)) = &self.maker else { return };
        while let Ok(vision) = made.try_recv() {
            if let Some((key, at, forgotten)) = self.making.take()
                && forgotten == self.forgotten
            {
                self.held = Some((key, at, Ok(vision)));
            }
        }
    }

    pub fn kept(&self) -> Result<Arc<Vision>, String> {
        self.held.as_ref().map_or_else(|| Err("the vision is being made".into()), |(_, _, held)| held.clone())
    }

    /// [`Self::kept`] while it is the vision of `choice`: another country's is no vision of this
    /// one, however recent.
    pub fn kept_for(&self, choice: &Choice) -> Result<Arc<Vision>, String> {
        match &self.held {
            Some((key, _, held)) if key.choice == *choice => held.clone(),
            Some(_) => Err("the vision of this perspective is still being copied".into()),
            None => Err("the vision is being made".into()),
        }
    }

    /// A seek's reset: the vision held is of another hour of the history, and none is shown until
    /// the copy of this one stands.
    pub fn forget(&mut self) {
        self.held = None;
        self.forgotten += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_views::Presentation;

    fn tabled() -> Tabled {
        Tabled {
            branch: 1,
            hour: 7,
            rows: 0x1000,
            perspective: Choice::default(),
            vision: 0x2000,
            engaged: false,
            left_out: Kinds::default(),
            labels: false,
            need: Need { air_missions: false, by_key: true, shown: Shown::EVERY },
            raids: None,
        }
    }

    /// A view that can draw no icon family asks for none of those families' tables, and the set it
    /// asks for is always a part of the whole list a tooltip, a click or a panel reads.
    #[test]
    fn the_rows_asked_for_are_the_tables_of_the_families_the_view_can_draw() {
        let every = overlay_tables();
        let none = tables_shown(Shown { units: false, bases: false, land_combats: false, naval_missions: false });
        assert!(none.iter().all(|table| every.contains(table)), "the families' tables are a part of the whole list");
        // What every view reads whatever it draws: the overlay's own tables and the vision's,
        // which is why Divisions and DivisionMovement belong to no family.
        for table in OVERLAY.iter().chain(viewer_foundation::map::perspective::VISION_TABLES) {
            assert!(none.contains(table), "{table:?} is read at every view, whatever it draws");
        }
        for (family, tables) in [("units", UNIT_COUNTERS), ("land combats", LAND_COMBATS), ("naval missions", NAVAL_MISSIONS), ("bases", BASE_ICONS)] {
            for table in tables {
                assert!(every.contains(table), "{family}'s {table:?} is read at a view that draws it");
                // A table some other reader of this maker's rows reads at every view is not a
                // family's to claim: dropping the family would not stop its rows being made.
                assert!(!none.contains(table), "{family}'s {table:?} is not read where nothing draws it");
            }
        }
        // Each family alone: no other family's tables come with it.
        let units = tables_shown(Shown { units: true, bases: false, land_combats: false, naval_missions: false });
        assert!(UNIT_COUNTERS.iter().all(|table| units.contains(table)));
        assert!(BASE_ICONS.iter().all(|table| !units.contains(table)));
    }

    /// A layout's key for a view of `on`: one memory throughout, because a view is the same view
    /// only while its memory is ([`CounterView`]'s own equality).
    fn laid_on(on: &Arc<Presentation>) -> Laid {
        Laid {
            tables: 3,
            view: CounterView { presentation: Arc::clone(on), ..CounterView::default() },
            pixels: [2560, 1440],
            ui_scale: 1.0f32.to_bits(),
            layered: [0; 6],
            lay_ins: Default::default(),
            left: LeftToLayers::default(),
            art: 0,
        }
    }

    /// The overlay's tables stand for the rows of the step's hour and the vision the step copied
    /// out: new rows, a new vision, the engaged index reaching the hour or a new set of raid
    /// arrows make them again. Where the camera stands is not one of them, so the tables of a step
    /// are made once however the camera moves.
    #[test]
    fn the_tables_stand_for_the_rows_and_vision_of_the_steps_hour() {
        let held = tabled();
        assert_eq!(held, tabled(), "nothing changed: the step keeps the tables it has");
        for changed in [
            Tabled { hour: 8, rows: 0x1800, ..tabled() },
            Tabled { vision: 0x2400, ..tabled() },
            Tabled { engaged: true, ..tabled() },
            Tabled { raids: Some(0x4000), ..tabled() },
            Tabled { left_out: Kinds::default().with(Kinds::LAND), ..tabled() },
            Tabled { need: Need { air_missions: false, by_key: false, shown: Shown::EVERY }, ..tabled() },
            Tabled { need: Need { shown: Shown { units: false, ..Shown::EVERY }, ..tabled().need }, ..tabled() },
        ] {
            assert_ne!(held, changed, "the tables are made again for it");
        }
    }

    /// A newer ask, or the maker stopping, ends the ask waiting for rows wherever it waits. A seek
    /// back leaves the store standing at an hour before the tables the maker holds, and the ask it
    /// is waiting for will never be reached from there: it waited on in 200-microsecond rounds,
    /// never taking the new picture's ask, and the stop a drop posts went the same way, so closing
    /// the map waited on `worker.join()` for ever.
    #[test]
    fn an_ask_superseded_after_a_seek_back_is_given_up_and_a_stop_is_taken_from_every_wait() {
        // Nothing newer asked: the ask waits for the rows of its own hour, however far they are.
        assert_eq!(waits(false, false, Some(50), Some(90)), Waits::Rows);
        assert_eq!(waits(false, false, None, None), Waits::Rows);
        // A newer ask, and rows newer than the tables it holds: this ask builds from them and the
        // next one takes the hour it waited for.
        assert_eq!(waits(false, true, Some(95), Some(90)), Waits::Build);
        assert_eq!(waits(false, true, Some(50), None), Waits::Build);
        // A seek back: the rows stand at or before the tables it holds. The ask is given up, which
        // is how the picture after the seek is asked for and how a stop is taken.
        assert_eq!(waits(false, true, Some(50), Some(90)), Waits::Give);
        assert_eq!(waits(false, true, Some(90), Some(90)), Waits::Give);
        assert_eq!(waits(false, true, None, Some(90)), Waits::Give);
        // A saved frame's ask is of exactly its hour: a newer ask or a stop ends it by name.
        assert_eq!(waits(true, true, Some(95), Some(90)), Waits::Give);
        assert_eq!(waits(true, false, Some(95), Some(90)), Waits::Rows);
    }

    /// A hover, a selection or an interface scale the frame posts makes a new layout, which the
    /// maker makes for the view even when no step is due; an unchanged view keeps the layout, so a
    /// still frame lays nothing out. The camera is not one of them: a layout is placed at any
    /// camera.
    #[test]
    fn a_changed_view_makes_a_new_layout_and_an_unchanged_one_keeps_it() {
        let on = Arc::new(Presentation::default());
        let laid = || laid_on(&on);
        let view = |view: CounterView| CounterView { presentation: Arc::clone(&on), ..view };
        let held = laid();
        assert_eq!(held, laid(), "nothing changed: the frame places the overlay it holds");
        let hovered = view(CounterView { hovered: Some(viewer_foundation::unit_handle::Unit::Division(7)), ..CounterView::default() });
        let selected = view(CounterView { selected: Arc::new([viewer_foundation::unit_handle::Unit::Division(3)].into()), ..CounterView::default() });
        for changed in [
            Laid { view: hovered, ..laid() },
            Laid { view: selected, ..laid() },
            Laid { ui_scale: 1.5f32.to_bits(), ..laid() },
            Laid { pixels: [1920, 1080], ..laid() },
            Laid { tables: 4, ..laid() },
            Laid { layered: [1, 0, 0, 0, 0, 0], ..laid() },
            Laid { art: 1, ..laid() },
            Laid { left: LeftToLayers { resource_icons: true, occupation_marks: false, radar: false }, ..laid() },
        ] {
            assert_ne!(held, changed, "the overlay is laid out again for it");
        }
        // The pressed plate and the plate under the pointer draw without a layout.
        let pressed = view(CounterView { pressed: Some(viewer_foundation::unit_handle::Unit::Division(7)), plate_hovered: Some(viewer_foundation::unit_handle::Unit::Division(7)), ..CounterView::default() });
        assert_eq!(held, Laid { view: pressed, ..laid() }, "a plate's state draws the layout again, never lays it out");
        // A view of its own memory is another view: its layouts are not this one's.
        assert_ne!(held, laid_on(&Arc::new(Presentation::default())));
    }

    /// The step gives the overlay's tables to the counter layers just before the first of those
    /// layers settles, so their products are made from the tables the frame's overlay was laid out
    /// over; the vision and engagements layers, which the tables read, are registered before it.
    #[test]
    fn the_tables_are_given_before_the_first_counter_layer_settles() {
        let names = ["political", "vision", "engagements", "counters", "land_markers", "labels"];
        assert_eq!(counters_at(&names), Some(3), "the land stacks' layer, after the vision and the engagements");
        assert_eq!(counters_at(&["political", "vision", "labels"]), None, "a map drawing no counter kind makes no tables for them");
    }

    /// Steps come faster than the overlay is built. Each build is of the newest hour asked, one is
    /// in flight at a time, no build finished is ever passed over, and the overlay shown only moves
    /// forward — so the only thing between the overlay and the step's hour is the build in flight.
    /// Three steps to a build, twelve game hours a step: the stress test's shape.
    #[test]
    fn steps_faster_than_a_build_keep_the_newest_overlay_and_never_go_back() {
        const STEPS_PER_BUILD: u32 = 3;
        const HOURS_PER_STEP: Hour = 12;
        let mut asked: Option<(u64, Hour)>;
        let mut building: Option<(u64, Hour, u32)> = None;
        let mut built: Option<(u64, Hour)> = None;
        let (mut taken, mut shown) = (0u64, None::<Hour>);
        let (mut finished, mut installed) = (Vec::new(), Vec::new());
        for step in 1u64..=12 {
            let hour = HOURS_PER_STEP * step as Hour;
            // The step's apply asks for its hour, latest wins.
            asked = Some((step, hour));
            // The maker works while the step settles its layers: a build begun is always finished,
            // and the next ask is taken only once the overlay published has been taken.
            if let Some((_, _, left)) = &mut building {
                *left -= 1;
                if *left == 0 {
                    let (stamp, at, _) = building.take().expect("held above");
                    assert!(built.is_none(), "nothing is built over an overlay the step has not taken");
                    finished.push(at);
                    built = Some((stamp, at));
                }
            }
            if building.is_none()
                && let Some((stamp, at)) = next_ask(&mut asked, built.is_some())
            {
                assert_eq!(at, hour, "every build is of the newest hour asked");
                building = Some((stamp, at, STEPS_PER_BUILD));
            }
            // The step's join takes what the maker published, while it is this picture's and newer.
            if let Some((stamp, at)) = built.take() {
                assert!(shows((0, stamp), 0, taken), "the build published is this picture's and newer than the one held");
                assert!(shown.is_none_or(|held| at > held), "the overlay shown only moves forward");
                (taken, shown) = (stamp, Some(at));
                installed.push(at);
            }
        }
        assert_eq!(installed, finished, "every build finished is shown, in order: only the build in flight is between the overlay and the step");
        assert_eq!(shown, Some(HOURS_PER_STEP * 9), "the overlay advanced with every build");
        assert!(!shows((0, taken + 1), 1, taken), "a build in flight across a reset is never shown");
    }

    /// The overlay the frame holds only moves forward, and a layout laid out for the picture before
    /// a reset is never installed as this one's. A seek clears the hour the overlay stands at, so
    /// the first layout of the history sought is installed although it stands before the one shown.
    #[test]
    fn a_layout_installed_is_this_pictures_and_never_stands_before_the_one_held() {
        assert!(installs((3, 100), 3, Some(100)), "the hour it stands at, laid out again");
        assert!(installs((3, 110), 3, Some(100)), "a later hour");
        assert!(!installs((3, 90), 3, Some(100)), "an earlier hour: the overlay never goes back");
        assert!(!installs((2, 200), 3, Some(100)), "laid out for the picture before this one");
        assert!(installs((3, 90), 3, None), "after a seek back, the first layout of the history sought");
    }

    /// A layout is laid out on the counters' memory the frame swaps in with it, so the maker lays
    /// the next one out only once the frame has taken the one published and handed the other memory
    /// back. Before this rule the maker laid one out about every seven milliseconds against a
    /// frame's eight, and the frame dropped 68-93% of them as laid out on a memory a layout ahead
    /// of the quads it held: the overlay then advanced on 4-7% of frames and stood hundreds of
    /// hours behind the layers. A whole settle is exempt: it lays out on the view's own memory and
    /// the frame it settles for waits for it.
    #[test]
    fn nothing_is_laid_out_while_the_layout_published_stands_untaken() {
        assert!(lays(true, false, false), "no layout untaken: the step's ask lays one out");
        assert!(!lays(true, false, true), "one untaken: the memory it is on is not written again");
        assert!(lays(true, true, true), "a whole settle lays out on the view's own memory");
        assert!(!lays(false, true, false), "a caught-up step lays nothing out");
        // Twelve frames of the stress test's shape: a cycle of two frames, a step a frame.
        const CYCLE: u32 = 2;
        let (mut untaken, mut laying, mut left) = (false, None::<Hour>, 0u32);
        let (mut laid, mut taken) = (Vec::new(), Vec::new());
        for frame in 1u64..=12 {
            let hour = 12 * frame as Hour;
            // The step's apply asks; the maker lays out only while nothing it laid out stands untaken.
            if laying.is_none() && lays(true, false, untaken) {
                (laying, left) = (Some(hour), CYCLE);
                laid.push(hour);
            }
            if let Some(at) = laying {
                left -= 1;
                if left == 0 {
                    (laying, untaken) = (None, true);
                    assert_eq!(laid.last(), Some(&at));
                }
            }
            // The frame's window takes what the maker published, with the memory it was laid on.
            if untaken {
                untaken = false;
                taken.push(laid[taken.len()]);
            }
        }
        assert_eq!(laid, taken, "every layout the maker lays out reaches the frame, in order");
        assert_eq!(taken.len(), 6, "one layout every two frames: the maker's cycle, and nothing lost to it");
    }

    /// A saved frame waits for the overlay of its own hour, and the hour it waits on is the hour
    /// the layout it holds stands at. The maker lays nothing out for an hour the overlay reads
    /// nothing new in — it publishes that hour alone, and the frame places the layout it already
    /// has — so a settle that asked instead for a layout waiting to be taken waited out its minute
    /// for one nobody was going to make, and a three-frame saved scene was refused at its third
    /// hour with the overlay already standing there.
    #[test]
    fn the_overlay_stands_at_the_hour_of_the_layout_the_frame_holds() {
        assert!(stands(Some(100), 0, 100), "the layout the frame holds was laid out for this hour");
        assert!(stands(Some(110), 0, 100), "a layout of a later hour stands at this one");
        assert!(!stands(Some(90), 0, 100), "a layout of an earlier hour: the hour has not arrived");
        assert!(!stands(None, 0, 100), "before the first layout there is no overlay to stand");
        assert!(!stands(Some(100), 1, 100), "a table its tables read is still missing");
        // Three saved frames, the third reading nothing new. Each hour is published, the first two
        // with a layout to take and the third without, and every one of them stands at its hour.
        let (mut hour, mut untaken) = (None::<Hour>, None::<Hour>);
        for (head, laid) in [(100, Some(100)), (200, Some(200)), (300, None)] {
            assert!(installs((0, head), 0, hour), "hour {head} is this picture's and no earlier than the one held");
            hour = Some(head);
            untaken = laid.or(untaken.take());
            assert!(stands(hour, 0, head), "hour {head}: the frame is given its picture");
            // The frame's window takes a layout when one was laid out, and nothing when none was.
            assert_eq!(untaken.take(), laid, "hour {head}");
        }
    }

    /// The overlay never lengthens a step: a step's join takes what the maker has published and
    /// waits for nothing. A whole settle waits for the ask it posted to be answered, and stops
    /// only then or when the wait runs out. The overlay standing at the hour is no reason to
    /// stop: a saved frame asks for its own hour again over the art that arrived while it drew,
    /// and the overlay has stood at that hour since its first layout, so a join that stopped
    /// there took no layout at all and the frame was saved with the layout the art moved under.
    #[test]
    fn a_step_never_waits_for_the_overlay_and_a_whole_settle_waits_for_the_ask_it_posted() {
        for answered in [false, true] {
            assert!(!joins_on(false, answered, false), "a step waits for nothing: answered {answered}");
        }
        assert!(joins_on(true, false, false), "a whole settle waits for the answer to the ask it posted");
        assert!(!joins_on(true, true, false), "answered: the layout this ask made is in");
        assert!(!joins_on(true, false, true), "the wait ran out");
    }

    use viewer_foundation::unit_handle::Unit;

    /// The division that leaves province A for B at hour 112 and comes back at 130.
    const DIVISION: Unit = Unit::Division(7);
    const OUT: (i32, u32) = (112, 2);
    const BACK: (i32, u32) = (130, 1);

    fn moves(of: &[(Unit, (i32, u32))]) -> Arc<MovedHours> {
        let mut moved = MovedHours::default();
        for &(unit, (hour, province)) in of {
            moved.note(unit, hour, province);
        }
        Arc::new(moved)
    }

    /// The maker's side of the handoff with the real slot, picture and moves, its tables and
    /// layout replaced by what each request's rows allow: the layout sink records the moves every
    /// layout read, as [`Maker::lay`] gives them to the counters.
    #[derive(Default)]
    struct Bench {
        slot: Option<Request<()>>,
        generation: u64,
        stamp: u64,
        carried: Carried,
        tabled: Option<Hour>,
        layouts: Vec<(Hour, Arc<MovedHours>)>,
    }

    impl Bench {
        fn ask(&mut self, hour: Hour, moved: Arc<MovedHours>, lay: bool) {
            self.stamp += 1;
            let want = Request { generation: self.generation, stamp: self.stamp, hour, moved, whole: false, lay, asking: () };
            post(&mut self.slot, want);
        }

        fn take(&mut self) -> Request<()> {
            let mut want = next_ask(&mut self.slot, false).expect("a request posted");
            self.carried.take(&mut want);
            want
        }

        /// `want` answered over the rows the store holds (`rows`), as [`Maker::rows_at`] decides
        /// once a newer request may have been posted (`newer`).
        fn answer(&mut self, want: &Request<()>, rows: Hour, newer: bool) {
            let made = if rows >= want.hour {
                Ok(rows)
            } else {
                match waits(want.whole, newer, Some(rows), self.tabled) {
                    Waits::Build => Ok(rows),
                    Waits::Give => Err(Gave::Superseded),
                    Waits::Rows => panic!("the bench only answers what the maker stops waiting on"),
                }
            }
            .map(|stands| {
                self.tabled = Some(stands);
                (stands, want.lay.then(|| self.layouts.push((stands, Arc::clone(&want.moved)))))
            });
            self.carried.ended(&made);
        }

        fn laid(&self) -> &[(i32, u32)] {
            self.layouts.last().expect("a layout").1.of(DIVISION)
        }
    }

    /// A request replaced before the maker took it hands its moves to the one that replaced it, so
    /// the layout that takes the newer request places the division out to B and back, in order.
    #[test]
    fn a_replaced_request_carries_its_moves_into_the_one_that_replaces_it() {
        let mut bench = Bench::default();
        bench.ask(118, moves(&[(DIVISION, OUT)]), true);
        bench.ask(136, moves(&[(DIVISION, BACK)]), true);
        let want = bench.take();
        bench.answer(&want, 136, false);
        assert_eq!(bench.laid(), [OUT, BACK], "the out-and-back reaches the layout");
    }

    /// The maker took a request and was waiting for its rows when a newer one came: it gives way,
    /// and its moves reach the layout of the request that superseded it.
    #[test]
    fn a_request_that_gives_way_while_waiting_for_rows_leaves_its_moves_to_the_next_layout() {
        let mut bench = Bench { tabled: Some(110), ..Bench::default() };
        bench.ask(118, moves(&[(DIVISION, OUT)]), true);
        let claimed = bench.take();
        bench.ask(136, moves(&[(DIVISION, BACK)]), true);
        bench.answer(&claimed, 110, true);
        assert!(bench.layouts.is_empty(), "the claimed request gave way with no layout");
        let want = bench.take();
        bench.answer(&want, 136, false);
        assert_eq!(bench.laid(), [OUT, BACK], "the out-and-back reaches the layout");
    }

    /// A request answered with its tables alone lays nothing out, and its moves wait for the layout
    /// that follows.
    #[test]
    fn a_request_answered_with_tables_alone_leaves_its_moves_to_the_next_layout() {
        let mut bench = Bench::default();
        bench.ask(118, moves(&[(DIVISION, OUT)]), false);
        let tables = bench.take();
        bench.answer(&tables, 118, false);
        assert!(bench.layouts.is_empty(), "tables alone: no layout");
        bench.ask(136, moves(&[(DIVISION, BACK)]), true);
        let want = bench.take();
        bench.answer(&want, 136, false);
        assert_eq!(bench.laid(), [OUT, BACK], "the out-and-back reaches the layout");
    }

    /// A layout made over rows older than the request retires only the moves up to its rows' hour:
    /// the later ones are placed by the next layout, and the ones it drew are not placed again.
    #[test]
    fn a_layout_over_older_rows_keeps_the_later_moves_for_the_next_layout() {
        let earlier = Unit::Division(9);
        let mut bench = Bench { tabled: Some(100), ..Bench::default() };
        bench.ask(148, moves(&[(earlier, (105, 3)), (DIVISION, OUT), (DIVISION, BACK)]), true);
        let want = bench.take();
        bench.ask(160, Arc::default(), true);
        bench.answer(&want, 110, true);
        assert_eq!(bench.layouts.last().map(|laid| laid.0), Some(110), "laid out over the rows at 110");
        let want = bench.take();
        bench.answer(&want, 160, false);
        assert_eq!(bench.laid(), [OUT, BACK], "the moves after 110 reach the next layout, in order");
        assert!(bench.layouts[1].1.of(earlier).is_empty(), "the move at 105 was drawn by the layout over 110");
    }

    /// A reset starts another picture: no move of the picture before reaches its layouts, whether
    /// the maker held it or a request it never took carried it.
    #[test]
    fn a_reset_clears_the_moves_carried() {
        let mut bench = Bench::default();
        bench.ask(118, moves(&[(DIVISION, OUT)]), false);
        let tables = bench.take();
        bench.answer(&tables, 118, false);
        bench.ask(130, moves(&[(DIVISION, BACK)]), true);
        bench.generation += 1;
        bench.ask(40, Arc::default(), true);
        let want = bench.take();
        bench.answer(&want, 40, false);
        assert!(bench.laid().is_empty(), "the picture before's moves are not this one's");
    }

    /// A layout asked for and not taken by a frame holds the counters' memory the next one would be
    /// laid out on: no step asks for another until the frame takes it, or the maker answers the ask
    /// with no layout pending.
    #[test]
    fn no_layout_is_asked_for_while_the_one_asked_for_is_untaken() {
        let mut owed = Owed::default();
        assert!(owed.lays(true, false, false, || unreachable!("nothing owed")), "nothing owed: the step lays out");
        owed.asked(true, false, 1);
        assert!(!owed.lays(true, false, false, || (0, false)), "the maker has not answered it");
        assert!(!owed.lays(true, false, false, || (1, true)), "answered with a layout no step installed");
        assert!(!owed.lays(true, false, true, || (1, false)), "installed and no frame took it");
        assert!(owed.lays(true, true, false, || (0, false)), "a whole settle lays out on the view's own memory");
        assert!(owed.lays(true, false, false, || (1, false)), "answered with no layout pending: let go");
        owed.asked(true, true, 2);
        assert!(owed.lays(true, false, false, || unreachable!("a whole settle owes nothing")), "a whole settle's ask holds nothing");
    }

    /// A strict join stands only on the answer to its own ask: an earlier ask's answer, however
    /// good, is another ask's overlay.
    #[test]
    fn a_strict_join_fails_unless_its_own_ask_was_answered() {
        assert!(fails(true, 3, 4, false), "strict, an earlier ask answered");
        assert!(!fails(false, 3, 4, false), "a step takes whatever was answered");
        assert!(!fails(true, 4, 4, false), "strict, its own ask answered");
        assert!(fails(false, 4, 4, true), "its own ask answered without what it asked for");
    }

    /// A vision held whose copies the test makes itself: what the maker's thread is sent, and what
    /// it sends back, are in the test's hands.
    struct Making {
        shown: ShownVision,
        sent: std::sync::mpsc::Receiver<crate::layers::vision::SeenCopy>,
        made: std::sync::mpsc::Sender<Arc<Vision>>,
    }

    impl Making {
        fn new() -> Self {
            let (jobs, sent) = std::sync::mpsc::channel();
            let (made, receive) = std::sync::mpsc::channel();
            Self { shown: ShownVision { maker: Some((jobs, receive)), ..ShownVision::default() }, sent, made }
        }

        /// A copy of `ours`'s vision for `choice` at `hour` sent to be made.
        fn ask(&mut self, choice: &Choice, hour: Hour, ours: u32) {
            let key = VisionKey { applied: Some(hour), published: crate::layers::Published::Current, carried: 0, choice: choice.clone() };
            let parts = viewer_foundation::map::perspective::Parts { ours, ..Default::default() };
            self.shown.make(key, Instant::now(), crate::layers::vision::SeenCopy::of_parts(parts));
        }

        /// The copy sent last made and taken in, as the maker's thread and the next frame do.
        fn arrive(&mut self) {
            let copy = self.sent.try_recv().expect("a copy was sent to be made");
            self.made.send(Arc::new(copy.make())).unwrap();
            self.shown.take_made();
        }
    }

    fn country(tag: &str) -> Choice {
        Choice { country: viewer_foundation::map::perspective::Country::Tag(tag.into()), fog: true }
    }

    /// The overlay lays its counters out over the vision of its own perspective alone: the copy
    /// made for one country is never another's, however recent, so switching from A to B shows no
    /// counter of B in A's sightings.
    #[test]
    fn a_vision_made_for_one_perspective_is_never_kept_for_another() {
        let (a, b) = (country("GER"), country("ENG"));
        let mut making = Making::new();
        assert!(making.shown.kept_for(&a).is_err(), "nothing made yet");
        making.ask(&a, 5, 1);
        making.arrive();
        assert_eq!(making.shown.kept_for(&a).map(|vision| vision.ours()), Ok(1), "the copy made for A is A's");
        assert!(making.shown.kept_for(&b).is_err(), "A's copy is no vision of B");
        making.ask(&b, 5, 2);
        assert!(making.shown.kept_for(&b).is_err(), "B's copy is still being made: A's is not shown meanwhile");
        making.arrive();
        assert_eq!(making.shown.kept_for(&b).map(|vision| vision.ours()), Ok(2), "the copy made for B, once it stands");
        assert!(making.shown.kept_for(&a).is_err(), "and B's is no vision of A");
    }

    /// A seek's reset forgets the vision held: none is kept until a copy asked after the reset
    /// stands, and a copy asked before it that arrives after is of another hour of the history, so
    /// it is let go rather than shown at the hour sought.
    #[test]
    fn a_forgotten_vision_is_not_kept_and_a_copy_asked_before_the_forget_is_let_go() {
        let a = country("GER");
        let mut making = Making::new();
        making.ask(&a, 90, 1);
        making.arrive();
        assert!(making.shown.kept_for(&a).is_ok(), "the copy of hour 90 stands");
        making.shown.forget();
        assert!(making.shown.kept_for(&a).is_err(), "forgotten: the vision of hour 90 is not kept");
        assert!(making.shown.kept().is_err(), "nor kept for any perspective");
        making.ask(&a, 95, 1);
        making.shown.forget();
        making.arrive();
        assert!(making.shown.kept_for(&a).is_err(), "a copy asked before the forget is let go when it arrives");
        assert!(making.shown.making.is_none(), "and nothing is still being made");
        making.ask(&a, 20, 3);
        making.arrive();
        assert_eq!(making.shown.kept_for(&a).map(|vision| vision.ours()), Ok(3), "a copy asked after the forget is kept");
    }
}
