//! Retained naval map windows and mission-grid items.
//!
//! Windows belong to per-view pools. Only [`Pool::visible`] entries draw. Refused
//! updates retain pool membership and placement while withholding draws until later
//! writes repair every unknown part ([`Pool::refuse`] and [`Withheld`]).
//! A prepared [`Plan`] contains ordered instance-independent writes. [`present`]
//! applies requests and erasures, icon visibility and placement, grid appends and
//! layout, at-sea counters, then one tick and the windows' draws.

mod draw_parts;

use super::*;
use std::collections::{HashMap, HashSet};
use viewer_foundation::combat_selection::Hit;
use viewer_foundation::combat_selection::Selection;
use viewer_foundation::map::camera::MapIconProjection;
use viewer_foundation::map::gui_resources::GuiAssets;
use viewer_foundation::map::gui_resources::count_noted;
use viewer_foundation::map::gui_resources::count_refused;
use viewer_foundation::map::gui_resources::hit_rectangle;
use viewer_foundation::map::gui_resources::noted_cause;
use viewer_foundation::map::gui_resources::refused_cause;
use viewer_foundation::map::gui_resources::viewport as gui_resources_viewport;
use viewer_foundation::map::icon_windows::Retry;
use viewer_foundation::map::icon_windows::visible_layout_kept;
use viewer_foundation::map::icon_windows::{
    POOL_EXHAUSTED, admission, frame, point, rectangle_inside,
};
use viewer_foundation::map::sprites::hovered_layer;

use viewer_foundation::{
    icon_frames::Kept,
    map::province_schedule::{Schedule, Update},
};

use viewer_foundation::map::native_gui::{
    self, Layout,
    lifecycle::{
        Admission, Held, Instance, MapIconPool, Pool, Registration, Rewrote, SlotId, Tick, Unknown,
        Visible, Withheld,
    },
};

struct BaseControls<'a> {
    background: &'a native::Element,
    strategic: &'a native::Element,
    capacity: &'a native::Element,
}

fn base_controls(root: &native::Element) -> Result<BaseControls<'_>, String> {
    let find = |kind, name: &str| {
        root.find(kind, name)
            .ok_or_else(|| format!("no control named {name}"))
    };
    Ok(BaseControls {
        background: find(native::Kind::Button, "bg_btn")?,
        strategic: find(native::Kind::Button, "bg_strat_btn")?,
        capacity: find(native::Kind::TextBox, "txt_capacity")?,
    })
}

pub(super) fn update_naval(
    instance: &mut Instance,
    root: &native::Element,
    glyph: &NavalBaseIcon,
    scale: f32,
    strategic: bool,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    let _profile = viewer_foundation::profiling::span("dc_nu_base_setters");
    let c = base_controls(root)?;
    for (button, shown) in [(c.background, !strategic), (c.strategic, strategic)] {
        instance.set_frame(
            root,
            button,
            if glyph.highlighted {
                11
            } else {
                glyph.frame.wrapping_add(1) as i32
            },
        )?;
        let bound = instance.control(root, button)?;
        bound.enabled = Some(glyph.enabled);
        bound.scale = Some(scale);
        if shown {
            instance.show(root, button)?;
        } else {
            instance.hide(root, button)?;
        }
    }
    match glyph.capacity.filter(|_| strategic) {
        Some(capacity) => {
            instance.set_text(root, c.capacity, &capacity.text(), resources)?;
            instance.show(root, c.capacity)
        }
        None => instance.hide(root, c.capacity),
    }
}

/// The port items of each naval base province: each item's update, or why the province's roster
/// is undecided (every item its base may have appended is then unknown, [`unknown_items`]).
pub(super) type PortItems = BTreeMap<u32, Result<Vec<gui_windows::Update>, Cow<'static, str>>>;

/// The owner of a naval combat icon: the province whose graphics own it and
/// the combat it shows.
pub(super) type CombatOwner = (u32, u64);

/// A map icon's window is identified by its pool and slot. Each icon type has
/// one pool whose slots start at 0, so combat and base pools share slot numbers.
/// An icon's window, grids, items and refusal are all keyed by both pool and slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum IconSlot {
    /// A naval combat icon (`MapIconPool::Combat`).
    Combat(SlotId),
    /// A naval base icon (`MapIconPool::Base`).
    Base(SlotId),
    /// A retained naval result icon (`MapIconPool::CombatResults`).
    Result(SlotId),
    /// A strategic-region mission icon, with its own three item pools.
    Mission(SlotId),
}

/// The item pool of an icon's object a grid item comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum ItemPool {
    /// A port item: the naval base icon's task-force grid wrapper.
    Port,
    /// A combat tile in this side's wrapper.
    Tile(usize),
    /// A convoy addition in the combat icon's pool, shared by both sides.
    Convoy,
}

/// A grid item's native identity: the icon whose object holds its pool, the pool, and its place
/// in the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ItemKey {
    icon: IconSlot,
    pool: ItemPool,
    index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CarrierKey {
    AtSea(u64),
    Port(ItemKey),
}
type CarrierViews = HashMap<CarrierKey, air_grid::View>;
type CarrierGrid = Result<Vec<air_grid::PreparedItem>, String>;

/// A combat icon's anchor and stack place, assigned by its owner's placing pass
/// and read each frame. A placing update whose place is undecided makes it
/// unknown; an update that does not place the icon keeps its previous place.
pub(super) const PLACE: &str = "its place in its province's icon stack";

/// One naval counter at sea: its window and the refusal standing on it ([`Withheld`]), both
/// destroyed with its source task force.
#[derive(Debug)]
struct Counter {
    instance: Instance,
    /// The column whose last finalize placed this item.
    column: land::ColumnIdentity,
    withheld: Withheld<()>,
    /// Its parts left unknown ([`Unknowns`]).
    unknown: Unknowns,
}

/// The parts of one window an update left unknown, by why, until an update writes them: the
/// window's own memory of them, which its refusal state ([`Withheld`]) cannot be asked for. A
/// drawable part stays in it while a Hide leaves it unseen ([`gui_windows::Update::hidden`]), so a
/// later Show that keeps it ([`gui_windows::Update::kept`]) withholds the window again.
type Unknowns = HashMap<&'static str, Cow<'static, str>>;

/// What an update of a window leaves on its refusal state ([`Withheld`], [`Pool::refuse`]).
pub(super) enum Mark {
    Refuse(Unknown, Cow<'static, str>),
    Rewrote(Rewrote),
}

/// The marks `update` leaves once applied (`applied`, why a call does not apply): a call that
/// does not apply withholds the window until its windows are built again
/// ([`Unknown::UntilRebuilt`]: every such failure is structural, the same on every update);
/// otherwise its calls are all made, every part it wrote is known again, and every part it left
/// unknown withholds the window under `prefix` and why until an update writes it.
pub(super) fn marks(
    update: &gui_windows::Update,
    applied: Result<(), Cow<'static, str>>,
    prefix: &str,
) -> Vec<Mark> {
    if let Err(why) = applied {
        return vec![Mark::Refuse(Unknown::UntilRebuilt, why)];
    }
    let mut marks = vec![Mark::Rewrote(Rewrote::Update)];
    marks.extend(
        update
            .wrote
            .iter()
            .map(|part| Mark::Rewrote(Rewrote::Part(part))),
    );
    marks.extend(
        update
            .unknown
            .iter()
            .map(|(part, why)| Mark::Refuse(Unknown::Part(part), format!("{prefix}{why}").into())),
    );
    marks
}

/// [`marks`] for a window that keeps the parts left unknown in `unknown`: a part an update leaves
/// unknown is remembered and one it writes forgotten; a drawable part it hides no longer withholds
/// the window but is remembered (a Hide replaces no drawable), and one it shows while keeping the
/// drawable withholds it again when that drawable is unknown.
fn window_marks(
    update: &gui_windows::Update,
    applied: Result<(), Cow<'static, str>>,
    prefix: &str,
    unknown: &mut Unknowns,
) -> Vec<Mark> {
    let ok = applied.is_ok();
    let mut left = marks(update, applied, prefix);
    if !ok {
        return left;
    }
    for (part, why) in &update.unknown {
        unknown.insert(*part, format!("{prefix}{why}").into());
    }
    for part in &update.wrote {
        unknown.remove(part);
    }
    left.extend(
        update
            .hidden
            .iter()
            .map(|part| Mark::Rewrote(Rewrote::Part(part))),
    );
    left.extend(update.kept.iter().filter_map(|part| {
        let why = unknown.get(part)?;
        Some(Mark::Refuse(Unknown::Part(part), why.clone()))
    }));
    left
}

/// A pooled mission item whose updates the hour does not know (its icon's grids undecided, or
/// its first build after such an update): the parts its update may leave as they were
/// ([`gui_windows::MISSION_ITEM_KEPT`]) are unknown, by `why`, remembered in `unknown`.
fn unknown_kept(unknown: &mut Unknowns, why: &str) -> Vec<Mark> {
    gui_windows::MISSION_ITEM_KEPT
        .into_iter()
        .map(|part| {
            let why: Cow<'static, str> = why.to_owned().into();
            unknown.insert(part, why.clone());
            Mark::Refuse(Unknown::Part(part), why)
        })
        .collect()
}

/// `marks` left on `withheld` for `key`.
pub(super) fn mark<K: Clone + Eq + std::hash::Hash>(
    withheld: &mut Withheld<K>,
    key: &K,
    marks: Vec<Mark>,
) {
    for mark in marks {
        match mark {
            Mark::Refuse(unknown, why) => withheld.refuse(key.clone(), unknown, why),
            Mark::Rewrote(rewrote) => withheld.rewrote(key, rewrote),
        }
    }
}

/// What a naval base icon's update wrote: its glyph (none hides it), scale and mode, its port
/// items, and the art and flags they drew with. The same again writes nothing new.
struct BaseWritten {
    province: u32,
    glyph: Option<bases::NavalBaseIcon>,
    scale: u32,
    strategic: bool,
    items: Result<Vec<gui_windows::Update>, Cow<'static, str>>,
    art: u64,
    countries: Option<Arc<Countries>>,
    /// Written quiet with the inputs it was written with before ([`base_due`]).
    settled: bool,
    /// The port items its grid had no slot for.
    overflow: usize,
}

/// A base's update this frame ([`base_due`]).
#[derive(Debug, PartialEq)]
enum BaseDue {
    /// Its window and port items stand as they are, and so do the items its grid left out.
    Stands { overflow: usize },
    /// It is written, settled when it was written quiet with the same inputs as last time.
    Write { settled: bool },
}

/// Whether a base stands this frame, from what it last wrote: `quiet` when nothing wrote its
/// window or a port item its grid holds since their tick and none changes on its own, `same` when
/// its update would be given what it last was and its roster is decided. It stands once written twice in a row with the
/// same inputs, the second time quiet: one write can leave what the next identical write changes
/// (a sprite replacement drops the frame written before it), the items take the grid's opacity
/// before the tick that ends the window's fade, and an item's movement ends where its grid's
/// finalization does not put it.
fn base_due(written: Option<&BaseWritten>, quiet: bool, same: bool) -> BaseDue {
    match written {
        Some(written) if quiet && same && written.settled => BaseDue::Stands { overflow: written.overflow },
        _ => BaseDue::Write { settled: quiet && same },
    }
}

impl BaseWritten {
    #[allow(clippy::too_many_arguments, reason = "every input a base icon's update reads")]
    fn same(
        &self,
        province: u32,
        glyph: Option<&bases::NavalBaseIcon>,
        scale: u32,
        strategic: bool,
        items: Result<&[gui_windows::Update], &Cow<'static, str>>,
        art: u64,
        countries: Option<&Arc<Countries>>,
    ) -> bool {
        let countries = match (&self.countries, countries) {
            (Some(held), Some(now)) => Arc::ptr_eq(held, now) || held.same_flags(now),
            (None, None) => true,
            _ => false,
        };
        self.province == province
            && self.glyph.as_ref() == glyph
            && self.scale == scale
            && self.strategic == strategic
            && self.items.as_ref().map(Vec::as_slice) == items
            && self.art == art
            && countries
    }
}

/// Each base written with flags the same as `now`'s holds `now`, so [`BaseWritten::same`] finds
/// them the same by pointer: the flags of each other table it held are compared once a frame.
fn adopt_countries<'a>(written: impl Iterator<Item = &'a mut BaseWritten>, now: Option<&Arc<Countries>>) {
    let Some(now) = now else {
        return;
    };
    let mut compared: Vec<(Arc<Countries>, bool)> = Vec::new();
    for base in written {
        let Some(held) = &base.countries else {
            continue;
        };
        if Arc::ptr_eq(held, now) {
            continue;
        }
        let same = match compared.iter().find(|(table, _)| Arc::ptr_eq(table, held)) {
            Some(&(_, same)) => same,
            None => {
                let same = held.same_flags(now);
                compared.push((Arc::clone(held), same));
                same
            }
        };
        if same {
            base.countries = Some(Arc::clone(now));
        }
    }
}

/// One grid item's window.
#[derive(Debug)]
struct Item {
    instance: Instance,
    naval_convoy: Option<u64>,
    selectable: Option<(bool, Arc<[u64]>)>,
    /// Its task force's owner slot, for the counter filter.
    owner: Option<u32>,
    port_tooltip: Option<viewer_foundation::map::counter_hovers::task_force_port::Metadata>,
    /// Its template: `unit_counter_navy`, or the convoy addition's.
    tiny: bool,
    /// Its parts left unknown ([`Unknowns`]).
    unknown: Unknowns,
}

/// What a grid of an icon's window holds, in the order its code appended them.
type Held_ = Vec<ItemKey>;

/// Every naval map-icon window of one map view.
#[derive(Default)]
pub struct Windows {
    /// The registry and GUI viewport the windows were built from; another rebuilds them.
    built: Option<(NativeWindows, [i32; 2])>,
    combats: Option<Pool<CombatOwner>>,
    bases: Option<Pool<u32>>,
    /// The retained result icons by province; `None` when the
    /// content declares no result window.
    results: Option<Pool<u32>>,
    /// Each live result icon's results from its province's last update in view.
    held_results: HashMap<u32, Vec<u64>>,
    /// The same as [`Self::diverged_combats`] for the result pool.
    diverged_results: Option<Cow<'static, str>>,
    items: HashMap<ItemKey, Item>,
    /// This frame's resources, named for the windows' kept layouts ([`resources_stamp`]).
    stamp: u64,
    /// Each icon's grids (by the grid element's path) and the items they hold.
    grids: HashMap<(IconSlot, String), Held_>,
    /// The refusals standing on the grid items' windows, the module's one refusal state
    /// ([`Withheld`]; the icons' own stand in their pools, [`Pool::refuse`]).
    items_withheld: Withheld<ItemKey>,
    /// The combat icons whose grids an update left undecided, by why: the game may have built
    /// more pooled items in that update (the game allocates the
    /// ones a wrapper lacks, then reuses them), so a mission item first built here afterwards has
    /// the same unknown parts as one that existed ([`unknown_kept`]).
    grown: HashMap<IconSlot, Cow<'static, str>>,
    /// Why the combat pool's slots may no longer be the game's: a request or erasure the view
    /// cannot reproduce (an icon with no decided place never requested, a pass erased because it
    /// cannot be planned, a combat whose resolution is undecided kept, the frame a growth
    /// completes), after which the game's round robin may give an owner another
    /// slot than the game's, with another history. Every slot allocated afterwards is unknown
    /// ([`allocate`]); kept until the windows are built again.
    diverged_combats: Option<Cow<'static, str>>,
    /// The same for the naval base pool (an unanchored base never requested, the pass erased
    /// because it cannot be planned, the frame a growth completes).
    diverged_bases: Option<Cow<'static, str>>,
    /// The combat icons whose pool slot's `bg_btn` frame this view established for their owner
    /// since the slot was allocated to it: by an engaged rotation, or by the engaged index's
    /// carried frame ([`kept_frame`]). Their pool's frame wins over the index's.
    framed: HashSet<CombatOwner>,
    /// The combat icons whose last update left `bg_btn` as their pool slot held it (an
    /// unengaged icon the pool established, or a spill-over icon), counted where they draw.
    pool_frame: HashSet<CombatOwner>,
    /// Each live combat icon's anchor and stack place from the last update that
    /// placed it, read each frame.
    placed: HashMap<CombatOwner, ([f32; 3], u32)>,
    /// What each naval base icon's update last wrote, by slot ([`BaseWritten`]).
    base_written: HashMap<SlotId, BaseWritten>,
    /// Naval counters at sea by their source task force.
    counters: HashMap<u64, Counter>,
    carrier_wings: CarrierViews,
    /// The previous frame's shown unit columns, including columns with only land items.
    columns: BTreeMap<land::ColumnIdentity, usize>,
    /// Last frame's counter membership; a returning item is appended again.
    held_counters: BTreeSet<u64>,
    /// The naval mission icons and their entries ([`naval_missions::MissionView`]).
    pub(super) missions: naval_missions::MissionView,
    pub(super) air_missions: air_missions::View,
    /// The view's clock at the last presented frame.
    last: Option<f32>,
    /// Something ran at the last frame's tick ([`Instance::active`], [`Pool::active`]).
    active: bool,
    /// Nothing ran at the last frame's tick: [`Self::active`] as the tick left it, before the
    /// view takes it.
    settled: bool,
    /// Something it shows changed on its own at the last frame: [`Self::active`] but for an icon a
    /// dirty province kept off screen, which asks only for the pools' passes.
    visible: bool,
    /// An icon of the presented frame is kept off screen by its province's dirty bit alone
    /// ([`admission`]): the frame the bit clears erases it.
    kept_off_screen: bool,
    /// The bases written this frame with a decided roster that do not stand yet ([`update_base`]):
    /// the next frame writes each again, so the view asks for that frame.
    owed: HashSet<SlotId>,
    /// The view's transition clock: the sum of every tick's step, on which the windows' fades and
    /// movements advance.
    clock: f32,
    /// The windows whose fade and movement are unknown ([`TRANSITION`], [`transition_unknown`]).
    settling: HashMap<Registered, Settling>,
    /// The at-sea counter whose instance the pointer state was last carried into.
    pointed: Option<u64>,
    /// The counter the pointer holds and its state, with the presentation time it entered it.
    pub(super) pointer_clock: Option<(Option<(Unit, bool)>, f32)>,
    /// The counters the pointer left, with the time each left, until their Up effects settle.
    pub(super) pointer_left: Vec<(Unit, f32)>,
}

