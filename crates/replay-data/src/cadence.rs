
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
    fn hourly_writer_audit_covers_every_active_table_and_reconciles_its_totals() {
        use std::collections::BTreeSet;
        let audit = include_str!("../cadence-audit.md");
        let (summary, detail) = audit.split_once("## Tables").unwrap();
        let rows: Vec<_> = detail
            .lines()
            .filter(|line| line.starts_with("| ") && !line.starts_with("| Table |"))
            .map(|line| {
                let mut columns = line.split('|').skip(1).map(str::trim);
                (
                    columns.next().unwrap(),
                    columns.next().unwrap().split(';').next().unwrap(),
                )
            })
            .collect();
        let recorded: BTreeSet<_> = rows.iter().map(|row| row.0.to_owned()).collect();
        let expected: BTreeSet<_> = TableId::ALL
            .into_iter()
            .filter(|table| !crate::rows::compatibility::RETIRED_TABLES.contains(table))
            .filter(|&table| cadence(table) == Cadence::Hourly)
            .map(|table| format!("{table:?}"))
            .collect();
        assert_eq!(rows.len(), recorded.len(), "duplicate audited table");
        assert!(
            recorded == expected,
            "missing: {:?}; extra: {:?}",
            expected.difference(&recorded).collect::<Vec<_>>(),
            recorded.difference(&expected).collect::<Vec<_>>()
        );
        let declared = |class: &str| {
            summary
                .lines()
                .find_map(|line| {
                    let columns: Vec<_> = line.split('|').map(str::trim).collect();
                    (columns.get(1) == Some(&class)).then(|| columns[3].parse::<usize>().unwrap())
                })
                .unwrap()
        };
        let mut total = 0;
        for class in ["hourly tick", "any hour", "hook log", "load", "recorder"] {
            let count = rows.iter().filter(|row| row.1 == class).count();
            assert_eq!(count, declared(class), "{class}");
            total += count;
        }
        assert_eq!(total, rows.len(), "unknown writer class");
        assert_eq!(total, declared("total"));
    }

    #[test]
    fn doctrine_cadence_audit_covers_each_table_and_reconciles_totals() {
        let audit = include_str!("../cadence-audit.md");
        let (summary, detail) = audit.split_once("## Tables").unwrap();
        let rows: Vec<_> = detail
            .lines()
            .filter(|line| line.starts_with("| ") && !line.starts_with("| Table |"))
            .collect();
        for table in [
            TableId::FactionAdvisorModifiers,
            TableId::FactionAccounts,
            TableId::FactionGoalStatus,
            TableId::FactionGoalEntries,
            TableId::FactionProjection,
            TableId::FactionResources,
            TableId::DoctrineCountries,
            TableId::DoctrineFolders,
            TableId::DoctrineTracks,
            TableId::DoctrineLeaderMastery,
            TableId::DoctrineCostReductions,
            TableId::DoctrineMasteryGrants,
            TableId::DoctrineEquipmentBonuses,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly);
            let prefix = format!("| {table:?} |");
            assert_eq!(
                rows.iter()
                    .filter(|line| line.starts_with(prefix.as_str()))
                    .count(),
                1,
                "{table:?}"
            );
        }
        let mut total = 0;
        for class in ["hourly tick", "any hour", "hook log", "load", "recorder"] {
            let count = rows
                .iter()
                .filter(|line| {
                    line.split('|')
                        .nth(2)
                        .unwrap()
                        .trim()
                        .split(';')
                        .next()
                        .unwrap()
                        == class
                })
                .count();
            let prefix = format!("| {class} |");
            let declared = summary
                .lines()
                .find(|line| line.starts_with(prefix.as_str()))
                .unwrap()
                .split('|')
                .nth(3)
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            assert_eq!(declared, count, "{class}");
            total += count;
        }
        let declared = summary
            .lines()
            .find(|line| line.starts_with("| total |"))
            .unwrap()
            .split('|')
            .nth(3)
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        assert_eq!(total, rows.len());
        assert_eq!(declared, total);
    }

    #[test]
    fn hourly_cadence_audit_covers_active_tables_and_reconciles_totals() {
        use std::collections::{BTreeMap, BTreeSet};
        let audit = include_str!("../cadence-audit.md");
        let (summary, inventory) = audit
            .split_once("| Table | Writers | Writer evidence |")
            .expect("hourly table inventory");
        let expected: BTreeSet<_> = TableId::ALL
            .iter()
            .filter(|&&table| cadence(table) == Cadence::Hourly && should_walk(table, 0, true))
            .map(|table| format!("{table:?}"))
            .collect();
        let mut listed = BTreeSet::new();
        let mut counts = BTreeMap::new();
        for row in inventory.lines().filter(|line| line.starts_with("| ")) {
            let columns: Vec<_> = row.split('|').map(str::trim).collect();
            let name = columns[1];
            assert!(
                expected.contains(name),
                "non-hourly or unknown table {name}"
            );
            assert!(listed.insert(name.to_owned()), "duplicate table {name}");
            assert!(!columns[3].is_empty(), "missing writer evidence for {name}");
            let class = columns[2].split(';').next().unwrap();
            assert!(
                ["hourly tick", "any hour", "hook log", "load", "recorder"].contains(&class),
                "unknown writer class for {name}: {class}"
            );
            *counts.entry(class).or_insert(0_usize) += 1;
        }
        assert_eq!(
            listed, expected,
            "audit must cover every active hourly table"
        );
        counts.insert("total", listed.len());
        for (class, count) in counts {
            let prefix = format!("| {class} |");
            let row = summary
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap();
            let recorded = row
                .split('|')
                .nth(3)
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            assert_eq!(recorded, count, "stale {class} audit total");
        }
    }

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
    fn each_cadence_refreshes_its_count_per_day_and_full_hours_walk_everything_recorded() {
        for base in [0, 1, 60_820_531, i32::MAX - 24] {
            for table in TableId::ALL {
                let hits = (base..base + 24).filter(|h| is_refresh(table, *h)).count();
                let expected = match cadence(table) {
                    Cadence::Hourly | Cadence::Sliced => 24,
                    Cadence::Daily(_) => 1,
                    Cadence::Static => 0,
                };
                assert_eq!(hits, expected, "{table:?} near {base}");
                assert_eq!(
                    should_walk(table, base, true),
                    table
                        .last_change()
                        .present(crate::format::FormatVersion::CURRENT.version()),
                    "{table:?}"
                );
            }
        }
    }

    #[test]
    fn map_tables_are_hourly_and_families_share_a_slot() {
        for table in [
            TableId::Clock,
            TableId::Characters,
            TableId::UnitLeaders,
            TableId::OrdersGroupCounterInputs,
            TableId::Provinces,
            TableId::States,
            TableId::Divisions,
            TableId::DivisionMovement,
            TableId::DivisionStatus,
            TableId::NewDivisionEquipment,
            TableId::LandTheatres,
            TableId::FactionTheatres,
            TableId::RadarCoverage,
            TableId::NavalRegionalMines,
            TableId::NavalAccidentReports,
            TableId::RelationLinks,
            TableId::DivisionTemplateIcons,
            TableId::CountryVisibility,
            TableId::StateVisibility,
            TableId::ShipModels,
            TableId::ProvinceSupply,
            TableId::TheatreAreas,
            TableId::WeatherProvinces,
            TableId::LandCombatDivisions,
            TableId::LandCombatSides,
            TableId::AirBaseRegistrations,
            TableId::ActiveAbilities,
            TableId::StateBuildings,
            TableId::ProvinceBuildings,
            TableId::RaidTargetStatuses,
            TableId::RaidCountryInputs,
            TableId::AceRangeInputs,
            TableId::StockpileArchetypes,
            TableId::DivisionRetreatBans,
            TableId::CountryLeaderRoles,
            TableId::AdvisorRoles,
            TableId::UnitLeaderTraits,
            TableId::Scientists,
            TableId::ScientistSkills,
            TableId::Operations,
            TableId::OperationAssignments,
            TableId::CharacterRoleTraits,
            TableId::CharacterOperatives,
            TableId::CountryRetiredCharacters,
            TableId::ScriptFlags,
            TableId::ScriptVariables,
            TableId::ActiveIdeas,
            TableId::PowerBalances,
            TableId::PowerBalanceMembers,
            TableId::PowerBalanceRanges,
            TableId::PowerBalanceOverrides,
            TableId::IntelligenceAgencies,
            TableId::CountryFactories,
            TableId::CountryLocalSubunits,
            TableId::DivisionTemplates,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
            assert_eq!(refresh_text(table), "hourly");
        }
        assert_eq!(cadence(TableId::HiredAdvisors), Cadence::Hourly);
        assert_eq!(refresh_slot(TableId::AirAces), Some(21));
        for family in [
            &[
                TableId::ProjectPrograms,
                TableId::ProjectBreakthroughs,
                TableId::ScientistDaily,
            ][..],
            &[TableId::Projects, TableId::ProjectHistory][..],
            &[TableId::RelationOpinions, TableId::RelationTuples][..],
            &[
                TableId::VariantDefinitions,
                TableId::VariantModules,
                TableId::VariantStats,
                TableId::AvailableEquipmentVariants,
                TableId::CountryUnitBonuses,
            ],
            &[TableId::DivisionEquipment, TableId::ArmyUpgradeGroups],
            &[
                TableId::ProductionLines,
                TableId::CountryProductionModifiers,
                TableId::ProductionLineModifiers,
                TableId::IndustrialOrganisations,
                TableId::IndustrialOrganisationHistory,
                TableId::IndustrialOrganisationDetails,
                TableId::IndustrialOrganisationPolicies,
                TableId::ProductionLineOutput,
                TableId::RocketProductionOutput,
            ],
            &[
                TableId::BuildingConstruction,
                TableId::RailwayConstruction,
                TableId::ConstructionLineOutput,
            ],
        ] {
            let shared = cadence(family[0]);
            assert_ne!(shared, Cadence::Hourly);
            assert!(family.iter().all(|t| cadence(*t) == shared), "{family:?}");
        }
        assert_eq!(cadence(TableId::RelationTuples), Cadence::Sliced);
        assert_eq!(refresh_slot(TableId::RelationTuples), None);
        assert_eq!(refresh_slot(TableId::ProjectPrograms), Some(4));
        assert_eq!(cadence(TableId::Projects), Cadence::Sliced);
        assert_eq!(cadence(TableId::LexicalTokens), Cadence::Static);
        assert_eq!(refresh_text(TableId::LexicalTokens), "static");
        assert!(is_staggered(TableId::LexicalTokens));
        assert_eq!(refresh_slot(TableId::LexicalTokens), None);
        assert_eq!(cadence(TableId::ModifierDefinitions), Cadence::Static);
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
    fn world_inputs_are_read_every_hour() {
        for table in [
            TableId::ProvinceNames,
            TableId::NavalRegionAccess,
            TableId::WorldTension,
            TableId::ThreatSources,
            TableId::ThreatCountryAggregates,
            TableId::DynamicModifiers,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
            assert!((-24..24).all(|hour| should_walk(table, hour, false)));
        }
        assert_eq!(refresh_slot(TableId::ResearchSlots), Some(11));
        assert_eq!(refresh_slot(TableId::PreferredTactics), Some(18));
    }

    #[test]
    fn relations_are_read_once_a_day_one_owner_slice_per_hour() {
        for table in [
            TableId::RelationOpinions,
            TableId::RelationTuples,
            TableId::CombatLogEquipment,
        ] {
            assert!(is_sliced(table), "{table:?}");
            assert!(!is_staggered(table), "{table:?}");
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

    #[test]
    fn event_ledger_tables_drain_each_hour() {
        for table in [
            TableId::AirLossEvents,
            TableId::BuildingBombingDamage,
            TableId::AirDailyRoster,
            TableId::NuclearStrikes,
            TableId::MapEvents,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly);
            assert_eq!(refresh_slot(table), None);
            assert!((-48..48).all(|hour| should_walk(table, hour, false)));
        }
    }

    #[test]
    fn occupation_province_hover_is_acquired_each_hour() {
        assert_eq!(cadence(TableId::StateResistance), Cadence::Hourly);
        assert!((-48..48).all(|hour| should_walk(TableId::StateResistance, hour, false)));
        assert_eq!(cadence(TableId::StateGarrisons), Cadence::Daily(3));
    }
    #[test]
    fn owner_daily_progress_exception_is_limited_to_approved_items() {
        assert_eq!(
            cadence(TableId::UnitLeaderTraitExperience),
            Cadence::Daily(12)
        );
        let slots = [
            COMMANDER_PROGRESS_SLOT,
            COMBAT_LOG_NUMERIC_SLOT,
            DIVISION_STATUS_DAILY_SLOT,
        ];
        assert!(slots.iter().all(|&slot| slot != 0 && slot < STAGGER_PERIOD as u32));
        assert!(slots[0] != slots[1] && slots[0] != slots[2] && slots[1] != slots[2]);
        for hour in -48..48 {
            assert_eq!(
                should_walk(TableId::UnitLeaderTraitExperience, hour, false),
                hour.rem_euclid(24) == 12
            );
            assert!(should_walk(TableId::UnitLeaderTraitExperience, hour, true));
            assert_eq!(
                Cadence::Daily(DIVISION_STATUS_DAILY_SLOT).is_refresh(hour),
                hour.rem_euclid(24) == 18
            );
            assert_eq!(
                Cadence::Daily(COMBAT_LOG_NUMERIC_SLOT).is_refresh(hour),
                hour.rem_euclid(24) == 6
            );
            assert!(should_walk(TableId::DivisionStatus, hour, false));
        }
        for table in [
            TableId::UnitLeaders,
            TableId::CombatLogObservers,
            TableId::IntelContributions,
            TableId::FactionAccounts,
            TableId::WarParticipation,
            TableId::States,
            TableId::TaskForceCounterInputs,
            TableId::AirRegionCombat,
            TableId::DivisionStatus,
            TableId::DivisionLandInputs,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
        }
    }

    #[test]
    fn combat_log_hourly_tables_have_writer_certificate_audit_entries() {
        let audit = include_str!("../cadence-audit.md");
        for table in [
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
        ] {
            assert_eq!(cadence(table), Cadence::Hourly);
            assert!(
                audit
                    .lines()
                    .any(|line| line.starts_with(&format!("| {table:?} |"))),
                "{table:?} has no audit evidence"
            );
        }
        let rows = audit
            .lines()
            .filter(|line| {
                line.split('|').nth(2).map(str::trim).is_some_and(|writer| {
                    ["hourly tick", "any hour", "hook log", "load", "recorder"]
                        .iter()
                        .any(|class| {
                            writer == *class
                                || writer
                                    .strip_prefix(*class)
                                    .is_some_and(|rest| rest.starts_with(';'))
                        })
                })
            })
            .count();
        assert!(audit.contains(&format!("| total | | {rows} |")));
    }
}
