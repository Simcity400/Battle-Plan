//! Trade flows' replies, read from the family's series publication ([`super::emit`]) through the
//! coverage it shares with the identity history: the chosen flows' runs over a window with their
//! daily amounts, totals or averages charted, each flow's integrals and active hours at the hours
//! asked, the reads and the deals they left out, and keyboard navigation. A flow's integrals are
//! its run start's integrals plus its terms for each hour since. Nothing reads the recording at
//! query time.

use super::{
    Flow, FlowKey, PartialSpan, Run, ScannedFlows, Terms, TradeCountryIdentity, TradeFlow,
    TradeFlowParams, TradeFlows, TradeFlowsReply,
    emit::{
        self, ALIASES, DELIVERED, FACT, FAMILY, FIRSTS, GROUPS, KNOWN, READ, REFUSALS, RESOURCES,
        SCHEMA, TOTALS, kind,
    },
    recording_warnings,
};
use dashboard_base::{
    Source,
    prepared::{
        engine, published,
        series::{self, Navigation},
    },
    prepared_chart::{ChartBucket, ChartRun, ChartSummary},
    products::{
        Product,
        country_groups::HourSpan,
        country_history::{
            Identity, RecordedHistories, read_prepared_histories, selected_identities,
        },
        resources::{RESOURCE_DECIMALS, ResourceNaming, present_stored_json},
    },
};
use dashboard_core::config::{ContentOptions, LoadContext};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

fn exceeded() -> io::Error {
    invalid("Trade integral exceeds its integer range")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    branch_id: String,
    tags: Option<Vec<String>>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    hours: Vec<i32>,
    /// Counting basis is separate from the chart viewport.
    basis: Option<i32>,
    op: Option<String>,
    identities: Option<Vec<String>>,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
    #[serde(default)]
    unobserved: Vec<HourSpan>,
    #[serde(default)]
    between: bool,
    /// The flows a chart draws, by key: navigation stops only at their changes.
    flows: Option<BTreeSet<String>>,
    measure: Option<String>,
    #[serde(default)]
    require_complete: bool,
    cursor: Option<String>,
}

/// The `trade-flows` reply to `params`, read from the trade family's publication pinned through
/// the coverage it shares with the identity history; null while it is pending.
pub fn query(
    source: &Source,
    params: Value,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let input: Params = serde_json::from_value(params.clone())?;
    let selection = TradeFlowParams {
        branch_id: input.branch_id.clone(),
        tags: input.tags.clone(),
    };
    TradeFlows::validate(&selection).map_err(dashboard_core::invalid)?;
    let branch = selection.branch();
    let coverage = match dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "trade-flows",
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        result => result?,
    };
    let complete = coverage.as_ref().is_some_and(|coverage| coverage.complete);
    if input.require_complete && !complete {
        return Ok(Value::Null);
    }
    // Integrals, running averages and navigation depend on earlier hours.
    if !complete
        && (input.op.is_some()
            || input
                .measure
                .as_deref()
                .is_some_and(|measure| measure != "current"))
    {
        return Ok(Value::Null);
    }
    let Some(coverage) = coverage else {
        return Ok(Value::Null);
    };
    // Pinned through the coverage the flows share with the identities: no read passes it.
    let Some(opened) = published::open_through(
        source,
        content,
        "trade",
        SCHEMA,
        branch,
        (!complete).then_some(coverage.last_hour),
    )?
    else {
        return Ok(Value::Null);
    };
    let (store, key) = dashboard_base::prepared::files(source, content)?;
    // The identities prepared through the accepted coverage, never read from the recording.
    let history = read_prepared_histories(&store, &key, branch, Some(coverage.last_hour))?
        .unwrap_or_default();
    let naming = ResourceNaming::stored(
        source,
        &LoadContext {
            content: content.cloned(),
        },
    );
    let last_hour = coverage.last_hour;
    let extent = dashboard_base::prepared::prefix::extent(source, branch, Some(coverage))?;
    let read = Read {
        branch,
        history: &history,
        last_hour,
        complete,
        naming: &naming,
        cancelled,
    };
    let value = opened.read(cancelled, |snapshot, directory| {
        read.pages(snapshot, directory, input, params)
    })?;
    let Some(mut value) = value else {
        return Ok(Value::Null);
    };
    value["coverage"] = json!(extent);
    present_stored_json(source, content, &mut value["resources"], cancelled)?;
    Ok(value)
}

/// A family scalar of `[Unsigned key, value]` pairs.
fn pairs(directory: &Directory, id: u64) -> impl Iterator<Item = (u64, &Scalar)> {
    directory
        .scalar(id)
        .map_or(&[][..], Scalar::list)
        .iter()
        .filter_map(|pair| match pair.list() {
            [key, value] => Some((u64::try_from(key.integer()?).ok()?, value)),
            _ => None,
        })
}

fn text(scalar: &Scalar) -> io::Result<String> {
    scalar
        .text()
        .and_then(|text| String::from_utf8(text.to_vec()).ok())
        .ok_or_else(|| invalid("A prepared trade text is not valid"))
}

fn unsigned(scalar: &Scalar) -> io::Result<u32> {
    scalar
        .integer()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid("A prepared trade number is not valid"))
}

/// A flow actor's flow: its seller and buyer holders and its resource, from its flow token.
fn flow_of(name: &[u8]) -> io::Result<FlowKey> {
    let (seller, buyer, resource): (String, String, u32) = serde_json::from_slice(name)?;
    Ok((
        serde_json::from_str(&seller)?,
        serde_json::from_str(&buyer)?,
        resource,
    ))
}

fn selection(key: u64) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<dashboard_series::Run>> {
    Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| run))
}

/// The hour of `key`'s first event in `first..=last`.
fn first_event<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    first: i32,
    last: i32,
) -> io::Result<Option<i32>> {
    let selected = [selection(key)];
    let mut found = None;
    snapshot.event_page(
        EventQuery {
            selection: &selected,
            native: dashboard_series::Span::new(first, last)?,
            as_of: None,
            order: EventOrder::Native,
        },
        None,
        dashboard_series::Event::SIZE + 1,
        |event| {
            found.get_or_insert(event.native_hour());
            Ok(())
        },
    )?;
    Ok(found)
}

