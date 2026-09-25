//! Capture cadence. Single source of truth.
//!
//! Hourly: map inputs, character identities and unit leaders. Daily: panel
//! data, including persistent character roles and traits, walked once per 24
//! game-hours in a fixed slot so every hour costs about the same. Static: data
//! that cannot change after a save loads, walked only on a full hour (branch
//! start, keyframe, load, gap). Off-slot hours record [`SKIPPED_REASON`] and
//! the replay carries the last walk forward.

use crate::rows::TableId;

pub const STAGGER_PERIOD: i32 = 24;
pub const SKIPPED_REASON: &str = "staggered off-hours; carry forward";
pub const STAGGER_LIMITATION: &str =
    "refreshed on its scheduled slot; off-slot hours hold the last refresh";
pub const STATIC_LIMITATION: &str = "read once per branch; cannot change after the save loads";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    Hourly,
    /// Map-visible but slow to change: every 4th hour, in the given slot.
    FourHourly(u32),
    Daily(u32),
    Static,
}

pub const fn cadence(table: TableId) -> Cadence {
    use Cadence::{Daily, FourHourly, Static};
    match table {
        // Cannot change after the save loads (the DLC set included).
        TableId::LexicalTokens | TableId::SimulationFeatures => Static,
        // Relations feed the perspective.
        TableId::RelationOpinions | TableId::RelationTuples => FourHourly(1),
        // Province names are live overrides: set_province_name and
        // reset_province_name effects can change them after a save loads.
        TableId::ProvinceNames => FourHourly(3),
        // Daily slots: whole shared-source families stay together. The
        // technology and resource definitions are static too, but their
        // daily consumers walk them as one census, so they share the slot.
        TableId::ResearchTechnologies | TableId::TechnologyDefinitions => Daily(0),
        // Intel lanes change only in CGameState::DailyUpdate, which runs inside
        // the hourly update entered at hour 23; slot 0 is the first sample after it.
        TableId::CountryIntel => Daily(0),
        TableId::ProjectPrograms
        | TableId::Projects
        | TableId::ProjectHistory
        | TableId::ProjectBreakthroughs => Daily(1),
        TableId::DivisionEquipment
        | TableId::ArmyUpgradeGroups
        | TableId::ArmyUpgradeRequests
        | TableId::ArmyUpgradeDeliveries
        | TableId::ArmyUpgradeProduced => Daily(2),
        TableId::StateBaseResources
        | TableId::BuildingConstruction
        | TableId::RailwayConstruction
        | TableId::OccupationSettings
        | TableId::StateOccupationLaws
        | TableId::StateGarrisons
        | TableId::StateAssignedManpower
        | TableId::CountryOccupationReserveManpower
        | TableId::StateResistance
        | TableId::CountryCollaboration => Daily(3),
        TableId::AirStatistics => Daily(5),
        TableId::DecisionCounts | TableId::Decisions => Daily(6),
        TableId::AirRegionHistory | TableId::AirRegionDamage => Daily(7),
        TableId::ProductionLines
        | TableId::Stockpiles
        | TableId::ProductionLicenses
        | TableId::DeploymentConveyors
        | TableId::DeploymentLines
        | TableId::DeploymentManpower
        | TableId::DeploymentEquipment
        | TableId::ProductionLicenseRecords => Daily(8),
        TableId::ScriptVariables | TableId::DivisionTemplates => Daily(9),
        TableId::ResourceDefinitions | TableId::CountryResources | TableId::ResourceExchanges => {
            Daily(10)
        }
        TableId::ResearchSlots => Daily(11),
        TableId::ScriptFlags => Daily(12),
        TableId::LandTheatres
        | TableId::LandTheatreGroups
        | TableId::LandTheatreMembers
        | TableId::AirTheatres
        | TableId::AirTheatreGroups
        | TableId::AirTheatreMembers => Daily(13),
        TableId::IntelNetworks => Daily(14),
        TableId::DynamicModifiers | TableId::ActiveIdeas => Daily(15),
        TableId::OperativeMemberships | TableId::ActiveAbilities | TableId::WingCumulative => {
            Daily(16)
        }
        TableId::Scientists
        | TableId::ScientistSkills
        | TableId::PowerBalances
        | TableId::PowerBalanceMembers
        | TableId::PowerBalanceRanges
        | TableId::PowerBalanceOverrides
        | TableId::CountryFuel
        | TableId::CountryExperience
        | TableId::HubMotorization
        | TableId::DisruptedSupply
        | TableId::ForeignHomebases
        | TableId::PurchaseContracts => Daily(17),
        TableId::PreferredTactics
        | TableId::PreferredTacticOperands
        | TableId::WorldTension
        | TableId::ThreatSources
        | TableId::ThreatCountryAggregates
        | TableId::TheatreAreas => Daily(18),
        // Raids stay hourly: RaidTargets is the same walk's map-icon projection.
        TableId::SunkShips | TableId::Focus | TableId::CompletedFocuses => Daily(19),
        TableId::IntelligenceAgencies | TableId::OperationTokens => Daily(20),
        TableId::AirAces
        | TableId::PeaceConferences
        | TableId::CountryLeaderRoles
        | TableId::AdvisorRoles
        | TableId::UnitLeaderTraits
        | TableId::UnitLeaderTraitExperience => Daily(21),
        TableId::Cryptology
        | TableId::IncomingDiplomaticActions
        | TableId::TechSharingGroups
        | TableId::TechSharingMembers
        | TableId::VariantDefinitions
        | TableId::VariantModules
        | TableId::VariantStats
        | TableId::VariantUpgrades => Daily(22),
        TableId::NavalSupremacy
        | TableId::Operations
        | TableId::OperationAssignments
        | TableId::SunkConvoys => Daily(23),
        // RadarCoverage is hourly because a controller change edits radar
        // groups at any hour (141093AC0). Cost at W1 (25 hours, vanilla map):
        // 0.11 ms p50 / 0.16 ms p95 read on the game thread, a 0.12 ms/h raw
        // copy on the decode worker, 104 B/h encoded.
        _ => Cadence::Hourly,
    }
}

