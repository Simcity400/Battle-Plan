//! Owner selection for [`replay_data::cadence::Cadence::Sliced`] tables.
//!
//! A sliced family reads, at each hour, the owners of that hour's slice
//! ([`replay_data::cadence::slice_active`]), every owner on a full hour, and every
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
//! then merges its tables by owner ([`replay_data::sliced::merge_slice`]).
//!
//! A read that was selected but not published (failed, truncated or dropped)
//! must be invalidated ([`SliceSelection::invalidate`]) so its owners are read
//! again at the next hour rather than at their next slice.
//!
//! The scan also records which owners are live. After a complete census
//! ([`SliceSelection::finish_census`]) the merge drops the held rows of owners
//! the census did not see ([`SliceSelection::live`]), so a vanished or
//! re-aliased owner's rows leave at the next hour, not at its next slice.
//!
//! A family whose values change for every owner in the game's daily tick and
//! otherwise only through hooked writers (the focus family) reads every owner
//! at the first hour after that tick instead of a rotating slice, and the
//! owners its writers marked at the other hours
//! ([`SliceSelection::reset_daily`]).
//!
//! A family whose every recorded field has hooked native writers (the raid
//! observer's) reads no rotating slice ([`SliceSelection::reset_hooked`]):
//! each hour reads every owner a writer marked since the previous hour
//! ([`SliceSelection::mark`]), new and unpublished owners, and nothing else.
//! Its take's epoch must be the one the store's previous hour was read under
//! ([`SliceSelection::certify`]); otherwise an owner outside this hour's read
//! may have changed unseen, and every owner is read.

use replay_data::rows::TableId;
use std::collections::HashMap;

/// What a family's owner IDs are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    /// Native country IDs.
    Country,
    /// Canonical country slots, which follow a tag through aliasing.
    CanonicalSlot,
    /// Native state IDs.
    State,
}

/// A sliced family: the tables read together for the same owners.
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

