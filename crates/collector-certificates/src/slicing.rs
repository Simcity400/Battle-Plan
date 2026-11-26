
use replay_data::rows::TableId;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    Country,
    CanonicalSlot,
    State,
}

pub struct SliceFamily {
    pub name: &'static str,
    pub owner: OwnerKind,
    pub tables: &'static [TableId],
}

pub const RESEARCH: usize = 0;
pub const TECHNOLOGIES: usize = 1;
pub const RAID_LAUNCH: usize = 2;
pub const RAID_STATE: usize = 3;
pub const RAID_BUILDINGS: usize = 4;
pub const RELATIONS: usize = 5;
pub const STABILITY: usize = 6;
pub const AIR_RELATIONS: usize = 7;
pub const AIR_HISTORIES: usize = 8;
pub const NAVAL_MISSIONS: usize = 9;
pub const COMBAT_LOG: usize = 10;
pub const SUPPLY_TRAINS: usize = 11;
pub const AVAILABLE_ADVISORS: usize = 12;
pub const FOCUS: usize = 13;
pub const PROJECTS: usize = 14;
pub const TRADE_ROUTES: usize = 15;

pub const FAMILIES: [SliceFamily; 16] = [
    SliceFamily {
        name: "research",
        owner: OwnerKind::Country,
        tables: &[
            TableId::ResearchInputs,
            TableId::ResearchAppliedBonuses,
            TableId::ResearchModifiers,
        ],
    },
    SliceFamily {
        name: "technologies",
        owner: OwnerKind::Country,
        tables: &[
            TableId::ResearchTechnologies,
            TableId::TechnologyDefinitions,
        ],
    },
    SliceFamily {
        name: "raid_launch",
        owner: OwnerKind::CanonicalSlot,
        tables: &[TableId::RaidAirSites],
    },
    SliceFamily {
        name: "raid_state",
        owner: OwnerKind::State,
        tables: &[
            TableId::StateResourceInputs,
            TableId::StateResources,
            TableId::StateDynamicModifiers,
            TableId::StateCores,
        ],
    },
    SliceFamily {
        name: "raid_country_buildings",
        owner: OwnerKind::Country,
        tables: &[TableId::CountryBuildingResources],
    },
    SliceFamily {
        name: "relations",
        owner: OwnerKind::Country,
        tables: &[TableId::RelationOpinions, TableId::RelationTuples],
    },
    SliceFamily {
        name: "stability",
        owner: OwnerKind::Country,
        tables: &[
            TableId::CountryStabilityInputs,
            TableId::CountryStabilityLines,
        ],
    },
    SliceFamily {
        name: "air_relations",
        owner: OwnerKind::CanonicalSlot,
        tables: &[TableId::AirCountryRelations],
    },
    SliceFamily {
        name: "air_histories",
        owner: OwnerKind::CanonicalSlot,
        tables: &[TableId::AirNavalPatrolHistories],
    },
    SliceFamily {
        name: "naval_missions",
        owner: OwnerKind::Country,
        tables: &[
            TableId::NavalMissionCountries,
            TableId::NavalMissionCommanders,
        ],
    },
    SliceFamily {
        name: "combat_log",
        owner: OwnerKind::Country,
        tables: &[TableId::CombatLogEquipment],
    },
    SliceFamily {
        name: "supply_trains",
        owner: OwnerKind::Country,
        tables: &[TableId::SupplyNodes, TableId::TrainInventory],
    },
    SliceFamily {
        name: "available_advisors",
        owner: OwnerKind::Country,
        tables: &[TableId::AvailableAdvisors],
    },
    SliceFamily {
        name: "focus",
        owner: OwnerKind::Country,
        tables: &[TableId::Focus, TableId::CompletedFocuses],
    },
    SliceFamily {
        name: "projects",
        owner: OwnerKind::Country,
        tables: &[TableId::Projects, TableId::ProjectHistory],
    },
    SliceFamily {
        name: "trade_routes",
        owner: OwnerKind::Country,
        tables: &[TableId::TradeRoutes],
    },
];

pub fn scan_countries(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    hour: i32,
    force_full: bool,
    selection: &mut SliceSelection,
    fingerprints: Option<&mut SliceFingerprints>,
) {
    selection.reset(hour, force_full || fingerprints.is_none());
    let Some(store) = fingerprints else {
        return;
    };
    let started = crate::clock::qpc();
    let census = match engine_native::ccountry::Reader::new(memory, base, game_state) {
        Err(_) => false,
        Ok(countries) => {
            let mut complete = true;
            for slot in 1..countries.count() {
                match countries.at(slot) {
                    Ok(Some(country)) => selection.scan(store, country.id, Some(0)),
                    Ok(None) => {}
                    Err(_) => complete = false,
                }
            }
            complete
        }
    };
    store.finish_scan();
    selection.finish_pass(store, census);
    selection.scan_ticks = crate::clock::qpc().saturating_sub(started).max(0) as u64;
}

