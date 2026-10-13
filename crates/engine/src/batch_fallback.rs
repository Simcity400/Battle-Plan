//! Bounded fallbacks of batched walks.
//!
//! A walk that reads several objects' spans in one guarded copy and finds
//! the copy incomplete reads that batch once more object by object, as the
//! serial walk does, so a persistent fault gives the serial walk's result.
//! Each such fallback is counted here for status, as is a country link
//! read ahead of its turn ([`Ahead`]) whose read failed and is read again at
//! its turn.
use crate::{Memory, ccountry, read};
use std::sync::atomic::{AtomicU64, Ordering};

/// A batched walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Batch {
    /// One block per resource pool ([`crate::ccountry_resources`]).
    CountryResources,
    /// One block per agency ([`crate::cintelligence_agency`]).
    IntelligenceAgencies,
    /// One block per run of decryption states ([`crate::ccryptology`]).
    Cryptology,
    /// One scatter of shared-census divisions' first viewer entry blocks
    /// ([`crate::unit_viewer_entry`]).
    UnitIntel,
    /// A batch of countries' production status pointers, then their status
    /// spans ([`crate::cproduction_status`]).
    CountryFactories,
    /// One batch of an owner's relation candidates' matrix cells, then their
    /// statuses ([`crate::crelation_status`]).
    RelationLinks,
    /// The AirWing census's wing-pointer runs and row and name-header
    /// gathers ([`crate::cair_wing`]).
    AirWingCensus,
    /// AirReferenceWitnesses' base gathers
    /// ([`crate::map_air_base_reference::Pass::read_bases`]).
    BaseWitnesses,
    /// AirMissionInputs' mission operand and CAS ratio copies
    /// ([`crate::map_air_sortie_inputs::read_missions`]); counted as the
    /// fallback starts, whatever its retry returns.
    MissionInputs,
    /// AirMissionInputs' pool definition gathers
    /// ([`crate::map_air_sortie_inputs::definition_air_flags_gathered`]).
    PoolDefinitions,
    /// One batch of a country's owned relation entries, type words and
    /// subject heads ([`crate::csubject_relation`]).
    Subjects,
    /// One run of up to 64 census ships' fields ([`crate::cfleet`]).
    ShipRuns,
    /// One census fleet's gathered fields ([`crate::cfleet`]).
    FleetGathers,
    /// One census task force's gathered fields ([`crate::cfleet`]).
    TaskForceGathers,
    /// One gathered object of the counter walk's fresh ETA graph (stat
    /// block, province, region, template, fleet, fleet leader;
    /// [`crate::ctask_force`]).
    FreshEtaGathers,
    /// One naval combat member's gathered fixed fields
    /// ([`crate::cfex_member`]).
    NavalMemberRecords,
    /// One run of up to 64 nested naval combat entries (damage entries, or
    /// air or naval hits with their records; [`crate::cfex_member`]).
    NavalEntryRuns,
    /// Governance slot pointers reader.
    GovernanceSlotPointers,
    /// Governance slots reader.
    GovernanceSlots,
    /// Governance assigned technologies reader.
    GovernanceAssignedTechnologies,
    /// Governance assigned templates reader.
    GovernanceAssignedTemplates,
    /// Governance modifier technologies reader.
    GovernanceModifierTechnologies,
    /// Governance modifier ideas reader.
    GovernanceModifierIdeas,
    /// Governance modifier scripts reader.
    GovernanceModifierScripts,
    /// Governance focus progress reader.
    GovernanceFocusProgress,
    /// Governance focus pointers reader.
    GovernanceFocusPointers,
    /// Governance focus definitions reader.
    GovernanceFocusDefinitions,
    /// Governance research drift reader.
    GovernanceResearchDrift,
    /// Governance technology drift reader.
    GovernanceTechnologyDrift,
    /// Bounded serial fallback for order roots.
    OrderRoots,
    /// Bounded serial fallback for order census.
    OrderCensus,
    /// Bounded serial fallback for order plans.
    OrderPlans,
    /// Bounded serial fallback for division maxima.
    DivisionMaxima,
    /// Bounded serial fallback for division acclimatization.
    DivisionAcclimatization,
    /// Bounded serial fallback for division naming.
    DivisionNaming,
    /// Bounded serial fallback for division provinces.
    DivisionProvinces,
    /// Bounded serial fallback for division movement speed.
    DivisionMovementSpeed,
    /// One run of up to 64 order-of-battle member or child entries, or one
    /// member's host identity and group backlink, whose guarded copy
    /// stopped early: the Naval copy reads on after the failed span.
    OrderOfBattle,
}