/// Every sliced family. Index with the family constants above.
pub const FAMILIES: [SliceFamily; 16] = [
    // Research inputs, applied bonuses and modifiers, read in their own job
    // as stability is: hooked writers mark the countries to read within the
    // hour (`country_input_changes`); the slice refreshes daily progress.
    SliceFamily {
        name: "research",
        owner: OwnerKind::Country,
        tables: &[
            TableId::ResearchInputs,
            TableId::ResearchAppliedBonuses,
            TableId::ResearchModifiers,
        ],
    },
    // Technology rosters, read in the Governance job: hooked writers mark the
    // countries to read within the hour (`country_input_changes`), a constant
    // print reads new owners and unpublished reads at the next hour, and the
    // slice refreshes daily progress. TechnologyDefinitions is its keyed
    // table.
    SliceFamily {
        name: "technologies",
        owner: OwnerKind::Country,
        tables: &[
            TableId::ResearchTechnologies,
            TableId::TechnologyDefinitions,
        ],
    },
    // RaidAirSites, read with no slice: new countries, unpublished reads and
    // every country a hooked site list writer marked (crate::raid_changes).
    SliceFamily {
        name: "raid_launch",
        owner: OwnerKind::CanonicalSlot,
        tables: &[TableId::RaidAirSites],
    },
    // The raid state inputs and the building-resource overrides read every
    // owner a hooked writer marked (crate::raid_changes) and new or
    // unpublished owners; only origin states keep a daily slice, for the
    // unhooked supply-route flag (owner decision 2026-10-11).
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
    // Stability inputs and tree lines, read in the Stability job with a constant
    // print: the hooked writers mark the countries to read within the hour
    // (`country_input_changes`), and the slice refreshes the daily drift.
    SliceFamily {
        name: "stability",
        owner: OwnerKind::Country,
        tables: &[
            TableId::CountryStabilityInputs,
            TableId::CountryStabilityLines,
        ],
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
    // The countries the hour's naval mission task forces name, read with no slice: the
    // fingerprint certifies every recorded field, so a changed print, a new owner and a full
    // hour read (extended/naval_mission_inputs.rs).
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
    // The decorative supply trains (owner decision 2026-10-11): each
    // country's supply companion, its nodes with their parent-chain vertices
    // and its train inventory, read in the SupplyTrains job with a constant
    // print, so a new country or an unpublished read is read at the next hour.
    SliceFamily {
        name: "supply_trains",
        owner: OwnerKind::Country,
        tables: &[TableId::SupplyNodes, TableId::TrainInventory],
    },
    // Each country's available-advisor pool, read in the Slow job with a
    // constant print (owner decision 2026-10-11: panel-only, daily).
    SliceFamily {
        name: "available_advisors",
        owner: OwnerKind::Country,
        tables: &[TableId::AvailableAdvisors],
    },
    // Each country's focus state and completed roster, read in the
    // Governance job: every country at the first hour after the daily tick,
    // which advances every country's focus progress, and in between the
    // countries a hooked focus writer marked (`country_input_changes`).
    SliceFamily {
        name: "focus",
        owner: OwnerKind::Country,
        tables: &[TableId::Focus, TableId::CompletedFocuses],
    },
    // Each country's special projects and their histories, read in the Gated
    // job like focus: every country after the daily tick, which advances
    // every project, and in between the countries a hooked project writer
    // marked. A row's owner is the project's stored owner tag, which can be
    // any raw id linking to its country, so the family selects and keeps
    // every raw id of a read country (the project capture).
    SliceFamily {
        name: "projects",
        owner: OwnerKind::Country,
        tables: &[TableId::Projects, TableId::ProjectHistory],
    },
    // Trade routes, by the country holding each route, read in the RaidState job with the
    // resources map tables: hooked route writers mark the countries to read within the hour
    // (crate::raid_changes) and a changed set of referenced routes reads its owner at once;
    // there is no slice.
    SliceFamily {
        name: "trade_routes",
        owner: OwnerKind::Country,
        tables: &[TableId::TradeRoutes],
    },
];

/// Scans the country census into `selection` at `hour` for a family read
/// once a day per native country id with a constant print: a new country, or
/// one whose last read was not published, joins the read at once. Without a
/// store every country is read, and so is every country of an hour whose
/// census could not be read whole.
pub fn scan_countries(
    memory: &impl engine_native::Memory,
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

/// The family that reads `table`, if it is sliced.
pub fn family_of(table: TableId) -> Option<usize> {
    FAMILIES
        .iter()
        .position(|family| family.tables.contains(&table))
}

/// Worker/spool insertion sizes from an original observed owner occurrence,
/// retaining ascending output without allocating an owner's entire ID domain.
/// A census visits owners in ascending order, so each lands at the end.
fn insert_owner(owners: &mut Vec<u32>, owner: u32) -> bool {
    let index = match owners.last() {
        Some(&last) if last < owner => owners.len(),
        None => 0,
        Some(_) => match owners.binary_search(&owner) {
            Ok(_) => return true,
            Err(index) => index,
        },
    };
    if engine_allocation::reserve_push_batch(owners, 1).is_err() {
        return false;
    }
    owners.insert(index, owner);
    true
}

/// The owners one hour's worker read of a family covered. Constructors are
/// empty; original observed owners grow storage before their read is published.
pub struct SliceSelection {
    pub hour: i32,
    pub force_full: bool,
    /// The family reads every owner at the daily boundary and no rotating
    /// slice ([`Self::reset_daily`]).
    daily: bool,
    /// A daily family's hour is its daily boundary.
    boundary: bool,
    /// Owners the scan found without drift this hour, ascending
    /// ([`Self::scan_drifting`]): neither the slice nor the daily boundary
    /// reads them, only a mark, a changed print or a full hour.
    settled: Vec<u32>,
    /// Owners read before their slice because their fingerprint changed.
    early: Vec<u32>,
    pub early_count: u32,
    /// Owners the scan saw this hour; meaningful once `census` is set.
    seen: Vec<u32>,
    /// The scan visited every live owner without a census error.
    census: bool,
    /// Owners the family's walker read this hour, when it reports them
    /// (`track_reads`): a walker that visits only some census owners stamps
    /// only those it read.
    walked: Vec<u32>,
    tracked: bool,
    /// Owners this hour's take of the family's hook marks named, ascending.
    marked: Vec<u32>,
    pub marked_count: u32,
    /// Owners whose fingerprint was compared this hour.
    pub scanned: u32,
    /// Owners whose bookkeeping allocation could not complete.
    pub out_of_range: u32,
    /// Worker ticks spent computing fingerprints (not game-thread time).
    pub scan_ticks: u64,
}

impl SliceSelection {
    /// Both owned packs and callback placeholders start without owner storage.
    pub fn new(_storage: bool) -> Self {
        Self {
            hour: 0,
            force_full: true,
            daily: false,
            boundary: false,
            settled: Vec::new(),
            early: Vec::new(),
            early_count: 0,
            seen: Vec::new(),
            census: false,
            walked: Vec::new(),
            tracked: false,
            marked: Vec::new(),
            marked_count: 0,
            scanned: 0,
            out_of_range: 0,
            scan_ticks: 0,
        }
    }

    /// Start an hour's read.
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

    /// Start an hour's read of a family with no rotating slice: every owner
    /// at the daily boundary, the first hour after daily update
    /// (daily game update, inside the hourly update entered at hour 23), and on a
    /// full hour; otherwise only the owners selected or marked.
    pub fn reset_daily(&mut self, hour: i32, force_full: bool) {
        self.reset(hour, force_full);
        self.daily = true;
        self.boundary = hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD) == 0;
    }

    /// Start an hour's read of a family whose every recorded field has
    /// hooked writers: no rotating slice and no daily boundary, only the
    /// owners selected or marked, and every owner on a full hour.
    pub fn reset_hooked(&mut self, hour: i32, force_full: bool) {
        self.reset(hour, force_full);
        self.daily = true;
    }

    /// Whether this hour reads any owner: a full hour, a selected or marked
    /// owner, or a scanned owner its slice covers.
    pub fn reads_any(&self) -> bool {
        self.force_full
            || !self.early.is_empty()
            || self.seen.iter().any(|&owner| self.contains(owner))
    }

    /// Owners the scan saw that this hour does not read: their held rows
    /// carry.
    pub fn carried(&self) -> usize {
        self.seen
            .iter()
            .filter(|&&owner| !self.contains(owner))
            .count()
    }

    /// [`Self::scan_drifting`] reading `drifts` only when it decides
    /// something: for an owner this hour's slice or daily boundary covers, a
    /// full hour, or an owner the store does not hold a current print for.
    /// Any other owner is only seen, keeping its held print: its rows
    /// change only through a mark until its slice, whose check reads it if
    /// it drifts now or its drift changed since its last check.
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
        if !insert_owner(&mut self.seen, owner) {
            self.out_of_range += 1;
            self.force_full = true;
        }
    }

    /// [`Self::scan`] for a family whose rows drift only for some owners: an
    /// owner without drift (`drifts` false) is settled, read only when a
    /// writer marks it, its print changes or the hour is full. The caller
    /// folds `drifts` into `fingerprint`, so an owner that starts or stops
    /// drifting is read at that hour.
    pub fn scan_drifting(
        &mut self,
        fingerprints: &mut SliceFingerprints,
        owner: u32,
        fingerprint: Option<u64>,
        drifts: bool,
    ) {
        if !drifts && !insert_owner(&mut self.settled, owner) {
            self.out_of_range += 1;
            self.force_full = true;
            return;
        }
        self.scan(fingerprints, owner, fingerprint);
    }

    /// Compare one owner's fingerprint (None: unreadable) with the store and
    /// add it to this hour's read if it changed or is new outside its slice.
    pub fn scan(
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

    /// [`Self::scan`] for a family whose fingerprint is a certificate of
    /// every recorded field (the naval mission inputs): an owner whose
    /// certificate changed is marked ([`Self::mark`]).
    pub fn scan_marking(
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
        if fingerprints.observe(owner, fingerprint) {
            self.mark(owner);
        }
    }

    /// Add `owner` to this hour's read outside its slice, without a
    /// fingerprint. Storage grows on the same worker before publication.
    pub fn select(&mut self, owner: u32) -> bool {
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
    pub fn finish_census(&mut self, fingerprints: &mut SliceFingerprints) {
        if self.out_of_range != 0 || engine_allocation::capture_stopped() {
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
    pub fn finish_pass(&mut self, fingerprints: &mut SliceFingerprints, complete: bool) {
        if complete {
            self.finish_census(fingerprints);
        } else {
            self.force_full = true;
        }
    }

    /// Whether `owner` may still hold rows: seen by a complete census or not
    /// known to be gone (no complete census).
    pub fn live(&self, owner: u32) -> bool {
        !self.census || self.seen.binary_search(&owner).is_ok()
    }

    /// Whether this hour's read covers `owner`.
    pub fn contains(&self, owner: u32) -> bool {
        self.force_full
            || (self.in_slice(owner, false) && self.settled.binary_search(&owner).is_err())
            || self.early.binary_search(&owner).is_ok()
    }

    /// Whether `owner` is in this hour's slice (for a daily family, every
    /// owner at its boundary and none off it).
    fn in_slice(&self, owner: u32, force_full: bool) -> bool {
        force_full
            || if self.daily {
                self.boundary
            } else {
                replay_data::cadence::slice_active(owner, self.hour, false)
            }
    }

    /// A hooked native writer touched `owner` since the previous take: it
    /// joins this hour's read. Storage grows on the same worker before
    /// publication.
    pub fn mark(&mut self, owner: u32) {
        let new = self.marked.binary_search(&owner).is_err();
        if !insert_owner(&mut self.marked, owner) {
            self.out_of_range += 1;
            self.force_full = true;
            return;
        }
        if new {
            self.marked_count += 1;
        }
        self.select(owner);
    }

    /// End this hour's take of the family's hook marks; `epoch` is the
    /// take's, `None` when it certified nothing. Unless it is the epoch under
    /// which the store's previous hour was read, an owner outside this
    /// hour's read may have changed unseen: every owner is read. The store
    /// then holds `epoch`, so the next hour's take can certify this hour's
    /// read.
    pub fn certify(&mut self, store: &mut SliceFingerprints, epoch: Option<u64>) {
        if epoch.is_none() || epoch != store.marks {
            self.force_full = true;
        }
        store.marks = epoch;
    }

    /// The marked owners outside this hour's slice: read only for a mark.
    pub fn marked_early(&self) -> u32 {
        self.marked
            .iter()
            .filter(|&&owner| !self.in_slice(owner, false))
            .count() as u32
    }

    /// This hour's walker reports the owners it read (`mark_read`); only
    /// those are read owners.
    pub fn track_reads(&mut self) {
        self.tracked = true;
        self.walked.clear();
    }

    /// The walker read `owner` this hour, rows or none.
    pub fn mark_read(&mut self, owner: u32) {
        if !insert_owner(&mut self.walked, owner) {
            self.out_of_range += 1;
            self.force_full = true;
        }
    }

    /// Owners the scan saw that this hour's read covers, ascending: the owners
    /// read now, including those that returned no rows (with a tracking
    /// walker, only those it read). Empty without a scan.
    pub fn read_owners(&self) -> impl Iterator<Item = u32> + '_ {
        self.seen
            .iter()
            .copied()
            .filter(|&owner| self.contains(owner))
            .filter(|&owner| !self.tracked || self.walked.binary_search(&owner).is_ok())
    }

    /// This hour's read was not published (failed, truncated or dropped):
    /// every owner it selected is a change at its next scan, so the next hour
    /// reads them again.
    pub fn invalidate(&self, fingerprints: &mut SliceFingerprints) {
        for &owner in &self.early {
            fingerprints.invalidate(owner);
        }
        fingerprints.stale_slices |= if self.force_full || (self.daily && self.boundary) {
            u32::MAX
        } else if self.daily {
            0
        } else {
            1 << self.hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD)
        };
        // A change-driven family's take this hour spent marks of owners it
        // may not have read: the next hour certifies nothing and reads every
        // owner ([`select_changed`]).
        fingerprints.held.epoch = None;
    }

    /// A selection as a family would record it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(hour: i32, force_full: bool, early: &[u32]) -> Self {
        let mut selection = Self::new(true);
        selection.reset(hour, force_full);
        for &owner in early {
            assert!(insert_owner(&mut selection.early, owner));
            selection.early_count += 1;
        }
        selection
    }

    /// This selection after a complete census that saw only `seen`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_census(mut self, seen: &[u32]) -> Self {
        for &owner in seen {
            assert!(insert_owner(&mut self.seen, owner));
        }
        self.census = true;
        self
    }
}

