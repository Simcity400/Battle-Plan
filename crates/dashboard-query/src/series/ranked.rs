
use super::{Line, NavigationKey, SeriesRead, SeriesState, hour_of, invalid};
use crate::identity::Country;
use serde::{Deserialize, Serialize};
use std::io;
use ts_rs::TS;

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
    #[doc = " Why the row's country is not named at the hour, for a recorded row whose holder is not resolved: it is ranked by its recorded readings."]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unnamed: Option<String>,
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

fn reading_interval(line: &Line<'_, '_>, state: &SeriesState, within: (i32, i32)) -> io::Result<(i32, i32)> {
    let hour = state.hour;
    if let Some(run) = &state.run {
        let first = if line.series.drops && hour > run.first_hour { run.first_hour + 1 } else { run.first_hour };
        return Ok((first, if state.drop.is_some() { hour } else { run.last_hour }));
    }
    let timeline = &line.query.timeline;
    if let Some(gap) = timeline.unheld.iter().find(|gap| gap.first <= hour && hour <= gap.last) {
        return Ok((gap.first, gap.last));
    }
    let Some(stretch) = timeline.at(hour) else { return Ok((hour, hour)) };
    let first = stretch.first.max(within.0);
    let last = stretch.last.min(within.1);
    let parts = line.parts(state.country.clone(), first, last);
    let gaps = line.missing_for_tile(state.country.clone(), first, last, &parts)?;
    Ok(gaps.iter().find(|gap| gap.first_hour <= hour && hour <= gap.last_hour && Some(&gap.reason) == state.missing.as_ref())
        .map_or((hour, hour), |gap| (gap.first_hour, gap.last_hour)))
}

pub(super) fn rank(line: &Line<'_, '_>, details: &[Line<'_, '_>], branch_id: &str, ask: Ask<'_>) -> io::Result<SeriesRankReply> {
    line.owners_only("a rank")?;
    let hour = hour_of(ask.hour, "hour")?;
    let mut held = line.query.holders.interval(hour);
    let named: Vec<Country> = match ask.group {
        None => ask.countries.to_vec(),
        Some(_) if ask.top.is_some() || !ask.countries.is_empty() => return Err(invalid("A rank names a group, a top or countries, one of them")),
        Some((group, resolve)) => {
            let members = resolve(group, hour)?;
            held = (held.0.max(members.from.min(hour)), held.1.min(members.until.max(hour)));
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
            Ok(ranked) => (ranked, None, Some(n as usize)),
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
    let unnamed: Vec<(usize, u32)> = match ask.top {
        Some(_) => asked.iter().enumerate().filter_map(|(at, country)| match country {
            Country::Slot(slot) if line.query.holders.tag_at(country, hour).is_none() => Some((at, *slot)),
            _ => None,
        }).collect(),
        None => Vec::new(),
    };
    let mut columns = vec![line.states(&reads, None)?];
    for detail in details {
        columns.push(detail.states(&reads, None)?);
    }
    if !unnamed.is_empty() {
        let unnamed_slots: Vec<u32> = unnamed.iter().map(|&(_, slot)| slot).collect();
        for (column, reader) in columns.iter_mut().zip(std::iter::once(line).chain(details)) {
            for (&(at, _), state) in unnamed.iter().zip(reader.unnamed_states(&unnamed_slots, hour)?) {
                column[at] = state;
            }
        }
    }
    let mut named: Vec<Option<String>> = vec![None; asked.len()];
    let mut slots = slots;
    for &(at, slot) in &unnamed {
        named[at] = Some(line.query.holders.unnamed(&asked[at], hour));
        slots[at] = Some(slot);
    }
    let mut order: Vec<usize> = (0..asked.len()).collect();
    if ask.top.is_none() {
        let value = |at: usize| columns[0][at].run.as_ref().and_then(|run| run.value_raw.parse::<i64>().ok());
        order.sort_by(|&a, &b| match (value(a), value(b)) {
            (Some(x), Some(y)) => (if ask.least { x.cmp(&y) } else { y.cmp(&x) }).then_with(|| asked[a].cmp(&asked[b])),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(&b),
        });
        refused = order.iter().find(|&&at| value(at).is_none()).map(|&at| columns[0][at].missing.clone().unwrap_or_else(|| super::board::measure_unrecorded(&asked[at].to_string(), line.measure.name)));
    }
    for (column, reader) in columns.iter().zip(std::iter::once(line).chain(details)) {
        for state in column {
            let (from, until) = reading_interval(reader, state, held)?;
            held = (held.0.max(from), held.1.min(until));
        }
    }
    let (from, until) = if let Some(n) = top {
        line.rank_holds(hour, n, ask.least, held, |at| at == asked.as_slice())?
    } else if ask.top.is_some() {
        line.query.timeline.unheld.iter().find(|gap| gap.first <= hour && hour <= gap.last)
            .map_or((hour, hour), |gap| (gap.first.max(held.0), gap.last.min(held.1)))
    } else {
        held
    };
    let mut measures = vec![SeriesRankMeasure { name: line.measure_name().to_owned(), decimals: line.decimals() }];
    measures.extend(details.iter().map(|detail| SeriesRankMeasure { name: detail.measure_name().to_owned(), decimals: detail.decimals() }));
    let mut cells: Vec<std::vec::IntoIter<SeriesState>> = columns.into_iter().map(Vec::into_iter).collect();
    let mut readings: Vec<Vec<SeriesState>> = (0..asked.len()).map(|_| cells.iter_mut().map(|column| column.next().expect("one reading per asked country")).collect()).collect();
    let rows = order.into_iter().map(|at| SeriesRankRow { country: asked[at].clone(), slot: slots[at], unnamed: named[at].take(), readings: std::mem::take(&mut readings[at]) }).collect();
    Ok(SeriesRankReply { branch_id: branch_id.to_owned(), hour, measures, rows, refused, from, until })
}

pub(super) fn stepped(line: &Line<'_, '_>, origin: Option<i32>, key: NavigationKey, bounds: (i32, i32), n: usize) -> io::Result<Vec<Country>> {
    let at = match (origin, key) {
        (Some(origin), _) => origin.clamp(bounds.0, bounds.1),
        (None, NavigationKey::End | NavigationKey::ArrowLeft) => bounds.1,
        (None, _) => bounds.0,
    };
    Ok(line.rank_at(at, n, false)?.unwrap_or_default())
}