impl std::fmt::Debug for Windows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Windows")
            .field("combats", &self.combats)
            .field("bases", &self.bases)
            .field("items", &self.items.len())
            .field("counters", &self.counters.len())
            .finish()
    }
}

impl Windows {
    pub fn retries(&self) -> Vec<Retry> {
        let mut retries = Vec::new();
        retries.extend(self.combats.as_ref().and_then(Pool::retry));
        retries.extend(self.bases.as_ref().and_then(Pool::retry));
        retries.extend(self.results.as_ref().and_then(Pool::retry));
        retries.extend(self.missions.retry());
        retries.extend(self.air_missions.retry());
        retries
    }

    /// Whether a window changed on its own at the last frame, cleared: the view presents another
    /// frame while one does, as the game ticks every window each frame.
    /// Keeps the view presenting: a land plate's pointer state still changes on the clock.
    pub(super) fn keep_active(&mut self) {
        self.active = true;
        self.visible = true;
    }

    /// Whether something it shows changes on its own, not cleared.
    pub fn running_visibly(&self) -> bool {
        self.visible || self.air_missions.running_visibly()
    }

    /// Presents the next frames for the pools' passes alone ([`PoolView::set_quiet`]).
    pub fn set_quiet(&mut self, quiet: bool) {
        self.air_missions.set_quiet(quiet);
    }

    pub fn take_active(&mut self) -> bool {
        let air = self.air_missions.take_active();
        std::mem::take(&mut self.active) || air
    }

    /// Builds the pools for `windows` in `viewport`, with `resources` (their registry's), when
    /// they were built from another registry or viewport (a new document's content, a resized
    /// window: the GUI is made again).
    fn build(
        &mut self,
        windows: &NativeWindows,
        viewport: [i32; 2],
        resources: &dyn native_gui::Resources,
    ) -> Result<(), String> {
        if self
            .built
            .as_ref()
            .is_some_and(|(built, at)| built.same(windows) && *at == viewport)
        {
            return Ok(());
        }
        *self = Self::default();
        // then the window hidden; neither
        // icon type's constructor writes its window.
        self.combats = Some(Pool::new(
            windows.naval_combat(),
            viewport,
            MapIconPool::Combat,
            Box::new(|_, _| Ok(())),
            resources,
        )?);
        self.bases = match windows.naval_base() {
            Some(base) => Some(Pool::new(
                base,
                viewport,
                MapIconPool::Base,
                Box::new(|_, root| base_controls(root).map(|_| ())),
                resources,
            )?),
            None => None,
        };
        // the window, then its `bg_btn`
        // found as a button; a content whose `bg_btn` is none builds no icon,
        // and the result pass counts every shown icon it omits.
        self.results = match windows.naval_result() {
            Some(result) if result.find(native::Kind::Button, "bg_btn").is_some() => {
                Some(Pool::new(
                    result,
                    viewport,
                    MapIconPool::CombatResults,
                    Box::new(|_, _| Ok(())),
                    resources,
                )?)
            }
            _ => None,
        };
        self.built = Some((windows.clone(), viewport));
        Ok(())
    }
}

/// What one prepared hour's naval map-icon windows ask of the presented view, decided without
/// any window.
#[derive(Default)]
pub(super) struct Plan {
    /// The combat icon pass below `NAVAL_COMBAT_DISTANCE_CUTOFF`; `None` when it
    /// does not run, which erases every combat icon.
    pub(super) combat: Option<CombatPass>,
    /// The naval base icons (below `NAVAL_BASE_DISTANCE_CUTOFF`); `None` when they are not
    /// drawn, which erases them.
    pub(super) bases: Option<BasePass>,
    /// Why the combat icon pass runs but is not planned (its window does not lay out): every
    /// combat icon's pooled items are then unknown ([`unknown_items`]).
    pub(super) combats_undecided: Option<Cow<'static, str>>,
    /// Why the naval base pass runs but is not planned (the base census or window): every base
    /// icon's port items are then unknown ([`unknown_items`]).
    pub(super) bases_undecided: Option<Cow<'static, str>>,
    /// Combat pass admission and per-province icon erasure state.
    pub(super) results: Option<ResultPass>,
    /// Why the result pass is not planned (its window does not lay out): its icons' requests and
    /// erasures are then not reproduced.
    pub(super) results_undecided: Option<Cow<'static, str>>,
    /// The task forces the hour holds, when the naval pass ran ([`settle_counters`]).
    pub(super) forces: Option<Forces>,
    /// The hour's countries, which the windows' flags resolve through.
    pub(super) countries: Option<Arc<Countries>>,
    /// The naval mission icons' regions ([`naval_missions::MissionPass`]).
    pub(super) missions: naval_missions::MissionPass,
    pub(super) carrier_sea: HashMap<u64, CarrierGrid>,
    pub(super) carrier_ports: BTreeMap<(u32, usize), CarrierGrid>,
}

/// The naval result icons' decisions for one camera.
pub(super) struct ResultPass {
    /// The hour's retained results, with their completeness.
    pub(super) tables: Arc<naval_markers::ResultTables>,
    /// The provinces whose combat vector holds a combat, and whether that census is complete,
    /// which a sea province's anchor reads ([`naval_markers::result_anchor`]).
    pub(super) raised: BTreeSet<u32>,
    pub(super) vectors_whole: bool,
    /// The result window's frame size.
    pub(super) size: [i32; 2],
}

/// The task forces one hour's TaskForces census records, and whether it is complete.
pub(super) struct Forces {
    pub(super) recorded: BTreeSet<u64>,
    pub(super) complete: bool,
}

/// The combat pass's decisions.
pub(super) struct CombatPass {
    /// The icons the passes place and update, in their order.
    pub(super) icons: Vec<CombatIcon>,
    /// Combats with an empty side: the game erases their icons when their province's
    /// pass reaches them.
    pub(super) gated: BTreeSet<u64>,
    /// Every combat the hour holds: an icon of any other is empty, unless
    /// [`Self::live_undecided`].
    pub(super) live: BTreeSet<u64>,
    /// Why [`Self::live`] may lack a combat that still resolves (an incomplete `NavalCombats`
    /// census): an icon of a combat it lacks is then kept and omitted ([`combat_resolves`]).
    pub(super) live_undecided: Option<Cow<'static, str>>,
    /// Each detailed combat's naval-HQ frame, for an icon the hour does not place again, or why
    /// it (or every one) is undecided.
    pub(super) hq: Result<BTreeMap<u64, Result<bool, String>>, String>,
    /// The window's frame size, which places each icon and tests the viewport.
    pub(super) size: [i32; 2],
}

/// One placed combat icon.
pub(super) struct CombatIcon {
    pub(super) owner: CombatOwner,
    /// Its anchor and its place in its province's stack ([`PLACE`]), when the hour decides them.
    pub(super) place: Option<([f32; 3], u32)>,
    /// The update after the base ([`gui_windows::combat_calls`]), with [`PLACE`].
    pub(super) update: gui_windows::Update,
    /// The game passed its early returns and its writes are known: the
    /// grids are emptied and then filled with [`Self::grids`].
    pub(super) rotates: bool,
    /// What the game appends, grid by grid, in its order.
    pub(super) grids: Vec<GridFill>,
    /// Where its `bg_btn` frame comes from (`None` for an omitted icon, whose frame is unknown
    /// when it rotates): an engaged rotation sets it; an unengaged rotating icon keeps an
    /// established frame, else [`kept_frame`] uses the index's frame or the native initial
    /// fallback. A spill-over icon keeps its pool slot's.
    pub(super) frame: Option<naval_markers::FrameSource>,
    /// An icon the hour places but cannot decide, counted with its cause by the preparation: its
    /// withheld window is not counted again where it would draw.
    pub(super) omitted: bool,
}

impl CombatIcon {
    /// Parts withheld after an unavailable combat rotation: bars, grids, background
    /// frame and placement. Grid items retain their own unknown state; later writes
    /// repair only the parts they reach.
    pub(super) fn omitted(
        owner: CombatOwner,
        place: Option<([f32; 3], u32)>,
        rotates: bool,
        cause: &str,
    ) -> Self {
        let mut update = gui_windows::Update::default();
        let mut unknown = |part| update.unknown.push((part, cause.to_owned()));
        if rotates {
            unknown(gui_windows::BG_FRAME);
            unknown(gui_windows::PROGRESS);
            unknown(gui_windows::GRIDS);
        }
        unknown(gui_windows::FRAME_BTN);
        match place {
            Some(_) => update.wrote.push(PLACE),
            None => update.unknown.push((PLACE, cause.to_owned())),
        }
        Self {
            owner,
            place,
            update,
            rotates: false,
            grids: Vec::new(),
            frame: None,
            omitted: true,
        }
    }
}

/// One grid's appends.
#[derive(Debug)]
pub(super) struct GridFill {
    /// The grid's path in the combat window.
    pub(super) grid: String,
    /// `Some(side)` for a side's task-force tiles, `None` for a convoy addition.
    pub(super) side: Option<usize>,
    /// Each appended item's update; `Err` (the gap it is counted under) draws nothing, though
    /// the game appends it.
    pub(super) items: Vec<Result<gui_windows::Update, Cow<'static, str>>>,
}

/// The naval base icons and their port items.
pub(super) struct BasePass {
    /// Every naval base province and its port anchor.
    pub(super) bases: Vec<(u32, [f32; 3])>,
    /// Each base's port items this hour, when the unit pass fills them (`None`:
    /// Empty every box, the camera being coarse or the
    /// viewer in a peace conference), or why every base's items are undecided.
    pub(super) ports: Result<Option<PortItems>, Cow<'static, str>>,
    /// Decided glyphs for the bases this hour.
    pub(super) glyphs: Vec<NavalBaseIcon>,
    /// The base window's frame size.
    pub(super) size: [i32; 2],
    /// Naval base provinces the content places no port anchor for, whose icons the view never
    /// requests though the game does.
    pub(super) unanchored: usize,
}

/// Project the icon anchor into the integer GUI viewport, then apply the
/// window's authored anchor and rounding before viewport admission.
pub(super) fn icon_point(
    inputs: &CounterInputs<'_>,
    anchor: [f32; 3],
    ordinal: u32,
    size: [i32; 2],
) -> [i32; 2] {
    let screen = inputs
        .camera
        .project_map_icon(
            anchor,
            inputs.viewport,
            gui_resources_viewport(inputs.viewport, inputs.ui_scale),
        )
        .unwrap_or(MapIconProjection::BEHIND)
        .position;
    let mut point = point(screen, size, 0x12);
    let offset = i32::from_ne_bytes(ordinal.to_ne_bytes()).wrapping_mul(size[1]);
    point[1] = point[1].wrapping_add(offset);
    point
}

/// the window's rectangle meets `[0, w] x [0, h]` of
/// the integer GUI viewport the point was projected in, edges included
///
pub(super) fn inside(inputs: &CounterInputs<'_>, point: [i32; 2], size: [i32; 2]) -> bool {
    rectangle_inside(
        point,
        size,
        gui_resources_viewport(inputs.viewport, inputs.ui_scale),
    )
}

/// Counted for each owner the pool could not give a window this frame (the game
/// found none free and started the pool's growth; the owner asks again the next frame).
/// Why a pool's slots may no longer be the game's after it is exhausted: the frame on which the
/// game's worker task completes the growth is in no
/// recording, so which frame's requests get the new slots may differ.
pub(super) const GROWTH_FRAME: &str = "the frame the pool's growth completes is not recorded";

/// Sequential update laps keep short/zero phases visible instead of censoring them at
/// PROFILE's ordinary span threshold. Nested detail still uses the ordinary spans.
struct UpdateLaps(Option<std::time::Instant>);

impl UpdateLaps {
    fn new() -> Self {
        Self(viewer_foundation::profile_writer::profiling().then(std::time::Instant::now))
    }

    fn lap(&mut self, name: &'static str) {
        if let Some(last) = self.0 {
            let now = std::time::Instant::now();
            viewer_foundation::profiling::record_lap(name, now.duration_since(last));
            self.0 = Some(now);
        }
    }
}

