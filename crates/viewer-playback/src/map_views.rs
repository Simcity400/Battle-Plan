//! Per-view composition of counter options, icon histories and the shared schedule.

use std::sync::Mutex;
use std::{collections::BTreeSet, sync::Arc};
use viewer_foundation::map::counter_move::SharedMovement;
use viewer_foundation::map::icon_windows::Pools;
use viewer_foundation::map::province_schedule::Shared;
use viewer_foundation::unit_handle::Unit;
use {
    viewer_counters::map::counters::{CounterSettings, naval_windows},
    viewer_foundation::map::icon_windows::{PoolView, Retry},
};

/// The canonical country slots a counter filter draws at this hour; `None` draws every
/// country. An hour lacking the complete tables a filter reads, or naming a human-controlled
/// country it cannot place, draws every country rather than guess.
fn counter_owners(
    frame: &replay_frame::frame::DecodedFrame,
    filter: &dashboard_core::protocol::CounterFilter,
) -> Option<Arc<BTreeSet<u32>>> {
    use dashboard_core::protocol::CounterFilter;
    use replay_data::rows::tables;
    match filter {
        CounterFilter::All => None,
        CounterFilter::Players => {
            let links = frame.complete::<tables::CountryLinks>().ok()?;
            let humans = frame.complete::<tables::HumanControllers>().ok()?;
            player_slots(
                links.iter().map(|link| (link.id, link.canonical_slot)),
                humans
                    .iter()
                    .filter(|row| row.count > 0)
                    .map(|row| row.country),
            )
            .map(Arc::new)
        }
        CounterFilter::Countries { tags } => {
            let countries = frame.complete::<tables::Countries>().ok()?;
            Some(Arc::new(chosen_slots(
                tags,
                countries
                    .iter()
                    .map(|country| (country.tag.as_slice(), country.canonical_slot)),
            )))
        }
    }
}

/// The slots of the countries humans control, through the hour's country links; none when any
/// one cannot be placed, so a player's counters never disappear for a missing link.
fn player_slots(
    links: impl IntoIterator<Item = (u32, u32)>,
    humans: impl IntoIterator<Item = u32>,
) -> Option<BTreeSet<u32>> {
    let slots: std::collections::BTreeMap<u32, u32> = links.into_iter().collect();
    humans
        .into_iter()
        .map(|country| slots.get(&country).copied())
        .collect()
}

/// The slots of the countries whose tag is chosen.
fn chosen_slots<'a>(
    tags: &[String],
    countries: impl IntoIterator<Item = (&'a [u8], u32)>,
) -> BTreeSet<u32> {
    let tags: BTreeSet<&[u8]> = tags.iter().map(|tag| tag.as_bytes()).collect();
    countries
        .into_iter()
        .filter(|(tag, _)| tags.contains(tag))
        .map(|(_, slot)| slot)
        .collect()
}

/// A view's resolved map settings, resolved again only when the store's generation or the
/// view's scope changes, so a frame never resolves them.
#[derive(Debug, Default)]
pub struct ResolvedSettings {
    read: Option<(u64, String, Arc<dashboard_core::protocol::ResolvedMapSettings>)>,
}

impl ResolvedSettings {
    pub fn get(
        &mut self,
        generation: u64,
        scope: String,
        resolve: impl FnOnce(&str) -> dashboard_core::protocol::ResolvedMapSettings,
    ) -> Arc<dashboard_core::protocol::ResolvedMapSettings> {
        match &self.read {
            Some((at, key, settings)) if *at == generation && *key == scope => Arc::clone(settings),
            _ => {
                let settings = Arc::new(resolve(&scope));
                self.read = Some((generation, scope, Arc::clone(&settings)));
                settings
            }
        }
    }
}

/// The tables [`counter_owners`] reads.
const OWNER_TABLES: [replay_data::rows::TableId; 3] = [
    replay_data::rows::TableId::CountryLinks,
    replay_data::rows::TableId::HumanControllers,
    replay_data::rows::TableId::Countries,
];

