//! The classification a carrier's producer contract is written in, shared by
//! every family that has one.
//!
//! A producer contract answers one question per mandatory leaf of a carrier's
//! schema: is this leaf the PRODUCER's, the VIEWER's, or nobody's yet. The
//! three answers and the rules a table of them has to satisfy are the same for
//! every family, so they live here and each family declares only its own
//! tables. XMAP13's are in `xmap13_producer_contract`, XMAP15's in
//! `xmap15_producer_contract`.
//!
//! # Why the classification lives in shared ground at all
//!
//! A carrier's schema types belong to the viewer's cargo workspace and its
//! producer belongs to the extractor's, and the two may not depend on each
//! other. So the classification sits where both already path-depend, and each
//! side binds it to what it owns: the viewer proves the table covers exactly
//! its serialized schema, and the extractor proves its mint covers exactly the
//! recorded class. Neither proof can be satisfied by editing this file alone.
//!
//! # The partition rule
//!
//! An entry may be a leaf path or a composite PREFIX, and a prefix is allowed
//! only where every leaf under it shares one source - a content key's eight
//! leaves are one content identity, so eight entries would be eight copies of
//! one fact. Where the halves differ the composite is expanded.
//! [`validate_carrier_contract_table`] holds that rule by refusing any entry
//! that is a strict prefix of another.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Who has to do the next piece of work before a gated leaf has any source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarrierGateOwner {
    /// A recorder-side read that is lawful to make and has not been built.
    Producer,
    /// A replay-side derivation the derivation rule already assigns to the
    /// viewer, whose LAW is not decoded yet.
    Viewer,
    /// The field cannot be sourced by either side while the schema declares it
    /// in its current shape.
    Schema,
    /// Whether anything is owed at all is a product decision, not a technical
    /// one.
    Owner,
}

/// Where one mandatory leaf of the XMAP13 carrier comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum CarrierLeafSource {
    /// The producer mints it from state the capture already carries.
    RecordedPlayTimeState {
        /// The recorded input it is minted from, in the reach report's
        /// vocabulary where one applies.
        recorded_input: &'static str,
    },
    /// The viewer owns it, and the law is settled.
    ReplayDerived { authority: &'static str },
    /// Nobody can source it today.
    Gate {
        owed_by: CarrierGateOwner,
        missing_input: &'static str,
        closing_step: &'static str,
    },
}

impl CarrierLeafSource {
    pub fn gate_owner(&self) -> Option<CarrierGateOwner> {
        match self {
            Self::Gate { owed_by, .. } => Some(*owed_by),
            _ => None,
        }
    }

    pub fn is_minted_by_the_producer(&self) -> bool {
        matches!(self, Self::RecordedPlayTimeState { .. })
    }
}

/// A recorded input the producer needs that ONE HOUR of ONE recording did not
/// carry.
///
/// Distinct from a contract gate in [`CarrierLeafSource::Gate`], which is true of
/// every capture ever made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarrierHourGate {
    /// One of the family's own recorded-input roster.
    pub recorded_input: String,
    /// What the recording says about it, in the recording's own words.
    pub recorded_state: String,
}

pub fn carrier_leaf_source_text(source: &CarrierLeafSource) -> &'static str {
    match source {
        CarrierLeafSource::RecordedPlayTimeState { recorded_input } => recorded_input,
        CarrierLeafSource::ReplayDerived { authority } => authority,
        CarrierLeafSource::Gate { missing_input, .. } => missing_input,
    }
}

/// The rules one classification table has to satisfy.
///
/// Takes the table rather than reading the constants, so the tests can feed it
/// a malformed one. A validator that can only ever be called on the real tables
/// is a validator nothing proves catches anything.
pub fn validate_carrier_contract_table(
    label: &str,
    table: &[(&'static str, CarrierLeafSource)],
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for (path, source) in table {
        if path.trim().is_empty() {
            return Err(format!("{label} contract carries a blank leaf path"));
        }
        if !seen.insert(*path) {
            return Err(format!("{label} contract classifies {path} twice"));
        }
        if carrier_leaf_source_text(source).trim().is_empty() {
            return Err(format!("{label} leaf {path} names no source"));
        }
        if let CarrierLeafSource::Gate { closing_step, .. } = source {
            if closing_step.trim().is_empty() {
                return Err(format!("{label} gate {path} names no closing step"));
            }
        }
    }
    // The partition rule: a prefix entry claims everything under it, so two
    // entries where one is the other's prefix would make the classification
    // ambiguous and let a new leaf inherit a class nobody chose for it.
    for (outer, _) in table {
        for (inner, _) in table {
            if outer != inner
                && inner
                    .strip_prefix(*outer)
                    .is_some_and(|rest| rest.starts_with('.'))
            {
                return Err(format!(
                    "{label} contract has {outer} as a strict prefix of {inner}; a prefix entry \
                     must cover leaves that share one source"
                ));
            }
        }
    }
    Ok(())
}

