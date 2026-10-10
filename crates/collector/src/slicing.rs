//! Owner selection for [`replay::cadence::Cadence::Sliced`] tables.
//!
//! A sliced family reads, at each hour, the owners of that hour's slice
//! ([`replay::cadence::slice_active`]), every owner on a full hour, and every
//! owner whose change fingerprint differs from its last observation, including
//! an owner seen for the first time. A fingerprint only adds reads: an equal
//! value never skips a slice read, so every emitted row is a complete read
//! whose hour the spool records in SliceReads. The family computes its fingerprints; this
//! module keeps the bookkeeping.
//!
//! Owners are native country IDs, canonical country slots or native state IDs,
//! one kind per family ([`OwnerKind`]). Each family has its own
//! [`SliceSelection`] per frame and its own [`SliceFingerprints`] store for the
//! attachment, so country 5, slot 5 and state 5 never share storage. Register a family in [`FAMILIES`]; the spool
//! then merges its tables by owner ([`replay::sliced::merge_slice`]).
//!
//! A read that was selected but not published (failed, truncated or dropped)
//! must be invalidated ([`SliceSelection::invalidate`]) so its owners are read
//! again at the next hour rather than at their next slice.
//!
//! The scan also records which owners are live. After a complete census
//! ([`SliceSelection::finish_census`]) the merge drops the held rows of owners
//! the census did not see ([`SliceSelection::live`]), so a vanished or
//! re-aliased owner's rows leave at the next hour, not at its next slice.

use replay::rows::TableId;
use std::collections::HashMap;

/// What a family's owner IDs are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerKind {
    /// Native country IDs.
    Country,
    /// Canonical country slots, which follow a tag through aliasing.
    CanonicalSlot,
    /// Native state IDs.
    State,
}

/// A sliced family: the tables read together for the same owners.
pub(crate) struct SliceFamily {
    pub(crate) name: &'static str,
    pub(crate) owner: OwnerKind,
    pub(crate) tables: &'static [TableId],
}

pub(crate) const RESEARCH: usize = 0;
pub(crate) const TECHNOLOGIES: usize = 1;
pub(crate) const RAID_LAUNCH: usize = 2;
pub(crate) const RAID_STATE: usize = 3;
pub(crate) const RAID_BUILDINGS: usize = 4;
pub(crate) const RELATIONS: usize = 5;
pub(crate) const STABILITY: usize = 6;
pub(crate) const RETREAT_BANS: usize = 7;
pub(crate) const AIR_RELATIONS: usize = 8;
pub(crate) const AIR_HISTORIES: usize = 9;
pub(crate) const NAVAL_MISSIONS: usize = 10;
pub(crate) const COMBAT_LOG: usize = 11;