/// The daily slot, for slot balancing and status.
pub const fn refresh_slot(table: TableId) -> Option<u32> {
    match cadence(table) {
        Cadence::Daily(slot) => Some(slot),
        Cadence::Hourly | Cadence::FourHourly(_) | Cadence::Static => None,
    }
}

/// Not walked every hour; off-hours carry the last walk forward.
pub const fn is_staggered(table: TableId) -> bool {
    matches!(
        cadence(table),
        Cadence::FourHourly(_) | Cadence::Daily(_) | Cadence::Static
    )
}

pub fn is_refresh(table: TableId, hour: crate::Hour) -> bool {
    cadence(table).is_refresh(hour)
}

impl Cadence {
    pub fn is_refresh(self, hour: crate::Hour) -> bool {
        match self {
            Cadence::Hourly => true,
            Cadence::FourHourly(slot) => hour.rem_euclid(4) == slot as i32,
            Cadence::Daily(slot) => hour.rem_euclid(STAGGER_PERIOD) == slot as i32,
            Cadence::Static => false,
        }
    }
}

pub fn should_walk(table: TableId, hour: crate::Hour, force_full: bool) -> bool {
    force_full || is_refresh(table, hour)
}

pub fn is_full_hour(hour: crate::Hour, branch_first: crate::Hour) -> bool {
    (i64::from(hour) - i64::from(branch_first)).rem_euclid(i64::from(crate::KEYFRAME_INTERVAL)) == 0
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
        const FOUR: [&str; 4] = ["H%4==0", "H%4==1", "H%4==2", "H%4==3"];
        match self {
            Cadence::Hourly => "hourly",
            Cadence::FourHourly(slot) => FOUR[slot as usize],
            Cadence::Daily(slot) => DAILY[slot as usize],
            Cadence::Static => "static",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cadence_refreshes_its_count_per_day_and_full_hours_walk_everything_recorded() {
        for base in [0, 1, 60_820_531, i32::MAX - 24] {
            for table in TableId::ALL {
                let hits = (base..base + 24).filter(|h| is_refresh(table, *h)).count();
                let expected = match cadence(table) {
                    Cadence::Hourly => 24,
                    Cadence::FourHourly(_) => 6,
                    Cadence::Daily(_) => 1,
                    Cadence::Static => 0,
                };
                assert_eq!(hits, expected, "{table:?} near {base}");
                assert!(should_walk(table, base, true));
            }
        }
    }

    #[test]
    fn map_tables_are_hourly_and_families_share_a_slot() {
        for table in [
            TableId::Clock,
            TableId::Characters,
            TableId::UnitLeaders,
            TableId::Provinces,
            TableId::States,
            TableId::Divisions,
            TableId::DivisionMovement,
            TableId::DivisionStatus,
            TableId::DivisionEtaInputs,
            TableId::MovementDefines,
            TableId::RadarCoverage,
            TableId::RelationLinks,
            TableId::CountryVisibility,
            TableId::StateVisibility,
            TableId::ShipModels,
            TableId::SupplyNodes,
            TableId::WeatherProvinces,
            TableId::LandCombatDivisions,
            TableId::AirBaseRegistrations,
            // Bombing changes a building's mesh and fire within the hour.
            TableId::StateBuildings,
            TableId::ProvinceBuildings,
            TableId::BuildingDetection,
        ] {
            assert_eq!(cadence(table), Cadence::Hourly, "{table:?}");
            assert_eq!(refresh_text(table), "hourly");
        }
        for family in [
            &[
                TableId::CountryLeaderRoles,
                TableId::AdvisorRoles,
                TableId::UnitLeaderTraits,
                TableId::UnitLeaderTraitExperience,
            ][..],
            &[
                TableId::ProjectPrograms,
                TableId::Projects,
                TableId::ProjectHistory,
            ][..],
            &[TableId::RelationOpinions, TableId::RelationTuples],
            &[
                TableId::VariantDefinitions,
                TableId::VariantModules,
                TableId::VariantStats,
            ],
        ] {
            let shared = cadence(family[0]);
            assert_ne!(shared, Cadence::Hourly);
            assert!(family.iter().all(|t| cadence(*t) == shared), "{family:?}");
        }
        assert_eq!(cadence(TableId::RelationTuples), Cadence::FourHourly(1));
        assert_eq!(refresh_text(TableId::RelationTuples), "H%4==1");
        assert_eq!(refresh_slot(TableId::RelationTuples), None);
        assert_eq!(refresh_slot(TableId::Projects), Some(1));
        assert_eq!(cadence(TableId::LexicalTokens), Cadence::Static);
        assert_eq!(refresh_text(TableId::LexicalTokens), "static");
        assert!(is_staggered(TableId::LexicalTokens));
        assert_eq!(refresh_slot(TableId::LexicalTokens), None);
    }

    #[test]
    fn full_hours_follow_the_keyframe_interval() {
        assert!(is_full_hour(100, 100));
        assert!(!is_full_hour(101, 100));
        assert!(is_full_hour(100 + crate::KEYFRAME_INTERVAL as i32, 100));
        assert!(is_full_hour(i32::MIN + 720, i32::MIN));
    }

    #[test]
    fn province_name_changes_are_recaptured_at_the_next_four_hour_slot() {
        assert_eq!(cadence(TableId::ProvinceNames), Cadence::FourHourly(3));
        for changed_at in -24..24 {
            let next_refresh = (changed_at + 1..=changed_at + 4)
                .find(|hour| should_walk(TableId::ProvinceNames, *hour, false))
                .expect("a changed override must be revisited within four game hours");
            assert_eq!(next_refresh.rem_euclid(4), 3);
            assert!(should_walk(TableId::ProvinceNames, changed_at, true));
        }
    }
}
