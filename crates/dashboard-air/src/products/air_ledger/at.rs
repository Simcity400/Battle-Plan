use super::{MilitaryAirCauseName, MilitaryAirFamily, MilitaryAirMetric, MilitaryAirParams, MilitaryAirReply, air_loss_cause, window_reply};
use dashboard_query::series::ranked::{GroupSelection, Members};
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

pub const LOSSES_NOT_RECORDED: &str = "Aircraft loss events were not recorded at this hour";
pub const BOMBING_NOT_RECORDED: &str = "Bombing events were not recorded at this hour";
pub const LINKS_NOT_RECORDED: &str = "Country links were not recorded at this hour, so events cannot be credited to countries";
pub const POPULATION_INCOMPLETE: &str = "Some military air events since the totals' start may be missing, so countries cannot be ranked";
pub const ATTRIBUTION_INCOMPLETE: &str = "Some events have no recorded country and could change this order, so countries cannot be ranked";
pub const UNATTRIBUTED: &str = "Some events since the totals' start have no recorded country; this country's share of them is not known";

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
    #[doc = " Only the members of this group at the hour, resolved from the recorded group history."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MilitaryAirCauseTotal {
    #[serde(flatten)]
    pub cause: MilitaryAirCauseName,
    pub taken: String,
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
    #[doc = " False where events since the totals' start may be missing, or have no recorded country."]
    pub complete: bool,
    #[doc = " Why the totals may be incomplete."]
    pub partial: Option<String>,
    #[doc = " Dealt per taken in hundredths, when both are complete and taken is not zero."]
    pub ratio_hundredths: Option<String>,
    #[doc = " Air losses: own losses by recorded cause since the totals' start, most first; empty for bombing."]
    pub causes: Vec<MilitaryAirCauseTotal>,
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
    #[doc = " Asked countries not present at the hour."]
    pub absent: Vec<Country>,
    #[doc = " Totals whose country is not recorded: dealt, then taken."]
    pub unattributed: [String; 2],
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(MilitaryAirAtReply);

fn run_at(spans: &[HourSpan], hour: i32) -> Option<&HourSpan> {
    let at = spans.partition_point(|span| span.last_hour < hour);
    spans.get(at).filter(|span| span.first_hour <= hour)
}

pub(super) fn asks(country: &Country, tag: &str, identity: &str) -> bool {
    match country {
        Country::Tag(wanted) => wanted == tag,
        Country::Slot(slot) => identity.split(':').next() == Some(slot.to_string().as_str()),
    }
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

#[derive(Default)]
struct Through<'a> {
    totals: HashMap<Option<&'a str>, Totals>,
    causes: HashMap<(&'a str, u32), i128>,
}

fn through(reply: &MilitaryAirReply, hour: i32) -> io::Result<Through<'_>> {
    let mut out = Through::default();
    if hour < reply.first_hour.saturating_sub(1) {
        return Ok(out);
    }
    for opening in &reply.openings {
        let total = parse(&opening.total)?;
        count(&mut out, opening.identity.as_deref(), opening.dealt, Some(opening.cause), total);
    }
    for part in &reply.parts {
        let at = part.points.partition_point(|point| point.1 <= hour);
        let Some(point) = at.checked_sub(1).and_then(|at| part.points.get(at)) else { continue };
        let total = parse(&point.2)?;
        count(&mut out, part.identity.as_deref(), part.dealt, part.cause, total);
    }
    Ok(out)
}

fn count<'a>(out: &mut Through<'a>, identity: Option<&'a str>, dealt: bool, cause: Option<u32>, total: i128) {
    let held = out.totals.entry(identity).or_default();
    if dealt {
        held.dealt += total;
        return;
    }
    held.taken += total;
    if let (Some(identity), Some(cause)) = (identity, cause) {
        *out.causes.entry((identity, cause)).or_default() += total;
    }
}

fn next_change(reply: &MilitaryAirReply, hour: i32, relevant: &dyn Fn(Option<&str>) -> bool) -> i32 {
    let mut next = reply.last_hour.saturating_add(1);
    let mut consider = |at: i32| {
        if at > hour && at < next {
            next = at;
        }
    };
    for part in reply.parts.iter().filter(|part| part.identity.is_none() || relevant(part.identity.as_deref())) {
        let at = part.points.partition_point(|point| point.1 <= hour);
        if let Some(point) = part.points.get(at) {
            consider(point.1);
        }
    }
    let shown = reply.countries.iter().filter(|country| relevant(Some(country.identity.as_str()))).flat_map(|country| &country.present);
    let spans = reply.observed.iter().chain(&reply.incomplete).chain(&reply.unjoined).chain(shown);
    for span in spans {
        consider(span.first_hour);
        consider(span.last_hour.saturating_add(1));
    }
    next
}