/// Every sliced family. Index with the family constants above.
pub(crate) const FAMILIES: [SliceFamily; 12] = [
    SliceFamily {
        name: "research",
        owner: OwnerKind::Country,
        tables: &[
            TableId::ResearchInputs,
            TableId::ResearchAppliedBonuses,
            TableId::ResearchModifiers,
        ],
    },
    // Technology rosters change daily for every country, so the family has
    // no change fingerprint: a constant one reads new owners and unpublished
    // reads at the next hour. TechnologyDefinitions is its keyed table.
    SliceFamily {
        name: "technologies",
        owner: OwnerKind::Country,
        tables: &[
            TableId::ResearchTechnologies,
            TableId::TechnologyDefinitions,
        ],
    },
    // The raid input families are read daily with a constant fingerprint,
    // as relations are: no hook or change fingerprint makes them exact within
    // the hour (owner rule: no harsher than the game's rolling re-check).
    SliceFamily {
        name: "raid_launch",
        owner: OwnerKind::CanonicalSlot,
        tables: &[
            TableId::RaidCountryInputs,
            TableId::StockpileArchetypes,
            TableId::RaidAirSites,
            TableId::AceRangeInputs,
        ],
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
    // Relations are read once a day, one owner slice per hour (owner rule),
    // with no change fingerprint: an opinion or relation changed by an event
    // waits for its owner's slice. A constant fingerprint reads new owners
    // and unpublished reads at the next hour.
    SliceFamily {
        name: "relations",
        owner: OwnerKind::Country,
        tables: &[TableId::RelationOpinions, TableId::RelationTuples],
    },
    // Stability inputs and tree lines, read in the Slow job; the fingerprint
    // is `engine::country_stability::fingerprint_country`.
    SliceFamily {
        name: "stability",
        owner: OwnerKind::Country,
        tables: &[
            TableId::CountryStabilityInputs,
            TableId::CountryStabilityLines,
        ],
    },
    // Commanders' retreat bans, read with a constant print by the roster
    // owner's canonical slot in the Presentation job.
    SliceFamily {
        name: "retreat_bans",
        owner: OwnerKind::CanonicalSlot,
        tables: &[TableId::DivisionRetreatBans],
    },
    // AirCountryRelations, held by its decode step and certified by the
    // native-writer hooks (`relation_changes`); the family's store is not
    // lent to an acquisition job.
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
    // The countries the hour's naval mission task forces name; the fingerprint is their
    // leader and headquarters-commander references (extended/naval_mission_inputs.rs).
    SliceFamily {
        name: "naval_missions",
        owner: OwnerKind::Country,
        tables: &[
            TableId::NavalMissionCountries,
            TableId::NavalMissionCommanders,
        ],
    },
    // The Combat Log, read in the Slow job without a fingerprint store: at
    // the last hour before the daily prune the family selects each country
    // holding a deleted group's log ([`SliceSelection::select`]); every other
    // read waits for its slice; a country whose last complete read is older
    // than the previous calendar month is read whole.
    SliceFamily {
        name: "combat_log",
        owner: OwnerKind::Country,
        tables: &[TableId::CombatLogEquipment],
    },
];

/// The family that reads `table`, if it is sliced.
pub(crate) fn family_of(table: TableId) -> Option<usize> {
    FAMILIES
        .iter()
        .position(|family| family.tables.contains(&table))
}

/// Worker/spool insertion sizes from an original observed owner occurrence,
/// retaining ascending output without allocating an owner's entire ID domain.
fn insert_owner(owners: &mut Vec<u32>, owner: u32) -> bool {
    let Err(index) = owners.binary_search(&owner) else {
        return true;
    };
    if engine::allocation::reserve_push_batch(owners, 1).is_err() {
        return false;
    }
    owners.insert(index, owner);
    true
}

/// The owners one hour's worker read of a family covered. Constructors are
/// empty; original observed owners grow storage before their read is published.
pub(crate) struct SliceSelection {
    pub(crate) hour: i32,
    pub(crate) force_full: bool,
    /// Owners read before their slice because their fingerprint changed.
    early: Vec<u32>,
    pub(crate) early_count: u32,
    /// Owners the scan saw this hour; meaningful once `census` is set.
    seen: Vec<u32>,
    /// The scan visited every live owner without a census error.
    census: bool,
    /// Owners the family's walker read this hour, when it reports them
    /// (`track_reads`): a walker that visits only some census owners stamps
    /// only those it read.
    walked: Vec<u32>,
    tracked: bool,
    /// Owners whose fingerprint was compared this hour.
    pub(crate) scanned: u32,
    /// Owners whose bookkeeping allocation could not complete.
    pub(crate) out_of_range: u32,
    /// Worker ticks spent computing fingerprints (not game-thread time).
    pub(crate) scan_ticks: u64,
}

impl SliceSelection {
    /// Both owned packs and callback placeholders start without owner storage.
    pub(crate) fn new(_storage: bool) -> Self {
        Self {
            hour: 0,
            force_full: true,
            early: Vec::new(),
            early_count: 0,
            seen: Vec::new(),
            census: false,
            walked: Vec::new(),
            tracked: false,
            scanned: 0,
            out_of_range: 0,
            scan_ticks: 0,
        }
    }

    /// Start an hour's read.
    pub(crate) fn reset(&mut self, hour: i32, force_full: bool) {
        self.hour = hour;
        self.force_full = force_full;
        self.early.clear();
        self.early_count = 0;
        self.seen.clear();
        self.walked.clear();
        self.census = false;
        self.tracked = false;
        self.scanned = 0;
        self.out_of_range = 0;
        self.scan_ticks = 0;
    }

    /// Compare one owner's fingerprint (None: unreadable) with the store and
    /// add it to this hour's read if it changed or is new outside its slice.
    pub(crate) fn scan(
        &mut self,
        fingerprints: &mut SliceFingerprints,
        owner: u32,
        fingerprint: Option<u64>,
    ) {
        self.scanned += 1;
        if !insert_owner(&mut self.seen, owner) {
            self.out_of_range += 1;
            self.force_full = true;
            return;
        }
        if fingerprints.observe(owner, fingerprint) && !self.contains(owner) {
            if insert_owner(&mut self.early, owner) {
                self.early_count += 1;
            } else {
                self.out_of_range += 1;
                self.force_full = true;
            }
        }
    }

    /// Add `owner` to this hour's read outside its slice, without a
    /// fingerprint. Storage grows on the same worker before publication.
    pub(crate) fn select(&mut self, owner: u32) -> bool {
        if !self.contains(owner) {
            if !insert_owner(&mut self.early, owner) {
                self.out_of_range += 1;
                self.force_full = true;
                return false;
            }
            self.early_count += 1;
        }
        true
    }

    /// The scan visited every live owner: owners it did not see are gone, and
    /// the store forgets them, so a returning or reused ID is new at once.
    pub(crate) fn finish_census(&mut self, fingerprints: &mut SliceFingerprints) {
        if self.out_of_range != 0 || engine::allocation::capture_stopped() {
            self.force_full = true;
            return;
        }
        self.census = true;
        fingerprints.keep_only(&self.seen);
    }

    /// End an hour's fingerprint pass. A complete pass is a census; a failed
    /// or truncated one compared only some owners, so an unchanged print
    /// certifies nothing this hour and every owner is read (the owners never
    /// compared would otherwise publish a slice read as current).
    pub(crate) fn finish_pass(&mut self, fingerprints: &mut SliceFingerprints, complete: bool) {
        if complete {
            self.finish_census(fingerprints);
        } else {
            self.force_full = true;
        }
    }

    /// Whether `owner` may still hold rows: seen by a complete census or not
    /// known to be gone (no complete census).
    pub(crate) fn live(&self, owner: u32) -> bool {
        !self.census || self.seen.binary_search(&owner).is_ok()
    }

    /// Whether this hour's read covers `owner`.
    pub(crate) fn contains(&self, owner: u32) -> bool {
        replay::cadence::slice_active(owner, self.hour, self.force_full)
            || self.early.binary_search(&owner).is_ok()
    }

    /// This hour's walker reports the owners it read (`mark_read`); only
    /// those are read owners.
    pub(crate) fn track_reads(&mut self) {
        self.tracked = true;
        self.walked.clear();
    }

    /// The walker read `owner` this hour, rows or none.
    pub(crate) fn mark_read(&mut self, owner: u32) {
        if !insert_owner(&mut self.walked, owner) {
            self.out_of_range += 1;
            self.force_full = true;
        }
    }

    /// Owners the scan saw that this hour's read covers, ascending: the owners
    /// read now, including those that returned no rows (with a tracking
    /// walker, only those it read). Empty without a scan.
    pub(crate) fn read_owners(&self) -> impl Iterator<Item = u32> + '_ {
        self.seen
            .iter()
            .copied()
            .filter(|&owner| self.contains(owner))
            .filter(|&owner| !self.tracked || self.walked.binary_search(&owner).is_ok())
    }

    /// This hour's read was not published (failed, truncated or dropped):
    /// every owner it selected is a change at its next scan, so the next hour
    /// reads them again.
    pub(crate) fn invalidate(&self, fingerprints: &mut SliceFingerprints) {
        for &owner in &self.early {
            fingerprints.invalidate(owner);
        }
        fingerprints.stale_slices |= if self.force_full {
            u32::MAX
        } else {
            1 << self.hour.rem_euclid(replay::cadence::STAGGER_PERIOD)
        };
    }

    /// A selection as a family would record it.
    #[cfg(test)]
    pub(crate) fn for_test(hour: i32, force_full: bool, early: &[u32]) -> Self {
        let mut selection = Self::new(true);
        selection.reset(hour, force_full);
        for &owner in early {
            assert!(insert_owner(&mut selection.early, owner));
            selection.early_count += 1;
        }
        selection
    }

    /// This selection after a complete census that saw only `seen`.
    #[cfg(test)]
    pub(crate) fn with_census(mut self, seen: &[u32]) -> Self {
        for &owner in seen {
            assert!(insert_owner(&mut self.seen, owner));
        }
        self.census = true;
        self
    }
}

