//! Fuel's replies, read from the family's series publication ([`super::emit`]): each tag's
//! read and current consumption at an hour with its native country's consumption segment,
//! charts of its levels and days over a view, keyboard navigation over its reads, pages of its
//! reads, and the reads the detail products start from.
//!
//! A day measure's run for hour `h` is published at `h + 23`. A publication cut at `L` holds
//! the days through `L - 23`; the rest are read from the reads overlapping the last 23 hours,
//! each as a later read can still decide it by `L`, as the replies show a branch preparing.

use super::emit::{
    self, ACTOR_META, BOUNDS, CAPACITY, CONSUMPTION, DAY, DAY_MEASURES, DROPS, FUEL, LAG, READ,
    REASONS, SEGMENT, STRIPS, TAG_META, UNNAMED, day_values, kind, left_out_of, native_name,
};
use super::*;
use dashboard_base::{
    prepared::{
        coverage::Coverage,
        engine, published,
        series::{self, Navigation},
    },
    prepared_chart::{ChartBucket, ChartRun, ChartSummary},
    products::country_groups::HourSpan,
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, Evidence, Knownness, PageRead, Run, Selection, Snapshot, Span,
    Viewport,
    codec::Binary,
    family::{Directory, Scalar},
    query::{EventView, Tail, TailRun},
};
use serde_json::{Value, json};

/// The family's name: its sections and publication roots.
pub const NAME: &str = "fuel";

/// Rows one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 24;

