
use crate::rows::TableId;

pub const COMMANDER_PROGRESS_SLOT: u32 = 12;
pub const COMBAT_LOG_NUMERIC_SLOT: u32 = 6;
pub const DIVISION_STATUS_DAILY_SLOT: u32 = 18;

pub const STAGGER_PERIOD: i32 = 24;
pub const SKIPPED_REASON: &str = "staggered off-hours; carry forward";
pub const STAGGER_LIMITATION: &str =
    "refreshed on its scheduled slot; off-slot hours hold the last refresh";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    Hourly,
    Daily(u32),
    Sliced,
    Static,
}

pub const fn slice_of(owner: u32) -> u32 {
    owner % STAGGER_PERIOD as u32
}

pub fn slice_active(owner: u32, hour: crate::Hour, force_full: bool) -> bool {
    force_full || slice_of(owner) == hour.rem_euclid(STAGGER_PERIOD) as u32
}

pub const fn cadence(table: TableId) -> Cadence {
    use Cadence::{Daily, Hourly, Sliced, Static};
    match table {
        TableId::FactionGoalStatus | TableId::FactionGoalEntries
        | TableId::FactionRules | TableId::FactionPrograms
        | TableId::FactionManpower | TableId::FactionUnlocks | TableId::FactionVariables | TableId::FactionAccounts | TableId::FactionResearch | TableId::FactionMinimumUpgrades | TableId::FactionDoctrine | TableId::FactionProjection | TableId::FactionResources | TableId::FactionAdvisorModifiers => Hourly,
        TableId::AirGroundCrews => Hourly,
        TableId::AirTransferInputs => Hourly,
        TableId::NavalAccidentReports => Hourly,
        TableId::NavalRegionalMines => Hourly,
        TableId::NavalDominanceTaskForceRanges => Hourly,
        TableId::NavalPatrolMissionInputs => Hourly,
        TableId::NavalResultSources => Hourly,
        TableId::CountryTemplateState => Hourly,
        TableId::CountryPowerDaily | TableId::CountryPowerDailyCosts => Daily(0),
        TableId::CountryPowerChanges | TableId::GovernmentChanges
        | TableId::PoliticsCaptureCoverage => Hourly,
        TableId::AgencyOperationBuffers | TableId::AgencyUpgradeLevels | TableId::OperationCompletions => Hourly,
        TableId::CountryFactories => Cadence::Hourly,
        TableId::CountryGameScoreInputs
        | TableId::AirWingGameScoreStats
        | TableId::GameScoreMultipliers => Hourly,
        TableId::AvailableAdvisors => Sliced,
        TableId::SupplyNodes | TableId::TrainInventory => Sliced,
        TableId::CountryExperienceGains => Hourly,
        TableId::EventFeedHistory => Hourly,
        TableId::PeaceSettlements | TableId::PeaceBids => Cadence::Hourly,
        TableId::DivisionCensus => Cadence::Hourly,
        TableId::NavalMissionLedgers | TableId::NavalMissionTaskForces => Hourly,
        TableId::ResearchInputs | TableId::ResearchAppliedBonuses | TableId::ResearchModifiers => {
            Sliced
        }
        TableId::RaidAirSites => Sliced,
        TableId::TradeRoutes => Sliced,
        TableId::StateCores => Sliced,
        TableId::StateBuildingInputs => Daily(14),
        TableId::StateResourceInputs
        | TableId::StateResources
        | TableId::StateDynamicModifiers
        | TableId::CountryBuildingResources => Sliced,
        TableId::ResearchTechnologies | TableId::TechnologyDefinitions => Sliced,
        TableId::AirCountryRelations | TableId::AirNavalPatrolHistories => Sliced,
        TableId::CountryStabilityInputs | TableId::CountryStabilityLines => Sliced,
        TableId::NavalMissionCountries | TableId::NavalMissionCommanders => Sliced,
        TableId::LexicalTokens | TableId::AirSortieModifierMetadata => Static,
        TableId::WarParticipation | TableId::WarCapturedProvinces | TableId::WarLendLeaseHistories | TableId::WarOwnedRelations | TableId::WarRelationGoals | TableId::WarOverviewCountries | TableId::WarNameRules
        | TableId::CountryWargoals | TableId::WargoalTargets | TableId::WargoalJustifications => Cadence::Hourly,
        TableId::ModifierDefinitions => Static,
        TableId::StrategicRegionWeights => Static,
        TableId::RelationOpinions | TableId::RelationTuples => Sliced,
        TableId::CombatLogEquipment => Sliced,
        TableId::UnitLeaderTraitExperience => Daily(COMMANDER_PROGRESS_SLOT),
        TableId::CountryIntel => Daily(0),
        TableId::LogisticsEquipment => Daily(0),
        TableId::NavalSupremacy => Daily(0),
        TableId::Projects | TableId::ProjectHistory => Sliced,
        TableId::ProjectPrograms | TableId::ProjectBreakthroughs | TableId::ScientistDaily => Daily(4),
        TableId::DivisionEquipment
        | TableId::ArmyUpgradeGroups
        | TableId::ArmyUpgradeRequests
        | TableId::ArmyUpgradeDeliveries
        | TableId::ArmyUpgradeProduced => Daily(2),
        TableId::StateBaseResources
        | TableId::BuildingConstruction
        | TableId::RailwayConstruction
        | TableId::ConstructionLineOutput
        | TableId::OccupationSettings
        | TableId::StateOccupationLaws
        | TableId::StateGarrisons
        | TableId::StateAssignedManpower
        | TableId::CountryOccupationReserveManpower
        | TableId::CountryCollaboration => Daily(3),
        TableId::AirStatistics => Daily(5),
        TableId::DecisionCounts | TableId::Decisions | TableId::DecisionRandomItems => Daily(6),
        TableId::ProductionLines
        | TableId::CountryProductionModifiers
        | TableId::ProductionLineModifiers
        | TableId::IndustrialOrganisations
        | TableId::IndustrialOrganisationHistory
        | TableId::IndustrialOrganisationDetails
        | TableId::IndustrialOrganisationPolicies
        | TableId::ProductionLineOutput
        | TableId::RocketProductionOutput
        | TableId::Stockpiles
        | TableId::ProductionLicenses
        | TableId::DeploymentConveyors
        | TableId::DeploymentLines
        | TableId::DeploymentManpower
        | TableId::DeploymentEquipment
        | TableId::ProductionLicenseRecords => Daily(8),
        TableId::ResourceDefinitions => Hourly,
        TableId::ResearchSlots => Daily(11),
        TableId::LandTheatreGroups
        | TableId::LandTheatreMembers
        | TableId::AirTheatres
        | TableId::AirTheatreGroups
        | TableId::AirTheatreMembers => Daily(13),
        TableId::IntelNetworks | TableId::IntelNetworkSummaries => Daily(0),
        TableId::OperativeMemberships
        | TableId::CapturedOperativeReferences
        | TableId::WingCumulative => Daily(16),
        TableId::PowerBalances
        | TableId::PowerBalanceMembers
        | TableId::PowerBalanceRanges
        | TableId::PowerBalanceOverrides => Cadence::Hourly,
        TableId::CountryFuel
        | TableId::CountryFuelConsumers
        | TableId::FuelDemandParts
        | TableId::CountryExperience
        | TableId::HubMotorization
        | TableId::DisruptedSupply
        | TableId::ForeignHomebases
        | TableId::PurchaseContracts => Daily(17),
        TableId::PreferredTactics => Daily(18),
        TableId::ProvinceNames
        | TableId::NavalRegionAccess
        | TableId::WorldTension
        | TableId::ThreatSources
        | TableId::ThreatCountryAggregates
        | TableId::DynamicModifiers => Hourly,
        TableId::SunkShips => Daily(19),
        TableId::Focus | TableId::CompletedFocuses => Sliced,
        TableId::OperationTokens => Daily(20),
        TableId::IntelligenceAgencies => Cadence::Hourly,
        TableId::AirAces | TableId::PeaceConferences => Daily(21),
        TableId::IncomingDiplomaticActions
        | TableId::TechSharingGroups
        | TableId::TechSharingMembers
        | TableId::VariantDefinitions
        | TableId::VariantModules
        | TableId::VariantStats
        | TableId::VariantUpgrades
        | TableId::AvailableEquipmentVariants
        | TableId::CountryUnitBonuses => Daily(22),
        TableId::SunkConvoys | TableId::LendLeaseEquipment => Daily(23),
        TableId::Operations | TableId::OperationAssignments => Hourly,
        TableId::SimulationFeatures => Cadence::Hourly,
        TableId::VariantChanges
        | TableId::VariantBonusSnapshots
        | TableId::DesignCountryModifiers
        | TableId::DesignCountryTemplates => Cadence::Hourly,
        TableId::DesignObserverContext | TableId::DesignOrganisationLiveness => Cadence::Hourly,
        TableId::LocalPlayer => Cadence::Hourly,
        TableId::Cryptology => Cadence::Hourly,
        TableId::GlobalSavedEventTargets => Cadence::Hourly,
        TableId::EquipmentDestruction => Cadence::Hourly,
        TableId::AirLossEvents | TableId::BuildingBombingDamage | TableId::AirDailyRoster => Cadence::Hourly,
        TableId::NuclearStrikes => Cadence::Hourly,
        TableId::MapEvents => Cadence::Hourly,
        TableId::PowerBalanceModifierBreakdowns => Cadence::Hourly,
        TableId::ScriptFlags
        | TableId::ScriptVariables
        | TableId::ActiveIdeas
        | TableId::TimedIdeas => {
            Cadence::Hourly
        }
        _ => Cadence::Hourly,
    }
}

