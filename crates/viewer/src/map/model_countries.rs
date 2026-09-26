//! The country operands the land-model selectors prefix entity names with.
//!
//! `141917880` tries the cosmetic tag (`CCountry+0x1488`), the tag and the
//! runtime graphical culture (`+0x1028`, `CountryAppearance`). The transport
//! ladder `141278920` uses the original country (`+0x130C`) of a dynamic
//! country instead, whose tag `CountryAppearance.name_tag` names. The ruling
//! party's ideology group filters the authored graphic pools.

use std::{collections::BTreeMap, io};

use replay::{frame::DecodedFrame, rows::tables};

use super::tables::rows;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelCountry {
    pub tag: String,
    pub cosmetic: String,
    /// Invalid bytes are unknown; they do not invalidate earlier prefixes.
    pub graphical_culture: Option<String>,
    /// The transport ladder's country: the original of a dynamic country.
    pub original_tag: Option<String>,
    pub original_graphical_culture: Option<String>,
    /// The ruling party's ideology group, `Some(None)` without a ruling
    /// party; `None` when this hour's party table is incomplete.
    pub ruling_ideology: Option<Option<String>>,
}

/// Canonical country slots with readable model operands. A country without an
/// appearance row is absent: a missing row does not prove it has no cosmetic tag.
pub fn read(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, ModelCountry>> {
    let links: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let countries = frame.complete::<tables::Countries>()?;
    let by_tag: BTreeMap<&[u8], u32> = countries
        .iter()
        .map(|row| (row.tag.as_slice(), row.canonical_slot))
        .collect();
    let appearance: BTreeMap<u32, _> = rows::<tables::CountryAppearance>(frame)?
        .rows()
        .iter()
        .filter_map(|row| Some((*links.get(&row.country)?, row)))
        .collect();
    let parties = rows::<tables::PoliticalParties>(frame)?;
    let parties_known = parties.is_complete();
    let mut ideology = BTreeMap::new();
    for party in parties.rows() {
        if party.is_ruling != 0
            && let Some(slot) = links.get(&party.country)
        {
            ideology.insert(*slot, String::from_utf8_lossy(&party.token).into_owned());
        }
    }
    let culture = |slot: &u32| {
        appearance
            .get(slot)
            .and_then(|row| std::str::from_utf8(&row.graphical_culture).ok())
            .map(str::to_owned)
    };
    Ok(countries
        .iter()
        .filter_map(|country| {
            let slot = country.canonical_slot;
            let row = appearance.get(&slot)?;
            let tag = std::str::from_utf8(&country.tag).ok()?;
            if tag.is_empty() {
                return None;
            }
            let (original_tag, original_graphical_culture) = if row.name_tag.is_empty() {
                (Some(tag.to_owned()), culture(&slot))
            } else {
                match by_tag.get(row.name_tag.as_slice()) {
                    Some(original) => (
                        std::str::from_utf8(&row.name_tag).ok().map(str::to_owned),
                        culture(original),
                    ),
                    None => (None, None),
                }
            };
            Some((
                slot,
                ModelCountry {
                    tag: tag.to_owned(),
                    cosmetic: std::str::from_utf8(&row.cosmetic_tag).ok()?.to_owned(),
                    graphical_culture: culture(&slot),
                    original_tag,
                    original_graphical_culture,
                    ruling_ideology: parties_known.then(|| ideology.get(&slot).cloned()),
                },
            ))
        })
        .collect())
}