/// A chart of `key` over `view`, its bins laid from `bounds`, within `spans`; every bin unknown
/// where the family never named the key.
fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    decimals: u8,
    spans: Option<&[HourSpan]>,
) -> io::Result<ChartSummary> {
    if let Some(key) = key {
        return engine::summary(snapshot, key, bounds, view, decimals, spans);
    }
    let (exact, bins) = series::summary_bins(bounds, view)?;
    Ok(ChartSummary {
        from: view.0,
        to: view.1,
        width: view.2,
        buckets: vec![None; bins.len()],
        runs: exact.then(Vec::new),
        approximate: Vec::new(),
    })
}

fn combine(first: &[HourSpan], second: &[HourSpan], both: bool) -> Vec<HourSpan> {
    let mut edges = BTreeMap::<i64, i32>::new();
    for span in first.iter().chain(second) {
        *edges.entry(i64::from(span.first_hour)).or_default() += 1;
        *edges.entry(i64::from(span.last_hour) + 1).or_default() -= 1;
    }
    let need = if both { 2 } else { 1 };
    let mut depth = 0;
    let mut start = None;
    let mut result = Vec::new();
    for (hour, delta) in edges {
        depth += delta;
        if depth >= need && start.is_none() {
            start = Some(hour);
        } else if depth < need
            && let Some(first) = start.take()
            && first < hour
        {
            result.push(HourSpan {
                first_hour: first as i32,
                last_hour: (hour - 1) as i32,
            });
        }
    }
    result
}

fn subtract(spans: &[HourSpan], holes: &[HourSpan]) -> Vec<HourSpan> {
    let mut result = Vec::new();
    for span in spans {
        let mut start = i64::from(span.first_hour);
        for hole in holes {
            if i64::from(hole.last_hour) < start || hole.first_hour > span.last_hour {
                continue;
            }
            if i64::from(hole.first_hour) > start {
                result.push(HourSpan {
                    first_hour: start as i32,
                    last_hour: hole.first_hour - 1,
                });
            }
            start = start.max(i64::from(hole.last_hour) + 1);
        }
        if start <= i64::from(span.last_hour) {
            result.push(HourSpan {
                first_hour: start as i32,
                last_hour: span.last_hour,
            });
        }
    }
    result
}

/// `spans` merged into ascending, disjoint spans.
fn union(mut spans: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    spans.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match merged.last_mut() {
            Some(held) if i64::from(first) <= i64::from(held.1) + 1 => held.1 = held.1.max(last),
            _ => merged.push((first, last)),
        }
    }
    merged
}

/// Hours of ascending disjoint `spans` through `hour`, each span's count before it alongside.
struct Counted {
    spans: Vec<(i32, i32)>,
    before: Vec<i128>,
    /// The last hour the counts hold through; none past it.
    last: i32,
}

impl Counted {
    fn new(spans: Vec<(i32, i32)>, last: i32) -> Self {
        let mut before = Vec::with_capacity(spans.len());
        let mut total = 0_i128;
        for &(first, last) in &spans {
            before.push(total);
            total += i128::from(last) - i128::from(first) + 1;
        }
        Self {
            spans,
            before,
            last,
        }
    }

    fn through(&self, hour: i32) -> i128 {
        if hour > self.last {
            return 0;
        }
        let at = self.spans.partition_point(|&(first, _)| first <= hour);
        match at.checked_sub(1) {
            Some(at) => {
                let (first, last) = self.spans[at];
                self.before[at] + i128::from(last.min(hour)) - i128::from(first) + 1
            }
            None => 0,
        }
    }
}

/// A flow's run as its start event holds it: its last hour, its terms and aliases, and its
/// integrals through the hour before it.
#[derive(Clone, Copy)]
struct Held {
    last: i32,
    terms: Terms,
    aliases: [u32; 2],
    base: [i128; 6],
}

/// The fields of a [`kind::RUN`] event.
fn run_fields<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: EventView,
) -> io::Result<(Terms, [u32; 2], [i128; 6])> {
    let malformed = || invalid("A prepared trade run is not valid");
    let EventView::Family(event) = event else {
        return Err(malformed());
    };
    if event.kind != kind::RUN || event.fields.len != 13 * Cell::SIZE as u64 {
        return Err(malformed());
    }
    let mut cells = [Cell::Null; 13];
    for (at, cell) in cells.iter_mut().enumerate() {
        *cell = snapshot.field(event.fields, at as u64)?;
    }
    let unsigned = |cell: Cell| match cell {
        Cell::Unsigned(value) => Ok(value),
        _ => Err(malformed()),
    };
    let integer = |cell: Cell| match cell {
        Cell::Integer(value) => Ok(value),
        _ => Err(malformed()),
    };
    let alias = |cell| u32::try_from(unsigned(cell)?).map_err(|_| malformed());
    let terms = Terms {
        delivered: i64::try_from(integer(cells[2])?).map_err(|_| malformed())?,
        factories: i32::try_from(integer(cells[3])?).map_err(|_| malformed())?,
        assigned: i32::try_from(integer(cells[4])?).map_err(|_| malformed())?,
        required: i32::try_from(integer(cells[5])?).map_err(|_| malformed())?,
        deals: u32::try_from(unsigned(cells[6])?).map_err(|_| malformed())?,
    };
    let mut base = [0_i128; 6];
    for (total, cell) in base.iter_mut().zip(&cells[7..]) {
        *total = integer(*cell)?;
    }
    Ok((terms, [alias(cells[0])?, alias(cells[1])?], base))
}

/// One flow's runs as a reply reads them, each read once.
struct Runs {
    fact: u64,
    /// The flow's first hour and the publication's last: its integrals hold from one through
    /// the other.
    first: i32,
    last: i32,
    held: BTreeMap<i32, Held>,
}