/// The presented frame: the owners, the manager's pass, the overrides and grids, the naval
/// counters at sea (`plates`: each counter's flotilla, local placement and graphical column;
/// `columns`: every shown unit column), one tick of every window, and their layouts and draws:
/// base glyphs into `bases`, port items into `counters`, and combat icons into `combats`.
#[allow(clippy::too_many_arguments)]
pub(super) fn present(
    inputs: &CounterInputs<'_>,
    plan: &Plan,
    plates: &[(&naval::Flotilla, &Countries, &land::AtSeaPlate)],
    columns: &[Arc<land::ColumnPlacement>],
    time: f32,
    pointer: Option<CounterPointer>,
    windows: &mut Windows,
    bases: &mut Batch,
    counters: &mut Batch,
    combats: &mut Batch,
    report: &mut CounterReport,
) -> io::Result<()> {
    let update_profile = viewer_foundation::profiling::span("dc_naval_update");
    let mut update_laps = UpdateLaps::new();
    let native = inputs.windows;
    let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
    let flags = gui_windows::window_flags(inputs, plan.countries.as_deref());
    let resources = gui_windows::gui_assets(inputs, flags.as_ref());
    if let Err(why) = windows.build(native, viewport, &resources) {
        *report
            .naval_window_notes
            .entry(format!("the naval map-icon windows are not built: {why}").into())
            .or_default() += 1;
        update_laps.lap("dc_nu_setup");
        return Ok(());
    }
    windows.stamp = resources_stamp(inputs, plan.countries.as_deref());
    viewer_foundation::map::redraws::note(
        if windows.settled { "naval.settled" } else { "naval.active" },
        1,
    );
    // The view's single presentation clock and the frame's step.
    let frame = frame(&mut windows.last, time);
    update_laps.lap("dc_nu_setup");
    // PreparedCounters::draw advances the shared province schedule before these consumers.
    let schedule = inputs
        .schedule
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    update_laps.lap("dc_nu_lock");
    let combat_window = native.naval_combat();
    combat_frame(inputs, plan, windows, combat_window, &schedule, report)?;
    update_laps.lap("dc_nu_combat");
    if let Some(base) = native.naval_base() {
        base_frame(inputs, plan, windows, base, &schedule, report)?;
    }
    update_laps.lap("dc_nu_base");
    if let Some(result) = native.naval_result() {
        result_frame(inputs, plan, windows, result, &schedule, report)?;
    }
    update_laps.lap("dc_nu_result");
    let mut mission_view = std::mem::take(&mut windows.missions);
    let missions = naval_missions::update(
        inputs,
        &plan.missions,
        &mut mission_view,
        &resources,
        &schedule,
        report,
    );
    for (slot, held) in mission_view.take_grid_held() {
        if let Err(why) = reach_items(windows, IconSlot::Mission(slot), &held) {
            *report.naval_mission_gaps.entry(why.into()).or_default() += 1;
        }
    }
    if let Some(template) = &missions {
        for &(region, slot) in &mission_view.updated_grids {
            if let Some(instance) = mission_view.window(naval_missions::MissionWindow::Icon(slot)) {
                let fills = plan
                    .missions
                    .regional
                    .get(&region)
                    .map_or(&[][..], |data| data.grids.as_slice());
                if let Err(why) = fill_grids_in(
                    &resources,
                    windows,
                    instance,
                    template.root,
                    IconSlot::Mission(slot),
                    fills,
                    report,
                ) {
                    *report
                        .naval_mission_gaps
                        .entry(format!("regional naval grids: {why}").into())
                        .or_default() += 1;
                }
            }
        }
    }
    windows.missions = mission_view;
    drop(schedule);
    update_laps.lap("dc_nu_missions");
    let navy = native.navy();
    let mut carrier_wings = std::mem::take(&mut windows.carrier_wings);
    carrier_wings.retain(|key, _| match key {
        CarrierKey::AtSea(source) => {
            windows.counters.contains_key(source)
                || plan
                    .forces
                    .as_ref()
                    .is_some_and(|forces| forces.recorded.contains(source))
        }
        CarrierKey::Port(item) => windows.items.contains_key(item),
    });
    update_laps.lap("dc_nu_carrier_retain");
    for view in carrier_wings.values_mut() {
        view.tick(time);
    }
    update_laps.lap("dc_nu_carrier_tick");
    // Port wrappers are pooled with their own unit item, never with another
    // base or with the at-sea representation of the same task force.
    let port_owners: HashMap<_, _> = windows
        .bases
        .as_ref()
        .map(|pool| {
            pool.live()
                .map(|(province, slot, _)| (slot, *province))
                .collect()
        })
        .unwrap_or_default();
    update_laps.lap("dc_nu_port_map");
    for (key, item) in &mut windows.items {
        let (IconSlot::Base(slot), ItemPool::Port) = (key.icon, key.pool) else {
            continue;
        };
        let offered = port_owners
            .get(&slot)
            .and_then(|province| plan.carrier_ports.get(&(*province, key.index)));
        if offered.is_some() || carrier_wings.contains_key(&CarrierKey::Port(*key)) {
            let view = carrier_wings.entry(CarrierKey::Port(*key)).or_default();
            if let Err(why) = view.update(
                0,
                None,
                &mut item.instance,
                navy,
                "wings_box",
                offered,
                inputs,
                plan.countries.as_deref(),
                &resources,
            ) {
                *report
                    .icon_gui_notes
                    .entry(format!("Port carrier wing grid: {why}"))
                    .or_default() += 1;
            }
        }
    }
    update_laps.lap("dc_nu_port_carriers");
    // Counters at sea: their source task forces' windows, destroyed with their source.
    if let Some(forces) = &plan.forces {
        settle_counters(windows, forces);
    }
    update_laps.lap("dc_nu_sea_settle");
    let mut at_sea = Vec::with_capacity(plates.len());
    for &(flotilla, countries, plate) in plates {
        let update = counter_calls(inputs, navy, flotilla, countries, report);
        at_sea.push((flotilla, plate, update));
    }
    update_laps.lap("dc_nu_sea_calls");
    let mut placed_at_sea = Vec::with_capacity(at_sea.len());
    for (flotilla, plate, update) in at_sea {
        let visible = windows.columns.contains_key(&plate.column.identity);
        match counter_window(
            windows,
            navy,
            flotilla.source,
            plate,
            visible,
            &update,
            &resources,
        ) {
            Ok(()) => {
                let key = CarrierKey::AtSea(flotilla.source);
                let offered = plan.carrier_sea.get(&flotilla.source);
                if offered.is_some() || carrier_wings.contains_key(&key) {
                    let view = carrier_wings.entry(key).or_default();
                    let counter = windows.counters.get_mut(&flotilla.source).unwrap();
                    if let Err(why) = view.update(
                        0,
                        None,
                        &mut counter.instance,
                        navy,
                        "wings_box",
                        offered,
                        inputs,
                        plan.countries.as_deref(),
                        &resources,
                    ) {
                        *report
                            .icon_gui_notes
                            .entry(format!("At-sea carrier wing grid: {why}"))
                            .or_default() += 1;
                    }
                }
                placed_at_sea.push((flotilla, plate));
            }
            Err(reason) => {
                skipped_counter(
                    windows,
                    flotilla.source,
                    &format!("its update stopped: {reason}"),
                );
                *report
                    .navy_counter_gaps
                    .entry(format!("{NAVY_WINDOW} is not drawn: {reason}").into())
                    .or_default() += 1;
            }
        }
    }
    update_laps.lap("dc_nu_sea_apply");
    let shown: BTreeMap<_, _> = columns
        .iter()
        .map(|column| (column.identity, column.count))
        .collect();
    // The game reaches every item when a shown column is hidden.
    for counter in windows.counters.values_mut().filter(|counter| {
        windows.columns.contains_key(&counter.column) && !shown.contains_key(&counter.column)
    }) {
        if let Err(why) = counter.instance.hide(navy, navy) {
            counter
                .withheld
                .refuse((), Unknown::UntilRebuilt, why.into());
        }
    }
    windows.columns = shown;
    windows.held_counters = plates
        .iter()
        .map(|(flotilla, ..)| flotilla.source)
        .collect();
    // A task force the unit column refuses this hour is updated by the game all the same.
    let updated: BTreeSet<u64> = placed_at_sea
        .iter()
        .map(|(flotilla, _)| flotilla.source)
        .collect();
    for (reason, forces) in &report.unplaced_task_forces {
        for &source in forces.iter().filter(|source| !updated.contains(source)) {
            skipped_counter(
                windows,
                source,
                &format!("its task force is not placed: {reason}"),
            );
        }
    }
    // The map's hit test found the pointer's task force: its window's `count_bg` takes the state.
    let held = pointer.and_then(|pointer| {
        let (flotilla, _) = placed_at_sea.iter().find(|(flotilla, _)| {
            flotilla.members.iter().any(|&id| {
                viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce.unit(id)
                    == pointer.unit
            })
        })?;
        Some((flotilla.source, pointer.down))
    });
    let plate = navy.find(native::Kind::Button, "count_bg");
    if held.is_some() && plate.is_none() {
        *report
            .navy_counter_gaps
            .entry(format!("{NAVY_WINDOW} has no count_bg for the pointer's state").into())
            .or_default() += 1;
    }
    let pointed = held.and_then(|(source, down)| {
        Some((
            source,
            native_gui::Pointed {
                path: plate?.path.clone(),
                down,
            },
        ))
    });
    let pointed_source = pointed.as_ref().map(|(source, _)| *source);
    if windows.pointed != pointed_source
        && let Some(counter) = windows
            .pointed
            .and_then(|source| windows.counters.get_mut(&source))
    {
        counter.instance.point(None, &[]);
    }
    if let Some((source, state)) = &pointed
        && let Some(counter) = windows.counters.get_mut(source)
    {
        counter.instance.point(Some(state.clone()), &[]);
    }
    windows.pointed = pointed_source;
    update_laps.lap("dc_nu_bookkeeping");
    drop(update_profile);
    let tick_profile = viewer_foundation::profiling::span("dc_naval_tick");
    tick(
        windows,
        native,
        missions.as_ref(),
        &resources,
        frame,
        report,
    );
    drop(tick_profile);
    let markers_profile = viewer_foundation::profiling::span("dc_naval_markers");
    if let Some(missions) = &missions {
        naval_missions::draw(
            inputs,
            &plan.missions,
            &windows.missions,
            windows,
            missions,
            &resources,
            combats,
            report,
        )?;
    }
    draw_combats(
        inputs,
        plan,
        windows,
        combat_window,
        &resources,
        combats,
        report,
    )?;
    if let Some(result) = native.naval_result() {
        draw_results(inputs, plan, windows, result, &resources, combats, report)?;
    }
    drop(markers_profile);
    let ports_profile = viewer_foundation::profiling::span("dc_ports");
    if let Some(base) = native.naval_base() {
        draw_ports(
            inputs,
            plan,
            windows,
            &mut carrier_wings,
            frame,
            base,
            &resources,
            bases,
            counters,
            report,
        )?;
    }
    drop(ports_profile);
    let _at_sea_profile = viewer_foundation::profiling::span("dc_naval_at_sea");
    // At unit-stack priority, each plate retains its column's icon order.
    // Transparent overlap still follows the presented paint order.
    for (flotilla, plate) in placed_at_sea {
        let Some(counter) = windows.counters.get(&flotilla.source) else {
            continue;
        };
        if let Some(why) = counter.withheld.why(&()) {
            *report
                .navy_counter_gaps
                .entry(why.to_owned().into())
                .or_default() += 1;
            continue;
        }
        let first = counters.quads.len();
        let layout_pointer = pointed
            .as_ref()
            .filter(|(source, _)| *source == flotilla.source)
            .map(|(_, state)| native_gui::Pointer {
                at: [f32::NAN; 2],
                pressed: state.down.then(|| state.path.clone()),
                over: (!state.down).then(|| state.path.clone()),
                clip_at: None,
                drawn: None,
                drawn_boxes: Vec::new(),
            });
        let layout = match counter
            .instance
            .layout_with_regions(navy, &resources, layout_pointer, true)
        {
            Ok(Some(layout)) => layout,
            Ok(None) => continue,
            Err(refusals) => {
                count_refused(&mut report.navy_counter_gaps, NAVY_WINDOW, &refusals);
                continue;
            }
        };
        count_noted(
            &mut report.navy_counter_gaps,
            NAVY_WINDOW,
            &layout.diagnostics,
        );
        // Move drawing; the viewport remains the logical parent.
        let local = counter.instance.position();
        let place = [0, 1].map(|axis| plate.column.grid[axis].wrapping_add(local[axis]));
        let composed =
            if let Some(view) = carrier_wings.get_mut(&CarrierKey::AtSea(flotilla.source)) {
                let drawn = viewer_foundation::map::icon_windows::Drawn {
                    owner: 0,
                    origin: plate.column.screen,
                    layout: layout.clone(),
                };
                view.compose_held(
                    0,
                    &drawn,
                    NAVY_WINDOW,
                    "wings_box",
                    inputs,
                    &resources,
                    frame,
                    hovered_layer(
                        plate.column.priority,
                        plate.column.hovered,
                        plate.ordinal,
                        0,
                    ),
                    Some((&plate.column.holder, place)),
                    IconFamily::Columns,
                    report,
                )
                .unwrap_or_else(|why| {
                    *report
                        .icon_gui_notes
                        .entry(format!("At-sea carrier wing grid: {why}"))
                        .or_default() += 1;
                    air_grid::Composed::empty(layout.holders.len())
                })
            } else {
                air_grid::Composed::empty(layout.holders.len())
            };
        let parts = draw_parts::place(&plate.column.holder, place, &layout, &composed.parts);
        let drawn = gui_windows::draw_held(
            &plate.column.holder,
            parts,
            plate.column.screen,
            &resources,
            counters,
            (plate.column.priority, 0),
        )?;
        let shown = inputs.display.naval_plate(flotilla.owner);
        if !shown {
            counters.quads.truncate(first);
        }
        match drawn {
            Ok(()) => {
                if shown && layout.pointed.is_some() {
                    report.pointed = pointer.map(|pointer| pointer.unit);
                }
                composed.commit(report);
                report.task_force_hovers.extend(naval_hovers::presented(
                    navy,
                    &layout,
                    &plate.column.holder,
                    place,
                    plate.column.screen,
                    &resources,
                    flotilla,
                    report.unit_hits.len(),
                    &mut report.icon_gui_notes,
                ));
                // The `count_bg` as the window laid it out, where the column placed the window.
                let scale = plate.column.holder.scale;
                let window_at = [0, 1].map(|axis| {
                    plate.column.screen[axis] as f32
                        + plate.column.holder.origin[axis]
                        + scale * place[axis] as f32
                });
                let count_bg = navy
                    .find(native::Kind::Button, "count_bg")
                    .filter(|element| {
                        element
                            .button()
                            .is_some_and(|b| !b.always_transparent && b.enabled)
                    })
                    .and_then(|element| {
                        layout
                            .regions
                            .iter()
                            .rev()
                            .find(|region| region.path == element.path)
                    })
                    .filter(|region| !region.disabled)
                    .and_then(|region| {
                        // A pass-through plate takes no pointer; an alpha-tested one is refused.
                        match region.hit.as_ref().map(|hit| hit.form) {
                            Some(viewer_foundation::map::native_gui::Hit::Through) => None,
                            Some(viewer_foundation::map::native_gui::Hit::Alpha) => {
                                *report.navy_counter_gaps.entry("count_bg tests its sprite's alpha (transparencecheck), which a counter plate's hit does not; its pointer state is not drawn".into()).or_default() += 1;
                                None
                            }
                            _ => Some([
                                window_at[0] + scale * region.rect[0],
                                window_at[1] + scale * region.rect[1],
                                scale * region.rect[2],
                                scale * region.rect[3],
                            ]),
                        }
                    });
                report
                    .unit_hits
                    .push(viewer_foundation::map::counter_hovers::unit_hits::Hit {
                        ours: matches!(&flotilla.side, Ok(Allegiance::Ours)),
                        port: false,
                        plate: count_bg,
                        rect: [
                            plate.column.screen[0] as f32
                                + plate.column.holder.origin[0]
                                + plate.column.holder.scale * place[0] as f32,
                            plate.column.screen[1] as f32
                                + plate.column.holder.origin[1]
                                + plate.column.holder.scale * place[1] as f32,
                            layout.size[0] as f32 * plate.column.holder.scale,
                            layout.size[1] as f32 * plate.column.holder.scale,
                        ],
                        kind: viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce,
                        members: Arc::clone(&flotilla.members),
                        layer: hovered_layer(
                            plate.column.priority,
                            plate.column.hovered,
                            plate.ordinal,
                            0,
                        ),
                    });
            }
            Err(refusal) => {
                count_refused(&mut report.navy_counter_gaps, NAVY_WINDOW, &[refusal]);
            }
        }
        let offset = hovered_layer(
            plate.column.priority,
            plate.column.hovered,
            plate.ordinal,
            0,
        ) - layer(plate.column.priority, 0, 0);
        for quad in &mut counters.quads[first..] {
            quad.layer += offset;
            quad.anchor = plate.column.anchor;
        }
    }
    for view in carrier_wings.values_mut() {
        let running = view.take_running();
        windows.active |= running;
        windows.visible |= running;
    }
    windows.settled = !windows.active;
    windows.carrier_wings = carrier_wings;
    Ok(())
}

/// A view window in the GUI transition list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Registered {
    Icon(IconSlot),
    Item(ItemKey),
    Counter(u64),
    Mission(naval_missions::MissionWindow),
}

/// Tick every registered window's transition in reverse registration order.
/// An ended fade or hide movement immediately hides its children, including
/// retained freed windows.
/// Whether an icon the pass admitted (kept off screen when `off_screen`) asks for the next frame:
/// only while its province `cleans` ([`viewer_foundation::map::province_schedule::Schedule::cleans`]),
/// the one off-screen icon a later pass erases. An icon kept by a province dirty in view is kept by
/// every later pass alike.
fn keeps_asking(off_screen: bool, cleans: bool) -> bool {
    off_screen && cleans
}

pub(super) fn tick(
    windows: &mut Windows,
    native: &NativeWindows,
    missions: Option<&naval_missions::MissionWindows<'_>>,
    resources: &dyn native_gui::Resources,
    tick: Tick,
    report: &mut CounterReport,
) {
    let navy = native.navy();
    let combat = native.naval_combat();
    let base = native.naval_base();
    let result_template = native.naval_result();
    let tiny = native.tiny();
    let item_template = |item: &Item| if item.tiny { tiny } else { Some(navy) };
    // The mission windows the frame's update left with an unknown fade and movement (a slot
    // allocated after a divergence), on the clock of the frame's owners, before this tick.
    for (window, why) in windows.missions.take_unsettled() {
        transition_unknown(windows, Registered::Mission(window), &why, Marked::Once);
    }
    windows.clock += tick.dt;
    if !tick_due(windows, missions.is_some()) {
        viewer_foundation::map::redraws::note("naval.tick_not_due", 1);
        settle_after(windows, native, missions.is_some(), Vec::new(), report);
        return;
    }
    // The windows whose tick failed, with why: a pool's phase fails as a whole.
    let mut failed: Vec<(Registered, String)> = Vec::new();
    fn failing(
        failed: &mut Vec<(Registered, String)>,
        windows: impl FnOnce() -> Vec<Registered>,
        result: Result<(), String>,
    ) {
        if let Err(why) = result {
            failed.extend(windows().into_iter().map(|window| (window, why.clone())));
        }
    }
    // Every drawable's clock.
    if let Some(pool) = windows.combats.as_mut() {
        let result = pool.tick_clocks(combat, resources, tick);
        failing(&mut failed, || pool_windows(pool, IconSlot::Combat), result);
    }
    if let (Some(pool), Some(base)) = (windows.bases.as_mut(), base) {
        let result = pool.tick_clocks(base, resources, tick);
        failing(&mut failed, || pool_windows(pool, IconSlot::Base), result);
    }
    if let (Some(pool), Some(template)) = (windows.results.as_mut(), result_template) {
        let result = pool.tick_clocks(template, resources, tick);
        failing(&mut failed, || pool_windows(pool, IconSlot::Result), result);
    }
    for (key, item) in &mut windows.items {
        if let Some(template) = item_template(item) {
            let result = item.instance.tick_clocks(template, resources, tick);
            failing(&mut failed, || vec![Registered::Item(*key)], result);
        }
    }
    for (source, counter) in &mut windows.counters {
        let result = counter.instance.tick_clocks(navy, resources, tick);
        failing(&mut failed, || vec![Registered::Counter(*source)], result);
    }
    if let Some(missions) = missions {
        let result = windows.missions.tick_clocks(missions, resources, tick);
        failing(&mut failed, || mission_windows(&windows.missions), result);
    }
    // GUI transitions, last registered first.
    let mut order: Vec<(Registration, Registered)> = Vec::new();
    if let Some(pool) = &windows.combats {
        order.extend(
            pool.registrations()
                .map(|(at, slot)| (at, Registered::Icon(IconSlot::Combat(slot)))),
        );
    }
    if let (Some(pool), Some(_)) = (&windows.bases, base) {
        order.extend(
            pool.registrations()
                .map(|(at, slot)| (at, Registered::Icon(IconSlot::Base(slot)))),
        );
    }
    if let (Some(pool), Some(_)) = (&windows.results, result_template) {
        order.extend(
            pool.registrations()
                .map(|(at, slot)| (at, Registered::Icon(IconSlot::Result(slot)))),
        );
    }
    order.extend(
        windows
            .items
            .iter()
            .map(|(key, item)| (item.instance.registration(), Registered::Item(*key))),
    );
    order.extend(windows.counters.iter().map(|(source, counter)| {
        (
            counter.instance.registration(),
            Registered::Counter(*source),
        )
    }));
    if missions.is_some() {
        order.extend(
            windows
                .missions
                .registrations()
                .map(|(at, window)| (at, Registered::Mission(window))),
        );
    }
    order.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    for (_, registered) in order {
        match registered {
            Registered::Icon(icon) => {
                let (result, held) = match icon {
                    IconSlot::Mission(_) => continue,
                    IconSlot::Combat(slot) => match windows.combats.as_mut() {
                        Some(pool) => {
                            (pool.tick_transitions_of(combat, slot), pool.take_held(slot))
                        }
                        None => continue,
                    },
                    IconSlot::Base(slot) => match (windows.bases.as_mut(), base) {
                        (Some(pool), Some(base)) => {
                            (pool.tick_transitions_of(base, slot), pool.take_held(slot))
                        }
                        _ => continue,
                    },
                    IconSlot::Result(slot) => match (windows.results.as_mut(), result_template) {
                        (Some(pool), Some(template)) => (
                            pool.tick_transitions_of(template, slot),
                            pool.take_held(slot),
                        ),
                        _ => continue,
                    },
                };
                failing(&mut failed, || vec![registered], result);
                // A failed reach leaves the items it did not reach where they were.
                let result = reach_items(windows, icon, &held);
                failing(&mut failed, || vec![registered], result);
            }
            Registered::Item(key) => {
                if let Some(item) = windows.items.get_mut(&key)
                    && let Some(template) = item_template(item)
                {
                    failing(
                        &mut failed,
                        || vec![registered],
                        item.instance.tick_transitions(template),
                    );
                }
            }
            Registered::Counter(source) => {
                if let Some(counter) = windows.counters.get_mut(&source) {
                    failing(
                        &mut failed,
                        || vec![registered],
                        counter.instance.tick_transitions(navy),
                    );
                }
            }
            Registered::Mission(window) => {
                if let Some(missions) = missions {
                    let result = windows.missions.tick_transitions_of(missions, window);
                    failing(&mut failed, || vec![registered], result);
                    for (slot, held) in windows.missions.take_grid_held() {
                        let result = reach_items(windows, IconSlot::Mission(slot), &held);
                        failing(&mut failed, || vec![registered], result);
                    }
                }
            }
        }
    }
    // The draw.
    if let Some(pool) = windows.combats.as_mut() {
        let result = pool.tick_draws(combat, resources);
        failing(&mut failed, || pool_windows(pool, IconSlot::Combat), result);
        count_noted(
            &mut report.naval_window_notes,
            NAVAL_COMBAT_WINDOW,
            &pool.take_diagnostics(),
        );
    }
    if let (Some(pool), Some(base)) = (windows.bases.as_mut(), base) {
        let result = pool.tick_draws(base, resources);
        failing(&mut failed, || pool_windows(pool, IconSlot::Base), result);
        count_noted(
            &mut report.naval_window_notes,
            NAVAL_BASE_WINDOW,
            &pool.take_diagnostics(),
        );
    }
    if let (Some(pool), Some(template)) = (windows.results.as_mut(), result_template) {
        let result = pool.tick_draws(template, resources);
        failing(&mut failed, || pool_windows(pool, IconSlot::Result), result);
        count_noted(
            &mut report.naval_window_notes,
            NAVAL_RESULT_WINDOW,
            &pool.take_diagnostics(),
        );
    }
    for (key, item) in &mut windows.items {
        if let Some(template) = item_template(item) {
            let result = item.instance.tick_draws(template, resources);
            failing(&mut failed, || vec![Registered::Item(*key)], result);
        }
    }
    for (source, counter) in &mut windows.counters {
        let result = counter.instance.tick_draws(navy, resources);
        failing(&mut failed, || vec![Registered::Counter(*source)], result);
    }
    if let Some(missions) = missions {
        let result = windows.missions.tick_draws(missions, resources, report);
        failing(&mut failed, || mission_windows(&windows.missions), result);
    }
    settle_after(windows, native, missions.is_some(), failed, report);
}

