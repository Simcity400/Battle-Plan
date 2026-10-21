//! Synthetic source states, faulting spool I/O, and indexed reads share no oracle logic.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    rc::Rc,
};

use crate::{
    BranchId, Hour, KEYFRAME_INTERVAL,
    branches::{BranchReason, BranchStart, BranchTracker, Gap, Parentage},
    container::{Replay, SeekResult},
    event_feed_rows::{
        DECISION_FAMILY_REFUSALS, EventFeedEventIdentity, EventFeedScope, EventFeedScript,
        EventFeedSource,
    },
    frame::{DecodedFrame, FrameDecoder, FrameEncoder, Metrics, TableUpdate, TableView},
    identity::{Identity, Roster, SessionStart, Verification},
    rows::{
        AirWingRaidReference, AirWingRow, ArmyOrderRow, CampaignRow, ClockRow, ControllerAreaNode,
        CountryLinkRow, CountryRow, DivisionRow, FactionMemberRow, FactionRow, FleetRow,
        HumanControllerRow, LandCombatRow, NavalCombatParticipantRow, NavalCombatRow, PlayerRow,
        ProvinceRow, Recorded, RowCodec, RowRef, ShipNamingOperands, ShipRow, StateRow, TableId,
        TableRows, TableSchema, TaskForceRow, VariantChangeRow, VariantOrganisationChange,
        VariantScalarChange, VariantSourceReferences, WarCapturedProvinceRow, WarRelationRow,
        compatibility::RETIRED_FIELDS, tables,
    },
    spool::{
        ControlRecord, Record, Recovery, SegmentBackend, SegmentSink, SegmentedSpool,
        encode_record, recover_prefix,
    },
};

const CAMPAIGN: &[u8] = b"synthetic-campaign-native-values";

const HOURS: usize = 2_000;
// Native handles retain the type in the low word and the instance id in the high word.
const DIVISION_A: u64 = (0x8000_0041_u64 << 32) | 53;
const DIVISION_B: u64 = (0x8000_0041_u64 << 32) | 54;
const DIVISION_EDGE_KEYS: [u64; 2] = [0, u64::MAX];
const OPTIONAL_SCALAR_CASES: [Option<i64>; 4] = [None, Some(0), Some(i64::MIN), Some(i64::MAX)];
// Every ordered pair of scalar cases occurs once per cycle, including the wraparound.
const OPTIONAL_SCALAR_ORDER: [usize; 16] = [0, 0, 1, 0, 2, 0, 3, 1, 1, 2, 1, 3, 2, 2, 3, 3];
const DOMAIN_TABLES: [TableId; TableId::ALL.len() - DIRECT_TABLES.len() - RETIRED_TABLES.len()] = [
    TableId::UnitLeaderTraitExperience,
    TableId::DivisionHistory,
    TableId::DivisionPanelSources,
    TableId::DivisionPanelStats,
    TableId::DoctrineFolders,
    TableId::DoctrineTracks,
    TableId::DoctrineLeaderMastery,
    TableId::DoctrineCostReductions,
    TableId::DoctrineMasteryGrants,
    TableId::DoctrineEquipmentBonuses,
    TableId::DoctrineCountries,
    TableId::FactionAccounts,
    TableId::FactionResearch,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionProjection,
    TableId::FactionResources,
    TableId::FactionGoalStatus,
    TableId::FactionGoalEntries,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionManpower,
    TableId::FactionUnlocks,
    TableId::FactionVariables,
    TableId::FactionAdvisorModifiers,
    TableId::CountryPowerDaily,
    TableId::CountryPowerChanges,
    TableId::CountryPowerDailyCosts,
    TableId::GovernmentChanges,
    TableId::PoliticsCaptureCoverage,
    TableId::WarRelations,
    TableId::WarOwnedRelations,
    TableId::WarRelationGoals,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
    TableId::CountryWargoals,
    TableId::WargoalTargets,
    TableId::WargoalJustifications,
    TableId::WarParticipation,
    TableId::WarCapturedProvinces,
    TableId::WarLendLeaseHistories,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::Players,
    TableId::HumanControllers,
    TableId::LandCombats,
    TableId::Fleets,
    TableId::TaskForces,
    TableId::Ships,
    TableId::NavalCombats,
    TableId::NavalCombatParticipants,
    TableId::AirWings,
    TableId::ArmyOrders,
    TableId::Focus,
    TableId::CompletedFocuses,
    TableId::ResearchTechnologies,
    TableId::ResearchSlots,
    TableId::TechnologyDefinitions,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::ActiveIdeas,
    TableId::DecisionCounts,
    TableId::Decisions,
    TableId::DynamicModifiers,
    TableId::PreferredTactics,
    TableId::CountryTacticUnlocks,
    TableId::TechSharingGroups,
    TableId::TechSharingMembers,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::PowerBalanceRanges,
    TableId::PowerBalanceOverrides,
    TableId::ProductionLines,
    TableId::CountryProductionModifiers,
    TableId::ProductionLineModifiers,
    TableId::Stockpiles,
    TableId::DivisionEquipment,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::VariantChanges,
    TableId::VariantBonusSnapshots,
    TableId::DesignCountryModifiers,
    TableId::DesignCountryTemplates,
    TableId::DesignObserverContext,
    TableId::DesignOrganisationLiveness,
    TableId::VariantUpgrades,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
    TableId::CountryFuel,
    TableId::CountryFuelConsumers,
    TableId::CountryFactories,
    TableId::ResourceDefinitions,
    TableId::CountryResources,
    TableId::ResourceExchanges,
    TableId::PurchaseContracts,
    TableId::ProvinceSupply,
    TableId::HubMotorization,
    TableId::DisruptedSupply,
    TableId::ForeignHomebases,
    TableId::CountryExperience,
    TableId::CountryExperienceGains,
    TableId::EventFeedHistory,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::StateBaseResources,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::OccupationSettings,
    TableId::StateOccupationLaws,
    TableId::StateGarrisons,
    TableId::StateAssignedManpower,
    TableId::CountryOccupationReserveManpower,
    TableId::StateResistance,
    TableId::CountryCollaboration,
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaders,
    TableId::UnitLeaderTraits,
    TableId::ActiveAbilities,
    TableId::Capitulation,
    TableId::Subjects,
    TableId::Autonomy,
    TableId::IntelligenceAgencies,
    TableId::OperativeMemberships,
    TableId::Cryptology,
    TableId::IntelNetworks,
    TableId::ScriptFlags,
    TableId::ScriptVariables,
    TableId::GlobalSavedEventTargets,
    TableId::EquipmentDestruction,
    TableId::PowerBalanceModifierBreakdowns,
    TableId::Operations,
    TableId::AgencyUpgradeLevels,
    TableId::OperationCompletions,
    TableId::OperationAssignments,
    TableId::WingEquipment,
    TableId::AirBases,
    TableId::WingCumulative,
    TableId::AirAces,
    TableId::AirStatistics,
    TableId::SunkConvoys,
    TableId::RailCooldowns,
    TableId::RailEdges,
    TableId::LandTheatres,
    TableId::LandTheatreGroups,
    TableId::LandTheatreMembers,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::SunkShips,
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectBreakthroughs,
    TableId::Nukes,
    TableId::RaidCountries,
    TableId::Raids,
    TableId::PeaceConferences,
    TableId::PeaceSettlements,
    TableId::PeaceBids,
    TableId::DivisionEquipmentDeletions,
    TableId::DivisionLandInputs,
    TableId::LandCombatResults,
    TableId::DestroyedDivisions,
    TableId::LandEventCoverage,
    TableId::NavalResultSources,
    TableId::AirLossEvents,
    TableId::BuildingBombingDamage,
    TableId::AirDailyRoster,
    TableId::AirRegionCombat,
    TableId::NuclearStrikes,
    TableId::RocketProductionOutput,
    TableId::WeatherRegions,
    TableId::WeatherProvinces,
    TableId::OperationTokens,
    TableId::LexicalTokens,
    TableId::NavalSupremacy,
    TableId::RadarCoverage,
    TableId::VolunteerHosts,
    TableId::SentVolunteers,
    TableId::LogisticsEquipment,
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationHistory,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
    TableId::ProductionLineOutput,
    TableId::ConstructionLineOutput,
    TableId::CombatLogEquipment,
    TableId::CombatLogState,
    TableId::CombatLogGroups,
    TableId::CombatLogMonths,
    TableId::CombatLogManpower,
    TableId::CombatLogTemplates,
    TableId::CombatLogReferences,
    TableId::CombatLogBattles,
    TableId::CombatLogEnemyEquipment,
    TableId::CombatLogBattleSides,
    TableId::CombatLogBattleEquipment,
    TableId::CombatLogBattleModifiers,
    TableId::CombatLogBattleCountries,
    TableId::CombatLogObservers,
    TableId::LendLeaseEquipment,
    TableId::DivisionCensus,
    TableId::TheatreAreas,
    TableId::StationedUnits,
    TableId::UnitIntel,
    TableId::AirBaseRegistrations,
    TableId::WorldTension,
    TableId::ProductionLicenses,
    TableId::DeploymentConveyors,
    TableId::DeploymentLines,
    TableId::DeploymentManpower,
    TableId::DeploymentEquipment,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
    TableId::ProductionLicenseRecords,
    TableId::OrdersGroups,
    TableId::OrdersGroupMembers,
    TableId::ArmyGroupMembers,
    TableId::IncomingDiplomaticActions,
    TableId::Scientists,
    TableId::ScientistDaily,
    TableId::ScientistSkills,
    TableId::RelationOpinions,
    TableId::RelationTuples,
    TableId::CountryIntel,
    TableId::ProvinceNames,
    TableId::LandCombatProgress,
    TableId::LandCombatSides,
    TableId::DivisionTemplates,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::StateFlags,
    TableId::ArmyOrderPlans,
    TableId::OrdersGroupPlans,
    TableId::Fronts,
    TableId::CountryAppearance,
    TableId::ArmyIconInputs,
    TableId::ShipModels,
    TableId::ProvinceUnitOrder,
    TableId::CharacterPortraits,
    TableId::LeaderPortraits,
    TableId::TaskForceCounterInputs,
    TableId::RailwayGunCounterInputs,
    TableId::TaskForceMapMotion,
    TableId::FleetCounterInputs,
    TableId::CountryCounterInputs,
    TableId::OrdersGroupCounterInputs,
    TableId::LeaderPortraitPaths,
    TableId::RaidTargets,
    TableId::NavalBases,
    TableId::FloatingHarborDeployments,
    TableId::FacilityMapFlags,
    TableId::NavalProvinceCombats,
    TableId::NavalCombatTransfers,
    TableId::NavalTransferUnits,
    TableId::TaskForceMissions,
    TableId::NavalCombatAirBaseWingHandles,
    TableId::ResearchInputs,
    TableId::ResearchAppliedBonuses,
    TableId::ResearchModifiers,
    TableId::SimulationFeatures,
    TableId::LocalPlayer,
    TableId::CountryVisibility,
    TableId::StateVisibility,
    TableId::RaidTargetStatuses,
    TableId::RaidCooldowns,
    TableId::RaidPathDistances,
    TableId::StateResourceInputs,
    TableId::StateResources,
    TableId::StateDynamicModifiers,
    TableId::CountryBuildingResources,
    TableId::RaidCountryInputs,
    TableId::CountryCommandPower,
    TableId::DivisionRetreatBans,
    TableId::StateCores,
    TableId::RaidSpeedInputs,
    TableId::RaidEquipment,
    TableId::InvasionPreparations,
    TableId::CountryCharacters,
    TableId::PartyLeaders,
    TableId::CountryCivilWars,
    TableId::AdvisorTokens,
    TableId::FactionAdvisors,
    TableId::RaidFacilities,
    TableId::RaidAirScores,
    TableId::RaidIntelInputs,
    TableId::RaidTooltipOperands,
    TableId::CountryRetiredCharacters,
    TableId::CharacterRoleTraits,
    TableId::CharacterOperatives,
    TableId::StockpileArchetypes,
    TableId::RaidAirSites,
    TableId::AceRangeInputs,
    TableId::CountryWars,
    TableId::NavalRegionAccess,
    TableId::BorderConflicts,
    TableId::RaidPlayerAllowedTypes,
    TableId::RelationLinks,
    TableId::SliceReads,
    TableId::DivisionTemplateIcons,
    TableId::CountryTemplateState,
    TableId::CountryLocalSubunits,
    TableId::AvailableEquipmentVariants,
    TableId::NewDivisionEquipment,
    TableId::AirRegionalMissions,
    TableId::AirCountryRelations,
    TableId::AirReferenceWitnesses,
    TableId::NavalRegionalMines,
    TableId::NavalDominanceTaskForceRanges,
    TableId::NavalPatrolMissionInputs,
    TableId::NavalAccidentReports,
    TableId::AirMissionInputs,
    TableId::AirSortieBaseCountries,
    TableId::AirSortieModifiers,
    TableId::AirSortieSpecialBases,
    TableId::AirSortieModifierMetadata,
    TableId::AirNavalPatrolCombats,
    TableId::AirNavalPatrolHistories,
    TableId::ModifierDefinitions,
    TableId::HiredAdvisors,
    TableId::AvailableAdvisors,
    TableId::TimedIdeas,
    TableId::DecisionRandomItems,
    TableId::CapturedOperativeReferences,
    TableId::ReservedCountryNaming,
    TableId::CountryStabilityInputs,
    TableId::CountryStabilityLines,
    TableId::SupplyNodes,
    TableId::TrainInventory,
    TableId::LandCombatDetails,
    TableId::LandCombatDivisions,
    TableId::LandCombatModifiers,
    TableId::LandCombatDivisionModifiers,
    TableId::FuelDemandParts,
    TableId::MapEvents,
    TableId::OwnerTransportSpeeds,
    TableId::ReserveModifierTrees,
    TableId::BattleSounds,
    TableId::OperativeMissions,
    TableId::StrategicOperatives,
    TableId::AgencyOperationBuffers,
    TableId::IntelNetworkSummaries,
    TableId::PropagandaEfforts,
    TableId::IntelContributions,
    TableId::IntelSources,
    TableId::IntelNetworkAuxiliaries,
    TableId::ResourceOrigins,
    TableId::ResourceRights,
    TableId::CountryOilFactors,
    TableId::StateTemporaryResources,
    TableId::ConvoyClients,
    TableId::TradeRoutes,
    TableId::RegionalConvoyEfficiency,
    TableId::CountryMarket,
    TableId::MarketContractTerms,
    TableId::LendLeaseAgreements,
    TableId::MarketEvents,
    TableId::MovementStateTerms,
    TableId::CoreSupplyAccess,
    TableId::CountryUnitSpeedEntries,
    TableId::RiverRelations,
    TableId::DivisionTransportSpeeds,
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    TableId::NavalModifierTrees,
    TableId::NavalModifierTreeSamples,
    TableId::CountryGameScoreInputs,
    TableId::AirWingGameScoreStats,
    TableId::GameScoreMultipliers,
    TableId::NavalCombatResultDetails,
    TableId::FactionTheatres,
    TableId::NavalMissionTaskForces,
    TableId::NavalMissionLedgers,
    TableId::NavalMissionCountries,
    TableId::NavalMissionCommanders,
    TableId::NavalRepairCountries,
    TableId::NavalRepairFleets,
    TableId::NavalRepairTaskForces,
    TableId::NavalTaskForceComposition,
    TableId::NavalRepairChildren,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::AirGroundCrews,
    TableId::AirTransferInputs,
];
/// Retired tables: no current writer emits them, and older recordings discard
/// them by name (the compatibility tests exercise that framing).
const RETIRED_TABLES: [TableId; crate::rows::compatibility::RETIRED_TABLES.len()] =
    crate::rows::compatibility::RETIRED_TABLES;