/// A view's resolved counter owners, read again only when the filter or one of the tables they
/// come from changes, so a frame never re-reads the hour's tables. While the map's playhead is on
/// they are read from its rows at the head, four times a second at most while playing.
#[derive(Debug, Default)]
pub struct CounterOwners {
    resolved: Option<ResolvedOwners>,
    head: Option<crate::playhead_host::views::AtHead>,
}

/// The table allocations and filter the owners were read for, and the owners. An allocation
/// belongs to one recording's decoder and is shared only by hours whose table did not change,
/// so another recording, branch or changed hour reads again.
type ResolvedOwners = (
    [replay_frame::frame::ViewIdentity; OWNER_TABLES.len()],
    dashboard_core::protocol::CounterFilter,
    Option<Arc<BTreeSet<u32>>>,
);

impl CounterOwners {
    /// The owners at `head`'s rows while the map's playhead is open, the last resolved while they
    /// load; at `today` with no open map.
    pub fn get(
        &mut self,
        today: Option<&replay_frame::frame::DecodedFrame>,
        head: Option<&crate::playhead_host::views::Views>,
        filter: &dashboard_core::protocol::CounterFilter,
    ) -> Option<Arc<BTreeSet<u32>>> {
        use crate::playhead_host::views::{AtHead, Pace};
        let Some(views) = head else {
            self.head = None;
            return today.and_then(|frame| self.read(frame, filter));
        };
        let at = self.head.get_or_insert_with(|| AtHead::new("counter owners", &OWNER_TABLES, Pace::Step).keeps_its_answer());
        match at.frame(views) {
            Some(rows) => self.read(&rows, filter),
            None => self.resolved.as_ref().and_then(|(_, _, owners)| owners.clone()),
        }
    }

    fn read(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        filter: &dashboard_core::protocol::CounterFilter,
    ) -> Option<Arc<BTreeSet<u32>>> {
        let tables = OWNER_TABLES.map(|table| frame.tables[table.index()].identity());
        self.get_with(tables, filter, |filter| counter_owners(frame, filter))
    }

    fn get_with(
        &mut self,
        tables: [replay_frame::frame::ViewIdentity; OWNER_TABLES.len()],
        filter: &dashboard_core::protocol::CounterFilter,
        read: impl FnOnce(&dashboard_core::protocol::CounterFilter) -> Option<Arc<BTreeSet<u32>>>,
    ) -> Option<Arc<BTreeSet<u32>>> {
        match &self.resolved {
            Some((at, cached, owners)) if *at == tables && cached == filter => owners.clone(),
            _ => {
                let owners = read(filter);
                self.resolved = Some((tables, filter.clone(), owners.clone()));
                owners
            }
        }
    }
}

/// All native icon views use one lock when checking whether another frame is needed.
#[derive(Debug, Default)]
pub struct Views {
    pub naval: naval_windows::Windows,
    pub labels: viewer_world::map::labels::LabelView,
    pub pings: PoolView,
    pub raids: PoolView,
    /// The Operatives map mode's state containers and their entries.
    pub intel: viewer_raids::map::intel_icons::IntelView,
    pub bases: viewer_counters::map::counters::BaseView,
    pub land_combats: viewer_counters::map::counters::land_windows::View,
    pub resources: viewer_world::map::resource_icons::ResourceIconView,
    pub occupation: viewer_world::map::occupation_icons::OccupationIconView,
    pub adjacency_rules: PoolView,
}

impl Views {
    pub fn retries(&self) -> Vec<Retry> {
        let mut retries = self.naval.retries();
        retries.extend(self.labels.retries());
        retries.extend(self.pings.retry());
        retries.extend(self.raids.retry());
        retries.extend(self.bases.retries());
        retries.extend(self.land_combats.retry());
        retries.extend(self.resources.retry());
        retries.extend(self.occupation.retry());
        retries.extend(self.adjacency_rules.retry());
        retries
    }