/// What a tick leaves behind, whether it ticked the windows or found none due: the windows whose
/// transition is unknown settled on the view's clock, and whether anything of the view still runs.
fn settle_after(
    windows: &mut Windows,
    native: &NativeWindows,
    missions: bool,
    failed: Vec<(Registered, String)>,
    report: &mut CounterReport,
) {
    if let Some((_, why)) = failed.first() {
        *report
            .naval_window_notes
            .entry(format!("a naval map-icon window does not tick: {why}").into())
            .or_default() += 1;
    }
    // A window whose tick failed may stand mid-transition where the game's does not.
    for (window, why) in failed {
        transition_unknown(windows, window, &format!("its window does not tick: {why}"), Marked::EveryFrame);
    }
    settle(windows, native);
    let asks = windows.settling.values().any(Settling::asks);
    for entry in windows.settling.values_mut() {
        entry.marked = None;
    }
    // The next frame is asked for only by what it can still change, each reason apart: a base owed
    // its second write, an unknown transition whose wait can run ([`Settling::asks`]), and a
    // window that runs and is drawn ([`runs_drawn`]).
    let owed = !std::mem::take(&mut windows.owed).is_empty();
    let held = |key: &ItemKey| runs_drawn(windows, key);
    windows.visible = owed
        || asks
        || windows.combats.as_ref().is_some_and(Pool::active)
        || windows.bases.as_ref().is_some_and(Pool::active)
        || windows.results.as_ref().is_some_and(Pool::active)
        || windows.grids.values().flatten().any(held)
        || windows
            .counters
            .values()
            .any(|counter| counter.instance.active())
        || (missions && windows.missions.active());
    windows.active = std::mem::take(&mut windows.kept_off_screen) || windows.visible;
    windows.settled = !windows.active;
}

/// Whether the grid item `key` (one a grid holds) runs and is drawn: it changes on its own and no
/// refusal withholds its draw. Only such an item asks for frames; it, or one recovering a
/// transition, keeps its base written ([`update_base`]).
fn runs_drawn(windows: &Windows, key: &ItemKey) -> bool {
    windows.items_withheld.why(key).is_none()
        && windows.items.get(key).is_some_and(|item| item.instance.active())
}

/// Whether a tick would change any window of the view: one that changes on its own, or one a call
/// changed since its last tick ([`Instance::idle`]). A tick of any other window only stamps the
/// frame's clock on it, which nothing a settled window draws reads
/// ([`native_gui::Shading::changing`]), so a frame with none due ticks none.
fn tick_due(windows: &Windows, missions: bool) -> bool {
    windows.combats.as_ref().is_some_and(Pool::tick_due)
        || windows.bases.as_ref().is_some_and(Pool::tick_due)
        || windows.results.as_ref().is_some_and(Pool::tick_due)
        || windows.items.values().any(|item| !item.instance.idle())
        || windows
            .counters
            .values()
            .any(|counter| !counter.instance.idle())
        || (missions && windows.missions.tick_due())
}

/// Every mission window, as the transition list names it: a phase of the mission view fails as a
/// whole.
fn mission_windows(missions: &naval_missions::MissionView) -> Vec<Registered> {
    missions
        .registrations()
        .map(|(_, window)| Registered::Mission(window))
        .collect()
}

/// Every window of `pool`, in use or not, as the transition list names it.
fn pool_windows<K: Clone + Eq + std::hash::Hash>(
    pool: &Pool<K>,
    icon: fn(SlotId) -> IconSlot,
) -> Vec<Registered> {
    pool.registrations()
        .map(|(_, slot)| Registered::Icon(icon(slot)))
        .collect()
}

/// A naval counter at sea's update.
fn counter_calls(
    inputs: &CounterInputs<'_>,
    navy: &native::Element,
    flotilla: &naval::Flotilla,
    countries: &Countries,
    report: &mut CounterReport,
) -> gui_windows::Update {
    let art = gui_windows::CounterArt {
        assets: inputs.assets,
        countries,
    };
    let flags = gui_windows::window_flags(inputs, Some(countries));
    let resources = gui_windows::gui_assets(inputs, flags.as_ref());
    let ctx = gui_windows::NavyContext {
        settings: inputs.settings,
        selected: inputs.selected,
        defines: &inputs.content.defines,
        art: &art,
        gui: &resources,
    };
    gui_windows::navy_calls(
        &ctx,
        navy,
        &gui_windows::NavyState {
            flotilla,
            countries,
        },
        &mut report.navy_counter_gaps,
    )
}

/// Place at-sea counter items in the source's units grid, preserving its
/// item order and finalized grid positions.
fn counter_window(
    windows: &mut Windows,
    navy: &native::Element,
    source: u64,
    plate: &land::AtSeaPlate,
    visible: bool,
    update: &gui_windows::Update,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    let viewport = windows
        .built
        .as_ref()
        .map(|(_, viewport)| *viewport)
        .ok_or("the windows are not built")?;
    let fresh = !windows.counters.contains_key(&source);
    let appended = !windows.held_counters.contains(&source)
        || windows
            .counters
            .get(&source)
            .is_some_and(|counter| counter.column != plate.column.identity);
    let finalized = windows
        .columns
        .get(&plate.column.identity)
        .is_none_or(|&count| count != plate.column.count);
    let counter = match windows.counters.entry(source) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            let instance = Instance::construct(navy, viewport, resources)?;
            entry.insert(Counter {
                instance,
                column: plate.column.identity,
                withheld: Withheld::default(),
                unknown: Unknowns::new(),
            })
        }
    };
    // Show visible items before finalizing the grid; hide other items afterward.
    if appended && visible {
        counter.instance.gui_show(navy, navy)?;
    }
    if fresh || finalized || counter.column != plate.column.identity {
        // Add the constructor offset saved by the game.
        counter.instance.set_position(navy, navy, plate.local)?;
    }
    if !visible {
        counter.instance.gui_show(navy, navy)?;
    }
    counter.column = plate.column.identity;
    let applied = gui_windows::apply(&mut counter.instance, navy, &update.calls, resources)
        .map_err(|why| {
            format!("{NAVY_WINDOW} is not drawn: its update does not apply: {why}").into()
        });
    let prefix = format!("{NAVY_WINDOW} is not drawn: ");
    let left = window_marks(update, applied, &prefix, &mut counter.unknown);
    mark(&mut counter.withheld, &(), left);
    windows.held_counters.insert(source);
    Ok(())
}

/// The combat icons' owners, the manager's pass and each updated icon's override and grids.
fn combat_frame(
    inputs: &CounterInputs<'_>,
    plan: &Plan,
    windows: &mut Windows,
    window: &native::Element,
    schedule: &Schedule,
    report: &mut CounterReport,
) -> io::Result<()> {
    let Some(mut pool) = windows.combats.take() else {
        return Ok(());
    };
    // Requests and erasures this frame the view cannot reproduce.
    let diverged = match &plan.combat {
        None => plan.combats_undecided.clone(),
        Some(pass) => pass
            .icons
            .iter()
            .find(|icon| {
                icon.place.is_none() && schedule.update(icon.owner.0) == Some(Update::InView)
            })
            .map(|icon| {
                Cow::Owned(format!(
                    "an icon of combat {} with no decided place is not requested",
                    icon.owner.1
                ))
            })
            .or_else(|| {
                pool.live()
                    .find(|(owner, ..)| combat_resolves(pass, owner.1).is_none())
                    .map(|(owner, ..)| {
                        Cow::Owned(format!(
                            "combat {}'s icon is kept though whether it resolves is undecided",
                            owner.1
                        ))
                    })
            }),
    };
    if diverged.is_some() {
        windows.diverged_combats = diverged;
    }
    let result = (|| -> Result<(), String> {
        let mut erased = combat_owners(
            windows,
            &mut pool,
            window,
            plan.combat.as_ref(),
            schedule,
            report,
        )?;
        let size = plan.combat.as_ref().map_or([0; 2], |pass| pass.size);
        let placed = &windows.placed;
        let mut kept_off_screen = false;
        let pass = pool.update(window, |owner, _| {
            let Some(combat) = &plan.combat else {
                return Admission::Empty;
            };
            if combat_resolves(combat, owner.1) == Some(false) {
                return Admission::Empty;
            }
            // Every live combat icon was placed before its request.
            let Some(&(anchor, ordinal)) = placed.get(owner) else {
                return Admission::Outside;
            };
            let (admitted, off_screen) = admission(
                schedule.dirty(owner.0),
                icon_point(inputs, anchor, ordinal, size),
                |point| inside(inputs, point, size),
            );
            // Kept off screen by a province dirty in view, the icon is kept by every later pass.
            kept_off_screen |= keeps_asking(off_screen, schedule.cleans(owner.0));
            admitted
        })?;
        windows.kept_off_screen |= kept_off_screen;
        for gone in pass.erased {
            windows.placed.remove(&gone.owner);
            windows.framed.remove(&gone.owner);
            erased.push((gone.slot, gone.held));
        }
        for (slot, held) in erased {
            reach_items(windows, IconSlot::Combat(slot), &held)?;
        }
        let Some(combat) = &plan.combat else {
            return Ok(());
        };
        let flags = gui_windows::window_flags(inputs, plan.countries.as_deref());
        let resources = gui_windows::gui_assets(inputs, flags.as_ref());
        windows.pool_frame.clear();
        for (owner, slot) in pass.updated {
            let (update, rotation) = planned_update(combat, owner, window, &resources);
            let frame = combat
                .icons
                .iter()
                .find(|icon| icon.owner == owner)
                .and_then(|icon| icon.frame);
            let update = match frame {
                Some(naval_markers::FrameSource::Kept(kept)) => Cow::Owned(kept_frame(
                    windows.framed.contains(&owner),
                    owner,
                    kept,
                    window,
                    update.into_owned(),
                    report,
                )),
                _ => update,
            };
            if update.wrote.contains(&gui_windows::BG_FRAME) {
                // An initial fallback must let a later index result establish history.
                if matches!(
                    frame,
                    Some(naval_markers::FrameSource::Kept(
                        Kept::Fresh | Kept::Unobserved | Kept::Undecided | Kept::Unindexed
                    ))
                ) {
                    windows.framed.remove(&owner);
                } else {
                    windows.framed.insert(owner);
                }
            } else if update
                .unknown
                .iter()
                .any(|(part, _)| *part == gui_windows::BG_FRAME)
            {
                windows.framed.remove(&owner);
            } else if matches!(
                frame,
                Some(naval_markers::FrameSource::Kept(_) | naval_markers::FrameSource::Spillover)
            ) {
                windows.pool_frame.insert(owner);
            }
            update_combat(
                windows, &mut pool, window, slot, &update, rotation, &resources, report,
            )?;
        }
        Ok(())
    })();
    let slots: Vec<IconSlot> = pool
        .registrations()
        .map(|(_, slot)| IconSlot::Combat(slot))
        .collect();
    let undecided = match (&result, &plan.combats_undecided) {
        (Err(why), _) => Some(format!("{NAVAL_COMBAT_WINDOW} does not update: {why}")),
        (Ok(()), Some(why)) => Some(why.to_string()),
        (Ok(()), None) => None,
    };
    // A pass that stops leaves the icons it did not reach as they were: withheld until the
    // windows are built again, as a call that does not apply is structural.
    if let Err(why) = &result {
        for icon in &slots {
            if let IconSlot::Combat(slot) = *icon {
                pool.refuse(
                    slot,
                    Unknown::UntilRebuilt,
                    format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not update: {why}").into(),
                );
            }
        }
    }
    windows.combats = Some(pool);
    if let Some(why) = undecided {
        unknown_pool(windows, slots, &why);
    }
    if let Err(why) = result {
        *report
            .naval_window_notes
            .entry(format!("{NAVAL_COMBAT_WINDOW} does not update: {why}").into())
            .or_default() += 1;
    }
    Ok(())
}

/// Apply combat-icon ownership through scheduled provinces in province order.
/// The prepared pass determines requests and erasures.
fn combat_owners(
    windows: &mut Windows,
    pool: &mut Pool<CombatOwner>,
    window: &native::Element,
    combat: Option<&CombatPass>,
    schedule: &Schedule,
    report: &mut CounterReport,
) -> Result<Vec<(SlotId, Vec<Held>)>, String> {
    let mut erased: Vec<(SlotId, Vec<Held>)> = Vec::new();
    let mut provinces: BTreeSet<u32> = pool.live().map(|(owner, ..)| owner.0).collect();
    if let Some(pass) = combat {
        provinces.extend(pass.icons.iter().map(|icon| icon.owner.0));
    }
    for province in provinces {
        let Some(update) = schedule.update(province) else {
            continue;
        };
        let pass = combat.filter(|_| update == Update::InView);
        let erasing: Vec<CombatOwner> = pool
            .live()
            .map(|(owner, ..)| *owner)
            .filter(|(owner, combat)| {
                *owner == province && pass.is_none_or(|pass| pass.gated.contains(combat))
            })
            .collect();
        for owner in erasing {
            let slot = pool.slot_of(&owner);
            let held = pool.erase(window, &owner)?;
            if let Some(slot) = slot {
                erased.push((slot, held));
            }
            windows.placed.remove(&owner);
            windows.framed.remove(&owner);
        }
        if let Some(pass) = pass {
            for icon in pass.icons.iter().filter(|icon| icon.owner.0 == province) {
                // An icon whose place the hour cannot decide keeps its window where it
                // stood; one it has none for is not given one (counted with its cause).
                let Some(place) = icon.place else {
                    continue;
                };
                windows.placed.insert(icon.owner, place);
                let diverged = windows.diverged_combats.clone();
                let slot = allocate(
                    windows,
                    pool,
                    icon.owner,
                    IconSlot::Combat,
                    &COMBAT_KEPT,
                    diverged,
                );
                if slot.is_none() {
                    *report
                        .naval_window_notes
                        .entry(POOL_EXHAUSTED.into())
                        .or_default() += 1;
                    windows.diverged_combats = Some(GROWTH_FRAME.into());
                }
            }
        }
    }
    Ok(erased)
}

/// Whether `combat` still resolves : `None` when the census that
/// would say is not whole and lacks it.
fn combat_resolves(pass: &CombatPass, combat: u64) -> Option<bool> {
    if pass.live.contains(&combat) {
        Some(true)
    } else if pass.live_undecided.is_some() {
        None
    } else {
        Some(false)
    }
}

/// A window's fade and movement : unknown
/// for a window whose history the view does not reproduce, and for one whose tick failed. No
/// update writes it. Every site goes through the one mechanism, [`transition_unknown`], and the
/// one lift, [`settle`] (the census is in `classes/lifecycle.md`):
///
/// - a pool slot newly allocated after the view's allocations may have left the game's
///   ([`allocate`]);
/// - each item of an icon whose update the view does not reproduce ([`unknown_items`]);
/// - an item of either kind, mission item or convoy addition, first constructed after such an
///   update, which the game may have built or reused then ([`Windows::grown`], [`update_item`]);
/// - a counter at sea whose update the view skips ([`skipped_counter`]);
/// - a mission icon's slot newly allocated after the mission pool's allocations may have left the
///   game's, the icon and each of its entries (`MissionView::allocated`, taken at the tick);
/// - a window whose tick fails ([`tick`]), mission windows included.
///
/// It is known again once the view's own window has been out of a transition for the longest
/// its registered windows can run ([`Instance::longest_transition`]), when whatever the game's
/// window was doing has ended too.
pub(super) const TRANSITION: &str = "the window's fade and movement";

/// One window whose [`TRANSITION`] is unknown.
#[derive(Debug)]
struct Settling {
    /// The time on [`Windows::clock`] since which the view's own window has been out of a
    /// transition, or since the last event that made the part unknown.
    since: f32,
    why: Cow<'static, str>,
    /// Its refusal stands on the window's refusal state ([`settle`] puts it there).
    standing: bool,
    /// What made the part unknown at this frame, when something did ([`transition_unknown`]).
    marked: Option<Marked>,
}

