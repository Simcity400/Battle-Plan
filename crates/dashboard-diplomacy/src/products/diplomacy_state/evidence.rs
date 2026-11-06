//! The identities an hour resolves, and its full-refresh witness.
use super::DiplomacyCountry;
use dashboard_text::metadata::{TaggedCountry, link_slot, tagged_countries};
use replay_data::{
    cadence::{self, Cadence},
    format::FormatVersion,
    rows::{CountryLinkRow, CountryRow, RowCodec, TableId, mask, tables},
};
use replay_frame::frame::Tables;
use std::{borrow::Cow, collections::BTreeMap, io};

#[derive(Default)]
pub(super) struct Joins {
    pub(super) countries: BTreeMap<String, DiplomacyCountry>,
    pub(super) raw: BTreeMap<u32, DiplomacyCountry>,
    pub(super) aliases: BTreeMap<DiplomacyCountry, Vec<u32>>,
    pub(super) unresolved: usize,
}

/// The rows of `T`'s complete table, each of the fields `mask` names; none when it is not
/// complete.
fn complete<T: replay_data::rows::TableSchema>(
    frame: &(impl Tables + ?Sized),
    mask: u64,
) -> io::Result<Option<Vec<T::Row>>>
where
    T::Row: RowCodec,
{
    let Ok(rows) = frame.complete_rows::<T>(mask) else {
        return Ok(None);
    };
    rows.iter()
        .map(|row| row.map(Cow::into_owned))
        .collect::<io::Result<Vec<_>>>()
        .map(Some)
}

impl Joins {
    /// The identities of the countries and links, each of the fields the joins read; none
    /// unless both are complete.
    pub(super) fn read(frame: &(impl Tables + ?Sized)) -> io::Result<Option<Self>> {
        let Some(countries) = complete::<tables::Countries>(
            frame,
            mask::<CountryRow>(&["canonical_slot", "tag"]),
        )?
        else {
            return Ok(None);
        };
        let Some(links) =
            complete::<tables::CountryLinks>(frame, mask::<CountryLinkRow>(&["canonical_slot"]))?
        else {
            return Ok(None);
        };
        let (countries, links) = (countries.as_slice(), links.as_slice());
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
        Ok(Some(out))
    }
}

/// The same conservative full-refresh witness used by relation comparisons:
/// an off-slot timed daily walk, excluding the independently timed intel buffer.
/// No snapshot/keyframe or table allocation is itself a full-refresh witness.
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
