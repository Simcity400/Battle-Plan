//! Party history's replies, read from the family's series publication ([`super::emit`]): the
//! chart's popularity summaries with its marks, the state at hours, keyboard navigation, mark
//! pages and the whole history, in the shapes the prepared widget reads.

use super::{
    CountryParams, FixedRun, HourSpan, LeaderRun, POPULARITY_DECIMALS, PartyNameRun, PartySeries,
    RulingRun, ScannedParties, emit::{self, Meta}, mark_unobserved, reply,
};
use dashboard_base::{
    Source,
    prepared::{
        coverage::Coverage,
        engine,
        series::{self, MeasureKey, Navigation},
    },
    prepared_chart::{ChartSummary, MarkCluster, MarkSummary},
    products::LoadContext,
};
use dashboard_series::{
    Event, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span, Viewport,
    codec::Binary,
    family::{Directory, Scalar},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

/// Rows one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 24;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Params {
    pub branch_id: String,
    tag: String,
    mode: String,
    #[serde(default = "values_default")]
    values: bool,
    #[serde(default)]
    fields: Option<Vec<String>>,
    within: Option<Vec<HourSpan>>,
    #[serde(default)]
    hours: Vec<i32>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    field: Option<String>,
    cursor: Option<String>,
    page_size: Option<u32>,
    navigation: Option<Navigation>,
}

impl Params {
    /// Whether it asks for the whole history, which waits for the branch's whole coverage.
    pub fn history(&self) -> bool {
        self.mode == "history"
    }
}

fn values_default() -> bool {
    true
}

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

fn within(spans: &Option<Vec<HourSpan>>, hour: i32) -> bool {
    spans.as_ref().is_none_or(|spans| {
        spans
            .iter()
            .any(|span| span.first_hour <= hour && span.last_hour >= hour)
    })
}

/// `spans` in order with overlaps joined, as an engine membership reads them.
fn membership(spans: &[HourSpan]) -> Vec<HourSpan> {
    let mut sorted = spans.to_vec();
    sorted.sort_by_key(|span| (span.first_hour, span.last_hour));
    let mut merged: Vec<HourSpan> = Vec::new();
    for span in sorted {
        match merged.last_mut() {
            Some(previous) if span.first_hour <= previous.last_hour => {
                previous.last_hour = previous.last_hour.max(span.last_hour);
            }
            _ => merged.push(span),
        }
    }
    merged
}

/// The walk's range, or each membership span clipped to it.
fn clipped(within: Option<&[HourSpan]>, navigation: &Navigation) -> Vec<(f64, f64)> {
    let (first, last) = (navigation.first_hour, navigation.last_hour);
    let mut spans: Vec<_> = match within {
        Some(spans) => spans
            .iter()
            .map(|span| {
                (
                    f64::from(span.first_hour).max(first),
                    f64::from(span.last_hour).min(last),
                )
            })
            .collect(),
        None => vec![(first, last)],
    };
    spans.retain(|(first, last)| first <= last);
    spans
}

fn forward(navigation: &Navigation) -> bool {
    matches!(navigation.key, MeasureKey::Home | MeasureKey::ArrowRight)
}

fn nearer(result: &mut Option<f64>, hour: f64, forward: bool) {
    *result = Some(result.map_or(hour, |previous| {
        if forward {
            previous.min(hour)
        } else {
            previous.max(hour)
        }
    }));
}

/// `key`'s nearest stop beyond the origin within each clipped span, a span's edge where a value
/// is carried into it; no fallback hour, so a key without a stop never outbids another's.
fn stop<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    within: Option<&[HourSpan]>,
    navigation: &Navigation,
) -> io::Result<Option<f64>> {
    let origin = navigation.origin()?;
    let forward = forward(navigation);
    let wide = Span {
        first: i32::MIN,
        last: i32::MAX,
    };
    let selection = Selection {
        key,
        membership: None,
    };
    let mut result = None;
    for (first, last) in clipped(within, navigation) {
        let at = if forward {
            origin.max(first - 1.0)
        } else {
            origin.min(last + 1.0)
        };
        let candidate = snapshot
            .next_change(selection, at, forward, wide)?
            .map(f64::from)
            .filter(|hour| *hour >= first && *hour <= last);
        let carried = snapshot.next_change(selection, first, false, wide)?.is_some();
        let edge = (carried && (forward && origin < first || !forward && origin > first))
            .then_some(first);
        for hour in [candidate, edge].into_iter().flatten() {
            nearer(&mut result, hour, forward);
        }
    }
    Ok(result)
}