pub const fn refresh_slot(table: TableId) -> Option<u32> {
    match cadence(table) {
        Cadence::Daily(slot) => Some(slot),
        Cadence::Hourly | Cadence::Sliced | Cadence::Static => None,
    }
}

pub const fn is_sliced(table: TableId) -> bool {
    matches!(cadence(table), Cadence::Sliced)
}

pub const fn is_staggered(table: TableId) -> bool {
    matches!(cadence(table), Cadence::Daily(_) | Cadence::Static)
}

pub fn is_refresh(table: TableId, hour: crate::Hour) -> bool {
    cadence(table).is_refresh(hour)
}

pub const EARLIER_CADENCES: [(TableId, u16, Cadence); 25] = {
    use crate::rows::compatibility::CADENCE_FORMAT as F;
    use Cadence::{Daily, Hourly, Sliced};
    [
        (TableId::AceRangeInputs, F, Sliced),
        (TableId::AirRegionDamage, F, Daily(7)),
        (TableId::AirRegionHistory, F, Daily(7)),
        (TableId::AvailableAdvisors, F, Hourly),
        (TableId::CompletedFocuses, F, Daily(19)),
        (TableId::CountryLocalSubunits, F, Daily(2)),
        (TableId::DivisionRetreatBans, F, Sliced),
        (TableId::DivisionTemplates, F, Daily(9)),
        (TableId::DynamicModifiers, F, Daily(15)),
        (TableId::Focus, F, Daily(19)),
        (TableId::HiredAdvisors, F, Daily(21)),
        (TableId::NavalRegionAccess, F, Daily(11)),
        (TableId::NavalSupremacy, F, Daily(23)),
        (TableId::ProjectHistory, F, Daily(4)),
        (TableId::Projects, F, Daily(4)),
        (TableId::ProvinceNames, F, Daily(1)),
        (TableId::RaidCountryInputs, F, Sliced),
        (TableId::ResourceDefinitions, F, Daily(10)),
        (TableId::StockpileArchetypes, F, Sliced),
        (TableId::SupplyNodes, F, Hourly),
        (TableId::TheatreAreas, F, Daily(18)),
        (TableId::ThreatCountryAggregates, F, Daily(18)),
        (TableId::ThreatSources, F, Daily(18)),
        (TableId::TrainInventory, F, Hourly),
        (TableId::WorldTension, F, Daily(18)),
    ]
};

