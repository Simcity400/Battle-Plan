//! World tension over a branch, for the World tension & threat widget: the world
//! tension the game showed, each country's contribution to it and the events
//! (threat sources) that raised or lowered it.
//!
//! Every value is a recorded field, kept exact (`engine_stability::cworld_threat`,
//! `engine_stability::cthreat_source`):
//!
//! - A threat source's contributions (`current_raw`, `final_raw`) are fixed
//!   point with [`cthreat_source::FIXED_POINT_SCALE`] units per percentage point
//!   of world tension: sum every source's current contribution,
//!   clamp the sum to [`cworld_threat::CLAMP_CEILING`] (100%) and store it
//!   divided by [`cworld_threat::SCALAR_DIVISOR`] as the world tension
//!   (`WorldTension.total_raw`). So world tension has 1,000 units per percentage
//!   point and contributions 100,000, and the sum of the contributions can pass
//!   100% or include negative contributions, such as those from a peace treaty.
//! - `ThreatCountryAggregates` is the engine's own sum of the current
//!   contributions per canonical country slot, with its array sized to the
//!   country census, so every country is listed and a zero is a recorded
//!   zero. These are the widget's layers. Each read checks it against the sources
//!   and the world tension against the clamped sum, and the reply says how many
//!   reads disagreed.
//! - A source's `reason` is the text the game built for it in the recording's
//!   language with colour codes, so its event name is the game's own
//!   ([`display_text`]).
//!   `final_raw` is the contribution it adds once its ramp ends, `date` the hour
//!   it was created. The vector has no ids and is compacted, so an event is
//!   identified by its own recorded country reference, second country, date and
//!   reason (and its place among sources equal in all four), never by the slot
//!   the reference resolves to: a reference first read unresolved and named at a
//!   later read stays one event, which takes the country once it is named.
//!
//! A country that no longer exists keeps its sources until they decay, so a
//! slot keeps the tag, and a raw link the slot, they were last recorded with, at
//! any hour recording `Countries` and `CountryLinks`, a read or not.
//!
//! A country's contributions are kept by its identity, its canonical slot, and
//! named by the tag the slot held at the read: one slot still contributing under a
//! tag that another slot now holds is a second country, never joined to it, and a
//! slot recorded under another tag is another entry.
//!
//! The three tables are read together every hour (daily, one slot, in older
//! recordings). A read is an hour whose walk read them
//! (`dashboard_text::metadata::slot_read`). Values start at a read, are carried over
//! the following recorded hours and end at a gap or at an hour whose tables are
//! incomplete, so an hour the recording lacks is never given a value.
//!
//! The family prepares on the segment driver ([`facts`], [`emit`]) and answers from its series
//! publication ([`query`]).

use dashboard_base::{
    products::BranchParams, products::Product, products::country_history::CountryIdentityRun,
    products::country_measure::FixedRun,
};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::{Source, products::LoadContext, products::country_history::branch_histories};
use engine_stability::{cthreat_source, cworld_threat};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, atomic::AtomicBool};
use std::io;
use ts_rs::TS;

pub struct WorldTension;

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod served;

#[cfg(any(test, feature = "test-support"))]
pub use row::{Scan, scan};

/// World tension units per percentage point: contributions' units over the
/// divisor the engine stores the clamped sum with.
const TENSION_SCALE: i64 = cthreat_source::FIXED_POINT_SCALE / cworld_threat::SCALAR_DIVISOR;
const _: () = assert!(
    TENSION_SCALE * cworld_threat::SCALAR_DIVISOR == cthreat_source::FIXED_POINT_SCALE
        && cthreat_source::FIXED_POINT_SCALE <= u32::MAX as i64
);

/// One read of the world's tension, held over the following hours until the next.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionRead {
    /// The hour it was read, where its values start.
    pub first_hour: i32,
    /// The last hour the read's values hold.
    pub last_hour: i32,
    /// The read follows a gap or an hour without the tables.
    pub starts_segment: bool,
    /// World tension as the game showed it, in [`TensionReply::tension_scale`]
    /// units (`WorldTension.total_raw`).
    pub tension_raw: String,
    /// Every country's contribution summed before the game's cap, in
    /// [`TensionReply::contribution_scale`] units.
    pub sum_raw: String,
    /// The part of `sum_raw` from country slots no recorded country row names,
    /// in the same units; "0" when every contributing country is named.
    pub unnamed_raw: Option<String>,
    /// Fields needing earlier holder observations in a frame-only first view.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub preparing: Vec<String>,
}

/// One country's contribution: runs of reads where it was not zero. An hour of a
/// read without a run of the country is a recorded zero; an hour of no read is
/// not recorded.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionCountry {
    /// The tag the country held at these reads.
    pub tag: String,
    /// Its identity: the canonical country slot its contributions were recorded
    /// under. Two entries may share a tag, never a slot and a tag.
    pub slot: u32,
    /// In [`TensionReply::contribution_scale`] units. A run starts a segment where
    /// the hours before it were not recorded (a gap, or an hour without the
    /// tables); after a recorded zero it continues the line.
    pub runs: Vec<FixedRun>,
    /// Native holders of this cached tag/slot. Colours and names may only be
    /// sampled within these recorded bounds, including after the holder vanished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
}

/// The contribution an event adds, as recorded from a read on.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionAdded {
    /// The read where the source first held this value.
    pub hour: i32,
    /// The source's `final_raw`, in [`TensionReply::contribution_scale`] units.
    pub added_raw: String,
}