/// What makes a window's [`TRANSITION`] unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Marked {
    /// An event of this frame alone: a slot allocated after a divergence, an item built into a
    /// grown pool. Its wait runs on the frames after it.
    Once,
    /// An update the view cannot decide or apply, made again every frame while that holds. Its
    /// wait starts only once it stops.
    EveryFrame,
}

impl Settling {
    /// Whether its wait asks for the next frame: not when this frame marked it only by what
    /// marks it every frame, which no frame advances.
    fn asks(&self) -> bool {
        self.marked != Some(Marked::EveryFrame)
    }
}

/// The template of the view's window `registered`, and its instance, when it still exists.
fn registered_window<'w, 'n>(
    windows: &'w Windows,
    native: &'n NativeWindows,
    registered: Registered,
) -> Option<(&'w Instance, &'n native::Element)> {
    match registered {
        Registered::Icon(IconSlot::Mission(slot)) => {
            let template = native.gui_type(naval_missions::MISSION_WINDOW.as_bytes())?;
            Some((
                windows
                    .missions
                    .window(naval_missions::MissionWindow::Icon(slot))?,
                template,
            ))
        }
        Registered::Icon(IconSlot::Combat(slot)) => Some((
            windows.combats.as_ref()?.instance(slot)?,
            native.naval_combat(),
        )),
        Registered::Icon(IconSlot::Base(slot)) => Some((
            windows.bases.as_ref()?.instance(slot)?,
            native.naval_base()?,
        )),
        Registered::Icon(IconSlot::Result(slot)) => Some((
            windows.results.as_ref()?.instance(slot)?,
            native.naval_result()?,
        )),
        Registered::Item(key) => {
            let item = windows.items.get(&key)?;
            let template = if item.tiny {
                native.tiny()?
            } else {
                native.navy()
            };
            Some((&item.instance, template))
        }
        Registered::Counter(source) => {
            Some((&windows.counters.get(&source)?.instance, native.navy()))
        }
        Registered::Mission(window) => {
            let missions = naval_missions::MissionWindows::find(native).ok()?;
            let template = match window {
                naval_missions::MissionWindow::Icon(_) => missions.root,
                naval_missions::MissionWindow::Entry(..) => missions.entry,
            };
            Some((windows.missions.window(window)?, template))
        }
    }
}

/// The one mechanism by which a window's [`TRANSITION`] becomes unknown, by `why`: a new event
/// restarts the wait from the view's clock now. [`settle`] refuses the window at the frame's tick,
/// before anything draws, and lifts it; a window whose registered windows have no transition is
/// left alone there.
fn transition_unknown(windows: &mut Windows, window: Registered, why: &str, cause: Marked) {
    let clock = windows.clock;
    let why: Cow<'static, str> = format!("{why} ({TRANSITION} is unknown)").into();
    let entry = windows.settling.entry(window).or_insert(Settling {
        since: clock,
        why: why.clone(),
        standing: false,
        marked: None,
    });
    entry.since = clock;
    entry.why = why;
    entry.marked = match entry.marked {
        Some(Marked::Once) => Some(Marked::Once),
        _ => Some(cause),
    };
}

/// At each tick, after its transitions: every window whose [`TRANSITION`] is unknown
/// ([`transition_unknown`]) has its refusal stand on its window's refusal state; one in a
/// transition ([`Instance::transitioning`]) restarts its wait, and one out of a transition for
/// the longest its registered windows can run ([`Instance::longest_transition`]) on the view's
/// clock is known again. A window with no transition is known at once, and one no longer built is
/// forgotten.
fn settle(windows: &mut Windows, native: &NativeWindows) {
    let clock = windows.clock;
    let mut settling = std::mem::take(&mut windows.settling);
    let mut refuse = Vec::new();
    let mut known = Vec::new();
    settling.retain(|&window, entry| {
        let Some((instance, template)) = registered_window(windows, native, window) else {
            return false;
        };
        // A template the instance was not built from cannot say: the part stays unknown.
        let longest = instance
            .longest_transition(template)
            .unwrap_or(f32::INFINITY);
        if longest <= 0.0 {
            if entry.standing {
                known.push(window);
            }
            return false;
        }
        if !entry.standing {
            entry.standing = true;
            refuse.push((window, entry.why.clone()));
        }
        if instance.transitioning() {
            entry.since = clock;
        } else if clock - entry.since >= longest {
            known.push(window);
            return false;
        }
        true
    });
    windows.settling = settling;
    for (window, why) in refuse {
        let unknown = Unknown::Part(TRANSITION);
        match window {
            Registered::Icon(IconSlot::Mission(slot)) => {
                windows
                    .missions
                    .refuse(naval_missions::MissionWindow::Icon(slot), unknown, why)
            }
            Registered::Icon(IconSlot::Combat(slot)) => {
                if let Some(pool) = windows.combats.as_mut() {
                    pool.refuse(slot, unknown, format!("icon omitted: {why}").into());
                }
            }
            Registered::Icon(IconSlot::Base(slot)) => {
                if let Some(pool) = windows.bases.as_mut() {
                    pool.refuse(slot, unknown, format!("icon omitted: {why}").into());
                }
            }
            Registered::Icon(IconSlot::Result(slot)) => {
                if let Some(pool) = windows.results.as_mut() {
                    pool.refuse(slot, unknown, format!("icon omitted: {why}").into());
                }
            }
            Registered::Item(key) => windows.items_withheld.refuse(key, unknown, why),
            Registered::Counter(source) => {
                if let Some(counter) = windows.counters.get_mut(&source) {
                    counter.withheld.refuse((), unknown, why);
                }
            }
            Registered::Mission(window) => windows.missions.refuse(window, unknown, why),
        }
    }
    for window in known {
        match window {
            Registered::Icon(IconSlot::Mission(slot)) => windows
                .missions
                .known(naval_missions::MissionWindow::Icon(slot), TRANSITION),
            Registered::Icon(IconSlot::Combat(slot)) => {
                if let Some(pool) = windows.combats.as_mut() {
                    pool.known(slot, TRANSITION);
                }
            }
            Registered::Icon(IconSlot::Base(slot)) => {
                if let Some(pool) = windows.bases.as_mut() {
                    pool.known(slot, TRANSITION);
                }
            }
            Registered::Icon(IconSlot::Result(slot)) => {
                if let Some(pool) = windows.results.as_mut() {
                    pool.known(slot, TRANSITION);
                }
            }
            Registered::Item(key) => windows.items_withheld.known(&key, TRANSITION),
            Registered::Counter(source) => {
                if let Some(counter) = windows.counters.get_mut(&source) {
                    counter.withheld.known(&(), TRANSITION);
                }
            }
            Registered::Mission(window) => windows.missions.known(window, TRANSITION),
        }
    }
}

/// `owner`'s icon from `pool` ([`Pool::request`]). A slot newly allocated after the view's
/// allocations may have left the game's (`diverged`, [`Windows::diverged_combats`]) may hold
/// another owner's history there: its items are unknown ([`unknown_items`]) and so are the
/// parts of the icon an update may keep (`kept`: for a combat icon the bars, the grids and
/// `bg_btn`'s frame, which an unengaged or non-rotating update does not write), and its window's
/// fade and movement ([`transition_unknown`]).
fn allocate<K: Clone + Eq + std::hash::Hash>(
    windows: &mut Windows,
    pool: &mut Pool<K>,
    owner: K,
    icon: fn(SlotId) -> IconSlot,
    kept: &[&'static str],
    diverged: Option<Cow<'static, str>>,
) -> Option<SlotId> {
    let new = pool.slot_of(&owner).is_none();
    let slot = pool.request(owner)?;
    if let (true, Some(why)) = (new, diverged) {
        let why = format!("its pool slot may not be the game's: {why}");
        for part in kept {
            pool.refuse(
                slot,
                Unknown::Part(part),
                format!("icon omitted: {why}").into(),
            );
        }
        transition_unknown(windows, Registered::Icon(icon(slot)), &why, Marked::Once);
        unknown_items(windows, icon(slot), &why, Marked::Once);
    }
    Some(slot)
}

/// The parts of a combat icon an update may leave as its window held them.
const COMBAT_KEPT: [&str; 3] = [
    gui_windows::BG_FRAME,
    gui_windows::PROGRESS,
    gui_windows::GRIDS,
];

/// The `bg_btn` frame of an unengaged rotating icon stays as it was. The pool
/// slot's frame wins when the view established it for this owner (`framed`),
/// without a write. Otherwise the engaged index's [`Kept::Frame`] establishes
/// the frame through `update` and is counted as carried ([`icon_frames::CARRIED`]).
/// Any other answer uses the binding's initial frame and counts its reason as
/// not from history.
///
/// [`Kept`]: crate::icon_frames::Kept
/// [`Kept::Frame`]: crate::icon_frames::Kept::Frame
/// [`icon_frames::CARRIED`]: crate::icon_frames::CARRIED
fn kept_frame(
    framed: bool,
    owner: CombatOwner,
    kept: Kept,
    window: &native::Element,
    mut update: gui_windows::Update,
    report: &mut CounterReport,
) -> gui_windows::Update {
    use viewer_foundation::icon_frames::NAVAL_NO_HISTORY;
    if framed {
        return update;
    }
    let frame = match kept {
        Kept::Frame(frame) => frame.wrapping_add(1) as i32,
        // The binding sets the background frame to one, overriding the authored frame.
        _ => 1,
    };
    let mut writes = gui_windows::Writes::new(window);
    writes.frame(native::Kind::Button, "bg_btn", frame);
    update.calls.extend(writes.finish());
    update.wrote.push(gui_windows::BG_FRAME);
    report
        .combat_limits
        .entry(("Naval", kept.limitation(NAVAL_NO_HISTORY)))
        .or_default()
        .insert(owner.1);
    update
}

/// What the manager's update of `owner`'s icon makes this frame, with the grids
/// its rotation refills: the hour's own update when the hour places the icon, decided or omitted
/// ([`CombatIcon::omitted`], its unknown parts withholding it, never taken for an icon the hour
/// does not place). An icon the hour no longer places keeps what it showed; the game still
/// writes its HQ frame, and its rotation returns before its writes when its province is not its
/// combat's (an icon of its own province is placed again).
fn planned_update<'p>(
    combat: &'p CombatPass,
    owner: CombatOwner,
    window: &native::Element,
    resources: &dyn native_gui::Resources,
) -> (Cow<'p, gui_windows::Update>, Option<&'p [GridFill]>) {
    if let Some(icon) = combat.icons.iter().find(|icon| icon.owner == owner) {
        return (
            Cow::Borrowed(&icon.update),
            icon.rotates.then_some(icon.grids.as_slice()),
        );
    }
    // A combat the incomplete census lacks may still resolve and be updated, rotation included:
    // the icon is kept, omitted.
    if let (None, Some(why)) = (combat_resolves(combat, owner.1), &combat.live_undecided) {
        return (
            Cow::Owned(CombatIcon::omitted(owner, None, true, why).update),
            None,
        );
    }
    let hq = match &combat.hq {
        Ok(frames) => frames.get(&owner.1).cloned().unwrap_or_else(|| {
            Err(format!(
                "the NavalCombatDetails table has no row for combat {}",
                owner.1
            ))
        }),
        Err(cause) => Err(cause.clone()),
    };
    let update = gui_windows::combat_calls(
        window,
        &gui_windows::CombatState { rotation: None, hq },
        resources,
    );
    (Cow::Owned(update), None)
}

/// Apply one admitted combat icon's ordered update after common placement
/// and visibility. Unknown parts withhold its draw until later writes repair
/// them; unknown grid updates also affect their pooled items.
#[allow(clippy::too_many_arguments)]
fn update_combat(
    windows: &mut Windows,
    pool: &mut Pool<CombatOwner>,
    window: &native::Element,
    slot: SlotId,
    update: &gui_windows::Update,
    rotation: Option<&[GridFill]>,
    resources: &dyn native_gui::Resources,
    report: &mut CounterReport,
) -> Result<(), String> {
    let icon = IconSlot::Combat(slot);
    let instance = pool.get(slot).ok_or("an updated icon has no window")?;
    // The base's own `gui_show` reached the grids it holds.
    let held = instance.take_held();
    reach_items(windows, icon, &held)?;
    let applied = gui_windows::apply(instance, window, &update.calls, resources).map_err(|why| {
        format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not update: {why}").into()
    });
    let held = instance.take_held();
    reach_items(windows, icon, &held)?;
    for mark in marks(update, applied, "") {
        match mark {
            Mark::Refuse(unknown, why) => pool.refuse(slot, unknown, why),
            Mark::Rewrote(rewrote) => pool.rewrote(slot, rewrote),
        }
    }
    if let Some((_, why)) = update
        .unknown
        .iter()
        .find(|(part, _)| *part == gui_windows::GRIDS)
    {
        unknown_items(windows, icon, why, Marked::EveryFrame);
    }
    if let Some(fills) = rotation {
        let instance = pool.get(slot).ok_or("an updated icon has no window")?;
        fill_grids(resources, windows, instance, window, slot, fills, report)?;
    }
    Ok(())
}

/// An unavailable pooled-item update withholds its unknown parts until later
/// writes repair them. Kept sprites, supply and other partial writes retain
/// their own validity; pool growth can leave additional items unknown.
fn unknown_items(windows: &mut Windows, icon: IconSlot, why: &str, cause: Marked) {
    let grids_undecided = |name: &str| -> Cow<'static, str> {
        format!("{name} is not drawn: its icon's grids are not reproduced ({why})").into()
    };
    windows.grown.insert(icon, grids_undecided(NAVY_WINDOW));
    let Windows {
        items,
        items_withheld,
        ..
    } = windows;
    let mut keys = Vec::new();
    for (key, item) in items.iter_mut().filter(|(key, _)| key.icon == icon) {
        let name = if item.tiny {
            TINY_UNIT_COUNTER_WINDOW
        } else {
            NAVY_WINDOW
        };
        let why = grids_undecided(name);
        let mut left = vec![Mark::Refuse(Unknown::Update, why.clone())];
        if !item.tiny {
            left.extend(unknown_kept(&mut item.unknown, &why));
        }
        mark(items_withheld, key, left);
        keys.push((*key, why));
    }
    // What the game's update did to the item's window (its Show or Hide, and so its fade) is
    // unknown with it.
    for (key, why) in keys {
        transition_unknown(windows, Registered::Item(key), &why, cause);
    }
}

/// [`unknown_items`] for every window of a pool, live or free, whose updates this frame does not
/// reproduce: a pass the hour cannot plan though it runs, or one that stops part-way (a call
/// that does not apply, structural and the same every frame).
fn unknown_pool(windows: &mut Windows, slots: Vec<IconSlot>, why: &str) {
    for icon in slots {
        unknown_items(windows, icon, why, Marked::EveryFrame);
    }
}

/// A naval counter at sea (by its `source`) whose update the view does not make this frame though
/// the game does (its task force refused by the unit column, or its window's update stopped): its
/// next update writes it again ([`Unknown::Update`]) and the parts that update may keep
/// ([`unknown_kept`]); the Show or Hide the skipped update may have made leaves its window's fade
/// and movement unknown too ([`TRANSITION`]).
fn skipped_counter(windows: &mut Windows, source: u64, why: &str) {
    let Some(counter) = windows.counters.get_mut(&source) else {
        return;
    };
    let why: Cow<'static, str> = format!("{NAVY_WINDOW} is not drawn: {why}").into();
    let mut left = vec![Mark::Refuse(Unknown::Update, why.clone())];
    left.extend(unknown_kept(&mut counter.unknown, &why));
    mark(&mut counter.withheld, &(), left);
    transition_unknown(windows, Registered::Counter(source), &why, Marked::EveryFrame);
}

/// Why a counter at sea whose source an incomplete TaskForces census lacks is kept unknown.
const COUNTER_SOURCE_UNRECORDED: &str =
    "its task force's row is absent from an incomplete TaskForces census";

/// The counters at sea against the hour's task forces: one whose source a complete census lacks
/// is destroyed with it; one whose source an incomplete census lacks may still stand and be
/// updated by the game, so it is kept and its update skipped ([`skipped_counter`]).
fn settle_counters(windows: &mut Windows, forces: &Forces) {
    if forces.complete {
        windows
            .counters
            .retain(|source, _| forces.recorded.contains(source));
        return;
    }
    let unrecorded: Vec<u64> = windows
        .counters
        .keys()
        .filter(|source| !forces.recorded.contains(source))
        .copied()
        .collect();
    for source in unrecorded {
        skipped_counter(windows, source, COUNTER_SOURCE_UNRECORDED);
    }
}

/// Propagate window show and hide calls through its retained grids and lists
/// to every held item in order.
fn reach_items(windows: &mut Windows, icon: IconSlot, held: &[Held]) -> Result<(), String> {
    let _profile = viewer_foundation::profiling::span("dc_nu_held");
    let Some((native, _)) = windows.built.clone() else {
        return Ok(());
    };
    for reached in held {
        let Some(keys) = windows.grids.get(&(icon, reached.path.clone())).cloned() else {
            continue;
        };
        for key in keys {
            let Some(item) = windows.items.get_mut(&key) else {
                continue;
            };
            let template = item_template(&native, item.tiny)?;
            if reached.shown {
                item.instance.gui_show(template, template)?;
            } else {
                item.instance.hide(template, template)?;
            }
        }
    }
    Ok(())
}

/// The template of a grid item's window: the convoy addition's, or `unit_counter_navy`.
fn item_template(native: &NativeWindows, tiny: bool) -> Result<&native::Element, String> {
    if tiny {
        native
            .tiny()
            .ok_or_else(|| format!("no {TINY_UNIT_COUNTER_WINDOW} window"))
    } else {
        Ok(native.navy())
    }
}

