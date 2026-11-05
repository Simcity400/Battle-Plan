//! Resource balance's replies, read from the family's series publication ([`super::emit`]): a
//! tag's resources with their pool runs, the deals known at the requested hours, the chart
//! summaries of a dense view and keyboard navigation, in the shapes the row pages answered.

use super::{
    CountryParams, PoolRun, RESOURCE_DECIMALS, ResourceBalanceReply, ScannedBalance,
    TradeDirection, TradeRun,
    emit::{COUNTS, KNOWN, MEMBERS, NAMES, POOL, SCALARS, SIZES, VALUE, kind},
};
use dashboard_base::{
    Source,
    prepared::{
        coverage::Coverage,
        engine,
        series::{self, Navigation},
    },
    prepared_chart::ChartSummary,
    products::{LoadContext, country_groups::HourSpan},
};
use dashboard_series::{
    Cell, PageRead, Selection, Snapshot,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    pub branch_id: String,
    pub tag: String,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    hours: Vec<i32>,
    pub op: Option<String>,
    #[serde(default)]
    resources: Vec<u32>,
    within: Option<Vec<HourSpan>>,
    #[serde(default)]
    pub require_complete: bool,
    cursor: Option<String>,
    /// A page budget below the transport's, so tests can page small replies.
    #[cfg(test)]
    page_budget: Option<usize>,
}

impl Params {
    /// Refuses a viewport no reply can answer, `last` standing in for its missing ends.
    pub fn check_view(&self, last: i32) -> io::Result<()> {
        series::viewport(
            self.from.or(Some(f64::from(last))),
            self.to.or(self.from).or(Some(f64::from(last))),
            Some(self.width.unwrap_or(1)),
        )
        .map(drop)
    }
}

/// A tag's keys in the publication.
struct Keys<'d> {
    directory: &'d Directory,
    actor: Option<u64>,
}

impl Keys<'_> {
    fn key(&self, measure: u64, dimension: Option<u64>) -> Option<u64> {
        self.actor
            .and_then(|actor| self.directory.key(actor, measure, dimension))
    }

    fn pool(&self, resource: u32) -> Option<u64> {
        self.key(POOL, Some(u64::from(resource)))
    }

    fn value(&self, resource: u32, index: usize) -> Option<u64> {
        self.key(VALUE, Some(u64::from(resource) << 4 | index as u64))
    }

    fn members(&self, resource: u32) -> Option<u64> {
        self.key(MEMBERS, Some(u64::from(resource)))
    }

    fn scalar(&self, id: u64) -> &[Scalar] {
        self.actor
            .and_then(|actor| self.directory.scalar(actor * SCALARS + id))
            .map_or(&[][..], Scalar::list)
    }
}

/// What the tag's scalars hold: its resources, counts and longest balance bars, as the row
/// catalogue kept them; none when the family never named the tag.
fn scanned(keys: &Keys<'_>) -> io::Result<Option<(ScannedBalance, BTreeMap<u32, String>)>> {
    if keys.actor.is_none() {
        return Ok(None);
    }
    let malformed = || invalid("Prepared resource scalars are not valid");
    let id = |scalar: &Scalar| {
        scalar
            .integer()
            .and_then(|id| u32::try_from(id).ok())
            .ok_or_else(malformed)
    };
    let mut scanned = ScannedBalance::default();
    for pair in keys.scalar(NAMES) {
        let [resource, token] = pair.list() else {
            return Err(malformed());
        };
        let token = token
            .text()
            .and_then(|token| String::from_utf8(token.to_vec()).ok())
            .ok_or_else(malformed)?;
        scanned.resources.push((id(resource)?, token));
    }
    let counts = keys.scalar(COUNTS);
    let count = |at: usize| {
        counts
            .get(at)
            .and_then(Scalar::integer)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0)
    };
    scanned.unnamed = count(0);
    scanned.unresolved = count(1);
    let mut maxima = BTreeMap::new();
    for pair in keys.scalar(SIZES) {
        let [resource, size] = pair.list() else {
            return Err(malformed());
        };
        let resource = id(resource)?;
        scanned.series.insert(resource, Vec::new());
        maxima.insert(resource, size.integer().ok_or_else(malformed)?.to_string());
    }
    Ok(Some((scanned, maxima)))
}

fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
) -> io::Result<Option<dashboard_series::Run>> {
    Ok(match key {
        Some(key) => engine::run_at(snapshot, key, hour)?.map(|(run, _)| run),
        None => None,
    })
}

fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    from: i32,
    to: i32,
) -> io::Result<Vec<dashboard_series::Run>> {
    match key {
        Some(key) => engine::runs(snapshot, key, from, to),
        None => Ok(Vec::new()),
    }
}

