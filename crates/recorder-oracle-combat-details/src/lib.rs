//! combat details save comparisons and recorded consistency checks.
use installed_content_save as save;
use installed_content_save::Decimal;
use installed_content_save::Save;
#[cfg(test)]
use recorder_oracle_support::fixtures;
use recorder_oracle_support::report::Report;
use recorder_oracle_support::support::*;
use replay_data::rows::CountryRow;
use replay_data::rows::TableId;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
pub mod combat_details;
