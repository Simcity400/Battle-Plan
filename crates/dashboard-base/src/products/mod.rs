//! The product contract and the shared products other products build on. The
//! dashboard registers every product, these included.

mod product;
pub use product::*;

pub mod artwork;
pub mod battles;
pub mod character_portraits;
pub mod country_experience;
pub mod country_factories;
pub mod country_groups;
pub mod country_history;
pub mod country_measure;
pub mod country_people;
pub mod country_styles;
pub mod dockyard_repairs;
pub mod dockyard_spending;
pub mod equipment;
pub mod equipment_icon;
pub mod equipment_produced;
pub mod faction_details;
pub mod faction_totals;
pub mod fixed5_runs;
pub mod force_census;
pub mod idea_format;
pub mod idea_history;
pub mod place_names;
pub mod politics_content;
pub mod production_lines;
pub mod production_spending;
pub mod research;
pub mod resources;
pub mod territory;
pub mod territory_breakdown;

crate::reply_warnings!(
    country_experience::ExperienceReply,
    country_factories::CountryFactoriesReply,
    country_groups::CountryGroupReply,
    country_measure::MeasureReply,
    country_people::PeopleReply,
    country_styles::CountryStyleReply,
    equipment_produced::EquipmentProducedReply,
    faction_totals::FactionTotalsReply,
    faction_details::FactionDetailReply,
    idea_history::IdeaHistoryReply,
    place_names::PlaceNamesReply,
    production_lines::ProductionLinesReply,
    production_lines::ProductionLineDetailReply,
    production_lines::ProductionLineBreakdownReply,
    production_spending::ProductionSpendingReply,
    research::ResearchReply,
    territory::TerritoryReply,
    territory_breakdown::TerritoryBreakdownReply,
);
