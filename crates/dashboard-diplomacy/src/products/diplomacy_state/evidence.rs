//! The identities an hour resolves, and its full-refresh witness.
use super::DiplomacyCountry;
use dashboard_text::metadata::{TaggedCountry, link_slot, tagged_countries};
use replay_data::{
    cadence::{self, Cadence},
    format::FormatVersion,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;

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