    /// Presents the next frames for the pools' passes alone ([`PoolView::set_quiet`]).
    pub fn set_quiet(&mut self, quiet: bool) {
        self.naval.set_quiet(quiet);
        self.labels.set_quiet(quiet);
        self.pings.set_quiet(quiet);
        self.raids.set_quiet(quiet);
        self.bases.set_quiet(quiet);
        self.land_combats.set_quiet(quiet);
        self.adjacency_rules.set_quiet(quiet);
    }

    /// Whether a window changed on its own at the last frame, not cleared: beside it a family runs
    /// only for its manager's pass (a dirty province keeping an icon off screen). The intel and
    /// occupation views count any running as changed.
    pub fn running_visibly(&self) -> bool {
        self.naval.running_visibly()
            || self.labels.running_visibly()
            || self.pings.running_visibly()
            || self.raids.running_visibly()
            || self.intel.running()
            || self.bases.running_visibly()
            || self.land_combats.running_visibly()
            || self.resources.running_visibly()
            || self.occupation.running()
            || self.adjacency_rules.running_visibly()
    }

    pub fn take_running(&mut self) -> Pools {
        let running = Pools {
            naval: self.naval.take_active(),
            labels: self.labels.take_running(),
            pings: self.pings.take_running(),
            raids: self.raids.take_running(),
            intel: self.intel.take_running(),
            bases: self.bases.take_running(),
            land_combats: self.land_combats.take_running(),
            resources: self.resources.take_running(),
            occupation: self.occupation.take_running(),
            adjacency_rules: self.adjacency_rules.take_running(),
        };
        for (running, name) in running.named() {
            viewer_foundation::map::redraws::note(name, u64::from(running));
        }
        running
    }
}

/// What one map view's counters keep from one update to the next, as the game's persistent
/// counter objects do: owned by the view (the window, a dashboard map, a dump), never shared
/// between views through the content. The view's clock is not here: each presented frame is
/// given it ([`PreparedCounters::draw`]).
#[derive(Debug, Default)]
pub struct Presentation {
    /// Map-icon window histories, touched by presentation, never by a prepared hour.
    pub windows: Mutex<Views>,
    /// Camera-independent counter programs, separate from native lifecycle state.
    pub counters: Mutex<viewer_counters::map::counters::Preparation>,
    /// The view's province schedule and dirty bits, which its scene advances each frame and its
    /// map icons read ([`crate::map::province_schedule`]).
    schedule: Shared,
    /// The view's unit presentation history, which its frame clocks and its counters and figures
    /// read.
    movement: SharedMovement,
}

impl Presentation {
    /// The view's province schedule, for its scene to run on.
    pub fn schedule(&self) -> &Shared {
        &self.schedule
    }

    pub fn movement(&self) -> &SharedMovement {
        &self.movement
    }

    /// Memories of its own over this view's province schedule and movement history: what lays the
    /// overlay out off the frame thread prepares on it, so the frame thread never waits for the
    /// view's own memory ([`Self::take_memory`]).
    pub fn sharing(&self) -> Self {
        Self {
            windows: Default::default(),
            counters: Default::default(),
            schedule: Arc::clone(&self.schedule),
            movement: Arc::clone(&self.movement),
        }
    }

    /// A presentation standing on its own: a copy of this one's province schedule, no unit
    /// animation, no window history and no memory, for drawing a layout apart from the view
    /// (the overlay's own gate).
    pub fn snapshot(&self) -> Self {
        let schedule = self.schedule.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        Self {
            windows: Default::default(),
            counters: Default::default(),
            schedule: Arc::new(Mutex::new(schedule)),
            movement: Default::default(),
        }
    }

    /// Takes the counters' memory a layout was laid out on, which is bound to that layout's tables
    /// and windows, and gives it this one's: the two memories alternate, and the frame draws the
    /// quads of the layout over the memory they were laid out on. The maker lays out only while no
    /// layout it published stands untaken ([`crate::playhead_host::overlay::lays`]), so the memory
    /// handed over here is never the one it is writing.
    pub fn take_memory(&self, laid: &Presentation) {
        let mut ours = self.counters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut theirs = laid.counters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::swap(&mut *ours, &mut *theirs);
    }