impl Runs {
    /// The run starting at `first`.
    fn held<P: PageRead>(&mut self, snapshot: &mut Snapshot<'_, P>, first: i32) -> io::Result<Held> {
        if let Some(&held) = self.held.get(&first) {
            return Ok(held);
        }
        let run = run_at(snapshot, self.fact, first)?
            .filter(|run| run.span.first == first)
            .ok_or_else(|| invalid("Trade fact coverage is missing"))?;
        let event = snapshot
            .latest_event(selection(self.fact), first, None)?
            .filter(|event| event.native_hour() == first)
            .ok_or_else(|| invalid("Trade fact is missing"))?;
        let (terms, aliases, base) = run_fields(snapshot, event)?;
        let held = Held {
            last: run.span.last,
            terms,
            aliases,
            base,
        };
        self.held.insert(first, held);
        Ok(held)
    }

    /// The integrals through `hour`: none before the flow's first run or past the last hour
    /// published.
    fn raw<P: PageRead>(&mut self, snapshot: &mut Snapshot<'_, P>, hour: i32) -> io::Result<[i128; 6]> {
        if hour < self.first || hour > self.last {
            return Ok([0; 6]);
        }
        let start = match run_at(snapshot, self.fact, hour)? {
            Some(run) => run.span.first,
            None => match snapshot.latest_event(selection(self.fact), hour, None)? {
                Some(event) => event.native_hour(),
                None => return Ok([0; 6]),
            },
        };
        let held = self.held(snapshot, start)?;
        emit::integrals(held.base, held.terms, start, hour.min(held.last)).ok_or_else(exceeded)
    }

    /// The integrals through `hour`, within `within`'s hours, as of `last_hour` at most.
    fn prefix<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        hour: i32,
        within: Option<&[HourSpan]>,
        last_hour: i32,
    ) -> io::Result<[i128; 6]> {
        let hour = hour.min(last_hour);
        let Some(spans) = within else {
            return self.raw(snapshot, hour);
        };
        let mut sum = [0_i128; 6];
        for span in spans.iter().filter(|span| span.first_hour <= hour) {
            let last = self.raw(snapshot, span.last_hour.min(hour))?;
            let before = match span.first_hour.checked_sub(1) {
                Some(before) => self.raw(snapshot, before)?,
                None => [0; 6],
            };
            for ((total, last), before) in sum.iter_mut().zip(last).zip(before) {
                *total = last
                    .checked_sub(before)
                    .and_then(|delta| total.checked_add(delta))
                    .ok_or_else(exceeded)?;
            }
        }
        Ok(sum)
    }

    /// The first and last hours of `basis..=hour` within `within` the flow holds a run.
    fn bounds<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        basis: i32,
        hour: i32,
        within: Option<&[HourSpan]>,
    ) -> io::Result<[Option<i32>; 2]> {
        let whole = [HourSpan {
            first_hour: basis,
            last_hour: hour,
        }];
        let mut result: [Option<i32>; 2] = [None, None];
        for span in within.unwrap_or(&whole) {
            let start = span.first_hour.max(basis);
            let end = span.last_hour.min(hour);
            if start > end {
                continue;
            }
            let earliest = if run_at(snapshot, self.fact, start)?.is_some() {
                Some(start)
            } else {
                first_event(snapshot, self.fact, start, end)?
            };
            if let Some(at) = earliest {
                result[0] = Some(result[0].map_or(at, |before| before.min(at)));
            }
            if let Some(event) = snapshot.latest_event(selection(self.fact), end, None)?
                && let Some(run) = run_at(snapshot, self.fact, event.native_hour())?
            {
                let at = run.span.last.min(end);
                if at >= start {
                    result[1] = Some(result[1].map_or(at, |before| before.max(at)));
                }
            }
        }
        Ok(result)
    }
}

/// Σ |c + rate·k| for k in 1..=n, exactly.
fn absolute_sum(c: i128, rate: i128, n: i128) -> Option<u128> {
    if rate == 0 {
        return c.unsigned_abs().checked_mul(u128::try_from(n).ok()?);
    }
    // Mirrored, the values rise: the first `negative` of them are below zero.
    let (c, rate) = if rate < 0 { (c.checked_neg()?, -rate) } else { (c, rate) };
    let negative = if c >= 0 {
        0
    } else {
        ((-c - 1) / rate).min(n)
    };
    let sum = |from: i128, to: i128| -> Option<i128> {
        // Σ (c + rate·k) for k in from..=to.
        let count = to - from + 1;
        if count <= 0 {
            return Some(0);
        }
        let ks = (from + to).checked_mul(count)? / 2;
        c.checked_mul(count)?.checked_add(rate.checked_mul(ks)?)
    };
    let below = sum(1, negative)?.checked_neg()?;
    let above = sum(negative + 1, n)?;
    u128::try_from(below.checked_add(above)?).ok()
}

/// A raw value as a chart shows it.
fn shown(raw: i128) -> f64 {
    raw as f64 / 10_f64.powi(i32::from(RESOURCE_DECIMALS))
}

/// The bucket of a flow's delivered integral over one bin, folded as a stored series of it folds:
/// its hours from the flow's first through the last published, each run's endpoints, ties at
/// their first hour.
struct Fold {
    bucket: Option<ChartBucket>,
}

