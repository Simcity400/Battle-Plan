//! Save-projection comparisons that also exercise oracle composition.
use super::{country_emission_layout, parse_country_emissions};
use installed_content_save::{ActualCountry, Save, SaveDocument};
use replay::{
    frame::DecodedFrame,
    rows::{Recorded, TableId, tables},
};

#[cfg(test)]
#[path = "country_emissions/tests.rs"]
mod tests;
