//! Exact comparison of complete hourly tables with the encoder's baselines,
//! after conversion and merging and before encoding.
//!
//! On its full path the encoder validates a complete hourly table, compares
//! it with the baseline, encodes the differing rows and drops the rows it does
//! not keep, all on the spool thread. This stage compares first, on the spool
//! thread and its conversion helpers together, each taking the next listed
//! table:
//!
//! - every row equal to its baseline position: `ComparedEqual`;
//! - every key at its baseline position and at most three quarters of the
//!   rows differing: `ComparedChanges` holding only the differing rows;
//! - anything else (a membership change, denser changes, a baseline that
//!   refuses row updates, an allocation failure): the complete table.
//!
//! The encoder writes the same bytes for each form
//! ([`replay_frame::frame::FrameEncoder::prepare_bytes`]): a row equal to its
//! baseline writes nothing on the full path, and a replacement writes exactly
//! the change the full path's merge writes for that row. Its baseline then
//! holds the current rows, as on the full path; only a differing row whose
//! recorded fields are all equal (no codec writes such a row) could leave the
//! two baselines apart, and only in what no frame records, so later frames
//! stay equal too. The spool is therefore unchanged. Only complete tables with
//! a baseline for exactly this hour are compared, so keyframes, new branches,
//! gaps and restarted spools keep the full path; the encoder checks every
//! token again. Unchanged converted rows are dropped by the thread that
//! compared them.
//!
//! Replacing most rows costs the encoder more than its full path, so a table
//! found densely changed is left complete for the next [`DENSE_RETRY_HOURS`]
//! hours before it is compared again.

use crate::spool_phases::{HELPER, Phase, SPOOL, thread_cycles};
use crate::status::{sequence, status};
use replay_data::{
    BranchId, Hour, rows::RowCodec, rows::TableId, rows::TableRows, rows::TableSchema, rows::tables,
};
use replay_frame::{frame::FrameEncoder, frame::FrameInput, frame::TableInput, frame::TableView};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering},
};

/// The frame the encoder writes next: its branch and hour, and whether it
/// restarts the spool, which makes every table a snapshot.
#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub(crate) branch: BranchId,
    pub(crate) hour: Hour,
    pub(crate) restarting: bool,
}

/// Hours a densely changed table stays complete before it is compared again.
pub(crate) const DENSE_RETRY_HOURS: i64 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Not a complete sampled table.
    Skipped,
    /// No baseline for this hour.
    NoBaseline,
    Equal,
    Changes(usize),
    /// Compared but left complete: a membership change, a refused row
    /// update or an allocation failure.
    Kept,
    /// More than three quarters of the rows differ; left complete.
    Dense,
    /// Densely changed within the last `DENSE_RETRY_HOURS`; not compared.
    Deferred,
}

type Compare = fn(&FrameEncoder, Target, &mut TableInput, &mut Vec<usize>, &AtomicI64) -> Outcome;

macro_rules! compared_tables {
    ($($table:ident),+ $(,)?) => {
        /// Every recorded hourly table but Clock, whose row depends on the enclosing
        /// hour, and MapEvents, whose one row is the interval ending at the
        /// frame's hour and so never equals the preceding hour's baseline,
        /// roughly in descending full-path cost so the threads finish
        /// together. Other cadences reach the encoder as shared or carried rows.
        pub(crate) const TABLES: [TableId; [$(stringify!($table)),+].len()] = [$(TableId::$table),+];
        const COMPARE: [Compare; TABLES.len()] = [$(compare::<tables::$table>),+];
    };
}