/// The events of `key`, within `members`, dated `first..=last`.
fn count<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    members: Option<&[Span]>,
    (first, last): (i64, i64),
) -> io::Result<u64> {
    let first = first.max(i64::from(i32::MIN));
    let last = last.min(i64::from(i32::MAX));
    if first > last {
        return Ok(0);
    }
    let selection = [Selection {
        key,
        membership: members,
    }];
    snapshot.event_count(EventQuery {
        selection: &selection,
        native: Span {
            first: first as i32,
            last: last as i32,
        },
        as_of: None,
        order: EventOrder::Native,
    })
}

/// The first `limit` event hours of `key`, within `members`, dated `from..=to`, in order.
fn events<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    members: Option<&[Span]>,
    (from, to): (i32, i32),
    limit: usize,
) -> io::Result<Vec<i32>> {
    let mut hours = Vec::new();
    if from > to || limit == 0 {
        return Ok(hours);
    }
    let selection = [Selection {
        key,
        membership: members,
    }];
    let budget = limit
        .saturating_add(1)
        .saturating_mul(Event::SIZE + 1)
        .min(BUDGET);
    let mut cursor = None;
    loop {
        let page = snapshot.event_page(
            EventQuery {
                selection: &selection,
                native: Span {
                    first: from,
                    last: to,
                },
                as_of: None,
                order: EventOrder::Native,
            },
            cursor,
            budget,
            |event| {
                hours.push(event.native_hour());
                Ok(())
            },
        )?;
        cursor = page.next;
        if cursor.is_none() || hours.len() >= limit {
            hours.truncate(limit);
            return Ok(hours);
        }
    }
}

/// The nearest election beyond the origin, like [`stop`]: an election is one hour, not a state,
/// so a span's edge is no stop.
fn election_stop<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    within: Option<&[HourSpan]>,
    navigation: &Navigation,
) -> io::Result<Option<f64>> {
    let origin = navigation.origin()?;
    let forward = forward(navigation);
    let start = i64::from(i32::MIN);
    let mut result = None;
    for (first, last) in clipped(within, navigation) {
        // The span's whole hours strictly beyond the origin.
        let (low, high) = if forward {
            (
                (origin.floor() as i64)
                    .saturating_add(1)
                    .max(first.ceil() as i64),
                last.floor() as i64,
            )
        } else {
            (
                first.ceil() as i64,
                (origin.ceil() as i64)
                    .saturating_sub(1)
                    .min(last.floor() as i64),
            )
        };
        if low > high {
            continue;
        }
        let before = count(snapshot, key, None, (start, low.saturating_sub(1)))?;
        let until = count(snapshot, key, None, (start, high))?;
        if before < until {
            let ordinal = if forward { before } else { until - 1 };
            let selection = Selection {
                key,
                membership: None,
            };
            let hour = snapshot
                .event_date(selection, None, ordinal)?
                .ok_or_else(|| invalid("Missing prepared election"))?;
            nearer(&mut result, f64::from(hour), forward);
        }
    }
    Ok(result)
}

/// The marks of `key`'s events over the viewport, within `spans`.
fn clusters<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    (from, to, width): (f64, f64, u32),
    spans: Option<&[HourSpan]>,
) -> io::Result<MarkSummary> {
    let Some(spans) = spans else {
        return engine::clusters(snapshot, key, (from, to, width));
    };
    let members = engine::spans(&membership(spans))?;
    let selection = [Selection {
        key,
        membership: Some(&members),
    }];
    let mut clusters = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.event_clusters(
            EventQuery {
                selection: &selection,
                native: Span {
                    first: i32::MIN,
                    last: i32::MAX,
                },
                as_of: None,
                order: EventOrder::Native,
            },
            Viewport { from, to, width },
            cursor,
            BUDGET,
            |cluster| {
                clusters.push(MarkCluster {
                    first_hour: cluster
                        .first_hour
                        .ok_or_else(|| invalid("A mark cluster without its first hour"))?,
                    last_hour: cluster
                        .last_hour
                        .ok_or_else(|| invalid("A mark cluster without its last hour"))?,
                    first_ordinal: cluster.first_ordinal.to_string(),
                    count: cluster.count.to_string(),
                });
                Ok(())
            },
        )?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(MarkSummary {
        from,
        to,
        width,
        clusters,
    })
}

