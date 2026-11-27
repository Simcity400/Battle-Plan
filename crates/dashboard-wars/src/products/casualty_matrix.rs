use super::casualties::{EndedCounters, Lifetime, cached_casualty_ends};
use dashboard_base::{Source, products::{Country, LoadContext, at_hour::{At, AtHour, Clock, Edits, ReadsAt, before, field_columns}, validate_country}};
use dashboard_query::{identity::{Holder, Holders, TAG_UNLINKED}, timeline::Timeline};
use dashboard_world::products::{
    groups::{FactionAt, FactionGroup, factions_at, factions_valid_through},
    holder_tags::Identity,
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
    pub slot: u32,
    pub country_id: u32,
    #[doc = " When this holder no longer holds `tag` at the query hour, its latest witnessed hour for reading its name and flag; null while it still holds `tag`."]
    pub shown_hour: Option<i32>,
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
    edges: Vec<i32>,
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

pub type Pairs = BTreeMap<(Identity, Identity), PairSum>;

pub fn fold(counters: &[Counter], known: impl Fn(Identity) -> bool, partial: impl Fn(Identity) -> bool) -> Pairs {
    let mut out: Pairs = BTreeMap::new();
    for counter in counters {
        if !known(counter.a) || !known(counter.b) || counter.a == counter.b {
            continue;
        }
        let incomplete = partial(counter.a) || partial(counter.b);
        let sides = [((counter.b, counter.a), counter.taken.map(|taken| taken.0)), ((counter.a, counter.b), counter.taken.map(|taken| taken.1))];
        for (key, dealt) in sides {
            let held = out.entry(key).or_insert(PairSum { dealt: Some(0), incomplete: false });
            held.dealt = held.dealt.zip(dealt).map(|(sum, more)| sum + i128::from(more));
            held.incomplete |= incomplete;
        }
    }
    out
}

pub struct Shape<'a> {
    pub tags: &'a HashMap<Identity, String>,
    pub shown_hours: &'a HashMap<Identity, i32>,
    pub chosen: Option<&'a HashSet<Identity>>,
    pub between: bool,
    pub top: Option<u32>,
    pub subject: Option<Identity>,
    pub factions: &'a HashMap<Identity, FactionAt>,
}

fn add(sum: Option<i128>, more: Option<i128>) -> Option<i128> {
    sum.zip(more).map(|(a, b)| a + b)
}

fn text(value: Option<i128>) -> Option<String> {
    value.map(|value| value.to_string())
}