/// Construct a pooled item against the GUI viewport, then apply its ordered
/// update. Named failures leave the affected parts unknown until a later
/// update writes them.
fn update_item(
    windows: &mut Windows,
    key: ItemKey,
    template: &native::Element,
    tiny: bool,
    update: Result<&gui_windows::Update, &Cow<'static, str>>,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    let viewport = windows
        .built
        .as_ref()
        .map(|(_, viewport)| *viewport)
        .ok_or("the windows are not built")?;
    // An item first built after an update that left its icon's grids undecided may be one the
    // game built or reused then ([`Windows::grown`]): its fade and movement are unknown, and a
    // mission item's retained parts too. Only its construction inherits them.
    let grown = windows.grown.get(&key.icon).cloned();
    let mut constructed = false;
    let mut left = Vec::new();
    let item = match windows.items.entry(key) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            let mut instance = Instance::construct(template, viewport, resources)?;
            let setup = if tiny {
                gui_windows::convoy_setup(template)
            } else {
                gui_windows::mission_item_setup(template)
            };
            gui_windows::apply(&mut instance, template, &setup, resources)?;
            let mut unknown = Unknowns::new();
            if let (Some(why), false) = (&grown, tiny) {
                left.extend(unknown_kept(&mut unknown, why));
            }
            constructed = true;
            entry.insert(Item {
                instance,
                selectable: None,
                owner: None,
                port_tooltip: None,
                naval_convoy: None,
                tiny,
                unknown,
            })
        }
    };
    let name = if tiny {
        TINY_UNIT_COUNTER_WINDOW
    } else {
        NAVY_WINDOW
    };
    left.extend(match update {
        Ok(update) => {
            item.naval_convoy = update.naval_convoy;
            item.selectable.clone_from(&update.selectable);
            item.owner = update.owner;
            item.port_tooltip.clone_from(&update.port_tooltip);
            let applied =
                gui_windows::apply(&mut item.instance, template, &update.calls, resources).map_err(
                    |why| format!("{name} is not drawn: its update does not apply: {why}").into(),
                );
            let prefix = format!("{name} is not drawn: ");
            window_marks(update, applied, &prefix, &mut item.unknown)
        }
        Err(gap) => vec![Mark::Refuse(Unknown::Update, gap.clone())],
    });
    mark(&mut windows.items_withheld, &key, left);
    if let (Some(why), true) = (grown, constructed) {
        transition_unknown(windows, Registered::Item(key), &why, Marked::Once);
    }
    Ok(())
}

/// Prepare item layout before the frame attaches its window for drawing.
/// Retained layout and refusal state remain per item.
fn read_grids(
    instance: &Instance,
    window: &native::Element,
    name: &str,
    resources: &dyn native_gui::Resources,
) -> (Option<Layout>, Option<String>) {
    let _profile = viewer_foundation::profiling::span("dc_nu_grid_layout");
    match instance.layout(window, resources, None) {
        Ok(laid) => (laid, None),
        Err(refusals) => (
            None,
            Some(
                refusals
                    .iter()
                    .map(|refusal| refused_cause(name, refusal))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        ),
    }
}

/// An item appended while its icon's grids are not read (`unread`, [`read_grids`]): whether the
/// append showed it and where finalize placed it are unknown, so it is refused until its next
/// update reads them ([`Unknown::Update`]), counted where it would draw.
fn unread_grid(windows: &mut Windows, key: ItemKey, tiny: bool, unread: Option<&str>) {
    let Some(unread) = unread else {
        return;
    };
    let name = if tiny {
        TINY_UNIT_COUNTER_WINDOW
    } else {
        NAVY_WINDOW
    };
    windows.items_withheld.refuse(
        key,
        Unknown::Update,
        format!("{name} is not drawn: the grid it is appended to is not read: {unread}").into(),
    );
}

/// Clear both side grids, detaching their previous items. Update and append
/// the prepared replacements in side and item order.
fn fill_grids(
    resources: &dyn native_gui::Resources,
    windows: &mut Windows,
    instance: &Instance,
    window: &native::Element,
    slot: SlotId,
    fills: &[GridFill],
    report: &mut CounterReport,
) -> Result<(), String> {
    fill_grids_in(
        resources,
        windows,
        instance,
        window,
        IconSlot::Combat(slot),
        fills,
        report,
    )
}

fn fill_grids_in(
    resources: &dyn native_gui::Resources,
    windows: &mut Windows,
    instance: &Instance,
    window: &native::Element,
    icon: IconSlot,
    fills: &[GridFill],
    report: &mut CounterReport,
) -> Result<(), String> {
    let (native, viewport) = windows.built.clone().ok_or("the windows are not built")?;
    // The icon as it stands after its calls: which grids are shown (a hidden grid is no holder)
    // and their content.
    let (laid, unread) = read_grids(instance, window, NAVAL_COMBAT_WINDOW, resources);
    windows.grids.retain(|(owner, _), _| *owner != icon);
    let mut cursors = [0usize; 2];
    let mut convoys = 0usize;
    for fill in fills {
        let Some(grid) = gui_windows::find_path(window, &fill.grid) else {
            continue;
        };
        let holder = laid
            .as_ref()
            .and_then(|laid| laid.holders.iter().find(|holder| holder.path == grid.path));
        let (template, tiny) = match fill.side {
            Some(_) => (native.navy(), false),
            None => match native.tiny() {
                Some(tiny) => (tiny, true),
                None => {
                    *report
                        .naval_grid_gaps
                        .entry(gui_windows::CONVOY_WINDOW_ABSENT.into())
                        .or_default() += fill.items.len();
                    continue;
                }
            },
        };
        for calls in &fill.items {
            let (pool, index) = match fill.side {
                Some(side) => {
                    let index = cursors[side];
                    cursors[side] += 1;
                    (ItemPool::Tile(side), index)
                }
                None => {
                    let index = convoys;
                    convoys += 1;
                    (ItemPool::Convoy, index)
                }
            };
            let key = ItemKey { icon, pool, index };
            update_item(windows, key, template, tiny, calls.as_ref(), resources)?;
            unread_grid(windows, key, tiny, unread.as_deref());
            if !append(windows, grid, holder, key, template)? {
                *report
                    .naval_grid_gaps
                    .entry(naval_markers::GRID_FULL.into())
                    .or_default() += 1;
            }
        }
    }
    finalize(windows, icon, window, laid.as_ref(), viewport)
}

/// Append an item only within the grid's authored slot limits. When visible,
/// show its window with the grid's propagated opacity. Return whether it joined.
fn append(
    windows: &mut Windows,
    grid: &native::Element,
    holder: Option<&native_gui::Holder>,
    key: ItemKey,
    template: &native::Element,
) -> Result<bool, String> {
    let _profile = viewer_foundation::profiling::span("dc_nu_grid_append");
    let held = windows
        .grids
        .entry((key.icon, grid.path.clone()))
        .or_default();
    let fits = native_gui::grid_slots(
        grid,
        holder.map_or([0; 2], |holder| holder.content),
        held.len() + 1,
        holder.and_then(|holder| holder.position),
    )?
    .len()
        > held.len();
    if !fits {
        return Ok(false);
    }
    held.push(key);
    if let Some(holder) = holder
        && let Some(item) = windows.items.get_mut(&key)
    {
        item.instance.gui_show(template, template)?;
        if item.instance.opacity() != holder.opacity {
            item.instance
                .set_opacity(template, template, holder.opacity, true)?;
        }
    }
    Ok(true)
}

/// The templates of the items `held` holds, in order.
fn held_templates<'n>(
    windows: &Windows,
    native: &'n NativeWindows,
    held: &[ItemKey],
) -> Vec<&'n native::Element> {
    held.iter()
        .map(|key| match windows.items.get(key) {
            Some(item) if item.tiny => native.tiny().unwrap_or(native.navy()),
            _ => native.navy(),
        })
        .collect()
}

/// Place each item in every grid of `icon`'s window at its slot plus the offset
/// saved by construction.
fn finalize(
    windows: &mut Windows,
    icon: IconSlot,
    window: &native::Element,
    laid: Option<&Layout>,
    viewport: [i32; 2],
) -> Result<(), String> {
    let _profile = viewer_foundation::profiling::span("dc_nu_grid_finalize");
    let Some((native, _)) = windows.built.clone() else {
        return Ok(());
    };
    let grids: Vec<(String, Held_)> = windows
        .grids
        .iter()
        .filter(|((owner, _), _)| *owner == icon)
        .map(|((_, path), held)| (path.clone(), held.clone()))
        .collect();
    for (path, held) in grids {
        let Some(grid) = gui_windows::find_path(window, &path) else {
            continue;
        };
        let holder = laid.and_then(|laid| laid.holders.iter().find(|holder| holder.path == path));
        let templates = held_templates(windows, &native, &held);
        let placed = native_gui::finalize_grid(
            grid,
            holder.map_or(viewport, |holder| holder.content),
            viewport,
            &templates,
            holder.and_then(|holder| holder.position),
        )
        .map_err(|refusal| refusal.to_string())?;
        for ((key, at), template) in held.iter().zip(&placed.items).zip(templates) {
            if let Some(item) = windows.items.get_mut(key) {
                item.instance.set_position(template, template, *at)?;
            }
        }
    }
    Ok(())
}

/// Rebuild the naval base's ship grid on each admitted update, preserving
/// the prepared port-item order.
fn base_frame(
    inputs: &CounterInputs<'_>,
    plan: &Plan,
    windows: &mut Windows,
    window: &native::Element,
    schedule: &Schedule,
    report: &mut CounterReport,
) -> io::Result<()> {
    let Some(mut pool) = windows.bases.take() else {
        return Ok(());
    };
    // Requests and erasures this frame the view cannot reproduce.
    let diverged = match &plan.bases {
        None => plan.bases_undecided.clone(),
        Some(pass) => (pass.unanchored != 0).then(|| {
            Cow::Owned(format!(
                "{} naval base icons with no port anchor are not requested",
                pass.unanchored
            ))
        }),
    };
    if diverged.is_some() {
        windows.diverged_bases = diverged;
    }
    let result = (|| -> Result<(), String> {
        let mut erased: Vec<(SlotId, Vec<Held>)> = Vec::new();
        // Request the icon for an admitted standing base; otherwise erase its owner.
        let maps_profile = viewer_foundation::profiling::span("dc_nu_base_maps");
        let mut provinces: BTreeSet<u32> = pool.live().map(|(owner, ..)| *owner).collect();
        if let Some(pass) = &plan.bases {
            provinces.extend(pass.bases.iter().map(|(province, _)| *province));
        }
        let anchors: HashMap<u32, [f32; 3]> = plan
            .bases
            .as_ref()
            .map(|pass| pass.bases.iter().copied().collect())
            .unwrap_or_default();
        drop(maps_profile);
        for province in provinces {
            let Some(update) = schedule.update(province) else {
                continue;
            };
            let shown = update == Update::InView && anchors.contains_key(&province);
            if !shown {
                if let Some(slot) = pool.slot_of(&province) {
                    let held = pool.erase(window, &province)?;
                    windows.base_written.remove(&slot);
                    erased.push((slot, held));
                }
                continue;
            }
            let diverged = windows.diverged_bases.clone();
            if allocate(windows, &mut pool, province, IconSlot::Base, &[], diverged).is_none() {
                *report
                    .naval_window_notes
                    .entry(POOL_EXHAUSTED.into())
                    .or_default() += 1;
                windows.diverged_bases = Some(GROWTH_FRAME.into());
            }
        }
        let size = plan.bases.as_ref().map_or([0; 2], |pass| pass.size);
        let mut kept_off_screen = false;
        let pass = pool.update(window, |province, _| {
            let Some(&anchor) = anchors.get(province) else {
                return Admission::Empty;
            };
            let (admitted, off_screen) = admission(
                schedule.dirty(*province),
                icon_point(inputs, anchor, 0, size),
                |point| inside(inputs, point, size),
            );
            kept_off_screen |= keeps_asking(off_screen, schedule.cleans(*province));
            admitted
        })?;
        windows.kept_off_screen |= kept_off_screen;
        for gone in pass.erased {
            windows.base_written.remove(&gone.slot);
            erased.push((gone.slot, gone.held));
        }
        for (slot, held) in erased {
            reach_items(windows, IconSlot::Base(slot), &held)?;
        }
        let Some(bases) = &plan.bases else {
            return Ok(());
        };
        let flags = gui_windows::window_flags(inputs, plan.countries.as_deref());
        let resources = gui_windows::gui_assets(inputs, flags.as_ref());
        let maps_profile = viewer_foundation::profiling::span("dc_nu_base_maps");
        adopt_countries(windows.base_written.values_mut(), plan.countries.as_ref());
        let glyphs: HashMap<_, _> = bases
            .glyphs
            .iter()
            .map(|glyph| (glyph.province, glyph))
            .collect();
        drop(maps_profile);
        let strategic = inputs.visibility.priorities
            == viewer_foundation::map::mode_visibility::PriorityTable::StrategicNavy;
        for (province, slot) in pass.updated {
            let glyph = glyphs.get(&province).copied();
            if strategic
                && let Some(reason) = glyph.and_then(|glyph| glyph.capacity_refusal)
            {
                *report
                    .icon_gui_notes
                    .entry(format!("{NAVAL_BASE_WINDOW}: {reason}"))
                    .or_default() += 1;
            }
            let scale = glyph.map_or(0.0, |glyph| {
                if glyph.highlighted {
                    inputs.content.defines.naval_base_highlighted_scale
                } else {
                    inputs.content.defines.naval_base_scale
                }
            });
            // No roster (or an emptied box) appends nothing; an undecided one appends what the
            // hour does not know.
            let items = match &bases.ports {
                Ok(None) => Ok(&[][..]),
                Ok(Some(ports)) => match ports.get(&province) {
                    None => Ok(&[][..]),
                    Some(Ok(items)) => Ok(items.as_slice()),
                    Some(Err(why)) => Err(why),
                },
                Err(why) => Err(why),
            };
            let _stood = update_or_withhold(
                windows,
                &mut pool,
                window,
                slot,
                BaseInputs { province, glyph, scale, strategic, items, art: inputs.art, countries: plan.countries.as_ref() },
                &resources,
                report,
            );
        }
        Ok(())
    })();
    let slots: Vec<IconSlot> = pool
        .registrations()
        .map(|(_, slot)| IconSlot::Base(slot))
        .collect();
    windows.bases = Some(pool);
    match (&result, &plan.bases_undecided) {
        (Err(why), _) => unknown_pool(windows, slots, &format!("port items are not placed: {why}")),
        (Ok(()), Some(why)) => unknown_pool(windows, slots, why),
        (Ok(()), None) => {}
    }
    // A frame that left the windows' parts unknown has every base written again by the next, which
    // no repaint brings any nearer to standing.
    if result.is_err() || plan.bases_undecided.is_some() {
        windows.base_written.clear();
        windows.owed.clear();
    }
    if let Err(why) = result {
        *report
            .navy_counter_gaps
            .entry(format!("port items are not placed: {why}").into())
            .or_default() += 1;
    }
    Ok(())
}

/// What one admitted base's update is given this frame.
struct BaseInputs<'a> {
    province: u32,
    glyph: Option<&'a bases::NavalBaseIcon>,
    scale: f32,
    strategic: bool,
    items: Result<&'a [gui_windows::Update], &'a Cow<'static, str>>,
    art: u64,
    countries: Option<&'a Arc<Countries>>,
}

/// [`update_base`], and a base whose update fails withheld alone, under why: it is written whole
/// again next frame, owing the view nothing, and the other bases keep what they wrote. Whether
/// the base stood.
fn update_or_withhold(
    windows: &mut Windows,
    pool: &mut Pool<u32>,
    window: &native::Element,
    slot: SlotId,
    given: BaseInputs<'_>,
    resources: &dyn native_gui::Resources,
    report: &mut CounterReport,
) -> bool {
    match update_base(windows, pool, window, slot, given, resources, report) {
        Ok(stood) => stood,
        Err(why) => {
            windows.base_written.remove(&slot);
            unknown_items(windows, IconSlot::Base(slot), &format!("port items are not placed: {why}"), Marked::EveryFrame);
            *report
                .navy_counter_gaps
                .entry(format!("port items are not placed: {why}").into())
                .or_default() += 1;
            false
        }
    }
}

/// One admitted base this frame: written whole (its glyph, its port items and their grid)
/// unless it stands ([`base_due`]). A base written with a decided roster that does not stand yet
/// owes the view the next frame ([`Windows::owed`]). Whether it stood.
fn update_base(
    windows: &mut Windows,
    pool: &mut Pool<u32>,
    window: &native::Element,
    slot: SlotId,
    given: BaseInputs<'_>,
    resources: &dyn native_gui::Resources,
    report: &mut CounterReport,
) -> Result<bool, String> {
    let BaseInputs { province, glyph, scale, strategic, items, art, countries } = given;
    let still = |instance: &Instance| !instance.written() && !instance.active();
    let held = windows.base_written.get(&slot);
    // The items its last decided roster put in its grid, its first ones up to those it left out:
    // it can stand only given that roster again. An item recovering its transition is drawn once its
    // movement ends, where the grid's finalization does not put it: the base is written through it
    // and once after.
    let in_grid = held.map_or(0, |written| {
        written.items.as_ref().map_or(0, Vec::len).saturating_sub(written.overflow)
    });
    let quiet = pool.instance(slot).is_some_and(still)
        && (0..in_grid).all(|index| {
            let key = ItemKey { icon: IconSlot::Base(slot), pool: ItemPool::Port, index };
            !runs_drawn(windows, &key)
                && windows
                    .items
                    .get(&key)
                    .is_none_or(|item| {
                        !item.instance.written()
                            && !(item.instance.transitioning()
                                && windows.settling.contains_key(&Registered::Item(key)))
                    })
        });
    // An undecided roster is written every frame: its items' parts and transitions stay unknown.
    let same = quiet
        && items.is_ok()
        && held.is_some_and(|written| written.same(province, glyph, scale.to_bits(), strategic, items, art, countries));
    let settled = match base_due(held, quiet, same) {
        BaseDue::Stands { overflow } => {
            viewer_foundation::map::redraws::note("naval.bases_kept", 1);
            if overflow > 0 {
                *report.navy_counter_gaps.entry(naval::PORT_GRID_FULL.into()).or_default() += overflow;
            }
            return Ok(true);
        }
        BaseDue::Write { settled } => settled,
    };
    windows.base_written.remove(&slot);
    let instance = pool.get(slot).ok_or("an updated icon has no window")?;
    let update_profile = viewer_foundation::profiling::span("dc_nu_base_update");
    match glyph {
        Some(glyph) => {
            if let Err(why) = update_naval(instance, window, glyph, scale, strategic, resources) {
                pool.refuse(slot, Unknown::Update, why.into());
                return Ok(false);
            }
        }
        None => {
            let c = base_controls(window)?;
            for control in [c.background, c.strategic, c.capacity] {
                instance.hide(window, control)?;
            }
            // BaseUnknowns already counts why this hour has no decided glyph.
        }
    }
    drop(update_profile);
    let items_profile = viewer_foundation::profiling::span("dc_nu_base_items");
    let overflow = base_items(windows, instance, window, slot, items, resources, report)?;
    drop(items_profile);
    pool.rewrote(slot, Rewrote::Update);
    viewer_foundation::map::redraws::note("naval.bases_written", 1);
    if !settled && items.is_ok() {
        windows.owed.insert(slot);
    }
    windows.base_written.insert(
        slot,
        BaseWritten {
            province,
            glyph: glyph.copied(),
            scale: scale.to_bits(),
            strategic,
            items: items.map(<[gui_windows::Update]>::to_vec).map_err(Clone::clone),
            art,
            countries: countries.cloned(),
            settled,
            overflow,
        },
    );
    Ok(false)
}

