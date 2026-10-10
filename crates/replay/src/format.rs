//! Readers accept proven unchanged contracts; writers emit only today's format.
use crate::{
    FORMAT_VERSION,
    codec::invalid,
    rows::{
        TableId,
        compatibility::{PROVEN_FROM, RETIRED_FORMAT, RecordKind},
    },
};
use std::io;

/// Preserved when the score family is disabled for a recording session.
pub const GAME_SCORE_DISABLED_REASON: &str = "game score capture disabled at attachment";

/// A recording version inside the proven compatibility horizon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatVersion(u16);

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
            RecordKind::ReplayBlock,
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
            Err(invalid(format!(
                "{kind:?} layout changed at format {}; this recording is format {}",
                change.format, self.0
            )))
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

    /// Byte-boundary information for discarding an incompatible row, never
    /// decoding its contents or producing a row in a different contract.
    #[cfg(test)]
    pub(crate) fn fields(self, table: TableId) -> usize {
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
    fn politics_additions_leave_the_published_table_contracts_readable() {
        let previous =
            FormatVersion::read(crate::rows::compatibility::WIDGET_CAPTURE_FORMAT - 1).unwrap();
        for table in [
            TableId::Countries,
            TableId::DivisionStatus,
            TableId::Characters,
            TableId::OperativeMemberships,
            TableId::CountryAppearance,
            TableId::AttritionModifierBreakdowns,
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
        assert_eq!(FORMAT_VERSION, 101);
        for version in PROVEN_FROM..=FORMAT_VERSION {
            if version == RETIRED_FORMAT {
                assert!(FormatVersion::read(version).is_err());
                continue;
            }
            let format = FormatVersion::read(version).unwrap();
            let mut expected = Vec::new();
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
                    TableId::LandCombatResults,
                    TableId::DestroyedDivisions,
                    TableId::LandEventCoverage,
                    TableId::NavalResultSources,
                    TableId::AirLossEvents,
                    TableId::BuildingBombingDamage,
                    TableId::AirDailyRoster,
                    TableId::EventFeedHistory,
                    TableId::RocketProductionOutput,
                ]);
            }
            if version <= 98 {
                expected.extend([
                    TableId::TimedIdeas,
                    TableId::DecisionRandomItems,
                    TableId::CapturedOperativeReferences,
                    TableId::ReservedCountryNaming,
                    TableId::AttritionModifierBreakdowns,
                ]);
            }
            if version <= 97 {
                expected.extend([TableId::AvailableAdvisors, TableId::OwnedWargoals]);
            }
            if version <= 93 {
                expected.push(TableId::RaidTooltipOperands);
            }
            expected.sort_unstable_by_key(|table| table.index());
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
        for version in PROVEN_FROM..FORMAT_VERSION {
            if version == RETIRED_FORMAT {
                continue;
            }
            let format = FormatVersion::read(version).unwrap();
            for (table, fields, gaps) in [
                (TableId::AirNavalPatrolHistories, 2, 1),
                (TableId::NavalBases, 5, 1),
                (
                    TableId::ConstructionLineOutput,
                    if version == 100 { 14 } else { 13 },
                    if version == 100 { 1 } else { 2 },
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
                    .table_refusal(TableId::NavalModifierBreakdowns)
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
}