/// The marks of `keys` (each a field of [`MARKS`] and its key), within `members`, dated
/// `low..=last`.
fn marks_through<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: &[(usize, u64)],
    members: Option<&[Span]>,
    (low, last): (i64, i64),
) -> io::Result<u64> {
    let mut total = 0_u64;
    for &(_, key) in keys {
        total = total
            .checked_add(count(snapshot, key, members, (low, last))?)
            .ok_or_else(|| invalid("Party mark count overflow"))?;
    }
    Ok(total)
}

/// One page of the marks of `keys` (each a field of [`MARKS`] and its key) in `from..=to`, by
/// hour then field, as the cursor counts them: past every earlier mark, or within `spans` among
/// their marks alone; with their total and the next page's cursor.
fn mark_page<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: &[(usize, u64)],
    (from, to): (i32, i32),
    spans: Option<&[HourSpan]>,
    cursor: u64,
    size: u32,
) -> io::Result<(Vec<(usize, i32)>, u64, Option<u64>)> {
    let members = spans
        .map(|spans| engine::spans(&membership(spans)))
        .transpose()?;
    let members = members.as_deref();
    let low = match members {
        Some(_) => i64::from(from),
        None => i64::from(i32::MIN),
    };
    let base = match members {
        Some(_) => 0,
        None => marks_through(snapshot, keys, None, (low, i64::from(from) - 1))?,
    };
    let last = marks_through(snapshot, keys, members, (low, i64::from(to)))?;
    let first = match members {
        Some(_) => cursor.min(last),
        None => cursor.max(base),
    };
    let end = first.saturating_add(u64::from(size)).min(last);
    let total = last.saturating_sub(base);
    let next = (end < last).then_some(end);
    if first >= end {
        return Ok((Vec::new(), total, next));
    }
    // The hour of the page's first mark: the first through which the marks pass `first`.
    let (mut bottom, mut top) = (i64::from(from), i64::from(to));
    while bottom < top {
        let middle = bottom + (top - bottom) / 2;
        if marks_through(snapshot, keys, members, (low, middle))? > first {
            top = middle;
        } else {
            bottom = middle + 1;
        }
    }
    let skip = first - marks_through(snapshot, keys, members, (low, bottom - 1))?;
    let wanted = usize::try_from(skip + (end - first))
        .map_err(|_| invalid("Party mark page is too large"))?;
    let mut marks = Vec::new();
    for &(field, key) in keys {
        let hours = events(snapshot, key, members, (bottom as i32, to), wanted)?;
        marks.extend(
            hours
                .into_iter()
                .enumerate()
                .map(|(at, hour)| (hour, field, at)),
        );
    }
    marks.sort_unstable();
    let page = marks
        .into_iter()
        .skip(skip as usize)
        .take((end - first) as usize)
        .map(|(hour, field, _)| (field, hour))
        .collect();
    Ok((page, total, next))
}

/// The fields with marks, and each one's mark measure.
const MARKS: [(&str, u64); 3] = [
    ("ruling", emit::RULING_MARKS),
    ("leader", emit::LEADER_MARKS),
    ("election", emit::ELECTIONS),
];

/// What a tag's publication names.
struct Tag<'d> {
    directory: &'d Directory,
    actor: Option<u64>,
    strings: Vec<String>,
}

