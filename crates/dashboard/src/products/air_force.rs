//! Planes in wings, and wings, over recorded hourly censuses. Stockpiled planes
//! are outside this measure. `CAirWing::AddEquipmentPool@140F45AD0` and
//! `AddAirplanes@140F45C50` set +0x7C to SumEquipmentQuantities / 100000;
//! the pool fuel reader `140F46340` consumes that signed whole-plane count.
//!
//! A single source/branch scan serves both ownership modes, both widget measures
//! and the Scoreboard count APIs. Content resolution happens after that scan,
//! using each variant's token at the observation, never a later daily definition.

mod reply;
mod scan;
#[cfg(test)]
mod tests;

use super::{
    LoadContext, Product,
    country_groups::HourSpan,
    country_history::{self, CountryIdentityRun},
    validate_tag,
};
use crate::{Source, catalogue, transport};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct AirForceSize;
pub use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
const MAX_EXACT: i64 = (1 << 53) - 1;
const UNIDENTIFIED: &str = ":not-identified";
const CACHED_BRANCHES: usize = 2;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum AirForceMeasure {
    Planes,
    Wings,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum AirForceOwnership {
    Roster,
    Effective,
}

impl AirForceOwnership {
    fn index(self) -> usize {
        match self {
            Self::Roster => 0,
            Self::Effective => 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirForceParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<u32>,
    pub measure: AirForceMeasure,
    pub ownership: AirForceOwnership,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirForcePart {
    pub key: String,
    pub name: Option<String>,
    pub unresolved: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirForceRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    /// Null when the census or ownership attribution is incomplete.
    #[ts(type = "number | null")]
    pub total: Option<i64>,
    /// Whole counts parallel to reply.parts; unresolved content retains its own
    /// numeric part. A quantity that cannot split exactly is null, never zero.
    #[ts(type = "Array<number | null>")]
    pub values: Vec<Option<i64>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirForceSeries {
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
    pub runs: Vec<AirForceRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AirForceReply {
    pub branch_id: String,
    pub parts: Vec<AirForcePart>,
    pub series: Vec<AirForceSeries>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Counts {
    planes: i64,
    wings: i64,
    /// Recorded equipment token (or unresolved variant handle) to whole planes.
    planes_by_token: BTreeMap<String, Option<i64>>,
    /// Each wing counts once under its set of observed equipment tokens. Reply
    /// content may collapse several tokens to one authored archetype.
    wings_by_tokens: BTreeMap<Vec<String>, i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawRun {
    first_hour: i32,
    last_hour: i32,
    starts_segment: bool,
    roster: Option<Arc<Counts>>,
    effective: Option<Arc<Counts>>,
}

impl RawRun {
    fn counts(&self, ownership: AirForceOwnership) -> Option<&Counts> {
        match ownership {
            AirForceOwnership::Roster => self.roster.as_deref(),
            AirForceOwnership::Effective => self.effective.as_deref(),
        }
    }

    fn total(&self, params: &AirForceParams) -> Option<i64> {
        self.counts(params.ownership)
            .map(|counts| match params.measure {
                AirForceMeasure::Planes => counts.planes,
                AirForceMeasure::Wings => counts.wings,
            })
    }
}

#[derive(Debug, Default)]
struct ScannedAir {
    series: BTreeMap<String, Vec<RawRun>>,
    segments: BTreeMap<String, Vec<HourSpan>>,
    unreadable_owners: [usize; 2],
    equipment_missing: usize,
    equipment_refused: usize,
    equipment_remainders: usize,
    #[cfg(test)]
    reconciled_wings: usize,
    #[cfg(test)]
    projected_countries: usize,
    undisplayable_tags: BTreeMap<Vec<u8>, usize>,
}

#[derive(Default)]
struct Scans {
    entries: VecDeque<(u64, Arc<ScannedAir>)>,
    runs: usize,
}

fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedAir>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    reply::cancel(cancelled)?;
    if let Some((_, scanned)) = scans.entries.iter().find(|(cached, _)| *cached == branch) {
        return Ok(Arc::clone(scanned));
    }
    let scanned = Arc::new(scan::scan(source.replay_ref(), branch, cancelled)?);
    scans.runs += 1;
    scans.entries.push_back((branch, Arc::clone(&scanned)));
    while scans.entries.len() > CACHED_BRANCHES {
        scans.entries.pop_front();
    }
    Ok(scanned)
}

/// Every country observed in this branch, with its roster-owned planes at this
/// exact hour. Missing census, gaps, incomplete ownership and countries absent
/// at the hour are None. Only a complete census proves an existing country's 0.
/// The widget and Scoreboard reuse the same per-source branch scan.
/// Tags that cannot be represented as UTF-8 refuse the map by name and count.
pub fn planes_at(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Option<i64>>> {
    let scanned = scanned_branch(source, branch, cancelled)?;
    super::require_displayable_tags(&scanned.undisplayable_tags)?;
    scanned
        .series
        .iter()
        .map(|(tag, runs)| {
            let at = runs.partition_point(|run| run.first_hour <= hour);
            let value = at
                .checked_sub(1)
                .and_then(|index| runs.get(index))
                .filter(|run| hour <= run.last_hour)
                .and_then(|run| run.roster.as_ref())
                .map(|counts| counts.planes);
            reply::cancel(cancelled)?;
            Ok((tag.clone(), value))
        })
        .collect::<io::Result<_>>()
}

/// Visit requested tags' roster-plane runs from the widget's cached branch scan.
/// Bounds, breaks and explicit unknowns are copied without coalescing; gaps and
/// unobserved tags emit nothing. Inputs are hourly censuses, not daily carries.
/// Callback errors and cancellation abort the visit; discard any partial result.
pub fn visit_plane_runs(
    source: &Source,
    branch: u64,
    tags: &[String],
    cancelled: &AtomicBool,
    mut visit: impl FnMut(&str, i32, i32, bool, Option<i64>) -> io::Result<()>,
) -> io::Result<()> {
    let scanned = scanned_branch(source, branch, cancelled)?;
    super::require_displayable_tags(&scanned.undisplayable_tags)?;
    for tag in tags {
        reply::cancel(cancelled)?;
        for run in scanned.series.get(tag).into_iter().flatten() {
            reply::cancel(cancelled)?;
            visit(
                tag,
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.roster.as_ref().map(|counts| counts.planes),
            )?;
        }
    }
    reply::cancel(cancelled)
}

impl Product for AirForceSize {
    const KEY: &'static str = "air-force-size";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirForceParams;
    type Reply = AirForceReply;

    fn types() -> Vec<String> {
        vec![
            AirForceParams::decl(),
            AirForceMeasure::decl(),
            AirForceOwnership::decl(),
            AirForcePart::decl(),
            AirForceRun::decl(),
            AirForceSeries::decl(),
            CountryIdentityRun::decl(),
            AirForceReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params
            .top
            .is_some_and(|count| !params.tags.is_empty() || count == 0)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().expect("validated branch");
        let scanned = scanned_branch(source, branch, cancelled)?;
        let history = country_history::branch_histories(source, branch, false, &[], cancelled)?;
        let mut selection = params.clone();
        if selection.top.is_none() {
            selection.tags = country_history::selected_aliases(&history, &params.tags);
        }
        let mut warnings = Vec::new();
        let catalogue = match context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        {
            Ok(catalogue) => {
                warnings.extend(catalogue.verification.clone());
                warnings.extend(catalogue.warnings.iter().cloned());
                Some(catalogue)
            }
            Err(error) => {
                warnings.push(format!("Air equipment type names are unavailable: {error}"));
                None
            }
        };
        let units = catalogue.as_ref().and_then(|catalogue| {
            match super::equipment::units(source, catalogue) {
                Ok(units) => {
                    warnings.extend(super::equipment::equipment_warnings(&units));
                    Some(units)
                }
                Err(error) => {
                    warnings.push(format!("Air equipment content is unavailable: {error}"));
                    None
                }
            }
        });
        let mut reply = reply::reply(
            &scanned,
            &selection,
            units.as_deref(),
            |token| {
                let catalogue = catalogue.as_ref()?;
                catalogue.localisation.get(token)?;
                catalogue.localised(token, catalogue::View::Plain)
            },
            warnings,
            cancelled,
        )?;
        for series in &mut reply.series {
            if let Some(country) = history.countries.get(&series.tag) {
                series.identities = country.identities.as_ref().clone();
            }
        }
        if let Some(catalogue) = catalogue {
            reply.warnings.extend(catalogue.refusal_lines());
        }
        if transport::json_len(&reply, MAX_REPLY_BYTES).is_none() {
            return Err(io::Error::other(
                "Air force history exceeds the reply budget; choose fewer countries",
            ));
        }
        Ok(reply)
    }
}