compared_tables!(
    // Owner-approved daily slot-12 training progress uses the ordinary carried-table path.
    VariantChanges,
    VariantBonusSnapshots,
    DesignCountryModifiers,
    DesignCountryTemplates,
    DesignObserverContext,
    DesignOrganisationLiveness,
    DivisionHistory,
    DivisionPanelSources,
    DivisionPanelStats,
    CombatLogGroups,
    CombatLogMonths,
    CombatLogManpower,
    CombatLogTemplates,
    CombatLogReferences,
    CombatLogBattles,
    CombatLogEnemyEquipment,
    CombatLogBattleSides,
    CombatLogBattleEquipment,
    CombatLogBattleModifiers,
    CombatLogBattleCountries,
    CombatLogObservers,
    NavalRepairCountries,
    NavalRepairFleets,
    NavalRepairTaskForces,
    NavalTaskForceComposition,
    NavalRepairBases,
    NavalRepairQueue,
    NavalRefitShips,
    NavalRefitProgress,
    NavalRepairChildren,
    CountryMarket,
    MarketContractTerms,
    LendLeaseAgreements,
    MarketEvents,
    WarParticipation,
    WarCapturedProvinces,
    WarLendLeaseHistories,
    DivisionStatus,
    DivisionModelInputs,
    CountryCounterInputs,
    ScriptVariables,
    AdvisorRoles,
    DivisionMovement,
    CharacterPortraits,
    Characters,
    WeatherProvinces,
    Provinces,
    ProvinceUnitOrder,
    ShipModels,
    NavalCombatShips,
    LandCombatDetails,
    RailCooldowns,
    UnitLeaderTraits,
    States,
    StateResistance,
    Fleets,
    ArmyOrderPlans,
    CharacterRoleTraits,
    LandCombatDivisions,
    DivisionTemplateIcons,
    CountryTemplateState,
    DivisionTemplates,
    CountryLocalSubunits,
    DivisionRetreatBans,
    OrdersGroupCounterInputs,
    CountryAppearance,
    AdvisorTokens,
    DivisionLandInputs,
    ActiveAbilities,
    ReservedCountryNaming,
    Factions,
    FactionGoalStatus,
    FactionGoalEntries,
    FactionRules,
    FactionPrograms,
    FactionManpower,
    FactionUnlocks,
    FactionAccounts,
    FactionResearch,
    FactionMinimumUpgrades,
    FactionDoctrine,
    FactionProjection,
    FactionResources,
    FactionVariables,
    FactionAdvisorModifiers,
    DoctrineFolders,
    DoctrineTracks,
    DoctrineLeaderMastery,
    DoctrineCostReductions,
    DoctrineMasteryGrants,
    DoctrineEquipmentBonuses,
    DoctrineCountries,
    WarRelations,
    LandCombatModifiers,
    LandCombatDivisionModifiers,
    ReserveModifierTrees,
    AirBaseRegistrations,
    ProvinceSupply,
    RelationLinks,
    UnitLeaders,
    NavalCombatDetails,
    WarOverviewCountries,
    CountryWargoals,
    WargoalTargets,
    WargoalJustifications,
    Divisions,
    ArmyOrders,
    TaskForceMissions,
    ActiveIdeas,
    DynamicModifiers,
    AirWingGameScoreStats,
    ProvinceBuildings,
    CountryGameScoreInputs,
    SliceReads,
    AirWings,
    CountryTacticUnlocks,
    PoliticalParties,
    CountryResources,
    Ships,
    TaskForceCounterInputs,
    WingEquipment,
    StateBuildings,
    OrdersGroupPlans,
    CountryLinks,
    GlobalSavedEventTargets,
    Fronts,
    EquipmentDestruction,
    Countries,
    AirMissionInputs,
    AirDailyRoster,
    NavalCombatResultDetails,
    OrdersGroups,
    TaskForceMapMotion,
    LandCombatProgress,
    RaidTargetStatuses,
    Raids,
    AirSortieSpecialBases,
    CountryFactories,
    CharacterOperatives,
    PoliticsCaptureCoverage,
    CountryCharacters,
    NavalMissionLedgers,
    OrdersGroupMembers,
    DivisionEquipmentDeletions,
    BuildingBombingDamage,
    NuclearStrikes,
    RaidPlayerAllowedTypes,
    CountryExperienceGains,
    WarOwnedRelations,
    LandCombats,
    PowerBalanceOverrides,
    NavalResults,
    StationedUnits,
    TheatreAreas,
    HiredAdvisors,
    LandCombatSides,
    AirNavalPatrolCombats,
    RaidTooltipOperands,
    Autonomy,
    RaidCountries,
    ArmyIconInputs,
    CountryWars,
    AirReferenceWitnesses,
    AirGroundCrews,
    AirTransferInputs,
    RaidAirScores,
    FleetCounterInputs,
    AirBases,
    RailEdges,
    RadarCoverage,
    NavalBases,
    RailwayGunCounterInputs,
    OwnerTransportSpeeds,
    MovementStateTerms,
    CoreSupplyAccess,
    CountryUnitSpeedEntries,
    RiverRelations,
    DivisionTransportSpeeds,
    CountryCommandPower,
    DivisionCensus,
    PowerBalanceMembers,
    IntelligenceAgencies,
    OperativeMissions,
    StrategicOperatives,
    AgencyOperationBuffers,
    PropagandaEfforts,
    IntelContributions,
    IntelSources,
    IntelNetworkAuxiliaries,
    ResourceOrigins,
    ResourceRights,
    CountryOilFactors,
    StateTemporaryResources,
    ConvoyClients,
    RegionalConvoyEfficiency,
    Operations,
    OperationAssignments,
    SimulationFeatures,
    Politics,
    StateFlags,
    NavalMissionTaskForces,
    StateVisibility,
    FactionTheatres,
    InvasionPreparations,
    PartyLeaders,
    LandCombatResults,
    TaskForces,
    ResourceExchanges,
    CountryLeaderRoles,
    ScriptFlags,
    WeatherRegions,
    LeaderPortraits,
    AirSortieBaseCountries,
    CountryPowerChanges,
    PeaceBids,
    Campaign,
    FactionMembers,
    Players,
    HumanControllers,
    NavalCombats,
    NavalCombatParticipants,
    PowerBalances,
    PowerBalanceRanges,
    Capitulation,
    Subjects,
    Cryptology,
    LandTheatres,
    NavalResultMembers,
    NavalResultCountries,
    Nukes,
    VolunteerHosts,
    UnitIntel,
    ArmyGroupMembers,
    Scientists,
    ScientistSkills,
    LeaderPortraitPaths,
    RaidTargets,
    CountryVisibility,
    NewDivisionEquipment,
    SentVolunteers,
    RaidCooldowns,
    RaidPathDistances,
    BorderConflicts,
    RaidSpeedInputs,
    RaidEquipment,
    CountryCivilWars,
    FactionAdvisors,
    AirRegionalMissions,
    AirRegionCombat,
    NavalRegionalMines,
    NavalDominanceTaskForceRanges,
    NavalPatrolMissionInputs,
    NavalAccidentReports,
    AirSortieModifiers,
    LocalPlayer,
    RaidFacilities,
    RaidIntelInputs,
    FloatingHarborDeployments,
    FacilityMapFlags,
    BattleSounds,
    CountryRetiredCharacters,
    NavalProvinceCombats,
    NavalCombatTransfers,
    NavalTransferUnits,
    NavalCombatAirBaseWingHandles,
    NavalModifierTrees,
    NavalModifierTreeSamples,
    PowerBalanceModifierBreakdowns,
    CombatLogState,
    TimedIdeas,
    GameScoreMultipliers,
    AgencyUpgradeLevels,
    OperationCompletions,
    GovernmentChanges,
    WarRelationGoals,
    WarNameRules,
    PeaceSettlements,
    DestroyedDivisions,
    LandEventCoverage,
    NavalResultSources,
    AirLossEvents,
    EventFeedHistory,
    ProvinceNames,
    StockpileArchetypes,
    RaidCountryInputs,
    AceRangeInputs,
    ResourceDefinitions,
    NavalRegionAccess,
    ThreatSources,
    ThreatCountryAggregates,
    WorldTension,
);