pub fn family_of(table: TableId) -> Option<usize> {
    FAMILIES
        .iter()
        .position(|family| family.tables.contains(&table))
}

const BITSET_WORDS: usize = 32;
const BITSET_BITS: usize = BITSET_WORDS * 64;

struct OwnerSet {
    owners: Vec<u32>,
    bits: [u64; BITSET_WORDS],
}

impl OwnerSet {
    const fn new() -> Self {
        Self {
            owners: Vec::new(),
            bits: [0; BITSET_WORDS],
        }
    }

    fn clear(&mut self) {
        if !self.owners.is_empty() {
            self.owners.clear();
            self.bits = [0; BITSET_WORDS];
        }
    }

    fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    fn as_slice(&self) -> &[u32] {
        &self.owners
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.owners.capacity()
    }

    #[inline]
    fn contains(&self, owner: u32) -> bool {
        let index = owner as usize;
        if index < BITSET_BITS {
            (self.bits[index / 64] >> (index % 64)) & 1 != 0
        } else {
            self.owners.binary_search(&owner).is_ok()
        }
    }

    fn insert(&mut self, owner: u32) -> bool {
        let index = owner as usize;
        if index < BITSET_BITS && (self.bits[index / 64] >> (index % 64)) & 1 != 0 {
            return true;
        }
        let pos = match self.owners.last() {
            Some(&last) if last < owner => self.owners.len(),
            None => 0,
            Some(_) => match self.owners.binary_search(&owner) {
                Ok(_) => return true,
                Err(pos) => pos,
            },
        };
        if engine_allocation::reserve_push_batch(&mut self.owners, 1).is_err() {
            return false;
        }
        self.owners.insert(pos, owner);
        if index < BITSET_BITS {
            self.bits[index / 64] |= 1_u64 << (index % 64);
        }
        true
    }
}

pub struct SliceSelection {
    pub hour: i32,
    pub force_full: bool,
    daily: bool,
    boundary: bool,
    settled: OwnerSet,
    early: OwnerSet,
    pub early_count: u32,
    seen: OwnerSet,
    census: bool,
    walked: OwnerSet,
    tracked: bool,
    marked: OwnerSet,
    pub marked_count: u32,
    pub scanned: u32,
    pub out_of_range: u32,
    pub scan_ticks: u64,
}

impl Default for SliceSelection {
    fn default() -> Self {
        Self::new()
    }
}

impl SliceSelection {
    pub fn new() -> Self {
        Self {
            hour: 0,
            force_full: true,
            daily: false,
            boundary: false,
            settled: OwnerSet::new(),
            early: OwnerSet::new(),
            early_count: 0,
            seen: OwnerSet::new(),
            census: false,
            walked: OwnerSet::new(),
            tracked: false,
            marked: OwnerSet::new(),
            marked_count: 0,
            scanned: 0,
            out_of_range: 0,
            scan_ticks: 0,
        }
    }

    pub fn reset(&mut self, hour: i32, force_full: bool) {
        self.hour = hour;
        self.force_full = force_full;
        self.daily = false;
        self.boundary = false;
        self.settled.clear();
        self.early.clear();
        self.early_count = 0;
        self.seen.clear();
        self.walked.clear();
        self.census = false;
        self.tracked = false;
        self.marked.clear();
        self.marked_count = 0;
        self.scanned = 0;
        self.out_of_range = 0;
        self.scan_ticks = 0;
    }

