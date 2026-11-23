
pub mod civil_wars;
pub mod history;
pub mod state;

use dashboard_query::{
    Query,
    identity::{Holder, Holders},
    read::Runs,
    timeline::Stretch,
};
use replay_columnar::summary::Source;
use replay_data::rows::TableId;
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PoliticsRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HolderTag {
    pub hour: i32,
    pub tag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Holding {
    pub first: i32,
    pub last: i32,
    pub slot: u32,
    pub id: u32,
    pub tag: String,
}

pub(crate) fn holdings(holders: &Holders) -> BTreeMap<u32, Vec<Holding>> {
    let mut out: BTreeMap<u32, Vec<Holding>> = BTreeMap::new();
    for (tag, runs) in &holders.tags {
        for run in runs {
            if let Holder::Found { slot, id } = run.holder {
                out.entry(slot).or_default().push(Holding { first: run.first, last: run.last, slot, id, tag: tag.clone() });
            }
        }
    }
    for held in out.values_mut() {
        held.sort_by(|a, b| (a.first, &a.tag).cmp(&(b.first, &b.tag)));
    }
    out
}

pub(crate) fn tags_of(held: &[Holding]) -> Vec<HolderTag> {
    let mut out: Vec<HolderTag> = Vec::new();
    for holding in held {
        if out.last().is_none_or(|last| last.tag != holding.tag) {
            out.push(HolderTag { hour: holding.first, tag: holding.tag.clone() });
        }
    }
    out
}

pub(crate) fn tag_at(tags: &[HolderTag], hour: i32) -> Option<&str> {
    let at = tags.partition_point(|entry| entry.hour <= hour);
    at.checked_sub(1).map(|at| tags[at].tag.as_str())
}

pub(crate) fn by_id(held: &BTreeMap<u32, Vec<Holding>>) -> HashMap<u32, Vec<&Holding>> {
    let mut out: HashMap<u32, Vec<&Holding>> = HashMap::new();
    for holding in held.values().flatten() {
        out.entry(holding.id).or_default().push(holding);
    }
    out
}

pub(crate) fn stretch_runs(query: &Query<'_>, stretch: &Stretch, table: TableId, column: &str) -> io::Result<HashMap<u128, Runs>> {
    let column = dashboard_declare::field(table, column)?;
    dashboard_query::read::runs(query.replay.recording(), stretch.run, table, Source::Cell(column), None, stretch.first, stretch.last)
}

pub(crate) fn within(runs: Option<&Runs>, first: i32, last: i32) -> Vec<(i32, i32, i64)> {
    runs.into_iter()
        .flatten()
        .filter_map(|&(from, to, value)| {
            let (from, to) = (from.max(first), to.min(last));
            Some((from, to, value?)).filter(|_| from <= to)
        })
        .collect()
}

pub(crate) fn decimal(runs: impl IntoIterator<Item = (i32, i32, i128)>) -> Vec<PoliticsRun> {
    let mut out: Vec<PoliticsRun> = Vec::new();
    for (first, last, value) in runs {
        let value = value.to_string();
        match out.last_mut() {
            Some(held) if held.value == value && i64::from(held.last_hour) + 1 == i64::from(first) => held.last_hour = last,
            _ => out.push(PoliticsRun { first_hour: first, last_hour: last, value }),
        }
    }
    out
}

pub(crate) fn pieces(first: i32, last: i32, lists: &[&[(i32, i32, i64)]]) -> Vec<(i32, i32)> {
    let mut cuts: Vec<i32> = vec![first];
    for list in lists {
        for &(from, to, _) in *list {
            cuts.push(from);
            if to < last {
                cuts.push(to + 1);
            }
        }
    }
    cuts.retain(|&cut| first <= cut && cut <= last);
    cuts.sort_unstable();
    cuts.dedup();
    cuts.iter().enumerate().map(|(at, &from)| (from, cuts.get(at + 1).map_or(last, |next| next - 1))).collect()
}

pub(crate) fn value_at(list: &[(i32, i32, i64)], hour: i32) -> Option<i64> {
    let at = list.partition_point(|&(_, to, _)| to < hour);
    list.get(at).filter(|&&(from, ..)| from <= hour).map(|&(.., value)| value)
}

pub(crate) fn token(bytes: &[u8]) -> String {
    String::from_utf8_lossy(content_reader_parser::parser::c_string(bytes)).into_owned()
}

#[cfg(test)]
mod tests;
