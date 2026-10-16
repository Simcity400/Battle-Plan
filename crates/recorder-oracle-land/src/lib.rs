//! land save comparisons and recorded consistency checks.
use installed_content_save as save;
use installed_content_save::Decimal;
#[cfg(test)]
use recorder_oracle_support::fixtures;
use recorder_oracle_support::report::Report;
use recorder_oracle_support::support::*;
use replay::container::Replay;
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
pub mod land;