/// Cumulative per-table decisions and the comparing threads' cycles, and the
/// hour each table was last found densely changed.
pub(crate) struct Counters {
    tables: [[AtomicU64; 8]; TABLES.len()],
    dense_at: [AtomicI64; TABLES.len()],
}

const COMPARED: usize = 0;
const EQUAL: usize = 1;
const CHANGES: usize = 2;
const CHANGED_ROWS: usize = 3;
const KEPT: usize = 4;
const DENSE: usize = 5;
const DEFERRED: usize = 6;
const CYCLES: usize = 7;

impl Default for Counters {
    fn default() -> Self {
        Self {
            tables: [const { [const { AtomicU64::new(0) }; 8] }; TABLES.len()],
            dense_at: [const { AtomicI64::new(i64::MIN) }; TABLES.len()],
        }
    }
}

impl Counters {
    fn record(&self, index: usize, outcome: Outcome, cycles: u64) {
        let slot = &self.tables[index];
        let add = |counter: usize, value: u64| {
            slot[counter].fetch_add(value, Ordering::Relaxed);
        };
        match outcome {
            Outcome::Skipped | Outcome::NoBaseline => return,
            Outcome::Deferred => return add(DEFERRED, 1),
            Outcome::Equal => add(EQUAL, 1),
            Outcome::Changes(rows) => {
                add(CHANGES, 1);
                add(CHANGED_ROWS, rows as u64);
            }
            Outcome::Kept => add(KEPT, 1),
            Outcome::Dense => add(DENSE, 1),
        }
        add(COMPARED, 1);
        add(CYCLES, cycles);
    }

