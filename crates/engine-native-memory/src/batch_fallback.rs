use crate::{Memory, country_identity::Country, read};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Batch {
    CountryResources,
    IntelligenceAgencies,
    Cryptology,
    UnitIntel,
    CountryFactories,
    RelationLinks,
    AirWingCensus,
    BaseWitnesses,
    MissionInputs,
    PoolDefinitions,
    Subjects,
    ShipRuns,
    FleetGathers,
    TaskForceGathers,
    FreshEtaGathers,
    NavalMemberRecords,
    NavalEntryRuns,
    GovernanceSlotPointers,
    GovernanceSlots,
    GovernanceAssignedTechnologies,
    GovernanceAssignedTemplates,
    GovernanceModifierTechnologies,
    GovernanceModifierIdeas,
    GovernanceModifierScripts,
    GovernanceFocusProgress,
    GovernanceFocusPointers,
    GovernanceFocusDefinitions,
    GovernanceResearchDrift,
    GovernanceTechnologyDrift,
    OrderRoots,
    OrderCensus,
    OrderPlans,
    DivisionMaxima,
    DivisionAcclimatization,
    DivisionNaming,
    DivisionProvinces,
    DivisionMovementSpeed,
    OrderOfBattle,
    ShipModelGuards,
}

impl Batch {
    pub const ALL: [Self; 39] = [
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
        Self::ShipModelGuards,
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
            Self::ShipModelGuards => "ship_model_guards",
        }
    }
}

static FALLBACKS: [AtomicU64; Batch::ALL.len()] = [const { AtomicU64::new(0) }; Batch::ALL.len()];

#[cfg(any(test, feature = "batch-fallback-test-counts"))]
thread_local! {
    static THREAD: [std::cell::Cell<u64>; Batch::ALL.len()] =
        const { [const { std::cell::Cell::new(0) }; Batch::ALL.len()] };
}

pub fn record(batch: Batch) {
    FALLBACKS[batch as usize].fetch_add(1, Ordering::Relaxed);
    #[cfg(any(test, feature = "batch-fallback-test-counts"))]
    THREAD.with(|counts| counts[batch as usize].set(counts[batch as usize].get() + 1));
}

#[derive(Clone, Copy)]
pub enum Link {
    Unread,
    Read(u64),
    Failed,
}

impl Link {
    pub fn resolve(
        self,
        memory: &impl Memory,
        country: Country,
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

pub fn counts() -> impl Iterator<Item = (&'static str, u64)> + Clone {
    Batch::ALL.into_iter().map(|batch| {
        (
            batch.name(),
            FALLBACKS[batch as usize].load(Ordering::Relaxed),
        )
    })
}

#[cfg(any(test, feature = "test-support"))]
pub struct Faulty<'a> {
    memory: crate::memory::SliceMemory<'a>,
    ranges: Vec<std::ops::Range<usize>>,
    once: bool,
    spent: std::cell::RefCell<Vec<bool>>,
}

#[cfg(any(test, feature = "test-support"))]
impl<'a> Faulty<'a> {
    pub fn new(
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

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "batch-fallback-test-counts"))]
pub fn count(batch: Batch) -> u64 {
    THREAD.with(|counts| counts[batch as usize].get())
}
