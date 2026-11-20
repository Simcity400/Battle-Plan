//! The one identity rule (SPEC 3.3), applied once for every per-country product: a tag's holder
//! is what `TagHolders` says. That table is a history whose rows are never deleted, so a run's
//! holder runs are the table as it stands at the run's last hour: one read of one small table,
//! whatever the campaign's length, kept while the recording is open. A tag is unresolved where
//! its row names no holder and absent where no row covers the hour; UNKNOWN rows are hours no
//! holder is known for any tag. A request names its country as a [`Country`]: a canonical slot,
//! followed through every tag that country holds, or a tag the user chose.

use crate::{read, timeline::Timeline};
use replay::columnar_frames::Recording;
use replay_data::rows::{TableId, TagHolderRow};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, io, sync::OnceLock};
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}


/// The country a request names: its canonical slot (the catalogue's country id), the country
/// itself whatever tag it holds, or a tag the user chose, whose holder is resolved at each hour.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize, TS)]
#[serde(untagged)]
#[ts(rename = "CountryRef")]
pub enum Country {
    Slot(u32),
    Tag(String),
}

/// Why a slot that holds no tag in a history has no value there: it is not in that history.
pub const NOT_IN_HISTORY: &str = "This country is not in this branch";

/// What starts a slot's [`Country::key`].
pub const SLOT_MARK: char = '\0';

impl Country {
    /// Its name where a string names it: the tag, or [`SLOT_MARK`] and the slot's number, which no
    /// tag can spell (a tag never holds a NUL).
    pub fn key(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Tag(tag) => std::borrow::Cow::Borrowed(tag),
            Self::Slot(slot) => std::borrow::Cow::Owned(format!("{SLOT_MARK}{slot}")),
        }
    }

    /// The tag a tag choice names; none for a slot.
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

/// What names a country's holder runs: a tag, or a [`Country`].
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

/// Who holds a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Holder {
    /// Its holder's canonical slot and raw country id.
    Found { slot: u32, id: u32 },
    /// The tag's countries, none of whose links resolves.
    Unresolved,
}

/// A stretch of hours one holder held a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HolderRun {
    pub first: i32,
    pub last: i32,
    pub holder: Holder,
    /// The run follows hours the tag was absent, unresolved, unknown or unrecorded, another
    /// holder, or a new stretch of the timeline: it starts a segment.
    pub starts_segment: bool,
}

/// Every tag's holders over a timeline.
#[derive(Clone, Debug, Default)]
pub struct Holders {
    /// By tag, in hour order.
    pub tags: BTreeMap<String, Vec<HolderRun>>,
    /// Hours the identity table was not available, with why.
    pub unknown: Vec<(i32, i32, String)>,
    slots: OnceLock<BTreeMap<u32, SlotHistory>>,
}

/// One canonical slot's history, from the tags it holds.
#[derive(Clone, Debug, Default)]
struct SlotHistory {
    /// The hours some tag's holder resolves to the slot, in hour order; the holder is the lowest
    /// raw id among them at each hour.
    runs: Vec<HolderRun>,
    /// Each tag's stretch held by the slot, (first, last, raw id, tag), by first hour.
    tags: Vec<(i32, i32, u32, String)>,
}

impl Holders {
    pub fn new(tags: BTreeMap<String, Vec<HolderRun>>, unknown: Vec<(i32, i32, String)>) -> Self {
        Self { tags, unknown, slots: OnceLock::new() }
    }

    fn slots(&self) -> &BTreeMap<u32, SlotHistory> {
        self.slots.get_or_init(|| slot_histories(&self.tags))
    }

    /// `country`'s runs in hour order: a tag's holder runs, or the hours a slot holds a tag.
    pub fn runs<K: Key + ?Sized>(&self, country: &K) -> &[HolderRun] {
        country.runs(self)
    }

    /// `country`'s holder at `hour`, with the run holding it.
    pub fn at<K: Key + ?Sized>(&self, country: &K, hour: i32) -> Option<&HolderRun> {
        let runs = country.runs(self);
        let at = runs.partition_point(|run| run.last < hour);
        runs.get(at).filter(|run| run.first <= hour)
    }