/// The fault campaign drives every table except those the clock, campaign,
/// country, province, state and division fixtures build directly, so a new table cannot
/// be added without being exercised here.
const DIRECT_TABLES: [TableId; 7] = [
    TableId::Clock,
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Provinces,
    TableId::States,
    TableId::Divisions,
];

#[test]
fn the_fault_campaign_covers_every_table_exactly_once() {
    let mut covered: Vec<TableId> = DOMAIN_TABLES
        .into_iter()
        .chain(DIRECT_TABLES)
        .chain(RETIRED_TABLES)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    covered.sort();
    assert_eq!(covered, TableId::ALL);
    assert_eq!(
        DOMAIN_TABLES.len() + DIRECT_TABLES.len() + RETIRED_TABLES.len(),
        TableId::ALL.len()
    );
    for table in RETIRED_TABLES {
        assert!(
            !table
                .last_change()
                .present(crate::format::FormatVersion::CURRENT.version())
        );
    }
}

const RAW_FLAG_CASES: [u8; 3] = [0, 2, 255];
const FACTION_A: u64 = (0x8000_0033_u64 << 32) | 75;

#[derive(Default)]
struct Disk {
    segments: BTreeMap<u32, Vec<u8>>,
    /// A partial write succeeds up to this many bytes; its next write fails.
    fail_after: Option<usize>,
}

#[derive(Clone, Default)]
struct MemoryBackend(Rc<RefCell<Disk>>);

struct MemorySink {
    disk: Rc<RefCell<Disk>>,
    index: u32,
}

impl SegmentBackend for MemoryBackend {
    type Sink = MemorySink;

    fn create(&mut self, index: u32) -> io::Result<Self::Sink> {
        assert!(
            self.0
                .borrow_mut()
                .segments
                .insert(index, Vec::new())
                .is_none()
        );
        Ok(MemorySink {
            disk: self.0.clone(),
            index,
        })
    }
}