pub fn build(pairs: &Pairs, shape: &Shape<'_>) -> CasualtyMatrixReply {
    let tag_of = |identity: Identity| shape.tags.get(&identity).map(String::as_str).unwrap_or("");
    let mut dealt: HashMap<Identity, Option<i128>> = HashMap::new();
    let mut taken: HashMap<Identity, Option<i128>> = HashMap::new();
    for (&(from, to), sum) in pairs {
        let held = dealt.entry(from).or_insert(Some(0));
        *held = add(*held, sum.dealt);
        let held = taken.entry(to).or_insert(Some(0));
        *held = add(*held, sum.dealt);
    }
    let volume = |identity: Identity| dealt.get(&identity).copied().flatten().unwrap_or(0) + taken.get(&identity).copied().flatten().unwrap_or(0);
    let mut countries: Vec<CasualtyMatrixCountry> = Vec::new();
    let mut index: HashMap<Identity, u32> = HashMap::new();
    let mut intern = |identity: Identity, countries: &mut Vec<CasualtyMatrixCountry>| -> u32 {
        if let Some(&at) = index.get(&identity) {
            return at;
        }
        let at = countries.len() as u32;
        let faction = shape.factions.get(&identity);
        let total_dealt = dealt.get(&identity).copied().unwrap_or(Some(0));
        let total_taken = taken.get(&identity).copied().unwrap_or(Some(0));
        countries.push(CasualtyMatrixCountry {
            tag: tag_of(identity).to_owned(),
            slot: identity.0,
            country_id: identity.1,
            shown_hour: shape.shown_hours.get(&identity).copied(),
            faction_id: faction.map(|faction| faction.id.clone()),
            faction_name: faction.and_then(|faction| faction.name.clone()),
            faction_name_missing: faction.and_then(|faction| faction.name_missing.clone()),
            dealt: text(total_dealt),
            taken: text(total_taken),
            missing: (total_dealt.is_none() || total_taken.is_none()).then(|| COUNTER_NOT_RECORDED.to_owned()),
        });
        index.insert(identity, at);
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
        let mut opponents: Vec<Identity> = pairs.keys().filter_map(|&(from, to)| (from == subject).then_some(to).or_else(|| (to == subject).then_some(from))).collect::<HashSet<_>>().into_iter().collect();
        let side = |from: Identity, to: Identity| pairs.get(&(from, to));
        let both = |opponent: Identity| side(subject, opponent).and_then(|sum| sum.dealt).unwrap_or(0) + side(opponent, subject).and_then(|sum| sum.dealt).unwrap_or(0);
        opponents.sort_by(|&a, &b| both(b).cmp(&both(a)).then_with(|| tag_of(a).cmp(tag_of(b))).then(a.cmp(&b)));
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

    let everyone: HashSet<Identity> = pairs.keys().flat_map(|&(from, to)| [from, to]).collect();
    let ranked_top: Option<HashSet<Identity>> = shape.top.filter(|_| shape.chosen.is_none()).map(|limit| {
        let mut ranked: Vec<Identity> = everyone.iter().copied().collect();
        ranked.sort_by(|&a, &b| volume(b).cmp(&volume(a)).then_with(|| tag_of(a).cmp(tag_of(b))).then(a.cmp(&b)));
        ranked.truncate(limit as usize);
        ranked.into_iter().collect()
    });
    let keep = |from: Identity, to: Identity| match (shape.chosen, &ranked_top) {
        (Some(chosen), _) if shape.between => chosen.contains(&from) && chosen.contains(&to),
        (Some(chosen), _) => chosen.contains(&from) || chosen.contains(&to),
        (None, Some(top)) => top.contains(&from) && top.contains(&to),
        (None, None) => true,
    };
    let kept: Vec<(&(Identity, Identity), &PairSum)> = pairs.iter().filter(|&(&(from, to), _)| keep(from, to)).collect();
    let mut shown: HashSet<Identity> = kept.iter().flat_map(|&(&(from, to), _)| [from, to]).collect();
    if let Some(chosen) = shape.chosen {
        shown.extend(chosen.iter().copied().filter(|id| everyone.contains(id)));
    }
    let group_key = |identity: Identity| match shape.factions.get(&identity) {
        Some(faction) => (0u8, faction.name.clone().unwrap_or_default(), faction.id.clone()),
        None => (1u8, String::new(), String::new()),
    };
    let score = |identity: Identity| dealt.get(&identity).copied().flatten().unwrap_or(0);
    let mut ordered: Vec<Identity> = shown.into_iter().collect();
    ordered.sort_by(|&a, &b| group_key(a).cmp(&group_key(b)).then(score(b).cmp(&score(a))).then_with(|| tag_of(a).cmp(tag_of(b))).then(a.cmp(&b)));
    let position: HashMap<Identity, u32> = ordered.iter().enumerate().map(|(at, &id)| (id, at as u32)).collect();
    reply.order = ordered.iter().map(|&id| intern(id, &mut countries)).collect();
    let mut most = 0i128;
    let mut largest = 0i128;
    for &id in &ordered {
        largest = largest.max(score(id));
    }
    for (&(from, to), sum) in kept {
        let (Some(&row), Some(&column)) = (position.get(&from), position.get(&to)) else { continue };
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

fn resolve_selected(holders: &Holders, country: &Country, hour: i32) -> Result<Option<Identity>, String> {
    match holders.at(country, hour).map(|run| run.holder) {
        Some(Holder::Found { slot, id }) => Ok(Some((slot, id))),
        Some(Holder::Unresolved) => Err(holders.not_known(country, hour).unwrap_or_else(|| format!("Who holds {} is not known: {TAG_UNLINKED}", holders.name(country, hour)))),
        None => {
            if matches!(country, Country::Slot(_)) {
                let unlinked = holders.unlinked();
                if unlinked.get(unlinked.partition_point(|&(_, last)| last < hour)).is_some_and(|&(first, _)| first <= hour) {
                    return Err(holders.unnamed(country, hour));
                }
                return Ok(None);
            }
            let runs = holders.runs(country);
            let before_hour = runs.partition_point(|run| run.first <= hour);
            match runs[..before_hour].last() {
                Some(run) => match run.holder {
                    Holder::Found { slot, id } => {
                        if let Some((_, _, reason)) = holders.unknown.iter().find(|&&(first, last, _)| last > run.last && first <= hour) {
                            return Err(format!("Who holds {} is not known: {reason}", holders.name(country, hour)));
                        }
                        Ok(Some((slot, id)))
                    }
                    Holder::Unresolved => Err(format!("Who holds {} is not known: {TAG_UNLINKED}", holders.name(country, hour))),
                },
                None => Ok(None),
            }
        }
    }
}

fn latest_holders(holders: &Holders, hour: i32) -> HashMap<Identity, (&str, i32, i32)> {
    let mut latest: HashMap<Identity, (&str, i32, i32)> = HashMap::new();
    for (tag, runs) in &holders.tags {
        for run in runs {
            if run.first > hour {
                continue;
            }
            if let Holder::Found { slot, id } = run.holder {
                let entry = latest.entry((slot, id)).or_insert((tag.as_str(), run.first, run.last));
                if run.first > entry.1 || (run.first == entry.1 && run.last > entry.2) {
                    *entry = (tag.as_str(), run.first, run.last);
                }
            }
        }
    }
    latest
}

impl AtHour for CasualtyMatrix {
    const KEY: &'static str = "casualty-matrix";
    const TITLE: &'static str = "Casualties between countries";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: Clock = Clock::Boundaries;
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

    fn columns(table: TableId) -> Option<Vec<usize>> {
        match table {
            TableId::CasualtyLifetimes => Some(field_columns(table, &["taken_a", "taken_b"])),
            TableId::CasualtyTotals => Some(field_columns(table, &["partial"])),
            _ => None,
        }
    }

    fn plan_changes(_stage: usize, _edits: &Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        false
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        let mut through = factions_valid_through(&at.context.factions, frame.hour)
            .min(before(at.holders.next_change(frame.hour)));
        if let Some(timeline) = &at.context.timeline {
            if let Some(stretch) = timeline.at(frame.hour) {
                through = through.min(stretch.last);
                if let Some(run) = at.context.runs.iter().find(|run| run.run == stretch.run)
                    && let Some(&next) = run.edges.get(run.edges.partition_point(|&edge| edge <= frame.hour))
                {
                    through = through.min(next.saturating_sub(1));
                }
            } else if let Some(next) = timeline.stretches.iter().find(|stretch| stretch.first > frame.hour) {
                through = through.min(next.first.saturating_sub(1));
            }
        }
        through
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
            let mut edges: Vec<i32> = lifetimes
                .iter()
                .flat_map(|&(_, born, gone)| [i32::try_from(born).ok(), i32::try_from(gone.saturating_add(1)).ok()])
                .flatten()
                .collect();
            edges.sort_unstable();
            edges.dedup();
            let ended = cached_casualty_ends(source, stretch.run)?;
            runs.push(RunCounters { run: stretch.run, lifetimes, edges, ended });
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
        let mut chosen: Option<HashSet<Identity>> = (!params.countries.is_empty()).then(HashSet::new);
        for country in &params.countries {
            match resolve_selected(at.holders, country, at.hour) {
                Ok(Some(id)) => {
                    chosen.as_mut().expect("non-empty countries").insert(id);
                }
                Ok(None) => {}
                Err(reason) => return Ok(empty(Some(reason), warnings)),
            }
        }
        let subject = match &params.subject {
            Some(country) => match resolve_selected(at.holders, country, at.hour) {
                Ok(Some(id)) => Some(id),
                Ok(None) => return Ok(empty(Some("The chosen country holds no tag at this hour".to_owned()), warnings)),
                Err(reason) => return Ok(empty(Some(reason), warnings)),
            },
            None => None,
        };
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
        let latest = latest_holders(at.holders, at.hour);
        let pairs = fold(&counters, |identity| latest.contains_key(&identity), |identity| partial.contains(&identity));
        let mut names: HashMap<Identity, String> = HashMap::new();
        let mut shown_hours: HashMap<Identity, i32> = HashMap::new();
        let factions_by_tag = factions_at(&at.context.factions, at.hour);
        let mut factions: HashMap<Identity, FactionAt> = HashMap::new();
        for identity in pairs.keys().flat_map(|&(a, b)| [a, b]).chain(chosen.iter().flatten().copied()).chain(subject) {
            if let Some(&(tag, _, last)) = latest.get(&identity) {
                names.entry(identity).or_insert_with(|| tag.to_owned());
                if last < at.hour {
                    shown_hours.insert(identity, last);
                } else if let Some(faction) = factions_by_tag.get(tag) {
                    factions.insert(identity, faction.clone());
                }
            }
        }
        let shape = Shape { tags: &names, shown_hours: &shown_hours, chosen: chosen.as_ref(), between: params.between, top: params.top, subject, factions: &factions };
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

    fn tags_map() -> HashMap<Identity, String> {
        [(GER, "GER"), (ENG, "ENG"), (POL, "POL"), (FRA, "FRA")]
            .into_iter()
            .map(|(id, tag)| (id, tag.to_owned()))
            .collect()
    }

    fn known(identity: Identity) -> bool {
        matches!(identity, GER | ENG | POL | FRA)
    }

    fn counter(a: Identity, b: Identity, taken: Option<(i64, i64)>) -> Counter {
        Counter { a, b, taken }
    }

    fn no_factions() -> HashMap<Identity, FactionAt> {
        HashMap::new()
    }

    fn no_shown_hours() -> HashMap<Identity, i32> {
        HashMap::new()
    }

    #[test]
    fn a_relation_counts_what_each_side_took_as_the_other_side_dealt_and_lifetimes_sum() {
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, POL, Some((5, 20)))], known, |_| false);
        assert_eq!(pairs[&(POL, GER)].dealt, Some(105));
        assert_eq!(pairs[&(GER, POL)].dealt, Some(920));
    }

    #[test]
    fn an_unrecorded_counter_makes_the_pair_and_its_totals_missing_never_zero() {
        let tags = tags_map();
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, FRA, None)], known, |_| false);
        let reply = build(&pairs, &Shape { tags: &tags, shown_hours: &no_shown_hours(), chosen: None, between: false, top: None, subject: None, factions: &no_factions() });
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
        let tags = tags_map();
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, ENG, Some((50, 10))), counter(FRA, GER, Some((300, 30)))], known, |_| false);
        let factions: HashMap<Identity, FactionAt> = [(ENG, "1", "Allies"), (FRA, "1", "Allies"), (POL, "1", "Allies")]
            .into_iter()
            .map(|(id, fid, name)| (id, FactionAt { id: fid.to_owned(), name: Some(name.to_owned()), name_missing: None }))
            .collect();
        let reply = build(&pairs, &Shape { tags: &tags, shown_hours: &no_shown_hours(), chosen: None, between: false, top: None, subject: None, factions: &factions });
        let order: Vec<&str> = reply.order.iter().map(|&at| reply.countries[at as usize].tag.as_str()).collect();
        assert_eq!(order, ["POL", "ENG", "FRA", "GER"]);
        assert_eq!(reply.largest_dealt, "1210");
        assert_eq!(reply.most, "900");
    }

    #[test]
    fn between_keeps_only_pairs_of_chosen_countries() {
        let tags = tags_map();
        let pairs = fold(&[counter(GER, POL, Some((1, 2))), counter(GER, ENG, Some((3, 4)))], known, |_| false);
        let chosen: HashSet<Identity> = [GER, POL].into_iter().collect();
        let reply = build(&pairs, &Shape { tags: &tags, shown_hours: &no_shown_hours(), chosen: Some(&chosen), between: true, top: None, subject: None, factions: &no_factions() });
        assert_eq!(reply.order.len(), 2);
        assert_eq!(reply.cells.len(), 2);
        let touching = build(&pairs, &Shape { tags: &tags, shown_hours: &no_shown_hours(), chosen: Some(&chosen), between: false, top: None, subject: None, factions: &no_factions() });
        assert_eq!(touching.order.len(), 3);
    }

    #[test]
    fn one_country_lists_opponents_by_casualties_both_ways() {
        let tags = tags_map();
        let pairs = fold(&[counter(GER, POL, Some((100, 900))), counter(GER, ENG, Some((2000, 10))), counter(GER, FRA, Some((1, 1)))], known, |identity| identity == FRA);
        let reply = build(&pairs, &Shape { tags: &tags, shown_hours: &no_shown_hours(), chosen: None, between: false, top: None, subject: Some(GER), factions: &no_factions() });
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

    #[test]
    fn reused_tag_keeps_each_holder_identity_separate_without_sharing_factions_or_totals() {
        const DEAD_POL: Identity = (3, 30);
        const LIVE_POL: Identity = (5, 50);
        let tags: HashMap<Identity, String> = [(GER, "GER"), (DEAD_POL, "POL"), (LIVE_POL, "POL")]
            .into_iter()
            .map(|(id, tag)| (id, tag.to_owned()))
            .collect();
        let pairs = fold(&[counter(GER, DEAD_POL, Some((100, 900))), counter(GER, LIVE_POL, Some((25, 40)))], |id| tags.contains_key(&id), |_| false);
        let factions: HashMap<Identity, FactionAt> = [(LIVE_POL, FactionAt { id: "1".to_owned(), name: Some("Allies".to_owned()), name_missing: None })].into_iter().collect();
        let shown_hours: HashMap<Identity, i32> = [(DEAD_POL, 104)].into_iter().collect();
        let reply = build(&pairs, &Shape { tags: &tags, shown_hours: &shown_hours, chosen: None, between: false, top: None, subject: None, factions: &factions });
        assert_eq!(reply.countries.len(), 3);
        let live = reply.countries.iter().find(|country| (country.slot, country.country_id) == LIVE_POL).unwrap();
        let dead = reply.countries.iter().find(|country| (country.slot, country.country_id) == DEAD_POL).unwrap();
        assert_eq!((live.tag.as_str(), live.shown_hour, live.dealt.as_deref(), live.taken.as_deref(), live.faction_id.as_deref()), ("POL", None, Some("25"), Some("40"), Some("1")));
        assert_eq!((dead.tag.as_str(), dead.shown_hour, dead.dealt.as_deref(), dead.taken.as_deref(), dead.faction_id.as_deref()), ("POL", Some(104), Some("100"), Some("900"), None));
        let subject_live = build(&pairs, &Shape { tags: &tags, shown_hours: &shown_hours, chosen: None, between: false, top: None, subject: Some(LIVE_POL), factions: &factions });
        assert_eq!(subject_live.opponents.len(), 1);
        assert_eq!(subject_live.opponents[0].dealt.as_deref(), Some("25"));
        assert_eq!(subject_live.opponents[0].taken.as_deref(), Some("40"));
    }

    #[test]
    fn unresolved_holder_never_falls_back_to_an_earlier_holder_identity() {
        use dashboard_query::identity::HolderRun;
        let mut tags = BTreeMap::new();
        tags.insert(
            "D04".to_owned(),
            vec![
                HolderRun { first: 100, last: 109, holder: Holder::Found { slot: 3, id: 30 }, starts_segment: true },
                HolderRun { first: 110, last: 119, holder: Holder::Unresolved, starts_segment: true },
            ],
        );
        tags.insert(
            "POL".to_owned(),
            vec![HolderRun { first: 100, last: 109, holder: Holder::Found { slot: 4, id: 40 }, starts_segment: true }],
        );
        let holders = Holders::new(tags, vec![(120, 129, "country links unread".to_owned())]);
        let d04 = Country::Tag("D04".to_owned());
        let pol = Country::Tag("POL".to_owned());
        assert_eq!(resolve_selected(&holders, &d04, 105), Ok(Some((3, 30))));
        assert!(resolve_selected(&holders, &d04, 115).unwrap_err().contains(TAG_UNLINKED), "unresolved current run refuses instead of falling back");
        assert!(resolve_selected(&holders, &pol, 125).unwrap_err().contains("country links unread"), "recorded unknown span refuses instead of falling back");
        assert_eq!(resolve_selected(&holders, &pol, 115), Ok(Some((4, 40))), "known absence still resolves the last proven holder");
        assert!(resolve_selected(&holders, &pol, 130).unwrap_err().contains("country links unread"), "unreadable holder span between last witness and query refuses instead of falling back");
        assert!(resolve_selected(&holders, &d04, 130).unwrap_err().contains(TAG_UNLINKED), "known absence after an unresolved run refuses instead of skipping to an older holder");
    }

    #[test]
    fn casualty_matrix_holds_quiet_hours_and_refreshes_on_casualty_lifetime_or_holder_changes() {
        use dashboard_base::{
            SourceKey,
            products::at_hour::{exact_panel, frame_panel},
            test_fixtures::{CAMPAIGN, materialized_frame, played, records, source, streamed},
        };
        use replay::campaign::Replay;
        use replay_data::rows::{CountryLinkRow, CountryRow, TableRows, WarRelationRow};
        use replay_frame::frame::TableView;
        use serde_json::json;

        let frames: Vec<_> = (100..=108)
            .map(|hour| {
                let mut f = source(1, hour, None);
                *f.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
                    CountryRow { id: 1, canonical_slot: 1, tag: b"GER".to_vec(), ..Default::default() },
                    CountryRow { id: if hour >= 106 { 4 } else { 2 }, canonical_slot: if hour >= 106 { 4 } else { 2 }, tag: b"POL".to_vec(), ..Default::default() },
                ]));
                *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow { id: 1, canonical_slot: 1 },
                    CountryLinkRow { id: if hour >= 106 { 4 } else { 2 }, canonical_slot: if hour >= 106 { 4 } else { 2 } },
                ]));
                let war = if hour <= 104 {
                    vec![WarRelationRow {
                        first_country: 1,
                        second_country: 2,
                        start_hour: 100,
                        end_hour: 0,
                        first_casualties: if hour >= 103 { 500 } else { 100 },
                        second_casualties: if hour >= 103 { 900 } else { 200 },
                        ..Default::default()
                    }]
                } else if hour >= 107 {
                    vec![WarRelationRow {
                        first_country: 1,
                        second_country: 4,
                        start_hour: 107,
                        end_hour: 0,
                        first_casualties: 15,
                        second_casualties: 30,
                        ..Default::default()
                    }]
                } else {
                    Vec::new()
                };
                *f.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(war));
                f
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let source = Source::from_replay(
            SourceKey { source_id: "cm-exact".into(), revision: "1".into() },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load = LoadContext { content: None };
        let params = json!({"branchId": "1"});
        let panel = exact_panel::<CasualtyMatrix>(&source, &load, &params).unwrap();
        let exact = panel.exact().unwrap();
        let mut whole = frame_panel::<CasualtyMatrix>(&source, &load, &params).unwrap();
        let steps = played(source.replay_ref(), exact, 100, 108);
        for step in &steps {
            let expected = streamed(&materialized_frame(&mut *whole, source.replay_ref(), 1, step.hour), step.fresh);
            assert_eq!(streamed(&step.value, step.fresh), expected, "casualty-matrix at hour {}", step.hour);
        }
        for hour in [101, 102, 104, 108] {
            assert_eq!(steps[(hour - 100) as usize].product_calls, 0, "quiet hour {hour} should hold");
        }
        let end_countries = steps[8].value.as_ref().as_ref().unwrap()["countries"].as_array().unwrap();
        let pols: Vec<_> = end_countries.iter().filter(|c| c["tag"] == "POL").collect();
        assert_eq!(pols.len(), 2, "reused POL tag preserves both holder identities");
        let old_pol = pols.iter().find(|c| c["slot"] == 2 && c["countryId"] == 2).unwrap();
        let new_pol = pols.iter().find(|c| c["slot"] == 4 && c["countryId"] == 4).unwrap();
        assert_eq!(old_pol["shownHour"], 105, "past holder carries its latest witnessed hour");
        assert!(new_pol["shownHour"].is_null(), "live holder leaves shownHour null so quiet hours hold");
        assert_eq!((exact.cold)(107, &AtomicBool::new(false)), materialized_frame(&mut *whole, source.replay_ref(), 1, 107));
    }
}