/// The fields of `key`'s latest event of `kind` by `hour`.
fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
    want: u64,
) -> io::Result<Option<Vec<Cell>>> {
    let Some(event) = snapshot.latest_event(
        Selection {
            key,
            membership: None,
        },
        hour,
        None,
    )?
    else {
        return Ok(None);
    };
    let EventView::Family(event) = event else {
        return Err(invalid("A prepared resource event is not valid"));
    };
    if event.kind != want {
        return Err(invalid("A prepared resource event is not valid"));
    }
    let count = event.fields.len / Cell::SIZE as u64;
    let mut cells = Vec::with_capacity(count as usize);
    for at in 0..count {
        cells.push(snapshot.field(event.fields, at)?);
    }
    Ok(Some(cells))
}

fn integer(cell: &Cell) -> io::Result<i128> {
    match *cell {
        Cell::Integer(value) => Ok(value),
        _ => Err(invalid("A prepared resource event is not valid")),
    }
}

fn unsigned(cell: &Cell) -> io::Result<u64> {
    match *cell {
        Cell::Unsigned(value) => Ok(value),
        _ => Err(invalid("A prepared resource event is not valid")),
    }
}

fn boolean(cell: &Cell) -> io::Result<bool> {
    match *cell {
        Cell::Boolean(value) => Ok(value),
        _ => Err(invalid("A prepared resource event is not valid")),
    }
}

fn narrow<T: TryFrom<i128>>(value: i128) -> io::Result<T> {
    T::try_from(value).map_err(|_| invalid("A prepared resource event is not valid"))
}

/// The pool run a POOL event holds, over `first_hour..=last_hour`.
fn pool_run(cells: &[Cell], first_hour: i32, last_hour: i32) -> io::Result<PoolRun> {
    let [starts, values @ ..] = cells else {
        return Err(invalid("A prepared resource pool is not valid"));
    };
    if values.len() != 8 {
        return Err(invalid("A prepared resource pool is not valid"));
    }
    let raw = |at: usize| integer(&values[at]).map(|value| value.to_string());
    Ok(PoolRun {
        first_hour,
        last_hour,
        starts_segment: boolean(starts)?,
        produced_raw: raw(0)?,
        subjects_raw: raw(1)?,
        imported_raw: raw(2)?,
        to_export_raw: raw(3)?,
        exported_raw: raw(4)?,
        available_raw: raw(5)?,
        after_projects_raw: raw(6)?,
        production_raw: raw(7)?,
    })
}

/// The trade run a DEAL event holds, over `first_hour..=last_hour`.
fn trade_run(
    directory: &Directory,
    cells: &[Cell],
    first_hour: i32,
    last_hour: i32,
) -> io::Result<TradeRun> {
    let [
        deal,
        resource,
        export,
        partner,
        delivered,
        factories,
        assigned,
        required,
        starts,
    ] = cells
    else {
        return Err(invalid("A prepared resource deal is not valid"));
    };
    let partner = usize::try_from(unsigned(partner)?)
        .ok()
        .and_then(|partner| directory.actors.get(partner))
        .ok_or_else(|| invalid("A prepared resource deal names an unknown country"))?;
    Ok(TradeRun {
        deal: unsigned(deal)?.to_string(),
        resource: narrow(i128::from(unsigned(resource)?))?,
        direction: if boolean(export)? {
            TradeDirection::Export
        } else {
            TradeDirection::Import
        },
        partner: String::from_utf8_lossy(&partner.name).into_owned(),
        delivered_raw: integer(delivered)?.to_string(),
        factories: narrow(integer(factories)?)?,
        convoys_assigned: narrow(integer(assigned)?)?,
        convoys_required: narrow(integer(required)?)?,
        first_hour,
        last_hour,
        starts_segment: boolean(starts)?,
    })
}

/// `key`'s chart over `view` within `bounds`; every bin unknown where the family never named
/// the key.
fn summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    bounds: (i32, i32),
    view: (f64, f64, u32),
    within: Option<&[HourSpan]>,
) -> io::Result<ChartSummary> {
    if let Some(key) = key {
        return engine::summary(snapshot, key, bounds, view, RESOURCE_DECIMALS, within);
    }
    let (exact, bins) = series::summary_bins(bounds, view)?;
    // The membership is refused as the row reader refused it, key or none.
    if let Some(spans) = within
        && (spans.iter().any(|span| span.first_hour > span.last_hour)
            || spans
                .windows(2)
                .any(|pair| pair[0].last_hour >= pair[1].first_hour))
    {
        return Err(invalid("Invalid series membership"));
    }
    Ok(ChartSummary {
        from: view.0,
        to: view.1,
        width: view.2,
        buckets: vec![None; bins.len()],
        runs: exact.then(Vec::new),
        approximate: Vec::new(),
    })
}

