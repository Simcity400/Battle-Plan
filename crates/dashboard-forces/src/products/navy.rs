//! Ships and current hull HP in each country's task-force roster, hour by hour, prepared as the
//! `navy` series family ([`facts`], [`emit`]) and answered from its publication ([`query`]),
//! which the scoreboard reads too. Each Ships row counts once; convoys outside that roster and
//! stockpiles do not. Both roster and logical ownership are kept. Content only names the
//! recorded sub-unit tokens and groups their native authored type masks.

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
mod scan;
pub mod served;

use dashboard_base::{
    catalogue,
    products::{
        Product,
        country_groups::HourSpan,
        country_history::{self, CountryIdentityRun},
    },
    ranking,
    source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag};
use dashboard_text::catalogue::Catalogue;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct NavySize;
pub use dashboard_base::products::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNavyRun {
    first_hour: i32,
    last_hour: i32,
    starts_segment: bool,
    total: Option<i64>,
    /// Interned token/spelling identities, dated to the recorded hour.
    classes: Arc<BTreeMap<u32, i64>>,
    reference_total: Option<i64>,
}

impl RawNavyRun {
    fn displayable(&self) -> bool {
        self.total
            .into_iter()
            .chain(self.reference_total)
            .all(|value| value <= MAX_EXACT)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedNavy {
    classes: Arc<Vec<ClassKey>>,
    series: [BTreeMap<String, Vec<RawNavyRun>>; 2],
    strength_series: [BTreeMap<String, Vec<RawNavyRun>>; 2],
    segments: BTreeMap<String, Vec<HourSpan>>,
    unattributed: [usize; 2],
    unread: usize,
    #[serde(with = "super::air_force::map_rows")]
    pub undisplayable_tags: BTreeMap<Vec<u8>, usize>,
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

    /// This scan's series of `spans`' tags only, each cut to its tag's spans.
    #[cfg(any(test, feature = "test-support"))]
    fn held(
        &self,
        spans: &BTreeMap<String, Vec<HourSpan>>,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let held = |all: &BTreeMap<String, Vec<RawNavyRun>>| -> io::Result<_> {
            let mut series = BTreeMap::new();
            for (tag, spans) in spans {
                let Some(runs) = all.get(tag) else {
                    continue;
                };
                let mut pieces = Vec::new();
                ranking::for_each_piece(
                    runs,
                    spans,
                    |run| (run.first_hour, run.last_hour),
                    cancelled,
                    |run, first_hour, last_hour| {
                        pieces.push(RawNavyRun {
                            first_hour,
                            last_hour,
                            starts_segment: run.starts_segment && first_hour == run.first_hour,
                            ..run.clone()
                        });
                        Ok(())
                    },
                )?;
                series.insert(tag.clone(), pieces);
            }
            Ok(series)
        };
        let [roster, logical] = &self.series;
        let [roster_strength, logical_strength] = &self.strength_series;
        Ok(Self {
            classes: Arc::clone(&self.classes),
            series: [held(roster)?, held(logical)?],
            strength_series: [held(roster_strength)?, held(logical_strength)?],
            segments: spans
                .keys()
                .filter_map(|tag| Some((tag.clone(), self.segments.get(tag)?.clone())))
                .collect(),
            unattributed: self.unattributed,
            unread: self.unread,
            undisplayable_tags: self.undisplayable_tags.clone(),
            refused_strength: self.refused_strength,
            maximum_overflows: self.maximum_overflows,
            #[cfg(test)]
            updated_ships: self.updated_ships,
        })
    }
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

#[cfg(any(test, feature = "test-support"))]
use scan::scan;

/// One scan per branch of the source revision.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Scans {
    entries: std::collections::BTreeMap<u64, Arc<ScannedNavy>>,
    runs: usize,
}

#[cfg(any(test, feature = "test-support"))]
fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedNavy>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    check_cancel(cancelled)?;
    if let Some(scan) = scans.entries.get(&branch) {
        return Ok(Arc::clone(scan));
    }
    let scan = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
    scans.runs += 1;
    scans.entries.insert(branch, Arc::clone(&scan));
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

/// Visit requested tags' roster-ship runs from the widget's cached branch scan.
/// Bounds, breaks and explicit unknowns are copied without coalescing; gaps and
/// unobserved tags emit nothing. Inputs are hourly censuses, not daily carries.
/// Callback errors and cancellation abort the visit; discard any partial result.
#[cfg(any(test, feature = "test-support"))]
pub fn visit_ship_runs(
    source: &Source,
    branch: u64,
    tags: &[String],
    cancelled: &AtomicBool,
    mut visit: impl FnMut(&str, i32, i32, bool, Option<i64>) -> io::Result<()>,
) -> io::Result<()> {
    let scanned = scanned_branch(source, branch, cancelled)?;
    dashboard_base::products::require_displayable_tags(&scanned.undisplayable_tags)?;
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

type PartResults = catalogue::KeyedResults<(ClassKey, usize), (NavyPart, Vec<String>)>;
/// The parts of the last catalogue read, kept apart for the recording's own catalogue (which
/// preparation configures) and the shown one ([`catalogue::ByCatalogue`]).
#[derive(Default)]
struct UnitContent {
    binding: Option<(Option<Arc<Catalogue>>, Arc<PartResults>)>,
}
fn part_content(source: &Source, catalogue: Option<&Arc<Catalogue>>) -> Arc<PartResults> {
    let state = source.state::<catalogue::ByCatalogue<UnitContent>>();
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
    Arc::clone(&state.binding.as_ref().expect("navy part binding").1)
}

/// The part a recorded class counts under in `grouping`, and why it stays unresolved.
/// Composite masks remain one broad class, so a multi-category ship counts once.
fn class_part(
    class_key: &ClassKey,
    units: Option<&installed_content_core::unit_leader::SubUnits>,
    grouping: NavyGrouping,
    localise: &dyn Fn(&str) -> Option<String>,
) -> (NavyPart, Vec<String>) {
    let (token, spelling) = class_key;
    let mut warnings = Vec::new();
    let mut refusal = None;
    let resolved = spelling.as_ref().zip(units).and_then(|(spelling, units)| {
        let registered = match units.get(spelling) {
            installed_content_core::unit_leader::SubUnit::Index(index) => {
                units.name(index).and_then(Result::ok)
            }
            _ => None,
        }?;
        match grouping {
            NavyGrouping::Class => Some((format!("class:{registered}"), localise(registered))),
            NavyGrouping::Type => {
                let mask = units.category(registered.as_bytes()).ok()?;
                let types = match installed_content_core::equipment::category_names(mask) {
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
                    .map(|key| localise(key))
                    .collect::<Option<Vec<_>>>()
                    .map(|names| names.join(" / "));
                Some((format!("type:{mask}"), name))
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
            let reason = refusal
                .as_deref()
                .unwrap_or("the recorded content does not resolve this ship class or broad role");
            warnings.push(format!("{visible}: {reason}; kept as its own part."));
            (
                format!(
                    "{}ship:{}",
                    dashboard_base::products::equipment::UNRESOLVED_MARK,
                    serde_json::to_string(&(token, spelling)).expect("class identity")
                ),
                Some("Ship class name unavailable".into()),
                true,
            )
        }
    };
    (
        NavyPart {
            key,
            name,
            unresolved,
        },
        warnings,
    )
}

/// The part key a recorded class counts under in `grouping`, as replies name it.
pub(super) fn part_key(
    class_key: &ClassKey,
    units: Option<&installed_content_core::unit_leader::SubUnits>,
    grouping: NavyGrouping,
) -> String {
    class_part(class_key, units, grouping, &|_: &str| None).0.key
}

/// Project recorded sub-unit tokens through the shared native content reader.
fn parts(
    classes: &BTreeSet<u32>,
    catalogue: &[ClassKey],
    units: Option<&installed_content_core::unit_leader::SubUnits>,
    grouping: NavyGrouping,
    localise: impl Fn(&str) -> Option<String>,
    cache: Option<&PartResults>,
) -> (Vec<NavyPart>, BTreeMap<u32, usize>, Vec<String>) {
    let mut definitions: BTreeMap<String, NavyPart> = BTreeMap::new();
    let mut tokens = BTreeMap::new();
    let mut warnings = Vec::new();
    for &class in classes {
        let class_key = &catalogue[class as usize];
        let resolve = |class_key: &ClassKey| class_part(class_key, units, grouping, &localise);
        let resolved = match cache {
            Some(cache) => cache
                .get_or_init(
                    &(
                        class_key.clone(),
                        usize::from(grouping == NavyGrouping::Type),
                    ),
                    |(key, _)| Ok(resolve(key)),
                )
                .expect("native class projection is infallible"),
            None => Arc::new(resolve(class_key)),
        };
        let key = resolved.0.key.clone();
        warnings.extend(resolved.1.iter().cloned());
        definitions
            .entry(key.clone())
            .or_insert_with(|| resolved.0.clone());
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

#[cfg(test)]
fn reply(
    scanned: &ScannedNavy,
    params: &NavyParams,
    units: Option<&installed_content_core::unit_leader::SubUnits>,
    localise: impl Fn(&str) -> Option<String>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<NavyReply> {
    reply_cached(scanned, params, units, localise, warnings, cancelled, None)
}

/// Rank and project the cached scan, then enforce the serialized reply budget.
fn reply_cached(
    scanned: &ScannedNavy,
    params: &NavyParams,
    units: Option<&installed_content_core::unit_leader::SubUnits>,
    localise: impl Fn(&str) -> Option<String>,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
    cache: Option<&PartResults>,
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
    selected(
        scanned, params, units, localise, warnings, cancelled, membership, cache,
    )
}
#[allow(clippy::too_many_arguments)]
fn selected(
    scanned: &ScannedNavy,
    params: &NavyParams,
    units: Option<&installed_content_core::unit_leader::SubUnits>,
    localise: impl Fn(&str) -> Option<String>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
    cache: Option<&PartResults>,
) -> io::Result<NavyReply> {
    let source = scanned.series(params);
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
    let (parts, positions, notes) = parts(
        &classes,
        &scanned.classes,
        units,
        params.grouping,
        localise,
        cache,
    );
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
            if !run.displayable() {
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
    if dashboard_core::transport::json_len(&reply, MAX_REPLY_BYTES).is_none() {
        return Err(dashboard_base::products::over_budget(
            "The navy size reply exceeds its transport budget",
        ));
    }
    Ok(reply)
}

impl Product for NavySize {
    const KEY: &'static str = "navy-size";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
    /// The row census's reply: the parity oracle of the publication's.
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &NavyParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<NavyReply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let mut scanned = scanned_branch(source, branch, cancelled)?;
        let history = country_history::branch_histories(source, branch, false, &[], cancelled)?;
        let mut selection = params.clone();
        // An alias counts only while a picked country holds it, as the prepared query counts it.
        if params.top.is_none() {
            let spans = country_history::selected_spans(
                &history,
                &params.tags,
                &BTreeMap::new(),
                cancelled,
            )?;
            scanned = Arc::new(scanned.held(&spans, cancelled)?);
            selection.tags = country_history::selected_aliases(&history, &params.tags);
        }
        present(
            source,
            &selection,
            context,
            cancelled,
            &scanned,
            Some(&history),
            None,
        )
    }
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;

pub fn content_units(
    catalogue: &Arc<Catalogue>,
    warnings: &mut Vec<String>,
) -> Option<Arc<installed_content_core::unit_leader::SubUnits>> {
    let database = catalogue
        .content
        .layers
        .derived::<catalogue::CachedResult<installed_content_core::unit_leader::SubUnits>>();
    match database.get_or_init(|| installed_content_core::unit_leader::sub_units(&catalogue.content.layers))
    {
        Ok(units) => {
            warnings.extend(units.warnings().iter().cloned());
            Some(units)
        }
        Err(error) => {
            warnings.push(format!("Ship class content is unavailable: {error}"));
            None
        }
    }
}

/// The reply for exactly `params.tags`: the caller chose the aliases a selection answers.
#[allow(clippy::too_many_arguments)]
fn present(
    source: &Source,
    params: &NavyParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
    scanned: &ScannedNavy,
    history: Option<&country_history::RecordedHistories>,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<NavyReply> {
    let build = |selection: &NavyParams,
                 units: Option<&installed_content_core::unit_leader::SubUnits>,
                 localise: &dyn Fn(&str) -> Option<String>,
                 warnings,
                 cache: Option<&PartResults>| match membership {
        Some(membership) => selected(
            scanned,
            selection,
            units,
            localise,
            warnings,
            cancelled,
            Some(membership),
            cache,
        ),
        None => reply_cached(
            scanned, selection, units, localise, warnings, cancelled, cache,
        ),
    };
    let mut warnings = Vec::new();
    let catalogue = context
        .content()
        .and_then(|options| catalogue::catalogue(source, options));
    let mut result = match catalogue {
        Ok(catalogue) => {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            let units = content_units(&catalogue, &mut warnings);
            let mut result = build(
                params,
                units.as_deref(),
                &|key| {
                    catalogue.localisation.get(key)?;
                    catalogue.localised(key, catalogue::View::Plain)
                },
                warnings,
                Some(&part_content(source, Some(&catalogue))),
            )?;
            result.warnings.extend(catalogue.refusal_lines());
            Ok(result)
        }
        Err(error) => {
            warnings.push(format!("Ship class names are unavailable: {error}"));
            build(
                params,
                None,
                &|_| None,
                warnings,
                Some(&part_content(source, None)),
            )
        }
    }?;
    for series in &mut result.series {
        if let Some(country) = history.and_then(|history| history.countries.get(&series.tag)) {
            series.identities = country.identities.as_ref().clone();
        }
    }
    checked_reply(result)
}