    /// Gives `laid`'s counters' memory the products this view shows, before it lays out on it.
    pub fn lend_layered(&self, laid: &Presentation) {
        let ours = self.counters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        laid.counters.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).adopt_layered(&ours);
    }

    /// Whether a map-icon window changed on its own or a unit still moves at the last presented
    /// frame, cleared: a view draws its retained overlay again on the next one while one does, as
    /// the game ticks every window and animator each frame. The hour/camera preparation does not
    /// read those clocks.
    pub fn take_running(&self) -> Running {
        let (windows, visible) = {
            let mut views = self.windows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let visible = views.running_visibly();
            (views.take_running(), visible)
        };
        let moving = self
            .movement
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .running();
        viewer_foundation::map::redraws::note("running.movement", u64::from(moving));
        // A unit still moving asks for the next frame but neither for a painting nor for the
        // pools' passes: its counter's window stands on a track the display frame evaluates at its
        // own head ([`viewer_foundation::map::anchored`]), so the overlay is painted for what
        // changes in it, presented for a pool that has a pass to run, and never for motion.
        Running { any: windows.any() || moving, visible, windows }
    }
}

/// What runs at the last presented frame ([`Presentation::take_running`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Running {
    /// Something asks for the next frame.
    pub any: bool,
    /// A window changed on its own or one asked for is still to be made: the next frame paints the
    /// batch over the pools it presents.
    pub visible: bool,
    /// The families of windows that have a pass to run on the next frame. With none the frame was
    /// asked for by a mover alone, which it answers by moving the windows it holds: presenting the
    /// pools would run every manager's pass over every window for a frame in which none of them
    /// has anything to do.
    pub windows: Pools,
}

/// One map view's counter options and memory, which its overlays are laid out with. Two views
/// are the same only when they share the memory.
#[derive(Clone, Debug, Default)]
pub struct CounterView {
    pub settings: CounterSettings,
    pub selected: Arc<BTreeSet<Unit>>,
    pub air_selected: Arc<Vec<u64>>,
    pub air_bases: Arc<Vec<u64>>,
    pub fleets_selected: Arc<Vec<u64>>,
    pub hovered: Option<Unit>,
    /// The counter whose `count_bg` the primary button went down on, while it is held.
    pub pressed: Option<Unit>,
    /// The counter whose `count_bg` is under the pointer.
    pub plate_hovered: Option<Unit>,
    /// Whether the view shows the Operatives map mode, whose intel icons it then draws.
    pub operatives: bool,
    /// The exact occupation mode; both share the same counter visibility.
    pub occupation: Option<viewer_world::map::occupation_icons::Mode>,
    pub presentation: Arc<Presentation>,
    /// What the view's map mode shows of the counters, icons and labels.
    pub visibility: viewer_foundation::map::mode_visibility::Visibility,
}

impl CounterView {
    /// Resolve the mode together so shared visibility cannot hide a label-mode change.
    pub fn set_mode(&mut self, mode: &viewer_world::map::map_mode::MapMode) {
        use viewer_world::map::{
            map_mode::{MapMode, Stock},
            occupation_icons::Mode,
        };
        self.visibility = viewer_world::map::map_mode::visibility::of(mode);
        self.operatives = matches!(mode, MapMode::Stock(Stock::Operatives));
        self.occupation = match mode {
            MapMode::Stock(Stock::Resistance) => Some(Mode::Resistance),
            MapMode::Stock(Stock::Compliance) => Some(Mode::Compliance),
            _ => None,
        };
    }

    /// A new view with no counter history, as a freshly loaded game.
    pub fn new(settings: CounterSettings) -> Self {
        Self {
            settings,
            selected: Arc::default(),
            air_selected: Arc::default(),
            air_bases: Arc::default(),
            fleets_selected: Arc::default(),
            hovered: None,
            pressed: None,
            plate_hovered: None,
            operatives: false,
            occupation: None,
            presentation: Arc::default(),
            visibility: Default::default(),
        }
    }
}