pub fn cadence_in(table: TableId, format: crate::format::FormatVersion) -> Cadence {
    EARLIER_CADENCES
        .iter()
        .find(|(changed, until, _)| *changed == table && format.version() < *until)
        .map_or_else(|| cadence(table), |(_, _, earlier)| *earlier)
}

impl Cadence {
    pub fn is_refresh(self, hour: crate::Hour) -> bool {
        match self {
            Cadence::Hourly | Cadence::Sliced => true,
            Cadence::Daily(slot) => hour.rem_euclid(STAGGER_PERIOD) == slot as i32,
            Cadence::Static => false,
        }
    }
}

pub fn should_walk(table: TableId, hour: crate::Hour, force_full: bool) -> bool {
    table
        .last_change()
        .present(crate::format::FormatVersion::CURRENT.version())
        && (force_full || is_refresh(table, hour))
}

pub fn refresh_text(table: TableId) -> &'static str {
    cadence(table).refresh_text()
}

impl Cadence {
    pub fn refresh_text(self) -> &'static str {
        const DAILY: [&str; 24] = [
            "H%24==0", "H%24==1", "H%24==2", "H%24==3", "H%24==4", "H%24==5", "H%24==6", "H%24==7",
            "H%24==8", "H%24==9", "H%24==10", "H%24==11", "H%24==12", "H%24==13", "H%24==14",
            "H%24==15", "H%24==16", "H%24==17", "H%24==18", "H%24==19", "H%24==20", "H%24==21",
            "H%24==22", "H%24==23",
        ];
        match self {
            Cadence::Hourly => "hourly",
            Cadence::Sliced => "one owner slice per hour, merged into the held table",
            Cadence::Daily(slot) => DAILY[slot as usize],
            Cadence::Static => "static",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_tables_are_never_walked() {
        for table in crate::rows::compatibility::RETIRED_TABLES {
            for hour in 0..STAGGER_PERIOD {
                assert!(!should_walk(table, hour, true), "{table:?}");
                assert!(!should_walk(table, hour, false), "{table:?}");
            }
        }
        assert!(should_walk(TableId::UnitLeaderTraits, 0, false));
        assert!(should_walk(TableId::LandCombatDivisions, 0, false));
        assert!(should_walk(TableId::NavalCombatShips, 0, false));
        assert!(should_walk(TableId::AirStatistics, 0, true));
    }

    #[test]
    fn sliced_tables_read_every_owner_once_per_day_and_everything_on_full_hours() {
        for table in [
            TableId::ResearchInputs,
            TableId::RaidAirSites,
            TableId::StateResourceInputs,
            TableId::StateResources,
            TableId::StateDynamicModifiers,
            TableId::CountryBuildingResources,
        ] {
            assert!(is_sliced(table), "{table:?}");
            assert!(!is_staggered(table), "{table:?}");
        }
        assert_eq!(refresh_slot(TableId::ResearchModifiers), None);
        assert!(is_sliced(TableId::CountryStabilityInputs));
        assert!(is_sliced(TableId::CountryStabilityLines));
        for owner in [0, 1, 23, 24, 439, 5_000, u32::MAX] {
            for base in [0, 60_827_795, -48] {
                let hits = (base..base + 24)
                    .filter(|hour| slice_active(owner, *hour, false))
                    .count();
                assert_eq!(hits, 1, "owner {owner} near {base}");
                assert!(slice_active(owner, base, true));
            }
        }
    }

    #[test]
    fn supply_trains_and_advisor_pools_read_each_owner_once_a_day() {
        for table in [
            TableId::SupplyNodes,
            TableId::TrainInventory,
            TableId::AvailableAdvisors,
        ] {
            assert_eq!(cadence(table), Cadence::Sliced, "{table:?}");
            assert!(is_sliced(table) && !is_staggered(table), "{table:?}");
            assert_eq!(refresh_slot(table), None);
            for base in [0, 60_827_795, -48] {
                assert!((base..base + 24).all(|hour| should_walk(table, hour, false)));
            }
        }
        for base in [0, 60_827_795, -48] {
            let mut reads = vec![0_u32; 5_001];
            for hour in base..base + 24 {
                for owner in 1..=5_000_u32 {
                    if slice_active(owner, hour, false) {
                        reads[owner as usize] += 1;
                    }
                    assert!(slice_active(owner, hour, true));
                }
            }
            assert!(reads[1..].iter().all(|&count| count == 1), "near {base}");
        }
        for table in [TableId::AdvisorTokens, TableId::ProvinceSupply] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
        }
    }