impl Write for MemorySink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if disk.fail_after == Some(0) {
            disk.fail_after = None;
            return Err(io::Error::other("injected disk write failure"));
        }
        let length = disk
            .fail_after
            .map_or(bytes.len(), |left| bytes.len().min(left));
        if let Some(left) = &mut disk.fail_after {
            *left -= length;
        }
        disk.segments
            .get_mut(&self.index)
            .unwrap()
            .extend_from_slice(&bytes[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SegmentSink for MemorySink {
    fn sync_all(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn session(id: u64) -> SessionStart {
    SessionStart {
        id,
        process_id: 100 + id as u32,
        process_start_time: u64::MAX - id,
        qpc_frequency: 10_000_000,
        sample_boundary: crate::identity::SampleBoundary::EndOfHour,
        identity: Identity {
            game_version: "synthetic build".into(),
            exe_hash: [11; 32],
            exe_size: 123,
            vanilla_hash: [12; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: format!("test-session-{id}"),
            collector_hash: [id as u8; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

fn random(index: usize, salt: u64) -> u64 {
    let mut value = (index as u64)
        .wrapping_add(salt)
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The new map domains share fault scheduling, but each source keeps its native
/// key shape, scalar widths and ordered arrays independent of the codec.
fn push_map<S: TableSchema>(tables: &mut [TableView], row: S::Row) {
    let TableView::Complete(rows) = &mut tables[S::ID.index()] else {
        unreachable!()
    };
    let rows = rows.rows_mut::<S>().unwrap();
    rows.push(row);
    rows.sort_unstable_by_key(RowCodec::key);
}

fn add_map_rows(index: usize, tables: &mut [TableView]) {
    for ordinal in 0..3 {
        if ordinal == 2 && index % 17 >= 5 {
            continue;
        }
        let tick = index + ordinal;
        let id = [0, 0x8000_0041_0000_0035, u64::MAX][ordinal];
        let country = [0, 7, u32::MAX][(tick / 3) % 3];
        let scalar = |offset: usize| match (tick + offset) % 31 {
            0 => i64::MIN,
            1 => i64::MAX,
            value => (1_i64 << 53) + value as i64,
        };
        let handles = |offset: usize| match (tick + offset) % 4 {
            0 => Vec::new(),
            1 => vec![0, u64::MAX, DIVISION_A],
            2 => vec![DIVISION_A, 0, u64::MAX],
            _ => vec![u64::MAX, u64::MAX],
        };
        let classes = |offset: usize| match (tick + offset) % 4 {
            0 => Vec::new(),
            1 => vec![0, 13, u32::MAX],
            2 => vec![1, 0, 0],
            _ => vec![0, 0],
        };
        let geometry = |offset: usize| match (tick + offset) % 4 {
            0 => Vec::new(),
            1 => vec![0, u32::MAX, 7],
            2 => vec![7, u32::MAX, 0],
            _ => vec![u32::MAX, u32::MAX],
        };

        push_map::<tables::Players>(
            tables,
            PlayerRow {
                id: [0, i32::MAX, i32::MIN][ordinal],
                country,
                flags: RAW_FLAG_CASES[tick % RAW_FLAG_CASES.len()],
                user: vec![0xff, 0, (tick % 251) as u8],
                name: match (tick / 5) % 3 {
                    0 => Vec::new(),
                    1 => b"Duplicate name".to_vec(),
                    _ => vec![0, 0xff, (tick % 251) as u8],
                },
            },
        );
        push_map::<tables::HumanControllers>(
            tables,
            HumanControllerRow {
                country: [0, 7, u32::MAX][ordinal],
                count: [i8::MIN, -1, 0, i8::MAX][tick % 4],
            },
        );
        push_map::<tables::LandCombats>(
            tables,
            LandCombatRow {
                id,
                province: country,
                duration: [i32::MIN, i32::MAX, tick as i32][tick % 3],
                day: [i32::MIN, -1, i32::MAX][(tick + 1) % 3],
                attackers: handles(0),
                defenders: handles(1),
                attacker_classes: classes(0),
                defender_classes: classes(1),
            },
        );
        push_map::<tables::Fleets>(
            tables,
            FleetRow {
                id,
                owner: country,
                home_base: [0, u32::MAX, tick as u32][tick % 3],
                name: Recorded::Value(match (tick / 5) % 3 {
                    0 => None,
                    1 => Some(Vec::new()),
                    _ => Some(vec![0, 0xff, (tick % 251) as u8]),
                }),
                leader: Recorded::Value([None, Some(0), Some(u64::MAX)][tick % 3]),
            },
        );
        push_map::<tables::TaskForces>(
            tables,
            TaskForceRow {
                id,
                fleet: [0, u64::MAX, FACTION_A][tick % 3],
                roster_owner: country,
                logical_owner: [0, 2, u32::MAX][tick % 3],
                expeditionary_owner: [0, 19, u32::MAX][(tick + 1) % 3],
                province: tick as u32,
                mission: [0, u32::MAX, 7][tick % 3],
                route_count: [0, -1, i32::MAX][(tick + 2) % 3],
                combat_references: [0, i32::MIN, 1][tick % 3],
            },
        );
        push_map::<tables::Ships>(
            tables,
            ShipRow {
                name: Recorded::Value(match tick % 4 {
                    0 => None,
                    1 => Some(Vec::new()),
                    2 => Some(format!("Synthetic ship {tick}").into_bytes()),
                    _ => Some(vec![0xff, 0, tick as u8]),
                }),
                naming: Recorded::Value((!tick.is_multiple_of(3)).then(|| ShipNamingOperands {
                    kind: [i32::MIN, 1, i32::MAX][tick % 3],
                    order: [0, -7, i32::MAX][tick % 3],
                    ordered: RAW_FLAG_CASES[tick % RAW_FLAG_CASES.len()],
                    override_name: vec![tick as u8, 0, 0xff],
                    programmatic: RAW_FLAG_CASES[(tick + 1) % RAW_FLAG_CASES.len()],
                    equipment: tick.is_multiple_of(2).then_some(u64::MAX),
                })),
                id,
                task_force: [0, u64::MAX, FACTION_A][tick % 3],
                strength: scalar(0),
                organisation: scalar(1),
                // Whole hundredths (ShipRow::EXPERIENCE_STEP) and the sentinels.
                experience: match (tick + 2) % 31 {
                    0 => i64::MIN,
                    1 => i64::MAX,
                    value => ((1_i64 << 42) - value as i64) * 1_000,
                },
                category: [0, u32::MAX, tick as u32][tick % 3],
                sunk_convoys: tick as i32,
                class_id: [i32::MIN, 0, i32::MAX][tick % 3],
                acquired_manpower: Recorded::Value(
                    [None, Some(0), Some(i32::MIN), Some(i32::MAX)][tick % 4],
                ),
                required_manpower: Recorded::Value(
                    [Some(i32::MAX), Some(i32::MIN), Some(0), None][tick % 4],
                ),
            },
        );
        push_map::<tables::NavalCombats>(
            tables,
            NavalCombatRow {
                id,
                province: country,
                attacker_positioning: scalar(0),
                defender_positioning: scalar(1),
                attacker_anti_air: scalar(2),
                defender_anti_air: scalar(3),
                registry_ordinal: [0, u32::MAX, tick as u32][tick % 3],
            },
        );
        push_map::<tables::NavalCombatParticipants>(
            tables,
            NavalCombatParticipantRow {
                combat: if ordinal == 2 {
                    u64::MAX
                } else {
                    0x8000_0041_0000_0061
                },
                side: (ordinal % 2) as u8,
                battle_slot: u32::MAX,
                ship: [0, u64::MAX, DIVISION_A][tick % 3],
                owner: country,
                state: [0, u32::MAX, tick as u32][tick % 3],
            },
        );
        push_map::<tables::AirWings>(
            tables,
            AirWingRow {
                id,
                owner: country,
                air_base: [0, u64::MAX, FACTION_A][tick % 3],
                base_override: [u64::MAX, FACTION_A, 0][tick % 3],
                country_override: [i32::MIN, -1, i32::MAX][tick % 3],
                planes: [i32::MIN, 0, i32::MAX][tick % 3],
                mission_mask: [0, u32::MAX, 0x8000_0001][tick % 3],
                mission_region: [None, Some(0), Some(u32::MAX)][tick % 3],
                base_range_raw: [i64::MIN, 0, i64::MAX][tick % 3],
                ace: [0, u64::MAX, FACTION_A][tick % 3],
                mission_active: [0, 1, u8::MAX][tick % 3],
                eligibility_veto: [1, 0, u8::MAX][tick % 3],
                eligibility_counter: [i32::MIN, 0, i32::MAX][tick % 3],
                eligibility_limit: [i32::MAX, -1, 0][tick % 3],
                attached_orders_group: [None, Some(0), Some(u64::MAX)][tick % 3],
                pool_ordinal: [0, u32::MAX, 1][tick % 3],
                name: Recorded::Value(
                    [None, Some(Vec::new()), Some(vec![0xff, 0, b'W'])][tick % 3].clone(),
                ),
                strategic_bombing_raw: Recorded::Value([None, Some(0), Some(i64::MIN)][tick % 3]),
                replacement_setting: Recorded::Value([None, Some(0), Some(u8::MAX)][tick % 3]),
                map_flags: Recorded::Value([None, Some(0), Some(u32::MAX)][tick % 3]),
                fuel_base_raw: Recorded::Value([None, Some(0), Some(i64::MIN)][tick % 3]),
                fuel_current_type: Recorded::Value([None, Some(0), Some(i32::MAX)][tick % 3]),
                fuel_effective_aircraft: Recorded::Value([None, Some(0), Some(i32::MIN)][tick % 3]),
                fuel_context_mode: Recorded::Value([None, Some(0), Some(255)][tick % 3]),
                fuel_capacity_class: Recorded::Value([None, Some(0), Some(i32::MIN)][tick % 3]),
                fuel_override_resolved: Recorded::Value([None, Some(0), Some(1)][tick % 3]),
                pool_subunit: Recorded::Value([None, Some(0), Some(u32::MAX)][tick % 3]),
                raid_reference: Recorded::Value(
                    [
                        None,
                        Some(AirWingRaidReference {
                            raw: 0,
                            resolves: false,
                        }),
                        Some(AirWingRaidReference {
                            raw: u64::MAX,
                            resolves: true,
                        }),
                    ][tick % 3],
                ),
                acquired_manpower: Recorded::Value(
                    [None, Some(0), Some(i32::MIN), Some(i32::MAX)][tick % 4],
                ),
                exile_country: Recorded::Value(
                    [Some(i32::MAX), Some(i32::MIN), Some(0), None][tick % 4],
                ),
            },
        );
        push_map::<tables::ArmyOrders>(
            tables,
            ArmyOrderRow {
                owner_group: if ordinal == 2 {
                    u64::MAX
                } else {
                    0x8000_0041_0000_0051
                },
                instance: [0, 7, u32::MAX][ordinal],
                country,
                order_type: [i32::MIN, 0, i32::MAX][tick % 3],
                active: RAW_FLAG_CASES[tick % RAW_FLAG_CASES.len()],
                paradrop_target: [0, u32::MAX, tick as u32][tick % 3],
                path: geometry(0),
                target_states: geometry(1),
            },
        );
    }
}

/// Independent direct reads: no previous rows, field masks, or format reader participate.
fn source(index: usize, branch: BranchId, hour: Hour, signal: i64) -> DecodedFrame {
    source_with_faults(index, branch, hour, signal, true)
}

fn source_with_faults(
    index: usize,
    branch: BranchId,
    hour: Hour,
    signal: i64,
    faults: bool,
) -> DecodedFrame {
    let optional_scalar = |offset: usize| {
        OPTIONAL_SCALAR_CASES[OPTIONAL_SCALAR_ORDER[(index + offset) % OPTIONAL_SCALAR_ORDER.len()]]
    };
    let country = |id, canonical_slot| CountryRow {
        border_land_doctrine_level: crate::rows::Recorded::Value(None),
        id,
        canonical_slot,
        tag: if id == 7 {
            b"GER".to_vec()
        } else {
            b"TAG".to_vec()
        },
        political_power_raw: optional_scalar(0),
        command_power_raw: -((index / 7) as i64) * 100_000,
        stability_raw: 78_901,
        war_support_raw: 54_321,
        convoys_raw: optional_scalar(5),
        peace_conference_participant: u8::from(index.is_multiple_of(11) && id == 7),
        manpower_residual: [i32::MIN, -1, 0, i32::MAX][index % 4],
    };
    let mut countries = vec![country(7, 1), country(19, 3)];
    let mut links = vec![
        CountryLinkRow {
            id: 2,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 7,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 19,
            canonical_slot: 3,
        },
    ];
    if index % 13 < 7 {
        countries.push(country(29, 5));
        links.push(CountryLinkRow {
            id: 29,
            canonical_slot: 5,
        });
    }
    let mut provinces: Vec<_> = (1..=8)
        .map(|id| ProvinceRow {
            id,
            state: id / 3,
            owner: 2,
            controller: if id == 1 {
                if index.is_multiple_of(2) { 7 } else { 2 }
            } else {
                [0, 2, 19][(index / 9 + id as usize) % 3]
            },
            victory_points: [i32::MIN, -1, 0, i32::MAX][(index + id as usize) % 4],
            fort_positive: u8::from((index + id as usize).is_multiple_of(3)),
            move_gate: [0, 1, 2, 3][(index + id as usize) % 4],
            port_sea: [0, 1, -1, i32::MAX][id as usize % 4],
            attack: Default::default(),
            selected_areas: Default::default(),
            fort_bunker: [None, Some(-25_000), Some(i64::MIN)][(index + id as usize) % 3].into(),
            // Provinces 2..=8 form one area labelled 2 or two split at 5;
            // province 1 is sea; an owner rebuild relabels every eleventh hour.
            controller_area: Recorded::Value(Some(match (id, index % 5 < 2) {
                (1, _) => 0,
                (5.., true) => 5,
                _ => 2,
            })),
            owner_area: Recorded::Value(if index % 11 == 3 {
                None
            } else {
                Some(if id == 1 { 0 } else { 2 })
            }),
            area_node: Recorded::Value(match (id, index % 5 < 2) {
                (2, split) => Some(ControllerAreaNode {
                    first_land: Some([3, 2][index % 2]),
                    neighbours: Some(if split { vec![5] } else { vec![] }),
                }),
                (5, true) => Some(ControllerAreaNode {
                    first_land: None,
                    neighbours: (!index.is_multiple_of(3)).then(|| vec![2, 2]),
                }),
                _ => None,
            }),
        })
        .collect();
    if index % 17 < 5 {
        provinces.push(ProvinceRow {
            id: 9,
            state: u32::MAX,
            owner: 0,
            controller: 0,
            victory_points: i32::MAX,
            fort_positive: 0,
            move_gate: 0,
            port_sea: 0,
            attack: Default::default(),
            selected_areas: Default::default(),
            fort_bunker: None.into(),
            controller_area: Default::default(),
            owner_area: Default::default(),
            area_node: Default::default(),
        });
    }
    let mut states: Vec<_> = (0..=2)
        .map(|id| StateRow {
            manpower_locked: Default::default(),

            id,
            owner: [2, 19][(index / 37 + id as usize) % 2],
            controller: [0, 7, 19][(index / 5 + id as usize) % 3],
            movement_infrastructure: [i64::MIN, 0, 100_000, i64::MAX][(index + id as usize) % 4],
            railway_infrastructure: [None, Some(0), Some(i64::MIN), Some(i64::MAX)]
                [(index + id as usize) % 4]
                .into(),
            manpower_available: [i32::MIN, 0, i32::MAX][(index + id as usize) % 3],
            population: [i32::MAX, i32::MIN, 0][(index + id as usize) % 3].into(),
            category: crate::rows::Recorded::Value(
                [None, Some(0x2fab), Some(u32::MAX)][(index / 13 + id as usize) % 3],
            ),
            continent: crate::rows::Recorded::Value(Some((index + id as usize) as u32)),
            // Empty, ASCII and non-UTF-8 names, changing on their own cadence.
            custom_name: match (index / 11 + id as usize) % 3 {
                0 => Vec::new(),
                1 => format!("Renamed {id}").into_bytes(),
                _ => vec![0xff, b'x', 0],
            },
            owner_name: match (index / 7 + id as usize) % 2 {
                0 => Vec::new(),
                _ => format!("Owner name {}", index % 3).into_bytes(),
            },

            best_vp_index: Default::default(),

            single_controller: Default::default(),
        })
        .collect();
    if index % 17 < 5 {
        states.push(StateRow {
            id: u32::MAX,
            ..StateRow::default()
        });
    }
    // Whole display steps (crate::display), as capture stores them, besides
    // the sentinel words.
    let native_value = |tick: usize| match tick % 31 {
        0 => i64::MIN,
        1 => i64::MAX,
        _ => ((1_i64 << 42) - tick as i64) * 10_000,
    };
    let division = |id, ordinal: usize| {
        DivisionRow {
            id,
            roster_owner: [2, 19][(index / 101 + ordinal) % 2],
            logical_owner: [7, 19][(index / 79 + ordinal) % 2],
            expeditionary_owner: [0, 2, 19][(index / 83 + ordinal) % 3],
            province: ((index / 11 + ordinal) % 8 + 1) as u32,
            template: ((0x8000_0029 + (index / 127 + ordinal) as u64 % 2) << 32) | 91,
            // Alternating ticks isolate the HP and organisation field masks.
            hit_points: native_value(index / 2),
            organisation: native_value(index.div_ceil(2)),
            // Zero is the engine's own unassigned division, and it must reach
            // the codec as a real value alongside full 64-bit group handles.
            orders_group: [0, (57_u64 << 32) | 53, (u32::MAX as u64) << 32 | 53]
                [(index / 23 + ordinal) % 3],
            reveal_skip: RAW_FLAG_CASES[(index / 37 + ordinal) % 3],
        }
    };
    let mut divisions = vec![division(DIVISION_A, 0), division(DIVISION_B, 1)];
    if index % 17 < 5 {
        // Codec boundary keys exercise a zero first key and the full unsigned key range.
        divisions.insert(0, division(DIVISION_EDGE_KEYS[0], 2));
        divisions.push(division(DIVISION_EDGE_KEYS[1], 3));
    }
    let relation = |first_country, second_country, offset: usize| WarRelationRow {
        first_country,
        second_country,
        start_hour: [i32::MIN, hour, i32::MAX][(index / 7 + offset) % 3],
        end_hour: [i32::MAX, 0, i32::MIN][(index / 11 + offset) % 3],
        cancel: RAW_FLAG_CASES[(index + offset) % RAW_FLAG_CASES.len()],
        first_casualties: native_value(index + offset),
        second_casualties: native_value(index + offset + 1),
        first_unknown_casualties: native_value(index + offset + 2),
        second_unknown_casualties: native_value(index + offset + 3),
        first_was_instigator: RAW_FLAG_CASES[(index / 3 + offset) % RAW_FLAG_CASES.len()],
    };
    // Direction is part of the key: both orders must remain separate rows.
    let mut relations = vec![relation(7, 19, 0), relation(19, 7, 1)];
    if index % 17 < 5 {
        relations.insert(0, relation(0, u32::MAX, 2));
        relations.push(relation(u32::MAX, 0, 3));
    }
    let mut factions = vec![
        FactionRow {
            id: 0,
            name: b"Synthetic faction".to_vec(),
            ..FactionRow::default()
        },
        FactionRow {
            id: FACTION_A,
            name: match (index / 11) % 3 {
                0 => Vec::new(),
                1 => vec![0xff, 0, b'F'],
                _ => b"Renamed faction".to_vec(),
            },
            // A template and colour that change now and then, as a recolour or a new
            // template would, so their columns are encoded as deltas too.
            template_token: (!(index / 13).is_multiple_of(3))
                .then_some(((index / 13) % 3) as u32 * 0x4c1a),
            color_red: (((index / 7) % 5) as f32 / 4.0).to_bits(),
            color_green: [0.5_f32, 0.25][(index / 9) % 2].to_bits(),
            color_blue: [f32::NAN, 0.0][(index / 23) % 2].to_bits(),
            color_alpha: [1.0_f32, 0.5][(index / 29) % 2].to_bits(),
            color_overridden: ((index / 19) % 3) as u8,
        },
    ];
    if index % 17 < 5 {
        factions.push(FactionRow {
            id: u64::MAX,
            name: b"Temporary faction".to_vec(),
            ..FactionRow::default()
        });
    }
    let mut members = vec![
        FactionMemberRow {
            country: 7,
            faction: FACTION_A,
            ordinal: ((index / 3) % 2) as u32,
        },
        FactionMemberRow {
            country: 19,
            faction: FACTION_A,
            ordinal: (1 - (index / 3) % 2) as u32,
        },
    ];
    if index % 13 < 7 {
        members.push(FactionMemberRow {
            country: 29,
            faction: if index % 17 < 5 { u64::MAX } else { 0 },
            ordinal: 0,
        });
    }
    let initial_tables = [
        TableView::Complete(TableRows::Clock(vec![ClockRow {
            hour,
            play_time: (1_u64 << 40) + index as u64,
            play_session_started_at: signal,
            seed: i32::MIN,
            speed: (index % 5) as i32,
            // Counters advance most hours, hold on others, and go absent on
            // every seventh hour so present/absent transitions are exercised.
            debug_current_ref_id: (index % 7 != 3).then(|| u32::MAX - index as u32),
            unit: (index % 7 != 3).then_some(4012 + index as i32 / 3),
            order_index: (index % 7 != 3).then_some(i32::MAX - index as i32),
            front_index: (index % 7 != 3).then_some(8230),
            theatre_index: (index % 7 != 3).then_some(112),
            theater_group_index: (index % 7 != 3).then_some(924),
            military_deployment_line_index: (index % 7 != 3).then_some(7841),
            military_deployment_conveyor_index: (index % 7 != 3).then_some(744),
            equipment_variant_index: (index % 7 != 3).then_some(6295),
            country_leader_index: (index % 7 != 3).then_some(1),
            navy_id: (index % 7 != 3).then_some(8263),
            land_combat_id: (index % 7 != 3).then_some(15_359),
            multiplayer_random_count: (index % 7 != 3)
                .then_some(i32::MIN.wrapping_add(index as i32 * 997)),
        }])),
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.to_vec(),
        }])),
        TableView::Complete(TableRows::Countries(countries)),
        TableView::Complete(TableRows::CountryLinks(links)),
        TableView::Complete(TableRows::Provinces(provinces)),
        TableView::Complete(TableRows::States(states)),
        TableView::Complete(TableRows::Divisions(divisions)),
        TableView::Complete(TableRows::WarRelations(relations)),
        TableView::Complete(TableRows::Factions(factions)),
        TableView::Complete(TableRows::FactionMembers(members)),
    ];
    let mut tables =
        std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
    for (destination, initial) in tables.iter_mut().zip(initial_tables) {
        *destination = initial;
    }
    add_map_rows(index, &mut tables);
    for table in TableId::ALL.into_iter().skip(20) {
        let mut rows = crate::rows::sample_extended_table(table, index).unwrap();
        // Shared round-trip fixtures also need to exercise every mutable campaign column.
        match &mut rows {
            TableRows::VariantChanges(rows) => {
                for (ordinal, row) in rows.iter_mut().enumerate() {
                    let tick = index + ordinal;
                    let snapshot = row.snapshot.take().unwrap();
                    row.removed = false;
                    match tick % 6 {
                        0 => {
                            row.removed = !(tick / 6).is_multiple_of(2);
                            row.snapshot = Some(snapshot);
                        }
                        1 => row.removed = true,
                        2 => row.flags = Some(RAW_FLAG_CASES),
                        3 => {
                            row.sources = Some(VariantSourceReferences {
                                direct: vec![0, u64::MAX, tick as u64],
                                named: vec![tick as u64],
                            });
                        }
                        4 => {
                            row.organisation = Some(VariantOrganisationChange {
                                organisation: row.variant,
                                live: (tick / 6).is_multiple_of(2),
                            });
                            row.variant = 0;
                        }
                        _ => {
                            row.scalar = Some(VariantScalarChange {
                                display_name: Some(vec![0xff, 0, tick as u8]),
                                ..Default::default()
                            });
                        }
                    }
                }
                // Only sequence zero can carry a named observation failure.
                rows.insert(
                    0,
                    VariantChangeRow {
                        hour: index as i32,
                        failure: format!("synthetic variant observation failure {index}")
                            .into_bytes(),
                        ..Default::default()
                    },
                );
            }
            TableRows::DesignCountryModifiers(rows) => {
                for (ordinal, row) in rows.iter_mut().enumerate() {
                    row.failure = ((index + ordinal) % 3) as u8;
                    if row.failure != 0 {
                        row.value = None;
                    }
                }
            }
            TableRows::DesignCountryTemplates(rows) => {
                for (ordinal, row) in rows.iter_mut().enumerate() {
                    row.failure = ((index + ordinal) % 3) as u8;
                    if row.failure != 0 {
                        row.uses.clear();
                        row.factors.clear();
                        row.equipment.clear();
                    }
                }
            }
            TableRows::EventFeedHistory(rows) => {
                let row = &mut rows[0];
                row.script_refusals = if index.is_multiple_of(2) {
                    0
                } else {
                    DECISION_FAMILY_REFUSALS
                };
                // Keep the ordinary events and add a script with a bound scope on odd ticks.
                if !index.is_multiple_of(2) {
                    let mut event = row.events[0];
                    event.sequence += 1;
                    event.source = EventFeedSource::ScriptEvent;
                    event.cause = Recorded::Value(Default::default());
                    event.script = Some(EventFeedScript {
                        token_len: 3,
                        scope_len: 1,
                        native_event: Some(EventFeedEventIdentity {
                            kind: index as u32,
                            id: u32::MAX - index as u32,
                        }),
                        ..Default::default()
                    });
                    row.script_tokens = vec![0xff, 0, index as u8];
                    row.script_scopes = vec![EventFeedScope {
                        country: event.actor.reference,
                        state: index as i32,
                        random: [0, u32::MAX],
                        ..Default::default()
                    }];
                    row.events.push(event);
                    if let Some(counts) = &mut row.counts {
                        counts.acquired += 1;
                        counts.settled += 1;
                    }
                }
            }
            TableRows::IndustrialOrganisationDetails(rows) => {
                for (ordinal, row) in rows.iter_mut().enumerate() {
                    let tick = index + ordinal;
                    let raw = [i64::MIN, -1, i64::MAX][tick % 3];
                    let integer = [i32::MIN, -1, i32::MAX][tick % 3];
                    row.state.points = integer;
                    row.state.research_bonus_raw = raw;
                    row.state.task_capacity = integer;
                    row.state.research_assign_cost_raw = raw;
                    row.state.production_assign_cost_raw = raw;
                    row.state.design_team_change_cost = integer;
                    row.state.funds_gain_factor_raw = raw;
                    row.state.size_cost_factor_raw = raw;
                    row.state.upgrades = [0, 1, u8::MAX][tick % 3];
                    row.state.queue_index = integer;
                    row.state.policy = [0, 1, u32::MAX][tick % 3];
                    row.state.source_location =
                        [None, Some([0, u32::MAX]), Some([u32::MAX, 0])][tick % 3];
                }
            }
            TableRows::CountryCounterInputs(rows) => {
                for (ordinal, row) in rows.iter_mut().enumerate() {
                    let tick = index + ordinal;
                    row.sent_expeditionaries = Recorded::Value(match tick % 3 {
                        0 => None,
                        1 => Some(Vec::new().into()),
                        _ => Some(vec![u64::MAX, 0, 7, 7, tick as u64].into()),
                    });
                    row.recruitment_modifiers =
                        Recorded::Value((!tick.is_multiple_of(3)).then_some([
                            Some(tick as i64),
                            None,
                            Some(i64::MIN),
                            Some(i64::MAX),
                        ]));
                    row.air_combat_modifiers_known = Recorded::Value(tick.is_multiple_of(2));
                    if !tick.is_multiple_of(2) {
                        row.air_superiority_efficiency = Recorded::Value(None);
                        row.air_intercept_efficiency = Recorded::Value(None);
                        row.air_cas_efficiency = Recorded::Value(None);
                        row.air_nav_efficiency = Recorded::Value(None);
                        row.air_attack_factor = Recorded::Value(None);
                        row.air_defence_factor = Recorded::Value(None);
                        row.air_strategic_bomber_defence_factor = Recorded::Value(None);
                    }
                }
            }
            TableRows::PropagandaEfforts(rows) => {
                for row in rows.iter_mut().filter(|row| row.sequence == u64::MAX) {
                    row.derived_drift = (index % 2) as u8;
                    if row.derived_drift != 0 {
                        row.stability = 0;
                        row.war_support = 0;
                    }
                }
            }
            _ => {}
        }
        tables[table.index()] = TableView::Complete(rows);
    }
    if faults {
        for table in TableId::ALL.into_iter().skip(2) {
            let draw = random(index, 91 + table.index() as u64);
            let force_partial = matches!(index, 0 | 38 | 758 | 830)
                && (table.index() >= TableId::Players.index()
                    || matches!(
                        table,
                        TableId::Countries
                            | TableId::States
                            | TableId::Divisions
                            | TableId::WarRelations
                            | TableId::Factions
                            | TableId::FactionMembers
                    ));
            if draw.is_multiple_of(47) || (index == 780 && table == TableId::Provinces) {
                tables[table.index()] = TableView::Unavailable {
                    reason: format!("synthetic pointer unavailable for {table:?} at {hour}"),
                };
            } else if draw.is_multiple_of(43) || force_partial {
                let TableView::Complete(rows) = &tables[table.index()] else {
                    unreachable!()
                };
                assert!(!rows.is_empty(), "{table:?} has no rows to truncate");
                let retained = usize::from(table.key_bits() != 0).min(rows.len() - 1);
                tables[table.index()] = TableView::Truncated {
                    rows: {
                        let mut partial = rows.clone();
                        partial.truncate(retained);
                        partial
                    },
                    missing: (rows.len() - retained) as u32,
                };
            }
        }
    }
    crate::frame::retire_absent(DecodedFrame {
        branch,
        hour,
        tables: tables.map(Into::into),
        metrics: Metrics {
            total_ns: 100_000 + index as u64,
            table_ns: std::array::from_fn(|table| table as u64 + 1),
            pool_bytes: 1 << 25,
            dropped_frames: (index / 61) as u64,
        },
    })
}

/// Protect scenario boundaries and known parent witnesses from random queue drops.
fn may_drop(index: usize) -> bool {
    ![
        0_usize, 37, 38, 399, 448, 758, 780, 828, 830, 880, 929, 930, 999, 1000, 1499, 1500, 1999,
    ]
    .iter()
    .any(|anchor| index.abs_diff(*anchor) <= 2)
        && random(index, 7).is_multiple_of(61)
}

#[derive(Default)]
struct DomainCoverage {
    changed_fields: u64,
    changes: usize,
    insertions: usize,
    deletions: usize,
    unavailable: usize,
    truncated: usize,
    partial_keyframes: usize,
}

#[derive(Default)]
struct Coverage {
    keyframes: usize,
    periodic_keyframes: usize,
    delta_tables: usize,
    deletions: usize,
    insertions: usize,
    unchanged_tables: usize,
    unavailable_tables: usize,
    truncated_tables: usize,
    partial_keyframes: usize,
    state_delta_tables: usize,
    state_unavailable_tables: usize,
    state_truncated_tables: usize,
    state_partial_keyframes: usize,
    division_hp_only_deltas: usize,
    division_organisation_only_deltas: usize,
    division_deletions: usize,
    division_insertions: usize,
    division_unavailable_tables: usize,
    division_truncated_tables: usize,
    division_partial_keyframes: usize,
    division_edge_snapshots: [bool; 2],
    division_edge_insertions: [bool; 2],
    division_edge_deletions: [bool; 2],
    country_optional_transitions: [[bool; 4]; 4],
    equipment_coverage_values: [bool; 2],
    domains: Vec<DomainCoverage>,
    war_captured_directions: [bool; 2],
    war_captured_unchanged: usize,
    war_field_changes: u64,
    war_flag_values: [[bool; RAW_FLAG_CASES.len()]; 2],
    war_casualty_extremes: [[bool; 2]; 4],
    member_ordinal_only_deltas: usize,
    member_faction_changes: usize,
}

struct Campaign {
    segments: Vec<(u32, Recovery)>,
    expected: BTreeMap<(BranchId, Hour), DecodedFrame>,
    missing: BTreeMap<(BranchId, Hour), String>,
    starts: Vec<BranchStart>,
    coverage: Coverage,
}

fn synthesize() -> Campaign {
    let backend = MemoryBackend::default();
    let disk = backend.0.clone();
    let mut spool = SegmentedSpool::new(backend);
    let mut tracker = BranchTracker::new();
    let mut encoder = FrameEncoder::new();
    let mut expected = BTreeMap::new();
    let mut missing = BTreeMap::new();
    let mut starts = Vec::new();
    let mut pending_gaps = Vec::new();
    let mut coverage = Coverage {
        domains: (0..DOMAIN_TABLES.len())
            .map(|_| DomainCoverage::default())
            .collect(),
        ..Coverage::default()
    };
    let mut current_session = session(1);
    let mut hour = 10_000;
    let mut signal = 1_000;
    let mut last_keyframe = None;
    let mut failed_length = None;
    let mut final_frame_bytes = 0;
    let mut final_hour = (0, 0);

    for index in 0..HOURS {
        if matches!(index, 0 | 1000 | 1500) {
            if index != 0 {
                spool
                    .append(&Record::Control(ControlRecord::Detach {
                        session: current_session.id,
                        last_hour: Some(hour),
                    }))
                    .unwrap();
                spool.finish_segment().unwrap();
            }
            current_session = session(if index == 0 {
                1
            } else if index == 1000 {
                2
            } else {
                3
            });
            tracker.begin_session(current_session.id).unwrap();
            if index == 1500 {
                hour = 10_449;
            }
        }
        match index {
            780 => {
                hour = 10_400;
                signal += 1;
            } // Earlier recorded hour, one possible parent.
            830 => {
                hour = 10_449;
                signal += 1;
            } // Repeat last sampled hour, ambiguous parent.
            880 => {
                hour = 12_000;
                signal += 1;
            } // Later save outside any recorded branch.
            930 => {
                signal += 1;
            } // Ordinary next-hour sample; only the load signal exposes it.
            _ => {}
        }
        let loaded_hour = hour;
        hour += 1;
        if may_drop(index) {
            let branch = tracker.current_branch().unwrap();
            let reason = "bounded queue overflow".to_owned();
            pending_gaps.push(Gap {
                branch,
                first_hour: hour,
                last_hour: hour,
                reason: reason.clone(),
            });
            assert!(missing.insert((branch, hour), reason).is_none());
            continue;
        }
        let observed = tracker
            .observe(Some(loaded_hour), hour, Some(signal), &pending_gaps)
            .unwrap();
        if let Some(start) = &observed.start {
            // Calculate ancestry from direct successfully written witnesses, not the tracker.
            let parents: Vec<_> = expected
                .keys()
                .filter_map(|&(branch, recorded)| (recorded == loaded_hour).then_some(branch))
                .collect();
            let parent = match parents.as_slice() {
                [branch] => Parentage::Probable {
                    branch: *branch,
                    hour: loaded_hour,
                },
                _ => Parentage::Unknown,
            };
            assert_eq!(start.parent, parent);
            starts.push(start.clone());
        }
        let source = source(index, observed.branch, hour, signal);
        let force = !spool.is_open() || observed.start.is_some();
        let periodic = !force
            && last_keyframe.is_some_and(|previous| {
                i64::from(hour) - i64::from(previous) >= i64::from(KEYFRAME_INTERVAL)
            });
        let keyframe = force || periodic;
        let frame = encoder.encode(source.clone(), force).unwrap();
        assert_eq!(
            frame.keyframe, keyframe,
            "keyframe cadence at source {index}"
        );
        if keyframe {
            last_keyframe = Some(hour);
        }
        coverage.keyframes += usize::from(keyframe);
        coverage.periodic_keyframes += usize::from(periodic);
        for (table, update) in TableId::ALL.into_iter().zip(&frame.tables) {
            let domain_index = DOMAIN_TABLES.iter().position(|id| *id == table);
            if let TableView::Complete(TableRows::EquipmentDestruction(rows)) =
                &*source.tables[table.index()]
            {
                for row in rows {
                    let Recorded::Value(value) = row.coverage else {
                        panic!("current equipment coverage was not recorded");
                    };
                    coverage.equipment_coverage_values[value as usize] = true;
                }
            }
            if let TableView::Complete(TableRows::WarCapturedProvinces(rows))
            | TableView::Truncated {
                rows: TableRows::WarCapturedProvinces(rows),
                ..
            } = &*source.tables[table.index()]
            {
                for row in rows {
                    assert_eq!(row.credited, 1, "captured-province membership marker");
                    coverage.war_captured_directions[row.direction as usize] = true;
                }
            }
            if table == TableId::WarRelations
                && let TableView::Complete(rows) = &*source.tables[table.index()]
            {
                for row in rows {
                    let RowRef::WarRelation(row) = row else {
                        unreachable!()
                    };
                    for (field, value) in [row.cancel, row.first_was_instigator]
                        .into_iter()
                        .enumerate()
                    {
                        let case = RAW_FLAG_CASES.iter().position(|raw| *raw == value).unwrap();
                        coverage.war_flag_values[field][case] = true;
                    }
                    for (field, value) in [
                        row.first_casualties,
                        row.second_casualties,
                        row.first_unknown_casualties,
                        row.second_unknown_casualties,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        coverage.war_casualty_extremes[field][0] |= value == i64::MIN;
                        coverage.war_casualty_extremes[field][1] |= value == i64::MAX;
                    }
                }
            }
            match update {
                TableUpdate::Delta {
                    baseline,
                    changed,
                    deleted,
                } => {
                    coverage.delta_tables += 1;
                    coverage.state_delta_tables += usize::from(table == TableId::States);
                    coverage.deletions += deleted.len();
                    coverage.insertions += changed
                        .iter()
                        .filter(|row| row.mask == table.full_mask())
                        .count();
                    if let Some(domain_index) = domain_index {
                        let previous: &DecodedFrame = &expected[&(observed.branch, *baseline)];
                        let TableView::Complete(before) = &*previous.tables[table.index()] else {
                            panic!("relation delta must have a complete direct source baseline");
                        };
                        coverage.domains[domain_index].deletions += deleted.len();
                        for row in changed {
                            if before.iter().any(|old| old.key() == row.key) {
                                coverage.domains[domain_index].changes += 1;
                                coverage.domains[domain_index].changed_fields |= row.mask;
                                if table == TableId::WarRelations {
                                    coverage.war_field_changes |= row.mask;
                                }
                                if table == TableId::FactionMembers {
                                    coverage.member_ordinal_only_deltas +=
                                        usize::from(row.mask == 1 << 1);
                                    coverage.member_faction_changes +=
                                        usize::from(row.mask & 1 != 0);
                                }
                            } else {
                                coverage.domains[domain_index].insertions += 1;
                                if table == TableId::WarCapturedProvinces {
                                    assert_eq!(row.mask, table.full_mask());
                                    let inserted =
                                        row.apply::<WarCapturedProvinceRow>(None).unwrap();
                                    let TableView::Complete(TableRows::WarCapturedProvinces(rows)) =
                                        &*source.tables[table.index()]
                                    else {
                                        panic!(
                                            "membership insertion needs a complete direct source"
                                        );
                                    };
                                    assert_eq!(
                                        rows.iter().find(|source| source.key() == row.key),
                                        Some(&inserted),
                                        "captured-province insertion differs from its direct source"
                                    );
                                }
                            }
                        }
                    }
                    if table == TableId::Divisions {
                        coverage.division_hp_only_deltas +=
                            changed.iter().filter(|row| row.mask == 1 << 5).count();
                        coverage.division_organisation_only_deltas +=
                            changed.iter().filter(|row| row.mask == 1 << 6).count();
                        coverage.division_deletions += deleted.len();
                        coverage.division_insertions += changed
                            .iter()
                            .filter(|row| row.mask == table.full_mask())
                            .count();
                        for (edge, key) in DIVISION_EDGE_KEYS.into_iter().enumerate() {
                            coverage.division_edge_insertions[edge] |= changed.iter().any(|row| {
                                row.key == u128::from(key) && row.mask == table.full_mask()
                            });
                            coverage.division_edge_deletions[edge] |=
                                deleted.contains(&u128::from(key));
                        }
                    }
                    if table == TableId::Countries {
                        let previous: &DecodedFrame = &expected[&(observed.branch, *baseline)];
                        let (TableView::Complete(before), TableView::Complete(after)) = (
                            &*previous.tables[table.index()],
                            &*source.tables[table.index()],
                        ) else {
                            panic!("country delta must have complete direct source baselines");
                        };
                        let (Some(RowRef::Country(before)), Some(RowRef::Country(after))) =
                            (before.first(), after.first())
                        else {
                            unreachable!()
                        };
                        for (old, new) in [
                            (before.political_power_raw, after.political_power_raw),
                            (before.convoys_raw, after.convoys_raw),
                        ] {
                            let old = OPTIONAL_SCALAR_CASES
                                .iter()
                                .position(|value| *value == old)
                                .unwrap();
                            let new = OPTIONAL_SCALAR_CASES
                                .iter()
                                .position(|value| *value == new)
                                .unwrap();
                            coverage.country_optional_transitions[old][new] = true;
                        }
                    }
                }
                TableUpdate::Unchanged { .. } => {
                    coverage.unchanged_tables += 1;
                    coverage.war_captured_unchanged +=
                        usize::from(table == TableId::WarCapturedProvinces);
                }
                TableUpdate::Unavailable { .. } | TableUpdate::Incompatible { .. } => {
                    coverage.unavailable_tables += 1;
                    if let Some(domain_index) = domain_index {
                        coverage.domains[domain_index].unavailable += 1;
                    }
                    coverage.state_unavailable_tables += usize::from(table == TableId::States);
                    coverage.division_unavailable_tables +=
                        usize::from(table == TableId::Divisions);
                }
                TableUpdate::Truncated { .. } => {
                    coverage.truncated_tables += 1;
                    if let Some(domain_index) = domain_index {
                        coverage.domains[domain_index].truncated += 1;
                    }
                    coverage.state_truncated_tables += usize::from(table == TableId::States);
                    coverage.division_truncated_tables += usize::from(table == TableId::Divisions);
                }
                TableUpdate::Snapshot(rows) => {
                    if table == TableId::Divisions {
                        for (edge, key) in DIVISION_EDGE_KEYS.into_iter().enumerate() {
                            coverage.division_edge_snapshots[edge] |=
                                rows.iter().any(|row| row.key() == u128::from(key));
                        }
                    }
                }
            }
        }

        if spool.is_open() {
            for gap in pending_gaps.drain(..) {
                spool.append(&Record::Gap(gap)).unwrap();
            }
            if let Some(start) = &observed.start {
                spool.append(&Record::BranchStart(start.clone())).unwrap();
            }
            if index == HOURS - 1 {
                spool
                    .append(&Record::Control(ControlRecord::ClockWitness {
                        session: current_session.id,
                        qpc: u64::MAX,
                        hour,
                        load_signal: Some(signal),
                    }))
                    .unwrap();
            }
            if index == 37 {
                disk.borrow_mut().fail_after =
                    Some(encode_record(&Record::Frame(frame.clone())).unwrap().len() / 2);
                assert!(spool.append(&Record::Frame(frame.clone())).is_err());
                assert!(!spool.is_open());
                let length = disk.borrow().segments[&0].len();
                assert!(spool.append(&Record::Frame(frame)).is_err());
                assert_eq!(disk.borrow().segments[&0].len(), length);
                failed_length = Some(length);
                let reason = "injected disk write failure".to_owned();
                pending_gaps.push(Gap {
                    branch: observed.branch,
                    first_hour: hour,
                    last_hour: hour,
                    reason: reason.clone(),
                });
                missing.insert((observed.branch, hour), reason);
                continue;
            }
            final_frame_bytes = encode_record(&Record::Frame(frame.clone())).unwrap().len();
            spool.append(&Record::Frame(frame)).unwrap();
        } else {
            let mut context = vec![
                Record::SessionStart(current_session.clone()),
                Record::BranchStart(tracker.branch_start(observed.branch).unwrap().clone()),
            ];
            context.extend(pending_gaps.drain(..).map(Record::Gap));
            spool.start_segment(&context, &frame).unwrap();
        }
        tracker.record_hour(observed.branch, hour).unwrap();
        let mut direct = source;
        if keyframe {
            for (id, table) in TableId::ALL.into_iter().zip(&mut direct.tables) {
                if let TableView::Truncated { missing, .. } = &**table {
                    coverage.partial_keyframes += 1;
                    coverage.state_partial_keyframes += usize::from(id == TableId::States);
                    coverage.division_partial_keyframes += usize::from(id == TableId::Divisions);
                    if let Some(domain_index) = DOMAIN_TABLES.iter().position(|table| *table == id)
                    {
                        coverage.domains[domain_index].partial_keyframes += 1;
                    }
                    *table = TableView::Unavailable {
                        reason: format!("truncated at keyframe: {missing} rows not read"),
                    }
                    .into();
                }
            }
        }
        final_hour = (observed.branch, hour);
        assert!(expected.insert(final_hour, direct).is_none());
    }
    spool.finish_segment().unwrap();
    assert!(pending_gaps.is_empty());
    let mut disk = disk.borrow_mut();
    assert_eq!(
        disk.segments[&0].len(),
        failed_length.unwrap(),
        "abandoned segment was appended to"
    );
    assert_eq!(disk.segments.len(), 4);
    // Model forced termination partway through the last record, after its clock witness.
    let last = disk.segments.last_entry().unwrap().into_mut();
    last.truncate(last.len() - final_frame_bytes / 2);
    expected.remove(&final_hour).unwrap();
    let mut segments = Vec::new();
    for (&number, bytes) in &disk.segments {
        let mut recovered = recover_prefix(bytes);
        assert_eq!(recovered.damage.is_some(), matches!(number, 0 | 3));
        if recovered.damage.is_some() {
            assert!(recovered.valid_bytes < bytes.len());
        }
        if number == 3 {
            let witness = recovered
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Record::Control(ControlRecord::ClockWitness { hour, .. }) => Some(*hour),
                    _ => None,
                })
                .unwrap();
            assert_eq!(witness, final_hour.1);
            let reason = recovered.damage.clone().unwrap();
            recovered.records.push(Record::Gap(Gap {
                branch: final_hour.0,
                first_hour: witness,
                last_hour: witness,
                reason: reason.clone(),
            }));
            missing.insert(final_hour, reason);
        }
        segments.push((number, recovered));
    }
    Campaign {
        segments,
        expected,
        missing,
        starts,
        coverage,
    }
}

fn assert_aliases(frame: &DecodedFrame) {
    let (
        TableView::Complete(countries),
        TableView::Complete(links),
        TableView::Complete(provinces),
    ) = (
        &*frame.tables[TableId::Countries.index()],
        &*frame.tables[TableId::CountryLinks.index()],
        &*frame.tables[TableId::Provinces.index()],
    )
    else {
        return;
    };
    let Some(RowRef::Province(province)) = provinces.first() else {
        return;
    };
    assert_eq!(province.owner, 2);
    let slot = links
        .iter()
        .find_map(|row| match row {
            RowRef::CountryLink(link) if link.id == province.owner => Some(link.canonical_slot),
            _ => None,
        })
        .unwrap();
    let country = countries
        .iter()
        .find_map(|row| match row {
            RowRef::Country(country) if country.canonical_slot == slot => Some(country),
            _ => None,
        })
        .unwrap();
    assert_eq!((country.id, country.canonical_slot), (7, 1));
    assert_ne!(province.owner, country.id);
}

fn assert_division_handles(frame: &DecodedFrame) {
    let TableView::Complete(divisions) = &*frame.tables[TableId::Divisions.index()] else {
        return;
    };
    let handles: Vec<_> = divisions
        .iter()
        .filter_map(|row| match row {
            RowRef::Division(division) if division.id >> 32 == DIVISION_A >> 32 => {
                Some(division.id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(handles, [DIVISION_A, DIVISION_B]);
}

fn assert_relations(frame: &DecodedFrame) {
    if let TableView::Complete(relations) = &*frame.tables[TableId::WarRelations.index()] {
        let ordered_pairs: Vec<_> = relations
            .iter()
            .filter_map(|row| match row {
                RowRef::WarRelation(row)
                    if matches!((row.first_country, row.second_country), (7, 19) | (19, 7)) =>
                {
                    Some((row.first_country, row.second_country))
                }
                _ => None,
            })
            .collect();
        assert_eq!(ordered_pairs, [(7, 19), (19, 7)]);
    }
    let TableView::Complete(members) = &*frame.tables[TableId::FactionMembers.index()] else {
        return;
    };
    assert!(
        members
            .iter()
            .zip(members.iter().skip(1))
            .all(|(left, right)| left.key() < right.key())
    );
    let first_faction: Vec<_> = members
        .iter()
        .filter_map(|row| match row {
            RowRef::FactionMember(row) if row.faction == FACTION_A => Some(row),
            _ => None,
        })
        .collect();
    assert_eq!(
        first_faction
            .iter()
            .map(|row| row.country)
            .collect::<Vec<_>>(),
        [7, 19]
    );
    let mut ordinals: Vec<_> = first_faction.iter().map(|row| row.ordinal).collect();
    ordinals.sort_unstable();
    assert_eq!(ordinals, [0, 1]);
    if let TableView::Complete(factions) = &*frame.tables[TableId::Factions.index()] {
        for member in members {
            let RowRef::FactionMember(member) = member else {
                unreachable!()
            };
            assert!(
                factions
                    .iter()
                    .any(|faction| faction.key() == u128::from(member.faction))
            );
        }
    }
}

#[test]
fn two_thousand_hours_survive_faults_and_append_with_exact_direct_state() {
    let campaign = synthesize();
    assert_eq!(campaign.expected.len() + campaign.missing.len(), HOURS);
    assert!(campaign.missing.len() > 20);
    assert_eq!(campaign.starts.len(), 7);
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.session)
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 2, 3]
    );
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.reason)
            .collect::<Vec<_>>(),
        [
            BranchReason::SessionStart,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::LoadSignal,
            BranchReason::SessionStart,
            BranchReason::SessionStart,
        ]
    );
    assert_eq!(
        campaign
            .starts
            .iter()
            .map(|start| start.parent.clone())
            .collect::<Vec<_>>(),
        [
            Parentage::Unknown,
            Parentage::Probable {
                branch: 1,
                hour: 10_400
            },
            Parentage::Unknown,
            Parentage::Unknown,
            Parentage::Probable {
                branch: 4,
                hour: 12_050
            },
            Parentage::Probable {
                branch: 5,
                hour: 12_120
            },
            Parentage::Unknown,
        ]
    );
    let coverage = &campaign.coverage;
    assert!(coverage.keyframes >= 9 && coverage.periodic_keyframes > 0);
    assert!(coverage.delta_tables > 1_000 && coverage.unchanged_tables > 1_000);
    assert!(coverage.deletions > 100 && coverage.insertions > 100);
    assert!(coverage.unavailable_tables > 50 && coverage.truncated_tables > 50);
    assert!(coverage.partial_keyframes >= 3);
    assert!(coverage.state_delta_tables > 100);
    assert!(coverage.state_unavailable_tables > 0 && coverage.state_truncated_tables > 0);
    assert!(coverage.state_partial_keyframes > 0);
    assert!(coverage.division_hp_only_deltas > 100);
    assert!(coverage.division_organisation_only_deltas > 100);
    assert!(coverage.division_deletions > 10 && coverage.division_insertions > 10);
    assert!(coverage.division_unavailable_tables > 0 && coverage.division_truncated_tables > 0);
    assert!(coverage.division_partial_keyframes > 0);
    assert!(
        coverage
            .division_edge_snapshots
            .into_iter()
            .all(|seen| seen)
    );
    assert!(
        coverage
            .division_edge_insertions
            .into_iter()
            .all(|seen| seen)
    );
    assert!(
        coverage
            .division_edge_deletions
            .into_iter()
            .all(|seen| seen)
    );
    for (old, next) in coverage.country_optional_transitions.iter().enumerate() {
        for (new, seen) in next.iter().enumerate() {
            assert!(
                old == new || *seen,
                "optional scalar transition {old} -> {new} was not exercised"
            );
        }
    }
    assert!(
        coverage
            .equipment_coverage_values
            .into_iter()
            .all(|seen| seen)
    );
    for (table, covered) in DOMAIN_TABLES.into_iter().zip(&coverage.domains) {
        let mutable_fields = match table {
            // Equipment coverage is fixed by key: amount rows and the key-zero marker.
            TableId::EquipmentDestruction => table.full_mask() & !(1 << 1),
            // Captured provinces are membership keys; their only value must be credited=1.
            TableId::WarCapturedProvinces => {
                assert_eq!(table.full_mask(), 1);
                0
            }
            _ => table.full_mask(),
        };
        // A retired whole field stores the same empty payload in every row.
        let mutable_fields = RETIRED_FIELDS
            .iter()
            .filter(|field| field.table == table && !field.name.contains('.'))
            .fold(mutable_fields, |mask, field| mask & !(1 << field.index));
        assert_eq!(
            covered.changed_fields, mutable_fields,
            "{table:?} mutable field coverage"
        );
        if table == TableId::WarCapturedProvinces {
            assert_eq!(
                covered.changes, 0,
                "captured-province membership was rewritten"
            );
            assert!(
                coverage.war_captured_unchanged > 10,
                "captured-province membership never stayed unchanged"
            );
            assert!(
                coverage
                    .war_captured_directions
                    .into_iter()
                    .all(|seen| seen)
            );
        } else {
            assert!(
                covered.changes > 10,
                "{table:?} never changed existing rows"
            );
        }
        if table.key_bits() != 0 {
            assert!(covered.insertions > 10, "{table:?} never inserted new rows");
            assert!(covered.deletions > 10, "{table:?} never deleted rows");
        }
        assert!(
            covered.unavailable > 0,
            "{table:?} never became unavailable"
        );
        assert!(covered.truncated > 0, "{table:?} never truncated rows");
        assert!(
            covered.partial_keyframes > 0,
            "{table:?} never truncated at a keyframe"
        );
    }
    assert_eq!(
        coverage.war_field_changes,
        TableId::WarRelations.full_mask()
    );
    assert!(coverage.war_flag_values.iter().flatten().all(|seen| *seen));
    assert!(
        coverage
            .war_casualty_extremes
            .iter()
            .flatten()
            .all(|seen| *seen)
    );
    assert!(coverage.member_ordinal_only_deltas > 100);
    assert!(coverage.member_faction_changes > 10);

    let first = Replay::from_recovered(CAMPAIGN.to_vec(), &campaign.segments[..2]).unwrap();
    assert_eq!(first.sessions.len(), 1);
    let old_bytes = first.bytes().to_vec();
    let appended = first.append_recovered(&campaign.segments[2..]).unwrap();
    assert_eq!(first.bytes(), old_bytes);
    for block in &first.index {
        let start = block.offset as usize;
        let end = start + block.length as usize;
        assert_eq!(&appended.bytes()[start..end], &old_bytes[start..end]);
        assert!(appended.index.contains(block));
    }
    let replay = Replay::open(appended.bytes().to_vec()).unwrap();
    assert_eq!(replay.sessions, [session(1), session(2), session(3)]);
    assert_eq!(replay.branches, campaign.starts);
    assert_eq!(replay.campaign_id, CAMPAIGN);
    assert_eq!(
        replay
            .recovery_notes
            .iter()
            .map(|note| note.segment)
            .collect::<Vec<_>>(),
        [0, 3]
    );
    for note in &replay.recovery_notes {
        let recovery = &campaign.segments[note.segment as usize].1;
        assert_eq!(note.valid_bytes, recovery.valid_bytes as u64);
        assert_eq!(Some(note.reason.as_str()), recovery.damage.as_deref());
    }
    assert!(replay.index.iter().all(|block| i64::from(block.last_hour)
        - i64::from(block.first_hour)
        < i64::from(KEYFRAME_INTERVAL)));

    // Sequential playback checks every independently generated state, including
    // partial rows and metrics as blocks store them, while decompressing each
    // block only once.
    let stored = |frame: &DecodedFrame| DecodedFrame {
        metrics: frame.metrics.stored(),
        ..frame.clone()
    };
    let mut visited = BTreeSet::new();
    let mut cold_targets = BTreeSet::new();
    for block in &replay.index {
        let frames = replay.read_block(block).unwrap();
        let mut decoder = FrameDecoder::new();
        for frame in &frames {
            let key = (frame.branch, frame.hour);
            assert!(visited.insert(key), "duplicate replay hour {key:?}");
            let expected = &stored(&campaign.expected[&key]);
            let actual = decoder.advance(frame).unwrap();
            assert_eq!(actual, expected, "branch {}, hour {}", key.0, key.1);
            assert_aliases(actual);
            assert_division_handles(actual);
            assert_relations(actual);
        }
        for frame in [frames.first(), frames.get(frames.len() / 2), frames.last()]
            .into_iter()
            .flatten()
        {
            cold_targets.insert((frame.branch, frame.hour));
        }
    }
    assert_eq!(visited, campaign.expected.keys().copied().collect());
    // Cold seeks independently exercise block boundaries, interior deltas and
    // both surviving sides of every dropped or damaged-record boundary.
    for &(branch, hour) in campaign.missing.keys() {
        for adjacent in [hour.checked_sub(1), hour.checked_add(1)]
            .into_iter()
            .flatten()
        {
            if campaign.expected.contains_key(&(branch, adjacent)) {
                cold_targets.insert((branch, adjacent));
            }
        }
    }
    let cold_targets: Vec<_> = cold_targets.into_iter().collect();
    for_each_parallel(&cold_targets, |&(branch, hour)| {
        let expected = &stored(&campaign.expected[&(branch, hour)]);
        match replay.seek(branch, hour).unwrap() {
            SeekResult::Frame(actual) => {
                assert_eq!(actual.as_ref(), expected, "branch {branch}, hour {hour}");
                assert_aliases(&actual);
                assert_division_handles(&actual);
                assert_relations(&actual);
            }
            other => panic!("readable branch {branch}, hour {hour}: {other:?}"),
        }
    });
    let missing: Vec<_> = campaign.missing.iter().collect();
    for_each_parallel(&missing, |&(&(branch, hour), expected_reason)| {
        let result = replay.seek(branch, hour).unwrap();
        match result {
            SeekResult::Gap {
                branch: actual_branch,
                hour: actual_hour,
                reason,
            } => {
                assert_eq!((actual_branch, actual_hour), (branch, hour));
                assert!(
                    reason.contains(expected_reason),
                    "{reason:?} lacks {expected_reason:?}"
                );
            }
            other => panic!("lost branch {branch}, hour {hour} was invented: {other:?}"),
        }
    });
}

/// Every table of the campaign, read and reconstructed in selections of 32
/// tables, equals the whole decoder's reconstruction at every hour: rows,
/// availability, sample hours, metrics and which hours share a table's
/// allocation, with every delta applied to rows no reader holds. Every other
/// table is the one unselected view, which each block's decoder shares anew
/// only at its first hour and then moves on.
#[test]
fn selected_tables_reconstruct_as_the_whole_decoder_in_every_codec() {
    use crate::frame::{SharedView, TABLE_COUNT, ViewIdentity, count_unselected, not_selected};
    let campaign = synthesize();
    let first = Replay::from_recovered(CAMPAIGN.to_vec(), &campaign.segments[..2]).unwrap();
    let replay = first.append_recovered(&campaign.segments[2..]).unwrap();
    let unselected = not_selected();
    for block in &replay.index {
        let mut whole = FrameDecoder::new();
        let reconstructed: Vec<DecodedFrame> = replay
            .read_block(block)
            .unwrap()
            .iter()
            .map(|frame| whole.apply(frame).unwrap())
            .collect();
        for selection in TableId::ALL.chunks(32) {
            let frames = replay.read_block_selected(block, selection).unwrap();
            assert_eq!(frames.len(), reconstructed.len());
            let mut decoder = FrameDecoder::for_tables(selection);
            let ((), shares) = count_unselected(|| {
                let mut before: Option<Vec<ViewIdentity>> = None;
                for (at, (frame, expected)) in frames.iter().zip(&reconstructed).enumerate() {
                    let actual = decoder.advance(frame).unwrap();
                    assert_eq!(
                        (actual.branch, actual.hour),
                        (expected.branch, expected.hour)
                    );
                    assert_eq!(actual.metrics, expected.metrics);
                    for table in TableId::ALL {
                        let index = table.index();
                        let Some(position) = selection.iter().position(|t| *t == table) else {
                            assert!(SharedView::ptr_eq(&actual.tables[index], &unselected));
                            continue;
                        };
                        assert_eq!(
                            actual.tables[index], expected.tables[index],
                            "{table:?} at hour {}",
                            expected.hour
                        );
                        assert_eq!(
                            actual.tables[index].sampled_at(),
                            expected.tables[index].sampled_at()
                        );
                        if let Some(before) = &before {
                            assert_eq!(
                                before[position].is(&actual.tables[index]),
                                SharedView::ptr_eq(
                                    &reconstructed[at - 1].tables[index],
                                    &expected.tables[index]
                                ),
                                "{table:?} shares the hour before's view in one reader only at {}",
                                expected.hour
                            );
                        }
                    }
                    before = Some(
                        selection
                            .iter()
                            .map(|table| actual.tables[table.index()].identity())
                            .collect(),
                    );
                }
            });
            assert_eq!(shares, TABLE_COUNT - selection.len());
        }
    }
}

/// The written layout's recording of the campaign, and its appended sessions,
/// packed on 1, 2 and many threads: blocks follow sessions, branches, spool
/// keyframes and the file's earlier keyframes, and every table changes in
/// every form, so a block whose bytes depended on another's packing differs.
#[test]
fn the_campaign_packs_to_the_sequential_bytes_on_any_worker_count() {
    use crate::container::PACK_WORKERS;
    let campaign = synthesize();
    let written: Vec<_> = [1, 2, parallel_budget().max(3)]
        .into_iter()
        .map(|workers| {
            PACK_WORKERS.set(Some(workers));
            let failed = |error: io::Error| panic!("{workers} workers: {error}");
            let first = Replay::from_recovered(CAMPAIGN.to_vec(), &campaign.segments[..2])
                .unwrap_or_else(failed);
            let appended = first
                .append_recovered(&campaign.segments[2..])
                .unwrap_or_else(failed);
            (workers, first.bytes().to_vec(), appended.bytes().to_vec())
        })
        .collect();
    PACK_WORKERS.set(None);
    for (workers, first, appended) in &written[1..] {
        assert!(
            (first, appended) == (&written[0].1, &written[0].2),
            "{workers} workers"
        );
    }
}

/// Each cold seek decodes its block independently, so the checks share no state
/// and can run in parallel; a failing check still panics the test.
fn for_each_parallel<T: Sync>(items: &[T], check: impl Fn(&T) + Sync) {
    let workers = parallel_budget().clamp(1, items.len().max(1));
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let check = &check;
            scope.spawn(move || items.iter().skip(worker).step_by(workers).for_each(check));
        }
    });
}

