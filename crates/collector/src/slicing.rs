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
//! Owners are native country IDs or native state IDs, one kind per family
//! ([`OwnerKind`]). Each family has its own [`SliceSelection`] per frame and
//! its own [`SliceFingerprints`] store for the attachment, so country 5 and
//! state 5 never share a bit. Register a family in [`FAMILIES`]; the spool
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

/// Owners with a fingerprint: the engine's u16 slot range. A larger ID is
/// still read in its slice; it has no early re-read and is counted in status.
pub(crate) const FINGERPRINT_OWNERS: usize = 1 << 16;
const OWNER_WORDS: usize = FINGERPRINT_OWNERS / 64;

/// What a family's owner IDs are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerKind {
    Country,
    #[allow(dead_code)]
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
pub(crate) const RELATIONS: usize = 2;

/// Every sliced family. Index with the family constants above.
pub(crate) const FAMILIES: [SliceFamily; 3] = [
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
    // Relations are read once a day, one owner slice per hour (owner rule),
    // with no change fingerprint: an opinion or relation changed by an event
    // waits for its owner's slice. A constant fingerprint reads new owners
    // and unpublished reads at the next hour.
    SliceFamily {
        name: "relations",
        owner: OwnerKind::Country,
        tables: &[TableId::RelationOpinions, TableId::RelationTuples],
    },
];

/// The family that reads `table`, if it is sliced.
pub(crate) fn family_of(table: TableId) -> Option<usize> {
    FAMILIES
        .iter()
        .position(|family| family.tables.contains(&table))
}

fn bit(words: &[u64], owner: u32) -> bool {
    let owner = owner as usize;
    words
        .get(owner / 64)
        .is_some_and(|word| word & (1 << (owner % 64)) != 0)
}

fn set_bit(words: &mut [u64], owner: u32, value: bool) {
    let owner = owner as usize;
    if let Some(word) = words.get_mut(owner / 64) {
        if value {
            *word |= 1 << (owner % 64);
        } else {
            *word &= !(1 << (owner % 64));
        }
    }
}

/// The owners one hour's read of a family covered. Preallocated per frame;
/// no allocation on the hour.
pub(crate) struct SliceSelection {
    pub(crate) hour: i32,
    pub(crate) force_full: bool,
    /// Owners read before their slice because their fingerprint changed.
    early: Box<[u64]>,
    pub(crate) early_count: u32,
    /// Owners the scan saw this hour; meaningful once `census` is set.
    seen: Box<[u64]>,
    /// The scan visited every live owner without a census error.
    census: bool,
    /// Owners whose fingerprint was compared this hour.
    pub(crate) scanned: u32,
    /// Owners above the fingerprint range: read in their slice only.
    pub(crate) out_of_range: u32,
    /// Worker ticks spent computing fingerprints (not game-thread time).
    pub(crate) scan_ticks: u64,
}

impl SliceSelection {
    /// `storage` false gives an allocation-free placeholder that must not scan.
    pub(crate) fn new(storage: bool) -> Self {
        Self {
            hour: 0,
            force_full: true,
            early: vec![0; if storage { OWNER_WORDS } else { 0 }].into_boxed_slice(),
            early_count: 0,
            seen: vec![0; if storage { OWNER_WORDS } else { 0 }].into_boxed_slice(),
            census: false,
            scanned: 0,
            out_of_range: 0,
            scan_ticks: 0,
        }
    }

    /// Start an hour's read.
    pub(crate) fn reset(&mut self, hour: i32, force_full: bool) {
        self.hour = hour;
        self.force_full = force_full;
        self.early.fill(0);
        self.early_count = 0;
        self.seen.fill(0);
        self.census = false;
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
        debug_assert!(!self.early.is_empty(), "placeholder selections cannot scan");
        self.scanned += 1;
        if owner as usize >= FINGERPRINT_OWNERS {
            self.out_of_range += 1;
            return;
        }
        set_bit(&mut self.seen, owner, true);
        if fingerprints.observe(owner, fingerprint) && !self.contains(owner) {
            set_bit(&mut self.early, owner, true);
            self.early_count += 1;
        }
    }

    /// The scan visited every live owner: owners it did not see are gone, and
    /// the store forgets them, so a returning or reused ID is new at once.
    pub(crate) fn finish_census(&mut self, fingerprints: &mut SliceFingerprints) {
        self.census = true;
        fingerprints.keep_only(&self.seen);
    }

    /// Whether `owner` may still hold rows: seen by a complete census, above
    /// the tracked range, or not known to be gone (no complete census).
    pub(crate) fn live(&self, owner: u32) -> bool {
        !self.census || owner as usize >= FINGERPRINT_OWNERS || bit(&self.seen, owner)
    }