/// The latest change fingerprint per owner for one sliced family.
pub(crate) struct SliceFingerprints {
    values: HashMap<u32, Fingerprint>,
    /// Slots (bit per slice) whose last read was not published; every owner
    /// of such a slice counts as a change at the next scan.
    stale_slices: u32,
}
struct Fingerprint {
    value: Option<u64>,
    stale: bool,
}

impl SliceFingerprints {
    /// Storage follows only the original owners observed by each worker scan.
    pub(crate) fn new() -> Self {
        Self {
            values: HashMap::new(),
            stale_slices: 0,
        }
    }

    /// Forget every observation, e.g. after an interrupted job. Every owner
    /// is then new at its next scan and is read at once.
    pub(crate) fn forget(&mut self) {
        self.values.clear();
        self.stale_slices = 0;
    }

    /// Forget every owner a complete census did not see.
    fn keep_only(&mut self, seen: &[u32]) {
        self.values
            .retain(|owner, _| seen.binary_search(owner).is_ok());
    }

    /// Make `owner` a change at its next scan.
    pub(crate) fn invalidate(&mut self, owner: u32) {
        if let Some(value) = self.values.get_mut(&owner) {
            value.stale = true;
        }
    }

    /// Record `value` (None: unreadable) for `owner`. True if the owner is new
    /// or was invalidated, if its value differs, or if it is unreadable: an
    /// unreadable fingerprint certifies nothing, so the owner is read at
    /// every hour until its fingerprint reads again, and that read publishes
    /// its exact rows or its failure.
    fn observe(&mut self, owner: u32, value: Option<u64>) -> bool {
        let previous = self.values.get(&owner);
        let changed = previous.is_none_or(|previous| previous.stale || previous.value != value)
            || value.is_none()
            || self.stale_slices & (1 << replay::cadence::slice_of(owner)) != 0;
        if previous.is_none() && engine::allocation::reserve_map(&mut self.values, 1).is_err() {
            return true;
        }
        self.values.insert(
            owner,
            Fingerprint {
                value,
                stale: value.is_none(),
            },
        );
        changed
    }

