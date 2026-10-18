//! Per-view composition of counter options, icon histories and the shared schedule.

use std::sync::Mutex;
use std::{collections::BTreeSet, sync::Arc};
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
    filter: &dashboard::protocol::CounterFilter,
) -> Option<Arc<BTreeSet<u32>>> {
    use dashboard::protocol::CounterFilter;
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
    read: Option<(u64, String, Arc<dashboard::protocol::ResolvedMapSettings>)>,
}

impl ResolvedSettings {
    pub fn get(
        &mut self,
        generation: u64,
        scope: String,
        resolve: impl FnOnce(&str) -> dashboard::protocol::ResolvedMapSettings,
    ) -> Arc<dashboard::protocol::ResolvedMapSettings> {
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
/// come from changes, so a frame never re-reads the hour's tables.
#[derive(Debug, Default)]
pub struct CounterOwners {
    resolved: Option<ResolvedOwners>,
}

/// The table allocations and filter the owners were read for, and the owners. An allocation
/// belongs to one recording's decoder and is shared only by hours whose table did not change,
/// so another recording, branch or changed hour reads again.
type ResolvedOwners = (
    [replay_frame::frame::ViewIdentity; OWNER_TABLES.len()],
    dashboard::protocol::CounterFilter,
    Option<Arc<BTreeSet<u32>>>,
);

impl CounterOwners {
    pub fn get(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        filter: &dashboard::protocol::CounterFilter,
    ) -> Option<Arc<BTreeSet<u32>>> {
        let tables = OWNER_TABLES.map(|table| frame.tables[table.index()].identity());
        self.get_with(tables, filter, |filter| counter_owners(frame, filter))
    }

    fn get_with(
        &mut self,
        tables: [replay_frame::frame::ViewIdentity; OWNER_TABLES.len()],
        filter: &dashboard::protocol::CounterFilter,
        read: impl FnOnce(&dashboard::protocol::CounterFilter) -> Option<Arc<BTreeSet<u32>>>,
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
    pub bases: [PoolView; viewer_counters::map::counters::BASE_FAMILIES],
    pub land_combats: viewer_counters::map::counters::land_windows::View,
    pub resources: viewer_world::map::resource_icons::ResourceIconView,
    pub adjacency_rules: PoolView,
}

impl Views {
    pub(crate) fn retries(&self) -> Vec<Retry> {
        let mut retries = self.naval.retries();
        retries.extend(self.labels.retries());
        retries.extend(self.pings.retry());
        retries.extend(self.raids.retry());
        retries.extend(self.bases.iter().filter_map(PoolView::retry));
        retries.extend(self.land_combats.retry());
        retries.extend(self.resources.retry());
        retries.extend(self.adjacency_rules.retry());
        retries
    }

    pub fn take_running(&mut self) -> bool {
        let naval = self.naval.take_active();
        let labels = self.labels.take_running();
        let pings = self.pings.take_running();
        let raids = self.raids.take_running();
        let intel = self.intel.take_running();
        let mut bases = false;
        for view in &mut self.bases {
            bases |= view.take_running();
        }
        let land_combats = self.land_combats.take_running();
        let resources = self.resources.take_running();
        let adjacency_rules = self.adjacency_rules.take_running();
        naval
            || labels
            || pings
            || raids
            || intel
            || bases
            || land_combats
            || resources
            || adjacency_rules
    }
}

/// What one map view's counters keep from one update to the next, as the game's persistent
/// counter objects do: owned by the view (the window, a dashboard map, a dump), never shared
/// between views through the content. The view's clock is not here: each presented frame is
/// given it ([`PreparedCounters::draw`]).
#[derive(Debug, Default)]
pub struct Presentation {
    /// Map-icon window histories, touched by presentation, never by a prepared hour.
    pub(crate) windows: Mutex<Views>,
    /// The view's province schedule and dirty bits, which its scene advances each frame and its
    /// map icons read ([`crate::map::province_schedule`]).
    schedule: Shared,
}

impl Presentation {
    /// The view's province schedule, for its scene to run on.
    pub fn schedule(&self) -> &Shared {
        &self.schedule
    }

    /// Whether a map-icon window changed on its own at the last presented frame, cleared: a view
    /// that shows frames lays its overlay out again on the next one while one does, as the game
    /// ticks every window each frame.
    pub fn take_running(&self) -> bool {
        self.windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take_running()
    }
}

/// One map view's counter options and memory, which its overlays are laid out with. Two views
/// are the same only when they share the memory.
#[derive(Clone, Debug, Default)]
pub struct CounterView {
    pub settings: CounterSettings,
    pub selected: Arc<BTreeSet<Unit>>,
    pub hovered: Option<Unit>,
    /// Whether the view shows the Operatives map mode, whose intel icons it then draws.
    pub operatives: bool,
    pub presentation: Arc<Presentation>,
    /// What the view's map mode shows of the counters, icons and labels.
    pub visibility: viewer_foundation::map::mode_visibility::Visibility,
}

impl CounterView {
    /// A new view with no counter history, as a freshly loaded game.
    pub fn new(settings: CounterSettings) -> Self {
        Self {
            settings,
            selected: Arc::default(),
            hovered: None,
            operatives: false,
            presentation: Arc::default(),
            visibility: Default::default(),
        }
    }
}

impl PartialEq for CounterView {
    fn eq(&self, other: &Self) -> bool {
        self.settings == other.settings
            && self.selected == other.selected
            && self.hovered == other.hovered
            && self.operatives == other.operatives
            && Arc::ptr_eq(&self.presentation, &other.presentation)
            && self.visibility == other.visibility
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard::protocol::CounterFilter;

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
            dashboard::protocol::ResolvedMapSettings::default()
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
