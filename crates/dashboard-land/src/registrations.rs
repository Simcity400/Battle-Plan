dashboard_base::register_products! {
    DOCTRINES_DOCTRINES => crate::products::doctrines::Doctrines,
    ARMY_ARMYSIZE => crate::products::army::ArmySize,
    LEADERS_LEADERPORTRAITS => crate::products::leaders::LeaderPortraits,
    LEADER_NAME_RUNS_LEADERNAMERUNS => crate::products::leader_name_runs::LeaderNameRuns,
    LEADERS_CAREER_LEADERCAREER => crate::products::leaders::career::LeaderCareer,
    LEADERS_CAREER_LEADERCHOICES => crate::products::leaders::career::LeaderChoices,
    DIVISION_LOG_DIVISIONLOG => crate::products::division_log::DivisionLog,
    DIVISION_LOG_DIVISIONLOGMANIFEST => crate::products::division_log::DivisionLogManifest,
    LAND_BATTLES_LANDBATTLESMANIFEST => crate::products::land_battles::LandBattlesManifest,
    LAND_BATTLES_LANDBATTLES => crate::products::land_battles::LandBattles,
    LAND_BATTLES_LANDBATTLESRANKING => crate::products::land_battles::LandBattlesRanking,
    LAND_BATTLES_LANDBATTLECOURSES => crate::products::land_battles::LandBattleCourses,
    LAND_TERRAIN_LANDBATTLETERRAINS => crate::products::land_terrain::LandBattleTerrains,
    COMBAT_LOG_COMBATLOG => crate::products::combat_log::CombatLog,
}

dashboard_base::register_hourly_products! {
    ARMY_TEMPLATEROSTER => crate::products::army::TemplateRoster,
    TRAINING_QUEUE_TRAININGQUEUE => crate::products::training_queue::TrainingQueue,
    DOCTRINE_MASTERY_DOCTRINEMASTERY => crate::products::doctrine_mastery::DoctrineMastery,
    DOCTRINE_MASTERY_REWARDS_DOCTRINEREWARDS => crate::products::doctrine_mastery::rewards::DoctrineRewards,
    LEADERS_UNITLEADERS => crate::products::leaders::UnitLeaders,
    LEADERS_CAREER_LEADERHOUR => crate::products::leaders::career::LeaderHour,
    DIVISIONS_TEMPLATES_DIVISIONTEMPLATES => crate::products::divisions::templates::DivisionTemplates,
    DIVISIONS_CAREERS_DIVISIONCAREERS => crate::products::divisions::careers::DivisionCareers,
    DIVISIONS_CAREERS_DIVISIONRECORD => crate::products::divisions::careers::DivisionRecord,
    DIVISIONS_EQUIPMENT_DIVISIONEQUIPMENT => crate::products::divisions::equipment::DivisionEquipment,
    ARMY_LOSSES_ARMYLOSSES => crate::products::army_losses::ArmyLosses,
    FORCES_AIRFORCEAT => crate::products::forces::AirForceAt,
    FORCES_NAVYAT => crate::products::forces::NavyAt,
    ARMY_ARMYAT => crate::products::army::ArmyAt,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_DOCTRINEMASTERY: ExactPanel = exact_panel::<crate::products::doctrine_mastery::DoctrineMastery>;
pub const EXACT_AIRFORCEAT: ExactPanel = exact_panel::<crate::products::forces::AirForceAt>;
pub const EXACT_NAVYAT: ExactPanel = exact_panel::<crate::products::forces::NavyAt>;
pub const EXACT_ARMYAT: ExactPanel = exact_panel::<crate::products::army::ArmyAt>;