/// Present retained result icons through scheduled provinces. Disabled passes
/// or provinces without a visible result erase their icons; admitted results
/// request one. Dirty provinces or viewport intersection can retain visibility.
/// Undecided requests or erasures leave pool history diverged.
fn result_frame(
    inputs: &CounterInputs<'_>,
    plan: &Plan,
    windows: &mut Windows,
    window: &native::Element,
    schedule: &Schedule,
    report: &mut CounterReport,
) -> io::Result<()> {
    let Some(mut pool) = windows.results.take() else {
        return Ok(());
    };
    if let Some(why) = &plan.results_undecided {
        windows.diverged_results = Some(why.clone());
    }
    let result = (|| -> Result<(), String> {
        // Icons whose decision the hour cannot make, by why: kept, and withheld.
        let mut undecided = match &plan.results_undecided {
            None => result_owners(
                windows,
                &mut pool,
                window,
                plan.results.as_ref(),
                schedule,
                report,
            )?,
            Some(_) => Vec::new(),
        };
        let size = plan.results.as_ref().map_or([0; 2], |pass| pass.size);
        let mut kept_off_screen = false;
        let held_results = &windows.held_results;
        let pass = pool.update(window, |province, instance| {
            let Some(pass) = &plan.results else {
                return Admission::Empty;
            };
            let held = held_results.get(province).map_or(&[][..], Vec::as_slice);
            match result_empty(&pass.tables, held) {
                Some(true) => return Admission::Empty,
                Some(false) => {}
                // Whether the game erased it is unknown: kept, and undecided.
                None => undecided.push((*province, IS_EMPTY_UNDECIDED.into())),
            }
            // An anchor the hour cannot decide leaves where the icon stands, and so whether it is
            // admitted, undecided: kept where it stood, and undecided.
            let point = match naval_markers::result_anchor(
                inputs,
                &pass.raised,
                pass.vectors_whole,
                *province,
            ) {
                Ok(anchor) => icon_point(inputs, anchor, 0, size),
                Err(why) => {
                    undecided.push((*province, why.into()));
                    return Admission::Inside(instance.position());
                }
            };
            let (admitted, off_screen) = admission(schedule.dirty(*province), point, |point| {
                inside(inputs, point, size)
            });
            kept_off_screen |= keeps_asking(off_screen, schedule.cleans(*province));
            admitted
        })?;
        windows.kept_off_screen |= kept_off_screen;
        for gone in pass.erased {
            windows.held_results.remove(&gone.owner);
        }
        undecided_results(windows, &pool, &undecided, report);
        let Some(results) = &plan.results else {
            return Ok(());
        };
        let flags = gui_windows::window_flags(inputs, plan.countries.as_deref());
        let resources = gui_windows::gui_assets(inputs, flags.as_ref());
        for (province, slot) in pass.updated {
            let held = windows
                .held_results
                .get(&province)
                .map_or(&[][..], Vec::as_slice);
            let update = gui_windows::result_calls(
                window,
                &resources,
                held_presentation(&results.tables, held),
                &mut report.undeclared_result_sprites,
            );
            let instance = pool.get(slot).ok_or("an updated icon has no window")?;
            let applied =
                gui_windows::apply(instance, window, &update.calls, &resources).map_err(|why| {
                    format!("icon omitted: {NAVAL_RESULT_WINDOW} does not update: {why}").into()
                });
            let mut left = marks(&update, applied, "");
            if let Some((_, why)) = undecided
                .iter()
                .find(|(undecided, _)| *undecided == province)
            {
                left.push(Mark::Refuse(Unknown::Update, why.clone()));
            }
            for mark in left {
                match mark {
                    Mark::Refuse(unknown, why) => pool.refuse(slot, unknown, why),
                    Mark::Rewrote(rewrote) => pool.rewrote(slot, rewrote),
                }
            }
        }
        Ok(())
    })();
    windows.results = Some(pool);
    if let Err(why) = result {
        *report
            .naval_window_notes
            .entry(format!("{NAVAL_RESULT_WINDOW} does not update: {why}").into())
            .or_default() += 1;
    }
    Ok(())
}