    fn load(&self, index: usize, counter: usize) -> u64 {
        self.tables[index][counter].load(Ordering::Relaxed)
    }

    /// (equal, changes, kept, dense, deferred) decisions of one listed table.
    #[cfg(test)]
    pub(crate) fn decisions(&self, table: TableId) -> [u64; 5] {
        let index = TABLES.iter().position(|listed| *listed == table).unwrap();
        [EQUAL, CHANGES, KEPT, DENSE, DEFERRED].map(|counter| self.load(index, counter))
    }

    /// (equal, changes, kept) decisions over every listed table.
    #[cfg(test)]
    pub(crate) fn total_decisions(&self) -> (u64, u64, u64) {
        let total = |counter| {
            (0..TABLES.len())
                .map(|index| self.load(index, counter))
                .sum()
        };
        (total(EQUAL), total(CHANGES), total(KEPT))
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        let hz = crate::hook_timing::tsc_hz();
        let total = |counter| {
            (0..TABLES.len())
                .map(|index| self.load(index, counter))
                .sum::<u64>()
        };
        status!({
            "scope": "cumulative decisions of the comparison before encoding: complete hourly tables with a baseline for exactly their hour, compared on the spool thread and its conversion helpers; equal become ComparedEqual, changes carry only the differing rows, kept stay complete (membership change, a refused row update or a failure), dense stay complete with more than three quarters of their rows differing, and deferred are such tables left uncompared for dense_retry_hours; cycles are the comparing threads' own",
            "dense_retry_hours": DENSE_RETRY_HOURS,
            "compared": total(COMPARED),
            "equal": total(EQUAL),
            "changes": total(CHANGES),
            "changed_rows": total(CHANGED_ROWS),
            "kept": total(KEPT),
            "dense": total(DENSE),
            "deferred": total(DEFERRED),
            "cycle_ns": crate::hook_timing::ticks_to_ns(total(CYCLES), hz),
            "tables": sequence((0..TABLES.len()).filter(move |&index| self.load(index, COMPARED) != 0).map(move |index| status!({
                "table": crate::status::DebugValue(TABLES[index]),
                "compared": self.load(index, COMPARED),
                "equal": self.load(index, EQUAL),
                "changes": self.load(index, CHANGES),
                "changed_rows": self.load(index, CHANGED_ROWS),
                "kept": self.load(index, KEPT),
                "dense": self.load(index, DENSE),
                "deferred": self.load(index, DEFERRED),
                "cycle_ns": crate::hook_timing::ticks_to_ns(self.load(index, CYCLES), hz),
            }))),
        })
    }
}

fn vacant() -> TableInput {
    // An empty reason allocates nothing; every slot is refilled before encoding.
    TableInput::Sampled(TableView::Unavailable {
        reason: String::new(),
    })
}

/// Compare the listed tables of `source` with `encoder`'s baselines, on this
/// thread and, if present, the helper.
pub(crate) fn compare_frame(
    encoder: &FrameEncoder,
    target: Target,
    source: &mut FrameInput,
    helpers: Option<crate::spool_helper::Helpers<'_>>,
    counters: &Counters,
) {
    if target.restarting {
        return;
    }
    let slots: [Mutex<TableInput>; TABLES.len()] = std::array::from_fn(|index| {
        Mutex::new(std::mem::replace(
            &mut source.tables[TABLES[index].index()],
            vacant(),
        ))
    });
    let next = AtomicUsize::new(0);
    let work = || {
        let mut indices = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= TABLES.len() || crate::allocation::failed() {
                break;
            }
            let mut input = slots[index].lock().unwrap_or_else(PoisonError::into_inner);
            let cycles = thread_cycles();
            let outcome = COMPARE[index](
                encoder,
                target,
                &mut input,
                &mut indices,
                &counters.dense_at[index],
            );
            drop(input);
            counters.record(index, outcome, thread_cycles().saturating_sub(cycles));
        }
    };
    let span = SPOOL.span(Phase::Compare);
    match helpers {
        Some(helpers) => {
            let context = crate::allocation::context();
            let job = || {
                let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
                crate::allocation::set_context(context);
                let _span = HELPER.span(Phase::Compare);
                work();
            };
            helpers
                .join_each(&job, || {
                    work();
                    span.switch(Phase::HelperWait)
                })
                .finish();
        }
        None => {
            work();
            span.finish();
        }
    }
    for (index, slot) in slots.into_iter().enumerate() {
        source.tables[TABLES[index].index()] =
            slot.into_inner().unwrap_or_else(PoisonError::into_inner);
    }
}