/// A `resource-balance` request answered from `snapshot`, the publication pinned through
/// `coverage`: its last hour (`last_hour`) bounds what is answered while it is not complete.
#[allow(clippy::too_many_arguments)]
pub fn reply<P: PageRead>(
    source: &Source,
    context: &LoadContext,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    input: Params,
    params: &Value,
    coverage: &Coverage,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let _cpu = replay_frame::cpu::scope_phase("resources:query_pages");
    let last_hour = coverage.last_hour;
    let complete = coverage.complete;
    let keys = Keys {
        directory,
        actor: directory.actor(input.tag.as_bytes()),
    };
    let (mut reply, maxima) = match scanned(&keys)? {
        Some((scanned, maxima)) => (
            super::reply(
                source,
                &CountryParams {
                    // The catalogue named its branch by number; a tag outside it echoes the request.
                    branch_id: input
                        .branch_id
                        .parse::<u64>()
                        .map_err(|_| invalid("Invalid resource branch"))?
                        .to_string(),
                    tag: input.tag.clone(),
                },
                context,
                &scanned,
                cancelled,
            )?,
            maxima,
        ),
        None => (
            ResourceBalanceReply {
                branch_id: input.branch_id.clone(),
                tag: input.tag.clone(),
                decimals: RESOURCE_DECIMALS,
                resources: Vec::new(),
                series: Vec::new(),
                deal_hours: Vec::new(),
                trades: Vec::new(),
                warnings: vec![format!(
                    "{}: no recorded resources in this branch",
                    input.tag
                )],
            },
            BTreeMap::new(),
        ),
    };
    let known = keys.key(KNOWN, None);
    let selected = |id: u32| input.resources.is_empty() || input.resources.contains(&id);
    if input.op.as_deref() == Some("navigate") {
        let mut selected_keys: Vec<u64> = reply
            .series
            .iter()
            .filter(|row| selected(row.id))
            .filter_map(|row| keys.pool(row.id))
            .collect();
        selected_keys.extend(
            reply
                .resources
                .iter()
                .filter(|row| selected(row.id))
                .filter_map(|row| keys.members(row.id)),
        );
        // Each span of the membership is walked on its own, overlapping or unordered, and a
        // reversed one never, as the row navigation walked them.
        let mut navigated: Vec<(u64, Option<&[HourSpan]>)> = Vec::new();
        for &key in &selected_keys {
            match input.within.as_deref() {
                None => navigated.push((key, None)),
                Some(spans) => navigated.extend(
                    spans
                        .iter()
                        .filter(|span| span.first_hour <= span.last_hour)
                        .map(|span| (key, Some(std::slice::from_ref(span)))),
                ),
            }
        }
        let navigation: Navigation = serde_json::from_value(params.clone())?;
        let hour = engine::navigate(snapshot, &navigated, &navigation)?;
        return Ok(Some(json!({ "hour": hour })));
    }
    if input.op.is_some() {
        return Err(invalid("Unknown resource operation"));
    }
    let view_from = input.from.unwrap_or(f64::from(last_hour));
    let view_to = input.to.unwrap_or(view_from);
    let (view_from, view_to, _) = series::viewport(
        Some(view_from),
        Some(view_to),
        Some(input.width.unwrap_or(1)),
    )?;
    let from = view_from.floor() as i32;
    let to = view_to.floor() as i32;
    if from > to || input.width == Some(0) {
        return Err(invalid("Invalid resource viewport"));
    }
    if !complete && (to > last_hour || input.hours.iter().any(|&hour| hour > last_hour)) {
        return Ok(None);
    }
    let dense = input
        .width
        .is_some_and(|width| i64::from(to) - i64::from(from) + 1 > i64::from(width));
    if let Some(width) = input.width {
        series::summary_budget(view_from, view_to, width, 1)?;
    }
    // Each selected resource's eleven summaries are a line; every page holds every row's facts.
    #[cfg(not(test))]
    let budget = dashboard_base::products::MAX_PRODUCT_REPLY_BYTES;
    #[cfg(test)]
    let budget = input
        .page_budget
        .unwrap_or(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES);
    let asked: BTreeSet<i32> = input.hours.iter().copied().collect();
    let exact_hours: BTreeSet<i32> = input.hours.iter().copied().chain([from, to]).collect();
    let mut hours = exact_hours.clone();
    if !dense {
        for run in runs(snapshot, known, from, to)? {
            hours.extend([run.span.first, run.span.last]);
        }
        for resource in &reply.resources {
            for run in runs(snapshot, keys.members(resource.id), from, to)? {
                hours.extend([run.span.first, run.span.last]);
            }
        }
    }
    let mut seen = BTreeSet::new();
    if known.is_some() {
        for hour in hours {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let Some(run) = run_at(snapshot, known, hour)? else {
                continue;
            };
            reply.deal_hours.push(HourSpan {
                first_hour: run.span.first,
                last_hour: run.span.last,
            });
            for resource in &reply.resources {
                let Some(members) = keys.members(resource.id) else {
                    continue;
                };
                let Some(cells) = latest(snapshot, members, hour, kind::MEMBERS)? else {
                    continue;
                };
                for cell in &cells {
                    let deal = unsigned(cell)?;
                    if let Some(run) = run_at(snapshot, Some(deal), hour)?
                        && seen.insert((deal, run.span.first, run.span.last))
                    {
                        let origin: i32 = narrow(run.value)?;
                        let cells = latest(snapshot, deal, origin, kind::TRADE)?
                            .ok_or_else(|| invalid("Resource deal event is missing"))?;
                        reply.trades.push(trade_run(
                            directory,
                            &cells,
                            run.span.first,
                            run.span.last,
                        )?);
                    }
                }
            }
        }
    }
    reply.deal_hours.sort_by_key(|run| run.first_hour);
    reply
        .deal_hours
        .dedup_by_key(|run| (run.first_hour, run.last_hour));
    reply
        .trades
        .sort_by_key(|run| (run.first_hour, run.deal.clone()));
    // Every row's runs are read again for each answer: a row's line may leave the page.
    series::paged(input.cursor.as_deref(), budget, |page| {
        let mut lines = 0;
        let mut summaries = Vec::new();
        for row in reply.series.iter_mut() {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let key = keys.pool(row.id);
            let mut moments = asked.clone();
            let ordinal = selected(row.id).then(|| {
                lines += 1;
                lines - 1
            });
            let mut held = if let Some(at) = ordinal.filter(|&at| dense && page.wants(at)) {
                let width = input.width.unwrap_or(1);
                let mut line = Vec::with_capacity(11);
                let mut drawn = BTreeSet::new();
                for index in 0..11 {
                    let value = keys.value(row.id, index);
                    let summary = summary(
                        snapshot,
                        value,
                        (from, to),
                        (view_from, view_to, width),
                        input.within.as_deref(),
                    )?;
                    for bucket in summary.buckets.iter().flatten() {
                        drawn.extend([
                            bucket.first_hour,
                            bucket.last_hour,
                            bucket.min_hour,
                            bucket.max_hour,
                        ]);
                    }
                    line.push(summary);
                }
                let line = json!({
                    "id": row.id, "supply": line[0], "demand": line[1], "parts": &line[2..10],
                    "total": line[10],
                });
                if page.admit(at, &line)? {
                    moments.extend(drawn);
                    summaries.push(line);
                }
                Vec::new()
            } else if dense {
                Vec::new()
            } else {
                runs(snapshot, key, from, to)?
            };
            for &hour in &moments {
                if let Some(run) = run_at(snapshot, key, hour)? {
                    held.push(run);
                }
            }
            row.runs = pool_runs(snapshot, key, held)?;
        }
        let mut data = serde_json::to_value(&reply)?;
        data["summaries"] = json!(summaries);
        data["campaignSizes"] = serde_json::to_value(&maxima)?;
        data["exactHours"] = json!(exact_hours);
        Ok(data)
    })
    .map(Some)
}

/// The pool runs of `key` that `held` reaches, each once and in hour order, as read at its origin.
fn pool_runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    held: Vec<dashboard_series::Run>,
) -> io::Result<Vec<PoolRun>> {
    let mut dates = BTreeSet::new();
    let mut runs = Vec::with_capacity(held.len());
    for run in held {
        let run = run_at(snapshot, key, run.span.first)?
            .ok_or_else(|| invalid("Resource pool coverage is missing"))?;
        if !dates.insert((run.span.first, run.span.last)) {
            continue;
        }
        let origin: i32 = narrow(run.value)?;
        let cells = latest(snapshot, key.expect("a pool run's key"), origin, kind::POOL)?
            .ok_or_else(|| invalid("Resource pool event is missing"))?;
        runs.push(pool_run(&cells, run.span.first, run.span.last)?);
    }
    runs.sort_by_key(|run| run.first_hour);
    Ok(runs)
}
