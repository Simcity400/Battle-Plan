//! Ships and current hull HP in each country's task-force roster, hour by hour.
//! Each Ships row counts once; convoys outside that roster and stockpiles do not.
//! The cached scan keeps both roster and logical ownership. Content only names
//! the recorded sub-unit tokens and groups their native authored type masks.

use super::{
    LoadContext, Product,
    country_groups::HourSpan,
    country_history::{self, CountryIdentityRun, Identity},
    validate_tag,
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::IdentityTrack,
    ranking,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct NavySize;
pub use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
const CACHED_BRANCHES: usize = 2;
const MAX_EXACT: i64 = (1 << 53) - 1;

type ClassKey = (u32, Option<String>);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum NavyMeasure {
    Ships,
    Strength,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum NavyOwnership {
    Roster,
    Logical,
}

impl NavyOwnership {
    fn index(self) -> usize {
        match self {
            Self::Roster => 0,
            Self::Logical => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum NavyGrouping {
    Class,
    Type,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavyParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<u32>,
    pub ownership: NavyOwnership,
    pub grouping: NavyGrouping,
    pub measure: NavyMeasure,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavyPart {
    pub key: String,
    pub name: Option<String>,
    pub unresolved: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    #[ts(type = "number | null")]
    pub total: Option<i64>,
    #[ts(type = "Array<number | null>")]
    pub values: Vec<Option<i64>>,
    /// Maximum HP in fixed5 for the strength measure; unknown independently.
    #[ts(type = "number | null")]
    pub reference_total: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavySeries {
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
    pub runs: Vec<NavyRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavyReply {
    pub branch_id: String,
    pub parts: Vec<NavyPart>,
    pub series: Vec<NavySeries>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawNavyRun {
    first_hour: i32,
    last_hour: i32,
    starts_segment: bool,
    total: Option<i64>,
    /// Interned token/spelling identities, dated to the recorded hour.
    classes: Arc<BTreeMap<u32, i64>>,
    reference_total: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct ScannedNavy {
    classes: Vec<ClassKey>,
    series: [BTreeMap<String, Vec<RawNavyRun>>; 2],
    strength_series: [BTreeMap<String, Vec<RawNavyRun>>; 2],
    segments: BTreeMap<String, Vec<HourSpan>>,
    unattributed: [usize; 2],
    unread: usize,
    undisplayable_tags: BTreeMap<Vec<u8>, usize>,
    refused_strength: [usize; 2],
    maximum_overflows: [usize; 2],
    #[cfg(test)]
    updated_ships: usize,
}

impl ScannedNavy {
    fn series(&self, params: &NavyParams) -> &BTreeMap<String, Vec<RawNavyRun>> {
        match params.measure {
            NavyMeasure::Ships => &self.series[params.ownership.index()],
            NavyMeasure::Strength => &self.strength_series[params.ownership.index()],
        }
    }
}

#[derive(Default)]
struct Track {
    runs: [Vec<RawNavyRun>; 2],
    strength_runs: [Vec<RawNavyRun>; 2],
    last: Option<(Identity, i32)>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
}

fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Navy size request cancelled",
        ))
    } else {
        Ok(())
    }
}

mod scan;
use scan::scan;

#[derive(Default)]
struct Scans {
    entries: VecDeque<(u64, Arc<ScannedNavy>)>,
    runs: usize,
}

fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedNavy>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    check_cancel(cancelled)?;
    if let Some((_, scan)) = scans.entries.iter().find(|(cached, _)| *cached == branch) {
        return Ok(Arc::clone(scan));
    }
    let scan = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
    scans.runs += 1;
    scans.entries.push_back((branch, Arc::clone(&scan)));
    while scans.entries.len() > CACHED_BRANCHES {
        scans.entries.pop_front();
    }
    Ok(scan)
}

#[cfg(test)]
fn scans_run(source: &Source) -> usize {
    source
        .state::<Scans>()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .runs
}

/// Every tag observed in the branch and its roster-owned ship count at `hour`.
/// Missing hours, absent countries and incomplete joins stay None. Uses the
/// widget's same per-source branch scan and needs no game-content context.
/// Tags that cannot be represented as UTF-8 refuse the map by name and count.
pub fn ships_at(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Option<i64>>> {
    let scanned = scanned_branch(source, branch, cancelled)?;
    super::require_displayable_tags(&scanned.undisplayable_tags)?;
    scanned.series[0]
        .iter()
        .map(|(tag, runs)| {
            let index = runs.partition_point(|run| run.first_hour <= hour);
            let value = index
                .checked_sub(1)
                .and_then(|index| runs.get(index))
                .filter(|run| hour <= run.last_hour)
                .and_then(|run| run.total);
            check_cancel(cancelled)?;
            Ok((tag.clone(), value))
        })
        .collect::<io::Result<_>>()
}

/// Visit requested tags' roster-ship runs from the widget's cached branch scan.
/// Bounds, breaks and explicit unknowns are copied without coalescing; gaps and
/// unobserved tags emit nothing. Inputs are hourly censuses, not daily carries.
/// Callback errors and cancellation abort the visit; discard any partial result.
pub fn visit_ship_runs(
    source: &Source,
    branch: u64,
    tags: &[String],
    cancelled: &AtomicBool,
    mut visit: impl FnMut(&str, i32, i32, bool, Option<i64>) -> io::Result<()>,
) -> io::Result<()> {
    let scanned = scanned_branch(source, branch, cancelled)?;
    super::require_displayable_tags(&scanned.undisplayable_tags)?;
    for tag in tags {
        check_cancel(cancelled)?;
        for run in scanned.series[0].get(tag).into_iter().flatten() {
            check_cancel(cancelled)?;
            visit(
                tag,
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.total,
            )?;
        }
    }
    check_cancel(cancelled)
}

#[derive(Default)]
struct UnitContent {
    catalogue: Option<(Arc<Catalogue>, Arc<installed_content::unit_leader::SubUnits>)>,
}

/// Project recorded sub-unit tokens through the shared native content reader.
/// Composite masks remain one broad class, so a multi-category ship counts once.
fn parts(
    classes: &BTreeSet<u32>,
    catalogue: &[ClassKey],
    units: Option<&installed_content::unit_leader::SubUnits>,
    grouping: NavyGrouping,
    localise: impl Fn(&str) -> Option<String>,
) -> (Vec<NavyPart>, BTreeMap<u32, usize>, Vec<String>) {
    let mut definitions: BTreeMap<String, NavyPart> = BTreeMap::new();
    let mut tokens = BTreeMap::new();
    let mut warnings = Vec::new();
    for &class in classes {
        let (token, spelling) = &catalogue[class as usize];
        let mut refusal = None;
        let resolved = spelling.as_ref().zip(units).and_then(|(spelling, units)| {
            let registered = match units.get(spelling) {
                installed_content::unit_leader::SubUnit::Index(index) => {
                    units.name(index).and_then(Result::ok)
                }
                _ => None,
            }?;
            match grouping {
                NavyGrouping::Class => Some((
                    format!("class:{registered}"),
                    Some(localise(registered).unwrap_or_else(|| registered.to_owned())),
                )),
                NavyGrouping::Type => {
                    let mask = units.category(registered.as_bytes()).ok()?;
                    let types = match installed_content::equipment::category_names(mask) {
                        Ok(types) => types,
                        Err(bits) => {
                            refusal = Some(format!("unknown native category bits {bits:#x}"));
                            return None;
                        }
                    };
                    if types.is_empty() {
                        return None;
                    }
                    let name = types
                        .iter()
                        .map(|key| localise(key).unwrap_or_else(|| (*key).to_owned()))
                        .collect::<Vec<_>>()
                        .join(" / ");
                    Some((format!("type:{mask}"), Some(name)))
                }
            }
        });
        let (key, name, unresolved) = match resolved {
            Some((key, name)) => (key, name, false),
            None => {
                let visible = spelling
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .map_or_else(
                        || format!("Other / not identified (Sub-unit {token})"),
                        str::to_owned,
                    );
                let reason = refusal.as_deref().unwrap_or(
                    "the recorded content does not resolve this ship class or broad role",
                );
                warnings.push(format!("{visible}: {reason}; kept as its own part."));
                (
                    format!(
                        "{}ship:{}",
                        super::equipment::UNRESOLVED_MARK,
                        serde_json::to_string(&(token, spelling)).expect("class identity")
                    ),
                    Some(visible),
                    true,
                )
            }
        };
        definitions.entry(key.clone()).or_insert(NavyPart {
            key: key.clone(),
            name,
            unresolved,
        });
        tokens.insert(class, key);
    }
    let parts: Vec<_> = definitions.into_values().collect();
    let positions: HashMap<&str, usize> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| (part.key.as_str(), index))
        .collect();
    let tokens = tokens
        .into_iter()
        .map(|(token, key)| (token, positions[key.as_str()]))
        .collect();
    (parts, tokens, warnings)
}

/// Rank and project the cached scan, then enforce the serialized reply budget.
fn reply(
    scanned: &ScannedNavy,
    params: &NavyParams,
    units: Option<&installed_content::unit_leader::SubUnits>,
    localise: impl Fn(&str) -> Option<String>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<NavyReply> {
    check_cancel(cancelled)?;
    let source = scanned.series(params);
    let membership = match params.top {
        Some(count) => Some(ranking::top_membership(
            source,
            count as usize,
            |run| (run.first_hour, run.last_hour),
            |run| run.total,
            cancelled,
        )?),
        None => None,
    };
    let tags: Vec<&str> = match &membership {
        Some(membership) => membership.keys().map(String::as_str).collect(),
        None => {
            let mut tags: Vec<_> = params.tags.iter().map(String::as_str).collect();
            tags.sort_unstable();
            tags.dedup();
            tags
        }
    };
    let classes = tags
        .iter()
        .flat_map(|tag| source.get(*tag).into_iter().flatten())
        .flat_map(|run| run.classes.keys().copied())
        .collect();
    let (parts, positions, notes) =
        parts(&classes, &scanned.classes, units, params.grouping, localise);
    warnings.extend(notes);
    let mut series = Vec::with_capacity(tags.len());
    for tag in tags {
        check_cancel(cancelled)?;
        let Some(raw) = source.get(tag) else {
            continue;
        };
        let segments = scanned
            .segments
            .get(tag)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let spans = membership
            .as_ref()
            .and_then(|membership| membership.get(tag));
        let mut runs = Vec::new();
        let mut project = |run: &RawNavyRun, first_hour: i32, last_hour: i32| {
            if run
                .total
                .into_iter()
                .chain(run.reference_total)
                .any(|value| value > MAX_EXACT)
            {
                return Err(io::Error::other(
                    "Navy reply value exceeds JavaScript's exact integer range",
                ));
            }
            let mut values = vec![run.total.map(|_| 0); parts.len()];
            if run.total.is_some() {
                for (token, count) in run.classes.iter() {
                    let value = &mut values[positions[token]];
                    *value.as_mut().expect("known count") += count;
                }
            }
            runs.push(NavyRun {
                first_hour,
                last_hour,
                starts_segment: run.starts_segment && first_hour == run.first_hour,
                total: run.total,
                values,
                reference_total: run.reference_total,
            });
            Ok(())
        };
        match spans {
            Some(spans) => ranking::for_each_piece(
                raw,
                spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                &mut project,
            )?,
            None => {
                for run in raw {
                    project(run, run.first_hour, run.last_hour)?;
                }
            }
        }
        series.push(NavySeries {
            tag: tag.to_owned(),
            identities: Vec::new(),
            runs,
            segments: segments.to_vec(),
            membership: spans.cloned(),
        });
    }
    if scanned.unread > 0 {
        warnings.push(format!(
            "{} hours have incomplete ship, task-force or country tables; counts stay unknown.",
            scanned.unread
        ));
    }
    if params.measure == NavyMeasure::Strength {
        let owner = params.ownership.index();
        for (count, reason) in [
            (
                scanned.refused_strength[owner],
                "have negative or overflowing native hull HP; their current hull strength stays unknown",
            ),
            (
                scanned.maximum_overflows[owner],
                "have overflowing maximum hull HP; their maximum hull strength stays unknown",
            ),
        ] {
            if count > 0 {
                warnings.push(format!("{count} country-hours {reason}."));
            }
        }
    }
    let unattributed = scanned.unattributed[params.ownership.index()];
    if unattributed > 0 {
        warnings.push(format!("{unattributed} ship observations name no single recorded owner; every country's count at those hours stays unknown."));
    }
    for (tag, count) in &scanned.undisplayable_tags {
        warnings.push(format!(
            "Country tag bytes {tag:?}: {count} observations are not UTF-8 and cannot be displayed."
        ));
    }
    let reply = NavyReply {
        branch_id: params.branch_id.clone(),
        parts,
        series,
        warnings,
    };
    Ok(reply)
}

fn checked_reply(reply: NavyReply) -> io::Result<NavyReply> {
    if crate::transport::json_len(&reply, MAX_REPLY_BYTES).is_none() {
        return Err(io::Error::other(
            "The navy size reply exceeds its transport budget",
        ));
    }
    Ok(reply)
}

impl Product for NavySize {
    const KEY: &'static str = "navy-size";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavyParams;
    type Reply = NavyReply;
    fn types() -> Vec<String> {
        vec![
            NavyParams::decl(),
            NavyOwnership::decl(),
            NavyGrouping::decl(),
            NavyMeasure::decl(),
            NavyPart::decl(),
            NavyRun::decl(),
            NavySeries::decl(),
            CountryIdentityRun::decl(),
            NavyReply::decl(),
        ]
    }
    fn validate(params: &NavyParams) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if let Some(top) = params.top
            && (!params.tags.is_empty() || top == 0)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &NavyParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<NavyReply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let scanned = scanned_branch(source, branch, cancelled)?;
        let history = country_history::branch_histories(source, branch, false, &[], cancelled)?;
        let mut selection = params.clone();
        if selection.top.is_none() {
            selection.tags = country_history::selected_aliases(&history, &params.tags);
        }
        let mut warnings = Vec::new();
        let catalogue = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let mut result = match catalogue {
            Ok(catalogue) => {
                warnings.extend(catalogue.verification.iter().cloned());
                warnings.extend(catalogue.warnings.iter().cloned());
                let state = source.state::<UnitContent>();
                let mut content = state.lock().unwrap_or_else(|error| error.into_inner());
                if !content
                    .catalogue
                    .as_ref()
                    .is_some_and(|(old, _)| Arc::ptr_eq(old, &catalogue))
                {
                    match installed_content::unit_leader::sub_units(&catalogue.content.layers) {
                        Ok(units) => {
                            content.catalogue = Some((Arc::clone(&catalogue), Arc::new(units)))
                        }
                        Err(error) => {
                            warnings.push(format!("Ship class content is unavailable: {error}"));
                            content.catalogue = None;
                        }
                    }
                }
                let units = content
                    .catalogue
                    .as_ref()
                    .map(|(_, units)| Arc::clone(units));
                drop(content);
                if let Some(units) = &units {
                    warnings.extend(units.warnings().iter().cloned());
                }
                let mut result = reply(
                    &scanned,
                    &selection,
                    units.as_deref(),
                    |key| {
                        catalogue.localisation.get(key)?;
                        catalogue.localised(key, catalogue::View::Plain)
                    },
                    warnings,
                    cancelled,
                )?;
                result.warnings.extend(catalogue.refusal_lines());
                Ok(result)
            }
            Err(error) => {
                warnings.push(format!("Ship class names are unavailable: {error}"));
                reply(&scanned, &selection, None, |_| None, warnings, cancelled)
            }
        }?;
        for series in &mut result.series {
            if let Some(country) = history.countries.get(&series.tag) {
                series.identities = country.identities.as_ref().clone();
            }
        }
        checked_reply(result)
    }
}

#[cfg(test)]
mod tests;
