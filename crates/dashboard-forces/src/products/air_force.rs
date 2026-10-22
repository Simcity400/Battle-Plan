//! Planes in wings, and wings, over recorded hourly censuses. Stockpiled planes
//! are outside this measure. The wing's whole-plane count truncates its fixed5
//! equipment-pool sum divided by 100000; fuel use reads that signed count.
//!
//! A single source/branch scan serves both ownership modes, both widget measures
//! and the test-only Scoreboard count oracle. Content resolution happens after
//! that scan, using each variant's token at the observation, never a later daily definition.

pub mod prepared;
mod reply;
mod scan;
#[cfg(test)]
mod tests;

use dashboard_base::{
    catalogue,
    products::{
        Product,
        country_groups::HourSpan,
        country_history::{self, CountryIdentityRun},
    },
    source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct AirForceSize;
pub use dashboard_base::products::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Counts {
    planes: i64,
    wings: i64,
    /// Recorded equipment token (or unresolved variant handle) to whole planes.
    planes_by_token: BTreeMap<String, Option<i64>>,
    /// Each wing counts once under its set of observed equipment tokens. Reply
    /// content may collapse several tokens to one authored archetype.
    #[serde(with = "map_rows")]
    wings_by_tokens: BTreeMap<Vec<String>, i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRun {
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedAir {
    #[serde(skip)]
    series: BTreeMap<String, Vec<RawRun>>,
    #[serde(skip)]
    segments: BTreeMap<String, Vec<HourSpan>>,
    unreadable_owners: [usize; 2],
    equipment_missing: usize,
    equipment_refused: usize,
    equipment_remainders: usize,
    #[cfg(test)]
    reconciled_wings: usize,
    #[cfg(test)]
    projected_countries: usize,
    #[serde(with = "map_rows")]
    pub undisplayable_tags: BTreeMap<Vec<u8>, usize>,
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

/// Visit requested tags' roster-plane runs from the widget's cached branch scan.
/// Bounds, breaks and explicit unknowns are copied without coalescing; gaps and
/// unobserved tags emit nothing. Inputs are hourly censuses, not daily carries.
/// Callback errors and cancellation abort the visit; discard any partial result.
#[cfg(any(test, feature = "test-support"))]
pub fn visit_plane_runs(
    source: &Source,
    branch: u64,
    tags: &[String],
    cancelled: &AtomicBool,
    mut visit: impl FnMut(&str, i32, i32, bool, Option<i64>) -> io::Result<()>,
) -> io::Result<()> {
    let scanned = scanned_branch(source, branch, cancelled)?;
    dashboard_base::products::require_displayable_tags(&scanned.undisplayable_tags)?;
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
        present(
            source,
            params,
            context,
            cancelled,
            &scanned,
            Some(&history),
            None,
        )
    }
}

// Native map keys are sequences; JSON continuation state stores their entries.
pub(super) mod map_rows {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;
    pub fn serialize<K: Serialize, V: Serialize, S: Serializer>(
        map: &BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<
        'de,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    >(
        deserializer: D,
    ) -> Result<BTreeMap<K, V>, D::Error> {
        Ok(Vec::<(K, V)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

/// The parts of the last catalogue read, kept apart for the recording's own catalogue (which
/// preparation configures) and the shown one ([`catalogue::ByCatalogue`]).
#[derive(Default)]
struct PartContent {
    binding: Option<(Option<Arc<catalogue::Catalogue>>, Arc<reply::Parts>)>,
}
fn part_content(
    source: &Source,
    catalogue: Option<&Arc<catalogue::Catalogue>>,
) -> Arc<reply::Parts> {
    let state = source.state::<catalogue::ByCatalogue<PartContent>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let state = state.of_optional(catalogue.map(|catalogue| &**catalogue));
    if !state
        .binding
        .as_ref()
        .is_some_and(|(old, _)| match (old.as_ref(), catalogue) {
            (Some(old), Some(current)) => Arc::ptr_eq(old, current),
            (None, None) => true,
            _ => false,
        })
    {
        state.binding = Some((catalogue.cloned(), Arc::default()));
    }
    Arc::clone(&state.binding.as_ref().expect("air part binding").1)
}
fn configure(source: &Source, context: &LoadContext) {
    if let Ok(catalogue) = context
        .content()
        .and_then(|options| catalogue::catalogue(source, options))
    {
        let _ = dashboard_base::products::equipment::units(source, &catalogue);
        part_content(source, Some(&catalogue));
    }
}

#[allow(clippy::too_many_arguments)]
fn present(
    source: &Source,
    params: &AirForceParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
    scanned: &ScannedAir,
    history: Option<&country_history::RecordedHistories>,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<AirForceReply> {
    let mut selection = params.clone();
    if selection.top.is_none()
        && let Some(history) = history
    {
        selection.tags = country_history::selected_aliases(history, &params.tags);
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
        match dashboard_base::products::equipment::units(source, catalogue) {
            Ok(units) => {
                warnings.extend(dashboard_base::products::equipment::equipment_warnings(
                    &units,
                ));
                Some(units)
            }
            Err(error) => {
                warnings.push(format!("Air equipment content is unavailable: {error}"));
                None
            }
        }
    });
    let localise = |token: &str| {
        let catalogue = catalogue.as_ref()?;
        catalogue.localisation.get(token)?;
        catalogue.localised(token, catalogue::View::Plain)
    };
    let parts = part_content(source, catalogue.as_ref());
    let mut reply = if let Some(membership) = membership {
        reply::selected(
            scanned,
            &selection,
            units.as_deref(),
            localise,
            warnings,
            cancelled,
            Some(membership),
            Some(&parts),
        )?
    } else {
        reply::reply_cached(
            scanned,
            &selection,
            units.as_deref(),
            localise,
            warnings,
            cancelled,
            Some(&parts),
        )?
    };
    for series in &mut reply.series {
        if let Some(country) = history.and_then(|history| history.countries.get(&series.tag)) {
            series.identities = country.identities.as_ref().clone();
        }
    }
    if let Some(catalogue) = catalogue {
        reply.warnings.extend(catalogue.refusal_lines());
    }
    dashboard_base::products::check_reply_budget(&reply).map_err(|_| {
        io::Error::other("Air force history exceeds the reply budget; choose fewer countries")
    })?;
    Ok(reply)
}