fn compare<T: TableSchema>(
    encoder: &FrameEncoder,
    target: Target,
    input: &mut TableInput,
    indices: &mut Vec<usize>,
    dense_at: &AtomicI64,
) -> Outcome {
    let TableInput::Sampled(TableView::Complete(rows)) = &*input else {
        return Outcome::Skipped;
    };
    // A mistyped table reaches the encoder unchanged, which refuses it.
    let Some(current) = rows.rows::<T>() else {
        return Outcome::Skipped;
    };
    let Some((token, baseline)) =
        encoder.comparison_baseline::<T>(target.branch, target.hour, target.restarting)
    else {
        return Outcome::NoBaseline;
    };
    let since_dense = i64::from(target.hour).saturating_sub(dense_at.load(Ordering::Relaxed));
    if (0..DENSE_RETRY_HOURS).contains(&since_dense) {
        return Outcome::Deferred;
    }
    if current.len() != baseline.len() {
        return Outcome::Kept;
    }
    let dense = current.len() / 4 * 3 + current.len() % 4 * 3 / 4;
    indices.clear();
    for (index, (row, previous)) in current.iter().zip(baseline).enumerate() {
        // Equal rows have equal keys; a differing row must keep its key.
        if row == previous {
            continue;
        }
        if row.key() != previous.key() {
            return Outcome::Kept;
        }
        if indices.len() == dense {
            dense_at.store(i64::from(target.hour), Ordering::Relaxed);
            return Outcome::Dense;
        }
        if engine_allocation::reserve(indices, 1).is_err() {
            return Outcome::Kept;
        }
        indices.push(index);
    }
    if indices.is_empty() {
        // The full path writes Unchanged and keeps the baseline rows.
        *input = TableInput::ComparedEqual(token);
        return Outcome::Equal;
    }
    if !token.allows_row_updates() {
        return Outcome::Kept;
    }
    let (Ok(mut owned), Ok(mut changed)) = (
        engine_allocation::vec::<usize>(indices.len()),
        engine_allocation::vec::<T::Row>(indices.len()),
    ) else {
        return Outcome::Kept;
    };
    owned.extend_from_slice(indices);
    let TableInput::Sampled(TableView::Complete(rows)) = std::mem::replace(input, vacant()) else {
        unreachable!("checked complete above");
    };
    let Ok(all) = T::take(rows) else {
        unreachable!("typed above");
    };
    let mut wanted = owned.iter().copied().peekable();
    for (index, row) in all.into_iter().enumerate() {
        if wanted.peek() == Some(&index) {
            wanted.next();
            changed.push(row);
        }
    }
    let count = changed.len();
    *input = TableInput::ComparedChanges {
        token,
        indices: owned,
        rows: TableRows::from_rows::<T>(changed),
    };
    Outcome::Changes(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TABLE_COUNT;
    use replay_data::{cadence::Cadence, rows::StateRow};
    use replay_frame::{frame::FrameDecoder, frame::Metrics};

    #[test]
    fn every_hourly_table_but_the_clock_and_map_events_is_compared_once() {
        let mut seen = [false; TABLE_COUNT];
        for table in TABLES {
            assert!(!seen[table.index()], "{table:?} listed twice");
            seen[table.index()] = true;
        }
        // Retired tables are absent from current frames and never compared.
        // Clock and MapEvents rows name the frame's own hour (MapEvents'
        // interval ends at it), so neither can equal the preceding hour's
        // baseline; comparing MapEvents would only ever find it dense.
        let current = replay_data::format::FormatVersion::CURRENT.version();
        let misplaced: Vec<(TableId, bool)> = TableId::ALL
            .into_iter()
            .filter_map(|table| {
                let hourly = replay_data::cadence::cadence(table) == Cadence::Hourly;
                let names_its_hour = matches!(table, TableId::Clock | TableId::MapEvents);
                let expected = hourly && !names_its_hour && table.last_change().present(current);
                (seen[table.index()] != expected).then_some((table, expected))
            })
            .collect();
        // Each entry is a table and whether it should be listed.
        assert_eq!(misplaced, []);
    }

    fn state(id: u32, owner: u32) -> StateRow {
        StateRow {
            id,
            owner,
            controller: owner,
            ..Default::default()
        }
    }

    fn frame(hour: Hour, states: Vec<StateRow>) -> FrameInput {
        let mut frame = FrameInput {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture table".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        frame.tables[TableId::ResourceOrigins.index()] =
            TableView::Complete(TableRows::ResourceOrigins(
                states
                    .iter()
                    .map(|state| replay_data::rows::ResourceOriginRow {
                        state: state.id,
                        country: state.owner,
                        efficiency: i64::from(state.controller),
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        // Sliced routes bypass the hourly comparison catalog, retaining every new column
        // through the encoder's ordinary baseline beside the compared hourly tables.
        frame.tables[TableId::TradeRoutes.index()] = TableView::Complete(TableRows::TradeRoutes(
            states
                .iter()
                .map(|state| replay_data::rows::TradeRouteRow {
                    country: state.id,
                    class: 1,
                    other: 1,
                    kind: 2,
                    source_state: Some(state.owner),
                    destination_state: Some(state.controller),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(states)).into();
        frame
    }

    /// Each hour's frame encodes to the bytes of the uncompared frame, and
    /// only the differing rows reach the encoder.
    #[test]
    fn compared_frames_encode_the_bytes_of_complete_frames() {
        let first = crate::spool_helper::SpoolHelper::start(None).unwrap();
        let second = crate::spool_helper::SpoolHelper::start(None).unwrap();
        for helpers in [0, 1, 2] {
            let counters = Counters::default();
            let mut compared = FrameEncoder::new();
            let mut ordinary = FrameEncoder::new();
            let mut decoder = FrameDecoder::new();
            let mut rows = vec![state(1, 10), state(2, 20), state(3, 30)];
            for hour in 0..11 {
                match hour {
                    2 => rows[1].owner = 21,
                    3 => {
                        rows[0].controller = 11;
                        rows[2].manpower_available = 5;
                    }
                    5 => rows.push(state(4, 40)),
                    6 => {
                        rows.remove(0);
                    }
                    7 => rows.iter_mut().for_each(|row| row.owner += 1),
                    9 => rows[0].owner += 1,
                    _ => {}
                }
                let mut input = frame(hour, rows.clone());
                compare_frame(
                    &compared,
                    Target {
                        branch: 1,
                        hour,
                        restarting: hour == 10,
                    },
                    &mut input,
                    (helpers != 0).then_some(crate::spool_helper::Helpers {
                        first: &first,
                        second: (helpers == 2).then_some(&second),
                        backlogged: hour % 2 == 0,
                    }),
                    &counters,
                );
                let states = &input.tables[TableId::States.index()];
                match hour {
                    // First hour, membership changes, dense changes, the hours
                    // after them and a restart keep the full path.
                    0 | 5..=10 => assert!(
                        matches!(states, TableInput::Sampled(TableView::Complete(_))),
                        "{hour}"
                    ),
                    1 | 4 => assert!(matches!(states, TableInput::ComparedEqual(_)), "{hour}"),
                    2 | 3 => {
                        let TableInput::ComparedChanges { indices, rows, .. } = states else {
                            panic!("{hour}: {states:?}");
                        };
                        assert_eq!(indices, &if hour == 2 { vec![1] } else { vec![0, 2] });
                        assert_eq!(rows.len(), indices.len());
                    }
                    _ => unreachable!(),
                }
                let actual = compared.encode_input(input, hour == 10).unwrap();
                let expected = ordinary
                    .encode_input(frame(hour, rows.clone()), hour == 10)
                    .unwrap();
                assert_eq!(actual, expected, "hour {hour}");
                let decoded = decoder.apply(&actual).unwrap();
                assert_eq!(
                    decoded.complete::<tables::States>().unwrap(),
                    rows.as_slice()
                );
                let routes = decoded.complete::<tables::TradeRoutes>().unwrap();
                assert_eq!(
                    routes
                        .iter()
                        .map(|route| (route.source_state, route.destination_state))
                        .collect::<Vec<_>>(),
                    rows.iter()
                        .map(|state| (Some(state.owner), Some(state.controller)))
                        .collect::<Vec<_>>()
                );
            }
            // Hours 1 and 4 equal, 2 and 3 changed, 5 and 6 kept for their
            // membership, 7 dense (every row differs) and 8 and 9 deferred.
            assert_eq!(counters.decisions(TableId::States), [2, 2, 2, 1, 2]);
        }
    }

    #[test]
    fn shared_compared_and_failed_inputs_are_left_to_the_encoder() {
        let counters = Counters::default();
        let mut encoder = FrameEncoder::new();
        let rows = vec![state(1, 10)];
        encoder.encode_input(frame(0, rows.clone()), false).unwrap();
        let shared =
            TableInput::SharedComplete(std::sync::Arc::new(TableRows::States(rows.clone())));
        let truncated = TableInput::Sampled(TableView::Truncated {
            rows: TableRows::States(rows.clone()),
            missing: 1,
        });
        for (index, other) in [shared, truncated].into_iter().enumerate() {
            let mut input = frame(1, rows.clone());
            input.tables[TableId::States.index()] = other.clone();
            compare_frame(
                &encoder,
                Target {
                    branch: 1,
                    hour: 1,
                    restarting: false,
                },
                &mut input,
                None,
                &counters,
            );
            assert_eq!(input.tables[TableId::States.index()], other, "{index}");
        }
        // Another branch has no baseline for this hour.
        let mut input = frame(1, rows.clone());
        compare_frame(
            &encoder,
            Target {
                branch: 2,
                hour: 1,
                restarting: false,
            },
            &mut input,
            None,
            &counters,
        );
        assert!(matches!(
            input.tables[TableId::States.index()],
            TableInput::Sampled(TableView::Complete(_))
        ));
        assert_eq!(counters.decisions(TableId::States), [0; 5]);
    }

    #[test]
    fn comparison_without_changes_allocates_nothing() {
        let counters = Counters::default();
        let mut encoder = FrameEncoder::new();
        let rows = vec![state(1, 10), state(2, 20)];
        encoder.encode_input(frame(0, rows.clone()), false).unwrap();
        let mut input = frame(1, rows);
        crate::test_support::with_allocations_denied(|| {
            compare_frame(
                &encoder,
                Target {
                    branch: 1,
                    hour: 1,
                    restarting: false,
                },
                &mut input,
                None,
                &counters,
            );
        });
        assert!(matches!(
            input.tables[TableId::States.index()],
            TableInput::ComparedEqual(_)
        ));
    }
    #[test]
    fn raid_dispatcher_comparison_changes_only_the_matching_memo_row() {
        use replay_data::rows::{RaidPathDispatcher, RaidPathDistanceRow};
        let mut compared = FrameEncoder::new();
        let mut ordinary = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let counters = Counters::default();
        for hour in 0..4 {
            let rows: Vec<_> = [RaidPathDispatcher::Land, RaidPathDispatcher::Naval]
                .into_iter()
                .map(|dispatcher| RaidPathDistanceRow {
                    dispatcher,
                    source: 7,
                    target: 9,
                    distance: if dispatcher == RaidPathDispatcher::Land {
                        900_000
                    } else {
                        500_000 + i64::from(hour / 2)
                    },
                    ok: 1,
                })
                .collect();
            let mut input = frame(hour, Vec::new());
            input.tables[TableId::RaidPathDistances.index()] =
                TableView::Complete(TableRows::RaidPathDistances(rows.clone())).into();
            let expected = ordinary.encode_input(input.clone(), false).unwrap();
            compare_frame(
                &compared,
                Target {
                    branch: 1,
                    hour,
                    restarting: false,
                },
                &mut input,
                None,
                &counters,
            );
            if hour == 2 {
                let TableInput::ComparedChanges { indices, .. } =
                    &input.tables[TableId::RaidPathDistances.index()]
                else {
                    panic!("naval-only delta");
                };
                assert_eq!(indices, &[1]);
            }
            let encoded = compared.encode_input(input, false).unwrap();
            assert_eq!(encoded, expected);
            assert_eq!(
                decoder
                    .apply(&encoded)
                    .unwrap()
                    .complete::<tables::RaidPathDistances>()
                    .unwrap(),
                rows
            );
        }
    }
}