    pub fn reset_daily(&mut self, hour: i32, force_full: bool) {
        self.reset(hour, force_full);
        self.daily = true;
        self.boundary = hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD) == 0;
    }

    pub fn reset_hooked(&mut self, hour: i32, force_full: bool) {
        self.reset(hour, force_full);
        self.daily = true;
    }

    pub fn extend_reads(&mut self, scheduled: &Self, extra: impl Fn(u32) -> bool) {
        self.reset_hooked(scheduled.hour, scheduled.force_full);
        self.census = scheduled.census;
        self.scan_ticks = scheduled.scan_ticks;
        self.scanned = scheduled.scanned;
        for &owner in scheduled.seen.as_slice() {
            if !self.seen.insert(owner) {
                self.out_of_range += 1;
                self.force_full = true;
            }
            if scheduled.contains(owner) || extra(owner) {
                self.select(owner);
            }
        }
        for &owner in scheduled.marked.as_slice() {
            self.mark(owner);
        }
    }

    pub fn reads_any(&self) -> bool {
        self.force_full
            || !self.early.is_empty()
            || self.seen.as_slice().iter().any(|&owner| self.contains(owner))
    }

    pub fn carried(&self) -> usize {
        self.seen
            .as_slice()
            .iter()
            .filter(|&&owner| !self.contains(owner))
            .count()
    }

    pub fn scan_lazily(
        &mut self,
        fingerprints: &mut SliceFingerprints,
        owner: u32,
        drifts: impl FnOnce() -> bool,
    ) {
        if self.force_full || self.in_slice(owner, false) || !fingerprints.holds(owner) {
            let drifts = drifts();
            self.scan_drifting(fingerprints, owner, Some(u64::from(drifts)), drifts);
            return;
        }
        self.scanned += 1;
        if !self.seen.insert(owner) {
            self.out_of_range += 1;
            self.force_full = true;
        }
    }

    pub fn scan_drifting(
        &mut self,
        fingerprints: &mut SliceFingerprints,
        owner: u32,
        fingerprint: Option<u64>,
        drifts: bool,
    ) {
        if !drifts && !self.settled.insert(owner) {
            self.out_of_range += 1;
            self.force_full = true;
            return;
        }
        self.scan(fingerprints, owner, fingerprint);
    }

    pub fn scan(
        &mut self,
        fingerprints: &mut SliceFingerprints,
        owner: u32,
        fingerprint: Option<u64>,
    ) {
        self.scanned += 1;
        if !self.seen.insert(owner) {
            self.out_of_range += 1;
            self.force_full = true;
            return;
        }
        if fingerprints.observe(owner, fingerprint) && !self.contains(owner) {
            if self.early.insert(owner) {
                self.early_count += 1;
            } else {
                self.out_of_range += 1;
                self.force_full = true;
            }
        }
    }

    pub fn scan_marking(
        &mut self,
        fingerprints: &mut SliceFingerprints,
        owner: u32,
        fingerprint: Option<u64>,
    ) {
        self.scanned += 1;
        if !self.seen.insert(owner) {
            self.out_of_range += 1;
            self.force_full = true;
            return;
        }
        if fingerprints.observe(owner, fingerprint) {
            self.mark(owner);
        }
    }

    pub fn select(&mut self, owner: u32) -> bool {
        if !self.contains(owner) {
            if !self.early.insert(owner) {
                self.out_of_range += 1;
                self.force_full = true;
                return false;
            }
            self.early_count += 1;
        }
        true
    }

    pub fn finish_census(&mut self, fingerprints: &mut SliceFingerprints) {
        if self.out_of_range != 0 || engine_allocation::capture_stopped() {
            self.force_full = true;
            return;
        }
        self.census = true;
        fingerprints.keep_only(self.seen.as_slice());
    }

    pub fn finish_pass(&mut self, fingerprints: &mut SliceFingerprints, complete: bool) {
        if complete {
            self.finish_census(fingerprints);
        } else {
            self.force_full = true;
        }
    }

    #[inline]
    pub fn live(&self, owner: u32) -> bool {
        !self.census || self.seen.contains(owner)
    }

    #[inline]
    pub fn contains(&self, owner: u32) -> bool {
        self.force_full
            || (self.in_slice(owner, false) && !self.settled.contains(owner))
            || self.early.contains(owner)
    }

    fn in_slice(&self, owner: u32, force_full: bool) -> bool {
        force_full
            || if self.daily {
                self.boundary
            } else {
                replay_data::cadence::slice_active(owner, self.hour, false)
            }
    }

    pub fn mark(&mut self, owner: u32) {
        let new = !self.marked.contains(owner);
        if !self.marked.insert(owner) {
            self.out_of_range += 1;
            self.force_full = true;
            return;
        }
        if new {
            self.marked_count += 1;
        }
        self.select(owner);
    }

    pub fn certify(&mut self, store: &mut SliceFingerprints, epoch: Option<u64>) {
        if epoch.is_none() || epoch != store.marks {
            self.force_full = true;
        }
        store.marks = epoch;
    }

    pub fn marked_early(&self) -> u32 {
        self.marked
            .as_slice()
            .iter()
            .filter(|&&owner| !self.in_slice(owner, false))
            .count() as u32
    }

    pub fn track_reads(&mut self) {
        self.tracked = true;
        self.walked.clear();
    }

    pub fn mark_read(&mut self, owner: u32) {
        if !self.walked.insert(owner) {
            self.out_of_range += 1;
            self.force_full = true;
        }
    }

    pub fn read_owners(&self) -> impl Iterator<Item = u32> + '_ {
        self.seen
            .as_slice()
            .iter()
            .copied()
            .filter(|&owner| self.contains(owner))
            .filter(|&owner| !self.tracked || self.walked.contains(owner))
    }

    pub fn invalidate(&self, fingerprints: &mut SliceFingerprints) {
        for &owner in self.early.as_slice() {
            fingerprints.invalidate(owner);
        }
        fingerprints.stale_slices |= if self.force_full || (self.daily && self.boundary) {
            u32::MAX
        } else if self.daily {
            0
        } else {
            1 << self.hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD)
        };
        fingerprints.held.epoch = None;
        fingerprints.secondary.epoch = None;
        fingerprints.secondary.values.clear();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(hour: i32, force_full: bool, early: &[u32]) -> Self {
        let mut selection = Self::new();
        selection.reset(hour, force_full);
        for &owner in early {
            assert!(selection.early.insert(owner));
            selection.early_count += 1;
        }
        selection
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_census(mut self, seen: &[u32]) -> Self {
        for &owner in seen {
            assert!(self.seen.insert(owner));
        }
        self.census = true;
        self
    }
}

