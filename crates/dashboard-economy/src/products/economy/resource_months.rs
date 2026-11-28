use dashboard_base::products::resources::{ResourceInfo, ResourceNaming};
use dashboard_base::products::{Country, LoadContext, Product, validate_country};
use dashboard_base::Source;
use dashboard_query::{
    Query,
    chart::ChartRun,
    series::{Ask, Line, MissingSpan},
};
use game_model::calendar::{HOUR_EPOCH, MONTH_LENGTHS};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const DAY: i32 = 24;
const YEAR: i32 = 365 * DAY;
pub(crate) const NOT_RECORDED: &str = "Resource balance was not recorded in this month.";
pub(crate) const NOT_HELD: &str = "The selected country did not hold this tag at this hour.";

pub(crate) fn month_of(hour: i32) -> i32 {
    let elapsed = hour - HOUR_EPOCH;
    let year = elapsed.div_euclid(YEAR);
    let mut day = elapsed.rem_euclid(YEAR) / DAY;
    let mut month = 0;
    for length in MONTH_LENGTHS {
        if day < length {
            break;
        }
        day -= length;
        month += 1;
    }
    year * 12 + month
}

pub(crate) fn month_first(month: i32) -> i32 {
    let (year, index) = (month.div_euclid(12), month.rem_euclid(12) as usize);
    HOUR_EPOCH + year * YEAR + MONTH_LENGTHS[..index].iter().sum::<i32>() * DAY
}

fn day_start(hour: i32) -> i32 {
    hour - (hour - HOUR_EPOCH).rem_euclid(DAY)
}

fn value_at(runs: &[ChartRun], hour: i32) -> Option<f64> {
    runs.get(runs.partition_point(|run| run.last_hour < hour)).filter(|run| run.first_hour <= hour).map(|run| run.value)
}