impl Tag<'_> {
    fn key(&self, measure: u64) -> Option<u64> {
        self.actor
            .and_then(|actor| self.directory.key(actor, measure, None))
    }

    fn string(&self, id: u32) -> io::Result<String> {
        self.strings
            .get(id as usize)
            .cloned()
            .ok_or_else(|| invalid("A prepared party run names an unknown string"))
    }

    /// The run of `measure` holding `hour`, whole.
    fn run<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        measure: u64,
        hour: i32,
    ) -> io::Result<Option<dashboard_series::Run>> {
        Ok(match self.key(measure) {
            Some(key) => engine::run_at(snapshot, key, hour)?.map(|(run, _)| run),
            None => None,
        })
    }

    fn ruling(&self, run: &dashboard_series::Run) -> io::Result<RulingRun> {
        Ok(RulingRun {
            token: self.string(emit::ruling(run.value))?,
            first_hour: run.span.first,
            last_hour: run.span.last,
            starts_segment: run.break_before,
        })
    }

    fn leader(&self, run: &dashboard_series::Run) -> io::Result<LeaderRun> {
        let (ideology, person) = emit::leader(run.value);
        Ok(LeaderRun {
            ideology: self.string(ideology)?,
            person,
            first_hour: run.span.first,
            last_hour: run.span.last,
            starts_segment: run.break_before,
        })
    }

    fn name(run: &dashboard_series::Run) -> PartyNameRun {
        PartyNameRun {
            name: emit::name(run.value),
            first_hour: run.span.first,
            last_hour: run.span.last,
        }
    }

    /// The run of `measure` holding `hour`, into `scanned` as a ruling, name or leader run.
    fn add_at<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        scanned: &mut ScannedParties,
        measure: u64,
        hour: i32,
    ) -> io::Result<()> {
        if let Some(run) = self.run(snapshot, measure, hour)? {
            self.add(scanned, measure, &run)?;
        }
        Ok(())
    }

    fn add(
        &self,
        scanned: &mut ScannedParties,
        measure: u64,
        run: &dashboard_series::Run,
    ) -> io::Result<()> {
        match measure {
            emit::RULING => scanned.ruling.push(self.ruling(run)?),
            emit::NAME => scanned.party_name_runs.push(Self::name(run)),
            _ => scanned.leaders.push(self.leader(run)?),
        }
        Ok(())
    }

    /// A mark of `field` at `hour`: the change's new run as of its first hour, after the run
    /// before it.
    fn mark<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        scanned: &mut ScannedParties,
        field: usize,
        hour: i32,
    ) -> io::Result<()> {
        if field == 2 {
            scanned.elections.push(hour);
            return Ok(());
        }
        let measure = if field == 0 { emit::RULING } else { emit::LEADER };
        let missing = || invalid("A prepared party mark has no run");
        if let Some(before) = hour
            .checked_sub(1)
            .map(|before| self.run(snapshot, measure, before))
            .transpose()?
            .flatten()
        {
            self.add(scanned, measure, &before)?;
        }
        let now = self.run(snapshot, measure, hour)?.ok_or_else(missing)?;
        let row = dashboard_series::Run {
            span: Span::new(hour, hour)?,
            break_before: false,
            ..now
        };
        self.add(scanned, measure, &row)
    }
}

/// The hours `tag` was not observed in `from..=to`: hours its parties were unavailable, and hours
/// between its holders' runs.
fn unavailable<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    tag: &Tag<'_>,
    from: i32,
    to: i32,
) -> io::Result<Vec<HourSpan>> {
    let mut spans = Vec::new();
    if let Some(key) = tag.directory.key(emit::FAMILY, emit::UNAVAILABLE, None) {
        for run in engine::runs(snapshot, key, from, to)?
            .into_iter()
            .filter(|run| run.value == 1)
        {
            spans.push(HourSpan {
                first_hour: run.span.first,
                last_hour: run.span.last,
            });
        }
    }
    if let Some(key) = tag.key(emit::OWNER) {
        let accepted = snapshot.publication().accepted;
        let selection = Selection {
            key,
            membership: None,
        };
        let mut owners: Vec<(i32, i32)> = engine::runs(snapshot, key, from, to)?
            .into_iter()
            .map(|run| (run.span.first, run.span.last))
            .collect();
        if let Some(hour) = snapshot.next_change(selection, from, false, accepted)? {
            let run = match engine::run_at(snapshot, key, hour)? {
                Some(run) => Some(run),
                None => match hour.checked_sub(1) {
                    Some(before) => engine::run_at(snapshot, key, before)?,
                    None => None,
                },
            };
            if let Some((run, _)) = run {
                owners.insert(0, (run.span.first, run.span.last));
            }
        }
        if let Some(hour) = snapshot.next_change(selection, to, true, accepted)?
            && let Some((run, _)) = engine::run_at(snapshot, key, hour)?
        {
            owners.push((run.span.first, run.span.last));
        }
        for pair in owners.windows(2) {
            if pair[0].1.saturating_add(1) < pair[1].0 {
                spans.push(HourSpan {
                    first_hour: pair[0].1.saturating_add(1).max(from),
                    last_hour: pair[1].0.saturating_sub(1).min(to),
                });
            }
        }
    }
    spans.sort_by_key(|span| span.first_hour);
    let mut result = Vec::new();
    for span in spans
        .into_iter()
        .filter(|span| span.first_hour <= span.last_hour)
    {
        mark_unobserved(&mut result, span.first_hour, span.last_hour);
    }
    Ok(result)
}

