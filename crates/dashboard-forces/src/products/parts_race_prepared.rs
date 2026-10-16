//! The prepared parts-race flow shared by the native air and ship censuses.
use dashboard_base::{
    prepared::series::{self, Accumulator, Reader, Run},
    products::{
        country_groups::{self, HourSpan},
        country_history,
    },
    source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag};
use replay::{
    container::Replay,
    format::FormatVersion,
    frame::{DecodedFrame, Frame, TableUpdate, TableView},
    rows::TableId,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(test)]
mod phase_b_tests;
#[cfg(test)]
mod progressive_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;

pub type Query = fn(&Source, &str, &Value, &LoadContext, &AtomicBool) -> io::Result<Value>;

pub struct Selection {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<u32>,
    pub channel: usize,
    pub group: usize,
}

/// A recorded tag and its native holder slot and id, when resolved.
pub type Holder = (Vec<u8>, Option<(u32, u32)>);

/// Only native input/projection shapes differ. Series, ranking and request flow live here.
pub trait Family {
    type Scan: Default + Serialize + DeserializeOwned;
    type Raw: Clone + Serialize + DeserializeOwned;
    type Scanned: Serialize + DeserializeOwned;
    type Part: Clone + Serialize + DeserializeOwned;
    type Reply: Serialize + dashboard_base::products::ReplyWarnings;
    type PointContent;
    const POINT_PARTS: bool;
    const NAME: &'static str;
    const GROUPS: usize;
    const MIXES: usize;
    const REFERENCES: usize;
    const TABLES: &'static [TableId];
    const SAMPLE_TABLES: &'static [TableId];
    const FIELDS: &'static [&'static str];
    #[cfg(test)]
    fn work(scan: &Self::Scan) -> dashboard_base::prepared::observer_tests::Work;
    fn holders(scan: &Self::Scan) -> &[Holder];
    fn point_content(source: &Source, context: &LoadContext) -> Self::PointContent;
    fn point_reply(
        scan: &Self::Scan,
        observed: &BTreeMap<String, Self::Raw>,
        selection: &Selection,
        content: &Self::PointContent,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Self::Reply>>;
    fn projectable(_raw: &Self::Raw) -> bool {
        true
    }
    fn configure(source: &Source, context: &LoadContext);
    fn applied(scan: &Self::Scan) -> Option<i32>;
    fn changed(scan: &Self::Scan, index: usize, hour: i32) -> BTreeMap<String, Self::Raw>;
    fn removed(scan: &Self::Scan) -> &[String];
    fn projection_scan(scan: &Self::Scan) -> Self::Scanned;
    fn step(
        scan: &mut Self::Scan,
        raw: &Frame,
        frame: &DecodedFrame,
        format: FormatVersion,
        cancelled: &AtomicBool,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()>;
    fn resume(
        scan: &mut Self::Scan,
        raw: &Frame,
        frame: &DecodedFrame,
        format: FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()>;
    fn observed(scan: &Self::Scan, index: usize, hour: i32) -> BTreeMap<String, Self::Raw>;
    fn runs(scan: &Self::Scan, index: usize) -> BTreeMap<String, &[Self::Raw]>;
    fn finish_hour(_scan: &mut Self::Scan) {}
    fn prune(scan: &mut Self::Scan);
    fn metadata(scan: &Self::Scan) -> Self::Scanned;
    fn bounds(raw: &Self::Raw) -> (i32, i32, bool);
    fn window(raw: &mut Self::Raw, first: i32, last: i32, starts: bool);
    fn total(raw: &Self::Raw, index: usize) -> Option<i64>;
    fn transient(run: Run, index: usize, maximum: Option<i128>) -> io::Result<Self::Raw>;
    fn maximum(_raw: &Self::Raw) -> Option<i64> {
        None
    }
    fn reference(_index: usize) -> Option<usize> {
        None
    }
    fn part_key(part: &Self::Part) -> &str;
    fn selection(params: Value) -> io::Result<Selection>;
    fn insert(
        scanned: &mut Self::Scanned,
        index: usize,
        tag: String,
        runs: Vec<Self::Raw>,
        segments: Vec<HourSpan>,
    );
    fn render(
        source: &Source,
        selection: &Selection,
        context: &LoadContext,
        cancelled: &AtomicBool,
        scanned: &Self::Scanned,
        history: Option<&country_history::RecordedHistories>,
        membership: Option<BTreeMap<String, Vec<HourSpan>>>,
    ) -> io::Result<Self::Reply>;
    #[allow(clippy::type_complexity)]
    fn projection(reply: Self::Reply) -> (Vec<Self::Part>, BTreeMap<String, Vec<Option<i64>>>);
}

#[derive(Serialize, Deserialize)]
#[serde(bound(
    serialize = "F::Scan: Serialize, F::Part: Serialize",
    deserialize = "F::Scan: DeserializeOwned, F::Part: DeserializeOwned"
))]
pub struct Prepared<F: Family> {
    scan: F::Scan,
    keys: BTreeMap<String, u64>,
    values: [Accumulator; 4],
    ranks: [Accumulator; 4],
    parts: Vec<Accumulator>,
    catalogue: Vec<BTreeMap<String, F::Part>>,
    mixes: Vec<Accumulator>,
    maximum: Vec<Accumulator>,
    // Current nonzero/refused parts, and still-open (including zero) part keys.
    present_parts: Vec<BTreeMap<String, BTreeSet<String>>>,
    open_parts: Vec<BTreeMap<String, BTreeSet<String>>>,
    #[serde(skip)]
    ready: bool,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    first: Option<i32>,
    last: i32,
}
impl<F: Family> Default for Prepared<F> {
    fn default() -> Self {
        Self {
            scan: Default::default(),
            keys: Default::default(),
            values: Default::default(),
            ranks: Default::default(),
            parts: (0..4 * F::GROUPS).map(|_| Accumulator::default()).collect(),
            catalogue: (0..4 * F::GROUPS).map(|_| BTreeMap::new()).collect(),
            mixes: (0..F::MIXES).map(|_| Accumulator::default()).collect(),
            maximum: (0..F::REFERENCES).map(|_| Accumulator::default()).collect(),
            present_parts: (0..4 * F::GROUPS).map(|_| BTreeMap::new()).collect(),
            open_parts: (0..4 * F::GROUPS).map(|_| BTreeMap::new()).collect(),
            ready: false,
            #[cfg(test)]
            work: Default::default(),
            first: None,
            last: 0,
        }
    }
}
fn mix_index<F: Family>(index: usize) -> usize {
    if F::MIXES == 1 { 0 } else { index }
}
fn mix_section<F: Family>(branch: u64, index: usize) -> String {
    if F::MIXES == 1 {
        format!("{}-mixes-{branch}", F::NAME)
    } else {
        format!("{}-mixes-{branch}-{index}", F::NAME)
    }
}
pub(super) fn updates(frame: &DecodedFrame, tables: &[TableId]) -> Frame {
    updates_changed(frame, tables, None, None)
}
pub(super) fn updates_changed(
    frame: &DecodedFrame,
    tables: &[TableId],
    changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    baseline: Option<i32>,
) -> Frame {
    let mut result = Frame {
        branch: frame.branch,
        hour: frame.hour,
        keyframe: false,
        metrics: frame.metrics.clone(),
        tables: std::array::from_fn(|_| TableUpdate::Unavailable {
            reason: String::new(),
        }),
    };
    for &id in tables {
        if let Some(baseline) =
            baseline.filter(|_| changes.is_some_and(|changes| !changes.rows_changed[id.index()]))
        {
            result.tables[id.index()] = match &*frame.tables[id.index()] {
                TableView::Complete(_) => TableUpdate::Unchanged { baseline },
                TableView::Unavailable { reason } => TableUpdate::Unavailable {
                    reason: reason.clone(),
                },
                TableView::Truncated { .. } => TableUpdate::Unavailable {
                    reason: String::new(),
                },
            };
            continue;
        }
        result.tables[id.index()] = match &*frame.tables[id.index()] {
            TableView::Complete(rows) => TableUpdate::Snapshot(rows.clone()),
            TableView::Truncated { rows, missing } => TableUpdate::Truncated {
                rows: rows.clone(),
                missing: *missing,
            },
            TableView::Unavailable { reason } => TableUpdate::Unavailable {
                reason: reason.clone(),
            },
        };
    }
    result
}

