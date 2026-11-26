use crate::{
    FORMAT_VERSION,
    codec::invalid,
    rows::{
        TableId,
        compatibility::{PROVEN_FROM, RETIRED_FORMAT, RecordKind},
    },
};
use std::io;

pub const GAME_SCORE_DISABLED_REASON: &str = "game score capture disabled at attachment";

#[derive(Debug)]
struct LayoutRefusal(String);

impl std::fmt::Display for LayoutRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LayoutRefusal {}

pub fn is_layout_refusal(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<LayoutRefusal>())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FormatVersion(u16);

impl Default for FormatVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

impl FormatVersion {
    pub const CURRENT: Self = Self(FORMAT_VERSION);

    pub fn read(version: u16) -> io::Result<Self> {
        if version < PROVEN_FROM {
            return Err(invalid(format!(
                "format {version} predates the proven compatibility history (from {PROVEN_FROM})"
            )));
        }
        if version > FORMAT_VERSION {
            return Err(invalid(format!(
                "recording format {version} is newer than the proven compatibility history (through {FORMAT_VERSION})"
            )));
        }
        if version == RETIRED_FORMAT {
            return Err(invalid(format!(
                "format {version} is retired and has no proven published recording contract"
            )));
        }
        let format = Self(version);
        for kind in [
            RecordKind::ReplayHeader,
            RecordKind::ReplayTrailer,
            RecordKind::ReplayFooter,
            RecordKind::Frame,
            RecordKind::Identity,
            RecordKind::SessionStart,
            RecordKind::BranchStart,
            RecordKind::Gap,
            RecordKind::RecoveryNote,
        ] {
            format.require(kind)?;
        }
        Ok(format)
    }

    pub const fn version(self) -> u16 {
        self.0
    }