fn fixed_run(run: &dashboard_series::Run) -> FixedRun {
    FixedRun {
        first_hour: run.span.first,
        last_hour: run.span.last,
        value_raw: run.value.to_string(),
        starts_segment: run.break_before,
    }
}

/// A `party-history` request answered from `snapshot`, the publication pinned through
/// `accepted`; nothing past it is read.
#[allow(clippy::too_many_arguments)]
pub fn query<P: PageRead>(
    source: &Source,
    params: Params,
    context: &LoadContext,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    accepted: Option<Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    CountryParams {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
    }
    .validate()
    .map_err(dashboard_core::invalid)?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| invalid("Invalid party branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(
        source.replay_ref(),
        source
            .replay_ref()
            .branches
            .iter()
            .find(|start| start.id == branch)
            .ok_or_else(|| invalid("Unknown party branch"))?,
    );
    let published = snapshot.publication().accepted;
    let tag = Tag {
        directory,
        actor: directory.actor(params.tag.as_bytes()),
        strings: directory
            .scalar(emit::STRINGS)
            .map_or(&[][..], Scalar::list)
            .iter()
            .map(|text| {
                text.text()
                    .and_then(|text| std::str::from_utf8(text).ok())
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("A prepared party string is not valid"))
            })
            .collect::<io::Result<_>>()?,
    };
    let meta: Meta = match tag
        .actor
        .and_then(|actor| directory.scalar(emit::META + actor))
    {
        Some(scalar) => serde_json::from_slice(
            scalar
                .text()
                .ok_or_else(|| invalid("A prepared party catalogue is not valid"))?,
        )?,
        None => Meta::default(),
    };
    // Each party ever read, by token, with its popularity key.
    let mut tokens = BTreeMap::new();
    if let Some(actor) = tag.actor {
        for &(id, key) in directory.dimensions(actor, emit::POPULARITY) {
            let id = u32::try_from(id).map_err(|_| invalid("A party key is not valid"))?;
            tokens.insert(tag.string(id)?, key);
        }
    }
    let Meta {
        namings,
        selected,
        people,
        types,
        invalid: unreadable,
        duplicate,
    } = meta;
    let mut scanned = ScannedParties {
        parties: tokens
            .keys()
            .map(|token| {
                (
                    token.clone(),
                    PartySeries {
                        token: token.clone(),
                        runs: Vec::new(),
                    },
                )
            })
            .collect(),
        namings,
        selected_tags: selected,
        people,
        ..ScannedParties::default()
    };
    if !unreadable.is_empty() {
        scanned.warnings.push(format!(
            "PoliticalParties ideology tokens are empty or not UTF-8; their popularity and names cannot be identified (count: {}).",
            unreadable.len()
        ));
    }
    let mark_keys: [Option<u64>; 3] = MARKS.map(|(_, measure)| tag.key(measure));
    let mut segments = Vec::new();
    let mut summaries = Vec::new();
    let mut marks = Vec::new();
    let mut cursor = None;
    let mut total = String::from("0");
    match params.mode.as_str() {
        "next" => {
            let navigation = params
                .navigation
                .as_ref()
                .ok_or_else(|| invalid("Missing party navigation"))?;
            navigation.origin()?;
            // Stops are only what the chart shows: popularity and its holder, and asked-for
            // marks. A party's name draws no mark, so its changes are no stop.
            let marked = |name: &str| {
                params
                    .fields
                    .as_ref()
                    .is_none_or(|fields| fields.iter().any(|field| field == name))
            };
            let within = params.within.as_deref();
            let mut keys: Vec<u64> = Vec::new();
            if params.values {
                keys.extend(tag.key(emit::OWNER));
                keys.extend(tokens.values().copied());
            }
            if marked("ruling") {
                keys.extend(tag.key(emit::RULING));
            }
            if marked("leader") {
                keys.extend(tag.key(emit::LEADER));
            }
            let forward = forward(navigation);
            let mut result = None;
            for key in keys {
                if let Some(hour) = stop(snapshot, key, within, navigation)? {
                    nearer(&mut result, hour, forward);
                }
            }
            if marked("election")
                && let Some(key) = mark_keys[2]
                && let Some(hour) = election_stop(snapshot, key, within, navigation)?
            {
                nearer(&mut result, hour, forward);
            }
            let hour = result.or_else(|| {
                navigation
                    .from
                    .map(|hour| hour.clamp(navigation.first_hour, navigation.last_hour))
            });
            return Ok(json!({ "hour": hour }));
        }
        "summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, scanned.parties.len())?;
            let spans = params.within.as_deref().map(membership);
            if params.values {
                for (token, party) in &mut scanned.parties {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let key = *tokens
                        .get(token)
                        .ok_or_else(|| invalid("Missing prepared party key"))?;
                    let summary: ChartSummary = engine::summary(
                        snapshot,
                        key,
                        bounds,
                        (from, to, width),
                        POPULARITY_DECIMALS,
                        spans.as_deref(),
                    )?;
                    for run in summary.runs.iter().flatten() {
                        let raw = engine::run_at(snapshot, key, run.first_hour)?
                            .map(|(found, _)| found.value)
                            .ok_or_else(|| invalid("Missing exact party summary value"))?;
                        party.runs.push(FixedRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            value_raw: raw.to_string(),
                            starts_segment: run.break_before,
                        });
                    }
                    summaries.push(json!({"token": token, "summary": summary}));
                }
            }
            let count = i64::from((to.floor() as i32).min(published.last))
                - i64::from((from.floor() as i32).max(published.first))
                + 1;
            let exact = count / i64::from(width) < 2;
            for (field, &(name, _)) in MARKS.iter().enumerate() {
                if params
                    .fields
                    .as_ref()
                    .is_some_and(|fields| !fields.iter().any(|wanted| wanted == name))
                {
                    continue;
                }
                let Some(key) = mark_keys[field] else {
                    continue;
                };
                let summary = clusters(snapshot, key, (from, to, width), spans.as_deref())?;
                let mut rows = Vec::new();
                if exact {
                    let (first, last) = (from.ceil() as i32, to.floor() as i32);
                    if first <= last {
                        rows = mark_page(
                            snapshot,
                            &[(field, key)],
                            (first, last),
                            params.within.as_deref(),
                            0,
                            width.saturating_mul(2).saturating_add(2),
                        )?
                        .0
                        .into_iter()
                        .map(|(_, hour)| hour)
                        .collect();
                    }
                } else {
                    let selection = Selection {
                        key,
                        membership: None,
                    };
                    for cluster in &summary.clusters {
                        if cluster.count == "1" {
                            let ordinal: u64 = cluster
                                .first_ordinal
                                .parse()
                                .map_err(|_| invalid("Invalid party cluster ordinal"))?;
                            rows.push(
                                snapshot
                                    .event_date(selection, None, ordinal)?
                                    .ok_or_else(|| invalid("Missing party cluster event"))?,
                            );
                        }
                    }
                }
                for hour in rows.into_iter().filter(|&hour| within(&params.within, hour)) {
                    tag.mark(snapshot, &mut scanned, field, hour)?;
                }
                marks.push(json!({"field": name, "summary": summary, "exact": exact}));
            }
        }
        "state" => {
            for &hour in &params.hours {
                dashboard_text::metadata::check_cancel(cancelled)?;
                if let Some(run) = tag.run(snapshot, emit::OWNER, hour)? {
                    segments.push(HourSpan {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                    });
                }
                for (token, party) in &mut scanned.parties {
                    let key = *tokens
                        .get(token)
                        .ok_or_else(|| invalid("Missing prepared party key"))?;
                    if let Some((run, _)) = engine::run_at(snapshot, key, hour)? {
                        party.runs.push(fixed_run(&run));
                    }
                }
                for measure in [emit::RULING, emit::NAME, emit::LEADER] {
                    tag.add_at(snapshot, &mut scanned, measure, hour)?;
                }
                scanned
                    .unobserved
                    .extend(unavailable(snapshot, &tag, hour, hour)?);
            }
        }
        "events" => {
            let name = params
                .field
                .as_deref()
                .filter(|name| ["ruling", "leader", "election", "all"].contains(name))
                .ok_or_else(|| invalid("Invalid party event field"))?;
            let (from, to, offset, size) = series::event_window(
                params.from,
                params.to,
                params.cursor.as_deref(),
                Some(params.page_size.unwrap_or(128)),
            )?;
            let mut keys = Vec::new();
            for (field, &(mark, _)) in MARKS.iter().enumerate() {
                let chosen = name == mark
                    || name == "all"
                        && params
                            .fields
                            .as_ref()
                            .is_none_or(|fields| fields.iter().any(|wanted| wanted == mark));
                if chosen && let Some(key) = mark_keys[field] {
                    keys.push((field, key));
                }
            }
            let (page, count, next) = mark_page(
                snapshot,
                &keys,
                (from, to),
                params.within.as_deref(),
                offset,
                size,
            )?;
            total = count.to_string();
            cursor = next.map(|ordinal| ordinal.to_string());
            for (field, hour) in page {
                dashboard_text::metadata::check_cancel(cancelled)?;
                tag.mark(snapshot, &mut scanned, field, hour)?;
                scanned.unobserved.extend(unavailable(
                    snapshot,
                    &tag,
                    hour.saturating_sub(1),
                    hour,
                )?);
            }
        }
        "history" => {
            let (from, to) = (published.first, published.last);
            for (token, party) in &mut scanned.parties {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let key = *tokens
                    .get(token)
                    .ok_or_else(|| invalid("Missing prepared party key"))?;
                party.runs = engine::runs(snapshot, key, from, to)?
                    .iter()
                    .map(fixed_run)
                    .collect();
            }
            for measure in [emit::RULING, emit::NAME, emit::LEADER] {
                if let Some(key) = tag.key(measure) {
                    for run in engine::runs(snapshot, key, from, to)? {
                        tag.add(&mut scanned, measure, &run)?;
                    }
                }
            }
            if let Some(key) = mark_keys[2] {
                scanned.elections = events(snapshot, key, None, (from, to), usize::MAX)?;
            }
            scanned.unobserved = unavailable(snapshot, &tag, from, to)?;
        }
        _ => return Err(invalid("Unknown party query mode")),
    }
    // The widget's validator refuses a history whose rows repeated a token or held two ruling
    // parties, even where the window does not reach that hour; that hour's rows are not kept.
    if let Some((hour, token, value)) = duplicate.as_ref() {
        let runs = &mut scanned
            .parties
            .entry(token.clone())
            .or_insert_with(|| PartySeries {
                token: token.clone(),
                runs: Vec::new(),
            })
            .runs;
        for _ in 0..2 {
            runs.push(FixedRun {
                first_hour: *hour,
                last_hour: *hour,
                value_raw: value.to_string(),
                starts_segment: true,
            });
        }
    }
    for party in scanned.parties.values_mut() {
        party.runs.sort_by_key(|run| run.first_hour);
        if duplicate.is_none() {
            party.runs.dedup();
        }
    }
    scanned.ruling.sort_by_key(|run| run.first_hour);
    scanned.ruling.dedup_by(|next, previous| {
        if next.first_hour == previous.first_hour
            && next.token == previous.token
            && next.starts_segment == previous.starts_segment
        {
            previous.last_hour = previous.last_hour.max(next.last_hour);
            true
        } else {
            false
        }
    });
    scanned.leaders.sort_by_key(|run| run.first_hour);
    scanned.leaders.dedup_by(|next, previous| {
        if next.first_hour == previous.first_hour
            && next.ideology == previous.ideology
            && next.person == previous.person
            && next.starts_segment == previous.starts_segment
        {
            previous.last_hour = previous.last_hour.max(next.last_hour);
            true
        } else {
            false
        }
    });
    scanned.party_name_runs.sort_by_key(|run| run.first_hour);
    scanned.party_name_runs.dedup();
    scanned.unobserved.sort_by_key(|span| span.first_hour);
    for span in std::mem::take(&mut scanned.unobserved) {
        mark_unobserved(&mut scanned.unobserved, span.first_hour, span.last_hour);
    }
    let historical: Option<&BTreeSet<String>> = tag.actor.map(|_| &types);
    let reply = reply(
        source,
        &CountryParams {
            branch_id: params.branch_id,
            tag: params.tag,
        },
        context,
        &scanned,
        historical,
        cancelled,
    )?;
    Ok(json!({
        "coverage": dashboard_base::prepared::prefix::extent(source, branch, accepted)?,
        "reply": reply,
        "segments": segments,
        "summaries": summaries,
        "marks": marks,
        "cursor": cursor,
        "total": total,
    }))
}
