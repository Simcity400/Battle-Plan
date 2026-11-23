use crate::status::{map, status};
use std::{
    arch::x86_64::{_mm_lfence, _rdtsc},
    cell::Cell,
    marker::PhantomData,
    sync::{
        Once,
        atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering},
    },
};

const OWNED_SLOTS: usize = 64;
const SHARED_SLOT: usize = OWNED_SLOTS;
const SLOT_COUNT: usize = OWNED_SLOTS + 1;
const DEFAULT_PERIOD_LOG2: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Hourly,
    EquipmentDestruction,
    LandEvents,
    PortraitChanges,
    OpinionChanges,
    ConstructionIc,
    LineOutput,
    CombatLogPrune,
    AdvisorChanges,
    RelationChanges,
    HistoryChanges,
    FlagChanges,
    PropagandaChanges,
    IntelSources,
    NetworkAuxiliary,
    BuildingChanges,
    AdvisorPoolChanges,
    PartyLeaderChanges,
    VariableChanges,
    PartyChanges,
    WingChanges,
    RosterChanges,
    PoliticalStatusChanges,
    MissionChanges,
    RailChanges,
    TerritoryChanges,
    ArmyStatChanges,
    CountryInputChanges,
    RaidChanges,
    NavalTreeChanges,
    NavalManagementChanges,
    UnitLeaderChanges,
    CensusChanges,
    OrdersGroupChanges,
    OrderMembershipChanges,
    WorldInputChanges,
    DoctrineChanges,
    AirRecordChanges,
    LocalSubunitChanges,
    TemplateChanges,
    ShipModelChanges,
    ProvinceUnitChanges,
    ProvinceCombatChanges,
    CombatEngagementRefreshes,
    AirBaseCarryChanges,
    ExperienceGains,
    FuelAwards,
    OperationOutcomes,
    PoliticsCapture,
    PoliticsCaptureTaps,
    LogisticsBalance,
    PeaceSettlements,
    NavalResultSources,
    AirLosses,
    BuildingBombing,
    NuclearStrikes,
    EventFeed,
    EventFeedProvenance,
    MapEvents,
    WorldLifetime,
    VariantChanges,
    FactionGoals,
    FactionCompanions,
    FactionResourceRebuild,
    FactionRefreshPhases,
    MarketChanges,
    WarParticipation,
    CommandBatch,
    PlayerCommands,
}
pub const FAMILY_COUNT: usize = Family::PlayerCommands as usize + 1;
const _: () = assert!(FAMILY_COUNT <= crate::capture_gate::SLOTS);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Detour,
    Tap,
    Split,
    Mixed,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Detour => "detour",
            Self::Tap => "tap",
            Self::Split => "split",
            Self::Mixed => "mixed",
        }
    }
    fn times_original(self) -> bool {
        matches!(self, Self::Detour | Self::Mixed)
    }
}