pub struct SliceFingerprints {
    values: HashMap<u32, Fingerprint>,
    stale_slices: u32,
    pub held: Held,
    secondary: SecondaryHeld,
    marks: Option<u64>,
    pub drift: crate::drift::Store,
    pub witnesses: Witnesses,
}
struct Fingerprint {
    value: Option<u64>,
    stale: bool,
}

impl Default for SliceFingerprints {
    fn default() -> Self {
        Self::new()
    }
}

impl SliceFingerprints {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            stale_slices: 0,
            held: Held::default(),
            secondary: SecondaryHeld::default(),
            marks: None,
            drift: Default::default(),
            witnesses: Witnesses::default(),
        }
    }

    pub fn forget(&mut self) {
        self.values.clear();
        self.stale_slices = 0;
        self.held = Held::default();
        self.secondary.epoch = None;
        self.secondary.context = None;
        self.secondary.values.clear();
        self.marks = None;
        self.drift.clear();
        self.witnesses.forget();
    }

    fn keep_only(&mut self, seen: &[u32]) {
        self.values
            .retain(|owner, _| seen.binary_search(owner).is_ok());
        self.secondary
            .values
            .retain(|owner, _| seen.binary_search(owner).is_ok());
        self.drift.retain(seen);
    }

    pub fn invalidate(&mut self, owner: u32) {
        if let Some(value) = self.values.get_mut(&owner) {
            value.stale = true;
        }
    }

    fn observe(&mut self, owner: u32, value: Option<u64>) -> bool {
        let stale_slice = self.stale_slices & (1 << replay_data::cadence::slice_of(owner)) != 0;
        let next = Fingerprint {
            value,
            stale: value.is_none(),
        };
        if let Some(previous) = self.values.get_mut(&owner) {
            let changed = previous.stale || previous.value != value || value.is_none();
            *previous = next;
            return changed || stale_slice;
        }
        if engine_allocation::reserve_map(&mut self.values, 1).is_ok() {
            self.values.insert(owner, next);
        }
        true
    }

    fn holds(&self, owner: u32) -> bool {
        self.stale_slices & (1 << replay_data::cadence::slice_of(owner)) == 0
            && self
                .values
                .get(&owner)
                .is_some_and(|print| !print.stale && print.value.is_some())
    }

    pub fn held(&self, owner: u32) -> Option<u64> {
        self.holds(owner)
            .then(|| self.values.get(&owner).and_then(|print| print.value))
            .flatten()
    }

    pub fn hold(&mut self, owner: u32, value: u64) {
        if let Some(print) = self.values.get_mut(&owner)
            && !print.stale
        {
            print.value = Some(value);
        }
    }

    pub fn finish_scan(&mut self) {
        self.stale_slices = 0;
    }

    pub fn certify_secondary(
        &mut self,
        changed: &mut Changed,
        take: impl FnOnce(&mut dyn FnMut(usize)) -> Option<u64>,
        context: u64,
    ) {
        changed.reset();
        let marked = &mut changed.marked;
        let mut overflow = false;
        let epoch = take(&mut |id| match u32::try_from(id) {
            Ok(id) => {
                if engine_allocation::reserve_push_batch(marked, 1).is_ok() {
                    marked.push(id);
                } else {
                    overflow = true;
                }
            }
            Err(_) => overflow = true,
        });
        marked.sort_unstable();
        marked.dedup();
        changed.certified = !overflow
            && epoch.is_some()
            && epoch == self.secondary.epoch
            && self.secondary.context == Some(context);
        if !changed.certified {
            self.secondary.values.clear();
        } else {
            for &owner in &changed.marked {
                self.secondary.values.remove(&owner);
            }
        }
        self.secondary.epoch = if overflow { None } else { epoch };
        self.secondary.context = Some(context);
    }

    pub fn secondary_held(&self, owner: u32) -> Option<u64> {
        self.secondary.values.get(&owner).copied()
    }

    pub fn hold_secondary(&mut self, owner: u32, value: u64) {
        if let Some(slot) = self.secondary.values.get_mut(&owner) {
            *slot = value;
        } else if engine_allocation::reserve_map(&mut self.secondary.values, 1).is_ok() {
            self.secondary.values.insert(owner, value);
        }
    }

    pub fn forget_secondary(&mut self, owner: u32) {
        self.secondary.values.remove(&owner);
    }

    pub fn forget_all_secondary(&mut self) {
        self.secondary.epoch = None;
        self.secondary.values.clear();
    }
}