impl Fold {
    /// Adds hours `first..=last` rising by `rate` an hour from `before`.
    fn add(&mut self, before: i128, rate: i128, first: i32, last: i32) -> io::Result<()> {
        let n = i128::from(last) - i128::from(first) + 1;
        let value = |k: i128| -> io::Result<i128> {
            rate.checked_mul(k)
                .and_then(|delta| before.checked_add(delta))
                .ok_or_else(exceeded)
        };
        let start = shown(value(1)?);
        let end = shown(value(n)?);
        // The first hour at which the values reach the last's.
        let reaching = |rising: bool| -> io::Result<i32> {
            let (mut low, mut high) = (1_i128, n);
            while low < high {
                let middle = low + (high - low) / 2;
                let at = shown(value(middle)?);
                if (rising && at >= end) || (!rising && at <= end) {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            Ok((i128::from(first) + low - 1) as i32)
        };
        let (min, max) = match rate.signum() {
            0 => ((start, first), (start, first)),
            1 => ((start, first), (end, reaching(true)?)),
            _ => ((end, reaching(false)?), (start, first)),
        };
        let total = absolute_sum(before, rate, n).ok_or_else(exceeded)?;
        let bucket = self.bucket.get_or_insert(ChartBucket {
            absolute_total: 0,
            first_hour: first,
            last_hour: last,
            first_value: start,
            last_value: end,
            last_value_hour: last,
            min_hour: min.1,
            min_value: min.0,
            max_hour: max.1,
            max_value: max.0,
            first_break: false,
            has_break: false,
        });
        bucket.absolute_total = bucket
            .absolute_total
            .checked_add(total)
            .ok_or_else(exceeded)?;
        bucket.last_hour = last;
        bucket.last_value = end;
        bucket.last_value_hour = last;
        if min.0 < bucket.min_value {
            (bucket.min_value, bucket.min_hour) = min;
        }
        if max.0 > bucket.max_value {
            (bucket.max_value, bucket.max_hour) = max;
        }
        Ok(())
    }
}

/// What a reply reads besides the publication.
pub(super) struct Read<'a> {
    pub(super) branch: u64,
    pub(super) history: &'a RecordedHistories,
    /// The last hour of the coverage read.
    pub(super) last_hour: i32,
    pub(super) complete: bool,
    pub(super) naming: &'a ResourceNaming,
    pub(super) cancelled: &'a AtomicBool,
}

/// A flow the reply names: its actor, keys and catalogue entry.
struct Named {
    keys: [u64; 2],
    first: i32,
    flow: Flow,
}

impl Read<'_> {
    /// The reply to `params` from the publication `snapshot` pins; none while it is pending.
    #[cfg(test)]
    pub(super) fn reply<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        directory: &Directory,
        params: Value,
    ) -> io::Result<Option<Value>> {
        let input: Params = serde_json::from_value(params.clone())?;
        self.pages(snapshot, directory, input, params)
    }

    /// The delivered integral's chart of flow `actor` over `view`'s bins of `bounds`.
    #[cfg(test)]
    pub(super) fn integral_chart<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        directory: &Directory,
        actor: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
    ) -> io::Result<ChartSummary> {
        let key = |measure| {
            directory
                .key(actor, measure, None)
                .ok_or_else(|| invalid("A prepared trade flow has no key"))
        };
        let mut runs = Runs {
            fact: key(FACT)?,
            first: directory.actors[actor as usize].first_hour,
            last: snapshot.publication().accepted.last,
            held: BTreeMap::new(),
        };
        self.integral_summary(snapshot, &mut runs, key(DELIVERED)?, bounds, view)
    }

    /// The delivered integral's chart over `view`'s bins of `bounds`: each bin's hours from the
    /// flow's first through the last published, bins without one unknown.
    fn integral_summary<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        runs: &mut Runs,
        delivered: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
    ) -> io::Result<ChartSummary> {
        let (exact, bins) = series::summary_bins(bounds, view)?;
        let mut buckets = vec![None; bins.len()];
        let defined = (
            bins.first().map_or(i32::MAX, |bin| bin.0).max(runs.first),
            bins.last().map_or(i32::MIN, |bin| bin.1).min(runs.last),
        );
        if defined.0 <= defined.1 {
            let mut total = match defined.0.checked_sub(1) {
                Some(before) => runs.raw(snapshot, before)?[0],
                None => 0,
            };
            // The delivered rate's runs, and no rate between them: every hour of `defined`.
            let mut pieces = Vec::new();
            let mut at = defined.0;
            for run in engine::runs(snapshot, delivered, defined.0, defined.1)? {
                if run.span.first > at {
                    pieces.push((at, run.span.first - 1, 0));
                }
                pieces.push((run.span.first, run.span.last, run.value));
                at = run.span.last.saturating_add(1);
            }
            if at <= defined.1 {
                pieces.push((at, defined.1, 0));
            }
            // The bins clipped to `defined` are contiguous: each goes on where the last ended.
            let (mut piece, mut at) = (0, defined.0);
            for (bucket, &(first, last)) in buckets.iter_mut().zip(&bins) {
                dashboard_text::metadata::check_cancel(self.cancelled)?;
                let (low, high) = (first.max(defined.0), last.min(defined.1));
                if low > high {
                    continue;
                }
                let mut fold = Fold { bucket: None };
                while at <= high {
                    let &(_, to, rate) = pieces
                        .get(piece)
                        .ok_or_else(|| invalid("A prepared trade integral has no rate"))?;
                    let through = to.min(high);
                    fold.add(total, rate, at, through)?;
                    let hours = i128::from(through) - i128::from(at) + 1;
                    total = rate
                        .checked_mul(hours)
                        .and_then(|delta| total.checked_add(delta))
                        .ok_or_else(exceeded)?;
                    if through == to {
                        piece += 1;
                    }
                    at = through.saturating_add(1);
                    if through == i32::MAX {
                        break;
                    }
                }
                if let Some(mut folded) = fold.bucket {
                    folded.first_break = low == runs.first;
                    folded.has_break = low > first || high < last;
                    *bucket = Some(folded);
                }
            }
        }
        Ok(ChartSummary {
            from: view.0,
            to: view.1,
            width: view.2,
            buckets,
            runs: exact.then(Vec::new),
            approximate: Vec::new(),
        })
    }

    fn pages<P: PageRead>(
        &self,
        snapshot: &mut Snapshot<'_, P>,
        directory: &Directory,
        input: Params,
        params: Value,
    ) -> io::Result<Option<Value>> {
        let _cpu = replay_frame::cpu::scope_phase("trade:query_pages");
        let history = self.history;
        let published = snapshot.publication().accepted;
        // An incomplete coverage is read through the last segment published by it.
        let last_hour = if self.complete {
            self.last_hour
        } else {
            self.last_hour.min(published.last)
        };
        let aliases: Vec<String> = directory
            .scalar(ALIASES)
            .map_or(&[][..], Scalar::list)
            .iter()
            .map(text)
            .collect::<io::Result<_>>()?;
        let resources: Vec<(u32, String)> = pairs(directory, RESOURCES)
            .map(|(id, token)| {
                Ok((
                    u32::try_from(id).map_err(|_| invalid("A prepared resource is not valid"))?,
                    text(token)?,
                ))
            })
            .collect::<io::Result<_>>()?;
        let (resources, mut warnings) = self.naming.infos(&resources, self.cancelled)?;
        let mut countries = aliases.clone();
        countries.sort_unstable();
        countries.dedup();
        let mut reply = TradeFlowsReply {
            branch_id: self.branch.to_string(),
            decimals: RESOURCE_DECIMALS,
            resources,
            countries,
            country_identities: history
                .countries
                .iter()
                .filter(|(_, country)| !country.identities.is_empty())
                .map(|(tag, country)| TradeCountryIdentity {
                    tag: tag.clone(),
                    runs: country.identities.as_ref().clone(),
                })
                .collect(),
            reads: Vec::new(),
            known: Vec::new(),
            partial: Vec::new(),
            flows: Vec::new(),
            warnings: Vec::new(),
        };
        let selected = if let Some(ids) = &input.identities {
            Some(
                ids.iter()
                    .map(|id| serde_json::from_str::<Identity>(id).map_err(io::Error::from))
                    .collect::<io::Result<BTreeSet<_>>>()?,
            )
        } else {
            input
                .tags
                .as_ref()
                .map(|tags| selected_identities(history, tags))
        };
        let mut membership = BTreeMap::<Identity, Vec<HourSpan>>::new();
        if let Some(members) = &input.membership {
            for (tag, spans) in members {
                for identity in history
                    .countries
                    .get(tag)
                    .into_iter()
                    .flat_map(|country| country.identities.iter())
                {
                    let clipped = combine(
                        &[HourSpan {
                            first_hour: identity.first_hour,
                            last_hour: identity.last_hour,
                        }],
                        spans,
                        true,
                    );
                    membership
                        .entry((identity.slot, identity.country_id))
                        .or_default()
                        .extend(clipped);
                }
            }
            for spans in membership.values_mut() {
                *spans = subtract(&combine(spans, &[], false), &input.unobserved);
            }
        }
        let mut flows = Vec::new();
        for (actor, named) in directory.actors.iter().enumerate() {
            let actor = actor as u64;
            let (seller_identity, buyer_identity, resource) = flow_of(&named.name)?;
            if selected.as_ref().is_some_and(|ids| {
                if input.between {
                    !(ids.contains(&seller_identity) && ids.contains(&buyer_identity))
                } else {
                    !(ids.contains(&seller_identity) || ids.contains(&buyer_identity))
                }
            }) {
                continue;
            }
            let mut keys = [0; 2];
            for (key, measure) in keys.iter_mut().zip([FACT, DELIVERED]) {
                *key = directory
                    .key(actor, measure, None)
                    .ok_or_else(|| invalid("A prepared trade flow has no key"))?;
            }
            let first = match directory
                .scalar(FIRSTS + actor)
                .map_or(&[][..], Scalar::list)
            {
                [seller, buyer] => [unsigned(seller)?, unsigned(buyer)?],
                _ => return Err(invalid("A prepared trade flow names no aliases")),
            };
            let alias = |at: u32| {
                aliases
                    .get(at as usize)
                    .cloned()
                    .ok_or_else(|| invalid("A prepared trade alias is not valid"))
            };
            flows.push(Named {
                keys,
                first: named.first_hour,
                flow: Flow {
                    seller: alias(first[0])?,
                    buyer: alias(first[1])?,
                    seller_identity,
                    buyer_identity,
                    resource,
                    runs: Vec::new(),
                },
            });
        }
        let diagnostic_selected = input
            .tags
            .as_ref()
            .map(|tags| selected_identities(history, tags));
        // Each group's key and seller and buyer, and its deals left out over every read.
        let mut groups = Vec::new();
        for &(id, key) in directory.dimensions(FAMILY, REFUSALS) {
            let pair = match directory.scalar(GROUPS + id) {
                Some(Scalar::Null) => None,
                Some(scalar) => match scalar.list() {
                    [seller_slot, seller_id, buyer_slot, buyer_id] => Some((
                        (unsigned(seller_slot)?, unsigned(seller_id)?),
                        (unsigned(buyer_slot)?, unsigned(buyer_id)?),
                    )),
                    _ => return Err(invalid("A prepared trade refusal is not valid")),
                },
                None => return Err(invalid("A prepared trade refusal names no group")),
            };
            groups.push((id, key, pair));
        }
        let totals: BTreeMap<u64, usize> = pairs(directory, TOTALS)
            .map(|(id, total)| {
                Ok((
                    id,
                    total
                        .integer()
                        .and_then(|total| usize::try_from(total).ok())
                        .ok_or_else(|| invalid("A prepared trade total is not valid"))?,
                ))
            })
            .collect::<io::Result<_>>()?;
        let selected_refusals: Vec<u64> = groups
            .iter()
            .filter(|(_, _, pair)| {
                pair.is_none_or(|(seller, buyer)| {
                    diagnostic_selected
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&seller) || ids.contains(&buyer))
                })
            })
            .map(|&(_, key, _)| key)
            .collect();
        let total = |id: &u64| totals.get(id).copied().unwrap_or(0);
        let unresolved = groups
            .iter()
            .filter(|(_, _, pair)| pair.is_none())
            .map(|(id, _, _)| total(id))
            .sum();
        let unnamed = groups
            .iter()
            .filter(|(_, key, pair)| pair.is_some() && selected_refusals.contains(key))
            .map(|(id, _, _)| total(id))
            .sum();
        let had_reads = matches!(directory.scalar(READ), Some(Scalar::Boolean(true)));
        recording_warnings(unnamed, unresolved, had_reads, &mut warnings);
        reply.warnings = warnings;
        let known = directory.key(FAMILY, KNOWN, None);
        if input.op.as_deref() == Some("navigate") {
            // A group's flow counts only while its members do, as the page reads cut it.
            let mut shown: Vec<(u64, Option<Vec<HourSpan>>)> = flows
                .iter()
                .filter(|named| {
                    input
                        .flows
                        .as_ref()
                        .is_none_or(|keys| keys.contains(&flow_key(&named.flow)))
                })
                .map(|named| {
                    let within = input.membership.as_ref().map(|_| {
                        combine(
                            membership
                                .get(&named.flow.seller_identity)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            membership
                                .get(&named.flow.buyer_identity)
                                .map(Vec::as_slice)
                                .unwrap_or(&[]),
                            input.between,
                        )
                    });
                    (named.keys[0], within)
                })
                .collect();
            shown.extend(known.map(|key| (key, None)));
            shown.extend(selected_refusals.iter().map(|&key| (key, None)));
            let keys: Vec<(u64, Option<&[HourSpan]>)> = shown
                .iter()
                .map(|(key, within)| (*key, within.as_deref()))
                .collect();
            let navigation: Navigation = serde_json::from_value(params)?;
            let hour = engine::navigate(snapshot, &keys, &navigation)?;
            return Ok(Some(json!({ "hour": hour })));
        }
        if input.op.is_some() {
            return Err(invalid("Unknown trade operation"));
        }
        let from = input.from.unwrap_or(f64::from(last_hour));
        let to = input.to.unwrap_or(from);
        let (from, to, width) =
            series::viewport(Some(from), Some(to), Some(input.width.unwrap_or(1)))?;
        let first = from.floor() as i32;
        let last = to.floor() as i32;
        if !self.complete && (last > last_hour || input.hours.iter().any(|&hour| hour > last_hour))
        {
            return Ok(None);
        }
        if input.width.is_some() {
            series::summary_budget(from, to, width, 1)?;
        }
        let dense = input
            .width
            .is_some_and(|width| i64::from(last) - i64::from(first) + 1 > i64::from(width));
        let basis = input.basis.unwrap_or(first);
        let moments: BTreeSet<_> = input.hours.iter().copied().chain([first, last]).collect();
        let measure = input.measure.as_deref().unwrap_or("current");
        if !["current", "total", "average"].contains(&measure) {
            return Err(invalid("Invalid trade measure"));
        }
        let view = (from, to, width);
        let display_coverage = subtract(
            &[HourSpan {
                first_hour: first,
                last_hour: last,
            }],
            &input.unobserved,
        );
        let known_summary = summary(
            snapshot,
            known,
            (first, last),
            view,
            0,
            Some(&display_coverage),
        )?;
        // Stable campaign aliases keep all catalogue indices fixed for paged reads.
        let countries = reply.countries.clone();
        let listed = |catalogue: &ScannedFlows| -> io::Result<Vec<TradeFlow>> {
            let (slice_countries, flows) = catalogue.reply_flows();
            let country_index = |tag: &str| {
                countries
                    .binary_search_by(|value| value.as_str().cmp(tag))
                    .map(|at| at as u32)
                    .map_err(|_| invalid("A prepared trade alias is not listed"))
            };
            let mut listed = Vec::with_capacity(flows.len());
            for mut flow in flows {
                flow.seller = country_index(&slice_countries[flow.seller as usize])?;
                flow.buyer = country_index(&slice_countries[flow.buyer as usize])?;
                for index in flow.seller_tags.iter_mut().chain(&mut flow.buyer_tags) {
                    *index = country_index(&slice_countries[*index as usize])?;
                }
                listed.push(flow);
            }
            Ok(listed)
        };
        // The reads' hours: every page holds the reads.
        let mut read_moments = moments.clone();
        if !dense {
            for (first, last) in spans_of(snapshot, &selected_refusals, first, last)? {
                read_moments.extend([first, last]);
            }
            if let Some(key) = known {
                for run in engine::runs(snapshot, key, first, last)? {
                    read_moments.extend(run.span.first..=run.span.last);
                }
            }
        }
        let mut seen = BTreeSet::new();
        for &hour in &read_moments {
            let Some(key) = known else {
                break;
            };
            if let Some(read) = last_read(snapshot, key, hour)? {
                reply.reads.push(read);
                if let Some(run) = run_at(snapshot, key, read)? {
                    reply.known.push(HourSpan {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                    });
                }
            }
            if let Some(run) = run_at(snapshot, key, hour)? {
                let mut start = run.span.first;
                let mut end = run.span.last;
                let mut skipped = 0_i128;
                for &id in &selected_refusals {
                    if let Some(refusal) = run_at(snapshot, id, hour)? {
                        start = start.max(refusal.span.first);
                        end = end.min(refusal.span.last);
                        skipped += refusal.value;
                    } else if let Some(next) =
                        snapshot.next_change(selection(id), hour, true, everything())?
                    {
                        end = end.min(next.saturating_sub(1));
                    }
                }
                if seen.insert((start, end)) {
                    reply.known.push(HourSpan {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                    });
                    if skipped > 0 {
                        reply.partial.push(PartialSpan {
                            first_hour: start,
                            last_hour: end,
                            deals: skipped
                                .try_into()
                                .map_err(|_| invalid("Invalid partial trade count"))?,
                        });
                    }
                }
            }
        }
        reply.reads.sort_unstable();
        reply.reads.dedup();
        reply.known.sort_by_key(|run| run.first_hour);
        reply
            .known
            .dedup_by_key(|run| (run.first_hour, run.last_hour));
        reply.partial.sort_by_key(|run| run.first_hour);
        let mut partial: Vec<PartialSpan> = Vec::new();
        for run in std::mem::take(&mut reply.partial) {
            if let Some(previous) = partial.last_mut()
                && previous.deals == run.deals
                && previous.last_hour.checked_add(1) == Some(run.first_hour)
            {
                previous.last_hour = run.last_hour;
            } else {
                partial.push(run);
            }
        }
        reply.partial = partial;
        // The known hours, and those leaving out a chosen group's deals, through each hour.
        let reach = read_moments
            .iter()
            .copied()
            .chain([basis.saturating_sub(1)])
            .max()
            .unwrap_or(published.first)
            .min(published.last);
        let counted = |spans: Vec<(i32, i32)>| Counted::new(union(spans), published.last);
        let known_hours = counted(match known {
            Some(key) if published.first <= reach => engine::runs(snapshot, key, published.first, reach)?
                .into_iter()
                .map(|run| (run.span.first, run.span.last))
                .collect(),
            _ => Vec::new(),
        });
        let partial_hours = counted(if published.first <= reach {
            spans_of(snapshot, &selected_refusals, published.first, reach)?
        } else {
            Vec::new()
        });
        let before = basis.saturating_sub(1);
        let mut coverage_counts = BTreeMap::new();
        for hour in read_moments {
            coverage_counts.insert(
                hour,
                [
                    known_hours.through(hour) - known_hours.through(before),
                    partial_hours.through(hour) - partial_hours.through(before),
                ],
            );
        }
        // A flow's summary, integrals and bounds are a line.
        let budget = dashboard_base::products::MAX_PRODUCT_REPLY_BYTES;
        series::paged(input.cursor.as_deref(), budget, |page| {
            let mut summaries = BTreeMap::new();
            let mut integrals = BTreeMap::new();
            let mut bounds = BTreeMap::new();
            let mut catalogue = ScannedFlows::default();
            for at in page.lines(flows.len()) {
                let Named {
                    keys: [fact, delivered],
                    first: flow_first,
                    ref flow,
                } = flows[at];
                let mut flow = flow.clone();
                dashboard_text::metadata::check_cancel(self.cancelled)?;
                let token = flow_key(&flow);
                let mut reader = Runs {
                    fact,
                    first: flow_first,
                    last: published.last,
                    held: BTreeMap::new(),
                };
                let within = input.membership.as_ref().map(|_| {
                    combine(
                        membership
                            .get(&flow.seller_identity)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        membership
                            .get(&flow.buyer_identity)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        input.between,
                    )
                });
                let flow_known = if let Some(within) = &within {
                    let within = combine(within, &display_coverage, true);
                    summary(snapshot, known, (first, last), view, 0, Some(&within))?
                } else {
                    known_summary.clone()
                };
                let mut dates = moments.clone();
                let mut found = if dense {
                    let mut summary = if measure == "current" {
                        engine::summary(
                            snapshot,
                            delivered,
                            (first, last),
                            view,
                            RESOURCE_DECIMALS,
                            within.as_deref(),
                        )?
                    } else {
                        self.integral_summary(snapshot, &mut reader, delivered, (first, last), view)?
                    };
                    if measure != "current" {
                        // A running average is bounded by its opening value and the daily amounts in
                        // the bucket. Cumulative-amount extrema alone can miss a ratio peak followed
                        // by low-volume trading.
                        let daily = (measure == "average")
                            .then(|| {
                                engine::summary(
                                    snapshot,
                                    delivered,
                                    (first, last),
                                    view,
                                    RESOURCE_DECIMALS,
                                    within.as_deref(),
                                )
                            })
                            .transpose()?;
                        let baseline = match basis.checked_sub(1) {
                            Some(hour) => reader.prefix(snapshot, hour, within.as_deref(), last_hour)?,
                            None => [0; 6],
                        };
                        for (column, bucket) in summary.buckets.iter_mut().enumerate() {
                            let Some(bucket) = bucket else {
                                continue;
                            };
                            let hours = [
                                bucket.first_hour,
                                bucket.last_hour,
                                bucket.min_hour,
                                bucket.max_hour,
                            ];
                            let mut samples = Vec::new();
                            for hour in hours {
                                let own = reader.prefix(snapshot, hour, within.as_deref(), last_hour)?;
                                let amount = own[0] - baseline[0];
                                let duration = own[5] - baseline[5];
                                let raw = if measure == "total" {
                                    amount as f64 / 24.0
                                } else if duration > 0 {
                                    amount as f64 / duration as f64
                                } else {
                                    0.0
                                };
                                samples.push((hour, raw / 10f64.powi(i32::from(RESOURCE_DECIMALS))));
                            }
                            bucket.first_value = samples[0].1;
                            bucket.last_value = samples[1].1;
                            let min = samples
                                .iter()
                                .min_by(|a, b| a.1.total_cmp(&b.1))
                                .expect("four samples");
                            let max = samples
                                .iter()
                                .max_by(|a, b| a.1.total_cmp(&b.1))
                                .expect("four samples");
                            bucket.min_hour = min.0;
                            bucket.min_value = min.1;
                            bucket.max_hour = max.0;
                            bucket.max_value = max.1;
                            bucket.last_value_hour = bucket.last_hour;
                            if let Some(daily) = daily.as_ref().and_then(|daily| daily.buckets[column])
                            {
                                if daily.min_value < bucket.min_value {
                                    bucket.min_value = daily.min_value;
                                    bucket.min_hour = daily.min_hour;
                                }
                                if daily.max_value > bucket.max_value {
                                    bucket.max_value = daily.max_value;
                                    bucket.max_hour = daily.max_hour;
                                }
                            }
                        }
                        summary.runs = None;
                    }
                    for (bucket, coverage) in summary.buckets.iter_mut().zip(&flow_known.buckets) {
                        if let Some(known) = coverage {
                            if let Some(values) = bucket {
                                if values.first_hour > known.first_hour
                                    || values.last_hour < known.last_hour
                                {
                                    values.min_value = values.min_value.min(0.0);
                                    values.max_value = values.max_value.max(0.0);
                                    if values.first_hour > known.first_hour {
                                        values.first_hour = known.first_hour;
                                        values.first_value = 0.0;
                                    }
                                    if values.last_hour < known.last_hour {
                                        values.last_hour = known.last_hour;
                                        values.last_value = 0.0;
                                        values.last_value_hour = known.last_hour;
                                    }
                                }
                                values.has_break |= known.has_break;
                                values.first_break |= known.first_break;
                            } else {
                                let mut zero = *known;
                                zero.first_value = 0.0;
                                zero.last_value = 0.0;
                                zero.min_value = 0.0;
                                zero.max_value = 0.0;
                                *bucket = Some(zero);
                            }
                        } else {
                            *bucket = None;
                        }
                    }
                    if let (Some(runs), Some(coverage)) = (&mut summary.runs, &flow_known.runs) {
                        // The native successful empty census means zero between listed deals.
                        // Apply that same existing zero fill to the exact supplying runs.
                        let active: Vec<_> = runs
                            .iter()
                            .map(|run| HourSpan {
                                first_hour: run.first_hour,
                                last_hour: run.last_hour,
                            })
                            .collect();
                        let known: Vec<_> = coverage
                            .iter()
                            .map(|run| HourSpan {
                                first_hour: run.first_hour,
                                last_hour: run.last_hour,
                            })
                            .collect();
                        for zero in subtract(&known, &active) {
                            runs.push(ChartRun {
                                first_hour: zero.first_hour,
                                last_hour: zero.last_hour,
                                value: 0.0,
                                break_before: true,
                            });
                        }
                        runs.sort_by_key(|run| run.first_hour);
                        let mut previous: Option<i32> = None;
                        for run in runs {
                            run.break_before =
                                previous.and_then(|hour| hour.checked_add(1)) != Some(run.first_hour);
                            previous = Some(run.last_hour);
                        }
                    }
                    for bucket in summary.buckets.iter().flatten() {
                        dates.extend([
                            bucket.first_hour,
                            bucket.last_hour,
                            bucket.min_hour,
                            bucket.max_hour,
                        ]);
                    }
                    summaries.insert(token.clone(), summary);
                    Vec::new()
                } else {
                    engine::runs(snapshot, fact, first, last)?
                };
                for &hour in &dates {
                    if let Some(run) = run_at(snapshot, fact, hour)? {
                        found.push(run);
                    }
                }
                let mut seen = BTreeSet::new();
                for clipped in found {
                    let run = run_at(snapshot, fact, clipped.span.first)?
                        .ok_or_else(|| invalid("Trade fact coverage is missing"))?;
                    if !seen.insert((run.span.first, run.span.last)) {
                        continue;
                    }
                    let start = i32::try_from(run.value)
                        .map_err(|_| invalid("Invalid trade fact date"))?;
                    let held = reader.held(snapshot, start)?;
                    let alias = |at: u32| {
                        aliases
                            .get(at as usize)
                            .cloned()
                            .ok_or_else(|| invalid("A prepared trade alias is not valid"))
                    };
                    let row = Run {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                        terms: held.terms,
                        seller_tag: alias(held.aliases[0])?,
                        buyer_tag: alias(held.aliases[1])?,
                    };
                    if let Some(spans) = &within {
                        for span in spans {
                            let first = row.first_hour.max(span.first_hour);
                            let last = row.last_hour.min(span.last_hour);
                            if first <= last {
                                flow.runs.push(Run {
                                    first_hour: first,
                                    last_hour: last,
                                    ..row.clone()
                                });
                            }
                        }
                    } else {
                        flow.runs.push(row);
                    }
                }
                flow.runs.sort_by_key(|run| run.first_hour);
                // Every requested prefix is a fixed number of page lookups, even in peace.
                for run in &flow.runs {
                    if let Some(hour) = run.first_hour.checked_sub(1) {
                        dates.insert(hour);
                    }
                    dates.extend([run.first_hour, run.last_hour]);
                }
                dates.extend([basis.saturating_sub(1)]);
                let mut prefixes = BTreeMap::new();
                let mut observed = BTreeMap::new();
                for hour in dates {
                    let sum = reader.prefix(snapshot, hour, within.as_deref(), last_hour)?;
                    prefixes.insert(hour, sum.map(|value| value.to_string()));
                    if hour >= basis {
                        observed.insert(
                            hour,
                            reader.bounds(snapshot, basis, hour, within.as_deref())?,
                        );
                    }
                }
                let line = json!({
                    "summary": summaries.get(&token), "prefixes": &prefixes, "bounds": &observed,
                });
                if !page.admit(at, &line)? {
                    summaries.remove(&token);
                    break;
                }
                bounds.insert(token.clone(), observed);
                integrals.insert(token, prefixes);
                catalogue.flows.push(flow);
            }
            reply.flows = listed(&catalogue)?;
            let mut data = serde_json::to_value(&reply)?;
            data["summaries"] = serde_json::to_value(summaries)?;
            data["prefixes"] = serde_json::to_value(integrals)?;
            data["bounds"] = serde_json::to_value(bounds)?;
            data["coverageCounts"] = serde_json::to_value(&coverage_counts)?;
            data["cumulativePending"] = json!(false);
            data["exactHours"] = json!(
                input
                    .hours
                    .iter()
                    .copied()
                    .chain([first, last])
                    .collect::<BTreeSet<_>>()
            );
            data["basis"] = json!(basis);
            data["scoped"] = json!(input.membership.is_some());
            data["keyCount"] = json!(directory.actors.len());
            Ok(data)
        })
        .map(Some)
    }
}