fn reason_at(missing: &[MissingSpan], hour: i32) -> Option<&str> {
    missing.iter().find(|span| span.first_hour <= hour && hour <= span.last_hour).map(|span| span.reason.as_str())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResourceMonth {
    pub first_hour: i32,
    #[doc = " The month's last hour, or the asked hour in the month that holds it."]
    pub last_hour: i32,
    #[doc = " The mean net per day over the month's days whose start is recorded; null when none is."]
    pub net: Option<f64>,
    #[doc = " Days of the month, through the asked hour, whose start is recorded, and all of them."]
    pub recorded_days: u32,
    pub days: u32,
    #[doc = " Why the month has no value."]
    pub missing: Option<String>,
}

pub(crate) fn months(runs: &[ChartRun], missing: &[MissingSpan], from: i32, hour: i32) -> Vec<ResourceMonth> {
    let mut out = Vec::new();
    let mut month = month_of(from);
    while month_first(month) <= hour {
        let (first, last) = (month_first(month), (month_first(month + 1) - 1).min(hour));
        let (mut sum, mut recorded, mut days, mut why) = (0.0, 0u32, 0u32, None);
        let mut day = day_start(first.max(from)).max(first);
        while day <= last {
            if day >= from {
                days += 1;
                match value_at(runs, day) {
                    Some(value) => {
                        sum += value;
                        recorded += 1;
                    }
                    None => why = why.or_else(|| reason_at(missing, day)),
                }
            }
            day += DAY;
        }
        out.push(ResourceMonth {
            first_hour: first.max(from),
            last_hour: last,
            net: (recorded > 0).then(|| sum / f64::from(recorded)),
            recorded_days: recorded,
            days,
            missing: (recorded == 0).then(|| why.unwrap_or(NOT_RECORDED).to_owned()),
        });
        month += 1;
    }
    out
}

pub(crate) fn next_change(runs: &[ChartRun], hour: i32) -> Option<i32> {
    let at = runs.partition_point(|run| run.last_hour < hour);
    match runs.get(at) {
        Some(run) if run.first_hour <= hour => Some(run.last_hour + 1),
        Some(run) => Some(run.first_hour),
        None => None,
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceMonthsParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResourceMonthRow {
    pub resource: ResourceInfo,
    #[doc = " The net per day at the asked hour; null with `missing` when it is not known."]
    pub net: Option<f64>,
    pub missing: Option<String>,
    pub months: Vec<ResourceMonth>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResourceMonthsReply {
    pub tag: String,
    pub hour: i32,
    #[doc = " Every recorded resource, in the game's order."]
    pub resources: Vec<ResourceMonthRow>,
    #[doc = " The first later hour at which any value shown changes; null when none does."]
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ResourceMonthsReply);

struct Held {
    infos: Vec<ResourceInfo>,
    lines: Vec<ResourceHistory>,
    warnings: Vec<String>,
}

struct ResourceHistory {
    runs: Vec<ChartRun>,
    missing: Vec<MissingSpan>,
    months: Vec<ResourceMonth>,
}

impl ResourceHistory {
    fn new(runs: Vec<ChartRun>, missing: Vec<MissingSpan>, from: i32, to: i32) -> Self {
        let months = months(&runs, &missing, from, to);
        Self { runs, missing, months }
    }

    fn through(&self, hour: i32) -> Vec<ResourceMonth> {
        let closed = self.months.partition_point(|month| month.last_hour <= hour);
        let mut out = self.months[..closed].to_vec();
        if let Some(open) = self.months.get(closed) {
            out.extend(months(&self.runs, &self.missing, open.first_hour, hour));
        }
        out
    }
}

#[derive(Default)]
struct Kept {
    held: HashMap<String, Arc<Held>>,
}

fn held(source: &Source, query: &Query<'_>, country: &Country, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<Held>> {
    let key = serde_json::to_string(&(query.timeline.id, query.timeline.last, country)).map_err(io::Error::other)?;
    let state = source.state::<Kept>();
    if let Some(found) = state.lock().unwrap_or_else(PoisonError::into_inner).held.get(&key) {
        return Ok(Arc::clone(found));
    }
    let series = dashboard_declare::series("resource-balance").ok_or_else(|| dashboard_core::invalid("Resource balance is not declared"))?;
    let measure = series.measure("net").ok_or_else(|| dashboard_core::invalid("Resource balance has no net"))?;
    let parts = series.parts.ok_or_else(|| dashboard_core::invalid("Resource balance has no parts"))?;
    let mut warnings = Vec::new();
    let tokens = crate::products::convoys::resource_tokens(source, query, None, &mut warnings)?;
    let (infos, notes) = ResourceNaming::stored(source, load).infos(&tokens, cancelled)?;
    warnings.extend(notes);
    let (from, to) = (query.timeline.first, query.timeline.last);
    let line = Line::new(query, series, measure)?;
    let asks: Vec<(Ask, Vec<(i32, i32)>)> = infos.iter().map(|info| (Ask { country: country.clone(), part: parts.key(0, u128::from(info.id)) }, vec![(from, to)])).collect();
    let width = u32::try_from(i64::from(to) - i64::from(from) + 1).map_err(|_| dashboard_core::invalid("The branch is too long"))?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let lines = line.summaries_within(&asks, from, to, width)?.into_iter()
        .map(|(summary, missing)| ResourceHistory::new(summary.runs.unwrap_or_default(), missing, from, to))
        .collect();
    let made = Arc::new(Held { infos, lines, warnings });
    state.lock().unwrap_or_else(PoisonError::into_inner).held.insert(key, Arc::clone(&made));
    Ok(made)
}

pub struct ResourceMonths;

impl Product for ResourceMonths {
    const KEY: &'static str = "resource-balance-months";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ResourceMonthsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read];
    type Reply = ResourceMonthsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ResourceMonthsParams::decl(), ResourceMonthsReply::decl(), ResourceMonthRow::decl(), ResourceMonth::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let hour = params.hour.min(query.timeline.last);
        let held = held(source, &query, &params.country, load, cancelled)?;
        let named = query.holders.named(&params.country);
        let next_day = day_start(hour) + DAY;
        let mut next = (next_day <= query.timeline.last).then_some(next_day);
        let resources = held
            .infos
            .iter()
            .zip(&held.lines)
            .map(|(info, history)| {
                next = next.into_iter().chain(next_change(&history.runs, hour).filter(|at| *at <= query.timeline.last)).min();
                let net = value_at(&history.runs, hour);
                ResourceMonthRow {
                    resource: info.clone(),
                    net,
                    missing: net.is_none().then(|| if named { reason_at(&history.missing, hour).unwrap_or(NOT_RECORDED) } else { NOT_HELD }.to_owned()),
                    months: history.through(hour),
                }
            })
            .collect();
        Ok(ResourceMonthsReply { tag: query.holders.name(&params.country, hour), hour, resources, next_change: next, warnings: held.warnings.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(first_hour: i32, last_hour: i32, value: f64) -> ChartRun {
        ChartRun { first_hour, last_hour, value, break_before: false }
    }

    #[test]
    fn a_month_is_the_mean_of_its_recorded_day_starts_through_the_hour() {
        let january = month_first(month_of(HOUR_EPOCH + 1936 * YEAR));
        let february = month_first(month_of(january) + 1);
        assert_eq!(february - january, 31 * DAY);
        assert_eq!(month_of(february - 1), month_of(january));
        let runs = [run(january, january + 10 * DAY - 1, 10.0), run(january + 20 * DAY, february + 5 * DAY, -2.0)];
        let missing = [MissingSpan { first_hour: january + 10 * DAY, last_hour: january + 20 * DAY - 1, reason: "Not read".into() }];
        let shown = months(&runs, &missing, january, february + 2 * DAY + 5);
        assert_eq!(shown.len(), 2);
        assert_eq!((shown[0].recorded_days, shown[0].days), (21, 31));
        assert_eq!(shown[0].net, Some((10.0 * 10.0 - 2.0 * 11.0) / 21.0));
        assert_eq!((shown[1].first_hour, shown[1].last_hour, shown[1].days, shown[1].net), (february, february + 2 * DAY + 5, 3, Some(-2.0)));
        let unread = months(&runs, &missing, january + 10 * DAY, january + 15 * DAY);
        assert_eq!((unread[0].net, unread[0].missing.as_deref()), (None, Some("Not read")));
        assert_eq!(next_change(&runs, january + 3), Some(january + 10 * DAY));
        assert_eq!(next_change(&runs, january + 12 * DAY), Some(january + 20 * DAY));
        assert_eq!(next_change(&runs, february + 6 * DAY), None);
    }

    #[test]
    fn resource_history_reuses_closed_months_and_projects_the_open_month_at_the_hour() {
        let january = month_first(month_of(HOUR_EPOCH + 1936 * YEAR));
        let february = month_first(month_of(january) + 1);
        let from = january + 4;
        let to = february + 2 * DAY + 5;
        let runs = [run(january, february - 1, 0.1), run(february + DAY, to, 3.0)];
        let missing = [MissingSpan { first_hour: february, last_hour: february + DAY - 1, reason: "Not read".into() }];
        let held = ResourceHistory::new(runs.to_vec(), missing.to_vec(), from, to);
        for hour in [from - 1, from, february - 1, february, february + DAY + 2, to] {
            assert_eq!(held.through(hour), months(&runs, &missing, from, hour));
        }
        let held = ResourceHistory { runs: Vec::new(), missing: Vec::new(), months: held.months };
        assert_eq!(held.through(february)[0], months(&runs, &missing, from, february)[0], "showing February never reads the days of January again");
    }
}
