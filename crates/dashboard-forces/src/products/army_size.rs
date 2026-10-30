//! Land army totals and the Army size race, prepared as the `army` series family
//! ([`facts`], [`emit`]) and answered from its publication ([`query`]); Scoreboard reads
//! the same publication. Missing reads and country identities end runs; they never
//! produce zeros.
//!
//! An expeditionary transfer retains the source owner while the target receives
//! the roster/logical links. Owner therefore means positive signed expeditionary
//! owner, otherwise roster owner; host is roster. The hourly census uses that
//! same owner rule, adding grouped and ungrouped divisions. Manpower wraps the
//! signed i32 contributor sum per division, then sums countries exactly.
//! Contributor origins never replace the division owner/host.
//! Combined strength is current HP, not a percentage or template maximum:
//! the game clamps it to 0..maximum and formats it as fixed5.
//!
//! The counter representative uses signed wrapping priority * count, selecting
//! the largest positive score and the first tie, with brigade (0,0)/null fallback.
//! The AI representative instead uses ai_priority * count. Division categories
//! use that AI representative, counter fallback, a template override and category
//! predicates. These differ from the battalion view: grouped template sub-units
//! (regiments and support companies) use their authored content group, never
//! either representative. No per-battalion HP is recorded, so HP stays unsplit.

use dashboard_base::{
    products::{Product, country_groups::HourSpan},
    source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag};
use dashboard_text::metadata::link_slot;
use replay_data::rows::{DivisionRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct ArmySize;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArmyMeasure {
    Manpower,
    Divisions,
    Strength,
    Battalions,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArmyAttribution {
    Owner,
    Host,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmySizeParams {
    // Reached country and hour for plain template/configuration sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub template_at: Option<(String, i32)>,
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<usize>,
    pub measure: ArmyMeasure,
    pub count_for: ArmyAttribution,
    pub by_type: bool,
    /// None is the capability/window manifest; histories use a replay-block index.
    #[serde(default)]
    pub window: Option<usize>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyType {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub total: Option<f64>,
    pub raw: String,
}

// Compact hourly totals: [first hour, geometry total, last hour or null, exact raw].
impl Serialize for ArmyRun {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (
            self.first_hour,
            self.total,
            (self.last_hour != self.first_hour).then_some(self.last_hour),
            &self.raw,
        )
            .serialize(serializer)
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyMixRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub values: Vec<Option<f64>>,
    /// Decimal counts retain exact labels; values are chart geometry only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub raw_values: Option<Vec<Option<String>>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmySeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<dashboard_base::products::country_history::CountryIdentityRun>>,
    #[ts(type = "Array<ArmyRun>")]
    pub runs: Vec<ArmyRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mix: Option<Vec<ArmyMixRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmySizeReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub template_sections: Option<Vec<String>>,
    pub branch_id: String,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    pub interval: Option<HourSpan>,
    pub available: bool,
    pub measures: Vec<ArmyMeasure>,
    /// While preparation continues, measures no hour read so far records: unknown, not absent.
    pub unknown_measures: Vec<ArmyMeasure>,
    pub types: Vec<ArmyType>,
    pub series: Vec<ArmySeries>,
    pub warnings: Vec<String>,
}

pub fn channel(measure: ArmyMeasure, attribution: ArmyAttribution) -> usize {
    match (measure, attribution) {
        (ArmyMeasure::Divisions, ArmyAttribution::Owner) => 0,
        (ArmyMeasure::Strength, ArmyAttribution::Owner) => 1,
        (ArmyMeasure::Divisions, ArmyAttribution::Host) => 2,
        (ArmyMeasure::Strength, ArmyAttribution::Host) => 3,
        (ArmyMeasure::Battalions, ArmyAttribution::Owner) => 4,
        (ArmyMeasure::Battalions, ArmyAttribution::Host) => 5,
        (ArmyMeasure::Manpower, ArmyAttribution::Owner) => 6,
        (ArmyMeasure::Manpower, ArmyAttribution::Host) => 7,
    }
}

fn amount(raw: Option<i128>, measure: ArmyMeasure) -> Option<f64> {
    raw.map(|raw| {
        raw as f64
            / if measure == ArmyMeasure::Strength {
                100_000.0
            } else {
                1.0
            }
    })
}

/// Owner/host divisions, HP, battalions, then native division manpower totals, by
/// [`channel`].
pub type Totals = [Option<i128>; 8];

/// A country's composition: each attribution's battalions by sub-unit name, None where a
/// division's template is unread or invalid.
pub type Mix = [Option<BTreeMap<String, i64>>; 2];

/// Census owner rule: positive signed expeditionary owner, otherwise roster owner.
pub fn owner(row: &DivisionRow) -> u32 {
    replay_data::military_aid::owner(row)
}

/// The raw country a division counts for under the chosen attribution: its
/// census owner, or the roster that hosts it.
pub fn attributed(row: &DivisionRow, attribution: ArmyAttribution) -> u32 {
    match attribution {
        ArmyAttribution::Owner => owner(row),
        ArmyAttribution::Host => row.roster_owner,
    }
}

fn add(target: &mut Option<i64>, value: i64) {
    *target = target.and_then(|current| current.checked_add(value));
}

impl Product for ArmySize {
    const KEY: &'static str = "army-size";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = ArmySizeParams;
    type Reply = ArmySizeReply;
    fn types() -> Vec<String> {
        vec![
            ArmyMeasure::decl(),
            ArmyAttribution::decl(),
            ArmySizeParams::decl(),
            ArmyType::decl(),
            "type ArmyRun = [number, number | null, number | null, string?];".into(),
            ArmyMixRun::decl(),
            ArmySeries::decl(),
            ArmySizeReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if let Some((tag, _)) = &params.template_at {
            validate_tag(tag)?;
        }
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params
            .top
            .is_some_and(|top| top == 0 || !params.tags.is_empty())
        {
            return Err("Invalid country selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        Ok(())
    }
}

/// Answers an `army-size` request naming `templateAt`: a country's templates and roster at
/// an hour, read from the recording.
pub fn template_roster(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<ArmySizeReply> {
    ArmySize::validate(params).map_err(dashboard_core::invalid)?;
    template_roster::query(source, params, context, cancelled)
}

pub mod aid;
mod composition;
pub mod emit;
pub mod facts;
mod inventory;
pub mod query;
pub mod served;
mod template_roster;
pub use composition::check_content;

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
