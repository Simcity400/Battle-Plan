mod demand;
pub mod details;

pub use demand::{DetailDemand, detail_demand};
pub use details::{
    BorderWarInputs, CommanderLearningTrait, CommanderSource, CommanderTimedTrait, CommanderTrait,
    Commanders, DetailArenas, DetailCounts, DetailsRow, Leaders, SKILL_HOVER_KEYS, SideDetails,
    Span, commander_sources, read_details, read_details_sharing,
};

#[cfg(test)]
mod layout_tests;
