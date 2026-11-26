
use crate::{read, timeline::Timeline};
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{TableId, TagHolderRow};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, io, sync::OnceLock};
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}


#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize, TS)]
#[serde(untagged)]
#[ts(rename = "CountryRef")]
pub enum Country {
    Slot(u32),
    Tag(String),
}

pub const NOT_IN_HISTORY: &str = "This country is not in this branch";
pub const NOT_HELD_AT_HOUR: &str = "This country holds no tag at this hour";
pub const TAG_UNLINKED: &str = "a recorded tag's country was not resolved at this hour";

pub const SLOT_MARK: char = '\0';

impl Country {
    pub fn key(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Tag(tag) => std::borrow::Cow::Borrowed(tag),
            Self::Slot(slot) => std::borrow::Cow::Owned(format!("{SLOT_MARK}{slot}")),
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Tag(tag) => Some(tag),
            Self::Slot(_) => None,
        }
    }
}

impl From<&str> for Country {
    fn from(tag: &str) -> Self {
        Self::Tag(tag.to_owned())
    }
}

impl From<String> for Country {
    fn from(tag: String) -> Self {
        Self::Tag(tag)
    }
}

impl From<&String> for Country {
    fn from(tag: &String) -> Self {
        Self::Tag(tag.clone())
    }
}

impl From<&Country> for Country {
    fn from(country: &Country) -> Self {
        country.clone()
    }
}

impl fmt::Display for Country {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Slot(slot) => write!(f, "{slot}"),
            Self::Tag(tag) => f.write_str(tag),
        }
    }
}

pub trait Key {
    fn runs<'h>(&self, holders: &'h Holders) -> &'h [HolderRun];
}

impl Key for str {
    fn runs<'h>(&self, holders: &'h Holders) -> &'h [HolderRun] {
        holders.tags.get(self).map_or(&[], Vec::as_slice)
    }
}

impl Key for String {
    fn runs<'h>(&self, holders: &'h Holders) -> &'h [HolderRun] {
        self.as_str().runs(holders)
    }
}

impl<K: Key + ?Sized> Key for &K {
    fn runs<'h>(&self, holders: &'h Holders) -> &'h [HolderRun] {
        (**self).runs(holders)
    }
}