impl Family {
    pub const ALL: [Self; FAMILY_COUNT] = [
        Self::Hourly,
        Self::EquipmentDestruction,
        Self::LandEvents,
        Self::PortraitChanges,
        Self::OpinionChanges,
        Self::ConstructionIc,
        Self::LineOutput,
        Self::CombatLogPrune,
        Self::AdvisorChanges,
        Self::RelationChanges,
        Self::HistoryChanges,
        Self::FlagChanges,
        Self::PropagandaChanges,
        Self::IntelSources,
        Self::NetworkAuxiliary,
        Self::BuildingChanges,
        Self::AdvisorPoolChanges,
        Self::PartyLeaderChanges,
        Self::VariableChanges,
        Self::PartyChanges,
        Self::WingChanges,
        Self::RosterChanges,
        Self::PoliticalStatusChanges,
        Self::MissionChanges,
        Self::RailChanges,
        Self::TerritoryChanges,
        Self::ArmyStatChanges,
        Self::CountryInputChanges,
        Self::RaidChanges,
        Self::NavalTreeChanges,
        Self::NavalManagementChanges,
        Self::UnitLeaderChanges,
        Self::CensusChanges,
        Self::OrdersGroupChanges,
        Self::OrderMembershipChanges,
        Self::WorldInputChanges,
        Self::DoctrineChanges,
        Self::AirRecordChanges,
        Self::LocalSubunitChanges,
        Self::TemplateChanges,
        Self::ShipModelChanges,
        Self::ProvinceUnitChanges,
        Self::ProvinceCombatChanges,
        Self::CombatEngagementRefreshes,
        Self::AirBaseCarryChanges,
        Self::ExperienceGains,
        Self::FuelAwards,
        Self::OperationOutcomes,
        Self::PoliticsCapture,
        Self::PoliticsCaptureTaps,
        Self::LogisticsBalance,
        Self::PeaceSettlements,
        Self::NavalResultSources,
        Self::AirLosses,
        Self::BuildingBombing,
        Self::NuclearStrikes,
        Self::EventFeed,
        Self::EventFeedProvenance,
        Self::MapEvents,
        Self::WorldLifetime,
        Self::VariantChanges,
        Self::FactionGoals,
        Self::FactionCompanions,
        Self::FactionResourceRebuild,
        Self::FactionRefreshPhases,
        Self::MarketChanges,
        Self::WarParticipation,
        Self::CommandBatch,
        Self::PlayerCommands,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::PlayerCommands => "player_commands",
            Self::Hourly => "hourly",
            Self::EquipmentDestruction => "equipment_destruction",
            Self::LandEvents => "land_events",
            Self::PortraitChanges => "portrait_changes",
            Self::OpinionChanges => "opinion_changes",
            Self::ConstructionIc => "construction_ic",
            Self::LineOutput => "line_output",
            Self::CombatLogPrune => "combat_log_prune",
            Self::AdvisorChanges => "advisor_changes",
            Self::RelationChanges => "relation_changes",
            Self::HistoryChanges => "history_changes",
            Self::FlagChanges => "flag_changes",
            Self::PropagandaChanges => "propaganda_changes",
            Self::IntelSources => "intel_sources",
            Self::NetworkAuxiliary => "network_auxiliary",
            Self::BuildingChanges => "building_changes",
            Self::AdvisorPoolChanges => "advisor_pool_changes",
            Self::PartyLeaderChanges => "party_leader_changes",
            Self::VariableChanges => "variable_changes",
            Self::PartyChanges => "party_changes",
            Self::WingChanges => "wing_changes",
            Self::RosterChanges => "roster_changes",
            Self::PoliticalStatusChanges => "political_status_changes",
            Self::MissionChanges => "mission_changes",
            Self::RailChanges => "rail_changes",
            Self::VariantChanges => "variant_changes",
            Self::TerritoryChanges => "territory_changes",
            Self::ArmyStatChanges => "army_stat_changes",
            Self::CountryInputChanges => "country_input_changes",
            Self::RaidChanges => "raid_changes",
            Self::NavalTreeChanges => "naval_tree_changes",
            Self::NavalManagementChanges => "naval_management_changes",
            Self::UnitLeaderChanges => "unit_leader_changes",
            Self::CensusChanges => "census_changes",
            Self::OrdersGroupChanges => "orders_group_changes",
            Self::OrderMembershipChanges => "order_membership_changes",
            Self::WorldInputChanges => "world_input_changes",
            Self::DoctrineChanges => "doctrine_changes",
            Self::AirRecordChanges => "air_record_changes",
            Self::LocalSubunitChanges => "local_subunit_changes",
            Self::TemplateChanges => "template_changes",
            Self::FactionGoals => "faction_goals",
            Self::FactionCompanions => "faction_companions",
            Self::FactionResourceRebuild => "faction_resource_rebuild",
            Self::FactionRefreshPhases => "faction_refresh_phases",
            Self::ShipModelChanges => "ship_model_changes",
            Self::ProvinceUnitChanges => "province_unit_changes",
            Self::ProvinceCombatChanges => "province_combat_changes",
            Self::CombatEngagementRefreshes => "combat_engagement_refreshes",
            Self::AirBaseCarryChanges => "air_base_carry_changes",
            Self::ExperienceGains => "experience_gains",
            Self::FuelAwards => "fuel_awards",
            Self::OperationOutcomes => "operation_outcomes",
            Self::PoliticsCapture => "politics_capture",
            Self::PoliticsCaptureTaps => "politics_capture_taps",
            Self::LogisticsBalance => "logistics_balance",
            Self::PeaceSettlements => "peace_settlements",
            Self::NavalResultSources => "naval_result_sources",
            Self::AirLosses => "air_losses",
            Self::BuildingBombing => "building_bombing",
            Self::NuclearStrikes => "nuclear_strikes",
            Self::EventFeed => "event_feed",
            Self::EventFeedProvenance => "event_feed_provenance",
            Self::MapEvents => "map_events",
            Self::WorldLifetime => "world_lifetime",
            Self::MarketChanges => "market_changes",
            Self::WarParticipation => "war_participation",
            Self::CommandBatch => "command_batch",
        }
    }
    pub fn kind(self) -> Kind {
        match self {
            Self::Hourly
            | Self::EquipmentDestruction
            | Self::PortraitChanges
            | Self::OpinionChanges
            | Self::ConstructionIc
            | Self::CombatLogPrune
            | Self::AdvisorChanges
            | Self::RelationChanges
            | Self::HistoryChanges
            | Self::FlagChanges
            | Self::PropagandaChanges
            | Self::IntelSources
            | Self::NetworkAuxiliary
            | Self::BuildingChanges
            | Self::AdvisorPoolChanges
            | Self::PartyLeaderChanges
            | Self::VariableChanges
            | Self::PartyChanges
            | Self::WingChanges
            | Self::RosterChanges
            | Self::PoliticalStatusChanges
            | Self::MissionChanges
            | Self::RailChanges
            | Self::VariantChanges
            | Self::TerritoryChanges
            | Self::ArmyStatChanges
            | Self::CountryInputChanges
            | Self::RaidChanges
            | Self::NavalTreeChanges
            | Self::UnitLeaderChanges
            | Self::CensusChanges
            | Self::OrdersGroupChanges
            | Self::OrderMembershipChanges
            | Self::WorldInputChanges
            | Self::AirRecordChanges
            | Self::LocalSubunitChanges
            | Self::TemplateChanges
            | Self::ShipModelChanges
            | Self::ProvinceUnitChanges
            | Self::ProvinceCombatChanges
            | Self::CombatEngagementRefreshes
            | Self::AirBaseCarryChanges
            | Self::OperationOutcomes
            | Self::PoliticsCapture
            | Self::LogisticsBalance
            | Self::NavalResultSources
            | Self::NuclearStrikes
            | Self::WarParticipation
            | Self::CommandBatch
            | Self::EventFeedProvenance
            | Self::FactionRefreshPhases
            | Self::FactionResourceRebuild => Kind::Detour,
            Self::LandEvents
            | Self::DoctrineChanges
            | Self::MapEvents
            | Self::MarketChanges
            | Self::FactionGoals
            | Self::FactionCompanions
            | Self::NavalManagementChanges => Kind::Mixed,
            Self::ExperienceGains
            | Self::PlayerCommands
            | Self::FuelAwards
            | Self::PoliticsCaptureTaps
            | Self::PeaceSettlements
            | Self::WorldLifetime => Kind::Tap,
            Self::LineOutput | Self::AirLosses | Self::BuildingBombing | Self::EventFeed => {
                Kind::Split
            }
        }
    }
    fn period_log2(self) -> u32 {
        match self {
            Self::Hourly | Self::MarketChanges => 0,
            _ => DEFAULT_PERIOD_LOG2,
        }
    }
    fn period(self) -> u64 {
        1 << self.period_log2()
    }
}