/// Respect the caller's thread limits on a shared machine: `--test-threads`,
/// then `RUST_TEST_THREADS`, then `CARGO_BUILD_JOBS`, otherwise the core count.
fn parallel_budget() -> usize {
    let parse = |value: &str| value.parse::<usize>().ok().filter(|&n| n > 0);
    let arguments: Vec<String> = std::env::args().collect();
    let flag = arguments.iter().enumerate().find_map(|(i, argument)| {
        match argument.strip_prefix("--test-threads") {
            Some("") => arguments.get(i + 1).and_then(|value| parse(value)),
            Some(rest) => rest.strip_prefix('=').and_then(parse),
            None => None,
        }
    });
    flag.or_else(|| {
        ["RUST_TEST_THREADS", "CARGO_BUILD_JOBS"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok().as_deref().and_then(parse))
    })
    .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |cores| cores.get()))
}

#[test]
fn strategic_action_acceptance_refreshes_operand_only_changes_and_rejects_missing() {
    let make = |hour, inputs| DecodedFrame {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            if index == TableId::DivisionMovement.index() {
                TableView::Complete(TableRows::DivisionMovement(vec![
                    crate::rows::DivisionMovementRow {
                        division: DIVISION_A,
                        path_total: 1,
                        path: vec![7],
                        strategic_redeployment: 1,
                        strategic_action: inputs,
                        ..Default::default()
                    },
                ]))
                .into()
            } else {
                TableView::Unavailable {
                    reason: "fixture has no sample".into(),
                }
                .into()
            }
        }),
    };
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for (hour, value) in [0, i64::MIN, i64::MAX, 0].into_iter().enumerate() {
        let inputs =
            crate::rows::StrategicAction::Available(crate::rows::StrategicRedeploymentInputs {
                state_modifier: Some(0),
                friendly_state_modifier: Some(value),
                ..Default::default()
            });
        let frame = make(hour as i32, crate::rows::Recorded::Value(inputs));
        let encoded = encoder.encode(frame, hour == 0).unwrap();
        let decoded = decoder.apply(&encoded).unwrap();
        assert_eq!(
            decoded.complete::<tables::DivisionMovement>().unwrap()[0].strategic_action,
            crate::rows::Recorded::Value(inputs)
        );
    }
    let withheld = crate::rows::StrategicAction::Withheld(
        crate::rows::StrategicActionFailure::CountryModifier,
    );
    let encoded = encoder
        .encode(make(4, crate::rows::Recorded::Value(withheld)), false)
        .unwrap();
    let decoded = decoder.apply(&encoded).unwrap();
    assert_eq!(
        decoded.complete::<tables::DivisionMovement>().unwrap()[0].strategic_action,
        crate::rows::Recorded::Value(withheld)
    );
    assert!(
        encoder
            .encode(
                make(
                    5,
                    crate::rows::Recorded::Value(crate::rows::StrategicAction::Unreached)
                ),
                false
            )
            .is_err()
    );
}