/// Stateless point reads use the native census; preparation projects its already updated census.
#[cfg(test)]
fn sample<F: Family>(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<dashboard_base::prepared::query::progressive::Samples> {
    let scan = point_scan::<F>(frame, &AtomicBool::new(false))?;
    let observed = std::array::from_fn(|index| F::observed(&scan, index, frame.hour));
    sample_rows::<F>(&scan, &observed, requested)
}

fn point_scan<F: Family>(frame: &DecodedFrame, cancelled: &AtomicBool) -> io::Result<F::Scan> {
    let mut scan = F::Scan::default();
    F::step(
        &mut scan,
        &updates(frame, F::TABLES),
        frame,
        FormatVersion::CURRENT,
        cancelled,
        None,
    )?;
    Ok(scan)
}

fn sample_rows<F: Family>(
    scan: &F::Scan,
    observed: &[BTreeMap<String, F::Raw>; 4],
    requested: Option<&[(String, String)]>,
) -> io::Result<dashboard_base::prepared::query::progressive::Samples> {
    if requested.is_some_and(|keys| {
        keys.iter().any(|(_, field)| {
            field.parse::<usize>().ok().is_none_or(|index| index >= 4)
                && field
                    .strip_prefix("maximum-")
                    .and_then(|field| field.parse::<usize>().ok())
                    .is_none_or(|index| index >= 4 || F::reference(index).is_none())
        })
    }) {
        return Err(dashboard_core::invalid("Unknown force sample channel"));
    }
    let wanted = requested.map(|keys| keys.iter().cloned().collect::<BTreeSet<_>>());
    let mut samples = BTreeMap::new();
    for (tag, identity) in F::holders(scan) {
        let Ok(tag) = std::str::from_utf8(tag) else {
            continue;
        };
        for (index, rows) in observed.iter().enumerate() {
            let key = (tag.to_owned(), index.to_string());
            if wanted.as_ref().is_none_or(|wanted| {
                wanted.contains(&key)
                    || wanted.contains(&(tag.to_owned(), format!("maximum-{index}")))
            }) {
                samples.insert(
                    key,
                    (
                        *identity,
                        rows.get(tag)
                            .and_then(|raw| F::total(raw, index))
                            .map(i128::from),
                    ),
                );
                if F::reference(index).is_some() {
                    samples.insert(
                        (tag.to_owned(), format!("maximum-{index}")),
                        (
                            *identity,
                            rows.get(tag).and_then(F::maximum).map(i128::from),
                        ),
                    );
                }
            }
        }
    }
    if let Some(requested) = requested {
        for key in requested {
            samples.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(samples)
}
pub(super) fn transient_at(runs: &[Run], hour: i32) -> Option<Run> {
    let index = runs
        .partition_point(|run| run.first_hour <= hour)
        .checked_sub(1)?;
    runs.get(index).copied().filter(|run| hour <= run.last_hour)
}
fn part_amount(total: Option<i128>, supplied: Option<Option<i128>>) -> Option<i128> {
    supplied.unwrap_or_else(|| total.map(|_| 0))
}

/// A part absent from a complete native census is zero; an explicitly refused part stays unknown.
fn part_runs(totals: &[Run], values: &[Run], witnessed: &[(i32, i32, u32, u32)]) -> Vec<Run> {
    let mut dates = BTreeSet::new();
    for (first, last) in totals
        .iter()
        .chain(values)
        .map(|run| (run.first_hour, run.last_hour))
        .chain(witnessed.iter().map(|span| (span.0, span.1)))
    {
        dates.insert(i64::from(first));
        dates.insert(i64::from(last) + 1);
    }
    let dates = dates.into_iter().collect::<Vec<_>>();
    dates
        .windows(2)
        .filter_map(|pair| {
            let first = i32::try_from(pair[0]).ok()?;
            let total = transient_at(totals, first)?;
            let part = transient_at(values, first);
            let supplied = witnessed
                .iter()
                .any(|span| span.0 <= first && span.1 >= first)
                .then_some(part.map(|run| run.value));
            let value = part_amount(Some(total.value), supplied)?;
            Some(Run::new(
                first,
                (pair[1] - 1) as i32,
                value,
                total.break_before && total.first_hour == first
                    || part.is_some_and(|run| run.break_before && run.first_hour == first),
            ))
        })
        .collect()
}
pub fn retain_tail<T>(items: &mut Vec<T>) {
    if items.len() > 1 {
        items.drain(..items.len() - 1);
    }
}
impl<F: Family> Prepared<F> {
    fn key(&mut self, tag: &str, field: &str) -> u64 {
        let next = self.keys.len() as u64;
        *self
            .keys
            .entry(series::field_key(tag, field))
            .or_insert(next)
    }
    pub fn resume(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        F::resume(
            &mut self.scan,
            &updates(frame, F::TABLES),
            frame,
            replay.format,
            cancelled,
        )
    }
    pub fn configure(&self, source: &Source, context: &LoadContext) {
        F::configure(source, context);
    }
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        source: &Source,
        context: &LoadContext,
        cancelled: &AtomicBool,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        let raw = updates_changed(frame, F::TABLES, Some(changes), F::applied(&self.scan));
        F::step(
            &mut self.scan,
            &raw,
            frame,
            replay.format,
            cancelled,
            Some(changes),
        )?;
        self.first.get_or_insert(frame.hour);
        self.last = frame.hour;
        for accumulator in self
            .values
            .iter_mut()
            .chain(&mut self.parts)
            .chain(&mut self.mixes)
            .chain(&mut self.maximum)
        {
            accumulator.advance(frame.hour)?;
        }
        for index in 0..4 {
            let observed = if self.ready {
                F::changed(&self.scan, index, frame.hour)
            } else {
                F::observed(&self.scan, index, frame.hour)
            };
            let changed: BTreeMap<_, _> = observed
                .iter()
                .filter(|(_, raw)| F::projectable(raw))
                .map(|(tag, raw)| (tag.clone(), vec![raw.clone()]))
                .collect();
            let mut projected = Vec::new();
            if !changed.is_empty() {
                let mut snapshot = F::projection_scan(&self.scan);
                let tags: Vec<String> = changed.keys().cloned().collect();
                for (tag, runs) in changed {
                    F::insert(&mut snapshot, index, tag, runs, Vec::new());
                }
                for group in 0..F::GROUPS {
                    let selection = Selection {
                        branch_id: frame.branch.to_string(),
                        tags: tags.clone(),
                        top: None,
                        channel: index,
                        group,
                    };
                    let (parts, countries) = F::projection(F::render(
                        source, &selection, context, cancelled, &snapshot, None, None,
                    )?);
                    let columns = parts
                        .iter()
                        .map(|part| F::part_key(part).to_owned())
                        .collect::<Vec<_>>();
                    self.catalogue[index * F::GROUPS + group].extend(
                        parts
                            .into_iter()
                            .map(|part| (F::part_key(&part).to_owned(), part)),
                    );
                    projected.push((columns, countries));
                }
            }
            let removed = F::removed(&self.scan).to_vec();
            let rows = observed
                .into_iter()
                .map(|(tag, raw)| (tag, Some(raw)))
                .chain(removed.into_iter().map(|tag| (tag, None)));
            for (tag, raw) in rows {
                let key = self.key(&tag, "");
                let starts = raw.as_ref().is_some_and(|raw| {
                    let (first, _, starts) = F::bounds(raw);
                    starts && first == frame.hour
                });
                let total = raw
                    .as_ref()
                    .and_then(|raw| F::total(raw, index))
                    .map(i128::from);
                let mix = mix_index::<F>(index);
                if index == mix {
                    let changed = raw
                        .as_ref()
                        .is_some_and(|raw| F::bounds(raw).0 == frame.hour)
                        || self.mixes[mix].state(key).is_none();
                    let version = self.mixes[mix]
                        .state(key)
                        .and_then(|state| state.last)
                        .map_or(0, |run| run.value)
                        + i128::from(changed);
                    if let Some(raw) = raw.as_ref().filter(|_| changed) {
                        self.mixes[mix].event(key, frame.hour, raw)?;
                    }
                    self.mixes[mix].observe(key, frame.hour, raw.as_ref().map(|_| version), starts);
                }
                self.values[index].observe(key, frame.hour, total, starts);
                if let Some(reference) = F::reference(index) {
                    self.maximum[reference].observe(
                        key,
                        frame.hour,
                        raw.as_ref().and_then(F::maximum).map(i128::from),
                        starts,
                    );
                }
                for group in 0..F::GROUPS {
                    let part_index = index * F::GROUPS + group;
                    let amounts = projected.get(group).and_then(|(columns, countries)| {
                        countries.get(&tag).map(|amounts| {
                            columns
                                .iter()
                                .zip(amounts)
                                .map(|(part, amount)| (part.clone(), amount.map(i128::from)))
                                .collect::<BTreeMap<_, _>>()
                        })
                    });
                    let previous = self.present_parts[part_index]
                        .remove(&tag)
                        .unwrap_or_default();
                    if let Some(amounts) = amounts.filter(|_| total.is_some()) {
                        let mut changed = previous;
                        if starts || !self.open_parts[part_index].contains_key(&tag) {
                            // A newly authenticated span reopens its formerly known zeros too.
                            // The tuple prefix visits this country's affected keys only.
                            let mut prefix = series::field_key(&tag, &format!("{part_index}/"));
                            prefix.truncate(prefix.len() - 2);
                            for (key, _) in self
                                .keys
                                .range(prefix.clone()..)
                                .take_while(|(key, _)| key.starts_with(&prefix))
                            {
                                let (_, field): (String, String) = serde_json::from_str(key)?;
                                changed.insert(
                                    field.split_once('/').expect("part field").1.to_owned(),
                                );
                            }
                        }
                        let current: BTreeSet<_> = amounts
                            .iter()
                            .filter(|(_, value)| **value != Some(0))
                            .map(|(part, _)| part.clone())
                            .collect();
                        self.open_parts[part_index].entry(tag.clone()).or_default();
                        for part in changed.union(&current) {
                            let amount = amounts.get(part).copied().unwrap_or(Some(0));
                            let part_key = self.key(&tag, &format!("{part_index}/{part}"));
                            self.parts[part_index].observe(part_key, frame.hour, amount, starts);
                            let open = self.open_parts[part_index].entry(tag.clone()).or_default();
                            if amount.is_some() {
                                open.insert(part.clone());
                            } else {
                                open.remove(part);
                            }
                            #[cfg(test)]
                            {
                                self.work.emitted += 1;
                            }
                        }
                        self.present_parts[part_index].insert(tag.clone(), current);
                    } else {
                        for part in self.open_parts[part_index].remove(&tag).unwrap_or_default() {
                            let part_key = self.key(&tag, &format!("{part_index}/{part}"));
                            self.parts[part_index].observe(part_key, frame.hour, None, starts);
                            #[cfg(test)]
                            {
                                self.work.emitted += 1;
                            }
                        }
                    }
                }
                #[cfg(test)]
                {
                    self.work.history_visits += 1;
                    self.work.emitted += 2;
                }
            }
        }
        self.ready = true;
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.values
            .iter()
            .chain(&self.ranks)
            .chain(&self.parts)
            .chain(&self.mixes)
            .chain(&self.maximum)
            .map(Accumulator::bytes)
            .sum()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let Some(first) = self.first.take() else {
            return Ok(());
        };
        F::finish_hour(&mut self.scan);
        for index in 0..4 {
            let runs: BTreeMap<_, _> = F::runs(&self.scan, index)
                .into_iter()
                .map(|(tag, runs)| {
                    (
                        tag,
                        runs.iter()
                            .filter(|run| F::bounds(run).1 >= first)
                            .map(|run| {
                                let mut raw = run.clone();
                                let (_, last, starts) = F::bounds(&raw);
                                let clipped = F::bounds(&raw).0.max(first);
                                F::window(&mut raw, clipped, last, starts);
                                raw
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            self.ranks[index].through(first);
            dashboard_base::ranking::rank_positions(
                &runs,
                |run| {
                    let (first, last, _) = F::bounds(run);
                    (first, last)
                },
                |run| F::total(run, index).map(i128::from),
                cancelled,
                |position, tag, from, to| {
                    let key = self.keys[&series::field_key(tag, "")];
                    self.ranks[index].push(
                        position as u64 * 2,
                        Run::new(from, to, i128::from(key), false),
                    );
                    self.ranks[index]
                        .push(key * 2 + 1, Run::new(from, to, position as i128 + 1, false));
                    Ok(())
                },
            )?;
            self.ranks[index].through(self.last);
            self.ranks[index].flush(
                writer,
                &format!("{}-ranks-{branch}-{index}", F::NAME),
                0,
                cancelled,
            )?;
            self.values[index].flush(
                writer,
                &format!("{}-values-{branch}-{index}", F::NAME),
                0,
                cancelled,
            )?;
        }
        for (index, parts) in self.parts.iter_mut().enumerate() {
            parts.flush(
                writer,
                &format!("{}-parts-{branch}-{index}", F::NAME),
                0,
                cancelled,
            )?;
        }
        for (index, maximum) in self.maximum.iter_mut().enumerate() {
            maximum.flush(
                writer,
                &format!("{}-maximum-{branch}-{index}", F::NAME),
                0,
                cancelled,
            )?;
        }
        for (index, mixes) in self.mixes.iter_mut().enumerate() {
            mixes.flush(writer, &mix_section::<F>(branch, index), 0, cancelled)?;
        }
        F::prune(&mut self.scan);
        series::write_catalogue(
            writer,
            F::NAME,
            branch,
            &self.keys,
            &(F::metadata(&self.scan), &self.catalogue),
        )
    }
}
pub fn query<F: Family>(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let branch = params["branchId"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| dashboard_core::invalid("Invalid force branch"))?;
    let coverage = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?;
    if !coverage.as_ref().is_some_and(|coverage| coverage.complete)
        && matches!(
            product,
            "series-summary" | "series-state" | "series-next-change"
        )
    {
        return progressive_query::<F>(
            source,
            product,
            params,
            context,
            coverage.as_ref(),
            cancelled,
        );
    }
    prepared_query::<F>(source, product, params, context, cancelled)
}

fn selection<F: Family>(params: &Value, tags: &[String]) -> io::Result<Selection> {
    let mut native =
        json!({"branchId":params["branchId"],"tags":tags,"top":params["family"]["top"]});
    for &field in F::FIELDS {
        native[field] = params["family"][field].clone();
    }
    F::selection(native)
}

fn prepared_query<F: Family>(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let reads: Vec<dashboard_base::prepared_measure::query::MeasureRead> =
        if product == "series-state" {
            serde_json::from_value(params["reads"].clone())?
        } else {
            Vec::new()
        };
    let read_tags: Vec<_> = reads.iter().map(|read| read.tag.clone()).collect();
    let tags = if product == "series-state" {
        read_tags.clone()
    } else {
        serde_json::from_value(params.get("tags").cloned().unwrap_or_else(|| json!([])))?
    };
    let selection = selection::<F>(params, &tags)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid(format!("Invalid {} branch", F::NAME)))?;
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let coverage = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?;
    let history = dashboard_base::prepared::query::progressive::histories(
        &store,
        &source_key,
        branch,
        coverage.as_ref(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        source.replay_ref(),
        &selection.tags,
        cancelled,
    )?;
    let Some((keys, (mut scanned, catalogue))) = series::read_catalogue::<(
        F::Scanned,
        Vec<BTreeMap<String, F::Part>>,
    )>(&store, &source_key, F::NAME, branch)?
    else {
        return Ok(Value::Null);
    };
    let index = selection.channel;
    let part_index = index * F::GROUPS + selection.group;
    let inverse: HashMap<_, _> = history
        .countries
        .keys()
        .filter_map(|tag| {
            keys.get(&series::field_key(tag, ""))
                .map(|&key| (key, tag.clone()))
        })
        .collect();
    let mut values = Reader::new(
        &store,
        &source_key,
        format!("{}-values-{branch}-{index}", F::NAME),
        cancelled,
    )?;
    let mut ranks = Reader::new(
        &store,
        &source_key,
        format!("{}-ranks-{branch}-{index}", F::NAME),
        cancelled,
    )?;
    let mut mixes = Reader::new(
        &store,
        &source_key,
        mix_section::<F>(branch, index),
        cancelled,
    )?;
    let within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    if product == "series-next-change" {
        let navigation: series::Navigation = serde_json::from_value(params.clone())?;
        let mut value_keys = Vec::new();
        let mut rank_keys = Vec::new();
        if let Some(top) = selection.top {
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            for position in 0..(top as usize).min(inverse.len()) {
                rank_keys.push((position as u64 * 2, None));
                if let Some(run) = ranks.value_at(position as u64 * 2, hour)? {
                    value_keys.push((run.value as u64, None));
                }
            }
        } else {
            for tag in country_history::selected_aliases(&history, &selection.tags) {
                if let Some(&key) = keys.get(&series::field_key(&tag, "")) {
                    value_keys.push((key, within.get(&tag).map(Vec::as_slice)));
                }
            }
        }
        return Ok(
            json!({"hour":series::navigate(&mut [(&mut values,value_keys), (&mut ranks,rank_keys)], &navigation)?}),
        );
    }
    if product == "events-page" {
        let (first, last, cursor, size) = series::event_window(
            params["from"].as_f64(),
            params["to"].as_f64(),
            params["cursor"].as_str(),
            params["pageSize"]
                .as_u64()
                .and_then(|size| u32::try_from(size).ok()),
        )?;
        let selected: Vec<_> = country_history::selected_aliases(&history, &selection.tags)
            .iter()
            .filter_map(|tag| keys.get(&series::field_key(tag, "")))
            .copied()
            .collect();
        let page = mixes.events(&selected, first, last, cursor, size)?;
        return Ok(
            json!({"firstOrdinal":page.first.to_string(),"total":page.total.to_string(),"cursor":page.cursor.map(|cursor|cursor.to_string()),
            "rows":page.rows.into_iter().map(|row|json!({"tag":inverse.get(&row.key),"hour":row.hour,"ordinal":row.ordinal.to_string(),"row":row.row})).collect::<Vec<_>>(),"warnings":[]}),
        );
    }
    let (from, to, width) = if product == "series-state" {
        (
            f64::from(reads.iter().map(|read| read.hour).min().unwrap_or(0)),
            f64::from(reads.iter().map(|read| read.hour).max().unwrap_or(0)),
            1,
        )
    } else {
        series::viewport(
            params["from"].as_f64(),
            params["to"].as_f64(),
            params["width"]
                .as_u64()
                .and_then(|width| u32::try_from(width).ok()),
        )?
    };
    let (first, last) = (from.floor() as i32, to.ceil() as i32);
    if product == "events-clusters" {
        let tag = params["tag"]
            .as_str()
            .ok_or_else(|| dashboard_core::invalid(format!("Missing {} event tag", F::NAME)))?;
        validate_tag(tag).map_err(dashboard_core::invalid)?;
        let summary = if let Some(&key) = keys.get(&series::field_key(tag, "")) {
            mixes.event_clusters(key, from, to, width)?
        } else {
            dashboard_base::prepared_chart::MarkSummary {
                from,
                to,
                width,
                clusters: Vec::new(),
            }
        };
        return Ok(
            json!({"branchId":selection.branch_id,"tag":tag,"summary":summary,"warnings":[]}),
        );
    }
    let mut membership: Option<BTreeMap<String, Vec<HourSpan>>> =
        selection.top.map(|_| BTreeMap::new());
    if let Some(top) = selection.top {
        for position in 0..(top as usize).min(inverse.len()) {
            for run in ranks.runs(position as u64 * 2, first, last)? {
                let tag = inverse.get(&(run.value as u64)).ok_or_else(|| {
                    dashboard_core::invalid(format!("Unknown {} rank key", F::NAME))
                })?;
                membership
                    .as_mut()
                    .unwrap()
                    .entry(tag.clone())
                    .or_default()
                    .push(HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    });
            }
        }
    }
    if let Some(members) = &mut membership {
        for spans in members.values_mut() {
            spans.sort_by_key(|span| span.first_hour);
            let mut merged = Vec::new();
            for span in std::mem::take(spans) {
                country_groups::mark_unobserved(&mut merged, span.first_hour, span.last_hour);
            }
            *spans = merged;
        }
    }
    let mut wanted: Vec<String> = membership.as_ref().map_or_else(
        || country_history::selected_aliases(&history, &selection.tags),
        |members| members.keys().cloned().collect(),
    );
    if product == "series-state" {
        wanted = read_tags;
        wanted.sort();
        wanted.dedup();
    }
    let selected = country_history::selected_identities(&history, &selection.tags);
    let dense =
        product != "series-state" && i64::from(last) - i64::from(first) + 1 > i64::from(width);
    let mut display = Vec::new();
    let mut parts = Reader::new(
        &store,
        &source_key,
        format!("{}-parts-{branch}-{part_index}", F::NAME),
        cancelled,
    )?;
    let mut reference = if dense {
        F::reference(index)
            .map(|reference| {
                Reader::new(
                    &store,
                    &source_key,
                    format!("{}-maximum-{branch}-{reference}", F::NAME),
                    cancelled,
                )
            })
            .transpose()?
    } else {
        None
    };
    for tag in &wanted {
        let Some(&key) = keys.get(&series::field_key(tag, "")) else {
            continue;
        };
        let mut spans: Vec<_> = if let Some(members) = &membership {
            members[tag].clone()
        } else {
            history.countries[tag]
                .identities
                .iter()
                .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect()
        };
        if let Some(within) = within.get(tag) {
            let mut clipped = Vec::new();
            dashboard_base::ranking::for_each_piece(
                &spans,
                within,
                |span| (span.first_hour, span.last_hour),
                cancelled,
                |_, first_hour, last_hour| {
                    clipped.push(HourSpan {
                        first_hour,
                        last_hour,
                    });
                    Ok(())
                },
            )?;
            spans = clipped;
        }
        let summary =
            values.summary_within(key, (first, last), (from, to, width), 0, Some(&spans))?;
        let mut dates = BTreeSet::new();
        if let Some(reference) = &mut reference {
            let maximum =
                reference.summary_within(key, (first, last), (from, to, width), 0, Some(&spans))?;
            for bucket in maximum.buckets.iter().flatten() {
                dates.extend([
                    bucket.first_hour,
                    bucket.last_hour,
                    bucket.min_hour,
                    bucket.max_hour,
                ]);
            }
        }
        let mut part_summaries = Vec::new();
        if dense {
            for part in catalogue[part_index].values() {
                if let Some(&part_key) = keys.get(&series::field_key(
                    tag,
                    &format!("{part_index}/{}", F::part_key(part)),
                )) {
                    let summary = parts.summary_within(
                        part_key,
                        (first, last),
                        (from, to, width),
                        0,
                        Some(&spans),
                    )?;
                    for bucket in summary.buckets.iter().flatten() {
                        dates.extend([
                            bucket.first_hour,
                            bucket.last_hour,
                            bucket.min_hour,
                            bucket.max_hour,
                        ]);
                    }
                    part_summaries.push(json!({"key":F::part_key(part),"summary":summary}));
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
        }
        let runs = if dense || product == "series-state" {
            if product == "series-state" {
                dates.extend(
                    reads
                        .iter()
                        .filter(|read| &read.tag == tag)
                        .map(|read| read.hour),
                );
            }
            let mut runs = BTreeMap::new();
            for hour in dates {
                if let Some(run) = mixes.value_at(key, hour)? {
                    if product == "series-state" {
                        let starts = run.break_before && run.first_hour == hour;
                        runs.insert(
                            hour,
                            Run {
                                first_hour: hour,
                                last_hour: hour,
                                break_before: starts,
                                ..run
                            },
                        );
                    } else {
                        runs.insert(run.first_hour, run);
                    }
                }
            }
            runs.into_values().collect()
        } else {
            mixes.runs(key, first, last)?
        };
        let mut raw = Vec::new();
        for run in runs {
            let (_, row) = mixes.latest_event(key, run.first_hour)?.ok_or_else(|| {
                dashboard_core::invalid(format!("Missing prepared {} composition", F::NAME))
            })?;
            let mut value: F::Raw = serde_json::from_value(row)?;
            let bounds = (
                if product == "series-state" {
                    run.first_hour
                } else {
                    run.first_hour.max(first)
                },
                if product == "series-state" {
                    run.last_hour
                } else {
                    run.last_hour.min(last)
                },
            );
            F::window(&mut value, bounds.0, bounds.1, run.break_before);
            if bounds.0 <= bounds.1 {
                raw.push(value);
            }
        }
        let segments = history.countries[tag]
            .segments
            .iter()
            .filter(|span| span.first_hour <= last && span.last_hour >= first)
            .cloned()
            .collect();
        F::insert(&mut scanned, index, tag.clone(), raw, segments);
        if product != "series-state" {
            series::summary_budget(from, to, width, display.len() + 1)?;
            display.push(json!({"tag":tag,"summary":summary,"parts":part_summaries}));
        }
    }
    let mut params = selection;
    params.tags = wanted;
    let mut reply = serde_json::to_value(F::render(
        source,
        &params,
        context,
        cancelled,
        &scanned,
        Some(&history),
        membership,
    )?)?;
    reply["summaries"] = json!(display);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

fn progressive_query<F: Family>(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    use dashboard_base::prepared::query::progressive;
    let reads: Vec<dashboard_base::prepared_measure::query::MeasureRead> =
        if product == "series-state" {
            serde_json::from_value(params["reads"].clone())?
        } else {
            Vec::new()
        };
    let tags: Vec<String> = if product == "series-state" {
        reads.iter().map(|read| read.tag.clone()).collect()
    } else {
        serde_json::from_value(params["tags"].clone())?
    };
    let selection = selection::<F>(params, &tags)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid force branch"))?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Unknown force branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(source.replay_ref(), start);
    let navigation = (product == "series-next-change")
        .then(|| serde_json::from_value::<series::Navigation>(params.clone()))
        .transpose()?;
    let (from, to, width) = if let Some(navigation) = &navigation {
        navigation.origin()?;
        (navigation.first_hour, navigation.last_hour, 1)
    } else if product == "series-state" {
        (
            f64::from(reads.iter().map(|read| read.hour).min().unwrap_or(0)),
            f64::from(reads.iter().map(|read| read.hour).max().unwrap_or(0)),
            1,
        )
    } else {
        series::viewport(
            params["from"].as_f64(),
            params["to"].as_f64(),
            params["width"]
                .as_u64()
                .and_then(|width| u32::try_from(width).ok()),
        )?
    };
    let field = selection.channel.to_string();
    if params["requireParts"] == true && !F::POINT_PARTS {
        return if coverage.is_some_and(|coverage| to <= f64::from(coverage.last_hour)) {
            prepared_query::<F>(source, product, params, context, cancelled)
        } else {
            Ok(Value::Null)
        };
    }
    let mut requested = tags
        .iter()
        .map(|tag| (tag.clone(), field.clone()))
        .collect::<Vec<_>>();
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let point_content = (params["requireParts"] == true).then(|| F::point_content(source, context));
    let point_parts = std::cell::RefCell::new(BTreeMap::<String, F::Part>::new());
    let point_warnings = std::cell::RefCell::new(BTreeSet::new());
    let point_metadata = std::cell::RefCell::new(None);
    let part_index = selection.channel * F::GROUPS + selection.group;
    let sampler = |frame: &DecodedFrame, keys: Option<&[(String, String)]>| {
        let scan = point_scan::<F>(frame, cancelled)?;
        let observed = std::array::from_fn(|index| F::observed(&scan, index, frame.hour));
        let mut samples = sample_rows::<F>(&scan, &observed, None)?;
        if let Some(point_content) = &point_content
            && let Some(mut reply) = F::point_reply(
                &scan,
                &observed[selection.channel],
                &selection,
                point_content,
                cancelled,
            )?
        {
            point_warnings.borrow_mut().extend(
                dashboard_base::products::ReplyWarnings::warnings_mut(&mut reply)
                    .iter()
                    .cloned(),
            );
            let (parts, rows) = F::projection(reply);
            for (tag, values) in rows {
                let identity = samples
                    .get(&(tag.clone(), field.clone()))
                    .and_then(|sample| sample.0);
                for (part, value) in parts.iter().zip(values) {
                    samples.insert(
                        (tag.clone(), format!("{part_index}/{}", F::part_key(part))),
                        (identity, value.map(i128::from)),
                    );
                }
            }
            point_parts.borrow_mut().extend(
                parts
                    .into_iter()
                    .map(|part| (F::part_key(&part).to_owned(), part)),
            );
        }
        *point_metadata.borrow_mut() = Some(F::metadata(&scan));
        samples.retain(|(_, channel), _| {
            channel == &field
                || channel == &format!("maximum-{}", selection.channel)
                || channel.starts_with(&format!("{part_index}/"))
        });
        if let Some(keys) = keys {
            for key in keys {
                samples.entry(key.clone()).or_insert((None, None));
            }
        }
        Ok(samples)
    };
    let mut tail = if product == "series-state" {
        progressive::state_tail(
            source,
            branch,
            coverage,
            F::SAMPLE_TABLES,
            &requested,
            &reads.iter().map(|read| read.hour).collect::<Vec<_>>(),
            &sampler,
            cancelled,
        )?
    } else {
        progressive::tail(
            source,
            branch,
            coverage,
            F::SAMPLE_TABLES,
            &requested,
            (from.floor() as i32, to.floor() as i32),
            &sampler,
            cancelled,
        )?
    };
    let history = progressive::histories(
        &store,
        &source_key,
        branch,
        coverage,
        &tail.2,
        &BTreeMap::new(),
        source.replay_ref(),
        &tags,
        cancelled,
    )?;
    let selected = country_history::selected_identities(&history, &tags);
    let mut keys = series::read_catalogue::<Value>(&store, &source_key, F::NAME, branch)?
        .map(|row| row.0)
        .unwrap_or_default();
    let mut values = Reader::new(
        &store,
        &source_key,
        format!("{}-values-{branch}-{}", F::NAME, selection.channel),
        cancelled,
    )?;
    if let Some(navigation) = navigation {
        let mut within: BTreeMap<String, Vec<HourSpan>> =
            serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
        requested = country_history::selected_aliases(&history, &tags)
            .into_iter()
            .map(|tag| (tag, field.clone()))
            .collect();
        let mut preceding = BTreeMap::new();
        if selection.top.is_none() {
            for key in &requested {
                let native = history
                    .countries
                    .get(&key.0)
                    .into_iter()
                    .flat_map(|country| country.identities.iter())
                    .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                    .map(|identity| HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    })
                    .collect::<Vec<_>>();
                let scopes = if let Some(spans) = within.get(&key.0) {
                    let mut scopes = Vec::new();
                    dashboard_base::ranking::for_each_piece(
                        &native,
                        spans,
                        |span| (span.first_hour, span.last_hour),
                        cancelled,
                        |_, first_hour, last_hour| {
                            scopes.push(HourSpan {
                                first_hour,
                                last_hour,
                            });
                            Ok(())
                        },
                    )?;
                    scopes
                } else {
                    native
                };
                within.insert(key.0.clone(), scopes);
            }
        }
        if let Some(top) = selection.top {
            let sampled: BTreeMap<_, _> = tail
                .0
                .iter()
                .filter(|((_, channel), _)| channel == &field)
                .map(|((tag, _), runs)| (tag.clone(), runs.clone()))
                .collect();
            let mut next = keys.values().max().map_or(0, |key| key + 1);
            for tag in sampled.keys() {
                keys.entry(series::field_key(tag, "")).or_insert_with(|| {
                    let key = next;
                    next += 1;
                    key
                });
            }
            requested = history
                .countries
                .keys()
                .map(|tag| (tag.clone(), field.clone()))
                .collect();
            within = dashboard_base::ranking::top_membership(
                &sampled,
                top as usize,
                |run| (run.first_hour, run.last_hour),
                |run| Some(run.value),
                cancelled,
            )?;
            let mut ranks = Reader::new(
                &store,
                &source_key,
                format!("{}-ranks-{branch}-{}", F::NAME, selection.channel),
                cancelled,
            )?;
            dashboard_base::ranking::rank_positions(
                &sampled,
                |run| (run.first_hour, run.last_hour),
                |run| Some(run.value),
                cancelled,
                |position, tag, first, last| {
                    if position < top as usize {
                        tail.0
                            .entry((String::new(), format!("rank-{position}")))
                            .or_default()
                            .push(Run::new(
                                first,
                                last,
                                i128::from(keys[&series::field_key(tag, "")]),
                                false,
                            ));
                    }
                    Ok(())
                },
            )?;
            let inverse: HashMap<_, _> = history
                .countries
                .keys()
                .filter_map(|tag| keys.get(&series::field_key(tag, "")).map(|&key| (key, tag)))
                .collect();
            for position in 0..(top as usize).min(inverse.len()) {
                let rank = (String::new(), format!("rank-{position}"));
                let actor = position as u64 * 2;
                let before = coverage
                    .map(|coverage| ranks.value_at(actor, coverage.last_hour))
                    .transpose()?
                    .flatten();
                let hour = series::navigate(&mut [(&mut ranks, vec![(actor, None)])], &navigation)?;
                preceding.insert(rank.clone(), (hour, before));
                requested.push(rank);
                if let Some(run) = ranks.value_at(
                    actor,
                    navigation
                        .origin()?
                        .clamp(navigation.first_hour, navigation.last_hour)
                        .floor() as i32,
                )? && let Some(tag) = inverse.get(&(run.value as u64))
                {
                    within.entry((*tag).clone()).or_default().push(HourSpan {
                        first_hour: navigation.first_hour.floor() as i32,
                        last_hour: coverage.map_or(i32::MIN, |coverage| coverage.last_hour),
                    });
                }
            }
            for key in &requested {
                if !key.0.is_empty() {
                    within.entry(key.0.clone()).or_default();
                }
            }
            for spans in within.values_mut() {
                spans.sort_by_key(|span| span.first_hour);
            }
        }
        for key in &requested {
            if key.0.is_empty() {
                continue;
            }
            let actor = keys
                .get(&series::field_key(&key.0, ""))
                .copied()
                .unwrap_or(u64::MAX);
            let before = coverage
                .map(|coverage| values.value_at(actor, coverage.last_hour))
                .transpose()?
                .flatten();
            let hour = series::navigate(
                &mut [(
                    &mut values,
                    vec![(actor, within.get(&key.0).map(Vec::as_slice))],
                )],
                &navigation,
            )?;
            preceding.insert(key.clone(), (hour, before));
        }
        return Ok(
            json!({"hour": progressive::navigate(&preceding, &tail.0, &requested, &within, &navigation)?}),
        );
    }
    let mut prefix_params = params.clone();
    let prefix = if let Some(coverage) = coverage {
        if product == "series-state" {
            prefix_params["reads"] = json!(
                reads
                    .iter()
                    .filter(|read| read.hour <= coverage.last_hour)
                    .collect::<Vec<_>>()
            );
        } else {
            prefix_params["to"] = json!(to.min(f64::from(coverage.last_hour)));
        }
        if from <= f64::from(coverage.last_hour) {
            prepared_query::<F>(source, product, &prefix_params, context, cancelled)?
        } else {
            Value::Null
        }
    } else {
        Value::Null
    };
    let manual = country_history::selected_aliases(&history, &tags)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut sampled = BTreeMap::new();
    for ((tag, channel), runs) in &tail.0 {
        if channel != &field || selection.top.is_none() && !manual.contains(tag) {
            continue;
        }
        let mut clipped = Vec::new();
        if selection.top.is_none() {
            let spans = history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect::<Vec<_>>();
            dashboard_base::ranking::for_each_piece(
                runs,
                &spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                |run, first, last| {
                    clipped.push(Run::new(
                        first,
                        last,
                        run.value,
                        run.break_before && first == run.first_hour,
                    ));
                    Ok(())
                },
            )?;
        } else {
            clipped.extend(runs.iter().copied());
        }
        sampled.insert(tag.clone(), clipped);
    }
    let membership = selection
        .top
        .map(|top| {
            dashboard_base::ranking::top_membership(
                &sampled,
                top as usize,
                |run| (run.first_hour, run.last_hour),
                |run| Some(run.value),
                cancelled,
            )
        })
        .transpose()?;
    let mut scanned = point_metadata
        .into_inner()
        .unwrap_or_else(|| F::metadata(&F::Scan::default()));
    let mut changes_by_tag: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for ((tag, channel), runs) in &tail.0 {
        if channel != &field {
            for run in runs {
                changes_by_tag
                    .entry(tag.clone())
                    .or_default()
                    .extend([i64::from(run.first_hour), i64::from(run.last_hour) + 1]);
            }
        }
    }
    for ((tag, channel), spans) in &tail.2 {
        if channel != &field {
            for &(first, last, _, _) in spans {
                changes_by_tag
                    .entry(tag.clone())
                    .or_default()
                    .extend([i64::from(first), i64::from(last) + 1]);
            }
        }
    }
    for (tag, runs) in &sampled {
        let mut raw = Vec::new();
        for run in runs {
            let mut dates =
                BTreeSet::from([i64::from(run.first_hour), i64::from(run.last_hour) + 1]);
            if let Some(changes) = changes_by_tag.get(tag) {
                dates.extend(
                    changes
                        .range(i64::from(run.first_hour)..=i64::from(run.last_hour))
                        .copied()
                        .filter(|date| *date > i64::from(run.first_hour)),
                );
            }
            let dates = dates.into_iter().collect::<Vec<_>>();
            for piece in dates.windows(2) {
                let first = piece[0] as i32;
                let maximum = tail
                    .0
                    .get(&(tag.clone(), format!("maximum-{}", selection.channel)))
                    .and_then(|runs| transient_at(runs, first))
                    .map(|run| run.value);
                raw.push(F::transient(
                    Run::new(
                        first,
                        (piece[1] - 1) as i32,
                        run.value,
                        run.break_before && first == run.first_hour,
                    ),
                    selection.channel,
                    maximum,
                )?);
            }
        }
        let spans = tail
            .2
            .get(&(tag.clone(), field.clone()))
            .into_iter()
            .flatten()
            .map(|&(first_hour, last_hour, _, _)| HourSpan {
                first_hour,
                last_hour,
            })
            .collect();
        F::insert(&mut scanned, selection.channel, tag.clone(), raw, spans);
    }
    let mut tail_selection = Selection {
        tags: if selection.top.is_some() {
            sampled.keys().cloned().collect()
        } else {
            selection.tags.clone()
        },
        ..selection
    };
    tail_selection.top = None;
    let tail_reply = serde_json::to_value(F::render(
        source,
        &tail_selection,
        context,
        cancelled,
        &scanned,
        Some(&history),
        membership,
    )?)?;
    let mut reply = if prefix.is_null() {
        let mut reply = tail_reply.clone();
        reply["series"] = json!([]);
        reply
    } else {
        prefix
    };
    let warnings = reply["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(tail_reply["warnings"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .chain(point_warnings.into_inner())
        .collect::<BTreeSet<_>>();
    reply["warnings"] = json!(warnings);
    let before_parts = reply["parts"].as_array().cloned().unwrap_or_default();
    let mut catalogue: BTreeMap<String, Value> = before_parts
        .iter()
        .filter_map(|part| {
            part["key"]
                .as_str()
                .map(|key| (key.to_owned(), part.clone()))
        })
        .collect();
    for (key, part) in point_parts.borrow().iter() {
        catalogue.insert(key.clone(), serde_json::to_value(part)?);
    }
    let part_keys = catalogue.keys().cloned().collect::<Vec<_>>();
    reply["parts"] = json!(catalogue.into_values().collect::<Vec<_>>());
    for row in reply["series"].as_array_mut().into_iter().flatten() {
        for run in row["runs"].as_array_mut().into_iter().flatten() {
            let known = !run["total"].is_null();
            let old = run["values"].as_array().cloned().unwrap_or_default();
            run["values"] = json!(
                part_keys
                    .iter()
                    .map(|key| before_parts
                        .iter()
                        .position(|part| part["key"].as_str() == Some(key.as_str()))
                        .and_then(|index| old.get(index))
                        .cloned()
                        .unwrap_or_else(|| if known { json!(0) } else { Value::Null }))
                    .collect::<Vec<_>>()
            );
        }
    }
    let mut sampled_parts = BTreeMap::new();
    if params["requireParts"] == true {
        for (tag, totals) in &sampled {
            for part in &part_keys {
                let key = (tag.clone(), format!("{part_index}/{part}"));
                sampled_parts.insert(
                    key.clone(),
                    part_runs(
                        totals,
                        tail.0.get(&key).map_or(&[][..], Vec::as_slice),
                        tail.2.get(&key).map_or(&[][..], Vec::as_slice),
                    ),
                );
            }
        }
    }
    let mut positions: HashMap<String, usize> = reply["series"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, row)| row["tag"].as_str().map(|tag| (tag.to_owned(), index)))
        .collect();
    if let Some(rows) = tail_reply["series"].as_array() {
        for incoming in rows {
            let mut incoming = incoming.clone();
            let tag = incoming["tag"].as_str().unwrap_or("").to_owned();
            for run in incoming["runs"].as_array_mut().into_iter().flatten() {
                let hour = run["firstHour"].as_i64().unwrap_or(0) as i32;
                run["values"] = json!(
                    part_keys
                        .iter()
                        .map(|part| sampled_parts
                            .get(&(tag.clone(), format!("{part_index}/{part}")))
                            .and_then(|runs| transient_at(runs, hour))
                            .map(|run| run.value))
                        .collect::<Vec<_>>()
                );
            }
            let rows = reply["series"]
                .as_array_mut()
                .ok_or_else(|| dashboard_core::invalid("Missing native force rows"))?;
            if let Some(row) = positions
                .get(&tag)
                .and_then(|&position| rows.get_mut(position))
            {
                for field in ["runs", "segments", "membership"] {
                    if let (Some(target), Some(source)) =
                        (row[field].as_array_mut(), incoming[field].as_array())
                    {
                        target.extend(source.iter().cloned());
                    }
                }
            } else {
                positions.insert(tag, rows.len());
                rows.push(incoming);
            }
        }
    }
    for row in reply["series"].as_array_mut().into_iter().flatten() {
        if let Some(country) = row["tag"]
            .as_str()
            .and_then(|tag| history.countries.get(tag))
        {
            row["identities"] = json!(country.identities);
        }
    }
    let mut within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    let wanted: BTreeSet<_> = reply["series"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["tag"].as_str().map(str::to_owned))
        .collect();
    for row in reply["series"].as_array().into_iter().flatten() {
        let Some(tag) = row["tag"].as_str() else {
            continue;
        };
        let membership = if selection.top.is_some() {
            serde_json::from_value::<Vec<HourSpan>>(row["membership"].clone())?
        } else {
            history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect()
        };
        let scopes = if let Some(requested) = within.get(tag) {
            let mut scopes = Vec::new();
            dashboard_base::ranking::for_each_piece(
                &membership,
                requested,
                |span| (span.first_hour, span.last_hour),
                cancelled,
                |_, first_hour, last_hour| {
                    scopes.push(HourSpan {
                        first_hour,
                        last_hour,
                    });
                    Ok(())
                },
            )?;
            scopes
        } else {
            membership
        };
        within.insert(tag.to_owned(), scopes);
    }
    series::summary_budget(
        from,
        to,
        width,
        wanted.len().saturating_mul(part_keys.len() + 1),
    )?;
    let mut part_reader = Reader::new(
        &store,
        &source_key,
        format!("{}-parts-{branch}-{part_index}", F::NAME),
        cancelled,
    )?;
    let mut summaries = Vec::new();
    for tag in wanted {
        let actor = keys
            .get(&series::field_key(&tag, ""))
            .copied()
            .unwrap_or(u64::MAX);
        let runs = sampled.get(&tag).map_or(&[][..], Vec::as_slice);
        let mut summary = values.summary_with_tail(
            actor,
            bounds,
            (from, to, width),
            0,
            runs,
            within.get(&tag).map(Vec::as_slice),
        )?;
        progressive::clip_approximate(&mut summary, &tail.1, within.get(&tag).map(Vec::as_slice));
        let mut parts = Vec::new();
        for part in &part_keys {
            let field = format!("{part_index}/{part}");
            let actor = keys
                .get(&series::field_key(&tag, &field))
                .copied()
                .unwrap_or(u64::MAX);
            let runs = sampled_parts
                .get(&(tag.clone(), field))
                .map_or(&[][..], Vec::as_slice);
            let mut part_summary = part_reader.summary_with_tail(
                actor,
                bounds,
                (from, to, width),
                0,
                runs,
                within.get(&tag).map(Vec::as_slice),
            )?;
            progressive::clip_approximate(
                &mut part_summary,
                &tail.1,
                within.get(&tag).map(Vec::as_slice),
            );
            parts.push(json!({"key": part, "summary": part_summary}));
        }
        summaries.push(json!({"tag":tag,"summary":summary,"parts":parts}));
    }
    if product == "series-state" {
        let mut moments = BTreeMap::<String, Vec<HourSpan>>::new();
        for read in &reads {
            moments.entry(read.tag.clone()).or_default().push(HourSpan {
                first_hour: read.hour,
                last_hour: read.hour,
            });
        }
        for spans in moments.values_mut() {
            spans.sort_by_key(|span| span.first_hour);
            spans.dedup();
        }
        let rows = reply["series"]
            .as_array_mut()
            .ok_or_else(|| dashboard_core::invalid("Missing native force rows"))?;
        rows.retain(|row| {
            row["tag"]
                .as_str()
                .is_some_and(|tag| moments.contains_key(tag))
        });
        for row in rows {
            let spans = &moments[row["tag"].as_str().unwrap()];
            let runs = row["runs"]
                .as_array()
                .ok_or_else(|| dashboard_core::invalid("Missing native force runs"))?;
            let mut reached = Vec::new();
            dashboard_base::ranking::for_each_piece(
                runs,
                spans,
                |run| {
                    (
                        run["firstHour"].as_i64().unwrap() as i32,
                        run["lastHour"].as_i64().unwrap() as i32,
                    )
                },
                cancelled,
                |run, first, last| {
                    let mut run = run.clone();
                    run["firstHour"] = json!(first);
                    run["lastHour"] = json!(last);
                    reached.push(run);
                    Ok(())
                },
            )?;
            row["runs"] = json!(reached);
        }
    }
    reply["summaries"] = json!(summaries);
    reply["approximate"] = json!(tail.1);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}