    pub fn require(self, kind: RecordKind) -> io::Result<()> {
        let change = kind.last_change();
        if self.0 < change.format {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                LayoutRefusal(format!(
                    "{kind:?} layout changed at format {}; this recording is format {}",
                    change.format, self.0
                )),
            ))
        } else {
            Ok(())
        }
    }

    pub fn table_refusal(self, table: TableId) -> Option<String> {
        table.last_change().table_gap(table, self.0)
    }

    pub fn field_gaps(self, table: TableId) -> impl Iterator<Item = String> {
        table
            .last_change()
            .appended
            .iter()
            .filter(move |field| self.0 < field.introduced)
            .map(move |field| {
                format!(
                    "{table:?}.{} not recorded (introduced at format {}; this recording is format {}); other fields decode",
                    field.name, field.introduced, self.0
                )
            })
    }

    pub fn nested_field_gaps(self) -> impl Iterator<Item = String> {
        RecordKind::ALL.into_iter().flat_map(move |kind| {
            kind.last_change()
                .appended
                .iter()
                .filter(move |field| self.0 < field.introduced)
                .map(move |field| {
                    format!(
                        "{} not recorded (introduced at format {}; this recording is format {}); other fields decode",
                        field.name, field.introduced, self.0
                    )
                })
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fields(self, table: TableId) -> usize {
        table.last_change().fields(table, self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_accepts_only_the_proven_horizon() {
        for version in 0..=u16::MAX {
            let result = FormatVersion::read(version);
            if (PROVEN_FROM..=FORMAT_VERSION).contains(&version) && version != RETIRED_FORMAT {
                assert_eq!(result.unwrap().version(), version);
            } else {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            }
        }
    }

    #[test]
    fn chunked_block_contract_does_not_retire_row_or_spool_contracts() {
        let previous = FormatVersion::read(crate::rows::compatibility::CHECKPOINT_FORMAT).unwrap();
        previous.require(RecordKind::Frame).unwrap();
        previous.require(RecordKind::SpoolEnvelope).unwrap();
        let error = previous.require(RecordKind::ReplayBlock).unwrap_err();
        assert!(is_layout_refusal(&error));
        assert!(
            error
                .to_string()
                .contains("ReplayBlock layout changed at format 124")
        );
        FormatVersion::CURRENT
            .require(RecordKind::ReplayBlock)
            .unwrap();
    }

    #[test]
    fn movement_layout_is_introduced_only_in_format_104() {
        let format = FormatVersion::read(104).unwrap();
        for table in [
            TableId::MovementStateTerms,
            TableId::CoreSupplyAccess,
            TableId::CountryUnitSpeedEntries,
            TableId::RiverRelations,
            TableId::DivisionTransportSpeeds,
        ] {
            assert_eq!(table.last_change().introduced, 104);
            assert!(
                FormatVersion::read(103)
                    .unwrap()
                    .table_refusal(table)
                    .is_some()
            );
            assert!(format.table_refusal(table).is_none());
            assert!(FormatVersion::CURRENT.table_refusal(table).is_none());
        }
        assert_eq!(TableId::RegionalConvoyEfficiency as u16, 338);
        for (table, id) in [
            (TableId::MovementStateTerms, 406),
            (TableId::CoreSupplyAccess, 407),
            (TableId::CountryUnitSpeedEntries, 408),
            (TableId::RiverRelations, 409),
            (TableId::DivisionTransportSpeeds, 410),
        ] {
            assert_eq!(table as u16, id);
        }
        assert_eq!(
            TableId::DivisionStatus.last_change().format,
            crate::rows::compatibility::STORED_ONCE_FORMAT
        );
        assert_eq!(TableId::DivisionTemplateIcons.last_change().format, 104);
        let fields: Vec<_> = TableId::DivisionTemplateIcons
            .last_change()
            .appended
            .iter()
            .filter(|field| field.introduced == 104)
            .map(|field| field.name)
            .collect();
        assert_eq!(
            fields,
            vec![
                "manpower_exile",
                "base_width",
                "layout",
                "settings",
                "speed_keys",
            ]
        );
    }

    #[test]
    fn politics_additions_leave_the_published_table_contracts_readable() {
        let previous =
            FormatVersion::read(crate::rows::compatibility::WIDGET_CAPTURE_FORMAT - 1).unwrap();
        for table in [
            TableId::Countries,
            TableId::Characters,
            TableId::OperativeMemberships,
            TableId::CountryAppearance,
        ] {
            assert!(previous.table_refusal(table).is_none(), "{table:?}");
        }
        for table in [
            TableId::CountryPowerDaily,
            TableId::CountryPowerChanges,
            TableId::CountryPowerDailyCosts,
            TableId::GovernmentChanges,
            TableId::PoliticsCaptureCoverage,
        ] {
            assert!(
                previous
                    .table_refusal(table)
                    .unwrap()
                    .contains("not recorded")
            );
            assert!(FormatVersion::CURRENT.table_refusal(table).is_none());
        }
    }

    #[test]
    fn compatibility_refusal_sets_are_exact_for_every_published_format() {
        assert_eq!(
            FORMAT_VERSION,
            crate::rows::compatibility::SURRENDER_INPUTS_FORMAT
        );
        for version in PROVEN_FROM..=FORMAT_VERSION {
            if version == RETIRED_FORMAT {
                assert!(FormatVersion::read(version).is_err());
                continue;
            }
            let format = FormatVersion::read(version).unwrap();
            let mut expected = crate::rows::compatibility::RETIRED_TABLES.to_vec();
            if version < crate::rows::compatibility::DIVISION_DAMAGE_FORMAT {
                expected.extend([TableId::LandDivisionDamage, TableId::DivisionKills]);
            }
            if version < crate::rows::compatibility::RECORDING_SIZE_AUDIT_FORMAT {
                expected.push(TableId::LandCombatResultObservers);
            }
            if version < crate::rows::compatibility::RECORDING_SIZE_FORMAT {
                expected.extend([
                    TableId::FuelAwardRecipients,
                    TableId::ScriptVariableNames,
                    TableId::ScriptVariableValues,
                ]);
            }
            if version < crate::rows::compatibility::LAND_COMBAT_EDGES_FORMAT {
                expected.push(TableId::LandCombatEdges);
            }
            if version < crate::rows::compatibility::ENGAGED_REFRESH_WITNESS_FORMAT {
                expected.push(TableId::CombatEngagementRefreshes);
            }
            if version < crate::rows::compatibility::VARIANT_STAT_WITNESS_FORMAT {
                expected.extend([
                    TableId::VariantStats,
                    TableId::VariantChanges,
                    TableId::RaidSpeedInputs,
                    TableId::RocketProductionOutput,
                ]);
            }
            if version < crate::rows::compatibility::POWER_LEDGER_DAY_LOG_FORMAT {
                expected.push(TableId::PowerSpendDays);
            }
            if version < crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT {
                expected.push(TableId::StrategicRegionWeights);
            }
            if version < crate::rows::compatibility::EXACT_NAVAL_CONDITION_FORMAT {
                expected.push(TableId::NavalCombatShips);
            }
            if version < crate::rows::compatibility::AIR_COMBAT_MENU_FORMAT && version >= 104 {
                expected.push(TableId::AirMissionInputs);
            }
            if version < crate::rows::compatibility::AIR_HISTORY_SLOTS_FORMAT {
                expected.push(TableId::AirNavalPatrolHistories);
            }
            if version < crate::rows::compatibility::RAID_DISPATCHERS_FORMAT {
                expected.push(TableId::RaidPathDistances);
            }
            if version < 107 {
                expected.push(TableId::Ships);
            }
            if version < crate::rows::compatibility::DAILY_PROGRESS_FORMAT {
                expected.extend([
                    TableId::CombatLogObservers,
                    TableId::UnitLeaders,
                    TableId::UnitLeaderTraitExperience,
                ]);
            }
            if version < crate::rows::compatibility::STORED_ONCE_FORMAT {
                expected.extend(crate::rows::compatibility::STORED_ONCE_TABLES);
            }
            if version < crate::rows::compatibility::OCCUPATION_MAP_FORMAT {
                expected.push(TableId::StateResistance);
            }
            if version < 104 {
                expected.extend([
                    TableId::CountryWargoals,
                    TableId::WargoalTargets,
                    TableId::WargoalJustifications,
                ]);
            }
            if version < crate::rows::compatibility::EXPERIENCE_FORMAT {
                expected.extend([
                    TableId::DivisionHistory,
                    TableId::DivisionPanelSources,
                    TableId::DivisionPanelStats,
                ]);
            }
            if version < crate::rows::compatibility::FACTION_STATE_FORMAT {
                expected.extend([
                    TableId::FactionGoalStatus,
                    TableId::FactionGoalEntries,
                    TableId::FactionRules,
                    TableId::FactionPrograms,
                    TableId::FactionManpower,
                    TableId::FactionUnlocks,
                    TableId::FactionVariables,
                    TableId::FactionAccounts,
                    TableId::FactionResearch,
                    TableId::FactionMinimumUpgrades,
                    TableId::FactionDoctrine,
                    TableId::FactionProjection,
                    TableId::FactionResources,
                    TableId::FactionAdvisorModifiers,
                ]);
            }
            if version < crate::rows::compatibility::DOCTRINES_FORMAT {
                expected.extend([
                    TableId::DoctrineFolders,
                    TableId::DoctrineTracks,
                    TableId::DoctrineLeaderMastery,
                    TableId::DoctrineCostReductions,
                    TableId::DoctrineMasteryGrants,
                    TableId::DoctrineEquipmentBonuses,
                    TableId::DoctrineCountries,
                ]);
            }
            if version < crate::rows::compatibility::EVENT_FEED_OCCURRENCE_DELTAS_FORMAT {
                expected.push(TableId::EventFeedHistory);
            }
            if version < 104 {
                expected.extend([
                    TableId::WarParticipation,
                    TableId::WarCapturedProvinces,
                    TableId::WarLendLeaseHistories,
                ]);
            }
            if version <= 101 {
                expected.extend([
                    TableId::NavalModifierTrees,
                    TableId::NavalModifierTreeSamples,
                ]);
            }
            if version < crate::rows::compatibility::AIR_REGION_COMBAT_FORMAT {
                expected.push(TableId::AirRegionCombat);
            }
            if version < crate::rows::compatibility::NUCLEAR_STRIKES_FORMAT {
                expected.push(TableId::NuclearStrikes);
            }
            if version < crate::rows::compatibility::EXACT_COMBAT_FORMAT {
                expected.extend([
                    TableId::LandCombatDivisions,
                    TableId::LandCombatDivisionModifiers,
                ]);
            }
            if version < crate::rows::compatibility::PRIVATE_COPY_ALIASES_FORMAT {
                expected.extend([TableId::Divisions, TableId::ProductionLines]);
            }
            if version < crate::rows::compatibility::FUEL_DEMAND_PARTS_FORMAT {
                expected.push(TableId::FuelDemandParts);
            }
            if version < crate::rows::compatibility::DESIGN_STATE_FORMAT {
                expected.extend([
                    TableId::VariantChanges,
                    TableId::VariantBonusSnapshots,
                    TableId::DesignCountryModifiers,
                    TableId::DesignCountryTemplates,
                    TableId::DesignObserverContext,
                    TableId::DesignOrganisationLiveness,
                ]);
            }
            if version < crate::rows::compatibility::SCRIPTED_MAP_FORMAT {
                expected.push(TableId::MapEvents);
            }
            if version < crate::rows::compatibility::OWNER_TRANSPORT_SPEEDS_FORMAT {
                expected.push(TableId::OwnerTransportSpeeds);
            }
            if version < crate::rows::compatibility::RESERVE_MODIFIER_TREES_FORMAT {
                expected.push(TableId::ReserveModifierTrees);
            }
            if version < crate::rows::compatibility::BATTLE_SOUNDS_FORMAT {
                expected.push(TableId::BattleSounds);
            }
            if version < crate::rows::compatibility::OPERATIVE_MISSIONS_FORMAT {
                expected.push(TableId::OperativeMissions);
            }
            if version < crate::rows::compatibility::AGENCY_OPERATION_BUFFERS_FORMAT {
                expected.push(TableId::AgencyOperationBuffers);
            }
            if version < crate::rows::compatibility::STRATEGIC_OPERATIVES_FORMAT {
                expected.push(TableId::StrategicOperatives);
            }
            if version < crate::rows::compatibility::OPERATION_PREPARATION_FORMAT {
                expected.push(TableId::Operations);
            }
            if version < crate::rows::compatibility::RESEARCH_PROJECT_DETAILS_FORMAT {
                expected.push(TableId::ScientistDaily);
            }
            if version < 104 {
                expected.extend([
                    TableId::CountryMarket,
                    TableId::MarketContractTerms,
                    TableId::LendLeaseAgreements,
                    TableId::MarketEvents,
                ]);
            }
            if version < crate::rows::compatibility::RESOURCES_MAP_FORMAT {
                expected.extend([
                    TableId::ResourceOrigins,
                    TableId::ResourceRights,
                    TableId::CountryOilFactors,
                    TableId::StateTemporaryResources,
                    TableId::ConvoyClients,
                    TableId::TradeRoutes,
                    TableId::RegionalConvoyEfficiency,
                ]);
            }
            if version < crate::rows::compatibility::INDUSTRIAL_ORGANISATION_DETAILS_FORMAT {
                expected.extend([
                    TableId::VariantStats,
                    TableId::IndustrialOrganisationDetails,
                    TableId::IndustrialOrganisationPolicies,
                ]);
            }
            if version < crate::rows::compatibility::TEMPLATE_SETTINGS_FORMAT {
                expected.push(TableId::CountryTemplateState);
            }
            if version < crate::rows::compatibility::MOVEMENT_STATE_TERMS_FORMAT {
                expected.push(TableId::MovementStateTerms);
            }
            if version < crate::rows::compatibility::CORE_SUPPLY_ACCESS_FORMAT {
                expected.push(TableId::CoreSupplyAccess);
            }
            if version < crate::rows::compatibility::COUNTRY_UNIT_SPEED_ENTRIES_FORMAT {
                expected.push(TableId::CountryUnitSpeedEntries);
            }
            if version < crate::rows::compatibility::RIVER_RELATIONS_FORMAT {
                expected.push(TableId::RiverRelations);
            }
            if version < crate::rows::compatibility::DIVISION_TRANSPORT_SPEEDS_FORMAT {
                expected.push(TableId::DivisionTransportSpeeds);
            }
            if version < 104 {
                expected.extend([
                    TableId::LandCombatResults,
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
                    TableId::NavalRepairCountries,
                    TableId::NavalRepairFleets,
                    TableId::NavalRepairTaskForces,
                    TableId::NavalTaskForceComposition,
                    TableId::NavalRepairBases,
                    TableId::NavalRepairQueue,
                    TableId::NavalRefitShips,
                    TableId::NavalRefitProgress,
                    TableId::NavalRepairChildren,
                    TableId::IntelNetworkSummaries,
                    TableId::PropagandaEfforts,
                    TableId::IntelContributions,
                    TableId::IntelSources,
                    TableId::IntelNetworkAuxiliaries,
                    TableId::RadarCoverage,
                    TableId::AirMissionInputs,
                    TableId::AirSortieModifiers,
                    TableId::RaidAirScores,
                    TableId::NavalMissionTaskForces,
                    TableId::AirGroundCrews,
                    TableId::AirTransferInputs,
                    TableId::NavalRegionalMines,
                    TableId::NavalAccidentReports,
                    TableId::NavalDominanceTaskForceRanges,
                    TableId::NavalPatrolMissionInputs,
                ]);
            }
            if version < crate::rows::compatibility::UNIT_BONUSES_FORMAT {
                expected.push(TableId::CountryUnitBonuses);
            }
            if version < crate::rows::compatibility::DIVISION_MODEL_INPUTS_FORMAT {
                expected.push(TableId::DivisionModelInputs);
            }
            if version < crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT {
                expected.push(TableId::LandUnitOrder);
            }
            if version < crate::rows::compatibility::CAPTURE_ADDITIONS_FORMAT {
                expected.push(TableId::StateBuildingInputs);
            }
            if version < crate::rows::compatibility::RETREAT_BANS_FORMAT {
                expected.push(TableId::DivisionRetreatBans);
            }
            if version < crate::rows::compatibility::ESSENTIAL_STOCK_FORMAT {
                expected.push(TableId::StockpileArchetypes);
            }
            if version < crate::rows::compatibility::INTEL_NETWORK_MAP_FORMAT {
                expected.push(TableId::IntelNetworks);
            }
            if version <= 100 {
                expected.extend([
                    TableId::ResearchTechnologies,
                    TableId::TechnologyDefinitions,
                ]);
            }
            if version <= 99 {
                expected.extend([
                    TableId::CountryProductionModifiers,
                    TableId::ProductionLineModifiers,
                    TableId::CountryExperienceGains,
                    TableId::CountryGameScoreInputs,
                    TableId::AirWingGameScoreStats,
                    TableId::GameScoreMultipliers,
                    TableId::AgencyUpgradeLevels,
                    TableId::OperationCompletions,
                    TableId::CountryPowerDaily,
                    TableId::CountryPowerChanges,
                    TableId::CountryPowerDailyCosts,
                    TableId::GovernmentChanges,
                    TableId::PoliticsCaptureCoverage,
                    TableId::WarOwnedRelations,
                    TableId::WarRelationGoals,
                    TableId::WarOverviewCountries,
                    TableId::WarNameRules,
                    TableId::PeaceSettlements,
                    TableId::PeaceBids,
                    TableId::DivisionEquipmentDeletions,
                    TableId::DivisionLandInputs,
                    TableId::DestroyedDivisions,
                    TableId::LandEventCoverage,
                    TableId::NavalResultSources,
                    TableId::AirLossEvents,
                    TableId::BuildingBombingDamage,
                    TableId::AirDailyRoster,
                    TableId::RocketProductionOutput,
                ]);
            }
            if version <= 98 {
                expected.extend([
                    TableId::TimedIdeas,
                    TableId::DecisionRandomItems,
                    TableId::CapturedOperativeReferences,
                    TableId::ReservedCountryNaming,
                ]);
            }
            if version <= 97 {
                expected.push(TableId::AvailableAdvisors);
            }
            if version <= 93 {
                expected.push(TableId::RaidTooltipOperands);
            }
            expected.sort_unstable_by_key(|table| table.index());
            expected.dedup();
            assert_eq!(
                TableId::ALL
                    .into_iter()
                    .filter(|&table| format.table_refusal(table).is_some())
                    .collect::<Vec<_>>(),
                expected,
                "format {version}",
            );
            for table in expected {
                assert!(
                    format
                        .table_refusal(table)
                        .unwrap()
                        .starts_with(&format!("{table:?} ")),
                    "format {version}, {table:?}",
                );
            }
        }
    }

    #[test]
    fn format101_preserves_published_air_and_oracle_prefixes() {
        for version in PROVEN_FROM..crate::rows::compatibility::POLITICAL_FORMAT {
            if version == RETIRED_FORMAT {
                continue;
            }
            let format = FormatVersion::read(version).unwrap();
            for (table, fields, gaps) in [
                (TableId::NavalBases, 5, 1),
                (
                    TableId::ConstructionLineOutput,
                    if version == 100 { 14 } else { 13 },
                    if version == 100 { 9 } else { 10 },
                ),
            ] {
                assert!(
                    format.table_refusal(table).is_none(),
                    "format {version}, {table:?}"
                );
                assert_eq!(format.fields(table), fields, "format {version}, {table:?}");
                assert_eq!(
                    format.field_gaps(table).count(),
                    gaps,
                    "format {version}, {table:?}"
                );
                assert_eq!(FormatVersion::CURRENT.fields(table), table.field_count());
                assert_eq!(FormatVersion::CURRENT.field_gaps(table).count(), 0);
            }
        }
    }

    #[test]
    fn compatibility_meaning_change_refuses_same_layout() {
        let format = FormatVersion::read(93).unwrap();
        let table = TableId::RaidTooltipOperands;
        assert_eq!(format.fields(table), table.field_count());
        assert_eq!(
            format.table_refusal(table).unwrap(),
            "RaidTooltipOperands layout or meaning changed at format 94; this recording is format 93"
        );
        assert!(
            FormatVersion::read(94)
                .unwrap()
                .table_refusal(table)
                .is_none()
        );
    }

    #[test]
    fn compatibility_record_change_is_named() {
        let error = FormatVersion::read(91)
            .unwrap()
            .require(RecordKind::Partial)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Partial layout changed at format 92; this recording is format 91"
        );
    }

    #[test]
    fn game_score_catalog_introductions_keep_previous_tables_readable() {
        for version in [
            crate::rows::compatibility::RECORDER_INPUTS_FORMAT - 1,
            crate::rows::compatibility::GAME_SCORE_FORMAT - 1,
        ] {
            let previous = FormatVersion::read(version).unwrap();
            assert!(
                previous
                    .table_refusal(TableId::NavalCombatDetails)
                    .is_none()
            );
            for table in [
                TableId::CountryGameScoreInputs,
                TableId::AirWingGameScoreStats,
                TableId::GameScoreMultipliers,
            ] {
                assert!(!table.last_change().present(version));
                assert_eq!(
                    previous.table_refusal(table).unwrap(),
                    format!(
                        "{table:?} not recorded at format {version} (introduced at format {})",
                        crate::rows::compatibility::GAME_SCORE_FORMAT,
                    ),
                );
                assert!(FormatVersion::CURRENT.table_refusal(table).is_none());
                assert!(table.last_change().present(FORMAT_VERSION));
            }
        }
    }

    #[test]
    fn stepped_naval_condition_layout_is_refused_before_exact_operands() {
        let current = crate::rows::compatibility::EXACT_NAVAL_CONDITION_FORMAT;
        let previous = FormatVersion::read(current - 1).unwrap();
        assert!(previous.table_refusal(TableId::NavalCombatShips).is_some());
        assert!(
            FormatVersion::CURRENT
                .table_refusal(TableId::NavalCombatShips)
                .is_none()
        );
    }
}