/// The runs of `keys` over `from..=to`, as spans.
fn spans_of<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: &[u64],
    from: i32,
    to: i32,
) -> io::Result<Vec<(i32, i32)>> {
    let mut spans = Vec::new();
    for &key in keys {
        for run in engine::runs(snapshot, key, from, to)? {
            spans.push((run.span.first, run.span.last));
        }
    }
    Ok(spans)
}

/// Every hour, as a bound navigation reads none outside.
fn everything() -> dashboard_series::Span {
    dashboard_series::Span {
        first: i32::MIN,
        last: i32::MAX,
    }
}

/// The last read at or before `hour` of the known key `key`, as of `hour`.
fn last_read<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<i32>> {
    if run_at(snapshot, key, hour)?.is_some() {
        return Ok(Some(hour));
    }
    let Some(previous) = snapshot.next_change(selection(key), hour, false, everything())? else {
        return Ok(None);
    };
    if let Some(run) = run_at(snapshot, key, previous)? {
        return Ok(Some(run.span.last.min(hour)));
    }
    Ok(match previous.checked_sub(1) {
        Some(at) => run_at(snapshot, key, at)?.map(|run| run.span.last.min(hour)),
        None => None,
    })
}

/// A flow's key as replies name it.
fn flow_key(flow: &Flow) -> String {
    super::flow_token(flow.seller_identity, flow.buyer_identity, flow.resource)
}