struct Counters {
    calls: AtomicU64,
    sampled: AtomicU64,
    detour: AtomicU64,
    original: AtomicU64,
    returns: AtomicU64,
    returns_sampled: AtomicU64,
    returns_detour: AtomicU64,
    overhead: AtomicU64,
    returns_overhead: AtomicU64,
}
impl Counters {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            sampled: AtomicU64::new(0),
            detour: AtomicU64::new(0),
            original: AtomicU64::new(0),
            returns: AtomicU64::new(0),
            returns_sampled: AtomicU64::new(0),
            returns_detour: AtomicU64::new(0),
            overhead: AtomicU64::new(0),
            returns_overhead: AtomicU64::new(0),
        }
    }
}

#[repr(align(64))]
struct Slot([Counters; FAMILY_COUNT]);
impl Slot {
    const fn new() -> Self {
        Self([const { Counters::new() }; FAMILY_COUNT])
    }
}

struct Table {
    slots: [Slot; SLOT_COUNT],
    claimed: AtomicUsize,
}
impl Table {
    const fn new() -> Self {
        Self {
            slots: [const { Slot::new() }; SLOT_COUNT],
            claimed: AtomicUsize::new(0),
        }
    }
    fn claim(&self) -> usize {
        self.claimed
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |claimed| {
                claimed.checked_add(1)
            })
            .map_or(SHARED_SLOT, |claimed| claimed.min(SHARED_SLOT))
    }
    #[inline(always)]
    fn enter(&'static self, slot: usize, family: Family, returning: bool) -> Span {
        if !returning {
            crate::capture_gate::pass(family as usize);
        }
        let phase = label();
        let counters = &self.slots[slot].0[family as usize];
        let shared = slot == SHARED_SLOT;
        let count = if returning {
            &counters.returns
        } else {
            &counters.calls
        };
        let previous = bump(count, shared, 1);
        let start = if previous & (family.period() - 1) == 0 || forced() {
            now().max(1)
        } else {
            0
        };
        Span {
            counters,
            shared,
            returning,
            start,
            original: Cell::new(0),
            originals: Cell::new(0),
            _phase: phase,
            _thread: PhantomData,
        }
    }
    fn totals(&self) -> [Totals; FAMILY_COUNT] {
        let mut totals = [Totals::default(); FAMILY_COUNT];
        let claimed = self.claimed.load(Ordering::Relaxed).min(OWNED_SLOTS);
        let slots = self.slots[..claimed]
            .iter()
            .chain(std::iter::once(&self.slots[SHARED_SLOT]));
        for slot in slots {
            for (total, counters) in totals.iter_mut().zip(&slot.0) {
                total.add(counters);
            }
        }
        totals
    }
}

static TABLE: Table = Table::new();
thread_local! {
    static SLOT: Cell<usize> = const { Cell::new(usize::MAX) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static SAMPLE_ALL: Cell<bool> = const { Cell::new(false) };
}
#[inline(always)]
fn forced() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    return SAMPLE_ALL.get();
    #[cfg(not(any(test, feature = "test-support")))]
    false
}
#[cfg(any(test, feature = "test-support"))]
pub fn sample_all() -> impl Drop {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            SAMPLE_ALL.set(false);
        }
    }
    SAMPLE_ALL.set(true);
    Restore
}
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug)]
pub struct Counts {
    pub calls: u64,
    pub sampled: u64,
    pub returns: u64,
    pub detour: u64,
    pub original: u64,
}
#[cfg(any(test, feature = "test-support"))]
pub fn counts(family: Family) -> Counts {
    let totals = TABLE.totals()[family as usize];
    Counts {
        calls: totals.calls,
        sampled: totals.sampled,
        returns: totals.returns,
        detour: totals.detour,
        original: totals.original,
    }
}

#[inline(always)]
fn bump(counter: &AtomicU64, shared: bool, amount: u64) -> u64 {
    if shared {
        counter.fetch_add(amount, Ordering::Relaxed)
    } else {
        let value = counter.load(Ordering::Relaxed);
        counter.store(value.wrapping_add(amount), Ordering::Relaxed);
        value
    }
}

#[inline(always)]
fn now() -> u64 {
    unsafe {
        _mm_lfence();
        _rdtsc()
    }
}

#[inline(always)]
fn current_slot() -> usize {
    let slot = SLOT.get();
    if slot != usize::MAX {
        return slot;
    }
    claim_current()
}
#[cold]
#[inline(never)]
fn claim_current() -> usize {
    let slot = TABLE.claim();
    SLOT.set(slot);
    slot
}

#[repr(align(64))]
struct CallSlot<const SITES: usize>([AtomicU64; SITES]);