    #[test]
    fn a_recording_reads_each_table_at_the_cadence_of_its_format() {
        use crate::format::FormatVersion;
        let previous = FormatVersion::read(crate::rows::compatibility::CADENCE_FORMAT - 1).unwrap();
        assert_eq!(
            cadence_in(TableId::WorldTension, previous),
            Cadence::Daily(18)
        );
        assert_eq!(cadence_in(TableId::Projects, previous), Cadence::Daily(4));
        assert_eq!(
            cadence_in(TableId::NavalSupremacy, previous),
            Cadence::Daily(23)
        );
        assert_eq!(
            cadence_in(TableId::Countries, previous),
            cadence(TableId::Countries)
        );
        for table in TableId::ALL {
            assert_eq!(
                cadence_in(table, FormatVersion::CURRENT),
                cadence(table),
                "{table:?}"
            );
        }
        for (index, &(table, until, earlier)) in EARLIER_CADENCES.iter().enumerate() {
            let next = EARLIER_CADENCES[index + 1..]
                .iter()
                .find(|(other, _, _)| *other == table);
            assert!(next.is_none_or(|&(_, later, _)| later > until), "{table:?}");
            let after = next.map_or_else(|| cadence(table), |&(_, _, cadence)| cadence);
            assert_ne!(earlier, after, "{table:?}");
        }
    }

}