impl Batch {
    pub const ALL: [Self; 38] = [
        Self::CountryResources,
        Self::IntelligenceAgencies,
        Self::Cryptology,
        Self::UnitIntel,
        Self::CountryFactories,
        Self::RelationLinks,
        Self::AirWingCensus,
        Self::BaseWitnesses,
        Self::MissionInputs,
        Self::PoolDefinitions,
        Self::Subjects,
        Self::ShipRuns,
        Self::FleetGathers,
        Self::TaskForceGathers,
        Self::FreshEtaGathers,
        Self::NavalMemberRecords,
        Self::NavalEntryRuns,
        Self::GovernanceSlotPointers,
        Self::GovernanceSlots,
        Self::GovernanceAssignedTechnologies,
        Self::GovernanceAssignedTemplates,
        Self::GovernanceModifierTechnologies,
        Self::GovernanceModifierIdeas,
        Self::GovernanceModifierScripts,
        Self::GovernanceFocusProgress,
        Self::GovernanceFocusPointers,
        Self::GovernanceFocusDefinitions,
        Self::GovernanceResearchDrift,
        Self::GovernanceTechnologyDrift,
        Self::OrderRoots,
        Self::OrderCensus,
        Self::OrderPlans,
        Self::DivisionMaxima,
        Self::DivisionAcclimatization,
        Self::DivisionNaming,
        Self::DivisionProvinces,
        Self::DivisionMovementSpeed,
        Self::OrderOfBattle,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::CountryResources => "country_resources",
            Self::IntelligenceAgencies => "intelligence_agencies",
            Self::Cryptology => "cryptology",
            Self::UnitIntel => "unit_intel",
            Self::CountryFactories => "country_factories",
            Self::RelationLinks => "relation_links",
            Self::AirWingCensus => "air_wing_census",
            Self::BaseWitnesses => "air_base_witnesses",
            Self::MissionInputs => "air_mission_inputs",
            Self::PoolDefinitions => "air_pool_definitions",
            Self::Subjects => "subjects",
            Self::ShipRuns => "ship_runs",
            Self::FleetGathers => "fleet_gathers",
            Self::TaskForceGathers => "task_force_gathers",
            Self::FreshEtaGathers => "fresh_eta_gathers",
            Self::NavalMemberRecords => "naval_member_records",
            Self::NavalEntryRuns => "naval_entry_runs",
            Self::GovernanceSlotPointers => "slot_pointers",
            Self::GovernanceSlots => "slots",
            Self::GovernanceAssignedTechnologies => "assigned_technologies",
            Self::GovernanceAssignedTemplates => "assigned_templates",
            Self::GovernanceModifierTechnologies => "modifier_technologies",
            Self::GovernanceModifierIdeas => "modifier_ideas",
            Self::GovernanceModifierScripts => "modifier_scripts",
            Self::GovernanceFocusProgress => "focus_progress",
            Self::GovernanceFocusPointers => "focus_pointers",
            Self::GovernanceFocusDefinitions => "focus_definitions",
            Self::GovernanceResearchDrift => "research_drift",
            Self::GovernanceTechnologyDrift => "technology_drift",
            Self::OrderRoots => "order_roots",
            Self::OrderCensus => "order_census",
            Self::OrderPlans => "order_plans",
            Self::DivisionMaxima => "division_maxima",
            Self::DivisionAcclimatization => "division_acclimatization",
            Self::DivisionNaming => "division_naming",
            Self::DivisionProvinces => "division_provinces",
            Self::DivisionMovementSpeed => "division_movement_speed",
            Self::OrderOfBattle => "order_of_battle",
        }
    }
}

static FALLBACKS: [AtomicU64; Batch::ALL.len()] = [const { AtomicU64::new(0) }; Batch::ALL.len()];

#[cfg(any(test, feature = "batch-fallback-test-counts"))]
thread_local! {
    /// This thread's fallbacks, including when engine is a test dependency
    /// of collector (where engine's cfg(test) is not enabled).
    static THREAD: [std::cell::Cell<u64>; Batch::ALL.len()] =
        const { [const { std::cell::Cell::new(0) }; Batch::ALL.len()] };
}

/// One batch of `batch` read again object by object.
pub fn record(batch: Batch) {
    FALLBACKS[batch as usize].fetch_add(1, Ordering::Relaxed);
    #[cfg(any(test, feature = "batch-fallback-test-counts"))]
    THREAD.with(|counts| counts[batch as usize].set(counts[batch as usize].get() + 1));
}