    /// The tag `country` goes by at `hour`: a tag choice's own tag, or the tag a slot's holder holds
    /// then; none for a slot holding no tag at that hour.
    pub fn tag_at<'a>(&'a self, country: &'a Country, hour: i32) -> Option<&'a str> {
        match country {
            Country::Tag(tag) => Some(tag),
            Country::Slot(slot) => {
                let Holder::Found { id, .. } = self.at(country, hour)?.holder else { return None };
                let held = self.slots().get(slot)?;
                held.tags.iter().find(|(first, last, holder, _)| *first <= hour && hour <= *last && *holder == id).map(|(.., tag)| tag.as_str())
            }
        }
    }

    /// Whether `country` goes by `tag` at `hour`: the tag itself, or a slot whose country holds it
    /// then.
    pub fn goes_by(&self, country: &Country, tag: &str, hour: i32) -> bool {
        match country {
            Country::Tag(chosen) => chosen == tag,
            Country::Slot(slot) => matches!(self.at(tag, hour).map(|run| run.holder), Some(Holder::Found { slot: held, .. }) if held == *slot),
        }
    }

    /// The tag to name `country` by near `hour`: its tag then, else the nearest tag it held,
    /// else (a slot holding no tag in this history) its [`Country::key`].
    pub fn name(&self, country: &Country, hour: i32) -> String {
        self.label(country, hour).unwrap_or_else(|| country.key().into_owned())
    }

    /// The tag to name `country` by near `hour` in text: as [`Self::name`], but none for a slot
    /// holding no tag in this history, which has no name to show.
    pub fn label(&self, country: &Country, hour: i32) -> Option<String> {
        if let Some(tag) = self.tag_at(country, hour) {
            return Some(tag.to_owned());
        }
        let Country::Slot(slot) = country else { return Some(country.to_string()) };
        let distance = |first: i32, last: i32| if hour < first { i64::from(first) - i64::from(hour) } else { i64::from(hour) - i64::from(last) };
        self.slots().get(slot).and_then(|held| held.tags.iter().min_by_key(|(first, last, ..)| distance(*first, *last))).map(|(.., tag)| tag.clone())
    }

    /// A message about `country` near `hour`, naming it by [`Self::label`]; [`NOT_IN_HISTORY`]
    /// for a slot that has no name to show.
    pub fn say(&self, country: &Country, hour: i32, message: impl FnOnce(&str) -> String) -> String {
        self.label(country, hour).map_or_else(|| NOT_IN_HISTORY.to_owned(), |name| message(&name))
    }

    /// `country`'s runs meeting `from..=to`, cut to it; the first keeps `starts_segment` only where
    /// its run starts there.
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

    /// The first hour of the segment holding `hour` of `country`'s runs.
    pub fn segment_first<K: Key + ?Sized>(&self, country: &K, hour: i32) -> Option<i32> {
        let runs = country.runs(self);
        let at = runs.partition_point(|run| run.last < hour);
        if runs.get(at).is_none_or(|run| run.first > hour) {
            return None;
        }
        runs[..=at].iter().rev().find(|run| run.starts_segment).map(|run| run.first)
    }

    /// Whether `country` is ever held on the timeline.
    pub fn named<K: Key + ?Sized>(&self, country: &K) -> bool {
        !country.runs(self).is_empty()
    }
}

/// Each slot's history from every tag's resolved holder runs.
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
        // One sweep over the edges, holding the raw ids whose runs cover each stretch.
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

/// One row of a run's identity history: a tag (or, unknown, why) and its holder from `first`
/// through `last`.
struct Row {
    tag: Option<String>,
    reason: String,
    holder: Holder,
    first: i32,
    last: i32,
}

/// Run `run`'s identity history: the identity table's lifetimes, each key one holder's stretch,
/// and the tag of each number, read at the run's last hour.
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

/// Every tag's holder runs over `timeline`, from each of its runs' identity history.
pub fn holders(recording: &Recording, timeline: &Timeline) -> io::Result<Holders> {
    let mut out = Holders::default();
    let mut read: std::collections::HashMap<u32, Vec<Row>> = std::collections::HashMap::new();
    for stretch in &timeline.stretches {
        if !read.contains_key(&stretch.run) {
            read.insert(stretch.run, history(recording, stretch.run)?);
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

/// `runs` in hour order, the same holder in adjacent hours (across a load into a recorded hour
/// too) one run; every run starts a segment, as another holder, hours unheld or unknown, or no
/// resolved holder lie before it.
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
