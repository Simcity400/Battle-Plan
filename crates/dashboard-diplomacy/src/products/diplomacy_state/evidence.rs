//! Owner read truth shared by opinions and relation tuples, without oracle dependencies.
use super::DiplomacyCountry;
use dashboard_text::metadata::{TaggedCountry, link_slot, tagged_countries};
use replay_data::{
    cadence::{self, Cadence},
    format::FormatVersion,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Joins {
    pub(super) countries: BTreeMap<String, DiplomacyCountry>,
    pub(super) raw: BTreeMap<u32, DiplomacyCountry>,
    pub(super) aliases: BTreeMap<DiplomacyCountry, Vec<u32>>,
    pub(super) unresolved: usize,
}
impl Joins {
    pub(super) fn read(frame: &DecodedFrame) -> Option<Self> {
        let countries = frame.complete::<tables::Countries>().ok()?;
        let links = frame.complete::<tables::CountryLinks>().ok()?;
        let mut out = Self::default();
        let mut slots = BTreeMap::new();
        for (tag, lookup) in tagged_countries(countries, links) {
            if let (Ok(tag), TaggedCountry::Found(row, slot)) = (std::str::from_utf8(tag), lookup) {
                if tag.is_empty() || row.id == 0 {
                    out.unresolved += 1;
                    continue;
                }
                let country = DiplomacyCountry {
                    tag: tag.to_owned(),
                    slot,
                    country_id: row.id,
                };
                slots.insert(slot, country.clone());
                out.countries.insert(tag.to_owned(), country);
            } else {
                out.unresolved += 1;
            }
        }
        for link in links {
            if link.id == 0 {
                continue;
            }
            if let Some(country) = link_slot(links, link.id).and_then(|slot| slots.get(&slot)) {
                out.raw.insert(link.id, country.clone());
                out.aliases
                    .entry(country.clone())
                    .or_default()
                    .push(link.id);
            }
        }
        Some(out)
    }
}

/// The same conservative full-refresh witness used by relation comparisons:
/// an off-slot timed daily walk, excluding the independently timed intel buffer.
/// No snapshot/keyframe or table allocation is itself a full-refresh witness.
pub(super) fn full_refresh(frame: &DecodedFrame, format: FormatVersion) -> bool {
    full_refresh_at(frame.hour, &frame.metrics, format)
}

pub(super) fn full_refresh_at(
    hour: i32,
    metrics: &replay_frame::frame::Metrics,
    format: FormatVersion,
) -> bool {
    TableId::ALL.into_iter().any(|table| {
        table != TableId::CountryIntel
            && metrics.table_ns[table.index()] != 0
            && matches!(cadence::cadence_in(table, format), Cadence::Daily(_))
            && !cadence::cadence_in(table, format).is_refresh(hour)
    })
}

pub(super) fn available(frame: &DecodedFrame, table: TableId) -> bool {
    matches!(
        &*frame.tables[table.index()],
        replay_frame::frame::TableView::Complete(_)
    )
}

/// A supplying stamp is evidence only alongside a complete table. Rowless
/// owners with no stamp are current only on their own scheduled slice; no
/// earlier supplying hour is invented. Canonical owners, not alias links,
/// select the scheduled slice. Remembered rowless reads belong to the observer.
#[cfg(test)]
pub(super) fn owner_read(
    frame: &DecodedFrame,
    format: FormatVersion,
    table: TableId,
    owner: u32,
) -> Option<i32> {
    owner_read_proven(frame, table, owner, full_refresh(frame, format))
}

pub(super) fn owner_read_proven(
    frame: &DecodedFrame,
    table: TableId,
    owner: u32,
    full: bool,
) -> Option<i32> {
    if !available(frame, table) {
        return None;
    }
    if full {
        return Some(frame.hour);
    }
    let reads = frame.complete::<tables::SliceReads>().ok()?;
    if let Some(hour) = replay_data::sliced::read_hour(reads, table, owner) {
        return (hour <= frame.hour
            && i64::from(frame.hour) - i64::from(hour) < i64::from(cadence::STAGGER_PERIOD))
        .then_some(hour);
    }
    let countries = frame.complete::<tables::Countries>().ok()?;
    (countries.binary_search_by_key(&owner, |row| row.id).is_ok()
        && cadence::slice_of(owner) == frame.hour.rem_euclid(cadence::STAGGER_PERIOD) as u32)
        .then_some(frame.hour)
}

/// Every freshly read resolved owner, including dirty slices and complete
/// rowless scheduled reads. Carried stamps do not refresh a supplying date.
pub(super) fn current_owners(
    frame: &DecodedFrame,
    format: FormatVersion,
    table: TableId,
    joins: &Joins,
) -> BTreeSet<DiplomacyCountry> {
    if !available(frame, table) {
        return BTreeSet::new();
    }
    let full = full_refresh(frame, format);
    let mut current = BTreeSet::new();
    for country in joins.countries.values() {
        if full || owner_read_proven(frame, table, country.country_id, full) == Some(frame.hour) {
            current.insert(country.clone());
        }
    }
    if let Ok(reads) = frame.complete::<tables::SliceReads>() {
        let id = table.index() as u32;
        let first = reads.partition_point(|read| read.table < id);
        let last = reads.partition_point(|read| read.table <= id);
        for read in &reads[first..last] {
            if read.sample_hour == frame.hour
                && let Some(country) = joins.raw.get(&read.owner)
            {
                current.insert(country.clone());
            }
        }
    }
    current
}

/// Each owner and the length of its run in rows sorted by owner.
pub(super) fn owner_runs<'r, R>(
    rows: &'r [R],
    owner: impl Fn(&R) -> u32 + 'r,
) -> impl Iterator<Item = (u32, usize)> + 'r {
    let mut at = 0;
    std::iter::from_fn(move || {
        let first = owner(rows.get(at)?);
        let run = rows[at..].partition_point(|row| owner(row) == first);
        at += run;
        Some((first, run))
    })
}