    /// End an hour's scan: whole-slice invalidations have reached every owner.
    pub(crate) fn finish_scan(&mut self) {
        self.stale_slices = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_and_fingerprints_start_empty_without_callback_allocation() {
        crate::allocation_failure_tests::with_allocations_denied(|| {
            for storage in [false, true] {
                let selection = SliceSelection::new(storage);
                for owners in [&selection.early, &selection.seen, &selection.walked] {
                    assert_eq!(owners.capacity(), 0);
                }
            }
            assert_eq!(SliceFingerprints::new().values.capacity(), 0);
        });
    }

    #[test]
    fn original_five_thousand_and_mod_sized_sparse_owner_occurrences_grow_before_publication() {
        let mut selection = SliceSelection::new(false);
        let mut fingerprints = SliceFingerprints::new();
        for count in [5000u32, 12_000] {
            fingerprints.forget();
            selection.reset(0, false);
            selection.track_reads();
            for index in 0..count {
                let owner = u32::MAX - index * 7919;
                selection.scan(&mut fingerprints, owner, Some(u64::from(index)));
                selection.mark_read(owner);
            }
            selection.finish_pass(&mut fingerprints, true);
            assert_eq!(selection.out_of_range, 0);
            assert_eq!(selection.read_owners().count(), count as usize);
            assert_eq!(fingerprints.values.len(), count as usize);
            assert_eq!(selection.seen.len(), count as usize);
            let capacities = [
                selection.early.capacity(),
                selection.seen.capacity(),
                selection.walked.capacity(),
                fingerprints.values.capacity(),
            ];
            crate::allocation_failure_tests::with_allocations_denied(|| {
                selection.invalidate(&mut fingerprints);
                selection.reset(1, false);
            });
            for index in 0..count {
                let owner = u32::MAX - index * 7919;
                selection.scan(&mut fingerprints, owner, Some(u64::from(index)));
                assert!(selection.contains(owner));
            }
            fingerprints.finish_scan();
            assert_eq!(selection.read_owners().count(), count as usize);
            assert_eq!(
                [
                    selection.early.capacity(),
                    selection.seen.capacity(),
                    selection.walked.capacity(),
                    fingerprints.values.capacity()
                ],
                capacities
            );
            let mut prior = 0;
            for owner in selection.read_owners() {
                assert!(owner > prior);
                prior = owner;
            }
        }
    }

    #[test]
    fn failed_owner_bookkeeping_never_certifies_a_partial_census_or_forgets_healthy_owners() {
        let mut original = SliceSelection::new(true);
        let mut fingerprints = SliceFingerprints::new();
        original.scan(&mut fingerprints, 7, Some(1));
        let mut failed = SliceSelection::new(false);
        failed.reset(1, false);
        crate::allocation_failure_tests::with_allocations_denied(|| {
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
        let mut selection = SliceSelection::new(true);
        // First sight reads everyone outside their slice (1 and 3); 29 is in slice 5.
        selection.reset(5, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (3, Some(3)), (29, Some(29))],
        );
        assert_eq!((selection.scanned, selection.early_count), (3, 2));
        assert!(selection.contains(1) && selection.contains(3) && selection.contains(29));
        // 1 changed (early), 29 changed but is in slice 5 again, 3 unchanged.
        selection.reset(29, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(100)), (3, Some(3)), (29, Some(290))],
        );
        assert_eq!(selection.early_count, 1);
        assert!(selection.contains(1) && selection.contains(29) && !selection.contains(3));
        // Failing, staying failed and recovering are each read at once.
        for (hour, value, read) in [(6, None, true), (7, None, true), (8, Some(3), true)] {
            selection.reset(hour, false);
            scan(&mut selection, &mut store, &[(3, value)]);
            assert_eq!(selection.contains(3), read, "hour {hour}");
        }
        // A new ID (a released country or a reused slot's alias) is read at once.
        selection.reset(9, false);
        scan(&mut selection, &mut store, &[(4_000, Some(1))]);
        assert!(selection.contains(4_000));
        // Forgetting makes every owner new again: read at once.
        store.forget();
        selection.reset(10, false);
        scan(&mut selection, &mut store, &[(1, Some(100)), (3, Some(3))]);
        assert_eq!(selection.early_count, 2);
        // Sparse high IDs have the same fingerprint and slice semantics.
        selection.reset(11, false);
        scan(
            &mut selection,
            &mut store,
            &[(70_000, Some(1)), (70_000, Some(2))],
        );
        assert_eq!((selection.early_count, selection.out_of_range), (1, 0));
        assert!(selection.contains(70_000) && !selection.contains(70_001));
        // A full hour reads everyone.
        let mut full = SliceSelection::new(false);
        full.reset(10, true);
        assert!(full.contains(12_345));
    }