#[test]
fn table_encoded_sizes_match_all_tables_and_update_forms() {
    use crate::{codec::Decoder, frame::Frame};

    let check = |frame: &Frame, form: &str, tick| {
        let measured = frame.encode_with_sizes(&mut Vec::new()).unwrap();
        let estimated = frame.table_encoded_sizes().unwrap();
        for table in TableId::ALL {
            assert_eq!(
                estimated[table.index()],
                measured[table.index()],
                "{table:?} {form} tick {tick}"
            );
        }
    };
    for tick in 0..4 {
        let source = source_with_faults(tick, 1, 1000, 0, false);
        let snapshot = FrameEncoder::new().encode(source, true).unwrap();
        // Retired tables are not encoded; the rest each hold a snapshot.
        let recorded = || {
            TableId::ALL
                .into_iter()
                .filter(|&table| crate::frame::current_absence(table).is_none())
        };
        for table in recorded() {
            assert!(
                matches!(&snapshot.tables[table.index()], TableUpdate::Snapshot(rows) if !rows.is_empty()),
                "{table:?}"
            );
        }
        check(&snapshot, "snapshot", tick);
        for delete in [false, true] {
            let mut delta = snapshot.clone();
            delta.keyframe = false;
            for table in recorded() {
                let TableUpdate::Snapshot(mut rows) = snapshot.tables[table.index()].clone() else {
                    unreachable!()
                };
                let deleted = if delete {
                    let key = rows.iter().last().unwrap().key();
                    rows.truncate(rows.len() - 1);
                    vec![key]
                } else {
                    Vec::new()
                };
                let mut bytes = Vec::new();
                rows.encode_changes(&TableRows::empty(table), &mut bytes, &mut Vec::new())
                    .unwrap();
                let mut input = Decoder::new(&bytes);
                let count = input.u32().unwrap();
                let mut previous_key = 0;
                let changed = (0..count)
                    .map(|_| {
                        let row =
                            crate::rows::decode_delta(table, previous_key, &mut input).unwrap();
                        previous_key = row.key;
                        row
                    })
                    .collect();
                assert_eq!(input.u32().unwrap(), 0); // Empty baseline has no deletions.
                input.finish().unwrap();
                delta.tables[table.index()] = TableUpdate::Delta {
                    baseline: 999,
                    changed,
                    deleted,
                };
            }
            check(
                &delta,
                if delete {
                    "delta with deletion"
                } else {
                    "delta"
                },
                tick,
            );
        }
        let mut truncated = snapshot;
        truncated.keyframe = false;
        for table in recorded() {
            let update = &mut truncated.tables[table.index()];
            let TableUpdate::Snapshot(rows) = update else {
                unreachable!()
            };
            let mut rows = rows.clone();
            rows.truncate(rows.len() - 1);
            *update = TableUpdate::Truncated { rows, missing: 1 };
        }
        check(&truncated, "truncated", tick);
    }
}