/// The latest change fingerprint per owner for one sliced family.
pub struct SliceFingerprints {
    values: HashMap<u32, Fingerprint>,
    /// Slots (bit per slice) whose last read was not published; every owner
    /// of such a slice counts as a change at the next scan.
    stale_slices: u32,
    /// A change-driven family's certificates ([`Held`]).
    pub held: Held,
    /// The hook marks' epoch the family's latest hour was read under
    /// ([`SliceSelection::certify`]); `None` before a certified take.
    marks: Option<u64>,
    /// The research and technology rows of each owner's last complete read
    /// that a certified slice carries ([`crate::drift`]).
    pub drift: crate::drift::Store,
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
    /// Storage follows only the original owners observed by each worker scan.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            stale_slices: 0,
            held: Held::default(),
            marks: None,
            drift: Default::default(),
        }
    }

    /// Forget every observation, e.g. after an interrupted job. Every owner
    /// is then new at its next scan and is read at once.
    pub fn forget(&mut self) {
        self.values.clear();
        self.stale_slices = 0;
        self.held = Held::default();
        self.marks = None;
        self.drift.clear();
    }

    /// Forget every owner a complete census did not see.
    fn keep_only(&mut self, seen: &[u32]) {
        self.values
            .retain(|owner, _| seen.binary_search(owner).is_ok());
        self.drift.retain(seen);
    }

    /// Make `owner` a change at its next scan.
    pub fn invalidate(&mut self, owner: u32) {
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

    /// Whether `owner`'s print is held and current: known, not invalidated,
    /// readable, and its slice not stale.
    fn holds(&self, owner: u32) -> bool {
        self.stale_slices & (1 << replay_data::cadence::slice_of(owner)) == 0
            && self
                .values
                .get(&owner)
                .is_some_and(|print| !print.stale && print.value.is_some())
    }

    /// The print `owner` holds, when current.
    pub fn held(&self, owner: u32) -> Option<u64> {
        self.holds(owner)
            .then(|| self.values.get(&owner).and_then(|print| print.value))
            .flatten()
    }

    /// After a read of `owner`, hold `value` as its print without making
    /// it a change.
    pub fn hold(&mut self, owner: u32, value: u64) {
        if let Some(print) = self.values.get_mut(&owner)
            && !print.stale
        {
            print.value = Some(value);
        }
    }

    /// End an hour's scan: whole-slice invalidations have reached every owner.
    pub fn finish_scan(&mut self) {
        self.stale_slices = 0;
    }
}