fn invalid(message: &'static str) -> io::Error {
    dashboard_core::invalid(message)
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesPoint {
    pub tag: String,
    pub hour: i32,
    pub approximate: bool,
    pub recorded: bool,
    pub fuel: Option<FuelRead>,
    pub consumption: Option<demand::FuelConsumptionRead>,
    pub consumption_segment: Option<HourSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fuel_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capacity_raw: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesStateReply {
    pub branch_id: String,
    pub states: Vec<FuelSeriesPoint>,
    pub replies: Vec<FuelReply>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesLine {
    pub tag: String,
    pub measure: String,
    pub summary: ChartSummary,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesSummaryReply {
    pub branch_id: String,
    pub series: Vec<FuelSeriesLine>,
    pub replies: Vec<FuelReply>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<dashboard_base::prepared_measure::query::MeasureRead>,
    #[serde(default)]
    measures: Vec<String>,
    #[serde(default)]
    within: BTreeMap<String, Vec<HourSpan>>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    key: Option<series::MeasureKey>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
    cursor: Option<String>,
    page_size: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    kind: String,
    measure: Option<String>,
}

/// A measure a summary charts.
#[derive(Clone, Copy)]
enum Measure {
    /// Kept on the hour it describes.
    Hourly(u64),
    /// A day measure, dated [`LAG`] hours late.
    Day(usize),
}

fn measure(name: &str) -> Option<Measure> {
    Some(match name {
        "fuel" => Measure::Hourly(FUEL),
        "capacity" => Measure::Hourly(CAPACITY),
        "consumption-0" => Measure::Hourly(CONSUMPTION),
        "consumption-1" => Measure::Hourly(CONSUMPTION + 1),
        "consumption-2" => Measure::Hourly(CONSUMPTION + 2),
        _ => Measure::Day(DAY_MEASURES.iter().position(|day| *day == name)?),
    })
}

fn level(name: &str) -> bool {
    matches!(name, "fuel" | "capacity")
}

/// A reason a cell names, by its id.
fn reason(directory: &Directory, cell: Cell) -> io::Result<Arc<str>> {
    let Cell::Unsigned(id) = cell else {
        return Err(invalid("Prepared fuel reason is not valid"));
    };
    let text = directory
        .scalar(REASONS + id)
        .and_then(Scalar::text)
        .ok_or_else(|| invalid("Prepared fuel names an unknown reason"))?;
    Ok(Arc::from(String::from_utf8_lossy(text).as_ref()))
}

fn integer(cell: Cell) -> io::Result<i128> {
    match cell {
        Cell::Integer(value) => Ok(value),
        _ => Err(invalid("Prepared fuel read is not valid")),
    }
}

fn narrow<T: TryFrom<i128>>(cell: Cell) -> io::Result<T> {
    T::try_from(integer(cell)?).map_err(|_| invalid("Prepared fuel read is not valid"))
}

fn optional_hour(cell: Cell) -> io::Result<Option<i32>> {
    match cell {
        Cell::Null => Ok(None),
        cell => narrow(cell).map(Some),
    }
}

/// The read and consumption a [`kind::READ`] event holds, held from `first` to `last`.
fn decode<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    event: EventView,
    (first, last): (i32, i32),
) -> io::Result<(RawRead, demand::RawConsumption)> {
    let EventView::Family(event) = event else {
        return Err(invalid("Prepared fuel read is missing"));
    };
    if event.kind != kind::READ || event.native_hour != first {
        return Err(invalid("Prepared fuel supplying read is missing"));
    }
    let mut cells = [Cell::Null; 21];
    if event.fields.len != (cells.len() * Cell::SIZE) as u64 {
        return Err(invalid("Prepared fuel read is not valid"));
    }
    for (at, cell) in cells.iter_mut().enumerate() {
        *cell = snapshot.field(event.fields, at as u64)?;
    }
    let country = match cells[0] {
        Cell::Unsigned(country) => {
            u32::try_from(country).map_err(|_| invalid("Prepared fuel read is not valid"))?
        }
        _ => return Err(invalid("Prepared fuel read is not valid")),
    };
    let country_instance = match cells[1] {
        Cell::Null => None,
        Cell::Unsigned(instance) => {
            Some(u32::try_from(instance).map_err(|_| invalid("Prepared fuel read is not valid"))?)
        }
        _ => return Err(invalid("Prepared fuel read is not valid")),
    };
    let flag = |cell: Cell| match cell {
        Cell::Boolean(flag) => Ok(flag),
        _ => Err(invalid("Prepared fuel read is not valid")),
    };
    let left_out = match cells[7] {
        Cell::Null => None,
        Cell::Unsigned(code) => Some(
            left_out_of(code as usize).ok_or_else(|| invalid("Prepared fuel read is not valid"))?,
        ),
        _ => return Err(invalid("Prepared fuel read is not valid")),
    };
    let awards_reason = match cells[8] {
        Cell::Null => None,
        cell => Some(reason(directory, cell)?),
    };
    let day = match cells[9] {
        Cell::Null => None,
        Cell::List(list) => {
            let count = list.len / Cell::SIZE as u64;
            let mut values = Vec::with_capacity(count as usize);
            for at in 0..count {
                values.push(integer(snapshot.field(list, at)?)?);
            }
            let small = |value: i128| {
                i64::try_from(value).map_err(|_| invalid("Prepared fuel day is not valid"))
            };
            Some(match values.as_slice() {
                &[gained, burned, r0, r1, r2, other] => RawDay::Ring {
                    gained: small(gained)?,
                    burned: small(burned)?,
                    received: [small(r0)?, small(r1)?, small(r2)?],
                    other: small(other)?,
                },
                &[
                    g0,
                    g1,
                    g2,
                    g3,
                    own_gain,
                    own_gain_quantity,
                    t0,
                    t1,
                    t2,
                    t3,
                    consumed,
                    r0,
                    r1,
                    r2,
                ] => RawDay::Awards(Box::new(AwardSums {
                    gains: [g0, g1, g2, g3],
                    own_gain,
                    own_gain_quantity,
                    transfers: [t0, t1, t2, t3],
                    consumed,
                    received: [r0, r1, r2],
                })),
                _ => return Err(invalid("Prepared fuel day is not valid")),
            })
        }
        _ => return Err(invalid("Prepared fuel day is not valid")),
    };
    let mut native_daily: [Result<i64, Arc<str>>; 3] = std::array::from_fn(|_| Err(Arc::from("")));
    for (family, daily) in native_daily.iter_mut().enumerate() {
        *daily = match cells[12 + family] {
            Cell::Integer(value) => Ok(i64::try_from(value)
                .map_err(|_| invalid("Prepared fuel consumption is not valid"))?),
            cell => Err(reason(directory, cell)?),
        };
    }
    let read = RawRead {
        country,
        country_instance,
        continuity_start_hour: optional_hour(cells[2])?,
        first_hour: first,
        last_hour: last,
        starts_segment: flag(cells[3])?,
        fuel: narrow(cells[4])?,
        capacity: narrow(cells[5])?,
        lend_lease: narrow(cells[6])?,
        day,
        left_out,
        awards_reason,
    };
    let consumption = demand::RawConsumption {
        first_hour: first,
        last_hour: last,
        starts_segment: flag(cells[10])?,
        segment_start_hour: narrow(cells[11])?,
        country,
        country_instance,
        native_daily,
        known_daily: [
            integer(cells[15])?,
            integer(cells[16])?,
            integer(cells[17])?,
        ],
        positive_daily: [
            integer(cells[18])?,
            integer(cells[19])?,
            integer(cells[20])?,
        ],
    };
    Ok((read, consumption))
}

/// Whether a slot read through `last` overlaps the kept day of `actor`'s read at `first`.
fn dropped<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    first: i32,
    last: i32,
) -> io::Result<bool> {
    let Some(key) = directory.key(actor, DROPS, None) else {
        return Ok(false);
    };
    let end = first.saturating_add(LAG).min(last);
    if end <= first {
        return Ok(false);
    }
    let selection = [Selection {
        key,
        membership: None,
    }];
    let query = EventQuery {
        selection: &selection,
        native: Span::new(first + 1, end)?,
        as_of: None,
        order: EventOrder::Native,
    };
    let mut drops = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.event_page(query, cursor, BUDGET, |event| {
            if let EventView::Family(event) = event
                && event.kind == kind::DROP
            {
                drops.push(event.fields);
            }
            Ok(())
        })?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    for fields in drops {
        if integer(snapshot.field(fields, 0)?)? == i128::from(first) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The read of `actor` whose run `run` is, as of the publication's last hour `last`.
fn read_of_run<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    run: Run,
    last: i32,
) -> io::Result<(RawRead, demand::RawConsumption)> {
    let key = directory
        .key(actor, READ, None)
        .ok_or_else(|| invalid("Prepared fuel read is missing"))?;
    let first = i32::try_from(run.value).map_err(|_| invalid("Prepared fuel read is not valid"))?;
    let event = snapshot
        .latest_event(
            Selection {
                key,
                membership: None,
            },
            first,
            None,
        )?
        .ok_or_else(|| invalid("Prepared fuel supplying read is missing"))?;
    let (mut read, consumption) = decode(snapshot, directory, event, (first, run.span.last))?;
    if read.day.is_some() && dropped(snapshot, directory, actor, first, last)? {
        read.day = None;
        read.left_out = Some(DayLeftOut::Overlaps);
    }
    Ok((read, consumption))
}

/// `actor`'s read holding `hour`.
pub(super) fn read_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    hour: i32,
) -> io::Result<Option<(RawRead, demand::RawConsumption)>> {
    let Some(key) = directory.key(actor, READ, None) else {
        return Ok(None);
    };
    let Some((run, _)) = engine::run_at(snapshot, key, hour)? else {
        return Ok(None);
    };
    let last = snapshot.publication().accepted.last;
    read_of_run(snapshot, directory, actor, run, last).map(Some)
}

/// `actor`'s reads holding any hour of `first..=last`, in hour order.
fn reads_within<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    (first, last): (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<Vec<(RawRead, demand::RawConsumption)>> {
    let Some(key) = directory.key(actor, READ, None) else {
        return Ok(Vec::new());
    };
    let published = snapshot.publication().accepted.last;
    let mut reads = Vec::new();
    for clipped in engine::runs(snapshot, key, first, last)? {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (run, _) = engine::run_at(snapshot, key, clipped.span.first)?
            .ok_or_else(|| invalid("Prepared fuel read is missing"))?;
        reads.push(read_of_run(snapshot, directory, actor, run, published)?);
    }
    Ok(reads)
}

/// Whether `actor`'s tag ever had a read, and its days left out by reason.
pub(super) fn tag_meta(directory: &Directory, actor: Option<u64>) -> (bool, [usize; 3]) {
    let Some(meta) = actor.and_then(|actor| directory.scalar(TAG_META + actor)) else {
        return (false, [0; 3]);
    };
    match meta.list() {
        [Scalar::Boolean(read), counts @ ..] if counts.len() == 3 => {
            let count = |at: usize| {
                counts[at]
                    .integer()
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or(0)
            };
            (*read, [count(0), count(1), count(2)])
        }
        _ => (false, [0; 3]),
    }
}

/// A native country's conflict flag and the tags that held it.
fn native_meta(directory: &Directory, actor: u64) -> Option<(bool, Vec<String>)> {
    let meta = directory.scalar(ACTOR_META + actor)?;
    let [_, Scalar::Boolean(conflict), tags] = meta.list() else {
        return None;
    };
    let tags = tags
        .list()
        .iter()
        .filter_map(|tag| {
            let actor = usize::try_from(tag.integer()?).ok()?;
            Some(String::from_utf8_lossy(&directory.actors.get(actor)?.name).into_owned())
        })
        .collect();
    Some((*conflict, tags))
}

/// The consumption segment of native country `country` holding `hour`; none while several
/// tags held it at once.
pub(super) fn segment_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    country: u32,
    hour: i32,
) -> io::Result<Option<HourSpan>> {
    let Some(actor) = directory.actor(&native_name(country)) else {
        return Ok(None);
    };
    if native_meta(directory, actor).is_some_and(|(conflict, _)| conflict) {
        return Ok(None);
    }
    let Some(key) = directory.key(actor, SEGMENT, None) else {
        return Ok(None);
    };
    Ok(
        engine::run_at(snapshot, key, hour)?.map(|(run, _)| HourSpan {
            first_hour: run.span.first,
            last_hour: run.span.last,
        }),
    )
}

pub(super) fn unnamed(directory: &Directory) -> BTreeSet<u32> {
    directory
        .scalar(UNNAMED)
        .map_or(&[][..], Scalar::list)
        .iter()
        .filter_map(|id| u32::try_from(id.integer()?).ok())
        .collect()
}

/// The first and last hours walked.
fn bounds(directory: &Directory) -> (i32, i32) {
    let held = directory.scalar(BOUNDS).map_or(&[][..], Scalar::list);
    let hour = |at: usize| {
        held.get(at)
            .and_then(Scalar::integer)
            .and_then(|hour| i32::try_from(hour).ok())
            .unwrap_or(0)
    };
    (hour(0), hour(1))
}

fn presentation(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    tag: &str,
    directory: &Directory,
) -> io::Result<FuelReply> {
    let (read, left_out) = tag_meta(directory, directory.actor(tag.as_bytes()));
    super::reply(
        source,
        &CountryParams {
            branch_id: branch.to_string(),
            tag: tag.into(),
        },
        context,
        &unnamed(directory),
        Counted { read, left_out },
    )
}

/// `spans`, each `by` hours later.
fn moved(spans: &[HourSpan], by: i32) -> io::Result<Vec<HourSpan>> {
    spans
        .iter()
        .map(|span| {
            Ok(HourSpan {
                first_hour: span
                    .first_hour
                    .checked_add(by)
                    .ok_or_else(|| invalid("Fuel hours overflow"))?,
                last_hour: span
                    .last_hour
                    .checked_add(by)
                    .ok_or_else(|| invalid("Fuel hours overflow"))?,
            })
        })
        .collect()
}

fn later(hour: i32) -> io::Result<i32> {
    hour.checked_add(LAG)
        .ok_or_else(|| invalid("Fuel hours overflow"))
}

/// `summary`, read on the day keys' dates, on the hours it describes.
fn described(summary: &mut ChartSummary, (from, to): (f64, f64)) {
    summary.from = from;
    summary.to = to;
    for bucket in summary.buckets.iter_mut().flatten() {
        let ChartBucket {
            first_hour,
            last_hour,
            last_value_hour,
            min_hour,
            max_hour,
            ..
        } = bucket;
        for hour in [first_hour, last_hour, last_value_hour, min_hour, max_hour] {
            *hour -= LAG;
        }
    }
    for run in summary.runs.iter_mut().flatten() {
        run.first_hour -= LAG;
        run.last_hour -= LAG;
    }
}

/// The runs of day measure `measure` of `actor`'s reads over the publication's last 23 hours,
/// on the hours they describe.
pub(super) fn day_tail<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    measure: usize,
    cancelled: &AtomicBool,
) -> io::Result<Vec<series::Run>> {
    let last = snapshot.publication().accepted.last;
    let first = last - LAG + 1;
    let mut tail = Vec::new();
    for (read, _) in reads_within(snapshot, directory, actor, (first, last), cancelled)? {
        let Some(value) = read.day.as_ref().and_then(|day| day_values(day)[measure]) else {
            continue;
        };
        if measure < STRIPS {
            if read.first_hour >= first {
                tail.push(series::Run::new(
                    read.first_hour,
                    read.first_hour,
                    value,
                    read.starts_segment,
                ));
            }
            continue;
        }
        let from = read.first_hour.max(first);
        tail.push(series::Run::new(
            from,
            read.last_hour,
            value,
            read.starts_segment && from == read.first_hour,
        ));
    }
    Ok(tail)
}

fn continues(previous: series::Run, next: series::Run) -> bool {
    previous.last_hour.checked_add(1) == Some(next.first_hour)
        && previous.value == next.value
        && !next.break_before
}

fn runs_within(runs: &[ChartRun], spans: &[HourSpan]) -> Vec<ChartRun> {
    runs.iter()
        .flat_map(|run| {
            let start = spans.partition_point(|span| span.last_hour < run.first_hour);
            spans[start..]
                .iter()
                .take_while(|span| span.first_hour <= run.last_hour)
                .filter_map(move |span| {
                    let first = run.first_hour.max(span.first_hour);
                    let last = run.last_hour.min(span.last_hour);
                    (first <= last).then_some(ChartRun {
                        first_hour: first,
                        last_hour: last,
                        break_before: run.break_before || first != run.first_hour,
                        ..run.clone()
                    })
                })
        })
        .collect()
}

/// Day measure `measure`'s chart of `actor` over `view`, its bins laid from `bounds`, clipped to
/// `within`: the published days, then the last 23 hours' from their reads.
#[allow(clippy::too_many_arguments)]
pub(super) fn day_summary<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: Option<u64>,
    measure: usize,
    bounds: (i32, i32),
    (from, to, width): (f64, f64, u32),
    within: Option<&[HourSpan]>,
    cancelled: &AtomicBool,
) -> io::Result<ChartSummary> {
    let key = actor.and_then(|actor| directory.key(actor, DAY + measure as u64, None));
    let tail = match (actor, key) {
        (Some(actor), Some(_)) => day_tail(snapshot, directory, actor, measure, cancelled)?,
        _ => Vec::new(),
    };
    let key = key.unwrap_or(u64::MAX);
    let dated = (later(bounds.0)?, later(bounds.1)?);
    let view = (
        from.floor() + f64::from(LAG),
        to.floor() + f64::from(LAG),
        width,
    );
    let membership = within.map(|spans| moved(spans, LAG)).transpose()?;
    if tail.is_empty() {
        let mut summary = engine::summary(snapshot, key, dated, view, 0, membership.as_deref())?;
        described(&mut summary, (from, to));
        return Ok(summary);
    }
    let spans = membership.as_deref().map(engine::spans).transpose()?;
    let selection = Selection {
        key,
        membership: spans.as_deref(),
    };
    let tail_runs = tail
        .iter()
        .map(|run| {
            let span = Span::new(later(run.first_hour)?, later(run.last_hour)?)?;
            Ok(TailRun {
                run: Run {
                    span,
                    value: run.value,
                    break_before: run.break_before,
                    supplying_boundary: false,
                    canonical: 0,
                    sequence: 0,
                },
                evidence: Evidence {
                    span,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let grid = Span::new(dated.0, dated.1)?;
    let viewport = Viewport {
        from: view.0,
        to: view.1,
        width,
    };
    let mut buckets = Vec::new();
    let mut cursor = None;
    let hourly = loop {
        let page = snapshot.summary_tiles(
            &[selection],
            grid,
            viewport,
            Tail {
                exact: &tail_runs,
                approximate: &[],
            },
            cursor,
            BUDGET,
            |_, bin| {
                buckets.push(bin.exact.map(engine::bucket).transpose()?);
                Ok(())
            },
        )?;
        cursor = page.page.next;
        if cursor.is_none() {
            break page.hourly;
        }
    };
    let mut summary = ChartSummary {
        from,
        to,
        width,
        buckets,
        runs: None,
        approximate: Vec::new(),
    };
    if hourly {
        // The original runs, as the published days give them and then the tail continues them.
        let whole = Selection {
            key,
            membership: None,
        };
        let mut runs = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot.hourly_runs(whole, grid, viewport, cursor, BUDGET, |run| {
                runs.push(ChartRun {
                    first_hour: run.span.first,
                    last_hour: run.span.last,
                    value: run.value as f64,
                    break_before: run.break_before,
                });
                Ok(())
            })?;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        summary.runs = Some(runs);
    }
    described(&mut summary, (from, to));
    if let Some(runs) = &mut summary.runs {
        let mut canonical = Vec::<series::Run>::new();
        for &run in &tail {
            if let Some(previous) = canonical.last_mut()
                && continues(*previous, run)
            {
                previous.last_hour = run.last_hour;
            } else {
                canonical.push(run);
            }
        }
        if let Some(head) = canonical.first_mut()
            && let Some((run, _)) = engine::run_at(snapshot, key, later(head.first_hour - 1)?)?
        {
            let previous = series::Run::new(
                run.span.first - LAG,
                head.first_hour - 1,
                run.value,
                run.break_before,
            );
            if continues(previous, *head) {
                head.first_hour = previous.first_hour;
                head.break_before = previous.break_before;
            }
        }
        for run in canonical {
            if let Some(previous) = runs
                .last_mut()
                .filter(|previous| previous.first_hour == run.first_hour)
            {
                previous.last_hour = run.last_hour;
            } else {
                runs.push(ChartRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    value: run.value as f64,
                    break_before: run.break_before,
                });
            }
        }
        let first = from.floor().max(f64::from(bounds.0));
        let last = to.floor().min(f64::from(bounds.1));
        runs.retain(|run| {
            first <= last && f64::from(run.last_hour) >= first && f64::from(run.first_hour) <= last
        });
        if let Some(spans) = within {
            *runs = runs_within(runs, spans);
        }
    }
    Ok(summary)
}

/// Answers a `country-fuel` series request from the family's publication; null before it
/// publishes the branch, and where a reply needs the branch prepared whole.
pub fn query(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    product: &str,
    params: Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(params)?;
    if params.family.kind != "country-fuel" {
        return Err(invalid("Unknown fuel series family"));
    }
    let branch = params.branch_id.parse::<u64>().map_err(io::Error::other)?;
    let accepted = match dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "country-fuel",
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
        result => result?,
    };
    let partial = accepted.as_ref().is_none_or(|coverage| !coverage.complete);
    let measures = if params.measures.is_empty() {
        vec![
            params
                .family
                .measure
                .clone()
                .unwrap_or_else(|| "fuel".into()),
        ]
    } else {
        params.measures.clone()
    };
    if partial
        && (matches!(
            product,
            "events-page" | "events-clusters" | "series-next-change"
        ) || product == "series-summary" && !measures.iter().any(|field| level(field)))
    {
        return Ok(Value::Null);
    }
    let Some(opened) = published::open(source, content, NAME, emit::SCHEMA, branch)? else {
        return Ok(Value::Null);
    };
    let context = LoadContext {
        content: content.cloned(),
    };
    let mut reply = opened.read(cancelled, |snapshot, directory| {
        answer(
            source,
            &context,
            product,
            params,
            &measures,
            (partial, accepted.as_ref()),
            snapshot,
            directory,
            cancelled,
        )
    })?;
    if partial && product == "series-summary" {
        reply["pendingMeasures"] = json!(
            measures
                .iter()
                .filter(|field| !level(field))
                .collect::<Vec<_>>()
        );
    }
    reply["coverage"] = json!(dashboard_base::prepared::prefix::extent(
        source, branch, accepted
    )?);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

#[allow(clippy::too_many_arguments)]
fn answer<P: PageRead>(
    source: &Source,
    context: &LoadContext,
    product: &str,
    params: Params,
    measures: &[String],
    (partial, accepted): (bool, Option<&Coverage>),
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let branch = params.branch_id.parse::<u64>().map_err(io::Error::other)?;
    let last = accepted.map(|coverage| coverage.last_hour);
    let tags: BTreeSet<_> = params
        .tags
        .iter()
        .cloned()
        .chain(params.reads.iter().map(|read| read.tag.clone()))
        .collect();
    let replies = tags
        .iter()
        .map(|tag| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
            presentation(source, context, branch, tag, directory)
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(match product {
        "series-state" => {
            let mut states = Vec::with_capacity(params.reads.len());
            for read in params.reads {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let actor = directory.actor(read.tag.as_bytes());
                let prepared = !partial || last.is_some_and(|last| read.hour <= last);
                let held = match actor.filter(|_| prepared) {
                    Some(actor) => read_at(snapshot, directory, actor, read.hour)?,
                    None => None,
                };
                let consumption_segment = match &held {
                    Some((_, consumption)) => {
                        segment_at(snapshot, directory, consumption.country, read.hour)?
                    }
                    None => None,
                };
                states.push(FuelSeriesPoint {
                    approximate: false,
                    recorded: tag_meta(directory, actor).0,
                    fuel: held.as_ref().map(|(read, _)| FuelRead::from(read)),
                    consumption: held
                        .as_ref()
                        .map(|(_, consumption)| demand::FuelConsumptionRead::from(consumption)),
                    consumption_segment,
                    fuel_raw: None,
                    capacity_raw: None,
                    tag: read.tag,
                    hour: read.hour,
                });
            }
            serde_json::to_value(FuelSeriesStateReply {
                branch_id: params.branch_id,
                states,
                replies,
            })?
        }
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(
                from,
                to,
                width,
                params
                    .tags
                    .len()
                    .saturating_mul(params.measures.len().max(1)),
            )?;
            let mut kinds = Vec::with_capacity(measures.len());
            for name in measures {
                kinds.push(measure(name).ok_or_else(|| {
                    dashboard_core::invalid(format!("Unknown fuel series measure: {name}"))
                })?);
            }
            let published = bounds(directory);
            let branch_bounds = || {
                let start = source
                    .replay_ref()
                    .branches
                    .iter()
                    .find(|start| start.id == branch)
                    .ok_or_else(|| invalid("Unknown fuel branch"))?;
                Ok::<_, io::Error>(dashboard_text::metadata::branch_bounds(
                    source.replay_ref(),
                    start,
                ))
            };
            let mut lines = Vec::new();
            for tag in &params.tags {
                let actor = directory.actor(tag.as_bytes());
                let within = params.within.get(tag).map(Vec::as_slice);
                for (name, kind) in measures.iter().zip(&kinds) {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let summary = match *kind {
                        Measure::Hourly(measure) => {
                            let key = actor
                                .and_then(|actor| directory.key(actor, measure, None))
                                .unwrap_or(u64::MAX);
                            let bounds = if partial && level(name) {
                                branch_bounds()?
                            } else {
                                published
                            };
                            engine::summary(snapshot, key, bounds, (from, to, width), 0, within)?
                        }
                        Measure::Day(measure) => day_summary(
                            snapshot,
                            directory,
                            actor,
                            measure,
                            published,
                            (from, to, width),
                            within,
                            cancelled,
                        )?,
                    };
                    lines.push(FuelSeriesLine {
                        tag: tag.clone(),
                        measure: name.clone(),
                        summary,
                    });
                }
            }
            serde_json::to_value(FuelSeriesSummaryReply {
                branch_id: params.branch_id,
                series: lines,
                replies,
            })?
        }
        "series-next-change" => {
            let navigation = Navigation {
                from: params.from,
                first_hour: params
                    .first_hour
                    .ok_or_else(|| invalid("Missing fuel navigation range"))?,
                last_hour: params
                    .last_hour
                    .ok_or_else(|| invalid("Missing fuel navigation range"))?,
                key: params
                    .key
                    .ok_or_else(|| invalid("Missing fuel navigation direction"))?,
            };
            let keys: Vec<(u64, Option<&[HourSpan]>)> = params
                .tags
                .iter()
                .filter_map(|tag| {
                    let actor = directory.actor(tag.as_bytes())?;
                    let key = directory.key(actor, READ, None)?;
                    Some((key, params.within.get(tag).map(Vec::as_slice)))
                })
                .collect();
            json!({ "hour": engine::navigate(snapshot, &keys, &navigation)? })
        }
        "events-page" => events_page(snapshot, directory, &params, cancelled)?,
        _ => return Err(invalid("Unknown fuel series query")),
    })
}

/// A page of the requested tags' reads (or their consumption), each tag's within its
/// `within`, in hour order.
fn events_page<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &Params,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let (first, last, cursor, page_size) = series::event_window(
        params.from,
        params.to,
        params.cursor.as_deref(),
        params.page_size,
    )?;
    let consumption = params.family.measure.as_deref() == Some("consumption");
    let mut keyed = Vec::new();
    for tag in &params.tags {
        let Some(actor) = directory.actor(tag.as_bytes()) else {
            continue;
        };
        let Some(key) = directory.key(actor, READ, None) else {
            continue;
        };
        let spans = params.within.get(tag).cloned().unwrap_or_else(|| {
            vec![HourSpan {
                first_hour: first,
                last_hour: last,
            }]
        });
        let spans = dashboard_base::ranking::joined_spans(spans, first, last);
        keyed.push((key, actor, tag.clone(), engine::spans(&spans)?));
    }
    let selections: Vec<Selection<'_>> = keyed
        .iter()
        .map(|(key, _, _, spans)| Selection {
            key: *key,
            membership: Some(spans),
        })
        .collect();
    let mut rows = Vec::new();
    let mut total = 0;
    if first <= last && !selections.is_empty() {
        let query = EventQuery {
            selection: &selections,
            native: Span::new(first, last)?,
            as_of: None,
            order: EventOrder::Native,
        };
        total = snapshot.event_count(query)?;
        let mut events = Vec::new();
        let mut position = 0_u64;
        let mut page = None;
        let end = cursor.saturating_add(u64::from(page_size)).min(total);
        while position < end {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let result = snapshot.event_page(query, page, BUDGET, |event| {
                if position >= cursor && position < end {
                    events.push(event);
                }
                position += 1;
                Ok(())
            })?;
            page = result.next;
            if page.is_none() {
                break;
            }
        }
        for (index, event) in events.into_iter().enumerate() {
            let (_, actor, tag, _) = keyed
                .iter()
                .find(|(key, ..)| *key == event.key())
                .ok_or_else(|| invalid("Prepared fuel event of no requested tag"))?;
            let hour = event.native_hour();
            let (run, _) = engine::run_at(snapshot, event.key(), hour)?
                .ok_or_else(|| invalid("Prepared fuel read is missing"))?;
            let published = snapshot.publication().accepted.last;
            let (read, demand) = read_of_run(snapshot, directory, *actor, run, published)?;
            let row = if consumption {
                serde_json::to_value(demand::FuelConsumptionRead::from(&demand))?
            } else {
                serde_json::to_value(FuelRead::from(&read))?
            };
            rows.push(json!({
                "tag": tag, "hour": hour, "ordinal": (cursor + index as u64).to_string(), "row": row,
            }));
        }
    }
    let end = cursor.saturating_add(rows.len() as u64);
    Ok(json!({
        "firstOrdinal": cursor.to_string(), "total": total.to_string(), "rows": rows,
        "cursor": (end < total).then(|| end.to_string()), "warnings": [],
    }))
}

/// In-memory fixtures keep the registered whole-branch scan as the oracle.
#[cfg(any(test, feature = "test-support"))]
fn reference_fixture(source: &Source) -> bool {
    source.path().is_relative()
}

/// The consumption read supplied at `basis`'s hour, of its native country identity.
pub(super) fn summary_at(
    source: &Source,
    basis: &FuelAwardsParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<demand::FuelConsumptionRead> {
    let branch = basis.branch_id.parse::<u64>().map_err(io::Error::other)?;
    #[cfg(any(test, feature = "test-support"))]
    if reference_fixture(source) {
        return Ok(demand::FuelConsumptionRead::from(demand::summary_at(
            &*scanned(source, branch, cancelled)?,
            basis,
        )?));
    }
    let opened = published::open(source, context.content.as_ref(), NAME, emit::SCHEMA, branch)?
        .ok_or_else(|| invalid("Fuel summary preparation is not available"))?;
    let read = opened.read(cancelled, |snapshot, directory| {
        match directory.actor(basis.tag.as_bytes()) {
            Some(actor) => read_at(snapshot, directory, actor, basis.hour),
            None => Ok(None),
        }
    })?;
    read.map(|(_, consumption)| demand::FuelConsumptionRead::from(&consumption))
        .filter(|read| {
            read.first_hour == basis.hour
                && read.country == basis.country
                && read.country_instance == Some(basis.country_instance)
        })
        .ok_or_else(|| invalid("Current fuel consumption actor or supplying read was not recorded"))
}

/// What the detail products read of a native country over a window: the tags that held it,
/// whether several held it at once, and those tags' reads holding the window's hours (at
/// least).
pub(super) struct ActorWindow {
    pub aliases: Vec<String>,
    pub conflict: bool,
    pub reads: Vec<RawRead>,
}

/// `country`'s window `window` on `branch` ([`ActorWindow`]).
pub(super) fn actor_window(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    country: &str,
    window: i32,
    cancelled: &AtomicBool,
) -> io::Result<ActorWindow> {
    let (first, last) = dashboard_base::products::production_lines::detail_window(window)
        .ok_or_else(|| invalid("Invalid fuel detail window"))?;
    #[cfg(any(test, feature = "test-support"))]
    if reference_fixture(source) {
        let summary = scanned(source, branch, cancelled)?;
        let aliases: Vec<String> = summary.actor_tags(country).cloned().collect();
        let reads = aliases
            .iter()
            .flat_map(|tag| summary.countries[tag].reads.iter().cloned())
            .collect();
        return Ok(ActorWindow {
            conflict: summary.actor_conflicts.contains(country),
            aliases,
            reads,
        });
    }
    let opened = published::open(source, context.content.as_ref(), NAME, emit::SCHEMA, branch)?
        .ok_or_else(|| invalid("Fuel summary preparation is not available"))?;
    let country: u32 = country
        .parse()
        .map_err(|_| invalid("Fuel country reference invalid"))?;
    opened.read(cancelled, |snapshot, directory| {
        actor_window_in(snapshot, directory, country, (first, last), cancelled)
    })
}

/// [`actor_window`] of native country `country` over `first..=last` in a publication.
pub(super) fn actor_window_in<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    country: u32,
    (first, last): (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<ActorWindow> {
    let Some((conflict, aliases)) = directory
        .actor(&native_name(country))
        .and_then(|actor| native_meta(directory, actor))
    else {
        return Ok(ActorWindow {
            aliases: Vec::new(),
            conflict: false,
            reads: Vec::new(),
        });
    };
    let mut reads = Vec::new();
    for tag in &aliases {
        let Some(actor) = directory.actor(tag.as_bytes()) else {
            continue;
        };
        for (read, _) in reads_within(snapshot, directory, actor, (first, last), cancelled)? {
            reads.push(read);
        }
    }
    Ok(ActorWindow {
        aliases,
        conflict,
        reads,
    })
}

/// The consumption reads of `basis`'s native country over `window` in the segment holding
/// its hour; none while several tags held it at once.
pub(super) fn actor_consumption(
    source: &Source,
    basis: &FuelAwardsParams,
    window: i32,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Vec<(String, demand::FuelConsumptionRead)>> {
    let branch = basis.branch_id.parse::<u64>().map_err(io::Error::other)?;
    #[cfg(any(test, feature = "test-support"))]
    if reference_fixture(source) {
        let summary = scanned(source, branch, cancelled)?;
        return Ok(demand::actor_consumption(&summary, basis)
            .into_iter()
            .map(|(tag, read)| (tag.to_owned(), demand::FuelConsumptionRead::from(read)))
            .collect());
    }
    let (first, last) = dashboard_base::products::production_lines::detail_window(window)
        .ok_or_else(|| invalid("Invalid current fuel consumption window"))?;
    let opened = published::open(source, context.content.as_ref(), NAME, emit::SCHEMA, branch)?
        .ok_or_else(|| invalid("Fuel summary preparation is not available"))?;
    let country: u32 = basis
        .country
        .parse()
        .map_err(|_| invalid("Fuel country reference invalid"))?;
    opened.read(cancelled, |snapshot, directory| {
        actor_consumption_in(
            snapshot,
            directory,
            country,
            basis.hour,
            (first, last),
            cancelled,
        )
    })
}

/// [`actor_consumption`] of native country `country` at `hour` over `first..=last` in a
/// publication.
pub(super) fn actor_consumption_in<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    country: u32,
    hour: i32,
    (first, last): (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<Vec<(String, demand::FuelConsumptionRead)>> {
    let Some(native) = directory.actor(&native_name(country)) else {
        return Ok(Vec::new());
    };
    let Some((false, aliases)) = native_meta(directory, native) else {
        return Ok(Vec::new());
    };
    let Some(key) = directory.key(native, SEGMENT, None) else {
        return Ok(Vec::new());
    };
    let Some((segment, _)) = engine::run_at(snapshot, key, hour)? else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for tag in aliases {
        let Some(actor) = directory.actor(tag.as_bytes()) else {
            continue;
        };
        for (_, consumption) in reads_within(snapshot, directory, actor, (first, last), cancelled)?
        {
            if consumption.country == country
                && engine::run_at(snapshot, key, consumption.first_hour)?
                    .is_some_and(|(held, _)| held.value == segment.value)
            {
                result.push((tag.clone(), demand::FuelConsumptionRead::from(&consumption)));
            }
        }
    }
    result.sort_by_key(|(_, read)| read.first_hour);
    Ok(result)
}
