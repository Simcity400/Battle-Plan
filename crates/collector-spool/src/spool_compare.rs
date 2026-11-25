use collector_workers::spool_phases::{HELPER, Phase, SPOOL};
use replay_data::{
    BranchId, Hour, rows::RowCodec, rows::TableId, rows::TableRows, rows::TableSchema, rows::tables,
};
use replay_frame::{frame::FrameEncoder, frame::FrameInput, frame::TableInput, frame::TableView};
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicI64, AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub(crate) branch: BranchId,
    pub(crate) hour: Hour,
    pub(crate) restarting: bool,
}

pub(crate) const DENSE_RETRY_HOURS: i64 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Skipped,
    NoBaseline,
    Equal,
    Changes(usize),
    Kept,
    Dense,
    Deferred,
}

type Compare = fn(&FrameEncoder, Target, &mut TableInput, &mut Vec<usize>, &AtomicI64) -> Outcome;

macro_rules! compared_tables {
    ($($table:ident),+ $(,)?) => {
        pub(crate) const TABLES: [TableId; [$(stringify!($table)),+].len()] = [$(TableId::$table),+];
        const COMPARE: [Compare; TABLES.len()] = [$(compare::<tables::$table>),+];
    };
}

compared_tables!(
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
    ScriptVariableNames,
    ScriptVariableValues,
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
    CombatEngagementRefreshes,
    LandCombatEdges,
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
    LandUnitOrder,
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

pub(crate) struct Counters {
    #[cfg(test)]
    tables: [[AtomicU64; 6]; TABLES.len()],
    dense_at: [AtomicI64; TABLES.len()],
}

#[cfg(test)]
const EQUAL: usize = 0;
#[cfg(test)]
const CHANGES: usize = 1;
#[cfg(test)]
const KEPT: usize = 2;
#[cfg(test)]
const DENSE: usize = 3;
#[cfg(test)]
const DEFERRED: usize = 4;

impl Default for Counters {
    fn default() -> Self {
        Self {
            #[cfg(test)]
            tables: [const { [const { AtomicU64::new(0) }; 6] }; TABLES.len()],
            dense_at: [const { AtomicI64::new(i64::MIN) }; TABLES.len()],
        }
    }
}

#[cfg(test)]
impl Counters {
    fn record(&self, index: usize, outcome: Outcome) {
        let counter = match outcome {
            Outcome::Skipped | Outcome::NoBaseline => return,
            Outcome::Deferred => DEFERRED,
            Outcome::Equal => EQUAL,
            Outcome::Changes(_) => CHANGES,
            Outcome::Kept => KEPT,
            Outcome::Dense => DENSE,
        };
        self.tables[index][counter].fetch_add(1, Ordering::Relaxed);
    }

    fn load(&self, index: usize, counter: usize) -> u64 {
        self.tables[index][counter].load(Ordering::Relaxed)
    }

    pub(crate) fn decisions(&self, table: TableId) -> [u64; 5] {
        let index = TABLES.iter().position(|listed| *listed == table).unwrap();
        [EQUAL, CHANGES, KEPT, DENSE, DEFERRED].map(|counter| self.load(index, counter))
    }
}

fn vacant() -> TableInput {
    TableInput::Sampled(TableView::Unavailable {
        reason: String::new(),
    })
}

pub(crate) fn compare_frame(
    encoder: &FrameEncoder,
    target: Target,
    source: &mut FrameInput,
    helpers: Option<collector_workers::spool_helper::Helpers<'_>>,
    counters: &Counters,
) {
    if target.restarting {
        return;
    }
    let mut active = [0_usize; TABLES.len()];
    let mut active_len = 0;
    for (index, table) in TABLES.iter().enumerate() {
        if matches!(
            source.tables[table.index()],
            TableInput::Sampled(TableView::Complete(_))
        ) {
            active[active_len] = index;
            active_len += 1;
        }
    }
    if active_len == 0 {
        return;
    }
    let active = &active[..active_len];
    let slots: [Mutex<TableInput>; TABLES.len()] = std::array::from_fn(|index| {
        if matches!(
            source.tables[TABLES[index].index()],
            TableInput::Sampled(TableView::Complete(_))
        ) {
            Mutex::new(std::mem::replace(
                &mut source.tables[TABLES[index].index()],
                vacant(),
            ))
        } else {
            Mutex::new(vacant())
        }
    });
    let next = AtomicUsize::new(0);
    let work = || {
        let mut indices = Vec::new();
        loop {
            let slot = next.fetch_add(1, Ordering::Relaxed);
            let Some(&index) = active.get(slot) else {
                break;
            };
            if collector_memory::terminal::failed() {
                break;
            }
            let mut input = slots[index].lock().unwrap_or_else(PoisonError::into_inner);
            let _outcome = COMPARE[index](
                encoder,
                target,
                &mut input,
                &mut indices,
                &counters.dense_at[index],
            );
            drop(input);
            #[cfg(test)]
            counters.record(index, _outcome);
        }
    };
    let span =
    // SAFETY: This worker records phases on the counters assigned to its spool role.
    unsafe { SPOOL.span(Phase::Compare) };
    match helpers.filter(|_| active_len > 1) {
        Some(helpers) => {
            let context = collector_memory::terminal::context();
            let job = || {
                let _phase =
                    collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
                collector_memory::terminal::set_context(context);
                let _span =
                // SAFETY: This worker records phases on the counters assigned to its spool role.
                unsafe { HELPER.span(Phase::Compare) };
                work();
            };
            // SAFETY: The spool owns this conversion round and uses no helper in a competing
            // round.
            unsafe {
                helpers.join_each(&job, || {
                    work();
                    span.switch(Phase::HelperWait)
                })
            }
            .finish();
        }
        None => {
            work();
            span.finish();
        }
    }
    let mut slots = slots.map(|slot| slot.into_inner().unwrap_or_else(PoisonError::into_inner));
    for &index in active {
        source.tables[TABLES[index].index()] = std::mem::replace(&mut slots[index], vacant());
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
        if indices.len() == indices.capacity()
            && engine_allocation::reserve(indices, 1).is_err()
        {
            return Outcome::Kept;
        }
        indices.push(index);
    }
    if indices.is_empty() {
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
    use replay_data::rows::StateRow;
    use replay_frame::{frame::FrameDecoder, frame::Metrics};

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

    #[test]
    fn compared_frames_encode_the_bytes_of_complete_frames() {
        let pool = [
            // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
            unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }.unwrap(),
            // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
            unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }.unwrap(),
        ];
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
                    (helpers != 0).then_some(collector_workers::spool_helper::Helpers {
                        pool: &pool[..helpers],
                        backlogged: hour % 2 == 0,
                    }),
                    &counters,
                );
                let states = &input.tables[TableId::States.index()];
                match hour {
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
        let mut input = frame(1, rows);
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
        collector_test_support::with_allocations_denied(|| {
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
