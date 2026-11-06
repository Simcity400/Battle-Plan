//! Native country references as keyed names spell them: a raw reference
//! resolves through the hour's country links to its canonical slot, and the
//! slot through the hour's countries to the tag it holds then. A name spells a
//! country `TAG@slot`, so a tag reused by another country, or a slot holding
//! another tag, is never merged with the country that held it before.

use replay_data::rows::{CountryLinkRow, CountryRow, TableId, column, mask, tables};
use replay_frame::frame::{ColumnFrame, DecodedFrame, Tables};
use std::{collections::HashMap, io};

/// One hour's country links and tags.
#[derive(Default)]
pub struct Countries {
    links: HashMap<u32, u32>,
    tags: HashMap<u32, String>,
}

impl Countries {
    /// The hour's links and countries; None when either is not recorded.
    pub fn read(frame: &DecodedFrame) -> Option<Self> {
        let links = frame.complete::<tables::CountryLinks>().ok()?;
        let countries = frame.complete::<tables::Countries>().ok()?;
        Some(Self::from_rows(
            links.iter().map(|link| (link.id, link.canonical_slot)),
            countries,
        ))
    }

    /// [`Self::read`] of an hour read through a walk's views.
    pub fn read_walked(frame: &ColumnFrame<'_>) -> io::Result<Option<Self>> {
        const LINKED: usize = column::<CountryLinkRow>("canonical_slot");
        const SLOT: usize = column::<CountryRow>("canonical_slot");
        const TAG: usize = column::<CountryRow>("tag");
        let (Ok(links), Ok(countries)) = (
            frame.complete(TableId::CountryLinks),
            frame.complete(TableId::Countries),
        ) else {
            return Ok(None);
        };
        let mut read = Self::default();
        read.links.reserve(links.len());
        for &at in links.slots() {
            read.links
                .insert(links.key(at) as u32, links.value::<u32>(at, LINKED)?);
        }
        read.tags.reserve(countries.len());
        for &at in countries.slots() {
            let tag = countries.bytes(at, TAG)?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "A country's tag was not recorded")
            })?;
            read.tags.insert(
                countries.value::<u32>(at, SLOT)?,
                String::from_utf8_lossy(tag).into_owned(),
            );
        }
        Ok(Some(read))
    }

    /// [`Self::read`] of any frame's tables, a walk's views among them: the links' slots and the
    /// countries' slots and tags alone.
    pub fn read_tables<F: Tables + ?Sized>(frame: &F) -> io::Result<Option<Self>> {
        const LINKED: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
        const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
        let (Ok(links), Ok(countries)) = (
            frame.complete_rows::<tables::CountryLinks>(LINKED),
            frame.complete_rows::<tables::Countries>(COUNTRY),
        ) else {
            return Ok(None);
        };
        let mut read = Self::default();
        read.links.reserve(links.len());
        for link in links.iter() {
            let link = link?;
            read.links.insert(link.id, link.canonical_slot);
        }
        read.tags.reserve(countries.len());
        for row in countries.iter() {
            let row = row?;
            read.tags.insert(
                row.canonical_slot,
                String::from_utf8_lossy(&row.tag).into_owned(),
            );
        }
        Ok(Some(read))
    }

    pub fn from_rows(
        links: impl IntoIterator<Item = (u32, u32)>,
        countries: &[CountryRow],
    ) -> Self {
        Self {
            links: links.into_iter().collect(),
            tags: countries
                .iter()
                .map(|row| {
                    (
                        row.canonical_slot,
                        String::from_utf8_lossy(&row.tag).into_owned(),
                    )
                })
                .collect(),
        }
    }

    /// The canonical slot a raw reference links to.
    pub fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    /// The tag the country in `slot` holds.
    pub fn tag(&self, slot: u32) -> Option<&str> {
        self.tags.get(&slot).map(String::as_str)
    }

    /// The name part of the country a raw reference names.
    pub fn raw(&self, raw: u32) -> Option<String> {
        let slot = self.slot(raw)?;
        Some(part(self.tag(slot)?, slot))
    }

    /// The name part of the country in canonical `slot`.
    pub fn slotted(&self, slot: u32) -> Option<String> {
        Some(part(self.tag(slot)?, slot))
    }

    /// What its tables hold.
    pub fn held(&self, bytes: &mut dashboard_base::prepared::prepass::HeldBytes) {
        bytes
            .table::<u32, u32>(self.links.capacity())
            .table::<u32, String>(self.tags.capacity());
        for tag in self.tags.values() {
            bytes.heap(tag);
        }
    }
}

/// A country's name part: its tag and canonical slot.
pub fn part(tag: &str, slot: u32) -> String {
    format!("{tag}@{slot}")
}

/// The tag and slot a country name part spells.
pub fn split(part: &str) -> Option<(&str, u32)> {
    let (tag, slot) = part.rsplit_once('@')?;
    Some((tag, slot.parse().ok()?))
}

/// Whether a country name part is the one a request selects: `TAG@slot` exactly,
/// or a bare tag at any slot.
pub fn selected(part: &str, selection: &str) -> bool {
    if split(selection).is_some() {
        part == selection
    } else {
        split(part).is_some_and(|(tag, _)| tag == selection)
    }
}

/// A requested country: a valid tag, optionally with its slot, and no name separator.
pub fn validate(selection: &str) -> Result<(), &'static str> {
    dashboard_base::products::validate_tag(split(selection).map_or(selection, |(tag, _)| tag))?;
    if selection.contains('|') {
        return Err("Invalid country name part");
    }
    Ok(())
}