pub struct CallCounters<const SITES: usize> {
    slots: [CallSlot<SITES>; SLOT_COUNT],
}
impl<const SITES: usize> Default for CallCounters<SITES> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const SITES: usize> CallCounters<SITES> {
    pub const fn new() -> Self {
        Self {
            slots: [const { CallSlot([const { AtomicU64::new(0) }; SITES]) }; SLOT_COUNT],
        }
    }
    #[inline(always)]
    pub fn increment(&self, site: usize) {
        self.increment_at(current_slot(), site);
    }
    #[inline(always)]
    fn increment_at(&self, slot: usize, site: usize) {
        bump(&self.slots[slot].0[site], slot == SHARED_SLOT, 1);
    }
    pub fn total(&self, site: usize) -> u64 {
        let claimed = TABLE.claimed.load(Ordering::Relaxed).min(OWNED_SLOTS);
        self.slots[..claimed]
            .iter()
            .chain(std::iter::once(&self.slots[SHARED_SLOT]))
            .fold(0, |total, slot| {
                total.wrapping_add(slot.0[site].load(Ordering::Relaxed))
            })
    }
}

#[inline(always)]
pub fn enter(family: Family) -> Span {
    TABLE.enter(current_slot(), family, false)
}
#[inline(always)]
pub fn enter_return(family: Family) -> Span {
    TABLE.enter(current_slot(), family, true)
}
#[inline(always)]
pub fn label() -> collector_memory::terminal::Scope {
    collector_memory::terminal::enter(collector_memory::terminal::Phase::Hook)
}

pub struct Span {
    counters: &'static Counters,
    shared: bool,
    returning: bool,
    start: u64,
    original: Cell<u64>,
    originals: Cell<u32>,
    _phase: collector_memory::terminal::Scope,
    _thread: PhantomData<*const ()>,
}
impl Span {
    #[inline(always)]
    pub fn original<T>(&self, original: impl FnOnce() -> T) -> T {
        if self.start == 0 {
            return original();
        }
        let begin = now();
        let result = original();
        let ticks = now().saturating_sub(begin);
        self.original.set(self.original.get().saturating_add(ticks));
        self.originals.set(self.originals.get().saturating_add(1));
        result
    }
    #[cold]
    #[inline(never)]
    fn record(&self) {
        let total = now().saturating_sub(self.start);
        let original = self.original.get().min(total);
        let detour = total - original;
        let overhead = overhead_ticks(self.originals.get()).min(detour);
        let counters = self.counters;
        if self.returning {
            bump(&counters.returns_sampled, self.shared, 1);
            bump(&counters.returns_detour, self.shared, detour);
            bump(&counters.returns_overhead, self.shared, overhead);
        } else {
            bump(&counters.sampled, self.shared, 1);
            bump(&counters.detour, self.shared, detour);
            bump(&counters.original, self.shared, original);
            bump(&counters.overhead, self.shared, overhead);
        }
    }
}
impl Drop for Span {
    #[inline(always)]
    fn drop(&mut self) {
        if self.start != 0 {
            self.record();
        }
    }
}

fn overhead_ticks(originals: u32) -> u64 {
    let [empty, per_original] = overhead_pair();
    empty.saturating_add(per_original.saturating_mul(u64::from(originals)))
}

fn overhead_pair() -> [u64; 2] {
    if !STARTED.is_completed() {
        return [0; 2];
    }
    OVERHEAD
        .each_ref()
        .map(|ticks| ticks.load(Ordering::Relaxed))
}

static OVERHEAD: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static STARTED: Once = Once::new();
const OVERHEAD_SAMPLES: usize = 512;

