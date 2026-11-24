
use super::{Line, MissingSpan, NavigationKey, SeriesRead, SeriesState, hour_of, invalid};
use crate::{chart::ChartSummary, identity::Country, tile::TilePart};
use serde::{Deserialize, Serialize};
use std::io;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RankedSpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRankedLine {
    pub tag: String,
    #[doc = " Its holders over the view."]
    pub parts: Vec<TilePart>,
    pub summary: ChartSummary,
    pub membership: Vec<RankedSpan>,
    pub missing: Vec<MissingSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRankedReply {
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub lines: Vec<SeriesRankedLine>,
}

const WIDTH: u32 = 8192;

pub(super) fn top(line: &Line<'_, '_>, top: Option<u32>) -> io::Result<Option<usize>> {
    match (top, line.series.parts) {
        (None, _) => Ok(None),
        (Some(_), Some(_)) => Ok(None),
        (Some(0), None) => Err(invalid(format!("A ranked {} names a top of at least 1", line.series.kind))),
        (Some(n), None) => Ok(Some(n as usize)),
    }
}

pub const PRODUCT: &str = "series-rank";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRankMeasure {
    pub name: String,
    pub decimals: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRankRow {
    pub country: Country,
    #[doc = " The recorded slot holding the country at the hour; none where its holder is not resolved."]
    pub slot: Option<u32>,
    #[doc = " One reading per measure of the reply, in the reply's order, each with its own missing reason."]
    pub readings: Vec<SeriesState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRankReply {
    pub branch_id: String,
    pub hour: i32,
    #[doc = " The ranking measure first, then each other measure asked."]
    pub measures: Vec<SeriesRankMeasure>,
    #[doc = " In the product's order: the ranked rows, then each asked country whose ranking reading is missing, in the asked order."]
    pub rows: Vec<SeriesRankRow>,
    #[doc = " Why the order is not a rank of everything asked (the population, or an asked country's ranking reading, is not known). A refused top holds no rows."]
    pub refused: Option<String>,
    #[doc = " The hours over which this reply holds unchanged; ask again outside them."]
    pub from: i32,
    pub until: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum GroupKind {
    Humans,
    Faction,
    War,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum GroupMembership {
    #[default]
    Hour,
    Ever,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupSelection {
    #[serde(rename = "type")]
    pub kind: GroupKind,
    #[doc = " The recorded faction id, for a faction."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    #[doc = " The country whose enemies are the group, for a war."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tag: Option<String>,
    #[doc = " Members at the asked hour, or every country that was one at or before it."]
    #[serde(default)]
    pub membership: GroupMembership,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Members {
    pub tags: Vec<String>,
    pub refused: Option<String>,
    pub from: i32,
    pub until: i32,
}

pub type Resolve<'r> = &'r dyn Fn(&GroupSelection, i32) -> io::Result<Members>;

pub struct Ask<'a> {
    pub hour: Option<f64>,
    pub top: Option<u32>,
    pub least: bool,
    pub countries: &'a [Country],
    pub group: Option<(&'a GroupSelection, Resolve<'a>)>,
}

pub(super) fn rank(line: &Line<'_, '_>, details: &[Line<'_, '_>], branch_id: &str, ask: Ask<'_>) -> io::Result<SeriesRankReply> {
    line.owners_only("a rank")?;
    let hour = hour_of(ask.hour, "hour")?;
    let mut held = (i32::MIN, i32::MAX);
    let named: Vec<Country> = match ask.group {
        None => ask.countries.to_vec(),
        Some(_) if ask.top.is_some() || !ask.countries.is_empty() => return Err(invalid("A rank names a group, a top or countries, one of them")),
        Some((group, resolve)) => {
            let members = resolve(group, hour)?;
            held = (members.from.min(hour), members.until.max(hour));
            if members.refused.is_some() || members.tags.is_empty() {
                let mut measures = vec![SeriesRankMeasure { name: line.measure_name().to_owned(), decimals: line.decimals() }];
                measures.extend(details.iter().map(|detail| SeriesRankMeasure { name: detail.measure_name().to_owned(), decimals: detail.decimals() }));
                let (from, until) = if members.refused.is_some() { (hour, hour) } else { held };
                return Ok(SeriesRankReply { branch_id: branch_id.to_owned(), hour, measures, rows: Vec::new(), refused: members.refused, from, until });
            }
            members.tags.into_iter().map(Country::from).collect()
        }
    };
    let mut countries: Vec<Country> = Vec::with_capacity(named.len());
    for country in named {
        if !countries.contains(&country) {
            countries.push(country);
        }
    }
    let (asked, mut refused, top) = match (ask.top, countries.is_empty()) {
        (Some(_), false) => return Err(invalid("A rank names a top or countries, not both")),
        (None, true) => return Err(invalid("A rank names a top or countries")),
        (Some(0), true) => return Err(invalid(format!("A ranked {} names a top of at least 1", line.series.kind))),
        (Some(n), true) => match line.rank_at(hour, n as usize, ask.least)? {
            Ok(tags) => (tags.into_iter().map(Country::from).collect(), None, Some(n as usize)),
            Err(reason) => (Vec::new(), Some(reason), None),
        },
        (None, false) => (countries, None, None),
    };
    let mut held_slots = std::collections::HashSet::new();
    let asked: Vec<(Country, Option<u32>)> = asked
        .into_iter()
        .map(|country| {
            let slot = line.query.holders.slot_at(&country, hour);
            (country, slot)
        })
        .filter(|(_, slot)| slot.is_none_or(|slot| held_slots.insert(slot)))
        .collect();
    let (asked, slots): (Vec<Country>, Vec<Option<u32>>) = asked.into_iter().unzip();
    let reads: Vec<SeriesRead> = asked.iter().map(|country| SeriesRead { country: country.clone(), hour, part: None }).collect();
    let mut columns = vec![line.states(&reads, None)?];
    for detail in details {
        columns.push(detail.states(&reads, None)?);
    }
    let mut order: Vec<usize> = (0..asked.len()).collect();
    if ask.top.is_none() {
        let value = |at: usize| columns[0][at].run.as_ref().and_then(|run| run.value_raw.parse::<i64>().ok());
        order.sort_by(|&a, &b| match (value(a), value(b)) {
            (Some(x), Some(y)) => (if ask.least { x.cmp(&y) } else { y.cmp(&x) }).then_with(|| asked[a].to_string().cmp(&asked[b].to_string())),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(&b),
        });
        refused = order.iter().find(|&&at| value(at).is_none()).map(|&at| columns[0][at].missing.clone().unwrap_or_else(|| super::board::measure_unrecorded(&asked[at].to_string(), line.measure.name)));
    }
    for state in columns.iter().flatten() {
        held = match &state.run {
            Some(run) => (held.0.max(run.first_hour), held.1.min(run.last_hour)),
            None => (hour, hour),
        };
    }
    let (from, until) = match (top, &refused) {
        (Some(n), None) => {
            let ranked: Vec<String> = asked.iter().map(ToString::to_string).collect();
            line.rank_holds(hour, n, ask.least, held, |at| at == ranked.as_slice())?
        }
        (None, None) => (held.0.min(hour), held.1.max(hour)),
        _ => (hour, hour),
    };
    let mut measures = vec![SeriesRankMeasure { name: line.measure_name().to_owned(), decimals: line.decimals() }];
    measures.extend(details.iter().map(|detail| SeriesRankMeasure { name: detail.measure_name().to_owned(), decimals: detail.decimals() }));
    let mut cells: Vec<std::vec::IntoIter<SeriesState>> = columns.into_iter().map(Vec::into_iter).collect();
    let mut readings: Vec<Vec<SeriesState>> = (0..asked.len()).map(|_| cells.iter_mut().map(|column| column.next().expect("one reading per asked country")).collect()).collect();
    let rows = order.into_iter().map(|at| SeriesRankRow { country: asked[at].clone(), slot: slots[at], readings: std::mem::take(&mut readings[at]) }).collect();
    Ok(SeriesRankReply { branch_id: branch_id.to_owned(), hour, measures, rows, refused, from, until })
}

pub(super) fn summary(line: &Line<'_, '_>, branch_id: &str, from: Option<f64>, to: Option<f64>, width: Option<u32>, n: usize) -> io::Result<SeriesRankedReply> {
    let (from, to) = (hour_of(from, "from")?, hour_of(to, "to")?);
    if from > to {
        return Err(invalid("Invalid view"));
    }
    let width = width.filter(|width| (1..=WIDTH).contains(width)).ok_or_else(|| invalid("Invalid width"))?;
    let mut lines = Vec::new();
    for ranked in line.ranked(from, to, width, n)? {
        let parts = line
            .parts(&ranked.tag, from, to)
            .into_iter()
            .map(|part| TilePart { first_hour: part.first, last_hour: part.last, slot: part.slot, country_id: part.id, segment_first_hour: part.segment_first })
            .collect();
        lines.push(SeriesRankedLine {
            tag: ranked.tag,
            parts,
            summary: ranked.summary,
            membership: ranked.membership.into_iter().map(|(first_hour, last_hour)| RankedSpan { first_hour, last_hour }).collect(),
            missing: ranked.missing,
        });
    }
    Ok(SeriesRankedReply { branch_id: branch_id.to_owned(), measure: line.measure_name().to_owned(), decimals: line.decimals(), lines })
}

pub(super) fn stepped(line: &Line<'_, '_>, origin: Option<i32>, key: NavigationKey, bounds: (i32, i32), n: usize) -> io::Result<Vec<String>> {
    let at = match (origin, key) {
        (Some(origin), _) => origin.clamp(bounds.0, bounds.1),
        (None, NavigationKey::End | NavigationKey::ArrowLeft) => bounds.1,
        (None, _) => bounds.0,
    };
    Ok(line.rank_at(at, n, false)?.unwrap_or_default())
}
