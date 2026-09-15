//! Staggered background tiers. Single source of truth for cadence.
//!
//! Tier-H (default) walks hourly. Tier-4 rotates every 4th hour by family.
//! Rotation smooths measured game-thread p50: slot0 Characters 11.576,
//! slot1 Relations 7.697, slot2 Research 7.857, slot3 Variants 2.105 plus
//! Projects 2.152 plus Buildings 1.612 plus LexicalTokens 4.357 (=10.226).
//! Hourly Tier-4 total 37.356, staggered average 9.339, saving 28.017 p50.
//! LexicalTokens falls back to Tier-4 slot3: witness cannot prove staleness-free
//! static-prefix caching within scheduler-only file ownership.

use crate::rows::TableId;

pub const STAGGER_PERIOD: i32 = 4;
pub const SKIPPED_REASON: &str = "staggered off-hours; carry forward";
pub const STAGGER_LIMITATION: &str =
    "refreshed every 4th hour; off-hours replay holds last refresh";

pub const fn refresh_slot(table: TableId) -> Option<u32> {
    match table {
        TableId::Characters
        | TableId::CountryLeaderRoles
        | TableId::AdvisorRoles
        | TableId::UnitLeaders
        | TableId::UnitLeaderTraits
        | TableId::UnitLeaderTraitExperience => Some(0),
        TableId::RelationOpinions | TableId::RelationTuples => Some(1),
        TableId::ResearchTechnologies | TableId::ResearchSlots | TableId::TechnologyDefinitions => {
            Some(2)
        }
        TableId::VariantDefinitions
        | TableId::VariantModules
        | TableId::VariantStats
        | TableId::VariantUpgrades
        | TableId::ProjectPrograms
        | TableId::Projects
        | TableId::ProjectHistory
        | TableId::ProjectBreakthroughs
        | TableId::StateBuildings
        | TableId::ProvinceBuildings
        | TableId::ProvinceNames
        | TableId::DivisionTemplates
        | TableId::LexicalTokens => Some(3),
        _ => None,
    }
}

pub const fn is_staggered(table: TableId) -> bool {
    refresh_slot(table).is_some()
}

pub fn is_refresh(table: TableId, hour: crate::Hour) -> bool {
    match refresh_slot(table) {
        None => true,
        Some(slot) => hour.rem_euclid(STAGGER_PERIOD) == slot as i32,
    }
}

pub fn should_walk(table: TableId, hour: crate::Hour, force_full: bool) -> bool {
    force_full || is_refresh(table, hour)
}

pub fn is_full_hour(hour: crate::Hour, branch_first: crate::Hour) -> bool {
    hour == branch_first || (hour - branch_first).rem_euclid(crate::KEYFRAME_INTERVAL as i32) == 0
}

pub fn refresh_text(table: TableId) -> &'static str {
    match refresh_slot(table) {
        None => "hourly",
        Some(0) => "H%4==0",
        Some(1) => "H%4==1",
        Some(2) => "H%4==2",
        _ => "H%4==3",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn rotation_covers_hours_h_through_h_plus_7() {
        for base in [0, 1, 60_820_531, i32::MAX - 8] {
            let mut seen: BTreeSet<TableId> = BTreeSet::new();
            for hour in base..base + 8 {
                for table in TableId::ALL {
                    if is_refresh(table, hour) {
                        seen.insert(table);
                    }
                }
            }
            for table in TableId::ALL {
                assert!(seen.contains(&table), "{table:?} missing near {base}");
            }
        }
    }

    #[test]
    fn every_tier4_family_refreshes_at_least_every_4th_hour() {
        let families: &[&[TableId]] = &[
            &[
                TableId::Characters,
                TableId::CountryLeaderRoles,
                TableId::AdvisorRoles,
                TableId::UnitLeaders,
                TableId::UnitLeaderTraits,
                TableId::UnitLeaderTraitExperience,
            ],
            &[TableId::RelationOpinions, TableId::RelationTuples],
            &[
                TableId::ResearchTechnologies,
                TableId::ResearchSlots,
                TableId::TechnologyDefinitions,
            ],
            &[
                TableId::VariantDefinitions,
                TableId::VariantModules,
                TableId::VariantStats,
                TableId::VariantUpgrades,
            ],
            &[
                TableId::ProjectPrograms,
                TableId::Projects,
                TableId::ProjectHistory,
                TableId::ProjectBreakthroughs,
            ],
            &[TableId::StateBuildings, TableId::ProvinceBuildings],
            &[TableId::LexicalTokens],
        ];
        for hour in [0, 7, 60_820_531] {
            for family in families {
                let mut hits = 0;
                for offset in 0..4 {
                    if family.iter().all(|t| is_refresh(*t, hour + offset)) {
                        hits += 1;
                    }
                }
                assert_eq!(hits, 1, "{family:?} near {hour}");
            }
        }
    }

    #[test]
    fn tier_h_defaults_to_hourly_and_force_full_walks_all() {
        for table in [
            TableId::Clock,
            TableId::Provinces,
            TableId::StateFlags,
            TableId::Divisions,
            TableId::DivisionMovement,
            TableId::DivisionStatus,
            TableId::DivisionEta,
            TableId::ProvinceVisibility,
            TableId::CountryAppearance,
            TableId::ArmyIconInputs,
            TableId::ShipModels,
            TableId::SupplyNodes,
            TableId::TrainInventory,
            TableId::LandCombatProgress,
            TableId::AirBaseRegistrations,
            TableId::ActiveAbilities,
        ] {
            assert!(!is_staggered(table));
            assert!(is_refresh(table, 123));
            assert!(should_walk(table, 124, false));
        }
        for table in TableId::ALL {
            assert!(should_walk(table, 1, true));
        }
    }

    #[test]
    fn staggered_slots_match_approved_rotation_map() {
        for table in [
            TableId::Characters,
            TableId::CountryLeaderRoles,
            TableId::AdvisorRoles,
            TableId::UnitLeaders,
            TableId::UnitLeaderTraits,
            TableId::UnitLeaderTraitExperience,
        ] {
            assert_eq!(refresh_slot(table), Some(0));
        }
        for table in [TableId::RelationOpinions, TableId::RelationTuples] {
            assert_eq!(refresh_slot(table), Some(1));
        }
        for table in [
            TableId::ResearchTechnologies,
            TableId::ResearchSlots,
            TableId::TechnologyDefinitions,
        ] {
            assert_eq!(refresh_slot(table), Some(2));
        }
        for table in [
            TableId::VariantDefinitions,
            TableId::VariantModules,
            TableId::VariantStats,
            TableId::VariantUpgrades,
            TableId::ProjectPrograms,
            TableId::Projects,
            TableId::ProjectHistory,
            TableId::ProjectBreakthroughs,
            TableId::StateBuildings,
            TableId::ProvinceBuildings,
            TableId::LexicalTokens,
        ] {
            assert_eq!(refresh_slot(table), Some(3));
        }
    }
}
