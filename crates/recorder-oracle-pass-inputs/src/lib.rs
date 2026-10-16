//! pass inputs save comparisons and recorded consistency checks.
use installed_content_save::Save;
use recorder_oracle_counters::counters;
use recorder_oracle_country_rosters::country_rosters;
use recorder_oracle_naval_missions::naval_missions;
#[cfg(test)]
use recorder_oracle_supply_trains::supply_trains;
#[cfg(test)]
use recorder_oracle_support::fixtures;
use recorder_oracle_support::report::Report;
use recorder_oracle_support::support::*;
use replay::frame::DecodedFrame;
#[cfg(test)]
use replay::frame::TableView;
use replay::rows::TableId;
#[cfg(test)]
use replay::rows::TableRows;
use replay::rows::tables;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
pub mod pass_inputs;