/// The next census slot of a country walk, read ahead of its turn: its
/// entry as [`ccountry::Reader::at`] gave it and, for a country, one link
/// field of the country object. Each is used at its own place in the serial
/// order.
pub(crate) struct Ahead {
    slot: u32,
    listed: Result<Option<ccountry::Country>, &'static str>,
    link: Link,
}

/// A country's link field as its walk holds it at the country's turn.
#[derive(Clone, Copy)]
pub(crate) enum Link {
    /// Not read ahead.
    Unread,
    Read(u64),
    /// Its read ahead failed.
    Failed,
}

impl Ahead {
    /// Reads slot `slot`'s entry and, for a country, its link at `offset`,
    /// handing a nonnull link to `hint` (which may only hint); `None` past
    /// the census.
    pub(crate) fn read<M: Memory>(
        memory: &M,
        countries: &ccountry::Reader<'_, M>,
        slot: u32,
        offset: usize,
        hint: impl FnOnce(usize),
    ) -> Option<Self> {
        if slot >= countries.count() {
            return None;
        }
        let listed = countries.at(slot);
        let link = match listed {
            Ok(Some(country)) => match country
                .pointer
                .checked_add(offset)
                .and_then(|at| read::<u64>(memory, at))
            {
                Some(link) => Link::Read(link),
                None => Link::Failed,
            },
            _ => Link::Unread,
        };
        if let Link::Read(link) = link
            && let Ok(target) = usize::try_from(link)
            && target != 0
        {
            hint(target);
        }
        Some(Self { slot, listed, link })
    }

    /// Slot `slot`'s entry and link: those read ahead for it, else its
    /// entry read now and its link unread.
    pub(crate) fn take<M: Memory>(
        ahead: &mut Option<Self>,
        countries: &ccountry::Reader<'_, M>,
        slot: u32,
    ) -> (Result<Option<ccountry::Country>, &'static str>, Link) {
        match ahead.take() {
            Some(ahead) if ahead.slot == slot => (ahead.listed, ahead.link),
            _ => (countries.at(slot), Link::Unread),
        }
    }
}

impl Link {
    /// `country`'s link at `offset`: the value read ahead, else read now. A
    /// read ahead that failed is read again here, once, and counted under
    /// `batch`.
    pub(crate) fn resolve(
        self,
        memory: &impl Memory,
        country: ccountry::Country,
        offset: usize,
        batch: Batch,
    ) -> Option<u64> {
        match self {
            Self::Read(link) => return Some(link),
            Self::Failed => record(batch),
            Self::Unread => {}
        }
        country
            .pointer
            .checked_add(offset)
            .and_then(|at| read::<u64>(memory, at))
    }
}

/// The fallbacks since attach, per batched walk.
pub fn counts() -> impl Iterator<Item = (&'static str, u64)> + Clone {
    Batch::ALL.into_iter().map(|batch| {
        (
            batch.name(),
            FALLBACKS[batch as usize].load(Ordering::Relaxed),
        )
    })
}

/// A source over a healthy image whose `ranges` fault on every touch, or
/// only on their first when `once`: the faults batched walks' tests place.
#[cfg(test)]
pub(crate) struct Faulty<'a> {
    memory: crate::memory::SliceMemory<'a>,
    ranges: Vec<std::ops::Range<usize>>,
    once: bool,
    spent: std::cell::RefCell<Vec<bool>>,
}

#[cfg(test)]
impl<'a> Faulty<'a> {
    pub(crate) fn new(
        memory: crate::memory::SliceMemory<'a>,
        ranges: Vec<std::ops::Range<usize>>,
        once: bool,
    ) -> Self {
        let spent = std::cell::RefCell::new(vec![false; ranges.len()]);
        Self {
            memory,
            ranges,
            once,
            spent,
        }
    }
}

#[cfg(test)]
impl crate::Memory for Faulty<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let end = address.saturating_add(destination.len());
        let mut spent = self.spent.borrow_mut();
        for (range, spent) in self.ranges.iter().zip(spent.iter_mut()) {
            if address < range.end && range.start < end && !*spent {
                *spent = self.once;
                return false;
            }
        }
        self.memory.copy(address, destination)
    }
}

/// `batch`'s fallbacks on this thread.
#[cfg(any(test, feature = "batch-fallback-test-counts"))]
pub fn count(batch: Batch) -> u64 {
    THREAD.with(|counts| counts[batch as usize].get())
}