impl PartialEq for CounterView {
    fn eq(&self, other: &Self) -> bool {
        self.settings == other.settings
            && self.selected == other.selected
            && self.air_selected == other.air_selected
            && self.air_bases == other.air_bases
            && self.fleets_selected == other.fleets_selected
            && self.hovered == other.hovered
            && self.operatives == other.operatives
            && self.occupation == other.occupation
            && Arc::ptr_eq(&self.presentation, &other.presentation)
            && self.visibility == other.visibility
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_core::protocol::CounterFilter;

    /// The maker lays a layout out on a memory of its own and the frame takes that memory with it;
    /// the two then alternate, so the memory the maker lays the next layout on is the one the frame
    /// has handed back and never the one it draws.
    #[test]
    fn the_memory_a_layout_was_laid_out_on_is_taken_with_it_and_the_two_alternate() {
        use viewer_counters::map::counters::{BaseValues, Kinds, Layered};
        let view = Presentation::default();
        let maker = view.sharing();
        // Each layout leaves a product of its own on the memory it was laid out on.
        let product = |memory: &Presentation| {
            let bases = Arc::new(BaseValues::default());
            let at = Arc::as_ptr(&bases) as usize;
            memory.counters.lock().unwrap().set_layered(Layered { bases: Some(bases), ..Layered::default() }, Kinds::BASES, Kinds::BASES);
            at
        };
        let drawing = || view.counters.lock().unwrap().layered().identity()[3];
        let making = || maker.counters.lock().unwrap().layered().identity()[3];
        let held = product(&view);
        let first = product(&maker);
        view.take_memory(&maker);
        assert_eq!(drawing(), first, "the view draws on the memory that layout was laid out on");
        assert_eq!(making(), held, "and the maker holds the memory the frame handed back");
        // The maker's next layout goes on that memory, never on the one the frame draws.
        let second = product(&maker);
        assert_eq!(drawing(), first, "what the frame draws is untouched while the next is laid out");
        view.take_memory(&maker);
        assert_eq!(drawing(), second);
    }

    /// What asks for the next frame and what asks for the pools' passes are separate: a unit still
    /// moving asks for the frame alone, which places the windows it holds at its own head, and
    /// only a pool with a pass to run asks for the pools to be presented.
    #[test]
    fn only_a_pool_with_a_pass_to_run_asks_for_the_pools_to_be_presented() {
        let view = Presentation::default();
        let idle = Running { any: false, visible: false, windows: Pools::default() };
        assert_eq!(view.take_running(), idle, "nothing asks for anything");
        view.windows.lock().unwrap().pings.keep_running();
        let running = view.take_running();
        // The family that has a pass to run asks for it, and no other family is presented for it.
        assert_eq!(
            running,
            Running {
                any: true,
                visible: false,
                windows: Pools { pings: true, ..Pools::default() },
            }
        );
        assert_eq!(view.take_running(), idle, "taken, so the frame after it asks for nothing");
    }

    #[test]
    fn occupation_mode_changes_invalidate_shared_visibility_and_clear_locally() {
        use viewer_world::map::{
            map_mode::{MapMode, Stock},
            occupation_icons::Mode,
        };
        let mut resistance = CounterView::default();
        resistance.set_mode(&MapMode::Stock(Stock::Resistance));
        let mut compliance = resistance.clone();
        compliance.set_mode(&MapMode::Stock(Stock::Compliance));
        assert_eq!(resistance.visibility, compliance.visibility);
        assert_eq!(resistance.occupation, Some(Mode::Resistance));
        assert_eq!(compliance.occupation, Some(Mode::Compliance));
        assert_ne!(resistance, compliance);
        compliance.set_mode(&MapMode::default());
        assert_eq!(compliance.occupation, None);
        assert_eq!(resistance.occupation, Some(Mode::Resistance));
    }

    #[test]
    fn a_changed_air_selection_invalidates_the_prepared_view() {
        let first = CounterView::default();
        let mut selected = first.clone();
        selected.air_selected = Arc::new(vec![7]);
        assert_ne!(first, selected);
        assert_eq!(selected, selected.clone());
        selected = first.clone();
        selected.air_bases = Arc::new(vec![9]);
        assert_ne!(first, selected);
        selected = first.clone();
        selected.fleets_selected = Arc::new(vec![11]);
        assert_ne!(first, selected);
    }

    #[test]
    fn players_are_placed_through_the_links_and_an_unplaced_one_draws_everyone() {
        let links = [(10, 1), (20, 2), (30, 3)];
        assert_eq!(player_slots(links, [30, 10]), Some(BTreeSet::from([1, 3])));
        // A link missing for one human-controlled country never hides that player's counters.
        assert_eq!(player_slots(links, [10, 40]), None);
        assert_eq!(player_slots(links, []), Some(BTreeSet::new()));
    }

    #[test]
    fn chosen_tags_pick_their_slots() {
        let countries = [(&b"GER"[..], 4), (&b"ENG"[..], 7), (&b"SOV"[..], 9)];
        assert_eq!(
            chosen_slots(&["SOV".into(), "GER".into(), "XXX".into()], countries),
            BTreeSet::from([4, 9])
        );
    }

    #[test]
    fn settings_are_resolved_again_only_for_another_generation_or_scope() {
        let mut settings = ResolvedSettings::default();
        let mut resolved = Vec::new();
        let mut resolve = |scope: &str| {
            resolved.push(scope.to_owned());
            dashboard_core::protocol::ResolvedMapSettings::default()
        };
        let first = settings.get(1, "c".into(), &mut resolve);
        let again = settings.get(1, "c".into(), &mut resolve);
        assert!(Arc::ptr_eq(&first, &again));
        settings.get(2, "c".into(), &mut resolve);
        settings.get(2, "c/widget/a".into(), &mut resolve);
        settings.get(2, "c/widget/a".into(), &mut resolve);
        assert_eq!(resolved, ["c", "c", "c/widget/a"]);
    }

    #[test]
    fn owners_are_read_again_only_for_other_tables_or_another_filter() {
        use replay_frame::{frame::SharedView, frame::TableView};
        let view = || -> SharedView {
            TableView::Unavailable {
                reason: "synthetic".into(),
            }
            .into()
        };
        // One recording's tables at an hour, the same allocations at a later hour that left
        // them unchanged, and another recording's equal tables at that same hour.
        let first: [SharedView; 3] = std::array::from_fn(|_| view());
        let other: [SharedView; 3] = std::array::from_fn(|_| view());
        assert_eq!(first, other);
        let identities = |tables: &[SharedView; 3]| tables.each_ref().map(SharedView::identity);
        let mut owners = CounterOwners::default();
        let mut reads = 0;
        let mut read = |filter: &CounterFilter| {
            reads += 1;
            (*filter != CounterFilter::All).then(|| Arc::new(BTreeSet::from([reads])))
        };
        let shown = owners.get_with(identities(&first), &CounterFilter::Players, &mut read);
        let again = owners.get_with(identities(&first), &CounterFilter::Players, &mut read);
        assert!(Arc::ptr_eq(
            shown.as_ref().unwrap(),
            again.as_ref().unwrap()
        ));
        // Another recording at the same branch, hour and filter never reuses these owners.
        let switched = owners.get_with(identities(&other), &CounterFilter::Players, &mut read);
        assert_eq!(switched, Some(Arc::new(BTreeSet::from([2]))));
        // One changed table is enough to read again.
        let mut changed = other.clone();
        changed[1] = view();
        owners.get_with(identities(&changed), &CounterFilter::Players, &mut read);
        owners.get_with(identities(&changed), &CounterFilter::All, &mut read);
        assert_eq!(
            owners.get_with(identities(&changed), &CounterFilter::All, &mut read),
            None
        );
        assert_eq!(reads, 4);
    }
}
