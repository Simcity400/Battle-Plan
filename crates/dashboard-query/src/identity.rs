//! The one identity rule (SPEC 3.3), applied once for every per-country product: a tag's holder
//! is what `TagHolders` says. That table is a history whose rows are never deleted, so a run's
//! holder runs are the table as it stands at the run's last hour: one read of one small table,
//! whatever the campaign's length, kept while the recording is open. A tag is unresolved where
//! its row names no holder and absent where no row covers the hour; UNKNOWN rows are hours no
//! holder is known for any tag.

use crate::{read, timeline::Timeline};
use replay::columnar_frames::Recording;
use replay_data::rows::{TableId, TagHolderRow};
use std::{collections::BTreeMap, io};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
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
}

impl Holders {
    /// `tag`'s holder at `hour`, with the run holding it.
    pub fn at(&self, tag: &str, hour: i32) -> Option<&HolderRun> {
        let runs = self.tags.get(tag)?;
        let at = runs.partition_point(|run| run.last < hour);
        runs.get(at).filter(|run| run.first <= hour)
    }

    /// `tag`'s runs meeting `from..=to`, cut to it; the first keeps `starts_segment` only where its
    /// run starts there.
    pub fn within(&self, tag: &str, from: i32, to: i32) -> Vec<HolderRun> {
        let Some(runs) = self.tags.get(tag) else { return Vec::new() };
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

    /// The first hour of the segment holding `hour` of `tag`'s runs.
    pub fn segment_first(&self, tag: &str, hour: i32) -> Option<i32> {
        let runs = self.tags.get(tag)?;
        let at = runs.partition_point(|run| run.last < hour);
        if runs.get(at).is_none_or(|run| run.first > hour) {
            return None;
        }
        runs[..=at].iter().rev().find(|run| run.starts_segment).map(|run| run.first)
    }

    /// Whether `tag` is ever held on the timeline.
    pub fn named(&self, tag: &str) -> bool {
        self.tags.contains_key(tag)
    }
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
