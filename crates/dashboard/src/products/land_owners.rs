//! One compact, campaign-wide ownership catalogue; no per-hour history retained.
//! Names are resolved at each country's first observed hour in the user's game
//! language (`installed_content::country_names`); without game content they are tags.
use super::{LoadContext, Product};
use crate::content::ContentKey;
use crate::{
    CountryMetadata, LandOwners, Source,
    metadata::{mapped_slot, short_text, validate_campaign},
};
use installed_content::{country_names::CountryNames, localisation::Localisation};
use replay::{
    container::Replay,
    frame::{FrameDecoder, TableView},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

/// Includes positive ownership observations from every recorded branch. Control
/// and cores do not establish ownership; unavailable intervals add no countries.
///
/// One entry per (tag, slot), witnessed on the lowest branch that saw it own
/// land (its first observation there), so the reply scales with countries and
/// not with countries x branches. Entries are ordered by tag, then witness
/// branch, then slot: the first entry of a tag is its lowest (branch, slot)
/// observation, as when every branch was listed.
pub fn land_owners(
    replay: &Replay,
    localisation: Option<&Localisation>,
    cancelled: &AtomicBool,
) -> io::Result<LandOwners> {
    let check = || {
        if cancelled.load(Ordering::Acquire) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Country discovery cancelled",
            ))
        } else {
            Ok(())
        }
    };
    // Slot -> (recorded tag, witness branch, entry); a slot holds very few tags.
    let mut countries = HashMap::<u32, Vec<(Vec<u8>, u64, CountryMetadata)>>::new();
    let mut incomplete = !replay.gaps.is_empty();
    let mut unnamed = 0_usize;
    let selected = [
        TableId::Campaign,
        TableId::States,
        TableId::CountryLinks,
        TableId::Countries,
        TableId::CountryAppearance,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::Autonomy,
    ];
    for block in &replay.index {
        check()?;
        let frames = match replay.read_block_selected(block, &selected) {
            Ok(frames) => frames,
            Err(_) => {
                incomplete = true;
                continue;
            }
        };
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            check()?;
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) if validate_campaign(decoded, replay).is_ok() => decoded,
                _ => {
                    incomplete = true;
                    break;
                }
            };
            let states = match &*decoded.tables[replay::rows::TableId::States.index()] {
                TableView::Complete(rows) => rows.rows::<tables::States>(),
                TableView::Truncated { rows, .. } => {
                    incomplete = true;
                    rows.rows::<tables::States>()
                }
                TableView::Unavailable { .. } => None,
            };
            let (Some(states), Ok(links), Ok(rows)) = (
                states,
                decoded.complete::<tables::CountryLinks>(),
                decoded.complete::<tables::Countries>(),
            ) else {
                incomplete = true;
                continue;
            };
            let mut owners: BTreeSet<_> = states
                .iter()
                .filter_map(|state| {
                    match links.binary_search_by_key(&state.owner, |link| link.id) {
                        Ok(index) => Some(links[index].canonical_slot),
                        Err(_) => {
                            incomplete |= state.owner != 0;
                            None
                        }
                    }
                })
                .collect();
            // Operands are read only for a frame that adds a catalogue entry.
            let mut names = None;
            for row in rows {
                let Ok(slot) = mapped_slot(row, links) else {
                    incomplete = true;
                    continue;
                };
                if row.tag.is_empty() || !owners.remove(&slot) {
                    continue;
                }
                let tags = countries.entry(slot).or_default();
                let known = tags.iter().position(|(tag, _, _)| *tag == row.tag);
                if known.is_some_and(|index| tags[index].1 <= frame.branch) {
                    continue;
                }
                let entry = (
                    row.tag.clone(),
                    frame.branch,
                    CountryMetadata {
                        id: slot.to_string(),
                        tag: short_text(&String::from_utf8_lossy(&row.tag), 512),
                        name: match localisation.map(|localisation| {
                            names
                                .get_or_insert_with(|| CountryNames::read(decoded, localisation))
                                .name(slot)
                        }) {
                            Some(Ok(name)) => short_text(&name, 512),
                            Some(Err(_)) => {
                                unnamed += 1;
                                short_text(&String::from_utf8_lossy(&row.tag), 512)
                            }
                            None => short_text(&String::from_utf8_lossy(&row.tag), 512),
                        },
                        observed_branch_id: frame.branch.to_string(),
                        observed_hour: frame.hour,
                    },
                );
                match known {
                    Some(index) => tags[index] = entry,
                    None => tags.push(entry),
                }
            }
            incomplete |= !owners.is_empty();
        }
    }
    check()?;
    let mut countries: Vec<_> = countries
        .into_iter()
        .flat_map(|(slot, tags)| {
            tags.into_iter()
                .map(move |(_, branch, country)| (branch, slot, country))
        })
        .collect();
    let mut warnings = Vec::new();
    if incomplete {
        warnings.push("Some ownership history is unavailable. Only countries with recorded land ownership are listed.".into());
    }
    if unnamed > 0 {
        warnings.push(format!(
            "{unnamed} country names could not be resolved from the recorded content and show their tag."
        ));
    }
    countries.sort_unstable_by(|a, b| (&a.2.tag, a.0, a.1).cmp(&(&b.2.tag, b.0, b.1)));
    Ok(LandOwners {
        countries: countries
            .into_iter()
            .map(|(_, _, country)| country)
            .collect(),
        warnings,
    })
}

pub struct LandOwnersProduct;

/// Land ownership is campaign-wide; there are no parameters.
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LandOwnersParams {}

impl Product for LandOwnersProduct {
    const KEY: &'static str = "land-owners";
    const CONTENT_DEPENDENT: bool = true;
    type Params = LandOwnersParams;
    type Reply = LandOwners;

    fn load(
        source: &Source,
        _params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = crate::content::content_key(context.content.as_ref());
        let state = source.state::<Option<(Option<ContentKey>, LandOwners)>>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        if state.as_ref().is_none_or(|(cached, _)| *cached != key) {
            let localisation = crate::content::name_localisation(source, context.content.as_ref());
            let mut owners =
                land_owners(&source.replay(), localisation.as_deref().ok(), cancelled)?;
            owners.warnings.extend(localisation.err());
            *state = Some((key, owners));
        }
        Ok(state.as_ref().expect("land owners cached").1.clone())
    }
}

#[cfg(test)]
mod tests;