/// Apply result-icon ownership over scheduled provinces in province order.
fn result_owners(
    windows: &mut Windows,
    pool: &mut Pool<u32>,
    window: &native::Element,
    results: Option<&ResultPass>,
    schedule: &Schedule,
    report: &mut CounterReport,
) -> Result<Vec<(u32, Cow<'static, str>)>, String> {
    let mut provinces: BTreeSet<u32> = pool.live().map(|(owner, ..)| *owner).collect();
    if let Some(pass) = results {
        provinces.extend(pass.tables.provinces.keys().copied());
        // A province the incomplete census lacks may be one the game requests an icon for.
        if let Some(why) = pass.tables.undecided {
            windows.diverged_results = Some(why.into());
        }
    }
    let mut undecided: Vec<(u32, Cow<'static, str>)> = Vec::new();
    for province in provinces {
        let Some(update) = schedule.update(province) else {
            continue;
        };
        let visible = match (results, update) {
            (Some(pass), Update::InView) => pass.tables.visible(province),
            _ => Ok(Vec::new()),
        };
        match visible {
            Ok(visible) if visible.is_empty() => {
                if pool.slot_of(&province).is_some() {
                    pool.erase(window, &province)?;
                    windows.held_results.remove(&province);
                }
            }
            Ok(visible) => {
                let diverged = windows.diverged_results.clone();
                if allocate(
                    windows,
                    pool,
                    province,
                    IconSlot::Result,
                    &[gui_windows::RESULT_SPRITE, gui_windows::BG_FRAME],
                    diverged,
                )
                .is_some()
                {
                    windows.held_results.insert(province, visible);
                } else {
                    *report
                        .naval_window_notes
                        .entry(POOL_EXHAUSTED.into())
                        .or_default() += 1;
                    windows.diverged_results = Some(GROWTH_FRAME.into());
                }
            }
            Err(why) => {
                windows.diverged_results = Some(why.into());
                undecided.push((province, why.into()));
            }
        }
    }
    Ok(undecided)
}

/// An unavailable ownership decision leaves a live icon's show and hide
/// history unknown until the view's transition state can establish it.
fn undecided_results(
    windows: &mut Windows,
    pool: &Pool<u32>,
    undecided: &[(u32, Cow<'static, str>)],
    report: &mut CounterReport,
) {
    for (province, why) in undecided {
        windows.diverged_results = Some(why.clone());
        match pool.slot_of(province) {
            Some(slot) => {
                transition_unknown(windows, Registered::Icon(IconSlot::Result(slot)), why, Marked::EveryFrame)
            }
            None => {
                report.unavailable_naval_results += 1;
                *report.naval_result_gaps.entry(why.clone()).or_default() += 1;
            }
        }
    }
}

/// Why a result icon's the game is undecided ([`result_empty`]).
const IS_EMPTY_UNDECIDED: &str =
    "whether a result icon's results still admit the viewer is undecided";

/// An icon holding `held` uses [`naval_markers::result_presentation`]. A reference
/// absent from a complete result census has expired and is skipped, but still
/// counts in `held`. One absent from an incomplete census may still resolve,
/// leaving presentation undecided.
fn held_presentation(
    tables: &naval_markers::ResultTables,
    held: &[u64],
) -> Result<(&'static str, u32), &'static str> {
    let mut resolving = Vec::with_capacity(held.len());
    for id in held {
        match (tables.facts.get(id), tables.undecided) {
            (Some(facts), _) => resolving.push(*facts),
            (None, Some(why)) => return Err(why),
            (None, None) => {}
        }
    }
    naval_markers::result_presentation(&resolving, held.len())
}

/// An icon holding `held` is empty unless a result still resolves in this hour
/// and admits the viewer. `None` when an unresolved list or incomplete result
/// tables leave that undecided.
fn result_empty(tables: &naval_markers::ResultTables, held: &[u64]) -> Option<bool> {
    let mut undecided = tables.undecided.is_some();
    for id in held {
        match tables
            .facts
            .get(id)
            .map(naval_markers::ResultFacts::admitted)
        {
            Some(Some(true)) => return Some(false),
            Some(None) => undecided = true,
            Some(Some(false)) | None => {}
        }
    }
    (!undecided).then_some(true)
}

/// The retained result icons of the visible list, laid out and drawn through their windows at
/// the content's result priority past the combat icons' ordinals; what is hit is `bg_btn` as
/// drawn, opening the icon's first result. A withheld icon or a refused layout is counted.
fn draw_results(
    inputs: &CounterInputs<'_>,
    plan: &Plan,
    windows: &Windows,
    window: &native::Element,
    resources: &GuiAssets<'_>,
    batch: &mut Batch,
    report: &mut CounterReport,
) -> io::Result<()> {
    let (Some(pool), Some(_)) = (&windows.results, &plan.results) else {
        return Ok(());
    };
    for icon in pool.visible() {
        let (province, instance) = (*icon.owner, icon.instance);
        if let Some(why) = icon.withheld {
            report.unavailable_naval_results += 1;
            *report
                .naval_result_gaps
                .entry(why.to_owned().into())
                .or_default() += 1;
            continue;
        }
        let layout = match instance.layout(window, resources, None) {
            Ok(Some(layout)) => layout,
            Ok(None) => continue,
            Err(refusals) => {
                report.unavailable_naval_results += 1;
                count_refused(
                    &mut report.naval_result_gaps,
                    NAVAL_RESULT_WINDOW,
                    &refusals,
                );
                continue;
            }
        };
        count_noted(
            &mut report.naval_result_gaps,
            NAVAL_RESULT_WINDOW,
            &layout.diagnostics,
        );
        let origin = instance.position();
        let key = (
            inputs.priority(PriorityIcon::NavalCombatResults),
            report.combat_ordinal(),
        );
        batch.at_anchor(
            plan.results
                .as_ref()
                .and_then(|pass| {
                    naval_markers::result_anchor(inputs, &pass.raised, pass.vectors_whole, province)
                        .ok()
                })
                .and_then(|anchor| {
                    inputs.icon_anchor(
                        IconFamily::Combats,
                        anchor,
                        inputs.content.defines.naval_combat_cutoff,
                    )
                }),
        );
        if let Err(refusal) = gui_windows::draw_window(&layout, &[], origin, resources, batch, key)?
        {
            report.unavailable_naval_results += 1;
            count_refused(
                &mut report.naval_result_gaps,
                NAVAL_RESULT_WINDOW,
                &[refusal],
            );
            continue;
        }
        if let (Some(hit), Some(&first)) = (
            window
                .find(native::Kind::Button, "bg_btn")
                .and_then(|button| {
                    hit_rectangle(
                        button,
                        &layout,
                        origin,
                        resources,
                        &mut report.icon_gui_notes,
                    )
                }),
            windows
                .held_results
                .get(&province)
                .and_then(|held| held.first()),
        ) {
            report.combat_hits.push(Hit {
                selection: Selection::NavalResult(first),
                rect: hit.rect,
                layer: layer(key.0, key.1, 0),
                order: hit.order,
                supported: hit.supported,
            });
        }
        report.naval_result_icons += 1;
    }
    batch.at_anchor(None);
    Ok(())
}

/// Apply one admitted naval-base update. Its show call reaches the retained
/// ship-grid items before their own updates.
fn base_items(
    windows: &mut Windows,
    instance: &mut Instance,
    window: &native::Element,
    slot: SlotId,
    items: Result<&[gui_windows::Update], &Cow<'static, str>>,
    resources: &dyn native_gui::Resources,
    report: &mut CounterReport,
) -> Result<usize, String> {
    match items {
        Ok(items) => fill_ports(windows, instance, window, slot, items, resources, report),
        Err(why) => {
            let held = instance.take_held();
            reach_items(windows, IconSlot::Base(slot), &held)?;
            unknown_items(windows, IconSlot::Base(slot), why, Marked::EveryFrame);
            Ok(0)
        }
    }
}

/// Apply one admitted naval-base update. Its show call reaches the retained
/// ship-grid items before their own updates. How many items its grid had no slot for.
fn fill_ports(
    windows: &mut Windows,
    instance: &mut Instance,
    window: &native::Element,
    slot: SlotId,
    items: &[gui_windows::Update],
    resources: &dyn native_gui::Resources,
    report: &mut CounterReport,
) -> Result<usize, String> {
    let (native, viewport) = windows.built.clone().ok_or("the windows are not built")?;
    let grid = window
        .find(native::Kind::Grid, "box_ships")
        .ok_or_else(|| format!("{NAVAL_BASE_WINDOW} declares no box_ships grid"))?;
    let icon = IconSlot::Base(slot);
    let held = instance.take_held();
    reach_items(windows, icon, &held)?;
    let (laid, unread) = read_grids(instance, window, NAVAL_BASE_WINDOW, resources);
    let holder = laid
        .as_ref()
        .and_then(|laid| laid.holders.iter().find(|holder| holder.path == grid.path));
    windows.grids.remove(&(icon, grid.path.clone()));
    let navy = native.navy();
    let mut overflow = 0;
    for (index, update) in items.iter().enumerate() {
        let key = ItemKey {
            icon,
            pool: ItemPool::Port,
            index,
        };
        update_item(windows, key, navy, false, Ok(update), resources)?;
        unread_grid(windows, key, false, unread.as_deref());
        if !append(windows, grid, holder, key, navy)? {
            *report
                .navy_counter_gaps
                .entry(naval::PORT_GRID_FULL.into())
                .or_default() += 1;
            overflow += 1;
        }
    }
    finalize(windows, icon, window, laid.as_ref(), viewport)?;
    Ok(overflow)
}

/// A drawn grid item: its place in its holder's content and its layout.
struct PlacedItem {
    key: ItemKey,
    naval_convoy: Option<u64>,
    place: [i32; 2],
    layout: Layout,
    selectable: Option<(bool, Arc<[u64]>)>,
    owner: Option<u32>,
    port_tooltip: Option<viewer_foundation::map::counter_hovers::task_force_port::Metadata>,
}

/// The items `grid` of `icon`'s window holds that draw, in the order it holds them, each laid
/// out through its instance with its place in the holder's content (the grid object's place, as
/// finalize measures it over every item held, plus the item window's own position): every held
/// item keeps its slot whether or not it draws. An item whose window is hidden draws nothing; one
/// on which a refusal stands ([`Windows::items_withheld`]) draws nothing and is counted under why;
/// the module's refusals are counted under the item window's name. `Err` is why the grid's
/// items cannot be placed.
fn held_items(
    windows: &Windows,
    icon: IconSlot,
    grid: &native::Element,
    holder: &native_gui::Holder,
    resources: &dyn native_gui::Resources,
    viewport: [i32; 2],
    report: &mut CounterReport,
) -> Result<Vec<PlacedItem>, String> {
    let Some((native, _)) = &windows.built else {
        return Ok(Vec::new());
    };
    let Some(held) = windows.grids.get(&(icon, grid.path.clone())) else {
        return Ok(Vec::new());
    };
    let templates = held_templates(windows, native, held);
    let origin =
        native_gui::finalize_grid(grid, holder.content, viewport, &templates, holder.position)
            .map_err(|refusal| refusal.to_string())?
            .origin;
    let mut items = Vec::new();
    for (key, template) in held.iter().zip(templates) {
        let Some(item) = windows.items.get(key) else {
            continue;
        };
        if !item.instance.drawn() {
            continue;
        }
        let (name, gaps) = if item.tiny {
            (TINY_UNIT_COUNTER_WINDOW, &mut report.naval_grid_gaps)
        } else {
            (NAVY_WINDOW, &mut report.navy_counter_gaps)
        };
        if let Some(why) = windows.items_withheld.why(key) {
            *gaps.entry(why.to_owned().into()).or_default() += 1;
            continue;
        }
        match item.instance.layout_kept(
            template,
            resources,
            item.port_tooltip.is_some() || item.naval_convoy.is_some(),
            windows.stamp,
        ) {
            Ok(Some(layout)) => {
                count_noted(gaps, name, &layout.diagnostics);
                let at = item.instance.position();
                items.push(PlacedItem {
                    key: *key,
                    naval_convoy: item.naval_convoy,
                    place: [origin[0].wrapping_add(at[0]), origin[1].wrapping_add(at[1])],
                    layout,
                    selectable: item.selectable.clone(),
                    owner: item.owner,
                    port_tooltip: item.port_tooltip.clone(),
                });
            }
            Ok(None) => {}
            Err(refusals) => count_refused(gaps, name, &refusals),
        }
    }
    Ok(items)
}

/// What a combat icon of the visible list ([`Pool::visible`]) draws: its window laid out, with
/// the items each of its grids holds placed in the grid ([`held_items`]). `None` when it draws
/// nothing: a refusal standing on its window ([`Pool::refuse`], counted with its combat unless
/// `counted`, an icon the preparation already counted as omitted this hour) or the module's
/// refusal (counted with its combat).
fn combat_parts(
    windows: &Windows,
    icon: &Visible<'_, CombatOwner>,
    counted: bool,
    window: &native::Element,
    resources: &dyn native_gui::Resources,
    viewport: [i32; 2],
    report: &mut CounterReport,
) -> Option<(Layout, Vec<Vec<native_gui::Part>>)> {
    let (slot, instance, (_, combat)) = (icon.slot, icon.instance, *icon.owner);
    if let Some(why) = icon.withheld {
        if !counted {
            report.unavailable_naval_combats += 1;
            naval_markers::naval_gap(report, why.to_owned(), [combat]);
        }
        return None;
    }
    let layout = match instance.layout(window, resources, None) {
        Ok(Some(layout)) => layout,
        Ok(None) => return None,
        Err(refusals) => {
            report.unavailable_naval_combats += 1;
            for refusal in &refusals {
                naval_markers::naval_gap(
                    report,
                    format!(
                        "icon omitted: {}",
                        refused_cause(NAVAL_COMBAT_WINDOW, refusal)
                    ),
                    [combat],
                );
            }
            return None;
        }
    };
    for diagnostic in &layout.diagnostics {
        naval_markers::naval_gap(
            report,
            format!(
                "part omitted: {}",
                noted_cause(NAVAL_COMBAT_WINDOW, diagnostic)
            ),
            [combat],
        );
    }
    let mut items = vec![Vec::new(); layout.holders.len()];
    for (index, holder) in layout.holders.iter().enumerate() {
        let Some(grid) = gui_windows::find_path(window, &holder.path) else {
            continue;
        };
        let icon = IconSlot::Combat(slot);
        match held_items(windows, icon, grid, holder, resources, viewport, report) {
            Ok(held) => {
                items[index] = held
                    .iter()
                    .flat_map(|item| holder.place(item.place, &item.layout))
                    .collect();
            }
            Err(reason) => {
                *report
                    .naval_grid_gaps
                    .entry(format!("a naval combat grid is not laid out: {reason}").into())
                    .or_default() += 1;
            }
        }
    }
    Some((layout, items))
}

/// Every combat icon of the visible list ([`Pool::visible`], the manager's re-attached live and
/// visible icons), with the items its grids hold, under the naval combat priority.
fn draw_combats(
    inputs: &CounterInputs<'_>,
    plan: &Plan,
    windows: &Windows,
    window: &native::Element,
    resources: &GuiAssets<'_>,
    batch: &mut Batch,
    report: &mut CounterReport,
) -> io::Result<()> {
    let (Some(pool), Some(pass)) = (&windows.combats, &plan.combat) else {
        return Ok(());
    };
    let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
    for icon in pool.visible() {
        let (owner, instance) = (icon.owner, icon.instance);
        let (_, combat) = *owner;
        let counted = pass
            .icons
            .iter()
            .any(|placed| placed.owner == *owner && placed.omitted);
        let Some((layout, items)) =
            combat_parts(windows, &icon, counted, window, resources, viewport, report)
        else {
            continue;
        };
        let origin = instance.position();
        let key = (
            inputs.priority(PriorityIcon::NavalCombat),
            report.combat_ordinal(),
        );
        batch.at_anchor(windows.placed.get(owner).and_then(|&(anchor, _)| {
            inputs.icon_anchor(
                IconFamily::Combats,
                anchor,
                inputs.content.defines.naval_combat_cutoff,
            )
        }));
        if let Err(refusal) =
            gui_windows::draw_window(&layout, &items, origin, resources, batch, key)?
        {
            report.unavailable_naval_combats += 1;
            naval_markers::naval_gap(
                report,
                format!(
                    "icon omitted: {}",
                    refused_cause(NAVAL_COMBAT_WINDOW, &refusal)
                ),
                [combat],
            );
            continue;
        }
        for (footprint, clickable) in naval_markers::combat_footprints(
            window,
            &layout,
            origin,
            resources,
            &mut report.icon_gui_notes,
        ) {
            let hit = Hit {
                selection: Selection::Naval(combat),
                rect: footprint.rect,
                layer: layer(key.0, key.1, 0),
                order: footprint.order,
                supported: footprint.supported,
            };
            report.naval_marker_hovers.push(hit.clone());
            if clickable {
                report.combat_hits.push(hit);
            }
        }
        report.naval_combats += 1;
        if windows.pool_frame.contains(owner) {
            report.base_frame_naval_combats += 1;
        }
    }
    batch.at_anchor(None);
    Ok(())
}

/// Compose the region's retained grid items through their authored holders.
/// Every child uses the same item update, refusal and transition machinery as
/// naval-base and combat grids, while its pool remains separate from both.
#[allow(clippy::too_many_arguments)]
pub(super) fn compose_mission_grids(
    windows: &Windows,
    region: u32,
    slot: SlotId,
    window: &native::Element,
    layout: &Layout,
    composed: &mut [Vec<native_gui::Part>],
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    priority: (i8, u32),
    report: &mut CounterReport,
) {
    let Some((native, viewport)) = &windows.built else {
        return;
    };
    for (at, holder) in layout.holders.iter().enumerate() {
        let Some(grid) = gui_windows::find_path(window, &holder.path) else {
            continue;
        };
        let items = match held_items(
            windows,
            IconSlot::Mission(slot),
            grid,
            holder,
            resources,
            *viewport,
            report,
        ) {
            Ok(items) => items,
            Err(why) => {
                *report.naval_grid_gaps.entry(why.into()).or_default() += 1;
                continue;
            }
        };
        for item in items {
            composed[at].extend(holder.place(item.place, &item.layout));
            if let (Some(reference), Some(template)) = (item.naval_convoy, native.tiny()) {
                let placed = Layout {
                    size: item.layout.size,
                    parts: holder.place(item.place, &item.layout),
                    regions: holder.place_regions(item.place, &item.layout.regions),
                    holders: Vec::new(),
                    diagnostics: Vec::new(),
                    pointed: None,
                    hovered: None,
                    target: None,
                    boxes: Vec::new(),
                };
                for region_hit in &placed.regions {
                    let Some(element) = gui_windows::find_path(template, &region_hit.path) else {
                        continue;
                    };
                    if let Some(hit) = gui_windows::native_hit_rectangle(
                        element,
                        &placed,
                        origin,
                        resources,
                        &mut report.icon_gui_notes,
                    ) {
                        report.naval_region_hovers.push(NavalRegionalHover {
                            region,
                            control: NavalRegionalControl::Convoy(reference),
                            rect: hit.rect,
                            layer: layer(priority.0, priority.1, 0),
                            hit: hit.geometry,
                            supported: hit.supported,
                            text: None,
                        });
                    }
                }
            }
            if let Some((ours, members)) = item.selectable {
                let corner = [
                    origin[0] as f32 + holder.origin[0] + holder.scale * item.place[0] as f32,
                    origin[1] as f32 + holder.origin[1] + holder.scale * item.place[1] as f32,
                ];
                let size = item.layout.size.map(|v| v as f32 * holder.scale);
                let mut bounds = [
                    corner[0],
                    corner[1],
                    corner[0] + size[0],
                    corner[1] + size[1],
                ];
                if let Some(clip) = holder.clip {
                    bounds[0] = bounds[0].max((origin[0] + clip[0]) as f32);
                    bounds[1] = bounds[1].max((origin[1] + clip[1]) as f32);
                    bounds[2] = bounds[2].min((origin[0] + clip[2]) as f32);
                    bounds[3] = bounds[3].min((origin[1] + clip[3]) as f32);
                }
                if let Some(metadata) = item.port_tooltip {
                    report
                        .task_force_port_hovers
                        .extend(super::naval_port_hovers::presented(
                            native.navy(),
                            &item.layout,
                            holder,
                            item.place,
                            origin,
                            resources,
                            report.unit_hits.len(),
                            &members,
                            &metadata,
                            &mut report.icon_gui_notes,
                        ));
                }
                report
                    .regional_naval_unit_hits
                    .insert(report.unit_hits.len());
                report
                    .unit_hits
                    .push(viewer_foundation::map::counter_hovers::unit_hits::Hit {
                        ours,
                        port: true,
                        plate: None,
                        rect: [
                            bounds[0],
                            bounds[1],
                            (bounds[2] - bounds[0]).max(0.0),
                            (bounds[3] - bounds[1]).max(0.0),
                        ],
                        kind: viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce,
                        members,
                        layer: layer(priority.0, priority.1, 0),
                    });
            }
            report.naval_counters += 1;
        }
    }
}

/// The port items a base icon of the visible list ([`Pool::visible`]) draws in `box_ships`
/// (`grid`): its window's `box_ships` holder and each drawn item's place in the holder's content
/// and layout ([`held_items`]). `None` when its grid holds nothing; when a refusal stands on its
/// window, its layout is refused or its items cannot be placed, counted once for each item it
/// holds (none of them is placed).
#[allow(clippy::too_many_arguments)]
fn port_parts(
    windows: &Windows,
    base: &Visible<'_, u32>,
    window: &native::Element,
    drawn: Option<&Layout>,
    grid: &native::Element,
    resources: &dyn native_gui::Resources,
    viewport: [i32; 2],
    report: &mut CounterReport,
) -> Option<(native_gui::Holder, Vec<PlacedItem>)> {
    let (icon, instance) = (IconSlot::Base(base.slot), base.instance);
    let held = windows
        .grids
        .get(&(icon, grid.path.clone()))
        .map_or(0, Vec::len);
    if held == 0 {
        return None;
    }
    if let Some(why) = base.withheld {
        *report
            .navy_counter_gaps
            .entry(format!("port items are not placed: {why}").into())
            .or_default() += held;
        return None;
    }
    // The glyph and its port grid share one instance and tick. Reuse the successful
    // glyph layout; refused or pending resources still take the ordinary second query.
    let layout;
    let drawn = match drawn.filter(|layout| layout.diagnostics.is_empty()) {
        Some(drawn) => drawn,
        None => {
            layout = match instance.layout(window, resources, None) {
                Ok(Some(layout)) => layout,
                Ok(None) => return None,
                Err(refusals) => {
                    for refusal in &refusals {
                        *report
                            .navy_counter_gaps
                            .entry(
                                format!(
                                    "port items are not placed: {}",
                                    refused_cause(NAVAL_BASE_WINDOW, refusal)
                                )
                                .into(),
                            )
                            .or_default() += held;
                    }
                    return None;
                }
            };
            &layout
        }
    };
    let holder = drawn
        .holders
        .iter()
        .find(|holder| holder.path == grid.path)?
        .clone();
    match held_items(windows, icon, grid, &holder, resources, viewport, report) {
        Ok(items) => Some((holder, items)),
        Err(refusal) => {
            *report
                .navy_counter_gaps
                .entry(format!("port items are not placed: {refusal}").into())
                .or_default() += held;
            None
        }
    }
}

/// What the windows' resources draw from, as one number: the assets, templates and localisation,
/// the hour's countries their flags resolve through, the viewport and the art decoded so far. A
/// window laid out with other resources is laid out again ([`Instance::layout_kept`]).
fn resources_stamp(inputs: &CounterInputs<'_>, countries: Option<&Countries>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    (
        std::ptr::from_ref(inputs.assets) as usize,
        std::ptr::from_ref(inputs.windows) as usize,
        std::ptr::from_ref(inputs.localisation) as usize,
        countries.map(|countries| std::ptr::from_ref(countries) as usize),
        inputs.viewport,
        inputs.ui_scale.to_bits(),
        inputs.art,
    )
        .hash(&mut hasher);
    hasher.finish()
}

/// Base glyphs and their port items use separate batches to preserve the label draw order.
#[allow(clippy::too_many_arguments)]
fn draw_ports(
    inputs: &CounterInputs<'_>,
    plan: &Plan,
    windows: &Windows,
    carrier_wings: &mut CarrierViews,
    tick: Tick,
    window: &native::Element,
    resources: &GuiAssets<'_>,
    glyphs: &mut Batch,
    batch: &mut Batch,
    report: &mut CounterReport,
) -> io::Result<()> {
    let Some(pool) = &windows.bases else {
        return Ok(());
    };
    let grid = window.find(native::Kind::Grid, "box_ships");
    let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
    let anchors: HashMap<u32, [f32; 3]> = plan
        .bases
        .as_ref()
        .map(|pass| pass.bases.iter().copied().collect())
        .unwrap_or_default();
    for base in pool.visible() {
        // A base and the ships in its port stand together.
        let anchor = anchors.get(base.owner).and_then(|&anchor| {
            inputs.icon_anchor(
                IconFamily::Bases,
                anchor,
                inputs.content.defines.naval_base_cutoff,
            )
        });
        glyphs.at_anchor(anchor);
        batch.at_anchor(anchor);
        let drawn = visible_layout_kept(
            &base,
            window,
            NAVAL_BASE_WINDOW,
            resources,
            &mut report.icon_gui_notes,
            windows.stamp,
        );
        if let Some(drawn) = &drawn {
            let key = layer(
                inputs.priority(PriorityIcon::NavalBase),
                report.base_ordinal(),
                0,
            );
            glyphs.at_layer(key);
            match native_gui::draw(&drawn.layout, drawn.origin, resources, glyphs) {
                Ok(()) => {
                    let first = report.base_hits.len();
                    super::base_windows::record_hit(
                        window,
                        &drawn.layout,
                        drawn.origin,
                        resources,
                        key,
                        report,
                    );
                    for index in first..report.base_hits.len() {
                        report.naval_base_hits.insert(index, *base.owner);
                    }
                    report.naval_bases += 1;
                }
                Err(why) => count_refused(&mut report.icon_gui_notes, NAVAL_BASE_WINDOW, &[why]),
            }
        }
        let Some(grid) = grid else {
            continue;
        };
        let Some((holder, items)) = port_parts(
            windows,
            &base,
            window,
            drawn.as_ref().map(|drawn| &drawn.layout),
            grid,
            resources,
            viewport,
            report,
        ) else {
            continue;
        };
        let origin = base.instance.position();
        for PlacedItem {
            key: item_key,
            naval_convoy: _,
            place,
            layout,
            selectable,
            owner,
            port_tooltip,
        } in items
        {
            // This footprint describes only the plate. An unclipped child can
            // extend outside it; native GUI drawing/hits use the child's clip.
            let corner = [
                origin[0] as f32 + holder.origin[0] + holder.scale * place[0] as f32,
                origin[1] as f32 + holder.origin[1] + holder.scale * place[1] as f32,
            ];
            let extent = layout.size.map(|value| value as f32 * holder.scale);
            let key = (
                inputs.priority(PriorityIcon::NavalBase),
                report.base_ordinal(),
            );
            let composed = if let Some(view) = carrier_wings.get_mut(&CarrierKey::Port(item_key)) {
                let drawn = viewer_foundation::map::icon_windows::Drawn {
                    owner: 0,
                    origin,
                    layout: layout.clone(),
                };
                view.compose_held(
                    0,
                    &drawn,
                    NAVY_WINDOW,
                    "wings_box",
                    inputs,
                    resources,
                    tick,
                    layer(key.0, key.1, 0),
                    Some((&holder, place)),
                    IconFamily::Bases,
                    report,
                )
                .unwrap_or_else(|why| {
                    *report
                        .icon_gui_notes
                        .entry(format!("Port carrier wing grid: {why}"))
                        .or_default() += 1;
                    air_grid::Composed::empty(layout.holders.len())
                })
            } else {
                air_grid::Composed::empty(layout.holders.len())
            };
            let parts = draw_parts::place(&holder, place, &layout, &composed.parts);
            let first = batch.quads.len();
            let drawn = gui_windows::draw_held(&holder, parts, origin, resources, batch, key)?;
            if !inputs.display.port_item(owner) {
                batch.quads.truncate(first);
            }
            match drawn {
                Ok(()) => {
                    composed.commit(report);
                    report.naval_counters += 1;
                    if let Some((ours, members)) = selectable {
                        let mut bounds = [
                            corner[0],
                            corner[1],
                            corner[0] + extent[0],
                            corner[1] + extent[1],
                        ];
                        if let Some(clip) = holder.clip {
                            bounds[0] = bounds[0].max((origin[0] + clip[0]) as f32);
                            bounds[1] = bounds[1].max((origin[1] + clip[1]) as f32);
                            bounds[2] = bounds[2].min((origin[0] + clip[2]) as f32);
                            bounds[3] = bounds[3].min((origin[1] + clip[3]) as f32);
                        }
                        let unit_hit = report.unit_hits.len();
                        if let Some(metadata) = port_tooltip {
                            let Some((native, _)) = &windows.built else {
                                continue;
                            };
                            report.task_force_port_hovers.extend(
                                super::naval_port_hovers::presented(
                                    native.navy(),
                                    &layout,
                                    &holder,
                                    place,
                                    origin,
                                    resources,
                                    unit_hit,
                                    &members,
                                    &metadata,
                                    &mut report.icon_gui_notes,
                                ),
                            );
                        }
                        report
                            .unit_hits
                            .push(viewer_foundation::map::counter_hovers::unit_hits::Hit {
                            ours,
                            port: true,
                            plate: None,
                            rect: [
                                bounds[0],
                                bounds[1],
                                (bounds[2] - bounds[0]).max(0.0),
                                (bounds[3] - bounds[1]).max(0.0),
                            ],
                            kind:
                                viewer_foundation::map::counter_hovers::unit_hits::Kind::TaskForce,
                            members,
                            layer: layer(key.0, key.1, 0),
                        });
                    }
                }
                Err(refusal) => {
                    count_refused(&mut report.navy_counter_gaps, NAVY_WINDOW, &[refusal])
                }
            }
        }
    }
    glyphs.at_anchor(None);
    batch.at_anchor(None);
    Ok(())
}

#[cfg(test)]
mod tests;