impl Key for Country {
    fn runs<'h>(&self, holders: &'h Holders) -> &'h [HolderRun] {
        match self {
            Self::Tag(tag) => tag.runs(holders),
            Self::Slot(slot) => holders.slots().get(slot).map_or(&[], |held| held.runs.as_slice()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Holder {
    Found { slot: u32, id: u32 },
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HolderRun {
    pub first: i32,
    pub last: i32,
    pub holder: Holder,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Holders {
    pub tags: BTreeMap<String, Vec<HolderRun>>,
    pub unknown: Vec<(i32, i32, String)>,
    slots: OnceLock<BTreeMap<u32, SlotHistory>>,
    edges: OnceLock<Vec<i32>>,
    unlinked: OnceLock<Vec<(i32, i32)>>,
    unknown_runs: OnceLock<Vec<HolderRun>>,
}

#[derive(Clone, Debug, Default)]
struct SlotHistory {
    runs: Vec<HolderRun>,
    tags: Vec<(i32, i32, u32, String)>,
}

impl Holders {
    pub fn new(tags: BTreeMap<String, Vec<HolderRun>>, unknown: Vec<(i32, i32, String)>) -> Self {
        Self { tags, unknown, slots: OnceLock::new(), edges: OnceLock::new(), unlinked: OnceLock::new(), unknown_runs: OnceLock::new() }
    }

    fn edges(&self) -> &[i32] {
        self.edges.get_or_init(|| {
            let runs = self.tags.values().flatten().map(|run| (run.first, run.last));
            let unknown = self.unknown.iter().map(|(first, last, _)| (*first, *last));
            let mut edges: Vec<i32> = runs.chain(unknown).flat_map(|(first, last)| [first, last.saturating_add(1)]).collect();
            edges.sort_unstable();
            edges.dedup();
            edges
        })
    }

    pub fn interval(&self, hour: i32) -> (i32, i32) {
        let edges = self.edges();
        let at = edges.partition_point(|&edge| edge <= hour);
        (edges[..at].last().copied().unwrap_or(i32::MIN), edges.get(at).map_or(i32::MAX, |edge| edge - 1))
    }

    pub fn next_change(&self, hour: i32) -> Option<i32> {
        let edges = self.edges();
        edges.get(edges.partition_point(|&edge| edge <= hour)).copied()
    }

    fn slots(&self) -> &BTreeMap<u32, SlotHistory> {
        self.slots.get_or_init(|| slot_histories(&self.tags))
    }

    fn unlinked(&self) -> &[(i32, i32)] {
        self.unlinked.get_or_init(|| {
            let mut spans: Vec<(i32, i32)> = self.tags.values().flatten().filter(|run| run.holder == Holder::Unresolved).map(|run| (run.first, run.last)).collect();
            spans.sort_unstable();
            let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
            for (first, last) in spans {
                match out.last_mut() {
                    Some(held) if i64::from(first) <= i64::from(held.1) + 1 => held.1 = held.1.max(last),
                    _ => out.push((first, last)),
                }
            }
            out
        })
    }

    pub fn naming_spans(&self, country: &Country) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = self.unknown.iter().map(|&(first, last, _)| (first, last)).chain(country.runs(self).iter().map(|run| (run.first, run.last))).collect();
        if let Country::Slot(slot) = country {
            out.extend_from_slice(self.unlinked());
            out.extend(self.slots().get(slot).into_iter().flat_map(|held| held.tags.iter().map(|&(first, last, ..)| (first, last))));
        }
        out
    }

    pub fn runs<K: Key + ?Sized>(&self, country: &K) -> &[HolderRun] {
        country.runs(self)
    }

    pub fn at<K: Key + ?Sized>(&self, country: &K, hour: i32) -> Option<&HolderRun> {
        let runs = country.runs(self);
        let at = runs.partition_point(|run| run.last < hour);
        runs.get(at).filter(|run| run.first <= hour).or_else(|| self.unknown_at(hour))
    }

    fn unknown_at(&self, hour: i32) -> Option<&HolderRun> {
        let runs = self.unknown_runs.get_or_init(|| {
            let mut runs: Vec<HolderRun> = self.unknown.iter().map(|&(first, last, _)| HolderRun { first, last, holder: Holder::Unresolved, starts_segment: true }).collect();
            runs.sort_unstable_by_key(|run| run.first);
            runs
        });
        let at = runs.partition_point(|run| run.last < hour);
        runs.get(at).filter(|run| run.first <= hour)
    }

    pub fn tag_at<'a>(&'a self, country: &'a Country, hour: i32) -> Option<&'a str> {
        match country {
            Country::Tag(tag) => Some(tag),
            Country::Slot(slot) => self.slot_tag(*slot, hour),
        }
    }

    pub fn slot_at(&self, country: &Country, hour: i32) -> Option<u32> {
        match country {
            Country::Slot(slot) => Some(*slot),
            Country::Tag(_) => match self.at(country, hour)?.holder {
                Holder::Found { slot, .. } => Some(slot),
                Holder::Unresolved => None,
            },
        }
    }

    pub fn goes_by(&self, country: &Country, tag: &str, hour: i32) -> bool {
        match country {
            Country::Tag(chosen) => chosen == tag,
            Country::Slot(slot) => matches!(self.at(tag, hour).map(|run| run.holder), Some(Holder::Found { slot: held, .. }) if held == *slot),
        }
    }

    pub fn name(&self, country: &Country, hour: i32) -> String {
        self.label(country, hour).unwrap_or_else(|| country.key().into_owned())
    }

    pub fn label(&self, country: &Country, hour: i32) -> Option<String> {
        self.tag_at(country, hour).map(str::to_owned)
    }

    pub fn unknown_reason(&self, hour: i32) -> Option<&str> {
        self.unknown.iter().find(|(first, last, _)| *first <= hour && hour <= *last).map(|(_, _, reason)| reason.as_str())
    }

    pub fn not_known(&self, country: &Country, hour: i32) -> Option<String> {
        self.unknown_reason(hour).map(|reason| who_holds(&self.name(country, hour), reason))
    }

    pub fn not_known_as(&self, name: &str, hour: i32) -> Option<String> {
        self.unknown_reason(hour).map(|reason| who_holds(name, reason))
    }

    pub fn unnamed(&self, country: &Country, hour: i32) -> String {
        if let Some(reason) = self.not_known(country, hour) {
            return reason;
        }
        if matches!(country, Country::Slot(_)) {
            let unlinked = self.unlinked();
            if unlinked.get(unlinked.partition_point(|&(_, last)| last < hour)).is_some_and(|&(first, _)| first <= hour) {
                return format!("Who holds {} is not known: {TAG_UNLINKED}", country.key());
            }
        }
        if country.runs(self).first().is_some_and(|run| run.first <= hour) { NOT_HELD_AT_HOUR } else { NOT_IN_HISTORY }.to_owned()
    }

    pub fn say(&self, country: &Country, hour: i32, message: impl FnOnce(&str) -> String) -> String {
        self.label(country, hour).map_or_else(|| self.unnamed(country, hour), |name| message(&name))
    }

    pub fn nearest_name(&self, country: &Country, hour: i32) -> String {
        if let Some(tag) = self.tag_at(country, hour) {
            return tag.to_owned();
        }
        let Country::Slot(slot) = country else { return country.to_string() };
        let distance = |first: i32, last: i32| if hour < first { i64::from(first) - i64::from(hour) } else { i64::from(hour) - i64::from(last) };
        self.slots().get(slot).and_then(|held| held.tags.iter().min_by_key(|(first, last, ..)| distance(*first, *last))).map_or_else(|| country.key().into_owned(), |(.., tag)| tag.clone())
    }

    pub fn within<K: Key + ?Sized>(&self, country: &K, from: i32, to: i32) -> Vec<HolderRun> {
        let runs = country.runs(self);
        let start = runs.partition_point(|run| run.last < from);
        runs[start..]
            .iter()
            .take_while(|run| run.first <= to)
            .map(|run| HolderRun {
                first: run.first.max(from),
                last: run.last.min(to),
                ..*run
            })
            .collect()
    }

    pub fn segment_first<K: Key + ?Sized>(&self, country: &K, hour: i32) -> Option<i32> {
        let runs = country.runs(self);
        let at = runs.partition_point(|run| run.last < hour);
        if runs.get(at).is_none_or(|run| run.first > hour) {
            return None;
        }
        runs[..=at].iter().rev().find(|run| run.starts_segment).map(|run| run.first)
    }

    pub fn named<K: Key + ?Sized>(&self, country: &K) -> bool {
        !country.runs(self).is_empty()
    }

    pub fn slot_tag(&self, slot: u32, hour: i32) -> Option<&str> {
        let Holder::Found { id, .. } = self.at(&Country::Slot(slot), hour)?.holder else { return None };
        self.slots().get(&slot)?.tags.iter().find(|(first, last, holder, _)| *first <= hour && hour <= *last && *holder == id).map(|(.., tag)| tag.as_str())
    }
}

fn slot_histories(tags: &BTreeMap<String, Vec<HolderRun>>) -> BTreeMap<u32, SlotHistory> {
    let mut out: BTreeMap<u32, SlotHistory> = BTreeMap::new();
    for (tag, runs) in tags {
        for run in runs {
            if let Holder::Found { slot, id } = run.holder {
                out.entry(slot).or_default().tags.push((run.first, run.last, id, tag.clone()));
            }
        }
    }
    for (&slot, held) in &mut out {
        held.tags.sort_unstable_by(|a, b| (a.0, &a.3).cmp(&(b.0, &b.3)));
        let mut edges: Vec<i64> = held.tags.iter().flat_map(|(first, last, ..)| [i64::from(*first), i64::from(*last) + 1]).collect();
        edges.sort_unstable();
        edges.dedup();
        let mut starts: Vec<(i64, u32)> = held.tags.iter().map(|(first, _, id, _)| (i64::from(*first), *id)).collect();
        let mut ends: Vec<(i64, u32)> = held.tags.iter().map(|(_, last, id, _)| (i64::from(*last) + 1, *id)).collect();
        starts.sort_unstable();
        ends.sort_unstable();
        let (mut started, mut ended) = (0, 0);
        let mut covering: BTreeMap<u32, usize> = BTreeMap::new();
        let mut runs: Vec<HolderRun> = Vec::new();
        for pair in edges.windows(2) {
            let at = pair[0];
            while let Some(&(_, id)) = ends.get(ended).filter(|(end, _)| *end <= at) {
                if let Some(count) = covering.get_mut(&id) {
                    *count -= 1;
                    if *count == 0 {
                        covering.remove(&id);
                    }
                }
                ended += 1;
            }
            while let Some(&(_, id)) = starts.get(started).filter(|(start, _)| *start <= at) {
                *covering.entry(id).or_default() += 1;
                started += 1;
            }
            let Some((&id, _)) = covering.first_key_value() else { continue };
            runs.push(HolderRun { first: at as i32, last: (pair[1] - 1) as i32, holder: Holder::Found { slot, id }, starts_segment: true });
        }
        held.runs = merged(runs);
    }
    out
}

struct Row {
    tag: Option<String>,
    reason: String,
    holder: Holder,
    first: i32,
    last: i32,
}

fn history(recording: &Recording, run: u32) -> io::Result<Vec<Row>> {
    let held = recording.run(run)?;
    let end = held.first_hour + held.ordinals as i32 - 1;
    let mut names: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let held = read::at(recording, run, &[read::Read::all(TableId::TagNames)], end)?;
    let rows = held[0].view();
    for slot in rows.rows() {
        let number = rows.key(slot).ok_or_else(|| invalid("A tag name has no key"))? as u32;
        let text = read::text(&rows, 0, slot).ok_or_else(|| invalid("A tag name names a string its run never added"))?;
        names.insert(number, String::from_utf8_lossy(text).into_owned());
    }
    let mut out = Vec::new();
    for (key, first, last) in crate::cost::lifetimes(recording, run, TableId::TagHolders)? {
        let (number, slot, id) = ((key >> 96) as u32, (key >> 32) as u32, key as u32);
        let (first, last) = (first as i32, last as i32);
        if number == TagHolderRow::UNKNOWN {
            let reason = TagHolderRow::REASONS.get(slot as usize).copied().unwrap_or("No holder is known at these hours");
            out.push(Row { tag: None, reason: reason.to_owned(), holder: Holder::Unresolved, first, last });
            continue;
        }
        let tag = names.get(&number).ok_or_else(|| invalid("An identity row's tag number has no name"))?.clone();
        let holder = if slot == TagHolderRow::NONE { Holder::Unresolved } else { Holder::Found { slot, id } };
        out.push(Row { tag: Some(tag), reason: String::new(), holder, first, last });
    }
    Ok(out)
}

fn who_holds(name: &str, reason: &str) -> String {
    format!("Who holds {name} is not known: {reason}")
}

pub fn holders(recording: &Recording, timeline: &Timeline) -> io::Result<Holders> {
    let mut out = Holders::default();
    let mut read: std::collections::HashMap<u32, Vec<Row>> = std::collections::HashMap::new();
    for stretch in &timeline.stretches {
        if let std::collections::hash_map::Entry::Vacant(slot) = read.entry(stretch.run) {
            slot.insert(history(recording, stretch.run)?);
        }
        for row in &read[&stretch.run] {
            let first = row.first.max(stretch.first);
            let last = row.last.min(stretch.last);
            if first > last {
                continue;
            }
            match &row.tag {
                Some(tag) => out.tags.entry(tag.clone()).or_default().push(HolderRun { first, last, holder: row.holder, starts_segment: true }),
                None => out.unknown.push((first, last, row.reason.clone())),
            }
        }
    }
    out.unknown.sort_unstable_by_key(|(first, ..)| *first);
    for runs in out.tags.values_mut() {
        *runs = merged(std::mem::take(runs));
    }
    Ok(out)
}

fn merged(mut runs: Vec<HolderRun>) -> Vec<HolderRun> {
    runs.sort_unstable_by_key(|run| run.first);
    let mut out: Vec<HolderRun> = Vec::with_capacity(runs.len());
    for run in runs {
        match out.last_mut() {
            Some(previous) if previous.holder == run.holder && i64::from(previous.last) + 1 == i64::from(run.first) => previous.last = run.last,
            _ => out.push(HolderRun { starts_segment: true, ..run }),
        }
    }
    out
}

#[cfg(test)]
mod tests;