fn calibrate_overhead() {
    static CALIBRATION: Counters = Counters::new();
    let mut samples = [[0_u64; OVERHEAD_SAMPLES]; 2];
    for (case, samples) in samples.iter_mut().enumerate() {
        for sample in samples.iter_mut() {
            let before = CALIBRATION.detour.load(Ordering::Relaxed);
            let phase = label();
            let span = Span {
                counters: &CALIBRATION,
                shared: false,
                returning: false,
                start: now().max(1),
                original: Cell::new(0),
                originals: Cell::new(0),
                _phase: phase,
                _thread: PhantomData,
            };
            if case == 1 {
                span.original(|| std::hint::black_box(0_u64));
            }
            drop(span);
            *sample = CALIBRATION.detour.load(Ordering::Relaxed) - before;
        }
        samples.sort_unstable();
    }
    let empty = samples[0][0];
    OVERHEAD[0].store(empty, Ordering::Relaxed);
    OVERHEAD[1].store(samples[1][0].saturating_sub(empty), Ordering::Relaxed);
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Totals {
    calls: u64,
    sampled: u64,
    detour: u64,
    original: u64,
    returns: u64,
    returns_sampled: u64,
    overhead: u64,
    estimated_detour: u128,
    estimated_original: u128,
    estimated_overhead: u128,
}
impl Totals {
    fn add(&mut self, counters: &Counters) {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        let calls = load(&counters.calls);
        let sampled = load(&counters.sampled);
        let detour = load(&counters.detour);
        let original = load(&counters.original);
        let returns = load(&counters.returns);
        let returns_sampled = load(&counters.returns_sampled);
        let returns_detour = load(&counters.returns_detour);
        let overhead = load(&counters.overhead);
        let returns_overhead = load(&counters.returns_overhead);
        self.calls = self.calls.saturating_add(calls);
        self.sampled = self.sampled.saturating_add(sampled);
        self.detour = self
            .detour
            .saturating_add(detour)
            .saturating_add(returns_detour);
        self.original = self.original.saturating_add(original);
        self.returns = self.returns.saturating_add(returns);
        self.returns_sampled = self.returns_sampled.saturating_add(returns_sampled);
        self.overhead = self
            .overhead
            .saturating_add(overhead)
            .saturating_add(returns_overhead);
        self.estimated_overhead +=
            scale(overhead, calls, sampled) + scale(returns_overhead, returns, returns_sampled);
        self.estimated_detour +=
            scale(detour, calls, sampled) + scale(returns_detour, returns, returns_sampled);
        self.estimated_original += scale(original, calls, sampled);
    }
}

fn scale(ticks: u64, calls: u64, sampled: u64) -> u128 {
    if sampled == 0 {
        return 0;
    }
    u128::from(ticks) * u128::from(calls) / u128::from(sampled)
}

fn nanoseconds(ticks: u128, hz: Option<u64>) -> Option<u64> {
    let hz = u128::from(hz.filter(|hz| *hz != 0)?);
    Some(u64::try_from(ticks.saturating_mul(1_000_000_000) / hz).unwrap_or(u64::MAX))
}

static ORIGIN_TSC: AtomicU64 = AtomicU64::new(0);
static ORIGIN_QPC: AtomicI64 = AtomicI64::new(0);

pub fn start() {
    STARTED.call_once(|| {
        let (tsc, qpc) = (now(), crate::clock::qpc());
        ORIGIN_TSC.store(tsc, Ordering::Release);
        ORIGIN_QPC.store(qpc, Ordering::Release);
        calibrate_overhead();
    });
}

pub fn tsc_hz() -> Option<u64> {
    let mut frequency = 0;
    if unsafe { windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency) }
        == 0
    {
        return None;
    }
    let origin = (
        ORIGIN_TSC.load(Ordering::Acquire),
        ORIGIN_QPC.load(Ordering::Acquire),
    );
    calibrate(origin, (now(), crate::clock::qpc()), frequency)
}

fn calibrate(origin: (u64, i64), now: (u64, i64), qpc_frequency: i64) -> Option<u64> {
    let qpc_frequency = u64::try_from(qpc_frequency).ok().filter(|f| *f != 0)?;
    if origin.1 == 0 {
        return None;
    }
    let qpc = u64::try_from(now.1.checked_sub(origin.1)?).ok()?;
    if qpc < qpc_frequency / 100 {
        return None;
    }
    let tsc = now.0.checked_sub(origin.0)?;
    u64::try_from(u128::from(tsc) * u128::from(qpc_frequency) / u128::from(qpc)).ok()
}

pub fn ticks_to_ns(ticks: u64, hz: Option<u64>) -> Option<u64> {
    nanoseconds(u128::from(ticks), hz)
}

const SCOPE: &str = "cumulative since attach: calls are exact entries into collector callbacks per family (split families count their entry half; returns count the return half). Every period-th call per hooked thread and family is timed with the TSC, starting with that thread's first call. sampled_detour_ns is collector time inside the sampled callbacks, excluding the timed original; estimated_detour_ns scales each thread's sampled time by its calls per sampled call. sampled_original_ns and estimated_original_ns time the originals the collector invokes from Rust (detour and mixed families; null where native code or a stub runs the original). Tap bridges, entry stubs and the retour relay lie outside the timed span. A sampled span also contains the timer's own reads, which unsampled calls never make: approximate_timer_overhead_ticks estimates that cost (an empty span's, and each timed original's) at attach from the fastest of 512 empty spans through the same span path; approximate_timer_overhead_ns is its share of estimated_detour_ns and approximate_net_detour_ns the estimate without it. They are diagnostic approximations, not bounds: subtracting them may also remove some collector work, so estimated_detour_ns stays the conservative cost. Nested families inside an original are timed by their own entries; the hourly original contains every between-tick hook it reaches. Ticks convert with tsc_hz, calibrated against QPC since attach. capture_waits counts a family's calls that waited for a capture reading beside the game to end. A wait at a call's entry comes before its span, so it is in no family's detour or original time; only a portrait writer the capture's pre-image store could not hold waits inside its span. Their total is capture.writer_wait_ns";

fn family_status(
    family: Family,
    totals: Totals,
    capture_waits: u64,
    hz: Option<u64>,
) -> impl serde::Serialize {
    let timed = family.kind().times_original();
    let split = family.kind() == Kind::Split;
    status!({
        "kind": family.kind().name(),
        "sample_period": family.period(),
        "calls": totals.calls,
        "capture_waits": capture_waits,
        "sampled_calls": totals.sampled,
        "returns": split.then_some(totals.returns),
        "sampled_returns": split.then_some(totals.returns_sampled),
        "sampled_detour_ns": nanoseconds(u128::from(totals.detour), hz),
        "estimated_detour_ns": nanoseconds(totals.estimated_detour, hz),
        "approximate_timer_overhead_ns": nanoseconds(totals.estimated_overhead, hz),
        "approximate_net_detour_ns": nanoseconds(
            totals.estimated_detour.saturating_sub(totals.estimated_overhead),
            hz,
        ),
        "sampled_original_ns": if timed { nanoseconds(u128::from(totals.original), hz) } else { None },
        "estimated_original_ns": if timed { nanoseconds(totals.estimated_original, hz) } else { None },
    })
}