#[derive(Default)]
pub struct Held {
    epoch: Option<u64>,
    context: Option<u64>,
}

#[derive(Default)]
struct SecondaryHeld {
    epoch: Option<u64>,
    context: Option<u64>,
    values: HashMap<u32, u64>,
}

mod digest;
mod witness;
pub use digest::Digest;
pub use witness::Witnesses;

#[derive(Default)]
pub struct Changed {
    marked: Vec<u32>,
    pub certified: bool,
}

impl Changed {
    pub fn reset(&mut self) {
        self.marked.clear();
        self.certified = false;
    }

    pub fn marked(&self) -> usize {
        self.marked.len()
    }

    pub fn carries(&self, owner: u32) -> bool {
        self.certified && self.marked.binary_search(&owner).is_err()
    }

    pub fn unread(&mut self, store: &mut SliceFingerprints, owner: u32) {
        store.drift.forget(owner);
    }
}

pub fn select_changed(
    selection: &mut SliceSelection,
    store: &mut SliceFingerprints,
    changed: &mut Changed,
    take: impl FnOnce(&mut dyn FnMut(usize)) -> Option<u64>,
    context: u64,
) {
    changed.reset();
    let marked = &mut changed.marked;
    let mut overflow = false;
    let epoch = take(&mut |id| match u32::try_from(id) {
        Ok(id) => {
            if engine_allocation::reserve_push_batch(marked, 1).is_ok() {
                marked.push(id);
            } else {
                overflow = true;
            }
        }
        Err(_) => overflow = true,
    });
    marked.sort_unstable();
    marked.dedup();
    for index in 0..changed.marked.len() {
        let owner = changed.marked[index];
        selection.select(owner);
    }
    changed.certified = !overflow
        && epoch.is_some()
        && epoch == store.held.epoch
        && store.held.context == Some(context);
    if !changed.certified {
        selection.force_full = true;
    }
    store.held.epoch = if overflow { None } else { epoch };
    store.held.context = Some(context);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_scans_mark_owners_whose_certificate_changed() {
        let mut store = SliceFingerprints::new();
        let hour = |store: &mut SliceFingerprints, prints: [u64; 3]| {
            let mut selection = SliceSelection::new();
            selection.reset(0, true);
            for (owner, print) in (1..).zip(prints) {
                selection.scan_marking(store, owner, Some(print));
            }
            selection.finish_census(store);
            selection.marked.as_slice().to_vec()
        };
        assert_eq!(hour(&mut store, [1, 1, 1]), [1, 2, 3]);
        assert_eq!(hour(&mut store, [1, 2, 1]), [2]);
        assert!(hour(&mut store, [1, 2, 1]).is_empty());
    }

    #[test]
    fn failed_owner_bookkeeping_never_certifies_a_partial_census_or_forgets_healthy_owners() {
        let mut original = SliceSelection::new();
        let mut fingerprints = SliceFingerprints::new();
        original.scan(&mut fingerprints, 7, Some(1));
        let mut failed = SliceSelection::new();
        failed.reset(1, false);
        crate::test_support::with_allocations_denied(|| {
            failed.scan(&mut fingerprints, u32::MAX, Some(2));
            failed.finish_census(&mut fingerprints);
        });
        assert!(failed.force_full && !failed.census);
        assert!(failed.live(7));
        assert_eq!(failed.out_of_range, 1);
        assert!(fingerprints.values.contains_key(&7));
        assert!(!fingerprints.values.contains_key(&u32::MAX));
    }

    fn scan(
        selection: &mut SliceSelection,
        store: &mut SliceFingerprints,
        owners: &[(u32, Option<u64>)],
    ) {
        for &(owner, value) in owners {
            selection.scan(store, owner, value);
        }
        store.finish_scan();
    }

    #[test]
    fn new_changed_and_failing_owners_are_read_early_and_unchanged_ones_are_not() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        selection.reset(5, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (3, Some(3)), (29, Some(29))],
        );
        assert_eq!((selection.scanned, selection.early_count), (3, 2));
        assert!(selection.contains(1) && selection.contains(3) && selection.contains(29));
        selection.reset(29, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(100)), (3, Some(3)), (29, Some(290))],
        );
        assert_eq!(selection.early_count, 1);
        assert!(selection.contains(1) && selection.contains(29) && !selection.contains(3));
        for (hour, value, read) in [(6, None, true), (7, None, true), (8, Some(3), true)] {
            selection.reset(hour, false);
            scan(&mut selection, &mut store, &[(3, value)]);
            assert_eq!(selection.contains(3), read, "hour {hour}");
        }
        selection.reset(9, false);
        scan(&mut selection, &mut store, &[(4_000, Some(1))]);
        assert!(selection.contains(4_000));
        store.forget();
        selection.reset(10, false);
        scan(&mut selection, &mut store, &[(1, Some(100)), (3, Some(3))]);
        assert_eq!(selection.early_count, 2);
        selection.reset(11, false);
        scan(
            &mut selection,
            &mut store,
            &[(70_000, Some(1)), (70_000, Some(2))],
        );
        assert_eq!((selection.early_count, selection.out_of_range), (1, 0));
        assert!(selection.contains(70_000) && !selection.contains(70_001));
        let mut full = SliceSelection::new();
        full.reset(10, true);
        assert!(full.contains(12_345));
    }

    #[test]
    fn an_incomplete_pass_reads_every_owner_and_forgets_nobody() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        selection.reset(0, true);
        scan(&mut selection, &mut store, &[(1, Some(1)), (2, Some(2))]);
        selection.finish_pass(&mut store, true);
        selection.reset(3, false);
        scan(&mut selection, &mut store, &[(1, Some(1))]);
        selection.finish_pass(&mut store, false);
        assert!(selection.force_full && selection.contains(2) && selection.contains(1));
        assert!(selection.live(2));
        selection.reset(4, false);
        scan(&mut selection, &mut store, &[(1, Some(1)), (2, Some(2))]);
        selection.finish_pass(&mut store, true);
        assert!(!selection.force_full && !selection.contains(2) && !selection.contains(1));
    }

    #[test]
    fn only_a_complete_census_marks_unseen_owners_gone() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        selection.reset(3, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (70_000, Some(1))],
        );
        assert!(selection.live(1) && selection.live(2));
        selection.finish_census(&mut store);
        assert!(selection.live(1) && !selection.live(2));
        assert!(selection.live(70_000) && !selection.live(80_000));
        selection.reset(4, false);
        assert!(selection.live(2));
    }

    #[test]
    fn an_owner_missing_from_a_complete_census_is_new_when_it_returns() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        let census = |selection: &mut SliceSelection,
                      store: &mut SliceFingerprints,
                      hour,
                      owners: &[(u32, Option<u64>)]| {
            selection.reset(hour, false);
            scan(selection, store, owners);
            selection.finish_census(store);
        };
        census(&mut selection, &mut store, 0, &[(1, Some(1)), (2, Some(2))]);
        census(&mut selection, &mut store, 3, &[(1, Some(1)), (2, Some(2))]);
        assert_eq!(selection.early_count, 0);
        census(&mut selection, &mut store, 4, &[(1, Some(1))]);
        census(&mut selection, &mut store, 5, &[(1, Some(1)), (2, Some(2))]);
        assert!(selection.contains(2) && !selection.contains(1));
        selection.reset(6, false);
        scan(&mut selection, &mut store, &[(1, Some(1))]);
        selection.reset(7, false);
        scan(&mut selection, &mut store, &[(1, Some(1)), (2, Some(2))]);
        assert!(!selection.contains(2));
    }

    #[test]
    fn unpublished_reads_are_read_again_at_the_next_hour() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        let owners = [(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(40))];
        selection.reset(0, true);
        scan(&mut selection, &mut store, &owners);
        selection.reset(1, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(41))],
        );
        assert_eq!(selection.early_count, 1);
        selection.invalidate(&mut store);
        selection.reset(2, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(41))],
        );
        assert_eq!(selection.early_count, 3);
        assert!(selection.contains(1) && selection.contains(25) && selection.contains(40));
        selection.reset(3, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(41))],
        );
        assert_eq!(selection.early_count, 0);
        selection.reset(4, true);
        scan(&mut selection, &mut store, &owners);
        selection.invalidate(&mut store);
        selection.reset(5, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(40))],
        );
        assert_eq!(selection.early_count, 4);
    }

    #[test]
    fn families_cover_exactly_the_sliced_tables_once() {
        for table in TableId::ALL {
            let families = FAMILIES
                .iter()
                .filter(|family| family.tables.contains(&table))
                .count();
            assert_eq!(
                families,
                usize::from(replay_data::cadence::is_sliced(table)),
                "{table:?}"
            );
            assert_eq!(
                family_of(table).is_some(),
                replay_data::cadence::is_sliced(table)
            );
        }
    }

    #[test]
    fn marked_owners_join_the_read_and_an_uncertified_take_reads_everyone() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        let census = |selection: &mut SliceSelection, store: &mut SliceFingerprints| {
            for owner in [1, 2, 3, 30] {
                selection.scan(store, owner, Some(0));
            }
            store.finish_scan();
            selection.finish_census(store);
        };
        selection.reset(0, false);
        census(&mut selection, &mut store);
        selection.certify(&mut store, Some(7));
        assert!(selection.force_full);
        selection.reset(2, false);
        census(&mut selection, &mut store);
        selection.mark(3);
        selection.mark(3);
        selection.mark(2);
        selection.certify(&mut store, Some(7));
        assert!(!selection.force_full);
        assert!(selection.contains(2) && selection.contains(3));
        assert!(!selection.contains(1) && !selection.contains(30));
        assert_eq!((selection.marked_count, selection.marked_early()), (2, 1));
        assert_eq!(selection.read_owners().collect::<Vec<_>>(), [2, 3]);
        for epoch in [None, Some(8)] {
            selection.reset(4, false);
            census(&mut selection, &mut store);
            selection.certify(&mut store, epoch);
            assert!(selection.force_full && selection.contains(1), "{epoch:?}");
        }
        selection.reset(5, false);
        census(&mut selection, &mut store);
        selection.certify(&mut store, Some(8));
        assert!(!selection.force_full);
        store.forget();
        selection.reset(7, false);
        census(&mut selection, &mut store);
        selection.certify(&mut store, Some(8));
        assert!(selection.force_full);
    }

    #[test]
    fn select_adds_an_owner_once_outside_its_slice() {
        let mut selection = SliceSelection::new();
        selection.reset(5, false);
        assert!(selection.select(29));
        assert_eq!(selection.early_count, 0);
        assert!(selection.select(30));
        assert!(selection.select(30));
        assert!(selection.select(6));
        assert_eq!(selection.early_count, 2);
        assert!(selection.contains(6) && selection.contains(30) && selection.contains(29));
        assert!(!selection.contains(7));
        assert!(selection.select(u32::MAX));
        assert_eq!(selection.out_of_range, 0);
    }

    fn changed_hour(
        store: &mut SliceFingerprints,
        changed: &mut Changed,
        hour: i32,
        owners: &[u32],
        marks: &[usize],
        epoch: Option<u64>,
        context: u64,
    ) -> SliceSelection {
        let mut selection = SliceSelection::new();
        selection.reset(hour, false);
        for &owner in owners {
            selection.scan(store, owner, Some(0));
        }
        store.finish_scan();
        selection.finish_pass(store, true);
        select_changed(
            &mut selection,
            store,
            changed,
            |visit| {
                for &mark in marks {
                    visit(mark);
                }
                epoch
            },
            context,
        );
        selection
    }

    #[test]
    fn marked_owners_join_the_hour_and_an_uncertified_take_reads_everyone() {
        let owners = [1, 2, 3, 29];
        let mut store = SliceFingerprints::new();
        let mut changed = Changed::default();
        let first = changed_hour(&mut store, &mut changed, 5, &owners, &[], Some(7), 1);
        assert!(first.force_full && !changed.certified);
        let next = changed_hour(&mut store, &mut changed, 5, &owners, &[2], Some(7), 1);
        assert!(changed.certified && !next.force_full);
        assert_eq!(changed.marked(), 1);
        assert!(next.contains(2) && next.contains(29) && !next.contains(1) && !next.contains(3));
        let quiet = changed_hour(&mut store, &mut changed, 6, &owners, &[], Some(7), 1);
        assert!(!quiet.force_full && (1..=3).all(|owner| !quiet.contains(owner)));
        for (epoch, context) in [(Some(8), 1), (None, 1), (Some(7), 2)] {
            let mut store = SliceFingerprints::new();
            changed_hour(&mut store, &mut changed, 5, &owners, &[], Some(7), 1);
            let hour = changed_hour(&mut store, &mut changed, 6, &owners, &[], epoch, context);
            assert!(hour.force_full && !changed.certified, "{epoch:?} {context}");
        }
        store.forget();
        let forgotten = changed_hour(&mut store, &mut changed, 6, &owners, &[], Some(7), 1);
        assert!(forgotten.force_full);
    }

    #[test]
    fn an_unpublished_hour_reads_every_owner_next() {
        let owners = [1, 2, 3];
        let mut store = SliceFingerprints::new();
        let mut changed = Changed::default();
        changed_hour(&mut store, &mut changed, 5, &owners, &[], Some(7), 1);
        let marked = changed_hour(&mut store, &mut changed, 6, &owners, &[2], Some(7), 1);
        assert!(changed.certified && marked.contains(2));
        marked.invalidate(&mut store);
        let next = changed_hour(&mut store, &mut changed, 7, &owners, &[], Some(7), 1);
        assert!(next.force_full && !changed.certified);
        let settled = changed_hour(&mut store, &mut changed, 8, &owners, &[], Some(7), 1);
        assert!(!settled.force_full && changed.certified);
    }

    #[test]
    fn a_daily_family_reads_everyone_at_the_boundary_and_only_selections_between() {
        let mut selection = SliceSelection::new();
        for hour in [0, 24, -24] {
            selection.reset_daily(hour, false);
            assert!(!selection.force_full && selection.contains(5), "{hour}");
        }
        selection.reset_daily(5, true);
        assert!(selection.contains(7));
        let mut store = SliceFingerprints::new();
        selection.reset_daily(0, false);
        selection.scan(&mut store, 5, Some(0));
        selection.scan(&mut store, 29, Some(0));
        store.finish_scan();
        selection.reset_daily(5, false);
        selection.scan(&mut store, 5, Some(0));
        selection.scan(&mut store, 29, Some(0));
        store.finish_scan();
        assert!(!selection.contains(5) && !selection.contains(29));
        assert!(selection.select(29));
        assert!(selection.contains(29) && !selection.contains(5));
        selection.mark(7);
        assert_eq!(selection.marked_early(), 1);
        selection.invalidate(&mut store);
        let mut next = SliceSelection::new();
        next.reset_daily(6, false);
        next.scan(&mut store, 5, Some(0));
        next.scan(&mut store, 29, Some(0));
        assert!(next.contains(29) && !next.contains(5));
        next.reset(5, false);
        assert!(next.contains(5));
    }

    #[test]
    fn settled_owners_are_read_only_for_a_change() {
        let owners: Vec<u32> = (1..=446).collect();
        let drifts = |owner: u32| owner.is_multiple_of(7);
        let drifting = owners.iter().filter(|&&owner| drifts(owner)).count();
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        let hour_reads = |selection: &mut SliceSelection, store: &mut SliceFingerprints| {
            for &owner in &owners {
                let drift = drifts(owner);
                selection.scan_drifting(store, owner, Some(u64::from(drift)), drift);
            }
            store.finish_scan();
            selection.finish_census(store);
            owners
                .iter()
                .filter(|&&owner| selection.contains(owner))
                .count()
        };
        selection.reset(0, false);
        assert_eq!(hour_reads(&mut selection, &mut store), owners.len());
        let mut day = 0;
        for hour in 1..=24 {
            selection.reset(hour, false);
            day += hour_reads(&mut selection, &mut store);
            assert!(
                owners
                    .iter()
                    .all(|&owner| drifts(owner) || !selection.contains(owner))
            );
        }
        assert_eq!(day, drifting);
        let mut daily = SliceSelection::new();
        daily.reset_daily(48, false);
        assert_eq!(hour_reads(&mut daily, &mut store), drifting);
        daily.reset_daily(49, false);
        assert_eq!(hour_reads(&mut daily, &mut store), 0);
        daily.reset_daily(50, false);
        daily.scan_drifting(&mut store, 1, Some(1), true);
        assert!(daily.contains(1));
        daily.reset_daily(51, false);
        daily.scan_drifting(&mut store, 2, Some(0), false);
        assert!(!daily.contains(2));
        daily.mark(2);
        assert!(daily.contains(2));
        daily.reset_daily(52, true);
        daily.scan_drifting(&mut store, 3, Some(0), false);
        assert!(daily.contains(3));
    }

    #[test]
    fn drift_is_read_only_when_it_decides_a_read() {
        let owners: Vec<u32> = (1..=446).collect();
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        let checks = std::cell::Cell::new(0);
        let hour = |selection: &mut SliceSelection, store: &mut SliceFingerprints| {
            for &owner in &owners {
                selection.scan_lazily(store, owner, || {
                    checks.set(checks.get() + 1);
                    owner.is_multiple_of(7)
                });
            }
            store.finish_scan();
            selection.finish_census(store);
        };
        selection.reset(0, false);
        hour(&mut selection, &mut store);
        assert_eq!(checks.replace(0), owners.len());
        for at in 1..=24 {
            selection.reset(at, false);
            hour(&mut selection, &mut store);
        }
        assert_eq!(checks.replace(0), owners.len());
        let mut daily = SliceSelection::new();
        daily.reset_daily(25, false);
        hour(&mut daily, &mut store);
        assert_eq!(checks.replace(0), 0);
        daily.reset_daily(48, false);
        hour(&mut daily, &mut store);
        assert_eq!(checks.replace(0), owners.len());
        store.invalidate(5);
        daily.reset_daily(49, false);
        hour(&mut daily, &mut store);
        assert_eq!(checks.replace(0), 1);
        assert!(daily.contains(5));
    }
}