/// A change-driven sliced family's certificates
/// (the country-input observer). Its hooked writers mark the owners they
/// change, and the hour reads those beside its slice. An owner no writer
/// marked keeps the rows of its last read only while the hour's take returns
/// the epoch held here (every write since the last take was marked) and the
/// per-read globals every row carries hash as held; otherwise the hour reads
/// every owner. (A family that carries an owner's daily drift from its held
/// rows, [`crate::drift`], reads only that drift at such an owner's slice.)
#[derive(Default)]
pub struct Held {
    /// The dirty-set epoch the latest take returned.
    epoch: Option<u64>,
    /// The hash of the globals the latest read's rows carry.
    context: Option<u64>,
}

mod digest;
pub use digest::Digest;

/// One hour's change-driven selection ([`select_changed`]).
#[derive(Default)]
pub struct Changed {
    /// The owners the take marked, ascending.
    marked: Vec<u32>,
    /// The take certified every unmarked owner's held rows.
    pub certified: bool,
}

impl Changed {
    /// Start an hour: nothing marked or certified.
    pub fn reset(&mut self) {
        self.marked.clear();
        self.certified = false;
    }

    pub fn marked(&self) -> usize {
        self.marked.len()
    }

    /// Whether `owner`'s held rows certify it this hour: the take certified
    /// every write and named no write of it.
    pub fn carries(&self, owner: u32) -> bool {
        self.certified && self.marked.binary_search(&owner).is_err()
    }