    /// Whether this hour's read covers `owner`.
    pub(crate) fn contains(&self, owner: u32) -> bool {
        replay::cadence::slice_active(owner, self.hour, self.force_full) || bit(&self.early, owner)
    }

    /// This hour's read was not published (failed, truncated or dropped):
    /// every owner it selected is a change at its next scan, so the next hour
    /// reads them again.
    pub(crate) fn invalidate(&self, fingerprints: &mut SliceFingerprints) {
        for (index, &word) in self.early.iter().enumerate() {
            let mut word = word;
            while word != 0 {
                fingerprints.invalidate((index * 64) as u32 + word.trailing_zeros());
                word &= word - 1;
            }
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
            set_bit(&mut selection.early, owner, true);
            selection.early_count += 1;
        }
        selection
    }

    /// This selection after a complete census that saw only `seen`.
    #[cfg(test)]
    pub(crate) fn with_census(mut self, seen: &[u32]) -> Self {
        for &owner in seen {
            set_bit(&mut self.seen, owner, true);
        }
        self.census = true;
        self
    }
}

/// The latest change fingerprint per owner for one sliced family.
pub(crate) struct SliceFingerprints {
    values: Box<[u64]>,
    /// Owners whose last scan produced a fingerprint.
    known: Box<[u64]>,
    /// Owners whose last scan could not read one, or whose last read was not
    /// published: their next scan counts as a change.
    stale: Box<[u64]>,
    /// Slots (bit per slice) whose last read was not published; every owner
    /// of such a slice counts as a change at the next scan.
    stale_slices: u32,
}

impl SliceFingerprints {
    /// Allocated once, before the hook is installed.
    pub(crate) fn new() -> Self {
        Self {
            values: vec![0; FINGERPRINT_OWNERS].into_boxed_slice(),
            known: vec![0; OWNER_WORDS].into_boxed_slice(),
            stale: vec![0; OWNER_WORDS].into_boxed_slice(),
            stale_slices: 0,
        }
    }

    /// Forget every observation, e.g. after an interrupted job. Every owner
    /// is then new at its next scan and is read at once.
    pub(crate) fn forget(&mut self) {
        self.known.fill(0);
        self.stale.fill(0);
        self.stale_slices = 0;
    }

    /// Forget every owner a complete census did not see (`seen` bit set).
    fn keep_only(&mut self, seen: &[u64]) {
        for (words, seen) in [&mut self.known, &mut self.stale]
            .into_iter()
            .flat_map(|words| words.iter_mut().zip(seen))
        {
            *words &= *seen;
        }
    }

    /// Make `owner` a change at its next scan.
    pub(crate) fn invalidate(&mut self, owner: u32) {
        set_bit(&mut self.stale, owner, true);
    }

    /// Record `value` (None: unreadable) for `owner`. True if the owner is new
    /// or was invalidated, if its value differs, or if its read starts or keeps
    /// failing after a success: the full read then publishes the exact rows
    /// or failure. An owner that stays unreadable is not re-read every hour.
    fn observe(&mut self, owner: u32, value: Option<u64>) -> bool {
        let index = owner as usize;
        if index >= FINGERPRINT_OWNERS {
            return false;
        }
        let known = bit(&self.known, owner);
        let failed = !known && bit(&self.stale, owner);
        let stale = bit(&self.stale, owner)
            || self.stale_slices & (1 << replay::cadence::slice_of(owner)) != 0;
        let previous = self.values[index];
        set_bit(&mut self.known, owner, value.is_some());
        set_bit(&mut self.stale, owner, value.is_none());
        if let Some(value) = value {
            self.values[index] = value;
        }
        match value {
            // Stays unreadable: its slice read publishes the failure.
            None if failed => false,
            None => true,
            Some(value) => stale || !known || value != previous,
        }
    }

    /// End an hour's scan: whole-slice invalidations have reached every owner.
    pub(crate) fn finish_scan(&mut self) {
        self.stale_slices = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Failing is a change; staying failed is not; recovering is.
        for (hour, value, read) in [(6, None, true), (7, None, false), (8, Some(3), true)] {
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
        // Owners above the fingerprint range are counted and read in their slice only.
        selection.reset(11, false);
        scan(
            &mut selection,
            &mut store,
            &[(70_000, Some(1)), (70_000, Some(2))],
        );
        assert_eq!((selection.early_count, selection.out_of_range), (0, 2));
        // A full hour reads everyone.
        let mut full = SliceSelection::new(false);
        full.reset(10, true);
        assert!(full.contains(12_345));
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
        // Owners above the tracked range cannot be proven gone.
        assert!(selection.live(70_000) && selection.live(80_000));
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
    }
}