/// A threat source: something a country did that changed world tension.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionEvent {
    /// The hour the game created it (`date`).
    pub hour: i32,
    /// The country that caused it; null when no recorded country row names it.
    pub tag: Option<String>,
    /// The game's own text for it, without formatting codes.
    pub reason: String,
    /// What it adds, from its first read on; a later entry when the game changed it.
    pub added: Vec<TensionAdded>,
    /// The first and last read listing it.
    pub first_read: i32,
    pub last_read: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TensionReply {
    pub branch_id: String,
    /// World tension units per percentage point.
    pub tension_scale: u32,
    /// Contribution units per percentage point.
    pub contribution_scale: u32,
    /// The sum of contributions world tension is capped at, in contribution units.
    pub ceiling_raw: String,
    /// In hour order.
    pub reads: Vec<TensionRead>,
    /// Every country that contributed in the branch, in tag then slot order.
    pub countries: Vec<TensionCountry>,
    /// In hour order, then first read.
    pub events: Vec<TensionEvent>,
    pub warnings: Vec<String>,
    /// Exact current amounts whose remembered contributor name is not prepared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub preparing_contributors: Vec<PreparingContributor>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreparingContributor {
    pub hour: i32,
    pub slot: u32,
    pub value_raw: String,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedTension {
    pub reads: Vec<TensionRead>,
    pub countries: Vec<TensionCountry>,
    pub events: Vec<TensionEvent>,
    /// Reads whose aggregates disagree with their sources' current contributions.
    pub aggregate_mismatches: usize,
    /// Reads whose world tension is not the game's cap of the aggregates' sum.
    pub tension_mismatches: usize,
    /// Reads whose source rows are incomplete, so their events may be missing.
    pub partial_sources: usize,
    unnamed_reads: usize,
    first_contributions: Vec<i32>,
    read_count: u64,
}

fn contribution_share(value: i128, tension: i128, sum: i128) -> io::Result<i128> {
    if sum <= 0 {
        return Ok(0);
    }
    let product = value * tension;
    let scale = i128::from(cworld_threat::SCALAR_DIVISOR);
    (product / sum)
        .checked_mul(scale)
        .and_then(|whole| whole.checked_add((product % sum) * scale / sum))
        .ok_or_else(|| io::Error::other("World tension share exceeds signed 128-bit range"))
}

impl Product for WorldTension {
    const KEY: &'static str = "world-tension";
    // Display text comes from the recording only.
    const CONTENT_DEPENDENT: bool = false;
    type Params = BranchParams;
    type Reply = TensionReply;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);

    fn types() -> Vec<String> {
        vec![
            BranchParams::decl(),
            TensionRead::decl(),
            TensionCountry::decl(),
            PreparingContributor::decl(),
            TensionAdded::decl(),
            TensionEvent::decl(),
            TensionReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch();
        let state = source.state::<row::Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache.entries.get(&branch).map(Arc::clone)
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(row::scan(&source.replay(), branch, cancelled)?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                Arc::clone(cache.entries.entry(branch).or_insert(scanned))
            }
        };
        let history = branch_histories(source, branch, false, &[], cancelled)?;
        scanned.reply(params, &history)
    }
}
impl ScannedTension {
    fn reply(
        &self,
        params: &BranchParams,
        history: &dashboard_base::products::country_history::RecordedHistories,
    ) -> io::Result<TensionReply> {
        let mut warnings = Vec::new();
        if self.read_count == 0 {
            warnings.push("No world tension was recorded in this branch.".to_owned());
        }
        if self.partial_sources > 0 {
            warnings.push(format!(
                "Reads whose threat sources were not all recorded: {}. Events they would name may be missing.",
                self.partial_sources
            ));
        }
        if self.aggregate_mismatches > 0 {
            warnings.push(format!(
                "Reads whose country contributions do not add up to their threat sources: {}.",
                self.aggregate_mismatches
            ));
        }
        if self.tension_mismatches > 0 {
            warnings.push(format!(
                "Reads whose world tension is not the game's cap of the contributions' sum: {}.",
                self.tension_mismatches
            ));
        }
        let unnamed = self.unnamed_reads;
        if unnamed > 0 {
            warnings.push(format!(
                "Reads with a contribution from a country the recording never names: {unnamed}. It is counted in Other."
            ));
        }
        let mut countries = self.countries.clone();
        for country in &mut countries {
            country.identities = Some(history.countries.get(&country.tag).map_or_else(
                Vec::new,
                |holder| {
                    holder
                        .identities
                        .iter()
                        .filter(|identity| identity.slot == country.slot)
                        .cloned()
                        .collect()
                },
            ));
        }
        let unproved = countries
            .iter()
            .zip(&self.first_contributions)
            .filter(|(country, first)| {
                !country.identities.as_ref().is_some_and(|identities| {
                    identities.iter().any(|identity| {
                        identity.first_hour <= **first && **first <= identity.last_hour
                    })
                })
            })
            .count();
        if unproved > 0 {
            warnings.push(format!("{unproved} contributor tag/slot entries have no proven native holder at their first contribution read; their counts remain exact, while country looks stay unknown."));
        }
        let reply = TensionReply {
            branch_id: params.branch_id.clone(),
            tension_scale: TENSION_SCALE as u32,
            contribution_scale: cthreat_source::FIXED_POINT_SCALE as u32,
            ceiling_raw: cworld_threat::CLAMP_CEILING.to_string(),
            reads: self.reads.clone(),
            countries,
            events: self.events.clone(),
            warnings,
            preparing_contributors: Vec::new(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
