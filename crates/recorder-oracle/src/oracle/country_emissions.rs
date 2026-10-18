//! Save-projection comparisons that also exercise oracle composition.
use super::{country_emission_layout, parse_country_emissions};
use installed_content_save::{ActualCountry, Save, SaveDocument};
use replay_data::{rows::Recorded, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;

#[cfg(test)]
#[path = "country_emissions/tests.rs"]
mod tests;