    /// A worker's read of `owner` did not complete: hold none of its drift
    /// rows, so its next slice reads it whole.
    pub fn unread(&mut self, store: &mut SliceFingerprints, owner: u32) {
        store.drift.forget(owner);
    }
}

/// After the hour's census scan ([`SliceSelection::scan`]), the family's
/// hourly take of its marks: selects each marked owner, and reads every
/// owner unless `take` returned the epoch the store holds and `context`
/// (the hash of the globals every row carries) equals the held one.
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
    fn digest_word_helpers_keep_the_exact_little_endian_byte_stream() {
        let mut expected = 0xcbf2_9ce4_8422_2325_u64;
        let mut digest = Digest::new();
        for value in [0, 1, u64::MAX, i64::MIN as u64, 0x1234_5678_9abc_def0] {
            digest.u64(value).i64(value as i64);
            for byte in value.to_le_bytes().into_iter().cycle().take(16) {
                expected = (expected ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
            assert_eq!(digest.finish(), expected);
        }
        let text = b"owned\0bytes";
        digest.text(text);
        for byte in (text.len() as u64)
            .to_le_bytes()
            .into_iter()
            .chain(text.iter().copied())
        {
            expected = (expected ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        assert_eq!(digest.finish(), expected);
        digest.bytes(&[]);
        assert_eq!(digest.finish(), expected);
    }

    #[test]
    fn digest_zero_suffix_shortcut_preserves_every_word_boundary() {
        let mut expected = Digest::new();
        let mut actual = Digest::new();
        for seed in [0, 1, 0x9876_5432_10fe_dcba_u64, u64::MAX] {
            for bits in 0..64 {
                for value in [seed, 1_u64 << bits, (1_u64 << bits).wrapping_sub(1)] {
                    expected.bytes(&value.to_le_bytes()).bytes(b"next");
                    actual.u64(value).bytes(b"next");
                    assert_eq!(actual.finish(), expected.finish());
                    expected.bytes(&(value as i64).to_le_bytes());
                    actual.i64(value as i64);
                    assert_eq!(actual.finish(), expected.finish());
                }
            }
        }
    }

    /// A certificate family marks each owner whose certificate changed,
    /// including one seen for the first time.
    #[test]
    fn certificate_scans_mark_owners_whose_certificate_changed() {
        let mut store = SliceFingerprints::new();
        let hour = |store: &mut SliceFingerprints, prints: [u64; 3]| {
            let mut selection = SliceSelection::new(true);
            selection.reset(0, true);
            for (owner, print) in (1..).zip(prints) {
                selection.scan_marking(store, owner, Some(print));
            }
            selection.finish_census(store);
            selection.marked.clone()
        };
        // First sight: every owner is new.
        assert_eq!(hour(&mut store, [1, 1, 1]), [1, 2, 3]);
        // Owner 2's certificate moved; the others held.
        assert_eq!(hour(&mut store, [1, 2, 1]), [2]);
        assert!(hour(&mut store, [1, 2, 1]).is_empty());
    }

    #[test]
    fn selections_and_fingerprints_start_empty_without_callback_allocation() {
        crate::test_support::with_allocations_denied(|| {
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
    fn failed_owner_bookkeeping_never_certifies_a_partial_census_or_forgets_healthy_owners() {
        let mut original = SliceSelection::new(true);
        let mut fingerprints = SliceFingerprints::new();
        original.scan(&mut fingerprints, 7, Some(1));
        let mut failed = SliceSelection::new(false);
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
                usize::from(replay_data::cadence::is_sliced(table)),
                "{table:?}"
            );
            assert_eq!(
                family_of(table).is_some(),
                replay_data::cadence::is_sliced(table)
            );
        }
    }

    /// A hook-marked owner joins the hour's read outside its slice. Only a
    /// take in the epoch the store's previous hour was read under keeps the
    /// slice: a refused take, another epoch and a forgotten store read every
    /// owner.
    #[test]
    fn marked_owners_join_the_read_and_an_uncertified_take_reads_everyone() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(true);
        let census = |selection: &mut SliceSelection, store: &mut SliceFingerprints| {
            for owner in [1, 2, 3, 30] {
                selection.scan(store, owner, Some(0));
            }
            store.finish_scan();
            selection.finish_census(store);
        };
        // The first take certifies no earlier read.
        selection.reset(0, false);
        census(&mut selection, &mut store);
        selection.certify(&mut store, Some(7));
        assert!(selection.force_full);
        // The same epoch: hour 2's slice and the marked owner 3, nobody else.
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
        // A refused take, then another epoch.
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
        // A forgotten store holds no epoch.
        store.forget();
        selection.reset(7, false);
        census(&mut selection, &mut store);
        selection.certify(&mut store, Some(8));
        assert!(selection.force_full);
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

    /// One change-driven hour over `owners`: the census scan, then the take
    /// of `marks` returning `epoch`.
    fn changed_hour(
        store: &mut SliceFingerprints,
        changed: &mut Changed,
        hour: i32,
        owners: &[u32],
        marks: &[usize],
        epoch: Option<u64>,
        context: u64,
    ) -> SliceSelection {
        let mut selection = SliceSelection::new(true);
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

    /// A marked owner is read within the hour; an unmarked one waits for its
    /// slice only while the take returns the held epoch and the globals hash
    /// as held. Anything else reads every owner.
    #[test]
    fn marked_owners_join_the_hour_and_an_uncertified_take_reads_everyone() {
        let owners = [1, 2, 3, 29];
        let mut store = SliceFingerprints::new();
        let mut changed = Changed::default();
        // First hour: nothing is held, so everyone is read.
        let first = changed_hour(&mut store, &mut changed, 5, &owners, &[], Some(7), 1);
        assert!(first.force_full && !changed.certified);
        // Same epoch and globals: only the slice (5, 29) and the marked 2.
        let next = changed_hour(&mut store, &mut changed, 5, &owners, &[2], Some(7), 1);
        assert!(changed.certified && !next.force_full);
        assert_eq!(changed.marked(), 1);
        assert!(next.contains(2) && next.contains(29) && !next.contains(1) && !next.contains(3));
        // A quiet hour reads the slice alone.
        let quiet = changed_hour(&mut store, &mut changed, 6, &owners, &[], Some(7), 1);
        assert!(!quiet.force_full && (1..=3).all(|owner| !quiet.contains(owner)));
        // A moved epoch, a refused take or changed globals read everyone.
        for (epoch, context) in [(Some(8), 1), (None, 1), (Some(7), 2)] {
            let mut store = SliceFingerprints::new();
            changed_hour(&mut store, &mut changed, 5, &owners, &[], Some(7), 1);
            let hour = changed_hour(&mut store, &mut changed, 6, &owners, &[], epoch, context);
            assert!(hour.force_full && !changed.certified, "{epoch:?} {context}");
        }
        // A forgotten store holds nothing: the next hour reads everyone.
        store.forget();
        let forgotten = changed_hour(&mut store, &mut changed, 6, &owners, &[], Some(7), 1);
        assert!(forgotten.force_full);
    }

    /// An hour whose read was not published spent marks of owners it may not
    /// have read: the next hour certifies nothing and reads every owner,
    /// then settles.
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

    /// A daily family reads every owner at the first hour after the daily
    /// tick and on a full hour, and otherwise only the owners selected for
    /// it: no rotating slice. An unpublished off-boundary hour re-reads only
    /// its selected owners.
    #[test]
    fn a_daily_family_reads_everyone_at_the_boundary_and_only_selections_between() {
        let mut selection = SliceSelection::new(false);
        for hour in [0, 24, -24] {
            selection.reset_daily(hour, false);
            assert!(!selection.force_full && selection.contains(5), "{hour}");
        }
        selection.reset_daily(5, true);
        assert!(selection.contains(7));
        // The boundary hour's census makes owners 5 and 29 known.
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
        let mut next = SliceSelection::new(false);
        next.reset_daily(6, false);
        next.scan(&mut store, 5, Some(0));
        next.scan(&mut store, 29, Some(0));
        assert!(next.contains(29) && !next.contains(5));
        // A rotating family's reset clears the daily mode.
        next.reset(5, false);
        assert!(next.contains(5));
    }

    /// An owner without drift is in neither the slice nor the daily
    /// boundary: over a quiet day the rotating family reads each drifting
    /// owner once and no settled one, the daily family reads only the
    /// drifting owners at its boundary. A change of drift, a mark or a full
    /// hour still reads a settled owner. 446 owners, one in seven drifting,
    /// as about a sixth of 1936's census owns a state.
    #[test]
    fn settled_owners_are_read_only_for_a_change() {
        let owners: Vec<u32> = (1..=446).collect();
        let drifts = |owner: u32| owner.is_multiple_of(7);
        let drifting = owners.iter().filter(|&&owner| drifts(owner)).count();
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(false);
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
        // The first hour sees every owner new.
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
        let mut daily = SliceSelection::new(false);
        daily.reset_daily(48, false);
        assert_eq!(hour_reads(&mut daily, &mut store), drifting);
        daily.reset_daily(49, false);
        assert_eq!(hour_reads(&mut daily, &mut store), 0);
        // Owner 1 starts drifting: read at once, then with the drifting.
        daily.reset_daily(50, false);
        daily.scan_drifting(&mut store, 1, Some(1), true);
        assert!(daily.contains(1));
        // A mark or a full hour reads a settled owner.
        daily.reset_daily(51, false);
        daily.scan_drifting(&mut store, 2, Some(0), false);
        assert!(!daily.contains(2));
        daily.mark(2);
        assert!(daily.contains(2));
        daily.reset_daily(52, true);
        daily.scan_drifting(&mut store, 3, Some(0), false);
        assert!(daily.contains(3));
    }

    /// A lazy scan reads an owner's drift only at its slice, its family's
    /// daily boundary, a full hour or without a current held print: over a
    /// quiet day each owner's drift is read once.
    #[test]
    fn drift_is_read_only_when_it_decides_a_read() {
        let owners: Vec<u32> = (1..=446).collect();
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(false);
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
        // The first hour holds no print: every owner is checked.
        selection.reset(0, false);
        hour(&mut selection, &mut store);
        assert_eq!(checks.replace(0), owners.len());
        for at in 1..=24 {
            selection.reset(at, false);
            hour(&mut selection, &mut store);
        }
        assert_eq!(checks.replace(0), owners.len());
        let mut daily = SliceSelection::new(false);
        daily.reset_daily(25, false);
        hour(&mut daily, &mut store);
        assert_eq!(checks.replace(0), 0);
        daily.reset_daily(48, false);
        hour(&mut daily, &mut store);
        assert_eq!(checks.replace(0), owners.len());
        // An invalidated owner is checked at once.
        store.invalidate(5);
        daily.reset_daily(49, false);
        hour(&mut daily, &mut store);
        assert_eq!(checks.replace(0), 1);
        assert!(daily.contains(5));
    }
}