pub fn answer(reply: &MilitaryAirReply, params: &MilitaryAirAtParams, members: Option<&Members>) -> io::Result<MilitaryAirAtReply> {
    let hour = params.hour;
    let mut out = MilitaryAirAtReply {
        family: params.family,
        hour,
        counted_from: reply.counted_from,
        valid_until: hour,
        missing: None,
        rank_missing: None,
        rows: Vec::new(),
        absent: Vec::new(),
        unattributed: ["0".to_owned(), "0".to_owned()],
        warnings: reply.warnings.clone(),
    };
    if let Some(refused) = members.and_then(|members| members.refused.as_ref()) {
        out.missing = Some(refused.clone());
        out.valid_until = members.map_or(hour, |members| members.until.max(hour));
        return Ok(out);
    }
    let wanted = |tag: &str, identity: &str| match (members, &params.countries) {
        (Some(members), _) => members.tags.binary_search_by(|held| held.as_str().cmp(tag)).is_ok(),
        (None, Some(countries)) => countries.iter().any(|country| asks(country, tag, identity)),
        (None, None) => true,
    };
    let ranked_population = members.is_none() && params.countries.is_none();
    let relevant = |identity: Option<&str>| {
        ranked_population || identity.is_some_and(|identity| reply.countries.iter().any(|country| country.identity == identity && wanted(&country.tag, identity)))
    };
    let mut until = next_change(reply, hour, &relevant).saturating_sub(1).max(hour);
    if let Some(members) = members {
        until = until.min(members.until.max(hour));
    }
    out.valid_until = until;
    let Some(observed) = run_at(&reply.observed, hour) else {
        out.missing = Some(
            if run_at(&reply.unjoined, hour).is_some() {
                LINKS_NOT_RECORDED
            } else if params.family == MilitaryAirFamily::AirLosses {
                LOSSES_NOT_RECORDED
            } else {
                BOMBING_NOT_RECORDED
            }
            .to_owned(),
        );
        return Ok(out);
    };
    let from = params.from_hour.or(reply.counted_from).unwrap_or(observed.first_hour);
    let read_whole = from <= hour && observed.first_hour <= from && !reply.incomplete.iter().any(|read| read.first_hour <= hour && read.last_hour >= from);
    let now = through(reply, hour)?;
    let before = through(reply, from.saturating_sub(1))?;
    let since = |identity: Option<&str>| {
        let (now, before) = (now.totals.get(&identity).copied().unwrap_or_default(), before.totals.get(&identity).copied().unwrap_or_default());
        Totals { dealt: now.dealt - before.dealt, taken: now.taken - before.taken }
    };
    let unattributed = since(None);
    out.unattributed = [unattributed.dealt.to_string(), unattributed.taken.to_string()];
    let attributed = unattributed.dealt == 0 && unattributed.taken == 0;
    let complete = read_whole && attributed;
    let partial = if !read_whole { Some(POPULATION_INCOMPLETE.to_owned()) } else if !attributed { Some(UNATTRIBUTED.to_owned()) } else { None };
    let mut causes_of: HashMap<&str, Vec<(u32, i128)>> = HashMap::new();
    if params.family == MilitaryAirFamily::AirLosses {
        for (&(identity, cause), &total) in &now.causes {
            let amount = total - before.causes.get(&(identity, cause)).copied().unwrap_or_default();
            if amount != 0 {
                causes_of.entry(identity).or_default().push((cause, amount));
            }
        }
    }
    let causes = |identity: &str| {
        let mut held = causes_of.get(identity).cloned().unwrap_or_default();
        held.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        held.into_iter().map(|(cause, amount)| MilitaryAirCauseTotal { cause: air_loss_cause(cause), taken: amount.to_string() }).collect()
    };
    let mut answered = vec![false; params.countries.as_ref().map_or(0, Vec::len)];
    let mut rows: Vec<(Totals, MilitaryAirAtRow)> = Vec::new();
    for country in &reply.countries {
        let Some(present) = run_at(&country.present, hour) else { continue };
        if !wanted(&country.tag, &country.identity) {
            continue;
        }
        for (at, asked) in params.countries.iter().flatten().enumerate() {
            if asks(asked, &country.tag, &country.identity) {
                answered[at] = true;
            }
        }
        let totals = since(Some(country.identity.as_str()));
        let ratio = (complete && totals.taken != 0).then(|| ((totals.dealt.abs() * 100 + totals.taken.abs() / 2) / totals.taken.abs()).to_string());
        rows.push((totals, MilitaryAirAtRow {
            tag: country.tag.clone(),
            identity: country.identity.clone(),
            segment_first_hour: present.first_hour,
            dealt: totals.dealt.to_string(),
            taken: totals.taken.to_string(),
            complete,
            partial: partial.clone(),
            ratio_hundredths: ratio,
            causes: causes(country.identity.as_str()),
        }));
    }
    out.absent = params.countries.iter().flatten().zip(&answered).filter(|(_, found)| !**found).map(|(country, _)| country.clone()).collect();
    rows.sort_by(|(a, a_row), (b, b_row)| value(*b, params.metric).cmp(&value(*a, params.metric)).then_with(|| a_row.tag.cmp(&b_row.tag)));
    let counted = if ranked_population { rows.iter().take_while(|(totals, _)| value(*totals, params.metric) != 0).count() } else { rows.len() };
    let shown = params.top.map_or(counted, |top| counted.min(usize::try_from(top).unwrap_or(usize::MAX)));
    let doubt = value(unattributed, params.metric).abs();
    let edge = (shown + 1).min(rows.len());
    let ordered = read_whole
        && rows[..edge].windows(2).all(|pair| doubt == 0 || value(pair[0].0, params.metric) - value(pair[1].0, params.metric) > doubt)
        && !(ranked_population && doubt > 0 && shown == 0);
    if ordered {
        rows.truncate(shown);
    } else {
        out.rank_missing = Some(if read_whole { ATTRIBUTION_INCOMPLETE } else { POPULATION_INCOMPLETE }.to_owned());
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

pub(super) fn window(source: &Source, params: &MilitaryAirAtParams, branch: u64, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Arc<MilitaryAirReply>> {
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
        dashboard_base::fields![branchId, family, regions, causes, fromHour: Start, hour: Read, metric, top, countries, group, "causes[]", "regions[]", "countries[]"];
    type Reply = MilitaryAirAtReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.top == Some(0) {
            return Err("Top count must be positive");
        }
        if params.group.is_some() && (params.top.is_some() || params.countries.is_some()) {
            return Err("A group is asked alone, without countries or a top");
        }
        params.countries.iter().flatten().try_for_each(validate_country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            MilitaryAirAtParams::decl(),
            MilitaryAirCauseTotal::decl(),
            MilitaryAirAtRow::decl(),
            MilitaryAirAtReply::decl(),
            super::tiles::AirLossSide::decl(),
            super::tiles::AirLossesTileFamily::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let reply = window(source, params, branch, context, cancelled)?;
        let members = match &params.group {
            Some(group) => {
                let query = dashboard_base::queries::query(source, branch)?;
                let groups = dashboard_world::products::groups::shared(source, &query, context, cancelled)?;
                Some(dashboard_world::products::group_members::members_at(&groups, group, params.hour, query.timeline.first)?)
            }
            None => None,
        };
        answer(&reply, params, members.as_ref())
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
            unjoined: Vec::new(),
            causes: Vec::new(),
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
        MilitaryAirAtParams { branch_id: "1".into(), family: MilitaryAirFamily::Bombing, regions: Vec::new(), causes: Vec::new(), from_hour: None, hour, metric: MilitaryAirMetric::Total, top: Some(10), countries: None, group: None }
    }

    fn answer_of(reply: &MilitaryAirReply, params: &MilitaryAirAtParams) -> io::Result<MilitaryAirAtReply> {
        answer(reply, params, None)
    }

    fn tags(reply: &MilitaryAirAtReply) -> Vec<(&str, &str, &str)> {
        reply.rows.iter().map(|row| (row.tag.as_str(), row.dealt.as_str(), row.taken.as_str())).collect()
    }

    #[test]
    fn the_product_ranks_the_top_at_the_hour_and_says_how_long_the_answer_holds() {
        let at = answer_of(&ledger(), &asked(15)).unwrap();
        assert_eq!(tags(&at), [("SOV", "200", "0"), ("GER", "100", "50")], "ranked by dealt plus taken; FRA is not present yet");
        assert_eq!(at.valid_until, 19, "the next event is read at hour 20");
        assert_eq!(at.unattributed, ["7".to_owned(), "0".to_owned()]);
        assert!(at.rows.iter().all(|row| !row.complete && row.partial.as_deref() == Some(UNATTRIBUTED)), "an event with no recorded country leaves every total open");
        assert!(at.rows.iter().all(|row| row.ratio_hundredths.is_none()), "no ratio of open totals");
        assert_eq!(answer_of(&ledger(), &MilitaryAirAtParams { top: Some(1), ..asked(15) }).unwrap().rows.len(), 1);
        let later = answer_of(&ledger(), &asked(25)).unwrap();
        assert_eq!(tags(&later), [("GER", "300", "50"), ("SOV", "200", "0")], "the order follows the hour");
    }

    #[test]
    fn chosen_countries_keep_a_recorded_zero_and_the_top_leaves_zeros_out() {
        let chosen = answer_of(&ledger(), &MilitaryAirAtParams { top: None, countries: Some(vec![Country::Tag("FRA".into()), Country::Slot(1)]), ..asked(60) }).unwrap();
        assert_eq!(tags(&chosen), [("GER", "300", "50"), ("FRA", "0", "0")]);
        let top = answer_of(&ledger(), &asked(60)).unwrap();
        assert!(top.rows.iter().all(|row| row.tag != "FRA"), "a ranked population leaves out countries with none");
    }

    #[test]
    fn a_partly_read_population_gives_witnessed_rows_but_no_rank_or_top() {
        let mut partly = ledger();
        partly.incomplete = vec![span(12, 12)];
        let top = answer_of(&partly, &asked(15)).unwrap();
        assert!(top.rows.is_empty(), "never a top over the countries that happened to be read");
        assert_eq!(top.rank_missing.as_deref(), Some(POPULATION_INCOMPLETE));
        let keyed = answer_of(&partly, &MilitaryAirAtParams { top: None, countries: Some(vec![Country::Tag("GER".into())]), ..asked(15) }).unwrap();
        assert_eq!(tags(&keyed), [("GER", "100", "50")]);
        assert!(!keyed.rows[0].complete, "a witnessed row is given, marked at least");
    }

    #[test]
    fn air_losses_give_each_country_its_own_losses_by_cause_since_the_start_most_first() {
        let mut losses = ledger();
        losses.family = MilitaryAirFamily::AirLosses;
        let caused = |cause: u32, dealt: bool, points: &[(i32, &str)]| MilitaryAirPart { cause: Some(cause), ..part(Some("GER"), Some("1:1"), dealt, points) };
        losses.parts = vec![caused(1, false, &[(5, "30"), (20, "40")]), caused(3, false, &[(6, "45")]), caused(1, true, &[(7, "99")])];
        let at = |hour: i32, from_hour: Option<i32>| answer_of(&losses, &MilitaryAirAtParams { family: MilitaryAirFamily::AirLosses, from_hour, ..asked(hour) }).unwrap();
        let causes = |reply: &MilitaryAirAtReply| reply.rows.iter().find(|row| row.tag == "GER").unwrap().causes.iter().map(|cause| (cause.cause.cause, cause.taken.clone())).collect::<Vec<_>>();
        assert_eq!(causes(&at(10, None)), [(3, "45".to_owned()), (1, "30".to_owned())], "own losses only, most first");
        assert_eq!(causes(&at(25, Some(10))), [(1, "10".to_owned())], "counted from the range start; a cause with none since then is left out");
        assert!(answer_of(&ledger(), &asked(25)).unwrap().rows.iter().all(|row| row.causes.is_empty()), "bombing has no loss causes");
    }

    #[test]
    fn an_unread_hour_is_not_recorded_and_a_range_counts_from_its_start() {
        let mut gap = ledger();
        gap.observed = vec![span(0, 40)];
        let unread = answer_of(&gap, &asked(60)).unwrap();
        assert_eq!(unread.missing.as_deref(), Some(BOMBING_NOT_RECORDED), "the reason names the events that were not recorded");
        assert!(unread.rows.is_empty());
        gap.unjoined = vec![span(41, 99)];
        assert_eq!(answer_of(&gap, &asked(60)).unwrap().missing.as_deref(), Some(LINKS_NOT_RECORDED), "read events without links are not credited");
        let quiet = MilitaryAirReply { parts: Vec::new(), ..ledger() };
        let none = answer_of(&quiet, &asked(60)).unwrap();
        assert_eq!((none.missing.as_deref(), none.rank_missing.as_deref(), none.rows.len()), (None, None, 0), "an observed hour with no events is a recorded none, not missing");
        let ranged = answer_of(&ledger(), &MilitaryAirAtParams { from_hour: Some(10), ..asked(25) }).unwrap();
        assert_eq!(tags(&ranged), [("GER", "200", "50")], "events before the range start do not count; SOV has none in range");
    }

    fn attributed() -> MilitaryAirReply {
        let mut reply = ledger();
        reply.parts.retain(|part| part.identity.is_some());
        reply
    }

    #[test]
    fn unattributed_losses_open_every_total_and_withhold_a_rank_they_could_change() {
        let mut only = ledger();
        only.parts = vec![part(None, None, false, &[(5, "40")])];
        let at = answer_of(&only, &MilitaryAirAtParams { metric: MilitaryAirMetric::Taken, ..asked(15) }).unwrap();
        assert_eq!((at.rows.len(), at.rank_missing.as_deref(), at.unattributed[1].as_str()), (0, Some(ATTRIBUTION_INCOMPLETE), "40"), "never \"none\" while losses have no country");
        let close = answer_of(&ledger(), &MilitaryAirAtParams { metric: MilitaryAirMetric::Dealt, ..asked(25) }).unwrap();
        assert_eq!(tags(&close), [("GER", "300", "50"), ("SOV", "200", "0")], "7 unattributed cannot close a gap of 100");
        let mut tight = ledger();
        tight.parts.push(part(None, None, true, &[(12, "200")]));
        assert_eq!(answer_of(&tight, &asked(25)).unwrap().rank_missing.as_deref(), Some(ATTRIBUTION_INCOMPLETE));
        let whole = answer_of(&attributed(), &asked(25)).unwrap();
        assert!(whole.rows.iter().all(|row| row.complete && row.partial.is_none()));
        assert_eq!(whole.rows[0].ratio_hundredths.as_deref(), Some("600"), "the product gives the ratio: 300 dealt per 50 taken");
    }

    #[test]
    fn a_requested_country_is_matched_once_and_one_not_present_is_named() {
        let at = answer_of(&attributed(), &MilitaryAirAtParams { top: None, countries: Some(vec![Country::Slot(1), Country::Tag("GER".into()), Country::Tag("ITA".into())]), ..asked(25) }).unwrap();
        assert_eq!(tags(&at), [("GER", "300", "50")], "slot 1 and GER are one country, one row");
        assert_eq!(at.absent, [Country::Tag("ITA".into())]);
    }

    #[test]
    fn a_group_is_its_members_at_the_hour_and_its_membership_bounds_the_answer() {
        let members = Members { tags: vec!["GER".into()], refused: None, from: 0, until: 21 };
        let at = answer(&attributed(), &MilitaryAirAtParams { top: None, ..asked(15) }, Some(&members)).unwrap();
        assert_eq!(tags(&at), [("GER", "100", "50")]);
        assert_eq!(at.valid_until, 19, "the next change of a member, not of SOV");
        let unknown = Members { tags: Vec::new(), refused: Some("Membership of this group is not recorded at this hour".into()), from: 15, until: 15 };
        assert_eq!(answer(&attributed(), &MilitaryAirAtParams { top: None, ..asked(15) }, Some(&unknown)).unwrap().missing.as_deref(), Some("Membership of this group is not recorded at this hour"));
        let keyed = answer_of(&attributed(), &MilitaryAirAtParams { top: None, countries: Some(vec![Country::Tag("SOV".into())]), ..asked(9) }).unwrap();
        assert_eq!(keyed.valid_until, 99, "neither GER's points nor FRA's arrival end SOV's answer: only the window does");
    }

    #[test]
    fn a_cause_without_a_proved_meaning_keeps_its_id_and_reason() {
        assert_eq!(air_loss_cause(3).name.as_deref(), Some("Air accidents"));
        let unknown = air_loss_cause(7);
        assert_eq!((unknown.cause, unknown.name, unknown.missing.as_deref()), (7, None, Some("Air loss cause 7 has no proved display meaning")));
    }

}