    #[test]
    fn an_incomplete_pass_reads_every_owner_and_forgets_nobody() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(true);
        selection.reset(0, true);
        scan(&mut selection, &mut store, &[(1, Some(1)), (2, Some(2))]);
        selection.finish_pass(&mut store, true);
        // Hour 3: the pass stopped after owner 1; owner 2 was never compared.
        selection.reset(3, false);
        scan(&mut selection, &mut store, &[(1, Some(1))]);
        selection.finish_pass(&mut store, false);
        assert!(selection.force_full && selection.contains(2) && selection.contains(1));
        assert!(selection.live(2));
        // A complete pass the next hour reads its slice only.
        selection.reset(4, false);
        scan(&mut selection, &mut store, &[(1, Some(1)), (2, Some(2))]);
        selection.finish_pass(&mut store, true);
        assert!(!selection.force_full && !selection.contains(2) && !selection.contains(1));
    }

    #[test]
    fn only_a_complete_census_marks_unseen_owners_gone() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(true);
        selection.reset(3, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (70_000, Some(1))],
        );
        // Without a complete census nothing is known to be gone.
        assert!(selection.live(1) && selection.live(2));
        selection.finish_census(&mut store);
        assert!(selection.live(1) && !selection.live(2));
        assert!(selection.live(70_000) && !selection.live(80_000));
        // The next hour starts without a census again.
        selection.reset(4, false);
        assert!(selection.live(2));
    }

    #[test]
    fn an_owner_missing_from_a_complete_census_is_new_when_it_returns() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(true);
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
        // Owner 2 is gone for an hour, then its ID returns with the same value.
        census(&mut selection, &mut store, 4, &[(1, Some(1))]);
        census(&mut selection, &mut store, 5, &[(1, Some(1)), (2, Some(2))]);
        assert!(selection.contains(2) && !selection.contains(1));
        // An incomplete census forgets nobody.
        selection.reset(6, false);
        scan(&mut selection, &mut store, &[(1, Some(1))]);
        selection.reset(7, false);
        scan(&mut selection, &mut store, &[(1, Some(1)), (2, Some(2))]);
        assert!(!selection.contains(2));
    }

    #[test]
    fn unpublished_reads_are_read_again_at_the_next_hour() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(true);
        let owners = [(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(40))];
        selection.reset(0, true);
        scan(&mut selection, &mut store, &owners);
        // Hour 1 reads slice 1 (1 and 25) plus 40 early; the read is not published.
        selection.reset(1, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(41))],
        );
        assert_eq!(selection.early_count, 1);
        selection.invalidate(&mut store);
        // Hour 2 reads all three again early; owner 2 is in slice 2 anyway.
        selection.reset(2, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(41))],
        );
        assert_eq!(selection.early_count, 3);
        assert!(selection.contains(1) && selection.contains(25) && selection.contains(40));
        // Published: hour 3 reads nobody early.
        selection.reset(3, false);
        scan(
            &mut selection,
            &mut store,
            &[(1, Some(1)), (2, Some(2)), (25, Some(25)), (40, Some(41))],
        );
        assert_eq!(selection.early_count, 0);
        // An unpublished full hour re-reads everyone at the next hour.
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
                usize::from(replay::cadence::is_sliced(table)),
                "{table:?}"
            );
            assert_eq!(
                family_of(table).is_some(),
                replay::cadence::is_sliced(table)
            );
        }
        assert_eq!(FAMILIES[RESEARCH].name, "research");
        assert_eq!(FAMILIES[RESEARCH].owner, OwnerKind::Country);
        assert_eq!(FAMILIES[TECHNOLOGIES].name, "technologies");
        assert_eq!(FAMILIES[TECHNOLOGIES].owner, OwnerKind::Country);
        assert_eq!(FAMILIES[RAID_LAUNCH].name, "raid_launch");
        assert_eq!(FAMILIES[RAID_LAUNCH].owner, OwnerKind::CanonicalSlot);
        assert_eq!(FAMILIES[RAID_STATE].owner, OwnerKind::State);
        assert_eq!(FAMILIES[RAID_BUILDINGS].owner, OwnerKind::Country);
        assert_eq!(FAMILIES[STABILITY].name, "stability");
        assert_eq!(FAMILIES[AIR_RELATIONS].owner, OwnerKind::CanonicalSlot);
        assert_eq!(FAMILIES[AIR_HISTORIES].owner, OwnerKind::CanonicalSlot);
        assert_eq!(FAMILIES[NAVAL_MISSIONS].name, "naval_missions");
        assert_eq!(FAMILIES[NAVAL_MISSIONS].owner, OwnerKind::Country);
        assert_eq!(FAMILIES[COMBAT_LOG].name, "combat_log");
        assert_eq!(FAMILIES[COMBAT_LOG].owner, OwnerKind::Country);
    }

    #[test]
    fn select_adds_an_owner_once_outside_its_slice() {
        let mut selection = SliceSelection::new(true);
        selection.reset(5, false);
        // Owner 29 is in hour 5's slice already: selecting it adds nothing.
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
}
