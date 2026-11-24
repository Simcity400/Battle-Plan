use super::{MilitaryAirFamily, MilitaryAirMetric, MilitaryAirParams, MilitaryAirReply, window_reply};
use dashboard_base::products::{Country, LoadContext, Product, validate_country};
use dashboard_base::{Source, span::HourSpan};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct MilitaryAirAt;

pub const NOT_OBSERVED: &str = "Military air events were not read at this hour";
pub const POPULATION_INCOMPLETE: &str = "Some military air events since the totals' start may be missing, so countries cannot be ranked";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MilitaryAirAtParams {
    pub branch_id: String,
    pub family: MilitaryAirFamily,
    #[serde(default)]
    pub regions: Vec<u32>,
    #[serde(default)]
    pub causes: Vec<u32>,
    #[doc = " A range starts at the events' own game hour; the timeline's first observed hour when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from_hour: Option<i32>,
    pub hour: i32,
    pub metric: MilitaryAirMetric,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " Only these countries' rows, a recorded zero included."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirAtRow {
    pub tag: String,
    pub identity: String,
    #[doc = " The first hour of the holder's unbroken presence that holds the hour."]
    pub segment_first_hour: i32,
    pub dealt: String,
    pub taken: String,
    #[doc = " False where events since the totals' start may be missing: the totals are at least these."]
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirAtReply {
    pub family: MilitaryAirFamily,
    pub hour: i32,
    pub counted_from: Option<i32>,
    #[doc = " The last hour this answer holds for."]
    pub valid_until: i32,
    #[doc = " Why nothing is known at the hour."]
    pub missing: Option<String>,
    #[doc = " Why the rows are not in rank order: the population is not fully read since the totals' start."]
    pub rank_missing: Option<String>,
    pub rows: Vec<MilitaryAirAtRow>,
    #[doc = " Totals whose country is not recorded: dealt, then taken."]
    pub unattributed: [String; 2],
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MilitaryAirAtReply);

fn run_at(spans: &[HourSpan], hour: i32) -> Option<&HourSpan> {
    let at = spans.partition_point(|span| span.last_hour < hour);
    spans.get(at).filter(|span| span.first_hour <= hour)
}

fn chosen(countries: &Option<Vec<Country>>, tag: &str, identity: &str) -> bool {
    countries.as_ref().is_none_or(|countries| {
        countries.iter().any(|country| match country {
            Country::Tag(wanted) => wanted == tag,
            Country::Slot(slot) => identity.starts_with(&format!("{slot}:")),
        })
    })
}

#[derive(Clone, Copy, Default)]
struct Totals {
    dealt: i128,
    taken: i128,
}

fn value(totals: Totals, metric: MilitaryAirMetric) -> i128 {
    match metric {
        MilitaryAirMetric::Dealt => totals.dealt,
        MilitaryAirMetric::Taken => totals.taken,
        MilitaryAirMetric::Total => totals.dealt + totals.taken,
    }
}

fn parse(text: &str) -> io::Result<i128> {
    text.parse().map_err(|_| dashboard_core::invalid("Military air total is not a whole number"))
}

fn through(reply: &MilitaryAirReply, hour: i32) -> io::Result<HashMap<Option<&str>, Totals>> {
    let mut out: HashMap<Option<&str>, Totals> = HashMap::new();
    if hour < reply.first_hour.saturating_sub(1) {
        return Ok(out);
    }
    for opening in &reply.openings {
        let held = out.entry(opening.identity.as_deref()).or_default();
        let total = parse(&opening.total)?;
        if opening.dealt { held.dealt += total } else { held.taken += total }
    }
    for part in &reply.parts {
        let at = part.points.partition_point(|point| point.1 <= hour);
        let Some(point) = at.checked_sub(1).and_then(|at| part.points.get(at)) else { continue };
        let held = out.entry(part.identity.as_deref()).or_default();
        let total = parse(&point.2)?;
        if part.dealt { held.dealt += total } else { held.taken += total }
    }
    Ok(out)
}

fn next_change(reply: &MilitaryAirReply, hour: i32) -> i32 {
    let mut next = reply.last_hour.saturating_add(1);
    let mut consider = |at: i32| {
        if at > hour && at < next {
            next = at;
        }
    };
    for part in &reply.parts {
        let at = part.points.partition_point(|point| point.1 <= hour);
        if let Some(point) = part.points.get(at) {
            consider(point.1);
        }
    }
    let spans = reply.observed.iter().chain(&reply.incomplete).chain(reply.countries.iter().flat_map(|country| &country.present));
    for span in spans {
        consider(span.first_hour);
        consider(span.last_hour.saturating_add(1));
    }
    next
}

pub fn answer(reply: &MilitaryAirReply, params: &MilitaryAirAtParams) -> io::Result<MilitaryAirAtReply> {
    let hour = params.hour;
    let mut out = MilitaryAirAtReply {
        family: params.family,
        hour,
        counted_from: reply.counted_from,
        valid_until: next_change(reply, hour).saturating_sub(1).max(hour),
        missing: None,
        rank_missing: None,
        rows: Vec::new(),
        unattributed: ["0".to_owned(), "0".to_owned()],
        warnings: reply.warnings.clone(),
    };
    let Some(observed) = run_at(&reply.observed, hour) else {
        out.missing = Some(NOT_OBSERVED.to_owned());
        return Ok(out);
    };
    let from = params.from_hour.or(reply.counted_from).unwrap_or(observed.first_hour);
    let complete = from <= hour && observed.first_hour <= from && !reply.incomplete.iter().any(|read| read.first_hour <= hour && read.last_hour >= from);
    let now = through(reply, hour)?;
    let before = through(reply, from.saturating_sub(1))?;
    let since = |identity: Option<&str>| {
        let (now, before) = (now.get(&identity).copied().unwrap_or_default(), before.get(&identity).copied().unwrap_or_default());
        Totals { dealt: now.dealt - before.dealt, taken: now.taken - before.taken }
    };
    let unattributed = since(None);
    out.unattributed = [unattributed.dealt.to_string(), unattributed.taken.to_string()];
    let mut rows: Vec<(Totals, MilitaryAirAtRow)> = Vec::new();
    for country in &reply.countries {
        let Some(present) = run_at(&country.present, hour) else { continue };
        if !chosen(&params.countries, &country.tag, &country.identity) {
            continue;
        }
        let totals = since(Some(country.identity.as_str()));
        rows.push((totals, MilitaryAirAtRow {
            tag: country.tag.clone(),
            identity: country.identity.clone(),
            segment_first_hour: present.first_hour,
            dealt: totals.dealt.to_string(),
            taken: totals.taken.to_string(),
            complete,
        }));
    }
    if complete {
        rows.sort_by(|(a, a_row), (b, b_row)| value(*b, params.metric).cmp(&value(*a, params.metric)).then_with(|| a_row.tag.cmp(&b_row.tag)));
        if let Some(top) = params.top {
            if params.countries.is_none() {
                rows.retain(|(totals, _)| value(*totals, params.metric) != 0);
            }
            rows.truncate(usize::try_from(top).unwrap_or(usize::MAX));
        }
    } else {
        out.rank_missing = Some(POPULATION_INCOMPLETE.to_owned());
        if params.top.is_some() {
            rows.clear();
        } else {
            rows.sort_by(|(_, a), (_, b)| a.tag.cmp(&b.tag).then_with(|| a.identity.cmp(&b.identity)));
        }
    }
    out.rows = rows.into_iter().map(|(_, row)| row).collect();
    Ok(out)
}

type WindowKey = (u64, MilitaryAirFamily, Vec<u32>, Vec<u32>, Option<i32>, u32);

#[derive(Default)]
struct Windows(VecDeque<(WindowKey, Arc<MilitaryAirReply>)>);

const KEPT_WINDOWS: usize = 8;

fn window(source: &Source, params: &MilitaryAirAtParams, branch: u64, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<MilitaryAirReply>> {
    let index = u32::try_from(params.hour.div_euclid(dashboard_world::products::windows::HOURS)).map_err(|_| dashboard_core::invalid("Hour is before the first window"))?;
    let key: WindowKey = (branch, params.family, params.regions.clone(), params.causes.clone(), params.from_hour, index);
    let state = source.state::<Windows>();
    if let Some((_, found)) = state.lock().unwrap_or_else(PoisonError::into_inner).0.iter().find(|(held, _)| *held == key) {
        return Ok(Arc::clone(found));
    }
    let query = dashboard_base::queries::query(source, branch)?;
    let ledger = MilitaryAirParams {
        branch_id: params.branch_id.clone(),
        family: params.family,
        regions: params.regions.clone(),
        causes: params.causes.clone(),
        from_hour: params.from_hour,
        window: index,
        top: None,
    };
    let made = Arc::new(window_reply(source, &query, &ledger, dashboard_world::products::windows::within(&query, index)?, context, cancelled)?);
    let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
    kept.0.retain(|(held, _)| *held != key);
    kept.0.push_back((key, Arc::clone(&made)));
    while kept.0.len() > KEPT_WINDOWS {
        kept.0.pop_front();
    }
    Ok(made)
}

impl Product for MilitaryAirAt {
    const KEY: &'static str = "military-air-at";
    const CONTENT_DEPENDENT: bool = true;
    type Params = MilitaryAirAtParams;
    const FIELDS: dashboard_base::products::Fields =
        dashboard_base::fields![branchId, family, regions, causes, fromHour: Start, hour: Read, metric, top, countries, "causes[]", "regions[]", "countries[]"];
    type Reply = MilitaryAirAtReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.top == Some(0) {
            return Err("Top count must be positive");
        }
        params.countries.iter().flatten().try_for_each(validate_country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![MilitaryAirAtParams::decl(), MilitaryAirAtRow::decl(), MilitaryAirAtReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let reply = window(source, params, branch, context, cancelled)?;
        answer(&reply, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::air_ledger::{MilitaryAirCountry, MilitaryAirPart};

    fn span(first_hour: i32, last_hour: i32) -> HourSpan {
        HourSpan { first_hour, last_hour }
    }

    fn country(tag: &str, identity: &str, present: HourSpan) -> MilitaryAirCountry {
        MilitaryAirCountry { tag: tag.into(), identity: identity.into(), present: vec![present], membership: None }
    }

    fn part(tag: Option<&str>, identity: Option<&str>, dealt: bool, points: &[(i32, &str)]) -> MilitaryAirPart {
        MilitaryAirPart {
            tag: tag.map(Into::into),
            identity: identity.map(Into::into),
            dealt,
            cause: None,
            opponent_known: None,
            building_token: None,
            building_id: None,
            state: None,
            points: points.iter().map(|&(hour, total)| (hour, hour, total.to_owned())).collect(),
        }
    }

    fn ledger() -> MilitaryAirReply {
        MilitaryAirReply {
            family: MilitaryAirFamily::Bombing,
            window: 0,
            first_hour: 0,
            last_hour: 99,
            counted_from: Some(0),
            available: true,
            observed: vec![span(0, 99)],
            incomplete: Vec::new(),
            countries: vec![country("GER", "1:1", span(0, 99)), country("SOV", "2:2", span(0, 99)), country("FRA", "3:3", span(50, 99))],
            openings: Vec::new(),
            parts: vec![
                part(Some("GER"), Some("1:1"), true, &[(5, "100"), (20, "300")]),
                part(Some("GER"), Some("1:1"), false, &[(10, "50")]),
                part(Some("SOV"), Some("2:2"), true, &[(8, "200")]),
                part(None, None, true, &[(9, "7")]),
            ],
            regions: Vec::new(),
            buildings: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn asked(hour: i32) -> MilitaryAirAtParams {
        MilitaryAirAtParams { branch_id: "1".into(), family: MilitaryAirFamily::Bombing, regions: Vec::new(), causes: Vec::new(), from_hour: None, hour, metric: MilitaryAirMetric::Total, top: Some(10), countries: None }
    }

    fn tags(reply: &MilitaryAirAtReply) -> Vec<(&str, &str, &str)> {
        reply.rows.iter().map(|row| (row.tag.as_str(), row.dealt.as_str(), row.taken.as_str())).collect()
    }

    #[test]
    fn the_product_ranks_the_top_at_the_hour_and_says_how_long_the_answer_holds() {
        let at = answer(&ledger(), &asked(15)).unwrap();
        assert_eq!(tags(&at), [("SOV", "200", "0"), ("GER", "100", "50")], "ranked by dealt plus taken; FRA is not present yet");
        assert_eq!(at.valid_until, 19, "the next event is read at hour 20");
        assert_eq!(at.unattributed, ["7".to_owned(), "0".to_owned()]);
        assert!(at.rows.iter().all(|row| row.complete));
        assert_eq!(answer(&ledger(), &MilitaryAirAtParams { top: Some(1), ..asked(15) }).unwrap().rows.len(), 1);
        let later = answer(&ledger(), &asked(25)).unwrap();
        assert_eq!(tags(&later), [("GER", "300", "50"), ("SOV", "200", "0")], "the order follows the hour");
    }

    #[test]
    fn chosen_countries_keep_a_recorded_zero_and_the_top_leaves_zeros_out() {
        let chosen = answer(&ledger(), &MilitaryAirAtParams { top: None, countries: Some(vec![Country::Tag("FRA".into()), Country::Slot(1)]), ..asked(60) }).unwrap();
        assert_eq!(tags(&chosen), [("GER", "300", "50"), ("FRA", "0", "0")]);
        let top = answer(&ledger(), &asked(60)).unwrap();
        assert!(top.rows.iter().all(|row| row.tag != "FRA"), "a ranked population leaves out countries with none");
    }

    #[test]
    fn a_partly_read_population_gives_witnessed_rows_but_no_rank_or_top() {
        let mut partly = ledger();
        partly.incomplete = vec![span(12, 12)];
        let top = answer(&partly, &asked(15)).unwrap();
        assert!(top.rows.is_empty(), "never a top over the countries that happened to be read");
        assert_eq!(top.rank_missing.as_deref(), Some(POPULATION_INCOMPLETE));
        let keyed = answer(&partly, &MilitaryAirAtParams { top: None, countries: Some(vec![Country::Tag("GER".into())]), ..asked(15) }).unwrap();
        assert_eq!(tags(&keyed), [("GER", "100", "50")]);
        assert!(!keyed.rows[0].complete, "a witnessed row is given, marked at least");
    }

    #[test]
    fn an_unread_hour_is_not_recorded_and_a_range_counts_from_its_start() {
        let mut gap = ledger();
        gap.observed = vec![span(0, 40)];
        let unread = answer(&gap, &asked(60)).unwrap();
        assert_eq!(unread.missing.as_deref(), Some(NOT_OBSERVED));
        assert!(unread.rows.is_empty());
        let ranged = answer(&ledger(), &MilitaryAirAtParams { from_hour: Some(10), ..asked(25) }).unwrap();
        assert_eq!(tags(&ranged), [("GER", "200", "50")], "events before the range start do not count; SOV has none in range");
    }
}
