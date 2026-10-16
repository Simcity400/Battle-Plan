//! combat details save comparisons and recorded consistency checks.
use installed_content_save as save;
use installed_content_save::Decimal;
use installed_content_save::Save;
#[cfg(test)]
use recorder_oracle_support::fixtures;
use recorder_oracle_support::report::Report;
use recorder_oracle_support::support::*;
use replay::frame::DecodedFrame;
use replay::rows::CountryRow;
use replay::rows::TableId;
use replay::rows::tables;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
pub mod combat_details;