fn status_from(
    totals: [Totals; FAMILY_COUNT],
    capture_waits: [u64; FAMILY_COUNT],
    hz: Option<u64>,
    claimed: usize,
) -> impl serde::Serialize {
    status!({
        "sample_period": 1_u64 << DEFAULT_PERIOD_LOG2,
        "tsc_hz": hz,
        "approximate_timer_overhead_ticks": overhead_pair(),
        "owned_thread_slots": claimed.min(OWNED_SLOTS),
        "shared_slot_threads": claimed.saturating_sub(OWNED_SLOTS),
        "families": map(Family::ALL.into_iter().zip(totals).zip(capture_waits).map(
            move |((family, totals), capture_waits)| {
                (family.name(), family_status(family, totals, capture_waits, hz))
            },
        )),
        "scope": SCOPE,
    })
}

pub fn status() -> impl serde::Serialize {
    start();
    status_from(
        TABLE.totals(),
        std::array::from_fn(crate::capture_gate::waits),
        tsc_hz(),
        TABLE.claimed.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn table() -> &'static Table {
        Box::leak(Box::new(Table::new()))
    }

    fn spin(ticks: u64) {
        let begin = now();
        while now().saturating_sub(begin) < ticks {
            std::hint::spin_loop();
        }
    }

    #[test]
    fn families_follow_their_index_and_names_are_unique() {
        for (index, family) in Family::ALL.into_iter().enumerate() {
            assert_eq!(family as usize, index);
        }
        let mut names = Family::ALL.map(Family::name);
        names.sort_unstable();
        assert!(names.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn a_span_holds_the_hook_phase_and_restores_the_outer_one() {
        use collector_memory::terminal::{Phase, enter as phase_scope, phase};
        std::thread::spawn(|| {
            assert_eq!(phase(), Phase::Unphased);
            {
                let _span = enter(Family::PortraitChanges);
                assert_eq!(phase(), Phase::Hook);
                let _callback = phase_scope(Phase::Callback);
                let returning = enter_return(Family::EventFeed);
                assert_eq!(phase(), Phase::Hook);
                drop(returning);
                assert_eq!(phase(), Phase::Callback);
            }
            assert_eq!(phase(), Phase::Unphased);
            let _spool = phase_scope(Phase::Spool);
            let result = std::panic::catch_unwind(|| {
                let _label = label();
                assert_eq!(phase(), Phase::Hook);
                panic!("unwinds through the label");
            });
            assert!(result.is_err());
            assert_eq!(phase(), Phase::Spool);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn war_participation_registers_sampled_detours_with_original_accounting() {
        let family = Family::WarParticipation;
        assert_eq!(family.name(), "war_participation");
        assert_eq!(family.kind(), Kind::Detour);
        assert_eq!(family.period(), 64);
        let table = table();
        table.claim();
        for _ in 0..65 {
            let span = table.enter(0, family, false);
            span.original(|| std::hint::black_box(7));
        }
        let totals = table.totals()[family as usize];
        assert_eq!((totals.calls, totals.sampled), (65, 2));
        assert!(totals.original > 0);
        assert!(totals.detour > 0);
        let status: Value = serde_json::to_value(status()).unwrap();
        assert!(status["families"]["war_participation"].is_object());
    }

    #[test]
    fn calls_are_exact_and_one_in_period_is_timed_from_the_first() {
        let table = table();
        for _ in 0..1000 {
            drop(table.enter(0, Family::ExperienceGains, false));
        }
        assert_eq!(table.totals()[Family::ExperienceGains as usize].calls, 0);
        assert_eq!(table.claim(), 0);
        let totals = table.totals()[Family::ExperienceGains as usize];
        assert_eq!(totals.calls, 1000);
        assert_eq!(totals.sampled, 16);
        assert_eq!(totals.returns, 0);
        let other = table.totals()[Family::PortraitChanges as usize];
        assert_eq!((other.calls, other.sampled), (0, 0));
    }

    #[test]
    fn hourly_times_every_call() {
        let table = table();
        table.claim();
        for _ in 0..3 {
            drop(table.enter(0, Family::Hourly, false));
        }
        let totals = table.totals()[Family::Hourly as usize];
        assert_eq!((totals.calls, totals.sampled), (3, 3));
    }

    #[test]
    fn the_original_is_timed_apart_and_excluded_from_the_detour() {
        let table = table();
        table.claim();
        let span = table.enter(0, Family::PortraitChanges, false);
        spin(20_000);
        assert_eq!(
            span.original(|| {
                spin(200_000);
                7
            }),
            7
        );
        drop(span);
        let totals = table.totals()[Family::PortraitChanges as usize];
        assert_eq!((totals.calls, totals.sampled), (1, 1));
        assert!(totals.original >= 200_000, "{totals:?}");
        assert!(totals.detour >= 20_000, "{totals:?}");
        assert!(totals.detour < totals.original, "{totals:?}");
        let span = table.enter(0, Family::PortraitChanges, false);
        assert_eq!(span.original(|| 9), 9);
        drop(span);
        let after = table.totals()[Family::PortraitChanges as usize];
        assert_eq!((after.calls, after.sampled), (2, 1));
        assert_eq!(after.original, totals.original);
    }

    #[test]
    fn diagnostic_calls_keep_owned_and_shared_thread_totals_without_sampling() {
        let counter = CallCounters::<2>::new();
        std::thread::scope(|threads| {
            for _ in 0..4 {
                threads.spawn(|| {
                    for _ in 0..1000 {
                        counter.increment(0);
                        counter.increment_at(SHARED_SLOT, 1);
                    }
                });
            }
        });
        assert_eq!(counter.total(0), 4000);
        assert_eq!(counter.total(1), 4000);
        assert_eq!(
            counter.slots[SHARED_SLOT].0[1].load(Ordering::Relaxed),
            4000
        );
        assert_eq!(TABLE.totals()[Family::RaidChanges as usize].calls, 0);
    }

    #[test]
    fn diagnostic_calls_claim_thread_slots_without_allocation() {
        let counter = CallCounters::<1>::new();
        collector_test_support::with_allocations_denied(|| counter.increment(0));
        assert_eq!(counter.total(0), 1);
    }

    #[test]
    fn split_returns_count_no_call_and_add_their_time() {
        let table = table();
        table.claim();
        let entry = table.enter(0, Family::AirLosses, false);
        spin(10_000);
        drop(entry);
        let returned = table.enter(0, Family::AirLosses, true);
        spin(10_000);
        drop(returned);
        let totals = table.totals()[Family::AirLosses as usize];
        assert_eq!((totals.calls, totals.sampled), (1, 1));
        assert_eq!((totals.returns, totals.returns_sampled), (1, 1));
        assert!(totals.detour >= 20_000, "{totals:?}");
        assert_eq!(totals.original, 0);
        assert!(totals.estimated_detour >= 20_000);
    }

    #[test]
    fn estimates_scale_each_slot_by_its_own_sampling_ratio() {
        let table = table();
        table.claim();
        table.claim();
        let set = |slot: usize, calls: u64, sampled: u64, detour: u64, original: u64| {
            let counters = &table.slots[slot].0[Family::OpinionChanges as usize];
            counters.calls.store(calls, Ordering::Relaxed);
            counters.sampled.store(sampled, Ordering::Relaxed);
            counters.detour.store(detour, Ordering::Relaxed);
            counters.original.store(original, Ordering::Relaxed);
        };
        set(0, 640, 10, 1000, 4000);
        set(1, 3, 1, 50, 20);
        set(SHARED_SLOT, 128, 2, 30, 0);
        let totals = table.totals()[Family::OpinionChanges as usize];
        assert_eq!(totals.calls, 771);
        assert_eq!(totals.sampled, 13);
        assert_eq!(totals.detour, 1080);
        assert_eq!(totals.original, 4020);
        assert_eq!(totals.estimated_detour, 64_000 + 150 + 1920);
        assert_eq!(totals.estimated_original, 256_000 + 60);
        assert_eq!(scale(0, 1, 0), 0);
    }

    #[test]
    fn threads_beyond_the_owned_slots_share_one_atomic_slot() {
        let table = table();
        for expected in 0..OWNED_SLOTS {
            assert_eq!(table.claim(), expected);
        }
        assert_eq!(table.claim(), SHARED_SLOT);
        assert_eq!(table.claim(), SHARED_SLOT);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..10_000 {
                        drop(table.enter(SHARED_SLOT, Family::EventFeed, false));
                    }
                });
            }
        });
        let totals = table.totals()[Family::EventFeed as usize];
        assert_eq!(totals.calls, 40_000);
        assert_eq!(totals.sampled, 625);
        let status = serde_json::to_value(status_from(
            table.totals(),
            [0; FAMILY_COUNT],
            Some(1_000_000_000),
            table.claimed.load(Ordering::Relaxed),
        ))
        .unwrap();
        assert_eq!(status["owned_thread_slots"], OWNED_SLOTS);
        assert_eq!(status["shared_slot_threads"], 2);
    }

    #[test]
    fn sampled_spans_record_their_timer_overhead_and_estimates_net_it_out() {
        let callers = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let starts = [(); 2].map(|()| {
                scope.spawn(|| {
                    callers.wait();
                    start();
                    overhead_pair()
                })
            });
            for started in starts {
                assert_ne!(started.join().unwrap(), [0; 2]);
            }
        });
        assert!(STARTED.is_completed());
        let [empty, per_original] = OVERHEAD
            .each_ref()
            .map(|ticks| ticks.load(Ordering::Relaxed));
        assert!(empty > 0, "an empty span still reads the timer twice");
        let table = table();
        table.claim();
        let span = table.enter(0, Family::PortraitChanges, false);
        spin(50_000);
        span.original(|| spin(10_000));
        drop(span);
        let totals = table.totals()[Family::PortraitChanges as usize];
        assert_eq!(totals.overhead, empty + per_original);
        assert!(totals.detour >= 50_000 && totals.detour > totals.overhead);
        for _ in 0..63 {
            drop(table.enter(0, Family::PortraitChanges, false));
        }
        drop(table.enter(0, Family::PortraitChanges, false));
        let after = table.totals()[Family::PortraitChanges as usize];
        assert_eq!((after.calls, after.sampled), (65, 2));
        let empty_detour = after.detour - totals.detour;
        assert_eq!(after.overhead - totals.overhead, empty.min(empty_detour));
        assert_eq!(
            after.estimated_overhead,
            scale(after.overhead, after.calls, after.sampled)
        );
        assert!(after.estimated_overhead <= after.estimated_detour);
    }

    #[test]
    fn calibration_needs_a_qpc_interval_and_scales_ticks() {
        assert_eq!(calibrate((0, 0), (5, 100), 10_000_000), None);
        assert_eq!(calibrate((1000, 1), (1000, 99_000), 10_000_000), None);
        assert_eq!(
            calibrate((1000, 1), (30_001_000, 100_001), 10_000_000),
            Some(3_000_000_000)
        );
        assert_eq!(calibrate((5, 1), (4, 200_001), 10_000_000), None);
        assert_eq!(nanoseconds(3_000, Some(3_000_000_000)), Some(1_000));
        assert_eq!(nanoseconds(3_000, None), None);
        assert_eq!(ticks_to_ns(6_000, Some(3_000_000_000)), Some(2_000));
    }

    #[test]
    fn status_publishes_every_family_with_its_timing_fields() {
        let mut totals = [Totals::default(); FAMILY_COUNT];
        totals[Family::ExperienceGains as usize] = Totals {
            calls: 6400,
            sampled: 100,
            detour: 3_000_000,
            overhead: 300_000,
            estimated_detour: 192_000_000,
            estimated_overhead: 19_200_000,
            ..Totals::default()
        };
        totals[Family::PortraitChanges as usize] = Totals {
            calls: 2,
            sampled: 1,
            detour: 300,
            original: 3000,
            estimated_detour: 600,
            estimated_original: 6000,
            ..Totals::default()
        };
        totals[Family::EventFeed as usize] = Totals {
            calls: 1,
            sampled: 1,
            returns: 1,
            returns_sampled: 1,
            detour: 30,
            estimated_detour: 30,
            ..Totals::default()
        };
        let mut capture_waits = [0; FAMILY_COUNT];
        capture_waits[Family::ExperienceGains as usize] = 5;
        let status: Value =
            serde_json::to_value(status_from(totals, capture_waits, Some(3_000_000_000), 3))
                .unwrap();
        assert_eq!(status["sample_period"], 64);
        assert_eq!(status["tsc_hz"], 3_000_000_000_u64);
        assert_eq!(status["owned_thread_slots"], 3);
        assert_eq!(status["shared_slot_threads"], 0);
        let families = status["families"].as_object().unwrap();
        assert_eq!(families.len(), FAMILY_COUNT);
        assert_eq!(
            status["families"]["experience_gains"],
            json!({
                "kind": "tap", "sample_period": 64, "calls": 6400, "capture_waits": 5,
                "sampled_calls": 100,
                "returns": null, "sampled_returns": null,
                "sampled_detour_ns": 1_000_000, "estimated_detour_ns": 64_000_000,
                "approximate_timer_overhead_ns": 6_400_000, "approximate_net_detour_ns": 57_600_000,
                "sampled_original_ns": null, "estimated_original_ns": null,
            })
        );
        assert_eq!(
            status["families"]["portrait_changes"],
            json!({
                "kind": "detour", "sample_period": 64, "calls": 2, "capture_waits": 0,
                "sampled_calls": 1,
                "returns": null, "sampled_returns": null,
                "sampled_detour_ns": 100, "estimated_detour_ns": 200,
                "approximate_timer_overhead_ns": 0, "approximate_net_detour_ns": 200,
                "sampled_original_ns": 1000, "estimated_original_ns": 2000,
            })
        );
        assert_eq!(status["families"]["event_feed"]["kind"], "split");
        assert_eq!(status["families"]["event_feed"]["returns"], 1);
        assert_eq!(status["families"]["event_feed"]["sampled_returns"], 1);
        assert_eq!(status["families"]["hourly"]["sample_period"], 1);
        assert_eq!(status["families"]["land_events"]["kind"], "mixed");
        assert_eq!(status["families"]["land_events"]["sampled_original_ns"], 0);
        let uncalibrated: Value =
            serde_json::to_value(status_from(totals, capture_waits, None, 0)).unwrap();
        assert_eq!(uncalibrated["tsc_hz"], Value::Null);
        assert_eq!(
            uncalibrated["families"]["portrait_changes"]["estimated_detour_ns"],
            Value::Null
        );
    }

    #[test]
    #[ignore = "release per-call span cost; run with --release --ignored --nocapture"]
    fn span_cost_profile() {
        #[inline(never)]
        fn native(value: u64) -> u64 {
            std::hint::black_box(value).wrapping_mul(3)
        }
        const CALLS: u64 = 20_000_000;
        let time = |run: &dyn Fn()| {
            let mut best = u128::MAX;
            for _ in 0..5 {
                let started = std::time::Instant::now();
                run();
                best = best.min(started.elapsed().as_nanos());
            }
            best as f64 / CALLS as f64
        };
        let plain = time(&|| {
            for value in 0..CALLS {
                std::hint::black_box(native(value));
            }
        });
        let spans = time(&|| {
            for value in 0..CALLS {
                let span = enter(Family::ExperienceGains);
                std::hint::black_box(span.original(|| native(value)));
            }
        });
        let _sampled = sample_all();
        let every = time(&|| {
            for value in 0..CALLS {
                let span = enter(Family::ExperienceGains);
                std::hint::black_box(span.original(|| native(value)));
            }
        });
        println!(
            "hook_timing per_call_ns plain={plain:.2} period_64={spans:.2} every_call={every:.2} added_period_64={:.2} added_sampled_call={:.2}",
            spans - plain,
            every - plain
        );
    }

    #[test]
    fn live_status_reports_spans_entered_on_this_thread() {
        let before: Value = serde_json::to_value(status()).unwrap();
        let calls = |status: &Value| status["families"]["combat_log_prune"]["calls"].as_u64();
        drop(enter(Family::CombatLogPrune));
        let after: Value = serde_json::to_value(status()).unwrap();
        assert!(calls(&after).unwrap() > calls(&before).unwrap());
        assert!(after["owned_thread_slots"].as_u64().unwrap() >= 1);
    }
}
