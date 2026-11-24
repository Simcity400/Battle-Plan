use super::casualties::{EndedCounters, Lifetime, cached_casualty_ends};
use dashboard_base::{Source, products::{Country, LoadContext, at_hour::{At, AtHour}, validate_country}};
use dashboard_query::timeline::Timeline;
use dashboard_world::products::{
    groups::{FactionAt, FactionGroup, factions_at},
    holder_tags::{HolderTags, Identity},
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 2] = [TableId::CasualtyLifetimes, TableId::CasualtyTotals];
const COUNTER_NOT_RECORDED: &str = "A war relation's casualties were not recorded at this hour";
const INCOMPLETE: &str = "Part of this pair's history fell in a recording gap; the figure may be low";

pub struct CasualtyMatrix;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyMatrixParams {
    pub branch_id: String,
    #[doc = " Chosen countries. Empty means every country."]
    #[serde(default)]
    pub countries: Vec<Country>,
    #[doc = " With countries chosen: only the casualties between them, otherwise theirs with anyone."]
    #[serde(default)]
    pub between: bool,
    #[doc = " The countries with the most casualties dealt and taken, and the pairs between them."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[doc = " One country: its casualties with each opponent instead of the matrix."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub subject: Option<Country>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyMatrixCountry {
    pub tag: String,
    pub faction_id: Option<String>,
    pub faction_name: Option<String>,
    pub faction_name_missing: Option<String>,
    #[doc = " Casualties it dealt to every opponent at the hour, as a decimal string; null where a pair is not recorded."]
    pub dealt: Option<String>,
    #[doc = " Casualties it took from every opponent."]
    pub taken: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyMatrixCell {
    #[doc = " Indexes into `countries`: the row dealt the casualties to the column."]
    pub row: u32,
    pub column: u32,
    #[doc = " Decimal string; null where not recorded (`missing` says why)."]
    pub dealt: Option<String>,
    pub missing: Option<String>,
    pub incomplete: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyOpponent {
    pub country: u32,
    #[doc = " Casualties the subject dealt to this opponent."]
    pub dealt: Option<String>,
    #[doc = " Casualties the subject took from this opponent."]
    pub taken: Option<String>,
    pub missing: Option<String>,
    pub incomplete: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CasualtyMatrixReply {
    pub countries: Vec<CasualtyMatrixCountry>,
    #[doc = " Indexes into `countries`, grouped by faction (by faction name, factionless last), then by casualties dealt."]
    #[doc = " Rows and columns are the same countries in the same order."]
    pub order: Vec<u32>,
    #[doc = " Pairs that were at war, by row then column; every other pair dealt none."]
    pub cells: Vec<CasualtyMatrixCell>,
    #[doc = " The largest displayed cell, for the colour scale."]
    pub most: String,
    #[doc = " The largest total dealt among the rows, for the total bars."]
    pub largest_dealt: String,
    #[doc = " With a subject: its opponents, most casualties both ways first; ties by tag."]
    pub subject: Option<u32>,
    pub opponents: Vec<CasualtyOpponent>,
    #[doc = " The largest single side among the opponents, for the bar scale."]
    pub largest_side: String,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(CasualtyMatrixReply);

pub struct RunCounters {
    run: u32,
    lifetimes: Arc<Vec<(u128, i64, i64)>>,
    ended: Arc<EndedCounters>,
}

pub struct MatrixContext {
    timeline: Option<Arc<Timeline>>,
    runs: Vec<RunCounters>,
    factions: Arc<Vec<FactionGroup>>,
    warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counter {
    pub a: Identity,
    pub b: Identity,
    pub taken: Option<(i64, i64)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PairSum {
    pub dealt: Option<i128>,
    pub incomplete: bool,
}

pub type Pairs = BTreeMap<(String, String), PairSum>;

pub fn fold(counters: &[Counter], tag_of: impl Fn(Identity) -> Option<String>, partial: impl Fn(Identity) -> bool) -> Pairs {
    let mut out: Pairs = BTreeMap::new();
    for counter in counters {
        let (Some(a), Some(b)) = (tag_of(counter.a), tag_of(counter.b)) else { continue };
        if a == b {
            continue;
        }
        let incomplete = partial(counter.a) || partial(counter.b);
        let sides = [((b.clone(), a.clone()), counter.taken.map(|taken| taken.0)), ((a, b), counter.taken.map(|taken| taken.1))];
        for (key, dealt) in sides {
            let held = out.entry(key).or_insert(PairSum { dealt: Some(0), incomplete: false });
            held.dealt = held.dealt.zip(dealt).map(|(sum, more)| sum + i128::from(more));
            held.incomplete |= incomplete;
        }
    }
    out
}

pub struct Shape<'a> {
    pub chosen: Option<&'a HashSet<String>>,
    pub between: bool,
    pub top: Option<u32>,
    pub subject: Option<&'a str>,
    pub factions: &'a HashMap<String, FactionAt>,
}

fn add(sum: Option<i128>, more: Option<i128>) -> Option<i128> {
    sum.zip(more).map(|(a, b)| a + b)
}

fn text(value: Option<i128>) -> Option<String> {
    value.map(|value| value.to_string())
}

pub fn build(pairs: &Pairs, shape: &Shape<'_>) -> CasualtyMatrixReply {
    let mut dealt: HashMap<&str, Option<i128>> = HashMap::new();
    let mut taken: HashMap<&str, Option<i128>> = HashMap::new();
    for ((from, to), sum) in pairs {
        let held = dealt.entry(from).or_insert(Some(0));
        *held = add(*held, sum.dealt);
        let held = taken.entry(to).or_insert(Some(0));
        *held = add(*held, sum.dealt);
    }
    let volume = |tag: &str| dealt.get(tag).copied().flatten().unwrap_or(0) + taken.get(tag).copied().flatten().unwrap_or(0);
    let mut countries: Vec<CasualtyMatrixCountry> = Vec::new();
    let mut index: HashMap<String, u32> = HashMap::new();
    let mut intern = |tag: &str, countries: &mut Vec<CasualtyMatrixCountry>| -> u32 {
        if let Some(&at) = index.get(tag) {
            return at;
        }
        let at = countries.len() as u32;
        let faction = shape.factions.get(tag);
        let total_dealt = dealt.get(tag).copied().unwrap_or(Some(0));
        let total_taken = taken.get(tag).copied().unwrap_or(Some(0));
        countries.push(CasualtyMatrixCountry {
            tag: tag.to_owned(),
            faction_id: faction.map(|faction| faction.id.clone()),
            faction_name: faction.and_then(|faction| faction.name.clone()),
            faction_name_missing: faction.and_then(|faction| faction.name_missing.clone()),
            dealt: text(total_dealt),
            taken: text(total_taken),
            missing: (total_dealt.is_none() || total_taken.is_none()).then(|| COUNTER_NOT_RECORDED.to_owned()),
        });
        index.insert(tag.to_owned(), at);
        at
    };
    let mut reply = CasualtyMatrixReply {
        countries: Vec::new(),
        order: Vec::new(),
        cells: Vec::new(),
        most: "0".to_owned(),
        largest_dealt: "0".to_owned(),
        subject: None,
        opponents: Vec::new(),
        largest_side: "0".to_owned(),
        missing: None,
        warnings: Vec::new(),
    };

    if let Some(subject) = shape.subject {
        let mut opponents: Vec<&str> = pairs.keys().filter_map(|(from, to)| (from == subject).then_some(to.as_str()).or_else(|| (to == subject).then_some(from.as_str()))).collect::<HashSet<_>>().into_iter().collect();
        let side = |from: &str, to: &str| pairs.get(&(from.to_owned(), to.to_owned()));
        let both = |opponent: &str| side(subject, opponent).and_then(|sum| sum.dealt).unwrap_or(0) + side(opponent, subject).and_then(|sum| sum.dealt).unwrap_or(0);
        opponents.sort_by(|a, b| both(b).cmp(&both(a)).then(a.cmp(b)));
        reply.subject = Some(intern(subject, &mut countries));
        let mut largest = 0i128;
        for opponent in opponents {
            let out = side(subject, opponent).cloned().unwrap_or_default();
            let back = side(opponent, subject).cloned().unwrap_or_default();
            largest = largest.max(out.dealt.unwrap_or(0)).max(back.dealt.unwrap_or(0));
            let country = intern(opponent, &mut countries);
            reply.opponents.push(CasualtyOpponent {
                country,
                dealt: text(out.dealt),
                taken: text(back.dealt),
                missing: (out.dealt.is_none() || back.dealt.is_none()).then(|| COUNTER_NOT_RECORDED.to_owned()),
                incomplete: (out.incomplete || back.incomplete).then(|| INCOMPLETE.to_owned()),
            });
        }
        reply.largest_side = largest.to_string();
        reply.countries = countries;
        return reply;
    }

    let everyone: HashSet<&str> = pairs.keys().flat_map(|(from, to)| [from.as_str(), to.as_str()]).collect();
    let ranked_top: Option<HashSet<&str>> = shape.top.filter(|_| shape.chosen.is_none()).map(|limit| {
        let mut ranked: Vec<&str> = everyone.iter().copied().collect();
        ranked.sort_by(|a, b| volume(b).cmp(&volume(a)).then(a.cmp(b)));
        ranked.truncate(limit as usize);
        ranked.into_iter().collect()
    });
    let keep = |from: &str, to: &str| match (shape.chosen, &ranked_top) {
        (Some(chosen), _) if shape.between => chosen.contains(from) && chosen.contains(to),
        (Some(chosen), _) => chosen.contains(from) || chosen.contains(to),
        (None, Some(top)) => top.contains(from) && top.contains(to),
        (None, None) => true,
    };
    let kept: Vec<(&(String, String), &PairSum)> = pairs.iter().filter(|((from, to), _)| keep(from, to)).collect();
    let mut shown: HashSet<&str> = kept.iter().flat_map(|((from, to), _)| [from.as_str(), to.as_str()]).collect();
    if let Some(chosen) = shape.chosen {
        shown.extend(chosen.iter().map(String::as_str).filter(|tag| everyone.contains(tag)));
    }
    let group_key = |tag: &str| match shape.factions.get(tag) {
        Some(faction) => (0u8, faction.name.clone().unwrap_or_default(), faction.id.clone()),
        None => (1u8, String::new(), String::new()),
    };
    let score = |tag: &str| dealt.get(tag).copied().flatten().unwrap_or(0);
    let mut ordered: Vec<&str> = shown.into_iter().collect();
    ordered.sort_by(|a, b| group_key(a).cmp(&group_key(b)).then(score(b).cmp(&score(a))).then(a.cmp(b)));
    let position: HashMap<&str, u32> = ordered.iter().enumerate().map(|(at, tag)| (*tag, at as u32)).collect();
    reply.order = ordered.iter().map(|tag| intern(tag, &mut countries)).collect();
    let mut most = 0i128;
    let mut largest = 0i128;
    for tag in &ordered {
        largest = largest.max(score(tag));
    }
    for ((from, to), sum) in kept {
        let (Some(&row), Some(&column)) = (position.get(from.as_str()), position.get(to.as_str())) else { continue };
        most = most.max(sum.dealt.unwrap_or(0));
        reply.cells.push(CasualtyMatrixCell {
            row,
            column,
            dealt: text(sum.dealt),
            missing: sum.dealt.is_none().then(|| COUNTER_NOT_RECORDED.to_owned()),
            incomplete: sum.incomplete.then(|| INCOMPLETE.to_owned()),
        });
    }
    reply.cells.sort_by_key(|cell| (cell.row, cell.column));
    reply.most = most.to_string();
    reply.largest_dealt = largest.to_string();
    reply.countries = countries;
    reply
}

fn counters(frame: &DecodedFrame, run: Option<&RunCounters>, hour: i32) -> io::Result<Vec<Counter>> {
    let rows = frame.complete::<tables::CasualtyLifetimes>()?;
    let mut live: HashMap<u128, (i64, i64)> = HashMap::with_capacity(rows.len());
    for row in rows {
        live.insert(row.key(), (row.taken_a, row.taken_b));
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut seen: HashSet<u128> = HashSet::with_capacity(rows.len());
    if let Some(run) = run {
        for &(key, born, gone) in run.lifetimes.iter() {
            if born > i64::from(hour) {
                continue;
            }
            let lifetime = Lifetime::of(key);
            let taken = if gone < i64::from(hour) {
                match run.ended.0.get(&key) {
                    Some((Some(a), Some(b))) => Some((*a, *b)),
                    _ => None,
                }
            } else {
                live.get(&key).copied()
            };
            seen.insert(key);
            out.push(Counter { a: lifetime.a, b: lifetime.b, taken });
        }
    }
    for (key, taken) in live {
        if !seen.contains(&key) {
            let lifetime = Lifetime::of(key);
            out.push(Counter { a: lifetime.a, b: lifetime.b, taken: Some(taken) });
        }
    }
    Ok(out)
}

fn empty(missing: Option<String>, warnings: Vec<String>) -> CasualtyMatrixReply {
    CasualtyMatrixReply {
        countries: Vec::new(),
        order: Vec::new(),
        cells: Vec::new(),
        most: "0".to_owned(),
        largest_dealt: "0".to_owned(),
        subject: None,
        opponents: Vec::new(),
        largest_side: "0".to_owned(),
        missing,
        warnings,
    }
}

impl AtHour for CasualtyMatrix {
    const KEY: &'static str = "casualty-matrix";
    const TITLE: &'static str = "Casualties between countries";
    const CONTENT_DEPENDENT: bool = false;
    type Params = CasualtyMatrixParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", between, top, subject];
    type Reply = CasualtyMatrixReply;
    type Context = MatrixContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            CasualtyMatrixParams::decl(),
            CasualtyMatrixCountry::decl(),
            CasualtyMatrixCell::decl(),
            CasualtyOpponent::decl(),
            CasualtyMatrixReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().try_for_each(validate_country)?;
        params.subject.as_ref().map_or(Ok(()), validate_country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(MatrixContext { timeline: None, runs: Vec::new(), factions: Arc::new(Vec::new()), warnings: Vec::new() })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let mut warnings = Vec::new();
        let Ok(branch) = params.branch_id.parse::<u64>() else { return Self::context(source, load) };
        let query = dashboard_base::queries::query(source, branch)?;
        let mut runs: Vec<RunCounters> = Vec::with_capacity(query.timeline.stretches.len());
        for stretch in &query.timeline.stretches {
            if runs.iter().any(|held| held.run == stretch.run) {
                continue;
            }
            let lifetimes = dashboard_base::queries::lifetimes(source, stretch.run, TableId::CasualtyLifetimes)?;
            let ended = cached_casualty_ends(source, stretch.run)?;
            runs.push(RunCounters { run: stretch.run, lifetimes, ended });
        }
        let factions = match dashboard_world::products::groups::groups(source, &query, &params.branch_id, load, &AtomicBool::new(false)) {
            Ok(reply) => Arc::new(reply.factions),
            Err(_) => {
                warnings.push("Factions could not be read for this branch; countries are not grouped.".to_owned());
                Arc::new(Vec::new())
            }
        };
        Ok(MatrixContext { timeline: Some(Arc::clone(&query.timeline)), runs, factions, warnings })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let warnings = at.context.warnings.clone();
        if let Some(reason) = dashboard_text::scope::unread(TableId::CasualtyLifetimes, &frame.tables[TableId::CasualtyLifetimes.index()]) {
            return Ok(empty(Some(reason), warnings));
        }
        let run = at.context.timeline.as_ref().and_then(|timeline| timeline.at(at.hour)).and_then(|stretch| {
            at.context.runs.iter().find(|run| run.run == stretch.run)
        });
        let counters = match counters(frame, run, at.hour) {
            Ok(counters) => counters,
            Err(error) => return Ok(empty(Some(error.to_string()), warnings)),
        };
        let partial: HashSet<Identity> = frame
            .complete::<tables::CasualtyTotals>()
            .map(|rows| rows.iter().filter(|row| row.partial != 0).map(|row| (row.slot, row.id)).collect())
            .unwrap_or_default();
        let tags = HolderTags::from_holders(at.holders);
        let pairs = fold(&counters, |identity| tags.latest_at(identity, at.hour).map(str::to_owned), |identity| partial.contains(&identity));
        let resolve = |country: &Country| -> Option<String> { at.holders.tag_at(country, at.hour).map(str::to_owned).or_else(|| country.tag().map(str::to_owned)) };
        let chosen: Option<HashSet<String>> = (!params.countries.is_empty()).then(|| params.countries.iter().filter_map(resolve).collect());
        let subject = params.subject.as_ref().and_then(resolve);
        if params.subject.is_some() && subject.is_none() {
            return Ok(empty(Some("The chosen country holds no tag at this hour".to_owned()), warnings));
        }
        let factions = factions_at(&at.context.factions, at.hour);
        let shape = Shape { chosen: chosen.as_ref(), between: params.between, top: params.top, subject: subject.as_deref(), factions: &factions };
        let mut reply = build(&pairs, &shape);
        reply.warnings = warnings;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GER: Identity = (1, 10);
    const ENG: Identity = (2, 20);
    const POL: Identity = (3, 30);
    const FRA: Identity = (4, 40);

    fn tag(identity: Identity) -> Option<String> {
        Some(match identity {
            GER => "GER",
            ENG => "ENG",
            POL => "POL",
            FRA => "FRA",
            _ => return None,
        }.to_owned())
    }

    fn counter(a: Identity, b: Identity, taken: Option<(i64, i64)>) -> Counter {
        Counter { a, b, taken }
    }

    fn no_factions() -> HashMap<String, FactionAt> {
        HashMap::new()
    }

    #[test]
    fn a_relation_counts_what_each_side_took_as_the_other_side_dealt_and_lifetimes_sum() {
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, POL, Some((5, 20)))], tag, |_| false);
        assert_eq!(pairs[&("POL".to_owned(), "GER".to_owned())].dealt, Some(105));
        assert_eq!(pairs[&("GER".to_owned(), "POL".to_owned())].dealt, Some(920));
    }

    #[test]
    fn an_unrecorded_counter_makes_the_pair_and_its_totals_missing_never_zero() {
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, FRA, None)], tag, |_| false);
        let reply = build(&pairs, &Shape { chosen: None, between: false, top: None, subject: None, factions: &no_factions() });
        let at = |name: &str| reply.countries.iter().position(|country| country.tag == name).unwrap() as u32;
        let cell = reply.cells.iter().find(|cell| cell.row == reply.order.iter().position(|&i| i == at("GER")).unwrap() as u32
            && cell.column == reply.order.iter().position(|&i| i == at("FRA")).unwrap() as u32).unwrap();
        assert_eq!(cell.dealt, None);
        assert_eq!(cell.missing.as_deref(), Some(COUNTER_NOT_RECORDED));
        let germany = &reply.countries[at("GER") as usize];
        assert_eq!(germany.dealt, None);
        assert!(germany.missing.is_some());
        assert_eq!(reply.countries[at("POL") as usize].dealt.as_deref(), Some("100"));
    }

    #[test]
    fn rows_group_by_faction_then_by_casualties_dealt() {
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, ENG, Some((50, 10))), counter(FRA, GER, Some((300, 30)))], tag, |_| false);
        let factions: HashMap<String, FactionAt> = [("ENG", "1", "Allies"), ("FRA", "1", "Allies"), ("POL", "1", "Allies")]
            .into_iter()
            .map(|(tag, id, name)| (tag.to_owned(), FactionAt { id: id.to_owned(), name: Some(name.to_owned()), name_missing: None }))
            .collect();
        let reply = build(&pairs, &Shape { chosen: None, between: false, top: None, subject: None, factions: &factions });
        let order: Vec<&str> = reply.order.iter().map(|&at| reply.countries[at as usize].tag.as_str()).collect();
        assert_eq!(order, ["POL", "ENG", "FRA", "GER"]);
        assert_eq!(reply.largest_dealt, "1210");
        assert_eq!(reply.most, "900");
    }

    #[test]
    fn between_keeps_only_pairs_of_chosen_countries() {
        let pairs = fold(&[counter(GER, POL, Some((1, 2))), counter(GER, ENG, Some((3, 4)))], tag, |_| false);
        let chosen: HashSet<String> = ["GER".to_owned(), "POL".to_owned()].into_iter().collect();
        let reply = build(&pairs, &Shape { chosen: Some(&chosen), between: true, top: None, subject: None, factions: &no_factions() });
        assert_eq!(reply.order.len(), 2);
        assert_eq!(reply.cells.len(), 2);
        let touching = build(&pairs, &Shape { chosen: Some(&chosen), between: false, top: None, subject: None, factions: &no_factions() });
        assert_eq!(touching.order.len(), 3);
    }

    #[test]
    fn one_country_lists_opponents_by_casualties_both_ways() {
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, ENG, Some((2000, 10))), counter(GER, FRA, Some((1, 1)))], tag, |identity| identity == FRA);
        let reply = build(&pairs, &Shape { chosen: None, between: false, top: None, subject: Some("GER"), factions: &no_factions() });
        let names: Vec<(&str, Option<&str>, Option<&str>)> = reply
            .opponents
            .iter()
            .map(|opponent| (reply.countries[opponent.country as usize].tag.as_str(), opponent.dealt.as_deref(), opponent.taken.as_deref()))
            .collect();
        assert_eq!(names, [("ENG", Some("10"), Some("2000")), ("POL", Some("900"), Some("100")), ("FRA", Some("1"), Some("1"))]);
        assert_eq!(reply.largest_side, "2000");
        assert!(reply.opponents[2].incomplete.is_some());
        assert!(reply.cells.is_empty());
    }
}
